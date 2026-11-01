//! #59 阶段 7 · PDF 导出测试（LibreOffice headless 通道）。
//!
//! Lead 验收线：
//! - 页眉页脚页码正确（Writer 渲染管线承载——本测以「PDF 文本层含页脚
//!   模板文字」等价验证）；
//! - 图片不丢（无图片夹具面——由 Writer 管线保证，不在本测伪造）；
//! - 中文字体嵌入（以「pdftotext 提取的中文完整」等价验证——渲染失败的
//!   中文提取不出来）；
//! - 100 页文档转换 < 30s。
//!
//! SKIP 纪律（issue #9）：soffice / pdftotext 缺失 → 响亮横幅 + return
//! （环境缺失不是业务失败）。

use bid_domain::evidence::EvidenceSpec;
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineSpec;
use bid_domain::{Anchor, ParseStatus, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn env_ready() -> bool {
    let missing: Vec<&str> = ["soffice", "pdftotext", "pdfinfo"]
        .iter()
        .filter(|t| which(t).is_none())
        .copied().collect();
    if missing.is_empty() {
        return true;
    }
    eprintln!(
        "SKIP: pdf_export——环境缺失: {}（安装 LibreOffice/poppler-utils 后重跑）",
        missing.join(", ")
    );
    false
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-pdf-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 完整导出前置夹具（绑定 + 三门 + 全叶 draft——EXPORTED 就绪态）。
fn svc_export_ready(tag: &str, n_reqs: usize, target_chars: u32) -> (BidService, String) {
    let cats = ["资格条件", "否决项", "评分项", "技术参数", "商务要求", "格式要求"];
    let mut svc = BidService::open(tmp_dir(tag)).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "PDF 导出测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", &format!("sha-pdf-{tag}"), 50, 5, 300, 0, ParseStatus::Ok, "",
    ).unwrap();
    let reqs: Vec<Requirement> = (0..n_reqs)
        .map(|i| Requirement {
            id: format!("REQ-{:04}", i + 1),
            category: cats[i % cats.len()].into(),
            clause_no: None,
            anchors: vec![Anchor {
                file: "A-招标文件.docx".into(), kind: "para".into(),
                locator: format!("第{}章", i / 10 + 1), excerpt: format!("第 {} 条合成要求", i + 1),
            }],
            params: Default::default(), source_file: "A-招标文件.docx".into(),
        })
        .collect();
    svc.set_requirements(reqs).unwrap();
    let _ = svc.prepare_outline_and_matrix(
        OutlineSpec { target_total_chars: target_chars, max_reqs_per_leaf: 8 },
    ).unwrap();
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "合成资质".into(), source_file: "EV.pdf".into(),
        source_sha256: "sha-ev".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 0, text_fingerprint: "fnv64:p".into() },
        fingerprint: "fnv64:p".into(), valid_from: None, valid_until: None,
        subject_name: "投标人".into(), scope: "合成".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
    }).unwrap();
    let must: Vec<String> = svc.store().requirements().iter()
        // #110 任务 1 缺失回报闸：技术参数类也须证据佐证（否则 draft 拒生成）
        .filter(|r| matches!(r.category.as_str(), "资格条件" | "否决项" | "技术参数"))
        .map(|r| r.id.clone()).collect();
    for rid in &must {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval, Gate::PreExportConfirm] {
        svc.store_mut().record_review(gate, "human:pdf-lead", Decision::Approved, vec![]).unwrap();
    }
    let parents: std::collections::HashSet<String> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.clone()).collect();
    let leaves: Vec<String> = svc.store().sections().iter()
        .filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect();
    for sid in &leaves {
        svc.draft_section(sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    (svc, pid)
}

/// pdftotext 提取全文（中文验证面）。
fn pdf_text(pdf: &std::path::Path) -> String {
    let out = std::process::Command::new("pdftotext")
        .arg(pdf).arg("-")
        .output().expect("pdftotext 已预检");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// pdfinfo 页数。
fn pdf_pages(pdf: &std::path::Path) -> u32 {
    let out = std::process::Command::new("pdfinfo").arg(pdf)
        .output().expect("pdfinfo 已预检");
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().find_map(|l| l.strip_prefix("Pages:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- 1. 主链：docx 链 → pdf 产物

#[test]
fn pdf_export_full_chain_with_chinese_and_footer() {
    if !env_ready() {
        return;
    }
    let (mut svc, pid) = svc_export_ready("main", 24, 12_000);
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "pdf", "output_path": "out/投标文件.pdf",
    })));
    let url = r["artifact"]["url"].as_str().unwrap().to_string();
    assert!(url.ends_with(".pdf"), "产物路径: {url}");
    assert!(r["artifact"]["size"].as_u64().unwrap() > 10_000, "产物 >10KB");
    assert_eq!(r["artifact"]["sha256"].as_str().unwrap().len(), 64, "PDF 的 sha256（非 docx 中间件）");
    assert_eq!(r["project_status"], "EXPORTED");
    // warning 面：转换耗时告知
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("PDF 转换")),
        "转换告警在位: {}", r["warnings"]);
    // 魔数 + 页数 + 中文
    let pdf = std::path::Path::new(&url);
    assert!(std::fs::read(pdf).unwrap().starts_with(b"%PDF-"), "PDF 魔数");
    let pages = pdf_pages(pdf);
    assert!(pages >= 3, "页数 {pages}（12,000 字应 ≥3 页）");
    let text = pdf_text(pdf);
    assert!(text.contains("资格响应") || text.contains("章"), "中文渲染可提取（章标题）");
    // 页脚模板（format-spec 基线：第 {page} 页 共 {pages} 页——Writer 展开）
    assert!(text.contains("第") && text.contains("页"), "页脚页码域展开: 首百字符 {:?}",
        &text.chars().take(80).collect::<String>());
    // 正文进 PDF（mock 正文固定句）
    assert!(text.contains("招标文件的实质性要求和条件"), "正文内容进 PDF");
}

