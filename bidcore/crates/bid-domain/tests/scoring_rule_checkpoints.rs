//! #224 第 3 步 · score_rules 消费（规则层可见性）域层验收。
//!
//! issue #224 需求 2 的「至少做到」两项：
//! - 细则里的关键条件（量化条件 / 资质限定词）能被提取成检查点；
//! - 章节正文明显没提 → 该评分项标记「可能失分」。
//!
//! 边界（规则层可见性——不越界，全部有断言钉住）：
//! - **判档不变**：检查点不参与 tier/estimated（同文同分——完整语义
//!   判档依赖模型，另开 issue）；
//! - 档位行 / 计分公式（数字+「分」）是评分口径不是响应条件，不入检查点；
//! - 提及性 ≠ 满足性：数量真伪规则层不可验（正文「案例丰富」算提及，
//!   但不伪装「已提供 3 个」）；
//! - 数字+计量词代偿有区分力门槛：泛计量词（个/项…）阻断——正文
//!   「3 个方案」不虚报已提「3 个以上案例」条件；人员/时间/金额类
//!   计量词允许（「不少于 5 人」↔「配备 30 人」「近三年」↔「2023 年」）；
//! - 跨章合并：任一章提及即提及（多章分工响应合法）；全部章未提
//!   才计可能失分（notes 与 by_item 同口径——单一真源 merged_*）。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::scoring::{
    merged_rule_checkpoints, GradeTier, ItemScore, MappingSource, RuleCheckpoint, SectionScorer,
};
use bid_domain::{Anchor, DomainStore, Requirement};
use std::collections::HashMap;

fn anchor(excerpt: &str) -> Anchor {
    Anchor {
        file: "A-招标文件.docx".into(),
        kind: "para".into(),
        locator: "第四章 评标办法".into(),
        excerpt: excerpt.into(),
    }
}

/// 评分项 REQ（params: score + section? + score_rules?——全 None 面用 Option）。
fn score_req(
    id: &str,
    excerpt: &str,
    score: f64,
    section: Option<&str>,
    rules: Option<&str>,
) -> Requirement {
    let mut params = HashMap::new();
    params.insert("score".to_string(), serde_json::json!(score));
    if let Some(s) = section {
        params.insert("section".to_string(), serde_json::json!(s));
    }
    if let Some(r) = rules {
        params.insert("score_rules".to_string(), serde_json::json!(r));
    }
    Requirement {
        id: id.into(),
        category: "评分项".into(),
        clause_no: None,
        anchors: vec![anchor(excerpt)],
        params,
        source_file: "A-招标文件.docx".into(),
    }
}

/// 正文类 REQ（叶节点树不变量：每叶 ≥1 REQ——评分项不进正文 #126）。
fn body_req(id: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: "技术参数".into(),
        clause_no: None,
        anchors: vec![anchor(excerpt)],
        params: HashMap::new(),
        source_file: "A-招标文件.docx".into(),
    }
}

fn leaf(id: &str, order: u32, title: &str, reqs: &[&str]) -> OutlineNode {
    OutlineNode {
        id: id.into(),
        parent_id: None,
        title: title.into(),
        level: 1,
        order,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars: 2_000,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }
}

/// 基座 store：项目 + 文件 + 两叶（SEC-01/SEC-02，各绑一个正文 REQ）+
/// 评分项 REQ-0001（映射/细则由调用方给定）。
fn store_base(section: Option<&str>, rules: Option<&str>) -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("检查点测试", "C-CP", "明标", "cp-key")
        .unwrap();
    s.upsert_file(
        "A-招标文件.docx",
        "招标文件",
        "sha-cp",
        10,
        1,
        50,
        0,
        bid_domain::ParseStatus::Ok,
        "",
    )
    .unwrap();
    s.set_requirements(vec![
        score_req(
            "REQ-0001",
            "4.1 类似项目业绩与人员配置 20 分（方案应覆盖案例与人员配备）",
            20.0,
            section,
            rules,
        ),
        body_req("REQ-0002", "5.1 核心交换机须支持 48 口千兆光接口"),
        body_req("REQ-0003", "5.2 售后响应时限 ≤ 2 小时"),
    ])
    .unwrap();
    s.set_outline(vec![
        leaf("SEC-01", 1, "技术方案", &["REQ-0002"]),
        leaf("SEC-02", 2, "售后服务", &["REQ-0003"]),
    ])
    .unwrap();
    s
}

// ---------------------------------------------------------------- 提取面

