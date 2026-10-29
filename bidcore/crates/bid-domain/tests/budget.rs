//! budget 集成测试 —— S-7 上下文预算与证据裁剪的验收面（issue #11 V3-5）。
//!
//! 黑盒口径：全部走 `bid_domain::budget` 的 pub API（不触私有实现）。
//! 覆盖（对应 issue #11 S-7 验收原文）：
//! 1. 溢出四阶梯的精确分界（全装下 / 恰降摘要 / 恰丢低相关 / 三态并存）；
//! 2. 约束区永不截断（超红线 = 硬错误；吃光规划目标 = overflow 裁决位）；
//! 3. 确定性（同输入两次 plan 全等；平局按 id 字典序——入参顺序无关）；
//! 4. fail-closed（borrow：mdcore `workbench/tests/persistence_recovery.rs`
//!    的响亮拒绝模式——重复 id / 空 section id / token 账不平全拒不 panic）；
//! 5. ContextPlan 版本门（borrow：mdcore 版本单调纪律——serde roundtrip 全等、
//!    未知 schema_version 拒载、缺省版本按当前版加载——M3 落盘前向兼容面）。

use bid_domain::budget::{
    estimate_tokens, plan, BudgetError, BudgetInput, ConstraintBlock, ContextBudget,
    ContextPlan, EvidenceCandidate, EvidenceMode, SectionSpecBlock, CONTEXT_PLAN_SCHEMA_VERSION,
};

/// 生成恰好 n tokens 的中文文本（n 个 CJK 字 = n tokens——口径的构造面）。
fn cjk(n: usize) -> String {
    "证".repeat(n)
}

/// 基础输入：约束 9 + 规格 2 tokens；前缀计量 = 9 + 2 + 128(头) + 16(块标签) = 155。
/// 预算 total=1,000 / headroom 10% → 规划目标 900 → evidence_budget = 745。
fn base_input() -> BudgetInput {
    BudgetInput {
        constraints: vec![ConstraintBlock::new("金额", cjk(9))],
        section_spec: SectionSpecBlock::new("SEC-0001", cjk(2)),
        candidates: vec![],
    }
}

fn small_budget() -> ContextBudget {
    ContextBudget { total: 1_000, constraint_reserve_headroom_pct: 10 }
}

/// 阶梯 b（全装下档）：总量恰在预算内 → 全 Full、零裁剪、零溢出；标记开销计入计量。
#[test]
fn tier_b_all_full_fits_with_marker_overhead() {
    // full 各 200：(200+24)×3 = 672 ≤ 745 → 全装下
    let mut input = base_input();
    input.candidates = vec![
        EvidenceCandidate::new("EV-0001", 0.9, cjk(200), vec![cjk(50)]),
        EvidenceCandidate::new("EV-0002", 0.5, cjk(200), vec![cjk(50)]),
        EvidenceCandidate::new("EV-0003", 0.1, cjk(200), vec![cjk(50)]),
    ];
    let p = plan(&input, &small_budget()).unwrap();
    assert_eq!(
        p.evidence_plan.iter().map(|e| e.mode).collect::<Vec<_>>(),
        vec![EvidenceMode::Full, EvidenceMode::Full, EvidenceMode::Full],
        "总量装得下 → 全 Full（top-3 由预算反推）"
    );
    assert!(!p.truncated_evidence, "全装下 = 无降级无丢弃 = 无裁剪");
    assert!(!p.overflow);
    // 计量对账：渲染标记开销计入（不是裸内容 token）
    assert_eq!(p.prefix_tokens, 155, "9(约束)+2(规格)+128(头)+16(块标签)");
    assert_eq!(p.total_tokens, 155 + 3 * (200 + 24), "每条 Full 计 200+24（块头标记）");
    assert_eq!(p.schema_version, CONTEXT_PLAN_SCHEMA_VERSION);
    assert_eq!((p.budget_total, p.planning_target), (1_000, 900));
}

/// 阶梯 c（恰降摘要档）：第 3 条 full 装不下、summary 装得下 → F/F/S。
#[test]
fn tier_c_exact_summary_boundary() {
    // b：(250+24)×3 = 822 > 745 → 只装前两条（548）；
    // c：第 3 条 summary (100+24) = 124 → 548+124 = 672 ≤ 745 → Summary
    let mut input = base_input();
    input.candidates = vec![
        EvidenceCandidate::new("EV-0001", 0.9, cjk(250), vec![cjk(100)]),
        EvidenceCandidate::new("EV-0002", 0.5, cjk(250), vec![cjk(100)]),
        EvidenceCandidate::new("EV-0003", 0.1, cjk(250), vec![cjk(100)]),
    ];
    let p = plan(&input, &small_budget()).unwrap();
    assert_eq!(
        p.evidence_plan.iter().map(|e| e.mode).collect::<Vec<_>>(),
        vec![EvidenceMode::Full, EvidenceMode::Full, EvidenceMode::Summary],
        "长证据降级 full→summary（高相关的优先保 summary）"
    );
    assert!(p.truncated_evidence, "降级即裁剪面——验收要求标 truncated_evidence");
    assert!(!p.overflow, "155+672 = 827 ≤ 900 规划目标");
    assert_eq!(p.total_tokens, 155 + 548 + 124);
}

