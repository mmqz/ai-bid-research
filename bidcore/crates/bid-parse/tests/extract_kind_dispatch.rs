//! #196 / #199 审查回归：按业务 kind 分发的值域矩阵。
//!
//! 背景：`extract_requirements_with_kind` 的分发必须与导入面
//! `TENDER_FILE_KINDS`（招标文件|附件|图纸|补遗|答疑|评分办法|格式模板）
//! 对齐。初版（#199 首commit）把主抽取写成封闭白名单
//! （招标文件|补遗|澄清），「答疑/评分办法/格式模板」三分支全不命中 →
//! 静默跳过——#196 要修的「真实文件名静默零抽取」换了马甲回来。
//! 现行为反向清单：仅附件/图纸/证据跳过，其余登记 kind 一律主抽取
//!（fail-visible，配合工具面 ZERO_REQUIREMENTS 告警）。

use bid_parse::extract::{extract_requirements, extract_requirements_with_kind};
use bid_parse::{Block, BlockKind, ParsedFile};

fn pf(name: &str, blocks: Vec<Block>) -> ParsedFile {
    ParsedFile {
        name: name.to_string(),
        sha256: "test".into(),
        pages: 1,
        tables: 0,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary: "test".into(),
        components: vec![],
    }
}

fn para(idx: usize, locator: &str, text: &str) -> Block {
    Block {
        idx,
        kind: BlockKind::Para,
        locator: locator.into(),
        text: text.into(),
        cells: vec![],
    }
}

/// 一份含两条可抽取条款的主文件内容（编号段落 + 章节定位）。
fn main_content() -> Vec<Block> {
    vec![
        para(
            0,
            "第二章 投标人须知 > 3.资格",
            "3.4.2 投标人未提供有效营业执照的，其投标将被否决。",
        ),
        para(
            1,
            "第二章 投标人须知 > 3.资格",
            "3.4.3 投标人须具备水利水电工程施工总承包贰级及以上资质。",
        ),
    ]
}

// #271：「格式模板」从主抽取面移入无条款面——格式件是官方填空模板
// （声明函/偏离表），条款真源是主件格式章 REQ；与 附件/图纸/证据 同口径。
const KINDS_MAIN: &[&str] = &["招标文件", "补遗", "答疑", "评分办法"];
const KINDS_SKIP: &[&str] = &["附件", "图纸", "证据", "格式模板"];

/// 主抽取面：TENDER_FILE_KINDS 中除附件/图纸/证据外的全部登记 kind，
/// 无论文件名带不带夹具前缀，都必须产出条款（#196 核心 + 审查修复面）。
#[test]
fn registered_kinds_dispatch_to_main_extraction() {
    for kind in KINDS_MAIN {
        for name in ["光纤租赁费招标文件.docx", "A-招标文件.docx"] {
            let f = pf(name, main_content());
            let reqs =
                extract_requirements_with_kind(&[f], |n| if n == name { Some(kind.to_string()) } else { None });
            assert_eq!(
                reqs.len(),
                2,
                "kind={kind} name={name} 应主抽取 2 条（实际 {}）",
                reqs.len()
            );
        }
    }
}

/// 附件面：附件/图纸/证据无条款抽取面——即使文件名带 A- 前缀也跳过
/// （kind 是登记的业务事实，优先于夹具前缀约定）。
#[test]
fn attachment_kinds_skip_even_with_main_prefix() {
    for kind in KINDS_SKIP {
        let f = pf("A-招标文件.docx", main_content());
        let reqs = extract_requirements_with_kind(&[f], |_| Some(kind.to_string()));
        assert!(
            reqs.is_empty(),
            "kind={kind} 应跳过抽取（实际 {} 条）",
            reqs.len()
        );
    }
}

/// 兼容面：kind 缺省（None）时前缀兜底——真实文件名（无前缀）不再
/// 静默零抽取（#196 原始缺陷的回归锁），B1/B2 前缀行为不变。
#[test]
fn none_kind_falls_back_to_prefix_and_real_names_extract() {
    // 真实文件名 + 无 kind → 主抽取（#196 原始缺陷）
    let f = pf("测试文件.docx", main_content());
    let reqs = extract_requirements_with_kind(&[f], |_| None);
    assert_eq!(reqs.len(), 2, "真实文件名（无 kind）不得静默零抽取");
    // 旧兼容入口 = kind 全 None 的等价面
    let reqs_old = extract_requirements(&[pf("测试文件.docx", main_content())]);
    assert_eq!(reqs_old.len(), 2, "兼容入口与 None-kind 语义一致");
    // B1/B2 前缀兜底保持（夹具约定路径不受 kind 缺省影响）
    let b1 = pf(
        "B1-设备清单.docx",
        vec![para(0, "附表1", "设备清单表内容——型号与数量")],
    );
    let reqs_b1 = extract_requirements_with_kind(&[b1], |_| None);
    // B1 路径产出形状由 b1 抽取器决定（可能为 0），关键是【不走主抽取
    // 抛错】——此处仅验证分发不 panic 且有定义行为。
    let _ = reqs_b1;
}

// ================================================================
// #473（B-07/B-08）：B1/B2 kind 判据活化 + XlsxRow 黑洞回归。
//
// 背景：B1/B2 分支的 kind 判据（Some("报价文件")/Some("扫描件")）此前
// 不在导入面 TENDER_FILE_KINDS 值域内——kind 路径不可达（死分支），
// 生产只能靠 B1/B2 文件名前缀触达（#196「前缀不得作生产判据」在
// B1/B2 面回归）；且 XlsxRow 块在 B1 面（只匹配 PdfLine）与主抽取面
// （extract_main_docx 只匹配 Para/TableRow，`_ => {}`）双黑洞。

