//! bid_health_check —— E 组竞品体检 harness（issue #11 · E-RUN）。
//!
//! 纪律（与任务指令一致）：
//! - **不改 crates/ 任何源码**：本 harness 只调用 main 基线（origin/main）上的
//!   `bid_engine::parse_docx_via_engine / parse_pdf_via_engine`（pdf 失败回退
//!   `bid_parse::parse_file`——与 bid-tools service.rs 的 import 路由同款）+
//!   `bid_parse::extract::extract_requirements`；
//! - 语料：`artifacts/bid-health-check/corpus/`（power_grid MIT 语料，provenance.json
//!   为唯一入库溯源账本，corpus/ 二进制不入 git）；
//! - 六维统计（近似口径，方法学局限见 REPORT.md）：
//!   ①否决项召回（LingYan 20 词表：解析块流命中段 vs 抽出条款命中，近似真值面）
//!   ②条款抽取面（条数/千字 + 类别分布）③类别命中（否决词命中条款的归类合理性）
//!   ④引用支持（locator 回查抽检 20 条/文件）⑤崩溃率（catch_unwind + Err 统计）
//!   ⑥吞吐（逐文件墙钟 → 100 页口径外推）；
//! - 输出：`results/<file>.json`（逐文件）+ `BASELINE.json`（汇总，serde_json
//!   默认 BTreeMap 键序，重跑可 diff 对比；schema 现为 `bid-health-check/2`——
//!   #462 强短语面从 8 词子串镜像升级为生产单一真源 46 词形正则，v1 基线
//!   与 v2 报告不可比）；
//! - 缺件按 issue #9 如实登记（ENV_MISSING / SKIP 语义，不伪装失败）。

use bid_domain::Requirement;
use bid_parse::extract::extract_requirements;
use bid_parse::veto_lexicon;
use bid_parse::{norm_n1, Block, ParsedFile};
use serde_json::{json, Value};
use bid_engine::EngineReader;
use regex::Regex;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

/// #462（B-16 残留面）：检测语义升级（8 子串镜像 → 生产 46 词形正则），
/// 旧 v1 基线与本版报告不可比——分记不折抵。
const SCHEMA: &str = "bid-health-check/2";
/// 抽检上限（引用支持维度：每文件确定性等距抽 20 条）
const LOCATOR_SAMPLE_N: usize = 20;
/// 漏召样本/嫌疑样本落盘上限（防 JSON 膨胀）
const SAMPLE_CAP: usize = 15;
/// 摘录截断长度（字符）
const EXCERPT_CAP: usize = 120;

/// 强否决短语面（#462/B-16 残留面：单一真源派生，非镜像）。
///
/// 旧实现自带 8 词子串镜像（「两处若漂移以 extract.rs 为准」），#440 后生产
/// 词面已达 46 词形（含 6 个正则形态），子串镜像无法表达且已漏 38 词族。
/// 现由 [`veto_lexicon::strong_patterns`] 同一 pattern 字符串编译——path
/// 依赖使漂移在构建期即不可能：改 bid-parse 词表，本 harness 重编译自动跟进。
fn re_strong_veto() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(veto_lexicon::strong_patterns())
            .expect("strong_patterns 为 veto_lexicon 单元测试锁定的合法正则")
    })
}

/// pdfium 在位性探测用最小 PDF（pdfium 对残缺 xref 有修复能力；探测口径 =
/// reader.rs 同款 MdcoreReader::extract_pdf_text，Err 含「打开失败」= 库缺件）。
const MINIMAL_PDF: &[u8] = b"%PDF-1.4\n1 0 obj\n<</Type/Catalog/Pages 2 0 R>>\nendobj\n2 0 obj\n<</Type/Pages/Kids[3 0 R]/Count 1>>\nendobj\n3 0 obj\n<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]>>\nendobj\ntrailer\n<</Root 1 0 R/Size 4>>\n%%EOF\n";

