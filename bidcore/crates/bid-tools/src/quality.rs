//! quality.rs —— #68 阶段 2：生成质量验证（覆盖度检查 + 报告落盘）。
//!
//! 分层：
//! - **单章覆盖度**（[`check_section`]）：已起草章节的自动检查——
//!   ① REQ 覆盖（章节响应面内每条要求是否被正文引用回应——issue §2.1）；
//!   ② EV 引用核对（援引的证据是否在矩阵绑定集内——幻觉引用单列）；
//!   ③ 字数带宽（`target_chars` ±20%——Lead 既有验收线口径）；
//!   ④ 无锚承诺（复用 draft::count_unanchored_claims）；
//! - **项目报告**（[`build_quality_report`]）：全叶章节逐章检查 + 汇总
//!   （FAILED / PENDING / 未覆盖 REQ 总数 / 带外章数 / needs_human 清单
//!   ——issue §2.2「最终报告：哪些章节需要人工处理」）；
//! - **落盘**（[`write_report`]）：`{workspace}/artifacts/quality-report.json`
//!   （serde JSON——验收线「报告落盘」）；失败章错误明细从 S-2 的
//!   `failed.json` 边车并入（assembler 在场时）。
//!
//! 纪律：只读诊断（D2 同语义——报告是导数不是源数据），不改稿不落库
//! 不推状态。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::DomainStore;
use serde::{Deserialize, Serialize};

use crate::assembly::ChapterAssembler;
use crate::draft::{count_unanchored_claims, scan_citations};

/// 单章质量面（报告的逐章条目）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionQuality {
    pub section_id: String,
    pub title: String,
    /// 域状态字面量（DRAFTED / FAILED / PENDING / …）。
    pub status: String,
    /// 正文长度（chars().count() 中文口径；未起草 = 0）。
    pub chars: usize,
    pub target_chars: u32,
    /// 字数带宽（±20%）：Some(true) 带内 / Some(false) 带外 / None 不评
    /// （未起草或 target=0——诚实面：无目标不判带）。
    pub within_band: Option<bool>,
    /// 正文已回应的 REQ。
    pub covered_reqs: Vec<String>,
    /// 响应面内**未被回应**的 REQ（issue §2.1 核心清单）。
    pub uncovered_reqs: Vec<String>,
    /// 章节绑定面外的 REQ（幽灵——抽取漂移，如实报不猜）。
    pub ghost_reqs: Vec<String>,
    /// 正文援引且在绑定集内的 EV。
    pub cited_evs: Vec<String>,
    /// 绑定集内未被援引的 EV（信息面——非全部证据都必须引用）。
    pub uncited_evs: Vec<String>,
    /// 正文引用了但不在已知集的编号（幻觉面——draft 时已剔除，>0 即异常）。
    pub unknown_citations: Vec<String>,
    pub unanchored_claims: usize,
    /// FAILED 章的错误明细（failed.json 边车在场时）。
    pub error: Option<String>,
    /// 需人工处理的判定（FAILED / 有未覆盖 REQ / 幻觉引用残留 / 带外字数）。
    pub needs_human: bool,
}

/// 项目级质量报告（artifacts/quality-report.json 的落盘形态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityReport {
    pub project_id: String,
    pub generated_at: u64,
    pub sections: Vec<SectionQuality>,
    pub summary: QualitySummary,
}

/// 汇总面。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualitySummary {
    pub total: usize,
    pub drafted: usize,
    pub failed: usize,
    pub pending: usize,
    /// 全项目未覆盖 REQ 计数（Σ 逐章 uncovered）。
    pub uncovered_req_total: usize,
    /// 字数带外章数。
    pub out_of_band: usize,
    pub unanchored_total: usize,
    /// 需人工处理清单（issue §2.2「最终报告」）。
    pub needs_human: Vec<String>,
}

// ---------------------------------------------------------------- 单章检查

