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
fn tool_names_are_the_contract_twenty() {
    // V3-5（S-4）：8+1+2 = 11；#42 阶段 10 增 save_version = 12；
    // #68 阶段 2 增 check_bid_quality = 13；
    // #80 第三批 MCP 补齐增 4（build_outline / register_evidence /
    // bind_evidence / record_review_gate）= 17（G1/G2/G3——外部 Agent
    // 纯 MCP 驱动全链）；
    // #100 任务 3 增 3（score_price / score_section / score_document）
    // = 20（B1 价格分 + B2 自评的 MCP 接线——「能得几分、该报多少价」）
    assert_eq!(TOOL_NAMES.len(), 20, "工具面必须恰为 20 个（17 + #100 任务 3 评分三件）");
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
    // 1 个未落地工具：必须返回 UNIMPLEMENTED_INTERNAL，不能"空成功"
    // （B-1 落地 create/import；B5 落地 extract/matrix；#59 阶段 1-3 落地
    // draft/export/validate——不在此清单。工具落地时本清单同步收缩，
    // 防止骨架与实现脱钩）
    let unimplemented = [
        "search_evidence",
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
fn tools_schema_lists_all_twenty() {
    let v = bid_mcp::tools_schema();
    let arr = v["tools"].as_array().unwrap();
    assert_eq!(arr.len(), 20);
    let implemented: Vec<&str> = arr
        .iter()
        .filter(|t| t["implemented"] == true)
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    // B-1：create/import；B5：extract/matrix；V3-5（S-4）：list_evidence/list_tasks；
    // #59 阶段 1+2：draft_response_section / export_bid_document；
    // #68 阶段 2：check_bid_quality；
    // + get_bid_project_state（TOOL_NAMES 声明序）——工具落地时本清单同步
    // 收缩，防骨架与实现脱钩；
    // #100 任务 3：score_price / score_section / score_document（B1/B2 接线）
    assert_eq!(
        implemented,
        vec![
            "create_bid_project",
            "import_tender_files",
            "extract_requirements",
            "get_compliance_matrix",
            "list_evidence",
            "list_tasks",
            "draft_response_section",
            "validate_bid_document",
            "export_bid_document",
            "get_bid_project_state",
            "check_bid_quality",
            "build_outline",
            "register_evidence",
            "bind_evidence",
            "record_review_gate",
            "score_price",
            "score_section",
            "score_document",
        ]
    );
    // 描述面：新工具不得是 unknown（describe 遗漏即挂）
    for t in arr.iter().filter(|t| t["name"].as_str().unwrap().starts_with("score_")) {
        assert_ne!(t["description"], "unknown", "score_* 描述必须在场: {t}");
    }
}
