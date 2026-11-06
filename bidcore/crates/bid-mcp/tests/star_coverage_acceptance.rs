//! #285 ① ★漏标检测真件验收。
//!
//! 验收面（issue #285 验收条款逐条）：
//! 1. star_total 与 manifest star_count 一致（±2 容差）——batch1 五件 +
//!    batch2 四件 PDF（doc/docx 件无 PDF 解析面，skip）
//! 2. danqing 黑盒 1/5 复现：Agent 标记覆盖商务表区间、仅提交 1 条
//!    substantive → 必须报出 missing=4（服务地点/验收要求/合同支付方式/
//!    履约验收方式四题全在 missing[]）
//! 3. 样板句「打"★"号条款为实质性条款…」不被计为真值（declarations=1，
//!    不出现在 missing/captured）
//! 4. 表格列★识别（p18 参数性质列独行「N ★」入真值；p19 商务表五题
//!    引擎路径 5/5 捕获 star=true）
//! 5. 无误报（captured + missing == star_truth，不漏不重）
//!
//! 环境门（issue #9 口径）：manifest 对账与 danqing 两路径断言依赖布局
//! 保真解析路（bid-engine pdfium / poppler 首选路——行号在两路间略有差异，
//! 断言按文本标签 NFKC 归一驱动，两路等价成立）；无外部引擎环境
//! （CI Windows）走纯 Rust 兜底路 → 打 SKIP 横幅提前 return（环境差异
//! 不伪装业务回归）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

/// NFKC 归一（黑龙江政采 PDF 文本层用康熙部首区：「合同⽀付⽅式」的
/// ⽀/⽅ 是 U+2F62/U+2F1D——断言两侧归一后比对，#267 同款形态纪律）。
fn nfkc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkc().collect()
}

fn repo_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-star-accept-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 待验收件（有 PDF + manifest 有 star_count）。
struct Tender {
    id: String,
    star_count: i64,
    pdf: PathBuf,
}

fn load_tenders() -> Vec<Tender> {
    let mut out = Vec::new();
    for batch in ["real-tender-batch1", "real-tender-batch2"] {
        let mf = repo_root().join("fixtures").join(batch).join("manifest.json");
        let Ok(m) = std::fs::read_to_string(&mf) else {
            continue;
        };
        let v: Value = serde_json::from_str(&m).unwrap_or_else(|e| panic!("manifest 解析: {e}"));
        for t in v["tenders"].as_array().unwrap_or(&vec![]) {
            let id = t["id"].as_str().unwrap_or("").to_string();
            // 批次一 path 为仓库根相对路径；批次二用 main_file（目录内文件名）。
            // 两种形态都归一到绝对路径再验在位。
            let mut pdf = t["path"].as_str().map(|p| {
                let pb = PathBuf::from(p);
                if pb.is_absolute() { pb } else { repo_root().join(pb) }
            });
            if pdf.is_none() {
                if let Some(f) = t["main_file"].as_str() {
                    pdf = Some(repo_root().join("fixtures").join(batch).join(&id).join(f));
                }
            }
            let Some(pdf) = pdf else { continue };
            if !pdf.exists() || pdf.extension().map(|e| e != "pdf").unwrap_or(true) {
                continue;
            }
            if let Some(sc) = t["star_count"].as_i64() {
                out.push(Tender { id, star_count: sc, pdf });
            }
        }
    }
    out
}

