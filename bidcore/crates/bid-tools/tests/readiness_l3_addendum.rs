//! readiness_l3_addendum.rs —— L3：补遗缺失必须阻塞（issue #103 之后续）。
//!
//! 事故形态（Lead 点名）：
//!
//! ```text
//! C-补遗文件01.pdf 因 CMap 缺失/加密/滤镜不支持 → parse_status = failed
//!   → E_PARTIAL 只说「1 件导入失败」→ 链路继续跑 → 校验 blocker = 0
//!   → 用户看到「一切正常」→ 按旧条款做标 → 废标
//! ```
//!
//! 验收线：
//! - 无补遗件 → `MissingAddendum { NotProvided }`
//! - 有补遗但 failed → `MissingAddendum { ParseFailed }`
//! - 补遗成功导入 → 不报
//! - 真实语料：`fixtures/corpus-A/files/C-补遗文件01.docx`

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::readiness::{
    addendum_parse_failures, check_draft_readiness, is_addendum_file, Blocker,
    AddendumIssue, DraftReadiness,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 夹具

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// 基座：招标文件 + 1 条条款 + 1 个叶章节（矩阵已建）。
fn base_store() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("补遗缺失测试", "ZFCG-2026-L3-001", "明标", "l3-key").unwrap();
    s.upsert_file(
        "A-招标文件.docx",
        "招标文件",
        "sha-tender",
        120, 8, 900, 0,
        ParseStatus::Ok,
        "engine-docx @ test",
    )
    .unwrap();
    s.set_requirements(vec![Requirement {
        id: "REQ-0001".into(),
        category: "技术参数".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第三章/REQ-0001".into(),
            excerpt: "REQ-0001 的条款原文摘录".into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }])
    .unwrap();
    s.set_outline(vec![OutlineNode {
        id: "SEC-0001".into(),
        parent_id: None,
        title: "总体技术方案".into(),
        level: 1,
        order: 1,
        requirement_ids: vec!["REQ-0001".into()],
        target_chars: 800,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }])
    .unwrap();
    s.build_matrix().unwrap();
    s
}

/// 导入一份补遗件（指定解析状态）。
fn import_addendum(s: &mut DomainStore, name: &str, status: ParseStatus, sha: &str) {
    s.upsert_file(name, "补遗", sha, 3, 1, 20, 0, status, "engine-docx @ test").unwrap();
}

fn has_blocker(r: &DraftReadiness) -> Option<&Blocker> {
    r.blockers.iter().find(|b| matches!(b, Blocker::MissingAddendum { .. }))
}

// ---------------------------------------------------------------- ① NotProvided

#[test]
fn no_addendum_blocks_with_not_provided() {
    let store = base_store();
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("缺补遗必须阻塞");
    match b {
        Blocker::MissingAddendum { reason, files } => {
            assert_eq!(*reason, AddendumIssue::NotProvided);
            assert!(files.is_empty(), "没补遗件时 files 应为空，实际 {files:?}");
        }
        other => panic!("期望 MissingAddendum，实际 {other:?}"),
    }
    assert!(!r.ready, "有阻塞时 ready 必须为 false");
    // 人话：不含 enum 名
    let msg = b.to_message();
    assert!(msg.contains("补遗"), "消息要点明补遗：{msg}");
    assert!(!msg.contains("MissingAddendum"), "人话里不该出现 enum 名：{msg}");
}

// ---------------------------------------------------------------- ② ParseFailed

#[test]
fn failed_addendum_blocks_with_parse_failed() {
    let mut store = base_store();
    import_addendum(&mut store, "C-补遗文件01.pdf", ParseStatus::Failed, "sha-add1");
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("补遗解析失败必须阻塞");
    match b {
        Blocker::MissingAddendum { reason, files } => {
            assert_eq!(*reason, AddendumIssue::ParseFailed);
            assert_eq!(files, &vec!["C-补遗文件01.pdf".to_string()]);
        }
        other => panic!("期望 MissingAddendum，实际 {other:?}"),
    }
    let msg = b.to_message();
    assert!(msg.contains("解析失败"), "消息要说清解析失败：{msg}");
    assert!(msg.contains("C-补遗文件01.pdf"), "消息要点名文件：{msg}");
}

// ---------------------------------------------------------------- ③ 成功导入 → 不报

