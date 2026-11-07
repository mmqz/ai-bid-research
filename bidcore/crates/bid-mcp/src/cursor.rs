//! cursor.rs —— S-4 大结果分页游标核心（issue #11 批次 v3 / V3-5）。
//!
//! 功能定位：12,000 行矩阵 / 10,000 条证据不能一次全返回——所有列表类工具
//! （`get_compliance_matrix` / `list_evidence` / `list_tasks`，以及 M2-b 落地时
//! 的 `search_evidence`）统一走本模块的 opaque cursor 信封：
//! `{ rows|items, total, limit, cursor(本次生效), next_cursor: string|null }`。
//! `next_cursor=null` 即取完；调用方把 next_cursor 原样回传取下一页。
//!
//! 设计决策：
//! - **游标即偏移**：cursor = base64(JSON{v, offset, fp})——对调用方 opaque，
//!   对本层只是「过滤后列表的稳定偏移」。不做键集分页（keyset）是因为各列表
//!   的业务键形态各异（CMP 位置序 / EV id 字典序 / SEC target_id 字典序），
//!   而工具层已保证**行序确定性**（见各工具的分页序注释），偏移在确定序上
//!   即安全；
//! - **游标与过滤器绑定（fail-closed）**：fp = (工具名 + project_id + 过滤参数)
//!   的 FNV-1a 64 稳定哈希（16 hex）。用 A 过滤器拿的游标配 B 过滤器调用 →
//!   E_UNPROCESSABLE 拒绝——静默给错页数据比报错危险得多（borrow：mdcore
//!   corrupt-sidecar 哲学——损坏/错配输入响亮拒绝，不猜测加载）。工具名也入
//!   fp：不同工具的 offset 索引的是不同列表，跨工具复用同样拒；
//! - **游标与数据版本绑定（#406/C-14）**：载荷携 cv = 签发时 store 的
//!   content_version；解码时与当前值不等 → E_UNPROCESSABLE「请从首页重取」。
//!   旧只防缩水（offset > total）不防增长——两页之间列表增长且插在本页
//!   之前，同一 offset 指向不同的行（静默重复/跳行），翻页链照常走完、
//!   对账面无法发现。cv 绑定后增长/缩水/任何变更一网打尽（缩水方向的
//!   越界检查保留为冗余兑底）。代价是「无关变更也拒」（过严方向：
//!   多取一次首页 vs 静默错页——fail-closed 教义下取前者）；
//! - **版本字节**：v=2（#406/C-14：线格式增 cv 字段——bump 版本，v1 老游标
//!   显式失效，与 StoreSnapshot 的 schema_version 同一口径）。不认识的
//!   版本（含未来版本）→ E_UNPROCESSABLE——跨版本游标语义不可知，
//!   fail-closed 不猜测；
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
/// v2（#406/C-14）：载荷增 cv（content_version）字段；v1 老游标被版本门拒。
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
    /// #406/C-14：签发时 store 的 content_version（数据版本绑定——
    /// 翻页中途任何 store 变更（增长/缩水/换序）都使旧游标失效）。
    /// `Option` 仅为 **v1→v2 解析过渡**：v1 载钩无此字段（解析为 None，
    /// 由版本门先行拒绝——报因明确指向版本而非笼统「损坏」）；v2 载钩
    /// 缺此字段 = 损坏（显式拒，不走 cv=0 默认值伪装）。
    cv: Option<u64>,
}

/// 编码游标（内部值：offset 由 slice 保证 ≤ total，fp 为内部计算值，
/// cv 为调用方传入的当前版本——纯标量 struct 的 serde_json 序列化不可失败）。
pub fn encode(offset: u64, fp: &str, cv: u64) -> String {
    use base64::Engine as _;
    let payload = CursorPayload { v: CURSOR_VERSION, offset, fp: fp.to_string(), cv: Some(cv) };
    let json = serde_json::to_string(&payload)
        .expect("游标 payload 序列化不可失败（u8/u64/String 标量字段）");
    base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
}

