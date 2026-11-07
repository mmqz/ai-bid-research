//! principal.rs —— #182 TrustedPrincipal 可信身份（P0-2：`human:` 前缀可被
//! Agent 伪造——人工审核身份此前无运行时校验）。
//!
//! 缺口：复审人只查 `human:` 字符串前缀（review.rs 旧口径），Agent 自称
//! `human:admin` 即可过门——「AI 不得自审自过」是约定不是保证。本模块把
//! 人工身份升级为 **HMAC-SHA256 签发的 receipt**（运行时可验证）：
//!
//! - **宿主签发面**（`issue_approval_receipt`）：真人审核员在宿主进程确认后，
//!   由宿主用服务态密钥签发 receipt——签发面不进 MCP 工具表，Agent 拿不到；
//! - **服务层验签面**（`verify_approval_receipt`）：过审工具（record_review_
//!   gate / record_addendum_waiver）收 receipt 而非裸身份串，验签通过才得
//!   [`TrustedPrincipal`]——这是 MCP 面的唯一入口（单一真源，壳层不重复实现）；
//! - **域层纵深**（`DomainStore::record_review` / `record_addendum_waiver`）：
//!   只认 `actor_type == Human`——即使壳层漏验，域层也不放 Agent 过门。
//!
//! receipt 线格式（v2，七段，点分隔——#323/R-06 内容绑定）：
//! `v2.<actor_id>.<actor_type>.<scope>.<issued_at_unix_secs>.<sections_digest_hex>.<hex_hmac_sha256>`
//!
//! - HMAC 消息 = `v2|actor_id|actor_type|scope|issued_at|sections_digest`（竖线
//!   分隔——与线格式分隔符错开，段内含 `:`（scope 如
//!   `review_gate:pre_export_confirm`）不炸）；
//! - **scope 绑定**：receipt 只对签发时声明的用途有效（`review_gate:<gate>`
//!   / `addendum_waiver:<file_id>`）——A 门的 receipt 过不了 B 门、TF-0003 的
//!   receipt 免不了 TF-0004（重放面收敛到具体裁决位）；
//! - **内容绑定（#323/R-06）**：第六段 = 签发时的章节树摘要
//!   （`sections_digest`，sha256 十六进制 64 位——与 #180 导出裁决同一真源
//!   口径：绑摘要不绑全局 content_version，后者被导入无关件/记审自身一律
//!   bump 必误报）。验签时与当前摘要比对——不一致 → `ContentDrifted`
//!   （「批的是旧版内容」的审计失真面收口：宿主签发后章节被改，旧
//!   receipt 拒绝过门）；摘要进 HMAC 消息——篡改摘要段 = BadSignature；
//!   无关写（导入/记审）不动章节树 → 摘要不变 → 不误报；
//! - **时效窗**：`issued_at ∈ [now-86400, now+60]`——24h 有效期防陈旧重放，
//!   +60s 容差吸收宿主/服务侧时钟微漂（越窗 → Expired / FutureDated）；
//! - **常量时间比对**：HMAC 比对不做短路比较（见 [`ct_eq`]）——时序侧信道
//!   不泄漏签名前缀信息；
//! - **v1 → v2 迁移（fail-closed）**：旧六段 v1 receipt 拒收（`LegacyV1`）
//!   ——receipt 是 24h 短命凭证无持久存量，破坏窗 ≤1 个 TTL；接受 v1 =
//!   R-06 的洞在过渡期继续开着（裁定依据见 #323 PR，请 Lead 确认）。
//!
//! 纯函数纪律：本模块零 IO、零时钟（`now` 由调用方注入——服务层取系统
//! 时钟，测试取定值），与 crate 纪律一致。签发/验签的密钥管理在 bid-tools
//! 服务层（`approval_key`——持久化工作区落 `<dir>/.mang/approval.key`，
//! 纯内存态内存生成不落盘）。

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;

/// receipt 线格式版本（v2——#323 内容绑定：第六位为 sections_digest 段；
/// 格式演进时升版本号，验签面只认当版）。
pub const RECEIPT_VERSION: &str = "v2";

/// 旧线格式版本（v1 六段无内容绑定——#323 起 fail-closed 拒收，
/// `LegacyV1` 分类；重新签发 v2 即可，无存量迁移面）。
pub const LEGACY_RECEIPT_VERSION: &str = "v1";

/// 验签通过的 TrustedPrincipal 携带的认证方式标记（审计面回显——
/// v2 起随线格式版本走）。
pub const AUTH_METHOD_HMAC_RECEIPT: &str = "hmac_receipt_v2";

/// receipt 有效期（秒）——24h：人工确认的时效窗，防陈旧重放。
pub const RECEIPT_TTL_SECS: u64 = 86_400;

/// 未来时间容差（秒）——宿主签发与服务验签的时钟微漂吸收面。
pub const RECEIPT_FUTURE_TOLERANCE_SECS: u64 = 60;

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------- 类型面
/// 操作者类别（#182 三态——过审面只认 [`ActorType::Human`]）。
///
/// Agent/System 的 receipt 可以被签发（宿主侧的机器操作面也有留痕需求），
/// 但**不用于过审面**——验签面见 [`ReceiptError::NotHuman`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActorType {
    Human,
    Agent,
    System,
}

