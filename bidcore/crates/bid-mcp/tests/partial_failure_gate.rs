//! partial_failure_gate.rs —— 地图 P0-1「部分失败不得错误放行」的 MCP 面验收。
//!
//! 验收场景（对应任务书）：
//! - 场景 1：补遗 OCR_PENDING → 起草被拒（E_UNPROCESSABLE，逐件点名）；
//! - 场景 2：附件解析失败 → 起草被拒（同上——#184 只拦补遗，P0-1 扩到附件）；
//! - 场景 3：主件不就绪 → 起草被拒（「已提取出一部分需求」不构成放行理由）；
//! - 降级路径：逐件人工确认（record_addendum_waiver + #182 receipt）→
//!   放行**但**响应显式标受限（input_integrity.status=RESTRICTED +
//!   warnings 受限起草 + waived_files 逐件）——不得静默当完整输入；
//! - 正向：全部 READY → 正常起草，input_integrity.status=COMPLETE。
//!
//! 并行安全：全部经 `dispatch_with` 注入独立 `BidService`。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
        });
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

fn err_text(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// 造一个「MATRIX_READY + 建纲 + 素材齐」的服务态（draft_tool.rs
/// svc_ready 的 P0-1 变体基座：主件 Ok + 补遗 Ok，坏件由各场景自行追加）。
fn svc_ready(tag: &str) -> (BidService, String, String) {
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "P0-1 输入完整性测试", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": tag,
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", &format!("sha-p01-t-{tag}"), 10, 1, 50, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    svc.store_mut().upsert_file(
        "C-补遗01.docx", "补遗", &format!("sha-p01-a1-{tag}"), 5, 0, 20, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![bid_domain::Requirement {
        id: "REQ-0001".into(),
        category: "资格条件".into(),
        clause_no: None,
        anchors: vec![bid_domain::Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第二章/1.1".into(),
            excerpt: "投标人须具备电子与智能化工程专业承包二级及以上资质".into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }]).unwrap();
    let (n_sec, _, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 3_000, max_reqs_per_leaf: 2 })
        .unwrap();
    assert!(n_sec >= 1);
    // 素材面：资格条件须可用资质 + 矩阵响应（隔离 P0-1 面——其余 Blocker 不混入断言）
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: "电子与智能化工程专业承包二级资质证书".into(),
        source_file: "EV-p01.pdf".into(),
        source_sha256: format!("sha-p01-ev-{tag}"),
        locator: EngineLocator::DocxParagraph { paragraph_index: 3, text_fingerprint: "fnv64:p01".into() },
        fingerprint: "fnv64:p01".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某科技有限公司".into(),
        scope: "电子与智能化工程".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    svc.store_mut().bind_evidence(&eid, "REQ-0001", "2026-12-31").unwrap();
    let _ = svc.store_mut().build_matrix().unwrap();
    let parents: std::collections::HashSet<&str> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.as_deref()).collect();
    let leaf = svc.store().sections().iter()
        .find(|n| !parents.contains(&n.id[..])).unwrap().id.clone();
    (svc, pid, leaf)
}

fn leaf_of(svc: &BidService) -> String {
    let parents: std::collections::HashSet<&str> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.as_deref()).collect();
    svc.store().sections().iter()
        .find(|n| !parents.contains(&n.id[..])).unwrap().id.clone()
}

// ---------------------------------------------------------------- 场景 1：补遗 OCR_PENDING → 拒

#[test]
fn acceptance_addendum_ocr_pending_rejects_draft() {
    let (mut svc, pid, leaf) = svc_ready("p01-add-ocr");
    // 补遗B：零文本扫描件（无文本层且 OCR 未产出）
    svc.store_mut().upsert_file(
        "C-补遗B-扫描件.pdf", "补遗", "sha-p01-add-b-scan", 8, 0, 0, 0,
        bid_domain::ParseStatus::OcrRequired, "无文本层扫描件",
    ).unwrap();
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "instructions": "P0-1 场景1",
    }));
    let msg = err_text(&reply);
    assert!(msg.contains("C-补遗B-扫描件.pdf"), "拒稿理由要点名坏件：{msg}");
    assert!(msg.contains("OCR"), "要点明 OCR 未完成：{msg}");
}

// ---------------------------------------------------------------- 场景 2：附件解析失败 → 拒

