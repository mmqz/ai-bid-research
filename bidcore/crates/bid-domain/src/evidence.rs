//! evidence.rs —— T-1 Evidence 证据台账（issue #11 批次 v3 / V3-1，最高优先空洞）。
//!
//! 现状补齐：全仓此前 `grep "Evidence"` 只命中测试目录名——引擎 `evidence`
//! crate 的 Locator/指纹/sidecar 语义在标书侧完全没接。本模块落**实体 + 台账
//! 操作**（工具面 G-8/G-9 属 V3-6，不在本批）。
//!
//! 状态机纪律（borrow：mdcore `evidence.rs::degrade_linkage` 只降不升）：
//! - [`EvidenceStatus`] 迁移单向：Active → Expiring → Expired，任何 → Superseded；
//!   **任何 → Active 都被拒**（过期证据无静默复活路径）；
//! - 续期/换证 = **新版本记录**（version+1，旧记录 Superseded）——mdcore
//!   `reverify` 唯一恢复通道在本仓映射为「重新登记新版本」，语义等价且留全审计
//!   （新旧记录并存，不覆盖历史）。
//!
//! 有效期口径（R-03 + domain-model §1 Evidence）：
//! - `valid_until < 开标日` → Expired，**禁止绑定**（E_UNPROCESSABLE）；
//! - 开标日 ≤ valid_until < 开标日+90 天 → Expiring（可绑，告警面）；
//! - 无 valid_until（营业执照/业绩合同等长期件）→ 不参与时间翻转；
//! - 日期 = ISO 字符串字典序比对（仓内既定口径，见 [`crate::dates`]）。
//!
//! 主体核验：`subject_name != 投标人名称` → 告警（不阻断——主体不符可能是
//! 集团/分公司件，留给人工裁决，诚实登记不伪装）。

use crate::dates;
use crate::locator::{fingerprint_algorithm, EngineLocator};
use crate::{DomainStore, ToolError};
use serde::{Deserialize, Serialize};

/// 即将到期窗口（天）：开标日前 90 天内到期 → Expiring（可绑 + 告警）。
pub const EXPIRING_WINDOW_DAYS: i64 = 90;

// ---------------------------------------------------------------- 类型面
/// 证据类别（issue #11 批次 v3 T-1 规格：资质|业绩|人员|证书|产品资料|方案模块|合同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Qualification,
    Performance,
    Personnel,
    Certificate,
    ProductDoc,
    SolutionModule,
    Contract,
}

impl EvidenceKind {
    /// 展示名（中文——对齐 GT 夹具命名面）。
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceKind::Qualification => "资质",
            EvidenceKind::Performance => "业绩",
            EvidenceKind::Personnel => "人员",
            EvidenceKind::Certificate => "证书",
            EvidenceKind::ProductDoc => "产品资料",
            EvidenceKind::SolutionModule => "方案模块",
            EvidenceKind::Contract => "合同",
        }
    }
}

/// 证据记录状态（T-1 规格：Active|Expiring|Expired|Superseded）。
///
/// 注意：domain-model §1 的 MISSING / CONFLICT 是**矩阵行视角**（某条款无证据/
/// 证据相抵），不属记录级——本枚举只表达单条证据记录自身的状态，MISSING 在
/// T-2 MatrixRow 落地时以「无绑定」表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceStatus {
    Active,
    Expiring,
    Expired,
    Superseded,
}

