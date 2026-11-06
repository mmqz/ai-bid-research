//! #272 跨平台章节结构探针（基线采集 / 验收对照两用）。
//!
//! 采集面（每份语料 dump 一份 JSON）：
//! - `chapters`：当前章识别面所见（re_chapter + NFKC——与 extract.rs
//!   `extract_main_pdf` 同口径逐块扫描），含块号与原文；
//! - `numbered_top_heads`：裸数字顶层头（`N.标题` 形态——询价骨架面）；
//! - `reqs_by_cat`：真实抽取（extract_requirements）的类别分布与 REQ 总数
//!   ——「识别结果不变」的可机 diff 锚；
//! - `reqs_digest`：(clause_no, category, anchor locator) 序列——批次一
//!   五份红线对照的逐条指纹。
//!
//! 用法：环境变量 `PROBE_272_OUT=<dir>` 时 dump（无环境变量则只跑 smoke
//! 断言——CI 不落盘）。基线采集跑在 main，改动后重跑 diff。
//! 逐份语料在位断言与正式验收在 `cross_platform_structure.rs`（本文件是
//! 采集/诊断工具，验收不依赖本文件的全量跑——9 份 PDF 约 11 分钟）。
//!
//! 防过拟合标注：本探针覆盖 3 个平台来源（黑龙江政采 5 / 内蒙古政采 3 /
//! 询价 1 + 吉林 docx 1——doc 面引擎不支持），未验证其它省份。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bid_parse::extract::extract_requirements;
use bid_parse::{parse_file, BlockKind};
use serde_json::json;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while p.join("fixtures").read_dir().is_err() {
        p = p
            .parent()
            .map(|x| x.to_path_buf())
            .expect("repo root not found");
    }
    p
}

fn nfkc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkc().collect()
}

/// 主件 PDF（跳过附件/ 目录——#271 附件格式件不在本探针面）。
fn main_pdfs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = dir.read_dir() {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("pdf")
                && e.file_type().map(|t| t.is_file()).unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn probe_file(path: &Path) -> serde_json::Value {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let pf = match parse_file(path, &name) {
        Ok(pf) => pf,
        Err(e) => return json!({"file": name, "parse_error": e}),
    };

    // ── 章识别面（与 extract_main_pdf 同口径：re_chapter + NFKC、TOC 行防御）──
    let re_chapter = regex::Regex::new(r"^第\s*([0-9一二三四五六七八九十百]+)\s*章").unwrap();
    let mut chapters: Vec<serde_json::Value> = Vec::new();
    for b in &pf.blocks {
        if !matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine) {
            continue;
        }
        let t = b.text.trim();
        if t.matches('.').count() >= 6 || t.contains("……") {
            continue; // 目录行防御（同主循环）
        }
        if re_chapter.is_match(&nfkc(t)) {
            chapters.push(json!({"idx": b.idx, "text": t}));
        }
    }

    // ── 裸数字顶层头（`N.标题` / `N．标题`——询价骨架面；≤30 字短行，
    //    排除后随数字的多级头 2.1——那是第二层）──
    let re_top_num =
        regex::Regex::new(r"^([0-9]{1,2})[.．、]\s*([^0-9.．、\s].{0,28})$").unwrap();
    let mut numbered_top_heads: Vec<serde_json::Value> = Vec::new();
    for b in &pf.blocks {
        if !matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine) {
            continue;
        }
        let t = b.text.trim();
        if let Some(c) = re_top_num.captures(t) {
            numbered_top_heads.push(json!({
                "idx": b.idx,
                "no": c[1].to_string(),
                "title": c[2].trim().to_string(),
            }));
        }
    }

    // ── 真实抽取面（识别结果的机 diff 锚）──
    let reqs = extract_requirements(std::slice::from_ref(&pf));
    let mut by_cat: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &reqs {
        *by_cat.entry(r.category.as_str()).or_default() += 1;
    }
    let digest: Vec<serde_json::Value> = reqs
        .iter()
        .map(|r| {
            json!({
                "clause": r.clause_no.clone().unwrap_or_default(),
                "cat": r.category,
                "loc": r.anchors.first().map(|a| a.locator.clone()).unwrap_or_default(),
                // 指纹：锚文本 NFKC 去空白的前 48 字（块对齐面不动原文）
                "fp": r.anchors.first()
                    .map(|a| nfkc(&a.excerpt)
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .take(48)
                        .collect::<String>())
                    .unwrap_or_default(),
            })
        })
        .collect();

    json!({
        "file": name,
        "pages": pf.pages,
        "blocks": pf.blocks.len(),
        "chapters": chapters,
        "chapter_n": chapters.len(),
        "numbered_top_heads": numbered_top_heads,
        "numbered_top_n": numbered_top_heads.len(),
        "req_total": reqs.len(),
        "reqs_by_cat": by_cat,
        "reqs_digest": digest,
    })
}

#[test]
fn probe_structure_baseline() {
    // poppler 门控（issue #9 纪律：环境差异不伪装业务结果——Windows CI 无
    // poppler 时 PDF 解析不可用，响亮 SKIP 而非假绿/假红）
    if bid_parse::env_preflight::skip_if_env_missing(
        "structure_probe_272",
        bid_parse::env_preflight::POPPLER_TEXT_TOOLS,
    ) {
        return;
    }
    let root = repo_root();
    let groups = [
        ("batch1", root.join("fixtures/real-tender-batch1")),
        ("batch2", root.join("fixtures/real-tender-batch2")),
    ];
    let out_dir = std::env::var("PROBE_272_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("probe-272"));
    std::fs::create_dir_all(&out_dir).unwrap();

    let mut probed = 0usize;
    let mut parsed_err = 0usize;
    for (group, dir) in &groups {
        if !dir.is_dir() {
            continue;
        }
        let mut subs: Vec<PathBuf> = dir
            .read_dir()
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        subs.sort();
        for sub in subs {
            let id_name = sub.file_name().unwrap().to_string_lossy().to_string();
            if let Ok(only) = std::env::var("PROBE_272_ONLY") {
                if !id_name.contains(&only) {
                    continue;
                }
            }
            for pdf in main_pdfs(&sub) {
                let v = probe_file(&pdf);
                if v.get("parse_error").is_some() {
                    parsed_err += 1;
                }
                probed += 1;
                let id = id_name.clone();
                let out = out_dir.join(format!("{group}-{id}.json"));
                std::fs::write(&out, serde_json::to_string_pretty(&v).unwrap()).unwrap();
                eprintln!(
                    "[probe-272] {} / {}: chapters={} numbered_top={} reqs={} cats={:?}",
                    group,
                    id,
                    v["chapter_n"].as_u64().unwrap_or(0),
                    v["numbered_top_n"].as_u64().unwrap_or(0),
                    v["req_total"].as_u64().unwrap_or(0),
                    v["reqs_by_cat"]
                );
            }
        }
    }
    // 语料在位性：全量模式（无过滤）下必须探到至少 1 份主件（全部缺件
    // = 环境异常，不静默）；逐份在位断言由 cross_platform_structure.rs
    // 精确承载（缺件逐个 panic）。probe 本职是基线采集，不重复重断言。
    if std::env::var("PROBE_272_ONLY").is_err() {
        assert!(probed >= 1, "语料全缺：一份主件都没探到（fixtures 不在仓？）");
        assert_eq!(parsed_err, 0, "有主件解析失败——先看 parse_error 字段");
    }
}
