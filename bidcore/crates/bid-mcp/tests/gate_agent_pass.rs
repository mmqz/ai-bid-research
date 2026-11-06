//! #277 方案 A 集成测试：agent 可过门但全程留痕（用户裁定）。
//!
//! 背景：#269 六场黑盒考验全部停在 MATRIX_READY——P5 起草门全断（无补遗
//! 确认门 + 证据门 + 矩阵未响应门），唯一出口 `record_addendum_waiver` /
//! `record_review_gate` 需**宿主为真人签发 receipt**，纯 Skill+MCP 的 Agent
//! 构成设计内死锁。方案 A（用户裁定原文：「我同意A，ai可以操控这个过去
//! 但是留痕」）：agent 签域（`:agent` 后缀）receipt 声明过门——留痕、导出
//! 带警告、可追认；严格模式（require_human_receipt=true，默认）保持现状。
//!
//! 验收对照（#277 issue 八条）：
//! 1. agent 带 agent 身份 → 过门成功 + 审计账留痕 ……… agent_waiver_passes_with_audit_trail
//! 2. require_human_receipt=true 工作区 → 仍被拒 ………… strict_mode_rejects_agent_declaration
//! 3. Human 用 agent 签域 → 拒 ……………………………… mixed_scope_receipts_rejected
//! 4. Agent 冒充 Human（人类签域）→ 仍拒 ………………… mixed_scope_receipts_rejected
//! 5. 导出带警告横幅 + 响应面清单 …………………………… agent_pass_e2e_banner_and_ratification
//! 6. 真人补 human receipt → 警告消失 + 两账 ……………… agent_pass_e2e_banner_and_ratification
//! 7. reason 为空 → 拒 ………………………………………… agent_reason_required
//! 8. 旧快照反序列化正常 ……………………………………… bid-domain 单测（legacy_snapshot_without_gate_passes_loads_empty）
//!
//! 链路纪律：写操作全部走 `dispatch_with` 的 MCP 工具分派（不绕开状态机）；
//! agent receipt 用宿主侧读面 `approval_key()` + `issue_approval_receipt_as`
//! 构造（模拟 agent 持钥自签——#277 承认的现实：密钥是文件，能读就能签；
//! 方案 A 的定位是**把不可见的绕过变成可见的留痕**，不是假装挡得住）。

use bid_domain::principal::{issue_approval_receipt_as, ActorType};
use bid_domain::review::Gate;

/// gate 字符串 → 域枚举（签域构造用）。
fn gate_str_to_gate(s: &str) -> Gate {
    match s {
        "matrix_approval" => Gate::MatrixApproval,
        "outline_approval" => Gate::OutlineApproval,
        "pre_export_confirm" => Gate::PreExportConfirm,
        _ => Gate::AddendumWaiver,
    }
}
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_ready(test_id: &str) -> bool {
    let main = fixtures().join("A-招标文件.docx");
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&main])
}

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut()
            .unwrap()
            .insert("generator".into(), json!("mock"));
    }
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功: {error}"),
    }
}

fn err_text(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => format!("{error}"),
        ToolReply::Ok(out) => panic!("预期错误: {}", out.result),
    }
}

/// 独立临时工作区（并行安全）。relaxed=true 写 `.mang/config.json`
/// `{"require_human_receipt": false}`（自动化跑批形态）；false = 无配置
/// 文件（严格模式默认——桌面/交互形态）。
fn tmp_ws(tag: &str, relaxed: bool) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-gate-agent-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    if relaxed {
        std::fs::create_dir_all(d.join(".mang")).unwrap();
        std::fs::write(
            d.join(".mang").join("config.json"),
            "{\"require_human_receipt\": false}",
        )
        .unwrap();
    } else {
        std::fs::create_dir_all(&d).unwrap();
    }
    d
}

/// agent 签域 receipt（agent 持钥自签——v2 七段，含内容绑定摘要 #323）。
fn agent_receipt(svc: &BidService, actor: &str, scope: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let digest = svc.receipt_binding_digest();
    issue_approval_receipt_as(svc.approval_key(), actor, ActorType::Agent, scope, now, &digest)
}

/// 人类签域 receipt（宿主签发面——真人口径）。
fn human_receipt(svc: &mut BidService, actor: &str, scope: &str) -> String {
    svc.issue_approval_receipt(actor, scope)
}

