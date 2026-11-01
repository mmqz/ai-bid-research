//! readiness.rs —— 生成前置检查（缺失回报核心，issue #103 子任务 C）。
//!
//! ## 为什么这是最根本的问题
//!
//! Lead 原话：「缺失回报，如果缺失这个内容却还硬做只会做错」。
//! 现状是 [draft_response_section] 不管有没有素材都生成：
//!
//! ```text
//! 没有业绩证据     → LLM 编造 → 废标
//! 缺资质证书       → 虚假响应 → 资格性审查否决
//! 技术参数不满足   → 硬写「完全响应」→ 中标后违约
//! ```
//!
//! 本模块把「能不能写」变成一个**可计算的判据**：起草前先问
//! [`check_draft_readiness`]，拿不到素材就别写，把缺什么如实回报——
//! 这叫缺失回报（缺什么报什么），不是「尽力而为地写一段」。
//!
//! ## 分层纪律
//!
//! - 本模块是**纯只读派生**（D2 同语义：诊断是导数不是源数据）——不写
//!   store、不推状态、不 bump content_version，随便调；
//! - **不接线**：接线到 `draft_section` 是云端代理 C 的活（它在改
//!   draft.rs）。这里只交付能力，[`draft.rs`] 一个字节都不动；
//! - 判定只用 store 已有的真源：章节树（requirement_ids / title）、条款
//!   集（category）、证据台账（kind / status）、绑定、矩阵、文件种类。
//!
//! ## 六种阻塞（Blocker）的边界
//!
//! | Blocker | 触发 | 级 |
//! |---|---|---|
//! | NoRequirements | 叶章节没绑任何 REQ | 阻塞 |
//! | MissingQualification | 标题含资格/资质 且 台账无可用资质证据 | 阻塞 |
//! | MissingPerformance | 标题含业绩/类似项目 且 可用业绩 < [`MIN_PERFORMANCE_COUNT`] | 阻塞 |
//! | MissingTechEvidence | 本章技术参数类 REQ 无可用证据佐证 | 阻塞 |
//! | UncoveredVetoItem | 本章负责的否决项在矩阵未响应 / blocking | 阻塞 |
//! | IncompleteTender | 招标包结构问题（无招标文件/多份正本/多份招标文件） | **警告**（信息面） |
//! | **MissingAddendum** | 招标文件已导入，但补遗/澄清**缺失或解析失败** | **阻塞**（L3） |
//!
//! ## L3：补遗缺失必须阻塞（本模块最容易被低估的一条）
//!
//! 事故形态（Lead 点名）：
//!
//! ```text
//! C-补遗文件01.pdf 因 CMap 缺失/加密/滤镜不支持 → parse_status = Failed
//!   → E_PARTIAL 只说「1 件导入失败」
//!   → 链路继续跑 → 校验 blocker = 0
//!   → 用户看到「一切正常」→ 按旧条款做标 → 废标
//! ```
//!
//! 补遗对条款是**实质性修改**（控制价、截止时间、参数、评分办法都在里面），
//! 丢一份补遗等于拿过期条款投标。所以这不是「信息面警告」，是**阻塞**。
//!
//! 「可用证据」= 状态 Active / Expiring（与 evidence.rs `can_bind`、
//! draft.rs `check_evidence_bindable` 同口径）——过期件不能算有。
//!
//! 措辞纪律：**面向业务说人话**。说「缺 3 个类似项目业绩，现有 1 条」，
//! 不说 `MissingPerformance { required: 3 }`。

use std::collections::BTreeSet;

use bid_domain::evidence::{Evidence, EvidenceKind};
use bid_domain::matrix::{MatrixRow, ResponseStatus};
use bid_domain::{DomainStore, Requirement};
use serde::{Deserialize, Serialize};

use crate::placeholder::{make_placeholder, PlaceholderKind};

// ---------------------------------------------------------------- 阈值与词表

/// 业绩证据最低条数（多数招标文件的「近三年类似项目业绩」门槛为 3）。
pub const MIN_PERFORMANCE_COUNT: usize = 3;

/// 资格/资质类章节的标题词。
const QUALIFICATION_TITLE_WORDS: &[&str] = &["资格", "资质"];

/// 业绩类章节的标题词。
const PERFORMANCE_TITLE_WORDS: &[&str] = &["业绩", "类似项目"];

