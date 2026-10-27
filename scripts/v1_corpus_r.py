#!/usr/bin/env python3
"""v1_corpus_r.py — corpus-R 结构分布测试集生成器（V1-a 首批：D1/D2/D5 三层）

verification-plan §3.1 V1-a 要求 corpus-R D1-tables / D2-nonbody / D5-scanned 分层
跑解析面。真实件去标识化样本在 V0 阶段不可得（D3/D4/D6 仍待真实样本），本生成器
以**确定性合成**复刻三层的结构特征（合并表头/跨页表/公式区、页眉页脚/文本框/批注/
修订、无文本层/双栏/盖章遮挡/图文混排）——结构压力即测试目的，内容全为示例虚构。

纪律：
- 反标记回声（同 corpus-A）：语料正文零 GT/规则标记；
- 确定性：无随机、无时间戳；zip 冻结（ooxml_util.freeze_zip）；PDF invariant；
  PIL→PNG 内容寻址（ImageReader，不走文件路径）；
- 文件全部经 fixture_gen.main() 生成 → 自动入 manifest（run_tests T3a 双向校验）。
"""
from __future__ import annotations

import io
import sys
import zipfile
from pathlib import Path

import reportlab.rl_config
reportlab.rl_config.invariant = 1  # 确定性 PDF（须在 canvas 前设置）

from docx import Document
from docx.enum.section import WD_SECTION
from docx.enum.table import WD_TABLE_ALIGNMENT
from docx.oxml.ns import qn
from openpyxl import Workbook
from openpyxl.styles import Alignment, Font, PatternFill
from PIL import Image, ImageDraw, ImageFont
from reportlab.lib.pagesizes import A4
from reportlab.lib.units import mm
from reportlab.pdfgen import canvas as rl_canvas

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ooxml_util import XLSX_FIXED_CORE, freeze_zip, freeze_docx, write_json  # noqa: E402

NOTO_PATH = "/usr/share/fonts/truetype/chinese/SarasaMonoSC-Regular.ttf"
# 字体事实（v0.4 B2 修复同步）：NotoSansSC[wght].ttf 可变字体在 PIL 下渲染全空白
# （FreeType 变量轴实例化失败，0 暗像素）——D5 扫描层图像与导出件插图一律用静态
# 字体，且生成时做像素级自证断言（防“空扫描页”静默溜过）。


def _assert_pixels_text(img, min_dark: int, label: str):
    """像素自证：文字必须真实渲染在像素层（防可变字体静默空白）。"""
    import numpy as np
    a = np.asarray(img.convert("L"))
    dark = int((a < 150).sum())
    assert dark >= min_dark, f"{label} 暗像素过少（{dark}<{min_dark}）——文字未渲染（可变字体空白回归？）"
    return dark

CORPUS_R_VERSION = "v0.2（T2 批：+D6-revisions 修订/批注专项层——引擎 #17/#18/#19 验收夹具）"

# 产出文件清单（run_tests 期望文件数单一真源；gen_corpus_r 断言保持同步）
OUTPUT_FILES = (
    "D1-tables/D1a-合并表头跨页表.docx",
    "D1-tables/D1b-宽表公式.xlsx",
    "D2-nonbody/D2a-非正文条款.docx",
    "D5-scanned/D5a-全扫描双栏盖章.pdf",
    "D5-scanned/D5b-图文混排.pdf",
    "D6-revisions/R6a-纯插入澄清.docx",
    "D6-revisions/R6b-纯删除澄清.docx",
    "D6-revisions/R6c-混合修订澄清.docx",
    "D6-revisions/R6d-带批注澄清.docx",
    "D6-revisions/R6e-修订批注组合澄清.docx",
    "D6-revisions/expected-readback.json",
)

README = """# corpus-R：真实结构分布测试集

> v0.1（V1-a 首批）：D1-tables / D2-nonbody / D5-scanned 三结构层已建。
> v0.2（T2 批，issue #2 评论 2026-10-27 03:28 派发）：+D6-revisions 修订/批注专项层
>（5 变体 + 期望读回契约）——引擎 mdcore#17/#18/#19 验收夹具 + 标书侧回归基线。
> 生成器：`scripts/v1_corpus_r.py`（确定性合成——复刻真实件的**结构特征**而非内容）。

## 已建分层

| 层 | 文件 | 结构特征 | 解析面验证口径 |
|---|---|---|---|
| D1-tables | D1a-合并表头跨页表.docx | 两行嵌套表头（横向合并×2 + 纵向跨行合并）、18 数据行长表（渲染自然跨页）、正文内合并单元格 | 表格单元格定位（DocxTableCell 行列编址在合并下稳定） |
| D1-tables | D1b-宽表公式.xlsx | 合并标题区、公式列（F=D×E）、合计 SUM、数据区纵向合并 | 公式双通道指纹（值变/公式变各自失效）+ 区域指纹行优先序 |
| D2-nonbody | D2a-非正文条款.docx | 页眉页脚载明限制条款、首页不同页眉、文本框条款（VML）、批注（comments part）、修订（w:ins/w:del） | 页眉页脚可读可定位；文本框/批注/修订**盘点但无定位器**（引擎缺口 G6，业务侧 M1 同边界——诚实上报不伪装支持） |
| D5-scanned | D5a-全扫描双栏盖章.pdf | 全页无文本层（图像字）、双栏排版、半透明印章遮挡 | 文本层检测=none → OCR 路径（M1 解析面外，登记不伪装） |
| D5-scanned | D5b-图文混排.pdf | 第 1 页文本层 + 第 2 页纯图像 | 文本层检测=partial（逐页判定，非全有全无） |
| D6-revisions | R6a-纯插入澄清.docx | 纯 w:ins 插入（2 处、单作者）——澄清补录形态 | ins 当前文本必须进入读出面（ECMA-376 §17.13.5；mdcore#17 验收） |
| D6-revisions | R6b-纯删除澄清.docx | 纯 w:del 删除（2 处 delText、整段/句中两形态）——废止条款形态 | delText 历史文本不进正文、入删除登记面（§17.13.4；#17 验收） |
| D6-revisions | R6c-混合修订澄清.docx | 同段替换（del+ins 相邻）+ 表格内修订 + 三作者异日期 | 替换/表格修订双面；作者/日期归属可读（#17 验收） |
| D6-revisions | R6d-带批注澄清.docx | 双批注（2 作者、commentRange 锚定）无修订 | comments 面可读可数（#19 验收）；正文文本不受批注污染 |
| D6-revisions | R6e-修订批注组合澄清.docx | del+ins+批注锚定组合——澄清实务形态 | 修订+批注双面组合（#17+#19 验收） |

D6-revisions/expected-readback.json：逐变体**期望读回契约**（按 OOXML 规范：
ins=当前生效文本、del 内 delText=历史文本；含作者/日期/编号归属）——引擎 #17 验收
测试与标书侧回归判分共用单一真源。

## 待建分层（需真实样本去标识化）

- D3-revisions/ 多版本修订链：补遗/答疑/澄清多次修改、相互矛盾、部分撤销
  （**文档级版本链**——与 D6-revisions 的 w:ins/w:del ** tracked changes 结构层**互补，勿混淆）
- D4-wording/ 表述漂移：同一要求不同表达、无固定编号、口语化、"参照执行"类引用
- D6-scale/ 超长超大：500 页级正文、附件图纸包、Excel 量价清单

来源纪律（真实样本入库时）：仅公开招标公告/文件去标识化构造——去除投标人/评审人
信息，保留版式与条款结构；入库登记来源 URL 与抓取日期。合成件（本 v0.1 批）无
任何真实主体信息，去标识化纪律自动满足。

完整分层定义：research/test-plan.md §5.1；验证执行：scripts/v1_pipeline.py（V1-a）。
"""

