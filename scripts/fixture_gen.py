#!/usr/bin/env python3
"""fixture_gen.py — corpus-A 可执行评测夹具生成器（V0，verification-plan §2）

消费 fixture_spec.py 生成：
  fixtures/corpus-A/files/          A.docx / B1.pdf / B2.pdf(扫描式) / B3.xlsx / C-补遗.docx / evidence/13件
  fixtures/corpus-A/ground-truth/   requirements.jsonl / evidence-links.jsonl / conflicts.json
  fixtures/corpus-A/expected-transitions/TC-01..12.json
  fixtures/corpus-A/manifest.json   全文件 sha256（夹具自完整性）
  fixtures/queries/exact-25.jsonl + eval-set.jsonl
  fixtures/corpus-R/README.md       真实结构分布测试集占位（D1~D6 后续建）

确定性纪律（判分与回归的基础）：无随机、无时间戳；DOCX/XLSX 冻结 zip 时间戳；
PDF 用 reportlab invariant 模式——同版本库重跑 sha256 逐字节一致（run_tests 校验）。
"""
from __future__ import annotations

import hashlib
import json
import shutil
import sys
import zipfile
from pathlib import Path

import reportlab.rl_config
reportlab.rl_config.invariant = 1  # 确定性 PDF（无时间戳漂移）

from openpyxl import Workbook
from openpyxl.styles import Alignment, Font, PatternFill
from PIL import Image, ImageDraw, ImageFont
from reportlab.lib.pagesizes import A4, landscape
from reportlab.lib import colors
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import mm
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.cidfonts import UnicodeCIDFont
from reportlab.pdfgen import canvas as rl_canvas
from reportlab.platypus import Paragraph, SimpleDocTemplate, Spacer, Table, TableStyle

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_spec as S  # noqa: E402
from ooxml_util import FREEZE_DATE, XLSX_FIXED_CORE, freeze_zip, sha256_file, write_json, write_jsonl  # noqa: E402, F401
import v1_corpus_r  # noqa: E402  corpus-R D1/D2/D5 结构层（V1-a，verification-plan §3.1）

REPO = Path(__file__).resolve().parent.parent
FIXTURES = REPO / "fixtures"

pdfmetrics.registerFont(UnicodeCIDFont("STSong-Light"))
CN_FONT = "STSong-Light"
NOTO_PATH = "/usr/share/fonts/truetype/chinese/SarasaMonoSC-Regular.ttf"
# 字体事实记录（2026-10-24 发现）：NotoSansSC[wght].ttf 可变字体在 PIL 下渲染为全空白
# （FreeType 变量轴实例化失败——0 个暗像素），B2 曾因此生成“只有公章没有文字”的空扫描页；
# reportlab+B1 不受影响（用内置 CID 字体）。B2 改用静态字体 + 像素级自证断言。


# freeze_zip / XLSX_FIXED_CORE / sha256_file / write_json / write_jsonl
# 自 ooxml_util 导入（V1 抽取共享——字节行为不变，T6 哈希回归把关）


# ---------------------------------------------------------------- A / C DOCX
def _build_docx(blocks: list[tuple], doc, filler_pages: int = 0):
    """按块渲染 DOCX；返回 (段落数, 表格数, 显式分页数)。"""
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
    # 填充页（凑足 30+ 页的规范正文——政府采购标准文本常见长度）
    filler_titles = [
        "总体架构要求", "网络系统技术要求", "综合布线系统技术要求", "视频监控系统技术要求",
        "门禁与入侵报警技术要求", "机房工程与 UPS 供电技术要求", "会议系统技术要求",
        "集成管理平台技术要求", "信息发布系统技术要求", "运维与网络安全技术要求",
        "竣工资料与验收规范", "培训与知识转移要求",
    ]
    for i in range(filler_pages):
        title = filler_titles[i % len(filler_titles)]
        doc.add_heading(f"第九章 技术规范与验收要求 9.{i + 1} {title}", level=2)
        doc.add_paragraph(f"9.{i + 1}.1 本节规定{title}的总体技术要求。承包人应确保所供货物与服务符合国家现行标准、"
                          "行业标准及本招标文件技术需求的规定；标准之间不一致时按较高标准执行。")
        doc.add_paragraph(f"9.{i + 1}.2 承包人应在投标文件中对本节要求逐条应答，明确响应程度（满足/部分满足/偏离），"
                          "并对偏离项说明原因及替代方案；未应答的条款按不利于投标人处理。")
        doc.add_paragraph(f"9.{i + 1}.3 本节所有技术参数均以货物到货验收时的实测值为准；实测不达标的，采购人有权"
                          "要求退换货并按合同违约条款处理，由此产生的费用由承包人承担。")
        doc.add_page_break()
        breaks += 1
        paras += 4
    return paras, tables, breaks


