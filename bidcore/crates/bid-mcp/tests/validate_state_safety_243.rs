//! validate_state_safety_243.rs —— #243 校验状态安全与恢复（MCP 工具层 + 全序 e2e）。
//!
//! 验收对照（issue #243 验收条件逐条落测）：
//! 1. 部分章节状态下全局校验 → E_DRAFT_INCOMPLETE + missing 清单；状态/
//!    版本/审核记录均不变（`incomplete_draft_guard_is_side_effect_free`）；
//! 2. 全稿校验失败仍可修订重试，修复后可导出（`failure_retry_then_export`
//!    ——含「未完成校验被拒绝 → 继续起草 → 完成全稿 → 校验 → 导出」全序）；
//! 3. 持久化重启后行为一致（`persistence_restart_holds_guard_and_recovery`）；
//! 4. 重复请求不落死锁态（守卫重试零副作用 + 恢复重试被状态门拒）；
//! 5. 历史异常 VALIDATING 态的受支持恢复（`recover_tool_via_mcp`——审计
//!    记录 + 恢复后起草面解锁 + 状态不变量）。
//!
//! 夹具纪律：走真实状态机与服务/工具调用；历史异常态经合法前向链构造
//!（状态机不校验草稿完成度——旧版 validate 正是这么误推进的）。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineSpec;
use bid_domain::{Anchor, ParseStatus, ProjectStatus, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolError, ToolReply};
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

fn mk_req(id: &str, category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
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

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-243-mcp-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 叶清单（文档序）。
fn leaf_ids(svc: &BidService) -> Vec<String> {
    let parents: std::collections::HashSet<String> =
        svc.store().sections().iter().filter_map(|n| n.parent_id.clone()).collect();
    svc.store().sections().iter().filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect()
}

/// 未完成叶清单（与域层 incomplete_leaf_ids 同口径——断言用真源）。
fn incomplete(svc: &BidService) -> Vec<String> {
    svc.store().incomplete_leaf_ids()
}

/// 建项目：两叶（资格组 + 格式组）+ 台账资质（登记不绑——readiness 放行、
/// 矩阵未响应留给校验面报告）。
fn build_two_leaf_project(tag: &str) -> (BidService, String) {
    let dir = tmp_dir(tag);
    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "243 工具层", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": &format!("ext-243-mcp-{tag}"),
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut()
        .upsert_file("A-招标文件.docx", "招标文件", &format!("sha-243-{tag}"), 10, 1, 50, 0, ParseStatus::Ok, "")
        .unwrap();
    svc.store_mut()
        .upsert_file("C-补遗文件01.docx", "补遗", &format!("sha-243-{tag}-c"), 5, 0, 20, 0, ParseStatus::Ok, "")
        .unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "具备水利水电工程施工总承包贰级资质"),
        mk_req("REQ-0002", "格式要求", "正文用仿宋三号字"),
    ])
    .unwrap();
    svc.prepare_outline_and_matrix(OutlineSpec { target_total_chars: 2_000, max_reqs_per_leaf: 8 })
        .unwrap();
    let _ = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: "水利水电工程施工总承包贰级资质证书".into(),
        source_file: "EV-ledger.pdf".into(),
        source_sha256: format!("sha-243-{tag}-lg"),
        locator: EngineLocator::DocxParagraph { paragraph_index: 7, text_fingerprint: "fnv64:lg".into() },
        fingerprint: "fnv64:lg".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某公司".into(),
        scope: "水利水电".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    });
    (svc, pid)
}

fn draft_leaf(svc: &mut BidService, pid: &str, sid: &str) -> Value {
    ok_result(call(svc, "draft_response_section", json!({
        "project_id": pid, "section_id": sid,
        "instructions": "按招标文件逐项响应，突出同类业绩与实施方案",
    })))
}

// ---------------------------------------------------------------- 1. 前置闸（零副作用）

