//! B3-4 xlsx 映射保真测试：`xlsx_to_parsed_file` 黄金锚点（B4 固化）。
//!
//! B4（issue #11 任务 3）：自研 xlsx 解析删除后 legacy 对照面不复存在——
//! 原「引擎 vs 旧路径」差分断言改造为**固化常量锚点**。取值时点：mdcore
//! @3204342d（PR #31 r3：全量差分 EQ+ED 91.57% 验收，D1b/B3 件 100% EQ）。
//! 锚点语义：引擎 bump 或映射回归的**防回归锚**——任何数值漂移必须来自
//! 显式的 mdcore rev 升级 PR（重跑差分后同步更新锚点），禁止静默漂移。
//!
//! 双轨（issue #11 测试纪律）：
//! - **自研轨**：块数/sheet 数/公式双通道账本/合并区账本/locator 口径
//!   （`{sheet}!行{N}` 矩阵行位）/敌意容器拒绝（数值锚点 + 逐块抽查）；
//! - **借鉴轨**：TR2 excel-complex.xlsx（mdcore @2d01444 借鉴件）——
//!   统计面对照 gen-log.txt。

use bid_engine::{parse_xlsx_via_engine, EngineReader, MdcoreReader, MDCORE_REV};
use std::path::Path;

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// 真实样本在位预检（issue #9 SKIP 机制文件面——PR #24 修复批次）：
/// 该测试文件依赖的 fixture 目录缺失 → 响亮横幅 + 调用方立即 return
/// （缺件不是业务失败，也绝不静默假装通过）。
fn fixtures_ready(test_id: &str) -> bool {
    let dirs: Vec<std::path::PathBuf> = ["../../../fixtures/corpus-R/D1-tables",
        "../../../fixtures/corpus-A/files",
        "../../../fixtures/engine-borrowed/TR2"]
        .iter()
        .map(|d| fixture(d))
        .collect();
    let refs: Vec<&std::path::Path> = dirs.iter().map(|p| p.as_path()).collect();
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &refs)
}

/// D1b-宽表公式.xlsx（corpus-R 结构层：宽表 + 公式行 + 合并表头）：
/// 引擎路径黄金锚点（B4 前差分锚定固化值——PR #31 r3 该件 100% EQ）。
/// 破坏推演：行块发射/全空行跳过/值面口径回归 → 任一断言红。
#[test]
fn xlsx_mapping_golden_anchor_on_d1b() {
    if !fixtures_ready("xlsx_mapping_golden_anchor_on_d1b") {{
        return;
    }}
    let path = fixture("../../../fixtures/corpus-R/D1-tables/D1b-宽表公式.xlsx");
    let engine = parse_xlsx_via_engine(&path).expect("引擎路径解析成功");

    // 块流总量 + sheet/页口径（B4 锚点：15 有效行 / 1 表 / 1 页）
    assert_eq!(engine.blocks.len(), 15, "行块数（D1b 黄金锚）");
    assert_eq!(engine.tables, 1, "表（sheet）计数");
    assert_eq!(engine.pages, 1, "页（sheet 计数）口径");
    assert_eq!(engine.parse_summary, format!(
        "engine-xlsx @ {MDCORE_REV}｜1 工作表，15 有效行，61 非空单元格"),
        "parse_summary 统计面（有效行/非空单元格锚）：{}", engine.parse_summary);

    // 逐块抽查：首行（标题行）与末行（合计行——仅第 5 列非空）
    let b0 = &engine.blocks[0];
    assert_eq!(b0.locator, "量价汇总!行1", "locator=「{{sheet}}!行{{N}}」口径");
    assert_eq!(b0.text, "量价汇总表（示例——公式与合并区结构测试件）", "首行标题锚");
    let b_last = engine.blocks.last().unwrap();
    assert_eq!(b_last.locator, "量价汇总!行15", "末行矩阵行位");
    assert_eq!(b_last.cells, ["", "", "", "", "合计", ""], "合计行 cells（值面）");
    assert_eq!(b_last.text, "合计", "text=非空列单空格连接");

    // 公式双通道账本（D1b 口径）：非空公式格入账本（present=13 / extracted=0）
    let formula_ledger = engine.components.iter().find(|c| c.component == "公式");
    assert!(
        formula_ledger.is_some(),
        "D1b 含公式行——「公式」账本行必须在位（双通道口径）"
    );
    assert_eq!(formula_ledger.unwrap().present, 13, "非空公式格数（黄金锚）");
    // 合并区账本：6 矩形
    let merge_ledger = engine.components.iter().find(|c| c.component == "合并区").unwrap();
    assert_eq!(merge_ledger.present, 6, "合并区矩形数（黄金锚）");
    assert!(
        engine.parse_summary.contains(MDCORE_REV),
        "parse_summary 溯源 rev：{}",
        engine.parse_summary
    );
}