# ---------------------------------------------------------------- D1a DOCX 合并表头跨页表
_D1A_BODY = [
    # (类别[纵向合并起点], 名称, 型号规格, 单位, 数量, 单价上限, 质保期, 备注)
    ("网络设备", "核心交换机", "示例型号 CE-9500-48C（示例）", "台", 2, 185_000, "3 年", "含模块"),
    (None,      "接入交换机", "示例型号 CE-6800-48T（示例）", "台", 12, 23_500, "3 年", ""),
    (None,      "无线控制器", "示例型号 WLC-3504（示例）", "台", 2, 78_000, "3 年", ""),
    ("安全设备", "下一代防火墙", "示例型号 FG-3100D（示例）", "台", 2, 156_000, "3 年", "含特征库"),
    (None,      "上网行为管理", "示例型号 AC-6000（示例）", "台", 1, 45_000, "3 年", ""),
    ("计算存储", "应用服务器", "示例型号 SR-660（示例）", "台", 4, 92_000, "3 年", ""),
    (None,      "数据库服务器", "示例型号 SR-860（示例）", "台", 2, 148_000, "3 年", ""),
    (None,      "分布式存储", "示例型号 ST-5000（示例）", "套", 1, 320_000, "3 年", ""),
    ("机房配套", "精密空调", "示例型号 CRV-40（示例）", "台", 2, 68_000, "2 年", ""),
    (None,      "UPS 电源", "示例型号 UH-60kVA（示例）", "套", 1, 210_000, "2 年", ""),
    (None,      "机柜及配件", "示例型号 NR-42U（示例）", "个", 14, 4_800, "2 年", ""),
    ("智能化专项", "视频监控", "示例型号 IPC-8X（示例）", "台", 48, 3_200, "2 年", ""),
    (None,      "门禁系统", "示例型号 AC-2100（示例）", "套", 16, 5_600, "2 年", ""),
    (None,      "综合布线", "六类非屏蔽（示例）", "项", 1, 185_000, "2 年", ""),
    (None,      "排队叫号", "示例型号 QM-300（示例）", "套", 8, 12_000, "2 年", ""),
    ("运维服务", "驻场运维", "3 年 7×24（示例）", "项", 1, 260_000, "—", "含备件"),
    (None,      "培训服务", "分批集中培训（示例）", "项", 1, 36_000, "—", ""),
    (None,      "迁移实施", "数据迁移与割接（示例）", "项", 1, 55_000, "—", ""),
]


def gen_d1a(out: Path) -> dict:
    path = out / "D1a-合并表头跨页表.docx"
    doc = Document()
    doc.add_heading("附件一：设备与技术服务量价表（示例——合并表头结构测试件）", level=1)
    doc.add_paragraph("说明：本表为两行嵌套表头（横向合并 + 纵向跨行合并）结构示例；"
                      "表体 18 行，渲染后自然跨页；单价上限合计不超过示例预算。")
    # 9 列：序号 | 设备信息(名称/型号规格/单位) | 商务信息(数量/单价上限/质保期) | 类别 | 备注
    t = doc.add_table(rows=2 + len(_D1A_BODY), cols=9)
    t.style = "Table Grid"
    t.alignment = WD_TABLE_ALIGNMENT.CENTER
    t.cell(0, 0).merge(t.cell(1, 0))          # 序号 纵向合并
    t.cell(0, 1).merge(t.cell(0, 2)).merge(t.cell(0, 3))   # 设备信息 横向合并
    t.cell(0, 4).merge(t.cell(0, 5)).merge(t.cell(0, 6))   # 商务信息 横向合并
    t.cell(0, 7).merge(t.cell(1, 7))          # 类别 纵向合并
    t.cell(0, 8).merge(t.cell(1, 8))          # 备注 纵向合并
    t.cell(0, 0).text = "序号"
    t.cell(0, 1).text = "设备信息"
    t.cell(0, 4).text = "商务信息"
    t.cell(0, 7).text = "类别"
    t.cell(0, 8).text = "备注"
    for i, name in enumerate(["名称", "型号规格", "单位"]):
        t.cell(1, 1 + i).text = name
    for i, name in enumerate(["数量", "单价上限（元）", "质保期"]):
        t.cell(1, 4 + i).text = name
    # 表体：类别列纵向合并（连续同类行合并——合并后仅首行保留类别名）
    for r, (cat, name, spec, unit, qty, price, warranty, note) in enumerate(_D1A_BODY):
        row = 2 + r
        t.cell(row, 0).text = str(r + 1)
        t.cell(row, 1).text = name
        t.cell(row, 2).text = spec
        t.cell(row, 3).text = unit
        t.cell(row, 4).text = str(qty)
        t.cell(row, 5).text = f"{price:,}"
        t.cell(row, 6).text = warranty
        if cat:
            t.cell(row, 7).text = cat          # 分组首行写类别名，随后纵向合并
        t.cell(row, 8).text = note
    # 类别列合并：网络设备(0-2) 计算存储(5-7) 机房配套(8-10) 智能化专项(11-14) 运维服务(15-17)
    for start, end in [(0, 2), (5, 7), (8, 10), (11, 14), (15, 17)]:
        t.cell(2 + start, 7).merge(t.cell(2 + end, 7))
    doc.add_paragraph("注：本表结构为合成示例，不构成任何真实采购需求。")
    doc.save(str(path))
    freeze_docx(path)
    return {"path": path.name, "cols": 9, "body_rows": len(_D1A_BODY),
            "header": {"rows": 2, "horizontal_merges": 2, "vertical_merges": 2},
            "body_vertical_merges": [("类别列", 5)]}


