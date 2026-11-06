//! MCP 协议层（issue #137）：JSON-RPC 2.0 + `initialize` / `tools/list` / `tools/call`。
//!
//! 此前 `bid-mcp-stdio` 是「逐行裸 JSON」（`{"tool":..,"args":..}` 直进
//! [`crate::dispatch_json`]）——标准 MCP 客户端（官方 Python/TypeScript SDK、
//! Claude Desktop）发的却是 JSON-RPC 2.0 信封，一个方法都答不上来。本模块
//! 把协议层补齐，**不重写工具面**：
//!
//! - `initialize`   → protocolVersion 协商（支持集内回显，集外回最新）+
//!                    capabilities + serverInfo（握手位推进）；
//! - `tools/list`   → 复用 [`crate::schema::tools_schema`]（#137 起 `inputSchema`
//!                    补齐为合法 JSON Schema——此前只有 name/description/implemented）；
//! - `tools/call`   → 参数形状校验（`{name, arguments}`）后包装既有
//!                    [`crate::dispatch`] / [`crate::dispatch_with`]，工具语义零变化。
//!
//! 传输分帧 = MCP stdio：**换行分隔的 JSON-RPC 2.0**（与旧 bin 的逐行循环同
//! 构，一帧一行）；HTTP / wasm 未来实现同一会话面即可。
//!
//! 错误映射（契约 §0 错误码 × MCP 双通道）：
//! - **工具执行错误**（ToolError：E_STATE / E_NOTFOUND / E_UNPROCESSABLE /
//!   E_CONFLICT / E_PARTIAL / UNIMPLEMENTED_INTERNAL）→ result 通道
//!   `isError:true` + `content[0].text` 携带契约 §0 包装 JSON（`error.code`
//!   字面量原样透传，调用方按 code 分支——不解析 message 的既有纪律不变）。
//!   这是 MCP 规范对工具执行错误的口径：错误发生在工具内部，LLM 宿主应看到
//!   并自行重试/改参。
//! - **协议层错误** → JSON-RPC error 通道：解析失败 -32700、非法请求 -32600、
//!   未知方法 -32601、参数形状非法（tools/call 缺 name / arguments 非对象 /
//!   未知工具名）-32602、内部错误 -32603、未握手先调用 -32002。
//!
//! 旧路径（`dispatch_json` / `bid-mcp-stdio`）一字未动——现有 e2e 与桌面壳
//! 接线面不变；新 bin `bid-mcp-server`（`src/bin/server.rs`）走本模块。

use serde_json::{json, Value};

use crate::schema::{ToolCall, ToolReply};

/// 本实现支持的 MCP 协议版本（客户端请求版本在其中 → 原样回显）。
///
/// 四版在「initialize / tools/list / tools/call + stdio 换行分帧」这个子集上
/// 线格式完全一致（差异面——resources/prompts/tasks 等——本服务未实现），
/// 故全量声明；官方 SDK 的 `SUPPORTED_PROTOCOL_VERSIONS` 同集。
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] =
    ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// 协商不匹配（客户端要的版本不认识）时的回退版本 = 本实现认识的最新版。
/// MCP 规范允许服务端回任意己方支持版本，由客户端决定去留。
pub const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

/// `serverInfo.name`（MCP 客户端的展示与日志面——crate 名）。
pub const SERVER_NAME: &str = "bid-mcp";

// ---------------------------------------------------------------- JSON-RPC 错误码

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// MCP 规范保留码：握手（initialize）完成前的请求一律拒收。
pub const SERVER_NOT_INITIALIZED: i64 = -32002;

/// initialize 应答的 `instructions`（给 LLM 宿主的操作指引，可选字段）。
const INSTRUCTIONS: &str = "标书全链工具面：建项目 → 导入招标文件 → 抽条款 → \
     建纲/矩阵 → 登记/绑定证据 → 起草 → 校验 → 导出（外加评分试算与只读诊断）。\
     工具语义与错误码以 docs/bid-mcp-contract.md 为准；每次调用的 \
     content[0].text 是契约 §0 包装 JSON（{ok, tool, data|error}），\
     error.code 为稳定字面量（E_STATE/E_NOTFOUND/E_UNPROCESSABLE/...），\
     请按 code 分支而非解析 message。断点恢复先调 get_bid_project_state。";

