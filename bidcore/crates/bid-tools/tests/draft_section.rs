//! #59 阶段 1 · draft_response_section 服务层测试（bid-tools draft.rs）。
//!
//! 验收面（Lead #59 指令 + 契约 §6）：
//! - mock 生成字数达标（target ±20%——多档位）；
//! - 引用回查（伪造 EV/REQ → 剔除 + warning，防幻觉）；
//! - EXPIRED / SUPERSEDED 证据拒绑（R-03——E_UNPROCESSABLE）；
//! - 结构节点拒正文；幽灵 REQ 剔除告警；空响应面拒；
//! - CAS：同内容 no-op（attempt 不变）、不同内容 attempt+1；
//! - 章节推进 DRAFTED、项目推进 DRAFTING；
//! - S-2 落盘：chapters/{sec}.json 在位 + 重启恢复；
//! - unanchored_claims 计数（无锚承诺句）。
//!
//! 双轨纪律（issue #11）：自研轨全部；mock 确定性（同输入同输出）单测。

use std::collections::HashSet;

use bid_domain::evidence::EvidenceSpec;
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineSpec;
use bid_domain::{Anchor, DomainStore, ParseStatus, ProjectStatus, Requirement, ToolError};
use bid_tools::draft::{
    build_section_input, count_unanchored_claims, generate_mock, scan_citations,
    strip_unknown_citations, verify_citations, MockGenerator, SectionDraftInput, SectionGenerator,
};
use bid_tools::service::BidService;

// ---------------------------------------------------------------- 夹具

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-draft-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn req(id: &str, category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第三章/1.2".into(),
            excerpt: excerpt.into(),
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

/// 登记一条证据（register_evidence——出生态 Active；过期经 valid_until 翻转）。
fn register_ev(svc: &mut BidService, title: &str, valid_until: Option<&str>) -> String {
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: title.into(),
        source_file: "EV-01.pdf".into(),
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
    }).unwrap();
    eid
}

/// 造一个「抽取完成 → 建纲 → 建矩阵（MATRIX_READY）→ 素材齐备」的服务态。
/// 3 条 REQ（资格/否决/评分各一）→ 章节树 + 矩阵。
///
/// #110 任务 1 后的起草硬前置（readiness 前置闸）：
/// - 「资格条件」叶标题含资格 → 台账必须有可用资质证据；
/// - 否决项 REQ 必须在矩阵 RESPONDED（绑证据 + 重建矩阵）——
///   否则 draft_section 直接 E_UNPROCESSABLE（缺素材拒生成，不是回归）。
fn svc_matrix_ready(tag: &str) -> BidService {
    let mut svc = BidService::open(tmp_dir(tag)).unwrap();
    svc.create_project("起草测试", "C-DFT", "明标", tag).unwrap();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-a", 10, 1, 60, 0, ParseStatus::Ok, "",
    ).unwrap();
    // 招标包完整性（IncompleteTender 只警告不阻塞——补遗件在场别让警告面进断言）
    svc.store_mut().upsert_file(
        "C-补遗文件01.docx", "补遗", "sha-add", 5, 0, 20, 0, ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        req("REQ-0001", "资格条件", "投标人须具备水利水电工程施工总承包贰级及以上资质"),
        req("REQ-0002", "否决项", "投标文件未经法定代表人或其授权代表签字的，其投标作否决处理"),
        req("REQ-0003", "评分项", "施工组织设计方案完整性分值 10 分"),
    ]).unwrap();
    let (n_sec, n_rows, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 3_000, max_reqs_per_leaf: 2 })
        .unwrap();
    assert!(n_sec >= 2, "3 REQ / 每叶 2 条 → 至少 2 节点（父 + 叶），实际 {n_sec}");
    assert!(n_rows >= 3);
    // 素材面：资质证据 ×1（资格类章节的标题词面）+ 资格/否决 REQ 全绑（矩阵 RESPONDED）
    let eid = register_ev(&mut svc, "水利水电工程施工总承包贰级资质证书", None);
    for rid in ["REQ-0001", "REQ-0002"] {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::MatrixReady);
    svc
}

