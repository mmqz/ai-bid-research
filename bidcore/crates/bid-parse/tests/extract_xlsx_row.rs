//! #472（B-08 + B-07②）：XlsxRow 黑洞回归——xlsx 清单型文件的抽取面。
//!
//! 背景：`extract_main_docx` 的 `match b.kind` 只认 Para/TableRow，XlsxRow
//! 全落 `_ => {}`——kind=招标文件的 `.xlsx`（清单型招标文件真实存在）整份
//! 静默零抽取（audit/module-audit-B.md B-08，CONFIRMED）；`extract_b1_lines`
//! 只认 PdfLine——B1 xlsx 报价清单的型号/中文大写日期行 0 抽取（B-07②）。
//! `table_key_of` 只认 docx「… 表N 行M」形态——xlsx locator「Sheet1!行5」
//! 每行异键，审查语境逐行被重置。
//!
//! #472 收口：XlsxRow 并入 TableRow 分支（text/cells 同构，通用否决/资格/
//! 参数分支适用；review_map/semantic 结构感知只按 TableRow 建，无双抽）；
//! b1_lines 扩 XlsxRow；table_key_of 补 xlsx 形态（sheet 名为表键）。

use bid_parse::extract::extract_requirements_with_kind;
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

/// XlsxRow 块（locator 形态对齐 bid-engine xlsx_to_parsed_file：
/// 「{sheet}!行{row}」；text = 非空单元格空格 join；cells = 原始行）。
fn xrow(idx: usize, sheet: &str, row: usize, cells: &[&str]) -> Block {
    let text = cells.join(" ");
    Block {
        idx,
        kind: BlockKind::XlsxRow,
        locator: format!("{sheet}!行{row}"),
        text,
        cells: cells.iter().map(|s| s.to_string()).collect(),
    }
}

/// B-08 主断言：xlsx 主文件（kind=招标文件）的负面清单行/审查表行
/// 不再落入 `_ => {}` 黑洞——按 TableRow 同构语义产出否决项 REQ。
#[test]
fn xlsx_main_negative_list_rows_extracted_as_veto() {
    let f = pf(
        "设备采购需求清单.xlsx",
        vec![
            xrow(0, "资格审查", 2, &["投标人", "弄虚作假", "取消投标资格"]),
            xrow(1, "资格审查", 3, &["串通投标", "其投标无效"]),
        ],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("招标文件".to_string()));
    assert_eq!(
        reqs.len(),
        2,
        "两条负面清单行必须产出——修复前全落 `_ => {{}}` 黑洞（0 条）"
    );
    assert!(reqs.iter().all(|r| r.category == "否决项"));
    // 锚点保真：xlsx_row kind + sheet!行 定位（下游 get_document_chunks
    // /coverage 对账依赖锚面与块面一致）
    assert_eq!(reqs[0].anchors[0].kind, "xlsx_row");
    assert!(reqs[0].anchors[0].locator.contains("资格审查!行2"));
    assert_eq!(reqs[0].source_file, "设备采购需求清单.xlsx");
}

/// B-08 审查语境面：xlsx 审查表内语境跨行传播——表头行开启
/// review_ctx，后续符合性行继承否决语义（table_key_of 兼容 xlsx
/// locator 是前提：同 sheet 同键不重置语境）。
#[test]
fn xlsx_review_context_propagates_within_sheet() {
    let f = pf(
        "评审细则.xlsx",
        vec![
            // 表头行：opens_review_context（「评审标准」）
            xrow(0, "评审标准", 1, &["评审标准", "投标人名称"]),
            // 符合性行：review_ctx 内 is_review_standard（「应与…一致」）
            xrow(1, "评审标准", 2, &["投标人名称应与营业执照一致"]),
        ],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("招标文件".to_string()));
    assert_eq!(
        reqs.len(),
        1,
        "审查语境行须产出——table_key_of 不兼容 xlsx locator 时语境逐行被重置（0 条）"
    );
    assert_eq!(reqs[0].category, "否决项");
}

/// B-08 跨 sheet 语境隔离：表切换检测对 xlsx = sheet 切换——
/// 上一 sheet 的审查语境不得泄漏到下一 sheet。
#[test]
fn xlsx_review_context_resets_across_sheets() {
    let f = pf(
        "评审与参数.xlsx",
        vec![
            xrow(0, "评审标准", 1, &["评审标准", "初步评审"]),
            // 跨 sheet：技术参数 sheet 的型号行不受审查语境影响
            xrow(1, "技术需求", 1, &["核心交换机", "HW-9700"]),
        ],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("招标文件".to_string()));
    // 型号行：locator 含「技术参数」+ re_model 命中 → 技术参数 REQ
    //（审查语境已跨 sheet 重置——不把它误判成否决项）
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].category, "技术参数");
}

/// B-07② 主断言：B1 报价清单的 xlsx 形态（XlsxRow + 型号/中文大写
/// 日期行）不再 0 抽取——与 PdfLine 形态同判。
/// （注：re_date_cn 的字符类不含「十」——「十二月/三十一日」形态
/// 不命中是既有词面口径，非本单回归；用例取「一月一日」无十形态。）
#[test]
fn b1_xlsx_rows_extract_technical_params() {
    let f = pf(
        "B1-设备报价清单.xlsx",
        vec![
            xrow(0, "报价", 2, &["核心交换机", "HW-9700", "2台"]),
            xrow(1, "报价", 3, &["到货期", "二〇二六年一月一日前"]),
        ],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("报价文件".to_string()));
    assert_eq!(
        reqs.len(),
        2,
        "型号行 + 中文大写日期行必须产出——修复前 extract_b1_lines 只认 PdfLine（0 条）"
    );
    assert!(reqs.iter().all(|r| r.category == "技术参数"));
    // 参数面：型号/日期进 params（fill_params 消费）
    let has_model = reqs
        .iter()
        .any(|r| r.params.values().any(|v| v.to_string().contains("HW-9700")));
    assert!(has_model, "型号值须进 params（到货/规格要求行的参数面）");
}

/// 回归锁：无条款语义的 xlsx 主文件（纯数据行）仍 0 产出——不虚报。
#[test]
fn xlsx_plain_data_rows_stay_silent() {
    let f = pf(
        "数量统计.xlsx",
        vec![
            xrow(0, "Sheet1", 1, &["序号", "名称", "数量"]),
            xrow(1, "Sheet1", 2, &["1", "签字笔", "40"]),
        ],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("招标文件".to_string()));
    assert!(
        reqs.is_empty(),
        "纯数据行不得虚报 REQ（B-08 修复是可见性修复，不是放宽抽取）"
    );
}
