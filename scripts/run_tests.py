#!/usr/bin/env python3
"""run_tests.py — 每轮测试入口（test-plan §3 冒烟口径 + 夹具自检 + V1/V2/V3 验证 + m1 并行线 + M1.7 真实链）
每轮开发/文档变更后运行：python3 scripts/run_tests.py
  T0 环境契约预检（SC-12）：fixtures-requirements.txt 全清单开跑即知（钉版包/外部引擎/tessdata/字体/cargo）
  T1 xref_check 常规模式（跨文档一致性，0 错误）
  T2 xref_check --inject（历史错误注入回归：R-11/R-14/TC-08 三处必须被捕获）
  T3 夹具完整性：manifest sha256 逐文件校验 + 结构计数（从 fixture_spec 派生，v0.5=44 文件含 corpus-R）
  T4 埋点读回：GT excerpt 逐字存在于对应源文件；B2 无文本层；B1/B3/C 关键串在位（含 T4c-OCR 回读门禁）
  T5 判分器自测（完美运行全绿 + 13 类破坏注入全捕获）
  T6 确定性：重生成到临时目录，44 文件 sha256 全一致（corpus-A 39 + corpus-R 5）
  T7 V1 四段验证（解析覆盖 / 引用回填 / 导出保真 / 暗标口径——脚本 v1_*.py）
  T8 V2 影响面验证（TC-09 主场景 / 级联 / 无关不重跑 / 终裁门禁 / 幂等 / 规则覆盖）
  T9 V3 内核集成三问（重写引用保持 / 撤销只降不升 / 导出中断一致性）
  T10 m1 并行线工具链：cargo build --release + 单测（状态机守卫/幂等/OCR 规整）
  T11 m1 四场景判分（TC-01/02/07/11）：G1/G2/G5/G7 全绿且失败集恰为 pending 三项
       {G3_policy, G4_citation, G6_rules}（M2/M3/M4 范围——出现集合外失败即回归）
  T12 m1 V1-a 解析覆盖：m1 产出 blocks.jsonl 对回 GT 锚点逐字命中（corpus-A 100% 标准）
  T13 m1 确定性：同场景双跑 db/trace/blocks/chunks 四工件字节一致（含 OCR 通道）
  （T10~T13 被测系统 = m1/ 单 crate 首轮实现，并行会话产出；bidcore/ 为演进版，回归在 cargo 侧）
  T14 M1.7 政府采购真实链（真实数据回归：公告级闭环 34 断言 + GT 草案；防新样本/解析器绕过）
  T15 M1.7-C P2 评测 GT 草案 24 条四要素完备性（定位核验/标准答案/判定理由/复核状态；未冻结仅探索性）
  T16 M1.7-TEL 通信行业真实链（txzbqy 四节点链 46 断言 + 文件级解析 17/17 + GT 草案 18 条）
退出码：0 全绿 / 1 存在失败（CI 门禁用）
"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import os
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_spec as S  # noqa: E402
import v1_corpus_r as R  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
FIXTURES = REPO / "fixtures"
CORPUS = FIXTURES / "corpus-A"
M1_DIR = REPO / "m1"
M1_BIN = M1_DIR / "target" / "release" / "m1"
M1_SCOPE = ("G1_recall", "G2_exact", "G5_transitions", "G7_idempotency")
M1_PENDING = {"G3_policy", "G4_citation", "G6_rules"}  # M2/M3/M4 范围——db 无该字段，判分器诚实 FAIL
SCENARIOS = ("TC-01", "TC-02", "TC-07", "TC-11")
# 期望文件数从规格派生（R2 P2-2：不再双处硬编码）
#   5 主文件 + 13 证据 + 4 GT + 12 转换 + 2 查询集 + 2 缺陷草稿 + 1 corpus-R README + 5 corpus-R 结构层
#   + 5 TR2 引擎借鉴夹具（PR #15 引入：word-complex/word-merged/excel-complex + gen-log + PROVENANCE）
EXPECTED_MANIFEST_FILES = (5 + len(S.EVIDENCE_FILES) + 4 + len(S.TRANSITIONS) + 2 + 2 + 1
                           + len(R.OUTPUT_FILES) + 5)
_RULE_CODES = ["CERT_EXPIRED", "PRICE_EXCEEDS_BUDGET", "DURATION_MISMATCH", "PAGE_LIMIT",
               "MISSING_SIGNATURE", "DARK_BID_WORD", "TODO_RESIDUE", "INJECTION_PATTERN"]
_ECHO_RE = None  # 延迟编译（防 import 顺序）


def _echo_regex():
    """标记回声正则（R2 P0-6）：大小写不敏感 + 全角数字/连字符变体，匹配 GT-编号。"""
    global _ECHO_RE
    import re
    if _ECHO_RE is None:
        _ECHO_RE = re.compile(r"(?i)\s?gt\s?[-\u2010-\u2015\uFF0D]?\s?[0-9\uFF10-\uFF19]")
    return _ECHO_RE


_passed, _failed, _skipped = [], [], []

# ---------------------------------------------------------------- SKIP 机制（issue #9）
# 「依赖缺失」不得判成「业务 FAIL」：T0 预检登记缺失清单，
# 受影响的 T 项跳过并显式登记原因（ENV_MISSING），汇总行单列 skipped 计数；
# 清单同步追加 baseline/ENV_BLOCKED.md（B1 机制复用）。
_ENV_MISSING: dict[str, str] = {}   # 工具名 -> 补装指引


def register_env_missing(tool: str, hint: str):
    """登记缺失工具（幂等）——供后续 T 项的 env_ok() 判定与汇总/登记消费。"""
    _ENV_MISSING.setdefault(tool, hint)


def env_ok(tools: list[str]) -> bool:
    """给定工具是否全部在位（在位 = 未被 T0 登记为缺失）。"""
    return all(t not in _ENV_MISSING for t in tools)


def skip(name: str, reason: str):
    """登记 SKIP 项（非失败）——issue #9 判定标准：N passed / M skipped / K failed。"""
    print(f"  ○ {name} —— SKIP: {reason}")
    _skipped.append(name)


def require_tools_or_skip(name: str, tools: list[str], what: str) -> bool:
    """T 项入口守卫：工具缺件则 SKIP 该项并返回 False（调用方应提前 return）。"""
    missing = [t for t in tools if t in _ENV_MISSING]
    if missing:
        skip(name, f"ENV_MISSING: {', '.join(missing)}（{what}）")
        return False
    return True


