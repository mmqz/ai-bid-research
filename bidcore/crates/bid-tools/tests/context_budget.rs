//! context 集成测试 —— S-7 单章输入组装的验收面（issue #11 批次 v3 / V3-5）。
//!
//! 黑盒口径：经 `bid_tools::context` 的 pub API 组装，对 **DomainStore 真实
//! 状态**（项目/条款/章节树/证据台账/绑定）端到端验收：
//! 1. 单章输入 ≤ 8k 实测（issue 验收原文：1 章 + 50 条证据 + 3 条约束）；
//! 2. 约束永不截断（逐字节完整出现在 prompt；约束区自身超红线 → 硬错误）；
//! 3. 组装确定性（同 store 两次组装 prompt 逐字节相等）；
//! 4. fail-closed（borrow：mdcore `workbench/tests/persistence_recovery.rs`
//!    的响亮拒绝模式——空章节 id / 幽灵章节 / 快照损坏的孤儿绑定全拒）；
//! 5. 默认供给面冒烟 + kb 供给面（S-3 检索分相关度 + chunk 文本）；
//! 6. matrix 已构建 / 未构建两口径的候选等价（response_section 与绑定反查同源）。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{DomainStore, ParseStatus, Requirement};
use bid_tools::context::{
    assemble_section_input, assemble_section_input_with, ChunkTextSource, ConstraintSource,
    ContextAssembleError, EvidenceTextSource, KbRelevance,
};
use bid_domain::budget::{estimate_tokens, ConstraintBlock, ContextBudget};
use bid_domain::evidence::{Evidence, EvidenceBinding};
use bid_kb::{Chunk, ExactQuery, Kb};
use std::collections::HashMap;

// ---------------------------------------------------------------- fixture
/// 基座 store：项目 + 文件 + 1 条 REQ + 单章树（SEC-0001 ← REQ-0003）。
fn base_store() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("智慧园区安防系统采购项目", "ZFCG-2026-GK-0311", "明标", "ext-key-s7").unwrap();
    s.upsert_file("A-招标文件.docx", "招标文件", "sha-a", 31, 12, 120, 0,
                  ParseStatus::Ok, "").unwrap();
    let reqs = vec![Requirement {
        id: "REQ-0003".into(),
        category: "资格条件".into(),
        clause_no: Some("3.1".into()),
        anchors: vec![],
        params: HashMap::new(),
        source_file: "A".into(),
    }];
    s.set_requirements(reqs).unwrap();
    s.set_outline(vec![OutlineNode {
        id: "SEC-0001".into(),
        parent_id: None,
        title: "资格响应".into(),
        level: 1,
        order: 1,
        requirement_ids: vec!["REQ-0003".into()],
        target_chars: 2000,
        status: SectionStatus::Pending,
        dirty: false,
        content: None, content_attempt: 0,
    }]).unwrap();
    s
}

