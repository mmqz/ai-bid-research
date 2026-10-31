//! pricing.rs —— B1 价格分引擎（issue #84 B1 / P0，从零到一）。
//!
//! 现状补齐：全仓无价格分算法（issue #84 实证 1：grep 零命中 + 域模型无对应
//! 类型）。报价占典型标书 35-65 分（最大权重），此前引擎对「报 X 万得几分、
//! 报多少分最高」一无所知——本模块补上这对核心能力。
//!
//! 域抽象（对齐真实评标办法的报价规则，非学术泛化）：
//! - [`PriceScoreMethod`]：**评标基准价怎么定**（基准价法/最低价法/平均价法/
//!   复合法/下浮率法五种——issue #84 B1 范围，逐 PR 交付，本 PR 落基准价法）；
//! - [`DeviationRule`]：**偏离基准价怎么扣分**（每高 1% 扣 E1、每低 1% 扣 E2、
//!   不足 1% 插入法即线性内插——亳州导则 E1/E2、康居嘉苑监理 0.2/0.1 同构；
//!   高价侧扣分通常更重）；
//! - [`PriceScoreSpec`]：方法 + 满分 + 控制价 + 偏离规则的组合规格。
//!
//! 两个核心 API（issue #84 B1 验收面）：
//! - [`PriceScoreSpec::score`]：给定报价 + 其他有效报价 → 价格分；
//! - [`PriceScoreSpec::optimal_price`]：其他有效报价给定 → 反推最优报价
//!   （基准价法最优 = 命中基准价，偏离为零即满分）。
//!
//! 真实规则锚点（可查证原文，验收对照用）：
//! - 财政部令第 87 号《政府采购货物和服务招标投标管理办法》第五十五条
//!   （mof.gov.cn / ccgp.gov.cn）：综合评分法价格分**低价优先法**，
//!   `得分 =（评标基准价/投标报价）× 100`，评标过程**不得去掉最高最低报价**；
//! - 东南大学四牌楼校区热工实验室修缮改造一期（ccgp.gov.cn 更正公告
//!   2026-08-21）：报价分 70 分（基准价命中得 69 分），基准价
//!   `C = A×Q1×K1 + B×Q2×K2`（A 为剔除后平均、B 为最高限价），
//!   K1∈{96%,97%,98%}、K2=95%、Q1∈{40%,50%,60%}；每高于基准价 1% 扣 0.9、
//!   每低 1% 扣 0.6，不足 1% 内插法，小数点后保留两位；
//! - 玉带河景观提升工程 EPC（ggzy.gov.cn，连云港 2021）：招标控制价 2480 万，
//!   方法一 `基准价 = A×K`（K∈{95%,96%,97%,98%} 开标抽取；按家数剔高剔低），
//!   方法二 `基准价 = A×K1×Q1 + B×K2×Q2`（园林绿化工程 K2=90%）；
//!   每低 1% 扣 0.5、每高 1% 扣 1.0，插入法；
//! - 康居嘉苑项目监理（ggzy.gov.cn，连云港 2021）：投标报价 44 分，
//!   方法三（平均×K）/方法四（`A×Q1 + B×Q2`，Q1∈{10%..30%}），
//!   每高 1% 扣 0.2、每低 1% 扣 0.1，插入法；
//! - 亳州市建设工程施工项目评标办法适用导则 2026 年版（ggzy.bozhou.gov.cn）：
//!   农业农村类综合评估法（合理低价）`基准价 = 评标价平均值×C`
//!   （C∈{0.980..1.000}），得分 `F − 偏差率×100×E1`（高于）/`F + 偏差率×100×E2`
//!   （低于），**E1≥2×E2**，得分为负按 0 计；房建市政类
//!   `J=(A×20%+B×35%+C×45%)×K`，其中 `A = 最高投标限价×(100%−下浮率N)`。
//!
//! 本 PR（基准价法）的验收口径说明：纯「控制价×系数」端到端评标办法未在
//! 本次调研中取得可引用原文（该形态实务中多作为复合公式的**控制价分量**
//! 出现——东南大学 B×K2 / 玉带河 B×K2 / 亳州 A=限价×(1−N)），故验收采用
//! **分量级真实对照**（三例）+ **合成场景**（测试内显式标注「合成」）+
//! 后续 PR 的退化等价（复合法 budget_weight=1 退化为基准价法）。
//!
//! 纪律（对齐仓内既定口径）：
//! - **有效报价前置**：`others` 必须是已通过有效性筛选的报价（超控制价 =
//!   无效标，87号令第六十三条——调用方先行剔除，引擎拒绝代为猜测，违反即
//!   `E_UNPROCESSABLE`）；
//! - **零 IO 纯函数**：本 crate 纪律——评标场景推演不落库、不依赖时钟；
//! - **诚实错误**：配置非法/前置违反 → `E_UNPROCESSABLE`（禁止用 NaN/0 伪装
//!   计算失败；issue 要求的 `-> f64` 签名以 `Result<f64, ToolError>` 落地，
//!   与 param.rs/matrix.rs 同款裁决面）；
//! - **得分夹取 [0, max_score]**：亳州导则「得分为负按 0 分计算」同构，
//!   超满分同样夹取（比率口径不会超，偏离口径靠 clamp 兜底）；
//! - **结果保留位数是展示层职责**：真实办法多要求「保留两位小数」，本引擎
//!   返回全精度 f64（对照表同时给出两位小数呈现），四舍五入归调用方。

