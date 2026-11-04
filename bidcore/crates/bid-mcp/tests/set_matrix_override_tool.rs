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
/// 签发矩阵覆写回执（签域 matrix_override:{rid} 行级绑定）。
fn override_receipt(svc: &mut BidService, actor: &str, rid: &str) -> String {
    svc.issue_approval_receipt(actor, &format!("matrix_override:{rid}"))
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

    let receipt = override_receipt(&mut svc, "226-lead", &rid);
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

    let receipt = override_receipt(&mut svc, "226-lead", &rid);
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
    let receipt = override_receipt(&mut svc, "226-lead", &rid);

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
    // 假 id 也要持真 receipt（签域绑 REQ-999999——验签应过、存在性门拒）
    let receipt = override_receipt(&mut svc, "226-lead", "REQ-999999");
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
    let receipt = override_receipt(&mut svc, "226-lead", &rid);
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
    let receipt = override_receipt(&mut svc, "226-lead", "REQ-0001");
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
    let receipt = override_receipt(&mut svc, "226-lead", &rid);
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

    let receipt = override_receipt(&mut svc, "226-lead", &rid);
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

    let receipt = override_receipt(&mut svc, "226-lead", &rid);
    ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_APPLICABLE",
        "deviation": "#226：先标不适用",
        "receipt": receipt,
    })));
    let r = ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": rid, "status": "NOT_RESPONDED",
        "receipt": receipt,
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
    let receipt = override_receipt(&mut svc, "226-lead", &rid);
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
    // 拿 A 行的 receipt 去覆写 B 行
    let receipt_a = override_receipt(&mut svc, "226-lead", &rid_a);
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
        &format!("matrix_override:{rid}"),
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
