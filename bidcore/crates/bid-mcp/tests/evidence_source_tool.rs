//! #185 缺口 1 验收：register_evidence **来源验证**（MCP 工具面全链）。
//!
//! issue #185 原文缺口：`register_evidence` 接受调用方传入
//! `source_file` / `source_sha256` / `fingerprint`，域层只校验格式——可以登记
//! `source_file = "不存在的合同.pdf"`、任意 hex sha 骗过校验成为「可信证据」。
//! 这不是密码学问题，是**来源身份没有验证**。
//!
//! 验收口径（issue 原文）：
//! ```text
//! A. 登记 source_file_id 指向不存在的文件 → 拒绝
//! B. 登记 locator 指向文件不存在的区域 → 拒绝
//! C. 正常登记 → 成功，指纹由系统生成（非调用方传入）
//! ```
//!
//! 另覆盖破坏面纪律：旧参数 `source_file` / `source_sha256` / `fingerprint`
//! 显式拒绝（E_UNPROCESSABLE + 指路 source_file_id——防「以为登记成功」的
//! 假成功），与 #182 reviewer 参数废除同款。
//!
//! 全部用例走 `dispatch_with`（工具面全链：参数校验 → 状态机裁决 →
//! 服务层来源验证 → 域层登记），不绕过壳层直测内部函数。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

// ---------------------------------------------------------------- 调用助手

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