/// 建项目 + 导入单件（每标段独立项目——real_corpus_e2e 同款）。
/// 返回 None = 环境门 SKIP（无 poppler 首选路——CI 兜底路不可对账）。
fn setup_one(tag: &str, tender: &Tender) -> Option<(BidService, String, String, PathBuf)> {
    let ws = tmp_ws(tag);
    let root = tender.pdf.parent().unwrap();
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(root);
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": format!("★验收——{}", tender.id), "bid_deadline": "2026-12-31",
                "mode": "明标", "external_key": format!("star-accept-{tag}") }),
    ))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    let import = match call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [{
            "upload_ref": tender.pdf.display().to_string(),
            "filename": tender.pdf.file_name().unwrap().to_string_lossy(),
            "kind": "招标文件"
        }]}),
    ) {
        ToolReply::Ok(out) => out.result,
        // CI Windows 形态（无 poppler/pdfium）：纯 Rust 兜底路对 CID-CJK 件
        // fail-loud（#169/#200）→ 导入 E_PARTIAL——解析器不可用的环境差异，
        // 不是业务失败（issue #9 SKIP 纪律；对照 real_corpus_e2e 同款处理）
        ToolReply::Err { error, .. } => {
            eprintln!(
                "【SKIP】{} 本环境导入失败（{error}）——无外部引擎形态，star 对账不可信，不伪装通过",
                tender.id
            );
            return None;
        }
    };
    let f0 = &import["per_file"][0];
    if f0["parse_status"] != json!("ok") {
        eprintln!(
            "【SKIP】{} 本环境解析不开（{tag}：{}）——star 对账不可信，不伪装通过",
            tender.id,
            f0["parse_error"]
        );
        return None;
    }
    // 环境门：manifest 与布局保真解析路同源（pdfium/poppler）——纯 Rust
    // 兜底路块结构不同源，对照即伪（#271 同款纪律：基于主件实际解析产物判）
    let layout_path = svc
        .parsed()
        .iter()
        .any(|p| layout_engine_path(&p.parse_summary));
    if !layout_path {
        eprintln!(
            "【SKIP】{} 走纯 Rust 兜底路（无 pdfium/poppler——CI Windows 形态）——与 manifest star_count 不同源，对账 SKIP（issue #9 口径）",
            tender.id
        );
        return None;
    }
    let fid = f0["file_id"].as_str().unwrap().to_string();
    Some((svc, pid, fid, ws))
}

/// 布局引擎预检（house 模式——songling 同款）：poppler 文本三件套不在位
/// 且无 pdfium 显式库（MDC_PDFIUM_LIB）时 SKIP 横幅提前返回——CI Windows
/// 无外部引擎形态下逐件导入既慢（兜底路逐件 fail-loud）又无对账意义
/// （issue #9 口径：环境缺件不伪装业务失败，也不浪费 runner 时间）。
fn skip_if_no_layout_engine(test_id: &str) -> bool {
    // pdfium 显式库在位（引擎路可用）——先查，避免「poppler 缺件但引擎路
    // 可用」的环境误打 SKIP 横幅
    if let Ok(p) = std::env::var("MDC_PDFIUM_LIB") {
        if !p.is_empty() && std::path::Path::new(&p).exists() {
            return false;
        }
    }
    bid_parse::env_preflight::skip_if_env_missing(
        test_id,
        bid_parse::env_preflight::POPPLER_TEXT_TOOLS,
    )
}

fn star_coverage_of(svc: &mut BidService, pid: &str, fid: &str) -> Value {
    ok_result(call(
        svc,
        "get_star_coverage",
        json!({ "project_id": pid, "file_id": fid }),
    ))
}

/// 环境门真源：布局保真解析路 = bid-engine pdfium（service 导入首选，
/// summary `engine-pdf @ …`）或 poppler 首选路（`pdf 文本层: …`）。
/// 纯 Rust 兜底路（`纯 Rust PDF 文本兜底` / `[pdf-single-source]`）块结构
/// 与 manifest 不同源 → 不对账（issue #9：环境差异不伪装业务回归）。
fn layout_engine_path(summary: &str) -> bool {
    summary.starts_with("engine-pdf") || summary.starts_with("pdf 文本层")
}

fn danqing() -> Option<Tender> {
    load_tenders()
        .into_iter()
        .find(|t| t.id == "danqing-reservoir-230001")
}

// ---------------------------------------------------------------- 验收 1+5：manifest 对账

