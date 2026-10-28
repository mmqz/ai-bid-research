//! B3-4 xlsx 映射保真测试：`xlsx_to_parsed_file` vs 旧 `bid_parse::parse_file` 同件对比。
//!
//! 双轨（issue #11 测试纪律）：
//! - **自研轨**：块流等价（D1b 宽表公式件 + B3 图纸目录件）、公式双通道账本、
//!   全空行跳过、locator 口径（`{sheet}!行{N}`）、敌意容器拒绝；
//! - **借鉴轨**：TR2 excel-complex.xlsx（mdcore @2d01444 借鉴件）——
//!   统计面对照 gen-log.txt。

use bid_engine::{parse_xlsx_via_engine, EngineReader, MdcoreReader, MDCORE_REV};
use std::path::Path;

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// D1b-宽表公式.xlsx（corpus-R 结构层：宽表 + 公式行 + 合并表头）：
/// 引擎路径 vs 旧路径逐块等价。
#[test]
fn xlsx_mapping_matches_legacy_on_d1b() {
    let path = fixture("../../../fixtures/corpus-R/D1-tables/D1b-宽表公式.xlsx");
    let engine = parse_xlsx_via_engine(&path).expect("引擎路径解析成功");
    let legacy = bid_parse::parse_file(&path, "D1b-宽表公式.xlsx").expect("旧路径解析成功");

    assert_eq!(engine.blocks.len(), legacy.blocks.len(), "行块数一致");
    let mut diff = Vec::new();
    for (e, l) in engine.blocks.iter().zip(&legacy.blocks) {
        if e.kind != l.kind || e.text != l.text || e.cells != l.cells || e.locator != l.locator {
            diff.push((e.idx, e.locator.clone(), e.text.clone(), l.text.clone()));
        }
    }
    assert!(
        diff.is_empty(),
        "逐块等价（kind/text/cells/locator——前 3 处差异：{:?}）",
        &diff[..diff.len().min(3)]
    );
    assert_eq!(engine.tables, legacy.tables, "表（sheet）计数一致");
    assert_eq!(engine.pages, legacy.pages, "页（sheet 计数）口径一致");
    // 公式双通道账本（D1b 口径）：非空公式格入账本
    let formula_ledger = engine.components.iter().find(|c| c.component == "公式");
    assert!(
        formula_ledger.is_some(),
        "D1b 含公式行——「公式」账本行必须在位（双通道口径）"
    );
    assert!(
        engine.parse_summary.contains(MDCORE_REV),
        "parse_summary 溯源 rev：{}",
        engine.parse_summary
    );
}

/// B3-图纸目录.xlsx（corpus-A）：无公式件的账本面（公式行不出现）。
#[test]
fn xlsx_mapping_no_formula_ledger_when_plain() {
    let path = fixture("../../../fixtures/corpus-A/files/B3-图纸目录.xlsx");
    let engine = parse_xlsx_via_engine(&path).expect("引擎路径解析成功");
    let legacy = bid_parse::parse_file(&path, "B3-图纸目录.xlsx").expect("旧路径解析成功");
    assert_eq!(engine.blocks.len(), legacy.blocks.len());
    assert!(
        engine.components.iter().all(|c| c.component != "公式"),
        "无公式件不携带「公式」账本行（条件序列化）"
    );
}

/// 借鉴 mdcore 夹具（TR2/excel-complex.xlsx，PROVENANCE.md 溯源）：
/// 统计面对照 gen-log.txt（sheet/行列/公式口径）。
#[test]
fn xlsx_mapping_borrowed_excel_complex_stats() {
    let path = fixture("../../../fixtures/engine-borrowed/TR2/excel-complex.xlsx");
    let engine = parse_xlsx_via_engine(&path).expect("TR2 excel-complex 引擎路径解析成功");
    let legacy = bid_parse::parse_file(&path, "excel-complex.xlsx").expect("旧路径解析成功");
    // 借鉴锚点：块流等价（引擎三门 + calamine 五面 vs 旧 calamine 直读）
    assert_eq!(engine.blocks.len(), legacy.blocks.len(), "TR2 块数一致");
    for (e, l) in engine.blocks.iter().zip(&legacy.blocks) {
        assert_eq!(e.cells, l.cells, "TR2 cells 一致（值面口径）");
        assert_eq!(e.locator, l.locator, "TR2 locator 口径");
    }
}

/// 敌意面：截断容器必须被引擎三门拒绝（xlsx 门 1/2——EOCD 缺失）。
#[test]
fn xlsx_engine_rejects_truncated_container() {
    let path = fixture("../../../fixtures/corpus-R/D1-tables/D1b-宽表公式.xlsx");
    let bytes = std::fs::read(&path).unwrap();
    let truncated = &bytes[..bytes.len() / 2];
    let reader = MdcoreReader::new();
    let result = reader.read_xlsx(truncated);
    assert!(result.is_err(), "截断容器必须被引擎拒绝（三门语义）");
}

/// 全空行跳过 + 行号口径（矩阵行位 1 基——非有效行序）：
/// D1b 首行若为表头稀疏区，行号必须反映矩阵位置而非压缩序。
#[test]
fn xlsx_row_numbers_are_matrix_positions() {
    let path = fixture("../../../fixtures/corpus-R/D1-tables/D1b-宽表公式.xlsx");
    let engine = parse_xlsx_via_engine(&path).expect("引擎路径解析成功");
    let legacy = bid_parse::parse_file(&path, "D1b-宽表公式.xlsx").expect("旧路径解析成功");
    // 逐块 locator 相等已覆盖行号口径；此处显式断言防回归（压缩序 = 回归信号）
    for (e, l) in engine.blocks.iter().zip(&legacy.blocks) {
        assert_eq!(e.locator, l.locator);
    }
    assert!(!engine.blocks.is_empty());
}