fn first_leaf(svc: &BidService) -> String {
    let parents: HashSet<&str> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.as_deref()).collect();
    svc.store().sections().iter()
        .find(|n| !parents.contains(&n.id[..]))
        .map(|n| n.id.clone()).unwrap()
}

fn first_structural(svc: &BidService) -> String {
    svc.store().sections().iter()
        .find(|n| n.parent_id.is_none())
        .map(|n| n.id.clone()).unwrap()
}

// ---------------------------------------------------------------- 1. mock 生成与字数

#[test]
fn mock_generation_is_deterministic() {
    let input = sample_input(1_000);
    assert_eq!(generate_mock(&input), generate_mock(&input), "同输入必须字节级同输出（跨进程可重放）");
}

#[test]
fn mock_word_count_hits_target_band() {
    // Lead 验收线：字数接近 target_chars（±20%）——多档位验证填充器
    for target in [500u32, 1_000, 2_000, 5_000] {
        let input = sample_input(target);
        let content = generate_mock(&input);
        let chars = content.chars().count() as f64;
        let t = f64::from(target);
        assert!(
            chars >= t * 0.80 && chars <= t * 1.20,
            "target={target} 实际 {chars} 字（须在 [{:.0}, {:.0}]）",
            t * 0.80, t * 1.20
        );
    }
}

#[test]
fn mock_covers_all_requirement_ids() {
    // Lead 验收线：生成内容覆盖所有 requirement_ids
    let input = sample_input(1_000);
    let content = generate_mock(&input);
    for r in &input.requirements {
        assert!(content.contains(&r.id), "正文未覆盖 {}", r.id);
    }
    for e in &input.evidences {
        assert!(content.contains(&e.id), "正文未引用证据 {}", e.id);
    }
}

/// #217（#185 缺口 2·验收 F）：mock 输出携带正文摘录与定位——
/// 「依据 [EV] …」之外追加原文摘录段，评标人可按定位回查原件。
#[test]
fn mock_output_carries_evidence_excerpt_and_locator() {
    let input = sample_input(1_000);
    let content = generate_mock(&input);
    // 摘录段模板要素齐备：定位 + 摘录正文 + 对账声明
    assert!(
        content.contains("其原始凭据 《EV-01-水利水电资质证书.pdf》第 1 页 原文摘录如下"),
        "摘录段含人话定位: {content}"
    );
    assert!(
        content.contains("资质等级：贰级；承包范围：水利水电工程施工总承包"),
        "摘录正文逐字呈现: {content}"
    );
    assert!(
        content.contains("不得超出摘录范围编造"),
        "摘录纪律声明在场: {content}"
    );
}

/// #217 负向：文件级锚（无摘录）→ 不出现摘录段（不编造「读过」的假象）。
#[test]
fn mock_output_omits_excerpt_when_unlocated() {
    let mut input = sample_input(1_000);
    input.evidences[0].excerpt = String::new();
    input.evidences[0].excerpt_locator = String::new();
    let content = generate_mock(&input);
    assert!(
        !content.contains("其原始凭据"),
        "无摘录不得渲染摘录段: {content}"
    );
    assert!(
        content.contains("依据 [EV-0001]"),
        "台账面佐证段仍在（不因无摘录丢佐证）"
    );
}

fn sample_input(target: u32) -> SectionDraftInput {
    SectionDraftInput {
        section_id: "SEC-0002".into(),
        title: "资格响应".into(),
        target_chars: target,
        requirements: vec![
            bid_tools::draft::ReqBrief {
                id: "REQ-0001".into(),
                category: "资格条件".into(),
                excerpt: "投标人须具备水利水电工程施工总承包贰级及以上资质".into(),
            },
            bid_tools::draft::ReqBrief {
                id: "REQ-0002".into(),
                category: "否决项".into(),
                excerpt: "投标文件未经签字的作否决处理".into(),
            },
        ],
        evidences: vec![bid_tools::draft::EvBrief {
            id: "EV-0001".into(),
            kind: "资质".into(),
            title: "水利水电工程施工总承包贰级资质证书".into(),
            scope: "全国".into(),
            excerpt: "资质等级：贰级；承包范围：水利水电工程施工总承包，可承担各类水利水电工程".into(),
            excerpt_locator: "《EV-01-水利水电资质证书.pdf》第 1 页".into(),
            placeholder: false,
        }],
        instructions: Some("按资格条件逐项响应".into()),
    }
}

