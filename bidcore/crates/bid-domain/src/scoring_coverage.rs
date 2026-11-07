//! scoring_coverage.rs —— #318 评标细则逐条响应覆盖检查。
//!
//! 评委按细则逐条打分：「技术方案完整性（10 分）：需包含总体架构、实施
//! 步骤、风险应对」。#252 已把细则映射到章节（ScoringMapping），但正文
//! 写完不检查「这条细则要求的点都写了吗」——漏点即扣分而我们不自知。
//!
//! 本模块补两层：
//! 1. **拆点**（[`extract_scoring_points`]）：从细则原文提取「要求的点」
//!    （关键词/短语）。**确定性字面挖掘**——触发词（需包含/包括/应具备/
//!    应提供…）后的顿号/连接词枚举，无触发词时回退顿号串形态。与 #285①
//!    ★漏标检测同一纪律：只报不补、不伪装语义理解（语义等价表述——
//!    「架构设计」vs「总体架构」——不识别，报告面如实标注）；
//! 2. **覆盖检查**（[`check_scoring_coverage`]）：每个点对细则的**映射
//!    章节**（ScoringItem.mappings——调用方已并入持久化账本）做字面子串
//!    命中（章节标题+正文），命中位报到章节；未覆盖点明确列出；
//!    建议扣分 = Σ(单项满分 × 未覆盖点占比)——启发式参考值。
//!
//! 纯函数纪律：零 IO、零时钟；章节正文的取用由调用方注入闭包（store 是
//! 唯一真源，本模块不重复实现取数）。

use crate::scoring::ScoringItem;

/// 拆点上限（每条细则）——防御性：病态文本（整页顿号）不炸报告。
pub const MAX_POINTS_PER_CRITERION: usize = 16;

/// 点覆盖判定结果（一条 ScoringPoint 的检查面）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointCoverage {
    /// 要求的点（细则原文提取的短语）。
    pub point: String,
    /// 命中章节（首个正文命中的映射章节 SEC-xxx）；None = 未覆盖
    /// （所有映射章节的正文都不含该点——漏点即扣分面）。
    pub hit_section: Option<String>,
}

/// 单条细则的覆盖裁决。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterionCoverage {
    /// 细则 REQ id（REQ-xxxx）。
    pub criterion_id: String,
    /// 细则锚点摘录（criteria——回显定位用）。
    pub criteria_excerpt: String,
    /// 单项满分（None = 否决项/无分值——扣分建议不计入，点覆盖照报）。
    pub max_score: Option<f64>,
    /// 映射章节（mappings 并集——该细则「应该写在哪」）。
    pub mapped_sections: Vec<String>,
    /// 逐点覆盖判定。
    pub points: Vec<PointCoverage>,
}

/// 覆盖检查总报告（#318 验收 ③ 的「报告」面）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoringCoverageReport {
    /// 逐细则覆盖裁决（含拆点失败的细则——points 空，如实可见）。
    pub criteria: Vec<CriterionCoverage>,
    /// 细则总数（评分项）。
    pub criteria_total: usize,
    /// 拆出 ≥1 点的细则数。
    pub criteria_with_points: usize,
    /// 无章节映射的细则数（点覆盖不可判——先建纲/映射）。
    pub criteria_unmapped: usize,
    /// 要求的点总数。
    pub points_total: usize,
    /// 已覆盖点数。
    pub points_covered: usize,
    /// 覆盖率（0-1；无点时 0——不伪装 100%）。
    pub coverage_ratio: f64,
    /// 建议扣分合计 = Σ(单项满分 × 未覆盖点占比)——启发式参考值
    /// （评标实际扣分以评委裁量为准——notes 明示）。
    pub suggested_deduction: f64,
    /// 口径与局限说明（诚实面——随报告透出，不藏在文档里）。
    pub notes: Vec<String>,
}

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- ① 拆点（字面挖掘）

