//! evidence_readiness_gate.rs —— #241 裁定端到端：正文响应与证据缺口两分账
//! （validate / check_bid_quality / get_compliance_matrix 三面契约接线）。
//!
//! 裁定（Lead 2026-11-05，issue #241）：
//! - 正文已响应但无外部证据 → 矩阵 RESPONDED（文本事实）；
//! - 该行不得进入「可提交」状态（合规事实——submission_readiness 阻断）；
//! - 导出：不阻断（草稿可导出——#253 分离面）。
//!
//! 验收映射：
//! 1+2. 正文有响应、无证据 → RESPONDED + evidence_missing 清单 + ready=false
//!      + 导出门放行（三工具响应面 + 域门同场断言）；
//! 3.   补充证据后缺口翻转 → ready=true（validate 与 check_bid_quality 双面）。
//!
//! 夹具纪律：域状态走真实服务调用（create/import/建纲联建矩阵/登记/起草），
//! 正文经 MCP draft_response_section（mock 生成器）落位。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineSpec;
use bid_domain::ParseStatus;
use bid_domain::{Anchor, Requirement};
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
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-241-gate-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 材料型资格条款夹具：REQ-0001（资格条件，原文含「提供…扫描件」要求词）
/// → 建纲联建矩阵 → 登记台账资质（不绑）→ mock 起草其叶（正文实写）。
///
/// 返回 (svc, pid, leaf_id)。
fn gated_project(tag: &str) -> (BidService, String, String) {
    let mut svc = BidService::open(tmp_dir(tag)).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "241 提交就绪门禁", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": tag,
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    svc.store_mut()
        .upsert_file("A-招标文件.docx", "招标文件", &format!("sha-241-{tag}"), 10, 1, 50, 0, ParseStatus::Ok, "")
        .unwrap();
    svc.store_mut()
        .upsert_file("C-补遗文件01.docx", "补遗", &format!("sha-241-add-{tag}"), 5, 0, 20, 0, ParseStatus::Ok, "")
        .unwrap();
    svc.set_requirements(vec![mk_req(
        "REQ-0001",
        "资格条件",
        "投标人须提供营业执照副本及资质证书扫描件",
    )])
    .unwrap();
    let _ = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 2_000, max_reqs_per_leaf: 8 })
        .unwrap();
    // 台账资质（登记不绑——readiness 的 MissingQualification 闸放行；
    // 不绑 = 保留证据缺口，供本套件断言两分账）
    svc.store_mut()
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "营业执照副本及资质证书".into(),
            source_file: "EV-241.pdf".into(),
            source_sha256: format!("sha-241-ev-{tag}"),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 3,
                text_fingerprint: "fnv64:241".into(),
            },
            fingerprint: "fnv64:241".into(),
            valid_from: None,
            valid_until: None,
            subject_name: "某建设工程有限公司".into(),
            scope: "水利水电".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: Default::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        })
        .unwrap();
    // mock 起草资格叶（正文实写——#241 裁定后该行翻 RESPONDED）。
    // 父节点聚合子并集也含该 REQ——取无子的最深层叶（leaf_of_req 同口径）。
    let parents: std::collections::HashSet<String> = svc
        .store()
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.clone())
        .collect();
    let leaf = svc
        .store()
        .sections()
        .iter()
        .filter(|n| !parents.contains(&n.id))
        .find(|n| n.requirement_ids.iter().any(|r| r == "REQ-0001"))
        .map(|n| n.id.clone())
        .expect("REQ-0001 必须落在叶章节里（build_outline 全量分组）");
    ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": pid, "section_id": leaf, "generator": "mock",
    })));
    (svc, pid, leaf)
}

