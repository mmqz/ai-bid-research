//! #68 阶段 2 · check_bid_quality 壳层契约测试（bid-mcp tools.rs）。
//!
//! 验收面（issue #68 §2.1/§2.2）：
//! - 状态机裁决：FILES_IMPORTED 越级 → E_STATE；项目路由 E_NOTFOUND；
//! - 无章节树 → E_STATE；
//! - 纯内存态：报告内联返回 + artifact null + 「未落盘」告警；
//! - workspace 在场：`artifacts/quality-report.json` 落盘（artifact.path/size）；
//! - 覆盖度面：draft 后 REQ 未回应 → stats.uncovered_requirements + warning；
//! - needs_human 清单在 out（§2.2「哪些章需要人工处理」的 Agent 面）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use bid_domain::outline::OutlineSpec;
use serde_json::{json, Value};

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut().unwrap().insert("generator".into(), json!("mock"));
    }
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn err_parts(reply: &ToolReply) -> (String, String) {
    match reply {
        ToolReply::Err { error, .. } => (error.code().to_string(), error.to_string()),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn mk_req(id: &str, excerpt: &str) -> bid_domain::Requirement {
    use bid_domain::{Anchor, Requirement};
    Requirement {
        id: id.into(),
        category: "技术参数".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第二章/{id}"),
            excerpt: excerpt.into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

/// 「MATRIX_READY + 建纲 + 已起草一章」服务态（记忆面或 workspace 面）。
fn svc_drafted(tag: &str, ws: Option<&std::path::Path>) -> (BidService, String, String) {
    let mut svc = match ws {
        Some(dir) => BidService::open(dir).unwrap(),
        None => BidService::new(),
    };
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "质量工具测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-q", 10, 1, 50, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "具备贰级及以上资质"),
        mk_req("REQ-0002", "项目经理须持一级建造师证书"),
    ]).unwrap();
    let (_, _, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 400, max_reqs_per_leaf: 2 })
        .unwrap();
    let parents: std::collections::HashSet<&str> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.as_deref()).collect();
    let leaf = svc.store().sections().iter()
        .find(|n| !parents.contains(&n.id[..])).unwrap().id.clone();
    (svc, pid, leaf)
}

// ---------------------------------------------------------------- 测试

#[test]
fn quality_rejects_early_state_and_bad_project() {
    // 早态：只导入（FILES_IMPORTED）→ guard_tool 拒（check 的状态门 MatrixReady+）
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "早态测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "q-early",
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A.docx", "招标文件", "sha-e", 5, 0, 20, 0, bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    let reply = call(&mut svc, "check_bid_quality", json!({ "project_id": &pid }));
    assert_eq!(err_parts(&reply).0, "E_STATE", "FILES_IMPORTED 态不可查质量");

    let (mut svc2, _, _) = svc_drafted("q-route", None);
    let reply = call(&mut svc2, "check_bid_quality", json!({ "project_id": "PRJ-9999" }));
    let (code, msg) = err_parts(&reply);
    assert_eq!(code, "E_NOTFOUND");
    assert!(msg.contains("PRJ-9999"), "{msg}");
}

#[test]
fn quality_report_inline_with_degrade_warning_in_memory_mode() {
    let (mut svc, pid, leaf) = svc_drafted("q-mem", None);
    // 起草一章（mock——REQ-0001/0002 都会被模板引用 → 全覆盖）
    let r = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf,
    })));
    assert_eq!(r["stats"]["generator"], "mock");
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": &pid,
    })));
    // summary 面：全部起草且覆盖
    assert_eq!(q["summary"]["total"], q["summary"]["drafted"]);
    assert_eq!(q["summary"]["uncovered_req_total"], 0, "mock 模板全覆盖");
    assert_eq!(q["summary"]["failed"], 0);
    // 记忆面：无 artifact + 告警
    assert!(q["artifact"].is_null(), "纯内存态不落盘");
    let warns = q["warnings"].as_array().unwrap();
    assert!(warns.iter().any(|w| w.as_str().unwrap().contains("未落盘")), "{warns:?}");
    // sections 内联（逐章明细）
    assert!(q["sections"].as_array().unwrap().len() >= 1);
}

#[test]
fn quality_artifact_written_in_workspace_mode() {
    let dir = std::env::temp_dir().join(format!(
        "bid-mcp-quality-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut svc, pid, leaf) = svc_drafted("q-ws", Some(&dir));
    ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf,
    })));
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": &pid,
    })));
    let artifact = &q["artifact"];
    assert!(artifact.is_object(), "workspace 在场必落盘: {artifact}");
    let path = artifact["path"].as_str().unwrap();
    assert!(path.ends_with("artifacts/quality-report.json"), "{path}");
    assert!(artifact["size"].as_u64().unwrap() > 0);
    // 落盘内容是合法报告（roundtrip——summary 面对账）
    let text = std::fs::read_to_string(path).unwrap();
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["summary"]["total"], q["summary"]["total"]);
    // 文件是 QualityReport 形态（needs_human 在 summary 内）；MCP out 是顶层
    assert_eq!(parsed["summary"]["needs_human"], q["needs_human"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn draft_out_reports_uncovered_requirements() {
    let (mut svc, pid, leaf) = svc_drafted("q-uncovered", None);
    // 定向起草：只响应 REQ-0001（override）——REQ-0002 不在响应面 → 不算未覆盖；
    // 再看全量起草的正常面：mock 模板逐 REQ 引用 → uncovered 恒空。
    // 未覆盖面的构造：用 instructions 不影响模板（mock 确定性）——改用
    // 「重起草前先查」路径：未起草章在报告里是 pending（uncovered=0）。
    // 真正的 uncovered 只在真实 AI 面出现——此处断言契约字段在场 + 空清单语义。
    let r = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf,
        "source_requirements": ["REQ-0001"],
    })));
    let unc = r["stats"]["uncovered_requirements"].as_array().unwrap();
    assert!(unc.is_empty(), "override 面 REQ-0001 被模板回应: {unc:?}");
    // 报告面：该章 covered=REQ-0001（响应面就是 override 集）
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": &pid,
    })));
    let sec = q["sections"].as_array().unwrap().iter()
        .find(|s| s["section_id"] == json!(leaf)).unwrap();
    assert_eq!(sec["covered_reqs"], json!(["REQ-0001"]), "override 面被回应");
    // 节点绑定面仍含 REQ-0002——正文未回应它 → 未覆盖清单如实报（Agent 面：
    // 要么补写要么补一次面向 REQ-0002 的起草）
    assert_eq!(sec["uncovered_reqs"], json!(["REQ-0002"]));
}

#[test]
fn needs_human_lists_failed_and_uncovered_sections() {
    let (mut svc, pid, leaf) = svc_drafted("q-human", None);
    // 域面经真实状态机落一个 FAILED 章（组装线的失败面——工具只读报告）
    use bid_domain::outline::SectionStatus;
    for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Failed] {
        svc.store_mut().advance_section_status(&leaf, to).unwrap();
    }
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": &pid,
    })));
    assert_eq!(q["summary"]["failed"], 1);
    let needs: Vec<&str> = q["needs_human"].as_array().unwrap()
        .iter().filter_map(|v| v.as_str()).collect();
    assert!(needs.contains(&&leaf[..]), "FAILED 章进 needs_human: {needs:?}");
}