#[test]
fn successfully_imported_addendum_does_not_block() {
    let mut store = base_store();
    import_addendum(&mut store, "C-补遗文件01.docx", ParseStatus::Ok, "sha-add-ok");
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(has_blocker(&r).is_none(), "补遗成功导入不应报 MissingAddendum：{:?}", r.blockers);
}

// ---------------------------------------------------------------- ④ 部分失败不阻塞但暴露

#[test]
fn partially_failed_addendum_does_not_block_but_is_reported() {
    let mut store = base_store();
    import_addendum(&mut store, "C-补遗01.pdf", ParseStatus::Ok, "sha-a1");
    import_addendum(&mut store, "C-补遗02.pdf", ParseStatus::Failed, "sha-a2");
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(
        has_blocker(&r).is_none(),
        "读到了补遗就不阻塞（过度阻塞会让工具不可用）：{:?}",
        r.blockers
    );
    // 但失败件必须能在报告面露出来
    let fails = addendum_parse_failures(&store);
    assert_eq!(fails, vec!["C-补遗02.pdf".to_string()], "失败件清单要准确");
}

// ---------------------------------------------------------------- ⑤ OCR 不算失败

#[test]
fn ocr_required_addendum_is_not_a_failure() {
    let mut store = base_store();
    // 扫描件补遗：无文本层但是已知可处理态（ParseStatus 三分语义）
    import_addendum(&mut store, "C-补遗-扫描件.pdf", ParseStatus::OcrRequired, "sha-a3");
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(has_blocker(&r).is_none(), "OcrRequired 不是失败，不该阻塞：{:?}", r.blockers);
}
// ---------------------------------------------------------------- ⑥ 文件名/类别识别

#[test]
fn addendum_recognition_covers_kind_and_filename() {
    // 真实语料实测命名（fixtures/corpus-A）：C-补遗文件01.docx
    assert!(is_addendum_file("附件", "C-补遗文件01.docx"), "靠文件名认补遗");
    assert!(is_addendum_file("补遗", "任何名字.pdf"), "靠 kind 认补遗");
    assert!(is_addendum_file("答疑", "x.pdf"));
    assert!(is_addendum_file("", "澄清公告.pdf"));
    assert!(is_addendum_file("", "更正公告.pdf"));
    // 不能误伤
    assert!(!is_addendum_file("招标文件", "A-招标文件.docx"));
    assert!(!is_addendum_file("设备清单", "B1-设备清单.pdf"));
    assert!(!is_addendum_file("图纸", "B3-图纸目录.xlsx"));
}

// ---------------------------------------------------------------- ⑦ 没有招标文件时不报补遗

#[test]
fn no_tender_file_does_not_report_missing_addendum() {
    // 只有附件、没有招标文件本体 → 该报的是空包告警（tender_gaps），
    // 不是「缺补遗」——空包比缺补遗更上游，重复报会误导
    let mut s = DomainStore::new();
    s.create_project("空包", "ZFCG-L3-EMPTY", "明标", "k").unwrap();
    s.upsert_file("B1-设备清单.pdf", "附件", "sha-b1", 5, 1, 30, 0, ParseStatus::Ok, "").unwrap();
    s.set_requirements(vec![Requirement {
        id: "REQ-0001".into(),
        category: "技术参数".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "B1-设备清单.pdf".into(),
            kind: "para".into(),
            locator: "L1".into(),
            excerpt: "摘录".into(),
        }],
        params: Default::default(),
        source_file: "B1-设备清单.pdf".into(),
    }]).unwrap();
    s.set_outline(vec![OutlineNode {
        id: "SEC-0001".into(),
        parent_id: None,
        title: "总体方案".into(),
        level: 1,
        order: 1,
        requirement_ids: vec!["REQ-0001".into()],
        target_chars: 800,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }]).unwrap();
    s.build_matrix().unwrap();
    let r = check_draft_readiness(&s, "SEC-0001");
    assert!(
        has_blocker(&r).is_none(),
        "没有招标文件本体时不该报缺补遗（那是空包告警的面）：{:?}",
        r.blockers
    );
}

// ---------------------------------------------------------------- ⑧ 真实语料

