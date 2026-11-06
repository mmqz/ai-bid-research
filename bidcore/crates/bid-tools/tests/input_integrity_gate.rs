//! input_integrity_gate.rs —— 地图 P0-1「部分失败不得错误放行」的判定面测试。
//!
//! #184 只把**补遗**的不就绪升格为阻塞；P0-1 把同一判据推广到整个招标包
//! （主件/附件/图纸——非补遗输入件）。本文件钉死：
//!
//! - 判据面 [`input_integrity`]：主件 READY + 所有补遗 READY + 无其他输入件
//!   OCR_PENDING/FAILED → complete；逐件确认放行的坏件 → restricted
//!   （受限起草的来源——放行 ≠ 没问题）；
//! - 阻塞面 [`Blocker::UnreadyInput`]：主件/附件不就绪且未确认 → 阻塞
//!   （与 [`Blocker::MissingAddendum`]（补遗面）不相交、不双报）；
//! - 边界面：证据源文件（kind=证据）不进输入完整性判定（可用性归证据
//!   系统自己的生命周期）；OCR 已产出内容（ocr_blocks>0）= Ready；
//! - 受限面：确认后 [`InputIntegrity::restricted`] = true 且逐件可见。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::readiness::{
    check_draft_readiness, input_integrity, is_tender_package_file, AddendumReadiness, Blocker,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 夹具

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// 基座：招标文件（Ok）+ 1 条条款 + 1 个叶章节（矩阵已建）。
/// 与 readiness_l3_addendum.rs 的 base_store 同款（本套件隔离 P0-1 面）。
fn base_store() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("输入完整性测试", "ZFCG-2026-P01-001", "明标", "p01-key").unwrap();
    s.upsert_file(
        "A-招标文件.docx",
        "招标文件",
        "sha-p01-tender",
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
                block_idx: None,
                page: None,
                line: None
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

/// 导入一件非补遗输入件（附件/图纸/主件形态），返回 (file_id)。
fn import_package_file(
    s: &mut DomainStore,
    name: &str,
    kind: &str,
    status: ParseStatus,
    sha: &str,
    ocr_blocks: usize,
) -> String {
    s.upsert_file(name, kind, sha, 3, 1, 20, ocr_blocks, status, "engine @ test")
        .unwrap()
        .0
}

fn unready_input_blocker(r: &bid_tools::readiness::DraftReadiness) -> Option<&Blocker> {
    r.blockers
        .iter()
        .find(|b| matches!(b, Blocker::UnreadyInput { .. }))
}

// ---------------------------------------------------------------- ① 判据面：全部就绪 → complete

#[test]
fn all_ready_input_is_complete() {
    let mut store = base_store();
    import_package_file(&mut store, "C-补遗01.docx", "补遗", ParseStatus::Ok, "sha-p01-a1", 0);
    import_package_file(&mut store, "E-附件-资格要求.pdf", "附件", ParseStatus::Ok, "sha-p01-e1", 0);
    let r = input_integrity(&store);
    assert!(r.complete, "全部就绪 → complete：{r:?}");
    assert!(!r.restricted(), "无确认放行件 → 非受限：{r:?}");
    assert!(r.unready.is_empty() && r.waived_unready.is_empty());
    // 起草面畅通（无 UnreadyInput / MissingAddendum 阻塞）
    let rd = check_draft_readiness(&store, "SEC-0001");
    assert!(
        unready_input_blocker(&rd).is_none(),
        "全就绪不应报 UnreadyInput：{:?}",
        rd.blockers
    );
}

// ---------------------------------------------------------------- ② 主件不就绪 → 阻塞（核心缺口）

/// P0-1 的核心事故形态：主件解析失败（CID 乱码/加密），但附件里「已提取出
/// 一部分需求」——旧现状放行，P0-1 起阻塞。
#[test]
fn failed_main_tender_file_blocks() {
    let mut store = DomainStore::new();
    store.create_project("主件坏", "ZFCG-2026-P01-002", "明标", "p01-key-2").unwrap();
    // 主件：解析失败（真实事故面：GBK-EUC 编码无映射 → 解析器明确报错）
    let (fid_main, _) = store
        .upsert_file(
            "A-招标文件.pdf", "招标文件", "sha-p01-bad-main", 0, 0, 0, 0,
            ParseStatus::Failed, "pdf 两条路输出均为疑似 CJK 乱码（GBK-EUC-H 无映射）",
        )
        .unwrap();
    // 附件正常（「已提取出一部分需求」的来源——不构成放行理由）
    import_package_file(&mut store, "E-附件-技术规范.pdf", "附件", ParseStatus::Ok, "sha-p01-e-ok", 0);
    store.set_requirements(vec![Requirement {
        id: "REQ-0001".into(),
        category: "技术参数".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "E-附件-技术规范.pdf".into(),
            kind: "para".into(),
            locator: "1.1".into(),
            excerpt: "附件条款摘录".into(),
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "E-附件-技术规范.pdf".into(),
    }]).unwrap();
    store.set_outline(vec![OutlineNode {
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
    }]).unwrap();
    store.build_matrix().unwrap();

    // 判据面：不完整 + 逐件点名主件
    let r = input_integrity(&store);
    assert!(!r.complete, "主件 FAILED → 不完整：{r:?}");
    assert_eq!(r.unready.len(), 1, "只有主件不就绪：{r:?}");
    assert_eq!(r.unready[0].file_id, fid_main);
    assert_eq!(r.unready[0].readiness, AddendumReadiness::Failed);

    // 阻塞面：UnreadyInput（不是 MissingAddendum——主件不是补遗）
    let rd = check_draft_readiness(&store, "SEC-0001");
    match unready_input_blocker(&rd).expect("主件坏必须阻塞（P0-1：不得因「已提取出一部分需求」放行）") {
        Blocker::UnreadyInput { files } => {
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].name, "A-招标文件.pdf");
            assert!(files[0].reason.contains("乱码"), "要点明坏在哪：{files:?}");
        }
        other => panic!("期望 UnreadyInput，实际 {other:?}"),
    }
    // 人话面：说清「不等于输入完整」+ 指路人工出口
    let msg = unready_input_blocker(&rd).unwrap().to_message();
    assert!(msg.contains("A-招标文件.pdf"), "要点名主件：{msg}");
    assert!(msg.contains("不等于输入完整"), "要说清判据：{msg}");
    assert!(msg.contains("record_addendum_waiver"), "要指路人工出口：{msg}");
}

