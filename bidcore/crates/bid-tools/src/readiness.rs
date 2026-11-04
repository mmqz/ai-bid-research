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
//! - 接线面：`draft_section` / `draft_section_iterative`（#110 任务 1 起）
//!   在生成前调 [`check_draft_readiness`]，阻塞 → E_UNPROCESSABLE；
//!   受限起草面（地图 P0-1）另消费 [`input_integrity`]——放行不等于
//!   没问题，响应显式标 RESTRICTED；
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
//! | UncoveredMandatoryItem | 本章负责的 **Agent 标必选**条款（非否决类）在矩阵未响应 / blocking | 阻塞（Agent 必选接线） |
//! | IncompleteTender | 招标包结构问题（无招标文件/多份正本/多份招标文件） | **警告**（信息面） |
//! | **MissingAddendum** | 招标文件已导入，但补遗/澄清**缺失或不就绪**（逐件分级） | **阻塞**（L3，#184 升级） |
//! | **UnreadyInput** | 招标包输入件（主件/附件/图纸——**非补遗**）不就绪且未逐件确认 | **阻塞**（地图 P0-1） |
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
//! ### #184 翻案：部分失败也阻塞（推翻「3 份补遗坏 1 份不该拦住工作」）
//!
//! **当初为何放行**：旧判定只看「是否**全部**解析失败」（
//! `failed.len() == addenda.len()` 才阻塞）——怕多份补遗坏一份就拦住整条
//! 工作流，过度阻塞会让工具不可用，还留了「坏件单独暴露不阻塞」的报告面。
//!
//! **现在为何推翻**：不知道 ≠ 没有。补遗 03 可能恰好是改控制价的那份——
//! 它解析失败 → 我们不知道它改了什么 → 按旧价报价 → 废标。「多份里坏
//! 一份通常还有别的覆盖」是赌概率，而废标是确定损失；工作流被拦住是
//! **可修复的摩擦**（修复重传 / 人工逐件确认放行），废标是**不可逆的结局**。
//! 阻塞的代价有出口（逐件 waiver），放行的代价没有出口。
//!
//! 新判定（分级就绪 + 逐件确认）：
//! - 每件补遗按 [`addendum_readiness`] 分级：Ok → Ready；OcrRequired 且
//!   OCR 已产出内容（ocr_blocks > 0）→ Ready；OcrRequired 零块 →
//!   OcrPending；Failed → Failed（详见 [`AddendumReadiness`]——含「为什么
//!   不区分更多态」的诚实说明）；
//! - **任一件不处 Ready 且无该 file_id 的逐件确认** → 阻塞（
//!   [`Blocker::MissingAddendum`]，逐件点名 + 原因）；
//! - 人工继续必须逐件记录 [`bid_domain::AddendumWaiverRecord`]（MCP 工具
//!   `record_addendum_waiver`）——非全局布尔，确认即审计面；
//! - 全局 Gate::AddendumWaiver（#131）语义不变：只救「无补遗件」分支，
//!   不掩蔽任何不就绪件（`addendum_waiver_does_not_mask_parse_failed` 钉死）。
//!
//! 「可用证据」= 状态 Active / Expiring（与 evidence.rs `can_bind`、
//! draft.rs `check_evidence_bindable` 同口径）——过期件不能算有。
//!
//! 措辞纪律：**面向业务说人话**。说「缺 3 个类似项目业绩，现有 1 条」，
//! 不说 `MissingPerformance { required: 3 }`。
//!
//! ## 地图 P0-1：输入完整性——部分失败不得错误放行
//!
//! #184 把**补遗**的不就绪升格为阻塞；P0-1 把同一判据推广到**整个招标包**
//! （主件/附件/图纸/评分办法/格式模板——一切非证据类导入件）：
//!
//! ```text
//! 主件 Ok + 补遗A Ok + 补遗B OCR_PENDING + 附件C 解析失败
//!   → 旧现状：「已提取出一部分需求」→ 允许起草
//!   → 后果：基于不完整输入写标书，漏掉其中的新控制价/新截止时间
//! ```
//!
//! 判据（可计算，文件级状态与项目级门禁联动）：
//! - 每件输入件按 [`addendum_readiness`] 分级（READY / OCR_PENDING /
//!   FAILED——同一标度，不另造第二套）；
//! - **主件 READY + 所有补遗 READY + 无其他输入件 OCR_PENDING/FAILED**
//!   → 完整（[`InputIntegrity::complete`]）；
//! - 任一件不就绪且无逐件人工确认 → 阻塞（补遗报
//!   [`Blocker::MissingAddendum`]，其余报 [`Blocker::UnreadyInput`]——
//!   同一件事不报两遍，人工出口同为 record_addendum_waiver）；
//! - 不就绪但已逐件确认 → 放行 + **受限起草**（`RESTRICTED`——放行
//!   ≠ 没问题，起草响应/账面/出参三面显式标注，不得静默当成完整输入）。
//!
//! 边界：**证据源文件（kind=证据）不在此面**——证据可用性归证据系统
//! 自己的生命周期裁决（状态/有效期/换版，见 evidence.rs）；本面只管
//! 「招标包输入是否读全了」。

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
    /// 本章负责的 **Agent 标必选**条款（否决类归 [`Blocker::UncoveredVetoItem`]
    /// 报，不双报）在合规矩阵里仍未被响应（必选漏响应 = 废标风险——Agent
    /// 已确认为需响应，不得静默漏写）。
    ///
    /// 与 [`Blocker::UncoveredVetoItem`] 的分界：那条管**类别阻断面**
    /// （否决项——含正则产物）；本条管 **Agent 必选标记**接线的面
    /// （matrix.rs `agent_mandatory`——AGENT_MANDATORY_NOT_BLOCKING 缺口的
    /// 起草半边）。类别已是否决项的 Agent 必选条款归那条报，不双报。
    ///
    /// 出口（与否决项同款）：绑证据 + 章节在位 → 矩阵 RESPONDED；或经人工
    /// receipt 覆写（set_matrix_override——不适用必须给理由）。
    UncoveredMandatoryItem { req_ids: Vec<String> },
    /// 招标包不完整（无招标文件本体 / 多份无法确定正本）。**警告面**。
    IncompleteTender { missing: Vec<String> },
    /// 招标文件已导入，但补遗/澄清件**缺失或不就绪**（L3——**阻塞**，#184
    /// 升级为逐件分级判定）。
    ///
    /// 与 [`Blocker::IncompleteTender`] 的分界：那条是「招标包结构可疑，
    /// 请人工确认」；这条是「已确认导入过招标文件，但补遗这一环断了」——
    /// 补遗含对条款的实质性修改，缺了就不能按现有条款做标。
    ///
    /// #184 起逐件点名：`files` 里每件带就绪度与原因（parse_summary /
    /// OCR 零块说明），阻塞消息照此展开——「3 份补遗坏 1 份」要说清坏的
    /// 是哪一份、坏在哪。
    MissingAddendum {
        /// 不就绪且未经人工确认的补遗件（逐件：文件 id / 名字 / 就绪度 / 原因）。
        /// `NotProvided` 形态为空——没有文件可列。
        files: Vec<AddendumFileState>,
        /// 缺失形态（分类标签——逐件真相在 `files` 里；混存时取更重的一档：
        /// 有 Failed 件报 ParseFailed，否则为 OcrPending）。
        reason: AddendumIssue,
    },
    /// 招标包输入件（主件/附件/图纸等，**非补遗**）不就绪且未逐件确认——
    /// 项目级阻塞（地图 P0-1：部分失败不得错误放行）。
    ///
    /// 与 [`Blocker::MissingAddendum`] 的分界：那条管补遗/澄清件（对条款
    /// 的实质性修改面——控制价/截止时间/评分办法）；本条管其余输入件——
    /// **主件读不了 = 章节树的全部条款都来自不可信的残缺输入**；附件/
    /// 图纸读不了 = 对应响应面可能有看不见的要求。「已提取出一部分需求」
    /// 不等于输入完整——门禁以文件级状态为准，不以「有没有产出」为准。
    ///
    /// 人工出口与补遗同款：逐件 [`bid_domain::AddendumWaiverRecord`]（
    /// MCP 工具 record_addendum_waiver——#184 起确认面按 file_id 落账，
    /// 覆盖所有招标包输入件；工具名带 addendum 是历史口径）。放行 ≠
    /// 没问题：起草响应带 `input_integrity.status = RESTRICTED`（受限
    /// 起草显式标注，不静默当完整输入）。
    UnreadyInput {
        /// 不就绪且未经人工确认的输入件（逐件：id/名/就绪度/原因）。
        files: Vec<AddendumFileState>,
    },
}