/// 每份文件的 star_total 与 manifest star_count 一致（±2）；分区不漏不重。
#[test]
fn manifest_star_count_reconciliation() {
    if skip_if_no_layout_engine("star_285_manifest_reconciliation") {
        return;
    }
    let tenders = load_tenders();
    assert!(
        tenders.len() >= 9,
        "真件验收语料应在位（batch1×5 + batch2×4 PDF），实际 {} 件",
        tenders.len()
    );
    let mut checked = 0usize;
    let mut wss: Vec<PathBuf> = Vec::new();
    for t in &tenders {
        let Some((mut svc, pid, fid, ws)) = setup_one("manifest", t) else {
            continue;
        };
        wss.push(ws);
        ok_result(call(
            &mut svc,
            "extract_requirements",
            json!({ "project_id": &pid }),
        ));
        let out = star_coverage_of(&mut svc, &pid, &fid);
        let total = out["star_total"].as_i64().unwrap_or(-1);
        let truth = out["star_truth"].as_i64().unwrap_or(-1);
        let cap = out["star_captured"].as_i64().unwrap_or(-1);
        let miss = out["star_missing"].as_i64().unwrap_or(-1);
        let decl = out["declarations"].as_i64().unwrap_or(-1);
        let diff = (total - t.star_count).abs();
        assert!(
            diff <= 2,
            "{}: star_total={total} vs manifest={}（±2 容差破）",
            t.id, t.star_count
        );
        assert_eq!(
            truth,
            cap + miss,
            "{}: 真值 = 已捕获 + 漏标（不漏不重——验收 5）",
            t.id
        );
        assert_eq!(
            total,
            truth + decl,
            "{}: 字面总数 = 真值 + 样板句（口径恒等式）",
            t.id
        );
        println!(
            "  {}: star_total={total} (manifest {}) decl={decl} captured={cap} missing={miss}",
            t.id, t.star_count
        );
        checked += 1;
    }
    for ws in wss {
        let _ = std::fs::remove_dir_all(ws);
    }
    // 环境门全 SKIP 时不算通过也不算失败——但本机/有 poppler 的 CI 必须对账
    if checked == 0 {
        eprintln!("【SKIP】全部件走兜底路——manifest 对账整面 SKIP（环境缺 poppler）");
    }
}

// ---------------------------------------------------------------- 验收 3+4+5：danqing 引擎路径

/// 商务表五题标签（黑龙江政采统一形态——与引擎路（pdfium/poppler）
/// 行号差异无关：断言按文本标签定位，两条解析路都成立）。
const BUSINESS_LABELS: [&str; 5] = ["服务期限", "服务地点", "验收要求", "合同支付方式", "履约验收方式"];

