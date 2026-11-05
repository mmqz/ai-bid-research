//! bid-domain —— 标书领域模型与权威状态机（docs/bid-domain-model.md 的 Rust 落地）。
//!
//! 纪律（对齐设计文档）：
//! - 状态转换由本 crate 裁决，工具层只能申请（越级/非法 → `E_STATE`，零副作用）；
//! - ID 命名空间隔离：PRJ/TF/REQ/EV/CMP/SEC/VF/RT 由业务库签发，DOC-* 属文档内核（本 crate 只存引用）；
//! - 诚实状态：未确认/缺证据一律显式枚举，禁止用空值或 0 默认替代；
//! - 本 crate 零 IO（纯逻辑 + 内存态），持久化由 bid-tools 层承接。
//!
//! M1 范围：项目状态机前段（DRAFT→FILES_IMPORTED→REQUIREMENTS_EXTRACTED）+
//! 幂等键存储 + content_version 乐观并发 + E_STATE 越级防护（TC-11）+
//! 幂等重放（TC-07）。矩阵/起草/审校/导出的后段状态在 M2+ 扩展。
//!
//! M1.5 扩展（V2/V3 SEMUL 语义实装——四关键案例契约测试的载体）：
//! - [`impact`]：影响面引擎（补遗级联/DIRTY/content_version/终裁门禁/幂等）；
//! - [`linkage`]：证据链状态账本（只降不升 + 显式重验 + 外部修改不自动 adopt）；
//! - [`export_gate`]：两阶段导出门（EXPORTING → sha256 落库为界 → EXPORTED）。
//!
//! V3-1 扩展（issue #11 批次 v3——三份台账起步 + 持久化地基）：
//! - [`locator`]：引擎定位器七变体镜像（serde 形态与 mdcore 逐字节一致）；
//! - [`dates`]：ISO 日期工具（有效期窗口推算的单一真源）；
//! - [`evidence`]：T-1 Evidence 证据台账（只降不升 + R-03 过期禁绑 + 换版 Superseded）；
//! - [`outline`]：T-5 OutlineNode 章节树（九不变量 + 从 REQ 推导 + 配额最大余数法）；
//! - [`StoreSnapshot`]：T-8 持久化快照 DTO（本 crate 仍零 IO——磁盘读写住 bid-tools）。
//!
//! V3-2 扩展（issue #11 批次 v3——三份台账收尾）：
//! - [`matrix`]：T-2 MatrixRow 需求响应矩阵（四态派生 + blocking 导出闸门 R-03/R-04）；
//! - [`scoring`]：T-3 ScoringItem 评分矩阵（分值/标准原文/硬门三维结构，派生态）。
//!
//! V3-3 扩展（issue #11 批次 v3——技术参数响应面）：
//! - [`param`]：T-4 ParamResponse 参数响应表（偏离自动检出——正/负/无/不可比，
//!   负偏离=废标级全量查询面）。
//!
//! V3-4 扩展（issue #11 批次 v3——任务分工面）：
//! - [`assignment`]：T-6 Assignment 任务分工（章节/条款批量分配 + list_tasks
//!   owner/status 双过滤 + 重指派复位语义——S-8 并发调度的前置实体）。
//! V3-4 扩展（issue #11 批次 v3——审核闭环）：
//! - [`review`]：T-7 ReviewRecord + RevisionTask 审核闭环（三道门门序强制 +
//!   BLOCKER 强制派生 RT + 复审人 human: 前缀 + 导出前置裁决 `export_allowed`）。
//!
//! V3-5 扩展（issue #11 批次 v3——单章调度输入面）：
//! - [`budget`]：S-7 上下文预算与证据裁剪（溢出四阶梯 + 约束区永不截断 +
//!   `truncated_evidence` 标记 + 确定性 token 估算——「单章输入 ≤ 8k」的裁决面）。
//!
//! B1 扩展（issue #84 B1 / P0——价格分，从零到一）：
//! - [`pricing`]：价格分引擎（基准价五方法 + 偏离扣分 + `score`/`optimal_price`
//!   ——「报 X 万得几分 / 报多少分最高」的裁决面）。

pub mod assignment;
pub mod budget;
pub mod dates;
pub mod evidence;
pub mod export_gate;
pub mod forms;
pub mod impact;
pub mod linkage;
pub mod locator;
pub mod matrix;
pub mod outline;
pub mod param;
pub mod party;
pub mod pricing;
pub mod principal;
pub mod review;
pub mod scoring;
pub mod scoring_outline;
pub mod coverage;
pub mod segments;

pub use principal::{ActorType, TrustedPrincipal};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------- 错误码（mcp-contract §0）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolError {
    /// 状态机拒绝越级或非法转换（无副作用）
    EState {
        current: ProjectStatus,
        attempted: &'static str,
    },
    /// project_version 过期（乐观并发冲突）
    EConflict { expected: u64, got: u64 },
    /// 对象不存在
    ENotFound { what: String },
    /// 参数不可用/不可恢复
    EUnprocessable { reason: String },
    /// 部分条目失败
    EPartial { item_errors: Vec<String> },
    /// #243：全稿最终校验前置拒——起草未完成（存在未达 DRAFTED 的叶章节）。
    ///
    /// 零副作用通道（不跑规则/不推进状态/不落版本/不动审核记录）；
    /// `missing` = 未完成叶的 section_id 清单（可操作：逐叶补起草后重校，
    /// 或走 scope=sections 做章节级检查）。
    EDraftIncomplete { missing: Vec<String> },
}

impl ToolError {
    pub fn code(&self) -> &'static str {
        match self {
            ToolError::EState { .. } => "E_STATE",
            ToolError::EConflict { .. } => "E_CONFLICT",
            ToolError::ENotFound { .. } => "E_NOTFOUND",
            ToolError::EUnprocessable { .. } => "E_UNPROCESSABLE",
            ToolError::EPartial { .. } => "E_PARTIAL",
            ToolError::EDraftIncomplete { .. } => "E_DRAFT_INCOMPLETE",
        }
    }
}

// ---------------------------------------------------------------- 项目状态机（domain-model §2）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectStatus {
    Draft,
    FilesImported,
    RequirementsExtracted,
    MatrixReady,
    EvidenceBound,
    Drafting,
    Validating,
    Reviewing,
    ReadyToExport,
    Exported,
}

impl ProjectStatus {
    /// 合法前向转换表（权威状态机——唯一真源）。
    pub fn next_allowed(&self) -> &'static [ProjectStatus] {
        use ProjectStatus::*;
        match self {
            Draft => &[FilesImported],
            FilesImported => &[RequirementsExtracted],
            RequirementsExtracted => &[MatrixReady],
            MatrixReady => &[EvidenceBound],
            EvidenceBound => &[Drafting],
            Drafting => &[Validating],
            Validating => &[Reviewing],
            Reviewing => &[ReadyToExport],
            ReadyToExport => &[Exported],
            Exported => &[],
        }
    }

    /// 每个状态允许调用的工具（mcp-contract 各工具「状态」行——越级防护的裁决表）。
    ///
    /// #59 阶段 2 补全（export 面与既有 PR #60 的 draft/validate 面同表——
    /// 三 PR 合并时本表取并集，冲突面一眼可解）：
    /// - `draft_response_section`：MatrixReady+（矩阵就绪即可起草；#60 落地）；
    /// - `validate_bid_document`：Drafting+（起草面随时可复验；#60 登记、
    ///   阶段 3 接线）；
    /// - `export_bid_document`：Drafting+ 与 EXPORTED——导出的硬闸门在工具
    ///   内部的**双前置检查**（矩阵 blocking 行清零 + 审核 BLOCKER findings
    ///   清零，契约 §8 precheck），不在状态机面预拦：阶段 3 的 validate 工具
    ///   落地前，项目没有走 Validating→Reviewing→ReadyToExport 的通道，
    ///   状态机预拦会把 e2e 锁死。EXPORTED 允许重导（幂等/换路径）。
    pub fn allowed_tools(&self) -> &'static [&'static str] {
        use ProjectStatus::*;
        match self {
            Draft => &[
                "create_bid_project",
                "import_tender_files",
                "get_bid_project_state",
            ],
            FilesImported => &[
                "import_tender_files",
                "extract_requirements",
                "get_compliance_matrix",
                "search_evidence",
                "get_bid_project_state",
                // #184：补遗分级阻塞的人工出口（导入后任意时刻可逐件确认——
                // 与 Gate::AddendumWaiver 门同窗口语义，坏补遗不能拖到 MATRIX_READY+ 才能救）
                "record_addendum_waiver",
                // #219 全块覆盖（导入后、抽取前后均可标——Agent 先读块再判断）
                "get_document_chunks",
                "submit_block_marks",
                "get_coverage_report",
            ],
            RequirementsExtracted => &[
                "import_tender_files",
                "extract_requirements",
                "get_compliance_matrix",
                "search_evidence",
                "get_bid_project_state",
                // #80 第三批 MCP 补齐 G1：建纲+矩阵首建（P3 的前置）
                "build_outline",
                // #184：同 FilesImported 的窗口语义（导入后任意时刻）
                "record_addendum_waiver",
                // #219 全块覆盖（建纲前最后机会补标——覆盖不足时 build_outline 应告警）
                "get_document_chunks",
                "submit_block_marks",
                "get_coverage_report",
            ],
            MatrixReady | EvidenceBound => &[
                "get_coverage_report",
                "get_compliance_matrix",
                "search_evidence",
                "get_bid_project_state",
                "draft_response_section",
                // #254 分段写入（与整稿同窗口——矩阵就绪即可续写）
                "append_section_segment",
                "rewrite_section_segment",
                "list_section_segments",
                "check_bid_quality",
                // #80 第三批 MCP 补齐 G2/G3：证据登记绑定 + 审批门（P4 面）
                "register_evidence",
                "bind_evidence",
                "record_review_gate",
                // #184：补遗分级阻塞的逐件确认（导入后任意时刻）
                "record_addendum_waiver",
                // #100 任务 3：B2 自评接线（只读诊断——与 check_bid_quality 同窗口）
                "score_section",
                "score_document",
                // #226：人工覆写条款响应（否决项死锁出口——矩阵已建才有可覆写对象）
                "set_matrix_override",
            ],
            Drafting => &[
                "get_coverage_report",
                "get_compliance_matrix",
                "search_evidence",
                "get_bid_project_state",
                "draft_response_section",
                // #254 分段写入（起草主战场——长章节安全续写）
                "append_section_segment",
                "rewrite_section_segment",
                "list_section_segments",
                "validate_bid_document",
                "export_bid_document",
                "check_bid_quality",
                // G3：PreExportConfirm 门在 Drafting+ 窗口内
                "record_review_gate",
                // #184：补遗分级阻塞的逐件确认（导入后任意时刻）
                "record_addendum_waiver",
                "score_section",
                "score_document",
                // #226：起草期照样要能翻案（发现某条是规则说明 → 标不适用）
                "set_matrix_override",
                // #241 裁定：证据补件通道在起草期必须可用——validate/
                // check_bid_quality 在本态输出 evidence_missing 清单，「见缺口
                // →补件→就绪」的解决出口不能断在最后一步（与 #226
                // set_matrix_override 同款窗口理由；#245 占位换版升级也依赖
                // 本通道）。域层自有 R-03 闸（EXPIRED/SUPERSEDED 禁绑）保证
                // 绑定质量，状态门不承担绑定校验职责。
                "register_evidence",
                "bind_evidence",
            ],
            Validating => &[
                "get_compliance_matrix",
                "get_coverage_report",
                "search_evidence",
                "get_bid_project_state",
                // #254 只读段账本（校验期对账面——写作工具不重开）
                "list_section_segments",
                "validate_bid_document",
                "export_bid_document",
                "check_bid_quality",
                "record_review_gate",
                // #184：补遗分级阻塞的逐件确认（导入后任意时刻）
                "record_addendum_waiver",
                "score_section",
                "score_document",
                // #226：导出前复核期同样可翻案（这里是 blocking 最后能解的地方）
                "set_matrix_override",
                // #241 裁定：复核期补件（readiness 证据缺口的解决出口——与
                // set_matrix_override 同窗口同理由：导出前是缺口的最后解处）
                "register_evidence",
                "bind_evidence",
                // #243：历史异常态（VALIDATING + 缺章——旧版校验误推进所致）
                // 的受支持恢复出口；三重约束见 recover_validating_to_drafting
                "recover_validating_state",
            ],
            Reviewing | ReadyToExport => &[
                "get_compliance_matrix",
                "get_coverage_report",
                "search_evidence",
                "get_bid_project_state",
                // #254 只读段账本（复核期对账面）
                "list_section_segments",
                "validate_bid_document",
                "export_bid_document",
                "check_bid_quality",
                "record_review_gate",
                // #184：补遗分级阻塞的逐件确认（导入后任意时刻）
                "record_addendum_waiver",
                "score_section",
                "score_document",
                // #226：导出前复核期同样可翻案（这里是 blocking 最后能解的地方）
                "set_matrix_override",
                // #241 裁定：导出前复核期补件（readiness 证据缺口的最后解处
                // ——与 set_matrix_override 同窗口同理由）
                "register_evidence",
                "bind_evidence",
            ],
            Exported => &[
                "get_coverage_report",
                "get_compliance_matrix",
                "search_evidence",
                "get_bid_project_state",
                "export_bid_document",
                // #254 只读段账本（导出后追溯面）
                "list_section_segments",
                "check_bid_quality",
                "record_review_gate",
                // #184：补遗分级阻塞的逐件确认（导入后任意时刻）
                "record_addendum_waiver",
                "score_section",
                "score_document",
                // #226：已导出仍可改判（再导出走幂等重导）
                "set_matrix_override",
            ],
        }
    }

    pub fn as_str(&self) -> &'static str {
        use ProjectStatus::*;
        match self {
            Draft => "DRAFT",
            FilesImported => "FILES_IMPORTED",
            RequirementsExtracted => "REQUIREMENTS_EXTRACTED",
            MatrixReady => "MATRIX_READY",
            EvidenceBound => "EVIDENCE_BOUND",
            Drafting => "DRAFTING",
            Validating => "VALIDATING",
            Reviewing => "REVIEWING",
            ReadyToExport => "READY_TO_EXPORT",
            Exported => "EXPORTED",
        }
    }
}

