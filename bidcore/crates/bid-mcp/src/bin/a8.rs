//! A8 端到端验收（issue #2 §B5）：一条命令——真实 .docx 招标文件 →
//! 导入（bid-engine 引擎路径）→ 抽条款（extract.rs 确定性规则）→ 合规矩阵 JSON。
//!
//! **全程 Rust，零 Python**（B5 验收硬标准）。
//!
//! 用法：`cargo run -p bid-mcp --bin a8 -- <招标文件.docx> [输出.json]`
//! 退出码：0 全链成功 / 1 任一环失败（stderr 带环节定位）。

use std::path::PathBuf;

use bid_tools::service::{BidService, ImportSpec};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: a8 <招标文件.docx> [输出.json]");
        std::process::exit(1);
    }
    let docx = PathBuf::from(&args[1]);
    let out_path = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("a8-compliance-matrix.json"));

    let mut svc = BidService::new();

    // ---- ① create_bid_project（状态机入口：DRAFT）
    let (_, created) = svc
        .store_mut()
        .create_project(
            "A8 端到端验收项目",
            "A8-DEMO",
            "公开招标",
            &format!("a8-{}", std::process::id()),
        )
        .unwrap_or_else(|e| die("create_project", &e.code()));
    let project_id = created["project_id"].as_str().unwrap_or_default().to_string();

    // ---- ② import_tender_files（docx → bid-engine 引擎路径 → DomainStore）
    let specs = [ImportSpec {
        upload_ref: docx.clone(),
        filename: docx
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "A-招标文件.docx".into()),
        kind: "主文件".into(),
        expected_pages: None,
    }];
    let report = svc.import(&specs);
    let ok = report
        .per_file
        .iter()
        .find(|o| o.parse_status != bid_domain::ParseStatus::Failed);
    let Some(ok) = ok else {
        die(
            "import_tender_files",
            &report
                .per_file
                .iter()
                .map(|o| o.errors.join("; "))
                .collect::<Vec<_>>()
                .join(" | "),
        );
    };
    eprintln!(
        "[A8-2] import: file_id={} chunks={} tables={} pages={}",
        ok.file_id.clone().unwrap_or_default(),
        ok.chunks,
        ok.tables,
        ok.pages
    );

    // ---- ③ extract_requirements（extract.rs 规则 × 引擎 blocks）
    let reqs = bid_parse::extract::extract_requirements(svc.parsed());
    let (total, added) = svc
        .store_mut()
        .set_requirements(reqs)
        .unwrap_or_else(|e| die("extract_requirements", &e.code()));
    eprintln!("[A8-3] extract: total={total} added={added}");

    // ---- ④ get_compliance_matrix（只读组装——与 MCP 工具同一存储面）
    let rows: Vec<serde_json::Value> = svc
        .store()
        .requirements()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let anchor = r.anchors.first();
            serde_json::json!({
                "cmp_id": format!("CMP-{:04}", i + 1),
                "requirement": {
                    "id": r.id,
                    "clause_no": r.clause_no,
                    "category": r.category,
                    "statement": anchor.map(|a| a.excerpt.as_str()).unwrap_or(""),
                    "source_anchor": anchor.map(|a| serde_json::json!({
                        "file": a.file, "kind": a.kind, "locator": a.locator,
                    })),
                },
                "status": "unanswered",
            })
        })
        .collect();

    let matrix = serde_json::json!({
        "project_id": project_id,
        "pipeline": ["create_bid_project", "import_tender_files(engine-docx)", "extract_requirements", "get_compliance_matrix"],
        "counts": { "total": total, "rows": rows.len() },
        "rows": rows,
    });
    let json = serde_json::to_string_pretty(&matrix).expect("矩阵序列化");
    std::fs::write(&out_path, &json).unwrap_or_else(|e| die("write_output", &e));
    println!(
        "[A8] 端到端完成：{} 条款 → {}（{} 字节）",
        total,
        out_path.display(),
        json.len()
    );
}

fn die(stage: &str, e: impl std::fmt::Display) -> ! {
    eprintln!("[A8-FAIL] {stage}: {e}");
    std::process::exit(1);
}
