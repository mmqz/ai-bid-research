//! bid-parse 真实电信招标文件解析例程（M1.7-TEL 文件级验证）。
//!
//! 用法：
//!   cargo run --release --example realfiles -- --dir <parse-input-dir> --out <report.json>
//!
//! 输入目录约定：文件名前缀承载业务类别（extract 层接口，内容为真实原始文件）：
//!   A- 主招标文件 / C- 补遗·变更 / B- 附件（仅解析，不抽条款）
//! 纪律：本例程只读解析与抽取，不改任何文件；报告含逐文件 sha256。
//!
//! B4 路由（issue #11 任务 3）：docx/xlsx 走 bid-engine 引擎门面
//! （`parse_docx_via_engine` / `parse_xlsx_via_engine`——自研 docx/xlsx 解析
//! 已删，引擎是 OOXML 读面单一真源）；pdf 保持 `parse_file`（文本层/
//! OCR 补遗链是标书侧保留面）。与 service.rs 同款扩展名路由。

use bid_parse::{extract::extract_requirements, parse_file};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

/// B4：docx/xlsx → 引擎；pdf → 旧路径（parse_file）。与 service.rs 同款路由。
fn parse_routed(path: &Path, name: &str) -> Result<bid_parse::ParsedFile, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "docx" => bid_engine::parse_docx_via_engine(path).map_err(|e| e.to_string()),
        "xlsx" => bid_engine::parse_xlsx_via_engine(path).map_err(|e| e.to_string()),
        _ => parse_file(path, name),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = PathBuf::new();
    let mut out = PathBuf::from("realfiles-report.json");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--out" => {
                out = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            _ => {
                eprintln!("未知参数 {}", args[i]);
                std::process::exit(2);
            }
        }
    }
    if !dir.is_dir() {
        eprintln!("--dir 必须是目录: {}", dir.display());
        std::process::exit(2);
    }

    let mut files: Vec<PathBuf> = Vec::new();
    collect(&dir, &mut files);
    files.sort();

    let mut parsed = Vec::new();
    let mut reports = Vec::new();
    let mut failures = Vec::new();
    for p in &files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !matches!(ext.as_str(), "docx" | "xlsx" | "pdf") {
            continue; // 跳过其余扩展名（不伪装解析）
        }
        match parse_routed(p, &name) {
            Ok(pf) => {
                reports.push(json!({
                    "name": pf.name,
                    "sha256": pf.sha256,
                    "pages": pf.pages,
                    "tables": pf.tables,
                    "has_text_layer": pf.has_text_layer,
                    "ocr_blocks": pf.ocr_blocks,
                    "blocks": pf.blocks.len(),
                    "para_blocks": pf.blocks.iter().filter(|b| matches!(b.kind, bid_parse::BlockKind::Para)).count(),
                    "table_rows": pf.blocks.iter().filter(|b| matches!(b.kind, bid_parse::BlockKind::TableRow)).count(),
                    "pdf_lines": pf.blocks.iter().filter(|b| matches!(b.kind, bid_parse::BlockKind::PdfLine)).count(),
                    "xlsx_rows": pf.blocks.iter().filter(|b| matches!(b.kind, bid_parse::BlockKind::XlsxRow)).count(),
                    "parse_summary": pf.parse_summary,
                }));
                parsed.push(pf);
            }
            Err(e) => failures.push(json!({"name": name, "error": e})),
        }
    }

    // 条款抽取（A-/C- 前缀主件与补遗；B- 附件按接口跳过抽取面）
    let reqs = extract_requirements(&parsed);
    let req_json: Vec<serde_json::Value> = reqs
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "category": r.category,
                "clause_no": r.clause_no,
                "source_file": r.source_file,
                "params": r.params,
                "anchors": r.anchors.iter().map(|a| json!({
                    "file": a.file, "kind": a.kind, "locator": a.locator, "excerpt": a.excerpt,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    let report = json!({
        "tool": "bid-parse realfiles example（Rust 真实解析器）",
        "input_dir": dir.display().to_string(),
        "files_parsed": reports.len(),
        "files_failed": failures.len(),
        "files": reports,
        "failures": failures,
        "requirements_n": req_json.len(),
        "requirements": req_json,
    });
    fs::write(&out, serde_json::to_string_pretty(&report).unwrap()).expect("写报告失败");
    println!(
        "解析 {} 成功 / {} 失败；条款抽取 {} 条 -> {}",
        reports.len(),
        failures.len(),
        req_json.len(),
        out.display()
    );
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).expect("读目录失败") {
        let e = e.expect("目录项失败");
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else {
            out.push(p);
        }
    }
}
