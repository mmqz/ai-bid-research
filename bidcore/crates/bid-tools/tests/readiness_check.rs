//! readiness_check.rs —— issue #103 子任务 C：缺失回报（生成前置检查 + 占位符）。
//!
//! 覆盖（issue 验收线）：
//! - 六种 Blocker 各至少一个用例（NoRequirements / MissingQualification /
//!   MissingPerformance / MissingTechEvidence / UncoveredVetoItem / IncompleteTender）；
//! - Agent 必选接线（UncoveredMandatoryItem）：Agent 标必选且类别不在
//!   资格/否决阻断面、矩阵未响应 → 阻塞；非必选/无标记不误断；否决类不双报；
//! - 承诺型裁定（PR #241 决策点 2）：正文已响应但无外部证据**不算已响应**
//!  （正文不是矩阵派生输入）；出口=绑佐证或人工覆写 RESPONDED+说明；
//! - 阻塞信息**人类可读**（不含 enum 名）；
//! - 占位符：扫描正文里的【待补充】、行号、往返一致；
//! - 真实场景：没有业绩证据的章节 → `ready = false`（issue 验收第 4 条）；
//! - 就绪：素材齐备 → `ready = true` 且零阻塞；
//! - 只读纪律：检查不改 store、不 bump content_version。
//!
//! 夹具纪律：域状态全部走真实状态机（create_project → upsert_file →
//! set_requirements → set_outline → register_evidence → bind_evidence →
//! build_matrix），不 force 字段——与生产路径同构。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::placeholder::{
    count_placeholders, make_placeholder, placeholder_report, scan_placeholders, PlaceholderKind,
};
use bid_tools::readiness::{
    check_all_readiness, check_draft_readiness, tender_gaps, Blocker, DraftReadiness,
    MIN_PERFORMANCE_COUNT,
};

// ---------------------------------------------------------------- 夹具

/// 招标文件导入（kind = 招标文件——`IncompleteTender` 的判定面）。
fn import_tender(s: &mut DomainStore, name: &str, kind: &str, sha: &str) {
    s.upsert_file(name, kind, sha, 120, 8, 900, 0, ParseStatus::Ok, "engine-docx @ test")
        .unwrap();
}

/// 条款（锚点逐条唯一——set_requirements 按 (file, locator, excerpt) 哈希去重）。
fn req(id: &str, category: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第三章/{id}"),
            excerpt: format!("{id} 的条款原文摘录（逐条唯一）"),
        }],
        params: Default::default(),
        source_file: "招标文件.docx".into(),
    }
}

/// Agent 路径 REQ（merge.rs 转换产物形状：params.marked_by + params.mandatory）。
fn agent_req(id: &str, category: &str, mandatory: bool) -> Requirement {
    let mut r = req(id, category);
    r.params.insert("marked_by".into(), serde_json::json!("agent"));
    r.params.insert("mandatory".into(), serde_json::json!(mandatory));
    r
}

fn node(id: &str, title: &str, order: u32, reqs: &[&str]) -> OutlineNode {
    OutlineNode {
        id: id.into(),
        parent_id: None,
        title: title.into(),
        level: 1,
        order,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars: 800,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }
}

