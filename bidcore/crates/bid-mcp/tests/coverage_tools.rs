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