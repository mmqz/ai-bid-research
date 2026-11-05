//! #185 缺口 1 服务层单测：`register_evidence_verified` 来源验证面。
//!
//! 覆盖 issue #185 点名的两条服务层断言：
//! - **库内 sha 优先于调用方**：登记入参没有 sha 位（`EvidenceRegistration`
//!   不接受来源三件套），台账里的 `source_file` / `source_sha256` /
//!   `fingerprint` 必须等于库内 TenderFile 与实际内容的系统生成值；
//! - **locator 指纹不匹配拒**：`DocxParagraph` 的 `text_fingerprint` 与
//!   解析产物实际段落文本不符 → E_UNPROCESSABLE（零入库）。
//!
//! 域层 `register_evidence`（纯逻辑面）不动——域测试用伪造 spec 测查重/
//! 换版/翻转是合法路径（见 bid-domain/src/evidence.rs 测试面）。

use bid_domain::evidence::{EvidenceKind, EvidenceFields};
use bid_domain::locator::EngineLocator;
use bid_domain::ToolError;
use bid_parse::env_preflight::skip_if_fixture_missing;
use bid_tools::service::{BidService, EvidenceRegistration, ImportSpec};
use std::path::PathBuf;

/// 仓库夹具根（#183 围栏：canonical 化——词法 `..` 会被围栏预拒）。
fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn fp_of(text: &str) -> String {
    format!("fnv64:{}", bid_domain::linkage::text_fingerprint(text))
}

/// 建项 + 导入 corpus-A A-招标文件.docx（引擎 docx 路径——解析产物在位）。
fn imported_docx_service(tag: &str) -> BidService {
    // #183 围栏：声明仓库夹具根（canonical——词法 `..` 会被围栏预拒）。
    let mut svc = BidService::new().with_fs_root(fixtures());
    svc.store_mut()
        .create_project(&format!("#185 服务层 {tag}"), "ZFCG-2026-SV", "明标", tag)
        .unwrap();
    let a = fixtures().join("corpus-A/files/A-招标文件.docx");
    if skip_if_fixture_missing("evidence_source_service", &[&a]) {
        return svc;
    }
    let specs = [ImportSpec {
        upload_ref: a,
        filename: "A-招标文件.docx".into(),
        kind: "招标文件".into(),
        expected_pages: None,
    }];
    let report = svc.import(&specs).unwrap();
    assert_eq!(report.item_failures, 0, "A 件应导入成功: {:?}", report.per_file);
    svc
}