// ---------------------------------------------------------------- 领域对象（M1 子集）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BidProject {
    pub id: String, // PRJ-xxxx
    pub name: String,
    pub code: String,
    pub mode: String, // 明标 | 暗标（M1 透传，导出面在 M4 消费）
    pub status: ProjectStatus,
    pub content_version: u64,
    pub external_key: String, // 创建幂等键
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseStatus {
    Ok,
    OcrRequired, // 无文本层扫描件——OCR 分档输入（不算失败）
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TenderFile {
    pub id: String, // TF-xxxx
    pub name: String,
    pub kind: String,   // 招标文件|附件|图纸|补遗|证据
    pub sha256: String, // 天然幂等键
    pub pages: usize,
    pub tables: usize,
    pub chunks: usize,
    pub ocr_blocks: usize, // OCR 提取块数（非扫描件为 0）
    pub parse_status: ParseStatus,
    pub parse_summary: String,
}

/// 文件的业务角色（#188——内容身份与业务角色分离）。
///
/// 同一内容（sha256 幂等键）可在不同批次以不同身份进包：先「附件」后
/// 「补遗」——第二次导入不再被 SHA 去重静默吞掉角色。角色**追加不覆盖**
///（审计面：每次导入的身份都留底）；补遗/正本判定看任一角色
///（readiness.rs——初始身份 = 首次导入的 `TenderFile.kind/name`，后续身份
/// = 本表登记）。幂等：完全相同的 (kind, declared_name) 重放 → no-op。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRole {
    pub file_id: String, // TF-xxxx（锚定 files——孤儿角色在 restore 拒载）
    pub kind: String, // 招标文件|附件|图纸|补遗|证据…（导入时声明）
    pub declared_name: String, // 导入时提供的文件名（调用方口径，保留原文）
    pub at: u64,                  // 角色登记时刻（unix epoch 秒）
}

/// 补遗逐件人工确认记录（#184）。
///
/// 分级就绪的伴生闸：任一补遗不处 Ready 时起草闸阻塞，人工继续必须**逐件**
/// 确认（非全局布尔——「3 份补遗坏 1 份」要对齐到具体是哪一份、为什么故
/// 意带病放行）。记录即审计面：file_id 锚定具体件、reason 是人类给出的
/// 免责理由、principal 是确认人。
///
/// - `principal` 字符串字段：`human:{actor_id}` 派生值（legacy 回显/报告面
///   ——「#182 之前的口径」的兼容载体，**不得另造第二套人设体系**）；
/// - `trusted`（#182）：验签后的 [`TrustedPrincipal`]。None = legacy 快照
///   （#182 之前的记录——宽容载入，裁决面按旧口径不回溯失效）；新记录
///   必 Some（record_addendum_waiver 收 TrustedPrincipal 构造）。
/// - 同一 file_id 重复确认 = 追加新记录（审计日志语义：每次确认都留底，
///   查询面返回全部记录；解除确认不走本面——导入可解析版本后阻塞自然
///   消失，确认记录无害残留，与 AddendumWaiver 门同款语义）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddendumWaiverRecord {
    pub file_id: String, // TF-xxxx（必须存在于 files——对不存在的件做确认 = 凭空免责）
    pub reason: String,  // 人工理由（非空白——「不可审计的确认」不成确认）
    pub at: u64,         // 确认时刻（unix epoch 秒）
    pub principal: String, // 确认人（human:{actor_id} 派生值——legacy 回显面）
    /// #182 可信身份：验签后的 TrustedPrincipal。None = legacy 快照
    /// （#182 之前的记录——宽容载入；新记录必须 Some）。
    #[serde(default)]
    pub trusted: Option<TrustedPrincipal>,
}

/// #243 状态恢复审计记录（VALIDATING → DRAFTING 的唯一受支持回退边）。
///
/// 历史缺陷（#243 之前的 main）：全局校验在草稿未完成/校验失败时仍无条件
/// 推进 VALIDATING，而 VALIDATING 态禁起草 → 项目死锁，用户只能终止进程
/// 并手工改 db.json 的状态字段。本记录是受支持恢复通道的审计面——每次
/// 恢复都留底（何时/为何/从哪到哪/当时缺哪些章）。恢复通道的三重约束
/// 与状态不变量（章节树/矩阵/审核记录不动）见
/// [`DomainStore::recover_validating_to_drafting`]。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateRecoveryRecord {
    /// 恢复时刻（unix epoch 秒）
    pub at: u64,
    /// 恒 VALIDATING（恢复的唯一起点）
    pub from: ProjectStatus,
    /// 恒 DRAFTING（恢复的唯一终点——起草面解锁）
    pub to: ProjectStatus,
    /// 恢复理由（非空白——不可审计的恢复不成恢复）
    pub reason: String,
    /// 恢复时的未完成叶快照（SEC-* 清单——审计面：恢复针对的缺章事实）
    pub missing_sections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,       // REQ-xxxx
    pub category: String, // 资格条件|否决项|评分项|技术参数|商务要求|格式要求|其他
    pub clause_no: Option<String>,
    pub anchors: Vec<Anchor>, // 溯源锚（可多文件多锚点——GT-07 双锚点形态）
    pub params: HashMap<String, serde_json::Value>,
    pub source_file: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub file: String,
    pub kind: String,    // para|table|ocr|xlsx_cell
    pub locator: String, // 章节路径 / 表行 / 页
    pub excerpt: String, // 原文引用片段（对齐判分依据——创建时固化，不可篡改）
}

// ---------------------------------------------------------------- 权威状态机存储
/// 领域库（内存态）：项目/文件/条款 + 幂等键存储 + content_version。
/// 所有写入经 `apply` 走状态机裁决——直接改字段绕过状态机是不可能的（字段私有 + 仅经受审方法）。
///
/// V3-1（T-8 扩面）：证据台账 / 绑定 / 章节树三集合入列。快照互操作经
/// [`DomainStore::to_snapshot`] / [`DomainStore::restore`]（纯函数——磁盘 IO 在
/// bid-tools `store_io`，本 crate 零 IO 纪律不变）。
pub struct DomainStore {
    project: Option<BidProject>,
    files: Vec<TenderFile>,
    requirements: Vec<Requirement>,
    /// T-1 证据台账（EV-xxxx——登记/换版/有效期裁决见 evidence.rs 的 impl 块）
    evidences: Vec<evidence::Evidence>,
    /// T-1 证据↔条款绑定（T-2 MatrixRow 落地后由矩阵行消费）
    evidence_bindings: Vec<evidence::EvidenceBinding>,
    /// T-5 章节树（SEC-xxxx——树校验/推导/状态推进见 outline.rs 的 impl 块）
    sections: Vec<outline::OutlineNode>,
    /// T-6 任务分工台账（target_id → 执行人/截止日/人侧进度——见 assignment.rs
    /// 的 impl 块；target_id 引用 SEC-/REQ-，无独立 id）
    assignments: Vec<assignment::Assignment>,
    /// T-2 矩阵行（CMP-xxxx——派生态，build_matrix 幂等重建；blocking = 导出闸门）
    matrix: Vec<matrix::MatrixRow>,
    /// T-2 人工覆写（不适用/偏离——派生态之上的唯一人工输入）
    matrix_overrides: HashMap<String, matrix::MatrixOverride>,
    /// T-4 参数响应表（PR-xxxx——技术参数逐行比对账本；偏离派生见 param.rs）
    params: Vec<param::ParamResponse>,
    /// T-7 审核发现（FND-xxxx——三道门复审产出；BLOCKER 强制派生 RT）
    findings: Vec<review::Finding>,
    /// T-7 审核记录（REV-xxxx——过门里程碑 + 拒审过程记录）
    reviews: Vec<review::ReviewRecord>,
    /// T-7 修订工单（RT-xxxx——BLOCKER 的关单面；全 DONE 才可导出）
    revision_tasks: Vec<review::RevisionTask>,
    /// #184 补遗逐件人工确认（非 Ready 件的人工放行账——追加式审计日志）
    addendum_waivers: Vec<AddendumWaiverRecord>,
    /// #188 文件业务角色（内容身份 sha256 唯一，角色可多——追加不覆盖；
    /// 登记见 [`DomainStore::register_file_role`]，不碰 content_version）
    file_roles: Vec<FileRole>,
    /// #219 全块覆盖标记（文件 ID → 区间标记列表；Agent 判断面，
    /// 引擎只做区间校验与覆盖统计——见 coverage.rs）
    block_marks: HashMap<String, Vec<coverage::BlockMark>>,
    /// #219 二步：Agent 提交的关键点（文件 ID → 提交序；
    /// `submit_block_marks` 落库面——`extract_requirements` 合并时消费）。
    /// PR #227 只存 marks 不存 reqs（回显后丢弃）——本字段补上这一半。
    submitted_requirements: HashMap<String, Vec<coverage::SubmittedRequirement>>,
    /// #224 评分项 ↔ 章节多对多映射账本（SM-xxxx——AgentSubmitted/Manual
    /// 的持久化落点；RuleParsed 派生映射不入账。登记见
    /// [`DomainStore::upsert_scoring_mapping`]，派生联结见
    /// [`DomainStore::scoring_items`]）
    scoring_mappings: Vec<scoring::ScoringMapping>,
    /// #243 状态恢复审计账（VALIDATING → DRAFTING 的留底——追加式审计
    /// 日志，见 [`DomainStore::recover_validating_to_drafting`]）
    state_recoveries: Vec<StateRecoveryRecord>,
    /// #254 章节正文分段账本（SEG-xxxx——合稿 = 有序段拼接，正文单一真相
    /// 仍在 `OutlineNode.content`；账本与正文同进退的不变量与写入面见
    /// [`DomainStore::append_section_segment`]）
    segments: Vec<segments::SectionSegment>,
    /// 幂等键 → 首次结果 JSON（同 key 重放返回首次结果，不重复执行）
    idempotency: HashMap<String, serde_json::Value>,
    id_seq: u64,
}