/// MCP 会话面（单连接一份；stdio 长进程在行与行之间保持握手位）。
///
/// 传输层只做「取一行 → [`McpSession::handle_line`] → 回一行（None 不回）」；
/// 工具语义全部经注入的 dispatch 闭包（生产 = [`crate::dispatch`]（全局服务
/// 态），测试 = `dispatch_with(&mut svc, ..)` 注入独立实例——与旧路径同一
/// 可测性纪律）。
#[derive(Debug, Default)]
pub struct McpSession {
    /// initialize 握手是否已完成（未完成时除 initialize 外的请求 → -32002）。
    initialized: bool,
}

impl McpSession {
    #[must_use]
    pub fn new() -> Self {
        Self { initialized: false }
    }

    /// 握手位（测试与诊断面）。
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// 处理一行入站消息，返回应答行（`None` = 无需应答：通知一律静默）。
    ///
    /// 单行坏消息只污染它自己的应答——传输面存活优先，永不 panic。
    /// 序列化失败（理论不可达）→ -32603 兜底。
    pub fn handle_line<F>(&mut self, line: &str, dispatch: F) -> Option<String>
    where
        F: FnMut(ToolCall) -> ToolReply,
    {
        match serde_json::from_str::<Value>(line) {
            Ok(msg) => self.handle_message(msg, dispatch),
            Err(e) => Some(error_response(
                Value::Null,
                PARSE_ERROR,
                format!("Parse error: {e}"),
                None,
            )),
        }
    }

    fn handle_message<F>(&mut self, msg: Value, dispatch: F) -> Option<String>
    where
        F: FnMut(ToolCall) -> ToolReply,
    {
        let Some(obj) = msg.as_object() else {
            return Some(error_response(
                Value::Null,
                INVALID_REQUEST,
                "Invalid Request: 消息必须是 JSON-RPC 2.0 单对象（batch 已废弃，逐条发送）".into(),
                None,
            ));
        };
        // jsonrpc 字段：必须在场且为 "2.0"（MCP 线格式硬性要求）
        match obj.get("jsonrpc") {
            Some(Value::String(s)) if s == "2.0" => {}
            _ => {
                return Some(error_response(
                    echo_id(obj),
                    INVALID_REQUEST,
                    r#"Invalid Request: 缺 "jsonrpc":"2.0"（MCP 线格式）"#.into(),
                    None,
                ));
            }
        }
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return Some(error_response(
                echo_id(obj),
                INVALID_REQUEST,
                "Invalid Request: 缺 method 字段".into(),
                None,
            ));
        };
        // id：缺席 = 通知（JSON-RPC 2.0 §Notification——服务端不得应答，
        // 含未知通知与错误通知）；在场须为 string | number | null。
        let id = match obj.get("id") {
            None => {
                // notifications/initialized 是握手收尾的预期通知（本实现握手位
                // 在 initialize 应答时已推进，此处无需动作）；cancelled 等
                // 其它通知静默吞——通知没有应答通道，拒绝也不该出声。
                return None;
            }
            Some(v) if matches!(v, Value::Number(_) | Value::String(_) | Value::Null) => {
                v.clone()
            }
            Some(_) => {
                return Some(error_response(
                    Value::Null,
                    INVALID_REQUEST,
                    "Invalid Request: id 须为 string | number | null".into(),
                    None,
                ));
            }
        };
        // 握手序：initialize 之前只受理 initialize（通知已在上方放行）
        if !self.initialized && method != "initialize" {
            return Some(error_response(
                id,
                SERVER_NOT_INITIALIZED,
                "Server not initialized: 先发 initialize 请求（MCP 握手序）".into(),
                None,
            ));
        }
        match method {
            "initialize" => {
                self.initialized = true;
                Some(result_response(id, initialize_result(obj.get("params"))))
            }
            "ping" => Some(result_response(id, json!({}))),
            "tools/list" => Some(result_response(id, tools_list_result())),
            "tools/call" => self.tools_call(obj.get("params"), id, dispatch),
            other => Some(error_response(
                id,
                METHOD_NOT_FOUND,
                format!(
                    "Method not found: {other}（本服务只实现 initialize / ping / \
                     tools.list / tools.call——resources/prompts 未声明）"
                ),
                None,
            )),
        }
    }

    /// `tools/call`：参数形状校验（协议层）→ dispatch（工具层）→ 双通道映射。
    fn tools_call<F>(&mut self, params: Option<&Value>, id: Value, mut dispatch: F) -> Option<String>
    where
        F: FnMut(ToolCall) -> ToolReply,
    {
        let Some(params) = params else {
            return Some(error_response(
                id,
                INVALID_PARAMS,
                "Invalid params: tools/call 缺 params（需要 {name, arguments}）".into(),
                None,
            ));
        };
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Some(error_response(
                id,
                INVALID_PARAMS,
                "Invalid params: tools/call.params.name 缺失或非字符串".into(),
                None,
            ));
        };
        // 未知工具是协议层错误（MCP 规范 tools/call 错误处理口径）——与既有
        // dispatch_json 的 E_NOTFOUND 分工：工具面内语义（如 project 不存在）
        // 才走 isError:true。data 携带契约错误码供调用方按 code 分支。
        if !crate::TOOL_NAMES.contains(&name) {
            return Some(error_response(
                id,
                INVALID_PARAMS,
                format!("Invalid params: 未知工具 {name}（工具面见 tools/list）"),
                Some(json!({"code": "E_NOTFOUND", "tool": name})),
            ));
        }
        let args = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(v @ Value::Object(_)) => v.clone(),
            Some(_) => {
                return Some(error_response(
                    id,
                    INVALID_PARAMS,
                    format!("Invalid params: tools/call.params.arguments 须为对象（工具 {name} 的参数）"),
                    None,
                ));
            }
        };
        let reply = dispatch(ToolCall {
            tool: name.to_string(),
            args,
        });
        Some(result_response(id, call_result(&reply)))
    }
}