def gen_a_docx(out: Path):
    from docx import Document
    doc = Document()
    core = doc.core_properties
    core.author = "ai-bid-research fixture"
    core.title = S.PROJECT["name"]
    # 显式固定元数据时间，避免默认当前时间进入 zip（双保险，freeze_zip 兜底）
    core.created = core.modified = __import__("datetime").datetime(2026, 10, 16, 9, 0, 0)
    paras, tables, breaks = _build_docx(S.A_DOC, doc, filler_pages=22)
    path = out / "A-招标文件.docx"
    doc.save(path)
    freeze_zip(path)
    return {"path": path.name, "paragraph_blocks": paras, "tables": tables, "page_breaks": breaks,
            "estimated_pages": breaks + 1}


def gen_c_docx(out: Path):
    from docx import Document
    doc = Document()
    core = doc.core_properties
    core.author = "ai-bid-research fixture"
    core.created = core.modified = __import__("datetime").datetime(2026, 10, 28, 9, 0, 0)
    paras, tables, breaks = _build_docx(S.C_DOC, doc, filler_pages=0)
    path = out / "C-补遗文件01.docx"
    doc.save(path)
    freeze_zip(path)
    return {"path": path.name, "paragraph_blocks": paras, "tables": tables}


# ---------------------------------------------------------------- B1 大表格 PDF
def gen_b1_pdf(out: Path):
    path = out / "B1-设备清单.pdf"
    styles = getSampleStyleSheet()
    normal = ParagraphStyle("cn", parent=styles["Normal"], fontName=CN_FONT, fontSize=8, leading=11)
    title = ParagraphStyle("cnt", parent=normal, fontSize=14, alignment=1, spaceAfter=6)
    doc = SimpleDocTemplate(str(path), pagesize=landscape(A4),
                            leftMargin=12 * mm, rightMargin=12 * mm,
                            topMargin=12 * mm, bottomMargin=12 * mm)
    story = [Paragraph("附件 B1：设备清单（含型号规格、单价上限与到货日期）", title),
             Paragraph(f"项目编号：{S.PROJECT['code']}　　共 40 项　　到货日期口径：{S.DELIVERY_CN}", normal),
             Spacer(1, 4 * mm)]
    header = [Paragraph(h, normal) for h in S.B1_COLUMNS]
    body = [[Paragraph(c, normal) for c in row] for row in S.B1_ROWS]
    tbl = Table([header] + body, colWidths=[12 * mm, 32 * mm, 40 * mm, 12 * mm, 14 * mm, 26 * mm, 46 * mm, 30 * mm],
                repeatRows=1)
    tbl.setStyle(TableStyle([
        ("GRID", (0, 0), (-1, -1), 0.4, colors.grey),
        ("BACKGROUND", (0, 0), (-1, 0), colors.Color(0.9, 0.9, 0.9)),
        ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
        ("FONTSIZE", (0, 0), (-1, -1), 8),
    ]))
    story.append(tbl)
    story.append(Spacer(1, 4 * mm))
    story.append(Paragraph("注：单价上限为含税价（元）；型号规格栏为响应精确匹配口径，与正文第五章参数表一致。",
                           normal))
    doc.build(story)
    return {"path": path.name, "rows": len(S.B1_ROWS), "cols": len(S.B1_COLUMNS)}


