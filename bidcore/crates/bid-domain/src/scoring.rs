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

use crate::{DomainStore, Requirement};
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoringItem {
    pub requirement_id: String,
    /// 分值/权重（评分项有；否决项 None——否决无分值，漏了直接废标）
    pub weight: Option<f64>,
    /// 单项满分（V3-2 = weight 同值；细则拆分档位时分离）
    pub max_score: Option<f64>,
    /// 评分标准原文（锚点摘录——审校比对用）
    pub criteria: String,
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
    /// 映射章节（SEC-xxx / 章号）；未映射 → None
    #[serde(default)]
    pub section_id: Option<String>,
    /// 我方自评预计得分（派生恒 None——[`SectionScorer`] 运行时填充）
    #[serde(default)]
    pub our_estimate: Option<f64>,
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
/// 24 字符窗口内见 `得`/`加` + 数字 + `分`。双窗口限距防跨句误配。
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
        // 窗口 2：得/加 + 数字 + 分
        let w2_end = (q + 24).min(chars.len());
        let mut j = q + 1;
        while j < w2_end {
            if chars[j] == '得' || chars[j] == '加' {
                // 后视数字（可隔空白）再后视 分
                let mut k = j + 1;
                while k < w2_end && chars[k].is_whitespace() {
                    k += 1;
                }
                let num_start = k;
                while k < w2_end && (chars[k].is_ascii_digit() || chars[k] == '.') {
                    k += 1;
                }
                if k > num_start {
                    let mut m = k;
                    while m < w2_end && chars[m].is_whitespace() {
                        m += 1;
                    }
                    if m < w2_end && chars[m] == '分' {
                        return true;
                    }
                }
            }
            j += 1;
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
                    section_id: r
                        .params
                        .get("section")
                        .and_then(|v| v.as_str())
                        .map(String::from),
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
                is_hard_gate: true,
                parent_id: r
                    .params
                    .get("parent")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                tiers: tiers_from_params(r.params.get("tiers")),
                is_objective: false, // 否决项无分值——主客观无意义，恒 false
                section_id: r
                    .params
                    .get("section")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                our_estimate: None,
            }),
            _ => None,
        })
        .collect()
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

// ---------------------------------------------------------------- DomainStore 扩面
impl DomainStore {
    /// 评分矩阵（派生态只读——每次从 requirements 重算，幂等无落库）。
    pub fn scoring_items(&self) -> Vec<ScoringItem> {
        derive_scoring_items(&self.requirements)
    }

    /// 评分树（B2——层级视图；同评分矩阵同源派生，幂等）。
    pub fn scoring_tree(&self) -> ScoringTree {
        derive_scoring_tree(&self.requirements)
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
        // 章节映射随树派生（自评对接面）
        assert_eq!(
            tree.index.get("S-TECH-a").unwrap().section_id.as_deref(),
            Some("SEC-03")
        );
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
}
