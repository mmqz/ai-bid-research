//! #59 阶段 1 · draft_response_section 壳层契约测试（bid-mcp tools.rs）。
//!
//! 验收面（契约 §6 + Lead #59 重排指令）：
//! - 状态机裁决：FILES_IMPORTED 越级 → E_STATE（既有 TC-11 语义保持）；
//! - 项目路由：错误 project_id → E_NOTFOUND；章节不存在 → E_NOTFOUND；
//! - 未建纲（无章节树）→ E_STATE（先建纲再起草）；
//! - 契约 out 形状：section_id / content_ref / version / stats{words,
//!   citations, unanchored_claims} / warnings / project_version；
//! - 幂等重放：同 (section_id, instructions, content_version) →
//!   deduplicated=true 且统计与首次全等；
//! - 乐观并发：project_version 过期 → E_CONFLICT；
//! - 参数校验：source_references 非字符串数组 → E_UNPROCESSABLE；
//! - 模板纪律：instructions 不落进任何硬编码模板位（每章差异由素材面承载）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use bid_domain::outline::OutlineSpec;
use serde_json::{json, Value};

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    // #68 阶段 1：确定性套件钉 mock（防 dev 机配了 BID_LLM_API_KEY 后 auto
    // 走真实调用——测试断言全部基于 mock 的确定性内容面）。
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut().unwrap().insert("generator".into(), serde_json::json!("mock"));
    }
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// 造一个「MATRIX_READY + 建纲」的服务态（经 MCP 工具走到抽取，再服务面建纲建矩阵）。
fn svc_ready(tag: &str) -> (BidService, String, String) {
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "壳层起草测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    // 服务面注入已解析产物（抽取输入——壳层测试不依赖真实文件解析）
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-shell", 10, 1, 50, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap(); // FILES_IMPORTED（状态门：抽取的前置）
    // 补遗件（#110 任务 1 后 readiness 的 IncompleteTender 只警告不阻塞——
    // 在场则警告面不进「零告警」断言）
    svc.store_mut().upsert_file(
        "C-补遗文件01.docx", "补遗", "sha-shell-add", 5, 0, 20, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "投标人须具备水利水电工程施工总承包贰级及以上资质"),
        mk_req("REQ-0002", "否决项", "投标文件未经法定代表人签字的作否决处理"),
        mk_req("REQ-0003", "评分项", "施工组织设计完整性 10 分"),
    ]).unwrap();
    let (n_sec, _, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 3_000, max_reqs_per_leaf: 2 })
        .unwrap();
    assert!(n_sec >= 2);
    // 素材面（#110 任务 1 缺失回报闸）：「资格条件」叶标题触发词面 → 台账须有
    // 可用资质；否决项 REQ-0002 须矩阵响应（绑定 + 重建）——否则 draft 拒生成
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: "水利水电工程施工总承包贰级资质证书".into(),
        source_file: "EV-shell.pdf".into(),
        source_sha256: "sha-shell-ev".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 3, text_fingerprint: "fnv64:se".into() },
        fingerprint: "fnv64:se".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某建设工程有限公司".into(),
        scope: "水利水电工程施工".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(), // #147
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    for rid in ["REQ-0001", "REQ-0002"] {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    // 取首个叶章节
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
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

// ---------------------------------------------------------------- 1. 状态机与路由

#[test]
fn draft_rejects_files_imported_escalation() {
    // 既有 TC-11 语义保持：FILES_IMPORTED 直接起草 → E_STATE
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "越级测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "esc",
    })))["project_id"].as_str().unwrap().to_string();
    // 服务面直接落一个文件（模拟导入完成——状态 FILES_IMPORTED）
    svc.store_mut().upsert_file(
        "A.docx", "招标文件", "sha-esc", 5, 0, 20, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": "SEC-0001",
    }));
    assert_eq!(err_code(&reply), "E_STATE", "FILES_IMPORTED 起草必须被状态机拒绝");
}

#[test]
fn draft_rejects_wrong_project() {
    let (mut svc, _, leaf) = svc_ready("wrong-proj");
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": "PRJ-9999", "section_id": leaf,
    }));
    assert_eq!(err_code(&reply), "E_NOTFOUND");
}

#[test]
fn draft_rejects_missing_section() {
    let (mut svc, pid, _) = svc_ready("missing-sec");
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": "SEC-9999",
    }));
    assert_eq!(err_code(&reply), "E_NOTFOUND");
}

#[test]
fn draft_rejects_no_outline() {
    // 矩阵已建但章节树为空 → 无起草对象（E_STATE 提示先建纲）
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "无纲测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "no-outline",
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A.docx", "招标文件", "sha-no-outline", 5, 0, 20, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap(); // FILES_IMPORTED（状态门：抽取的前置）
    svc.set_requirements(vec![mk_req("REQ-0001", "资格条件", "资质")]).unwrap();
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": "SEC-0001",
    }));
    let code = err_code(&reply);
    assert!(
        code == "E_STATE" || code == "E_UNPROCESSABLE",
        "无章节树的起草必须被拒（实际 {code}）——业务层给出建纲指引"
    );
}

// ---------------------------------------------------------------- 2. 契约 out 形状与主链

