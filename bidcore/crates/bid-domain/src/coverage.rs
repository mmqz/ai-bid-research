//! coverage.rs —— 全块覆盖模型（issue #219）。
//!
//! 背景（实测）：条款抽取依赖正则匹配字符形态（`1.1` / `一、` / `第X章`），
//! 真实招标文件里：
//!   · 技术要求用「1、」编号 → 抽不到（已由 #222 部分修复）
//!   · 大量「投标人须知」流程条款被误判为必选 → 卡死导出（#226）
//!   · 无编号段落、表格内容 → 完全丢失
//!
//! 架构转变：
//! ```text
//! 现状：文档 → 正则猜边界 → 匹配不上的静默丢弃
//! 目标：文档 → 切成有序块 → Agent 标记区间 → 引擎做覆盖检查
//! ```
//!
//! **职责切分（本模块的边界）**：
//! - 引擎：块 ID（稳定）、区间校验（不重叠/全覆盖）、覆盖统计
//! - Agent：理解语义、判断哪些块是要求、提取关键点
//!
//! 引擎不判断「什么算条款」——那是语义判断，正则追不上。

use serde::{Deserialize, Serialize};

/// 块标记结论（Agent 判断面的值域）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockVerdict {
    /// 需要响应：本块含我方必须回答的要求。
    Requirement,
    /// 噪声：与响应无关（法律依据/术语定义/目录/页码等）。
    Noise,
    /// 规则说明：招标人声明的规则（我方遵守即可，不写响应正文）。
    /// **#226 的核心**——这类常被误判为否决项导致死锁。
    Rule,
    /// 审查表：资格性/符合性/详细评审表（由 review_table 或 Agent 结构化提取）。
    ReviewTable,
}

impl BlockVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlockVerdict::Requirement => "requirement",
            BlockVerdict::Noise => "noise",
            BlockVerdict::Rule => "rule",
            BlockVerdict::ReviewTable => "review_table",
        }
    }

    /// 是否产生「需响应条款」。
    pub fn produces_requirement(&self) -> bool {
        matches!(self, BlockVerdict::Requirement)
    }
}

/// 区间标记：Agent 对一段连续块给出结论。
///
/// **为什么是区间而非逐块**：一份 160 页招标文件有 3600+ 块，前 100 页是
/// 「投标人须知」流程内容——逐块标记成本极高。区间标记既保证「每块有归属」
/// 又不要求逐块判断。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockMark {
    /// 起始块（含）。
    pub from: usize,
    /// 结束块（含）。
    pub to: usize,
    pub verdict: BlockVerdict,
    /// 判断理由（人可读——审计面）。
    #[serde(default)]
    pub reason: String,
}

impl BlockMark {
    /// 区间是否合法（from <= to）。
    pub fn is_valid_range(&self) -> bool {
        self.from <= self.to
    }

    /// 区间是否覆盖某块。
    pub fn covers(&self, idx: usize) -> bool {
        idx >= self.from && idx <= self.to
    }
}

/// Agent 提交的requirement（从某个块提取的关键点）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmittedRequirement {
    /// 来源块（必须落在 verdict=requirement 的区间内）。
    pub block_idx: usize,
    /// 关键点陈述（不得为空）。
    pub statement: String,
    /// 分类（Agent 判断——引擎只校验合法性）。
    #[serde(default)]
    pub category: Option<String>,
    /// 是否必选（漏响应会废标）。
    #[serde(default)]
    pub mandatory: bool,
}

/// 覆盖检查结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageReport {
    /// 总块数。
    pub total_blocks: usize,
    /// 已被区间覆盖的块数。
    pub covered_blocks: usize,
    /// 未覆盖的块区间（`[(from, to), ...]`，连续块合并）。
    pub uncovered: Vec<(usize, usize)>,
    /// 是否全覆盖。
    pub complete: bool,
    /// 各类结论的块数统计。
    pub by_verdict: std::collections::BTreeMap<String, usize>,
}