use crate::ToolError;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- 类型面
/// 评标基准价的确定方法（真实评标办法报价规则的抽象——五种，逐 PR 交付）。
///
/// 「基准价怎么定」是价格分的第一半；「偏离怎么扣分」见 [`DeviationRule`]。
/// 本 PR 交付 `BenchmarkPrice`（基准价法）；后续 PR 依次交付最低价法（87号令
/// 第五十五条低价优先）、平均价法（康居嘉苑方法三）、复合法（东南大学
/// C=A×Q1×K1+B×Q2×K2 / 玉带河方法二 / 康居嘉苑方法四）、下浮率法。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PriceScoreMethod {
    /// 基准价法：评标基准价 = 控制价（[`PriceScoreSpec::budget`]）× `coefficient`。
    ///
    /// 真实分量锚点（复合公式中的控制价侧）：
    /// - 东南大学修缮：`C = A×Q1×K1 + B×Q2×K2`，K2=95% → 分量 B×0.95；
    /// - 玉带河 EPC 方法二：园林绿化工程 K2=90% → 分量 B×0.90；
    /// - 亳州导则房建市政：`A = 最高投标限价×(100%−N)`，N∈{1.5%,…,4.5%}。
    ///
    /// 最优报价 = 基准价本身（偏离为零即满分；高低两侧都扣分的线性口径下，
    /// 峰值恰在基准价处）。
    BenchmarkPrice {
        /// 基准价系数（0 < coefficient ≤ 1——真实办法均为下浮系数：87%..100%）。
        coefficient: f64,
    },
}

/// 偏离扣分规则（「等于基准价得满分，偏离按幅度扣分」的具体口径）。
///
/// 真实办法的高低价侧扣分**不对称**（亳州导则明文 E1≥2×E2；康居嘉苑
/// 0.2/0.1；东南大学 0.9/0.6；玉带河 1.0/0.5）——高于基准价罚更重，
/// 防低价抢标与高价套利双向约束。「不足 1% 按插入法」= 偏离百分比的
/// **线性**函数（连续，非阶梯）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeviationRule {
    /// 价格偏离：每**高于**基准价 1% 扣 `deduct_high` 分、每**低于** 1% 扣
    /// `deduct_low` 分，不足 1% 按插入法（线性内插）。
    Price {
        /// 高于基准价每 1% 的扣分（≥0；实务常为低价侧的 1.5-2 倍）
        deduct_high: f64,
        /// 低于基准价每 1% 的扣分（≥0；0 = 低于不罚）
        deduct_low: f64,
    },
}

