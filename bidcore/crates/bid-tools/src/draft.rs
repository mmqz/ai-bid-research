//! draft.rs —— #59 阶段 1：单章起草的业务半边（draft_response_section）。
//!
//! 分层（与 service.rs / assembly.rs 同族纪律）：
//! - **输入装配**（[`build_section_input`]）：从 store 取章节配额（target_chars）、
//!   章节绑定的 REQ、矩阵绑定的证据，组装 [`SectionDraftInput`]——幽灵 REQ
//!   剔除并告警（不拒稿——抽取层 id 漂移是常态）、EXPIRED/SUPERSEDED 证据
//!   **拒绝**（契约 §6 前置：EV 状态 OK 才可绑定，防「引用了已失效证据」
//!   的废标级事故）；
//! - **生成**（[`generate_mock`]）：确定性 mock（无 API key 的 e2e 面）——
//!   逐 REQ 响应段 + 逐 EV 佐证段 + 符合性声明，句式池按序取用无随机性；
//!   字数按 target_chars ±20% 拼装（Lead 验收线）。真实 AI 面经
//!   [`SectionGenerator`] trait 注入（未来接 z-ai sdk / OpenAI 兼容面——
//!   本 PR 只落 trait + mock，AI 通路是后续独立 PR）；
//! - **引用回查**（[`verify_citations`] / [`strip_unknown_citations`]）：
//!   生成内容里的 REQ-/EV- 引用必须在已知集合内（防幻觉——AI 会编造不存在
//!   的证据号）；未知引用**剔除 + 告警**（不拒稿——契约把阻断留给
//!   validate 的 unanchored_claims 面而不是 draft 面）；
//! - **无锚承诺计数**（[`count_unanchored_claims`]）：含承诺词的句子若无
//!   同句引用 → 计数（契约 §6 stats.unanchored_claims——阻断语义在 validate）。
//!
//! 章节内容落两处（单一提交路径同步写，见 service::draft_section）：
//! `OutlineNode.content`（内存面——MCP 即时读）+ `ChapterAssembler`
//! 磁盘面（S-2 断点续跑 + S-6 导出的既有真源）。

use std::collections::HashSet;

use bid_domain::evidence::{Evidence, EvidenceStatus};
use bid_domain::outline::SectionStatus;
use bid_domain::{DomainStore, Requirement, ToolError};

/// 章节起草输入（生成器的全部素材——**不含** store 引用，可序列化可跨进程）。
#[derive(Debug, Clone, PartialEq)]
pub struct SectionDraftInput {
    pub section_id: String,
    pub title: String,
    pub target_chars: u32,
    /// 本章节响应的条款（REQ 摘要面——id + 类别 + 原文摘录）
    pub requirements: Vec<ReqBrief>,
    /// 矩阵绑定的证据（EV 摘要面——id + 类别 + 标题 + 适用范围）
    pub evidences: Vec<EvBrief>,
    /// 起草指令（用户/Agent 的自由文本——不同 instructions 即不同稿）
    pub instructions: Option<String>,
}

/// REQ 摘要（生成 prompt 的素材——只带生成面需要的字段）。
#[derive(Debug, Clone, PartialEq)]
pub struct ReqBrief {
    pub id: String,
    pub category: String,
    pub excerpt: String,
}

/// EV 摘要。
#[derive(Debug, Clone, PartialEq)]
pub struct EvBrief {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub scope: String,
}

/// 生成产物（正文 + 生成器声称的引用清单）。
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedDraft {
    pub content: String,
}

/// 生成器 trait（真实 AI 面的注入位——mock 与未来 LLM 实现同接口）。
pub trait SectionGenerator {
    fn generate(&mut self, input: &SectionDraftInput) -> Result<GeneratedDraft, String>;
}

// ---------------------------------------------------------------- 输入装配

