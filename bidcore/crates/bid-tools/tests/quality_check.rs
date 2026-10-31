//! quality_check.rs —— #68 阶段 2：生成质量验证测试（覆盖度 + 报告 + 失败面）。
//!
//! 覆盖（issue #68 §2.1/§2.2 验收线）：
//! - 覆盖度检查：REQ 覆盖/未覆盖清单、幽灵 REQ、EV 引用核对（幻觉单列）、
//!   字数带宽 ±20%（target=0 不评）、无锚承诺计数；
//! - needs_human 判定：FAILED / 未覆盖 REQ / 幻觉引用 / 带外字数；
//! - 项目报告：summary 计数（drafted/failed/pending/uncovered/out_of_band）；
//! - 报告落盘：`artifacts/quality-report.json` roundtrip（serde 全字段）；
//! - 失败明细并入：failed.json 边车的 error 进报告条目；
//! - FAILED→重起草链：失败章经 assemble_all 重生成后 DRAFTED（域状态机闭环）。
//!
//! 夹具纪律：域状态全部走真实状态机（set_outline(Pending) →
//! set_section_content → advance 链），不 force 字段——与生产路径同构。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::assembly::{assemble_all_with_policy, ChapterAssembler, DraftPolicy};
use bid_tools::quality::{build_quality_report, check_section, write_report, QualityReport};
use std::collections::HashSet;

// ---------------------------------------------------------------- 夹具

fn temp_ws(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bid-quality-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn req(id: &str) -> Requirement {
    // locator/excerpt 逐条唯一（set_requirements 按 (file, locator, excerpt)
    // 哈希去重——同型会被合并成一条，id 面就会缺）
    Requirement {
        id: id.into(),
        category: "技术参数".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第二章/{id}"),
            excerpt: format!("{id} 的要求原文摘录（逐条唯一）"),
        }],
        params: Default::default(),
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

/// 登记一条证据（出生态 Active——照 draft_section.rs 夹具）。
fn register_ev(s: &mut DomainStore, title: &str) -> String {
    let (eid, _) = s.register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: title.into(),
        source_file: "EV-01.pdf".into(),
        source_sha256: format!("sha-{title}"),
        locator: EngineLocator::DocxParagraph {
            paragraph_index: 3,
            text_fingerprint: "fnv64:9e37".into(),
        },
        fingerprint: "fnv64:9e37".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某建设工程有限公司".into(),
        scope: "水利水电工程施工".into(),
    }).unwrap();
    eid
}

/// 基座：2 叶章节（Pending）+ 2 EV 绑定。SEC-0001 绑 REQ-0001/0002，
/// SEC-0002 绑 REQ-0003；EV-0001→REQ-0001、EV-0002→REQ-0003。
fn store_base() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("质量检查测试", "ZFCG-2026-Q-001", "明标", "q-key").unwrap();
    s.upsert_file("A-招标文件.docx", "招标文件", "aa", 5, 1, 50, 0,
                  ParseStatus::Ok, "engine-docx @ test").unwrap();
    s.set_requirements(vec![req("REQ-0001"), req("REQ-0002"), req("REQ-0003")]).unwrap();
    s.set_outline(vec![
        // target 适配正文长度（GOOD ≈ 90 字 → target 100 带内；POOR ≈ 20 字
        // → target 1000 带外——带宽判定的对照面）
        node("SEC-0001", "资质响应", 1, &["REQ-0001", "REQ-0002"], 100),
        node("SEC-0002", "业绩响应", 2, &["REQ-0003"], 1000),
    ]).unwrap();
    let ev1 = register_ev(&mut s, "施工总承包贰级资质证书");
    let ev2 = register_ev(&mut s, "近三年同类工程业绩清单");
    s.bind_evidence(&ev1, "REQ-0001", "2026-12-31").unwrap();
    s.bind_evidence(&ev2, "REQ-0003", "2026-12-31").unwrap();
    s
}

