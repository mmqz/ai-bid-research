//! matrix.rs —— T-2 MatrixRow 需求响应矩阵（issue #11 批次 v3 / V3-2）。
//!
//! 定位：REQ（招标要求）× 响应状态 的权威账本——导出闸门的判定面
//! （**blocking=true 的行未清零 → 禁止导出**，R-03/R-04 闸门）。
//!
//! 状态派生（单一真源 [`derive_matrix_rows`]——工具面/导出门/持久化共用）：
//! - `RESPONDED`（已响应）：显式 override 标记，或「证据在绑 + 章节在位」；
//! - `PARTIAL`（部分）：有证据绑定但响应章节缺位（材料齐、正文未落）；
//! - `PENDING_EVIDENCE`（待补，#245）：仅占位/待核验证据在绑——占位材料
//!   不得计为有效证据（不清阻塞），但待补可见可追踪；
//! - `NOT_RESPONDED`（未响应）：无证据无章节——**必选类（资格条件/否决项）即 blocking**；
//! - `NOT_APPLICABLE`（不适用）：显式 override（带偏离说明——不适用必须给理由）。
//!
//! blocking 语义（Lead T-2 规格 + 契约 mandatory 口径对齐 + #245 占位面）：
//! `(category ∈ {资格条件, 否决项} || agent_mandatory(req))
//!   && status ∈ {NOT_RESPONDED, PENDING_EVIDENCE}`
//! → blocking=true（与 get_compliance_matrix 的 `mandatory` 判定同源——工具面的
//! mandatory 即本门输入）。**PENDING_EVIDENCE 保持 blocking**：占位资质
//! 不得清除合规阻塞（#245 核心语义——「Agent 注册占位资质绑定后阻塞归零」
//! 的缺口收口）；非必选行占位 → PENDING_EVIDENCE 不阻断（业务允许继续
//! 写作，呈现待补状态）。
//!
//! Agent 必选标记接线（AGENT_MANDATORY_NOT_BLOCKING 缺口的收口）：
//! `agent_mandatory(req)` = `params.marked_by == "agent" && params.mandatory == true`
//! （[`agent_mandatory`]）。语义前提是结构性的：`validate_submitted` 只允许
//! requirement 挂在 Agent 判为「需响应」（verdict=requirement）的区间上——
//! noise/规则说明区间挂 requirement 直接拒。因此带 Agent 必选标记的 REQ
//! 必是「Agent 标必选 且 确认为需响应」的条款；正则产物无此二键，
//! 永不进入本面（真实语料 GT 零位移）。
//!
//! 派生 vs 覆写：矩阵行是**派生态**（从 requirements + bindings + sections + overrides
//! 重算，幂等）；人工裁决只经 [`DomainStore::set_matrix_override`]（不适用/偏离说明）
//! 落 overrides 集合——派生输入不变则重算结果不变（可复现）。
//!
//! #241 裁定（Lead，2026-11-05）：「正文已响应但无外部证据」的两分账——
//!
//! ```text
//! 投标人写没写回应？ → 文本事实 → 看正文 → 写了就是写了 → RESPONDED
//! 能不能提交？       → 合规事实 → 看证据 → 没材料就是废标风险 → evidence_missing
//! ```
//!
//! - 状态面：正文有实质响应（归属章节 content 非空）且无证据无占位 → **RESPONDED**
//!   （旧语义判 NOT_RESPONDED 是把两件事混为一谈——冤枉真响应，用户去补写
//!   本来已写好的内容）；
//! - 证据缺口走**正交标记** [`MatrixRow::evidence_required`] /
//!   [`MatrixRow::evidence_missing`]，不塞进 ResponseStatus（第三态会破坏
//!   既有消费方：match 不穷尽、快照迁移、报告分桶——正交是加法，旧快照
//!   旧消费方零影响，与 #255 的 `#[serde(default)]` 迁移面同源纪律）；
//! - 门禁分工：矩阵 blocking 语义**不变**（必选未响应/待补仍阻断导出）；
//!   证据缺口**不阻断导出**（草稿可导出），由提交就绪面
//!   （bid-tools `validate::submission_readiness`——evidence_missing 的
//!   必选条款阻断正式提交）承载；
//! - 正文响应的判定输入是章节 **content**（非仅树归属）——因此 content
//!   变化后必须重算矩阵（draft 路径联建，见 bid-tools draft.rs）。

use crate::evidence::{EvidenceStatus, Evidence};
use crate::outline::OutlineNode;
use crate::principal::TrustedPrincipal;
use crate::{DomainStore, ProjectStatus, Requirement, ToolError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------- 类型面
/// 响应状态（T-2 规格：未响应|已响应|部分|不适用 + #245 待补）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResponseStatus {
    NotResponded,
    Responded,
    Partial,
    NotApplicable,
    /// 待补（#245）：仅占位/待核验证据在绑——占位不清阻塞（必选行仍
    /// blocking），但待补可见可追踪（`pending_evidence_ids` 随行）。
    PendingEvidence,
}

impl ResponseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResponseStatus::NotResponded => "NOT_RESPONDED",
            ResponseStatus::Responded => "RESPONDED",
            ResponseStatus::Partial => "PARTIAL",
            ResponseStatus::NotApplicable => "NOT_APPLICABLE",
            ResponseStatus::PendingEvidence => "PENDING_EVIDENCE",
        }
    }
}

/// 矩阵行（T-2 规格 struct 的 Rust 落地）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatrixRow {
    pub id: String,                     // CMP-xxxx（store 单序列签发）
    pub requirement_id: String,
    /// 响应章节（SEC id——从章节树 requirement_ids 反查）
    pub response_section: Option<String>,
    pub evidence_ids: Vec<String>,
    /// 占位/待核验证据 id（#245——仅 PLACEHOLDER 态证据，待补可追踪面；
    /// serde default：旧快照零位移）
    #[serde(default)]
    pub pending_evidence_ids: Vec<String>,
    /// #241 裁定：本条款是否**要求证明材料**（类别面：资格条件——资格性
    /// 审查的对象就是证明材料本身；∪ 原文要求词面：提供/附/证明材料/
    /// 证书/扫描件等，见 [`EVIDENCE_DEMAND_MARKERS`]）。serde default：
    /// 旧快照零位移。
    #[serde(default)]
    pub evidence_required: bool,
    /// #241 裁定：要求证明材料但无有效证据在绑
    /// （`evidence_required && evidence_ids.is_empty()`）。**正交于 status**——
    /// 正文响应（RESPONDED）与证据缺口（evidence_missing）可同真：文本事实
    /// 与合规事实分账。占位/待核验绑定不算有效证据（#245 口径），不清本
    /// 标记。serde default：旧快照零位移。
    #[serde(default)]
    pub evidence_missing: bool,
    /// #276：★实质性条款（数据源 `params.star`——标记面 Agent 显式判定 ∪
    /// 抽取面★检测两条路径统一，见 [`substantive`]）。**不参与 blocking
    /// 派生**（★响应门的分级是 Lead 裁决面）——本列只做识别与统计
    /// （★响应 N/M，不废标校验从外部文本刮变为引擎可编程）。
    /// serde default：旧快照零位移。
    #[serde(default)]
    pub substantive: bool,
    pub status: ResponseStatus,
    /// 偏离说明（NOT_APPLICABLE 必填——不适用必须给理由，诚实状态）
    pub deviation: Option<String>,
    /// 必选未响应/待补 → 阻断导出（R-03/R-04 闸门 + #245 占位不清阻塞）
    pub blocking: bool,
}

/// 人工覆写面（不适用/偏离说明——派生态之上的唯一人工输入）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatrixOverride {
    pub requirement_id: String,
    pub status: ResponseStatus,
    pub deviation: Option<String>,
    /// #226/#182：覆写人（human:{actor_id} 派生值——legacy 回显面，
    /// 与 AddendumWaiverRecord.principal 同款）。空串 = 旧快照
    /// （receipt 面之前的记录，宽容载入）。
    #[serde(default)]
    pub principal: String,
    /// #182 可信身份：验签后的 TrustedPrincipal（谁豁免了这一行，
    /// 回溯可查——服务层 HMAC 验签注入，域层不重复验签只纵深拦
    /// actor_type）。None = legacy 快照。
    #[serde(default)]
    pub trusted: Option<TrustedPrincipal>,
    /// 覆写时刻（unix epoch 秒——no-chrono 纪律）。
    #[serde(default)]
    pub at: u64,
}

