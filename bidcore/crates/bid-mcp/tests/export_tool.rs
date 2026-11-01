//! #59 阶段 2 · export_bid_document 测试（服务层 + 壳层 + 暗标 + format-spec 集成）。
//!
//! 验收面（Lead 指令 + 契约 §8）：
//! - 双前置门：blocking 矩阵行未清零 → E_UNPROCESSABLE 带清单；
//! - format-spec：.mang/format-spec.json 读盘优先 / 条款提取兜底 / 基线降级；
//! - 暗标：公司名 → 「投标人」（正文 + 章标题）；bidder 空 → warning；
//! - 幂等：同 (content_version, format, dark_bid, path) 重放 deduplicated；
//! - 状态推进 EXPORTED（Drafting 起合法链）；EXPORTED 重导放行；
//! - pdf → 诚实拒；路径逃逸 → 拒；
//! - 产物验证：docx 解包含页眉页脚 + 字体（format-spec 应用面）。

use bid_domain::outline::OutlineSpec;
use bid_domain::{Anchor, ParseStatus, ProjectStatus, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::Read;

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

fn mk_req(id: &str, category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第二章/1.1".into(),
            excerpt: excerpt.into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}


/// 三道门过审（T-7 审核闭环——reviewer 必须 human: 前缀，AI 不得自审自过）。
fn pass_three_gates(svc: &mut BidService) {
    use bid_domain::review::{Decision, Gate};
    // #131：无补遗确认门一并过（基座只导入招标文件，L3 阻塞的人工出口；
    // #118 合入时漏适配本套件的起草面）
    svc.store_mut()
        .record_review(Gate::AddendumWaiver, "human:lead", Decision::Approved, vec![])
        .unwrap_or_else(|e| panic!("无补遗确认门失败（{e:?}）"));
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval, Gate::PreExportConfirm] {
        svc.store_mut().record_review(gate, "human:lead", Decision::Approved, vec![])
            .unwrap_or_else(|e| panic!("三门过审失败（{e:?}）——门序/状态窗检查夹具"));
    }
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-export-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 完整链夹具：create → import → extract → outline → matrix → draft（全部叶 DRAFTED）→ 项目 DRAFTING。
fn svc_drafted(tag: &str, format_req: bool) -> (BidService, String, std::path::PathBuf) {
    let dir = tmp_dir(tag);
    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "导出测试项目", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", &format!("sha-{tag}"), 10, 1, 50, 0, ParseStatus::Ok, "",
    ).unwrap();
    let mut reqs = vec![
        mk_req("REQ-0001", "资格条件", "投标人须具备水利水电工程施工总承包贰级及以上资质"),
        mk_req("REQ-0002", "否决项", "投标文件未经法定代表人签字的作否决处理"),
    ];
    if format_req {
        reqs.push(mk_req("REQ-0003", "格式要求", "投标文件正文用仿宋三号字，行距为 1.5 倍"));
    }
    svc.set_requirements(reqs).unwrap();
    let _ = svc.prepare_outline_and_matrix(OutlineSpec { target_total_chars: 1_500, max_reqs_per_leaf: 8 })
        .unwrap();
    // 证据绑定（导出门的放行面：必选行需响应——register + bind 走真实链路）
    use bid_domain::evidence::EvidenceSpec;
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "水利水电工程施工总承包贰级资质证书".into(),
        source_file: "EV-01.pdf".into(),
        source_sha256: format!("sha-ev-{tag}"),
        locator: EngineLocator::DocxParagraph { paragraph_index: 3, text_fingerprint: "fnv64:1a".into() },
        fingerprint: "fnv64:1a".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某建设工程有限公司".into(),
        scope: "水利水电".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(), // #147
    }).unwrap();
    for rid in ["REQ-0001", "REQ-0002"] {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap(); // 绑定后重建（幂等派生态）
    pass_three_gates(&mut svc);
    // 全部叶起草（mock 生成器——draft 工具在并往 PR #60；此处直接走服务面同真源）
    let leaf_ids: Vec<String> = {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        svc.store().sections().iter()
            .filter(|n| !parents.contains(&n.id))
            .map(|n| n.id.clone()).collect()
    };
    assert!(!leaf_ids.is_empty());
    for sid in &leaf_ids {
        svc.draft_section(sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Drafting);
    (svc, pid, dir)
}

/// 解包 docx 取部件文本。
fn part_text(path: &std::path::Path, name: &str) -> String {
    let bytes = std::fs::read(path).unwrap();
    let cursor = std::io::Cursor::new(&bytes);
    let mut zip = zip::ZipArchive::new(cursor).expect("docx 必须是合法 zip");
    let mut f = zip.by_name(name).unwrap_or_else(|_| panic!("缺部件 {name}"));
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    s
}

// ---------------------------------------------------------------- 1. 主链与契约形状

#[test]
fn export_full_chain_and_contract_shape() {
    let (mut svc, pid, _dir) = svc_drafted("shape", true);
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/标书.docx",
    })));
    // 契约 §8 out
    assert!(r["artifact"]["url"].as_str().unwrap().ends_with("标书.docx"));
    assert_eq!(r["artifact"]["sha256"].as_str().unwrap().len(), 64, "sha256 十六进制 64 位");
    assert!(r["artifact"]["size"].as_u64().unwrap() > 0);
    assert!(r["chapters_written"].as_u64().unwrap() >= 1);
    assert!(r["total_chars"].as_u64().unwrap() > 0);
    assert_eq!(r["toc_field_present"], json!(true));
    assert_eq!(r["project_status"], "EXPORTED", "导出成功推进 EXPORTED");
    assert_eq!(r["precheck"]["blockers"], json!(0));
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("提取")),
        "format-spec 条款提取告警在位: {}", r["warnings"]);
    // 状态机：EXPORTED
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Exported);
    // 产物在盘
    let url = r["artifact"]["url"].as_str().unwrap();
    assert!(std::path::Path::new(url).is_file(), "产物落盘: {url}");
    // format-spec 应用面：styles.xml 有提取的字体字号（三号 16pt → 32 半点）
    let styles = part_text(std::path::Path::new(url), "word/styles.xml");
    assert!(styles.contains("w:sz w:val=\"32\""), "三号字号应用（提取链生效）");
    let header = part_text(std::path::Path::new(url), "word/header1.xml");
    assert!(header.contains("导出测试项目"), "页眉项目名");
}