/// initialize 应答体（协议协商 + 能力面 + 服务端身份）。
fn initialize_result(params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str)
        .unwrap_or(LATEST_PROTOCOL_VERSION);
    let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
        requested
    } else {
        LATEST_PROTOCOL_VERSION
    };
    json!({
        "protocolVersion": negotiated,
        "capabilities": {
            // 只声明 tools（listChanged:false——工具面是静态的，不发
            // notifications/tools/list_changed；多声明的能力就是多欠的债）
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": INSTRUCTIONS,
    })
}

/// `tools/list` 应答体：单一真源 [`crate::schema::tools_schema`]。
///
/// #137 前该函数只有 name/description/implemented（描述性清单）；现补齐
/// `inputSchema`（合法 JSON Schema——官方 SDK 的 Tool 模型必填位）。
fn tools_list_result() -> Value {
    crate::schema::tools_schema()
}

/// `tools/call` 应答体（MCP CallToolResult）。
///
/// - `content[0].text`：契约 §0 包装 JSON 全文（`{ok, tool, data|error}`——
///   与旧 dispatch_json 路径**同一序列化真源**，Agent 侧解析器可复用）；
/// - `structuredContent`：成功 = 工具 out 原文 / 失败 = `{code, what, ...}`
///   （程序化取 code 免二次解析 text；未声明 outputSchema 的结构化载荷是
///   增益面——旧客户端按未知字段忽略）；
/// - `isError`：成功 false / ToolError 一律 true。
fn call_result(reply: &ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => json!({
            "content": [{ "type": "text", "text": serde_json::to_string(reply).unwrap_or_default() }],
            "structuredContent": out.result,
            "isError": false,
        }),
        ToolReply::Err { error, .. } => json!({
            "content": [{ "type": "text", "text": serde_json::to_string(reply).unwrap_or_default() }],
            "structuredContent": serde_json::to_value(error).unwrap_or(Value::Null),
            "isError": true,
        }),
    }
}

/// 回显用的 id：可合法回显（string | number | null）才透传，否则 null
/// （JSON-RPC 2.0：检测不到合法 id 的错误应答，id 必须为 null）。
fn echo_id(obj: &serde_json::Map<String, Value>) -> Value {
    match obj.get("id") {
        Some(v) if matches!(v, Value::Number(_) | Value::String(_) | Value::Null) => v.clone(),
        _ => Value::Null,
    }
}

