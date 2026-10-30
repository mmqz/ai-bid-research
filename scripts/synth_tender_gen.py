#!/usr/bin/env python3
"""synth_tender_gen.py — fixtures/synth-tender 合成招标夹具生成器（#80 路线 1）

消费 synth_tender_spec.py 生成：
  fixtures/synth-tender/招标文件/
    A-招标文件.docx（~30 页，6 章结构 + 前附表 + 坑 T1-T7）
    B-技术规范书.docx（~10 页，参数表——引擎现状无抽取路径 G1）
    C-补遗文件.pdf（2 页文本层，T4 跨行条款 + 控制价调整）
    D-图纸说明.md（非办公扩展名——fail-closed 显式拒绝面）
    E-原招标公告盖章页-扫描件.pdf（T5 无文本层图像页）
  fixtures/synth-tender/参考答案/
    expected-requirements.json / expected-outline.json / expected-format-spec.json /
    expected-validation.json / expected-import.json
  fixtures/synth-tender/README.md + manifest.json（sha256 自完整性）

确定性纪律（corpus-A 同款）：freeze_zip / reportlab invariant / PIL 静态字体。
自检：excerpt_key ⊆ 源文本；坑位在场；页数估算区间。

用法：python3 scripts/synth_tender_gen.py [--out fixtures/synth-tender]
"""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

import reportlab.rl_config
reportlab.rl_config.invariant = 1  # 确定性 PDF

from openpyxl.styles import Alignment  # noqa: F401  (保持与 fixture_gen 同款依赖面)
from reportlab.lib.pagesizes import A4
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.cidfonts import UnicodeCIDFont
from reportlab.pdfgen import canvas as rl_canvas

sys.path.insert(0, str(Path(__file__).resolve().parent))
import synth_tender_spec as S  # noqa: E402
from ooxml_util import freeze_zip, sha256_file, write_json  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO / "fixtures" / "synth-tender"

pdfmetrics.registerFont(UnicodeCIDFont("STSong-Light"))
CN_FONT = "STSong-Light"
NOTO_PATH = "/usr/share/fonts/truetype/chinese/SarasaMonoSC-Regular.ttf"


# ---------------------------------------------------------------- docx 构建
def _build_docx(blocks: list[tuple], doc, filler_titles: list[str], filler_lead: str,
                filler_head_fmt: str):
    """按块渲染 DOCX + 填充节；返回 (块数, 表数, 显式分页数)。

    filler 节：h2（保持章上下文——as_subsection 一、二、…形态）+ 3 段无编号
    无信号词散文 + pb。散文体 = 现实需求书细化条款，抽取面不触发（设计如此：
    让 REQ 集 = 设计条款集，评测口径干净）。
    """
    paras = tables = breaks = 0
    for block in blocks:
        kind = block[0]
        if kind == "h1":
            doc.add_heading(block[1], level=1)
        elif kind == "h2":
            doc.add_heading(block[1], level=2)
        elif kind == "p":
            doc.add_paragraph(block[1])
        elif kind == "table":
            headers, rows = block[1], block[2]
            t = doc.add_table(rows=1 + len(rows), cols=len(headers))
            t.style = "Table Grid"
            for j, h in enumerate(headers):
                t.rows[0].cells[j].text = h
            for i, row in enumerate(rows):
                for j, cell in enumerate(row):
                    t.rows[i + 1].cells[j].text = cell
            tables += 1
        elif kind == "pb":
            doc.add_page_break()
            breaks += 1
        paras += 1
    cn_ord = "一二三四五六七八九十"
    for i, title in enumerate(filler_titles):
        n = 7 + i  # A 文件第五章已有一~六节；B 文件从七续（B 侧单独计数无冲突——两文件独立）
        ordinal = _cn(n)
        doc.add_heading(filler_head_fmt.format(ordinal=ordinal, title=title), level=2)
        doc.add_paragraph(filler_lead.format(title=title))
        doc.add_paragraph(
            f"本节所涉设备、软件与服务的型号规格、数量及验收口径，均以第五章技术需求书"
            f"（或本规范书）对应条款为准；本节未尽事宜按国家现行标准执行。")
        doc.add_paragraph(
            "承包人应在投标文件中对本节要求逐项应答，明确响应程度（满足/部分满足/偏离），"
            "并对偏离项说明原因及替代方案；未应答的条款按不利于投标人处理。")
        doc.add_page_break()
        breaks += 1
        paras += 4
    return paras, tables, breaks