/// 解码游标（**不可信输入面**——任何一步坏 → E_UNPROCESSABLE，绝不 panic）。
///
/// 检查序：base64 → JSON → 版本字节 → 指纹匹配 → **数据版本匹配**（#406/C-14：
/// cv ≠ 当前 content_version → 拒——两页之间 store 变更使 offset 语义漂移，
/// 静默给错页比要求重取危险得多）。任一失败即 fail-closed；通过则返回
/// offset（是否越界由 [`slice`] 结合当前总数裁决）。
pub fn decode(cursor: &str, expected_fp: &str, expected_cv: u64) -> Result<u64, ToolError> {
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
                "cursor 版本 v{} 不认识（当前支持 v{CURSOR_VERSION}——跨版本游标语义不可知，fail-closed 拒；线格式升级后请从首页重取）",
                payload.v
            ),
        });
    }
    if payload.fp != expected_fp {
        return Err(ToolError::EUnprocessable {
            what: "cursor 与当前过滤器指纹不匹配（跨过滤器/跨工具复用游标——拒绝静默给错页数据；请用相同过滤参数从头取）".into(),
        });
    }
    // v2 载钩 cv 必备（v1 无 cv 已被上方版本门拦下——此处 None 即损坏）
    let payload_cv = payload.cv.ok_or_else(|| ToolError::EUnprocessable {
        what: "cursor 载钩缺 cv 字段（v2 线格式必备——损坏拒，fail-closed）".into(),
    })?;
    if payload_cv != expected_cv {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "cursor 已过期：签发时数据版本 cv={payload_cv}，当前 cv={expected_cv}（翻页期间数据已变更——增长/缩水/换序都会使偏移漂移；请从首页重取，fail-closed 不静默跳行/重复）"
            ),
        });
    }
    Ok(payload.offset)
}

// ---------------------------------------------------------------- 窗口解析面

/// 生效分页窗口（offset + 已校验 limit）。
#[derive(Debug)]
pub struct Window {
    /// 本次起始偏移（cursor 解码值 / page 映射值 / 首页 0）。
    pub offset: u64,
    /// 生效页宽（1..=1000，已显式校验）。
    pub limit: u64,
}