/// 条款类别字面量（extract 产物口径：资格条件|否决项|评分项|技术参数|商务要求|格式要求|其他）。
pub const CAT_TECH: &str = "技术参数";
pub const CAT_QUALIFICATION: &str = "资格条件";
pub const CAT_VETO: &str = "否决项";

/// 招标文件导入件的种类字面量。
const FILE_KIND_TENDER: &str = "招标文件";

// ---------------------------------------------------------------- 结果面

/// 章节生成就绪度（[`check_draft_readiness`] 的产物）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftReadiness {
    /// 是否可起草（= `blockers.is_empty()`）。
    pub ready: bool,
    /// 阻塞项（非空 = 不得生成正文，先把缺什么报上去）。
    pub blockers: Vec<Blocker>,
    /// 告警（不阻塞，但起草时应当知晓——如缺包干价口径）。
    pub warnings: Vec<String>,
}

impl DraftReadiness {
    fn from_parts(blockers: Vec<Blocker>, warnings: Vec<String>) -> Self {
        DraftReadiness { ready: blockers.is_empty(), blockers, warnings }
    }

    /// 一次性人类可读回报（日志/工具返回值）：先说能不能写，再说为什么。
    pub fn to_message(&self) -> String {
        if self.ready {
            return if self.warnings.is_empty() {
                "可以起草：素材齐备，无阻塞项".to_string()
            } else {
                format!("可以起草（{} 条告警）：{}", self.warnings.len(), self.warnings.join("；"))
            };
        }
        format!(
            "暂不可起草：{} 条阻塞项——{}",
            self.blockers.len(),
            self.blockers.iter().map(Blocker::to_message).collect::<Vec<_>>().join("；")
        )
    }
}

/// 生成阻塞原因（六种）。
///
/// 字段只放**回报所需**的事实（谁、缺几个、缺哪些），不放内部 id 集合的
/// 裸转储——[`Blocker::to_message`] 负责翻成人话。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Blocker {
    /// 章节没有绑定任何要求——空响应面，写出来必是空话。
    NoRequirements { section_id: String },
    /// 资格/资质章节，但证据台账里没有可用的资质证据。
    MissingQualification {
        /// 被要求的资质名（章节标题里是啥就报啥；标题没具体名字则给通用名）。
        required: Vec<String>,
        /// 台账里同类但不可用的证据（过期/被替代）——如实说，不假装没有。
        have: Vec<String>,
    },
    /// 业绩章节，可用业绩证据不足。
    MissingPerformance { required_count: usize, have_count: usize },
    /// 本章负责的技术参数类条款没有证据佐证（硬写「完全响应」= 违约）。
    MissingTechEvidence { req_ids: Vec<String> },
    /// 本章负责的否决项在合规矩阵里仍未被响应（否决项漏响应 = 直接废标）。
    UncoveredVetoItem { req_ids: Vec<String> },
    /// 招标包不完整（无招标文件本体 / 多份无法确定正本）。**警告面**。
    IncompleteTender { missing: Vec<String> },
    /// 招标文件已导入，但补遗/澄清件**缺失或解析失败**（L3——**阻塞**）。
    ///
    /// 与 [`Blocker::IncompleteTender`] 的分界：那条是「招标包结构可疑，
    /// 请人工确认」；这条是「已确认导入过招标文件，但补遗这一环断了」——
    /// 补遗含对条款的实质性修改，缺了就不能按现有条款做标。
    MissingAddendum {
        /// 涉及的文件名（`ParseFailed` 是被解析失败的那几件；`NotProvided`
        /// 为空——没有文件可列）。
        files: Vec<String>,
        /// 缺失形态。
        reason: AddendumIssue,
    },
}

/// 补遗缺失的两种形态（L3 规格原文）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddendumIssue {
    /// 导入的文件里根本没有补遗/澄清件。
    ///
    /// 注意：真实项目**可能真的没有补遗**（没发过澄清）。所以这一条是
    /// 「必须人工确认」，不是「一定错误」——但确认之前不能当它不存在。
    NotProvided,
    /// 有补遗件但解析失败（CMap 缺失/加密/滤镜不支持/扫描件 OCR 断链）。
    ///
    /// 这一条没有歧义：补遗确实存在，只是我们读不了它——
    /// 读不了就等于没有，绝不能按「没补遗」继续。
    ParseFailed,
}