#[test]
fn incomplete_draft_guard_is_side_effect_free() {
    let (mut svc, pid) = build_two_leaf_project("guard");
    let leaves = leaf_ids(&svc);
    // 只起草首叶（缺章态）
    draft_leaf(&mut svc, &pid, &leaves[0]);
    let status_before = svc.store().project().unwrap().status;
    let version_before = svc.store().content_version();
    let reviews_before = svc.store().reviews().to_vec();
    let missing_before = incomplete(&svc);
    assert_eq!(missing_before.len(), 1, "夹具：恰缺一叶");

    // 全稿校验 → E_DRAFT_INCOMPLETE + missing 清单
    let reply = call(&mut svc, "validate_bid_document", json!({ "project_id": pid, "scope": "all" }));
    match &reply {
        ToolReply::Err { error, .. } => match error {
            ToolError::EDraftIncomplete { missing } => {
                assert_eq!(*missing, missing_before, "missing = 未完成叶清单（可操作）");
                // 线形状（structuredContent 同源）：tag=code + missing 数组
                let wire = serde_json::to_value(error).unwrap();
                assert_eq!(wire["code"], "E_DRAFT_INCOMPLETE");
                assert_eq!(wire["missing"], json!(missing_before), "missing 随错误载荷透传");
            }
            other => panic!("预期 E_DRAFT_INCOMPLETE，实际 {other}"),
        },
        ToolReply::Ok(out) => panic!("缺章全稿校验不得成功: {}", out.result),
    }
    // 零副作用：状态/版本/审核记录均不变
    assert_eq!(svc.store().project().unwrap().status, status_before);
    assert_eq!(svc.store().content_version(), version_before);
    assert_eq!(svc.store().reviews(), reviews_before.as_slice());
    // 重复请求同样零副作用（重试安全）
    let reply2 = call(&mut svc, "validate_bid_document", json!({ "project_id": pid, "scope": "all" }));
    assert_eq!(err_code(&reply2), "E_DRAFT_INCOMPLETE");
    assert_eq!(svc.store().content_version(), version_before);

    // 起草面照常可用（与旧缺陷的差异面：VALIDATING 死锁不再发生）
    let second = leaves.iter().find(|s| **s != leaves[0]).unwrap().clone();
    draft_leaf(&mut svc, &pid, &second);
    assert!(incomplete(&svc).is_empty(), "补起草后完成");
    // 起草期间的章节级检查（scope=sections）不受前置闸
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": pid, "scope": "sections", "section_ids": [leaves[0]],
    })));
    assert_eq!(r["project_status"], "DRAFTING", "章节级检查不推进");
}

// ---------------------------------------------------------------- 2. 全序 e2e：拒 → 起草 → 校验 → 导出

