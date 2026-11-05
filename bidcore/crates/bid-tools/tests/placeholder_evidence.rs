//! #245 服务层单测：占位/待核验证据（PLACEHOLDER 态）。
//!
//! 覆盖 issue #245 服务层验收面：
//! - **登记裁决**：`placeholder: true` → tier=PLACEHOLDER（即便 locator 对账
//!   通过也不升格——「文件里有这段文本」≠「资质真实具备」）；摘录照记
//!   （占位指向回溯面）；
//! - **矩阵分流**：占位绑定 → PENDING_EVIDENCE + 必选行 blocking 保留
//!   （bind_evidence_batch 回执的 blocking 面即刻可见）；真实证据响应零回归；
//! - **起草面**：mock 生成器对占位证据落【待补：...】占位文案——不引用
//!   [EV-xxxx] 编号、不声称「已具备/已满足」；
//! - **就绪面**：占位证据不满足资格章节的素材就绪判定（MissingQualification
//!   仍报——「待补」可见但「素材齐了」的结论不能下）；
//! - **升级路径**：真实材料登记（换版）→ 绑定迁移 → 矩阵行按真实证据派生。
//!
//! 域层纯逻辑（查重/换版/绑定迁移）见 bid-domain/src/evidence.rs 测试面；
//! MCP 契约面（placeholder 参数/自报拒）见 bid-mcp/tests/placeholder_evidence_tool.rs。

use bid_domain::evidence::{EvidenceKind, EvidenceFields, VerificationTier};
use bid_domain::locator::EngineLocator;
use bid_domain::matrix::ResponseStatus;
use bid_domain::outline::OutlineSpec;
use bid_domain::{Anchor, ParseStatus, Requirement};
use bid_parse::env_preflight::skip_if_fixture_missing;
use bid_parse::BlockKind;
use bid_tools::draft::{build_section_input, MockGenerator, SectionGenerator};
use bid_tools::readiness::is_usable_evidence;
use bid_tools::service::{BidService, EvidenceRegistration, ImportSpec};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn fp_of(text: &str) -> String {
    format!("fnv64:{}", bid_domain::linkage::text_fingerprint(text))
}

fn req(id: &str, category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("第三章/{id}"),
            excerpt: excerpt.into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

/// 建项 + 导入真实 A-招标文件.docx（解析产物在位——register_evidence_verified
/// 的对账面）+ 合成 3 REQ（资格/否决/评分）+ 建纲建矩阵 → MATRIX_READY。
fn matrix_ready_service(tag: &str) -> BidService {
    let mut svc = BidService::new().with_fs_root(fixtures());
    svc.store_mut()
        .create_project(&format!("#245 服务层 {tag}"), "ZFCG-2026-PH", "明标", tag)
        .unwrap();
    let a = fixtures().join("corpus-A/files/A-招标文件.docx");
    if skip_if_fixture_missing("placeholder_evidence_service", &[&a]) {
        return svc;
    }
    let report = svc
        .import(&[ImportSpec {
            upload_ref: a,
            filename: "A-招标文件.docx".into(),
            kind: "招标文件".into(),
            expected_pages: None,
        }])
        .unwrap();
    assert_eq!(report.item_failures, 0, "A 件应导入成功");
    // 补齐招标包完整性面（补遗件在场—— readiness 警告面不进断言）
    svc.store_mut()
        .upsert_file("C-补遗件01.docx", "补遗", "sha-add-245", 5, 0, 20, 0, ParseStatus::Ok, "")
        .unwrap();
    svc.set_requirements(vec![
        req("REQ-0001", "资格条件", "投标人须具备电子与智能化工程专业承包二级及以上资质"),
        req("REQ-0002", "否决项", "投标文件未经法定代表人或其授权代表签字的，其投标作否决处理"),
        req("REQ-0003", "评分项", "施工组织设计方案完整性分值 10 分"),
    ])
    .unwrap();
    let (n_sec, n_rows, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 3_000, max_reqs_per_leaf: 2 })
        .unwrap();
    assert!(n_sec >= 2 && n_rows >= 3, "建纲建矩阵应成功：{n_sec} 节点 / {n_rows} 行");
    svc
}