// ---------------------------------------------------------------- 纯派生面
/// 矩阵行派生（单一真源——必选类判定与工具面 mandatory 同源）。
///
/// **id 稳定纪律**：`CMP-{REQ 数字后缀}` 镜像派生（非 store 序列）——重建幂等
/// （重算不产生新 id）、视图/构建态同 id、零序列水位污染；REQ 无数字后缀时
/// 回退 `0000`（extract 产物恒为 REQ-NNNN，仅手造件可能触发）。
pub fn is_mandatory(category: &str) -> bool {
    category == "资格条件" || category == "否决项"
}

/// Agent 标必选（`submit_block_marks` 提交面 `mandatory=true` 的落库痕迹——
/// merge.rs 转换时写入 `params.marked_by="agent"` + `params.mandatory=true`）。
///
/// 语义前提（结构性保证，见 [`validate_submitted`](crate::coverage::validate_submitted)）：
/// 只有落在 Agent 判为「需响应」（verdict=requirement）区间内的块才能提交
/// 关键点——noise/规则说明区间挂 requirement 直接拒。因此本函数命中 =
/// 「Agent 标必选 且 确认为需响应」，正是要接进 blocking 面的人群；
/// 正则产物（含被 veto_rule.rs 降级的规则说明）无此二键，永不受影响。
///
/// 非必选（mandatory=false）与缺键（正则/手造件）都返回 false——
/// 口径与 [`is_mandatory`] 取并集后进入 [`derive_matrix_rows`] 的 blocking 派生。
pub fn agent_mandatory(r: &Requirement) -> bool {
    r.params.get("marked_by").and_then(|v| v.as_str()) == Some("agent")
        && r.params.get("mandatory").and_then(|v| v.as_bool()) == Some(true)
}

/// #276：★实质性条款（两条路径统一派生面）。
///
/// 数据源 `params.star == true`，两个写入方在此汇流：
/// - **标记面**（本 PR）：`submit_block_marks.requirements[].substantive`
///   → merge 转换 `params.star`（bool 都写——Agent 显式判定）；
/// - **抽取面**（#280）：语义表/★字符检测，仅在检测到★时写 true。
///
/// 金标准口径：songling `requirements.reviewed.json` 的 `star` 字段（#251）。
/// **不参与 blocking 派生**（★未响应是否阻断导出是门禁分级，Lead 裁决）——
/// 本函数只做「可编程」：识别 + 统计（★响应 N/M，见 get_compliance_matrix）。
pub fn substantive(r: &Requirement) -> bool {
    r.params.get("star").and_then(|v| v.as_bool()) == Some(true)
}

/// #241 裁定：证据要求词（原文含任一 → 本条款要求证明材料）。
///
/// 词面取 Lead 裁定原文（「提供/附/证明材料/证书/扫描件」）+ 紧邻扩展
/// （复印件/凭证/执照/授权/业绩/社保/纳税——采购文书的高频材料词）。
/// 宁可多报不可漏报（fail-closed）：多报的代价是 readiness 多拦一条
/// （出口明确：补绑证据或人工豁免），漏报的代价是开标废标。
pub const EVIDENCE_DEMAND_MARKERS: &[&str] = &[
    "提供", "附", "证明材料", "证书", "扫描件", "复印件", "凭证", "执照", "授权", "业绩", "社保", "纳税",
];

/// #241 裁定：本条款是否要求证明材料（两信号取或）。
///
/// 1. **类别面**：资格条件 → 恒真——资格性审查的审查对象就是证明材料
///    （营业执照/资质证书/社保证明），不依赖词面；
/// 2. **原文要求词面**（任何类别）：锚点 excerpt（含 Agent 提交的
///    statement——merge.rs 双写 params.statement 与 excerpt）含
///    [`EVIDENCE_DEMAND_MARKERS`] 任一词。否决项**不按类别一刀切**——
///    否决项大量是行为承诺（签字盖章/保证金到账），其材料要求由词面
///    捕捉（「须提供保证金凭证」命中「提供/凭证」），避免把行为承诺
///    也强制要证据的过收口。
pub fn requires_evidence(r: &Requirement) -> bool {
    if r.category == "资格条件" {
        return true;
    }
    let text_hit = |s: &str| EVIDENCE_DEMAND_MARKERS.iter().any(|m| s.contains(m));
    r.anchors.iter().any(|a| text_hit(&a.excerpt))
        || r.params.get("statement").and_then(|v| v.as_str()).map(text_hit).unwrap_or(false)
}

fn derived_cmp_id(requirement_id: &str) -> String {
    let suffix = requirement_id.rsplit('-').next().unwrap_or("0000");
    if suffix.chars().all(|c| c.is_ascii_digit()) && !suffix.is_empty() {
        format!("CMP-{suffix}")
    } else {
        format!("CMP-0000-{requirement_id}")
    }
}

