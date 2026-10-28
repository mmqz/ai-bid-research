//! outline.rs —— T-5 OutlineNode 章节树（issue #11 批次 v3 / V3-1，百万字分片的骨架）。
//!
//! 定位：500 章级标书的调度根——没有章节树，「每章 2,000 字 × 500 次 per-section
//! 调度」无从谈起（S-1/S-2/S-6/S-8 全部依赖本实体）。
//!
//! 状态词表（**xref_check C7 强制口径**，domain-model §1 BidSection）：
//! `PENDING → OUTLINING → DRAFTING → DRAFTED → VALIDATING → REVISING → ACCEPTED`；
//! `dirty` 是**可叠加脏标记**（非状态——上游 content_version 变化时打标，R-05）。
//! （注：issue #11 批次 v3 评论的 5 态草图与 domain-model 词表不一致，本实现
//! 取 domain-model——它是 xref 的机器检查真源。）
//!
//! 树不变量（[`DomainStore::set_outline`] 的验收面，违反即 E_UNPROCESSABLE）：
//! 1. id 唯一；2. parent 引用存在（无孤儿）；3. 无环（父链有限）；4. 根 level=1；
//! 5. 子 level = 父 level + 1（无跳级）；6. level ∈ 1..=4；7. order 全局唯一；
//! 8. 每节点绑定 ≥1 条 REQ（T-5 验收：分片的最小调度单元必须有活干）；
//! 9. 父节点 requirement_ids == 子节点并集（反范式聚合的一致性——查询面免递归，
//!    重建面由 set_outline 把关）。

use crate::{DomainStore, ProjectStatus, Requirement, ToolError};
use serde::{Deserialize, Serialize};

/// 章节最大层级（1..=4——T-5 规格「1-3（可到 4）」）。
pub const MAX_LEVEL: u8 = 4;

// ---------------------------------------------------------------- 状态机
/// 章节状态（domain-model §1 词表——xref C7 真源；ACCEPTED 后需显式 reopen，R-06）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SectionStatus {
    Pending,
    Outlining,
    Drafting,
    Drafted,
    Validating,
    Revising,
    Accepted,
}

impl SectionStatus {
    /// 合法前向链（domain-model §1 的线性推进）。
    pub fn next_allowed(&self) -> &'static [SectionStatus] {
        use SectionStatus::*;
        match self {
            Pending => &[Outlining],
            Outlining => &[Drafting],
            Drafting => &[Drafted],
            Drafted => &[Validating],
            // 校验零发现直通验收（免空转 REVISING）；有发现走修订
            Validating => &[Revising, Accepted],
            Revising => &[Accepted],
            Accepted => &[], // 终态——修改需显式 reopen（R-06 幂等保护）
        }
    }

    pub fn as_str(&self) -> &'static str {
        use SectionStatus::*;
        match self {
            Pending => "PENDING",
            Outlining => "OUTLINING",
            Drafting => "DRAFTING",
            Drafted => "DRAFTED",
            Validating => "VALIDATING",
            Revising => "REVISING",
            Accepted => "ACCEPTED",
        }
    }
}

// ---------------------------------------------------------------- 实体
/// 章节树节点（T-5 规格 struct 的 Rust 落地——字段对齐 issue #11 批次 v3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineNode {
    pub id: String,                  // SEC-xxxx（store 单序列签发）
    pub parent_id: Option<String>,   // None = 根
    pub title: String,
    pub level: u8,                   // 1..=4
    pub order: u32,                  // 文档序（全局唯一）
    pub requirement_ids: Vec<String>, // 本章响应的 REQ（叶 = 分片块；父 = 子并集聚合）
    pub target_chars: u32,           // 字数配额（父 = 子配额和）
    pub status: SectionStatus,
    /// 脏标记（可叠加——上游变更打标，非状态机成员；R-05 影响面分析的输入）
    #[serde(default)]
    pub dirty: bool,
}

// ---------------------------------------------------------------- 纯逻辑面
/// 派生参数：`max_reqs_per_leaf` 决定分片粒度（验收口径：12,000 REQ / 500 章 ≈ 24）。
#[derive(Debug, Clone, Copy)]
pub struct OutlineSpec {
    pub target_total_chars: u32,
    pub max_reqs_per_leaf: usize,
}