/// 价格分规格（一个评标办法报价规则的完整参数化——B2 评分自评的消费面）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceScoreSpec {
    /// 基准价确定方法（五种之一）
    pub method: PriceScoreMethod,
    /// 价格分满分（0 < max_score ≤ 100——评标办法价格分占总分 100 的惯例）
    pub max_score: f64,
    /// 控制价/最高限价（None = 招标文件未设；BenchmarkPrice 必需）。
    /// 报价超过控制价 = 无效标（87号令第六十三条），由本引擎前置校验兜底。
    pub budget: Option<f64>,
    /// 偏离扣分规则
    pub deviation_rule: DeviationRule,
}

/// 单次评分的过程明细（验收对照与审计面——「手工算 vs 引擎算」的引擎侧
/// 中间量全公开：基准价/偏离度/扣分/最终分）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceScoreDetail {
    /// 评标基准价（本方法在当前报价集合下实际采用的基准值）
    pub benchmark: f64,
    /// 偏离度（价格口径：相对基准价的百分比，+为高/−为低，如 0.031 = +3.1%）
    pub deviation: f64,
    /// 扣分（≥0；偏离为零时为 0）
    pub deduction: f64,
    /// 最终得分（= clamp(max_score − deduction, 0, max_score)）
    pub score: f64,
}

// ---------------------------------------------------------------- 内部工具
fn eun(reason: impl Into<String>) -> ToolError {
    ToolError::EUnprocessable { reason: reason.into() }
}

/// 单个报价的合法性（>0 且有限；设了控制价时不得超过——超控制价 = 无效标，
/// 87号令第六十三条（四）。有效报价筛选是调用方前置，引擎只做兜底拒绝）。
fn check_price(price: f64, budget: Option<f64>, what: &str) -> Result<(), ToolError> {
    if !price.is_finite() || price <= 0.0 {
        return Err(eun(format!("PRICE_INVALID: {what} 报价必须为正的有限数（got {price}）")));
    }
    if let Some(b) = budget {
        if price > b {
            return Err(eun(format!(
                "PRICE_EXCEEDS_BUDGET: {what} 报价 {price} 超过控制价/最高限价 {b}\
                 （无效标须先行剔除——87号令第六十三条，引擎拒绝代为猜测）"
            )));
        }
    }
    Ok(())
}

fn check_others(others: &[f64], budget: Option<f64>) -> Result<(), ToolError> {
    for (i, p) in others.iter().enumerate() {
        check_price(*p, budget, &format!("others[{i}]"))?;
    }
    Ok(())
}

/// 分数夹取 [0, max]（亳州导则「得分为负按 0 分计算」同构；超满分兜底）。
fn clamp_score(raw: f64, max_score: f64) -> f64 {
    raw.clamp(0.0, max_score)
}

// ---------------------------------------------------------------- 评分面
impl PriceScoreSpec {
    /// 规格合法性（单一真源——score/optimal_price 前置调用，MCP 写入面复用）。
    pub fn validate(&self) -> Result<(), ToolError> {
        if !self.max_score.is_finite() || self.max_score <= 0.0 || self.max_score > 100.0 {
            return Err(eun(format!(
                "PRICE_SPEC_INVALID: max_score 须在 (0,100]（评标办法价格分占总分 100 的惯例），got {}",
                self.max_score
            )));
        }
        if let Some(b) = self.budget {
            if !b.is_finite() || b <= 0.0 {
                return Err(eun(format!("PRICE_SPEC_INVALID: budget 须为正的有限数，got {b}")));
            }
        }
        match &self.method {
            PriceScoreMethod::BenchmarkPrice { coefficient } => {
                if self.budget.is_none() {
                    return Err(eun(
                        "PRICE_SPEC_INVALID: BenchmarkPrice 需要 budget（控制价/最高限价）——\
                         基准价 = 控制价 × coefficient，无控制价无法定基准",
                    ));
                }
                if !coefficient.is_finite() || *coefficient <= 0.0 || *coefficient > 1.0 {
                    return Err(eun(format!(
                        "PRICE_SPEC_INVALID: BenchmarkPrice.coefficient 须在 (0,1]\
                         （真实办法均为下浮系数，如 95%/90%/84%），got {coefficient}"
                    )));
                }
            }
        }
        match &self.deviation_rule {
            DeviationRule::Price { deduct_high, deduct_low } => {
                if !deduct_high.is_finite() || *deduct_high < 0.0
                    || !deduct_low.is_finite() || *deduct_low < 0.0
                {
                    return Err(eun(format!(
                        "PRICE_SPEC_INVALID: Price 扣分须 ≥0（deduct_high={deduct_high}, \
                         deduct_low={deduct_low}）"
                    )));
                }
            }
        }
        Ok(())
    }

