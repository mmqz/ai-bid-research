//! coverage_tools.rs —— #219 全块覆盖三工具 + #285 ★漏标检测。
//!
//! 架构转变：条款抽取从「正则猜边界」改为「引擎切块 + Agent 标区间」。
//!
//! | 工具 | 职责 |
//! |---|---|
//! | `get_document_chunks` | 取带 block_idx 的块（分页，防吃爆上下文） |
//! | `submit_block_marks` | 提交区间标记 + requirements（校验后落库） |
//! | `get_coverage_report` | 查覆盖（哪些块没归属） |
//! | `get_star_coverage` | ★漏标检测（#285——字面★真值 vs 已抽 REQ 差集） |
//!
//! **职责边界**：引擎只管块 ID 稳定性、区间合法性、覆盖统计、★字面对账；
//! 语义判断（哪些块是要求、★条款怎么标）归 Agent——本工具只报漏不补标。

use std::collections::HashMap;

use bid_domain::coverage::{BlockMark, BlockVerdict, SubmittedRequirement};
use serde_json::{json, Value};

use crate::error::{ToolError, ToolResult};
use crate::tools::{opt_u64, req_str, route_guard, route_project};
use bid_tools::service::BidService;

/// 单次返回块数上限（防上下文吃爆——160 页文件有 3600+ 块）。
const MAX_CHUNK_PAGE: u64 = 200;

/// 按 file_id 找解析产物（join 键 = sha256，与 #217 locate_content 同口径）。
/// #219 二步起 tools.rs 的 extract 合并面复用（pub(crate)）。
pub(crate) fn find_parsed<'a>(
    svc: &'a BidService,
    file_id: &str,
    at: &str,
) -> Result<&'a bid_parse::ParsedFile, ToolError> {
    let tf = svc
        .store()
        .files()
        .iter()
        .find(|f| f.id == file_id)
        .ok_or_else(|| ToolError::ENotFound {
            what: format!("{at}: 文件 {file_id} 不存在（先 import_tender_files）"),
        })?;
    svc.parsed()
        .iter()
        .find(|p| p.sha256 == tf.sha256)
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!(
                "{at}: 文件 {file_id} 无解析产物在服务态（须先经 import_tender_files 成功导入）"
            ),
        })
}

/// `get_document_chunks`：按文件取块的文本窗口。
///
/// 入参：`project_id`、`file_id`、可选 `from`（起始块）/`size`（窗口大小，默认 50，上限 200）。
/// 出参：块数组（block_idx + locator + text）+ 该文件覆盖状态摘要。
pub fn get_document_chunks(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "get_document_chunks";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let file_id = req_str(&args, "file_id", TOOL)?;
    let from = opt_u64(&args, "from", TOOL)?.unwrap_or(0) as usize;
    let size = opt_u64(&args, "size", TOOL)?.unwrap_or(50);
    if size == 0 || size > MAX_CHUNK_PAGE {
        return Err(ToolError::EUnprocessable {
            what: format!("{TOOL}: size 值域 1..={MAX_CHUNK_PAGE}（收到 {size}）——一次取全量会吃爆上下文，分页取"),
        });
    }

    route_project(svc, &project_id, TOOL)?;

    let file = find_parsed(svc, &file_id, TOOL)?;
    let total = file.blocks.len();
    let end = (from + size as usize).min(total);
    if from >= total && total > 0 {
        return Err(ToolError::EUnprocessable {
            what: format!("{TOOL}: from={from} 越界（文件共 {total} 块）"),
        });
    }

    let marks = svc.store().block_marks_of(&file_id);
    let covered: HashMap<usize, &str> = marks
        .iter()
        .flat_map(|m| (m.from..=m.to).map(move |i| (i, m.verdict.as_str())))
        .collect();

    let chunks: Vec<Value> = file.blocks[from..end]
        .iter()
        .map(|b| {
            json!({
                "block_idx": b.idx,
                "kind": format!("{:?}", b.kind),
                "locator": b.locator,
                "text": b.text,
                "cells": b.cells,
                "verdict": covered.get(&b.idx).copied(),
            })
        })
        .collect();

    let report = svc.store().coverage_report(&file_id, total);
    Ok(json!({
        "project_id": project_id,
        "file_id": file_id,
        "total_blocks": total,
        "from": from,
        "returned": chunks.len(),
        "next_from": if end < total { Some(end) } else { None },
        "chunks": chunks,
        "coverage": coverage_json(&report),
    }))
}