impl EvidenceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceStatus::Active => "ACTIVE",
            EvidenceStatus::Expiring => "EXPIRING",
            EvidenceStatus::Expired => "EXPIRED",
            EvidenceStatus::Superseded => "SUPERSEDED",
        }
    }

    /// 可绑定面：Active / Expiring 可绑；Expired（R-03）/ Superseded（已被换版）禁绑。
    pub fn can_bind(&self) -> bool {
        matches!(self, EvidenceStatus::Active | EvidenceStatus::Expiring)
    }

    /// 单向降级迁移（borrow：mdcore `evidence.rs::linkage_only_degrades_never_recovers`）。
    ///
    /// 合法迁移：Active→{Expiring, Expired, Superseded}、Expiring→{Expired, Superseded}、
    /// Expired→{Superseded}（过期件被换版淘汰）；**任何 → Active 拒**（静默复活禁令）、
    /// 失效态互转（Expired→Expiring 等）拒、同态 no-op。
    /// 返回是否发生迁移（false = 拒绝或无变化——调用方不得当作成功）。
    pub fn degrade_to(&mut self, to: EvidenceStatus) -> bool {
        use EvidenceStatus::*;
        let allowed = matches!(
            (*self, to),
            (Active, Expiring) | (Active, Expired) | (Active, Superseded)
                | (Expiring, Expired) | (Expiring, Superseded)
                | (Expired, Superseded)
        );
        if allowed {
            *self = to;
        }
        allowed
    }
}

/// 证据台账记录（T-1 规格 struct 的 Rust 落地——字段对齐 issue #11 批次 v3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,               // EV-xxxx（store 单序列签发）
    pub kind: EvidenceKind,       // 资质|业绩|人员|证书|产品资料|方案模块|合同
    pub title: String,            // 证据标题（如「电子与智能化工程专业承包二级资质证书」）
    pub source_file: String,      // 原文件名（TenderFile.name 引用）
    pub source_sha256: String,    // 原文件 sha256（幂等/溯源面——引擎不做文件级指纹，标书侧算）
    pub locator: EngineLocator,   // 引擎锚点（可回溯原文）
    pub fingerprint: String,      // 定位时刻内容指纹（fnv64:xxxx——判内容变更）
    pub valid_from: Option<String>,  // ISO YYYY-MM-DD（None = 不适用）
    pub valid_until: Option<String>, // ISO YYYY-MM-DD（有效期——R-03 禁绑判定面）
    pub subject_name: String,     // 主体名称（与投标人比对——不符告警）
    pub scope: String,            // 适用范围（专业/区域/金额上限）
    pub version: u32,             // 换版代数（同主体同证新版本 = 旧版 Superseded）
    pub status: EvidenceStatus,
}

/// 证据 ↔ 条款绑定（最小面——T-2 MatrixRow 落地后由矩阵行消费）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceBinding {
    pub evidence_id: String,
    pub requirement_id: String,
}

/// 登记规格（幂等键 = (kind, title, subject_name, scope) 语义四元组）。
#[derive(Debug, Clone)]
pub struct EvidenceSpec {
    pub kind: EvidenceKind,
    pub title: String,
    pub source_file: String,
    pub source_sha256: String,
    pub locator: EngineLocator,
    pub fingerprint: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub subject_name: String,
    pub scope: String,
}

// ---------------------------------------------------------------- 纯逻辑面
/// 时间驱动的目标态计算（纯函数——不碰存储）。
///
/// - `valid_until < eval_date` → Expired；
/// - `eval_date ≤ valid_until < eval_date + 90 天` → Expiring；
/// - 其余（含无有效期件）→ None（不翻转——长期证/业绩合同不参与时间面）。
pub fn validity_flip(valid_until: Option<&str>, eval_date: &str) -> Option<EvidenceStatus> {
    let vu = valid_until?;
    if vu < eval_date {
        Some(EvidenceStatus::Expired)
    } else {
        let horizon = dates::add_days(eval_date, EXPIRING_WINDOW_DAYS)
            .unwrap_or_else(|| eval_date.to_string());
        if vu < horizon.as_str() {
            Some(EvidenceStatus::Expiring)
        } else {
            None
        }
    }
}

/// 主体核验（纯函数）：主体名与投标人不符 → 告警文案（None = 一致）。
pub fn subject_warning(ev: &Evidence, bidder_name: &str) -> Option<String> {
    if ev.subject_name != bidder_name {
        Some(format!(
            "EVIDENCE_SUBJECT_MISMATCH: {} 主体「{}」与投标人「{}」不符（疑似集团/分公司件——人工裁决）",
            ev.id, ev.subject_name, bidder_name
        ))
    } else {
        None
    }
}