/// 起步管线：建项目 + 导入主件（corpus-A A-招标文件.docx）到 FILES_IMPORTED。
/// 返回 (svc, project_id, file_id)——file_id 是主件档（addendum 确认的对象）。
fn imported_project(tag: &str, relaxed: bool) -> Option<(BidService, String, String)> {
    let ws = tmp_ws(tag, relaxed);
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(fixtures());
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "#277 agent 过门测试", "bid_deadline": "2026-12-31",
        "mode": "明标", "external_key": tag,
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    let r = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": &pid,
        "files": [{
            "upload_ref": fixtures().join("A-招标文件.docx").display().to_string(),
            "filename": "A-招标文件.docx", "kind": "招标文件",
        }],
    })));
    let fid = r["file_ids"][0].as_str()?.to_string();
    Some((svc, pid, fid))
}

// ---------------------------------------------------------------- 验收 1/2：通道开关

/// 验收 1：relaxed 工作区（require_human_receipt=false）——agent 声明确认
/// 补遗门 → 过门成功 + 审计账留痕（method=agent_declared、human_confirmed=false）。
#[test]
fn agent_waiver_passes_with_audit_trail() {
    if !corpus_ready("gate-agent-pass") {
        return;
    }
    let (mut svc, pid, fid) = imported_project("pass", true).expect("夹具就绪");
    assert!(!svc.require_human_receipt(), "relaxed 工作区配置面生效");

    let scope = format!("addendum_waiver:{fid}:agent");
    let receipt = agent_receipt(&svc, "agent-01", &scope);
    let r = ok_result(call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid,
        "reason": "主件完整解析在案，补遗面无未就绪件（agent 声明——待人工追认）",
        "receipt": receipt,
    })));
    // 审计面：agent 声明过门（human_confirmed=false——导出警告判据）
    assert_eq!(r["recorded"]["method"], "agent_declared");
    assert_eq!(r["recorded"]["human_confirmed"], false);
    assert_eq!(r["recorded"]["actor"]["actor_type"], "agent");
    assert_eq!(r["recorded"]["principal"], "agent:agent-01", "回显面带 agent: 前缀（不冒充 human:）");
    // 响应面 agent_waivers 清单 + human_review_complete=false
    assert_eq!(r["human_review_complete"], false);
    let waivers = r["agent_waivers"].as_array().unwrap();
    assert_eq!(waivers.len(), 1);
    assert_eq!(waivers[0]["gate"], "addendum_waiver");
    assert_eq!(waivers[0]["actor"], "agent-01");
    // 域层审计账（GatePassRecord）在座
    let gp = svc.store().latest_gate_pass(Gate::AddendumWaiver).unwrap();
    assert_eq!(gp.method, "agent_declared");
    assert!(!gp.human_confirmed);
    assert!(gp.reason.contains("agent 声明"));
}

/// 验收 2：严格模式工作区（无 config.json → 默认 true）——同样的 agent
/// 声明被拒，报因说明 + 开启指引。
#[test]
fn strict_mode_rejects_agent_declaration() {
    if !corpus_ready("gate-agent-strict") {
        return;
    }
    let (mut svc, pid, fid) = imported_project("strict", false).expect("夹具就绪");
    assert!(svc.require_human_receipt(), "无配置 = 严格模式默认（Q1）");

    let scope = format!("addendum_waiver:{fid}:agent");
    let receipt = agent_receipt(&svc, "agent-01", &scope);
    let e = call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid,
        "reason": "尝试 agent 声明（严格模式下应被拒）",
        "receipt": receipt,
    }));
    let msg = err_text(&e);
    assert!(
        msg.contains("require_human_receipt"),
        "报错必须说明严格模式原因：{msg}"
    );
    assert!(
        msg.contains("config.json"),
        "报错必须给开启指引（工作区配置位）：{msg}"
    );
    // 拒绝零副作用：审计账无记录
    assert!(svc.store().gate_passes().is_empty());

    // 严格模式下 human receipt 仍走通（现状不变——双路验签第一路）
    let human_scope = format!("addendum_waiver:{fid}");
    let receipt = human_receipt(&mut svc, "reviewer-1", &human_scope);
    let r = ok_result(call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid,
        "reason": "人工确认（严格模式正常路径）",
        "receipt": receipt,
    })));
    assert_eq!(r["recorded"]["method"], "human_receipt");
    assert_eq!(r["recorded"]["human_confirmed"], true);
    assert_eq!(r["human_review_complete"], true);
}

