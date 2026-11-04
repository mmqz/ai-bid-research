//! #219 全块覆盖三工具契约测试。
//!
//! 覆盖：
//! - `get_document_chunks`：分页窗口 / size 上限 / 越界拒绝 / 带 verdict 回显
//! - `submit_block_marks`：区间落库 / 重叠拒绝 / 越界拒绝 / requirement 越区间拒绝
//! - `get_coverage_report`：覆盖统计 / 未覆盖区间合并
//!
//! 真件：corpus-A 主文件（docx，真实块面）。

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

fn err_code(reply: ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期失败，实际成功: {}", out.result),
    }
}

/// 独立临时工作区（并行安全）。
fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-coverage-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 建项目 + 导入主文件，返回 (svc, file_id, total_blocks)。
fn setup(tag: &str) -> Option<(BidService, String, usize)> {
    let ws = tmp_ws(tag);
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "覆盖测试", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": "coverage-test"
    })));

    let main = fixtures().join("A-招标文件.docx");
    let out = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{
            "upload_ref": main.to_string_lossy(),
            "filename": "A-招标文件.docx",
            "kind": "招标文件"
        }]
    })));
    let fid = out["file_ids"][0].as_str()?.to_string();
    let blocks = out["per_file"][0]["chunks"].as_u64()? as usize;
    Some((svc, fid, blocks))
}

#[test]
fn chunks_paging_and_verdict_echo() {
    let Some((mut svc, fid, total)) = setup("paging") else { return };
    assert!(total > 0, "文件应有块");

    // 首页
    let out = ok_result(call(&mut svc, "get_document_chunks", json!({
        "project_id": "PRJ-0001", "file_id": fid, "size": 5
    })));
    assert_eq!(out["total_blocks"].as_u64().unwrap() as usize, total);
    assert_eq!(out["returned"].as_u64().unwrap(), 5);
    assert!(out["chunks"].as_array().unwrap().iter().all(|c| c["verdict"].is_null()),
        "未提交标记前 verdict 应为 null");

    // 越界 from
    let code = err_code(call(&mut svc, "get_document_chunks", json!({
        "project_id": "PRJ-0001", "file_id": fid, "from": total + 10
    })));
    assert_eq!(code, "E_UNPROCESSABLE");

    // size 超上限
    let code = err_code(call(&mut svc, "get_document_chunks", json!({
        "project_id": "PRJ-0001", "file_id": fid, "size": 9999
    })));
    assert_eq!(code, "E_UNPROCESSABLE");
}

#[test]
fn submit_marks_full_coverage() {
    let Some((mut svc, fid, total)) = setup("full") else { return };
    let last = total - 1;
    let mid = total / 2;

    let out = ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": mid - 1, "verdict": "noise", "reason": "封面与目录"},
            {"from": mid, "to": last, "verdict": "requirement", "reason": "正文要求"}
        ]
    })));
    assert_eq!(out["coverage"]["complete"].as_bool().unwrap(), true);
    assert_eq!(out["coverage"]["covered_blocks"].as_u64().unwrap() as usize, total);
}

#[test]
fn submit_marks_overlap_rejected() {
    let Some((mut svc, fid, total)) = setup("overlap") else { return };
    if total < 4 { return; }

    let code = err_code(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": 3, "verdict": "noise"},
            {"from": 2, "to": 5, "verdict": "requirement"}
        ]
    })));
    assert_eq!(code, "E_UNPROCESSABLE");
}

#[test]
fn submit_marks_out_of_bounds_rejected() {
    let Some((mut svc, fid, total)) = setup("oob") else { return };
    let code = err_code(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [{"from": 0, "to": total + 100, "verdict": "noise"}]
    })));
    assert_eq!(code, "E_UNPROCESSABLE");
}

#[test]
fn requirement_outside_requirement_range_rejected() {
    let Some((mut svc, fid, total)) = setup("reqout") else { return };
    if total < 4 { return; }

    let code = err_code(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [{"from": 0, "to": total - 1, "verdict": "rule"}],
        "requirements": [{"block_idx": 0, "statement": "某要求"}]
    })));
    assert_eq!(code, "E_UNPROCESSABLE", "rule 区间不允许挂 requirement");
}

#[test]
fn empty_statement_rejected() {
    let Some((mut svc, fid, total)) = setup("empty") else { return };
    let code = err_code(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [{"from": 0, "to": total - 1, "verdict": "requirement"}],
        "requirements": [{"block_idx": 0, "statement": "   "}]
    })));
    assert_eq!(code, "E_UNPROCESSABLE");
}