fn reg(source_file_id: &str, locator: Option<EngineLocator>, title: &str, placeholder: bool) -> EvidenceRegistration {
    EvidenceRegistration {
        source_file_id: source_file_id.to_string(),
        placeholder,
        kind: EvidenceKind::Qualification,
        title: title.to_string(),
        locator,
        valid_from: None,
        valid_until: None,
        subject_name: "投标人甲".into(),
        scope: "电子与智能化工程".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: EvidenceFields::default(),
    }
}

/// 登记 + 返回 EV id（占位开关驱动）。
fn register(svc: &mut BidService, title: &str, placeholder: bool) -> String {
    let tf = svc.store().files().first().cloned().unwrap();
    let (ev_id, _) = svc
        .register_evidence_verified(reg(&tf.id, None, title, placeholder))
        .unwrap();
    ev_id
}

// ================================================================ 登记裁决

/// placeholder=true → tier=PLACEHOLDER（文件级锚占位）——不因「来源真实」
/// 升格为已验证：占位登记的语义就是「内容未经核验」。
#[test]
fn placeholder_registration_lands_placeholder_tier() {
    let mut svc = matrix_ready_service("tier-file");
    if svc.store().files().is_empty() {
        return; // 夹具缺失 SKIP
    }
    let ev_id = register(&mut svc, "电子与智能化工程专业承包资质（占位）", true);
    let ev = svc.store().evidence(&ev_id).unwrap();
    assert_eq!(ev.verification_tier, Some(VerificationTier::Placeholder));
    assert!(ev.is_placeholder());
    assert_eq!(ev.excerpt, None, "文件级锚占位无摘录");
    // 对照：非占位登记 → FILE_EXISTS（既有语义零回归）
    let ev2 = register(&mut svc, "电子与智能化工程专业承包资质（真实）", false);
    assert_eq!(
        svc.store().evidence(&ev2).unwrap().verification_tier,
        Some(VerificationTier::FileExists)
    );
}

/// placeholder=true 且带 locator（对账通过）→ 仍 PLACEHOLDER：占位材料的
/// 定位摘录是「占位指向」的回溯面，不是「内容已核实」的凭证——即便指向
/// 的文本逐字在库（如招标文件里的资质要求文本），也不得计为已验证。
#[test]
fn placeholder_with_locator_still_placeholder_tier() {
    let mut svc = matrix_ready_service("tier-locator");
    if svc.store().files().is_empty() {
        return;
    }
    let tf = svc.store().files().first().cloned().unwrap();
    let parsed = svc.parsed().iter().find(|p| p.sha256 == tf.sha256).unwrap();
    let para = parsed
        .blocks
        .iter()
        .find(|b| b.kind == BlockKind::Para)
        .cloned()
        .unwrap();
    let loc = EngineLocator::DocxParagraph {
        paragraph_index: parsed
            .blocks
            .iter()
            .filter(|b| b.kind == BlockKind::Para)
            .position(|b| b.text == para.text)
            .unwrap() as u32,
        text_fingerprint: fp_of(&para.text),
    };
    let (ev_id, _) = svc
        .register_evidence_verified(reg(&tf.id, Some(loc), "资质条款占位（带定位）", true))
        .unwrap();
    let ev = svc.store().evidence(&ev_id).unwrap();
    assert_eq!(
        ev.verification_tier,
        Some(VerificationTier::Placeholder),
        "locator 对账通过也不升格——占位材料的定位是回溯面不是验证凭证"
    );
    assert!(ev.excerpt.is_some(), "摘录照记（占位指向回溯）");
    assert!(!ev.verification_tier.unwrap().content_read());
}

// ================================================================ 矩阵分流

/// 占位绑定 → 矩阵 PENDING_EVIDENCE + 必选行 blocking 保留（bind_evidence_batch
/// 回执面即刻可见）；非必选行待补不阻断。
#[test]
fn placeholder_binding_keeps_blocking_in_batch_reply() {
    let mut svc = matrix_ready_service("matrix-block");
    if svc.store().files().is_empty() {
        return;
    }
    let ev_id = register(&mut svc, "资质占位（矩阵面）", true);
    // 占位证据绑定到全部条款（事故形态：批量绑定）
    let reqs: Vec<String> = svc.store().requirements().iter().map(|r| r.id.clone()).collect();
    let (_, _, errs, _rows, blocking) = svc
        .bind_evidence_batch(&ev_id, &reqs, "2026-11-15")
        .unwrap();
    assert!(errs.is_empty(), "占位绑定本身不拒（待补载体）: {errs:?}");
    assert!(blocking > 0, "占位绑定后必选行仍 blocking（事故缺口收口——阻塞不归零）");
    // 逐行断言：资格/否决行 → PENDING_EVIDENCE + blocking；占位进待补面
    let mut pending_rows = 0usize;
    for row in svc.store().matrix() {
        if row.pending_evidence_ids.contains(&ev_id) {
            pending_rows += 1;
            assert_eq!(row.status, ResponseStatus::PendingEvidence);
            assert!(!row.evidence_ids.contains(&ev_id), "占位不进有效证据列");
        }
    }
    assert!(pending_rows >= blocking, "blocking 行必在待补面中（{pending_rows} 待补 / {blocking} 阻断）");
}

