//! scoring.rs —— T-3 ScoringItem 评分矩阵（issue #11 批次 v3 / V3-2 → issue #94 B2 重构）。
//!
//! V3-2 现状：`Requirement.category == "评分项"` 给评分面三维结构——**分值（weight/
//! max_score）+ 标准原文（criteria）+ 否决门（is_hard_gate）**，平铺无层级。
//!
//! B2 扩展（issue #94 二轮——从「一个数字」到「能算总分」）：
//! - **五新面**：`parent_id`（多级结构：技术分 30 → 总体方案 10）/ `tiers`（档位：
//!   优/良/中/差 各自分值区间）/ `is_objective`（客观可数 vs 主观评审）/
//!   `section_id`（评分项 ↔ 章节映射）/ `our_estimate`（我方自评——派生恒 None，
//!   B2 任务 3 的 [`SectionScorer`](crate::scoring) 填充）；
//! - **派生树**：[`ScoringTree`]——从平铺 requirements 派生层级树（`children` /
//!   `max_score` 子树满分和 / `validate_sums` 结构体检：子和失配·孤儿·环·总分惯例）；
//! - **确定性派生不变**：新面全部来自 `params` 槽位（`parent`/`tiers`/`objective`/
//!   `section`）+ 纯函数启发（[`infer_objective`]——证书/可数形态 → 客观），零 IO、
//!   重算幂等；`parse_tiers` 提供档位文本（"优：9-10 分；良：7-9 分…"）的确定性
//!   解析辅面（手写扫描，无 regex 依赖——bid-domain 依赖面不加宽）。
//!
//! 诚实状态纪律（对齐「分值缺失显式登记」）：
//! - 档位/父项/章节缺失 → None/空表（不伪装）；
//! - 子项满分和 ≠ 父项声明值、parent 指向不存在节点、环、根合计 ≠ 100——
//!   一律 [`ScoringTree::validate_sums`] 显式登记（如实报告，不硬套不阻断）。

use crate::{DomainStore, Requirement, ToolError};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------- 实体

/// 档位（优/良/中/差 各自分值区间——评分细则的常见形态）。
///
/// `min_score`/`max_score` 为该档位的**分值区间**（非百分比）。真实评标办法
/// 档位标签不限于优良中差（好/较好/一般/合格档亦常见）——`label` 原文保留。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreTier {
    /// 档位标签原文（"优"/"良"/"中"/"差"/"好"/"较好"/"一般"…）
    pub label: String,
    /// 该档位最低分（含）
    pub min_score: f64,
    /// 该档位最高分（含）
    pub max_score: f64,
}

/// 评分矩阵项（T-2/T-3 规格 struct 的 Rust 落地 → B2 五新面）。
///
/// `is_hard_gate=true`（否决项）：**漏响应即废标**——独立闸门（T-2 blocking
/// 的否决面子集；本实体只登记，闸门裁决在 matrix.rs `check_export_ready`）。
///
/// B2 新面派生来源（确定性，全部可重算）：
/// - `parent_id` ← `params.parent`（父评分项 REQ id——多级评分表：技术分 55 →
///   方案完整性 30）；缺失 → None（平铺评分表常态）。
/// - `tiers` ← `params.tiers`（结构化档位表，`[[label,min,max],…]` 或
///   `[{label,min,max},…]` 双形态兼容——extract 层形态未定，宽进）；或由
///   [`parse_tiers`] 从 criteria 文本解析（辅面）。
/// - `is_objective` ← `params.objective`（显式优先）；缺失 → [`infer_objective`]
///   纯函数启发（证书/可数计分形态 → 客观；评审形容词 → 主观；默认主观）。
/// - `section_id` ← `params.section`（映射章节 SEC-id/章号——B2 任务 3 自评的
///   「章节 ↔ 评分项」对接面）；缺失 → None（未映射显式化）。
/// - `our_estimate`：**派生恒 None**（自评是运行时评估产物，不是抽取物）——
///   由任务 3 [`SectionScorer`] 填充，此处只占位不伪装。
/// - `score_rules` ← `params.score_rules`（#186 预留面：完整评分细则原文——
///   `criteria` 只是首个锚点摘录，可能残缺半句）。**当前判档不消费本字段**
///   （assess 仍是关键词启发式 rule_heuristic——数据先透出，不伪装已接入），
///   细则判档接入见 #186 第二步（依赖 #170 评分映射）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoringItem {
    pub requirement_id: String,
    /// 分值/权重（评分项有；否决项 None——否决无分值，漏了直接废标）
    pub weight: Option<f64>,
    /// 单项满分（V3-2 = weight 同值；细则拆分档位时分离）
    pub max_score: Option<f64>,
    /// 评分标准原文（锚点摘录——审校比对用）
    pub criteria: String,
    /// 完整评分细则原文（#186 预留面——判档不消费，透出供诊断/起草对照；
    /// None = 解析层未登记细则）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_rules: Option<String>,
    /// 否决项/废标项 → true（漏响应即废标的独立闸门）
    pub is_hard_gate: bool,
    /// 父评分项 REQ id（多级结构：技术分 → 子项）；顶层/平铺 → None
    #[serde(default)]
    pub parent_id: Option<String>,
    /// 档位表（优/良/中/差 分值区间）；无档位形态 → 空表
    #[serde(default)]
    pub tiers: Vec<ScoreTier>,
    /// 客观分（可精确计算：证书/业绩个数/价格）true；主观分（评委打分）false
    #[serde(default)]
    pub is_objective: bool,
    /// #224 多对多章节映射（SEC-xxx × weight_share——派生联结面：
    /// `params.section`（RuleParsed，share=1.0）∪ store 持久化映射。
    /// issue 原文的 `mapping_ids: Vec<String>` 落地为本形态——share 是
    /// 跨章节摊分的必需信息（同项多章各算一次 → 总分虚高的主防）；
    /// SM-id 实体面在 [`DomainStore::scoring_mappings`]，经
    /// `(scoring_req_id, section_id)` 复合键与本字段联结）。空 = 未映射。
    #[serde(default)]
    pub mappings: Vec<SectionMapping>,
    /// 我方自评预计得分（派生恒 None——[`SectionScorer`] 运行时填充）
    #[serde(default)]
    pub our_estimate: Option<f64>,
}

/// #224 评分项 → 章节的单条映射联结（多对多的元素形态）。
///
/// `weight_share` ∈ (0,1]：该章节承载的权重占比——「技术方案 20 分」跨
/// 2 章节（0.6/0.4）时，两章的该项满分分别为 12/8（[`SectionScorer::score_section`]
/// 按 share 摊分，防同一项重复计入总分）。RuleParsed 单值映射恒 1.0
/// （与历史行为逐位一致）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionMapping {
    /// 被评章节（SEC-xxx / 章号——与历史 params.section 同域）
    pub section_id: String,
    /// 该章节承载的权重占比（多章节分摊；单映射恒 1.0）
    pub weight_share: f64,
}

/// #224 映射来源（业务事实的审计面——谁建立的映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MappingSource {
    /// 解析层从评分表/评审表行派生（params.section——确定性回放）
    RuleParsed,
    /// Agent 提交（区间标记/推导——MCP 写面后续接入，本步先立面）
    AgentSubmitted,
    /// 人工登记（#182 可信身份面后续接入）
    Manual,
}

/// #224 评分项 ↔ 章节多对多映射（持久化业务事实——独立身份可追溯可审计）。
///
/// 与 [`SectionMapping`]（派生联结面）的分工：本实体是**账本**（含证据引用/
/// 原文锚/来源——AgentSubmitted 与 Manual 的落点，快照持久化）；派生面是
/// **消费视图**（RuleParsed 的 params.section 不进本账本——每次确定性重放，
/// 入账反而制造快照噪声）。幂等键 = `(scoring_req_id, section_id)`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoringMapping {
    /// SM-xxxx（store 签发）
    pub id: String,
    /// 评分项 REQ id（必须存在于 requirements——孤儿映射拒载）
    pub scoring_req_id: String,
    /// 被评章节 SEC-xxx（必须存在于 sections——孤儿映射拒载）
    pub section_id: String,
    /// 该章节承载的权重占比 ∈ (0,1]（多章节分摊；重复计分检测见 validate_sums 扩展——第 2 步）
    pub weight_share: f64,
    /// 支撑该映射的证据（EV-id——映射依据的可追溯面）
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// 原始评分规则原文锚（追溯：分数 → 评分项 → 规则原文页/行）
    pub source_anchor: crate::Anchor,
    /// 谁建立的映射（RuleParsed 不入账；本集合只见 AgentSubmitted/Manual）
    pub created_from: MappingSource,
}

// ---------------------------------------------------------------- 档位解析（纯函数辅面）

/// 档位文本确定性解析（手写扫描——bid-domain 不引 regex，依赖面不加宽）。
///
/// 真实评标办法的档位表述（解析面按首现序全量收集，无匹配段跳过）：
/// - "优：9-10 分；良：7-9 分；中：5-7 分；差：0-5 分"
/// - "好 9～10分，较好 7～8.9分，一般 5～6.9分，差 0～4.9分"
/// - "优秀 18-20；良好 15-17.9；一般 10-14.9；差 0-9.9"
///
/// 段分隔：`；;，,\n`；段内形态：`标签[:：]?\s*数字 [-—～~] 数字 [分]`。
/// 返回空表 = 文本无档位形态（显式——调用方不伪装档位存在）。
pub fn parse_tiers(text: &str) -> Vec<ScoreTier> {
    let mut out = Vec::new();
    for seg in text.split(['；', ';', '，', ',', '\n', '。']) {
        if let Some(t) = parse_tier_segment(seg) {
            out.push(t);
        }
    }
    out
}

/// 单段解析：`标签 数字 sep 数字`（sep ∈ `-—～~`）。标签 = 首数字前的非空白
/// 内容（去 `：:`），限 1-6 字（防长句误配）。
fn parse_tier_segment(seg: &str) -> Option<ScoreTier> {
    let chars: Vec<char> = seg.chars().collect();
    // 全段定位首个数字序列（标签在前——"优：9-10 分"）
    let (n1s, n1e, n1v) = find_number(&chars)?;
    // 分隔符（跳过空白）
    let mut p = n1e;
    while p < chars.len() && chars[p].is_whitespace() {
        p += 1;
    }
    if p >= chars.len() || !matches!(chars[p], '-' | '—' | '～' | '~') {
        return None;
    }
    let second_num = scan_number(&chars, p + 1)?;
    // 标签：段首 → 首数字前，去空白与冒号
    let label: String = chars[..n1s]
        .iter()
        .filter(|c| !c.is_whitespace() && **c != '：' && **c != ':')
        .collect();
    if label.is_empty() || label.chars().count() > 6 {
        return None;
    }
    Some(ScoreTier {
        label,
        min_score: n1v,
        max_score: second_num.2,
    })
}

/// 全段定位首个数字序列（含小数；跳过任意非数字前缀——标签/空白/冒号）。
/// 返回 (起始, 结束, 值)。
fn find_number(chars: &[char]) -> Option<(usize, usize, f64)> {
    let mut i = 0;
    while i < chars.len() && !chars[i].is_ascii_digit() {
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    let start = i;
    let mut s = String::new();
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
        s.push(chars[i]);
        i += 1;
    }
    if s == "." {
        return None;
    }
    s.parse::<f64>().ok().map(|v| (start, i, v))
}

/// 从 `pos` 起扫描数字（含小数）。返回 (起始, 结束, 值)。
fn scan_number(chars: &[char], pos: usize) -> Option<(usize, usize, f64)> {
    let mut i = pos;
    while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '分') {
        i += 1;
    }
    let start = i;
    let mut s = String::new();
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
        s.push(chars[i]);
        i += 1;
    }
    if s.is_empty() || s == "." {
        return None;
    }
    s.parse::<f64>().ok().map(|v| (start, i, v))
}

// ---------------------------------------------------------------- 主客观启发（纯函数）

/// 主客观判定启发（`params.objective` 缺失时的兜底——纯字符串规则，可测试可追溯）。
///
/// **客观**（可精确计算——数得出来）：
/// - 可数计分形态：`每 …（人/个/项/份/家/年/个）…（得/加）N 分`；
/// - 证书/资质/等级形态：`证书`/`ISO`/`AAA`/`AA`/`资质`/`注册建造师`/`职称`/
///   `认证`/`资信`/`获奖`/`奖项`。
///
/// **主观**（评委评审——判档打分）：评审形容词（`完整性`/`针对性`/`合理性`/
/// `可行性`/`科学性`/`先进性`/`响应程度`/`理解`/`深度`/`优`…档位形态）；
/// 默认主观（技术方案评审的保守默认——宁可标主观不自诩客观）。
pub fn infer_objective(criteria: &str) -> bool {
    // 证书/资质/等级词面（强客观信号）
    const CERT_WORDS: &[&str] = &[
        "证书",
        "ISO",
        "iso",
        "AAA",
        "AA级",
        "资质",
        "注册建造师",
        "职称",
        "认证",
        "资信",
        "获奖",
        "奖项",
        "安全生产许可证",
        "许可证",
    ];
    if CERT_WORDS.iter().any(|w| criteria.contains(w)) {
        return true;
    }
    // 可数计分：`每 X … 得/加 N 分`（X 短距离量词）——手写扫描
    if has_countable_pattern(criteria) {
        return true;
    }
    false
}