/// 登记一条证据（出生态 Active）。
fn register_ev(s: &mut DomainStore, kind: EvidenceKind, title: &str, valid_until: Option<&str>) -> String {
    let (eid, _) = s
        .register_evidence(EvidenceSpec {
            kind,
            title: title.into(),
            source_file: format!("{title}.pdf"),
            source_sha256: format!("sha-{title}"),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 3,
                text_fingerprint: "fnv64:9e37".into(),
            },
            fingerprint: "fnv64:9e37".into(),
            valid_from: None,
            valid_until: valid_until.map(str::to_string),
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
    eid
}

/// 基座：一份招标文件 + 一份补遗 + 条款 + 章节（矩阵未建）。
///
/// **L3 补遗**：本文件的所有用例都是「章节级」判定（资格/业绩/技术证据/否决项），
/// 需要补遗件在场才能隔离出它们各自的 Blocker——`MissingAddendum` 是**全局**
/// 阻塞（见 readiness.rs 模块头），没有补遗时它会出现在每一个用例里，
/// 把「本章还缺什么」的断言全部污染。基座补一份成功导入的补遗，
/// 补遗缺失本身的行为由 `readiness_l3_addendum.rs` 专门覆盖。
/// （#127 代理 C 注：此行曾被 5777ba8 误删、#122 拆分版继承——9 用例
/// 被 MISSING_ADDENDUM 污染全挂；现按 e363ba0 原样恢复。）
fn base_store(sections: Vec<OutlineNode>, reqs: Vec<Requirement>) -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("缺失回报测试", "ZFCG-2026-R-001", "明标", "r-key").unwrap();
    import_tender(&mut s, "招标文件.docx", "招标文件", "sha-tender");
    // L3：补遗件在场（全局阻塞不干扰章节级断言）
    import_tender(&mut s, "C-补遗文件01.docx", "补遗", "sha-addendum");
    s.set_requirements(reqs).unwrap();
    s.set_outline(sections).unwrap();
    s.build_matrix().unwrap();
    s
}

fn codes(r: &DraftReadiness) -> Vec<&'static str> {
    r.blockers.iter().map(Blocker::code).collect()
}

// ---------------------------------------------------------------- ① NoRequirements

#[test]
fn blocker_no_requirements() {
    // 空响应面章节：set_outline 的结构校验拒绝「未绑 REQ」的节点（分片最小
    // 调度单元必须 ≥1），所以这种节点只能从**存量快照**进来（历史库、外部
    // 导入、或上游绕校验写入）——前置检查必须能识别它而不是假设它不存在。
    let mut s = base_store(
        vec![node("SEC-0002", "其他", 2, &["REQ-0001"])],
        vec![req("REQ-0001", "商务要求")],
    );
    let mut snap = s.to_snapshot();
    snap.sections.push(node("SEC-0001", "项目理解", 1, &[]));
    snap.sections.sort_by_key(|n| n.order);
    s = DomainStore::restore(snap).expect("快照恢复（空响应面章节）");

    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(!r.ready, "空响应面必须阻塞：{:?}", r);
    assert_eq!(codes(&r), vec!["NO_REQUIREMENTS"]);
    let msg = r.blockers[0].to_message();
    assert!(msg.contains("SEC-0001") && msg.contains("没有关联任何招标要求"), "{msg}");
    assert!(!msg.contains("NoRequirements"), "不得泄漏 enum 名：{msg}");
}

#[test]
fn unknown_section_is_blocked_not_panicking() {
    // 前置检查本身不能成为崩溃源
    let s = base_store(vec![node("SEC-0001", "响应", 1, &["REQ-0001"])], vec![req("REQ-0001", "商务要求")]);
    let r = check_draft_readiness(&s, "SEC-9999");
    assert!(!r.ready);
    assert_eq!(codes(&r), vec!["NO_REQUIREMENTS"]);
    assert!(r.blockers[0].to_message().contains("SEC-9999"));
}

// ---------------------------------------------------------------- ② MissingQualification

#[test]
fn blocker_missing_qualification() {
    // 「资格及资质证明」章：台账里一条资质证据都没有
    let s = base_store(
        vec![node("SEC-0001", "资格及资质证明文件", 1, &["REQ-0001"])],
        vec![req("REQ-0001", "资格条件")],
    );
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(!r.ready);
    assert!(codes(&r).contains(&"MISSING_QUALIFICATION"), "{:?}", codes(&r));
    let msg = r.blockers[0].to_message();
    assert!(msg.contains("资格及资质证明文件"), "要点名缺哪项：{msg}");
    assert!(msg.contains("证据库里没有可用的资格/资质证明材料"), "{msg}");
    assert!(!msg.contains("MissingQualification"), "{msg}");
}