/// 主件是零文本扫描件（OcrRequired 且 OCR 未产出）——「内容未知」同 Failed 面。
#[test]
fn ocr_pending_main_tender_file_blocks() {
    let mut store = base_store();
    // 把基座主件换成零文本扫描形态：直接 upsert 同名（sha 不同 = 新档语义）
    store
        .upsert_file(
            "A-招标文件-扫描件.pdf", "招标文件", "sha-p01-scan-main", 40, 0, 0, 0,
            ParseStatus::OcrRequired, "无文本层扫描件",
        )
        .unwrap();
    let r = input_integrity(&store);
    assert!(!r.complete, "主件 OCR_PENDING → 不完整：{r:?}");
    assert!(
        r.unready.iter().any(|f| f.readiness == AddendumReadiness::OcrPending),
        "要按 OCR_PENDING 分级：{r:?}"
    );
    let rd = check_draft_readiness(&store, "SEC-0001");
    assert!(unready_input_blocker(&rd).is_some(), "主件零文本必须阻塞");
}

// ---------------------------------------------------------------- ③ 附件不就绪 → 阻塞（验收场景 2）

#[test]
fn failed_attachment_blocks_with_file_name() {
    let mut store = base_store();
    import_package_file(&mut store, "C-补遗01.docx", "补遗", ParseStatus::Ok, "sha-p01-a1", 0);
    let (fid_att, _) = store
        .upsert_file(
            "E-资格审查表-扫描件.pdf", "附件", "sha-p01-bad-att", 0, 0, 0, 0,
            ParseStatus::Failed, "加密文档：用户口令保护，内容不可读",
        )
        .unwrap();
    let r = input_integrity(&store);
    assert!(!r.complete, "附件 FAILED → 不完整：{r:?}");
    assert_eq!(r.unready.len(), 1, "只有附件不就绪：{r:?}");
    assert_eq!(r.unready[0].file_id, fid_att);

    let rd = check_draft_readiness(&store, "SEC-0001");
    match unready_input_blocker(&rd).expect("附件坏必须阻塞") {
        Blocker::UnreadyInput { files } => {
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].name, "E-资格审查表-扫描件.pdf");
            assert!(files[0].reason.contains("加密"), "要点明原因：{files:?}");
        }
        other => panic!("期望 UnreadyInput，实际 {other:?}"),
    }
}

