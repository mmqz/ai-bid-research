//! #312 篇幅产能墙 · 写作时刻反馈回路 + 建纲配额面（bid-mcp tools.rs）。
//!
//! 诊断背景（黑盒实测 20,309/160,000=12.7%）：draft/append 返回体有数字
//! 但零警告——引擎在任何字数下都推 DRAFTED 且 warnings=[]，等于给「这叶
//! 完成了」的错误确认；E_CHARS_BELOW_MIN 要到 validate/export（管道末端）
//! 才出现。本组测试钉死三面：
//!
//! 1. **draft 反馈**：external 稿低于 80% 线 → warnings 主动告警（评分叶
//!    E 级口径 / 行政实质叶 W 级口径——与 validate 规则 4 同源同阈值）+
//!    quota.min_chars / quota.below_min 字段；达标稿零篇幅告警；
//! 2. **append 反馈**：合稿低于 80% 线 → 同款告警 + min_chars/below_min；
//!    补写达标后 below_min 翻 false；幂等重放（changed=false）不重复告警；
//! 3. **build_outline 配额面**：leaf_quotas 全量返回（section_id/title/
//!    target_chars，建纲即可规划批次）+ warnings 字段在场。
//!
//! 评分叶的构造说明：注入式 REQ（无评分表锚）不产生评分树章节——评分叶
//! 由真实评分表解析面（ScoringBuild）派生。单测用 `upsert_scoring_mapping`
//! 的映射信号（is_scoring_leaf 的第二判定面）把一个 legacy 叶升级为评分
//! 承载叶——分类面与 validate 完全同源，不另造判定。「配额回退」警告的
//! 真值面（评分叶底价 10k）依赖真实评分表，由真件探针验证（见 PR 描述
//! ——wenming 160k < 19 评分叶×10k 实测触发），不在合成注入面重复。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut()
            .unwrap()
            .insert("generator".into(), json!("mock"));
    }
    dispatch_with(
        svc,
        ToolCall {
            tool: tool.to_string(),
            args,
        },
    )
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn mk_req(id: &str, category: &str, excerpt: &str) -> bid_domain::Requirement {
    use bid_domain::{Anchor, Requirement};
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

/// #312 测试基座：三 REQ（资格/否决/评分）+ 20,000 字纲（服务面建纲建矩阵，
/// 走到 MATRIX_READY——draft/append 的合法窗口）。
///
/// 评分叶构造：outline 建好后把 REQ-0003（评分项）映射到「技术承载叶」
/// （upsert_scoring_mapping share=1.0）——is_scoring_leaf 的映射信号命中，
/// 该叶按评分叶的 E 级口径告警（与 validate 同一判定函数）。
fn svc_312(tag: &str) -> (BidService, String) {
    let mut svc = BidService::new();
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": "312 反馈回路测试", "bid_deadline": "2026-12-31", "mode": "明标",
            "external_key": tag,
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    svc.store_mut()
        .upsert_file(
            "A-招标文件.docx",
            "招标文件",
            "sha-312",
            10,
            1,
            50,
            0,
            bid_domain::ParseStatus::Ok,
            "",
        )
        .unwrap();
    // 补遗件在场：IncompleteTender 只警告不阻塞（与 draft_tool 测试同款）
    svc.store_mut()
        .upsert_file(
            "C-补遗文件01.docx",
            "补遗",
            "sha-312-add",
            5,
            0,
            20,
            0,
            bid_domain::ParseStatus::Ok,
            "",
        )
        .unwrap();
    svc.set_requirements(vec![
        mk_req(
            "REQ-0001",
            "资格条件",
            "投标人须具备水利水电工程施工总承包贰级及以上资质",
        ),
        mk_req(
            "REQ-0002",
            "否决项",
            "投标文件未经法定代表人签字的作否决处理",
        ),
        mk_req("REQ-0003", "评分项", "施工组织设计完整性 10 分"),
    ])
    .unwrap();
    let _ = svc
        .prepare_outline_and_matrix(OutlineSpec {
            target_total_chars: 20_000,
            max_reqs_per_leaf: 2,
        })
        .unwrap();
    // 素材面：资格/否决过 readiness 闸（评分叶不要求证据）
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc
        .store_mut()
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "水利水电工程施工总承包贰级资质证书".into(),
            source_file: "EV-312.pdf".into(),
            source_sha256: "sha-312-ev".into(),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 3,
                text_fingerprint: "fnv64:se".into(),
            },
            fingerprint: "fnv64:se".into(),
            valid_from: None,
            valid_until: None,
            subject_name: "某建设工程有限公司".into(),
            scope: "水利水电工程施工".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: Default::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        })
        .unwrap();
    for rid in ["REQ-0001", "REQ-0002"] {
        svc.store_mut()
            .bind_evidence(&eid, rid, "2026-12-31")
            .unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    (svc, pid)
}

