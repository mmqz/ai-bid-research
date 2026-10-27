//! B3-2 reader 保真度测试（EngineReader::read_docx——引擎→镜像字段搬运零漂移）。
//!
//! 双轨设计（issue #11 测试要求）：
//! - **自研**：镜像结构与引擎结构逐一对应（块数/块型/批注锚/修订面/拒绝分类）；
//! - **借鉴 mdcore**：期望值取自引擎仓已验收口径——
//!   - `fixtures/engine-borrowed/TR2/gen-log.txt`（word-complex 生成统计行）
//!   - `fixtures/corpus-R/D6-revisions/expected-readback.json`（R6 系修订读回）
//!   - `docs/engine-bid-mapping.md` T17d 验收数（R6b delText warnings=2）

use bid_engine::{EngineBlock, EngineError, EngineReader, MdcoreReader};

fn fixture(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read_docx(rel: &str) -> bid_engine::EngineDoc {
    let bytes = std::fs::read(fixture(rel)).expect("夹具在位");
    MdcoreReader::new().read_docx(&bytes).expect("引擎读链成功")
}

/// 拍平块流为段落文本序（含文本框内层——#18 同构语义）。
fn flattened_para_texts(doc: &bid_engine::EngineDoc) -> Vec<String> {
    fn walk(blocks: &[EngineBlock], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                EngineBlock::Paragraph(p) => out.push(p.text.clone()),
                EngineBlock::Table(_) => {}
                EngineBlock::TextBox(tb) => walk(&tb.blocks, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.blocks, &mut out);
    out
}

/// 期望段落序必须以**子序列**出现在镜像读回（Word 最终视图语义——
/// delText 段可能消失/合并，ins 段为生效口径）。
fn assert_subsequence(expected: &[String], actual: &[String], ctx: &str) {
    let mut it = actual.iter().peekable();
    for e in expected {
        let found = it.find(|a| a.trim() == e.trim());
        assert!(
            found.is_some(),
            "{ctx}: 期望段未按序出现：{e:?}\n实际流：{actual:#?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 自研：镜像结构保真
// ---------------------------------------------------------------------------

/// D2a：镜像块数/块型与引擎块序一一对应（TextBox 保持结构——映射层才拍平）。
#[test]
fn reader_docx_mirror_preserves_block_structure() {
    let doc = read_docx("../../../fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx");
    assert!(!doc.blocks.is_empty(), "块流非空");

    // D2a 含文本框（#18）——镜像保结构，内层块与正文同构（段落）
    let tb_texts: Vec<String> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            EngineBlock::TextBox(tb) => Some(
                tb.blocks
                    .iter()
                    .filter_map(|ib| match ib {
                        EngineBlock::Paragraph(p) => Some(p.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        tb_texts.len(),
        1,
        "D2a 文本框内层段数（引擎验收：text_boxes=1）"
    );
    assert!(
        tb_texts[0].contains("文本框条款"),
        "内层段语义在位：{}",
        &tb_texts[0][..tb_texts[0].len().min(30)]
    );

    // 引擎保留空段（EMPTY-PARA 预期差异面——映射层才丢弃，reader 必须忠实传递）
    let empty_paras = doc
        .blocks
        .iter()
        .filter(|b| matches!(b, EngineBlock::Paragraph(p) if p.text.trim().is_empty()))
        .count();
    assert!(
        empty_paras >= 1,
        "引擎空段面透传（EMPTY-PARA 差异面源数据）：{empty_paras}"
    );
}

/// D2a：批注镜像携带锚与内容文本（#19）。
#[test]
fn reader_docx_carries_comments_with_anchor() {
    let doc = read_docx("../../../fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx");
    assert_eq!(doc.comments.len(), 1, "D2a 批注数（引擎验收口径）");
    let c = &doc.comments[0];
    assert!(c.anchor_block_idx.is_some(), "锚关联在位（非孤儿批注）");
    assert!(!c.texts.is_empty(), "批注内容文本非空");
}

/// 截断容器：必须以 `EngineRejected`（引擎三门语义）拒绝——**不是**
/// `EngineFailed`。敌意面差分依赖此分类（B4 前置 A3）。
#[test]
fn reader_docx_truncated_rejected_with_engine_rejected_variant() {
    let bytes = std::fs::read(fixture(
        "../../../fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx",
    ))
    .expect("夹具在位");
    let truncated = &bytes[..bytes.len() / 2];
    match MdcoreReader::new().read_docx(truncated) {
        Err(EngineError::EngineRejected(_)) => {} // 正确分类
        Err(other) => panic!("截断容器应为 EngineRejected，实际：{other:?}"),
        Ok(_) => panic!("截断容器不应读成功"),
    }
}

/// 段落级 B3 扩面字段全链搬运：heading_level（A1）/ fields_dropped（A2）/
/// footnote_refs（#19）/ has_numbering。
#[test]
fn reader_docx_carries_paragraph_extension_fields() {
    let doc = read_docx("../../../real-samples/北京水利施工示范文本2025-LO.docx");
    // 真实件必有标题段（A1 四链解析——真 Word 中文版 styleId="1" 形覆盖）
    let headings = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            EngineBlock::Paragraph(p) => p.heading_level,
            _ => None,
        })
        .count();
    assert!(headings > 0, "真实件标题段 A1 解出面：{headings}");
}

// ---------------------------------------------------------------------------
// 借鉴：mdcore 已验收期望（gen-log / expected-readback / T17d）
// ---------------------------------------------------------------------------

/// word-complex.docx（mdcore TR2 借鉴件）：镜像统计面 == gen-log.txt 生成统计行
/// （"paras=13 tables=1 comments=1 revisions=1 fields=1"——引擎侧验收口径）。
#[test]
fn borrowed_word_complex_matches_gen_log_stats() {
    let doc = read_docx("../../../fixtures/engine-borrowed/TR2/word-complex.docx");

    let tables = doc
        .blocks
        .iter()
        .filter(|b| matches!(b, EngineBlock::Table(_)))
        .count();
    assert_eq!(tables, 1, "gen-log: tables=1");

    assert_eq!(doc.comments.len(), 1, "gen-log: comments=1");

    // revisions=1：w:ins 修订在位（插入文本进 revision_inserted 标记面）
    let ins: usize = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            EngineBlock::Paragraph(p) => Some(p.revision_inserted.len()),
            _ => None,
        })
        .sum();
    assert!(ins >= 1, "gen-log: revisions=1（ins run 标记面：{ins}）");

    // fields=1：A2 表现型域剥离计数透传
    let fields: usize = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            EngineBlock::Paragraph(p) => Some(p.fields_dropped),
            _ => None,
        })
        .sum();
    assert!(
        fields >= 1,
        "gen-log: fields=1（fields_dropped 透传：{fields}）"
    );
}

/// R6 系修订件（expected-readback.json 借鉴期望）：各变体的
/// expected_final_view_paragraphs 必须以子序列出现在镜像段落流。
#[test]
fn borrowed_expected_readback_r6_final_view() {
    let readback: serde_json::Value = serde_json::from_reader(
        std::fs::File::open(fixture(
            "../../../fixtures/corpus-R/D6-revisions/expected-readback.json",
        ))
        .expect("expected-readback.json 在位"),
    )
    .expect("JSON 合法");

    let variants = readback["variants"].as_object().expect("variants 对象");
    assert!(variants.len() >= 5, "R6a-R6e 五变体：{}", variants.len());

    for (fname, spec) in variants {
        let doc = read_docx(&format!("../../../fixtures/corpus-R/D6-revisions/{fname}"));
        let actual = flattened_para_texts(&doc);
        let expected: Vec<String> = spec["expected_final_view_paragraphs"]
            .as_array()
            .expect("期望段落数组")
            .iter()
            .map(|v| v.as_str().expect("字符串").to_string())
            .collect();
        assert_subsequence(&expected, &actual, fname);
    }
}

/// R6a：插入修订文本 == expected-readback inserted_runs（#17 标记面与文本面双验）。
#[test]
fn borrowed_r6a_inserted_runs_match_readback() {
    let doc = read_docx("../../../fixtures/corpus-R/D6-revisions/R6a-纯插入澄清.docx");
    let readback: serde_json::Value = serde_json::from_reader(
        std::fs::File::open(fixture(
            "../../../fixtures/corpus-R/D6-revisions/expected-readback.json",
        ))
        .expect("readback 在位"),
    )
    .expect("JSON 合法");
    let expected: Vec<String> = readback["variants"]["R6a-纯插入澄清.docx"]["inserted_runs"]
        .as_array()
        .expect("inserted_runs 数组")
        .iter()
        .map(|r| r["text"].as_str().expect("text").to_string())
        .collect();
    assert!(!expected.is_empty(), "readback 期望非空");

    let actual: Vec<String> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            EngineBlock::Paragraph(p) => Some(p.revision_inserted.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(actual, expected, "R6a 插入 run 文本面与引擎验收口径一致");
}

/// R6b：删除修订登记计数 == 2（T17d 验收口径——RevisionDelete 警告逐段计）。
#[test]
fn borrowed_r6b_delete_registered_via_warning_count() {
    let doc = read_docx("../../../fixtures/corpus-R/D6-revisions/R6b-纯删除澄清.docx");
    let del = doc
        .warnings
        .iter()
        .filter(|w| w.code == "RevisionDelete")
        .count();
    assert_eq!(del, 2, "T17d：R6b RevisionDelete warnings=2");
}