/// 附件是零文本扫描件 → 同为不就绪（与 #184 补遗零文本同判：不知道 ≠ 没有）。
#[test]
fn ocr_pending_attachment_blocks() {
    let mut store = base_store();
    import_package_file(
        &mut store, "E-图纸-扫描件.pdf", "图纸", ParseStatus::OcrRequired, "sha-p01-scan-att", 0,
    );
    let rd = check_draft_readiness(&store, "SEC-0001");
    assert!(unready_input_blocker(&rd).is_some(), "附件零文本必须阻塞（验收场景 2 的 OCR_PENDING 形态）");
}

/// 反例：扫描件但 OCR 已产出内容（ocr_blocks>0）→ Ready，不阻塞。
#[test]
fn ocr_attachment_with_extracted_blocks_is_ready() {
    let mut store = base_store();
    import_package_file(
        &mut store, "E-盖章页-扫描件.pdf", "附件", ParseStatus::OcrRequired, "sha-p01-ocr-att", 12,
    );
    let r = input_integrity(&store);
    assert!(r.complete, "OCR 已产出（12 块）= Ready：{r:?}");
    let rd = check_draft_readiness(&store, "SEC-0001");
    assert!(unready_input_blocker(&rd).is_none(), "不应误伤已 OCR 件：{:?}", rd.blockers);
}

// ---------------------------------------------------------------- ④ 边界：证据源文件不在此面

#[test]
fn evidence_kind_file_is_not_input_integrity_face() {
    let mut store = base_store();
    // 证据源文件解析失败——归证据系统管（#185 的面），不进「招标包输入不完整」
    let (fid_ev, _) = store
        .upsert_file(
            "EV-03-类似业绩合同.pdf", "证据", "sha-p01-bad-ev", 0, 0, 0, 0,
            ParseStatus::Failed, "解析失败（证据源文件）",
        )
        .unwrap();
    assert!(!is_tender_package_file(
        store.files().iter().find(|f| f.id == fid_ev).unwrap()
    ));
    let r = input_integrity(&store);
    assert!(r.complete, "证据源文件不进完整性判定：{r:?}");
    let rd = check_draft_readiness(&store, "SEC-0001");
    assert!(unready_input_blocker(&rd).is_none(), "证据源文件不该报 UnreadyInput：{:?}", rd.blockers);
}

// ---------------------------------------------------------------- ⑤ 与补遗面不相交、不双报

