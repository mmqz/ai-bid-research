//! review.rs —— T-7 ReviewRecord + RevisionTask 审核闭环（issue #11 批次 v3 / V3-4；
//! 任务池原文批次号 V4-4 系笔误，落库一律写 V3-4）。
//!
//! 现状补齐：`ProjectStatus::Reviewing` 此前只是个状态——无实体、无裁决面，
//! 评审发现问题后无法关单（发现没有落点、修复没有工单、修没修完没有口径）。
//! 本模块补审核面三件套：**发现（Finding）→ 记录（ReviewRecord）→ 工单
//! （RevisionTask）**。
//!
//! 三道门（对应人类团队的评审节点，门名取规格原文蛇形）：
//! - `matrix_approval` 矩阵定稿门：条款响应矩阵（REQ × 响应状态）定稿——
//!   先定「响应什么」；
//! - `outline_approval` 章节树定稿门：大纲定稿——再定「怎么组织」；
//! - `pre_export_confirm` 导出前确认门：付印前最后一道人眼。
//! **门序强制**（矩阵先于大纲先于导出确认——人类流程先定响应面再定结构面）：
//! 跳门/乱序记审 → E_STATE typed reject（零副作用）——这就是「绕过门」的裁决面。
//!
//! BLOCKER → RT 强制派生（**结构保证非约定保证**）：`record_review` 对每条
//! Blocker finding 自动创建 RevisionTask（owner 空、status=Open、锚
//! `section_id.unwrap_or("PROJECT")`）。「每个 BLOCKER 必有 RT」是数据结构
//! 不变量——findings 中 Blocker 数 ≡ revision_tasks 数（无遗漏无多生），
//! restore 对孤儿 RT 同样把关（半个闭环拒载）。Warning/Info 不派生——
//! 不构成阻断，不进关单面。
//!
//! 导出前置裁决（[`DomainStore::export_allowed`]——单一真源）：
//! `三门均 Approved ∧ 全部 RT DONE ∧ 审核内容未变更` 才放行。第三面是
//! #180 的内容绑定：PreExportConfirm 记录落账时快照章节树摘要
//! （[`crate::outline::sections_digest`]），导出时比对——过门后改了章节
//! 正文/结构 = 旧审核失效（「审核针对的内容已变更」）。绑摘要不绑全局
//! content_version（后者被导入无关件/记审自身/导出推进一律 bump——必误报）。
//!
//! 与 export_gate.rs 两阶段门的
//! 语义串联：本面是「**能不能开始导出**」（pre_export 门禁的前置裁决面），
//! 那面是「**导出原子性**」（EXPORTING → sha256 落库为界 → EXPORTED——
//! 中断绝不出现「业务态 EXPORTED 而产物缺失」的虚假成功，该纪律不变）；
//! 两面前后串联、互补不重叠。
//!
//! 复审人身份（#182 TrustedPrincipal）：旧口径只查 `human:` 字符串前缀
//! ——Agent 自称 `human:admin` 即可过门（P0-2 缺口）。现口径：过审面收
//! 验签后的 [`crate::principal::TrustedPrincipal`]（HMAC-SHA256 receipt，
//! 服务层验签是 MCP 面的唯一入口）；域层 `record_review` 再拦一次
//! `actor_type == Human`（纵深防御——即使壳层漏验，域层也不放 Agent
//! 过门）。`reviewer` 字段降级为 `human:{actor_id}` 派生值（legacy 快照
//! 兼容 + 报告回显），不再是身份判据。

use crate::principal::{ActorType, TrustedPrincipal};
use crate::{DomainStore, ProjectStatus, ToolError};
use serde::{Deserialize, Serialize};

/// 复审人前缀（borrow mdcore workbench ReviewPolicy：`reviewer_prefixes` 默认
/// `["human:"]`——人侧操作者一律 `human:` 前缀，AI 不得自审自过）。
/// #182 起：仅作 `human:{actor_id}` 派生值的格式面（不再是从入参查的
/// 身份判据——身份判据升级为 TrustedPrincipal 验签）。
/// （pub：lib.rs 的 record_addendum_waiver 审计接线同口径派生——单一真源。）
pub const HUMAN_PREFIX: &str = "human:";

/// 落钟（no-chrono 纪律——u64 UNIX_EPOCH 秒；与 dates.rs `today_ymd` 同手法，
/// 时钟异常回退 → 0，绝不 panic）。
fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- 类型面
/// 审核门（T-7 规格；issue #11 批次 v3 / V3-4）。三道门对应人类团队评审：
/// 矩阵定稿门（条款响应矩阵）/ 章节树定稿门（大纲）/ 导出前确认门。
/// 另有第四道独立闸门 [`Gate::AddendumWaiver`]（无补遗确认——#131 真实
/// 语料验证落地的 L3 人工出口，不入三道主门的门序链）。
///
/// `as_str` 取规格原文蛇形（`matrix_approval` 等——门名/错误消息的引用口径）；
/// serde 形随库惯例 SCREAMING_SNAKE_CASE（持久化面），两形并存各司其职。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Gate {
    MatrixApproval,
    OutlineApproval,
    PreExportConfirm,
    /// 无补遗确认门（#131 真实语料验证发现的缺口补齐）：L3 的
    /// `MissingAddendum` 阻塞提示语承诺「或明确确认本项目无补遗后再
    /// 继续」，但该确认通路此前不存在——真实单件招标文件（无补遗件）
    /// 被死锁在起草闸外，而 synth e2e 因夹具自带 C-补遗件永远踩不到。
    /// Approved = 人工确认本项目无补遗（`missing_addendum` 放行）；
    /// 与三道主门正交：无状态门、无门序前置（导入后任意时刻可记）。
    /// 导入补遗件后走正常判定，waiver 不再被消费（自然失效）。
    AddendumWaiver,
}

impl Gate {
    pub fn as_str(&self) -> &'static str {
        match self {
            Gate::MatrixApproval => "matrix_approval",
            Gate::OutlineApproval => "outline_approval",
            Gate::PreExportConfirm => "pre_export_confirm",
            Gate::AddendumWaiver => "addendum_waiver",
        }
    }
}

/// 过门裁决。Approved = 过门里程碑（同门重复 → 幂等放行不复位 RT）；
/// Rejected = 拒审过程记录（不撤回既有 Approved 里程碑——新问题经新 RT
/// 收紧导出面，门撤销/重开不在 T-7 规格面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Decision {
    Approved,
    Rejected,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Approved => "APPROVED",
            Decision::Rejected => "REJECTED",
        }
    }
}

/// 发现严重度。BLOCKER = 阻断级（强制派生 RT + 导出闸门）；WARNING/INFO =
/// 不阻断、不派生（登记在案但不进关单面——只有 BLOCKER 有 RT）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Blocker,
    Warning,
    Info,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Blocker => "BLOCKER",
            Severity::Warning => "WARNING",
            Severity::Info => "INFO",
        }
    }
}

/// 修订工单状态（Open→InProgress→Done 逐级；Done 终态不可重开——重走门 =
/// 新 finding 新 RT，不复活旧单）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RTStatus {
    Open,
    InProgress,
    Done,
}

impl RTStatus {
    /// 合法前向链（T-7 规格）。
    pub fn next_allowed(&self) -> &'static [RTStatus] {
        use RTStatus::*;
        match self {
            Open => &[InProgress],
            InProgress => &[Done],
            Done => &[],
        }
    }

    pub fn as_str(&self) -> &'static str {
        use RTStatus::*;
        match self {
            Open => "OPEN",
            InProgress => "IN_PROGRESS",
            Done => "DONE",
        }
    }
}

/// 审核发现（评审时点的事实快照——section_id 不校验章节存在性：大纲重推导
/// 不追溯失效历史发现，历史发现按记录时点锚定）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String, // FND-xxxx（store 单序列）
    pub severity: Severity,
    /// 自由分类面（如 "金额冲突"/"占位符残留"——S-5 检查器的输出口径）
    pub category: String,
    pub message: String,
    /// None = 全局级发现（不挂在单章——RT 派生时锚根项目章 "PROJECT"）
    pub section_id: Option<String>,
}

/// 审核记录（一次过门/拒审的完整快照——T-7 规格 struct 落地；#180 增
/// 内容绑定双字段）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub id: String, // REV-xxxx（store 单序列）
    pub gate: Gate,
    /// 复审人（`human:{actor_id}` 派生值——legacy 快照兼容 + 报告回显；
    /// 身份判据已升级为 #182 TrustedPrincipal，见 `principal` 字段）
    pub reviewer: String,
    /// #182 可信身份：验签后的 TrustedPrincipal。None = legacy 快照
    /// （#182 之前的记录——宽容载入，裁决面按旧口径不回溯失效）；
    /// 新记录必须 Some（record_review 收 TrustedPrincipal 构造）。
    #[serde(default)]
    pub principal: Option<TrustedPrincipal>,
    pub decision: Decision,
    /// FND-id 清单（发现实体住 store 的 findings 集合——记录引用不复制）
    pub findings: Vec<String>,
    /// UNIX_EPOCH 秒（no-chrono 纪律）
    pub at: u64,
    /// 记审时点的全局 content_version（#180 **辅助账面**——判定不消费：
    /// 全局版本对「导入无关件/记审自身/导出推进」一律 bump，绑它必误报；
    /// 真判据是下面的 digest）。`#[serde(default)]` 加性迁移（#147 先例）。
    #[serde(default)]
    pub reviewed_content_version: u64,
    /// 记审时点的章节树内容摘要（#180 **判据**：`export_allowed` 与当前
    /// [`DomainStore::sections_digest`] 比对——变了 = 审核针对的内容已变更）。
    /// `""` = legacy 快照（#180 之前的记录未绑定——判定面跳过，宽容载入，
    /// 旧数据的洞保留；重新过门即补上绑定）。
    #[serde(default)]
    pub reviewed_content_digest: String,
}

