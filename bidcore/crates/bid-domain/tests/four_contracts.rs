//! four_contracts.rs —— 四关键业务案例的 Rust 契约测试（P1-b）。
//!
//! Python SEMUL（V1-b/V2/V3）→ bidcore Rust 实装的交叉验证：
//!   C1 补遗级联失效（v2_impact.py R0~R5 断言组逐条对应）
//!   C2 证据关联只降不升（v1_citation R2 + v3 Q2：改写恰一链 / undo 不回升 /
//!      外部修改不自动 adopt）
//!   C3 多轮章节重写后锚点正确（v3 Q1：五轮重写、恰一链降级、零悬空、逐轮重验恢复）
//!   C4 导出中断不产生虚假成功（v3 Q3：停 EXPORTING、不变式、恢复幂等同 sha、
//!      与 ProjectStatus::Exported 状态机的一致性）
//!
//! 验证状态词典（verification-plan §6）：
//!   本文件全绿 = IMPACT/LINKAGE/EXPORT_GATE 语义 IMPLEMENTED；
//!   Python SEMUL 仍保留（文件级 DOCX 书签寻址证据）——双实现交叉对照，
//!   结果分记，互不折抵。

use bid_domain::export_gate::{ExportGate, ExportPhase};
use bid_domain::impact::{AcceptBlock, AddendumSpec, BindingStatus, ImpactEngine};
use bid_domain::linkage::{text_fingerprint, LinkLocator, LinkageLedger, LinkageStatus};
use bid_domain::ProjectStatus;
use std::collections::HashMap;

// ================================================================ C1 补遗级联失效

/// corpus-A C-补遗文件01 三变更联合（fixture_spec.py PROJECT/MODELS 同构值）。
fn corpus_a_addendum() -> AddendumSpec {
    AddendumSpec {
        new_model: "USG6705E-AC650".into(),     // MODELS.firewall_new
        new_eval_date: "2026-11-27".into(),     // PROJECT.open_bid_date_after_addendum
        control_price: 4_650_000,                // PROJECT.control_price_c
        conflict_gt_ids: vec!["GT-21".into(), "GT-08".into()],
    }
}

fn corpus_a_engine() -> ImpactEngine {
    let mut eng = ImpactEngine::corpus_a();
    eng.prime_validity(); // A 口径初始判定（GT-13 本已 EXPIRED——2025-08-01 < 2026-11-15）
    eng
}

#[test]
fn c1_addendum_cascade_dirty_exact_and_version_bump() {
    let mut eng = corpus_a_engine();
    let c_sha = "sha256:补遗C";
    let impact = eng.import_addendum("C-补遗文件01.docx", c_sha, &corpus_a_addendum());

    // R1：DIRTY 恰三章节（技术方案-网络与安全 / 商务部分-报价 / 资格响应）
    let mut dirty = impact.dirty_sections.clone();
    dirty.sort();
    assert_eq!(
        dirty,
        vec!["商务部分-报价", "技术方案-网络与安全", "资格响应"],
        "DIRTY 章节必须恰为三——多则影响面污染、少则级联漏网"
    );
    // R1：GT-18 v2 生效（latest_version_wins）——current_model + model_v2 双参数断言（R3-P2-2）
    let gt18 = eng.reqs().get("GT-18").unwrap();
    assert_eq!(gt18.params["current_model"].as_str().unwrap(), "USG6705E-AC650");
    assert_eq!(gt18.params["model_v2"].as_str().unwrap(), "USG6705E-AC650", "model_v2 参数与 current_model 同源（补遗版本链）");
    // R1：CONFLICT-01 双版本登记
    let conflicts = eng.conflicts();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts["CONFLICT-01"].versions.len(), 2);
    assert!(!conflicts["CONFLICT-01"].resolved);
    // R1：content_version 恰 +1（一次导入一个版本戳——无新旧混合）
    assert_eq!(eng.content_version, 2);
    assert_eq!(eng.eval_date(), "2026-11-27");
}

