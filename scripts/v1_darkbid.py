#!/usr/bin/env python3
"""v1_darkbid.py — V1-d 页数与格式口径（verification-plan §3.1 第四段）

暗标纪律五断言（TC-12 扩展口径：渲染页数 + 禁字 + 元数据清零 + 金丝雀防伪绿
+ 标识符面）：
  D1 渲染页数：LO headless 渲染口径页数 ≤ 招标文件页数上限（PROJECT.page_limit），
     且与明标件一致（暗标化不改变分页——格式口径稳定性）
  D2 禁字扫描（双面）：
     - DOCX 全部件 XML（document/headers/footers/docProps）零禁字
     - LO 渲染 PDF 逐页文本零禁字（比 XML 面更强：渲染口径复查）
  D3 元数据清零（docProps）：
     - core.xml dc:creator / cp:lastModifiedBy 置空
     - app.xml Company 移除（G3——引擎侧缺口，本断言为业务侧缓解的实证）
     - LO 产出 PDF 文档元数据 author/creator 为空
  D4 金丝雀（防"扫描器坏掉→空洞全绿"）：明标件必须被检出禁字（公司名/信用代码/
     app.xml Company）——证明扫描器有效，暗标全绿才是真绿
  D5 标识符面（SC-06，issue #18 吸收）：暗标件的**标识符命名空间**（书签名 /
     域代码 / 隐藏文本）零身份泄漏——D2 扫内容，D5 扫标识符：评标人解压暗标件
     即见书签名，CIT- 链名携带投标人或宿主身份即形式性度分风险；金丝雀 =
     明标件副本注入携带身份的书签名后扫描器必须检出（防空洞绿）

禁字表：投标人公司全称 / 统一社会信用代码 / 简称（长中短三级）。
标识符宿主表：生产链/宿主身份 token（工具链名、仓库名）——标识符携带
宿主指纹同属暗标纪律违例（issue #18：宿主模型名前缀泄漏进出站载荷）。
"""
from __future__ import annotations

import json
import re
import sys
import tempfile
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_spec as S  # noqa: E402
import v1_export  # noqa: E402
from ooxml_util import sha256_file  # noqa: E402

REPO = Path(__file__).resolve().parent.parent

# 禁字表（含正负对照说明）——v1_export.FORBIDDEN_WORDS 单一真源
FORBIDDEN = v1_export.FORBIDDEN_WORDS

# 宿主/生产链身份 token（SC-06）：标识符命名空间零容忍面。取词原则：
# ① 生产链真实存在（生成器/仓库/宿主）② ≥4 字符（标识符为结构化短串，
# 过短词假阳性风险高）。业务侧变更需过同构审查（威胁面：解压可见）。
IDENT_HOST_TOKENS = ["python-docx", "bidcore", "ai-bid-research", "cpa-multi-plugins"]


def _docx_parts_text(docx_path: Path) -> dict[str, str]:
    with zipfile.ZipFile(docx_path) as z:
        return {n: z.read(n).decode("utf-8", errors="replace")
                for n in z.namelist() if n.endswith((".xml", ".rels"))}


def scan_forbidden(text: str) -> list[str]:
    return [w for w in FORBIDDEN if w in text]


def _identifier_stream(xml: str) -> tuple[str, int]:
    """标识符命名空间抽取：书签名 + 域指令代码 + 隐藏文本（w:vanish run）。

    与 D2 内容面分离：D2 扫的是「读得出的正文」，D5 扫的是「结构里携带的
    名字」——泄漏路径不同（解压可见、域展开可见、隐藏文本可见）。"""
    names = re.findall(r'w:bookmarkStart[^>]*w:name="([^"]*)"', xml)
    instr = re.findall(r"<w:instrText[^>]*>([^<]*)</w:instrText>", xml)
    hidden: list[str] = []
    for run in re.finditer(
        r"<w:r\b(?:(?!</w:r>).)*?<w:vanish/>(?:(?!</w:r>).)*?</w:r>", xml, re.S
    ):
        hidden += re.findall(r"<w:t[^>]*>([^<]*)</w:t>", run.group(0))
    return " ".join(names + instr + hidden), len(names)