#[test]
fn full_order_rejected_then_draft_validate_export() {
    let (mut svc, pid) = build_two_leaf_project("full-order");
    let leaves = leaf_ids(&svc);
    // ① 起草一半误触发全局校验 → 被拒（无副作用，起草可继续）
    draft_leaf(&mut svc, &pid, &leaves[0]);
    let v0 = svc.store().content_version();
    let reply = call(&mut svc, "validate_bid_document", json!({ "project_id": pid, "scope": "all" }));
    assert_eq!(err_code(&reply), "E_DRAFT_INCOMPLETE", "① 未完成校验被拒绝");
    assert_eq!(svc.store().content_version(), v0, "① 零副作用");

    // ② 继续起草 → 完成全稿（草稿期先过前两门：矩阵门/大纲门的状态窗
    //    在 DRAFTING 内——门序与 e2e_pipeline 同序）
    for sid in leaves.iter().skip(1) {
        draft_leaf(&mut svc, &pid, sid);
    }
    assert!(incomplete(&svc).is_empty(), "② 全稿完成");
    for (gate, actor) in [("matrix_approval", "243-matrix"), ("outline_approval", "243-outline")] {
        let receipt = svc.issue_approval_receipt(actor, &format!("review_gate:{gate}"));
        let _g = ok_result(call(&mut svc, "record_review_gate", json!({
            "project_id": pid, "gate": gate, "decision": "APPROVED",
            "receipt": receipt,
        })));
    }

    // ③ 全稿校验（矩阵未响应 → 失败面先走一轮：留 DRAFTING 可修订）
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({ "project_id": pid, "scope": "all" })));
    assert_eq!(r["ok"], json!(false), "③ 资格未响应（台账未绑）→ ok=false");
    assert_eq!(r["project_status"], "DRAFTING", "③ 失败不推进（可修订可重试）");
    // 修订：补绑证据（服务面——修订通道之一）
    let eid = svc.store().evidences()[0].id.clone();
    svc.store_mut().bind_evidence(&eid, "REQ-0001", "2030-12-31").unwrap();
    let (_rows, blocking) = svc.store_mut().build_matrix().unwrap();
    assert_eq!(blocking, 0);

    // ④ 校验通过 → VALIDATING
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({ "project_id": pid, "scope": "all" })));
    assert_eq!(r["ok"], json!(true), "④ 修订后全绿: {}", r["errors"]);
    assert_eq!(r["project_status"], "VALIDATING", "④ 通过才推进");
    let version_after_advance = svc.store().content_version();
    assert_eq!(r["project_version"].as_u64().unwrap(), version_after_advance, "④ 响应携带新版本号（契约 §0）");
    // 重复校验：幂等（无变迁不落版本）
    let r2 = ok_result(call(&mut svc, "validate_bid_document", json!({ "project_id": pid, "scope": "all" })));
    assert_eq!(r2["counts"], r["counts"]);
    assert_eq!(svc.store().content_version(), version_after_advance, "重验不推进");

    // ⑤ 导出（PreExportConfirm 记审 + 导出门——#242 篇幅门语义不放宽；
    //    前两门已在草稿期过审）
    let receipt = svc.issue_approval_receipt("243-lead", "review_gate:pre_export_confirm");
    let _g = ok_result(call(&mut svc, "record_review_gate", json!({
        "project_id": pid, "gate": "pre_export_confirm", "decision": "APPROVED",
        "receipt": receipt,
    })));
    let e = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/投标文件-243.docx",
    })));
    assert_eq!(e["project_status"], "EXPORTED", "⑤ 修复后正常进入导出流程");
    let url = e["artifact"]["url"].as_str().unwrap();
    assert!(url.ends_with("投标文件-243.docx"), "⑤ 产物: {url}");
    assert!(e["artifact"]["size"].as_u64().unwrap() > 5_000, "⑤ 产物 >5KB");
    assert_eq!(e["chapters_written"].as_u64().unwrap(), leaves.len() as u64, "⑤ 全部落章");
}

// ---------------------------------------------------------------- 3. 历史异常态恢复（MCP 面）

#[test]
fn recover_tool_via_mcp_with_audit_and_repeat_safety() {
    let (mut svc, pid) = build_two_leaf_project("recover");
    let leaves = leaf_ids(&svc);
    // 历史异常态：缺章 + 合法链推进 VALIDATING（旧版 validate 的误推进形态）
    draft_leaf(&mut svc, &pid, &leaves[0]);
    svc.store_mut().advance_project_status(ProjectStatus::Validating).unwrap();
    // 死锁面在场（事故形态）
    let blocked = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": &leaves[1],
    }));
    assert_eq!(err_code(&blocked), "E_STATE", "历史异常态：VALIDATING 禁起草");

    let sec_before: Vec<_> = svc.store().sections().to_vec();
    let reviews_before = svc.store().reviews().to_vec();
    let version_before = svc.store().content_version();

    // 受支持恢复（MCP 工具面）
    let r = ok_result(call(&mut svc, "recover_validating_state", json!({
        "project_id": pid, "reason": "历史异常态：旧版校验在缺章时误推进 VALIDATING",
    })));
    assert_eq!(r["status"], "DRAFTING", "恢复后起草面解锁");
    assert_eq!(r["recovery"]["from"], "VALIDATING");
    assert_eq!(r["recovery"]["to"], "DRAFTING");
    assert_eq!(r["recovery"]["reason"], "历史异常态：旧版校验在缺章时误推进 VALIDATING");
    assert!(r["recovery"]["at"].as_u64().unwrap() > 0, "审计时刻落钟");
    assert!(
        r["recovery"]["missing_sections"].as_array().unwrap().len() == 1,
        "缺章快照随审计记录返回: {}",
        r["recovery"]["missing_sections"]
    );
    // 恢复记录在账（审计面——域层真源复核）
    assert_eq!(svc.store().state_recoveries().len(), 1);
    let rec = &svc.store().state_recoveries()[0];
    assert_eq!(rec.from, ProjectStatus::Validating);
    assert_eq!(rec.to, ProjectStatus::Drafting);
    assert!(!rec.missing_sections.is_empty(), "缺章快照入审计");
    // 状态不变量：章节树/审核记录原样；版本 +1
    assert_eq!(svc.store().sections(), sec_before.as_slice(), "恢复不动章节树");
    assert_eq!(svc.store().reviews(), reviews_before.as_slice(), "恢复不动审核记录");
    assert_eq!(svc.store().content_version(), version_before + 1, "恢复推进版本");
    assert_eq!(r["project_version"].as_u64().unwrap(), version_before + 1, "响应携带新版本号");

    // 起草面解锁（事故解除）
    let d = draft_leaf(&mut svc, &pid, &leaves[1]);
    assert_eq!(d["section_status"], "DRAFTED", "恢复后可续起草");

    // 重复请求/超时重试：状态门拒（E_STATE——非死锁态，项目照常可操作）
    let retry = call(&mut svc, "recover_validating_state", json!({
        "project_id": pid, "reason": "重复请求",
    }));
    assert_eq!(err_code(&retry), "E_STATE", "重复恢复被状态门拒");
    assert_eq!(svc.store().state_recoveries().len(), 1, "审计账不重复");
    // DRAFTING 态调用（非历史异常态）→ 状态门拒
    let wrong_state = call(&mut svc, "recover_validating_state", json!({
        "project_id": pid, "reason": "理由",
    }));
    assert_eq!(err_code(&wrong_state), "E_STATE");
    // 空白理由 → E_UNPROCESSABLE（参数面）
    let mut svc2 = {
        // 另一个历史异常态实例（验参数面）
        let (mut s, p) = build_two_leaf_project("recover-blank");
        let ls = leaf_ids(&s);
        draft_leaf(&mut s, &p, &ls[0]);
        s.store_mut().advance_project_status(ProjectStatus::Validating).unwrap();
        let reply = call(&mut s, "recover_validating_state", json!({
            "project_id": p, "reason": "   ",
        }));
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "空白理由拒");
        assert_eq!(s.store().project().unwrap().status, ProjectStatus::Validating, "拒绝零副作用");
        assert!(s.store().state_recoveries().is_empty(), "拒绝不入审计账");
        s
    };
    let _ = &mut svc2; // 保留实例（本用例内的断言已全部完成）
}

