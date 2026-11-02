//! scoring_b2_self_eval.rs —— B2 评分体系重构 + 自评的集成契约（issue #94）。
//!
//! 消费面视角（B3 质量检查 / B4 生成循环将以本链路取数）：
//!   DomainStore（requirements 落库）→ scoring_tree() 派生 →
//!   SectionScorer::score_document（技术/商务/价格三段聚合）→ DocumentScore。
//!
//! 验证词典（issue #94 §五验收）：
//!   1. store 派生 = 纯函数派生（单一真源——幂等，无落库态分裂）；
//!   2. 未映射项 / 缺章节 / 未报价 → notes 诚实登记（不伪装 0 分满分）；
//!   3. 理想标书自评 vs 手工（<10%——档位中点口径全链可复算）；
//!   4. 凑字数章节显著低分（档位差距——自评的区分力）；
//!   5. B1 价格分接线（PriceScoreSpec 只调用不改）；
//!   6. rationale 全链可追溯（四维明细 + 档位 + 估分算式）。

use bid_domain::pricing::{DeviationRule, PriceScoreMethod, PriceScoreSpec};
use bid_domain::scoring::{derive_scoring_tree, PriceInput, SectionScorer};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use std::collections::HashMap;

/// 评分表：技术 30（三主观子项各 10）/ 商务 20（两客观子项各 10）/
/// 价格 40 + 否决项（硬门）+ 一项故意不映射 section（UNMAPPED 面）。
fn requirement(id: &str, score: Option<f64>, parent: Option<&str>, section: Option<&str>,
               criteria: &str) -> Requirement {
    let mut params = HashMap::new();
    if let Some(s) = score {
        params.insert("score".to_string(), serde_json::json!(s));
    }
    if let Some(p) = parent {
        params.insert("parent".to_string(), serde_json::json!(p));
    }
    if let Some(s) = section {
        params.insert("section".to_string(), serde_json::json!(s));
    }
    Requirement {
        id: id.into(),
        category: if score.is_none() { "否决项".into() } else { "评分项".into() },
        clause_no: None,
        anchors: vec![Anchor {
            file: "tender.docx".into(),
            kind: "para".into(),
            locator: "第四章 评标办法".into(),
            excerpt: criteria.into(),
        }],
        params,
        source_file: "tender.docx".into(),
    }
}

fn scoring_table_reqs() -> Vec<Requirement> {
    vec![
        requirement("T-TECH", Some(30.0), None, None, "技术分（30 分）"),
        requirement("T1", Some(10.0), Some("T-TECH"), Some("SEC-01"), "技术方案完整性与针对性 10 分"),
        requirement("T2", Some(10.0), Some("T-TECH"), Some("SEC-02"), "项目实施方案与进度保障措施 10 分"),
        requirement("T3", Some(10.0), Some("T-TECH"), Some("SEC-03"), "售后服务方案（含响应时限承诺）10 分"),
        requirement("C-COMM", Some(20.0), None, None, "商务分（20 分）"),
        requirement("C1", Some(10.0), Some("C-COMM"), Some("SEC-10"), "类似工程业绩，每提供 1 个得 2 分，最多 5 个"),
        requirement("C2", Some(10.0), Some("C-COMM"), Some("SEC-10"),
            "企业具备 ISO9001 质量管理体系认证及 ISO27001 信息安全管理体系认证"),
        // 故意不映射（UNMAPPED 面的登记验证）
        requirement("C3", Some(5.0), Some("C-COMM"), None, "银行资信等级 AAA 级 5 分"),
        requirement("P-PRICE", Some(40.0), None, None,
            "价格分（40 分）：有效投标报价算术平均值下浮 5% 为基准价，价格分＝基准价／投标报价×40"),
        requirement("V1", None, None, None, "投标文件未经法定代表人签字的将被否决"),
    ]
}

fn store_with_scoring_table() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("智慧病房物联网平台", "ZHYY-2026-GK-018", "明标", "it-key")
        .unwrap();
    s.upsert_file("tender.docx", "招标文件", "sha-b2", 12, 3, 240, 0, ParseStatus::Ok, "")
        .unwrap();
    let (kept, _) = s.set_requirements(scoring_table_reqs()).unwrap();
    assert_eq!(kept, 10, "十条 REQ 全量写入");
    s
}

/// 优档章节（关键词全覆盖 + 字数达标 + 证据 + 具体性）。
fn ideal_section(keywords: &[&str], min_len: usize) -> String {
    let mut s = format!(
        "本章节围绕某市第一人民医院智慧病房项目（ZHYY-2026-GK-018）展开：{}。",
        keywords.join("、")
    );
    s.push_str("覆盖 6 个病区 320 张床位，部署物联网网关 180 台，核心链路可用性 99.9%。");
    s.push_str("同类项目业绩与合同证明材料详见附件（编号 EV-2023-001 至 EV-2023-005）。");
    while s.chars().filter(|c| !c.is_whitespace()).count() < min_len {
        s.push_str("上述安排已在本单位同类项目中形成标准化执行路径并持续改进。");
    }
    s
}

