//! validate_state_safety.rs —— #243 校验状态安全与恢复（服务层回归）。
//!
//! 缺陷形态（main @ 6105beb，repro_243 探针实测）：
//! 起草未完成时 `validate_document(None)` 无条件推进 Drafting→Validating
//! （校验失败也推进），而 VALIDATING 态禁 draft_response_section → 起草
//! 死锁，用户只能杀进程手改 db.json。
//!
//! #243 服务层语义（见 `BidService::validate_document` 文档）：
//! - 前置闸：scope=all 要求起草完成，否则 `E_DRAFT_INCOMPLETE`（missing
//!   清单）零副作用拒；scope=sections（章节级检查）不受此闸；
//! - 仅「scope=all 且 ok=true」才推进 VALIDATING；失败留在原态可修订重试；
//! - `recover_validating_state`：历史异常态（VALIDATING+缺章）的受支持
//!   恢复（域层三重约束 + 审计账），恢复后 checkpoint（无需手改 db.json）。
//!
//! 夹具纪律：域状态全部走真实状态机与真实服务调用（建纲/起草/绑定），
//! 历史异常态经合法前向链构造（状态机本身不校验草稿完成度——这正是
//! 旧缺陷能发生的通道）；不 force 字段。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::{OutlineNode, OutlineSpec, SectionStatus};
use bid_domain::principal::TrustedPrincipal;
use bid_domain::{Anchor, ParseStatus, ProjectStatus, Requirement, ToolError};
use bid_tools::draft::MockGenerator;
use bid_tools::service::BidService;
use std::collections::HashSet;

// ---------------------------------------------------------------- 夹具

fn temp_ws(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bid-243-svc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn mk_req(id: &str, category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第二章/{id}"),
            excerpt: excerpt.into(),
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

/// 两叶夹具（资格组叶 + 格式组叶）：
/// - `draft_all=true`：全叶起草（DRAFTING、全稿完成——#241 裁定后资格行
///   随正文实写翻 RESPONDED，本夹具不再天然制造 E_QUAL_UNMET，失败面
///   构造见 `svc_unmapped_qual`）；
/// - `draft_all=false`：只起草格式叶（DRAFTING、缺资格叶——未完成态）。
///
/// 资格 REQ-0001 只登记台账证据（不绑——readiness 闸放行）；格式
/// REQ-0002 零绑定可起草。
fn svc_two_leaves(tag: &str, draft_all: bool) -> (BidService, Vec<String>) {
    let dir = temp_ws(tag);
    let mut svc = BidService::open(&dir).unwrap();
    let _ = svc
        .create_project("243 服务层", "ZFCG-243", "明标", &format!("ext-243-{tag}"))
        .unwrap();
    svc.store_mut()
        .upsert_file("A-招标文件.docx", "招标文件", "sha-243-a", 10, 1, 50, 0, ParseStatus::Ok, "")
        .unwrap();
    svc.store_mut()
        .upsert_file("C-补遗文件01.docx", "补遗", "sha-243-c", 5, 0, 20, 0, ParseStatus::Ok, "")
        .unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "具备水利水电工程施工总承包贰级资质"),
        mk_req("REQ-0002", "格式要求", "正文用仿宋三号字"),
    ])
    .unwrap();
    svc.prepare_outline_and_matrix(OutlineSpec { target_total_chars: 2_000, max_reqs_per_leaf: 8 })
        .unwrap();
    // 台账资质（登记不绑——清 MissingQualification 闸，资格叶可零绑定起草；
    // 矩阵行保持未响应 → 全稿校验报 E_QUAL_UNMET）
    svc.store_mut()
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "水利水电工程施工总承包贰级资质证书".into(),
            source_file: "EV-ledger.pdf".into(),
            source_sha256: "sha-243-lg".into(),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 7,
                text_fingerprint: "fnv64:lg".into(),
            },
            fingerprint: "fnv64:lg".into(),
            valid_from: None,
            valid_until: None,
            subject_name: "某公司".into(),
            scope: "水利水电".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: Default::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        })
        .unwrap();
    let fmt_leaf = leaf_of_req(&svc, "REQ-0002");
    svc.draft_section(&fmt_leaf, None, None, None, &mut MockGenerator).unwrap();
    if draft_all {
        let qual_leaf = leaf_of_req(&svc, "REQ-0001");
        svc.draft_section(&qual_leaf, None, None, None, &mut MockGenerator).unwrap();
    }
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Drafting);
    let leaves = leaf_ids(&svc);
    (svc, leaves)
}

