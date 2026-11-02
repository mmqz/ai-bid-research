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
//! receipt 线格式（六段，点分隔）：
//! `v1.<actor_id>.<actor_type>.<scope>.<issued_at_unix_secs>.<hex_hmac_sha256>`
//!
//! - HMAC 消息 = `v1|actor_id|actor_type|scope|issued_at`（竖线分隔——与线格式
//!   分隔符错开，段内含 `:`（scope 如 `review_gate:pre_export_confirm`）不炸）；
//! - **scope 绑定**：receipt 只对签发时声明的用途有效（`review_gate:<gate>`
//!   / `addendum_waiver:<file_id>`）——A 门的 receipt 过不了 B 门、TF-0003 的
//!   receipt 免不了 TF-0004（重放面收敛到具体裁决位）；
//! - **时效窗**：`issued_at ∈ [now-86400, now+60]`——24h 有效期防陈旧重放，
//!   +60s 容差吸收宿主/服务侧时钟微漂（越窗 → Expired / FutureDated）；
//! - **常量时间比对**：HMAC 比对不做短路比较（见 [`ct_eq`）——时序侧信道
//!   不泄漏签名前缀信息。
//!
//! 纯函数纪律：本模块零 IO、零时钟（`now` 由调用方注入——服务层取系统
//! 时钟，测试取定值），与 crate 纪律一致。签发/验签的密钥管理在 bid-tools
//! 服务层（`approval_key`——持久化工作区落 `<dir>/.mang/approval.key`，
//! 纯内存态内存生成不落盘）。

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;

/// receipt 线格式版本（v1 起步——格式演进时升版本号，验签面只认当版）。
pub const RECEIPT_VERSION: &str = "v1";

/// 验签通过的 TrustedPrincipal 携带的认证方式标记（审计面回显）。
pub const AUTH_METHOD_HMAC_RECEIPT: &str = "hmac_receipt_v1";

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
/// 域层（record_review / record_addendum_waiver）只校验 `actor_type == Human`
/// （纵深防御）；真正的身份保证在服务层 HMAC 验签——域层零密钥零时钟，
/// 不重复实现验签（单一真源）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustedPrincipal {
    /// 操作者 id（如 "admin"、"import-lead"——宿主侧的身份命名）。
    pub actor_id: String,
    /// 操作者类别（过审面只认 Human）。
    pub actor_type: ActorType,
    /// 认证方式（"hmac_receipt_v1"——审计面回显）。
    pub authentication_method: String,
    /// 完整 receipt 串（审计留痕——回溯时可复验）。
    pub approval_receipt: String,
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
    /// 非 Human 身份（Agent/System 的 receipt 不用于过审面）。
    NotHuman,
    /// 时间超前（issued_at > now+60s 容差——未来签发）。
    FutureDated,
}

impl ReceiptError {
    /// 分类码（服务层 E_UNPROCESSABLE reason 的机器可读前缀）。
    pub fn code(&self) -> &'static str {
        match self {
            ReceiptError::Malformed => "MALFORMED_RECEIPT",
            ReceiptError::BadSignature => "BAD_SIGNATURE",
            ReceiptError::Expired => "EXPIRED_RECEIPT",
            ReceiptError::NotHuman => "NOT_HUMAN_ACTOR",
            ReceiptError::FutureDated => "FUTURE_DATED_RECEIPT",
        }
    }
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ReceiptError::Malformed => "receipt 结构非法（须 v1.<actor_id>.<actor_type>.<scope>.<issued_at>.<hex_hmac> 六段）",
            ReceiptError::BadSignature => "receipt 验签失败（HMAC 不符或签域 scope 不匹配——非本密钥对本用途签发）",
            ReceiptError::Expired => "receipt 已过期（issued_at 超出 24h 有效窗——陈旧重放）",
            ReceiptError::NotHuman => "receipt 非 Human 身份（Agent/System 不得过人工审核面）",
            ReceiptError::FutureDated => "receipt 签发时间超前（issued_at 超出 now+60s 容差）",
        };
        f.write_str(s)
    }
}

impl std::error::Error for ReceiptError {}

// ---------------------------------------------------------------- 签发面
/// 宿主签发（Human）——纯函数：密钥与时刻由调用方注入（宿主持密钥，
/// `now` 取系统时钟；测试取定值）。域层不调用本函数（宿主/测试用）。
///
/// # Panics
/// `actor_id` / `scope` 含 `.` 或 `|`（破坏六段/消息分隔的结构面）——
/// 编程错误面，宿主侧的身份命名与 scope 构造都是代码不是用户输入。
pub fn issue_approval_receipt(key: &[u8; 32], actor_id: &str, scope: &str, now: u64) -> String {
    issue_approval_receipt_as(key, actor_id, ActorType::Human, scope, now)
}

