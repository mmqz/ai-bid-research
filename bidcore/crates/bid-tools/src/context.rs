//! context.rs —— S-7 单章输入组装（issue #11 批次 v3 / V3-5 的胶水半边）。
//!
//! 分层纪律：纯预算/选择/裁剪逻辑在 bid-domain `budget`（零 IO、确定性、
//! 可单测）；本模块是碰 store 的胶水——从 store 取约束/章节规格/证据候选，
//! 调 `budget::plan` 规划，渲染成最终 prompt 文本。只读 + 纯渲染，
//! 同一 store 状态产出逐字节相同（可 diff、可重放——S-1/S-2 调度的输入面）。
//!
//! prompt 形态（确定序：约束前缀 → 章节规格 → 证据块）：
//! - 约束区（金额/工期/术语表等全局约束）**永不裁剪**——逐字节原样渲染；
//! - 证据块按计划落点渲染：Full 全文 / Summary 关键段（标
//!   `truncated_evidence`）/ Dropped 走裁剪清单行（标 `truncated_evidence`）；
//! - 8k 红线的最后一道门：渲染后 `estimate_tokens(prompt_text)` 实测，
//!   超红线 → `RenderOverflow` 硬错误（绝不放行超限——规划只对规划目标
//!   装填，标记开销的估算偏差由这道实测门兜底）。
//!
//! 供给面 trait（DomainStore 现状的诚实边界——有什么用什么，没有的留钩子）：
//! - 约束：store 尚无「项目约束」权威集合（金额/工期/术语表未落实体）——
//!   [`ConstraintSource`] 默认实现 [`BuiltinConstraintSource`] 只取项目标识
//!   （BidProject 面）；金额/工期/术语表由嵌入方注入（M3 落 tender_constraints
//!   实体后在此接线）；
//! - 证据文本：`bid_domain::evidence::Evidence` 是台账记录
//!   （locator + 指纹——不含正文），正文在解析产物 / kb chunk 侧——
//!   [`EvidenceTextSource`] 默认 [`LedgerTextSource`]（台账元数据渲染），
//!   kb 在场时用 [`ChunkTextSource`]（Locator 段落锚 ↔ chunk 定位）；
//! - 相关度：默认 [`BindingOrderRelevance`]（matrix 绑定序的确定性分数
//!   ——kb 不可用时的兜底口径）；kb 在场时 [`KbRelevance`]
//!   （S-3 `SearchHit.score` 既有口径）。

use bid_domain::budget::{
    plan as plan_budget, estimate_tokens, BudgetInput, BudgetError, ConstraintBlock,
    ContextBudget, ContextPlan, EvidenceCandidate, EvidenceMode, SectionSpecBlock,
    DROPPED_ID_TOKENS, DROPPED_LINE_BASE_TOKENS, EVIDENCE_MARKER_TOKENS,
};
use bid_domain::evidence::Evidence;
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineNode;
use bid_domain::DomainStore;
use bid_kb::{Chunk, ExactQuery, Kb};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ---------------------------------------------------------------- 供给面 trait
/// 全局约束供给面（S-7 约束前缀的素材源——金额/工期/术语表等）。
///
/// DomainStore 现无「项目约束」权威集合——默认实现 [`BuiltinConstraintSource`]
/// 只取项目标识；M3 计划把 tender_constraints 落进 store 后由 store 直供
/// （届时本 trait 的默认实现改为读权威集合，嵌入方注入面自然退役）。
pub trait ConstraintSource {
    fn constraint_blocks(&self, store: &DomainStore) -> Vec<ConstraintBlock>;
}

/// 默认约束源：项目标识（BidProject 面——store 里真实存在的全局信息）。
pub struct BuiltinConstraintSource;

impl ConstraintSource for BuiltinConstraintSource {
    fn constraint_blocks(&self, store: &DomainStore) -> Vec<ConstraintBlock> {
        match store.project() {
            Ok(p) => vec![ConstraintBlock::new(
                "项目",
                format!("项目名称：{}；项目编号：{}；投标模式：{}", p.name, p.code, p.mode),
            )],
            // 项目未创建 = 无全局约束面（空约束区——诚实空态，不是错误）
            Err(_) => vec![],
        }
    }
}

/// 证据文本供给面（full 全文 + Locator 定位到段的关键段摘要）。
pub trait EvidenceTextSource {
    /// 证据全文（full 形态素材——预算的 `tokens_full` 计量面）。
    fn full_text(&self, evidence: &Evidence) -> String;
    /// 预处理好的关键段（引擎 Locator 定位到段的摘要——`tokens_summary` 面；
    /// 渲染时以 `\n` 连接）。
    fn summary_paragraphs(&self, evidence: &Evidence) -> Vec<String>;
}

