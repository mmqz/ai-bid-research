//! baseline_harness —— B1 基线冻结的逐文件解析入口（issue #2 · B1）。
//!
//! 用法：baseline-harness --file <path> [--rel <相对路径>]
//!
//! 纪律（与 issue #2 启动指令一致）：
//! - 只调用当前实现 `bid_parse::parse_file`，本 harness 不含任何解析业务逻辑；
//! - 逐文件单进程：由 Python 编排层（scripts/baseline_run.py）负责逐文件 spawn、
//!   超时（10s→hang）、峰值内存（rusage children）与确定性 JSON 落盘；
//! - 输出 schema：{file, sha256, pages, has_text_layer, parse_summary,
//!   components[], blocks[{idx,kind,locator,text,cells}], elapsed_ms, status}；
//! - 退出码：0 = ok；3 = 显式拒绝（Err，status=failed 带 error 原文）；
//!   101 = panic（status=failed 带 panic 原文）。
//! - 无时间戳、无随机：重跑一致性由排序后规范化 digest 保证（编排层负责）。

use bid_parse::parse_file;
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut file = PathBuf::new();
    let mut rel = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                file = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--rel" => {
                rel = args[i + 1].clone();
                i += 2;
            }
            _ => {
                eprintln!("未知参数 {}", args[i]);
                std::process::exit(2);
            }
        }
    }
    if file.as_os_str().is_empty() {
        eprintln!("用法: baseline-harness --file <path> [--rel <相对路径>]");
        std::process::exit(2);
    }
    let name = file.file_name().unwrap().to_string_lossy().to_string();
    let rel_display = if rel.is_empty() { name.clone() } else { rel.clone() };

    let started = Instant::now();
    let outcome = parse_file(&file, &name);
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let payload = match outcome {
        Ok(pf) => json!({
            "file": rel_display,
            "sha256": pf.sha256,
            "pages": pf.pages,
            "has_text_layer": pf.has_text_layer,
            "parse_summary": pf.parse_summary,
            "components": pf.components,
            "blocks": pf.blocks,
            "elapsed_ms": elapsed_ms,
            "status": "ok",
        }),
        Err(e) => json!({
            "file": rel_display,
            "sha256": null,
            "pages": 0,
            "has_text_layer": false,
            "parse_summary": "",
            "components": [],
            "blocks": [],
            "elapsed_ms": elapsed_ms,
            "status": "failed",
            "error": e,
        }),
    };
    // 单行 JSON 输出到 stdout；编排层负责 sort_keys + pretty 落盘
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", serde_json::to_string(&payload).unwrap());
    let _ = out.flush();
    // 退出码：ok=0；Err=3（显式拒绝）。panic 时 Rust runtime 自动 101。
    std::process::exit(if payload["status"] == "ok" { 0 } else { 3 });
}
