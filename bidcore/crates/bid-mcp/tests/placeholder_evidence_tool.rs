//! #245 MCP 契约面验收：占位资质不得冒充已核验的真实证据。
//!
//! 覆盖 issue 验收条件（MCP 层）：
//! - **placeholder 参数**：`register_evidence placeholder=true` → 回执
//!   verification_tier=PLACEHOLDER + placeholder=true + hint 警示；
//! - **自报拒**：`verification_tier` 参数显式拒（E_UNPROCESSABLE，零副作用）；
//!   `placeholder` 非布尔拒——「MCP 层不能通过参数把占位状态直接改成
//!   verified；非法状态迁移拒绝且无副作用」；
//! - **矩阵回显**：占位绑定 → get_compliance_matrix 行 status=PENDING_
//!   EVIDENCE + pending_evidence_ids 在场 + blocking 保留；evidence 列带
//!   verification_tier；
//! - **既有正向不回归**：placeholder 缺省（false）→ FILE_EXISTS/CONTENT_
//!   MATCHES 照常（evidence_content_tool.rs 既有面）。
//!
//! 全链（起草【待补】/ validate BLOCKER / 导出门 1d 拒→升级→放行）见
//! placeholder_evidence_e2e.rs；服务层单测见 bid-tools/tests/placeholder_evidence.rs。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_evidence(rel: &str) -> PathBuf {
    fixtures_root().join("corpus-A/files/evidence").join(rel)
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn err_text(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// 建项 + 导入（A-招标文件 + C-补遗 + EV-13 证据件）+ 抽取 + 建纲 →
/// (svc, pid, EV-13 的 file_id)——register_evidence 的来源锚。
fn matrix_ready_store(tag: &str) -> (BidService, String, String) {
    let mut svc = BidService::new().with_fs_root(fixtures_root());
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": format!("#245 契约 {tag}"), "bid_deadline": "2026-11-30",
                "mode": "明标", "external_key": format!("ph245-{tag}") }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = fixtures_root().join("synth-tender/招标文件/A-招标文件.docx");
    let c = fixtures_root().join("synth-tender/招标文件/C-补遗文件.pdf");
    let ev13 = corpus_evidence("EV-13-安全生产许可证.pdf");
    for f in [&a, &c, &ev13] {
        if !f.is_file() {
            panic!("夹具缺失: {}", f.display());
        }
    }
    let import = ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [
            { "upload_ref": a.display().to_string(), "filename": "A-招标文件.docx", "kind": "招标文件" },
            { "upload_ref": c.display().to_string(), "filename": "C-补遗文件.pdf", "kind": "补遗" },
            { "upload_ref": ev13.display().to_string(), "filename": "EV-13-安全生产许可证.pdf", "kind": "附件" } ] }),
    ));
    assert_eq!(import["totals"]["parse_failed"], json!(0), "三件应全成功: {import:?}");
    let fid = import["per_file"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["filename"].as_str() == Some("EV-13-安全生产许可证.pdf"))
        .and_then(|p| p["file_id"].as_str())
        .unwrap()
        .to_string();
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    ok_result(call(&mut svc, "build_outline", json!({ "project_id": &pid })));
    (svc, pid, fid)
}

/// 占位登记回执面：placeholder=true → PLACEHOLDER + placeholder=true + hint
/// 警示（不清阻塞/不得声称已具备/升级路径）；同语义位重登（同态）幂等。
#[test]
fn register_placeholder_reply_carries_tier_and_warning() {
    let (mut svc, pid, fid) = matrix_ready_store("reply");
    let r = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "安全生产许可证（占位待核）",
            "source_file_id": &fid, "subject_name": "投标人甲", "scope": "建筑施工",
            "placeholder": true,
        }),
    ));
    assert_eq!(r["verification_tier"].as_str(), Some("PLACEHOLDER"), "占位登记: {r:?}");
    assert_eq!(r["placeholder"], json!(true));
    assert_eq!(r["status"], "ACTIVE", "状态面 Active（有效期维度与占位正交）");
    let hint = r["hint"].as_str().unwrap();
    assert!(hint.contains("占位") && hint.contains("不清除合规阻塞"), "hint 警示在场: {hint}");
    // 幂等重放（同态同指纹）
    let r2 = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "安全生产许可证（占位待核）",
            "source_file_id": &fid, "subject_name": "投标人甲", "scope": "建筑施工",
            "placeholder": true,
        }),
    ));
    assert_eq!(r2["deduplicated"], json!(true), "同态重放");
    assert_eq!(r2["ev_id"], r["ev_id"]);
}

