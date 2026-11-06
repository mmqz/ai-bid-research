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
            (Active, Expiring)
                | (Active, Expired)
                | (Active, Superseded)
                | (Expiring, Expired)
                | (Expiring, Superseded)
                | (Expired, Superseded)
        );
        if allowed {
            *self = to;
        }
        allowed
    }
}

/// 结构化证据字段（#147 search_evidence exact 通道的结构化真源）。
///
/// 全部 `Option` + `#[serde(default)]`：缺省 = 未录入（绝不落 0/空串——与
/// `client`/`amount_wan`/`completed_on` 同款诚实状态纪律）；旧快照/旧调用方
/// 不传 `fields` 时零影响（向后兼容）。
///
/// 字段按证据三族分组：
/// - **合同/业绩类**：project_name / client / amount_wan / signed_at / finished_at
/// - **证书/资质类**：cert_name / cert_no / issuer
/// - **人员类**：person_name / person_title / years
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EvidenceFields {
    // ---- 合同/业绩类
    /// 项目名称（业绩合同标的项目）。
    pub project_name: Option<String>,
    /// 发包人/建设单位（与 Evidence.client 业务面同义，此处为结构化查询面）。
    pub client: Option<String>,
    /// 合同金额（万元）。
    pub amount_wan: Option<f64>,
    /// 签订日期（YYYY-MM-DD）。
    pub signed_at: Option<String>,
    /// 完成/竣工日期（YYYY-MM-DD）。
    pub finished_at: Option<String>,
    // ---- 证书/资质类
    /// 证书名称。
    pub cert_name: Option<String>,
    /// 证书编号。
    pub cert_no: Option<String>,
    /// 发证机关。
    pub issuer: Option<String>,
    // ---- 人员类
    /// 人员姓名。
    pub person_name: Option<String>,
    /// 职称（如「高级工程师」）。
    pub person_title: Option<String>,
    /// 从业年限。
    pub years: Option<u32>,
}

/// 从业年限上界（超过即非法——防止把「2015」当年的笔误塞进年限面）。
pub const MAX_YEARS: u32 = 100;

impl EvidenceFields {
    /// 结构化字段校验（纯函数——错误文案即 E_UNPROCESSABLE reason 的素材）。
    ///
    /// - `signed_at` / `finished_at` 必须为严格 `YYYY-MM-DD`（{@link dates::is_valid_ymd}）；
    /// - `amount_wan` 必须有限且 `>= 0`（NaN/∞/负数拒）；
    /// - `years` 必须 `<= MAX_YEARS`（"#" 100 年上界）。
    ///
    /// 任一项非法 → `Err(说明)`；全 `None`（空结构）恒合法。
    pub fn validate(&self) -> Result<(), String> {
        for (label, d) in [
            ("signed_at", &self.signed_at),
            ("finished_at", &self.finished_at),
        ] {
            if let Some(v) = d {
                if !dates::is_valid_ymd(v) {
                    return Err(format!(
                        "fields.{label} 非法 ISO 日期: {v:?}（须 YYYY-MM-DD）"
                    ));
                }
            }
        }
        if let Some(a) = self.amount_wan {
            if !a.is_finite() {
                return Err(format!("fields.amount_wan 非法数值: {a}（须有限数）"));
            }
            if a < 0.0 {
                return Err(format!("fields.amount_wan 为负: {a}（须 >= 0，万元口径）"));
            }
        }
        if let Some(y) = self.years {
            if y > MAX_YEARS {
                return Err(format!(
                    "fields.years={y} 越界（须 <= {MAX_YEARS} 年）"
                ));
            }
        }
        Ok(())
    }

    /// 是否为空结构（全 None——「无结构化字段」判定，search_evidence 回落文本匹配用）。
    pub fn is_empty(&self) -> bool {
        *self == EvidenceFields::default()
    }
}

/// 证据内容验证五态（#217 / #185 缺口 2 + #245 占位态）——
/// 「找到证据」≠「读过证据」≠「资质已核实」的三层区分面。
///
/// | 态 | 含义 | 达成路径 |
/// |---|---|---|
/// | [`Placeholder`] | **占位/待核验**：材料内容未经核验，不得当作已验证证据（#245——占位不得清除合规阻塞，升级只能经换版重登） | 登记面 `placeholder: true`（调用方可自报占位——诚实方向；但**不能自报任何更高态**） |
/// | [`FileExists`] | 来源文件在库且非失败件，但**正文未被定位读取** | `register_evidence` 无 locator（文件级锚） |
/// | [`ContentMatches`] | 定位文本与解析产物**逐字对账通过** + 摘录入账 | 带 locator 且指纹一致（服务端 `bid_tools::BidService::register_evidence_verified` 裁决） |
/// | [`SemanticSupport`] | 摘录与登记语义（标题/范围）经语义比对互证 | **预留**（M2-b 语义服务落地前不可达——枚举先立面，防第三态悄悄冒充第四态） |
/// | [`HumanConfirmed`] | 人工确认摘录与原件一致 | **预留**（接 #182 TrustedPrincipal receipt 链路；当前不可经登记面设置） |
///
/// 裁决纪律：tier 由**服务端**在登记时刻写入（域层纯透传），调用方无从自报——
/// 与 #185 缺口 1 的 sha256/指纹同款「不信调用方」纪律。唯一例外是
/// `Placeholder`（#245）：调用方**可以声明占位**（从低不从高的诚实方向——
/// 自报“未核验”永远安全，自报“已核验”才危险），但 `Placeholder` 在矩阵
/// 派生/导出门中不作为有效证据（见 [`crate::matrix`] 的 PENDING_EVIDENCE 态），
/// 升级为真实证据只能经**换版重登**（新指纹 → 新记录，旧记录 Superseded
/// 留审计痕——无原地迁移通道即无非法迁移通道）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerificationTier {
    /// 占位/待核验（#245）：内容未经核验——可携带定位与摘录（可回溯占位
    /// 登记指向了什么），但**不得作为已验证证据清阻塞/导出**。
    Placeholder,
    /// 文件级锚：来源在库、未失败，但正文未定位（无摘录）。
    FileExists,
    /// 内容级锚：定位文本与解析产物逐字一致，摘录（excerpt）入账。
    ContentMatches,
    /// 语义互证（预留——M2-b）。
    SemanticSupport,
    /// 人工确认（预留——#182 receipt 链路）。
    HumanConfirmed,
}