/// 从 (requirements, bindings, sections, overrides) 派生全量矩阵行。
///
/// #245 绑定三分输入：`effective`（真实且有效的证据绑定——计为响应材料）
/// 与 `pending`（占位/待核验证据绑定——待补可见，**不计为有效证据**）。
/// 同一 EV 同时出现在两表属调用方错误（服务层按证据 tier 唯一分流）。
///
/// - 行序 = requirements 序（稳定——REQ 集合幂等合并后顺序确定）；
/// - id = `CMP-{REQ 后缀}` 派生稳定（见 [`derived_cmp_id`]）；
/// - 12,000 行量级：全量派生 O(N+B+S)，无逐行查找平方面。
pub fn derive_matrix_rows(
    reqs: &[Requirement],
    effective: &[(String, String)], // (evidence_id, requirement_id)——真实有效证据
    pending: &[(String, String)],   // (evidence_id, requirement_id)——占位/待核验
    sections: &[OutlineNode],
    overrides: &HashMap<String, MatrixOverride>,
) -> Vec<MatrixRow> {
    // 绑定与章节的反查索引（一次建表，避免逐 REQ 全扫）
    let mut ev_by_req: HashMap<&str, Vec<&str>> = HashMap::new();
    for (ev_id, req_id) in effective {
        ev_by_req.entry(req_id.as_str()).or_default().push(ev_id.as_str());
    }
    let mut pend_by_req: HashMap<&str, Vec<&str>> = HashMap::new();
    for (ev_id, req_id) in pending {
        pend_by_req.entry(req_id.as_str()).or_default().push(ev_id.as_str());
    }
    // 章节层级索引（PR #24 修复批次）：旧实现 and_modify 闭包内
    // `sections.iter().find(|x| x.id == *cur)` 是 O(S)——嵌套在
    // O(Σ|requirement_ids|) 循环里，真实章节树（S=800-1500 × 12,000 REQ 引用）
    // ≈ 千万级扫描。预建 id→level 表后 and_modify O(1)，语义零变化
    // （find 不到时 unwrap_or(0) 与 get().copied().unwrap_or(0) 同口径）。
    let level_by_sec: HashMap<&str, u8> =
        sections.iter().map(|s| (s.id.as_str(), s.level)).collect();
    let mut sec_by_req: HashMap<&str, &str> = HashMap::new();
    for s in sections {
        for rid in &s.requirement_ids {
            // 首个命中章节为响应位（叶子优先于父？——树内 REQ 唯一归属由 outline 聚合
            // 不变量保证：父聚合 = 子并集，父与子都含同一 REQ 时取**最深层**（叶子）
            sec_by_req.entry(rid.as_str())
                .and_modify(|cur| {
                    let cur_level = level_by_sec.get(*cur).copied().unwrap_or(0);
                    if s.level > cur_level {
                        *cur = &s.id;
                    }
                })
                .or_insert_with(|| &s.id);
        }
    }
    // #241 裁定：章节正文在位索引（REQ 归属章节的 content 非空且 trim 后
    // 非空 = 「写了就是写了」的文本事实判定）。与 sec_by_req 同一「最深层
    // 章节」口径——正文响应看的是 REQ 的实际响应位（叶），不是父聚合位。
    let content_by_sec: HashMap<&str, bool> = sections.iter()
        .map(|s| (
            s.id.as_str(),
            s.content.as_deref().map(|c| !c.trim().is_empty()).unwrap_or(false),
        ))
        .collect();
    reqs.iter().map(|r| {
        let evs = ev_by_req.get(r.id.as_str()).cloned().unwrap_or_default();
        let pends = pend_by_req.get(r.id.as_str()).cloned().unwrap_or_default();
        let sec = sec_by_req.get(r.id.as_str()).copied();
        // 状态派生：override > 真实证据+章节 > 占位 > 正文实写 > 缺失
        // （#245 三分：真实且有效 / 占位待核验 / 缺失；#241 裁定新增：
        // 正文有实质响应且无证据无占位 → RESPONDED——文本事实与合规
        // 事实分账，证据缺口由 evidence_missing 正交承载）
        let (status, deviation) = match overrides.get(&r.id) {
            Some(o) => (o.status, o.deviation.clone()),
            None => {
                let has_ev = !evs.is_empty();
                let has_pending = !pends.is_empty();
                let has_sec = sec.is_some();
                // #241 裁定：正文实质在位（归属章节 content 非空）。章节在
                // 树但未写（content None/空白）不称「实质响应」——写了就是
                // 写了，没写就是没写。
                let body_responded = sec
                    .map(|id| content_by_sec.get(id).copied().unwrap_or(false))
                    .unwrap_or(false);
                let st = if has_ev {
                    // 真实证据在位——占位绑定只进 pending_evidence_ids 追踪面，
                    // 不降级状态（真实与占位并存 = 真实为准，待补仍可追踪）
                    if has_sec {
                        ResponseStatus::Responded
                    } else {
                        ResponseStatus::Partial
                    }
                } else if has_pending {
                    // 仅占位材料——待补态（章节在位与否不改变「证据未满足」；
                    // #245 语义不变：占位不清阻塞）
                    ResponseStatus::PendingEvidence
                } else if body_responded {
                    // #241 裁定：正文有实质响应 → RESPONDED（文本事实）；
                    // 证据缺口由 evidence_missing 正交标记承载（合规事实），
                    // 由提交就绪面（submission_readiness）拦截，不压进状态。
                    ResponseStatus::Responded
                } else {
                    ResponseStatus::NotResponded
                };
                (st, None)
            }
        };
        // #241 裁定：证据要求与缺口（正交标记——与状态分账）。
        // evidence_ids 只含有效证据（split_live_bindings 已按 tier 分流，
        // 占位进 pending 不进 effective）——占位绑定不清缺口（#245 口径）。
        let evidence_required = requires_evidence(r);
        let evidence_missing = evidence_required && evs.is_empty();
        // 必选面 = 类别阻断面（资格条件/否决项）∪ Agent 必选标记——
        // 未响应**或仅占位待补**时阻断（#245：占位资质不得清除合规阻塞；
        // 已响应/部分/人工覆写即放行——真实材料在位时占位只是追踪面）。
        let blocking = (is_mandatory(&r.category) || agent_mandatory(r))
            && matches!(
                status,
                ResponseStatus::NotResponded | ResponseStatus::PendingEvidence
            );
        MatrixRow {
            id: derived_cmp_id(&r.id),
            requirement_id: r.id.clone(),
            response_section: sec.map(|s| s.to_string()),
            evidence_ids: evs.into_iter().map(|e| e.to_string()).collect(),
            pending_evidence_ids: pends.into_iter().map(|e| e.to_string()).collect(),
            evidence_required,
            evidence_missing,
            substantive: substantive(r),
            status,
            deviation,
            blocking,
        }
    }).collect()
}

// ---------------------------------------------------------------- DomainStore 扩面（T-2 矩阵操作）
impl DomainStore {
    /// 矩阵只读面（空 = 未构建——build_matrix 幂等填充）。
    pub fn matrix(&self) -> &[MatrixRow] {
        &self.matrix
    }

    pub fn matrix_overrides(&self) -> &HashMap<String, MatrixOverride> {
        &self.matrix_overrides
    }

    /// 构建/重建矩阵（幂等派生态——requirements/bindings/sections 任一变化后重跑）。
    ///
    /// 状态门：REQUIREMENTS_EXTRACTED+（无条款无矩阵）；重算发生实际变化 → content_version+1。
    /// 首次构建成功 → 项目推进 MATRIX_READY（#59 阶段 1：矩阵就绪是起草
    /// （draft_response_section）的硬前置——此前 MatrixReady 无推进通道，
    /// 测试靠 force_status 绕过状态机；现在构建即就绪，语义闭环）。
    /// 返回 (行数, blocking 行数)。
    pub fn build_matrix(&mut self) -> Result<(usize, usize), ToolError> {
        let p = self.project()?;
        if matches!(p.status, ProjectStatus::Draft | ProjectStatus::FilesImported) {
            return Err(ToolError::EState {
                current: p.status,
                attempted: "build_matrix",
            });
        }
        if self.requirements.is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "条款集为空（先 extract_requirements——空矩阵无意义）".into(),
            });
        }
        let was_first_build = p.status == ProjectStatus::RequirementsExtracted;
        // #245 绑定分流：按证据 tier 分为有效（真实）与待补（占位）两束——
        // 占位绑定进 pending 面（待补可见），不计为响应材料。
        let (effective, pending) = self.split_live_bindings();
        let overrides = self.matrix_overrides.clone();
        let rows = derive_matrix_rows(&self.requirements, &effective, &pending, &self.sections, &overrides);
        let blocking = rows.iter().filter(|r| r.blocking).count();
        let changed = rows != self.matrix;
        self.matrix = rows;
        if changed {
            if let Some(p) = self.project.as_mut() {
                p.content_version += 1;
            }
        }
        // 首建推进（RequirementsExtracted → MatrixReady；重跑时已在
        // MatrixReady+，不动——幂等）
        if was_first_build {
            if let Some(p) = self.project.as_mut() {
                p.status = ProjectStatus::MatrixReady;
            }
        }
        Ok((self.matrix.len(), blocking))
    }

    /// #226：覆写后重算矩阵（把「人工裁决」立刻反映到 blocking 面）。
    ///
    /// 为什么单独一个入口：`set_matrix_override` 只往 overrides 表写一行，
    /// `matrix` 是派生面、不会自动跟着变。调用方如果只调 set 不重算，
    /// `export_blockers()` 看到的仍是旧 blocking——那是 #226 死锁真正
    /// 卡住的地方（不只是「没有工具」，而是「就算有工具也不会生效」）。
    ///
    /// 语义 = `build_matrix`（幂等派生），单独命名是为了让调用点读得出
    /// 「这次重算是被人工裁决触发的」。
    pub fn rebuild_matrix_for_override(&mut self) -> Result<(usize, usize), ToolError> {
        self.build_matrix()
    }

    /// 人工覆写（不适用/偏离——NOT_APPLICABLE 必须给理由，诚实状态）。
    ///
    /// #226/#182 验签面：principal 由服务层 receipt 验签注入（宿主签发，
    /// Agent 不得伪造）；域层纵深拦 actor_type——非 Human 拒（与
    /// record_addendum_waiver 同款，真正的身份保证在服务层 HMAC）。
    pub fn set_matrix_override(&mut self, requirement_id: String,
                               status: ResponseStatus, deviation: Option<String>,
                               principal: &TrustedPrincipal)
        -> Result<(), ToolError>
    {
        self.project()?;
        if principal.actor_type != crate::principal::ActorType::Human {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "覆写人 {:?} 是 {} 身份（#182：矩阵覆写只认 Human——把必选条款标 NOT_APPLICABLE 是豁免动作，Agent 不得自免单）",
                    principal.actor_id,
                    principal.actor_type.as_str()
                ),
            });
        }
        if status == ResponseStatus::NotApplicable
            && deviation.as_deref().map(str::trim).unwrap_or("").is_empty()
        {
            return Err(ToolError::EUnprocessable {
                reason: "NOT_APPLICABLE 必须给偏离说明（不适用不给理由 = 不可审计的豁免）".into(),
            });
        }
        if !self.requirements.iter().any(|r| r.id == requirement_id) {
            return Err(ToolError::ENotFound {
                what: format!("requirement {requirement_id} 不存在"),
            });
        }
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.matrix_overrides.insert(requirement_id.clone(), MatrixOverride {
            requirement_id, status, deviation,
            // legacy 回显面从可信身份派生（human:{actor_id}）——不另造第二套人设体系
            principal: format!("human:{}", principal.actor_id),
            trusted: Some(principal.clone()),
            at,
        });
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }

    /// 导出阻断清单（M5 export 工具的硬闸门输入——本批以领域门交付）。
    ///
    /// blocking 行未清零 → `check_export_ready` E_UNPROCESSABLE（逐行列出）；
    /// 矩阵未构建 → 同样拒绝（无矩阵 = 未对账，不得导出——诚实状态）。
    pub fn export_blockers(&self) -> Result<Vec<String>, ToolError> {
        self.project()?;
        if self.matrix.is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "矩阵未构建（先 build_matrix——未对账不得导出）".into(),
            });
        }
        Ok(self.matrix.iter()
            .filter(|r| r.blocking)
            .map(|r| format!("{}（{}）: 必选未响应——R-03/R-04 闸门", r.requirement_id, r.id))
            .collect())
    }

    /// 导出就绪裁决（blocking 清零 + 矩阵在位 → Ok；否则 E_UNPROCESSABLE 逐行列出）。
    pub fn check_export_ready(&self) -> Result<(), ToolError> {
        let blockers = self.export_blockers()?;
        if blockers.is_empty() {
            Ok(())
        } else {
            Err(ToolError::EUnprocessable {
                reason: format!("导出被阻断：{} 条必选未响应（{}）", blockers.len(), blockers.join("; ")),
            })
        }
    }

    /// 只读派生视图（get_compliance_matrix 工具面——不落库不 bump 版本，
    /// 与 build_matrix 同一派生真源；EVIDENCE 过滤：EXPIRED/SUPERSEDED 证据不计入响应面）。
    pub fn matrix_view(&self) -> Vec<MatrixRow> {
        // #245：live 绑定按 tier 分流（占位 → pending 面——待补可见不清阻塞）
        let (effective, pending) = self.split_live_bindings();
        let overrides = self.matrix_overrides.clone();
        derive_matrix_rows(&self.requirements, &effective, &pending, &self.sections, &overrides)
    }

    /// 活跃绑定按证据 tier 分流（#245 单一分流真源——build_matrix 与
    /// matrix_view 共用，两入口口径永不分叉）。
    ///
    /// - 活跃面：EXPIRED/SUPERSEDED 证据的绑定两束都不进（有效性维度，
    ///   既有语义不变——死绑定不计响应也不计待补）；
    /// - 有效束：Active/Expiring 且**非占位**（真实证据——计为响应材料）；
    /// - 待补束：Active/Expiring 且**占位**（PLACEHOLDER——待补可见，
    ///   不计为有效证据，进 `pending_evidence_ids` 追踪面）。
    fn split_live_bindings(&self) -> (Vec<(String, String)>, Vec<(String, String)>) {
        let mut effective = Vec::with_capacity(self.evidence_bindings.len());
        let mut pending = Vec::new();
        for b in &self.evidence_bindings {
            let Some(ev) = self.evidences.iter().find(|e| e.id == b.evidence_id) else {
                continue; // 幽灵绑定（EV 已删除）——两束都不进
            };
            if !matches!(ev.status, EvidenceStatus::Active | EvidenceStatus::Expiring) {
                continue; // 死绑定（EXPIRED/SUPERSEDED）——两束都不进
            }
            let pair = (b.evidence_id.clone(), b.requirement_id.clone());
            if ev.is_placeholder() {
                pending.push(pair);
            } else {
                effective.push(pair);
            }
        }
        (effective, pending)
    }
}