#[test]
fn c1_cascade_precise_flips_no_overreach() {
    let mut eng = corpus_a_engine();
    let impact = eng.import_addendum("C-补遗文件01.docx", "sha-c", &corpus_a_addendum());

    // R2：GT-02 恰翻转 eligible → EXPIRED（2026-11-25 < 2026-11-27；资格判断下游）
    assert_eq!(eng.bindings()["GT-02"].status, BindingStatus::Expired);
    assert_eq!(eng.matrix()["GT-02"].policy, "not_eligible");
    assert!(impact.flips.contains(&"GT-02".to_string()));
    // R2：GT-13 维持 EXPIRED 禁绑（本已过期——重检不推翻；评分结论下游，细项 0 分）
    assert_eq!(eng.bindings()["GT-13"].status, BindingStatus::Expired);
    assert_eq!(eng.matrix()["GT-13"].score_subitem, Some(0));
    // R2：其余证据零误翻（重检 ≠ 全推翻）
    for gt in ["GT-01", "GT-05", "GT-06"] {
        assert_eq!(eng.bindings()[gt].status, BindingStatus::Valid, "{gt} 不得误翻");
    }
    // R2：重检日志恰两轮（A 口径 + C 口径）
    assert_eq!(eng.recheck_log, vec!["2026-11-15", "2026-11-27"]);

    // R3：无关不重跑——NOT DIRTY 恰为影响面外章节
    let mut clean: Vec<String> = eng
        .sections()
        .iter()
        .filter(|(_, s)| !s.dirty)
        .map(|(k, _)| k.clone())
        .collect();
    clean.sort();
    assert_eq!(clean, vec!["售后服务方案", "格式与签署", "项目团队"], "影响面外章节零误伤（恰三章节；UTF-8 码点序）");
}

#[test]
fn c1_adjudication_gate_and_idempotency() {
    let mut eng = corpus_a_engine();
    let c_sha = "sha-c";
    let _ = eng.import_addendum("C-补遗文件01.docx", c_sha, &corpus_a_addendum());

    // R4：脏阻断
    match eng.try_accept("商务部分-报价") {
        Err(AcceptBlock::Dirty) => {}
        other => panic!("终裁前脏章节不得 ACCEPTED，实际 {other:?}"),
    }
    // 重起草清脏 → 未裁决冲突仍阻断
    eng.redraft("商务部分-报价");
    match eng.try_accept("商务部分-报价") {
        Err(AcceptBlock::UnresolvedConflict(ids)) => {
            assert_eq!(ids, vec!["CONFLICT-01"]);
        }
        other => panic!("未裁决冲突必须阻断，实际 {other:?}"),
    }
    // 终裁（补遗优先 → 465 万）→ 放行 + 口径切换
    let (final_v, rule) = eng.adjudicate("CONFLICT-01").expect("终裁");
    assert_eq!(final_v, 4_650_000);
    assert!(rule.contains("补遗优先"));
    assert_eq!(eng.matrix()["GT-21"].control_price_caliber, Some(4_650_000));
    assert_eq!(eng.matrix()["GT-08"].control_price_caliber, Some(4_650_000));
    assert!(eng.try_accept("商务部分-报价").is_ok());

    // R5：幂等——同 sha 重复导入 no-op（version/DIRTY 均不动）
    let before_dirty = eng.dirty_sections();
    let again = eng.import_addendum("C-补遗文件01.docx", c_sha, &corpus_a_addendum());
    assert!(again.idempotent_noop);
    assert_eq!(eng.content_version, 2);
    assert_eq!(eng.dirty_sections(), before_dirty);
}