fn reg(source_file_id: &str, locator: Option<EngineLocator>, title: &str) -> EvidenceRegistration {
    EvidenceRegistration {
        source_file_id: source_file_id.to_string(),
        placeholder: false,
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

/// 库内 sha 优先于调用方：入参无 sha 位，台账来源三件套必须等于库内与
/// 实际内容的系统生成值（文件级锚：指纹 = sha256:<库内 sha>）。
#[test]
fn stored_sha_and_system_fingerprint_win() {
    let mut svc = imported_docx_service("stored-sha");
    let Some(tf) = svc.store().files().first().cloned() else {
        return; // 夹具缺失 SKIP（skip_if_fixture_missing 已打横幅）
    };
    let (ev_id, dedup) = svc
        .register_evidence_verified(reg(&tf.id, None, "营业执照（服务层·文件级锚）"))
        .unwrap();
    assert!(!dedup, "首登非重放");
    let ev = svc.store().evidence(&ev_id).unwrap().clone();
    assert_eq!(ev.source_file, tf.name, "source_file 取 TenderFile.name（库内真源）");
    assert_eq!(ev.source_sha256, tf.sha256, "source_sha256 取库内 sha（调用方无从传入）");
    assert_eq!(
        ev.fingerprint,
        format!("sha256:{}", tf.sha256),
        "文件级锚指纹由系统从库内 sha 派生"
    );
    assert_eq!(
        ev.locator,
        EngineLocator::MarkdownBlock {
            block_id: "mcp-registered:A-招标文件.docx".into(),
            text_fingerprint: format!("sha256:{}", tf.sha256),
        },
        "缺省 locator = 登记面诚实标记（系统生成，非调用方拼装）"
    );
    // 重放幂等（来源验证不破坏语义位查重）
    let (ev_id2, dedup2) = svc
        .register_evidence_verified(reg(&tf.id, None, "营业执照（服务层·文件级锚）"))
        .unwrap();
    assert!(dedup2, "同 payload 重放");
    assert_eq!(ev_id2, ev_id, "重放回原 id");
}

/// locator 指纹不匹配拒：paragraph_index 在范围内但 text_fingerprint 与
/// 实际段落文本不符 → E_UNPROCESSABLE + 零入库。
#[test]
fn locator_fingerprint_mismatch_rejected() {
    let mut svc = imported_docx_service("fp-mismatch");
    let Some(tf) = svc.store().files().first().cloned() else {
        return;
    };
    let paras = para_texts(&svc, &tf.sha256);
    assert!(paras.len() > 5, "A 件正文段规模: {}", paras.len());
    let n_before = svc.store().evidences().len();
    let err = svc
        .register_evidence_verified(reg(
            &tf.id,
            Some(EngineLocator::DocxParagraph {
                paragraph_index: 0,
                text_fingerprint: "fnv64:0000000000000000".into(),
            }),
            "指纹不符（服务层）",
        ))
        .unwrap_err();
    match &err {
        ToolError::EUnprocessable { reason } => {
            assert!(reason.contains("指纹不匹配"), "错误文案: {reason}");
            assert!(reason.contains(&fp_of(&paras[0])), "携带实际指纹便于对账: {reason}");
        }
        other => panic!("应 E_UNPROCESSABLE: {other:?}"),
    }
    assert_eq!(svc.store().evidences().len(), n_before, "拒绝路径零入库");
}

/// locator 段落越界拒（验收 B 服务层面）：paragraph_index 超出正文段范围。
#[test]
fn locator_paragraph_out_of_range_rejected() {
    let mut svc = imported_docx_service("para-range");
    let Some(tf) = svc.store().files().first().cloned() else {
        return;
    };
    let paras = para_texts(&svc, &tf.sha256);
    let err = svc
        .register_evidence_verified(reg(
            &tf.id,
            Some(EngineLocator::DocxParagraph {
                paragraph_index: paras.len() as u32 + 100,
                text_fingerprint: "fnv64:deadbeef".into(),
            }),
            "越界段落（服务层）",
        ))
        .unwrap_err();
    match &err {
        ToolError::EUnprocessable { reason } => {
            assert!(reason.contains("越界"), "错误文案: {reason}");
        }
        other => panic!("应 E_UNPROCESSABLE: {other:?}"),
    }
}

/// 正常登记（带 locator）：指纹 = 定位文本系统生成值（fnv64:<hex>——
/// 与既有 text_fingerprint 同族），locator 原样入账。
#[test]
fn locator_registration_uses_content_fingerprint() {
    let mut svc = imported_docx_service("content-fp");
    let Some(tf) = svc.store().files().first().cloned() else {
        return;
    };
    let paras = para_texts(&svc, &tf.sha256);
    let (ev_id, _) = svc
        .register_evidence_verified(reg(
            &tf.id,
            Some(EngineLocator::DocxParagraph {
                paragraph_index: 1,
                text_fingerprint: fp_of(&paras[1]),
            }),
            "资质证书（服务层·段落锚）",
        ))
        .unwrap();
    let ev = svc.store().evidence(&ev_id).unwrap().clone();
    assert_eq!(ev.fingerprint, fp_of(&paras[1]), "指纹 = 系统从实际段落内容生成");
    assert_eq!(ev.source_sha256, tf.sha256, "sha 取库内");
    assert_eq!(
        ev.locator,
        EngineLocator::DocxParagraph {
            paragraph_index: 1,
            text_fingerprint: fp_of(&paras[1]),
        },
        "locator 原样入账（已过对账）"
    );
}

/// 幽灵 file_id / 解析失败件 / 无解析产物 → 拒（服务层面三防线）。
#[test]
fn ghost_failed_and_unparsed_sources_rejected() {
    let mut svc = imported_docx_service("rejects");
    // 幽灵 id
    let err = svc
        .register_evidence_verified(reg("TF-9999", None, "幽灵文件（服务层）"))
        .unwrap_err();
    assert!(matches!(err, ToolError::ENotFound { .. }), "幽灵 id → E_NOTFOUND: {err:?}");

    // 解析失败件（D-图纸说明.md——不支持的文件类型 → Failed 档）
    let d = fixtures().join("synth-tender/招标文件/D-图纸说明.md");
    if skip_if_fixture_missing("evidence_source_service_d", &[&d]) {
        return;
    }
    let report = svc
        .import(&[ImportSpec {
            upload_ref: d,
            filename: "D-图纸说明.md".into(),
            kind: "图纸".into(),
            expected_pages: None,
        }])
        .unwrap();
    assert_eq!(report.item_failures, 1, "D 件应失败登记");
    let fid_d = svc
        .store()
        .files()
        .iter()
        .find(|f| f.name == "D-图纸说明.md")
        .unwrap()
        .id
        .clone();
    let err = svc
        .register_evidence_verified(reg(&fid_d, None, "失败件（服务层）"))
        .unwrap_err();
    match &err {
        ToolError::EUnprocessable { reason } => {
            assert!(reason.contains("解析失败"), "错误文案: {reason}");
        }
        other => panic!("失败件应 E_UNPROCESSABLE: {other:?}"),
    }

    // 无解析产物：库里有档但 parsed 不在场（绕过 import 管线直插的档）
    let fid_ghost = {
        let st = svc.store_mut();
        st.upsert_file(
            "手工档.pdf", "附件", "sha-not-in-parsed", 1, 0, 1, 0,
            bid_domain::ParseStatus::Ok, "手工插入",
        )
        .unwrap()
        .0
    };
    let err = svc
        .register_evidence_verified(reg(&fid_ghost, None, "无解析产物（服务层）"))
        .unwrap_err();
    match &err {
        ToolError::EUnprocessable { reason } => {
            assert!(reason.contains("解析产物"), "错误文案: {reason}");
        }
        other => panic!("无解析产物应 E_UNPROCESSABLE: {other:?}"),
    }
}