# ---------------------------------------------------------------- D1b XLSX 宽表公式
_D1B_ROWS = [
    ("布线材料", "六类网线（示例）", "箱", 120, 620),
    (None, "光缆（示例）", "米", 2000, 18),
    (None, "配线架（示例）", "个", 24, 480),
    ("网络模块", "接入交换机（示例）", "台", 12, 23500),
    (None, "核心交换机（示例）", "台", 2, 185000),
    ("安全组件", "防火墙（示例）", "台", 2, 156000),
    (None, "行为管理（示例）", "台", 1, 45000),
    ("机房材料", "UPS 电池（示例）", "节", 80, 1250),
    (None, "空调冷媒（示例）", "kg", 60, 260),
    ("施工辅材", "桥架（示例）", "米", 850, 42),
    (None, "线管（示例）", "米", 1200, 12),
    (None, "辅材杂项（示例）", "项", 1, 38000),
]


def gen_d1b(out: Path) -> dict:
    import datetime
    path = out / "D1b-宽表公式.xlsx"
    wb = Workbook()
    wb.properties.created = wb.properties.modified = datetime.datetime(2026, 10, 16, 9, 0, 0)
    ws = wb.active
    ws.title = "量价汇总"
    ws["A1"] = "量价汇总表（示例——公式与合并区结构测试件）"
    ws["A1"].font = Font(bold=True, size=13)
    ws.merge_cells("A1:F1")
    headers = ["序号", "分类", "项目", "单位", "数量", "合价（元）"]
    ws.append(headers)
    for cell in ws[2]:
        cell.font = Font(bold=True)
        cell.fill = PatternFill("solid", fgColor="DDDDDD")
        cell.alignment = Alignment(horizontal="center", vertical="center")
    for i, (cat, item, unit, qty, unit_price) in enumerate(_D1B_ROWS):
        r = 3 + i
        ws.cell(r, 1, i + 1)
        ws.cell(r, 2, cat if cat else None)
        ws.cell(r, 3, item)
        ws.cell(r, 4, unit)
        ws.cell(r, 5, qty)
        ws.cell(r, 6, f"=E{r}*{unit_price}")   # 公式通道：合价 = 数量 × 单价
    # 分类列纵向合并（连续同类行：3-5 布线 / 6-7 网络 / 8-9 安全 / 10-11 机房 / 12-14 施工）
    for start, end in [(3, 5), (6, 7), (8, 9), (10, 11), (12, 14)]:
        ws.merge_cells(f"B{start}:B{end}")
    last = 2 + len(_D1B_ROWS)
    ws.cell(last + 1, 5, "合计")
    ws.cell(last + 1, 5).font = Font(bold=True)
    ws.cell(last + 1, 6, f"=SUM(F3:F{last})")
    ws.cell(last + 1, 6).font = Font(bold=True)
    for col, width in zip("ABCDEF", [6, 12, 22, 8, 8, 14]):
        ws.column_dimensions[col].width = width
    wb.save(str(path))
    freeze_zip(path, patches={"docProps/core.xml": XLSX_FIXED_CORE})
    formulas = [f"F{3 + i}" for i in range(len(_D1B_ROWS))] + [f"F{last + 1}"]
    return {"path": path.name, "rows": len(_D1B_ROWS), "formulas": formulas,
            "merged": ["A1:F1"]}


# ---------------------------------------------------------------- D2a DOCX 非正文条款
_D2A_HEADER_CLAUSE = "示例项目澄清文件（第 1 次）｜页眉载明：本页眉所示内容与正文具有同等效力"
_D2A_FOOTER_CLAUSE = "受控文件编号：CL-2026-SY-001（页脚载明：受控编号条款与正文具有同等效力）"
_D2A_TEXTBOX_CLAUSE = "文本框条款：本文件所附技术参数表以最新澄清版本为准（本条款载于文本框内，非正文段落）"
_D2A_COMMENT = "评审查疑点：本条与后续补遗可能存在口径差异（示例批注——批注不属正文）"
_D2A_INS_TEXT = "（修订插入：响应截止时间顺延至 2026-12-01 09:30）"
_D2A_DEL_TEXT = "原表述：响应截止时间 2026-11-20 09:30"


def _add_textbox_paragraph(doc, text: str):
    """VML 文本框段落（python-docx 正文段落遍历不可见其文本——非正文结构特征）。"""
    from docx.oxml import parse_xml
    ns = ('xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" '
          'xmlns:v="urn:schemas-microsoft-com:vml" '
          'xmlns:o="urn:schemas-microsoft-com:office:office"')
    xml = (
        f'<w:p {ns}><w:r><w:pict>'
        f'<v:shape id="tbClause1" type="#_x0000_t202" style="width:340pt;height:56pt;">'
        f'<v:textbox><w:txbxContent><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:txbxContent></v:textbox>'
        f'</v:shape></w:pict></w:r></w:p>'
    )
    doc.element.body.append(parse_xml(xml))