// ---------------------------------------------------------------- 2. 引用回查（防幻觉）

#[test]
fn scan_citations_finds_both_forms() {
    let text = "针对 [REQ-0001] 的要求，依据 EV-0002 与 [EV-0003] 作答；REQ-0001 重复出现只记一次。";
    assert_eq!(
        scan_citations(text),
        vec!["REQ-0001".to_string(), "EV-0002".to_string(), "EV-0003".to_string()]
    );
}

/// #217 判别力：裸形态必须是独立 token——来源文件名（`EV-03-xx.pdf`）/
/// 更长 token（`REQ-0001x`）里的 id 前缀不算引用；CJK 紧邻仍算。
#[test]
fn scan_citations_ignores_filename_prefixes_but_keeps_cjk_adjacent() {
    // 文件名前缀：`EV-03-类似业绩合同.pdf` 里的 `EV-03` 不是引用
    let text = "其原始凭据 《EV-03-类似业绩合同-智慧园区弱电工程.pdf》第 1 页 原文摘录如下。";
    assert!(
        scan_citations(text).is_empty(),
        "文件名前缀不算引用: {:?}",
        scan_citations(text)
    );
    // 更长 token 的前缀/后缀
    assert!(scan_citations("版本 REQ-0001x 已发布").is_empty(), "REQ-0001x 不是 REQ-0001");
    assert!(scan_citations("扩展名 EV-0002.pdf 见附件").is_empty(), "EV-0002.pdf 视为文件名");
    // CJK 紧邻（中文无空格分词）仍是真引用
    assert_eq!(
        scan_citations("依据EV-0002作答"),
        vec!["EV-0002".to_string()],
        "CJK 紧邻的裸 id 仍是引用"
    );
    // 括号/标点紧邻仍是真引用
    assert_eq!(
        scan_citations("（EV-0003）与、EV-0004、均有效"),
        vec!["EV-0003".to_string(), "EV-0004".to_string()]
    );
}

/// #217 剥除面：剥 `EV-03` 不得误伤 `[EV-034]`（子串）与 `EV-03-xx.pdf`（文件名）。
#[test]
fn strip_unknown_citations_spares_longer_tokens_and_filenames() {
    let cleaned = strip_unknown_citations(
        "依据 [EV-034] 与《EV-03-类似业绩合同.pdf》第 1 页作答，另见 [EV-9999]。",
        &["EV-03".to_string(), "EV-9999".to_string()],
    );
    assert!(cleaned.contains("[EV-034]"), "已知更长 id 不被误伤: {cleaned}");
    assert!(
        cleaned.contains("《EV-03-类似业绩合同.pdf》"),
        "文件名完整保留: {cleaned}"
    );
    assert!(!cleaned.contains("EV-9999"), "未知引用仍被剥除: {cleaned}");
}

#[test]
fn verify_citations_splits_known_and_unknown() {
    let known: HashSet<String> = ["REQ-0001".to_string(), "EV-0001".to_string()].into_iter().collect();
    let (valid, unknown) = verify_citations("响应 [REQ-0001] 并援引 [EV-9999]（幻觉）", &known);
    assert_eq!(valid, vec!["REQ-0001"]);
    assert_eq!(unknown, vec!["EV-9999"], "编造的证据号必须被识别");
}

#[test]
fn strip_unknown_citations_removes_them() {
    let cleaned = strip_unknown_citations(
        "响应 [REQ-0001]，依据 [EV-9999] 与 [EV-0001] 作答。",
        &["EV-9999".to_string()],
    );
    assert!(!cleaned.contains("EV-9999"));
    assert!(cleaned.contains("REQ-0001"));
    assert!(cleaned.contains("EV-0001"));
}

#[test]
fn unanchored_claims_counted_only_without_citation() {
    // 带锚承诺不计；无锚承诺逐句计
    let text = "我方承诺按时竣工。针对 [REQ-0001]，我方保证质量达标。另保证安全生产。";
    assert_eq!(count_unanchored_claims(text), 2, "「承诺按时竣工」与「保证安全生产」无锚");
}

