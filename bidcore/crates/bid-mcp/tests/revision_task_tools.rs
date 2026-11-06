//! #321（A-23）：RT 生命周期 + 分工 + 章节重开 + 参数应答工具族测试。
//!
//! 背景（audit A-23）：assign/advance/complete_revision_task 域层完备但
//! 服务层零包装、MCP 面零工具——拒审派生 Open RT 后无任何产品出口，
//! export_allowed 恒拒 = 结构性死锁。本文件钉住：
//!
//! 1. RT 状态机工具面全链（认领→开工→关单）与非法转移拒绝；
//! 2. 「拒审→派生→处置→导出」全链 e2e（死锁面打通的直接证据）；
//! 3. 分工面（batch_task/advance_task：upsert 语义 + 线性推进）；
//! 4. reopen_section（ACCEPTED 重开 + 非终态拒）；
//! 5. set_param_responses（写入 + 空表拒 + 域层状态窗一致）。

use bid_domain::outline::{OutlineSpec, SectionStatus};
use bid_domain::review::{Decision, Gate};
use bid_domain::{Anchor, ParseStatus, ProjectStatus, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use bid_tools::service::review_gate_scope;
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

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn err_msg(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => format!("{error:?}"),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn mk_req(id: &str, category: &str, excerpt: &str) -> Requirement {
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

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-rt321-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 起步夹具：create → import → extract → outline+matrix → 全叶起草 → DRAFTING。
fn svc_drafted(tag: &str) -> (BidService, String, std::path::PathBuf) {
    let dir = tmp_dir(tag);
    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "RT 工具族测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", &format!("sha-{tag}"), 10, 1, 50, 0, ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "投标人须具备水利水电工程施工总承包贰级及以上资质"),
        mk_req("REQ-0002", "技术参数", "服务器内存容量不低于 128GB"),
    ]).unwrap();
    let _ = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 1_500, max_reqs_per_leaf: 8 })
        .unwrap();
    // 资格证据登记+绑定（readiness 闸：无证硬写等于虚假响应——与 export_tool
    // 夹具同构）
    use bid_domain::evidence::EvidenceSpec;
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "水利水电工程施工总承包贰级资质证书".into(),
        source_file: "EV-01.pdf".into(),
        source_sha256: format!("sha-ev-{tag}"),
        locator: EngineLocator::DocxParagraph { paragraph_index: 3, text_fingerprint: "fnv64:1a".into() },
        fingerprint: "fnv64:1a".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某建设工程有限公司".into(),
        scope: "水利水电".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    svc.store_mut().bind_evidence(&eid, "REQ-0001", "2026-12-31").unwrap();
    svc.store_mut().bind_evidence(&eid, "REQ-0002", "2026-12-31").unwrap();
    let _ = svc.store_mut().build_matrix().unwrap();
    // 无补遗确认门（L3 阻塞的人工出口——与 export_tool 夹具同构，直接走
    // 域层 record_review 面免 receipt 编排）
    svc.store_mut()
        .record_review(Gate::AddendumWaiver, &bid_domain::TrustedPrincipal::human("lead"), Decision::Approved, vec![], None)
        .unwrap_or_else(|e| panic!("无补遗确认门失败（{e:?}）"));
    let leaf_ids: Vec<String> = {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        svc.store().sections().iter()
            .filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect()
    };
    assert!(!leaf_ids.is_empty());
    for sid in &leaf_ids {
        svc.draft_section(sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Drafting);
    (svc, pid, dir)
}

/// 过门（Approved 无发现）。
fn pass_gate(svc: &mut BidService, gate: Gate) {
    let receipt = svc.issue_approval_receipt("lead", &review_gate_scope(gate));
    let g = gate.as_str();
    call(svc, "record_review_gate", json!({
        "project_id": svc.store().project().unwrap().id,
        "gate": g, "decision": "Approved", "receipt": receipt, "findings": [],
    }));
}

