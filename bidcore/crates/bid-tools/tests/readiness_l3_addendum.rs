//! readiness_l3_addendum.rs —— L3：补遗缺失/不就绪必须阻塞（issue #103 之后续，
//! #184 升级为分级就绪 + 逐件确认）。
//!
//! 事故形态（Lead 点名）：
//!
//! ```text
//! C-补遗文件01.pdf 因 CMap 缺失/加密/滤镜不支持 → parse_status = failed
//!   → E_PARTIAL 只说「1 件导入失败」→ 链路继续跑 → 校验 blocker = 0
//!   → 用户看到「一切正常」→ 按旧条款做标 → 废标
//! ```
//!
//! 验收线（#103 → #184）：
//! - 无补遗件 → `MissingAddendum { NotProvided }`（人工出口：AddendumWaiver 门）
//! - 有补遗但解析失败 → `MissingAddendum { ParseFailed }`，**逐件点名**
//! - 补遗全部就绪（Ok / OcrRequired 且已出块）→ 不报
//! - **#184 A**：部分失败（3 份坏 1 份）→ **阻塞**，消息点名坏件名与原因
//!   （翻案：旧判定「只有全坏才拦、部分失败放行」——不知道 ≠ 没有）
//! - **#184 B**：OcrRequired 且 ocr_blocks==0（零文本）→ **阻塞**
//!   （OcrPending——「OCR 没产出内容」不能当「没有内容」放行）
//! - **#184 C**：逐件 `record_addendum_waiver` → 该件不再阻塞，记录可查
//! - **#184 D**：全部就绪 → 直接放行
//! - 真实语料：`fixtures/corpus-A/files/C-补遗文件01.docx`

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::readiness::{
    addendum_statuses, check_draft_readiness, is_addendum_file, AddendumReadiness, Blocker,
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

/// 导入一份补遗件（指定解析状态；ocr_blocks=0——零文本形态，#184 B 的默认面）。
/// 返回 file_id（逐件确认的锚）。
fn import_addendum(s: &mut DomainStore, name: &str, status: ParseStatus, sha: &str) -> String {
    s.upsert_file(name, "补遗", sha, 3, 1, 20, 0, status, "engine-docx @ test")
        .unwrap()
        .0
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

// ---------------------------------------------------------------- ② ParseFailed（全坏）

#[test]
fn failed_addendum_blocks_with_parse_failed() {
    let mut store = base_store();
    import_addendum(&mut store, "C-补遗文件01.pdf", ParseStatus::Failed, "sha-add1");
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("补遗解析失败必须阻塞");
    match b {
        Blocker::MissingAddendum { reason, files } => {
            assert_eq!(*reason, AddendumIssue::ParseFailed);
            // #184：files 是逐件状态展开（id/名/就绪度/原因），不再只是名字串
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].name, "C-补遗文件01.pdf");
            assert_eq!(files[0].readiness, AddendumReadiness::Failed);
            assert!(files[0].file_id.starts_with("TF-"), "逐件确认的锚必须在场：{:?}", files[0]);
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

// ---------------------------------------------------------------- ④ #184 验收 A：部分失败也阻塞（翻案）

/// **#184 翻案用例**（旧名 `partially_failed_addendum_does_not_block_but_is_reported`）。
///
/// 当初为何放行：旧判定 `failed.len() == addenda.len()` 才阻塞——怕「3 份补遗
/// 坏 1 份」拦住整条工作流，过度阻塞让工具不可用，另留报告面暴露失败件。
/// 为何推翻：**不知道 ≠ 没有**——坏的那份可能正是改控制价/截止时间的，
/// 读不到不等于没改；按旧条款报价是废标（不可逆），工作流被拦是可修复的
/// 摩擦（修复重传 / 逐件确认放行——出口在本文件 ⑨）。分级阻塞让「放行」
/// 也变成留底的可审计行为，而非静默赌概率。
#[test]
fn partial_failure_blocks_with_file_names() {
    let mut store = base_store();
    import_addendum(&mut store, "C-补遗01.docx", ParseStatus::Ok, "sha-a1");
    import_addendum(&mut store, "C-补遗02.docx", ParseStatus::Ok, "sha-a2");
    store
        .upsert_file(
            "C-补遗03.pdf", "补遗", "sha-a3", 0, 0, 0, 0,
            ParseStatus::Failed, "ToUnicode CMap 缺失：文本层无法解码",
        )
        .unwrap();
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("3 份补遗坏 1 份必须阻塞（#184 验收 A——部分失败也是「不知道」）");
    match b {
        Blocker::MissingAddendum { reason, files } => {
            assert_eq!(*reason, AddendumIssue::ParseFailed);
            assert_eq!(files.len(), 1, "只点名坏件（好件不进阻塞清单）：{files:?}");
            assert_eq!(files[0].name, "C-补遗03.pdf");
            assert_eq!(files[0].readiness, AddendumReadiness::Failed);
            assert!(files[0].reason.contains("CMap"), "要点明坏在哪：{files:?}");
        }
        other => panic!("期望 MissingAddendum，实际 {other:?}"),
    }
    // 验收 A：消息点名坏件名与原因，好件不混入
    let msg = b.to_message();
    assert!(msg.contains("C-补遗03.pdf"), "要点名坏件：{msg}");
    assert!(msg.contains("CMap"), "要点明原因：{msg}");
    assert!(!msg.contains("C-补遗01"), "好件不进拒稿理由：{msg}");
    assert!(!msg.contains("C-补遗02"), "好件不进拒稿理由：{msg}");
}

// ---------------------------------------------------------------- ⑤ #184 验收 B：零文本扫描件阻塞（翻案）

/// **#184 翻案用例**（旧名 `ocr_required_addendum_is_not_a_failure`——夹具
/// `import_addendum` 传 ocr_blocks=0，恰是零文本形态）。
///
/// 当初为何放行：旧判定把 OcrRequired 一律当「已知可处理态」不算失败——
/// 对**已 OCR 出内容**的件是对的；对**零块**件是把「不知道」当「没有」。
/// 域层暂无「OCR 跑过但零产出」与「还没跑」的区分信号，合一态保守阻塞
/// （不虚构区分）。正例见 `ocr_required_with_extracted_blocks_passes`。
#[test]
fn ocr_required_zero_text_addendum_blocks() {
    let mut store = base_store();
    // 扫描件补遗：无文本层且 OCR 未产出任何内容（零块）——内容未知
    import_addendum(&mut store, "C-补遗-扫描件.pdf", ParseStatus::OcrRequired, "sha-a3");
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("零文本扫描件必须阻塞（#184 验收 B——内容未知 ≠ 没有内容）");
    match b {
        Blocker::MissingAddendum { reason, files } => {
            assert_eq!(*reason, AddendumIssue::OcrPending);
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].name, "C-补遗-扫描件.pdf");
            assert_eq!(files[0].readiness, AddendumReadiness::OcrPending);
            assert!(!files[0].reason.is_empty(), "零块件要说清「内容未知」：{files:?}");
        }
        other => panic!("期望 MissingAddendum，实际 {other:?}"),
    }
    let msg = b.to_message();
    assert!(msg.contains("C-补遗-扫描件.pdf"), "要点名文件：{msg}");
    assert!(msg.contains("OCR"), "要说清 OCR 未完成：{msg}");
}

/// OcrRequired 且 ocr_blocks > 0（OCR 已产出内容）→ Ready 放行——
/// 与 service.rs `ocr_tier_of` 的 "ocr" 档同口径（正例，保留旧用例的合理面）。
#[test]
fn ocr_required_with_extracted_blocks_passes() {
    let mut store = base_store();
    store
        .upsert_file(
            "C-补遗-扫描件.pdf", "补遗", "sha-a3b", 2, 0, 0, 12,
            ParseStatus::OcrRequired, "ocr @ 12 blocks",
        )
        .unwrap();
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(
        has_blocker(&r).is_none(),
        "OCR 已出内容的件是可读的，不该阻塞：{:?}",
        r.blockers
    );
    // 报告面就绪度 = Ready（分级基元的正向钉死）
    let st = addendum_statuses(&store);
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].readiness, AddendumReadiness::Ready);
}