/// #395（#269 卡点 #3）：类别英文键 → 中文字面量（与 tools.rs `category_key`
/// 的映射互逆——响应面读英文、入参面写中文的双语词汇单向桥）。
/// 中文字面量原样通过；未知英文键原样返回（由域层 REQUIREMENT_CATEGORIES
/// 校验拒——fail-loud 不吞）。
fn normalize_category(raw: &str) -> String {
    match raw {
        "qualification" => "资格条件".to_string(),
        "veto" => "否决项".to_string(),
        "scoring" => "评分项".to_string(),
        "tech_param" => "技术参数".to_string(),
        "commercial" => "商务要求".to_string(),
        "format" => "格式要求".to_string(),
        "other" => "其他".to_string(),
        cn => cn.to_string(),
    }
}

fn parse_verdict(s: &str, at: &str) -> Result<BlockVerdict, ToolError> {
    match s {
        "requirement" => Ok(BlockVerdict::Requirement),
        "noise" => Ok(BlockVerdict::Noise),
        "rule" => Ok(BlockVerdict::Rule),
        "review_table" => Ok(BlockVerdict::ReviewTable),
        other => Err(ToolError::EUnprocessable {
            what: format!(
                "{at}: verdict 非法：{other}（合法值：requirement|noise|rule|review_table）"
            ),
        }),
    }
}

/// `submit_block_marks`：提交区间标记 + requirements。
///
/// 入参：
/// ```text
/// marks: [{from, to, verdict, reason}]
///   verdict: requirement | noise | rule | review_table
/// requirements: [{block_idx, statement, category?, mandatory?}]（可选）
/// ```
/// **校验**（不过即拒）：区间合法 + 不越界 + 不重叠 + requirement 落在 requirement 区间内。
pub fn submit_block_marks(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "submit_block_marks";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let file_id = req_str(&args, "file_id", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;

    let raw_marks = args
        .get("marks")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{TOOL}: marks 必填（非空数组）"),
        })?;
    if raw_marks.is_empty() {
        return Err(ToolError::EUnprocessable {
            what: format!("{TOOL}: marks 不得为空（空提交无意义——不标记就永远报未覆盖）"),
        });
    }

    let mut marks: Vec<BlockMark> = Vec::with_capacity(raw_marks.len());
    for m in raw_marks {
        let from = m.get("from").and_then(|v| v.as_u64()).ok_or_else(|| {
            ToolError::EUnprocessable { what: format!("{TOOL}: marks[].from 必填（u64）") }
        })? as usize;
        let to = m.get("to").and_then(|v| v.as_u64()).ok_or_else(|| {
            ToolError::EUnprocessable { what: format!("{TOOL}: marks[].to 必填（u64）") }
        })? as usize;
        let v = m.get("verdict").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::EUnprocessable { what: format!("{TOOL}: marks[].verdict 必填") }
        })?;
        marks.push(BlockMark {
            from,
            to,
            verdict: parse_verdict(v, TOOL)?,
            reason: m.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        });
    }

    let mut reqs: Vec<SubmittedRequirement> = Vec::new();
    if let Some(arr) = args.get("requirements").and_then(|v| v.as_array()) {
        for r in arr {
            let block_idx = r.get("block_idx").and_then(|v| v.as_u64()).ok_or_else(|| {
                ToolError::EUnprocessable { what: format!("{TOOL}: requirements[].block_idx 必填") }
            })? as usize;
            let statement = r.get("statement").and_then(|v| v.as_str()).ok_or_else(|| {
                ToolError::EUnprocessable { what: format!("{TOOL}: requirements[].statement 必填") }
            })?.to_string();
            reqs.push(SubmittedRequirement {
                block_idx,
                statement,
                // #395（#269 卡点 #3）：类别词汇双语兼容——响应面（extract/
                // matrix 的 by_category）用英文键，入参面此前只认中文字面量，
                // 同一概念「读时英文写时要中文」造成黑盒会话首标被拒
                // （commercial → 商务要求）。中文字面量原样通过（零位移）；
                // 英文键归一化为中文后落库。
                category: r.get("category").and_then(|v| v.as_str()).map(normalize_category),
                mandatory: r.get("mandatory").and_then(|v| v.as_bool()).unwrap_or(false),
                // #276：★实质性条款（缺省 false——未声明视为非★，与 mandatory 同宽）
                substantive: r.get("substantive").and_then(|v| v.as_bool()).unwrap_or(false),
            });
        }
    }

    route_guard(svc, &project_id, TOOL, project_version)?;

    let total_blocks = find_parsed(svc, &file_id, TOOL)?.blocks.len();

    // 区间校验（不合法直接拒——不做部分写入）
    bid_domain::coverage::validate_marks(&marks, total_blocks)
        .map_err(|e| ToolError::EUnprocessable { what: format!("{TOOL}: {}", e.to_message()) })?;
    bid_domain::coverage::validate_submitted(&marks, &reqs)
        .map_err(|e| ToolError::EUnprocessable { what: format!("{TOOL}: {}", e.to_message()) })?;

    let marks_n = marks.len();
    let reqs_n = reqs.len();
    // #219 二步：marks + submitted 原子落库（经服务层——持久化工作区
    // 写后自动落盘；PR #227 只存 marks 且直改 store 无 checkpoint）
    svc.set_block_coverage(&file_id, marks, reqs, total_blocks)
        .map_err(ToolError::from)?;

    let report = svc.store().coverage_report(&file_id, total_blocks);
    Ok(json!({
        "project_id": project_id,
        "file_id": file_id,
        "marks_accepted": marks_n,
        "requirements_accepted": reqs_n,
        "total_blocks": total_blocks,
        "coverage": coverage_json(&report),
        "next": if report.complete {
            "全覆盖——可以继续 build_outline"
        } else {
            "仍有未判定块——用 get_document_chunks 查看后补标（已开始标记的文件必须标满才能建纲，#244）"
        },
    }))
}

