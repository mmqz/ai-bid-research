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
    /// ★实质性条款（#276——招标文件★号/「实质性条款」样板句声明，一条
    /// 不响应即响应无效）。语义正交于 `mandatory`：mandatory 是响应完备性
    /// 口径（必选），substantive 是招标人明示的**废标线**——★必非必选
    /// （如★技术参数），必选也未必★。落库 `params.star`（与 #280 抽取面
    /// 及 songling 金标准 #251 `star` 字段同键口径）。
    #[serde(default)]
    pub substantive: bool,
}

/// 覆盖检查结果。
///
/// #244 三态语义（建纲前闭环处置）：每个块必居其一——
/// - **A 已判定为要求**：`requirement` / `review_table` 标记（产 REQ 或结构化提取）；
/// - **B 已排除**：`noise` / `rule` 标记（判定理由在 `BlockMark.reason`，可追溯）；
/// - **C 未判定**：压根没标记——遗漏待审，建纲门（#244）阻断面。
///
/// 旧字段 `uncovered` 只是一个区间列表，分不清「排除了」还是「没看」——
/// 半途而废的标记集与全排除的标记集在旧报告里长得一样，门无法裁决。
/// 细分后：`excluded_blocks`（B 态计数）与 `unjudged_blocks`（C 态计数）
/// 分账，建纲门只对 C 态非零阻断。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageReport {
    /// 总块数。
    pub total_blocks: usize,
    /// 已被区间覆盖的块数（A + B 态）。
    pub covered_blocks: usize,
    /// B 态：明确判为 noise/rule 的块数（已排除——排除理由见 marks.reason）。
    /// `review_table` 不算排除：它是结构化提取的载体（A 态变体），
    /// 审查表会产出 REQ（否决项/评分项等）。
    pub excluded_blocks: usize,
    /// C 态：压根没标记的块数（遗漏待审——建纲门阻断面）。
    pub unjudged_blocks: usize,
    /// C 态区间（`[(from, to), ...]`，连续块合并）。原 `uncovered` 改名
    /// （#244 三态语义——「未覆盖」说不清是排除了还是没看）。
    pub unjudged_ranges: Vec<(usize, usize)>,
    /// 是否全覆盖（无 C 态块）。
    pub complete: bool,
    /// 各类结论的块数统计。
    pub by_verdict: std::collections::BTreeMap<String, usize>,
}