impl AddendumIssue {
    pub fn as_str(&self) -> &'static str {
        match self {
            AddendumIssue::NotProvided => "NOT_PROVIDED",
            AddendumIssue::ParseFailed => "PARSE_FAILED",
        }
    }
}

impl Blocker {
    /// 人类可读的说明（**不是** enum 名字——业务方看的就是这句）。
    pub fn to_message(&self) -> String {
        match self {
            Blocker::NoRequirements { section_id } => format!(
                "章节 {section_id} 没有关联任何招标要求，写出来只能是空话——先把它负责的条款挂上"
            ),
            Blocker::MissingQualification { required, have } => {
                let req = if required.is_empty() {
                    "资格/资质类证明材料".to_string()
                } else {
                    required.join("、")
                };
                let tail = if have.is_empty() {
                    String::new()
                } else {
                    format!("（台账里有但已失效：{}）", have.join("、"))
                };
                format!("缺{req}：证据库里没有可用的资格/资质证明材料{tail}——无证硬写等于虚假响应")
            }
            Blocker::MissingPerformance { required_count, have_count } => format!(
                "业绩证据不足：需要 {required_count} 个类似项目业绩，现有 {have_count} 个——编造业绩会直接废标"
            ),
            Blocker::MissingTechEvidence { req_ids } => format!(
                "技术参数缺少证据佐证：{} 条技术参数条款没有对应的证明材料（{}）——硬写「完全响应」会导致中标后违约",
                req_ids.len(),
                req_ids.join("、")
            ),
            Blocker::UncoveredVetoItem { req_ids } => format!(
                "否决项尚未响应：{} 条否决条款在合规矩阵里还没响应（{}）——否决项漏响应直接废标",
                req_ids.len(),
                req_ids.join("、")
            ),
            Blocker::IncompleteTender { missing } => format!(
                "招标包信息不完整：{}",
                missing.join("；")
            ),
            Blocker::MissingAddendum { files, reason } => match reason {
                AddendumIssue::NotProvided => "未发现补遗/澄清文件——若本项目曾发过澄清答疑，现行条款可能已被修改。请补充导入补遗件，或明确确认本项目无补遗后再继续".to_string(),
                AddendumIssue::ParseFailed => format!(
                    "补遗/澄清文件解析失败：{}——补遗里有对条款的实质性修改（控制价/截止时间/参数/评分办法），读不了就等于没收到，按现有条款做标会直接废标",
                    files.join("、")
                ),
            },
        }
    }

    /// 稳定机器码（日志/统计面；**不用于**给人看）。
    pub fn code(&self) -> &'static str {
        match self {
            Blocker::NoRequirements { .. } => "NO_REQUIREMENTS",
            Blocker::MissingQualification { .. } => "MISSING_QUALIFICATION",
            Blocker::MissingPerformance { .. } => "MISSING_PERFORMANCE",
            Blocker::MissingTechEvidence { .. } => "MISSING_TECH_EVIDENCE",
            Blocker::UncoveredVetoItem { .. } => "UNCOVERED_VETO_ITEM",
            Blocker::IncompleteTender { .. } => "INCOMPLETE_TENDER",
            Blocker::MissingAddendum { .. } => "MISSING_ADDENDUM",
        }
    }

    /// 该阻塞对应的占位建议（接线上正文缺这一块时该写什么占位）。
    ///
    /// 接线方可直接 `content.push_str(&b.suggest_placeholder())`，把缺的
    /// 东西显式留在正文里，而不是让模型自由发挥。
    pub fn suggest_placeholder(&self) -> String {
        match self {
            Blocker::NoRequirements { .. } => {
                make_placeholder(PlaceholderKind::Confirm, "本章响应范围待确认")
            }
            Blocker::MissingQualification { required, .. } => {
                let what = if required.is_empty() {
                    "请提供资格/资质证明材料".to_string()
                } else {
                    format!("请提供{}", required.join("、"))
                };
                make_placeholder(PlaceholderKind::Missing, &what)
            }
            Blocker::MissingPerformance { required_count, have_count } => {
                make_placeholder(
                    PlaceholderKind::Missing,
                    &format!("请提供 {required_count} 个类似项目业绩（现有 {have_count} 个）"),
                )
            }
            Blocker::MissingTechEvidence { req_ids } => make_placeholder(
                PlaceholderKind::Missing,
                &format!("请提供 {} 条技术参数的证明材料", req_ids.len()),
            ),
            Blocker::UncoveredVetoItem { req_ids } => make_placeholder(
                PlaceholderKind::Confirm,
                &format!("{} 条否决项条款待确认响应方式", req_ids.len()),
            ),
            Blocker::IncompleteTender { .. } => make_placeholder(
                PlaceholderKind::Confirm,
                "招标文件补遗/澄清情况待确认",
            ),
            Blocker::MissingAddendum { files, reason } => match reason {
                AddendumIssue::NotProvided => make_placeholder(
                    PlaceholderKind::Missing,
                    "请提供补遗/澄清文件",
                ),
                AddendumIssue::ParseFailed => make_placeholder(
                    PlaceholderKind::Missing,
                    &format!(
                        "补遗文件 {} 解析失败，请提供可解析版本",
                        files.join("、")
                    ),
                ),
            },
        }
    }
}

