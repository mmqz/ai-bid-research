//! budget.rs —— S-7 上下文预算与证据裁剪（issue #11 批次 v3 / V3-5）。
//!
//! 定位：单章 LLM 输入的**预算规划器**——「任意单章输入 ≤ 8k tokens」的裁决面。
//! 纯函数、零 IO、确定性（同输入逐字节同输出——issue 验收「实测」的尺子）。
//! 组装胶水（从 store 取素材 + 渲染 prompt 文本）在 bid-tools `context`。
//!
//! 溢出四阶梯（issue #11 原文顺序，严格遵守）：
//! a. 约束区（全局约束 + 章节规格）**永不裁剪**：先扣除其 token；
//!    约束区自身超红线（`budget.total`）→ **硬错误**（Err）——绝不静默截断
//!    约束、绝不悄悄放行超限（调用方必须显式处理，无静默降级通道）；
//! b. 剩余预算给证据：按 relevance 降序贪心装满 full（top-N 的 N 由预算反推
//!    ——前缀语义：装不下第 N 条即停，第 N+1 起进入降级面）；
//! c. 装不下的长证据降级 full→summary：候选全集先全部转 summary，再从**低
//!    相关端**逐条丢弃直到装得下（高相关的 summary 优先保，低相关的整个丢）；
//! d. 仍不行 → 继续从低相关端砍（此时砍的是高相关的 summary）直到装得下；
//!    全丢完仍超**规划目标**（安全边际被约束区吃光）→ 计划带 `overflow: true`
//!    （非失败——调用方裁决位）。
//!
//! 8k 红线的三重结构保证：约束区超红线硬错误（a）+ 证据可全裁到 0（b-d）+
//! 组装侧渲染后实测校验（bid-tools `context` 的最后一道门）。规划本身只对
//! **规划目标**（红线 - 安全边际）装填，给渲染标记留 10% 余量。
//!
//! 确定性纪律（R-15 同源精神）：
//! - relevance 平局按 evidence_id 字典序（`f32::total_cmp`——IEEE totalOrder，
//!   NaN 亦有确定位；全程不用 HashMap 迭代序——结果逐字节可重放）；
//! - `evidence_plan` 输出序 = (relevance desc, id asc)——可 diff、可审计。
//!
//! token 估算口径（[`estimate_tokens`]——全仓单一真源，S-7 验收的尺子）：
//! - 非 ASCII 字符（中文/全角标点等）≈ 1 token/字——标书以中文为主体；
//! - ASCII 连续非空白串按 4 字符 ≈ 1 token 折算（向上取整——英文词/数字串
//!   的保守估计；宁多计不少计：预算方向的误差只会更早触发裁剪，不会放行超限）；
//! - ASCII 空白不计（词间分隔已摊销进 4 字符口径）。
//!   同一输入永远同一输出（纯字符扫描，无状态无随机）。
//!
//! 渲染开销计量（保守上界——与 bid-tools `context::render_prompt` 双向锁定）：
//! 规划不只算内容 token，还计入渲染 prompt 时必然产生的标记行开销
//! （区块头/证据块头/裁剪清单行），常量见下方 `*_TOKENS`。计量的每一项都
//! ≥ 渲染实测值（保守方向）；红线最终由渲染后实测校验兜底（超长 id 等
//! 极端构造由该门拦截——fail-closed 绝不放行超限）。

use serde::{Deserialize, Deserializer, Serialize};

// ---------------------------------------------------------------- 常量面
/// ContextPlan 序列化 schema 版本（M3 起草落盘的前向兼容面——加性演进：
/// 新字段一律 `#[serde(default)]`；结构性变更 → 版本 +1 + migrate）。
pub const CONTEXT_PLAN_SCHEMA_VERSION: u32 = 1;

/// 单章输入 token 红线（issue #11 S-7 规格：单章输入 ≤ 8k tokens）。
pub const DEFAULT_TOTAL_TOKENS: u64 = 8_000;

/// 安全边际百分比（规划目标 = 红线 × (100 - pct)% = 7,200——吸收渲染标记
/// 与估算口径的偏差；红线是硬门，规划目标才是装填线）。
pub const DEFAULT_HEADROOM_PCT: u64 = 10;