/// 混存：坏补遗 + 坏附件 → 两条阻塞各管各的（补遗归 MissingAddendum，
/// 附件归 UnreadyInput），同一件不会同时进两个清单。
#[test]
fn bad_addendum_and_bad_attachment_report_disjoint_blockers() {
    let mut store = base_store();
    store
        .upsert_file(
            "C-补遗02.pdf", "补遗", "sha-p01-bad-add", 0, 0, 0, 0,
            ParseStatus::Failed, "ToUnicode CMap 缺失",
        )
        .unwrap();
    store
        .upsert_file(
            "E-附件-资格.pdf", "附件", "sha-p01-bad-att2", 0, 0, 0, 0,
            ParseStatus::OcrRequired, "扫描件",
        )
        .unwrap();
    let rd = check_draft_readiness(&store, "SEC-0001");
    let missing_addendum = rd
        .blockers
        .iter()
        .find(|b| matches!(b, Blocker::MissingAddendum { .. }))
        .expect("坏补遗归 MissingAddendum");
    let unready = unready_input_blocker(&rd).expect("坏附件归 UnreadyInput");
    // 不双报：附件不进补遗清单，补遗不进输入件清单
    if let Blocker::MissingAddendum { files, .. } = missing_addendum {
        assert!(files.iter().all(|f| f.name.starts_with("C-补遗")), "补遗清单只含补遗：{files:?}");
    }
    if let Blocker::UnreadyInput { files } = unready {
        assert!(files.iter().all(|f| f.name.starts_with("E-")), "输入件清单只含非补遗：{files:?}");
    }
    // 判据面看全貌（两件都算）
    let r = input_integrity(&store);
    assert_eq!(r.unready.len(), 2, "判据面覆盖全部包件（含补遗）：{r:?}");
}

// ---------------------------------------------------------------- ⑥ 受限面：确认放行 → restricted

/// 降级路径：附件坏 → 逐件人工确认 → 阻塞消失 + restricted（受限起草来源，
/// 放行 ≠ 没问题——这是 MCP 响应标 RESTRICTED 的判定依据）。
#[test]
fn waived_unready_file_turns_blocker_into_restricted() {
    let mut store = base_store();
    let fid = import_package_file(
        &mut store, "E-资格审查表.pdf", "附件", ParseStatus::Failed, "sha-p01-waived", 0,
    );
    // 确认前：阻塞
    assert!(unready_input_blocker(&check_draft_readiness(&store, "SEC-0001")).is_some());

    // 逐件人工确认（域层 API——MCP 面走 record_addendum_waiver + receipt）
    store
        .record_addendum_waiver(
            &fid,
            "已人工核对纸质原件，本表与招标文件第三章一致",
            &bid_domain::TrustedPrincipal::human("p01-lead"),
        )
        .unwrap();

    // 确认后：阻塞消失，但 restricted = true（三面可见：判据/人话/逐件）
    let rd = check_draft_readiness(&store, "SEC-0001");
    assert!(unready_input_blocker(&rd).is_none(), "确认后该件不再阻塞");
    let r = input_integrity(&store);
    assert!(r.complete, "确认后 complete（放行面成立）");
    assert!(r.restricted(), "但 restricted——本次起草基于不完整输入：{r:?}");
    assert_eq!(r.waived_unready.len(), 1);
    assert_eq!(r.waived_unready[0].file_id, fid, "逐件点名确认来源");
    let msg = r.to_message();
    assert!(msg.contains("E-资格审查表.pdf"), "人话要点名：{msg}");
    assert!(msg.contains("不完整"), "要说清受限语义：{msg}");
}

/// 全部就绪 + 无确认 → restricted = false（正向：不误伤完整输入）。
#[test]
fn complete_without_waivers_is_not_restricted() {
    let store = base_store();
    let r = input_integrity(&store);
    assert!(r.complete);
    assert!(!r.restricted(), "完整输入不得标受限——只有确认放行才受限");
}

// ---------------------------------------------------------------- ⑦ 全局面：章节不存在也报

/// UnreadyInput 是项目级阻塞（与 MissingAddendum 同位）——查一个不存在的
/// 章节也不能显得风平浪静。
#[test]
fn unready_input_reported_even_for_unknown_section() {
    let mut store = base_store();
    import_package_file(&mut store, "E-附件-坏.pdf", "附件", ParseStatus::Failed, "sha-p01-ghost", 0);
    let rd = check_draft_readiness(&store, "SEC-9999");
    assert!(
        unready_input_blocker(&rd).is_some(),
        "全局阻塞先于章节判定：{:?}",
        rd.blockers
    );
}

// （夹具根引用保留给未来真件导入面——当前判定面测试全部内存态。）
#[allow(dead_code)]
fn _fixtures_guard() -> PathBuf {
    fixtures()
}
