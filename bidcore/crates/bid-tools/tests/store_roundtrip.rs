//! T-8 持久化契约测试：重启续跑验收（issue #11 批次 v3 / V3-1）。
//!
//! 验收剧本（Lead 口径）：「建项 → 导入 → 抽条款 → 重启进程 → 状态完整恢复」——
//! 本文件以两个 `BidService` 生命周期模拟进程重启（进程内隔离等价：磁盘是唯一
//! 传递介质，内存态零参与）。
//!
//! 覆盖面：
//! - 全集合恢复保真（project/files/requirements/evidences/sections/bindings/
//!   idempotency/id_seq/content_version + parsed 解析产物）；
//! - id 序列连续性（重启后新签 id 不与既有记录碰撞——水位校验的消费面）；
//! - sha256 幂等跨重启仍生效（重导入 → deduplicated）；
//! - 损坏 db.json → open 响亮拒绝（**不**静默空态重启——数据失踪不得伪装成首启）；
//! - 落盘确定性（同状态双 checkpoint 字节一致——断点续跑对账地基）。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineSpec;
use bid_domain::ProjectStatus;
use bid_tools::service::{BidService, ImportSpec};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
        });
    // #183 围栏：词法 `..` 会被围栏预拒——受信夹具根统一 canonical 绝对路径
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn spec(path: PathBuf, filename: &str, kind: &str) -> ImportSpec {
    ImportSpec { upload_ref: path, filename: filename.to_string(), kind: kind.to_string(), expected_pages: None }
}

fn store_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bid-svc-restart-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// GT-13 语义证据件（安全生产许可证——过期禁绑面在 bid-domain 已测，此处只关心持久化）。
fn evidence_spec() -> EvidenceSpec {
    EvidenceSpec {
        kind: EvidenceKind::Certificate,
        title: "安全生产许可证".into(),
        source_file: "EV-13-安全生产许可证.pdf".into(),
        source_sha256: "cafebabe13".into(),
        locator: EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:13".into() },
        fingerprint: "fnv64:13".into(),
        valid_from: Some("2022-08-01".into()),
        valid_until: Some("2025-08-01".into()),
        subject_name: "智慧园区科技有限公司".into(),
        scope: "全国".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }
}

