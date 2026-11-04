//! #226 层 1 · set_matrix_override 工具契约（否决项死锁的人工出口）。
//!
//! 背景：真实招标文件里「本次招标不接受联合体投标」这类**规则说明**被判成
//! 否决项 → mandatory → 未响应即 blocking，但它本就没有可响应的正文，
//! 于是章节永久不可起草、导出被阻断。人工标 NOT_APPLICABLE（附理由）是唯一
//! 诚实的出路。
//!
//! 本文件只测「工具面契约 + 死锁确实被解开」，**不测分类判定**（那是层 2）。
//!
//! 纪律：负向必须真拒绝——不带理由的 NOT_APPLICABLE、不存在的 requirement_id、
//! 未到 MATRIX_READY 的状态，都必须报错而不是静默通过。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_a_main() -> PathBuf {
    fixtures().join("corpus-A/files/A-招标文件.docx")
}

fn corpus_a_ready(test_id: &str) -> bool {
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&corpus_a_main()])
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(
        svc,
        ToolCall {
            tool: tool.to_string(),
            args,
        },
    )
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
        ToolReply::Err { error, .. } => format!("{error}"),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// 宿主签发面（#182——不进 MCP 工具表）：真人确认后宿主用服务态密钥
/// 签发矩阵覆写回执。签域四重绑定（项目/行/状态/理由摘要——Lead 合并门
/// 2026-11-04 口径），格式串借 bid_mcp re-export 的单一真源，不在测试面
/// 复制一份（防格式漂移后测试仍用旧串假绿）。
fn override_receipt(
    svc: &mut BidService,
    actor: &str,
    pid: &str,
    rid: &str,
    status: &str,
    deviation: &str,
) -> String {
    svc.issue_approval_receipt(
        actor,
        &bid_mcp::matrix_override_scope(pid, rid, status, deviation),
    )
}

/// 建项目 + 导入 + 抽取 + 建矩阵 —— 得到 MATRIX_READY 且矩阵已填。
fn ready_project(svc: &mut BidService, key: &str) -> String {
    let r = ok_result(call(svc, "create_bid_project", json!({
        "name": "226 层1 契约", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": key,
    })));
    let pid = r["project_id"].as_str().unwrap().to_string();
    ok_result(call(svc, "import_tender_files", json!({
        "project_id": pid,
        "files": [{
            "upload_ref": corpus_a_main().display().to_string(),
            "filename": "A-招标文件.docx",
            "kind": "招标文件",
        }],
    })));
    ok_result(call(svc, "extract_requirements", json!({ "project_id": pid })));
    ok_result(call(svc, "build_outline", json!({ "project_id": pid })));
    pid
}

/// 取矩阵里第一条 blocking 行（真实语料上必然存在）。
fn first_blocking(svc: &mut BidService, pid: &str) -> (String, Value) {
    let m = ok_result(call(svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })));
    let rows = m["rows"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["blocking"].as_bool().unwrap_or(false))
        .unwrap_or_else(|| panic!("真实语料上应存在 blocking 行（#226 的前提）"));
    // 矩阵行的条款 id 嵌在 requirement.id（行本身没有顶层 requirement_id）。
    let rid = row["requirement"]["id"].as_str().unwrap().to_string();
    (rid, m)
}

// ---------------------------------------------------------------- 1. 工具已注册

#[test]
fn tool_is_registered() {
    assert!(
        bid_mcp::TOOL_NAMES.contains(&"set_matrix_override"),
        "set_matrix_override 必须在 TOOL_NAMES 里（否则 dispatch 直接 E_NOTFOUND）"
    );
    // schema 面：tools/list 必须能列出它，且标记为已实现
    let schema = bid_mcp::tools_schema();
    let tools = schema["tools"].as_array().expect("tools 数组");
    let t = tools
        .iter()
        .find(|t| t["name"] == "set_matrix_override")
        .expect("tools/list 必须含 set_matrix_override");
    assert!(
        t["inputSchema"]["properties"]["status"]["enum"].is_array(),
        "status 必须声明枚举（客户端要能枚举可选值）"
    );
}

// ---------------------------------------------------------------- 2. 死锁真的解开

