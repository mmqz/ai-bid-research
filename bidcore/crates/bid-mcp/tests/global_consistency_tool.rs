//! #423 · check_bid_quality 全局一致性接线契约测试（S-5 run_review 的首个
//! 生产消费方）。
//!
//! 验收面（issue #423 验收标准 1/2/3）：
//! - 两章金额冲突（3500000 vs 3580000）→ `global_conflicts` 非空，含两处
//!   位置（SEC 双方）与归一化值；
//! - 无冲突 → `global_conflicts` 为**空数组**（字段不缺席——Agent 消费面
//!   契约：缺席 = 未知，空 = 已查无冲突）；
//! - 负偏离查询（A-07）挂同一响应面：negative_deviations 逐行可见；
//! - 工具面不变（不加新工具——本文件不注册任何新工具名，工具数断言在
//!   dispatch_contract.rs 由 tool-surface 派生面统一把守）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use bid_domain::outline::OutlineSpec;
use serde_json::{json, Value};

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    // 本套件不用 generator 注入（external 稿直注 content——与 generator 互斥）
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn mk_req(id: &str, excerpt: &str) -> bid_domain::Requirement {
    use bid_domain::{Anchor, Requirement};
    Requirement {
        id: id.into(),
        category: "商务要求".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第二章/{id}"),
            excerpt: excerpt.into(),
            block_idx: None,
            page: None,
            line: None,
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

/// 「MATRIX_READY + 建纲（每叶 1 REQ → 两叶）+ 证据绑定 + 补遗豁免」服务态。
fn svc_two_leaves(tag: &str) -> (BidService, String, String, String) {
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "全局一致性测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-g", 10, 1, 50, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "投标保证金须按招标文件要求缴纳"),
        mk_req("REQ-0002", "服务费按合同约定支付"),
    ]).unwrap();
    let (_, _, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 400, max_reqs_per_leaf: 1 })
        .unwrap();
    // 素材面（#110）：商务要求类 REQ 须有证据佐证，否则 draft 拒生成
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::ProductDoc,
        title: "缴费凭证".into(),
        source_file: "EV-glob.pdf".into(),
        source_sha256: "sha-g-tech".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 5, text_fingerprint: "fnv64:gl".into() },
        fingerprint: "fnv64:gl".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某公司".into(),
        scope: "商务承诺佐证".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    for rid in ["REQ-0001", "REQ-0002"] {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    // L3 无补遗阻塞的人工出口（与 check_quality_tool 同口径）
    use bid_domain::review::{Decision, Gate};
    svc.store_mut()
        .record_review(Gate::AddendumWaiver, &bid_domain::TrustedPrincipal::human("global-consistency-test"), Decision::Approved, vec![], None)
        .unwrap();
    let parents: std::collections::HashSet<&str> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.as_deref()).collect();
    let leaves: Vec<String> = svc.store().sections().iter()
        .filter(|n| !parents.contains(&n.id[..]))
        .map(|n| n.id.clone())
        .collect();
    assert!(leaves.len() >= 2, "每叶 1 REQ 应产出 ≥2 叶（实际 {}）", leaves.len());
    let (a, b) = (leaves[0].clone(), leaves[1].clone());
    (svc, pid, a, b)
}

/// 外部稿直注起草（content 在场即 external 语义——引擎不生成只提交）。
fn draft_external(svc: &mut BidService, pid: &str, sid: &str, content: &str) -> Value {
    ok_result(call(svc, "draft_response_section", json!({
        "project_id": pid, "section_id": sid, "content": content,
    })))
}

// ---------------------------------------------------------------- 测试

/// 验收 1：两章金额冲突（3500000 vs 3580000）→ global_conflicts 非空，
/// message 含两处位置（SEC 双方）与两个归一化值。
#[test]
fn two_chapter_amount_conflict_surfaces_in_global_conflicts() {
    let (mut svc, pid, leaf_a, leaf_b) = svc_two_leaves("g-conflict");
    draft_external(&mut svc, &pid, &leaf_a,
        "致招标人：我方认真研究了招标文件。本项目投标保证金为 3500000 元，由基本账户转出。");
    draft_external(&mut svc, &pid, &leaf_b,
        "费用说明：投标保证金金额为 3580000 元，详见附表。");
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": &pid,
    })));
    let gc = q["global_conflicts"].as_array()
        .expect("global_conflicts 字段必须在场（不缺席契约）");
    let amount: Vec<&Value> = gc.iter().filter(|f| f["kind"] == "amount_conflict").collect();
    assert_eq!(amount.len(), 1, "两章金额冲突必报一条: {gc:?}");
    let f = amount[0];
    assert_eq!(f["severity"], "blocker", "全局冲突是废标级: {f:?}");
    let msg = f["message"].as_str().unwrap();
    assert!(msg.contains(&leaf_a) && msg.contains(&leaf_b), "两处位置都必须在 message: {msg}");
    assert!(msg.contains("3500000") && msg.contains("3580000"), "两个归一化值: {msg}");
    assert!(msg.contains("投标保证金"), "维度键可读: {msg}");
    // 锚点 = 少数派取值首现位（两值各一章 → 位置数 1:1 平局取字典序）
    let anchor = f["section_id"].as_str().unwrap();
    assert!(anchor == leaf_a || anchor == leaf_b, "锚点必为其中一章: {anchor}");
}