/// 单件补遗的就绪度分级（#184——判定矩阵的基元）。
///
/// - `Ok` → `Ready`：内容可读，条款修改可见；
/// - `OcrRequired` 且 `ocr_blocks > 0` → `Ready`：OCR 已产出内容
///   （与 service.rs `ocr_tier_of` 的 "ocr" 档同口径）；
/// - `OcrRequired` 且零块 → `OcrPending`：**域层暂无「OCR 跑过但零产出」
///   与「OCR 还没跑」的区分信号**（TenderFile 只记块数），两形态在本版本
///   合为一态。信号面补齐（导入链路区分 OcrEmpty/Pending）后应拆开；
///   现在不虚构区分——零块一律当「内容未知」，保守阻塞；
/// - `Failed` → `Failed`：解析器明确报错（CMap/加密/滤镜不支持）。
///
/// 注：issue #184 提过的 `Partial`（部分页成功）变体无信号支撑（
/// ParseStatus 无页级信息），不造死代码，不设该变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AddendumReadiness {
    Ready,
    OcrPending,
    Failed,
}

impl AddendumReadiness {
    pub fn as_str(&self) -> &'static str {
        match self {
            AddendumReadiness::Ready => "READY",
            AddendumReadiness::OcrPending => "OCR_PENDING",
            AddendumReadiness::Failed => "FAILED",
        }
    }
}