// ---------------------------------------------------------------- 验收 3/4/7：签域纪律

/// 验收 3/4：混用签域双向拒——Human 用 agent 签域（NOT_AGENT_ACTOR）、
/// Agent 用人类签域（冒充真人，NOT_HUMAN_ACTOR 路径报错指明 agent 通道）。
#[test]
fn mixed_scope_receipts_rejected() {
    if !corpus_ready("gate-agent-mixed") {
        return;
    }
    let (mut svc, pid, fid) = imported_project("mixed", true).expect("夹具就绪");

    // 验收 3：Human 用 agent 签域（宿主误签——签域与身份不匹配）
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let key = svc.approval_key().clone(); // approval_key() -> &[u8;32] 拷贝
    let digest = svc.receipt_binding_digest();
    let human_on_agent = issue_approval_receipt_as(
        &key,
        "admin",
        ActorType::Human,
        &format!("addendum_waiver:{fid}:agent"),
        now,
        &digest,
    );
    let e = call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid, "reason": "r",
        "receipt": human_on_agent,
    }));
    let msg = err_text(&e);
    assert!(
        msg.contains("NOT_AGENT_ACTOR") || msg.contains("签域与身份不匹配"),
        "Human 用 agent 签域必须拒（防混淆）：{msg}"
    );

    // 验收 4：Agent 用人类签域（冒充真人——现有防线不弱化）
    let agent_on_human = agent_receipt(&svc, "agent-01", &format!("addendum_waiver:{fid}"));

    let e2 = call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid, "reason": "r",
        "receipt": agent_on_human,
    }));
    let msg2 = err_text(&e2);
    assert!(
        msg2.contains("NOT_HUMAN_ACTOR") || msg2.contains("Agent 不得伪造") || msg2.contains("agent 声明过门"),
        "Agent 冒充 Human 必须拒（报错指明 agent 通道）：{msg2}"
    );
    assert!(svc.store().gate_passes().is_empty(), "两路拒绝零副作用");
}

/// 验收 7：agent 声明 reason 为空 → 拒（不能空着过）。
#[test]
fn agent_reason_required() {
    if !corpus_ready("gate-agent-reason") {
        return;
    }
    let (mut svc, pid, fid) = imported_project("reason", true).expect("夹具就绪");
    let scope = format!("addendum_waiver:{fid}:agent");
    let receipt = agent_receipt(&svc, "agent-01", &scope);
    for absent in [json!(null), json!(""), json!("   ")] {
        let e = call(&mut svc, "record_addendum_waiver", json!({
            "project_id": &pid, "file_id": &fid,
            "reason": absent,
            "receipt": receipt.clone(),
        }));
        assert!(
            err_text(&e).contains("理由") || err_text(&e).contains("reason"),
            "reason 必填：{absent}"
        );
    }
    assert!(svc.store().gate_passes().is_empty());
}

// ---------------------------------------------------------------- 验收 5/6：全链 + 横幅 + 追认