// ---------------------------------------------------------------- 2. 100 页 < 30s 红线

#[test]
fn pdf_export_100pages_under_30s() {
    if !env_ready() {
        return;
    }
    // ~120 叶 × 2,000 字 ≈ 240,000 字 ≈ 100+ 页
    let (mut svc, pid) = svc_export_ready("scale", 960, 240_000);
    let t0 = std::time::Instant::now();
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "pdf", "output_path": "out/百页.pdf",
    })));
    let elapsed = t0.elapsed().as_millis();
    let url = r["artifact"]["url"].as_str().unwrap().to_string();
    let pages = pdf_pages(std::path::Path::new(&url));
    println!("[PDF 规模] {pages} 页 / {elapsed}ms（红线 30s）");
    assert!(pages >= 80, "页数 {pages}（百页口径的 ±20%）");
    assert!(elapsed < 30_000, "红线：{pages} 页转换 {elapsed}ms < 30s");
}

// ---------------------------------------------------------------- 3. 暗标与幂等

#[test]
fn pdf_export_dark_bid_scrubbed() {
    if !env_ready() {
        return;
    }
    let (mut svc, pid) = svc_export_ready("dark", 12, 6_000);
    // 植入公司名稿
    let sid: String = {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        svc.store().sections().iter().find(|n| !parents.contains(&n.id)).unwrap().id.clone()
    };
    struct CompanyGen;
    impl bid_tools::draft::SectionGenerator for CompanyGen {
        fn generate(&mut self, i: &bid_tools::draft::SectionDraftInput)
            -> Result<bid_tools::draft::GeneratedDraft, String> {
            let base = bid_tools::draft::generate_mock(i);
            Ok(bid_tools::draft::GeneratedDraft {
                content: format!("{base}\n\n某某建设工程有限公司承建本项目。"),
            })
        }
    }
    svc.draft_section(&sid, None, None, Some("公司名稿"), &mut CompanyGen).unwrap();
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "pdf", "output_path": "out/暗标.pdf",
        "profile": { "dark_bid": true, "bidder_name": "某某建设工程有限公司" },
    })));
    let url = r["artifact"]["url"].as_str().unwrap().to_string();
    let text = pdf_text(std::path::Path::new(&url));
    assert!(!text.contains("某某建设工程有限公司"), "暗标：PDF 文本层零公司名");
    assert!(text.contains("投标人"), "暗标：替换词在位");
}

#[test]
fn pdf_export_idempotent_same_artifact() {
    if !env_ready() {
        return;
    }
    let (mut svc, pid) = svc_export_ready("idem", 12, 6_000);
    let a = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "pdf", "output_path": "out/i.pdf",
    })));
    let sha_a = a["artifact"]["sha256"].as_str().unwrap().to_string();
    let mtime_a = std::fs::metadata(
        a["artifact"]["url"].as_str().unwrap()
    ).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    let b = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "pdf", "output_path": "out/i.pdf",
    })));
    // 幂等：staging docx 未变 → 不重转（产物 mtime 不变——soffice 的 PDF
    // 二进制不确定（CreationDate/ID），「不重写」由跳过路径承载）
    assert!(
        b["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("PDF 幂等")),
        "第二次必须走跳过路径: {}", b["warnings"]
    );
    let mtime_b = std::fs::metadata(
        b["artifact"]["url"].as_str().unwrap()
    ).unwrap().modified().unwrap();
    assert_eq!(mtime_a, mtime_b, "产物不重写（mtime 不变）");
    assert_eq!(a["artifact"]["sha256"], b["artifact"]["sha256"]);
}

// ---------------------------------------------------------------- 4. 诚实拒

#[test]
fn pdf_rejects_bad_format_and_env_honored() {
    let (mut svc, pid) = svc_export_ready("reject", 8, 4_000);
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "xlsx", "output_path": "out/x.xlsx",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
    // docx 通道不受 pdf 环境影响（回归）
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/回归.docx",
    })));
    assert!(r["artifact"]["url"].as_str().unwrap().ends_with(".docx"));
}