def _add_revision_paragraph(doc, base_text: str, ins_text: str, del_text: str):
    """修订段：w:ins 插入 + w:del 删除（python-docx p.text 对两者均不可见——修订特征）。"""
    from docx.oxml import parse_xml
    ns = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
    author, date = "示例作者", "2026-10-20T09:00:00Z"
    xml = (
        f'<w:p {ns}>'
        f'<w:r><w:t xml:space="preserve">{base_text}</w:t></w:r>'
        f'<w:ins w:id="901" w:author="{author}" w:date="{date}">'
        f'<w:r><w:t xml:space="preserve">{ins_text}</w:t></w:r></w:ins>'
        f'<w:del w:id="902" w:author="{author}" w:date="{date}">'
        f'<w:r><w:delText xml:space="preserve">{del_text}</w:delText></w:r></w:del>'
        f'</w:p>'
    )
    doc.element.body.append(parse_xml(xml))


def _add_comment(doc, paragraph, text: str):
    """comments part 注入（python-docx 1.2 无公开批注 API——按 OOXML 规约手写）。"""
    from docx.oxml import parse_xml
    W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
    # 1) 在目标段落首 run 前插 commentRangeStart，末插 commentRangeEnd + commentReference
    first_run = paragraph.runs[0]._r
    rng_start = parse_xml(f'<w:commentRangeStart xmlns:w="{W}" w:id="701"/>')
    rng_end = parse_xml(f'<w:commentRangeEnd xmlns:w="{W}" w:id="701"/>')
    ref = parse_xml(f'<w:r xmlns:w="{W}"><w:rPr><w:rStyle w:val="a5"/></w:rPr>'
                    f'<w:commentReference w:id="701"/></w:r>')
    first_run.addprevious(rng_start)
    paragraph._p.append(rng_end)
    paragraph._p.append(ref)