/// 任意 actor_type 的签发面——**测试/审计专用**（过审面只认 Human，宿主
/// 签发走 [`issue_approval_receipt`]）。pub 是为了让跨 crate 集成测试能
/// 构造「Agent 伪装」「过期重放」等攻击样本（验签分类的验收面）——签发
/// 仍需持密钥，不构成生产面的旁路。
///
/// # Panics
/// 同 [`issue_approval_receipt`]。
pub fn issue_approval_receipt_as(
    key: &[u8; 32],
    actor_id: &str,
    actor_type: ActorType,
    scope: &str,
    now: u64,
) -> String {
    assert_structural(actor_id, "actor_id");
    assert_structural(scope, "scope");
    let actor_type_s = actor_type.as_str();
    let mac = hmac_sha256(
        key,
        format!("{RECEIPT_VERSION}|{actor_id}|{actor_type_s}|{scope}|{now}").as_bytes(),
    );
    format!(
        "{RECEIPT_VERSION}.{actor_id}.{actor_type_s}.{scope}.{now}.{}",
        hex::encode(mac)
    )
}

/// 段内禁字符断言（`.` 炸六段线格式、`|` 炸 HMAC 消息——结构面，编程错误）。
fn assert_structural(s: &str, what: &str) {
    assert!(
        !s.is_empty() && !s.contains('.') && !s.contains('|'),
        "{what} 不得为空、不得含 '.' 或 '|'（receipt 结构分隔符）：{s:?}"
    );
}