fn err_what(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// 导入回执（Ok 或 E_PARTIAL 均携带 per_file——错误码不吞成功面）里按
/// 文件名取 file_id（外部 Agent 视角的唯一来源发现面）。
fn file_id_of_import(reply: &ToolReply, filename: &str) -> String {
    let data = match reply {
        ToolReply::Ok(out) => out.result.clone(),
        ToolReply::Err {
            error: bid_mcp::ToolError::EPartial { data: Some(d), .. },
            ..
        } => d.clone(),
        other => panic!("导入意外错误: {other:?}"),
    };
    data["per_file"]
        .as_array()
        .unwrap_or_else(|| panic!("导入回执缺 per_file: {data}"))
        .iter()
        .find(|p| p["filename"].as_str() == Some(filename))
        .and_then(|p| p["file_id"].as_str())
        .unwrap_or_else(|| panic!("导入回执缺 {filename} 的 file_id: {data}"))
        .to_string()
}

// ---------------------------------------------------------------- 夹具

/// 仓库夹具根（#183 围栏：canonical 化——词法 `..` 会被围栏预拒）。
fn fixtures_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn synth_dir() -> PathBuf {
    fixtures_root().join("synth-tender/招标文件")
}

fn corpus_evidence(rel: &str) -> PathBuf {
    fixtures_root().join("corpus-A/files/evidence").join(rel)
}

/// 正文段块文本（0 起序——仅计 `BlockKind::Para`，表格行不占号；与
/// 服务层 locator 对账同一基准。测试以「读过文件的调用方」视角取材）。
fn para_texts(svc: &BidService, sha: &str) -> Vec<String> {
    let parsed = svc
        .parsed()
        .iter()
        .find(|p| p.sha256 == sha)
        .unwrap_or_else(|| panic!("解析产物缺 sha={sha}"));
    parsed
        .blocks
        .iter()
        .filter(|b| b.kind == bid_parse::BlockKind::Para)
        .map(|b| b.text.clone())
        .collect()
}

/// 定位文本 → 内容指纹（fnv64:<hex>——与 linkage::text_fingerprint 同源，
/// 测试侧独立复算以核对「系统生成值」）。
fn fp_of(text: &str) -> String {
    format!("fnv64:{}", bid_domain::linkage::text_fingerprint(text))
}

/// PDF 页文本（该页全部行块拼接——与服务层同口径）。
/// 取某 PDF 第 page 页的全部文本（#185 验收 C 的 pdf 页锚用）。
///
/// **必须同时兼容两条解析路**（CI 无 poppler，本地常有——只写一种会「本地绿、
/// CI 红」）：
/// - poppler 首选路：`第1页 行1`（页后无空格）
/// - 纯 Rust 兜底路：`第1 页 > 第1 行`（页后有空格）
fn pdf_page_text(svc: &BidService, sha: &str, page: u32) -> String {
    let parsed = svc
        .parsed()
        .iter()
        .find(|p| p.sha256 == sha)
        .unwrap_or_else(|| panic!("解析产物缺 sha={sha}"));
    // 两种前缀都试：兜底路是「第N 页」（空格在页前）
    let prefixes = [format!("第{page}页"), format!("第{page} 页")];
    parsed
        .blocks
        .iter()
        .filter(|b| prefixes.iter().any(|p| b.locator.starts_with(p.as_str())))
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 建项 + 导入（A 招标文件 docx + EV-02 证据 pdf）+ 抽取 + 建纲 →
/// (svc, pid, A 的 file_id, EV-02 的 file_id)。register_evidence 需
/// MATRIX_READY（G2 状态门——先过建纲，才能测到来源验证面而非 E_STATE）。
fn matrix_ready_store(tag: &str) -> (BidService, String, String, String) {
    // #183 围栏：受信导入根（canonical——synth-tender 与 corpus-A 两个夹具
    // 目录的共同父；词法 `..` 会被围栏预拒）。
    let mut svc = BidService::new().with_fs_root(fixtures_root());
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": format!("#185 来源验证 {tag}"), "bid_deadline": "2026-11-30",
                "mode": "明标", "external_key": format!("evidence-source-{tag}") }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    let ev02 = corpus_evidence("EV-02-电子与智能化工程专业承包二级资质证书.pdf");
    if !ev02.is_file() {
        panic!("夹具缺失: {}", ev02.display());
    }
    let import = ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [
            { "upload_ref": synth_dir().join("A-招标文件.docx").display().to_string(),
              "filename": "A-招标文件.docx", "kind": "招标文件" },
            { "upload_ref": ev02.display().to_string(),
              "filename": "EV-02-电子与智能化工程专业承包二级资质证书.pdf", "kind": "附件" } ] }),
    ));
    assert_eq!(import["totals"]["parse_failed"], json!(0), "A+EV-02 应全成功: {import:?}");
    let fid_a = import["per_file"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["filename"].as_str() == Some("A-招标文件.docx"))
        .and_then(|p| p["file_id"].as_str())
        .unwrap()
        .to_string();
    let fid_ev = import["per_file"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["filename"].as_str() == Some("EV-02-电子与智能化工程专业承包二级资质证书.pdf"))
        .and_then(|p| p["file_id"].as_str())
        .unwrap()
        .to_string();
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    ok_result(call(&mut svc, "build_outline", json!({ "project_id": &pid })));
    (svc, pid, fid_a, fid_ev)
}

/// 库内 TenderFile（按 file_id 查——测试对账用）。
fn tender_file(svc: &BidService, fid: &str) -> bid_domain::TenderFile {
    svc.store()
        .files()
        .iter()
        .find(|f| f.id == fid)
        .unwrap_or_else(|| panic!("TenderFile {fid} 不存在"))
        .clone()
}

// ---------------------------------------------------------------- 验收 A

/// 验收 A：source_file_id 指向不存在的文件 → E_NOTFOUND（零入库）。
#[test]
fn nonexistent_source_file_id_rejected() {
    let (mut svc, pid, _, _) = matrix_ready_store("a");
    let n_before = svc.store().evidences().len();
    let reply = call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "不存在的合同", "source_file_id": "TF-9999",
            "subject_name": "投标人甲",
        }),
    );
    assert_eq!(err_code(&reply), "E_NOTFOUND", "幽灵文件必须拒: {}", err_what(&reply));
    assert!(err_what(&reply).contains("TF-9999"), "错误定位到具体 file_id: {}", err_what(&reply));
    assert_eq!(svc.store().evidences().len(), n_before, "拒绝路径零入库");
}

// ---------------------------------------------------------------- 验收 B