/// 默认文本源：台账元数据渲染（store-only 路径——零外部依赖，确定性）。
pub struct LedgerTextSource;

impl EvidenceTextSource for LedgerTextSource {
    fn full_text(&self, ev: &Evidence) -> String {
        format!(
            "{}\n类别：{}；适用范围：{}\n来源文件：{}（{}）；主体：{}",
            ev.title,
            ev.kind.as_str(),
            ev.scope,
            ev.source_file,
            locator_brief(&ev.locator),
            ev.subject_name,
        )
    }

    fn summary_paragraphs(&self, ev: &Evidence) -> Vec<String> {
        // 关键段 = 台账的检索面两要素（标题 + 适用范围）——确定性的最小摘要
        vec![ev.title.clone(), format!("适用范围：{}", ev.scope)]
    }
}

/// kb 增强文本源：DocxParagraph 锚 ↔ chunk 块序定位（引擎 Locator 语义在
/// kb 侧的对应面——chunk.block_idx 即段落块序）；锚不上其他引擎面时降级
/// 台账口径（诚实降级，不猜测）。
pub struct ChunkTextSource<'a> {
    pub kb: &'a Kb,
    /// 关键段截断长度（字符——summary 面的确定性上界；默认 400）。
    pub summary_max_chars: usize,
}

impl<'a> ChunkTextSource<'a> {
    pub fn new(kb: &'a Kb) -> Self {
        ChunkTextSource { kb, summary_max_chars: 400 }
    }
}

impl EvidenceTextSource for ChunkTextSource<'_> {
    fn full_text(&self, ev: &Evidence) -> String {
        match anchor_chunk(self.kb, ev) {
            Some(c) => c.text.clone(),
            None => LedgerTextSource.full_text(ev), // 锚不上 → 台账口径降级
        }
    }

    fn summary_paragraphs(&self, ev: &Evidence) -> Vec<String> {
        // 引擎 Locator 已定位到段（chunk 本身就是该段）——关键段取段内
        // 前 summary_max_chars 字符（段内要点的确定性截取）
        let full = self.full_text(ev);
        if full.chars().count() <= self.summary_max_chars {
            vec![full]
        } else {
            let head: String = full.chars().take(self.summary_max_chars).collect();
            vec![format!("{head}……（长证据已截取关键段）")]
        }
    }
}

/// 相关度供给面（批量打分——顺序敏感口径需要全序列上下文）。
pub trait RelevanceOracle {
    fn score_all(&self, evidences: &[&Evidence]) -> Vec<f32>;
}

/// 绑定顺序确定性分数（kb 不可用时的兜底口径）：该章证据绑定序越靠前
/// 相关度越高——严格递减（1.0 → 1/n），无平局（顺序即裁决）。
pub struct BindingOrderRelevance;

impl RelevanceOracle for BindingOrderRelevance {
    fn score_all(&self, evidences: &[&Evidence]) -> Vec<f32> {
        let n = evidences.len();
        evidences
            .iter()
            .enumerate()
            .map(|(i, _)| (n - i) as f32 / n as f32)
            .collect()
    }
}

/// kb 检索分相关度（S-3 `SearchHit.score` 既有口径）：章节查询面 → 命中块
/// 按文件标签关联到证据 → 取该证据的最高命中分；未命中 = 0.0（平局由
/// budget 侧按 id 字典序破——确定性不依赖 kb 迭代序）。
pub struct KbRelevance<'a> {
    pub kb: &'a Kb,
    /// 章节查询面（章节标题/条款关键词——组装侧构造的 ExactQuery）。
    pub query: ExactQuery,
}

impl RelevanceOracle for KbRelevance<'_> {
    fn score_all(&self, evidences: &[&Evidence]) -> Vec<f32> {
        let hits = self.kb.search_exact(&self.query);
        evidences
            .iter()
            .map(|ev| {
                hits.iter()
                    .filter(|h| file_matches(&h.file, &ev.source_file))
                    .map(|h| h.score as f32)
                    .fold(0.0f32, f32::max)
            })
            .collect()
    }
}