// ---------------------------------------------------------------- 验签面
/// 验签纯函数（零 IO / 零时钟——`now` 由调用方注入）。
///
/// 裁决序（分类确定）：
/// 1. 六段解析 + 版本/段值合法性 → [`ReceiptError::Malformed`]；
/// 2. actor_type 只接受 Human → [`ReceiptError::NotHuman`]（Agent/System
///    的 receipt 不用于过审面）；
/// 3. 时效窗：issued_at > now+60 → [`ReceiptError::FutureDated`]；
///    issued_at < now-86400 → [`ReceiptError::Expired`]；
/// 4. scope 绑定 + HMAC 常量时间比对：期望 scope 直接进 MAC 输入——
///    他域 receipt（A 门 receipt 过 B 门）MAC 必不符 → [`ReceiptError::BadSignature`]。
pub fn verify_approval_receipt(
    receipt: &str,
    key: &[u8; 32],
    scope: &str,
    now: u64,
) -> Result<TrustedPrincipal, ReceiptError> {
    // ---- 1. 六段解析（线格式 v1.<actor_id>.<actor_type>.<scope>.<issued_at>.<mac>）
    let parts: Vec<&str> = receipt.split('.').collect();
    if parts.len() != 6 {
        return Err(ReceiptError::Malformed);
    }
    let [version, actor_id, actor_type_s, receipt_scope, issued_at_s, mac_hex] =
        [parts[0], parts[1], parts[2], parts[3], parts[4], parts[5]];
    if version != RECEIPT_VERSION {
        return Err(ReceiptError::Malformed);
    }
    if actor_id.is_empty() || receipt_scope.is_empty() {
        return Err(ReceiptError::Malformed);
    }
    let actor_type =
        ActorType::from_str(actor_type_s).ok_or(ReceiptError::Malformed)?;
    let issued_at: u64 = issued_at_s
        .parse()
        .map_err(|_| ReceiptError::Malformed)?;

    // ---- 2. 过审面只认 Human（Agent/System 的 receipt 不用于过审面）
    if actor_type != ActorType::Human {
        return Err(ReceiptError::NotHuman);
    }

    // ---- 3. 时效窗（24h 有效期 + 60s 未来容差）
    if issued_at > now.saturating_add(RECEIPT_FUTURE_TOLERANCE_SECS) {
        return Err(ReceiptError::FutureDated);
    }
    if now.saturating_sub(issued_at) > RECEIPT_TTL_SECS {
        return Err(ReceiptError::Expired);
    }

    // ---- 4. scope 绑定 + HMAC 常量时间比对
    //      期望 scope 进 MAC 输入：他域 receipt 的 MAC 对不上（BadSignature），
    //      签域不符与签名伪造同一分类——都意味着「这串 receipt 不是本密钥
    //      对本用途签发的」。
    if receipt_scope != scope {
        return Err(ReceiptError::BadSignature);
    }
    let presented = hex::decode(mac_hex).map_err(|_| ReceiptError::Malformed)?;
    if presented.len() != 32 {
        // 结构面：HMAC-SHA256 恒 32 字节——非 32 字节的 mac 段是结构残缺
        // 不是签名不符（空串/截断的 hex 都解得出字节，长度把关归 Malformed）。
        return Err(ReceiptError::Malformed);
    }
    let expected = hmac_sha256(
        key,
        format!("{RECEIPT_VERSION}|{actor_id}|{actor_type_s}|{receipt_scope}|{issued_at_s}")
            .as_bytes(),
    );
    if !ct_eq(&presented, &expected) {
        return Err(ReceiptError::BadSignature);
    }

    Ok(TrustedPrincipal {
        actor_id: actor_id.to_string(),
        actor_type,
        authentication_method: AUTH_METHOD_HMAC_RECEIPT.to_string(),
        approval_receipt: receipt.to_string(),
    })
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

    fn verify(receipt: &str, now: u64) -> Result<TrustedPrincipal, ReceiptError> {
        verify_approval_receipt(receipt, &TEST_KEY, SCOPE, now)
    }

    #[test]
    fn issue_verify_roundtrip_yields_human_principal() {
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now);
        let p = verify(&r, now).unwrap();
        assert_eq!(p.actor_id, "admin");
        assert_eq!(p.actor_type, ActorType::Human);
        assert_eq!(p.authentication_method, "hmac_receipt_v1");
        assert_eq!(p.approval_receipt, r, "审计留痕：完整 receipt 在座");
    }

    #[test]
    fn tampered_hex_is_bad_signature() {
        let now = 1_800_000_000u64;
        let r = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now);
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
        let r = issue_approval_receipt(&[9u8; 32], "admin", SCOPE, now);
        assert_eq!(verify(&r, now), Err(ReceiptError::BadSignature));
    }

    #[test]
    fn scope_mismatch_is_bad_signature() {
        let now = 1_800_000_000u64;
        // A 门的 receipt 过 B 门：scope 绑定
        let r = issue_approval_receipt(&TEST_KEY, "admin", "review_gate:matrix_approval", now);
        assert_eq!(verify(&r, now), Err(ReceiptError::BadSignature));
        // TF-0003 的 receipt 免不了 TF-0004
        let w = issue_approval_receipt(&TEST_KEY, "admin", "addendum_waiver:TF-0003", now);
        assert_eq!(
            verify_approval_receipt(&w, &TEST_KEY, "addendum_waiver:TF-0004", now),
            Err(ReceiptError::BadSignature)
        );
    }

    #[test]
    fn agent_and_system_receipts_are_not_human() {
        let now = 1_800_000_000u64;
        for ty in [ActorType::Agent, ActorType::System] {
            let r = issue_approval_receipt_as(&TEST_KEY, "bot-01", ty, SCOPE, now);
            assert_eq!(verify(&r, now), Err(ReceiptError::NotHuman), "{ty:?}");
        }
    }

    #[test]
    fn stale_receipt_is_expired() {
        let issued = 1_800_000_000u64;
        let r = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, issued);
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
        let within = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now + RECEIPT_FUTURE_TOLERANCE_SECS);
        assert!(verify(&within, now).is_ok());
        // 容差外（+61s）：FutureDated
        let beyond = issue_approval_receipt(&TEST_KEY, "admin", SCOPE, now + RECEIPT_FUTURE_TOLERANCE_SECS + 1);
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
            "v2.admin.human.scope.1.deadbeef",         // 版本不符
            "v1.admin.person.scope.1.deadbeef",        // 未知 actor_type
            "v1.admin.human.scope.notanumber.de",      // issued_at 非数
            &format!("v1.admin.human.{SCOPE}.{now}.zz!!"), // mac 非 hex
            "v1..human.scope.1.deadbeef",              // 空 actor_id
            &format!("v1.admin.human.{SCOPE}.{now}."), // 空 mac（六段但末段空）
        ] {
            assert_eq!(
                verify_approval_receipt(bad, &TEST_KEY, SCOPE, now),
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
        assert_eq!(p.authentication_method, "hmac_receipt_v1");
    }

    #[test]
    #[should_panic(expected = "actor_id")]
    fn issue_panics_on_structural_actor_id() {
        issue_approval_receipt(&TEST_KEY, "zhang.san", SCOPE, 0);
    }

    #[test]
    #[should_panic(expected = "scope")]
    fn issue_panics_on_structural_scope() {
        issue_approval_receipt(&TEST_KEY, "zhang", "a.b", 0);
    }
}