/// 混存取重：Failed 与 OcrPending 同场 → 分类标签取更重的一档 ParseFailed
/// （逐件真相在 files 里逐件展开，不因取重丢失细节）。
#[test]
fn mixed_failed_and_ocr_pending_classifies_as_parse_failed() {
    let mut store = base_store();
    import_addendum(&mut store, "C-补遗-扫.pdf", ParseStatus::OcrRequired, "sha-m1");
    import_addendum(&mut store, "C-补遗-坏.pdf", ParseStatus::Failed, "sha-m2");
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("两件都不就绪必须阻塞");
    match b {
        Blocker::MissingAddendum { reason, files } => {
            assert_eq!(*reason, AddendumIssue::ParseFailed, "混存取更重一档");
            assert_eq!(files.len(), 2, "逐件真相不因取重丢失：{files:?}");
            assert!(files.iter().any(|f| f.readiness == AddendumReadiness::OcrPending));
            assert!(files.iter().any(|f| f.readiness == AddendumReadiness::Failed));
        }
        other => panic!("期望 MissingAddendum，实际 {other:?}"),
    }
}

// ---------------------------------------------------------------- ⑨ #184 验收 C：逐件确认放行 + 可查

/// 逐件 `record_addendum_waiver`：只救被确认的那一件（非全局布尔）——
/// 「3 份补遗坏 2 份」要确认两次才放行，每次确认对齐到具体 file_id 与理由。
#[test]
fn per_file_waiver_releases_parse_failed_and_is_queryable() {
    let mut store = base_store();
    import_addendum(&mut store, "C-补遗01.docx", ParseStatus::Ok, "sha-w1");
    let fid_bad1 = import_addendum(&mut store, "C-补遗02.pdf", ParseStatus::Failed, "sha-w2");
    let fid_bad2 = import_addendum(&mut store, "C-补遗03.pdf", ParseStatus::Failed, "sha-w3");
    // 确认前：阻塞（fail-closed）
    assert!(has_blocker(&check_draft_readiness(&store, "SEC-0001")).is_some());

    // 只确认第一份坏件 → 仍阻塞（第二份未确认——逐件语义的正身）
    let rec1 = store
        .record_addendum_waiver(&fid_bad1, "已人工核对纸质原件，本件无实质条款变化", "human:l3-lead")
        .unwrap();
    assert_eq!(rec1.file_id, fid_bad1);
    assert_eq!(rec1.principal, "human:l3-lead");
    assert!(rec1.at > 0, "确认时刻落钟");
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("另一份坏件未确认，仍必须阻塞");
    match b {
        Blocker::MissingAddendum { reason, files } => {
            assert_eq!(*reason, AddendumIssue::ParseFailed);
            assert_eq!(files.len(), 1, "已确认件不计入，未确认件点名：{files:?}");
            assert_eq!(files[0].file_id, fid_bad2);
        }
        other => panic!("期望 MissingAddendum，实际 {other:?}"),
    }

    // 确认第二份 → 不再阻塞（验收 C：逐件确认放行）
    let rec2 = store
        .record_addendum_waiver(&fid_bad2, "扫描原件比对，条款与正文一致", "human:l3-lead")
        .unwrap();
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(
        has_blocker(&r).is_none(),
        "全部不就绪件逐件确认后必须放行：{:?}",
        r.blockers
    );

    // 记录可查（验收 C：出参/查询面含该记录——审计面不因放行消失）
    let ws = store.addendum_waivers();
    assert_eq!(ws.len(), 2, "追加式审计日志");
    assert_eq!(ws[0].file_id, fid_bad1);
    assert_eq!(ws[0].reason, "已人工核对纸质原件，本件无实质条款变化");
    assert_eq!(ws[1].file_id, fid_bad2);
    // 报告面：两件 waived=true，就绪度仍是 FAILED（确认不改事实，只免责）
    let st = addendum_statuses(&store);
    for fid in [&fid_bad1, &fid_bad2] {
        let s = st.iter().find(|s| &s.file_id == fid).unwrap();
        assert!(s.waived, "已确认件在报告面标记 waived：{s:?}");
        assert_eq!(s.readiness, AddendumReadiness::Failed, "确认不改就绪度事实：{s:?}");
    }
}

