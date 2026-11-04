//! #224 第 1 步 · ScoringMapping 多对多映射 + UNMAPPED 域层验收。
//!
//! 覆盖面（issue #224 验收 2/3/4 + 快照面）：
//! - **多对多**：一个评分项跨 2 章节（weight_share 0.6/0.4）→ 两章
//!   score_section 都查得到它，满分按 share 摊分（12/8）——同一项跨章
//!   各算一次导致总分虚高的主防；
//! - **一章节承载多评分项**：item_scores 逐项呈现；
//! - **UNMAPPED 结构化**：unmapped_scoring_items（req_id/criteria/weight）
//!   + notes 响亮告警；SectionScore/ItemScore 的 share 摊分回归；
//! - **账本**：upsert 幂等 + 越界/孤儿/幽灵证据 fail-loud 拒；
//! - **快照**：往返保真 + 旧快照（无 scoring_mappings 键）空集合加载 +
//!   孤儿映射/复合键重复拒载；
//! - **派生联结**：store.scoring_items() 并入持久化映射（与 params.section
//!   去重）。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::scoring::{derive_scoring_items, MappingSource, ScoringMapping, SectionScorer};
use bid_domain::{Anchor, DomainStore, Requirement, StoreSnapshot, ToolError};
use std::collections::HashMap;

fn anchor(excerpt: &str) -> Anchor {
    Anchor {
        file: "A-招标文件.docx".into(),
        kind: "para".into(),
        locator: "第四章 评标办法".into(),
        excerpt: excerpt.into(),
    }
}

fn score_req(id: &str, excerpt: &str, score: f64, section: Option<&str>) -> Requirement {
    let mut params = HashMap::new();
    params.insert("score".to_string(), serde_json::json!(score));
    if let Some(s) = section {
        params.insert("section".to_string(), serde_json::json!(s));
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

/// 正文类 REQ（叶节点树不变量：每叶 ≥1 REQ——评分项不进正文 #126，
/// 用技术参数承担叶子绑定职责）。
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

/// 基座 store：项目 + 文件 + 评分项 + 两叶章节（SEC-01 / SEC-02）。
fn store_base() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("映射测试", "C-M", "明标", "m-key")
        .unwrap();
    s.upsert_file(
        "A-招标文件.docx",
        "招标文件",
        "sha-map",
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
            "4.1 技术方案完整性与针对性 20 分（方案须覆盖架构与选型）",
            20.0,
            None,
        ),
        score_req(
            "REQ-0002",
            "4.2 售后服务方案 10 分（含响应时限承诺）",
            10.0,
            None,
        ),
        body_req("REQ-0003", "5.1 核心交换机须支持 48 口千兆光接口"),
        body_req("REQ-0004", "5.2 售后响应时限 ≤ 2 小时"),
    ])
    .unwrap();
    s.set_outline(vec![
        leaf("SEC-01", 1, "技术方案", &["REQ-0003"]),
        leaf("SEC-02", 2, "售后服务", &["REQ-0004"]),
    ])
    .unwrap();
    s
}

fn upsert(s: &mut DomainStore, req: &str, sec: &str, share: f64) -> ScoringMapping {
    s.upsert_scoring_mapping(
        req.to_string(),
        sec.to_string(),
        share,
        vec![],
        anchor("4.1 技术方案完整性……（规则原文锚）"),
        MappingSource::AgentSubmitted,
    )
    .unwrap()
}

// ---------------------------------------------------------------- 多对多（验收 2）