/// 登记规格校验（纯函数——错误文案即 E_UNPROCESSABLE reason）。
fn validate_spec(spec: &EvidenceSpec) -> Result<(), ToolError> {
    let un = |reason: String| ToolError::EUnprocessable { reason };
    if spec.title.trim().is_empty() {
        return Err(un("证据标题不可为空".into()));
    }
    if spec.source_file.trim().is_empty() {
        return Err(un("证据原文件引用不可为空".into()));
    }
    if spec.source_sha256.trim().is_empty() {
        return Err(un("证据原文件 sha256 不可为空（无文件身份不入账）".into()));
    }
    if spec.subject_name.trim().is_empty() {
        return Err(un("证据主体名称不可为空（主体核验需要）".into()));
    }
    fingerprint_algorithm(&spec.fingerprint)
        .map_err(|e| un(format!("证据内容指纹格式非法: {e}")))?;
    validate_locator(&spec.locator).map_err(un)?;
    for (label, d) in [("valid_from", &spec.valid_from), ("valid_until", &spec.valid_until)] {
        if let Some(v) = d {
            if !dates::is_valid_ymd(v) {
                return Err(un(format!("{label} 非法 ISO 日期: {v:?}")));
            }
        }
    }
    if let (Some(f), Some(u)) = (&spec.valid_from, &spec.valid_until) {
        if f > u {
            return Err(un(format!("valid_from({f}) 晚于 valid_until({u})")));
        }
    }
    Ok(())
}

/// 定位器结构校验（坐标域 + 区域端点序——构造面防线的最后一环）。
pub fn validate_locator(loc: &EngineLocator) -> Result<(), String> {
    match loc {
        EngineLocator::XlsxCell { cell, .. } => {
            CellCoordLike::validate(cell)
        }
        EngineLocator::XlsxRegion { start, end, .. } => {
            CellCoordLike::validate(start)
                .and_then(|_| CellCoordLike::validate(end))
                .and_then(|_| crate::locator::validate_region(start, end)
                    .map_err(|e| format!("区域端点逆序: {e:?}")))
        }
        _ => Ok(()),
    }
}

/// 坐标串域校验（绕过 `CellCoord::parse` 直接构造的兜底——pub 字段不可信任）。
struct CellCoordLike;
impl CellCoordLike {
    fn validate(c: &crate::locator::CellCoord) -> Result<(), String> {
        crate::locator::CellCoord::parse(&c.0)
            .map(|_| ())
            .map_err(|_| format!("坐标域非法: {:?}", c.0))
    }
}

// ---------------------------------------------------------------- DomainStore 扩面（T-1 台账操作）
/// 语义幂等键：同主体 + 同证 + 同范围 = 同一证据语义位（换证 = 该位上版本递增）。
fn semantic_key(ev: &Evidence) -> (EvidenceKind, String, String, String) {
    (ev.kind, ev.title.clone(), ev.subject_name.clone(), ev.scope.clone())
}

impl DomainStore {
    /// 台账只读面。
    pub fn evidences(&self) -> &[Evidence] {
        &self.evidences
    }

    pub fn evidence(&self, id: &str) -> Result<&Evidence, ToolError> {
        self.evidences.iter().find(|e| e.id == id).ok_or_else(|| ToolError::ENotFound {
            what: format!("evidence {id} 不存在"),
        })
    }

    pub fn evidence_bindings(&self) -> &[EvidenceBinding] {
        &self.evidence_bindings
    }

