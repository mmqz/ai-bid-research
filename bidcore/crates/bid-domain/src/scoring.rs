//! scoring.rs —— T-3 ScoringItem 评分矩阵（issue #11 批次 v3 / V3-2）。
//!
//! 现状补齐：`Requirement.category == "评分项"` 此前只是字符串——无权重、无
//! 评分标准结构。本模块给评分面三维结构：**分值（weight/max_score）+ 标准原文
//! （criteria）+ 否决门（is_hard_gate）**。
//!
//! 派生态设计（与 T-2 矩阵同哲学）：ScoringItem 从 requirements 确定性派生
//! （category + params.score + 锚点摘录），不落库——重算幂等、无同步问题；
//! 人工调权等覆写面待评分细则工具（G-1 get_scoring_matrix，V3-6）再议。
//!
//! 分值来源（extract.rs T-3 扩展）：`params.score`——评分细则行/表格行的
//! 确定性正则槽位（`N、报价：65 分` → 65.0）。无分值行（评标基准价算法类）
//! weight = None——**分值缺失显式登记**（`scoring_gaps()`），不伪装 0 分。

use crate::{DomainStore, Requirement};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- 实体
/// 评分矩阵项（T-2/T-3 规格 struct 的 Rust 落地）。
///
/// `is_hard_gate=true`（否决项）：**漏响应即废标**——独立闸门（T-2 blocking
/// 的否决面子集；本实体只登记，闸门裁决在 matrix.rs `check_export_ready`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoringItem {
    pub requirement_id: String,
    /// 分值/权重（评分项有；否决项 None——否决无分值，漏了直接废标）
    pub weight: Option<f64>,
    /// 单项满分（V3-2 = weight 同值；细则拆分档位时分离）
    pub max_score: Option<f64>,
    /// 评分标准原文（锚点摘录——审校比对用）
    pub criteria: String,
    /// 否决项/废标项 → true（漏响应即废标的独立闸门）
    pub is_hard_gate: bool,
}

/// 评分矩阵派生（单一真源——纯函数，12,000 REQ 量级 O(N)）。
///
/// - `评分项` → weight/max_score 取 `params.score`（缺失 → None，入 gaps）；
/// - `否决项` → is_hard_gate=true（无分值）；
/// - 其余类别不进评分矩阵。
pub fn derive_scoring_items(reqs: &[Requirement]) -> Vec<ScoringItem> {
    reqs.iter().filter_map(|r| match r.category.as_str() {
        "评分项" => {
            let weight = r.params.get("score").and_then(|v| v.as_f64());
            Some(ScoringItem {
                requirement_id: r.id.clone(),
                weight,
                max_score: weight,
                criteria: r.anchors.first().map(|a| a.excerpt.clone()).unwrap_or_default(),
                is_hard_gate: false,
            })
        }
        "否决项" => Some(ScoringItem {
            requirement_id: r.id.clone(),
            weight: None,
            max_score: None,
            criteria: r.anchors.first().map(|a| a.excerpt.clone()).unwrap_or_default(),
            is_hard_gate: true,
        }),
        _ => None,
    }).collect()
}

/// 评分面健康度（sanity——分值缺失/总分越界登记，不阻断）：
/// - `scoring_gaps`：评分项无分值（分值缺失显式化——「满分 100 分」总分层误配
///   或表结构未覆盖，人工核对面）；
/// - `total_weight`：分值合计（评标办法惯例 ≤ 100——越界 = 疑误配，告警）。
pub fn scoring_sanity(items: &[ScoringItem]) -> (Vec<String>, f64) {
    let gaps: Vec<String> = items.iter()
        .filter(|i| !i.is_hard_gate && i.weight.is_none())
        .map(|i| format!("SCORING_GAP: {} 分值缺失（评分细则行无分值模式——人工核对）", i.requirement_id))
        .collect();
    let total: f64 = items.iter().filter_map(|i| i.weight).sum();
    (gaps, total)
}

// ---------------------------------------------------------------- DomainStore 扩面
impl DomainStore {
    /// 评分矩阵（派生态只读——每次从 requirements 重算，幂等无落库）。
    pub fn scoring_items(&self) -> Vec<ScoringItem> {
        derive_scoring_items(&self.requirements)
    }