impl VerificationTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerificationTier::Placeholder => "PLACEHOLDER",
            VerificationTier::FileExists => "FILE_EXISTS",
            VerificationTier::ContentMatches => "CONTENT_MATCHES",
            VerificationTier::SemanticSupport => "SEMANTIC_SUPPORT",
            VerificationTier::HumanConfirmed => "HUMAN_CONFIRMED",
        }
    }

    /// 正文是否已实际读取（CONTENT_MATCHES 及以上——SEMANTIC_SUPPORT/
    /// HUMAN_CONFIRMED 是其严格超集：语义/人工确认必先有逐字摘录；
    /// PLACEHOLDER 即便携带摘录也不算——占位材料的摘录是“指向占位登记
    /// 位置”的回溯面，不是“内容已核实”的凭证）。
    pub fn content_read(&self) -> bool {
        matches!(
            self,
            VerificationTier::ContentMatches
                | VerificationTier::SemanticSupport
                | VerificationTier::HumanConfirmed
        )
    }
}

/// 证据台账记录（T-1 规格 struct 的 Rust 落地——字段对齐 issue #11 批次 v3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,                  // EV-xxxx（store 单序列签发）
    pub kind: EvidenceKind,          // 资质|业绩|人员|证书|产品资料|方案模块|合同
    pub title: String,               // 证据标题（如「电子与智能化工程专业承包二级资质证书」）
    pub source_file: String,         // 原文件名（TenderFile.name 引用）
    pub source_sha256: String,       // 原文件 sha256（幂等/溯源面——引擎不做文件级指纹，标书侧算）
    pub locator: EngineLocator,      // 引擎锚点（可回溯原文）
    pub fingerprint: String,         // 定位时刻内容指纹（fnv64:xxxx——判内容变更）
    pub valid_from: Option<String>,  // ISO YYYY-MM-DD（None = 不适用）
    pub valid_until: Option<String>, // ISO YYYY-MM-DD（有效期——R-03 禁绑判定面）
    pub subject_name: String,        // 主体名称（与投标人比对——不符告警）
    pub scope: String,               // 适用范围（专业/区域/金额上限）
    pub version: u32,                // 换版代数（同主体同证新版本 = 旧版 Superseded）
    pub status: EvidenceStatus,

    // ---- #117 任务 1：业绩类证据的业务面（业绩一览表数据源）。
    //      `None` = 未录入（渲染层落【待填】，绝不落 0/空白）；
    //      非业绩类证据不消费这三面（serde default + skip——旧快照零影响）。
    /// 发包人/建设单位（业绩合同甲方）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// 合同金额（万元；None = 未录入）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_wan: Option<f64>,
    /// 完成/竣工日期（ISO YYYY-MM-DD 或「2024.06」口径；None = 未录入）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_on: Option<String>,

    // ---- #147：结构化字段（search_evidence exact 通道的精确查询面）。
    //      serde default + skip_serializing_if —— 空结构不落快照，旧快照零影响。
    /// 结构化字段（合同/证书/人员三族的金额、证书号、日期等）。
    #[serde(default, skip_serializing_if = "EvidenceFields::is_empty")]
    pub fields: EvidenceFields,

    // ---- #217（#185 缺口 2）：证据正文链路（四态验证 + 正文摘录）。
    //      serde default + skip_serializing_if —— 旧快照零影响。
    /// 证据正文摘录：定位时刻从解析产物（ParsedFile.blocks）**逐字截取**的
    /// 实际内容——不是文件名/标题转写。`None` = 文件级锚未定位正文。
    /// 生成器据此引用金额/时间/范围等实料（不读正文的「佐证」是编造温床）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// 摘录定位（人话面：`《文件名》正文段 3` / `《文件名》第 2 页`——
    /// 机器面是 `locator`，本字段给生成器/导出做可读引用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt_locator: Option<String>,
    /// 内容验证五态（见 [`VerificationTier`]——服务端登记时裁决；
    /// `Some(Placeholder)` 是 #245 占位面，只能经换版重登升级）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_tier: Option<VerificationTier>,
}

