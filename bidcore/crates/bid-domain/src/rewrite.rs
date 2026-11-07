//! rewrite.rs —— #319 补遗后增量重写（域层：变更账本 + 改写指令）。
//!
//! ## 定位（与既有面的分工）
//!
//! - **impact.rs**（V2 SEMUL）：影响面引擎的**契约语义面**（corpus-A /
//!   中传028 真实链的 contract 测试锚），自成一体不接生产——R-mod/R-del/
//!   R-add/R-date/R-conf 的语义真源；
//! - **本模块**：同一 R-mod/R-del/R-add 语义在生产 `DomainStore` 上的落点
//!   ——补遗变更登记（条款集/章节树/矩阵/绑定的**增量**维护 + DIRTY 打标）
//!   与改写指令账本（RD-xxxx）。R-date（评估基准日全量重检）与 R-conf
//!   （双版本冲突登记）不在本模块——前者属 #322 有效期面、后者属终裁面，
//!   本模块只管「补遗后怎么把已写的稿子改对」；
//! - **segments.rs**（#254）：段账本——指令的 `affected_segments` 指向
//!   SEG-xxxx，执行面（局部改写）复用 `rewrite_section_segment`，
//!   不另起第二套写入路径。
//!
//! ## 语义（与 impact.rs 对齐）
//!
//! - R-mod 数值变更：REQ 既有 `params.addendum_requirement` 记最新要求文本
//!   （锚点 excerpt 创建时固化不可篡改——#185 纪律，改要求不动锚）；
//!   关联叶章节打 DIRTY；
//! - R-add 新增要求：新 REQ 入条款集（id = 既有最大数字后缀 + 1，类别
//!   受检）；指定归属叶章节时入 `requirement_ids` 并沿祖先传播（不变量 9：
//!   父 = 子并集）；无归属 → 零章节影响（无指令，只进矩阵）；
//! - R-del 删除要求：REQ 出条款集 + 绑定解除 + 矩阵行随重算消失 + 覆写
//!   清理；从各叶 `requirement_ids` 移除（叶清空 → 拒——整章下线属结构
//!   重建，不隐式放行）；关联叶打 DIRTY。
//!
//! ## DIRTY 纪律
//!
//! 只打**叶**章节（正文只住叶——父节点无内容可脏）。指令全部处置
//! （Applied/Dismissed）后清标——「改写追上补遗」的可计算判据。
//!
//! ## 持久化（加性迁移）
//!
//! `StoreSnapshot.addendum_changes` / `rewrite_directives`（`#[serde(default)]`
//! ——旧快照空集合加载，行为与旧版一致；schema_version 不 bump，见
//! StoreSnapshot 头注释的加性演进纪律）。AC-/RD- 入 id 水位（防重启重签）。

use crate::{DomainStore, ToolError};
use serde::{Deserialize, Serialize};

/// 补遗变更类型（issue #319 ①：数值变更 / 新增要求 / 删除要求）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    /// 数值变更（R-mod）——既有条款的要求文本/参数更新
    ValueChanged,
    /// 新增要求（R-add）——补遗新提出的条款
    RequirementAdded,
    /// 删除要求（R-del）——补遗废除的条款
    RequirementRemoved,
}

impl ChangeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeType::ValueChanged => "value_changed",
            ChangeType::RequirementAdded => "requirement_added",
            ChangeType::RequirementRemoved => "requirement_removed",
        }
    }

    /// MCP 入参键（服务层校验用——非法键显式拒，不静默落默认）。
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "value_changed" => Some(ChangeType::ValueChanged),
            "requirement_added" => Some(ChangeType::RequirementAdded),
            "requirement_removed" => Some(ChangeType::RequirementRemoved),
            _ => None,
        }
    }
}

/// 指令处置状态（Pending → Applied；Dismissed = 人工裁定无需改写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectiveStatus {
    Pending,
    Applied,
    Dismissed,
}

impl DirectiveStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DirectiveStatus::Pending => "pending",
            DirectiveStatus::Applied => "applied",
            DirectiveStatus::Dismissed => "dismissed",
        }
    }
}

/// 补遗变更登记（AC-xxxx——append-only 审计账）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddendumChangeRecord {
    pub id: String,
    /// 目标条款（add：由域层签发的新 REQ id——回显给调用方）
    pub req_id: String,
    pub change_type: ChangeType,
    /// 旧要求片段（受影响原文的定位线索——可空；mod/del 建议提供）
    pub old_text: String,
    /// 新要求文本（add/mod 必填；del 为空）
    pub new_requirement: String,
    /// 关联补遗件（TF-xxxx——可溯源；缺省 = 人工转述）
    pub source_file_id: Option<String>,
    pub registered_at: u64,
    /// 登记时的 content_version（审计面——变更序与版本序对账）
    pub content_version_at: u64,
}

/// 指令影响面里的段定位（服务层匹配、域层落账）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffectedSegment {
    pub segment_id: String,
    pub seq: u32,
    /// 命中腿：`citation`（段引用了该 REQ）| `old_text`（含旧要求片段）|
    /// `citation+old_text`（双腿）
    pub matched_by: String,
}