/// 修订工单（BLOCKER finding 的关单面——每个 BLOCKER 恰一个 RT，结构不变量）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionTask {
    pub id: String, // RT-xxxx（store 单序列）
    /// 源 FND-id（restore 把关存在性——孤儿 RT 拒载）
    pub finding_id: String,
    /// RT 锚定章节（全局发现锚根项目章 "PROJECT"）
    pub section_id: String,
    /// 修订执行者（任意操作者面——与复审人不同，无 human: 前缀约束：
    /// AI 可以修单，不能过门）
    pub owner: String,
    pub status: RTStatus,
    /// 关单说明（DONE 时必填非空——空说明不算修完，诚实状态）
    pub resolution: Option<String>,
}

/// 记审回执（record_review 的返回面——工具层透传给复审发起者）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewReceipt {
    pub review_id: String,
    pub gate: Gate,
    pub decision: Decision,
    pub finding_ids: Vec<String>,
    /// 本次记审派生的 RT 清单（BLOCKER 强制派生——Warning/Info 不在此面）
    pub spawned_rt_ids: Vec<String>,
    /// 本次记审绑定的章节树内容摘要（#180——receipt 加法面；幂等重放时为
    /// 既有记录的摘要，新落账时为当前摘要）。
    pub reviewed_content_digest: String,
}

// ---------------------------------------------------------------- #277 方案 A：GatePassRecord

/// 过门方式（审计账的 method 面——#277 方案 A）。
///
/// - `human_receipt`：宿主签发的人类签域 receipt 验签通过（#182 现状路径）
/// - `agent_declared`：agent 签域 receipt 验签通过（#277 新增——agent 自
///   声明过门，require_human_receipt=false 的工作区才放行，导出带警告）
pub const METHOD_HUMAN_RECEIPT: &str = "human_receipt";
pub const METHOD_AGENT_DECLARED: &str = "agent_declared";

/// 过门审计记录（#277 方案 A——**无法事后抹除**的过门四要素+理由）。
///
/// 与 [`ReviewRecord`] 的分工：reviews 是**裁决面**（门序/幂等/内容绑定/
/// RT 派生的机器判据），gate_passes 是**审计面**（谁、以什么身份、用什么
/// 方法、什么时候、为什么过门——append-only，追认不覆盖：真人补 human
/// receipt 是**追加**一条 human_confirmed=true，两条并存可追溯，导出
/// 警告看「每门最新一条」）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatePassRecord {
    pub id: String, // GP-xxxx（store 单序列）
    pub gate: Gate,
    /// 过门者 id（human 面 = 宿主身份命名；agent 面 = 调用方自述，如
    /// "agent-01"——actor_type 才是结构保证，id 只是审计四要素之一）。
    pub actor_id: String,
    pub actor_type: crate::principal::ActorType,
    /// `human_receipt` | `agent_declared`（见 METHOD_* 常量）。
    pub method: String,
    /// false = agent 声明过门（未经人工确认——导出警告的判据）；
    /// true = human receipt 过门/追认。
    pub human_confirmed: bool,
    /// UNIX_EPOCH 秒（no-chrono 纪律）。
    pub at: u64,
    /// 过门理由（agent 路径必填非空——「为什么可以过这个门」；human 路径
    /// 可空——receipt 本身就是理由的信任锚）。
    pub reason: String,
}

// ---------------------------------------------------------------- DomainStore 扩面（T-7 审核闭环）
impl DomainStore {
    /// 门是否已有 Approved 记录（门序进度的判定面——Rejected 是过程记录，
    /// 不推进门序；Approved 是里程碑事实，不因后续 Rejected 撤销）。
    /// pub 化（#80 第三批 MCP 补齐 G3）：`record_review_gate` 工具出参
    /// gates_approved 消费同一真源——壳层不得自建第二套门状态。
    pub fn gate_approved(&self, gate: Gate) -> bool {
        self.reviews.iter().any(|r| r.gate == gate && r.decision == Decision::Approved)
    }