/// 单件文件的状态展开（阻塞消息 / MCP 报告面的逐件行）。
///
/// #184 时只服务补遗面；地图 P0-1 起同为 [`Blocker::UnreadyInput`] /
/// [`InputIntegrity`] 的逐件载体（字段本就是通用的 id/名/就绪度/原因，
/// 名字里的 Addendum 是历史口径，不另造第二套同构体）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddendumFileState {
    /// 文件 id（逐件确认的锚——record_addendum_waiver 按它对号）。
    pub file_id: String,
    /// 文件名（人类可读点名）。
    pub name: String,
    /// 就绪度分级。
    pub readiness: AddendumReadiness,
    /// 不就绪原因（Failed → parse_summary 原文；OcrPending → 零块说明）。
    pub reason: String,
}

/// 补遗缺失的形态（L3 规格 + #184 扩面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddendumIssue {
    /// 导入的文件里根本没有补遗/澄清件。
    ///
    /// 注意：真实项目**可能真的没有补遗**（没发过澄清）。所以这一条是
    /// 「必须人工确认」，不是「一定错误」——但确认之前不能当它不存在。
    /// 人工出口：Gate::AddendumWaiver（#131——全局确认本项目无补遗）。
    NotProvided,
    /// 有补遗件但解析失败（CMap 缺失/加密/滤镜不支持）。
    ///
    /// 这一条没有歧义：补遗确实存在，只是我们读不了它——
    /// 读不了就等于没有，绝不能按「没补遗」继续。
    /// 人工出口：逐件 [`bid_domain::AddendumWaiverRecord`]（#184）。
    ParseFailed,
    /// 有补遗件但 OCR 未产出任何内容（零块——内容未知，#184）。
    ///
    /// 与 ParseFailed 同为「不知道 ≠ 没有」的形态：件在场、内容不可见。
    /// 旧判定把 OcrRequired 一律当「不算失败」放行——对已 OCR 出内容的件
    /// 是对的，对零文本件就是把「不知道」当「没有」。人工出口同 ParseFailed。
    OcrPending,
}

