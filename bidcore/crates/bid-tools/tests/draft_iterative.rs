//! draft_iterative.rs —— B4：生成-自评-改写循环（issue #97）。
//!
//! 验收线（issue #97 §B4 验收）：
//! - 同一章节：单轮 vs 迭代，得分提升 > 15%（自适应 mock——读改写提示补强）；
//! - 每轮得分与改进可追溯（RoundRecord.rounds）；
//! - 达 target 提前停（不浪费轮次——LLM 调用次数 = 轮次）；
//! - 不超 max_rounds（硬上限防死循环——构造 clamp 到 [1,8]）；
//! - 无映射评分项提前停（不空转）；改写回归保最优轮；
//! - 服务面 e2e：最优稿经 draft_section 既有路径落库（状态/引用/字数）。
//!
//! mock 纪律（issue §六-2）：无 LLM key 环境——ScriptedGenerator 验循环
//! 控制，AdaptiveGenerator 验「改写提示真的携带失分项信号」（端到端反馈面）。

use std::collections::HashMap;

use bid_domain::evidence::EvidenceSpec;
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineNode;
use bid_domain::scoring::{derive_scoring_tree, SectionScorer};
use bid_domain::{Anchor, ParseStatus, Requirement};
use bid_tools::draft::{
    build_rewrite_prompt, GeneratedDraft, IterativeDrafter, SectionDraftInput, SectionGenerator,
    MAX_ROUNDS_CAP,
};
use bid_tools::quality::ProjectContext;
use bid_tools::service::BidService;

// ---------------------------------------------------------------- 夹具

/// 评分项 REQ（B2 槽位：score + section + criteria）。
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

/// 两评分项映射 SEC-0001（合计满分 20）。
fn reqs_for_sec1() -> Vec<Requirement> {
    vec![
        scoring_req("REQ-0101", 10.0, "SEC-0001", "技术方案完整性与针对性 10 分"),
        scoring_req("REQ-0102", 10.0, "SEC-0001", "关键设备配置与性能参数 10 分"),
    ]
}

fn project() -> ProjectContext {
    ProjectContext {
        project_name: "塔里木水库除险加固".into(),
        tenderer_name: String::new(),
        extra_terms: vec!["SL-2026-018".into()],
    }
}

fn scorer_for_sec1() -> SectionScorer {
    let p = project();
    SectionScorer::new(derive_scoring_tree(&reqs_for_sec1()), p.scorer_terms())
}

fn sample_input() -> SectionDraftInput {
    SectionDraftInput {
        section_id: "SEC-0001".into(),
        title: "技术方案".into(),
        target_chars: 600,
        requirements: vec![bid_tools::draft::ReqBrief {
            id: "REQ-0101".into(),
            category: "评分项".into(),
            excerpt: "技术方案完整性与针对性 10 分".into(),
        }],
        evidences: vec![bid_tools::draft::EvBrief {
            id: "EV-0001".into(),
            kind: "资质".into(),
            title: "水利水电施工总承包贰级资质".into(),
            scope: "水利水电工程".into(),
        }],
        instructions: None,
    }
}

// ---------------------------------------------------------------- mock 生成器

/// 剧本生成器（队列逐轮取稿——循环控制面：轮次/调用数/提前停）。
struct Scripted {
    drafts: Vec<String>,
    calls: usize,
    last_instructions: Option<String>,
}

impl Scripted {
    fn new(drafts: Vec<String>) -> Self {
        Scripted { drafts, calls: 0, last_instructions: None }
    }
}

impl SectionGenerator for Scripted {
    fn generate(&mut self, input: &SectionDraftInput) -> Result<GeneratedDraft, String> {
        self.calls += 1;
        self.last_instructions = input.instructions.clone();
        let idx = (self.calls - 1).min(self.drafts.len() - 1);
        Ok(GeneratedDraft { content: self.drafts[idx].clone() })
    }
}

/// 差稿（无关键词/无专名/无参数/无证据——差档 0.275）。
fn filler_draft() -> String {
    let mut s = String::from("# 技术方案\n\n我司高度重视本项目，将组建优秀团队，科学管理，精心组织。");
    while s.chars().filter(|c| !c.is_whitespace()).count() < 600 {
        s.push_str("我司将秉承诚信原则，提供一流的服务，全力以赴，确保万无一失。");
    }
    s
}

