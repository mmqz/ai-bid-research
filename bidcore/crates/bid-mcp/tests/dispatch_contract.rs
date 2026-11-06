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
    // = 20（B1 价格分 + B2 自评的 MCP 接线——「能得几分、该报多少价」）；
    // #184 增 1（record_addendum_waiver——补遗分级阻塞的逐件人工确认）
    // = 21
    // #219 增 3（get_document_chunks / submit_block_marks /
    // get_coverage_report——全块覆盖：Agent 区间标记 + 引擎覆盖检查）= 24；
    // #226 增 1（set_matrix_override——否决项死锁的人工出口）= 25；
    // #243 增 1（recover_validating_state——历史异常态恢复出口）= 26
assert_eq!(TOOL_NAMES.len(), 32, "工具面必须恰为 32 个（21 + #219 全块覆盖三工具 + #226 否决项死锁出口 + #243 恢复出口 + #254 分段三工具 + #279 行协议发现面 + #287 篇幅档位 + #285 ★漏标检测）");

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
    // #145 任务 1：search_evidence 接线后清单**清空**——全部 20 工具实装
    // （骨架纪律保留：未来新增工具未接线时回到此清单，防止骨架与实现脱钩）。
    // 此前 B-1 落地 create/import；B5 落地 extract/matrix；#59 阶段 1-3 落地
    // draft/export/validate——不在此清单。
    let unimplemented: [&str; 0] = [];
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
assert_eq!(arr.len(), 32, "tools/list 必须列出全部 32 个工具");

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
    // #100 任务 3：score_price / score_section / score_document（B1/B2 接线）；
    // #145 任务 1：search_evidence 接线 + save_version 补登（此前 is_implemented
    // 漏登致 tools/list 误报 false——#145 测试面抓出，如实登记）→ 全部 20 实装；
    // #184：record_addendum_waiver（补遗分级阻塞的逐件人工确认）→ 21 实装；
    // #219 增 3 覆盖工具 → 24 实装；
    // #226：set_matrix_override（否决项死锁出口）→ 25 实装；
    // #243：recover_validating_state（历史异常态恢复出口）→ 26 实装
    assert_eq!(
        implemented,
        vec![
            "create_bid_project",
            "import_tender_files",
            "extract_requirements",
            "get_compliance_matrix",
            "list_evidence",
            "list_tasks",
            "search_evidence",
            "draft_response_section",
            "validate_bid_document",
            "export_bid_document",
            "get_bid_project_state",
            "save_version",
            "check_bid_quality",
            "build_outline",
            "register_evidence",
            "bind_evidence",
            "record_review_gate",
            "record_addendum_waiver",
            "score_price",
            "score_section",
            "score_document",
            // #219 全块覆盖三工具
            "get_document_chunks",
            "submit_block_marks",
            "get_coverage_report",
            // #285 ① ★漏标检测
            "get_star_coverage",
            // #226 层 1：否决项死锁的人工出口
            "set_matrix_override",
            // #243：历史异常态（VALIDATING+缺章）的受支持恢复出口
            "recover_validating_state",
            // #254 章节级增量写入三工具
            "append_section_segment",
            "rewrite_section_segment",
            "list_section_segments",
            // #287 篇幅档位推荐
            "recommend_length_profile",
            // #279 行协议发现面
            "list_tools",
        ]
    );
    // 描述面：新工具不得是 unknown（describe 遗漏即挂）
    for t in arr.iter().filter(|t| t["name"].as_str().unwrap().starts_with("score_")) {
        assert_ne!(t["description"], "unknown", "score_* 描述必须在场: {t}");
    }
    let ra = arr.iter().find(|t| t["name"] == "record_addendum_waiver").unwrap();
    assert_ne!(ra["description"], "unknown", "record_addendum_waiver 描述必须在场: {ra}");
    let rv = arr.iter().find(|t| t["name"] == "recover_validating_state").unwrap();
    assert_ne!(rv["description"], "unknown", "recover_validating_state 描述必须在场: {rv}");
    for t in arr.iter().filter(|t| t["name"].as_str().unwrap().starts_with("append_section")
        || t["name"].as_str().unwrap().starts_with("rewrite_section")
        || t["name"].as_str().unwrap().starts_with("list_section")) {
        assert_ne!(t["description"], "unknown", "#254 分段工具描述必须在场: {t}");
    }
    let lt = arr.iter().find(|t| t["name"] == "list_tools").unwrap();
    assert_ne!(lt["description"], "unknown", "#279 list_tools 描述必须在场: {lt}");
}