/// 每条入选证据（Full/Summary）的渲染块头开销上界（`[证据 EV-xxxx｜完整]`/
/// `[证据 EV-xxxx｜摘要 truncated_evidence]` 一行——id ≤ 16 字符时实测 ≤ 16）。
pub const EVIDENCE_MARKER_TOKENS: u64 = 24;

/// 前缀区渲染头部开销上界（【项目约束】/【章节规格】/【证据材料】三个区块
/// 头 + 证据区统计行，实测 ~56——保守 128）。
pub const PREFIX_HEADER_TOKENS: u64 = 128;

/// 每条约束块的标签开销上界（`[金额]` 一类——kind ≤ 11 字时实测 ≤ 9）。
pub const CONSTRAINT_LABEL_TOKENS: u64 = 16;

/// 裁剪清单行（dropped 汇总）的固定开销上界（区块头 + 尾注）。
pub const DROPPED_LINE_BASE_TOKENS: u64 = 32;

/// 裁剪清单行每个 dropped id 的开销上界（id + 顿号，实测 ~3/id）。
pub const DROPPED_ID_TOKENS: u64 = 4;

// ---------------------------------------------------------------- token 估算
/// 确定性 token 估算（口径见模块头注释——S-7 验收「实测」的尺子）。
pub fn estimate_tokens(text: &str) -> u64 {
    let ceil4 = |n: u64| (n + 3) / 4; // 4 字符 ≈ 1 token，向上取整（保守）
    let mut total: u64 = 0;
    let mut ascii_run: u64 = 0; // 当前 ASCII 连续非空白串长度
    for ch in text.chars() {
        if ch.is_ascii() {
            if ch.is_ascii_whitespace() {
                total += ceil4(ascii_run); // 空白终结当前 ASCII 串
                ascii_run = 0;
            } else {
                ascii_run += 1;
            }
        } else {
            total += ceil4(ascii_run); // 非 ASCII 终结当前 ASCII 串
            ascii_run = 0;
            total += 1; // 中文/全角 ≈ 1 token/字
        }
    }
    total + ceil4(ascii_run)
}

// ---------------------------------------------------------------- 预算参数
/// 预算规划参数（8k 是规格红线；规划目标留 10% 安全边际即 7,200）。
///
/// `total` 是硬门（约束区超它 = 硬错误；渲染实测超它 = 组装失败）；
/// [`ContextBudget::planning_target`] 才是证据装填线。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudget {
    pub total: u64,
    /// 安全边际百分比（0..=100——超出按 100 截断，规划目标退化为 0）。
    pub constraint_reserve_headroom_pct: u64,
}

impl Default for ContextBudget {
    fn default() -> Self {
        ContextBudget {
            total: DEFAULT_TOTAL_TOKENS,
            constraint_reserve_headroom_pct: DEFAULT_HEADROOM_PCT,
        }
    }
}

impl ContextBudget {
    /// 规划目标 = 红线 × (100 - headroom)%（默认 8,000 × 90% = 7,200）。
    pub fn planning_target(&self) -> u64 {
        let pct = self.constraint_reserve_headroom_pct.min(100);
        self.total.saturating_mul(100 - pct) / 100
    }
}

// ---------------------------------------------------------------- 预算输入
/// 全局约束块（金额/工期/术语表等——压缩成固定前缀，永不裁剪）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintBlock {
    /// 约束类别标签（渲染为 `[金额]` 一类前缀——≤11 字以内的计量口径）。
    pub kind: String,
    /// 约束全文（渲染逐字节原样——验收：约束文本必须完整出现在输出里）。
    pub text: String,
    /// `estimate_tokens(text)` 的预计算副本（plan 校验一致性，防谎报）。
    pub tokens: u64,
}

impl ConstraintBlock {
    /// 便捷构造（tokens 现算——单一真源 [`estimate_tokens`]）。
    pub fn new(kind: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let tokens = estimate_tokens(&text);
        ConstraintBlock {
            kind: kind.into(),
            text,
            tokens,
        }
    }
}

/// 章节规格块（单章的规格来源——OutlineNode 的渲染文本，随 token 计量）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionSpecBlock {
    pub section_id: String,
    /// 组装侧渲染好的规格文本（章节/状态/响应条款/字数配额）。
    pub text: String,
    /// `estimate_tokens(text)` 的预计算副本（plan 校验一致性）。
    pub tokens: u64,
}