#[test]
fn not_applicable_unblocks_a_blocking_row() {
    if !corpus_a_ready("not_applicable_unblocks_a_blocking_row") {
        return;
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-unblock");
    let (rid, _) = first_blocking(&mut svc, &pid);
    // blocking 的权威面是 export_blockers()（导出闸门的输入）；矩阵响应里
    // 没有 counts.blocking 字段——用它做对照会得到一个"看起来通过"的真空断言。
    let blocking_before = svc.store().export_blockers().unwrap().len();
    assert!(blocking_before > 0, "前提：真实语料有 blocking");

    let receipt = override_receipt(
        &mut svc, "226-lead", &pid, &rid, "NOT_APPLICABLE",
        "#226 测试：该条为招标人规则说明，我方独立投标，无需响应正文",
    );
    let r = ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid,
        "requirement_id": rid,
        "status": "NOT_APPLICABLE",
        "deviation": "#226 测试：该条为招标人规则说明，我方独立投标，无需响应正文",
        "receipt": receipt,
    })));

    // 响应体里的该行必须已经不 blocking（回显取自重算后的矩阵，不是抄入参）
    let row = r["row"].as_object().expect("row 必须回显");
    assert_eq!(row["requirement_id"], rid);
    assert_eq!(row["status"], "NOT_APPLICABLE");
    assert_eq!(row["blocking"], false, "覆写后该行必须不再 blocking");
    assert!(
        row["deviation"].as_str().unwrap().contains("规则说明"),
        "偏离说明必须落库并回显"
    );

    // #226/#182：审计面——谁豁免了这一行、何时（回溯可查）
    assert_eq!(r["actor"]["actor_id"], "226-lead", "可信身份回显");
    assert_eq!(r["actor"]["actor_type"], "human");
    assert_eq!(
        r["override"]["principal"], "human:226-lead",
        "覆写落账携带 human:{{actor_id}} 回显面"
    );
    assert!(
        r["override"]["at"].as_u64().unwrap() > 0,
        "覆写时刻落账"
    );

    // 全局 blocking 计数必须下降（死锁解开的硬证据）
    let blocking_after = r["blocking_rows"].as_u64().unwrap();
    assert_eq!(
        blocking_after as usize,
        blocking_before - 1,
        "blocking 应恰减 1（{blocking_before} → {blocking_after}）"
    );

    // 再拉一次矩阵确认持久生效（不是只在响应体里好看）
    let after = svc.store().export_blockers().unwrap().len();
    assert_eq!(
        after, blocking_after as usize,
        "独立重读的 export_blockers 必须与覆写响应回显一致"
    );
}

#[test]
fn unblocked_row_lets_export_proceed_further() {
    if !corpus_a_ready("unblocked_row_lets_export_proceed_further") {
        return;
    }
    // 死锁的原始症状是「导出被阻断：N 条必选未响应」。这里验证覆写确实
    // 作用在导出闸门的输入上：blocking 计数下降后，export_blockers 里
    // 不再出现该条款 id。
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-export");
    let (rid, _) = first_blocking(&mut svc, &pid);

    let blockers_before = svc.store().export_blockers().unwrap();
    assert!(
        blockers_before.iter().any(|b| b.contains(&rid)),
        "前提：该条款在导出阻断清单里"
    );

    let receipt = override_receipt(
        &mut svc, "226-lead", &pid, &rid, "NOT_APPLICABLE", "#226：规则说明，不产生响应正文",
    );
    ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid,
        "requirement_id": rid,
        "status": "NOT_APPLICABLE",
        "deviation": "#226：规则说明，不产生响应正文",
        "receipt": receipt,
    })));

    let blockers_after = svc.store().export_blockers().unwrap();
    assert!(
        !blockers_after.iter().any(|b| b.contains(&rid)),
        "覆写后该条款必须从导出阻断清单里消失（死锁解开）"
    );
    assert_eq!(
        blockers_after.len(),
        blockers_before.len() - 1,
        "阻断清单应恰减少一条"
    );
}

// ---------------------------------------------------------------- 3. 负向：必须真拒绝

