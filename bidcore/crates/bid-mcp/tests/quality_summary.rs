//! #145 任务 3 · check_bid_quality 的跨章查重摘要 + 篇幅核对（bid-mcp 壳层）。
//!
//! 验收面（issue #145 §1）——\`summary\` 追加 5 字段，Agent 不必自己遍历
//! sections 算：
//!
//! | 字段 | 口径 |
//! |---|---|
//! | \`dup_pairs\` | 相似度 ≥ 0.8 的章节**对**数（a.id < b.id 只计一次） |
//! | \`dup_max_similarity\` | 全项目最高相似度（无重复 0.0） |
//! | \`char_total\` | 叶节点 content 字符数之和 |
//! | \`char_target\` | outline 根节点配额和（\`outline_total_chars\`） |
//! | \`char_deviation_pct\` | (total-target)/target*100，1 位小数；target=0 → null |
//!
//! 现场（两章写同一段「7×24 小时服务」承诺）：引擎的句级 Jaccard 早就算出来
//! 并回填了 \`dup_with\`（quality.rs \`detect_duplication\`），本套件验的是
//! **壳层把它汇总成摘要**——摘要断了 Agent 就得自己爬 sections（#145 的缺口）。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut().unwrap().insert("generator".into(), json!("mock"));
    }
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
        category: "技术参数".into(),
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

/// 建项目 → 抽条款 → 建纲 → 矩阵/证据/人工门。
///
/// 与 check_quality_tool.rs 的 \`svc_drafted\` 同基座（服务面步骤显式标注），
/// 差别是本套件要**多叶**——跨章查重至少两章才有比对面。
fn svc_with_outline(tag: &str) -> (BidService, String, Vec<String>) {
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "跨章查重摘要测试", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-qs", 10, 1, 50, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    // 抽取面（服务面——四条 REQ 保证建纲后至少两个叶：max_reqs_per_leaf=2）
    svc.set_requirements(vec![
        mk_req("REQ-0001", "提供 7×24 小时技术响应服务"),
        mk_req("REQ-0002", "故障响应时间不超过 30 分钟"),
        mk_req("REQ-0003", "提供贰级及以上资质证明"),
        mk_req("REQ-0004", "项目经理须持一级建造师证书"),
    ]).unwrap();
    // 建纲 + 建矩阵（服务面：derive_and_set_outline 的调度入口）
    let (n_sec, _, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 4_000, max_reqs_per_leaf: 2 })
        .unwrap();
    assert!(n_sec >= 2, "建纲后至少两节点（跨章查重需要两章比对面）: {n_sec}");

    // 素材面（#110 任务 1 缺失回报闸）：技术参数类 REQ 须有可用证据佐证，
    // 否则 draft 拒生成
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::ProductDoc,
        title: "服务承诺书与设备检测报告".into(),
        source_file: "EV-svc.pdf".into(),
        source_sha256: "sha-qs-ev".into(),
        locator: EngineLocator::DocxParagraph {
            paragraph_index: 3,
            text_fingerprint: "fnv64:qs".into(),
        },
        fingerprint: "fnv64:qs".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某公司".into(),
        scope: "技术服务承诺佐证".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
    }).unwrap();
    let req_ids: Vec<String> =
        svc.store().requirements().iter().map(|r| r.id.clone()).collect();
    for rid in &req_ids {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    // 无补遗阻塞的人工出口（#131——本基座只导入招标文件，起草会被
    // MissingAddendum 拒；本套件面在摘要字段，不在 L3 阻塞语义）
    use bid_domain::review::{Decision, Gate};
    svc.store_mut()
        .record_review(Gate::AddendumWaiver, "human:quality-summary-test", Decision::Approved, vec![])
        .unwrap();

    let parents: std::collections::HashSet<String> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.clone()).collect();
    let leaves: Vec<String> = svc.store().sections().iter()
        .filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect();
    assert!(leaves.len() >= 2, "叶节点至少两个: {leaves:?}");
    (svc, pid, leaves)
}

/// 高度相似的正文：同一段「7×24 小时服务」承诺重复 20 遍。
///
/// 选句依据：句长 ≥ 6 字（\`DUP_MIN_SENTENCE_CHARS\`）才入句集，且章内
/// **归一化后去重**——重复同一句在句集里只有一句，两章的句集完全相同，
/// Jaccard = 1.0（严重雷同的极端形态，正是「复制粘贴」现场）。
fn duplicated_content() -> String {
    let sentence = "我方承诺提供7×24小时服务，接到故障报修后30分钟内响应并派员到场处置。";
    let mut s = String::new();
    for _ in 0..20 {
        s.push_str(sentence);
    }
    s
}

// ---------------------------------------------------------------- 验收