// ---------------------------------------------------------------- 3. 输入装配

#[test]
fn build_input_rejects_structural_node() {
    let svc = svc_matrix_ready("struct-node");
    let sid = first_structural(&svc);
    let err = build_section_input(svc.store(), &sid, None, None, None).unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE");
}

#[test]
fn build_input_rejects_unknown_section() {
    let svc = svc_matrix_ready("unknown-sec");
    let err = build_section_input(svc.store(), "SEC-9999", None, None, None).unwrap_err();
    assert_eq!(err.code(), "E_NOTFOUND");
}

#[test]
fn build_input_strips_ghost_reqs_with_warning() {
    let mut svc = svc_matrix_ready("ghost-req");
    let sid = first_leaf(&svc);
    let (input, warnings) = build_section_input(
        svc.store(), &sid,
        Some(&["REQ-0001".to_string(), "REQ-9999".to_string()]),
        None, None,
    ).unwrap();
    assert_eq!(input.requirements.len(), 1, "幽灵 REQ 剔除后只剩 REQ-0001");
    assert!(warnings.iter().any(|w| w.contains("REQ-9999")), "幽灵 REQ 必须告警: {warnings:?}");
}

#[test]
fn build_input_rejects_expired_evidence() {
    let mut svc = svc_matrix_ready("expired-ev");
    let sid = first_leaf(&svc);
    // 登记一条已过期证据（valid_until 早于开标日 → apply_validity 翻转 EXPIRED）
    let eid = register_ev(&mut svc, "已过期资质", Some("2020-01-01"));
    svc.store_mut().apply_validity("2026-11-01").unwrap();
    let err = build_section_input(
        svc.store(), &sid, None,
        Some(&[eid]), None,
    ).unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE", "EXPIRED 禁绑（R-03）: {err:?}");
}

#[test]
fn build_input_rejects_empty_response_surface() {
    let svc = svc_matrix_ready("empty-surface");
    let sid = first_leaf(&svc);
    let err = build_section_input(
        svc.store(), &sid, Some(&["REQ-9999".to_string()]), None, None,
    ).unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE", "全幽灵 override = 空响应面必须拒");
}

// ---------------------------------------------------------------- 4. 服务集成（draft_section 全链）

#[test]
fn draft_section_full_chain() {
    let mut svc = svc_matrix_ready("full-chain");
    let sid = first_leaf(&svc);
    let node = svc.store().sections().iter().find(|n| n.id == sid).unwrap();
    let target = node.target_chars;

    let mut gen = MockGenerator;
    let out = svc.draft_section(&sid, None, None, Some("按招标文件逐项响应"), &mut gen).unwrap();

    // 契约面：attempt=1、字数达标、引用非空、零无锚承诺（mock 模板句句带锚）
    assert_eq!(out.attempt, 1);
    let chars = out.chars as f64;
    let t = f64::from(target.max(1));
    assert!(chars >= t * 0.80 && chars <= t * 1.20, "target={target} 实际 {chars}");
    assert!(!out.citations.is_empty());
    assert_eq!(out.unanchored_claims, 0);

    // store 面：content 落位 + 章节推进 DRAFTED + 项目推进 DRAFTING
    let node = svc.store().sections().iter().find(|n| n.id == sid).unwrap();
    assert!(node.content.is_some());
    assert_eq!(node.content_attempt, 1);
    assert_eq!(node.status.as_str(), "DRAFTED");
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Drafting);

    // S-2 落盘面：chapters/{sid}.json 在位且内容一致
    let ws = svc.workspace_dir().unwrap();
    let rec = bid_tools::assembly::ChapterAssembler::open(&ws).unwrap();
    let record = rec.record(&sid).unwrap();
    assert_eq!(record.content, node.content.clone().unwrap());
    assert_eq!(record.char_count, out.chars);
    assert_eq!(record.attempt, 1);
}

#[test]
fn draft_section_idempotent_same_content() {
    let mut svc = svc_matrix_ready("idem");
    let sid = first_leaf(&svc);
    let mut gen = MockGenerator;
    let a = svc.draft_section(&sid, None, None, None, &mut gen).unwrap();
    let b = svc.draft_section(&sid, None, None, None, &mut gen).unwrap();
    assert_eq!(a.attempt, b.attempt, "同 instructions 重提交 → attempt 不推进（no-op）");
    assert_eq!(a.chars, b.chars);
    assert_eq!(a.content, b.content);
}

