//! changediff_chain.rs —— #320 多轮改标对比验收（版本链 / 累积 diff / 最终态报告）。
//!
//! 验收对照（issue #320）：
//! 1. 能管理 3+ 版本链（含乱序 push 重排）—— synthetic_chain_semantics
//! 2. 累积 diff 准确（修改/新增/删除三分类 + 轨迹）—— synthetic_chain_semantics
//! 3. 最终要求清晰（逐条最终值 + 变更轮轨迹，不用脑补）—— real_cuc028_chain
//! 4. 真实多轮改标案例验证 —— real_cuc028_chain（cuc-028 真实公告链）
//!
//! 真实数据源：real-corpus/gov-procurement（cuc-028 三节点链）
//! 招标 2026-07-13 → 更正 2026-07-28 → 中标 2026-08-14；
//! 更正轮 GT（ground-truth-draft.jsonl GT-M17-11/12/13）：
//!   open_bid_time 2026-08-03 14:00 → 2026-08-13 14:00（MODIFY）
//!   node_1        2026-07-31        → 2026-08-30（MODIFY）
//!   node_2        2026-08-30        → 2026-09-10（MODIFY）
//!   node_3 / node_4 不变（GT-M17-14/15「不变陷阱」——被动即失实）

use std::collections::HashMap;

use bid_domain::changediff::{
    snapshot_from_reqs, ChainError, ChangeKind, VersionChain, VersionKind,
};
use bid_domain::impact::{AddendumEvent, ImpactEngine};
use serde_json::json;

/// 快捷构造单条款快照。
fn snap(pairs: &[(&str, &[(&str, serde_json::Value)])]) -> bid_domain::changediff::ClauseSnapshot {
    pairs
        .iter()
        .map(|(id, kvs)| {
            (
                id.to_string(),
                kvs.iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            )
        })
        .collect()
}

// ─────────────────────────────────────────────────────── 验收 1+2：合成链语义