/// 证据列组装素材（契约 §4 `rows[].evidence:[{ev_id, title, status, valid_until,
/// verification_tier}]`——#245：验证态随行回显，占位件 PLACEHOLDER 可辨）。
pub fn evidence_column(evidences: &[Evidence], ev_ids: &[String]) -> Vec<serde_json::Value> {
    ev_ids.iter().filter_map(|id| {
        evidences.iter().find(|e| &e.id == id).map(|e| serde_json::json!({
            "ev_id": e.id,
            "title": e.title,
            "status": e.status.as_str(),
            "valid_until": e.valid_until,
            "verification_tier": e.verification_tier.map(|t| t.as_str()),
        }))
    }).collect()
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{EvidenceBinding, EvidenceStatus};
    use crate::outline::{OutlineNode, SectionStatus};
    use crate::ParseStatus;

    fn req(id: &str, category: &str) -> Requirement {
        // 去重键 = 锚点（file|locator|excerpt）——合成件必须锤唯一锚点，否则
        // set_requirements 的 dedupe_hash 幂等合并会把空锚件归并为一条
        Requirement {
            id: id.into(), category: category.into(), clause_no: None,
            anchors: vec![crate::Anchor {
                file: "A".into(), kind: "para".into(),
                locator: format!("第三章 > {id}"),
                excerpt: format!("{id} 条款原文（合成矩阵件）"),
            }],
            params: HashMap::new(), source_file: "A".into(),
        }
    }

    fn sec(id: &str, level: u8, reqs: &[&str]) -> OutlineNode {
        OutlineNode {
            id: id.into(), parent_id: None, title: id.into(), level, order: level as u32,
            requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
            target_chars: 100, status: SectionStatus::Pending, dirty: false,
            content: None, content_attempt: 0,
        }
    }

    fn store_with_matrix_base(n_reqs: usize) -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("矩阵项目", "C-MTX", "明标", "mtx-key").unwrap();
        s.upsert_file("A.docx", "招标文件", "sha-m", 5, 1, 50, 0, ParseStatus::Ok, "").unwrap();
        let cats = ["资格条件", "否决项", "评分项", "技术参数", "商务要求"];
        let reqs: Vec<Requirement> = (0..n_reqs)
            .map(|i| req(&format!("REQ-{:04}", i + 3), cats[i % cats.len()]))
            .collect();
        s.set_requirements(reqs).unwrap();
        s
    }

    /// Agent 路径 REQ（merge.rs 转换产物形状：params.marked_by + params.mandatory）。
    fn agent_req(id: &str, category: &str, mandatory: bool) -> Requirement {
        let mut r = req(id, category);
        r.params.insert("marked_by".into(), serde_json::json!("agent"));
        r.params.insert("mandatory".into(), serde_json::json!(mandatory));
        r
    }

    /// #276：★实质性派生面——两条写入路径（标记面 bool 都写 / 抽取面仅★写
    /// true / 正则无键）在 `== true` 判定下统一；MatrixRow.substantive 随行；
    /// 不参与 blocking 派生（★响应门分级是 Lead 裁决面）。
    #[test]
    fn substantive_two_write_paths_unified_derivation() {
        // 标记面：Agent 显式判★（params.star = true）
        let mut marked_star = req("REQ-0001", "技术参数");
        marked_star.params.insert("marked_by".into(), serde_json::json!("agent"));
        marked_star.params.insert("star".into(), serde_json::json!(true));
        // 标记面：Agent 看过判非★（params.star = false——显式审计面）
        let mut marked_plain = req("REQ-0002", "技术参数");
        marked_plain.params.insert("marked_by".into(), serde_json::json!("agent"));
        marked_plain.params.insert("star".into(), serde_json::json!(false));
        // 抽取面（#280 口径）：仅检测到★时写 true
        let mut extracted_star = req("REQ-0003", "商务要求");
        extracted_star.params.insert("star".into(), serde_json::json!(true));
        // 正则形状：无 star 键（绝大多数既有 REQ——serde 旧快照同形态）
        let regex_plain = req("REQ-0004", "商务要求");

        assert!(substantive(&marked_star));
        assert!(!substantive(&marked_plain), "显式 false ≠ ★（看过判非★）");
        assert!(substantive(&extracted_star), "抽取面★检测 → 统一派生 true");
        assert!(!substantive(&regex_plain), "无键 = 非★（缺省语义）");

        let rows = derive_matrix_rows(
            &[marked_star, marked_plain, extracted_star, regex_plain],
            &[], &[], &[], &HashMap::new(),
        );
        assert!(rows[0].substantive && rows[2].substantive);
        assert!(!rows[1].substantive && !rows[3].substantive);
        // ★未响应不改变 blocking（本测试四条均非阻断面类别/必选——blocking 全 false；
        // 若未来★接门，由 Lead 裁定后另测——本断言锁定「不参与派生」现状）
        assert!(rows.iter().all(|r| !r.blocking));
    }

    /// #276：旧快照零位移——无 substantive 键的 MatrixRow JSON 反序列化
    /// 缺省 false（serde default；快照互操作不因加列破）。
    #[test]
    fn matrix_row_substantive_old_snapshot_zero_shift() {
        let old_json = serde_json::json!({
            "id": "CMP-0001",
            "requirement_id": "REQ-0001",
            "response_section": null,
            "evidence_ids": [],
            "pending_evidence_ids": [],
            "evidence_required": false,
            "evidence_missing": false,
            "status": "NOT_RESPONDED",
            "deviation": null,
            "blocking": false
        });
        let row: MatrixRow = serde_json::from_value(old_json).expect("旧快照反序列化");
        assert!(!row.substantive, "旧快照 substantive 缺省 false（零位移）");
    }

    /// Agent 必选标记接线：类别不在阻断面（资格条件/否决项）的 Agent 必选条款，
    /// 未响应 → blocking；已响应（证据+章节）→ 放行；人工覆写 → 放行。
    #[test]
    fn agent_mandatory_wires_blocking_outside_category_face() {
        let reqs = vec![
            agent_req("REQ-0001", "商务要求", true),   // 未响应 → blocking（接线主断言）
            agent_req("REQ-0002", "技术参数", true),   // 证据+章节 → RESPONDED → 放行
            agent_req("REQ-0003", "评分项", true),     // 人工覆写不适用（带理由）→ 放行
        ];
        let bindings = vec![("EV-0001".to_string(), "REQ-0002".to_string())];
        let sections = vec![sec("SEC-0001", 1, &["REQ-0002"])];
        let mut overrides = HashMap::new();
        overrides.insert("REQ-0003".to_string(), MatrixOverride {
            requirement_id: "REQ-0003".into(),
            status: ResponseStatus::NotApplicable,
            deviation: Some("评分项本地化承诺不适用（人工 receipt 覆写）".into()),
            principal: String::new(),
            trusted: None,
            at: 0,
        });
        let rows = derive_matrix_rows(&reqs, &bindings, &[], &sections, &overrides);
        assert!(rows[0].blocking, "Agent 必选（商务要求）未响应 → blocking（缺口收口）");
        assert_eq!(rows[0].status, ResponseStatus::NotResponded);
        assert!(!rows[1].blocking, "Agent 必选已响应（证据+章节）→ 放行");
        assert_eq!(rows[1].status, ResponseStatus::Responded);
        assert!(!rows[2].blocking, "Agent 必选经人工覆写（不适用+理由）→ 放行");
        assert_eq!(rows[2].status, ResponseStatus::NotApplicable);
    }

    /// 非误断对照：非必选（mandatory=false）、正则形状（无 marked_by）、
    /// 仅单键（防手造件半标记）都不进 blocking 面——真实语料 GT 零位移的域层保证。
    #[test]
    fn agent_mandatory_negative_face_non_mandatory_and_unmarked() {
        let mut regex_shaped = req("REQ-0002", "商务要求");
        regex_shaped.params.insert("mandatory".into(), serde_json::json!(true)); // 无 marked_by
        let mut half_marked = req("REQ-0003", "其他");
        half_marked.params.insert("marked_by".into(), serde_json::json!("agent")); // 无 mandatory
        let reqs = vec![
            agent_req("REQ-0001", "商务要求", false), // Agent 标非必选
            regex_shaped,
            half_marked,
        ];
        let rows = derive_matrix_rows(&reqs, &[], &[], &[], &HashMap::new());
        assert!(!rows[0].blocking, "Agent 非必选未响应 → 不阻断（评分/商务面不误伤）");
        assert!(!rows[1].blocking, "正则形状（无 marked_by）→ 不受接线影响");
        assert!(!rows[2].blocking, "仅 marked_by 无 mandatory → 不算 Agent 必选");
        // 与类别阻断面并集不打架：类别必选 + Agent 非必选标记 → 仍按类别阻断
        let union = vec![agent_req("REQ-0004", "否决项", false)];
        let rows2 = derive_matrix_rows(&union, &[], &[], &[], &HashMap::new());
        assert!(rows2[0].blocking, "类别阻断面（否决项）不因 Agent 标非必选而放松");
    }

    /// 派生四态 + blocking（必选未响应 = 闸门；非必选未响应不阻断）。
    #[test]
    fn derive_covers_four_states_and_blocking() {
        let reqs = vec![
            req("REQ-0001", "资格条件"),  // 无绑定无章节 → NOT_RESPONDED + blocking
            req("REQ-0002", "评分项"),    // 有证据无章节 → PARTIAL（不阻断）
            req("REQ-0003", "技术参数"),  // 证据+章节 → RESPONDED
            req("REQ-0004", "商务要求"),  // override 不适用（带理由）
        ];
        let bindings = vec![
            ("EV-0009".to_string(), "REQ-0002".to_string()),
            ("EV-0010".to_string(), "REQ-0003".to_string()),
        ];
        let sections = vec![sec("SEC-0011", 1, &["REQ-0003"])];
        let mut overrides = HashMap::new();
        overrides.insert("REQ-0004".to_string(), MatrixOverride {
            requirement_id: "REQ-0004".into(),
            status: ResponseStatus::NotApplicable,
            deviation: Some("本地化服务条款不适用于本项目（招标文件 §8.3 豁免面）".into()),
            // 审计字段不参与派生（derive 只读 status/deviation）——纯函数测试补默认面
            principal: String::new(),
            trusted: None,
            at: 0,
        });
        let rows = derive_matrix_rows(&reqs, &bindings, &[], &sections, &overrides);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].id, "CMP-0001", "id = REQ 后缀镜像派生（稳定）");
        assert_eq!(rows[0].status, ResponseStatus::NotResponded);
        assert!(rows[0].blocking, "资格条件未响应 → blocking");
        assert_eq!(rows[1].status, ResponseStatus::Partial, "有证据无章节 → 部分");
        assert!(!rows[1].blocking, "评分项未完成不阻断导出");
        assert_eq!(rows[2].status, ResponseStatus::Responded);
        assert_eq!(rows[2].response_section.as_deref(), Some("SEC-0011"));
        assert_eq!(rows[2].evidence_ids, vec!["EV-0010".to_string()]);
        assert_eq!(rows[3].status, ResponseStatus::NotApplicable);
        assert!(rows[3].deviation.as_deref().unwrap().contains("豁免"));
    }

    /// T-2 验收主测：12,000 REQ（1330 页件量级）全量有行 + blocking 精确。
    #[test]
    fn build_matrix_at_12000_scale() {
        let mut s = store_with_matrix_base(12_000);
        let (rows, blocking) = s.build_matrix().unwrap();
        assert_eq!(rows, 12_000, "12,000 条 REQ 全部有行（T-2 验收）");
        // 5 类轮转：资格条件+否决项 = 2/5 必选，全未响应
        let expect_blocking = 12_000 * 2 / 5;
        assert_eq!(blocking, expect_blocking, "必选类（资格+否决）未响应全 blocking");
        // 幂等重建：无变化不 bump
        let v = s.content_version();
        let (rows2, blocking2) = s.build_matrix().unwrap();
        assert_eq!((rows2, blocking2), (rows, blocking));
        assert_eq!(s.content_version(), v, "无变化重建不 bump content_version");
        // 导出门：blocking 未清零 → 拒绝
        let err = s.check_export_ready().unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE");
        // 全部绑定证据 + 章节响应 → blocking 清零 → 放行
        for r in s.requirements().to_vec() {
            s.evidence_bindings.push(EvidenceBinding {
                evidence_id: "EV-0003".into(), requirement_id: r.id,
            });
        }
        // #245：build_matrix 经 split_live_bindings 查台账 tier 分流——
        // 绑定指向的证据必须在台账且非占位才计为响应材料（幽灵绑定/
        // 占位绑定不得清阻塞——「无凭据断言清零」正是本 issue 的缺口面）。
        // 本测试验证闸门翻转，补登一件真实证据（Active + CONTENT_MATCHES）。
        push_ev(&mut s, "EV-0003", Some(crate::evidence::VerificationTier::ContentMatches));
        s.set_matrix_override("REQ-0003".into(), ResponseStatus::Responded, None,
                              &TrustedPrincipal::human("域层测试")).ok();
        // 章节树真正落 store（PR #24 修复批次）：旧代码 `let _ = sections;`
        // 构造完即丢弃——「绑定+章节全响应」的验收意图从未被检验（blocking=0
        // 实际全靠绑定→PARTIAL 不阻断达成，章节位恒缺）。set_outline 过树校验
        // + REQ 存在性门后，绑定（→有证据）× 章节（→有响应位）合力达成
        // RESPONDED，闸门放行才名副其实。
        let sections: Vec<OutlineNode> = ["资格条件", "否决项", "评分项", "技术参数", "商务要求"]
            .iter().enumerate()
            .map(|(i, t)| {
                let mut n = sec(&format!("SEC-{:04}", i + 10), 1,
                    &s.requirements().iter().filter(|r| r.category == *t)
                        .map(|r| r.id.as_str()).collect::<Vec<_>>());
                n.order = i as u32 + 10; // 文档序全局唯一（sec 默认 order=level，5 节点会撞号）
                n
            })
            .collect();
        s.set_outline(sections).unwrap();
        let (_, blocking3) = s.build_matrix().unwrap();
        assert_eq!(blocking3, 0, "绑定+章节全响应 → blocking 清零");
        assert!(s.check_export_ready().is_ok(), "闸门放行");
    }

    /// 覆写校验：不适用必须给理由；幽灵 REQ 拒；矩阵未建导出拒。
    #[test]
    fn override_validation_and_unbuilt_gate() {
        let mut s = store_with_matrix_base(5);
        let hp = TrustedPrincipal::human("域层测试");
        let err = s.set_matrix_override("REQ-0003".into(), ResponseStatus::NotApplicable, None, &hp).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "不适用无理由拒");
        let err = s.set_matrix_override("REQ-9999".into(), ResponseStatus::NotApplicable, Some("理由".into()), &hp).unwrap_err();
        assert_eq!(err.code(), "E_NOTFOUND", "幽灵 REQ 拒");
        // 矩阵未建 → 导出拒（未对账不得导出）
        let err = s.check_export_ready().unwrap_err();
        assert!(format!("{err:?}").contains("矩阵未构建"));
        // override 生效路径
        s.set_matrix_override("REQ-0003".into(), ResponseStatus::NotApplicable, Some("豁免理由".into()), &hp).unwrap();
        let _ = s.build_matrix().unwrap();
        let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0003").unwrap();
        assert_eq!(row.status, ResponseStatus::NotApplicable);
        // #226/#182：覆写落账携带审计面（谁、何时）
        let ov = s.matrix_overrides().get("REQ-0003").unwrap();
        assert_eq!(ov.principal, "human:域层测试");
        assert!(ov.trusted.is_some(), "可信身份留痕");
        assert!(ov.at > 0, "覆写时刻落账");
    }

    /// #226/#182：矩阵覆写的身份纵深——非 Human 拒（服务层验签的域层半边）。
    #[test]
    fn override_rejects_non_human_principal() {
        use crate::principal::{ActorType, TrustedPrincipal};
        let mut s = store_with_matrix_base(5);
        let agent = TrustedPrincipal {
            actor_id: "bid-agent".into(),
            actor_type: ActorType::Agent,
            authentication_method: "hmac_receipt_v1".into(),
            approval_receipt: String::new(),
        };
        let err = s.set_matrix_override(
            "REQ-0003".into(), ResponseStatus::NotApplicable,
            Some("Agent 自免单".into()), &agent,
        ).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE");
        assert!(format!("{err:?}").contains("只认 Human"));
        // 拒绝路径零副作用：不落覆写
        assert!(s.matrix_overrides().get("REQ-0003").is_none());
    }

    /// 视图过滤：EXPIRED/SUPERSEDED 证据的绑定不计入响应面（R-03 时序一致性）。
    #[test]
    fn matrix_view_filters_dead_evidence_bindings() {
        let mut s = store_with_matrix_base(3);
        // 台账：EV-A Active / EV-B Expired
        for (id, status) in [("EV-A", EvidenceStatus::Active), ("EV-B", EvidenceStatus::Expired)] {
            s.evidences.push(crate::evidence::Evidence {
                id: id.into(), kind: crate::evidence::EvidenceKind::Certificate,
                title: format!("证-{id}"), source_file: "f.pdf".into(), source_sha256: id.into(),
                locator: crate::locator::EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:1".into() },
                fingerprint: "fnv64:1".into(), valid_from: None, valid_until: Some("2020-01-01".into()),
                subject_name: "S".into(), scope: "".into(), version: 1, status,
                // #117 任务 1 给 Evidence 加的业绩三面（Option——非业绩类
                // 证据不消费；本测试只验时序过滤，None 即可）
                client: None, amount_wan: None, completed_on: None,
                // #147 结构化字段（本测试只验时序过滤——空结构即可）
                fields: Default::default(),
                // #217 正文摘录 + 验证态（本测试只验时序过滤——None 即可）
                excerpt: None, excerpt_locator: None, verification_tier: None,
            });
        }
        s.evidence_bindings.push(EvidenceBinding { evidence_id: "EV-A".into(), requirement_id: "REQ-0003".into() });
        s.evidence_bindings.push(EvidenceBinding { evidence_id: "EV-B".into(), requirement_id: "REQ-0004".into() });
        let view = s.matrix_view();
        let r3 = view.iter().find(|r| r.requirement_id == "REQ-0003").unwrap();
        let r4 = view.iter().find(|r| r.requirement_id == "REQ-0004").unwrap();
        assert_eq!(r3.evidence_ids, vec!["EV-A".to_string()], "活证据计入响应面");
        assert_eq!(r4.evidence_ids.len(), 0, "过期证据绑定被视图过滤（R-03）");
        assert_eq!(r4.status, ResponseStatus::NotResponded, "死绑定不算响应");
        // 视图不污染 id 序列（派生 id 零序列消耗）
        let seq_before = s.to_snapshot().id_seq;
        let _ = s.matrix_view();
        assert_eq!(s.to_snapshot().id_seq, seq_before, "只读视图零序列污染");
    }

    // ============================ #245：占位证据不清阻塞（PENDING_EVIDENCE 态） ============================

    /// 构造带 tier 的台账证据（直推 evidences——绕过登记面，本组测试只验矩阵派生）。
    fn push_ev(s: &mut DomainStore, id: &str, tier: Option<crate::evidence::VerificationTier>) {
        s.evidences.push(crate::evidence::Evidence {
            id: id.into(), kind: crate::evidence::EvidenceKind::Qualification,
            title: format!("证-{id}"), source_file: "f.pdf".into(), source_sha256: id.into(),
            locator: crate::locator::EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:1".into() },
            fingerprint: format!("fnv64:{id}").to_lowercase().replace("ev-", ""),
            valid_from: None, valid_until: None,
            subject_name: "S".into(), scope: "".into(), version: 1,
            status: EvidenceStatus::Active,
            client: None, amount_wan: None, completed_on: None, fields: Default::default(),
            excerpt: None, excerpt_locator: None, verification_tier: tier,
        });
    }

    /// #245 核心断言：占位资质绑定后，资格条件的 blocking **不归零**——
    /// 状态从 NOT_RESPONDED 翻为 PENDING_EVIDENCE（待补可见），但阻断保留。
    /// 这是事故缺口（Agent 注册占位资质绑定 → 阻塞静默清零）的域层收口。
    #[test]
    fn placeholder_binding_keeps_mandatory_blocking() {
        let mut s = store_with_matrix_base(5);
        push_ev(&mut s, "EV-PH", Some(crate::evidence::VerificationTier::Placeholder));
        // 占位证据绑到全部条款（事故形态：Agent 批量绑定占位资质）
        for r in s.requirements().to_vec() {
            s.evidence_bindings.push(EvidenceBinding {
                evidence_id: "EV-PH".into(), requirement_id: r.id,
            });
        }
        let (_, blocking) = s.build_matrix().unwrap();
        assert_eq!(blocking, 2, "5 条轮转中资格条件+否决项 = 2 条必选——占位绑定后仍全部 blocking（事故缺口收口）");
        let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0003").unwrap();
        assert_eq!(row.status, ResponseStatus::PendingEvidence, "仅占位绑定 → 待补态（非 RESPONDED）");
        assert!(row.blocking, "资格条件行占位绑定 → 仍阻断");
        assert_eq!(row.pending_evidence_ids, vec!["EV-PH".to_string()], "待补面可追踪");
        assert_eq!(row.evidence_ids.len(), 0, "占位证据不计入有效证据列");
        // 非必选行：占位 → 待补但不阻断（业务允许继续写作）
        let row2 = s.matrix().iter().find(|r| r.requirement_id == "REQ-0005").unwrap();
        assert_eq!(row2.status, ResponseStatus::PendingEvidence);
        assert!(!row2.blocking, "非必选行占位 → 待补可见不阻断");
        // 导出门仍拒（占位绑定不清阻塞）
        assert!(s.check_export_ready().is_err(), "占位绑定 → 矩阵门拒绝导出");
    }

    /// 真实与占位并存：真实为准（RESPONDED/PARTIAL 照常派生），占位进
    /// 待补追踪面——既有正向流程（真实证据响应）零回归。
    #[test]
    fn real_evidence_dominates_placeholder_tracking() {
        let mut s = store_with_matrix_base(5);
        push_ev(&mut s, "EV-REAL", Some(crate::evidence::VerificationTier::ContentMatches));
        push_ev(&mut s, "EV-PH", Some(crate::evidence::VerificationTier::Placeholder));
        s.evidence_bindings.push(EvidenceBinding {
            evidence_id: "EV-REAL".into(), requirement_id: "REQ-0003".into(),
        });
        s.evidence_bindings.push(EvidenceBinding {
            evidence_id: "EV-PH".into(), requirement_id: "REQ-0003".into(),
        });
        // 章节在位（REQ-0003 → SEC）
        let mut node = sec("SEC-0001", 1, &["REQ-0003"]);
        node.order = 10;
        s.set_outline(vec![node]).unwrap();
        let _ = s.build_matrix().unwrap();
        let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0003").unwrap();
        assert_eq!(row.status, ResponseStatus::Responded, "真实证据+章节 → 已响应（占位不降级状态）");
        assert!(!row.blocking, "真实材料在位 → 放行");
        assert_eq!(row.evidence_ids, vec!["EV-REAL".to_string()], "有效证据列只含真实件");
        assert_eq!(row.pending_evidence_ids, vec!["EV-PH".to_string()], "占位件进待补追踪面");
    }

    /// 纯占位 + 章节在位：仍是待补（章节正文不能替代证据——与 #241
    /// 「正文已响应不算已响应」同源语义）。
    #[test]
    fn placeholder_with_section_still_pending() {
        let mut s = store_with_matrix_base(5);
        push_ev(&mut s, "EV-PH", Some(crate::evidence::VerificationTier::Placeholder));
        s.evidence_bindings.push(EvidenceBinding {
            evidence_id: "EV-PH".into(), requirement_id: "REQ-0003".into(),
        });
        let mut node = sec("SEC-0001", 1, &["REQ-0003"]);
        node.order = 10;
        s.set_outline(vec![node]).unwrap();
        let _ = s.build_matrix().unwrap();
        let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0003").unwrap();
        assert_eq!(row.status, ResponseStatus::PendingEvidence, "占位 + 章节在位 → 仍待补");
        assert!(row.blocking, "资格条件仍阻断");
    }

    /// 人工豁免通道不受占位影响：Human receipt 覆写 NOT_APPLICABLE（带理由）
    /// → 待补行翻不适用、阻断解除——占位不堵死人工裁决（#182 边界保留）。
    #[test]
    fn human_override_still_releases_pending_row() {
        let mut s = store_with_matrix_base(5);
        push_ev(&mut s, "EV-PH", Some(crate::evidence::VerificationTier::Placeholder));
        s.evidence_bindings.push(EvidenceBinding {
            evidence_id: "EV-PH".into(), requirement_id: "REQ-0003".into(),
        });
        let _ = s.build_matrix().unwrap();
        let hp = TrustedPrincipal::human("人工裁决");
        s.set_matrix_override("REQ-0003".into(), ResponseStatus::NotApplicable,
                              Some("集团共享资质豁免（人工 receipt）".into()), &hp).unwrap();
        let _ = s.rebuild_matrix_for_override().unwrap();
        let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0003").unwrap();
        assert_eq!(row.status, ResponseStatus::NotApplicable, "人工覆写优先于派生态");
        assert!(!row.blocking, "豁免行不阻断");
    }

    /// 旧快照兼容：#245 前的 MatrixRow JSON 无 pending_evidence_ids 字段
    /// → serde default 空 Vec 载入零位移（升级旧数据的兼容策略）。
    #[test]
    fn legacy_matrix_row_snapshot_loads_without_pending_field() {
        let legacy = serde_json::json!({
            "id": "CMP-0003", "requirement_id": "REQ-0003",
            "response_section": null, "evidence_ids": ["EV-0001"],
            "status": "PARTIAL", "deviation": null, "blocking": false
        });
        let row: MatrixRow = serde_json::from_value(legacy).unwrap();
        assert_eq!(row.pending_evidence_ids, Vec::<String>::new(), "旧快照待补面缺省为空");
        assert_eq!(row.status, ResponseStatus::Partial);
        // PENDING_EVIDENCE 序列化往返（新快照面）
        let j = serde_json::to_string(&ResponseStatus::PendingEvidence).unwrap();
        assert_eq!(j, "\"PENDING_EVIDENCE\"");
        let back: ResponseStatus = serde_json::from_str(&j).unwrap();
        assert_eq!(back, ResponseStatus::PendingEvidence);
    }

    // ============================ #241 裁定：正文响应与证据缺口两分账 ============================

    /// 带正文的章节构造（sec() 的 content 面——「写了就是写了」）。
    fn sec_with_content(id: &str, level: u8, reqs: &[&str], content: Option<&str>) -> OutlineNode {
        let mut n = sec(id, level, reqs);
        n.order = 10; // 文档序全局唯一
        n.content = content.map(|c| c.to_string());
        n
    }

    /// #241 专用最小 store（不预装轮转 REQ——set_requirements 是合并语义，
    /// 复用 store_with_matrix_base 会残留未映射条款污染导出门断言）。
    fn fresh_241_store(reqs: Vec<Requirement>) -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("#241 裁定", "C-241", "明标", "241-key").unwrap();
        s.upsert_file("A.docx", "招标文件", "sha-241", 5, 1, 50, 0, ParseStatus::Ok, "").unwrap();
        s.set_requirements(reqs).unwrap();
        s
    }

    fn worded_req(id: &str, category: &str, excerpt: &str) -> Requirement {
        let mut r = req(id, category);
        r.anchors[0].excerpt = excerpt.into();
        r
    }

    /// 验收 1（裁定操作口径）：正文有响应、无证据 → 矩阵 RESPONDED +
    /// evidence_missing=true（要求材料的必选条款）；必选行**不再 blocking**
    /// （证据缺口不阻断导出，由提交就绪面承载）；行为承诺型（无材料词）
    /// 正文响应 → RESPONDED 且无缺口标记。
    #[test]
    fn body_responded_without_evidence_is_responded_with_gap() {
        let mut s = fresh_241_store(vec![
            worded_req("REQ-0001", "否决项", "投标人须提供投标保证金缴纳凭证，否则否决"),
            worded_req("REQ-0002", "商务要求", "投标人须承诺质保期不少于五年"),
        ]);
        let node = sec_with_content("SEC-0001", 1, &["REQ-0001", "REQ-0002"],
            Some("我方对本章条款逐一作出完整回应：保证金凭证与质保承诺均已落实。"));
        s.set_outline(vec![node]).unwrap();
        let _ = s.build_matrix().unwrap();
        // 材料型否决项：正文响应 → RESPONDED（不再冤枉真响应）但缺口在
        let r1 = s.matrix().iter().find(|r| r.requirement_id == "REQ-0001").unwrap();
        assert_eq!(r1.status, ResponseStatus::Responded, "正文有实质响应 → RESPONDED（文本事实）");
        assert!(!r1.blocking, "正文响应的必选行不阻断导出（两分账主面）");
        assert!(r1.evidence_required, "词面命中材料要求词（提供/凭证）");
        assert!(r1.evidence_missing, "无有效证据在绑 → 缺口为真（合规事实）");
        // 承诺型商务条款：正文响应 → RESPONDED 且无缺口（承诺的载体就是正文）
        let r2 = s.matrix().iter().find(|r| r.requirement_id == "REQ-0002").unwrap();
        assert_eq!(r2.status, ResponseStatus::Responded);
        assert!(!r2.blocking);
        assert!(!r2.evidence_required, "无材料要求词 → 不要求证明材料");
        assert!(!r2.evidence_missing);
        // 导出门：两行皆非 blocking → 放行（草稿可导出——裁定门禁分工）
        assert!(s.check_export_ready().is_ok(), "证据缺口不拦导出");
    }

    /// 验收 4：正文无响应（章节在树但 content 空 / 无章节）→ 仍 NOT_RESPONDED
    /// （不变）——「写了就是写了」的另一面是「没写就是没写」；必选未响应
    /// 仍阻断导出（不变面）。
    #[test]
    fn body_not_written_stays_not_responded() {
        let mut s = fresh_241_store(vec![
            worded_req("REQ-0001", "否决项", "投标人须提供投标保证金缴纳凭证，否则否决"),
            worded_req("REQ-0002", "资格条件", "投标人须具备同类项目业绩"),
        ]);
        let nodes = vec![
            sec_with_content("SEC-0001", 1, &["REQ-0001"], None),
            sec_with_content("SEC-0002", 1, &["REQ-0002"], Some("   ")),
        ];
        let mut nodes = nodes;
        nodes[1].order = 11; // 文档序全局唯一
        s.set_outline(nodes).unwrap();
        let _ = s.build_matrix().unwrap();
        let r1 = s.matrix().iter().find(|r| r.requirement_id == "REQ-0001").unwrap();
        assert_eq!(r1.status, ResponseStatus::NotResponded, "章节在树但未写 → 未响应（不变）");
        assert!(r1.blocking, "否决项未响应 → 仍阻断（验收 4 不变面）");
        let r2 = s.matrix().iter().find(|r| r.requirement_id == "REQ-0002").unwrap();
        assert_eq!(r2.status, ResponseStatus::NotResponded, "空白正文不算实质响应");
        assert!(r2.blocking);
        assert!(r2.evidence_missing, "未响应行同样携带缺口标记（信息面）");
        assert!(s.check_export_ready().is_err(), "必选未响应仍拦导出（不变）");
    }

    /// 验收 3：补充证据后 evidence_missing 翻转为 false（缺口关闭通道）。
    /// 资格条件类别面恒要求证据（词面无关）。
    #[test]
    fn evidence_binding_flips_missing_false() {
        let mut s = fresh_241_store(vec![
            worded_req("REQ-0001", "资格条件", "投标人须具备独立法人资格"),
        ]);
        let node = sec_with_content("SEC-0001", 1, &["REQ-0001"],
            Some("资格响应：我方具备招标文件要求的全部资格条件。"));
        s.set_outline(vec![node]).unwrap();
        let _ = s.build_matrix().unwrap();
        let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0001").unwrap();
        assert_eq!(row.status, ResponseStatus::Responded);
        assert!(row.evidence_required, "资格条件类别面恒要求证明材料");
        assert!(row.evidence_missing, "无证据 → 缺口在");
        // 补绑真实证据（Active + 非占位）→ 翻转
        push_ev(&mut s, "EV-0001", Some(crate::evidence::VerificationTier::ContentMatches));
        s.evidence_bindings.push(EvidenceBinding {
            evidence_id: "EV-0001".into(), requirement_id: "REQ-0001".into(),
        });
        let _ = s.build_matrix().unwrap();
        let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0001").unwrap();
        assert!(!row.evidence_missing, "补绑有效证据 → 缺口翻 false（验收 3）");
        assert_eq!(row.status, ResponseStatus::Responded);
    }

    /// requires_evidence 两信号面：类别面（资格条件恒真）∪ 词面（任何类别
    /// 的材料要求词）；行为承诺型否决项不按类别一刀切。
    #[test]
    fn requires_evidence_category_and_word_faces() {
        // 类别面
        assert!(requires_evidence(&req("REQ-0001", "资格条件")));
        // 词面：任何类别命中材料词
        let mut worded = req("REQ-0002", "否决项");
        worded.anchors[0].excerpt = "投标人须提供投标保证金缴纳凭证，否则投标无效".into();
        assert!(requires_evidence(&worded));
        // Agent 提交面：params.statement 命中（merge.rs 双写面）
        let mut agent_stmt = req("REQ-0003", "商务要求");
        agent_stmt.params.insert("statement".into(),
            serde_json::json!("投标人须附上法定代表人授权委托书扫描件"));
        assert!(requires_evidence(&agent_stmt));
        // 负向：行为承诺型（无材料词）不要求证据——承诺的载体就是正文
        let mut promise = req("REQ-0004", "商务要求");
        promise.anchors[0].excerpt = "投标人须承诺质保期不少于五年".into();
        assert!(!requires_evidence(&promise));
        // 负向：否决项类别单独不触发（词面捕捉，不一刀切）
        let mut veto_promise = req("REQ-0005", "否决项");
        veto_promise.anchors[0].excerpt = "投标文件未按规定签字盖章的将被否决".into();
        assert!(!requires_evidence(&veto_promise));
    }

    /// #245 语义不回归：占位绑定 + 正文在位 → 仍 PENDING_EVIDENCE 且
    /// blocking（占位不清阻塞），evidence_missing 保持 true（占位不算
    /// 有效证据——不清缺口）。
    #[test]
    fn placeholder_keeps_pending_blocking_and_gap_with_body() {
        let mut s = fresh_241_store(vec![
            worded_req("REQ-0001", "资格条件", "投标人须提供同类项目业绩证明材料"),
        ]);
        push_ev(&mut s, "EV-PH", Some(crate::evidence::VerificationTier::Placeholder));
        s.evidence_bindings.push(EvidenceBinding {
            evidence_id: "EV-PH".into(), requirement_id: "REQ-0001".into(),
        });
        let node = sec_with_content("SEC-0001", 1, &["REQ-0001"],
            Some("正文响应已写（占位材料在绑——待补态不因正文翻已响应）"));
        s.set_outline(vec![node]).unwrap();
        let _ = s.build_matrix().unwrap();
        let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0001").unwrap();
        assert_eq!(row.status, ResponseStatus::PendingEvidence,
            "#245：占位仍待补（不因正文降级为 RESPONDED）");
        assert!(row.blocking, "资格条件 + 待补 → 仍阻断（#245 不回归）");
        assert_eq!(row.pending_evidence_ids, vec!["EV-PH".to_string()], "待补面可追踪");
        assert!(row.evidence_missing, "占位绑定不算有效证据——缺口仍在（#245 口径）");
        assert!(s.check_export_ready().is_err(), "占位绑定 → 导出门仍拒（#245 不回归）");
    }

    /// 旧快照兼容（#241）：无 evidence_required / evidence_missing 字段的
    /// MatrixRow JSON → serde default false 载入零位移（验收 5）。
    #[test]
    fn legacy_matrix_row_snapshot_loads_without_evidence_fields() {
        let legacy = serde_json::json!({
            "id": "CMP-0001", "requirement_id": "REQ-0001",
            "response_section": null, "evidence_ids": [],
            "pending_evidence_ids": [],
            "status": "NOT_RESPONDED", "deviation": null, "blocking": true
        });
        let row: MatrixRow = serde_json::from_value(legacy).unwrap();
        assert!(!row.evidence_required, "旧快照证据要求面缺省 false（验收 5）");
        assert!(!row.evidence_missing);
        assert_eq!(row.status, ResponseStatus::NotResponded);
        // 新快照往返：两标记序列化面在场
        let j = serde_json::to_string(&row).unwrap();
        assert!(j.contains("\"evidence_required\":false"), "序列化携带新字段：{j}");
        assert!(j.contains("\"evidence_missing\":false"));
    }
}