    // ------------------------------------------------------------ 记审（T-7 主入口）
    /// 记审（三道门的统一写入面；#182——身份面收验签后的 TrustedPrincipal；
    /// #277 方案 A——agent 声明过门面）。
    ///
    /// 裁决序（全部前置校验通过才落账——拒绝路径零副作用）：
    /// 1. 可信身份门（#182/#277：`(actor_type, scope_is_agent)` 二元组——
    ///    Human×人类签域 = 真人（现状不变）；Agent×agent 签域 = agent
    ///    声明过门（留痕——放行前提在服务层 require_human_receipt 已裁）；
    ///    混用签域 / System 一律拒）；
    /// 2. agent 路径 reason 必填非空（#277 验收 7——agent 必须说明「为什么
    ///    可以过这个门」，不能空着过）；
    /// 3. 状态门（裁决表，见各 match 臂注释）；
    /// 4. 门序（OutlineApproval 需矩阵门先过；PreExportConfirm 需前两门均过
    ///    ——跳门 → E_STATE typed reject，这就是「绕过门」的裁决面）；
    /// 5. findings 形状（category/message 非空——空发现不可审计）；
    /// 6. Rejected 必须携带 ≥1 条 BLOCKER（Warning 不构成拒审依据——诚实状态）；
    /// 7. 同门重复 Approved → 幂等放行（#277 起 agent/human 族区分：human
    ///    只对 human 记录幂等（agent 过的门真人补签 = 追认，落新记录）；
    ///    agent 对同 actor 同摘要幂等（重试不刷账），换 agent 落新记录）。
    ///
    /// BLOCKER 强制派生：每条 Blocker finding 自动创建 RevisionTask（owner 空、
    /// status=Open、锚 `section_id.unwrap_or("PROJECT")`）——「每个 BLOCKER 有
    /// RT」由结构保证而非约定保证。
    ///
    /// #277：Approved 落账时同步追加 [`GatePassRecord`]（审计面——导出警告
    /// 与追认判据）。`agent_reason`：agent 路径必填；human 路径可空。
    ///
    /// findings 入参四元组：(severity, category, message, section_id)。
    pub fn record_review(
        &mut self,
        gate: Gate,
        principal: &TrustedPrincipal,
        decision: Decision,
        findings: Vec<(Severity, String, String, Option<String>)>,
        agent_reason: Option<&str>,
    ) -> Result<ReviewReceipt, ToolError> {
        self.project()?; // 审核记录挂在项目下（M1 单项目存储）
        let status = self.project()?.status;

        // ---- 1. 可信身份门（#182 纵深防御 + #277 方案 A 二元组）
        //      服务层验签是唯一入口（域层不重复实现验签——零密钥零时钟）；
        //      域层拦的是**身份/签域不匹配**：Human 必须人类签域（#182 口径
        //      不变），Agent 必须 agent 签域（scope_is_agent 只能由
        //      verify_agent_receipt 验出——构造面不可伪造）。
        let is_agent_pass = match (principal.actor_type, principal.scope_is_agent) {
            (ActorType::Human, false) => false, // 真人人门（现状不变）
            (ActorType::Agent, true) => true,  // agent 声明过门（留痕）
            (ActorType::System, _) => {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "复审人 {:?} 是 system 身份（过门面不收 System——机器内部操作走操作日志，不走审核门）",
                        principal.actor_id
                    ),
                });
            }
            (ActorType::Human, true) => {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "复审人 {:?} 是 Human 身份但携带 agent 签域（签域与身份不匹配——人类签域不带 :agent 后缀，人类面不走 agent 声明通道）",
                        principal.actor_id
                    ),
                });
            }
            (ActorType::Agent, false) => {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "复审人 {:?} 是 agent 身份但未携带 agent 签域（Agent 不得用人类签域冒充真人过门——agent 声明过门走 :agent 签域，require_human_receipt=false 工作区才放行，见 #277）",
                        principal.actor_id
                    ),
                });
            }
        };
        // ---- 2. agent 路径 reason 必填（#277 验收 7）
        let agent_reason_owned = if is_agent_pass {
            let r = agent_reason.unwrap_or("").trim().to_string();
            if r.is_empty() {
                return Err(ToolError::EUnprocessable {
                    reason: "agent 声明过门必须携带非空 reason（审计四要素之「为什么可以过这个门」——空着过 = 不可审计的确认）".into(),
                });
            }
            r
        } else {
            String::new()
        };
        // legacy 回显面：reviewer 从可信身份派生（human:{actor_id} / agent:{actor_id}）
        let reviewer = if is_agent_pass {
            format!("agent:{}", principal.actor_id)
        } else {
            format!("{HUMAN_PREFIX}{}", principal.actor_id)
        };

        // ---- 2. 状态门 + 3. 门序（按门分派的裁决表）
        match gate {
            // 矩阵门：矩阵产出后（MATRIX_READY/EVIDENCE_BOUND/DRAFTING——T-7
            // 规格状态集合；VALIDATING+ 不再受理矩阵门复审——门要在窗口内过）
            Gate::MatrixApproval => {
                if !matches!(
                    status,
                    ProjectStatus::MatrixReady
                        | ProjectStatus::EvidenceBound
                        | ProjectStatus::Drafting
                ) {
                    return Err(ToolError::EState {
                        current: status,
                        attempted: "record_review(matrix_approval)",
                    });
                }
            }
            // 大纲门：非 Draft 早期态（REQUIREMENTS_EXTRACTED+——规格简化口径：
            // 大纲门本质上跟在矩阵门后，状态面只需挡住「条款都没抽」的早期态）
            Gate::OutlineApproval => {
                if matches!(status, ProjectStatus::Draft | ProjectStatus::FilesImported) {
                    return Err(ToolError::EState {
                        current: status,
                        attempted: "record_review(outline_approval)",
                    });
                }
                // 门序：矩阵先于大纲（人类流程先定响应面再定结构面）
                if !self.gate_approved(Gate::MatrixApproval) {
                    return Err(ToolError::EState {
                        current: status,
                        attempted: "record_review(outline_approval)——前置门 matrix_approval 未 Approved（门序：矩阵先于大纲）",
                    });
                }
            }
            // 导出确认门：无状态门（三门最末，只受门序约束——矩阵门的状态
            // 集合已保证项目走到 MATRIX_READY+ 才可能有前两门的 Approved）
            Gate::PreExportConfirm => {
                if !self.gate_approved(Gate::MatrixApproval) {
                    return Err(ToolError::EState {
                        current: status,
                        attempted: "record_review(pre_export_confirm)——前置门 matrix_approval 未 Approved（门序强制）",
                    });
                }
                if !self.gate_approved(Gate::OutlineApproval) {
                    return Err(ToolError::EState {
                        current: status,
                        attempted: "record_review(pre_export_confirm)——前置门 outline_approval 未 Approved（门序强制）",
                    });
                }
            }
            // 无补遗确认门（#131）：无状态门、无门序前置——它是 L3 阻塞的
            // 人工出口，与三道主门正交（导入后任意状态可确认；记审语义与
            // 主门一致：Approved 是里程碑事实，Rejected 是过程记录）
            Gate::AddendumWaiver => {}
        }

        // ---- 4. findings 形状（诚实状态：空分类/空消息的发现是无效记录）
        for (_, category, message, _) in &findings {
            if category.trim().is_empty() || message.trim().is_empty() {
                return Err(ToolError::EUnprocessable {
                    reason: "finding 的 category/message 不得为空（空发现 = 不可审计的记录）".into(),
                });
            }
        }

        // ---- 5. 拒审依据门：Rejected 必须有阻断级依据
        if decision == Decision::Rejected
            && !findings.iter().any(|(sev, ..)| *sev == Severity::Blocker)
        {
            return Err(ToolError::EUnprocessable {
                reason: "Rejected 必须携带至少一条 BLOCKER finding（Warning 不构成拒审依据——诚实状态）".into(),
            });
        }

        // ---- #180 内容绑定素材：记审时点的章节树摘要与全局版本（全部门
        //      记录统一携带；判定面只消费 PreExportConfirm——见 export_allowed）。
        //      先于幂等分支计算：幂等判定本身要比摘要（内容变了不走幂等）。
        let reviewed_content_digest = self.sections_digest();
        let reviewed_content_version = self.content_version();

        // ---- 7. 同门重复 Approved：幂等放行（**先比 #180 摘要**——内容
        //      已变更则不走幂等、落新 REV，否则改稿后重过门永远返回旧记录
        //      = 死锁；比对取**最后一条**同门 Approved：多轮过门时旧记录的
        //      摘要已陈旧，幂等只对最新一轮成立。legacy 空摘要视为不匹配
        //      → 落新记录顺带补上绑定，只发生一次）。
        //      #277 幂等的族区分：human 过门只对 human 族记录幂等（agent
        //      过的门真人补签 = **追认**，落新记录 + 新 GatePass——警告才能
        //      消失且两账可追溯）；agent 过门对**同 actor 同摘要**幂等
        //      （重试不刷账本），换 agent actor 落新记录（审计可数）。
        if decision == Decision::Approved {
            let existing = self
                .reviews
                .iter()
                .rposition(|r| r.gate == gate && r.decision == Decision::Approved)
                .map(|i| &self.reviews[i]);
            if let Some(existing) = existing {
                if existing.reviewed_content_digest == reviewed_content_digest {
                    let existing_is_human = existing
                        .principal
                        .as_ref()
                        .map_or(true, |p| p.actor_type == ActorType::Human); // legacy None = #182 前的人类记录
                    let same_agent_actor = existing.principal.as_ref().is_some_and(|p| {
                        p.actor_type == ActorType::Agent && p.actor_id == principal.actor_id
                    });
                    let idempotent = if is_agent_pass {
                        same_agent_actor
                    } else {
                        existing_is_human
                    };
                    if idempotent {
                        // 返回**既有序列号**（首次过门记录的 REV-id + 其 FND 清单 +
                        // 首次派生的 RT 清单），不新建记录、content_version 不动。
                        // 携带的新 findings 不入账——门已 Approved，重复过门不是登记
                        // 新发现的通道（新发现走 Rejected 记录）。既有 RT 一律不动
                        // （不复位/不重派/不重复派生）：**重新过门 ≠ 已修完**——修没
                        // 修完只看 RT 状态，不看门又开了一次。
                        let spawned_rt_ids = self.revision_tasks.iter()
                            .filter(|rt| existing.findings.contains(&rt.finding_id))
                            .map(|rt| rt.id.clone())
                            .collect();
                        return Ok(ReviewReceipt {
                            review_id: existing.id.clone(),
                            gate,
                            decision,
                            finding_ids: existing.findings.clone(),
                            spawned_rt_ids,
                            reviewed_content_digest: existing.reviewed_content_digest.clone(),
                        });
                    }
                    // 不幂等（agent→human 追认 / human→agent 声明 / 换 agent）
                    // → 落新记录（下方统一落账路径）。
                }
                // 摘要 ≠ 当前（含 legacy ""）→ 不走幂等，落新记录（下方统一
                // 落账路径）——新记录绑当前摘要，重过门后 export_allowed
                // 恢复放行。
            }
        }

        // ---- 落账：签发 FND → BLOCKER 派生 RT → 落 ReviewRecord → 版本 +1
        let mut finding_ids = Vec::with_capacity(findings.len());
        let mut spawned_rt_ids = Vec::new();
        for (severity, category, message, section_id) in findings {
            let fid = self.next_id("FND");
            if severity == Severity::Blocker {
                // BLOCKER 强制派生 RT：锚 section_id.unwrap_or("PROJECT")
                // （全局发现锚根项目章——关单面仍有落点）
                let rid = self.next_id("RT");
                self.revision_tasks.push(RevisionTask {
                    id: rid.clone(),
                    finding_id: fid.clone(),
                    section_id: section_id.clone().unwrap_or_else(|| "PROJECT".to_string()),
                    owner: String::new(), // 派生即 Open 未指派——认领走 assign_revision_task
                    status: RTStatus::Open,
                    resolution: None,
                });
                spawned_rt_ids.push(rid);
            }
            self.findings.push(Finding {
                id: fid.clone(),
                severity,
                category,
                message,
                section_id,
            });
            finding_ids.push(fid);
        }
        let review_id = self.next_id("REV");
        self.reviews.push(ReviewRecord {
            id: review_id.clone(),
            gate,
            reviewer,
            principal: Some(principal.clone()),
            decision,
            findings: finding_ids.clone(),
            at: now_epoch_secs(),
            // #180 内容绑定：落账即快照（判据在 export_allowed 消费）
            reviewed_content_version,
            reviewed_content_digest: reviewed_content_digest.clone(),
        });
        // ---- #277 方案 A：Approved 过门同步落审计账（GatePassRecord——
        //      append-only，追认不覆盖；导出警告判据 = 每门最新一条的
        //      human_confirmed）
        if decision == Decision::Approved {
            let gp_id = self.next_id("GP");
            self.gate_passes.push(GatePassRecord {
                id: gp_id,
                gate,
                actor_id: principal.actor_id.clone(),
                actor_type: principal.actor_type,
                method: if is_agent_pass {
                    METHOD_AGENT_DECLARED.to_string()
                } else {
                    METHOD_HUMAN_RECEIPT.to_string()
                },
                human_confirmed: !is_agent_pass,
                at: now_epoch_secs(),
                reason: agent_reason_owned,
            });
        }
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(ReviewReceipt {
            review_id,
            gate,
            decision,
            finding_ids,
            spawned_rt_ids,
            reviewed_content_digest,
        })
    }

    // ------------------------------------------------------------ 只读面
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    pub fn reviews(&self) -> &[ReviewRecord] {
        &self.reviews
    }

    pub fn revision_tasks(&self) -> &[RevisionTask] {
        &self.revision_tasks
    }

    /// #277 方案 A：过门审计账（append-only——追认不覆盖，两条并存可追溯）。
    pub fn gate_passes(&self) -> &[GatePassRecord] {
        &self.gate_passes
    }

    /// #277 方案 A：指定门的**最新一条**过门记录（导出警告的判据位——
    /// 追认后最新条翻 human，警告消失；历史 agent 条保留在账本里）。
    pub fn latest_gate_pass(&self, gate: Gate) -> Option<&GatePassRecord> {
        self.gate_passes.iter().rev().find(|p| p.gate == gate)
    }

    /// #277 方案 A：agent 声明过门（未经人工确认）的门清单——每门最新一条
    /// `human_confirmed == false` 即列入（导出警告横幅 + 响应面清单的素材）。
    pub fn agent_gate_passes(&self) -> Vec<&GatePassRecord> {
        [
            Gate::MatrixApproval,
            Gate::OutlineApproval,
            Gate::PreExportConfirm,
            Gate::AddendumWaiver,
        ]
        .iter()
        .filter_map(|g| self.latest_gate_pass(*g))
        .filter(|p| !p.human_confirmed)
        .collect()
    }

    /// #277 方案 A：全部门（有过门记录的）最新一条都是 human 确认 → true
    /// （导出无警告）；任一门最新条是 agent 声明 → false（导出带横幅）。
    /// 无任何过门记录的空账本 → true（没有门被 agent 过——无从警告）。
    pub fn human_review_complete(&self) -> bool {
        self.agent_gate_passes().is_empty()
    }

    /// 未关单清单（status != Done 的 RT——export_allowed 的 RT 面输入）。
    pub fn open_blockers(&self) -> Vec<&RevisionTask> {
        self.revision_tasks.iter()
            .filter(|rt| rt.status != RTStatus::Done)
            .collect()
    }

    // ------------------------------------------------------------ RT 生命周期
    /// RT 指派（认领/改派）。owner 是修订执行者面——任意操作者（人与 AI
    /// 均可修单；与复审人不同，无 human: 前缀约束：AI 可以修单，不能过门）。
    pub fn assign_revision_task(&mut self, rt_id: &str, owner: &str) -> Result<(), ToolError> {
        let idx = self.revision_tasks.iter().position(|rt| rt.id == rt_id)
            .ok_or_else(|| ToolError::ENotFound { what: format!("revision_task {rt_id} 不存在") })?;
        if self.revision_tasks[idx].status == RTStatus::Done {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "{rt_id} 已 DONE——终态不可改派（重走门 = 新 finding 新 RT，不复活旧单）"
                ),
            });
        }
        if owner.trim().is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "owner 不得为空（空指派 = 无人认领的幽灵工单）".into(),
            });
        }
        self.revision_tasks[idx].owner = owner.to_string();
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }

    /// RT 状态通用推进面（complete_revision_task 是它的「带 resolution 特化」）。
    ///
    /// - Open → InProgress：唯一可经本面推进的边；
    /// - → Done：**拒绝**——关单必须带 resolution（结构不变量：Done ⟹
    ///   resolution 非空），本面无 resolution 参，走 [`Self::complete_revision_task`]；
    /// - Done：终态零出边（next_allowed = []——重开 = 重走门派新单，不复活旧单）。
    pub fn advance_rt_status(&mut self, rt_id: &str, to: RTStatus) -> Result<(), ToolError> {
        let idx = self.revision_tasks.iter().position(|rt| rt.id == rt_id)
            .ok_or_else(|| ToolError::ENotFound { what: format!("revision_task {rt_id} 不存在") })?;
        let cur = self.revision_tasks[idx].status;
        // 目标是 Done 时优先给「走 complete」指引（无论当前态是否含 Done 边
        // ——Open 直接推 Done 的报错也该指向正路，而不是只报合法前向清单）
        if to == RTStatus::Done {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "RT {rt_id} 当前 {}——advance 不接受 Done：关单必须带 resolution，走 complete_revision_task（空说明不算修完）",
                    cur.as_str()
                ),
            });
        }
        if !cur.next_allowed().contains(&to) {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "RT 状态 {} 不允许推进到 {}（合法前向: {:?}；Done 终态不可重开——重走门 = 新 finding 新 RT）",
                    cur.as_str(),
                    to.as_str(),
                    cur.next_allowed().iter().map(|s| s.as_str()).collect::<Vec<_>>()
                ),
            });
        }
        self.revision_tasks[idx].status = to;
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }

    /// RT 关单（advance 的 Done 特化——resolution 必填非空）。
    ///
    /// - 仅 InProgress 可关（Open 直接 Done = 越级拒——先认领开工再关单）；
    /// - resolution 必填非空（空说明不算修完——诚实状态）；
    /// - DONE 后不可重开（重走门 = 新 finding 新 RT，不复活旧单）。
    pub fn complete_revision_task(&mut self, rt_id: &str, resolution: &str) -> Result<(), ToolError> {
        let idx = self.revision_tasks.iter().position(|rt| rt.id == rt_id)
            .ok_or_else(|| ToolError::ENotFound { what: format!("revision_task {rt_id} 不存在") })?;
        let cur = self.revision_tasks[idx].status;
        if cur == RTStatus::Done {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "{rt_id} 已 DONE——终态不可重开（重走门 = 新 finding 新 RT，不复活旧单）"
                ),
            });
        }
        if !cur.next_allowed().contains(&RTStatus::Done) {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "{rt_id} 当前 {} 不允许直接关单（Open→InProgress→Done 逐级——先 advance 开工）",
                    cur.as_str()
                ),
            });
        }
        if resolution.trim().is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "resolution 不得为空（空说明不算修完——关单必须交代改了什么）".into(),
            });
        }
        self.revision_tasks[idx].status = RTStatus::Done;
        self.revision_tasks[idx].resolution = Some(resolution.to_string());
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }

    // ------------------------------------------------------------ 导出前置裁决
    /// 导出前置裁决（单一真源——「能不能开始导出」）。
    ///
    /// - 三门均须有 Approved 记录（缺任一 → E_STATE 点名缺哪门——「绕过门
    ///   typed reject」的查询面孪生：记审面挡写入，本面挡导出起步）；
    /// - #180 内容绑定：最后一条 PreExportConfirm Approved 的
    ///   `reviewed_content_digest` 非空且 ≠ 当前 [`DomainStore::sections_digest`]
    ///   → E_UNPROCESSABLE（审核针对的内容已变更，请重新审核）——三门
    ///   之后、RT 之前：门虽过了，但针对的是旧内容，重审比修 RT 更根本；
    /// - 全部 RT DONE（open_blockers 空——否则 E_UNPROCESSABLE，消息含未关单数）。
    ///
    /// 与 export_gate.rs 两阶段门的串联语义：本面是**前置裁决面**
    /// （pre_export_confirmed ∧ 全 RT DONE ∧ 内容未变更 才放行 EXPORTING 起步）；
    /// 那面承载**导出原子性**（EXPORTING → sha256 落库为界 → EXPORTED——渲染中断/
    /// 确认前崩溃绝不产生「业务态 EXPORTED 而产物缺失」的虚假成功）。两面
    /// 前后串联、纪律互补不重叠。
    pub fn export_allowed(&self) -> Result<(), ToolError> {
        let p = self.project()?;
        // 三门（门序遍历——缺哪门点哪门）
        for gate in [Gate::MatrixApproval, Gate::OutlineApproval, Gate::PreExportConfirm] {
            if !self.gate_approved(gate) {
                return Err(ToolError::EState {
                    current: p.status,
                    attempted: missing_gate_attempted(gate),
                });
            }
        }
        // #180 内容绑定（取**最后一条** PreExportConfirm Approved——多轮过门时
        // 旧记录的摘要陈旧；空摘要 = legacy 快照未绑定 → 跳过判定，宽容载入）
        if let Some(last) = self.reviews.iter().rev().find(|r| {
            r.gate == Gate::PreExportConfirm && r.decision == Decision::Approved
        }) {
            let current = self.sections_digest();
            if !last.reviewed_content_digest.is_empty()
                && last.reviewed_content_digest != current
            {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "导出前置裁决未过：审核针对的内容已变更，请重新审核（{} 审核摘要 {}…，当前 {}…——章节正文/结构在过门后被修改）",
                        last.id,
                        digest_head(&last.reviewed_content_digest),
                        digest_head(&current),
                    ),
                });
            }
        }
        // 全部 RT DONE（BLOCKER 必须修完并带关单说明——RT 面单一真源）
        let open = self.open_blockers();
        if !open.is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "导出前置裁决未过：{} 条 RevisionTask 未关单（{}）——BLOCKER 必须修完并带关单说明",
                    open.len(),
                    open.iter().map(|rt| rt.id.as_str()).collect::<Vec<_>>().join(", ")
                ),
            });
        }
        Ok(())
    }
}

