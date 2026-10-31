//! #68 阶段 1 · draft_response_section 生成器切换 + #100 任务 2 external 四态。
//!
//! 验收面（issue #68 §1.2 + issue #100 任务 2）：
//! - `generator` 参数校验：非 auto/mock/real/external → E_UNPROCESSABLE；
//! - `mock` 强制：无 key 环境照常出稿，stats.generator=mock，无降级告警；
//! - `real` 无 key：E_UNPROCESSABLE 且错误说明配置面（显式要求要响亮）；
//! - `auto` 无 key：降级 mock + 告警「未配置 api_key」（Agent 可感知）；
//! - `external`：content 手稿直注（引擎不生成只提交）——互斥/引用回查/
//!   幂等指纹与 mock/real 同提交面；out.stats.generator 字段在场。
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

// ---------------------------------------------------------------- #100 任务 2：external 态

/// external 主链：外部手稿直注 → 提交面与 mock/real 完全一致
/// （stats.generator=external、DRAFTED、store 正文逐字落位、引用回查照跑）。
#[test]
fn external_manuscript_roundtrips_through_submission() {
    let (mut svc, pid, leaf) = svc_ready("gen-ext-main");
    let manuscript = "## 售后服务承诺\n\n我方针对 [REQ-0001] 建立三级售后保障体系：\n\
        7×24 小时热线响应，2 小时内远程响应，8 小时内到场处置。\n\
        质保期内免费提供系统升级与巡检服务。";
    let r = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf,
        "generator": "external", "content": manuscript,
    })));
    assert_eq!(r["stats"]["generator"], "external", "stats.generator 必须如实登记 external");
    assert_eq!(r["section_status"], "DRAFTED");
    // 正文逐字落位（引擎不生成只提交——无 mock 模板痕迹）
    let stored = svc.store().sections().iter().find(|n| n.id == leaf).unwrap();
    assert_eq!(stored.content.as_deref(), Some(manuscript), "外部手稿必须逐字落库");
    assert_eq!(r["stats"]["citations"], 1, "[REQ-0001] 引用被回查登记");
    // external 模式告警在场（Agent 感知：不生成只提交）
    let warns = r["warnings"].as_array().unwrap();
    assert!(warns.iter().any(|w| w.as_str().unwrap().contains("external 模式")),
        "external 模式说明告警应在场: {warns:?}");
}

/// external 无手稿：响亮拒（要响亮不要猜——无手稿的 external 无意义）。
#[test]
fn external_without_content_is_loud() {
    let (mut svc, pid, leaf) = svc_ready("gen-ext-naked");
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "generator": "external",
    }));
    let (code, msg) = err_parts(&reply);
    assert_eq!(code, "E_UNPROCESSABLE");
    assert!(msg.contains("content"), "{msg}");
}

/// 双生成源互斥：content 与 generator=mock/real 同场 → 响亮拒
/// （静默择一会埋下「手稿被模板覆盖」的不可见事故）。
#[test]
fn content_conflicts_with_mock_and_real() {
    let (mut svc, pid, leaf) = svc_ready("gen-ext-conflict");
    for g in ["mock", "real"] {
        let reply = call(&mut svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": &leaf,
            "generator": g, "content": "手稿",
        }));
        let (code, msg) = err_parts(&reply);
        assert_eq!(code, "E_UNPROCESSABLE", "generator={g} + content 必须拒");
        assert!(msg.contains("互斥"), "{msg}");
    }
    // 空白手稿也拒（空稿无提交意义）
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "content": "   ",
    }));
    let (code, msg) = err_parts(&reply);
    assert_eq!(code, "E_UNPROCESSABLE");
    assert!(msg.contains("空白"), "{msg}");
}

/// content 在场即 external 语义（generator 缺省/auto 不冲突）。
#[test]
fn content_implies_external_even_without_generator_param() {
    let (mut svc, pid, leaf) = svc_ready("gen-ext-implicit");
    for gv in [None, Some("auto"), Some("external")] {
        let mut args = json!({
            "project_id": &pid, "section_id": &leaf,
            "content": "外部手稿（含 [REQ-0001] 引用）",
        });
        if let Some(g) = gv {
            args["generator"] = json!(g);
        }
        let r = ok_result(call(&mut svc, "draft_response_section", args));
        assert_eq!(r["stats"]["generator"], "external", "generator={gv:?} + content → external");
    }
}

/// external 稿幂等指纹：同章节同指令不同稿 = 不同请求（不误重放）；
/// 同稿重提 = deduplicated（content 进幂等键——否则换稿被首次结果覆盖）。
#[test]
fn external_manuscript_fingerprinted_in_idempotency_key() {
    let (mut svc, pid, leaf) = svc_ready("gen-ext-idem");
    let call_ext = |svc: &mut BidService, content: &str| {
        ok_result(call(svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": &leaf,
            "generator": "external", "content": content,
        })))
    };
    let r1 = call_ext(&mut svc, "第一版手稿，含 [REQ-0001]");
    assert_eq!(r1["version"], 1);
    // 换稿：不同指纹 → 不 deduplicated，新 attempt 落库
    let r2 = call_ext(&mut svc, "第二版手稿（改写），含 [REQ-0001]");
    assert_eq!(r2.get("deduplicated"), None, "不同手稿不得命中首次结果");
    assert_eq!(r2["version"], 2);
    let stored = svc.store().sections().iter().find(|n| n.id == leaf).unwrap();
    assert_eq!(stored.content.as_deref(), Some("第二版手稿（改写），含 [REQ-0001]"));
    // 同稿重提：指纹命中 → deduplicated 直返
    let r3 = call_ext(&mut svc, "第一版手稿，含 [REQ-0001]");
    assert_eq!(r3["deduplicated"], true, "同手稿重提应命中幂等缓存");
    assert_eq!(r3["version"], 1, "重放返回首次结果");
}

/// external 稿的引用回查照跑：幻觉引用被剔除 + 告警（提交面与 mock/real 一致）。
#[test]
fn external_manuscript_citation_check_strips_hallucinations() {
    let (mut svc, pid, leaf) = svc_ready("gen-ext-halluc");
    let r = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf,
        "generator": "external",
        "content": "我方针对 [REQ-0001] 作出承诺，并援引 [REQ-9999] 的业绩证明。",
    })));
    let warns = r["warnings"].as_array().unwrap();
    assert!(warns.iter().any(|w| {
        let ws = w.as_str().unwrap_or("");
        ws.contains("REQ-9999") && ws.contains("幻觉")
    }), "幻觉引用告警应在场: {warns:?}");
    let stored = svc.store().sections().iter().find(|n| n.id == leaf).unwrap();
    let landed = stored.content.as_deref().unwrap();
    assert!(landed.contains("[REQ-0001]"), "真引用保留");
    assert!(!landed.contains("REQ-9999"), "幻觉引用必须被剔除: {landed}");
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