// ---------------------------------------------------------------- 证据可用性

/// 可用证据 = 状态可绑（Active / Expiring）。
///
/// 与 evidence.rs `EvidenceStatus::can_bind`、draft.rs 的禁绑同口径——
/// 过期（R-03）/ 被替代的证据不能算「有」。
pub fn is_usable_evidence(ev: &Evidence) -> bool {
    ev.status.can_bind()
}

/// 台账里某类**可用**证据。
fn usable_evidences(store: &DomainStore, kind: EvidenceKind) -> Vec<&Evidence> {
    store
        .evidences()
        .iter()
        .filter(|e| e.kind == kind && is_usable_evidence(e))
        .collect()
}

/// 给主检查与正文生成共用：某类可用证据的展示名（章节标题 → 素材清单）。
pub fn usable_evidence_titles(store: &DomainStore, kind: EvidenceKind) -> Vec<String> {
    usable_evidences(store, kind).iter().map(|e| e.title.clone()).collect()
}

// ---------------------------------------------------------------- 主检查

/// 检查章节的生成条件。
///
/// 只读：不写 store、不推状态、不 bump 版本。章节不存在 → 返回一条
/// [`Blocker::NoRequirements`]（`section_id` 原样回报），不会 panic——
/// 前置检查本身不能成为崩溃源。
///
/// 判定顺序按「越靠前越致命」排列：无要求 → 资格 → 业绩 → 技术证据 →
/// 否决项；招标包不完整是警告，不进 blockers（但会出现在 warnings 与
/// [`DraftReadiness::to_message`] 里）。
pub fn check_draft_readiness(store: &DomainStore, section_id: &str) -> DraftReadiness {
    let mut warnings: Vec<String> = Vec::new();
    let mut blockers = Vec::new();

    // ---- L3：补遗缺失 / 解析失败 —— **阻塞**（全局面，先于章节判定）
    //
    // 这是全局阻塞（不属于某一章）：补遗是对条款的实质性修改，缺了它之后
    // 每一章都是在过期条款上做标。放在章节判定之前——章节不存在时也要报，
    // 否则「查一个不存在的章节」会显得风平浪静。
    if let Some(blocker) = missing_addendum(store) {
        blockers.push(blocker);
    }

    // ---- 招标包完整性（警告面——与章节无关，先说，免得读者以为「没有阻塞」就万事大吉）
    for missing in tender_gaps(store) {
        warnings.push(
            Blocker::IncompleteTender { missing: vec![missing.clone()] }.to_message(),
        );
    }

    let Some(node) = store.sections().iter().find(|n| n.id == section_id) else {
        blockers.push(Blocker::NoRequirements { section_id: section_id.to_string() });
        return DraftReadiness::from_parts(blockers, warnings);
    };

    // ---- 1. 空响应面
    if node.requirement_ids.is_empty() {
        blockers.push(Blocker::NoRequirements { section_id: section_id.to_string() });
        // 空响应面时其余检查无从谈起（没有 REQ 就没有技术参数/否决项可查），
        // 但章节标题词面仍能提示素材缺——那部分照查，避免漏报。
    }

    // ---- 2. 资格 / 资质
    if let Some(required) = qualification_requirement(&node.title) {
        let quals = usable_evidences(store, EvidenceKind::Qualification);
        if quals.is_empty() {
            blockers.push(Blocker::MissingQualification {
                required,
                have: stale_qualification_titles(store),
            });
        }
    }

    // ---- 3. 业绩
    if title_has_any(&node.title, PERFORMANCE_TITLE_WORDS) {
        let have_count = usable_evidences(store, EvidenceKind::Performance).len();
        if have_count < MIN_PERFORMANCE_COUNT {
            blockers.push(Blocker::MissingPerformance {
                required_count: MIN_PERFORMANCE_COUNT,
                have_count,
            });
        }
    }

    // ---- 4. 技术参数证据 / 5. 否决项响应
    let reqs: Vec<&Requirement> = node
        .requirement_ids
        .iter()
        .filter_map(|rid| store.requirements().iter().find(|r| &r.id == rid))
        .collect();

    let tech_ids: Vec<String> = reqs
        .iter()
        .filter(|r| r.category == CAT_TECH)
        .filter(|r| !req_has_usable_evidence(store, &r.id))
        .map(|r| r.id.clone())
        .collect();
    if !tech_ids.is_empty() {
        blockers.push(Blocker::MissingTechEvidence { req_ids: tech_ids });
    }

    let veto_ids: Vec<String> = reqs
        .iter()
        .filter(|r| r.category == CAT_VETO)
        .filter(|r| !veto_covered(store, &r.id))
        .map(|r| r.id.clone())
        .collect();
    if !veto_ids.is_empty() {
        blockers.push(Blocker::UncoveredVetoItem { req_ids: veto_ids });
    }

    // ---- 其他提示面（不阻塞）
    if reqs.is_empty() && !node.requirement_ids.is_empty() {
        warnings.push(format!(
            "章节 {section_id} 关联的 {} 条要求都不在条款集中（抽取重跑后 id 漂移？）——起草前先确认",
            node.requirement_ids.len()
        ));
    }
    let ghost = node.requirement_ids.len().saturating_sub(reqs.len());
    if ghost > 0 && !reqs.is_empty() {
        warnings.push(format!("章节 {section_id} 有 {ghost} 条关联要求不在条款集中（幽灵条款，按未覆盖处理）"));
    }

    DraftReadiness::from_parts(blockers, warnings)
}