impl ActorType {
    /// 线格式（receipt 段 / 回显面的引用口径——小写三态）。
    pub fn as_str(&self) -> &'static str {
        match self {
            ActorType::Human => "human",
            ActorType::Agent => "agent",
            ActorType::System => "system",
        }
    }

    /// 解析（严格小写——receipt 段是字节精确面，不接受大小写变体）。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "human" => Some(ActorType::Human),
            "agent" => Some(ActorType::Agent),
            "system" => Some(ActorType::System),
            _ => None,
        }
    }
}

/// 验签后的可信身份（#182——人工审核的身份载体）。
///
/// 生产面的实例只来自两条路：
/// 1. [`verify_approval_receipt`] 验签通过（服务层——MCP 面的唯一入口）；
/// 2. 宿主签发面直接构造（真人刚确认、receipt 即将发出的时点）。
///
/// 域层（record_review / record_addendum_waiver）只校验身份/签域匹配
/// （#277 方案 A 的 `(actor_type, scope_is_agent)` 二元组——纵深防御）；
/// 真正的身份保证在服务层 HMAC 验签——域层零密钥零时钟，不重复实现验签
/// （单一真源）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustedPrincipal {
    /// 操作者 id（如 "admin"、"import-lead"——宿主侧的身份命名；agent 面
    /// 如 "agent-01"——调用方自述，无全局唯一性保证，审计面只保证
    /// actor_type 真实——#277 agent 过门不冒充 Human 的结构面）。
    pub actor_id: String,
    /// 操作者类别（human 过审面 + #277 agent 声明过门面）。
    pub actor_type: ActorType,
    /// 认证方式（"hmac_receipt_v1"——审计面回显）。
    pub authentication_method: String,
    /// 完整 receipt 串（审计留痕——回溯时可复验）。
    pub approval_receipt: String,
    /// #277 方案 A：验签时所用签域族——`false` = 人类签域
    /// （`review_gate:{gate}` / `addendum_waiver:{file_id}`，
    /// [`verify_approval_receipt`] 验出），`true` = agent 签域
    /// （`…:agent` 后缀，[`verify_agent_receipt`] 验出）。
    /// **由验签函数按所验 scope 派生，不是调用方可声明面**——身份不冒充
    /// 的结构保证：域层 match `(actor_type, scope_is_agent)`，混用签域拒绝。
    /// `#[serde(default)]` 加性迁移（#147 先例）：旧快照验签都是人类签域
    /// → false，行为不变（验收 8）。
    #[serde(default)]
    pub scope_is_agent: bool,
}

impl TrustedPrincipal {
    /// Human 便捷构造——**域层单测与已验签路径用，不自带身份保证**：
    /// 它不经验签（生产面的身份保证在服务层 HMAC 验签，见模块注释）。
    /// 域层消费它时只查 actor_type（纵深），不重复验签。
    pub fn human(actor_id: &str) -> Self {
        Self {
            actor_id: actor_id.to_string(),
            actor_type: ActorType::Human,
            authentication_method: AUTH_METHOD_HMAC_RECEIPT.to_string(),
            approval_receipt: String::new(),
            scope_is_agent: false,
        }
    }

    /// #277 方案 A：Agent 声明面便捷构造（域层单测用——生产面的 agent
    /// 身份只能来自 [`verify_agent_receipt`] 验签，本构造不自带身份保证；
    /// 与 [`Self::human`] 对偶）。`scope_is_agent: true`——域层二元组
    /// match 的 agent 侧入参。
    pub fn agent(actor_id: &str) -> Self {
        Self {
            actor_id: actor_id.to_string(),
            actor_type: ActorType::Agent,
            authentication_method: AUTH_METHOD_HMAC_RECEIPT.to_string(),
            approval_receipt: String::new(),
            scope_is_agent: true,
        }
    }
}

/// receipt 验签失败分类（服务层映射 E_UNPROCESSABLE 的报因面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptError {
    /// 结构非法（非六段 / 版本不符 / 段值不可解析）。
    Malformed,
    /// HMAC 不符（篡改/伪造/签域 scope 不匹配——receipt 不是本密钥对本
    /// 用途签发的）。
    BadSignature,
    /// 已过期（issued_at 早于 now-24h——陈旧重放）。
    Expired,
    /// 非 Human 身份（人类签域验签面只认 Human——#182 现状口径）。
    NotHuman,
    /// #277 方案 A：agent 签域验签面只认 Agent（Human 用 agent 签域 /
    /// System 身份 → 拒——签域与身份不匹配的明确分类）。
    NotAgent,
    /// 时间超前（issued_at > now+60s 容差——未来签发）。
    FutureDated,
    /// 内容漂移（#323/R-06：receipt 绑定的 sections_digest ≠ 验签时当前
    /// 摘要——签发后章节树被改，「批的是旧版内容」）。
    ContentDrifted,
    /// 旧版线格式（v1 六段无内容绑定——#323 起 fail-closed 拒收；
    /// 重新签发 v2 即可，无存量迁移面）。
    LegacyV1,
}

