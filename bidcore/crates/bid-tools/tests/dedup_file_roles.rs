//! dedup_file_roles.rs —— #188 同 SHA 去重业务角色分离的验收契约。
//!
//! 问题现场（issue 原文）：第一次以「附件」导入的文件，第二次以「补遗」导入
//! （同 SHA）→ 命中去重返回原记录，新 kind/文件名**静默丢失** → 补遗识别
//! （readiness 的 is_addendum_file）看不到它 → 补遗就绪/阻塞判定（#184 链路）失明。
//!
//! 修法：内容身份（sha256 幂等键）与业务角色（FileRole）分离——角色追加不
//! 覆盖，判定面看任一身份；冲突显式报告（METADATA_CONFLICT 告警不失败）。
//!
//! 验收对照：
//! | 用例 | 断言 |
//! |---|---|
//! | A. 同文件先「附件」后「补遗」导入 | 两个身份都在，补遗识别可见，L3 门放行 |
//! | B. 完全相同的重放（同 SHA 同 kind 同名） | 幂等 no-op，无告警，不增角色 |
//! | C. 同 SHA 不同 kind | METADATA_CONFLICT 报告，但不失败 |

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, FileRole, ParseStatus, Requirement, StoreSnapshot};
use bid_parse::env_preflight::skip_if_fixture_missing;
use bid_tools::readiness::{
    addendum_statuses, check_draft_readiness, AddendumIssue, AddendumReadiness, Blocker,
};
use bid_tools::service::{import_files, ImportSpec};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // crates/bid-tools → 仓库根
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
        });
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn spec(path: PathBuf, filename: &str, kind: &str) -> ImportSpec {
    ImportSpec { upload_ref: path, filename: filename.to_string(), kind: kind.to_string(), expected_pages: None }
}

/// L3 门测试基座：招标文件 + 1 条款 + 1 叶章节（矩阵已建）——复制
/// readiness_l3_addendum.rs 的 base_store 形态（跨文件不可 import，按需最小化）。
fn base_store_with_l3_gate() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("去重角色测试", "ZFCG-2026-188-001", "明标", "dedup-key").unwrap();
    s.upsert_file("A-招标文件.docx", "招标文件", "sha-tender-188", 120, 8, 900, 0,
        ParseStatus::Ok, "engine-docx @ test").unwrap();
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
    }]).unwrap();
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
    }]).unwrap();
    s.build_matrix().unwrap();
    s
}

// ---------------------------------------------------------------- 域层单元面

/// register_file_role：孤儿拒登记 / 幂等 no-op / 追加不覆盖 / 不碰 content_version。
#[test]
fn register_file_role_orphan_idempotent_and_version_neutral() {
    let mut s = DomainStore::new();
    s.create_project("p", "ZFCG-2026-188-U", "明标", "dedup-unit").unwrap();
    let (fid, _) = s
        .upsert_file("a.pdf", "附件", "sha-u1", 3, 1, 20, 0, ParseStatus::Ok, "ok")
        .unwrap();
    // 孤儿角色 → 拒（凭空身份）
    assert!(s.register_file_role("TF-9999", "补遗", "x.pdf", 0).is_err(), "孤儿角色必须拒");
    // 首个角色 → 落账
    s.register_file_role(&fid, "补遗", "补遗-03.pdf", 100).unwrap();
    assert_eq!(s.file_roles(&fid).len(), 1);
    // 完全相同的重放 → 幂等 no-op（不重复登记；at 不被覆写）
    let replay = s.register_file_role(&fid, "补遗", "补遗-03.pdf", 999).unwrap();
    assert_eq!(replay.at, 100, "幂等重放返回既有角色（at 不覆写）");
    assert_eq!(s.file_roles(&fid).len(), 1, "不重复登记");
    // 不同身份 → 追加（不覆盖）
    s.register_file_role(&fid, "补遗", "更正公告.pdf", 200).unwrap();
    assert_eq!(s.file_roles(&fid).len(), 2, "角色追加不覆盖");
    // content_version 不动（角色是元数据面——upsert 时已 bump，不重复计）
    let v = s.content_version();
    s.register_file_role(&fid, "图纸", "图-01.dwg", 300).unwrap();
    assert_eq!(s.content_version(), v, "角色登记不碰 content_version");
}