def write_env_blocked():
    """缺失清单追加 baseline/ENV_BLOCKED.md（B1 机制复用，issue #9 要求 4）。"""
    if not _ENV_MISSING:
        return
    import datetime
    p = REPO / "baseline" / "ENV_BLOCKED.md"
    stamp = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    lines = ["", f"## {stamp} run_tests.py 预检登记（issue #9 SKIP 机制）", ""]
    for tool, hint in _ENV_MISSING.items():
        lines.append(f"- {tool} —— {hint}")
    lines += ["", f"受影响测试项已 SKIP（{len(_skipped)} 项），未计为失败。", ""]
    with open(p, "a", encoding="utf-8") as f:
        f.write("\n".join(lines))


# ---------------------------------------------------------------- T0（环境契约预检，SC-12）
def t_env_preflight():
    """SC-12（issue #7 吸收）：环境契约完备性预检——开跑前核验
    fixtures-requirements.txt 全清单。缺件即响亮红并给补装指引：
    本会话 T4c-OCR 与 R5 e2e_m1 双实证「缺件时下游响亮 FAIL 而非静默过绿」，
    预检把「踩了才知道」升为「开跑即知」——远端/新沙箱拉起首个红点即环境面
    而非代码回归（R5 环境契约提示的固化）。"""
    print("[T0] 环境契约预检（SC-12：fixtures-requirements 全清单）")

    # T0a Python 钉版六包（版本漂移 = 确定性回归隐患源，T6/T13 依赖）
    import importlib.metadata as im
    pins = {"python-docx": "1.2.0", "reportlab": "4.4.9", "openpyxl": "3.1.5",
            "Pillow": "11.3.0", "pdfplumber": "0.11.9", "pymupdf": "1.26.7"}
    bad = []
    for pkg, want in pins.items():
        try:
            got = im.version(pkg)
        except im.PackageNotFoundError:
            bad.append(f"{pkg} 未安装（钉版 {want}）")
            continue
        if got != want:
            bad.append(f"{pkg}=={got} ≠ 钉版 {want}")
    check("T0a Python 钉版六包一致（fixtures-requirements.txt）", not bad,
          "; ".join(bad) if bad else "")

    # T0b 外部解析引擎四件（poppler 双件 / tesseract / LibreOffice headless）
    # issue #9：缺件不判 FAIL——登记入 _ENV_MISSING，受影响 T 项（T4c-OCR/T7/T10~T13）
    # 由 require_tools_or_skip 跳过；汇总行单列 skipped。
    import shutil as _sh
    bins = {"pdftotext": "apt install poppler-utils",
            "pdftoppm": "apt install poppler-utils",
            "pdfinfo": "apt install poppler-utils",
            "tesseract": "apt install tesseract-ocr",
            "soffice": "apt install libreoffice-writer"}
    miss = [b for b in bins if not _sh.which(b)]
    for b in miss:
        register_env_missing(b, bins[b])
    if miss:
        skip("T0b 外部引擎五件在位（pdftotext/pdftoppm/pdfinfo/tesseract/soffice）",
             f"ENV_MISSING: {', '.join(miss)}——已登记，受影响测试项将 SKIP（非失败）")
    else:
        check("T0b 外部引擎五件在位（pdftotext/pdftoppm/pdfinfo/tesseract/soffice）", True, "")

    # T0c chi_sim 语言数据（OCR 分档——issue #9 实测变体：二进制在位而数据缺失同样阻断链路）
    tessdata = Path.home() / ".tessdata"
    ok_c = (tessdata / "chi_sim.traineddata").exists()
    if not ok_c:
        register_env_missing("chi_sim.traineddata",
                             "github.com/tesseract-ocr/tessdata_fast → chi_sim.traineddata 放 ~/.tessdata/")
        skip("T0c chi_sim 语言数据在位（~/.tessdata，tessdata_fast）",
             "ENV_MISSING: chi_sim.traineddata——受影响测试项将 SKIP（非失败）")
    else:
        check("T0c chi_sim 语言数据在位（~/.tessdata，tessdata_fast）", True, "")

    # T0d 渲染字体（可变字体陷阱后钉版——PIL 像素层/B2 扫描件/插图全依赖）
    font = Path("/usr/share/fonts/truetype/chinese/SarasaMonoSC-Regular.ttf")
    check("T0d SarasaMonoSC 渲染字体在位（像素层钉版）", font.exists(),
          "" if font.exists() else "缺失时 B2/D5a/D5b/插图生成全空白（Task 16 字体陷阱）")

    # T0e Rust 工具链（m1 并行线 T10~T13 与 bidcore cargo 回归）
    cargo_path = Path.home() / ".cargo" / "bin" / "cargo"
    ok_e = bool(_sh.which("cargo")) or cargo_path.exists()
    check("T0e cargo 工具链可达（rustup stable，实测 1.99.1）", ok_e,
          "" if ok_e else "补装：sh data/rustup-init.sh -y --profile minimal --default-toolchain stable")


def check(name: str, ok: bool, detail: str = ""):
    mark = "✓" if ok else "✗"
    print(f"  {mark} {name}" + (f" —— {detail}" if detail and not ok else (f"（{detail}）" if detail else "")))
    (_passed if ok else _failed).append(name)


def run(cmd: list[str]) -> tuple[int, str]:
    p = subprocess.run([sys.executable, *cmd], capture_output=True, text=True, cwd=REPO)
    return p.returncode, (p.stdout + p.stderr)


def fail_detail(out: str) -> str:
    """R2 P2-1：失败时展示全部 ✗ 行，不再只取末行。"""
    lines = [l.strip() for l in out.splitlines() if l.strip().startswith("✗")]
    return " | ".join(lines[:6]) if lines else (out.strip().splitlines()[-1] if out.strip() else "无输出")


# ---------------------------------------------------------------- T1/T2
def t_xref():
    print("[T1] xref_check 常规模式")
    code, out = run(["scripts/xref_check.py"])
    check("T1 跨文档一致性 0 错误", code == 0, fail_detail(out) if code else "")
    print("[T2] xref_check 注入自测")
    code, out = run(["scripts/xref_check.py", "--inject"])
    check("T2 历史错误注入全部捕获", code == 0, fail_detail(out) if code else "")


# ---------------------------------------------------------------- T3/T4
def _docx_text(path: Path) -> str:
    from docx import Document
    doc = Document(str(path))
    parts = [p.text for p in doc.paragraphs]
    for t in doc.tables:
        for row in t.rows:
            for cell in row.cells:
                parts.append(cell.text)
    return "\n".join(parts)


