//! submission_readiness.rs —— #241 裁定：提交就绪面（Export ≠ Submission Readiness）。
//!
//! 裁定（Lead 2026-11-05，issue #241）：
//! - 矩阵记 RESPONDED（文本事实——正文写了就是写了）；
//! - 该行不得进入「可提交」状态（合规事实——没证明材料就是废标风险）；
//! - 门禁分工：导出**不阻断**（草稿可导出，见 #253）；提交就绪由
//!   evidence_missing 的必选条款阻断。
//!
//! 覆盖（裁定验收 1-4 的服务层半边 + 门禁分工正反面）：
//! 1. 正文有响应、无证据 → 矩阵 RESPONDED + evidence_missing 清单在场 +
//!    ready=false；**同一状态导出门放行**（两分账的直接证据）；
//! 2. 补充证据后缺口翻转、ready=true（验收 3）；
//! 3. 正文无响应 → unresponded 清单在场（验收 4——否决项未响应仍阻断）；
//! 4. 合法人工豁免（NOT_APPLICABLE + 偏离说明 + Human receipt）清除缺口
//!    阻断；普通 RESPONDED 覆写不豁免（#247 纪律同源）；
//! 5. 矩阵未构建 → not_built + ready=false（未对账不可判就绪）。
//!
//! 夹具纪律：域状态走真实状态机（create_project → upsert_file →
//! set_requirements → set_outline → build_matrix → register/bind），正文经
//! advance_section_status + set_section_content 落位，不 force 字段。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::principal::TrustedPrincipal;
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::validate::submission_readiness;

// ---------------------------------------------------------------- 夹具

fn req(id: &str, category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第三章/{id}"),
            excerpt: excerpt.into(),
        }],
        params: Default::default(),
        source_file: "招标文件.docx".into(),
    }
}

fn leaf(id: &str, title: &str, order: u32, reqs: &[&str]) -> OutlineNode {
    OutlineNode {
        id: id.into(),
        parent_id: None,
        title: title.into(),
        level: 1,
        order,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars: 800,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }
}

/// 基座：招标文件 + 条款 + 章节树 + 矩阵已建（正文未写）。
fn base_store(reqs: Vec<Requirement>, sections: Vec<OutlineNode>) -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("#241 提交就绪", "C-241R", "明标", "241r-key").unwrap();
    s.upsert_file("招标文件.docx", "招标文件", "sha-r", 10, 1, 50, 0, ParseStatus::Ok, "").unwrap();
    s.set_requirements(reqs).unwrap();
    s.set_outline(sections).unwrap();
    s.build_matrix().unwrap();
    s
}

/// 章节正文实写（真实状态链——不 force 字段）。
fn write_body(s: &mut DomainStore, sec_id: &str, content: &str) {
    s.advance_section_status(sec_id, SectionStatus::Outlining).unwrap();
    s.advance_section_status(sec_id, SectionStatus::Drafting).unwrap();
    s.set_section_content(sec_id, content.into()).unwrap();
    s.advance_section_status(sec_id, SectionStatus::Drafted).unwrap();
    s.build_matrix().unwrap();
}

/// 登记一条真实证据（Active，非占位）并返回 id。
fn register_real_ev(s: &mut DomainStore, id_hint: &str) -> String {
    let (eid, _) = s
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: format!("资质证书-{id_hint}"),
            source_file: format!("ev-{id_hint}.pdf"),
            source_sha256: format!("sha-{id_hint}"),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 3,
                text_fingerprint: format!("fnv64:{id_hint}"),
            },
            fingerprint: format!("fnv64:{id_hint}"),
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
    eid
}

fn ids(items: &[bid_tools::validate::ReadinessItem]) -> Vec<String> {
    items.iter().map(|i| i.requirement_id.clone()).collect()
}

// ---------------------------------------------------------------- 1. 裁定主面：两分账

