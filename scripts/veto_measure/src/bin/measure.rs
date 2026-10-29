//! veto-measure —— issue #35 第三步前置：可复现测量器。
//!
//! 对 GT（artifacts/veto-gt/ground-truth.json，veto-gt/1）列出的每件语料跑
//! **生产路径解析**（pdf→引擎，Err 回退旧 parse_file；docx→引擎——与
//! bid-tools service.rs import 路由同款），再跑 extract_requirements，
//! 将 REQ 输出与 GT true 条款匹配，输出三层指标 + 漏检清单：
//!
//! - **L1 抽取层召回**：GT true 条款被任一 REQ「覆盖」的比例。
//!   覆盖判定（三通道，块对齐优先）：
//!   a) GT 条款块（clause_blocks ∪ main_block_idx）任一 ∈ REQ 锚定块集
//!      （anchor 的 (locator, excerpt) 对齐块 idx——碎片级块对齐）
//!   b) REQ anchors 拼接文本（规范化）完整包含 GT 条款文本（跨行拼接还原）
//!   c) REQ 某 anchor（规范化 ≥6 字）是 GT 条款文本的子串（行级碎片文本面兜底）
//! - **L2 否决分类召回**：被覆盖的 GT true 中，命中 REQ 存在 category=="否决项"
//! - **精确率（否决类）**：category=="否决项" 的 REQ 落在任一 GT true 条款内
//!   的比例（REQ 拼接是 GT 子串，或包含完整 GT 短句；<6 字碎屑不算 TP）
//!
//! 规范化 = 去全部空白（含全角空格 U+3000）——对齐 GT 标注时的跨行拼接口径。
//! 测量器只读 harness，不改 bidcore 任何源码（与 veto-gt dump/baseline 同纪律）。
//!
//! 用法：veto-measure <corpus_dir> <ground_truth.json> <out.json>

use std::collections::BTreeMap;
use std::path::PathBuf;

use bid_parse::ParsedFile;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct GtFile {
    file: String,
}

#[derive(Deserialize)]
struct GtAnn {
    id: String,
    file: String,
    page: serde_json::Value,
    #[allow(dead_code)]
    locator: serde_json::Value,
    #[allow(dead_code)]
    main_block_idx: serde_json::Value,
    #[serde(default)]
    clause_blocks: Vec<serde_json::Value>,
    text: String,
    is_veto: bool,
    #[serde(default)]
    category: String,
}

