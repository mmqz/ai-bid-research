//! veto-gt-dump —— issue #35 第一步（标注集）辅助工具
//!
//! 对 corpus 16 件（power_grid 真实语料）跑**生产解析路径**（与 bid-tools service.rs
//! import 路由同款：docx→引擎；pdf→引擎，Err 回退旧 parse_file），dump 全量块流
//! （idx / kind / locator / text / cells）并打宽模式候选标记——供标注员逐条裁决。
//!
//! 纪律：
//! - 只读 harness，不改 bidcore 任何源码（与 bid_health_check 同纪律）
//! - 候选池宁滥勿缺：20 词表 + 否决句式模式，**标注时人工裁决去假阳**
//! - 语料 corpus/ 二进制不入 git（provenance.json 为唯一溯源账本）
//!
//! 用法：veto-gt-dump <corpus_dir> <out_dir>

use std::path::{Path, PathBuf};
use std::collections::BTreeMap;

use bid_parse::ParsedFile;
use serde_json::json;

/// LingYan 20 词（veto_lexicon.json 字面口径，体检 truth 面同源）
const LEXICON: &[&str] = &[
    "无效", "否决", "拒收", "不予受理", "不合格", "取消", "废标", "作废", "放弃", "撤回",
    "拒绝", "不接受", "不被接收", "失效", "罚金", "违约金", "没收", "无权参加", "列入", "实质性响应",
];

/// 宽模式补充（句式面——词表外否决语义的候选扩展，标注员裁决）
const PATTERNS: &[(&str, &str)] = &[
    ("否则", "否则-续接处置"),
    ("视为", "视为-拟制处置"),
    ("有权拒", "有权拒-处置动词"),
    ("有权取", "有权取-处置动词"),
    ("有权否", "有权否-处置动词"),
    ("不予接", "不予接-处置动词"),
    ("不予评", "不予评-处置动词"),
    ("不予认", "不予认-处置动词"),
    ("不予退", "不予退-保证金"),
    ("不予签", "不予签-合同"),
    ("保证金不", "保证金不-处置"),
    ("按废标", "按废标-处置"),
    ("作无效", "作无效-处置"),
    ("按无效", "按无效-处置"),
    ("非实质性", "非实质性-响应"),
    ("未按要求", "未按要求-条件"),
    ("未按规定", "未按规定-条件"),
    ("未按招标", "未按招标-条件"),
    ("串通投标", "串通投标-违法"),
    ("弄虚作假", "弄虚作假-违法"),
    ("虚假", "虚假-违法"),
    ("造假", "造假-违法"),
    ("骗取", "骗取-违法"),
    ("行贿", "行贿-违法"),
    ("失信", "失信-信用"),
    ("黑名单", "黑名单-信用"),
    ("不良行为", "不良行为-信用"),
    ("取消其", "取消其-处置"),
    ("取消资", "取消资-处置"),
    ("废标处", "废标处-处置"),
    ("暗标", "暗标-形式评审"),
    ("密封", "密封-形式评审"),
    ("签章", "签章-形式评审"),
    ("签字盖章", "签字盖章-形式评审"),
    ("逾期", "逾期-程序"),
    ("截止", "截止-程序"),
    ("超期", "超期-程序"),
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("用法: veto-gt-dump <corpus_dir> <out_dir>");
        std::process::exit(2);
    }
    let corpus = PathBuf::from(&args[0]);
    let out = PathBuf::from(&args[1]);
    std::fs::create_dir_all(&out).expect("out dir");

    // 语料收集：corpus/*.pdf + corpus/doc/*.docx（与 bid_health_check 同布局）
    let mut files: Vec<PathBuf> = vec![];
    let mut tops: Vec<PathBuf> = list_files(&corpus, "pdf");
    files.append(&mut tops);
    let docdir = corpus.join("doc");
    if docdir.is_dir() {
        let mut docs = list_files(&docdir, "docx");
        files.append(&mut docs);
    }

    eprintln!("[veto-gt-dump] corpus={} files={}", corpus.display(), files.len());

    let mut summary = BTreeMap::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        eprintln!("---- dump {name}");

        // ---- 生产路径解析（镜像 bid-tools service.rs import 路由）
        let parsed: Result<ParsedFile, String> = match ext.as_str() {
            "pdf" => match bid_engine::parse_pdf_via_engine(path) {
                Ok(pf) => Ok(pf),
                Err(e) => {
                    eprintln!("    engine_pdf_err→legacy: {e}");
                    bid_parse::parse_file(path, &name).map_err(|e| e.to_string())
                }
            },
            "docx" => bid_engine::parse_docx_via_engine(path).map_err(|e| e.to_string()),
            other => Err(format!("SKIP: .{other}")),
        };

        let pf = match parsed {
            Ok(pf) => pf,
            Err(e) => {
                eprintln!("    ERR {e}");
                continue;
            }
        };

        // ---- 块流 dump + 候选标记
        let mut blocks = vec![];
        let mut n_lex = 0usize;
        let mut n_pat = 0usize;
        for b in &pf.blocks {
            let lex_hits: Vec<&str> = LEXICON.iter().filter(|w| b.text.contains(**w)).copied().collect();
            let pat_hits: Vec<&str> = PATTERNS
                .iter()
                .filter(|(p, _)| b.text.contains(p))
                .map(|(_, tag)| *tag)
                .collect();
            let is_candidate = !lex_hits.is_empty() || !pat_hits.is_empty();
            if !lex_hits.is_empty() { n_lex += 1; }
            if !pat_hits.is_empty() { n_pat += 1; }
            blocks.push(json!({
                "idx": b.idx,
                "kind": format!("{:?}", b.kind),
                "locator": b.locator,
                "page": parse_page(&b.locator),
                "text": b.text,
                "cells": b.cells,
                "lex_hits": lex_hits,
                "pat_hits": pat_hits,
                "candidate": is_candidate,
            }));
        }

        let doc = json!({
            "schema": "veto-gt-dump/1",
            "file": name,
            "sha256": pf.sha256,
            "pages": pf.pages,
            "tables": pf.tables,
            "blocks_total": blocks.len(),
            "candidates_lexicon": n_lex,
            "candidates_pattern": n_pat,
            "blocks": blocks,
        });
        let outp = out.join(format!("{name}.blocks.json"));
        std::fs::write(&outp, serde_json::to_vec(&doc).unwrap()).expect("write");
        summary.insert(name.clone(), json!({
            "blocks": blocks.len(), "lex": n_lex, "pat": n_pat,
            "lex_or_pat": blocks.iter().filter(|b| b["candidate"].as_bool().unwrap_or(false)).count(),
        }));
    }

    std::fs::write(out.join("_summary.json"), serde_json::to_vec_pretty(&json!(summary)).unwrap())
        .expect("summary");
    eprintln!("==== done，汇总 → {}/_summary.json", out.display());
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

/// locator 如「第10页 行10」→ 10；docx 章节型 locator → null（原样保留在 locator 字段）
fn parse_page(locator: &str) -> Option<u32> {
    let bytes = locator.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\xe7' && i + 2 < bytes.len() {
            // 「第」UTF-8 = E7 AC AC
            if bytes[i + 1] == b'\xac' && bytes[i + 2] == b'\xac' {
                let rest = &locator[i + 3..];
                let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if !num.is_empty() {
                    if let Ok(n) = num.parse() {
                        return Some(n);
                    }
                }
            }
        }
        i += 1;
    }
    None
}