def gen_d2a(out: Path) -> dict:
    path = out / "D2a-非正文条款.docx"
    doc = Document()
    doc.add_heading("示例项目澄清文件（第 1 次）", level=1)
    p1 = doc.add_paragraph("澄清一：机房建设标准按现行国家标准执行，联合接地电阻不大于 1 欧姆；"
                           "本条与招标文件第三章相应条款具有同等效力。")
    doc.add_paragraph("澄清二：投标文件递交地点调整为示例大厦三层开标室（示例地址）；"
                      "递交截止时间以本文件页眉及修订标记载明的口径为准。")
    doc.add_paragraph("澄清三：评分细则中『类似的类似业绩』认定年限，以合同签订时间在近三年内为准。")
    _add_textbox_paragraph(doc, _D2A_TEXTBOX_CLAUSE)
    _add_revision_paragraph(doc, "澄清四：响应截止时间口径如下——", _D2A_INS_TEXT, _D2A_DEL_TEXT)
    # 页眉页脚（默认 + 首页不同）
    sec = doc.sections[0]
    sec.different_first_page_header_footer = True
    sec.header.paragraphs[0].text = _D2A_HEADER_CLAUSE
    sec.footer.paragraphs[0].text = _D2A_FOOTER_CLAUSE
    first_hdr = sec.first_page_header
    first_hdr.paragraphs[0].text = "示例项目澄清文件（第 1 次）——首页（首页页眉与默认页眉不同）"
    _add_comment(doc, p1, _D2A_COMMENT)
    doc.save(str(path))
    # comments part zip 层注入
    W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
    comments_xml = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        f'<w:comments xmlns:w="{W}">'
        f'<w:comment w:id="701" w:author="示例作者" w:date="2026-10-20T09:00:00Z" w:initials="SY">'
        f'<w:p><w:r><w:t>{_D2A_COMMENT}</w:t></w:r></w:p></w:comment></w:comments>'
    ).encode("utf-8")
    with zipfile.ZipFile(path) as zin:
        items = {i.filename: zin.read(i.filename) for i in zin.infolist()}
        types = items["[Content_Types].xml"].decode("utf-8")
        if "comments+xml" not in types:
            types = types.replace(
                "</Types>",
                '<Override PartName="/word/comments.xml" ContentType='
                '"application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/></Types>')
            items["[Content_Types].xml"] = types.encode("utf-8")
        rels = items["word/_rels/document.xml.rels"].decode("utf-8")
        if "comments.xml" not in rels:
            rels = rels.replace(
                "</Relationships>",
                '<Relationship Id="rIdCmt701" '
                'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" '
                'Target="comments.xml"/></Relationships>')
            items["word/_rels/document.xml.rels"] = rels.encode("utf-8")
        items["word/comments.xml"] = comments_xml
    tmp = path.with_suffix(".docx.tmp")
    with zipfile.ZipFile(tmp, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for name, data in items.items():
            zout.writestr(name, data)
    tmp.replace(path)
    freeze_docx(path)
    return {"path": path.name,
            "header_clause": True, "footer_clause": True, "first_page_header_diff": True,
            "textbox": 1, "comments": 1, "revisions": {"ins": 1, "del": 1}}


# ---------------------------------------------------------------- D5 扫描层
_D5A_LEFT = ["评分因素：技术方案完整性", "评分因素：设备参数响应程度", "评分因素：类似业绩",
             "评分因素：项目团队配置", "评分因素：售后服务承诺", "评分因素：实施进度计划"]
_D5A_RIGHT = ["分值：20 分", "分值：15 分", "分值：10 分", "分值：10 分", "分值：15 分", "分值：10 分"]
_D5A_TITLE = "评标办法附表（全扫描示例件——双栏排版）"


def gen_d5a(out: Path) -> dict:
    path = out / "D5a-全扫描双栏盖章.pdf"
    W, H = int(A4[0]), int(A4[1])
    img = Image.new("RGB", (W, H), (250, 249, 246))
    d = ImageDraw.Draw(img, "RGBA")
    font_t = ImageFont.truetype(NOTO_PATH, 52)
    font_h = ImageFont.truetype(NOTO_PATH, 30)
    font_b = ImageFont.truetype(NOTO_PATH, 26)
    d.text((60, 70), _D5A_TITLE, fill=(25, 25, 25), font=font_t)
    # 双栏
    y = 200
    for left, right in zip(_D5A_LEFT, _D5A_RIGHT):
        d.text((70, y), left, fill=(40, 40, 40), font=font_b)
        d.text((430, y), right, fill=(40, 40, 40), font=font_b)
        y += 64
    # 分栏线
    d.rectangle((400, 190, 402, y - 20), fill=(120, 120, 120))
    # 半透明印章遮挡（红色圆 + 内圈 + 竖排文字示意）
    cx, cy, r = W // 2 + 20, H // 2 + 130, 150
    d.ellipse((cx - r, cy - r, cx + r, cy + r), outline=(200, 30, 30, 110), width=10)
    d.ellipse((cx - r + 18, cy - r + 18, cx + r - 18, cy + r - 18), outline=(200, 30, 30, 90), width=4)
    stamp_font = ImageFont.truetype(NOTO_PATH, 34)
    d.text((cx - 52, cy - 60), "示例专用章", fill=(200, 30, 30, 130), font=stamp_font)
    d.text((cx - 46, cy + 10), "（遮挡区域）", fill=(200, 30, 30, 120), font=font_b)
    # 像素自证（防可变字体静默空白）：双栏文本+印章必须真实落在像素层
    dark = _assert_pixels_text(img, 20000, "D5a")

    c = rl_canvas.Canvas(str(path), pagesize=A4)
    c.setTitle("D5a 全扫描双栏盖章（结构测试件）")
    buf = io.BytesIO()
    img.save(buf, format="PNG", optimize=True)
    buf.seek(0)
    from reportlab.lib.utils import ImageReader
    c.drawImage(ImageReader(buf), 0, 0, width=A4[0], height=A4[1])
    c.showPage()
    c.save()
    return {"path": path.name, "text_layer": "none", "columns": 2, "stamp_occlusion": True}


_D5B_P1 = [
    "澄清通知（图文混排示例件）",
    "正文第一页含可提取文本层：本通知是对招标文件的澄清与补充。",
    "一、原招标文件第五章 5.2 条中的技术参数由文本层载明，与附表不一致时以本通知为准。",
    "二、附表（见次页图像页）为扫描版格式样例，无文本层。",
    "三、本通知不构成对投标截止时间的变更。",
]


def gen_d5b(out: Path) -> dict:
    path = out / "D5b-图文混排.pdf"
    # 第 1 页：文本层
    c = rl_canvas.Canvas(str(path), pagesize=A4)
    c.setTitle("D5b 图文混排（结构测试件）")
    pdfmetrics_font = "STSong-Light"
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.cidfonts import UnicodeCIDFont
    if pdfmetrics_font not in pdfmetrics.getRegisteredFontNames():
        pdfmetrics.registerFont(UnicodeCIDFont(pdfmetrics_font))
    y = A4[1] - 60 * mm
    for i, line in enumerate(_D5B_P1):
        c.setFont(pdfmetrics_font, 24 if i == 0 else 13)
        c.drawString(50, y, line)
        y -= 18 if i == 0 else 22
    c.showPage()
    # 第 2 页：纯图像（无文本层）
    W, H = int(A4[0]), int(A4[1])
    img = Image.new("RGB", (W, H), (252, 251, 249))
    d = ImageDraw.Draw(img)
    font_t = ImageFont.truetype(NOTO_PATH, 44)
    font_b = ImageFont.truetype(NOTO_PATH, 26)
    d.text((70, 90), "附表：响应偏离表（图像页——无文本层）", fill=(30, 30, 30), font=font_t)
    yy = 210
    for k in range(10):
        d.text((80, yy), f"序号 {k + 1}｜条款（示例）｜响应：完全响应", fill=(45, 45, 45), font=font_b)
        yy += 52
    # 像素自证（防可变字体静默空白）：附表文本必须真实落在像素层
    _assert_pixels_text(img, 8000, "D5b-附表页")
    buf = io.BytesIO()
    img.save(buf, format="PNG", optimize=True)
    buf.seek(0)
    from reportlab.lib.utils import ImageReader
    c.drawImage(ImageReader(buf), 0, 0, width=A4[0], height=A4[1])
    c.showPage()
    c.save()
    return {"path": path.name, "text_layer": "partial", "pages": 2,
            "page1_text": True, "page2_image_only": True}


# ---------------------------------------------------------------- D6 修订/批注层（T2 批）
# issue #2 评论 2026-10-27 03:28 派发：修订（w:ins/w:del）/批注（comments part）专项
# 用例集——引擎 mdcore#17（修订提取 P0）/#19（批注脚注读面 P2）验收夹具 + 标书侧回归
# 基线。期望读回按 ECMA-376：w:ins 内 w:r/w:t 为**当前生效文本**（最终视图可见），
# w:del 内 w:delText 为**历史文本**（最终视图不可见，入删除登记面）。
# 与 README「待建 D3-revisions」的区别：D3 是文档级多版本链，D6 是 OOXML tracked
# changes 结构层——互补关系，勿混淆。

_W_NS = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
D6_AUTHORS = ("示例澄清专员甲", "示例澄清专员乙", "示例澄清专家丙")
D6_DATES = ("2026-10-21T09:00:00Z", "2026-10-22T09:00:00Z", "2026-10-23T09:00:00Z")


def _d6_p_xml(children: str) -> str:
    """拼一个 w:p 的 XML（children 为已拼好的 run/ins/del 序列）。"""
    return f'<w:p {_W_NS}>{children}</w:p>'


def _d6_base(text: str) -> str:
    return f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>'


def _d6_ins(rid: int, author: str, date: str, text: str) -> str:
    """w:ins 修订插入：内含 run 为当前生效文本（ECMA-376 §17.13.5）。"""
    return (f'<w:ins w:id="{rid}" w:author="{author}" w:date="{date}">'
            f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:ins>')


def _d6_del(rid: int, author: str, date: str, text: str) -> str:
    """w:del 修订删除：内含 delText 为历史文本（ECMA-376 §17.13.4——注意非 w:t）。"""
    return (f'<w:del w:id="{rid}" w:author="{author}" w:date="{date}">'
            f'<w:r><w:delText xml:space="preserve">{text}</w:delText></w:r></w:del>')


def _d6_ns_wrap(elem_xml: str) -> str:
    """独立解析单个修订元素时的命名空间注入（嵌套于 _d6_p_xml 内时由 w:p 根声明继承，无需）。"""
    head, rest = elem_xml.split(">", 1)
    return f"{head} {_W_NS}>{rest}"


def _d6_append_paras(doc, paras_xml: list[str]):
    from docx.oxml import parse_xml
    for xml in paras_xml:
        doc.element.body.append(parse_xml(xml))


def _d6_anchor_comment(doc, paragraph, cid: int):
    """commentRange 锚定：目标段落首 run 前 Start、末尾 End+Reference（同 D2a 协议，id 参数化）。"""
    from docx.oxml import parse_xml
    W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
    first = paragraph.runs[0]._r if paragraph.runs else paragraph._p[0]
    paragraph._p.insert(0 if not paragraph.runs else list(paragraph._p).index(first),
                       parse_xml(f'<w:commentRangeStart xmlns:w="{W}" w:id="{cid}"/>'))
    paragraph._p.append(parse_xml(f'<w:commentRangeEnd xmlns:w="{W}" w:id="{cid}"/>'))
    paragraph._p.append(parse_xml(
        f'<w:r xmlns:w="{W}"><w:rPr><w:rStyle w:val="a5"/></w:rPr>'
        f'<w:commentReference w:id="{cid}"/></w:r>'))


def _d6_inject_comments(path: Path, comments: list[dict]):
    """comments part zip 层注入（同 gen_d2a 协议；支持多批注+多作者）。"""
    W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
    body = "".join(
        f'<w:comment w:id="{c["id"]}" w:author="{c["author"]}" w:date="{c["date"]}" '
        f'w:initials="{c["initials"]}"><w:p><w:r><w:t>{c["text"]}</w:t></w:r></w:p></w:comment>'
        for c in comments)
    comments_xml = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        f'<w:comments xmlns:w="{W}">{body}</w:comments>'
    ).encode("utf-8")
    with zipfile.ZipFile(path) as zin:
        items = {i.filename: zin.read(i.filename) for i in zin.infolist()}
        types = items["[Content_Types].xml"].decode("utf-8")
        if "comments+xml" not in types:
            types = types.replace(
                "</Types>",
                '<Override PartName="/word/comments.xml" ContentType='
                '"application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/></Types>')
            items["[Content_Types].xml"] = types.encode("utf-8")
        rels = items["word/_rels/document.xml.rels"].decode("utf-8")
        if "comments.xml" not in rels:
            rels = rels.replace(
                "</Relationships>",
                '<Relationship Id="rIdD6Cmt" '
                'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" '
                'Target="comments.xml"/></Relationships>')
            items["word/_rels/document.xml.rels"] = rels.encode("utf-8")
        items["word/comments.xml"] = comments_xml
    tmp = path.with_suffix(".docx.tmp")
    with zipfile.ZipFile(tmp, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for name, data in items.items():
            zout.writestr(name, data)
    tmp.replace(path)


# ---- R6a 纯插入（澄清补录形态：base + ins，2 处、单作者） ----
_D6A_INS1 = "示例大厦三层第二开标室（插入后生效）"
_D6A_INS2 = "2026-11-20 12:00（顺延两小时）"

def _gen_r6a(out: Path) -> dict:
    path = out / "R6a-纯插入澄清.docx"
    doc = Document()
    doc.add_heading("示例项目澄清文件（第 1 次·纯插入修订）", level=1)
    doc.add_paragraph("澄清一：开标地点调整如下（插入内容为当前生效口径）。")
    _d6_append_paras(doc, [
        _d6_p_xml(_d6_base("开标地点：") + _d6_ins(101, D6_AUTHORS[0], D6_DATES[0], _D6A_INS1)),
        _d6_p_xml(_d6_base("投标保证金缴纳截止时间：") + _d6_ins(102, D6_AUTHORS[0], D6_DATES[0], _D6A_INS2)),
        _d6_p_xml(_d6_base("澄清二：本澄清文件与招标文件具有同等效力；插入内容为修订生效口径。")),
    ])
    doc.save(str(path))
    freeze_docx(path)
    return {"path": path.name, "kind": "pure-insert", "ins": 2, "del": 0, "comments": 0}


# ---- R6b 纯删除（废止条款形态：句中删除 + 整段删除，2 处 delText） ----
_D6B_DEL1 = "具备建筑工程施工总承包三级及以上资质"
_D6B_DEL2 = "项目经理须持有一级注册建造师执业资格证书"

def _gen_r6b(out: Path) -> dict:
    path = out / "R6b-纯删除澄清.docx"
    doc = Document()
    doc.add_heading("示例项目澄清文件（第 2 次·纯删除修订）", level=1)
    _d6_append_paras(doc, [
        _d6_p_xml(_d6_base("原资格条款：") + _d6_del(201, D6_AUTHORS[1], D6_DATES[1], _D6B_DEL1)),
        _d6_p_xml(_d6_del(202, D6_AUTHORS[1], D6_DATES[1], _D6B_DEL2)),
        _d6_p_xml(_d6_base("上述两条资格要求经复核后删除，不再执行；删除内容为历史文本，不属于当前生效条款。")),
    ])
    doc.save(str(path))
    freeze_docx(path)
    return {"path": path.name, "kind": "pure-delete", "ins": 0, "del": 2, "comments": 0}


# ---- R6c 混合修订（同段替换 + 表格内修订 + 三作者异日期） ----
_D6C_DEL1 = "原开标时间：2026-11-15 09:30"
_D6C_INS1 = "更正后开标时间：2026-11-27 09:30"
_D6C_INS2 = "1.5%（原空白，现补充明确）"
_D6C_DEL2 = "原付款条款：预付合同价款 30% 后开工"
_D6C_TBL_INS = "商务偏差表（修订后表头）"
_D6C_TBL_DEL = "技术偏差表（原表头）"

def _gen_r6c(out: Path) -> dict:
    from docx.oxml import parse_xml
    path = out / "R6c-混合修订澄清.docx"
    doc = Document()
    doc.add_heading("示例项目澄清文件（第 3 次·混合修订）", level=1)
    _d6_append_paras(doc, [
        # 同段替换：del 在前 ins 在后——Word「修订替换」最典型形态
        _d6_p_xml(_d6_del(301, D6_AUTHORS[0], D6_DATES[0], _D6C_DEL1)
                  + _d6_ins(302, D6_AUTHORS[1], D6_DATES[1], _D6C_INS1)),
        _d6_p_xml(_d6_base("评标基准价计算公式中的下浮率K取值：")
                  + _d6_ins(303, D6_AUTHORS[2], D6_DATES[2], _D6C_INS2)),
        # 整段删除：段落仅含 del（历史文本整体废止）
        _d6_p_xml(_d6_del(304, D6_AUTHORS[1], D6_DATES[1], _D6C_DEL2)),
    ])
    # 表格内修订：表头格 ins 修正 + 数据格 del 废止（独立解析片段需 ns 注入）
    tbl = doc.add_table(rows=2, cols=2)
    tbl.cell(0, 0).paragraphs[0].add_run("偏差表类别")
    p_hdr = tbl.cell(0, 1).paragraphs[0]
    p_hdr._p.append(parse_xml(_d6_ns_wrap(_d6_ins(305, D6_AUTHORS[2], D6_DATES[2], _D6C_TBL_INS))))
    p_old = tbl.cell(1, 0).paragraphs[0]
    p_old._p.append(parse_xml(_d6_ns_wrap(_d6_del(306, D6_AUTHORS[0], D6_DATES[0], _D6C_TBL_DEL))))
    tbl.cell(1, 1).paragraphs[0].add_run("以澄清后口径为准")
    doc.save(str(path))
    freeze_docx(path)
    return {"path": path.name, "kind": "mixed", "ins": 3, "del": 3, "comments": 0, "table_cells": "ins+del"}


# ---- R6d 带批注（双作者锚定，无修订——隔离批注面） ----
_D6D_CMT1 = "评审注记一：本条澄清与招标文件第 3.2 条冲突时以本澄清为准。"
_D6D_CMT2 = "评审注记二：类似业绩年限口径已按本澄清统一，评分细则同步修订。"

def _gen_r6d(out: Path) -> dict:
    path = out / "R6d-带批注澄清.docx"
    doc = Document()
    doc.add_heading("示例项目澄清文件（第 4 次·批注面）", level=1)
    p1 = doc.add_paragraph("澄清一：类似业绩认定年限以合同签订时间为准。")
    doc.add_paragraph("澄清二：开标地点为示例大厦三层第二开标室。")
    p3 = doc.add_paragraph("澄清三：本澄清文件与招标文件具有同等效力。")
    _d6_anchor_comment(doc, p1, 801)
    _d6_anchor_comment(doc, p3, 802)
    doc.save(str(path))
    _d6_inject_comments(path, [
        {"id": 801, "author": "示例评审专家一", "date": D6_DATES[1], "initials": "PS1", "text": _D6D_CMT1},
        {"id": 802, "author": "示例评审专家二", "date": D6_DATES[2], "initials": "PS2", "text": _D6D_CMT2},
    ])
    freeze_docx(path)
    return {"path": path.name, "kind": "comments-only", "ins": 0, "del": 0, "comments": 2}


# ---- R6e 修订批注组合（澄清实务形态：del+ins+批注锚定） ----
_D6E_DEL1 = "原响应截止时间 2026-11-20 09:30"
_D6E_INS1 = "更正后响应截止时间 2026-11-27 09:30"
_D6E_INS2 = "评分细则第 2.2.2 项业绩认定年限以合同签订时间为准"
_D6E_CMT1 = "澄清说明一：本条按投标人疑问第 3 条澄清，原表述废止。"
_D6E_CMT2 = "澄清说明二：本条插入内容为新增评分口径，评审时按此执行。"

def _gen_r6e(out: Path) -> dict:
    path = out / "R6e-修订批注组合澄清.docx"
    doc = Document()
    doc.add_heading("示例项目澄清文件（第 5 次·修订批注组合）", level=1)
    _d6_append_paras(doc, [
        _d6_p_xml(_d6_base("澄清一：") + _d6_del(501, D6_AUTHORS[0], D6_DATES[0], _D6E_DEL1)
                  + _d6_ins(502, D6_AUTHORS[0], D6_DATES[0], _D6E_INS1)),
        _d6_p_xml(_d6_base("澄清二：") + _d6_ins(503, D6_AUTHORS[1], D6_DATES[1], _D6E_INS2)),
    ])
    # 批注锚定：澄清一（del+ins 条，body 第 2 段）与澄清二（ins 条，第 3 段）。
    # 注：注入段落内 run 嵌套于 w:ins，paragraph.runs 为空——commentRangeStart 落段首（结构合法）。
    _d6_anchor_comment(doc, doc.paragraphs[1], 811)
    _d6_anchor_comment(doc, doc.paragraphs[2], 812)
    doc.save(str(path))
    _d6_inject_comments(path, [
        {"id": 811, "author": "示例澄清专员甲", "date": D6_DATES[0], "initials": "CZJ", "text": _D6E_CMT1},
        {"id": 812, "author": "示例评审专家二", "date": D6_DATES[1], "initials": "PS2", "text": _D6E_CMT2},
    ])
    freeze_docx(path)
    return {"path": path.name, "kind": "combo", "ins": 2, "del": 1, "comments": 2}


def _gen_d6_expected(out: Path) -> dict:
    """期望读回契约（单一真源）：期望值直接由构造常量派生，防双处漂移。"""
    A, B, C = D6_AUTHORS
    expected = {
        "spec_ref": (
            "ECMA-376 §17.13.5（w:ins：内含 run 为当前生效文本）/"
            "§17.13.4（w:del：内含 delText 为历史文本）；"
            "comments part 按 §17.13.4.x commentRange/Reference 锚定"
        ),
        "acceptance_mapping": {
            "mdcore#17": "expected_final_view + inserted_runs + deleted_runs 逐变体可验",
            "mdcore#19": "expected_comments 逐变体可验（R6d/R6e）",
            "bid-side-baseline": "baseline/parse/R6*.json 记录当前真实行为（对照项）",
        },
        "variants": {
            "R6a-纯插入澄清.docx": {
                "kind": "pure-insert",
                "expected_final_view_paragraphs": [
                    "示例项目澄清文件（第 1 次·纯插入修订）",
                    "澄清一：开标地点调整如下（插入内容为当前生效口径）。",
                    f"开标地点：{_D6A_INS1}",
                    f"投标保证金缴纳截止时间：{_D6A_INS2}",
                    "澄清二：本澄清文件与招标文件具有同等效力；插入内容为修订生效口径。",
                ],
                "inserted_runs": [
                    {"id": 101, "author": A, "date": D6_DATES[0], "text": _D6A_INS1},
                    {"id": 102, "author": A, "date": D6_DATES[0], "text": _D6A_INS2},
                ],
                "deleted_runs": [],
                "comments": [],
                "note": "ins 文本必须出现在读出面正文流中（Word 最终视图即此形态）",
            },
            "R6b-纯删除澄清.docx": {
                "kind": "pure-delete",
                "expected_final_view_paragraphs": [
                    "示例项目澄清文件（第 2 次·纯删除修订）",
                    "原资格条款：",
                    "",
                    "上述两条资格要求经复核后删除，不再执行；删除内容为历史文本，不属于当前生效条款。",
                ],
                "inserted_runs": [],
                "deleted_runs": [
                    {"id": 201, "author": B, "date": D6_DATES[1], "text": _D6B_DEL1},
                    {"id": 202, "author": B, "date": D6_DATES[1], "text": _D6B_DEL2},
                ],
                "comments": [],
                "note": "delText 不得进入正文流；整段删除形态下段落最终视图为空串",
            },
            "R6c-混合修订澄清.docx": {
                "kind": "mixed",
                "expected_final_view_paragraphs": [
                    "示例项目澄清文件（第 3 次·混合修订）",
                    _D6C_INS1,
                    f"评标基准价计算公式中的下浮率K取值：{_D6C_INS2}",
                    "",
                ],
                "expected_table_final_view": {
                    "cells": [["偏差表类别", _D6C_TBL_INS], ["", "以澄清后口径为准"]],
                },
                "inserted_runs": [
                    {"id": 302, "author": B, "date": D6_DATES[1], "text": _D6C_INS1},
                    {"id": 303, "author": C, "date": D6_DATES[2], "text": _D6C_INS2},
                    {"id": 305, "author": C, "date": D6_DATES[2], "text": _D6C_TBL_INS},
                ],
                "deleted_runs": [
                    {"id": 301, "author": A, "date": D6_DATES[0], "text": _D6C_DEL1},
                    {"id": 304, "author": B, "date": D6_DATES[1], "text": _D6C_DEL2},
                    {"id": 306, "author": A, "date": D6_DATES[0], "text": _D6C_TBL_DEL},
                ],
                "comments": [],
                "note": "同段替换（del 前 ins 后）与表格内修订为真实澄清件高频形态；作者/日期三值分布供归属判定验收",
            },
            "R6d-带批注澄清.docx": {
                "kind": "comments-only",
                "expected_final_view_paragraphs": [
                    "示例项目澄清文件（第 4 次·批注面）",
                    "澄清一：类似业绩认定年限以合同签订时间为准。",
                    "澄清二：开标地点为示例大厦三层第二开标室。",
                    "澄清三：本澄清文件与招标文件具有同等效力。",
                ],
                "inserted_runs": [],
                "deleted_runs": [],
                "comments": [
                    {"id": 801, "author": "示例评审专家一", "text": _D6D_CMT1, "anchor": "澄清一段"},
                    {"id": 802, "author": "示例评审专家二", "text": _D6D_CMT2, "anchor": "澄清三段"},
                ],
                "note": "批注不改变正文文本（正文 4 段与无批注件全同）；批注面独立可读可数",
            },
            "R6e-修订批注组合澄清.docx": {
                "kind": "combo",
                "expected_final_view_paragraphs": [
                    "示例项目澄清文件（第 5 次·修订批注组合）",
                    f"澄清一：{_D6E_INS1}",
                    f"澄清二：{_D6E_INS2}",
                ],
                "inserted_runs": [
                    {"id": 502, "author": A, "date": D6_DATES[0], "text": _D6E_INS1},
                    {"id": 503, "author": B, "date": D6_DATES[1], "text": _D6E_INS2},
                ],
                "deleted_runs": [
                    {"id": 501, "author": A, "date": D6_DATES[0], "text": _D6E_DEL1},
                ],
                "comments": [
                    {"id": 811, "author": "示例澄清专员甲", "text": _D6E_CMT1, "anchor": "澄清一段（del+ins 条）"},
                    {"id": 812, "author": "示例评审专家二", "text": _D6E_CMT2, "anchor": "澄清二段（ins 条）"},
                ],
                "note": "澄清实务形态：del+ins 同段 + 批注锚定双面组合——#17+#19 联合验收",
            },
        },
    }
    write_json(out / "expected-readback.json", expected)
    return {"path": "expected-readback.json", "variants": len(expected["variants"])}


def gen_d6_revisions(d6: Path) -> dict:
    """d6 即目标目录（gen_corpus_r 已传入 corpus-R/D6-revisions——内部不得再拼层名）。"""
    d6.mkdir(parents=True, exist_ok=True)
    variants = [
        _gen_r6a(d6), _gen_r6b(d6), _gen_r6c(d6), _gen_r6d(d6), _gen_r6e(d6),
        _gen_d6_expected(d6),
    ]
    return {"files": variants}


# ---------------------------------------------------------------- 汇总入口
def gen_corpus_r(corpus_r_dir: Path) -> dict:
    d1 = corpus_r_dir / "D1-tables"
    d2 = corpus_r_dir / "D2-nonbody"
    d5 = corpus_r_dir / "D5-scanned"
    d6 = corpus_r_dir / "D6-revisions"
    for d in (d1, d2, d5, d6):
        d.mkdir(parents=True, exist_ok=True)
    layers = {
        "D1-tables": [gen_d1a(d1), gen_d1b(d1)],
        "D2-nonbody": [gen_d2a(d2)],
        "D5-scanned": [gen_d5a(d5), gen_d5b(d5)],
        "D6-revisions": gen_d6_revisions(d6)["files"],
    }
    (corpus_r_dir / "README.md").write_text(README, encoding="utf-8")
    files = [f["path"] for rows in layers.values() for f in rows]
    assert sorted(Path(f).name for f in files) == sorted(Path(f).name for f in OUTPUT_FILES), \
        f"corpus-R 产出与 OUTPUT_FILES 漂移：{files}"
    return {"version": CORPUS_R_VERSION, "files": files, "layers": layers,
            "counts": {"D1": 2, "D2": 1, "D5": 2, "D6": 6, "total": len(files)}}


if __name__ == "__main__":
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("fixtures/corpus-R")
    print(gen_corpus_r(out)["counts"])