/// 拆点触发词（细则「要求什么」的谓语面——长词在前防半截命中）。
/// 来源：真实评标办法表述族（需包含/包括/应具备/应提供/须响应…）。
const TRIGGERS: [&str; 18] = [
    "需包含", "应包含", "须包含", "需包括", "应包括", "须包括", "需涵盖", "应涵盖",
    "需具备", "应具备", "须具备", "需提供", "应提供", "须提供", "需响应", "应响应",
    "含有", "包括",
];

/// 段内连接词（「A、B及C」的段内再拆——及/和/与/或）。
const CONNECTIVES: [&str; 5] = ["以及", "及", "和", "与", "或"];

/// 从细则原文提取「要求的点」——确定性字面挖掘（#318 ①）。
///
/// 规则（保守、可解释、可复现）：
/// 1. 触发词扫描：命中任一触发词后，取其后的**同句**片段（到 。；！？\n
///    或串尾），按 、切分，段内再按连接词拆；
/// 2. 无触发词回退：全文找**顿号串**（≥2 个合法段）——「总体架构、实施
///    步骤、风险应对」直接成点；
/// 3. 合法段 = 2..=12 字、CJK 主导（汉字数×2 ≥ 字符数——滤掉「（10分）」
///    类数字/标点残段）、非纯数字、不含句读；
/// 4. 段首剥连接词残根、段尾剥「等」；去重保序；上限
///    [`MAX_POINTS_PER_CRITERION`]。
///
/// 返回空 = 细则无枚举形态（如实——不虚构点；报告面计入
/// `criteria_with_points` 的分母差，提示人工读细则）。
pub fn extract_scoring_points(rule_text: &str) -> Vec<String> {
    let text = rule_text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let mut points: Vec<String> = Vec::new();
    let mut used_trigger = false;

    // ---- 1. 触发词扫描（字节位安全：find 定位天然在字符边界）
    //      同一触发词多次命中全收；不同触发词重叠命中由去重收敛。
    let mut hits: Vec<usize> = Vec::new(); // 触发词尾字节位
    for t in TRIGGERS {
        let mut from = 0usize;
        while let Some(pos) = text[from..].find(t) {
            hits.push(from + pos + t.len());
            from += pos + t.len();
        }
    }
    if !hits.is_empty() {
        used_trigger = true;
        hits.sort_unstable();
        hits.dedup();
        for end in hits {
            let tail = sentence_tail(&text[end..]);
            collect_segments(tail, &mut points);
        }
    }

    // ---- 2. 无触发词回退：顿号串形态（≥2 个合法段才是枚举证据——
    //      单段无顿号 = 普通句，不成点：「评委酌情打分」不虚构为点）
    if !used_trigger {
        let valid_segments = text
            .split('、')
            .filter(|s| clean_point(s).is_some())
            .count();
        if valid_segments >= 2 {
            collect_segments(text, &mut points);
        }
    }

    dedup_cap(points)
}

/// 取同句尾段（到句终结符或串尾——触发词后的枚举不住在下一句）。
fn sentence_tail(s: &str) -> &str {
    match s.find(['。', '；', ';', '！', '？', '?', '!', '\n']) {
        Some(end) => &s[..end],
        None => s,
    }
}

/// 顿号切分 + 段内连接词再拆 + 合法性过滤，收集进 points。
fn collect_segments(tail: &str, points: &mut Vec<String>) {
    for seg in tail.split('、') {
        // 段内连接词再拆（「B及C」）——替换为分隔符后重切
        let mut replaced = seg.to_string();
        for c in CONNECTIVES {
            replaced = replaced.replace(c, "、");
        }
        for piece in replaced.split('、') {
            if let Some(p) = clean_point(piece) {
                points.push(p);
            }
        }
    }
}

