//! engine-probe-dump —— issue #2 · T1：引擎侧全量块 JSON 产出（双跑差分供数）
//!
//! 与主探针（`engine-probe`，报告入库 `artifacts/engine-probe/report.json`）不同：
//! 本二进制按文件**全量**落盘引擎原生读面，经 harness 侧最小机械映射
//! （`ParagraphData.text` → para 块 / `TableData.rows` → table_row 块——
//! `docs/engine-bid-mapping.md` §1 前两行的机械部分）转成 bid 可比形状，
//! 供 `scripts/engine_diff.py` 双跑差分。**产品映射 = B3 的 bid-engine 实装**，
//! 本 bin 是验证 harness（scripts/ 纪律，非产品代码）。
//!
//! 用法：`engine-probe-dump <repo_root> <out_dir> <file1> [file2 ...]`
//! 输出：`<out_dir>/<basename>.engine.json`
//!   {file, status, blocks: [{idx, kind, locator, text, cells}]}
//!   拒绝件：{file, status: "rejected", error_text}

use std::collections::BTreeMap;
use std::path::Path;

type Obj = BTreeMap<String, String>;

fn dump_docx(bytes: &[u8]) -> Result<serde_json::Value, String> {
    let d = mdc_office_io::docx::read_docx_from_bytes(bytes).map_err(|e| format!("{e}"))?;
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let mut p_no = 0usize;
    let mut t_no = 0usize;
    for b in &d.blocks {
        match b {
            mdc_office_io::docx::Block::Paragraph(p) => {
                p_no += 1;
                blocks.push(serde_json::json!({
                    "idx": blocks.len(),
                    "kind": "para",
                    "locator": format!("段{p_no}"),
                    "text": p.text(),
                    "cells": [],
                }));
            }
            mdc_office_io::docx::Block::Table(t) => {
                t_no += 1;
                for (ri, row) in t.rows.iter().enumerate() {
                    let cells: Vec<String> =
                        row.iter().map(|c| c.clone()).collect::<Vec<_>>();
                    let text = cells
                        .iter()
                        .filter(|c| !c.trim().is_empty())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" ");
                    blocks.push(serde_json::json!({
                        "idx": blocks.len(),
                        "kind": "table_row",
                        "locator": format!("表{t_no} 行{}", ri + 1),
                        "text": text,
                        "cells": cells,
                    }));
                }
            }
        }
    }
    let mut components: Vec<Obj> = Vec::new();
    let mut ledger = |component: &str, present: usize, extracted: usize, note: &str| {
        if present > 0 {
            let mut o = BTreeMap::new();
            o.insert("component".into(), component.to_string());
            o.insert("present".into(), present.to_string());
            o.insert("extracted".into(), extracted.to_string());
            o.insert("note".into(), note.to_string());
            components.push(o);
        }
    };
    ledger(
        "页眉",
        d.headers.len(),
        0,
        "引擎读面在位（HeaderFooterData）；harness dump 不入块流——与旧实现口径一致",
    );
    ledger(
        "页脚",
        d.footers.len(),
        0,
        "同上",
    );
    ledger(
        "图片",
        d.images.len(),
        0,
        "容器级枚举（图片位置不入块模型——引擎设计登记）",
    );
    ledger(
        "警告",
        d.warnings.len(),
        0,
        "引擎兼容性警告（B3 将透传 parse_summary/components note）",
    );
    Ok(serde_json::json!({
        "status": "ok",
        "blocks": blocks,
        "components": components,
    }))
}

fn dump_xlsx(bytes: &[u8]) -> Result<serde_json::Value, String> {
    let b = mdc_office_io::io::read_book_from_bytes(bytes).map_err(|e| format!("{e}"))?;
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    for s in &b.sheets {
        for (ri, row) in s.values.rows().enumerate() {
            let cells: Vec<String> = row.iter().map(|c| c.to_string()).collect();
            if cells.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            let text = cells
                .iter()
                .filter(|c| !c.trim().is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            blocks.push(serde_json::json!({
                "idx": blocks.len(),
                "kind": "xlsx_row",
                "locator": format!("{} 行{}", s.name, ri + 1),
                "text": text,
                "cells": cells,
            }));
        }
    }
    Ok(serde_json::json!({
        "status": "ok",
        "blocks": blocks,
        "components": [],
    }))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("用法: engine-probe-dump <repo_root> <out_dir> <file...>");
        std::process::exit(2);
    }
    let out_dir = Path::new(&args[2]);
    std::fs::create_dir_all(out_dir).expect("创建输出目录");
    for f in &args[3..] {
        let path = Path::new(f);
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("SKIP {f}: {e}");
                continue;
            }
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let parsed = if name.to_lowercase().ends_with(".docx") {
            dump_docx(&bytes)
        } else if name.to_lowercase().ends_with(".xlsx") {
            dump_xlsx(&bytes)
        } else {
            Err(format!("dump 通道未支持: {name}（pdf 见 B3 适配器；T1 工具 schema 就绪）"))
        };
        let value = match parsed {
            Ok(mut v) => {
                v["file"] = serde_json::Value::String(name.clone());
                v
            }
            Err(e) => serde_json::json!({
                "file": name,
                "status": "rejected",
                "error_text": e.chars().take(400).collect::<String>(),
                "blocks": [],
            }),
        };
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let out = out_dir.join(format!("{stem}.engine.json"));
        std::fs::write(&out, serde_json::to_string_pretty(&value).unwrap()).expect("写出");
        let n = value["blocks"].as_array().map(|a| a.len()).unwrap_or(0);
        println!("{} -> {} (blocks={n}, status={})", f, out.display(), value["status"].as_str().unwrap_or("?"));
    }
}