/// **T-8 主验收**：建项 → 导入（引擎路径真件）→ 抽条款 → 证据台账 + 章节树 →
/// 「重启」→ 全状态恢复 + id 连续 + 幂等跨重启。
#[test]
fn restart_resumes_full_service_state() {
    let dir = store_dir("main");
    let a_docx = fixtures().join("corpus-A/files/A-招标文件.docx");
    // PR #24 修复批次：真实样本缺件 → SKIP 横幅（issue #9 机制文件面）
    if bid_parse::env_preflight::skip_if_fixture_missing(
        "restart_resumes_full_service_state",
        &[&a_docx],
    ) {
        return;
    }

    // ---- 生命周期 1：全链路写入（写后自动 checkpoint）
    let (snap1, parsed1, req_count, ev_id, sec_count) = {
        // #183 围栏：upload_ref 指向仓库夹具（工作区外）——声明受信导入根
        let mut svc = BidService::open(&dir).unwrap().with_fs_root(fixtures());
        svc.create_project("重启续跑项目", "ZFCG-2026-GK-0311", "明标", "restart-key").unwrap();

        let report = svc.import(&[spec(a_docx.clone(), "A-招标文件.docx", "招标文件")]).unwrap();
        assert_eq!(report.item_failures, 0, "真件导入零失败（引擎路径）");

        let reqs = bid_parse::extract::extract_requirements(svc.parsed());
        let (total, _) = svc.set_requirements(reqs).unwrap();
        assert!(total > 0, "corpus-A 主文件必须抽出条款（GT-25 面）");

        // 台账 + 章节树（domain 级操作经 store_mut + 显式 checkpoint——工具面 V3-6 接线）
        let ev_id = svc.store_mut().register_evidence(evidence_spec()).unwrap().0;
        let (sec_count, quota_total) = svc.store_mut()
            .derive_and_set_outline(OutlineSpec { target_total_chars: 60_000, max_reqs_per_leaf: 4 })
            .unwrap();
        assert!(quota_total > 0);
        svc.checkpoint().unwrap();

        (svc.store().to_snapshot(), svc.parsed().to_vec(), total, ev_id, sec_count)
    }; // <- 生命周期结束（drop：内存态消失，磁盘是唯一幸存面）

    // ---- 生命周期 2：「重启」——全部状态从 db.json 恢复
    {
        let mut svc = BidService::open(&dir).unwrap().with_fs_root(fixtures());
        let p = svc.store().project().unwrap();
        assert_eq!(p.status, ProjectStatus::RequirementsExtracted, "状态机位置恢复");
        assert_eq!(p.external_key, "restart-key");
        // 全集合快照全等（含 content_version / id_seq / 台账 / 章节树）
        assert_eq!(svc.store().to_snapshot(), snap1, "域快照全等（T-8 状态完整恢复）");
        // 解析产物随行恢复（extract 的输入面——重启后可重抽/续跑）
        assert_eq!(svc.parsed(), &parsed1[..], "解析产物全等");
        assert_eq!(svc.store().requirements().len(), req_count);
        assert_eq!(svc.store().evidences().len(), 1);
        assert_eq!(svc.store().sections().len(), sec_count);

        // id 连续性：重启后新签 id 不碰撞（id_seq 水位跨重启保真）
        let ev2 = svc.store_mut().register_evidence(EvidenceSpec {
            title: "营业执照".into(), kind: EvidenceKind::Qualification,
            source_file: "EV-01-营业执照.pdf".into(), source_sha256: "bb0001".into(),
            locator: EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:01".into() },
            fingerprint: "fnv64:01".into(), valid_from: None, valid_until: None,
            subject_name: "智慧园区科技有限公司".into(), scope: "全国".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: Default::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        }).unwrap().0;
        assert_ne!(ev2, ev_id, "重启后新 id 不与既有证据碰撞");

        // sha256 幂等跨重启：同文件重导入 → deduplicated（files 集合从磁盘恢复）
        let before = svc.store().files().len();
        let before_version = svc.store().content_version();
        let report = svc.import(&[spec(a_docx.clone(), "A-招标文件.docx", "招标文件")]).unwrap();
        assert_eq!(report.new_records, 0, "重导入 no-op");
        assert!(report.per_file[0].deduplicated, "sha256 幂等跨重启仍生效");
        assert_eq!(svc.store().files().len(), before);
        assert_eq!(svc.store().content_version(), before_version, "no-op 不 bump");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// 损坏 db.json → open 响亮拒绝（Corrupt）——**不得**静默降级为首启空态
/// （数据失踪伪装成新开始 = 最危险的静默失败）。
#[test]
fn corrupt_db_rejected_loudly_not_fresh_start() {
    let dir = store_dir("corrupt");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("db.json"), b"{ torn write").unwrap();
    match BidService::open(&dir) {
        Err(bid_tools::store_io::StoreIoError::Corrupt(_)) => (), // 预期：响亮拒绝
        Err(other) => panic!("损坏库必须 Corrupt 拒绝，实际: {other}"),
        Ok(_) => panic!("损坏库必须拒绝，不得成功打开"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 「重启」后同状态双 checkpoint 字节一致（对账/审计地基——run_tests T13 口径）。
#[test]
fn double_checkpoint_after_restart_is_byte_identical() {
    let dir = store_dir("det");
    {
        let mut svc = BidService::open(&dir).unwrap();
        svc.create_project("确定性", "C-DET", "暗标", "det-key").unwrap();
        // 写后已自动 checkpoint 一次
    }
    let path = dir.join("db.json");
    let b1 = std::fs::read(&path).unwrap();
    {
        let svc = BidService::open(&dir).unwrap();
        // 重启后无变更再显式 checkpoint
        svc.checkpoint().unwrap();
    }
    let b2 = std::fs::read(&path).unwrap();
    assert_eq!(b1, b2, "重启后同状态双 checkpoint 必须字节一致（BTreeMap 序确定）");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 绑定与翻转的持久化：EXPIRED 裁决结果跨重启保持（重开不复活——只降不升的存储面）。
#[test]
fn expired_status_survives_restart_no_revival() {
    let dir = store_dir("expired");
    {
        let mut svc = BidService::open(&dir).unwrap();
        svc.create_project("过期面", "C-EXP", "明标", "exp-key").unwrap();
        let ev_id = svc.store_mut().register_evidence(evidence_spec()).unwrap().0;
        svc.store_mut().apply_validity("2026-11-15").unwrap();
        assert_eq!(svc.store().evidence(&ev_id).unwrap().status,
                   bid_domain::evidence::EvidenceStatus::Expired);
        svc.checkpoint().unwrap();
    }
    {
        let svc = BidService::open(&dir).unwrap();
        let ev = svc.store().evidences()[0].clone();
        assert_eq!(ev.status, bid_domain::evidence::EvidenceStatus::Expired,
                   "EXPIRED 裁决跨重启保持（重开 ≠ 复活）");
        assert_eq!(ev.valid_until.as_deref(), Some("2025-08-01"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// #184 补遗逐件确认的持久化：确认记录跨「重启」往返保真（写后自动
/// checkpoint → 重开可查），旧快照（无 addendum_waivers 字段的 #184 前库）
/// 经 serde(default) 以空集合加载——加性迁移，老库不拒载。
#[test]
fn addendum_waiver_roundtrip_and_old_snapshot_default_load() {
    // ---- 生命周期 1：坏补遗 + 逐件确认（record_addendum_waiver 写后自动落盘）
    let dir = store_dir("addendum-waiver");
    let fid = {
        let mut svc = BidService::open(&dir).unwrap();
        svc.create_project("补遗确认持久化", "C-AW", "明标", "aw-key").unwrap();
        let (fid, _) = svc
            .store_mut()
            .upsert_file(
                "C-补遗01.pdf", "补遗", "sha-aw-rt", 0, 0, 0, 0,
                bid_domain::ParseStatus::Failed, "ToUnicode CMap 缺失：文本层无法解码",
            )
            .unwrap();
        // #182：宿主签发 receipt（签域绑定 file_id）→ 服务层验签过免责面
        let receipt = svc.issue_approval_receipt("rt-lead", &format!("addendum_waiver:{fid}"));
        let (rec, principal) = svc
            .record_addendum_waiver(&fid, "已人工核对纸质原件", &receipt)
            .unwrap();
        assert_eq!(rec.file_id, fid);
        assert!(rec.at > 0, "确认时刻落钟");
        // #182：回执身份（服务层验签产物）+ 记录内的 trusted 都在座
        assert_eq!(principal.actor_id, "rt-lead");
        let t = rec.trusted.as_ref().expect("新记录必有 trusted");
        assert_eq!(t.actor_id, "rt-lead");
        fid
    }; // <- drop：record_addendum_waiver 已 auto_checkpoint，磁盘是唯一幸存面

    // ---- 生命周期 2：重开 → 确认记录保真可查（验收 C 的持久化面）
    {
        let svc = BidService::open(&dir).unwrap();
        let ws = svc.store().addendum_waivers();
        assert_eq!(ws.len(), 1, "确认记录跨重启保真");
        assert_eq!(ws[0].file_id, fid);
        assert_eq!(ws[0].reason, "已人工核对纸质原件");
        assert_eq!(ws[0].principal, "human:rt-lead");
        // #182：trusted 身份跨重启保真（快照序列化面）
        assert_eq!(
            ws[0].trusted.as_ref().map(|t| t.actor_id.as_str()),
            Some("rt-lead")
        );
        assert!(svc.store().addendum_waived(&fid), "逐件查询面跨重启可用");
    }
    let _ = std::fs::remove_dir_all(&dir);

    // ---- 旧快照兼容：#184 前的库无 addendum_waivers 字段 → serde(default)
    //      空集合加载（不拒载、不误报——加性迁移纪律，schema_version 不 bump）
    let mut s = bid_domain::DomainStore::new();
    s.create_project("旧快照", "C-OLD", "明标", "old-key").unwrap();
    s.upsert_file(
        "A-招标文件.docx", "招标文件", "sha-old", 1, 0, 5, 0,
        bid_domain::ParseStatus::Ok, "",
    )
    .unwrap();
    let mut json = serde_json::to_value(s.to_snapshot()).unwrap();
    json.as_object_mut().unwrap().remove("addendum_waivers");
    let old: bid_domain::StoreSnapshot =
        serde_json::from_value(json).expect("旧快照（缺字段）必须可载");
    assert!(old.addendum_waivers.is_empty(), "缺字段 → serde(default) 空集合");
    let restored = bid_domain::DomainStore::restore(old).unwrap();
    assert!(restored.addendum_waivers().is_empty(), "恢复后查询面为空（而非报错）");
}

/// #182 审批密钥文件：首启生成（32 字节 / 0600）→ 重启复用（跨进程身份
/// 连续——旧 receipt 在重开后的服务态仍可验，24h 窗内）。
#[test]
fn approval_key_file_persists_and_receipt_survives_reopen() {
    let dir = store_dir("approval-key");
    let receipt = {
        let mut svc = BidService::open(&dir).unwrap();
        svc.issue_approval_receipt("admin", "review_gate:pre_export_confirm")
    };
    {
        let svc = BidService::open(&dir).unwrap();
        // 密钥文件在座：恰 32 字节、unix 权限 0600（同组/其他不可读）
        let key_path = dir.join(".mang").join("approval.key");
        let meta = std::fs::metadata(&key_path).expect("approval.key 首启生成");
        assert_eq!(meta.len(), 32, "密钥恒 32 字节");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                meta.permissions().mode() & 0o777,
                0o600,
                "密钥文件权限 0600"
            );
        }
        // 重启后同钥：首启签发的 receipt 仍可验（重开不换钥 = 在途确认不作废）
        let p = svc
            .verify_approval_receipt(&receipt, "review_gate:pre_export_confirm")
            .expect("跨重启同钥：receipt 仍可验");
        assert_eq!(p.actor_id, "admin");
        assert_eq!(p.actor_type, bid_domain::ActorType::Human);
        // 签域不符：同 receipt 过不了别的门
        assert!(svc
            .verify_approval_receipt(&receipt, "review_gate:matrix_approval")
            .is_err());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
