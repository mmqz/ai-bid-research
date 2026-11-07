//! AGENT_MANDATORY_NOT_BLOCKING 缺口收口的端到端门禁测试（Agent 必选接线）。
//!
//! 链路（真实语料 corpus-A，全覆盖标记——合并产物只含 Agent 关键点）：
//! `submit_block_marks`（Agent 标必选，类别不在资格/否决阻断面）
//! → `extract_requirements`（AGENT_MANDATORY_OFFCATEGORY 提示 + REQ-A 落库）
//! → `build_outline` / `build_matrix`（blocking 面接线）
//! → **导出门**（export_blockers 含 REQ-A0001）
//! → **起草门**（draft_response_section → UNCOVERED_MANDATORY_ITEM 阻塞）
//! → 人工 receipt 覆写（set_matrix_override，#182 签域）→ 两门放行。
//! → **承诺型出口**（决策点 2 裁定）：商务承诺条款无外部佐证材料时，
//! 人工覆写 RESPONDED+说明 是正门（NOT_APPLICABLE 留给真不适用）。
//!
//! 非误断对照：非必选（mandatory=false）条款全程不进 blocking 面；
//! noise/规则说明区间结构性不可能携带 requirement（validate_submitted——
//! 见 coverage_tools.rs `rule_range_rejects_requirement` /
//! `requirement_outside_requirement_range_rejected`，此处不重复）。
//!
//! GT 回归：无标记严格 no-op 由 coverage_tools.rs
//! `extract_no_marks_is_strict_noop` 保护；真实语料全链由 CI
//! real_corpus_e2e / scoring_real_corpus 保护（正则产物无 Agent 标记键，
//! blocking 面零位移）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_ready(test_id: &str) -> bool {
    let main = fixtures().join("A-招标文件.docx");
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&main])
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

fn err_msg(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => format!("{error}"),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// 独立临时工作区（并行安全）。
fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-agent-mandatory-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 宿主签发面（#182——与 set_matrix_override_tool.rs 同款，签域格式串借
/// bid_mcp re-export 的单一真源，不在测试面复制）。
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

/// 全链夹具：导入主文件 + 全覆盖标记（前 1/4 noise；k+1 隔离带 rule；
/// k+2..k+3 requirement + 两条 Agent 关键点承接；尾部 rule）
/// + extract + build_outline（内部联建矩阵——prepare_outline_and_matrix）。
///
/// 返回 (svc, project_id, matrix_blocking)。合并产物只含两条 REQ-A——
/// #422 口径下的隔离法：requirement 区间仅覆盖有 submitted 承接的两块
/// （无承接的 requirement 覆盖不再丢弃正则 REQ——黑洞防护），其余块用
/// noise/rule 显式拒绝，起草/导出门的断言不被正则条款的面目污染。
fn gated_project(tag: &str, req_mandatory: bool) -> Option<(BidService, String, u64)> {
    let ws = tmp_ws(tag);
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "Agent 必选门禁", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": tag,
    })));
    let pid = "PRJ-0001".to_string();
    let main = fixtures().join("A-招标文件.docx");
    let out = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": pid,
        "files": [{
            "upload_ref": main.to_string_lossy(),
            "filename": "A-招标文件.docx",
            "kind": "招标文件"
        }]
    })));
    let fid = out["file_ids"][0].as_str()?.to_string();
    let total = out["per_file"][0]["chunks"].as_u64()? as usize;
    if total < 8 {
        return None; // corpus-A 主文件块面太小，测试前提不成立（与 coverage_tools 同守卫）
    }
    let k = total / 4;
    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": pid, "file_id": fid,
        "marks": [
            {"from": 0, "to": k, "verdict": "noise", "reason": "前置公告说明区"},
            // #422 隔离带：requirement 区间只覆盖有承接的两块（k+2/k+3），
            // 其余块 rule 显式拒绝——正则 REQ 被拒丢弃，不进矩阵污染门断言。
            {"from": k + 1, "to": k + 1, "verdict": "rule", "reason": "隔离带（测试面——#422 后 requirement 区须有承接）"},
            {"from": k + 2, "to": k + 3, "verdict": "requirement", "reason": "正文要求区（两条关键点均有承接）"},
            {"from": k + 4, "to": total - 1, "verdict": "rule", "reason": "尾部区（测试面——显式拒绝）"},
        ],
        "requirements": [
            {
                "block_idx": k + 2,
                "statement": "投标人须承诺质保期不少于五年（Agent 提取，必选）",
                "category": "商务要求",
                "mandatory": req_mandatory
            },
            {
                "block_idx": k + 3,
                "statement": "投标人可提供本地化驻场服务（Agent 提取，非必选加分项）",
                "category": "商务要求",
                "mandatory": false
            }
        ]
    })));
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));
    let outline = ok_result(call(&mut svc, "build_outline", json!({ "project_id": pid })));
    let blocking = outline["counts"]["matrix_blocking"].as_u64()?;
    Some((svc, pid, blocking))
}

