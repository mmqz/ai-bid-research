//! quality_integrate.rs —— B3 收口：与 B2 自评联动（issue #97 §与自评的联动）。
//!
//! 验收线：
//! - 一个章节的检查报告里同时有：响应性（covered/uncovered）、质量
//!   （specificity/substance/dup_with）、得分（B2 SectionScore）；
//! - 质量与得分的相关性（issue：「质量高的章节得分也应高」）；
//! - 旧调用面（无 ctx）完全向后兼容：新字段全 None/空；
//! - 旧报告 JSON（无新字段）反序列化经 serde default 兼容。
//!
//! 夹具：域状态走真实状态机（quality_check.rs 同款纪律）。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::scoring::{derive_scoring_tree, SectionScorer};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::quality::{
    build_quality_report, build_quality_report_with, check_section, check_section_with,
    write_report, ProjectContext, QualityContext, QualityReport,
};
use std::collections::HashMap;

// ---------------------------------------------------------------- 夹具

/// 评分项 REQ（B2 槽位：score + section + criteria 摘录）。
fn scoring_req(id: &str, score: f64, section: &str, criteria: &str) -> Requirement {
    let mut params = HashMap::new();
    params.insert("score".to_string(), serde_json::json!(score));
    params.insert("section".to_string(), serde_json::json!(section));
    Requirement {
        id: id.into(),
        category: "评分项".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第四章/{id}"),
            excerpt: criteria.into(),
        }],
        params,
        source_file: "A-招标文件.docx".into(),
    }
}

fn node(id: &str, title: &str, order: u32, reqs: &[&str], target: u32) -> OutlineNode {
    OutlineNode {
        id: id.into(),
        parent_id: None,
        title: title.into(),
        level: 1,
        order,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars: target,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }
}

/// 真实状态机推进：写内容 + Pending→Outlining→Drafting→DRAFTED。
fn draft_it(s: &mut DomainStore, id: &str, content: &str) {
    s.set_section_content(id, content.to_string()).unwrap();
    for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
        s.advance_section_status(id, to).unwrap();
    }
}

/// 好章节（覆盖评分项关键词 + 专名 + 参数 + 证据引用——优档形态）。
fn good_content() -> String {
    let mut s = String::from(
        "# 技术方案\n\n\
        本章论述塔里木水库除险加固工程技术方案的完整性与针对性，并给出针对性落实路径。\n\n\
        坝体加固：坝顶全长 1850 米，加固厚度 0.6 米，混凝土 1.2 万立方米，工期 180 日历天。\
        防渗墙深度 42 米，围堰按 20 年一遇洪水设计。\n\n\
        针对 [REQ-0101] 的全部要求本章均已回应；同类业绩与资质证明见 [EV-0001] 至 [EV-0005]。",
    );
    while s.chars().filter(|c| !c.is_whitespace()).count() < 600 {
        s.push_str("技术方案完整性与针对性方面的落实措施在本单位同类项目中已形成标准化路径。");
    }
    s
}

/// 凑数章节（无关键词/无专名/无参数/无证据——差档形态；引用 [REQ-0102]
/// 使形式合规面干净——needs_human 的判定不被形式面噪声污染）。
fn filler_content() -> String {
    let mut s = String::from(
        "# 服务承诺\n\n我司高度重视本项目（响应 [REQ-0102]），将组建优秀团队，\
        采用先进理念，科学管理，精心组织，确保圆满履约。",
    );
    while s.chars().count() < 600 {
        s.push_str("我司将秉承诚信原则，提供一流的服务，全力以赴，确保万无一失。");
    }
    s
}

/// 基座：三章（好/凑数/好章的整段复制）+ 两评分项（映射 SEC-0001/SEC-0002）。
fn store_base() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("塔里木水库除险加固", "SL-2026-018", "明标", "int-key").unwrap();
    s.upsert_file("A-招标文件.docx", "招标文件", "aa", 5, 1, 50, 0,
                  ParseStatus::Ok, "engine-docx @ test").unwrap();
    s.set_requirements(vec![
        scoring_req("REQ-0101", 10.0, "SEC-0001", "技术方案完整性与针对性 10 分"),
        scoring_req("REQ-0102", 10.0, "SEC-0002", "服务承诺与售后响应 10 分"),
        scoring_req("REQ-0103", 5.0, "SEC-0003", "施工组织与总平面布置 5 分"),
    ]).unwrap();
    s.set_outline(vec![
        node("SEC-0001", "技术方案", 1, &["REQ-0101"], 600),
        node("SEC-0002", "服务承诺", 2, &["REQ-0102"], 600),
        node("SEC-0003", "复制章", 3, &["REQ-0103"], 600),
    ]).unwrap();
    s
}