#[test]
fn c1_delete_add_rules_synthetic() {
    // R6（SYNTHETIC——D3 真实补遗链样本待建）：删除 → 绑定解除+行移除+恰一章节；
    // 新增（无章节归属）→ 新行 + 零章节误伤
    let mut eng = corpus_a_engine();
    let dirty = eng.clause_delete("GT-13");
    assert!(eng.reqs().get("GT-13").is_none());
    assert!(eng.bindings().get("GT-13").is_none());
    assert!(eng.matrix().get("GT-13").is_none());
    assert_eq!(dirty, vec!["售后服务方案"]);

    let dirty_add = eng.clause_add("SYN-NEW-01", "商务要求", "9.1", None);
    assert_eq!(eng.matrix()["SYN-NEW-01"].policy, "new_row");
    assert!(dirty_add.is_empty(), "无章节归属的新增零章节误伤");
}

// ================================================================ C2 证据关联只降不升

fn mk_locators(fps: &[&str]) -> HashMap<String, LinkLocator> {
    let mut m = HashMap::new();
    for (i, gt) in ["GT-01", "GT-02", "GT-05"].iter().enumerate() {
        m.insert(gt.to_string(), LinkLocator {
            paragraph_index: i,
            text_fingerprint: text_fingerprint(fps[i]),
        });
    }
    m
}

#[test]
fn c2_rewrite_degrades_exactly_one_chain_and_never_rises() {
    let locators = mk_locators(&[
        "见资质证书编号 D233012345。",
        "项目经理具有 PMP 证书。",
        "财务状况良好。",
    ]);
    let mut fingerprints: Vec<String> = vec![
        text_fingerprint("见资质证书编号 D233012345。"),
        text_fingerprint("项目经理具有 PMP 证书。"),
        text_fingerprint("财务状况良好。"),
    ];
    let chains: Vec<String> = locators.keys().cloned().collect();
    let mut ledger = LinkageLedger::new(&chains);

    // run 级改写：仅第 2 链所在段落被改 → 恰一链 stale_content（其余零误伤）
    fingerprints[1] = text_fingerprint("项目经理具有 PMP 证书（含培训记录）。");
    let degraded = ledger.refresh(true, &locators, &fingerprints);
    assert_eq!(degraded, vec!["GT-02"], "改写恰降一链——锚不跨链误伤");
    assert_eq!(ledger.status_of("GT-01"), Some(LinkageStatus::Valid));
    assert_eq!(ledger.status_of("GT-05"), Some(LinkageStatus::Valid));

    // undo：内容恢复原文 → Stale 不自发回升（只降不升）
    fingerprints[1] = text_fingerprint("项目经理具有 PMP 证书。");
    let re = ledger.refresh(true, &locators, &fingerprints);
    assert!(re.is_empty(), "恢复内容不产生新降级");
    assert_eq!(ledger.status_of("GT-02"), Some(LinkageStatus::Stale), "undo 后不得自发回升");

    // 显式重验（唯一回升通道）→ Valid
    assert!(ledger.reverify("GT-02", &locators["GT-02"], &fingerprints));
    assert!(ledger.all_valid());
}

#[test]
fn c2_external_change_no_silent_adopt() {
    // 外部修改重开（adopt 面）：指纹比对检测 → 不自动 adopt（R3-M-2：
    // 检测路径走 refresh 而非 setter——外部改动的真实入口是内容变化而非显式声明）
    let locators = mk_locators(&["a", "b", "c"]);
    let chains: Vec<String> = locators.keys().cloned().collect();
    let mut ledger = LinkageLedger::new(&chains);
    let fps_before = vec![text_fingerprint("a"), text_fingerprint("b"), text_fingerprint("c")];
    let _ = ledger.refresh(true, &locators, &fps_before);
    assert!(ledger.all_valid());
    // 外部编辑第 1 段（指纹变化）→ refresh 检出 → Stale + 审计有痕（不自动 adopt）
    let mut fps_after = fps_before.clone();
    fps_after[0] = text_fingerprint("a（外部编辑）");
    let degraded = ledger.refresh(true, &locators, &fps_after);
    assert_eq!(degraded, vec!["GT-01"], "外部修改检出恰一链");
    assert_eq!(ledger.status_of("GT-01"), Some(LinkageStatus::Stale), "不静默 adopt");
    assert!(!ledger.all_valid());
    assert!(matches!(
        ledger.audit.last(),
        Some(bid_domain::linkage::AuditEntry::BodyRefresh { .. })
    ));
    // adopt 裁决权归业务：显式重验（转人工复核后）才回升
    let new_loc = LinkLocator { paragraph_index: 0, text_fingerprint: fps_after[0].clone() };
    assert!(ledger.reverify("GT-01", &new_loc, &fps_after));
    assert!(ledger.all_valid());
}