/// 负例：waiver 的 file_id 不存在 → 拒（对不存在的件做确认 = 凭空免责）；
/// principal 无 human: 前缀 → 拒（AI 不得自审自过——与复审人同口径）。
#[test]
fn per_file_waiver_rejects_ghost_file_and_non_human_principal() {
    let mut store = base_store();
    let fid = import_addendum(&mut store, "C-补遗01.pdf", ParseStatus::Failed, "sha-n1");
    assert!(matches!(
        store.record_addendum_waiver("TF-9999", "r", "human:x"),
        Err(bid_domain::ToolError::ENotFound { .. })
    ), "幽灵 file_id 必须拒");
    assert!(matches!(
        store.record_addendum_waiver(&fid, "r", "agent:bot"),
        Err(bid_domain::ToolError::EUnprocessable { .. })
    ), "非 human: 前缀必须拒");
    assert!(store.addendum_waivers().is_empty(), "拒绝路径零副作用");
}

// ---------------------------------------------------------------- ⑩ #184 验收 D：全部就绪直接放行

#[test]
fn all_ready_addenda_pass_directly() {
    let mut store = base_store();
    import_addendum(&mut store, "C-补遗01.docx", ParseStatus::Ok, "sha-d1");
    import_addendum(&mut store, "C-补遗02.docx", ParseStatus::Ok, "sha-d2");
    store
        .upsert_file(
            "C-补遗03-扫描件.pdf", "补遗", "sha-d3", 2, 0, 0, 8,
            ParseStatus::OcrRequired, "ocr @ 8 blocks",
        )
        .unwrap();
    let r = check_draft_readiness(&store, "SEC-0001");
    assert!(
        has_blocker(&r).is_none(),
        "全部就绪（Ok + OCR 已出块）必须直接放行，不得要求任何确认：{:?}",
        r.blockers
    );
    // 就绪度报告面：3 件全 Ready
    let st = addendum_statuses(&store);
    assert_eq!(st.len(), 3);
    assert!(st.iter().all(|s| s.readiness == AddendumReadiness::Ready), "{st:?}");
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
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].name, "C-补遗文件01.docx");
            assert_eq!(files[0].readiness, AddendumReadiness::Failed);
        }
        other => panic!("期望 MissingAddendum，实际 {other:?}"),
    }
}

// ---------------------------------------------------------------- ⑪ AddendumWaiver 全局门（#131）
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

/// 全局门不掩蔽新失败态（#184 后语义更重：不掩蔽**任何**不就绪件——
/// Failed 与 OcrPending 都不能被「人工确认无补遗」洗白）。
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

/// 全局门同样不掩蔽 OcrPending（零文本件——#184 B 与全局门的交叉面）。
#[test]
fn addendum_waiver_does_not_mask_ocr_pending() {
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
    import_addendum(&mut store, "C-补遗-扫描件.pdf", ParseStatus::OcrRequired, "sha-add-ocr");
    let r = check_draft_readiness(&store, "SEC-0001");
    let b = has_blocker(&r).expect("零文本扫描件不因全局 waiver 放行");
    match b {
        Blocker::MissingAddendum { reason, .. } => {
            assert_eq!(*reason, AddendumIssue::OcrPending);
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