    /// 登记证据（幂等 + 换版）。
    ///
    /// - 语义四元组 (kind, title, subject, scope) 相同且指纹+双日期全同 → 重放
    ///   no-op（deduplicated: true，content_version 不动）；
    /// - 语义位已占用但指纹/日期有变 → **换版**：旧记录 Superseded（经单向降级
    ///   迁移——Expired→Superseded 亦合法），新记录 version = 旧+1；
    /// - 新语义位 → 直接入账，version = 1。
    ///
    /// 出生态一律 Active——时间面翻转（Expired/Expiring）由 [`Self::apply_validity`]
    /// 与 [`Self::bind_evidence`] 携带的开标日裁决（登记时不知道评标基准日）。
    pub fn register_evidence(&mut self, spec: EvidenceSpec) -> Result<(String, bool), ToolError> {
        self.project()?; // 台账挂在项目下（M1 单项目存储）
        validate_spec(&spec)?;
        let key = (spec.kind, spec.title.clone(), spec.subject_name.clone(), spec.scope.clone());
        // 语义位查重：同 key 的最新非 Superseded 记录（rfind——从尾向头找最新版）
        if let Some(existing) = self.evidences.iter()
            .rfind(|e| semantic_key(e) == key && e.status != EvidenceStatus::Superseded)
        {
            let same_payload = existing.fingerprint == spec.fingerprint
                && existing.valid_from == spec.valid_from
                && existing.valid_until == spec.valid_until;
            if same_payload {
                return Ok((existing.id.clone(), true)); // 重放 no-op
            }
            // 换版：旧记录单向降级到 Superseded
            let old_id = existing.id.clone();
            let old_version = existing.version;
            let idx = self.evidences.iter().position(|e| e.id == old_id).unwrap();
            // Superseded 是一切状态的合法去向（含 Expired→Superseded：过期件被换版淘汰）
            self.evidences[idx].status = EvidenceStatus::Superseded;
            let id = self.next_id("EV");
            self.evidences.push(Evidence {
                id: id.clone(),
                kind: spec.kind,
                title: spec.title,
                source_file: spec.source_file,
                source_sha256: spec.source_sha256,
                locator: spec.locator,
                fingerprint: spec.fingerprint,
                valid_from: spec.valid_from,
                valid_until: spec.valid_until,
                subject_name: spec.subject_name,
                scope: spec.scope,
                version: old_version + 1,
                status: EvidenceStatus::Active,
            });
            if let Some(p) = self.project.as_mut() {
                p.content_version += 1;
            }
            return Ok((id, false));
        }
        // 新语义位
        let id = self.next_id("EV");
        self.evidences.push(Evidence {
            id: id.clone(),
            kind: spec.kind,
            title: spec.title,
            source_file: spec.source_file,
            source_sha256: spec.source_sha256,
            locator: spec.locator,
            fingerprint: spec.fingerprint,
            valid_from: spec.valid_from,
            valid_until: spec.valid_until,
            subject_name: spec.subject_name,
            scope: spec.scope,
            version: 1,
            status: EvidenceStatus::Active,
        });
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok((id, false))
    }

    /// 批量有效期裁决（开标日口径）：时间驱动的单向翻转 + 翻转清单（审计面）。
    ///
    /// 幂等：已达目标态的记录不动（同态迁移被 `degrade_to` 拒）；
    /// 只降不升：一旦 Expired，改回未来日期也不复活（复活 = 重新登记新版本）。
    /// 翻转 ≥1 条 → content_version +1（真实状态变化）。
    pub fn apply_validity(&mut self, bid_open_date: &str) -> Result<Vec<String>, ToolError> {
        self.project()?;
        if !dates::is_valid_ymd(bid_open_date) {
            return Err(ToolError::EUnprocessable {
                reason: format!("bid_open_date 非法 ISO 日期: {bid_open_date:?}"),
            });
        }
        let mut flipped = Vec::new();
        for ev in self.evidences.iter_mut() {
            if let Some(target) = validity_flip(ev.valid_until.as_deref(), bid_open_date) {
                let before = ev.status;
                if ev.status.degrade_to(target) {
                    flipped.push(format!("{}: {} -> {}", ev.id, before.as_str(), ev.status.as_str()));
                }
            }
        }
        if !flipped.is_empty() {
            if let Some(p) = self.project.as_mut() {
                p.content_version += 1;
            }
        }
        Ok(flipped)
    }