// ---------------------------------------------------------------- 工具函数
/// EngineLocator 的溯源简述（渲染面——证据块头下方的回溯锚）。
pub fn locator_brief(loc: &EngineLocator) -> String {
    match loc {
        EngineLocator::XlsxCell { sheet, cell, .. } => format!("{sheet}!{}", cell.0),
        EngineLocator::XlsxRegion { sheet, start, end, .. } => {
            format!("{sheet}!{}:{}", start.0, end.0)
        }
        EngineLocator::DocxParagraph { paragraph_index, .. } => {
            format!("docx 段落 #{paragraph_index}")
        }
        EngineLocator::DocxTableCell { table_index, row, col, .. } => {
            format!("docx 表 {table_index} 行 {row} 列 {col}")
        }
        EngineLocator::MarkdownBlock { block_id, .. } => format!("md 块 {block_id}"),
        EngineLocator::PdfPageText { page, .. } => format!("pdf 第 {page} 页"),
        EngineLocator::CanvasObject { object_id, .. } => format!("画布对象 {object_id}"),
    }
}

/// chunk 文件标签 ↔ 证据 source_file 匹配（Kb 标签口径「evidence/ 前缀或裸
/// 文件名」——尾段容错，标签对齐责任在组装侧）。
fn file_matches(chunk_file: &str, source_file: &str) -> bool {
    chunk_file == source_file || chunk_file.rsplit('/').next() == Some(source_file)
}

/// DocxParagraph 锚 ↔ chunk 块序定位（其他锚型暂无 chunk 对应面——M3 扩展）。
fn anchor_chunk<'a>(kb: &'a Kb, ev: &Evidence) -> Option<&'a Chunk> {
    let block_idx = match &ev.locator {
        EngineLocator::DocxParagraph { paragraph_index, .. } => *paragraph_index as usize,
        _ => return None,
    };
    kb.chunks
        .iter()
        .find(|c| file_matches(&c.file, &ev.source_file) && c.block_idx == block_idx)
}

/// 该章证据候选 id（保序去重）：matrix 已构建走 `response_section == section_id`
/// 关联（任务口径）；未构建时 bindings × 章节条款反查（与 derive_matrix_rows
/// 的 response_section 同源——那是本反查的派生态，两口径等价）。
fn evidence_ids_for_section(store: &DomainStore, node: &OutlineNode) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let push = |id: &str, ids: &mut Vec<String>, seen: &mut HashSet<String>| {
        if seen.insert(id.to_string()) {
            ids.push(id.to_string());
        }
    };
    if store.matrix().is_empty() {
        let req_ids: HashSet<&str> = node.requirement_ids.iter().map(|r| r.as_str()).collect();
        for b in store.evidence_bindings() {
            if req_ids.contains(b.requirement_id.as_str()) {
                push(&b.evidence_id, &mut ids, &mut seen);
            }
        }
    } else {
        for row in store.matrix() {
            if row.response_section.as_deref() == Some(node.id.as_str()) {
                for id in &row.evidence_ids {
                    push(id, &mut ids, &mut seen);
                }
            }
        }
    }
    ids
}

/// 章节规格文本（OutlineNode 的渲染面——单章的规格来源）。
fn render_section_spec(node: &OutlineNode) -> String {
    format!(
        "章节：{} {}（层级 {}，文档序 {}）\n状态：{}\n响应条款：{}\n字数配额：{} 字",
        node.id,
        node.title,
        node.level,
        node.order,
        node.status.as_str(),
        node.requirement_ids.join("、"),
        node.target_chars,
    )
}

// ---------------------------------------------------------------- 组装产物
/// 单章输入（S-1/S-2 per-section 调度的最终输入面）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionInput {
    pub section_id: String,
    /// 渲染完成的 prompt 文本（确定序：约束前缀 → 章节规格 → 证据块）。
    pub prompt_text: String,
    /// 渲染后实测 token（`estimate_tokens(prompt_text)`——8k 验收数字）。
    pub total_tokens: u64,
    pub plan: ContextPlan,
    /// 任何证据被降级/丢弃 → true（= plan.truncated_evidence 的便捷面）。
    pub truncated_evidence: bool,
}

/// 组装错误（fail-closed——全部响亮拒绝，不 panic 不静默）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextAssembleError {
    /// 空/纯空白章节 id（borrow：mdcore persistence_recovery 的拒绝模式）。
    EmptySectionId,
    /// 章节不存在。
    SectionNotFound { section_id: String },
    /// 绑定引用了不存在的证据（孤儿绑定——快照损坏面，restore 未覆盖的
    /// 引用完整性由消费侧把关：半个证据链不猜测）。
    OrphanBinding { evidence_id: String },
    /// 预算规划硬错误（约束区超红线 / id 重复 / token 账不平——透传）。
    Budget(BudgetError),
    /// 渲染后实测超红线（8k 结构保证的最后一道门——绝不放行超限）。
    RenderOverflow { actual: u64, total: u64 },
}