/// 阶梯 c→d（恰丢低相关档）：summary 也装不下 → 低相关整条丢。
#[test]
fn tier_c_then_d_drop_lowest_relevance() {
    // b：E1/E2 full (250+24)×2 = 548；E3 full 装不下（822 > 745）；
    // c：E3 summary (300+24) = 324 → 548+324 = 872 > 745
    // d：砍 E3（唯一降级候选，最低相关）→ 598…不对：548 ≤ 745 ✓ → Dropped
    let mut input = base_input();
    input.candidates = vec![
        EvidenceCandidate::new("EV-0001", 0.9, cjk(250), vec![cjk(300)]),
        EvidenceCandidate::new("EV-0002", 0.5, cjk(250), vec![cjk(300)]),
        EvidenceCandidate::new("EV-0003", 0.1, cjk(250), vec![cjk(300)]),
    ];
    let p = plan(&input, &small_budget()).unwrap();
    assert_eq!(
        p.evidence_plan.iter().map(|e| e.mode).collect::<Vec<_>>(),
        vec![EvidenceMode::Full, EvidenceMode::Full, EvidenceMode::Dropped],
        "summary 装不下 → 低相关整条丢（先砍低相关）"
    );
    assert!(p.truncated_evidence);
    // dropped 清单行开销计入保守计量：155 + 548 + (32 + 4×1) = 739
    assert_eq!(p.total_tokens, 155 + 548 + 36);
}

/// 四阶梯并存分界：高相关 Full / 中相关 Summary / 低相关 Dropped 的精确切面。
#[test]
fn three_modes_coexist_precise_boundary() {
    // b：E1 (400+24) = 424 入；E2 (400+24) → 848 > 745 停（top-1，N 由预算反推）；
    // c：E2/E3 全转 summary (150+24)+(300+24) = 498 → 424+498 = 922 > 745
    //    → 从低相关端丢 E3 → 424+174 = 598 ≤ 745；
    // d：无需再砍。结果恰为 Full/Summary/Dropped 三态并存。
    let mut input = base_input();
    input.candidates = vec![
        EvidenceCandidate::new("EV-0001", 0.9, cjk(400), vec![cjk(150)]),
        EvidenceCandidate::new("EV-0002", 0.5, cjk(400), vec![cjk(150)]),
        EvidenceCandidate::new("EV-0003", 0.1, cjk(400), vec![cjk(300)]),
    ];
    let p = plan(&input, &small_budget()).unwrap();
    assert_eq!(
        p.evidence_plan.iter().map(|e| e.mode).collect::<Vec<_>>(),
        vec![EvidenceMode::Full, EvidenceMode::Summary, EvidenceMode::Dropped],
        "高相关 Full / 中相关 Summary / 低相关 Dropped 的精确分界"
    );
    assert_eq!(p.evidence_plan[0].tokens, 400 + 24);
    assert_eq!(p.evidence_plan[1].tokens, 150 + 24);
    assert_eq!(p.evidence_plan[2].tokens, 0, "Dropped 不占证据预算");
    assert_eq!(p.total_tokens, 155 + 424 + 174 + 32 + 4);
    // 输出序 = (relevance desc)——可 diff 的确定性形态
    assert_eq!(
        p.evidence_plan.iter().map(|e| e.evidence_id.as_str()).collect::<Vec<_>>(),
        vec!["EV-0001", "EV-0002", "EV-0003"]
    );
}

/// 阶梯 a 硬错误：约束区超红线 → Err（绝不静默截断约束、绝不悄悄放行超限）。
#[test]
fn constraints_overflow_is_hard_error() {
    // 约束 950 + 规格 2 + 标记 144 = 1096 > 红线 1000
    let mut input = base_input();
    input.constraints = vec![ConstraintBlock::new("金额", cjk(950))];
    let err = match plan(&input, &small_budget()) {
        Ok(_) => panic!("约束区超红线必须硬错误——绝不悄悄放行超限"),
        Err(e) => e,
    };
    assert_eq!(err, BudgetError::ConstraintOverflow { prefix_tokens: 1_096, total: 1_000 });
    let msg = format!("{err}");
    assert!(msg.contains("约束区"), "错误信息指名约束区: {msg}");
    assert!(msg.contains("绝不静默截断"), "错误信息必须说明绝不静默截断: {msg}");
}