def _pdf_text(path: Path) -> str:
    import pdfplumber
    out = []
    with pdfplumber.open(str(path)) as pdf:
        for page in pdf.pages:
            out.append(page.extract_text() or "")
    return "\n".join(out)


def _norm_cn(s: str) -> str:
    """OCR 噪声容忍归一化：NFKC（全角→半角）+ 去空白 + 去标点。仅用于 GT↔OCR 对齐，不改变判分纪律。"""
    import re
    import unicodedata
    s = unicodedata.normalize("NFKC", s)
    return re.sub(r"[\s\W]+", "", s, flags=re.UNICODE)


def _b2_ocr_roundtrip(path: Path) -> str:
    """B2 扫描件 OCR 回读三段验证：像素层有字 → tesseract chi_sim 回读 → 归一化条款包含 + 四要素。
    返回 'ok' 或失败原因（环境缺 tesseract/chi_sim 属环境契约缺失，判 FAIL 不静默跳过）。"""
    try:
        import fitz  # PyMuPDF
        import numpy as np
        from PIL import Image
    except ImportError as e:
        return f"环境缺依赖：{e}"
    tess = None
    for cand in ("/usr/bin/tesseract", "/bin/tesseract"):
        import os
        if os.path.exists(cand):
            tess = cand
            break
    if not tess:
        return "环境契约缺失：tesseract 不可用"
    tessdata = Path.home() / ".tessdata"
    if not (tessdata / "chi_sim.traineddata").exists():
        return "环境契约缺失：chi_sim 语言数据不可用（~/.tessdata）"
    doc = fitz.open(str(path))
    imgs = doc[0].get_images(full=True)
    if not imgs:
        return "B2 无嵌入图像"
    pix = fitz.Pixmap(doc, imgs[0][0])
    img = Image.frombytes("L" if pix.n == 1 else "RGB", (pix.width, pix.height), pix.samples) \
        if pix.n <= 3 else None
    if img is None:  # CMYK 等罕见模式 → 走中转
        pix.save("/tmp/_b2_ocr.png")
        img = Image.open("/tmp/_b2_ocr.png")
    a = np.array(img.convert("L"))
    if int(a.min()) > 60 or (a < 150).sum() < 20000:
        return f"像素层无有效文字（min={a.min()}, dark={(a < 150).sum()}）——字体渲染失败回归"
    img.save("/tmp/_b2_ocr_input.png")
    env = {**__import__("os").environ, "TESSDATA_PREFIX": str(tessdata)}
    r = subprocess.run([tess, "/tmp/_b2_ocr_input.png", "stdout", "-l", "chi_sim", "--psm", "6"],
                       capture_output=True, text=True, env=env, timeout=120)
    if r.returncode != 0:
        return f"tesseract 失败：{r.stderr.strip()[:80]}"
    n_ocr = _norm_cn(r.stdout)
    clause = _norm_cn(S.B2_KEY_CLAUSE)
    if clause not in n_ocr:
        return "OCR 未回读出关键条款（归一化包含失败）"
    missing = [kw for kw in ("投标函", "投标总报价", "盖单位章", "日期") if _norm_cn(kw) not in n_ocr]
    if missing:
        return f"OCR 缺要素：{missing}"
    return "ok"


def t_fixture_integrity():
    print("[T3] 夹具完整性（manifest + 结构计数）")
    manifest = json.loads((CORPUS / "manifest.json").read_text(encoding="utf-8"))
    bad = []
    import hashlib
    for f in manifest["files"]:
        p = FIXTURES / f["path"]  # manifest 路径相对 fixtures 根
        if not p.exists():
            bad.append(f"{f['path']} 缺失")
        elif hashlib.sha256(p.read_bytes()).hexdigest() != f["sha256"]:
            bad.append(f"{f['path']} 哈希不符")
    # R2 P1-4：反向校验——磁盘→manifest（游离文件会被真实流水线当语料摄入）
    on_disk = {str(p.relative_to(FIXTURES)) for p in FIXTURES.rglob("*")
               if p.is_file() and p.name != "manifest.json" and not p.name.endswith(".tmp")}
    stray = sorted(on_disk - {f["path"] for f in manifest["files"]})
    if stray:
        bad.append(f"游离文件未入清单：{stray[:3]}")
    check(f"T3a manifest 双向校验（{EXPECTED_MANIFEST_FILES} 文件）",
          not bad and len(manifest["files"]) == EXPECTED_MANIFEST_FILES, "; ".join(bad[:3]))
    c = manifest["counts"]
    check("T3b 埋点 25 条且类别分布 6/5/4/5/3/2",
          c["requirements"] == 25 and c["categories"] == {"资格条件": 6, "否决项": 5, "评分项": 4,
                                                           "技术参数": 5, "商务要求": 3, "格式要求": 2},
          json.dumps(c["categories"], ensure_ascii=False))
    check("T3c 证据 13 件 + 冲突 1 对 + 转换 12 个 + 查询 25 条 + 章节 6 类 + 缺陷 8 类",
          c["evidence_files"] == 13 and c["conflicts"] == 1 and c["transitions"] == 12
          and c["exact_queries"] == 25 and c["sections"] == 6 and c["defects"] == 8)
    check("T3d 关键口径（开标 11-15→11-27；控制价 480→465 万；EV-02 至 11-25）",
          manifest["key_facts"]["open_bid_date"] == "2026-11-15 09:30"
          and manifest["key_facts"]["open_bid_date_after_addendum"] == "2026-11-27 09:30"
          and manifest["key_facts"]["control_price_a"] == 4_800_000
          and manifest["key_facts"]["control_price_c"] == 4_650_000)
    check("T3e B1 单价上限合计 ≤ 设备预算 350 万（无隐性冲突）",
          manifest["key_facts"]["b1_total_cap"] == S.B1_TOTAL_CAP and S.B1_TOTAL_CAP <= 3_500_000,
          f"合计 {S.B1_TOTAL_CAP:,} 元")


