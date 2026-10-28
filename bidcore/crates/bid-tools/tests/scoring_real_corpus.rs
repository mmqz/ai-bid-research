//! T-3 评分矩阵真件验收（issue #11 批次 v3 / V3-2）：corpus-A 引擎路径抽取 →
//! 评分项分值结构真值 + 否决项硬门。
//!
//! 真实数据面（corpus-A 主文件 + 补遗，引擎路径）：
//! - 4 条评分项**全部**带分值（30/55/15/3）——价格/技术/商务三大项合计恰 100；
//! - 第 4 条「加 3 分」为子项分（首次匹配语义）——sanity 总分 103 > 100 触发
//!   告警，人工核对兜底（确定性抽取不猜测语义，分值缺失/越界显式登记）；
//! - 否决项全量 is_hard_gate（漏响应即废标的独立闸门）。

use bid_domain::scoring::{derive_scoring_items, scoring_sanity};
use bid_parse::extract::extract_requirements;
use bid_tools::service::{BidService, ImportSpec};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
        })
}

fn spec(path: PathBuf, filename: &str, kind: &str) -> ImportSpec {
    ImportSpec { upload_ref: path, filename: filename.to_string(), kind: kind.to_string(), expected_pages: None }
}

#[test]
fn corpus_a_scoring_items_carry_real_weights() {
    let mut svc = BidService::new();
    svc.create_project("评分真件验收", "ZFCG-2026-GK-0311", "明标", "scr-real-key").unwrap();
    let report = svc.import(&[
        spec(fixtures().join("corpus-A/files/A-招标文件.docx"), "A-招标文件.docx", "招标文件"),
        spec(fixtures().join("corpus-A/files/C-补遗文件01.docx"), "C-补遗文件01.docx", "补遗"),
    ]).unwrap();
    assert_eq!(report.item_failures, 0, "双件引擎路径零失败");
    let reqs = extract_requirements(svc.parsed());
    let (total, _) = svc.set_requirements(reqs).unwrap();
    assert_eq!(total, 119, "corpus-A 双件条款面（引擎路径已知口径）");

    // T-3 真值面：评分项分值 + 否决项硬门
    let items = svc.store().scoring_items();
    let scoring: Vec<_> = items.iter().filter(|i| !i.is_hard_gate).collect();
    let hard: Vec<_> = items.iter().filter(|i| i.is_hard_gate).collect();
    assert_eq!(scoring.len(), 4, "corpus-A 评分项 4 条（真件口径）");
    assert_eq!(hard.len(), 6, "corpus-A 否决项 6 条（真件口径）——全量硬门");

    // 三大项分值真值（价格 30 / 技术 55 / 商务 15——合计恰 100）
    let weights: Vec<f64> = scoring.iter().filter_map(|i| i.weight).collect();
    assert_eq!(weights, vec![30.0, 55.0, 15.0, 3.0], "分值序列（真件锚定）");
    assert!((weights[..3].iter().sum::<f64>() - 100.0).abs() < 1e-9,
        "价格+技术+商务三大项合计恰 100（评标办法口径）");

    // 硬门与标准原文
    assert!(hard.iter().all(|i| i.weight.is_none()), "否决项无分值（漏了直接废标，不打分）");
    assert!(hard.iter().all(|i| !i.criteria.is_empty()), "否决标准原文随行（审校比对面）");

    // sanity：总分 103（含子项分 3）> 100 → 告警登记（确定性抽取不猜测语义）
    let (gaps, total_weight) = scoring_sanity(&items);
    assert!(gaps.is_empty(), "corpus-A 评分项无分值缺失");
    assert!((total_weight - 103.0).abs() < 1e-9, "总分 = 30+55+15+3（子项分入账）");

    // store 硬门清单与 T-2 blocking 联动：否决项未响应 → check_export_ready 拒绝
    let hard_ids = svc.store().hard_gate_requirement_ids();
    assert_eq!(hard_ids.len(), 6);
    svc.store_mut().build_matrix().unwrap();
    let err = svc.store().check_export_ready().unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE", "必选未响应（资格 6 + 否决 6）阻断导出");
}