/// overflow 裁决位：约束区吃光规划目标（未超红线）→ 证据预算 0、全部丢弃、
/// 计划带 overflow: true（非失败——调用方裁决；红线由渲染实测硬守）。
#[test]
fn overflow_flag_when_prefix_eats_planning_target() {
    // 约束 800 + 规格 2 + 标记 144 = 946：≤ 红线 1000（不 Err），
    // 但 > 规划目标 900 → evidence_budget = 0 → 唯一证据整条丢
    let mut input = base_input();
    input.constraints = vec![ConstraintBlock::new("金额", cjk(800))];
    input.candidates = vec![EvidenceCandidate::new("EV-0001", 0.9, cjk(100), vec![cjk(20)])];
    let p = plan(&input, &small_budget()).unwrap();
    assert_eq!(p.evidence_plan[0].mode, EvidenceMode::Dropped, "证据预算 0 → 全裁");
    assert!(p.truncated_evidence);
    assert!(p.overflow, "计量 946+36 = 982 > 规划目标 900——安全边际被吃");
    assert_eq!(p.total_tokens, 946 + 32 + 4);
}

/// 确定性（验收要求）：同输入两次 plan 逐字段相等；50 条同 relevance 证据
/// 打乱入参顺序 → 计划不变（平局按 evidence_id 字典序的证明——结果与
/// 入参顺序、容器迭代序都无关）。
#[test]
fn deterministic_plan_and_tie_break_by_id() {
    let make = |ids: &[&str]| BudgetInput {
        constraints: vec![ConstraintBlock::new("金额", cjk(9))],
        section_spec: SectionSpecBlock::new("SEC-0001", cjk(2)),
        candidates: ids.iter().map(|id| {
            // 全部同 relevance（0.5）——平局面；full 长度由 **id** 决定（51..100），
            // 与入参位置无关——打乱后候选集逐字段不变，计划若变只能是顺序影响裁决。
            let n: usize = id.parse().unwrap();
            EvidenceCandidate::new(format!("EV-{id}"), 0.5, cjk(50 + n), vec![cjk(10)])
        }).collect(),
    };
    let ids: Vec<String> = (1..=50).map(|i| format!("{i:04}")).collect();
    let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    let p1 = plan(&make(&refs), &ContextBudget::default()).unwrap();
    let p2 = plan(&make(&refs), &ContextBudget::default()).unwrap();
    assert_eq!(p1, p2, "同输入两次 plan 必须逐字段相等（可复现是验收前提）");

    // 打乱入参顺序（逆序 + 错位旋转——同 relevance 集合）
    let mut shuffled = refs.clone();
    shuffled.reverse();
    shuffled.rotate_left(7);
    let p3 = plan(&make(&shuffled), &ContextBudget::default()).unwrap();
    assert_eq!(p1, p3, "平局按 id 字典序——计划与入参顺序无关");

    // 输出序证明：平局下 evidence_plan 的 id 序必须是字典序
    let out_ids: Vec<&str> = p1.evidence_plan.iter().map(|e| e.evidence_id.as_str()).collect();
    let mut expect: Vec<String> = refs.iter().map(|r| format!("EV-{r}")).collect();
    expect.sort();
    assert_eq!(out_ids, expect, "relevance 平局 → id 字典序输出");
}

/// relevance 高低与 id 序的对抗：id 字典序小但 relevance 低 → 仍排在后
/// （relevance 优先，id 只破平局）。
#[test]
fn relevance_dominates_id_ordering() {
    let mut input = base_input();
    input.candidates = vec![
        EvidenceCandidate::new("EV-0001", 0.1, cjk(50), vec![cjk(10)]),
        EvidenceCandidate::new("EV-0002", 0.9, cjk(50), vec![cjk(10)]),
    ];
    let p = plan(&input, &small_budget()).unwrap();
    assert_eq!(
        p.evidence_plan.iter().map(|e| e.evidence_id.as_str()).collect::<Vec<_>>(),
        vec!["EV-0002", "EV-0001"],
        "relevance 降序优先，id 字典序只破平局"
    );
}