#[test]
fn export_idempotent_replay() {
    // 双层幂等：① manifest 指纹层（首次导出推进 content_version 后，同输入
    // 重导走「指纹全等 + sha 吻合 → no-op 直返」——产物一个字节不重写）；
    // ② 幂等键层（EXPORTED 稳定态下同键重放 → deduplicated=true 零 IO）。
    let (mut svc, pid, _dir) = svc_drafted("idem", false);
    let a = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/标书.docx",
    })));
    // 第二次：状态推进后的 content_version 与首键不同——走 manifest no-op
    // （结果全等；产物零重写）
    let b = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/标书.docx",
    })));
    assert_eq!(a["artifact"]["sha256"], b["artifact"]["sha256"], "产物 sha 全等（manifest no-op）");
    assert_eq!(a["chapters_written"], b["chapters_written"]);
    // 第三次：EXPORTED 稳定态（content_version 不再变）——幂等键命中
    let c = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/标书.docx",
    })));
    assert_eq!(c["deduplicated"], json!(true), "稳定态重放 → deduplicated");
    assert_eq!(a["artifact"]["sha256"], c["artifact"]["sha256"]);
}

#[test]
fn export_pdf_channel_or_env_honest() {
    // 阶段 7 后：pdf 已实现——soffice 在场则真转换，缺失则响亮环境告警
    // （两条路径都不得是「空成功」或「未实现」）
    let (mut svc, pid, _) = svc_drafted("pdf", false);
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "pdf", "output_path": "out/标书.pdf",
    }));
    match &reply {
        ToolReply::Ok(out) => {
            assert!(out.result["artifact"]["url"].as_str().unwrap().ends_with(".pdf"));
        }
        ToolReply::Err { error, .. } => {
            assert_eq!(error.code(), "E_UNPROCESSABLE");
            assert!(
                format!("{error}").contains("soffice") || format!("{error}").contains("LibreOffice"),
                "环境缺失必须指路 LibreOffice: {error}"
            );
        }
    }
}

#[test]
fn export_rejects_path_escape() {
    let (mut svc, pid, _) = svc_drafted("escape", false);
    for bad in ["../outside.docx", "/abs/path.docx"] {
        let reply = call(&mut svc, "export_bid_document", json!({
            "project_id": pid, "format": "docx", "output_path": bad,
        }));
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "路径逃逸必须拒: {bad}");
    }
}

#[test]
fn export_rejects_wrong_project_and_version_conflict() {
    let (mut svc, pid, _) = svc_drafted("route", false);
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": "PRJ-9999", "format": "docx", "output_path": "o.docx",
    }));
    assert_eq!(err_code(&reply), "E_NOTFOUND");
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "o.docx", "project_version": 1,
    }));
    assert_eq!(err_code(&reply), "E_CONFLICT");
}

// ---------------------------------------------------------------- 2. 前置门