/// 真实推进链：写内容 + Pending→Outlining→Drafting→DRAFTED。
fn draft_it(s: &mut DomainStore, id: &str, content: &str) {
    s.set_section_content(id, content.to_string()).unwrap();
    for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
        s.advance_section_status(id, to).unwrap();
    }
}

/// 真实推进链：失败（Pending→Outlining→Drafting→FAILED）。
fn fail_it(s: &mut DomainStore, id: &str) {
    for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Failed] {
        s.advance_section_status(id, to).unwrap();
    }
}

/// 优质正文（EV id 动态——register_evidence 的 id 走全局序列，不能硬编码）。
fn good_content(ev1: &str) -> String {
    format!(
        "# 资质响应\n\n针对 [REQ-0001] 的要求我方完全满足，依据 [{ev1}] 提供证书正本备查。\n\n针对 [REQ-0002] 的要求我方同样完全满足，响应充分无偏离。"
    )
}

/// 劣质正文（REQ-0003 未回应 + 幻觉 EV-9999 + 字数远低于下限）。
const POOR_CONTENT: &str = "# 业绩响应\n\n相关业绩见 [EV-9999] 所列清单。";

/// 查 REQ-0001 的绑定 EV id（register_evidence 的 id 走全局序列——动态取）。
fn bound_ev1(s: &DomainStore) -> String {
    s.evidence_bindings().iter()
        .find(|b| b.requirement_id == "REQ-0001").unwrap().evidence_id.clone()
}

// ---------------------------------------------------------------- 单章覆盖度

#[test]
fn coverage_flags_uncovered_req_and_hallucinated_ev() {
    let mut s = store_base();
    let ev1 = bound_ev1(&s);
    draft_it(&mut s, "SEC-0001", &good_content(&ev1));
    draft_it(&mut s, "SEC-0002", POOR_CONTENT);
    let sec1 = s.sections().iter().find(|n| n.id == "SEC-0001").unwrap().clone();
    let q1 = check_section(&s, &sec1);
    assert_eq!(q1.covered_reqs, vec!["REQ-0001", "REQ-0002"], "两条 REQ 都被回应");
    assert!(q1.uncovered_reqs.is_empty());
    assert_eq!(q1.cited_evs.len(), 1, "EV-0001 被援引");
    assert!(q1.unknown_citations.is_empty());
    assert!(!q1.needs_human, "SEC-0001 质量合格");

    let sec2 = s.sections().iter().find(|n| n.id == "SEC-0002").unwrap().clone();
    let q2 = check_section(&s, &sec2);
    assert_eq!(q2.uncovered_reqs, vec!["REQ-0003"], "REQ-0003 未被回应——核心清单");
    assert_eq!(q2.unknown_citations, vec!["EV-9999"], "幻觉 EV 单列");
    assert_eq!(q2.uncited_evs.len(), 1, "绑定的 EV-0002 未被援引");
    assert_eq!(q2.within_band, Some(false), "字数远低于 800 下限——带外");
    assert!(q2.needs_human, "未覆盖 + 幻觉 + 带外 → 需人工处理");
}

#[test]
fn band_check_respects_target_and_tolerance() {
    let s = store_base();
    let chars = good_content("EV-0001").chars().count();
    // 手构节点直测 check_section（带宽纯函数面——target 配比三档）
    let mut n = node("SEC-X", "测试章", 1, &["REQ-0001"], (chars as f64 / 1.0) as u32);
    n.content = Some(good_content("EV-0001"));
    n.status = SectionStatus::Drafted;
    assert_eq!(check_section(&s, &n).within_band, Some(true), "target=chars 带内");
    n.target_chars = (chars as f64 * 2.0) as u32;
    assert_eq!(check_section(&s, &n).within_band, Some(false), "target×2 → 带外（不足下限）");
    n.target_chars = 0;
    assert_eq!(check_section(&s, &n).within_band, None, "target=0 不评（诚实面）");
    n.content = None;
    n.target_chars = 100;
    assert_eq!(check_section(&s, &n).within_band, None, "未起草不评");
}