#[test]
fn coverage_report_reports_gap() {
    let Some((mut svc, fid, total)) = setup("gap") else { return };
    if total < 10 { return; }

    // 只标前一半
    let half = total / 2;
    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [{"from": 0, "to": half - 1, "verdict": "noise"}]
    })));

    let out = ok_result(call(&mut svc, "get_coverage_report", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(out["complete"].as_bool().unwrap(), false);
    let files = out["files"].as_array().unwrap();
    let f = files.iter().find(|f| f["file_id"].as_str() == Some(fid.as_str())).unwrap();
    assert_eq!(f["covered_blocks"].as_u64().unwrap() as usize, half);
    assert!(f["uncovered_range_count"].as_u64().unwrap() > 0);
}

#[test]
fn unknown_file_rejected() {
    let Some((mut svc, _fid, _total)) = setup("unknown") else { return };
    let code = err_code(call(&mut svc, "get_document_chunks", json!({
        "project_id": "PRJ-0001", "file_id": "TF-9999"
    })));
    assert_eq!(code, "E_NOTFOUND");
}
// ====================================================================
// #219 第二步：抽取通路合并（Agent 优先 + 正则兜底——merge.rs 接线）
// ====================================================================

/// 主链路：Agent 标记优先（noise 区正则让位）+ 正则兜底（未覆盖块保留）
/// + Agent 关键点转 REQ-A0001 承接。
#[test]
fn extract_merge_agent_priority_with_regex_fallback() {
    let Some((mut svc, fid, total)) = setup("merge-priority") else { return };
    assert!(total >= 8, "corpus-A 主文件块面太小，测试前提不成立：{total}");
    // 全覆盖标记：前 1/4 noise（招标公告/说明区），其余 requirement（正文区）
    let k = total / 4;
    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": k, "verdict": "noise", "reason": "前置公告说明区"},
            {"from": k + 1, "to": total - 1, "verdict": "requirement", "reason": "正文要求区"},
        ],
        "requirements": [{
            "block_idx": k + 2,
            "statement": "投标人须具备试验检测资质（Agent 提取）",
            "category": "资格条件",
            "mandatory": true
        }]
    })));

    let r = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));
    let merge = &r["merge"];
    let regex_total = merge["regex_total"].as_u64().unwrap();
    let dropped = merge["regex_dropped_covered"].as_u64().unwrap();
    assert!(regex_total > 0, "corpus-A 必有正则产物（GT 25 埋点件）");
    assert!(dropped > 0, "noise 区（前 1/4 块）必有正则误报被 Agent 压制");
    assert_eq!(merge["agent_requirements"].as_u64().unwrap(), 1);
    assert_eq!(merge["coverage_complete"].as_bool().unwrap(), true, "全覆盖标记");
    // counts 自洽：total = regex_total - dropped + agent
    let t = r["counts"]["total"].as_u64().unwrap();
    assert_eq!(t, regex_total - dropped + 1, "合并守恒：正则保留 + Agent 承接");
    assert_eq!(r["status"], "REQUIREMENTS_EXTRACTED");

    // REQ-A0001 真落库（矩阵面验证：params.statement 承载 Agent 陈述）
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(m["counts"]["total"].as_u64().unwrap(), t);
    let rows = m["rows"].as_array().unwrap();
    let a1 = rows
        .iter()
        .find(|row| row["requirement"]["id"].as_str() == Some("REQ-A0001"))
        .expect("REQ-A0001 必须在矩阵中");
    assert_eq!(
        a1["requirement"]["params"]["statement"].as_str().unwrap(),
        "投标人须具备试验检测资质（Agent 提取）"
    );
    assert_eq!(a1["requirement"]["category"].as_str().unwrap(), "资格条件");
    assert_eq!(a1["requirement"]["params"]["marked_by"].as_str().unwrap(), "agent");
    // 溯源锚 = 块原文（k+2 号块）
    assert!(a1["requirement"]["source_anchor"].is_object(), "Agent REQ 必须可溯源到块原文");
}