#[test]
fn qualification_blocked_when_only_expired_certificate() {
    // 只剩一张过期证 → 仍阻塞，且如实回报「有但是失效了」（不假装没有）
    let mut s = base_store(
        vec![node("SEC-0001", "资质证明", 1, &["REQ-0001"])],
        vec![req("REQ-0001", "资格条件")],
    );
    register_ev(&mut s, EvidenceKind::Qualification, "建筑工程施工总承包二级资质", Some("2020-01-01"));
    // 过期是**时间驱动**的派生态：登记出生态恒为 Active（R-03 单向降级不复活的
    // 另一面——不按开标日过一遍，过期证会一直「活着」）。真实管线里由开标日前
    // 的 apply_validity 完成这一跳，这里同样走真实链路，不手改 status。
    let expired = s.apply_validity("2026-06-01").unwrap();
    assert_eq!(expired.len(), 1, "开标日 2026-06-01 该把 2020 年到期的证翻成 EXPIRED：{expired:?}");
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(!r.ready, "过期证不算有：{:?}", r);
    let b = r.blockers.iter().find(|b| b.code() == "MISSING_QUALIFICATION").expect("应缺资质");
    let Blocker::MissingQualification { have, .. } = b else { panic!("{b:?}") };
    assert!(have.iter().any(|t| t.contains("EXPIRED")), "须报出失效明细：{have:?}");
    assert!(b.to_message().contains("台账里有但不可用"), "{}", b.to_message());
}

// ---------------------------------------------------------------- ③ MissingPerformance

/// issue 验收第 4 条：真实场景——没有业绩证据的章节 → ready = false。
#[test]
fn real_scenario_no_performance_evidence_blocks() {
    let s = base_store(
        vec![node("SEC-0001", "类似项目业绩", 1, &["REQ-0001"])],
        vec![req("REQ-0001", "评分项")],
    );
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(!r.ready, "没有业绩证据的章节绝不能就绪");
    assert_eq!(codes(&r), vec!["MISSING_PERFORMANCE"]);
    let msg = r.blockers[0].to_message();
    assert!(!msg.contains("MissingPerformance"), "不得泄漏 enum 名：{msg}");
    assert!(msg.contains("需要 3 个类似项目业绩，现有 0 个"), "{msg}");
    // 正文该留的占位也一并给出（接线方直接可用）
    let ph = r.blockers[0].suggest_placeholder();
    assert_eq!(ph, "【待补充：请提供 3 个类似项目业绩（现有 0 个）】");
}

#[test]
fn performance_needs_three_usable_records() {
    let mut s = base_store(
        vec![node("SEC-0001", "类似项目业绩", 1, &["REQ-0001"])],
        vec![req("REQ-0001", "评分项")],
    );
    for i in 1..=MIN_PERFORMANCE_COUNT {
        register_ev(&mut s, EvidenceKind::Performance, &format!("类似项目合同 {i}"), None);
    }
    // 再加一条过期的（不算数）
    register_ev(&mut s, EvidenceKind::Performance, "过期项目合同", Some("2020-01-01"));
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(r.ready, "3 条可用业绩即达标（过期那条不计）：{:?}", r);

    // 少一条 → 命名精确回报
    let mut s2 = base_store(
        vec![node("SEC-0001", "类似项目业绩", 2, &["REQ-0001"])],
        vec![req("REQ-0001", "评分项")],
    );
    register_ev(&mut s2, EvidenceKind::Performance, "类似项目合同 1", None);
    let r2 = check_draft_readiness(&s2, "SEC-0001");
    assert!(!r2.ready);
    assert!(r2.blockers[0].to_message().contains("现有 1 个"), "{}", r2.blockers[0].to_message());
}

// ---------------------------------------------------------------- ④ MissingTechEvidence