fn main() {
    let mut corpus = None;
    let mut out = None;
    let mut baseline = None;
    let mut lexicon_path = None;
    let mut git_branch = String::new();
    let mut git_head = String::new();
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--corpus" => {
                    corpus = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--out" => {
                    out = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--baseline" => {
                    baseline = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--lexicon" => {
                    lexicon_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--git-branch" => {
                    git_branch = args[i + 1].clone();
                    i += 2;
                }
                "--git-head" => {
                    git_head = args[i + 1].clone();
                    i += 2;
                }
                other => {
                    eprintln!("未知参数 {other}");
                    eprintln!("用法: bid-health-check [--corpus <dir>] [--out <dir>] [--baseline <file>] [--lexicon <file>] [--git-branch <b>] [--git-head <sha>]");
                    std::process::exit(2);
                }
            }
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.parent().and_then(|p| p.parent()).expect("仓库根定位");
    let corpus = corpus.unwrap_or_else(|| repo.join("artifacts/bid-health-check/corpus"));
    let out = out.unwrap_or_else(|| repo.join("artifacts/bid-health-check/results"));
    let baseline_path =
        baseline.unwrap_or_else(|| repo.join("artifacts/bid-health-check/BASELINE.json"));
    let lexicon_path =
        lexicon_path.unwrap_or_else(|| repo.join("artifacts/bid-health-check/veto_lexicon.json"));

    // ---- 词表载入（LingYan 20 词废标反查表）
    let (lexicon, lexicon_prov) = load_lexicon(&lexicon_path);

    // ---- 环境预检（issue #9 缺件登记口径）
    let env_report = env_preflight();

    // ---- 语料收集：corpus/*.pdf + corpus/doc/*.docx（排序 → 确定性）
    let mut files: Vec<PathBuf> = Vec::new();
    if corpus.is_dir() {
        let mut tops: Vec<PathBuf> = list_files(&corpus, "pdf");
        files.append(&mut tops);
        let docdir = corpus.join("doc");
        if docdir.is_dir() {
            let mut docs: Vec<PathBuf> = list_files(&docdir, "docx");
            files.append(&mut docs);
        }
    }
    if files.is_empty() {
        eprintln!("FATAL: corpus 为空（{}）——先按 provenance.json 布置语料", corpus.display());
        std::process::exit(3);
    }
    eprintln!("[E-RUN] corpus={} files={} lexicon_words={}", corpus.display(), files.len(), lexicon.len());

    std::fs::create_dir_all(&out).expect("results 目录创建");

    // ---- 逐文件体检（panic 捕获 + 增量落盘——硬崩溃也保留已完成的文件面）
    let mut per_file: Vec<Value> = Vec::new();
    let mut panic_count = 0usize;
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        eprintln!("---- 处理 {name}");
        let started = Instant::now();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process_file(path, &name, &lexicon)
        }));
        let mut record = match outcome {
            Ok(v) => v,
            Err(p) => {
                panic_count += 1;
                let msg = panic_message(&p);
                eprintln!("!! PANIC {name}: {msg}");
                json!({
                    "schema": SCHEMA,
                    "file": name,
                    "sha256": sha256_or_none(path),
                    "status": "panic",
                    "panic_message": msg,
                    "minimal_repro": format!("cargo run --manifest-path scripts/bid_health_check/Cargo.toml --release -- --corpus <dir> 后单文件复现：cp '<abs>' /tmp/repro/ && cargo run … --corpus /tmp/repro"),
                    "elapsed_total_ms": started.elapsed().as_millis() as u64,
                })
            }
        };
        if let Some(obj) = record.as_object_mut() {
            obj.insert("wall_ms_incl_unwind".into(), json!(started.elapsed().as_millis() as u64));
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let rf = out.join(format!("{stem}.json"));
        let pretty = serde_json::to_string_pretty(&record).unwrap();
        if let Err(e) = std::fs::write(&rf, pretty) {
            eprintln!("!! 落盘失败 {}: {e}", rf.display());
        }
        per_file.push(record);
    }

    // ---- 汇总 BASELINE
    let baseline_value = aggregate(&per_file, &env_report, &lexicon, &lexicon_prov, &git_branch, &git_head);
    let bp = serde_json::to_string_pretty(&baseline_value).unwrap();
    std::fs::write(&baseline_path, bp).expect("BASELINE.json 落盘");

    // ---- 控制台摘要
    let ok = baseline_value["status_counts"]["ok"].as_u64().unwrap_or(0);
    let err = baseline_value["status_counts"]["err"].as_u64().unwrap_or(0);
    eprintln!("\n==== E-RUN 汇总 ====");
    eprintln!("files={} ok={} err={} panic={}", per_file.len(), ok, err, panic_count);
    eprintln!("veto recall (unique face): {}", baseline_value["veto_aggregate"]["unique_face"]["recall"]);
    eprintln!("BASELINE → {}", baseline_path.display());
    if panic_count > 0 {
        eprintln!("!! 存在 panic——见 results/*.json 的 panic_message 与 minimal_repro");
    }
}

// ---------------------------------------------------------------------------
// 语料/词表/环境
// ---------------------------------------------------------------------------

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

fn load_lexicon(path: &Path) -> (Vec<String>, Value) {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("词表载入失败 {}: {e}（--lexicon 指定 veto_lexicon.json）", path.display()));
    let v: Value = serde_json::from_str(&raw).expect("veto_lexicon.json 解析失败");
    let words: Vec<String> = v["words"]
        .as_array()
        .expect("words 数组缺失")
        .iter()
        .filter_map(|w| w.as_str().map(|s| s.to_string()))
        .collect();
    let prov = v["provenance"].clone();
    (words, prov)
}

fn sha256_or_none(path: &Path) -> Value {
    match bid_parse::sha256_file(path) {
        Ok(s) => json!(s),
        Err(_) => json!(null),
    }
}

fn tool_version(cmd: &str, flag: &str) -> String {
    match std::process::Command::new(cmd).arg(flag).output() {
        Ok(o) => {
            let txt = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            let line = txt.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            format!("ok: {line}")
        }
        Err(e) => format!("ENV_MISSING: {cmd} 无法启动（{e}）"),
    }
}

fn tessdata_chi_sim_status() -> String {
    // pdf.rs 的生效口径：TESSDATA_PREFIX env → $HOME/.tessdata
    let base = std::env::var("TESSDATA_PREFIX")
        .unwrap_or_else(|_| format!("{}/.tessdata", std::env::var("HOME").unwrap_or_default()));
    let cand = PathBuf::from(&base).join("chi_sim.traineddata");
    if cand.is_file() {
        format!("ok: {}（pdf.rs 生效路径）", cand.display())
    } else {
        format!("ENV_MISSING: {} 不存在——OCR 中文面将逐页诚实降级（issue #9）", cand.display())
    }
}