/// 评分承载叶：把 REQ-0003 映射到绑定 REQ-0002 的叶（否决响应章——
/// 映射后 is_scoring_leaf 命中）。返回 (section_id, target_chars)。
fn scoring_leaf(svc: &mut BidService) -> (String, u32) {
    let (leaf, target) = leaf_of_rid(svc, "REQ-0002");
    svc.store_mut()
        .upsert_scoring_mapping(
            "REQ-0003".into(),
            leaf.clone(),
            1.0,
            vec![],
            bid_domain::Anchor {
                file: "A-招标文件.docx".into(),
                kind: "para".into(),
                locator: "第三章/评标".into(),
                excerpt: "施工组织设计完整性 10 分".into(),
            },
            bid_domain::scoring::MappingSource::Manual,
        )
        .expect("映射登记（REQ-0003 是评分项、leaf 在场——两前置均满足）");
    (leaf, target)
}

/// 找「绑定指定 REQ 的叶」（requirement_ids 含 rid 的叶节点）。
fn leaf_of_rid(svc: &BidService, rid: &str) -> (String, u32) {
    let parents: std::collections::HashSet<&str> = svc
        .store()
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.as_deref())
        .collect();
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| !parents.contains(&n.id[..]) && n.requirement_ids.iter().any(|r| r == rid))
        .unwrap_or_else(|| panic!("{rid} 未落在任何叶上"));
    (node.id.clone(), node.target_chars)
}