/// 无标记时严格 no-op（GT 红线保护）：merge 统计全零、total == regex_total、
/// 与纯正则产物逐位一致。
#[test]
fn extract_no_marks_is_strict_noop() {
    let Some((mut svc, _fid, _total)) = setup("merge-noop") else { return };
    let r = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));
    let merge = &r["merge"];
    assert_eq!(merge["regex_dropped_covered"].as_u64().unwrap(), 0, "无标记零丢弃");
    assert_eq!(merge["agent_requirements"].as_u64().unwrap(), 0, "无提交零承接");
    assert_eq!(
        r["counts"]["total"].as_u64().unwrap(),
        merge["regex_total"].as_u64().unwrap(),
        "无标记时 total == 正则总数（no-op）"
    );
    // 矩阵口径不受影响：首行仍是 CMP-0001 / REQ-0001 旧序
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": "PRJ-0001"
    })));
    let rows = m["rows"].as_array().unwrap();
    assert_eq!(rows[0]["cmp_id"], "CMP-0001");
    assert_eq!(rows[0]["requirement"]["id"], "REQ-0001");
}

/// replace 语义：先 extract（无标记）→ Agent 补标 noise → 重跑 extract，
/// 被覆盖块上的旧正则 REQ 必须从库里消失（只增不删的 set 语义做不到）。
#[test]
fn extract_remarks_drop_covered_regex() {
    let Some((mut svc, fid, total)) = setup("merge-replace") else { return };
    // 第一次：无标记纯正则
    let r1 = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));
    let t1 = r1["counts"]["total"].as_u64().unwrap();
    assert!(t1 > 0);

    // Agent 补标：全部块 noise（极端面——正则应被清空）
    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [{"from": 0, "to": total - 1, "verdict": "noise", "reason": "全部判噪（测试用）"}]
    })));

    let r2 = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));
    let t2 = r2["counts"]["total"].as_u64().unwrap();
    assert_eq!(t2, 0, "全覆盖 noise + 无提交 → 正则清空（Agent 优先）");
    assert_eq!(
        r2["merge"]["regex_dropped_covered"].as_u64().unwrap(),
        t1,
        "丢弃数 = 首跑正则总数"
    );
    // added 为负（整集替换语义下的诚实账目）
    assert!(r2["counts"]["added"].as_i64().unwrap() < 0, "清空时 added 必为负");
}

/// category 非法在提交时即拒（fail-loud——不等合并时才炸出半个库）。
#[test]
fn submit_invalid_category_rejected() {
    let Some((mut svc, fid, total)) = setup("merge-badcat") else { return };
    let e = err_code(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [{"from": 0, "to": total - 1, "verdict": "requirement"}],
        "requirements": [{
            "block_idx": 0,
            "statement": "某要求",
            "category": "不存在的类别"
        }]
    })));
    assert_eq!(e, "E_UNPROCESSABLE");
    // 拒绝路径零副作用：标记未落库（覆盖报告仍无覆盖）
    let rep = ok_result(call(&mut svc, "get_coverage_report", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(rep["complete"].as_bool().unwrap(), false);
}

/// submitted 持久化面：submit → 重开工作区（新 BidService）→ extract
/// 仍能承接 Agent 关键点（PR #227 的 checkpoint 缺口修复验证）。
#[test]
fn submitted_persists_across_restart() {
    if !corpus_ready("submitted_persists_across_restart") {
        return;
    }
    let ws = tmp_ws("merge-restart");
    {
        let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
        ok_result(call(&mut svc, "create_bid_project", json!({
            "name": "重启测试", "bid_deadline": "2026-12-31", "mode": "明标",
            "external_key": "merge-restart"
        })));
        let main = fixtures().join("A-招标文件.docx");
        let out = ok_result(call(&mut svc, "import_tender_files", json!({
            "project_id": "PRJ-0001",
            "files": [{"upload_ref": main.to_string_lossy(), "filename": "A-招标文件.docx", "kind": "招标文件"}]
        })));
        let fid = out["file_ids"][0].as_str().unwrap().to_string();
        let total = out["per_file"][0]["chunks"].as_u64().unwrap() as usize;
        ok_result(call(&mut svc, "submit_block_marks", json!({
            "project_id": "PRJ-0001", "file_id": fid,
            "marks": [{"from": 0, "to": total - 1, "verdict": "requirement"}],
            "requirements": [{"block_idx": 1, "statement": "重启后仍在的要求"}]
        })));
    } // svc drop（写盘）
    // 重开：不重新 submit，直接 extract——submitted 必须从快照恢复
    let mut svc2 = BidService::open(&ws).expect("重开工作区").with_fs_root(fixtures());
    let r = ok_result(call(&mut svc2, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(r["merge"]["agent_requirements"].as_u64().unwrap(), 1,
        "submitted 必须经快照持久化——PR #227 checkpoint 缺口的修复验证");
}
