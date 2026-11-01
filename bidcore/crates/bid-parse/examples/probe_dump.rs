//! #144 探针：单文件解析 + 抽取 dump（JSON 单行——测量脚本消费口径）。
//!
//! 用法：
//!   cargo run -q -p bid-parse --example probe_dump -- [--extract] [--json] <file...>
//!
//! 输出（stdout 单行 JSON——measure/miss_scan 脚本对齐口径）：
//! ```json
//! {"files":[{"name","sha256","pages","blocks":[{"idx","kind","loc","text"}]}],
//!  "reqs":[{"id","cat","clause","src","anchors":[{"loc","text"}]}]}
//! ```
//! 纪律：只读不改；docx/xlsx 走 bid-engine 门面（service.rs 同款路由），
//! pdf 走 parse_file（文本层/OCR 链）。

use bid_parse::{extract::extract_requirements, parse_file, ParsedFile};
use serde_json::json;
use std::path::Path;

fn parse_routed(path: &Path, name: &str) -> Result<ParsedFile, String> {
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
    let do_extract = args.iter().any(|a| a == "--extract");
    let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if paths.is_empty() {
        eprintln!("用法: probe_dump [--extract] [--json] <file...>");
        std::process::exit(2);
    }
    let mut files_json = Vec::new();
    let mut parsed: Vec<ParsedFile> = Vec::new();
    for p in &paths {
        let path = Path::new(p.as_str());
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(p.as_str())
            .to_string();
        match parse_routed(path, &name) {
            Ok(pf) => {
                files_json.push(json!({
                    "name": pf.name,
                    "sha256": pf.sha256,
                    "pages": pf.pages,
                    "blocks": pf.blocks.iter().map(|b| json!({
                        "idx": b.idx,
                        "kind": format!("{:?}", b.kind),
                        "loc": b.locator,
                        "text": b.text,
                    })).collect::<Vec<_>>(),
                }));
                parsed.push(pf);
            }
            Err(e) => eprintln!("解析失败 {name}: {e}"),
        }
    }
    let mut reqs_json = Vec::new();
    if do_extract {
        for r in extract_requirements(&parsed) {
            reqs_json.push(json!({
                "id": r.id,
                "cat": r.category,
                "clause": r.clause_no,
                "src": r.source_file,
                "anchors": r.anchors.iter().map(|a| json!({
                    "loc": a.locator,
                    "text": a.excerpt,
                })).collect::<Vec<_>>(),
            }));
        }
    }
    // 单行输出（tail -1 友好——cargo run 的编译信息走 stderr）
    println!(
        "{}",
        serde_json::to_string(&json!({ "files": files_json, "reqs": reqs_json })).unwrap()
    );
}
