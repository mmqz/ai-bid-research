//! #217（#185 缺口 2）服务层单测：证据正文链路。
//!
//! 覆盖 issue 验收 D/E 的服务半边：
//! - **摘录 = 解析产物实际文本**（docx 段落锚 / pdf 页锚两种形态——不是
//!   文件名/标题转写）；
//! - **四态裁决**：带 locator 对账通过 → CONTENT_MATCHES；文件级锚 →
//!   FILE_EXISTS（摘录为 None，诚实标记「正文未定位」）；
//! - **摘录定位**：`《文件名》正文段 N（0 起）` / `《文件名》第 N 页`；
//! - **指纹一致**：摘录与指纹同源（同一 `locate_content` 产物）。
//!
//! 检索面（验收 E）与 MCP 出参见 bid-mcp/tests/evidence_content_tool.rs。

use bid_domain::evidence::{EvidenceKind, EvidenceFields, VerificationTier};
use bid_domain::locator::EngineLocator;
use bid_parse::env_preflight::skip_if_fixture_missing;
use bid_tools::service::{BidService, EvidenceRegistration, ImportSpec};
use std::path::PathBuf;

/// 仓库夹具根（#183 围栏：canonical 化）。
fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn fp_of(text: &str) -> String {
    format!("fnv64:{}", bid_domain::linkage::text_fingerprint(text))
}

/// 正文段文本（0 起序——与服务层 locator 对账同一基准）。
fn para_texts(svc: &BidService, sha: &str) -> Vec<String> {
    svc.parsed()
        .iter()
        .find(|p| p.sha256 == sha)
        .unwrap()
        .blocks
        .iter()
        .filter(|b| b.kind == bid_parse::BlockKind::Para)
        .map(|b| b.text.clone())
        .collect()
}

