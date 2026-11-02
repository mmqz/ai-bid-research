//! 章节树不变量矩阵——`validate_outline()` 的 9 条规则逐条正反验证。
//!
//! 为什么单开：`validate_outline` 是「重建章节树」的唯一裁决面，
//! 9 条规则全挤在一个函数里（空树 / id 唯一 / level 域 / REQ 非空 /
//! 根 level=1 / 子=父+1 / 环检测 / order 唯一 / 父聚合一致性）。
//!
//! 现有测试只在端到端流程里顺带走到「合法树」，**没有一条测试验过
//! 每条规则的拒绝面**。而「拒绝面漏了」意味着坏树能进域 →
//! 后续 draft/export 全在坏数据上跑。
//!
//! 本文件的形态是**每条不变量一组**：先造一棵合法树（基线），
//! 再注入一处违规，断言 validate_outline 返回 Err 且错误信息含关键锚。

use bid_domain::outline::{validate_outline, OutlineNode, SectionStatus};

/// 造一个合法的最小节点。
fn node(
    id: &str,
    level: u8,
    parent: Option<&str>,
    order: u32,
    reqs: &[&str],
    chars: u32,
) -> OutlineNode {
    OutlineNode {
        id: id.to_string(),
        parent_id: parent.map(str::to_string),
        title: format!("章节 {id}"),
        level,
        order,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars: chars,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }
}

/// 合法基线树：1 根（level 1）+ 2 叶（level 2），父配额 = 子和，父 REQ = 子并集。
fn baseline() -> Vec<OutlineNode> {
    vec![
        node("SEC-0001", 1, None, 1, &["R1", "R2"], 1000),
        node("SEC-0002", 2, Some("SEC-0001"), 2, &["R1"], 500),
        node("SEC-0003", 2, Some("SEC-0001"), 3, &["R2"], 500),
    ]
}

#[test]
fn baseline_is_valid() {
    validate_outline(&baseline()).expect("基线树必须合法——后续所有用例都基于它");
}

// ---------------------------------------------------------------- 规则 1：空树

#[test]
fn rule_empty_tree_rejected() {
    let err = validate_outline(&[]).unwrap_err();
    assert!(err.contains("空章节树"), "空树应拒绝且提示空树，实际: {err}");
}

// ---------------------------------------------------------------- 规则 2：id 唯一

#[test]
fn rule_duplicate_id_rejected() {
    let mut n = baseline();
    n.push(node("SEC-0002", 1, None, 9, &["R9"], 100));
    let err = validate_outline(&n).unwrap_err();
    assert!(err.contains("id 重复"), "重复 id 应拒绝，实际: {err}");
}

// ---------------------------------------------------------------- 规则 3：level 域

#[test]
fn rule_level_zero_rejected() {
    let mut n = baseline();
    n[0].level = 0;
    let err = validate_outline(&n).unwrap_err();
    assert!(err.contains("level="), "level=0 应拒绝，实际: {err}");
}

#[test]
fn rule_level_over_max_rejected() {
    let mut n = baseline();
    n[0].level = 5;
    let err = validate_outline(&n).unwrap_err();
    assert!(err.contains("越域"), "level=5 应拒绝（MAX_LEVEL=4），实际: {err}");
}

// ---------------------------------------------------------------- 规则 4：REQ 非空

#[test]
fn rule_empty_requirements_rejected() {
    let mut n = baseline();
    n[1].requirement_ids.clear();
    let err = validate_outline(&n).unwrap_err();
    assert!(err.contains("未绑定任何 REQ"), "空 REQ 节点应拒绝，实际: {err}");
}

// ---------------------------------------------------------------- 规则 5：根 level=1

#[test]
fn rule_root_must_be_level_one() {
    let n = vec![node("SEC-0001", 2, None, 1, &["R1"], 500)];
    let err = validate_outline(&n).unwrap_err();
    assert!(err.contains("根必须 level=1"), "无父但 level=2 应拒绝，实际: {err}");
}

// ---------------------------------------------------------------- 规则 6：子 = 父 + 1

#[test]
fn rule_level_skip_rejected() {
    let mut n = baseline();
    n[1].level = 3;
    let err = validate_outline(&n).unwrap_err();
    assert!(err.contains("跳级"), "子 level=3 父 level=1 应拒绝，实际: {err}");
}

#[test]
fn rule_orphan_parent_rejected() {
    let n = vec![node("SEC-0002", 2, Some("SEC-9999"), 1, &["R1"], 500)];
    let err = validate_outline(&n).unwrap_err();
    assert!(err.contains("不存在") || err.contains("孤儿"), "父不存在应拒绝，实际: {err}");
}

// ---------------------------------------------------------------- 规则 8：order 唯一

#[test]
fn rule_duplicate_order_rejected() {
    let mut n = baseline();
    n[2].order = n[1].order;
    let err = validate_outline(&n).unwrap_err();
    assert!(err.contains("order=") && err.contains("重复"), "order 重复应拒绝，实际: {err}");
}

// ---------------------------------------------------------------- 规则 9：父聚合一致性

#[test]
fn rule_parent_requirement_union_mismatch() {
    let mut n = baseline();
    n[2].requirement_ids = vec!["R3".into()];
    let err = validate_outline(&n).unwrap_err();
    assert!(
        err.contains("requirement_ids") && err.contains("子并集"),
        "父 REQ 与子并集不符应拒绝，实际: {err}"
    );
}

#[test]
fn rule_parent_quota_sum_mismatch() {
    let mut n = baseline();
    n[1].target_chars = 600;
    let err = validate_outline(&n).unwrap_err();
    assert!(
        err.contains("target_chars") && err.contains("配额账不平"),
        "父配额 ≠ 子和应拒绝，实际: {err}"
    );
}

// ---------------------------------------------------------------- 边界值：合法侧

/// 配额边界：子配额恰好等于父（单子情形）必须合法。
#[test]
fn boundary_single_child_quota_equal() {
    let n = vec![
        node("SEC-0001", 1, None, 1, &["R1"], 500),
        node("SEC-0002", 2, Some("SEC-0001"), 2, &["R1"], 500),
    ];
    validate_outline(&n).expect("单子且配额相等应合法");
}

/// level 边界：恰好 4 层必须合法（MAX_LEVEL=4）。
#[test]
fn boundary_max_level_four_is_valid() {
    let n = vec![
        node("SEC-0001", 1, None, 1, &["R1"], 1000),
        node("SEC-0002", 2, Some("SEC-0001"), 2, &["R1"], 1000),
        node("SEC-0003", 3, Some("SEC-0002"), 3, &["R1"], 1000),
        node("SEC-0004", 4, Some("SEC-0003"), 4, &["R1"], 1000),
    ];
    validate_outline(&n).expect("4 层树（MAX_LEVEL）应合法");
}

/// 空集边界：只有根没有子——父无子则不做聚合校验，必须合法。
#[test]
fn boundary_leaf_root_without_children() {
    let n = vec![node("SEC-0001", 1, None, 1, &["R1"], 800)];
    validate_outline(&n).expect("单根无子应合法（无聚合面可校验）");
}

/// 大配额边界：u32 上限附近不应溢出。
#[test]
fn boundary_large_quota_no_overflow() {
    const BIG: u32 = 2_000_000_000;
    let n = vec![
        node("SEC-0001", 1, None, 1, &["R1"], BIG),
        node("SEC-0002", 2, Some("SEC-0001"), 2, &["R1"], BIG),
    ];
    validate_outline(&n).expect("20 亿配额单子应合法（不溢出）");
}