fn filler_section(n: usize) -> String {
    let mut s = String::new();
    while s.chars().count() < n {
        s.push_str("我司高度重视本项目，将组建优秀团队，采用先进理念，科学管理，精心组织，确保圆满履约。");
    }
    s
}

/// 词典 1：store 树派生 = 纯函数派生（单一真源，幂等）。
#[test]
fn store_tree_derivation_is_single_source_of_truth() {
    let s = store_with_scoring_table();
    let tree_a = s.scoring_tree();
    let tree_b = s.scoring_tree();
    assert_eq!(tree_a.roots.len(), tree_b.roots.len(), "幂等");
    // 与纯函数派生同构（同一 requirements 输入——index 键集合全等）
    let direct = derive_scoring_tree(&scoring_table_reqs());
    let keys = |t: &bid_domain::scoring::ScoringTree| {
        let mut v: Vec<String> = t.index.keys().cloned().collect();
        v.sort();
        v
    };
    assert_eq!(keys(&tree_a), keys(&direct), "store 派生 = 纯函数派生（单一真源）");
    // 层级对照：T1 的 parent = T-TECH；C1/C2/C3 的 parent = C-COMM
    assert_eq!(tree_a.index["T1"].parent_id.as_deref(), Some("T-TECH"));
    assert_eq!(tree_a.index["C3"].parent_id.as_deref(), Some("C-COMM"));
    // 硬门进树但不计分（否决项无分值）
    assert!(tree_a.index.contains_key("V1"));
    assert!(s.hard_gate_requirement_ids().contains(&"V1".to_string()));
    // 满分聚合：子和为准（商务 10+10+5=25 ≠ 声明 20 → validate 如实报）
    let issues = tree_a.validate_sums();
    assert!(issues.iter().any(|i| i.contains("C-COMM") && i.contains("25")),
        "子和 25 ≠ 声明 20 如实登记：{issues:?}");
}

/// 词典 2+3：理想标书（一章凑字数 + 缺一章 + 未映射一项 + 未报价）→
/// 诚实登记 + 手工 vs 引擎 <10%。
#[test]
fn document_self_eval_honest_notes_and_manual_agreement() {
    let s = store_with_scoring_table();
    let scorer = SectionScorer::new(s.scoring_tree(), vec!["某市第一人民医院".to_string()]);
    // T3 映射 SEC-03 但不提供内容（SECTION_MISSING 面）
    let sections = vec![
        ("SEC-01".to_string(), ideal_section(&["技术方案完整性", "针对性"], 600)),
        ("SEC-02".to_string(), ideal_section(&["项目实施方案", "进度保障措施"], 600)),
        ("SEC-03".to_string(), filler_section(700)), // 凑字数（档位差距面）
        ("SEC-10".to_string(), format!(
            "类似工程业绩，每提供 1 个得 2 分，最多 5 个，我方提供 5 个\
             （2019-2023 年累计合同额 8000 万元，详见附件合同复印件）。\
             企业具备 ISO9001 质量管理体系认证及 ISO27001 信息安全管理体系认证。{}",
            "证明材料齐全可核验。".repeat(48)
        )),
    ];
    // 未报价：total 不含价格面 + note 登记
    let ds = scorer.score_document(&sections, None);
    assert!(ds.notes.iter().any(|n| n.contains("NO_PRICE_INPUT")), "{:?}", ds.notes);
    // UNMAPPED（C3 无 section）+ SECTION_MISSING 面不误报（SEC-03 已提供——凑字数≠缺内容）
    assert!(ds.notes.iter().any(|n| n.contains("UNMAPPED") && n.contains("C3")),
        "{:?}", ds.notes);
    assert!(!ds.notes.iter().any(|n| n.contains("SECTION_MISSING")), "{:?}", ds.notes);

    // 手工（档位中点口径）：
    //   技术 = 9.5(T1 优) + 9.5(T2 优) + 2.75(T3 凑字数差档) = 21.75
    //   商务 = 9.5(C1 优) + 9.5(C2 优)；C3 未映射 → 不计入（人工核对）
    //   价格 = 未报价 0（note 登记）
    assert!((ds.technical - 21.75).abs() < 0.01, "got {}", ds.technical);
    assert_eq!(ds.commercial, Some(19.0), "got {:?}", ds.commercial);
    assert!((ds.total - 40.75).abs() < 0.01, "got {}", ds.total);
    let manual = 40.75;
    let rel = ((ds.total - manual).abs() / manual * 100.0).round();
    assert!(rel < 10.0, "差距 {rel}%：engine={} manual={}", ds.total, manual);

    // 词典 4：凑字数章节显著低分（同章节好/差对照）
    let good_t3 = scorer.score_section("SEC-03",
        &ideal_section(&["售后服务方案", "响应时限承诺"], 600));
    let bad_t3 = ds.breakdown.iter().find(|b| b.section_id == "SEC-03").unwrap();
    assert!(good_t3.estimated > 3.0 * bad_t3.estimated,
        "优 {:.2} vs 凑字数 {:.2} 须 3 倍以上差距", good_t3.estimated, bad_t3.estimated);
    assert_eq!(bad_t3.missing_items, vec!["T3".to_string()]);

    // 词典 5：B1 价格分接线（AveragePrice 0.95 + Ratio，报价 1600/对手 1500-1800）
    let spec = PriceScoreSpec {
        method: PriceScoreMethod::AveragePrice { coefficient: 0.95 },
        max_score: 40.0,
        budget: None,
        deviation_rule: DeviationRule::Ratio,
    };
    let ds2 = scorer.score_document(
        &sections,
        Some(PriceInput { spec: &spec, bid_price: 1600.0, others: vec![1500.0, 1600.0, 1700.0, 1800.0] }),
    );
    // 手工：基准 = mean(1600,1500,1600,1700,1800) = 1640 × 0.95 = 1558；
    //       价格分 = 1558/1600 × 40 = 38.95 → total = 21.75 + 19 + 38.95 = 79.7
    assert!((ds2.price - 38.95).abs() < 0.01, "B1 接线：got {}", ds2.price);
    assert!((ds2.total - 79.7).abs() < 0.01, "got {}", ds2.total);

    // 词典 6：rationale 全链可追溯（四维明细 + 档位判定 + 估分算式）
    for b in &ds2.breakdown {
        if b.item_scores.is_empty() {
            continue;
        }
        assert!(b.rationale.contains("覆盖"), "{} 缺覆盖明细", b.section_id);
        assert!(b.rationale.contains("→"), "{} 缺档位判定", b.section_id);
    }
    let t1 = ds2.breakdown.iter().find(|b| b.section_id == "SEC-01").unwrap();
    assert!(t1.item_scores[0].rationale.contains("10.00×0.950=9.50"),
        "估分算式可复核：{}", t1.item_scores[0].rationale);
}