#[test]
fn synthetic_chain_semantics() {
    let mut chain = VersionChain::new();

    // V1（招标）：3 条要求
    let r1 = chain
        .push_round(
            "2026-01-10",
            "招标公告.htm",
            "sha-t1",
            VersionKind::Tender,
            snap(&[
                ("A-工期", &[("start", json!("2026-02-01"))]),
                ("B-质保", &[("years", json!(3))]),
                ("C-付款", &[("ratio", json!("30%"))]),
            ]),
        )
        .unwrap();
    assert_eq!(r1, 1);

    // V2（第一次更正）：A 改值 + 新增 D
    chain
        .push_round(
            "2026-01-20",
            "更正公告一.htm",
            "sha-c1",
            VersionKind::Correction,
            snap(&[
                ("A-工期", &[("start", json!("2026-03-01"))]), // 修改
                ("B-质保", &[("years", json!(3))]),            // 不变
                ("C-付款", &[("ratio", json!("30%"))]),        // 不变
                ("D-培训", &[("days", json!(2))]),             // 新增
            ]),
        )
        .unwrap();

    // V3（第二次更正）：A 再改 + C 删除
    chain
        .push_round(
            "2026-02-01",
            "更正公告二.htm",
            "sha-c2",
            VersionKind::Correction,
            snap(&[
                ("A-工期", &[("start", json!("2026-04-01"))]), // 二次修改
                ("B-质保", &[("years", json!(3))]),            // 不变
                ("D-培训", &[("days", json!(2))]),             // 不变（C 已删）
            ]),
        )
        .unwrap();

    // V4（中标公告）：无要求变更（快照同 V3）
    let r4 = chain
        .push_round(
            "2026-03-01",
            "中标公告.htm",
            "sha-a1",
            VersionKind::Award,
            snap(&[
                ("A-工期", &[("start", json!("2026-04-01"))]),
                ("B-质保", &[("years", json!(3))]),
                ("D-培训", &[("days", json!(2))]),
            ]),
        )
        .unwrap();
    assert_eq!(r4, 4, "顺序 push 轮号应为 4");
    assert_eq!(chain.len(), 4, "验收①：4 版本链");

    // ---- 累积 diff V1→V3（验收②：三分类准确）----
    let diff = chain.cumulative_diff(1, 3).unwrap();
    // Modified：A（净口径 V1→V3），轨迹 [2, 3]（两轮都动过）
    assert_eq!(diff.modified.len(), 1);
    let m = &diff.modified[0];
    assert_eq!(m.clause_id, "A-工期");
    assert_eq!(m.kind, ChangeKind::Modified);
    assert_eq!(m.touched_rounds, vec![2, 3], "A 在第 2、3 轮各变更一次");
    assert_eq!(m.last_touched_date, "2026-02-01");
    assert_eq!(
        m.keys,
        vec![bid_domain::changediff::KeyChange {
            key: "start".into(),
            old: Some(json!("2026-02-01")),
            new: Some(json!("2026-04-01")),
        }],
        "净口径：from→to 的 old/new（不是中间值）"
    );
    // Added：D，首现轮 2
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.added[0].clause_id, "D-培训");
    assert_eq!(diff.added[0].touched_rounds, vec![2]);
    // Deleted：C，消失轮 3
    assert_eq!(diff.deleted.len(), 1);
    assert_eq!(diff.deleted[0].clause_id, "C-付款");
    assert_eq!(diff.deleted[0].touched_rounds, vec![3]);
    assert_eq!(diff.deleted[0].last_touched_date, "2026-02-01");
    // B 全程不变 → 不入 diff（不变陷阱口径）
    assert!(!diff
        .modified
        .iter()
        .chain(diff.added.iter())
        .chain(diff.deleted.iter())
        .any(|c| c.clause_id == "B-质保"));

    // ---- V1→V4 净变更与 V1→V3 相同（V4 无要求变更——链位仍占）----
    let diff14 = chain.cumulative_diff(1, 4).unwrap();
    assert_eq!(diff14.modified, diff.modified);
    assert_eq!(diff14.added, diff.added);
    assert_eq!(diff14.deleted, diff.deleted);

    // ---- 报告（验收③雏形：最终态 + 轨迹）----
    let report = chain.final_report().unwrap();
    assert_eq!(report.rounds, 4);
    assert_eq!(report.correction_rounds, 2);
    assert_eq!(report.date_span, ("2026-01-10".to_string(), "2026-03-01".to_string()));
    assert_eq!(
        report.summary_line(),
        "共 4 轮公告（2 轮更正），累计修改 1 条 / 新增 1 条 / 删除 1 条要求"
    );
    // 最终态：A/B/D 三条在场，C 已删（只在 cumulative.deleted）
    let ids: Vec<&str> = report
        .final_states
        .iter()
        .map(|s| s.clause_id.as_str())
        .collect();
    assert_eq!(ids, vec!["A-工期", "B-质保", "D-培训"]);
    let a = &report.final_states[0];
    assert_eq!(a.params.get("start"), Some(&json!("2026-04-01")), "最终值");
    assert_eq!(a.origin_round, 1);
    assert_eq!(
        a.touched_rounds,
        vec![(2, "2026-01-20".to_string()), (3, "2026-02-01".to_string())]
    );
    let b = &report.final_states[1];
    assert!(b.touched_rounds.is_empty(), "B 全程未动");
    let d = &report.final_states[2];
    assert_eq!(d.origin_round, 2, "D 首现于第 2 轮");
    // JSON 面形状钉死（未来 MCP 接线口径）
    let rj = report.to_json();
    assert_eq!(rj["summary"], "共 4 轮公告（2 轮更正），累计修改 1 条 / 新增 1 条 / 删除 1 条要求");
    assert_eq!(rj["cumulative"]["modified"][0]["clause_id"], "A-工期");
    assert_eq!(rj["final_states"][2]["origin_round"], 2);
}

// ───────────────────────────── 乱序 push / 幂等 / 错误面（验收①的边界）

