//! bidi.rs —— #285 ② 块级双向索引（派生态构建面）。
//!
//! 数据纪律（issue #285 ②）：
//! - **派生态不持久化**：`block_requirement_index` 每次从 REQ anchors +
//!   BlockMarks 现算，可随时重建（旧快照/新抽取同一入口）；
//! - 块位真源 = `Anchor.block_idx`（#285 ② 起抽取时回填）；旧快照锚
//!   （字段为 None）按 `(locator, excerpt)` 精确匹配兜底解析——与
//!   `merge.rs` ② 过滤、real_corpus_e2e 锚→块对齐同一纪律；
//! - 只索引**有 REQ 的块**（`req_ids` 非空才出行）——「无 REQ 块」是
//!   覆盖报告（`get_coverage_report`）与 overlay 的面，不混进本索引；
//! - 同块不双源：`merge_extraction` 的 ② 过滤保证 Agent 覆盖块丢弃正则
//!   REQ，`marked_by` 取 REQ 落库键（`params.marked_by`）。

use crate::{Block, ParsedFile};
use bid_domain::coverage::{BlockMark, BlockRequirementIndex, BlockVerdict, MarkSource};
use bid_domain::{Anchor, Requirement};

/// 锚 → 块位（正向索引的单锚腿）。
///
/// 优先 `anchor.block_idx`（新抽取回填值，含越界防御）；旧快照锚按
/// `(locator, excerpt)` 与块 `(locator, text)` 精确匹配兜底——匹配不到
/// （跨版本解析差异/部分列锚）→ `None` 诚实口径，不猜。
pub fn resolve_anchor_block_idx(pf: &ParsedFile, a: &Anchor) -> Option<usize> {
    if let Some(bi) = a.block_idx {
        return pf.blocks.get(bi).map(|_| bi);
    }
    pf.blocks
        .iter()
        .find(|b| b.locator == a.locator && b.text == a.excerpt)
        .map(|b| b.idx)
}

/// REQ → 块位集合（正向索引：一个 REQ 可跨多块——续行/双锚形态）。
///
/// 只收本文件锚（`anchor.file == pf.name`）；全部锚都解析不到块 → 空集
/// （REQ 仍在库，但本文件索引不收——诚实：锚失效可见于 provenance 面）。
pub fn requirement_block_idxs(pf: &ParsedFile, r: &Requirement) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for a in &r.anchors {
        if a.file != pf.name {
            continue;
        }
        if let Some(bi) = resolve_anchor_block_idx(pf, a) {
            if !out.contains(&bi) {
                out.push(bi);
            }
        }
    }
    out
}

/// REQ 的块级来源（落库键类型化：`params.marked_by`——extract.rs "regex" /
/// merge.rs "agent"；缺键按正则产物口径——agent 项必写）。
pub fn mark_source_of(r: &Requirement) -> MarkSource {
    match r.params.get("marked_by").and_then(|v| v.as_str()) {
        Some("agent") => MarkSource::Agent,
        _ => MarkSource::RegexFallback,
    }
}

/// 块级双向索引（派生态主体）：block → req_ids 反查面。
///
/// 行序 = 块序（文档序）；`verdict` 恒 [`BlockVerdict::Requirement`]——
/// 索引行 = 有 REQ 的块（Agent REQ 必落 requirement 区间、正则 REQ 本就是
/// 要求），非 requirement 结论的块（noise/rule）由 marks 面呈现，不在此。
pub fn block_requirement_index(
    pf: &ParsedFile,
    marks: &[BlockMark],
    reqs: &[Requirement],
) -> Vec<BlockRequirementIndex> {
    let _ = marks; // marks 不进索引行（verdict 已定 Requirement）；保留参数：
                   // 签名面给消费方（工具层同时要 marks 与索引），避免二次收集
    let mut by_block: std::collections::BTreeMap<usize, Vec<&Requirement>> =
        std::collections::BTreeMap::new();
    for r in reqs {
        for bi in requirement_block_idxs(pf, r) {
            by_block.entry(bi).or_default().push(r);
        }
    }
    by_block
        .into_iter()
        .map(|(block_idx, rs)| {
            // 同块多 REQ 时取首个 REQ 的来源（②过滤保证不混源；防御性口径）
            let marked_by = rs
                .first()
                .map(|r| mark_source_of(r))
                .unwrap_or(MarkSource::RegexFallback);
            BlockRequirementIndex {
                file_sha256: pf.sha256.clone(),
                block_idx,
                req_ids: rs.iter().map(|r| r.id.clone()).collect(),
                verdict: BlockVerdict::Requirement,
                marked_by,
            }
        })
        .collect()
}

