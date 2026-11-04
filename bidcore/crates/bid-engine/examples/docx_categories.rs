//! #233 DOCX 判据不对称的对照工具（改前基线/改后对照用）。
//!
//!     cargo run --release -p bid-engine --example docx_categories -- <real-corpus 目录>
//!
//! 两面输出（对齐 bid-parse 的 count_categories/dump_rule_statements）：
//! 1. 逐 DOCX 件的分类计数——层 2 接入 DOCX 前后的对照面；
//! 2. 文本形「规则说明」清单（excerpt 命中 is_rule_statement）——
//!    DOCX 未接层 2 时 = 改后将被降级分类的候选清单（改前基线的核心面）。

use std::collections::BTreeMap;
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
            } else if p.extension().is_some_and(|x| x == "docx") {
                entries.push(p);
            }
        }
    }
    entries.sort();

    println!("{:<46} {:>6} {:>6} {:>6} {:>6} {:>6}", "file", "total", "否决项", "资格", "评分", "商务");
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut n_rule = 0usize;
    for p in &entries {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let Ok(f) = bid_engine::parse_docx_via_engine(p) else {
            println!("{:<46} 解析失败", name.chars().take(44).collect::<String>());
            continue;
        };
        let reqs = bid_parse::extract::extract_requirements(std::slice::from_ref(&f));
        let mut by: BTreeMap<String, usize> = BTreeMap::new();
        for r in &reqs {
            *by.entry(r.category.clone()).or_default() += 1;
            *totals.entry(r.category.clone()).or_default() += 1;
        }
        println!(
            "{:<46} {:>6} {:>6} {:>6} {:>6} {:>6}",
            name.chars().take(44).collect::<String>(),
            reqs.len(),
            by.get("否决项").copied().unwrap_or(0),
            by.get("资格条件").copied().unwrap_or(0),
            by.get("评分项").copied().unwrap_or(0),
            by.get("商务要求").copied().unwrap_or(0),
        );
        // 规则说明候选清单（excerpt 命中 is_rule_statement——改后将被
        // 降级分类的候选，改前基线逐条人工复核面）
        for r in &reqs {
            let text = r
                .anchors
                .first()
                .map(|a| a.excerpt.clone())
                .unwrap_or_default();
            if bid_parse::veto_rule::is_rule_statement(&text) {
                n_rule += 1;
                println!(
                    "  [规则说明候选 {:>3}] {:<10} {}",
                    n_rule,
                    r.category,
                    text.chars().take(100).collect::<String>()
                );
            }
        }
    }
    println!();
    println!("== DOCX 语料合计 ==");
    for (k, v) in &totals {
        println!("  {k:<8} {v}");
    }
    println!("  规则说明候选合计 {n_rule} 条");
}