fn pdfium_status() -> String {
    let lib_note = std::env::var("MDC_PDFIUM_LIB")
        .map(|p| format!("MDC_PDFIUM_LIB={p}"))
        .unwrap_or_else(|_| "MDC_PDFIUM_LIB 未设（走编译期候选）".to_string());
    let reader = bid_engine::MdcoreReader::new();
    match reader.extract_pdf_text(MINIMAL_PDF) {
        Ok(_) => format!("ok: 引擎探测通过（{lib_note}）"),
        Err(e) => {
            let s = e.to_string();
            if s.contains("打开失败") {
                format!("ENV_MISSING: pdfium 动态库不可用（{lib_note}）——pdf 引擎通道整体回退旧路径")
            } else {
                // 库已加载、探测文档本身残缺——不影响真实文件判定（逐文件 route 见真章）
                format!("ok(lib loaded): 探测文档被引擎拒绝（{s}）——真实文件以逐文件 route 为准（{lib_note}）")
            }
        }
    }
}

fn env_preflight() -> Value {
    json!({
        "pdftotext": tool_version("pdftotext", "-v"),
        "pdftoppm": tool_version("pdftoppm", "-v"),
        "pdfinfo": tool_version("pdfinfo", "-v"),
        "tesseract": tool_version("tesseract", "--version"),
        "tessdata_chi_sim": tessdata_chi_sim_status(),
        "pdfium": pdfium_status(),
        "mdcore_rev": bid_engine::MDCORE_REV,
        "note_libreoffice": ".doc→docx 转换在语料制备阶段离线完成（见 provenance.json conversion 字段），harness 不调用 LO",
    })
}

// ---------------------------------------------------------------------------
// 单文件体检
// ---------------------------------------------------------------------------

fn process_file(path: &Path, name: &str, lexicon: &[String]) -> Value {
    let t0 = Instant::now();
    let sha = bid_parse::sha256_file(path).ok();
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    // ---- 引擎路径解析（镜像 bid-tools service.rs import 路由：
    //      docx→引擎；pdf→引擎，Err 回退旧 parse_file）
    let mut route = String::new();
    let mut engine_error: Option<String> = None;
    let parsed: Result<ParsedFile, String> = match ext.as_str() {
        "pdf" => {
            let t = Instant::now();
            let r = bid_engine::parse_pdf_via_engine(path);
            let engine_ms = t.elapsed().as_millis() as u64;
            match r {
                Ok(pf) => {
                    route = format!("engine_pdf({engine_ms}ms)");
                    Ok(pf)
                }
                Err(e) => {
                    let es = e.to_string();
                    engine_error = Some(es.clone());
                    let t2 = Instant::now();
                    let legacy = bid_parse::parse_file(path, name);
                    let legacy_ms = t2.elapsed().as_millis() as u64;
                    route = format!("engine_pdf_err→legacy_pdftotext({engine_ms}ms+{legacy_ms}ms)");
                    legacy
                }
            }
        }
        "docx" => {
            let r = bid_engine::parse_docx_via_engine(path);
            route = "engine_docx".to_string();
            r.map_err(|e| e.to_string())
        }
        other => Err(format!("SKIP: 不支持的扩展名 .{other}（{name}）——链路仅 docx/xlsx/pdf 面")),
    };
    let elapsed_parse_ms = t0.elapsed().as_millis() as u64;

    let mut pf = match parsed {
        Ok(pf) => pf,
        Err(e) => {
            return json!({
                "schema": SCHEMA, "file": name, "sha256": sha, "bytes": bytes,
                "route": route, "engine_error": engine_error,
                "status": if e.starts_with("SKIP:") { "skip" } else { "err" },
                "error": e,
                "elapsed_parse_ms": elapsed_parse_ms, "elapsed_extract_ms": 0,
                "elapsed_total_ms": elapsed_parse_ms,
            });
        }
    };
    // 统一名称面（引擎路径 name=file_name；旧路径已传 name——均应为 corpus 业务名）
    if pf.name.is_empty() {
        pf.name = name.to_string();
    }

    // ---- 条款抽取（每文件独立调用——extract_requirements 无跨文件状态，等价于
    //      m1 累积调用面；source_file 字段做归属锚）
    let t1 = Instant::now();
    let reqs = extract_requirements(&[pf.clone()]);
    let elapsed_extract_ms = t1.elapsed().as_millis() as u64;
    let elapsed_total_ms = t0.elapsed().as_millis() as u64;

    // ---- 双路由对照探针（诊断面，非生产路由）：仅对「引擎路由成功」的 pdf 追跑
    //      旧 pdftotext 路径，量化两路由在真实语料上的块面/条款面差分。
    //      引擎失败已回退旧路径的文件不重跑（OCR 件双跑成本高且生产面即旧路径）。
    let legacy_probe: Value = if ext == "pdf" && !route.starts_with("engine_pdf_err") {
        let t = Instant::now();
        let legacy = bid_parse::parse_file(path, name);
        let ms = t.elapsed().as_millis() as u64;
        match legacy {
            Ok(lpf) => {
                let lreqs = extract_requirements(&[lpf.clone()]);
                let lchars: usize = lpf.blocks.iter().map(|b| b.text.chars().count()).sum();
                json!({
                    "route": "legacy_pdftotext",
                    "elapsed_ms": ms,
                    "blocks": lpf.blocks.len(),
                    "chars": lchars,
                    "reqs": lreqs.len(),
                    "veto_truth_blocks": count_veto_blocks(&lpf, lexicon),
                    "note": "诊断探针：与生产路由（engine_pdf）同源文件对照；不参与吞吐主指标",
                })
            }
            Err(e) => json!({ "route": "legacy_pdftotext", "error": e, "elapsed_ms": ms }),
        }
    } else {
        Value::Null
    };

    // ---- 基础面
    let chars: usize = pf.blocks.iter().map(|b| b.text.chars().count()).sum();
    let ms_per_page = if pf.pages > 0 {
        elapsed_total_ms as f64 / pf.pages as f64
    } else {
        0.0
    };

    // ---- ② 条款抽取面
    let mut by_category: BTreeMap<&str, usize> = BTreeMap::new();
    let mut with_clause_no = 0usize;
    for r in &reqs {
        *by_category.entry(r.category.as_str()).or_insert(0) += 1;
        if r.clause_no.is_some() {
            with_clause_no += 1;
        }
    }
    let reqs_per_1000_chars = if chars > 0 {
        (reqs.len() as f64) * 1000.0 / chars as f64
    } else {
        0.0
    };

    // ---- ① 否决项召回（近似口径）+ ③ 类别命中
    let veto = veto_metrics(&pf, &reqs, lexicon);
    let cat_hit = category_hit_metrics(&reqs, lexicon);

    // ---- ④ 引用支持（locator 回查，等距抽 20 条）
    let locator = locator_check(&pf, &reqs);

    json!({
        "schema": SCHEMA,
        "file": name,
        "sha256": sha,
        "bytes": bytes,
        "route": route,
        "engine_error": engine_error,
        "status": "ok",
        "pages": pf.pages,
        "blocks": pf.blocks.len(),
        "chars": chars,
        "tables": pf.tables,
        "has_text_layer": pf.has_text_layer,
        "ocr_blocks": pf.ocr_blocks,
        "parse_summary": pf.parse_summary,
        "components_ledger": pf.components,
        "elapsed_parse_ms": elapsed_parse_ms,
        "elapsed_extract_ms": elapsed_extract_ms,
        "elapsed_total_ms": elapsed_total_ms,
        "ms_per_page": round2(ms_per_page),
        "est_100page_ms": round2(ms_per_page * 100.0),
        "requirements": {
            "total": reqs.len(),
            "per_1000_chars": round4(reqs_per_1000_chars),
            "with_clause_no": with_clause_no,
            "by_category": bmap_to_json(&by_category),
        },
        "veto": veto,
        "category_hit": cat_hit,
        "locator_check": locator,
        "legacy_probe": legacy_probe,
    })
}