/// 可数计分形态扫描：`每` 后 12 字符窗口内见量词（人/个/项/份/家/年/例）且再
/// 24 字符窗口内见 `数字 + 分`（「每提供 1 个**得 2 分**」/「每个 **1 分**」双形态
/// ——「得/加」字可缺省）。双窗口限距防跨句误配。
fn has_countable_pattern(criteria: &str) -> bool {
    let chars: Vec<char> = criteria.chars().collect();
    let quants = ['人', '个', '项', '份', '家', '年', '例'];
    for (i, c) in chars.iter().enumerate() {
        if *c != '每' {
            continue;
        }
        // 窗口 1：量词
        let w1_end = (i + 12).min(chars.len());
        let quant_at = (i + 1..w1_end).find(|&j| quants.contains(&chars[j]));
        let Some(q) = quant_at else { continue };
        // 窗口 2：数字 + 分（可隔空白——「得/加」可选）
        let w2_end = (q + 24).min(chars.len());
        let mut j = q + 1;
        while j < w2_end {
            if chars[j].is_ascii_digit() || (chars[j] == '.' && j > q + 1) {
                while j < w2_end && (chars[j].is_ascii_digit() || chars[j] == '.') {
                    j += 1;
                }
                let mut m = j;
                while m < w2_end && chars[m].is_whitespace() {
                    m += 1;
                }
                if m < w2_end && chars[m] == '分' {
                    return true;
                }
            } else {
                j += 1;
            }
        }
    }
    false
}

// ---------------------------------------------------------------- 槽位派生

/// `params.tiers` 双形态兼容解析（extract 层形态未定——宽进）：
/// `[[label,min,max],…]` / `[{label,min_score,max_score},…]`。
fn tiers_from_params(v: Option<&serde_json::Value>) -> Vec<ScoreTier> {
    let Some(arr) = v.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in arr {
        let tier = match e {
            // 数组形态 [label, min, max]
            serde_json::Value::Array(a) if a.len() >= 3 => {
                match (a[0].as_str(), a[1].as_f64(), a[2].as_f64()) {
                    (Some(l), Some(mn), Some(mx)) => Some(ScoreTier {
                        label: l.to_string(),
                        min_score: mn,
                        max_score: mx,
                    }),
                    _ => None,
                }
            }
            // 对象形态 {label, min_score, max_score}
            serde_json::Value::Object(_) => {
                let l = e.get("label").and_then(|x| x.as_str());
                let mn = e.get("min_score").and_then(|x| x.as_f64());
                let mx = e.get("max_score").and_then(|x| x.as_f64());
                match (l, mn, mx) {
                    (Some(l), Some(mn), Some(mx)) => Some(ScoreTier {
                        label: l.to_string(),
                        min_score: mn,
                        max_score: mx,
                    }),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(t) = tier {
            out.push(t);
        }
    }
    out
}

/// 评分矩阵派生（单一真源——纯函数，12,000 REQ 量级 O(N)；B2 五新面并入）。
///
/// - `评分项` → weight/max_score 取 `params.score`（缺失 → None，入 gaps）；
///   parent/tiers/objective/section 四槽位见 [`ScoringItem`] 文档；
/// - `否决项` → is_hard_gate=true（无分值；parent 等槽位照派生但主客观恒 false
///   ——否决无分值，主客观无意义）；
/// - 其余类别不进评分矩阵。
pub fn derive_scoring_items(reqs: &[Requirement]) -> Vec<ScoringItem> {
    reqs.iter()
        .filter_map(|r| match r.category.as_str() {
            "评分项" => {
                let weight = r.params.get("score").and_then(|v| v.as_f64());
                Some(ScoringItem {
                    requirement_id: r.id.clone(),
                    weight,
                    max_score: weight,
                    criteria: r
                        .anchors
                        .first()
                        .map(|a| a.excerpt.clone())
                        .unwrap_or_default(),
                    score_rules: r
                        .params
                        .get("score_rules")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    is_hard_gate: false,
                    parent_id: r
                        .params
                        .get("parent")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    tiers: tiers_from_params(r.params.get("tiers")),
                    is_objective: r
                        .params
                        .get("objective")
                        .and_then(|v| v.as_bool())
                        .unwrap_or_else(|| {
                            infer_objective(
                                r.anchors.first().map(|a| a.excerpt.as_str()).unwrap_or(""),
                            )
                        }),
                    // #224：params.section 单值 → RuleParsed 映射（share 恒 1.0，
                    // 与历史单映射行为逐位一致；多对多扩面由 store 持久化映射并入）
                    mappings: mappings_from_params(r.params.get("section")),
                    our_estimate: None, // 派生恒 None——SectionScorer 运行时填充
                })
            }
            "否决项" => Some(ScoringItem {
                requirement_id: r.id.clone(),
                weight: None,
                max_score: None,
                criteria: r
                    .anchors
                    .first()
                    .map(|a| a.excerpt.clone())
                    .unwrap_or_default(),
                score_rules: r
                    .params
                    .get("score_rules")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                is_hard_gate: true,
                parent_id: r
                    .params
                    .get("parent")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                tiers: tiers_from_params(r.params.get("tiers")),
                is_objective: false, // 否决项无分值——主客观无意义，恒 false
                mappings: mappings_from_params(r.params.get("section")),
                our_estimate: None,
            }),
            _ => None,
        })
        .collect()
}

/// #224：`params.section`（RuleParsed 单值）→ 映射列表（share 恒 1.0）。
fn mappings_from_params(v: Option<&serde_json::Value>) -> Vec<SectionMapping> {
    v.and_then(|v| v.as_str())
        .map(|s| {
            vec![SectionMapping {
                section_id: s.to_string(),
                weight_share: 1.0,
            }]
        })
        .unwrap_or_default()
}

/// 评分面健康度（sanity——分值缺失/总分越界登记，不阻断）：
/// - `scoring_gaps`：评分项无分值（分值缺失显式化——「满分 100 分」总分层误配
///   或表结构未覆盖，人工核对面）；
/// - `total_weight`：分值合计（评标办法惯例 ≤ 100——越界 = 疑误配，告警）。
pub fn scoring_sanity(items: &[ScoringItem]) -> (Vec<String>, f64) {
    let gaps: Vec<String> = items
        .iter()
        .filter(|i| !i.is_hard_gate && i.weight.is_none())
        .map(|i| {
            format!(
                "SCORING_GAP: {} 分值缺失（评分细则行无分值模式——人工核对）",
                i.requirement_id
            )
        })
        .collect();
    let total: f64 = items.iter().filter_map(|i| i.weight).sum();
    (gaps, total)
}

// ---------------------------------------------------------------- B2 任务 2：派生树

/// 评分树（平铺 requirements → 层级结构；B2 任务 2——「能算总分」的骨架）。
///
/// 派生态设计（与评分矩阵同哲学）：不落库、重算幂等。结构病（子和失配/孤儿/
/// 环/根合计失配）**不阻断建树**——树如实反映输入，病面由 [`ScoringTree::validate_sums`]
/// 显式登记（issue #94 §七：遇到结构千奇百怪的评分表，如实报告，不硬套）。
pub struct ScoringTree {
    /// 顶层节点（parent_id = None，首现序）
    pub roots: Vec<ScoringItem>,
    /// 扁平索引（id → item；重复 id 后者覆盖前者并登记——见 validate_sums）
    pub index: HashMap<String, ScoringItem>,
    /// 首现序（children 保序消费——HashMap 无序，另存 id 序列）
    order: Vec<String>,
}

/// 从 requirements 派生评分树（derive_scoring_items 的层级视图——同一真源）。
pub fn derive_scoring_tree(reqs: &[Requirement]) -> ScoringTree {
    let items = derive_scoring_items(reqs);
    let mut tree = ScoringTree {
        roots: Vec::new(),
        index: HashMap::new(),
        order: Vec::new(),
    };
    for item in items {
        let id = item.requirement_id.clone();
        let is_root = item.parent_id.is_none();
        // 重复 id：显式登记（DEDUP 面如实报告——后者覆盖 index 但不重复入 roots/order）
        if tree.index.contains_key(&id) {
            tree.order.push(format!("__DUP__{}", id));
            tree.index.insert(id, item);
            continue;
        }
        tree.order.push(id.clone());
        tree.index.insert(id, item.clone());
        if is_root {
            tree.roots.push(item);
        }
    }
    tree
}

impl ScoringTree {
    /// 某节点的直接子项（首现序——requirements 的原始顺序）。
    pub fn children(&self, id: &str) -> Vec<&ScoringItem> {
        self.order
            .iter()
            .filter_map(|oid| {
                let item = self.index.get(oid.strip_prefix("__DUP__").unwrap_or(oid))?;
                (item.parent_id.as_deref() == Some(id)).then_some(item)
            })
            .collect()
    }

    /// 某节点（含子树）的满分：
    /// - 叶（无子）→ 本项 `max_score`（None → 0.0——分值缺失已由 scoring_gaps/
    ///   validate_sums 登记，求和面不伪装）；
    /// - 枝（有子）→ Σ 子树满分（**声明值失效时以子和为准**——评标办法真实惯例：
    ///   子项细则才是打分依据，父项声明值是转写）；
    /// - id 不在树中 → 0.0（配合 validate_sums 的孤儿登记——调用方先验存在性）；
    /// - 环 → 环上节点贡献 0（递归 visited 防护——结构病已由 validate_sums 报 CYCLE）。
    pub fn max_score(&self, id: &str) -> f64 {
        let mut visited: HashSet<String> = HashSet::new();
        self.max_score_inner(id, &mut visited)
    }

    /// 递归内面（visited 环防护——同节点二次入即环，返回 0 停振）。
    fn max_score_inner(&self, id: &str, visited: &mut HashSet<String>) -> f64 {
        if !visited.insert(id.to_string()) {
            return 0.0; // 环——零贡献停振（CYCLE 已由 validate_sums 登记）
        }
        let Some(item) = self.index.get(id) else {
            return 0.0;
        };
        let kids = self.children(id);
        if kids.is_empty() {
            item.max_score.unwrap_or(0.0)
        } else {
            kids.iter()
                .map(|k| self.max_score_inner(&k.requirement_id, visited))
                .sum()
        }
    }

    /// 结构体检（四病显式登记，不阻断——「如实报告，不硬套」）：
    ///
    /// | 病 | 判定 | 报文示例 |
    /// |---|---|---|
    /// | 子和失配 | 枝节点 Σ子满分 ≠ 本项声明 max_score（±1e-9） | `SUM_MISMATCH: X 子和 28 ≠ 声明 30（差 -2）` |
    /// | 父无分值 | 枝节点本项 max_score 缺失（子有分值） | `PARENT_NO_SCORE: X 子和 55 但本项无声明值` |
    /// | 子无分值 | 枝的某子 max_score 缺失（贡献 0） | `CHILD_NO_SCORE: Y 在 X 下无分值（按 0 计）` |
    /// | 孤儿/环 | 从 roots 不可达（parent 不存在 / 成环） | `ORPHAN: Z parent=P 不存在` / `CYCLE: A→B→A` |
    /// | 重复 id | 同 id 双登记 | `DUPLICATE_ID: X（后者覆盖 index）` |
    /// | 总分惯例 | 根合计（去否决门）≠ 100 | `TOTAL_MISMATCH: 根合计 98 ≠ 100（惯例告警）` |
    pub fn validate_sums(&self) -> Vec<String> {
        let mut issues = Vec::new();
        // 重复 id
        for oid in &self.order {
            if let Some(dup) = oid.strip_prefix("__DUP__") {
                issues.push(format!(
                    "DUPLICATE_ID: {dup}（同 id 多次登记——index 取后者）"
                ));
            }
        }
        // 可达集（DFS from roots）
        let mut reachable: HashSet<String> = HashSet::new();
        let mut stack: Vec<String> = self
            .roots
            .iter()
            .map(|r| r.requirement_id.clone())
            .collect();
        while let Some(id) = stack.pop() {
            if !reachable.insert(id.clone()) {
                continue;
            }
            for k in self.children(&id) {
                stack.push(k.requirement_id.clone());
            }
        }
        // 不可达 → 孤儿（parent 不存在）或环（沿 parent 链回到自身）
        for oid in &self.order {
            let id = oid.strip_prefix("__DUP__").unwrap_or(oid);
            if reachable.contains(id) {
                continue;
            }
            let Some(item) = self.index.get(id) else {
                continue;
            };
            match &item.parent_id {
                Some(p) if !self.index.contains_key(p) => {
                    issues.push(format!(
                        "ORPHAN: {id} parent={p} 不存在（不挂树、不计总分）"
                    ));
                }
                Some(_) => {
                    // 环检测：沿 parent 链走 len+1 步，回到自身即环
                    let mut cur = id.to_string();
                    let mut is_cycle = false;
                    for _ in 0..=self.index.len() {
                        let next = self
                            .index
                            .get(cur.as_str())
                            .and_then(|it| it.parent_id.clone());
                        match next {
                            Some(n) if n == id => {
                                is_cycle = true;
                                break;
                            }
                            Some(n) => cur = n,
                            None => break,
                        }
                    }
                    if is_cycle {
                        issues.push(format!(
                            "CYCLE: {id} → … → {id}（parent 链成环，整支不可达）"
                        ));
                    } else {
                        issues.push(format!(
                            "ORPHAN: {id} 挂在不可达父 {}（环或孤儿链下游）",
                            item.parent_id.as_deref().unwrap_or("?")
                        ));
                    }
                }
                None => issues.push(format!(
                    "ORPHAN: {id} 无 parent 但不在 roots（不应发生——防御登记）"
                )),
            }
        }
        // 子和/父值/子值体检（全部节点遍历——可达与不可达都查，病面如实）
        for oid in &self.order {
            let id = oid.strip_prefix("__DUP__").unwrap_or(oid);
            let Some(item) = self.index.get(id) else {
                continue;
            };
            if item.is_hard_gate {
                continue; // 否决门无分值不参与求和
            }
            let kids = self.children(id);
            if kids.is_empty() {
                continue; // 叶：分值缺失由 scoring_gaps 管
            }
            let child_sum: f64 = kids.iter().map(|k| self.max_score(&k.requirement_id)).sum();
            for k in &kids {
                if !k.is_hard_gate && k.max_score.is_none() {
                    issues.push(format!(
                        "CHILD_NO_SCORE: {} 在 {} 下无分值（按 0 计入子和）",
                        k.requirement_id, id
                    ));
                }
            }
            match item.max_score {
                Some(declared) => {
                    if (child_sum - declared).abs() > 1e-9 {
                        issues.push(format!(
                            "SUM_MISMATCH: {id} 子项满分和 {child_sum} ≠ 本项声明 {declared}（差 {:+.}）",
                            child_sum - declared
                        ));
                    }
                }
                None => issues.push(format!(
                    "PARENT_NO_SCORE: {id} 子和 {child_sum} 但本项无声明值（总分层误配形态）"
                )),
            }
        }
        // 根合计惯例（去否决门；≠100 告警不阻断）
        let total: f64 = self
            .roots
            .iter()
            .filter(|r| !r.is_hard_gate)
            .map(|r| self.max_score(&r.requirement_id))
            .sum();
        if (total - 100.0).abs() > 1e-9 {
            issues.push(format!(
                "TOTAL_MISMATCH: 根满分合计 {total} ≠ 100（评标办法惯例——如实登记不阻断）"
            ));
        }
        issues
    }
}

// ---------------------------------------------------------------- B2 任务 3：自评

/// 字数达标线（字/分——30 分项需 1500 字；评标实务「每分至少半页」的保守折算）。
pub const WORDS_PER_POINT: f64 = 50.0;

/// 自评档位（issue #94 §三档位表——四维规则的判定结果）。
///
/// 估分口径：**档位区间中点**（优 95% / 良 82.5% / 中 65% / 差 27.5%——评委
/// 打分的期望居中假设；口径写入 rationale，可追溯可复核）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GradeTier {
    /// 优（90-100% 满分）：覆盖度 100% + 字数达标 + 有证据 + 有具体性
    Excellent,
    /// 良（75-90%）：覆盖度 ≥80% + 字数达标 + 有证据
    Good,
    /// 中（55-75%）：覆盖度 ≥60% + 字数达标
    Fair,
    /// 差（0-55%）：其余（含凑字数形态——覆盖缺失/无证据/无具体性）
    Poor,
}

impl GradeTier {
    /// 档位区间中点（估分比例）。
    pub fn ratio(self) -> f64 {
        match self {
            GradeTier::Excellent => 0.95,
            GradeTier::Good => 0.825,
            GradeTier::Fair => 0.65,
            GradeTier::Poor => 0.275,
        }
    }
    /// 档位标签。
    pub fn label(self) -> &'static str {
        match self {
            GradeTier::Excellent => "优",
            GradeTier::Good => "良",
            GradeTier::Fair => "中",
            GradeTier::Poor => "差",
        }
    }
}

/// 四维判定中间量（覆盖度/字数/证据/具体性——rationale 的可追溯数据面）。
#[derive(Debug, Clone, PartialEq)]
struct Dimension {
    /// 关键词命中率（0-1）
    coverage: f64,
    /// 命中/总关键词数
    hits: (usize, usize),
    /// 字数/达标线
    chars: (usize, f64),
    /// 是否引证据（业绩/资质/证明词面）
    evidence: bool,
    /// 是否具具体性（项目专名 或 数字+单位 ≥2 处）
    specificity: bool,
}

/// 价格分输入（[`SectionScorer::score_document`] 的价格面——价格分 = f(报价)，
/// 不做内容自评；`None` = 未报价，诚实登记不伪装 0 分满分）。
pub struct PriceInput<'a> {
    /// B1 价格分规格（benchmark 方法/偏离规则/满分——只调用不改）
    pub spec: &'a crate::pricing::PriceScoreSpec,
    /// 我方报价
    pub bid_price: f64,
    /// 其他有效报价（基准价计算面）
    pub others: Vec<f64>,
}

