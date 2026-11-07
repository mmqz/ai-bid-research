//! cursor.rs —— S-4 大结果分页游标核心（issue #11 批次 v3 / V3-5）。
//!
//! 功能定位：12,000 行矩阵 / 10,000 条证据不能一次全返回——所有列表类工具
//! （`get_compliance_matrix` / `list_evidence` / `list_tasks`，以及 M2-b 落地时
//! 的 `search_evidence`）统一走本模块的 opaque cursor 信封：
//! `{ rows|items, total, limit, cursor(本次生效), next_cursor: string|null }`。
//! `next_cursor=null` 即取完；调用方把 next_cursor 原样回传取下一页。
//!
//! 设计决策：
//! - **游标即偏移**：cursor = base64(JSON{v, offset, fp, total})——对调用方 opaque，
//!   对本层只是「过滤后列表的稳定偏移」。不做键集分页（keyset）是因为各列表
//!   的业务键形态各异（CMP 位置序 / EV id 字典序 / SEC target_id 字典序），
//!   而工具层已保证**行序确定性**（见各工具的分页序注释），偏移在确定序上
//!   即安全；
//! - **游标与过滤器绑定（fail-closed）**：fp = (工具名 + project_id + 过滤参数)
//!   的 FNV-1a 64 稳定哈希（16 hex）。用 A 过滤器拿的游标配 B 过滤器调用 →
//!   E_UNPROCESSABLE 拒绝——静默给错页数据比报错危险得多（borrow：mdcore
//!   corrupt-sidecar 哲学——损坏/错配输入响亮拒绝，不猜测加载）。工具名也入
//!   fp：不同工具的 offset 索引的是不同列表，跨工具复用同样拒；
//! - **游标与行集大小绑定（C-14 / #399）**：v2 起游标签发时携带「过滤后总数」
//!   total_at_issue，翻页时与当前总数比对——两页之间行集**增长**（重跑 extract
//!   新增 REQ / REQ-A 追加）时 offset ≤ total 仍成立，旧防线（越界拒）不触发，
//!   slice 会静默返回移位后的窗口（翻页跳行/重行）。总数绑定后：签发时 ≠
//!   当前 → E_UNPROCESSABLE（报双总数，从首页重取）——增长与缩水两个位移面
//!   都被拦住。残余面（同总数内容置换——删一行增一行）需域层版本绑定才能
//!   覆盖，已登记 #399 不掩盖；
//! - **版本字节**：v=2（v1→v2 = 增加 total 绑定，老游标显式失效）。不认识的
//!   版本（含未来版本）→ E_UNPROCESSABLE——跨版本游标语义不可知，fail-closed
//!   不猜测（线格式演进时 bump 版本 + 老游标显式失效，与 StoreSnapshot 的
//!   schema_version 同一口径）；
//! - **limit 显式拒**：1..=1000，缺省 100。越界/非整数 → E_UNPROCESSABLE
//!   而非 clamp——静默 clamp 会让调用方以为拿到了请求的页宽（对账面出错），
//!   这是 B5 期 get_compliance_matrix `.max(1)` clamp 习惯的显式修正；
//! - **offset 越界拒**：offset > 过滤后总数（两页之间底层列表缩水——行被删）
//!   → E_UNPROCESSABLE，不静默给空页（空页会被误读为「取完了」）。
//!   offset == total 合法（恰取到尾的空页，next_cursor=null）。
//!
//! **M2-b（search_evidence 双通道检索）落地时必须走本游标模块**——检索
//! hits 面同属大结果列表，不得自带第二套分页口径。
//!
//! 纯函数无状态：全部输入经参数、无全局态、无 IO——确定性由构造保证
//! （serde_json 对 struct 按声明序序列化，base64 STANDARD 确定，同输入
//! 同输出，跨进程可重放）。