/// 验收 2：无冲突 → global_conflicts 为空数组（字段在场——缺席 ≠ 空）。
#[test]
fn no_conflict_yields_empty_array_not_missing_field() {
    let (mut svc, pid, leaf_a, leaf_b) = svc_two_leaves("g-clean");
    draft_external(&mut svc, &pid, &leaf_a,
        "致招标人：本项目投标保证金为 3500000 元，由基本账户转出。");
    draft_external(&mut svc, &pid, &leaf_b,
        // 单窗口词（「7x24 全天候」是同位语——check.rs 刻意不归并两词面，
        // 同句双词会被判承诺冲突，此处测的是「不同维度无冲突」不是词表口径）
        "服务承诺：我方提供 7x24 服务，响应时间为 4 小时。");
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": &pid,
    })));
    let gc = q["global_conflicts"].as_array()
        .expect("无冲突也是空数组——字段不缺席（缺席 = 未知，空 = 已查无冲突）");
    assert!(gc.is_empty(), "不同维度无冲突: {gc:?}");
    let nd = q["negative_deviations"].as_array()
        .expect("negative_deviations 字段必须在场");
    assert!(nd.is_empty(), "无参数偏离: {nd:?}");
}

/// 参数账接线（「含参数响应账」）：章正文 3500000 vs 参数行 3580000 →
/// 跨面冲突，位置锚给出 SEC 与 PR 两面（修复回路直达参数行）。
#[test]
fn param_ledger_conflict_anchors_pr_row() {
    let (mut svc, pid, leaf_a, leaf_b) = svc_two_leaves("g-ledger");
    draft_external(&mut svc, &pid, &leaf_a,
        "致招标人：本项目投标保证金为 3500000 元。");
    draft_external(&mut svc, &pid, &leaf_b, "服务承诺：详见技术方案。");
    // 参数账：投标保证金响应 3580000 元（与章正文冲突）
    use bid_domain::param::ParamInput;
    svc.store_mut().set_param_responses(vec![ParamInput {
        requirement_id: "REQ-0001".into(),
        param_name: "投标保证金".into(),
        required_value: "3500000元".into(),
        response_value: "3580000元".into(),
        evidence_id: None,
    }]).unwrap();
    let pr_id = svc.store().param_responses()[0].id.clone();
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": &pid,
    })));
    let gc = q["global_conflicts"].as_array().unwrap();
    let amount: Vec<&Value> = gc.iter().filter(|f| f["kind"] == "amount_conflict").collect();
    assert_eq!(amount.len(), 1, "章×参数账冲突必报: {gc:?}");
    let msg = amount[0]["message"].as_str().unwrap();
    assert!(msg.contains(&leaf_a) && msg.contains(&pr_id), "SEC 与 PR 两面位置: {msg}");
    assert!(msg.contains("3500000") && msg.contains("3580000"), "两侧值: {msg}");
}

/// A-07 负偏离查询挂同一响应面：数值负偏离行（2 年 < 不低于 3 年）逐行可见。
#[test]
fn negative_deviations_surface_param_rows() {
    let (mut svc, pid, leaf_a, leaf_b) = svc_two_leaves("g-negdev");
    draft_external(&mut svc, &pid, &leaf_a, "致招标人：我方郑重承诺。");
    draft_external(&mut svc, &pid, &leaf_b, "质保方案详见后文。");
    use bid_domain::param::ParamInput;
    svc.store_mut().set_param_responses(vec![
        ParamInput {
            requirement_id: "REQ-0001".into(),
            param_name: "质保期".into(),
            required_value: "不低于 3 年".into(),
            response_value: "2年".into(),
            evidence_id: None,
        },
        ParamInput {
            requirement_id: "REQ-0002".into(),
            param_name: "服务费".into(),
            required_value: "1000000元".into(),
            response_value: "1000000元".into(),
            evidence_id: None,
        },
    ]).unwrap();
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({
        "project_id": &pid,
    })));
    let nd = q["negative_deviations"].as_array().unwrap();
    assert_eq!(nd.len(), 1, "只报负偏离行（无偏离行不进清单）: {nd:?}");
    let row = &nd[0];
    assert_eq!(row["param_name"], "质保期");
    assert_eq!(row["required_value"], "不低于 3 年");
    assert_eq!(row["response_value"], "2年");
    assert_eq!(row["requirement_id"], "REQ-0001");
    assert!(row["param_id"].as_str().unwrap().starts_with("PR-"), "param_id 契约: {row:?}");
    // 全局面：参数账两行（质保期非词表维度、服务费单值）不产冲突
    let gc = q["global_conflicts"].as_array().unwrap();
    assert!(gc.iter().all(|f| f["kind"] != "amount_conflict"), "无金额冲突面: {gc:?}");
}