impl ReceiptError {
    /// 分类码（服务层 E_UNPROCESSABLE reason 的机器可读前缀）。
    pub fn code(&self) -> &'static str {
        match self {
            ReceiptError::Malformed => "MALFORMED_RECEIPT",
            ReceiptError::BadSignature => "BAD_SIGNATURE",
            ReceiptError::Expired => "EXPIRED_RECEIPT",
            ReceiptError::NotHuman => "NOT_HUMAN_ACTOR",
            ReceiptError::NotAgent => "NOT_AGENT_ACTOR",
            ReceiptError::FutureDated => "FUTURE_DATED_RECEIPT",
            ReceiptError::ContentDrifted => "CONTENT_DRIFTED_RECEIPT",
            ReceiptError::LegacyV1 => "LEGACY_V1_RECEIPT",
        }
    }
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ReceiptError::Malformed => "receipt 结构非法（须 v2.<actor_id>.<actor_type>.<scope>.<issued_at>.<sections_digest>.<hex_hmac> 七段）",
            ReceiptError::BadSignature => "receipt 验签失败（HMAC 不符或签域 scope 不匹配——非本密钥对本用途签发）",
            ReceiptError::Expired => "receipt 已过期（issued_at 超出 24h 有效窗——陈旧重放）",
            ReceiptError::NotHuman => "receipt 非 Human 身份（人类签域只认真人——Agent 声明过门走 :agent 签域，见 #277 方案 A）",
            ReceiptError::NotAgent => "receipt 非 Agent 身份（agent 签域只认 Agent——Human 不得用 agent 签域，Agent 不得用人类签域，签域与身份必须匹配）",
            ReceiptError::FutureDated => "receipt 签发时间超前（issued_at 超出 now+60s 容差）",
            ReceiptError::ContentDrifted => "receipt 内容漂移（签发后章节树已变更——批的是旧版内容，#323：重新走人工确认签发新 receipt）",
            ReceiptError::LegacyV1 => "receipt 为旧版 v1 六段格式（无内容绑定——#323 起 fail-closed 拒收，请重新签发 v2）",
        };
        f.write_str(s)
    }
}

impl std::error::Error for ReceiptError {}

// ---------------------------------------------------------------- 签发面
/// 宿主签发（Human）——纯函数：密钥与时刻由调用方注入（宿主持密钥，
/// `now` 取系统时钟；测试取定值）。域层不调用本函数（宿主/测试用）。
///
/// #323：`content_digest` = 签发时章节树摘要（服务层宿主面取
/// `DomainStore::sections_digest()`——真人看到/批下的内容状态锚点；
/// 过门时与当前摘要比对，变了即拒）。
///
/// # Panics
/// `actor_id` / `scope` 含 `.` 或 `|`（破坏七段/消息分隔的结构面）；
/// `content_digest` 非完整 64 位小写 hex——均为编程错误面（宿主侧的
/// 身份命名/scope 构造/digest 来源都是代码不是用户输入）。
pub fn issue_approval_receipt(
    key: &[u8; 32],
    actor_id: &str,
    scope: &str,
    now: u64,
    content_digest: &str,
) -> String {
    issue_approval_receipt_as(key, actor_id, ActorType::Human, scope, now, content_digest)
}

/// 任意 actor_type 的签发面——**测试/审计专用**（过审面只认 Human，宿主
/// 签发走 [`issue_approval_receipt`]）。pub 是为了让跨 crate 集成测试能
/// 构造「Agent 伪装」「过期重放」「内容漂移」等攻击样本（验签分类的
/// 验收面）——签发仍需持密钥，不构成生产面的旁路。
///
/// # Panics
/// 同 [`issue_approval_receipt`]。
pub fn issue_approval_receipt_as(
    key: &[u8; 32],
    actor_id: &str,
    actor_type: ActorType,
    scope: &str,
    now: u64,
    content_digest: &str,
) -> String {
    assert_structural(actor_id, "actor_id");
    assert_structural(scope, "scope");
    assert_sha256_hex(content_digest, "content_digest");
    let actor_type_s = actor_type.as_str();
    let mac = hmac_sha256(
        key,
        format!(
            "{RECEIPT_VERSION}|{actor_id}|{actor_type_s}|{scope}|{now}|{content_digest}"
        )
        .as_bytes(),
    );
    format!(
        "{RECEIPT_VERSION}.{actor_id}.{actor_type_s}.{scope}.{now}.{content_digest}.{}",
        hex::encode(mac)
    )
}

/// 段内禁字符断言（`.` 炸七段线格式、`|` 炸 HMAC 消息——结构面，编程错误）。
fn assert_structural(s: &str, what: &str) {
    assert!(
        !s.is_empty() && !s.contains('.') && !s.contains('|'),
        "{what} 不得为空、不得含 '.' 或 '|'（receipt 结构分隔符）：{s:?}"
    );
}