/// 量化条件（数字+计量词）与资质限定词入册；计分公式（数字+「分」）
/// 不入——评分口径非响应条件。正文明显未提 → 可能失分标记（rationale
/// 响亮登记）+ **判档不变**（同文同分——检查点只标记风险不改估分）。
#[test]
fn quantified_gap_flags_possible_loss_without_score_change() {
    let rules = "提供 3 个以上类似项目案例；项目负责人具备一级注册建造师执业资格；每缺一项扣 2 分";
    let content =
        "我们高度重视售后服务工作，建立了完整的售后服务体系与快速响应机制，服务团队经验丰富。";
    let s = store_base(Some("SEC-01"), Some(rules));
    let scorer = SectionScorer::new(s.scoring_tree(), vec![]);
    let sec = scorer.score_section("SEC-01", content);

    // 提取面：量化 + 资质两条入册；「每缺一项扣 2 分」不入
    let it = &sec.item_scores[0];
    assert_eq!(
        it.rule_checkpoints.len(),
        2,
        "量化+资质入册、公式不入：{:?}",
        it.rule_checkpoints
    );
    assert!(
        it.rule_checkpoints
            .iter()
            .any(|c| c.text.contains("类似项目案例")),
        "量化条件在册：{:?}",
        it.rule_checkpoints
    );
    assert!(
        it.rule_checkpoints
            .iter()
            .any(|c| c.text.contains("一级注册建造师")),
        "资质条件在册：{:?}",
        it.rule_checkpoints
    );
    // 比对面：正文只谈售后服务——两条均明显未提
    assert_eq!(it.rule_gaps, 2, "两条均未提");
    assert!(it.rule_checkpoints.iter().all(|c| !c.mentioned));
    // rationale 响亮登记（可能失分 + 点名到条件原文）
    assert!(
        sec.rationale.contains("可能失分"),
        "登记可能失分：{}",
        sec.rationale
    );
    assert!(
        sec.rationale.contains("类似项目案例"),
        "缺口点名条件：{}",
        sec.rationale
    );

    // 判档不变（#224 第 3 步的边界）：同文同分——无细则基线对照
    let base = SectionScorer::new(store_base(Some("SEC-01"), None).scoring_tree(), vec![])
        .score_section("SEC-01", content);
    assert_eq!(
        sec.estimated, base.estimated,
        "检查点不改估分：{} vs {}",
        sec.estimated, base.estimated
    );
    assert_eq!(it.tier, base.item_scores[0].tier, "判档不变");
    assert_eq!(sec.covered_items, base.covered_items, "覆盖判定不变");
    assert_eq!(sec.max, base.max, "满分不变");
}

/// 档位行（优 9-10 分…）与得分表述（本项满分 20 分）不入检查点——
/// 它们是评分口径不是响应条件；量化条件（5 名项目经理）正常入册。
#[test]
fn tier_rows_and_score_formulas_are_not_conditions() {
    let rules =
        "优：9-10 分；良：7-9 分；中：5-7 分；差：0-5 分；提供 5 名项目经理；本项满分 20 分";
    let s = store_base(Some("SEC-01"), Some(rules));
    let scorer = SectionScorer::new(s.scoring_tree(), vec![]);
    let sec = scorer.score_section("SEC-01", "项目管理机构设置完备，职责分工明确。");
    let it = &sec.item_scores[0];
    assert_eq!(
        it.rule_checkpoints.len(),
        1,
        "只入量化条件：{:?}",
        it.rule_checkpoints
    );
    assert!(it.rule_checkpoints[0].text.contains("名项目经理"));
    // 正文未提项目经理且无数字+名形态 → 明显未提
    assert!(!it.rule_checkpoints[0].mentioned);
    assert_eq!(it.rule_gaps, 1);
    assert!(sec.rationale.contains("可能失分"));
}

// ---------------------------------------------------------------- 比对面（提及性三路）

/// 主题词组命中（路①）：正文提及条件主题词 → mentioned（提及性——
/// 数量真伪不验，不伪装满足）→ 无缺口不虚报。
#[test]
fn topic_word_mention_clears_gap() {
    let rules = "提供 3 个以上类似项目案例";
    let s = store_base(Some("SEC-01"), Some(rules));
    let scorer = SectionScorer::new(s.scoring_tree(), vec![]);
    let sec = scorer.score_section(
        "SEC-01",
        "我方近年完成的类似项目案例覆盖多个行业，全部案例附合同扫描件备查。",
    );
    let it = &sec.item_scores[0];
    assert_eq!(it.rule_checkpoints.len(), 1);
    assert!(
        it.rule_checkpoints[0].mentioned,
        "主题词「类似项目案例」在场 → 提及"
    );
    assert_eq!(it.rule_gaps, 0, "提及 → 无缺口");
    assert!(
        !sec.rationale.contains("可能失分"),
        "无缺口不虚报：{}",
        sec.rationale
    );
}