impl SectionSpecBlock {
    pub fn new(section_id: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let tokens = estimate_tokens(&text);
        SectionSpecBlock {
            section_id: section_id.into(),
            text,
            tokens,
        }
    }
}

/// 证据候选（预算的选择面——相关度 + 全文/摘要双形态 token）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceCandidate {
    pub evidence_id: String,
    /// 相关度（kb 检索分或绑定序确定性分数——平局按 id 字典序破）。
    pub relevance: f32,
    /// 证据全文（full 形态的素材）。
    pub full_text: String,
    /// 预处理好的关键段（引擎 Locator 定位到段的摘要——组装侧准备；
    /// 渲染时以 `\n` 连接）。
    pub summary_paragraphs: Vec<String>,
    /// `estimate_tokens(full_text)` 的预计算副本（plan 校验一致性）。
    pub tokens_full: u64,
    /// `estimate_tokens(summary_paragraphs.join("\n"))` 的预计算副本。
    pub tokens_summary: u64,
}

impl EvidenceCandidate {
    /// 便捷构造（两个 token 字段现算——口径钉死在 [`estimate_tokens`]）。
    pub fn new(
        evidence_id: impl Into<String>,
        relevance: f32,
        full_text: impl Into<String>,
        summary_paragraphs: Vec<String>,
    ) -> Self {
        let full_text = full_text.into();
        let tokens_full = estimate_tokens(&full_text);
        let summary_text = summary_paragraphs.join("\n");
        let tokens_summary = estimate_tokens(&summary_text);
        EvidenceCandidate {
            evidence_id: evidence_id.into(),
            relevance,
            full_text,
            summary_paragraphs,
            tokens_full,
            tokens_summary,
        }
    }
}

/// 预算输入（组装侧备齐：约束区 + 章节规格 + 证据候选集）。
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetInput {
    pub constraints: Vec<ConstraintBlock>,
    pub section_spec: SectionSpecBlock,
    pub candidates: Vec<EvidenceCandidate>,
}

// ---------------------------------------------------------------- 规划产物
/// 证据在计划中的形态（溢出阶梯 b/c/d 的落点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceMode {
    /// 全文入选（阶梯 b）。
    Full,
    /// 降级为关键段摘要（阶梯 c——`truncated_evidence` 标记面）。
    Summary,
    /// 整条丢弃（阶梯 c/d——低相关优先，`truncated_evidence` 标记面）。
    Dropped,
}

impl EvidenceMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceMode::Full => "full",
            EvidenceMode::Summary => "summary",
            EvidenceMode::Dropped => "dropped",
        }
    }
}

/// 单条证据的规划落点（含审计面：为什么它被保/被裁）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidencePlanEntry {
    pub evidence_id: String,
    /// 相关度快照（裁决依据留痕——同 relevance 平局按 id 序的证明素材）。
    pub relevance: f32,
    pub mode: EvidenceMode,
    /// 计入预算的 token（Full/Summary 含块头标记开销；Dropped = 0）。
    pub tokens: u64,
}

/// 上下文预算计划（可序列化——M3 起草落盘的前向兼容面，版本门见 Deserialize）。
///
/// 字段语义：
/// - `prefix_tokens`：约束区（约束 + 章节规格 + 前缀渲染标记）——永不裁剪面；
/// - `total_tokens`：**保守计量上界**（≥ 渲染实测——审计/规划对账用；
///   红线判据是组装侧渲染后的 `estimate_tokens` 实测，不是本字段）；
/// - `truncated_evidence`：任何证据被降级（Summary）或丢弃（Dropped）即 true
///   （issue 验收：证据被裁剪时输出里必须标 `truncated_evidence`）；
/// - `overflow`：保守计量突破**规划目标**（约束区吃光安全边际 / 裁剪清单
///   过长挤压边际）——非失败，调用方裁决位；红线由渲染实测硬守。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextPlan {
    #[serde(default = "default_plan_version")]
    pub schema_version: u32,
    /// 规划参数快照（红线——审计对账：这份计划按什么预算装的）。
    pub budget_total: u64,
    /// 规划目标（红线 × (100 - headroom)%）。
    pub planning_target: u64,
    pub prefix_tokens: u64,
    /// 输出序 = (relevance desc, id asc)——确定性可 diff。
    pub evidence_plan: Vec<EvidencePlanEntry>,
    pub total_tokens: u64,
    pub truncated_evidence: bool,
    pub overflow: bool,
}

