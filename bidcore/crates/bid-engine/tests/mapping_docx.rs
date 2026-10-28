//! B5 映射保真测试：`docx_to_parsed_file` vs 旧 `bid_parse::parse_file` 同件对比。
//!
//! 验收口径（issue #2 §B5 import 接线）：引擎路径产物必须与旧路径**块流等价**
//! （B3-3 差分已证 ≥90% 等价 + 差异全归类；本测试是产品映射面的抽样锚定——
//! A-招标文件.docx 为 B3-3 全量差分 100% EQ 件，映射后必须逐块等价）。
//!
//! 双轨测试设计（issue #11 测试纪律）：
//! - **自研**：块流等价（kind/locator/text/cells 逐块）、账本登记面、
//!   章节上下文 locator 口径、空段对齐、表行行号消耗；
//! - **借鉴 mdcore**（fixtures/engine-borrowed/TR2，PROVENANCE.md 溯源）：
//!   word-complex.docx 统计面（tables/comments/fields）经引擎读回后账本一致。

use bid_engine::{parse_docx_via_engine, EngineReader, MDCORE_REV};
use std::path::Path;

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// A-招标文件.docx（corpus-A 主文件）：引擎路径 vs 旧路径逐块等价
/// （B3-3 差分该件 100% EQUIVALENT——映射层不得引入新差异）。
#[test]
fn mapping_matches_legacy_on_corpus_a_main() {
    let path = fixture("../../../fixtures/corpus-A/files/A-招标文件.docx");
    let engine = parse_docx_via_engine(&path).expect("引擎路径解析成功（A 件无嵌套表）");
    let legacy = bid_parse::parse_file(&path, "A-招标文件.docx").expect("旧路径解析成功");

    // 块流等价：kind + text + cells（locator 允许空差异——旧路径同文件同口径应一致）
    assert_eq!(
        engine.blocks.len(),
        legacy.blocks.len(),
        "块数一致（B3-3 差分 100% EQ 件——空段对齐后块数必须相等）"
    );
    let mut kind_mismatch = 0usize;
    let mut text_mismatch = Vec::new();
    for (e, l) in engine.blocks.iter().zip(&legacy.blocks) {
        if e.kind != l.kind {
            kind_mismatch += 1;
        }
        if e.text.trim() != l.text.trim() {
            text_mismatch.push((e.idx, e.text.clone(), l.text.clone()));
        }
    }
    assert_eq!(kind_mismatch, 0, "kind 全等");
    assert!(
        text_mismatch.is_empty(),
        "文本全等（前 3 处差异：{:?}）",
        &text_mismatch[..text_mismatch.len().min(3)]
    );

    // 表计数一致（ParsedFile.tables——下游 chunk 索引消费面）
    assert_eq!(engine.tables, legacy.tables, "表计数一致");
    // parse_summary 溯源：引擎路径必须携带 rev
    assert!(
        engine.parse_summary.contains(MDCORE_REV),
        "parse_summary 溯源 rev：{}",
        engine.parse_summary
    );
}

/// 章节上下文 locator：引擎映射必须携带与旧路径同款章节（extract.rs 类别映射依赖）。
#[test]
fn mapping_carries_chapter_context() {
    let path = fixture("../../../fixtures/corpus-A/files/A-招标文件.docx");
    let engine = parse_docx_via_engine(&path).expect("引擎路径解析成功");
    let legacy = bid_parse::parse_file(&path, "A-招标文件.docx").expect("旧路径解析成功");

    // 旧路径有章节的块，引擎路径 locator 必须同值（' > ' 分隔口径——split_locator 消费）
    let mut locator_mismatch = Vec::new();
    for (e, l) in engine.blocks.iter().zip(&legacy.blocks) {
        if !l.locator.is_empty() && e.locator != l.locator {
            locator_mismatch.push((e.idx, e.locator.clone(), l.locator.clone()));
        }
    }
    assert!(
        locator_mismatch.is_empty(),
        "章节 locator 口径一致（前 3 处：{:?}）",
        &locator_mismatch[..locator_mismatch.len().min(3)]
    );
}