#[test]
fn export_blocked_by_unresponded_mandatory_rows() {
    // blocking 矩阵行未清零（有资格/否决 REQ 但零证据绑定零覆写 → NotResponded）
    // → E_UNPROCESSABLE 逐行列出——防废标门（R-03/R-04 闸门）
    let dir = tmp_dir("blocking");
    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "阻断测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "blocking",
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A.docx", "招标文件", "sha-blk", 5, 0, 20, 0, ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "具备贰级资质"),
        mk_req("REQ-0002", "否决项", "未签字作否决"),
    ]).unwrap();
    // max_reqs_per_leaf=1：资格 REQ-0001 与否决 REQ-0002 分属两叶——
    // 「资格叶（台账有资质）可起草推进 Drafting + 否决叶（未响应）拒生成
    // 保持 Pending」是本用例的设计意图；两 REQ 同叶时全拒，项目停在
    // MatrixReady，导出闸测不到（#131 修复：#118 后本用例在 main 上已断）。
    let _ = svc.prepare_outline_and_matrix(OutlineSpec { target_total_chars: 1_000, max_reqs_per_leaf: 1 }).unwrap();
    // #131：无补遗确认门（L3 人工出口——本用例面在导出闸语义，不在补遗面；
    // 不过此门则全部叶被 MissingAddendum 拒，状态停在导出前，闸语义测不到）
    use bid_domain::review::{Decision, Gate};
    svc.store_mut()
        .record_review(Gate::AddendumWaiver, "human:lead", Decision::Approved, vec![])
        .unwrap();
    // 先起草（进 DRAFTING——export 的 guard 面），保持零绑定零覆写。
    // 缺失回报闸（#110 任务 1）：登记台账资质（不绑）让资格叶可起草；含未响应
    // 否决项（REQ-0002）的叶拒生成保持 Pending——未响应面仍由导出门逐行列出
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let _ = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: "水利水电工程施工总承包贰级资质证书".into(),
        source_file: "EV-ledger.pdf".into(),
        source_sha256: "sha-blk-ledger".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 7, text_fingerprint: "fnv64:bl".into() },
        fingerprint: "fnv64:bl".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某公司".into(),
        scope: "水利水电".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(), // #147
    }).unwrap();
    let parents: std::collections::HashSet<String> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.clone()).collect();
    let leaves: Vec<String> = svc.store().sections().iter()
        .filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect();
    for sid in &leaves {
        match svc.draft_section(sid, None, None, None, &mut bid_tools::draft::MockGenerator) {
            Ok(_) => {}
            Err(e) => assert_eq!(e.code(), "E_UNPROCESSABLE",
                "零绑定时只有含未响应否决项的叶会拒（闸语义）: {e:?}"),
        }
    }
    // 无绑定无覆写：矩阵必选行 NotResponded → 导出必须拒（防废标门）
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/x.docx",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
    let msg = format!("{reply:?}");
    assert!(msg.contains("REQ-0001"), "逐行列出: {msg}");
    assert!(msg.contains("R-03/R-04"), "闸门语义: {msg}");
    // 人工覆写响应后放行（矩阵门的解除通道——NOT_APPLICABLE 不可无理由，
    // 用 Responded 覆写）
    svc.store_mut().set_matrix_override(
        "REQ-0001".into(),
        bid_domain::matrix::ResponseStatus::Responded,
        None,
    ).unwrap();
    svc.store_mut().set_matrix_override(
        "REQ-0002".into(),
        bid_domain::matrix::ResponseStatus::Responded,
        None,
    ).unwrap();
    let _ = svc.store_mut().build_matrix().unwrap();
    // 仍需章内容：draft 全部叶
    let parents: std::collections::HashSet<String> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.clone()).collect();
    let leaves: Vec<String> = svc.store().sections().iter()
        .filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect();
    for sid in &leaves {
        svc.draft_section(sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    pass_three_gates(&mut svc);
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/x.docx",
    })));
    assert_eq!(r["project_status"], "EXPORTED", "覆写响应 + 三门过审后导出放行");
}

#[test]
fn export_requires_persisted_workspace() {
    // 纯内存态（BidService::new）无工作区 → E_UNPROCESSABLE（无章内容可导）
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "内存态", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "mem",
    })))["project_id"].as_str().unwrap().to_string();
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "o.docx",
    }));
    let code = err_code(&reply);
    assert!(code == "E_UNPROCESSABLE" || code == "E_STATE", "内存态导出必须被拒（实际 {code}）");
}

// ---------------------------------------------------------------- 3. 暗标