fn result_response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_response(id: Value, code: i64, message: String, data: Option<Value>) -> String {
    let mut error = json!({ "code": code, "message": message });
    if let Some(d) = data {
        error["data"] = d;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error }).to_string()
}

// ---------------------------------------------------------------- 协议层单测
//
// 传输无关（dispatch 注入独立 BidService）——官方 SDK 的进程级 e2e 在
// tests/mcp_sdk_e2e.rs（Python mcp 客户端拉起 bid-mcp-server 子进程）。

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch_with;
    use crate::BidService;
    use serde_json::json;

    fn sess() -> McpSession {
        McpSession::new()
    }

    /// 独立服务态的 dispatch（与生产 dispatch 同语义，测试零全局污染）。
    fn call(svc: &mut BidService, c: ToolCall) -> ToolReply {
        dispatch_with(svc, c)
    }

    fn init(s: &mut McpSession, version: &str) -> Value {
        let line = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "t", "version": "0"} }
        });
        let resp = s.handle_line(&line.to_string(), |_| unreachable!()).unwrap();
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 1);
        v["result"].clone()
    }

    #[test]
    fn initialize_negotiates_supported_version_by_echo() {
        for v in SUPPORTED_PROTOCOL_VERSIONS {
            let mut s = sess(); // 每版本独立会话（握手位推进后 method 面变化）
            let r = init(&mut s, v);
            assert_eq!(r["protocolVersion"], v, "支持集内版本应原样回显: {v}");
            assert!(s.is_initialized());
        }
    }

    #[test]
    fn initialize_unknown_version_falls_back_to_latest() {
        let mut s = sess();
        let r = init(&mut s, "1999-01-01");
        assert_eq!(r["protocolVersion"], LATEST_PROTOCOL_VERSION);
    }

    #[test]
    fn initialize_result_shape() {
        let mut s = sess();
        let r = init(&mut s, "2025-06-18");
        assert_eq!(r["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(r["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(r["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(r["instructions"].as_str().is_some_and(|i| !i.is_empty()));
        // 未声明的能力不得出现在 capabilities 里（声明即债）
        assert!(r["capabilities"].get("resources").is_none());
        assert!(r["capabilities"].get("prompts").is_none());
    }

    #[test]
    fn requests_before_initialize_are_rejected_32002() {
        let mut s = sess();
        let line = json!({"jsonrpc":"2.0","id":7,"method":"tools/list"}).to_string();
        let resp: Value = serde_json::from_str(&s.handle_line(&line, |_| unreachable!()).unwrap()).unwrap();
        assert_eq!(resp["error"]["code"], SERVER_NOT_INITIALIZED);
        assert_eq!(resp["id"], 7);
        // 通知不受握手序约束（无应答通道）
        let note = json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string();
        assert!(s.handle_line(&note, |_| unreachable!()).is_none());
    }

    #[test]
    fn parse_error_is_32700_with_null_id() {
        let mut s = sess();
        let resp: Value =
            serde_json::from_str(&s.handle_line("{not json", |_| unreachable!()).unwrap()).unwrap();
        assert_eq!(resp["error"]["code"], PARSE_ERROR);
        assert_eq!(resp["id"], Value::Null);
    }

    #[test]
    fn non_object_and_missing_envelope_are_32600() {
        let mut s = sess();
        let mut s2 = sess();
        let arr = json!([{"jsonrpc":"2.0","id":1,"method":"ping"}]).to_string();
        let resp: Value =
            serde_json::from_str(&s.handle_line(&arr, |_| unreachable!()).unwrap()).unwrap();
        assert_eq!(resp["error"]["code"], INVALID_REQUEST); // batch 已废弃
        let no_jsonrpc = json!({"id": 3, "method": "ping"}).to_string();
        let resp2: Value =
            serde_json::from_str(&s2.handle_line(&no_jsonrpc, |_| unreachable!()).unwrap()).unwrap();
        assert_eq!(resp2["error"]["code"], INVALID_REQUEST);
        assert_eq!(resp2["id"], 3);
    }

    #[test]
    fn unknown_method_is_32601_and_unknown_notification_silent() {
        let mut s = sess();
        let _ = init(&mut s, "2025-06-18");
        let line = json!({"jsonrpc":"2.0","id":"abc","method":"resources/list"}).to_string();
        let resp: Value = serde_json::from_str(&s.handle_line(&line, |_| unreachable!()).unwrap()).unwrap();
        assert_eq!(resp["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(resp["id"], "abc", "字符串 id 原样回显");
        let note = json!({"jsonrpc":"2.0","method":"notifications/roots/list_changed"}).to_string();
        assert!(s.handle_line(&note, |_| unreachable!()).is_none());
    }

    #[test]
    fn ping_answers_empty_object() {
        let mut s = sess();
        let _ = init(&mut s, "2025-06-18");
        let line = json!({"jsonrpc":"2.0","id":42,"method":"ping"}).to_string();
        let resp: Value = serde_json::from_str(&s.handle_line(&line, |_| unreachable!()).unwrap()).unwrap();
        assert_eq!(resp["result"], json!({}));
        assert_eq!(resp["id"], 42);
    }

    #[test]
    fn tools_list_returns_full_tool_face_with_input_schema() {
        let mut s = sess();
        let _ = init(&mut s, "2025-06-18");
        let line = json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}).to_string();
        let resp: Value = serde_json::from_str(&s.handle_line(&line, |_| unreachable!()).unwrap()).unwrap();
        let tools = resp["result"]["tools"].as_array().expect("tools 数组");
        assert_eq!(tools.len(), crate::TOOL_NAMES.len(), "与 TOOL_NAMES 单一真源同量");
        for t in tools {
            assert!(t["inputSchema"].is_object(), "{} 缺 inputSchema", t["name"]);
            assert_eq!(t["inputSchema"]["type"], "object");
            assert!(t["description"].as_str().is_some_and(|d| !d.is_empty()));
        }
    }

    #[test]
    fn tools_call_unknown_tool_is_32602_with_contract_code() {
        let mut s = sess();
        let _ = init(&mut s, "2025-06-18");
        let line = json!({
            "jsonrpc":"2.0","id":9,"method":"tools/call",
            "params": {"name": "no_such_tool", "arguments": {}}
        })
        .to_string();
        let resp: Value = serde_json::from_str(&s.handle_line(&line, |_| unreachable!()).unwrap()).unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
        assert_eq!(resp["error"]["data"]["code"], "E_NOTFOUND", "契约错误码透传");
    }

    #[test]
    fn tools_call_bad_arguments_shape_is_32602() {
        let mut s = sess();
        let _ = init(&mut s, "2025-06-18");
        let mut svc = BidService::new();
        for bad in [
            json!({"name": "create_bid_project"}),                            // arguments 缺席（合法→空参）——另行断言，此处占位
            json!({"name": "create_bid_project", "arguments": [1, 2]}),       // arguments 非对象
            json!({"arguments": {}}),                                         // name 缺席
            json!({"name": 42, "arguments": {}}),                             // name 非字符串
        ] {
            let line = json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params": bad}).to_string();
            let resp: Value =
                serde_json::from_str(&s.handle_line(&line, |c| call(&mut svc, c)).unwrap()).unwrap();
            if bad["name"] == "create_bid_project" && bad.get("arguments").is_none() {
                // arguments 缺席 = 空参对象（合法）——走工具层 E_UNPROCESSABLE
                assert_eq!(resp["result"]["isError"], true);
                assert_eq!(resp["result"]["structuredContent"]["code"], "E_UNPROCESSABLE");
            } else {
                assert_eq!(resp["error"]["code"], INVALID_PARAMS, "参数形状非法: {bad}");
            }
        }
    }

    #[test]
    fn tools_call_success_maps_to_content_and_structured() {
        let mut s = sess();
        let _ = init(&mut s, "2025-06-18");
        let mut svc = BidService::new();
        let line = json!({
            "jsonrpc":"2.0","id":11,"method":"tools/call",
            "params": {"name": "create_bid_project", "arguments": {
                "name": "协议层测试项目", "bid_deadline": "2099-12-31",
                "mode": "明标", "external_key": "mcp-protocol-test"
            }}
        })
        .to_string();
        let resp: Value =
            serde_json::from_str(&s.handle_line(&line, |c| call(&mut svc, c)).unwrap()).unwrap();
        assert_eq!(resp["id"], 11);
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let wire: Value = serde_json::from_str(text).expect("content.text 是契约 §0 包装 JSON");
        assert_eq!(wire["ok"], true);
        assert_eq!(wire["tool"], "create_bid_project");
        assert!(wire["data"]["project_id"].as_str().is_some());
        assert_eq!(resp["result"]["structuredContent"]["project_id"], wire["data"]["project_id"]);
    }

    #[test]
    fn tools_call_tool_error_maps_to_is_error_with_code() {
        let mut s = sess();
        let _ = init(&mut s, "2025-06-18");
        let mut svc = BidService::new();
        // 无项目时只读工具 → E_NOTFOUND（工具执行错误 → isError:true，非 JSON-RPC error）
        let line = json!({
            "jsonrpc":"2.0","id":"e1","method":"tools/call",
            "params": {"name": "get_compliance_matrix", "arguments": {"project_id": "P-404"}}
        })
        .to_string();
        let resp: Value =
            serde_json::from_str(&s.handle_line(&line, |c| call(&mut svc, c)).unwrap()).unwrap();
        assert_eq!(resp["id"], "e1");
        assert_eq!(resp["result"]["isError"], true);
        // #325（R-13）：wire tag 已钉回契约字面量 "E_NOTFOUND"（变体级
        // serde rename——此前 SCREAMING_SNAKE_CASE 机械展开成 "E_NOT_FOUND"，
        // 与 ToolError::code() / 契约 §0 漂移，#137 时期选择只透传不改，
        // 现按 #325 裁定单真源=契约并修复）；MCP 层与 dispatch_json 同一
        // 序列化真源，一致性由 error.rs 的
        // wire_tag_matches_code_literal_all_variants 测试把守。
        assert_eq!(resp["result"]["structuredContent"]["code"], "E_NOTFOUND");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let wire: Value = serde_json::from_str(text).unwrap();
        assert_eq!(wire["ok"], false);
        assert_eq!(wire["error"]["code"], "E_NOTFOUND", "契约包装 JSON 原样进 content（#325 钉回契约字面量）");
    }

    #[test]
    fn tools_call_state_error_keeps_contract_semantics() {
        let mut s = sess();
        let _ = init(&mut s, "2025-06-18");
        let mut svc = BidService::new();
        let mk = |key: &str| {
            json!({
                "jsonrpc":"2.0","id":1,"method":"tools/call",
                "params": {"name": "create_bid_project", "arguments": {
                    "name": "状态机测试", "bid_deadline": "2099-12-31",
                    "mode": "暗标", "external_key": key
                }}
            })
            .to_string()
        };
        // 建项目 → DRAFT；未导入直接抽条款 → E_STATE（状态机越级，零副作用）
        let first: Value = serde_json::from_str(
            &s.handle_line(&mk("state-a"), |c| call(&mut svc, c)).unwrap(),
        )
        .unwrap();
        let project_id = first["result"]["structuredContent"]["project_id"]
            .as_str()
            .expect("建项目成功")
            .to_string();
        let extract = json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params": {"name": "extract_requirements", "arguments": {"project_id": project_id}}
        })
        .to_string();
        let resp: Value = serde_json::from_str(
            &s.handle_line(&extract, |c| call(&mut svc, c)).unwrap(),
        )
        .unwrap();
        assert_eq!(resp["result"]["isError"], true);
        assert_eq!(resp["result"]["structuredContent"]["code"], "E_STATE");
        // 同 external_key 重放 → 幂等成功（deduplicated:true）
        let replay: Value = serde_json::from_str(
            &s.handle_line(&mk("state-a"), |c| call(&mut svc, c)).unwrap(),
        )
        .unwrap();
        assert_eq!(replay["result"]["isError"], false, "同 external_key 幂等重放");
        assert_eq!(replay["result"]["structuredContent"]["deduplicated"], true);
    }
}