fn xlsx_row(idx: usize, sheet: &str, row: usize, cells: Vec<&str>) -> Block {
    let text = cells
        .iter()
        .filter(|c| !c.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    Block {
        idx,
        kind: BlockKind::XlsxRow,
        locator: format!("{sheet}!行{row}"),
        text,
        cells: cells.into_iter().map(String::from).collect(),
    }
}

fn ocr_line(idx: usize, locator: &str, text: &str) -> Block {
    Block {
        idx,
        kind: BlockKind::OcrLine,
        locator: locator.into(),
        text: text.into(),
        cells: vec![],
    }
}

/// ① B1 kind 活化（B-07①②）：kind=报价文件（真实文件名，无 B1 前缀）
/// + xlsx 报价清单（XlsxRow 型号行）→ 技术参数 REQ。修前双黑洞：
/// kind 登记不进导入面 + XlsxRow 不匹配 PdfLine。
#[test]
fn b1_kind_quotation_xlsx_extracts() {
    let f = pf(
        "监控设备报价清单.xlsx",
        vec![
            xlsx_row(0, "报价", 1, vec!["序号", "设备名称", "型号", "数量"]),
            xlsx_row(1, "报价", 2, vec!["1", "网络摄像机", "IPC-4T-IR-D2", "20"]),
            xlsx_row(2, "报价", 3, vec!["2", "录像机", "NVR-64H-8P", "2"]),
        ],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("报价文件".into()));
    assert_eq!(
        reqs.len(),
        2,
        "kind=报价文件 的 xlsx 报价清单（2 行型号）不得静默零抽取（B-07①②）"
    );
    assert!(
        reqs.iter().all(|r| r.category == "技术参数"),
        "B1 面产出应全为技术参数，实际：{:?}",
        reqs.iter().map(|r| r.category.as_str()).collect::<Vec<_>>()
    );
}

/// ② B2 kind 活化（B-07①）：kind=扫描件（真实文件名，无 B2 前缀）
/// + OCR 否决行 → 否决项 REQ。
#[test]
fn b2_kind_scan_ocr_extracts() {
    let f = pf(
        "资质证书扫描件.pdf",
        vec![ocr_line(
            0,
            "第1页 行3",
            "未按规定签署并盖章的投标文件，评标委员会将否决其投标",
        )],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("扫描件".into()));
    assert_eq!(
        reqs.len(),
        1,
        "kind=扫描件 的 OCR 否决行不得静默零抽取（B-07①）"
    );
    assert_eq!(reqs[0].category, "否决项");
}

/// ③ XlsxRow 主文件黑洞回归（B-08）：kind=招标文件 的清单型 xlsx
/// （blocks 全 XlsxRow）此前落入 extract_main_docx 的 `_ => {}` 静默
/// 零抽取。修后按表格行同口径：否决词行 → 否决项 REQ。
#[test]
fn xlsx_main_file_not_blackholed() {
    let f = pf(
        "物料采购需求清单.xlsx",
        vec![
            xlsx_row(0, "须知", 1, vec!["条款", "内容"]),
            xlsx_row(1, "须知", 2, vec!["1", "投标文件未按要求密封的，其投标将被否决"]),
        ],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("招标文件".into()));
    assert_eq!(
        reqs.len(),
        1,
        "xlsx 主文件的 XlsxRow 否决行不得落入 `_ => {{}}` 黑洞（B-08）"
    );
    assert_eq!(reqs[0].category, "否决项");
    // 定位保真：anchor 指回原块（sheet/行号在 locator 可查）
    assert!(
        reqs[0]
            .anchors
            .iter()
            .any(|a| a.locator.contains("须知!行2")),
        "anchor locator 应保留 xlsx 块定位，实际：{:?}",
        reqs[0].anchors.iter().map(|a| a.locator.as_str()).collect::<Vec<_>>()
    );
}

/// ④ xlsx 审查语境传播（#473 table_key_of 增强）：同一 sheet 内
/// 「评审标准」表头行开启语境 → 后续符合性行继承否决语义。修前
/// xlsx locator（`{sheet}!行{N}`）无 ` 行` 分隔 → 每行自成表键 →
/// review_ctx 逐行重置，语境传播在 xlsx 面失效。
#[test]
fn xlsx_review_context_propagates_within_sheet() {
    let f = pf(
        "评审细则.xlsx",
        vec![
            xlsx_row(0, "评审标准", 1, vec!["评审标准", "说明"]),
            xlsx_row(1, "评审标准", 2, vec!["投标保证金", "应与招标文件要求相一致"]),
            xlsx_row(2, "其他", 3, vec!["联系人", "张三"]),
        ],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("招标文件".into()));
    assert_eq!(
        reqs.len(),
        1,
        "同一 sheet 内审查语境应传播到符合性行（实际 {} 条：{:?}）",
        reqs.len(),
        reqs.iter().map(|r| (r.category.as_str(), r.anchors[0].locator.as_str())).collect::<Vec<_>>()
    );
    // 「其他」sheet 是新表键 → 语境终止，联系人行不产出
    assert!(
        !reqs.iter()
            .any(|r| r.anchors.iter().any(|a| a.locator.contains("其他!行3"))),
        "跨 sheet 语境应终止（表键切换）"
    );
}