/// idx 宽容解析（GT 里可能是数字或字符串）。
fn idx_of(v: &serde_json::Value) -> Option<u64> {
    match v {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// 规范化：去全部空白字符（含全角空格）。
fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        eprintln!("用法: veto-measure <corpus_dir> <ground_truth.json> <out.json>");
        std::process::exit(2);
    }
    let corpus = PathBuf::from(&args[0]);
    let gt_path = PathBuf::from(&args[1]);
    let out_path = PathBuf::from(&args[2]);

    let gt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&gt_path).expect("read ground-truth"))
            .expect("parse ground-truth");
    let gt_files: Vec<GtFile> = serde_json::from_value(gt["files"].clone()).expect("gt files");
    let gt_anns: Vec<GtAnn> =
        serde_json::from_value(gt["annotations"].clone()).expect("gt annotations");
    eprintln!(
        "[veto-measure] gt files={} annotations={} true={}",
        gt_files.len(),
        gt_anns.len(),
        gt_anns.iter().filter(|a| a.is_veto).count()
    );

    // GT 按文件分组（只统计 true 条款）
    let mut gt_by_file: BTreeMap<String, Vec<&GtAnn>> = BTreeMap::new();
    for a in &gt_anns {
        if a.is_veto {
            gt_by_file.entry(a.file.clone()).or_default().push(a);
        }
    }

    let mut by_file = serde_json::Map::new();
    let mut missed: Vec<serde_json::Value> = vec![];
    let mut fp_list: Vec<serde_json::Value> = vec![];
    // 全局累计
    let mut tot_gt = 0usize;
    let mut tot_cov = 0usize;
    let mut tot_l2 = 0usize;
    let mut tot_veto_req = 0usize;
    let mut tot_veto_tp = 0usize;
    // 分组累计 [gt, covered, l2]
    let mut g: BTreeMap<&str, [usize; 3]> = BTreeMap::new();

    for gf in &gt_files {
        let name = &gf.file;
        let path = corpus.join(name);
        let path = if path.exists() {
            path
        } else {
            corpus.join("doc").join(name)
        };
        if !path.exists() {
            eprintln!("    SKIP（corpus 缺文件）{name}");
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        eprintln!("---- measure {name}");
        let parsed: Result<ParsedFile, String> = match ext.as_str() {
            "pdf" => match bid_engine::parse_pdf_via_engine(&path) {
                Ok(pf) => Ok(pf),
                Err(_e) => bid_parse::parse_file(&path, name).map_err(|e| e.to_string()),
            },
            "docx" => bid_engine::parse_docx_via_engine(&path).map_err(|e| e.to_string()),
            other => Err(format!("SKIP: .{other}")),
        };
        let pf = match parsed {
            Ok(pf) => pf,
            Err(e) => {
                eprintln!("    ERR {e}");
                continue;
            }
        };
        // 块流先留副本（extract_requirements 按值消费 pf）
        let blocks = pf.blocks.clone();
        let reqs = bid_parse::extract::extract_requirements(&[pf]);
        // 块对齐表：anchor (locator, excerpt) → 块 idx（块 idx 是引擎/解析器固化的
        // 稳定序号，与 GT 标注时 dump 的 idx 同源同序——生产路径解析确定性前提）
        use std::collections::BTreeSet;
        let mut anchor_idx: BTreeSet<u64> = BTreeSet::new();
        let mut unmatched_anchor = 0usize;
        for r in &reqs {
            for a in &r.anchors {
                let hit = blocks
                    .iter()
                    .find(|b| b.locator == a.locator && b.text == a.excerpt);
                match hit {
                    Some(b) => {
                        anchor_idx.insert(b.idx as u64);
                    }
                    None => unmatched_anchor += 1,
                }
            }
        }
        // REQ 预计算：拼接文本 + 各 anchor 规范化
        struct R {
            joined: String,
            anchors: Vec<String>,
            cat: String,
        }
        let rs: Vec<R> = reqs
            .iter()
            .map(|r| R {
                joined: norm(&r.anchors.iter().map(|a| a.excerpt.as_str()).collect::<String>()),
                anchors: r.anchors.iter().map(|a| norm(&a.excerpt)).collect(),
                cat: r.category.clone(),
            })
            .collect();

        let gts = match gt_by_file.get(name) {
            Some(v) => v,
            None => continue,
        };
        eprintln!(
            "    blocks={} anchors_matched={} unmatched={}",
            blocks.len(),
            anchor_idx.len(),
            unmatched_anchor
        );

        let mut cov = 0usize;
        let mut l2 = 0usize;
        for a in gts.iter() {
            let g = norm(&a.text);
            // L1 覆盖三通道（块对齐优先，文本包含兜底）：
            //   a) GT 条款块（clause_blocks ∪ main）任一 ∈ REQ 锚定块集（碎片级块对齐）
            //   b) REQ 拼接文本 ⊇ GT 全文（跨行拼接完整还原）
            //   c) REQ 某 anchor（≥6 字规范化）是 GT 全文子串（行级碎片文本面兜底）
            let gt_block_ids: Vec<u64> = a
                .clause_blocks
                .iter()
                .filter_map(idx_of)
                .chain(idx_of(&a.main_block_idx))
                .collect();
            let block_hit = gt_block_ids.iter().any(|i| anchor_idx.contains(i));
            let mut hit_veto = false;
            let mut covered_flag = block_hit;
            // 无论块命中与否都扫文本面：块命中时找否决分类 REQ，未命中时文本兜底
            for r in &rs {
                let full = r.joined.contains(&g);
                let frag = r
                    .anchors
                    .iter()
                    .any(|an| char_len(an) >= 6 && g.contains(an));
                if full || frag {
                    covered_flag = true;
                    if r.cat == "否决项" {
                        hit_veto = true;
                    }
                }
            }
            if covered_flag {
                cov += 1;
                if hit_veto {
                    l2 += 1;
                } else {
                    missed.push(json!({
                        "id": a.id, "file": a.file, "page": a.page,
                        "category": a.category, "text": a.text,
                        "miss_layer": "L2（已抽取，未分类为否决项）",
                    }));
                }
            } else {
                missed.push(json!({
                    "id": a.id, "file": a.file, "page": a.page,
                    "category": a.category, "text": a.text,
                    "miss_layer": "L1（未抽取到 REQ）",
                }));
            }
        }
        // 精确率（否决类 REQ 的 TP）
        let mut veto_n = 0usize;
        let mut veto_tp = 0usize;
        for r in &rs {
            if r.cat != "否决项" {
                continue;
            }
            veto_n += 1;
            if char_len(&r.joined) < 6 {
                continue; // 碎屑不算 TP
            }
            let tp = gts.iter().any(|a| {
                let g = norm(&a.text);
                g.contains(&r.joined) || (r.joined.contains(&g) && char_len(&g) >= 6)
            });
            if tp {
                veto_tp += 1;
            } else {
                fp_list.push(json!({
                    "file": name,
                    "joined": r.joined.chars().take(120).collect::<String>(),
                }));
            }
        }

        let key: &str = if name.ends_with(".pdf") { "pdf" } else { "docx" };
        let e = g.entry(key).or_insert([0, 0, 0]);
        e[0] += gts.len();
        e[1] += cov;
        e[2] += l2;
        tot_gt += gts.len();
        tot_cov += cov;
        tot_l2 += l2;
        tot_veto_req += veto_n;
        tot_veto_tp += veto_tp;

        by_file.insert(
            name.clone(),
            json!({
                "gt_true": gts.len(),
                "covered_L1": cov,
                "veto_cls_L2": l2,
                "l1_recall": if gts.is_empty() { 0.0 } else { cov as f64 / gts.len() as f64 },
                "l2_recall": if gts.is_empty() { 0.0 } else { l2 as f64 / gts.len() as f64 },
                "reqs_total": rs.len(),
                "reqs_veto_cls": veto_n,
                "veto_tp": veto_tp,
                "veto_precision": if veto_n == 0 { 0.0 } else { veto_tp as f64 / veto_n as f64 },
            }),
        );
        eprintln!(
            "    gt={} covered={} l2={} veto_req={} tp={}",
            gts.len(),
            cov,
            l2,
            veto_n,
            veto_tp
        );
    }

    let grp_out = |k: &str| -> serde_json::Value {
        match g.get(k) {
            Some([n, c, l2]) => json!({
                "gt_true": n, "covered": c, "l2": l2,
                "l1_recall": if *n == 0 { 0.0 } else { *c as f64 / *n as f64 },
                "l2_recall": if *n == 0 { 0.0 } else { *l2 as f64 / *n as f64 },
            }),
            None => serde_json::Value::Null,
        }
    };

    let report = json!({
        "schema": "veto-measure/1",
        "gt_path": gt_path.display().to_string(),
        "overall": {
            "gt_true_total": tot_gt,
            "covered_L1": tot_cov,
            "veto_cls_L2": tot_l2,
            "extraction_recall_L1": if tot_gt == 0 { 0.0 } else { tot_cov as f64 / tot_gt as f64 },
            "veto_class_recall_L2": if tot_gt == 0 { 0.0 } else { tot_l2 as f64 / tot_gt as f64 },
            "reqs_veto_cls": tot_veto_req,
            "veto_tp": tot_veto_tp,
            "precision_veto_cls": if tot_veto_req == 0 { 0.0 } else { tot_veto_tp as f64 / tot_veto_req as f64 },
        },
        "pdf_group": grp_out("pdf"),
        "docx_group": grp_out("docx"),
        "by_file": by_file,
        "missed_total": missed.len(),
        "missed": missed,
        "fp_total": fp_list.len(),
        "fp": fp_list,
    });
    std::fs::write(&out_path, serde_json::to_vec(&report).expect("serialize")).expect("write out");
    eprintln!(
        "==== done L1={:.4} L2={:.4} P={:.4} missed={} → {}",
        if tot_gt == 0 { 0.0 } else { tot_cov as f64 / tot_gt as f64 },
        if tot_gt == 0 { 0.0 } else { tot_l2 as f64 / tot_gt as f64 },
        if tot_veto_req == 0 { 0.0 } else { tot_veto_tp as f64 / tot_veto_req as f64 },
        missed.len(),
        out_path.display()
    );
}