#[test]
fn export_dark_bid_anonymizes_company_name() {
    let (mut svc, pid, dir) = svc_drafted("dark", false);
    // 在正文中植入公司名（重起草一章带公司名的稿）
    let sid: String = {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        svc.store().sections().iter().find(|n| !parents.contains(&n.id)).unwrap().id.clone()
    };
    struct CompanyGen;
    impl bid_tools::draft::SectionGenerator for CompanyGen {
        fn generate(&mut self, input: &bid_tools::draft::SectionDraftInput)
            -> Result<bid_tools::draft::GeneratedDraft, String> {
            let base = bid_tools::draft::generate_mock(input);
            Ok(bid_tools::draft::GeneratedDraft {
                content: format!("{}\n\n本项目由某某建设工程有限公司承建，某某建设工程有限公司具有同类业绩。",
                    base),
            })
        }
    }
    svc.draft_section(&sid, None, None, Some("带公司名稿"), &mut CompanyGen).unwrap();

    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/暗标.docx",
        "profile": { "dark_bid": true, "bidder_name": "某某建设工程有限公司" },
    })));
    let url = r["artifact"]["url"].as_str().unwrap();
    let doc_xml = part_text(std::path::Path::new(url), "word/document.xml");
    assert!(!doc_xml.contains("某某建设工程有限公司"), "暗标：公司名必须全部脱敏");
    assert!(doc_xml.contains("投标人"), "暗标：替换为「投标人」");
    let _ = dir;
}

#[test]
fn export_dark_bid_missing_bidder_warns() {
    let (mut svc, pid, _) = svc_drafted("dark-empty", false);
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/d.docx",
        "profile": { "dark_bid": true },
    })));
    let warnings = r["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w.as_str().unwrap().contains("bidder_name")),
        "暗标漏配必须响亮告警: {warnings:?}"
    );
}

// ---------------------------------------------------------------- 4. format-spec 装载优先级

#[test]
fn export_handwritten_spec_overrides_derivation() {
    let (mut svc, pid, dir) = svc_drafted("spec-file", true); // 含「三号」条款（提取 → 32 半点）
    // 手写 spec（五号 10.5pt → 21 半点）应压过条款提取
    let mang = dir.join(".mang");
    std::fs::create_dir_all(&mang).unwrap();
    std::fs::write(mang.join("format-spec.json"), serde_json::json!({
        "global": {
            "page": { "size": "A4", "margin": { "top": 35, "bottom": 30, "left": 28, "right": 26 } },
            "font": { "body": "宋体", "size": 10.5, "lineHeight": 1.5 },
            "header": { "enabled": true, "content": "{project_name}" },
            "footer": { "enabled": true, "content": "第 {page} 页" }
        }
    }).to_string()).unwrap();
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/s.docx",
    })));
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("format-spec.json 装载")));
    let url = r["artifact"]["url"].as_str().unwrap();
    let styles = part_text(std::path::Path::new(url), "word/styles.xml");
    assert!(styles.contains("w:sz w:val=\"21\""), "手写 spec 的五号（21 半点）压过条款提取的三号（32）");
    assert!(styles.contains("宋体"));
}

#[test]
fn export_rejects_corrupt_spec_file() {
    let (mut svc, pid, dir) = svc_drafted("spec-bad", false);
    let mang = dir.join(".mang");
    std::fs::create_dir_all(&mang).unwrap();
    std::fs::write(mang.join("format-spec.json"), "{ not valid json").unwrap();
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/x.docx",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "坏 spec 必须拒（fail-closed，不静默降级）");
}

#[test]
fn export_baseline_fallback_when_no_format_clauses() {
    let (mut svc, pid, _) = svc_drafted("baseline", false); // 无格式条款
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/b.docx",
    })));
    assert!(
        r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("基线")),
        "零命中必须按基线 + 告警: {}", r["warnings"]
    );
    let url = r["artifact"]["url"].as_str().unwrap();
    let styles = part_text(std::path::Path::new(url), "word/styles.xml");
    assert!(styles.contains("仿宋_GB2312"), "基线字体");
    assert!(styles.contains("w:sz w:val=\"24\""), "基线小四（24 半点）");
}

// ---------------------------------------------------------------- #86 摩擦 #14：watermark 告警降噪

#[test]
fn export_watermark_false_does_not_warn() {
    // watermark:false 是显式关闭——不该触发「暂不支持」告警（旧实现按键
    // 在场即告警，噪音面）；watermark:true 仍应如实告警（未支持面不静默）
    let (mut svc, pid, _) = svc_drafted("wm-false", false);
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": pid, "format": "docx", "output_path": "out/标书-a.docx",
        "options": { "watermark": false },
    })));
    assert!(
        !r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("watermark")),
        "显式关闭不告警: {}",
        r["warnings"]
    );

    let (mut svc2, pid2, _) = svc_drafted("wm-true", false);
    let r2 = ok_result(call(&mut svc2, "export_bid_document", json!({
        "project_id": pid2, "format": "docx", "output_path": "out/标书-b.docx",
        "options": { "watermark": true },
    })));
    assert!(
        r2["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("watermark")),
        "开启未支持面仍告警: {}",
        r2["warnings"]
    );
}