def _cn(n: int) -> str:
    """1-30 → 中文序数（十一/二十一形态）。"""
    if n <= 10:
        return "一二三四五六七八九十"[n - 1]
    if n < 20:
        return "十" + "一二三四五六七八九"[n - 11]
    return "二十" + ("一二三四五六七八九"[n - 21] if n != 30 else "十")


def gen_docx(blocks, filler_titles, filler_lead, filler_head_fmt, out: Path, name: str,
             author_time):
    from docx import Document
    doc = Document()
    core = doc.core_properties
    core.author = "ai-bid-research synth-tender fixture"
    core.title = S.PROJECT["name"]
    import datetime
    core.created = core.modified = datetime.datetime(*author_time)
    paras, tables, breaks = _build_docx(blocks, doc, filler_titles, filler_lead, filler_head_fmt)
    path = out / name
    doc.save(path)
    freeze_zip(path)
    return {"path": name, "paragraph_blocks": paras, "tables": tables,
            "page_breaks": breaks, "estimated_pages": breaks + 1}


# ---------------------------------------------------------------- C.pdf（文本层，行受控）
def gen_c_pdf(out: Path):
    path = out / "C-补遗文件.pdf"
    c = rl_canvas.Canvas(str(path), pagesize=A4)
    c.setTitle("补遗文件 01 号")
    for page in S.C_PDF_PAGES:
        y = 810
        c.setFont(CN_FONT, 16)
        # 每页首行 = 标题（第 2 页无标题则直接正文体）
        first = page[0] if page and page[0] else None
        idx = 0
        if page and page[0] == "" and len(page) > 1:
            idx = 1
        if _is_page_title(page):
            c.setFont(CN_FONT, 15)
            c.drawCentredString(297.5, y, S.C_PDF_TITLE)
            y -= 34
            idx = 1
        c.setFont(CN_FONT, 11)
        while idx < len(page):
            line = page[idx]
            if line:
                c.drawString(72, y, line)
            y -= 22
            idx += 1
        c.showPage()
    c.save()
    return {"path": path.name, "pages": len(S.C_PDF_PAGES),
            "nonempty_lines": sum(1 for p in S.C_PDF_PAGES for l in p if l)}


def _is_page_title(page: list[str]) -> bool:
    """第 1 页以标题行开头（首元素是项目编号行——标题独立绘制）。"""
    return bool(page) and page[0].startswith("项目编号：")


# ---------------------------------------------------------------- E.pdf（扫描件：图像页无文本层）
def gen_e_pdf(out: Path):
    from PIL import Image, ImageDraw, ImageFont
    from reportlab.lib.utils import ImageReader
    import io

    W, H = 1240, 1754  # A4 @150dpi
    img = Image.new("L", (W, H), 248)
    d = ImageDraw.Draw(img)
    font_t = ImageFont.truetype(NOTO_PATH, 56)
    font_b = ImageFont.truetype(NOTO_PATH, 30)
    y = 120
    w = d.textlength(S.E_SCAN_TITLE, font=font_t)
    d.text(((W - w) / 2, y), S.E_SCAN_TITLE, fill=30, font=font_t)
    y += 110
    for text in S.E_SCAN_LINES:
        d.text((110, y), text, fill=30, font=font_b)
        y += 64
    y += 60
    d.text((110, y), S.E_SCAN_STAMP, fill=30, font=font_b)
    y += 90
    d.text((110, y), "2026 年 10 月 20 日", fill=30, font=font_b)
    # 红色公章（椭圆占位——扫描件特征）
    d.ellipse((880, y - 140, 1180, y + 160), outline=120, width=6)
    for i in range(5):
        d.arc((950 + i, y - 100 + i, 1110 - i, y, ), start=200, end=340, fill=110, width=3)
    d.text((960, y + 40), "（示例公章）", fill=110, font=font_b)

    # 像素级自证（corpus-A B2 v0.4 纪律）：文字必须真实渲染
    import numpy as _np
    _a = _np.array(img)
    assert int(_a.min()) <= 60, f"E 像素层无文字（min={_a.min()}）"
    assert (_a < 150).sum() >= 8000, f"E 暗像素过少（{(_a < 150).sum()}）"

    c = rl_canvas.Canvas(str(out / "E-原招标公告盖章页-扫描件.pdf"), pagesize=A4)
    c.setTitle("原招标公告盖章页（扫描件）")
    buf = io.BytesIO()
    img.save(buf, format="PNG", optimize=True)
    buf.seek(0)
    c.drawImage(ImageReader(buf), 0, 0, width=A4[0], height=A4[1])
    c.showPage()
    c.save()
    return {"path": "E-原招标公告盖章页-扫描件.pdf", "text_layer": False, "pages": 1}


