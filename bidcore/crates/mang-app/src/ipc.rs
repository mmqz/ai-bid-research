//! mangipc:// —— JS→Rust 命令通道（wry 0.46 导航拦截方案）。
//!
//! 为什么走导航不走 IPC handler：wry 0.46.3 webkitgtk 实现在 IPC 回调里
//! `Request::builder().uri(webview.uri())` —— 对空 authority 的 URI
//! panic（mdcore mdshell-native s5a-env Xvfb 实证，未修上游）。
//! 导航拦截通道（`location.assign` + navigation_handler 返回 false 取消）
//! 与该 parse 路径解耦，跨平台（WebView2/webkitgtk）行为一致。
//!
//! 协议形：
//! ```text
//! mangipc://<verb>/<percent-encoded-JSON-payload>
//!   verb   = 命令名（tree / open / save / snapshot / versions / read-version / rollback）
//!   payload= JSON 参数（各 verb 自定义 schema）
//! ```
//! 响应：Rust 侧 `evaluate_script("window.__mang.onMessage(<json>)")`
//! 回注（evaluate_script 无返回值——异步回调形是唯一通道）。
//!
//! 纯函数面（编码/解码/verb 校验）在无 GUI 环境全量可测。

use serde_json::Value;

/// URL scheme。
pub const SCHEME: &str = "mangipc";

/// 支持的命令集（白名单——未知 verb 一律 Err，防御面收敛）。
pub const VERBS: &[&str] = &[
    "tree",         // 请求文件树（响应 bootstrap 形：ScanResult JSON）
    "open",         // {path} → 打开文件（响应 {ok, path, content, ftype}）
    "save",         // {path, content} → 写盘（响应 {ok}）
    "snapshot",     // {path, message?} → AI 主动存版本（响应 {ok, snapshot}）
    "versions",     // {path} → 版本列表（响应 {ok, versions: [...]}）
    "read-version", // {path, snapshot} → 读某版本内容
    "rollback",     // {path, snapshot} → 回滚（写工作文件 + 存回滚前快照）
    "recent",       // 请求最近打开（响应 {ok, entries: [...]}）
];

/// 解析一条 mangipc:// URL → (verb, payload)。
///
/// 非法形（错 scheme / 未知 verb / 坏 percent 编码 / 坏 JSON）全 `Err`——
/// 上层忽略并继续（导航取消，页面无感）。
pub fn decode(url: &str) -> Result<(String, Value), String> {
    let rest = url
        .strip_prefix(&format!("{SCHEME}://"))
        .ok_or_else(|| format!("非 {SCHEME}:// scheme"))?;
    // verb 段 = 首个 '/' 前
    let (verb, payload_enc) = match rest.split_once('/') {
        Some(v) => v,
        None => (rest, ""),
    };
    if !VERBS.contains(&verb) {
        return Err(format!("未知命令: {verb}"));
    }
    if payload_enc.is_empty() {
        return Ok((verb.to_string(), Value::Null));
    }
    let payload_str = percent_decode(payload_enc);
    if payload_str.trim().is_empty() {
        return Ok((verb.to_string(), Value::Null));
    }
    let v: Value =
        serde_json::from_str(&payload_str).map_err(|e| format!("payload 非 JSON: {e}"))?;
    Ok((verb.to_string(), v))
}

/// 编码一条命令 URL（测试对称面 + Rust 侧自发自收场景）。
pub fn encode(verb: &str, payload: &Value) -> Result<String, String> {
    if !VERBS.contains(&verb) {
        return Err(format!("未知命令: {verb}"));
    }
    let body = if payload.is_null() {
        String::new()
    } else {
        percent_encode(&serde_json::to_string(payload).map_err(|e| e.to_string())?)
    };
    Ok(format!("{SCHEME}://{verb}/{}", body))
}

/// percent-encoding（RFC 3986 非保留字符外全编——保守面，解码端对称即可）。
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// percent-decoding（尽力而为：非法转义原样保留，不因敌意输入失败）。
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let take_hex = |idx: usize| -> Option<u8> {
            if idx >= bytes.len() {
                return None;
            }
            match bytes[idx] {
                b'0'..=b'9' => Some(bytes[idx] - b'0'),
                b'a'..=b'f' => Some(bytes[idx] - b'a' + 10),
                b'A'..=b'F' => Some(bytes[idx] - b'A' + 10),
                _ => None,
            }
        };
        if bytes[i] == b'%' {
            if let (Some(h), Some(l)) = (take_hex(i + 1), take_hex(i + 2)) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_verbs() {
        for verb in VERBS {
            let payload = json!({ "path": "项目A/报价 x.md", "n": 42 });
            let url = encode(verb, &payload).unwrap();
            assert!(url.starts_with("mangipc://"));
            let (v, p) = decode(&url).unwrap();
            assert_eq!(&v, verb);
            assert_eq!(p, payload, "{verb} 载荷往返");
        }
    }

    #[test]
    fn decode_null_and_empty_payload() {
        let (v, p) = decode("mangipc://tree").unwrap();
        assert_eq!(v, "tree");
        assert!(p.is_null());
        let (v, p) = decode("mangipc://tree/").unwrap();
        assert_eq!(v, "tree");
        assert!(p.is_null());
    }

    #[test]
    fn decode_rejects_unknown_verb_and_bad_payload() {
        assert!(decode("mangipc://rm-rf/{}").is_err());
        // 坏转义不 panic（尽力解码 → 残串非 JSON → 正常 Err——行为面钉定）
        assert!(decode("mangipc://open/%zz").is_err());
        assert!(decode("http://evil/").is_err());
        let bad = format!("mangipc://open/{}", percent_encode("not json"));
        assert!(decode(&bad).is_err());
    }

    #[test]
    fn unicode_payload() {
        let url = encode("snapshot", &json!({ "message": "第2章 技术方案完成" })).unwrap();
        let (v, p) = decode(&url).unwrap();
        assert_eq!(v, "snapshot");
        assert_eq!(p["message"], "第2章 技术方案完成");
    }

    #[test]
    fn percent_roundtrip_samples() {
        for s in ["plain", "a b/c", "中文%100%", "{\"k\":1}", "?q=&#"] {
            assert_eq!(percent_decode(&percent_encode(s)), s, "样本 {s:?}");
        }
    }
}