/// 单评分项的自评明细（B3 质量检查 / B4 生成循环的消费面——逐项可追溯）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemScore {
    pub item_id: String,
    /// 自评得分（max × 档位中点比例）
    pub estimated: f64,
    /// 满分
    pub max: f64,
    /// 档位
    pub tier: GradeTier,
    /// 覆盖度（0-1）
    pub coverage: f64,
    /// 客观项（业绩/资质——精确得分取决于证照数量，自评为提及性估计）
    pub is_objective: bool,
    /// 判定理由（四维明细——可追溯，非黑盒数字）
    pub rationale: String,
    /// #224 多对多摊分比（本章承载的权重占比；单映射 1.0——与历史行为一致）
    #[serde(default = "default_weight_share")]
    pub weight_share: f64,
    /// 评分细则原文（#186 预留面——ScoringItem.score_rules 透传；**判档
    /// 不消费**，供起草/审校对照细则。None = 无细则）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_rules: Option<String>,
}

/// #224：旧快照/旧调用方的 weight_share 缺省面（单映射口径）。
fn default_weight_share() -> f64 {
    1.0
}

/// 章节自评结果（issue #94 §二 struct + `item_scores` 明细扩展）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionScore {
    pub section_id: String,
    /// 能得几分
    pub estimated: f64,
    /// 满分
    pub max: f64,
    /// 命中的评分项（覆盖 ≥60% 即中档以上）
    pub covered_items: Vec<String>,
    /// 丢失的评分项（没写/覆盖 <60%）
    pub missing_items: Vec<String>,
    /// 判定理由（逐项明细汇总——可追溯）
    pub rationale: String,
    /// 逐项明细（B3/B4 消费面——估分聚合与档位追溯的单一真源）
    pub item_scores: Vec<ItemScore>,
}

/// 整本标书自评（issue #94 §二 struct + `notes` 诚实登记扩展）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentScore {
    /// 技术分合计（自评——主观项：方案/措施/服务类）
    pub technical: f64,
    /// 价格分（调 B1 [`crate::pricing::PriceScoreSpec`]——未报价 = 0 + note）
    pub price: f64,
    /// 商务分（客观项：业绩/资质/证照——提及性估计；树中无客观项 → None）
    pub commercial: Option<f64>,
    /// 总分 = technical + price + commercial
    pub total: f64,
    /// 各章节明细
    pub breakdown: Vec<SectionScore>,
    /// #224 未映射评分项（非空 = 总分可能偏低——响亮告警；同步 notes 保留字符串面）
    #[serde(default)]
    pub unmapped_scoring_items: Vec<UnmappedItem>,
    /// 诚实登记（未映射评分项 / 章节缺内容 / 未报价 / 价格类跳过——不伪装）
    pub notes: Vec<String>,
}

/// #224 未映射评分项的结构化登记（UNMAPPED 响亮告警面——旧 notes 字符串保留）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnmappedItem {
    pub req_id: String,
    /// criteria 摘要（截断——完整原文见评分项/规则锚）
    pub criteria: String,
    /// 分值（None = 分值也缺失——双重缺陷面，人工核对）
    pub weight: Option<f64>,
}

/// 章节自评器（B2 任务 3——**规则判定，非 LLM**：覆盖度/字数/证据/具体性四维
/// → 档位 → 估分；LLM 生成在 B4，本层只做可解释的确定性规则）。
pub struct SectionScorer {
    pub tree: ScoringTree,
    /// 项目专名（项目名/招标人/标段号——具体性判定锚词，调用方提供；
    /// 空表 = 无专名可用，具体性退化为「数字+单位」形态判定）。
    pub project_terms: Vec<String>,
}

/// 证据词面（「是否引用了相关证据」——业绩/资质/证明/附件引用形态）。
const EVIDENCE_WORDS: &[&str] = &[
    "业绩", "合同", "案例", "获奖", "证书", "资质", "认证", "资信", "证明",
    "复印件", "扫描件", "详见", "参见", "附件",
];

/// 具体性单位表（数字+单位形态——「数字+单位」出现 ≥2 处即具体）。
const SPECIFIC_UNITS: &[&str] = &[
    "万元", "亿", "台", "套", "件", "人", "年", "日历天", "%", "Gb", "GB",
    "MB", "小时", "分钟", "天", "平方米", "公里", "家",
];

/// 段级单字剥离集（前导/尾随循环剥——虚词与计分字：
/// 「含响应时限承诺」→「响应时限承诺」、「资格且无在建项目的加」→「资格且无在建项目」）。
const STRIP_CHARS: &[char] = &[
    '的', '与', '和', '及', '或', '含', '按', '须', '应', '为', '在', '等',
    '个', '项', '分', '级', '上', '中', '下', '以', '内', '外', '其', '该', '本',
    '每', '得', '加', '计', '扣', '减', '起',
];

/// 整词停用（计分副词碎片——「最多得」剥尾后形态）。
const STOP_WORDS: &[&str] = &["最多", "最少"];

/// 关键词组提取（确定性——标点/空白/ASCII 数字序列切段 + 连词切 +
/// 前导/尾随单字剥离 + 长段滑窗化）。
///
/// 组语义（coverage 的判定单位——一组内**任一**窗口命中即该组命中）：
/// - 不超过 12 字段 → 单元素组（整段即关键词：「售后服务方案」）；
/// - 超过 12 字段 → 4 字滑窗（步长 2，尾窗 ≥2 字）——真实评分细则的主语是
///   长句（「项目负责人具备一级注册建造师」13 字），整段要求内容逐字
///   复现过严；滑窗让「内容提及主题词（一级注册建造师）」即可命中，
///   同时窗口成组保留段的判定粒度（不因单窗口命中虚抬覆盖度以外的项）。
///
/// 数字作为分隔符的副作用恰为正例：`ISO9001` → 关键词 `ISO`，内容含
/// `ISO9001` 即子串命中。计分公式碎片（`最多`/`个得`）被停用/剥离滤除。
fn extract_keyword_groups(criteria: &str) -> Vec<Vec<String>> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    // 1. 标点/空白/ASCII 数字序列切段
    for seg in criteria.split(|c: char| {
        c.is_whitespace() || c.is_ascii_digit()
            || matches!(c, '，' | '。' | '；' | '：' | ';' | '、' | '（' | '）'
                | '(' | ')' | '/' | '「' | '」' | '《' | '》' | '！' | '？'
                | '+' | '=' | '×' | '－' | '—' | '-' | '.' | '·')
    }) {
        // 2. 连词切（与/和/及/或——「技术方案完整性与针对性」→ 两词）
        for piece in seg.split(['与', '和', '及', '或']) {
            // 3. 前导/尾随单字循环剥离
            let mut p: Vec<char> = piece.trim().chars().collect();
            loop {
                if p.is_empty() {
                    break;
                }
                if STRIP_CHARS.contains(&p[0]) {
                    p.remove(0);
                } else if STRIP_CHARS.contains(&p[p.len() - 1]) {
                    p.pop();
                } else {
                    break;
                }
            }
            let n = p.len();
            if n < 2 || STOP_WORDS.contains(&p.iter().collect::<String>().as_str()) {
                continue;
            }
            let group: Vec<String> = if n <= 12 {
                // 4a. 短段：整段单元素组
                vec![p.iter().collect()]
            } else {
                // 4b. 长段：4 字滑窗步长 2（尾窗 ≥2 字）
                let mut wins: Vec<String> = Vec::new();
                let mut i = 0;
                while i < n {
                    let end = (i + 4).min(n);
                    if end - i >= 2 {
                        let w: String = p[i..end].iter().collect();
                        if !wins.contains(&w) {
                            wins.push(w);
                        }
                    }
                    if end >= n {
                        break;
                    }
                    i += 2;
                }
                wins
            };
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
    }
    groups
}

/// 「数字+单位」形态计数（具体性判定的量化面——通用套话无参数，具体方案有）。
fn count_number_unit(content: &str) -> usize {
    let chars: Vec<char> = content.chars().collect();
    let mut n = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            // 数字序列
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // 紧随单位（跳空白）
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let rest: String = chars[i..].iter().take(4).collect();
            if SPECIFIC_UNITS.iter().any(|u| rest.starts_with(u)) {
                n += 1;
            }
        } else {
            i += 1;
        }
    }
    n
}

/// 价格类评分项识别（criteria 词面——价格分 = f(报价)，内容自评无意义，
/// 由 [`crate::pricing::PriceScoreSpec`] 管）。
fn is_price_item(item: &ScoringItem) -> bool {
    let c = &item.criteria;
    c.contains("价格分") || c.contains("投标报价") || c.contains("下浮率") || c.contains("基准价")
}