def scan_identifier_leaks(docx_path: Path) -> dict:
    """SC-06 D5：暗标件标识符面扫描——身份词 = FORBIDDEN（投标人）+ 宿主 token。

    pass 判据双条件：零命中 **且** 标识符存在（ident_count > 0）——无书签的
    文件空扫不得真空过绿（空洞绿防护与 D4 同纪律）。"""
    id_words = list(FORBIDDEN) + IDENT_HOST_TOKENS
    hits: dict[str, list[str]] = {}
    ident_count = 0
    for name, xml in _docx_parts_text(docx_path).items():
        stream, n = _identifier_stream(xml)
        ident_count += n
        found = [w for w in id_words if w in stream]
        if found:
            hits[name] = sorted(set(found))
    return {"ident_count": ident_count, "host_tokens": IDENT_HOST_TOKENS,
            "hits": hits, "pass": not hits and ident_count > 0}


def _inject_identifier_leak(src: Path, dst: Path) -> None:
    """D5 金丝雀注入：明标件副本的书签名携带投标人简称——扫描器必须检出。

    注入面 = 纯标识符（bookmarkStart w:name），不碰正文内容（与 D4 内容面
    金丝雀互补：D4 证内容扫描器活，D5 证标识符扫描器活）。"""
    leak_word = FORBIDDEN[-1]  # 简称级（短词——最难检出的那一级）
    with zipfile.ZipFile(src) as zin:
        entries = {n: zin.read(n) for n in zin.namelist()}
    doc = entries["word/document.xml"].decode("utf-8")
    canary = (f'<w:bookmarkStart w:id="98" w:name="CIT-CANARY-{leak_word}"/>'
              f'<w:bookmarkEnd w:id="98"/>')
    assert "<w:sectPr" in doc, "注入点（body 末 sectPr 前）不在位"
    doc = doc.replace("<w:sectPr", canary + "<w:sectPr", 1)
    entries["word/document.xml"] = doc.encode("utf-8")
    with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as zout:
        for n, data in entries.items():
            zout.writestr(n, data)