/// 拒审（Rejected + 1 条 BLOCKER）→ 返回派生的 RT id。
/// 前置：先过 matrix/outline 两门（pre_export_confirm 有门序强制）。
fn reject_with_blocker(svc: &mut BidService, gate: Gate, code: &str) -> String {
    if !svc.store().gate_approved(Gate::MatrixApproval) {
        pass_gate(svc, Gate::MatrixApproval);
    }
    if !svc.store().gate_approved(Gate::OutlineApproval) {
        pass_gate(svc, Gate::OutlineApproval);
    }
    let receipt = svc.issue_approval_receipt("lead", &review_gate_scope(gate));
    let g = gate.as_str();
    let r = ok_result(call(svc, "record_review_gate", json!({
        "project_id": svc.store().project().unwrap().id,
        "gate": g, "decision": "Rejected", "receipt": receipt,
        "findings": [{ "severity": "BLOCKER", "code": code, "message": "资质章节未附证书扫描件" }],
    })));
    let rts = r["receipt"]["spawned_rt_ids"].as_array().unwrap().clone();
    assert_eq!(rts.len(), 1, "BLOCKER 必须派生 1 条 RT: {r}");
    rts[0].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------- 1. RT 状态机工具面

/// 合法链：Open → assign → IN_PROGRESS → complete(说明) → DONE。
#[test]
fn rt_lifecycle_happy_path() {
    let (mut svc, pid, _dir) = svc_drafted("rt-happy");
    let rt_id = reject_with_blocker(&mut svc, Gate::PreExportConfirm, "E_CERT_MISSING");

    // 认领
    let r = ok_result(call(&mut svc, "assign_revision", json!({
        "project_id": pid, "rt_id": rt_id, "owner": "agent-9707",
    })));
    assert_eq!(r["rt"]["owner"], "agent-9707");
    assert_eq!(r["rt"]["status"], "OPEN", "认领不改状态（改派面≠进度面）");

    // 开工
    let r2 = ok_result(call(&mut svc, "advance_revision", json!({
        "project_id": pid, "rt_id": rt_id, "to": "IN_PROGRESS",
    })));
    assert_eq!(r2["rt"]["status"], "IN_PROGRESS");

    // 关单（带说明）
    let r3 = ok_result(call(&mut svc, "complete_revision", json!({
        "project_id": pid, "rt_id": rt_id, "resolution": "已补附证书扫描件并更新第 3.2 节",
    })));
    assert_eq!(r3["rt"]["status"], "DONE");
    assert_eq!(r3["open_rt_count"], 0, "全部关单");
    assert!(
        r3["export_allowed_hint"].as_str().unwrap().contains("放行"),
        "即时回显导出面: {r3}"
    );
}

/// 非法转移矩阵：Open 直接关单拒 / advance 到 DONE 拒 / 终态改派拒 / 终态推进拒 /
/// 不存在的 RT → E_NOTFOUND。
#[test]
fn rt_illegal_transitions_rejected() {
    let (mut svc, pid, _dir) = svc_drafted("rt-illegal");
    let rt_id = reject_with_blocker(&mut svc, Gate::PreExportConfirm, "E_X1");

    // Open 直接关单 = 越级（先认领开工）
    let reply = call(&mut svc, "complete_revision", json!({
        "project_id": pid, "rt_id": rt_id, "resolution": "跳步关单",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
    assert!(err_msg(&reply).contains("先 advance 开工"), "{}", err_msg(&reply));

    // advance 直接到 DONE = 拒（关单必须带 resolution——走 complete）
    let reply2 = call(&mut svc, "advance_revision", json!({
        "project_id": pid, "rt_id": rt_id, "to": "DONE",
    }));
    assert_eq!(err_code(&reply2), "E_UNPROCESSABLE");
    assert!(err_msg(&reply2).contains("complete_revision"), "{}", err_msg(&reply2));

    // 走完合法链到 DONE
    ok_result(call(&mut svc, "assign_revision", json!({
        "project_id": pid, "rt_id": rt_id, "owner": "a",
    })));
    ok_result(call(&mut svc, "advance_revision", json!({
        "project_id": pid, "rt_id": rt_id, "to": "IN_PROGRESS",
    })));
    ok_result(call(&mut svc, "complete_revision", json!({
        "project_id": pid, "rt_id": rt_id, "resolution": "修完",
    })));

    // 终态改派 = 拒（重走门 = 新 finding 新 RT，不复活旧单）
    let reply3 = call(&mut svc, "assign_revision", json!({
        "project_id": pid, "rt_id": rt_id, "owner": "b",
    }));
    assert_eq!(err_code(&reply3), "E_UNPROCESSABLE");
    assert!(err_msg(&reply3).contains("不可改派"), "{}", err_msg(&reply3));

    // 终态推进 = 拒
    let reply4 = call(&mut svc, "advance_revision", json!({
        "project_id": pid, "rt_id": rt_id, "to": "IN_PROGRESS",
    }));
    assert_eq!(err_code(&reply4), "E_UNPROCESSABLE");

    // 不存在
    let reply5 = call(&mut svc, "assign_revision", json!({
        "project_id": pid, "rt_id": "RT-9999", "owner": "a",
    }));
    assert_eq!(err_code(&reply5), "E_NOTFOUND");

    // 状态串非法
    let reply6 = call(&mut svc, "advance_revision", json!({
        "project_id": pid, "rt_id": rt_id, "to": "DONE!",
    }));
    assert_eq!(err_code(&reply6), "E_UNPROCESSABLE");
    assert!(err_msg(&reply6).contains("OPEN|IN_PROGRESS|DONE"));
}

// ---------------------------------------------------------------- 2. 拒审→派生→处置→导出 全链 e2e

/// #321 核心验收：拒审派生 RT → 导出被 RT 面拦 → 认领/开工/关单 → 导出放行。
/// （修复前：RT 无工具出口——export_allowed 恒拒，唯一出路是手改 db.json。）
#[test]
fn e2e_reject_spawn_resolve_export() {
    let (mut svc, pid, _dir) = svc_drafted("rt-e2e");
    // 三门先过（Approved 无发现）
    pass_gate(&mut svc, Gate::MatrixApproval);
    pass_gate(&mut svc, Gate::OutlineApproval);
    pass_gate(&mut svc, Gate::PreExportConfirm);

    // 二轮拒审：PreExportConfirm Rejected + BLOCKER → 派生 RT（Approved 不被撤回）
    let rt_id = reject_with_blocker(&mut svc, Gate::PreExportConfirm, "E_CERT_MISSING");
    assert!(svc.store().project().unwrap().status == ProjectStatus::Drafting);

    // 死锁面直接证据：导出被 RT 面拦（三门齐、内容未变——只剩 RT 未关单）
    let blocked = call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/标书.docx",
    }));
    assert_eq!(err_code(&blocked), "E_UNPROCESSABLE");
    assert!(
        err_msg(&blocked).contains("RevisionTask 未关单"),
        "必须是 RT 面拦截（三门已过、内容未变）: {}",
        err_msg(&blocked)
    );

    // 处置：认领 → 开工 → 关单
    ok_result(call(&mut svc, "assign_revision", json!({
        "project_id": pid, "rt_id": rt_id, "owner": "agent-9707",
    })));
    ok_result(call(&mut svc, "advance_revision", json!({
        "project_id": pid, "rt_id": rt_id, "to": "IN_PROGRESS",
    })));
    let done = ok_result(call(&mut svc, "complete_revision", json!({
        "project_id": pid, "rt_id": rt_id, "resolution": "补附资质证书扫描件（EV-0001）",
    })));
    assert_eq!(done["open_rt_count"], 0);

    // 导出放行（RT 面清零 → 全门通过 → EXPORTED）
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/标书.docx",
    })));
    assert_eq!(r["project_status"], "EXPORTED", "死锁面已打通");
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Exported);
}