fn warns_contain(out: &Value, needle: &str) -> bool {
    out["warnings"]
        .as_array()
        .map(|ws| {
            ws.iter()
                .any(|w| w.as_str().map(|s| s.contains(needle)).unwrap_or(false))
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------- 1. draft 反馈

/// external 小稿（< 80% 线）→ 评分承载叶 E 级告警 + below_min/min_chars 字段。
#[test]
fn draft_below_min_warns_scoring_leaf() {
    let (mut svc, pid) = svc_312("below-scoring");
    let (leaf, target) = scoring_leaf(&mut svc);
    assert!(target > 0, "评分承载叶须有配额（实际 {target}）——测试前提");

    let tiny = "施工组织设计响应。".repeat(20); // ~180 字 << 80% 线
    let out = ok_result(call(
        &mut svc,
        "draft_response_section",
        json!({
            "project_id": pid, "section_id": leaf,
            "generator": "external", "content": tiny,
        }),
    ));

    let quota = &out["quota"];
    assert_eq!(quota["target_chars"].as_u64(), Some(u64::from(target)));
    assert_eq!(quota["below_min"], json!(true), "小稿必须标 below_min=true");
    let min_chars = quota["min_chars"].as_u64().expect("min_chars 在场");
    assert_eq!(
        min_chars,
        (f64::from(target) * 0.80).ceil() as u64,
        "80% 线与 validate 同口径"
    );
    assert!(
        warns_contain(&out, "篇幅不足（评分叶）"),
        "评分承载叶告警必须点名分轨，实际 warnings: {}",
        out["warnings"]
    );
    assert!(
        warns_contain(&out, "E_CHARS_BELOW_MIN"),
        "评分承载叶必须预告 BLOCKER 口径（与 validate 一致），实际: {}",
        out["warnings"]
    );
    // 小稿提示（<1000 字）
    assert!(
        warns_contain(&out, "碎片小稿"),
        "小稿提示应在场: {}",
        out["warnings"]
    );
    // 状态机语义不变：仍推 DRAFTED（本 PR 只加警告不改状态）
    assert_eq!(out["section_status"], json!("DRAFTED"));
}

/// 行政/实质叶同款低稿 → W 级口径（MAJOR 不拦导出——不吓唬也不漏报）。
#[test]
fn draft_below_min_warns_non_scoring_leaf() {
    let (mut svc, pid) = svc_312("below-admin");
    let (leaf, target) = leaf_of_rid(&svc, "REQ-0001");
    assert!(target > 0, "行政叶须有配额");

    let tiny = "资质响应。".repeat(10); // ~50 字
    let out = ok_result(call(
        &mut svc,
        "draft_response_section",
        json!({
            "project_id": pid, "section_id": leaf,
            "generator": "external", "content": tiny,
        }),
    ));
    assert_eq!(out["quota"]["below_min"], json!(true));
    assert!(
        warns_contain(&out, "篇幅不足（行政/实质叶）"),
        "非评分叶告警必须分轨，实际: {}",
        out["warnings"]
    );
    assert!(
        warns_contain(&out, "W_CHARS_BELOW_MIN"),
        "非评分叶必须预告 MAJOR 口径，实际: {}",
        out["warnings"]
    );
    assert!(
        !warns_contain(&out, "E_CHARS_BELOW_MIN"),
        "非评分叶不得用评分叶的 BLOCKER 话术吓唬: {}",
        out["warnings"]
    );
}

/// 达标稿（≥ 80% 线）→ 零篇幅告警（警告面不制造噪音——否则 Agent 学会忽略）。
#[test]
fn draft_at_min_no_length_warning() {
    let (mut svc, pid) = svc_312("at-min");
    let (leaf, target) = leaf_of_rid(&svc, "REQ-0001");
    let need = (f64::from(target) * 0.80).ceil() as usize + 10;
    let body: String = "资质响应材料齐备。"
        .repeat(need / 8 + 2)
        .chars()
        .take(need)
        .collect();

    let out = ok_result(call(
        &mut svc,
        "draft_response_section",
        json!({
            "project_id": pid, "section_id": leaf,
            "generator": "external", "content": body,
        }),
    ));
    assert_ne!(
        out["quota"]["below_min"],
        json!(true),
        "达标稿 below_min 必须为 false"
    );
    assert!(
        !warns_contain(&out, "篇幅不足"),
        "达标稿不得有篇幅告警（噪音会训练 Agent 忽略警告），实际: {}",
        out["warnings"]
    );
}

// ---------------------------------------------------------------- 2. append 反馈

/// append 小批 → 合稿低于 80% 线告警；继续补到达标 → below_min 翻 false。
#[test]
fn append_below_min_then_top_up() {
    let (mut svc, pid) = svc_312("append-feedback");
    let (leaf, target) = scoring_leaf(&mut svc);
    let min_chars = (f64::from(target) * 0.80).ceil() as u64;

    // 批 1：小批（远低于线）
    let out1 = ok_result(call(
        &mut svc,
        "append_section_segment",
        json!({
            "project_id": pid, "section_id": leaf,
            "content": "施工组织设计方案首批。".repeat(20),
            "idem_key": format!("{leaf}/b1"),
        }),
    ));
    assert_eq!(out1["below_min"], json!(true), "批 1 后合稿应 below_min");
    assert_eq!(out1["min_chars"].as_u64(), Some(min_chars));
    assert!(
        warns_contain(&out1, "篇幅不足（评分叶）"),
        "append 也必须告警（黑盒实测 19 次 append 全程零告警），实际: {}",
        out1["warnings"]
    );

    // 批 2..N：补到达标（模拟真实续写节奏）
    let mut assembled = out1["assembled_chars"].as_u64().unwrap();
    let mut batch = 2u32;
    while assembled < min_chars {
        let seg = "分批续写正文内容，按评分细则逐项展开响应要点。".repeat(100);
        let out = ok_result(call(
            &mut svc,
            "append_section_segment",
            json!({
                "project_id": pid, "section_id": leaf,
                "content": seg, "idem_key": format!("{leaf}/b{batch}"),
            }),
        ));
        assembled = out["assembled_chars"].as_u64().unwrap();
        if assembled >= min_chars {
            assert_ne!(
                out["below_min"],
                json!(true),
                "达标批后 below_min 必须翻 false"
            );
            assert!(
                !warns_contain(&out, "篇幅不足"),
                "达标后不得继续告警，实际: {}",
                out["warnings"]
            );
        }
        batch += 1;
        assert!(batch < 30, "补写循环失控（target={target}）");
    }
}

/// 幂等重放（同 idem_key 同内容）不重复告警——重放者不是在写作。
#[test]
fn append_idempotent_replay_no_warning_duplication() {
    let (mut svc, pid) = svc_312("append-replay");
    let (leaf, _target) = scoring_leaf(&mut svc);
    let seg = "幂等重放测试段。".repeat(10);

    let _first = ok_result(call(
        &mut svc,
        "append_section_segment",
        json!({
            "project_id": pid, "section_id": leaf, "content": seg,
            "idem_key": format!("{leaf}/idem"),
        }),
    ));
    // 同 idem_key 同内容重放 → deduplicated（changed=false）
    let replay = ok_result(call(
        &mut svc,
        "append_section_segment",
        json!({
            "project_id": pid, "section_id": leaf, "content": seg,
            "idem_key": format!("{leaf}/idem"),
        }),
    ));
    assert_eq!(replay["deduplicated"], json!(true));
    // 字段面如实（below_min 仍 true——合稿确实不足），但零篇幅告警
    assert!(
        !warns_contain(&replay, "篇幅不足"),
        "幂等重放不得重复篇幅告警（账面零变动），实际: {}",
        replay["warnings"]
    );
}

// ---------------------------------------------------------------- 3. build_outline 配额面

/// 建纲走 MCP 工具通道的基座（不预建纲——build_outline 的合法前置态）。
fn svc_pre_outline(tag: &str) -> (BidService, String) {
    let mut svc = BidService::new();
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": "312 建纲配额面测试", "bid_deadline": "2026-12-31", "mode": "明标",
            "external_key": tag,
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    svc.store_mut()
        .upsert_file(
            "A-招标文件.docx",
            "招标文件",
            "sha-312-o",
            10,
            1,
            50,
            0,
            bid_domain::ParseStatus::Ok,
            "",
        )
        .unwrap();
    svc.set_requirements(vec![
        mk_req(
            "REQ-0001",
            "资格条件",
            "投标人须具备水利水电工程施工总承包贰级及以上资质",
        ),
        mk_req(
            "REQ-0002",
            "否决项",
            "投标文件未经法定代表人签字的作否决处理",
        ),
        mk_req("REQ-0003", "评分项", "施工组织设计完整性 10 分"),
    ])
    .unwrap();
    (svc, pid)
}

/// leaf_quotas 全量返回 + 守恒；warnings 字段在场（合成注入无评分树——
/// 恒空；「配额回退」的真值面由真件探针验证，见 PR 描述）。
#[test]
fn outline_leaf_quotas_face() {
    let (mut svc, pid) = svc_pre_outline("outline-face");
    let out = ok_result(call(
        &mut svc,
        "build_outline",
        json!({
            "project_id": pid, "target_total_chars": 30_000, "max_reqs_per_leaf": 2,
        }),
    ));
    let quotas = out["leaf_quotas"].as_array().expect("leaf_quotas 在场");
    assert!(!quotas.is_empty(), "leaf_quotas 不得为空");
    let n_leaves = out["counts"]["leaves"].as_u64().unwrap() as usize;
    assert_eq!(
        quotas.len(),
        n_leaves,
        "leaf_quotas 必须全量（建纲即可规划）"
    );
    let sum: u64 = quotas
        .iter()
        .map(|q| q["target_chars"].as_u64().unwrap_or(0))
        .sum();
    let total = out["counts"]["total_target_chars"].as_u64().unwrap();
    assert_eq!(
        sum, total,
        "leaf_quotas 配额和 == counts.total_target_chars（守恒）"
    );
    for q in quotas {
        assert!(q["section_id"].is_string() && q["title"].is_string());
    }
    // warnings 字段必须在场（合成注入无评分树 → 空；字段缺失才是回归）
    assert!(
        out["warnings"].as_array().is_some(),
        "warnings 字段必须在场（#312 建纲警告通道），实际 keys: {:?}",
        out.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );
    assert!(
        out["warnings"].as_array().unwrap().is_empty(),
        "无评分树的合成纲不应有回退警告，实际: {}",
        out["warnings"]
    );
}

/// 小目标建纲同样带配额面（warnings 字段在场——合成注入无评分树恒空，
/// 「配额回退」真值面由真件探针验证，见 PR 描述）。
#[test]
fn outline_small_target_quota_face() {
    let (mut svc, pid) = svc_pre_outline("outline-small");
    let out = ok_result(call(
        &mut svc,
        "build_outline",
        json!({
            "project_id": pid, "target_total_chars": 3_000, "max_reqs_per_leaf": 2,
        }),
    ));
    let quotas = out["leaf_quotas"].as_array().expect("leaf_quotas 在场");
    assert!(!quotas.is_empty());
    let sum: u64 = quotas
        .iter()
        .map(|q| q["target_chars"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(sum, out["counts"]["total_target_chars"].as_u64().unwrap());
    // warnings 通道在场（合成无评分树 → 空；字段缺失才是回归）
    assert!(out["warnings"].as_array().is_some());
    assert!(out["warnings"].as_array().unwrap().is_empty());
}