/// 验收 B：locator 指向文件不存在的区域 → E_UNPROCESSABLE（零入库）。
///
/// - docx：paragraph_index 超出正文段范围；
/// - pdf：page 超出解析页数。
#[test]
fn locator_out_of_range_rejected() {
    let (mut svc, pid, fid_a, fid_ev) = matrix_ready_store("b");
    let tf_a = tender_file(&svc, &fid_a);
    let tf_ev = tender_file(&svc, &fid_ev);
    let n_before = svc.store().evidences().len();

    // docx：段落越界（正文段共 N 个，paragraph_index = N + 100）
    let paras = para_texts(&svc, &tf_a.sha256);
    assert!(paras.len() > 5, "A 件正文段规模: {}", paras.len());
    let reply = call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "越界段落证据", "source_file_id": &fid_a,
            "subject_name": "投标人甲",
            "locator": { "kind": "docx_paragraph",
                         "paragraph_index": paras.len() + 100,
                         "text_fingerprint": "fnv64:deadbeef" },
        }),
    );
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "段落越界必须拒: {}", err_what(&reply));
    assert!(err_what(&reply).contains("越界"), "错误文案定位到越界: {}", err_what(&reply));

    // pdf：页号越界（解析页数 + 100）
    let pages = svc
        .parsed()
        .iter()
        .find(|p| p.sha256 == tf_ev.sha256)
        .unwrap()
        .pages;
    assert!(pages >= 1, "EV-02 页数: {pages}");
    let reply = call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "certificate",
            "title": "越界页证据", "source_file_id": &fid_ev,
            "subject_name": "投标人甲",
            "locator": { "kind": "pdf_page_text",
                         "page": pages + 100,
                         "text_fingerprint": "fnv64:deadbeef" },
        }),
    );
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "页号越界必须拒: {}", err_what(&reply));
    assert!(err_what(&reply).contains("越界"), "错误文案定位到越界: {}", err_what(&reply));

    assert_eq!(svc.store().evidences().len(), n_before, "拒绝路径零入库");
}

/// 验收 B 补：locator 在范围内但内容指纹不匹配 → 拒（证明调用方
/// 确实看过该内容——乱填指纹登不进台账）。
#[test]
fn locator_fingerprint_mismatch_rejected() {
    let (mut svc, pid, fid_a, _) = matrix_ready_store("b2");
    let tf_a = tender_file(&svc, &fid_a);
    let n_before = svc.store().evidences().len();
    let paras = para_texts(&svc, &tf_a.sha256);
    let reply = call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "指纹不符证据", "source_file_id": &fid_a,
            "subject_name": "投标人甲",
            "locator": { "kind": "docx_paragraph",
                         "paragraph_index": 0,
                         "text_fingerprint": "fnv64:0000000000000000" },
        }),
    );
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "指纹不符必须拒: {}", err_what(&reply));
    assert!(err_what(&reply).contains("指纹不匹配"), "错误文案定位到指纹: {}", err_what(&reply));
    let expect = fp_of(&paras[0]);
    assert!(err_what(&reply).contains(&expect), "错误文案携带实际指纹便于对账: {}", err_what(&reply));
    assert_eq!(svc.store().evidences().len(), n_before, "拒绝路径零入库");
}

// ---------------------------------------------------------------- 验收 C