#[test]
fn out_of_order_push_reorders_by_date() {
    let mut chain = VersionChain::new();
    // 按「抓取顺序」乱序入链：先更正、再中标、最后招标
    let r_corr = chain
        .push_round(
            "2026-07-28",
            "更正公告.htm",
            "sha-c",
            VersionKind::Correction,
            snap(&[("X", &[("v", json!(2))])]),
        )
        .unwrap();
    assert_eq!(r_corr, 1, "首推即占位 1");
    let r_award = chain
        .push_round(
            "2026-08-14",
            "中标公告.htm",
            "sha-a",
            VersionKind::Award,
            snap(&[("X", &[("v", json!(2))])]),
        )
        .unwrap();
    assert_eq!(r_award, 2);
    // 招标公告日期最早 → 插到头部，更正/中标轮号后移
    let r_tender = chain
        .push_round(
            "2026-07-13",
            "招标公告.htm",
            "sha-t",
            VersionKind::Tender,
            snap(&[("X", &[("v", json!(1))])]),
        )
        .unwrap();
    assert_eq!(r_tender, 1, "日期最早的招标公告回到链首");
    let rounds = chain.rounds();
    assert_eq!(
        rounds
            .iter()
            .map(|r| (r.kind.as_str(), r.date.as_str()))
            .collect::<Vec<_>>(),
        vec![("tender", "2026-07-13"), ("correction", "2026-07-28"), ("award", "2026-08-14")],
        "轮号恒为日期序（AGENTS：真实抓取顺序不保证时间序）"
    );
    // 重排后的 diff 语义正确：V1(X=1) → V3(X=2) 恰一条 Modified 在轮 2
    let diff = chain.cumulative_diff(1, 3).unwrap();
    assert_eq!(diff.modified.len(), 1);
    assert_eq!(diff.modified[0].touched_rounds, vec![2]);
}

#[test]
fn duplicate_sha_and_bad_dates_are_rejected() {
    let mut chain = VersionChain::new();
    chain
        .push_round(
            "2026-01-01",
            "a.htm",
            "sha-x",
            VersionKind::Tender,
            snap(&[("A", &[("k", json!(1))])]),
        )
        .unwrap();
    // 同 sha 重复 → 幂等拒绝（与引擎 imported 口径一致）
    assert_eq!(
        chain.push_round(
            "2026-02-01",
            "b.htm",
            "sha-x",
            VersionKind::Correction,
            snap(&[("A", &[("k", json!(2))])]),
        ),
        Err(ChainError::DuplicateSha("sha-x".into()))
    );
    // 非法日期（月份 13）→ 拒绝。月长面（如 2026-02-30）由单一真源
    // dates::is_valid_ymd 裁定：#381（#375 修复）合入后本链自动变严，
    // 不在此重复实现月长规则（低耦合：日期合法性只有一处定义）。
    assert_eq!(
        chain.push_round(
            "2026-13-01",
            "c.htm",
            "sha-y",
            VersionKind::Correction,
            snap(&[]),
        ),
        Err(ChainError::BadDate("2026-13-01".into()))
    );
    assert_eq!(
        chain.push_round(
            "not-a-date",
            "c2.htm",
            "sha-y2",
            VersionKind::Correction,
            snap(&[]),
        ),
        Err(ChainError::BadDate("not-a-date".into()))
    );
    // 区间退化 / 轮号越界 / 空链
    assert_eq!(
        chain.cumulative_diff(1, 1),
        Err(ChainError::EmptyRange { from: 1, to: 1 })
    );
    assert_eq!(
        chain.cumulative_diff(1, 5),
        Err(ChainError::NoSuchRound(5))
    );
    assert_eq!(
        VersionChain::new().final_report(),
        Err(ChainError::EmptyChain)
    );
    // 同日双公告：保序追加（不拒——公告同日多发是真实形态）
    let r2 = chain
        .push_round(
            "2026-01-01",
            "same-day.htm",
            "sha-z",
            VersionKind::Correction,
            snap(&[("A", &[("k", json!(1))])]),
        )
        .unwrap();
    assert_eq!(r2, 2, "同日按 push 顺序保序");
}

// ─────────────────────────── 验收 3+4：cuc-028 真实三节点链