/// 内容摘要段结构断言（sha256 十六进制 64 位小写——编程错误面：
/// digest 来源是 `sections_digest()` 代码路径，不是用户输入）。
fn assert_sha256_hex(s: &str, what: &str) {
    assert!(
        is_sha256_hex(s),
        "{what} 须为 64 位小写十六进制（sha256——sections_digest 口径），实际：{s:?}"
    );
}

/// sha256 十六进制判据（64 位、只认小写——与 `hex::encode` 输出面一致；
/// 验签侧非此形态 → Malformed，不与签名混淆）。
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// ---------------------------------------------------------------- 验签面
/// 验签纯函数（零 IO / 零时钟——`now` 由调用方注入）。
///
/// #323：`current_content_digest` = 验签时当前章节树摘要（服务层取
/// `DomainStore::sections_digest()`）——与 receipt 第六段比对，
/// 不一致 → [`ReceiptError::ContentDrifted`]（宿主签发后章节被改）。
///
/// 裁决序（分类确定）：
/// 1. 七段解析 + 版本/段值合法性 → [`ReceiptError::Malformed`]；
///    （v1 六段 → [`ReceiptError::LegacyV1`]——fail-closed 迁移面）
/// 2. actor_type 只接受 Human → [`ReceiptError::NotHuman`]（Agent/System
///    的 receipt 不用于过审面）；
/// 3. 时效窗：issued_at > now+60 → [`ReceiptError::FutureDated`]；
///    issued_at < now-86400 → [`ReceiptError::Expired`]；
/// 4. scope 绑定 + HMAC 常量时间比对：期望 scope 直接进 MAC 输入——
///    他域 receipt（A 门 receipt 过 B 门）MAC 必不符 → [`ReceiptError::BadSignature`]；
///    摘要段也在 MAC 输入里——篡改摘要段同样 BadSignature。
/// 5. 内容绑定（#323）：receipt 摘要 ≠ 当前摘要 → [`ReceiptError::ContentDrifted`]；
///    置于 HMAC 之后——只对验签成功的真 receipt 报漂移，不为伪造样本泄信息。
pub fn verify_approval_receipt(
    receipt: &str,
    key: &[u8; 32],
    scope: &str,
    now: u64,
    current_content_digest: &str,
) -> Result<TrustedPrincipal, ReceiptError> {
    // ---- 1. 七段解析（线格式 v2.<actor_id>.<actor_type>.<scope>.<issued_at>.<digest>.<mac>）
    let parsed = parse_receipt(receipt)?;
    let ParsedReceipt { actor_id, actor_type, receipt_scope, issued_at, content_digest, mac_hex: _ } =
        &parsed;

    // ---- 2. 过审面只认 Human（Agent/System 的 receipt 不用于过审面）
    if *actor_type != ActorType::Human {
        return Err(ReceiptError::NotHuman);
    }

    // ---- 3. 时效窗（24h 有效期 + 60s 未来容差）
    verify_time_window(*issued_at, now)?;

    // ---- 4. scope 绑定 + HMAC 常量时间比对
    //      期望 scope 进 MAC 输入：他域 receipt 的 MAC 对不上（BadSignature），
    //      签域不符与签名伪造同一分类——都意味着「这串 receipt 不是本密钥
    //      对本用途签发的」。
    if receipt_scope.as_str() != scope {
        return Err(ReceiptError::BadSignature);
    }
    verify_mac(key, &parsed)?;

    // ---- 5. 内容绑定（#323/R-06）——批的是签发时的内容，不是现在的
    if content_digest.as_str() != current_content_digest {
        return Err(ReceiptError::ContentDrifted);
    }

    Ok(TrustedPrincipal {
        actor_id: actor_id.clone(),
        actor_type: *actor_type,
        authentication_method: AUTH_METHOD_HMAC_RECEIPT.to_string(),
        approval_receipt: receipt.to_string(),
        scope_is_agent: false,
    })
}

