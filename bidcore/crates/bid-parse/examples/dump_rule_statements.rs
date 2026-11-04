//! 列出被判为「规则说明」的条款原文（层 2 改动的人工复核面）。
//!
//!     cargo run --release -p bid-parse --example dump_rule_statements -- ..\real-corpus
//!
//! 输出：每条被判为规则说明的段落 + 它最终落到的分类。人工逐条看
//! 「这个判定对不对」——veto 数量下降是结果，判定对不对才是验收。

use std::path::Path;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "..".to_string());
    let dir = Path::new(&dir);
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "pdf") {
                entries.push(p);
            }
        }
    }
    entries.sort();

    let mut n = 0usize;
    for p in &entries {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let Ok(f) = bid_parse::pdf_text::parse_via_fallback(p, &name) else { continue };
        let reqs = bid_parse::extract::extract_requirements(std::slice::from_ref(&f));
        for r in &reqs {
            // 条款正文在 anchors[].excerpt（创建时固化的原文片段）。
            let text = r
                .anchors
                .first()
                .map(|a| a.excerpt.clone())
                .unwrap_or_default();
            if bid_parse::veto_rule::is_rule_statement(&text) {
                n += 1;
                println!("[{:>4}] {:<10} {}", n, r.category, text.chars().take(100).collect::<String>());
            }
        }
    }
    println!("\n合计 {n} 条被判为规则说明");
}