impl CoverageReport {
    /// #244 门禁判据：该文件是否「已启用标记流程但未判完」。
    ///
    /// - `marks` 空（零标记）→ 正则兜底路径，不是本门的对象（返回 false
    ///   ——由调用方另行判定，见 tools.rs build_outline #244 门）；
    /// - 本方法只回答「标记集非空时是否存在 C 态块」。
    #[must_use]
    pub fn has_unjudged(&self) -> bool {
        self.unjudged_blocks > 0
    }
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
    /// category 不在合法值域（#219 二步：Agent 提交面 fail-loud）。
    InvalidCategory { block_idx: usize, category: String },
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
            CoverageError::InvalidCategory { block_idx, category } => format!(
                "块 {block_idx} 的 category 非法：{category}（合法值：{}）",
                REQUIREMENT_CATEGORIES.join("|")
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

/// 合法类别值域（与 `Requirement.category` / 矩阵 blocking 面对齐——
/// bid-mcp schema.rs `requirements[].category` 的 enum 同源）。
/// #219 二步：Agent 提交面 fail-loud 校验的单一真源。
pub const REQUIREMENT_CATEGORIES: [&str; 7] = [
    "资格条件", "否决项", "评分项", "技术参数", "商务要求", "格式要求", "其他",
];

/// 校验提交的 requirement（必须落在 requirement 区间内 + statement 非空
/// + category 在合法值域）。
pub fn validate_submitted(
    marks: &[BlockMark],
    reqs: &[SubmittedRequirement],
) -> Result<(), CoverageError> {
    for r in reqs {
        if r.statement.trim().is_empty() {
            return Err(CoverageError::EmptyStatement { block_idx: r.block_idx });
        }
        if let Some(c) = &r.category {
            if !REQUIREMENT_CATEGORIES.contains(&c.as_str()) {
                return Err(CoverageError::InvalidCategory {
                    block_idx: r.block_idx,
                    category: c.clone(),
                });
            }
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
    // #244 三态分账：B 态（排除）= noise + rule 的块计数（review_table
    // 是结构化提取载体，不算排除）；C 态（未判定）= 总数 - 已覆盖。
    let excluded_blocks = *by_verdict.get("noise").unwrap_or(&0)
        + *by_verdict.get("rule").unwrap_or(&0);
    let unjudged_blocks = total_blocks.saturating_sub(covered_blocks);
    CoverageReport {
        total_blocks,
        covered_blocks,
        excluded_blocks,
        unjudged_blocks,
        unjudged_ranges: uncovered,
        complete: covered_blocks == total_blocks,
        by_verdict,
    }
}

// ---------------------------------------------------------------- #285 ② 块级双向索引

/// REQ 的块级标记来源（`Requirement.params.marked_by` 的类型化镜像）。
///
/// 与抽取侧落库键同口径（extract.rs `marked_by: "regex"` / merge.rs
/// `"agent"`）——派生面统一按枚举判，不再裸字符串比对。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkSource {
    /// Agent 提交（`submit_block_marks` requirements → REQ-Axxxx）。
    Agent,
    /// 正则兜底抽取（引擎抽取面 → REQ-xxxx）。
    #[serde(rename = "regex")]
    RegexFallback,
}

impl MarkSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            MarkSource::Agent => "agent",
            MarkSource::RegexFallback => "regex",
        }
    }
}