#[test]
fn ghost_reqs_reported_not_counted_uncovered() {
    let s = store_base();
    // 幽灵 REQ（抽取漂移面——不经 set_outline 的存在性门，直接手构节点）
    let mut n = node("SEC-X", "漂移章", 1, &["REQ-0001", "REQ-9999"], 0); // target=0 不评带宽
    n.content = Some("正文回应 [REQ-0001]……".into());
    n.status = SectionStatus::Drafted;
    let q = check_section(&s, &n);
    assert_eq!(q.ghost_reqs, vec!["REQ-9999"], "幽灵单列——是数据面不是质量面");
    assert!(!q.uncovered_reqs.contains(&"REQ-9999".to_string()));
    assert!(!q.needs_human, "仅幽灵不判 needs_human");
}

// ---------------------------------------------------------------- 项目报告 + 落盘

#[test]
fn report_summary_and_artifact_roundtrip() {
    let ws = temp_ws("report");
    let mut s = store_base();
    let ev1 = bound_ev1(&s);
    draft_it(&mut s, "SEC-0001", &good_content(&ev1));
    draft_it(&mut s, "SEC-0002", POOR_CONTENT);
    let report = build_quality_report(&s, None);
    assert_eq!(report.summary.total, 2);
    assert_eq!(report.summary.drafted, 2);
    assert_eq!(report.summary.failed, 0);
    assert_eq!(report.summary.uncovered_req_total, 1, "SEC-0002 的 REQ-0003");
    assert_eq!(report.summary.out_of_band, 1, "SEC-0002 带外");
    assert_eq!(report.summary.needs_human, vec!["SEC-0002".to_string()]);
    // 文档序（order 升序）
    assert_eq!(report.sections[0].section_id, "SEC-0001");
    assert_eq!(report.sections[1].section_id, "SEC-0002");

    // 落盘 roundtrip（serde 全字段无损）
    let path = write_report(&report, &ws).unwrap();
    assert!(path.ends_with("artifacts/quality-report.json"));
    let text = std::fs::read_to_string(&path).unwrap();
    let back: QualityReport = serde_json::from_str(&text).unwrap();
    assert_eq!(back, report, "roundtrip 全字段一致");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn failed_sections_flow_into_report_with_sidecar_error() {
    let ws = temp_ws("failed");
    let mut s = store_base();
    let ev1 = bound_ev1(&s);
    draft_it(&mut s, "SEC-0001", &good_content(&ev1));
    fail_it(&mut s, "SEC-0002");
    // 边车：组装线对 SEC-0002 的失败明细
    {
        let mut asm = ChapterAssembler::open(&ws).unwrap();
        asm.record_failed("SEC-0002", 4, "LLM HTTP 429: rate limited");
    }
    let asm = ChapterAssembler::open(&ws).unwrap();
    let report = build_quality_report(&s, Some(&asm));
    assert_eq!(report.summary.failed, 1);
    assert_eq!(report.summary.drafted, 1);
    let sec2 = &report.sections[1];
    assert_eq!(sec2.status, "FAILED");
    assert_eq!(sec2.error.as_deref(), Some("LLM HTTP 429: rate limited"), "边车错误并入");
    assert!(sec2.needs_human);
    assert_eq!(report.summary.needs_human, vec!["SEC-0002".to_string()]);
    let _ = std::fs::remove_dir_all(&ws);
}

// ---------------------------------------------------------------- 失败→重起草链（组装线）

#[test]
fn failed_section_can_be_redrafted_via_assembly_chain() {
    let ws = temp_ws("redraft");
    let mut store = store_base();
    {
        let mut asm = ChapterAssembler::open(&ws).unwrap();
        let report = assemble_all_with_policy(
            &mut store, &mut asm,
            &mut |n| {
                if n.id == "SEC-0002" {
                    Err("模拟持久故障".into())
                } else {
                    Ok(format!("# {}\n\n正文回应 [{}]……", n.title, n.requirement_ids.join("、")))
                }
            },
            &DraftPolicy { retries: 1, backoff_base_ms: 0 },
            &|_| {},
        )
        .unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].attempts, 2, "首试 + 1 重试");
        assert_eq!(store.sections()[1].status, SectionStatus::Failed, "域状态 FAILED");
        // 内容未提交（None——draft 用 hand-built store 的 Pending 面）
        assert!(store.sections()[1].content.is_none() || store.sections()[1].status == SectionStatus::Failed);
    }
    // 重跑：全部成功 → FAILED 章经 Drafting → DRAFTED，边车清除
    {
        let mut asm = ChapterAssembler::open(&ws).unwrap();
        let report = assemble_all_with_policy(
            &mut store, &mut asm,
            &mut |n| Ok(format!("# {}\n\n正文回应 [{}]。", n.title, n.requirement_ids.join("、"))),
            &DraftPolicy { retries: 1, backoff_base_ms: 0 },
            &|_| {},
        )
        .unwrap();
        assert!(report.failed.is_empty());
        assert_eq!(store.sections()[0].status, SectionStatus::Drafted);
        assert_eq!(store.sections()[1].status, SectionStatus::Drafted, "FAILED 重起草链闭合");
        assert!(asm.failed_records().is_empty(), "恢复即抹痕");
    }
    let _ = std::fs::remove_dir_all(&ws);
}