/// #277 方案 A：agent 签域验签面——receipt 的 actor_type 必须 Agent 且
/// scope 为 agent 签域（`…:agent` 后缀，由调用方传入——服务层构造）。
///
/// 与 [`verify_approval_receipt`] 对偶：身份/签域一致性由验签函数结构保证
/// （Agent 只能从本函数验出，`scope_is_agent` 恒 true）——域层
/// `(actor_type, scope_is_agent)` match 的 agent 侧只能从这里构造。
/// **验签通过 ≠ 放行**：`require_human_receipt` 工作区配置（默认 true，
/// 严格模式）在服务层裁决——本函数只证明「receipt 是本密钥对 agent 签域
/// 签发的」，不证明「agent 被允许过门」。
/// #323：`current_content_digest` 与人类面同口径——agent 声明同样绑内容
/// （声明过门后内容再变 = 声明针对旧内容，拒）。
///
/// 裁决序与人类面同构：七段解析 → actor 门（只认 Agent）→ 时效窗 →
/// 签域绑定 + HMAC 常量时间比对 → 内容绑定（#323）。
pub fn verify_agent_receipt(
    receipt: &str,
    key: &[u8; 32],
    agent_scope: &str,
    now: u64,
    current_content_digest: &str,
) -> Result<TrustedPrincipal, ReceiptError> {
    let parsed = parse_receipt(receipt)?;
    let ParsedReceipt { actor_id, actor_type, receipt_scope, issued_at, content_digest, mac_hex: _ } =
        &parsed;

    // ---- agent 签域只认 Agent（Human 用 agent 签域 → 拒；System 同拒）
    if *actor_type != ActorType::Agent {
        return Err(ReceiptError::NotAgent);
    }

    // ---- 时效窗（与人类面同参数——agent 声明同样怕陈旧重放）
    verify_time_window(*issued_at, now)?;

    // ---- 签域绑定 + HMAC 常量时间比对（scope 进 MAC 输入——人类签域的
    //      receipt 在这里自然 BadSignature，无需特判）
    if receipt_scope.as_str() != agent_scope {
        return Err(ReceiptError::BadSignature);
    }
    verify_mac(key, &parsed)?;

    // ---- 内容绑定（#323——agent 声明同样不得针对旧内容）
    if content_digest.as_str() != current_content_digest {
        return Err(ReceiptError::ContentDrifted);
    }

    Ok(TrustedPrincipal {
        actor_id: actor_id.clone(),
        actor_type: *actor_type,
        authentication_method: AUTH_METHOD_HMAC_RECEIPT.to_string(),
        approval_receipt: receipt.to_string(),
        scope_is_agent: true,
    })
}

/// 解析后的 receipt 各段（验签面共享结构——人类/agent 两路只差 actor 门
/// 与签域，解析/时效窗/MAC 三段同构，#323 起再加内容绑定段）。
struct ParsedReceipt {
    actor_id: String,
    actor_type: ActorType,
    receipt_scope: String,
    issued_at: u64,
    content_digest: String,
    mac_hex: String,
}

/// 七段解析 + 版本/段值合法性（v1 六段 → LegacyV1；其余结构残缺 → Malformed）。
fn parse_receipt(receipt: &str) -> Result<ParsedReceipt, ReceiptError> {
    let parts: Vec<&str> = receipt.split('.').collect();
    // v1 六段识别在先：旧版 receipt 给迁移分类（不与结构残缺混淆），
    // 其余段数一律 Malformed（含 6 段但版本非 v1 的怪形）。
    if parts.len() == 6 && parts[0] == LEGACY_RECEIPT_VERSION {
        return Err(ReceiptError::LegacyV1);
    }
    if parts.len() != 7 {
        return Err(ReceiptError::Malformed);
    }
    let [version, actor_id, actor_type_s, receipt_scope, issued_at_s, content_digest, mac_hex] =
        [parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6]];
    if version != RECEIPT_VERSION {
        return Err(ReceiptError::Malformed);
    }
    if actor_id.is_empty() || receipt_scope.is_empty() {
        return Err(ReceiptError::Malformed);
    }
    let actor_type = ActorType::from_str(actor_type_s).ok_or(ReceiptError::Malformed)?;
    let issued_at: u64 = issued_at_s.parse().map_err(|_| ReceiptError::Malformed)?;
    // 摘要段结构：sha256 小写 64 位（非此形态 = 结构残缺，不是签名不符）
    if !is_sha256_hex(content_digest) {
        return Err(ReceiptError::Malformed);
    }
    // mac 段结构把关同归 parse（验签函数不再重复）
    let presented = hex::decode(mac_hex).map_err(|_| ReceiptError::Malformed)?;
    if presented.len() != 32 {
        // 结构面：HMAC-SHA256 恒 32 字节——非 32 字节的 mac 段是结构残缺
        // 不是签名不符（空串/截断的 hex 都解得出字节，长度把关归 Malformed）。
        return Err(ReceiptError::Malformed);
    }
    Ok(ParsedReceipt {
        actor_id: actor_id.to_string(),
        actor_type,
        receipt_scope: receipt_scope.to_string(),
        issued_at,
        content_digest: content_digest.to_string(),
        mac_hex: mac_hex.to_string(),
    })
}

/// 时效窗（两路共享——24h 有效期 + 60s 未来容差）。
fn verify_time_window(issued_at: u64, now: u64) -> Result<(), ReceiptError> {
    if issued_at > now.saturating_add(RECEIPT_FUTURE_TOLERANCE_SECS) {
        return Err(ReceiptError::FutureDated);
    }
    if now.saturating_sub(issued_at) > RECEIPT_TTL_SECS {
        return Err(ReceiptError::Expired);
    }
    Ok(())
}

