//! veto-gt-baseline —— issue #35 第二步：基线测量
//! 对 corpus 16 件跑生产路径解析 + extract_requirements，dump 全量 REQ
//! （id/category/clause_no/locator/excerpt），供与 veto-gt ground-truth 匹配
//! 计算抽取层召回 / 分类层召回 / 精确率 / 漏检清单。
//!
//! 用法：veto-gt-baseline <corpus_dir> <out_dir>

use std::path::{Path, PathBuf};

use bid_parse::ParsedFile;
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("用法: veto-gt-baseline <corpus_dir> <out_dir>");
        std::process::exit(2);
    }
    let corpus = PathBuf::from(&args[0]);
    let out = PathBuf::from(&args[1]);
    std::fs::create_dir_all(&out).expect("out dir");

    let mut files: Vec<PathBuf> = vec![];
    let mut tops: Vec<PathBuf> = list_files(&corpus, "pdf");
    files.append(&mut tops);
    let docdir = corpus.join("doc");
    if docdir.is_dir() {
        let mut docs = list_files(&docdir, "docx");
        files.append(&mut docs);
    }
    eprintln!("[veto-gt-baseline] corpus={} files={}", corpus.display(), files.len());

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        eprintln!("---- baseline {name}");
        let parsed: Result<ParsedFile, String> = match ext.as_str() {
            "pdf" => match bid_engine::parse_pdf_via_engine(path) {
                Ok(pf) => Ok(pf),
                Err(e) => bid_parse::parse_file(path, &name).map_err(|e| e.to_string()),
            },
            "docx" => bid_engine::parse_docx_via_engine(path).map_err(|e| e.to_string()),
            other => Err(format!("SKIP: .{other}")),
        };
        let pf = match parsed {
            Ok(pf) => pf,
            Err(e) => { eprintln!("    ERR {e}"); continue; }
        };
        let blocks_total = pf.blocks.len();
        let sha = pf.sha256.clone();
        let reqs = bid_parse::extract::extract_requirements(&[pf]);
        let mut req_out = vec![];
        for r in &reqs {
            req_out.push(json!({
                "id": r.id,
                "category": r.category,
                "clause_no": r.clause_no,
                "source_file": r.source_file,
                "anchors": r.anchors.iter().map(|a| json!({
                    "kind": a.kind, "locator": a.locator, "excerpt": a.excerpt,
                })).collect::<Vec<_>>(),
            }));
        }
        let mut cat_map = std::collections::BTreeMap::new();
        for r in &reqs { *cat_map.entry(r.category.clone()).or_insert(0u32) += 1; }
        let doc = json!({
            "schema": "veto-gt-baseline/1",
            "file": name,
            "sha256": sha,
            "blocks_total": blocks_total,
            "reqs_total": reqs.len(),
            "by_category": cat_map,
            "reqs": req_out,
        });
        let outp = out.join(format!("{name}.reqs.json"));
        std::fs::write(&outp, serde_json::to_vec(&doc).unwrap()).expect("write");
    }
    eprintln!("==== done → {}", out.display());
}

fn list_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().and_then(|x| x.to_str()) == Some(ext))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}
