//! #276 ★实质性条款（substantive）标记面→矩阵面端到端测试。
//!
//! 链路（真实语料 corpus-A，全覆盖标记——合并产物只含 Agent 关键点）：
//! `submit_block_marks`（requirements[].substantive 声明）
//! → `extract_requirements`（merge 落库 params.star）
//! → `build_outline`（联建矩阵——MatrixRow.substantive 派生）
//! → `get_compliance_matrix`（行 substantive 列 + star_summary 统计口）。
//!
//! 与 agent_mandatory_gate.rs 同款夹具形态（差异只在标记面声明）：
//! - REQ-A0001：substantive=true + mandatory=false（★与必选**正交**主断言——
//!   ★技术参数/商务条款未必必选，mandatory 不可从★推断）
//! - REQ-A0002：substantive=false（显式判非★——落库 params.star=false）
//!
//! 门禁语义锁定：substantive **不参与 blocking 派生**（★未响应是否阻断
//! 导出是门禁分级，Lead 裁决面）——本测试锁定「识别+统计可编程、门不误接」。

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

fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-star-substantive-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 全链夹具：REQ-A0001（★+非必选）/ REQ-A0002（非★）+ 矩阵构建。
fn starred_project(tag: &str) -> Option<(BidService, String)> {
    let ws = tmp_ws(tag);
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "★实质性条款标记", "bid_deadline": "2026-12-31", "mode": "明标",
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
        return None; // corpus-A 主文件块面太小，测试前提不成立
    }
    let k = total / 4;
    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": pid, "file_id": fid,
        "marks": [
            {"from": 0, "to": k, "verdict": "noise", "reason": "前置公告说明区"},
            {"from": k + 1, "to": total - 1, "verdict": "requirement", "reason": "正文要求区"},
        ],
        "requirements": [
            {
                "block_idx": k + 2,
                "statement": "★投标人须承诺接到报修通知后30分钟内响应（实质性条款）",
                "category": "商务要求",
                "mandatory": false,
                "substantive": true
            },
            {
                "block_idx": k + 3,
                "statement": "投标人可提供本地化驻场服务（非实质性加分项）",
                "category": "商务要求",
                "mandatory": false,
                "substantive": false
            }
        ]
    })));
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));
    ok_result(call(&mut svc, "build_outline", json!({ "project_id": pid })));
    Some((svc, pid))
}

fn matrix(svc: &mut BidService, pid: &str) -> Value {
    ok_result(call(svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 500,
    })))
}