/// 验收 1+2：正文有响应、无证据 → RESPONDED + evidence_missing 清单阻断
/// 提交就绪（明确理由 + 条款清单）；**同一状态导出门放行**（草稿可导出）。
#[test]
fn body_responded_gap_blocks_readiness_but_not_export() {
    let mut s = base_store(
        vec![req("REQ-0001", "资格条件", "投标人须提供营业执照副本及资质证书扫描件")],
        vec![leaf("SEC-0001", "资格响应", 1, &["REQ-0001"])],
    );
    write_body(&mut s, "SEC-0001", "我方具备完整资格条件，营业执照与资质证书齐备（见附件）。");

    // 矩阵面：RESPONDED（文本事实）+ 缺口标记（合规事实）
    let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0001").unwrap();
    assert_eq!(row.status, bid_domain::matrix::ResponseStatus::Responded);
    assert!(row.evidence_missing);

    // 提交就绪：缺口清单在场（条款可对账 + 理由含解决出口）
    let r = submission_readiness(&s);
    assert!(!r.ready, "证据缺口的必选条款不得进入可提交状态（裁定主面）");
    assert!(r.unresponded.is_empty(), "正文已响应——未响应清单为空");
    assert_eq!(ids(&r.evidence_missing), vec!["REQ-0001".to_string()], "缺口清单点名条款");
    let item = &r.evidence_missing[0];
    assert_eq!(item.row_id, "CMP-0001", "行 id 可与合规矩阵对账");
    assert_eq!(item.category, "资格条件");
    assert!(item.excerpt.contains("营业执照"), "原文摘录在场：{}", item.excerpt);
    assert!(item.message.contains("bind_evidence"), "理由含解决出口：{}", item.message);

    // 导出门：同一状态放行（两分账——草稿可导出，#253 分离面）
    assert!(s.check_export_ready().is_ok(), "导出不受证据缺口影响");
}

/// 验收 3：补充证据后缺口翻转 → ready=true（全清面）。
#[test]
fn evidence_binding_clears_gap_and_flips_ready() {
    let mut s = base_store(
        vec![req("REQ-0001", "资格条件", "投标人须提供营业执照副本及资质证书扫描件")],
        vec![leaf("SEC-0001", "资格响应", 1, &["REQ-0001"])],
    );
    write_body(&mut s, "SEC-0001", "我方具备完整资格条件（见附件）。");
    assert!(!submission_readiness(&s).ready, "缺证据 → 未就绪");

    let eid = register_real_ev(&mut s, "zz");
    s.bind_evidence(&eid, "REQ-0001", "2030-12-31").unwrap();
    s.build_matrix().unwrap();

    let r = submission_readiness(&s);
    assert!(r.ready, "补绑有效证据 → 缺口翻转、就绪（验收 3）: {r:?}");
    assert_eq!(r.blockers(), 0);
}

// ---------------------------------------------------------------- 2. 未响应面（验收 4）

/// 验收 4：正文无响应（否决项）→ 仍 NOT_RESPONDED，unresponded 清单阻断
/// 提交就绪（#253 验收 8：否决项未响应仍阻断 submission readiness）。
#[test]
fn unresponded_veto_blocks_readiness() {
    let s = base_store(
        vec![
            req("REQ-0001", "否决项", "投标文件未按规定格式签署将被否决"),
            req("REQ-0002", "商务要求", "服务期三年"),
        ],
        vec![leaf("SEC-0001", "响应章", 1, &["REQ-0001", "REQ-0002"])],
    );
    let r = submission_readiness(&s);
    assert!(!r.ready);
    assert_eq!(ids(&r.unresponded), vec!["REQ-0001".to_string()], "否决项未响应在列");
    assert!(r.unresponded[0].message.contains("废标级风险"), "理由点名风险等级");
    assert!(r.evidence_missing.iter().all(|i| i.requirement_id != "REQ-0002"),
        "非必选商务条款不进缺口阻断（信息面）");
}

// ---------------------------------------------------------------- 3. 人工豁免通道

/// 合法人工豁免（NOT_APPLICABLE + 偏离说明 + 可信 Human receipt）清除缺口
/// 阻断——人工裁决优先于机器判缺。
#[test]
fn legitimate_human_exemption_clears_gap() {
    let mut s = base_store(
        vec![req("REQ-0001", "资格条件", "投标人须提供营业执照副本及资质证书扫描件")],
        vec![leaf("SEC-0001", "资格响应", 1, &["REQ-0001"])],
    );
    write_body(&mut s, "SEC-0001", "我方具备完整资格条件（见附件）。");
    s.set_matrix_override(
        "REQ-0001".into(),
        bid_domain::matrix::ResponseStatus::NotApplicable,
        Some("集团共享资质由母公司统一持有，本项目凭授权函响应（人工裁定）".into()),
        &TrustedPrincipal::human("资格豁免裁定人"),
    )
    .unwrap();
    s.build_matrix().unwrap();

    let r = submission_readiness(&s);
    assert!(r.ready, "合法人工豁免 → 缺口阻断清除: {r:?}");
}