# ---------------------------------------------------------------- B2 扫描式 PDF（无文本层）
def gen_b2_pdf(out: Path):
    """模拟扫描件：PIL 渲染整页图像（含红章）→ 嵌入 PDF，无文本层 → OCR 路径输入。
    页面内容一律取自 fixture_spec（B2_* 常量单一真源——GT-07 扫描件锚点在此页上）。"""
    path = out / "B2-投标函格式页-扫描件.pdf"
    W, H = 1240, 1754  # A4 @150dpi
    img = Image.new("L", (W, H), 248)  # 浅灰底模拟纸张
    d = ImageDraw.Draw(img)
    font_t = ImageFont.truetype(NOTO_PATH, 64)
    font_h = ImageFont.truetype(NOTO_PATH, 36)
    font_b = ImageFont.truetype(NOTO_PATH, 28)

    def wrap(text: str, font, max_w: int, indent: str = "") -> list[str]:
        """按像素宽度断行（逐字符累积——CJK 无空格分词；续行加缩进保持视觉层级）。"""
        lines, cur, cur_w = [], "", 0.0
        indent_w = d.textlength(indent, font=font) if indent else 0.0
        first = True
        for ch in text:
            cw = d.textlength(ch, font=font)
            budget = max_w if first else max_w - indent_w
            if cur and cur_w + cw > budget:
                lines.append(cur)
                cur, cur_w, first = indent + ch, indent_w + cw, False
            else:
                cur += ch
                cur_w += cw
        if cur:
            lines.append(cur)
        return lines

    drawn: list[str] = []  # R2 P0-7：逐行收集实际绘制文本，生成时与规格自证
    source_lines: list[str] = []  # v0.4：wrap 后按源行分组自证（拼接无丢字）
    y = 120
    for seg in wrap(S.B2_TITLE, font_t, W - 80):
        w = d.textlength(seg, font=font_t)
        d.text(((W - w) / 2, y), seg, fill=30, font=font_t); drawn.append(seg); y += 84
    source_lines.append(S.B2_TITLE)
    for seg in wrap(S.B2_SUBTITLE, font_h, W - 80):
        w = d.textlength(seg, font=font_h)
        d.text(((W - w) / 2, y), seg, fill=30, font=font_h); drawn.append(seg); y += 70
    source_lines.append(S.B2_SUBTITLE)
    # GT-07 扫描件锚点条款（图像形式存在——OCR 通道须抽取到）；v0.4：断行防右缘裁字
    for seg in wrap(S.B2_KEY_CLAUSE, font_b, W - 90 - 50):
        d.text((90, y), seg, fill=30, font=font_b); drawn.append(seg); y += 40
    source_lines.append(S.B2_KEY_CLAUSE)
    for text in S.B2_BODY_LINES:
        for seg in wrap(text, font_b, W - 90 - 50, indent="　"):
            d.text((90, y), seg, fill=30, font=font_b); drawn.append(seg); y += 40
        source_lines.append(text)
    y += 40
    for text in S.B2_SIGN_LINES:
        for seg in wrap(text, font_b, W - 110 - 50, indent="　"):
            d.text((110, y), seg, fill=30, font=font_b); drawn.append(seg); y += 44
        y += 36
        source_lines.append(text)
    # 自证：wrap 分段拼接 == 源行（无丢字无错序）——防“断行丢尾字”回归
    i = 0
    for src in source_lines:
        segs: list[str] = []
        while i < len(drawn):
            segs.append(drawn[i]); i += 1
            joined = "".join(segs)
            if src.startswith(joined) and len(joined) >= len(src):
                break
            if not src.startswith(joined):
                raise AssertionError(f"B2 断行漂移：{segs!r} 不构成源行前缀（源行：{src[:30]}…）")
        assert "".join(segs) == src, f"B2 断行拼接丢字：{''.join(segs)!r} != {src[:30]}…"
    assert all(line in S.B2_PAGE_TEXT for line in source_lines), "B2 绘制行未全部落在 B2_PAGE_TEXT 规格内"
    # 红色公章（椭圆占位，扫描件特征）
    d.ellipse((830, 1040, 1150, 1360), outline=120, width=6)
    for i in range(5):
        d.arc((900 + i, 1080 + i, 1080 - i, 1180 - i), start=200, end=340, fill=110, width=3)
    d.text((930, 1220), "（示例公章）", fill=110, font=font_b)

    # 像素级自证（v0.4 修复）：文字必须真实渲染在像素层——防可变字体静默空白再次溜过。
    # 列表断言只能证明“调过 d.text”，证明不了“画出了字”。
    import numpy as _np
    _a = _np.array(img)
    assert int(_a.min()) <= 60, f"B2 像素层无文字（min={_a.min()}）——字体渲染失败"
    assert (_a < 150).sum() >= 20000, f"B2 暗像素过少（{(_a < 150).sum()}<20000）——文字大面积缺失"
    _rows = (_a < 150).sum(axis=1)
    _text_rows = int((_rows[:1000] > 20).sum())  # y<1000 为文字区（公章在 1040+）
    assert _text_rows >= 20, f"B2 文字区行数不足（{_text_rows}<20）"

    c = rl_canvas.Canvas(str(path), pagesize=A4)
    c.setTitle("B2 投标函格式页（扫描件）")
    import io
    from reportlab.lib.utils import ImageReader
    buf = io.BytesIO()
    img.save(buf, format="PNG", optimize=True)
    buf.seek(0)
    # 关键：传 ImageReader（内容寻址命名）而非文件路径（路径寻址 → 不可复现）
    c.drawImage(ImageReader(buf), 0, 0, width=A4[0], height=A4[1])
    c.showPage()
    c.save()
    return {"path": path.name, "text_layer": False, "image_page": True,
            "key_clause": S.B2_KEY_CLAUSE, "drawn_selfcheck": "passed",
            "key_strings": ["投标函", "投标总报价", "投标人", "盖单位章", "日期"]}