// ================================================================ C3 多轮重写后锚点正确

#[test]
fn c3_five_round_rewrite_anchors_stay_correct() {
    // 五轮重写循环：每轮恰改写一条引用段（其余链 valid）——
    // 语义等价于 V3 Q1 的 DOCX 文件循环（此处为指纹向量层；文件级书签证据在 SEMUL）
    let base_texts = [
        "投标函详见附件一。",
        "资质响应文件真实有效。",
        "技术方案满足第五章要求。",
        "项目经理到场承诺书见附件二。",
        "售后服务网点覆盖全国。",
    ];
    let chains: Vec<String> = (1..=5).map(|i| format!("CIT-{i}")).collect();
    let mut locators = HashMap::new();
    for (i, cid) in chains.iter().enumerate() {
        locators.insert(cid.clone(), LinkLocator {
            paragraph_index: i,
            text_fingerprint: text_fingerprint(base_texts[i]),
        });
    }
    let mut ledger = LinkageLedger::new(&chains);
    let mut fingerprints: Vec<String> = base_texts.iter().map(|t| text_fingerprint(t)).collect();

    for round in 0..5 {
        let victim = chains[round].clone();
        // 本轮改写 victim 段（追加润色文本）——内联捕获不支持数组索引，显式参数
        let rewritten = format!("{}（第 {} 轮润色）", base_texts[round], round + 1);
        fingerprints[round] = text_fingerprint(&rewritten);
        // 书签语义：victim 的新位置 = 原 index（同段改写不位移）——重验用新指纹
        let degraded = ledger.refresh(true, &locators, &fingerprints);
        // 恰一链降级（本轮改写者；此前轮次已显式重验恢复）
        assert_eq!(degraded, vec![victim.clone()], "第 {} 轮恰一链降级", round + 1);
        // 显式重验：rebind 到当前段落（同位置新指纹）→ 恢复 Valid
        let new_locator = LinkLocator {
            paragraph_index: round,
            text_fingerprint: fingerprints[round].clone(),
        };
        assert!(ledger.reverify(&victim, &new_locator, &fingerprints), "重验必须成功");
        assert!(ledger.all_valid(), "每轮重验后全链恢复——下一轮基线干净");
        locators.insert(victim, new_locator);
        // 零悬空（R3-M-1 真化断言，重验后时点）：每条链的定位器必须指向**当前
        // 有效段**——索引在界内且指纹与该段当前指纹一致（victim 已 rebind，其余链未动）
        for c in &chains {
            let loc = &locators[c];
            assert!(loc.paragraph_index < fingerprints.len(),
                "书签 {c} 索引越界（悬空）");
            assert_eq!(loc.text_fingerprint, fingerprints[loc.paragraph_index],
                "书签 {c} 指纹失配（悬空——定位器与当前段不一致）");
        }
    }
    // 五轮后：全链 valid + 每轮审计有痕（降级 5 条 + 重验 5 条）
    assert!(ledger.all_valid());
    let degraded_events = ledger
        .audit
        .iter()
        .filter(|e| matches!(e, bid_domain::linkage::AuditEntry::BodyRefresh { .. }))
        .count();
    assert_eq!(degraded_events, 5);
}