fn default_plan_version() -> u32 {
    CONTEXT_PLAN_SCHEMA_VERSION
}

/// 版本门（borrow：mdcore `schema_version_gate` 口径 + 本仓 store_io 同款纪律）：
/// 反序列化遇到**高于当前支持**的版本 → 响亮拒绝（未来数据遇旧代码——
/// 不猜测降级加载）；缺省版本 = 当前版（加性演进的向后兼容面）。
impl<'de> Deserialize<'de> for ContextPlan {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default = "default_plan_version")]
            schema_version: u32,
            budget_total: u64,
            planning_target: u64,
            prefix_tokens: u64,
            evidence_plan: Vec<EvidencePlanEntry>,
            total_tokens: u64,
            truncated_evidence: bool,
            overflow: bool,
        }
        let raw = Raw::deserialize(d)?;
        if raw.schema_version > CONTEXT_PLAN_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(format!(
                "ContextPlan schema_version={} 高于当前支持 {}（未知版本拒载——升级程序后重试）",
                raw.schema_version, CONTEXT_PLAN_SCHEMA_VERSION
            )));
        }
        Ok(ContextPlan {
            schema_version: raw.schema_version,
            budget_total: raw.budget_total,
            planning_target: raw.planning_target,
            prefix_tokens: raw.prefix_tokens,
            evidence_plan: raw.evidence_plan,
            total_tokens: raw.total_tokens,
            truncated_evidence: raw.truncated_evidence,
            overflow: raw.overflow,
        })
    }
}

// ---------------------------------------------------------------- 错误面
/// 预算规划错误（fail-closed——全部响亮拒绝，无静默降级通道）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// 约束区超红线（阶梯 a 硬错误——绝不静默截断约束，绝不悄悄放行超限）。
    ConstraintOverflow { prefix_tokens: u64, total: u64 },
    /// 证据 id 重复（平局破序的前提被破坏——确定性失效面，拒）。
    DuplicateEvidenceId { id: String },
    /// 章节规格缺 section id（空/纯空白——组装侧不该放行到这里，防御性双门）。
    EmptySectionId,
    /// token 字段与 `estimate_tokens` 实测不符（谎报预算 = 红线失守面，拒）。
    TokenAccountingMismatch {
        what: String,
        declared: u64,
        measured: u64,
    },
}

impl std::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetError::ConstraintOverflow {
                prefix_tokens,
                total,
            } => write!(
                f,
                "约束区 {prefix_tokens} tokens 超出预算红线 {total}（约束区超预算，绝不静默截断、\
                 绝不悄悄放行超限——请拆分约束或显式上调 budget.total）"
            ),
            BudgetError::DuplicateEvidenceId { id } => {
                write!(
                    f,
                    "证据 id 重复: {id}（平局破序前提被破坏——确定性失效，拒绝规划）"
                )
            }
            BudgetError::EmptySectionId => {
                write!(
                    f,
                    "章节规格缺 section id（空/纯空白——单章输入必须有明确归属）"
                )
            }
            BudgetError::TokenAccountingMismatch {
                what,
                declared,
                measured,
            } => write!(
                f,
                "{what} 的 tokens 字段({declared})与 estimate_tokens 实测({measured})不符\
                 （谎报预算会让红线失守——拒绝规划）"
            ),
        }
    }
}

impl std::error::Error for BudgetError {}

// ---------------------------------------------------------------- 规划核心
/// 前缀区渲染标记开销（头部 + 每约束块标签——保守上界，见模块头）。
fn prefix_marker_tokens(constraint_count: usize) -> u64 {
    PREFIX_HEADER_TOKENS + CONSTRAINT_LABEL_TOKENS * constraint_count as u64
}

