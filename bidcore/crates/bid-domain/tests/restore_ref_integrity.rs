//! #372 · restore 快照四类引用完整性（R-08/A-05）。
//!
//! 覆盖面（issue #372 验收 5：负例四类各至少一例 + 合法快照回归）：
//! - **合法基线**：四类数据全在场、引用全真实 → 照常恢复（回归面——
//!   既有合法快照不受影响，只有带孤儿引用的坏快照被拒）；
//! - **evidence_bindings**：幽灵证据 / 幽灵条款 / 复合键重复 → 拒载
//!   （E_UNPROCESSABLE + 报错定位实体 id）；
//! - **matrix**（存量 CMP 行）：幽灵 requirement / 幽灵 response_section /
//!   幽灵 evidence / 行 id 重复 → 拒载；
//! - **matrix_overrides**：幽灵键 / 键值分裂（serde 键值独立反序列化，
//!   手改可造）→ 拒载；
//! - **block_marks / submitted_requirements**：幽灵 file_id 键 → 拒载。
//!
//! 威胁模型：手改/半截/旧版本 db.json（restore 头注释自列）——快照面是
//! 登记面之外的第二条写入口，restore 不得比登记面宽（#224 mapping 三查
//! 同款纪律）。幽灵绑定的实害：直接进 derive_matrix_rows 的 ev_by_req →
//! 该 REQ 恒 has_ev（PARTIAL 以上）→ 资格条件不再 blocking。
//!
//! 测法：基线 store 走真实 API 构造（项目/文件/REQ/章节），四类数据在
//! snapshot 上补齐合法形态（这些字段的登记面需要服务层 receipt/HMAC 面，
//! 域层快照直塞等价于「API 写出的合法形态」）；负例 = 基线快照的副本
//! 上打脏单个字段——精确模拟「手改 db.json」。

use bid_domain::coverage::{BlockMark, BlockVerdict, SubmittedRequirement};
use bid_domain::evidence::{Evidence, EvidenceKind, EvidenceStatus};
use bid_domain::locator::EngineLocator;
use bid_domain::matrix::{MatrixOverride, MatrixRow, ResponseStatus};
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, Requirement, ToolError};
use std::collections::{BTreeMap, HashMap};

const FILE: &str = "A-招标文件.docx";

fn anchor(excerpt: &str) -> Anchor {
    Anchor {
        file: FILE.into(),
        kind: "para".into(),
        locator: "第四章 评标办法".into(),
        excerpt: excerpt.into(),
        block_idx: None,
        page: None,
        line: None,
    }
}

fn req(id: &str, category: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        // 锚须逐条区分：requirement_hash = sha256(anchors)——同锚 REQ 会被
        // set_requirements 的幂等合并视同一条（覆盖去重）。
        anchors: vec![anchor(&format!("★条款 {id}（测试合成锚）"))],
        params: HashMap::new(),
        source_file: FILE.into(),
    }
}

fn leaf(id: &str, order: u32, title: &str, reqs: &[&str]) -> OutlineNode {
    OutlineNode {
        id: id.into(),
        parent_id: None,
        title: title.into(),
        level: 1,
        order,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars: 2_000,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }
}

/// 合法证据（字段全真——不虚构缺省；locator 用最简变体 DocxParagraph）。
fn ev(id: &str) -> Evidence {
    Evidence {
        id: id.into(),
        kind: EvidenceKind::Qualification,
        title: "电子与智能化工程专业承包二级资质证书".into(),
        source_file: FILE.into(),
        source_sha256: "sha-00000000000000000000000000000000000000000000000000000000"
            .into(),
        locator: EngineLocator::DocxParagraph {
            paragraph_index: 3,
            text_fingerprint: "fnv64:test-fp".into(),
        },
        fingerprint: "fnv64:test-fp".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "测试投标人".into(),
        scope: "全部专业".into(),
        version: 0,
        status: EvidenceStatus::Active,
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }
}

fn cmp_row(id: &str, req_id: &str, sec: Option<&str>, evs: &[&str]) -> MatrixRow {
    MatrixRow {
        id: id.into(),
        requirement_id: req_id.into(),
        response_section: sec.map(|s| s.to_string()),
        evidence_ids: evs.iter().map(|s| s.to_string()).collect(),
        pending_evidence_ids: vec![],
        evidence_required: false,
        evidence_missing: false,
        substantive: false,
        status: ResponseStatus::Responded,
        deviation: None,
        blocking: false,
    }
}

fn mark(from: usize, to: usize) -> BlockMark {
    BlockMark {
        from,
        to,
        verdict: BlockVerdict::Requirement,
        reason: "测试合成标记".into(),
    }
}