/// 真实证据响应零回归：非占位登记绑定 → PARTIAL/RESPONDED 照常、blocking 清零。
#[test]
fn real_evidence_binding_still_clears_blocking() {
    let mut svc = matrix_ready_service("matrix-real");
    if svc.store().files().is_empty() {
        return;
    }
    let ev_id = register(&mut svc, "资质真实（矩阵对照）", false);
    let reqs: Vec<String> = svc.store().requirements().iter().map(|r| r.id.clone()).collect();
    let (_, _, errs, _rows, blocking) = svc
        .bind_evidence_batch(&ev_id, &reqs, "2026-11-15")
        .unwrap();
    assert!(errs.is_empty());
    assert_eq!(blocking, 0, "真实证据绑定 → blocking 清零（既有正向流程零回归）");
    let row = &svc.store().matrix()[0];
    // 建纲后章节在位：真实证据+章节 → RESPONDED（有证据无章节才是 PARTIAL）
    assert_eq!(row.status, ResponseStatus::Responded, "真实证据+章节 → 已响应");
    assert!(row.evidence_ids.contains(&ev_id));
}

// ================================================================ 起草面

/// mock 生成器占位纪律：占位证据 → 【待补：...】文案；不引用 [EV-xxxx]；
/// 不输出「已具备/可核验」结论。
#[test]
fn mock_draft_renders_placeholder_as_pending_not_claim() {
    let mut svc = matrix_ready_service("draft-pending");
    if svc.store().files().is_empty() {
        return;
    }
    let tf = svc.store().files().first().cloned().unwrap();
    let parsed = svc.parsed().iter().find(|p| p.sha256 == tf.sha256).unwrap();
    let para = parsed
        .blocks
        .iter()
        .find(|b| b.kind == bid_parse::BlockKind::Para)
        .cloned()
        .unwrap();
    // 占位登记（带定位——摘录在场，生成器仍不得引用）
    let loc = EngineLocator::DocxParagraph {
        paragraph_index: parsed
            .blocks
            .iter()
            .filter(|b| b.kind == bid_parse::BlockKind::Para)
            .position(|b| b.text == para.text)
            .unwrap() as u32,
        text_fingerprint: fp_of(&para.text),
    };
    let (ev_id, _) = svc
        .register_evidence_verified(reg(&tf.id, Some(loc), "资质占位（起草面）", true))
        .unwrap();
    // 绑到首个 REQ；取矩阵行携带的章节（叶——结构节点不承载正文）
    let req_id = svc.store().requirements()[0].id.clone();
    let (_, _, _, _, _) = svc.bind_evidence_batch(&ev_id, &[req_id.clone()], "2026-11-15").unwrap();
    let section_id = {
        let sections = svc.store().sections();
        let has_child: std::collections::HashSet<&str> = sections
            .iter()
            .filter_map(|n| n.parent_id.as_deref())
            .collect();
        sections
            .iter()
            .filter(|n| !has_child.contains(n.id.as_str()) && n.requirement_ids.contains(&req_id))
            .map(|n| n.id.clone())
            .next()
            .expect("章节树应含该 REQ 的叶章节")
    };
    // 起草输入装配：占位标记随行
    let (input, _warns) = build_section_input(&svc.store(), &section_id, None, None, None).unwrap();
    let brief = input
        .evidences
        .iter()
        .find(|e| e.id == ev_id)
        .expect("占位证据进起草输入（生成器需知道待补位）");
    assert!(brief.placeholder, "EvBrief 携带占位标记");
    // mock 生成
    let mut gen = MockGenerator;
    let draft = gen.generate(&input).unwrap();
    assert!(draft.content.contains("【待补："), "占位证据落【待补：...】占位文案");
    assert!(
        !draft.content.contains(&format!("[{ev_id}]")),
        "占位证据不得被引用为佐证（无 [EV] 编号引用）"
    );
    assert!(
        !draft.content.contains("具备可核验的证据支撑"),
        "占位在章不得声称已具备可核验的证据支撑"
    );
    assert!(
        draft.content.contains("不作已具备声明"),
        "收束段措辞收敛（占位在章不作已具备声明）"
    );
}