/// 中稿（全覆盖 + 字数 + 证据，无具体性——良档 0.825）。
fn good_no_specific_draft() -> String {
    let mut s = String::from(
        "# 技术方案\n\n本章论述技术方案完整性与针对性，并说明关键设备配置与性能参数。\n\n\
        技术方案完整性方面：覆盖全部建设内容的实施方案。关键设备配置与性能参数\
        方面：满足招标文件要求。同类业绩见 [EV-0001]。",
    );
    while s.chars().filter(|c| !c.is_whitespace()).count() < 600 {
        s.push_str("技术方案完整性与针对性、关键设备配置与性能参数方面的安排持续展开说明。");
    }
    s
}

/// 优稿（全覆盖 + 字数 + 证据 + 具体性——优档 0.95）。
fn excellent_draft() -> String {
    let mut s = format!(
        "# 技术方案\n\n本章针对塔里木水库除险加固工程论述技术方案完整性与针对性。\n\n\
        关键设备配置与性能参数：振动沉模机 2 台、混凝土拌合站 2 座、管理人员 28 名，\
        工期 180 日历天。同类业绩见 [EV-0001]（水利水电施工总承包贰级）。"
    );
    while s.chars().filter(|c| !c.is_whitespace()).count() < 600 {
        s.push_str("技术方案完整性与针对性、关键设备配置与性能参数的落实路径在同类项目中已标准化。");
    }
    s
}

/// 自适应生成器（读改写提示补强——验证「提示真的携带失分项信号」）：
/// 无指令 → 通用差稿；有指令 → 提示中「…」引用的评分标准/项目名逐条论述 +
/// 真实 REQ/EV 引用（白名单内的编号——不从提示外的上下文编造）。
struct Adaptive {
    calls: usize,
    last_instructions: Option<String>,
}

impl SectionGenerator for Adaptive {
    fn generate(&mut self, input: &SectionDraftInput) -> Result<GeneratedDraft, String> {
        self.calls += 1;
        self.last_instructions = input.instructions.clone();
        let content = match &input.instructions {
            None => filler_draft(),
            Some(prompt) => adapt_to_prompt(prompt, input),
        };
        Ok(GeneratedDraft { content })
    }
}

/// 从改写提示提取「…」引用（评分标准原文/项目名——build_rewrite_prompt
/// 的失分项信号面），逐条生成论述段（含参数/证据/引用）。
fn adapt_to_prompt(prompt: &str, input: &SectionDraftInput) -> String {
    let open = '「'.len_utf8();
    let quoted: Vec<&str> = prompt
        .match_indices('「')
        .filter_map(|(i, _)| {
            let rest = &prompt[i + open..];
            rest.find('」').map(|j| &rest[..j])
        })
        .collect();
    let req_refs: Vec<String> =
        input.requirements.iter().map(|r| format!("[{}]", r.id)).collect();
    let ev_refs: Vec<String> =
        input.evidences.iter().map(|e| format!("[{}]", e.id)).collect();
    let mut paras: Vec<String> = vec!["# 技术方案".to_string()];
    for q in &quoted {
        paras.push(format!(
            "针对{q}的要求（响应{}），本方案安排如下：管理人员 28 名、主要设备 45 台套、\
            施工工期 180 日历天、核心指标满足率 100%；同类项目业绩与资质证明见{}，\
            覆盖该评分标准的全部主题词且优于要求。",
            req_refs.join("、"),
            if ev_refs.is_empty() { "证据原件".to_string() } else { ev_refs.join("、") },
        ));
    }
    let mut s = paras.join("\n\n");
    // 填充到达标线（填充不含判定特征——不虚增覆盖/证据/具体性）
    while s.chars().filter(|c| !c.is_whitespace()).count() < 600 {
        s.push_str("上述安排在本单位同类项目中已形成标准化执行路径并持续改进。");
    }
    s
}

// ---------------------------------------------------------------- 循环控制