fn make_ctx(store: &DomainStore) -> (ProjectContext, SectionScorer) {
    let project = ProjectContext {
        project_name: "塔里木水库除险加固".into(),
        tenderer_name: String::new(), // store 无此字段的真实面
        extra_terms: vec!["SL-2026-018".into()],
    };
    let scorer = SectionScorer::new(
        derive_scoring_tree(store.requirements()),
        project.scorer_terms(),
    );
    (project, scorer)
}

// ---------------------------------------------------------------- 联动验收

#[test]
fn report_carries_responsiveness_quality_and_score_together() {
    let mut s = store_base();
    draft_it(&mut s, "SEC-0001", &good_content());
    draft_it(&mut s, "SEC-0002", &filler_content());
    let (project, scorer) = make_ctx(&s);
    let ctx = QualityContext { scorer: &scorer, project: &project };
    let report = build_quality_report_with(&s, None, Some(&ctx));

    let good = report.sections.iter().find(|q| q.section_id == "SEC-0001").unwrap();
    // 响应性 + 质量 + 得分三面并存（issue 联动验收的核心断言）
    assert!(!good.covered_reqs.is_empty(), "响应性面：REQ 已回应");
    let sp = good.specificity.as_ref().expect("质量面：针对性");
    let st = good.substance.as_ref().expect("质量面：实质性");
    let sc = good.score.as_ref().expect("得分面：B2 自评");
    assert!(sp.score > 0.7, "好章节针对性 > 0.7（实测 {:.3}）", sp.score);
    assert!(st.score > 0.5, "好章节实质性（实测 {:.3}）", st.score);
    assert!(sc.max > 0.0 && sc.estimated > 0.0, "自评有分：{sc:?}");

    let filler = report.sections.iter().find(|q| q.section_id == "SEC-0002").unwrap();
    let sp2 = filler.specificity.as_ref().unwrap();
    let st2 = filler.substance.as_ref().unwrap();
    let sc2 = filler.score.as_ref().unwrap();
    assert!(sp2.score < 0.4, "凑数章针对性 < 0.4（实测 {:.3}）", sp2.score);
    assert!(st2.score < 0.4, "凑数章实质性 < 0.4（实测 {:.3}）", st2.score);
    assert!(sc2.estimated < sc2.max, "凑数章未满分：{sc2:?}");

    // 相关性（issue：「质量高的章节得分也应高」）
    let good_ratio = sc.estimated / sc.max;
    let filler_ratio = sc2.estimated / sc2.max;
    assert!(
        good_ratio > filler_ratio,
        "质量↔得分正相关：好 {good_ratio:.3} > 凑数 {filler_ratio:.3}"
    );
}

#[test]
fn severe_duplication_flags_needs_human_and_backfills_dup_with() {
    let mut s = store_base();
    let good = good_content();
    draft_it(&mut s, "SEC-0001", &good);
    draft_it(&mut s, "SEC-0002", &filler_content());
    draft_it(&mut s, "SEC-0003", &good); // 整段复制 SEC-0001
    let (project, scorer) = make_ctx(&s);
    let ctx = QualityContext { scorer: &scorer, project: &project };
    let report = build_quality_report_with(&s, None, Some(&ctx));

    let dup = report.duplication.as_ref().expect("ctx 在场且 ≥2 已起草 → 有重复面");
    assert_eq!(dup.severe, 1, "整段复制 = 严重对：{dup:?}");
    assert_eq!(report.summary.severe_dup_pairs, 1);

    let a = report.sections.iter().find(|q| q.section_id == "SEC-0001").unwrap();
    let c = report.sections.iter().find(|q| q.section_id == "SEC-0003").unwrap();
    assert!(!a.dup_with.is_empty(), "dup_with 回填：{:?}", a.dup_with);
    assert_eq!(a.dup_with[0].0, "SEC-0003");
    assert!(a.dup_with[0].1 > 0.8, "相似度 > 0.8（实测 {:.3}）", a.dup_with[0].1);
    assert_eq!(c.dup_with[0].0, "SEC-0001", "双向回填");
    // 严重重复 → needs_human（依据：结构性缺陷须人工裁决删改哪段）
    assert!(a.needs_human, "严重重复章需人工处理");
    assert!(c.needs_human);
    assert!(report.summary.needs_human.contains(&"SEC-0001".to_string()));
    assert!(report.summary.needs_human.contains(&"SEC-0003".to_string()));
    // 非重复章不受牵连
    let b = report.sections.iter().find(|q| q.section_id == "SEC-0002").unwrap();
    assert!(b.dup_with.is_empty());
    assert!(!b.needs_human, "凑数章低质量不触发人工闸门（B4 循环的输入；REQ 已引用+带内=形式合规）");
}