#[test]
fn not_applicable_without_deviation_is_rejected() {
    if !corpus_a_ready("not_applicable_without_deviation_is_rejected") {
        return;
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-no-dev");
    let (rid, _) = first_blocking(&mut svc, &pid);
    // 参数门（deviation 必填）先于验签面——receipt 按「无理由」口径签发即可
    let receipt = override_receipt(&mut svc, "226-lead", &pid, &rid, "NOT_APPLICABLE", "");

    // 缺 deviation 字段
    let r1 = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "receipt": receipt,
    }));
    assert_eq!(err_code(&r1), "E_UNPROCESSABLE", "无理由的不适用必须被拒");
    assert!(
        err_msg(&r1).contains("不可审计"),
        "报错要说清理由（不能只说「参数错误」）：{}",
        err_msg(&r1)
    );

    // deviation 是空白串（不是缺字段）——同样不算理由
    let r2 = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE", "deviation": "   ",
        "receipt": receipt,
    }));
    assert_eq!(err_code(&r2), "E_UNPROCESSABLE", "空白 deviation 同样必须被拒");

    // 拒绝后 blocking 不得变化（拒绝必须是真拒绝，不留半个状态）
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })));
    let row = m["rows"].as_array().unwrap().iter()
        .find(|r| r["requirement"]["id"] == rid.as_str()).unwrap();
    assert_eq!(row["blocking"], true, "被拒后该行仍 blocking");
}

#[test]
fn unknown_requirement_id_is_rejected() {
    if !corpus_a_ready("unknown_requirement_id_is_rejected") {
        return;
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-bad-req");
    // 假 id 也要持真 receipt（签域四重绑定 REQ-999999/NOT_APPLICABLE/理由——
    // 验签应过、存在性门拒——证明拒绝来自存在性而非验签）
    let receipt = override_receipt(
        &mut svc, "226-lead", &pid, "REQ-999999", "NOT_APPLICABLE",
        "伪造 id 不该能刷出豁免",
    );
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid,
        "requirement_id": "REQ-999999",
        "status": "NOT_APPLICABLE",
        "deviation": "伪造 id 不该能刷出豁免",
        "receipt": receipt,
    }));
    assert_eq!(err_code(&r), "E_NOTFOUND", "不存在的条款必须 E_NOTFOUND");
}

#[test]
fn invalid_status_is_rejected() {
    if !corpus_a_ready("invalid_status_is_rejected") {
        return;
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-bad-status");
    let (rid, _) = first_blocking(&mut svc, &pid);
    // 参数门（status 枚举）先于验签面——receipt 按 NOT_APPLICABLE 口径签发即可
    let receipt = override_receipt(&mut svc, "226-lead", &pid, &rid, "NOT_APPLICABLE", "");
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "IGNORED",
        "receipt": receipt,
    }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE", "未知 status 必须被拒");
    assert!(
        err_msg(&r).contains("RESPONDED"),
        "报错要列出合法值：{}",
        err_msg(&r)
    );
}

#[test]
fn draft_state_is_rejected_by_state_machine() {
    let mut svc = BidService::new().with_fs_root(fixtures());
    let r = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "226 态门", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "226-state",
    })));
    let pid = r["project_id"].as_str().unwrap().to_string();
    // DRAFT 态：矩阵都没建，覆写无对象 → E_STATE（receipt 先签好——
    // 状态门的报错不应被「缺 receipt」的参数错误顶掉）
    let receipt = override_receipt(
        &mut svc, "226-lead", &pid, "REQ-0001", "NOT_APPLICABLE",
        "DRAFT 态不该能覆写",
    );
    let reply = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid,
        "requirement_id": "REQ-0001",
        "status": "NOT_APPLICABLE",
        "deviation": "DRAFT 态不该能覆写",
        "receipt": receipt,
    }));
    assert_eq!(err_code(&reply), "E_STATE", "DRAFT 态必须被状态机拒绝");
}

#[test]
fn wrong_project_id_is_rejected() {
    if !corpus_a_ready("wrong_project_id_is_rejected") {
        return;
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-wrong-proj");
    let (rid, _) = first_blocking(&mut svc, &pid);
    // 路由门先拒——receipt 按真实项目口径签发即可
    let receipt = override_receipt(
        &mut svc, "226-lead", &pid, &rid, "NOT_APPLICABLE", "错项目 id 必须被拒",
    );
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": "PRJ-9999",
        "requirement_id": rid,
        "status": "NOT_APPLICABLE",
        "deviation": "错项目 id 必须被拒",
        "receipt": receipt,
    }));
    assert_eq!(err_code(&r), "E_NOTFOUND", "项目路由失败必须 E_NOTFOUND");
}

// ---------------------------------------------------------------- 4. 其它状态同样合法