/// 登记 + 绑定一条证据（返回 store 签发的 EV id）。
fn register_bound(s: &mut DomainStore, i: usize, source_file: &str,
                  locator: EngineLocator) -> String {
    let (id, _) = s.register_evidence(EvidenceSpec {
        kind: EvidenceKind::Performance,
        title: format!("业绩合同-{i:02}：智能化工程项目"),
        source_file: source_file.into(),
        source_sha256: format!("sha-ev-{i:02}"),
        locator,
        fingerprint: format!("fnv64:{i:02}"),
        valid_from: None,
        valid_until: None,
        subject_name: "测试投标人".into(),
        scope: format!("适用范围：建筑智能化-{i}").into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    s.bind_evidence(&id, "REQ-0003", "2026-03-11").unwrap();
    id
}

/// 注入式约束源（金额/工期/术语表——store 尚无权威集合，嵌入方注入面）。
struct FixedConstraints {
    blocks: Vec<(&'static str, String)>,
}

impl ConstraintSource for FixedConstraints {
    fn constraint_blocks(&self, _store: &DomainStore) -> Vec<ConstraintBlock> {
        self.blocks.iter().map(|(k, t)| ConstraintBlock::new(*k, t.clone())).collect()
    }
}

/// 注入式文本源：full = 恰好 full_tokens、summary = 恰好 summary_tokens
/// （单字重复——estimate_tokens 口径的精确构造面）。
struct InjectedText {
    full_tokens: usize,
    summary_tokens: usize,
}

impl EvidenceTextSource for InjectedText {
    fn full_text(&self, _e: &Evidence) -> String {
        "材".repeat(self.full_tokens)
    }
    fn summary_paragraphs(&self, _e: &Evidence) -> Vec<String> {
        vec!["摘".repeat(self.summary_tokens)]
    }
}

/// S-7 三约束（金额/工期/术语表——issue 验收场景的素材）。
fn s7_constraints() -> FixedConstraints {
    FixedConstraints {
        blocks: vec![
            ("金额", "招标控制价：人民币叁佰伍拾万元整，投标报价超过控制价即作废标处理".into()),
            ("工期", "总工期玖拾日历天，自监理开工令载明的开工日期起算至竣工验收合格".into()),
            ("术语", "术语对照：SCADA 指数据采集与监控系统；UPS 指不间断电源；BIM 指建筑信息模型".into()),
        ],
    }
}

// ---------------------------------------------------------------- 验收主测
/// issue #11 S-7 验收原文：1 章 + 50 条证据（每条 2000 token）+ 3 条约束 →
/// assemble 后 `estimate_tokens(prompt_text)` **实测** ≤ 8000；
/// 50 条必裁 → truncated_evidence == true 且输出里有 truncated_evidence 标记。
#[test]
fn single_section_input_within_8k_hard_acceptance() {
    let mut s = base_store();
    let mut first_id = String::new();
    for i in 0..50 {
        let id = register_bound(&mut s, i, "evidence/业绩合同.pdf",
                                EngineLocator::PdfPageText { page: 1, text_fingerprint: format!("fnv64:{i:02}") });
        if i == 0 {
            first_id = id;
        }
    }
    let input = assemble_section_input_with(
        &s, "SEC-0001", &ContextBudget::default(),
        &s7_constraints(),
        &InjectedText { full_tokens: 2_000, summary_tokens: 100 },
        &bid_tools::context::BindingOrderRelevance,
    ).unwrap();
    // —— 8k 硬验收（实测口径：estimate_tokens 是全仓单一真源尺子）——
    let kept = input.plan.evidence_plan.iter()
        .filter(|e| e.mode != bid_domain::budget::EvidenceMode::Dropped).count();
    eprintln!("[S-7 验收] 单章输入实测 {} tokens（红线 8000 / 规划目标 7200 / 保留 {kept} of 50）",
              input.total_tokens);
    assert!(input.total_tokens <= 8_000,
            "单章输入实测 {} tokens 必须 ≤ 8000", input.total_tokens);
    assert_eq!(input.total_tokens, estimate_tokens(&input.prompt_text),
               "SectionInput.total_tokens 与实测算尺逐字节同口径");
    // —— 证据必裁（50 × 2000 tokens 物理上装不进 8k）——
    assert!(input.truncated_evidence, "50 条必裁——truncated_evidence 必须为 true");
    assert!(input.prompt_text.contains("truncated_evidence"),
            "验收要求：证据被裁剪时输出里标 truncated_evidence");
    let kept2 = input.plan.evidence_plan.iter()
        .filter(|e| e.mode != bid_domain::budget::EvidenceMode::Dropped).count();
    assert_eq!(kept, kept2, "保留数稳定（重算一致性）");
    assert!(kept < 50, "50 条 2000-token 证据不可能全保留（实际保留 {kept}）");
    assert!(kept >= 1, "至少保留最高相关证据（预算反推的 top-N ≥ 1）");
    // 最高相关（绑定序首位）必须是 Full（四阶梯 b：高相关优先装全文）
    assert_eq!(input.plan.evidence_plan[0].evidence_id, first_id);
    assert_eq!(input.plan.evidence_plan[0].mode, bid_domain::budget::EvidenceMode::Full);
    // 溢出计划标记（约束+裁剪清单挤压安全边际——裁决位，非失败）
    assert!(input.plan.overflow, "保守计量 7200+ 必然突破规划目标（dropped 清单计入）");
}

/// 约束永不截断：约束区 2000 token + 大量证据 → 约束文本**逐字节**完整
/// 出现在 prompt；约束区自身 9000 token → 硬错误（Err），错误信息说明
/// 「约束区超预算，绝不静默截断」。
#[test]
fn constraints_never_truncated_and_hard_error_on_overflow() {
    let mut s = base_store();
    for i in 0..30 {
        register_bound(&mut s, i, "evidence/业绩合同.pdf",
                       EngineLocator::PdfPageText { page: 1, text_fingerprint: format!("fnv64:{i:02}") });
    }
    // —— 2000-token 约束区 + 30 条证据：约束逐字节完整 ——
    let long_constraint = "投".repeat(2_000);
    let cons = FixedConstraints { blocks: vec![
        ("金额", long_constraint.clone()),
        ("工期", "总工期玖拾日历天".into()),
    ]};
    let input = assemble_section_input_with(
        &s, "SEC-0001", &ContextBudget::default(),
        &cons, &InjectedText { full_tokens: 500, summary_tokens: 100 },
        &bid_tools::context::BindingOrderRelevance,
    ).unwrap();
    assert!(input.prompt_text.contains(&long_constraint),
            "约束文本必须逐字节完整出现在 prompt（永不裁剪）");
    assert!(input.total_tokens <= 8_000);

    // —— 约束区 9000 token：自身超红线 → 硬错误 ——
    let huge = FixedConstraints { blocks: vec![("金额", "约".repeat(9_000))] };
    let err = match assemble_section_input_with(
        &s, "SEC-0001", &ContextBudget::default(),
        &huge, &InjectedText { full_tokens: 100, summary_tokens: 10 },
        &bid_tools::context::BindingOrderRelevance,
    ) {
        Ok(_) => panic!("约束区超红线必须硬错误——绝不悄悄放行超限"),
        Err(e) => e,
    };
    assert!(matches!(err, ContextAssembleError::Budget(
        bid_domain::budget::BudgetError::ConstraintOverflow { .. })));
    let msg = format!("{err}");
    assert!(msg.contains("绝不静默截断"), "错误信息必须说明绝不静默截断: {msg}");
    assert!(msg.contains("约束区"), "错误信息指名约束区: {msg}");
}

/// 组装面四阶梯端到端：注入可控 token 的文本源，验证 Full/Summary/Dropped
/// 三态在 prompt 里的落点形态（块头标记 + 裁剪清单行）。
#[test]
fn four_tier_lands_in_prompt_shape() {
    let mut s = base_store();
    let ids: Vec<String> = (0..3).map(|i| {
        register_bound(&mut s, i, "evidence/业绩合同.pdf",
                       EngineLocator::PdfPageText { page: 1, text_fingerprint: format!("fnv64:{i:02}") })
    }).collect();
    // 小预算逼出三态：total 600 / 目标 540；约束 30 + 规格 ~50 + 标记 176
    let input = assemble_section_input_with(
        &s, "SEC-0001",
        &bid_domain::budget::ContextBudget { total: 600, constraint_reserve_headroom_pct: 10 },
        &FixedConstraints { blocks: vec![("金额", "控制价三百五十万元整".into())] },
        &InjectedText { full_tokens: 120, summary_tokens: 40 },
        &bid_tools::context::BindingOrderRelevance,
    ).unwrap();
    let modes: Vec<_> = input.plan.evidence_plan.iter().map(|e| e.mode).collect();
    assert!(modes.contains(&bid_domain::budget::EvidenceMode::Full), "必有 Full：{modes:?}");
    assert!(modes.contains(&bid_domain::budget::EvidenceMode::Dropped), "必有 Dropped：{modes:?}");
    assert!(input.total_tokens <= 600, "小预算红线同样实测硬守: {}", input.total_tokens);
    // Full 证据的全文逐字节在 prompt；Dropped 的全文绝不在 prompt
    for (e, id) in input.plan.evidence_plan.iter().zip(ids.iter()) {
        match e.mode {
            bid_domain::budget::EvidenceMode::Full => {
                assert!(input.prompt_text.contains(&"材".repeat(120)),
                        "Full 证据全文必须逐字节入 prompt");
            }
            bid_domain::budget::EvidenceMode::Dropped => {
                assert!(!input.prompt_text.contains(&format!("[证据 {id}｜")),
                        "Dropped 证据不占证据块（只进裁剪清单行）");
            }
            _ => {}
        }
    }
    // 裁剪清单行：dropped id 以、连接出现在 truncated_evidence 标记行
    let dropped_ids: Vec<&str> = input.plan.evidence_plan.iter()
        .filter(|e| e.mode == bid_domain::budget::EvidenceMode::Dropped)
        .map(|e| e.evidence_id.as_str()).collect();
    if !dropped_ids.is_empty() {
        assert!(input.prompt_text.contains(&format!(
            "【裁剪清单 truncated_evidence】{}", dropped_ids.join("、"))));
    }
}

/// 组装确定性：同 store 两次组装 → prompt_text 逐字节相等 + plan 全等。
#[test]
fn assemble_is_byte_deterministic() {
    let mut s = base_store();
    for i in 0..50 {
        register_bound(&mut s, i, "evidence/业绩合同.pdf",
                       EngineLocator::PdfPageText { page: 1, text_fingerprint: format!("fnv64:{i:02}") });
    }
    let run = || assemble_section_input_with(
        &s, "SEC-0001", &ContextBudget::default(),
        &s7_constraints(),
        &InjectedText { full_tokens: 2_000, summary_tokens: 100 },
        &bid_tools::context::BindingOrderRelevance,
    ).unwrap();
    let a = run();
    let b = run();
    assert_eq!(a.prompt_text, b.prompt_text, "同 store 两次组装 prompt 必须逐字节相等");
    assert_eq!(a.plan, b.plan, "计划全等（可复现是调度输入面的前提）");
    assert_eq!(a.total_tokens, b.total_tokens);
}

/// fail-closed（borrow：mdcore `workbench/tests/persistence_recovery.rs` 的
/// 响亮拒绝模式——损坏数据不猜测加载、非法输入不 panic 不静默）：
/// 空/空白章节 id、幽灵章节、快照损坏的孤儿绑定 → 全部 Err。
#[test]
fn fail_closed_bad_section_and_corrupt_snapshot() {
    let mut s = base_store();
    register_bound(&mut s, 0, "evidence/业绩合同.pdf",
                   EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:00".into() });
    let budget = ContextBudget::default();
    // 空 / 纯空白章节 id
    for bad in ["", "   "] {
        assert_eq!(assemble_section_input(&s, bad, &budget),
                   Err(ContextAssembleError::EmptySectionId));
    }
    // 幽灵章节
    assert_eq!(assemble_section_input(&s, "SEC-9999", &budget),
               Err(ContextAssembleError::SectionNotFound { section_id: "SEC-9999".into() }));

    // 快照损坏：注入孤儿绑定（restore 的引用完整性校验未覆盖绑定面——
    // 语法合法但语义残缺，消费侧把关：半个证据链不猜测，响亮拒绝组装）
    let mut bad = s.to_snapshot();
    bad.evidence_bindings.push(EvidenceBinding {
        evidence_id: "EV-9999".into(),
        requirement_id: "REQ-0003".into(),
    });
    let s2 = DomainStore::restore(bad)
        .expect("restore 放行（绑定引用面不在其校验集——消费侧把关的缝）");
    let err = match assemble_section_input(&s2, "SEC-0001", &budget) {
        Ok(_) => panic!("孤儿绑定必须拒绝组装（fail-closed 不猜测）"),
        Err(e) => e,
    };
    assert_eq!(err, ContextAssembleError::OrphanBinding { evidence_id: "EV-9999".into() });
    assert!(format!("{err}").contains("EV-9999"), "错误面可辨识: {err}");
}

/// 默认供给面冒烟：项目标识约束（BidProject 面）+ 台账文本 + 绑定序相关度
/// ——无注入时的诚实基线（金额/工期/术语表的 M3 接线位见 trait 注释）。
#[test]
fn default_sources_smoke() {
    let mut s = base_store();
    register_bound(&mut s, 0, "evidence/业绩合同.pdf",
                   EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:00".into() });
    let input = assemble_section_input(&s, "SEC-0001", &ContextBudget::default()).unwrap();
    // 项目标识约束（store 现有面的全局信息）
    assert!(input.prompt_text.contains("智慧园区安防系统采购项目"));
    assert!(input.prompt_text.contains("ZFCG-2026-GK-0311"));
    // 章节规格（OutlineNode 渲染面）
    assert!(input.prompt_text.contains("SEC-0001 资格响应"));
    assert!(input.prompt_text.contains("REQ-0003"));
    // 台账文本源：证据标题 + 适用范围 + 溯源锚（locator_brief）
    assert!(input.prompt_text.contains("业绩合同-00：智能化工程项目"));
    assert!(input.prompt_text.contains("pdf 第 1 页"));
    // 单条小证据 → Full 入选、无裁剪
    assert_eq!(input.plan.evidence_plan.len(), 1);
    assert_eq!(input.plan.evidence_plan[0].mode, bid_domain::budget::EvidenceMode::Full);
    assert!(!input.truncated_evidence);
    assert!(input.total_tokens <= 8_000);
}

/// kb 供给面：S-3 检索分相关度（命中文件的相关度高、未命中 0 分）+
/// ChunkTextSource（DocxParagraph 锚 ↔ chunk 块序——chunk 全文入 prompt）。
#[test]
fn kb_relevance_and_chunk_text_sources() {
    let mut s = base_store();
    // 证据 A（锚 docx 段 3——命中 chunk）、证据 B（锚 docx 段 3——不同文件）
    let id_a = register_bound(&mut s, 0, "业绩合同A.pdf",
        EngineLocator::DocxParagraph { paragraph_index: 3, text_fingerprint: "fnv64:a".into() });
    let _id_b = register_bound(&mut s, 1, "业绩合同B.pdf",
        EngineLocator::DocxParagraph { paragraph_index: 3, text_fingerprint: "fnv64:b".into() });
    let chunk_text = "智慧园区一期安防工程业绩正文：合同金额壹佰贰拾万元，验收合格。".repeat(6);
    let kb = Kb { chunks: vec![
        Chunk { chunk_id: "c-a".into(), file: "业绩合同A.pdf".into(), file_sha: "sha-a".into(),
                kind_rank: 4, block_idx: 3, locator: "para-3".into(), text: chunk_text.clone() },
        Chunk { chunk_id: "c-b".into(), file: "业绩合同B.pdf".into(), file_sha: "sha-b".into(),
                kind_rank: 4, block_idx: 3, locator: "para-3".into(),
                text: "其他类型项目建设履历简述。".repeat(3) },
    ]};
    let input = assemble_section_input_with(
        &s, "SEC-0001", &ContextBudget::default(),
        &s7_constraints(),
        &ChunkTextSource::new(&kb),
        &KbRelevance { kb: &kb, query: ExactQuery {
            query_id: "s7-sec-0001".into(), query_type: "model".into(),
            query: "智慧园区一期安防工程".into() } },
    ).unwrap();
    // kb 检索分口径：A 的文件命中查询（score 1.0）→ 排首位；B 未命中（0 分）
    assert_eq!(input.plan.evidence_plan[0].evidence_id, id_a,
               "kb 检索分高的证据排首位");
    assert!(input.plan.evidence_plan[0].relevance > input.plan.evidence_plan[1].relevance);
    // ChunkTextSource：锚定 chunk 的全文逐字节入 prompt
    assert!(input.prompt_text.contains(&chunk_text),
            "DocxParagraph 锚 ↔ chunk 块序定位的全文必须入 prompt");
    assert!(input.total_tokens <= 8_000);
}

/// matrix 已构建 / 未构建两口径等价：response_section 关联与绑定反查同源
/// （derive_matrix_rows 的 response_section 即本反查的派生态）。
#[test]
fn matrix_built_and_unbuilt_paths_are_equivalent() {
    let mut s = base_store();
    for i in 0..5 {
        register_bound(&mut s, i, "evidence/业绩合同.pdf",
                       EngineLocator::PdfPageText { page: 1, text_fingerprint: format!("fnv64:{i:02}") });
    }
    let ids_of = |p: &bid_domain::budget::ContextPlan| -> Vec<String> {
        p.evidence_plan.iter().map(|e| e.evidence_id.clone()).collect()
    };
    // 未构建：bindings × 章节条款反查
    let r1 = assemble_section_input_with(
        &s, "SEC-0001", &ContextBudget::default(),
        &s7_constraints(),
        &InjectedText { full_tokens: 300, summary_tokens: 80 },
        &bid_tools::context::BindingOrderRelevance,
    ).unwrap();
    // 构建后：matrix 行的 response_section 关联（任务原文口径）
    s.build_matrix().unwrap();
    let r2 = assemble_section_input_with(
        &s, "SEC-0001", &ContextBudget::default(),
        &s7_constraints(),
        &InjectedText { full_tokens: 300, summary_tokens: 80 },
        &bid_tools::context::BindingOrderRelevance,
    ).unwrap();
    assert_eq!(ids_of(&r1.plan), ids_of(&r2.plan), "两口径的候选集与序必须等价");
    assert_eq!(r1.prompt_text, r2.prompt_text, "prompt 逐字节等价（同源反查的证明）");
}