impl AddendumIssue {
    pub fn as_str(&self) -> &'static str {
        match self {
            AddendumIssue::NotProvided => "NOT_PROVIDED",
            AddendumIssue::ParseFailed => "PARSE_FAILED",
            AddendumIssue::OcrPending => "OCR_PENDING",
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
            Blocker::UncoveredMandatoryItem { req_ids } => format!(
                "必选条款尚未响应：{} 条 Agent 标必选的条款在合规矩阵里还没响应（{}）——必选漏响应有废标风险，先绑证据或经人工覆写（set_matrix_override）后再起草",
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
                    "补遗/澄清文件解析失败：{}——补遗里有对条款的实质性修改（控制价/截止时间/参数/评分办法），读不了就等于没收到，按现有条款做标会直接废标。请修复重传，或逐件人工确认后放行（record_addendum_waiver）",
                    format_not_ready_files(files)
                ),
                AddendumIssue::OcrPending => format!(
                    "补遗/澄清文件 OCR 未完成（零内容）：{}——扫描件内容未知，不确定它改了什么就不能按旧条款做标。请先跑 OCR 或提供可解析版本，或逐件人工确认后放行（record_addendum_waiver）",
                    format_not_ready_files(files)
                ),
            },
            Blocker::UnreadyInput { files } => format!(
                "招标包输入不完整：{}——这些文件不是 READY（解析失败/OCR 零内容），「已提取出一部分需求」不等于输入完整（主件坏=全部条款来自残缺输入；附件/图纸坏=对应响应面可能有看不见的要求）。请修复重传，或逐件人工确认后放行（record_addendum_waiver——放行后为受限起草）",
                format_not_ready_files(files)
            ),
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
            Blocker::UncoveredMandatoryItem { .. } => "UNCOVERED_MANDATORY_ITEM",
            Blocker::IncompleteTender { .. } => "INCOMPLETE_TENDER",
            Blocker::MissingAddendum { .. } => "MISSING_ADDENDUM",
            Blocker::UnreadyInput { .. } => "UNREADY_INPUT",
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
            Blocker::UncoveredMandatoryItem { req_ids } => make_placeholder(
                PlaceholderKind::Confirm,
                &format!("{} 条必选条款待确认响应方式", req_ids.len()),
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
                        files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join("、")
                    ),
                ),
                AddendumIssue::OcrPending => make_placeholder(
                    PlaceholderKind::Missing,
                    &format!(
                        "补遗文件 {} OCR 未完成（零内容），请先跑 OCR 或提供可解析版本",
                        files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join("、")
                    ),
                ),
            },
            Blocker::UnreadyInput { files } => make_placeholder(
                PlaceholderKind::Confirm,
                &format!(
                    "招标包输入不完整（{}），请提供可解析版本或人工确认后继续",
                    files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join("、")
                ),
            ),
        }
    }
}