# ---------------------------------------------------------------- 参考答案 / README / manifest
def gen_answers(out: Path):
    d = out / "参考答案"
    d.mkdir(parents=True, exist_ok=True)
    write_json(d / "expected-requirements.json", {
        "fixture": "synth-tender",
        "design_note": "人工核对的标准答案（ideal）+ 跑前人工推演的引擎行为预判（engine_expect）——"
                       "跑后不回改（防循环论证）；推演错误在评测报告中如实呈现。"
                       "v0.2（2026-10-30）：补登 SYN-A-013a/013b/015a（3.3.1 份数构成 / 3.3.2 正副本标注 / "
                       "3.3.6 中文编制——Lead #80 评论指令的答案修订）；三条的 engine_expect 按当日引擎"
                       "实测登记（已判对），非独立跑前推演——条目 note 中如实标注",
        "traps": S.TRAPS,
        "requirements": S.EXPECTED_REQUIREMENTS,
        "counts": {
            "total": len(S.EXPECTED_REQUIREMENTS),
            "by_ideal_category": _count_by(S.EXPECTED_REQUIREMENTS, "ideal_category"),
        },
        "engine_prediction": {
            "total": S.EXPECTED_ENGINE_TOTAL,
            "by_category": S.EXPECTED_ENGINE_BY_CATEGORY,
            "note": "A(74) + C(7) 推演；B/E 因 G1 无抽取路径为 0。"
                    "v0.2 补登的 3 条（SYN-A-013a/013b/015a）不进此冻结总账——"
                    "它们是跑后实测登记（见 design_note），预演面保持 v0.1 原样不动",
        },
    })
    write_json(d / "expected-outline.json", S.EXPECTED_OUTLINE)
    write_json(d / "expected-format-spec.json", S.EXPECTED_FORMAT_SPEC)
    write_json(d / "expected-validation.json", S.EXPECTED_VALIDATION)
    write_json(d / "expected-import.json", S.EXPECTED_IMPORT)
    return {"entries": len(S.EXPECTED_REQUIREMENTS)}


def _count_by(entries, key):
    out = {}
    for e in entries:
        out[e[key]] = out.get(e[key], 0) + 1
    return out


