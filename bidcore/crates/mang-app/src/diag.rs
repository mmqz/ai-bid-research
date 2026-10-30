//! diag.rs —— 设置面板的真实状态源（issue #76 问题 2）。
//!
//! 四个面：
//!   * [`mcp_status`]：MCP 通道自检。bid-mcp 是 stdio 传输（AI 客户端拉起
//!     `bid-mcp-stdio` 子进程），本壳不监听端口——「地址/端口」不适用，
//!     如实显示通道形态 + 本壳内嵌引擎（直链 bidcore，方案 A）的自检结果。
//!   * [`llm_status`]：`.mang/llm.json` + 用量台账。**api_key 只出掩码，
//!     绝不回明文**（对面是 WebView，等同把 key 打进 console）。
//!   * [`llm_save`]：合并写 `.mang/llm.json`（省略字段 = 保持原值；
//!     `clearApiKey` = 移除 key）。写后 `resolve` 验证——有 key 无 model
//!     这类配置在保存时就拒，不留到生成时才炸。
//!   * [`fspec_status`]：`.mang/format-spec.json` 只读摘要（页面/字体/行距）。
//!
//! 纪律：状态命令**永不因「文件缺失」而失败**——缺失本身就是要显示的状态。

use serde_json::{json, Value};

use crate::ipc::{Args, IpcError};
use crate::paths::{self, Workspace};

/// `.mang/llm.json` 的文件面（与 bid-tools `LlmConfigFile` 同形；
/// 那边是私有序列化面，这里为了掩码/合并需要独立读原文）。
#[derive(Default, serde::Deserialize)]
struct LlmFile {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_tokens_cap: Option<u64>,
}

fn env_nonempty(k: &str) -> bool {
    std::env::var(k).map(|v| !v.trim().is_empty()).unwrap_or(false)
}

/// key 掩码（`sk-abcdef…` → `sk-***cdef`；过短全星号）。
fn mask_key(k: &str) -> String {
    let t = k.trim();
    if t.len() <= 8 {
        return "****".to_string();
    }
    format!("{}***{}", &t[..4.min(t.len())], &t[t.len() - 4..])
}

fn read_llm_file(ws: &Workspace) -> Result<LlmFile, IpcError> {
    let path = ws.sub(".mang").join("llm.json");
    if !path.is_file() {
        return Ok(LlmFile::default());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| IpcError::io(format!("读取 {} 失败：{e}", paths::display(&path))))?;
    serde_json::from_str(&text).map_err(|e| {
        IpcError::bad_json(format!(
            "{} 不是合法 JSON：{e}（LLM 配置错误要响亮）",
            paths::display(&path)
        ))
    })
}