fn submitted(block_idx: usize) -> SubmittedRequirement {
    SubmittedRequirement {
        block_idx,
        statement: "核心交换机须支持 48 口千兆光接口".into(),
        category: Some("技术参数".into()),
        mandatory: false,
        substantive: false,
    }
}

/// 合法基线快照：项目 + 文件 + 两 REQ + 两章节 + 四类数据全在场（引用
/// 全真实——EV-0001↔REQ-0001 绑定、CMP-0001 行、REQ-0002 覆写、
/// block_marks/submitted 挂真实文件键）。
///
/// 注意：block_marks/submitted_requirements 的键是 `TenderFile.id`
/// （TF-xxxx，set_block_marks 的存在性校验口径），不是文件名。
fn baseline_snapshot() -> bid_domain::StoreSnapshot {
    let mut s = DomainStore::new();
    s.create_project("引用完整性测试", "C-M", "明标", "m-key")
        .unwrap();
    let (file_id, _) = s
        .upsert_file(
            FILE,
            "招标文件",
            "sha-ref",
            10,
            1,
            50,
            0,
            bid_domain::ParseStatus::Ok,
            "",
        )
        .unwrap();
    s.set_requirements(vec![
        req("REQ-0001", "资格条件"),
        req("REQ-0002", "技术参数"),
    ])
    .unwrap();
    s.set_outline(vec![
        leaf("SEC-01", 1, "资格证明", &["REQ-0001"]),
        leaf("SEC-02", 2, "技术响应", &["REQ-0002"]),
    ])
    .unwrap();

    let mut snap = s.to_snapshot();
    snap.evidences = vec![ev("EV-0001")];
    snap.evidence_bindings = vec![bid_domain::evidence::EvidenceBinding {
        evidence_id: "EV-0001".into(),
        requirement_id: "REQ-0001".into(),
    }];
    snap.matrix = vec![cmp_row("CMP-0001", "REQ-0001", Some("SEC-01"), &["EV-0001"])];
    snap.matrix_overrides = HashMap::from([(
        "REQ-0002".to_string(),
        MatrixOverride {
            requirement_id: "REQ-0002".into(),
            status: ResponseStatus::NotApplicable,
            deviation: Some("不适用（测试合成覆写）".into()),
            principal: "human:tester".into(),
            trusted: None,
            at: 0,
        },
    )]);
    snap.block_marks = BTreeMap::from([(file_id.clone(), vec![mark(1, 3)])]);
    snap.submitted_requirements =
        BTreeMap::from([(file_id, vec![submitted(2)])]);
    snap
}

/// 断言拒载且错误定位到给定实体 id（fail-closed + 可追溯报错面）。
fn assert_rejected(snap: bid_domain::StoreSnapshot, expect: &str) {
    let Err(e) = DomainStore::restore(snap) else {
        panic!("带幽灵引用的快照必须拒载（期望报错含 {expect}）");
    };
    assert!(
        matches!(e, ToolError::EUnprocessable { .. }),
        "拒载错误必须是 E_UNPROCESSABLE：{e:?}"
    );
    let msg = format!("{e:?}");
    assert!(msg.contains(expect), "报错须定位实体 {expect}：{msg}");
}

// ---------------------------------------------------------------- 合法面

/// 合法基线（四类数据全在场、引用全真实）照常恢复——回归面：既有合法
/// 快照零影响，新校验只拒带孤儿引用的坏快照。恢复后往返回看四类数据
/// 保真（无访问器的三面经 to_snapshot 对照）。
#[test]
fn baseline_snapshot_with_all_four_collections_restores() {
    let snap = baseline_snapshot();
    let ok = DomainStore::restore(snap.clone()).unwrap();
    let fid = &snap.files[0].id;
    assert_eq!(ok.block_marks_of(fid).len(), 1);
    let back = ok.to_snapshot();
    assert_eq!(back.evidence_bindings, snap.evidence_bindings);
    assert_eq!(back.matrix, snap.matrix);
    assert_eq!(back.matrix_overrides, snap.matrix_overrides);
}

// ------------------------------------------------- ① evidence_bindings

/// 幽灵证据：binding 指向不存在的 EV-9999 → 拒（恢复即「已响应」假象）。
#[test]
fn restore_rejects_ghost_evidence_binding() {
    let mut bad = baseline_snapshot();
    bad.evidence_bindings[0].evidence_id = "EV-9999".into();
    assert_rejected(bad, "EV-9999");
}