README_TMPL = """# synth-tender：合成招标文件夹具（#80 路线 1 · 收官测验参考基线）

> 定位：**不碰真标书的收官准备**。按政采/七部委范本结构合成的完整招标包，
> 带人工核对的参考答案——引擎每次改动后重跑本夹具，对照答案看有没有退化。

## 文件清单

| 文件 | 角色 | 引擎行为（现状） |
|---|---|---|
| 招标文件/A-招标文件.docx | 主文件（~30 页，6 章结构+前附表） | 解析 ✓ 抽取 ✓（A- 前缀 docx 主文件路径） |
| 招标文件/B-技术规范书.docx | 技术规范分册（~10 页参数表） | 解析 ✓ 抽取 ✗（**G1**：B- 前缀无路径） |
| 招标文件/C-补遗文件.pdf | 补遗（2 页文本层：开标变更/控制价/否决补充/评分修正） | 解析 ✓ 抽取 ✓（C- 前缀 PDF 路径） |
| 招标文件/D-图纸说明.md | 图纸说明（Markdown） | 解析 ✗ 显式拒绝（fail-closed 既有设计） |
| 招标文件/E-原招标公告盖章页-扫描件.pdf | 盖章扫描页（无文本层） | 导入走 OCR 链 ✓ 抽取 ✗（**G1**） |

## 项目设定（虚构但自洽）

{name}（{code}）· 预算 {budget_note} · 工期 {duration} · 开标 {open_bid_date}
（补遗后 {open_bid_date_after_addendum}）。投标方响应本包时所需的事实全在
A/B/C/D/E 五件里——无外部依赖。

## 7 个坑（健壮性探针）

| 坑 | 位置 | 测什么 | 预期结果 |
|---|---|---|---|
| T1 | 前附表 行6/行10 | 否决项藏在表格里 | 前附表分支抽取（clause=前附表-N）✅ |
| T2 | 评标办法 4.1-4.4 | 评分项分值形态锁 | 评分项 + params.score ✅ |
| T2b | 评标办法 4.6 | 含分值的否决条款 | **G4**：被吞为评分项 ❌ |
| T3 | 须知 3.3.3 | 格式要求中文数字 | 字号 ✅；行距/边距 ❌（**G2**） |
| T4 | 须知 3.7.2(8) + 补遗 2.1 | 跨页/跨行条款 | docx 拆 2 条（**G5**）；PDF 重组还原 ✅ |
| T5 | E-扫描件 | 无文本层附录 | OCR 链 ✅；抽取无路径（**G1**）❌ |
| T6 | 须知 3.7.3 | 串通成组列举（子项无关键词） | 语境传播 9 条 ✅ |
| T7 | 须知 3.7.3 ↔ 评标办法章 | 双生文本 | 两处各抽（**G7** 不去重） |

## 已知引擎缺口（设计时登记，供差距报告引用）

- **G1** B-/E- 前缀文件无抽取路径（extract.rs 按文件名前缀分发：仅 A-/C-/B1/B2）
- **G2** 中文数字行距/边距不识别（format_spec.rs 只认 ASCII 数字）
- **G3** 正面信用要求含「失信」被误判否决项（负面清单词抢先）
- **G4** 评标办法章内含分值形态的否决条款被判评分项（分值形态锁盲面）
- **G5** docx 显式分页符打断条款 → 碎片化（空段丢弃不杀语境，但拆两条 REQ）
- **G6** 补遗无章节上下文 → 商务/评分修正条款落「其他」
- **G7** 双生文本不去重（dedupe_hash 含 locator——语义去重属 M2）

## 参考答案（人工核对，非引擎生成）

| 文件 | 内容 |
|---|---|
| expected-requirements.json | {n_entries} 条设计条款：ideal 分类 + engine_expect 跑前预判 |
| expected-outline.json | 评测用 OutlineSpec 下的章节树预测（6 类目 16 叶） |
| expected-format-spec.json | 理想格式 vs 引擎可识别面（T3 差距） |
| expected-validation.json | 全链完成后的校验预测（ok=true, blocker=0, rule5 告警面） |
| expected-import.json | 5 件的导入面预测（D 件 Failed 是预期正确行为） |

## 评测入口

```bash
cd bidcore && cargo test -p bid-mcp --test synth_tender_eval -- --nocapture
# 产物落盘 artifacts/synth-tender/run/latest/（含与参考答案的逐条 diff 报告）
```

## 跑后发现（v0.1 评测实录——2026-10-30）

设计期登记的 G1-G7 全部实锤（T1 前附表-6/10、T2 score=30/55/15/10、T4-pdf
跨行重组还原、T6 语境传播 9 条、T7 双生两处各抽、T3 边距回落基线 35/30）。
跑后新发现（v0.1 报告留痕，待 v0.2 答案修订与引擎侧裁决）：

- **F1** 节标题「七、开标、评标与否决条款」含「否决」→ 其下编号条款 3.7.1
  （开标程序）/3.7.4（评标纪律）被章节上下文整段判否决项（2 条 FP）
- **F2** 节标题「三、串通投标与弄虚作假的认定」自身被抽为否决项 REQ（标题非条款）
- **F3** 评标办法章内无分值无否决词的条款（4.7 评标结果排序）被判评分项（章节 blanket）
- **预测系统性偏差**：编号条款全量抽取（其他类 26 条 = 宁多勿漏设计面）——
  跑前推演 81 vs 实际 115；推演准确 94/95（设计条款面），偏差集中在全量抽取认知
- **答案侧补登（v0.2 已完成 2026-10-30）**：3.3.1/3.3.2/3.3.6 三条格式条款
  （份数构成/正副本标注/中文编制）已补入 ideal 答案（SYN-A-013a/013b/015a）——
  v0.1 评测实录保持原样不重写；下次评测按 98 条口径

## 纪律

1. 参考答案的 engine_expect 是【跑前人工推演】——跑后不许回改（防循环论证）。
2. 重生成：`python3 scripts/synth_tender_gen.py`（确定性输出，sha256 见 manifest.json）。
3. 本夹具不替代 corpus-A（GT 25 埋点件）——corpus-A 测引擎解析面，本件测
   【业务全链 + 真实结构 + 已知缺口的行为锁定】。
"""