def t_readback():
    print("[T4] 埋点读回（ground truth ↔ 源文件逐字核对）+ 反标记回声")
    files = CORPUS / "files"
    a_text = _docx_text(files / "A-招标文件.docx")
    c_text = _docx_text(files / "C-补遗文件01.docx")
    b1_text = _pdf_text(files / "B1-设备清单.pdf")
    b2_text = _pdf_text(files / "B2-投标函格式页-扫描件.pdf")
    from openpyxl import load_workbook
    wb = load_workbook(str(files / "B3-图纸目录.xlsx"))
    ws = wb.active
    b3_text = "\n".join(str(cell.value) for row in ws.iter_rows() for cell in row if cell.value)

    # T4a：逐埋点逐锚点读回（B2 为图像页，按 B2_PAGE_TEXT 规格核对——OCR 真实性由 D5 矩阵/D5 管道承接）
    sources = {"A-招标文件.docx": a_text, "B1-设备清单.pdf": b1_text,
               "B2-投标函格式页-扫描件.pdf": S.B2_PAGE_TEXT, "C-补遗文件01.docx": c_text}
    miss = []
    for r in S.REQUIREMENTS:
        for a in r["anchors"]:
            src = sources.get(a["file"])
            if src is None:
                miss.append(f"{r['gt_id']} 无读回源（{a['file']}）")
            elif a["excerpt"] not in src:
                miss.append(f"{r['gt_id']}『{a['excerpt'][:18]}…』未见于 {a['file']}")
    check("T4a 25 埋点全部锚点逐字存在（含 GT-07×2 / GT-20×2 双锚点）", not miss, "; ".join(miss[:3]))

    dd_text = _docx_text(CORPUS / "defect-drafts" / S.DEFECT_DRAFT_NAME)

    # T4h（审计 S1 + R2 P0-6）：全语料反标记回声——A/C/B1/B3/B2 规格/证据/缺陷草稿全扫；
    # 大小写不敏感 + 全角数字/连字符变体 + 8 个规则码字面量
    echo = []
    corpus_texts = {"A.docx": a_text, "C.docx": c_text, "B1.pdf": b1_text, "B3.xlsx": b3_text,
                    "B2 规格": S.B2_PAGE_TEXT, "缺陷草稿": dd_text}
    for label, text in corpus_texts.items():
        if _echo_regex().search(text):
            echo.append(f"{label} 含 GT 标记")
        hits = [rc for rc in _RULE_CODES if rc in text]
        if hits:
            echo.append(f"{label} 含规则码 {hits[:2]}")
    ev_leak = [f.name for f in (files / "evidence").glob("*.pdf")
               if _echo_regex().search(_pdf_text(f)) or any(rc in _pdf_text(f) for rc in _RULE_CODES)]
    if ev_leak:
        echo.append(f"证据件泄露标记：{ev_leak[:2]}")
    check("T4h 反标记回声：全语料零 GT/规则标记（大小写/全角变体）", not echo, "; ".join(echo))

    check("T4b B1 设备清单含三型号 + 中文大写日期 + 40 行",
          all(m in b1_text for m in [S.MODELS["server"], S.MODELS["switch"], S.MODELS["firewall_old"]])
          and S.DELIVERY_CN in b1_text and b1_text.count("含安装辅材") >= 13)
    check("T4c B2 无文本层（OCR 路径输入特征）+ GT-07 键条款在页面规格中",
          b2_text.strip() == "" and S.B2_KEY_CLAUSE in S.B2_PAGE_TEXT, f"提取到 {len(b2_text.strip())} 字符")
    # T4c-OCR（v0.4）：扫描件 OCR 通道真实性——嵌入图像须有像素文字 + tesseract 回读关键条款与四要素。
    # 背景：v0.4 前可变字体静默渲染空白（图像只有公章没有字），OCR 通道建立在空图像上未被任何测试发现。
    # issue #9：tesseract/chi_sim 缺件 → SKIP（不伪装成业务 FAIL）
    _ocr_missing = [t for t in ("tesseract", "chi_sim.traineddata") if t in _ENV_MISSING]
    if _ocr_missing:
        skip("T4c-OCR B2 扫描件 OCR 回读（像素层有字 + 条款归一化包含 + 四要素）",
             f"ENV_MISSING: {', '.join(_ocr_missing)}")
    else:
        ocr_note = _b2_ocr_roundtrip(files / "B2-投标函格式页-扫描件.pdf")
        check("T4c-OCR B2 扫描件 OCR 回读（像素层有字 + 条款归一化包含 + 四要素）",
              ocr_note == "ok", ocr_note if ocr_note != "ok" else "OCR 通道真实可用")
    check("T4d B3 图纸目录 GC-01~GC-20 在位", all(f"GC-{i:02d}" in b3_text for i in range(1, 21)))
    check("T4e 补遗 C 三变更在位（新型号/新开标日/新控制价）",
          S.MODELS["firewall_new"] in c_text and "2026-11-27 09:30" in c_text and "人民币 465 万元（含税）" in c_text)

    # T4g：TC-05 缺陷草稿读回——8 类缺陷样本逐字在位
    dm = json.loads((CORPUS / "defect-drafts" / "defect-map.json").read_text(encoding="utf-8"))
    dmiss = [d["rule_code"] for d in dm["defects"] if d["excerpt"] not in dd_text]
    check("T4g 缺陷草稿 8 类缺陷样本逐字在位", len(dm["defects"]) == 8 and not dmiss, str(dmiss))

    ev_dir = files / "evidence"
    ev_ok, ev_note = True, ""
    ev2 = _pdf_text(ev_dir / "EV-02-电子与智能化工程专业承包二级资质证书.pdf")
    ev13 = _pdf_text(ev_dir / "EV-13-安全生产许可证.pdf")
    if "2026 年 11 月 25 日" not in ev2:
        ev_ok, ev_note = False, "EV-02 有效期 2026-11-25 未见于证据"
    if "2025 年 08 月 01 日" not in ev13:
        ev_ok, ev_note = False, "EV-13 过期日 2025-08-01 未见于证据"
    check("T4f 证据陷阱在位（EV-02 临界 11-25；EV-13 过期 2025-08-01）", ev_ok, ev_note)


# ---------------------------------------------------------------- T7~T12：M1 Rust 流水线
def _cargo() -> str | None:
    return shutil.which("cargo") or str(Path.home() / ".cargo" / "bin" / "cargo")


def _m1_run(scenario: str, out: Path) -> tuple[int, str]:
    p = subprocess.run([str(M1_BIN), "run", "--scenario", scenario,
                        "--corpus", str(CORPUS / "files"),
                        "--queries", str(FIXTURES / "queries" / "exact-25.jsonl"),
                        "--out", str(out)],
                       capture_output=True, text=True, cwd=REPO)
    return p.returncode, p.stdout + p.stderr


