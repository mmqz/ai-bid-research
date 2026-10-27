//! title_impact —— issue #2 · B2.5 标题识别缺失对标书业务的影响面量化。
//!
//! 方法（不修改 bidcore/m1 任何源码——path 依赖调用真实 extract_requirements）：
//!   V0 基线：加载 baseline/parse 的真实件解析块流 → bid_parse::extract_requirements 真跑；
//!   V1 标题全丢：剔除「标题行」（extract.rs 章节上下文正则的镜像判定——数据准备层，
//!      非语义复刻）→ 同一真跑入口再抽；
//!   V2 A2 编号污染：docx 块流中编号行按引擎 TR-2D 实测签名（"1.1"→"11.1"：Word
//!      自动编号与文本编号叠加）前置复制前缀 → 同一真跑入口再抽；
//!   度量：REQ 总量/类别分布/类别翻转/条款号污染率/GT 24 召回输入（判分由
//!      scripts/title_impact_score.py 复用 real_eval_score.py 同源对齐逻辑完成）。
//!
//! 用法：engine-probe-title <repo_root> <out_json>

use bid_domain::Requirement;
use bid_parse::{extract::extract_requirements, Block, BlockKind, ParsedFile};
use std::collections::BTreeMap;
use std::path::Path;

fn kind_of(s: &str) -> BlockKind {
    match s {
        "para" => BlockKind::Para,
        "table_row" => BlockKind::TableRow,
        "pdf_line" => BlockKind::PdfLine,
        "ocr_line" => BlockKind::OcrLine,
        "xlsx_row" => BlockKind::XlsxRow,
        _ => BlockKind::Para,
    }
}

fn load_parsed(json_path: &Path) -> Option<(String, ParsedFile)> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(json_path).ok()?).ok()?;
    if v.get("status").map(|s| s.as_str() != Some("ok")).unwrap_or(true) {
        return None;
    }
    let raw = v.get("file")?.as_str()?.to_string();
    // extract 按 A-/B-/C- 前缀分发（extract.rs:33-38）——baseline 记录的是相对
    // 路径，此处取 basename 还原 extract 层接口约定（realfiles example 同口径）。
    let name = raw.rsplit('/').next().unwrap_or(&raw).to_string();
    let blocks: Vec<Block> = v
        .get("blocks")?
        .as_array()?
        .iter()
        .map(|b| Block {
            idx: b.get("idx").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
            kind: kind_of(b.get("kind").and_then(|x| x.as_str()).unwrap_or("para")),
            locator: b.get("locator").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            text: b.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            cells: b
                .get("cells")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|c| c.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default(),
        })
        .collect();
    Some((
        name.clone(),
        ParsedFile {
            name,
            sha256: v.get("sha256").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            pages: v.get("pages").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
            tables: 0,
            has_text_layer: v.get("has_text_layer").and_then(|x| x.as_bool()).unwrap_or(false),
            ocr_blocks: 0,
            blocks,
            components: Vec::new(),
            parse_summary: String::new(),
        },
    ))
}

/// 标题行判定（extract.rs:57-60 章节上下文正则的镜像——数据准备层）。
fn is_heading_line(text: &str) -> bool {
    let t = text.trim();
    // 目录行防御同款（extract.rs:87-90）
    if t.matches('.').count() >= 6 || t.contains("……") {
        return false;
    }
    let re_chapter = regex::Regex::new(r"^第\s*([0-9一二三四五六七八九十百]+)\s*章").unwrap();
    let re_subsection = regex::Regex::new(r"^[一二三四五六七八九十]+、").unwrap();
    re_chapter.is_match(t) || (re_subsection.is_match(t) && t.chars().count() <= 40)
}

/// A2 编号污染（TR-2D P1-1 实测签名：Word 自动编号叠加文本编号 "1.1"→"11.1"）。
fn corrupt_a2(text: &str) -> String {
    let re_num = regex::Regex::new(r"^(\d+(?:\.\d+)*)").unwrap();
    if let Some(m) = re_num.captures(text) {
        let head = m[1].to_string();
        let rest = &text[head.len()..];
        return format!("1{head}{rest}");
    }
    text.to_string()
}

fn group_of(path_str: &str) -> &'static str {
    if path_str.contains("water") {
        "water"
    } else if path_str.contains("telecom") {
        "telecom"
    } else if path_str.contains("corpus-A") {
        "fixtures-A"
    } else if path_str.contains("corpus-R") {
        "fixtures-R"
    } else {
        "fixtures-other"
    }
}