/// 幽灵条款：binding 指向已删的 REQ-0009 → 拒（死数据不得进派生面）。
#[test]
fn restore_rejects_ghost_requirement_binding() {
    let mut bad = baseline_snapshot();
    bad.evidence_bindings[0].requirement_id = "REQ-0009".into();
    assert_rejected(bad, "REQ-0009");
}

/// 复合键重复：(EV-0001, REQ-0001) 出现两次 → 拒（幂等面破坏——
/// 与 scoring_mappings 复合键查重同款纪律）。
#[test]
fn restore_rejects_duplicate_evidence_binding_pair() {
    let mut bad = baseline_snapshot();
    let first = bad.evidence_bindings[0].clone();
    bad.evidence_bindings.push(first);
    assert_rejected(bad, "重复");
}

// -------------------------------------------------------- ② matrix 行

/// 幽灵 requirement：CMP 行指向不存在的 REQ-0009 → 拒（存量矩阵进
/// 导出门 1a 读面）。
#[test]
fn restore_rejects_matrix_row_with_ghost_requirement() {
    let mut bad = baseline_snapshot();
    bad.matrix[0].requirement_id = "REQ-0009".into();
    assert_rejected(bad, "REQ-0009");
}

/// 幽灵 response_section：CMP 行指向已删章节 SEC-09 → 拒。
#[test]
fn restore_rejects_matrix_row_with_ghost_section() {
    let mut bad = baseline_snapshot();
    bad.matrix[0].response_section = Some("SEC-09".into());
    assert_rejected(bad, "SEC-09");
}

/// 幽灵证据：CMP 行 evidence_ids 指向不存在的 EV-9999 → 拒。
#[test]
fn restore_rejects_matrix_row_with_ghost_evidence() {
    let mut bad = baseline_snapshot();
    bad.matrix[0].evidence_ids = vec!["EV-9999".into()];
    assert_rejected(bad, "EV-9999");
}

/// 幽灵证据（pending 面）：pending_evidence_ids 同样拒（#245 占位
/// 追踪面与有效证据面同源纪律）。
#[test]
fn restore_rejects_matrix_row_with_ghost_pending_evidence() {
    let mut bad = baseline_snapshot();
    bad.matrix[0].pending_evidence_ids = vec!["EV-9999".into()];
    assert_rejected(bad, "EV-9999");
}

/// 行 id 重复：两条 CMP-0001 → 拒（与 9 集合 id 查重同款）。
#[test]
fn restore_rejects_duplicate_matrix_row_id() {
    let mut bad = baseline_snapshot();
    let first = bad.matrix[0].clone();
    let mut second = first.clone();
    second.requirement_id = "REQ-0002".into(); // 引用真实，只造 id 撞
    bad.matrix.push(second);
    assert_rejected(bad, "重复");
}

// -------------------------------------------------- ③ matrix_overrides

/// 幽灵键：覆写挂在已删的 REQ-0009 上 → 拒（消费面 overrides.get(&r.id)
/// 永不命中，死数据无信号）。
#[test]
fn restore_rejects_ghost_matrix_override_key() {
    let mut bad = baseline_snapshot();
    let o = bad.matrix_overrides.remove("REQ-0002").unwrap();
    bad.matrix_overrides.insert("REQ-0009".into(), o);
    assert_rejected(bad, "REQ-0009");
}

/// 键值分裂：键 REQ-0002 但值内 requirement_id=REQ-0001 → 拒（serde
/// 键值独立反序列化——手改可造出的账本内部不一致）。
#[test]
fn restore_rejects_matrix_override_key_value_split() {
    let mut bad = baseline_snapshot();
    bad.matrix_overrides
        .get_mut("REQ-0002")
        .unwrap()
        .requirement_id = "REQ-0001".into();
    assert_rejected(bad, "键值分裂");
}

// ------------------------------------- ④ block_marks / submitted 键

/// 孤儿 block_marks：键指向不存在的文件（TF id 口径）→ 拒（覆盖率统计
/// 口径污染）。
#[test]
fn restore_rejects_ghost_block_marks_file_key() {
    let mut bad = baseline_snapshot();
    let fid = bad.files[0].id.clone();
    let marks = bad.block_marks.remove(&fid).unwrap();
    bad.block_marks.insert("TF-9999".into(), marks);
    assert_rejected(bad, "TF-9999");
}

/// 孤儿 submitted_requirements：键指向不存在的文件 → 拒（合并面口径）。
#[test]
fn restore_rejects_ghost_submitted_file_key() {
    let mut bad = baseline_snapshot();
    let fid = bad.files[0].id.clone();
    let subs = bad.submitted_requirements.remove(&fid).unwrap();
    bad.submitted_requirements.insert("TF-9999".into(), subs);
    assert_rejected(bad, "TF-9999");
}