#[test]
fn pending_section_gets_none_quality_and_score() {
    let mut s = store_base();
    draft_it(&mut s, "SEC-0001", &good_content());
    // SEC-0002/SEC-0003 未起草
    let (project, scorer) = make_ctx(&s);
    let ctx = QualityContext { scorer: &scorer, project: &project };
    let report = build_quality_report_with(&s, None, Some(&ctx));
    let pending = report.sections.iter().find(|q| q.section_id == "SEC-0002").unwrap();
    assert!(pending.specificity.is_none(), "未起草不评质量（不伤噪）");
    assert!(pending.substance.is_none());
    assert!(pending.score.is_none());
    // 已起草章照评
    let good = report.sections.iter().find(|q| q.section_id == "SEC-0001").unwrap();
    assert!(good.specificity.is_some() && good.score.is_some());
    // 汇总均值只含已评章节（诚实面）
    assert!(report.summary.avg_specificity.is_some());
    assert!(report.summary.avg_score_ratio.is_some());
}

// ---------------------------------------------------------------- 向后兼容

#[test]
fn legacy_call_path_yields_none_new_fields() {
    let mut s = store_base();
    draft_it(&mut s, "SEC-0001", &good_content());
    // 旧调用面（无 ctx）
    let report: QualityReport = build_quality_report(&s, None);
    assert!(report.duplication.is_none(), "无 ctx 不算重复");
    assert!(report.summary.avg_specificity.is_none());
    assert!(report.summary.avg_substance.is_none());
    assert!(report.summary.avg_score_ratio.is_none());
    assert_eq!(report.summary.severe_dup_pairs, 0);
    for q in &report.sections {
        assert!(q.specificity.is_none() && q.substance.is_none() && q.score.is_none());
        assert!(q.dup_with.is_empty());
    }
    // 旧单章面（check_section）等价 check_section_with(None)
    let node = s.sections().iter().find(|n| n.id == "SEC-0001").unwrap().clone();
    let a = check_section(&s, &node);
    let b = check_section_with(&s, &node, None);
    assert_eq!(a, b, "旧面 = 新面的 None 路径");
}

#[test]
fn old_report_json_deserializes_with_defaults() {
    // B3 之前的报告形态（无新字段）——serde default 兼容，旧件可读
    let old_json = r#"{
      "project_id": "PRJ-0001",
      "generated_at": 1700000000,
      "sections": [{
        "section_id": "SEC-0001",
        "title": "技术方案",
        "status": "DRAFTED",
        "chars": 600,
        "target_chars": 600,
        "within_band": true,
        "covered_reqs": ["T1"],
        "uncovered_reqs": [],
        "ghost_reqs": [],
        "cited_evs": [],
        "uncited_evs": [],
        "unknown_citations": [],
        "unanchored_claims": 0,
        "requirement_ids": ["T1"],
        "error": null,
        "needs_human": false
      }],
      "summary": {
        "total": 1, "drafted": 1, "failed": 0, "pending": 0,
        "uncovered_req_total": 0, "out_of_band": 0, "unanchored_total": 0,
        "needs_human": []
      }
    }"#;
    let report: QualityReport = serde_json::from_str(old_json)
        .expect("旧报告 JSON 经 serde default 反序列化");
    let q = &report.sections[0];
    assert!(q.specificity.is_none() && q.substance.is_none() && q.score.is_none());
    assert!(q.dup_with.is_empty());
    assert!(report.duplication.is_none());
    assert_eq!(report.summary.severe_dup_pairs, 0);
}

#[test]
fn report_roundtrip_and_write() {
    let mut s = store_base();
    draft_it(&mut s, "SEC-0001", &good_content());
    draft_it(&mut s, "SEC-0002", &filler_content());
    let (project, scorer) = make_ctx(&s);
    let ctx = QualityContext { scorer: &scorer, project: &project };
    let report = build_quality_report_with(&s, None, Some(&ctx));

    // serde 全字段 roundtrip（含 B2 SectionScore / B3 三维度的新字段面）
    let json = serde_json::to_string(&report).unwrap();
    let back: QualityReport = serde_json::from_str(&json).unwrap();
    assert_eq!(report, back, "roundtrip 无损");

    // 落盘 + 回读
    let ws = std::env::temp_dir().join(format!("bid-int-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    let path = write_report(&report, &ws).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    let from_disk: QualityReport = serde_json::from_str(&raw).unwrap();
    assert_eq!(report, from_disk, "落盘回读无损");
    let _ = std::fs::remove_dir_all(&ws);
}