/// SC-05（cpa-multi-plugins issue #3 吸收）：重写请求只可作用于请求内对象——
/// **请求外章节零触碰**四断言（定位器零重绑 / 指纹零变化 / 状态恒 Valid /
/// 审计零提及）。与 c3「恰一链」计数断言互补——c3 证「作用面不小于请求」，
/// 本测试证「作用面不大于请求」。反例锚：#3 的 `rewriteSystemInPlace`
/// 未过滤作用对象 + 一刀切替换 → 全部章节连带丢失；契约要求该形态必须
/// **响亮暴露**（全量降级审计），绝不允许被静默计为单章节重写。
/// 破坏推演：refresh 锚判定误跨链牵连（邻居段指纹比对错误）→ 零触碰断言红；
/// blanket 改写被压成单事件 → 反例断言红；Stale 自发回升 → 只降不升断言红。
#[test]
fn c3b_rewrite_request_scoped_out_of_target_untouched() {
    let base_texts = [
        "投标函详见附件一。",
        "资质响应文件真实有效。",
        "技术方案满足第五章要求。",
        "项目经理到场承诺书见附件二。",
        "售后服务网点覆盖全国。",
    ];
    let chains: Vec<String> = (1..=5).map(|i| format!("CIT-{i}")).collect();
    let mut locators = HashMap::new();
    for (i, cid) in chains.iter().enumerate() {
        locators.insert(cid.clone(), LinkLocator {
            paragraph_index: i,
            text_fingerprint: text_fingerprint(base_texts[i]),
        });
    }
    let mut ledger = LinkageLedger::new(&chains);
    let mut fingerprints: Vec<String> = base_texts.iter().map(|t| text_fingerprint(t)).collect();
    let before_locators = locators.clone();
    let before_fingerprints = fingerprints.clone();

    // ----（1）定向重写：请求只含第 3 章节（index 2）——请求外四章节零触碰
    let target = chains[2].clone();
    fingerprints[2] = text_fingerprint("技术方案全面满足第五章要求并附对比表。");
    let degraded = ledger.refresh(true, &locators, &fingerprints);
    assert_eq!(degraded, vec![target.clone()], "降级必须恰为请求内对象");
    for (c, loc) in &locators {
        if c == &target {
            continue;
        }
        assert_eq!(*loc, before_locators[c], "请求外链 {c} 定位器被重绑（越权改写）");
        assert_eq!(
            fingerprints[loc.paragraph_index], before_fingerprints[loc.paragraph_index],
            "请求外链 {c} 所指段指纹变化（越权改写）"
        );
        assert_eq!(ledger.status_of(c), Some(LinkageStatus::Valid),
            "请求外链 {c} 状态被动摇");
    }
    // 审计流只提及请求内对象（审计是越权改写的唯一观测面——必须完整）
    match ledger.audit.last() {
        Some(bid_domain::linkage::AuditEntry::BodyRefresh { degraded }) => {
            assert_eq!(degraded, &vec![target.clone()]);
        }
        other => panic!("定向改写后末条审计须为 BodyRefresh(恰一链)：{other:?}"),
    }

    // ----（2）反例响亮暴露（#3 一刀切形态）：全部段同时改写 →
    // 全部 Valid 链必须集体降级进审计——「全改了但只报一条」即静默越权
    for i in 0..5 {
        fingerprints[i] = text_fingerprint(&format!("{}（被一刀切改写）", base_texts[i]));
    }
    let degraded = ledger.refresh(true, &locators, &fingerprints);
    let mut rest = chains.clone();
    rest.remove(2); // target 已 Stale（只降不升——refresh 跳过）
    assert_eq!(degraded, rest, "一刀切改写必须响亮降级全部其余链——审计不得静默缩水");

    // ----（3）只降不升交叉断言：target 内容虽被（2）再次改写，仍是 Stale——
    // refresh 从不是回升通道（CIT-3 在（1）降级后经历两轮 refresh 不得自愈）
    assert_eq!(ledger.status_of(&target), Some(LinkageStatus::Stale));
    assert!(!ledger.all_valid());
}

// ================================================================ C1-real 真实补遗链（D3 公告级）