def _grade_dims(run_dir: Path) -> dict:
    """判分一次 M1 运行：返回 {维度: pass}（report.json 由 grade.py 写入）。"""
    subprocess.run([sys.executable, "scripts/grade.py", "--run", str(run_dir)],
                   capture_output=True, text=True, cwd=REPO)
    rep = json.loads((run_dir / "report.json").read_text(encoding="utf-8"))
    return {k: bool(v.get("pass")) for k, v in rep["dimensions"].items()}


def _anchor_hit(block_texts: list[str], excerpt: str) -> bool:
    """锚点命中双口径：① 单块逐字包含（原口径，文本层锚点走此路，严格度不变）
    ② 回退：_norm_cn 归一化 + 相邻两块拼接——OCR/解析按视觉行出块，跨行锚点
    （如 B2 GT-07 条款在扫描件上拆为 …作否决投 + 标处理 两行）需相邻块拼接才能逐字命中。
    与 T4c 全页归一化同纪律且更严：限定相邻块对，不做全文件拼接/模糊匹配。"""
    if any(excerpt in t for t in block_texts):
        return True
    ex = _norm_cn(excerpt)
    if not ex:
        return False
    norms = [_norm_cn(t) for t in block_texts]
    return any(ex in norms[i] + norms[i + 1] for i in range(len(norms) - 1))


def t_m1():
    print("[T10] m1 并行线 Rust 工具链（构建 + 单测）")
    # issue #9：m1 链路依赖 poppler（pdftotext/pdfimages）+ tesseract + chi_sim——
    # 缺件则 T10~T13 整组 SKIP（测试需真实 OCR 通道，非业务失败）
    if not require_tools_or_skip(
            "T10~T13 m1 并行线（构建/单测/场景/覆盖/确定性）",
            ["pdftotext", "pdfinfo", "tesseract", "chi_sim.traineddata"],
            "m1 OCR 链路"):
        return
    cargo = _cargo()
    if not cargo:
        check("T10a cargo 可用（~/.cargo/bin）", False, "未找到 cargo 工具链")
        return
    b = subprocess.run([cargo, "build", "--release"], capture_output=True, text=True,
                       cwd=M1_DIR, env={**os.environ, "PATH": f"{Path.home()}/.cargo/bin:{os.environ['PATH']}"})
    ok_build = b.returncode == 0 and M1_BIN.exists()
    check("T10b m1 构建成功（cargo build --release）", ok_build,
          fail_detail(b.stdout + b.stderr) if not ok_build else "")
    t = subprocess.run([cargo, "test", "--release"], capture_output=True, text=True,
                       cwd=M1_DIR, env={**os.environ, "PATH": f"{Path.home()}/.cargo/bin:{os.environ['PATH']}"})
    n_ok = t.stdout.count("... ok") + t.stdout.count("test result: ok")
    check("T10c m1 单测全绿（状态机守卫/幂等/OCR 规整）", t.returncode == 0 and "0 failed" in t.stdout,
          fail_detail(t.stdout + t.stderr) if t.returncode else f"{n_ok} 项")
    if not ok_build:
        return

    runs = REPO / "runs"
    runs.mkdir(exist_ok=True)
    for sc in SCENARIOS:
        code, out = _m1_run(sc, runs / sc)
        if code != 0:
            check(f"T11 {sc} 场景运行（step expects 自检）", False, fail_detail(out))
            return
    check("T11a 四场景运行 step expects 自检零失败（TC-01/02/07/11）", True,
          "create→import×3→extract→chunk→search×5→replay→guard 全部通过")

    # 四场景判分：m1 范围四维全绿 + 失败集恰为 pending 三项
    for sc in SCENARIOS:
        dims = _grade_dims(runs / sc)
        bad = [d for d in M1_SCOPE if not dims.get(d)]
        unexpected = {d for d, v in dims.items() if not v} - M1_PENDING
        check(f"T11b {sc} 判分：G1/G2/G5/G7 全绿且无范围外失败",
              not bad and not unexpected,
              f"失败维度: {sorted((d for d, v in dims.items() if not v))}" if bad or unexpected else "")

    # T12：m1 V1-a 解析覆盖（blocks.jsonl 对回 GT 锚点——corpus-A 100% 标准）
    print("[T12] m1 V1-a 解析覆盖（m1 实际产出 vs GT 锚点）")
    blocks: dict[str, list[str]] = {}
    for line in (runs / "TC-01" / "blocks.jsonl").read_text(encoding="utf-8").splitlines():
        blk = json.loads(line)
        blocks.setdefault(blk["file"], []).append(blk["text"])
    miss, total = [], 0
    for r in S.REQUIREMENTS:
        for a in r["anchors"]:
            total += 1
            if not _anchor_hit(blocks.get(a["file"], []), a["excerpt"]):
                miss.append(f"{r['gt_id']}@{a['file']}")
    check(f"T12 锚点覆盖 {total - len(miss)}/{total}（corpus-A 100% 标准；含 GT-07×2 扫描件 OCR 跨行拼接 / GT-20×2 日期歧义）",
          not miss, "; ".join(miss[:3]))

    # T13：m1 确定性（双跑字节一致，含 OCR 通道）
    print("[T13] m1 确定性（同场景双跑四工件字节一致）")
    with tempfile.TemporaryDirectory() as td:
        d1, d2 = Path(td) / "r1", Path(td) / "r2"
        _m1_run("TC-01", d1)
        _m1_run("TC-01", d2)
        diff = [f for f in ("db.json", "trace.jsonl", "blocks.jsonl", "chunks.jsonl")
                if (d1 / f).read_bytes() != (d2 / f).read_bytes()]
        check("T13 双跑 db/trace/blocks/chunks 字节一致（OCR 通道含 OMP_THREAD_LIMIT=1 钉版）",
              not diff, "; ".join(diff))


# ---------------------------------------------------------------- T5/T6
def t_grade():
    print("[T5] 判分器自测")
    code, out = run(["scripts/grade.py", "--self-test"])
    check("T5 场景全绿 + 13 类破坏全捕获（含 4 类 R2 攻击样本）", code == 0,
          fail_detail(out) if code else out.strip().splitlines()[-1] if out else "")