    /// 评标基准价（当前报价集合下的实际基准值；基准价法不依赖报价集合，
    /// 签名保持五方法统一）。
    fn benchmark(&self, _bid_price: f64, _others: &[f64]) -> Result<f64, ToolError> {
        match &self.method {
            PriceScoreMethod::BenchmarkPrice { coefficient } => {
                // validate() 已确保 budget 存在且为正
                let budget = self.budget.ok_or_else(|| {
                    eun("PRICE_SPEC_INVALID: BenchmarkPrice 需要 budget（不应到达——validate 已拦截）")
                })?;
                Ok(budget * coefficient)
            }
        }
    }

    /// 给定报价算得分（issue #84 B1 核心 API 之一）。
    ///
    /// `others` = 其他**有效**报价（超控制价的无效标须先行剔除）；基准价法
    /// 的基准值不依赖报价集合，但 others 仍参与前置校验（口径统一）。
    pub fn score(&self, bid_price: f64, others: &[f64]) -> Result<f64, ToolError> {
        Ok(self.score_detail(bid_price, others)?.score)
    }

    /// 评分明细（基准价/偏离度/扣分/最终分——「手工算 vs 引擎算」对照的
    /// 引擎侧中间量，MCP 审计面同款消费）。
    pub fn score_detail(
        &self,
        bid_price: f64,
        others: &[f64],
    ) -> Result<PriceScoreDetail, ToolError> {
        self.validate()?;
        check_price(bid_price, self.budget, "己方")?;
        check_others(others, self.budget)?;
        let benchmark = self.benchmark(bid_price, others)?;
        // 偏离度：相对基准价的百分比（+高 / −低；87号令/亳州导则/康居嘉苑同口径）
        let deviation = (bid_price - benchmark) / benchmark;
        let deduction = match &self.deviation_rule {
            DeviationRule::Price { deduct_high, deduct_low } => {
                if deviation > 0.0 {
                    deviation * 100.0 * deduct_high
                } else {
                    (-deviation) * 100.0 * deduct_low
                }
            }
        };
        let score = clamp_score(self.max_score - deduction, self.max_score);
        Ok(PriceScoreDetail { benchmark, deviation, deduction, score })
    }

    /// 反推最优报价（issue #84 B1 核心 API 之二）。
    ///
    /// 基准价法：最优 = 基准价本身（双向扣分的线性口径下峰值恰在偏离为零处；
    /// 低价侧不罚时低于基准的等分区间取最高价即基准价——收益最大）。
    /// 基准价 = 控制价×系数 ≤ 控制价，天然不越限价。
    pub fn optimal_price(&self, others: &[f64]) -> Result<f64, ToolError> {
        self.validate()?;
        check_others(others, self.budget)?;
        match &self.method {
            PriceScoreMethod::BenchmarkPrice { .. } => {
                // benchmark 不依赖报价集合；coeff ≤ 1 保证 ≤ budget
                Ok(self.benchmark(f64::NAN, &[])?)
            }
        }
    }
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;

    fn benchmark_spec(coefficient: f64, max_score: f64, budget: f64,
                      deduct_high: f64, deduct_low: f64) -> PriceScoreSpec {
        PriceScoreSpec {
            method: PriceScoreMethod::BenchmarkPrice { coefficient },
            max_score,
            budget: Some(budget),
            deviation_rule: DeviationRule::Price { deduct_high, deduct_low },
        }
    }