#[test]
fn draft_contract_shape_and_full_chain() {
    let (mut svc, pid, leaf) = svc_ready("shape");
    let r = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf, "instructions": "按招标文件逐项响应",
    })));
    // 契约 §6 out 键
    assert_eq!(r["section_id"], json!(leaf));
    assert!(r["content_ref"].as_str().unwrap().starts_with(&format!("DOC-{leaf}-v")), "content_ref 形态: {}", r["content_ref"]);
    assert_eq!(r["version"], 1);
    let stats = &r["stats"];
    assert!(stats["words"].as_u64().unwrap() > 0);
    assert!(stats["citations"].as_u64().unwrap() > 0, "响应面非空——mock 逐 REQ 引用");
    assert_eq!(stats["unanchored_claims"], 0, "mock 模板句句带锚");
    assert!(r["warnings"].as_array().unwrap().is_empty());
    assert_eq!(r["section_status"], "DRAFTED");
    assert_eq!(r["project_status"], "DRAFTING", "首调起草推进项目 DRAFTING");
    assert!(r["project_version"].as_u64().unwrap() > 0);
    assert!(!r["request_id"].as_str().unwrap_or("").is_empty(), "request_id 必须在位");
    // 章节配额面：字数接近 target ±20%（Lead 验收）
    let node = svc.store().sections().iter().find(|n| n.id == leaf).unwrap();
    let target = f64::from(node.target_chars.max(1));
    let chars = stats["words"].as_f64().unwrap();
    assert!(chars >= target * 0.80 && chars <= target * 1.20, "target={} 实际 {chars}", node.target_chars);
    // 覆盖面：正文含全部绑定 REQ（Lead 验收）
    let content = node.content.as_ref().unwrap();
    for rid in &node.requirement_ids {
        assert!(content.contains(rid.as_str()), "正文未覆盖 {rid}");
    }
}

#[test]
fn draft_idempotent_replay_returns_first_result() {
    let (mut svc, pid, leaf) = svc_ready("idem");
    let a = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf, "instructions": "稳定指令",
    })));
    let b = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf, "instructions": "稳定指令",
    })));
    assert_eq!(b["deduplicated"], json!(true), "同键重放必须标记 deduplicated");
    // 统计面全等（chars/citations/version 不重执行）
    assert_eq!(a["stats"], b["stats"]);
    assert_eq!(a["version"], b["version"]);
    assert_eq!(a["content_ref"], b["content_ref"]);
    // store 不再推进（attempt 保持 1）
    let node = svc.store().sections().iter().find(|n| n.id == leaf).unwrap();
    assert_eq!(node.content_attempt, 1);
}

#[test]
fn draft_version_conflict_typed() {
    let (mut svc, pid, leaf) = svc_ready("conflict");
    // 传过期 project_version → E_CONFLICT
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf, "project_version": 1,
    }));
    assert_eq!(err_code(&reply), "E_CONFLICT");
}

#[test]
fn draft_validates_param_types() {
    let (mut svc, pid, leaf) = svc_ready("params");
    // source_requirements 非数组 → 拒
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf, "source_requirements": "REQ-0001",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
    // source_requirements 含非字符串 → 拒
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf, "source_requirements": [1, 2],
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
    // 缺 project_id → 拒
    let reply = call(&mut svc, "draft_response_section", json!({ "section_id": leaf }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
}

#[test]
fn draft_override_requirements_and_ghost_warning() {
    let (mut svc, pid, leaf) = svc_ready("override");
    let r = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf,
        "source_requirements": ["REQ-0001", "REQ-9999"],
    })));
    // 幽灵 REQ 告警 + 有效响应面只剩 REQ-0001
    let warnings = r["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("REQ-9999")), "{warnings:?}");
    let cited: Vec<&str> = r["stats"]["cited_ids"].as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap()).collect();
    assert!(cited.contains(&"REQ-0001"));
    assert!(!cited.contains(&"REQ-9999"));
    assert!(!cited.iter().any(|c| c.starts_with("REQ-0002") || c.starts_with("REQ-0003")),
        "override 后非指定 REQ 不进响应面: {cited:?}");
}

// ---------------------------------------------------------------- 3. 契约工具面回归（22——#184 补遗逐件确认 + #226 否决项死锁出口）

#[test]
fn tool_face_remains_twenty_with_draft_implemented() {
    let v = bid_mcp::tools_schema();
    let arr = v["tools"].as_array().unwrap();
    // #145 时点 20；#184 增 record_addendum_waiver（补遗分级阻塞的逐件
    // 人工确认）→ 21；#219 增 3 覆盖工具 → 24；#226 增 set_matrix_override
    // （否决项死锁出口）→ 25；#243 增 recover_validating_state（历史
    // 异常态恢复出口）→ 26；#254 增分段三工具（append/rewrite/list
assert_eq!(arr.len(), 44, "工具面保持 44 个（21 + #219 三工具 + #226 死锁出口 + #243 恢复出口 + #254 分段三工具 + #279 发现面 + #287 篇幅档位推荐 + #285① ★漏标检测 + #285② 块级双向索引三工具 + #319 补遗后增量重写两工具 + #321 RT 生命周期/分工/重开/参数应答七工具）");
    let draft = arr.iter().find(|t| t["name"] == "draft_response_section").unwrap();
    assert_eq!(draft["implemented"], json!(true));
    // #145 任务 1：search_evidence 已接线——原「不得提前标实现」守卫随之撤销
    // （骨架纪律由 dispatch_contract 的 UNIMPLEMENTED 清单承载，见该文件）
    let search = arr.iter().find(|t| t["name"] == "search_evidence").unwrap();
    assert_eq!(search["implemented"], json!(true), "search_evidence 已接线（#145）");
    let exported = arr.iter().find(|t| t["name"] == "export_bid_document").unwrap();
    assert_eq!(exported["implemented"], json!(true), "export 已落地（阶段 2）");
    // #100 任务 3：评分三件已实现（B1/B2 接线）
    for name in ["score_price", "score_section", "score_document"] {
        let t = arr.iter().find(|t| t["name"] == name).unwrap();
        assert_eq!(t["implemented"], json!(true), "{name} 已接线（#100 任务 3）");
    }
}