#[test]
fn acceptance_failed_attachment_rejects_draft() {
    let (mut svc, pid, leaf) = svc_ready("p01-att-fail");
    // 附件C：解析失败（真实事故面：加密 PDF）
    svc.store_mut().upsert_file(
        "E-资格审查表-附件.pdf", "附件", "sha-p01-att-enc", 0, 0, 0, 0,
        bid_domain::ParseStatus::Failed, "加密文档：用户口令保护，内容不可读",
    ).unwrap();
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "instructions": "P0-1 场景2",
    }));
    let msg = err_text(&reply);
    assert!(msg.contains("E-资格审查表-附件.pdf"), "拒稿理由要点名坏附件：{msg}");
    assert!(msg.contains("不等于输入完整"), "要说清判据（不是「有产出就能写」）：{msg}");
}

// ---------------------------------------------------------------- 场景 3：主件不就绪 → 拒

#[test]
fn acceptance_unready_main_tender_rejects_draft() {
    let (mut svc, pid, _leaf) = svc_ready("p01-main-bad");
    // 主件追加一件坏形态（多份招标文件本有警告面——这里造主件不可读形态：
    // 用第二份「招标文件」类坏件模拟主件解析失败的事故面）
    let (fid_main, _) = svc.store_mut().upsert_file(
        "A-招标文件-正式版.pdf", "招标文件", "sha-p01-main-bad", 0, 0, 0, 0,
        bid_domain::ParseStatus::Failed, "pdf 两条路输出均为疑似 CJK 乱码（GBK-EUC-H 无映射）",
    ).unwrap();
    let leaf = leaf_of(&svc);
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "instructions": "P0-1 场景3",
    }));
    let msg = err_text(&reply);
    assert!(msg.contains("A-招标文件-正式版.pdf"), "拒稿理由要点名坏主件：{msg}");
    assert!(!fid_main.is_empty());
}

// ---------------------------------------------------------------- 降级路径：确认放行 → 受限起草

#[test]
fn waiver_yields_restricted_draft_not_silent_pass() {
    let (mut svc, pid, leaf) = svc_ready("p01-waiver");
    let (fid_att, _) = svc.store_mut().upsert_file(
        "E-资格审查表-附件.pdf", "附件", "sha-p01-waived-att", 0, 0, 0, 0,
        bid_domain::ParseStatus::Failed, "加密文档：用户口令保护，内容不可读",
    ).unwrap();
    // 确认前：拒
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "instructions": "P0-1 降级前",
    }));
    assert!(err_text(&reply).contains("E-资格审查表-附件.pdf"));

    // 逐件人工确认（#182 receipt——宿主签发，签域绑定 file_id）
    let receipt = svc.issue_approval_receipt("p01-lead", &format!("addendum_waiver:{fid_att}"));
    let w = ok_result(call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid_att,
        "reason": "已人工核对纸质原件：本表与招标文件第三章一致",
        "receipt": receipt,
    })));
    assert_eq!(w["recorded"]["file_id"], json!(&fid_att));

    // 确认后：放行——但必须显式标受限（三面：机器面/告警面/逐件面）
    let d = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "instructions": "P0-1 降级后",
    })));
    assert_eq!(
        d["input_integrity"]["status"], json!("RESTRICTED"),
        "机器面：受限起草必须显式标注，不得静默当完整输入：{d}"
    );
    let waived = d["input_integrity"]["waived_files"].as_array().unwrap();
    assert_eq!(waived.len(), 1, "逐件面：确认来源可见：{waived:?}");
    assert_eq!(waived[0]["file_id"], json!(&fid_att));
    assert_eq!(waived[0]["readiness"], json!("FAILED"));
    let warnings = d["warnings"].as_array().unwrap();
    let hit = warnings.iter().map(|w| w.as_str().unwrap_or(""))
        .find(|w| w.contains("受限起草"));
    let hit = hit.unwrap_or_else(|| panic!("告警面：受限起草要说人话：{warnings:?}"));
    assert!(hit.contains("E-资格审查表-附件.pdf"), "告警要点名件：{hit}");
    // 账面：draft_note 随行（正文渲染面不碰，审计面可查）
    let note = d["draft_note"].as_str().unwrap_or("");
    assert!(note.contains("受限起草"), "账面要留受限记录：{note}");
}

// ---------------------------------------------------------------- 正向：全部 READY → COMPLETE