def t_determinism():
    print("[T6] 夹具确定性（重生成哈希比对）")
    with tempfile.TemporaryDirectory() as td:
        code, out = run(["scripts/fixture_gen.py", "--out", td])
        if code != 0:
            check("T6 生成器可重跑", False, out[-200:])
            return
        m1 = json.loads((CORPUS / "manifest.json").read_text(encoding="utf-8"))
        m2 = json.loads((Path(td) / "corpus-A" / "manifest.json").read_text(encoding="utf-8"))
        f1 = {f["path"]: f["sha256"] for f in m1["files"]}
        f2 = {f["path"]: f["sha256"] for f in m2["files"]}
        diff = [p for p in f1 if f1[p] != f2.get(p)]
        check(f"T6 {EXPECTED_MANIFEST_FILES} 文件重生成 sha256 全一致",
              not diff and len(f1) == len(f2) == EXPECTED_MANIFEST_FILES, "; ".join(diff[:3]))


# ---------------------------------------------------------------- T7（V1 四段验证）
def t_v1():
    """V1 全链路四段验证（verification-plan §3.1）：解析覆盖→引用回填→导出保真→暗标口径。
    共享一个临时目录：明标件构建一次供 T7b/c/d/e 复用（LO 渲染 3 次，约 15s）。"""
    print("[T7] V1 四段验证（解析覆盖 / 引用回填 / 导出保真 / 暗标口径）")
    # issue #9：导出保真段需 soffice（LO headless 渲染）——缺件则 T7 整组 SKIP
    if not require_tools_or_skip(
            "T7 V1 四段验证（解析覆盖 / 引用回填 / 导出保真 / 暗标口径）",
            ["soffice", "pdftotext"], "LO headless 导出渲染"):
        return
    import v1_citation
    import v1_darkbid
    import v1_export
    import v1_pipeline

    # T7a 解析覆盖
    rep_a = v1_pipeline.build_parse_report()
    ca, cr = rep_a["corpus_a"], rep_a["corpus_r"]
    check("T7a corpus-A 文本层锚点 26/26 + 扫描层登记 1（B2 OCR 路径）",
          ca["anchors"]["located"] == 26 and ca["anchors"]["text_layer_total"] == 26
          and ca["anchors"]["scan_deferred"] == 1 and not ca["anchors"]["failures"],
          "; ".join(f"{f['gt_id']}:{f['reason']}" for f in ca["anchors"]["failures"][:3]))
    check("T7a corpus-R 结构探针 11/11 + G7 归因检出 5/5",
          cr["locatable"]["pass"] == 11 and cr["locatable"]["expect"] == 11
          and cr["gap_attributed"]["detected"] == 5,
          "; ".join(f"{p['probe']}" for p in cr["probes"] if p["status"] == "not_found"))

    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        bright = td / "技术标-导出样本.docx"
        dark = td / "技术标-暗标样本.docx"
        info = v1_export.build_bid_docx(bright, "bright", workdir=td)
        v1_export.build_bid_docx(dark, "dark", workdir=td)

        # T7b 引用回填
        rep_b = v1_citation.run_citation_checks(bright, td)
        check("T7b 引用链 10 条回查全 valid（位置+指纹两级）", rep_b["R1_recheck"]["pass"])
        check("T7b 内容改写恰致 1 链 stale_content（其余 valid，只降不升）",
              rep_b["R2_content_stale"]["pass"])
        check("T7b 结构漂移致索引型定位器全体失效 + 书签重验恢复",
              rep_b["R3_structure_drift"]["pass"] and rep_b["R3b_bookmark_rebind"]["pass"])

        # T7c 导出保真
        rep_c = v1_export.run_fidelity_checks(bright, td / "lo_c")
        st, rd = rep_c["structure"], rep_c["render"]
        check("T7c 结构读回（书签10/域/图片/分页7/合并表/双分节/盖章位）",
              all(st.values()), "; ".join(k for k, v in st.items() if not v))
        check(f"T7c LO 渲染（{rd['pages']} 页：分页纪律/页眉页脚/图片在技术方案页/引用可见）",
              all(v for k, v in rd.items() if k != "pages"),
              "; ".join(k for k, v in rd.items() if k != "pages" and not v))

        # T7d 暗标口径（SC-06 后为五断言：页数/禁字/元数据/金丝雀/标识符面）
        rep_d = v1_darkbid.run_darkbid_checks(dark, bright, td)
        check(f"T7d 暗标五断言（页数 {rep_d['D1_pages']['pages']}≤上限且明暗一致 / 禁字双面零命中 / "
              f"元数据清零 / 金丝雀检出 {len(rep_d['D4_canary']['hits'])} 处防空洞绿 / "
              f"标识符面 {rep_d['D5_identifier_scan']['scan']['ident_count']} 个零命中+金丝雀）",
              rep_d["pass"],
              json.dumps({k: rep_d[k] for k in ("D2_forbidden_scan", "D3_metadata_zeroing", "D5_identifier_scan")},
                         ensure_ascii=False)[:150] if not rep_d["pass"] else "")

        # T7e 导出件确定性（freeze_docx 字节级）
        again = td / "again.docx"
        info2 = v1_export.build_bid_docx(again, "bright", workdir=td)
        check("T7e 明标导出件重生成 sha256 一致（确定性）",
              info["docx_sha256"] == info2["docx_sha256"])


# ---------------------------------------------------------------- T8（V2 影响面验证）
def t_v2():
    """V2 补遗/答疑/版本影响面计算（verification-plan §3.2）：TC-09 契约消费 +
    级联完整性 + 无关不重跑 + 矛盾终裁门禁 + 幂等 + 删除/新增规则（影响面规则为
    bid-domain-model §2 设计语义的 Python 复刻 = SEMUL 口径，交叉验证待 cargo）。"""
    print("[T8] V2 影响面验证（TC-09 主场景 / 级联 / 无关不重跑 / 终裁门禁 / 幂等 / 规则覆盖）")
    import v2_impact
    report = v2_impact.run_v2()
    r = report["results"]
    check("T8 TC-09 断言契约全消费（expected-transitions 六断言）",
          r["R0_tc09_contract"]["pass"])
    check("T8 主场景 DIRTY 恰三章节 + GT-18 v2 + 冲突双版本登记 + 版本戳恰 +1",
          r["R1_tc09_main"]["pass"])
    check("T8 开标日级联：全项目重检 + GT-02 翻转 EXPIRED + EV-13 维持禁绑 + 零误翻",
          r["R2_cascade"]["pass"])
    check("T8 无关不重跑：售后/团队/格式三章零误伤",
          r["R3_no_overreach"]["pass"])
    check("T8 矛盾终裁门禁：脏阻断→未裁决阻断→终裁后放行 + 口径 465",
          r["R4_adjudication_gate"]["pass"])
    check("T8 同 sha 重复导入 no-op（幂等）", r["R5_idempotent"]["pass"])
    check("T8 删除/新增规则（SYNTHETIC）：绑定解除+行移除恰一章节 / 新增零误伤",
          r["R6_delete_add_rules"]["pass"])