fn leaf_ids(svc: &BidService) -> Vec<String> {
    let parents: HashSet<String> =
        svc.store().sections().iter().filter_map(|n| n.parent_id.clone()).collect();
    svc.store().sections().iter().filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect()
}

/// 定位承载指定 REQ 的**叶**（结构节点聚合了子并集，不能起草）。
fn leaf_of_req(svc: &BidService, rid: &str) -> String {
    let parents: HashSet<String> =
        svc.store().sections().iter().filter_map(|n| n.parent_id.clone()).collect();
    svc.store()
        .sections()
        .iter()
        .find(|n| {
            !parents.contains(&n.id) && n.requirement_ids.iter().any(|r| r == rid)
        })
        .map(|n| n.id.clone())
        .unwrap_or_else(|| panic!("REQ {rid} 无承载叶"))
}

// ---------------------------------------------------------------- 1. 前置闸

#[test]
fn validate_all_on_incomplete_draft_rejects_with_missing_list_zero_side_effect() {
    let (mut svc, _leaves) = svc_two_leaves("guard", false);
    let status_before = svc.store().project().unwrap().status;
    let version_before = svc.store().content_version();
    let reviews_before = svc.store().reviews().to_vec();
    let qual_leaf = leaf_of_req(&svc, "REQ-0001");
    let missing_expect = vec![qual_leaf.clone()];

    // 全稿校验 → E_DRAFT_INCOMPLETE + missing 清单（可操作面）
    let e = svc.validate_document(None).unwrap_err();
    match &e {
        ToolError::EDraftIncomplete { missing } => {
            assert_eq!(*missing, missing_expect, "missing = 未完成叶清单");
        }
        other => panic!("预期 E_DRAFT_INCOMPLETE，实际 {other:?}"),
    }
    assert_eq!(e.code(), "E_DRAFT_INCOMPLETE");
    // 零副作用：状态/版本/审核记录均不变
    assert_eq!(svc.store().project().unwrap().status, status_before);
    assert_eq!(svc.store().content_version(), version_before);
    assert_eq!(svc.store().reviews(), reviews_before.as_slice());
    // 起草面未受影响（缺章态下 draft 仍可用——与旧缺陷的差异面）
    assert!(svc.store().guard_tool("draft_response_section").is_ok());

    // 重复请求同样零副作用（重试安全——不会落在无法恢复的状态）
    let e2 = svc.validate_document(None).unwrap_err();
    assert_eq!(e2.code(), "E_DRAFT_INCOMPLETE");
    assert_eq!(svc.store().content_version(), version_before, "重试仍零副作用");
}

// ---------------------------------------------------------------- 2. 章节级检查分面

#[test]
fn validate_sections_scope_is_the_drafting_time_check() {
    let (mut svc, _leaves) = svc_two_leaves("sections-scope", false);
    let fmt_leaf = leaf_of_req(&svc, "REQ-0002");
    let version_before = svc.store().content_version();
    let status_before = svc.store().project().unwrap().status;

    // 起草未完成（资格叶 Pending）：章节级检查（scope=sections）不受前置闸
    let report = svc.validate_document(Some(&[fmt_leaf.clone()])).unwrap();
    assert_eq!(report.sections_checked, 1, "只校验指定叶");
    assert!(!report.findings.iter().any(|f| f.rule_code == "E_SECTION_EMPTY" && f.section_id.as_deref() == Some(fmt_leaf.as_str())),
        "已起草叶不应报空章");
    // 纯读：状态/版本不变（章节级检查不是全稿最终校验，不推进）
    assert_eq!(svc.store().project().unwrap().status, status_before);
    assert_eq!(svc.store().content_version(), version_before);
}

// ---------------------------------------------------------------- 3. 失败可修订重试