#[test]
fn real_cuc028_chain() {
    let mut chain = VersionChain::new();

    // 节点 1/3：招标公告（2026-07-13）——GT-M17-05/12/13/14 的原值
    chain
        .push_round(
            "2026-07-13",
            "cuc-028-tender-20260713.txt",
            "announce:t20260713_26925801",
            VersionKind::Tender,
            snap(&[
                ("open_bid_time", &[("value", json!("2026-08-03 14:00"))]),
                ("node_1", &[("due", json!("2026-07-31"))]),
                ("node_2", &[("due", json!("2026-08-30"))]),
                ("node_3", &[("due", json!("2026-09-23"))]),
            ]),
        )
        .unwrap();

    // 节点 2/3：更正公告（2026-07-28）——GT-M17-11/12/13 的 MODIFY + 14/15 不变
    chain
        .push_round(
            "2026-07-28",
            "cuc-028-correction-20260728.txt",
            "announce:t20260728_27025731",
            VersionKind::Correction,
            snap(&[
                ("open_bid_time", &[("value", json!("2026-08-13 14:00"))]), // MODIFY
                ("node_1", &[("due", json!("2026-08-30"))]),                // MODIFY
                ("node_2", &[("due", json!("2026-09-10"))]),                // MODIFY
                ("node_3", &[("due", json!("2026-09-23"))]),                // UNCHANGED
            ]),
        )
        .unwrap();

    // 节点 3/3：中标公告（2026-08-14）——无要求变更（快照同更正后）
    chain
        .push_round(
            "2026-08-14",
            "cuc-028-award-20260814.txt",
            "announce:t20260814_27133532",
            VersionKind::Award,
            snap(&[
                ("open_bid_time", &[("value", json!("2026-08-13 14:00"))]),
                ("node_1", &[("due", json!("2026-08-30"))]),
                ("node_2", &[("due", json!("2026-09-10"))]),
                ("node_3", &[("due", json!("2026-09-23"))]),
            ]),
        )
        .unwrap();

    assert_eq!(chain.len(), 3, "真实链 3 节点");

    // ---- 累积 diff V1→V3：恰 3 条 Modified、0 新增、0 删除 ----
    let diff = chain.cumulative_diff(1, 3).unwrap();
    assert_eq!(diff.added.len(), 0, "真实更正无新增条款");
    assert_eq!(diff.deleted.len(), 0, "真实更正无删除条款");
    assert_eq!(diff.modified.len(), 3, "GT-M17-11/12/13 三条 MODIFY");
    let by_id: HashMap<&str, &bid_domain::changediff::ClauseChange> = diff
        .modified
        .iter()
        .map(|c| (c.clause_id.as_str(), c))
        .collect();
    // 逐条核对真实 old→new（GT expected 字段的实值）
    let m_open = by_id["open_bid_time"];
    assert_eq!(
        m_open.keys,
        vec![bid_domain::changediff::KeyChange {
            key: "value".into(),
            old: Some(json!("2026-08-03 14:00")),
            new: Some(json!("2026-08-13 14:00")),
        }]
    );
    assert_eq!(by_id["node_1"].keys[0].old, Some(json!("2026-07-31")));
    assert_eq!(by_id["node_1"].keys[0].new, Some(json!("2026-08-30")));
    assert_eq!(by_id["node_2"].keys[0].old, Some(json!("2026-08-30")));
    assert_eq!(by_id["node_2"].keys[0].new, Some(json!("2026-09-10")));
    // 全部变更发生在第 2 轮（07-28 更正）；第 3 轮（中标）零要求变更
    for m in &diff.modified {
        assert_eq!(m.touched_rounds, vec![2]);
        assert_eq!(m.last_touched_date, "2026-07-28");
    }
    // 不变陷阱（GT-M17-14）：node_3 明示不变——不得入 diff
    assert!(!by_id.contains_key("node_3"), "「2026年9月23日前」明示不变，被动即失实");

    // ---- 报告：3 轮 / 1 轮更正 / 累计修改 3 条 ----
    let report = chain.final_report().unwrap();
    assert_eq!(report.rounds, 3);
    assert_eq!(report.correction_rounds, 1);
    assert_eq!(
        report.date_span,
        ("2026-07-13".to_string(), "2026-08-14".to_string())
    );
    assert_eq!(
        report.summary_line(),
        "共 3 轮公告（1 轮更正），累计修改 3 条 / 新增 0 条 / 删除 0 条要求"
    );
    // 最终态（验收③：投标人视角逐条最终值，不用脑补）
    let finals: HashMap<&str, &bid_domain::changediff::FinalClauseState> = report
        .final_states
        .iter()
        .map(|s| (s.clause_id.as_str(), s))
        .collect();
    assert_eq!(finals.len(), 4);
    assert_eq!(
        finals["open_bid_time"].params.get("value"),
        Some(&json!("2026-08-13 14:00")),
        "最终开标时间 = 更正后值"
    );
    assert_eq!(
        finals["node_3"].touched_rounds,
        Vec::<(u32, String)>::new(),
        "node_3 全程未动——最终值即原值"
    );
    assert_eq!(
        finals["node_1"].touched_rounds,
        vec![(2, "2026-07-28".to_string())]
    );
    // 全部条款源于第 1 轮（招标公告建立初始要求集）
    for s in report.final_states.iter() {
        assert_eq!(s.origin_round, 1);
    }
}