use crate::error::ToolError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 游标线格式版本。bump = 线格式不兼容变更（老游标显式失效，fail-closed）。
/// v2（#399）：payload 增加 `total`（签发时过滤后总数）——行集增长位移守卫。
pub const CURSOR_VERSION: u8 = 2;

/// limit 下界（0 页宽无意义——显式拒）。
pub const LIMIT_MIN: u64 = 1;
/// limit 上界（单页 1000 行封顶——12k/10k 量级列表的吞吐与内存护栏）。
pub const LIMIT_MAX: u64 = 1000;
/// limit 缺省值（契约信封示例口径）。
pub const LIMIT_DEFAULT: u64 = 100;

// ---------------------------------------------------------------- 指纹面

/// FNV-1a 64 位偏移基（参照 public domain 参考实现——非密码学哈希，
/// 输入是调用方可见的过滤参数串，只求稳定与快速，不求抗碰撞攻击）。
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 位素数。
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 过滤器指纹：`[工具名, project_id, 过滤参数...]` → 16 hex（8 字节）。
///
/// - 分量间以 0x1f（单元分隔符）混入，防拼接歧义（`["ab","c"]` 与
///   `["a","bc"]` 必须异指纹）；
/// - 缺席分量（None）以 0x00 标记、在值分量以 0x01 引导——「无过滤」与
///   「过滤值为空串」是不同语义，指纹不得混同；
/// - 大端序输出 hex，与仓内 `fnv64:xxxx` / sha256-hex 指纹口径同型。
pub fn fingerprint(parts: &[Option<&str>]) -> String {
    let mut h = FNV_OFFSET_BASIS;
    for part in parts {
        match part {
            Some(s) => {
                h ^= 0x01; // 在值引导字节（与缺席标记区分）
                h = h.wrapping_mul(FNV_PRIME);
                for b in s.as_bytes() {
                    h ^= u64::from(*b);
                    h = h.wrapping_mul(FNV_PRIME);
                }
            }
            None => {
                h ^= 0x00; // 缺席标记（≠ 空串值）
                h = h.wrapping_mul(FNV_PRIME);
            }
        }
        h ^= 0x1f; // 单元分隔符
        h = h.wrapping_mul(FNV_PRIME);
    }
    let mut out = String::with_capacity(16);
    for b in h.to_be_bytes() {
        out.push(hex_digit(b >> 4));
        out.push(hex_digit(b & 0x0f));
    }
    out
}

fn hex_digit(n: u8) -> char {
    if n < 10 { (b'0' + n) as char } else { (b'a' + (n - 10)) as char }
}

// ---------------------------------------------------------------- 编解码面

/// 游标线格式（对调用方 opaque；本层 JSON 内嵌 base64）。
#[derive(Serialize, Deserialize)]
struct CursorPayload {
    /// 版本字节（不认识 → E_UNPROCESSABLE）。
    v: u8,
    /// 过滤后列表的起始偏移。
    offset: u64,
    /// 过滤器指纹（与本次调用的过滤参数绑定——错配即拒）。
    fp: String,
    /// 签发时过滤后总数（v2 新增——增长位移守卫锚点）。Option 仅服务于
    /// 反序列化宽容面：v1 老游标先被版本检查拦下；伪造 v2 缺 total →
    /// fail-closed 拒。encode 路径恒 Some。
    total: Option<u64>,
}

/// 解码结果（offset + 签发时总数——两者都是 slice 裁决的输入）。
#[derive(Debug, PartialEq, Eq)]
pub struct DecodedCursor {
    /// 起始偏移。
    pub offset: u64,
    /// 签发时过滤后总数（位移守卫锚点——与当前总数比对）。
    pub total: u64,
}

/// 编码游标（内部值：offset/total 由 slice 保证 ≤ 实际总数，fp 为内部计算值——
/// 纯标量 struct 的 serde_json 序列化不可失败）。
pub fn encode(offset: u64, fp: &str, total: u64) -> String {
    use base64::Engine as _;
    let payload = CursorPayload {
        v: CURSOR_VERSION,
        offset,
        fp: fp.to_string(),
        total: Some(total),
    };
    let json = serde_json::to_string(&payload)
        .expect("游标 payload 序列化不可失败（u8/u64/String/Option 标量字段）");
    base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
}