/// 溢出四阶梯规划（纯函数——确定性，同输入逐字节同输出）。
///
/// 返回 [`ContextPlan`]（含 overflow 裁决位）或 [`BudgetError`]（约束区超
/// 红线 / id 重复 / token 账不平——全 fail-closed）。阶梯语义见模块头注释。
pub fn plan(input: &BudgetInput, budget: &ContextBudget) -> Result<ContextPlan, BudgetError> {
    // ---- fail-closed 面（先于一切计算——坏输入零产出）
    if input.section_spec.section_id.trim().is_empty() {
        return Err(BudgetError::EmptySectionId);
    }
    let mut seen = std::collections::HashSet::new();
    for c in &input.candidates {
        if !seen.insert(c.evidence_id.as_str()) {
            return Err(BudgetError::DuplicateEvidenceId {
                id: c.evidence_id.clone(),
            });
        }
    }
    // token 记账一致性（字段是便捷构造器的预计算副本——手工构造谎报即拒）
    for (i, c) in input.candidates.iter().enumerate() {
        let measured = estimate_tokens(&c.full_text);
        if c.tokens_full != measured {
            return Err(BudgetError::TokenAccountingMismatch {
                what: format!("candidates[{i}]（{}）tokens_full", c.evidence_id),
                declared: c.tokens_full,
                measured,
            });
        }
        let measured = estimate_tokens(&c.summary_paragraphs.join("\n"));
        if c.tokens_summary != measured {
            return Err(BudgetError::TokenAccountingMismatch {
                what: format!("candidates[{i}]（{}）tokens_summary", c.evidence_id),
                declared: c.tokens_summary,
                measured,
            });
        }
    }
    for (i, c) in input.constraints.iter().enumerate() {
        let measured = estimate_tokens(&c.text);
        if c.tokens != measured {
            return Err(BudgetError::TokenAccountingMismatch {
                what: format!("constraints[{i}]（{}）tokens", c.kind),
                declared: c.tokens,
                measured,
            });
        }
    }
    {
        let measured = estimate_tokens(&input.section_spec.text);
        if input.section_spec.tokens != measured {
            return Err(BudgetError::TokenAccountingMismatch {
                what: format!("section_spec（{}）tokens", input.section_spec.section_id),
                declared: input.section_spec.tokens,
                measured,
            });
        }
    }

    // ---- 阶梯 a：约束区永不裁剪，先扣除
    let prefix_tokens = input.constraints.iter().map(|c| c.tokens).sum::<u64>()
        + input.section_spec.tokens
        + prefix_marker_tokens(input.constraints.len());
    if prefix_tokens > budget.total {
        return Err(BudgetError::ConstraintOverflow {
            prefix_tokens,
            total: budget.total,
        });
    }
    let planning_target = budget.planning_target();
    let evidence_budget = planning_target.saturating_sub(prefix_tokens);

    // ---- 确定性排序：relevance 降序（total_cmp——NaN 亦有确定位）→ id 字典序
    let mut sorted: Vec<&EvidenceCandidate> = input.candidates.iter().collect();
    sorted.sort_by(|a, b| {
        b.relevance
            .total_cmp(&a.relevance)
            .then_with(|| a.evidence_id.cmp(&b.evidence_id))
    });

    // ---- 阶梯 b：按 relevance 降序贪心装 full（top-N 前缀语义：装不下即停）
    let mut used: u64 = 0;
    let mut modes: Vec<EvidenceMode> = Vec::with_capacity(sorted.len());
    let mut n_full = 0usize;
    for c in &sorted {
        let cost = c.tokens_full + EVIDENCE_MARKER_TOKENS;
        if used + cost <= evidence_budget {
            used += cost;
            modes.push(EvidenceMode::Full);
            n_full += 1;
        } else {
            break; // 第 n_full+1 条装不下 → top-N 到此为止，其余进降级面
        }
    }

    // ---- 阶梯 c+d：剩余候选全转 summary，从低相关端（尾部）逐条丢弃直到装得下
    // kept 是 [n_full..] 的前缀（高相关的 summary 优先保——丢弃只发生在尾部）。
    let rest = &sorted[n_full..];
    let summary_cost = |kept_n: usize| -> u64 {
        rest[..kept_n]
            .iter()
            .map(|c| c.tokens_summary + EVIDENCE_MARKER_TOKENS)
            .sum()
    };
    let mut kept_n = rest.len();
    while kept_n > 0 && used + summary_cost(kept_n) > evidence_budget {
        kept_n -= 1; // 砍最低相关的一条（整条丢——不保它的 summary）
    }
    for _ in n_full..sorted.len() {
        if modes.len() < n_full + kept_n {
            modes.push(EvidenceMode::Summary);
        } else {
            modes.push(EvidenceMode::Dropped);
        }
    }
    used += summary_cost(kept_n);

    // ---- 计划产物（输出序 = 排序序——确定性可 diff）
    let dropped_n = sorted.len() - n_full - kept_n;
    let evidence_plan: Vec<EvidencePlanEntry> = sorted
        .iter()
        .zip(modes.iter())
        .map(|(c, m)| {
            let tokens = match m {
                EvidenceMode::Full => c.tokens_full + EVIDENCE_MARKER_TOKENS,
                EvidenceMode::Summary => c.tokens_summary + EVIDENCE_MARKER_TOKENS,
                EvidenceMode::Dropped => 0,
            };
            EvidencePlanEntry {
                evidence_id: c.evidence_id.clone(),
                relevance: c.relevance,
                mode: *m,
                tokens,
            }
        })
        .collect();
    let truncated_evidence = evidence_plan.iter().any(|e| e.mode != EvidenceMode::Full);
    // dropped 清单行的渲染开销也计入保守计量（审计诚实：它真实占用 prompt）
    let dropped_line_tokens = if dropped_n > 0 {
        DROPPED_LINE_BASE_TOKENS + DROPPED_ID_TOKENS * dropped_n as u64
    } else {
        0
    };
    let total_tokens = prefix_tokens + used + dropped_line_tokens;
    Ok(ContextPlan {
        schema_version: CONTEXT_PLAN_SCHEMA_VERSION,
        budget_total: budget.total,
        planning_target,
        prefix_tokens,
        evidence_plan,
        total_tokens,
        truncated_evidence,
        overflow: total_tokens > planning_target,
    })
}

