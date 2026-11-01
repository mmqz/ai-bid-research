//! readiness_wire.rs —— issue #110 任务 1：缺失回报接线（draft_section 前置闸）。
//!
//! 与 readiness_check.rs（本地团队交付的判定单元）的分工：那边证「判据对不对」，
//! 这边证「判据真的接进了起草通路」——issue 验收线逐条落测试：
//!
//! ```text
//! [x] 没有业绩证据的章节 → draft_section 返回 E_UNPROCESSABLE（不是硬生成）
//! [x] 错误信息人类可读（不是 enum 名）
//! [x] IncompleteTender 只 warning 不阻塞
//! [x] 现有测试不回归（585 基线 → 加上本文件新增）
//! ```
//!
//! 外加两条接线的工程语义（issue 伪代码第 2 步 + B4 交互）：
//! - 缺失回报（warning 面）注入生成指令 → 生成器知道要留【待补充】占位；
//! - 迭代起草（draft_section_iterative）先闸后循环——阻塞章节一轮 LLM 都不烧。
//!
//! 夹具纪律：域状态全部走真实状态机（create_project → upsert_file →
//! set_requirements → set_outline → build_matrix → register/bind_evidence），
//! 不 force 字段——与生产路径同构。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineNode;
use bid_domain::{Anchor, ParseStatus, Requirement, ToolError};
use bid_tools::draft::{GeneratedDraft, SectionDraftInput, SectionGenerator};
use bid_tools::service::BidService;

