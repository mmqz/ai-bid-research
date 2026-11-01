//! T-3 评分矩阵真件验收（issue #11 批次 v3 / V3-2）：corpus-A 引擎路径抽取 →
//! 评分项分值结构真值 + 否决项硬门。
//!
//! 真实数据面（corpus-A 主文件 + 补遗，引擎路径）：
//! - 4 条评分项**全部**带分值（30/55/15/3）——价格/技术/商务三大项合计恰 100；
//! - 第 4 条「加 3 分」为子项分（首次匹配语义）——sanity 总分 103 > 100 触发
//!   告警，人工核对兜底（确定性抽取不猜测语义，分值缺失/越界显式登记）；
//! - 否决项全量 is_hard_gate（漏响应即废标的独立闸门）。
//!
//! SKIP 面（PR #24 修复批次，issue #9 机制）：真实样本缺失 → 响亮横幅 +
//! 提前 return（缺件不是业务失败，也绝不静默假装通过）。

use bid_domain::scoring::scoring_sanity;
use bid_parse::extract::extract_requirements;
use bid_parse::env_preflight::skip_if_fixture_missing;
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
    let a = fixtures().join("corpus-A/files/A-招标文件.docx");
    let c = fixtures().join("corpus-A/files/C-补遗文件01.docx");
    if skip_if_fixture_missing("corpus_a_scoring_items_carry_real_weights", &[&a, &c]) {
        return;
    }
    let mut svc = BidService::new();
    svc.create_project("评分真件验收", "ZFCG-2026-GK-0311", "明标", "scr-real-key").unwrap();
    let report = svc.import(&[
        spec(a, "A-招标文件.docx", "招标文件"),
        spec(c, "C-补遗文件01.docx", "补遗"),
    ]).unwrap();
    assert_eq!(report.item_failures, 0, "双件引擎路径零失败");
    let reqs = extract_requirements(svc.parsed());
    let (total, _) = svc.set_requirements(reqs).unwrap();
    // P0-B（否决分类信号源修复）后抽取面 119 -> 121：
    // +2 条来自「资格负面清单 / 审查表条目」句式（父条款强信号 -> 列举子项继承）。
    // 这些是真 REQ 不是噪声——评分项 4 条 / 硬门 6 条的真值锚定未变，抽取质量零损害。
    // 见 PR #45「待 Lead 裁决」段。
    assert_eq!(total, 121, "corpus-A 双件条款面（P0-B 后已知口径）");

    // T-3 真值面：评分项分值 + 否决项硬门
    let items = svc.store().scoring_items();
    let scoring: Vec<_> = items.iter().filter(|i| !i.is_hard_gate).collect();
    let hard: Vec<_> = items.iter().filter(|i| i.is_hard_gate).collect();
    assert_eq!(scoring.len(), 4, "corpus-A 评分项 4 条（真件口径）");
    // P0-B 后否决项 6 -> 13：旧实现只认「前附表 + 否决」两条件，漏掉正文
    // 编号否决条款 7 条。逐条核对（明细见 PR #45）：
    //   报价超限价 / 技术偏离表 / 暗标身份标记 / 页数超 200 / 报价超控制价 /
    //   保证金 / 签署盖章 / 工期 90 天 / 补遗控制价 465 万 —— 9 条真否决
    //   失信承诺（#3，资格面误标）+ 开标出席（#8，弱信号）—— 2 条存疑
    // 【fix/e76d9de-red-tests】13 -> 12：3.2.6 失信承诺（「须提供未被列入
    // 失信名单的书面承诺」）经 veto_lexicon 的「未被列入」正面证明剥离
    // 归位**资格条件**——PR #45 核对时已注明「资格面误标」，本修复兑现
    // 该注记（GT-06 的资格裁决同口径）；开标出席弱信号仍在否决面
    // （宁多勿漏纪律），其余 11 条真否决不变。
    assert_eq!(hard.len(), 12, "corpus-A 否决项 12 条（P0-B 后 13 条中失信承诺归位资格面）");

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
    // 同上：【fix/e76d9de-red-tests】硬门 13 -> 12——失信承诺归位资格面
    //（veto_lexicon「未被列入」正面证明剥离，PR #45 核对注记兑现）
    assert_eq!(hard_ids.len(), 12, "硬门清单与 T-3 口径一致");
    svc.store_mut().build_matrix().unwrap();
    let err = svc.store().check_export_ready().unwrap_err();
    assert_eq!(err.code(), "E_UNPROCESSABLE", "必选未响应（资格 7 + 否决 12）阻断导出");
}