/// 旧路径探针用：词表命中块计数（与 veto_metrics 真值面同口径，轻量版）
fn count_veto_blocks(pf: &ParsedFile, lexicon: &[String]) -> usize {
    pf.blocks
        .iter()
        .filter(|b| lexicon.iter().any(|w| b.text.contains(w.as_str())))
        .count()
}

// ---------------------------------------------------------------------------
// 维度 ①③：否决词命中（真值面=解析块流；抽取面=条款锚文本）
// ---------------------------------------------------------------------------

fn veto_metrics(pf: &ParsedFile, reqs: &[Requirement], lexicon: &[String]) -> Value {
    // 真值面：块流命中（block_idx → 命中词集）
    let mut truth: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut per_word_truth: BTreeMap<String, usize> = BTreeMap::new();
    for b in &pf.blocks {
        for w in lexicon {
            if b.text.contains(w.as_str()) {
                truth.entry(b.idx).or_default().push(w.clone());
                *per_word_truth.entry(w.clone()).or_insert(0) += 1;
            }
        }
    }
    // 抽取面锚文本（归一化一次性预计算）
    let anchor_norms: Vec<String> = reqs
        .iter()
        .flat_map(|r| r.anchors.iter())
        .map(|a| norm_n1(&a.excerpt))
        .filter(|s| !s.is_empty())
        .collect();
    // 抽取面逐词命中（条款计数）
    let mut per_word_extracted: BTreeMap<String, usize> = BTreeMap::new();
    for r in reqs {
        let joined: String = r.anchors.iter().map(|a| a.excerpt.as_str()).collect::<Vec<_>>().join("");
        for w in lexicon {
            if joined.contains(w.as_str()) {
                *per_word_extracted.entry(w.clone()).or_insert(0) += 1;
            }
        }
    }
    // 覆盖判定：真值块文本（归一化）⊆ 某锚归一化文本（子串）→ 该块命中进入抽取面。
    // 近似口径：子串匹配对极短块可能高估覆盖（方法学局限，REPORT 声明）。
    let mut covered = 0usize;
    let mut per_word_covered: BTreeMap<String, usize> = BTreeMap::new();
    let mut missed_samples: Vec<Value> = Vec::new();
    for (idx, words) in &truth {
        let b = pf.blocks.get(*idx);
        let bnorm = b.map(|b| norm_n1(&b.text)).unwrap_or_default();
        let hit = !bnorm.is_empty() && anchor_norms.iter().any(|a| a.contains(bnorm.as_str()));
        if hit {
            covered += 1;
            for w in words {
                *per_word_covered.entry(w.clone()).or_insert(0) += 1;
            }
        } else if missed_samples.len() < SAMPLE_CAP {
            if let Some(b) = b {
                missed_samples.push(json!({
                    "block_idx": b.idx,
                    "kind": format!("{:?}", b.kind),
                    "locator": b.locator,
                    "text": trunc(&b.text, EXCERPT_CAP),
                    "words": words,
                }));
            }
        }
    }
    let truth_blocks = truth.len();
    let recall = if truth_blocks > 0 {
        json!(round4(covered as f64 / truth_blocks as f64))
    } else {
        json!(null) // 词表零命中（真值面空——OCR 降级件常见）：召回未定义，不伪装 1.0
    };
    json!({
        "truth_blocks": truth_blocks,
        "covered_blocks": covered,
        "recall": recall,
        "per_word_truth": per_word_truth,
        "per_word_covered_blocks": per_word_covered,
        "per_word_extracted_reqs": per_word_extracted,
        "missed_samples": missed_samples,
    })
}