# ---------------------------------------------------------------- T9（V3 内核集成三问）
def t_v3():
    """V3 文档内核集成边界（verification-plan §3.3）：三问断言（设计语义 SEMUL——
    doc-engine-audit §5 映射契约；引擎接入后升 ENGINE_RUN）。"""
    print("[T9] V3 内核集成三问（重写引用保持 / 撤销只降不升 / 导出中断一致性）")
    import v3_integration
    with tempfile.TemporaryDirectory() as td:
        report = v3_integration.run_v3(Path(td))
    r = report["results"]
    check("T9 Q1 五轮重写：每轮恰一链降级 + 重验恢复 + 无悬空书签",
          r["Q1_rewrite_5rounds"]["pass"])
    check("T9 Q2 撤销只降不升（undo 不自发恢复）+ 外部修改不静默 adopt",
          r["Q2_undo_and_reopen"]["pass"])
    check("T9 Q3 两阶段导出：中断停 EXPORTING 零假 EXPORTED + 恢复幂等同 sha",
          r["Q3_export_interrupt"]["pass"])


# ---------------------------------------------------------------- T10（M1.7 真实数据回归）
def t_m17():
    """M1.7 政府采购真实链（S2 批次一，research/s2-scenario-plan.md）：
    真实文件解析→变更提取→影响面→输出复验→GT 草案核对（34 断言）+ 盘点在案检查。
    真实数据层回归：防止新解析器/新样本引入绕过（与 T5 攻击样本回归同目的）；
    文件级（招标文件本体）为在案待人工项，不得宣称文件级验收。"""
    print("[T14] M1.7 政府采购真实链（cuc-028 三节点公告级闭环 + GT 草案）")
    rc = REPO / "real-corpus" / "gov-procurement"
    stats = json.loads((rc / "stats.json").read_text(encoding="utf-8"))
    check("T14a 真实语料盘点在案（576 份官方公告：224 招标/174 更正/178 中标，溯源齐全）",
          stats["total_files"] == 576
          and stats["by_type"] == {"award": 178, "correction": 174, "tender": 224}
          and stats["with_url"] == 576,
          json.dumps(stats.get("by_type", {}), ensure_ascii=False))
    code, out = run(["scripts/m17_real_chain.py"])
    check("T14b 真实链闭环全绿（P0 解析/P1 变更/P2 影响面/P3 复验/P4 GT = 34 断言；报告含 sha256 溯源）",
          code == 0 and "全绿" in out, fail_detail(out) if code else "")




# ---------------------------------------------------------------- T15（M1.7-C GT 草案四要素）
def t_gt_enrich():
    """M1.7-C：P2 真实文件评测 GT 草案 24 条四要素完备性回归（artifacts/real-eval）。
    口径：draft_pending_human 未冻结——本用例只锁结构与定位核验状态，不把草案指标当验收。"""
    print("[T15] M1.7-C GT 草案四要素（原文定位核验/标准答案/判定理由/复核状态）")
    gt = json.loads((REPO / "artifacts" / "real-eval" / "ground-truth-draft.json").read_text(encoding="utf-8"))
    items = gt["items"]
    ok_fields = all(i.get("expected") and i.get("rationale") and i.get("review_status") == "draft_pending_human"
                    and i.get("anchor_verification", {}).get("readback") == "PASS" for i in items)
    check("T15a 24 条四要素完备且定位核验全 PASS（20 verbatim/2 normalized/2 keyword；0 存疑）",
          len(items) == 24 and ok_fields
          and gt["enrichment"]["anchor_stats"].get("unlocated") == 0
          and gt["enrichment"]["verbatim_fixed"] == ["GT-R01", "GT-T01"],
          f"n={len(items)}")
    check("T15b 争议条目注记在案（7 条退回候选：T02 改判/B04 类别/R05 拆列/S03-05 跨行/V04 跨行/R01 T01 转写修正）",
          sorted(gt["enrichment"]["disputed_items"]) ==
          ["GT-B04", "GT-R05", "GT-S03", "GT-S04", "GT-S05", "GT-T02", "GT-V04"]
          and all("review_note" in next(i for i in items if i["gt_id"] == k)
                  for k in gt["enrichment"]["disputed_items"]),
          str(gt["enrichment"]["disputed_items"]))

def t_telecom():
    """M1.7-TEL：通信行业真实样本（S2 场景扩充·txzbqy 工信部法定平台）。
    A批次盘点（20 节点/16 文件溯源+哈希）+ B批次四节点链 46 断言（招标→失败→重招→公示）
    + 文件级解析（Rust 亲跑 17/17 + 2040 条款 + 14/14 跨向锚点）+ C批次 GT 草案 18 条四要素。
    口径：文件级真实测试已亲跑；GT 未冻结仅探索性指标，不宣称验收。"""
    print("[T16] M1.7-TEL 通信行业真实链（txzbqy 四节点链 + 文件级解析 + GT 草案）")
    rc = REPO / "real-corpus" / "telecom"
    fr = json.loads((rc / "fetch-report.json").read_text(encoding="utf-8"))
    nodes = fr["nodes"]
    files = fr["files"]
    n_ok = sum(1 for f in files if f.get("status") == "OK")
    n_mm = sum(1 for f in files if f.get("status") == "MD5_MISMATCH")
    check("T16a 电信语料盘点在案（20 节点：招标/变更 10 + 中标公示 10；16 文件溯源+哈希，14 md5 校验 OK + 2 MISMATCH 双记录）",
          len(nodes) == 20 and sum(1 for n in nodes if n["node_type"] == "bulletin") == 10
          and sum(1 for n in nodes if n["node_type"] == "publicity") == 10
          and len(files) == 16 and n_ok == 14 and n_mm == 2
          and all((f.get("sha256") or f.get("md5_platform")) for f in files),
          f"nodes={len(nodes)} files={len(files)} ok={n_ok} mm={n_mm}")
    if env_ok(["pdftotext"]):
        code, out = run(["scripts/telecom_chain.py"])
        check("T16b 电信四节点链闭环全绿（P0 溯源/P1 变更提取/P2 影响面/P3 输出复验/P4 文件级 = 46 断言）",
              code == 0 and "46/46" in out, fail_detail(out) if code else "")
    else:
        skip("T16b 电信四节点链闭环全绿（P0 溯源/P1 变更提取/P2 影响面/P3 输出复验/P4 文件级 = 46 断言）",
             "ENV_MISSING: pdftotext（文件级解析通道）")
    flr = json.loads((rc / "file-level-report.json").read_text(encoding="utf-8"))
    check("T16c 文件级解析亲跑在案（Rust 17/17 真实文件 + 条款抽取≥2000（资格/否决/评分非零）+ 跨向锚点 14/14 PASS）",
          len(flr["parse_summary"]) == 17 and not flr["parse_failures"]
          and flr["requirements_n"] >= 2000
          and all(flr["requirements_by_category"].get(c, 0) > 0
                  for c in ("资格条件", "否决项", "评分项"))
          and len(flr["anchor_checks"]) == 14 and all(c["pass"] for c in flr["anchor_checks"]),
          f"files={len(flr['parse_summary'])} reqs={flr['requirements_n']}")
    gt = [json.loads(l) for l in (rc / "gt" / "ground-truth-draft.jsonl").read_text(encoding="utf-8").strip().splitlines()]
    ok4 = all(i.get("expected") and i.get("rationale") and i.get("review_status") == "draft_pending_human"
              and i.get("locate_check", {}).get("status") == "PASS" for i in gt)
    check("T16d 电信 GT 草案 18 条四要素完备且定位核验全 PASS（17 verbatim/1 normalized；draft_pending_human 未冻结）",
          len(gt) == 18 and ok4, f"n={len(gt)}")