/// 块的 Agent 标记结论（无标记/越界 → `None`）——`get_block_requirements`
/// 的 verdict 面（覆盖区间内取该块命中的标记）。
pub fn block_verdict_of(
    marks: &[BlockMark],
    block_idx: usize,
    total_blocks: usize,
) -> Option<BlockVerdict> {
    if block_idx >= total_blocks {
        return None;
    }
    marks
        .iter()
        .find(|m| block_idx >= m.from && block_idx <= m.to)
        .map(|m| m.verdict)
}

/// 测试/工具面便利：块原文（越界 → `None`）。
pub fn block_of(pf: &ParsedFile, idx: usize) -> Option<&Block> {
    pf.blocks.get(idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, BlockKind};

    fn pf(blocks: &[(&str, &str)]) -> ParsedFile {
        ParsedFile {
            name: "A-主文件.pdf".into(),
            sha256: "sha-1".into(),
            pages: 2,
            tables: 0,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks: blocks
                .iter()
                .enumerate()
                .map(|(i, (loc, text))| Block {
                    idx: i,
                    kind: BlockKind::PdfLine,
                    locator: loc.to_string(),
                    text: text.to_string(),
                    cells: vec![],
                })
                .collect(),
            parse_summary: "pdf 文本层: 测试".into(),
            components: vec![],
        }
    }

    fn anchor_at(locator: &str, excerpt: &str, block_idx: Option<usize>) -> Anchor {
        Anchor {
            file: "A-主文件.pdf".into(),
            kind: "pdf_line".into(),
            locator: locator.into(),
            excerpt: excerpt.into(),
            block_idx,
            page: None,
            line: None,
        }
    }

    fn req(id: &str, anchors: Vec<Anchor>, marked_by: Option<&str>) -> Requirement {
        let mut params = std::collections::HashMap::new();
        if let Some(m) = marked_by {
            params.insert("marked_by".to_string(), serde_json::json!(m));
        }
        Requirement {
            id: id.into(),
            category: "其他".into(),
            clause_no: None,
            anchors,
            params,
            source_file: "A-主文件.pdf".into(),
        }
    }

    #[test]
    fn resolve_prefers_backfilled_block_idx_with_bounds_guard() {
        let p = pf(&[("第1页 行1", "a"), ("第1页 行2", "b")]);
        let a = anchor_at("第1页 行2", "b", Some(1));
        assert_eq!(resolve_anchor_block_idx(&p, &a), Some(1));
        // 越界（跨版本块数收缩的防御面）→ 不命中，不 panic
        let bad = anchor_at("第1页 行2", "b", Some(99));
        assert_eq!(resolve_anchor_block_idx(&p, &bad), None);
    }

    #[test]
    fn resolve_falls_back_to_locator_excerpt_for_legacy_anchor() {
        let p = pf(&[("第1页 行1", "a"), ("第1页 行2", "b")]);
        // 旧快照锚（block_idx = None）——精确匹配兜底
        let legacy = anchor_at("第1页 行2", "b", None);
        assert_eq!(resolve_anchor_block_idx(&p, &legacy), Some(1));
        // locator 对但 excerpt 不是块全文（部分列锚）→ None 诚实口径
        let partial = anchor_at("第1页 行2", "b（部分）", None);
        assert_eq!(resolve_anchor_block_idx(&p, &partial), None);
        // 完全不匹配 → None
        let alien = anchor_at("第9页 行9", "z", None);
        assert_eq!(resolve_anchor_block_idx(&p, &alien), None);
    }

    #[test]
    fn index_rows_only_for_req_bearing_blocks_in_document_order() {
        let p = pf(&[
            ("第1页 行1", "覆盖无提交"), // 0：无 REQ
            ("第1页 行2", "带宽要求"),   // 1：regex REQ
            ("第1页 行3", "服务地点"),   // 2：agent REQ（双锚跨块 2-3）
            ("第1页 行4", "验收要求"),   // 3：agent REQ 第二锚
        ]);
        let reqs = vec![
            req(
                "REQ-0001",
                vec![anchor_at("第1页 行2", "带宽要求", Some(1))],
                Some("regex"),
            ),
            req(
                "REQ-A0001",
                vec![
                    anchor_at("第1页 行3", "服务地点", Some(2)),
                    anchor_at("第1页 行4", "验收要求", Some(3)),
                ],
                Some("agent"),
            ),
        ];
        let idx = block_requirement_index(&p, &[], &reqs);
        assert_eq!(idx.len(), 3, "无 REQ 块（0）不出行");
        assert_eq!(idx[0].block_idx, 1);
        assert_eq!(idx[0].req_ids, vec!["REQ-0001"]);
        assert_eq!(idx[0].marked_by, MarkSource::RegexFallback);
        assert_eq!(idx[0].verdict, BlockVerdict::Requirement);
        assert_eq!(idx[1].block_idx, 2);
        assert_eq!(idx[1].req_ids, vec!["REQ-A0001"]);
        assert_eq!(idx[1].marked_by, MarkSource::Agent);
        assert_eq!(idx[2].block_idx, 3, "双锚 REQ 跨块入两行（文档序）");
        assert_eq!(idx[2].req_ids, vec!["REQ-A0001"]);
        assert!(idx.iter().all(|r| r.file_sha256 == "sha-1"));
    }

    #[test]
    fn index_skips_other_file_anchors_and_unresolvable() {
        let p = pf(&[("第1页 行1", "a"), ("第1页 行2", "b")]);
        let reqs = vec![
            // 他文件锚（多文件 REQ）——不进本文件索引
            req(
                "REQ-0009",
                vec![Anchor {
                    file: "B-附件.pdf".into(),
                    kind: "pdf_line".into(),
                    locator: "第1页 行1".into(),
                    excerpt: "a".into(),
                    block_idx: Some(0),
                    page: None,
                    line: None,
                }],
                Some("agent"),
            ),
            // 本文件但锚解析不到块（旧快照跨版本）——REQ 在库、索引不收
            req(
                "REQ-0010",
                vec![anchor_at("第7页 行1", "已改版原文", None)],
                Some("regex"),
            ),
        ];
        let idx = block_requirement_index(&p, &[], &reqs);
        assert!(idx.is_empty(), "他文件锚/不可解析锚都不产生行");
    }

    #[test]
    fn mark_source_defaults_to_regex_when_key_missing() {
        // 旧库 REQ（#250 之前无 marked_by 键）——按正则产物口径
        let r = req("REQ-0001", vec![], None);
        assert_eq!(mark_source_of(&r), MarkSource::RegexFallback);
    }

    #[test]
    fn block_verdict_of_reads_marks_with_bounds_guard() {
        use bid_domain::coverage::BlockMark;
        let marks = vec![
            BlockMark {
                from: 0,
                to: 0,
                verdict: BlockVerdict::Noise,
                reason: String::new(),
            },
            BlockMark {
                from: 2,
                to: 3,
                verdict: BlockVerdict::Requirement,
                reason: String::new(),
            },
        ];
        assert_eq!(block_verdict_of(&marks, 0, 4), Some(BlockVerdict::Noise));
        assert_eq!(block_verdict_of(&marks, 1, 4), None, "未标记块无结论");
        assert_eq!(
            block_verdict_of(&marks, 3, 4),
            Some(BlockVerdict::Requirement)
        );
        assert_eq!(block_verdict_of(&marks, 9, 4), None, "越界防御");
    }
}
