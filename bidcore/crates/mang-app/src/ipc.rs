//! IPC 协议 v1 —— 请求/响应/事件/大响应落盘。
//!
//! 契约见 docs/mang/IPC-CONTRACT.md。要点：
//!   * 请求：`{"id":"<uuid>","cmd":"fs.read","path":"a.md"}`（UI 侧 JSON.stringify 后 postMessage）
//!   * 响应：`{"id", "ok":true, "result":{...}}` 或 `{"id", "ok":false, "code", "message"}`
//!   * 事件：`{"ev":"drop","paths":[...]}`（无 id）
//!   * 大响应（JSON > 1.5MB）：写 `%TEMP%\\mang-ipc\\<id>.json`，
//!     响应体变成 `{"id","ok":true,"result":{"file":"<id>.json"}}`，UI fetch
//!     `mang://localhost/__ipc/<id>.json` 取回，**服务一次即删除**。

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::paths::{self, Workspace};

/// 内联进注入脚本的 JSON 上限（超过就落盘）
pub const INLINE_LIMIT: usize = 1_500_000;

/// typed error：`{"ok":false,"code","message"}`
#[derive(Debug, Clone)]
pub struct IpcError {
    pub code: &'static str,
    pub message: String,
}

impl IpcError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
    #[allow(dead_code)]
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new("not_found", m)
    }
    pub fn denied(m: impl Into<String>) -> Self {
        Self::new("denied", m)
    }
    #[allow(dead_code)]
    pub fn outside(m: impl Into<String>) -> Self {
        Self::new("outside_workspace", m)
    }
    pub fn too_large(m: impl Into<String>) -> Self {
        Self::new("too_large", m)
    }
    pub fn io(m: impl Into<String>) -> Self {
        Self::new("io", m)
    }
    pub fn unknown_cmd(m: impl Into<String>) -> Self {
        Self::new("unknown_cmd", m)
    }
    pub fn bad_json(m: impl Into<String>) -> Self {
        Self::new("bad_json", m)
    }
    pub fn bad_args(m: impl Into<String>) -> Self {
        Self::new("bad_args", m)
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/** 从请求里取参数的小工具（类型不对就 bad_args，绝不 panic） */
pub struct Args<'a> {
    pub map: &'a Map<String, Value>,
}

impl<'a> Args<'a> {
    pub fn new(map: &'a Map<String, Value>) -> Self {
        Self { map }
    }
    pub fn get(&self, k: &str) -> Option<&'a Value> {
        self.map.get(k).filter(|v| !v.is_null())
    }
    pub fn str(&self, k: &str) -> Result<Option<&'a str>, IpcError> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(IpcError::bad_args(format!("参数 {k} 必须是字符串"))),
        }
    }
    pub fn req_str(&self, k: &str) -> Result<&'a str, IpcError> {
        self.str(k)?
            .ok_or_else(|| IpcError::bad_args(format!("缺少参数 {k}")))
    }
    pub fn path(&self, k: &str) -> Result<&'a str, IpcError> {
        let s = self.req_str(k)?;
        if s.is_empty() {
            return Err(IpcError::bad_args(format!("参数 {k} 不能为空")));
        }
        Ok(s)
    }
    pub fn opt_path(&self, k: &str) -> Result<Option<&'a str>, IpcError> {
        Ok(self.str(k)?.filter(|s| !s.is_empty()))
    }
    pub fn u64(&self, k: &str) -> Result<Option<u64>, IpcError> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::Number(n)) => n
                .as_u64()
                .map(Some)
                .ok_or_else(|| IpcError::bad_args(format!("参数 {k} 必须是非负整数"))),
            Some(_) => Err(IpcError::bad_args(format!("参数 {k} 必须是数字"))),
        }
    }
    pub fn bool(&self, k: &str) -> Result<Option<bool>, IpcError> {
        match self.get(k) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(IpcError::bad_args(format!("参数 {k} 必须是布尔"))),
        }
    }
}

/// 已解析的请求
#[derive(Debug, Clone)]
pub struct Request {
    pub id: Value,
    pub cmd: String,
    pub body: Map<String, Value>,
}

/// 解析一行 IPC。JSON 坏 → bad_json（id 为 null，仍会回一条错误帧）。
pub fn parse(body: &str) -> Result<Request, IpcError> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| IpcError::bad_json(format!("JSON 解析失败：{e}")))?;
    let obj = v
        .as_object()
        .ok_or_else(|| IpcError::bad_json("请求必须是 JSON 对象"))?;
    let id = obj.get("id").cloned().unwrap_or(Value::Null);
    // 老协议（handoff 文档里的 {cmd:"load_state"}）没有 id，也照常处理
    let cmd = match obj.get("cmd") {
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(IpcError::bad_args("cmd 必须是字符串")),
        None => return Err(IpcError::bad_args("缺少 cmd")),
    };
    Ok(Request { id, cmd, body: obj.clone() })
}