// ================================================================ 就绪面

/// 占位证据不满足就绪判定：is_usable_evidence 对占位返回 false——
/// 资格章节的素材就绪不被占位登记静默满足。
#[test]
fn placeholder_is_not_usable_evidence() {
    let mut svc = matrix_ready_service("readiness");
    if svc.store().files().is_empty() {
        return;
    }
    let ev_ph = register(&mut svc, "资质占位（就绪面）", true);
    let ev_real = register(&mut svc, "资质真实（就绪面）", false);
    let ph = svc.store().evidence(&ev_ph).unwrap().clone();
    let real = svc.store().evidence(&ev_real).unwrap().clone();
    assert!(!is_usable_evidence(&ph), "占位证据不可用（不算「有」）");
    assert!(is_usable_evidence(&real), "真实证据可用（零回归）");
    assert_eq!(ph.status.as_str(), "ACTIVE", "占位状态面仍 Active（有效期维度正交）");
}

// ================================================================ 升级路径

/// 占位 → 真实升级全路径（服务层半边）：真实材料登记（同语义位异指纹 →
/// 换版）→ 旧占位记录 Superseded + 绑定迁移到新记录 → 矩阵按真实证据派生。
#[test]
fn upgrade_placeholder_to_real_migrates_bindings_and_matrix() {
    let mut svc = matrix_ready_service("upgrade");
    if svc.store().files().is_empty() {
        return;
    }
    let ev_ph = register(&mut svc, "升级路径资质（占位）", true);
    let req_id = svc.store().requirements()[0].id.clone();
    let (_, _, errs, _, blocking_before) =
        svc.bind_evidence_batch(&ev_ph, &[req_id.clone()], "2026-11-15").unwrap();
    assert!(errs.is_empty());
    assert!(blocking_before >= 1 || svc.store().matrix().is_empty(), "占位绑定不清阻塞（或无必选行）");

    // 换版升级：真实登记（同 title/scope/subject —— 同语义位；文件级锚指纹同？
    // 非占位文件级锚指纹 = sha256:<同文件> 与占位文件级锚指纹相同 → tier 不同
    // 即触发换版（#245：same_payload 含 tier）。
    let tf = svc.store().files().first().cloned().unwrap();
    let (ev_real, dedup) = svc
        .register_evidence_verified(reg(&tf.id, None, "升级路径资质（占位）", false))
        .unwrap();
    assert!(!dedup, "tier 变化 → 换版（非重放）");
    let ph = svc.store().evidence(&ev_ph).unwrap();
    assert_eq!(ph.status.as_str(), "SUPERSEDED", "旧占位记录留审计痕");
    let real = svc.store().evidence(&ev_real).unwrap();
    assert_eq!(real.version, 2, "版本递增");
    assert!(!real.is_placeholder());
    // 绑定迁移：旧占位绑定改指新记录（无残留）
    assert!(
        svc.store().evidence_bindings().iter().any(|b| b.evidence_id == ev_real),
        "占位绑定迁移到真实记录"
    );
    assert!(
        !svc.store().evidence_bindings().iter().any(|b| b.evidence_id == ev_ph),
        "旧占位绑定不残留"
    );
    // 矩阵按真实证据派生（build_matrix 重跑）
    let (_, _, _, _, _) =
        svc.bind_evidence_batch(&ev_real, &[req_id], "2026-11-15").unwrap();
    let row = svc.store().matrix().iter().find(|r| r.requirement_id == "REQ-0001").unwrap();
    // 建纲后章节在位：真实证据+章节 → RESPONDED（占位阻塞解除）
    assert_eq!(row.status, ResponseStatus::Responded, "真实证据+章节 → 已响应（占位阻塞解除）");
    assert!(!row.pending_evidence_ids.contains(&ev_ph), "旧占位不进待补面（已迁移）");
    let all_blocking: usize = svc.store().matrix().iter().filter(|r| r.blocking).count();
    assert!(all_blocking < blocking_before, "升级后 blocking 面收缩（REQ-0001 行解除）");
}