/// D3 真实补遗链（SC-07/SC-08 场景库 L 类落地，替换 V2-R6 的 SYNTHETIC 层）：
/// 中国传媒大学教学资源中心建设（项目编号 028 号）——招标公告 2026-07-13 →
/// 更正公告 2026-07-28 → 中标公告 2026-08-14，**全部字段可溯源**（公开公告
/// URL 见 real-samples/中传028真实采购链.json；grade=REAL_DATA 公告级）。
/// 更正内容（公告原文）：①开标 08-03→08-13；②实施节点一 07-31→08-30；
/// ③节点二 08-30→09-10；④**无预算变更**；⑤**「其他内容均无修改」**。
/// 破坏推演：DIRTY 扩散（多章节）→「其他内容均无修改」被违反即红；
/// 零翻转被翻/未变节点被动 → 零误翻断言红；重导入触碰任何字段 → SC-07 快照红。
/// 诚实边界：公告级（文件级招标文件需购买不入仓）；公告无可证的有效期证据
/// → R-date 重检真实运行但断言**零翻转**（不虚构到期证书）。
#[test]
fn c1_real_addendum_chain_cuc028() {
    use bid_domain::impact::AddendumEvent;
    let mut eng = ImpactEngine::cuc_2026_028();
    eng.prime_validity(); // 基线判定（基准日 2026-08-03）
    assert_eq!(eng.eval_date(), "2026-08-03");

    // sha 为公告标识（announce:<URL slug>）——公告级无文件哈希，诚实口径
    let fname = "中国传媒大学教学资源中心建设更正公告-20260728.htm";
    let sha = "announce:t20260728_27025731";
    let mut e1 = HashMap::new();
    e1.insert("milestone_date".to_string(), serde_json::json!("2026-08-30"));
    let mut e2 = HashMap::new();
    e2.insert("milestone_date".to_string(), serde_json::json!("2026-09-10"));
    let events = vec![
        AddendumEvent::ClauseModified { gt_id: "GT-S01".into(), params: e1 }, // 节点一 07-31→08-30
        AddendumEvent::ClauseModified { gt_id: "GT-S02".into(), params: e2 }, // 节点二 08-30→09-10
        AddendumEvent::EvalDateChanged { new_date: "2026-08-13".into() },     // 开标 08-03→08-13
        // 无 VersionConflict 事件——公告明确无预算变更（虚构冲突即失实）
    ];

    // ---- SC-07 前置快照：重导入前基线（幂等 ≠ 无害清空——diff 须限变更集）
    let impact = eng.import_events(fname, sha, &events);

    // R-mod/R-date 联合后的影响面：DIRTY 恰 {技术方案-实施计划}——
    // 公告「其他内容均无修改」= 零扩散的**真实世界确认**（商务报价/资格响应不动）
    let mut dirty = eng.dirty_sections();
    dirty.sort();
    assert_eq!(dirty, vec!["技术方案-实施计划"],
        "DIRTY 必须恰为被更正章节——「其他内容均无修改」的真实世界确认");
    assert!(impact.dirty_matrix.contains(&"GT-S01".to_string()));
    assert!(impact.dirty_matrix.contains(&"GT-S02".to_string()));
    assert!(!impact.dirty_matrix.contains(&"GT-S03".to_string()),
        "节点三 09-23 公告明示不变——不得入脏面");
    assert!(!impact.dirty_matrix.contains(&"GT-B01".to_string()),
        "预算未变更——商务行不得入脏面");

    // 版本戳恰 +1；基准日切换；无冲突登记（真实：无价格变更）
    assert_eq!(eng.content_version, 2);
    assert_eq!(eng.eval_date(), "2026-08-13");
    assert!(eng.conflicts().is_empty(), "公告无预算变更——不得虚构冲突");

    // latest_version_wins：被改节点新值生效；未改节点原值保持（9-23 不变）
    assert_eq!(eng.reqs()["GT-S01"].params["milestone_date"], serde_json::json!("2026-08-30"));
    assert_eq!(eng.reqs()["GT-S02"].params["milestone_date"], serde_json::json!("2026-09-10"));
    assert_eq!(eng.reqs()["GT-S03"].params["milestone_date"], serde_json::json!("2026-09-23"),
        "未变更节点保持原值（公告明示不变的节点被动即失实）");

    // R-date 全项目重检真实运行 + 零翻转（公告无可证有效期证据——不虚构翻转）
    assert!(eng.recheck_log.last().unwrap() == "2026-08-13");
    assert!(impact.flips.is_empty(), "零误翻：无可证到期面");
    assert_eq!(eng.bindings()["GT-Q01"].status, BindingStatus::Valid);

    // 门禁语义：被更正章节阻断 ACCEPTED；无关章节不受牵连（真实范围声明）
    assert!(matches!(eng.try_accept("技术方案-实施计划"), Err(AcceptBlock::Dirty)));
    eng.try_accept("资格响应").expect("无关章节可 ACCEPTED——零扩散的真实面");
    eng.try_accept("商务报价").expect("预算未变更——商务报价可 ACCEPTED");

    // 中标公告交叉核（真实世界一致性）：中标 105 万 ≤ 预算 108 万
    let award: i64 = 1_050_000; // 中标公告 2026-08-14 原文
    let budget = eng.reqs()["GT-B01"].params["budget"].as_i64().unwrap();
    assert!(award <= budget, "中标金额必须在预算内（真实世界一致性核验）");

    // ---- SC-07：同 sha 重导入 → no-op 且**零触碰**（幂等 ≠ 无害清空；
    // 刷新路径不得触碰未变更字段——全字段快照逐位比对）
    let snap_reqs = format!("{:?}", eng.reqs());
    let snap_bindings = format!("{:?}", eng.bindings());
    let snap_matrix = format!("{:?}", eng.matrix());
    let snap_sections = format!("{:?}", eng.sections());
    let snap_recheck = eng.recheck_log.clone();
    let reimport = eng.import_events(fname, sha, &events);
    assert!(reimport.idempotent_noop, "同 sha 重导入必须 no-op");
    assert_eq!(eng.content_version, 2, "no-op 不得动版本戳");
    assert_eq!(format!("{:?}", eng.reqs()), snap_reqs, "重导入触碰了 reqs（SC-07 违例）");
    assert_eq!(format!("{:?}", eng.bindings()), snap_bindings, "重导入触碰了 bindings");
    assert_eq!(format!("{:?}", eng.matrix()), snap_matrix, "重导入触碰了 matrix");
    assert_eq!(format!("{:?}", eng.sections()), snap_sections, "重导入触碰了 sections");
    assert_eq!(eng.recheck_log, snap_recheck, "重导入不得追加重检轮次");
}