#[test]
fn responded_status_also_unblocks_and_keeps_evidence_slot() {
    if !corpus_a_ready("responded_status_also_unblocks_and_keeps_evidence_slot") {
        return;
    }
    // RESPONDED 是「我写了正文」的正向确认——同样应解除 blocking。
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-responded");
    let (rid, _) = first_blocking(&mut svc, &pid);
    let blocking_before = svc.store().export_blockers().unwrap().len();

    let receipt = override_receipt(&mut svc, "226-lead", &pid, &rid, "RESPONDED", "");
    let r = ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "RESPONDED",
        "receipt": receipt,
    })));
    assert_eq!(r["row"]["blocking"], false, "RESPONDED 同样解除 blocking");
    assert_eq!(
        r["blocking_rows"].as_u64().unwrap() as usize,
        blocking_before - 1
    );
}

#[test]
fn not_responded_keeps_blocking() {
    if !corpus_a_ready("not_responded_keeps_blocking") {
        return;
    }
    // 把一条已覆写的改回 NOT_RESPONDED —— blocking 必须回来（覆写可逆，
    // 不是单向开关；否则「标错了」就再也改不回来）。
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-restore");
    let (rid, _) = first_blocking(&mut svc, &pid);
    let blocking_before = svc.store().export_blockers().unwrap().len();

    let receipt1 = override_receipt(
        &mut svc, "226-lead", &pid, &rid, "NOT_APPLICABLE", "#226：先标不适用",
    );
    ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "deviation": "#226：先标不适用",
        "receipt": receipt1,
    })));
    // 状态在签域里——改回 NOT_RESPONDED 必须换签（无 deviation 场景绑空串摘要）
    let receipt2 = override_receipt(&mut svc, "226-lead", &pid, &rid, "NOT_RESPONDED", "");
    let r = ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_RESPONDED",
        "receipt": receipt2,
    })));
    assert_eq!(r["row"]["blocking"], true, "改回 NOT_RESPONDED 后必须重新 blocking");
    assert_eq!(
        r["blocking_rows"].as_u64().unwrap() as usize,
        blocking_before,
        "blocking 计数应回到覆写前水平"
    );
}

// ---------------------------------------------------------------- 5. 幂等与版本

#[test]
fn repeated_same_override_is_idempotent() {
    if !corpus_a_ready("repeated_same_override_is_idempotent") {
        return;
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-idem");
    let (rid, _) = first_blocking(&mut svc, &pid);
    let receipt = override_receipt(
        &mut svc, "226-lead", &pid, &rid, "NOT_APPLICABLE", "#226：幂等测试",
    );
    let args = json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "deviation": "#226：幂等测试", "receipt": receipt,
    });
    let r1 = ok_result(call(&mut svc, "set_matrix_override", args.clone()));
    let r2 = ok_result(call(&mut svc, "set_matrix_override", args));
    assert_eq!(
        r1["blocking_rows"], r2["blocking_rows"],
        "重复同一覆写不得改变 blocking 计数"
    );
    assert_eq!(
        r1["row"]["status"], r2["row"]["status"],
        "重复覆写结果一致"
    );
}

// ---------------------------------------------------------------- 6. #182 receipt 验签面
// 人工出口只认真人：宿主签发的 receipt（HMAC + 行级签域 + 24h 时效窗）。
// Agent 伪造、错签域、跨行挪用、非 Human 身份，都必须真拒绝且零副作用。

#[test]
fn missing_receipt_is_rejected() {
    if !corpus_a_ready("missing_receipt_is_rejected") {
        return;
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-no-receipt");
    let (rid, _) = first_blocking(&mut svc, &pid);
    // 不带 receipt 字段——参数面直接拒（req_str 报缺参）
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "deviation": "没有 receipt 的豁免",
    }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE", "缺 receipt 必须被拒");
    assert!(
        err_msg(&r).contains("receipt"),
        "报错要指向缺失参数：{}",
        err_msg(&r)
    );
    // 零副作用
    let row = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })))["rows"].as_array().unwrap().iter()
        .find(|r| r["requirement"]["id"] == rid.as_str()).unwrap().clone();
    assert_eq!(row["blocking"], true, "被拒后该行仍 blocking");
}