/// PDF 页文本（两条解析路前缀兼容——poppler `第1页 行1` / 兜底 `第1 页 > 第1 行`）。
fn pdf_page_text(svc: &BidService, sha: &str, page: u32) -> String {
    let parsed = svc
        .parsed()
        .iter()
        .find(|p| p.sha256 == sha)
        .unwrap_or_else(|| panic!("解析产物缺 sha={sha}"));
    let prefixes = [format!("第{page}页"), format!("第{page} 页")];
    parsed
        .blocks
        .iter()
        .filter(|b| prefixes.iter().any(|p| b.locator.starts_with(p.as_str())))
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 建项 + 导入 A-招标文件.docx + EV-03 业绩合同 PDF →
/// (svc, A 的 file_id, EV-03 的 file_id)。
fn service_with_sources(tag: &str) -> (BidService, String, String) {
    let mut svc = BidService::new().with_fs_root(fixtures());
    svc.store_mut()
        .create_project(&format!("#217 正文链路 {tag}"), "ZFCG-2026-EC", "明标", tag)
        .unwrap();
    let a = fixtures().join("corpus-A/files/A-招标文件.docx");
    let ev03 = fixtures().join("corpus-A/files/evidence/EV-03-类似业绩合同-智慧园区弱电工程.pdf");
    if skip_if_fixture_missing("evidence_content_service", &[&a, &ev03]) {
        panic!("夹具缺失——本测试依赖 corpus-A（A-招标文件.docx + EV-03）");
    }
    let report = svc
        .import(&[
            ImportSpec {
                upload_ref: a,
                filename: "A-招标文件.docx".into(),
                kind: "招标文件".into(),
                expected_pages: None,
            },
            ImportSpec {
                upload_ref: ev03,
                filename: "EV-03-类似业绩合同-智慧园区弱电工程.pdf".into(),
                kind: "附件".into(),
                expected_pages: None,
            },
        ])
        .unwrap();
    assert_eq!(report.item_failures, 0, "两件应全部导入成功: {:?}", report.per_file);
    let fid_a = svc.store().files().iter().find(|f| f.name == "A-招标文件.docx").unwrap().id.clone();
    let fid_ev = svc
        .store()
        .files()
        .iter()
        .find(|f| f.name == "EV-03-类似业绩合同-智慧园区弱电工程.pdf")
        .unwrap()
        .id
        .clone();
    (svc, fid_a, fid_ev)
}

fn reg(source_file_id: &str, locator: Option<EngineLocator>, title: &str) -> EvidenceRegistration {
    EvidenceRegistration {
        source_file_id: source_file_id.to_string(),
        kind: EvidenceKind::Performance,
        title: title.to_string(),
        locator,
        valid_from: None,
        valid_until: None,
        subject_name: "投标人甲".into(),
        scope: "详见原件".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: EvidenceFields::default(),
    }
}

/// 验收 D（docx 面）：段落锚登记 → excerpt = 该段实际文本（逐字），
/// tier = CONTENT_MATCHES，excerpt_locator 人话定位。
#[test]
fn docx_locator_anchor_captures_paragraph_text() {
    let (mut svc, fid_a, _) = service_with_sources("docx-anchor");
    let tf = svc.store().files().iter().find(|f| f.id == fid_a).unwrap().clone();
    let paras = para_texts(&svc, &tf.sha256);
    assert!(paras.len() > 80, "A 件正文段规模: {}", paras.len());
    let idx = 1usize; // 项目名称段
    let (ev_id, _) = svc
        .register_evidence_verified(reg(
            &fid_a,
            Some(EngineLocator::DocxParagraph {
                paragraph_index: idx as u32,
                text_fingerprint: fp_of(&paras[idx]),
            }),
            "类似业绩（docx 段落锚）",
        ))
        .unwrap();
    let ev = svc.store().evidence(&ev_id).unwrap().clone();
    assert_eq!(
        ev.verification_tier,
        Some(VerificationTier::ContentMatches),
        "段落锚对账通过 → CONTENT_MATCHES"
    );
    assert_eq!(
        ev.excerpt.as_deref(),
        Some(paras[idx].as_str()),
        "摘录 = 解析产物该段实际文本（逐字，非标题转写）"
    );
    assert_eq!(
        ev.excerpt_locator.as_deref(),
        Some(format!("《A-招标文件.docx》正文段 {idx}（0 起）").as_str()),
        "摘录定位 = 文件名 + 段序（人话面）"
    );
    // 指纹与摘录同源（同一 locate_content 产物——验收「指纹验证一致」）
    assert_eq!(ev.fingerprint, fp_of(&paras[idx]));
}

/// 验收 D（pdf 面）：页锚登记 → excerpt = 该页全部行块拼接的实际文本，
/// tier = CONTENT_MATCHES，定位 = 《文件》第 N 页。
#[test]
fn pdf_page_anchor_captures_page_text() {
    let (mut svc, _, fid_ev) = service_with_sources("pdf-anchor");
    let tf = svc
        .store()
        .files()
        .iter()
        .find(|f| f.id == fid_ev)
        .unwrap()
        .clone();
    let page1 = pdf_page_text(&svc, &tf.sha256, 1);
    assert!(!page1.trim().is_empty(), "EV-03 首页应有文本层");
    assert!(
        page1.contains("合同金额") && page1.contains("260"),
        "EV-03 首页实料在位: {page1}"
    );
    let (ev_id, _) = svc
        .register_evidence_verified(reg(
            &fid_ev,
            Some(EngineLocator::PdfPageText {
                page: 1,
                text_fingerprint: fp_of(&page1),
            }),
            "类似业绩合同（pdf 页锚）",
        ))
        .unwrap();
    let ev = svc.store().evidence(&ev_id).unwrap().clone();
    assert_eq!(ev.verification_tier, Some(VerificationTier::ContentMatches));
    assert_eq!(
        ev.excerpt.as_deref(),
        Some(page1.as_str()),
        "摘录 = 该页实际文本（行块拼接，与服务层同口径）"
    );
    assert_eq!(
        ev.excerpt_locator.as_deref(),
        Some("《EV-03-类似业绩合同-智慧园区弱电工程.pdf》第 1 页"),
        "摘录定位 = 文件名 + 页号"
    );
}

/// 四态：文件级锚（无 locator）→ FILE_EXISTS，摘录为 None——
/// 来源在库但正文未定位，不编造「读过了」的假象。
#[test]
fn file_level_anchor_is_file_exists_without_excerpt() {
    let (mut svc, _, fid_ev) = service_with_sources("file-anchor");
    let (ev_id, _) = svc
        .register_evidence_verified(reg(&fid_ev, None, "类似业绩合同（文件级锚）"))
        .unwrap();
    let ev = svc.store().evidence(&ev_id).unwrap().clone();
    assert_eq!(ev.verification_tier, Some(VerificationTier::FileExists));
    assert_eq!(ev.excerpt, None, "文件级锚无摘录（正文未定位）");
    assert_eq!(ev.excerpt_locator, None);
}

/// 换版链路：同语义位换 locator（内容变更）→ 新版本记录携带新摘录 +
/// 新 tier；旧记录 Superseded（正文链路不破坏换版纪律）。
#[test]
fn re_registration_with_locator_upgrades_tier_and_excerpt() {
    let (mut svc, _, fid_ev) = service_with_sources("upgrade");
    let (id1, _) = svc
        .register_evidence_verified(reg(&fid_ev, None, "业绩合同（升级链）"))
        .unwrap();
    let ev1 = svc.store().evidence(&id1).unwrap().clone();
    assert_eq!(ev1.verification_tier, Some(VerificationTier::FileExists));

    let tf = svc.store().files().iter().find(|f| f.id == fid_ev).unwrap().clone();
    let page1 = pdf_page_text(&svc, &tf.sha256, 1);
    // 同语义位（kind+title+subject+scope 全同）带 locator 重登 → 换版
    let (id2, _) = svc
        .register_evidence_verified(reg(
            &fid_ev,
            Some(EngineLocator::PdfPageText {
                page: 1,
                text_fingerprint: fp_of(&page1),
            }),
            "业绩合同（升级链）",
        ))
        .unwrap();
    assert_ne!(id1, id2, "换版 = 新记录新 id");
    let ev2 = svc.store().evidence(&id2).unwrap().clone();
    assert_eq!(ev2.verification_tier, Some(VerificationTier::ContentMatches), "换版升级到内容级锚");
    assert_eq!(ev2.version, 2, "版本递增");
    assert!(ev2.excerpt.as_deref().unwrap().contains("合同金额"), "新版本携带摘录");
    assert_eq!(
        svc.store().evidence(&id1).unwrap().status,
        bid_domain::evidence::EvidenceStatus::Superseded,
        "旧版本被淘汰（单向降级）"
    );
}