/// 全链 e2e（relaxed 工作区）。两条验收流共用本管线至「全部叶起草完成」：
/// - 验收 5（横幅流）：三门 + 补遗门全部 agent 声明 → 导出 → 警告横幅在位；
/// - 验收 6（追认流）：agent 声明后、导出前真人补 human receipt 重过全门
///   → 审计账两条并存 → 导出 → 横幅消失。
///   （追认窗口在导出**前**：EXPORTED 是审核门终态——先追认再导出正式稿
///   正是人类流程的时序；横幅导出是「未经追认就要成品」的告知面。）
fn agent_e2e_ready(ws: &std::path::Path) -> (BidService, String, String, Vec<String>) {
    let mut svc = BidService::open(ws).unwrap().with_fs_root(fixtures());
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "#277 e2e——agent 全门声明", "bid_deadline": "2026-12-31",
        "mode": "明标", "external_key": ws.file_name().unwrap().to_string_lossy(),
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();

    // ② 导入主件
    ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": &pid,
        "files": [{
            "upload_ref": fixtures().join("A-招标文件.docx").display().to_string(),
            "filename": "A-招标文件.docx", "kind": "招标文件",
        }],
    })));
    // ③ 抽取
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    // ④ 建纲建矩阵（服务面——无 MCP 工具的步骤）
    use bid_domain::outline::OutlineSpec;
    svc.prepare_outline_and_matrix(OutlineSpec { target_total_chars: 6_000, max_reqs_per_leaf: 6 })
        .unwrap();

    // ⑤ 证据绑定（服务面——全 REQ 绑一条证据清 blocking）+ 补遗门 agent 声明
    use bid_domain::evidence::EvidenceSpec;
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "电子与智能化工程专业承包二级资质证书".into(),
        source_file: "EV-02.pdf".into(),
        source_sha256: "sha-277-ev".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 0, text_fingerprint: "fnv64:277".into() },
        fingerprint: "fnv64:277".into(),
        valid_from: None, valid_until: None,
        subject_name: "投标人".into(), scope: "电子与智能化工程".into(),
        client: None, amount_wan: None, completed_on: None,
        fields: Default::default(), excerpt: None, excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    for r in svc.store().requirements().to_vec() {
        svc.store_mut().bind_evidence(&eid, &r.id, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();

    // 补遗门 agent 声明（MCP 面——:agent 签域 + reason）
    let fid = svc.store().files().first().unwrap().id.clone();
    let aw_scope = format!("addendum_waiver:{fid}:agent");
    let aw_receipt = agent_receipt(&svc, "agent-01", &aw_scope);
    ok_result(call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid,
        "reason": "单件招标无补遗件（导入面仅主件在案）——agent 声明，待人工追认",
        "receipt": aw_receipt,
    })));

    // 矩阵门 + 大纲门 agent 声明（MCP 面 record_review_gate——与 human 路径
    // 同一工具，receipt 签域决定身份路径）
    for gate in ["matrix_approval", "outline_approval"] {
        let scope = bid_mcp::review_gate_scope(gate_str_to_gate(gate));
        let agent_scope = format!("{scope}:agent");
        let receipt = agent_receipt(&svc, "agent-01", &agent_scope);
        let r = ok_result(call(&mut svc, "record_review_gate", json!({
            "project_id": &pid, "gate": gate, "decision": "APPROVED",
            "receipt": receipt,
            "reason": "矩阵/大纲已全量核对（agent 声明——导出将带警告，待人工追认）",
        })));
        assert_eq!(r["receipt"]["method"], "agent_declared", "{gate} agent 过门");
    }

    // ⑥ 逐章起草（MCP §6——补遗门已过，不再拦）
    let leaf_ids: Vec<String> = {
        let parents: std::collections::HashSet<String> = svc
            .store()
            .sections()
            .iter()
            .filter_map(|n| n.parent_id.clone())
            .collect();
        svc.store()
            .sections()
            .iter()
            .filter(|n| !parents.contains(&n.id))
            .map(|n| n.id.clone())
            .collect()
    };
    assert!(!leaf_ids.is_empty());
    for sid in &leaf_ids {
        ok_result(call(&mut svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": sid,
        })));
    }
    (svc, pid, fid, leaf_ids)
}

/// 验收 5（横幅流）：agent 全门声明 → 导出——响应面 agent_waivers/
/// human_review_complete=false + 产物正文首页警告横幅（Q3 措辞）。
#[test]
fn agent_pass_e2e_exports_with_banner() {
    if !corpus_ready("gate-agent-e2e") {
        return;
    }
    let ws = tmp_ws("e2e-banner", true);
    let (mut svc, pid, _fid, _leaf_ids) = agent_e2e_ready(&ws);

    // ⑦ 校验
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": &pid, "scope": "all",
    })));
    assert_eq!(r["ok"], json!(true), "⑦ 校验: {}", r["errors"]);

    // 导出确认门 agent 声明（全部叶起草完成后——#180 内容绑定口径）
    let pe_scope = bid_mcp::review_gate_scope(Gate::PreExportConfirm);
    let pe_receipt = agent_receipt(&svc, "agent-01", &format!("{pe_scope}:agent"));
    ok_result(call(&mut svc, "record_review_gate", json!({
        "project_id": &pid, "gate": "pre_export_confirm", "decision": "APPROVED",
        "receipt": pe_receipt,
        "reason": "成品内容 agent 自检通过（声明——待人工复核）",
    })));

    // ---- 验收 5：导出（响应面 + 产物横幅）
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-agent声明.docx",
    })));
    assert_eq!(r["human_review_complete"], false, "全门 agent 声明 → 未完成人工复核");
    let waivers = r["agent_waivers"].as_array().unwrap();
    assert!(waivers.len() >= 3, "三门+补遗门的 agent 声明都在清单：{waivers:?}");
    assert!(
        r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap_or("").contains("agent")),
        "warnings 携带 agent 过门事实：{waivers:?}"
    );
    // 产物正文横幅（解包 document.xml——「Word 打开」的机器等价面）
    let url = r["artifact"]["url"].as_str().unwrap().to_string();
    let bytes = std::fs::read(&url).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut doc_xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut doc_xml).unwrap();
    assert!(
        doc_xml.contains("本标书的部分审核门由 AI 声明通过，未经人工复核"),
        "验收 5：产物首页警告横幅在位（Q3 措辞）"
    );

    // 重启续跑面：审计账持久化（gate_passes 进快照）
    let svc2 = BidService::open(&ws).unwrap();
    assert_eq!(
        svc2.store().gate_passes().len(),
        svc.store().gate_passes().len(),
        "重启后过门审计账完整恢复（GP-xxxx 持久化）"
    );
}

