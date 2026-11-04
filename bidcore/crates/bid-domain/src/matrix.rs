//! matrix.rs —— T-2 MatrixRow 需求响应矩阵（issue #11 批次 v3 / V3-2）。
//!
//! 定位：REQ（招标要求）× 响应状态 的权威账本——导出闸门的判定面
//! （**blocking=true 的行未清零 → 禁止导出**，R-03/R-04 闸门）。
//!
//! 状态派生（单一真源 [`derive_matrix_rows`]——工具面/导出门/持久化共用）：
//! - `RESPONDED`（已响应）：显式 override 标记，或「证据在绑 + 章节在位」；
//! - `PARTIAL`（部分）：有证据绑定但响应章节缺位（材料齐、正文未落）；
//! - `NOT_RESPONDED`（未响应）：无证据无章节——**必选类（资格条件/否决项）即 blocking**；
//! - `NOT_APPLICABLE`（不适用）：显式 override（带偏离说明——不适用必须给理由）。
//!
//! blocking 语义（Lead T-2 规格 + 契约 mandatory 口径对齐）：
//! `category ∈ {资格条件, 否决项} && status == NOT_RESPONDED` → blocking=true
//! （与 get_compliance_matrix 的 `mandatory` 判定同源——工具面的 mandatory 即本门输入）。
//!
//! 派生 vs 覆写：矩阵行是**派生态**（从 requirements + bindings + sections + overrides
//! 重算，幂等）；人工裁决只经 [`DomainStore::set_matrix_override`]（不适用/偏离说明）
//! 落 overrides 集合——派生输入不变则重算结果不变（可复现）。

use crate::evidence::{EvidenceStatus, Evidence};
use crate::outline::OutlineNode;
use crate::principal::TrustedPrincipal;
use crate::{DomainStore, ProjectStatus, Requirement, ToolError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------- 类型面
/// 响应状态（T-2 规格：未响应|已响应|部分|不适用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResponseStatus {
    NotResponded,
    Responded,
    Partial,
    NotApplicable,
}

impl ResponseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResponseStatus::NotResponded => "NOT_RESPONDED",
            ResponseStatus::Responded => "RESPONDED",
            ResponseStatus::Partial => "PARTIAL",
            ResponseStatus::NotApplicable => "NOT_APPLICABLE",
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
    pub status: ResponseStatus,
    /// 偏离说明（NOT_APPLICABLE 必填——不适用必须给理由，诚实状态）
    pub deviation: Option<String>,
    /// 必选未响应 → 阻断导出（R-03/R-04 闸门）
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
/// - 行序 = requirements 序（稳定——REQ 集合幂等合并后顺序确定）；
/// - id = `CMP-{REQ 后缀}` 派生稳定（见 [`derived_cmp_id`]）；
/// - 12,000 行量级：全量派生 O(N+B+S)，无逐行查找平方面。
pub fn derive_matrix_rows(
    reqs: &[Requirement],
    bindings: &[(String, String)], // (evidence_id, requirement_id)
    sections: &[OutlineNode],
    overrides: &HashMap<String, MatrixOverride>,
) -> Vec<MatrixRow> {
    // 绑定与章节的反查索引（一次建表，避免逐 REQ 全扫）
    let mut ev_by_req: HashMap<&str, Vec<&str>> = HashMap::new();
    for (ev_id, req_id) in bindings {
        ev_by_req.entry(req_id.as_str()).or_default().push(ev_id.as_str());
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
    reqs.iter().map(|r| {
        let evs = ev_by_req.get(r.id.as_str()).cloned().unwrap_or_default();
        let sec = sec_by_req.get(r.id.as_str()).copied();
        // 状态派生：override > 证据+章节
        let (status, deviation) = match overrides.get(&r.id) {
            Some(o) => (o.status, o.deviation.clone()),
            None => {
                let has_ev = !evs.is_empty();
                let has_sec = sec.is_some();
                let st = match (has_ev, has_sec) {
                    (true, true) => ResponseStatus::Responded,
                    (true, false) => ResponseStatus::Partial,
                    _ => ResponseStatus::NotResponded,
                };
                (st, None)
            }
        };
        let blocking = is_mandatory(&r.category) && status == ResponseStatus::NotResponded;
        MatrixRow {
            id: derived_cmp_id(&r.id),
            requirement_id: r.id.clone(),
            response_section: sec.map(|s| s.to_string()),
            evidence_ids: evs.into_iter().map(|e| e.to_string()).collect(),
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
        let bindings: Vec<(String, String)> = self.evidence_bindings.iter()
            .map(|b| (b.evidence_id.clone(), b.requirement_id.clone())).collect();
        let overrides = self.matrix_overrides.clone();
        let rows = derive_matrix_rows(&self.requirements, &bindings, &self.sections, &overrides);
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
        let live_bindings: Vec<(String, String)> = self.evidence_bindings.iter()
            .filter(|b| self.evidences.iter().any(|e| {
                e.id == b.evidence_id && matches!(e.status, EvidenceStatus::Active | EvidenceStatus::Expiring)
            }))
            .map(|b| (b.evidence_id.clone(), b.requirement_id.clone()))
            .collect();
        let overrides = self.matrix_overrides.clone();
        derive_matrix_rows(&self.requirements, &live_bindings, &self.sections, &overrides)
    }
}

/// 证据列组装素材（契约 §4 `rows[].evidence:[{ev_id, title, status, valid_until}]`）。
pub fn evidence_column(evidences: &[Evidence], ev_ids: &[String]) -> Vec<serde_json::Value> {
    ev_ids.iter().filter_map(|id| {
        evidences.iter().find(|e| &e.id == id).map(|e| serde_json::json!({
            "ev_id": e.id,
            "title": e.title,
            "status": e.status.as_str(),
            "valid_until": e.valid_until,
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
        let rows = derive_matrix_rows(&reqs, &bindings, &sections, &overrides);
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
        // EV-0003 需在台账（matrix_view 活证据过滤）——build_matrix 用全量绑定（含过期证据
        // 由 R-03 绑定时刻把关，视图层过滤），此处直接放行验证闸门翻转
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
}
