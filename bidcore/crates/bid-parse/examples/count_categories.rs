//! #226 层 2 对照工具：逐文件统计分类分布（veto 计数基线/改后对照用）。
//!
//!     cargo run --release -p bid-parse --example count_categories -- ..\real-corpus
//!
//! 输出每个文件的条款总数与各分类计数——层 2 的验收要求「veto 数量合理
//! 下降（不是清零）」，这个工具直接给出改前/改后两列数字。

use std::collections::BTreeMap;
use std::path::Path;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "..".to_string());
    let dir = Path::new(&dir);
    // 递归收 PDF：语料按 water/telecom/gov-procurement 分类目录存放。
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

    println!("{:<46} {:>6} {:>6} {:>6} {:>6} {:>6}", "file", "total", "否决项", "资格", "评分", "商务");
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();
    for p in &entries {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        // 本机无 poppler：走纯 Rust 兜底（crates/bid-parse/src/pdf_text.rs），
        // 与 measure_extract 的 [兜底] 列同一条路——两边数字才可比。
        let res = bid_parse::pdf_text::parse_via_fallback(p, &name);
        let Ok(f) = res else { continue };
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
    }
    println!();
    println!("== 全语料合计 ==");
    for (k, v) in &totals {
        println!("  {k:<8} {v}");
    }
}