    /// 硬门清单（否决项 REQ id——T-3 验收「植入 1 个否决项漏响应 → 阻断导出」的输入面；
    /// 阻断裁决在 `check_export_ready`，此处只供核对/展示）。
    pub fn hard_gate_requirement_ids(&self) -> Vec<String> {
        self.scoring_items().iter()
            .filter(|i| i.is_hard_gate)
            .map(|i| i.requirement_id.clone())
            .collect()
    }
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParseStatus;
    use std::collections::HashMap;

    fn scoring_req(id: &str, score: Option<f64>, excerpt: &str) -> Requirement {
        let mut params = HashMap::new();
        if let Some(s) = score {
            params.insert("score".to_string(), serde_json::json!(s));
        }
        Requirement {
            id: id.into(), category: "评分项".into(), clause_no: None,
            anchors: vec![crate::Anchor {
                file: "A".into(), kind: "para".into(), locator: "评标办法".into(),
                excerpt: excerpt.into(),
            }],
            params, source_file: "A".into(),
        }
    }

    fn veto_req(id: &str) -> Requirement {
        Requirement {
            id: id.into(), category: "否决项".into(), clause_no: None,
            anchors: vec![crate::Anchor {
                file: "A".into(), kind: "para".into(), locator: "须知".into(),
                excerpt: "投标文件未经法定代表人签字的将被否决".into(),
            }],
            params: HashMap::new(), source_file: "A".into(),
        }
    }

    /// T-3 验收：20+ 评分项含分值 + 否决项 hard gate + 总分 sane + 缺失登记。
    #[test]
    fn scoring_matrix_extracts_20_plus_items_with_weights() {
        let mut reqs: Vec<Requirement> = (0..24).map(|i| {
            scoring_req(&format!("REQ-{:04}", i + 3), Some((i % 5 + 1) as f64),
                        &format!("评分细则第{}项：技术响应与偏离（{}分）", i + 1, (i % 5 + 1)))
        }).collect();
        reqs.push(veto_req("REQ-0027"));
        reqs.push(scoring_req("REQ-0028", None, "评标基准价 = 满足要求的最低价为基准")); // 算法行无分值

        let items = derive_scoring_items(&reqs);
        assert_eq!(items.len(), 26, "24 评分项 + 1 否决项 + 1 无分值算法行");
        let with_weight = items.iter().filter(|i| i.weight.is_some()).count();
        assert_eq!(with_weight, 24, "T-3 验收：20+ 评分项含分值");
        let hard = items.iter().filter(|i| i.is_hard_gate).count();
        assert_eq!(hard, 1, "否决项 = 硬门");
        let veto = items.iter().find(|i| i.is_hard_gate).unwrap();
        assert!(veto.criteria.contains("否决"), "标准原文随行");
        assert_eq!(veto.weight, None, "否决项无分值");

        // sanity：缺失登记 + 总分
        let (gaps, total) = scoring_sanity(&items);
        assert_eq!(gaps.len(), 1, "无分值算法行显式登记不伪装");
        assert!(gaps[0].contains("REQ-0028"));
        let expect_total: f64 = (0..24).map(|i| (i % 5 + 1) as f64).sum();
        assert!((total - expect_total).abs() < 1e-9, "总分 = Σ 分值（{total}）");
        assert!(total <= 100.0, "评标办法总分惯例 ≤100——sanity 面不越界");
    }

    /// store 面接线：抽取 → 评分矩阵派生 + 硬门清单。
    #[test]
    fn store_scoring_derivation_and_hard_gates() {
        let mut s = DomainStore::new();
        s.create_project("评分项目", "C-SCR", "明标", "scr-key").unwrap();
        s.upsert_file("A.docx", "招标文件", "sha-s", 5, 1, 50, 0, ParseStatus::Ok, "").unwrap();
        let reqs = vec![
            scoring_req("REQ-0003", Some(65.0), "1、报价：65 分（基准价 = 最低价）"),
            scoring_req("REQ-0004", Some(30.0), "2、技术方案：30 分"),
            veto_req("REQ-0005"),
        ];
        s.set_requirements(reqs).unwrap();
        let items = s.scoring_items();
        assert_eq!(items.len(), 3);
        let (_, total) = scoring_sanity(&items);
        assert!((total - 95.0).abs() < 1e-9);
        assert_eq!(s.hard_gate_requirement_ids(), vec!["REQ-0005".to_string()]);
        // 幂等：双派生全等（派生态无隐藏可变性）
        assert_eq!(s.scoring_items(), items);
    }
}
