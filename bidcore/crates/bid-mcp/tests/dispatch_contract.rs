//! M0 骨架契约测试：**未实现 ≠ 成功**。
//!
//! 本文件存在的唯一目的：防止"骨架期工具返回空成功"这种假绿。
//! 每个未落地工具都必须被断言为 `UNIMPLEMENTED_INTERNAL`，而不是 ok=true。

use bid_mcp::{dispatch, dispatch_json, schema::ToolCall, TOOL_NAMES};
use serde_json::json;

fn call(tool: &str, args: serde_json::Value) -> bid_mcp::ToolReply {
    dispatch(ToolCall {
        tool: tool.to_string(),
        args,
    })
}

#[test]
fn tool_names_are_the_contract_eleven() {
    // V3-5（S-4）：原 8 工具 + get_bid_project_state + list_evidence/list_tasks = 11
    assert_eq!(TOOL_NAMES.len(), 11, "工具面必须恰为 11 个（8+1+2 契约——V3-5 增两列表工具）");
    for name in TOOL_NAMES {
        assert!(!name.is_empty());
    }
}

#[test]
fn unknown_tool_is_typed_not_found() {
    match call("not_a_tool", json!({})) {
        bid_mcp::ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_NOTFOUND"),
        bid_mcp::ToolReply::Ok(o) => panic!("未知工具不得成功: {o:?}"),
    }
}

#[test]
fn unimplemented_tools_return_typed_unimplemented_not_ok() {
    // 4 个未落地工具：必须全部返回 UNIMPLEMENTED_INTERNAL，一个都不能"空成功"
    // （B-1 落地 create/import；B5 落地 extract_requirements/get_compliance_matrix
    // ——不在此清单。工具落地时本清单同步收缩，防止骨架与实现脱钩）
    let unimplemented = [
        "search_evidence",
        "draft_response_section",
        "validate_bid_document",
        "export_bid_document",
    ];
    for tool in unimplemented {
        match call(tool, json!({})) {
            bid_mcp::ToolReply::Err { error, .. } => {
                assert_eq!(
                    error.code(),
                    "UNIMPLEMENTED_INTERNAL",
                    "{tool} 必须返回 UNIMPLEMENTED_INTERNAL（骨架期不得假装成功）"
                );
            }
            bid_mcp::ToolReply::Ok(o) => panic!("{tool} 骨架期不得成功，实际: {o:?}"),
        }
    }
}

#[test]
fn implemented_tool_returns_state_and_allowed_tools() {
    match call("get_bid_project_state", json!({"project_status": "DRAFT"})) {
        bid_mcp::ToolReply::Ok(out) => {
            assert_eq!(out.result["status"], "DRAFT");
            let allowed = out.result["allowed_next_tools"].as_array().unwrap();
            assert!(allowed.iter().any(|v| v == "create_bid_project"));
            assert_eq!(out.result["terminal"], false);
        }
        bid_mcp::ToolReply::Err { error, .. } => panic!("已落地工具不得失败: {error}"),
    }
}

#[test]
fn implemented_tool_rejects_bad_status_typed() {
    match call("get_bid_project_state", json!({"project_status": "NOT_A_STATUS"})) {
        bid_mcp::ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_UNPROCESSABLE"),
        bid_mcp::ToolReply::Ok(o) => panic!("非法状态必须被拒: {o:?}"),
    }
}

#[test]
fn terminal_state_is_marked() {
    match call("get_bid_project_state", json!({"project_status": "EXPORTED"})) {
        bid_mcp::ToolReply::Ok(out) => assert_eq!(out.result["terminal"], true),
        bid_mcp::ToolReply::Err { error, .. } => panic!("{error}"),
    }
}

#[test]
fn json_roundtrip_failure_is_typed() {
    let out = dispatch_json("{ this is not json");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "E_UNPROCESSABLE");
}

#[test]
fn tools_schema_lists_all_eleven() {
    let v = bid_mcp::tools_schema();
    let arr = v["tools"].as_array().unwrap();
    assert_eq!(arr.len(), 11);
    let implemented: Vec<&str> = arr
        .iter()
        .filter(|t| t["implemented"] == true)
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    // B-1：create/import；B5：extract/matrix；V3-5（S-4）：list_evidence/list_tasks；
    // + get_bid_project_state（TOOL_NAMES 声明序）——B5 期漏登 extract/matrix 的
    // implemented 位已一并修正（工具落地时本清单同步收缩，防骨架与实现脱钩）
    assert_eq!(
        implemented,
        vec![
            "create_bid_project",
            "import_tender_files",
            "extract_requirements",
            "get_compliance_matrix",
            "list_evidence",
            "list_tasks",
            "get_bid_project_state",
        ]
    );
}
