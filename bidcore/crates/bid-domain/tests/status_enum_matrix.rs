//! 域内四个状态枚举的全覆盖矩阵——Evidence / Section / Response / Parse。
//!
//! 为什么单开：`state_machine_matrix.rs` 只覆盖 ProjectStatus（项目级状态机，
//! 有 next_allowed/allowed_tools 两张权威表）。域内还有四个**子状态机**，
//! 它们没有显式的迁移表，状态判定散在各自的 impl 里：
//!
//! | 枚举 | 位置 | 语义 |
//! |---|---|---|
//! | `EvidenceStatus` | evidence.rs | Active / Expiring / Expired / Superseded |
//! | `SectionStatus` | outline.rs | Pending / Outlining / Drafting / Failed / Drafted / Validating / Revising / Accepted |
//! | `ResponseStatus` | matrix.rs | NotResponded / Responded / Partial / NotApplicable |
//! | `ParseStatus` | lib.rs | Ok / OcrRequired / Failed（推测） |
//!
//! 没有穷举测试时，最容易出的事是**加变体忘改判定分支**——
//! 比如给 EvidenceStatus 加一个 `Archived`，某处 `match` 用了 `_ =>` 兜底，
//! 于是新状态被静默当成旧状态处理。
//!
//! 本文件的三类断言：
//! 1. **往返稳定**：serde 序列化 → 反序列化 = 原值（持久化面）
//! 2. **展示串唯一**：所有变体的 as_str 互不相同（映射面）
//! 3. **语义谓词全覆盖**：如 `can_bind()` / `next_allowed()` 对每个变体有确定答案

use bid_domain::evidence::EvidenceStatus;
use bid_domain::matrix::ResponseStatus;
use bid_domain::outline::SectionStatus;
use bid_domain::ParseStatus;
use serde::{de::DeserializeOwned, Serialize};

/// serde 往返：Serialize → Deserialize 必须等于原值。
fn roundtrip<T>(v: &T) -> T
where
    T: Serialize + DeserializeOwned,
{
    let s = serde_json::to_string(v).expect("序列化失败");
    serde_json::from_str(&s).expect("反序列化失败")
}

// ---------------------------------------------------------------- EvidenceStatus

const EVIDENCE_ALL: [EvidenceStatus; 4] = [
    EvidenceStatus::Active,
    EvidenceStatus::Expiring,
    EvidenceStatus::Expired,
    EvidenceStatus::Superseded,
];

#[test]
fn evidence_status_roundtrip() {
    for s in EVIDENCE_ALL {
        let back = roundtrip(&s);
        assert_eq!(back, s, "EvidenceStatus::{s:?} serde 往返不相等");
    }
}

#[test]
fn evidence_status_display_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for s in EVIDENCE_ALL {
        let lit = s.as_str().to_string();
        assert!(!lit.is_empty(), "EvidenceStatus::{s:?} 展示串为空");
        assert!(seen.insert(lit.clone()), "展示串 {lit} 重复（两变体同串 → 持久化歧义）");
    }
    // 钉死字面量（MCP 契约/盘上快照靠它分支）
    assert_eq!(EvidenceStatus::Active.as_str(), "ACTIVE");
    assert_eq!(EvidenceStatus::Expiring.as_str(), "EXPIRING");
    assert_eq!(EvidenceStatus::Expired.as_str(), "EXPIRED");
    assert_eq!(EvidenceStatus::Superseded.as_str(), "SUPERSEDED");
}

/// 「可用性」谓词必须对每个变体有确定答案——EXPIRED / SUPERSEDED 不可绑，
/// 其余可用。这条钉的是绑定门的语义边界。
#[test]
fn evidence_can_bind_predicate_is_total() {
    // 绑定门的语义边界（evidence.rs::can_bind 的文档承诺）：
    assert!(EvidenceStatus::Active.can_bind(), "ACTIVE 应可绑");
    assert!(
        EvidenceStatus::Expiring.can_bind(),
        "EXPIRING 应可绑（临期仅告警，不拦绑定）"
    );
    assert!(!EvidenceStatus::Expired.can_bind(), "EXPIRED 禁绑（R-03）");
    assert!(
        !EvidenceStatus::Superseded.can_bind(),
        "SUPERSEDED 禁绑（已被新版取代）"
    );
}

// ---------------------------------------------------------------- SectionStatus

const SECTION_ALL: [SectionStatus; 8] = [
    SectionStatus::Pending,
    SectionStatus::Outlining,
    SectionStatus::Drafting,
    SectionStatus::Failed,
    SectionStatus::Drafted,
    SectionStatus::Validating,
    SectionStatus::Revising,
    SectionStatus::Accepted,
];

#[test]
fn section_status_roundtrip() {
    for s in SECTION_ALL {
        let back = roundtrip(&s);
        assert_eq!(back, s, "SectionStatus::{s:?} serde 往返不相等");
    }
}

#[test]
fn section_status_display_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for s in SECTION_ALL {
        let lit = s.as_str().to_string();
        assert!(!lit.is_empty(), "SectionStatus::{s:?} 展示串为空");
        assert!(seen.insert(lit.clone()), "展示串 {lit} 重复");
    }
}