    /// 绑定证据到条款（R-03 闸门：EXPIRED/SUPERSEDED 禁绑 → E_UNPROCESSABLE）。
    ///
    /// 携带开标日做即时有效性裁决（绑定时刻 = 评标口径，与 [`Self::apply_validity`]
    /// 同一翻转函数，状态变化立即生效）。重复绑定 → 幂等 no-op（true）。
    pub fn bind_evidence(&mut self, ev_id: &str, requirement_id: &str, bid_open_date: &str)
        -> Result<bool, ToolError>
    {
        self.project()?;
        if !dates::is_valid_ymd(bid_open_date) {
            return Err(ToolError::EUnprocessable {
                reason: format!("bid_open_date 非法 ISO 日期: {bid_open_date:?}"),
            });
        }
        if !self.requirements.iter().any(|r| r.id == requirement_id) {
            return Err(ToolError::ENotFound {
                what: format!("requirement {requirement_id} 不存在"),
            });
        }
        // 即时有效性裁决（开标日口径）
        if let Some(target) = self.evidences.iter()
            .find(|e| e.id == ev_id)
            .and_then(|e| validity_flip(e.valid_until.as_deref(), bid_open_date))
        {
            let idx = self.evidences.iter().position(|e| e.id == ev_id).unwrap();
            self.evidences[idx].status.degrade_to(target); // 只降不升；同态 no-op
        }
        let ev = self.evidence(ev_id)?; // E_NOTFOUND 面
        if !ev.status.can_bind() {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "证据 {} 状态 {} 禁止绑定（R-03：EXPIRED/SUPERSEDED 禁绑；续期请重新登记新版本）",
                    ev_id, ev.status.as_str()
                ),
            });
        }
        if self.evidence_bindings.iter()
            .any(|b| b.evidence_id == ev_id && b.requirement_id == requirement_id)
        {
            return Ok(true); // 幂等重放
        }
        self.evidence_bindings.push(EvidenceBinding {
            evidence_id: ev_id.into(),
            requirement_id: requirement_id.into(),
        });
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(false)
    }

    /// 台账主体核验：全部主体不符项的告警清单（导入对账面——不阻断）。
    pub fn subject_warnings(&self, bidder_name: &str) -> Vec<String> {
        self.evidences.iter()
            .filter(|e| e.status != EvidenceStatus::Superseded)
            .filter_map(|e| subject_warning(e, bidder_name))
            .collect()
    }

    /// 绑定可用性快照（矩阵构建输入——T-2 将消费）：可绑证据 id 清单。
    pub fn bindable_evidence_ids(&self, bid_open_date: &str) -> Vec<String> {
        self.evidences.iter()
            .filter(|e| {
                validity_flip(e.valid_until.as_deref(), bid_open_date)
                    .is_none_or(|t| t != EvidenceStatus::Expired) // 未翻转或翻转目标非 Expired
                    && matches!(e.status, EvidenceStatus::Active | EvidenceStatus::Expiring)
            })
            .map(|e| e.id.clone())
            .collect()
    }
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::locator::{CellCoord, EngineLocator};

    /// 台账基座：项目 + 已抽条款（绑定需要 REQ 存在）。
    fn store_with_reqs() -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("测试项目", "ZFCG-2026-GK-0311", "明标", "ext-key-ev").unwrap();
        s.upsert_file("A-招标文件.docx", "招标文件", "aa", 31, 12, 120, 0,
                      crate::ParseStatus::Ok, "").unwrap();
        let reqs = vec![crate::Requirement {
            id: "REQ-0002".into(), category: "资格条件".into(), clause_no: Some("3.2.2".into()),
            anchors: vec![crate::Anchor { file: "A".into(), kind: "para".into(),
                locator: "第三章".into(), excerpt: "具备安全生产许可证".into() }],
            params: std::collections::HashMap::new(), source_file: "A".into(),
        }];
        s.set_requirements(reqs).unwrap();
        s
    }

    /// GT-13 语义件：安全生产许可证，valid_until=2025-08-01（已过期）。
    fn gt13_spec() -> EvidenceSpec {
        EvidenceSpec {
            kind: EvidenceKind::Certificate,
            title: "安全生产许可证".into(),
            source_file: "EV-13-安全生产许可证.pdf".into(),
            source_sha256: "cafebabe01".into(),
            locator: EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:13".into() },
            fingerprint: "fnv64:13".into(),
            valid_from: Some("2022-08-01".into()),
            valid_until: Some("2025-08-01".into()),
            subject_name: "智慧园区科技有限公司".into(),
            scope: "全国".into(),
        }
    }

    /// GT-02 语义件：资质证书，valid_until=2026-11-25（开标日附近）。
    fn gt02_spec() -> EvidenceSpec {
        EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "电子与智能化工程专业承包二级资质证书".into(),
            source_file: "EV-02-电子与智能化工程专业承包二级资质证书.pdf".into(),
            source_sha256: "cafebabe02".into(),
            locator: EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:02".into() },
            fingerprint: "fnv64:02".into(),
            valid_from: Some("2021-11-25".into()),
            valid_until: Some("2026-11-25".into()),
            subject_name: "智慧园区科技有限公司".into(),
            scope: "全国".into(),
        }
    }

    // ============================ 借鉴测试（mdcore evidence crate 只降不升纪律） ============================

    /// borrow：mdcore `evidence.rs::linkage_only_degrades_never_recovers`（298-313）
    /// ——状态机单向性：降级放行、上升拒绝、失效态互转拒绝、同态 no-op。
    #[test]
    fn evidence_status_only_degrades_never_recovers() {
        use EvidenceStatus::*;
        // 合法降级
        let mut s = Active;
        assert!(s.degrade_to(Expiring), "Active→Expiring 放行");
        assert!(s.degrade_to(Expired), "Expiring→Expired 放行");
        assert!(s.degrade_to(Superseded), "Expired→Superseded 放行（过期件换版淘汰）");
        assert_eq!(s, Superseded);
        // 上升全拒
        let mut e = Expired;
        assert!(!e.degrade_to(Active), "Expired→Active 拒（静默复活禁令）");
        assert!(!e.degrade_to(Expiring), "Expired→Expiring 拒（失效态互转）");
        assert_eq!(e, Expired, "拒绝后状态不变");
        // 同态 no-op
        let mut a = Active;
        assert!(!a.degrade_to(Active), "同态迁移 = 无变化");
        // 跨级跳（Active→Expired 直接过期）合法——补遗把开标日提前的场景
        let mut a2 = Active;
        assert!(a2.degrade_to(Expired), "Active→Expired 跨级放行（开标日提前）");
    }

    // ============================ GT 驱动测试（corpus-A evidence-links.jsonl 黄金数据） ============================

    /// GT-02 双态翻转：`eligible@2026-11-15;EXPIRED@2026-11-27`——同一证据对两个
    /// 评标基准日的确定性结论（10 天窗口内 Expiring 可绑 / 过界 Expired 禁绑）。
    #[test]
    fn gt02_validity_flips_between_dual_eval_dates() {
        let mut s = store_with_reqs();
        let (ev_id, _) = s.register_evidence(gt02_spec()).unwrap();
        // @2026-11-15：valid_until(11-25) ≥ 开标日 → 未过期；且 < 开标日+90 天 → Expiring（可绑）
        let flipped = s.apply_validity("2026-11-15").unwrap();
        assert_eq!(flipped, vec!["EV-0003: ACTIVE -> EXPIRING".to_string()],
            "GT-02 @2026-11-15 → Expiring（10 天内到期，可绑 = eligible）");
        assert!(s.evidence(&ev_id).unwrap().status.can_bind(), "Expiring 可绑");
        // 幂等：同日再裁决不动
        assert!(s.apply_validity("2026-11-15").unwrap().is_empty(), "同态迁移被拒（幂等）");
        // @2026-11-27：valid_until(11-25) < 开标日 → Expired
        let flipped2 = s.apply_validity("2026-11-27").unwrap();
        assert_eq!(flipped2, vec!["EV-0003: EXPIRING -> EXPIRED".to_string()],
            "GT-02 @2026-11-27 → EXPIRED");
        assert!(!s.evidence(&ev_id).unwrap().status.can_bind(), "过期禁绑");
    }

    /// GT-13 过期禁绑（`forbidden_bind: true`）：valid_until=2025-08-01 对 2026 开标
    /// → EXPIRED + 绑定 → E_UNPROCESSABLE（R-03）。
    #[test]
    fn gt13_expired_evidence_forbidden_to_bind() {
        let mut s = store_with_reqs();
        let (ev_id, _) = s.register_evidence(gt13_spec()).unwrap();
        // 直接绑定（携带开标日做即时裁决——不经 apply_validity 前置）
        let err = s.bind_evidence(&ev_id, "REQ-0002", "2026-11-15").unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "GT-13 过期禁绑 → R-03");
        assert!(format!("{err:?}").contains("禁止绑定"));
        assert_eq!(s.evidence(&ev_id).unwrap().status, EvidenceStatus::Expired,
            "绑定尝试本身已触发即时翻转（开标日口径）");
        assert!(s.evidence_bindings().is_empty(), "拒绝路径零绑定残留");
    }

    /// 绑定正常路径 + 幂等重放 + REQ 不存在拒绝。
    #[test]
    fn bind_active_evidence_idempotent_and_validates_requirement() {
        let mut s = store_with_reqs();
        let (ev_id, _) = s.register_evidence(gt02_spec()).unwrap();
        // REQ 不存在 → E_NOTFOUND
        let err = s.bind_evidence(&ev_id, "REQ-9999", "2026-11-15").unwrap_err();
        assert_eq!(err.code(), "E_NOTFOUND");
        assert!(s.evidence_bindings().is_empty());
        // 正常绑定（2026-11-15 → Expiring 可绑）
        let dup1 = s.bind_evidence(&ev_id, "REQ-0002", "2026-11-15").unwrap();
        assert!(!dup1, "首次绑定 = 新绑定");
        assert_eq!(s.evidence_bindings().len(), 1);
        let v = s.content_version();
        // 幂等重放
        let dup2 = s.bind_evidence(&ev_id, "REQ-0002", "2026-11-15").unwrap();
        assert!(dup2, "重复绑定 = 幂等 no-op");
        assert_eq!(s.evidence_bindings().len(), 1, "无重复绑定记录");
        assert_eq!(s.content_version(), v, "no-op 不 bump content_version");
    }

    /// 登记幂等（重放 no-op）与换版（新版本 Superseded 旧版本）。
    #[test]
    fn register_replay_noop_and_renewal_supersedes() {
        let mut s = store_with_reqs();
        let (id1, dup) = s.register_evidence(gt13_spec()).unwrap();
        assert!(!dup);
        let v = s.content_version();
        // 全同重放 → no-op
        let (id2, dup2) = s.register_evidence(gt13_spec()).unwrap();
        assert_eq!(id1, id2, "重放返回原 id");
        assert!(dup2, "deduplicated");
        assert_eq!(s.content_version(), v, "重放不 bump version");
        assert_eq!(s.evidences().len(), 1);
        // 换版：新有效期 + 新指纹（证书续期 = 新内容）
        let mut renewed = gt13_spec();
        renewed.valid_until = Some("2028-08-01".into());
        renewed.fingerprint = "fnv64:13v2".into();
        let (id3, dup3) = s.register_evidence(renewed).unwrap();
        assert_ne!(id1, id3, "换版 = 新记录新 id");
        assert!(!dup3);
        assert_eq!(s.evidences().len(), 2, "新旧并存（审计不覆盖历史）");
        let old = s.evidence(&id1).unwrap();
        let new = s.evidence(&id3).unwrap();
        assert_eq!(old.status, EvidenceStatus::Superseded, "旧版被淘汰");
        assert_eq!(new.version, 2, "版本递增");
        assert_eq!(new.status, EvidenceStatus::Active, "新版 Active");
        assert_eq!(new.valid_until.as_deref(), Some("2028-08-01"));
        // Superseded 禁绑
        let err = s.bind_evidence(&id1, "REQ-0002", "2026-11-15").unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "Superseded 禁绑");
    }

    /// 无有效期件（营业执照/业绩合同）不参与时间翻转（GT-01/GT-03 语义）。
    #[test]
    fn perpetual_evidence_immune_to_validity_flip() {
        let mut s = store_with_reqs();
        let spec = EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "营业执照".into(),
            source_file: "EV-01-营业执照.pdf".into(),
            source_sha256: "cafebabe01x".into(),
            locator: EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:01".into() },
            fingerprint: "fnv64:01".into(),
            valid_from: None, valid_until: None,
            subject_name: "智慧园区科技有限公司".into(),
            scope: "全国".into(),
        };
        let (id, _) = s.register_evidence(spec).unwrap();
        assert!(s.apply_validity("2099-01-01").unwrap().is_empty(),
            "无 valid_until → 任何评标基准日都不翻转（GT-01 eligible 语义）");
        assert_eq!(s.evidence(&id).unwrap().status, EvidenceStatus::Active);
        s.bind_evidence(&id, "REQ-0002", "2099-01-01").unwrap();
        assert_eq!(s.evidence_bindings().len(), 1, "长期件可绑");
    }

    /// 主体核验告警（GT 姿态：不阻断，人工裁决面）。
    #[test]
    fn subject_mismatch_raises_warning_not_rejection() {
        let mut s = store_with_reqs();
        let mut spec = gt02_spec();
        spec.subject_name = "智慧园区科技集团股份有限公司".into(); // 集团件
        s.register_evidence(spec).unwrap();
        let warns = s.subject_warnings("智慧园区科技有限公司");
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("EVIDENCE_SUBJECT_MISMATCH"), "告警码可被调用方分支");
        assert!(warns[0].contains("集团"), "主体名双呈现（人工裁决素材）");
        // 一致时零告警
        assert!(s.subject_warnings("智慧园区科技集团股份有限公司").is_empty());
    }

    /// 登记参数校验面（诚实状态——非法输入 E_UNPROCESSABLE，零入库）。
    #[test]
    fn register_rejects_malformed_specs() {
        let mut s = store_with_reqs();
        let n_before = s.evidences().len();
        // 指纹缺算法前缀（裸值不可辨识算法）
        let mut sp0 = gt13_spec();
        sp0.fingerprint = "no-prefix-bare".into();
        let err = s.register_evidence(sp0).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "指纹缺前缀拒");
        assert_eq!(s.evidences().len(), n_before, "拒绝路径零入库");
        // 其余校验分支
        let mut sp1 = gt13_spec(); sp1.valid_until = Some("2026-13-40".into());
        assert!(s.register_evidence(sp1).is_err(), "非法 ISO 日期拒");
        let mut sp2 = gt13_spec(); sp2.valid_from = Some("2026-01-01".into()); sp2.valid_until = Some("2025-01-01".into());
        assert!(s.register_evidence(sp2).is_err(), "from > until 拒");
        let mut sp3 = gt13_spec(); sp3.title = "  ".into();
        assert!(s.register_evidence(sp3).is_err(), "空标题拒");
        let mut sp4 = gt13_spec(); sp4.subject_name = String::new();
        assert!(s.register_evidence(sp4).is_err(), "空主体拒");
        let mut sp5 = gt13_spec();
        sp5.locator = EngineLocator::XlsxRegion {
            sheet: "报价表".into(),
            start: CellCoord("B2".into()), end: CellCoord("A1".into()), // 逆序区域
            region_fingerprint: "fnv64:xx".into(),
        };
        assert!(s.register_evidence(sp5).is_err(), "区域端点逆序拒");
        // 台账前置：无项目 → E_NOTFOUND
        let mut empty = DomainStore::new();
        assert!(matches!(empty.register_evidence(gt13_spec()), Err(ToolError::ENotFound { .. })));
    }

    /// bindable 快照面：T-2 矩阵构建的输入（GT-02 可绑 / GT-13 不可绑并存的判定）。
    #[test]
    fn bindable_snapshot_respects_validity() {
        let mut s = store_with_reqs();
        let (id02, _) = s.register_evidence(gt02_spec()).unwrap();
        let (id13, _) = s.register_evidence(gt13_spec()).unwrap();
        let ids = s.bindable_evidence_ids("2026-11-15");
        assert!(ids.contains(&id02), "GT-02 @11-15 可绑（Expiring）");
        assert!(!ids.contains(&id13), "GT-13 @11-15 不可绑（Expired）");
    }
}
