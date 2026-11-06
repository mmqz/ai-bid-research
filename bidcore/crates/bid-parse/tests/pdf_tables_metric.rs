//! #324（R-12/B-04）：tables 死指标修复——真件正例验收（poppler 文本层）。
//!
//! 审计：`collapse_spaces` 折叠后 `contains("  ")` 恒假 → `ParsedFile.tables`
//! 恒 0，任何消费该指标的统计/诊断面（import 诊断 JSON / realfiles / a8）
//! 读到假 0。修复后真件正例必须 `tables > 0`。
//!
//! 环境门（issue #9 / AGENTS.md 13.1）：poppler 三件（pdftotext/pdftoppm/
//! pdfinfo）+ 真件 fixture 缺任一 → SKIP 横幅提前 return，不伪装通过。

use std::path::PathBuf;

fn fixtures() -> PathBuf {
    std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // crates/bid-parse → 仓库根
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
        })
}

/// B1-设备清单.pdf（corpus-A 文本层 PDF，40 行设备表）：
/// 修复前 `tables` 恒 0（死指标）；修复后列对齐行被计数——真件正例。
#[test]
fn real_pdf_tables_metric_positive_on_b1() {
    if bid_parse::env_preflight::skip_if_env_missing(
        "real_pdf_tables_metric_positive_on_b1",
        bid_parse::env_preflight::POPPLER_TEXT_TOOLS,
    ) {
        return;
    }
    let files_dir = fixtures().join("corpus-A/files");
    if bid_parse::env_preflight::skip_if_fixture_missing(
        "real_pdf_tables_metric_positive_on_b1",
        &[&files_dir],
    ) {
        return;
    }
    let path = files_dir.join("B1-设备清单.pdf");
    let pf = bid_parse::parse_file(&path, "B1-设备清单.pdf")
        .expect("B1 文本层件解析成功（poppler 在位）");
    assert!(pf.has_text_layer, "B1 是文本层件（非扫描）");
    assert!(
        pf.tables > 0,
        "B1 为 40 行设备表——tables 必须 > 0（恒 0 即死指标回归）；实际 {}",
        pf.tables
    );
    assert!(
        pf.blocks.iter().all(|b| !b.text.contains("  ")),
        "块文本口径不变（仍折叠单空格——只修指标不修文本）"
    );
}