/// scope 绑定后的 MAC 常量时间比对（消息含摘要段——篡改任何已签段
/// （scope/issued_at/digest/actor）都是 BadSignature）。
fn verify_mac(key: &[u8; 32], p: &ParsedReceipt) -> Result<(), ReceiptError> {
    let presented = hex::decode(&p.mac_hex).map_err(|_| ReceiptError::Malformed)?;
    debug_assert_eq!(presented.len(), 32, "parse_receipt 已把关 32 字节");
    let expected = hmac_sha256(
        key,
        format!(
            "{RECEIPT_VERSION}|{}|{}|{}|{}|{}",
            p.actor_id,
            p.actor_type.as_str(),
            p.receipt_scope,
            p.issued_at,
            p.content_digest
        )
        .as_bytes(),
    );
    if !ct_eq(&presented, &expected) {
        return Err(ReceiptError::BadSignature);
    }
    Ok(())
}

/// HMAC-SHA256（一次性消息——Simple MAC 面无需流式 update）。
fn hmac_sha256(key: &[u8; 32], msg: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key)
        .expect("HMAC-SHA256 接受任意长度密钥（[u8; 32] 恒可）");
    mac.update(msg);
    let out = mac.finalize().into_bytes();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

/// 常量时间字节比对（无提前 return 的短路——每字节都参与折叠；长度不等
/// 直接 false：长度不是秘密，两端正常都是 32 字节 HMAC）。
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: [u8; 32] = [7u8; 32];
    const SCOPE: &str = "review_gate:pre_export_confirm";

    /// 测试用合法摘要（64 位小写 hex——sections_digest 形态）。
    fn dg(c: char) -> String {
        std::iter::repeat(c).take(64).collect()
    }

    fn verify(receipt: &str, now: u64) -> Result<TrustedPrincipal, ReceiptError> {
        verify_approval_receipt(receipt, &TEST_KEY, SCOPE, now, &dg('a'))
    }

    #[test]
    fn issue_verify_roundtrip_yields_human_principal() {
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now, &dg('a'));
        let p = verify(&r, now).unwrap();
        assert_eq!(p.actor_id, "admin");
        assert_eq!(p.actor_type, ActorType::Human);
        assert_eq!(p.authentication_method, "hmac_receipt_v2");
        assert_eq!(p.approval_receipt, r, "审计留痕：完整 receipt 在座");
    }

    // ------------------------------------------------------------ #323 内容绑定

    #[test]
    fn content_drift_after_issue_is_rejected() {
        // R-06 负例主线：签发（摘要 a…）→ 章节内容变更（摘要变 b…）→
        // 旧 receipt 过同一门 → ContentDrifted（批的是旧版内容）。
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now, &dg('a'));
        assert_eq!(
            verify_approval_receipt(&r, &TEST_KEY, SCOPE, now, &dg('b')),
            Err(ReceiptError::ContentDrifted)
        );
        // 内容未变（摘要同值）→ 放行
        assert!(verify_approval_receipt(&r, &TEST_KEY, SCOPE, now, &dg('a')).is_ok());
    }

    #[test]
    fn tampered_digest_segment_is_bad_signature() {
        // 篡改摘要段（第六位换字符）——摘要在 MAC 输入里：
        // 换摘要不改 mac = BadSignature（不是 ContentDrifted——裁决序在
        // HMAC 之后才比摘要）。
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now, &dg('a'));
        let tampered = r.replace(&dg('a'), &dg('b'));
        assert_eq!(verify(&tampered, now), Err(ReceiptError::BadSignature));
    }

    #[test]
    fn legacy_v1_six_segment_receipt_is_rejected() {
        // 迁移裁定（fail-closed）：v1 六段（无内容绑定）→ LegacyV1，
        // 不与 Malformed/BadSignature 混淆——重签 v2 即可。
        let now = 1_800_000_000u64;
        let legacy = format!("v1.admin.human.{SCOPE}.{now}.{}", "ab".repeat(32));
        assert_eq!(
            verify_approval_receipt(&legacy, &TEST_KEY, SCOPE, now, &dg('a')),
            Err(ReceiptError::LegacyV1)
        );
        // agent 面同口径拒收
        let legacy_agent = format!("v1.agent-01.agent.{SCOPE}:agent.{now}.{}", "ab".repeat(32));
        assert_eq!(
            verify_agent_receipt(&legacy_agent, &TEST_KEY, &format!("{SCOPE}:agent"), now, &dg('a')),
            Err(ReceiptError::LegacyV1)
        );
    }

    #[test]
    fn non_hex_digest_segment_is_malformed() {
        let now = 1_800_000_000u64;
        // 短摘要 / 大写 / 非 hex —— 结构残缺
        for bad in ["short", &dg('A'), &format!("{}z", dg('a').chars().take(63).collect::<String>())] {
            let r = format!("v2.admin.human.{SCOPE}.{now}.{bad}.{}", "ab".repeat(32));
            assert_eq!(
                verify_approval_receipt(&r, &TEST_KEY, SCOPE, now, &dg('a')),
                Err(ReceiptError::Malformed),
                "须 Malformed：{bad:?}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "content_digest")]
    fn issue_panics_on_structural_digest() {
        issue_approval_receipt(&TEST_KEY, "admin", SCOPE, 0, "not-a-sha256");
    }

    // ------------------------------------------------------------ 原有分类面

    #[test]
    fn tampered_hex_is_bad_signature() {
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now, &dg('a'));
        // 改末段一个 hex 字符（签名段——伪造面）
        let mut chars = r.chars().collect::<Vec<_>>();
        let last = chars.last_mut().unwrap();
        *last = if *last == 'a' { 'b' } else { 'a' };
        let tampered: String = chars.into_iter().collect();
        assert_eq!(verify(&tampered, now), Err(ReceiptError::BadSignature));
    }

    #[test]
    fn wrong_key_is_bad_signature() {
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt(&[9u8; 32], "admin", SCOPE, now, &dg('a'));
        assert_eq!(verify(&r, now), Err(ReceiptError::BadSignature));
    }

    #[test]
    fn scope_mismatch_is_bad_signature() {
        let now = 1_800_000_000u64;
        // A 门的 receipt 过 B 门：scope 绑定
        let r = issue_approval_receipt(&TEST_KEY, "admin", "review_gate:matrix_approval", now, &dg('a'));
        assert_eq!(verify(&r, now), Err(ReceiptError::BadSignature));
        // TF-0003 的 receipt 免不了 TF-0004
        let w = issue_approval_receipt(&TEST_KEY, "admin", "addendum_waiver:TF-0003", now, &dg('a'));
        assert_eq!(
            verify_approval_receipt(&w, &TEST_KEY, "addendum_waiver:TF-0004", now, &dg('a')),
            Err(ReceiptError::BadSignature)
        );
    }

    #[test]
    fn agent_and_system_receipts_are_not_human() {
        let now = 1_800_000_000u64;
        for ty in [ActorType::Agent, ActorType::System] {
            let r = issue_approval_receipt_as(&TEST_KEY, "bot-01", ty, SCOPE, now, &dg('a'));
            assert_eq!(verify(&r, now), Err(ReceiptError::NotHuman), "{ty:?}");
        }
    }

    #[test]
    fn stale_receipt_is_expired() {
        let issued = 1_800_000_000u64;
        let r = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, issued, &dg('a'));
        // 窗内边界：恰好 24h → 放行
        assert!(verify(&r, issued + RECEIPT_TTL_SECS).is_ok());
        // 越窗 1 秒 → Expired（陈旧重放）
        assert_eq!(
            verify(&r, issued + RECEIPT_TTL_SECS + 1),
            Err(ReceiptError::Expired)
        );
    }

    #[test]
    fn future_receipt_outside_tolerance_is_rejected() {
        let now = 1_800_000_000u64;
        // 容差内（+60s）：时钟微漂吸收
        let within = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now + RECEIPT_FUTURE_TOLERANCE_SECS, &dg('a'));
        assert!(verify(&within, now).is_ok());
        // 容差外（+61s）：FutureDated
        let beyond = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now + RECEIPT_FUTURE_TOLERANCE_SECS + 1, &dg('a'));
        assert_eq!(verify(&beyond, now), Err(ReceiptError::FutureDated));
    }

    #[test]
    fn structural_garbage_is_malformed() {
        let now = 1_800_000_000u64;
        // 注：裁决序是 解析 → Human → 时效窗 → 签域 → HMAC——时间窗与签域
        // 检查先于 hex 解码，故对应样本的 issued_at 须取 now、scope 须取
        // 期望签域（否则先吃 Expired / BadSignature）。
        for bad in [
            "",                                        // 空
            "v1",                                      // 单段
            "v3.admin.human.scope.1.<dg>.deadbeef",         // 版本超前（非 v1/v2）
            "v2.admin.person.scope.1.<dg>.deadbeef",        // 未知 actor_type
            "v2.admin.human.scope.notanumber.<dg>.de",      // issued_at 非数
            &format!("v2.admin.human.{SCOPE}.{now}.<dg>.zz!!"), // mac 非 hex
            "v2..human.scope.1.<dg>.deadbeef",              // 空 actor_id
            &format!("v2.admin.human.{SCOPE}.{now}.<dg>."), // 空 mac（七段但末段空）
            &format!("v2.admin.human.{SCOPE}.{now}"),        // 六段但版本 v2 → Malformed（非 v1 六段）
        ] {
            let bad = bad.replace("<dg>", &dg('a'));
            assert_eq!(
                verify_approval_receipt(&bad, &TEST_KEY, SCOPE, now, &dg('a')),
                Err(ReceiptError::Malformed),
                "须 Malformed：{bad:?}"
            );
        }
    }

    #[test]
    fn human_convenience_constructor_is_human() {
        let p = TrustedPrincipal::human("zhang");
        assert_eq!(p.actor_type, ActorType::Human);
        assert_eq!(p.actor_id, "zhang");
        assert_eq!(p.authentication_method, "hmac_receipt_v2");
    }

    #[test]
    #[should_panic(expected = "actor_id")]
    fn issue_panics_on_structural_actor_id() {
        issue_approval_receipt(&TEST_KEY, "zhang.san", SCOPE, 0, &dg('a'));
    }

    #[test]
    #[should_panic(expected = "scope")]
    fn issue_panics_on_structural_scope() {
        issue_approval_receipt(&TEST_KEY, "zhang", "a.b", 0, &dg('a'));
    }

    // ------------------------------------------------------------ #277 方案 A

    const AGENT_SCOPE: &str = "review_gate:pre_export_confirm:agent";

    #[test]
    fn agent_receipt_roundtrip_yields_agent_principal() {
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt_as(&TEST_KEY, "agent-01", ActorType::Agent, AGENT_SCOPE, now, &dg('a'));
        let p = verify_agent_receipt(&r, &TEST_KEY, AGENT_SCOPE, now, &dg('a')).unwrap();
        assert_eq!(p.actor_id, "agent-01");
        assert_eq!(p.actor_type, ActorType::Agent);
        assert!(p.scope_is_agent, "agent 签域验出 → scope_is_agent=true");
    }

    #[test]
    fn agent_receipt_content_binding_rejects_drift() {
        // #323：agent 声明过门同样绑内容——声明后内容变更 → 拒
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt_as(&TEST_KEY, "agent-01", ActorType::Agent, AGENT_SCOPE, now, &dg('a'));
        assert_eq!(
            verify_agent_receipt(&r, &TEST_KEY, AGENT_SCOPE, now, &dg('b')),
            Err(ReceiptError::ContentDrifted)
        );
    }

    #[test]
    fn human_receipt_on_agent_scope_is_not_agent() {
        // 验收 3：Human 用 agent 签域 → 拒（防止混淆）
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt_as(&TEST_KEY, "admin", ActorType::Human, AGENT_SCOPE, now, &dg('a'));
        assert_eq!(
            verify_agent_receipt(&r, &TEST_KEY, AGENT_SCOPE, now, &dg('a')),
            Err(ReceiptError::NotAgent)
        );
    }

    #[test]
    fn agent_receipt_on_human_scope_is_not_human() {
        // 对偶面：Agent 用人类签域 → 人类验签面拒（NotHuman）；agent 验签面
        // 对该门的 agent 签域（{SCOPE}:agent，服务层构造口径）→ BadSignature
        // （receipt 的 scope 段进 MAC，天然不符）
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt_as(&TEST_KEY, "agent-01", ActorType::Agent, SCOPE, now, &dg('a'));
        assert_eq!(verify(&r, now), Err(ReceiptError::NotHuman));
        let gate_agent_scope = format!("{SCOPE}:agent");
        assert_eq!(
            verify_agent_receipt(&r, &TEST_KEY, &gate_agent_scope, now, &dg('a')),
            Err(ReceiptError::BadSignature),
            "agent receipt（人类签域签发）对该门 agent 签域：scope 段进 MAC → BadSignature"
        );
    }

    #[test]
    fn agent_receipt_gate_binding_and_tamper() {
        let now = 1_800_000_000u64;
        // 签域绑定：A 门 agent receipt 过不了 B 门
        let other = "review_gate:matrix_approval:agent";
        let r = issue_approval_receipt_as(&TEST_KEY, "agent-01", ActorType::Agent, other, now, &dg('a'));
        assert_eq!(
            verify_agent_receipt(&r, &TEST_KEY, AGENT_SCOPE, now, &dg('a')),
            Err(ReceiptError::BadSignature)
        );
        // 篡改末段 → BadSignature
        let mut chars = r.chars().collect::<Vec<_>>();
        let last = chars.last_mut().unwrap();
        *last = if *last == 'a' { 'b' } else { 'a' };
        let tampered: String = chars.into_iter().collect();
        assert_eq!(
            verify_agent_receipt(&tampered, &TEST_KEY, other, now, &dg('a')),
            Err(ReceiptError::BadSignature)
        );
        // 时效窗：越 24h → Expired
        let stale = issue_approval_receipt_as(&TEST_KEY, "agent-01", ActorType::Agent, other, now, &dg('a'));
        assert_eq!(
            verify_agent_receipt(&stale, &TEST_KEY, other, now + RECEIPT_TTL_SECS + 1, &dg('a')),
            Err(ReceiptError::Expired)
        );
    }

    #[test]
    fn agent_convenience_constructor_flags_scope() {
        let p = TrustedPrincipal::agent("bot-9");
        assert_eq!(p.actor_type, ActorType::Agent);
        assert!(p.scope_is_agent);
        // 旧快照反序列化（验收 8）：无 scope_is_agent 字段的 JSON → false
        // （authentication_method 是回显字符串不是判据——旧快照里的
        // "hmac_receipt_v1" 原样保留，不做版本改写）
        let legacy = serde_json::json!({
            "actor_id": "admin", "actor_type": "HUMAN",
            "authentication_method": "hmac_receipt_v1", "approval_receipt": "v1..."
        });
        let p2: TrustedPrincipal = serde_json::from_value(legacy).unwrap();
        assert!(!p2.scope_is_agent, "旧快照缺省 false（人类签域口径）");
    }
}