def gen_readme(out: Path, n_entries: int):
    (out / "README.md").write_text(
        README_TMPL.format(n_entries=n_entries, **S.PROJECT), encoding="utf-8")


def gen_manifest(out: Path, stats: dict):
    files = []
    for p in sorted(out.rglob("*")):
        if p.is_file() and p.name != "manifest.json" and "run" not in p.parts:
            files.append({"path": str(p.relative_to(out)), "sha256": sha256_file(p),
                          "bytes": p.stat().st_size})
    write_json(out / "manifest.json", {
        "fixture": "synth-tender",
        "spec_version": "v0.1",
        "spec_source": "scripts/synth_tender_spec.py（内容单一真源）+ synth_tender_gen.py",
        "deterministic": True,
        "issue": "#80 路线 1（Lead 2026-10-30 13:24 指令：先造合成招标文件保底）",
        "stats": stats,
        "files": files,
    })
    return len(files)


# ---------------------------------------------------------------- 自检
def self_check(stats: dict):
    errs = []

    def src_text(name: str) -> str:
        if name.startswith("A-"):
            parts = [b[1] for b in S.A_DOC if b[0] in ("h1", "h2", "p")]
            for b in S.A_DOC:
                if b[0] == "table":
                    parts += [c for row in b[2] for c in row] + list(b[1])
            # 填充段不含设计条款——不在检核面
            return "\n".join(parts)
        if name.startswith("B-"):
            parts = [b[1] for b in S.B_DOC if b[0] in ("h1", "h2", "p")]
            for b in S.B_DOC:
                if b[0] == "table":
                    parts += [c for row in b[2] for c in row] + list(b[1])
            return "\n".join(parts)
        if name.startswith("C-"):
            return "\n".join(l for p in S.C_PDF_PAGES for l in p if l)
        if name.startswith("E-"):
            return "\n".join([S.E_SCAN_TITLE, *S.E_SCAN_LINES, S.E_SCAN_STAMP])
        return ""

    texts = {}
    for e in S.EXPECTED_REQUIREMENTS:
        fn = e["source_file"]
        if fn not in texts:
            texts[fn] = src_text(fn)
        if e["excerpt_key"] not in texts[fn]:
            errs.append(f"{e['syn_id']} excerpt_key 不在 {fn} 源文本中: {e['excerpt_key'][:30]}…")

    # 坑位在场
    a_txt = texts.get("A-招标文件.docx", "")
    if "未按本表第 9 项规定格式填写技术偏离表的" not in a_txt:
        errs.append("T1a 前附表否决行缺失")
    if "统一调整为" not in texts.get("C-补遗文件.pdf", ""):
        errs.append("T4-pdf 跨行条款缺失")
    for probe, tag in [("价格分（30 分）", "T2"), ("行距为一点五倍行距", "T3"),
                       ("视为投标人相互串通投标", "T6/T7")]:
        if probe not in a_txt:
            errs.append(f"{tag} 探针缺失: {probe}")
    if a_txt.count("（1）不同投标人的投标文件由同一单位或者个人编制的") != 2:
        errs.append("T7 双生子项（1）未在须知/评标办法两处出现")
    # T4 docx：分页符夹在 (8) 两半之间
    texts_a = [b for b in S.A_DOC]
    idx8 = next(i for i, b in enumerate(texts_a)
                if b[0] == "p" and b[1].startswith("（8）投标文件未按"))
    if not (texts_a[idx8 + 1][0] == "pb" and texts_a[idx8 + 2][1].startswith("招标人将拒收其投标文件")):
        errs.append("T4 docx 分页符未夹在第(8)条两半之间")

    # 页数估算区间（Lead：A 20-30 页 / B ~10 页 / C 2 页 / E 1 页）
    if not 20 <= stats["A"]["estimated_pages"] <= 35:
        errs.append(f"A 页数估算越界: {stats['A']['estimated_pages']}")
    if not 6 <= stats["B"]["estimated_pages"] <= 14:
        errs.append(f"B 页数估算越界: {stats['B']['estimated_pages']}")
    if stats["C"]["pages"] != 2:
        errs.append("C 应为 2 页")
    if stats["E"]["pages"] != 1:
        errs.append("E 应为 1 页")

    # 预测总账自洽（v0.2 口径：冻结预演只针对 v0.1 基线集——字母后缀 syn_id
    # 是 v0.2 补登条目，其 engine_expect 为实测登记不进冻结总账）
    import re as _re
    _is_base = lambda e: _re.fullmatch(r"SYN-[A-Z]-\d+", e["syn_id"]) is not None
    occ_base = sum(e["expect_engine_occurrences"] for e in S.EXPECTED_REQUIREMENTS
                   if e["engine_expect"]["will_extract"] and _is_base(e))
    if occ_base != S.EXPECTED_ENGINE_TOTAL:
        errs.append(f"基线集 occurrence 合计 {occ_base} ≠ 预测总数 {S.EXPECTED_ENGINE_TOTAL}（推演账不平）")
    v02 = [e for e in S.EXPECTED_REQUIREMENTS if not _is_base(e)]
    for e in v02:
        if not e["engine_expect"]["will_extract"]:
            errs.append(f"v0.2 补登 {e['syn_id']} 未登记引擎行为（补登面要求 will_extract=true）")
    cat_sum = sum(S.EXPECTED_ENGINE_BY_CATEGORY.values())
    if cat_sum != S.EXPECTED_ENGINE_TOTAL:
        errs.append(f"分类预测合计 {cat_sum} ≠ 预测总数 {S.EXPECTED_ENGINE_TOTAL}")

    return errs