#[test]
fn draft_section_attempt_increments_on_new_instructions() {
    // #127 P0-2 语义更新：正文不再嵌「起草指令」（句子劈断缺陷）——同
    // REQ/EV 下换指令产**同正文**，CAS 判定内容未变 → attempt 不推进
    // （版本账记内容不记指令）；指令变化体现在 draft_note 账面。
    let mut svc = svc_matrix_ready("attempt-inc");
    let sid = first_leaf(&svc);
    let mut gen = MockGenerator;
    let a = svc.draft_section(&sid, None, None, Some("第一稿"), &mut gen).unwrap();
    let b = svc.draft_section(&sid, None, None, Some("第二稿：补充业绩佐证"), &mut gen).unwrap();
    assert_eq!(b.attempt, a.attempt, "指令不进正文 → 同内容 no-op（attempt 不推进）");
    let node = svc.store().sections().iter().find(|n| n.id == sid).unwrap();
    assert_eq!(node.content_attempt, 1, "内容版本账只记内容变化");
    // 指令落到 draft_note 账面（信息不丢——#127 验收线）
    assert!(
        b.draft_note.as_deref().unwrap_or("").contains("第二稿：补充业绩佐证"),
        "新指令更新 draft_note 账面: {:?}",
        b.draft_note
    );
    assert!(
        b.draft_note.as_deref().unwrap_or("").contains("起草指令"),
        "账面带「起草指令：」标头: {:?}",
        b.draft_note
    );
    // 正文零「起草指令」字样（P0-2 验收线）
    assert!(
        !b.content.contains("起草指令"),
        "正文不得含指令标记（句子劈断缺陷）: {}",
        &b.content.chars().take(120).collect::<String>()
    );
}

#[test]
fn draft_section_strips_hallucinated_citations() {
    let mut svc = svc_matrix_ready("hallucination");
    let sid = first_leaf(&svc);
    // 注入会幻觉的生成器（编造 EV-8888）
    struct Hallucinating;
    impl SectionGenerator for Hallucinating {
        fn generate(&mut self, input: &SectionDraftInput) -> Result<bid_tools::draft::GeneratedDraft, String> {
            let base = generate_mock(input);
            Ok(bid_tools::draft::GeneratedDraft {
                content: format!("{}\n\n另据 EV-8888 的记载补充说明。", base),
            })
        }
    }
    let mut gen = Hallucinating;
    let out = svc.draft_section(&sid, None, None, None, &mut gen).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("EV-8888")), "幻觉引用必须告警: {:?}", out.warnings);
    assert!(!out.content.contains("EV-8888"), "幻觉引用必须被剔除");
    assert!(!out.citations.contains(&"EV-8888".to_string()));
}

#[test]
fn draft_section_restarts_from_disk() {
    let dir = tmp_dir("restart");
    let sid;
    let (chars, attempt);
    {
        let mut svc = BidService::open(&dir).unwrap();
        svc.create_project("重启测试", "C-RST", "明标", "restart-key").unwrap();
        svc.store_mut().upsert_file("A.docx", "招标文件", "sha-r", 5, 0, 30, 0, ParseStatus::Ok, "").unwrap();
        // 补遗件（完整性警告面不进场）
        svc.store_mut().upsert_file("C-补遗.docx", "补遗", "sha-r-add", 3, 0, 10, 0, ParseStatus::Ok, "").unwrap();
        svc.set_requirements(vec![
            req("REQ-0001", "资格条件", "具备贰级资质"),
            req("REQ-0002", "否决项", "未签字作否决"),
        ]).unwrap();
        svc.prepare_outline_and_matrix(OutlineSpec { target_total_chars: 2_000, max_reqs_per_leaf: 2 }).unwrap();
        // 素材面（readiness 前置闸：资格叶需资质证据，否决项需矩阵 RESPONDED）
        let eid = register_ev(&mut svc, "水利水电工程施工总承包贰级资质证书", None);
        for rid in ["REQ-0001", "REQ-0002"] {
            svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
        }
        let _ = svc.store_mut().build_matrix().unwrap();
        sid = first_leaf(&svc);
        let mut gen = MockGenerator;
        let out = svc.draft_section(&sid, None, None, None, &mut gen).unwrap();
        chars = out.chars;
        attempt = out.attempt;
    }
    // 重启：store + 章节工作区双恢复
    let svc = BidService::open(&dir).unwrap();
    let node = svc.store().sections().iter().find(|n| n.id == sid).unwrap();
    assert_eq!(node.content_attempt, attempt);
    assert_eq!(node.content.as_ref().unwrap().chars().count() as u32, chars);
    assert_eq!(node.status.as_str(), "DRAFTED");
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Drafting);
    let asm = bid_tools::assembly::ChapterAssembler::open(&dir).unwrap();
    assert!(asm.has_committed(&sid));
}