/// mcp.status —— MCP 通道与引擎自检。
pub fn mcp_status() -> Result<Value, IpcError> {
    let schema = bid_mcp::tools_schema();
    let tools = schema
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let implemented = schema
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter(|t| {
                    t.get("implemented")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    // 分发自检：未知工具必须给出 E_NOT_FOUND JSON 帧——证明「同一套引擎代码
    // 在本进程里能应答」（与 bid-mcp-stdio 同源编译）。这不是探测外部进程
    // （stdio 服务由 AI 客户端拉起，本壳看不见它的生死），如实标注。
    let probe = bid_mcp::dispatch_json(r#"{"tool":"__mang_selftest__","args":{}}"#);
    let probe_ok = probe.contains("E_NOT_FOUND");
    Ok(json!({
        "channel": "stdio",
        "server": "bid-mcp-stdio（由 MCP 客户端拉起；本壳不监听端口）",
        "protocol": "JSONL over stdio",
        "tools": tools,
        "implemented": implemented,
        "ready": probe_ok && tools > 0,
        "engine": "本壳直链 bidcore（方案 A），与 bid-mcp 同源",
        "probe": if probe_ok { "dispatch 应答正常" } else { "dispatch 应答异常" },
    }))
}

/// llm.status —— 配置 + 用量 + 生效模式（key 只出掩码）。
pub fn llm_status(ws: &Workspace) -> Result<Value, IpcError> {
    let file = read_llm_file(ws)?;
    let path = ws.sub(".mang").join("llm.json");
    let usage_path = ws.sub(".mang").join("llm-usage.json");
    let usage: Value = std::fs::read_to_string(&usage_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({
            "calls": 0, "prompt_tokens": 0, "completion_tokens": 0,
            "total_tokens": 0, "last_at": 0
        }));
    // resolve 有效面（env 覆盖文件）——错误也是状态，不是失败
    let resolved = bid_tools::llm::LlmConfig::resolve(Some(ws.root_canonical()));
    let (effective, resolve_error) = match &resolved {
        Ok(Some(_)) => ("real", Value::Null),
        Ok(None) => ("mock（未配置 api_key，自动降级）", Value::Null),
        Err(e) => ("配置错误", json!(e)),
    };
    let has_env_key = env_nonempty(bid_tools::llm::LlmConfig::ENV_API_KEY);
    let has_key = has_env_key || file.api_key.as_deref().map(|k| !k.trim().is_empty()).unwrap_or(false);
    Ok(json!({
        "file": paths::display(&path),
        "file_present": path.is_file(),
        "api_base": file.api_base.clone().unwrap_or_default(),
        "model": file.model.clone().unwrap_or_default(),
        "timeout_secs": file.timeout_secs.unwrap_or(60),
        "temperature": file.temperature.unwrap_or(0.3),
        "max_tokens_cap": file.max_tokens_cap.unwrap_or(8192),
        "has_api_key": has_key,
        "api_key_hint": file.api_key.as_deref().map(mask_key).unwrap_or_default(),
        "key_from_env": has_env_key,
        "effective": effective,
        "resolve_error": resolve_error,
        "usage_file": paths::display(&usage_path),
        "usage": usage,
    }))
}

/// llm.save `{ apiBase?, apiKey?, model?, timeoutSecs?, temperature?, maxTokensCap?, clearApiKey? }`
///
/// 合并语义：省略 = 保持原值；`clearApiKey:true` = 移除 key。
/// 写后 resolve 验证（有 key 缺 model 在保存时就拒）。成功**不回显 key**。
pub fn llm_save(ws: &Workspace, a: &Args) -> Result<Value, IpcError> {
    let mut file = read_llm_file(ws)?;
    if a.bool("clearApiKey")?.unwrap_or(false) {
        file.api_key = None;
    }
    if let Some(s) = a.str("apiBase")? {
        file.api_base = Some(s.trim_end_matches('/').to_string());
    }
    if let Some(k) = a.str("apiKey")? {
        file.api_key = if k.trim().is_empty() { None } else { Some(k.to_string()) };
    }
    if let Some(m) = a.str("model")? {
        file.model = Some(m.trim().to_string());
    }
    if let Some(t) = a.u64("timeoutSecs")? {
        if t == 0 {
            return Err(IpcError::bad_args("timeoutSecs 须 > 0"));
        }
        file.timeout_secs = Some(t);
    }
    if let Some(v) = a.get("temperature") {
        let t = v.as_f64().ok_or_else(|| IpcError::bad_args("参数 temperature 必须是数字"))?;
        if !(0.0..=2.0).contains(&t) {
            return Err(IpcError::bad_args(format!("temperature 超界（0.0-2.0）：{t}")));
        }
        file.temperature = Some(t);
    }
    if let Some(c) = a.u64("maxTokensCap")? {
        if c < 256 {
            return Err(IpcError::bad_args(format!("maxTokensCap 过小（>=256）：{c}")));
        }
        file.max_tokens_cap = Some(c);
    }
    // 文件面只写已设置的字段（serde skip_serializing_if——引擎读侧全可选）
    #[derive(serde::Serialize)]
    struct Out<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        api_base: &'a Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        api_key: &'a Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: &'a Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_secs: &'a Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        temperature: &'a Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_tokens_cap: &'a Option<u64>,
    }
    let text = serde_json::to_string_pretty(&Out {
        api_base: &file.api_base,
        api_key: &file.api_key,
        model: &file.model,
        timeout_secs: &file.timeout_secs,
        temperature: &file.temperature,
        max_tokens_cap: &file.max_tokens_cap,
    })
    .map_err(|e| IpcError::io(format!("LLM 配置序列化失败：{e}")))?;
    let dir = ws.sub(".mang");
    std::fs::create_dir_all(&dir)
        .map_err(|e| IpcError::io(format!("创建 {} 失败：{e}", paths::display(&dir))))?;
    let path = dir.join("llm.json");
    // 旧文件面（写后验证失败要回滚——不留半成品配置）
    let backup = std::fs::read_to_string(&path).ok();
    std::fs::write(&path, &text)
        .map_err(|e| IpcError::io(format!("写入 {} 失败：{e}", paths::display(&path))))?;
    // resolve 验证（有 key 无 model → 回滚 + 拒，不留到生成时）
    if let Err(e) = bid_tools::llm::LlmConfig::resolve(Some(ws.root_canonical())) {
        match &backup {
            Some(old) => {
                let _ = std::fs::write(&path, old);
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        return Err(IpcError::bad_args(format!("保存被拒绝（配置无效）：{e}")));
    }
    let effective = match bid_tools::llm::LlmConfig::resolve(Some(ws.root_canonical())) {
        Ok(Some(_)) => "real",
        _ => "mock（未配置 api_key，自动降级）",
    };
    Ok(json!({
        "ok": true,
        "file": paths::display(&path),
        "effective": effective,
        "saved_key": file.api_key.is_some(),
    }))
}

/// fspec.status —— `.mang/format-spec.json` 只读摘要。
pub fn fspec_status(ws: &Workspace) -> Result<Value, IpcError> {
    let path = ws.sub(".mang").join("format-spec.json");
    let base = json!({
        "path": paths::display(&path),
        "hint": "由 MCP 管线（import_tender_files → extract）从招标文件推导落盘",
    });
    if !path.is_file() {
        return Ok(json!({ "present": false, "base": base }));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| IpcError::io(format!("读取 {} 失败：{e}", paths::display(&path))))?;
    match serde_json::from_str::<bid_tools::format_spec::FormatSpec>(&text) {
        Ok(spec) => Ok(json!({
            "present": true,
            "invalid": false,
            "bytes": text.len(),
            "page": {
                "size": spec.global.page.size,
                "margin_mm": {
                    "top": spec.global.page.margin.top,
                    "bottom": spec.global.page.margin.bottom,
                    "left": spec.global.page.margin.left,
                    "right": spec.global.page.margin.right,
                },
            },
            "font": {
                "body": spec.global.font.body,
                "size_pt": spec.global.font.size,
                "line_height": spec.global.font.line_height,
            },
            "per_section_overrides": spec.per_section.len(),
            "base": base,
        })),
        Err(e) => Ok(json!({
            "present": true,
            "invalid": true,
            "error": format!("{e}"),
            "base": base,
        })),
    }
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_ws(tag: &str) -> Workspace {
        let d = std::env::temp_dir().join(format!("mang-diag-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".mang")).unwrap();
        Workspace::new(&d).unwrap()
    }

    /// 从 JSON 字符串构造参数 map（测试便捷面；Args 在调用点构造，避免临时值释放）
    fn map_of(json: &str) -> serde_json::Map<String, Value> {
        serde_json::from_str::<Value>(json).unwrap().as_object().unwrap().clone()
    }

    #[test]
    fn mcp_status_is_stdio_and_ready() {
        let v = mcp_status().unwrap();
        assert_eq!(v["channel"], "stdio");
        assert_eq!(v["tools"], 13, "工具面 13 个（#68 阶段 2 后）");
        assert_eq!(v["implemented"], 11);
        assert_eq!(v["ready"], true, "未知工具探测应答 E_NOT_FOUND 即就绪");
    }

    #[test]
    fn llm_status_masks_key_and_reports_mock_when_absent() {
        let ws = tmp_ws("llm-mock");
        let v = llm_status(&ws).unwrap();
        assert_eq!(v["has_api_key"], false);
        assert_eq!(v["effective"], "mock（未配置 api_key，自动降级）");
        assert_eq!(v["file_present"], false);
        assert_eq!(v["usage"]["total_tokens"], 0);
    }

    #[test]
    fn llm_status_masks_key_when_present() {
        let ws = tmp_ws("llm-mask");
        std::fs::write(
            ws.sub(".mang").join("llm.json"),
            r#"{"api_key":"sk-0123456789abcdef","model":"deepseek-chat","api_base":"https://api.deepseek.com/v1"}"#,
        )
        .unwrap();
        let v = llm_status(&ws).unwrap();
        // 环境变量可能带 key（CI 不该带，但本地会话可能带）——只断言掩码形态
        if v["api_key_hint"].as_str().is_some_and(|h| !h.is_empty()) {
            let h = v["api_key_hint"].as_str().unwrap();
            assert!(h.contains("***"), "掩码必须含 ***：{h}");
            assert!(!h.contains("0123456789abcdef"), "明文绝不外泄");
        }
        assert_eq!(v["model"], "deepseek-chat");
    }

    #[test]
    fn llm_save_roundtrip_and_merge() {
        let ws = tmp_ws("llm-save");
        // 首存：只有 key + model
        let m1 = map_of(r#"{"apiKey":"sk-aaaaaaaaaaaaaaaa","model":"glm-4","apiBase":"https://open.bigmodel.cn/api/paas/v4"}"#);
        let a1 = Args::new(&m1);
        let v = llm_save(&ws, &a1).unwrap();
        assert_eq!(v["effective"], "real");
        // 合并：只改 model，key 应保留
        let m2 = map_of(r#"{"model":"glm-4-plus"}"#);
        let a2 = Args::new(&m2);
        let v2 = llm_save(&ws, &a2).unwrap();
        assert_eq!(v2["effective"], "real", "合并写不丢 key");
        let s = llm_status(&ws).unwrap();
        assert_eq!(s["model"], "glm-4-plus");
        assert_eq!(s["has_api_key"], true);
        // 清 key
        let m3 = map_of(r#"{"clearApiKey":true}"#);
        let a3 = Args::new(&m3);
        let v3 = llm_save(&ws, &a3).unwrap();
        assert_eq!(v3["effective"], "mock（未配置 api_key，自动降级）");
    }

    #[test]
    fn llm_save_rejects_key_without_model() {
        let ws = tmp_ws("llm-reject");
        let m = map_of(r#"{"apiKey":"sk-bbbbbbbbbbbbbbbb"}"#);
        let a = Args::new(&m);
        let e = llm_save(&ws, &a).unwrap_err();
        assert_eq!(e.code, "bad_args");
        assert!(e.message.contains("model"), "错误应指出缺 model：{}", e.message);
    }

    #[test]
    fn fspec_status_absent_vs_present() {
        let ws = tmp_ws("fspec");
        let v = fspec_status(&ws).unwrap();
        assert_eq!(v["present"], false);
        std::fs::write(
            ws.sub(".mang").join("format-spec.json"),
            r#"{"global":{"page":{"size":"A4","margin":{"top":25,"bottom":25,"left":28,"right":26}},"font":{"body":"仿宋_GB2312","size":12,"lineHeight":1.5},"header":{"enabled":false,"content":""},"footer":{"enabled":true,"content":"第 {page} 页 / 共 {pages} 页"}},"per_section":{"技术方案":{"font":{"body":"黑体","size":14,"lineHeight":1.5}}}}"#,
        )
        .unwrap();
        let v2 = fspec_status(&ws).unwrap();
        assert_eq!(v2["present"], true);
        assert_eq!(v2["invalid"], false);
        assert_eq!(v2["page"]["size"], "A4");
        assert_eq!(v2["font"]["body"], "仿宋_GB2312");
        assert_eq!(v2["font"]["line_height"], 1.5);
        assert_eq!(v2["per_section_overrides"], 1);
        // 坏 JSON：present + invalid（不是失败）
        std::fs::write(ws.sub(".mang").join("format-spec.json"), "{oops").unwrap();
        let v3 = fspec_status(&ws).unwrap();
        assert_eq!(v3["present"], true);
        assert_eq!(v3["invalid"], true);
    }
}