fn category_hit_metrics(reqs: &[Requirement], lexicon: &[String]) -> Value {
    // 词表命中条款的 category 归类面（规则可核对）
    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut hit_reqs = 0usize;
    let mut hit_dump: Vec<Value> = Vec::new();
    // 强否决短语面（extract.rs 判定动词面镜像）：含强短语却非「否决项」= 嫌疑漏归
    let mut strong_total = 0usize;
    let mut strong_not_veto = 0usize;
    let mut suspects: Vec<Value> = Vec::new();
    for r in reqs {
        let joined: String = r.anchors.iter().map(|a| a.excerpt.as_str()).collect::<Vec<_>>().join("");
        let lex_hit: Vec<&String> = lexicon.iter().filter(|w| joined.contains(w.as_str())).collect();
        if !lex_hit.is_empty() {
            hit_reqs += 1;
            *by_category.entry(r.category.clone()).or_insert(0) += 1;
            if hit_dump.len() < SAMPLE_CAP {
                hit_dump.push(json!({
                    "id": r.id, "category": r.category, "clause_no": r.clause_no,
                    "words": lex_hit.iter().map(|w| w.as_str()).collect::<Vec<_>>(),
                    "excerpt": trunc(&joined, EXCERPT_CAP),
                }));
            }
        }
        if re_strong_veto().is_match(&joined) {
            strong_total += 1;
            if r.category != "否决项" {
                strong_not_veto += 1;
                if suspects.len() < SAMPLE_CAP {
                    suspects.push(json!({
                        "id": r.id, "category": r.category, "clause_no": r.clause_no,
                        "excerpt": trunc(&joined, EXCERPT_CAP),
                    }));
                }
            }
        }
    }
    json!({
        "lexicon_hit_reqs": hit_reqs,
        "by_category": by_category,
        "hit_samples": hit_dump,
        "strong_phrase_reqs": strong_total,
        "strong_phrase_not_veto": strong_not_veto,
        "strong_phrase_suspects": suspects,
    })
}

// ---------------------------------------------------------------------------
// 维度 ④：引用支持（locator 回查）
// ---------------------------------------------------------------------------

fn locator_check(pf: &ParsedFile, reqs: &[Requirement]) -> Value {
    let n = reqs.len();
    let mut checked = 0usize;
    let mut pass = 0usize;
    let mut fail_no_block = 0usize;
    let mut fail_text_mismatch = 0usize;
    let mut samples: Vec<Value> = Vec::new();
    if n == 0 {
        return json!({ "checked": 0, "pass": 0, "fail_no_block": 0, "fail_text_mismatch": 0, "samples": [] });
    }
    let step = (n / LOCATOR_SAMPLE_N).max(1);
    let mut i = 0usize;
    while i < n {
        let r = &reqs[i];
        // 取第一锚（多锚条款的回查面等价——锚构造同源）
        let anchor = r.anchors.first();
        let verdict = anchor.map(|a| {
            let candidates: Vec<&Block> =
                pf.blocks.iter().filter(|b| b.locator == a.locator).collect();
            if candidates.is_empty() {
                (false, "no_block", Value::Null)
            } else {
                let an = norm_n1(&a.excerpt);
                let mut ok = false;
                let mut matched: Option<&Block> = None;
                for c in &candidates {
                    let cn = norm_n1(&c.text);
                    if !cn.is_empty() && (an.contains(cn.as_str()) || cn.contains(an.as_str())) {
                        ok = true;
                        matched = Some(*c);
                        break;
                    }
                }
                if ok {
                    (true, "pass", json!(matched.map(|m| trunc(&m.text, 80)).unwrap_or_default()))
                } else {
                    (
                        false,
                        "text_mismatch",
                        json!(candidates.first().map(|m| trunc(&m.text, 80)).unwrap_or_default()),
                    )
                }
            }
        });
        checked += 1;
        let (ok, kind, block_text) = verdict
            .map(|(a, b, c)| (a, b.to_string(), c))
            .unwrap_or((false, "no_anchor".to_string(), Value::Null));
        if ok {
            pass += 1;
        } else if kind == "no_block" || kind == "no_anchor" {
            fail_no_block += 1;
        } else {
            fail_text_mismatch += 1;
        }
        if samples.len() < LOCATOR_SAMPLE_N {
            samples.push(json!({
                "req_id": r.id,
                "category": r.category,
                "locator": anchor.map(|a| a.locator.clone()).unwrap_or_default(),
                "anchor_excerpt": anchor.map(|a| trunc(&a.excerpt, 80)).unwrap_or_default(),
                "verdict": kind,
                "block_text": block_text,
            }));
        }
        i += step;
    }
    json!({
        "checked": checked,
        "pass": pass,
        "fail_no_block": fail_no_block,
        "fail_text_mismatch": fail_text_mismatch,
        "samples": samples,
    })
}

// ---------------------------------------------------------------------------
// 汇总
// ---------------------------------------------------------------------------