impl Evidence {
    /// 是否为占位/待核验材料（#245）：`verification_tier == PLACEHOLDER`。
    ///
    /// 占位证据**可以绑定**（待补可见）、**可以进起草输入**（生成器输出
    /// 【待补】文案），但**不得计为有效证据**——矩阵派生
    /// （[`crate::matrix::derive_matrix_rows`]）、起草就绪
    /// （`is_usable_evidence`）、验证与导出门均以本判定分流。
    pub fn is_placeholder(&self) -> bool {
        self.verification_tier == Some(VerificationTier::Placeholder)
    }
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
    // ---- #117 任务 1：业绩业务面（None = 未录入；非业绩类证据不消费）
    pub client: Option<String>,
    pub amount_wan: Option<f64>,
    pub completed_on: Option<String>,
    // ---- #147：结构化字段（search_evidence exact 通道的精确查询面）
    pub fields: EvidenceFields,
    // ---- #217（#185 缺口 2）：正文摘录 + 验证态（服务端裁决后经此入账）
    pub excerpt: Option<String>,
    pub excerpt_locator: Option<String>,
    pub verification_tier: Option<VerificationTier>,
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
    for (label, d) in [
        ("valid_from", &spec.valid_from),
        ("valid_until", &spec.valid_until),
    ] {
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
    // #147：结构化字段校验（非法日期/负金额/超大年限一律拒，零入库）
    spec.fields.validate().map_err(un)?;
    // #217：四态不变量——声称「内容一致」就必须携带该内容的逐字摘录
    // （CONTENT_MATCHES 的凭证就是 excerpt；空摘录的 CONTENT_MATCHES 是
    // 无凭据的强声明，域层拒绝——调用方无从绕过服务端自己标四级态）。
    if let Some(VerificationTier::ContentMatches) = spec.verification_tier {
        if spec.excerpt.as_deref().map_or(true, |s| s.trim().is_empty()) {
            return Err(un(
                "verification_tier=CONTENT_MATCHES 必须携带非空 excerpt（正文摘录是内容验证的凭证——无摘录请降级 FILE_EXISTS）"
                    .into(),
            ));
        }
    }
    // 携带摘录但态缺省 → 不猜：摘录只在「声明了验证态」时有意义
    // （CONTENT_MATCHES 的凭证 / PLACEHOLDER 的占位回溯面），其余态
    // 带 excerpt 属于口径漂移（文件级锚哪来的正文？），显式拒。
    if spec.excerpt.is_some()
        && !matches!(
            spec.verification_tier,
            Some(VerificationTier::ContentMatches) | Some(VerificationTier::Placeholder)
        )
    {
        return Err(un(
            "携带 excerpt 必须同时声明 verification_tier（CONTENT_MATCHES——摘录与验证态不可拆；#245：PLACEHOLDER 占位登记也可带摘录回溯占位位置）"
                .into(),
        ));
    }
    // #245：占位声明纪律——PLACEHOLDER 不得携带 locator 指纹对账外的任何
    // 「已核验」暗示。域层不重复 #212 的来源对账（服务端职责），但占位态
    // 必须可持久化、可审计（excerpt/excerpt_locator 照记，供回溯占位登记
    // 指向了什么）。
    Ok(())
}

/// 定位器结构校验（坐标域 + 区域端点序——构造面防线的最后一环）。
pub fn validate_locator(loc: &EngineLocator) -> Result<(), String> {
    match loc {
        EngineLocator::XlsxCell { cell, .. } => CellCoordLike::validate(cell),
        EngineLocator::XlsxRegion { start, end, .. } => CellCoordLike::validate(start)
            .and_then(|_| CellCoordLike::validate(end))
            .and_then(|_| {
                crate::locator::validate_region(start, end)
                    .map_err(|e| format!("区域端点逆序: {e:?}"))
            }),
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
    (
        ev.kind,
        ev.title.clone(),
        ev.subject_name.clone(),
        ev.scope.clone(),
    )
}

impl DomainStore {
    /// 台账只读面。
    pub fn evidences(&self) -> &[Evidence] {
        &self.evidences
    }

    pub fn evidence(&self, id: &str) -> Result<&Evidence, ToolError> {
        self.evidences
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| ToolError::ENotFound {
                what: format!("evidence {id} 不存在"),
            })
    }

    pub fn evidence_bindings(&self) -> &[EvidenceBinding] {
        &self.evidence_bindings
    }

    /// 登记证据（幂等 + 换版）。
    ///
    /// - 语义四元组 (kind, title, subject, scope) 相同且指纹+双日期+验证态全同
    ///   → 重放 no-op（deduplicated: true，content_version 不动）；
    /// - 语义位已占用但指纹/日期/验证态有变 → **换版**：旧记录 Superseded（经
    ///   单向降级迁移——Expired→Superseded 亦合法），新记录 version = 旧+1，
    ///   且**旧记录的既有绑定迁移到新记录**（语义位不变 → 响应面跟随当前版，
    ///   #245：占位→真实的升级路径由此承载，旧绑定不残留）；
    /// - 新语义位 → 直接入账，version = 1。
    ///
    /// #245 验证态入幂等键的理由：同指纹不同 tier（如 PLACEHOLDER →
    /// CONTENT_MATCHES）语义不同——若按重放 no-op 返回旧记录 id，调用方
    /// 会拿到「以为转正了」的假成功（实际还是占位）；按换版处理则
    /// 留审计痕（旧记录 Superseded 可查）且绑定迁移到新记录。
    ///
    /// 出生态一律 Active——时间面翻转（Expired/Expiring）由 [`Self::apply_validity`]
    /// 与 [`Self::bind_evidence`] 携带的开标日裁决（登记时不知道评标基准日）。
    pub fn register_evidence(&mut self, spec: EvidenceSpec) -> Result<(String, bool), ToolError> {
        self.project()?; // 台账挂在项目下（M1 单项目存储）
        validate_spec(&spec)?;
        let key = (
            spec.kind,
            spec.title.clone(),
            spec.subject_name.clone(),
            spec.scope.clone(),
        );
        // 语义位查重：同 key 的最新非 Superseded 记录（rfind——从尾向头找最新版）
        if let Some(existing) = self
            .evidences
            .iter()
            .rfind(|e| semantic_key(e) == key && e.status != EvidenceStatus::Superseded)
        {
            let same_payload = existing.fingerprint == spec.fingerprint
                && existing.valid_from == spec.valid_from
                && existing.valid_until == spec.valid_until
                && existing.verification_tier == spec.verification_tier;
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
                client: spec.client,
                amount_wan: spec.amount_wan,
                completed_on: spec.completed_on,
                fields: spec.fields,
                excerpt: spec.excerpt,
                excerpt_locator: spec.excerpt_locator,
                verification_tier: spec.verification_tier,
                version: old_version + 1,
                status: EvidenceStatus::Active,
            });
            // #245 换版绑定迁移：旧记录的绑定改指新记录（去重——新位已有
            // 同条款绑定时只删旧，不双绑）。绑定的语义是「这个语义位（资质）
            // 响应了这个条款」——版本迭代后响应面应跟随当前版本，否则旧
            // 绑定会残留成「Superseded 仍绑定」的规则 8 BLOCKER，把合法的
            // 换版/占位升级流程卡死在导出门外。
            //
            // 两步式（retain_mut 闭包内改 id 再返回 false 会把迁移项误删）：
            // 1) 收集旧位绑定对应的条款（排除新位已有的）；2) 重建绑定表。
            let migrate_req_ids: Vec<String> = self
                .evidence_bindings
                .iter()
                .filter(|b| b.evidence_id == old_id)
                .map(|b| b.requirement_id.clone())
                .filter(|rid| {
                    !self
                        .evidence_bindings
                        .iter()
                        .any(|x| x.evidence_id == id && x.requirement_id == *rid)
                })
                .collect();
            self.evidence_bindings.retain(|b| b.evidence_id != old_id);
            for rid in migrate_req_ids {
                self.evidence_bindings.push(EvidenceBinding {
                    evidence_id: id.clone(),
                    requirement_id: rid,
                });
            }
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
            client: spec.client,
            amount_wan: spec.amount_wan,
            completed_on: spec.completed_on,
            fields: spec.fields,
            excerpt: spec.excerpt,
            excerpt_locator: spec.excerpt_locator,
            verification_tier: spec.verification_tier,
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
                    flipped.push(format!(
                        "{}: {} -> {}",
                        ev.id,
                        before.as_str(),
                        ev.status.as_str()
                    ));
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
    pub fn bind_evidence(
        &mut self,
        ev_id: &str,
        requirement_id: &str,
        bid_open_date: &str,
    ) -> Result<bool, ToolError> {
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
        if let Some(target) = self
            .evidences
            .iter()
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
        if self
            .evidence_bindings
            .iter()
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
        self.evidences
            .iter()
            .filter(|e| e.status != EvidenceStatus::Superseded)
            .filter_map(|e| subject_warning(e, bidder_name))
            .collect()
    }

    /// 绑定可用性快照（矩阵构建输入——T-2 将消费）：可绑证据 id 清单。
    pub fn bindable_evidence_ids(&self, bid_open_date: &str) -> Vec<String> {
        self.evidences
            .iter()
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
        s.create_project("测试项目", "ZFCG-2026-GK-0311", "明标", "ext-key-ev")
            .unwrap();
        s.upsert_file(
            "A-招标文件.docx",
            "招标文件",
            "aa",
            31,
            12,
            120,
            0,
            crate::ParseStatus::Ok,
            "",
        )
        .unwrap();
        let reqs = vec![crate::Requirement {
            id: "REQ-0002".into(),
            category: "资格条件".into(),
            clause_no: Some("3.2.2".into()),
            anchors: vec![crate::Anchor {
                file: "A".into(),
                kind: "para".into(),
                locator: "第三章".into(),
                excerpt: "具备安全生产许可证".into(),
                    block_idx: None,
                    page: None,
                    line: None
            }],
            params: std::collections::HashMap::new(),
            source_file: "A".into(),
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
            locator: EngineLocator::PdfPageText {
                page: 1,
                text_fingerprint: "fnv64:13".into(),
            },
            fingerprint: "fnv64:13".into(),
            valid_from: Some("2022-08-01".into()),
            valid_until: Some("2025-08-01".into()),
            subject_name: "智慧园区科技有限公司".into(),
            scope: "全国".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: EvidenceFields::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        }
    }

    /// GT-02 语义件：资质证书，valid_until=2026-11-25（开标日附近）。
    fn gt02_spec() -> EvidenceSpec {
        EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "电子与智能化工程专业承包二级资质证书".into(),
            source_file: "EV-02-电子与智能化工程专业承包二级资质证书.pdf".into(),
            source_sha256: "cafebabe02".into(),
            locator: EngineLocator::PdfPageText {
                page: 1,
                text_fingerprint: "fnv64:02".into(),
            },
            fingerprint: "fnv64:02".into(),
            valid_from: Some("2021-11-25".into()),
            valid_until: Some("2026-11-25".into()),
            subject_name: "智慧园区科技有限公司".into(),
            scope: "全国".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: EvidenceFields::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
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
        assert!(
            s.degrade_to(Superseded),
            "Expired→Superseded 放行（过期件换版淘汰）"
        );
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
        assert!(
            a2.degrade_to(Expired),
            "Active→Expired 跨级放行（开标日提前）"
        );
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
        assert_eq!(
            flipped,
            vec!["EV-0003: ACTIVE -> EXPIRING".to_string()],
            "GT-02 @2026-11-15 → Expiring（10 天内到期，可绑 = eligible）"
        );
        assert!(
            s.evidence(&ev_id).unwrap().status.can_bind(),
            "Expiring 可绑"
        );
        // 幂等：同日再裁决不动
        assert!(
            s.apply_validity("2026-11-15").unwrap().is_empty(),
            "同态迁移被拒（幂等）"
        );
        // @2026-11-27：valid_until(11-25) < 开标日 → Expired
        let flipped2 = s.apply_validity("2026-11-27").unwrap();
        assert_eq!(
            flipped2,
            vec!["EV-0003: EXPIRING -> EXPIRED".to_string()],
            "GT-02 @2026-11-27 → EXPIRED"
        );
        assert!(!s.evidence(&ev_id).unwrap().status.can_bind(), "过期禁绑");
    }

    /// GT-13 过期禁绑（`forbidden_bind: true`）：valid_until=2025-08-01 对 2026 开标
    /// → EXPIRED + 绑定 → E_UNPROCESSABLE（R-03）。
    #[test]
    fn gt13_expired_evidence_forbidden_to_bind() {
        let mut s = store_with_reqs();
        let (ev_id, _) = s.register_evidence(gt13_spec()).unwrap();
        // 直接绑定（携带开标日做即时裁决——不经 apply_validity 前置）
        let err = s
            .bind_evidence(&ev_id, "REQ-0002", "2026-11-15")
            .unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "GT-13 过期禁绑 → R-03");
        assert!(format!("{err:?}").contains("禁止绑定"));
        assert_eq!(
            s.evidence(&ev_id).unwrap().status,
            EvidenceStatus::Expired,
            "绑定尝试本身已触发即时翻转（开标日口径）"
        );
        assert!(s.evidence_bindings().is_empty(), "拒绝路径零绑定残留");
    }

    /// 绑定正常路径 + 幂等重放 + REQ 不存在拒绝。
    #[test]
    fn bind_active_evidence_idempotent_and_validates_requirement() {
        let mut s = store_with_reqs();
        let (ev_id, _) = s.register_evidence(gt02_spec()).unwrap();
        // REQ 不存在 → E_NOTFOUND
        let err = s
            .bind_evidence(&ev_id, "REQ-9999", "2026-11-15")
            .unwrap_err();
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
            locator: EngineLocator::PdfPageText {
                page: 1,
                text_fingerprint: "fnv64:01".into(),
            },
            fingerprint: "fnv64:01".into(),
            valid_from: None,
            valid_until: None,
            subject_name: "智慧园区科技有限公司".into(),
            scope: "全国".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: EvidenceFields::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        };
        let (id, _) = s.register_evidence(spec).unwrap();
        assert!(
            s.apply_validity("2099-01-01").unwrap().is_empty(),
            "无 valid_until → 任何评标基准日都不翻转（GT-01 eligible 语义）"
        );
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
        assert!(
            warns[0].contains("EVIDENCE_SUBJECT_MISMATCH"),
            "告警码可被调用方分支"
        );
        assert!(warns[0].contains("集团"), "主体名双呈现（人工裁决素材）");
        // 一致时零告警
        assert!(s
            .subject_warnings("智慧园区科技集团股份有限公司")
            .is_empty());
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
        let mut sp1 = gt13_spec();
        sp1.valid_until = Some("2026-13-40".into());
        assert!(s.register_evidence(sp1).is_err(), "非法 ISO 日期拒");
        let mut sp2 = gt13_spec();
        sp2.valid_from = Some("2026-01-01".into());
        sp2.valid_until = Some("2025-01-01".into());
        assert!(s.register_evidence(sp2).is_err(), "from > until 拒");
        let mut sp3 = gt13_spec();
        sp3.title = "  ".into();
        assert!(s.register_evidence(sp3).is_err(), "空标题拒");
        let mut sp4 = gt13_spec();
        sp4.subject_name = String::new();
        assert!(s.register_evidence(sp4).is_err(), "空主体拒");
        let mut sp5 = gt13_spec();
        sp5.locator = EngineLocator::XlsxRegion {
            sheet: "报价表".into(),
            start: CellCoord("B2".into()),
            end: CellCoord("A1".into()), // 逆序区域
            region_fingerprint: "fnv64:xx".into(),
        };
        assert!(s.register_evidence(sp5).is_err(), "区域端点逆序拒");
        // 台账前置：无项目 → E_NOTFOUND
        let mut empty = DomainStore::new();
        assert!(matches!(
            empty.register_evidence(gt13_spec()),
            Err(ToolError::ENotFound { .. })
        ));
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

    // ============================ #147 结构化字段（EvidenceFields） ============================

    /// 满配样例（三族字段全填——serde 往返的对照基准）。
    fn full_fields() -> EvidenceFields {
        EvidenceFields {
            project_name: Some("智慧园区一期智能化工程".into()),
            client: Some("智慧园区管委会".into()),
            amount_wan: Some(1280.5),
            signed_at: Some("2022-03-15".into()),
            finished_at: Some("2023-01-20".into()),
            cert_name: Some("电子与智能化工程专业承包二级资质证书".into()),
            cert_no: Some("D234000123".into()),
            issuer: Some("省住房和城乡建设厅".into()),
            person_name: Some("张三".into()),
            person_title: Some("高级工程师".into()),
            years: Some(12),
        }
    }

    /// serde 往返：满配字段 serialize → deserialize 逐字段相等（不丢字段、不串位）。
    #[test]
    fn evidence_fields_serde_roundtrip_full() {
        let f = full_fields();
        let js = serde_json::to_string(&f).unwrap();
        let back: EvidenceFields = serde_json::from_str(&js).unwrap();
        assert_eq!(back, f, "满配字段 serde 往返恒等");
        assert!(!back.is_empty());
        assert_eq!(back.amount_wan, Some(1280.5));
        assert_eq!(back.years, Some(12));
    }

    /// 向后兼容：空 JSON 对象 → 全 None（serde default）；未知键不炸（forward-compat）。
    #[test]
    fn evidence_fields_serde_defaults_and_unknown_keys() {
        let empty: EvidenceFields = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, EvidenceFields::default(), "空对象 → 全 None");
        assert!(empty.is_empty(), "空结构 is_empty");
        assert!(empty.validate().is_ok(), "全 None 恒合法");
        let partial: EvidenceFields =
            serde_json::from_str(r#"{"cert_no":"X-1","future_field":"ignored"}"#).unwrap();
        assert_eq!(partial.cert_no.as_deref(), Some("X-1"));
        assert_eq!(partial.cert_name, None, "未给字段保持 None");
        assert!(!partial.is_empty());
    }

    /// 老快照兼容：Evidence 的 JSON 无 fields 键 → 反序列化为空结构；
    /// 空结构序列化时**不落** fields 键（skip_serializing_if——快照零膨胀）。
    #[test]
    fn evidence_fields_absent_in_legacy_snapshot() {
        let mut s = store_with_reqs();
        let (id, _) = s.register_evidence(gt02_spec()).unwrap();
        let ev = s.evidence(&id).unwrap().clone();
        let js = serde_json::to_value(&ev).unwrap();
        assert!(
            js.get("fields").is_none(),
            "空结构化字段不落快照（旧格式兼容）"
        );
        // 去掉 fields 键后仍可反序列化（旧快照 → 空结构）
        let back: Evidence = serde_json::from_value(js).unwrap();
        assert!(back.fields.is_empty(), "旧快照反序列化 → fields 为默认空");
        assert_eq!(back, ev, "除 fields 外全等");
    }

    /// validate 拒绝面：非法日期 / 负金额 / NaN / ∞ / 超大年限 → Err（文案指明字段）。
    #[test]
    fn evidence_fields_validate_rejects_malformed() {
        // 日期形态
        for bad in ["2024-13-01", "2024.06.15", "2024-06", "", "20240615"] {
            let mut f = full_fields();
            f.signed_at = Some(bad.into());
            let e = f.validate().unwrap_err();
            assert!(e.contains("signed_at"), "signed_at 非法日期拒: {bad:?}（{e}）");
            let mut f2 = full_fields();
            f2.finished_at = Some(bad.into());
            assert!(
                f2.validate().unwrap_err().contains("finished_at"),
                "finished_at 非法日期拒: {bad:?}"
            );
        }
        // 金额
        let mut neg = full_fields();
        neg.amount_wan = Some(-0.01);
        assert!(neg.validate().unwrap_err().contains("amount_wan"), "负金额拒");
        let mut nan = full_fields();
        nan.amount_wan = Some(f64::NAN);
        assert!(nan.validate().unwrap_err().contains("amount_wan"), "NaN 拒");
        let mut inf = full_fields();
        inf.amount_wan = Some(f64::INFINITY);
        assert!(inf.validate().unwrap_err().contains("amount_wan"), "∞ 拒");
        // 年限
        let mut y = full_fields();
        y.years = Some(MAX_YEARS + 1);
        assert!(y.validate().unwrap_err().contains("years"), "年限 > 100 拒");
        // 边界放行
        let mut ok = full_fields();
        ok.amount_wan = Some(0.0);
        ok.years = Some(MAX_YEARS);
        assert!(ok.validate().is_ok(), "0 金额 / 100 年 = 合法边界");
    }

    /// 登记面：结构化字段随记录落账（换版亦保留新字段）；非法字段 → E_UNPROCESSABLE 零入库。
    #[test]
    fn register_evidence_persists_and_validates_fields() {
        let mut s = store_with_reqs();
        let mut spec = gt02_spec();
        spec.fields = full_fields();
        let (id, _) = s.register_evidence(spec).unwrap();
        assert_eq!(
            s.evidence(&id).unwrap().fields,
            full_fields(),
            "结构化字段随记录落账"
        );
        // 换版：新字段随新版落账（旧版 Superseded 保留旧字段作审计）
        let mut renewed = gt02_spec();
        let mut f2 = full_fields();
        f2.cert_no = Some("D234000124".into());
        renewed.fields = f2.clone();
        renewed.fingerprint = "fnv64:02v2".into();
        let (id2, _) = s.register_evidence(renewed).unwrap();
        assert_eq!(s.evidence(&id2).unwrap().fields.cert_no, f2.cert_no);
        assert_eq!(
            s.evidence(&id).unwrap().fields.cert_no.as_deref(),
            Some("D234000123"),
            "旧版字段不被覆盖（审计保留）"
        );
        // 非法字段 → E_UNPROCESSABLE 零入库
        let n_before = s.evidences().len();
        let mut bad = gt13_spec();
        bad.fields.signed_at = Some("2024-02-31T00:00:00".into());
        let err = s.register_evidence(bad).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "非法日期字段拒登记");
        assert_eq!(s.evidences().len(), n_before, "拒绝路径零入库");
        let mut bad2 = gt13_spec();
        bad2.fields.amount_wan = Some(-5.0);
        assert_eq!(
            s.register_evidence(bad2).unwrap_err().code(),
            "E_UNPROCESSABLE",
            "负金额字段拒登记"
        );
    }