/// 段清洗：剥连接词残根/首尾空白/尾「等」；合法性（2..=12 字、CJK 主导、
/// 无句读、非纯数字）不过 → None。
fn clean_point(raw: &str) -> Option<String> {
    let mut p = raw.trim().to_string();
    // 段首连接词残根（「及A」→「A」）
    for c in CONNECTIVES {
        if p.starts_with(c) {
            p = p[c.len()..].trim().to_string();
        }
    }
    // 段尾「等」（「C等」→「C」——枚举省略号不是点本身）
    if p.ends_with('等') {
        p.pop();
    }
    let total = p.chars().count();
    if total < 2 || total > 12 {
        return None;
    }
    if p.contains(['。', '；', ';', '：', ':', '，', ',', '（', '）', '(', ')', '"', '"', '「', '」']) {
        return None;
    }
    let cjk = p.chars().filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c)).count();
    let total = p.chars().count();
    if cjk == 0 || cjk * 2 < total {
        return None; // CJK 不主导（数字/字母/符号残段）
    }
    if p.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(p)
}

/// 去重保序 + 上限。
fn dedup_cap(mut points: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    points.retain(|p| seen.insert(p.clone()));
    points.truncate(MAX_POINTS_PER_CRITERION);
    points
}

// ---------------------------------------------------------------- ② 覆盖检查

/// 覆盖检查（#318 ②/③——纯函数）。
///
/// * `items`：评分项（调用方负责并入持久化映射——与 score 工具同口径：
///   `derive_scoring_items` + store.scoring_mappings() 去重并入）；
/// * `section_text`：section_id → 章节正文文本（命中 haystack——**纯正文，
///   不含建纲标题**：#252 评分驱动建纲的 L2 标题派生自细则原文（「保密
///   措施」章节标题天然含点「保密措施」），标题自命中是伪覆盖——空章节
///   不得因同名标题被判「已响应」。正文内部自带的标题行照常命中。）
///
/// 建议扣分 = Σ(有分值细则的 满分 × 未覆盖点/总点)；无点/无映射细则不
/// 计入（不可判 ≠ 全扣——诚实面）。coverage_ratio 无点时为 0（不伪装 100%）。
pub fn check_scoring_coverage<F>(items: &[ScoringItem], section_text: F) -> ScoringCoverageReport
where
    F: Fn(&str) -> String,
{
    let mut criteria = Vec::new();
    let mut criteria_with_points = 0usize;
    let mut criteria_unmapped = 0usize;
    let mut points_total = 0usize;
    let mut points_covered = 0usize;
    let mut suggested_deduction = 0f64;

    for item in items {
        let rule_text = item
            .score_rules
            .as_deref()
            .unwrap_or(item.criteria.as_str());
        let points = extract_scoring_points(rule_text);
        let mapped: Vec<String> = item
            .mappings
            .iter()
            .map(|m| m.section_id.clone())
            .collect();
        if mapped.is_empty() {
            criteria_unmapped += 1;
        }
        if !points.is_empty() {
            criteria_with_points += 1;
        }

        let mut pcs = Vec::new();
        let mut covered = 0usize;
        for point in &points {
            // 无映射章节 → 覆盖不可判（None——与「写了但没命中」同面：
            // 都是漏点，但 notes 归因到「先建纲/映射」）
            let hit = mapped
                .iter()
                .find(|sid| section_text(sid).contains(point.as_str()))
                .cloned();
            if hit.is_some() {
                covered += 1;
            }
            pcs.push(PointCoverage { point: point.clone(), hit_section: hit });
        }
        points_total += points.len();
        points_covered += covered;

        // 建议扣分：满分 × 未覆盖占比（无点 → 0；无分值 → 0）
        if let (Some(max), false) = (item.max_score, points.is_empty()) {
            let uncovered = points.len() - covered;
            suggested_deduction += max * uncovered as f64 / points.len() as f64;
        }

        criteria.push(CriterionCoverage {
            criterion_id: item.requirement_id.clone(),
            criteria_excerpt: truncate_excerpt(&item.criteria),
            max_score: item.max_score,
            mapped_sections: mapped,
            points: pcs,
        });
    }

    let criteria_total = criteria.len();
    let coverage_ratio = if points_total == 0 {
        0.0
    } else {
        points_covered as f64 / points_total as f64
    };

    let mut notes = vec![
        "拆点 = 细则原文的字面枚举提取（触发词 + 顿号/连接词分隔——确定性、无语义理解）：\
         语义等价表述（如「架构设计」vs「总体架构」）不识别，覆盖判定按字面子串命中。"
            .to_string(),
        "建议扣分 = Σ(单项满分 × 未覆盖点占比)——启发式参考值，评标实际扣分以评委裁量为准。"
            .to_string(),
    ];
    if criteria_unmapped > 0 {
        notes.push(format!(
            "无章节映射的细则 {criteria_unmapped} 条——点覆盖不可判（先建纲/映射再查；\
             其点计入未覆盖分母是保守面）。"
        ));
    }
    let criteria_no_points = criteria_total - criteria_with_points;
    if criteria_no_points > 0 {
        notes.push(format!(
            "{criteria_no_points} 条细则未拆出点（无枚举形态/拆点局限）——需人工读细则核对。"
        ));
    }

    ScoringCoverageReport {
        criteria,
        criteria_total,
        criteria_with_points,
        criteria_unmapped,
        points_total,
        points_covered,
        coverage_ratio,
        suggested_deduction,
        notes,
    }
}