/// 阻塞消息里的逐件展开：「名字（原因）、名字（原因）」——#184 起要能
/// 对齐到具体是哪一份、坏在哪（验收 A：报具体文件名与原因）。
fn format_not_ready_files(files: &[AddendumFileState]) -> String {
    files
        .iter()
        .map(|f| {
            if f.reason.trim().is_empty() {
                f.name.clone()
            } else {
                format!("{}（{}）", f.name, f.reason)
            }
        })
        .collect::<Vec<_>>()
        .join("、")
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

    // ---- 输入完整性（地图 P0-1）：非补遗输入件（主件/附件/图纸）不就绪 —— 阻塞
    //
    // 全局面（与 missing_addendum 同位，先于章节判定——「查一个不存在的
    // 章节」也不能显得风平浪静）。补遗不在此列（missing_addendum 已管，
    // 同一件事不报两遍）；证据源文件不在此列（可用性归证据系统）。
    if let Some(blocker) = unready_input_files(store) {
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
        .filter(|r| !matrix_responded(store, &r.id))
        .map(|r| r.id.clone())
        .collect();
    if !veto_ids.is_empty() {
        blockers.push(Blocker::UncoveredVetoItem { req_ids: veto_ids });
    }

    // ---- 5b. Agent 必选条款响应（AGENT_MANDATORY_NOT_BLOCKING 缺口的
    //      起草半边）：本章负责的 Agent 标必选条款在矩阵未响应 → 阻塞。
    //      否决类除外（归 UncoveredVetoItem 报，不双报）；资格类虽已在
    //      类别阻断面（导出门拦），起草门此前只靠标题启发式（Missing-
    //      Qualification），这里补上逐条款的矩阵判定。判定与否决项同源：
    //      矩阵行 RESPONDED，或人工覆写不适用且带理由，才算已处理。
    let mandatory_ids: Vec<String> = reqs
        .iter()
        .filter(|r| r.category != CAT_VETO)
        .filter(|r| bid_domain::matrix::agent_mandatory(r))
        .filter(|r| !matrix_responded(store, &r.id))
        .map(|r| r.id.clone())
        .collect();
    if !mandatory_ids.is_empty() {
        blockers.push(Blocker::UncoveredMandatoryItem { req_ids: mandatory_ids });
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

/// 矩阵行是否「已响应」（否决项与 Agent 必选条款共用的覆盖判定）。
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
fn matrix_responded(store: &DomainStore, req_id: &str) -> bool {
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
    // #188：正本判定看任一角色（同内容先附件后招标文件导入也算正本在场）
    let tenders: Vec<&str> = store
        .files()
        .iter()
        .filter(|f| file_has_kind(store, f, FILE_KIND_TENDER))
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

// ---------------------------------------------------------------- 输入完整性（地图 P0-1）

/// 证据源文件的业务类别字面量（运行器侧自有类别——service.rs 分层：
/// MCP 壳的 kind 枚举里没有它，只有受信内部导入面会用）。
const FILE_KIND_EVIDENCE: &str = "证据";

/// 是否招标包输入件（输入完整性判定的覆盖面：主件/补遗/附件/图纸/
/// 评分办法/格式模板——一切非证据类导入件）。
///
/// 证据源文件不在面：证据的可用性由证据系统自己的生命周期裁决
/// （状态 Active/Expiring、有效期、换版），把「证据源文件解析失败」
/// 混进「招标包输入不完整」会让两套阻塞语义打架——前者该报在证据面。
pub fn is_tender_package_file(f: &bid_domain::TenderFile) -> bool {
    f.kind != FILE_KIND_EVIDENCE
}

/// 项目级输入完整性（地图 P0-1——「全部就绪才允许完整起草」的可计算判据）。
///
/// 覆盖**所有**招标包输入件（含补遗——受限判定要看全貌）；判据 =
/// 每件都处 [`AddendumReadiness::Ready`]，或坏件有逐件人工确认。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputIntegrity {
    /// 完整（unready 为空——所有输入件 READY 或经逐件确认放行）。
    pub complete: bool,
    /// 不就绪且未确认的件（非空 → 起草闸阻塞：补遗报
    /// [`Blocker::MissingAddendum`]，其余报 [`Blocker::UnreadyInput`]）。
    pub unready: Vec<AddendumFileState>,
    /// 已人工确认**但仍不就绪**的件（放行 ≠ 没问题——受限起草的来源）。
    pub waived_unready: Vec<AddendumFileState>,
}

impl InputIntegrity {
    /// 受限起草判定：完整性是靠人工确认支撐的 → 本次起草基于不完整
    /// 输入，响应必须显式标注（不得静默当成完整输入）。
    pub fn restricted(&self) -> bool {
        !self.waived_unready.is_empty()
    }

    /// 人类可读回报（受限 warning / draft_note 账面用）。
    pub fn to_message(&self) -> String {
        format!(
            "以下 {} 件输入不就绪但已经人工确认放行（{}）——本次起草基于不完整输入，其中不可见的要求由确认人承担",
            self.waived_unready.len(),
            format_not_ready_files(&self.waived_unready)
        )
    }
}

/// 输入完整性总判（项目级——起草闸的判据面 / 受限起草的标注面）。
///
/// 纯只读派生，随便调；逐件真相在 `unready`/`waived_unready`，
/// 与 [`addendum_statuses`]（报告面）同源同口径。
pub fn input_integrity(store: &DomainStore) -> InputIntegrity {
    let mut unready = Vec::new();
    let mut waived_unready = Vec::new();
    for f in store.files().iter().filter(|f| is_tender_package_file(f)) {
        if addendum_readiness(f) == AddendumReadiness::Ready {
            continue;
        }
        let state = AddendumFileState {
            file_id: f.id.clone(),
            name: f.name.clone(),
            readiness: addendum_readiness(f),
            reason: not_ready_reason(f),
        };
        if store.addendum_waived(&f.id) {
            waived_unready.push(state);
        } else {
            unready.push(state);
        }
    }
    InputIntegrity { complete: unready.is_empty(), unready, waived_unready }
}

/// 非补遗输入件的不就绪检查（地图 P0-1——[`Blocker::UnreadyInput`] 的
/// 判定体）。
///
/// 与 [`missing_addendum`] 的分界：那条管补遗/澄清件（kind 或名字命中
/// [`is_addendum_file`]）；本条管其余——两集合不相交，不会双报。
fn unready_input_files(store: &DomainStore) -> Option<Blocker> {
    let files: Vec<AddendumFileState> = store
        .files()
        .iter()
        .filter(|f| is_tender_package_file(f) && !is_addendum_file(&f.kind, &f.name))
        .filter(|f| addendum_readiness(f) != AddendumReadiness::Ready)
        .filter(|f| !store.addendum_waived(&f.id))
        .map(|f| AddendumFileState {
            file_id: f.id.clone(),
            name: f.name.clone(),
            readiness: addendum_readiness(f),
            reason: not_ready_reason(f),
        })
        .collect();
    if files.is_empty() {
        None
    } else {
        Some(Blocker::UnreadyInput { files })
    }
}

// ---------------------------------------------------------------- L3：补遗缺失检测（#184 分级就绪）

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

/// #188：文件的全部业务身份（初始 = 导入台账的 kind/name；后续 = FileRole
/// 登记——同 SHA 异身份二次导入不吞角色）。判定面（补遗/正本）看任一身份。
fn file_identities<'a>(
    store: &'a DomainStore,
    f: &'a bid_domain::TenderFile,
) -> Vec<(&'a str, &'a str)> {
    let mut v = vec![(f.kind.as_str(), f.name.as_str())];
    v.extend(
        store
            .file_roles(&f.id)
            .into_iter()
            .map(|r| (r.kind.as_str(), r.declared_name.as_str())),
    );
    v
}

/// 是否补遗/澄清类文件（#188 扩面：任一身份命中即算——先「附件」后「补遗」
/// 导入的同一份内容，补遗判定看得到它）。
fn file_is_addendum(store: &DomainStore, f: &bid_domain::TenderFile) -> bool {
    file_identities(store, f)
        .iter()
        .any(|(k, n)| is_addendum_file(k, n))
}

/// 首个命中补遗判定的身份名（报告面用——与判定依据一致；无 → None）。
fn addendum_identity_name(store: &DomainStore, f: &bid_domain::TenderFile) -> Option<String> {
    file_identities(store, f)
        .into_iter()
        .find(|(k, n)| is_addendum_file(k, n))
        .map(|(_, n)| n.to_string())
}

/// 文件是否具有某类别身份（#188：任一角色命中即算——正本/证据等类别
/// 判定与补遗判定同扩面，不再只看首次导入的 kind）。
fn file_has_kind(store: &DomainStore, f: &bid_domain::TenderFile, kind: &str) -> bool {
    f.kind == kind || store.file_roles(&f.id).iter().any(|r| r.kind == kind)
}

/// 单件补遗的就绪度分级（#184 判定矩阵的基元，详见 [`AddendumReadiness`]）。
pub fn addendum_readiness(f: &bid_domain::TenderFile) -> AddendumReadiness {
    use bid_domain::ParseStatus;
    match f.parse_status {
        ParseStatus::Ok => AddendumReadiness::Ready,
        // 已 OCR 出内容（ocr_blocks>0）→ 可读，与 service.rs ocr_tier_of
        // 的 "ocr" 档同口径；零块 → 内容未知（详见枚举文档：域层无
        // 「OCR 跑过但零产出」信号，OcrEmpty/Pending 合一态，不虚构区分）
        ParseStatus::OcrRequired => {
            if f.ocr_blocks > 0 {
                AddendumReadiness::Ready
            } else {
                AddendumReadiness::OcrPending
            }
        }
        ParseStatus::Failed => AddendumReadiness::Failed,
    }
}

/// 不就绪原因文本（Failed → parse_summary 原文；OcrPending → 零块说明）。
fn not_ready_reason(f: &bid_domain::TenderFile) -> String {
    match addendum_readiness(f) {
        AddendumReadiness::Failed => {
            if f.parse_summary.trim().is_empty() {
                "解析失败（未携带原因）".to_string()
            } else {
                f.parse_summary.clone()
            }
        }
        AddendumReadiness::OcrPending => {
            "无文本层扫描件且 OCR 未产出任何内容（零块）——内容未知".to_string()
        }
        AddendumReadiness::Ready => String::new(), // 不入不就绪清单，不占理由面
    }
}

/// 报告面的单件补遗状态（就绪度 + 是否已人工确认）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddendumStatus {
    pub file_id: String,
    pub name: String,
    pub readiness: AddendumReadiness,
    /// 不就绪原因（Ready 件为空串）。
    pub reason: String,
    /// 已逐件人工确认（record_addendum_waiver 的审计回显面）。
    pub waived: bool,
}

/// 全部补遗件的逐件状态（报告面——含已确认件：确认了也要能查到，审计面
/// 不因放行而消失）。MCP import 响应 / record_addendum_waiver 出参的
/// `addendum_readiness` 块由此组装（含 waived 标记）。
///
/// 取代旧 `addendum_parse_failures`（#184 前的死代码：只列 Failed 名字、
/// 零消费者，且语义已被分级判定覆盖——「不就绪」不止 Failed 一种）。
pub fn addendum_statuses(store: &DomainStore) -> Vec<AddendumStatus> {
    store
        .files()
        .iter()
        // #188：任一身份命中即算补遗（去重不再吞角色）
        .filter(|f| file_is_addendum(store, f))
        .map(|f| AddendumStatus {
            file_id: f.id.clone(),
            // 名字面用「触发补遗判定的那个身份」（同内容多角色时，报告名
            // 与判定依据一致——如第二次以「补遗-03.pdf」导入的角色名）
            name: addendum_identity_name(store, f).unwrap_or_else(|| f.name.clone()),
            readiness: addendum_readiness(f),
            reason: not_ready_reason(f),
            waived: store.addendum_waived(&f.id),
        })
        .collect()
}

/// L3 主判定：补遗缺失或任一件不就绪且未逐件确认 → 一条阻塞；
/// 一切就绪（或已逐件放行） → None。
///
/// ## 判定（只认导入台账里的事实，不猜文件内容）
///
/// 1. **没有招标文件本体** → None（整包没导，那是 [`tender_gaps`] 的面，
///    不该在这里重复报「缺补遗」——空包比缺补遗更上游）；
/// 2. 有招标文件，**没有任何补遗件** → `MissingAddendum { NotProvided }`
///    （人工出口：Gate::AddendumWaiver，#131）；
/// 3. 有补遗件，**任一件不处 Ready 且无该 file_id 的逐件确认** →
///    `MissingAddendum { ParseFailed | OcrPending }`，files 逐件点名
///    （id/名字/就绪度/原因）。**#184 翻案**：旧判定是
///    `failed.len() == addenda.len()` 才阻塞——只有全部失败才拦，部分
///    失败放行。当初的理由是「3 份补遗坏 1 份不该拦住工作」（怕过度
///    阻塞让工具不可用）；推翻它因为**不知道 ≠ 没有**：坏的那份可能正
///    是改控制价的，读不到不等于没改——按旧条款报价就是废标。工作流
///    被拦是可修复的摩擦（修复重传 / 逐件 waiver），废标是不可逆的结局；
///    分级阻塞 + 逐件确认让「放行」也变成留底的可审计行为，而非静默赌
///    概率；
/// 4. **全部 Ready**（含 OCR 已产出内容的件）→ None——验收 D 直接放行；
/// 5. **不就绪但已有逐件确认** → 该件不计入阻塞（确认即审计面；
///    导入可解析版本后阻塞自然消失，确认记录无害残留）。
fn missing_addendum(store: &DomainStore) -> Option<Blocker> {
    // ① 没有招标文件本体 → 交回 tender_gaps（空包告警），不在本函数报
    //（#188：正本身份看任一角色——先附件后招标文件的二次导入也算）
    let has_tender = store
        .files()
        .iter()
        .any(|f| file_has_kind(store, f, FILE_KIND_TENDER_L3));
    if !has_tender {
        return None;
    }

    let statuses = addendum_statuses(store);

    // ② 一件补遗都没有——L3 阻塞的人工出口：AddendumWaiver 门 Approved
    //    = 人工确认本项目无补遗（#131 真实语料验证：此前该确认通路不存在，
    //    单件真实招标文件被死锁——阻塞提示语承诺的「明确确认无补遗后再
    //    继续」在此落地）。导入补遗件后本分支不再触发，waiver 自然失效。
    //    （#184 语义不变：全局门只救 NotProvided——不掩蔽任何不就绪件。）
    if statuses.is_empty() {
        if store.gate_approved(bid_domain::review::Gate::AddendumWaiver) {
            return None;
        }
        return Some(Blocker::MissingAddendum {
            files: Vec::new(),
            reason: AddendumIssue::NotProvided,
        });
    }

    // ③ 任一件不处 Ready 且无该 file_id 的逐件确认 → 阻塞（#184 判定矩阵）。
    //    旧实现是 `failed.len() == addenda.len()`（全坏才拦）——翻案理由
    //    见函数级文档：不知道 ≠ 没有，部分失败也是「不知道」。
    let blocking: Vec<AddendumFileState> = statuses
        .iter()
        .filter(|s| s.readiness != AddendumReadiness::Ready && !s.waived)
        .map(|s| AddendumFileState {
            file_id: s.file_id.clone(),
            name: s.name.clone(),
            readiness: s.readiness,
            reason: s.reason.clone(),
        })
        .collect();
    if blocking.is_empty() {
        return None;
    }
    // 混存取重：有 Failed 件报 ParseFailed，否则 OcrPending（逐件真相在 files）
    let reason = if blocking
        .iter()
        .any(|f| f.readiness == AddendumReadiness::Failed)
    {
        AddendumIssue::ParseFailed
    } else {
        AddendumIssue::OcrPending
    };
    Some(Blocker::MissingAddendum { files: blocking, reason })
}

// （旧 `tender_has_addendum` 已被 [`is_addendum_file`] 取代——L3 把判定面扩到 kind+名字双匹配，语义单一真源。）
// （旧 `addendum_parse_failures`（只列 Failed 名字、零消费者）已被 [`addendum_statuses`]
//  取代——#184 的非 Ready 清单不止 Failed 一种，且接线到 MCP import 响应与
//  record_addendum_waiver 出参，不再留死代码。）

/// 占位文本随需求生成（接线方可直接用）：本章第一条阻塞的建议占位。
pub fn first_placeholder(r: &DraftReadiness) -> Option<String> {
    r.blockers.first().map(Blocker::suggest_placeholder)
}