/// 解码游标（**不可信输入面**——任何一步坏 → E_UNPROCESSABLE，绝不 panic）。
///
/// 检查序：base64 → JSON → 版本字节 → total 绑定在场 → 指纹匹配。任一失败即
/// fail-closed；通过则返回 offset 与签发时总数（是否漂移/越界由 [`slice`] 结合
/// 当前总数裁决）。
pub fn decode(cursor: &str, expected_fp: &str) -> Result<DecodedCursor, ToolError> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cursor.as_bytes())
        .map_err(|e| ToolError::EUnprocessable {
            what: format!("cursor 非合法 base64（fail-closed 拒解码）: {e}"),
        })?;
    let payload: CursorPayload = serde_json::from_slice(&bytes)
        .map_err(|e| ToolError::EUnprocessable {
            what: format!("cursor 内嵌 JSON 损坏（fail-closed 拒解析）: {e}"),
        })?;
    if payload.v != CURSOR_VERSION {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "cursor 版本 v{} 不认识（当前支持 v{CURSOR_VERSION}——跨版本游标语义不可知，fail-closed 拒；请从首页重取）",
                payload.v
            ),
        });
    }
    // v2 游标必须携带 total 绑定（伪造缺字段 → fail-closed，不猜测为 0/无限）
    let total = payload.total.ok_or_else(|| ToolError::EUnprocessable {
        what: "cursor 缺 total 绑定（v2 线格式必需字段——fail-closed 拒，请从首页重取）".into(),
    })?;
    if payload.fp != expected_fp {
        return Err(ToolError::EUnprocessable {
            what: "cursor 与当前过滤器指纹不匹配（跨过滤器/跨工具复用游标——拒绝静默给错页数据；请用相同过滤参数从头取）".into(),
        });
    }
    Ok(DecodedCursor { offset: payload.offset, total })
}

// ---------------------------------------------------------------- 窗口解析面

/// 生效分页窗口（offset + 已校验 limit + 签发时总数锚点）。
#[derive(Debug)]
pub struct Window {
    /// 本次起始偏移（cursor 解码值 / page 映射值 / 首页 0）。
    pub offset: u64,
    /// 生效页宽（1..=1000，已显式校验）。
    pub limit: u64,
    /// 游标签发时的过滤后总数（None = 首页 / page 兼容路径——无位移守卫
    /// 锚点，只走 offset 越界检查；Some = cursor 路径，slice 与当前总数比对）。
    pub total_at_issue: Option<u64>,
}