#[test]
fn blocker_missing_tech_evidence() {
    let s = base_store(
        vec![node("SEC-0001", "技术方案", 1, &["REQ-0001", "REQ-0002"])],
        vec![req("REQ-0001", "技术参数"), req("REQ-0002", "技术参数")],
    );
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(!r.ready);
    let b = r.blockers.iter().find(|b| b.code() == "MISSING_TECH_EVIDENCE").expect("应缺技术证据");
    let msg = b.to_message();
    assert!(msg.contains("REQ-0001") && msg.contains("REQ-0002"), "要点名缺证据的条款：{msg}");
    assert!(msg.contains("中标后违约"), "要说清后果：{msg}");
    assert!(!msg.contains("MissingTechEvidence"), "不得泄漏 enum 名：{msg}");
}

#[test]
fn tech_evidence_bound_clears_blocker() {
    // 绑定页在 register_evidence 之后才能拿到 EV id，故不放进 base_store()
    let mut s = base_store(
        vec![node("SEC-0001", "技术方案", 1, &["REQ-0001", "REQ-0002"])],
        vec![req("REQ-0001", "技术参数"), req("REQ-0002", "技术参数")],
    );
    let ev = register_ev(&mut s, EvidenceKind::ProductDoc, "设备检测报告", None);
    s.bind_evidence(&ev, "REQ-0001", "2026-12-31").unwrap();
    s.build_matrix().unwrap();
    let r = check_draft_readiness(&s, "SEC-0001");
    let b = r.blockers.iter().find(|b| b.code() == "MISSING_TECH_EVIDENCE").expect("REQ-0002 仍缺");
    assert_eq!(b, &Blocker::MissingTechEvidence { req_ids: vec!["REQ-0002".into()] }, "已绑的 REQ-0001 不再报");
}

// ---------------------------------------------------------------- ⑤ UncoveredVetoItem

#[test]
fn blocker_uncovered_veto_item() {
    let s = base_store(
        vec![node("SEC-0001", "响应承诺", 1, &["REQ-0001"])],
        vec![req("REQ-0001", "否决项")],
    );
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(!r.ready);
    let b = r.blockers.iter().find(|b| b.code() == "UNCOVERED_VETO_ITEM").expect("应缺否决项响应");
    let msg = b.to_message();
    assert!(msg.contains("否决项尚未响应") && msg.contains("REQ-0001"), "{msg}");
    assert!(msg.contains("直接废标"), "要说清后果：{msg}");
    assert!(!msg.contains("UncoveredVetoItem"), "不得泄漏 enum 名：{msg}");
}

#[test]
fn veto_cleared_by_evidence_bind_and_by_override_with_reason() {
    let mut s = base_store(
        vec![node("SEC-0001", "响应承诺", 1, &["REQ-0001", "REQ-0002"])],
        vec![req("REQ-0001", "否决项"), req("REQ-0002", "否决项")],
    );
    // REQ-0001：证据绑定 → 矩阵 RESPONDED
    let ev = register_ev(&mut s, EvidenceKind::Certificate, "安全生产许可证", None);
    s.bind_evidence(&ev, "REQ-0001", "2026-12-31").unwrap();
    s.build_matrix().unwrap();
    let r = check_draft_readiness(&s, "SEC-0001");
    let b = r.blockers.iter().find(|b| b.code() == "UNCOVERED_VETO_ITEM").expect("REQ-0002 仍未响应");
    assert_eq!(b, &Blocker::UncoveredVetoItem { req_ids: vec!["REQ-0002".into()] });

    // REQ-0002：带理由的不适用覆写 → 视为已处理（可审计的豁免）
    s.set_matrix_override(
        "REQ-0002".into(),
        bid_domain::matrix::ResponseStatus::NotApplicable,
        Some("本项目不含进口设备，该否决项不适用（招标文件 §9.2）".into()),
        &bid_domain::principal::TrustedPrincipal::human("readiness-lead"),
    )
    .unwrap();
    s.build_matrix().unwrap();
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(r.ready, "否决项全部处理完 → 就绪：{:?}", r);
}