// ---------------------------------------------------------------- 单元测试
// 口径级细粒度断言住这里（src 内可见私有常量的最小面）；验收级集成测试
// （四阶梯/确定性/fail-closed/版本门——pub API 黑盒）在 tests/budget.rs。
#[cfg(test)]
mod tests {
    use super::*;

    /// estimate_tokens 口径单测：纯中文/纯 ASCII/混合/空串/纯空白的边界值。
    #[test]
    fn estimate_tokens_deterministic_cjk_ascii_mixed_empty() {
        // 空串 / 纯空白（ASCII 空白不计）
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("   \n\t  "), 0);
        // 纯中文：1 token/字（含全角标点——非 ASCII 各 1）
        assert_eq!(estimate_tokens("招标控制价"), 5);
        assert_eq!(estimate_tokens("，。；"), 3);
        // 纯 ASCII：连续非空白串按 4 字符 ≈ 1 token（向上取整）
        assert_eq!(estimate_tokens("ab"), 1); // ceil(2/4)
        assert_eq!(estimate_tokens("ABCD"), 1); // ceil(4/4)
        assert_eq!(estimate_tokens("ABCDE"), 2); // ceil(5/4)
        assert_eq!(estimate_tokens("budget"), 2); // ceil(6/4)
        assert_eq!(estimate_tokens("hello world"), 4); // 两个串：ceil(5/4)*2
        assert_eq!(estimate_tokens("350"), 1); // ceil(3/4)
                                               // 混合：CJK 各 1 + ASCII 串折算（空白与交错都终结 ASCII 串）
        assert_eq!(estimate_tokens("控制价 budget 350 万"), 7); // 3 + 2 + 1 + 1
        assert_eq!(estimate_tokens("abc证"), 2); // ceil(3/4)=1 + 1
        assert_eq!(estimate_tokens("证abc"), 2); // 1 + ceil(3/4)
                                                 // 确定性：同输入多次调用逐次相等（验收「实测」的尺子必须无状态）
        for t in ["", "招标控制价", "hello world", "控制价 budget 350 万"] {
            assert_eq!(estimate_tokens(t), estimate_tokens(t));
        }
    }

    /// planning_target 算术：默认 8,000 × 90% = 7,200；pct 越界截断。
    #[test]
    fn planning_target_arithmetic() {
        assert_eq!(ContextBudget::default().total, DEFAULT_TOTAL_TOKENS);
        assert_eq!(ContextBudget::default().planning_target(), 7_200);
        assert_eq!(
            ContextBudget {
                total: 8_050,
                constraint_reserve_headroom_pct: 10
            }
            .planning_target(),
            7_245,
        );
        assert_eq!(
            ContextBudget {
                total: 1_000,
                constraint_reserve_headroom_pct: 150
            }
            .planning_target(),
            0,
            "pct > 100 按 100 截断——规划目标退化为 0（红线仍由 total 硬守）",
        );
    }
}