/// 批量版：一次查多个章节（起草队列/报告面用），返回 `(section_id, 就绪度)`。
///
/// 只查叶章节（结构节点不承载正文，检查它没有意义）。
pub fn check_all_readiness(store: &DomainStore) -> Vec<(String, DraftReadiness)> {
    let has_child: BTreeSet<&str> = store
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.as_deref())
        .collect();
    store
        .sections()
        .iter()
        .filter(|n| !has_child.contains(n.id.as_str()))
        .map(|n| (n.id.clone(), check_draft_readiness(store, &n.id)))
        .collect()
}

// ---------------------------------------------------------------- 判定辅助

fn title_has_any(title: &str, words: &[&str]) -> bool {
    words.iter().any(|w| title.contains(w))
}

/// 资格/资质章节的「被要求项」：标题里有具体名字就用它（如「安全生产许可证」），
/// 否则给通用名。None = 本章不是资格/资质类。
fn qualification_requirement(title: &str) -> Option<Vec<String>> {
    if !title_has_any(title, QUALIFICATION_TITLE_WORDS) {
        return None;
    }
    let cleaned = title
        .trim()
        .trim_end_matches(|c: char| c.is_whitespace() || "章节、及与".contains(c))
        .to_string();
    if cleaned.is_empty() {
        Some(vec!["资格/资质证明材料".to_string()])
    } else {
        Some(vec![cleaned])
    }
}

/// 台账里**不可用**的资质证据标题（过期/被替代——如实回报，不假装没有）。
fn stale_qualification_titles(store: &DomainStore) -> Vec<String> {
    store
        .evidences()
        .iter()
        .filter(|e| e.kind == EvidenceKind::Qualification && !is_usable_evidence(e))
        .map(|e| format!("{}（{}）", e.title, e.status.as_str()))
        .collect()
}