impl DomainStore {
    pub fn new() -> Self {
        Self {
            project: None,
            files: vec![],
            requirements: vec![],
            evidences: vec![],
            evidence_bindings: vec![],
            sections: vec![],
            assignments: vec![],
            matrix: vec![],
            matrix_overrides: HashMap::new(),
            params: vec![],
            findings: vec![],
            reviews: vec![],
            revision_tasks: vec![],
            addendum_waivers: vec![],
            file_roles: vec![],
            block_marks: HashMap::new(),
            submitted_requirements: HashMap::new(),
            scoring_mappings: vec![],
            state_recoveries: vec![],
            segments: vec![],
            idempotency: HashMap::new(),
            id_seq: 0,
        }
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.id_seq += 1;
        let n = self.id_seq;
        format!("{prefix}-{n:04}")
    }

    pub fn project(&self) -> Result<&BidProject, ToolError> {
        self.project.as_ref().ok_or_else(|| ToolError::ENotFound {
            what: "project 未创建".into(),
        })
    }

    pub fn files(&self) -> &[TenderFile] {
        &self.files
    }

    /// #188：某文件的全部业务角色（登记序；空 = 只有初始身份——首次导入
    /// 的 TenderFile.kind/name，直接 upsert_file 造出的文件也属此类）。
    pub fn file_roles(&self, file_id: &str) -> Vec<&FileRole> {
        self.file_roles
            .iter()
            .filter(|r| r.file_id == file_id)
            .collect()
    }

    /// #188：登记一条业务角色（导入去重命中且身份不同时由服务层调）。
    ///
    /// 裁决序：① file_id 必须存在于 files（孤儿角色 = 凭空身份，拒）；
    /// ② 完全相同的 (kind, declared_name) 已在 → 幂等 no-op（返回既有角色，
    /// 不重复登记）。登记**不碰 content_version**（角色是文件的元数据面，
    /// 不是新内容；下游判定面如补遗就绪度是活算的，无脏可标）。
    pub fn register_file_role(
        &mut self,
        file_id: &str,
        kind: &str,
        declared_name: &str,
        at: u64,
    ) -> Result<FileRole, ToolError> {
        if !self.files.iter().any(|f| f.id == file_id) {
            return Err(ToolError::ENotFound {
                what: format!("file_id {file_id} 不存在（孤儿角色——凭空身份，拒登记）"),
            });
        }
        if let Some(r) = self.file_roles.iter().find(|r| {
            r.file_id == file_id && r.kind == kind && r.declared_name == declared_name
        }) {
            return Ok(r.clone()); // 幂等 no-op
        }
        let role = FileRole {
            file_id: file_id.into(),
            kind: kind.into(),
            declared_name: declared_name.into(),
            at,
        };
        self.file_roles.push(role.clone());
        Ok(role)
    }

    pub fn requirements(&self) -> &[Requirement] {
        &self.requirements
    }

    pub fn content_version(&self) -> u64 {
        self.project
            .as_ref()
            .map(|p| p.content_version)
            .unwrap_or(0)
    }

    /// 幂等键查询：命中 → 返回首次结果（调用方必须直接复用，不得再执行）。
    pub fn idempotent_hit(&self, key: &str) -> Option<&serde_json::Value> {
        self.idempotency.get(key)
    }

    pub fn record_idempotent(&mut self, key: String, result: serde_json::Value) {
        self.idempotency.insert(key, result);
    }

    // ------------------------------------------------------------ create_bid_project（契约 §1）
    pub fn create_project(
        &mut self,
        name: &str,
        code: &str,
        mode: &str,
        external_key: &str,
    ) -> Result<(String, serde_json::Value), ToolError> {
        if let Some(p) = &self.project {
            if p.external_key == external_key {
                // 契约：external_key 相同 → 静默幂等返回已有 project_id
                let r = serde_json::json!({ "project_id": p.id, "status": p.status.as_str(),
                                             "deduplicated": true });
                return Ok((p.id.clone(), r));
            }
            return Err(ToolError::EUnprocessable {
                reason: "store 已持有不同 external_key 的项目（M1 单项目存储）".into(),
            });
        }
        let id = self.next_id("PRJ");
        let proj = BidProject {
            id: id.clone(),
            name: name.into(),
            code: code.into(),
            mode: mode.into(),
            status: ProjectStatus::Draft,
            content_version: 1,
            external_key: external_key.into(),
        };
        let r = serde_json::json!({ "project_id": id, "status": "DRAFT" });
        self.project = Some(proj);
        Ok((id, r))
    }

    // ------------------------------------------------------------ 工具准入裁决（E_STATE 门）
    /// TC-11：状态机越级防护——attempted 工具不在当前状态允许清单 → E_STATE，零副作用。
    pub fn guard_tool(&self, tool: &str) -> Result<(), ToolError> {
        let p = self.project.as_ref().ok_or_else(|| ToolError::ENotFound {
            what: "project 未创建".into(),
        })?;
        if p.status.allowed_tools().contains(&tool) {
            Ok(())
        } else {
            Err(ToolError::EState {
                current: p.status,
                attempted: tool_name(tool),
            })
        }
    }

    /// 乐观并发检查（契约 §0：project_version 过期 → E_CONFLICT）。
    /// M1 工具调用方传 `None` 表示跳过检查（后端内部步骤）；外部调用必须传。
    pub fn check_version(&self, expected: Option<u64>) -> Result<u64, ToolError> {
        let p = self.project()?;
        let cur = p.content_version;
        if let Some(exp) = expected {
            if exp != cur {
                return Err(ToolError::EConflict {
                    expected: exp,
                    got: cur,
                });
            }
        }
        Ok(cur)
    }

    /// 项目状态正式推进（#59 阶段 1——MatrixReady 起的后段状态此前无推进
    /// 通道，测试靠 force_status 直改字段绕过状态机）。
    ///
    /// - `to` 必须在当前状态的 `next_allowed` 内，或等于当前状态（幂等 no-op
    ///   ——重放安全）；
    /// - 跳级/回退 → E_STATE（零副作用）；
    /// - 实际推进时 content_version + 1（与 upsert_file / set_requirements 的
    ///   「状态变更即版本变更」先例同口径）。
    pub fn advance_project_status(&mut self, to: ProjectStatus) -> Result<(), ToolError> {
        let cur = self.project()?.status;
        if cur == to {
            return Ok(()); // 幂等 no-op
        }
        if !cur.next_allowed().contains(&to) {
            return Err(ToolError::EState {
                current: cur,
                attempted: to.as_str(),
            });
        }
        if let Some(p) = self.project.as_mut() {
            p.status = to;
            p.content_version += 1;
        }
        Ok(())
    }

    // ------------------------------------------------------------ #243 校验状态安全与恢复
    /// #243：全稿「起草未完成」判定——未达 DRAFTED 的叶章节清单。
    ///
    /// 「已起草」口径与 validate 规则 3 同源（[`SectionStatus::is_drafted`]：
    /// Drafted/Validating/Revising/Accepted；Pending/Outlining/Drafting/Failed
    /// 都是未完成——Failed 是失败的一等结果，重试成功后才算完成）。
    /// 返回序 = 章节树文档序（确定性）。
    pub fn incomplete_leaf_ids(&self) -> Vec<String> {
        let parents: std::collections::HashSet<&str> = self
            .sections
            .iter()
            .filter_map(|n| n.parent_id.as_deref())
            .collect();
        self.sections
            .iter()
            .filter(|n| !parents.contains(&n.id[..]) && !n.status.is_drafted())
            .map(|n| n.id.clone())
            .collect()
    }

    /// #243 状态恢复审计账（追加式——查询面返回全部记录）。
    pub fn state_recoveries(&self) -> &[StateRecoveryRecord] {
        &self.state_recoveries
    }

