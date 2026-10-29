//! 诊断探针（E-RUN）：A-02 扫描件 OCR 面对账——逐页块数/字符数 + 第 1 页全文。
//! 用途：验证管线 OCR 与直接 tesseract 输出一致性（结论：逐字节一致——
//! 逐页对照 direct: TESSDATA_PREFIX=~/.tessdata tesseract <png> stdout -l chi_sim --psm 6）。
//! 属体检诊断面，非生产代码；不修改 crates/ 任何源码。

fn main() {
    let path = std::path::Path::new(
        "/home/z/my-project/bid-research/artifacts/bid-health-check/corpus/A-02_福建修订通知_扫描件.pdf",
    );
    let pf = bid_parse::parse_file(path, "A-02_福建修订通知_扫描件.pdf").expect("parse");
    println!("pages={} blocks={} ocr_blocks={} chars={}",
        pf.pages, pf.blocks.len(), pf.ocr_blocks,
        pf.blocks.iter().map(|b| b.text.chars().count()).sum::<usize>());
    let mut by_page: std::collections::BTreeMap<String, (usize, usize)> = std::collections::BTreeMap::new();
    for b in &pf.blocks {
        let page = b.locator.split_whitespace().next().unwrap_or("?").to_string();
        let e = by_page.entry(page).or_insert((0, 0));
        e.0 += 1;
        e.1 += b.text.chars().count();
    }
    for (p, (n, c)) in &by_page {
        println!("{p}: blocks={n} chars={c}");
    }
    println!("---- 第 1 页全文（对账 direct tesseract）----");
    for b in pf.blocks.iter().filter(|b| b.locator.contains("第1页")) {
        println!("{}", b.text);
    }
}