/// 区间校验错误。
#[derive(Debug, Clone, PartialEq)]
pub enum CoverageError {
    /// 区间起点晚于终点。
    InvalidRange { from: usize, to: usize },
    /// 区间越出块范围。
    OutOfBounds { from: usize, to: usize, total: usize },
    /// 区间重叠。
    Overlap { a: (usize, usize), b: (usize, usize) },
    /// requirement 的 block_idx 不在任何 requirement 区间内。
    RequirementOutsideRequirementRange { block_idx: usize },
    /// statement 为空。
    EmptyStatement { block_idx: usize },
}

impl CoverageError {
    pub fn to_message(&self) -> String {
        match self {
            CoverageError::InvalidRange { from, to } => format!(
                "区间起点晚于终点：[{from}, {to}]"
            ),
            CoverageError::OutOfBounds { from, to, total } => format!(
                "区间越界：[{from}, {to}]，文件共 {total} 块"
            ),
            CoverageError::Overlap { a, b } => format!(
                "区间重叠：[{}, {}] 与 [{}, {}]——同一块只能有一个结论",
                a.0, a.1, b.0, b.1
            ),
            CoverageError::RequirementOutsideRequirementRange { block_idx } => format!(
                "块 {block_idx} 提交了 requirement，但该块不在任何 verdict=requirement 的区间内——先标记区间再提取"
            ),
            CoverageError::EmptyStatement { block_idx } => format!(
                "块 {block_idx} 的 statement 为空——空关键点不可审计"
            ),
        }
    }
}

/// 校验一组标记（纯函数——零 IO）。
pub fn validate_marks(marks: &[BlockMark], total_blocks: usize) -> Result<(), CoverageError> {
    for m in marks {
        if !m.is_valid_range() {
            return Err(CoverageError::InvalidRange { from: m.from, to: m.to });
        }
        if total_blocks > 0 && m.to >= total_blocks {
            return Err(CoverageError::OutOfBounds {
                from: m.from, to: m.to, total: total_blocks,
            });
        }
    }
    // 重叠检测（按起点排序后线性扫描）
    let mut sorted: Vec<&BlockMark> = marks.iter().collect();
    sorted.sort_by_key(|m| m.from);
    for w in sorted.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b.from <= a.to {
            return Err(CoverageError::Overlap {
                a: (a.from, a.to), b: (b.from, b.to),
            });
        }
    }
    Ok(())
}

/// 校验提交的 requirement（必须落在 requirement 区间内 + statement 非空）。
pub fn validate_submitted(
    marks: &[BlockMark],
    reqs: &[SubmittedRequirement],
) -> Result<(), CoverageError> {
    for r in reqs {
        if r.statement.trim().is_empty() {
            return Err(CoverageError::EmptyStatement { block_idx: r.block_idx });
        }
        let in_req_range = marks
            .iter()
            .any(|m| m.verdict.produces_requirement() && m.covers(r.block_idx));
        if !in_req_range {
            return Err(CoverageError::RequirementOutsideRequirementRange {
                block_idx: r.block_idx,
            });
        }
    }
    Ok(())
}

