//! real_eval.rs —— P2 真实文件评测 runner（bid-tools bin）。
//!
//! 输入：真实公开招标文件（test_samples/water_tender_docs——政府采购公开文件，
//! 来源与许可见评测报告 source_notes）。这是**合成覆盖率的外部效度检验**：
//! corpus-A 全绿只证明合成契约成立，本评测回答「真实文件的解析/抽取面
//! 表现如何」——漏项/误报/引用支持率以人工 ground truth 草案为准。
//!
//! 解析路由（B4，与 service.rs 同款）：docx/xlsx → bid-engine 引擎门面；
//! pdf → 引擎优先（pdfium 文本层）+ 旧路径回退（pdftotext/OCR 补遗链）；
//! 其它扩展名 → `parse_file` 显式拒绝（zip 容器样本的「预期显式拒绝」
//! 行为保持）。
//!
//! 诚实登记：
//! - ground truth 为 AI 初标草案（待人工验收——验收前指标为「草案对齐结果」，
//!   不得外推为产品验收指标）；
//! - 不外推合成覆盖率（corpus-A 的 100% 召回不适用真实文件面）。
//!
//! 用法：cargo run -p bid-tools --bin real_eval -- <samples_dir> <out_dir>

use bid_domain::Requirement;
use bid_parse::{extract::extract_requirements, parse_file, ParsedFile};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// B4：service.rs 同款扩展名路由（docx/xlsx 引擎；pdf 引擎+回退；其它报错）。
fn parse_routed(path: &Path, name: &str) -> Result<ParsedFile, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "docx" => bid_engine::parse_docx_via_engine(path).map_err(|e| e.to_string()),
        "xlsx" => bid_engine::parse_xlsx_via_engine(path).map_err(|e| e.to_string()),
        // pdf 引擎通道（pdfium 文本层）——扫描件（MappingGap）或无 pdfium
        // （EngineFailed）回退旧 pdftotext/OCR 路径（映射表 §3）
        "pdf" => bid_engine::parse_pdf_via_engine(path)
            .or_else(|_| parse_file(path, name)),
        _ => parse_file(path, name),
    }
}

struct Sample {
    path: PathBuf,
    alias: &'static str,
    kind_desc: &'static str,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("用法: real_eval <samples_dir> <out_dir>");
        std::process::exit(2);
    }
    let samples_dir = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out_dir).unwrap();
    std::fs::create_dir_all(out_dir.join("parse")).unwrap();
    std::fs::create_dir_all(out_dir.join("extract")).unwrap();

    let samples = [
        Sample {
            path: samples_dir.join("01_塔里木河流域希尼尔水库除险加固工程坝基防渗处理施工标_招标文件.pdf"),
            alias: "A-塔里木水库除险加固招标文件.pdf",
            kind_desc: "主文件·文本层 PDF·284 页",
        },
        Sample {
            path: samples_dir.join("04_某水利工程施工标段_施工招标文件.pdf"),
            alias: "A-某水利施工标段招标文件.pdf",
            kind_desc: "主文件·文本层 PDF·142 页（GT 主体）",
        },
        Sample {
            path: samples_dir.join("05_伊犁州奎屯市东郊水库工程项目法人全过程质量检测_招标文件.pdf"),
            alias: "A-伊犁水库质量检测招标文件.pdf",
            kind_desc: "主文件·文本层 PDF·76 页",
        },
        Sample {
            path: samples_dir.join("02_福建省水利工程电子招标文件示范文本修订通知.pdf"),
            alias: "C-福建修订通知-扫描件.pdf",
            kind_desc: "补遗·纯扫描 PDF·8 页（OCR 通道真实样本）",
        },
        Sample {
            path: samples_dir.join("03_北京市水利工程电子招标投标示范文本文件2025版.zip"),
            alias: "B3-北京示范文本2025.zip",
            kind_desc: "不支持格式验证样本（zip 容器，预期显式拒绝）",
        },
    ];

    let mut parsed: Vec<ParsedFile> = Vec::new();
    let mut report = serde_json::json!({
        "eval": "P2-real-file",
        "semantics": "INTEGRATION_TESTED 解析面（真实文件 × bid-parse 真跑）",
        "ground_truth_status": "AI 初标草案（待人工验收，不外推为验收指标）",
        "files": [],
    });
    let files_report = report["files"].as_array_mut().unwrap();

    for s in &samples {
        println!("== {}（{}）", s.alias, s.kind_desc);
        let started = Instant::now();
        match parse_routed(&s.path, s.alias) {
            Ok(pf) => {
                let elapsed = started.elapsed().as_secs_f64();
                let n_blocks = pf.blocks.len();
                let ocr = pf.ocr_blocks;
                let has_text = pf.has_text_layer;
                let pages = pf.pages;
                let summary = pf.parse_summary.clone();
                println!("   OK {pages} 页 {n_blocks} 块（OCR {ocr}）{elapsed:.1}s —— {summary}");
                let json_path = out_dir
                    .join("parse")
                    .join(format!("{}.json", s.alias.replace('/', "_")));
                std::fs::write(&json_path, serde_json::to_vec_pretty(&pf).unwrap()).unwrap();
                files_report.push(serde_json::json!({
                    "alias": s.alias,
                    "desc": s.kind_desc,
                    "status": "ok",
                    "pages": pages,
                    "blocks": n_blocks,
                    "ocr_blocks": ocr,
                    "has_text_layer": has_text,
                    "parse_summary": summary,
                    "elapsed_secs": elapsed,
                    "blocks_json": json_path.to_string_lossy(),
                }));
                parsed.push(pf);
            }
            Err(e) => {
                let elapsed = started.elapsed().as_secs_f64();
                println!("   预期/实际拒绝：{e}");
                files_report.push(serde_json::json!({
                    "alias": s.alias,
                    "desc": s.kind_desc,
                    "status": "rejected",
                    "reason": e,
                    "elapsed_secs": elapsed,
                }));
            }
        }
    }

    // 条款抽取（真实面——含 PDF 主文件通道）
    let started = Instant::now();
    let reqs: Vec<Requirement> = extract_requirements(&parsed);
    let elapsed = started.elapsed().as_secs_f64();
    println!("== 条款抽取：{} 条 REQ（{elapsed:.1}s）", reqs.len());
    std::fs::write(
        out_dir.join("extract").join("requirements.json"),
        serde_json::to_vec_pretty(&reqs).unwrap(),
    )
    .unwrap();

    let by_file: std::collections::BTreeMap<String, usize> =
        reqs.iter().fold(std::collections::BTreeMap::new(), |mut m, r| {
            *m.entry(r.source_file.clone()).or_insert(0) += 1;
            m
        });
    let by_cat: std::collections::BTreeMap<String, usize> =
        reqs.iter().fold(std::collections::BTreeMap::new(), |mut m, r| {
            *m.entry(r.category.clone()).or_insert(0) += 1;
            m
        });
    report["extract"] = serde_json::json!({
        "total": reqs.len(),
        "by_file": by_file,
        "by_category": by_cat,
        "elapsed_secs": elapsed,
    });

    let report_path = out_dir.join("real-eval-report.json");
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("== 报告：{}", report_path.display());
    let _ = Path::new(&report_path);
}