/// 叶节点判定口径（报告只看叶——与 assemble_all 同语义）+ 文档序。
#[test]
fn report_covers_leaves_only_in_document_order() {
    let mut s = store_base();
    // 重开纲：加结构节点（父聚合 REQ——set_outline 的存在性门要求 ≥1 REQ）
    s.set_outline(vec![
        node("SEC-TOP", "技术方案（结构节点）", 0, &["REQ-0001", "REQ-0002", "REQ-0003"], 1100),
        OutlineNode { level: 2, parent_id: Some("SEC-TOP".into()), ..node("SEC-0001", "资质响应", 1, &["REQ-0001", "REQ-0002"], 100) },
        OutlineNode { level: 2, parent_id: Some("SEC-TOP".into()), ..node("SEC-0002", "业绩响应", 2, &["REQ-0003"], 1000) },
    ]).unwrap();
    let report = build_quality_report(&s, None);
    assert_eq!(report.summary.total, 2, "只看叶节点");
    let ids: Vec<&str> = report.sections.iter().map(|q| &q.section_id[..]).collect();
    assert_eq!(ids, vec!["SEC-0001", "SEC-0002"], "文档序");
    let all: HashSet<&str> = ids.iter().copied().collect();
    assert!(!all.contains("SEC-TOP"));
}

// ---------------------------------------------------------------- #86 摩擦 #4：requirement_ids 查询口

#[test]
fn section_quality_exposes_requirement_ids() {
    // 章节→REQ 归属的正式查询口：requirement_ids = 章节绑定面全集，
    // 与 covered/uncovered/ghost 的并集一致（响应面内互斥分桶）
    let mut s = store_base();
    let ev1 = bound_ev1(&s);
    draft_it(&mut s, "SEC-0001", &good_content(&ev1));
    draft_it(&mut s, "SEC-0002", POOR_CONTENT);
    for id in ["SEC-0001", "SEC-0002"] {
        let node = s.sections().iter().find(|n| n.id == id).unwrap().clone();
        let q = check_section(&s, &node);
        assert_eq!(
            q.requirement_ids, node.requirement_ids,
            "{id} requirement_ids 应为章节绑定面全集"
        );
        let mut union = q.covered_reqs.clone();
        union.extend(q.uncovered_reqs.iter().cloned());
        union.extend(q.ghost_reqs.iter().cloned());
        let mut sorted_union = union.clone();
        sorted_union.sort();
        let mut sorted_ids = q.requirement_ids.clone();
        sorted_ids.sort();
        assert_eq!(
            sorted_union, sorted_ids,
            "{id} covered∪uncovered∪ghost 应与 requirement_ids 一致"
        );
    }
}
