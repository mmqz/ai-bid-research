//! B3 冒烟验收（issue #11 验收纪律 3——「不许读了就信」）：
//!
//! 本测试**直接调用引擎公开 API** 读取真实夹具，证明 git 依赖不是空壳：
//! - `read_docx_from_bytes` 走引擎全链（容器三门 → 自有关系层 → docx-rs 模型 → 联接提取）；
//! - 断言的形状锚点来自 `docs/engine-bid-mapping.md`（D2a/R6 系夹具在引擎侧的
//!   已验收读回：#17 修订 / #18 文本框 / #19 批注）。
//!
//! 注意：本测试是**引擎直调探针**（证明接通），不是映射层验收——
//! EngineReader 实装与 ParsedFile 映射在 lib 内测试面（B3 后续批次）。

use mdc_office_io::docx::read_docx_from_bytes;

fn fixture(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// D2a-非正文条款.docx（文本框 + 批注 + 修订混合件——B3 差分基准对之一）：
/// 引擎必须成功读出块流，且文本框/批注面非零（#18/#19 修复已在钉住的 rev 内）。
#[test]
fn smoke_engine_reads_d2a_nonbody_fixture() {
    let bytes = std::fs::read(fixture("../../../fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx"))
        .expect("夹具在位（仓库 fixtures/corpus-R）");
    let doc = read_docx_from_bytes(&bytes).expect("引擎读链必须成功（容器三门放行真件）");

    assert!(!doc.blocks.is_empty(), "块流非空：实际 {} 块", doc.blocks.len());
    // 文本框内容在块流里（#18：宿主段之后文档序发射）
    let has_textbox = doc.blocks.iter().any(|b| matches!(b, mdc_office_io::docx::Block::TextBox(_)));
    assert!(has_textbox, "D2a 含文本框（#18 读面）：blocks={}", doc.blocks.len());
    // 批注面（#19）
    assert_eq!(doc.comments.len(), 1, "D2a 批注数（引擎侧已验收口径）");
}

/// R6a-纯插入澄清.docx：#17 修订插入标记面（w:ins 内 run 的 revision_inserted）。
#[test]
fn smoke_engine_reads_r6a_insert_revision_fixture() {
    let bytes = std::fs::read(fixture("../../../fixtures/corpus-R/D6-revisions/R6a-纯插入澄清.docx"))
        .expect("夹具在位");
    let doc = read_docx_from_bytes(&bytes).expect("引擎读链成功");

    // 修订插入 run 标记面（#17：w:ins 内 run 的 revision_inserted = true；
    // 映射表 T17d 验收口径：R6a ins 全入读回）
    let ins_runs = doc.blocks.iter().filter_map(|b| match b {
        mdc_office_io::docx::Block::Paragraph(p) => {
            Some(p.runs.iter().filter(|r| r.revision_inserted).count())
        }
        _ => None,
    }).sum::<usize>();
    assert!(ins_runs > 0, "R6a 含插入修订 run：{}", ins_runs);

    // 纯插入件无删除修订（RevisionDelete 警告计数为 0）
    let del_count = doc
        .warnings
        .iter()
        .filter(|w| matches!(w.kind, mdc_office_io::io::WarningKind::RevisionDelete))
        .count();
    assert_eq!(del_count, 0, "R6a 纯插入无删除修订");
}

/// R6b-纯删除澄清.docx：#17 修订删除登记面（delText 不入正文，逐段
/// RevisionDelete 警告计数——映射表 T17d 验收口径 = 2）。
#[test]
fn smoke_engine_reads_r6b_delete_revision_fixture() {
    let bytes = std::fs::read(fixture("../../../fixtures/corpus-R/D6-revisions/R6b-纯删除澄清.docx"))
        .expect("夹具在位");
    let doc = read_docx_from_bytes(&bytes).expect("引擎读链成功");

    let del_count = doc
        .warnings
        .iter()
        .filter(|w| matches!(w.kind, mdc_office_io::io::WarningKind::RevisionDelete))
        .count();
    assert_eq!(del_count, 2, "R6b 删除修订逐段登记（T17d 口径）");
}

/// 北京水利示范文本（真实 LO 转换件，58 页眉 + 58 页脚 + 52 表）：
/// 引擎读大件块流 + 页眉页脚挂接面（P2-1 前读面即有 HeaderFooterData）。
#[test]
fn smoke_engine_reads_real_beijing_sample() {
    let bytes = std::fs::read(fixture("../../../real-samples/北京水利施工示范文本2025-LO.docx"))
        .expect("真实样本在位");
    let doc = read_docx_from_bytes(&bytes).expect("引擎读链成功");

    assert!(doc.blocks.len() > 1000, "真实件块流量级：{}", doc.blocks.len());
    assert!(!doc.headers.is_empty(), "页眉挂接面非空（D3：引擎有读面）");
    assert!(!doc.footers.is_empty(), "页脚挂接面非空");
}

/// 敌意容器：引擎三门必须拒绝（截断容器——baseline/security/04 同族）。
#[test]
fn smoke_engine_rejects_truncated_container() {
    let bytes = std::fs::read(fixture("../../../fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx"))
        .expect("夹具在位");
    let truncated = &bytes[..bytes.len() / 2];
    let result = read_docx_from_bytes(truncated);
    assert!(result.is_err(), "截断容器必须被引擎拒绝（三门语义——标书侧不自研守卫）");
}
