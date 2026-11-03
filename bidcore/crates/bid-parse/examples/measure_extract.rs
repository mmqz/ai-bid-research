//! #218 measure_extract——real-corpus 逐文件解析/抽取测量（条款数对照基线）。
//!
//! 用法（从 bidcore 目录，与 Lead 建立的测量命令一致）：
//!   cargo run --release --example measure_extract -p bid-parse -- ../real-corpus
//!   cargo run --release --example measure_extract -p bid-parse -- ../real-corpus --preview 5
//!
//! 逐 PDF 测**两条路**并对照：
//!   · 纯 Rust 兜底（`pdf_text::parse_via_fallback`）——零外部依赖，正是 #218
//!     的修复面（无 pdfium / 无 poppler 环境的降级路）；
//!   · 全链 `parse_file`——pdfium/poppler 可用时走外部引擎（环境对照面，
//!     本沙箱有 poppler 时可复现「旧环境 814 条」级别的基线）。
//!
//! 输出：每文件「页数 / 行块数 / 条款数 / 首块预览」的对照表（stdout）
//! + 全细节 JSON（measure-report.json，含两条路各自的 blocks[0..preview]）。
//! 纪律：只读解析与抽取，不改任何文件。

use bid_parse::{extract::extract_requirements, parse_file, pdf_text};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

struct Measured {
    pages: usize,
    blocks: usize,
    reqs: usize,
    first_blocks: Vec<String>,
}

fn measure_parsed(pf: &bid_parse::ParsedFile, preview: usize) -> Measured {
    let reqs = extract_requirements(std::slice::from_ref(pf));
    Measured {
        pages: pf.pages,
        blocks: pf.blocks.len(),
        reqs: reqs.len(),
        first_blocks: pf
            .blocks
            .iter()
            .take(preview)
            .map(|b| b.text.trim().to_string())
            .collect(),
    }
}

/// 取 JSON 路对象里 first_blocks 的第 i 条（42 字符截断预览；越界 → "-"）。
fn preview_of(route: Option<&serde_json::Value>, i: usize) -> String {
    route
        .and_then(|r| r.get("first_blocks"))
        .and_then(|b| b.get(i))
        .and_then(|v| v.as_str())
        .map(|s| {
            let cut: String = s.trim().chars().take(42).collect();
            format!("\"{cut}\"")
        })
        .unwrap_or_else(|| "-".to_string())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = PathBuf::from("../real-corpus");
    let mut preview = 3usize;
    let mut dump: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--preview" => {
                preview = args[i + 1].parse().unwrap_or(3);
                i += 2;
            }
            "--dump" => {
                dump = Some(args[i + 1].clone());
                i += 2;
            }
            a if !a.starts_with("--") => {
                dir = PathBuf::from(a);
                i += 1;
            }
            _ => {
                eprintln!("未知参数 {}", args[i]);
                std::process::exit(2);
            }
        }
    }
    if !dir.is_dir() {
        eprintln!("目录不存在: {}（从 bidcore 目录跑请传 ../real-corpus）", dir.display());
        std::process::exit(2);
    }

    let mut files: Vec<PathBuf> = Vec::new();
    collect(&dir, &mut files);
    files.sort();

    let mut rows = Vec::new();
    for p in &files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if !name.to_lowercase().ends_with(".pdf") {
            continue;
        }
        // 路 1：纯 Rust 兜底（#218 修复面——不依赖环境里有没有 pdfium/poppler）
        let fb: Option<Measured> = match pdf_text::parse_via_fallback(p, &name) {
            Ok(pf) => {
                if let Some(out) = dump.as_deref() {
                    let path = format!("{}/{}.fallback.txt", out, name);
                    let body: String =
                        pf.blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
                    std::fs::write(path, body).ok();
                }
                let text = pdf_text::parsed_text_of(&pf);
                let ratio = pdf_text::cjk_ratio(&text);
                let raw = std::fs::read(p).unwrap_or_default();
                let probe = format!("{:?}", pdf_text::probe_cjk_garble(&raw, &text));
                let m = measure_parsed(&pf, preview);
                eprintln!(
                    "[兜底] {} 页={} 块={} 条款={} cjk_ratio={:.4} probe={}",
                    name, m.pages, m.blocks, m.reqs, ratio, probe
                );
                Some(m)
            }
            Err(e) => {
                eprintln!("[兜底] {} 解析失败: {}", name, e);
                None
            }
        };
        // 路 2：全链 parse_file（环境对照——本机有 poppler 时走外部引擎）
        let full: Option<Measured> = match parse_file(p, &name) {
            Ok(pf) => {
                if let Some(out) = dump.as_deref() {
                    let path = format!("{}/{}.full.txt", out, name);
                    let body: String =
                        pf.blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
                    std::fs::write(path, body).ok();
                }
                let m = measure_parsed(&pf, preview);
                eprintln!("[全链] {} 页={} 块={} 条款={}", name, m.pages, m.blocks, m.reqs);
                Some(m)
            }
            Err(e) => {
                eprintln!("[全链] {} 解析失败: {}", name, e);
                None
            }
        };
        rows.push(json!({
            "name": name,
            "fallback": fb.as_ref().map(|m| json!({
                "pages": m.pages, "blocks": m.blocks, "reqs": m.reqs,
                "first_blocks": m.first_blocks,
            })),
            "full": full.as_ref().map(|m| json!({
                "pages": m.pages, "blocks": m.blocks, "reqs": m.reqs,
                "first_blocks": m.first_blocks,
            })),
        }));
    }

    // 人类可读对照表（兜底路——#218 的修复面）
    println!("\n== measure_extract：纯 Rust 兜底路（#218 修复面）==");
    println!("{:<44} {:>4} {:>6} {:>5}  blocks[0..2]", "文件", "页", "块", "条款");
    for r in &rows {
        let name: &str = r["name"].as_str().unwrap();
        let short: String = name.chars().take(42).collect();
        let fb = r.get("fallback");
        if let Some(f) = fb.and_then(|v| v.as_object()) {
            println!(
                "{:<44} {:>4} {:>6} {:>5}  {} | {} | {}",
                short,
                f["pages"], f["blocks"], f["reqs"],
                preview_of(fb, 0), preview_of(fb, 1), preview_of(fb, 2),
            );
        } else {
            println!("{:<44}  解析失败（详见 stderr / JSON）", short);
        }
    }
    println!("\n== 对照：全链 parse_file（环境相关）==");
    for r in &rows {
        let name: &str = r["name"].as_str().unwrap();
        let short: String = name.chars().take(42).collect();
        let full = r.get("full");
        if let Some(f) = full.and_then(|v| v.as_object()) {
            println!(
                "{:<44} {:>4} {:>6} {:>5}  {}",
                short, f["pages"], f["blocks"], f["reqs"], preview_of(full, 0),
            );
        } else {
            println!("{:<44}  解析失败", short);
        }
    }

    fs::write("measure-report.json", serde_json::to_string_pretty(&json!({
        "tool": "bid-parse measure_extract（#218 条款数对照基线）",
        "input_dir": dir.display().to_string(),
        "files": rows,
    })).unwrap()).expect("写报告失败");
    println!("\nJSON 明细 -> measure-report.json");
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
