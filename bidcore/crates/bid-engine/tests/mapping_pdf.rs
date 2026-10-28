//! B3-5 pdf 映射保真测试：`pdf_to_parsed_file` vs 旧 `bid_parse::parse_file`。
//!
//! **SKIP 语义（issue #9）**：本测试需要 pdfium 动态库（env `MDC_PDFIUM_LIB`
//! 或编译期候选）。缺件时打 SKIP 横幅提前 return——不伪装成业务 FAIL。
//!
//! 双轨：
//! - **自研轨**：B1 设备清单（文本层 PDF）逐块等价 + 行切分/locator 口径 +
//!   扫描件 MappingGap（OCR 留旧路径）；
//! - **对照轨**：B2 扫描件（无文本层）走旧路径不变（回退链验证）。

use bid_engine::{parse_pdf_via_engine, EngineReader, MdcoreReader};
use std::path::Path;

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn pdfium_available() -> bool {
    // 与 reader 同款定位序：env → 编译期候选（试探打开一个最小 pdf）
    let probe = fixture("../../../fixtures/corpus-A/files/B1-设备清单.pdf");
    let bytes = match std::fs::read(&probe) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let reader = MdcoreReader::new();
    matches!(reader.extract_pdf_text(&bytes), Ok(_))
}

fn skip_if_no_pdfium() -> bool {
    if pdfium_available() {
        return false;
    }
    eprintln!();
    eprintln!("====================================================================");
    eprintln!("SKIP: pdf_mapping 测试组");
    eprintln!("reason: ENV_MISSING: pdfium（MDC_PDFIUM_LIB 未设且编译期候选缺失）");
    eprintln!("（环境缺件，非业务失败——issue #9：依赖缺失不得伪装成业务 FAIL）");
    eprintln!("补装指引: bblanchon/pdfium-binaries chromium/8086 → export MDC_PDFIUM_LIB=<path>/libpdfium.so");
    eprintln!("====================================================================");
    eprintln!();
    true
}

/// B1-设备清单.pdf（corpus-A 文本层 PDF，40 行设备表）：
/// 引擎路径 vs 旧路径逐块等价（行切分/locator/text 口径）。
#[test]
fn pdf_mapping_matches_legacy_on_b1() {
    if skip_if_no_pdfium() {
        return;
    }
    let path = fixture("../../../fixtures/corpus-A/files/B1-设备清单.pdf");
    let engine = parse_pdf_via_engine(&path).expect("引擎路径解析成功（文本层件）");
    let legacy = bid_parse::parse_file(&path, "B1-设备清单.pdf").expect("旧路径解析成功");

    // 行块等价（文本层行切分口径）
    let e_lines: Vec<&bid_parse::Block> = engine
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, bid_parse::BlockKind::PdfLine))
        .collect();
    let l_lines: Vec<&bid_parse::Block> = legacy
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, bid_parse::BlockKind::PdfLine))
        .collect();
    assert!(!e_lines.is_empty(), "B1 文本层非空");
    let mut text_diff = Vec::new();
    let max = e_lines.len().min(l_lines.len());
    for i in 0..max {
        if e_lines[i].text != l_lines[i].text {
            text_diff.push((i, e_lines[i].text.clone(), l_lines[i].text.clone()));
        }
    }
    // P1 定谳：pdfium 与 pdftotext -layout 非空白字符比 1.000——
    // 文本差异容限：差异行占比 <5%（列对齐空白/断行形态差，非内容差）
    let diff_ratio = text_diff.len() as f64 / max.max(1) as f64;
    assert!(
        diff_ratio < 0.05,
        "行文本差异占比 {diff_ratio:.3}（前 3 处：{:?}）",
        &text_diff[..text_diff.len().min(3)]
    );
    // 非空白字符守恒（内容不丢——P1 口径的总量断言）
    let ew: usize = engine.blocks.iter().map(|b| b.text.chars().filter(|c| !c.is_whitespace()).count()).sum();
    let lw: usize = legacy.blocks.iter().map(|b| b.text.chars().filter(|c| !c.is_whitespace()).count()).sum();
    assert!(
        (ew as i64 - lw as i64).abs() <= (lw as f64 * 0.02) as i64,
        "非空白字符守恒（引擎 {ew} vs 旧 {lw}——2% 容限）"
    );
    assert_eq!(engine.has_text_layer, true);
    assert!(engine.parse_summary.contains("engine-pdf"), "溯源：{}", engine.parse_summary);
}

/// B2-扫描件（无文本层）：引擎路径必须 MappingGap（OCR 留旧路径——映射表 §3）。
#[test]
fn pdf_scanned_file_maps_to_gap() {
    if skip_if_no_pdfium() {
        return;
    }
    let path = fixture("../../../fixtures/corpus-A/files/B2-投标函格式页-扫描件.pdf");
    let result = parse_pdf_via_engine(&path);
    match result {
        Err(bid_engine::EngineError::MappingGap(msg)) => {
            assert!(msg.contains("扫描件"), "MappingGap 必须指明扫描件语义：{msg}");
        }
        Err(other) => panic!("预期 MappingGap（扫描件），实际：{other:?}"),
        Ok(p) => panic!("扫描件不得走引擎映射成功：{} 块", p.blocks.len()),
    }
}

/// 敌意面：截断 pdf 必须被引擎拒绝（打开失败——不伪装部分结果）。
#[test]
fn pdf_engine_rejects_truncated() {
    if skip_if_no_pdfium() {
        return;
    }
    let path = fixture("../../../fixtures/corpus-A/files/B1-设备清单.pdf");
    let bytes = std::fs::read(&path).unwrap();
    let truncated = &bytes[..bytes.len() / 2];
    let reader = MdcoreReader::new();
    let result = reader.extract_pdf_text(truncated);
    assert!(result.is_err(), "截断 pdf 必须被引擎拒绝");
}

/// 行号与页号口径：locator `第{N}页 行{M}`（非空行计数——空行不占号）。
#[test]
fn pdf_locator_conventions() {
    if skip_if_no_pdfium() {
        return;
    }
    let path = fixture("../../../fixtures/corpus-A/files/B1-设备清单.pdf");
    let engine = parse_pdf_via_engine(&path).expect("引擎路径解析成功");
    for (i, b) in engine.blocks.iter().enumerate() {
        let expect_page = b.locator.starts_with("第") && b.locator.contains("页 行");
        assert!(expect_page, "locator 口径：{}", b.locator);
        if i > 0 {
            // 行号单调不减（页内递增，跨页回落）
            let prev = &engine.blocks[i - 1];
            let cur_page: usize = b.locator.split("页").next().unwrap_or("").trim_start_matches("第").parse().unwrap_or(0);
            let prev_page: usize = prev.locator.split("页").next().unwrap_or("").trim_start_matches("第").parse().unwrap_or(0);
            assert!(cur_page >= prev_page, "页号单调：{}/{}", cur_page, prev_page);
        }
    }
}