/// SC-02 部件账本：引擎路径登记面（正文/页眉/页脚/文本框/批注/脚注尾注/修订/图片）。
#[test]
fn mapping_emits_component_ledger() {
    let path = fixture("../../../fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx");
    let engine = parse_docx_via_engine(&path).expect("引擎路径解析成功（D2a：文本框+批注+修订混合件）");

    let names: Vec<&str> = engine.components.iter().map(|c| c.component.as_str()).collect();
    for expect in ["正文", "页眉", "页脚", "文本框", "批注", "脚注尾注", "修订插入", "修订删除", "图片"] {
        assert!(
            names.contains(&expect),
            "账本缺 {expect} 面（实际：{names:?}）——SC-02 对账不变式"
        );
    }
    // D2a 已知部件（B3-2 验收口径）：文本框 1 + 批注 1 + 修订删除 1
    let tb = engine.components.iter().find(|c| c.component == "文本框").unwrap();
    assert_eq!(tb.present, 1, "D2a 文本框 1（#18 口径）");
    let cm = engine.components.iter().find(|c| c.component == "批注").unwrap();
    assert_eq!(cm.present, 1, "D2a 批注 1（#19 口径）");
    let rd = engine.components.iter().find(|c| c.component == "修订删除").unwrap();
    assert!(rd.present >= 1, "D2a 修订删除登记（delText 计数）");
    // 零提取面必带 note（SC-02 不变式）
    for c in &engine.components {
        if c.extracted == 0 && c.present > 0 {
            assert!(
                !c.note.is_empty(),
                "部件 {} present>0 而 extracted=0 必须带 note（登记不伪装）",
                c.component
            );
        }
    }
}

/// 敌意面：截断容器必须被引擎三门拒绝（A3 口径——映射层不自研守卫）。
#[test]
fn mapping_rejects_truncated_container() {
    let path = fixture("../../../fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx");
    let bytes = std::fs::read(&path).unwrap();
    let truncated = &bytes[..bytes.len() / 2];
    let reader = bid_engine::MdcoreReader::new();
    let result = reader.read_docx(truncated);
    assert!(result.is_err(), "截断容器必须被引擎拒绝（三门语义）");
}

/// 借鉴 mdcore 夹具（TR2/word-complex.docx，PROVENANCE.md 溯源）：
/// 引擎读回统计面（tables=1/comments=1）经映射后账本一致。
#[test]
fn mapping_borrowed_word_complex_stats() {
    let path = fixture("../../../fixtures/engine-borrowed/TR2/word-complex.docx");
    let engine = parse_docx_via_engine(&path).expect("TR2 word-complex 引擎路径解析成功");
    // gen-log.txt 口径：tables=1 / comments=1 / revisions=1 / fields=1
    assert_eq!(engine.tables, 1, "TR2 word-complex tables=1（gen-log 口径）");
    let cm = engine.components.iter().find(|c| c.component == "批注").unwrap();
    assert_eq!(cm.present, 1, "TR2 word-complex comments=1（gen-log 口径）");
    // 修订面（revisions=1：ins 或 del 至少一面登记）
    let has_rev = engine.components.iter().any(|c| {
        (c.component == "修订插入" || c.component == "修订删除") && c.present > 0
    });
    assert!(has_rev, "TR2 word-complex revisions=1（gen-log 口径）——修订面必须登记");
}

/// 表行细节口径：全空行跳过但行号消耗；text=非空列单空格连接（旧实现对齐）。
#[test]
fn mapping_table_row_conventions() {
    let path = fixture("../../../fixtures/corpus-R/D1-tables/D1a-合并表头跨页表.docx");
    let engine = parse_docx_via_engine(&path).expect("D1a 引擎路径解析成功");
    let legacy = bid_parse::parse_file(&path, "D1a-合并表头跨页表.docx").expect("旧路径解析成功");

    let e_rows: Vec<&bid_parse::Block> = engine
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, bid_parse::BlockKind::TableRow))
        .collect();
    let l_rows: Vec<&bid_parse::Block> = legacy
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, bid_parse::BlockKind::TableRow))
        .collect();
    assert_eq!(e_rows.len(), l_rows.len(), "表行数一致（D1a 100% EQ 件）");
    for (e, l) in e_rows.iter().zip(&l_rows) {
        assert_eq!(e.text, l.text, "表行 text 口径（非空列单空格连接）");
        assert_eq!(e.cells, l.cells, "表行 cells 直连");
        assert_eq!(e.locator, l.locator, "表行 locator（{{章}} 表N 行M）");
    }
}