    fn assert_close(a: f64, b: f64, ctx: &str) {
        assert!((a - b).abs() < 1e-9, "{ctx}: 引擎 {a} vs 期望 {b}");
    }

    // ---------------- 校验面（诚实错误——E_UNPROCESSABLE 全枚举）

    #[test]
    fn validation_rejects_bad_specs_and_prices() {
        // 无控制价：基准价法无法定基准
        let mut spec = benchmark_spec(0.97, 30.0, 1000.0, 0.9, 0.6);
        spec.budget = None;
        assert!(spec.score(970.0, &[]).is_err(), "无 budget 必须拒绝");

        // 系数越界（真实办法均为下浮系数 ≤1；>1 = 上浮基准不存在）
        spec = benchmark_spec(1.2, 30.0, 1000.0, 0.9, 0.6);
        assert!(spec.score(970.0, &[]).is_err(), "coefficient>1 必须拒绝");
        spec = benchmark_spec(0.0, 30.0, 1000.0, 0.9, 0.6);
        assert!(spec.score(970.0, &[]).is_err(), "coefficient=0 必须拒绝");

        // 满分越界（评标办法价格分 ≤100 惯例——防 8600 误配成 86）
        spec = benchmark_spec(0.97, 101.0, 1000.0, 0.9, 0.6);
        assert!(spec.score(970.0, &[]).is_err(), "max_score>100 必须拒绝");
        spec = benchmark_spec(0.97, 0.0, 1000.0, 0.9, 0.6);
        assert!(spec.score(970.0, &[]).is_err(), "max_score=0 必须拒绝");

        // 扣分为负
        spec = benchmark_spec(0.97, 30.0, 1000.0, -0.1, 0.6);
        assert!(spec.score(970.0, &[]).is_err(), "负扣分必须拒绝");

        // 报价非法：0 / 负 / 非有限
        spec = benchmark_spec(0.97, 30.0, 1000.0, 0.9, 0.6);
        assert!(spec.score(0.0, &[]).is_err(), "报价 0 必须拒绝");
        assert!(spec.score(-5.0, &[]).is_err(), "负报价必须拒绝");
        assert!(spec.score(f64::NAN, &[]).is_err(), "NaN 报价必须拒绝");

        // 超控制价 = 无效标兜底（己方与 others 同口径）
        assert!(spec.score(1000.5, &[]).is_err(), "己方超控制价必须拒绝");
        assert!(spec.score(970.0, &[990.0, 1200.0]).is_err(), "others 超控制价必须拒绝");
        assert!(spec.score(970.0, &[0.0]).is_err(), "others 报价 0 必须拒绝");
    }

    // ---------------- 真实分量对照（B1 验收——三例可查证原文）

    /// 真实分量 1/3：东南大学修缮（ccgp.gov.cn 更正公告 2026-08-21）——
    /// `C = A×Q1×K1 + B×Q2×K2` 的控制价分量 B×K2：
    /// 最高限价 B = 1200 万，K2 = 95% → 分量基准 = 1200 × 0.95 = 1140 万。
    /// 偏离扣分：每高 1% 扣 0.9、每低 1% 扣 0.6（内插法）。
    #[test]
    fn real_component_seu_budget_side() {
        // 东南大学规则的控制价侧分量（A×Q1×K1 部分属复合法，PR4 全公式对照）
        let spec = benchmark_spec(0.95, 69.0, 1200.0, 0.9, 0.6);
        let d = spec.score_detail(1100.0, &[980.0, 1010.0, 1035.0, 1060.0, 1090.0, 1120.0, 1160.0])
            .expect("合法输入");
        // 手工：基准 = 1200×0.95 = 1140
        assert_close(d.benchmark, 1200.0 * 0.95, "东南大学 B×K2 分量基准");
        // 手工：1100 低于基准 40 → 偏离 = −40/1140 = −3.508772%（低价侧）
        let dev_hand = (1100.0 - 1140.0) / 1140.0;
        assert_close(d.deviation, dev_hand, "偏离度");
        // 手工：69 − 3.508772×0.6 = 69 − 2.105263 = 66.894737
        let score_hand = 69.0 - (-dev_hand) * 100.0 * 0.6;
        assert_close(d.score, score_hand, "东南大学分量口径得分");
        assert!((d.score - 66.894737).abs() < 1e-6, "两位小数呈现: {:.2}", d.score);
    }