#[test]
fn acceptance_all_ready_drafts_complete() {
    let (mut svc, pid, leaf) = svc_ready("p01-positive");
    // 再加一件正常附件（覆盖面：附件 Ok 不误伤）
    svc.store_mut().upsert_file(
        "E-附件-技术规范.pdf", "附件", "sha-p01-att-ok", 6, 1, 30, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    let d = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "instructions": "P0-1 正向",
    })));
    assert_eq!(
        d["input_integrity"]["status"], json!("COMPLETE"),
        "正向：完整输入标 COMPLETE：{d}"
    );
    assert_eq!(
        d["input_integrity"]["waived_files"].as_array().unwrap().len(), 0,
        "无确认放行件：{d}"
    );
    let warnings = d["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().all(|w| !w.as_str().unwrap_or("").contains("受限起草")),
        "完整输入不得带受限告警：{warnings:?}"
    );
}

// ---------------------------------------------------------------- import 响应接线（第一现场）

/// 坏附件经真件导入（截断 docx → R-01 登记 Failed）：import 响应（E_PARTIAL
/// 的 data）携带 input_integrity 块 + INPUT_FILE_NOT_READY 指路告警——
/// Agent 拿 file_id 即可闭环（重传或 record_addendum_waiver）。
#[test]
fn import_response_carries_input_integrity_block() {
    let word_complex = fixtures().join("engine-borrowed/TR2/word-complex.docx");
    if bid_parse::env_preflight::skip_if_fixture_missing(
        "import_response_carries_input_integrity_block",
        &[&word_complex],
    ) {
        return;
    }
    let root = std::env::temp_dir().join(format!("p01_uploads_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let wc_up = root.join("word-complex.docx");
    std::fs::copy(&word_complex, &wc_up).expect("好件拷入受信根");
    // 截断 docx = 确定性坏件（zip EOCD 缺失 → 防护拒——import_tender_files_tool 同款手法）
    let bad = root.join("E-坏附件.docx");
    let bytes = std::fs::read(&word_complex).expect("word-complex.docx 夹具缺失");
    std::fs::write(&bad, &bytes[..1200]).unwrap();
    let mut svc = BidService::new().with_fs_root(&root);
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "P0-1 导入接线", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": "p01-import",
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    let reply = call(&mut svc, "import_tender_files", json!({
        "project_id": &pid,
        "files": [
            { "upload_ref": wc_up.display().to_string(), "filename": "A-招标文件.docx", "kind": "招标文件" },
            { "upload_ref": bad.display().to_string(), "filename": "E-坏附件.docx", "kind": "附件" },
        ],
    }));
    let data = match &reply {
        ToolReply::Err { error, .. } => {
            assert_eq!(error.code(), "E_PARTIAL", "截断 docx 必须部分失败：{error}");
            serde_json::to_value(error).unwrap()["data"].clone()
        }
        ToolReply::Ok(o) => panic!("坏附件在批必须 E_PARTIAL，实际成功: {}", o.result),
    };
    // input_integrity 块：不完整 + 逐件点名坏附件（Agent 闭环的 file_id 在场）
    let ii = &data["input_integrity"];
    assert_eq!(ii["complete"], json!(false), "坏附件 → 不完整：{ii}");
    let unready = ii["unready"].as_array().unwrap();
    assert_eq!(unready.len(), 1, "只有坏附件不就绪：{ii}");
    assert_eq!(unready[0]["name"], json!("E-坏附件.docx"));
    assert_eq!(unready[0]["readiness"], json!("FAILED"));
    assert!(unready[0]["file_id"].as_str().unwrap().starts_with("TF-"), "file_id 在场");
    // 告警面：INPUT_FILE_NOT_READY 点名 + 指路（与 ADDENDUM_NOT_READY 同构）
    let warnings = data["warnings"].as_array().unwrap();
    let hit = warnings.iter().find_map(|w| {
        w.as_str()
            .and_then(|s| s.starts_with("INPUT_FILE_NOT_READY").then_some(s.to_string()))
    }).expect("必须有 INPUT_FILE_NOT_READY 告警");
    assert!(hit.contains("E-坏附件.docx"), "告警点名坏附件：{hit}");
    assert!(hit.contains("record_addendum_waiver"), "告警指路确认工具：{hit}");
    assert!(
        !warnings.iter().any(|w| w.as_str().unwrap_or("").starts_with("ADDENDUM_NOT_READY")),
        "附件不触发补遗告警（语义分面）：{warnings:?}"
    );
}