fn aggregate(
    per_file: &[Value],
    env_report: &Value,
    lexicon: &[String],
    lexicon_prov: &Value,
    git_branch: &str,
    git_head: &str,
) -> Value {
    let mut status_counts = BTreeMap::new();
    let mut route_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut seen_sha: HashSet<String> = HashSet::new();
    // 聚合面（unique = 按 sha256 去重，首见优先——A-19 与 A-01 重复入 corpus 属实登计）
    let mut u_truth = 0usize;
    let mut u_covered = 0usize;
    let mut u_reqs = 0usize;
    let mut u_chars = 0usize;
    let mut u_cat: BTreeMap<String, usize> = BTreeMap::new();
    let mut u_lex_hit = 0usize;
    let mut u_strong = 0usize;
    let mut u_strong_not_veto = 0usize;
    let mut per_word_truth: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_word_covered: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_word_extracted: BTreeMap<String, usize> = BTreeMap::new();
    let mut loc_checked = 0usize;
    let mut loc_pass = 0usize;
    let mut loc_fail_no_block = 0usize;
    let mut loc_fail_text = 0usize;
    let mut total_ms = 0u64;
    let mut total_pages = 0usize;
    let mut pdf_ms = 0u64;
    let mut pdf_pages = 0usize;
    let mut pdf_rows: Vec<Value> = Vec::new();
    let mut docx_ms = 0u64;
    let mut docx_chars = 0usize;
    let mut docx_rows: Vec<Value> = Vec::new();
    let mut throughput_rows: Vec<Value> = Vec::new();
    let mut route_delta_rows: Vec<Value> = Vec::new();
    let mut files_summary: Vec<Value> = Vec::new();
    let mut err_list: Vec<Value> = Vec::new();
    let mut panic_list: Vec<Value> = Vec::new();

    for f in per_file {
        let status = f["status"].as_str().unwrap_or("unknown").to_string();
        *status_counts.entry(status.clone()).or_insert(0) += 1;
        let route = f["route"].as_str().unwrap_or("").to_string();
        if !route.is_empty() {
            *route_counts.entry(route.split('(').next().unwrap_or(&route).to_string()).or_insert(0) += 1;
        }
        let sha = f["sha256"].as_str().unwrap_or("").to_string();
        let unique = !sha.is_empty() && seen_sha.insert(sha);
        if status == "ok" {
            let pages = f["pages"].as_u64().unwrap_or(0) as usize;
            let ms = f["elapsed_total_ms"].as_u64().unwrap_or(0);
            total_ms += ms;
            total_pages += pages;
            throughput_rows.push(json!({
                "file": f["file"], "route": f["route"], "pages": pages,
                "ms": ms, "ms_per_page": f["ms_per_page"],
                "est_100page_ms": f["est_100page_ms"],
            }));
            if route.contains("pdf") {
                // pdf 面（含 OCR 回退件）：页口径吞吐
                pdf_ms += ms;
                pdf_pages += pages;
                pdf_rows.push(json!({
                    "file": f["file"], "route": f["route"], "pages": pages, "ms": ms,
                    "ms_per_page": f["ms_per_page"],
                }));
            } else {
                // docx 面（页概念不适用——每文件/每千字口径）
                docx_ms += ms;
                docx_chars += f["chars"].as_u64().unwrap_or(0) as usize;
                docx_rows.push(json!({
                    "file": f["file"], "ms": ms, "chars": f["chars"],
                    "ms_per_1000_chars": if f["chars"].as_u64().unwrap_or(0) > 0 {
                        round2(ms as f64 * 1000.0 / f["chars"].as_u64().unwrap_or(0) as f64)
                    } else { 0.0 },
                }));
            }
            if unique {
                u_truth += f["veto"]["truth_blocks"].as_u64().unwrap_or(0) as usize;
                u_covered += f["veto"]["covered_blocks"].as_u64().unwrap_or(0) as usize;
                u_reqs += f["requirements"]["total"].as_u64().unwrap_or(0) as usize;
                u_chars += f["chars"].as_u64().unwrap_or(0) as usize;
                if let Some(bc) = f["requirements"]["by_category"].as_object() {
                    for (k, v) in bc {
                        *u_cat.entry(k.clone()).or_insert(0) += v.as_u64().unwrap_or(0) as usize;
                    }
                }
                u_lex_hit += f["category_hit"]["lexicon_hit_reqs"].as_u64().unwrap_or(0) as usize;
                u_strong += f["category_hit"]["strong_phrase_reqs"].as_u64().unwrap_or(0) as usize;
                u_strong_not_veto +=
                    f["category_hit"]["strong_phrase_not_veto"].as_u64().unwrap_or(0) as usize;
                if let Some(pw) = f["veto"]["per_word_truth"].as_object() {
                    for (k, v) in pw {
                        *per_word_truth.entry(k.clone()).or_insert(0) += v.as_u64().unwrap_or(0) as usize;
                    }
                }
                if let Some(pw) = f["veto"]["per_word_covered_blocks"].as_object() {
                    for (k, v) in pw {
                        *per_word_covered.entry(k.clone()).or_insert(0) += v.as_u64().unwrap_or(0) as usize;
                    }
                }
                if let Some(pw) = f["veto"]["per_word_extracted_reqs"].as_object() {
                    for (k, v) in pw {
                        *per_word_extracted.entry(k.clone()).or_insert(0) += v.as_u64().unwrap_or(0) as usize;
                    }
                }
            }
            loc_checked += f["locator_check"]["checked"].as_u64().unwrap_or(0) as usize;
            loc_pass += f["locator_check"]["pass"].as_u64().unwrap_or(0) as usize;
            loc_fail_no_block += f["locator_check"]["fail_no_block"].as_u64().unwrap_or(0) as usize;
            loc_fail_text += f["locator_check"]["fail_text_mismatch"].as_u64().unwrap_or(0) as usize;
            if let Some(lp) = f["legacy_probe"].as_object() {
                if lp.get("blocks").is_some() {
                    route_delta_rows.push(json!({
                        "file": f["file"],
                        "engine_blocks": f["blocks"],
                        "legacy_blocks": lp["blocks"],
                        "block_delta": (f["blocks"].as_i64().unwrap_or(0)
                            - lp["blocks"].as_i64().unwrap_or(0)),
                        "engine_chars": f["chars"],
                        "legacy_chars": lp["chars"],
                        "engine_reqs": f["requirements"]["total"],
                        "legacy_reqs": lp["reqs"],
                        "engine_veto_truth": f["veto"]["truth_blocks"],
                        "legacy_veto_truth": lp["veto_truth_blocks"],
                    }));
                }
            }
        } else if status == "err" || status == "skip" {
            err_list.push(json!({
                "file": f["file"], "status": status, "route": f["route"],
                "engine_error": f["engine_error"], "error": f["error"],
            }));
        } else if status == "panic" {
            panic_list.push(json!({
                "file": f["file"], "panic_message": f["panic_message"],
                "minimal_repro": f["minimal_repro"],
            }));
        }
        files_summary.push(json!({
            "file": f["file"], "status": status, "route": f["route"],
            "pages": f["pages"].as_u64().unwrap_or(0),
            "blocks": f["blocks"].as_u64().unwrap_or(0),
            "reqs": f["requirements"]["total"].as_u64().unwrap_or(0),
            "veto_recall": f["veto"]["recall"],
            "ms": f["elapsed_total_ms"].as_u64().unwrap_or(0),
            "result_ref": format!("results/{}.json", stem_of(f["file"].as_str().unwrap_or("")))
        }));
    }
    let n_files = per_file.len();
    let n_unique = seen_sha.len();
    let recall = if u_truth > 0 { u_covered as f64 / u_truth as f64 } else { 1.0 };
    let reqs_per_kchars = if u_chars > 0 { u_reqs as f64 * 1000.0 / u_chars as f64 } else { 0.0 };
    let ms_per_page = if total_pages > 0 { total_ms as f64 / total_pages as f64 } else { 0.0 };
    let pdf_ms_per_page = if pdf_pages > 0 { pdf_ms as f64 / pdf_pages as f64 } else { 0.0 };
    let docx_ms_per_kchars = if docx_chars > 0 { docx_ms as f64 * 1000.0 / docx_chars as f64 } else { 0.0 };
    let mut per_word_table: Vec<Value> = lexicon
        .iter()
        .map(|w| {
            let t = per_word_truth.get(w).copied().unwrap_or(0);
            let c = per_word_covered.get(w).copied().unwrap_or(0);
            let e = per_word_extracted.get(w).copied().unwrap_or(0);
            json!({
                "word": w,
                "truth_block_hits": t,
                "covered_blocks": c,
                "word_recall": if t > 0 { json!(round4(c as f64 / t as f64)) } else { Value::Null },
                "extracted_req_hits": e,
            })
        })
        .collect();
    per_word_table.sort_by(|a, b| {
        let ta = a["truth_block_hits"].as_u64().unwrap_or(0);
        let tb = b["truth_block_hits"].as_u64().unwrap_or(0);
        tb.cmp(&ta).then(a["word"].as_str().cmp(&b["word"].as_str()))
    });

    json!({
        "schema": SCHEMA,
        "task": "E-RUN 竞品体检（issue #11 E 组）——power_grid 真实语料跑 main 基线标书链路",
        "git": { "branch": git_branch, "head": git_head },
        "env": env_report,
        "lexicon": { "words": lexicon.len(), "provenance": lexicon_prov },
        "corpus": {
            "files": n_files,
            "unique_by_sha256": n_unique,
            "dedup_note": "A-19 与 A-01 为同一官方附件（power_grid test_samples/ 与 rag_seed/ 双路径重复入库）——聚合指标按 sha256 去重（首见），逐文件结果保留两份",
            "pages_total_ok_files": total_pages,
        },
        "status_counts": bmap_to_json(&status_counts),
        "route_counts": route_counts,
        "veto_aggregate": {
            "unique_face": {
                "truth_blocks": u_truth,
                "covered_blocks": u_covered,
                "recall": round4(recall),
            },
            "per_word": per_word_table,
        },
        "extraction_aggregate": {
            "total_reqs_unique": u_reqs,
            "total_chars_unique": u_chars,
            "reqs_per_1000_chars": round4(reqs_per_kchars),
            "by_category_unique": u_cat,
        },
        "category_hit_aggregate": {
            "lexicon_hit_reqs_unique": u_lex_hit,
            "strong_phrase_reqs_unique": u_strong,
            "strong_phrase_not_veto_unique": u_strong_not_veto,
        },
        "locator_aggregate": {
            "checked": loc_checked,
            "pass": loc_pass,
            "fail_no_block": loc_fail_no_block,
            "fail_text_mismatch": loc_fail_text,
        },
        "throughput": {
            "note": "pdf 面用页口径（含 OCR 回退件）；docx 面无页概念，用每文件/每千字口径；overall 仅作参考（docx 时间混入分子）",
            "pdf_face": {
                "files": pdf_rows.len(), "pages": pdf_pages, "total_ms": pdf_ms,
                "ms_per_page": round2(pdf_ms_per_page),
                "est_100page_ms": round2(pdf_ms_per_page * 100.0),
                "per_file": pdf_rows,
            },
            "docx_face": {
                "files": docx_rows.len(), "total_ms": docx_ms, "chars": docx_chars,
                "ms_per_1000_chars": round2(docx_ms_per_kchars),
                "per_file": docx_rows,
            },
            "overall_ms_per_page_mixed": round2(ms_per_page),
            "total_ms_ok_files": total_ms,
            "pages_mixed": total_pages,
            "per_file": throughput_rows,
        },
        "route_delta_engine_vs_legacy": route_delta_rows,
        "errors": err_list,
        "panics": panic_list,
        "files": files_summary,
    })
}

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "<非字符串 panic 载荷>".to_string()
    }
}

