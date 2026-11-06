//! bidi_tools.rs —— #285 ② 块级双向索引三工具（只读）。
//!
//! | 工具 | 方向 | 职责 |
//! |---|---|---|
//! | `get_requirement_provenance` | REQ → 原文 | 单条 REQ 的溯源：锚点带 block_idx/page/line（块位真源 = Anchor 回填值；旧快照锚按 (locator, excerpt) 兜底解析——`bid_parse::bidi::resolve_anchor_block_idx`） |
//! | `get_block_requirements` | 块 → REQ | 单块反查：原文 + verdict（Agent 标记面，无标记 = null 诚实口径）+ req_ids + marked_by |
//! | `get_annotation_overlay` | 文件 → 视图层 | 全块 overlay 数据面：req_ids/semantic/status/suspected_miss（渲染上色由视图层决定——本工具只给数据，不做 UI） |
//!
//! **数据纪律**：反查索引是派生态（`bid_parse::bidi::block_requirement_index`
//! 每次现算，不持久化）；suspected_miss/emphasis 复用 #285 ① 的
//! `star_coverage`（同源双腿匹配——两工具口径恒一致）。

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::coverage_tools::find_parsed;
use crate::error::{ToolError, ToolResult};
use crate::tools::{req_str, route_project};
use bid_tools::service::BidService;

/// overlay 明细上限（与 get_star_coverage 的 MAX_STAR_DETAIL 同纪律——
/// nm-lib 级大文件全量回显吃爆上下文；计数全量、明细截断）。
const MAX_OVERLAY_DETAIL: usize = 200;

/// `get_requirement_provenance`：单条 REQ 的溯源面（REQ → 原文）。
///
/// 入参：`project_id`、`req_id`。
/// 出参锚点逐个带 `block_idx` / `page` / `line`——块位三级真源：
/// ① `Anchor.block_idx`（#285 ② 起抽取时回填）；
/// ② 旧快照锚按 `(locator, excerpt)` 精确匹配兜底（`resolved_via` 标记
///    腿来源——`backfilled` = 回填值 / `fallback_match` = 兜底命中 /
///    `null` = 解析不到（跨版本锚失效，诚实可见））；
/// ③ page/line 从 locator 解析（PDF 面；DOCX 章节路径面为 null）。
///
/// **geometric（quads 页面坐标）不产出**——#285 ② 明确不做 mdcore 依赖，
/// 几何投影由 mdcore #239 的 overlay-pdf 承担（本工具给 block_idx，
/// 视图层拿它去 mdcore 侧查几何）。
pub fn get_requirement_provenance(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "get_requirement_provenance";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let req_id = req_str(&args, "req_id", TOOL)?;
    route_project(svc, &project_id, TOOL)?;

    let req = svc
        .store()
        .requirements()
        .iter()
        .find(|r| r.id == req_id)
        .cloned()
        .ok_or_else(|| ToolError::ENotFound {
            what: format!("{TOOL}: REQ {req_id} 不存在（先 extract_requirements）"),
        })?;

    // 锚点文件 → 解析产物索引（多文件多锚形态——GT-07 双锚点）
    let parsed_by_name: BTreeMap<String, &bid_parse::ParsedFile> = svc
        .parsed()
        .iter()
        .map(|p| (p.name.clone(), p))
        .collect();

    let anchors: Vec<Value> = req
        .anchors
        .iter()
        .map(|a| {
            // 块位三级真源：回填值 → (locator, excerpt) 兜底 → null（诚实）
            let (block_idx, resolved_via) = match parsed_by_name.get(&a.file) {
                Some(pf) => match a.block_idx {
                    Some(bi) => (Some(bi), "backfilled"),
                    None => match bid_parse::bidi::resolve_anchor_block_idx(pf, a) {
                        Some(bi) => (Some(bi), "fallback_match"),
                        None => (None, "unresolved"),
                    },
                },
                None => (a.block_idx, "file_not_parsed"),
            };
            json!({
                "file": a.file,
                "kind": a.kind,
                "locator": a.locator,
                "excerpt": a.excerpt,
                "block_idx": block_idx,
                "page": a.page,
                "line": a.line,
                "resolved_via": resolved_via,
            })
        })
        .collect();

    let substantive = req
        .params
        .get("star")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            req.params
                .get("substantive")
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false);
    let marked_by = bid_parse::bidi::mark_source_of(&req);

    Ok(json!({
        "project_id": project_id,
        "req_id": req.id,
        "category": req.category,
        "clause_no": req.clause_no,
        "substantive": substantive,
        "statement": req.params.get("statement").cloned().unwrap_or(Value::Null),
        "marked_by": marked_by.as_str(),
        "source_file": req.source_file,
        "anchors": anchors,
        "next": if anchors.is_empty() {
            "本 REQ 无锚点（合成/人工件）"
        } else {
            "锚点带 block_idx——get_block_requirements(block_idx) 反查原文上下文"
        },
    }))
}