/// 普通 RESPONDED 覆写不豁免缺口（#247 纪律同源：覆写在场 ≠ 豁免成立——
/// 人工确认响应成立不等于材料已提交）。
#[test]
fn responded_override_does_not_exempt_gap() {
    let mut s = base_store(
        vec![req("REQ-0001", "资格条件", "投标人须提供营业执照副本及资质证书扫描件")],
        vec![leaf("SEC-0001", "资格响应", 1, &["REQ-0001"])],
    );
    write_body(&mut s, "SEC-0001", "我方具备完整资格条件（见附件）。");
    s.set_matrix_override(
        "REQ-0001".into(),
        bid_domain::matrix::ResponseStatus::Responded,
        Some("人工确认响应成立（材料后补）".into()),
        &TrustedPrincipal::human("响应确认人"),
    )
    .unwrap();
    s.build_matrix().unwrap();

    let r = submission_readiness(&s);
    assert!(!r.ready, "RESPONDED 覆写只确认响应事实，不清材料缺口");
    assert_eq!(ids(&r.evidence_missing), vec!["REQ-0001".to_string()]);
}

// ---------------------------------------------------------------- 4. 流程面

/// 矩阵未构建 → not_built + ready=false（未对账不可判就绪——诚实状态）。
#[test]
fn unbuilt_matrix_is_not_ready() {
    let mut s = DomainStore::new();
    s.create_project("#241 未建", "C-241N", "明标", "241n-key").unwrap();
    s.upsert_file("招标文件.docx", "招标文件", "sha-n", 10, 1, 50, 0, ParseStatus::Ok, "").unwrap();
    s.set_requirements(vec![req("REQ-0001", "商务要求", "服务期三年")]).unwrap();
    let r = submission_readiness(&s);
    assert!(!r.ready);
    assert!(r.not_built, "矩阵未构建 → not_built 标记");
    assert_eq!(r.blockers(), 0, "清单为空（参考面）——阻断来自流程面 not_built");
}

/// 就绪判定消费新鲜视图：draft 后无需手工重建矩阵，readiness 即时反映
/// 正文实写（content 变化 → 状态翻转的消费面保证）。
#[test]
fn readiness_consumes_fresh_view_without_manual_rebuild() {
    let mut s = base_store(
        vec![req("REQ-0001", "商务要求", "投标人须承诺质保期不少于五年")],
        vec![leaf("SEC-0001", "商务响应", 1, &["REQ-0001"])],
    );
    // 起草前：未响应（注意——非必选条款不进 unresponded 阻断面，
    // 但正文未写时矩阵行状态是 NOT_RESPONDED）
    let row = s.matrix().iter().find(|r| r.requirement_id == "REQ-0001").unwrap();
    assert_eq!(row.status, bid_domain::matrix::ResponseStatus::NotResponded);
    // 正文实写（不 build_matrix——模拟「调用方忘了重建」的场景）
    s.advance_section_status("SEC-0001", SectionStatus::Outlining).unwrap();
    s.advance_section_status("SEC-0001", SectionStatus::Drafting).unwrap();
    s.set_section_content("SEC-0001", "我方承诺质保期不少于五年。".into()).unwrap();
    // submission_readiness 内部走 matrix_view()（新鲜派生）——非必选
    // 承诺型条款：无未响应阻断、无缺口 → ready（fresh view 生效的间接断言：
    // 若消费陈旧 built matrix，REQ-0001 仍是 NOT_RESPONDED 也不影响 ready
    // ——非必选不阻断；故本用例同时断言视图行状态已翻转）
    let r = submission_readiness(&s);
    assert!(r.ready, "承诺型条款正文实写 → 就绪: {r:?}");
    let view = s.matrix_view();
    let vr = view.iter().find(|x| x.requirement_id == "REQ-0001").unwrap();
    assert_eq!(vr.status, bid_domain::matrix::ResponseStatus::Responded,
        "新鲜视图已反映正文实写（陈旧 built matrix 仍是 NOT_RESPONDED——两视图分工）");
}
