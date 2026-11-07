//! review_wire.rs —— S-5 审核面的 store 接线（issue #423——run_review 的首个生产消费方）。
//!
//! 定位：[`crate::check`] 的输入面 [`ChapterInput`] 与存储层解耦（借用式、零
//! bid-domain 依赖——check.rs 的既有纪律，本模块不动它）；本模块是**桥**——把
//! [`DomainStore`] 的真源面（叶章节正文 + 参数响应账）装配成 ChapterInput 喂给
//! [`crate::check::run_review`]，让全局一致性检查（金额/工期/术语/承诺冲突）
//! 从「已实现零接线」变成 check_bid_quality 的生产信号。
//!
//! 装配面（哪些真源参与全局比对）：
//! - **叶章节正文**（`sections()` 中无子且 `content` 非空者——结构节点是调度
//!   单元不承载内容，未起草章无文本可比对）；
//! - **参数响应账**（`param_responses()` 逐行合成单行输入——响应值是**我方
//!   承诺面**，与章节正文里的同维度数值必须一致：投标函写 3500000 而参数表
//!   写 3580000 = 跨面冲突，正是 #423 的典型失守场景）。
//!
//! 合成纪律（高内聚低耦合的落点）：
//! - **逐行合成**而非整账单输入：section_id 用参数行 id（PR-xxxx）——冲突
//!   finding 的位置锚直接指到具体参数行（Agent 修复回路闭合），「整账一个
//!   位置」在多行同维度时无法定位；
//! - **只喂 `response_value` 不喂 `required_value`**：要求值是招标方的事实
//!   （我方改不了它），响应值对要求值的偏离是 A-07（derive_deviation）的
//!   域，不是全局一致性的域——两个域不在这里互相越界；
//! - 词表保守性天然防误报：合成行 `{参数名}：{响应值}。` 只有参数名命中
//!   check.rs 的维度词表（投标保证金/工期…）才会抽特征，技术参数类行
//!   （存储容量等）不进比对面。
//!
//! 输出面纪律：run_review 返回**完整** ReviewOutcome（两柱都在）；调用方
//! （bid-mcp 壳层）按 [`CheckKind::is_global`] 只取全局柱进响应——局部柱
//! （占位符/悬空引用/格式偏差）**有意不进 check_bid_quality 响应**：P6
//! validate_bid_document 已用既有规则码（W_PLACEHOLDER/E_CITATION_GHOST/
//! W_CHARS_OFF_TARGET）覆盖同域信号，双工具异阈值双报只会制造信号冲突。
//!
//! 已知范围边界（如实声明，不伪装全覆盖）：导出层的报价决策（ExportDecision
//! 的 bid_amount/price_breakdown）按设计**不入库**（「调用方显式给出，不入库、
//! 不猜测」——真源在决策方手上），check 时不可得，故不参与本比对面；报价面
//! 数值的一致性由它们最终落进章节正文/参数账的部分覆盖。

use crate::check::{self, ChapterInput, ReviewOutcome};
use bid_domain::param::ParamResponse;
use bid_domain::DomainStore;
use std::collections::HashSet;

/// 参数行 → 单行合成正文（`{参数名}：{响应值}。`——句号收尾让扫描器的句窗
/// 恰好覆盖本行，行间不串味）。
pub fn param_row_content(row: &ParamResponse) -> String {
    format!("{}：{}。", row.param_name, row.response_value)
}

/// known_refs（悬空判定面）：全部 REQ id + 全部 EV id——与 check.rs 的
/// citations 前缀口径（REQ-/EV-）对齐。
fn known_refs(store: &DomainStore) -> HashSet<String> {
    store
        .requirements()
        .iter()
        .map(|r| r.id.clone())
        .chain(store.evidences().iter().map(|e| e.id.clone()))
        .collect()
}

/// 装配比对输入：叶章节正文 + 参数响应账逐行合成。
///
/// `ledger` 是合成的拥有态（PR id → 合成正文）——借用式 ChapterInput 不能
/// 指向函数内新建的 String，由调用方先建后借（[`run_review_from_store`]
/// 内部完成，外部只见结果）。
fn chapter_inputs<'a>(
    store: &'a DomainStore,
    ledger: &'a [(String, String)],
) -> Vec<ChapterInput<'a>> {
    // 叶判定与报告层同口径：无子者即叶（parents = 被引用为父的 id 集）
    let parents: HashSet<&str> = store
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.as_deref())
        .collect();
    let mut out = Vec::new();
    for n in store.sections() {
        if parents.contains(n.id.as_str()) {
            continue; // 结构节点——调度单元，无正文
        }
        let Some(content) = n.content.as_deref() else {
            continue; // 未起草——无文本可比对（不进 = 不误报，不是漏报）
        };
        if content.is_empty() {
            continue;
        }
        out.push(ChapterInput {
            section_id: &n.id,
            content,
            // 本章声称响应的 REQ（悬空判定面——回退/重建后的幽灵 REQ 在此可见）
            citations: &n.requirement_ids,
            target_chars: Some(n.target_chars),
        });
    }
    for (id, content) in ledger {
        out.push(ChapterInput {
            section_id: id, // PR-xxxx——位置锚直达参数行
            content,
            citations: &[], // 合成行无引用面
            target_chars: None, // 无字数配额语义
        });
    }
    out
}

/// 从 store 真源跑 S-5 全量审核（两柱合流——返回完整 ReviewOutcome，全局柱
/// 的分桶归调用方）。
pub fn run_review_from_store(store: &DomainStore) -> ReviewOutcome {
    let ledger: Vec<(String, String)> = store
        .param_responses()
        .iter()
        .map(|r| (r.id.clone(), param_row_content(r)))
        .collect();
    let inputs = chapter_inputs(store, &ledger);
    let refs = known_refs(store);
    check::run_review(&inputs, &refs)
}