// ---------------------------------------------------------------- ⑤b UncoveredMandatoryItem（Agent 必选接线）

/// Agent 标必选（类别不在资格/否决阻断面）且矩阵未响应 → 起草门阻塞。
#[test]
fn agent_mandatory_unresponded_blocks_draft() {
    let s = base_store(
        vec![node("SEC-0001", "商务响应", 1, &["REQ-0001"])],
        vec![agent_req("REQ-0001", "商务要求", true)],
    );
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(!r.ready, "Agent 必选未响应必须阻塞：{:?}", r);
    let b = r.blockers
        .iter()
        .find(|b| b.code() == "UNCOVERED_MANDATORY_ITEM")
        .expect("应报 Agent 必选未响应");
    let msg = b.to_message();
    assert!(msg.contains("必选条款尚未响应") && msg.contains("REQ-0001"), "{msg}");
    assert!(!msg.contains("UncoveredMandatoryItem"), "不得泄漏 enum 名：{msg}");
}

/// 出口与否决项同款：证据绑定（矩阵 RESPONDED）或人工覆写（不适用+理由）→ 放行。
#[test]
fn agent_mandatory_cleared_by_evidence_and_override() {
    let mut s = base_store(
        vec![node("SEC-0001", "商务响应", 1, &["REQ-0001", "REQ-0002"])],
        vec![
            agent_req("REQ-0001", "商务要求", true),
            agent_req("REQ-0002", "商务要求", true),
        ],
    );
    // REQ-0001：证据绑定 + 章节在位 → 矩阵 RESPONDED
    let ev = register_ev(&mut s, EvidenceKind::ProductDoc, "展期服务承诺函", None);
    s.bind_evidence(&ev, "REQ-0001", "2026-12-31").unwrap();
    s.build_matrix().unwrap();
    let r = check_draft_readiness(&s, "SEC-0001");
    let b = r.blockers
        .iter()
        .find(|b| b.code() == "UNCOVERED_MANDATORY_ITEM")
        .expect("REQ-0002 仍未响应");
    assert_eq!(
        b,
        &Blocker::UncoveredMandatoryItem { req_ids: vec!["REQ-0002".into()] },
        "已响应的 REQ-0001 不再报"
    );

    // REQ-0002：带理由的人工覆写（#182 receipt 面的域层半边）→ 放行
    s.set_matrix_override(
        "REQ-0002".into(),
        bid_domain::matrix::ResponseStatus::NotApplicable,
        Some("该承诺由厂商标准配置满足，无需专项响应（招标文件 §5.4）".into()),
        &bid_domain::principal::TrustedPrincipal::human("readiness-lead"),
    )
    .unwrap();
    s.build_matrix().unwrap();
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(r.ready, "Agent 必选全部处理完 → 就绪：{:?}", r);
}

/// 非误断对照：非必选（mandatory=false）与无标记（正则形状）未响应 → 不进本门。
#[test]
fn non_mandatory_and_unmarked_do_not_trip_gate() {
    let s = base_store(
        vec![node("SEC-0001", "商务响应", 1, &["REQ-0001", "REQ-0002"])],
        vec![
            agent_req("REQ-0001", "商务要求", false),
            req("REQ-0002", "商务要求"),
        ],
    );
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(r.ready, "非必选条款未响应不得误断：{:?}", r);
    assert!(!codes(&r).contains(&"UNCOVERED_MANDATORY_ITEM"));
}

/// 类别已是否决项的 Agent 必选条款归 UncoveredVetoItem 报，不双报。
#[test]
fn agent_mandatory_veto_category_reports_via_veto_blocker_only() {
    let s = base_store(
        vec![node("SEC-0001", "响应承诺", 1, &["REQ-0001"])],
        vec![agent_req("REQ-0001", "否决项", true)],
    );
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(!r.ready);
    assert!(codes(&r).contains(&"UNCOVERED_VETO_ITEM"), "否决类照常报：{:?}", codes(&r));
    assert!(
        !codes(&r).contains(&"UNCOVERED_MANDATORY_ITEM"),
        "否决类不双报：{:?}",
        codes(&r)
    );
}