impl Default for OutlineSpec {
    fn default() -> Self {
        // 1,000,000 字 / (12,000 REQ ÷ 24) ≈ 500 章——批次 v3 §零 的规模口径
        OutlineSpec { target_total_chars: 1_000_000, max_reqs_per_leaf: 24 }
    }
}

/// 叶节点配额下限（防零配额退化——低于此值的章节失去独立调度意义）。
const MIN_LEAF_CHARS: u32 = 500;

/// 从 REQ 清单推导章节树（纯函数——确定性：按类目首现序 + id 序，无隐藏随机性）。
///
/// 结构：每个非空类目 → 1 个 level-1 节点；类目内 REQ 按 id 序切块（每块
/// ≤ max_reqs_per_leaf）→ level-2 叶节点。父节点 requirement_ids = 子并集、
/// target_chars = 子和（聚合一致性由构造保证，set_outline 复核）。
///
/// 配额：最大余数法（largest remainder）——先按 REQ 数占比取整分配，余量补给
/// 小数部分最大的叶（并列按文档序），总和恰等于 target（下限保护除外）。
///
/// `next_id` 签名器由调用方注入（store 传 `self.next_id`；测试传计数闭包）——
/// 派生逻辑与 id 序列解耦。
pub fn derive_outline(
    reqs: &[Requirement],
    spec: OutlineSpec,
    next_id: &mut dyn FnMut(&str) -> String,
) -> Vec<OutlineNode> {
    // 类目分桶（首现序——输入已按 id 序时全确定）
    let mut category_order: Vec<String> = Vec::new();
    let mut buckets: Vec<(String, Vec<&Requirement>)> = Vec::new();
    for r in reqs {
        match buckets.iter_mut().find(|(c, _)| *c == r.category) {
            Some((_, v)) => v.push(r),
            None => {
                category_order.push(r.category.clone());
                buckets.push((r.category.clone(), vec![r]));
            }
        }
    }
    let total_reqs: usize = reqs.len();
    if total_reqs == 0 {
        return vec![];
    }
    // 叶块清单（先收集再配额——需要全局余数视角）
    struct LeafDraft { category: String, req_ids: Vec<String> }
    let mut leaves: Vec<LeafDraft> = Vec::new();
    for (cat, rs) in &buckets {
        for chunk in rs.chunks(spec.max_reqs_per_leaf.max(1)) {
            leaves.push(LeafDraft {
                category: cat.clone(),
                req_ids: chunk.iter().map(|r| r.id.clone()).collect(),
            });
        }
    }
    // 最大余数法配额
    let leaf_counts: Vec<u64> = leaves.iter().map(|l| l.req_ids.len() as u64).collect();
    let sum_counts: u64 = leaf_counts.iter().sum();
    let mut quotas: Vec<u32> = leaf_counts.iter()
        .map(|&c| ((u64::from(spec.target_total_chars) * c / sum_counts) as u32).max(MIN_LEAF_CHARS))
        .collect();
    // 下限保护后的偏差：差额按 REQ 占比回补/扣减（简单二次均衡，保总和 ≈ target）
    let quota_sum: u64 = quotas.iter().map(|q| u64::from(*q)).sum();
    let target = u64::from(spec.target_total_chars);
    if quota_sum != target {
        let diff = target.abs_diff(quota_sum);
        let step: u32 = if quota_sum < target { 1 } else { 0 }; // 1 = 加，0 = 减
        let mut i = 0;
        let mut remaining = diff;
        while remaining > 0 {
            let idx = i % quotas.len();
            if step == 1 {
                quotas[idx] += 1;
                remaining -= 1;
            } else if quotas[idx] > MIN_LEAF_CHARS {
                quotas[idx] -= 1;
                remaining -= 1;
            }
            i += 1;
            if i > quotas.len() * 4 && remaining == diff {
                break; // 全部触底（target 过小）——接受偏差（验收容差 ±10% 兜底）
            }
        }
    }
    // 组树：类目 → level-1，叶 → level-2
    let mut nodes: Vec<OutlineNode> = Vec::new();
    let mut order: u32 = 0;
    for (cat, _) in &buckets {
        let cat_leaves: Vec<usize> = leaves.iter()
            .enumerate()
            .filter(|(_, l)| &l.category == cat)
            .map(|(i, _)| i)
            .collect();
        let parent_id = next_id("SEC");
        order += 1;
        // 聚合面（子并集 / 配额和）
        let mut agg_reqs: Vec<String> = Vec::new();
        let mut agg_chars: u32 = 0;
        for &li in &cat_leaves {
            for rid in &leaves[li].req_ids {
                if !agg_reqs.contains(rid) {
                    agg_reqs.push(rid.clone());
                }
            }
            agg_chars += quotas[li];
        }
        nodes.push(OutlineNode {
            id: parent_id.clone(),
            parent_id: None,
            title: cat.clone(),
            level: 1,
            order,
            requirement_ids: agg_reqs,
            target_chars: agg_chars,
            status: SectionStatus::Pending,
            dirty: false,
        });
        for &li in &cat_leaves {
            order += 1;
            nodes.push(OutlineNode {
                id: next_id("SEC"),
                parent_id: Some(parent_id.clone()),
                title: format!("{}（第{}块）", cat, cat_leaves.iter().position(|&x| x == li).unwrap() + 1),
                level: 2,
                order,
                requirement_ids: leaves[li].req_ids.clone(),
                target_chars: quotas[li],
                status: SectionStatus::Pending,
                dirty: false,
            });
        }
    }
    nodes
}