// ---------------------------------------------------------------- 夹具

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-readiness-wire-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn req(id: &str, category: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第三章/{id}"),
            excerpt: format!("{id} 的条款原文摘录（逐条唯一）"),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

fn leaf(id: &str, title: &str, reqs: &[&str]) -> OutlineNode {
    OutlineNode {
        id: id.into(),
        parent_id: None,
        title: title.into(),
        level: 1,
        order: 1,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars: 600,
        status: bid_domain::outline::SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }
}

fn register_ev(svc: &mut BidService, kind: EvidenceKind, title: &str) -> String {
    let (eid, _) = svc
        .store_mut()
        .register_evidence(EvidenceSpec {
            kind,
            title: title.into(),
            source_file: format!("{title}.pdf"),
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
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: Default::default(),
        })
        .unwrap();
    eid
}

/// 基座：一份招标文件（`addendum=false` 时无补遗——IncompleteTender 警告面
/// 在场）+ 指定章节树与条款集，矩阵已建（MATRIX_READY）。
fn svc_base(
    tag: &str,
    addendum: bool,
    sections: Vec<OutlineNode>,
    reqs: Vec<Requirement>,
) -> BidService {
    let mut svc = BidService::open(tmp_dir(tag)).unwrap();
    svc.create_project("缺失回报接线测试", "C-RW", "明标", tag)
        .unwrap();
    svc.store_mut()
        .upsert_file(
            "A-招标文件.docx",
            "招标文件",
            "sha-rw",
            120,
            8,
            900,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
    if addendum {
        svc.store_mut()
            .upsert_file(
                "C-补遗文件01.docx",
                "补遗",
                "sha-rw-add",
                5,
                0,
                20,
                0,
                ParseStatus::Ok,
                "",
            )
            .unwrap();
    }
    svc.set_requirements(reqs).unwrap();
    svc.store_mut().set_outline(sections).unwrap();
    svc.store_mut().build_matrix().unwrap();
    svc
}

/// 取 E_UNPROCESSABLE 的 reason（typed 断言——不靠 Display 形态）。
fn unprocessable_reason(err: &ToolError) -> &str {
    match err {
        ToolError::EUnprocessable { reason } => reason,
        other => panic!("预期 E_UNPROCESSABLE，实际 {other:?}"),
    }
}

/// 记录型生成器（不生成只记账——接线面测试看的是「生成器收到了什么/被
/// 调了几次」，不是内容质量）。
struct Recorder {
    calls: usize,
    instructions: Vec<Option<String>>,
}

impl Recorder {
    fn new() -> Self {
        Recorder {
            calls: 0,
            instructions: Vec::new(),
        }
    }
}

impl SectionGenerator for Recorder {
    fn generate(&mut self, input: &SectionDraftInput) -> Result<GeneratedDraft, String> {
        self.calls += 1;
        self.instructions.push(input.instructions.clone());
        Ok(GeneratedDraft {
            content: format!(
                "# {}\n\n针对 [{}] 的响应说明。",
                input.title, input.requirements[0].id
            ),
        })
    }
}

// ---------------------------------------------------------------- ① 缺业绩 → 拒生成

/// issue 验收第 1 条（真实场景）：没有业绩证据的章节 → E_UNPROCESSABLE，
/// **不是**硬生成。外加副作用面：拒生成必须零副作用（不写正文、不推进
/// 状态、不落盘、不 bump 版本）——「拒」要是干净的拒。
#[test]
fn no_performance_evidence_refuses_instead_of_fabricating() {
    let mut svc = svc_base(
        "perf-block",
        false,
        vec![leaf("SEC-0001", "类似项目业绩", &["REQ-0001"])],
        vec![req("REQ-0001", "评分项")],
    );
    let version_before = svc.store().content_version();

    let mut gen = Recorder::new();
    let err = svc
        .draft_section("SEC-0001", None, None, None, &mut gen)
        .unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE", "缺业绩必须拒生成: {err:?}");

    // 人类可读（issue 验收第 2 条）：说清缺什么、缺多少、后果是什么——
    // 不是 enum 名。
    let reason = unprocessable_reason(&err);
    assert!(reason.contains("章节 SEC-0001"), "要点名章节: {reason}");
    assert!(reason.contains("业绩证据不足"), "{reason}");
    assert!(
        reason.contains("需要 3 个类似项目业绩，现有 0 个"),
        "{reason}"
    );
    assert!(reason.contains("废标"), "要说清后果: {reason}");
    for enum_name in ["MissingPerformance", "Blocker", "ready"] {
        assert!(
            !reason.contains(enum_name),
            "不得泄漏枚举名 {enum_name}: {reason}"
        );
    }

    // 生成器一次都没被调（拒生成 ≠ 生成后丢弃）
    assert_eq!(gen.calls, 0, "阻塞章节不得触发生成器");
    // 零副作用：不写正文 / 状态停在 PENDING / 项目不进 DRAFTING / 无 S-2 落盘 / 版本不动
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == "SEC-0001")
        .unwrap();
    assert!(node.content.is_none(), "拒生成不得写正文");
    assert_eq!(node.status.as_str(), "PENDING");
    assert_eq!(
        svc.store().project().unwrap().status.as_str(),
        "MATRIX_READY"
    );
    assert_eq!(
        svc.store().content_version(),
        version_before,
        "拒生成不得 bump 版本"
    );
    let ws = svc.workspace_dir().unwrap();
    assert!(
        !ws.join("chapters").join("SEC-0001.json").exists(),
        "拒生成不得落工作区"
    );
}

/// 业绩补齐到门槛（3 条可用）→ 同一章节放行（拒与放是同一判据的两面）。
#[test]
fn performance_evidence_cleared_unblocks_drafting() {
    let mut svc = svc_base(
        "perf-clear",
        // #127 代理 C：L3 后无补遗是全局阻塞（#118 遗漏适配——本测只验业绩面，
        // 补遗在场才能隔离）
        true,
        vec![leaf("SEC-0001", "类似项目业绩", &["REQ-0001"])],
        vec![req("REQ-0001", "评分项")],
    );
    for i in 1..=3 {
        register_ev(
            &mut svc,
            EvidenceKind::Performance,
            &format!("类似项目合同 {i}"),
        );
    }
    let out = svc
        .draft_section(
            "SEC-0001",
            None,
            None,
            None,
            &mut bid_tools::draft::MockGenerator,
        )
        .unwrap();
    assert_eq!(out.section_id, "SEC-0001");
    assert_eq!(out.attempt, 1, "放行后正常提交");
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == "SEC-0001")
        .unwrap();
    assert_eq!(node.status.as_str(), "DRAFTED");
}

// ---------------------------------------------------------------- ② 其余阻塞面

/// 缺资质（资格/资质章 + 台账无可用资质证据）→ 拒（无证硬写 = 虚假响应）。
#[test]
fn missing_qualification_refuses() {
    let mut svc = svc_base(
        "qual-block",
        false,
        vec![leaf("SEC-0001", "资格及资质证明文件", &["REQ-0001"])],
        vec![req("REQ-0001", "资格条件")],
    );
    let err = svc
        .draft_section(
            "SEC-0001",
            None,
            None,
            None,
            &mut bid_tools::draft::MockGenerator,
        )
        .unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE");
    let reason = unprocessable_reason(&err);
    assert!(
        reason.contains("缺资格及资质证明文件"),
        "要点名缺哪项: {reason}"
    );
    assert!(reason.contains("虚假响应"), "{reason}");
    assert!(!reason.contains("MissingQualification"), "{reason}");
}