/// 裁定（PR #241 报 Lead 决策点 2，2026-11-04）：普通商务必选条款
/// 「正文已响应、但没有外部证据」**不算已响应**。
///
/// 依据：矩阵四态（T-2 规格 / V3-2 实装注）的派生输入只有
/// 绑定 / 章节归属 / 覆写三个真源——**章节正文内容不是派生输入**
/// （derive_matrix_rows 结构上不读 content）。承诺型商务条款的出口
/// 与 #226 死锁同款：有佐证材料就绑定（→RESPONDED），没有就走
/// 人工覆写（RESPONDED+说明——正文承诺型；NOT_APPLICABLE+理由——
/// 真不适用）；Agent 不得以「正文写了承诺」自证已响应（#182：覆写
/// 只认 Human receipt）。
#[test]
fn drafted_body_without_evidence_is_not_response() {
    let mut s = base_store(
        vec![node("SEC-0001", "商务响应", 1, &["REQ-0001"])],
        vec![agent_req("REQ-0001", "商务要求", true)],
    );
    // 正文已落承诺（真实写路径：状态链 Pending→Outlining→Drafting→Drafted
    // + set_section_content——不 force 字段）
    s.advance_section_status("SEC-0001", SectionStatus::Outlining).unwrap();
    s.advance_section_status("SEC-0001", SectionStatus::Drafting).unwrap();
    s.set_section_content(
        "SEC-0001",
        "我方郑重承诺：质保期不少于五年，满足招标文件全部要求。".into(),
    )
    .unwrap();
    s.advance_section_status("SEC-0001", SectionStatus::Drafted).unwrap();
    s.build_matrix().unwrap();

    // 矩阵面：正文在位，无证据无覆写 → 仍 NOT_RESPONDED、仍 blocking
    let row = s
        .matrix()
        .iter()
        .find(|r| r.requirement_id == "REQ-0001")
        .expect("REQ-0001 在矩阵中");
    assert_eq!(
        row.status,
        bid_domain::matrix::ResponseStatus::NotResponded,
        "正文不是矩阵派生输入——无证据无覆写即未响应（裁定主面）"
    );
    assert!(row.blocking, "Agent 必选商务条款未响应 → blocking");

    // 起草门：已起草的正文不豁免响应账（复核/再起草同口径）
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(!r.ready, "正文已写不等于已响应：{:?}", r);
    let b = r
        .blockers
        .iter()
        .find(|b| b.code() == "UNCOVERED_MANDATORY_ITEM")
        .expect("正文在位仍应报必选未响应");
    let msg = b.to_message();
    assert!(msg.contains("REQ-0001") && !msg.contains("UncoveredMandatoryItem"), "{msg}");
}

/// 承诺型商务必选条款的**正门出口**：人工覆写 RESPONDED（带说明）。
///
/// 与 NOT_APPLICABLE（真不适用）的分野：条款适用、已以正文承诺响应、
/// 仅无外部佐证材料时，诚实态是 RESPONDED+说明——不是谎报「不适用」。
/// 全链出口见 agent_mandatory_gate.rs
/// `promise_form_mandatory_exits_via_responded_override_receipt`（receipt 面）。
#[test]
fn responded_override_with_reason_is_the_promise_form_exit() {
    let mut s = base_store(
        vec![node("SEC-0001", "商务响应", 1, &["REQ-0001"])],
        vec![agent_req("REQ-0001", "商务要求", true)],
    );
    s.set_section_content(
        "SEC-0001",
        "我方郑重承诺：质保期不少于五年，满足招标文件全部要求。".into(),
    )
    .unwrap();
    let reason = "商务承诺条款：正文承诺响应，无外部佐证材料（人工确认响应成立）";
    s.set_matrix_override(
        "REQ-0001".into(),
        bid_domain::matrix::ResponseStatus::Responded,
        Some(reason.into()),
        &bid_domain::principal::TrustedPrincipal::human("readiness-lead"),
    )
    .unwrap();
    s.build_matrix().unwrap();

    let row = s
        .matrix()
        .iter()
        .find(|r| r.requirement_id == "REQ-0001")
        .expect("REQ-0001 在矩阵中");
    assert_eq!(row.status, bid_domain::matrix::ResponseStatus::Responded);
    assert!(!row.blocking, "人工确认的正文承诺响应 → 不再 blocking");
    assert_eq!(row.deviation.as_deref(), Some(reason), "说明落账（可审计）");

    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(r.ready, "承诺型条款经人工 RESPONDED 覆写后应就绪：{:?}", r);
}