    /// #243 受支持的状态恢复：VALIDATING → DRAFTING（唯一回退边，有审计）。
    ///
    /// 通道三重约束（不是任意状态写入）：
    /// 1. 仅 VALIDATING 态可恢复——前段态无死锁面；Reviewing+ 属审核/导出
    ///    阶段，语义不同，不在本通道；
    /// 2. 仅草稿未完成（存在未 DRAFTED 叶）可恢复——这正是历史缺陷制造的
    ///    异常态（校验误推进 + 起草被锁死）；草稿完整的 VALIDATING 是健康
    ///    态（校验已过），恢复通道拒之；
    /// 3. 理由必填非空白——每次恢复都落 [`StateRecoveryRecord`] 审计账
    ///    （何时/为何/从哪到哪/当时缺哪些章）。
    ///
    /// 效果：status → DRAFTING（起草面解锁）+ content_version + 1 + 审计
    /// 入账。**不动**章节树/矩阵/审核记录（恢复是状态面动作，内容面
    /// 原样——恢复前后的不变量由 #243 测试锁定）。拒绝路径零副作用。
    pub fn recover_validating_to_drafting(
        &mut self,
        reason: &str,
    ) -> Result<StateRecoveryRecord, ToolError> {
        let cur = self.project()?.status;
        if cur != ProjectStatus::Validating {
            return Err(ToolError::EState {
                current: cur,
                attempted: "recover_validating_to_drafting",
            });
        }
        let missing = self.incomplete_leaf_ids();
        if missing.is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "草稿已完整（全部叶达 DRAFTED）——当前 VALIDATING 是健康态（校验已过），不构成恢复条件（#243 恢复通道仅服务历史异常的 VALIDATING+缺章态）".into(),
            });
        }
        if reason.trim().is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "恢复理由不得为空白（审计面：不可审计的恢复不成恢复）".into(),
            });
        }
        let rec = StateRecoveryRecord {
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            from: ProjectStatus::Validating,
            to: ProjectStatus::Drafting,
            reason: reason.trim().to_string(),
            missing_sections: missing,
        };
        if let Some(p) = self.project.as_mut() {
            p.status = ProjectStatus::Drafting;
            p.content_version += 1; // 状态变更即版本变更（与 advance_project_status 同口径）
        }
        self.state_recoveries.push(rec.clone());
        Ok(rec)
    }

    // ------------------------------------------------------------ import_tender_files（契约 §2）
    /// 文件按 sha256 天然幂等：已存在 → no-op 返回原 file_id（deduplicated）。
    pub fn upsert_file(
        &mut self,
        name: &str,
        kind: &str,
        sha256: &str,
        pages: usize,
        tables: usize,
        chunks: usize,
        ocr_blocks: usize,
        parse_status: ParseStatus,
        parse_summary: &str,
    ) -> Result<(String, bool), ToolError> {
        if let Some(f) = self.files.iter().find(|f| f.sha256 == sha256) {
            return Ok((f.id.clone(), true)); // deduplicated: true
        }
        let id = self.next_id("TF");
        self.files.push(TenderFile {
            id: id.clone(),
            name: name.into(),
            kind: kind.into(),
            sha256: sha256.into(),
            pages,
            tables,
            chunks,
            ocr_blocks,
            parse_status,
            parse_summary: parse_summary.into(),
        });
        if let Some(p) = self.project.as_mut() {
            // 【issue #174】解析失败的文件**不推进**状态。
            //
            // 失败件仍会落库（有意设计：Failed + 禁引 R-01，便于重放与审计），
            // 但「落库」不等于「导入成功」。若失败件也把 Draft 推进到
            // FILES_IMPORTED，后续会认为「文件已导入」而允许 extract_requirements，
            // 可失败件是禁引的、抽不到内容——于是基于空内容一路走到导出，
            // **产出空标书**。这是最危险的形态（用户拿空文件去投标）。
            //
            // 语义：全部失败 → 停在 Draft（用户立刻知道没成）；
            //       部分成功 → 成功那件推进（合理）；正常文件 → 推进。
            if p.status == ProjectStatus::Draft && parse_status != ParseStatus::Failed {
                p.status = ProjectStatus::FilesImported;
            }
            p.content_version += 1; // 内容变化（追加文件属允许的脏追加，契约 §2）
        }
        Ok((id, false))
    }

    // ------------------------------------------------------------ #184 补遗逐件确认
    /// 补遗逐件确认记录（全量查询面——追加式审计日志，含历史确认）。
    pub fn addendum_waivers(&self) -> &[AddendumWaiverRecord] {
        &self.addendum_waivers
    }

    /// 某补遗件是否已有确认记录（readiness 判定的消费面：file_id 匹配即
    /// 该件已人工接受）。
    pub fn addendum_waived(&self, file_id: &str) -> bool {
        self.addendum_waivers.iter().any(|w| w.file_id == file_id)
    }

    /// 记录一条补遗逐件确认（#184——分级就绪的人工出口；#182 升级为
    /// TrustedPrincipal 验签面）。
    ///
    /// 裁决序（全部前置校验通过才落账——拒绝路径零副作用）：
    /// 1. project 在场（记录挂在项目下）；
    /// 2. `principal` 须 Human 身份（#182——服务层验签是 MCP 面的唯一入口，
    ///    域层再拦一次 actor_type 是纵深防御：即使壳层漏验，域层也不放
    ///    Agent/System 过「带病放行」的免责面）；
    /// 3. `reason` 非空白（不可审计的确认不成确认）；
    /// 4. `file_id` 必须存在于 files（对不存在的件做确认 = 凭空免责）。
    ///
    /// 落账：追加 `AddendumWaiverRecord`（同件重复确认追加不覆盖——审计
    /// 日志语义）+ content_version + 1（与 record_review 的「写账即版本」
    /// 同口径）。不设状态门：与 Gate::AddendumWaiver 同窗口语义——
    /// 导入后任意时刻可确认（阻塞可能出现在任何后续阶段）。
    pub fn record_addendum_waiver(
        &mut self,
        file_id: &str,
        reason: &str,
        principal: &TrustedPrincipal,
    ) -> Result<AddendumWaiverRecord, ToolError> {
        self.project()?;
        if principal.actor_type != ActorType::Human {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "确认人 {:?} 是 {} 身份（#182：确认面只认 Human——Agent/System 不得自审自过，「带病放行」的免责面没有理由放宽）",
                    principal.actor_id,
                    principal.actor_type.as_str()
                ),
            });
        }
        if reason.trim().is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "确认理由不得为空白（不可审计的确认不成确认）".into(),
            });
        }
        if !self.files.iter().any(|f| f.id == file_id) {
            return Err(ToolError::ENotFound {
                what: format!("文件 {file_id} 不存在（对不存在的件做确认 = 凭空免责）"),
            });
        }
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let rec = AddendumWaiverRecord {
            file_id: file_id.to_string(),
            reason: reason.trim().to_string(),
            at,
            // #182：legacy 回显面从可信身份派生（human:{actor_id}）——
            // 不另造第二套人设体系
            principal: format!("human:{}", principal.actor_id),
            trusted: Some(principal.clone()),
        };
        self.addendum_waivers.push(rec.clone());
        // 【接线修复】豁免记录必须同时写入 Gate 审计面——
        // `readiness::missing_addendum` 判定的是 `gate_approved(AddendumWaiver)`，
        // 而本函数此前只 push `addendum_waivers`，两边不通：
        // 用户记录了「本项目无补遗」确认，起草门仍报「未发现补遗/澄清文件」死锁。
        //
        // 实测复现（2026-11-02，无补遗件的真实招标文件）：
        //   record_addendum_waiver → ok:true，project_version 6
        //   draft_response_section → E_UNPROCESSABLE「未发现补遗/澄清文件」
        //
        // 两道记录职责不同，不能只写一处：
        //   · addendum_waivers = 逐件免责台账（哪些件、谁确认、为什么）
        //   · reviews(AddendumWaiver, Approved) = 门禁消费的放行事实
        // 与 record_review 同款：内容绑定快照（#180）——补遗确认针对的是当前章节树
        let review_id = self.next_id("REV");
        let reviewed_content_version = self.project.as_ref().map_or(0, |p| p.content_version);
        let reviewed_content_digest = self.sections_digest();
        self.reviews.push(review::ReviewRecord {
            id: review_id,
            gate: review::Gate::AddendumWaiver,
            // legacy 回显面同款派生（record_review 口径——human:{actor_id}）
            reviewer: format!("{}{}", review::HUMAN_PREFIX, principal.actor_id),
            principal: Some(principal.clone()),
            decision: review::Decision::Approved,
            findings: Vec::new(),
            at,
            reviewed_content_version,
            reviewed_content_digest,
        });
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(rec)
    }

    // ---------------------------------------------------------------- #219 全块覆盖

    /// 写入某文件的区间标记（#219）。
    ///
    /// 校验：区间合法 + 不越界 + 不重叠。通过后整体替换该文件的标记集。
    pub fn set_block_marks(
        &mut self,
        file_id: &str,
        marks: Vec<coverage::BlockMark>,
        total_blocks: usize,
    ) -> Result<(), ToolError> {
        if !self.files.iter().any(|f| f.id == file_id) {
            return Err(ToolError::ENotFound {
                what: format!("文件 {file_id} 不存在"),
            });
        }
        coverage::validate_marks(&marks, total_blocks)
            .map_err(|e| ToolError::EUnprocessable { reason: e.to_message() })?;
        self.block_marks.insert(file_id.to_string(), marks);
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }

    /// 读某文件的区间标记。
    pub fn block_marks_of(&self, file_id: &str) -> &[coverage::BlockMark] {
        self.block_marks.get(file_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// #219 二步：写入某文件的区间标记 + Agent 关键点（原子——两项同验同写）。
    ///
    /// 合并 `set_block_marks` 与 submitted 落库：PR #227 的工具层只调
    /// `set_block_marks`（reqs 校验后回显即丢，且不经服务层无 checkpoint）。
    /// 二步起 reqs 是合并面的输入，必须持久化。
    pub fn set_block_coverage(
        &mut self,
        file_id: &str,
        marks: Vec<coverage::BlockMark>,
        reqs: Vec<coverage::SubmittedRequirement>,
        total_blocks: usize,
    ) -> Result<(), ToolError> {
        if !self.files.iter().any(|f| f.id == file_id) {
            return Err(ToolError::ENotFound {
                what: format!("文件 {file_id} 不存在"),
            });
        }
        coverage::validate_marks(&marks, total_blocks)
            .map_err(|e| ToolError::EUnprocessable { reason: e.to_message() })?;
        coverage::validate_submitted(&marks, &reqs)
            .map_err(|e| ToolError::EUnprocessable { reason: e.to_message() })?;
        self.block_marks.insert(file_id.to_string(), marks);
        self.submitted_requirements.insert(file_id.to_string(), reqs);
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }

    /// #219 二步：读某文件的 Agent 关键点。
    pub fn submitted_requirements_of(
        &self,
        file_id: &str,
    ) -> &[coverage::SubmittedRequirement] {
        self.submitted_requirements
            .get(file_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// 覆盖报告（纯派生）。
    pub fn coverage_report(&self, file_id: &str, total_blocks: usize) -> coverage::CoverageReport {
        coverage::compute_coverage(self.block_marks_of(file_id), total_blocks)
    }

    // ------------------------------------------------------------ extract_requirements（契约 §3）
    /// 写入条款集（dedupe_hash 幂等合并：同源锚+同陈述 → 复用既有 REQ）。
    pub fn set_requirements(
        &mut self,
        reqs: Vec<Requirement>,
    ) -> Result<(usize, usize), ToolError> {
        let p = self.project.as_ref().ok_or_else(|| ToolError::ENotFound {
            what: "project 未创建".into(),
        })?;
        if p.status != ProjectStatus::FilesImported
            && p.status != ProjectStatus::RequirementsExtracted
        {
            return Err(ToolError::EState {
                current: p.status,
                attempted: "extract_requirements",
            });
        }
        // 幂等合并：dedupe_hash = sha256(排序后 (file, locator, excerpt, statement_norm))
        //
        // 性能治本（PR #24 修复批次——CI「12 分钟卡死」根因）：旧实现
        // `merged.iter().position(|x| requirement_hash(x) == h)` 对每条新 REQ
        // 线性扫描已有集且**逐个重算 sha256**——N 条全新件 = O(N²) 次哈希
        // （12,000 条 ≈ 7,200 万次，Linux 实测 488s；Windows CI 外推 1.5-2x
        // ≈ 12-16 分钟，即 Lead 观测的「cargo test 卡死」）。治本：已有集与
        // 新增项各只算一次哈希，HashMap<hash, idx> O(1) 查表——O(N²) → O(N)。
        // 语义不变：同 hash 取**首个**席位（与 position 首个命中一致），
        // 重跑覆盖同位、新件追加——dedupe 语义零变化，仅复杂度治本。
        let mut merged: Vec<Requirement> = self.requirements.clone();
        let before = merged.len();
        let mut index: HashMap<String, usize> = HashMap::with_capacity(merged.len());
        for (i, r) in merged.iter().enumerate() {
            index.entry(requirement_hash(r)).or_insert(i);
        }
        for r in reqs {
            let h = requirement_hash(&r);
            match index.get(&h) {
                Some(&i) => merged[i] = r, // 重跑覆盖（同 hash → 稳定）
                None => {
                    index.insert(h, merged.len());
                    merged.push(r);
                }
            }
        }
        let added = merged.len() - before;
        // #224 引用完整性（Lead 2026-11-04 review blocker 2）：评分映射引用的
        // 评分项不得随合并消失——同哈希覆盖（重跑换 id）同样会移走被映射的
        // REQ id。拒绝破坏性写入（fail-loud）：映射账本是审计面，不得静默
        // 蒸发，也不得留孤儿映射（孤儿态一经 checkpoint 落盘，下一次
        // restore 拒载——快照卡死）。
        if !self.scoring_mappings.is_empty() {
            let known: std::collections::HashSet<&str> =
                merged.iter().map(|r| r.id.as_str()).collect();
            if let Some(m) = self
                .scoring_mappings
                .iter()
                .find(|m| !known.contains(m.scoring_req_id.as_str()))
            {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "评分映射 {} 引用的评分项 {} 将在本次写入后不存在（#224 引用完整性：条款重建不得孤儿化映射账本——先处理该映射再重建）",
                        m.id, m.scoring_req_id
                    ),
                });
            }
        }
        self.requirements = merged;
        let p = self.project.as_mut().unwrap();
        if p.status == ProjectStatus::FilesImported {
            p.status = ProjectStatus::RequirementsExtracted;
        }
        p.content_version += 1;
        Ok((self.requirements.len(), added))
    }

    /// #219 二步：整集替换条款集（「Agent 优先 + 正则兜底」合并的落库面）。
    ///
    /// 与 [`DomainStore::set_requirements`] 的差异：
    /// - set：dedupe_hash 幂等合并、**只增不删**——重跑覆盖同位、新件追加；
    /// - replace：整集替换——场景「先 extract（无标记）→ Agent 标 noise →
    ///   重跑 extract」时，被覆盖块上的旧正则 REQ 必须从库里消失
    ///   （Agent 优先），只增不删的合并语义做不到。
    ///
    /// 抽取窗口内整集替换是安全的：大纲/矩阵/证据绑定/矩阵覆写只存在于
    /// MATRIX_READY+，而 extract 在该态被 E_STATE 拦死——不存在孤儿引用面。
    /// （#224 修订：评分映射无状态门、可在抽取窗口内入账，故下方加映射
    /// 引用完整性守卫——被映射的 REQ id 将被移走时拒绝替换，Lead
    /// 2026-11-04 review blocker 2。）
    pub fn replace_requirements(&mut self, reqs: Vec<Requirement>) -> Result<usize, ToolError> {
        let p = self.project.as_ref().ok_or_else(|| ToolError::ENotFound {
            what: "project 未创建".into(),
        })?;
        if p.status != ProjectStatus::FilesImported
            && p.status != ProjectStatus::RequirementsExtracted
        {
            return Err(ToolError::EState {
                current: p.status,
                attempted: "extract_requirements",
            });
        }
        // id 唯一性（fail-loud——合并面编号由 merge_extraction 保证唯一，
        // 这里拦手造件/未来回归）
        let mut seen = std::collections::HashSet::new();
        if let Some(dup) = reqs.iter().find(|r| !seen.insert(r.id.as_str())) {
            return Err(ToolError::EUnprocessable {
                reason: format!("条款 id 重复：{}（合并面编号必须唯一）", dup.id),
            });
        }
        // #224 引用完整性（Lead 2026-11-04 review blocker 2）：整集替换更
        // 容易移走被映射的 REQ id——同 set_requirements 守卫，拒破坏性替换。
        if !self.scoring_mappings.is_empty() {
            let known: std::collections::HashSet<&str> =
                reqs.iter().map(|r| r.id.as_str()).collect();
            if let Some(m) = self
                .scoring_mappings
                .iter()
                .find(|m| !known.contains(m.scoring_req_id.as_str()))
            {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "评分映射 {} 引用的评分项 {} 将在整集替换后不存在（#224 引用完整性：条款重建不得孤儿化映射账本——先处理该映射再替换）",
                        m.id, m.scoring_req_id
                    ),
                });
            }
        }
        self.requirements = reqs;
        let p = self.project.as_mut().unwrap();
        if p.status == ProjectStatus::FilesImported {
            p.status = ProjectStatus::RequirementsExtracted;
        }
        p.content_version += 1;
        Ok(self.requirements.len())
    }
}