/// export_allowed 缺门时的 attempted 面（点名缺哪门——EState 的唯一字符串
/// 载体，沿用「方法名 + 语义注记」的既有口径）。
fn missing_gate_attempted(gate: Gate) -> &'static str {
    match gate {
        Gate::MatrixApproval => "export_allowed——缺 matrix_approval 门 Approved 记录",
        Gate::OutlineApproval => "export_allowed——缺 outline_approval 门 Approved 记录",
        Gate::PreExportConfirm => "export_allowed——缺 pre_export_confirm 门 Approved 记录",
        // AddendumWaiver 不入 export_allowed 的三门遍历（#131：它是 L3 阻塞的
        // 人工确认出口，不是导出门序的一环）——显式点名而非通配，保穷尽性
        Gate::AddendumWaiver => "export_allowed——addendum_waiver 不属导出门序（不可达）",
    }
}

/// 摘要展示头（前 8 位十六进制——正常 sha256 hex 恒 64 字符；异形/损坏
/// 快照短于 8 位时原样展示，不 panic）。
fn digest_head(d: &str) -> &str {
    d.get(..8).unwrap_or(d)
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParseStatus;
    use crate::Requirement;
    use crate::outline::{OutlineNode, SectionStatus};
    use std::collections::HashMap;

    /// 审核闭环基座：项目 + 文件 + 1 条 REQ（真实状态链走到
    /// REQUIREMENTS_EXTRACTED——只有最后一段拨位）。
    fn store_review_base() -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("审核闭环项目", "ZFCG-2026-GK-0707", "明标", "ext-key-t7").unwrap();
        s.upsert_file("A-招标文件.docx", "招标文件", "sha-t7", 31, 12, 120, 0,
                      ParseStatus::Ok, "").unwrap();
        s.set_requirements(vec![Requirement {
            id: "REQ-0001".into(),
            category: "资格条件".into(),
            clause_no: Some("3.2.1".into()),
            anchors: vec![crate::Anchor {
                file: "A".into(),
                kind: "para".into(),
                locator: "第三章".into(),
                excerpt: "独立承担民事责任能力的法人营业执照".into(),
                    block_idx: None,
                    page: None,
                    line: None
            }],
            params: HashMap::new(),
            source_file: "A".into(),
        }]).unwrap();
        s
    }

    /// 状态拨位（测试脚手架）：MATRIX_READY/EVIDENCE_BOUND/DRAFTING 在
    /// next_allowed 状态机已定义、尚无公开写入器（M4+ 接线批次）——测试经
    /// crate 内私有字段布置 fixture（仅测试脚手架，生产面无此通道）。
    fn force_status(s: &mut DomainStore, st: ProjectStatus) {
        s.project.as_mut().unwrap().status = st;
    }

    /// 基座 + 拨到 MATRIX_READY（矩阵门状态窗的起点）。
    fn store_at_matrix_ready() -> DomainStore {
        let mut s = store_review_base();
        force_status(&mut s, ProjectStatus::MatrixReady);
        s
    }

    /// 三门全过的完整走门脚手架（各门 human: 复审人 Approved；第三门携带
    /// findings——返回第三门回执）。
    fn walk_three_gates(
        s: &mut DomainStore,
        pre_export_findings: Vec<(Severity, String, String, Option<String>)>,
    ) -> ReviewReceipt {
        s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("zhang"), Decision::Approved, vec![], None).unwrap();
        s.record_review(Gate::OutlineApproval, &TrustedPrincipal::human("li"), Decision::Approved, vec![], None).unwrap();
        s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("wang"), Decision::Approved, pre_export_findings, None).unwrap()
    }

    /// #180 摘要绑定基座：基座 + 两级章节树（根 + 2 叶，全绑 REQ-0001）——
    /// 过门时点有真实结构面可摘要（digest 涵盖 id/parent/title/level/order/
    /// content；编码契约见 outline::sections_digest）。
    fn store_with_outline() -> DomainStore {
        let mut s = store_review_base(); // REQUIREMENTS_EXTRACTED——set_outline 窗口内
        let leaf = |id: &str, order: u32| OutlineNode {
            id: id.into(),
            parent_id: Some("SEC-0001".into()),
            title: format!("响应章 {id}"),
            level: 2,
            order,
            requirement_ids: vec!["REQ-0001".into()],
            target_chars: 500,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        };
        s.set_outline(vec![
            OutlineNode {
                id: "SEC-0001".into(),
                parent_id: None,
                title: "技术响应".into(),
                level: 1,
                order: 1,
                requirement_ids: vec!["REQ-0001".into()],
                target_chars: 1000,
                status: SectionStatus::Pending,
                dirty: false,
                content: None,
                content_attempt: 0,
            },
            leaf("SEC-0002", 2),
            leaf("SEC-0003", 3),
        ]).unwrap();
        s
    }

    // ============================ 自研测试 ============================

    /// 规格硬验收：三门走完（PreExportConfirm 携 2 Blocker + 1 Warning）→
    /// 2 RT 自动派生（Warning 不派生——只有 BLOCKER 有 RT）→ 未关单禁导出 →
    /// 跳级被拒 → 逐个 assign→advance→complete → 全 Done 后放行。
    #[test]
    fn acceptance_blocker_rt_lifecycle_and_export_gate() {
        let mut s = store_at_matrix_ready();
        // 门一：矩阵定稿（零发现——干净过门）
        let r1 = s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("zhang"), Decision::Approved, vec![], None).unwrap();
        assert_eq!(r1.gate, Gate::MatrixApproval);
        assert!(r1.finding_ids.is_empty());
        assert!(r1.spawned_rt_ids.is_empty());
        // 门二：章节树定稿（1 条 Warning——不派生 RT）
        let r2 = s.record_review(Gate::OutlineApproval, &TrustedPrincipal::human("li"), Decision::Approved, vec![
            (Severity::Warning, "占位符残留".into(), "大纲第 3 章标题占位".into(), Some("SEC-0001".into())),
        ], None).unwrap();
        assert_eq!(r2.finding_ids.len(), 1);
        assert_eq!(r2.spawned_rt_ids.len(), 0, "Warning 不派生 RT——只有 BLOCKER 有 RT");
        // 门三：导出前确认（2 Blocker + 1 Warning）
        let r3 = s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("wang"), Decision::Approved, vec![
            (Severity::Blocker, "金额冲突".into(), "报价总价与大写金额不一致".into(), Some("SEC-0007".into())),
            (Severity::Blocker, "占位符残留".into(), "服务承诺章仍有 {{待定}} 模板占位".into(), None),
            (Severity::Warning, "格式要求".into(), "页眉缺少项目编号".into(), None),
        ], None).unwrap();
        assert_eq!(r3.finding_ids.len(), 3);
        assert_eq!(r3.spawned_rt_ids.len(), 2, "2 条 BLOCKER → 2 个 RT；Warning 不派生");
        assert_eq!(s.findings().len(), 4);
        assert_eq!(s.revision_tasks().len(), 2);
        assert!(s.reviews()[2].at > 0, "at 为 UNIX_EPOCH 秒落钟（no-chrono）");
        // 未关单 → 导出前置裁决拒（E_UNPROCESSABLE，消息含未关单数）
        match s.export_allowed() {
            Err(ToolError::EUnprocessable { reason }) => {
                assert!(reason.contains("2 条"), "消息须含未关单数：{reason}");
            }
            other => panic!("未关单须 E_UNPROCESSABLE，得到 {other:?}"),
        }
        // 跳级先试：Open 直接 Done → 越级拒
        let rt_first = r3.spawned_rt_ids[0].clone();
        assert_eq!(
            s.complete_revision_task(&rt_first, "跳级关单").unwrap_err().code(),
            "E_UNPROCESSABLE"
        );
        // 逐个 assign → advance(InProgress) → complete(resolution)
        // （owner 面开放：人与 AI 均可修单——只有复审人必须 human: 前缀）
        for (i, rt_id) in r3.spawned_rt_ids.iter().enumerate() {
            let owner = if i == 0 { "human:fixer" } else { "ai:glm-7" };
            s.assign_revision_task(rt_id, owner).unwrap();
            s.advance_rt_status(rt_id, RTStatus::InProgress).unwrap();
            s.complete_revision_task(rt_id, "已按补遗 v2 对齐金额与占位文案").unwrap();
        }
        // 全 DONE → 放行
        assert!(s.open_blockers().is_empty());
        assert!(s.export_allowed().is_ok());
        // 每个 DONE RT 都带非空关单说明（诚实状态的结构面）
        for rt in s.revision_tasks() {
            assert_eq!(rt.status, RTStatus::Done);
            assert!(rt.resolution.as_deref().map(|r| !r.trim().is_empty()).unwrap_or(false));
        }
    }

    /// 规格硬验收：绕过门/乱序记审 → EState typed reject（零副作用）。
    #[test]
    fn bypassing_gates_typed_reject() {
        // 早期状态：矩阵门状态窗未开（REQUIREMENTS_EXTRACTED——矩阵未产出）
        let mut early = store_review_base();
        let v = early.content_version();
        assert_eq!(
            early.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("zhang"), Decision::Approved, vec![], None)
                .unwrap_err().code(),
            "E_STATE"
        );
        assert_eq!(early.content_version(), v, "拒绝零副作用");
        assert!(early.reviews().is_empty());

        // 门序强制：第三门直跳（前两门未过）→ EState 点名缺 matrix_approval
        let mut s = store_at_matrix_ready();
        let err = s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("wang"), Decision::Approved, vec![], None).unwrap_err();
        match &err {
            ToolError::EState { attempted, .. } =>
                assert!(attempted.contains("matrix_approval"), "须点名第一缺门：{attempted}"),
            other => panic!("须 EState，得到 {other:?}"),
        }
        // 乱序：大纲门先于矩阵门 → EState
        assert_eq!(
            s.record_review(Gate::OutlineApproval, &TrustedPrincipal::human("li"), Decision::Approved, vec![], None)
                .unwrap_err().code(),
            "E_STATE"
        );
        assert!(s.reviews().is_empty(), "乱序记审零入账");
        // 矩阵门过后、大纲门未过 → 第三门仍拒（点名缺 outline_approval）
        s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("zhang"), Decision::Approved, vec![], None).unwrap();
        let err = s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("wang"), Decision::Approved, vec![], None).unwrap_err();
        match &err {
            ToolError::EState { attempted, .. } =>
                assert!(attempted.contains("outline_approval"), "须点名缺门：{attempted}"),
            other => panic!("须 EState，得到 {other:?}"),
        }
        assert_eq!(s.reviews().len(), 1, "只有合法矩阵门记录入账");
        // 查询面同口径：三门缺一 → export_allowed EState 点名
        match s.export_allowed() {
            Err(ToolError::EState { attempted, .. }) =>
                assert!(attempted.contains("outline_approval"), "查询面须点名缺门：{attempted}"),
            other => panic!("缺门须 EState，得到 {other:?}"),
        }
    }

    /// #182 可信身份门：Agent/System 的 TrustedPrincipal 不得过门（域层纵深
    /// ——语义迁移自旧「human: 前缀校验」：旧测试断言裸字符串前缀拒收，
    /// 新测试断言非 Human 身份拒收；身份保证本体在服务层 HMAC 验签）。
    #[test]
    fn reviewer_must_be_human_principal() {
        let mut s = store_at_matrix_ready();
        let v = s.content_version();
        for (bad_id, bad_type) in [("glm-4", ActorType::Agent), ("cron", ActorType::System)] {
            // 无 agent 签域的裸构造（scope_is_agent=false）：Agent 用人类签域 → 拒；
            // System 一律拒（#277 二元组 match 的两个拒绝臂）
            let bad = TrustedPrincipal {
                actor_id: bad_id.into(),
                actor_type: bad_type,
                authentication_method: "hmac_receipt_v1".into(),
                approval_receipt: String::new(),
                scope_is_agent: false,
            };
            match s.record_review(Gate::MatrixApproval, &bad, Decision::Approved, vec![], None) {
                Err(ToolError::EUnprocessable { reason }) => {
                    assert!(
                        reason.contains("签域") || reason.contains("身份"),
                        "须指出身份/签域要求：{reason}"
                    );
                    assert!(
                        reason.contains("#277") || reason.contains("system"),
                        "须指向 agent 声明面的正确通道（#277）：{reason}"
                    );
                }
                other => panic!("复审人 {bad_id:?}（{bad_type:?}）必须拒，得到 {other:?}"),
            }
        }
        assert!(s.reviews().is_empty());
        assert_eq!(s.content_version(), v, "拒绝路径零副作用");
        // Human 放行：reviewer 派生值回填 human:{actor_id}（legacy 回显口径）
        s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("wang"), Decision::Approved, vec![], None).unwrap();
        assert_eq!(s.reviews().len(), 1);
        assert_eq!(s.reviews()[0].reviewer, "human:wang");
        // #182：新记录落 TrustedPrincipal（审计留痕——receipt 串在座）
        let p = s.reviews()[0].principal.as_ref().unwrap();
        assert_eq!(p.actor_id, "wang");
        assert_eq!(p.actor_type, ActorType::Human);
    }

    /// #182 legacy 快照兼容：带 reviewer 无 principal 的旧 ReviewRecord JSON
    /// 载入不炸、裁决面宽容（gate_approved 不回溯失效既有 Approved）。
    #[test]
    fn legacy_review_record_without_principal_loads_leniently() {
        let legacy = serde_json::json!({
            "id": "REV-0001",
            "gate": "MATRIX_APPROVAL",
            "reviewer": "human:zhang",
            "decision": "APPROVED",
            "findings": [],
            "at": 1_800_000_000u64,
            "reviewed_content_version": 3,
            "reviewed_content_digest": "deadbeef",
        });
        let rec: ReviewRecord = serde_json::from_value(legacy).unwrap();
        assert!(rec.principal.is_none(), "legacy 快照 principal 缺席 → None");
        assert_eq!(rec.reviewer, "human:zhang");
        // 裁决面宽容：gate_approved 消费既有 Approved（不回溯失效）
        let mut s = store_at_matrix_ready();
        let mut snap = s.to_snapshot();
        snap.reviews.push(rec);
        let restored = DomainStore::restore(snap).unwrap();
        assert!(restored.gate_approved(Gate::MatrixApproval), "legacy Approved 不回溯失效");
    }

    /// 拒审必须有阻断级依据（诚实状态：Warning 不构成拒审）。
    #[test]
    fn rejected_without_blocker_is_dishonest() {
        let mut s = store_at_matrix_ready();
        // 纯 Warning 的 Rejected → EUnprocessable
        let err = s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("wang"), Decision::Rejected, vec![
            (Severity::Warning, "格式要求".into(), "页眉缺项目编号".into(), None),
        ], None).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE");
        // 零 findings 的 Rejected → 同拒
        assert_eq!(
            s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("wang"), Decision::Rejected, vec![], None)
                .unwrap_err().code(),
            "E_UNPROCESSABLE"
        );
        assert!(s.reviews().is_empty() && s.findings().is_empty(), "拒绝路径零副作用");
        // 带 Blocker 的 Rejected 放行且 RT 照常派生（拒审不断关单通道）
        let r = s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("wang"), Decision::Rejected, vec![
            (Severity::Blocker, "金额冲突".into(), "报价总价与大写金额不一致".into(), Some("SEC-0007".into())),
        ], None).unwrap();
        assert_eq!(r.decision, Decision::Rejected);
        assert_eq!(r.spawned_rt_ids.len(), 1, "拒审的 Blocker 照常派生 RT");
        assert_eq!(s.reviews().len(), 1);
        // 空 message 的发现 → 拒（空发现不可审计——findings 形状门的落点）
        let err = s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("li"), Decision::Approved, vec![
            (Severity::Info, "备注".into(), "   ".into(), None),
        ], None).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE");
        assert_eq!(s.findings().len(), 1, "非法发现零入账");
    }

    /// 结构不变量：任意 review 序列后 findings 中 Blocker 数 ≡ revision_tasks
    /// 数（每个 BLOCKER 恰一个 RT——无遗漏无多生；含 Approved 后再 Rejected
    /// 的过程记录路径）。
    #[test]
    fn blocker_rt_structural_invariant() {
        let mut s = store_at_matrix_ready();
        // 轮 1：矩阵门 Rejected（1 Blocker + 1 Warning）
        s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("wang"), Decision::Rejected, vec![
            (Severity::Blocker, "金额冲突".into(), "总价与大写不一致".into(), Some("SEC-0007".into())),
            (Severity::Warning, "格式要求".into(), "页眉缺编号".into(), None),
        ], None).unwrap();
        // 轮 2：矩阵门重审 Approved（零发现——上轮 RT 未修完不阻断过门，
        // 收紧面在 export_allowed 的 RT 判决）
        s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("zhang"), Decision::Approved, vec![], None).unwrap();
        // 轮 3：大纲门 Approved（1 Blocker）
        s.record_review(Gate::OutlineApproval, &TrustedPrincipal::human("li"), Decision::Approved, vec![
            (Severity::Blocker, "占位符残留".into(), "第 5 章标题占位".into(), None),
        ], None).unwrap();
        // 轮 4：第三门 Rejected（1 Blocker）
        s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("wang"), Decision::Rejected, vec![
            (Severity::Blocker, "金额冲突".into(), "报价单税率与正文不一致".into(), None),
        ], None).unwrap();
        // 轮 5：第三门 Approved（1 Blocker + 1 Info）
        s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("wang"), Decision::Approved, vec![
            (Severity::Blocker, "占位符残留".into(), "签署页日期占位".into(), None),
            (Severity::Info, "备注".into(), "建议附电子版目录".into(), None),
        ], None).unwrap();
        // 轮 6：Approved 后的过程记录（第三门再 Rejected + 1 Blocker——
        // Rejected 不撤回 Approved 里程碑，新问题经新 RT 收紧导出面）
        s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("li"), Decision::Rejected, vec![
            (Severity::Blocker, "金额冲突".into(), "大写金额错字".into(), None),
        ], None).unwrap();
        // —— 结构不变量断言 ——
        assert_eq!(s.findings().len(), 7);
        let blockers = s.findings().iter().filter(|f| f.severity == Severity::Blocker).count();
        assert_eq!(blockers, 5);
        assert_eq!(s.revision_tasks().len(), 5, "Blocker 数 ≡ RT 数（结构保证非约定保证）");
        for rt in s.revision_tasks() {
            let f = s.findings().iter().find(|f| f.id == rt.finding_id).unwrap();
            assert_eq!(f.severity, Severity::Blocker, "RT 只从 Blocker 派生");
            assert_eq!(rt.status, RTStatus::Open, "全程未修单——全部保持 Open");
        }
        for f in s.findings().iter().filter(|f| f.severity == Severity::Blocker) {
            assert_eq!(
                s.revision_tasks().iter().filter(|rt| rt.finding_id == f.id).count(),
                1,
                "每个 Blocker 恰一个 RT（无遗漏无多生）"
            );
        }
    }

    /// RT 关单纪律：resolution 必填 + 状态链逐级 + 终态封闭 + 拒绝零副作用。
    #[test]
    fn rt_completion_requires_resolution_and_chain() {
        let mut s = store_at_matrix_ready();
        let r = s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("wang"), Decision::Rejected, vec![
            (Severity::Blocker, "金额冲突".into(), "总价与大写不一致".into(), None),
            (Severity::Blocker, "占位符残留".into(), "签署页占位".into(), None),
        ], None).unwrap();
        let (rt_a, rt_b) = (r.spawned_rt_ids[0].clone(), r.spawned_rt_ids[1].clone());
        // 不存在 → ENotFound
        assert!(matches!(
            s.assign_revision_task("RT-9999", "human:x"),
            Err(ToolError::ENotFound { .. })
        ));
        assert!(matches!(
            s.complete_revision_task("RT-9999", "x"),
            Err(ToolError::ENotFound { .. })
        ));
        // rt_a：认领 → 开工 →（空白 resolution 拒）→ 合法关单 → 终态封闭
        s.assign_revision_task(&rt_a, "human:fixer").unwrap();
        s.advance_rt_status(&rt_a, RTStatus::InProgress).unwrap();
        let v = s.content_version();
        assert_eq!(
            s.complete_revision_task(&rt_a, "   ").unwrap_err().code(),
            "E_UNPROCESSABLE"
        );
        assert_eq!(s.content_version(), v, "拒绝路径零副作用");
        s.complete_revision_task(&rt_a, "金额已按补遗 v2 重算对齐").unwrap();
        let v = s.content_version();
        assert_eq!(
            s.advance_rt_status(&rt_a, RTStatus::InProgress).unwrap_err().code(),
            "E_UNPROCESSABLE"
        );
        assert_eq!(
            s.complete_revision_task(&rt_a, "再改一次").unwrap_err().code(),
            "E_UNPROCESSABLE"
        );
        assert_eq!(
            s.assign_revision_task(&rt_a, "human:other").unwrap_err().code(),
            "E_UNPROCESSABLE"
        );
        assert_eq!(s.content_version(), v, "终态三拒全零副作用");
        // rt_b：Open 直接 Done 越级拒（两条路都试）
        assert_eq!(
            s.complete_revision_task(&rt_b, "跳级").unwrap_err().code(),
            "E_UNPROCESSABLE"
        );
        assert_eq!(
            s.advance_rt_status(&rt_b, RTStatus::Done).unwrap_err().code(),
            "E_UNPROCESSABLE"
        );
        // rt_b 合法走完：开工 →（无 resolution 参的 Done 推进也拒）→ 带说明关单
        s.assign_revision_task(&rt_b, "ai:glm-7").unwrap();
        s.advance_rt_status(&rt_b, RTStatus::InProgress).unwrap();
        assert_eq!(
            s.advance_rt_status(&rt_b, RTStatus::Done).unwrap_err().code(),
            "E_UNPROCESSABLE"
        );
        s.complete_revision_task(&rt_b, "占位已按定稿版替换").unwrap();
        assert!(s.open_blockers().is_empty());
    }

    /// 同门重复 Approved：幂等返回既有序列号，既有 RT 不复位（重新过门 ≠ 已修完）。
    #[test]
    fn duplicate_gate_approval_idempotent_no_reset() {
        let mut s = store_at_matrix_ready();
        let r = walk_three_gates(&mut s, vec![
            (Severity::Blocker, "金额冲突".into(), "总价与大写不一致".into(), None),
            (Severity::Blocker, "占位符残留".into(), "签署页占位".into(), None),
        ]);
        // 半途修单：rt_a 关单、rt_b 开工未关
        let (rt_a, rt_b) = (r.spawned_rt_ids[0].clone(), r.spawned_rt_ids[1].clone());
        s.assign_revision_task(&rt_a, "human:f1").unwrap();
        s.advance_rt_status(&rt_a, RTStatus::InProgress).unwrap();
        s.complete_revision_task(&rt_a, "已修").unwrap();
        s.assign_revision_task(&rt_b, "human:f2").unwrap();
        s.advance_rt_status(&rt_b, RTStatus::InProgress).unwrap();
        let shape = (s.findings().len(), s.revision_tasks().len(), s.reviews().len());
        let v = s.content_version();
        // 同门二次 Approved（携同形 findings 重放）→ 幂等返回
        let r2 = s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("wang"), Decision::Approved, vec![
            (Severity::Blocker, "金额冲突".into(), "总价与大写不一致".into(), None),
            (Severity::Blocker, "占位符残留".into(), "签署页占位".into(), None),
        ], None).unwrap();
        assert_eq!(r2.review_id, r.review_id, "返回既有 REV 序号——不新建记录");
        assert_eq!(r2.finding_ids, r.finding_ids);
        assert_eq!(r2.spawned_rt_ids.len(), 2, "回执带首次过门派生的 RT 清单");
        // 零副作用：findings/RT/reviews 不增、content_version 不动（幂等重放）
        assert_eq!((s.findings().len(), s.revision_tasks().len(), s.reviews().len()), shape);
        assert_eq!(s.content_version(), v);
        // 既有 RT 状态不被复位（Done 仍 Done、InProgress 仍 InProgress）
        let ra = s.revision_tasks().iter().find(|t| t.id == rt_a).unwrap();
        assert_eq!((ra.status, ra.resolution.as_deref()), (RTStatus::Done, Some("已修")));
        let rb = s.revision_tasks().iter().find(|t| t.id == rt_b).unwrap();
        assert_eq!(rb.status, RTStatus::InProgress);
        // 导出裁决不受门重开影响：未全关仍拒 → 关掉 rt_b → 放行
        assert!(s.export_allowed().is_err());
        s.complete_revision_task(&rt_b, "已修").unwrap();
        assert!(s.export_allowed().is_ok());
    }

    /// 章节域发现锚原章节；全局发现（section_id=None）锚根项目章 PROJECT。
    #[test]
    fn section_scoped_and_global_findings() {
        let mut s = store_at_matrix_ready();
        let r = s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("wang"), Decision::Rejected, vec![
            (Severity::Blocker, "金额冲突".into(), "总价与大写不一致".into(), Some("SEC-0007".into())),
            (Severity::Blocker, "占位符残留".into(), "全文仍有 {{待定}} 占位".into(), None),
        ], None).unwrap();
        assert_eq!(r.spawned_rt_ids.len(), 2);
        let rts = s.revision_tasks();
        assert_eq!(rts[0].section_id, "SEC-0007", "章节域发现锚原章节 id");
        assert_eq!(rts[1].section_id, "PROJECT", "全局发现锚根项目章 PROJECT");
        // Finding 本体保留原始 section_id（None 不被改写——全局性是发现的
        // 属性，锚 PROJECT 是 RT 派生面的落点口径）
        assert_eq!(s.findings()[0].section_id.as_deref(), Some("SEC-0007"));
        assert_eq!(s.findings()[1].section_id, None);
    }

    // ============================ #180 审核内容绑定（P0-1） ============================

    /// A｜事故重演与封堵：三门全过（落账绑当时摘要）→ set_section_content
    /// 改叶正文（Drafting 全程放行——合法面）→ export_allowed 拒
    /// （E_UNPROCESSABLE「审核针对的内容已变更」——旧审核不再有效）。
    #[test]
    fn pre_export_digest_binds_section_content() {
        let mut s = store_with_outline();
        s.set_section_content("SEC-0002", "第一章正文 v1（审核时点）".into()).unwrap();
        s.set_section_content("SEC-0003", "第二章正文 v1（审核时点）".into()).unwrap();
        force_status(&mut s, ProjectStatus::Drafting);
        let receipt = walk_three_gates(&mut s, vec![]);
        // 落账即绑定：回执与记录都携带过门时点的章节摘要
        assert_eq!(receipt.reviewed_content_digest, s.sections_digest());
        assert_eq!(s.reviews()[2].reviewed_content_digest, s.sections_digest());
        assert!(s.export_allowed().is_ok());
        // 事故形态：v10 过门 → v11 Agent 改叶正文 → 直接导出（此前放行 = 洞）
        s.set_section_content("SEC-0002", "第一章正文 v2（过门后被改）".into()).unwrap();
        match s.export_allowed() {
            Err(ToolError::EUnprocessable { reason }) => {
                assert!(reason.contains("审核针对的内容已变更"), "须点名内容变更：{reason}");
                assert!(reason.contains("请重新审核"), "须给出行动指引：{reason}");
                assert!(reason.contains(&receipt.review_id), "须点名失效的 REV：{reason}");
            }
            other => panic!("改叶正文后导出须 E_UNPROCESSABLE，得到 {other:?}"),
        }
    }

    /// B｜无关变更不误伤：过门 → upsert_file 导入无关件（bump 全局
    /// content_version 但不触章节摘要）→ export_allowed 仍放行。这是
    /// 「绑摘要」与「绑全局版本」的分水岭——区分「正文变了」和「别的
    /// 东西变了」。
    #[test]
    fn unrelated_import_does_not_invalidate_review() {
        let mut s = store_with_outline();
        s.set_section_content("SEC-0002", "正文 v1".into()).unwrap();
        force_status(&mut s, ProjectStatus::Drafting);
        walk_three_gates(&mut s, vec![]);
        assert!(s.export_allowed().is_ok());
        let v = s.content_version();
        s.upsert_file("C-答疑纪要.pdf", "补遗", "sha-180-unrelated", 2, 0, 3, 0,
                      ParseStatus::Ok, "无关件导入").unwrap();
        assert_eq!(s.files().len(), 2);
        assert!(s.content_version() > v, "导入确实 bump 了全局版本");
        assert!(s.export_allowed().is_ok(), "无关件不触章节摘要——导出仍放行");
    }

    /// C｜幂等重导：过门 → export_allowed Ok → 连跳 4 态（每次推进 bump
    /// content_version——模拟导出后的状态链）→ 仍 Ok。摘要不绑全局版本，
    /// 「不改正文重复导出」恒放行。
    #[test]
    fn repeat_export_after_version_bumps_still_allowed() {
        let mut s = store_with_outline();
        s.set_section_content("SEC-0002", "正文 v1".into()).unwrap();
        force_status(&mut s, ProjectStatus::Drafting);
        walk_three_gates(&mut s, vec![]);
        assert!(s.export_allowed().is_ok());
        let v = s.content_version();
        for st in [ProjectStatus::Validating, ProjectStatus::Reviewing,
                   ProjectStatus::ReadyToExport, ProjectStatus::Exported] {
            s.advance_project_status(st).unwrap();
        }
        assert_eq!(s.content_version(), v + 4, "4 次状态推进 = 4 次 bump");
        assert!(s.export_allowed().is_ok(), "版本 bump ≠ 内容变更——仍放行");
    }

    /// 改稿后重过门：幂等分支先比摘要——不同则落**新** REV（绑新摘要），
    /// 不被「同门重复 Approved 幂等」吞掉（否则改稿后重过门永远返回旧
    /// 记录 = 死锁）；内容再不变 → 幂等恢复（对最新一轮成立）。
    #[test]
    fn reapproval_after_content_change_creates_new_record() {
        let mut s = store_with_outline();
        s.set_section_content("SEC-0002", "正文 v1".into()).unwrap();
        force_status(&mut s, ProjectStatus::Drafting);
        let first = walk_three_gates(&mut s, vec![]);
        assert!(s.export_allowed().is_ok());
        // 改稿 → 旧门失效
        s.set_section_content("SEC-0002", "正文 v2（改稿）".into()).unwrap();
        assert!(s.export_allowed().is_err(), "改稿后旧审核失效");
        // 重过门：新 REV（非幂等重放），落账绑改后摘要
        let second = s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("wang"), Decision::Approved, vec![], None).unwrap();
        assert_ne!(second.review_id, first.review_id, "落新 REV——幂等分支没吞");
        assert_eq!(s.reviews().len(), 4, "三门 3 条 + 重过门 1 条");
        assert_eq!(second.reviewed_content_digest, s.sections_digest(), "新记录绑改后摘要");
        let rec = s.reviews().iter().find(|r| r.id == second.review_id).unwrap();
        assert!(rec.reviewed_content_version > 0, "辅助账面在座（判定不消费）");
        assert!(s.export_allowed().is_ok(), "重过门后恢复放行");
        // 内容不再变 → 再过门恢复幂等（比对的是**最后一条**同门 Approved）
        let third = s.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("li"), Decision::Approved, vec![], None).unwrap();
        assert_eq!(third.review_id, second.review_id, "摘要未变 → 幂等返回最新一轮记录");
        assert_eq!(s.reviews().len(), 4, "幂等重放零新账");
    }

    /// legacy 快照宽容载入：#180 之前的 reviews 无新字段（serde(default)
    /// 加性迁移——#147 先例）→ 载入成功、digest==""、export_allowed
    /// 判定面跳过（旧数据的洞保留——不因新规则把存量项目锁死在导出闸外；
    /// 重新过门即补上绑定）。
    #[test]
    fn legacy_snapshot_without_digest_still_loads() {
        let mut s = store_with_outline();
        s.set_section_content("SEC-0002", "正文 v1".into()).unwrap();
        force_status(&mut s, ProjectStatus::Drafting);
        walk_three_gates(&mut s, vec![]);
        // 模拟旧快照：剥掉 reviews 的 #180 两字段再反序列化（serde 加载面）
        let mut v = serde_json::to_value(s.to_snapshot()).unwrap();
        for r in v["reviews"].as_array_mut().unwrap() {
            let obj = r.as_object_mut().unwrap();
            obj.remove("reviewed_content_version");
            obj.remove("reviewed_content_digest");
        }
        let legacy: crate::StoreSnapshot = serde_json::from_value(v).unwrap();
        let mut s2 = DomainStore::restore(legacy).unwrap();
        assert!(s2.reviews().iter().all(|r| r.reviewed_content_digest.is_empty()),
                "旧记录摘要为空（legacy 未绑定）");
        // 判定面跳过：即便过门后改正文，空摘要不拒（宽容面——已知保留的洞）
        s2.set_section_content("SEC-0002", "正文 v2（legacy 之后被改）".into()).unwrap();
        assert!(s2.export_allowed().is_ok(), "空摘要跳过判定——存量项目不被锁死");
    }

    // ============================ 借鉴测试（borrow 注明出处） ============================

    /// borrow：mdcore `workbench/tests/persistence_recovery.rs::session_reopen_state_and_continuity`
    /// ——重开后全量状态在位 + 裁决等价 + 新签 id 不碰撞（续跑不回退）。
    #[test]
    fn snapshot_roundtrip_review_state_survives_restart() {
        let mut s = store_at_matrix_ready();
        let r = walk_three_gates(&mut s, vec![
            (Severity::Blocker, "金额冲突".into(), "总价与大写不一致".into(), None),
        ]);
        s.assign_revision_task(&r.spawned_rt_ids[0], "human:f1").unwrap();
        // 半途（1 条 RT 未关）→ 裁决拒
        let verdict_before = s.export_allowed().unwrap_err();
        let snap = s.to_snapshot();
        // —— 重启（快照往返：丢弃原库，从快照恢复）——
        drop(s);
        let mut s2 = DomainStore::restore(snap.clone()).unwrap();
        assert_eq!(s2.to_snapshot(), snap, "重开全量状态全等（findings/reviews/RTs/版本/水位）");
        // 裁决等价：重开后 export_allowed 同形拒绝（同一未关单）
        assert_eq!(s2.export_allowed().unwrap_err(), verdict_before);
        // 续跑：新签 id 不碰撞（水位恢复——新 FND/REV/RT 与既有集互异）
        let existing: Vec<String> = snap.findings.iter().map(|f| f.id.clone())
            .chain(snap.reviews.iter().map(|r| r.id.clone()))
            .chain(snap.revision_tasks.iter().map(|t| t.id.clone()))
            .collect();
        let r2 = s2.record_review(Gate::PreExportConfirm, &TrustedPrincipal::human("wang"), Decision::Rejected, vec![
            (Severity::Blocker, "金额冲突".into(), "复审新发现：大写金额错字".into(), None),
        ], None).unwrap();
        assert!(!existing.contains(&r2.review_id), "新 REV id 不碰撞");
        assert!(r2.finding_ids.iter().all(|f| !existing.contains(f)), "新 FND id 不碰撞");
        assert!(r2.spawned_rt_ids.iter().all(|t| !existing.contains(t)), "新 RT id 不碰撞");
        // 两单全关 → 放行（重开后裁决面仍活——闭环跨重启成立）
        for rt_id in [r.spawned_rt_ids[0].clone(), r2.spawned_rt_ids[0].clone()] {
            s2.advance_rt_status(&rt_id, RTStatus::InProgress).unwrap();
            s2.complete_revision_task(&rt_id, "已修（重启后续跑关单）").unwrap();
        }
        assert!(s2.export_allowed().is_ok());
    }

    /// borrow：mdcore `workbench/tests/persistence_recovery.rs` 的 fail-closed 模式
    /// ——损坏但语法合法的快照也拒载，不猜测（孤儿 RT / 幽灵 FND 引用 /
    /// 重复 id / 水位破坏）。
    #[test]
    fn restore_rejects_orphan_rt_and_duplicate_ids() {
        let mut s = store_at_matrix_ready();
        s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("wang"), Decision::Rejected, vec![
            (Severity::Blocker, "金额冲突".into(), "总价与大写不一致".into(), None),
        ], None).unwrap();
        let base = s.to_snapshot();
        assert!(DomainStore::restore(base.clone()).is_ok(), "对照组：基线快照可载");
        // 孤儿 RT：finding_id 指向不存在的 FND → 拒（半个闭环）
        // （FND-0000：序列从 0001 起——必不存在且后缀 0 不触发水位误报）
        let mut bad = base.clone();
        bad.revision_tasks[0].finding_id = "FND-0000".into();
        assert!(DomainStore::restore(bad).is_err());
        // 重复 REV id → 拒
        let mut bad = base.clone();
        bad.reviews.push(base.reviews[0].clone());
        assert!(DomainStore::restore(bad).is_err());
        // 重复 FND id → 拒
        let mut bad = base.clone();
        bad.findings.push(base.findings[0].clone());
        assert!(DomainStore::restore(bad).is_err());
        // ReviewRecord 引用不存在的 FND → 拒（幽灵引用）
        let mut bad = base.clone();
        bad.reviews[0].findings = vec!["FND-0000".into()];
        assert!(DomainStore::restore(bad).is_err());
        // 水位破坏：id_seq 归零 → 拒（重启后新 id 会碰撞）
        let mut bad = base.clone();
        bad.id_seq = 0;
        assert!(DomainStore::restore(bad).is_err());
    }

    // ------------------------------------------------------------ #277 方案 A：agent 声明过门

    /// 验收 1：agent 带 agent 签域身份过门 → 成功 + 审计账留痕。
    #[test]
    fn agent_pass_with_agent_scope_succeeds_and_leaves_audit_trail() {
        let mut s = store_at_matrix_ready();
        let r = s
            .record_review(
                Gate::MatrixApproval,
                &TrustedPrincipal::agent("agent-01"),
                Decision::Approved,
                vec![],
                Some("单件招标无补遗，矩阵已全量核对（agent 声明——待人工追认）"),
            )
            .unwrap();
        assert_eq!(r.gate, Gate::MatrixApproval);
        assert!(s.gate_approved(Gate::MatrixApproval), "agent 过门推进门序");
        // 审计账：method=agent_declared / human_confirmed=false / reason 在座
        let gp = s.latest_gate_pass(Gate::MatrixApproval).unwrap();
        assert_eq!(gp.actor_id, "agent-01");
        assert_eq!(gp.actor_type, ActorType::Agent);
        assert_eq!(gp.method, "agent_declared");
        assert!(!gp.human_confirmed);
        assert!(gp.reason.contains("agent 声明"));
        // reviewer 回显带 agent: 前缀（不冒充 human:）
        assert_eq!(s.reviews()[0].reviewer, "agent:agent-01");
        // 导出警告判据：human_review_complete=false
        assert!(!s.human_review_complete());
        assert_eq!(s.agent_gate_passes().len(), 1);
    }

    /// 验收 3：Human 用 agent 签域（scope_is_agent=true 的 Human principal）→ 拒。
    #[test]
    fn human_with_agent_scope_is_rejected() {
        let mut s = store_at_matrix_ready();
        let bad = TrustedPrincipal {
            scope_is_agent: true,
            ..TrustedPrincipal::human("wang")
        };
        let err = s
            .record_review(Gate::MatrixApproval, &bad, Decision::Approved, vec![], None)
            .unwrap_err();
        assert!(format!("{err:?}").contains("签域与身份不匹配"));
        assert!(s.gate_passes().is_empty(), "拒绝路径零副作用");
    }

    /// 验收 4：Agent 冒充 Human（无 agent 签域的 Agent principal）→ 仍拒
    /// （现有防线不弱化——scope_is_agent 只能由 verify_agent_receipt 验出）。
    #[test]
    fn agent_without_agent_scope_still_rejected() {
        let mut s = store_at_matrix_ready();
        let bad = TrustedPrincipal {
            actor_type: ActorType::Agent,
            ..TrustedPrincipal::human("imp")
        };
        assert!(
            s.record_review(Gate::MatrixApproval, &bad, Decision::Approved, vec![], None).is_err()
        );
        assert!(s.gate_passes().is_empty());
    }

    /// 验收 7：agent 过门 reason 为空 → 拒（不能空着过）。
    #[test]
    fn agent_pass_requires_non_blank_reason() {
        let mut s = store_at_matrix_ready();
        for absent in [None, Some("   "), Some("")] {
            let err = s
                .record_review(Gate::MatrixApproval, &TrustedPrincipal::agent("a1"),
                    Decision::Approved, vec![], absent)
                .unwrap_err();
            assert!(format!("{err:?}").contains("reason"), "reason 必填：{err:?}");
        }
        assert!(s.gate_passes().is_empty(), "拒绝路径零副作用");
    }

    /// 验收 6：agent 过门 → 真人补 human receipt 追认 → 警告消失 + 两账并存。
    #[test]
    fn human_ratification_after_agent_pass_clears_warning_keeps_both_records() {
        let mut s = store_at_matrix_ready();
        s.record_review(Gate::MatrixApproval, &TrustedPrincipal::agent("agent-01"),
            Decision::Approved, vec![], Some("agent 先行放行（理由在案）")).unwrap();
        assert!(!s.human_review_complete(), "agent 过门后警告在");

        // 追认：真人同门同摘要再过 → 落新记录（不走幂等短路——human 只对
        // human 族记录幂等）
        s.record_review(Gate::MatrixApproval, &TrustedPrincipal::human("chief"),
            Decision::Approved, vec![], None).unwrap();
        // 两账并存（append-only 追认不覆盖）
        let passes: Vec<_> = s
            .gate_passes()
            .iter()
            .filter(|p| p.gate == Gate::MatrixApproval)
            .collect();
        assert_eq!(passes.len(), 2, "追认后两条过门记录并存");
        // 最新一条 = human → 警告消失
        assert!(s.human_review_complete());
        assert!(s.agent_gate_passes().is_empty());
        assert_eq!(s.latest_gate_pass(Gate::MatrixApproval).unwrap().method, "human_receipt");
    }

    /// agent 重试幂等（同 actor 同摘要不刷账）；换 agent actor 落新记录（审计可数）。
    #[test]
    fn agent_repass_idempotent_same_actor_new_record_for_different_actor() {
        let mut s = store_at_matrix_ready();
        let p = TrustedPrincipal::agent("agent-01");
        let r1 = s.record_review(Gate::MatrixApproval, &p, Decision::Approved, vec![],
            Some("首轮")).unwrap();
        let r2 = s.record_review(Gate::MatrixApproval, &p, Decision::Approved, vec![],
            Some("重试")).unwrap();
        assert_eq!(r1.review_id, r2.review_id, "同 actor 同摘要 → 幂等直返");
        assert_eq!(s.gate_passes().len(), 1, "幂等不刷审计账");

        // 换 agent（第二个自动化进程）→ 新记录（Q2：不限制数量但可统计）
        s.record_review(Gate::MatrixApproval, &TrustedPrincipal::agent("agent-02"),
            Decision::Approved, vec![], Some("二号进程")).unwrap();
        assert_eq!(s.gate_passes().len(), 2);
        let n: usize = s.gate_passes().iter().filter(|g| g.actor_id == "agent-02").count();
        assert_eq!(n, 1, "每个 agent 声明可追溯可计数");
    }

    /// 验收 8（gate_passes 面）：无 gate_passes 字段的旧快照 JSON → 空账加载，
    /// human_review_complete=true（无从警告），行为与旧版一致。
    #[test]
    fn legacy_snapshot_without_gate_passes_loads_empty() {
        let snap_json = serde_json::json!({
            "schema_version": crate::STORE_SCHEMA_VERSION,
            "project": null,
            "id_seq": 0,
            // 其余集合缺省——serde(default) 加性迁移
        });
        let snap: crate::StoreSnapshot = serde_json::from_value(snap_json).unwrap();
        let s = DomainStore::restore(snap).unwrap();
        assert!(s.gate_passes().is_empty());
        assert!(s.human_review_complete(), "空账本无从警告");
    }
}