/// 改写指令（RD-xxxx——issue #319 ①：影响面 → 改写指令）。
///
/// 一条变更 × 一个受影响章节 = 一条指令。`affected_segments` 由服务层
/// 按「段引用该 REQ ∪ 段含 old_text」匹配段账本得出（#254 段是真增量
/// 单元——指令落在段上，整章重写不再是唯一出口）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RewriteDirective {
    pub id: String,
    pub section_id: String,
    /// 来源变更（AC-xxxx）
    pub change_id: String,
    pub req_id: String,
    pub change_type: ChangeType,
    pub old_text: String,
    pub new_requirement: String,
    /// 受影响段（SEG-xxxx 清单——执行面只许改这些段）；新增要求恒空
    /// （无既有段可改——响应走 append_section_segment 新增段）
    pub affected_segments: Vec<AffectedSegment>,
    pub status: DirectiveStatus,
    pub created_at: u64,
    pub applied_at: Option<u64>,
}

/// 变更登记入参（服务层组装——MCP 形态到域形态的翻译半边）。
#[derive(Debug, Clone)]
pub struct ChangeSpec {
    /// mod/del：目标 REQ（必须已存在）；add：缺省（域层签发新 id）
    pub req_id: Option<String>,
    pub change_type: ChangeType,
    pub old_text: String,
    pub new_requirement: String,
    /// add 必填：条款类别（资格条件|否决项|评分项|技术参数|商务要求|格式要求|其他）
    pub category: Option<String>,
    /// add 可选：条款号
    pub clause_no: Option<String>,
    /// add 可选：归属叶章节（缺省 = 零章节影响——只进条款集与矩阵）
    pub section_id: Option<String>,
    /// 关联补遗件（TF-xxxx，须已导入）
    pub source_file_id: Option<String>,
}

/// 单条变更的登记结果（受影响叶章节清单 = 指令生成面）。
#[derive(Debug, Clone)]
pub struct RegisteredChange {
    pub change: AddendumChangeRecord,
    /// 受影响叶章节（SEC-xxxx——DIRTY 已打标；空 = 无章节影响面）
    pub affected_sections: Vec<String>,
    pub warnings: Vec<String>,
}

/// 合法条款类别（derive_matrix_rows 的 is_mandatory 消费子集——add 面
/// 受检，防「自报类别绕过 blocking 派生」）。
pub const REQ_CATEGORIES: &[&str] = &[
    "资格条件",
    "否决项",
    "评分项",
    "技术参数",
    "商务要求",
    "格式要求",
    "其他",
];

/// 补遗要求文本入 `params` 的键（mod/add 共用——矩阵/溯源面的最新要求可见）。
pub const PARAM_KEY: &str = "addendum_requirement";

impl DomainStore {
    // ------------------------------------------------------------ 读面

    pub fn addendum_changes(&self) -> &[AddendumChangeRecord] {
        &self.addendum_changes
    }

    pub fn rewrite_directives(&self) -> &[RewriteDirective] {
        &self.rewrite_directives
    }