/// 资质词直配（路②）：长句滑窗组对正文短表述不命中，但条件核心词
/// （一级）在正文在场 → mentioned——窗口 miss + 词面命中的精确验证。
#[test]
fn qualification_word_matches_even_when_window_groups_miss() {
    let rules = "项目负责人具备一级注册建造师执业资格且无在建项目";
    let s = store_base(Some("SEC-01"), Some(rules));
    let scorer = SectionScorer::new(s.scoring_tree(), vec![]);
    let sec = scorer.score_section("SEC-01", "拟派项目经理持一级建造师证书，目前无在手工程。");
    let it = &sec.item_scores[0];
    assert_eq!(
        it.rule_checkpoints.len(),
        1,
        "资质限定词单独成条件：{:?}",
        it.rule_checkpoints
    );
    let cp = &it.rule_checkpoints[0];
    assert_eq!(cp.hits.0, 0, "滑窗组对短表述不命中（先证伪窗口路径）");
    assert!(cp.mentioned, "条件核心词「一级」在正文在场 → 词面命中");
    assert_eq!(it.rule_gaps, 0);
}

/// 数字+计量词代偿（路③）：人员类计量词允许——「项目团队人员不少于
/// 5 人」对正文「常驻人员 30 人」（主题词组必 miss 的实数回应形态）。
#[test]
fn personnel_count_compensation_via_digit_counter() {
    let rules = "项目团队人员不少于 5 人";
    let s = store_base(Some("SEC-01"), Some(rules));
    let scorer = SectionScorer::new(s.scoring_tree(), vec![]);

    // 未提面：无人员数字形态、无主题词 → 明显未提
    let a = scorer.score_section("SEC-01", "项目团队配置完善，各专业工种齐全到位。");
    assert_eq!(a.item_scores[0].rule_checkpoints.len(), 1);
    assert!(
        !a.item_scores[0].rule_checkpoints[0].mentioned,
        "无数字+人 → 未提"
    );
    assert_eq!(a.item_scores[0].rule_gaps, 1);

    // 代偿面：正文以「30 人」实数回应（主题词组「目团队人员不少于」不在场）
    let b = scorer.score_section(
        "SEC-01",
        "项目部常驻人员 30 人，涵盖施工、质量、安全各岗位。",
    );
    assert!(
        b.item_scores[0].rule_checkpoints[0].mentioned,
        "数字+人形态 → 代偿提及（数量真伪不验）"
    );
    assert_eq!(b.item_scores[0].rule_gaps, 0);
    assert!(!b.rationale.contains("可能失分"));
}

/// 泛计量词阻断（负向——防虚报）：正文「3 个专项模块」含数字+个，
/// 但「个」无区分力 → 不代偿「提供 3 个以上类似项目案例」——主题词
/// （案例）不在场仍是明显未提。
#[test]
fn generic_counter_digit_form_does_not_fake_mention() {
    let rules = "提供 3 个以上类似项目案例";
    let s = store_base(Some("SEC-01"), Some(rules));
    let scorer = SectionScorer::new(s.scoring_tree(), vec![]);
    let sec = scorer.score_section("SEC-01", "本次方案共设计 3 个专项模块，形成完整管理闭环。");
    let it = &sec.item_scores[0];
    assert!(
        !it.rule_checkpoints[0].mentioned,
        "「3 个模块」≠「3 个以上案例」——泛计量词不代偿"
    );
    assert_eq!(it.rule_gaps, 1, "案例条件仍明显未提");
    assert!(sec.rationale.contains("可能失分"));
}

// ---------------------------------------------------------------- 文档面 + 序列化