/// 从工具入参解析分页窗口。
///
/// - `limit`：1..=1000，缺省 100，越界/非整数显式拒（不 clamp）；
/// - `cursor`：opaque 字符串，优先级最高（与 `page` 并存时 page 被忽略——
///   page 仅为旧调用兼容保留，映射 offset=(page-1)*limit 后走同一游标路径）；
/// - `page`：正整数（1 起；0 → 拒——B5 期 `.max(1)` clamp 习惯的显式修正），
///   乘法溢出 → 拒（fail-closed）。
pub fn resolve(args: &Value, tool: &str, fp: &str) -> Result<Window, ToolError> {
    // ---- limit（显式边界，不静默 clamp）
    let limit = match args.get("limit") {
        None | Some(Value::Null) => LIMIT_DEFAULT,
        Some(Value::Number(n)) => {
            let v = n.as_u64().ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{tool}: limit 须为 {LIMIT_MIN}..={LIMIT_MAX} 的整数"),
            })?;
            if !(LIMIT_MIN..=LIMIT_MAX).contains(&v) {
                return Err(ToolError::EUnprocessable {
                    what: format!(
                        "{tool}: limit={v} 越界（合法 {LIMIT_MIN}..={LIMIT_MAX}，缺省 {LIMIT_DEFAULT}——显式拒绝，不静默 clamp）"
                    ),
                });
            }
            v
        }
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{tool}: limit 须为 {LIMIT_MIN}..={LIMIT_MAX} 的整数"),
            });
        }
    };
    // ---- offset（cursor 优先；page 兼容映射；缺省首页 0）
    let (offset, total_at_issue) = match args.get("cursor") {
        Some(Value::String(c)) => {
            let d = decode(c, fp)?;
            (d.offset, Some(d.total))
        }
        None | Some(Value::Null) => match args.get("page") {
            None | Some(Value::Null) => (0, None),
            Some(Value::Number(n)) => {
                let page = n.as_u64().ok_or_else(|| ToolError::EUnprocessable {
                    what: format!("{tool}: page 须为正整数（1 起）"),
                })?;
                if page < 1 {
                    return Err(ToolError::EUnprocessable {
                        what: format!(
                            "{tool}: page={page} 越界（页号从 1 起——旧 .max(1) clamp 习惯已改显式拒）"
                        ),
                    });
                }
                let offset = page
                    .checked_sub(1)
                    .and_then(|zero_based| zero_based.checked_mul(limit))
                    .ok_or_else(|| ToolError::EUnprocessable {
                        what: format!("{tool}: page={page} 偏移计算溢出（fail-closed）"),
                    })?;
                (offset, None)
            }
            Some(_) => {
                return Err(ToolError::EUnprocessable {
                    what: format!("{tool}: page 须为正整数（1 起）"),
                });
            }
        },
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{tool}: cursor 须为字符串（opaque base64——由本工具 next_cursor 原样回传）"),
            });
        }
    };
    Ok(Window { offset, limit, total_at_issue })
}

// ---------------------------------------------------------------- 切片面

/// 切片结果（信封素材：本页行区间 + 两个游标字段）。
#[derive(Debug)]
pub struct PageSlice {
    /// 本页起始下标（含）。
    pub start: usize,
    /// 本页结束下标（不含）。
    pub end: usize,
    /// 本次生效游标（offset 的 opaque 回显——重放同页的锚点）。
    pub cursor: String,
    /// 下一页游标（None = 取完）。
    pub next_cursor: Option<String>,
}