    // ============================ #217（#185 缺口 2）：四态验证 + 正文摘录 ============================

    /// 四态不变量①：CONTENT_MATCHES 必须携带非空 excerpt——空摘录的
    /// 「内容一致」是无凭据的强声明，域层拒绝（调用方无从绕过服务端自标态）。
    #[test]
    fn content_matches_tier_requires_excerpt() {
        let mut s = store_with_reqs();
        let mut sp = gt13_spec();
        sp.verification_tier = Some(VerificationTier::ContentMatches);
        sp.excerpt = None;
        let err = s.register_evidence(sp).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "CONTENT_MATCHES 无摘录拒");
        assert!(
            format!("{err:?}").contains("excerpt"),
            "错误文案指向 excerpt 面: {err:?}"
        );
        // 空白摘录同拒（trim 口径——纯空白不是凭证）
        let mut sp2 = gt13_spec();
        sp2.verification_tier = Some(VerificationTier::ContentMatches);
        sp2.excerpt = Some("   ".into());
        assert_eq!(
            s.register_evidence(sp2).unwrap_err().code(),
            "E_UNPROCESSABLE",
            "空白摘录同拒"
        );
        assert!(s.evidences().is_empty(), "拒绝路径零入库");
    }

    /// 四态不变量②：excerpt 与 verification_tier 不可拆——带摘录不声明态
    /// 属口径漂移（文件级锚哪来的正文？），显式拒。
    #[test]
    fn excerpt_without_tier_rejected() {
        let mut s = store_with_reqs();
        let mut sp = gt02_spec();
        sp.excerpt = Some("承包范围：电子与智能化工程施工总承包".into());
        sp.excerpt_locator = Some("《EV-02-资质证书.pdf》第 1 页".into());
        sp.verification_tier = None;
        let err = s.register_evidence(sp).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "摘录与态不可拆");
        assert!(
            format!("{err:?}").contains("verification_tier"),
            "错误文案指路声明态: {err:?}"
        );
    }

    /// 正向：CONTENT_MATCHES + 摘录入账透传 + serde 往返保留（旧快照兼容面：
    /// 三新字段 default+skip——旧 JSON 反序列化为 None）。
    #[test]
    fn tier_and_excerpt_passthrough_with_serde_roundtrip() {
        let mut s = store_with_reqs();
        let mut sp = gt02_spec();
        sp.verification_tier = Some(VerificationTier::ContentMatches);
        sp.excerpt = Some("资质等级：二级；承包范围：电子与智能化工程".into());
        sp.excerpt_locator = Some("《EV-02-电子与智能化工程专业承包二级资质证书.pdf》第 1 页".into());
        let (id, _) = s.register_evidence(sp).unwrap();
        let ev = s.evidence(&id).unwrap();
        assert_eq!(
            ev.verification_tier,
            Some(VerificationTier::ContentMatches),
            "态透传入账"
        );
        assert!(
            ev.excerpt.as_deref().unwrap().contains("资质等级"),
            "摘录透传"
        );
        assert!(
            ev.excerpt_locator.as_deref().unwrap().contains("第 1 页"),
            "摘录定位透传"
        );
        // serde 往返（store 快照面——json 序列化再反序列化，新字段保留）
        let json_str = serde_json::to_string(ev).unwrap();
        let back: Evidence = serde_json::from_str(&json_str).unwrap();
        assert_eq!(back.verification_tier, Some(VerificationTier::ContentMatches));
        assert_eq!(back.excerpt, ev.excerpt);
        // 旧快照（无三新字段的 JSON）→ 反序列化为 None（default 兜底，零破坏）
        let legacy = serde_json::json!({
            "id": "EV-0001", "kind": "certificate", "title": "旧证据",
            "source_file": "a.pdf", "source_sha256": "aa", "locator": {
                "kind": "pdf_page_text", "page": 1, "text_fingerprint": "fnv64:x" },
            "fingerprint": "fnv64:x", "valid_from": null, "valid_until": null,
            "subject_name": "甲", "scope": "", "version": 1, "status": "ACTIVE"
        });
        let old: Evidence = serde_json::from_value(legacy).unwrap();
        assert_eq!(old.verification_tier, None, "旧快照无态（登记于 #217 前）");
        assert_eq!(old.excerpt, None, "旧快照无摘录");
    }

    /// 四态门面：FILE_EXISTS（文件级锚诚实态）+ content_read 判定 +
    /// 预留态的序列化面（SEMANTIC_SUPPORT / HUMAN_CONFIRMED 枚举先立面）。
    #[test]
    fn verification_tier_str_and_content_read_faces() {
        assert_eq!(VerificationTier::FileExists.as_str(), "FILE_EXISTS");
        assert_eq!(VerificationTier::ContentMatches.as_str(), "CONTENT_MATCHES");
        assert_eq!(VerificationTier::SemanticSupport.as_str(), "SEMANTIC_SUPPORT");
        assert_eq!(VerificationTier::HumanConfirmed.as_str(), "HUMAN_CONFIRMED");
        assert!(!VerificationTier::FileExists.content_read(), "文件级锚正文未读");
        assert!(VerificationTier::ContentMatches.content_read());
        assert!(VerificationTier::SemanticSupport.content_read(), "语义互证必先有摘录");
        assert!(VerificationTier::HumanConfirmed.content_read());
        // serde SCREAMING_SNAKE_CASE（MCP 出参口径直接对齐）
        let j = serde_json::to_string(&VerificationTier::HumanConfirmed).unwrap();
        assert_eq!(j, "\"HUMAN_CONFIRMED\"");
    }

    // ============================ #245：占位/待核验证据（PLACEHOLDER 态） ============================

    /// 占位态入账：is_placeholder 判定 + 可绑（Active 状态面不受占位影响——
    /// 绑定是「待补可见」的载体，占位绑定不清阻塞由矩阵派生负责）+ 摘录
    /// 可随占位登记入账（回溯占位指向）+ serde 往返保留。
    #[test]
    fn placeholder_tier_persists_binds_and_roundtrips() {
        let mut s = store_with_reqs();
        let mut sp = gt02_spec();
        sp.verification_tier = Some(VerificationTier::Placeholder);
        sp.excerpt = Some("占位登记：待投标人提供真实资质证书后重新登记换版".into());
        sp.excerpt_locator = Some("《招标文件》第三章 资格要求".into());
        let (id, dedup) = s.register_evidence(sp).unwrap();
        assert!(!dedup, "新语义位首登");
        let ev = s.evidence(&id).unwrap().clone();
        assert!(ev.is_placeholder(), "占位判定命中");
        assert_eq!(ev.status, EvidenceStatus::Active, "状态面仍是 Active（有效期维度与占位维度正交）");
        assert!(!VerificationTier::Placeholder.content_read(), "占位态不算已读正文");
        // 占位证据可绑（绑定让待补可见——不拒绑，但矩阵派生不清阻塞）
        s.bind_evidence(&id, "REQ-0002", "2026-11-15").unwrap();
        assert_eq!(s.evidence_bindings().len(), 1, "占位绑定成功（待补载体）");
        // serde 往返保留占位态（持久化面）
        let back: Evidence = serde_json::from_str(&serde_json::to_string(&ev).unwrap()).unwrap();
        assert!(back.is_placeholder(), "快照往返占位态保留");
        // 旧快照（verification_tier 缺省）不是占位——兼容面零位移
        assert!(!serde_json::from_value::<Evidence>(serde_json::json!({
            "id": "EV-0001", "kind": "qualification", "title": "旧证据",
            "source_file": "a.pdf", "source_sha256": "aa", "locator": {
                "kind": "pdf_page_text", "page": 1, "text_fingerprint": "fnv64:x" },
            "fingerprint": "fnv64:x", "valid_from": null, "valid_until": null,
            "subject_name": "甲", "scope": "", "version": 1, "status": "ACTIVE"
        })).unwrap().is_placeholder(), "旧快照非占位");
    }

    /// 同指纹不同验证态 → 换版（不幂等重放）：PLACEHOLDER → CONTENT_MATCHES
    /// 的「转正」必须签发新记录 + 旧记录 Superseded 留审计痕——若按重放
    /// no-op 返回旧 id，调用方会拿到「以为转正了」的假成功。
    #[test]
    fn same_fingerprint_different_tier_bumps_version_not_replay() {
        let mut s = store_with_reqs();
        let mut ph = gt02_spec();
        ph.verification_tier = Some(VerificationTier::Placeholder);
        let (id_ph, _) = s.register_evidence(ph).unwrap();
        // 同指纹、同日期、不同 tier → 换版
        let mut real = gt02_spec(); // 指纹同 fnv64:02
        real.verification_tier = Some(VerificationTier::ContentMatches);
        real.excerpt = Some("资质等级：二级；承包范围：电子与智能化工程".into());
        real.excerpt_locator = Some("《EV-02》第 1 页".into());
        let (id_real, dedup) = s.register_evidence(real).unwrap();
        assert!(!dedup, "tier 不同 = 语义变化 → 换版（非重放）");
        assert_ne!(id_ph, id_real, "签发新 EV");
        assert_eq!(s.evidence(&id_ph).unwrap().status, EvidenceStatus::Superseded, "旧占位记录留痕");
        assert_eq!(s.evidence(&id_real).unwrap().version, 2, "版本递增");
        assert!(!s.evidence(&id_real).unwrap().is_placeholder(), "新记录非占位");
    }

    /// 换版绑定迁移：占位记录的既有绑定在换版后改指新记录（语义位的响应面
    /// 跟随当前版本）——占位→真实升级后矩阵行立即按真实证据派生，且旧绑定
    /// 不残留成「Superseded 仍绑定」。
    #[test]
    fn version_bump_migrates_bindings_to_new_record() {
        let mut s = store_with_reqs();
        let mut ph = gt02_spec();
        ph.verification_tier = Some(VerificationTier::Placeholder);
        let (id_ph, _) = s.register_evidence(ph).unwrap();
        s.bind_evidence(&id_ph, "REQ-0002", "2026-11-15").unwrap();
        assert!(s.evidence_bindings().iter().any(|b| b.evidence_id == id_ph));
        // 换版（真实证据：新指纹）
        let mut real = gt02_spec();
        real.fingerprint = "fnv64:02-real".into();
        real.verification_tier = Some(VerificationTier::ContentMatches);
        real.excerpt = Some("资质等级：二级".into());
        real.excerpt_locator = Some("《EV-02》第 1 页".into());
        let (id_real, _) = s.register_evidence(real).unwrap();
        assert!(
            s.evidence_bindings().iter().any(|b| b.evidence_id == id_real && b.requirement_id == "REQ-0002"),
            "旧占位绑定迁移到新记录"
        );
        assert!(
            !s.evidence_bindings().iter().any(|b| b.evidence_id == id_ph),
            "旧记录绑定不残留（Superseded 仍绑定会卡规则 8）"
        );
        assert_eq!(s.evidence_bindings().len(), 1, "迁移去重——无双绑");
    }

    /// 幂等重放不变（tier 相同 + 指纹相同 → no-op 返回原 id）——same_payload
    /// 加入 tier 比较后，同态重放路径零回归。
    #[test]
    fn same_tier_same_fingerprint_still_replays() {
        let mut s = store_with_reqs();
        let mut ph = gt02_spec();
        ph.verification_tier = Some(VerificationTier::Placeholder);
        let (id1, _) = s.register_evidence(ph.clone()).unwrap();
        let (id2, dedup) = s.register_evidence(ph).unwrap();
        assert!(dedup, "同态同指纹 → 幂等重放");
        assert_eq!(id1, id2, "原 id 直返");
    }
}