/// `get_block_requirements`：单块反查（块 → REQ）。
///
/// 入参：`project_id`、`file_id`、`block_idx`。
/// 出参：块原文 + verdict（Agent 标记面；未标记 = null——无结论不是
/// 「noise」，是「还没看过」）+ req_ids（派生索引聚合）+ marked_by。
/// 无 REQ 块照常返回（req_ids: []——「这段原文没被抽成任何要求」可见）。
pub fn get_block_requirements(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "get_block_requirements";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let file_id = req_str(&args, "file_id", TOOL)?;
    let block_idx = crate::tools::opt_u64(&args, "block_idx", TOOL)?
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{TOOL}: block_idx 必填（0 基块序——get_document_chunks 的 chunks[].block_idx）"),
        })? as usize;
    route_project(svc, &project_id, TOOL)?;

    let pf = find_parsed(svc, &file_id, TOOL)?;
    let block = pf.blocks.get(block_idx).ok_or_else(|| ToolError::ENotFound {
        what: format!(
            "{TOOL}: block_idx {block_idx} 越界（本文件共 {} 块）",
            pf.blocks.len()
        ),
    })?;

    // 派生索引（现算——REQ→块聚合的反向）
    let reqs: Vec<bid_domain::Requirement> =
        svc.store().requirements().to_vec();
    let marks = svc.store().block_marks_of(&file_id).to_vec();
    let idx = bid_parse::bidi::block_requirement_index(pf, &marks, &reqs);
    let row = idx.iter().find(|r| r.block_idx == block_idx);

    let verdict_from_mark =
        bid_parse::bidi::block_verdict_of(&marks, block_idx, pf.blocks.len());
    let (page, line) = bid_parse::page_line_of(&block.locator);

    Ok(json!({
        "project_id": project_id,
        "file_id": file_id,
        "file_name": pf.name,
        "block_idx": block_idx,
        "kind": format!("{:?}", block.kind).to_lowercase(),
        "locator": block.locator,
        "page": page,
        "line": line,
        "text": block.text,
        "in_table": !block.cells.is_empty(),
        "verdict": row.map(|r| r.verdict.as_str()).or_else(|| verdict_from_mark.as_ref().map(|v| v.as_str())),
        "req_ids": row.map(|r| r.req_ids.clone()).unwrap_or_default(),
        "marked_by": row.map(|r| r.marked_by.as_str()),
        "next": if row.is_some() {
            "本块已抽成 REQ——get_requirement_provenance(req_id) 查溯源"
        } else {
            "本块无 REQ——若漏抽，submit_block_marks 补标或修抽取"
        },
    }))
}