// ---------------------------------------------------------------- ⑥ IncompleteTender（警告）

#[test]
fn blocker_incomplete_tender_is_warning_not_blocker() {
    // 只有附件、没有招标文件本体；也没有补遗件
    let mut s = DomainStore::new();
    s.create_project("缺件测试", "ZFCG-2026-R-002", "明标", "r-key-2").unwrap();
    import_tender(&mut s, "图纸.dwg", "图纸", "sha-dwg");
    s.set_requirements(vec![req("REQ-0001", "商务要求")]).unwrap();
    s.set_outline(vec![node("SEC-0001", "商务响应", 1, &["REQ-0001"])]).unwrap();
    s.build_matrix().unwrap();

    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(r.ready, "招标包缺口只警告不阻塞：{:?}", r);
    assert!(r.blockers.is_empty(), "{:?}", r.blockers);
    let joined = r.warnings.join("\n");
    assert!(joined.contains("没有导入任何「招标文件」类文件"), "{joined}");
    // 「未导入补遗」警告已按 L3（#118）删除——同一件事报两遍是噪音；
    // 空包时 MissingAddendum 判 ①（无招标文件本体）不报，补遗缺口归
    // blocker 面（tender_without_addendum_is_blocker_not_warning 专测）。
    assert!(!joined.contains("IncompleteTender"), "不得泄漏 enum 名：{joined}");
}

#[test]
fn tender_gaps_reports_duplicate_tenders() {
    let mut s = DomainStore::new();
    s.create_project("多份招标文件", "ZFCG-2026-R-003", "明标", "r-key-3").unwrap();
    import_tender(&mut s, "招标文件.docx", "招标文件", "sha-a");
    import_tender(&mut s, "招标文件-修订.pdf", "招标文件", "sha-b");
    import_tender(&mut s, "补遗书01.pdf", "补遗", "sha-c");
    let gaps = tender_gaps(&s);
    assert_eq!(gaps.len(), 1, "补遗已在位，只剩正本歧义：{gaps:?}");
    assert!(gaps[0].contains("无法确定正本"), "{gaps:?}");
    assert_eq!(
        Blocker::IncompleteTender { missing: gaps }.code(),
        "INCOMPLETE_TENDER"
    );
}

// ---------------------------------------------------------------- 就绪 + 批量 + 只读

#[test]
fn fully_provisioned_section_is_ready() {
    let mut s = base_store(
        vec![node("SEC-0001", "技术方案", 1, &["REQ-0001"])],
        vec![req("REQ-0001", "技术参数")],
    );
    let ev = register_ev(&mut s, EvidenceKind::ProductDoc, "设备检测报告", None);
    s.bind_evidence(&ev, "REQ-0001", "2026-12-31").unwrap();
    s.build_matrix().unwrap();
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(r.ready, "{:?}", r);
    assert!(r.blockers.is_empty());
    assert!(r.to_message().starts_with("可以起草"), "{}", r.to_message());
}