def main():
    out = DEFAULT_OUT
    if "--out" in sys.argv:
        out = Path(sys.argv[sys.argv.index("--out") + 1])
    files_dir = out / "招标文件"
    files_dir.mkdir(parents=True, exist_ok=True)
    stats = {}
    stats["A"] = gen_docx(S.A_DOC, S.A_FILLER_TITLES[:14], S.A_FILLER_LEAD,
                          "{ordinal}、{title}", files_dir, "A-招标文件.docx", (2026, 10, 20, 9, 0, 0))
    stats["B"] = gen_docx(S.B_DOC, S.B_FILLER_TITLES + ["软件回归测试方法", "环境适应性测试方法"], S.B_FILLER_LEAD,
                          "{ordinal}、{title}", files_dir, "B-技术规范书.docx", (2026, 10, 20, 9, 30, 0))
    stats["C"] = gen_c_pdf(files_dir)
    (files_dir / "D-图纸说明.md").write_text(S.D_MD, encoding="utf-8")
    stats["D"] = {"path": "D-图纸说明.md", "bytes": len(S.D_MD.encode("utf-8"))}
    stats["E"] = gen_e_pdf(files_dir)
    stats["answers"] = gen_answers(out)
    gen_readme(out, stats["answers"]["entries"])
    n_files = gen_manifest(out, stats)

    errs = self_check(stats)
    print(json.dumps({
        "out": str(out),
        "A": stats["A"], "B": stats["B"], "C": stats["C"], "E": stats["E"],
        "answers": stats["answers"], "manifest_files": n_files,
        "self_check": "PASS" if not errs else errs,
    }, ensure_ascii=False))
    if errs:
        sys.exit(1)
    print("synth-tender 生成完成（确定性输出）")


if __name__ == "__main__":
    main()