// ---------------------------------------------------------------- 4. 持久化重启

#[test]
fn persistence_restart_holds_guard_and_recovery() {
    let (mut svc, pid) = build_two_leaf_project("restart");
    let leaves = leaf_ids(&svc);
    draft_leaf(&mut svc, &pid, &leaves[0]);
    // 历史异常态 + 恢复（全部落盘）
    svc.store_mut().advance_project_status(ProjectStatus::Validating).unwrap();
    let _ = ok_result(call(&mut svc, "recover_validating_state", json!({
        "project_id": pid, "reason": "重启前恢复（留档）",
    })));
    let ws = svc.workspace_dir().unwrap();
    drop(svc);

    // 重启：恢复结果与审计账保持；守卫行为一致（缺章全稿校验仍拒）
    let mut svc2 = BidService::open(&ws).unwrap();
    assert_eq!(svc2.store().project().unwrap().status, ProjectStatus::Drafting, "恢复结果跨重启保持");
    assert_eq!(svc2.store().state_recoveries().len(), 1, "审计账跨重启保持");
    let reply = call(&mut svc2, "validate_bid_document", json!({ "project_id": pid, "scope": "all" }));
    assert_eq!(err_code(&reply), "E_DRAFT_INCOMPLETE", "重启后守卫仍在");
    // 续跑至全绿
    let leaves2 = leaf_ids(&svc2);
    for sid in &leaves2 {
        let r = draft_leaf(&mut svc2, &pid, sid);
        assert_eq!(r["section_status"], "DRAFTED", "重启后可续起草 {sid}");
    }
    let eid = svc2.store().evidences()[0].id.clone();
    svc2.store_mut().bind_evidence(&eid, "REQ-0001", "2030-12-31").unwrap();
    let (_rows, blocking) = svc2.store_mut().build_matrix().unwrap();
    assert_eq!(blocking, 0);
    let r = ok_result(call(&mut svc2, "validate_bid_document", json!({ "project_id": pid, "scope": "all" })));
    assert_eq!(r["ok"], json!(true), "重启后全绿: {}", r["errors"]);
    assert_eq!(r["project_status"], "VALIDATING");
}
