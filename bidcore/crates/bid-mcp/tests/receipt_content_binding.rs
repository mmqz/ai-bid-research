//! receipt_content_binding.rs —— #323/R-06：receipt 内容绑定（MCP 工具层）。
//!
//! issue #323 验收对照：
//! 1. **主线负例**（验收 2）：宿主签发 receipt → 章节内容变更 → 旧 receipt
//!    过同一门 → `CONTENT_DRIFTED_RECEIPT` 拒（「批的是旧版内容」审计失真
//!    面收口）；重签 v2（绑新摘要）→ 放行；
//! 2. **无关写不误报**：导入无关件 / 记审等不动章节树的写（content_version
//!    照常 bump）不改 sections_digest → receipt 仍有效（#180 口径对称面）；
//! 3. **v1 fail-closed**：旧六段 receipt → `LEGACY_V1_RECEIPT`（迁移裁定）；
//! 4. **读面**：`get_bid_project_state` 回 `content_binding.sections_digest`
//!    （宿主签发 v2 前的取值真源）。
//!
//! 夹具纪律：走真实状态机（全稿后先矩阵门后大纲门——门序见 review.rs），
//! 内容变更走域层 `set_section_content`（与 MCP 起草同归宿）。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, ParseStatus, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};

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

fn mk_req(id: &str, category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第二章/{id}"),
            excerpt: excerpt.into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-323-mcp-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 建项目：两叶 + 矩阵；全稿起草完成（过门状态窗在 DRAFTING 内）。
fn fully_drafted_project(tag: &str) -> (BidService, String) {
    let dir = tmp_dir(tag);
    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "323 内容绑定", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": &format!("ext-323-mcp-{tag}"),
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut()
        .upsert_file("A-招标文件.docx", "招标文件", &format!("sha-323-{tag}"), 10, 1, 50, 0, ParseStatus::Ok, "")
        .unwrap();
    svc.store_mut()
        .upsert_file("C-补遗文件01.docx", "补遗", &format!("sha-323-{tag}-c"), 5, 0, 20, 0, ParseStatus::Ok, "")
        .unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "具备水利水电工程施工总承包贰级资质"),
        mk_req("REQ-0002", "格式要求", "正文用仿宋三号字"),
    ])
    .unwrap();
    let mk_leaf = |id: &str, title: &str, order: u32, reqs: &[&str]| OutlineNode {
        id: id.into(), parent_id: None, title: title.into(), level: 1, order,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars: 800, status: SectionStatus::Pending, dirty: false,
        content: None, content_attempt: 0,
    };
    svc.store_mut().set_outline(vec![
        mk_leaf("SEC-0001", "资格响应", 1, &["REQ-0001"]),
        mk_leaf("SEC-0002", "格式响应", 2, &["REQ-0002"]),
    ]).unwrap();
    svc.store_mut().build_matrix().unwrap();
    // 资格证据登记（起草门：无证硬写 = 虚假响应——与 243 夹具同款）
    let _ = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: "水利水电工程施工总承包贰级资质证书".into(),
        source_file: "EV-ledger.pdf".into(),
        source_sha256: format!("sha-323-{tag}-lg"),
        locator: EngineLocator::DocxParagraph { paragraph_index: 7, text_fingerprint: "fnv64:lg".into() },
        fingerprint: "fnv64:lg".into(),
        valid_from: None, valid_until: None,
        subject_name: "某公司".into(),
        scope: "水利水电".into(),
        client: None, amount_wan: None, completed_on: None,
        fields: Default::default(), excerpt: None, excerpt_locator: None,
        verification_tier: None,
    });
    // 全稿（mock 起草——内容非空即可）
    for sid in ["SEC-0001", "SEC-0002"] {
        ok_result(call(&mut svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": sid,
            "instructions": "按招标文件逐项响应",
        })));
    }
    (svc, pid)
}

fn record_gate(svc: &mut BidService, pid: &str, gate: &str, receipt: &str) -> ToolReply {
    call(svc, "record_review_gate", json!({
        "project_id": pid, "gate": gate, "decision": "APPROVED",
        "receipt": receipt,
    }))
}

/// MCP 只读面的内容绑定摘要（#323——宿主签发 v2 receipt 的取值真源）。
fn state_digest(svc: &mut BidService) -> String {
    let st = ok_result(call(svc, "get_bid_project_state", json!({})));
    st["content_binding"]["sections_digest"]
        .as_str()
        .expect("get_bid_project_state 回 content_binding.sections_digest（#323）")
        .to_string()
}

// ---------------------------------------------------------------- 1. 主线负例