/// `get_annotation_overlay`：全块 overlay 数据面（文件 → 视图层渲染输入）。
///
/// 入参：`project_id`、`file_id`、可选 `page`（过滤单页）。
/// 出参 overlay[] 逐块：`req_ids` / `semantic` / `status` / `suspected_miss`。
///
/// - `semantic`（渲染语义——对齐 mdcore OverlayMark 词汇，#239 已定义
///   Emphasis/Comment/Freehand/Region；标书面只产出有数据源的两值）：
///   `"emphasis"` = ★真值块（招标人明示强调——废标线信号）；
///   `"requirement"` = 有 REQ 锚定（Agent/正则抽取的要求）；
///   其余 = null（无信号——视图层不上色）。
/// - `status`：`"valid"`（当前抽取快照内——锚失效检测等文档重导面，
///   先恒 valid，诚实注明）。
/// - `suspected_miss`：#285 ① `star_coverage` 的 missing 块位（同源直接
///   复用——双腿匹配口径，非块位恒等；字符计数面见 `get_star_coverage`）。
pub fn get_annotation_overlay(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "get_annotation_overlay";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let file_id = req_str(&args, "file_id", TOOL)?;
    let page_filter = crate::tools::opt_u64(&args, "page", TOOL)?;
    route_project(svc, &project_id, TOOL)?;

    let pf = find_parsed(svc, &file_id, TOOL)?;
    let reqs: Vec<bid_domain::Requirement> = svc.store().requirements().to_vec();
    let marks = svc.store().block_marks_of(&file_id).to_vec();
    let idx = bid_parse::bidi::block_requirement_index(pf, &marks, &reqs);
    let idx_by_block: BTreeMap<usize, &bid_domain::coverage::BlockRequirementIndex> =
        idx.iter().map(|r| (r.block_idx, r)).collect();

    // ★面（#285 ① star_scan 同源——**直接复用 `star_coverage` 的双腿匹配**
    // （locator 腿 + ★内容前缀腿带页守卫），不得另起块位恒等判定：续行吸收
    // 形态（docx 多块合并锚）前缀腿命中而块位不在锚集——两口径必须一致）。
    // emphasis = 真值块（captured ∪ missing）；suspected_miss = missing 块。
    let file_name = pf.name.clone();
    let reqs_of_file: Vec<bid_domain::Requirement> = reqs
        .iter()
        .filter(|r| {
            r.source_file == file_name || r.anchors.iter().any(|a| a.file == file_name)
        })
        .cloned()
        .collect();
    let cov = bid_parse::star_scan::star_coverage(pf, &reqs_of_file);
    let star_truth: std::collections::BTreeSet<usize> = cov
        .captured
        .iter()
        .map(|c| c.block_idx)
        .chain(cov.missing.iter().map(|m| m.block_idx))
        .collect();
    let star_missing_blocks: std::collections::BTreeSet<usize> =
        cov.missing.iter().map(|m| m.block_idx).collect();

    let mut overlay: Vec<Value> = Vec::new();
    let mut truncated = false;
    let mut req_block_count = 0usize;
    let mut suspected_miss_count = 0usize;
    for b in &pf.blocks {
        let (page, line) = bid_parse::page_line_of(&b.locator);
        if let Some(p) = page_filter {
            if page != Some(p as u32) {
                continue;
            }
        }
        let row = idx_by_block.get(&b.idx);
        let req_ids: Vec<&str> = row
            .map(|r| r.req_ids.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        let has_req = !req_ids.is_empty();
        if has_req {
            req_block_count += 1;
        }
        let is_star_truth = star_truth.contains(&b.idx);
        let suspected_miss = star_missing_blocks.contains(&b.idx);
        if suspected_miss {
            suspected_miss_count += 1;
        }
        if overlay.len() >= MAX_OVERLAY_DETAIL {
            truncated = true;
            continue;
        }
        overlay.push(json!({
            "block_idx": b.idx,
            "page": page,
            "line": line,
            "locator": b.locator,
            "text": b.text,
            "req_ids": req_ids,
            // 语义优先级：★强调 > 有 REQ（同块两者兼有时 emphasis 优先——
            // 招标人的强调信号高于我方抽取信号）
            "semantic": if is_star_truth {
                json!("emphasis")
            } else if has_req {
                json!("requirement")
            } else {
                Value::Null
            },
            "status": "valid",
            "suspected_miss": suspected_miss,
        }));
    }

    let mut warnings: Vec<String> = Vec::new();
    if truncated {
        warnings.push(format!(
            "明细超 {} 块截断（计数全量）——按 page 参数分页取",
            MAX_OVERLAY_DETAIL
        ));
    }
    if suspected_miss_count > 0 {
        warnings.push(format!(
            "{suspected_miss_count} 块★真值无对应 REQ（suspected_miss）——get_star_coverage 查漏标明细后补标（#285 ① 同源数据）"
        ));
    }

    Ok(json!({
        "project_id": project_id,
        "file_id": file_id,
        "file_name": pf.name,
        "total_blocks": pf.blocks.len(),
        "req_bearing_blocks": req_block_count,
        "suspected_miss_blocks": suspected_miss_count,
        "truncated": truncated,
        "overlay": overlay,
        "warnings": warnings,
        "next": if req_block_count == 0 {
            "本文件无 REQ 锚定块——先 extract_requirements"
        } else {
            "视图层按 semantic 上色；suspected_miss 块先补标"
        },
    }))
}

