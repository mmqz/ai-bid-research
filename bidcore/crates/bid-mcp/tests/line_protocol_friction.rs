//! #279 行协议摩擦测试（MCP 协议契约缺陷三处修复）。
//!
//! 来源：#269 六场黑盒考验的协议摩擦实证——
//! - 4/6 场踩中信封坑：参数放顶层（`{"tool":..,"name":..}`）时 serde 静默
//!   吞掉、`args` 落空，工具层报误导性的「name 缺失或非字符串」；
//!   danqing / hc-cluster 各 **5 连败**后靠猜悟出信封结构；
//! - 两个独立代理都试探了 `list_tools`（Agent 先握手的合理预期）——行协议
//!   无此工具只能 E_NOTFOUND；
//! - CAS project_version 心智负担：单代理顺序作业仍被迫追踪版本号。
//!
//! 三项对应：信封结构错误（第 2 项）/ list_tools 发现面（第 1 项）/
//! project_version "latest"（第 3 项）。
//!
//! 并行安全：信封错误路径在 dispatch 前短路（零状态触达）；穿透到分发的
//! 反例均为只读或参数校验失败（无副作用）；CAS 面经 `dispatch_with` 注入
//! 独立 `BidService`。

use bid_mcp::{dispatch_json, dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn err_what(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

/// dispatch_json 应答解析助手（行协议线格式：{ok, error{code, what}}）。
fn json_reply(input: &str) -> Value {
    serde_json::from_str(&dispatch_json(input)).unwrap()
}

// ---------------------------------------------------------------- 第 2 项：信封结构错误

/// #269 实测主形态：tool 在场、参数放顶层、args 缺失——必须报**结构问题**
/// （「参数须嵌套在 args 下」+ 点名误置字段），不得再报「字段缺失」误导。
#[test]
fn top_level_args_reports_envelope_error_not_field_missing() {
    let v = json_reply(
        r#"{"tool":"create_bid_project","name":"水库项目","bid_deadline":"2099-12-31","mode":"明标","external_key":"env-1"}"#,
    );
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "E_UNPROCESSABLE");
    let what = v["error"]["what"].as_str().unwrap();
    assert!(
        what.contains("参数须嵌套在 \"args\" 下"),
        "须报信封结构错误：{what}"
    );
    assert!(what.contains("name"), "须点名误置字段：{what}");
    assert!(
        !what.contains("缺失或非字符串"),
        "不得再报字段缺失（误导性错误——六场 11 次失败调用的根因）：{what}"
    );
    assert!(
        what.contains("create_bid_project"),
        "正确形态示例须带工具名：{what}"
    );
}

/// args 显式空对象 + 参数放顶层：同一误置形态的另一写法，同样报结构错误。
#[test]
fn empty_args_with_top_level_fields_same_envelope_error() {
    let v = json_reply(r#"{"tool":"create_bid_project","args":{},"name":"x"}"#);
    assert_eq!(v["error"]["code"], "E_UNPROCESSABLE");
    assert!(v["error"]["what"].as_str().unwrap().contains("参数须嵌套在 \"args\" 下"));
    // args 显式 null 同理
    let v2 = json_reply(r#"{"tool":"extract_requirements","args":null,"project_id":"PRJ-0001"}"#);
    assert!(v2["error"]["what"].as_str().unwrap().contains("参数须嵌套在 \"args\" 下"));
}

/// 解析失败路径：连 tool 都没给、参数全放顶层——serde 裸错误（missing field
/// `tool`）对这类调用方没有指引价值，须附带结构提示。
#[test]
fn parse_failure_with_tool_args_at_top_gets_structural_hint() {
    let v = json_reply(r#"{"name":"x","bid_deadline":"2099-12-31","external_key":"k"}"#);
    assert_eq!(v["error"]["code"], "E_UNPROCESSABLE");
    let what = v["error"]["what"].as_str().unwrap();
    assert!(what.contains("missing field `tool`"), "保留 serde 原始诊断：{what}");
    assert!(what.contains("应嵌套在 \"args\" 下"), "须附结构指引：{what}");
}

/// 反例（不越权）：顶层字段是**未知**字段（非工具参数）——维持原语义，
/// 工具层自报缺失（调用方真的没传参数）。
#[test]
fn unknown_top_level_fields_keep_original_semantics() {
    let v = json_reply(r#"{"tool":"create_bid_project","id":7}"#);
    assert_eq!(v["error"]["code"], "E_UNPROCESSABLE");
    let what = v["error"]["what"].as_str().unwrap();
    assert!(
        what.contains("缺失或非字符串"),
        "未知顶层字段不触发信封错误——工具层原语义：{what}"
    );
}

/// 反例：参数正确嵌套的信封完全不受影响（正常错误/正常成功路径）。
#[test]
fn properly_nested_args_unaffected() {
    // 嵌套 + 项目不存在 → 正常 E_NOTFOUND（非信封错误）
    let v = json_reply(r#"{"tool":"get_bid_project_state","args":{"project_id":"PRJ-404"}}"#);
    assert_eq!(v["error"]["code"], "E_NOT_FOUND");
    // 无参工具裸调用（args 缺省、顶层无多余字段）→ 不触发
    let v2 = json_reply(r#"{"tool":"list_tools"}"#);
    assert_eq!(v2["ok"], true, "list_tools 无参可调：{v2}");
}

// ---------------------------------------------------------------- 第 1 项：list_tools 发现面

/// 行协议握手入口：返回工具清单（单一真源 TOOL_NAMES）+ 契约摘要——
/// 把「六场靠猜」变成「一次调用可知」。
#[test]
fn list_tools_returns_tool_face_and_contract_summary() {
    let mut svc = BidService::new();
    let data = ok_result(call(&mut svc, "list_tools", json!({})));
    assert_eq!(data["count"], bid_mcp::TOOL_NAMES.len(), "与 TOOL_NAMES 单一真源同量");
    let tools = data["tools"].as_array().unwrap();
    assert_eq!(tools.len(), bid_mcp::TOOL_NAMES.len());
    for (i, name) in bid_mcp::TOOL_NAMES.iter().enumerate() {
        assert_eq!(tools[i]["name"], *name, "清单序 = TOOL_NAMES 序");
        assert!(
            tools[i]["summary"].as_str().is_some_and(|s| !s.is_empty()),
            "{name} 缺摘要"
        );
    }
    // 契约摘要：信封形态（六场摩擦的直接解药）
    let lp = &data["line_protocol"];
    assert!(
        lp["request"].as_str().unwrap().contains("\"args\""),
        "信封形态必须教 args 嵌套：{}",
        lp["request"]
    );
    assert!(lp["response"].as_str().unwrap().contains("error"));
    let notes = lp["notes"].as_array().unwrap();
    let notes_text: Vec<&str> = notes.iter().filter_map(Value::as_str).collect();
    assert!(
        notes_text.iter().any(|n| n.contains("error.code")),
        "须教按 code 分支（不解析文案）"
    );
    assert!(
        notes_text.iter().any(|n| n.contains("latest")),
        "须教 CAS latest 形态"
    );
}

/// 无状态前置：全新服务态（未建项目）即可握手——发现面不得依赖项目路由。
#[test]
fn list_tools_works_before_any_project_exists() {
    let mut svc = BidService::new();
    let r = call(&mut svc, "list_tools", json!({}));
    assert!(matches!(r, ToolReply::Ok(_)), "发现面无状态前置：{r:?}");
}

/// JSON-RPC 面等价可见：tools/list 清单包含 list_tools（schema 单一真源）。
#[test]
fn list_tools_visible_in_jsonrpc_tools_list() {
    let schema = bid_mcp::tools_schema();
    let names: Vec<&str> = schema["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"list_tools"), "tools/list 必须含发现面工具");
}

// ---------------------------------------------------------------- 第 3 项：CAS "latest"

/// "latest" = 显式声明无并发：跳过 CAS 检查（与缺省同语义）；数字强一致
/// 路径不动（过期仍 E_CONFLICT）；非法字符串显式拒。
///
/// 幽灵文件放在**受信根内**（缺失件不在围栏层抢答——R-01 逐件失败语义在
/// 服务层 sha256_file），使调用能走到 CAS / 文件处理面。
#[test]
fn cas_latest_skips_check_numeric_path_intact() {
    let root = std::env::temp_dir().join(format!("mcp279_cas_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let ghost = root.join("ghost.docx").display().to_string();
    let mut svc = BidService::new().with_fs_root(&root);
    let r = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "CAS latest 测试", "bid_deadline": "2099-01-01",
        "mode": "明标", "external_key": "cas-latest-1",
    })));
    let pid = r["project_id"].as_str().unwrap().to_string();
    let files = json!([{"upload_ref": ghost, "filename": "ghost.docx", "kind": "招标文件"}]);

    // 数字强一致路径不动：过期版本 → E_CONFLICT（现状保持）
    let stale = call(&mut svc, "import_tender_files", json!({
        "project_id": pid, "project_version": 999, "files": files,
    }));
    assert_eq!(err_code(&stale), "E_CONFLICT", "数字 CAS 强一致路径不动");

    // "latest" → 跳过 CAS：同样的调用不再 E_CONFLICT（走到后续的文件处理面）
    let latest = call(&mut svc, "import_tender_files", json!({
        "project_id": pid, "project_version": "latest", "files": files,
    }));
    assert_ne!(err_code(&latest), "E_CONFLICT", "\"latest\" 必须跳过 CAS 检查：{latest:?}");

    // 非法字符串 → 显式拒（不是静默跳过）
    let banana = call(&mut svc, "import_tender_files", json!({
        "project_id": pid, "project_version": "banana", "files": files,
    }));
    assert_eq!(err_code(&banana), "E_UNPROCESSABLE");
    assert!(
        err_what(&banana).contains("latest"),
        "拒绝文案须指明合法形态（非负整数或 \"latest\"）"
    );
}

/// 行协议线上 "latest" 同样生效（dispatch_json 全链穿透）。
#[test]
fn cas_latest_works_over_line_protocol() {
    // 参数解析在项目路由前——用不存在的项目隔离全局态（E_NOTFOUND 证明
    // version 参数已被接受，而非 E_UNPROCESSABLE 参数拒）
    let v = json_reply(
        r#"{"tool":"extract_requirements","args":{"project_id":"PRJ-404","project_version":"latest"}}"#,
    );
    assert_eq!(v["error"]["code"], "E_NOT_FOUND", "\"latest\" 须被接受：{v}");
    let v2 = json_reply(
        r#"{"tool":"extract_requirements","args":{"project_id":"PRJ-404","project_version":"soon"}}"#,
    );
    assert_eq!(v2["error"]["code"], "E_UNPROCESSABLE", "非法字符串仍拒：{v2}");
}