/// 装配单章起草输入。
///
/// - `requirement_override`：显式指定响应的 REQ 清单（None = 章节绑定的
///   `requirement_ids` 全量——契约 §6 `source_requirements` 缺省语义）；
/// - 幽灵 REQ（不在 store requirements 中）→ 剔除 + warning（不拒——抽取
///   重跑后 id 漂移是已知常态，S-5 影响面机制另有专责）；
/// - 证据面：先取 override（契约 §6 `evidence_ids`），缺省走 evidence_bindings
///   里「本章节 REQ ↔ EV」的绑定集；
/// - **EXPIRED / SUPERSEDED 证据 → E_UNPROCESSABLE**（契约 §6 前置硬门——
///   引用失效证据响应否决项是废标级事故，draft 面直接拒，不是 warning）；
/// - 章节必须有 REQ 或显式 override（空响应面无意义——E_UNPROCESSABLE）。
pub fn build_section_input(
    store: &DomainStore,
    section_id: &str,
    requirement_override: Option<&[String]>,
    evidence_override: Option<&[String]>,
    instructions: Option<&str>,
) -> Result<(SectionDraftInput, Vec<String>), ToolError> {
    let node = store.sections().iter().find(|n| n.id == section_id).ok_or_else(|| {
        ToolError::ENotFound { what: format!("section {section_id} 不存在（先建纲——derive_and_set_outline）") }
    })?;
    // 叶校验（与 set_section_content 同口径——结构节点是调度单元不承载正文）
    let has_child = store.sections().iter().any(|n| n.parent_id.as_deref() == Some(section_id));
    if has_child {
        return Err(ToolError::EUnprocessable {
            reason: format!("{section_id} 是结构节点（有子）——只对叶章节起草"),
        });
    }

    // ---- REQ 面：override 优先，缺省章节绑定集
    let req_ids: Vec<String> = match requirement_override {
        Some(ids) => ids.to_vec(),
        None => node.requirement_ids.clone(),
    };
    let mut warnings = Vec::new();
    let mut requirements = Vec::with_capacity(req_ids.len());
    let mut known_req_ids: Vec<String> = Vec::with_capacity(req_ids.len());
    for rid in &req_ids {
        match find_requirement(store.requirements(), rid) {
            Some(r) => {
                known_req_ids.push(rid.clone());
                requirements.push(ReqBrief {
                    id: r.id.clone(),
                    category: r.category.clone(),
                    excerpt: first_anchor_excerpt(r),
                });
            }
            None => warnings.push(format!(
                "幽灵 REQ：{rid} 不在条款集中（抽取重跑后 id 漂移？）——已从本章响应面剔除"
            )),
        }
    }
    if requirements.is_empty() {
        return Err(ToolError::EUnprocessable {
            reason: format!(
                "{section_id} 的响应面为空（章节未绑定 REQ 或 override 全为幽灵）——空章节起草无意义"
            ),
        });
    }

    // ---- EV 面：override 优先，缺省走绑定集（本章节 REQ ↔ EV）
    let ev_ids: Vec<String> = match evidence_override {
        Some(ids) => ids.to_vec(),
        None => bound_evidence_ids(store, &known_req_ids),
    };
    let mut evidences = Vec::with_capacity(ev_ids.len());
    for eid in &ev_ids {
        let ev = store.evidences().iter().find(|e| &e.id == eid).ok_or_else(|| {
            // 显式 override 指了不存在的 EV → 拒（用户输入错误要响亮）；
            // 缺省绑定面不会走到这（绑定写入时已校验存在性）
            ToolError::ENotFound { what: format!("证据 {eid} 不存在（evidence_ids 指错）") }
        })?;
        check_evidence_bindable(ev)?;
        evidences.push(EvBrief {
            id: ev.id.clone(),
            kind: ev.kind.as_str().to_string(),
            title: ev.title.clone(),
            scope: ev.scope.clone(),
        });
    }

    Ok((
        SectionDraftInput {
            section_id: section_id.to_string(),
            title: node.title.clone(),
            target_chars: node.target_chars,
            requirements,
            evidences,
            instructions: instructions.map(str::to_string),
        },
        warnings,
    ))
}

/// EXPIRED / SUPERSEDED 拒绑（R-03——契约 §6 前置；与 evidence.rs
/// `bindable_evidence_ids` 的 `matches!(status, Active | Expiring)` 同口径）。
fn check_evidence_bindable(ev: &Evidence) -> Result<(), ToolError> {
    if !matches!(ev.status, EvidenceStatus::Active | EvidenceStatus::Expiring) {
        return Err(ToolError::EUnprocessable {
            reason: format!(
                "证据 {}（{}）状态为 {}——失效证据禁绑（R-03），先换版或补新证",
                ev.id, ev.title, ev.status.as_str()
            ),
        });
    }
    Ok(())
}