impl Default for DomainStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- T-8 持久化快照（纯函数面）
/// 快照 schema 版本（加性演进：新字段一律 `#[serde(default)]`，旧数据可加载；
/// 结构性变更 → 版本号 +1 + `migrate`（bid-tools store_io）——参考 mdcore sidecar）。
pub const STORE_SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    STORE_SCHEMA_VERSION
}

/// 领域库快照（T-8）——`DomainStore` 全量状态的 serde DTO。
///
/// - 全字段 `#[serde(default)]`：旧版快照缺新集合字段 → 空集合加载（加性迁移）；
/// - `content_version` 住在 `BidProject` 内随快照走；
/// - 语义完整性由 [`DomainStore::restore`] 把关（id 唯一 / 序列水位 / 孤儿数据），
///   JSON 语法与版本门在 bid-tools `store_io`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreSnapshot {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub project: Option<BidProject>,
    #[serde(default)]
    pub files: Vec<TenderFile>,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub evidences: Vec<evidence::Evidence>,
    #[serde(default)]
    pub evidence_bindings: Vec<evidence::EvidenceBinding>,
    #[serde(default)]
    pub sections: Vec<outline::OutlineNode>,
    #[serde(default)]
    pub assignments: Vec<assignment::Assignment>,
    #[serde(default)]
    pub matrix: Vec<matrix::MatrixRow>,
    #[serde(default)]
    pub matrix_overrides: HashMap<String, matrix::MatrixOverride>,
    /// T-4 参数响应表（加性迁移——V3-3 之前的旧快照无此字段 → 空集合加载）
    #[serde(default)]
    pub params: Vec<param::ParamResponse>,
    #[serde(default)]
    pub findings: Vec<review::Finding>,
    #[serde(default)]
    pub reviews: Vec<review::ReviewRecord>,
    #[serde(default)]
    pub revision_tasks: Vec<review::RevisionTask>,
    /// #184 补遗逐件人工确认（加性迁移——旧快照无此字段 → 空集合加载；
    /// schema_version 不 bump，见 StoreSnapshot 头注释的加性演进纪律）
    #[serde(default)]
    pub addendum_waivers: Vec<AddendumWaiverRecord>,
    /// #188 文件业务角色（加性迁移——旧快照无此字段 → 空集合加载：
    /// 角色为空时初始身份回退到 TenderFile.kind/name，行为与旧版一致）
    #[serde(default)]
    pub file_roles: Vec<FileRole>,
    /// #219 全块覆盖标记（加性迁移——旧快照无此字段 → 空映射加载）
    #[serde(default)]
    pub block_marks: BTreeMap<String, Vec<coverage::BlockMark>>,
    /// #219 二步：Agent 提交的关键点（加性迁移——旧快照无此字段 →
    /// 空映射加载；PR #227 时代的快照无 submitted 面，合并时视为未提交）
    #[serde(default)]
    pub submitted_requirements: BTreeMap<String, Vec<coverage::SubmittedRequirement>>,
    /// #224 评分项 ↔ 章节多对多映射（加性迁移——旧快照无此字段 → 空集合
    /// 加载：无持久化映射时评分项仅带 params.section 派生映射，行为与
    /// 旧版一致；schema_version 不 bump，见头注释加性演进纪律）
    #[serde(default)]
    pub scoring_mappings: Vec<scoring::ScoringMapping>,
    /// #243 状态恢复审计账（加性迁移——旧快照无此字段 → 空集合加载；
    /// schema_version 不 bump，见头注释的加性演进纪律）
    #[serde(default)]
    pub state_recoveries: Vec<StateRecoveryRecord>,
    /// #254 章节正文分段（加性迁移——旧快照无此字段 → 空集合加载：
    /// 无分段账本时正文仍以 `OutlineNode.content` 为真相，行为与旧版一致）
    #[serde(default)]
    pub segments: Vec<segments::SectionSegment>,
    #[serde(default)]
    pub idempotency: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub id_seq: u64,
}

impl DomainStore {
    /// 全量快照（纯读——checkpoint 的素材面）。
    pub fn to_snapshot(&self) -> StoreSnapshot {
        StoreSnapshot {
            schema_version: STORE_SCHEMA_VERSION,
            project: self.project.clone(),
            files: self.files.clone(),
            requirements: self.requirements.clone(),
            evidences: self.evidences.clone(),
            evidence_bindings: self.evidence_bindings.clone(),
            sections: self.sections.clone(),
            assignments: self.assignments.clone(),
            matrix: self.matrix.clone(),
            matrix_overrides: self.matrix_overrides.clone(),
            params: self.params.clone(),
            findings: self.findings.clone(),
            reviews: self.reviews.clone(),
            revision_tasks: self.revision_tasks.clone(),
            addendum_waivers: self.addendum_waivers.clone(),
            file_roles: self.file_roles.clone(),
            // HashMap → BTreeMap（与 idempotency 同纪律：持久化 DTO 要字节一致）
            block_marks: self.block_marks.clone().into_iter().collect(),
            submitted_requirements: self
                .submitted_requirements
                .clone()
                .into_iter()
                .collect(),
            scoring_mappings: self.scoring_mappings.clone(),
            state_recoveries: self.state_recoveries.clone(),
            segments: self.segments.clone(),
            // HashMap → BTreeMap：持久化 DTO 要求跨进程字节一致（HashMap 迭代序不确定）
            idempotency: self.idempotency.clone().into_iter().collect(),
            id_seq: self.id_seq,
        }
    }