/// 验收主链：两章同一段承诺 → 摘要必须把「严重雷同」摆出来（dup_pairs ≥ 1
/// + dup_max_similarity ≥ 0.8），同时给出篇幅全局账。
#[test]
fn summary_reports_cross_section_duplication_and_char_totals() {
    let (mut svc, pid, leaves) = svc_with_outline("qs-dup");

    // 起草两章（MCP §6——mock 生成，确定性内容面）
    for sid in leaves.iter().take(2) {
        let r = ok_result(call(&mut svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": sid,
            "instructions": "按招标文件逐项响应",
        })));
        assert_eq!(r["section_status"], "DRAFTED", "章节 {sid} 起草失败");
    }

    // 现场构造：两章正文改成同一段承诺（句集完全相同 → 相似度 1.0）。
    // 走域面 set_section_content（正文真源 = node.content——与 draft 的 CAS
    // 面同门），状态已是 DRAFTED（上一轮真实起草打通的机器）。
    let dup_body = duplicated_content();
    for sid in leaves.iter().take(2) {
        svc.store_mut().set_section_content(sid, dup_body.clone()).unwrap();
    }

    let q = ok_result(call(&mut svc, "check_bid_quality", json!({ "project_id": &pid })));
    let sm = &q["summary"];

    // ---- 跨章查重摘要
    let dup_pairs = sm["dup_pairs"].as_u64().expect("dup_pairs 必须是整数");
    assert!(dup_pairs >= 1, "两章同文必须计严重雷同对（实测 {dup_pairs}）: {sm}");
    let dup_max = sm["dup_max_similarity"].as_f64()
        .expect("dup_max_similarity 必须是数值");
    assert!(dup_max >= 0.8, "最高相似度应 ≥ 0.8（实测 {dup_max}）");
    // 对只计一次（dup_with 双向回填——两章各有一条对端，不能算成 2 对）
    assert_eq!(dup_pairs, 1, "两章一对：a.id < b.id 只计一次（实测 {dup_pairs}）");

    // ---- 篇幅核对
    let char_total = sm["char_total"].as_u64().expect("char_total 必须是整数");
    let char_target = sm["char_target"].as_u64().expect("char_target 必须是整数");
    assert!(char_total > 0, "已写字数应 > 0（实测 {char_total}）");
    assert!(char_target > 0, "配额总数应 > 0（实测 {char_target}）");
    assert_eq!(char_target, 4_000, "char_target = outline 根节点配额和（spec 4,000）");
    // 字符口径：两章同文（dup_body 各 36 字 × 20）+ 其余叶子（无正文 = 0）
    assert_eq!(
        char_total,
        (dup_body.chars().count() * 2) as u64,
        "char_total = 叶节点 content 字符数之和"
    );

    // ---- 偏离百分比：数字或 null（target>0 时必为数字），1 位小数
    let dev = &sm["char_deviation_pct"];
    assert!(dev.is_number() || dev.is_null(), "char_deviation_pct 应为数字或 null: {dev}");
    let v = dev.as_f64().expect("target > 0 时偏离必须是数字而非 null");
    let expect = ((char_total as f64 - char_target as f64) / char_target as f64 * 100.0 * 10.0)
        .round() / 10.0;
    assert!(
        (v - expect).abs() < 1e-9,
        "偏离口径 (total-target)/target*100 保留 1 位: {v} vs {expect}"
    );
}

/// 无重复 + 无已写正文时的诚实面：dup_pairs = 0、dup_max_similarity = 0.0
/// （不是 null——「查过且没有」与「没查」是两件事）。
#[test]
fn summary_is_zero_not_null_when_no_duplication() {
    let (mut svc, pid, _leaves) = svc_with_outline("qs-empty");
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({ "project_id": &pid })));
    let sm = &q["summary"];
    assert_eq!(sm["dup_pairs"], 0, "未起草章无比对面: {sm}");
    assert_eq!(sm["dup_max_similarity"].as_f64(), Some(0.0), "无重复 → 0.0: {sm}");
    assert_eq!(sm["char_total"], 0, "未起草 → 已写 0 字: {sm}");
    assert_eq!(sm["char_target"], 4_000, "配额面照常给出: {sm}");
    assert_eq!(
        sm["char_deviation_pct"].as_f64(), Some(-100.0),
        "0 字 vs 4,000 配额 = -100.0: {sm}"
    );
}

/// target = 0（无配额）→ char_deviation_pct 为 null（无分母，不伪装 0.0）。
///
/// 构造说明：建纲路径**不会**产出 target=0 的树——`derive_outline` 给每个
/// 叶子兜底 `MIN_LEAF_CHARS`(500)（target_total_chars=0 时配额仍被抬到
/// 500/叶），所以零配额只能经 `set_outline` 直接落树。本用例验的是摘要
/// 函数自身的除零面（壳层契约），不是建纲行为（那个面归 outline.rs）。
#[test]
fn char_deviation_is_null_when_target_is_zero() {
    use bid_domain::outline::OutlineNode;
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "零配额测试", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": "qs-zero",
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A.docx", "招标文件", "sha-z", 5, 0, 20, 0, bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![mk_req("REQ-0001", "提供服务")]).unwrap();
    // 手工落一棵 target_chars=0 的树（set_outline 是树的真相入口——
    // 不走 derive 的兜底下限）
    svc.store_mut().set_outline(vec![OutlineNode {
        id: "SEC-0001".into(),
        parent_id: None,
        title: "服务方案".into(),
        level: 1,
        order: 1,
        requirement_ids: vec!["REQ-0001".into()],
        target_chars: 0,
        content: None,
        status: bid_domain::outline::SectionStatus::Pending,
        content_attempt: 0,
        dirty: false,
    }]).unwrap();
    // 建矩阵 + 推进到 MATRIX_READY（check_bid_quality 的状态门）
    let _ = svc.store_mut().build_matrix().unwrap();
    let q = ok_result(call(&mut svc, "check_bid_quality", json!({ "project_id": &pid })));
    let sm = &q["summary"];
    assert_eq!(sm["char_target"], 0, "零配额: {sm}");
    assert!(sm["char_deviation_pct"].is_null(), "target=0 → null（不伪装 0.0）: {sm}");
}