fn trunc(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        s.to_string()
    } else {
        let t: String = s.chars().take(cap).collect();
        format!("{t}…")
    }
}

fn round2(v: f64) -> f64 { (v * 100.0).round() / 100.0 }
fn round4(v: f64) -> f64 { (v * 10000.0).round() / 10000.0 }

fn bmap_to_json<V: serde::Serialize, K: Ord + serde::Serialize>(m: &BTreeMap<K, V>) -> Value {
    serde_json::to_value(m).unwrap_or(Value::Null)
}

fn stem_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) => name[..i].to_string(),
        None => name.to_string(),
    }
}

// ---------------------------------------------------------------------------
// #462（B-16 残留面）测试：强短语面与生产单一真源同源
// ---------------------------------------------------------------------------
#[cfg(test)]
mod strong_veto_tests {
    use super::re_strong_veto;

    /// 旧 8 词镜像漏报族抽样——v1 镜像下这些全部不命中（contains 不含），
    /// 现必须命中（46 词形真源）。
    #[test]
    fn b16_missed_word_families_now_hit() {
        let missed_by_old_mirror = [
            // （条款示例文本, 旧镜像漏掉的词族）
            ("未按规定格式编制的，其投标无效。", "投标无效"),
            ("逾期送达的投标文件，招标人不予受理。", "不予受理"),
            ("技术参数不允许偏离，偏离的作废标处理。", "不允许偏离"),
            ("投标人以他人名义投标的，取消中标资格。", "以他人名义投标"),
            ("串通投标的，其投标保证金将被没收，投标被拒绝。", "投标被拒绝"),
            ("未按要求盖章的文件，招标人有权拒收。", "有权拒收"),
        ];
        for (text, why) in missed_by_old_mirror {
            assert!(
                re_strong_veto().is_match(text),
                "旧镜像漏报族应命中（词族：{why}）：{text}"
            );
        }
    }

