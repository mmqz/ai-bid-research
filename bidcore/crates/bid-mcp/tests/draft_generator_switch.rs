//! #68 阶段 1 · draft_response_section 生成器三态切换（bid-mcp tools.rs 壳层）。
//!
//! 验收面（issue #68 §1.2）：
//! - `generator` 参数校验：非 auto/mock/real → E_UNPROCESSABLE；
//! - `mock` 强制：无 key 环境照常出稿，stats.generator=mock，无降级告警；
//! - `real` 无 key：E_UNPROCESSABLE 且错误说明配置面（显式要求要响亮）；
//! - `auto` 无 key：降级 mock + 告警「未配置 api_key」（Agent 可感知）；
//! - out.stats.generator 字段在场（契约 §6 新字段）。
//!
//! 环境门：real/auto 的无 key 断言在 BID_LLM_API_KEY 在场时 SKIP
//! （dev 机配了 key 时 auto 会走真实调用——那是 feature 不是 regression）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use bid_domain::outline::OutlineSpec;
use serde_json::{json, Value};

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
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

/// 「MATRIX_READY + 建纲」服务态（照 draft_tool.rs 夹具——记忆面无 workspace，
/// 即无 .mang/llm.json：LLM 配置只受 env 影响）。
fn svc_ready(tag: &str) -> (BidService, String, String) {
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "生成器切换测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-gen", 10, 1, 50, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "投标人须具备水利水电工程施工总承包贰级及以上资质"),
    ]).unwrap();
    let (n_sec, _, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 1_000, max_reqs_per_leaf: 2 })
        .unwrap();
    assert!(n_sec >= 1);
    let parents: std::collections::HashSet<&str> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.as_deref()).collect();
    let leaf = svc.store().sections().iter()
        .find(|n| !parents.contains(&n.id[..])).unwrap().id.clone();
    (svc, pid, leaf)
}

fn mk_req(id: &str, category: &str, excerpt: &str) -> bid_domain::Requirement {
    use bid_domain::{Anchor, Requirement};
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第二章/1.1".into(),
            excerpt: excerpt.into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

fn env_key_absent() -> bool {
    std::env::var("BID_LLM_API_KEY").map(|v| v.trim().is_empty()).unwrap_or(true)
}

// ---------------------------------------------------------------- 测试

#[test]
fn generator_param_rejects_unknown_value() {
    let (mut svc, pid, leaf) = svc_ready("gen-bad");
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "generator": "gpt4",
    }));
    let (code, msg) = err_parts(&reply);
    assert_eq!(code, "E_UNPROCESSABLE");
    assert!(msg.contains("auto | mock | real"), "{msg}");
}

#[test]
fn generator_mock_forced_works_without_key() {
    let (mut svc, pid, leaf) = svc_ready("gen-mock");
    let r = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "generator": "mock",
    })));
    assert_eq!(r["stats"]["generator"], "mock");
    assert_eq!(r["section_status"], "DRAFTED");
    // 显式 mock 是选择不是降级——无降级告警
    let warns = r["warnings"].as_array().unwrap();
    assert!(warns.iter().all(|w| !w.as_str().unwrap().contains("降级 mock")),
        "显式 mock 不应有降级告警: {warns:?}");
}

#[test]
fn generator_real_without_key_is_loud() {
    if !env_key_absent() {
        eprintln!("SKIP: BID_LLM_API_KEY 在场（real 有配置可用——走 real_api_smoke 面）");
        return;
    }
    let (mut svc, pid, leaf) = svc_ready("gen-real");
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "generator": "real",
    }));
    let (code, msg) = err_parts(&reply);
    assert_eq!(code, "E_UNPROCESSABLE");
    assert!(msg.contains("api_key"), "{msg}");
    assert!(msg.contains("BID_LLM_API_KEY") || msg.contains(".mang/llm.json"), "{msg}");
}

#[test]
fn generator_auto_degrades_to_mock_with_warning() {
    if !env_key_absent() {
        eprintln!("SKIP: BID_LLM_API_KEY 在场（auto 走真实生成——降级面不适用）");
        return;
    }
    let (mut svc, pid, leaf) = svc_ready("gen-auto");
    let r = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, // generator 缺省 = auto
    })));
    assert_eq!(r["stats"]["generator"], "mock");
    assert_eq!(r["section_status"], "DRAFTED");
    // 降级告警在场（Agent 感知面）
    let warns = r["warnings"].as_array().unwrap();
    assert!(warns.iter().any(|w| w.as_str().unwrap().contains("降级 mock")),
        "auto 无 key 应有降级告警: {warns:?}");
}