#[test]
fn forged_receipt_is_rejected() {
    if !corpus_a_ready("forged_receipt_is_rejected") {
        return;
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-forged");
    let (rid, _) = first_blocking(&mut svc, &pid);
    // Agent 手搓的假 receipt（六段结构对但 HMAC 不对）
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "deviation": "伪造 receipt 自免单",
        "receipt": "v1.bid-agent.Human.matrix_override:REQ-0001.1730000000.deadbeef",
    }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE");
    assert!(
        err_msg(&r).contains("BAD_SIGNATURE") || err_msg(&r).contains("MALFORMED_RECEIPT"),
        "伪造 receipt 要报验签分类码：{}",
        err_msg(&r)
    );
}

#[test]
fn cross_requirement_receipt_is_rejected() {
    if !corpus_a_ready("cross_requirement_receipt_is_rejected") {
        return;
    }
    // 行级签域：REQ-A 的 receipt 免不了 REQ-B（豁免不得一票通融）
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-cross-scope");
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })));
    let rows = m["rows"].as_array().unwrap();
    let b1 = rows.iter().find(|r| r["blocking"].as_bool().unwrap_or(false)).unwrap();
    let rid_a = b1["requirement"]["id"].as_str().unwrap().to_string();
    let rid_b = rows.iter()
        .filter(|r| r["blocking"].as_bool().unwrap_or(false) && r["requirement"]["id"] != rid_a)
        .next().map(|r| r["requirement"]["id"].as_str().unwrap().to_string());
    let Some(rid_b) = rid_b else {
        return; // 只有一条 blocking 的语料形态——跳过（不适用本测试）
    };
    // 拿 A 行的 receipt 去覆写 B 行（同状态同理由——唯一差异是行号，
    // 证明行级绑定本身在拒，不是状态/理由碰巧不同）
    let receipt_a = override_receipt(
        &mut svc, "226-lead", &pid, &rid_a, "NOT_APPLICABLE",
        "拿 A 行的 receipt 豁免 B 行",
    );
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid_b, "status": "NOT_APPLICABLE",
        "deviation": "拿 A 行的 receipt 豁免 B 行",
        "receipt": receipt_a,
    }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE", "跨行挪用 receipt 必须被拒");
    assert!(
        err_msg(&r).contains("BAD_SIGNATURE"),
        "签域不匹配报 BAD_SIGNATURE：{}",
        err_msg(&r)
    );
    // B 行零副作用
    let row_b = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })))["rows"].as_array().unwrap().iter()
        .find(|r| r["requirement"]["id"] == rid_b).unwrap().clone();
    assert_eq!(row_b["blocking"], true, "被拒后 B 行仍 blocking");
}

#[test]
fn agent_identity_receipt_is_rejected() {
    if !corpus_a_ready("agent_identity_receipt_is_rejected") {
        return;
    }
    // 非 Human 身份：持钥签发的 Agent receipt（跨 crate 集成测试构造攻击
    // 样本的合法路径——approval_key 是宿主侧只读面）
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-agent-id");
    let (rid, _) = first_blocking(&mut svc, &pid);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let agent_receipt = bid_domain::principal::issue_approval_receipt_as(
        svc.approval_key(),
        "bid-agent",
        bid_domain::principal::ActorType::Agent,
        &bid_mcp::matrix_override_scope(&pid, &rid, "NOT_APPLICABLE", "Agent 给自己免单"),
        now,
    );
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "deviation": "Agent 给自己免单",
        "receipt": agent_receipt,
    }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE");
    assert!(
        err_msg(&r).contains("NOT_HUMAN_ACTOR"),
        "Agent 身份 receipt 报 NOT_HUMAN_ACTOR：{}",
        err_msg(&r)
    );
    // 零副作用：不落覆写
    let row = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })))["rows"].as_array().unwrap().iter()
        .find(|r| r["requirement"]["id"] == rid.as_str()).unwrap().clone();
    assert_eq!(row["blocking"], true, "被拒后该行仍 blocking");
}

#[test]
fn bare_identity_params_are_rejected() {
    if !corpus_a_ready("bare_identity_params_are_rejected") {
        return;
    }
    // #182：裸身份参数显式拒绝——参数面先于验签面（防人工出口退化成
    // 直传 operator 的 Agent 自免单通道）
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-bare-param");
    let (rid, _) = first_blocking(&mut svc, &pid);
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "deviation": "裸身份直传",
        "principal": "human:admin",
    }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE");
    assert!(
        err_msg(&r).contains("principal"),
        "报错要指明被拒参数：{}",
        err_msg(&r)
    );
}