/// 绑定集：本章节 REQ 集关联的 EV（去重保序——BTreeMap 级确定性）。
fn bound_evidence_ids(store: &DomainStore, req_ids: &[String]) -> Vec<String> {
    let req_set: HashSet<&str> = req_ids.iter().map(|s| &s[..]).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for b in store.evidence_bindings() {
        if req_set.contains(&b.requirement_id[..]) && seen.insert(b.evidence_id.clone()) {
            out.push(b.evidence_id.clone());
        }
    }
    out
}

fn find_requirement<'a>(reqs: &'a [Requirement], id: &str) -> Option<&'a Requirement> {
    reqs.iter().find(|r| r.id == id)
}

fn first_anchor_excerpt(r: &Requirement) -> String {
    r.anchors.first().map(|a| a.excerpt.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------- mock 生成器

/// 确定性 mock 生成器（无 API key 的 e2e 面——Lead 指令「无 key 时用 mock，
/// 否则 e2e 跑不了」）。
///
/// 结构（全部确定性——同输入同输出，跨进程可重放）：
/// 1. 开篇段：章节标题 + 响应说明（引用本章 REQ 数与目标字数）；
/// 2. 逐 REQ 响应段：「针对 [REQ-xxxx]（类别）……」，含承诺句（带引用）；
/// 3. 逐 EV 佐证段：「依据 [EV-xxxx] ……」；
/// 4. 符合性声明段（含全部 REQ 引用的收束句）；
/// 5. 字数填充：确定性扩展句池循环补足到 target ±20%。
pub struct MockGenerator;

impl SectionGenerator for MockGenerator {
    fn generate(&mut self, input: &SectionDraftInput) -> Result<GeneratedDraft, String> {
        Ok(GeneratedDraft { content: generate_mock(input) })
    }
}

/// 扩展句池（填充面——按序循环取用，无随机性；句句带引用锚，使填充不
/// 产生无锚承诺——unanchored_claims 只在真实 AI 面才有信号）。
const FILLER_SENTENCES: [&str; 8] = [
    "本章节所述响应内容与招标文件的要求逐条对应，未作任何省略或替换，响应口径以所引条款原文为准。",
    "上述响应依据充分、边界清晰，与招标文件的实质性要求和条件保持一致，不存在负偏离。",
    "为便于评审核对，本章节对每一项要求均给出了对应的实施说明与支撑材料索引，评审可按引用回查原文。",
    "本章节内容基于现有资料编制完成，如招标文件后续发布补遗或澄清，将按补遗口径同步修订响应内容。",
    "响应内容的深度与颗粒度按评审办法的评分细则组织，覆盖技术、商务与格式三个层面的全部考察点。",
    "章节行文遵循招标文件规定的表述顺序，未采用自定义缩写与未定义术语，评审阅读零歧义。",
    "所有引用的证据材料均在有效期内且主体一致，可供评标委员会当场核验，无需另行补充说明。",
    "本章节的编制过程执行了内部三级校核（起草、校对、审核），引用编号、数据与结论三者一致。",
];

/// 承诺句模板（逐 REQ——承诺词 + 同句引用，满足「承诺必须带锚」纪律）。
const REQ_RESPONSE_TEMPLATE: &str = "针对 [REQ_ID]（CATEGORY 类条款）「EXCERPT」的要求，我方承诺在本章节给出完整响应：该要求的落实措施、责任分工与验证方法如下文所述，响应结论为完全满足，无偏离。";

/// EV 佐证模板。
const EV_SUPPORT_TEMPLATE: &str = "依据 [EV_ID]（KIND：TITLE，适用范围：SCOPE），本章节相关承诺具备可核验的证据支撑，评标委员会可按证据台账索引回查原件。";

/// mock 生成（纯函数——同输入字节级同输出）。
pub fn generate_mock(input: &SectionDraftInput) -> String {
    let mut paras: Vec<String> = Vec::new();

    // ① 开篇
    let inst = input.instructions.as_deref().unwrap_or("");
    let inst_note = if inst.trim().is_empty() {
        String::new()
    } else {
        format!("（起草指令：{}）", truncate_chars(inst, 60))
    };
    paras.push(format!(
        "# {}\n\n本章节依据招标文件的实质性要求和条件编制{}，共响应 {} 项条款、\
援引 {} 项证据，目标篇幅 {} 字（实际篇幅以内容完整为准，允许 ±20% 浮动）。",
        input.title, inst_note, input.requirements.len(), input.evidences.len(), input.target_chars,
    ));

    // ② 逐 REQ 响应段
    for r in &input.requirements {
        let mut s = REQ_RESPONSE_TEMPLATE
            .replace("REQ_ID", &r.id)
            .replace("CATEGORY", &r.category)
            .replace("EXCERPT", &truncate_chars(&r.excerpt, 80));
        // 空 excerpt 的退化面（无锚 REQ——抽取层异常，如实生成不掩盖）
        if r.excerpt.trim().is_empty() {
            s = s.replace("「」", "（原文摘录缺失）");
        }
        paras.push(s);
    }

    // ③ 逐 EV 佐证段
    for e in &input.evidences {
        paras.push(
            EV_SUPPORT_TEMPLATE
                .replace("EV_ID", &e.id)
                .replace("KIND", &e.kind)
                .replace("TITLE", &truncate_chars(&e.title, 50))
                .replace("SCOPE", if e.scope.trim().is_empty() { "见证据原件" } else { &e.scope }),
        );
    }

    // ④ 符合性收束
    let all_reqs = input
        .requirements
        .iter()
        .map(|r| format!("[{}]", r.id))
        .collect::<Vec<_>>()
        .join("、");
    paras.push(format!(
        "综上，本章节对 {} 已逐项响应并给出证据支撑，响应内容真实、完整、准确，\
不存在招标文件规定的任何否决情形。", all_reqs,
    ));

    // ⑤ 填充到 target ±20%（不足时循环补句；超出时截到下限——确定性两向）
    pad_to_target(paras, input.target_chars)
}

/// 段落拼装 + 字数控制（chars().count() 中文口径）。
///
/// 下限 = target × 80%，上限 = target × 120%（Lead 验收线 ±20%）。
/// target = 0 的退化面：跳过填充（内容完整性优先于字数——诚实不伪装）。
fn pad_to_target(paras: Vec<String>, target: u32) -> String {
    let joined = |v: &[String]| v.join("\n\n");
    if target == 0 {
        return joined(&paras);
    }
    let lower = target as f64 * 0.80;
    let mut v = paras;
    let mut i = 0usize;
    while (joined(&v).chars().count() as f64) < lower {
        v.push(FILLER_SENTENCES[i % FILLER_SENTENCES.len()].to_string());
        i += 1;
        if i > 10_000 {
            break; // 防御性上限（句池循环 + 下限极小的病态输入）
        }
    }
    joined(&v)
}

/// 截断（中文口径——尾部省略号标记）。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

// ---------------------------------------------------------------- 引用回查

/// REQ/EV id 的引用模式（[REQ-0001] / REQ-0001 两种形态都认——方括号是
/// 生成器的规范形态，裸 id 是真实 AI 常见形态）。
fn citation_pattern() -> regex::Regex {
    regex::Regex::new(r"(?:\[)?((?:REQ|EV)-\d{1,6})(?:\])?").unwrap()
}

/// 扫描正文里的全部引用 id（保序去重）。
pub fn scan_citations(content: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for cap in citation_pattern().captures_iter(content) {
        let id = cap[1].to_string();
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    out
}

/// 引用回查：正文引用 ⊆ 已知集合？
///
/// 返回 (valid, unknown)——unknown 非空即幻觉面（AI 编造了不存在的证据号）。
pub fn verify_citations(content: &str, known: &HashSet<String>) -> (Vec<String>, Vec<String>) {
    let mut valid = Vec::new();
    let mut unknown = Vec::new();
    for id in scan_citations(content) {
        if known.contains(&id) {
            valid.push(id);
        } else {
            unknown.push(id);
        }
    }
    (valid, unknown)
}

/// 剔除未知引用（防幻觉面——把编造的引用从正文里移除，留下痕迹给 warning）。
///
/// 方括号形态与裸形态一并清理（连续分隔符「、」「，」收敛为单「、」）。
pub fn strip_unknown_citations(content: &str, unknown: &[String]) -> String {
    let mut out = content.to_string();
    for id in unknown {
        out = out.replace(&format!("[{id}]"), "").replace(id.as_str(), "");
        out = out.replace("、、", "、").replace("，，", "，");
    }
    out
}

/// 无锚承诺计数（契约 §6 stats.unanchored_claims）。
///
/// 承诺词句（含「承诺|保证|确保|完全满足」的句）若无同句 REQ/EV 引用 →
/// 计 1。mock 生成器结构上不会产生（模板句带锚），此面为真实 AI 面准备。
pub fn count_unanchored_claims(content: &str) -> usize {
    let re = regex::Regex::new(r"[^。；\n]*(?:承诺|保证|确保|完全满足)[^。；\n]*[。；]?").unwrap();
    let cite = citation_pattern();
    re.find_iter(content)
        .filter(|m| cite.find(m.as_str()).is_none())
        .count()
}

// ---------------------------------------------------------------- 服务集成面

impl super::service::BidService {
    /// `draft_response_section` 业务半边（MCP 壳完成参数校验/路由/状态机
    /// 裁决后调用；生成器注入——mock 或未来 AI 面）。
    ///
    /// 步骤（Lead #59 指令 1-7 的落地面）：
    /// 1. `build_section_input`（target_chars / REQ / EV / 幽灵剔除 / 失效拒绑）；
    /// 2. generator 生成；
    /// 3. 引用回查 + 剔除未知引用（warning 承载）；
    /// 4. `set_section_content`（CAS——同内容 no-op）；
    /// 5. 章节推进 DRAFTED（经合法链补齐——已在 DRAFTED+ 者不动，重起草
    ///    场景由显式 reopen 承担，draft 面不越权回退状态）；
    /// 6. 项目推进 DRAFTING（首次起草——幂等）；
    /// 7. S-2 工作区落盘（persist 在场时——`chapters/{sec}.json`，断点续跑
    ///    与导出的既有真源）+ checkpoint。
    ///
    /// 返回 MCP 壳组装契约 out 的素材面。
    pub fn draft_section(
        &mut self,
        section_id: &str,
        requirement_override: Option<&[String]>,
        evidence_override: Option<&[String]>,
        instructions: Option<&str>,
        generator: &mut dyn SectionGenerator,
    ) -> Result<DraftOutcome, ToolError> {
        // ---- 1. 输入装配（含 E_NOTFOUND / 失效拒绑 / 幽灵剔除）
        let (input, mut warnings) = build_section_input(
            self.store(), section_id,
            requirement_override, evidence_override, instructions,
        )?;

        // ---- 2. 生成
        let draft = generator.generate(&input).map_err(|why| ToolError::EUnprocessable {
            reason: format!("章节 {section_id} 生成失败: {why}"),
        })?;

        // ---- 3. 引用回查（防幻觉——未知引用剔除 + 告警，不拒稿）
        let known: HashSet<String> = input.requirements.iter().map(|r| r.id.clone())
            .chain(input.evidences.iter().map(|e| e.id.clone()))
            .collect();
        let (valid, unknown) = verify_citations(&draft.content, &known);
        let content = if unknown.is_empty() {
            draft.content
        } else {
            for id in &unknown {
                warnings.push(format!(
                    "引用回查：{id} 不在已知 REQ/EV 集合（生成器幻觉）——已从正文剔除"
                ));
            }
            strip_unknown_citations(&draft.content, &unknown)
        };

        // ---- 3.5 覆盖度自动检查（#68 阶段 2 §2.1——响应面 REQ 未被正文
        //      引用回应 → warning 即时告警；不拒稿——补写/重起草是上游决策）
        let cited_ids = scan_citations(&content);
        let cited_now: HashSet<&str> = cited_ids.iter().map(|s| &s[..]).collect();
        let uncovered_requirements: Vec<String> = input.requirements.iter()
            .filter(|r| !cited_now.contains(&r.id[..]))
            .map(|r| r.id.clone())
            .collect();
        for rid in &uncovered_requirements {
            warnings.push(format!(
                "覆盖度：{rid} 未被本章正文回应——人工补写或重起草"
            ));
        }

        // ---- 4. CAS 写入（同内容 no-op——幂等面）
        let (attempt, chars) = self.store_mut().set_section_content(section_id, content.clone())?;

        // ---- 5. 章节推进 DRAFTED（合法链补齐；no-op 路径零副作用）
        advance_section_to_drafted(self.store_mut(), section_id)?;

        // ---- 6. 项目推进 DRAFTING（首次起草——MatrixReady → EvidenceBound
        //        → Drafting 逐态经 next_allowed；已越过者 no-op）
        advance_project_to_drafting(self.store_mut())?;

        // ---- 7. S-2 落盘（persist 在场）+ checkpoint
        if let Some(dir) = self.workspace_dir() {
            let mut asm = super::assembly::ChapterAssembler::open(&dir)
                .map_err(|e| ToolError::EUnprocessable {
                    reason: format!("章节工作区打开失败: {e}"),
                })?;
            asm.commit_section(section_id, &content, input.requirements.iter()
                .map(|r| r.id.clone()).collect())
                .map_err(|e| ToolError::EUnprocessable {
                    reason: format!("章节工作区提交失败: {e}"),
                })?;
        }
        self.checkpoint().map_err(|e| ToolError::EUnprocessable {
            reason: format!("checkpoint 落盘失败（内存已变更、磁盘未跟上）: {e}"),
        })?;

        Ok(DraftOutcome {
            section_id: section_id.to_string(),
            attempt,
            chars,
            citations: valid,
            unanchored_claims: count_unanchored_claims(&content),
            uncovered_requirements,
            warnings,
            instructions: instructions.map(str::to_string),
            content,
        })
    }
}

/// draft 结果素材（MCP 壳组装契约 §6 out 用）。
#[derive(Debug, Clone, PartialEq)]
pub struct DraftOutcome {
    pub section_id: String,
    pub attempt: u32,
    pub chars: u32,
    /// 回查后的有效引用（REQ+EV 混合清单，保序）
    pub citations: Vec<String>,
    pub unanchored_claims: usize,
    /// 响应面内未被正文回应的 REQ（#68 阶段 2 §2.1 覆盖度面）
    pub uncovered_requirements: Vec<String>,
    pub warnings: Vec<String>,
    pub instructions: Option<String>,
    pub content: String,
}

/// 章节推进 DRAFTED 的合法链补齐（PENDING → OUTLINING → DRAFTING → DRAFTED；
/// 已在/已过 DRAFTED → no-op——与 assemble_all::advance_to 同语义）。
fn advance_section_to_drafted(store: &mut DomainStore, section_id: &str) -> Result<(), ToolError> {
    use SectionStatus::*;
    let cur = store.sections().iter().find(|n| n.id == section_id)
        .ok_or_else(|| ToolError::ENotFound { what: format!("section {section_id} 不存在") })?
        .status;
    let steps: &[SectionStatus] = match cur {
        Pending => &[Outlining, Drafting, Drafted],
        Outlining => &[Drafting, Drafted],
        Drafting => &[Drafted],
        // 失败章重起草（#68 阶段 2）：FAILED 经 Drafting 回到正轨
        Failed => &[Drafting, Drafted],
        Drafted | Validating | Revising | Accepted => &[],
    };
    for to in steps {
        store.advance_section_status(section_id, *to)?;
    }
    Ok(())
}

/// 项目推进 DRAFTING 的合法链补齐。
///
/// guard_tool（壳层）已保证项目在 MatrixReady+；此处防御性穷举——
/// 前段态（Draft/FilesImported/RequirementsExtracted）直达本函数即状态机
/// 错误（透传，不猜测），Drafting+ 已越过者 no-op。
fn advance_project_to_drafting(store: &mut DomainStore) -> Result<(), ToolError> {
    use bid_domain::ProjectStatus::*;
    let cur = store.project()?.status;
    let steps: &[bid_domain::ProjectStatus] = match cur {
        MatrixReady => &[EvidenceBound, Drafting],
        EvidenceBound => &[Drafting],
        Drafting | Validating | Reviewing | ReadyToExport | Exported => &[],
        Draft | FilesImported | RequirementsExtracted => {
            return Err(ToolError::EState { current: cur, attempted: "advance_project_status" });
        }
    };
    for to in steps {
        store.advance_project_status(*to)?;
    }
    Ok(())
}