fn row_of(m: &Value, rid: &str) -> Value {
    m["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["requirement"]["id"].as_str() == Some(rid))
        .cloned()
        .unwrap_or_else(|| panic!("{rid} 必须在矩阵中"))
}

#[test]
fn substantive_flows_into_matrix_rows_and_star_summary() {
    if !corpus_ready("substantive_flows_into_matrix_rows_and_star_summary") {
        return;
    }
    let (mut svc, pid) = starred_project("matrix-summary").expect("夹具就绪");
    let m = matrix(&mut svc, &pid);

    // 行级：substantive 列随行（两条 REQ-A 全覆盖 → 矩阵只含这两条）
    let a1 = row_of(&m, "REQ-A0001");
    assert_eq!(a1["substantive"], true, "Agent 标★ → 行 substantive=true");
    assert_eq!(
        a1["requirement"]["params"]["star"], true,
        "params.star 落库透传（审计面）"
    );
    // ★与 mandatory 正交：substantive=true 不改变 mandatory=false
    assert_eq!(
        a1["requirement"]["mandatory"], false,
        "★不推断必选（两条轴独立——★未必必选）"
    );
    // ★未响应不接 blocking（门禁分级是 Lead 裁决面——本断言锁定现状）
    assert_eq!(a1["status"], "NOT_RESPONDED");
    assert_eq!(a1["blocking"], false, "★未响应不阻断（识别+统计可编程，门不误接）");

    let a2 = row_of(&m, "REQ-A0002");
    assert_eq!(a2["substantive"], false, "Agent 显式判非★ → substantive=false");

    // 统计口：star_summary 全量口径（total/responded/not_responded + share）
    let s = &m["star_summary"];
    assert_eq!(s["total"], 1, "★条款恰 1 条（REQ-A0001）");
    assert_eq!(s["responded"], 0, "未起草 → ★响应 0");
    assert_eq!(s["not_responded"], 1);
    assert_eq!(s["not_applicable"], 0);
    assert_eq!(
        s["responded_share"].as_f64().unwrap_or(0.0), 0.0,
        "★响应率 0/1 = 0.0（有★时不为 null）"
    );
}

/// 统计口的 responded 面激活：★条款经人工覆写 RESPONDED（承诺型正门——
/// #182 签域 receipt）→ star_summary.responded 计数。矩阵行 substantive
/// 与 status 派生正交（识别列不随状态翻面）。
#[test]
fn star_summary_counts_responded_after_override() {
    if !corpus_ready("star_summary_counts_responded_after_override") {
        return;
    }
    let (mut svc, pid) = starred_project("summary-responded").expect("夹具就绪");
    // 宿主签发面（#182——与 agent_mandatory_gate.rs 同款）
    let receipt = svc.issue_approval_receipt(
        "主投标人",
        &bid_mcp::matrix_override_scope(
            &pid, "REQ-A0001", "RESPONDED", "承诺型覆写：30分钟响应承诺在正文 SEC 已写实",
        ),
    );
    ok_result(call(&mut svc, "set_matrix_override", json!({
        "project_id": pid,
        "requirement_id": "REQ-A0001",
        "status": "RESPONDED",
        "deviation": "承诺型覆写：30分钟响应承诺在正文 SEC 已写实",
        "receipt": receipt,
    })));
    let m = matrix(&mut svc, &pid);
    let s = &m["star_summary"];
    assert_eq!(s["total"], 1);
    assert_eq!(s["responded"], 1, "人工覆写 RESPONDED → ★响应计数");
    assert_eq!(s["not_responded"], 0);
    assert_eq!(
        s["responded_share"].as_f64().unwrap_or(0.0), 1.0,
        "★响应率 1/1"
    );
    let a1 = row_of(&m, "REQ-A0001");
    assert_eq!(a1["substantive"], true, "覆写改状态不改识别列（正交）");
    assert_eq!(a1["status"], "RESPONDED");
}

/// 无★条款的统计口形态：responded_share=null（不是 0/0 的假 0——
/// 验收面区分「没有★」与「★全未响应」两种截然不同的风险面）。
#[test]
fn star_summary_null_share_when_no_star() {
    if !corpus_ready("star_summary_null_share_when_no_star") {
        return;
    }
    let ws = tmp_ws("no-star");
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "无★条款", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": "no-star",
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
    let fid = out["file_ids"][0].as_str().unwrap().to_string();
    let total = out["per_file"][0]["chunks"].as_u64().unwrap() as usize;
    let k = total / 4;
    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": pid, "file_id": fid,
        "marks": [
            {"from": 0, "to": k, "verdict": "noise", "reason": "前置公告说明区"},
            {"from": k + 1, "to": total - 1, "verdict": "requirement", "reason": "正文要求区"},
        ],
        "requirements": [
            {
                "block_idx": k + 2,
                "statement": "普通条款（无★声明）",
                "category": "商务要求",
                "mandatory": false
            }
        ]
    })));
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));
    ok_result(call(&mut svc, "build_outline", json!({ "project_id": pid })));
    let m = matrix(&mut svc, &pid);
    let s = &m["star_summary"];
    assert_eq!(s["total"], 0, "未声明 substantive → 无★条款");
    assert!(s["responded_share"].is_null(), "无★时 share=null（区分「没有★」与「★全未响应」）");
}