impl SectionScorer {
    /// 从评分树构造（`project_terms` = 项目专名锚词，可空）。
    pub fn new(tree: ScoringTree, project_terms: Vec<String>) -> Self {
        Self { tree, project_terms }
    }

    /// 四维判定（纯规则——覆盖度/字数/证据/具体性）。
    ///
    /// 覆盖度 = 命中组数 / 总组数（组语义见 [`extract_keyword_groups`]：
    /// 长细则滑窗组内任一窗口命中即组命中——理想标书提及主题词即覆盖）。
    fn assess(&self, item: &ScoringItem, content: &str) -> Dimension {
        let groups = extract_keyword_groups(&item.criteria);
        let hits = groups
            .iter()
            .filter(|g| g.iter().any(|w| content.contains(w.as_str())))
            .count();
        let coverage = if groups.is_empty() { 1.0 } else { hits as f64 / groups.len() as f64 };
        let min_chars = item.max_score.unwrap_or(10.0) * WORDS_PER_POINT;
        let n_chars = content.chars().filter(|c| !c.is_whitespace()).count();
        let evidence = EVIDENCE_WORDS.iter().any(|w| content.contains(w));
        let specificity = self.project_terms.iter().any(|t| content.contains(t.as_str()))
            || count_number_unit(content) >= 2;
        Dimension {
            coverage,
            hits: (hits, groups.len()),
            chars: (n_chars, min_chars),
            evidence,
            specificity,
        }
    }

    /// 档位判定（issue #94 §三档位表——从优到差短路，条件累进）。
    fn grade(d: &Dimension) -> GradeTier {
        let words_ok = d.chars.0 as f64 >= d.chars.1;
        if d.coverage >= 1.0 && words_ok && d.evidence && d.specificity {
            GradeTier::Excellent
        } else if d.coverage >= 0.8 && words_ok && d.evidence {
            GradeTier::Good
        } else if d.coverage >= 0.6 && words_ok {
            GradeTier::Fair
        } else {
            GradeTier::Poor
        }
    }

    /// 对某个章节打分（基于内容特征 + 档位判定——**规则非 LLM**）。
    ///
    /// 评分项集合：树中映射到 `section_id` 的**可达叶**（枝 = 容器不判定；
    /// 硬门跳过；价格类叶跳过——价格分由报价计算）。无映射项 → 空结果
    /// （rationale 显式说明，不伪装 0 分）。
    /// #224 多对多：同一评分项可映射多章节，按 [`SectionMapping::weight_share`]
    /// 摊分（本章该项满分 = 原满分 × share——同一项跨章各算一次导致总分虚高的
    /// 主防；单映射 share=1.0 与历史行为逐位一致）。
    pub fn score_section(&self, section_id: &str, content: &str) -> SectionScore {
        let items = self.leaf_items_for_section(section_id);
        let mut item_scores = Vec::new();
        let mut covered = Vec::new();
        let mut missing = Vec::new();
        let mut rationale_lines = Vec::new();
        if items.is_empty() {
            return SectionScore {
                section_id: section_id.to_string(),
                estimated: 0.0,
                max: 0.0,
                covered_items: vec![],
                missing_items: vec![],
                rationale: format!("无映射到本章节的评分项（section_id={section_id}——评分项 section 映射面为空，人工核对面）"),
                item_scores: vec![],
            };
        }
        for (item, share) in &items {
            if is_price_item(item) {
                rationale_lines.push(format!(
                    "[{}] 价格类评分项——内容自评无意义（价格分 = f(报价)，由 PriceScoreSpec 计算），跳过",
                    short_id(&item.requirement_id)
                ));
                continue;
            }
            let d = self.assess(item, content);
            let tier = Self::grade(&d);
            // #224 摊分：本章该项满分 = 原满分 × weight_share（多对多分摊）
            let max_full = item.max_score.unwrap_or(0.0);
            let max = (max_full * share * 100.0).round() / 100.0;
            let estimated = (max_full * share * tier.ratio() * 100.0).round() / 100.0; // 两位小数（评标惯例）
            let obj_note = if item.is_objective {
                "客观项——精确得分取决于证照/业绩数量，自评为提及性估计"
            } else {
                "主观项"
            };
            let share_note = if (*share - 1.0).abs() < 1e-9 {
                String::new()
            } else {
                format!(" 摊分{:.0}%（满分{}×share={}）", share * 100.0, max_full, max)
            };
            rationale_lines.push(format!(
                "[{}] 覆盖 {}/{}={:.0}% 字数 {}/{} {} 证据 {} 具体性 {} → {}（{:.1}%）{:.2}×{:.3}={:.2}（{}）{}",
                short_id(&item.requirement_id),
                d.hits.0, d.hits.1, d.coverage * 100.0,
                d.chars.0, d.chars.1 as usize, if d.chars.0 as f64 >= d.chars.1 { "✓" } else { "✗" },
                if d.evidence { "✓" } else { "✗" },
                if d.specificity { "✓" } else { "✗" },
                tier.label(), tier.ratio() * 100.0,
                max, tier.ratio(), estimated, obj_note, share_note
            ));
            if d.coverage >= 0.6 {
                covered.push(item.requirement_id.clone());
            } else {
                missing.push(item.requirement_id.clone());
            }
            item_scores.push(ItemScore {
                item_id: item.requirement_id.clone(),
                estimated,
                max,
                tier,
                coverage: (d.coverage * 10000.0).round() / 10000.0,
                is_objective: item.is_objective,
                rationale: rationale_lines.last().unwrap().clone(),
                weight_share: *share,
                score_rules: item.score_rules.clone(),
            });
        }
        let estimated: f64 = item_scores.iter().map(|s| s.estimated).sum();
        let max: f64 = items
            .iter()
            .filter(|(i, _)| !is_price_item(i))
            .map(|(i, share)| ((i.max_score.unwrap_or(0.0) * share) * 100.0).round() / 100.0)
            .sum();
        SectionScore {
            section_id: section_id.to_string(),
            estimated: (estimated * 100.0).round() / 100.0,
            max,
            covered_items: covered,
            missing_items: missing,
            rationale: rationale_lines.join("\n"),
            item_scores,
        }
    }