fn by_category(reqs: &[Requirement]) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for r in reqs {
        *m.entry(r.category.clone()).or_insert(0usize) += 1;
    }
    m
}

fn main() {
    let repo = std::env::args().nth(1).expect("repo root");
    let out_path = std::env::args().nth(2).expect("out json");
    let root = Path::new(&repo).join("baseline/parse");
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for dir in ["real-corpus/water/files", "real-corpus/telecom/parse-input", "fixtures/corpus-A/files", "fixtures/corpus-R"] {
        let d = root.join(dir);
        if d.is_dir() {
            let mut v = walk(&d);
            v.sort();
            paths.extend(v);
        }
    }
    let mut files: Vec<(String, String, ParsedFile)> = Vec::new(); // (group, relpath, parsed)
    let mut skipped = 0usize;
    for p in &paths {
        if let Some((name, pf)) = load_parsed(p) {
            let rel = p.strip_prefix(&root).unwrap_or(p).to_string_lossy().to_string();
            files.push((group_of(&rel).to_string(), rel, pf));
            let _ = name;
        } else {
            skipped += 1;
        }
    }

    let mut report: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    report.insert("loaded_files".into(), serde_json::json!(files.len()));
    report.insert("skipped_non_ok".into(), serde_json::json!(skipped));
    report.insert("files_by_group".into(), {
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        for (g, _, _) in &files {
            *m.entry(g.clone()).or_insert(0) += 1;
        }
        serde_json::json!(m)
    });

    // ---- V0 基线
    let v0_files: Vec<ParsedFile> = files.iter().map(|(_, _, pf)| clone_pf(pf)).collect();
    let v0 = extract_requirements(&v0_files);
    // ---- V1 标题全丢（剔除标题行）
    let v1_files: Vec<ParsedFile> = files
        .iter()
        .map(|(_, _, pf)| {
            let mut f = clone_pf(pf);
            let (kept, removed): (Vec<Block>, Vec<Block>) =
                f.blocks.clone().into_iter().partition(|b| !is_heading_line(&b.text));
            f.blocks = reindex(kept);
            f.parse_summary = format!("headings_removed={}", removed.len());
            f
        })
        .collect();
    let v1 = extract_requirements(&v1_files);
    // ---- V2 A2 编号污染（仅 docx 块面：Para/TableRow——引擎 docx 导入链产物）
    let v2_files: Vec<ParsedFile> = files
        .iter()
        .map(|(_, _, pf)| {
            let mut f = clone_pf(pf);
            let is_docx = f.blocks.iter().any(|b| matches!(b.kind, BlockKind::Para | BlockKind::TableRow));
            if is_docx {
                let mut n = 0usize;
                let re_num = regex::Regex::new(r"^\d+(?:\.\d+)+").unwrap();
                let blocks = std::mem::take(&mut f.blocks);
                let mut nb = Vec::with_capacity(blocks.len());
                for mut b in blocks {
                    let t = b.text.trim().to_string();
                    if re_num.is_match(&t) {
                        b.text = corrupt_a2(&t);
                        n += 1;
                    }
                    nb.push(b);
                }
                f.blocks = nb;
                f.parse_summary = format!("a2_corrupted_lines={n}");
            } else {
                f.parse_summary = "a2_not_applied_pdf".into();
            }
            f
        })
        .collect();
    let v2 = extract_requirements(&v2_files);

    // ---- 标题行统计（V0 各组被识别为标题的行数）
    let mut heading_stats: BTreeMap<String, usize> = BTreeMap::new();
    for (g, _, pf) in &files {
        let n = pf.blocks.iter().filter(|b| is_heading_line(&b.text)).count();
        *heading_stats.entry(g.clone()).or_insert(0) += n;
    }

    // ---- 配对工具
    fn pair<'a>(a: &'a [Requirement], b: &'a [Requirement]) -> Vec<(&'a Requirement, &'a Requirement)> {
        use std::collections::HashMap;
        let mb: HashMap<_, _> = b
            .iter()
            .map(|r| ((r.source_file.clone(), r.anchors.first().map(|x| x.excerpt.clone()).unwrap_or_default()), r))
            .collect();
        a.iter()
            .filter_map(|r| {
                let k = (r.source_file.clone(), r.anchors.first().map(|x| x.excerpt.clone()).unwrap_or_default());
                mb.get(&k).map(|r2| (r, *r2))
            })
            .collect()
    }
    let pairs01 = pair(&v0, &v1);
    let flipped01 = pairs01.iter().filter(|(a, b)| a.category != b.category).count();
    // V2 配对按文件内序数（V2 只改文本前缀不改抽取顺序——v0_n==v2_n 为前提断言）
    {
        use std::collections::BTreeMap as BM;
        let mut by_file_v0: BM<&str, Vec<&Requirement>> = BM::new();
        for r in &v0 { by_file_v0.entry(r.source_file.as_str()).or_default().push(r); }
        let mut by_file_v2: BM<&str, Vec<&Requirement>> = BM::new();
        for r in &v2 { by_file_v2.entry(r.source_file.as_str()).or_default().push(r); }
        assert_eq!(v0.len(), v2.len(), "V2 不增删 REQ（顺序配对前提）");
        let mut matched = 0usize;
        let mut corrupted = 0usize;
        let mut corrupted_docx = 0usize;
        let mut docx_matched = 0usize;
        for (fname, a) in &by_file_v0 {
            let b = match by_file_v2.get(fname) { Some(b) if b.len() == a.len() => b, _ => continue };
            for (ra, rb) in a.iter().zip(b.iter()) {
                matched += 1;
                if ra.clause_no != rb.clause_no { corrupted += 1; }
                if fname.ends_with(".docx") {
                    docx_matched += 1;
                    if ra.clause_no != rb.clause_no { corrupted_docx += 1; }
                }
            }
        }
        report.insert("v0_v2_ordinal_matched".into(), serde_json::json!(matched));
        report.insert("v0_v2_clause_no_corrupted".into(), serde_json::json!(corrupted));
        report.insert("v0_v2_docx_matched".into(), serde_json::json!(docx_matched));
        report.insert("v0_v2_docx_clause_corrupted".into(), serde_json::json!(corrupted_docx));
    }

    report.insert("heading_lines_by_group".into(), serde_json::json!(heading_stats));
    report.insert("v0_reqs_n".into(), serde_json::json!(v0.len()));
    report.insert("v1_reqs_n".into(), serde_json::json!(v1.len()));
    report.insert("v2_reqs_n".into(), serde_json::json!(v2.len()));
    report.insert("v0_by_category".into(), serde_json::json!(by_category(&v0)));
    report.insert("v1_by_category".into(), serde_json::json!(by_category(&v1)));
    report.insert("v2_by_category".into(), serde_json::json!(by_category(&v2)));
    report.insert("v0_v1_matched_pairs".into(), serde_json::json!(pairs01.len()));
    report.insert("v0_v1_category_flipped".into(), serde_json::json!(flipped01));


    // ---- REQ dump（判分器输入）
    let dump = |reqs: &[Requirement]| -> serde_json::Value {
        serde_json::json!(reqs
            .iter()
            .map(|r| serde_json::json!({
                "id": r.id, "category": r.category, "clause_no": r.clause_no,
                "anchors": r.anchors.iter().map(|a| serde_json::json!({
                    "file": a.file, "kind": a.kind, "locator": a.locator, "excerpt": a.excerpt,
                })).collect::<Vec<_>>(),
                "params": r.params, "source_file": r.source_file,
            }))
            .collect::<Vec<_>>())
    };
    report.insert("v0_dump".into(), dump(&v0));
    report.insert("v1_dump".into(), dump(&v1));
    report.insert("v2_dump".into(), dump(&v2));

    std::fs::write(&out_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    eprintln!(
        "files={} v0={} v1={} v2={} flips={}/{}",
        files.len(),
        v0.len(),
        v1.len(),
        v2.len(),
        flipped01,
        pairs01.len(),
    );
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else if p.extension().map(|x| x == "json").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    out
}

fn clone_pf(pf: &ParsedFile) -> ParsedFile {
    ParsedFile {
        name: pf.name.clone(),
        sha256: pf.sha256.clone(),
        pages: pf.pages,
        tables: pf.tables,
        has_text_layer: pf.has_text_layer,
        ocr_blocks: pf.ocr_blocks,
        blocks: pf.blocks.clone(),
        components: pf.components.clone(),
        parse_summary: pf.parse_summary.clone(),
    }
}

fn reindex(mut blocks: Vec<Block>) -> Vec<Block> {
    for (i, b) in blocks.iter_mut().enumerate() {
        b.idx = i;
    }
    blocks
}