/// 计算覆盖（纯函数）。
pub fn compute_coverage(marks: &[BlockMark], total_blocks: usize) -> CoverageReport {
    let mut covered = vec![false; total_blocks];
    let mut by_verdict: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for m in marks {
        let v = m.verdict.as_str().to_string();
        for idx in m.from..=m.to.min(total_blocks.saturating_sub(1)) {
            if idx < covered.len() && !covered[idx] {
                covered[idx] = true;
                *by_verdict.entry(v.clone()).or_insert(0) += 1;
            }
        }
    }
    // 连续未覆盖块合并成区间
    let mut uncovered: Vec<(usize, usize)> = Vec::new();
    let mut run_start: Option<usize> = None;
    for (idx, &c) in covered.iter().enumerate() {
        match (c, run_start) {
            (false, None) => run_start = Some(idx),
            (true, Some(s)) => {
                uncovered.push((s, idx - 1));
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = run_start {
        uncovered.push((s, total_blocks.saturating_sub(1)));
    }
    let covered_blocks = covered.iter().filter(|&&c| c).count();
    CoverageReport {
        total_blocks,
        covered_blocks,
        complete: covered_blocks == total_blocks,
        uncovered,
        by_verdict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(from: usize, to: usize, v: BlockVerdict) -> BlockMark {
        BlockMark { from, to, verdict: v, reason: String::new() }
    }

    #[test]
    fn invalid_range_rejected() {
        let m = mark(10, 5, BlockVerdict::Noise);
        assert!(matches!(
            validate_marks(&[m], 100),
            Err(CoverageError::InvalidRange { .. })
        ));
    }

    #[test]
    fn out_of_bounds_rejected() {
        let m = mark(0, 200, BlockVerdict::Noise);
        assert!(matches!(
            validate_marks(&[m], 100),
            Err(CoverageError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn overlap_rejected() {
        let marks = vec![
            mark(0, 50, BlockVerdict::Noise),
            mark(40, 80, BlockVerdict::Requirement),
        ];
        assert!(matches!(
            validate_marks(&marks, 100),
            Err(CoverageError::Overlap { .. })
        ));
    }

    #[test]
    fn adjacent_ranges_ok() {
        let marks = vec![
            mark(0, 49, BlockVerdict::Noise),
            mark(50, 99, BlockVerdict::Requirement),
        ];
        assert!(validate_marks(&marks, 100).is_ok());
    }

    #[test]
    fn full_coverage_detected() {
        let marks = vec![
            mark(0, 49, BlockVerdict::Noise),
            mark(50, 99, BlockVerdict::Requirement),
        ];
        let r = compute_coverage(&marks, 100);
        assert!(r.complete);
        assert_eq!(r.covered_blocks, 100);
        assert!(r.uncovered.is_empty());
        assert_eq!(r.by_verdict.get("noise"), Some(&50));
        assert_eq!(r.by_verdict.get("requirement"), Some(&50));
    }

    #[test]
    fn partial_coverage_reports_gap() {
        let marks = vec![
            mark(0, 29, BlockVerdict::Noise),
            mark(50, 99, BlockVerdict::Requirement),
        ];
        let r = compute_coverage(&marks, 100);
        assert!(!r.complete);
        assert_eq!(r.covered_blocks, 80);
        assert_eq!(r.uncovered, vec![(30, 49)]);
    }

    #[test]
    fn requirement_outside_requirement_range_rejected() {
        let marks = vec![
            mark(0, 49, BlockVerdict::Noise),
            mark(50, 99, BlockVerdict::Requirement),
        ];
        let reqs = vec![SubmittedRequirement {
            block_idx: 10,  // 在 noise 区间
            statement: "某要求".into(),
            category: None,
            mandatory: false,
        }];
        assert!(matches!(
            validate_submitted(&marks, &reqs),
            Err(CoverageError::RequirementOutsideRequirementRange { .. })
        ));
    }

    #[test]
    fn empty_statement_rejected() {
        let marks = vec![mark(0, 99, BlockVerdict::Requirement)];
        let reqs = vec![SubmittedRequirement {
            block_idx: 10,
            statement: "   ".into(),
            category: None,
            mandatory: false,
        }];
        assert!(matches!(
            validate_submitted(&marks, &reqs),
            Err(CoverageError::EmptyStatement { .. })
        ));
    }

    #[test]
    fn rule_verdict_does_not_produce_requirement() {
        let marks = vec![mark(0, 99, BlockVerdict::Rule)];
        let reqs = vec![SubmittedRequirement {
            block_idx: 10,
            statement: "某要求".into(),
            category: None,
            mandatory: false,
        }];
        // rule 区间不允许挂 requirement——规则说明不需要响应
        assert!(validate_submitted(&marks, &reqs).is_err());
    }
}