/// 按窗口切过滤后列表（`total` = 过滤后总数——调用方先过滤后分页）。
///
/// 两道 fail-closed 门（先总数漂移、后 offset 越界——漂移门信息更全，
/// 缩水场景两门皆可触达时先报漂移）：
/// - **总数漂移拒**（C-14 / #399）：游标签发时总数 ≠ 当前总数（两页之间
///   行集增长或缩水——增长时 offset ≤ total 仍成立，位移窗口静默翻错页）
///   → E_UNPROCESSABLE（报双总数，从首页重取）；
/// - **offset 越界拒**：offset > 当前总数 → E_UNPROCESSABLE，不静默给空页
///   （空页会被误读为「取完了」）。offset == total 合法（恰取到尾的空页）。
pub fn slice(window: &Window, fp: &str, total: usize) -> Result<PageSlice, ToolError> {
    if let Some(prev) = window.total_at_issue {
        if prev != total as u64 {
            return Err(ToolError::EUnprocessable {
                what: format!(
                    "cursor 签发后底层列表已变化（签发时过滤后总数 {prev}，当前 {total}——增长/缩水位移风险，fail-closed 拒绝静默翻错页；请从首页重取）",
                ),
            });
        }
    }
    if window.offset > total as u64 {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "cursor offset={} 越界（当前过滤后总数 {total}——底层列表已变化，请从首页重取；fail-closed 不静默给空页）",
                window.offset
            ),
        });
    }
    let start = window.offset as usize;
    // limit ≤ 1000 已在 resolve 收口，saturating_add 仅作 32 位平台兜底
    let end = start.saturating_add(window.limit as usize).min(total);
    let next_cursor = if end < total {
        Some(encode(end as u64, fp, total as u64))
    } else {
        None
    };
    Ok(PageSlice {
        start,
        end,
        cursor: encode(window.offset, fp, total as u64),
        next_cursor,
    })
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn un(e: ToolError) -> String {
        match e {
            ToolError::EUnprocessable { what } => what,
            other => panic!("预期 E_UNPROCESSABLE，实际 {other:?}"),
        }
    }

    /// 指纹稳定性与区分度：同输入同输出；分量边界不混同。
    #[test]
    fn fingerprint_stable_and_unambiguous() {
        let a = fingerprint(&[Some("get_compliance_matrix"), Some("PRJ-0001"), None, None]);
        // 稳定（同输入两次全等——跨进程可重放的根基）
        assert_eq!(a, fingerprint(&[Some("get_compliance_matrix"), Some("PRJ-0001"), None, None]));
        // 拼接歧义防护
        assert_ne!(
            fingerprint(&[Some("ab"), Some("c")]),
            fingerprint(&[Some("a"), Some("bc")])
        );
        // 缺席 ≠ 空串值（None 与 Some("") 是不同过滤语义）
        assert_ne!(fingerprint(&[None]), fingerprint(&[Some("")]));
        // 任一过滤参数变化 → 指纹变化（跨过滤器互斥的根基）
        assert_ne!(a, fingerprint(&[Some("get_compliance_matrix"), Some("PRJ-0001"), Some("scoring"), None]));
        assert_ne!(a, fingerprint(&[Some("list_evidence"), Some("PRJ-0001"), None, None]));
        assert_ne!(a, fingerprint(&[Some("get_compliance_matrix"), Some("PRJ-0002"), None, None]));
        // 形态：16 位小写 hex
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    /// 编解码往返 + 全部坏输入面 fail-closed（不 panic）。
    #[test]
    fn encode_decode_roundtrip_and_fail_closed() {
        let fp = fingerprint(&[Some("t"), Some("PRJ-0001"), None]);
        let c = encode(120, &fp, 1_000);
        let d = decode(&c, &fp).unwrap();
        assert_eq!((d.offset, d.total), (120, 1_000), "往返携带 offset + 签发时总数");
        // 坏 base64 / 坏 JSON / 坏版本 / 错指纹 / 负 offset / 缺字段 → 全拒
        assert!(decode("!!!", &fp).is_err());
        assert!(decode(
            &base64::engine::general_purpose::STANDARD.encode(b"\x00\xff garbage"),
            &fp
        ).is_err());
        assert!(decode(&encode_for_test(9, 5, Some(7), &fp), &fp).is_err(), "版本 9 拒");
        assert!(decode(&c, "0000000000000000").is_err(), "指纹错配拒");
        assert!(decode(&encode_for_test(1, 5, Some(7), &fp), "wrong").is_err(), "指纹错配拒（版本 1 也在版本检查即拒）");
        assert!(decode(
            &base64::engine::general_purpose::STANDARD
                .encode(br#"{"v":2,"offset":-5,"fp":"x"}"#),
            &fp
        ).is_err(), "负 offset（u64 反序列化拒）");
        assert!(decode(
            &base64::engine::general_purpose::STANDARD
                .encode(br#"{"v":2,"offset":5}"#),
            &fp
        ).is_err(), "缺 fp 字段拒");
        assert!(decode("", &fp).is_err(), "空串拒");
        // 截断（丢 padding）拒
        assert!(decode(&c[..c.len() - 4], &fp).is_err());
    }

    /// v1 老游标（无 total 绑定）与伪造 v2 缺 total → 显式拒（#399 版本纪律）。
    #[test]
    fn v1_and_missing_total_cursors_rejected() {
        let fp = "fp";
        // v1 线格式（审计基线时代的真实形态：无 total 字段）→ 版本不认识拒
        let v1 = base64::engine::general_purpose::STANDARD
            .encode(br#"{"v":1,"offset":5,"fp":"fp"}"#);
        let err = decode(&v1, fp).unwrap_err();
        let msg = un(err);
        assert!(msg.contains("v1"), "v1 版本号进报文（可辨认的失效原因）: {msg}");
        // 伪造 v2 缺 total 绑定 → fail-closed 拒（不猜测为 0/无限）
        let v2_no_total = base64::engine::general_purpose::STANDARD
            .encode(br#"{"v":2,"offset":5,"fp":"fp"}"#);
        let err = decode(&v2_no_total, fp).unwrap_err();
        let msg = un(err);
        assert!(msg.contains("total"), "缺绑定报文指明 total: {msg}");
    }

    /// 测试专用锻造（绕过 encode 的版本常量——坏版本注入面）。
    fn encode_for_test(v: u8, offset: u64, total: Option<u64>, fp: &str) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_string(&CursorPayload { v, offset, fp: fp.to_string(), total }).unwrap())
    }

    /// resolve：limit 边界 / cursor 优先 / page 映射与显式拒。
    #[test]
    fn resolve_limit_cursor_page_semantics() {
        let fp = "fp";
        // 缺省 limit=100
        let w = resolve(&serde_json::json!({}), "t", fp).unwrap();
        assert_eq!((w.offset, w.limit), (0, 100));
        // 边界值合法
        for ok in [1u64, 1000] {
            assert_eq!(resolve(&serde_json::json!({ "limit": ok }), "t", fp).unwrap().limit, ok);
        }
        // 越界 / 非整数显式拒（不 clamp）
        for bad in [0u64, 1001, u64::MAX] {
            assert!(resolve(&serde_json::json!({ "limit": bad }), "t", fp).is_err(), "limit={bad} 拒");
        }
        for bad in [serde_json::json!("50"), serde_json::json!(1.5), serde_json::json!(-3)] {
            assert!(resolve(&serde_json::json!({ "limit": bad }), "t", fp).is_err(), "limit 非整数拒");
        }
        // null = 缺省（与 opt_u64 缺席口径一致）
        assert_eq!(resolve(&serde_json::json!({ "limit": null }), "t", fp).unwrap().limit, 100);
        // page 映射：offset=(page-1)*limit；page=1 首页
        let w = resolve(&serde_json::json!({ "page": 3, "limit": 50 }), "t", fp).unwrap();
        assert_eq!((w.offset, w.limit), (100, 50));
        // page=0 拒（显式，不 clamp）；page 非整数拒
        assert!(resolve(&serde_json::json!({ "page": 0 }), "t", fp).is_err());
        assert!(resolve(&serde_json::json!({ "page": "2" }), "t", fp).is_err());
        // page 溢出拒
        assert!(resolve(&serde_json::json!({ "page": u64::MAX, "limit": 1000 }), "t", fp).is_err());
        // cursor 优先于 page
        let c = encode(77, fp, 500);
        let w = resolve(&serde_json::json!({ "cursor": c, "page": 9, "limit": 10 }), "t", fp).unwrap();
        assert_eq!(w.offset, 77, "cursor 与 page 并存 → cursor 优先");
        assert_eq!(w.total_at_issue, Some(500), "cursor 路径携带签发时总数");
        // page 路径无位移守卫锚点（兼容面只走 offset 越界检查）
        let w = resolve(&serde_json::json!({ "page": 2, "limit": 10 }), "t", fp).unwrap();
        assert_eq!(w.total_at_issue, None, "page 路径无 total_at_issue");
        // cursor 非字符串拒
        assert!(resolve(&serde_json::json!({ "cursor": 42 }), "t", fp).is_err());
        // cursor 指纹错配拒
        assert!(resolve(&serde_json::json!({ "cursor": c }), "t", "other-fp").is_err());
    }

    /// slice：区间 / 尾页 / 恰尽 / 越界拒 / 空集。
    /// （Window 直构造 total_at_issue=None——纯 offset 语义面；位移守卫另测。）
    #[test]
    fn slice_windows_and_bounds() {
        let fp = "fp";
        // 中页
        let s = slice(&Window { offset: 100, limit: 100, total_at_issue: None }, fp, 12_000).unwrap();
        assert_eq!((s.start, s.end), (100, 200));
        assert_eq!(s.next_cursor.as_deref(), Some(encode(200, fp, 12_000).as_str()));
        assert_eq!(s.cursor, encode(100, fp, 12_000));
        // 尾页不整除
        let s = slice(&Window { offset: 11_950, limit: 100, total_at_issue: None }, fp, 12_000).unwrap();
        assert_eq!((s.start, s.end), (11_950, 12_000));
        assert!(s.next_cursor.is_none(), "尾页取完 next_cursor=null");
        // 恰尽（limit == 剩余）
        let s = slice(&Window { offset: 11_900, limit: 100, total_at_issue: None }, fp, 12_000).unwrap();
        assert_eq!(s.end, 12_000);
        assert!(s.next_cursor.is_none(), "limit>剩余 单页取完 next_cursor=null");
        // offset == total 合法（空尾页）
        let s = slice(&Window { offset: 12_000, limit: 100, total_at_issue: None }, fp, 12_000).unwrap();
        assert_eq!((s.start, s.end), (12_000, 12_000));
        assert!(s.next_cursor.is_none());
        // offset > total 拒（数据缩水 fail-closed）
        let err = slice(&Window { offset: 12_001, limit: 100, total_at_issue: None }, fp, 12_000).unwrap_err();
        assert!(un(err).contains("越界"), "越界错误须指明语义");
        // 空集：offset 0 合法
        let s = slice(&Window { offset: 0, limit: 100, total_at_issue: None }, fp, 0).unwrap();
        assert_eq!((s.start, s.end), (0, 0));
        assert!(s.next_cursor.is_none());
    }

    /// **C-14 位移守卫**：签发时总数 ≠ 当前总数（增长/缩水）→ 拒（报双总数）；
    /// 无变化连续翻页通过；next_cursor 携带当前总数（下一页守卫锚点）。
    #[test]
    fn slice_total_binding_rejects_drift() {
        let fp = "fp";
        // 增长位移（本单核心断言）：签发于 total=100，翻页时 total=105——
        // offset=100 ≤ 105 旧防线不触发，总数绑定必须拦下
        let w = Window { offset: 100, limit: 50, total_at_issue: Some(100) };
        let err = slice(&w, fp, 105).unwrap_err();
        let msg = un(err);
        assert!(msg.contains("100") && msg.contains("105"), "报文含双总数: {msg}");
        // 缩水：先撞漂移门（比 offset 越界更早、信息更全）
        let err = slice(&Window { offset: 10, limit: 50, total_at_issue: Some(100) }, fp, 60).unwrap_err();
        let msg = un(err);
        assert!(msg.contains("签发后底层列表已变化"), "缩水走漂移门: {msg}");
        // 无变化：连续翻页通过
        let s = slice(&Window { offset: 50, limit: 50, total_at_issue: Some(100) }, fp, 100).unwrap();
        assert_eq!((s.start, s.end), (50, 100));
        assert!(s.next_cursor.is_none(), "尾页取完");
        // next_cursor 携带当前总数（下一页守卫锚点）
        let s = slice(&Window { offset: 0, limit: 50, total_at_issue: None }, fp, 100).unwrap();
        let nc = s.next_cursor.unwrap();
        let d = decode(&nc, fp).unwrap();
        assert_eq!((d.offset, d.total), (50, 100), "next_cursor 锚定当前总数");
        // 增长后再用新首页游标正常翻页（拒绝面不是无差别拒绝）
        let s = slice(&Window { offset: 50, limit: 50, total_at_issue: Some(105) }, fp, 105).unwrap();
        assert_eq!((s.start, s.end), (50, 100), "总数匹配的游标照常翻页");
    }
}
