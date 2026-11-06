//! #324（R-12/B-04）：`pdf_to_parsed_file` tables 指标修复测试（映射副本）。
//!
//! 与 bid-parse `poppler.rs` 的 `text_layer_parsed` 同型双副本——审计只登记了
//! poppler 侧，本侧（engine 映射）是同型死指标第二副本，两处同修（issue 验收）。
//!
//! 本测试**不需要 pdfium**：直接构造 `EnginePdf` 调纯映射函数
//! （`pdf_to_parsed_file` 是确定性字段搬运，无引擎调用）。

use bid_engine::types::EnginePdf;
use bid_engine::{pdf_to_parsed_file, FileMeta};

fn meta() -> FileMeta {
    FileMeta {
        name: "tables-metric-probe.pdf".to_string(),
        sha256: String::new(),
        bytes: 0,
    }
}

/// 折叠前判定：列对齐双空格行计入 tables，块文本仍折叠（口径不变）。
#[test]
fn pdf_tables_metric_counts_pre_collapse_double_spaces() {
    let pdf = EnginePdf {
        pages: vec![
            "序号  设备名称  数量\n1  心电监护仪  3\n正常段落单空格 文本一行".to_string()],
        any_text_layer: true,
    };
    let pf = pdf_to_parsed_file(pdf, &meta()).expect("文本层件映射成功");
    assert_eq!(pf.blocks.len(), 3, "三行块");
    assert_eq!(pf.tables, 2, "两行列对齐双空格（折叠前判定）——恒 0 即死指标回归");
    assert!(
        pf.blocks.iter().all(|b| !b.text.contains("  ")),
        "块文本仍折叠为单空格（tables 是指标修复，块文本口径不变）"
    );
}

/// 负例：纯段落（无连续双空格）→ tables == 0（不虚报）。
#[test]
fn pdf_tables_metric_zero_on_plain_paragraphs() {
    let pdf = EnginePdf {
        pages: vec!["第一章 采购需求\n本项目为医疗设备采购。\n交货期：合同签订后 30 日内。".to_string()],
        any_text_layer: true,
    };
    let pf = pdf_to_parsed_file(pdf, &meta()).expect("文本层件映射成功");
    assert_eq!(pf.tables, 0, "纯段落不虚报表格行");
}

/// 与旧路径（bid-parse poppler）同口径：同一页文本两副本 tables 计数一致。
#[test]
fn pdf_tables_metric_parity_with_legacy_text_layer() {
    // 直接调 bid-parse 的公开等价面：parse_file 需要真件，这里钉「同输入同计数」
    // 的口径契约——两副本对同一段文本必须产出相同 tables（同修后的一致性锚）。
    let page_text = "包1  监护仪  10台\n包2  呼吸机  5台\n说明文字一整段";
    let engine_pdf = EnginePdf {
        pages: vec![page_text.to_string()],
        any_text_layer: true,
    };
    let pf = pdf_to_parsed_file(engine_pdf, &meta()).expect("文本层件映射成功");
    assert_eq!(pf.tables, 2, "engine 映射副本：两行双空格");
    // 旧路径副本经 engines::poppler::text_layer_parsed（pub(super)——
    // 由 bid-parse 侧单测 text_layer_tables_metric_counts_pre_collapse_double_spaces
    // 钉同款断言，此处不跨 crate 重复引用，口径由两处测试共同锚定。
}