/// 树不变量校验（纯函数——set_outline 的裁决面，错误即 E_UNPROCESSABLE reason）。
pub fn validate_outline(nodes: &[OutlineNode]) -> Result<(), String> {
    if nodes.is_empty() {
        return Err("空章节树（REQ 为空时无从推导——先 extract_requirements）".into());
    }
    // 1. id 唯一
    let mut seen = std::collections::HashSet::new();
    for n in nodes {
        if !seen.insert(&n.id) {
            return Err(format!("章节 id 重复: {}", n.id));
        }
    }
    // 2-6. 结构面
    for n in nodes {
        if n.level == 0 || n.level > MAX_LEVEL {
            return Err(format!("{} level={} 越域（1..={MAX_LEVEL}）", n.id, n.level));
        }
        if n.requirement_ids.is_empty() {
            return Err(format!("{} 未绑定任何 REQ（每节点 ≥1——分片最小调度单元必须有活干）", n.id));
        }
        match &n.parent_id {
            None => {
                if n.level != 1 {
                    return Err(format!("{} 无父但 level={}（根必须 level=1）", n.id, n.level));
                }
            }
            Some(pid) => {
                let parent = nodes.iter().find(|x| &x.id == pid).ok_or_else(|| {
                    format!("{} 的父 {} 不存在（孤儿节点）", n.id, pid)
                })?;
                if n.level != parent.level + 1 {
                    return Err(format!(
                        "{} level={} 与父 {} level={} 跳级（子 = 父 + 1）",
                        n.id, n.level, pid, parent.level
                    ));
                }
                // 3. 环检测：父链必须在有限步内到根
                let mut cursor = parent.clone();
                let mut hops = 0;
                while let Some(ref pp) = cursor.parent_id {
                    cursor = nodes.iter().find(|x| x.id == *pp).ok_or_else(|| {
                        format!("{} 的祖先 {} 不存在", cursor.id, pp)
                    })?.clone();
                    hops += 1;
                    if hops > nodes.len() {
                        return Err(format!("{} 的父链成环", n.id));
                    }
                }
            }
        }
    }
    // 7. order 全局唯一
    let mut orders = std::collections::HashSet::new();
    for n in nodes {
        if !orders.insert(n.order) {
            return Err(format!("order={} 重复（文档序必须全局唯一）", n.order));
        }
    }
    // 9. 父聚合一致性（requirement_ids == 子并集 && target_chars == 子和）
    for n in nodes {
        let children: Vec<&OutlineNode> = nodes.iter().filter(|x| x.parent_id.as_deref() == Some(&n.id[..])).collect();
        if !children.is_empty() {
            let mut union: Vec<String> = Vec::new();
            let mut sum = 0u64;
            for c in &children {
                for rid in &c.requirement_ids {
                    if !union.contains(rid) {
                        union.push(rid.clone());
                    }
                }
                sum += u64::from(c.target_chars);
            }
            let mut stored = n.requirement_ids.clone();
            stored.sort();
            let mut expect = union;
            expect.sort();
            if stored != expect {
                return Err(format!(
                    "{} 的 requirement_ids 与子并集不一致（聚合面失真——重建章节树）", n.id
                ));
            }
            if u64::from(n.target_chars) != sum {
                return Err(format!(
                    "{} target_chars={} ≠ 子配额和 {}（配额账不平）", n.id, n.target_chars, sum
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- DomainStore 扩面（T-5 章节树操作）
impl DomainStore {
    /// 章节树只读面。
    pub fn sections(&self) -> &[OutlineNode] {
        &self.sections
    }

    /// 章节树总字数配额（**根节点求和**——父配额 = 子配额和，全树求和会双计）。
    pub fn outline_total_chars(&self) -> u64 {
        self.sections.iter()
            .filter(|n| n.parent_id.is_none())
            .map(|n| u64::from(n.target_chars)).sum()
    }

    /// 整树替换（重开纲——补遗后的重推导走同门）。
    ///
    /// 状态门：REQUIREMENTS_EXTRACTED / MATRIX_READY / EVIDENCE_BOUND 可重开纲；
    /// DRAFT 起草前置面未抽条款、DRAFTING+ 起草中重开纲需 S-5 影响面机制（后续批次）。
    pub fn set_outline(&mut self, nodes: Vec<OutlineNode>) -> Result<(usize, u64), ToolError> {
        let p = self.project()?;
        let allowed = matches!(
            p.status,
            ProjectStatus::RequirementsExtracted | ProjectStatus::MatrixReady | ProjectStatus::EvidenceBound
        );
        if !allowed {
            return Err(ToolError::EState {
                current: p.status,
                attempted: "set_outline",
            });
        }
        if let Err(reason) = validate_outline(&nodes) {
            return Err(ToolError::EUnprocessable { reason });
        }
        // REQ 存在性（引用完整性——挂空 REQ 的章节是幽灵调度单元）
        let known: std::collections::HashSet<&str> = self.requirements.iter().map(|r| r.id.as_str()).collect();
        for n in &nodes {
            for rid in &n.requirement_ids {
                if !known.contains(&rid[..]) {
                    return Err(ToolError::EUnprocessable {
                        reason: format!("{} 引用了不存在的条款 {rid}（幽灵调度单元）", n.id),
                    });
                }
            }
        }
        let total = nodes.iter()
            .filter(|n| n.parent_id.is_none())
            .map(|n| u64::from(n.target_chars)).sum::<u64>();
        let count = nodes.len();
        self.sections = nodes;
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok((count, total))
    }

    /// 从已抽条款推导并落树（验收主入口——「从 REQ 清单推导章节树」）。
    pub fn derive_and_set_outline(&mut self, spec: OutlineSpec) -> Result<(usize, u64), ToolError> {
        let reqs = self.requirements.clone();
        let nodes = derive_outline(&reqs, spec, &mut |prefix| self.next_id(prefix));
        self.set_outline(nodes)
    }

    /// 章节状态推进（domain-model 线性链 + Validating 直通 Accepted 捷径）。
    /// 拒绝面零副作用（E_UNPROCESSABLE——T-7 审核闭环落地时升格为 typed 状态错）。
    pub fn advance_section_status(&mut self, id: &str, to: SectionStatus) -> Result<(), ToolError> {
        let idx = self.sections.iter().position(|n| n.id == id).ok_or_else(|| {
            ToolError::ENotFound { what: format!("section {id} 不存在") }
        })?;
        let cur = self.sections[idx].status;
        if !cur.next_allowed().contains(&to) {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "章节状态 {} 不允许推进到 {}（合法前向: {:?}）",
                    cur.as_str(), to.as_str(),
                    cur.next_allowed().iter().map(|s| s.as_str()).collect::<Vec<_>>()
                ),
            });
        }
        self.sections[idx].status = to;
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }

    /// 显式 reopen（R-06）：ACCEPTED → REVISING + 打脏标记。
    /// 唯一离开终态的通道——「验收后修改必须显式」，防静默回退。
    pub fn reopen_section(&mut self, id: &str) -> Result<(), ToolError> {
        let idx = self.sections.iter().position(|n| n.id == id).ok_or_else(|| {
            ToolError::ENotFound { what: format!("section {id} 不存在") }
        })?;
        if self.sections[idx].status != SectionStatus::Accepted {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "reopen 仅适用于 ACCEPTED（当前 {}——非终态用 advance）",
                    self.sections[idx].status.as_str()
                ),
            });
        }
        self.sections[idx].status = SectionStatus::Revising;
        self.sections[idx].dirty = true;
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParseStatus;

    /// 章节树基座：项目 + 文件 + N 条 REQ。
    fn store_with_reqs(n_reqs: usize) -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("测试项目", "ZFCG-2026-GK-0311", "明标", "ext-key-sec").unwrap();
        s.upsert_file("A-招标文件.docx", "招标文件", "aa", 31, 12, 120, 0,
                      ParseStatus::Ok, "").unwrap();
        let categories = ["资格条件", "技术参数", "评分项", "商务要求", "格式要求", "其他"];
        let reqs: Vec<Requirement> = (0..n_reqs).map(|i| Requirement {
            id: format!("REQ-{:04}", i + 3),
            category: categories[i % categories.len()].into(),
            clause_no: Some(format!("3.{}", i + 1)),
            anchors: vec![crate::Anchor {
                file: "A".into(), kind: "para".into(),
                locator: "第三章".into(),
                excerpt: format!("条款-{i} 的原文摘录"),
            }],
            params: std::collections::HashMap::new(),
            source_file: "A".into(),
        }).collect();
        s.set_requirements(reqs).unwrap();
        s
    }

    /// GT sections.jsonl 六章节语义面（人工树——验证 set_outline 与聚合一致性）。
    fn gt_six_sections() -> Vec<OutlineNode> {
        let gt_sections: &[(&str, &[&str])] = &[
            ("资格响应", &["REQ-0003", "REQ-0004", "REQ-0005"]),
            ("技术方案-网络与安全", &["REQ-0006", "REQ-0007", "REQ-0008"]),
            ("商务部分-报价", &["REQ-0009", "REQ-0010", "REQ-0011", "REQ-0012"]),
            ("售后服务方案", &["REQ-0013"]),
            ("项目团队", &["REQ-0014"]),
            ("格式与签署", &["REQ-0015", "REQ-0016", "REQ-0017"]),
        ];
        gt_sections.iter().enumerate().map(|(i, (title, gts))| {
            let reqs: Vec<String> = gts.iter().map(|g| g.to_string()).collect();
            let chars = (reqs.len() as u32) * 2000;
            OutlineNode {
                id: format!("SEC-{:04}", i + 1),
                parent_id: None,
                title: (*title).into(),
                level: 1,
                order: (i + 1) as u32,
                requirement_ids: reqs,
                target_chars: chars,
                status: SectionStatus::Pending,
                dirty: false,
            }
        }).collect()
    }

    // ============================ 自研测试 ============================

    /// T-5 验收主测：320 条 REQ → 300-500 节点、每节点 ≥1 REQ、配额和 ≈ 目标 ±10%。
    #[test]
    fn derive_acceptance_scale_320_reqs() {
        let mut s = store_with_reqs(320);
        let (count, total) = s.derive_and_set_outline(OutlineSpec {
            target_total_chars: 640_000,
            max_reqs_per_leaf: 1, // 每叶 1 REQ → 320 叶 + 类目父
        }).unwrap();
        assert!((300..=500).contains(&count), "节点数 {count} 必须在 300..=500（T-5 验收带）");
        // 每节点 ≥1 REQ（validate_outline 已把关——这里复核总量守恒）
        let bound: std::collections::HashSet<&str> = s.sections().iter()
            .flat_map(|n| n.requirement_ids.iter())
            .map(|r| &r[..])
            .collect();
        // 320 REQ 全覆盖（level-2 叶共 320 个块，每块恰 1 条）
        let leaves: Vec<&OutlineNode> = s.sections().iter().filter(|n| n.level == 2).collect();
        assert_eq!(leaves.len(), 320, "每叶恰 1 REQ");
        assert_eq!(leaves.iter().map(|l| l.requirement_ids.len()).sum::<usize>(), 320);
        assert_eq!(bound.len(), 320, "REQ 全覆盖无遗漏");
        // 配额和 ≈ 目标 ±10%
        let dev = (total as f64 - 640_000.0).abs() / 640_000.0;
        assert!(dev <= 0.10, "配额和 {total} 偏离目标 ±10%（实际偏离 {:.1}%）", dev * 100.0);
        assert_eq!(s.outline_total_chars(), total, "账面一致");
        // 默认参数（24/叶）：320 REQ ÷ 6 类目 → 每类 53-54 REQ → 每类 ceil(54/24)=3 叶
        // → 6 父 + 18 叶 = 24 节点
        let mut s2 = store_with_reqs(320);
        let (c2, _) = s2.derive_and_set_outline(OutlineSpec::default()).unwrap();
        assert_eq!(c2, 6 + 6 * 3, "默认粒度分片数（6 类目 × 3 叶 + 6 父）");
    }

    /// 确定性：同输入双推导全等（分片调度的可重放前提——S-2 断点续跑的地基）。
    #[test]
    fn derive_is_deterministic() {
        let reqs_of = |s: &DomainStore| s.requirements.clone();
        let mut s1 = store_with_reqs(50);
        let mut s2 = store_with_reqs(50);
        s1.derive_and_set_outline(OutlineSpec { target_total_chars: 100_000, max_reqs_per_leaf: 7 }).unwrap();
        s2.derive_and_set_outline(OutlineSpec { target_total_chars: 100_000, max_reqs_per_leaf: 7 }).unwrap();
        assert_eq!(s1.sections(), s2.sections(), "同输入双推导必须全等");
        assert_eq!(reqs_of(&s1), reqs_of(&s2));
        // 配额和恰等于目标（最大余数法无下限触发时）
        assert_eq!(s1.outline_total_chars(), 100_000, "最大余数法：总和恰等于目标");
    }

    /// GT sections.jsonl 六章节（人工树走 set_outline——全 level-1 平铺也合法）。
    #[test]
    fn gt_six_section_outline_lands() {
        let mut s = store_with_reqs(15);
        let (count, total) = s.set_outline(gt_six_sections()).unwrap();
        assert_eq!(count, 6, "六章节（sections.jsonl 口径）");
        assert_eq!(total, (3 + 3 + 4 + 1 + 1 + 3) * 2000, "配额账 = REQ 数 × 2000（全 level-1 平铺——根求和口径）");
        // 幂等重设（重开纲）：同树替换成功（补遗后重推导同门）
        let v = s.content_version();
        let (count2, _) = s.set_outline(gt_six_sections()).unwrap();
        assert_eq!(count2, 6);
        assert_eq!(s.content_version(), v + 1, "重开纲 bump（真实结构变化语义）");
    }

    /// 树不变量拒绝面（九类破坏逐项注入——校验器的回归闸门）。
    #[test]
    fn set_outline_rejects_broken_trees() {
        let mut s = store_with_reqs(15);
        let base = gt_six_sections();
        let assert_reject = |s: &mut DomainStore, nodes: Vec<OutlineNode>, why: &str| {
            let n_before = s.sections().len();
            let err = s.set_outline(nodes).unwrap_err();
            assert_eq!(err.code(), "E_UNPROCESSABLE", "{why}");
            assert_eq!(s.sections().len(), n_before, "{why}: 拒绝路径零残留");
        };
        // 孤儿父
        let mut t1 = base.clone();
        t1[1].parent_id = Some("SEC-9999".into());
        t1[1].level = 2;
        assert_reject(&mut s, t1, "孤儿父");
        // 环（两节点互为父子——先造合法两代树再互指）
        let mut t2 = vec![
            OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 1, order: 1,
                requirement_ids: vec!["REQ-0003".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false },
            OutlineNode { id: "SEC-B".into(), parent_id: Some("SEC-A".into()), title: "B".into(), level: 2, order: 2,
                requirement_ids: vec!["REQ-0004".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false },
        ];
        t2[0].parent_id = Some("SEC-B".into()); // A 的父是 B，B 的父是 A
        assert_reject(&mut s, t2, "父链成环");
        // 跳级
        let t3 = vec![
            OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 1, order: 1,
                requirement_ids: vec!["REQ-0003".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false },
            OutlineNode { id: "SEC-B".into(), parent_id: Some("SEC-A".into()), title: "B".into(), level: 3, order: 2,
                requirement_ids: vec!["REQ-0004".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false },
        ];
        assert_reject(&mut s, t3, "level 跳级");
        // level 越域（5 层）
        let t4 = vec![
            OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 5, order: 1,
                requirement_ids: vec!["REQ-0003".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false },
        ];
        assert_reject(&mut s, t4, "level 越域");
        // 根非 level 1
        let t5 = vec![OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 2, order: 1,
            requirement_ids: vec!["REQ-0003".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false }];
        assert_reject(&mut s, t5, "根 level≠1");
        // order 重复
        let mut t6 = gt_six_sections();
        t6[1].order = t6[0].order;
        assert_reject(&mut s, t6, "order 重复");
        // 空绑定
        let mut t7 = gt_six_sections();
        t7[3].requirement_ids = vec![];
        assert_reject(&mut s, t7, "节点零 REQ 绑定");
        // 幽灵 REQ
        let mut t8 = gt_six_sections();
        t8[4].requirement_ids = vec!["REQ-9999".into()];
        assert_reject(&mut s, t8, "引用不存在的 REQ");
        // 聚合失真（父聚合 ≠ 子并集）
        let t9 = vec![
            OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 1, order: 1,
                requirement_ids: vec!["REQ-0003".into(), "REQ-0004".into()], target_chars: 200, status: SectionStatus::Pending, dirty: false },
            OutlineNode { id: "SEC-B".into(), parent_id: Some("SEC-A".into()), title: "B".into(), level: 2, order: 2,
                requirement_ids: vec!["REQ-0003".into(), "REQ-0005".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false },
        ];
        assert_reject(&mut s, t9, "父聚合与子并集不一致");
        // 空树
        assert_reject(&mut s, vec![], "空章节树");
    }

    /// 状态门：未抽条款（FILES_IMPORTED）不可开纲；DRAFTING+ 不可重开纲（S-5 面）。
    #[test]
    fn set_outline_state_gates() {
        let mut s = DomainStore::new();
        s.create_project("p", "c", "明标", "k-sec").unwrap();
        s.upsert_file("f", "招标文件", "sha-x", 1, 0, 5, 0, ParseStatus::Ok, "").unwrap();
        let err = s.set_outline(gt_six_sections()).unwrap_err();
        assert_eq!(err.code(), "E_STATE", "FILES_IMPORTED 无 REQ 不可开纲");
        // 推到 REQUIREMENTS_EXTRACTED 后放行
        s.set_requirements(vec![Requirement {
            id: "REQ-0003".into(), category: "资格条件".into(), clause_no: None,
            anchors: vec![], params: std::collections::HashMap::new(), source_file: "f".into(),
        }]).unwrap();
        let mut tree = gt_six_sections();
        tree.truncate(1);
        tree[0].requirement_ids = vec!["REQ-0003".into()];
        assert!(s.set_outline(tree).is_ok(), "REQUIREMENTS_EXTRACTED 可开纲");
    }

    /// 章节状态机：线性链 + 直通捷径 + 拒绝面零副作用 + reopen 唯一出口。
    #[test]
    fn section_status_chain_gates_and_reopen() {
        use SectionStatus::*;
        // 链形态
        assert_eq!(Pending.next_allowed(), &[Outlining]);
        assert_eq!(Validating.next_allowed(), &[Revising, Accepted], "零发现直通 + 有发现修订");
        assert_eq!(Accepted.next_allowed(), &[], "终态");
        // 推进 + 越级拒绝
        let mut s = store_with_reqs(15);
        s.set_outline(gt_six_sections()).unwrap();
        let v = s.content_version();
        let err = s.advance_section_status("SEC-0001", Drafted).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "PENDING→DRAFTED 越级拒");
        assert_eq!(s.sections()[0].status, Pending, "拒绝零副作用");
        assert_eq!(s.content_version(), v, "拒绝不 bump");
        s.advance_section_status("SEC-0001", Outlining).unwrap();
        s.advance_section_status("SEC-0001", Drafting).unwrap();
        s.advance_section_status("SEC-0001", Drafted).unwrap();
        s.advance_section_status("SEC-0001", Validating).unwrap();
        s.advance_section_status("SEC-0001", Accepted).unwrap(); // 直通捷径
        assert_eq!(s.sections()[0].status, Accepted);
        // reopen 唯一出口（R-06）
        s.reopen_section("SEC-0001").unwrap();
        assert_eq!(s.sections()[0].status, Revising);
        assert!(s.sections()[0].dirty, " reopen 必打脏标记");
        // 非 ACCEPTED 的 reopen 拒
        let err = s.reopen_section("SEC-0002").unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "reopen 仅适用终态");
        // 不存在
        assert!(matches!(
            s.advance_section_status("SEC-9999", Outlining),
            Err(ToolError::ENotFound { .. })
        ));
    }
}