impl std::fmt::Display for ContextAssembleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextAssembleError::EmptySectionId => {
                write!(f, "章节 id 为空/纯空白——单章输入必须有明确归属")
            }
            ContextAssembleError::SectionNotFound { section_id } => {
                write!(f, "章节 {section_id} 不存在（先建章节树）")
            }
            ContextAssembleError::OrphanBinding { evidence_id } => write!(
                f,
                "绑定引用了不存在的证据 {evidence_id}（孤儿绑定——半个证据链，fail-closed 拒绝组装）"
            ),
            ContextAssembleError::Budget(e) => write!(f, "预算规划被拒: {e}"),
            ContextAssembleError::RenderOverflow { actual, total } => write!(
                f,
                "渲染后实测 {actual} tokens 超出预算红线 {total}（绝不放行超限——\
                 证据标记开销的估算偏差兜底门；请收紧约束或下调证据预算）"
            ),
        }
    }
}

impl std::error::Error for ContextAssembleError {}

impl From<BudgetError> for ContextAssembleError {
    fn from(e: BudgetError) -> Self {
        ContextAssembleError::Budget(e)
    }
}

// ---------------------------------------------------------------- 组装核心
/// 组装单章输入（默认供给面：项目标识约束 + 台账文本 + 绑定序相关度）。
pub fn assemble_section_input(
    store: &DomainStore,
    section_id: &str,
    budget: &ContextBudget,
) -> Result<SectionInput, ContextAssembleError> {
    assemble_section_input_with(
        store,
        section_id,
        budget,
        &BuiltinConstraintSource,
        &LedgerTextSource,
        &BindingOrderRelevance,
    )
}

/// 组装单章输入（注入供给面——kb 相关度 / chunk 文本 / 自定义约束的入口）。
///
/// 管线（只读 store，确定性）：
/// 1. 章节定位（空 id / 不存在 → 拒）；
/// 2. 约束区（永不裁剪）+ 章节规格 → 固定前缀；
/// 3. 该章证据候选（matrix 关联/绑定反查）→ 相关度 + 全文/摘要双形态；
/// 4. `budget::plan` 四阶梯规划；
/// 5. 渲染 prompt + 实测红线校验（超 8k → 拒——绝不放行超限）。
pub fn assemble_section_input_with(
    store: &DomainStore,
    section_id: &str,
    budget: &ContextBudget,
    constraints_src: &dyn ConstraintSource,
    text_src: &dyn EvidenceTextSource,
    relevance_src: &dyn RelevanceOracle,
) -> Result<SectionInput, ContextAssembleError> {
    // ---- 1. 章节定位（fail-closed：空 id / 幽灵 id 全拒）
    if section_id.trim().is_empty() {
        return Err(ContextAssembleError::EmptySectionId);
    }
    let node = store
        .sections()
        .iter()
        .find(|n| n.id == section_id)
        .ok_or_else(|| ContextAssembleError::SectionNotFound { section_id: section_id.to_string() })?;

    // ---- 2. 固定前缀（约束区 + 章节规格——budget 侧永不裁剪面）
    let constraints = constraints_src.constraint_blocks(store);
    let section_spec = SectionSpecBlock::new(node.id.clone(), render_section_spec(node));

    // ---- 3. 证据候选（绑定关联 + 相关度 + 双形态文本）
    let ev_ids = evidence_ids_for_section(store, node);
    let mut evidences: Vec<&Evidence> = Vec::with_capacity(ev_ids.len());
    for id in &ev_ids {
        let ev = store.evidence(id).map_err(|_| ContextAssembleError::OrphanBinding {
            evidence_id: id.clone(),
        })?;
        evidences.push(ev);
    }
    let relevances = relevance_src.score_all(&evidences);
    let candidates: Vec<EvidenceCandidate> = evidences
        .iter()
        .zip(relevances.into_iter())
        .map(|(ev, rel)| {
            EvidenceCandidate::new(
                ev.id.clone(),
                rel,
                text_src.full_text(ev),
                text_src.summary_paragraphs(ev),
            )
        })
        .collect();

    // ---- 4. 四阶梯规划（约束区超红线等硬错误在此响亮拒绝）
    let input = BudgetInput { constraints, section_spec, candidates };
    let plan = plan_budget(&input, budget)?;

    // ---- 5. 渲染 + 实测红线（8k 结构保证的最后一道门）
    let prompt_text = render_prompt(&input, &plan);
    let actual = estimate_tokens(&prompt_text);
    if actual > budget.total {
        return Err(ContextAssembleError::RenderOverflow { actual, total: budget.total });
    }
    Ok(SectionInput {
        section_id: section_id.to_string(),
        truncated_evidence: plan.truncated_evidence,
        total_tokens: actual,
        prompt_text,
        plan,
    })
}