/// 失败面夹具（#241 裁定后的 E_QUAL_UNMET 构造）：REQ-0003（资格条件）
/// **不落任何章节**（覆盖缺口形态）——全叶起草后仍 NOT_RESPONDED，
/// 规则 2 报 E_QUAL_UNMET。旧构造（资格叶已起草但无证据 → 未响应）
/// 由 #241 裁定废止：正文实写即 RESPONDED（文本事实），证据缺口另走
/// evidence_missing/提交就绪面，不再出现在状态面。
fn svc_unmapped_qual(tag: &str) -> BidService {
    let dir = temp_ws(tag);
    let mut svc = BidService::open(&dir).unwrap();
    svc.create_project("243 服务层", "ZFCG-243", "明标", &format!("ext-243-{tag}")).unwrap();
    for (name, kind, sha) in [
        ("A-招标文件.docx", "招标文件", "sha-243-a"),
        ("C-补遗文件01.docx", "补遗", "sha-243-c"),
    ] {
        svc.store_mut().upsert_file(name, kind, sha, 10, 1, 50, 0, ParseStatus::Ok, "").unwrap();
    }
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "具备水利水电工程施工总承包贰级资质"),
        mk_req("REQ-0002", "格式要求", "正文用仿宋三号字"),
        mk_req("REQ-0003", "资格条件", "项目经理须持有效安全生产考核合格证"),
    ])
    .unwrap();
    // 手工建纲：只映射 REQ-0001/0002（REQ-0003 留空——覆盖缺口）
    let mk_leaf = |id: &str, title: &str, order: u32, reqs: &[&str]| OutlineNode {
        id: id.into(), parent_id: None, title: title.into(), level: 1, order,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars: 800, status: SectionStatus::Pending, dirty: false,
        content: None, content_attempt: 0,
    };
    svc.store_mut().set_outline(vec![
        mk_leaf("SEC-0001", "资格响应", 1, &["REQ-0001"]),
        mk_leaf("SEC-0002", "格式响应", 2, &["REQ-0002"]),
    ]).unwrap();
    svc.store_mut().build_matrix().unwrap();
    // 台账资质（登记不绑——清 MissingQualification 闸，资格叶可零绑定起草）
    svc.store_mut()
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "水利水电工程施工总承包贰级资质证书".into(),
            source_file: "EV-ledger.pdf".into(),
            source_sha256: "sha-243-lg".into(),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 7,
                text_fingerprint: "fnv64:lg".into(),
            },
            fingerprint: "fnv64:lg".into(),
            valid_from: None,
            valid_until: None,
            subject_name: "某公司".into(),
            scope: "水利水电".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: Default::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        })
        .unwrap();
    // 全叶起草（全稿完成——E_DRAFT_INCOMPLETE 前置闸不触发）
    for rid in ["REQ-0001", "REQ-0002"] {
        let leaf = leaf_of_req(&svc, rid);
        svc.draft_section(&leaf, None, None, None, &mut MockGenerator).unwrap();
    }
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Drafting);
    svc
}

#[test]
fn validate_failure_stays_in_drafting_then_retry_passes_and_advances() {
    let mut svc = svc_unmapped_qual("retry");

    // 第一轮：全稿完成但 REQ-0003（资格条件，未落章节）未响应 → ok=false，
    // 留在 DRAFTING（#241 裁定后资格叶正文实写已翻 RESPONDED——失败源
    // 改为覆盖缺口形态的未映射条款，旧「无证据即未响应」口径废止）
    let version_before = svc.store().content_version();
    let report = svc.validate_document(None).unwrap();
    assert!(!report.ok, "E_QUAL_UNMET 在场（未映射资格行未响应）");
    assert!(report.findings.iter().any(|f| f.rule_code == "E_QUAL_UNMET"));
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Drafting, "失败不推进（#243 核心）");
    assert_eq!(svc.store().content_version(), version_before, "失败零副作用（版本不变）");
    // 可修订：draft_response_section 仍可用（修订通道未被锁死）
    assert!(svc.store().guard_tool("draft_response_section").is_ok());

    // 修订：人工确认响应成立（#182：Human receipt 面——覆写 RESPONDED+说明，
    // 覆写后重算矩阵，未响应清零）。旧路径（补绑证据即清）对未映射条款
    // 只到 PARTIAL（证据无章节落位）——响应成立需人工确认或补落章节。
    svc.store_mut()
        .set_matrix_override(
            "REQ-0003".into(),
            bid_domain::matrix::ResponseStatus::Responded,
            Some("安全生产考核合格证随投标文件正本提交（人工确认响应成立）".into()),
            &TrustedPrincipal::human("243-retry-lead"),
        )
        .unwrap();
    let (_rows, blocking) = svc.store_mut().build_matrix().unwrap();
    assert_eq!(blocking, 0, "覆写后 blocking 清零");

    // 第二轮：ok=true → 推进 VALIDATING（复审语义——成功面）
    let version_mid = svc.store().content_version();
    let report2 = svc.validate_document(None).unwrap();
    assert!(
        report2.ok,
        "修订后全绿: {:?}",
        report2.findings.iter().map(|f| f.rule_code.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Validating, "成功推进");
    assert_eq!(svc.store().content_version(), version_mid + 1, "推进即版本 +1");

    // 重复校验（VALIDATING 态重验）：幂等 no-op，不再推进
    let report3 = svc.validate_document(None).unwrap();
    assert_eq!(report3.counts, report2.counts, "同输入复校结果全等（纯读诊断）");
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Validating);
    assert_eq!(svc.store().content_version(), version_mid + 1, "重验不推进（无变迁不落版本）");
}