// ================================================================ C4 导出中断不产生虚假成功

#[test]
fn c4_export_interrupt_never_fakes_exported() {
    // 中断注入：渲染后确认前"kill"（等效 V3 Q3 的 gate2.render 后不 confirm）
    let mut gate = ExportGate::new();
    let sha = gate.render(b"docx-artifact-bytes");
    assert_eq!(gate.state(), ExportPhase::Exporting, "中断后停 EXPORTING");
    assert!(!gate.is_exported(), "中断不得进入 EXPORTED");
    assert!(gate.artifact_sha().is_none(), "无落库就没有产物指纹——虚假成功条件封死");
    assert!(gate.consistent(), "不变式：EXPORTED ⟺ sha 落库");

    // 正常路径：confirm → EXPORTED + sha 落库
    assert!(gate.confirm());
    assert!(gate.is_exported());
    assert_eq!(gate.artifact_sha(), Some(sha.as_str()));
    assert!(gate.consistent());

    // 恢复路径（幂等）：重新确认同产物 → 同 sha
    assert!(gate.confirm());
    assert_eq!(gate.artifact_sha(), Some(sha.as_str()));

    // 独立会话重导同产物 → 同 sha（同产物幂等）
    let mut g2 = ExportGate::new();
    let sha2 = g2.render(b"docx-artifact-bytes");
    assert_eq!(sha, sha2, "同产物 sha 必须一致");
    g2.confirm();
    assert_eq!(g2.artifact_sha(), gate.artifact_sha());
}