/// 验收 C：正常登记 → 成功；sha 与指纹全部由系统从库内与实际内容生成
/// （调用方无从传入——工具面已无该参数，台账值必须等于系统复算值）。
#[test]
fn normal_registration_generates_fingerprint() {
    let (mut svc, pid, fid_a, fid_ev) = matrix_ready_store("c");
    let tf_a = tender_file(&svc, &fid_a);
    let tf_ev = tender_file(&svc, &fid_ev);

    // ---- docx 段落锚：paragraph_index=0 + 实际内容指纹
    let paras = para_texts(&svc, &tf_a.sha256);
    let r = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "电子与智能化工程专业承包二级资质证书（#185 验收 C）",
            "source_file_id": &fid_a,
            "subject_name": "投标人", "scope": "电子与智能化工程",
            "locator": { "kind": "docx_paragraph", "paragraph_index": 0,
                         "text_fingerprint": fp_of(&paras[0]) },
        }),
    ));
    let ev_id = r["ev_id"].as_str().unwrap().to_string();
    assert!(ev_id.starts_with("EV-"), "EV 签发: {r:?}");
    assert_eq!(r["status"], json!("ACTIVE"), "出生态 ACTIVE: {r:?}");
    assert_eq!(r["deduplicated"], json!(false), "首登非重放: {r:?}");
    let ev = svc.store().evidence(&ev_id).unwrap().clone();
    // 来源三件套 = 库内与实际内容（不是任何调用方值——入参面根本没有）
    assert_eq!(ev.source_file, tf_a.name, "source_file 取 TenderFile.name");
    assert_eq!(ev.source_sha256, tf_a.sha256, "source_sha256 取库内 sha（不信调用方）");
    assert_eq!(ev.fingerprint, fp_of(&paras[0]), "指纹 = 定位文本系统生成值");
    // 同 payload 重放 → 幂等（来源验证不破坏语义位查重）
    let r2 = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "电子与智能化工程专业承包二级资质证书（#185 验收 C）",
            "source_file_id": &fid_a,
            "subject_name": "投标人", "scope": "电子与智能化工程",
            "locator": { "kind": "docx_paragraph", "paragraph_index": 0,
                         "text_fingerprint": fp_of(&paras[0]) },
        }),
    ));
    assert_eq!(r2["deduplicated"], json!(true), "重放幂等: {r2:?}");
    assert_eq!(r2["ev_id"], json!(ev_id), "重放回原 id: {r2:?}");

    // ---- pdf 页锚（另一文件、另一语义位）：page=1 + 实际页文本指纹
    let page1 = pdf_page_text(&svc, &tf_ev.sha256, 1);
    assert!(!page1.trim().is_empty(), "EV-02 首页应有文本层");
    let r3 = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "certificate",
            "title": "资质证书扫描件（#185 验收 C·pdf 锚）",
            "source_file_id": &fid_ev,
            "subject_name": "投标人",
            "locator": { "kind": "pdf_page_text", "page": 1,
                         "text_fingerprint": fp_of(&page1) },
        }),
    ));
    let ev3 = svc.store().evidence(r3["ev_id"].as_str().unwrap()).unwrap().clone();
    assert_eq!(ev3.source_sha256, tf_ev.sha256, "pdf 锚也取库内 sha");
    assert_eq!(ev3.fingerprint, fp_of(&page1), "pdf 锚指纹 = 页文本系统生成值");

    // ---- 文件级锚（不带 locator）：指纹退 sha256:<库内 sha>（真源是库）
    let r4 = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "营业执照（#185 验收 C·文件级锚）",
            "source_file_id": &fid_ev,
            "subject_name": "投标人",
        }),
    ));
    let ev4 = svc.store().evidence(r4["ev_id"].as_str().unwrap()).unwrap().clone();
    assert_eq!(ev4.fingerprint, format!("sha256:{}", tf_ev.sha256), "文件级锚指纹 = 库内 sha 派生");
    assert_eq!(
        ev4.locator,
        bid_domain::locator::EngineLocator::MarkdownBlock {
            block_id: "mcp-registered:EV-02-电子与智能化工程专业承包二级资质证书.pdf".into(),
            text_fingerprint: format!("sha256:{}", tf_ev.sha256),
        },
        "文件级锚诚实标记登记面（不冒充引擎抽取）"
    );
}

// ---------------------------------------------------------------- 破坏面纪律