// ---------------------------------------------------------------- 渲染面
/// 渲染 prompt 文本（确定序：约束前缀 → 章节规格 → 证据块）。
///
/// 标记开销与 budget 侧计量常量**双向锁定**（渲染格式的标记行不得超出
/// `EVIDENCE_MARKER_TOKENS` / `DROPPED_*` 的保守上界——超长 id 等极端构造
/// 由组装侧的渲染后实测校验兜底）。
fn render_prompt(input: &BudgetInput, plan: &ContextPlan) -> String {
    let mut out = String::new();
    // 约束前缀（永不裁剪——逐字节原样）
    out.push_str("【项目约束】（全局约束——永不裁剪）\n");
    for c in &input.constraints {
        out.push_str(&format!("[{}] {}\n", c.kind, c.text));
    }
    out.push('\n');
    // 章节规格
    out.push_str("【章节规格】\n");
    out.push_str(&input.section_spec.text);
    out.push_str("\n\n");
    // 证据块（输出序 = 计划序：relevance desc, id asc）
    let kept = plan.evidence_plan.iter().filter(|e| e.mode != EvidenceMode::Dropped).count();
    let total = plan.evidence_plan.len();
    let trunc_mark = if plan.truncated_evidence { "，truncated_evidence" } else { "" };
    out.push_str(&format!(
        "【证据材料】（按相关度降序，预算内保留 {kept}/{total} 条{trunc_mark}）\n"
    ));
    for entry in &plan.evidence_plan {
        let cand = input
            .candidates
            .iter()
            .find(|c| c.evidence_id == entry.evidence_id)
            .expect("计划条目必来自候选集（plan 与 render 同源输入）");
        match entry.mode {
            EvidenceMode::Full => {
                out.push_str(&format!("[证据 {}｜完整]\n{}\n", entry.evidence_id, cand.full_text));
            }
            EvidenceMode::Summary => {
                out.push_str(&format!(
                    "[证据 {}｜摘要 truncated_evidence]\n{}\n",
                    entry.evidence_id,
                    cand.summary_paragraphs.join("\n")
                ));
            }
            EvidenceMode::Dropped => { /* 走下方裁剪清单行——不逐条占块 */ }
        }
    }
    // 裁剪清单行（dropped 汇总——DROPPED_LINE_BASE + DROPPED_ID/id 计量面）
    let dropped: Vec<&str> = plan
        .evidence_plan
        .iter()
        .filter(|e| e.mode == EvidenceMode::Dropped)
        .map(|e| e.evidence_id.as_str())
        .collect();
    if !dropped.is_empty() {
        out.push_str(&format!(
            "【裁剪清单 truncated_evidence】{}（共 {} 条低相关证据因 token 预算被裁剪，未纳入）\n",
            dropped.join("、"),
            dropped.len(),
        ));
    }
    // 静态断言口径（注释面）：本函数产出的每证据标记行 ≤ EVIDENCE_MARKER_TOKENS、
    // 裁剪清单行 ≤ DROPPED_LINE_BASE_TOKENS + DROPPED_ID_TOKENS × n（id ≤ 16 字符）
    let _ = (EVIDENCE_MARKER_TOKENS, DROPPED_LINE_BASE_TOKENS, DROPPED_ID_TOKENS);
    out
}

// ---------------------------------------------------------------- 单元测试
#[cfg(test)]
mod tests {
    use super::*;

    /// 渲染确定性冒烟：同输入两次渲染逐字节相等（prompt 可 diff 的地基）。
    #[test]
    fn render_prompt_is_deterministic() {
        let input = BudgetInput {
            constraints: vec![ConstraintBlock::new("金额", "控制价 350 万")],
            section_spec: SectionSpecBlock::new("SEC-0001", "章节：SEC-0001 测试"),
            candidates: vec![
                EvidenceCandidate::new("EV-0001", 0.9, "全文甲".repeat(3), vec!["摘要甲".into()]),
                EvidenceCandidate::new("EV-0002", 0.5, "全文乙".repeat(3), vec!["摘要乙".into()]),
            ],
        };
        let p = plan_budget(&input, &ContextBudget::default()).unwrap();
        assert_eq!(render_prompt(&input, &p), render_prompt(&input, &p));
    }
}