/// 响应帧构造
/// 成功帧。**result 会同时出现在 result 与 data 两个键上**：
///   * UI 的 JS 读 ev.result（app.js:290-315）
///   * 早先的脚本/探针读 data
/// 多给一份只是几个字节，省掉一整类"字段名对不上"的往返。
pub fn ok_frame(id: &Value, result: Value) -> Value {
    json!({ "id": id, "ok": true, "result": result, "data": result })
}

/// 错误帧。同时给 **顶层** code/message 和 **嵌套** error{code,message}：
/// UI 侧两种读法都有（app.js 读 error.code，早期脚本读顶层 code），多给一份无害。
pub fn err_frame(id: &Value, e: &IpcError) -> Value {
    json!({
        "id": id,
        "ok": false,
        "code": e.code,
        "message": e.message,
        "error": { "code": e.code, "message": e.message },
    })
}

/// 事件帧（无 id）
#[allow(dead_code)]
pub fn event_frame(ev: &str, extra: Value) -> Value {
    let mut obj = match extra {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    obj.insert("ev".to_string(), Value::String(ev.to_string()));
    Value::Object(obj)
}

// ---------------------------------------------------------------- 注入脚本

/// JSON → 可以直接塞进 <script> 的字符串（转义 </script>、U+2028/2029）
pub fn js_json(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_else(|_| "null".to_string());
    s.replace("</", "<\\/")
        .replace("\u{2028}", "\\u2028")
        .replace("\u{2029}", "\\u2029")
}

/// 响应注入：`try{window.__ipcResolve(id, resp, ok)}catch(e){}`
pub fn resolve_script(id: &Value, resp: &Value, ok: bool) -> String {
    format!(
        "try{{window.__ipcResolve&&window.__ipcResolve({},{},{});}}catch(e){{}}",
        js_json(id),
        js_json(resp),
        ok
    )
}

/// 事件注入：`try{window.__ipcEvent(ev)}catch(e){}`
pub fn event_script(ev: &Value) -> String {
    format!(
        "try{{window.__ipcEvent&&window.__ipcEvent({});}}catch(e){{}}",
        js_json(ev)
    )
}

// ---------------------------------------------------------------- 大响应落盘

/// `%TEMP%\\mang-ipc`，确保存在
pub fn pending_dir() -> PathBuf {
    let d = std::env::temp_dir().join("mang-ipc");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// `%TEMP%\\mang-preview`
pub fn preview_dir() -> PathBuf {
    let d = std::env::temp_dir().join("mang-preview");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// id → 安全的文件名片段（uuid 一般已经安全，这里兜底）
pub fn safe_file_id(id: &Value) -> String {
    let raw = match id {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => "no-id".to_string(),
    };
    let mut out: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    out.truncate(120);
    if out.is_empty() {
        out = "no-id".to_string();
    }
    out
}

/// 把大响应写到 `%TEMP%\\mang-ipc\\<id>.json`，返回替换后的响应帧。
/// 写盘失败就退回内联（宁可慢也不要丢数据）。
pub fn spill_if_large(id: &Value, resp: Value) -> Value {
    let size = serde_json::to_vec(&resp).map(|v| v.len()).unwrap_or(0);
    if size <= INLINE_LIMIT {
        return resp;
    }
    let name = format!("{}.json", safe_file_id(id));
    let path = pending_dir().join(&name);
    let written = serde_json::to_vec(&resp).map_err(|e| e.to_string()).and_then(|buf| {
        std::fs::write(&path, &buf).map_err(|e| e.to_string())
    });
    match written {
        Ok(()) => json!({
            "id": id,
            "ok": true,
            "result": { "file": name, "bytes": size, "inline": false },
        }),
        Err(e) => {
            eprintln!("[mang-ipc] 大响应落盘失败（退回内联 {size}B）：{e}");
            resp
        }
    }
}

/// `mang://localhost/__ipc/<name>` 的读取：读一次后删除
pub fn take_pending(name: &str) -> Option<Vec<u8>> {
    // 只允许纯文件名，杜绝 ../ 读取任意文件
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    if !name.ends_with(".json") {
        return None;
    }
    let path = pending_dir().join(name);
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(bytes)
}

/// 启动时清理上次遗留
pub fn purge_pending() {
    let dir = pending_dir();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|x| x.to_str()) == Some("json") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

// ---------------------------------------------------------------- 分发

/// 执行上下文：工作区 + UI 目录 + 版本信息
pub struct Ctx {
    pub workspace: Workspace,
    pub ui_dir: PathBuf,
}

impl Ctx {
    pub fn new(workspace: Workspace, ui_dir: PathBuf) -> Self {
        Self { workspace, ui_dir }
    }

    /// 执行一条请求，返回响应帧（永不 panic、永远给 id 一个答复）
    pub fn dispatch(&self, req: &Request) -> Value {
        let res = self.exec(&req.cmd, Args::new(&req.body));
        match res {
            Ok(v) => ok_frame(&req.id, v),
            Err(e) => err_frame(&req.id, &e),
        }
    }

    fn exec(&self, cmd: &str, a: Args<'_>) -> Result<Value, IpcError> {
        match cmd {
            "ping" => Ok(json!({ "pong": true, "t": now_ms() })),

            "app.info" => Ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "workspace": self.workspace.root_display(),
                "ui_dir": paths::display(&self.ui_dir),
                "exe": std::env::current_exe().map(|p| paths::display(&p)).unwrap_or_default(),
                "pid": std::process::id(),
                "platform": std::env::consts::OS,
                "protocol": 1,
            })),

            "fs.tree" => crate::fsapi::tree(&self.workspace, a),

            "fs.read" => crate::fsapi::read(&self.workspace, a),

            "fs.readBytes" => crate::fsapi::read_bytes(&self.workspace, a),

            "fs.writeBytes" => crate::fsapi::write_bytes(&self.workspace, a),

            "fs.stat" => crate::fsapi::stat(&self.workspace, a),

            "fs.mkdir" => crate::fsapi::mkdir(&self.workspace, a),

            "explorer.reveal" => crate::shell::reveal(&self.workspace, a),

            "recent.load" => crate::recent::load(),

            "recent.save" => crate::recent::save(a),

            "history.list" => crate::recent::history_list(&self.workspace, a),

            // git 提交记录（只读）：改动/历史面板的数据源
            "git.log" => {
                let rel = a.opt_path("path")?;
                let limit = a.u64("limit")?.unwrap_or(80) as usize;
                crate::gitlog::log(&self.workspace, rel, limit)
            }

            "git.show" => {
                let hash = a.req_str("hash")?;
                let rel = a.opt_path("path")?;
                crate::gitlog::show(&self.workspace, hash, rel)
            }

            "git.status" => {
                let rel = a.opt_path("path")?;
                crate::gitlog::status(&self.workspace, rel)
            }

            "preview.write" => crate::fsapi::preview_write(a),

            // 绝对路径（拖拽/对话框/最近打开）→ 归一成 {path, rel, kind, size, mtimeMs, insideWorkspace}
            "file.fromPath" => crate::fsapi::from_path(&self.workspace, a),

            // 对话框必须在主线程同步跑（Win32 模态），这里只做参数校验占位，
            // 真正的执行在 main.rs 的事件循环里（main_thread_cmd）。
            "dialog.openFile" | "dialog.openFolder" => Err(IpcError::new(
                "io",
                "对话框需要在主线程执行（见 main_thread_cmd）",
            )),

            other => Err(IpcError::unknown_cmd(format!("未知命令：{other}"))),
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 文件 mtime → 毫秒（文件系统读不到就 0）
pub fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 供 main.rs 复用：路径 + 元信息
#[allow(dead_code)]
pub fn file_meta(p: &Path) -> Option<std::fs::Metadata> {
    std::fs::metadata(p).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_errors() {
        let r = parse(r#"{"id":"a1","cmd":"ping"}"#).unwrap();
        assert_eq!(r.cmd, "ping");
        assert_eq!(r.id, json!("a1"));
        let e = parse("not json").unwrap_err();
        assert_eq!(e.code, "bad_json");
        let e = parse(r#"{"id":1}"#).unwrap_err();
        assert_eq!(e.code, "bad_args");
        let r = parse(r#"{"cmd":"load_state"}"#).unwrap();
        assert_eq!(r.id, Value::Null);
    }

    #[test]
    fn scripts_are_guarded() {
        let s = resolve_script(&json!("x"), &json!({"a": "</script>"}), true);
        assert!(s.starts_with("try{window.__ipcResolve&&window.__ipcResolve("));
        assert!(s.ends_with("}catch(e){}"));
        assert!(!s.contains("</script>"));
        let e = event_script(&json!({"ev":"drop"}));
        assert!(e.contains("window.__ipcEvent"));
    }

    #[test]
    fn spill_and_take() {
        let id = json!("test-spill-xyz");
        let big = json!({ "id": id, "ok": true, "result": { "text": "x".repeat(INLINE_LIMIT + 10) } });
        let out = spill_if_large(&id, big);
        assert_eq!(out["result"]["file"], json!("test-spill-xyz.json"));
        let bytes = take_pending("test-spill-xyz.json").expect("pending file");
        assert!(bytes.len() > INLINE_LIMIT);
        // 读一次就删
        assert!(take_pending("test-spill-xyz.json").is_none());
        // 目录穿越被拒
        assert!(take_pending("../etc/passwd").is_none());
        assert!(take_pending("C:/windows/win.ini").is_none());
    }

    #[test]
    fn small_stays_inline() {
        let id = json!("s1");
        let small = ok_frame(&id, json!({"pong":true}));
        let out = spill_if_large(&id, small.clone());
        assert_eq!(out, small);
    }
}