/// B3-图纸目录.xlsx（corpus-A）：无公式件的账本面（公式行不出现）+
/// 矩阵行位黄金锚（23 块而末块「行24」——行 3 为空行被跳过且**不压缩行号**）。
#[test]
fn xlsx_mapping_no_formula_ledger_when_plain() {
    if !fixtures_ready("xlsx_mapping_no_formula_ledger_when_plain") {{
        return;
    }}
    let path = fixture("../../../fixtures/corpus-A/files/B3-图纸目录.xlsx");
    let engine = parse_xlsx_via_engine(&path).expect("引擎路径解析成功");
    assert_eq!(engine.blocks.len(), 23, "行块数（B3 黄金锚）");
    assert!(
        engine.components.iter().all(|c| c.component != "公式"),
        "无公式件不携带「公式」账本行（条件序列化）"
    );
    // 矩阵行位证明：块数 23 < 末块行号 24（行 3 空行跳过、行号不压缩）
    let b0 = &engine.blocks[0];
    assert_eq!(b0.locator, "图纸目录!行1", "首行矩阵行位");
    assert_eq!(b0.text, "附件 B3：图纸目录", "首行标题锚");
    let b_last = engine.blocks.last().unwrap();
    assert_eq!(b_last.locator, "图纸目录!行24", "末块=矩阵行位 24（非压缩序 23）");
    assert_eq!(b_last.text, "20 GC-20 系统接地示意图 A3 4 V1.0", "末行内容锚");
    assert!(engine.blocks.iter().all(|b| b.locator != "图纸目录!行3"), "空行 3 不入流");
    // 合并区账本（B3 有 1 处合并表头）
    let merge_ledger = engine.components.iter().find(|c| c.component == "合并区").unwrap();
    assert_eq!(merge_ledger.present, 1, "合并区矩形数（黄金锚）");
}

/// 借鉴 mdcore 夹具（TR2/excel-complex.xlsx，PROVENANCE.md 溯源）：
/// 统计面对照 gen-log.txt（sheet/行列/公式口径）——黄金锚（B4 固化）。
#[test]
fn xlsx_mapping_borrowed_excel_complex_stats() {
    if !fixtures_ready("xlsx_mapping_borrowed_excel_complex_stats") {{
        return;
    }}
    let path = fixture("../../../fixtures/engine-borrowed/TR2/excel-complex.xlsx");
    let engine = parse_xlsx_via_engine(&path).expect("TR2 excel-complex 引擎路径解析成功");
    // 借鉴锚点（B4 固化）：8 行块 / 2 表（sheet）/ 2 页 / 公式格 2
    assert_eq!(engine.blocks.len(), 8, "TR2 块数一致");
    assert_eq!((engine.tables, engine.pages), (2, 2), "sheet/页口径");
    // 逐块抽查（值面 cells + locator——含跨 sheet 与矩阵行位跳空）
    assert_eq!(engine.blocks[0].locator, "报价明细!行1");
    assert_eq!(engine.blocks[0].cells, ["序号", "名称"], "TR2 首块 cells（值面口径）");
    assert_eq!(engine.blocks[4].locator, "评分表!行3");
    assert_eq!(engine.blocks[4].cells, ["商务", "30", "0.3"], "TR2 评分表 cells");
    assert_eq!(engine.blocks[7].locator, "评分表!行7", "末块矩阵行位（行 6 空行跳过）");
    assert_eq!(engine.blocks[7].cells, ["招标公告", "", ""], "TR2 末块 cells");
    let formula = engine.components.iter().find(|c| c.component == "公式").unwrap();
    assert_eq!(formula.present, 2, "TR2 公式格数（gen-log 口径）");
}

/// 敌意面：截断容器必须被引擎三门拒绝（xlsx 门 1/2——EOCD 缺失）。
#[test]
fn xlsx_engine_rejects_truncated_container() {
    if !fixtures_ready("xlsx_engine_rejects_truncated_container") {{
        return;
    }}
    let path = fixture("../../../fixtures/corpus-R/D1-tables/D1b-宽表公式.xlsx");
    let bytes = std::fs::read(&path).unwrap();
    let truncated = &bytes[..bytes.len() / 2];
    let reader = MdcoreReader::new();
    let result = reader.read_xlsx(truncated);
    assert!(result.is_err(), "截断容器必须被引擎拒绝（三门语义）");
}

/// 全空行跳过 + 行号口径（矩阵行位 1 基——非有效行序）：
/// D1b 逐块 locator 全列锚（B4 固化：行 1..15 连续满行——压缩序与矩阵位
/// 恰同值，跳空面的矩阵位证明由 B3 件「行24≠块23」承担）。
#[test]
fn xlsx_row_numbers_are_matrix_positions() {
    if !fixtures_ready("xlsx_row_numbers_are_matrix_positions") {{
        return;
    }}
    let path = fixture("../../../fixtures/corpus-R/D1-tables/D1b-宽表公式.xlsx");
    let engine = parse_xlsx_via_engine(&path).expect("引擎路径解析成功");
    // 逐块 locator 全列（D1b 无空行——锚定「{sheet}!行{N}」精确格式）
    for (i, b) in engine.blocks.iter().enumerate() {
        assert_eq!(b.locator, format!("量价汇总!行{}", i + 1), "第 {i} 块 locator");
    }
    assert!(!engine.blocks.is_empty());
}