// ---------------------------------------------------------------- 7. 签域四重绑定（Lead 合并门 2026-11-04）
//
// 合并门口径原文："require a host-signed receipt bound to the project and
// exact override scope (requirement/status/reason)"。行级绑定之外的三段
// （项目/状态/理由）在此逐项钉死：换了任何一段，就是换了人批的那件事。

#[test]
fn wrong_status_receipt_is_rejected() {
    if !corpus_a_ready("wrong_status_receipt_is_rejected") {
        return;
    }
    // 人批的是「这一条标不适用」；Agent 拿这张签去改 RESPONDED → 拒。
    // 状态换签不通用——否则人工确认面可以被挪去任何状态。
    // Lead 复核（2026-11-04）：单维隔离——RESPONDED 调用携带与签域**完全
    // 相同**的 deviation，项目/行/理由摘要三段全对齐，唯一错配段是状态。
    // 若此处省略 deviation，签域的空串摘要也错配，测试就证明不了状态段
    // 独立生效（status 和 reason 双重不符，拒了也说不清是哪段拦的）。
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-status-swap");
    let (rid, _) = first_blocking(&mut svc, &pid);
    let receipt = override_receipt(
        &mut svc, "226-lead", &pid, &rid, "NOT_APPLICABLE",
        "#226：人批的不适用豁免",
    );
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "RESPONDED",
        "deviation": "#226：人批的不适用豁免",
        "receipt": receipt,
    }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE", "换状态必须换签");
    assert!(
        err_msg(&r).contains("BAD_SIGNATURE"),
        "签域含状态段，状态不符即验签不过：{}",
        err_msg(&r)
    );
    // 零副作用
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })));
    let row = m["rows"].as_array().unwrap().iter()
        .find(|r| r["requirement"]["id"] == rid.as_str()).unwrap().clone();
    assert_eq!(row["blocking"], true, "被拒后该行仍 blocking");
}

#[test]
fn wrong_reason_receipt_is_rejected() {
    if !corpus_a_ready("wrong_reason_receipt_is_rejected") {
        return;
    }
    // 人批的是理由 A；Agent 提交理由 B → 拒。豁免说明被偷换 = 人批的那件
    // 事被偷换——理由全文以 sha256 摘要进签域。
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-reason-swap");
    let (rid, _) = first_blocking(&mut svc, &pid);
    let receipt = override_receipt(
        &mut svc, "226-lead", &pid, &rid, "NOT_APPLICABLE",
        "#226：人批的理由——该条为招标人规则说明",
    );
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "deviation": "#226：Agent 偷换的理由——同为不适用但内容变了",
        "receipt": receipt,
    }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE", "换理由必须换签");
    assert!(
        err_msg(&r).contains("BAD_SIGNATURE"),
        "签域含理由摘要，理由不符即验签不过：{}",
        err_msg(&r)
    );
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })));
    let row = m["rows"].as_array().unwrap().iter()
        .find(|r| r["requirement"]["id"] == rid.as_str()).unwrap().clone();
    assert_eq!(row["blocking"], true, "被拒后该行仍 blocking（理由未偷换成功）");
}

#[test]
fn cross_project_receipt_is_rejected() {
    if !corpus_a_ready("cross_project_receipt_is_rejected") {
        return;
    }
    // 签域绑项目：同键工作区给另一个项目签出的 receipt，不得豁免本项目
    // （REQ 编号跨项目会撞号——没有项目段时 REQ-0001 的签到处通用）。
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ready_project(&mut svc, "226-cross-project");
    let (rid, _) = first_blocking(&mut svc, &pid);
    let other = svc.issue_approval_receipt(
        "226-lead",
        &bid_mcp::matrix_override_scope(
            "PRJ-OTHER", &rid, "NOT_APPLICABLE", "#226：跨项目签",
        ),
    );
    let r = call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "deviation": "#226：跨项目签",
        "receipt": other,
    }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE", "跨项目 receipt 必须被拒");
    assert!(
        err_msg(&r).contains("BAD_SIGNATURE"),
        "签域含项目段，项目不符即验签不过：{}",
        err_msg(&r)
    );
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })));
    let row = m["rows"].as_array().unwrap().iter()
        .find(|r| r["requirement"]["id"] == rid.as_str()).unwrap().clone();
    assert_eq!(row["blocking"], true, "被拒后该行仍 blocking");
}