/// #244 覆盖面 JSON 组装（三工具同口径）。
///
/// `uncovered_ranges` 键名保持（对外契约兼容——docs/SDK 已引用）；
/// 新增 `excluded_blocks`（B 态已排除）/ `unjudged_blocks`（C 态未判定）
/// 分账字段（#244 三态语义）。
fn coverage_json(report: &bid_domain::coverage::CoverageReport) -> Value {
    json!({
        "covered_blocks": report.covered_blocks,
        "complete": report.complete,
        "uncovered_ranges": report.unjudged_ranges,
        "excluded_blocks": report.excluded_blocks,
        "unjudged_blocks": report.unjudged_blocks,
        "by_verdict": report.by_verdict,
    })
}

/// `get_coverage_report`：查覆盖状态（哪些块没归属）。
pub fn get_coverage_report(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "get_coverage_report";
    let project_id = req_str(&args, "project_id", TOOL)?;
    route_project(svc, &project_id, TOOL)?;

    let tender_files: Vec<(String, String)> = svc
        .store()
        .files()
        .iter()
        .map(|f| (f.id.clone(), f.sha256.clone()))
        .collect();
    let parsed_by_sha: HashMap<&str, &bid_parse::ParsedFile> =
        svc.parsed().iter().map(|p| (p.sha256.as_str(), p)).collect();
    let mut files: Vec<Value> = Vec::new();
    let mut total_blocks = 0usize;
    let mut total_covered = 0usize;
    let mut total_uncovered_ranges = 0usize;
    let mut total_unjudged_blocks = 0usize;

    for (fid, sha) in tender_files.iter() {
        let Some(f) = parsed_by_sha.get(sha.as_str()) else { continue };
        let n = f.blocks.len();
        let report = svc.store().coverage_report(fid, n);
        total_blocks += n;
        total_covered += report.covered_blocks;
        total_uncovered_ranges += report.unjudged_ranges.len();
        total_unjudged_blocks += report.unjudged_blocks;
        files.push(json!({
            "file_id": fid,
            "total_blocks": n,
            "covered_blocks": report.covered_blocks,
            "complete": report.complete,
            "uncovered_ranges": report.unjudged_ranges.iter().take(50).collect::<Vec<_>>(),
            "uncovered_range_count": report.unjudged_ranges.len(),
            "excluded_blocks": report.excluded_blocks,
            "unjudged_blocks": report.unjudged_blocks,
            "by_verdict": report.by_verdict,
        }));
    }

    let complete = total_blocks > 0 && total_covered == total_blocks;
    Ok(json!({
        "project_id": project_id,
        "complete": complete,
        "total_blocks": total_blocks,
        "covered_blocks": total_covered,
        "uncovered_range_count": total_uncovered_ranges,
        "unjudged_blocks": total_unjudged_blocks,
        "files": files,
        "next": if total_blocks == 0 {
            "还没导入文件——先 import_tender_files"
        } else if complete {
            "全覆盖——可以 build_outline"
        } else {
            "有未判定块——submit_block_marks 补标（已开始标记的文件必须标满才能建纲，#244；未标记过的文件走正则兜底不受限）"
        },
    }))
}

// ---------------------------------------------------------------- #285 ★漏标检测

/// 明细列表上限（nm-lib 实测 638 处漏标——全量回显会吃爆上下文；计数全量、明细截断）。
const MAX_STAR_DETAIL: usize = 100;