    /// 6 个正则形态词中抽样——子串 contains 结构上无法表达，升级为正则
    /// 匹配后首次真正生效。
    #[test]
    fn b16_regex_forms_take_effect() {
        // 「不接受[^，。；]{0,12}(投标|报价|备选|联合体)」形
        assert!(re_strong_veto().is_match("招标人不接受联合体报价的投标文件"));
        assert!(re_strong_veto().is_match("本项目不接受备选方案"));
        // 「利害关系[^，。；]{0,32}不得参加投标」形（间隔窗内命中）
        assert!(re_strong_veto().is_match("与招标人存在利害关系的单位不得参加投标"));
        // 「备选[^，。；]{0,8}不允许」形
        assert!(re_strong_veto().is_match("报价中备选方案出现时不允许"));
    }

    /// 超集验证——旧 8 词全部仍命中（无行为回退面）。
    #[test]
    fn b16_old_mirror_words_still_hit() {
        let old_eight = [
            "将被否决",
            "否决投标处理",
            "否决所有投标",
            "视为无效",
            "无效投标",
            "取消其中标",
            "没收其投标保证金",
            "废标",
        ];
        for w in old_eight {
            let text = format!("违反本条规定的，{w}。");
            assert!(
                re_strong_veto().is_match(&text),
                "旧镜像词不应回退失命中：{w}"
            );
        }
    }

    /// 反例面——正面资格/商务要求句不含处置动词，不得误命中。
    #[test]
    fn b16_positive_clauses_do_not_hit() {
        let positive = [
            "投标人应具备有效的营业执照。",
            "投标人须提供近三年的财务报表。",
            "本项目要求投标人具有 ISO9001 质量管理体系认证。",
            "服务期限为合同签订后 12 个月。",
        ];
        for text in positive {
            assert!(
                !re_strong_veto().is_match(text),
                "正面资格/商务句不应命中强短语面：{text}"
            );
        }
    }

    /// 单一真源锁定——本 harness 的 pattern 与生产 veto_lexicon 逐字符相等
    /// （防止未来有人把派生改回拷贝）。
    #[test]
    fn b16_pattern_is_single_sourced() {
        assert_eq!(
            re_strong_veto().as_str(),
            bid_parse::veto_lexicon::strong_patterns(),
            "harness 强短语面必须由 veto_lexicon::strong_patterns 派生（B-16：不得回拷）"
        );
    }

    /// SCHEMA 升版锁定——v2 起检测语义变更（8 子串 → 46 词形正则），
    /// 旧基线不可比；若再改检测语义必须再升版。
    #[test]
    fn b16_schema_version_bumped() {
        assert_eq!(super::SCHEMA, "bid-health-check/2");
    }
}