#[test]
fn content_drift_rejects_old_receipt_then_reissue_passes() {
    let (mut svc, pid) = fully_drafted_project("drift");

    // 矩阵门先行（门序）——receipt 绑当前摘要 D1
    let matrix_receipt = svc.issue_approval_receipt("323-matrix", "review_gate:matrix_approval");
    let _m = ok_result(record_gate(&mut svc, &pid, "matrix_approval", &matrix_receipt));

    // 宿主签发大纲门 receipt（摘要 D1）
    let d1 = state_digest(&mut svc);
    assert_eq!(d1, svc.receipt_binding_digest(), "MCP 读面与宿主读面同源");
    let stale_receipt = svc.issue_approval_receipt("323-outline", "review_gate:outline_approval");

    // 签发后章节内容变更 → 摘要 D2（sections_digest 含 content——#180 编码契约）
    svc.store_mut()
        .set_section_content("SEC-0002", "格式响应（修订稿）：全文仿宋三号字，行距 28 磅。".into())
        .unwrap();
    let d2 = state_digest(&mut svc);
    assert_ne!(d1, d2, "夹具：内容变更必须改变 sections_digest");
    assert!(
        svc.store().content_version() > 0,
        "夹具：content_version 亦 bump（无关面——不影响摘要口径）"
    );

    // 旧 receipt 过同一门 → CONTENT_DRIFTED_RECEIPT（R-06 收口）
    let reply = record_gate(&mut svc, &pid, "outline_approval", &stale_receipt);
    let msg = err_text(&reply);
    assert!(
        msg.contains("CONTENT_DRIFTED_RECEIPT"),
        "旧 receipt 须报内容漂移分类码：{msg}"
    );

    // 重签（绑 D2）→ 放行
    let fresh_receipt = svc.issue_approval_receipt("323-outline", "review_gate:outline_approval");
    let _g = ok_result(record_gate(&mut svc, &pid, "outline_approval", &fresh_receipt));
}

// ---------------------------------------------------------------- 2. 无关写不误报

#[test]
fn unrelated_write_does_not_invalidate_receipt() {
    let (mut svc, pid) = fully_drafted_project("unrelated");

    let matrix_receipt = svc.issue_approval_receipt("323-matrix", "review_gate:matrix_approval");
    let _m = ok_result(record_gate(&mut svc, &pid, "matrix_approval", &matrix_receipt));

    // 大纲门 receipt（摘要 D1）
    let outline_receipt = svc.issue_approval_receipt("323-outline", "review_gate:outline_approval");
    let d1 = state_digest(&mut svc);

    // 无关写：补遗确认（真实流常见——bump content_version、写审计账，
    // 章节树不动；lib.rs record_addendum_waiver 的版本推进面）。
    let (fid, _) = svc
        .store_mut()
        .upsert_file("C-补遗文件01.docx", "补遗", "sha-323-unrelated", 5, 0, 20, 0, ParseStatus::Ok, "")
        .unwrap();
    let v_before = svc.store().content_version();
    let waiver_receipt = svc.issue_approval_receipt("323-lead", &format!("addendum_waiver:{fid}"));
    let _ = svc
        .record_addendum_waiver(
            &fid,
            "已人工核对：本补遗仅重申开标地点，条款无实质变化",
            &waiver_receipt,
        )
        .expect("补遗确认（无关写）成功");
    assert!(
        svc.store().content_version() > v_before,
        "夹具：无关写 bump content_version（版本面与摘要面解耦的对照）"
    );
    assert_eq!(state_digest(&mut svc), d1, "无关写不改 sections_digest（不误报）");

    // 原 receipt 仍有效（#180 口径：绑摘要不绑全局版本）
    let _g = ok_result(record_gate(&mut svc, &pid, "outline_approval", &outline_receipt));
}

// ---------------------------------------------------------------- 3. v1 fail-closed

#[test]
fn legacy_v1_receipt_is_rejected_at_gate() {
    let (mut svc, pid) = fully_drafted_project("legacy");

    let matrix_receipt = svc.issue_approval_receipt("323-matrix", "review_gate:matrix_approval");
    let _m = ok_result(record_gate(&mut svc, &pid, "matrix_approval", &matrix_receipt));

    // 手搓 v1 六段（结构合法的旧格式）→ LEGACY_V1_RECEIPT（fail-closed 迁移）
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let v1 = format!(
        "v1.323-lead.human.review_gate:outline_approval.{now}.{}",
        "ab".repeat(32)
    );
    let reply = record_gate(&mut svc, &pid, "outline_approval", &v1);
    let msg = err_text(&reply);
    assert!(
        msg.contains("LEGACY_V1_RECEIPT"),
        "v1 旧格式须报迁移分类码（fail-closed）：{msg}"
    );
}