/// danqing 引擎路径：五题★全捕获（#266 语义表 + params.star）、样板句排除、
/// 漏标 6 = p18 参数性质列×4 + p48 偏离表模板×2（诚实报告——检测面不修饰）。
#[test]
fn danqing_engine_path_five_business_captured() {
    if skip_if_no_layout_engine("star_285_danqing_engine_path") {
        return;
    }
    let Some(t) = danqing() else {
        eprintln!("【SKIP】danqing fixture 不在仓");
        return;
    };
    let Some((mut svc, pid, fid, ws)) = setup_one("engine", &t) else {
        return;
    };
    let extract = ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    // 抽取面 star_summary（#285 集成）：真值 12 / 已捕获 6 / 漏 6 / 样板句 1
    let ss = &extract["star_summary"];
    assert_eq!(ss["total"], json!(12), "danqing 真值★（13 字面 - 1 样板句）");
    assert_eq!(ss["captured"], json!(6));
    assert_eq!(ss["missing"], json!(6));
    assert_eq!(ss["declarations"], json!(1));
    assert!(
        extract["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap_or("").starts_with("STAR_MISSING")),
        "漏标>0 必须给 STAR_MISSING 警告: {:?}",
        extract["warnings"]
    );

    let out = star_coverage_of(&mut svc, &pid, &fid);
    assert_eq!(out["star_total"], json!(13), "字面★（与 manifest 13 同口径）");
    assert_eq!(out["declarations"], json!(1), "样板句排除（验收 3）");
    assert_eq!(out["star_truth"], json!(12));
    assert_eq!(out["star_captured"], json!(6));
    assert_eq!(out["star_missing"], json!(6));
    // 验收 3：样板句样本可人工核对，且不出现在 missing/captured
    let sample = out["declaration_samples"][0].as_str().unwrap_or("");
    assert!(sample.contains("实质性条款"), "样板句样本: {sample}");
    for list in ["captured", "missing"] {
        for e in out[list].as_array().unwrap() {
            assert!(
                !e["text"].as_str().unwrap_or("").contains("号条款为实质性条款"),
                "样板句不得计入真值（{list}）: {:?}",
                e["text"]
            );
        }
    }
    // 验收 4：商务表五题（表格列★）5/5 捕获且 star=true（#266+#276 汇流）
    for label in BUSINESS_LABELS {
        let hit = out["captured"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                let txt = nfkc(c["text"].as_str().unwrap_or(""));
                txt.contains('★') && txt.contains(label)
            })
            .unwrap_or_else(|| panic!("商务表★行未捕获: {label}\n{out}"));
        assert_eq!(hit["star"], json!(true), "{label} 必须 star=true（标没标可见）");
    }
    // p18 参数性质列「1 ★」经 REQ 锚捕获但未标★——「命中但未标」可见
    let p18_hit = out["captured"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["text"].as_str().unwrap_or("").trim() == "1 ★")
        .expect("p18 参数性质列「1 ★」应经 REQ 锚捕获");
    assert_eq!(p18_hit["star"], json!(false), "命中但未标★——检测面如实展示");
    // 诚实漏标清单：p18 参数性质列×4 + p48 偏离表模板×2（NFKC 归一比对）
    let missing_texts: Vec<String> = out["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| nfkc(m["text"].as_str().unwrap_or("").trim()))
        .collect();
    assert_eq!(
        missing_texts,
        vec!["2 ★", "3 ★", "4 ★", "5 ★", "★ 1.1", "★ 2.1"],
        "引擎路径漏标清单（参数性质列前导缓冲形态 + 偏离表模板）"
    );
    let _ = std::fs::remove_dir_all(ws);
}

// ---------------------------------------------------------------- 验收 2：黑盒 1/5 → missing=4

/// 黑盒复现（issue 验收 2 的「当前实测捕获 1/5，应报 missing=4」）：
///
/// 六场黑盒实测中 Agent 只把商务表五题的 1 题标成 substantive——本用例
/// 复刻该形态：Agent 标记覆盖商务表区间（第19页 行6..行25 → 引擎正则
/// REQ 让位），仅提交 1 条 substantive（服务期限）；四题必须全部报出。
#[test]
fn danqing_blackbox_one_of_five_reports_missing_four() {
    if skip_if_no_layout_engine("star_285_danqing_blackbox") {
        return;
    }
    let Some(t) = danqing() else {
        eprintln!("【SKIP】danqing fixture 不在仓");
        return;
    };
    let Some((mut svc, pid, fid, ws)) = setup_one("blackbox", &t) else {
        return;
    };
    // 商务表五题★块定位（文本标签驱动——与解析路行号差异无关）。
    // 标记区间 = 五题★块首尾（含中间续行块——均在商务表页内）；
    // 合并面「任一锚命中覆盖块即让位」语义下，覆盖五题★块即足以
    // 让五个引擎 REQ（REQ-表1-行N）整体让位。
    let (from, to, row1) = {
        let pf = svc
            .parsed()
            .iter()
            .find(|p| layout_engine_path(&p.parse_summary))
            .expect("环境门已保证布局保真解析路");
        let is_business_star = |b: &&bid_parse::Block| -> bool {
            let t = nfkc(&b.text);
            t.contains('★') && BUSINESS_LABELS.iter().any(|l| t.contains(*l))
        };
        let blocks: Vec<&bid_parse::Block> =
            pf.blocks.iter().filter(is_business_star).collect();
        assert_eq!(
            blocks.len(),
            5,
            "商务表五题★块应在位（实际 {}）：{:?}",
            blocks.len(),
            blocks.iter().map(|b| b.text.clone()).collect::<Vec<_>>()
        );
        (
            blocks.first().unwrap().idx,
            blocks.last().unwrap().idx,
            blocks
                .iter()
                .find(|b| nfkc(&b.text).contains("服务期限"))
                .expect("行1 服务期限块")
                .idx,
        )
    };

    // Agent 标记：整段 requirement；关键点仅提交 1 条 substantive（黑盒 1/5 形态）
    ok_result(call(
        &mut svc,
        "submit_block_marks",
        json!({ "project_id": &pid, "file_id": &fid,
            "marks": [{ "from": from, "to": to, "verdict": "requirement",
                        "reason": "商务要求表五题★（黑盒 1/5 复现场景）" }],
            "requirements": [{ "block_idx": row1,
                                "statement": "服务期限：自合同签订之日起30个日历日内提交阶段正式成果",
                                "category": "商务要求", "substantive": true }] }),
    ));

    let extract = ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    // 合并面统计可见：Agent 关键点 1 条承接（engine REQ 对被覆盖块让位）
    assert_eq!(extract["merge"]["agent_requirements"], json!(1), "Agent 提交 1 条关键点");

    let out = star_coverage_of(&mut svc, &pid, &fid);
    assert_eq!(out["star_total"], json!(13));
    assert_eq!(out["star_truth"], json!(12));
    // 黑盒 1/5：服务期限捕获（Agent substantive → params.star=true）
    let cap = out["star_captured"].as_i64().unwrap();
    let miss = out["star_missing"].as_i64().unwrap();
    assert_eq!(cap + miss, 12, "不漏不重（验收 5——黑盒路径同样成立）");
    let served = out["captured"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| {
            let txt = nfkc(c["text"].as_str().unwrap_or(""));
            txt.contains('★') && txt.contains("服务期限")
        })
        .expect("服务期限（唯一被 Agent 标记的★题）必须捕获");
    assert_eq!(served["star"], json!(true), "Agent substantive → star=true");
    assert!(
        served["req_id"].as_str().unwrap_or("").starts_with("REQ-A"),
        "Agent REQ 承接: {}",
        served["req_id"]
    );

    // ★验收 2 核心：missing=4——四题全部报出（服务地点/验收要求/合同支付方式/履约验收方式）
    let missing_texts: Vec<String> = out["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| nfkc(m["text"].as_str().unwrap_or("")))
        .collect();
    for label in ["服务地点", "验收要求", "合同支付方式", "履约验收方式"] {
        assert!(
            missing_texts.iter().any(|t| t.contains(label)),
            "漏标四题必须报出：{label} 不在 missing（黑盒 1/5 场景）\nmissing: {missing_texts:?}"
        );
    }
    let business_missing = missing_texts
        .iter()
        .filter(|t| t.contains("★"))
        .filter(|t| {
            ["服务地点", "验收要求", "合同支付方式", "履约验收方式"]
                .iter()
                .any(|l| t.contains(*l))
        })
        .count();
    assert_eq!(business_missing, 4, "商务表漏标恰为 4（missing=4）");
    // 黑盒路径总量口径：Agent 只接 1 题 + p18 行1 锚（REQ-0149 未被覆盖）= 2 捕获
    assert_eq!((cap, miss), (2, 10), "黑盒 1/5 形态：真值 12 = 捕获 2 + 漏 10（四商务题 + p18×4 + p48×2）");
    let _ = std::fs::remove_dir_all(ws);
}

/// 兜底路环境（无 poppler）下 load_tenders/路径门自身可运行（CI 编译面）。
#[test]
fn tender_loader_finds_corpus() {
    let tenders = load_tenders();
    let ids: Vec<&str> = tenders.iter().map(|t| t.id.as_str()).collect();
    for expect in [
        "danqing-reservoir-230001",
        "hq-dev-procure-230001",
        "hc-cluster-230001",
        "college-ai-230001",
        "wenming-longjiang-230001",
    ] {
        assert!(ids.contains(&expect), "batch1 五件应在位：{ids:?}");
    }
    let danqing_star = danqing().map(|t| t.star_count);
    assert_eq!(danqing_star, Some(13), "danqing manifest star_count=13");
}