    /// 真实分量 2/3：玉带河景观提升工程 EPC（ggzy.gov.cn，连云港 2021）——
    /// 方法二 `基准价 = A×K1×Q1 + B×K2×Q2` 的控制价分量 B×K2：
    /// 招标控制价 B = 2480 万，园林绿化工程 K2 = 90% → 分量基准 = 2480×0.90 = 2232 万。
    #[test]
    fn real_component_yudaihe_budget_side() {
        let spec = benchmark_spec(0.90, 84.0, 2480.0, 1.0, 0.5);
        let d = spec.score_detail(2232.0, &[2000.0, 2150.0, 2250.0, 2400.0]).expect("合法输入");
        // 手工：基准 = 2480×0.90 = 2232
        assert_close(d.benchmark, 2480.0 * 0.90, "玉带河 B×K2 分量基准");
        // 命中基准 → 偏离 0 → 满分
        assert_close(d.deviation, 0.0, "命中基准");
        assert_close(d.score, 84.0, "玉带河报价满分 84");
        // 手工：报价 2300 高于基准 68 → 偏离 68/2232 = 3.046595% × 1.0
        //      → 84 − 3.046595 = 80.953405
        let d2 = spec.score_detail(2300.0, &[2000.0, 2150.0, 2250.0, 2400.0]).unwrap();
        let dev_hand = (2300.0 - 2232.0) / 2232.0;
        assert_close(d2.score, 84.0 - dev_hand * 100.0 * 1.0, "玉带河高价侧扣分");
        assert!((d2.score - 80.953405).abs() < 1e-6, "两位小数呈现: {:.2}", d2.score);
    }

    /// 真实分量 3/3：亳州市评标办法适用导则 2026 年版（ggzy.bozhou.gov.cn）——
    /// 房建市政 `J=(A×20%+B×35%+C×45%)×K` 的 A 分量：`A = 最高投标限价×(100%−N)`，
    /// N∈{1.5%,…,4.5%}。取 N=3%、限价 5000 万 → A = 5000×0.97 = 4850 万。
    /// （J 全式含随机抽取的 B 分量——「有效投标报价范围内除 C 值外任意一个」，
    /// 非确定性，不入引擎；A 分量为确定性控制价×系数形态。）
    #[test]
    fn real_component_bozhou_guide_a_side() {
        let spec = benchmark_spec(0.97, 65.0, 5000.0, 1.5, 0.75); // 亳州口径：高=1.5×E，低=E
        let d = spec.score_detail(4850.0, &[4700.0, 4900.0, 4950.0]).expect("合法输入");
        // 手工：A = 5000×(1−3%) = 4850
        assert_close(d.benchmark, 5000.0 * 0.97, "亳州 A=限价×(1−N) 分量基准");
        assert_close(d.score, 65.0, "命中 A 分量得满分");
        // 手工：报价 5000 高于 4850 达 150 → 偏离 150/4850 = 3.092784% × 1.5
        //      → 65 − 4.639175 = 60.360825
        let d2 = spec.score_detail(5000.0, &[4700.0, 4900.0, 4950.0]).unwrap();
        let dev_hand = (5000.0 - 4850.0) / 4850.0;
        assert_close(d2.score, 65.0 - dev_hand * 100.0 * 1.5, "亳州高价侧 1.5×E 扣分");
        assert!((d2.score - 60.360825).abs() < 1e-6, "两位小数呈现: {:.2}", d2.score);
    }

    // ---------------- 合成场景（显式标注——非真实办法引用，验证引擎数学面）