// ---------------------------------------------------------------- 3. 分工面

#[test]
fn batch_task_and_advance_task() {
    let (mut svc, pid, _dir) = svc_drafted("task-batch");
    // 派工：两章一条款
    let sec1 = svc.store().sections()[0].id.clone();
    let r = ok_result(call(&mut svc, "batch_task", json!({
        "project_id": pid, "targets": [sec1, "REQ-0002"], "owner": "张工", "deadline": "2026-12-20",
    })));
    assert_eq!(r["total"], 2);
    assert_eq!(r["added"], 2);

    // 同 owner 重批 = 改期不改进度（upsert 幂等）
    let r2 = ok_result(call(&mut svc, "batch_task", json!({
        "project_id": pid, "targets": ["REQ-0002"], "owner": "张工", "deadline": "2026-12-25",
    })));
    assert_eq!(r2["added"], 0, "同 owner 重批 = no-op add（进度轴不动）");

    // 推进：ASSIGNED → IN_PROGRESS
    let r3 = ok_result(call(&mut svc, "advance_task", json!({
        "project_id": pid, "target_id": "REQ-0002", "to": "IN_PROGRESS",
    })));
    assert_eq!(r3["task"]["status"], "IN_PROGRESS");
    assert_eq!(r3["task"]["owner"], "张工");

    // 越级：ASSIGNED 直接 DONE = 拒（sec1 未开工）
    let reply = call(&mut svc, "advance_task", json!({
        "project_id": pid, "target_id": sec1, "to": "DONE",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");

    // 走完链
    ok_result(call(&mut svc, "advance_task", json!({
        "project_id": pid, "target_id": "REQ-0002", "to": "DONE",
    })));

    // 空批 = 拒；非字符串数组 = 拒
    let reply2 = call(&mut svc, "batch_task", json!({
        "project_id": pid, "targets": [], "owner": "x",
    }));
    assert_eq!(err_code(&reply2), "E_UNPROCESSABLE");
    let reply3 = call(&mut svc, "batch_task", json!({
        "project_id": pid, "targets": [1, 2], "owner": "x",
    }));
    assert_eq!(err_code(&reply3), "E_UNPROCESSABLE");

    // list_tasks 可见分工（既有只读面——接线一致性）
    let lt = ok_result(call(&mut svc, "list_tasks", json!({
        "project_id": pid, "filters": { "owner": "张工" },
    })));
    assert_eq!(lt["total"], 2, "分工账与 list_tasks 同源: {lt}");
}

// ---------------------------------------------------------------- 4. reopen_section

#[test]
fn reopen_section_tool() {
    let (mut svc, pid, _dir) = svc_drafted("reopen");
    // 取**叶**章节（叶被起草为 Drafted；父节点停在 Outlining/Pending——
    // reopen 前置 ACCEPTED 只对叶可达）
    let sec = {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        svc.store().sections().iter()
            .find(|n| !parents.contains(&n.id)).unwrap().id.clone()
    };
    let reply = call(&mut svc, "reopen_section", json!({
        "project_id": pid, "section_id": sec,
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
    assert!(err_msg(&reply).contains("ACCEPTED"), "{}", err_msg(&reply));

    // 域层推进到 ACCEPTED（Drafted→Validating→Accepted——本测试的编排前置）
    let st = svc.store_mut();
    st.advance_section_status(&sec, SectionStatus::Validating).unwrap();
    st.advance_section_status(&sec, SectionStatus::Accepted).unwrap();
    drop(st);

    // 重开 → REVISING + dirty
    let r = ok_result(call(&mut svc, "reopen_section", json!({
        "project_id": pid, "section_id": sec,
    })));
    assert_eq!(r["section"]["status"], "REVISING");
    assert_eq!(r["section"]["dirty"], true);

    // 不存在
    let reply2 = call(&mut svc, "reopen_section", json!({
        "project_id": pid, "section_id": "SEC-9999",
    }));
    assert_eq!(err_code(&reply2), "E_NOTFOUND");
}

// ---------------------------------------------------------------- 5. set_param_responses

#[test]
fn set_param_responses_tool() {
    let (mut svc, pid, _dir) = svc_drafted("param-resp");
    let r = ok_result(call(&mut svc, "set_param_responses", json!({
        "project_id": pid,
        "rows": [
            { "requirement_id": "REQ-0002", "param_name": "内存容量",
              "required_value": "不低于 128GB", "response_value": "256GB" },
            { "requirement_id": "REQ-0002", "param_name": "硬盘",
              "required_value": "2TB", "response_value": "2TB", "evidence_id": null },
        ],
    })));
    assert_eq!(r["total"], 2);
    assert_eq!(r["added"], 2);

    // 重放幂等（同键覆盖 add 计零）
    let r2 = ok_result(call(&mut svc, "set_param_responses", json!({
        "project_id": pid,
        "rows": [
            { "requirement_id": "REQ-0002", "param_name": "内存容量",
              "required_value": "不低于 128GB", "response_value": "256GB" },
        ],
    })));
    assert_eq!(r2["added"], 0, "幂等重放");

    // 空表 = 拒
    let reply = call(&mut svc, "set_param_responses", json!({
        "project_id": pid, "rows": [],
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");

    // 缺必填字段 = 拒（逐字段点名）
    let reply2 = call(&mut svc, "set_param_responses", json!({
        "project_id": pid,
        "rows": [{ "requirement_id": "REQ-0002", "param_name": "内存" }],
    }));
    assert_eq!(err_code(&reply2), "E_UNPROCESSABLE");
    assert!(err_msg(&reply2).contains("required_value"), "{}", err_msg(&reply2));
}

// ---------------------------------------------------------------- 6. 状态窗

/// RT 工具族在 FILES_IMPORTED（无 RT 可言）→ E_STATE（状态机门）。
#[test]
fn rt_tools_state_gated() {
    let dir = tmp_dir("rt-gate");
    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "状态窗测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "rt-gate",
    })))["project_id"].as_str().unwrap().to_string();
    let reply = call(&mut svc, "assign_revision", json!({
        "project_id": pid, "rt_id": "RT-0001", "owner": "a",
    }));
    assert_eq!(err_code(&reply), "E_STATE", "FILES_IMPORTED 不可认领 RT（状态窗裁决）");
}