#[test]
fn real_corpus_addendum_docx_imports_without_blocking() {
    let f = fixtures().join("fixtures/corpus-A/files/C-补遗文件01.docx");
    if bid_parse::env_preflight::skip_if_fixture_missing("real_corpus_addendum", &[&f]) {
        return;
    }
    let mut store = base_store();
    // 真实补遗件：名字是 C-补遗文件01.docx（既非 kind 也非标准词——
    // 靠名字里的「补遗」认出来，正是 is_addendum_file 的设计面）
    store
        .upsert_file(
            "C-补遗文件01.docx",
            "附件",
            "c0f0e3da2f3dbb1ac864be3c85c53544cabcab728e1e9854b3b03ec70e04c1f0",
            3, 1, 20, 0,
            ParseStatus::Ok,
            "engine-docx @ real",
        )
        .unwrap();
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(
        has_blocker(&r).is_none(),
        "真实补遗件导入后不应阻塞：{:?}",
        r.blockers
    );
}

#[test]
fn real_corpus_addendum_parse_failure_blocks() {
    let f = fixtures().join("fixtures/corpus-A/files/C-补遗文件01.docx");
    if bid_parse::env_preflight::skip_if_fixture_missing("real_corpus_addendum_fail", &[&f]) {
        return;
    }
    // 同一个真实文件，模拟「CMap 缺失导致解析失败」的现场
    let mut store = base_store();
    store
        .upsert_file(
            "C-补遗文件01.docx",
            "附件",
            "sha-real-add-fail",
            0, 0, 0, 0,
            ParseStatus::Failed,
            "ToUnicode CMap 缺失：文本层无法解码",
        )
        .unwrap();
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("真实补遗件解析失败必须阻塞");
    match b {
        Blocker::MissingAddendum { reason, files } => {
            assert_eq!(*reason, AddendumIssue::ParseFailed);
            assert_eq!(files, &vec!["C-补遗文件01.docx".to_string()]);
        }
        other => panic!("期望 MissingAddendum，实际 {other:?}"),
    }
}

// ---------------------------------------------------------------- ⑤ AddendumWaiver（#131）
// 真实语料验证发现的缺口：L3 阻塞提示语承诺「明确确认本项目无补遗后再继续」，
// 但确认通路此前不存在——单件真实招标文件（无补遗件）被死锁在起草闸外。
// Gate::AddendumWaiver Approved = 人工确认无补遗 → NotProvided 阻塞放行。

#[test]
fn addendum_waiver_releases_not_provided_blocker() {
    let mut store = base_store();
    // 确认前：阻塞在场（fail-closed 面——waiver 不是默认态）
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(has_blocker(&r).is_some(), "确认前必须阻塞（fail-closed）");

    // 人工确认（human: 前缀——AI 不得自审自过）
    use bid_domain::review::{Decision, Gate};
    store
        .record_review(
            Gate::AddendumWaiver,
            "human:real-corpus-lead",
            Decision::Approved,
            vec![],
        )
        .unwrap();
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(
        has_blocker(&r).is_none(),
        "AddendumWaiver Approved 后 NotProvided 阻塞必须放行：{:?}",
        r.blockers
    );
}

#[test]
fn addendum_waiver_does_not_mask_parse_failed() {
    // waiver 只救「没补遗」分支：补遗在场但解析失败（ParseFailed）时，
    // 人工确认不能把「读不了的补遗」洗成「没有补遗」——那是两种废标风险
    let mut store = base_store();
    use bid_domain::review::{Decision, Gate};
    store
        .record_review(
            Gate::AddendumWaiver,
            "human:real-corpus-lead",
            Decision::Approved,
            vec![],
        )
        .unwrap();
    import_addendum(&mut store, "C-补遗文件01.pdf", ParseStatus::Failed, "sha-add1");
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("补遗解析失败不因 waiver 放行");
    match b {
        Blocker::MissingAddendum { reason, .. } => {
            assert_eq!(*reason, AddendumIssue::ParseFailed);
        }
        other => panic!("期望 MissingAddendum，实际 {other:?}"),
    }
}

#[test]
fn addendum_waiver_survives_reimport_of_addendum() {
    // 确认后又导入成功补遗：正常路径（不阻塞），waiver 记录无害残留
    let mut store = base_store();
    use bid_domain::review::{Decision, Gate};
    store
        .record_review(
            Gate::AddendumWaiver,
            "human:real-corpus-lead",
            Decision::Approved,
            vec![],
        )
        .unwrap();
    import_addendum(&mut store, "C-补遗文件02.docx", ParseStatus::Ok, "sha-add2");
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(has_blocker(&r).is_none(), "补遗在场时 waiver 不影响正常判定");
}