    /// 合成场景：满分 30 / 控制价 1000 / 系数 0.97 / 高 0.9 低 0.6。
    /// 手工全表 vs 引擎全表（含扣分为负夹 0 的下界）。
    #[test]
    fn synthetic_scenario_full_table() {
        let spec = benchmark_spec(0.97, 30.0, 1000.0, 0.9, 0.6);
        // 基准 = 1000×0.97 = 970
        // (a) 命中基准：970 → 偏离 0 → 30 分
        let a = spec.score_detail(970.0, &[950.0, 980.0]).unwrap();
        assert_close(a.benchmark, 970.0, "基准价");
        assert_close(a.score, 30.0, "命中基准满分");

        // (b) 高于基准：1000 → 偏离 = +30/970 = +3.092784% × 0.9
        //     手工：30 − 3.092784×0.9 = 30 − 2.783505 = 27.216495
        let b = spec.score_detail(1000.0, &[950.0, 980.0]).unwrap();
        assert_close(b.score, 30.0 - (30.0 / 970.0) * 100.0 * 0.9, "高价侧");
        assert!((b.score - 27.216495).abs() < 1e-6, "呈现: {:.2}", b.score);

        // (c) 低于基准（低价侧扣分轻——不对称）：940 → −3.092784% × 0.6
        //     手工：30 − 1.855670 = 28.144330（比报价 1000 得分高）
        let c = spec.score_detail(940.0, &[950.0, 980.0]).unwrap();
        assert_close(c.score, 30.0 - (30.0 / 970.0) * 100.0 * 0.6, "低价侧");
        assert!(c.score > b.score, "不对称扣分：低价侧罚轻于高价侧");

        // (d) 大幅偏离 → 扣成负分 → 夹 0（亳州「得分为负按 0 计算」同构）
        //     手工：200 → −770/970 = −79.381443% × 0.6 = −47.628866 → 30−47.63 < 0 → 0
        let d = spec.score_detail(200.0, &[950.0, 980.0]).unwrap();
        assert_close(d.score, 0.0, "负分下界夹 0");

        // (e) 明细字段一致性：deviation/deduction/score 链路自洽
        assert_close(b.deduction, (30.0 / 970.0) * 100.0 * 0.9, "扣分中间量");
        assert_close(b.score, (30.0 - b.deduction).clamp(0.0, 30.0), "链路自洽");
    }

    /// 合成场景：低价侧不罚（deduct_low = 0）——等分区间取最高价（基准价），
    /// 收益最大化的诚实口径。
    #[test]
    fn synthetic_zero_low_deduction_prefers_benchmark() {
        let spec = benchmark_spec(0.97, 30.0, 1000.0, 0.9, 0.0);
        // 低于基准不罚：940 与 970 同分
        let lo = spec.score_detail(940.0, &[]).unwrap();
        let hit = spec.score_detail(970.0, &[]).unwrap();
        assert_close(lo.score, hit.score, "低于不罚：等分");
        // 最优仍报基准价（等分区间最高价）
        assert_close(spec.optimal_price(&[940.0, 980.0]).unwrap(), 970.0, "等分区间取最高价");
    }

    // ---------------- 最优报价（反推）

    /// 基准价法最优 = 基准价（双向扣分线性口径的峰值；不依赖报价集合；
    /// 天然 ≤ 控制价）。
    #[test]
    fn optimal_price_hits_benchmark_exactly() {
        let spec = benchmark_spec(0.97, 30.0, 1000.0, 0.9, 0.6);
        // 空竞争集合：基准价法基准不依赖 others → 最优仍是 970
        let x0 = spec.optimal_price(&[]).unwrap();
        assert_close(x0, 970.0, "空 others");
        // 有竞争集合：不变（基准价法基准与报价集合无关）
        let x1 = spec.optimal_price(&[950.0, 960.0, 980.0]).unwrap();
        assert_close(x1, 970.0, "带 others");
        // 命中验证：最优报价的得分 = 满分（偏离 0）
        let d = spec.score_detail(x1, &[950.0, 960.0, 980.0]).unwrap();
        assert_close(d.score, 30.0, "最优报价得满分");
        assert_close(d.deviation, 0.0, "最优报价偏离为零");
        // 不越限价
        assert!(x1 <= 1000.0, "最优报价 ≤ 控制价");
    }