/// 从工具入参解析分页窗口。
///
/// - `limit`：1..=1000，缺省 100，越界/非整数显式拒（不 clamp）；
/// - `cursor`：opaque 字符串，优先级最高（与 `page` 并存时 page 被忽略——
///   page 仅为旧调用兼容保留，映射 offset=(page-1)*limit 后走同一游标路径）；
/// - `cv`：当前 store 的 content_version（#406/C-14——与游标载荷比对，
///   翻页期间数据变更即拒）；
/// - `page`：正整数（1 起；0 → 拒——B5 期 `.max(1)` clamp 习惯的显式修正），
///   乘法溢出 → 拒（fail-closed）。
pub fn resolve(args: &Value, tool: &str, fp: &str, cv: u64) -> Result<Window, ToolError> {
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
    let offset = match args.get("cursor") {
        Some(Value::String(c)) => decode(c, fp, cv)?,
        None | Some(Value::Null) => match args.get("page") {
            None | Some(Value::Null) => 0,
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
                page.checked_sub(1)
                    .and_then(|zero_based| zero_based.checked_mul(limit))
                    .ok_or_else(|| ToolError::EUnprocessable {
                        what: format!("{tool}: page={page} 偏移计算溢出（fail-closed）"),
                    })?
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
    Ok(Window { offset, limit })
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
/// offset > total → E_UNPROCESSABLE（底层列表在两页之间缩水——fail-closed
/// 不静默给空页；#406/C-14 cv 绑定后此检查为冗余兑底，保留无成本）；
/// offset == total 合法（恰取到尾的空页）。
pub fn slice(window: &Window, fp: &str, cv: u64, total: usize) -> Result<PageSlice, ToolError> {
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
        Some(encode(end as u64, fp, cv))
    } else {
        None
    };
    Ok(PageSlice { start, end, cursor: encode(window.offset, fp, cv), next_cursor })
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
        let c = encode(120, &fp, 7);
        assert_eq!(decode(&c, &fp, 7).unwrap(), 120);
        // 坏 base64 / 坏 JSON / 坏版本 / 错指纹 / 负 offset / 缺字段 → 全拒
        assert!(decode("!!!", &fp, 7).is_err());
        assert!(decode(
            &base64::engine::general_purpose::STANDARD.encode(b"\x00\xff garbage"),
            &fp,
            7
        ).is_err());
        assert!(decode(&encode_for_test(9, 5, &fp, 7), &fp, 7).is_err(), "版本 9 拒");
        assert!(decode(&c, "0000000000000000", 7).is_err(), "指纹错配拒");
        assert!(decode(&encode_for_test(1, 5, &fp, 7), "wrong", 7).is_err(), "指纹错配拒");
        assert!(decode(
            &base64::engine::general_purpose::STANDARD
                .encode(br#"{"v":2,"offset":-5,"fp":"x","cv":7}"#),
            &fp,
            7
        ).is_err(), "负 offset（u64 反序列化拒）");
        assert!(decode(
            &base64::engine::general_purpose::STANDARD
                .encode(br#"{"v":2,"offset":5,"cv":7}"#),
            &fp,
            7
        ).is_err(), "缺 fp 字段拒");
        assert!(decode("", &fp, 7).is_err(), "空串拒");
        // 截断（丢 padding）拒
        assert!(decode(&c[..c.len() - 4], &fp, 7).is_err());
    }

    /// #406/C-14：数据版本绑定——cv 变化拒（增长位移防护的核心面）。
    #[test]
    fn cv_binding_rejects_on_any_data_change() {
        let fp = fingerprint(&[Some("get_compliance_matrix"), Some("PRJ-0001"), None, None]);
        // 页 1 在 cv=7 时签发
        let page1 = encode(0, &fp, 7);
        assert_eq!(decode(&page1, &fp, 7).unwrap(), 0, "同 cv 正常翻页");
        // 翻页中途 store 任何变更（content_version bump）→ 旧游标拒
        let err = decode(&page1, &fp, 8).unwrap_err();
        let msg = un(err);
        assert!(msg.contains("已过期"), "报因是数据漂移（非过滤器错配）：{msg}");
        assert!(msg.contains("cv=7") && msg.contains("cv=8"), "报因带版本号对比");
        assert!(msg.contains("首页"), "指引从首页重取");
        // 与过滤器错配报因可区分（调用方排障面）
        let fp_err = un(decode(&page1, "0000000000000000", 7).unwrap_err());
        assert!(fp_err.contains("过滤器"), "过滤器错配报因不同：{fp_err}");
        // v1 旧游标（无 cv 字段）→ 版本门拒（线格式升级显式失效）
        let v1 = base64::engine::general_purpose::STANDARD
            .encode(br#"{"v":1,"offset":100,"fp":"x"}"#);
        let v1_err = un(decode(&v1, "x", 7).unwrap_err());
        assert!(v1_err.contains("v1") || v1_err.contains("版本"), "v1 游标被版本门拒：{v1_err}");
        // v2 形状但缺 cv 字段 → JSON 解析拒（fail-closed）
        let no_cv = base64::engine::general_purpose::STANDARD
            .encode(br#"{"v":2,"offset":5,"fp":"x"}"#);
        assert!(decode(&no_cv, "x", 7).is_err(), "缺 cv 字段拒");
    }

    /// 测试专用锻造（绕过 encode 的版本常量——坏版本注入面）。
    fn encode_for_test(v: u8, offset: u64, fp: &str, cv: u64) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_string(&CursorPayload { v, offset, fp: fp.to_string(), cv: Some(cv) }).unwrap())
    }

    /// resolve：limit 边界 / cursor 优先 / page 映射与显式拒。
    #[test]
    fn resolve_limit_cursor_page_semantics() {
        let fp = "fp";
        // 缺省 limit=100
        let w = resolve(&serde_json::json!({}), "t", fp, 7).unwrap();
        assert_eq!((w.offset, w.limit), (0, 100));
        // 边界值合法
        for ok in [1u64, 1000] {
            assert_eq!(resolve(&serde_json::json!({ "limit": ok }), "t", fp, 7).unwrap().limit, ok);
        }
        // 越界 / 非整数显式拒（不 clamp）
        for bad in [0u64, 1001, u64::MAX] {
            assert!(resolve(&serde_json::json!({ "limit": bad }), "t", fp, 7).is_err(), "limit={bad} 拒");
        }
        for bad in [serde_json::json!("50"), serde_json::json!(1.5), serde_json::json!(-3)] {
            assert!(resolve(&serde_json::json!({ "limit": bad }), "t", fp, 7).is_err(), "limit 非整数拒");
        }
        // null = 缺省（与 opt_u64 缺席口径一致）
        assert_eq!(resolve(&serde_json::json!({ "limit": null }), "t", fp, 7).unwrap().limit, 100);
        // page 映射：offset=(page-1)*limit；page=1 首页
        let w = resolve(&serde_json::json!({ "page": 3, "limit": 50 }), "t", fp, 7).unwrap();
        assert_eq!((w.offset, w.limit), (100, 50));
        // page=0 拒（显式，不 clamp）；page 非整数拒
        assert!(resolve(&serde_json::json!({ "page": 0 }), "t", fp, 7).is_err());
        assert!(resolve(&serde_json::json!({ "page": "2" }), "t", fp, 7).is_err());
        // page 溢出拒
        assert!(resolve(&serde_json::json!({ "page": u64::MAX, "limit": 1000 }), "t", fp, 7).is_err());
        // cursor 优先于 page（cv 匹配时）
        let c = encode(77, fp, 7);
        let w = resolve(&serde_json::json!({ "cursor": c, "page": 9, "limit": 10 }), "t", fp, 7).unwrap();
        assert_eq!(w.offset, 77, "cursor 与 page 并存 → cursor 优先");
        // cursor 非字符串拒
        assert!(resolve(&serde_json::json!({ "cursor": 42 }), "t", fp, 7).is_err());
        // cursor 指纹错配拒
        assert!(resolve(&serde_json::json!({ "cursor": c }), "t", "other-fp", 7).is_err());
        // #406/C-14：cv 变化 → resolve 里的 decode 同样拒
        assert!(
            resolve(&serde_json::json!({ "cursor": c }), "t", fp, 8).is_err(),
            "翻页期间数据变更（cv 7→8）→ 拒"
        );
    }

    /// slice：区间 / 尾页 / 恰尽 / 越界拒 / 空集。
    #[test]
    fn slice_windows_and_bounds() {
        let fp = "fp";
        // 中页
        let s = slice(&Window { offset: 100, limit: 100 }, fp, 7, 12_000).unwrap();
        assert_eq!((s.start, s.end), (100, 200));
        assert_eq!(s.next_cursor.as_deref(), Some(encode(200, fp, 7).as_str()));
        assert_eq!(s.cursor, encode(100, fp, 7));
        // 尾页不整除
        let s = slice(&Window { offset: 11_950, limit: 100 }, fp, 7, 12_000).unwrap();
        assert_eq!((s.start, s.end), (11_950, 12_000));
        assert!(s.next_cursor.is_none(), "尾页取完 next_cursor=null");
        // 恰尽（limit == 剩余）
        let s = slice(&Window { offset: 11_900, limit: 100 }, fp, 7, 12_000).unwrap();
        assert_eq!(s.end, 12_000);
        assert!(s.next_cursor.is_none(), "limit>剩余 单页取完 next_cursor=null");
        // offset == total 合法（空尾页）
        let s = slice(&Window { offset: 12_000, limit: 100 }, fp, 7, 12_000).unwrap();
        assert_eq!((s.start, s.end), (12_000, 12_000));
        assert!(s.next_cursor.is_none());
        // offset > total 拒（数据缩水 fail-closed——cv 绑定后的冗余兑底）
        let err = slice(&Window { offset: 12_001, limit: 100 }, fp, 7, 12_000).unwrap_err();
        assert!(un(err).contains("越界"), "越界错误须指明语义");
        // 空集：offset 0 合法
        let s = slice(&Window { offset: 0, limit: 100 }, fp, 7, 0).unwrap();
        assert_eq!((s.start, s.end), (0, 0));
        assert!(s.next_cursor.is_none());
        // 翻页全链模拟（同 cv 三页连翻 + 中途变更拒）——#406/C-14 端到端口径
        let mut offset = 0u64;
        let mut pages = 0;
        loop {
            let w = Window { offset, limit: 500 };
            let s = slice(&w, fp, 7, 12_000).unwrap();
            pages += 1;
            match s.next_cursor {
                Some(nc) => offset = decode(&nc, fp, 7).unwrap(),
                None => break,
            }
        }
        assert_eq!(pages, 24, "12k 行 ÷ 500 页宽 = 24 页全链同 cv 翻完");
        // 中途数据变更（cv 7→8）→ 下一页游标拒
        let s1 = slice(&Window { offset: 0, limit: 500 }, fp, 7, 12_000).unwrap();
        let next = s1.next_cursor.unwrap();
        let err = decode(&next, fp, 8).unwrap_err();
        assert!(un(err).contains("已过期"), "增长位移防护：翻页中途变更 → 拒");
    }
}