// ─────────────────────────── 引擎适配面：ImpactEngine 快照入链

#[test]
fn engine_rounds_feed_the_chain() {
    // 引擎语义与 four_contracts c1_real_addendum_chain_cuc028 同源：
    // 初始态 = 招标后要求集；一次更正 import_events 改 GT-S01/GT-S02。
    let mut eng = ImpactEngine::cuc_2026_028();
    let mut chain = VersionChain::new();

    // 轮 1：招标后快照（引擎初始态）
    chain
        .push_engine_round(
            "2026-07-13",
            "cuc-028-tender-20260713.txt",
            "announce:t20260713_26925801",
            VersionKind::Tender,
            &eng,
        )
        .unwrap();

    // 更正轮事件（真实：节点一 07-31→08-30、节点二 08-30→09-10）
    let mut e1 = HashMap::new();
    e1.insert("milestone_date".to_string(), json!("2026-08-30"));
    let mut e2 = HashMap::new();
    e2.insert("milestone_date".to_string(), json!("2026-09-10"));
    let events = vec![
        AddendumEvent::ClauseModified { gt_id: "GT-S01".into(), params: e1 },
        AddendumEvent::ClauseModified { gt_id: "GT-S02".into(), params: e2 },
    ];
    eng.import_events(
        "中国传媒大学教学资源中心建设更正公告-20260728.htm",
        "announce:t20260728_27025731",
        &events,
    );

    // 轮 2：更正后快照
    chain
        .push_engine_round(
            "2026-07-28",
            "中国传媒大学教学资源中心建设更正公告-20260728.htm",
            "announce:t20260728_27025731",
            VersionKind::Correction,
            &eng,
        )
        .unwrap();

    // 链 diff 与引擎语义对齐：GT-S01/GT-S02 两条 Modified、old→new 为实值
    let diff = chain.cumulative_diff(1, 2).unwrap();
    assert_eq!(diff.modified.len(), 2);
    assert_eq!(diff.added.len(), 0);
    assert_eq!(diff.deleted.len(), 0);
    let by_id: HashMap<&str, &bid_domain::changediff::ClauseChange> = diff
        .modified
        .iter()
        .map(|c| (c.clause_id.as_str(), c))
        .collect();
    assert_eq!(by_id["GT-S01"].keys[0].old, Some(json!("2026-07-31")));
    assert_eq!(by_id["GT-S01"].keys[0].new, Some(json!("2026-08-30")));
    assert_eq!(by_id["GT-S02"].keys[0].old, Some(json!("2026-08-30")));
    assert_eq!(by_id["GT-S02"].keys[0].new, Some(json!("2026-09-10")));
    // 未变更条款不入 diff（GT-S03 真实口径：公告明示不变）
    assert!(!by_id.contains_key("GT-S03"));

    // 引擎快照适配的确定性：直接从 reqs 拍的快照 == push_engine_round 存的
    let direct = snapshot_from_reqs(eng.reqs());
    assert_eq!(&direct, chain.snapshot(2).unwrap());
}