/// SC-01（issues #35/#12 吸收）：导出中断后**任意查询面**（进度查询/产物列表/
/// 状态接口）在中间态的返回——必须显式 EXPORTING 或缺失标记，禁止缺省成功；
/// 进度面与产物面分离（产物未落库 ≠ 导出失败，未确认 ≠ EXPORTED）。
/// 破坏推演：query_view 中间态映射到 Exported/NotStarted → 显式 EXPORTING
/// 断言红；preview 泄漏为 artifact 语义（或反之）→ 分离断言红。
#[test]
fn c4b_query_face_explicit_exporting_no_default_success() {
    use bid_domain::export_gate::ExportQueryView;

    // 态 1：未发起——查询面读 NotStarted（无任何「成功」暗示）
    let mut gate = ExportGate::new();
    assert_eq!(gate.query_view(), ExportQueryView::NotStarted);
    assert!(!gate.is_exported());

    // 态 2：渲染完成、确认前中断（V3 Q3 中断注入点）——任何查询面
    // 必须读到**显式 EXPORTING**：禁止缺省成功（is_exported 恒 false、
    // 产物面恒空），进度面 preview_sha 可见而 artifact 语义不可得
    let preview = gate.render(b"docx-artifact-bytes");
    assert_eq!(
        gate.query_view(),
        ExportQueryView::Exporting { preview_sha: preview.clone() },
        "中间态查询必须显式 EXPORTING（携带进度面预览指纹）"
    );
    assert!(!gate.is_exported(), "未确认 ≠ EXPORTED");
    assert!(gate.artifact_sha().is_none(), "产物面缺失标记（≠ 导出失败——进度面仍活）");
    // preview_sha 是进度面口径：与产物面同值但**不同语义通道**——
    // 中间态下任何查询面都不得把它当 artifact 语义消费（分离纪律）

    // 态 3：确认落库——查询面才切 Exported（产物面出现）
    assert!(gate.confirm());
    assert_eq!(
        gate.query_view(),
        ExportQueryView::Exported { artifact_sha: preview.clone() },
        "EXPORTED 仅当产物 sha 落库（同产物 preview == artifact）"
    );
    assert!(gate.is_exported());

    // 跨中断查询稳定性：中断态（新门同产物）进度查询与前次 preview 一致 →
    // 恢复确认后产物面与前次 preview 同源同值（幂等恢复的查询面投影）
    let mut resumed = ExportGate::new();
    let p2 = resumed.render(b"docx-artifact-bytes");
    assert_eq!(p2, preview, "同产物 preview 跨会话一致");
    assert_eq!(resumed.query_view(),
        ExportQueryView::Exporting { preview_sha: p2 });
    resumed.confirm();
    match resumed.query_view() {
        ExportQueryView::Exported { artifact_sha } => assert_eq!(artifact_sha, preview),
        other => panic!("确认后查询面必须为 Exported：{other:?}"),
    }
}

#[test]
fn c4_exported_state_alignment_with_project_status() {
    // 导出门与 ProjectStatus 状态机的一致性：Exported 为终态且
    // ReadyToExport → Exported 是唯一入边（guard_tool 越级防护面）
    assert_eq!(ProjectStatus::Exported.next_allowed(), &[]);
    assert!(ProjectStatus::ReadyToExport.next_allowed().contains(&ProjectStatus::Exported));
    // 非法路径：Drafting 不得直接到 Exported
    assert!(!ProjectStatus::Drafting.next_allowed().contains(&ProjectStatus::Exported));
    // 治理失败（T-04 五分类）与导出门的语义闭环：Timeout/Overflow/NonZero
    // 均 Err → parse 不到 Ok → 不可能触发 confirm（guard 测试 timeout_never_lies_as_success
    // 已证）。此处断言状态机不接受"非 ReadyToExport"的导出意图。
    assert!(!ProjectStatus::FilesImported.next_allowed().contains(&ProjectStatus::Exported));
}
