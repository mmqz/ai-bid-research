//! #234 抽取层验收探针：真实文件 → 全链 → 全量 REQ dump（改前/改后逐条对照）。
//!
//! ```text
//! cargo run --release -p bid-mcp --example req_dump -- <pdf 绝对路径>
//! ```
//!
//! 链路与 blocking_probe 完全一致（create → import → extract），
//! 额外输出每条 REQ 的完整锚面（kind/locator/excerpt 全文），供
//! diff 对照「其余 REQ 逐条不变」的验收红线（纪律 1：dump 复核）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

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
        ToolReply::Err { error, .. } => panic!("req-dump 步骤失败: {error}"),
    }
}

fn main() {
    let pdf = std::env::args()
        .nth(1)
        .expect("用法: req_dump <pdf 绝对路径>");
    let pdf = std::fs::canonicalize(&pdf).unwrap_or_else(|_| PathBuf::from(&pdf));
    let name = pdf
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let files_root = pdf.parent().unwrap().to_path_buf();

    let ws = std::env::temp_dir().join(format!("req-dump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(files_root);

    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": format!("req-dump——{name}"), "bid_deadline": "2026-12-31",
                "mode": "明标", "external_key": format!("req-dump-{name}") }),
    ))["project_id"]
    .as_str()
    .unwrap()
    .to_string();

    let import = ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [
            { "upload_ref": pdf.display().to_string(), "filename": &name, "kind": "招标文件" }] }),
    ));
    let f0 = &import["per_file"][0];
    assert_eq!(f0["parse_status"], "ok", "导入失败: {f0:?}");

    let extract = ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    let n_reqs = extract["counts"]["total"].as_u64().unwrap_or(0);
    let reqs = svc.store().requirements().to_vec();
    println!("# total={n_reqs}");
    for r in &reqs {
        let cno = r.clause_no.clone().unwrap_or_default();
        println!(
            "REQ\t{}\t{}\t{}\tanchors={}",
            r.id, r.category, cno, r.anchors.len()
        );
        for a in &r.anchors {
            println!("ANCHOR\t{}\t{}\t{}\t{}", r.id, a.kind, a.locator, a.excerpt);
        }
    }
    let _ = std::fs::remove_dir_all(&ws);
}