/// 旧参数 source_file / source_sha256 / fingerprint 显式拒绝——
/// E_UNPROCESSABLE + 文案指路 source_file_id（防「以为登记成功」的假成功）。
#[test]
fn legacy_source_params_rejected() {
    let (mut svc, pid, fid_a, _) = matrix_ready_store("legacy");
    let n_before = svc.store().evidences().len();
    let base = json!({
        "project_id": &pid, "kind": "qualification",
        "title": "旧参数样例", "source_file_id": &fid_a,
        "subject_name": "投标人甲",
    });
    for (legacy, value) in [
        ("source_file", json!("EV-虚构.pdf")),
        ("source_sha256", json!("sha-attacker-controlled")),
        ("fingerprint", json!("fnv64:forged")),
    ] {
        let mut args = base.clone();
        args[legacy] = value;
        let reply = call(&mut svc, "register_evidence", args);
        assert_eq!(
            err_code(&reply),
            "E_UNPROCESSABLE",
            "旧参数 {legacy} 必须显式拒: {}",
            err_what(&reply)
        );
        let what = err_what(&reply);
        assert!(what.contains(legacy), "错误点名废除参数: {what}");
        assert!(what.contains("source_file_id"), "错误指路 source_file_id: {what}");
    }
    assert_eq!(svc.store().evidences().len(), n_before, "拒绝路径零入库");
}

/// 缺 source_file_id → E_UNPROCESSABLE（新必填参数）。
#[test]
fn missing_source_file_id_rejected() {
    let (mut svc, pid, _, _) = matrix_ready_store("missing");
    let reply = call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "缺来源参数", "subject_name": "投标人甲",
        }),
    );
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "缺 source_file_id 拒: {}", err_what(&reply));
}

/// 不支持的 locator 形态（markdown_block / xlsx_cell）→ 显式拒，不静默跳过。
#[test]
fn unsupported_locator_kind_rejected() {
    let (mut svc, pid, fid_a, _) = matrix_ready_store("locator-kind");
    for kind in ["markdown_block", "xlsx_cell"] {
        let reply = call(
            &mut svc,
            "register_evidence",
            json!({
                "project_id": &pid, "kind": "qualification",
                "title": "不支持定位器形态", "source_file_id": &fid_a,
                "subject_name": "投标人甲",
                "locator": { "kind": kind, "text_fingerprint": "fnv64:xx" },
            }),
        );
        assert_eq!(
            err_code(&reply),
            "E_UNPROCESSABLE",
            "locator.kind={kind} 应显式拒（fail-closed）: {}",
            err_what(&reply)
        );
    }
}

/// 解析失败件禁引（R-01）：source_file_id 指向 Failed 档 → E_UNPROCESSABLE。
///
/// D-图纸说明.md 是不支持的文件类型 → Failed 落库（显式登记）；同一批的
/// A 件正常导入支撑 extract/build_outline 过 MATRIX_READY 状态门。
#[test]
fn failed_parse_file_rejected() {
    // #183 围栏：声明夹具根（A 正常件 + D 坏件都在 synth-tender 下）。
    let mut svc = BidService::new().with_fs_root(fixtures_root());
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": "#185 失败件禁引", "bid_deadline": "2026-11-30",
                "mode": "明标", "external_key": "evidence-source-failed" }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    let reply = call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [
            { "upload_ref": synth_dir().join("A-招标文件.docx").display().to_string(),
              "filename": "A-招标文件.docx", "kind": "招标文件" },
            { "upload_ref": synth_dir().join("D-图纸说明.md").display().to_string(),
              "filename": "D-图纸说明.md", "kind": "图纸" } ] }),
    );
    // D 件失败 → E_PARTIAL（错误码不吞成功面：A 已落库）
    let fid_d = file_id_of_import(&reply, "D-图纸说明.md");
    let tf_d = tender_file(&svc, &fid_d);
    assert_eq!(tf_d.parse_status, bid_domain::ParseStatus::Failed, "D 件应 Failed 档: {tf_d:?}");
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    ok_result(call(&mut svc, "build_outline", json!({ "project_id": &pid })));

    let n_before = svc.store().evidences().len();
    let reg = call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "product_doc",
            "title": "图纸说明（失败件）", "source_file_id": &fid_d,
            "subject_name": "投标人甲",
        }),
    );
    assert_eq!(err_code(&reg), "E_UNPROCESSABLE", "失败件禁引: {}", err_what(&reg));
    assert!(err_what(&reg).contains("解析失败"), "错误文案定位到失败件: {}", err_what(&reg));
    assert_eq!(svc.store().evidences().len(), n_before, "拒绝路径零入库");
}