/// 一个评分项跨 2 章节（0.6/0.4）→ 两章都查得到；满分按 share 摊分（12/8）。
#[test]
fn one_item_across_two_sections_shares_max() {
    let mut s = store_base();
    upsert(&mut s, "REQ-0001", "SEC-01", 0.6);
    upsert(&mut s, "REQ-0001", "SEC-02", 0.4);

    let mut tree_nodes = bid_domain::scoring::derive_scoring_tree(&s.requirements().to_vec());
    // 联结（build_scorer 的域层等价面）
    for (_, item) in tree_nodes.index.iter_mut() {
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
    let scorer = SectionScorer::new(tree_nodes, vec![]);
    let content =
        "技术方案：系统架构采用三层设计，设备选型经过充分比选论证，方案完整且针对本项目特点。";

    let a = scorer.score_section("SEC-01", content);
    let b = scorer.score_section("SEC-02", content);
    assert_eq!(a.item_scores.len(), 1, "SEC-01 含跨章评分项");
    assert_eq!(b.item_scores.len(), 1, "SEC-02 也含同一评分项（多对多）");
    assert_eq!(a.item_scores[0].item_id, "REQ-0001");
    assert_eq!(b.item_scores[0].item_id, "REQ-0001");
    // 满分摊分：20 × 0.6 = 12；20 × 0.4 = 8
    assert!(
        (a.item_scores[0].max - 12.0).abs() < 1e-9,
        "SEC-01 满分 = 20×0.6"
    );
    assert!(
        (b.item_scores[0].max - 8.0).abs() < 1e-9,
        "SEC-02 满分 = 20×0.4"
    );
    assert!((a.item_scores[0].weight_share - 0.6).abs() < 1e-9);
    assert!((b.item_scores[0].weight_share - 0.4).abs() < 1e-9);
    // estimated 同比摊分（同内容同档位 → 12r / 8r）
    let ra = a.item_scores[0].estimated;
    let rb = b.item_scores[0].estimated;
    assert!(
        (ra * 0.4 - rb * 0.6).abs() < 0.02,
        "estimated 按 share 同比摊分：{ra}×0.4 ≈ {rb}×0.6"
    );
    // 章满分聚合也是摊分后的
    assert!((a.max - 12.0).abs() < 1e-9);
    assert!((b.max - 8.0).abs() < 1e-9);
    // rationale 摊分说明在场（非 1.0 share 才注明）
    assert!(
        a.item_scores[0].rationale.contains("摊分"),
        "rationale 带摊分说明"
    );
}

/// 一章节承载 2 评分项 → item_scores = 2。
#[test]
fn one_section_carries_two_items() {
    let mut s = store_base();
    upsert(&mut s, "REQ-0001", "SEC-01", 1.0);
    upsert(&mut s, "REQ-0002", "SEC-01", 1.0);
    let mut tree_nodes = bid_domain::scoring::derive_scoring_tree(&s.requirements().to_vec());
    for (_, item) in tree_nodes.index.iter_mut() {
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
    let scorer = SectionScorer::new(tree_nodes, vec![]);
    let r = scorer.score_section("SEC-01", "技术方案与售后服务均完整覆盖。");
    assert_eq!(r.item_scores.len(), 2, "一章节承载 2 评分项");
    assert!(r.max > 0.0);
}

/// 单映射（params.section / share=1.0）与历史行为逐位一致（零回归锚）。
#[test]
fn single_mapping_share_one_matches_history() {
    let reqs = vec![score_req(
        "REQ-0009",
        "4.9 项目经理业绩 5 分（近三年类似项目）",
        5.0,
        Some("SEC-07"),
    )];
    let items = derive_scoring_items(&reqs);
    assert_eq!(items[0].mappings.len(), 1);
    assert_eq!(items[0].mappings[0].section_id, "SEC-07");
    assert!((items[0].mappings[0].weight_share - 1.0).abs() < 1e-9);
}

// ---------------------------------------------------------------- UNMAPPED（验收 2 附带）

/// 无映射评分项 → unmapped_scoring_items 结构化（req_id/criteria/weight）
/// + notes 响亮告警。
#[test]
fn unmapped_items_structured_and_loud() {
    let mut s = store_base(); // REQ-0001/0002 均无 params.section、无账本映射
    let tree_nodes = bid_domain::scoring::derive_scoring_tree(&s.requirements().to_vec());
    let scorer = SectionScorer::new(tree_nodes, vec![]);
    let doc = scorer.score_document(&[("SEC-01".into(), "写了一点技术方案内容。".into())], None);
    assert_eq!(doc.unmapped_scoring_items.len(), 2, "两个评分项均未映射");
    let u = &doc.unmapped_scoring_items[0];
    assert_eq!(u.req_id, "REQ-0001");
    assert!(!u.criteria.is_empty(), "criteria 摘要在场");
    assert!((u.weight.unwrap() - 20.0).abs() < 1e-9, "weight 在场");
    let joined = doc.notes.join("\n");
    assert!(joined.contains("UNMAPPED"), "notes 保留字符串告警面");
    assert!(joined.contains("总分可能偏低"), "响亮告警语义");
    // 单章自评也可见文档级缺口
    let sec = scorer.score_section("SEC-01", "内容");
    let _ = sec; // （unmapped 在 MCP 层出参——域层 SectionScore 不带，文档级才带）
}

// ---------------------------------------------------------------- 账本（upsert 校验）

/// upsert 幂等：同 (req, section) 重提 = 更新；weight_share 越界拒。
#[test]
fn upsert_idempotent_and_validates() {
    let mut s = store_base();
    let m1 = upsert(&mut s, "REQ-0001", "SEC-01", 0.6);
    assert!(m1.id.starts_with("SM-"), "SM-xxxx 签发");
    let m2 = upsert(&mut s, "REQ-0001", "SEC-01", 1.0); // 同键 → 更新非身份字段（单行 1.0 合法）
    assert_eq!(m1.id, m2.id, "幂等键命中不重签 id");
    assert_eq!(s.scoring_mappings().len(), 1);
    assert!((s.scoring_mappings()[0].weight_share - 1.0).abs() < 1e-9);
    // 聚合不变量（Lead 2026-11-04 blocker 1）：先降回 0.6 腾出份额，
    // 再追加 SEC-02（0.6+0.4=1.0 合法）——旧版的 1.0+0.4 会撞新校验
    upsert(&mut s, "REQ-0001", "SEC-01", 0.6);
    upsert(&mut s, "REQ-0001", "SEC-02", 0.4); // 新键 → 追加
    assert_eq!(s.scoring_mappings().len(), 2);

    // weight_share 越界（0 / >1）拒
    for bad in [0.0_f64, 1.5, -0.1] {
        let e = s
            .upsert_scoring_mapping(
                "REQ-0002".into(),
                "SEC-01".into(),
                bad,
                vec![],
                anchor("x"),
                MappingSource::Manual,
            )
            .unwrap_err();
        assert!(
            matches!(e, ToolError::EUnprocessable { .. }),
            "share={bad} 应拒"
        );
    }
    // 评分项不存在拒
    let e = s
        .upsert_scoring_mapping(
            "REQ-9999".into(),
            "SEC-01".into(),
            1.0,
            vec![],
            anchor("x"),
            MappingSource::Manual,
        )
        .unwrap_err();
    assert!(matches!(e, ToolError::ENotFound { .. }));
    // 非评分面类别拒（映射对其他类别无意义）——全新 REQ id，
    // 避开 set_requirements 的幂等合并（同锚旧记录不覆盖类别）
    let mut s2 = store_base();
    s2.set_requirements(vec![body_req("REQ-0500", "6.1 质保期三年（全新锚）")])
        .unwrap();
    let e = s2
        .upsert_scoring_mapping(
            "REQ-0500".into(),
            "SEC-01".into(),
            1.0,
            vec![],
            anchor("x"),
            MappingSource::Manual,
        )
        .unwrap_err();
    assert!(matches!(e, ToolError::EUnprocessable { .. }));
    // 章节不存在拒
    let e = s
        .upsert_scoring_mapping(
            "REQ-0001".into(),
            "SEC-9999".into(),
            1.0,
            vec![],
            anchor("x"),
            MappingSource::Manual,
        )
        .unwrap_err();
    assert!(matches!(e, ToolError::ENotFound { .. }));
    // 幽灵证据拒
    let e = s
        .upsert_scoring_mapping(
            "REQ-0001".into(),
            "SEC-01".into(),
            1.0,
            vec!["EV-9999".into()],
            anchor("x"),
            MappingSource::Manual,
        )
        .unwrap_err();
    assert!(matches!(e, ToolError::ENotFound { .. }));
}

/// 派生联结：store.scoring_items() 并入持久化映射（与 params.section 去重）。
#[test]
fn persistent_mappings_join_derived_items() {
    let mut s = store_base();
    // REQ-0002 有 params.section（RuleParsed 单值）；REQ-0001 无
    let mut reqs = s.requirements().to_vec();
    reqs[1]
        .params
        .insert("section".into(), serde_json::json!("SEC-02"));
    s.set_requirements(reqs).unwrap();
    upsert(&mut s, "REQ-0001", "SEC-01", 0.6);
    upsert(&mut s, "REQ-0001", "SEC-02", 0.4); // 多对多：账本两条
    let items = s.scoring_items();
    let it1 = items
        .iter()
        .find(|i| i.requirement_id == "REQ-0001")
        .unwrap();
    assert_eq!(it1.mappings.len(), 2, "账本两条映射并入");
    assert_eq!(it1.mappings[0].section_id, "SEC-01");
    assert_eq!(it1.mappings[1].section_id, "SEC-02");
    let it2 = items
        .iter()
        .find(|i| i.requirement_id == "REQ-0002")
        .unwrap();
    assert_eq!(it2.mappings.len(), 1, "params.section 派生映射保留");
    assert_eq!(it2.mappings[0].section_id, "SEC-02");
    // 去重：对 REQ-0002 的 (REQ-0002, SEC-02) 再登记 → 派生联结不双计
    upsert(&mut s, "REQ-0002", "SEC-02", 1.0);
    let items = s.scoring_items();
    let it2 = items
        .iter()
        .find(|i| i.requirement_id == "REQ-0002")
        .unwrap();
    assert_eq!(
        it2.mappings.len(),
        1,
        "同 (req, section) 键不重复入 mappings"
    );
}

// ---------------------------------------------------------------- serde / 快照

/// MappingSource 三值 SCREAMING_SNAKE_CASE 往返。
#[test]
fn mapping_source_serde_roundtrip() {
    for (v, lit) in [
        (MappingSource::RuleParsed, "RULE_PARSED"),
        (MappingSource::AgentSubmitted, "AGENT_SUBMITTED"),
        (MappingSource::Manual, "MANUAL"),
    ] {
        let j = serde_json::to_string(&v).unwrap();
        assert_eq!(j, format!("\"{lit}\""));
        let back: MappingSource = serde_json::from_str(&j).unwrap();
        assert_eq!(back, v);
    }
}

// ---------------------------------------------------------------- Lead review blockers（2026-11-04）

/// 三叶夹具：1 评分项 + 3 技术参数叶（SEC-01/02/03）——聚合份额多行场景。
fn store_three_leaves() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("聚合份额", "C-SUM", "明标", "sum-key")
        .unwrap();
    s.upsert_file(
        "A-招标文件.docx",
        "招标文件",
        "sha-sum",
        10,
        1,
        50,
        0,
        bid_domain::ParseStatus::Ok,
        "",
    )
    .unwrap();
    s.set_requirements(vec![
        score_req("REQ-0001", "4.1 技术方案 20 分", 20.0, None),
        body_req("REQ-0003", "5.1 甲接口"),
        body_req("REQ-0004", "5.2 乙接口"),
        body_req("REQ-0005", "5.3 丙接口"),
    ])
    .unwrap();
    s.set_outline(vec![
        leaf("SEC-01", 1, "技术方案一", &["REQ-0003"]),
        leaf("SEC-02", 2, "技术方案二", &["REQ-0004"]),
        leaf("SEC-03", 3, "技术方案三", &["REQ-0005"]),
    ])
    .unwrap();
    s
}

/// 聚合不变量——新增路径：0.7+0.7 拒（单项贡献 140% 的主防，零副作用）；
/// 恰好 1.0 放行；浮点和 1.0000000000000002 在 epsilon 内放行。
#[test]
fn aggregate_share_rejects_overshoot_on_insert() {
    let mut s = store_three_leaves();
    upsert(&mut s, "REQ-0001", "SEC-01", 0.7);
    // 0.7 + 0.7 = 1.4 → 拒
    let e = s
        .upsert_scoring_mapping(
            "REQ-0001".into(),
            "SEC-02".into(),
            0.7,
            vec![],
            anchor("x"),
            MappingSource::Manual,
        )
        .unwrap_err();
    assert!(
        matches!(e, ToolError::EUnprocessable { .. }),
        "0.7+0.7 必须拒（Lead blocker 1：单项贡献不得超满分）"
    );
    assert!(
        format!("{e:?}").contains("≤ 1.0"),
        "报错须给出总量纪律：{e:?}"
    );
    assert_eq!(s.scoring_mappings().len(), 1, "被拒后零副作用");
    // 0.7 + 0.3 = 1.0 → 放行（边界）
    upsert(&mut s, "REQ-0001", "SEC-02", 0.3);
    // 0.7 + 0.3 + 0.1 = 1.1 → 拒
    let e = s
        .upsert_scoring_mapping(
            "REQ-0001".into(),
            "SEC-03".into(),
            0.1,
            vec![],
            anchor("x"),
            MappingSource::Manual,
        )
        .unwrap_err();
    assert!(matches!(e, ToolError::EUnprocessable { .. }), "1.1 必须拒");
    assert_eq!(s.scoring_mappings().len(), 2, "被拒后零副作用");
    // 浮点边界：0.1 + 0.2 + 0.7 = 1.0000000000000002 ≤ 1.0+1e-9 → 全部放行
    let mut f = store_three_leaves();
    upsert(&mut f, "REQ-0001", "SEC-01", 0.1);
    upsert(&mut f, "REQ-0001", "SEC-02", 0.2);
    upsert(&mut f, "REQ-0001", "SEC-03", 0.7);
    assert_eq!(f.scoring_mappings().len(), 3, "epsilon 内的 1.0 放行");
}

/// 聚合不变量——更新路径：排除被替换行后校验（0.7+0.3 合法态原值重提
/// 不得被误拒）；上调致超限拒且零副作用。
#[test]
fn aggregate_share_rejects_overshoot_on_update() {
    let mut s = store_three_leaves();
    upsert(&mut s, "REQ-0001", "SEC-01", 0.7);
    upsert(&mut s, "REQ-0001", "SEC-02", 0.3);
    // 上调 B：0.7 + 0.5 = 1.2 → 拒；B 保持 0.3（零副作用）
    let e = s
        .upsert_scoring_mapping(
            "REQ-0001".into(),
            "SEC-02".into(),
            0.5,
            vec![],
            anchor("x"),
            MappingSource::Manual,
        )
        .unwrap_err();
    assert!(matches!(e, ToolError::EUnprocessable { .. }), "0.7+0.5 必须拒");
    let b = s
        .scoring_mappings()
        .iter()
        .find(|m| m.section_id == "SEC-02")
        .unwrap();
    assert!((b.weight_share - 0.3).abs() < 1e-9, "被拒后原值不变");
    // 原值重提：排除被替换行 → 0.7+0.3=1.0 放行（不排除则 1.3 误拒）
    upsert(&mut s, "REQ-0001", "SEC-02", 0.3);
    // 先降再升的合法调整链：A 0.7→0.6（0.9）→ B 0.3→0.4（1.0）
    upsert(&mut s, "REQ-0001", "SEC-01", 0.6);
    upsert(&mut s, "REQ-0001", "SEC-02", 0.4);
    assert_eq!(s.scoring_mappings().len(), 2);
}

/// 派生 params.section（RuleParsed 单值 share=1.0）计入有效份额：异节
/// 账本行必超 1.0 → 拒；同节入账为联结去重的 inert 行（总量仍 1.0）→ 放行。
#[test]
fn derived_section_mapping_counts_into_sum() {
    let mut s = DomainStore::new();
    s.create_project("派生份额", "C-DER", "明标", "der-key")
        .unwrap();
    s.upsert_file(
        "A-招标文件.docx",
        "招标文件",
        "sha-der",
        10,
        1,
        50,
        0,
        bid_domain::ParseStatus::Ok,
        "",
    )
    .unwrap();
    s.set_requirements(vec![
        score_req("REQ-0001", "4.1 技术方案 20 分", 20.0, Some("SEC-01")),
        body_req("REQ-0003", "5.1 甲接口"),
        body_req("REQ-0004", "5.2 乙接口"),
    ])
    .unwrap();
    s.set_outline(vec![
        leaf("SEC-01", 1, "技术方案一", &["REQ-0003"]),
        leaf("SEC-02", 2, "技术方案二", &["REQ-0004"]),
    ])
    .unwrap();
    // 派生 (SEC-01, 1.0) 恒在联结面 → 异节 SEC-02 任何正份额都超限
    let e = s
        .upsert_scoring_mapping(
            "REQ-0001".into(),
            "SEC-02".into(),
            0.4,
            vec![],
            anchor("x"),
            MappingSource::Manual,
        )
        .unwrap_err();
    assert!(
        matches!(e, ToolError::EUnprocessable { .. }),
        "派生 1.0 + 异节 0.4 必须拒（联结面总量 1.4）"
    );
    assert_eq!(s.scoring_mappings().len(), 0, "被拒后零副作用");
    // 同节入账：联结去重（派生赢）→ 总量仍 1.0 → 放行（账本仅记录）
    upsert(&mut s, "REQ-0001", "SEC-01", 1.0);
    assert_eq!(s.scoring_mappings().len(), 1);
}

/// 引用完整性——条款重建（Lead blocker 2）：set_requirements 的同哈希
/// 换 id 覆盖与 replace_requirements 的整集替换都不得移走被映射的评分项
/// （孤儿映射一经 checkpoint 落盘，下一次 restore 拒载——快照卡死）。
#[test]
fn requirement_rebuild_rejects_dropping_mapped_req() {
    let mut s = store_base();
    upsert(&mut s, "REQ-0001", "SEC-01", 1.0);

    // set_requirements 幂等合并面：同锚同文克隆换 id（哈希命中 → 覆盖），
    // 被映射的 REQ-0001 将消失 → 拒
    let drifted = score_req(
        "REQ-0099",
        "4.1 技术方案完整性与针对性 20 分（方案须覆盖架构与选型）",
        20.0,
        None,
    );
    let e = s.set_requirements(vec![drifted]).unwrap_err();
    assert!(
        matches!(e, ToolError::EUnprocessable { .. }),
        "换 id 覆盖被映射评分项必须拒"
    );
    assert!(
        s.requirements().iter().any(|r| r.id == "REQ-0001"),
        "被拒后条款集不变"
    );
    assert_eq!(s.scoring_mappings().len(), 1, "被拒后映射账本不变");

    // replace_requirements 整集替换：缺 REQ-0001 → 拒
    let e = s
        .replace_requirements(vec![
            score_req("REQ-0002", "4.2 售后服务方案 10 分（含响应时限承诺）", 10.0, None),
            body_req("REQ-0003", "5.1 核心交换机须支持 48 口千兆光接口"),
            body_req("REQ-0004", "5.2 售后响应时限 ≤ 2 小时"),
        ])
        .unwrap_err();
    assert!(
        matches!(e, ToolError::EUnprocessable { .. }),
        "整集替换移走被映射评分项必须拒"
    );
    assert!(
        s.requirements().iter().any(|r| r.id == "REQ-0001"),
        "被拒后条款集不变"
    );
    assert_eq!(s.scoring_mappings().len(), 1, "被拒后映射账本不变");

    // 保留被映射 id 的替换 → 放行（守卫只拦破坏性面）
    let n = s
        .replace_requirements(vec![
            score_req(
                "REQ-0001",
                "4.1 技术方案完整性与针对性 20 分（方案须覆盖架构与选型）",
                20.0,
                None,
            ),
            score_req("REQ-0002", "4.2 售后服务方案 10 分（含响应时限承诺）", 10.0, None),
        ])
        .unwrap();
    assert_eq!(n, 2, "保留被映射 id 的替换放行");
}

/// 引用完整性——大纲重挂（Lead blocker 2）：移走被映射章节必须拒
/// （SEC id 随树重签；保留被映射章节的重挂放行）。
#[test]
fn outline_rebuild_rejects_dropping_mapped_section() {
    let mut s = store_base();
    upsert(&mut s, "REQ-0001", "SEC-01", 1.0);

    // 重挂只含 SEC-02 的树 → SEC-01 将消失 → 拒
    let e = s
        .set_outline(vec![leaf("SEC-02", 1, "售后服务", &["REQ-0004"])])
        .unwrap_err();
    assert!(
        matches!(e, ToolError::EUnprocessable { .. }),
        "重挂移走被映射章节必须拒"
    );
    assert_eq!(s.sections().len(), 2, "被拒后大纲不变");
    assert_eq!(s.scoring_mappings().len(), 1, "被拒后映射账本不变");

    // 保留被映射章节的重挂（改 title/target_chars）→ 放行
    s.set_outline(vec![
        leaf("SEC-01", 1, "技术方案（改）", &["REQ-0003"]),
        leaf("SEC-02", 2, "售后服务（改）", &["REQ-0004"]),
    ])
    .unwrap();
    assert_eq!(s.sections().len(), 2, "保留被映射章节的重挂放行");
}

/// restore 行约束与登记面同源（Lead blocker 2）：份额越界与幽灵证据
/// 引用拒载；合法快照照常恢复。
#[test]
fn restore_enforces_registration_row_constraints() {
    let mut s = store_base();
    upsert(&mut s, "REQ-0001", "SEC-01", 1.0);

    // 份额越界（登记面拒的，快照面同样拒）
    let mut bad_share = s.to_snapshot();
    bad_share.scoring_mappings[0].weight_share = 1.5;
    assert!(matches!(
        DomainStore::restore(bad_share),
        Err(ToolError::EUnprocessable { .. })
    ));

    // 幽灵证据引用
    let mut ghost_ev = s.to_snapshot();
    ghost_ev.scoring_mappings[0].evidence_refs = vec!["EV-9999".into()];
    assert!(matches!(
        DomainStore::restore(ghost_ev),
        Err(ToolError::EUnprocessable { .. })
    ));

    // 合法快照照常恢复
    let ok = DomainStore::restore(s.to_snapshot()).unwrap();
    assert_eq!(ok.scoring_mappings().len(), 1);
}

/// content_version 语义（Lead blocker 3）：真变更（新增 / 任一字段更新）
/// bump；全字段等值重提（真 no-op）不 bump——映射影响评分结果，乐观并发
/// 面不得「版本没变但评分面变了」。
#[test]
fn mapping_mutation_bumps_content_version_but_noop_does_not() {
    let mut s = store_base();
    let v0 = s.content_version();

    // 新增 → +1
    let m1 = upsert(&mut s, "REQ-0001", "SEC-01", 0.6);
    assert_eq!(s.content_version(), v0 + 1, "新增映射 bump");

    // 全字段等值重提（同键同值同锚同来源）→ 真 no-op，不 bump
    let m2 = upsert(&mut s, "REQ-0001", "SEC-01", 0.6);
    assert_eq!(m1.id, m2.id, "幂等键命中不重签 id");
    assert_eq!(s.content_version(), v0 + 1, "等值重提不 bump");
    assert_eq!(s.scoring_mappings().len(), 1, "等值重提不追加行");

    // 真更新（share 变）→ +1
    upsert(&mut s, "REQ-0001", "SEC-01", 0.7);
    assert_eq!(s.content_version(), v0 + 2, "share 更新 bump");

    // 真更新（created_from 变，share 不变）→ +1（非身份字段也算变更）
    s.upsert_scoring_mapping(
        "REQ-0001".into(),
        "SEC-01".into(),
        0.7,
        vec![],
        anchor("4.1 技术方案完整性……（规则原文锚）"),
        MappingSource::Manual,
    )
    .unwrap();
    assert_eq!(s.content_version(), v0 + 3, "created_from 更新 bump");

    // created_from 回位（又变）→ bump；随后终态等值重提 → 不 bump
    upsert(&mut s, "REQ-0001", "SEC-01", 0.7);
    assert_eq!(s.content_version(), v0 + 4, "created_from 回位也 bump");
    upsert(&mut s, "REQ-0001", "SEC-01", 0.7);
    assert_eq!(s.content_version(), v0 + 4, "终态等值重提不 bump");
}

/// 快照往返 + 旧快照（无 scoring_mappings 键）空集合加载 + 孤儿/复合键重复拒载。
#[test]
fn mapping_snapshot_roundtrip_and_old_snapshot_loads() {
    let mut s = store_base();
    upsert(&mut s, "REQ-0001", "SEC-01", 0.6);
    upsert(&mut s, "REQ-0001", "SEC-02", 0.4);

    // 往返保真
    let snap = s.to_snapshot();
    assert_eq!(snap.scoring_mappings.len(), 2);
    let restored = DomainStore::restore(snap).unwrap();
    assert_eq!(restored.scoring_mappings(), s.scoring_mappings());

    // 旧快照（无 scoring_mappings 字段）→ serde(default) 空集合加载
    let mut json = serde_json::to_value(s.to_snapshot()).unwrap();
    json.as_object_mut().unwrap().remove("scoring_mappings");
    let old_snap: StoreSnapshot = serde_json::from_value(json).unwrap();
    assert!(old_snap.scoring_mappings.is_empty());
    let old_store = DomainStore::restore(old_snap).unwrap();
    assert!(old_store.scoring_mappings().is_empty());

    // 孤儿映射（章节不存在）拒载
    let mut bad = s.to_snapshot();
    bad.scoring_mappings.push(ScoringMapping {
        id: "SM-9999".into(),
        scoring_req_id: "REQ-0001".into(),
        section_id: "SEC-9999".into(),
        weight_share: 1.0,
        evidence_refs: vec![],
        source_anchor: anchor("x"),
        created_from: MappingSource::Manual,
    });
    assert!(matches!(
        DomainStore::restore(bad),
        Err(ToolError::EUnprocessable { .. })
    ));

    // 复合键 (req, section) 重复拒载（幂等面破坏）
    let mut dup = s.to_snapshot();
    dup.scoring_mappings.push(ScoringMapping {
        id: "SM-8888".into(),
        scoring_req_id: "REQ-0001".into(),
        section_id: "SEC-01".into(),
        weight_share: 0.1,
        evidence_refs: vec![],
        source_anchor: anchor("x"),
        created_from: MappingSource::Manual,
    });
    assert!(matches!(
        DomainStore::restore(dup),
        Err(ToolError::EUnprocessable { .. })
    ));

    // SM id 序列入水位：快照 id_seq 低水位拒载
    let mut low = s.to_snapshot();
    low.id_seq = 0;
    assert!(matches!(
        DomainStore::restore(low),
        Err(ToolError::EUnprocessable { .. })
    ));
}