    /// 从快照恢复（重启续跑的领域半边）。
    ///
    /// 语义完整性把关（损坏但语法合法的 JSON 也拒——fail-closed 不猜测）：
    /// - schema_version 必须等于当前版本（旧版本由 IO 层 migrate 后再进这里）；
    /// - 各集合 id 唯一；
    /// - `id_seq` 水位 ≥ 全体 id 的数字后缀（防重启后 id 重签碰撞）；
    /// - T-7 引用完整：RT 的 finding_id 与 ReviewRecord 引用的 FND-id 必须
    ///   存在于 findings（孤儿 RT / 幽灵 FND 引用 = 半个闭环，拒载）；
    /// - 有数据必有项目（孤儿集合 = 半个库）。
    pub fn restore(snap: StoreSnapshot) -> Result<Self, ToolError> {
        if snap.schema_version != STORE_SCHEMA_VERSION {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "快照 schema_version={} ≠ 当前支持 {}（migrate 先行——bid-tools store_io）",
                    snap.schema_version, STORE_SCHEMA_VERSION
                ),
            });
        }
        let un = |reason: String| ToolError::EUnprocessable { reason };
        // id 唯一性（逐集合）
        let dup_in = |ids: Vec<String>, what: &str| -> Option<String> {
            let mut seen = std::collections::HashSet::new();
            ids.into_iter()
                .find_map(|i| (!seen.insert(i)).then(|| format!("{what} id 重复")))
        };
        if let Some(e) = dup_in(snap.files.iter().map(|f| f.id.clone()).collect(), "files")
            .or_else(|| {
                dup_in(
                    snap.requirements.iter().map(|r| r.id.clone()).collect(),
                    "requirements",
                )
            })
            .or_else(|| {
                dup_in(
                    snap.evidences.iter().map(|e| e.id.clone()).collect(),
                    "evidences",
                )
            })
            .or_else(|| {
                dup_in(
                    snap.sections.iter().map(|s| s.id.clone()).collect(),
                    "sections",
                )
            })
            .or_else(|| dup_in(snap.params.iter().map(|p| p.id.clone()).collect(), "params"))
            .or_else(|| {
                dup_in(
                    snap.findings.iter().map(|f| f.id.clone()).collect(),
                    "findings",
                )
            })
            .or_else(|| {
                dup_in(
                    snap.reviews.iter().map(|r| r.id.clone()).collect(),
                    "reviews",
                )
            })
            .or_else(|| {
                dup_in(
                    snap.revision_tasks.iter().map(|t| t.id.clone()).collect(),
                    "revision_tasks",
                )
            })
            .or_else(|| {
                dup_in(
                    snap.scoring_mappings.iter().map(|m| m.id.clone()).collect(),
                    "scoring_mappings",
                )
            })
        {
            return Err(un(e));
        }
        // assignments 的唯一键是 target_id（T-6 本实体无独立 id——幂等 upsert 的
        // 自然键，见 assignment.rs 模块头），消息必须指名真键而非笼统的「id 重复」。
        {
            let mut seen = std::collections::HashSet::new();
            if let Some(a) = snap
                .assignments
                .iter()
                .find(|a| !seen.insert(a.target_id.clone()))
            {
                return Err(un(format!("assignments target_id 重复: {}", a.target_id)));
            }
        }
        // id_seq 水位（防重启后重签碰撞——续跑的第一件事就是签新 id）。
        // 注意：只校验 **store 签发**的命名空间（PRJ/TF/EV/SEC/PR——PR 为 T-4
        // 参数行前缀）——REQ id 由 extract 层自有计数器签发（每次抽取重置，
        // 与 store 序列无关），不在本水位管辖面。
        // 注意：只校验 **store 签发**的命名空间（PRJ/TF/EV/SEC）——REQ id 由
        // extract 层自有计数器签发（每次抽取重置，与 store 序列无关），
        // 不在本水位管辖面。
        // assignments 同样**不入水位数组**：本实体无自签 id（target_id 引用外部
        // 实体——SEC- 目标经幽灵门必存在于 sections，其后缀已被上面的 sections
        // 水位覆盖；REQ-/其他前缀本就豁免）——加入只重复计算，不会抬高水位。
        // 注意：只校验 **store 签发**的命名空间（PRJ/TF/EV/SEC——V3-4 扩
        // FND/REV/RT 三前缀）——REQ id 由 extract 层自有计数器签发（每次
        // 抽取重置，与 store 序列无关），CMP id 为 REQ 后缀镜像派生（非
        // store 序列），均不在本水位管辖面。
        let watermark = [
            max_suffix(snap.files.iter().map(|f| f.id.clone())),
            max_suffix(snap.evidences.iter().map(|e| e.id.clone())),
            max_suffix(snap.sections.iter().map(|s| s.id.clone())),
            max_suffix(snap.params.iter().map(|p| p.id.clone())),
            max_suffix(snap.findings.iter().map(|f| f.id.clone())),
            max_suffix(snap.reviews.iter().map(|r| r.id.clone())),
            max_suffix(snap.revision_tasks.iter().map(|t| t.id.clone())),
            // #224 SM-xxxx 入水位（store 签发命名空间）
            max_suffix(snap.scoring_mappings.iter().map(|m| m.id.clone())),
            // #254 SEG-xxxx 入水位（store 签发命名空间——防重启后重签碰撞）
            max_suffix(snap.segments.iter().map(|s| s.id.clone())),
            snap.project
                .as_ref()
                .map(|p| max_suffix([p.id.clone()].into_iter()))
                .unwrap_or(0),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        if snap.id_seq < watermark {
            return Err(un(format!(
                "id_seq={} 低于 id 水位 {}（重启后新 id 会与既有记录碰撞——快照被截断？",
                snap.id_seq, watermark
            )));
        }
        // T-7 引用完整性（半个闭环拒载——fail-closed 不猜测）：
        // RT 的 finding_id 必须存在（孤儿 RT）；ReviewRecord 引用的 FND 必须存在。
        let fnd_ids: std::collections::HashSet<&str> =
            snap.findings.iter().map(|f| f.id.as_str()).collect();
        if let Some(rt) = snap
            .revision_tasks
            .iter()
            .find(|t| !fnd_ids.contains(t.finding_id.as_str()))
        {
            return Err(un(format!(
                "revision_task {} 引用了不存在的 finding {}（孤儿 RT——半个闭环，拒载）",
                rt.id, rt.finding_id
            )));
        }
        if let Some(rev) = snap
            .reviews
            .iter()
            .find(|r| r.findings.iter().any(|f| !fnd_ids.contains(f.as_str())))
        {
            return Err(un(format!(
                "review {} 引用了不存在的 finding（幽灵 FND 引用——拒载）",
                rev.id
            )));
        }
        // #184 补遗确认引用完整：waiver 的 file_id 必须存在于 files
        // （孤儿确认 = 对不存在的件免责——半个账本，拒载）。
        let file_ids: std::collections::HashSet<&str> =
            snap.files.iter().map(|f| f.id.as_str()).collect();
        if let Some(w) = snap
            .addendum_waivers
            .iter()
            .find(|w| !file_ids.contains(w.file_id.as_str()))
        {
            return Err(un(format!(
                "addendum_waiver 引用了不存在的文件 {}（孤儿确认——拒载）",
                w.file_id
            )));
        }
        // #188 角色引用完整：角色的 file_id 必须存在于 files（孤儿角色 =
        // 对不存在的件声明身份——半个账本，拒载）。
        if let Some(r) = snap
            .file_roles
            .iter()
            .find(|r| !file_ids.contains(r.file_id.as_str()))
        {
            return Err(un(format!(
                "file_role 引用了不存在的文件 {}（孤儿角色——拒载）",
                r.file_id
            )));
        }
        // #224 映射引用完整：scoring_req_id 必须在 requirements，section_id
        // 必须在 sections（孤儿映射 = 半个账本，拒载）；同 (req, section)
        // 复合键重复 = 幂等面破坏，拒载。
        let req_ids: std::collections::HashSet<&str> =
            snap.requirements.iter().map(|r| r.id.as_str()).collect();
        let sec_ids: std::collections::HashSet<&str> =
            snap.sections.iter().map(|s| s.id.as_str()).collect();
        if let Some(m) = snap.scoring_mappings.iter().find(|m| {
            !req_ids.contains(m.scoring_req_id.as_str())
                || !sec_ids.contains(m.section_id.as_str())
        }) {
            return Err(un(format!(
                "scoring_mapping {} 引用了不存在的评分项/章节（{} / {}——孤儿映射，拒载）",
                m.id, m.scoring_req_id, m.section_id
            )));
        }
        {
            let mut seen: std::collections::HashSet<(String, String)> =
                std::collections::HashSet::new();
            if let Some(m) = snap
                .scoring_mappings
                .iter()
                .find(|m| !seen.insert((m.scoring_req_id.clone(), m.section_id.clone())))
            {
                return Err(un(format!(
                    "scoring_mapping 复合键重复：({}, {})——幂等面破坏，拒载",
                    m.scoring_req_id, m.section_id
                )));
            }
        }
        // #224 行约束与登记面同源（Lead 2026-11-04 review blocker 2）：
        // restore 不得比 upsert 宽——份额越界（权重语义破坏）与幽灵证据
        // 引用（半个账本）在快照面同样拒载。
        if let Some(m) = snap
            .scoring_mappings
            .iter()
            .find(|m| !(0.0 < m.weight_share && m.weight_share <= 1.0))
        {
            return Err(un(format!(
                "scoring_mapping {} weight_share={} 越界（合法域 (0,1]——与登记面同约束，拒载）",
                m.id, m.weight_share
            )));
        }
        let ev_ids: std::collections::HashSet<&str> =
            snap.evidences.iter().map(|e| e.id.as_str()).collect();
        if let Some((m, ev)) = snap.scoring_mappings.iter().find_map(|m| {
            m.evidence_refs
                .iter()
                .find(|e| !ev_ids.contains(e.as_str()))
                .map(|ev| (m, ev))
        }) {
            return Err(un(format!(
                "scoring_mapping {} 引用了不存在的证据 {ev}（幽灵证据引用——与登记面同约束，拒载）",
                m.id
            )));
        }
        // #224 聚合不变量（Lead 2026-11-04 二次阻塞审查）：行约束挡不住
        // 「每行合法、同一评分项合计 1.4」的快照——restore 是登记面之外的
        // 第二条写入口（手改/旧版本落盘），恢复即账本态，scoring_items
        // 随即消费（跨章评分超单项满分）。口径与 upsert 登记面同源：
        // - 派生 params.section 单值映射（share=1.0）计入；
        // - 与派生同节的账本行联结去重（派生赢、不生效）→ 不计入；
        // - epsilon 1e-9 与登记面一致（0.1+0.2+0.7 浮点和放行）。
        {
            use std::collections::HashMap;
            // 被映射的评分项 → 派生 params.section（孤儿检查已保证 REQ 在场；
            // 无 params.section → None）
            let mut derived: HashMap<&str, Option<&str>> = HashMap::new();
            for m in &snap.scoring_mappings {
                derived
                    .entry(m.scoring_req_id.as_str())
                    .or_insert_with(|| {
                        snap.requirements
                            .iter()
                            .find(|r| r.id == m.scoring_req_id)
                            .and_then(|r| r.params.get("section"))
                            .and_then(|v| v.as_str())
                    });
            }
            // 联结面有效份额：异节账本行求和（与派生同节 = 去重不计）
            let mut sums: HashMap<&str, f64> = HashMap::new();
            for m in &snap.scoring_mappings {
                let ds = derived.get(m.scoring_req_id.as_str()).copied().flatten();
                if Some(m.section_id.as_str()) == ds {
                    continue;
                }
                *sums.entry(m.scoring_req_id.as_str()).or_insert(0.0) += m.weight_share;
            }
            for (rid, ds) in &derived {
                let mut total = *sums.get(*rid).unwrap_or(&0.0);
                if ds.is_some() {
                    total += 1.0;
                }
                if total > 1.0 + 1e-9 {
                    return Err(un(format!(
                        "评分项 {rid} 的映射份额合计 {total:.3} 超过 1.0（#224 聚合不变量——与登记面同约束，拒载；派生 params.section 单值映射 share=1.0 计入，与派生同节的账本行联结去重不计入）"
                    )));
                }
            }
        }
        // 孤儿数据（有集合无项目 = 半个库）
        if snap.project.is_none()
            && (!snap.files.is_empty()
                || !snap.requirements.is_empty()
                || !snap.evidences.is_empty()
                || !snap.sections.is_empty()
                || !snap.assignments.is_empty()
                || !snap.findings.is_empty()
                || !snap.reviews.is_empty()
                || !snap.revision_tasks.is_empty()
                || !snap.addendum_waivers.is_empty()
                || !snap.file_roles.is_empty()
                || !snap.scoring_mappings.is_empty()
                || !snap.state_recoveries.is_empty()
                || !snap.segments.is_empty())
        {
            return Err(un("快照有数据但无项目（孤儿集合——拒绝加载半个库）".into()));
        }
        // #254 分段账本引用完整性：id 唯一；section_id 必须存在于章节树
        // （孤儿段 = 凭空正文，拒）；同一章节内 seq 唯一（合稿顺序的确定性）。
        if let Some(e) = dup_in(snap.segments.iter().map(|s| s.id.clone()).collect(), "segments") {
            return Err(un(e));
        }
        if let Some(bad) = snap.segments.iter().find(|s| {
            !snap.sections.iter().any(|n| n.id == s.section_id)
        }) {
            return Err(un(format!(
                "segment {} 引用的章节 {} 不存在（孤儿段——半个账本，拒载）",
                bad.id, bad.section_id
            )));
        }
        {
            let mut seen: std::collections::HashSet<(String, u32)> =
                std::collections::HashSet::new();
            if let Some(bad) = snap.segments.iter().find(|s| {
                !seen.insert((s.section_id.clone(), s.seq))
            }) {
                return Err(un(format!(
                    "章节 {} 内段序号 {} 重复（segment {}——合稿顺序不确定，拒载）",
                    bad.section_id, bad.seq, bad.id
                )));
            }
        }
        // #254 幂等面：同章节内 idem_key 重复 = 重放将命中错误段（幂等键
        // 唯一性是 append 语义的地基），拒载。
        {
            let mut seen: std::collections::HashSet<(String, String)> =
                std::collections::HashSet::new();
            if let Some(bad) = snap.segments.iter().find(|s| {
                !seen.insert((s.section_id.clone(), s.idem_key.clone()))
            }) {
                return Err(un(format!(
                    "章节 {} 内段幂等键 {} 重复（segment {}——幂等面破坏，拒载）",
                    bad.section_id, bad.idem_key, bad.id
                )));
            }
        }
        Ok(Self {
            project: snap.project,
            files: snap.files,
            requirements: snap.requirements,
            evidences: snap.evidences,
            evidence_bindings: snap.evidence_bindings,
            sections: snap.sections,
            assignments: snap.assignments,
            matrix: snap.matrix,
            matrix_overrides: snap.matrix_overrides,
            params: snap.params,
            findings: snap.findings,
            reviews: snap.reviews,
            revision_tasks: snap.revision_tasks,
            addendum_waivers: snap.addendum_waivers,
            file_roles: snap.file_roles,
            block_marks: snap.block_marks.into_iter().collect(),
            submitted_requirements: snap.submitted_requirements.into_iter().collect(),
            scoring_mappings: snap.scoring_mappings,
            state_recoveries: snap.state_recoveries,
            segments: snap.segments,
            idempotency: snap.idempotency.into_iter().collect(),
            id_seq: snap.id_seq,
        })
    }
}

/// id 数字后缀最大值（水位校验：`TF-0007` → 7；非数字尾 → 忽略）。
fn max_suffix(ids: impl Iterator<Item = String>) -> u64 {
    ids.filter_map(|i| i.rsplit('-').next().and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0)
}