# ---------------------------------------------------------------- B3 图纸目录 XLSX
def gen_b3_xlsx(out: Path):
    import datetime
    path = out / "B3-图纸目录.xlsx"
    wb = Workbook()
    wb.properties.created = wb.properties.modified = datetime.datetime(2026, 10, 16, 9, 0, 0)
    ws = wb.active
    ws.title = "图纸目录"
    ws["A1"] = "附件 B3：图纸目录"
    ws["A1"].font = Font(bold=True, size=13)
    ws.merge_cells("A1:F1")
    ws["A2"] = f"项目编号：{S.PROJECT['code']}"
    ws.append([])
    ws.append(S.B3_COLUMNS)
    header_row = ws.max_row
    for row in S.B3_ROWS:
        ws.append(row)
    for cell in ws[header_row]:
        cell.font = Font(bold=True)
        cell.fill = PatternFill("solid", fgColor="DDDDDD")
        cell.alignment = Alignment(horizontal="center", vertical="center")
    for col, width in zip("ABCDEF", [8, 14, 30, 8, 8, 8]):
        ws.column_dimensions[col].width = width
    wb.save(path)
    freeze_zip(path, patches={"docProps/core.xml": XLSX_FIXED_CORE})
    return {"path": path.name, "rows": len(S.B3_ROWS), "cols": len(S.B3_COLUMNS)}


# ---------------------------------------------------------------- 企业资料库（13 件文本 PDF）
def gen_evidence(out: Path):
    ev_dir = out / "evidence"
    ev_dir.mkdir(parents=True, exist_ok=True)
    made = []
    for fname, cat, lines, valid_until, gts in S.EVIDENCE_FILES:
        path = ev_dir / fname
        c = rl_canvas.Canvas(str(path), pagesize=A4)
        c.setTitle(fname)
        y = 800
        c.setFont(CN_FONT, 16)
        c.drawCentredString(297.5, y, cat)
        y -= 40
        c.setFont(CN_FONT, 11)
        c.drawString(72, y, "（合成夹具证据文件——内容为虚构示例，仅供测试管线使用）")
        y -= 36
        c.setFont(CN_FONT, 13)
        for line in lines:
            c.drawString(72, y, line)
            y -= 26
        if valid_until:
            y -= 10
            c.setFont(CN_FONT, 11)
            c.setFillColorRGB(0.55, 0.1, 0.1)
            c.drawString(72, y, f"【有效期至 {valid_until}】")
            c.setFillColorRGB(0, 0, 0)
        c.showPage()
        c.save()
        made.append({"path": f"evidence/{fname}", "category": cat, "valid_until": valid_until})
    return made