// ---------------------------------------------------------------- 服务层验收 A/B/C

/// 验收 A+C：同内容先「附件」后「补遗」导入——文件不重复存（幂等键空间
/// 不变）、原记录不覆盖、新角色登记、冲突显式告警（不失败）。
#[test]
fn same_sha_reimport_as_addendum_registers_role_with_conflict_warning() {
    let mut store = DomainStore::new();
    store.create_project("p", "ZFCG-2026-188-A", "明标", "dedup-a").unwrap();
    let mut parsed = vec![];
    let path = fixtures().join("corpus-R/D6-revisions/R6b-纯删除澄清.docx");
    if skip_if_fixture_missing("dedup_roles_acceptance_a", &[&path]) {
        return;
    }

    // 首次导入：附件
    let r1 = import_files(&mut store, &mut parsed, &[spec(path.clone(), "tender-attachment.pdf", "附件")], None);
    assert_eq!(r1.item_failures, 0);
    assert_eq!(r1.new_records, 1);
    assert_eq!(store.files().len(), 1);
    let fid = store.files()[0].id.clone();
    let version_after_first = store.content_version();

    // 二次导入：同 SHA、不同 kind + 不同名（补遗）——验收 A / C
    let r2 = import_files(&mut store, &mut parsed, &[spec(path, "补遗-03.pdf", "补遗")], None);
    assert_eq!(r2.item_failures, 0, "同 SHA 异身份导入不失败（验收 C：告警不失败）");
    assert_eq!(r2.new_records, 0);
    let o = &r2.per_file[0];
    assert_eq!(o.file_id.as_deref(), Some(fid.as_str()), "返回原 file_id（幂等键空间不变）");
    assert!(o.deduplicated, "deduplicated 语义不变");
    assert!(
        o.warnings.iter().any(|w| w.contains("METADATA_CONFLICT")
            && w.contains("tender-attachment.pdf") && w.contains("补遗-03.pdf")),
        "冲突显式报告（新旧身份都点名）：{:?}",
        o.warnings
    );
    // 内容身份唯一 + 原记录不覆盖 + 角色登记
    assert_eq!(store.files().len(), 1, "同 SHA 不重复存文件（幂等语义保持）");
    assert_eq!(store.files()[0].kind, "附件", "原记录 kind 不被覆盖");
    let roles = store.file_roles(&fid);
    assert_eq!(roles.len(), 1, "新身份登记为 FileRole（追加不覆盖）");
    assert_eq!(roles[0].kind, "补遗");
    assert_eq!(roles[0].declared_name, "补遗-03.pdf");
    // 角色登记不碰 content_version（元数据面——与 upsert 的内容 bump 分离）
    assert_eq!(store.content_version(), version_after_first);

    // 补遗识别看得到它（验收 A 的判定面）：报告名与判定身份一致
    let statuses = addendum_statuses(&store);
    let st = statuses.iter().find(|s| s.file_id == fid)
        .expect("补遗判定必须看到该文件（#188 修复目标）");
    assert_eq!(st.name, "补遗-03.pdf", "报告名 = 触发判定的身份名");
    assert_eq!(st.readiness, AddendumReadiness::Ready, "docx 解析 Ok → Ready");
}

/// 验收 B：完全相同的重放（同 SHA 同 kind 同名）→ 幂等 no-op，无告警、
/// 不增角色、不动版本。
#[test]
fn identical_reimport_is_pure_noop_without_warning() {
    let mut store = DomainStore::new();
    store.create_project("p", "ZFCG-2026-188-B", "明标", "dedup-b").unwrap();
    let mut parsed = vec![];
    let path = fixtures().join("corpus-R/D6-revisions/R6b-纯删除澄清.docx");
    if skip_if_fixture_missing("dedup_roles_acceptance_b", &[&path]) {
        return;
    }

    import_files(&mut store, &mut parsed, &[spec(path.clone(), "tender-attachment.pdf", "附件")], None);
    let fid = store.files()[0].id.clone();
    let version = store.content_version();

    let r2 = import_files(&mut store, &mut parsed, &[spec(path, "tender-attachment.pdf", "附件")], None);
    assert_eq!(r2.item_failures, 0);
    let o = &r2.per_file[0];
    assert!(o.deduplicated);
    assert!(o.warnings.is_empty(), "完全相同的重放无告警（验收 B）：{o:?}");
    assert_eq!(store.file_roles(&fid).len(), 0, "不新增角色");
    assert_eq!(store.content_version(), version, "纯 no-op 不动版本");
}