// ---------------------------------------------------------------- 单元测试

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::CheckKind;
    use bid_domain::param::ParamInput;

    /// 项目基座（param.rs 同款：create → import → extract——后续写入面
    /// outline/params 的状态门才能过）。
    fn store_at_requirements() -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("接线项目", "C-RW", "明标", "rw-key").unwrap();
        s.upsert_file(
            "A-招标文件.docx", "招标文件", "sha-rw", 10, 1, 50, 0,
            bid_domain::ParseStatus::Ok, "",
        ).unwrap();
        s.set_requirements(vec![bid_domain::Requirement {
            id: "REQ-0001".into(),
            category: "商务要求".into(),
            clause_no: None,
            anchors: vec![],
            params: Default::default(),
            source_file: "A-招标文件.docx".into(),
        }]).unwrap();
        s
    }

    /// 空库：无章无参数 → 零发现零特征（空输入不 panic——run_review 的空
    /// 分片面在接线侧再次钉住）。
    #[test]
    fn empty_store_yields_empty_review() {
        let store = DomainStore::new();
        let out = run_review_from_store(&store);
        assert!(out.findings.is_empty());
        assert_eq!(out.feature_count, 0);
        assert_eq!(out.peak_shards, 0);
    }

    /// 参数行合成正文：格式契约（维度词：值。句号收尾）。
    #[test]
    fn param_row_content_format() {
        let row = ParamResponse {
            id: "PR-0001".into(),
            requirement_id: "REQ-0001".into(),
            param_name: "投标保证金".into(),
            required_value: "3500000元".into(),
            response_value: "3500000元".into(),
            deviation: bid_domain::param::Deviation::None,
            evidence_id: None,
        };
        assert_eq!(param_row_content(&row), "投标保证金：3500000元。");
    }

    /// 章节正文 × 参数账的跨面冲突（#423 典型场景：投标函 3500000 vs
    /// 参数表 3580000）——位置锚必须同时给出 SEC 与 PR 两面。
    #[test]
    fn chapter_vs_param_ledger_amount_conflict() {
        let mut store = store_at_requirements();
        // 章节正文：投标保证金 3500000 元
        store.set_outline(vec![bid_domain::outline::OutlineNode {
            id: "SEC-0001".into(),
            parent_id: None,
            title: "投标函".into(),
            level: 1,
            order: 1,
            requirement_ids: vec!["REQ-0001".into()],
            target_chars: 100,
            status: bid_domain::outline::SectionStatus::Drafted,
            dirty: false,
            content: Some("我方投标保证金为 3500000 元，与招标文件要求一致。".into()),
            content_attempt: 1,
        }]).unwrap();
        // 参数账：投标保证金响应 3580000（元）
        store.set_param_responses(vec![ParamInput {
            requirement_id: "REQ-0001".into(),
            param_name: "投标保证金".into(),
            required_value: "3500000元".into(),
            response_value: "3580000元".into(),
            evidence_id: None,
        }]).unwrap();
        // 实际签发的 PR id（store 单序列——不猜序号，从真源读）
        let pr_id = store.param_responses()[0].id.clone();
        let out = run_review_from_store(&store);
        let amount_conflicts: Vec<_> = out
            .findings
            .iter()
            .filter(|f| f.kind == CheckKind::AmountConflict)
            .collect();
        assert_eq!(amount_conflicts.len(), 1, "跨面金额冲突必报: {:?}", out.findings);
        let f = amount_conflicts[0];
        assert_eq!(f.section_id, pr_id, "锚点 = 少数派取值首现位（参数行）");
        assert!(f.message.contains("SEC-0001=3500000"), "章位置与值: {}", f.message);
        assert!(f.message.contains(&format!("{pr_id}=3580000")), "参数行位置与值: {}", f.message);
    }

    /// 同值不报（章 3500000 + 参数账 3500000 = 一致——空 global 面的正面钉）。
    #[test]
    fn chapter_and_ledger_same_value_no_conflict() {
        let mut store = store_at_requirements();
        store.set_outline(vec![bid_domain::outline::OutlineNode {
            id: "SEC-0001".into(),
            parent_id: None,
            title: "投标函".into(),
            level: 1,
            order: 1,
            requirement_ids: vec!["REQ-0001".into()],
            target_chars: 100,
            status: bid_domain::outline::SectionStatus::Drafted,
            dirty: false,
            content: Some("我方投标保证金为 3500000 元。".into()),
            content_attempt: 1,
        }]).unwrap();
        store.set_param_responses(vec![ParamInput {
            requirement_id: "REQ-0001".into(),
            param_name: "投标保证金".into(),
            required_value: "3500000元".into(),
            response_value: "3500000元".into(),
            evidence_id: None,
        }]).unwrap();
        let out = run_review_from_store(&store);
        assert!(
            out.findings.iter().all(|f| !f.kind.is_global()),
            "同值零冲突: {:?}",
            out.findings
        );
    }

    /// is_global 分桶：四类全局为真、三类局部为假（接线面的分桶判据钉死）。
    #[test]
    fn check_kind_global_bucket() {
        assert!(CheckKind::AmountConflict.is_global());
        assert!(CheckKind::ScheduleConflict.is_global());
        assert!(CheckKind::TermConflict.is_global());
        assert!(CheckKind::CommitmentConflict.is_global());
        assert!(!CheckKind::Placeholder.is_global());
        assert!(!CheckKind::DanglingRef.is_global());
        assert!(!CheckKind::FormatDeviation.is_global());
    }
}