/// 条款是否已绑定可用证据。
fn req_has_usable_evidence(store: &DomainStore, req_id: &str) -> bool {
    store.evidence_bindings().iter().any(|b| {
        b.requirement_id == req_id
            && store.evidences().iter().any(|e| e.id == b.evidence_id && is_usable_evidence(e))
    })
}

/// 否决项是否已在矩阵响应。
///
/// 矩阵反映**人工覆写优先**后的真实响应态（override 是唯一人工输入），
/// 所以这里直接看矩阵，不看绑定——「绑了证据但矩阵仍标未响应」说明证据被
/// 判为不可用或被覆写成不适用，两种都该阻塞。
///
/// 判定：
/// - 命中行且 `RESPONDED` → 已响应；
/// - 命中行 `NOT_APPLICABLE` 且**有偏离说明** → 视为已处理（可审计的豁免，
///   与 matrix.rs「不适用必须给理由」同口径）；
/// - 其余（NOT_RESPONDED / PARTIAL / blocking / 无矩阵行）→ 未响应。
fn veto_covered(store: &DomainStore, req_id: &str) -> bool {
    let Some(row) = store.matrix().iter().find(|r: &&MatrixRow| r.requirement_id == req_id) else {
        return false; // 矩阵未构建或漏行——没对过账就是没响应
    };
    if row.blocking {
        return false;
    }
    match row.status {
        ResponseStatus::Responded => true,
        ResponseStatus::NotApplicable => {
            row.deviation.as_deref().map(str::trim).is_some_and(|d| !d.is_empty())
        }
        ResponseStatus::NotResponded | ResponseStatus::Partial => false,
    }
}

/// 招标包缺口（每项一条人类可读说明；空 = 包完整）。
///
/// 判定（只认导入台账里的事实，不猜文件内容）：
/// 1. **没有招标文件本体**——只有附件/图纸，无从判断响应要求；
/// 2. **多份招标文件**——无法确定正本（补遗往往也是「招标文件」类，需人工说明口径）；
/// 3. **没有补遗/澄清件**——真实项目常有澄清答疑，缺失需人工确认（最常见的
///    信息面缺口：澄清里常有对条款的实质性修改）。
pub fn tender_gaps(store: &DomainStore) -> Vec<String> {
    let mut out = Vec::new();
    let tenders: Vec<&str> = store
        .files()
        .iter()
        .filter(|f| f.kind == FILE_KIND_TENDER)
        .map(|f| f.name.as_str())
        .collect();
    if tenders.is_empty() {
        out.push(format!(
            "没有导入任何「{FILE_KIND_TENDER}」类文件——无法确认响应要求（只有附件/图纸不算招标包）"
        ));
    } else if tenders.len() > 1 {
        out.push(format!(
            "导入了 {} 份「{FILE_KIND_TENDER}」类文件（{}）——无法确定正本，请人工确认哪份是有效版本",
            tenders.len(),
            tenders.join("、")
        ));
    }
    // 补遗缺失已升格为**阻塞项**（L3 / [`Blocker::MissingAddendum`]），
    // 不再在这里重复一条警告——同一件事报两遍会让报告面噪音翻倍，
    // 而且「警告」与「阻塞」并列出现会削弱阻塞的分量。
    out
}

// ---------------------------------------------------------------- L3：补遗缺失检测

/// 补遗/澄清类文件的**类别关键字**（导入时 kind 字面量）。
///
/// 与 service.rs 的业务类别词表同源（招标文件|附件|图纸|补遗|答疑|评分办法|
/// 格式模板）；「澄清」不在枚举里但真实件常这么命名，一并认。
pub const ADDENDUM_KIND_WORDS: &[&str] = &[
    "补遗",
    "澄清",
    "答疑",
    "更正",
    "变更公告",
];

/// 补遗/澄清类文件的**文件名标记**（导出链/真实样本的实际命名）。
///
/// 真实语料实测（fixtures/corpus-A）：文件叫 `C-补遗文件01.docx`——
/// 既非 kind 也非标准词，靠名字里的「补遗」认出来。招标代理的命名习惯
/// 五花八门（`C-补遗01` / `澄清答疑函` / `更正公告`），故名字与 kind 双面。
pub const ADDENDUM_NAME_WORDS: &[&str] = &[
    "补遗",
    "澄清",
    "答疑",
    "更正",
    "变更",
];