/// `get_star_coverage`：★漏标检测（只读——★字面真值 vs 已抽 REQ 的差集）。
///
/// 入参：`project_id`、`file_id`。
/// 出参（口径三级恒等式：star_total = declarations + star_truth =
/// declarations + captured + missing——不漏不重）：
///
/// ```text
/// star_total      ★字面总数（含样板句——与 fixtures manifest star_count 同口径）
/// declarations    样板句★数（「打★号条款为实质性条款」是声明不是条款——排除）
/// star_truth      真值★数
/// star_captured   已捕获（位置对应 REQ 在场——locator 腿 / ★前缀腿带页守卫）
/// star_missing    漏标（真值 - 已捕获）
/// captured[]      块级明细（req_id 优先取 star-marked REQ——「标没标」可见）
/// missing[]       块级明细（page/line/locator/text/reason——补标定位面）
/// warnings[]      表格行装配面提示 / 未抽取提示
/// ```
///
/// **只报不补**（#285 明确不做：不自动补标、不改抽取逻辑）——补标由
/// Agent/人决策（submit_block_marks substantive=true 或修抽取）。
pub fn get_star_coverage(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "get_star_coverage";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let file_id = req_str(&args, "file_id", TOOL)?;

    route_project(svc, &project_id, TOOL)?;

    let file = find_parsed(svc, &file_id, TOOL)?;
    let file_name = file.name.clone();
    let reqs = svc.store().requirements().to_vec();
    let reqs_of_file: Vec<bid_domain::Requirement> = reqs
        .into_iter()
        .filter(|r| {
            r.source_file == file_name
                || r.anchors.iter().any(|a| a.file == file_name)
        })
        .collect();
    let cov = bid_parse::star_scan::star_coverage(file, &reqs_of_file);

    let mut warnings: Vec<String> = Vec::new();
    if reqs_of_file.is_empty() {
        warnings.push(format!(
            "本文件无任何 REQ——若尚未 extract_requirements，先抽取再查（当前口径：全部 {} 处真值★计为漏标）",
            cov.truth_stars
        ));
    }
    let table_row_missing = cov
        .missing
        .iter()
        .filter(|m| m.reason.contains("表格行"))
        .count();
    if table_row_missing > 0 {
        warnings.push(format!(
            "★在表格行内（cells 非空）——{table_row_missing} 块可能因表格装配问题漏抽"
        ));
    }

    let captured_detail: Vec<Value> = cov
        .captured
        .iter()
        .take(MAX_STAR_DETAIL)
        .map(|c| {
            json!({
                "req_id": c.req_id,
                "star": c.star,
                "page": c.page,
                "line": c.line,
                "block_idx": c.block_idx,
                "stars": c.stars,
                "locator": c.locator,
                "text": c.text,
            })
        })
        .collect();
    let missing_detail: Vec<Value> = cov
        .missing
        .iter()
        .take(MAX_STAR_DETAIL)
        .map(|m| {
            json!({
                "page": m.page,
                "line": m.line,
                "block_idx": m.block_idx,
                "stars": m.stars,
                "locator": m.locator,
                "text": m.text,
                "reason": m.reason,
            })
        })
        .collect();

    Ok(json!({
        "project_id": project_id,
        "file_id": file_id,
        "file_name": file_name,
        "star_total": cov.star_total,
        "declarations": cov.declaration_stars,
        "declaration_samples": cov.declaration_samples,
        "star_truth": cov.truth_stars,
        "star_captured": cov.captured_stars,
        "star_missing": cov.missing_stars,
        "captured": captured_detail,
        "missing": missing_detail,
        "captured_truncated": cov.captured.len() > MAX_STAR_DETAIL,
        "missing_truncated": cov.missing.len() > MAX_STAR_DETAIL,
        "warnings": warnings,
        "next": if cov.missing_stars == 0 && cov.truth_stars > 0 {
            "★全覆盖——无漏标"
        } else if cov.truth_stars == 0 {
            "本文件无★实质性条款标记（字面无真值★）"
        } else {
            "有★漏标——用 missing[].block_idx 定位后 submit_block_marks 补标（substantive=true），重跑 extract_requirements 生效"
        },
    }))
}
#[cfg(test)]
mod tests {
    use super::*;

    /// #395（#269 卡点 #3）：类别英文键归一化——双语词汇单向桥（响应读英文、
    /// 入参写中文）。中文字面量零位移；未知键原样透传（域层 fail-loud 拒）。
    #[test]
    fn normalize_category_bridges_english_keys() {
        assert_eq!(normalize_category("commercial"), "商务要求");
        assert_eq!(normalize_category("qualification"), "资格条件");
        assert_eq!(normalize_category("veto"), "否决项");
        assert_eq!(normalize_category("scoring"), "评分项");
        assert_eq!(normalize_category("tech_param"), "技术参数");
        assert_eq!(normalize_category("format"), "格式要求");
        assert_eq!(normalize_category("other"), "其他");
        // 中文原样通过（零位移）
        assert_eq!(normalize_category("商务要求"), "商务要求");
        // 未知键透传——由 REQUIREMENT_CATEGORIES 校验拒（不吞错）
        assert_eq!(normalize_category("nonsense"), "nonsense");
    }
}