def t_engine_diff():
    """T17 · T1 双跑差分工具（issue #2 评论 2026-10-27 03:28 派发）＋ T3 后验收验：
    ① 合成注入自测五分类全命中；② 修复前证据（f8c860a）；③ 修复后证据（f7ddd39——
    引擎 #17/#18/#19 合入后标书侧独立验收复测）。"""
    print("[T17] engine_diff 双跑差分工具（T1 接口预演 + T3 验收复测）")
    import engine_diff

    fails = engine_diff.selftest()
    check("T17a 差分分类合成自测（EQUIVALENT/ENGINE_MISSING(D6-INS)/OLD_BUG(D6-DEL-LEAK)/EXPECTED_DIFF(A2)/UNCLASSIFIED）",
          not fails, "; ".join(fails[:3]))

    def _totals(path):
        rows = json.loads(Path(path).read_text(encoding="utf-8"))
        tot: dict[str, int] = {}
        for r in rows:
            for k, v in r["verdict_counts"].items():
                tot[k] = tot.get(k, 0) + v
        return len(rows), tot, sum(r["old_blocks"] for r in rows), sum(r["new_blocks"] for r in rows)

    # ② 修复前（f8c860a 基线）：9 引擎缺口面（D6-INS×7 + D4-TEXTBOX×2）与 B2 目录对齐
    n, tb, ob, nb = _totals(REPO / "artifacts/engine-diff/summary.json")
    check("T17b 修复前证据（f8c860a）：8 对 54 等价 + 9 引擎缺口（D6/D4）+ 2 旧泄漏 + 4 预期差",
          n == 8 and tb.get("EQUIVALENT") == 54 and tb.get("ENGINE_MISSING") == 9
          and tb.get("OLD_BUG") == 2 and tb.get("EXPECTED_DIFF") == 4
          and ob == 65 and nb == 66,
          f"files={n} counts={tb}")

    # ③ 修复后（f7ddd39：mdcore#17/#18/#19 合入）：引擎缺口清零，bid 侧 4 缺陷显影
    n2, ta, oa, na = _totals(REPO / "artifacts/engine-diff-after/summary.json")
    check("T17c 验收复测（f7ddd39）：8 对 61 等价 + **0 引擎缺口** + 4 旧侧缺陷（泄漏×3+双重提取×1）+ 3 预期差",
          n2 == 8 and ta.get("EQUIVALENT") == 61 and ta.get("ENGINE_MISSING", 0) == 0
          and ta.get("OLD_BUG") == 4 and ta.get("EXPECTED_DIFF") == 3
          and ta.get("UNCLASSIFIED", 0) == 0 and oa == 65 and na == 67,
          f"files={n2} counts={ta}")

    # ④ 探针后验证据：A1/A2/#17/#18/#19 逐面在案
    rep = json.loads((REPO / "artifacts/engine-probe/report-after-fixes-f7ddd39.json").read_text("utf-8"))
    wc = rep["d456_tr2_word_complex_engine"]
    r6b = rep["d6rev_r6b_pure_delete_engine"]
    d2a = rep["d456_corpus_r_d2a_engine"]
    r6d = rep["d6rev_r6d_comments_engine"]
    check("T17d 探针验收（f7ddd39）：A2 修复（word-complex 读回 1.1）+ A1 修复（headings=4）+ #17（R6b warnings=2 计数）/ #18（D2a text_boxes=1）/ #19（R6d comments=2）",
          json.loads(wc["block_texts"]).__contains__("1.1 表格")
          and wc["heading_style_paras"] == "4"
          and r6b["warnings_n"] == "2" and r6b["revision_inserted_runs"] == "0"
          and d2a["text_boxes_n"] == "1" and d2a["comments_n"] == "1"
          and d2a["revision_inserted_runs"] == "1"
          and r6d["comments_n"] == "2",
          "探针后验字段不符")


def main() -> int:
    print("=" * 62)
    print("ai-bid-research 每轮测试（T0 环境预检 + V0 夹具 + 一致性 + 判分器 + V1/V2/V3 + m1 并行线 + M1.7 真实链 T14/T15 + M1.7-TEL T16 + T17 差分工具）")
    print("=" * 62)
    t_env_preflight()
    t_xref()
    t_fixture_integrity()
    t_readback()
    t_grade()
    t_determinism()
    t_v1()
    t_v2()
    t_v3()
    t_m1()
    t_m17()
    t_gt_enrich()
    t_telecom()
    t_engine_diff()
    print("-" * 62)
    # issue #9 判定标准：N passed / M skipped / K failed（依赖缺失 ≠ 业务失败）
    print(f"结果：{len(_passed)} 通过 / {len(_skipped)} 跳过 / {len(_failed)} 失败")
    if _skipped:
        print("跳过项（环境缺件，非业务失败）：", _skipped)
    if _failed:
        print("失败项：", _failed)
    write_env_blocked()
    return 0 if not _failed else 1


if __name__ == "__main__":
    sys.exit(main())