/// 招标文件本体（正本）的类别字面量——L3 判定的「前提条件」。
///
/// 只有**确实导入过招标文件**时，「没有补遗」才有意义（否则整包没导，
/// 该报的是 [`tender_gaps`] 的空包告警，不是补遗缺失）。
const FILE_KIND_TENDER_L3: &str = "招标文件";

/// 是否补遗/澄清类文件（kind 或文件名命中）。
pub fn is_addendum_file(kind: &str, name: &str) -> bool {
    ADDENDUM_KIND_WORDS.iter().any(|w| kind.contains(w))
        || ADDENDUM_NAME_WORDS.iter().any(|w| name.contains(w))
}

/// L3 主判定：补遗缺失或解析失败 → 一条阻塞；一切正常 → None。
///
/// ## 判定（只认导入台账里的事实，不猜文件内容）
///
/// 1. **没有招标文件本体** → None（整包没导，那是 [`tender_gaps`] 的面，
///    不该在这里重复报「缺补遗」——空包比缺补遗更上游）；
/// 2. 有招标文件，**没有任何补遗件** → `MissingAddendum { NotProvided }`；
/// 3. 有补遗件，**全部解析失败** → `MissingAddendum { ParseFailed }`；
/// 4. 有补遗件，**部分成功** → None（读到了就是读到了；失败件在
///    [`addendum_parse_failures`] 单独暴露给报告面，不阻塞——
///    多份补遗里坏一份通常还有别的覆盖，过度阻塞会让工具不可用）；
/// 5. 补遗件 `OcrRequired` 不算失败（无文本层扫描件是已知可处理态，
///    与 ParseStatus 的三分语义一致）。
fn missing_addendum(store: &DomainStore) -> Option<Blocker> {
    // ① 没有招标文件本体 → 交回 tender_gaps（空包告警），不在本函数报
    let has_tender = store.files().iter().any(|f| f.kind == FILE_KIND_TENDER_L3);
    if !has_tender {
        return None;
    }

    let addenda: Vec<&bid_domain::TenderFile> = store
        .files()
        .iter()
        .filter(|f| is_addendum_file(&f.kind, &f.name))
        .collect();

    // ② 一件补遗都没有——L3 阻塞的人工出口：AddendumWaiver 门 Approved
    //    = 人工确认本项目无补遗（#131 真实语料验证：此前该确认通路不存在，
    //    单件真实招标文件被死锁——阻塞提示语承诺的「明确确认无补遗后再
    //    继续」在此落地）。导入补遗件后本分支不再触发，waiver 自然失效。
    if addenda.is_empty() {
        if store.gate_approved(bid_domain::review::Gate::AddendumWaiver) {
            return None;
        }
        return Some(Blocker::MissingAddendum {
            files: Vec::new(),
            reason: AddendumIssue::NotProvided,
        });
    }

    // ③ 有补遗但**全部**失败（Failed 是唯一失败态；OcrRequired 不算）
    let failed: Vec<String> = addenda
        .iter()
        .filter(|f| f.parse_status == bid_domain::ParseStatus::Failed)
        .map(|f| f.name.clone())
        .collect();
    if failed.len() == addenda.len() {
        return Some(Blocker::MissingAddendum {
            files: failed,
            reason: AddendumIssue::ParseFailed,
        });
    }

    None
}

/// 补遗解析失败件清单（**不阻塞**，但报告面要露出来——
/// 「3 份补遗坏 1 份」不该拦住工作，但不该让用户不知道）。
pub fn addendum_parse_failures(store: &DomainStore) -> Vec<String> {
    store
        .files()
        .iter()
        .filter(|f| is_addendum_file(&f.kind, &f.name))
        .filter(|f| f.parse_status == bid_domain::ParseStatus::Failed)
        .map(|f| f.name.clone())
        .collect()
}

// （旧 `tender_has_addendum` 已被 [`is_addendum_file`] 取代——L3 把判定面扩到 kind+名字双匹配，语义单一真源。）

/// 占位文本随需求生成（接线方可直接用）：本章第一条阻塞的建议占位。
pub fn first_placeholder(r: &DraftReadiness) -> Option<String> {
    r.blockers.first().map(Blocker::suggest_placeholder)
}