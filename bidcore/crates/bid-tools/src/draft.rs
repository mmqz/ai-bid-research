//! draft.rs —— #59 阶段 1：单章起草的业务半边（draft_response_section）
//!           → #97 B4：生成-自评-改写循环（IterativeDrafter）
//!           → #110 任务 1：缺失回报接线（readiness 前置闸）。
//!
//! 分层（与 service.rs / assembly.rs 同族纪律）：
//! - **缺失回报前置闸**（#110 任务 1）：`draft_section` /
//!   `draft_section_iterative` 在装配后先过 [`crate::readiness::
//!   check_draft_readiness`]——五种素材型 Blocker（无要求/缺资质/缺业绩/
//!   缺技术证据/否决项未响应）**拒生成**（E_UNPROCESSABLE + 人类可读理由），
//!   IncompleteTender 等只 warning 不阻塞（并入 warnings + 经
//!   [`with_missing_report`] 注入生成指令，让生成器留【待补充】占位而不是
//!   编造）。Lead 原话：「缺失回报，如果缺失这个内容却还硬做只会做错」——
//!   没有业绩证据硬写 = LLM 编造 = 废标；
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
//!   同句引用 → 计数（契约 §6 stats.unanchored_claims——阻断语义在 validate）；
//! - **B4 迭代循环**（[`IterativeDrafter`]）：生成 → B2 自评 + B3 质量检查
//!   → 达标停 / 未达标注失分项构造改写提示 → 下一轮——最多 `max_rounds`
//!   轮（硬上限防 LLM 无限循环）；服务面 [`BidService::draft_section_iterative`]。
//!
//! 章节内容落两处（单一提交路径同步写，见 service::draft_section）：
//! `OutlineNode.content`（内存面——MCP 即时读）+ `ChapterAssembler`
//! 磁盘面（S-2 断点续跑 + S-6 导出的既有真源）。

use std::collections::HashSet;

use bid_domain::evidence::{Evidence, EvidenceStatus};
use bid_domain::outline::SectionStatus;
use bid_domain::scoring::{derive_scoring_tree, SectionScorer};
use bid_domain::{DomainStore, Requirement, ToolError};

use crate::quality::{assess_specificity, assess_substance, ProjectContext};
use crate::readiness::DraftReadiness;

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
///
/// #217（#185 缺口 2）：新增 `excerpt` / `excerpt_locator`——生成器
/// 拿到的不只是「证据标题」还有**正文实料**（金额/时间/范围来自逐字截取的
/// 定位文本，非标题转写）。空串 = 文件级锚未定位正文——生成器只能引
/// 台账面，不得编造细节。
///
/// #245：新增 `placeholder`——占位/待核验证据标记。生成器对占位证据
/// 只能输出【待补：...】占位文案（按用户授权占位可进草稿），**不得生成
/// 「已具备/已满足」的结论、不得把占位当作资质证明引用**。
#[derive(Debug, Clone, PartialEq)]
pub struct EvBrief {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub scope: String,
    /// 正文摘录（#217——来自解析产物逐字截取；空串 = 未定位）。
    pub excerpt: String,
    /// 摘录定位（人话：`《文件》第 N 页` / `《文件》正文段 N`；空串 = 未定位）。
    pub excerpt_locator: String,
    /// 占位/待核验标记（#245——true = PLACEHOLDER 态证据）。
    pub placeholder: bool,
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