def run_darkbid_checks(dark_docx: Path, bright_docx: Path, workdir: Path) -> dict:
    # ---- D2 DOCX 部件面禁字
    parts = _docx_parts_text(dark_docx)
    docx_hits = {name: scan_forbidden(xml) for name, xml in parts.items()}
    docx_hits = {k: v for k, v in docx_hits.items() if v}
    d2_docx_ok = not docx_hits

    # ---- D3 docProps 清零
    core = parts.get("docProps/core.xml", "")
    app = parts.get("docProps/app.xml", "")
    d3 = {
        "core_creator_empty": not re.search(r"<dc:creator>[^<]+</dc:creator>", core),
        "core_last_modified_by_empty": not re.search(r"<cp:lastModifiedBy>[^<]+</cp:lastModifiedBy>", core),
        "app_company_removed": "<Company>" not in app,
    }

    # ---- D1/D2 渲染面：LO 转 PDF
    pdf = v1_export.lo_convert_pdf(dark_docx, workdir / "lo_dark")
    import fitz
    d = fitz.open(str(pdf))
    page_texts = [p.get_text() for p in d]
    pdf_hits = {i + 1: scan_forbidden(t) for i, t in enumerate(page_texts)}
    pdf_hits = {k: v for k, v in pdf_hits.items() if v}
    meta = d.metadata or {}
    d1 = {"pages": len(page_texts), "within_limit": len(page_texts) <= S.PROJECT["page_limit"]}
    d2_pdf_ok = not pdf_hits
    # PDF 元数据：author（身份字段）必须为空；creator/producer 为应用字段（Writer/
    # LibreOffice——非投标人身份，实务口径容忍）；另扫元数据值中禁字
    meta_words = scan_forbidden(" ".join(str(v) for v in meta.values()))
    d3["pdf_metadata_author_empty"] = not meta.get("author") and not meta_words

    # ---- D4 金丝雀：明标件必须被检出（扫描器有效性证明）
    bright_parts = _docx_parts_text(bright_docx)
    canary_hits = []
    for name in ("word/document.xml", "docProps/app.xml"):
        hits = scan_forbidden(bright_parts.get(name, ""))
        if hits:
            canary_hits.append({"part": name, "words": hits})
    bright_pdf = v1_export.lo_convert_pdf(bright_docx, workdir / "lo_bright")
    bd = fitz.open(str(bright_pdf))
    for i, p in enumerate(bd):
        hits = scan_forbidden(p.get_text())
        if hits:
            canary_hits.append({"page": i + 1, "words": hits})
    d4_ok = len(canary_hits) >= 2  # XML 面与渲染面均检出

    # ---- D5 标识符面（SC-06）：暗标件标识符零身份泄漏 + 注入式金丝雀
    d5_scan = scan_identifier_leaks(dark_docx)
    # 金丝雀：明标件副本注入携带投标人简称的书签名 → 扫描器必须检出该词
    # （注入前明标件标识符面本身零命中——检出只能来自注入，防空洞绿）
    leak_copy = workdir / "canary_ident_leak.docx"
    _inject_identifier_leak(bright_docx, leak_copy)
    d5_canary = scan_identifier_leaks(leak_copy)
    leak_word = FORBIDDEN[-1]
    d5_canary_hit = any(leak_word in words for words in d5_canary["hits"].values())
    d5 = {"scan": d5_scan, "canary_detected": d5_canary_hit,
          "pass": d5_scan["pass"] and d5_canary_hit}

    # ---- 明暗页数一致（暗标化不改变分页）
    same_pages = len(page_texts) == len(bd)

    return {
        "forbidden_words": FORBIDDEN,
        "D1_pages": {**d1, "same_as_bright": same_pages},
        "D2_forbidden_scan": {"docx_parts_hits": docx_hits, "pdf_page_hits": pdf_hits,
                              "pass": d2_docx_ok and d2_pdf_ok},
        "D3_metadata_zeroing": {**d3, "pass": all(d3.values())},
        "D4_canary": {"hits": canary_hits, "pass": d4_ok},
        "D5_identifier_scan": d5,
        "dark_docx_sha256": sha256_file(dark_docx),
        "pass": (d1["within_limit"] and d2_docx_ok and d2_pdf_ok and all(d3.values())
                 and d4_ok and same_pages and d5["pass"]),
    }


def main() -> int:
    out = Path(sys.argv[sys.argv.index("--out") + 1]) if "--out" in sys.argv else REPO / "artifacts" / "v1"
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        bright = out / "技术标-导出样本.docx"
        dark = out / "技术标-暗标样本.docx"
        v1_export.build_bid_docx(bright, "bright", workdir=td)
        v1_export.build_bid_docx(dark, "dark", workdir=td)
        report = run_darkbid_checks(dark, bright, td)
        (out / "darkbid-report.json").write_text(
            json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print("[V1-d] D1 页数:", report["D1_pages"]["pages"], "≤", S.PROJECT["page_limit"],
              "| 明暗一致:", report["D1_pages"]["same_as_bright"])
        print("[V1-d] D2 禁字扫描（DOCX 部件 + 渲染 PDF）:", report["D2_forbidden_scan"]["pass"])
        print("[V1-d] D3 元数据清零:", report["D3_metadata_zeroing"]["pass"])
        print("[V1-d] D4 金丝雀检出（防空洞全绿）:", report["D4_canary"]["pass"],
              f"（检出 {len(report['D4_canary']['hits'])} 处）")
        d5 = report["D5_identifier_scan"]
        print("[V1-d] D5 标识符面（SC-06）:", d5["pass"],
              f"（标识符 {d5['scan']['ident_count']} 个零命中 + 金丝雀检出 {d5['canary_detected']}）")
        return 0 if report["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