/// 块级双向索引行（**派生态**——build 时从 REQ anchors + BlockMarks 聚合，
/// 不持久化、可随时重建；`get_block_requirements` 反查的数据面）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockRequirementIndex {
    /// 文件内容幂等键（`ParsedFile.sha256`——跨版本重导时锚失效判定面）。
    pub file_sha256: String,
    /// 块序（`ParsedFile.blocks[].idx`）。
    pub block_idx: usize,
    /// 该块承载的 REQ（REQ→块正向索引的反向聚合；无 REQ 块不入行——
    /// 「无 REQ」由覆盖报告/overlay 面呈现，不混进本索引）。
    pub req_ids: Vec<String>,
    /// 块结论（Agent 标记面——REQ 行恒为 [`BlockVerdict::Requirement`]：
    /// Agent REQ 必落在 requirement 区间，正则 REQ 本就是抽取的要求）。
    pub verdict: BlockVerdict,
    /// REQ 来源（agent / regex——`merge_extraction` ②过滤保证同块不双源）。
    pub marked_by: MarkSource,
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

    // ---------------------------------------------------------------- #285 ②

    #[test]
    fn mark_source_serde_matches_params_key() {
        // 落库键口径：agent / regex（extract.rs 与 merge.rs 的 params.marked_by）
        assert_eq!(serde_json::to_string(&MarkSource::Agent).unwrap(), "\"agent\"");
        assert_eq!(
            serde_json::to_string(&MarkSource::RegexFallback).unwrap(),
            "\"regex\""
        );
        assert_eq!(MarkSource::Agent.as_str(), "agent");
        assert_eq!(MarkSource::RegexFallback.as_str(), "regex");
    }

    #[test]
    fn block_requirement_index_serde_roundtrip() {
        let row = BlockRequirementIndex {
            file_sha256: "ab12".into(),
            block_idx: 427,
            req_ids: vec!["REQ-0015".into(), "REQ-0016".into()],
            verdict: BlockVerdict::Requirement,
            marked_by: MarkSource::Agent,
        };
        let j = serde_json::to_string(&row).unwrap();
        let back: BlockRequirementIndex = serde_json::from_str(&j).unwrap();
        assert_eq!(row, back);
        assert!(j.contains("\"marked_by\":\"agent\""), "来源面小写蛇形：{j}");
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
        assert!(r.unjudged_ranges.is_empty());
        assert_eq!(r.unjudged_blocks, 0, "全覆盖零未判定");
        assert_eq!(r.excluded_blocks, 50, "noise 半区已排除");
        assert!(!r.has_unjudged());
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
        assert_eq!(r.unjudged_ranges, vec![(30, 49)]);
        assert_eq!(r.unjudged_blocks, 20, "中间缺口 20 块未判定");
        assert_eq!(r.excluded_blocks, 30, "前 30 块 noise 已排除");
        assert!(r.has_unjudged());
    }

    /// #244 三态分账：全排除（B 态）与半途而废（C 态非零）在报告上
    /// 必须可区分——旧 `uncovered` 一锅论分不清「排除了」和「没看」。
    #[test]
    fn tri_state_accounting_distinguishes_excluded_from_unjudged() {
        // 同样 covered_blocks=50：全部是排除 vs 一半未判定
        let all_excluded = compute_coverage(&[mark(0, 49, BlockVerdict::Rule)], 50);
        assert_eq!(all_excluded.covered_blocks, 50);
        assert_eq!(all_excluded.excluded_blocks, 50);
        assert_eq!(all_excluded.unjudged_blocks, 0);
        assert!(!all_excluded.has_unjudged(), "全排除 → 无未判定 → 门放行");

        let half_unjudged = compute_coverage(&[mark(0, 24, BlockVerdict::Rule)], 50);
        assert_eq!(half_unjudged.covered_blocks, 25);
        assert_eq!(half_unjudged.excluded_blocks, 25);
        assert_eq!(half_unjudged.unjudged_blocks, 25);
        assert!(half_unjudged.has_unjudged(), "半途而废 → 有未判定 → 门阻断");
    }

    /// review_table 是结构化提取载体（A 态变体）——不算排除。
    #[test]
    fn review_table_not_counted_as_excluded() {
        let marks = vec![
            mark(0, 9, BlockVerdict::ReviewTable),
            mark(10, 19, BlockVerdict::Noise),
        ];
        let r = compute_coverage(&marks, 20);
        assert!(r.complete);
        assert_eq!(r.excluded_blocks, 10, "只有 noise 计排除");
        assert_eq!(r.by_verdict.get("review_table"), Some(&10));
        assert_eq!(r.unjudged_blocks, 0);
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
            substantive: false,
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
            substantive: false,
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
            substantive: false,
        }];
        // rule 区间不允许挂 requirement——规则说明不需要响应
        assert!(validate_submitted(&marks, &reqs).is_err());
    }

    #[test]
    fn invalid_category_rejected() {
        // #219 二步：Agent 提交面 fail-loud——非法 category 在提交时即拒，
        // 不等 extract 合并时才炸（晚失败 = 半个库状态）
        let marks = vec![mark(0, 99, BlockVerdict::Requirement)];
        let reqs = vec![SubmittedRequirement {
            block_idx: 10,
            statement: "某要求".into(),
            category: Some("乱写的类别".into()),
            mandatory: false,
            substantive: false,
        }];
        assert!(matches!(
            validate_submitted(&marks, &reqs),
            Err(CoverageError::InvalidCategory { .. })
        ));
        // 合法七值放行
        for c in REQUIREMENT_CATEGORIES {
            let reqs = vec![SubmittedRequirement {
                block_idx: 10,
                statement: "某要求".into(),
                category: Some(c.to_string()),
                mandatory: false,
                substantive: false,
            }];
            assert!(validate_submitted(&marks, &reqs).is_ok(), "合法类别被拒：{c}");
        }
    }
}