// ---------------------------------------------------------------- 8. 直起草路径（Lead 复核要求可执行化）
//
// Lead 2026-11-04 复核意见：直起草路径测试「valuable for the stated #226
// acceptance criterion」——落成可执行测试让 CI 持续钉住：
//   ① blocking 未解 → draft_response_section 直接 E_UNPROCESSABLE
//      （#226 死锁本体：章节根本不可起草，不只是挡导出）；
//   ② 有效 receipt 覆写（NOT_APPLICABLE + 理由）→ blocking 解除；
//   ③ 同一章节再起草 → 成功（人工出口的完整闭环）。
//
// 夹具为合成面（draft_tool.rs svc_ready 同款，刻意**不**给否决项绑证据——
// UncoveredVetoItem 正是要测的阻塞；补遗件在场防 MissingAddendum 抢戏，
// 资格证据在库防资格门抢戏）。

fn mk_veto_req(id: &str, excerpt: &str) -> bid_domain::Requirement {
    use bid_domain::{Anchor, Requirement};
    Requirement {
        id: id.into(),
        category: "否决项".into(),
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

#[test]
fn direct_draft_path_e_unprocessable_until_valid_override() {
    let mut svc = BidService::new();
    let created = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "226 直起草路径", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": "226-draft-path",
    })));
    let pid = created["project_id"].as_str().unwrap().to_string();
    // 主件 + 补遗都在场且 Ok（缺补遗会触发 MissingAddendum，掩盖要测的阻塞）
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-226-dp", 10, 1, 50, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    svc.store_mut().upsert_file(
        "C-补遗文件01.docx", "补遗", "sha-226-dp-add", 5, 0, 20, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    // 资格证据在库：readiness 的资格门看「台账里有可用资质」——章节标题触发
    // 词面命中时无可用证据会独立阻塞，抢否决项门的戏。只登记不绑定：
    // 否决项的响应来自覆写，不来自证据。
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: "水利水电工程施工总承包贰级资质证书".into(),
        source_file: "EV-226-dp.pdf".into(),
        source_sha256: "sha-226-dp-ev".into(),
        locator: EngineLocator::DocxParagraph {
            paragraph_index: 3,
            text_fingerprint: "fnv64:se".into(),
        },
        fingerprint: "fnv64:se".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某建设工程有限公司".into(),
        scope: "水利水电工程施工".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    svc.set_requirements(vec![mk_veto_req(
        "REQ-0001",
        "投标文件未经法定代表人签字的作否决处理",
    )]).unwrap();
    svc.prepare_outline_and_matrix(bid_domain::outline::OutlineSpec {
        target_total_chars: 3_000,
        max_reqs_per_leaf: 2,
    }).unwrap();
    let parents: std::collections::HashSet<&str> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.as_deref()).collect();
    let leaf = svc.store().sections().iter()
        .find(|n| !parents.contains(&n.id[..])
            && n.requirement_ids.iter().any(|r| r == "REQ-0001"))
        .map(|n| n.id.clone())
        .expect("REQ-0001 须落在叶章节（否则无起草对象）");

    // ① 覆写前：直起草 → E_UNPROCESSABLE（否决项尚未响应——死锁本体）
    let r1 = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf, "generator": "mock",
    }));
    assert_eq!(err_code(&r1), "E_UNPROCESSABLE", "blocking 未解时直起草必须被拒");
    assert!(
        err_msg(&r1).contains("否决项尚未响应"),
        "拒因须指向未响应的否决项（#226 死锁本体）：{}",
        err_msg(&r1)
    );

    // ② 有效 receipt 覆写（人工出口——只认真人签下的那一行/那一态/那一理由）
    let receipt = override_receipt(
        &mut svc, "226-lead", &pid, "REQ-0001", "NOT_APPLICABLE",
        "#226 直起草测试：程序性否决条款，无响应正文",
    );
    let r2 = ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": "REQ-0001", "status": "NOT_APPLICABLE",
        "deviation": "#226 直起草测试：程序性否决条款，无响应正文",
        "receipt": receipt,
    })));
    assert_eq!(r2["row"]["blocking"], false, "覆写后该行不再 blocking");
    assert_eq!(r2["blocking_rows"], 0, "合成夹具仅一条 blocking——应清零");

    // ③ 同一章节再起草 → 成功（死锁解开；CI 持续钉住这条主链）
    let r3 = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf, "generator": "mock",
    })));
    assert_eq!(r3["section_id"], json!(leaf), "同一章节起草成功（死锁已解）");
}