#[test]
fn three_rounds_progress_and_reach_target() {
    let scorer = scorer_for_sec1();
    let p = project();
    let mut gen = Scripted::new(vec![filler_draft(), good_no_specific_draft(), excellent_draft()]);
    let mut it = IterativeDrafter::new(&mut gen, &scorer, &p, 3, 0.85);
    let out = it.draft(&sample_input()).unwrap();

    assert_eq!(out.rounds.len(), 3, "三轮走满（0.275 → 0.825 → 0.95）");
    assert_eq!(gen.calls, 3, "LLM 调用次数 = 轮次（3 轮 = 3 次）");
    assert!(out.reached_target, "第三轮达标");
    // 得分递进可追溯
    assert!(out.rounds[0].ratio < out.rounds[1].ratio && out.rounds[1].ratio < out.rounds[2].ratio,
        "逐轮提升：{:?}", out.rounds.iter().map(|r| r.ratio).collect::<Vec<_>>());
    assert!((out.rounds[0].ratio - 0.275).abs() < 1e-6, "差档 0.275（实测 {:.3}）", out.rounds[0].ratio);
    assert!((out.rounds[1].ratio - 0.825).abs() < 1e-6, "良档 0.825（实测 {:.3}）", out.rounds[1].ratio);
    assert!((out.rounds[2].ratio - 0.95).abs() < 1e-6, "优档 0.95（实测 {:.3}）", out.rounds[2].ratio);
    // changes 可追溯
    assert!(out.rounds[0].changes.contains("首轮生成"), "首轮 = 基线说明");
    assert!(out.rounds[1].changes.contains("第 2 轮改写") && out.rounds[1].changes.contains("失分项"),
        "改写轮说明针对什么改：{}", out.rounds[1].changes);
    // 最终内容 = 最优（达标）轮
    assert_eq!(out.content, excellent_draft(), "达标轮内容落稿");
}

#[test]
fn target_reached_early_stops_and_saves_rounds() {
    let scorer = scorer_for_sec1();
    let p = project();
    let mut gen = Scripted::new(vec![filler_draft(), excellent_draft(), excellent_draft()]);
    let mut it = IterativeDrafter::new(&mut gen, &scorer, &p, 3, 0.85);
    let out = it.draft(&sample_input()).unwrap();

    assert_eq!(out.rounds.len(), 2, "第二轮达标即停（不浪费轮次）");
    assert_eq!(gen.calls, 2, "LLM 调用 = 2 次（省 1 次）");
    assert!(out.reached_target);
    assert!(out.notes.is_empty(), "正常路径无诚实登记噪声：{:?}", out.notes);
}

#[test]
fn hard_cap_stops_loop_and_constructor_clamps() {
    let scorer = scorer_for_sec1();
    let p = project();
    // 构造 clamp：0 → 1；超上限 → MAX_ROUNDS_CAP
    {
        let mut gen = Scripted::new(vec![filler_draft()]);
        let it = IterativeDrafter::new(&mut gen, &scorer, &p, 0, 0.9);
        assert_eq!(it.max_rounds, 1, "max_rounds=0 → clamp 1（零轮无意义）");
    }
    {
        let mut gen = Scripted::new(vec![filler_draft()]);
        let mut it = IterativeDrafter::new(&mut gen, &scorer, &p, 100, 0.9);
        assert_eq!(it.max_rounds, MAX_ROUNDS_CAP, "max_rounds=100 → clamp 8");
        let out = it.draft(&sample_input()).unwrap();
        assert_eq!(out.rounds.len(), MAX_ROUNDS_CAP, "行为面同样被 cap（不无限循环）");
        assert_eq!(gen.calls, MAX_ROUNDS_CAP);
        assert!(!out.reached_target);
        assert!(out.notes.iter().any(|n| n.contains("硬上限")), "达上限登记：{:?}", out.notes);
    }
    // 正常上限
    let mut gen = Scripted::new(vec![filler_draft()]);
    let mut it = IterativeDrafter::new(&mut gen, &scorer, &p, 3, 0.95);
    let out = it.draft(&sample_input()).unwrap();
    assert_eq!(out.rounds.len(), 3, "max_rounds=3 走满");
    assert!(!out.reached_target, "差稿永不达标");
    assert_eq!(gen.calls, 3);
}

