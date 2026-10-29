//! B-1 服务层契约测试：`service::import_files` 导入管线（runner 提升后的单一真源）。
//!
//! 覆盖 MCP 壳之下的业务半边：落库真源口径（DomainStore）、解析累积
//! （M1-b 输入面）、不可读件的逐件失败语义、kind 分层放行（契约枚举
//! 校验属壳，服务层透传——运行器「证据」类别依赖此分层）。

use bid_domain::{DomainStore, ParseStatus, ProjectStatus};
use bid_parse::env_preflight::skip_if_fixture_missing;
use bid_tools::service::{import_files, BidService, ImportSpec};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // crates/bid-tools → 仓库根
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
        })
}

fn spec(path: PathBuf, filename: &str, kind: &str) -> ImportSpec {
    ImportSpec { upload_ref: path, filename: filename.to_string(), kind: kind.to_string(), expected_pages: None }
}

/// 真实主文件（corpus-A A-招标文件.docx）经管线落库：统计面与
/// corpus_probe 硬口径一致（tables >= 5），状态机 DRAFT → FILES_IMPORTED，
/// 解析累积在位（M1-b 抽取输入面）。
#[test]
fn service_imports_corpus_a_main_file_into_store() {
    let mut store = DomainStore::new();
    store.create_project("p", "ZFCG-2026-GK-0311", "明标", "ext-svc").unwrap();
    let mut parsed = vec![];
    let a = fixtures().join("corpus-A/files/A-招标文件.docx");
    if skip_if_fixture_missing("service_imports_corpus_a_main_file_into_store", &[&a]) {
        return;
    }

    let report = import_files(&mut store, &mut parsed, &[spec(a.clone(), "A-招标文件.docx", "招标文件")]);

    assert_eq!(report.item_failures, 0);
    assert_eq!(report.new_records, 1);
    assert_eq!(store.files().len(), 1);
    let f = &store.files()[0];
    assert_eq!(f.parse_status, ParseStatus::Ok);
    assert!(f.tables >= 5, "corpus_probe 硬口径：A 件表格 >= 5，实际 {}", f.tables);
    assert!(f.chunks > 0);
    assert_eq!(store.project().unwrap().status, ProjectStatus::FilesImported);
    assert_eq!(store.content_version(), 2, "create=1，导入=2");
    assert_eq!(parsed.len(), 1, "成功件入解析累积");
    assert_eq!(parsed[0].sha256, f.sha256, "累积面与落库面同源（sha256 一致）");
}

/// 不可读 upload_ref：无文件身份（无 sha）→ 不落库（空 sha 会污染幂等键
/// 空间）、逐件失败、批次不中止（R-01）。
#[test]
fn unreadable_upload_is_item_failure_and_not_recorded() {
    let mut store = DomainStore::new();
    store.create_project("p", "c", "明标", "ext-svc-ghost").unwrap();
    let mut parsed = vec![];
    let good = fixtures().join("corpus-R/D6-revisions/R6b-纯删除澄清.docx");
    if skip_if_fixture_missing("unreadable_upload_is_item_failure_and_not_recorded", &[&good]) {
        return;
    }
    let ghost = fixtures().join("no-such/ghost.docx");

    let report = import_files(
        &mut store,
        &mut parsed,
        &[spec(good, "R6b-纯删除澄清.docx", "补遗"), spec(ghost, "ghost.docx", "附件")],
    );

    assert_eq!(report.item_failures, 1);
    assert_eq!(store.files().len(), 1, "只有好件落库");
    assert!(store.files().iter().all(|f| !f.sha256.is_empty()), "不得以空 sha 落档");
    assert_eq!(parsed.len(), 1);
    // 无身份件在 per_file 中 file_id 为 None（对账面可见）
    assert!(report.per_file.iter().any(|o| o.file_id.is_none()));
}

/// kind 分层：契约七类枚举校验属 MCP 壳——服务层透传（运行器「证据」
/// 类别走同管线，不因枚举被拒）。
#[test]
fn service_passes_kind_through_without_contract_enum() {
    let mut store = DomainStore::new();
    store.create_project("p", "c", "明标", "ext-svc-kind").unwrap();
    let mut parsed = vec![];
    let ev = fixtures().join("corpus-A/files/evidence/EV-01-营业执照.pdf");
    if skip_if_fixture_missing("service_passes_kind_through_without_contract_enum", &[&ev]) {
        return;
    }
    // PDF 导入依赖外部工具链（pdftotext）——无工具环境按 SKIP 机制跳过。
    if bid_parse::env_preflight::skip_if_env_missing(
        "service_passes_kind_through_without_contract_enum", &["pdftotext"]) {
        return;
    }

    let report = import_files(&mut store, &mut parsed, &[spec(ev, "evidence/EV-01-营业执照.pdf", "证据")]);

    assert_eq!(report.item_failures, 0);
    assert_eq!(store.files()[0].kind, "证据", "服务层不重复契约枚举（分层：壳校验、层透传）");
}

/// BidService 聚合面：store / parsed 同一服务态内可见（MCP dispatch_with
/// 的注入体），import 方法与自由函数同源。
#[test]
fn bid_service_holds_store_and_parsed_together() {
    let mut svc = BidService::new();
    assert!(svc.store().project().is_err());
    svc.store_mut().create_project("p", "c", "暗标", "ext-svc-bid").unwrap();
    let d1a = fixtures().join("corpus-R/D1-tables/D1a-合并表头跨页表.docx");
    if skip_if_fixture_missing("bid_service_holds_store_and_parsed_together", &[&d1a]) {
        return;
    }

    let report = svc.import(&[spec(d1a, "D1a-合并表头跨页表.docx", "招标文件")]).unwrap();

    assert_eq!(report.item_failures, 0);
    assert_eq!(svc.store().files().len(), 1);
    assert_eq!(svc.parsed().len(), 1);
    assert_eq!(svc.store().files()[0].chunks, svc.parsed()[0].blocks.len());
}