/// 验收 1+2：正文有响应、无证据 → 三面同场——validate 的 ok=true（正文
/// 响应不冤枉）+ submission_readiness.ready=false（缺口阻断可提交）+
/// 矩阵行 RESPONDED/缺口标记 + 导出门放行（两分账）。
#[test]
fn body_responded_gap_surfaces_on_all_three_tool_faces() {
    let (mut svc, pid, _leaf) = gated_project("three-faces");

    // ---- 面 1：validate_bid_document——ok=true（无 E_QUAL_UNMET）但未就绪
    let v = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": pid,
    })));
    assert_eq!(v["ok"], true, "正文实写 → 资格行 RESPONDED，ok 不再被 E_QUAL_UNMET 打断");
    assert!(
        !v["findings"].as_array().unwrap().iter().any(|f| f["rule_code"] == "E_QUAL_UNMET"),
        "正文响应的资格行不再报未响应（裁定主面）"
    );
    let sr = &v["submission_readiness"];
    assert_eq!(sr["ready"], false, "证据缺口的必选条款不得进入可提交状态");
    assert_eq!(sr["not_built"], false);
    assert_eq!(sr["evidence_missing_count"], 1);
    assert_eq!(sr["unresponded_count"], 0, "正文已响应——未响应清单为空");
    let gap = &sr["evidence_missing"][0];
    assert_eq!(gap["requirement_id"], "REQ-0001");
    assert_eq!(gap["row_id"], "CMP-0001", "行 id 与合规矩阵对账");
    assert_eq!(gap["category"], "资格条件");
    assert!(gap["excerpt"].as_str().unwrap().contains("营业执照"));
    assert!(gap["message"].as_str().unwrap().contains("bind_evidence"), "理由含解决出口");

    // ---- 面 2：get_compliance_matrix——行级正交标记
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 50,
    })));
    let row = &m["rows"][0];
    assert_eq!(row["requirement"]["id"], "REQ-0001");
    assert_eq!(row["status"], "RESPONDED", "正文实写 → 矩阵 RESPONDED（文本事实）");
    assert_eq!(row["evidence_required"], true, "词面命中（提供/扫描件）");
    assert_eq!(row["evidence_missing"], true, "无有效证据在绑 → 缺口为真（合规事实）");
    assert_eq!(row["blocking"], false, "正文响应的必选行不阻断导出");

    // ---- 面 3：check_bid_quality——同一 submission_readiness 面
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": pid,
    })));
    let qsr = &q["submission_readiness"];
    assert_eq!(qsr["ready"], false, "质量检查面同场回显提交就绪");
    assert_eq!(qsr["evidence_missing_count"], 1);
    assert_eq!(qsr["evidence_missing"][0]["requirement_id"], "REQ-0001");

    // ---- 域门：导出矩阵门放行（草稿可导出——#241 裁定门禁分工）
    assert!(svc.store().check_export_ready().is_ok(), "证据缺口不拦导出（两分账）");
}

/// 验收 3：补绑证据 → 缺口翻转 → submission_readiness.ready=true
/// （validate 与 check_bid_quality 双面）。
///
/// 状态机时序：validate 成功会推进 VALIDATING（禁 bind_evidence），故
/// 「翻转前」读数走只读的 check_bid_quality（不推状态），绑定后再双面复核。
#[test]
fn evidence_binding_flips_ready_on_both_tool_faces() {
    let (mut svc, pid, _leaf) = gated_project("flip-ready");

    // 翻转前（只读面——不推进状态，保留绑定期）：缺口在场
    let q0 = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": pid,
    })));
    assert_eq!(q0["submission_readiness"]["ready"], false, "绑定前缺口在场");
    assert_eq!(q0["submission_readiness"]["evidence_missing_count"], 1);

    // 补绑证据（登记过的台账件 → bind_evidence 批量面：ev_id + requirement_ids）
    let eid = svc.store().evidences()[0].id.clone();
    let out = ok_result(call(&mut svc, "bind_evidence", json!({
        "project_id": pid, "ev_id": eid, "requirement_ids": ["REQ-0001"],
        "bid_open_date": "2026-12-31",
    })));
    assert_eq!(out["matrix"]["blocking"].as_u64().unwrap(), 0, "绑定后矩阵 blocking 清零");

    // check_bid_quality 面：就绪翻转
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": pid,
    })));
    assert_eq!(q["submission_readiness"]["ready"], true, "补绑有效证据 → 缺口翻转、可提交（验收 3）");
    assert_eq!(q["submission_readiness"]["evidence_missing_count"], 0);
    assert_eq!(q["submission_readiness"]["blockers"], 0);

    // validate 面：同一翻转（ok=true + ready=true 同场——成功推进 VALIDATING）
    let v = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": pid,
    })));
    assert_eq!(v["ok"], true);
    assert_eq!(v["submission_readiness"]["ready"], true);
    assert_eq!(v["submission_readiness"]["evidence_missing_count"], 0);

    // 矩阵行：缺口标记同步翻转（单一真源）
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page_size": 50,
    })));
    assert_eq!(m["rows"][0]["evidence_missing"], false, "行级标记同步翻转");
    assert_eq!(m["rows"][0]["status"], "RESPONDED");
}