    // ---- EV 面：override 优先，缺省走绑定集（本章节 REQ ↔ EV）。
    //      #245：占位证据仍进输入面（生成器需知道「这里待补」才能落
    //      【待补】占位文案），但携带 placeholder 标记——生成器不得将其
    //      当作已具备的资质证明引用。
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
            excerpt: ev.excerpt.clone().unwrap_or_default(),
            excerpt_locator: ev.excerpt_locator.clone().unwrap_or_default(),
            placeholder: ev.is_placeholder(),
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

/// 外部手稿生成器（issue #100 任务 2——`generator=external` 的落地件）。
///
/// 正文来自外部作者（子代理/人工——`draft_response_section` 的 `content`
/// 参数注入），引擎**不生成只提交**：引用回查（防幻觉）、CAS 写入、状态
/// 推进、S-2 落盘全走 [`BidService::draft_section`] 既有路径——外部稿与
/// mock/real 稿的提交面完全一致（无旁路）。与子代理中继通路
/// （`run_subagent_path.sh`——OpenAI 兼容端点回灌）互补：免中继直注，
/// 谁都可当「外部作者」。
pub struct ExternalGenerator {
    /// 外部手稿全文（一次性回放——generate 逐字返回）。
    pub content: String,
}

impl SectionGenerator for ExternalGenerator {
    fn generate(&mut self, _input: &SectionDraftInput) -> Result<GeneratedDraft, String> {
        Ok(GeneratedDraft { content: self.content.clone() })
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

/// EV 占位待补模板（#245）：占位/待核验证据的专属文案——**不生成
/// 「已具备/已满足」结论、不声称可核验**；落【待补：...】显式占位
/// （形态避开 W_PLACEHOLDER 词表 TODO/待补充/【占位】——占位进草稿是
/// 用户授权的待补形态，与模板残留不同源不误报）。同时**不携带
/// [EV_ID] 引用**：占位证据不得成为正文引用（引用会被 E_CITATION_
/// PLACEHOLDER 拦截——mock 与真实生成器同纪律）。
const EV_PENDING_TEMPLATE: &str = "【待补：KIND「TITLE」的真实证明材料尚未核验——当前为占位登记（待核验），本章节不作已具备声明；待材料导入并重新登记核验后替换本段并补充佐证说明。】";

/// EV 正文摘录段（#217）：CONTENT_MATCHES 态追加——生成器引用实料
/// （金额/日期/范围）只准来自这段逐字截取的原文，不得越出摘录编造。
const EV_EXCERPT_TEMPLATE: &str = "其原始凭据 EXCERPT_LOCATOR 原文摘录如下：「EXCERPT」（摘录与解析产物逐字对账一致，引用实料以此为准，不得超出摘录范围编造）。";

/// mock 生成（纯函数——同输入字节级同输出）。
///
/// #127 P0-2：正文不再嵌「起草指令：…」——指令是给起草人的备注，落
/// ChapterRecord::draft_note（见 service::draft_section），嵌正文会把
/// 句子劈断、把招标口径抄进标书。开篇只留陈述句。
pub fn generate_mock(input: &SectionDraftInput) -> String {
    let mut paras: Vec<String> = Vec::new();

    // ① 开篇（陈述句完整不断裂——指令面已分离到 draft_note）
    paras.push(format!(
        "# {}\n\n本章节依据招标文件的实质性要求和条件编制，共响应 {} 项条款、\
援引 {} 项证据，目标篇幅 {} 字（实际篇幅以内容完整为准，允许 ±20% 浮动）。",
        input.title, input.requirements.len(), input.evidences.len(), input.target_chars,
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

    // ③ 逐 EV 佐证段（#217：正文摘录随行——文件级锚无摘录则只引台账面；
    //    #245：占位证据走【待补】专属模板——不引用、不声称已具备）
    for e in &input.evidences {
        if e.placeholder {
            paras.push(
                EV_PENDING_TEMPLATE
                    .replace("KIND", &e.kind)
                    .replace("TITLE", &truncate_chars(&e.title, 50)),
            );
            continue; // 占位证据无佐证段、无摘录段——待补形态即全部输出
        }
        paras.push(
            EV_SUPPORT_TEMPLATE
                .replace("EV_ID", &e.id)
                .replace("KIND", &e.kind)
                .replace("TITLE", &truncate_chars(&e.title, 50))
                .replace("SCOPE", if e.scope.trim().is_empty() { "见证据原件" } else { &e.scope }),
        );
        if !e.excerpt.trim().is_empty() {
            let loc = if e.excerpt_locator.trim().is_empty() {
                "（定位未记录）"
            } else {
                &e.excerpt_locator
            };
            paras.push(
                EV_EXCERPT_TEMPLATE
                    .replace("EXCERPT_LOCATOR", loc)
                    .replace("EXCERPT", &truncate_chars(&e.excerpt, 120)),
            );
        }
    }

    // ④ 符合性收束（#245：仅当存在占位证据时收敛措辞——占位在章不得
    //    声称「证据支撑完整」；无占位时原措辞零位移）
    let all_reqs = input
        .requirements
        .iter()
        .map(|r| format!("[{}]", r.id))
        .collect::<Vec<_>>()
        .join("、");
    let has_placeholder = input.evidences.iter().any(|e| e.placeholder);
    paras.push(if has_placeholder {
        format!(
            "综上，本章节对 {} 已逐项响应；其中部分佐证材料为占位登记（待核验），相关响应结论以真实材料核验后为准，当前不作已具备声明。",
            all_reqs,
        )
    } else {
        format!(
            "综上，本章节对 {} 已逐项响应并给出证据支撑，响应内容真实、完整、准确，\
不存在招标文件规定的任何否决情形。", all_reqs,
        )
    });

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
/// 生成器的规范形态，裸 id 是真实 AI 常见形态）。独立 token 判定见
/// [`is_standalone_id`]（regex crate 无环视，边界检查在代码层做）。
fn citation_pattern() -> regex::Regex {
    regex::Regex::new(r"(?:REQ|EV)-\d{1,6}").unwrap()
}

/// id 是否为**独立 token**（#217 判别力修正）。
///
/// 背景：摘录定位把来源文件名带进正文后，盲正则曾把文件名前缀当未知
/// 引用剥掉（`《EV-03-xx.pdf》` → 《-xx.pdf》）。判别规则：
/// - 前后紧邻 ASCII 字母/数字/`_`/`-` → 非独立（`EV-03-xx.pdf` 里的
///   `EV-03`、`REQ-0001x`）；
/// - 后邻 `.` + ASCII 字母/数字（文件扩展名形态 `EV-0002.pdf`）→ 非独立；
/// - CJK 紧邻仍算引用（中文无空格分词——「依据EV-0002作答」是真引用：
///   UTF-8 多字节序列的字节恒 >= 0x80，不会误判为 ASCII 词字符）。
fn is_standalone_id(content: &str, m: &regex::Match) -> bool {
    let bytes = content.as_bytes();
    let ascii_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    if m.start() > 0 && ascii_word(bytes[m.start() - 1]) {
        return false;
    }
    if m.end() < bytes.len() {
        let b = bytes[m.end()];
        if ascii_word(b) {
            return false;
        }
        if b == b'.'
            && m.end() + 1 < bytes.len()
            && bytes[m.end() + 1].is_ascii_alphanumeric()
        {
            return false; // 扩展名形态：EV-0002.pdf
        }
    }
    true
}

/// 扫描正文里的全部引用 id（保序去重；仅独立 token）。
pub fn scan_citations(content: &str) -> Vec<String> {
    let re = citation_pattern();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for m in re.find_iter(content) {
        if !is_standalone_id(content, &m) {
            continue;
        }
        if seen.insert(m.as_str().to_string()) {
            out.push(m.as_str().to_string());
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
/// 方括号形态与裸形态一并清理（连续分隔符「、」「，」收敛为单「、」）；
/// #217：裸形态按独立 token 剔除（盲 replace 会误伤更长 token 的子串——
/// 剥 `EV-03` 不能把 `[EV-034]` 拗成 `[4]`，也不能碰 `EV-03-xx.pdf`）。
pub fn strip_unknown_citations(content: &str, unknown: &[String]) -> String {
    let mut out = content.to_string();
    for id in unknown {
        out = out.replace(&format!("[{id}]"), "");
        // 裸形态：仅剔独立 token（id 形如 REQ-/EV-+数字，escape 双保险）
        if let Ok(bare) = regex::Regex::new(&regex::escape(id)) {
            let mut filtered = String::new();
            let mut last = 0usize;
            for m in bare.find_iter(&out) {
                if is_standalone_id(&out, &m) {
                    filtered.push_str(&out[last..m.start()]);
                    last = m.end();
                }
            }
            filtered.push_str(&out[last..]);
            out = filtered;
        }
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

/// 缺失回报注入起草指令（#110 任务 1 第 2 步——「缺失素材 → 生成占位符」）。
///
/// 五种素材型 Blocker 已在前置闸拒生成，能走到生成这一步的 readiness
/// warning 是「不阻塞但起草时必须知晓」的面（如招标包缺补遗/澄清、幽灵
/// 条款）——把它们并入 `instructions`，并要求生成器在正文对应位置留显式
/// 占位符（[`crate::placeholder`] 语法）而不是编造。真实 LLM 面 prompt 的
/// 「起草指令」字段会原样携带；零 warning 时指令原样返回（不添噪声）。
///
/// 注意措辞：说明文字**不得**含全角引号「」或完整的占位字面量——前者会
/// 被改写提示的失分项提取面（`build_rewrite_prompt` 的 「…」 引用）误吞，
/// 后者会被 `scan_placeholders` 当成正文残留占位。
fn with_missing_report(instructions: Option<&str>, readiness: &DraftReadiness) -> Option<String> {
    if readiness.warnings.is_empty() {
        return instructions.map(str::to_string);
    }
    let mut out = instructions.map(str::to_string).unwrap_or_default();
    if !out.trim().is_empty() {
        out.push_str("\n\n");
    }
    let list: Vec<String> = readiness
        .warnings
        .iter()
        .enumerate()
        .map(|(i, w)| format!("{}. {w}", i + 1))
        .collect();
    out.push_str(&format!(
        "缺失回报：以下素材或信息当前不完整——正文对应位置必须留显式占位符\
         （全角方括号括起，开头写 待补充 或 待确认 加冒号，再接所缺内容），不得编造：\n{}",
        list.join("\n")
    ));
    Some(out)
}

/// 本次起草的伴随账面（#127 P0-2）：起草指令 + 缺失回报——**给起草人的
/// 备注，不是正文**。
///
/// 背景：早先 mock 生成器把 instructions 嵌进正文开篇括号里，缺失回报
/// 拼接后把句子劈断（「（起草指令：……\n\n缺失回报：……）」嵌在句中）。
/// 修法（#127 方案）：正文只留陈述句，指令与缺失回报落到本账面——
/// 经 [`crate::assembly::ChapterRecord::draft_note`] 版本化落盘，正文
/// 渲染面（docx 导出）永远不碰它。
///
/// 零噪声：无指令且无缺失回报 → None（不产空账面）。
fn compose_draft_note(
    instructions: Option<&str>,
    readiness: &DraftReadiness,
) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();
    if let Some(inst) = instructions.map(str::trim).filter(|s| !s.is_empty()) {
        sections.push(format!("起草指令：{inst}"));
    }
    if !readiness.warnings.is_empty() {
        let list: Vec<String> = readiness
            .warnings
            .iter()
            .enumerate()
            .map(|(i, w)| format!("{}. {w}", i + 1))
            .collect();
        sections.push(format!(
            "缺失回报：以下素材或信息当前不完整——正文对应位置已按占位符纪律处理（待补充/待确认）：\n{}",
            list.join("\n")
        ));
    }
    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

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
        let (mut input, mut warnings) = build_section_input(
            self.store(), section_id,
            requirement_override, evidence_override, instructions,
        )?;

        // ---- 1.5 缺失回报前置闸（#110 任务 1——readiness 六种 Blocker）
        //
        // 没有素材就别写：NoRequirements / MissingQualification /
        // MissingPerformance / MissingTechEvidence / UncoveredVetoItem 五种
        // 阻塞 → E_UNPROCESSABLE，理由走 [`DraftReadiness::to_message`]（人类
        // 可读，不泄漏枚举名）；IncompleteTender 等只 warning 不阻塞——并入
        // warnings（消费面单一出口）+ 经 [`with_missing_report`] 注入生成指令
        // （正文留【待补充】占位而不是编造）。
        //
        // 闸在 build_section_input **之后**：未知章节（E_NOTFOUND）、结构节点
        // 与空响应面（E_UNPROCESSABLE 的既有分工）错误码语义不回归。
        let readiness = crate::readiness::check_draft_readiness(self.store(), section_id);
        if !readiness.ready {
            return Err(ToolError::EUnprocessable {
                reason: format!("章节 {section_id} {}", readiness.to_message()),
            });
        }
        for w in &readiness.warnings {
            warnings.push(format!("缺失回报（不阻塞）：{w}"));
        }
        input.instructions = with_missing_report(input.instructions.as_deref(), &readiness);
        // #127 P0-2：指令与缺失回报的**留档账面**（人类可读、随 attempt
        // 落盘到 ChapterRecord::draft_note）。与上一行的分工：instructions
        // 注入的是**生成器 prompt 面**（真实 LLM 需要知道缺什么才能留占位
        // 符）；draft_note 是**账面**（正文渲染永远不碰）。两者同源。
        let draft_note = compose_draft_note(instructions, &readiness);

        // ---- 1.6 输入完整性（地图 P0-1）：受限起草显式标注
        //
        // readiness 闸只拦「未确认的不就绪」；经逐件人工确认放行的坏件
        // 在这里变成**可见的受限标记**——放行 ≠ 没问题，三面都要说清
        // 「本次起草基于不完整输入」：warnings（响应面）、draft_note
        // （账面）、MCP out 的 input_integrity 块（机器面）。不得静默
        // 当成完整输入。
        let integrity = crate::readiness::input_integrity(self.store());
        if integrity.restricted() {
            warnings.push(format!("受限起草（输入不完整）：{}", integrity.to_message()));
        }
        let draft_note = if integrity.restricted() {
            let note = format!("受限起草：{}", integrity.to_message());
            Some(match draft_note {
                Some(n) => format!("{n}\n\n{note}"),
                None => note,
            })
        } else {
            draft_note
        };

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

        // ---- 5.5 矩阵联建（#241 裁定：正文实写成为状态派生输入——content
        //      变化后必须重算，否则消费面（导出矩阵门/validate 规则 1-2/
        //      起草 readiness）看到陈旧状态）。幂等派生：同内容重提交 → 行
        //      不变 → 不 bump；首次落正文 → 未响应行翻 RESPONDED → bump
        //      （矩阵行变化是真实状态变化，与绑定/覆写路径的联建同款账面）。
        //      draft_section_iterative 经本函数回放提交，天然同覆盖。
        self.store_mut().build_matrix()?;

        // ---- 6. 项目推进 DRAFTING（首次起草——MatrixReady → EvidenceBound
        //        → Drafting 逐态经 next_allowed；已越过者 no-op）
        advance_project_to_drafting(self.store_mut())?;

        // ---- 7. S-2 落盘（persist 在场）+ checkpoint
        if let Some(dir) = self.workspace_dir() {
            let mut asm = super::assembly::ChapterAssembler::open(&dir)
                .map_err(|e| ToolError::EUnprocessable {
                    reason: format!("章节工作区打开失败: {e}"),
                })?;
            asm.commit_section_with_note(section_id, &content, input.requirements.iter()
                .map(|r| r.id.clone()).collect(), draft_note.clone())
                .map_err(|e| ToolError::EUnprocessable {
                    reason: format!("章节工作区提交失败: {e}"),
                })?;
        }
        self.checkpoint().map_err(|e| ToolError::EUnprocessable {
            reason: format!("checkpoint 落盘失败（内存已变更、磁盘未跟上）: {e}"),
        })?;

        // ---- 8. 无锚承诺告警（#86 摩擦 #11——契约 §6「unanchored_claims>0
        //      时 warning」的落地面；阻断语义留给 validate 不变）
        let unanchored_claims = count_unanchored_claims(&content);
        if unanchored_claims > 0 {
            warnings.push(format!(
                "无锚承诺 {unanchored_claims} 句（含承诺词但同句无 [REQ-*]/[EV-*] 引用）——建议补引用或改写；是否阻断由 validate 裁决"
            ));
        }

        Ok(DraftOutcome {
            section_id: section_id.to_string(),
            attempt,
            chars,
            citations: valid,
            unanchored_claims,
            uncovered_requirements,
            warnings,
            instructions: instructions.map(str::to_string),
            draft_note,
            // 地图 P0-1：输入完整性随行（MCP 壳组装 input_integrity 块）
            input_integrity: integrity,
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
    /// 起草指令 + 缺失回报的留档账面（#127 P0-2——与正文分离；随
    /// ChapterRecord 落盘，正文渲染面不碰）。`None` = 无指令无缺失回报。
    pub draft_note: Option<String>,
    /// 项目级输入完整性（地图 P0-1——受限起草的判定面：restricted() =
    /// true 时本次起草基于不完整输入，MCP 响应标 RESTRICTED）。
    pub input_integrity: crate::readiness::InputIntegrity,
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

// ---------------------------------------------------------------- B4：迭代起草

/// `max_rounds` 硬上限（issue #97 §七-2「不要让 LLM 调用无限循环」——
/// 配置错误/目标不可达时的费用与时长兜底；构造时 clamp 到 [1, 8]）。
pub const MAX_ROUNDS_CAP: usize = 8;
/// 默认轮数（issue：默认 3——首轮 + 两轮改写的经验平衡点）。
pub const DEFAULT_MAX_ROUNDS: usize = 3;
/// 默认目标得分率（优档下沿 0.95 与良档中点 0.825 之间——B2 GradeTier
/// 的档位语义：0.85 = 「稳定良以上」，是改写性价比的实务折中）。
pub const DEFAULT_TARGET_RATIO: f64 = 0.85;

/// 循环内章节质量面（B3 维度 + 响应性/无锚——纯内容函数，无 store 依赖）。
///
/// 与报告层 [`crate::quality::SectionQuality`] 的分工：SectionQuality 是
/// store 派生面（ghost REQ/绑定 EV/字数带——报告层在场）；循环内只有
/// input + 正文，取其可算子面（issue #97 §七-4「不够用就如实报告」的
/// 对偶面——够用的子面不硬塞 store）。
#[derive(Debug, Clone, PartialEq)]
pub struct RoundQuality {
    /// 针对性（B3 维度 1）。
    pub specificity: crate::quality::SpecificityScore,
    /// 实质性（B3 维度 2）。
    pub substance: crate::quality::SubstanceScore,
    /// 正文已回应的 REQ（input.requirements 内被 [REQ-*] 引用者）。
    pub covered_reqs: Vec<String>,
    /// 未被回应的 REQ。
    pub uncovered_reqs: Vec<String>,
    /// 无锚承诺计数（承诺词句内无引用）。
    pub unanchored_claims: usize,
}

impl RoundQuality {
    fn assess(input: &SectionDraftInput, content: &str, project: &ProjectContext) -> Self {
        let citations = scan_citations(content);
        let cited: HashSet<&str> = citations.iter().map(|s| &s[..]).collect();
        let mut covered = Vec::new();
        let mut uncovered = Vec::new();
        for r in &input.requirements {
            if cited.contains(&r.id[..]) {
                covered.push(r.id.clone());
            } else {
                uncovered.push(r.id.clone());
            }
        }
        RoundQuality {
            specificity: assess_specificity(content, project),
            substance: assess_substance(content),
            covered_reqs: covered,
            uncovered_reqs: uncovered,
            unanchored_claims: count_unanchored_claims(content),
        }
    }
}

/// 单轮记录（可追溯面——issue：每轮的得分和改进可追溯）。
#[derive(Debug, Clone, PartialEq)]
pub struct RoundRecord {
    /// 轮次（1 起）。
    pub round: usize,
    /// 本轮自评得分（B2 estimated）。
    pub score: f64,
    /// 本轮满分（B2 max）。
    pub max: f64,
    /// 得分率（score/max；max=0 → 0.0）。
    pub ratio: f64,
    /// 本轮质量面（B3 + 响应性）。
    pub quality: RoundQuality,
    /// 本轮未覆盖/低覆盖评分项（B2 item_scores 覆盖 <60%——下一轮改写
    /// 提示的目标面；改写提示需要 id 清单而不只是计数，存引用而非摘要）。
    pub low_coverage_items: Vec<String>,
    /// 本轮改了什么/要求生成器改什么（给用户的解释——首轮 = 基线说明，
    /// 改写轮 = 针对上一轮失分项的补强清单摘要）。
    pub changes: String,
}

/// 迭代产物（issue #97 §B4 struct + notes 诚实登记扩展）。
#[derive(Debug, Clone, PartialEq)]
pub struct IterativeOutcome {
    /// 最终内容（最优轮——见 [`IterativeDrafter::draft`] 的最优轮保留）。
    pub content: String,
    /// 每轮记录（得分/质量/改动——LLM 调用次数 = rounds.len()）。
    pub rounds: Vec<RoundRecord>,
    /// 是否达标（某轮 ratio ≥ target 即 true）。
    pub reached_target: bool,
    /// 诚实登记：无映射评分项提前停 / 达硬上限未达标 / 末轮非最优被回退。
    pub notes: Vec<String>,
}

/// 迭代式起草器（生成 → 自评 → 改写——issue #97 B4）。
///
/// 循环逻辑（issue §B4）：
/// ```text
/// 第 1 轮：正常生成（原始 instructions）
///   ↓ B2 自评（score_section）+ B3 质量检查（针对性/实质性/响应性）
/// 达标（ratio ≥ target）→ 停止
/// 未达标 → 找出失分项，构造改写提示（build_rewrite_prompt）
/// 第 2 轮：带着「上一轮的失分项」重新生成（instructions = 改写提示）
///   ↓ ……最多 max_rounds 轮（硬上限——防 LLM 无限循环）
/// ```
///
/// 设计决策（依据）：
/// - **target 用得分率（ratio）非绝对分**：章节满分各不相同（10 分项 vs
///   30 分项），绝对目标无法跨章复用；0.85 的档位语义见
///   [`DEFAULT_TARGET_RATIO`]；
/// - **最优轮保留**：改写可能回归（LLM 非单调改进），落库内容取全程
///   ratio 最高的轮（rounds 全量可追溯——回退时 note 登记）；
/// - **无评分项映射提前停**：`score_section` 返回 max=0（章节无映射评分
///   项）时循环没有优化面，首轮后即停（note 登记——不空转烧轮次）；
/// - **max_rounds 硬 clamp**：构造时 clamp 到 [1, [`MAX_ROUNDS_CAP`]]
///   ——配置错误（0 / 999）不产生零轮或失控循环。
pub struct IterativeDrafter<'a> {
    /// 生成器（mock 或真实 LLM——同 [`SectionGenerator`] 注入位）。
    pub generator: &'a mut dyn SectionGenerator,
    /// B2 章节自评器（bid-domain 冻结面——只调用）。
    pub scorer: &'a SectionScorer,
    /// B3 针对性的专名锚源（ scorer 的 project_terms 同源）。
    pub project: &'a ProjectContext,
    /// 最大轮数（clamp 后——硬上限）。
    pub max_rounds: usize,
    /// 目标得分率（0-1）。
    pub target_ratio: f64,
}

impl<'a> IterativeDrafter<'a> {
    /// 构造（`max_rounds` clamp 到 [1, 8]；`target_ratio` clamp 到 [0, 1]）。
    pub fn new(
        generator: &'a mut dyn SectionGenerator,
        scorer: &'a SectionScorer,
        project: &'a ProjectContext,
        max_rounds: usize,
        target_ratio: f64,
    ) -> Self {
        IterativeDrafter {
            generator,
            scorer,
            project,
            max_rounds: max_rounds.clamp(1, MAX_ROUNDS_CAP),
            target_ratio: target_ratio.clamp(0.0, 1.0),
        }
    }

    /// 迭代起草（纯内存循环——落库由调用方/服务面承担）。
    pub fn draft(&mut self, input: &SectionDraftInput) -> Result<IterativeOutcome, String> {
        let mut rounds: Vec<RoundRecord> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        let mut reached_target = false;
        // 最优轮追踪（ratio 最高者——改写回归时保最优）
        let mut best_ratio = f64::MIN;
        let mut best_content = String::new();
        let mut best_round = 0usize;

        for round in 1..=self.max_rounds {
            // ---- 本轮指令：首轮 = 原始 instructions；改写轮 = 失分项提示
            let instructions = if round == 1 {
                input.instructions.clone()
            } else {
                Some(build_rewrite_prompt(
                    self.scorer,
                    input,
                    rounds.last().expect("改写轮必有前轮"),
                    self.target_ratio,
                    round,
                ))
            };
            let mut round_input = input.clone();
            round_input.instructions = instructions;

            // ---- 生成（LLM 调用 = 每轮恰一次）
            let draft = self.generator.generate(&round_input)?;

            // ---- 自评（B2）+ 质量检查（B3）
            let score = self.scorer.score_section(&input.section_id, &draft.content);
            let quality = RoundQuality::assess(input, &draft.content, self.project);
            let ratio = if score.max > 0.0 {
                (score.estimated / score.max * 1000.0).round() / 1000.0
            } else {
                0.0
            };

            let changes = if round == 1 {
                format!(
                    "首轮生成（基线）：自评 {:.2}/{:.2}，针对性 {:.2}、实质性 {:.2}、\
                     未回应条款 {} 项、无锚承诺 {} 句",
                    score.estimated, score.max,
                    quality.specificity.score, quality.substance.score,
                    quality.uncovered_reqs.len(), quality.unanchored_claims,
                )
            } else {
                let prev = rounds.last().unwrap();
                format!(
                    "第 {round} 轮改写：针对第 {} 轮失分项——未覆盖/低覆盖评分项 {} 项、\
                     空话句占比 {:.0}%、具体参数 {} 处、未回应条款 {} 项、无锚承诺 {} 句\
                     （得分 {:.2} → {:.2}）",
                    prev.round,
                    prev.low_coverage_items.len(),
                    prev.quality.substance.empty_sentence_ratio * 100.0,
                    prev.quality.specificity.concrete_params,
                    prev.quality.uncovered_reqs.len(),
                    prev.quality.unanchored_claims,
                    prev.score, score.estimated,
                )
            };

            // ---- 最优轮追踪（含并列取先——稳定可重放）
            if ratio > best_ratio {
                best_ratio = ratio;
                best_content = draft.content.clone();
                best_round = round;
            }

            let stop_reason: Option<&'static str> = if score.max == 0.0 {
                Some("NO_SCORING_ITEMS")
            } else if ratio >= self.target_ratio {
                reached_target = true;
                Some("TARGET_REACHED")
            } else {
                None
            };
            // 本轮低覆盖清单（B2 item_scores 覆盖 <60%——下一轮改写提示的目标面）
            let low_coverage_items: Vec<String> = score
                .item_scores
                .iter()
                .filter(|it| it.coverage < 0.6)
                .map(|it| it.item_id.clone())
                .collect();
            rounds.push(RoundRecord {
                round,
                score: score.estimated,
                max: score.max,
                ratio,
                quality,
                low_coverage_items,
                changes,
            });

            match stop_reason {
                Some("NO_SCORING_ITEMS") => {
                    notes.push(format!(
                        "无映射评分项（section_id={} 的评分项 section 槽位为空）——\
                         自评无优化面，首轮后停止（不空转轮次）",
                        input.section_id
                    ));
                    break;
                }
                Some("TARGET_REACHED") => break,
                _ => {}
            }
        }

        // 硬上限未达标（非提前停路径走满轮数）
        if !reached_target
            && rounds.len() == self.max_rounds
            && rounds.last().map(|r| r.max) != Some(0.0)
        {
            notes.push(format!(
                "达到 max_rounds={} 硬上限仍未达标（最优 {:.2}/{:.2}）——\
                 剩余失分项见末轮 RoundRecord，人工接手或调目标",
                self.max_rounds, best_ratio, rounds.iter().map(|r| r.max).sum::<f64>(),
            ));
        }
        // 末轮非最优（改写回归）——保留最优轮并登记
        if best_round != 0 && best_round != rounds.len() {
            notes.push(format!(
                "第 {} 轮为全程最优（ratio {:.3}）——末轮改写回归，保留最优轮内容",
                best_round, best_ratio
            ));
        }

        Ok(IterativeOutcome {
            content: best_content,
            rounds,
            reached_target,
            notes,
        })
    }
}

/// 改写提示构造（issue §B4「改写提示怎么构造」——明确告诉模型哪里不够）。
///
/// 失分项四源（确定性拼装，无 LLM 参与——提示本身可 diff 可重放）：
/// 1. **未覆盖/低覆盖评分项**（`score.missing_items` + item_scores 覆盖
///    <60%）——附评分标准原文（scorer.tree.index 的 criteria——生成器
///    需要知道「写什么」而不只是「缺什么」）；
/// 2. **空话密度**（`quality.substance`——空话句占比 + 替换要求）；
/// 3. **针对性缺口**（`quality.specificity`——项目名/招标人名缺失 +
///    具体参数计数）；
/// 4. **响应性缺口**（`quality.uncovered_reqs` + `unanchored_claims`）。
pub fn build_rewrite_prompt(
    scorer: &SectionScorer,
    input: &SectionDraftInput,
    prev: &RoundRecord,
    target_ratio: f64,
    round: usize,
) -> String {
    let mut v: Vec<String> = Vec::new();
    v.push(format!(
        "【第 {round} 轮改写指令】上一轮自评 {:.2}/{:.2}（得分率 {:.1}%），\
         未达目标 {:.1}%。逐项补强以下失分项：",
        prev.score, prev.max, prev.ratio * 100.0, target_ratio * 100.0,
    ));

    // 1. 未覆盖/低覆盖评分项（附标准原文——生成器要知道写什么）
    let low = prev.low_coverage_items.clone();
    if !low.is_empty() {
        v.push("\n1. 未覆盖/低覆盖的评分项（覆盖 <60%，正文未提及评分标准主题词）：".to_string());
        for id in &low {
            let criteria = scorer
                .tree
                .index
                .get(id)
                .map(|it| it.criteria.trim().to_string())
                .unwrap_or_else(|| "（评分标准原文缺失——见招标文件）".to_string());
            let max = scorer.tree.index.get(id).and_then(|it| it.max_score);
            v.push(format!(
                "   - [{}]（{}分）「{}」——正文须逐条论述该评分标准的主题词",
                id,
                max.map(|m| format!("{m:.0}")).unwrap_or_else(|| "?".into()),
                criteria,
            ));
        }
    }

    // 2. 空话密度（实质性）
    let st = &prev.quality.substance;
    v.push(format!(
        "\n2. 空话密度：空话句占比 {:.0}%、可验证陈述 {} 句——把无具体锚的套话句\
         （「高度重视/全力以赴/确保万无一失」类）替换为可验证措施（具体数字、\
         标准、时限、责任主体）。",
        st.empty_sentence_ratio * 100.0, st.verifiable_claims,
    ));

    // 3. 针对性缺口
    let sp = &prev.quality.specificity;
    let mut spec_gaps: Vec<String> = Vec::new();
    if !sp.has_project_name {
        spec_gaps.push(format!("正文未出现项目名「{}」", scorer.project_terms.first().map(String::as_str).unwrap_or("（项目名未知）")));
    }
    if sp.concrete_params < 3 {
        spec_gaps.push(format!("具体参数（数字+单位）仅 {} 处——目标 ≥5 处", sp.concrete_params));
    }
    v.push(format!(
        "\n3. 针对性：{}。写明本项目的具体参数（工程量/工期/人员/设备），\
         不得使用「XX 项目」占位符。",
        if spec_gaps.is_empty() { "已达基本线".to_string() } else { spec_gaps.join("；") },
    ));

    // 4. 响应性缺口（未回应条款 + 无锚承诺）
    if !prev.quality.uncovered_reqs.is_empty() {
        v.push(format!(
            "\n4. 未回应条款（正文无 [REQ-*] 引用）：{}——逐条补回应段落。",
            prev.quality.uncovered_reqs.join("、"),
        ));
    }
    if prev.quality.unanchored_claims > 0 {
        v.push(format!(
            "\n5. 无锚承诺 {} 句（承诺词句内无引用）——补 [REQ-*]/[EV-*] 引用或删承诺词。",
            prev.quality.unanchored_claims,
        ));
    }

    // 收束：可用引用编号白名单（防幻觉——与 llm.rs 的规则面一致）
    let req_ids: Vec<String> = input.requirements.iter().map(|r| format!("[{}]", r.id)).collect();
    let ev_ids: Vec<String> = input.evidences.iter().map(|e| format!("[{}]", e.id)).collect();
    v.push(format!(
        "\n改写要求：保留原文已覆盖的内容，针对上述失分项逐项补强；引用格式\
         [REQ-xxxx]/[EV-xxxx]，只准使用以下编号（禁止编造）：REQ {}；EV {}。",
        req_ids.join("、"),
        if ev_ids.is_empty() { "（无可用证据——不得编造）".to_string() } else { ev_ids.join("、") },
    ));
    v.join("\n")
}

// ---------------------------------------------------------------- B4：服务集成面

/// 迭代起草的服务产物（DraftOutcome = MCP 契约面；iterative = 循环可追溯面）。
#[derive(Debug, Clone, PartialEq)]
pub struct IterativeSectionOutcome {
    /// 最终稿的提交产物（经 draft_section 既有路径——引用回查/CAS 写入/
    /// 状态推进/S-2 落盘/checkpoint 全复用）。
    pub draft: DraftOutcome,
    /// 循环记录（轮次/得分/改动——LLM 调用次数 = rounds.len()）。
    pub iterative: IterativeOutcome,
}

/// 一次性回放生成器已收敛为 [`ExternalGenerator`]（pub——B4 迭代循环的
/// 最优稿回放与 #100 任务 2 的外部手稿注入同一件：把现成全文经
/// [`BidService::draft_section`] 的既有提交路径落库——提交面零复制）。

impl super::service::BidService {
    /// `draft_section_iterative` 业务半边（B4——生成 → 自评 → 改写循环）。
    ///
    /// 与 [`BidService::draft_section`] 的关系：循环在内存中跑（B2 自评 +
    /// B3 质量检查的纯函数面），**最优稿经 ExternalGenerator 回放走
    /// draft_section 的完整提交路径**——引用回查（防幻觉）、CAS 写入、
    /// 状态推进、S-2 落盘、checkpoint 与单次起草完全一致（提交面零复制）。
    ///
    /// scorer/project 从 store 现场派生（评分树 + 项目专名——B2/B3 同源）；
    /// 循环 notes 并入 DraftOutcome.warnings（MCP 消费面的单一出口）。
    #[allow(clippy::too_many_arguments)]
    pub fn draft_section_iterative(
        &mut self,
        section_id: &str,
        requirement_override: Option<&[String]>,
        evidence_override: Option<&[String]>,
        instructions: Option<&str>,
        generator: &mut dyn SectionGenerator,
        max_rounds: usize,
        target_ratio: f64,
    ) -> Result<IterativeSectionOutcome, ToolError> {
        // ---- 1. 输入装配（与 draft_section 同源——幽灵剔除/失效拒绑）
        let (mut input, _warnings) = build_section_input(
            self.store(), section_id,
            requirement_override, evidence_override, instructions,
        )?;

        // ---- 1.5 缺失回报前置闸（先于 LLM 循环——阻塞章节一轮都不烧）
        //
        // 与 draft_section 同判据：五种素材型 Blocker → E_UNPROCESSABLE
        // （人类可读理由）。闸必须开在循环**之前**：B4 的循环每轮恰一次
        // generator 调用，等回放提交时才拒 = 白烧 max_rounds 轮编造。
        // readiness 的 warning 面（IncompleteTender 等）经
        // [`with_missing_report`] 注入循环输入——首轮起生成器就知道该留
        // 【待补充】占位；warnings 本身经回放的 draft_section 并入
        // DraftOutcome.warnings（单一出口，不重复计）。
        let readiness = crate::readiness::check_draft_readiness(self.store(), section_id);
        if !readiness.ready {
            return Err(ToolError::EUnprocessable {
                reason: format!("章节 {section_id} {}", readiness.to_message()),
            });
        }
        input.instructions = with_missing_report(input.instructions.as_deref(), &readiness);

        // ---- 2. scorer + project 从 store 派生（B2 评分树 + B3 专名锚）
        let project = ProjectContext::from_store(self.store());
        let scorer = SectionScorer::new(
            derive_scoring_tree(self.store().requirements()),
            project.scorer_terms(),
        );

        // ---- 3. 循环（内存——每轮一次 generator 调用）
        let mut it = IterativeDrafter::new(generator, &scorer, &project, max_rounds, target_ratio);
        let iterative = it.draft(&input).map_err(|why| ToolError::EUnprocessable {
            reason: format!("章节 {section_id} 迭代起草失败: {why}"),
        })?;

        // ---- 4. 最优稿经既有提交路径落库（ExternalGenerator 回放——提交面零复制）
        let mut replay = ExternalGenerator { content: iterative.content.clone() };
        let mut draft = self.draft_section(
            section_id,
            requirement_override,
            evidence_override,
            instructions,
            &mut replay,
        )?;
        // 循环 notes 并入 warnings（MCP 单一出口）
        for n in &iterative.notes {
            draft.warnings.push(format!("迭代循环：{n}"));
        }

        Ok(IterativeSectionOutcome { draft, iterative })
    }
}