    /// 整本标书自评（逐章节 → 三段聚合：技术/商务/价格）。
    ///
    /// 聚合口径（有真实依据的结构映射，非字面魔法）：
    /// - **技术** = 主观叶（方案/措施/服务——评委判档打分面）估分和；
    /// - **商务** = 客观叶（证书/业绩/资信——数得出来）估分和；树中无客观叶
    ///   → `None`（无此面，不伪装 0 分面）；
    /// - **价格** = [`PriceInput`] 调 B1 `PriceScoreSpec::score`；`None` → 0 +
    ///   note 登记「未报价」。
    pub fn score_document(
        &self,
        sections: &[(String, String)],
        price: Option<PriceInput<'_>>,
    ) -> DocumentScore {
        let mut breakdown = Vec::new();
        let mut technical = 0.0;
        let mut commercial_acc: Option<f64> = None;
        let mut notes: Vec<String> = Vec::new();
        for (sec, content) in sections {
            let s = self.score_section(sec, content);
            if !s.item_scores.is_empty() {
                for it in &s.item_scores {
                    if it.is_objective {
                        *commercial_acc.get_or_insert(0.0) += it.estimated;
                    } else {
                        technical += it.estimated;
                    }
                }
            }
            breakdown.push(s);
        }
        // 未映射/缺内容叶登记（诚实状态——不计入不伪装）
        // #224：UNMAPPED 升结构化面（unmapped_scoring_items）+ 保留 notes 字符串
        let section_ids: Vec<&str> = sections.iter().map(|(s, _)| s.as_str()).collect();
        let unmapped_scoring_items = self.unmapped_items();
        if !unmapped_scoring_items.is_empty() {
            notes.push(format!(
                "UNMAPPED: {} 个评分项未映射章节（{}）——无法自评，总分可能偏低（section 槽位空，人工核对）",
                unmapped_scoring_items.len(),
                unmapped_scoring_items
                    .iter()
                    .map(|u| u.req_id.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        for item in self.reachable_leaves() {
            if item.is_hard_gate || is_price_item(item) {
                continue;
            }
            for m in &item.mappings {
                if !section_ids.contains(&m.section_id.as_str()) {
                    notes.push(format!(
                        "SECTION_MISSING: {} 映射 {} 但未提供该章节内容——按未写计",
                        item.requirement_id, m.section_id
                    ));
                }
            }
        }
        // #186 预留面显式化：细则已入树但判档不消费（rule_heuristic）——
        // 不伪装「按细则评分」（细则判档接入见 #186 第二步，依赖 #170）。
        let rules_n = self
            .reachable_leaves()
            .iter()
            .filter(|i| i.score_rules.is_some())
            .count();
        if rules_n > 0 {
            notes.push(format!(
                "SCORE_RULES_RESERVED: {rules_n} 项已登记评分细则——当前判档为关键词启发式（rule_heuristic），细则未参与判档（预留面，接入见 #186 第二步）"
            ));
        }
        // 价格分（B1 只调用不改）
        let price_score = match price {
            Some(p) => match p.spec.score(p.bid_price, &p.others) {
                Ok(v) => v,
                Err(e) => {
                    notes.push(format!("PRICE_ERROR: 价格分计算失败（{e:?}）——按 0 计"));
                    0.0
                }
            },
            None => {
                notes.push("NO_PRICE_INPUT: 未提供报价——价格分未计入（total 不含价格面）".to_string());
                0.0
            }
        };
        let commercial = commercial_acc;
        let total = technical + price_score + commercial.unwrap_or(0.0);
        DocumentScore {
            technical: (technical * 100.0).round() / 100.0,
            price: (price_score * 100.0).round() / 100.0,
            commercial: commercial.map(|c| (c * 100.0).round() / 100.0),
            total: (total * 100.0).round() / 100.0,
            breakdown,
            unmapped_scoring_items,
            notes,
        }
    }

    /// 映射到某章节的可达叶 + 摊分比（保序；硬门除外）。
    ///
    /// #224 多对多：`mappings` 中每个命中本章的条目产出 `(item, weight_share)`；
    /// 同一 item 理论上只产出一条（同 item 同章重复映射在 store 幂等键下不可
    /// 能——(req_id, section_id) 唯一；派生面 params.section 单值也不重复）。
    fn leaf_items_for_section(&self, section_id: &str) -> Vec<(&ScoringItem, f64)> {
        self.reachable_leaves()
            .into_iter()
            .filter(|i| !i.is_hard_gate)
            .filter_map(|i| {
                i.mappings
                    .iter()
                    .find(|m| m.section_id == section_id)
                    .map(|m| (i, m.weight_share))
            })
            .collect()
    }

    /// #224 未映射评分项（文档级告警面——score_section/score_document 出参共用）。
    /// 非硬门可达叶中 `mappings` 为空者（req_id + criteria 摘要 + weight）。
    /// 价格类叶不入（价格分 = f(报价)——不依赖章节映射，与 notes 循环同语义）。
    pub fn unmapped_items(&self) -> Vec<UnmappedItem> {
        self.reachable_leaves()
            .into_iter()
            .filter(|i| !i.is_hard_gate && !is_price_item(i) && i.mappings.is_empty())
            .map(|i| UnmappedItem {
                req_id: i.requirement_id.clone(),
                criteria: truncate_chars(&i.criteria, 60),
                weight: i.weight,
            })
            .collect()
    }

    /// 全部可达叶（roots DFS——孤儿/环不计；保首现序）。
    fn reachable_leaves(&self) -> Vec<&ScoringItem> {
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut stack: Vec<String> = self.tree.roots.iter().map(|r| r.requirement_id.clone()).collect();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let kids = self.children_of(&id);
            if kids.is_empty() {
                if let Some(item) = self.tree.index.get(&id) {
                    if !item.is_hard_gate {
                        out.push(item);
                    }
                }
            } else {
                for k in kids {
                    stack.push(k.requirement_id.clone());
                }
            }
        }
        // 保首现序（DFS 栈序不保序——按 order 重排）
        let mut ordered = Vec::new();
        for oid in &self.tree.order {
            if let Some(item) = self.tree.index.get(oid.strip_prefix("__DUP__").unwrap_or(oid)) {
                if out.iter().any(|o| o.requirement_id == item.requirement_id) {
                    ordered.push(item);
                }
            }
        }
        ordered
    }

    /// children 的内部转发（tree 字段可见性——同文件自由访问）。
    fn children_of(&self, id: &str) -> Vec<&ScoringItem> {
        self.tree.children(id)
    }
}

/// id 短形式（rationale 可读面：SYN-A-042a → 042a；非该前缀原样）。
fn short_id(id: &str) -> String {
    id.rsplit('-').next().unwrap_or(id).to_string()
}

/// 字符级截断（超长加省略号——criteria 摘要面；不切 UTF-8 边界）。
fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let cut: String = s.chars().take(n).collect();
    format!("{cut}……")
}

// ---------------------------------------------------------------- DomainStore 扩面
impl DomainStore {
    /// 评分矩阵（派生态只读——每次重算，幂等无落库）。
    /// #224：并入持久化映射（AgentSubmitted/Manual——多对多；params.section
    /// 的 RuleParsed 单值已在 derive 内进 mappings）。
    pub fn scoring_items(&self) -> Vec<ScoringItem> {
        let mut items = derive_scoring_items(&self.requirements);
        for m in &self.scoring_mappings {
            if let Some(it) = items
                .iter_mut()
                .find(|i| i.requirement_id == m.scoring_req_id)
            {
                if !it.mappings.iter().any(|x| x.section_id == m.section_id) {
                    it.mappings.push(SectionMapping {
                        section_id: m.section_id.clone(),
                        weight_share: m.weight_share,
                    });
                }
            }
        }
        items
    }

    /// 评分树（B2——层级视图；同评分矩阵同源派生，幂等）。
    pub fn scoring_tree(&self) -> ScoringTree {
        derive_scoring_tree(&self.requirements)
    }

    /// #224 映射账本（持久化只读——AgentSubmitted/Manual 的落点；
    /// RuleParsed 派生映射不入账，确定性重放）。
    pub fn scoring_mappings(&self) -> &[ScoringMapping] {
        &self.scoring_mappings
    }

    /// #224 登记评分映射（幂等键 = (scoring_req_id, section_id)——同键重
    /// 提 = 更新该条的非身份字段；新键 = 追加并签发 SM-xxxx）。
    ///
    /// 校验（fail-loud，不猜测）：
    /// - `scoring_req_id` 必须在 requirements 且是评分面（评分项/否决项
    ///   ——映射对其他类别无意义）；
    /// - `section_id` 必须在 sections（孤儿映射 = 半个账本）；
    /// - `weight_share` ∈ (0,1]（越界 = 权重语义破坏）；
    /// - `evidence_refs` 中每个 EV-id 必须在 evidences（幽灵引用拒收）。
    pub fn upsert_scoring_mapping(
        &mut self,
        scoring_req_id: String,
        section_id: String,
        weight_share: f64,
        evidence_refs: Vec<String>,
        source_anchor: crate::Anchor,
        created_from: MappingSource,
    ) -> Result<ScoringMapping, ToolError> {
        let bad = |reason: String| ToolError::EUnprocessable { reason };
        if !(0.0 < weight_share && weight_share <= 1.0) {
            return Err(bad(format!(
                "weight_share={weight_share} 越界（合法域 (0,1]——多章节分摊时各章占比之和 ≤ 1.0）"
            )));
        }
        let req = self
            .requirements
            .iter()
            .find(|r| r.id == scoring_req_id)
            .ok_or_else(|| ToolError::ENotFound {
                what: format!("评分项 {scoring_req_id} 不存在（先 extract_requirements）"),
            })?;
        if !matches!(req.category.as_str(), "评分项" | "否决项") {
            return Err(bad(format!(
                "REQ {scoring_req_id} 类别为 {}——仅评分项/否决项可建映射（其余类别不进评分面）",
                req.category
            )));
        }
        if !self.sections.iter().any(|s| s.id == section_id) {
            return Err(ToolError::ENotFound {
                what: format!("章节 {section_id} 不存在（先 build_outline）"),
            });
        }
        let ev_ids: std::collections::HashSet<&str> =
            self.evidences.iter().map(|e| e.id.as_str()).collect();
        if let Some(ev) = evidence_refs.iter().find(|e| !ev_ids.contains(e.as_str())) {
            return Err(ToolError::ENotFound {
                what: format!("证据 {ev} 不存在（先 register_evidence）"),
            });
        }
        // 幂等键命中 → 更新非身份字段；未命中 → 追加签新 id
        if let Some(existing) = self
            .scoring_mappings
            .iter_mut()
            .find(|m| m.scoring_req_id == scoring_req_id && m.section_id == section_id)
        {
            existing.weight_share = weight_share;
            existing.evidence_refs = evidence_refs;
            existing.source_anchor = source_anchor;
            existing.created_from = created_from;
            return Ok(existing.clone());
        }
        let m = ScoringMapping {
            id: self.next_id("SM"),
            scoring_req_id,
            section_id,
            weight_share,
            evidence_refs,
            source_anchor,
            created_from,
        };
        self.scoring_mappings.push(m.clone());
        Ok(m)
    }

    /// 硬门清单（否决项 REQ id——T-3 验收「植入 1 个否决项漏响应 → 阻断导出」的输入面；
    /// 阻断裁决在 `check_export_ready`，此处只供核对/展示）。
    pub fn hard_gate_requirement_ids(&self) -> Vec<String> {
        self.scoring_items()
            .iter()
            .filter(|i| i.is_hard_gate)
            .map(|i| i.requirement_id.clone())
            .collect()
    }
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParseStatus;
    use std::collections::HashMap;

    fn scoring_req(id: &str, score: Option<f64>, excerpt: &str) -> Requirement {
        let mut params = HashMap::new();
        if let Some(s) = score {
            params.insert("score".to_string(), serde_json::json!(s));
        }
        Requirement {
            id: id.into(),
            category: "评分项".into(),
            clause_no: None,
            anchors: vec![crate::Anchor {
                file: "A".into(),
                kind: "para".into(),
                locator: "评标办法".into(),
                excerpt: excerpt.into(),
            }],
            params,
            source_file: "A".into(),
        }
    }

    fn veto_req(id: &str) -> Requirement {
        Requirement {
            id: id.into(),
            category: "否决项".into(),
            clause_no: None,
            anchors: vec![crate::Anchor {
                file: "A".into(),
                kind: "para".into(),
                locator: "须知".into(),
                excerpt: "投标文件未经法定代表人签字的将被否决".into(),
            }],
            params: HashMap::new(),
            source_file: "A".into(),
        }
    }

    /// T-3 验收：20+ 评分项含分值 + 否决项 hard gate + 总分 sane + 缺失登记。
    #[test]
    fn scoring_matrix_extracts_20_plus_items_with_weights() {
        let mut reqs: Vec<Requirement> = (0..24)
            .map(|i| {
                scoring_req(
                    &format!("REQ-{:04}", i + 3),
                    Some((i % 5 + 1) as f64),
                    &format!("评分细则第{}项：技术响应与偏离（{}分）", i + 1, (i % 5 + 1)),
                )
            })
            .collect();
        reqs.push(veto_req("REQ-0027"));
        reqs.push(scoring_req(
            "REQ-0028",
            None,
            "评标基准价 = 满足要求的最低价为基准",
        )); // 算法行无分值

        let items = derive_scoring_items(&reqs);
        assert_eq!(items.len(), 26, "24 评分项 + 1 否决项 + 1 无分值算法行");
        let with_weight = items.iter().filter(|i| i.weight.is_some()).count();
        assert_eq!(with_weight, 24, "T-3 验收：20+ 评分项含分值");
        let hard = items.iter().filter(|i| i.is_hard_gate).count();
        assert_eq!(hard, 1, "否决项 = 硬门");
        let veto = items.iter().find(|i| i.is_hard_gate).unwrap();
        assert!(veto.criteria.contains("否决"), "标准原文随行");
        assert_eq!(veto.weight, None, "否决项无分值");
        // B2 新面默认值（向后兼容——旧输入无槽位时全空不伪装）
        assert!(items
            .iter()
            .all(|i| i.parent_id.is_none() && i.tiers.is_empty()));
        assert!(
            items.iter().all(|i| i.our_estimate.is_none()),
            "派生恒 None"
        );

        // sanity：缺失登记 + 总分
        let (gaps, total) = scoring_sanity(&items);
        assert_eq!(gaps.len(), 1, "无分值算法行显式登记不伪装");
        assert!(gaps[0].contains("REQ-0028"));
        let expect_total: f64 = (0..24).map(|i| (i % 5 + 1) as f64).sum();
        assert!(
            (total - expect_total).abs() < 1e-9,
            "总分 = Σ 分值（{total}）"
        );
        assert!(total <= 100.0, "评标办法总分惯例 ≤100——sanity 面不越界");
    }

    /// store 面接线：抽取 → 评分矩阵派生 + 硬门清单。
    #[test]
    fn store_scoring_derivation_and_hard_gates() {
        let mut s = DomainStore::new();
        s.create_project("评分项目", "C-SCR", "明标", "scr-key")
            .unwrap();
        s.upsert_file(
            "A.docx",
            "招标文件",
            "sha-s",
            5,
            1,
            50,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
        let reqs = vec![
            scoring_req("REQ-0003", Some(65.0), "1、报价：65 分（基准价 = 最低价）"),
            scoring_req("REQ-0004", Some(30.0), "2、技术方案：30 分"),
            veto_req("REQ-0005"),
        ];
        s.set_requirements(reqs).unwrap();
        let items = s.scoring_items();
        assert_eq!(items.len(), 3);
        let (_, total) = scoring_sanity(&items);
        assert!((total - 95.0).abs() < 1e-9);
        assert_eq!(s.hard_gate_requirement_ids(), vec!["REQ-0005".to_string()]);
        // 幂等：双派生全等（派生态无隐藏可变性）
        assert_eq!(s.scoring_items(), items);
        // B2：树视图同源派生（roots 平铺三节点 + 硬门在树不计分）
        let tree = s.scoring_tree();
        assert_eq!(tree.roots.len(), 3);
        assert!((tree.max_score("REQ-0004") - 30.0).abs() < 1e-9);
    }

    // ============ B2 任务 1：档位解析 ============

    /// 档位文本解析：分号/逗号双分隔 × 全角/波浪线分隔符 × 无"分"字形态。
    #[test]
    fn parse_tiers_real_forms() {
        let t1 = parse_tiers("优：9-10 分；良：7-9 分；中：5-7 分；差：0-5 分");
        assert_eq!(t1.len(), 4);
        assert_eq!(
            t1[0],
            ScoreTier {
                label: "优".into(),
                min_score: 9.0,
                max_score: 10.0
            }
        );
        assert_eq!(
            t1[3],
            ScoreTier {
                label: "差".into(),
                min_score: 0.0,
                max_score: 5.0
            }
        );

        // 好中差档（真实评标办法亦常见好/较好/一般标签）+ 小数 + 无分字
        let t2 = parse_tiers("好 9～10分，较好 7～8.9分，一般 5～6.9分，差 0～4.9分");
        assert_eq!(t2.len(), 4, "四档全解");
        assert_eq!(t2[1].label, "较好");
        assert!((t2[1].min_score - 7.0).abs() < 1e-9 && (t2[1].max_score - 8.9).abs() < 1e-9);

        // 非档位文本 → 空表（显式，不伪装）
        assert!(parse_tiers("技术方案完整性与针对性 30 分").is_empty());
        assert!(parse_tiers("").is_empty());
    }

    /// params.tiers 双形态（数组/对象）+ 无槽位默认空表。
    #[test]
    fn tiers_from_params_dual_form() {
        let mut p1 = HashMap::new();
        p1.insert(
            "tiers".to_string(),
            serde_json::json!([["优", 9.0, 10.0], ["良", 7.5, 9.0]]),
        );
        let mut p2 = HashMap::new();
        p2.insert(
            "tiers".to_string(),
            serde_json::json!([
                {"label": "优", "min_score": 9, "max_score": 10},
                {"label": "良", "min_score": 7.5, "max_score": 9},
            ]),
        );
        let mut reqs = vec![
            scoring_req("R1", Some(10.0), "档位项"),
            scoring_req("R2", Some(10.0), "档位项"),
            scoring_req("R3", Some(10.0), "无槽位项"),
        ];
        reqs[0].params = p1;
        reqs[1].params = p2;
        let items = derive_scoring_items(&reqs);
        assert_eq!(items[0].tiers.len(), 2, "数组形态 [label,min,max]");
        assert_eq!(items[0].tiers[0].label, "优");
        assert_eq!(
            items[1].tiers.len(),
            2,
            "对象形态 {{label,min_score,max_score}}"
        );
        assert!((items[1].tiers[1].min_score - 7.5).abs() < 1e-9);
        assert!(items[2].tiers.is_empty(), "无槽位 → 空表不伪装");
    }

    // ============ B2 任务 1：主客观启发 ============

    /// 主客观启发：证书/可数计分 → 客观；评审形容词/档位 → 主观（默认主观）。
    #[test]
    fn objective_inference_real_criteria() {
        // 客观：证书/资质/等级（synth-tender 4.3 商务分原句）
        assert!(infer_objective("企业认证体系（ISO9001）1 分"));
        assert!(infer_objective("银行资信等级 AAA 级 2 分"));
        assert!(infer_objective(
            "项目负责人具备一级注册建造师（机电工程）资格且无在建项目的加 3 分"
        ));
        // 客观：可数计分（每 X 得/加 N 分）
        assert!(infer_objective(
            "近三年完成的合同额 500 万元（含）以上类似工程业绩，每提供 1 个得 2 分，最多得 6 分"
        ));
        assert!(infer_objective(
            "项目团队具有高级网络工程师职称证书的每人 1 分，最多 3 分"
        ));
        // 主观：评审形容词（synth-tender 4.2 技术分原句）
        assert!(!infer_objective("技术方案完整性与针对性 30 分"));
        assert!(!infer_objective("售后服务方案（含响应时限承诺）8 分"));
        // 主观：档位形态
        assert!(!infer_objective("优 9-10 分；良 7-9 分"));
        // 默认：无特征 → 主观（保守默认）
        assert!(!infer_objective("报价得分计算"));
    }

    /// 显式 params.objective 优先于启发（extract 层可覆写）。
    #[test]
    fn explicit_objective_overrides_inference() {
        let mut p = HashMap::new();
        p.insert("score".to_string(), serde_json::json!(10.0));
        p.insert("objective".to_string(), serde_json::json!(false));
        let mut r = scoring_req("R1", Some(10.0), "具备 ISO9001 证书 10 分");
        r.params = p;
        let items = derive_scoring_items(&[r]);
        assert!(!items[0].is_objective, "显式 false 压过启发 true");
    }

    // ============ B2 任务 2：派生树 ============

    /// synth-tender 评分表全形态树派生（PR1 验收对照表用例——A §第四章原句）。
    ///
    /// 手工树（spec 单一真源 scripts/synth_tender_spec.py L209-214）：
    /// ```text
    /// 价格分 30（4.1，叶）
    /// 技术分 55（4.2）→ 方案完整性 30 / 实施进度 10 / 售后 8 / 信息安全 7
    /// 项目团队 10（4.4，计入技术分）→ 建造师 3 / 高职称证 3 / 经理业绩 2
    /// 商务分 15（4.3）→ 业绩 6 / ISO9001 1 / ISO27001 1 / AAA 2 / 奖项 3 / 团队职称 2
    /// ```
    /// 注意两处**真实结构病**（引擎须如实检出，不硬套）：
    /// ① 4.4 子和 3+3+2=8 ≠ 声明 10（SUM_MISMATCH）；
    /// ② 4.4「计入技术分」挂技术分下 → 技术分子树和 55+8=63，根合计 63+30+15=108 ≠ 100
    ///    （合成文件自身的不自洽——对照表如实呈现）。
    fn synth_tender_reqs() -> Vec<Requirement> {
        let req = |id: &str, score: Option<f64>, parent: Option<&str>, excerpt: &str| {
            let mut params = HashMap::new();
            if let Some(s) = score {
                params.insert("score".to_string(), serde_json::json!(s));
            }
            if let Some(p) = parent {
                params.insert("parent".to_string(), serde_json::json!(p));
            }
            Requirement {
                id: id.into(),
                category: "评分项".into(),
                clause_no: None,
                anchors: vec![crate::Anchor {
                    file: "A".into(),
                    kind: "para".into(),
                    locator: "第四章 评标办法".into(),
                    excerpt: excerpt.into(),
                }],
                params,
                source_file: "A".into(),
            }
        };
        vec![
            req("SYN-A-041", Some(30.0), None, "4.1 价格分（30 分）：以通过初步评审的有效投标报价的算术平均值下浮 5% 为基准价，价格分＝（基准价／投标报价）×30"),
            req("SYN-A-042", Some(55.0), None, "4.2 技术分（55 分）"),
            req("SYN-A-042a", Some(30.0), Some("SYN-A-042"), "技术方案完整性与针对性 30 分"),
            req("SYN-A-042b", Some(10.0), Some("SYN-A-042"), "项目实施方案与进度保障措施 10 分"),
            req("SYN-A-042c", Some(8.0), Some("SYN-A-042"), "售后服务方案（含响应时限承诺）8 分"),
            req("SYN-A-042d", Some(7.0), Some("SYN-A-042"), "信息安全与等保符合性方案 7 分"),
            req("SYN-A-044", Some(10.0), Some("SYN-A-042"), "4.4 项目团队评分（10 分，计入技术分）"),
            req("SYN-A-044a", Some(3.0), Some("SYN-A-044"), "项目负责人具备一级注册建造师（机电工程）资格且无在建项目的加 3 分"),
            req("SYN-A-044b", Some(3.0), Some("SYN-A-044"), "项目团队具有高级网络工程师或高级信息系统项目管理师职称证书的每人 1 分，最多 3 分"),
            req("SYN-A-044c", Some(2.0), Some("SYN-A-044"), "拟派项目经理具有同类项目管理业绩的每个 1 分，最多 2 分"),
            req("SYN-A-043", Some(15.0), None, "4.3 商务分（15 分）"),
            req("SYN-A-043a", Some(6.0), Some("SYN-A-043"), "近三年完成的合同额 500 万元（含）以上类似工程业绩，每提供 1 个得 2 分，最多得 6 分"),
            req("SYN-A-043b", Some(1.0), Some("SYN-A-043"), "企业认证体系（ISO9001）1 分"),
            req("SYN-A-043c", Some(1.0), Some("SYN-A-043"), "企业认证体系（ISO27001）1 分"),
            req("SYN-A-043d", Some(2.0), Some("SYN-A-043"), "银行资信等级 AAA 级 2 分"),
            req("SYN-A-043e", Some(3.0), Some("SYN-A-043"), "类似项目获省级以上奖项每个 1 分，最多 3 分"),
            req("SYN-A-043f", Some(2.0), Some("SYN-A-043"), "拟派项目团队中级以上职称人员每人 1 分，最多 2 分"),
        ]
    }

    /// 树派生：roots/children/首现序 + max_score 子树和（对照表核心用例）。
    #[test]
    fn tree_derives_synth_tender_hierarchy() {
        let tree = derive_scoring_tree(&synth_tender_reqs());
        // 手工：roots = 价格 30 / 技术 55 / 商务 15（4.4 计入技术分非根）
        assert_eq!(tree.roots.len(), 3, "4.4 挂技术分下不占根");
        assert_eq!(
            tree.roots
                .iter()
                .map(|r| r.requirement_id.clone())
                .collect::<Vec<_>>(),
            vec!["SYN-A-041", "SYN-A-042", "SYN-A-043"],
            "roots 首现序"
        );
        // 手工：技术分 children = 4.2 四子 + 4.4（5 项）
        let kids = tree.children("SYN-A-042");
        assert_eq!(kids.len(), 5);
        assert_eq!(kids[0].requirement_id, "SYN-A-042a", "children 首现序");
        assert_eq!(kids[4].requirement_id, "SYN-A-044");
        // 手工：max_score 子树和
        //   4.2a 叶 = 30（声明值）
        assert!(
            (tree.max_score("SYN-A-042a") - 30.0).abs() < 1e-9,
            "叶 = 声明值"
        );
        //   4.4 = 3+3+2 = 8（子和 ≠ 声明 10——按子和，声明值失配由 validate 检出）
        assert!((tree.max_score("SYN-A-044") - 8.0).abs() < 1e-9, "子和为准");
        //   技术分子树 = 4.2 四子 55 + 4.4 子树 8 = 63（「计入技术分」挂 042 下）
        assert!(
            (tree.max_score("SYN-A-042") - 63.0).abs() < 1e-9,
            "4.2 子树和 = 55 + 8 = 63"
        );
        // 手工：商务分 = 6+1+1+2+3+2 = 15
        assert!((tree.max_score("SYN-A-043") - 15.0).abs() < 1e-9);
        // 主客观启发随树派生
        let iso = tree.index.get("SYN-A-043b").unwrap();
        assert!(iso.is_objective, "ISO9001 证书 = 客观");
        let scheme = tree.index.get("SYN-A-042a").unwrap();
        assert!(!scheme.is_objective, "方案完整性针对性 = 主观");
    }

    /// validate_sums 四病全检：子和失配 / 父无分值 / 子无分值 / 孤儿 / 环 / 重复。
    #[test]
    fn validate_sums_catches_all_four_diseases() {
        let req = |id: &str, score: Option<f64>, parent: Option<&str>| {
            let mut params = HashMap::new();
            if let Some(s) = score {
                params.insert("score".to_string(), serde_json::json!(s));
            }
            if let Some(p) = parent {
                params.insert("parent".to_string(), serde_json::json!(p));
            }
            Requirement {
                id: id.into(),
                category: "评分项".into(),
                clause_no: None,
                anchors: vec![crate::Anchor {
                    file: "A".into(),
                    kind: "para".into(),
                    locator: "评标办法".into(),
                    excerpt: format!("{id} 评分项"),
                }],
                params,
                source_file: "A".into(),
            }
        };
        // 病①子和失配：父声明 10，子和 3+3=6
        // 病②父无分值：父 P2 无 score，子和 5+5=10
        // 病③子无分值：P3 下 C3a 无 score（按 0 计）
        // 病④孤儿：O1 parent=GHOST 不存在；环：Z1→Z2→Z1
        // 病⑤重复 id：D1 双登记
        let reqs = vec![
            req("P1", Some(10.0), None),
            req("P1a", Some(3.0), Some("P1")),
            req("P1b", Some(3.0), Some("P1")),
            req("P2", None, None),
            req("P2a", Some(5.0), Some("P2")),
            req("P2b", Some(5.0), Some("P2")),
            req("P3", Some(10.0), None),
            req("P3a", None, Some("P3")),
            req("P3b", Some(10.0), Some("P3")),
            req("O1", Some(2.0), Some("GHOST")),
            req("Z1", Some(1.0), Some("Z2")),
            req("Z2", Some(1.0), Some("Z1")),
            req("D1", Some(4.0), None),
            req("D1", Some(4.0), None),
        ];
        let tree = derive_scoring_tree(&reqs);
        let issues = tree.validate_sums();
        let joined = issues.join("\n");
        assert!(
            joined.contains("SUM_MISMATCH: P1 子项满分和 6 ≠ 本项声明 10"),
            "病①子和失配\n{joined}"
        );
        assert!(
            joined.contains("PARENT_NO_SCORE: P2 子和 10 但本项无声明值"),
            "病②父无分值\n{joined}"
        );
        assert!(
            joined.contains("CHILD_NO_SCORE: P3a 在 P3 下无分值"),
            "病③子无分值\n{joined}"
        );
        assert!(
            joined.contains("ORPHAN: O1 parent=GHOST 不存在"),
            "病④孤儿\n{joined}"
        );
        assert!(joined.contains("CYCLE: Z1"), "病④环\n{joined}");
        assert!(joined.contains("DUPLICATE_ID: D1"), "病⑤重复 id\n{joined}");
        assert!(joined.contains("TOTAL_MISMATCH"), "总分惯例告警\n{joined}");
        // 健康部分不受污染：P3 子树和 = 0 + 10 = 10 = 声明（P3 无 SUM_MISMATCH）
        assert!(
            !joined.contains("SUM_MISMATCH: P3"),
            "P3 声明一致不误报\n{joined}"
        );
        // 孤儿/环不计根合计：根 = P1(6) + P2(10) + P3(10) + D1(4) = 30
        assert!((tree.max_score("P1") - 6.0).abs() < 1e-9);
    }

    /// 健康树零报文（多级三层 + 档位 + 章节映射 + 自评占位）。
    #[test]
    fn healthy_tree_passes_validation_silent() {
        let req = |id: &str, score: f64, parent: Option<&str>, extra: &str| {
            let mut params = HashMap::new();
            params.insert("score".to_string(), serde_json::json!(score));
            if let Some(p) = parent {
                params.insert("parent".to_string(), serde_json::json!(p));
            }
            if !extra.is_empty() {
                params.insert("section".to_string(), serde_json::json!(extra));
            }
            Requirement {
                id: id.into(),
                category: "评分项".into(),
                clause_no: None,
                anchors: vec![crate::Anchor {
                    file: "A".into(),
                    kind: "para".into(),
                    locator: "评标办法".into(),
                    excerpt: format!("{id} 评分项"),
                }],
                params,
                source_file: "A".into(),
            }
        };
        // 技术分 30（10+10+10 三子项各 10——issue #94 §五验收形态）/ 商务 20 /
        // 价格 40 / 其他 10 —— 根合计 100（TOTAL_MISMATCH 面零告警）
        let reqs = vec![
            req("S-TECH", 30.0, None, ""),
            req("S-TECH-a", 10.0, Some("S-TECH"), "SEC-03"),
            req("S-TECH-b", 10.0, Some("S-TECH"), "SEC-04"),
            req("S-TECH-c", 10.0, Some("S-TECH"), "SEC-05"),
            req("S-COMM", 20.0, None, ""),
            req("S-PRICE", 40.0, None, "SEC-02"),
            req("S-OTHER", 10.0, None, ""),
        ];
        let tree = derive_scoring_tree(&reqs);
        assert!(tree.validate_sums().is_empty(), "健康树零报文");
        assert!((tree.max_score("S-TECH") - 30.0).abs() < 1e-9);
        assert!((tree.max_score("S-PRICE") - 40.0).abs() < 1e-9);
        // 章节映射随树派生（自评对接面；#224 多对多形态——RuleParsed share=1.0）
        let a = tree.index.get("S-TECH-a").unwrap();
        assert_eq!(a.mappings.len(), 1);
        assert_eq!(a.mappings[0].section_id, "SEC-03");
        assert!((a.mappings[0].weight_share - 1.0).abs() < 1e-9);
        // our_estimate 派生恒 None（任务 3 填充）
        assert!(tree.index.values().all(|i| i.our_estimate.is_none()));
    }

    /// PR1 验收对照表（synth-tender 全表：手工推导 vs 引擎输出——打印人工核对）。
    #[test]
    fn acceptance_report_synth_tender_tree() {
        let tree = derive_scoring_tree(&synth_tender_reqs());
        // 手工全表（spec 原句推导，见 tree_derives_synth_tender_hierarchy 文档）
        let manual: Vec<(&str, f64)> = vec![
            ("SYN-A-041 价格分", 30.0),      // 叶
            ("SYN-A-042a 方案完整性", 30.0), // 叶
            ("SYN-A-042b 实施进度", 10.0),
            ("SYN-A-042c 售后", 8.0),
            ("SYN-A-042d 信息安全", 7.0),
            ("SYN-A-042 技术分子树和", 63.0), // 55 + 8（4.4 计入挂其下）
            ("SYN-A-044a 建造师", 3.0),
            ("SYN-A-044b 高职称证", 3.0),
            ("SYN-A-044c 经理业绩", 2.0),
            ("SYN-A-044 团队（子和≠声明）", 8.0), // 3+3+2=8 vs 声明 10
            ("SYN-A-043a 业绩", 6.0),
            ("SYN-A-043b ISO9001", 1.0),
            ("SYN-A-043c ISO27001", 1.0),
            ("SYN-A-043d AAA", 2.0),
            ("SYN-A-043e 奖项", 3.0),
            ("SYN-A-043f 团队职称", 2.0),
            ("SYN-A-043 商务分（子和）", 15.0), // 6+1+1+2+3+2
        ];
        let engine: Vec<(&str, f64)> = vec![
            ("SYN-A-041", tree.max_score("SYN-A-041")),
            ("SYN-A-042a", tree.max_score("SYN-A-042a")),
            ("SYN-A-042b", tree.max_score("SYN-A-042b")),
            ("SYN-A-042c", tree.max_score("SYN-A-042c")),
            ("SYN-A-042d", tree.max_score("SYN-A-042d")),
            ("SYN-A-042", tree.max_score("SYN-A-042")),
            ("SYN-A-044a", tree.max_score("SYN-A-044a")),
            ("SYN-A-044b", tree.max_score("SYN-A-044b")),
            ("SYN-A-044c", tree.max_score("SYN-A-044c")),
            ("SYN-A-044", tree.max_score("SYN-A-044")),
            ("SYN-A-043a", tree.max_score("SYN-A-043a")),
            ("SYN-A-043b", tree.max_score("SYN-A-043b")),
            ("SYN-A-043c", tree.max_score("SYN-A-043c")),
            ("SYN-A-043d", tree.max_score("SYN-A-043d")),
            ("SYN-A-043e", tree.max_score("SYN-A-043e")),
            ("SYN-A-043f", tree.max_score("SYN-A-043f")),
            ("SYN-A-043", tree.max_score("SYN-A-043")),
        ];
        let mut mismatches = 0;
        for (i, ((mlabel, mval), (elabel, eval_))) in manual.iter().zip(engine.iter()).enumerate() {
            assert_eq!(mlabel.chars().next(), elabel.chars().next(), "行 {i} 对齐");
            if (mval - eval_).abs() > 1e-9 {
                mismatches += 1;
                eprintln!("MISMATCH {elabel}: manual={mval} engine={eval_}");
            }
        }
        assert_eq!(mismatches, 0, "手工 vs 引擎 17 行全一致（1e-9）");
        // 结构病如实检出（两处——见函数文档）
        let issues = tree.validate_sums();
        let joined = issues.join("\n");
        assert!(
            joined.contains("SUM_MISMATCH: SYN-A-044 子项满分和 8 ≠ 本项声明 10"),
            "4.4 子和病如实报\n{joined}"
        );
        assert!(
            joined.contains("TOTAL_MISMATCH"),
            "根合计 108 ≠ 100 如实报\n{joined}"
        );
        // 引擎侧重述（PR 对照表素材）：
        eprintln!("---- PR1 验收对照（synth-tender）----");
        for (l, v) in &manual {
            eprintln!("  {l}: {v}");
        }
        eprintln!(
            "根合计 = {}",
            tree.roots
                .iter()
                .filter(|r| !r.is_hard_gate)
                .map(|r| tree.max_score(&r.requirement_id))
                .sum::<f64>()
        );
        for i in &issues {
            eprintln!("  [检出] {i}");
        }
    }

    // ============ B2 任务 3：自评 ============

    /// 关键词组提取与具体性计数的单元面（判定规则可追溯的最底层）。
    #[test]
    fn keyword_extraction_and_specificity_units() {
        // 组打平视图（断言可读面——组内窗口用 | 连接）
        let flat = |c: &str| -> Vec<String> {
            extract_keyword_groups(c)
                .iter()
                .map(|g| g.join("|"))
                .collect()
        };
        // 连词切 + 数字切段 + 短段整段组
        assert_eq!(
            flat("技术方案完整性与针对性 30 分"),
            vec!["技术方案完整性".to_string(), "针对性".to_string()]
        );
        // ISO9001 → 数字切开留 ISO（内容含 ISO9001 即子串命中）
        let kws2 = flat("企业认证体系（ISO9001）1 分");
        assert!(kws2.contains(&"企业认证体系".to_string()) && kws2.contains(&"ISO".to_string()));
        // 括号切段 + 前导「含」剥离
        assert_eq!(
            flat("售后服务方案（含响应时限承诺）8 分"),
            vec!["售后服务方案".to_string(), "响应时限承诺".to_string()]
        );
        // 尾随计分字/虚词剥离：「资格且无在建项目的加」→「资格且无在建项目」
        assert_eq!(
            flat("项目负责人具备一级注册建造师（机电工程）资格且无在建项目的加 3 分")
                .last()
                .unwrap(),
            "资格且无在建项目"
        );
        // 长段滑窗化（>12 字不整段保留——窗口 ≤4 字；计分碎片滤除）
        let long = extract_keyword_groups(
            "近三年完成的合同额 500 万元（含）以上类似工程业绩，每提供 1 个得 2 分，最多得 6 分",
        );
        let joined = long.iter().map(|g| g.join("|")).collect::<Vec<_>>();
        // 「近三年完成的合同额」9 字 → 整段保留；「以上类似工程业绩」剥前导后 → 「类似工程业绩」
        assert!(joined.contains(&"近三年完成的合同额".to_string()), "{joined:?}");
        assert!(joined.contains(&"类似工程业绩".to_string()), "{joined:?}");
        // 「最多得」→ 剥尾「得」→「最多」→ 整词停用滤除；「个得」→ 剥空滤除
        assert!(!joined.iter().any(|j| j.contains("最多") || j.contains("个得")), "{joined:?}");
        // 任何关键词 ≤12 字（长段已滑窗化，无整段长串）
        assert!(long
            .iter()
            .all(|g| g.iter().all(|w| w.chars().count() <= 12)));
        // 数字不混入关键词（数字是分隔符）
        assert!(!long.iter().any(|g| g.iter().any(|w| w.contains('5'))), "{joined:?}");
        // >12 字段 → 多窗口组（组内任一命中即组命中）
        let g14 = extract_keyword_groups("拟派项目经理具有同类项目管理业绩的每个 1 分，最多 2 分");
        assert!(!g14.is_empty() && g14[0].len() > 1, "长段窗口组：{:?}", g14);
        assert!(g14[0].iter().all(|w| w.chars().count() <= 4), "窗口 ≤4 字");
        // 数字+单位计数（具体性）
        assert_eq!(count_number_unit("预算 1650 万元，工期 180 日历天"), 2);
        assert_eq!(count_number_unit("我司高度重视，经验丰富，服务周到"), 0);
    }

    /// issue #94 §五验收形态的评分表构造器：
    /// 技术分 30（三子项各 10）/ 商务分 20（两客观子项各 10）/ 价格分 40（Ratio 口径）。
    fn b2_form_reqs() -> Vec<Requirement> {
        let req = |id: &str, score: f64, parent: Option<&str>, section: Option<&str>,
                       criteria: &str| {
            let mut params = HashMap::new();
            params.insert("score".to_string(), serde_json::json!(score));
            if let Some(p) = parent {
                params.insert("parent".to_string(), serde_json::json!(p));
            }
            if let Some(s) = section {
                params.insert("section".to_string(), serde_json::json!(s));
            }
            Requirement {
                id: id.into(),
                category: "评分项".into(),
                clause_no: None,
                anchors: vec![crate::Anchor {
                    file: "A".into(),
                    kind: "para".into(),
                    locator: "第四章 评标办法".into(),
                    excerpt: criteria.into(),
                }],
                params,
                source_file: "A".into(),
            }
        };
        vec![
            req("T-TECH", 30.0, None, None, "技术分（30 分）"),
            req("T1", 10.0, Some("T-TECH"), Some("SEC-01"), "技术方案完整性与针对性 10 分"),
            req("T2", 10.0, Some("T-TECH"), Some("SEC-02"), "项目实施方案与进度保障措施 10 分"),
            req("T3", 10.0, Some("T-TECH"), Some("SEC-03"), "项目团队配置与人员资历 10 分"),
            req("C-COMM", 20.0, None, None, "商务分（20 分）"),
            req("C1", 10.0, Some("C-COMM"), Some("SEC-10"), "类似工程业绩，每提供 1 个得 2 分，最多 5 个"),
            req("C2", 10.0, Some("C-COMM"), Some("SEC-10"),
                "企业具备 ISO9001 质量管理体系认证及 ISO27001 信息安全管理体系认证"),
            req("P-PRICE", 40.0, None, None,
                "价格分（40 分）：有效投标报价算术平均值下浮 5% 为基准价，价格分＝基准价／投标报价×40"),
        ]
    }

    /// 优档章节内容（全覆盖 + 字数达标（`min_len`——按映射最大分值项的
    /// 50 字/分线传入）+ 证据 + 具体性——项目专名/数字单位）。
    fn excellent_section(keywords: &[&str], min_len: usize) -> String {
        let mut s = format!(
            "本章节针对某市第一人民医院智慧病房物联网平台建设项目（ZHYY-2026-GK-018）展开论述。{}。",
            keywords.join("、")
        );
        s.push_str("具体而言：覆盖 6 个病区 320 张床位，部署物联网网关 180 台，核心链路可用性 99.9%。");
        s.push_str("同类项目业绩与合同证明材料详见附件（编号 EV-2023-001 至 EV-2023-005）。");
        // 填充到达标线以上——填充不含判定特征（不增关键词/证据/具体性）
        while s.chars().filter(|c| !c.is_whitespace()).count() < min_len {
            s.push_str("上述安排已在本单位同类项目中形成标准化执行路径并持续改进。");
        }
        s
    }

    /// 凑字数章节（通用套话——无关键词/无证据/无具体数字）。
    fn filler_section(n_chars: usize) -> String {
        let mut s = String::new();
        while s.chars().count() < n_chars {
            s.push_str("我司高度重视本项目，将组建优秀团队，采用先进理念，科学管理，精心组织，确保圆满履约。");
        }
        s
    }

    /// 四维→四档判定（issue §三档位表逐档验证）。
    #[test]
    fn grade_tier_four_dimensions() {
        let scorer = SectionScorer::new(
            derive_scoring_tree(&b2_form_reqs()),
            vec!["某市第一人民医院".to_string(), "ZHYY-2026-GK-018".to_string()],
        );
        let t1 = scorer.tree.index.get("T1").unwrap();
        // 优：全覆盖 + 字数 + 证据 + 具体性
        let d = scorer.assess(t1, &excellent_section(&["技术方案完整性", "针对性"], 600));
        assert_eq!(SectionScorer::grade(&d), GradeTier::Excellent, "{d:?}");
        // 中：全覆盖 + 字数，无证据/无具体性（通用但切题的正文）
        let mut medium = String::from("本章论述技术方案完整性与针对性，并说明针对性落实路径。");
        while medium.chars().count() < 600 {
            medium.push_str("技术方案完整性与针对性方面的内容持续展开说明。");
        }
        let d = scorer.assess(t1, &medium);
        assert_eq!(SectionScorer::grade(&d), GradeTier::Fair, "{d:?}");
        // 差：字数达标但覆盖 0（凑字数形态）
        let d = scorer.assess(t1, &filler_section(700));
        assert_eq!(SectionScorer::grade(&d), GradeTier::Poor, "{d:?}");
        // 良：覆盖 100% + 字数 + 证据、无具体性（无项目名/数字单位）
        let mut good = String::from(
            "本章论述技术方案完整性与针对性，相关业绩合同详见附件。",
        );
        while good.chars().count() < 600 {
            good.push_str("技术方案完整性与针对性持续深化论证。");
        }
        let d = scorer.assess(t1, &good);
        assert_eq!(SectionScorer::grade(&d), GradeTier::Good, "{d:?}");
    }

    /// 好章节 vs 凑字数章节：自评得分明显差距（issue §五验收 2）。
    #[test]
    fn score_section_good_vs_filler_clear_gap() {
        let scorer = SectionScorer::new(
            derive_scoring_tree(&b2_form_reqs()),
            vec!["某市第一人民医院".to_string()],
        );
        let good = scorer.score_section(
            "SEC-01",
            &excellent_section(&["技术方案完整性", "针对性"], 600),
        );
        let bad = scorer.score_section("SEC-01", &filler_section(700));
        // 优 9.5 vs 差 2.75（10 分项）——3.5 倍差距
        assert!((good.estimated - 9.5).abs() < 0.01, "优档 = 9.5，got {}", good.estimated);
        assert!((bad.estimated - 2.75).abs() < 0.01, "凑字数 = 2.75，got {}", bad.estimated);
        assert!(good.estimated > 3.0 * bad.estimated, "差距须显著");
        // covered/missing 清单
        assert_eq!(good.covered_items, vec!["T1".to_string()]);
        assert!(good.missing_items.is_empty());
        assert_eq!(bad.missing_items, vec!["T1".to_string()]);
        assert!(bad.covered_items.is_empty());
        // rationale 可追溯（四维明细 + 档位 + 估分算式）
        assert!(good.rationale.contains("覆盖 2/2=100%"), "\n{}", good.rationale);
        assert!(good.rationale.contains("字数"), "字数明细");
        assert!(good.rationale.contains("证据 ✓"), "证据明细");
        assert!(good.rationale.contains("具体性 ✓"), "具体性明细");
        assert!(good.rationale.contains("优（95.0%）"), "档位+比例");
        assert!(good.rationale.contains("10.00×0.950=9.50"), "估分算式可复核");
    }

    /// score_document：三段聚合 + B1 价格分接线 + 无报价诚实登记。
    #[test]
    fn score_document_aggregates_three_faces() {
        let scorer = SectionScorer::new(
            derive_scoring_tree(&b2_form_reqs()),
            vec!["某市第一人民医院".to_string()],
        );
        let comm_good = format!(
            "商务响应：类似工程业绩，每提供 1 个得 2 分，最多 5 个，我方提供 5 个（2019-2023 年累计合同额 8000 万元，详见附件合同复印件）。\
             企业具备 ISO9001 质量管理体系认证及 ISO27001 信息安全管理体系认证（证书扫描件附后）。{}",
            "类似工程业绩与体系认证的证明材料齐全，可随时核验。".repeat(25)
        );
        let sections = vec![
            ("SEC-01".to_string(), excellent_section(&["技术方案完整性", "针对性"], 600)),
            ("SEC-02".to_string(), excellent_section(&["项目实施方案", "进度保障措施"], 600)),
            ("SEC-03".to_string(), excellent_section(&["项目团队配置", "人员资历"], 600)),
            ("SEC-10".to_string(), comm_good),
        ];
        // 价格分（B1 AveragePrice 0.95 + Ratio——synth-tender 4.1 同款规则，满分 40）
        let spec = crate::pricing::PriceScoreSpec {
            method: crate::pricing::PriceScoreMethod::AveragePrice { coefficient: 0.95 },
            max_score: 40.0,
            budget: None,
            deviation_rule: crate::pricing::DeviationRule::Ratio,
        };
        let ds = scorer.score_document(
            &sections,
            Some(PriceInput { spec: &spec, bid_price: 1600.0, others: vec![1500.0, 1600.0, 1700.0, 1800.0] }),
        );
        // 手工：基准 = mean(1600,1500,1600,1700,1800)=1640 ×0.95 = 1558；
        //      价格分 = 1558/1600 × 40 = 38.95
        assert!((ds.price - 38.95).abs() < 0.01, "价格分调 B1：got {}", ds.price);
        // 技术 = 3 × 9.5 = 28.5；商务（客观项提及性估计）= 2 × 9.5 = 19
        assert!((ds.technical - 28.5).abs() < 0.01, "got {}", ds.technical);
        assert_eq!(ds.commercial, Some(19.0), "got {:?}", ds.commercial);
        // 总分 = 28.5 + 19 + 38.95 = 86.45
        assert!((ds.total - 86.45).abs() < 0.01, "got {}", ds.total);
        // 价格类叶不做内容自评（rationale 登记）——SEC 里没映射价格项
        // 未报价面：诚实登记
        let ds2 = scorer.score_document(&sections, None);
        assert!(ds2.notes.iter().any(|n| n.contains("NO_PRICE_INPUT")), "{:?}", ds2.notes);
        assert!((ds2.total - 47.5).abs() < 0.01, "未报价 total 不含价格面，got {}", ds2.total);
        // 明细面：breakdown 四章，SEC-10 两客观项
        let sec10 = ds.breakdown.iter().find(|s| s.section_id == "SEC-10").unwrap();
        assert_eq!(sec10.item_scores.len(), 2);
        assert!(sec10.item_scores.iter().all(|i| i.is_objective));
        assert!(sec10.rationale.contains("客观项"), "客观项局限说明可追溯");
    }

    /// issue #94 §五验收 1：真实评分表形态，理想标书手工 vs 引擎 <10%。
    #[test]
    fn acceptance_manual_vs_engine_under_10pct() {
        let scorer = SectionScorer::new(
            derive_scoring_tree(&b2_form_reqs()),
            vec!["某市第一人民医院".to_string()],
        );
        let comm_good = format!(
            "类似工程业绩，每提供 1 个得 2 分，最多 5 个，我方提供 5 个（2019-2023 年累计合同额 8000 万元，详见附件合同复印件）。\
             企业具备 ISO9001 质量管理体系认证及 ISO27001 信息安全管理体系认证。{}",
            "证明材料齐全可核验。".repeat(48)
        );
        let sections = vec![
            ("SEC-01".to_string(), excellent_section(&["技术方案完整性", "针对性"], 600)),
            ("SEC-02".to_string(), excellent_section(&["项目实施方案", "进度保障措施"], 600)),
            ("SEC-03".to_string(), excellent_section(&["项目团队配置", "人员资历"], 600)),
            ("SEC-10".to_string(), comm_good),
        ];
        let spec = crate::pricing::PriceScoreSpec {
            method: crate::pricing::PriceScoreMethod::AveragePrice { coefficient: 0.95 },
            max_score: 40.0,
            budget: None,
            deviation_rule: crate::pricing::DeviationRule::Ratio,
        };
        let ds = scorer.score_document(
            &sections,
            Some(PriceInput { spec: &spec, bid_price: 1600.0, others: vec![1500.0, 1600.0, 1700.0, 1800.0] }),
        );
        // 手工（人肉按档位规则推）：理想标书 = 优档 95% × 主观 30 = 28.5；
        // 客观项提及性估计 95% × 20 = 19；价格分 = (1640×0.95)/1600×40 = 38.95
        // → 总分 86.45
        let manual_total = 86.45;
        let rel = ((ds.total - manual_total).abs() / manual_total * 100.0).round();
        assert!(rel < 10.0, "差距 {rel}% ≥ 10%：engine={} manual={}", ds.total, manual_total);
        // 半成品标书（一章凑字数 + 商务半数缺）：手工 =
        //   技术 9.5+9.5+2.75 = 21.75；商务 9.5+2.75 = 12.25；价格 38.95 → 72.95
        let half_comm = format!(
            "类似工程业绩，每提供 1 个得 2 分，最多 5 个，我方提供 5 个（2019-2023 年累计合同额 8000 万元，详见附件合同复印件）。{}",
            "其他证明材料后续补充。".repeat(50)
        );
        let half = vec![
            ("SEC-01".to_string(), excellent_section(&["技术方案完整性", "针对性"], 600)),
            ("SEC-02".to_string(), excellent_section(&["项目实施方案", "进度保障措施"], 600)),
            ("SEC-03".to_string(), filler_section(700)),
            ("SEC-10".to_string(), half_comm),
        ];
        let ds2 = scorer.score_document(
            &half,
            Some(PriceInput { spec: &spec, bid_price: 1600.0, others: vec![1500.0, 1600.0, 1700.0, 1800.0] }),
        );
        let manual2 = 72.95;
        let rel2 = ((ds2.total - manual2).abs() / manual2 * 100.0).round();
        assert!(rel2 < 10.0, "半成品差距 {rel2}%：engine={} manual={}", ds2.total, manual2);
        // 理想 vs 半成品差距显著（issue §五验收 2 的文档级形态）
        assert!(ds.total - ds2.total > 10.0, "文档级差距 {} 须显著", ds.total - ds2.total);
        // rationale 全链可追溯（issue §五验收 3）
        for s in &ds.breakdown {
            assert!(s.rationale.contains("覆盖"), "{} rationale 缺覆盖明细", s.section_id);
            assert!(s.rationale.contains("→"), "{} rationale 缺档位判定", s.section_id);
        }
        eprintln!("---- PR2 验收对照（issue #94 §五形态）----");
        eprintln!("  理想标书：manual=86.45 engine={} (差 {}%)", ds.total, rel);
        eprintln!("  半成品：manual=72.95 engine={} (差 {}%)", ds2.total, rel2);
        eprintln!("  差距：理想-半成品 = {:.2} 分", ds.total - ds2.total);
    }

    /// synth-tender 全链自评（PR1 树 + B1 价格分 30 分 AveragePrice——
    /// A §4.1 原句「算术平均值下浮 5% 为基准价，价格分＝（基准价／投标报价）×30」）。
    #[test]
    fn synth_tender_full_document_self_eval() {
        // 复用 PR1 的树构造，补 section 槽位（4.2 四子 + 4.4 三子 → SEC-T1；
        // 4.3 六子 → SEC-C1；价格 4.1 无 section——价格类跳过）
        let mut reqs = synth_tender_reqs();
        let sec_map = [
            ("SYN-A-042a", "SEC-T1"), ("SYN-A-042b", "SEC-T1"), ("SYN-A-042c", "SEC-T1"),
            ("SYN-A-042d", "SEC-T1"), ("SYN-A-044a", "SEC-T2"), ("SYN-A-044b", "SEC-T2"),
            ("SYN-A-044c", "SEC-T2"), ("SYN-A-043a", "SEC-C1"), ("SYN-A-043b", "SEC-C1"),
            ("SYN-A-043c", "SEC-C1"), ("SYN-A-043d", "SEC-C1"), ("SYN-A-043e", "SEC-C1"),
            ("SYN-A-043f", "SEC-C1"),
        ];
        for r in &mut reqs {
            if let Some((_, sec)) = sec_map.iter().find(|(id, _)| *id == r.id) {
                r.params.insert("section".to_string(), serde_json::json!(sec));
            }
        }
        let scorer = SectionScorer::new(
            derive_scoring_tree(&reqs),
            vec!["某市第一人民医院".to_string()],
        );
        // 理想技术章节（4.2 四项关键词全命中——30 分项字数线 1500 → 1600 字）
        // + 理想团队章节（4.4 三项细则原文引用）+ 理想商务章节（4.3 六项原文引用）
        let tech = excellent_section(&["技术方案完整性", "针对性", "项目实施方案", "进度保障措施",
            "售后服务方案", "响应时限承诺", "信息安全", "等保符合性方案"], 1600);
        let team = excellent_section(&["项目负责人具备一级注册建造师（机电工程）资格且无在建项目",
            "项目团队具有高级网络工程师或高级信息系统项目管理师职称证书",
            "拟派项目经理具有同类项目管理业绩"], 600);
        let comm = excellent_section(&["近三年完成的合同额 500 万元（含）以上类似工程业绩，每提供 1 个得 2 分",
            "企业认证体系（ISO9001）", "企业认证体系（ISO27001）", "银行资信等级 AAA 级",
            "类似项目获省级以上奖项", "拟派项目团队中级以上职称人员"], 600);
        let sections = vec![
            ("SEC-T1".to_string(), tech),
            ("SEC-T2".to_string(), team),
            ("SEC-C1".to_string(), comm),
        ];
        let spec = crate::pricing::PriceScoreSpec {
            method: crate::pricing::PriceScoreMethod::AveragePrice { coefficient: 0.95 },
            max_score: 30.0,
            budget: Some(1650.0),
            deviation_rule: crate::pricing::DeviationRule::Ratio,
        };
        // 报价 1500 万，对手 [1480, 1520, 1560, 1590]：
        // 基准 = mean(1500,1480,1520,1560,1590)=1530 ×0.95=1453.5
        // 价格分 = 1453.5/1500 × 30 = 29.07
        let ds = scorer.score_document(
            &sections,
            Some(PriceInput { spec: &spec, bid_price: 1500.0, others: vec![1480.0, 1520.0, 1560.0, 1590.0] }),
        );
        assert!((ds.price - 29.07).abs() < 0.01, "B1 接线：got {}", ds.price);
        // 手工：技术主观叶（042a-d 30+10+8+7=55 主观 + 044 三子 8 客观计商务面）
        //   理想章节 → 全优 95%：主观 55×0.95 = 52.25；客观（044a-c 8 + 043 六子 15）= 23×0.95 = 21.85
        //   total = 52.25 + 21.85 + 29.07 = 103.17（>100：synth-tender 根合计 108 病的传导，
        //   自评如实呈现不截断——B3 消费时以树校验为准）
        assert!((ds.technical - 52.25).abs() < 0.05, "got {}", ds.technical);
        assert_eq!(ds.commercial, Some(21.85), "got {:?}", ds.commercial);
        eprintln!("---- PR2 synth-tender 全链 ----");
        eprintln!("  technical={} commercial={:?} price={} total={}",
            ds.technical, ds.commercial, ds.price, ds.total);
        for s in &ds.breakdown {
            eprintln!("  [{}] est={}/max={}", s.section_id, s.estimated, s.max);
            for it in &s.item_scores {
                eprintln!("    {} {} {} est={}", it.item_id, it.tier.label(), it.coverage, it.estimated);
            }
        }
        // 档位差距面：凑字数技术章 vs 理想技术章
        let bad = scorer.score_section("SEC-T1", &filler_section(3000));
        assert!(bad.estimated < ds.breakdown[0].estimated / 2.0, "凑字数须显著低分");
    }
}