// ---------------------------------------------------------------- 5. 状态机补全面（domain 层回归）

#[test]
fn advance_project_status_follows_chain_and_rejects_skips() {
    let mut s = DomainStore::new();
    s.create_project("状态机", "C-ADV", "明标", "adv-key").unwrap();
    s.upsert_file("A.docx", "招标文件", "sha-adv", 1, 0, 5, 0, ParseStatus::Ok, "").unwrap();
    // FILES_IMPORTED 直跳 DRAFTING（跳级）→ 拒
    let err = s.advance_project_status(ProjectStatus::Drafting).unwrap_err();
    assert_eq!(err.code(), "E_STATE");
    // 推进到 REQUIREMENTS_EXTRACTED（合法前向）
    s.set_requirements(vec![req("REQ-0001", "资格条件", "资质要求")]).unwrap();
    assert_eq!(s.project().unwrap().status, ProjectStatus::RequirementsExtracted);
    // 同态 no-op（幂等——不 bump 不报错）
    let v = s.content_version();
    s.advance_project_status(ProjectStatus::RequirementsExtracted).unwrap();
    assert_eq!(s.content_version(), v, "幂等 no-op 不 bump 版本");
    // 回退 → 拒
    let err = s.advance_project_status(ProjectStatus::FilesImported).unwrap_err();
    assert_eq!(err.code(), "E_STATE", "回退必须拒绝");
}

#[test]
fn set_section_content_rejects_structural_node_and_cas_semantics() {
    let mut s = DomainStore::new();
    s.create_project("CAS", "C-CAS", "明标", "cas-key").unwrap();
    s.upsert_file("A.docx", "招标文件", "sha-cas", 1, 0, 5, 0, ParseStatus::Ok, "").unwrap();
    s.set_requirements(vec![req("REQ-0001", "资格条件", "资质要求"), req("REQ-0002", "否决项", "签字要求")]).unwrap();
    s.derive_and_set_outline(OutlineSpec { target_total_chars: 2_000, max_reqs_per_leaf: 2 }).unwrap();
    let structural = s.sections().iter().find(|n| n.parent_id.is_none()).unwrap().id.clone();
    let leaf = {
        let parents: HashSet<&str> = s.sections().iter().filter_map(|n| n.parent_id.as_deref()).collect();
        s.sections().iter().find(|n| !parents.contains(&n.id[..])).unwrap().id.clone()
    };
    // 结构节点拒正文
    let err = s.set_section_content(&structural, "不该写进结构节点".into()).unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE");
    // CAS：首写 attempt=1 → 同内容 no-op → 异内容 attempt=2
    let (a1, c1) = s.set_section_content(&leaf, "第一章正文内容。".into()).unwrap();
    assert_eq!(a1, 1);
    let v = s.content_version();
    let (a2, c2) = s.set_section_content(&leaf, "第一章正文内容。".into()).unwrap();
    assert_eq!((a2, c2), (a1, c1), "同内容 no-op");
    assert_eq!(s.content_version(), v, "no-op 不 bump");
    let (a3, _) = s.set_section_content(&leaf, "修订后的正文内容。".into()).unwrap();
    assert_eq!(a3, 2, "异内容 attempt+1");
    // 不存在
    let err = s.set_section_content("SEC-9999", "x".into()).unwrap_err();
    assert_eq!(err.code(), "E_NOTFOUND");
}