/// REQ-A0001（Agent 标必选，商务要求）在矩阵/工具面的形状。
fn row_of(svc: &mut BidService, pid: &str, rid: &str) -> Value {
    let m = ok_result(call(svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })));
    m["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["requirement"]["id"].as_str() == Some(rid))
        .cloned()
        .unwrap_or_else(|| panic!("{rid} 必须在矩阵中"))
}

// ---------------------------------------------------------------- 1. 矩阵面 + 导出门

#[test]
fn agent_mandatory_offcategory_flows_into_matrix_and_export_gate() {
    if !corpus_ready("agent_mandatory_offcategory_flows_into_matrix_and_export_gate") {
        return;
    }
    let (mut svc, pid, blocking) = gated_project("matrix-export", true).expect("夹具就绪");
    assert_eq!(blocking, 1, "建纲联建矩阵的 blocking 计数：恰 REQ-A0001 一条");

    // 矩阵面（派生真源）：必选未响应 → blocking；非必选未响应 → 不 blocking
    let a1 = row_of(&mut svc, &pid, "REQ-A0001");
    assert_eq!(a1["status"], "NOT_RESPONDED");
    assert_eq!(a1["blocking"], true, "Agent 必选（商务要求）未响应 → blocking（缺口收口）");
    // 工具面 mandatory 与门输入同源（此前硬编码类别判定，Agent 标记被无视）
    assert_eq!(a1["requirement"]["mandatory"], true, "工具面 mandatory 必须反映 Agent 必选标记");
    let a2 = row_of(&mut svc, &pid, "REQ-A0002");
    assert_eq!(a2["requirement"]["mandatory"], false, "非必选条款不因接线误标");
    assert_eq!(a2["blocking"], false, "Agent 非必选未响应 → 不阻断");

    // 导出门（check_export_ready 的输入面）：必选在列、非必选不在
    let blockers = svc.store().export_blockers().unwrap();
    assert!(
        blockers.iter().any(|b| b.contains("REQ-A0001")),
        "导出阻断清单必须含 Agent 必选未响应条款：{blockers:?}"
    );
    assert!(
        !blockers.iter().any(|b| b.contains("REQ-A0002")),
        "非必选条款不得进导出阻断清单：{blockers:?}"
    );
    let err = svc.store().check_export_ready().unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE", "导出门必须拒绝");
}

// ---------------------------------------------------------------- 2. 起草门 + receipt 放行

#[test]
fn agent_mandatory_draft_gate_blocks_until_receipt_override() {
    if !corpus_ready("agent_mandatory_draft_gate_blocks_until_receipt_override") {
        return;
    }
    let (mut svc, pid, _) = gated_project("draft-gate", true).expect("夹具就绪");

    // REQ-A0001 落在哪个叶章节（父聚合也含该 REQ——取最深层）
    let sec_id = svc
        .store()
        .sections()
        .iter()
        .filter(|n| n.requirement_ids.iter().any(|r| r == "REQ-A0001"))
        .max_by_key(|n| n.level as u32)
        .map(|n| n.id.clone())
        .expect("REQ-A0001 必须落在章节树里（build_outline 全量分组）");

    // 起草门：Agent 必选未响应 → E_UNPROCESSABLE，消息点名 REQ-A0001
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": sec_id, "generator": "mock",
    }));
    let msg = err_msg(&reply);
    assert!(
        msg.contains("必选条款尚未响应") && msg.contains("REQ-A0001"),
        "起草门必须拦 Agent 必选未响应：{msg}"
    );

    // 人工 receipt 覆写（不适用 + 理由——#182 签域四重绑定）
    let deviation = "质保期承诺由厂商标准保修条款覆盖，无需专项响应正文（人工裁定）";
    let receipt = override_receipt(&mut svc, "mandatory-lead", &pid, "REQ-A0001", "NOT_APPLICABLE", deviation);
    ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": "REQ-A0001",
        "status": "NOT_APPLICABLE", "deviation": deviation, "receipt": receipt,
    })));

    // 导出门：该条款从阻断清单消失
    let blockers = svc.store().export_blockers().unwrap();
    assert!(
        !blockers.iter().any(|b| b.contains("REQ-A0001")),
        "覆写后 REQ-A0001 必须从导出阻断清单消失：{blockers:?}"
    );
    // 矩阵行翻转（覆写态落账）
    let a1 = row_of(&mut svc, &pid, "REQ-A0001");
    assert_eq!(a1["status"], "NOT_APPLICABLE");
    assert_eq!(a1["blocking"], false);

    // 起草门：REQ-A0001 不再出现在阻塞消息里（其余阻塞如 MissingAddendum
    // 与本面无关——只断言该条款退出，不断言全就绪）
    let reply2 = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": sec_id, "generator": "mock",
    }));
    match reply2 {
        ToolReply::Err { error, .. } => {
            let m = format!("{error}");
            assert!(
                !m.contains("REQ-A0001"),
                "覆写后 REQ-A0001 不得再出现在起草阻塞里：{m}"
            );
        }
        ToolReply::Ok(_) => {} // 全部阻塞清零 → 直接放行（同样合法）
    }
}

