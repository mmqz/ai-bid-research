//! #285 ① get_star_coverage 工具契约测试。
//!
//! 覆盖（契约面——真件验收见同目录 real-fixture 用例）：
//! - 参数校验：未知 project / 未知 file → E_NOTFOUND
//! - 只读窗口：FILES_IMPORTED 态（未抽取）可查——全部真值计漏标 + 提示先抽取
//! - 无★文件：诚实空报告（star_total=0，next 提示无★）
//! - 零副作用：查前后 project_version 不动
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
        "bid-star-coverage-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 建项目 + 导入 corpus-A 主文件，返回 (svc, project_id, file_id)。
fn setup(tag: &str) -> Option<(BidService, String, String)> {
    if !corpus_ready(tag) {
        return None;
    }
    let ws = tmp_ws(tag);
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": "★漏标检测契约测试", "bid_deadline": "2026-12-31", "mode": "明标",
                "external_key": format!("star-coverage-contract-{tag}") }),
    ))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    let main = fixtures().join("A-招标文件.docx");
    let out = ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [
            { "upload_ref": main.display().to_string(), "filename": "A-招标文件.docx", "kind": "招标文件" }
        ]}),
    ));
    let fid = out["per_file"][0]["file_id"].as_str().unwrap().to_string();
    Some((svc, pid, fid))
}

/// 未知项目 / 未知文件 → E_NOTFOUND（只读工具同路由纪律）。
#[test]
fn unknown_project_and_file_rejected() {
    let Some((mut svc, pid, _fid)) = setup("notfound") else {
        return;
    };
    assert_eq!(
        err_code(call(
            &mut svc,
            "get_star_coverage",
            json!({ "project_id": "P-不存在", "file_id": "TF-0001" })
        )),
        "E_NOTFOUND"
    );
    // 缺 file_id → E_UNPROCESSABLE（必填参数）
    assert_eq!(
        err_code(call(
            &mut svc,
            "get_star_coverage",
            json!({ "project_id": &pid })
        )),
        "E_UNPROCESSABLE"
    );
    // 项目对但文件错 → E_NOTFOUND
    assert_eq!(
        err_code(call(
            &mut svc,
            "get_star_coverage",
            json!({ "project_id": &pid, "file_id": "TF-9999" })
        )),
        "E_NOTFOUND"
    );
}

/// FILES_IMPORTED（未抽取）可查——只读窗口 + 诚实口径（全部真值计漏标）。
#[test]
fn files_imported_readable_with_extract_hint() {
    let Some((mut svc, pid, fid)) = setup("pre-extract") else {
        return;
    };
    let out = ok_result(call(
        &mut svc,
        "get_star_coverage",
        json!({ "project_id": &pid, "file_id": &fid }),
    ));
    // corpus-A 主文件无★（字面真值 0）——诚实空报告
    assert_eq!(out["star_total"], json!(0), "无★文件 star_total=0: {out}");
    assert_eq!(out["star_truth"], json!(0));
    assert_eq!(out["star_captured"], json!(0));
    assert_eq!(out["star_missing"], json!(0));
    assert!(
        out["next"].as_str().unwrap().contains("无★"),
        "无★提示: {}",
        out["next"]
    );
    // 未抽取 + 有真值★时的提示口径见真件验收用例；此处无★无 REQ 不触发
    // （warnings 只在 truth>0 且无 REQ 时提示先抽取）
    let _ = std::fs::remove_dir_all(tmp_ws("pre-extract"));
}

/// 只读零副作用：查前后 content_version 不动（与 get_coverage_report 同纪律）。
#[test]
fn read_only_no_version_bump() {
    let Some((mut svc, pid, fid)) = setup("readonly") else {
        return;
    };
    let before = ok_result(call(&mut svc, "get_bid_project_state", json!({ "project_id": &pid })))
        ["project_version"]
        .clone();
    let _ = ok_result(call(
        &mut svc,
        "get_star_coverage",
        json!({ "project_id": &pid, "file_id": &fid }),
    ));
    let after = ok_result(call(&mut svc, "get_bid_project_state", json!({ "project_id": &pid })))
        ["project_version"]
        .clone();
    assert_eq!(before, after, "只读工具不得 bump project_version");
}