/// #186 预留面：score_rules 透传三面（树 / score_section / score_document）
/// + **判档不消费**（同文同分——细则关键词不进覆盖度）+ 预留面 note 显式登记。
#[test]
fn score_rules_reserved_surface_wired_but_not_consumed() {
    let rules = "技术方案应具有针对性、组织体系合理、进度计划可执行，并提供应急保障措施；每缺一项扣 2 分";
    // T1 注入细则（params.score_rules —— extract.rs 评审表路径的登记形态）
    let mut reqs = scoring_table_reqs();
    reqs.iter_mut().find(|r| r.id == "T1").unwrap()
        .params.insert("score_rules".to_string(), serde_json::json!(rules));
    let mut s = DomainStore::new();
    s.create_project("智慧病房物联网平台", "ZHYY-2026-GK-018", "明标", "it-key-b").unwrap();
    s.upsert_file("tender.docx", "招标文件", "sha-b2", 12, 3, 240, 0, ParseStatus::Ok, "")
        .unwrap();
    s.set_requirements(reqs).unwrap();

    // 面 1：树派生透传（T1 带细则；T2 无 → None——不虚构）
    let tree = s.scoring_tree();
    assert_eq!(tree.index["T1"].score_rules.as_deref(), Some(rules));
    assert_eq!(tree.index["T2"].score_rules, None);

    // 面 2：score_section 的 item_scores 透传细则原文（起草/审校对照面）
    // + 判档不消费的强验证：同一份「只写部分关键词」的正文，带细则前后
    //   同文同分（细则里的「组织体系/进度计划/应急保障」若进覆盖度必然变化）
    let partial = ideal_section(&["技术方案完整性", "针对性"], 600);
    let scorer = SectionScorer::new(tree, vec![]);
    let with_rules = scorer.score_section("SEC-01", &partial);
    let base_scorer = SectionScorer::new(store_with_scoring_table().scoring_tree(), vec![]);
    let base = base_scorer.score_section("SEC-01", &partial);
    assert_eq!(with_rules.estimated, base.estimated,
        "判档不消费细则——同文同分：{} vs {}", with_rules.estimated, base.estimated);
    assert_eq!(with_rules.covered_items, base.covered_items);
    let t1 = with_rules.item_scores.iter().find(|i| i.item_id == "T1").unwrap();
    assert_eq!(t1.score_rules.as_deref(), Some(rules));
    assert_eq!(
        base.item_scores.iter().find(|i| i.item_id == "T1").unwrap().score_rules,
        None
    );

    // 面 3：score_document notes 预留面登记（不伪装「按细则评分」）
    let ds = scorer.score_document(&[("SEC-01".to_string(), partial.clone())], None);
    assert!(ds.notes.iter().any(|n| n.contains("SCORE_RULES_RESERVED") && n.contains("1 项")),
        "{:?}", ds.notes);
    let ds_base = base_scorer.score_document(&[("SEC-01".to_string(), partial)], None);
    assert!(!ds_base.notes.iter().any(|n| n.contains("SCORE_RULES_RESERVED")),
        "无细则不登记：{:?}", ds_base.notes);
}