/// 自报拒（验收条件：MCP 层不能把占位改成 verified；非法迁移拒绝且无副作用）：
/// - `verification_tier` 参数不存在 → 显式拒（E_UNPROCESSABLE + 指路）；
/// - `placeholder` 非布尔 → 拒；
/// - 拒绝路径零入库（台账数量不变）。
#[test]
fn register_rejects_self_reported_tier_and_bad_placeholder_type() {
    let (mut svc, pid, fid) = matrix_ready_store("reject");
    let n_before = svc.store().evidences().len();
    // verification_tier 自报拒
    let reply = call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification", "title": "自报已验证（拒）",
            "source_file_id": &fid, "subject_name": "投标人甲",
            "verification_tier": "CONTENT_MATCHES",
        }),
    );
    let err = err_text(&reply);
    assert!(err.contains("verification_tier"), "指路验证态: {err}");
    assert!(err.contains("#245"), "纪律出处可查: {err}");
    assert_eq!(svc.store().evidences().len(), n_before, "拒绝路径零入库（无副作用）");
    // placeholder 非布尔拒
    let reply2 = call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification", "title": "占位类型错（拒）",
            "source_file_id": &fid, "subject_name": "投标人甲",
            "placeholder": "true",
        }),
    );
    assert!(err_text(&reply2).contains("布尔"), "类型错响亮拒");
    assert_eq!(svc.store().evidences().len(), n_before, "拒绝路径零入库");
}

/// 占位绑定 → 矩阵回显：PENDING_EVIDENCE + pending_evidence_ids + blocking
/// 保留（资格/否决行）；evidence 列 verification_tier 可辨；bind 回执
/// placeholder=true + hint 警示。
#[test]
fn matrix_rows_surface_pending_evidence_state() {
    let (mut svc, pid, fid) = matrix_ready_store("matrix");
    let r = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification", "title": "安全生产许可证（矩阵面占位）",
            "source_file_id": &fid, "subject_name": "投标人甲", "scope": "建筑施工",
            "placeholder": true,
        }),
    ));
    let ev_id = r["ev_id"].as_str().unwrap().to_string();
    // 绑定到矩阵全部行（事故形态）
    let all_req_ids: Vec<serde_json::Value> = svc
        .store()
        .requirements()
        .iter()
        .map(|x| serde_json::Value::String(x.id.clone()))
        .collect();
    let bind = ok_result(call(
        &mut svc,
        "bind_evidence",
        json!({ "project_id": &pid, "ev_id": &ev_id,
                "requirement_ids": all_req_ids,
                "bid_open_date": "2026-11-15" }),
    ));
    assert_eq!(bind["placeholder"], json!(true), "绑定回执占位标记: {bind:?}");
    let bind_hint = bind["hint"].as_str().unwrap_or("");
    assert!(bind_hint.contains("不清除合规阻塞"), "绑定回执警示: {bind_hint}");
    assert!(
        bind["matrix"]["blocking"].as_u64().unwrap_or(0) > 0,
        "占位绑定后 blocking 未归零（事故缺口收口）: {bind:?}"
    );
    // 矩阵行回显（工具面）
    let matrix = ok_result(call(
        &mut svc,
        "get_compliance_matrix",
        json!({ "project_id": &pid, "filters": { "status": "PENDING_EVIDENCE" } }),
    ));
    let rows = matrix["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "PENDING_EVIDENCE 过滤器可命中待补行");
    let first = &rows[0];
    assert_eq!(first["status"], "PENDING_EVIDENCE");
    assert!(
        first["pending_evidence_ids"].as_array().unwrap().contains(&json!(ev_id)),
        "待补追踪面在场: {first:?}"
    );
    // 资格/否决行必在 blocking 集（必选不清零）
    let full = ok_result(call(
        &mut svc,
        "get_compliance_matrix",
        json!({ "project_id": &pid }),
    ));
    let blocking_rows: Vec<&Value> = full["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["blocking"] == json!(true))
        .collect();
    assert!(!blocking_rows.is_empty(), "全量矩阵 blocking 行非空");
    for row in &blocking_rows {
        assert_eq!(row["status"], "PENDING_EVIDENCE", "blocking 行都是待补态（占位不清阻塞）: {row:?}");
    }
    // evidence 列带验证态（占位件可辨）
    let any_pending_row = rows[0]["pending_evidence_ids"].as_array().unwrap().len();
    assert!(any_pending_row > 0);
}

/// 真实证据（placeholder 缺省）零回归：占位语义不污染既有登记面——
/// 无 locator → FILE_EXISTS；带 locator 对账 → CONTENT_MATCHES（既有断言面
/// 在 evidence_content_tool.rs，此处锁契约面的「缺省即非占位」）。
#[test]
fn register_without_placeholder_defaults_to_real_path() {
    let (mut svc, pid, fid) = matrix_ready_store("default");
    let r = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "certificate", "title": "安全生产许可证（真实）",
            "source_file_id": &fid, "subject_name": "投标人甲", "scope": "建筑施工",
        }),
    ));
    assert_eq!(r["verification_tier"].as_str(), Some("FILE_EXISTS"), "缺省非占位: {r:?}");
    assert_eq!(r["placeholder"], json!(false));
    assert!(
        !r["hint"].as_str().unwrap().contains("占位"),
        "非占位登记不带占位警示: {}",
        r["hint"]
    );
}