/// 跨章合并（单一真源 merged_*）：同一评分项跨两章（0.6/0.4）——案例
/// 条件只在 A 章提及：逐章行 rule_gaps 各自独立（A=1/B=2）；item 级
/// 合并缺口 = 全部章都未提的条件数（=1，仅项目经理）→ notes 计数
/// 「其中 1 项」（按评分项合并，不逐行重复计）。
#[test]
fn score_document_merges_checkpoints_across_sections() {
    let rules = "提供 3 个以上类似项目案例；配备 5 名专职项目经理；每缺一项扣 2 分";
    let mut s = store_base(None, Some(rules)); // 无派生映射——账本跨两章
    for (sec, share) in [("SEC-01", 0.6), ("SEC-02", 0.4)] {
        s.upsert_scoring_mapping(
            "REQ-0001".into(),
            sec.into(),
            share,
            vec![],
            anchor("4.1 类似项目业绩 20 分，详见评审细则"),
            MappingSource::AgentSubmitted,
        )
        .unwrap();
    }
    let scorer = {
        // 联结（build_scorer 的域层等价面——账本并入派生树，同
        // scoring_mapping.rs 的写法；域层 scoring_tree() 只做派生面）
        let mut tree = bid_domain::scoring::derive_scoring_tree(&s.requirements().to_vec());
        for (_, item) in tree.index.iter_mut() {
            for m in s.scoring_mappings() {
                if m.scoring_req_id == item.requirement_id
                    && !item.mappings.iter().any(|x| x.section_id == m.section_id)
                {
                    item.mappings.push(bid_domain::scoring::SectionMapping {
                        section_id: m.section_id.clone(),
                        weight_share: m.weight_share,
                    });
                }
            }
        }
        SectionScorer::new(tree, vec![])
    };
    let ds = scorer.score_document(
        &[
            (
                "SEC-01".to_string(),
                "我方类似项目案例储备充足，可按评标要求提供备查。".to_string(),
            ),
            (
                "SEC-02".to_string(),
                "售后服务体系完善，响应机制健全。".to_string(),
            ),
        ],
        None,
    );

    // 逐章行：A 提了案例（缺项目经理 1）；B 两条件都未提（缺 2）
    let row = |sec: &str| {
        ds.breakdown
            .iter()
            .find(|b| b.section_id == sec)
            .unwrap()
            .item_scores
            .iter()
            .find(|i| i.item_id == "REQ-0001")
            .unwrap()
            .clone()
    };
    let (ra, rb) = (row("SEC-01"), row("SEC-02"));
    assert_eq!(ra.rule_gaps, 1, "A 章缺项目经理（案例已提）");
    assert_eq!(rb.rule_gaps, 2, "B 章两条件都未提");

    // item 级合并（单一真源）：案例任一章提及 → 不算缺口；项目经理
    // 全部章未提 → 缺口 1
    let rows: Vec<&ItemScore> = ds
        .breakdown
        .iter()
        .flat_map(|b| b.item_scores.iter())
        .filter(|i| i.item_id == "REQ-0001")
        .collect();
    let merged = merged_rule_checkpoints(&rows);
    assert_eq!(merged.len(), 2, "两条检查点（去重合并后）");
    assert!(merged
        .iter()
        .any(|c| c.text.contains("类似项目案例") && c.mentioned));
    assert!(merged
        .iter()
        .any(|c| c.text.contains("名专职项目经理") && !c.mentioned));

    // notes：登记比对发生 + 缺口按评分项合并计数（1 项，不是 2 章）
    let note = ds
        .notes
        .iter()
        .find(|n| n.contains("SCORE_RULES_CHECKPOINTS"))
        .expect("检查点 note 在场");
    assert!(note.contains("1 项已登记评分细则"), "{note:?}");
    assert!(note.contains("其中 1 项"), "缺口按 item 合并计数：{note:?}");
    assert!(note.contains("可能失分"));
}

/// 序列化兼容：旧 ItemScore JSON（#224 第 3 步之前的形态——无
/// rule_checkpoints/rule_gaps 键）可载，新字段取缺省——旧快照/旧消费方
/// 不破；新字段在场时往返保真。
#[test]
fn old_item_score_json_deserializes_with_defaults() {
    let old = serde_json::json!({
        "item_id": "REQ-0001", "estimated": 13.0, "max": 20.0,
        "tier": "Fair", "coverage": 0.6, "is_objective": false,
        "rationale": "r", "weight_share": 1.0
    });
    let v: ItemScore = serde_json::from_value(old).expect("旧 JSON 可载（新字段缺省）");
    assert!(v.rule_checkpoints.is_empty(), "缺省空检查点");
    assert_eq!(v.rule_gaps, 0, "缺省无缺口");
    assert_eq!(v.tier, GradeTier::Fair);

    // 新字段在场：往返保真（含 hits 元组 → [h, t] 数组形态）
    let full = ItemScore {
        item_id: "REQ-0001".into(),
        estimated: 13.0,
        max: 20.0,
        tier: GradeTier::Fair,
        coverage: 0.6,
        is_objective: false,
        rationale: "r".into(),
        weight_share: 0.6,
        score_rules: Some("提供 3 个以上类似项目案例".into()),
        rule_checkpoints: vec![RuleCheckpoint {
            text: "提供 3 个以上类似项目案例".into(),
            hits: (1, 2),
            mentioned: false,
        }],
        rule_gaps: 1,
    };
    let rt: ItemScore = serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
    assert_eq!(rt, full, "往返保真");
    let raw = serde_json::to_value(&full).unwrap();
    assert_eq!(
        raw["rule_checkpoints"][0]["hits"],
        serde_json::json!([1, 2]),
        "hits 序列化为二元数组"
    );
}