#[test]
fn build_matrix_promotes_to_matrix_ready_once() {
    let mut s = DomainStore::new();
    s.create_project("矩阵推进", "C-MTX", "明标", "mtx-key").unwrap();
    s.upsert_file("A.docx", "招标文件", "sha-m2", 1, 0, 5, 0, ParseStatus::Ok, "").unwrap();
    s.set_requirements(vec![req("REQ-0001", "资格条件", "资质")]).unwrap();
    s.derive_and_set_outline(OutlineSpec::default()).unwrap();
    let _ = s.build_matrix().unwrap();
    assert_eq!(s.project().unwrap().status, ProjectStatus::MatrixReady);
    // 重跑幂等（不回退不重复推进）
    let _ = s.build_matrix().unwrap();
    assert_eq!(s.project().unwrap().status, ProjectStatus::MatrixReady);
}

// ---------------------------------------------------------------- 6. 错误码透传

#[test]
fn draft_error_codes_are_typed() {
    let mut svc = svc_matrix_ready("err-codes");
    // 不存在章节
    let err = svc.draft_section("SEC-9999", None, None, None, &mut MockGenerator).unwrap_err();
    assert_eq!(err.code(), "E_NOTFOUND");
    // 生成器失败 → E_UNPROCESSABLE（typed 透传，不是 panic）
    struct Failing;
    impl SectionGenerator for Failing {
        fn generate(&mut self, _i: &SectionDraftInput) -> Result<bid_tools::draft::GeneratedDraft, String> {
            Err("模型超时".into())
        }
    }
    let sid = first_leaf(&svc);
    let err = svc.draft_section(&sid, None, None, None, &mut Failing).unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE");
    assert!(matches!(err, ToolError::EUnprocessable { .. }));
}

// ---------------------------------------------------------------- 7. 无锚承诺告警（#86 摩擦 #11）

/// 固定正文生成器（测告警面——正文带无锚承诺句 + 有效 REQ 引用保覆盖度）。
struct UnanchoredBody {
    title: String,
}

impl SectionGenerator for UnanchoredBody {
    fn generate(&mut self, _i: &SectionDraftInput) -> Result<bid_tools::draft::GeneratedDraft, String> {
        Ok(bid_tools::draft::GeneratedDraft {
            content: format!(
                "# {}\n\n针对 [REQ-0001] 我方完全响应。我方承诺按时竣工。\n\n保证安全生产，确保一次验收合格。",
                self.title
            ),
        })
    }
}

#[test]
fn unanchored_claims_positive_emits_warning() {
    // 契约 §6：「unanchored_claims>0 时 warning」——无锚承诺句（承诺按时竣工/
    // 保证安全生产/确保…合格）应触发告警；阻断语义留给 validate（这里只验
    // warning 在场 + 计数一致）
    let mut svc = svc_matrix_ready("unanchored-warn");
    let sid = first_leaf(&svc);
    let title = svc.store().sections().iter().find(|n| n.id == sid).unwrap().title.clone();
    let out = svc
        .draft_section(&sid, None, None, None, &mut UnanchoredBody { title })
        .unwrap();
    assert!(out.unanchored_claims >= 2, "无锚承诺句计数: {}", out.unanchored_claims);
    assert!(
        out.warnings.iter().any(|w| w.contains("无锚承诺")),
        "warning 应在场（契约 §6 承诺面）: {:?}",
        out.warnings
    );
    // 带锚路径不告警（mock 模板句句带锚——既有断言 unanchored_claims==0 的
    // 镜像：warnings 不含无锚面）。
    // P0-1（#121）后本 fixture 大纲仅一叶：资格+否决合入「资格与响应」组
    // （2 REQ / 每叶 2 条 = 单叶）、评分项不进正文——「第二个叶」不再存在。
    // 改用同章换 MockGenerator 重起草验证（语义等价：带锚正文 → 零无锚零告警）。
    let out2 = svc.draft_section(&sid, None, None, None, &mut MockGenerator).unwrap();
    assert_eq!(out2.unanchored_claims, 0);
    assert!(
        !out2.warnings.iter().any(|w| w.contains("无锚承诺")),
        "零无锚不应告警: {:?}",
        out2.warnings
    );
}