// ---------------------------------------------------------------- 3. 非必选不误断（全程对照）

#[test]
fn non_mandatory_agent_marks_never_block_any_gate() {
    if !corpus_ready("non_mandatory_agent_marks_never_block_any_gate") {
        return;
    }
    // 同一夹具、唯一差异：REQ-A0001 也标非必选（mandatory=false）
    let (mut svc, pid, blocking) = gated_project("non-mandatory", false).expect("夹具就绪");
    assert_eq!(blocking, 0, "非必选标记不得产生 blocking 行");

    let a1 = row_of(&mut svc, &pid, "REQ-A0001");
    assert_eq!(a1["requirement"]["mandatory"], false);
    assert_eq!(a1["blocking"], false, "非必选条款不得进 blocking 面");

    // 导出门：阻断清单为空（两条 REQ-A 都非必选且无正则残留）
    let blockers = svc.store().export_blockers().unwrap();
    assert!(blockers.is_empty(), "非必选未响应不得拦导出：{blockers:?}");
    assert!(svc.store().check_export_ready().is_ok(), "导出门放行");
}

// ---------------------------------------------------------------- 4. 承诺型出口（报 Lead 决策点 2 的裁定落测）
//
// 裁定（2026-11-04）：普通商务必选条款「正文已响应、但没有外部证据」
// **不算已响应**——矩阵派生只认 绑定/章节归属/覆写 三真源，章节正文
// 内容不是派生输入（单测钉负向面：readiness_check.rs
// `drafted_body_without_evidence_is_not_response`）。承诺型条款的诚实
// 出口 = 人工 receipt 覆写 **RESPONDED**（带说明）——区别于上一用例的
// NOT_APPLICABLE（真不适用）：条款适用且以正文承诺响应、仅无外部佐证
// 材料时，不该谎报「不适用」。本用例钉全链：门拦 → RESPONDED 覆写 →
// 起草放行（正文落承诺）→ 导出门放行。

#[test]
fn promise_form_mandatory_exits_via_responded_override_receipt() {
    if !corpus_ready("promise_form_mandatory_exits_via_responded_override_receipt") {
        return;
    }
    let (mut svc, pid, blocking) = gated_project("promise-exit", true).expect("夹具就绪");
    assert_eq!(blocking, 1, "REQ-A0001 商务必选未响应 → 恰一条 blocking");

    // REQ-A0001 落在哪个叶章节（父聚合也含该 REQ——取最深层）
    let sec_id = svc
        .store()
        .sections()
        .iter()
        .filter(|n| n.requirement_ids.iter().any(|r| r == "REQ-A0001"))
        .max_by_key(|n| n.level as u32)
        .map(|n| n.id.clone())
        .expect("REQ-A0001 必须落在章节树里（build_outline 全量分组）");

    // 前置复核：未响应的承诺型必选条款拦起草（无证据无覆写，正文无从落）
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": sec_id, "generator": "mock",
    }));
    let msg = err_msg(&reply);
    assert!(
        msg.contains("必选条款尚未响应") && msg.contains("REQ-A0001"),
        "起草门先拦：{msg}"
    );

    // 承诺型正门：人工 receipt 覆写 RESPONDED + 说明（#182 签域含 status+deviation）
    let deviation = "商务承诺条款：正文承诺响应，无外部佐证材料（人工确认响应成立）";
    let receipt = override_receipt(&mut svc, "mandatory-lead", &pid, "REQ-A0001", "RESPONDED", deviation);
    ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid, "requirement_id": "REQ-A0001",
        "status": "RESPONDED", "deviation": deviation, "receipt": receipt,
    })));

    // 矩阵面：RESPONDED（非 NOT_APPLICABLE——适用条款的诚实态）且不再 blocking
    let a1 = row_of(&mut svc, &pid, "REQ-A0001");
    assert_eq!(a1["status"], "RESPONDED", "承诺型出口落 RESPONDED，不是谎报不适用");
    assert_eq!(a1["blocking"], false);

    // 起草门：REQ-A0001 退出阻塞（其余阻塞如 MissingAddendum 与本面无关——
    // 只断言该条款退出，不断言全就绪；放行即正文落承诺）
    let reply2 = call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": sec_id, "generator": "mock",
    }));
    match reply2 {
        ToolReply::Err { error, .. } => {
            let m = format!("{error}");
            assert!(
                !m.contains("REQ-A0001"),
                "RESPONDED 覆写后 REQ-A0001 不得再拦起草：{m}"
            );
        }
        ToolReply::Ok(_) => {} // 全部阻塞清零 → 直接放行（同样合法）
    }

    // 导出门：REQ-A0001 退出阻断清单（承诺型条款经人工确认后导出放行）
    let blockers = svc.store().export_blockers().unwrap();
    assert!(
        !blockers.iter().any(|b| b.contains("REQ-A0001")),
        "RESPONDED 覆写后 REQ-A0001 必须退出导出阻断：{blockers:?}"
    );
}
