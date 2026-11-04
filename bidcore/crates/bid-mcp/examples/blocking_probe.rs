//! #226 层 2 验收探针：真实文件 → 全链 → matrix_blocking（PR #230 审核口径）。
//!
//! ```text
//! cargo run --release -p bid-mcp --example blocking_probe -- <pdf 绝对路径>
//! ```
//!
//! 链路：create → import → extract → build_outline（矩阵首建），输出：
//! - REQ 总数 / 分类计数 / matrix_rows / **matrix_blocking**
//! - 全部 blocking 行（req_id、分类、条款号、anchor 摘录）——供人工逐条
//!   判定「真否决（该保留） vs 规则说明（判据仍有漏）」。
//!
//! Lead 基线（main，本机 poppler 路径）：
//! A-某水利施工标段 935 条 → build_outline → matrix_blocking = 121。
//! 解析路径说明：import 走 parse_file（pdfium → pdftotext → 纯 Rust 兜底），
//! 与真实 E2E 同路；与本探针对照的数字必须在**同一解析路径**上取。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Instant;

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
        ToolReply::Err { error, .. } => panic!("blocking-probe 步骤失败: {error}"),
    }
}

fn main() {
    let pdf = std::env::args()
        .nth(1)
        .expect("用法: blocking_probe <pdf 绝对路径>");
    let pdf = std::fs::canonicalize(&pdf).unwrap_or_else(|_| PathBuf::from(&pdf));
    let name = pdf
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let files_root = pdf.parent().unwrap().to_path_buf();

    let t0 = Instant::now();
    let ws = std::env::temp_dir().join(format!("blocking-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(files_root);

    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": format!("blocking-probe——{name}"), "bid_deadline": "2026-12-31",
                "mode": "明标", "external_key": format!("blocking-probe-{name}") }),
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
    println!(
        "导入: {} 页 / {} 行块 / {:.1}s",
        f0["pages"],
        f0["chunks"],
        t0.elapsed().as_secs_f32()
    );

    let extract = ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    let n_reqs = extract["counts"]["total"].as_u64().unwrap_or(0);
    let reqs = svc.store().requirements().to_vec();
    let mut cats: Vec<(String, usize)> = {
        let mut m = std::collections::BTreeMap::new();
        for r in &reqs {
            *m.entry(r.category.clone()).or_insert(0usize) += 1;
        }
        m.into_iter().collect()
    };
    cats.sort_by(|a, b| b.1.cmp(&a.1));
    println!("抽取: {n_reqs} 条 | {:.1}s", t0.elapsed().as_secs_f32());
    for (c, n) in &cats {
        println!("    {c:<8} {n}");
    }

    let outline = ok_result(call(
        &mut svc,
        "build_outline",
        json!({ "project_id": &pid }),
    ));
    let c = &outline["counts"];
    println!(
        "建纲: sections={} leaves={} matrix_rows={} **matrix_blocking={}** | {:.1}s",
        c["sections"],
        c["leaves"],
        c["matrix_rows"],
        c["matrix_blocking"],
        t0.elapsed().as_secs_f32()
    );

    // blocking 明细（人工逐条判定面：真否决 vs 规则说明漏判）
    let view = svc.store().matrix_view();
    let blocking: Vec<&bid_domain::matrix::MatrixRow> =
        view.iter().filter(|m| m.blocking).collect();
    let req_by_id: std::collections::HashMap<&str, &bid_domain::Requirement> =
        reqs.iter().map(|r| (r.id.as_str(), r)).collect();
    println!("\n==== blocking 明细（{} 条）====", blocking.len());
    for (i, m) in blocking.iter().enumerate() {
        let r = req_by_id.get(m.requirement_id.as_str());
        let cat = r.map(|r| r.category.as_str()).unwrap_or("?");
        let cno = r
            .and_then(|r| r.clause_no.clone())
            .unwrap_or_default();
        let text = r
            .and_then(|r| r.anchors.first())
            .map(|a| a.excerpt.chars().take(70).collect::<String>())
            .unwrap_or_default();
        println!("[{:>3}] {:<10} {:<5} {:<6} {}", i + 1, m.requirement_id, cno, cat, text);
    }
    let _ = std::fs::remove_dir_all(&ws);
}