# ---------------------------------------------------------------- TC-05 缺陷样本草稿（validate 工具的植入输入）
def gen_defect_drafts(corpus: Path):
    """植入 8 类确定性缺陷的草稿 + 缺陷→规则映射——TC-05 的可执行输入。
    注意：草稿中不标注任何 GT/rule 标记（防标记回声），映射关系只在 defect-map.json。"""
    dd = corpus / "defect-drafts"
    dd.mkdir(parents=True, exist_ok=True)
    from docx import Document
    doc = Document()
    core = doc.core_properties
    core.author = "ai-bid-research fixture"
    core.created = core.modified = __import__("datetime").datetime(2026, 11, 8, 9, 0, 0)
    doc.add_heading("技术标（草稿·缺陷样本）", level=1)
    doc.add_paragraph("（合成夹具：本草稿植入 8 类确定性审校缺陷，用于 validate_bid_document 的 8/8 命中断言）")
    for defect in S.DEFECTS:
        doc.add_paragraph(defect["excerpt"])
    path = dd / S.DEFECT_DRAFT_NAME
    doc.save(path)
    freeze_zip(path)
    write_json(dd / "defect-map.json", {"draft": S.DEFECT_DRAFT_NAME, "defects": S.DEFECTS})
    return {"draft": S.DEFECT_DRAFT_NAME, "defects": len(S.DEFECTS)}


# ---------------------------------------------------------------- ground-truth / transitions / queries
def gen_ground_truth(corpus: Path):
    gt = corpus / "ground-truth"
    gt.mkdir(parents=True, exist_ok=True)
    write_jsonl(gt / "requirements.jsonl", S.REQUIREMENTS)
    write_jsonl(gt / "evidence-links.jsonl", S.EVIDENCE_LINKS)
    write_jsonl(gt / "sections.jsonl", S.SECTIONS_SPEC)
    write_json(gt / "conflicts.json", S.CONFLICTS)
    return {"requirements": len(S.REQUIREMENTS), "evidence_links": len(S.EVIDENCE_LINKS),
            "sections": len(S.SECTIONS_SPEC), "conflicts": len(S.CONFLICTS)}


def gen_transitions(corpus: Path):
    et = corpus / "expected-transitions"
    et.mkdir(parents=True, exist_ok=True)
    for tc in S.TRANSITIONS:
        write_json(et / f"{tc['tc_id']}.json", tc)
    return {"files": len(S.TRANSITIONS)}


def gen_queries(queries_dir: Path):
    queries_dir.mkdir(parents=True, exist_ok=True)
    write_jsonl(queries_dir / "exact-25.jsonl", S.QUERIES)
    write_jsonl(queries_dir / "eval-set.jsonl", S.EVAL_SET)
    return {"exact_queries": len(S.QUERIES), "eval_entries": len(S.EVAL_SET)}


# ---------------------------------------------------------------- manifest
def gen_engine_borrowed(out: Path) -> dict:
    """TR2 借鉴夹具面（PR #15 引入）：从仓内 fixtures/engine-borrowed/TR2/ 复制到输出根。

    这些是 mdcore 引擎仓借鉴二进制（word-complex / word-merged / excel-complex +
    gen-log + PROVENANCE 溯源），非生成器产物——确定性语义 = 字节级复制（sha256 恒定）。
    T6 重生成比对依赖本画（复制到 --out 临时目录后由 gen_manifest 收录，与仓内
    manifest 双向一致）；T3a 反向校验由此消除「游离文件未入清单」。
    """
    src_root = REPO / "fixtures" / "engine-borrowed"
    if not src_root.exists():
        return {"files": 0}
    copied = 0
    for p in sorted(src_root.rglob("*")):
        if not p.is_file():
            continue
        dst = out / "engine-borrowed" / p.relative_to(src_root)
        if dst.resolve() == p.resolve():
            copied += 1  # 默认输出 = 仓内 fixtures 本身，原地即目标
            continue
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(p, dst)
        copied += 1
    return {"files": copied}