/// 单章覆盖度检查（只读——store + 节点；failed 明细由调用方并入）。
pub fn check_section(store: &DomainStore, node: &OutlineNode) -> SectionQuality {
    let content = node.content.as_deref().unwrap_or("");
    let cited: Vec<String> = scan_citations(content);

    // ---- REQ 面：响应面（node.requirement_ids）逐条核对
    let known_req: HashSet<&str> =
        store.requirements().iter().map(|r| r.id.as_str()).collect();
    let mut covered_reqs = Vec::new();
    let mut uncovered_reqs = Vec::new();
    let mut ghost_reqs = Vec::new();
    let has_content = !content.trim().is_empty();
    for rid in &node.requirement_ids {
        if !known_req.contains(&rid[..]) {
            ghost_reqs.push(rid.clone()); // 抽取漂移——不是质量问题是数据面问题
            continue;
        }
        // 未起草章不算未覆盖（没有正文就没有「未回应」可言——pending 计数
        // 已单独承载「还没写到」，needs_human 不收批次进行中的噪声）
        if !has_content {
            continue;
        }
        if cited.iter().any(|c| c == rid) {
            covered_reqs.push(rid.clone());
        } else {
            uncovered_reqs.push(rid.clone());
        }
    }

    // ---- EV 面：绑定集（与 draft::bound_evidence_ids 同口径——本章节 REQ ↔ EV）
    let req_set: HashSet<&str> = node.requirement_ids.iter().map(|s| &s[..]).collect();
    let mut known_ev: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for b in store.evidence_bindings() {
        if req_set.contains(&b.requirement_id[..]) && seen.insert(b.evidence_id.clone()) {
            known_ev.push(b.evidence_id.clone());
        }
    }
    let known_all: HashSet<&str> = known_req.iter().copied()
        .chain(known_ev.iter().map(|s| &s[..])).collect();
    let (cited_evs, unknown_citations): (Vec<String>, Vec<String>) = {
        let mut evs = Vec::new();
        let mut unk = Vec::new();
        for c in &cited {
            if c.starts_with("EV-") {
                evs.push(c.clone());
            }
            if !known_all.contains(&c[..]) {
                unk.push(c.clone());
            }
        }
        (evs, unk)
    };
    let uncited_evs: Vec<String> = known_ev.iter()
        .filter(|e| !cited_evs.contains(e)).cloned().collect();

    // ---- 字数带宽（±20%——target=0 或未起草不评）
    let chars = content.chars().count();
    let within_band = if node.target_chars == 0 || content.trim().is_empty() {
        None
    } else {
        let t = f64::from(node.target_chars);
        Some((chars as f64) >= t * 0.80 && (chars as f64) <= t * 1.20)
    };

    let unanchored = if has_content { count_unanchored_claims(content) } else { 0 };
    let needs_human = node.status == SectionStatus::Failed
        || (has_content && (!uncovered_reqs.is_empty()
            || !unknown_citations.is_empty()
            || within_band == Some(false)));

    SectionQuality {
        section_id: node.id.clone(),
        title: node.title.clone(),
        status: node.status.as_str().to_string(),
        chars,
        target_chars: node.target_chars,
        within_band,
        covered_reqs,
        uncovered_reqs,
        ghost_reqs,
        cited_evs,
        uncited_evs,
        unknown_citations,
        unanchored_claims: unanchored,
        error: None, // 调用方（build_quality_report）从边车并入
        needs_human,
    }
}

// ---------------------------------------------------------------- 项目报告

/// 全项目质量报告（叶章节按文档序；assembler 在场时并入失败边车明细）。
pub fn build_quality_report(
    store: &DomainStore,
    asm: Option<&ChapterAssembler>,
) -> QualityReport {
    let failed_map = asm.map(|a| a.failed_records()).unwrap_or_default();
    let parents: HashSet<&str> =
        store.sections().iter().filter_map(|n| n.parent_id.as_deref()).collect();
    let mut leaves: Vec<&OutlineNode> = store.sections().iter()
        .filter(|n| !parents.contains(&n.id[..])).collect();
    leaves.sort_by_key(|n| n.order);

    let mut sections = Vec::with_capacity(leaves.len());
    let mut drafted = 0usize;
    let mut failed = 0usize;
    let mut pending = 0usize;
    let mut uncovered_req_total = 0usize;
    let mut out_of_band = 0usize;
    let mut unanchored_total = 0usize;
    let mut needs_human = Vec::new();
    let total = leaves.len();
    for node in &leaves {
        let mut q = check_section(store, node);
        if let Some(rec) = failed_map.get(&node.id) {
            q.error = Some(rec.error.clone());
        }
        match node.status {
            SectionStatus::Drafted | SectionStatus::Validating
            | SectionStatus::Revising | SectionStatus::Accepted => drafted += 1,
            SectionStatus::Failed => failed += 1,
            _ => pending += 1,
        }
        uncovered_req_total += q.uncovered_reqs.len();
        if q.within_band == Some(false) {
            out_of_band += 1;
        }
        unanchored_total += q.unanchored_claims;
        if q.needs_human {
            needs_human.push(node.id.clone());
        }
        sections.push(q);
    }
    QualityReport {
        project_id: store.project().map(|p| p.id.clone()).unwrap_or_default(),
        generated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        sections,
        summary: QualitySummary {
            total,
            drafted,
            failed,
            pending,
            uncovered_req_total,
            out_of_band,
            unanchored_total,
            needs_human,
        },
    }
}

/// 报告落盘：`{ws}/artifacts/quality-report.json`（tmp + rename 原子写）。
///
/// 返回落盘路径（调用方进 MCP out 的 artifact 面）。
pub fn write_report(report: &QualityReport, ws: &Path) -> Result<PathBuf, String> {
    let dir = ws.join("artifacts");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建 artifacts 目录失败: {e}"))?;
    let path = dir.join("quality-report.json");
    let tmp = dir.join("quality-report.json.tmp");
    let body = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).map_err(|e| format!("写报告失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("报告原子替换失败: {e}"))?;
    Ok(path)
}