/// 否决项未响应 → 拒（否决项漏响应 = 直接废标——六种 Blocker 里后果最重的一个）。
#[test]
fn uncovered_veto_refuses() {
    let mut svc = svc_base(
        "veto-block",
        // #127 代理 C：L3 后无补遗是全局阻塞（#118 遗漏适配——本测只验否决面）
        true,
        vec![leaf("SEC-0001", "响应承诺", &["REQ-0001"])],
        vec![req("REQ-0001", "否决项")],
    );
    let err = svc
        .draft_section(
            "SEC-0001",
            None,
            None,
            None,
            &mut bid_tools::draft::MockGenerator,
        )
        .unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE");
    let reason = unprocessable_reason(&err);
    assert!(
        reason.contains("否决项尚未响应") && reason.contains("REQ-0001"),
        "{reason}"
    );
    assert!(reason.contains("直接废标"), "{reason}");
    // 绑定证据 + 重建矩阵（RESPONDED）→ 放行（矩阵反映真实响应态）
    let eid = register_ev(&mut svc, EvidenceKind::Certificate, "安全生产许可证");
    svc.store_mut()
        .bind_evidence(&eid, "REQ-0001", "2026-12-31")
        .unwrap();
    svc.store_mut().build_matrix().unwrap();
    let out = svc
        .draft_section(
            "SEC-0001",
            None,
            None,
            None,
            &mut bid_tools::draft::MockGenerator,
        )
        .unwrap();
    assert_eq!(out.attempt, 1, "响应态补齐后放行");
}

/// 技术参数无证据佐证 → 拒（硬写「完全响应」= 中标后违约）。
#[test]
fn missing_tech_evidence_refuses() {
    let mut svc = svc_base(
        "tech-block",
        false,
        vec![leaf("SEC-0001", "技术方案", &["REQ-0001", "REQ-0002"])],
        vec![req("REQ-0001", "技术参数"), req("REQ-0002", "技术参数")],
    );
    let err = svc
        .draft_section(
            "SEC-0001",
            None,
            None,
            None,
            &mut bid_tools::draft::MockGenerator,
        )
        .unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE");
    let reason = unprocessable_reason(&err);
    assert!(
        reason.contains("2 条技术参数条款") && reason.contains("REQ-0002"),
        "{reason}"
    );
    assert!(reason.contains("中标后违约"), "{reason}");
}

// ---------------------------------------------------------------- ③ IncompleteTender 只警告

/// 招标包结构可疑（多份正本无法确定）→ **不阻塞**，起草照常，warning 如实
/// 上报（Agent 感知面）。
///
/// #127 代理 C 注：原版验「无补遗 → 警告」——L3（#118）后无补遗升级为
/// **阻塞**，原语义已死；本测改为 L3 之后的警告面分界：**多份招标文件
/// 正本不明**仍是警告不阻塞（补遗在场，隔离 MISSING_ADDENDUM）。
#[test]
fn incomplete_tender_warns_but_does_not_block() {
    let mut svc = svc_base(
        "tender-warn",
        true, // 补遗在场（隔离全局阻塞面）
        vec![leaf("SEC-0001", "商务响应", &["REQ-0001"])],
        vec![req("REQ-0001", "商务要求")],
    );
    // 多份「招标文件」类文件 → IncompleteTender 警告（无法确定正本）
    svc.store_mut()
        .upsert_file(
            "B-招标文件副本.docx",
            "招标文件",
            "sha-rw-dup",
            120,
            8,
            900,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
    let out = svc
        .draft_section(
            "SEC-0001",
            None,
            None,
            None,
            &mut bid_tools::draft::MockGenerator,
        )
        .unwrap();
    assert_eq!(out.attempt, 1, "IncompleteTender 不阻塞——正文照常提交");
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == "SEC-0001")
        .unwrap();
    assert_eq!(node.status.as_str(), "DRAFTED");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("缺失回报（不阻塞）") && w.contains("正本")),
        "警告须如实上报（含多正本说明）: {:?}",
        out.warnings
    );
    assert!(
        out.warnings.iter().all(|w| !w.contains("IncompleteTender")),
        "警告面同样不得泄漏枚举名: {:?}",
        out.warnings
    );
}

// ---------------------------------------------------------------- ④ 缺失回报 → 生成指令