/// 验收 6（追认流）：agent 声明后、导出前真人补 human receipt 重过全门
/// → 审计账两条并存（append-only）→ 导出无横幅、human_review_complete=true。
#[test]
fn human_ratification_before_export_clears_banner_two_ledger_records() {
    if !corpus_ready("gate-agent-ratify") {
        return;
    }
    let ws = tmp_ws("e2e-ratify", true);
    let (mut svc, pid, fid, _leaf) = agent_e2e_ready(&ws);

    // 追认（Drafting 态窗口内——矩阵门窗口含 Drafting）：补遗门 + 矩阵门 +
    // 大纲门 human receipt 重过（append-only：agent 声明条保留在账）
    let human_aw = format!("addendum_waiver:{fid}");
    let aw_receipt = human_receipt(&mut svc, "chief-reviewer", &human_aw);
    ok_result(call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid,
        "reason": "人工追认：确认无补遗（覆盖 agent 声明）",
        "receipt": aw_receipt,
    })));
    for gate in ["matrix_approval", "outline_approval"] {
        let scope = bid_mcp::review_gate_scope(gate_str_to_gate(gate));
        let receipt = human_receipt(&mut svc, "chief-reviewer", &scope);
        ok_result(call(&mut svc, "record_review_gate", json!({
            "project_id": &pid, "gate": gate, "decision": "APPROVED",
            "receipt": receipt,
        })));
    }
    // 审计账两条并存（验收 6：追认不覆盖——agent 声明条可追溯）
    let matrix_passes: Vec<_> = svc
        .store()
        .gate_passes()
        .iter()
        .filter(|p| p.gate == Gate::MatrixApproval)
        .collect();
    assert_eq!(matrix_passes.len(), 2, "验收 6：追认后两账并存（agent + human）");
    assert_eq!(
        svc.store().latest_gate_pass(Gate::MatrixApproval).unwrap().method,
        "human_receipt",
        "当前有效状态 = 最近一条（human）"
    );

    // 校验 → pre_export 门 human 过（定稿内容）→ 导出
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": &pid, "scope": "all",
    })));
    assert_eq!(r["ok"], json!(true));
    let pe_scope = bid_mcp::review_gate_scope(Gate::PreExportConfirm);
    let pe_receipt = human_receipt(&mut svc, "chief-reviewer", &pe_scope);
    ok_result(call(&mut svc, "record_review_gate", json!({
        "project_id": &pid, "gate": "pre_export_confirm", "decision": "APPROVED",
        "receipt": pe_receipt,
    })));

    let r2 = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-追认后.docx",
    })));
    assert_eq!(r2["human_review_complete"], true, "追认后人工复核完成");
    assert_eq!(r2["agent_waivers"].as_array().unwrap().len(), 0, "警告清单清空");
    let url2 = r2["artifact"]["url"].as_str().unwrap().to_string();
    let bytes2 = std::fs::read(&url2).unwrap();
    let mut zip2 = zip::ZipArchive::new(std::io::Cursor::new(&bytes2)).unwrap();
    let mut doc_xml2 = String::new();
    zip2.by_name("word/document.xml").unwrap().read_to_string(&mut doc_xml2).unwrap();
    assert!(
        !doc_xml2.contains("本标书的部分审核门由 AI 声明通过"),
        "验收 6：追认后产物横幅消失"
    );
}