    pub fn rewrite_directive(&self, id: &str) -> Result<&RewriteDirective, ToolError> {
        self.rewrite_directives
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| ToolError::ENotFound {
                what: format!("rewrite directive {id} 不存在"),
            })
    }

    /// 全部待处置指令（get_bid_project_state 的发现面）。
    pub fn pending_rewrite_directives(&self) -> Vec<&RewriteDirective> {
        self.rewrite_directives
            .iter()
            .filter(|d| d.status == DirectiveStatus::Pending)
            .collect()
    }

    pub fn pending_rewrite_directives_of(&self, section_id: &str) -> Vec<&RewriteDirective> {
        self.rewrite_directives
            .iter()
            .filter(|d| d.status == DirectiveStatus::Pending && d.section_id == section_id)
            .collect()
    }

    // ------------------------------------------------------------ 登记面（批量原子）

    /// 批量登记补遗变更（**原子**：全部校验通过才落账——任一失败零副作用）。
    ///
    /// 落账动作（逐变更）：
    /// 1. 幂等预检：同 req + 同类型 + 同文本的**未处置**变更已存在 → 跳过
    ///    （warning 回显，不重复打标/不重复产指令）；
    /// 2. 条款集维护（mod：params 记最新要求；add：新 REQ；del：移除 +
    ///    绑定解除 + 覆写清理）；
    /// 3. 章节树维护（add：入叶 + 祖先传播；del：出叶 + 祖先重算）；
    /// 4. 关联叶打 DIRTY + content_version +1。
    ///
    /// 矩阵重算**不在本方法**（批量落账后由调用方统一 `build_matrix`
    /// ——一次重算，避免逐变更重算的版本抖动）。
    pub fn register_addendum_changes(
        &mut self,
        specs: &[ChangeSpec],
    ) -> Result<Vec<RegisteredChange>, ToolError> {
        if specs.is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "changes 不得为空（空登记无意义——补遗变更至少一条）".into(),
            });
        }
        if self.project.is_none() {
            return Err(ToolError::ENotFound {
                what: "project 未创建".into(),
            });
        }
        // ---- 校验面（纯读——全部通过才动第一笔账；错误码保真，
        //      消息加 changes[i] 定位前缀——调用方按 code 分支不受影响）
        for (i, spec) in specs.iter().enumerate() {
            if let Err(e) = self.validate_change_spec(spec) {
                return Err(match e {
                    ToolError::EUnprocessable { reason } => ToolError::EUnprocessable {
                        reason: format!("changes[{i}] {reason}"),
                    },
                    ToolError::ENotFound { what } => ToolError::ENotFound {
                        what: format!("changes[{i}] {what}"),
                    },
                    other => other,
                });
            }
        }
        // ---- 落账面（逐条——校验已全过，此处不再失败）
        let mut out = Vec::with_capacity(specs.len());
        for spec in specs {
            out.push(self.apply_change_spec(spec));
        }
        Ok(out)
    }

    /// 单条入参校验（纯读）。返回 Err = 拒因（E_UNPROCESSABLE 语义）。
    fn validate_change_spec(&self, spec: &ChangeSpec) -> Result<(), ToolError> {
        let req_exists = |id: &str| self.requirements.iter().any(|r| r.id == id);
        match spec.change_type {
            ChangeType::ValueChanged => {
                let Some(req_id) = &spec.req_id else {
                    return Err(ToolError::EUnprocessable {
                        reason: "value_changed 必须指定 req_id（改的是既有条款）".into(),
                    });
                };
                if !req_exists(req_id) {
                    return Err(ToolError::ENotFound {
                        what: format!(
                            "requirement {req_id} 不存在（value_changed 的对象必须是已抽取条款）"
                        ),
                    });
                }
                if spec.new_requirement.trim().is_empty() {
                    return Err(ToolError::EUnprocessable {
                        reason: "value_changed 的 new_requirement 不得为空白（变更后的要求文本）"
                            .into(),
                    });
                }
            }
            ChangeType::RequirementAdded => {
                if let Some(id) = &spec.req_id {
                    if req_exists(id) {
                        return Err(ToolError::EUnprocessable {
                            reason: format!("requirement_added 要求新条款，但 {id} 已存在（改既有条款用 value_changed）"),
                        });
                    }
                }
                let Some(cat) = &spec.category else {
                    return Err(ToolError::EUnprocessable {
                        reason: "requirement_added 必须指定 category（类别驱动矩阵 blocking 派生——不得缺省）".into(),
                    });
                };
                if !REQ_CATEGORIES.contains(&cat.as_str()) {
                    return Err(ToolError::EUnprocessable {
                        reason: format!(
                            "category 非法：{cat}（合法值：{}）",
                            REQ_CATEGORIES.join("|")
                        ),
                    });
                }
                if spec.new_requirement.trim().is_empty() {
                    return Err(ToolError::EUnprocessable {
                        reason: "requirement_added 的 new_requirement 不得为空白".into(),
                    });
                }
                if let Some(sec) = &spec.section_id {
                    self.guard_leaf_section(sec)?;
                }
            }
            ChangeType::RequirementRemoved => {
                let Some(req_id) = &spec.req_id else {
                    return Err(ToolError::EUnprocessable {
                        reason: "requirement_removed 必须指定 req_id（删的是既有条款）".into(),
                    });
                };
                if !req_exists(req_id) {
                    return Err(ToolError::ENotFound {
                        what: format!("requirement {req_id} 不存在（requirement_removed 的对象必须是已抽取条款）"),
                    });
                }
                // #224 引用完整性（与 set_requirements/replace_requirements 同款）：
                // 评分映射引用的评分项不得蒸发。
                if let Some(m) = self
                    .scoring_mappings
                    .iter()
                    .find(|m| m.scoring_req_id == *req_id)
                {
                    return Err(ToolError::EUnprocessable {
                        reason: format!(
                            "评分映射 {} 引用的评分项 {req_id} 将随删除消失（#224 引用完整性——先 remove_scoring_mapping({req_id}, {}) 撤销该映射再删除）",
                            m.id, m.section_id
                        ),
                    });
                }
                // 叶清空守卫：某叶仅剩本条款 → 移除后违反「每节点 ≥1 REQ」
                // （不变量 2）——整章下线属结构重建，显式拒。
                for n in &self.sections {
                    let has_child = self
                        .sections
                        .iter()
                        .any(|c| c.parent_id.as_deref() == Some(&n.id[..]));
                    if !has_child
                        && n.requirement_ids.iter().any(|r| r == req_id)
                        && n.requirement_ids.len() == 1
                    {
                        return Err(ToolError::EUnprocessable {
                            reason: format!(
                                "叶章节 {} 仅绑定 {req_id} 一条条款——删除将清空该叶（违反每节点 ≥1 REQ 不变量）。该条款下线属章节结构重建：先 build_outline 重组或人工调整章节树，再登记删除",
                                n.id
                            ),
                        });
                    }
                }
            }
        }
        if let Some(fid) = &spec.source_file_id {
            if !self.files.iter().any(|f| &f.id == fid) {
                return Err(ToolError::ENotFound {
                    what: format!("source_file_id {fid} 不存在（须为已导入的补遗件 TF-xxxx）"),
                });
            }
        }
        Ok(())
    }

    /// 叶章节守卫（add 的归属目标）：存在 + 叶（正文与分段只住叶）。
    fn guard_leaf_section(&self, section_id: &str) -> Result<(), ToolError> {
        self.sections
            .iter()
            .find(|n| n.id == section_id)
            .ok_or_else(|| ToolError::ENotFound {
                what: format!("section {section_id} 不存在"),
            })?;
        let has_child = self
            .sections
            .iter()
            .any(|c| c.parent_id.as_deref() == Some(section_id));
        if has_child {
            return Err(ToolError::EUnprocessable {
                reason: format!("{section_id} 是结构节点（有子节点）——新条款只可挂叶章节"),
            });
        }
        Ok(())
    }

    /// 落账单条（校验已过——本方法不再返回 Err，告警走 warnings）。
    fn apply_change_spec(&mut self, spec: &ChangeSpec) -> RegisteredChange {
        let mut warnings = Vec::new();
        // 疑似重复登记探测（告警面，不拒）：同 req + 同类型 + 同文本的变更
        // 已在账——重复登记照常落账（补遗可能确实重发了同一条），但告警
        // 提示调用方核对。add 面 req_id 由域层签发（spec 无 id），以
        // 「同类型 + 同文本」近似匹配。
        let known_req_id = spec.req_id.clone().unwrap_or_default();
        let dup = self.addendum_changes.iter().any(|c| {
            let same_req = if spec.req_id.is_some() {
                c.req_id == known_req_id
            } else {
                true
            };
            same_req
                && c.change_type == spec.change_type
                && c.old_text == spec.old_text
                && c.new_requirement == spec.new_requirement
        });

        let at = now_epoch();
        let cv = self
            .project
            .as_ref()
            .map(|p| p.content_version)
            .unwrap_or(0);

        let req_id = match spec.change_type {
            ChangeType::ValueChanged | ChangeType::RequirementRemoved => {
                spec.req_id.clone().expect("校验面已保证")
            }
            ChangeType::RequirementAdded => self.next_req_id(),
        };

        // ---- 条款集/章节树维护（变更真身）
        let affected: Vec<String> = match spec.change_type {
            ChangeType::ValueChanged => {
                if let Some(r) = self.requirements.iter_mut().find(|r| r.id == req_id) {
                    r.params.insert(
                        PARAM_KEY.to_string(),
                        serde_json::Value::String(spec.new_requirement.clone()),
                    );
                }
                self.dirty_leaves_of_req(&req_id)
            }
            ChangeType::RequirementAdded => {
                let mut params = std::collections::HashMap::new();
                params.insert(
                    PARAM_KEY.to_string(),
                    serde_json::Value::String(spec.new_requirement.clone()),
                );
                self.requirements.push(crate::Requirement {
                    id: req_id.clone(),
                    category: spec.category.clone().expect("校验面已保证"),
                    clause_no: spec.clause_no.clone(),
                    anchors: vec![],
                    params,
                    source_file: spec
                        .source_file_id
                        .clone()
                        .unwrap_or_else(|| "addendum:manual".to_string()),
                });
                match &spec.section_id {
                    Some(sec) => {
                        self.link_requirement_to_section(sec, &req_id);
                        vec![sec.clone()]
                    }
                    None => {
                        warnings.push(format!(
                            "新增要求 {req_id} 未指定归属章节——已入条款集与矩阵（响应面待建纲/人工归章），零章节影响（R-add 无归属语义）"
                        ));
                        vec![]
                    }
                }
            }
            ChangeType::RequirementRemoved => {
                self.requirements.retain(|r| r.id != req_id);
                // R-del：绑定解除 + 覆写清理（矩阵行随批量重算消失）
                self.evidence_bindings
                    .retain(|b| b.requirement_id != req_id);
                self.matrix_overrides.remove(&req_id);
                self.unlink_requirement_from_sections(&req_id)
            }
        };

        let change = AddendumChangeRecord {
            id: self.next_id("AC"),
            req_id: req_id.clone(),
            change_type: spec.change_type,
            old_text: spec.old_text.clone(),
            new_requirement: spec.new_requirement.clone(),
            source_file_id: spec.source_file_id.clone(),
            registered_at: at,
            content_version_at: cv,
        };
        self.addendum_changes.push(change.clone());
        // DIRTY 打标（叶）+ content_version +1（上游变化即版本变化）
        for sec in &affected {
            if let Some(n) = self.sections.iter_mut().find(|n| &n.id == sec) {
                n.dirty = true;
            }
        }
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        if dup {
            warnings.push(format!(
                "疑似重复登记：{req_id} 的 {:?} 变更与在账未处置变更加本一致——已照常落账，请核对补遗原文",
                spec.change_type
            ));
        }
        RegisteredChange {
            change,
            affected_sections: affected,
            warnings,
        }
    }

    /// REQ 关联叶章节（打标面——只取叶：正文只住叶，父节点无内容可脏）。
    fn dirty_leaves_of_req(&self, req_id: &str) -> Vec<String> {
        self.sections
            .iter()
            .filter(|n| {
                n.requirement_ids.iter().any(|r| r == req_id)
                    && !self
                        .sections
                        .iter()
                        .any(|c| c.parent_id.as_deref() == Some(&n.id[..]))
            })
            .map(|n| n.id.clone())
            .collect()
    }

    /// #456（A-09 残留面）：上游变更统一打脏入口——REQ 集变化 / 证据换版 /
    /// 新绑定后，按 REQ→叶章节关联（[`Self::dirty_leaves_of_req`] 同一语义）
    /// 打 DIRTY（失效信号：`get_bid_project_state` 的 `dirty_sections` 自动可见）。
    ///
    /// 幂等安全：已脏章节重复打标无副作用；返回**本次实际翻转**
    /// （false→true）的章节 id（确定序——BTreeSet 去重排序，测试断言面）；
    /// 空输入 / REQ 无归属章节 → 空 vec（与 #319 R-add 无归属语义对齐，不 panic）。
    ///
    /// 只打脏不清脏——清除仍归补遗指令处置面
    /// （apply/dismiss 后 `pending_rewrite_directives_of` 为空才清，见上方两方法）
    /// 与章节重开重走流程，本方法不发明第二条清标路径。
    pub(crate) fn mark_dirty_by_requirements(&mut self, req_ids: &[String]) -> Vec<String> {
        let targets: std::collections::BTreeSet<String> = req_ids
            .iter()
            .flat_map(|id| self.dirty_leaves_of_req(id))
            .collect();
        let mut marked = Vec::with_capacity(targets.len());
        for sec in targets {
            if let Some(n) = self.sections.iter_mut().find(|n| n.id == sec) {
                if !n.dirty {
                    n.dirty = true;
                    marked.push(sec);
                }
            }
        }
        marked
    }

    /// 新条款 id：既有最大数字后缀 + 1（与抽取层 REQ-NNNN 同形态；
    /// 建纲后抽取面 E_STATE 关闭，无重签碰撞面）。
    fn next_req_id(&mut self) -> String {
        let max = self
            .requirements
            .iter()
            .filter_map(|r| r.id.strip_prefix("REQ-"))
            .filter_map(|s| s.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        format!("REQ-{:04}", max + 1)
    }

    /// 条款挂叶 + 祖先传播（不变量 9：父 requirement_ids = 子并集）。
    fn link_requirement_to_section(&mut self, section_id: &str, req_id: &str) {
        let mut cursor = Some(section_id.to_string());
        while let Some(id) = cursor {
            if let Some(n) = self.sections.iter_mut().find(|n| n.id == id) {
                if !n.requirement_ids.iter().any(|r| r == req_id) {
                    n.requirement_ids.push(req_id.to_string());
                }
                cursor = n.parent_id.clone();
            } else {
                break;
            }
        }
    }

    /// 条款出各叶 + 祖先重算（从叶并集重推——移除后父不再含该条款时同步下架）。
    fn unlink_requirement_from_sections(&mut self, req_id: &str) -> Vec<String> {
        let affected: Vec<String> = self
            .sections
            .iter()
            .filter(|n| n.requirement_ids.iter().any(|r| r == req_id))
            .map(|n| n.id.clone())
            .collect();
        for n in self.sections.iter_mut() {
            n.requirement_ids.retain(|r| r != req_id);
        }
        // 祖先重算（防孤儿父残留——父 = 子并集，非并集成员不得留）。
        // 注：单节点树（无父子关系）不受影响。
        //
        // 借用纪律：先算各父节点的并集（纯读），再逐节点写回——避免
        // `iter_mut` 与并集计算的同帧交叉借用。
        let unions: Vec<(String, Vec<String>)> = self
            .sections
            .iter()
            .filter_map(|n| {
                let children: Vec<&crate::outline::OutlineNode> = self
                    .sections
                    .iter()
                    .filter(|c| c.parent_id.as_deref() == Some(&n.id[..]))
                    .collect();
                if children.is_empty() {
                    return None;
                }
                let mut union: Vec<String> = Vec::new();
                for c in &children {
                    for rid in &c.requirement_ids {
                        if !union.contains(rid) {
                            union.push(rid.clone());
                        }
                    }
                }
                Some((n.id.clone(), union))
            })
            .collect();
        for (id, union) in unions {
            if let Some(n) = self.sections.iter_mut().find(|n| n.id == id) {
                if n.requirement_ids != union {
                    n.requirement_ids = union;
                }
            }
        }
        // 打标面 = 受影响的**叶**（affected 里可能含父——过滤）
        affected
            .into_iter()
            .filter(|id| {
                !self
                    .sections
                    .iter()
                    .any(|c| c.parent_id.as_deref() == Some(&id[..]))
            })
            .collect()
    }

    // ------------------------------------------------------------ 指令面

    /// 落一条改写指令（服务层匹配好 affected_segments 后入账——Pending）。
    pub fn store_rewrite_directive(
        &mut self,
        change: &AddendumChangeRecord,
        section_id: &str,
        affected_segments: Vec<AffectedSegment>,
    ) -> RewriteDirective {
        let d = RewriteDirective {
            id: self.next_id("RD"),
            section_id: section_id.to_string(),
            change_id: change.id.clone(),
            req_id: change.req_id.clone(),
            change_type: change.change_type,
            old_text: change.old_text.clone(),
            new_requirement: change.new_requirement.clone(),
            affected_segments,
            status: DirectiveStatus::Pending,
            created_at: now_epoch(),
            applied_at: None,
        };
        self.rewrite_directives.push(d.clone());
        d
    }

    /// 处置指令（Applied——applied_segment_ids 为本次实际改写的段；
    /// Dismissed——人工裁定无需改写）。本模块只翻状态，段写入由
    /// segments.rs 承担（服务层编排：先改段、后处置——处置失败不回滚
    /// 段内容：段已改是事实，指令可重处置）。
    ///
    /// 处置后若该章节再无 Pending 指令 → 清 DIRTY（「改写追上补遗」）。
    pub fn apply_rewrite_directive(
        &mut self,
        directive_id: &str,
        applied_segment_ids: &[String],
    ) -> Result<RewriteDirective, ToolError> {
        let d = self.rewrite_directive(directive_id)?;
        if d.status != DirectiveStatus::Pending {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "指令 {directive_id} 状态为 {}（仅 pending 可处置——重放不是错误但不再改账）",
                    d.status.as_str()
                ),
            });
        }
        let idx = self
            .rewrite_directives
            .iter()
            .position(|d| d.id == directive_id)
            .expect("rewrite_directive 已定位");
        self.rewrite_directives[idx].status = DirectiveStatus::Applied;
        self.rewrite_directives[idx].applied_at = Some(now_epoch());
        let out = self.rewrite_directives[idx].clone();
        let section_id = out.section_id.clone();
        let _ = applied_segment_ids; // 审计面：段级明细由服务层响应回显（域层只记状态翻转）
        if self.pending_rewrite_directives_of(&section_id).is_empty() {
            if let Some(n) = self.sections.iter_mut().find(|n| n.id == section_id) {
                n.dirty = false;
            }
        }
        Ok(out)
    }

    /// 人工裁定无需改写（Dismissed——同样参与「追上补遗」的清标判定）。
    pub fn dismiss_rewrite_directive(
        &mut self,
        directive_id: &str,
    ) -> Result<RewriteDirective, ToolError> {
        let d = self.rewrite_directive(directive_id)?;
        if d.status != DirectiveStatus::Pending {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "指令 {directive_id} 状态为 {}（仅 pending 可处置）",
                    d.status.as_str()
                ),
            });
        }
        let idx = self
            .rewrite_directives
            .iter()
            .position(|d| d.id == directive_id)
            .expect("rewrite_directive 已定位");
        self.rewrite_directives[idx].status = DirectiveStatus::Dismissed;
        self.rewrite_directives[idx].applied_at = Some(now_epoch());
        let out = self.rewrite_directives[idx].clone();
        let section_id = out.section_id.clone();
        if self.pending_rewrite_directives_of(&section_id).is_empty() {
            if let Some(n) = self.sections.iter_mut().find(|n| n.id == section_id) {
                n.dirty = false;
            }
        }
        Ok(out)
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- 单元测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::{OutlineNode, SectionStatus};
    use crate::{ParseStatus, Requirement};

    /// 基座：项目 + 文件 + 3 条 REQ + 两叶一父树（父聚合并集）。
    fn store_with_tree() -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("测试项目", "ZFCG-2026-GK-0311", "明标", "ext-key-319")
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
        // 锚点互异（requirement_hash 只认锚——空锚件会被 dedupe 合并成一条）
        let reqs = ["REQ-0001", "REQ-0002", "REQ-0003"]
            .iter()
            .enumerate()
            .map(|(i, id)| Requirement {
                id: id.to_string(),
                category: "技术参数".into(),
                clause_no: Some("3.1".into()),
                anchors: vec![crate::Anchor {
                    file: "TF-0001".into(),
                    kind: "para".into(),
                    locator: format!("第{}章", i + 1),
                    excerpt: format!("锚点摘录 {i}"),
                    block_idx: None,
                    page: None,
                    line: None,
                }],
                params: std::collections::HashMap::new(),
                source_file: "A".into(),
            })
            .collect::<Vec<_>>();
        s.set_requirements(reqs).unwrap();
        let parent = OutlineNode {
            id: "SEC-P".into(),
            parent_id: None,
            title: "父".into(),
            level: 1,
            order: 1,
            requirement_ids: vec!["REQ-0001".into(), "REQ-0002".into(), "REQ-0003".into()],
            target_chars: 200,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        };
        let leaf_a = OutlineNode {
            id: "SEC-A".into(),
            parent_id: Some("SEC-P".into()),
            title: "叶A".into(),
            level: 2,
            order: 2,
            requirement_ids: vec!["REQ-0001".into(), "REQ-0002".into()],
            target_chars: 100,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        };
        let leaf_b = OutlineNode {
            id: "SEC-B".into(),
            parent_id: Some("SEC-P".into()),
            title: "叶B".into(),
            level: 2,
            order: 3,
            requirement_ids: vec!["REQ-0003".into()],
            target_chars: 100,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        };
        s.set_outline(vec![parent, leaf_a, leaf_b]).unwrap();
        s.build_matrix().unwrap();
        s
    }

    fn spec_value_changed(req: &str, old: &str, new: &str) -> ChangeSpec {
        ChangeSpec {
            req_id: Some(req.into()),
            change_type: ChangeType::ValueChanged,
            old_text: old.into(),
            new_requirement: new.into(),
            category: None,
            clause_no: None,
            section_id: None,
            source_file_id: None,
        }
    }

    /// R-mod：params 记最新要求 + 关联叶 DIRTY + 版本推进。
    #[test]
    fn value_changed_marks_dirty_and_records_param() {
        let mut s = store_with_tree();
        let cv0 = s.project().unwrap().content_version;
        let out = s
            .register_addendum_changes(&[spec_value_changed("REQ-0001", "旧值 100", "新值 200")])
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].change.req_id, "REQ-0001");
        assert_eq!(
            out[0].affected_sections,
            vec!["SEC-A".to_string()],
            "REQ-0001 只住叶A——打标面恰为该叶"
        );
        let node = s.sections().iter().find(|n| n.id == "SEC-A").unwrap();
        assert!(node.dirty, "关联叶必须打脏");
        let parent = s.sections().iter().find(|n| n.id == "SEC-P").unwrap();
        assert!(!parent.dirty, "父节点无内容不打脏");
        let other = s.sections().iter().find(|n| n.id == "SEC-B").unwrap();
        assert!(!other.dirty, "无关叶零误伤（R3 语义）");
        let req = s
            .requirements()
            .iter()
            .find(|r| r.id == "REQ-0001")
            .unwrap();
        assert_eq!(
            req.params.get(PARAM_KEY).and_then(|v| v.as_str()),
            Some("新值 200"),
            "最新要求文本入 params"
        );
        assert!(
            s.project().unwrap().content_version > cv0,
            "上游变化推进版本"
        );
    }

    /// R-add：新 REQ 入集 + 挂叶沿祖先传播（不变量 9 保持）。
    #[test]
    fn requirement_added_links_and_propagates() {
        let mut s = store_with_tree();
        let out = s
            .register_addendum_changes(&[ChangeSpec {
                req_id: None,
                change_type: ChangeType::RequirementAdded,
                old_text: String::new(),
                new_requirement: "新增：投标保证金 10 万元".into(),
                category: Some("商务要求".into()),
                clause_no: Some("6.2".into()),
                section_id: Some("SEC-A".into()),
                source_file_id: None,
            }])
            .unwrap();
        let new_id = out[0].change.req_id.clone();
        assert_eq!(new_id, "REQ-0004", "新 id = 最大数字后缀 + 1");
        assert_eq!(out[0].affected_sections, vec!["SEC-A".to_string()]);
        let leaf = s.sections().iter().find(|n| n.id == "SEC-A").unwrap();
        assert!(leaf.requirement_ids.contains(&new_id), "叶挂新条款");
        let parent = s.sections().iter().find(|n| n.id == "SEC-P").unwrap();
        assert!(parent.requirement_ids.contains(&new_id), "父沿并集传播");
        assert!(leaf.dirty, "归属叶打脏");
        // 矩阵重算后新行在场（blocking 由类别派生——商务要求非必选）
        s.build_matrix().unwrap();
        assert!(s.matrix().iter().any(|r| r.requirement_id == new_id));
    }

    /// R-add 无归属：零章节影响（不产指令面）。
    #[test]
    fn requirement_added_without_section_is_zero_impact() {
        let mut s = store_with_tree();
        let out = s
            .register_addendum_changes(&[ChangeSpec {
                req_id: None,
                change_type: ChangeType::RequirementAdded,
                old_text: String::new(),
                new_requirement: "新增：无归属条款".into(),
                category: Some("其他".into()),
                clause_no: None,
                section_id: None,
                source_file_id: None,
            }])
            .unwrap();
        assert!(out[0].affected_sections.is_empty());
        assert!(!out[0].warnings.is_empty(), "零章节影响要可见（告警面）");
    }

    /// R-del：条款出集 + 绑定解除 + 叶/父 requirement_ids 同步下架。
    #[test]
    fn requirement_removed_unlinks_everywhere() {
        let mut s = store_with_tree();
        // 先绑一条证据到 REQ-0001（R-del 绑定解除的验收面）
        s.evidence_bindings.push(crate::evidence::EvidenceBinding {
            evidence_id: "EV-0001".into(),
            requirement_id: "REQ-0001".into(),
        });
        let out = s
            .register_addendum_changes(&[ChangeSpec {
                req_id: Some("REQ-0001".into()),
                change_type: ChangeType::RequirementRemoved,
                old_text: "旧值 100".into(),
                new_requirement: String::new(),
                category: None,
                clause_no: None,
                section_id: None,
                source_file_id: None,
            }])
            .unwrap();
        assert_eq!(out[0].affected_sections, vec!["SEC-A".to_string()]);
        assert!(
            !s.requirements().iter().any(|r| r.id == "REQ-0001"),
            "条款出集"
        );
        assert!(
            !s.evidence_bindings
                .iter()
                .any(|b| b.requirement_id == "REQ-0001"),
            "R-del 绑定解除"
        );
        let leaf = s.sections().iter().find(|n| n.id == "SEC-A").unwrap();
        assert!(!leaf.requirement_ids.contains(&"REQ-0001".to_string()));
        assert!(leaf.dirty);
        let parent = s.sections().iter().find(|n| n.id == "SEC-P").unwrap();
        assert!(
            !parent.requirement_ids.contains(&"REQ-0001".to_string()),
            "父并集同步下架（不变量 9）"
        );
        // 叶A 仍有 REQ-0002——不变量 2（每节点 ≥1）保持
        assert!(!leaf.requirement_ids.is_empty());
    }

    /// R-del 叶清空守卫：仅剩一条的叶 → 显式拒（结构重建面）。
    #[test]
    fn requirement_removed_rejects_emptying_leaf() {
        let mut s = store_with_tree();
        let err = s
            .register_addendum_changes(&[ChangeSpec {
                req_id: Some("REQ-0003".into()),
                change_type: ChangeType::RequirementRemoved,
                old_text: String::new(),
                new_requirement: String::new(),
                category: None,
                clause_no: None,
                section_id: None,
                source_file_id: None,
            }])
            .unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE");
        assert!(format!("{err:?}").contains("SEC-B"), "拒因指名叶");
    }

    /// 批量原子：任一失败 → 零副作用。
    #[test]
    fn batch_is_atomic() {
        let mut s = store_with_tree();
        let cv0 = s.project().unwrap().content_version;
        let dirty0: Vec<bool> = s.sections().iter().map(|n| n.dirty).collect();
        let err = s
            .register_addendum_changes(&[
                spec_value_changed("REQ-0001", "旧", "新"),
                spec_value_changed("REQ-9999", "旧", "新"), // 不存在 → 整批拒
            ])
            .unwrap_err();
        assert_eq!(err.code(), "E_NOTFOUND");
        assert_eq!(s.project().unwrap().content_version, cv0, "版本零推进");
        assert_eq!(
            s.sections().iter().map(|n| n.dirty).collect::<Vec<_>>(),
            dirty0,
            "DIRTY 零打标"
        );
        assert!(s.addendum_changes().is_empty(), "账本零落账");
    }

    /// 指令处置：Applied 后无剩余 Pending → 清 DIRTY；有剩余 → 保持脏。
    #[test]
    fn directive_apply_clears_dirty_when_caught_up() {
        let mut s = store_with_tree();
        // 两条变更都打在叶A → 两条 Pending 指令
        let reg = s
            .register_addendum_changes(&[
                spec_value_changed("REQ-0001", "旧1", "新1"),
                spec_value_changed("REQ-0002", "旧2", "新2"),
            ])
            .unwrap();
        let d1 = s.store_rewrite_directive(&reg[0].change, "SEC-A", vec![]);
        let d2 = s.store_rewrite_directive(&reg[1].change, "SEC-A", vec![]);
        assert!(s.sections().iter().find(|n| n.id == "SEC-A").unwrap().dirty);
        // 处置第一条：仍有 Pending → 保持脏
        s.apply_rewrite_directive(&d1.id, &[]).unwrap();
        assert!(s.sections().iter().find(|n| n.id == "SEC-A").unwrap().dirty);
        // 处置第二条：追上补遗 → 清脏
        let done = s.apply_rewrite_directive(&d2.id, &[]).unwrap();
        assert_eq!(done.status, DirectiveStatus::Applied);
        assert!(
            !s.sections().iter().find(|n| n.id == "SEC-A").unwrap().dirty,
            "全部指令处置完 = 改写追上补遗"
        );
        // 已处置指令重放 → 显式拒（零副作用）
        let err = s.apply_rewrite_directive(&d2.id, &[]).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE");
    }

    /// 快照往返：变更账 + 指令账全保真（加性迁移面）。
    #[test]
    fn snapshot_roundtrip_preserves_ledgers() {
        let mut s = store_with_tree();
        // 先造真段（restore 的引用完整性要求指令影响面指向真实段账本）
        let seg = s
            .append_section_segment("SEC-A", "受影响段正文".into(), "k1")
            .unwrap();
        let reg = s
            .register_addendum_changes(&[spec_value_changed("REQ-0001", "旧", "新")])
            .unwrap();
        s.store_rewrite_directive(
            &reg[0].change,
            "SEC-A",
            vec![AffectedSegment {
                segment_id: seg.segment.id.clone(),
                seq: seg.segment.seq,
                matched_by: "citation".into(),
            }],
        );
        let snap = s.to_snapshot();
        assert_eq!(snap.addendum_changes.len(), 1);
        assert_eq!(snap.rewrite_directives.len(), 1);
        let restored = DomainStore::restore(snap).unwrap();
        assert_eq!(restored.addendum_changes().len(), 1);
        assert_eq!(restored.rewrite_directives().len(), 1);
        assert_eq!(
            restored.rewrite_directives()[0].affected_segments[0].segment_id,
            seg.segment.id
        );
    }

    /// ChangeType 键解析（MCP 入参形态）。
    #[test]
    fn change_type_keys() {
        assert_eq!(
            ChangeType::from_key("value_changed"),
            Some(ChangeType::ValueChanged)
        );
        assert_eq!(
            ChangeType::from_key("requirement_added"),
            Some(ChangeType::RequirementAdded)
        );
        assert_eq!(
            ChangeType::from_key("requirement_removed"),
            Some(ChangeType::RequirementRemoved)
        );
        assert_eq!(ChangeType::from_key("modified"), None, "非法键显式拒");
        assert_eq!(
            serde_json::to_string(&ChangeType::ValueChanged).unwrap(),
            "\"value_changed\""
        );
    }
}