/// issue 伪代码第 2 步：「可降级的 warning 塞进 input，让生成器知道要留
/// 【待补充】」。验证：警告在场 → 生成器收到的 instructions 携带缺失回报
/// 说明（用户原指令保留为前缀）；契约面 outcome.instructions 仍是用户的
/// 原话（echo 语义）；警告清零 → 指令原样（不添噪声）。
#[test]
fn missing_report_note_reaches_generator_and_echo_stays_users() {
    // 有警告（多份正本不明）：note 注入 + 用户指令保留
    // （#127 代理 C：原用「无补遗」作警告源——L3 后它是阻塞，改多正本面）
    let mut svc = svc_base(
        "note-hit",
        true,
        vec![leaf("SEC-0001", "商务响应", &["REQ-0001"])],
        vec![req("REQ-0001", "商务要求")],
    );
    svc.store_mut()
        .upsert_file(
            "B-招标文件副本.docx",
            "招标文件",
            "sha-rw-dup",
            120,
            8,
            900,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
    let mut gen = Recorder::new();
    let out = svc
        .draft_section("SEC-0001", None, None, Some("按招标文件逐项响应"), &mut gen)
        .unwrap();
    assert_eq!(gen.calls, 1);
    let got = gen.instructions[0]
        .as_deref()
        .expect("警告在场 → 指令必须在场");
    assert!(
        got.starts_with("按招标文件逐项响应"),
        "用户原指令保留为前缀: {got}"
    );
    assert!(got.contains("缺失回报"), "{got}");
    assert!(got.contains("不得编造"), "要给出占位纪律: {got}");
    assert!(got.contains("正本"), "警告明细要在场: {got}");
    // 契约面 echo = 用户原话（不是合并稿）
    assert_eq!(out.instructions.as_deref(), Some("按招标文件逐项响应"));

    // 无警告（补遗已导入）：指令原样透传——不添噪声
    let mut svc2 = svc_base(
        "note-clean",
        true,
        vec![leaf("SEC-0001", "商务响应", &["REQ-0001"])],
        vec![req("REQ-0001", "商务要求")],
    );
    let mut gen2 = Recorder::new();
    let out2 = svc2
        .draft_section(
            "SEC-0001",
            None,
            None,
            Some("按招标文件逐项响应"),
            &mut gen2,
        )
        .unwrap();
    assert_eq!(
        gen2.instructions[0].as_deref(),
        Some("按招标文件逐项响应"),
        "零警告零注入"
    );
    assert!(
        out2.warnings.iter().all(|w| !w.contains("缺失回报")),
        "素材齐备不应报缺失: {:?}",
        out2.warnings
    );
}

// ---------------------------------------------------------------- ⑤ 迭代起草（B4）与闸的交互

/// 阻塞章节 → draft_section_iterative 在循环**之前**拒——LLM 一轮都不烧
/// （等回放提交才拒 = 白烧 max_rounds 轮编造，违背缺失回报的初衷）。
#[test]
fn iterative_gate_burns_zero_llm_rounds() {
    let mut svc = svc_base(
        "iter-block",
        false,
        vec![leaf("SEC-0001", "类似项目业绩", &["REQ-0001"])],
        vec![req("REQ-0001", "评分项")],
    );
    let mut gen = Recorder::new();
    let err = svc
        .draft_section_iterative("SEC-0001", None, None, None, &mut gen, 3, 0.85)
        .unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE");
    assert!(unprocessable_reason(&err).contains("业绩证据不足"));
    assert_eq!(gen.calls, 0, "阻塞章节迭代循环一轮都不烧: {} 次", gen.calls);
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == "SEC-0001")
        .unwrap();
    assert!(node.content.is_none(), "零副作用");
}

/// 就绪但有警告 → 迭代循环首轮起生成器就收到缺失回报（占位纪律贯穿循环，
/// 不是只有单发面才有）；说明文字不得含全角引号（会被改写提示的失分项
/// 提取面误吞——见 with_missing_report 的措辞纪律）。
#[test]
fn iterative_round_one_carries_missing_report_note() {
    let mut svc = svc_base(
        "iter-note",
        // #127 代理 C：原用「无补遗」作警告源——L3 后它是阻塞，改多正本面
        true,
        vec![leaf("SEC-0001", "商务响应", &["REQ-0001"])],
        vec![req("REQ-0001", "商务要求")],
    );
    svc.store_mut()
        .upsert_file(
            "B-招标文件副本.docx",
            "招标文件",
            "sha-rw-dup",
            120,
            8,
            900,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
    let mut gen = Recorder::new();
    let out = svc
        .draft_section_iterative("SEC-0001", None, None, None, &mut gen, 1, 0.85)
        .unwrap();
    assert_eq!(gen.calls, 1, "max_rounds=1 → 恰一轮");
    let round1 = gen.instructions[0]
        .as_deref()
        .expect("警告在场 → 首轮指令在场");
    assert!(
        round1.contains("缺失回报") && round1.contains("正本"),
        "{round1}"
    );
    // 措辞纪律只约束**说明文字**（首行——会被改写提示的失分项提取面
    // 扫描）；警告明细是用户数据（多正本警告自带「招标文件」引号），
    // 不适用该纪律。
    let note_header = round1.split('\n').next().unwrap_or("");
    assert!(
        !note_header.contains('「'),
        "说明文字不得混入全角引号（失分项提取面）: {note_header}"
    );
    // 提交面照常（回放走 draft_section 既有路径）
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == "SEC-0001")
        .unwrap();
    assert_eq!(node.status.as_str(), "DRAFTED");
    assert!(
        out.draft
            .warnings
            .iter()
            .any(|w| w.contains("缺失回报（不阻塞）")),
        "警告经回放并入 DraftOutcome.warnings: {:?}",
        out.draft.warnings
    );
}