/// 章节前向链的性质（`SectionStatus::next_allowed` 是章节级状态机的真源）。
///
/// 与项目级 ProjectStatus 不同，章节链**不是纯线性**——Drafting 有两个
/// 后继（Drafted / Failed），Validating 有两个（Revising / Accepted）。
/// 这两处分支是有意的失败/免空转建模，钉死它们防后人「顺手线性化」。
#[test]
fn section_forward_chain_shape() {
    use SectionStatus::*;
    // 线性段
    assert_eq!(Pending.next_allowed(), &[Outlining]);
    assert_eq!(Outlining.next_allowed(), &[Drafting]);
    assert_eq!(Drafted.next_allowed(), &[Validating]);
    assert_eq!(Revising.next_allowed(), &[Accepted]);
    // 分支点 1：Drafting → Drafted | Failed（失败是一等结果）
    let from_drafting = Drafting.next_allowed();
    assert_eq!(from_drafting.len(), 2, "Drafting 必须有两个后继（成功/失败）");
    assert!(from_drafting.contains(&Drafted));
    assert!(from_drafting.contains(&Failed));
    // 分支点 2：Validating → Revising | Accepted（零发现直通验收）
    let from_validating = Validating.next_allowed();
    assert_eq!(from_validating.len(), 2, "Validating 必须有两个后继");
    assert!(from_validating.contains(&Revising));
    assert!(from_validating.contains(&Accepted));
    // Failed 的回退口：只能 Drafting（不回退 Pending）
    assert_eq!(Failed.next_allowed(), &[Drafting]);
    // 终态
    assert!(Accepted.next_allowed().is_empty(), "Accepted 是终态");
}

/// 章节链必须把 8 个变体全部覆盖（无不可达、无死胡同）。
#[test]
fn section_chain_covers_all_variants() {
    let mut reachable = vec![SectionStatus::Pending];
    let mut i = 0;
    while i < reachable.len() {
        for &n in reachable[i].next_allowed() {
            if !reachable.contains(&n) {
                reachable.push(n);
            }
        }
        i += 1;
    }
    for s in SECTION_ALL {
        assert!(
            reachable.contains(&s),
            "SectionStatus::{s:?} 从 Pending 不可达（孤儿变体）"
        );
    }
}

// ---------------------------------------------------------------- ResponseStatus

const RESPONSE_ALL: [ResponseStatus; 4] = [
    ResponseStatus::NotResponded,
    ResponseStatus::Responded,
    ResponseStatus::Partial,
    ResponseStatus::NotApplicable,
];

#[test]
fn response_status_roundtrip() {
    for s in RESPONSE_ALL {
        let back = roundtrip(&s);
        assert_eq!(back, s, "ResponseStatus::{s:?} serde 往返不相等");
    }
}

#[test]
fn response_status_display_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for s in RESPONSE_ALL {
        let lit = s.as_str().to_string();
        assert!(!lit.is_empty(), "ResponseStatus::{s:?} 展示串为空");
        assert!(seen.insert(lit.clone()), "展示串 {lit} 重复");
    }
}

/// 必选项判定（`matrix::is_mandatory`）——参数级，不是状态级。
///
/// 澄清一个易混点：ResponseStatus **没有** is_blocking()。矩阵的阻塞判定
/// 是「REQ 是必选项（资格条件/否决项）AND 该行 NotResponded」两件套，
/// 不是单靠状态枚举能决定的。本测试钉的是必选项那一半。
#[test]
fn mandatory_category_predicate() {
    use bid_domain::matrix::is_mandatory;
    assert!(is_mandatory("资格条件"), "资格条件必选");
    assert!(is_mandatory("否决项"), "否决项必选");
    // 非必选面：不阻塞导出
    for c in ["技术参数", "商务要求", "格式要求", "其他", "评分项"] {
        assert!(!is_mandatory(c), "{c} 不该是必选项");
    }
}

// ---------------------------------------------------------------- ParseStatus

const PARSE_ALL: [ParseStatus; 3] = [
    ParseStatus::Ok,
    ParseStatus::OcrRequired,
    ParseStatus::Failed,
];

#[test]
fn parse_status_roundtrip() {
    for s in PARSE_ALL {
        let back = roundtrip(&s);
        assert_eq!(back, s, "ParseStatus::{s:?} serde 往返不相等");
    }
}

#[test]
fn parse_status_display_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for s in PARSE_ALL {
        let lit = s.as_str().to_string();
        assert!(!lit.is_empty(), "ParseStatus::{s:?} 展示串为空");
        assert!(seen.insert(lit.clone()), "展示串 {lit} 重复");
    }
}

// ---------------------------------------------------------------- 跨枚举一致性

/// 四个枚举的 serde 表示都必须是 **SCREAMING_SNAKE_CASE 字面量**（不是数字索引）。
///
/// 索引会随变体插入顺序漂移——盘上快照/契约文本都不该依赖顺序。
#[test]
fn all_status_serialize_as_string_literals() {
    for s in EVIDENCE_ALL {
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.starts_with('"'), "EvidenceStatus::{s:?} 序列化成非字符串 {j}");
    }
    for s in SECTION_ALL {
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.starts_with('"'), "SectionStatus::{s:?} 序列化成非字符串 {j}");
    }
    for s in RESPONSE_ALL {
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.starts_with('"'), "ResponseStatus::{s:?} 序列化成非字符串 {j}");
    }
    for s in PARSE_ALL {
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.starts_with('"'), "ParseStatus::{s:?} 序列化成非字符串 {j}");
    }
}