    /// 最优报价的全局验证：网格暴力搜索不超过解析最优的得分（上界性）。
    #[test]
    fn optimal_price_is_global_by_grid_search() {
        let spec = benchmark_spec(0.97, 30.0, 1000.0, 0.9, 0.6);
        let x = spec.optimal_price(&[900.0, 950.0]).unwrap();
        let best = spec.score(x, &[900.0, 950.0]).unwrap();
        // 网格 1 元步长全扫（0, 1000]：任何报价的得分不得超过最优得分
        let mut i = 1;
        while (i as f64) <= 1000.0 {
            let s = spec.score(i as f64, &[900.0, 950.0]).unwrap();
            assert!(s <= best + 1e-9, "网格点 {i} 得分 {s} 超过最优 {best}");
            i += 1;
        }
    }

    /// 纯函数性/幂等：同输入双算全等（派生态无隐藏可变性）。
    #[test]
    fn score_is_deterministic() {
        let spec = benchmark_spec(0.95, 69.0, 1200.0, 0.9, 0.6);
        let a = spec.score_detail(1100.0, &[980.0, 1160.0]).unwrap();
        let b = spec.score_detail(1100.0, &[980.0, 1160.0]).unwrap();
        assert_eq!(a, b, "双算全等");
    }

    /// B1 验收报告（手工算 vs 引擎算对照表——`cargo test pricing -- --nocapture`
    /// 查看完整表；断言与表同源，打印仅供 PR/审计引用）。
    #[test]
    fn acceptance_report_benchmark_price() {
        // —— 真实分量（三例可查证原文，详见各 real_component_* 测试）——
        let cases = [
            ("东南大学修缮 B×K2", 0.95, 1200.0, 69.0, 0.9, 0.6, 1100.0),
            ("玉带河 EPC B×K2", 0.90, 2480.0, 84.0, 1.0, 0.5, 2300.0),
            ("亳州导则 A=限价×(1−N)", 0.97, 5000.0, 65.0, 1.5, 0.75, 5000.0),
        ];
        for (name, k, b, m, dh, dl, bid) in cases {
            let spec = benchmark_spec(k, m, b, dh, dl);
            let d = spec.score_detail(bid, &[]).unwrap();
            let dev = (bid - b * k) / (b * k);
            let hand = m - dev.abs() * 100.0 * if dev > 0.0 { dh } else { dl };
            assert!((d.score - hand).abs() < 1e-9, "{name}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| {name} | 基准 {:.2}（= {b}×{k}） | 报价 {bid:.2} | 偏离 {:+.6}% | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                d.benchmark, d.deviation * 100.0, d.score
            );
        }
        // —— 合成场景（标注：非真实办法引用）——
        let spec = benchmark_spec(0.97, 30.0, 1000.0, 0.9, 0.6);
        for bid in [970.0, 1000.0, 940.0, 200.0] {
            let d = spec.score_detail(bid, &[950.0, 980.0]).unwrap();
            let dev = (bid - 970.0) / 970.0;
            let hand = (30.0 - dev.abs() * 100.0 * if dev > 0.0 { 0.9 } else { 0.6 })
                .clamp(0.0, 30.0);
            assert!((d.score - hand).abs() < 1e-9, "合成 {bid}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| 合成场景 | 基准 970.00（= 1000×0.97） | 报价 {bid:.2} | 偏离 {:+.6}% | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                d.deviation * 100.0, d.score
            );
        }
        let opt = spec.optimal_price(&[950.0, 980.0]).unwrap();
        assert!((opt - 970.0).abs() < 1e-9, "最优报价 = 基准价");
        println!("| 合成场景 | 最优报价反推 | — | — | 手工 970.00 | 引擎 {opt:.6} | 一致 |");
    }
}