/// 验收 A 的 L3 门闭环：先「附件」导入的文件再以「补遗」身份导入后，
/// MissingAddendum(NotProvided) 阻塞消失（补遗就绪判定看得到它）。
#[test]
fn addendum_role_release_l3_not_provided_blocker() {
    let mut store = base_store_with_l3_gate();
    // 前置：无补遗 → NotProvided 阻塞（fail-closed 面）
    let r0 = check_draft_readiness(&store, "SEC-0001");
    match r0.blockers.iter().find(|b| matches!(b, Blocker::MissingAddendum { .. })) {
        Some(Blocker::MissingAddendum { reason, .. }) => {
            assert_eq!(*reason, AddendumIssue::NotProvided);
        }
        _ => panic!("无补遗必须阻塞 NotProvided：{:?}", r0.blockers),
    }

    let mut parsed = vec![];
    let path = fixtures().join("corpus-R/D6-revisions/R6b-纯删除澄清.docx");
    if skip_if_fixture_missing("dedup_roles_l3_gate", &[&path]) {
        return;
    }
    // 同一份内容：先附件、后补遗（#188 事故形态的忠实重演）
    import_files(&mut store, &mut parsed, &[spec(path.clone(), "tender-attachment.pdf", "附件")], None);
    let r1 = check_draft_readiness(&store, "SEC-0001");
    assert!(
        r1.blockers.iter().any(|b| matches!(b, Blocker::MissingAddendum { reason, .. }
            if *reason == AddendumIssue::NotProvided)),
        "附件身份不算补遗（阻塞仍在——不虚构）：{:?}",
        r1.blockers
    );
    import_files(&mut store, &mut parsed, &[spec(path, "补遗-03.pdf", "补遗")], None);
    let r2 = check_draft_readiness(&store, "SEC-0001");
    assert!(
        !r2.blockers.iter().any(|b| matches!(b, Blocker::MissingAddendum { .. })),
        "补遗角色登记后 L3 门必须看到它（Ready → 放行）：{:?}",
        r2.blockers
    );
}

// ---------------------------------------------------------------- 快照兼容面

/// 角色随快照走 + 旧快照（无 file_roles 字段）加性加载 + 孤儿角色拒载。
#[test]
fn file_roles_snapshot_roundtrip_and_legacy_compat() {
    let mut s = DomainStore::new();
    s.create_project("p", "ZFCG-2026-188-S", "明标", "dedup-snap").unwrap();
    let (fid, _) = s
        .upsert_file("a.pdf", "附件", "sha-snap", 3, 1, 20, 0, ParseStatus::Ok, "ok")
        .unwrap();
    s.register_file_role(&fid, "补遗", "补遗-03.pdf", 42).unwrap();

    // roundtrip：角色存活
    let restored = DomainStore::restore(s.to_snapshot()).unwrap();
    let roles = restored.file_roles(&fid);
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].kind, "补遗");
    assert_eq!(roles[0].at, 42);

    // 旧快照兼容（#147 加性迁移先例）：剥掉 file_roles 字段仍可载
    //（角色为空 → 判定面回退 legacy kind/name，行为与旧版一致）
    let json = serde_json::to_value(s.to_snapshot()).unwrap();
    let mut old = json.clone();
    old.as_object_mut().unwrap().remove("file_roles");
    let old_snap: StoreSnapshot = serde_json::from_value(old)
        .expect("旧快照（缺 file_roles 字段）必须可载（serde default）");
    let old_store = DomainStore::restore(old_snap).unwrap();
    assert!(old_store.file_roles(&fid).is_empty(), "旧快照无角色 → 空集合");

    // 孤儿角色 → 拒载（fail-closed：对不存在的件声明身份 = 半个账本）
    let mut bad = s.to_snapshot();
    bad.file_roles.push(FileRole {
        file_id: "TF-9999".into(),
        kind: "补遗".into(),
        declared_name: "幽灵补遗.pdf".into(),
        at: 0,
    });
    assert!(DomainStore::restore(bad).is_err(), "孤儿角色必须拒载");
}