#[test]
fn readiness_is_read_only() {
    let s = base_store(
        vec![node("SEC-0001", "类似项目业绩", 1, &["REQ-0001"])],
        vec![req("REQ-0001", "评分项")],
    );
    let v = s.content_version();
    let secs = s.sections().len();
    let evs = s.evidences().len();
    let _ = check_draft_readiness(&s, "SEC-0001");
    let _ = check_all_readiness(&s);
    assert_eq!(s.content_version(), v, "诊断不得 bump 版本");
    assert_eq!(s.sections().len(), secs);
    assert_eq!(s.evidences().len(), evs);
}

#[test]
fn check_all_covers_leaf_sections_only_and_message_aggregates() {
    // SEC-0000 结构节点（有子）不参与；两个叶：一个缺业绩、一个就绪。
    // 父聚合不变量由 set_outline 强制：requirement_ids = 子并集，
    // target_chars = 子配额和（2 × 800）。
    let mut parent = node("SEC-0000", "商务部分", 0, &["REQ-0001", "REQ-0002"]);
    parent.level = 1;
    parent.target_chars = 1600;
    let mut leaf1 = node("SEC-0001", "类似项目业绩", 1, &["REQ-0001"]);
    leaf1.parent_id = Some("SEC-0000".into());
    leaf1.level = 2;
    let mut leaf2 = node("SEC-0002", "技术方案", 2, &["REQ-0002"]);
    leaf2.parent_id = Some("SEC-0000".into());
    leaf2.level = 2;
    let mut s = base_store(
        vec![parent, leaf1, leaf2],
        vec![req("REQ-0001", "评分项"), req("REQ-0002", "技术参数")],
    );
    let ev = register_ev(&mut s, EvidenceKind::ProductDoc, "检测报告", None);
    s.bind_evidence(&ev, "REQ-0002", "2026-12-31").unwrap();
    s.build_matrix().unwrap();

    let all = check_all_readiness(&s);
    assert_eq!(all.len(), 2, "只查叶章节：{:?}", all.iter().map(|(id, _)| id).collect::<Vec<_>>());
    assert!(!all[0].1.ready, "缺业绩的叶必须阻塞");
    assert!(all[1].1.ready, "素材齐的叶就绪：{:?}", all[1].1);

    let msg = all[0].1.to_message();
    assert!(msg.starts_with("暂不可起草："), "{msg}");
    assert!(msg.contains("业绩证据不足"), "{msg}");
}

// ---------------------------------------------------------------- 占位符机制

#[test]
fn placeholder_scan_finds_tbd_markers_in_draft_body() {
    let body = "## 类似项目业绩\n\n我司近三年完成类似项目 0 个。\n\n【待补充：请提供 3 个类似项目业绩】\n\n资质部分见附件。\n【待补充：安全生产许可证】";
    let found = scan_placeholders(body);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].line, 5);
    assert_eq!(found[0].what, "请提供 3 个类似项目业绩");
    assert_eq!(found[1].line, 8);
    assert_eq!(found[0].kind, PlaceholderKind::Missing);

    let report = placeholder_report(body).expect("有占位就该有报告");
    assert!(report.contains("残留 2 处占位"), "{report}");
    assert!(!scan_placeholders("正文干干净净，没有占位。").len().gt(&0));
}

#[test]
fn placeholder_make_roundtrip_matches_issue_format() {
    // issue 原文的样例：「【待补充：请提供 3 个类似项目业绩】」
    let p = make_placeholder(PlaceholderKind::Missing, "请提供 3 个类似项目业绩");
    assert_eq!(p, "【待补充：请提供 3 个类似项目业绩】");
    assert_eq!(count_placeholders(&p), 1);
    let back = scan_placeholders(&p);
    assert_eq!(back[0].what, "请提供 3 个类似项目业绩");
    assert_eq!(back[0].render(), p);

    let c = make_placeholder(PlaceholderKind::Confirm, "工期口径待确认");
    assert_eq!(c, "【待确认：工期口径待确认】");
}