// ---------------------------------------------------------------- 4. 历史异常态恢复 + 持久化

#[test]
fn recover_historical_validating_state_persists_across_restart() {
    let (mut svc, _leaves) = svc_two_leaves("recover", false);
    // 构造历史异常态：缺章 + 合法链推进 VALIDATING（Drafting→Validating
    // 是 next_allowed 内的合法转换——正是旧版 validate 无条件做的事，
    // 状态机本身不校验草稿完成度）
    svc.store_mut().advance_project_status(ProjectStatus::Validating).unwrap();
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Validating);
    // 死锁面：VALIDATING 禁 draft_response_section（历史事故形态）
    assert!(svc.store().guard_tool("draft_response_section").is_err());

    // 受支持恢复（服务面——域层三重约束 + 审计 + checkpoint）
    let version_before = svc.store().content_version();
    let dir = temp_ws("recover"); // svc 的 ws（重建同 tag 路径即同目录）
    let _ = dir;
    let rec = svc
        .recover_validating_state("历史异常态：旧版校验在缺章时误推进 VALIDATING")
        .unwrap();
    assert_eq!(rec.from, ProjectStatus::Validating);
    assert_eq!(rec.to, ProjectStatus::Drafting);
    assert!(!rec.missing_sections.is_empty(), "缺章快照入审计");
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::Drafting, "恢复后起草面解锁");
    assert_eq!(svc.store().content_version(), version_before + 1);
    assert!(svc.store().guard_tool("draft_response_section").is_ok(), "起草面恢复");

    // 持久化重启：db.json 已带恢复后状态与审计账
    let ws = svc.workspace_dir().expect("工作区在场（BidService::open）");
    drop(svc);
    let mut svc2 = BidService::open(&ws).unwrap();
    assert_eq!(svc2.store().project().unwrap().status, ProjectStatus::Drafting, "恢复结果跨重启保持");
    assert_eq!(svc2.store().state_recoveries().len(), 1, "审计账跨重启保持");
    assert_eq!(
        svc2.store().state_recoveries()[0].reason,
        "历史异常态：旧版校验在缺章时误推进 VALIDATING"
    );
    assert!(svc2.store().guard_tool("draft_response_section").is_ok());

    // 恢复后可续跑：补起草剩余叶 + 补绑证据（矩阵未响应清零）→ 全稿校验通过
    let qual_leaf = leaf_of_req(&svc2, "REQ-0001");
    svc2.draft_section(&qual_leaf, None, None, None, &mut MockGenerator).unwrap();
    let eid = svc2.store().evidences()[0].id.clone();
    svc2.store_mut().bind_evidence(&eid, "REQ-0001", "2030-12-31").unwrap();
    let (_rows, blocking) = svc2.store_mut().build_matrix().unwrap();
    assert_eq!(blocking, 0, "绑定后 blocking 清零");
    let report = svc2.validate_document(None).unwrap();
    assert!(
        report.ok,
        "补齐后全绿: {:?}",
        report.findings.iter().map(|f| f.rule_code.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(svc2.store().project().unwrap().status, ProjectStatus::Validating);
}