#[test]
fn no_scoring_items_stops_after_first_round() {
    // 评分项映射 SEC-9999（无 SEC-0001 的项）→ 自评无优化面
    let reqs = vec![scoring_req("REQ-0901", 10.0, "SEC-9999", "其他章节的评分项 10 分")];
    let p = project();
    let scorer = SectionScorer::new(derive_scoring_tree(&reqs), p.scorer_terms());
    let mut gen = Scripted::new(vec![filler_draft(), excellent_draft(), excellent_draft()]);
    let mut it = IterativeDrafter::new(&mut gen, &scorer, &p, 3, 0.85);
    let out = it.draft(&sample_input()).unwrap();

    assert_eq!(out.rounds.len(), 1, "无优化面首轮即停（不空转）");
    assert_eq!(gen.calls, 1);
    assert!(!out.reached_target);
    assert_eq!(out.rounds[0].max, 0.0, "max=0 如实登记");
    assert!(out.notes.iter().any(|n| n.contains("无映射评分项")), "{:?}", out.notes);
}

#[test]
fn regression_keeps_best_round() {
    // 第二轮改写回归（0.825 → 0.275）——保留最优轮并登记
    let scorer = scorer_for_sec1();
    let p = project();
    let mut gen = Scripted::new(vec![good_no_specific_draft(), filler_draft()]);
    let mut it = IterativeDrafter::new(&mut gen, &scorer, &p, 2, 0.95);
    let out = it.draft(&sample_input()).unwrap();

    assert_eq!(out.rounds.len(), 2);
    assert!(!out.reached_target);
    assert_eq!(out.content, good_no_specific_draft(), "回归时保留最优轮");
    assert!(out.notes.iter().any(|n| n.contains("保留最优轮")), "{:?}", out.notes);
}

// ---------------------------------------------------------------- 反馈面（自适应）

#[test]
fn adaptive_mock_improves_score_over_15pct() {
    // issue 验收：同一章节，单轮 vs 迭代，得分提升 > 15%
    let scorer = scorer_for_sec1();
    let p = project();

    // 单轮（无循环——只有首轮）
    let mut once = Adaptive { calls: 0, last_instructions: None };
    {
        let mut it = IterativeDrafter::new(&mut once, &scorer, &p, 1, 0.85);
        let single = it.draft(&sample_input()).unwrap();
        assert_eq!(single.rounds.len(), 1, "单轮对照面");
        assert!((single.rounds[0].ratio - 0.275).abs() < 1e-6, "单轮差档（实测 {:.3}）", single.rounds[0].ratio);
    }

    // 三轮迭代（自适应读提示补强）
    let mut multi = Adaptive { calls: 0, last_instructions: None };
    let mut it = IterativeDrafter::new(&mut multi, &scorer, &p, 3, 0.85);
    let out = it.draft(&sample_input()).unwrap();

    let first = out.rounds[0].score;
    let last = out.rounds.iter().map(|r| r.ratio).fold(f64::MIN, f64::max) * out.rounds[0].max;
    let improvement = (last - first) / first * 100.0;
    assert!(
        improvement > 15.0,
        "迭代提升 {:.1}% > 15%（首轮 {first:.2} → 最优 {last:.2}）",
        improvement
    );
    assert!(out.reached_target, "自适应补强后达标");
    assert!(out.rounds.len() < 3, "达标提前停：{} 轮", out.rounds.len());
    assert_eq!(multi.calls, out.rounds.len(), "LLM 调用次数 = 轮次");
}

#[test]
fn rewrite_prompt_carries_deficit_signals() {
    // 首轮差稿 → 改写提示必须携带四类失分项信号（自适应 mock 能补强的前提）
    let scorer = scorer_for_sec1();
    let p = project();
    let mut gen = Adaptive { calls: 0, last_instructions: None };
    let mut it = IterativeDrafter::new(&mut gen, &scorer, &p, 2, 0.85);
    let out = it.draft(&sample_input()).unwrap();
    assert_eq!(out.rounds.len(), 2, "首轮差 → 进改写轮");

    let prompt = gen.last_instructions.expect("第二轮收到改写提示");
    // 1. 未覆盖评分项 + 标准原文（生成器要知道写什么）
    assert!(prompt.contains("REQ-0101"), "低覆盖项 id：{prompt}");
    assert!(prompt.contains("技术方案完整性与针对性"), "评分标准原文：{prompt}");
    assert!(prompt.contains("REQ-0102") && prompt.contains("关键设备配置与性能参数"));
    // 2. 空话密度
    assert!(prompt.contains("空话密度") && prompt.contains("空话句占比"));
    // 3. 针对性缺口（项目名点名）
    assert!(prompt.contains("针对性") && prompt.contains("塔里木水库除险加固"));
    // 4. 引用白名单（防幻觉——只准用已知编号）
    assert!(prompt.contains("禁止编造") && prompt.contains("[EV-0001]"));
    // 提示确定性（同前轮状态 → 同提示）
    let prev = out.rounds[0].clone();
    let again = build_rewrite_prompt(&scorer, &sample_input(), &prev, 0.85, 2);
    let first_prompt = prompt;
    assert_eq!(again, first_prompt, "提示确定性——可 diff 可重放");
}