/// fail-closed（borrow：mdcore `persistence_recovery.rs` 的响亮拒绝模式——
/// 损坏/非法输入全拒，不 panic 不静默）：重复证据 id / 空 section id /
/// token 字段谎报 → 全部 Err。
#[test]
fn fail_closed_bad_inputs_rejected_loudly() {
    let budget = ContextBudget::default();
    // 重复 evidence_id（平局破序前提被破坏——确定性失效面）
    let mut input = base_input();
    input.candidates = vec![
        EvidenceCandidate::new("EV-0001", 0.9, cjk(10), vec![]),
        EvidenceCandidate::new("EV-0001", 0.5, cjk(10), vec![]),
    ];
    assert_eq!(
        plan(&input, &budget),
        Err(BudgetError::DuplicateEvidenceId { id: "EV-0001".into() })
    );
    // 空 section id（含纯空白）——单章输入必须有明确归属
    for bad in ["", "   "] {
        let mut input = base_input();
        input.section_spec = SectionSpecBlock::new(bad, cjk(2));
        assert_eq!(plan(&input, &budget), Err(BudgetError::EmptySectionId));
    }
    // token 账不平：手工构造谎报 tokens_full（谎报预算 = 红线失守面）
    let mut input = base_input();
    let mut c = EvidenceCandidate::new("EV-0001", 0.9, cjk(100), vec![cjk(30)]);
    c.tokens_full = 1;
    input.candidates = vec![c];
    assert!(matches!(
        plan(&input, &budget),
        Err(BudgetError::TokenAccountingMismatch { .. })
    ));
    // 谎报 tokens_summary 同拒
    let mut input = base_input();
    let mut c = EvidenceCandidate::new("EV-0001", 0.9, cjk(100), vec![cjk(30)]);
    c.tokens_summary = 999;
    input.candidates = vec![c];
    assert!(matches!(
        plan(&input, &budget),
        Err(BudgetError::TokenAccountingMismatch { .. })
    ));
    // 谎报约束 tokens 同拒
    let mut input = base_input();
    input.constraints = vec![ConstraintBlock { kind: "金额".into(), text: cjk(9), tokens: 3 }];
    assert!(matches!(
        plan(&input, &budget),
        Err(BudgetError::TokenAccountingMismatch { .. })
    ));
}

/// 版本门（borrow：mdcore 版本单调纪律 + 本仓 store_io `schema_version_gate`
/// 同款）：ContextPlan serde roundtrip 全等；未知版本反序列化拒（未来数据遇
/// 旧代码——不猜测降级加载）；缺省版本字段按当前版加载（M3 落盘的加性面）。
#[test]
fn context_plan_schema_version_gate() {
    let mut input = base_input();
    input.candidates = vec![EvidenceCandidate::new("EV-0001", 0.9, cjk(50), vec![cjk(10)])];
    let p = plan(&input, &small_budget()).unwrap();
    // roundtrip 全等（序列化面是 M3 起草落盘的前向兼容地基）
    let json = serde_json::to_string(&p).unwrap();
    let back: ContextPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p, "ContextPlan serde roundtrip 必须全等");
    // 未知版本（高于当前支持）→ 响亮拒绝
    let future = json.replace(
        &format!("\"schema_version\":{CONTEXT_PLAN_SCHEMA_VERSION}"),
        "\"schema_version\":99",
    );
    let err = match serde_json::from_str::<ContextPlan>(&future) {
        Ok(_) => panic!("未知 schema_version 必须拒载（fail-closed 不猜测）"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("高于当前支持"), "错误面可辨识: {err}");
    // 缺省版本字段 → 按当前版加载（旧数据的加性兼容面）
    let legacy = json.replace(
        &format!("\"schema_version\":{CONTEXT_PLAN_SCHEMA_VERSION},"), "",
    );
    let back: ContextPlan = serde_json::from_str(&legacy).unwrap();
    assert_eq!(back.schema_version, CONTEXT_PLAN_SCHEMA_VERSION);
    assert_eq!(back, p, "缺省版本加载后语义全等");
}

/// 便捷构造器的 token 预计算与 estimate_tokens 单一口径自洽
/// （tokens 字段是副本——plan 的账目校验依赖两者恒等，此处置钉口径）。
#[test]
fn constructor_tokens_match_estimate_single_source() {
    let c = ConstraintBlock::new("金额", "控制价三百五十万元整");
    assert_eq!(c.tokens, estimate_tokens(&c.text));
    let s = SectionSpecBlock::new("SEC-0001", "章节规格文本");
    assert_eq!(s.tokens, estimate_tokens(&s.text));
    let e = EvidenceCandidate::new("EV-0001", 0.5, "全文内容", vec!["段一".into(), "段二".into()]);
    assert_eq!(e.tokens_full, estimate_tokens(&e.full_text));
    assert_eq!(e.tokens_summary, estimate_tokens(&e.summary_paragraphs.join("\n")));
}

/// 空候选集：无证据 → 空 evidence_plan、无裁剪标记（诚实空态，不是错误）。
#[test]
fn empty_candidates_is_clean_empty_plan() {
    let p = plan(&base_input(), &small_budget()).unwrap();
    assert!(p.evidence_plan.is_empty());
    assert!(!p.truncated_evidence);
    assert!(!p.overflow);
    assert_eq!(p.total_tokens, 155);
}