fn tool_name(tool: &str) -> &'static str {
    match tool {
        "create_bid_project" => "create_bid_project",
        "import_tender_files" => "import_tender_files",
        "extract_requirements" => "extract_requirements",
        "search_evidence" => "search_evidence",
        "get_bid_project_state" => "get_bid_project_state",
        "draft_response_section" => "draft_response_section",
        "validate_bid_document" => "validate_bid_document",
        "export_bid_document" => "export_bid_document",
        "build_outline" => "build_outline",
        "register_evidence" => "register_evidence",
        "bind_evidence" => "bind_evidence",
        "record_review_gate" => "record_review_gate",
        "record_addendum_waiver" => "record_addendum_waiver",
        "recover_validating_state" => "recover_validating_state",
        _ => "unknown_tool",
    }
}

pub fn requirement_hash(r: &Requirement) -> String {
    let mut parts: Vec<String> = r
        .anchors
        .iter()
        .map(|a| format!("{}|{}|{}", a.file, a.locator, a.excerpt))
        .collect();
    parts.sort();
    let stmt = r
        .anchors
        .iter()
        .map(|a| a.excerpt.as_str())
        .collect::<Vec<_>>()
        .join("");
    let mut hasher = Sha256::new();
    hasher.update(parts.join("\n"));
    hasher.update(&stmt);
    hex::encode(hasher.finalize())[..24].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_files() -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("测试项目", "ZFCG-2026-GK-0311", "明标", "ext-key-1")
            .unwrap();
        s.upsert_file(
            "A-招标文件.docx",
            "招标文件",
            "aa",
            31,
            12,
            120,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
        s
    }

    #[test]
    fn tc11_state_guard_rejects_skip_ahead_with_no_side_effect() {
        // TC-11：FILES_IMPORTED 阶段直接调 draft_response_section → E_STATE
        let s = store_with_files();
        let err = s.guard_tool("draft_response_section").unwrap_err();
        assert_eq!(err.code(), "E_STATE");
        // 无副作用：项目状态不变、无章节对象（M1 无章节面——以状态+文件数为准）
        assert_eq!(s.project().unwrap().status, ProjectStatus::FilesImported);
        assert_eq!(s.files().len(), 1);
    }

    #[test]
    fn state_sequence_forward_only() {
        let mut s = DomainStore::new();
        s.create_project("p", "c", "明标", "k").unwrap();
        assert_eq!(s.project().unwrap().status, ProjectStatus::Draft);
        s.upsert_file("f1", "招标文件", "sha1", 1, 0, 5, 0, ParseStatus::Ok, "")
            .unwrap();
        assert_eq!(s.project().unwrap().status, ProjectStatus::FilesImported);
        // Draft 之后唯一前向是 FilesImported；FilesImported 的工具面不含 draft_response_section
        assert!(ProjectStatus::FilesImported
            .next_allowed()
            .contains(&ProjectStatus::RequirementsExtracted));
    }

    #[test]
    fn create_project_idempotent_by_external_key() {
        let mut s = DomainStore::new();
        let (id1, r1) = s.create_project("p", "c", "明标", "key-A").unwrap();
        let (id2, r2) = s.create_project("p2", "c2", "暗标", "key-A").unwrap();
        assert_eq!(id1, id2, "同 external_key 必须返回原 project_id");
        assert_eq!(r2["deduplicated"], true);
        assert_eq!(r1["status"], "DRAFT");
    }

    #[test]
    fn tc07_import_same_sha_is_noop() {
        let mut s = store_with_files();
        let before_files = s.files().len();
        let before_version = s.content_version();
        let (id2, dup) = s
            .upsert_file(
                "A-招标文件.docx",
                "招标文件",
                "aa",
                31,
                12,
                120,
                0,
                ParseStatus::Ok,
                "",
            )
            .unwrap();
        assert!(dup, "同 sha256 必须判 deduplicated");
        assert_eq!(s.files().len(), before_files, "无重复文件记录");
        assert_eq!(
            s.content_version(),
            before_version,
            "no-op 不 bump content_version"
        );
        let (id1, _) = s.files().iter().map(|f| (f.id.clone(), ())).next().unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn content_version_bumps_on_write_not_on_read() {
        let mut s = store_with_files();
        let v = s.content_version();
        let _ = s.guard_tool("get_bid_project_state"); // 只读不 bump
        assert_eq!(s.content_version(), v);
        s.upsert_file("B3.xlsx", "图纸", "bb", 2, 1, 20, 0, ParseStatus::Ok, "")
            .unwrap();
        assert_eq!(s.content_version(), v + 1);
    }

    #[test]
    fn extract_requirements_idempotent_merge() {
        let mut s = store_with_files();
        let reqs = vec![Requirement {
            id: "REQ-0001".into(),
            category: "资格条件".into(),
            clause_no: Some("3.2.1".into()),
            anchors: vec![Anchor {
                file: "A".into(),
                kind: "para".into(),
                locator: "第三章".into(),
                excerpt: "独立承担民事责任能力的法人营业执照".into(),
            }],
            params: HashMap::new(),
            source_file: "A".into(),
        }];
        let (n1, added1) = s.set_requirements(reqs.clone()).unwrap();
        assert_eq!((n1, added1), (1, 1));
        let (n2, added2) = s.set_requirements(reqs).unwrap();
        assert_eq!((n2, added2), (1, 0), "同 dedupe_hash 重放 → 合并不新增");
        assert_eq!(
            s.project().unwrap().status,
            ProjectStatus::RequirementsExtracted
        );
    }

    #[test]
    fn optimistic_concurrency_conflict() {
        let s = store_with_files();
        assert!(matches!(
            s.check_version(Some(999)),
            Err(ToolError::EConflict { .. })
        ));
        let v = s.content_version();
        assert!(s.check_version(Some(v)).is_ok());
        assert!(s.check_version(None).is_ok(), "内部步骤跳过检查");
    }

    // ------------------------------------------------------------ #184 补遗逐件确认

    fn store_with_failed_addendum() -> (DomainStore, String) {
        let mut s = store_with_files();
        let (fid, _) = s
            .upsert_file(
                "C-补遗01.pdf",
                "补遗",
                "sha-aw-1",
                0, 0, 0, 0,
                ParseStatus::Failed,
                "ToUnicode CMap 缺失：文本层无法解码",
            )
            .unwrap();
        (s, fid)
    }

    #[test]
    fn addendum_waiver_happy_path_records_and_queries() {
        let (mut s, fid) = store_with_failed_addendum();
        let v = s.content_version();
        let rec = s
            .record_addendum_waiver(&fid, "已人工核对纸质原件", &TrustedPrincipal::human("184-lead"))
            .unwrap();
        assert_eq!(rec.file_id, fid);
        assert!(rec.at > 0, "确认时刻落钟");
        assert_eq!(rec.principal, "human:184-lead");
        assert!(s.addendum_waived(&fid), "file_id 匹配即该件已接受");
        let ws = s.addendum_waivers();
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].file_id, fid);
        assert_eq!(s.content_version(), v + 1, "写账即版本（与 record_review 同口径）");

        // 同件重复确认：追加不覆盖（审计日志语义）
        let rec2 = s
            .record_addendum_waiver(&fid, "复核后维持原确认", &TrustedPrincipal::human("184-second"))
            .unwrap();
        assert_ne!(rec2.at, 0);
        assert_eq!(s.addendum_waivers().len(), 2, "追加式留底");
        assert!(s.addendum_waived(&fid));
    }

    #[test]
    fn addendum_waiver_rejects_bad_principal_reason_and_ghost_file() {
        let (mut s, fid) = store_with_failed_addendum();
        // #182：Agent 身份的 TrustedPrincipal（域层纵深——即使壳层漏验，
        // 域层也不放 Agent 过「带病放行」的免责面；先于 file 存在性校验，
        // 操作者门是最上游的资格面，与 record_review 的裁决序同构）
        let agent = TrustedPrincipal {
            actor_id: "bot".into(),
            actor_type: ActorType::Agent,
            authentication_method: "hmac_receipt_v1".into(),
            approval_receipt: String::new(),
        };
        assert!(matches!(
            s.record_addendum_waiver(&fid, "r", &agent),
            Err(ToolError::EUnprocessable { .. })
        ));
        // 空白理由
        assert!(matches!(
            s.record_addendum_waiver(&fid, "   ", &TrustedPrincipal::human("x")),
            Err(ToolError::EUnprocessable { .. })
        ));
        // 幽灵 file_id（对不存在的件做确认 = 凭空免责）
        assert!(matches!(
            s.record_addendum_waiver("TF-9999", "r", &TrustedPrincipal::human("x")),
            Err(ToolError::ENotFound { .. })
        ));
        // 拒绝路径零副作用
        assert!(s.addendum_waivers().is_empty());
    }

    #[test]
    fn addendum_waiver_snapshot_roundtrip_and_old_snapshot_loads() {
        // 新字段往返：to_snapshot → restore 全保真
        let (mut s, fid) = store_with_failed_addendum();
        s.record_addendum_waiver(&fid, "人工核对原件", &TrustedPrincipal::human("rt-lead")).unwrap();
        let snap = s.to_snapshot();
        assert_eq!(snap.addendum_waivers.len(), 1);
        let restored = DomainStore::restore(snap).unwrap();
        assert_eq!(restored.addendum_waivers(), s.addendum_waivers());

        // 旧快照（无 addendum_waivers 字段）→ serde(default) 空集合加载
        let mut json = serde_json::to_value(s.to_snapshot()).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("addendum_waivers");
        let old_snap: StoreSnapshot = serde_json::from_value(json).unwrap();
        assert!(old_snap.addendum_waivers.is_empty());
        let old_store = DomainStore::restore(old_snap).unwrap();
        assert!(old_store.addendum_waivers().is_empty());

        // 孤儿确认（引用不存在的 file_id）→ 拒载（fail-closed）
        let mut bad = s.to_snapshot();
        bad.addendum_waivers.push(AddendumWaiverRecord {
            file_id: "TF-9999".into(),
            reason: "幽灵确认".into(),
            at: 1,
            principal: "human:x".into(),
            trusted: None,
        });
        assert!(matches!(
            DomainStore::restore(bad),
            Err(ToolError::EUnprocessable { .. })
        ));

        // #182 legacy 快照兼容：带 principal 无 trusted 的旧 waiver JSON
        // 载入不炸（trusted → None，宽容）；新记录则必有 trusted
        let legacy = serde_json::json!({
            "file_id": fid,
            "reason": "旧口径确认",
            "at": 1_700_000_000u64,
            "principal": "human:old-lead",
        });
        let rec: AddendumWaiverRecord = serde_json::from_value(legacy).unwrap();
        assert!(rec.trusted.is_none());
        assert_eq!(rec.principal, "human:old-lead");
        let new_rec = s.addendum_waivers()[0].clone();
        assert!(
            new_rec.trusted.as_ref().is_some_and(|t| t.actor_type == ActorType::Human),
            "新记录必有 trusted（Human）"
        );
    }

    #[test]
    fn record_addendum_waiver_state_window_is_files_imported_plus() {
        // 与 Gate::AddendumWaiver 同窗口：DRAFT 拒（E_STATE），FILES_IMPORTED+ 放行
        let mut s = DomainStore::new();
        s.create_project("窗口", "C-W", "明标", "w-key").unwrap();
        assert!(matches!(
            s.guard_tool("record_addendum_waiver"),
            Err(ToolError::EState { .. })
        ));
        s.upsert_file("A-招标文件.docx", "招标文件", "sha-w", 1, 0, 5, 0, ParseStatus::Ok, "")
            .unwrap();
        assert!(s.guard_tool("record_addendum_waiver").is_ok());
    }

    // ------------------------------------------------------------ #243 校验状态安全与恢复

    /// #243 夹具：两叶大纲（一章两节）+ 项目推进到指定状态。
    /// SEC-0003/SEC-0004 是叶（父 SEC-0002），REQ-0001/0002 挂两叶。
    fn store_with_outline_243(advance_to: ProjectStatus) -> DomainStore {
        use outline::{OutlineNode, SectionStatus};
        let mut s = store_with_files();
        s.set_requirements(vec![
            Requirement {
                id: "REQ-0001".into(),
                category: "格式要求".into(),
                clause_no: None,
                anchors: vec![Anchor {
                    file: "A-招标文件.docx".into(),
                    kind: "para".into(),
                    locator: "第一章/1.1".into(),
                    excerpt: "正文用仿宋三号字".into(),
                }],
                params: HashMap::new(),
                source_file: "A-招标文件.docx".into(),
            },
            Requirement {
                id: "REQ-0002".into(),
                category: "技术参数".into(),
                clause_no: None,
                anchors: vec![Anchor {
                    file: "A-招标文件.docx".into(),
                    kind: "para".into(),
                    locator: "第二章/2.1".into(),
                    excerpt: "设备功率不低于 100kW".into(),
                }],
                params: HashMap::new(),
                source_file: "A-招标文件.docx".into(),
            },
        ])
        .unwrap();
        let mk = |id: &str, pid: Option<&str>, title: &str, level: u8, order: u32, target: u32, reqs: Vec<&str>| OutlineNode {
            id: id.into(),
            parent_id: pid.map(str::to_string),
            title: title.into(),
            level,
            order,
            requirement_ids: reqs.into_iter().map(str::to_string).collect(),
            target_chars: target,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        };
        s.set_outline(vec![
            mk("SEC-0001", None, "技术标", 1, 1, 200, vec!["REQ-0001", "REQ-0002"]),
            mk("SEC-0002", Some("SEC-0001"), "响应正文", 2, 2, 200, vec!["REQ-0001", "REQ-0002"]),
            mk("SEC-0003", Some("SEC-0002"), "格式响应", 3, 3, 100, vec!["REQ-0001"]),
            mk("SEC-0004", Some("SEC-0002"), "参数响应", 3, 4, 100, vec!["REQ-0002"]),
        ])
        .unwrap();
        // 手写 SEC-0001..0004 不经 next_id 签发——restore 的水位校验要求
        // id_seq ≥ 最大后缀（4）。补两个哑件把序列抬到位（快照往返测试面）。
        s.upsert_file("B-哑件1.docx", "附件", "sha-dummy-1", 1, 0, 1, 0, ParseStatus::Ok, "").unwrap();
        s.upsert_file("B-哑件2.docx", "附件", "sha-dummy-2", 1, 0, 1, 0, ParseStatus::Ok, "").unwrap();
        // 合法链推进到目标态（状态机本身不校验草稿完成度——#243 的前置
        // 闸在服务层 validate；这正是历史缺陷能发生的原因，也是测试构造
        // 历史异常态的通道）
        for to in [
            ProjectStatus::MatrixReady,
            ProjectStatus::EvidenceBound,
            ProjectStatus::Drafting,
            ProjectStatus::Validating,
            ProjectStatus::Reviewing,
            ProjectStatus::ReadyToExport,
            ProjectStatus::Exported,
        ] {
            if s.project().unwrap().status == advance_to {
                break;
            }
            s.advance_project_status(to).unwrap();
        }
        assert_eq!(s.project().unwrap().status, advance_to);
        s
    }

    #[test]
    fn draft_incomplete_error_serializes_with_code_and_missing() {
        // 契约字面量 + missing 清单序列化形状。域层错误是外标签 serde
        //（无 tag="code"——那是 mcp 镜像 error.rs 的线形状，由 bid-mcp 测试
        // 钉）；这里钉域层形状（code() 字面量 + 变体名 + missing 清单）。
        let e = ToolError::EDraftIncomplete {
            missing: vec!["SEC-0003".into(), "SEC-0004".into()],
        };
        assert_eq!(e.code(), "E_DRAFT_INCOMPLETE");
        let j = serde_json::to_value(&e).unwrap();
        assert_eq!(
            j["E_DRAFT_INCOMPLETE"]["missing"],
            serde_json::json!(["SEC-0003", "SEC-0004"]),
            "missing 清单随变体携带（可操作面）"
        );
    }

    #[test]
    fn incomplete_leaf_ids_reports_undrafted_leaves_in_doc_order() {
        use outline::SectionStatus;
        let mut s = store_with_outline_243(ProjectStatus::Drafting);
        // 全 Pending：两叶都未完成
        assert_eq!(s.incomplete_leaf_ids(), vec!["SEC-0003", "SEC-0004"]);
        // SEC-0003 起草完成（经合法链 Pending→Outlining→Drafting→Drafted）
        for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
            s.advance_section_status("SEC-0003", to).unwrap();
        }
        assert_eq!(s.incomplete_leaf_ids(), vec!["SEC-0004"], "只报未完成叶");
        // SEC-0004 失败（Failed——失败的一等结果，也算未完成）
        for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Failed] {
            s.advance_section_status("SEC-0004", to).unwrap();
        }
        assert_eq!(
            s.incomplete_leaf_ids(),
            vec!["SEC-0004"],
            "Failed 仍未完成（重试成功后才算）"
        );
        // SEC-0004 重试成功 → 全部完成
        for to in [SectionStatus::Drafting, SectionStatus::Drafted] {
            s.advance_section_status("SEC-0004", to).unwrap();
        }
        assert!(s.incomplete_leaf_ids().is_empty(), "全叶 DRAFTED+ → 完成");
        // 结构节点（SEC-0001/0002 非叶）永不出现在清单
    }

    #[test]
    fn recover_validating_to_drafting_happy_path_with_audit() {
        use outline::SectionStatus;
        // 历史异常态：VALIDATING + SEC-0004 未完成（旧版校验误推进所致）
        let mut s = store_with_outline_243(ProjectStatus::Validating);
        for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
            s.advance_section_status("SEC-0003", to).unwrap();
        }
        let sec4_before = s
            .sections()
            .iter()
            .find(|n| n.id == "SEC-0004")
            .cloned()
            .unwrap();
        let reviews_before = s.reviews().to_vec();
        let version = s.content_version();

        let rec = s
            .recover_validating_to_drafting("历史异常态：旧版校验在缺章时误推进 VALIDATING")
            .unwrap();
        // 状态与版本
        assert_eq!(s.project().unwrap().status, ProjectStatus::Drafting, "回退到 DRAFTING（起草面解锁）");
        assert_eq!(s.content_version(), version + 1, "状态变更即版本变更");
        // 审计账（何时/为何/从哪到哪/缺哪些章）
        assert!(rec.at > 0, "恢复时刻落钟");
        assert_eq!(rec.from, ProjectStatus::Validating);
        assert_eq!(rec.to, ProjectStatus::Drafting);
        assert!(!rec.reason.trim().is_empty());
        assert_eq!(rec.missing_sections, vec!["SEC-0004"], "缺章快照入审计");
        assert_eq!(s.state_recoveries().len(), 1);
        assert_eq!(s.state_recoveries()[0].missing_sections, rec.missing_sections);
        // 不变量：章节树/审核记录原样（恢复是状态面动作）
        let sec4_after = s
            .sections()
            .iter()
            .find(|n| n.id == "SEC-0004")
            .cloned()
            .unwrap();
        assert_eq!(sec4_before, sec4_after, "章节内容/状态原样");
        assert_eq!(s.reviews(), reviews_before.as_slice(), "审核记录原样");
        // 起草面解锁：guard 放行 draft_response_section
        assert!(s.guard_tool("draft_response_section").is_ok());
    }

    #[test]
    fn recover_channel_rejects_wrong_state_complete_draft_and_blank_reason() {
        use outline::SectionStatus;
        // ① 非 VALIDATING 态（DRAFTING）→ E_STATE
        let mut s = store_with_outline_243(ProjectStatus::Drafting);
        let e = s.recover_validating_to_drafting("理由").unwrap_err();
        assert!(matches!(e, ToolError::EState { .. }), "DRAFTING 无死锁面: {e:?}");
        // ② VALIDATING 但草稿完整 → E_UNPROCESSABLE（健康态不构成恢复条件）
        let mut s = store_with_outline_243(ProjectStatus::Validating);
        for sid in ["SEC-0003", "SEC-0004"] {
            for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
                s.advance_section_status(sid, to).unwrap();
            }
        }
        let e = s.recover_validating_to_drafting("理由").unwrap_err();
        assert!(matches!(e, ToolError::EUnprocessable { .. }), "健康 VALIDATING 拒恢复: {e:?}");
        assert_eq!(s.project().unwrap().status, ProjectStatus::Validating, "拒绝零副作用");
        // ③ 理由空白 → E_UNPROCESSABLE（不可审计的恢复不成恢复）
        let mut s = store_with_outline_243(ProjectStatus::Validating);
        let e = s.recover_validating_to_drafting("   ").unwrap_err();
        assert!(matches!(e, ToolError::EUnprocessable { .. }), "空白理由拒: {e:?}");
        assert_eq!(s.project().unwrap().status, ProjectStatus::Validating, "拒绝零副作用");
        assert!(s.state_recoveries().is_empty(), "拒绝不入审计账");
    }

    #[test]
    fn recover_is_repeat_safe_second_call_hits_state_gate() {
        use outline::SectionStatus;
        let mut s = store_with_outline_243(ProjectStatus::Validating);
        for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
            s.advance_section_status("SEC-0003", to).unwrap();
        }
        s.recover_validating_to_drafting("首次恢复").unwrap();
        // 重复请求/超时重试：第二次调用 → E_STATE（项目已在 DRAFTING，
        // 可继续起草——不会落在无法恢复的状态）
        let e = s.recover_validating_to_drafting("重复请求").unwrap_err();
        assert!(matches!(e, ToolError::EState { .. }), "重复恢复被状态门拒: {e:?}");
        assert_eq!(s.state_recoveries().len(), 1, "审计账不重复");
        assert!(s.guard_tool("draft_response_section").is_ok(), "项目仍可起草");
    }

    #[test]
    fn state_recoveries_snapshot_roundtrip_and_legacy_load() {
        use outline::SectionStatus;
        let mut s = store_with_outline_243(ProjectStatus::Validating);
        for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
            s.advance_section_status("SEC-0003", to).unwrap();
        }
        s.recover_validating_to_drafting("快照往返").unwrap();
        // 往返全保真
        let snap = s.to_snapshot();
        assert_eq!(snap.state_recoveries.len(), 1);
        let restored = DomainStore::restore(snap).unwrap();
        assert_eq!(restored.state_recoveries(), s.state_recoveries());
        assert_eq!(restored.project().unwrap().status, ProjectStatus::Drafting, "恢复后的状态随快照走");
        // 旧快照（无 state_recoveries 字段）→ serde(default) 空集合加载
        let mut json = serde_json::to_value(s.to_snapshot()).unwrap();
        json.as_object_mut().unwrap().remove("state_recoveries");
        let old_snap: StoreSnapshot = serde_json::from_value(json).unwrap();
        let old_store = DomainStore::restore(old_snap).unwrap();
        assert!(old_store.state_recoveries().is_empty());
    }

    #[test]
    fn validating_allows_recovery_tool_and_drafting_keeps_draft() {
        // 工具面矩阵（#243 增量）：VALIDATING 放行 recover_validating_state；
        // DRAFTING 不放行（本就可起草，无需恢复）；Reviewing+ 不放行（审核/导出
        // 阶段语义不同——不在本通道）。
        let s = store_with_outline_243(ProjectStatus::Validating);
        assert!(s.guard_tool("recover_validating_state").is_ok());
        let s = store_with_outline_243(ProjectStatus::Drafting);
        assert!(matches!(
            s.guard_tool("recover_validating_state"),
            Err(ToolError::EState { .. })
        ));
        assert!(s.guard_tool("draft_response_section").is_ok(), "DRAFTING 起草面不受影响");
        let s = store_with_outline_243(ProjectStatus::Reviewing);
        assert!(matches!(
            s.guard_tool("recover_validating_state"),
            Err(ToolError::EState { .. })
        ));
    }
}