def gen_manifest(fixtures_root: Path, stats: dict):
    """覆盖整个 fixtures 根（corpus-A + queries + corpus-R），路径相对 fixtures/。"""
    files = []
    for p in sorted(fixtures_root.rglob("*")):
        if p.is_file() and p.name != "manifest.json":
            files.append({"path": str(p.relative_to(fixtures_root)), "sha256": sha256_file(p),
                          "bytes": p.stat().st_size})
    manifest = {
        "fixture": "corpus-A (+ queries/corpus-R)",
        "spec_version": "v0.6",
        "spec_source": "scripts/fixture_spec.py + research/test-plan.md §1（v0.6：corpus-R +D6-revisions 修订/批注专项层——引擎 mdcore#17/#19 验收夹具，T2 批 issue #2；此前 v0.5：v0.2 审计修正 + v0.4 B2 扫描件修复 + corpus-R D1/D2/D5 结构层）",
        "deterministic": True,
        "stats_note": "A.paragraph_blocks 为生成器块计数（非 docx 物理段落数）；B1.rows 为数据行数（物理跨 2 页）",
        "counts": {
            "requirements": len(S.REQUIREMENTS),
            "categories": {c: sum(1 for r in S.REQUIREMENTS if r["category"] == c)
                           for c in ["资格条件", "否决项", "评分项", "技术参数", "商务要求", "格式要求"]},
            "evidence_files": len(S.EVIDENCE_FILES),
            "conflicts": len(S.CONFLICTS),
            "transitions": len(S.TRANSITIONS),
            "exact_queries": len(S.QUERIES),
            "sections": len(S.SECTIONS_SPEC),
            "defects": len(S.DEFECTS),
        },
        "key_facts": {
            "open_bid_date": S.PROJECT["open_bid_date"],
            "open_bid_date_after_addendum": S.PROJECT["open_bid_date_after_addendum"],
            "control_price_a": S.PROJECT["control_price_a"],
            "control_price_c": S.PROJECT["control_price_c"],
            "b1_total_cap": S.B1_TOTAL_CAP,
            "b1_total_cap_note": "Σ(单价上限×数量) ≤ 设备预算 350 万（防隐性冲突）",
            "gt02_flip": "EV-02 valid_until 2026-11-25：eligible@11-15 → not_eligible@11-27",
        },
        "stats": stats,
        "files": files,
    }
    write_json(fixtures_root / "corpus-A" / "manifest.json", manifest)
    return manifest


CORPUS_R_README_PLACEHOLDER = """# corpus-R：真实结构分布测试集

v0.1（V1-a 首批）：D1-tables / D2-nonbody / D5-scanned 三结构层已建（确定性合成——
复刻真实件的**结构特征**而非内容；去标识化纪律自动满足：无任何真实主体信息）。
D3-revisions / D4-wording / D6-scale 三层待真实样本去标识化后补建（见 README 由
v1_corpus_r 生成时的展开说明）。完整分层定义：research/test-plan.md §5.1。
"""


def main():
    out = FIXTURES
    if "--out" in sys.argv:
        out = Path(sys.argv[sys.argv.index("--out") + 1])
    corpus = out / "corpus-A"
    files_dir = corpus / "files"
    files_dir.mkdir(parents=True, exist_ok=True)
    stats = {}
    stats["A"] = gen_a_docx(files_dir)
    stats["C"] = gen_c_docx(files_dir)
    stats["B1"] = gen_b1_pdf(files_dir)
    stats["B2"] = gen_b2_pdf(files_dir)
    stats["B3"] = gen_b3_xlsx(files_dir)
    stats["evidence"] = gen_evidence(files_dir)
    stats["ground_truth"] = gen_ground_truth(corpus)
    stats["defect_drafts"] = gen_defect_drafts(corpus)
    stats["transitions"] = gen_transitions(corpus)
    stats["queries"] = gen_queries(out / "queries")
    stats["corpus_r"] = v1_corpus_r.gen_corpus_r(out / "corpus-R")
    stats["engine_borrowed"] = gen_engine_borrowed(out)
    manifest = gen_manifest(out, stats)

    print(json.dumps({
        "out": str(out),
        "A_pages_est": stats["A"]["estimated_pages"],
        "B1": [stats["B1"]["rows"], stats["B1"]["cols"], "total_cap", S.B1_TOTAL_CAP],
        "B2_text_layer": stats["B2"]["text_layer"],
        "evidence_files": len(stats["evidence"]),
        "defects": stats["defect_drafts"]["defects"],
        "requirements": manifest["counts"]["requirements"],
        "manifest_files": len(manifest["files"]),
    }, ensure_ascii=False, indent=2))
    print("corpus-A 生成完成（确定性输出）")


if __name__ == "__main__":
    main()