// ---------------------------------------------------------------- 服务面 e2e

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-b4-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn service_iterative_draft_commits_best_content() {
    let ws = tmp_dir("svc");
    let mut svc = BidService::open(&ws).unwrap();
    svc.create_project("塔里木水库除险加固", "SL-2026-018", "明标", "b4-svc-key").unwrap();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-a", 10, 1, 60, 0, ParseStatus::Ok, "",
    ).unwrap();
    // 评分项 REQ-0101/0102 映射 SEC-0001（树面）+ SEC-0001 绑 REQ-0101（响应面）
    svc.set_requirements(reqs_for_sec1()).unwrap();
    svc.store_mut().set_outline(vec![OutlineNode {
        id: "SEC-0001".into(),
        parent_id: None,
        title: "技术方案".into(),
        level: 1,
        order: 1,
        requirement_ids: vec!["REQ-0101".into()],
        target_chars: 600,
        status: bid_domain::outline::SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }]).unwrap();
    svc.store_mut().build_matrix().unwrap();
    // 证据：登记 + 绑定 REQ-0101（自适应 mock 引用白名单内的真实 id）
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "水利水电施工总承包贰级资质".into(),
        source_file: "EV-01.pdf".into(),
        source_sha256: "sha-b4".into(),
        locator: EngineLocator::DocxParagraph {
            paragraph_index: 3,
            text_fingerprint: "fnv64:9e37".into(),
        },
        fingerprint: "fnv64:9e37".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某建设工程有限公司".into(),
        scope: "水利水电工程施工".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
    }).unwrap();
    svc.store_mut().bind_evidence(&eid, "REQ-0101", "2027-12-31").unwrap();

    let mut gen = Adaptive { calls: 0, last_instructions: None };
    let out = svc.draft_section_iterative(
        "SEC-0001", None, None, None, &mut gen, 3, 0.85,
    ).unwrap();

    // 循环面
    assert!(out.iterative.reached_target, "自适应补强后达标");
    assert!(out.iterative.rounds.len() >= 2, "首轮差 → 至少一轮改写");
    assert_eq!(gen.calls, out.iterative.rounds.len(), "LLM 调用 = 轮次");
    // 提交面（经 draft_section 既有路径——与单次起草同保证）
    let node = svc.store().sections().iter().find(|n| n.id == "SEC-0001").unwrap();
    assert_eq!(node.content.as_deref(), Some(out.iterative.content.as_str()),
        "最优稿落 OutlineNode.content（内存面）");
    assert_eq!(node.status.as_str(), "DRAFTED", "状态推进");
    assert_eq!(out.draft.section_id, "SEC-0001");
    assert!(out.draft.chars > 0, "字数入契约面");
    // 引用回查后 citations = 有效集（幻觉面在 draft_section 内剔除+告警——
    // adaptive 只用白名单 id，warnings 无「幻觉」条目即为白名单纪律的证）
    assert!(out.draft.citations.contains(&eid), "真实 EV id 被引用：{:?}", out.draft.citations);
    assert!(out.draft.warnings.iter().all(|w| !w.contains("幻觉")),
        "白名单纪律——无幻觉剔除告警：{:?}", out.draft.warnings);
    // S-2 落盘
    let chapter = ws.join("chapters").join("SEC-0001.json");
    assert!(chapter.exists(), "工作区章节文件在位：{chapter:?}");
    let _ = std::fs::remove_dir_all(&ws);
}