/// 摘录回显截断（长锚点不炸报告——40 字符 + 省略号）。
fn truncate_excerpt(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 40 {
        s.to_string()
    } else {
        let head: String = chars[..40].iter().collect();
        format!("{head}…")
    }
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::SectionMapping;

    fn item(rule: &str, score: Option<f64>, sections: &[&str]) -> ScoringItem {
        ScoringItem {
            requirement_id: "REQ-0001".into(),
            weight: score,
            max_score: score,
            criteria: "技术方案完整性".into(),
            score_rules: Some(rule.into()),
            is_hard_gate: false,
            parent_id: None,
            tiers: vec![],
            is_objective: false,
            mappings: sections
                .iter()
                .map(|s| SectionMapping { section_id: (*s).into(), weight_share: 1.0 })
                .collect(),
            our_estimate: None,
        }
    }

    #[test]
    fn issue_example_extracts_three_points() {
        // issue #318 原文例：「需包含总体架构、实施步骤、风险应对」
        let pts = extract_scoring_points("技术方案完整性（10分）：需包含总体架构、实施步骤、风险应对。");
        assert_eq!(pts, vec!["总体架构", "实施步骤", "风险应对"]);
    }

    #[test]
    fn trigger_families_and_connectives() {
        // 触发词族
        assert_eq!(
            extract_scoring_points("应具备质量保证体系"),
            vec!["质量保证体系"]
        );
        assert_eq!(
            extract_scoring_points("须提供项目经理资质证书、技术负责人证书"),
            vec!["项目经理资质证书", "技术负责人证书"]
        );
        // 段内连接词：「A、B及C」
        assert_eq!(
            extract_scoring_points("需包含运维方案、应急预案及培训计划"),
            vec!["运维方案", "应急预案", "培训计划"]
        );
        // 尾「等」剥离
        assert_eq!(
            extract_scoring_points("包括施工组织设计、进度计划等。"),
            vec!["施工组织设计", "进度计划"]
        );
    }

    #[test]
    fn fallback_enumeration_without_trigger() {
        // 无触发词的顿号串（L2 细则标题/正文常见形态）
        let pts = extract_scoring_points("保密措施、保密制度、保密承诺");
        assert_eq!(pts, vec!["保密措施", "保密制度", "保密承诺"]);
        // 无枚举形态 → 空（如实，不虚构）
        assert!(extract_scoring_points("本项评分由评委酌情打分。").is_empty());
        assert!(extract_scoring_points("").is_empty());
    }

    #[test]
    fn garbage_segments_filtered() {
        // 数字/括号残段、超长段、单字段全滤
        let pts = extract_scoring_points("需包含（10分）评分、营业执照副本、这个超长的要点远远超过十二个字符上限要滤掉、A");
        assert_eq!(pts, vec!["营业执照副本"]);
    }

    #[test]
    fn coverage_positive_negative_and_deduction() {
        // issue 例：A/B 写了、C 漏了 → 2/3 覆盖，建议扣 10×1/3
        let rule = "技术方案完整性（10分）：需包含总体架构、实施步骤、风险应对";
        let it = item(rule, Some(10.0), &["SEC-0001"]);
        let sections = |sid: &str| {
            match sid {
                // 标题命中「总体架构」，正文命中「实施步骤」，风险应对缺失
                "SEC-0001" => "3.1 总体架构。本章论述实施步骤与里程碑。".to_string(),
                _ => String::new(),
            }
        };
        let r = check_scoring_coverage(&[it], sections);
        assert_eq!(r.criteria_total, 1);
        assert_eq!(r.points_total, 3);
        assert_eq!(r.points_covered, 2);
        assert!((r.coverage_ratio - 2.0 / 3.0).abs() < 1e-9);
        assert!((r.suggested_deduction - 10.0 / 3.0).abs() < 1e-9);
        let c = &r.criteria[0];
        assert_eq!(c.points[0].hit_section.as_deref(), Some("SEC-0001"), "正文标题行命中");
        assert_eq!(c.points[1].hit_section.as_deref(), Some("SEC-0001"));
        assert_eq!(c.points[2].hit_section, None, "风险应对未找到——漏点面");
        // notes 含口径与局限说明
        assert!(r.notes.iter().any(|n| n.contains("字面")));
    }

    #[test]
    fn unmapped_criterion_counts_uncovered_and_notes() {
        let it = item("需包含A方案、B方案", Some(5.0), &[]);
        let r = check_scoring_coverage(&[it.clone()], |_| String::new());
        assert_eq!(r.criteria_unmapped, 1);
        assert_eq!(r.points_covered, 0);
        assert!((r.suggested_deduction - 5.0).abs() < 1e-9, "无映射全未覆盖");
        assert!(r.notes.iter().any(|n| n.contains("无章节映射")));
    }

    #[test]
    fn no_points_criterion_is_honest_not_penalized() {
        let it = item("评委酌情打分", Some(8.0), &["SEC-0001"]);
        let r = check_scoring_coverage(&[it], |_| "任意正文".into());
        assert_eq!(r.criteria_with_points, 0);
        assert_eq!(r.points_total, 0);
        assert_eq!(r.coverage_ratio, 0.0, "无点不伪装 100%");
        assert_eq!(r.suggested_deduction, 0.0, "不可判 ≠ 全扣");
        assert!(r.notes.iter().any(|n| n.contains("未拆出点")));
    }

    #[test]
    fn multi_section_hit_reports_first_mapped() {
        // 多章节映射：点命中最先出现的映射章节（归因面）
        let it = item("需包含运维方案", Some(4.0), &["SEC-A", "SEC-B"]);
        let sections = |sid: &str| {
            match sid {
                "SEC-B" => "运维方案详见本章。".to_string(),
                _ => String::new(),
            }
        };
        let r = check_scoring_coverage(&[it], sections);
        assert_eq!(r.criteria[0].points[0].hit_section.as_deref(), Some("SEC-B"));
    }

    #[test]
    fn points_cap_defends_garbage_flood() {
        // 顿号洪水：截到 MAX_POINTS_PER_CRITERION
        let flood: String = (0..40).map(|i| format!("要点{i:02}、")).collect();
        let pts = extract_scoring_points(&flood);
        assert_eq!(pts.len(), MAX_POINTS_PER_CRITERION);
    }
}
