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
/// 已交付：`BenchmarkPrice`（基准价法）/ `LowestPrice`（最低价法）/
/// `AveragePrice`（平均价法）/ `Composite`（复合法）；待交付：下浮率法。
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
    /// 最低价法：评标基准价 = **全部有效报价（含己方）的最低值**。
    ///
    /// 真实锚点：
    /// - 财政部令第 87 号第五十五条（政采综合评分法价格分低价优先法）：
    ///   「满足招标文件要求且投标价格最低的投标报价为评标基准价，其价格分为
    ///   满分。其他投标人的价格分统一按照下列公式计算：投标报价得分 =
    ///   （评标基准价/投标报价）× 100」且「**评标过程中，不得去掉报价中的
    ///   最高报价和最低报价**」——比率口径配 [`DeviationRule::Ratio`]；
    /// - 康居嘉苑项目监理方法一（ggzy.gov.cn）：最低价基准 + 每高 1% 扣 0.2、
    ///   每低 1% 扣 0.1、插入法（44 分）——偏离扣分口径配 [`DeviationRule::Price`]。
    ///
    /// 注：己方报价 ≥ 最低价恒成立（基准含己方），故己方偏离度 ≥ 0——
    /// 低价侧扣分对最低价基准不可达（对己方），保留通用公式不特殊化。
    ///
    /// 最优报价 = min(others)（并列最低即满分，取等分区间最高价；无竞争报价
    /// 且无控制价时无界拒绝，无竞争报价有控制价时取控制价）。
    LowestPrice,
    /// 平均价法：评标基准价 = **全部有效报价（含己方）算术平均** × `coefficient`。
    ///
    /// 真实锚点：
    /// - 康居嘉苑项目监理方法三（ggzy.gov.cn，连云港 2021）：「以有效投标文件
    ///   的投标报价算术平均值为 A，评标基准价 = A×K」，K∈{95%,96%,97%,98%}
    ///   开标抽取；每高 1% 扣 0.2、每低 1% 扣 0.1，插入法（44 分）；
    /// - 亳州市评标办法适用导则 2026（ggzy.bozhou.gov.cn）农业农村类
    ///   综合评估法（合理低价）：「评标价平均值×系数 C 为评标基准价」，
    ///   C∈{0.980,0.985,0.990,0.995,1.000}；F−偏差率×100×E1（高于）/
    ///   F+偏差率×100×E2（低于），E1≥2×E2，负分按 0。
    ///
    /// 注：康居方法三的按家数剔除（7≤n<10 去 1 高 1 低、n≥10 去 2 高 2 低）
    /// 属复合法的剔除面（PR4 交付）——本方法为纯平均（对应办法中 n<7 或
    /// 剔除数为 0 的形态）。
    ///
    /// 最优报价（闭式解）：`x* = coefficient × S / (n − coefficient)`，
    /// S = Σothers，n = 全部家数（含己方）——解 `x = c·(S+x)/n` 的不动点，
    /// 命中基准价（偏离为零/比率为一）；超出控制价时夹到控制价。
    /// 唯一投标人（n=1）：得分与报价无关（c<1 恒偏高 (1−c)/c；c=1 恒满分）
    /// → 有控制价取控制价，无控制价无界拒绝。
    AveragePrice {
        /// 平均值系数（0 < coefficient ≤ 1——真实办法均为下浮系数）。
        coefficient: f64,
    },
    /// 复合法：剔除后平均价 A 与控制价 B 的加权复合（真实工程评标的主流形态）——
    /// `基准价 = A × avg_coefficient × (1 − budget_weight)
    ///          + 控制价 × budget_coefficient × budget_weight`。
    ///
    /// 真实锚点（四套可查证原文，全部为本方法的参数化实例）：
    /// - **东南大学修缮**（ccgp.gov.cn 更正公告 2026-08-21）：
    ///   `C = A×Q1×K1 + B×Q2×K2`，K1∈{96%,97%,98%}、K2=95%、Q1∈{40%,50%,60%}，
    ///   7≤n<10 去 1 高 1 低 / n≥10 去 2 高 2 低，基准命中得 69 分（满分 70），
    ///   每高 1% 扣 0.9 / 每低 1% 扣 0.6，内插法；
    /// - **玉带河 EPC 方法二**（ggzy.gov.cn）：`A×K1×Q1 + B×K2×Q2`，
    ///   园林绿化工程 K2=90%，控制价 2480 万；
    /// - **玉带河 EPC 方法一**（同上）：`A×K`（budget_weight=0 退化），
    ///   4-6 家只剔最高（trim_high=1/trim_low=0），每低 1% 扣 0.5/每高 1% 扣 1.0；
    /// - **康居嘉苑监理方法四**（ggzy.gov.cn）：`A×Q1 + B×Q2`，
    ///   Q1∈{10%,…,30%}（两侧系数均为 1），每高 1% 扣 0.2/每低 1% 扣 0.1（44 分）。
    ///
    /// 退化等价（测试锁定）：budget_weight=1 ⟺ [`PriceScoreMethod::BenchmarkPrice`]；
    /// budget_weight=0 且不剔除 ⟺ [`PriceScoreMethod::AveragePrice`]。
    ///
    /// 最优报价：分段线性峰值的**候选集精确求解**——三类不动点（带内闭式解 /
    /// 高位剔除区常量基准 / 低位剔除区常量基准）、全部竞争报价（分段边界）
    /// 与控制价上限，逐候选评分取最优（同分取高价）。基准价（x）随分段连续，
    /// 峰值必在不动点或分段边界——候选集完备，无需数值逼近。
    ///
    /// 注：按家数动态定剔除数（如「n≥7 去 20% 四舍五入」）是规则文本→参数
    /// 的解析职责（bid-parse 层），本引擎消费确定的剔除数。
    Composite {
        /// 剔除最高报价的家数（0 = 不剔高）
        trim_high: usize,
        /// 剔除最低报价的家数（0 = 不剔低）
        trim_low: usize,
        /// 控制价权重 Q2 ∈ [0,1]（0 = 纯均价，1 = 纯控制价——即基准价法）
        budget_weight: f64,
        /// 控制价侧系数 K2（0 < c ≤ 1，如玉带河园林绿化 0.90/东南大学 0.95）
        budget_coefficient: f64,
        /// 平均价侧系数 K1（0 < c ≤ 1，如康居方法四 1.0/东南大学 0.97）
        avg_coefficient: f64,
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
    /// 比率口径（87号令第五十五条）：得分 =（基准价/报价）× 满分。
    ///
    /// 报价 = 基准价时恰满分；高于基准价线性折减（低价侧天然不罚——
    /// 比率 ≤ 1 时截到满分）。政采综合评分法价格分强制此口径。
    Ratio,
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
            // 最低价法：基准价 = 全集合最低，无额外参数（87号令§55）；
            // 控制价可选（仅作无效标判定与无竞争报价时的最优上界）
            PriceScoreMethod::LowestPrice => {}
            PriceScoreMethod::AveragePrice { coefficient } => {
                if !coefficient.is_finite() || *coefficient <= 0.0 || *coefficient > 1.0 {
                    return Err(eun(format!(
                        "PRICE_SPEC_INVALID: AveragePrice.coefficient 须在 (0,1]\
                         （真实办法均为下浮系数，如康居方法三 K∈95%..98%/亳州 C∈98%..100%），got {coefficient}"
                    )));
                }
            }
            PriceScoreMethod::Composite { budget_weight, budget_coefficient,
                                          avg_coefficient, .. } => {
                if *budget_weight > 0.0 && self.budget.is_none() {
                    return Err(eun(
                        "PRICE_SPEC_INVALID: Composite.budget_weight > 0 需要 budget（控制价）——\
                         复合公式的控制价分量无值可用",
                    ));
                }
                if !budget_weight.is_finite() || *budget_weight < 0.0 || *budget_weight > 1.0 {
                    return Err(eun(format!(
                        "PRICE_SPEC_INVALID: Composite.budget_weight 须在 [0,1]（Q2 权重），got {budget_weight}"
                    )));
                }
                for (name, c) in [("budget_coefficient", budget_coefficient),
                                  ("avg_coefficient", avg_coefficient)] {
                    if !c.is_finite() || *c <= 0.0 || *c > 1.0 {
                        return Err(eun(format!(
                            "PRICE_SPEC_INVALID: Composite.{name} 须在 (0,1]（下浮系数），got {c}"
                        )));
                    }
                }
                // 注：剔除数与报价家数的适配（剔除后至少剩 1 家）在 benchmark
                // 计算时校验——validate 时不知道报价集合大小
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
            // 比率口径（87号令§55）：得分 =（基准/报价）×满分，无参数
            DeviationRule::Ratio => {}
        }
        Ok(())
    }

    /// 评标基准价（当前报价集合下的实际基准值；基准价法不依赖报价集合，
    /// 签名保持五方法统一）。
    fn benchmark(&self, bid_price: f64, others: &[f64]) -> Result<f64, ToolError> {
        match &self.method {
            PriceScoreMethod::BenchmarkPrice { coefficient } => {
                // validate() 已确保 budget 存在且为正
                let budget = self.budget.ok_or_else(|| {
                    eun("PRICE_SPEC_INVALID: BenchmarkPrice 需要 budget（不应到达——validate 已拦截）")
                })?;
                Ok(budget * coefficient)
            }
            // 87号令第五十五条：基准价 = 满足要求且最低的投标报价。
            // 集合 = 己方 + others（含己方——「全部有效投标文件」口径，
            // 评标过程不得去掉最高最低，87号令明文）。
            PriceScoreMethod::LowestPrice => {
                let mut min = bid_price;
                for p in others {
                    if *p < min {
                        min = *p;
                    }
                }
                Ok(min)
            }
            // 平均价法：A = 全部有效报价（含己方）算术平均，基准 = A × coefficient
            // （康居方法三 / 亳州农业农村「平均值×系数 C」同构）
            PriceScoreMethod::AveragePrice { coefficient } => {
                let n = (others.len() + 1) as f64;
                let sum: f64 = others.iter().sum::<f64>() + bid_price;
                Ok(sum / n * coefficient)
            }
            // 复合法：剔高剔低后平均 A 与控制价 B 的加权复合
            //   基准 = A × avg_coefficient × (1 − budget_weight)
            //        + B × budget_coefficient × budget_weight
            PriceScoreMethod::Composite { trim_high, trim_low, budget_weight,
                                          budget_coefficient, avg_coefficient } => {
                let mut all: Vec<f64> = others.to_vec();
                all.push(bid_price);
                all.sort_by(|a, b| a.partial_cmp(b).expect("价格已校验为有限正数"));
                let n = all.len();
                // 剔除后至少剩 1 家（否则基准无定义——诚实错误而非除零）
                if n <= trim_high + trim_low {
                    return Err(eun(format!(
                        "PRICE_SPEC_INVALID: Composite 剔除数超过报价家数（{n} 家剔最高 {trim_high} + 最低 {trim_low}）——剔除后无剩余报价，基准无定义"
                    )));
                }
                let kept = n - trim_high - trim_low;
                let sum: f64 = all[*trim_low..n - trim_high].iter().sum();
                let a = sum / kept as f64;
                let budget_part = if *budget_weight > 0.0 {
                    self.budget.unwrap_or(0.0) * budget_coefficient * budget_weight
                } else {
                    0.0
                };
                Ok(a * avg_coefficient * (1.0 - budget_weight) + budget_part)
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
        let (deduction, score) = match &self.deviation_rule {
            DeviationRule::Price { deduct_high, deduct_low } => {
                let d = if deviation > 0.0 {
                    deviation * 100.0 * deduct_high
                } else {
                    (-deviation) * 100.0 * deduct_low
                };
                (d, clamp_score(self.max_score - d, self.max_score))
            }
            // 87号令第五十五条：得分 =（基准价/报价）× 满分。
            // 比率 > 1（报价低于基准）截到满分；比率 ≥ 0 保证下界天然不破。
            DeviationRule::Ratio => {
                let raw = benchmark / bid_price * self.max_score;
                let s = clamp_score(raw, self.max_score);
                ((self.max_score - s).max(0.0), s)
            }
        };
        Ok(PriceScoreDetail { benchmark, deviation, deduction, score })
    }

    /// 反推最优报价（issue #84 B1 核心 API 之二）。
    ///
    /// 基准价法：最优 = 基准价本身（双向扣分的线性口径下峰值恰在偏离为零处；
    /// 低价侧不罚时低于基准的等分区间取最高价即基准价——收益最大）。
    /// 基准价 = 控制价×系数 ≤ 控制价，天然不越限价。
    ///
    /// 最低价法：并列最低即满分（基准含己方，报价 ≤ min(others) 时基准 =
    /// 己方报价、偏离为零/比率为一）——取等分区间最高价 min(others)。
    /// 无竞争报价（others 空）：有控制价取控制价（等分区间上限），
    /// 无控制价无界拒绝（诚实错误——不假装知道上限）。
    pub fn optimal_price(&self, others: &[f64]) -> Result<f64, ToolError> {
        self.validate()?;
        check_others(others, self.budget)?;
        match &self.method {
            PriceScoreMethod::BenchmarkPrice { .. } => {
                // benchmark 不依赖报价集合；coeff ≤ 1 保证 ≤ budget
                Ok(self.benchmark(f64::NAN, &[])?)
            }
            PriceScoreMethod::LowestPrice => {
                let min_others = others.iter().cloned().fold(f64::INFINITY, f64::min);
                if min_others.is_finite() {
                    Ok(min_others)
                } else if let Some(b) = self.budget {
                    // 唯一投标人：任意报价均为最低（基准 = 己方）→ 取上限
                    Ok(b)
                } else {
                    Err(eun(
                        "PRICE_UNBOUNDED: LowestPrice 无竞争报价且无控制价——任意报价都得满分，\
                         最优报价无上界（提供 others 或 budget）",
                    ))
                }
            }
            // 平均价法闭式解：x* = c·S/(n−c)（解 x = c·(S+x)/n 的不动点）。
            // 对 Price（含不对称/低价不罚）与 Ratio 口径同为最优：
            // - Price：峰值在偏离为零处（x* 恰命中基准价）；
            // - Ratio：x ≤ x* 时基准 ≥ 报价 → 比率 ≥ 1 截满分，x* 为等分区间最高价。
            PriceScoreMethod::AveragePrice { coefficient } => {
                if others.is_empty() {
                    // 唯一投标人：得分与报价无关（c<1 恒偏高 (1−c)/c；c=1 恒满分）
                    if let Some(b) = self.budget {
                        return Ok(b);
                    }
                    return Err(eun(
                        "PRICE_UNBOUNDED: AveragePrice 无竞争报价且无控制价——得分与报价无关，\
                         最优报价无上界（提供 others 或 budget）",
                    ));
                }
                let s: f64 = others.iter().sum();
                let n = (others.len() + 1) as f64;
                let x = coefficient * s / (n - coefficient);
                // 不变量（c ≤ 1）：x* ≤ c·m·M/(m+1−c) ≤ M = max(others) ≤ 控制价
                //   ——闭式解永不超过最高竞争报价，无越限价风险；min 为防御性保护
                Ok(match self.budget {
                    Some(b) if x > b => b,
                    _ => x,
                })
            }
            // 复合法：分段线性峰值的候选集精确求解。
            //
            // 得分函数在 x 的每个「名次段」内是仿射基准下的分段线性函数，且
            // 基准价（x）随名次段连续（边界处剔出/剔入的互换保持集合值不变），
            // 故峰值必落在：不动点（x = 基准（x））或分段边界（某竞争报价）或
            // 控制价夹取——三类不动点 + 全部竞争报价 + 控制价构成完备候选集：
            //   ① 带内不动点：x 不被剔除时，被保留的竞争报价固定为
            //      os[trim_low..m−trim_high]，闭式解
            //      x_in = [a·S_k + b·k1] / (k1 − a)，k1 = 保留家数（含己方）；
            //   ② 高位剔除区不动点：x 为最高且被剔（trim_high ≥ 1）时基准为
            //      常量 B_top（保留竞争报价多保一家顶部），x = B_top；
            //   ③ 低位剔除区不动点：对称地 x = B_bot；
            //   ④ 全部竞争报价（分段边界）与控制价（上限夹取）。
            // 逐候选评分取最优，同分取高价（收益最大）——无数值逼近，确定性。
            PriceScoreMethod::Composite { trim_high, trim_low, budget_weight,
                                          budget_coefficient, avg_coefficient } => {
                let m = others.len();
                if m < trim_high + trim_low {
                    return Err(eun(format!(
                        "PRICE_SPEC_INVALID: Composite 剔除数超过报价家数（{} 家剔 {}/{}）",
                        m + 1, trim_high, trim_low
                    )));
                }
                let mut os: Vec<f64> = others.to_vec();
                os.sort_by(|a, b| a.partial_cmp(b).expect("价格已校验为有限正数"));
                let a = avg_coefficient * (1.0 - budget_weight);   // 平均价侧系数×权重
                let b = if *budget_weight > 0.0 {                    // 控制价侧常量分量
                    self.budget.unwrap_or(0.0) * budget_coefficient * budget_weight
                } else {
                    0.0
                };
                let k1 = (m + 1 - trim_high - trim_low) as f64;     // 保留家数（含己方）
                // ① 带内不动点（被保留竞争报价固定：os[trim_low..m−trim_high]）
                let s_k: f64 = os[*trim_low..m - trim_high].iter().sum();
                let mut cands: Vec<f64> = Vec::with_capacity(m + 5);
                if k1 - a != 0.0 {
                    cands.push((a * s_k + b * k1) / (k1 - a));
                }
                // ②/③ 高/低位剔除区常量基准（x 被剔时基准与 x 无关，不动点即常量）
                if *trim_high > 0 {
                    let s_top: f64 = os[*trim_low..m - trim_high + 1].iter().sum();
                    cands.push(a * s_top / k1 + b);
                }
                if *trim_low > 0 {
                    let s_bot: f64 = os[*trim_low - 1..m - trim_high].iter().sum();
                    cands.push(a * s_bot / k1 + b);
                }
                // ④ 分段边界（全部竞争报价）与控制价上限
                cands.extend_from_slice(&os);
                if let Some(budget) = self.budget {
                    cands.push(budget);
                }
                // 过滤 + 逐候选评分，argmax（同分取高价）
                let mut best: Option<(f64, f64)> = None; // (score, price)
                for p in cands {
                    if !p.is_finite() || p <= 0.0 {
                        continue;
                    }
                    if let Some(budget) = self.budget {
                        if p > budget {
                            continue;
                        }
                    }
                    let s = self.score(p, others)?;
                    let take = match best {
                        None => true,
                        // 同分（容差内）取高价——收益最大
                        Some((bs, bp)) => s > bs + 1e-12 || (s > bs - 1e-12 && p > bp),
                    };
                    if take {
                        best = Some((s, p));
                    }
                }
                best.map(|(_, p)| p).ok_or_else(|| {
                    eun("PRICE_UNBOUNDED: Composite 无可用候选（无竞争报价且无控制价）——\
                         提供预算或竞争报价")
                })
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

    // ================================================================ 最低价法（PR2）

    fn lowest_spec(max_score: f64, rule: DeviationRule) -> PriceScoreSpec {
        PriceScoreSpec {
            method: PriceScoreMethod::LowestPrice,
            max_score,
            budget: None,
            deviation_rule: rule,
        }
    }

    /// 真实规则 1：财政部令第 87 号第五十五条（mof.gov.cn / ccgp.gov.cn）——
    /// 政采综合评分法价格分**低价优先法**：
    /// 「满足招标文件要求且投标价格最低的投标报价为评标基准价，其价格分为
    /// 满分。其他投标人的价格分统一按照下列公式计算：
    /// 投标报价得分 =（评标基准价/投标报价）× 100」
    /// 「评标过程中，不得去掉报价中的最高报价和最低报价」
    ///
    /// 场景：政采仪器采购，价格分 30 分（货物 ≥30% 满足），5 家报价
    /// （含己方 115 万）：98.0 / 105.0 / 110.0 / 115.0 / 126.0。
    #[test]
    fn real_rule_87_lowest_price_ratio() {
        let spec = lowest_spec(30.0, DeviationRule::Ratio);
        let others = [98.0, 105.0, 110.0, 126.0];

        // 己方 115：手工 = 98/115 × 30 = 25.5652173913
        let d = spec.score_detail(115.0, &others).unwrap();
        assert_close(d.benchmark, 98.0, "基准价 = 最低报价（含己方全集合）");
        assert_close(d.score, 98.0 / 115.0 * 30.0, "87号令公式（基准/报价）×满分");
        assert!((d.score - 25.565217).abs() < 1e-6, "呈现: {:.2}", d.score);

        // 最低价者 98：98/98 × 30 = 30（满分）
        let d_min = spec.score_detail(98.0, &others).unwrap();
        assert_close(d_min.score, 30.0, "最低报价满分");

        // 最高价者 126：98/126 × 30 = 23.333333
        let d_max = spec.score_detail(126.0, &others).unwrap();
        assert_close(d_max.score, 98.0 / 126.0 * 30.0, "最高报价按公式折减");
        assert!((d_max.score - 23.333333).abs() < 1e-6, "呈现: {:.2}", d_max.score);

        // 87号令明文「不得去掉最高最低」：基准价必须是纯最低值——
        // 5 家报价场景下基准不受任何剔除影响（与复合法的对照面）
        assert_close(d.benchmark, 98.0, "不得去掉最高最低");

        // 最优报价 = min(others) = 98（并列最低即满分）
        let opt = spec.optimal_price(&others).unwrap();
        assert_close(opt, 98.0, "最优 = 并列最低");
        assert_close(spec.score(opt, &others).unwrap(), 30.0, "最优报价得满分");
    }

    /// 87号令 + 唯一投标人（others 空）：任意报价都是最低 → 满分；
    /// 最优反推有控制价取控制价（等分区间上限），无控制价无界拒绝。
    #[test]
    fn real_rule_87_sole_bidder_and_unbounded() {
        let mut spec = lowest_spec(30.0, DeviationRule::Ratio);
        // 唯一投标人：报 88 万也是最低 → 满分
        let d = spec.score_detail(88.0, &[]).unwrap();
        assert_close(d.benchmark, 88.0, "唯一报价即基准");
        assert_close(d.score, 30.0, "唯一投标人满分");

        // 无控制价 + 无竞争报价：最优无界 → 诚实拒绝
        assert!(spec.optimal_price(&[]).is_err(), "无界必须拒绝");
        // 有控制价：取控制价（任意报价满分中的最高价）
        spec.budget = Some(100.0);
        let opt = spec.optimal_price(&[]).unwrap();
        assert_close(opt, 100.0, "唯一投标人取控制价上限");
        assert_close(spec.score(100.0, &[]).unwrap(), 30.0, "控制价报价满分");
    }

    /// 真实规则 2：康居嘉苑项目监理方法一（ggzy.gov.cn，连云港 2021）——
    /// 「以有效投标文件的最低投标报价为评标基准价。投标报价等于评标基准价
    /// 的得满分；偏离评标基准价的，投标报价每高于评标基准价 1% 扣减 0.2 分；
    /// 投标报价每低于评标基准价 1% 扣减 0.1 分；偏离不足 1% 的，按照插入法
    /// 计算得分」（44 分）。
    ///
    /// 场景：监理报价（万）己方 95.0，others = 98.0 / 102.0 / 106.0 / 112.0。
    #[test]
    fn real_rule_kangju_method1_lowest_price_deviation() {
        let spec = lowest_spec(44.0, DeviationRule::Price { deduct_high: 0.2, deduct_low: 0.1 });
        let others = [98.0, 102.0, 106.0, 112.0];

        // 己方 95 = 最低价 → 基准 = 95 → 偏离 0 → 满分
        let d0 = spec.score_detail(95.0, &others).unwrap();
        assert_close(d0.benchmark, 95.0, "最低报价为基准");
        assert_close(d0.score, 44.0, "命中基准满分");

        // 若己方报 98（并列最低之一）：基准 = 95（others 里的最低），偏离 = 3/95
        // 手工：98/95 − 1 = 3.157895% × 0.2 = 0.631579 → 44 − 0.631579 = 43.368421
        let d1 = spec.score_detail(98.0, &[95.0, 102.0, 106.0, 112.0]).unwrap();
        assert_close(d1.benchmark, 95.0, "基准 = 全集合最低");
        assert_close(d1.score, 44.0 - (3.0 / 95.0) * 100.0 * 0.2, "每高1%扣0.2（插入法）");
        assert!((d1.score - 43.368421).abs() < 1e-6, "呈现: {:.2}", d1.score);

        // 报 102：偏离 = 7/95 = 7.368421% × 0.2 = 1.473684 → 42.526316
        let d2 = spec.score_detail(102.0, &[95.0, 98.0, 106.0, 112.0]).unwrap();
        assert_close(d2.score, 44.0 - (7.0 / 95.0) * 100.0 * 0.2, "高价侧线性折减");
        assert!((d2.score - 42.526316).abs() < 1e-6, "呈现: {:.2}", d2.score);

        // 最优报价 = min(others) = 98：并列最低（基准含己方，偏离为零），
        // 取等分区间最高价
        let opt = spec.optimal_price(&others).unwrap();
        assert_close(opt, 98.0, "最优 = 并列最低（等分区间最高价）");
        assert_close(spec.score(opt, &others).unwrap(), 44.0, "最优得满分");
    }

    /// 最低价法最优报价的上界性：网格暴力全扫无超者。
    #[test]
    fn lowest_optimal_is_global_by_grid_search() {
        let spec = lowest_spec(30.0, DeviationRule::Ratio);
        let others = [98.0, 105.0, 110.0, 126.0];
        let x = spec.optimal_price(&others).unwrap();
        let best = spec.score(x, &others).unwrap();
        let mut p = 1.0;
        while p <= 140.0 {
            if let Ok(s) = spec.score(p, &others) {
                assert!(s <= best + 1e-9, "网格点 {p} 得分 {s} 超过最优 {best}");
            }
            p += 0.5;
        }
    }

    /// PR2 验收报告（87号令全表 + 康居方法一全表——`--nocapture` 查看）。
    #[test]
    fn acceptance_report_lowest_price() {
        // —— 87号令第五十五条：5 家政采仪器（价格分 30）——
        let spec = lowest_spec(30.0, DeviationRule::Ratio);
        let others = [98.0, 105.0, 110.0, 126.0];
        for bid in [98.0, 105.0, 110.0, 115.0, 126.0] {
            let d = spec.score_detail(bid, &others).unwrap();
            let hand = (98.0 / bid) * 30.0;
            assert!((d.score - hand).abs() < 1e-9, "{bid}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| 87号令§55 | 基准 98.00（最低报价） | 报价 {bid:.2} | 比率 {:.6} | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                98.0 / bid, d.score
            );
        }
        let opt = spec.optimal_price(&others).unwrap();
        println!("| 87号令§55 | 最优报价反推 | — | — | 手工 98.00 | 引擎 {opt:.6} | 一致 |");

        // —— 康居嘉苑方法一：监理 44 分（每高1%扣0.2/低1%扣0.1，插入法）——
        let spec = lowest_spec(44.0, DeviationRule::Price { deduct_high: 0.2, deduct_low: 0.1 });
        let others = [95.0, 98.0, 102.0, 106.0, 112.0];
        for bid in [95.0, 98.0, 102.0, 106.0, 112.0] {
            let d = spec.score_detail(bid, &others).unwrap();
            let hand = 44.0 - ((bid - 95.0) / 95.0) * 100.0 * 0.2;
            assert!((d.score - hand).abs() < 1e-9, "{bid}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| 康居方法一 | 基准 95.00（最低报价） | 报价 {bid:.2} | 偏离 {:+.6}% | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                d.deviation * 100.0, d.score
            );
        }
    }

    // ================================================================ 平均价法（PR3）

    fn average_spec(coefficient: f64, max_score: f64, budget: Option<f64>,
                    deduct_high: f64, deduct_low: f64) -> PriceScoreSpec {
        PriceScoreSpec {
            method: PriceScoreMethod::AveragePrice { coefficient },
            max_score,
            budget,
            deviation_rule: DeviationRule::Price { deduct_high, deduct_low },
        }
    }

    /// 真实规则 3：康居嘉苑项目监理方法三（ggzy.gov.cn，连云港 2021）——
    /// 「以有效投标文件的投标报价算术平均值为 A，评标基准价 = A×K。
    /// K 值的抽取范围为 95%、96%、97%、98%」「投标报价等于评标基准价的得满分；
    /// 偏离评标基准价的，投标报价每高于评标基准价 1% 扣减 0.2 分；投标报价
    /// 每低于评标基准价 1% 扣减 0.1 分；偏离不足 1% 的，按照插入法计算得分」
    /// （44 分；5 家 < 7 家不剔除——纯平均形态）。
    ///
    /// 场景：监理报价（万）5 家（含己方 108）：95 / 100 / 105 / 108 / 115，
    /// K = 96%，最高限价 120 万。
    #[test]
    fn real_rule_kangju_method3_average_price() {
        let spec = average_spec(0.96, 44.0, Some(120.0), 0.2, 0.1);
        let others = [95.0, 100.0, 105.0, 115.0];

        // 手工：A = (95+100+105+108+115)/5 = 523/5 = 104.6
        //       基准 = 104.6×0.96 = 100.416
        let d = spec.score_detail(108.0, &others).unwrap();
        assert_close(d.benchmark, (95.0 + 100.0 + 105.0 + 108.0 + 115.0) / 5.0 * 0.96, "A×K");
        assert!((d.benchmark - 100.416).abs() < 1e-9, "基准 = 100.416");

        // 手工：己方 108 偏高 7.584 → 7.584/100.416 = 7.552581% × 0.2 = 1.510516
        //       → 44 − 1.510516 = 42.489484
        assert_close(d.deviation, 7.584 / 100.416, "偏离度");
        assert_close(d.score, 44.0 - (7.584 / 100.416) * 100.0 * 0.2, "高价侧扣分");
        assert!((d.score - 42.489484).abs() < 1e-5, "呈现: {:.2}", d.score);

        // 低价侧（不对称罚轻）：报价 95 偏低 5.416 → 5.393563% × 0.1
        //       → 44 − 0.539356 = 43.460644
        let d_low = spec.score_detail(95.0, &[100.0, 105.0, 108.0, 115.0]).unwrap();
        assert!((d_low.benchmark - 100.416).abs() < 1e-9, "同集合同基准");
        assert_close(d_low.score, 44.0 - (5.416 / 100.416) * 100.0 * 0.1, "低价侧扣分");
        assert!((d_low.score - 43.460644).abs() < 1e-5, "呈现: {:.2}", d_low.score);

        // 最优报价闭式解：x* = 0.96×415/(5−0.96) = 398.4/4.04 = 98.613861
        let opt = spec.optimal_price(&others).unwrap();
        assert_close(opt, 0.96 * 415.0 / 4.04, "闭式解 x* = cS/(n−c)");
        assert!((opt - 98.613861).abs() < 1e-6, "呈现: {opt:.2}");
        // 命中验证：最优报价处基准 = 自身 → 偏离 0 → 满分
        let d_opt = spec.score_detail(opt, &others).unwrap();
        assert_close(d_opt.deviation, 0.0, "最优报价偏离为零");
        assert_close(d_opt.score, 44.0, "最优报价得满分");
        assert!(opt <= 120.0, "最优 ≤ 控制价");
    }

    /// 真实规则 4：亳州市评标办法适用导则 2026 年版（ggzy.bozhou.gov.cn）
    /// 农业农村类综合评估法（合理低价）——
    /// 「将评标价平均值乘以评标基准价系数 C 作为评标基准价」（C=0.985 形态）
    /// 「偏差率 = 100%×(投标人评标价−评标基准价)/评标基准价」
    /// 「评标价 > 基准价：得分 = F − 偏差率×100×E1；评标价 ≤ 基准价：得分 =
    /// F + 偏差率×100×E2（E1≥2×E2）」「当投标报价得分为负时，均按 0 分计算」
    ///
    /// 场景：F=60，E1=1.5，E2=0.75，C=0.985，4 家（含己方 1000）：
    /// 980 / 995 / 1000 / 1010。
    #[test]
    fn real_rule_bozhou_agri_average_price() {
        let spec = average_spec(0.985, 60.0, None, 1.5, 0.75);
        let others = [980.0, 995.0, 1010.0];

        // 手工：平均 = 3985/4 = 996.25；基准 = 996.25×0.985 = 981.30625
        let d = spec.score_detail(1000.0, &others).unwrap();
        assert_close(d.benchmark, 996.25 * 0.985, "平均值×C");
        assert!((d.benchmark - 981.30625).abs() < 1e-9, "基准 = 981.30625");

        // 手工：己方 1000 偏高 18.69375 → 18.69375/981.30625 = 1.904986% × 1.5
        //       → 60 − 2.857480 = 57.142520
        assert_close(d.score, 60.0 - (18.69375 / 981.30625) * 100.0 * 1.5, "亳州 E1 高价侧");
        assert!((d.score - 57.142520).abs() < 1e-5, "呈现: {:.2}", d.score);

        // 低价侧：报价 980 偏低 1.30625 → 0.133118% × 0.75 → 60 − 0.099838
        let d_low = spec.score_detail(980.0, &[995.0, 1000.0, 1010.0]).unwrap();
        assert_close(d_low.score, 60.0 - (1.30625 / 981.30625) * 100.0 * 0.75, "亳州 E2 低价侧");

        // 最优报价：x* = 0.985×2985/(4−0.985) = 2940.225/3.015 = 975.199005
        let opt = spec.optimal_price(&others).unwrap();
        assert_close(opt, 2940.225 / 3.015, "闭式解");
        let d_opt = spec.score_detail(opt, &others).unwrap();
        assert_close(d_opt.deviation, 0.0, "最优报价偏离为零");
        assert_close(d_opt.score, 60.0, "最优报价得满分");
    }

    /// 平均价法 + 比率口径（真实组合形态：平均基准×比率得分）——
    /// x ≤ x* 时基准 ≥ 报价 → 比率 ≥ 1 截满分；x* 为等分区间最高价。
    #[test]
    fn average_price_with_ratio_rule() {
        let mut spec = average_spec(0.96, 44.0, Some(120.0), 0.2, 0.1);
        spec.deviation_rule = DeviationRule::Ratio;
        let others = [95.0, 100.0, 105.0, 115.0];
        let x_star = 0.96 * 415.0 / 4.04; // 98.613861

        // 低于 x*：基准 > 报价 → 比率 > 1 → 截满分
        let below = spec.score_detail(90.0, &others).unwrap();
        assert_close(below.score, 44.0, "低于不动点截满分");
        // 恰在 x*：比率 = 1 → 满分
        let hit = spec.score_detail(x_star, &others).unwrap();
        assert_close(hit.score, 44.0, "不动点满分");
        assert_close(hit.benchmark, x_star, "基准 = 报价（不动点）");
        // 高于 x*：比率 < 1 → 折减（基准/报价 × 44）
        let above = spec.score_detail(108.0, &others).unwrap();
        assert_close(above.score, 100.416 / 108.0 * 44.0, "高于不动点按比率折减");

        // 最优报价同闭式解（等分区间最高价）
        let opt = spec.optimal_price(&others).unwrap();
        assert!((opt - x_star).abs() < 1e-9, "Ratio 口径最优 = 不动点");
    }

    /// 平均价法校验与边界：系数越界拒绝；唯一投标人取控制价/无界拒绝；
    /// 闭式解超控制价时夹到控制价（受限最优）。
    #[test]
    fn average_price_validation_and_edges() {
        // 系数越界
        let mut spec = average_spec(1.2, 44.0, None, 0.2, 0.1);
        assert!(spec.score(100.0, &[95.0]).is_err(), "coefficient>1 拒绝");
        spec = average_spec(0.0, 44.0, None, 0.2, 0.1);
        assert!(spec.score(100.0, &[95.0]).is_err(), "coefficient=0 拒绝");

        // 唯一投标人：得分与报价无关 → 有控制价取控制价；无控制价无界拒绝
        let spec2 = average_spec(0.96, 44.0, Some(120.0), 0.2, 0.1);
        assert_close(spec2.optimal_price(&[]).unwrap(), 120.0, "唯一投标人取控制价");
        let d = spec2.score_detail(50.0, &[]).unwrap();
        let d2 = spec2.score_detail(110.0, &[]).unwrap();
        assert_close(d.score, d2.score, "唯一投标人得分与报价无关");
        let spec3 = average_spec(0.96, 44.0, None, 0.2, 0.1);
        assert!(spec3.optimal_price(&[]).is_err(), "无界拒绝");

        // 不变量（c ≤ 1 时恒成立）：x* = cS/(n−c) ≤ max(others) ≤ 控制价
        //   ——证明：x* ≤ c·m·M/(m+1−c)，与 M 比较即 c(m+1) ≤ m+1 ⟺ c ≤ 1。
        //   即闭式解永不超过最高竞争报价，无越限价风险；min() 仅作防御性保护。
        // 手工：x* = 0.96×294/(4−0.96) = 282.24/3.04 = 92.842 ≤ max(others)=99
        let spec4 = average_spec(0.96, 44.0, Some(100.0), 0.2, 0.1);
        let opt = spec4.optimal_price(&[99.0, 98.0, 97.0]).unwrap();
        assert_close(opt, 282.24 / 3.04, "闭式解");
        assert!(opt <= 99.0, "x* ≤ max(others) 不变量");
        assert!(opt <= 100.0, "x* ≤ 控制价");
        // 最优性验证：网格上界（0.25 步长全扫无超者）
        let best = spec4.score(opt, &[99.0, 98.0, 97.0]).unwrap();
        let mut p = 1.0;
        while p <= 100.0 {
            if let Ok(s) = spec4.score(p, &[99.0, 98.0, 97.0]) {
                assert!(s <= best + 1e-9, "网格点 {p} 得分 {s} 超过最优 {best}");
            }
            p += 0.25;
        }
    }

    /// PR3 验收报告（康居方法三 + 亳州农业农村全表——`--nocapture` 查看）。
    #[test]
    fn acceptance_report_average_price() {
        // —— 康居方法三：K=96%，5 家监理（44 分，高0.2/低0.1）——
        let spec = average_spec(0.96, 44.0, Some(120.0), 0.2, 0.1);
        let all = [95.0, 100.0, 105.0, 108.0, 115.0];
        let avg = all.iter().sum::<f64>() / 5.0; // 104.6
        let bench = avg * 0.96; // 100.416
        for bid in all {
            let others: Vec<f64> = all.iter().cloned().filter(|p| *p != bid).collect();
            let d = spec.score_detail(bid, &others).unwrap();
            let hand = 44.0 - ((bid - bench) / bench).abs() * 100.0
                * if bid > bench { 0.2 } else { 0.1 };
            assert!((d.score - hand).abs() < 1e-9, "{bid}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| 康居方法三 | 基准 {:.3}（=A{}×0.96） | 报价 {bid:.2} | 偏离 {:+.6}% | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                bench, avg, d.deviation * 100.0, d.score
            );
        }
        let opt = spec.optimal_price(&[95.0, 100.0, 105.0, 115.0]).unwrap();
        assert!((opt - 98.613861).abs() < 1e-5, "最优反推呈现: {opt:.2}");
        println!("| 康居方法三 | 最优报价反推 x*=cS/(n−c) | — | — | 手工 98.613861 | 引擎 {opt:.6} | 一致 |");

        // —— 亳州农业农村：C=0.985，F=60，E1=1.5/E2=0.75，4 家 ——
        let spec = average_spec(0.985, 60.0, None, 1.5, 0.75);
        let all = [980.0, 995.0, 1000.0, 1010.0];
        let avg = all.iter().sum::<f64>() / 4.0; // 996.25
        let bench = avg * 0.985; // 981.30625
        for bid in all {
            let others: Vec<f64> = all.iter().cloned().filter(|p| *p != bid).collect();
            let d = spec.score_detail(bid, &others).unwrap();
            let hand = 60.0 - ((bid - bench) / bench).abs() * 100.0
                * if bid > bench { 1.5 } else { 0.75 };
            assert!((d.score - hand).abs() < 1e-9, "{bid}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| 亳州农业农村 | 基准 {:.5}（=A{}×0.985） | 报价 {bid:.2} | 偏离 {:+.6}% | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                bench, avg, d.deviation * 100.0, d.score
            );
        }
        let opt = spec.optimal_price(&[980.0, 995.0, 1010.0]).unwrap();
        assert!((opt - 975.199005).abs() < 1e-5, "最优反推呈现: {opt:.2}");
        println!("| 亳州农业农村 | 最优报价反推 x*=cS/(n−c) | — | — | 手工 975.199005 | 引擎 {opt:.6} | 一致 |");
    }

    // ================================================================ 复合法（PR4）

    #[allow(clippy::too_many_arguments)]
    fn composite_spec(trim_high: usize, trim_low: usize, budget_weight: f64,
                      budget_coefficient: f64, avg_coefficient: f64,
                      max_score: f64, budget: Option<f64>,
                      deduct_high: f64, deduct_low: f64) -> PriceScoreSpec {
        PriceScoreSpec {
            method: PriceScoreMethod::Composite { trim_high, trim_low, budget_weight,
                                                  budget_coefficient, avg_coefficient },
            max_score,
            budget,
            deviation_rule: DeviationRule::Price { deduct_high, deduct_low },
        }
    }

    /// 真实规则 5：康居嘉苑项目监理方法四（ggzy.gov.cn，连云港 2021，本项目选定）——
    /// 「以有效投标文件的投标报价算术平均值为 A，最高投标限价为 B，则：
    /// 评标基准价 = A×Q1 + B×Q2。Q2=1−Q1，Q1 的取值范围为 10%,15%,20%,25%,30%」
    /// 「计算算术平均值 A 时，若 7≤有效投标文件<10 家时，应去掉其中的一个最高价
    /// 和一个最低价；若有效投标文件≥10 家，应去掉其中的两个最高价和两个最低价」
    /// 「投标报价等于评标基准价的得满分；每高于 1% 扣减 0.2 分；每低于 1% 扣减
    /// 0.1 分；偏离不足 1% 的，按照插入法计算得分」（44 分，B=120 万）。
    ///
    /// 场景：8 家（7≤n<10 → 去 1 高 1 低），Q1=20%（开标抽中），
    /// others = 92/96.5/99/101/103.5/106/118，己方 100。
    #[test]
    fn real_rule_kangju_method4_composite() {
        let spec = composite_spec(1, 1, 0.8, 1.0, 1.0, 44.0, Some(120.0), 0.2, 0.1);
        let others = [92.0, 96.5, 99.0, 101.0, 103.5, 106.0, 118.0];

        // 手工：全 8 家排序 92,96.5,99,100,101,103.5,106,118 → 去 1 高(118)+1 低(92)
        //       A = (96.5+99+100+101+103.5+106)/6 = 606/6 = 101
        //       基准 = 101×0.2 + 120×0.8 = 20.2 + 96 = 116.2
        let d = spec.score_detail(100.0, &others).unwrap();
        assert!((d.benchmark - 116.2).abs() < 1e-9, "基准 = 116.2，got {}", d.benchmark);

        // 手工：己方 100 偏低 16.2 → 16.2/116.2 = 13.941481% × 0.1
        //       → 44 − 1.394148 = 42.605852
        assert_close(d.deviation, -16.2 / 116.2, "偏离度");
        assert_close(d.score, 44.0 - (16.2 / 116.2) * 100.0 * 0.1, "低价侧扣分");
        assert!((d.score - 42.605852).abs() < 1e-5, "呈现: {:.2}", d.score);

        // 最优报价（带内不动点，手工）：
        //   保留竞争报价 = 96.5+99+101+103.5+106 = 506（S_k），保留家数 k1 = 6
        //   a = 1×(1−0.8) = 0.2，b = 1×0.8×120 = 96
        //   x* = (0.2×506 + 96×6)/(6−0.2) = (101.2+576)/5.8 = 677.2/5.8 = 116.758621
        let opt = spec.optimal_price(&others).unwrap();
        assert_close(opt, 677.2 / 5.8, "带内不动点闭式解");
        assert!((opt - 116.758621).abs() < 1e-5, "呈现: {opt:.2}");
        // 命中验证：最优报价处基准 = 自身 → 满分
        let d_opt = spec.score_detail(opt, &others).unwrap();
        assert_close(d_opt.deviation, 0.0, "最优报价偏离为零");
        assert_close(d_opt.score, 44.0, "最优报价得满分");
        assert!(opt <= 120.0, "最优 ≤ 控制价");
    }

    /// 真实规则 6：东南大学四牌楼校区热工实验室修缮改造一期
    /// （ccgp.gov.cn 更正公告 2026-08-21，现版报价分 70 分、基准命中得 69 分）——
    /// 「以有效投标文件的报价算术平均值为 A（若 ≥7 家，去掉一个最高价和一个
    /// 最低价后取算术平均值为 A）。最高限价为 B；评标基准价为 C；
    /// C = A*Q1*K1 + B*Q2*K2（K1∈{96%,97%,98%}，K2=95%，Q1∈{40%,50%,60%}，
    /// Q2=1−Q1）。等于评标基准价 C 的为最高分(69分)…投标价每高于基准价 1% 的
    /// 扣 0.9 分，低于基准价 1% 的扣 0.6 分，不足 1% 的按内插法计算
    /// （小数点后保留两位）」
    ///
    /// 场景：8 家（去 1 高 1 低），K1=97%、K2=95%、Q1=50%，B=1200 万，
    /// others = 980/1010/1035/1060/1090/1120/1160，己方 1050。
    #[test]
    fn real_rule_seu_full_composite() {
        let spec = composite_spec(1, 1, 0.5, 0.95, 0.97, 69.0, Some(1200.0), 0.9, 0.6);
        let others = [980.0, 1010.0, 1035.0, 1060.0, 1090.0, 1120.0, 1160.0];

        // 手工：A = (1010+1035+1050+1060+1090+1120)/6 = 6365/6，
        //       C = A×0.485 + 570 = 514.504167 + 570 = 1084.504167
        let bench = (6365.0 / 6.0) * 0.485 + 570.0;
        let d = spec.score_detail(1050.0, &others).unwrap();
        assert!((d.benchmark - bench).abs() < 1e-9, "C = 1084.504167，got {}", d.benchmark);

        // 手工：己方 1050 偏低 34.504167 → 3.181565% × 0.6 → 69 − 1.908939 = 67.091061
        assert_close(d.score, 69.0 - ((1050.0 - bench) / bench).abs() * 100.0 * 0.6, "低价侧 0.6 扣分");
        assert!((d.score - 67.091061).abs() < 1e-4, "呈现: {:.2}", d.score);

        // 最优报价（带内不动点，手工）：
        //   S_k = 1010+1035+1060+1090+1120 = 5315，k1 = 6
        //   a = 0.97×0.5 = 0.485，b = 0.95×0.5×1200 = 570
        //   x* = (0.485×5315 + 570×6)/(6−0.485) = (2577.775+3420)/5.515
        //      = 5997.775/5.515 = 1087.538531
        let opt = spec.optimal_price(&others).unwrap();
        assert_close(opt, 5997.775 / 5.515, "带内不动点闭式解");
        assert!((opt - 1087.538531).abs() < 1e-5, "呈现: {opt:.2}");
        let d_opt = spec.score_detail(opt, &others).unwrap();
        assert_close(d_opt.score, 69.0, "最优报价得最高分 69");
        assert_close(d_opt.deviation, 0.0, "最优报价偏离为零");
    }

    /// 真实规则 7：玉带河景观提升工程 EPC 方法一（ggzy.gov.cn，连云港 2021）——
    /// 「以有效投标文件…的评标价算术平均值为 A（当有效投标文件 4-6 家时，
    /// 剔除最高报价（最高报价相同的均剔除）后进行算术平均）。评标基准价 = A×K，
    /// K 值…随机抽取…取值范围为 95%、96%、97%、98%」「评标价等于评标基准价的
    /// 得满分，评标价相对评标基准价每低 1% 扣 0.5 分，每高 1% 扣 1.0 分；
    /// 偏离不足 1% 的，按照插入法计算得分」（报价评审 84 分）。
    ///
    /// 场景：5 家（4-6 家 → 只剔最高 1 家），K=97%，
    /// others = 2000/2150/2250/2400，己方 2100。
    #[test]
    fn real_rule_yudaihe_method1_composite() {
        // budget_weight=0（无控制价分量），trim_high=1/trim_low=0（4-6 家只剔最高）
        let spec = composite_spec(1, 0, 0.0, 1.0, 0.97, 84.0, None, 1.0, 0.5);
        let others = [2000.0, 2150.0, 2250.0, 2400.0];

        // 手工：排序 2000,2100,2150,2250,2400 → 剔最高(2400)
        //       A = (2000+2100+2150+2250)/4 = 8500/4 = 2125
        //       基准 = 2125×0.97 = 2061.25
        let d = spec.score_detail(2100.0, &others).unwrap();
        assert!((d.benchmark - 2061.25).abs() < 1e-9, "基准 = 2061.25，got {}", d.benchmark);

        // 手工：己方 2100 偏高 38.75 → 38.75/2061.25 = 1.879879% × 1.0
        //       → 84 − 1.879879 = 82.120121
        assert_close(d.score, 84.0 - (38.75 / 2061.25) * 100.0 * 1.0, "高价侧 1.0 扣分");
        assert!((d.score - 82.120073).abs() < 1e-4, "呈现: {:.2}", d.score);

        // 低价侧（不对称罚半）：报价 2000 偏低 61.25 → 61.25/2061.25 = 2.971498%
        //       × 0.5 → 84 − 1.485749 = 82.514251
        let d_low = spec.score_detail(2000.0, &[2100.0, 2150.0, 2250.0, 2400.0]).unwrap();
        assert!((d_low.benchmark - 2061.25).abs() < 1e-9, "同集合同基准");
        assert_close(d_low.score, 84.0 - (61.25 / 2061.25) * 100.0 * 0.5, "低价侧 0.5 扣分");
        assert!((d_low.score - 82.514251).abs() < 1e-4, "呈现: {:.2}", d_low.score);

        // 最优报价（带内不动点，手工）：
        //   S_k = 2000+2150+2250 = 6400（剔 2400），k1 = 4，a = 0.97，b = 0
        //   x* = 0.97×6400/(4−0.97) = 6208/3.03 = 2048.844884
        let opt = spec.optimal_price(&others).unwrap();
        assert_close(opt, 6208.0 / 3.03, "带内不动点闭式解");
        assert!((opt - 2048.844884).abs() < 1e-5, "呈现: {opt:.2}");
        let d_opt = spec.score_detail(opt, &others).unwrap();
        assert_close(d_opt.deviation, 0.0, "最优报价偏离为零");
        assert_close(d_opt.score, 84.0, "最优报价得满分");
    }

    /// 真实规则 8：玉带河景观提升工程 EPC 方法二（ggzy.gov.cn）——
    /// 「评标基准价 = A×K1×Q1 + B×K2×Q2（Q2=1−Q1，Q1∈{65%,70%,75%,80%,85%}，
    /// K1∈{95%,96%,97%,98%}；本工程（园林绿化）K2 值取为 90%）」；
    /// A 的剔除同方法一（≥7 家去掉最高和最低 20% 四舍五入取整——7 家去 1+1）；
    /// 扣分同方法一（每低 1% 扣 0.5/每高 1% 扣 1.0，插入法）。
    /// 招标控制价 B = 2480 万。
    ///
    /// 场景：7 家（20%×7=1.4 → 四舍五入去 1 高 1 低），K1=96%、Q1=70%、K2=90%，
    /// others = 2000/2050/2100/2180/2230/2450，己方 2150。
    #[test]
    fn real_rule_yudaihe_method2_composite() {
        let spec = composite_spec(1, 1, 0.3, 0.90, 0.96, 84.0, Some(2480.0), 1.0, 0.5);
        let others = [2000.0, 2050.0, 2100.0, 2180.0, 2230.0, 2450.0];

        // 手工：排序 2000,2050,2100,2150,2180,2230,2450 → 去 1 高(2450)+1 低(2000)
        //       A = (2050+2100+2150+2180+2230)/5 = 10710/5 = 2142
        //       基准 = 2142×0.96×0.7 + 2480×0.90×0.3
        //            = 1439.424 + 669.6 = 2109.024
        let d = spec.score_detail(2150.0, &others).unwrap();
        assert!((d.benchmark - 2109.024).abs() < 1e-9, "基准 = 2109.024，got {}", d.benchmark);

        // 手工：己方 2150 偏高 40.976 → 40.976/2109.024 = 1.942888% × 1.0
        //       → 84 − 1.942888 = 82.057112
        assert_close(d.score, 84.0 - (40.976 / 2109.024) * 100.0 * 1.0, "高价侧 1.0 扣分");
        assert!((d.score - 82.057112).abs() < 1e-4, "呈现: {:.2}", d.score);

        // 最优报价（带内不动点，手工）：
        //   S_k = 2050+2100+2180+2230 = 8560，k1 = 5
        //   a = 0.96×0.7 = 0.672，b = 0.90×0.3×2480 = 669.6
        //   x* = (0.672×8560 + 669.6×5)/(5−0.672) = (5752.32+3348)/4.328
        //      = 9100.32/4.328 = 2102.661744
        let opt = spec.optimal_price(&others).unwrap();
        assert_close(opt, 9100.32 / 4.328, "带内不动点闭式解");
        assert!((opt - 2102.661744).abs() < 1e-5, "呈现: {opt:.2}");
        let d_opt = spec.score_detail(opt, &others).unwrap();
        assert_close(d_opt.deviation, 0.0, "最优报价偏离为零");
        assert_close(d_opt.score, 84.0, "最优报价得满分");
    }

    /// PR1 承诺的退化等价：Composite{budget_weight=1} ≡ BenchmarkPrice；
    /// Composite{budget_weight=0, 不剔除} ≡ AveragePrice。
    #[test]
    fn composite_degenerates_to_benchmark_and_average() {
        let others = [950.0, 980.0, 1010.0];
        // budget_weight=1 → 纯控制价分量 ≡ 基准价法（系数 0.95）
        let comp_b = composite_spec(0, 0, 1.0, 0.95, 0.97, 69.0, Some(1200.0), 0.9, 0.6);
        let bench = PriceScoreSpec {
            method: PriceScoreMethod::BenchmarkPrice { coefficient: 0.95 },
            max_score: 69.0,
            budget: Some(1200.0),
            deviation_rule: DeviationRule::Price { deduct_high: 0.9, deduct_low: 0.6 },
        };
        for bid in [1000.0, 1100.0, 1140.0] {
            let a = comp_b.score_detail(bid, &others).unwrap();
            let b = bench.score_detail(bid, &others).unwrap();
            assert_close(a.score, b.score, "退化等价（基准价法）{bid}");
            assert_close(a.benchmark, b.benchmark, "基准等价");
        }
        assert_close(comp_b.optimal_price(&others).unwrap(),
                     bench.optimal_price(&others).unwrap(), "最优等价");

        // budget_weight=0 且不剔除 → 纯平均 ≡ 平均价法（系数 0.96）
        let comp_a = composite_spec(0, 0, 0.0, 0.95, 0.96, 44.0, Some(1200.0), 0.2, 0.1);
        let avg = PriceScoreSpec {
            method: PriceScoreMethod::AveragePrice { coefficient: 0.96 },
            max_score: 44.0,
            budget: Some(1200.0),
            deviation_rule: DeviationRule::Price { deduct_high: 0.2, deduct_low: 0.1 },
        };
        for bid in [950.0, 1000.0, 1080.0] {
            let a = comp_a.score_detail(bid, &others).unwrap();
            let b = avg.score_detail(bid, &others).unwrap();
            assert_close(a.score, b.score, "退化等价（平均价法）{bid}");
        }
        assert_close(comp_a.optimal_price(&others).unwrap(),
                     avg.optimal_price(&others).unwrap(), "最优等价");
    }

    /// 高位剔除区精确求解：控制价分量占主导时，带内不动点越出分段边界——
    /// 真实峰值在「己方被剔、基准为常量」区的不动点 x = 常量基准本身。
    ///
    /// 场景（合成，验证候选集完备性）：others = 100/110/120，剔 1+1，
    /// budget_weight=0.8、两侧系数 1.0、控制价 1000：
    ///   己方 > 120 时被剔高 → 保留 others 的 110,120 → 基准 = (110+120)/2×0.2+800 = 823
    ///   峰值 = 823（常量基准的不动点——偏离为零即满分）
    #[test]
    fn composite_high_trim_region_exact_optimum() {
        let spec = composite_spec(1, 1, 0.8, 1.0, 1.0, 60.0, Some(1000.0), 0.9, 0.6);
        let others = [100.0, 110.0, 120.0];

        let opt = spec.optimal_price(&others).unwrap();
        // 带内不动点 = (0.2×110 + 800×2)/(2−0.2) = 1620/1.8 = 900（越界候选，应被更优者击败）
        // 高位剔除区常量 = 0.2×(110+120)/2 + 800 = 23 + 800 = 823（真峰值）
        assert!((opt - 823.0).abs() < 1e-9, "高位剔除区不动点 823，got {opt}");
        let d = spec.score_detail(opt, &others).unwrap();
        assert_close(d.deviation, 0.0, "峰值偏离为零");
        assert_close(d.score, 60.0, "峰值满分");

        // 网格暴力上界：0.5 步长全扫 [1,1000] 无超者（候选集精确性的独立验证）
        let best = spec.score(opt, &others).unwrap();
        let mut p = 1.0;
        while p <= 1000.0 {
            if let Ok(s) = spec.score(p, &others) {
                assert!(s <= best + 1e-9, "网格点 {p} 得分 {s} 超过最优 {best}");
            }
            p += 0.5;
        }
    }

    /// 复合法校验与边界：权重/系数越界、无控制价但有权重、剔除数超家数。
    #[test]
    fn composite_validation_and_edges() {
        // budget_weight > 0 但无控制价
        let mut spec = composite_spec(1, 1, 0.5, 0.95, 0.97, 69.0, None, 0.9, 0.6);
        assert!(spec.score(1050.0, &[980.0, 1160.0]).is_err(), "有权重无控制价拒绝");
        // 权重越界
        spec = composite_spec(1, 1, 1.5, 0.95, 0.97, 69.0, Some(1200.0), 0.9, 0.6);
        assert!(spec.score(1050.0, &[980.0, 1160.0]).is_err(), "权重>1 拒绝");
        spec = composite_spec(1, 1, -0.1, 0.95, 0.97, 69.0, Some(1200.0), 0.9, 0.6);
        assert!(spec.score(1050.0, &[980.0, 1160.0]).is_err(), "权重<0 拒绝");
        // 系数越界
        spec = composite_spec(1, 1, 0.5, 1.2, 0.97, 69.0, Some(1200.0), 0.9, 0.6);
        assert!(spec.score(1050.0, &[980.0, 1160.0]).is_err(), "budget_coefficient>1 拒绝");
        spec = composite_spec(1, 1, 0.5, 0.95, 0.0, 69.0, Some(1200.0), 0.9, 0.6);
        assert!(spec.score(1050.0, &[980.0, 1160.0]).is_err(), "avg_coefficient=0 拒绝");
        // 剔除数超家数：3 家剔 2+2
        spec = composite_spec(2, 2, 0.0, 1.0, 0.97, 84.0, None, 1.0, 0.5);
        assert!(spec.score(2100.0, &[2000.0, 2400.0]).is_err(), "剔除后无剩余拒绝");
        assert!(spec.optimal_price(&[2000.0, 2400.0]).is_err(), "最优面同口径拒绝");
    }

    /// PR4 验收报告（四套真实规则——`--nocapture` 查看）。
    #[test]
    fn acceptance_report_composite() {
        // —— 康居方法四：8 家去 1+1，Q1=20%，B=120，44 分（高0.2/低0.1）——
        let spec = composite_spec(1, 1, 0.8, 1.0, 1.0, 44.0, Some(120.0), 0.2, 0.1);
        let all = [92.0, 96.5, 99.0, 100.0, 101.0, 103.5, 106.0, 118.0];
        for bid in all {
            let others: Vec<f64> = all.iter().cloned().filter(|p| *p != bid).collect();
            let d = spec.score_detail(bid, &others).unwrap();
            let hand = 44.0 - ((bid - 116.2) / 116.2).abs() * 100.0
                * if bid > 116.2 { 0.2 } else { 0.1 };
            assert!((d.score - hand).abs() < 1e-9, "{bid}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| 康居方法四 | 基准 116.200（=A101×0.2+B120×0.8） | 报价 {bid:.2} | 偏离 {:+.6}% | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                d.deviation * 100.0, d.score
            );
        }
        let opt = spec.optimal_price(&[92.0, 96.5, 99.0, 101.0, 103.5, 106.0, 118.0]).unwrap();
        assert!((opt - 116.758621).abs() < 1e-5, "最优反推呈现: {opt:.2}");
        println!("| 康居方法四 | 最优报价反推（带内不动点） | — | — | 手工 116.758621 | 引擎 {opt:.6} | 一致 |");

        // —— 东南大学：8 家去 1+1，K1=97%/K2=95%/Q1=50%，B=1200，69 分（高0.9/低0.6）——
        let spec = composite_spec(1, 1, 0.5, 0.95, 0.97, 69.0, Some(1200.0), 0.9, 0.6);
        let all = [980.0, 1010.0, 1035.0, 1050.0, 1060.0, 1090.0, 1120.0, 1160.0];
        for bid in all {
            let others: Vec<f64> = all.iter().cloned().filter(|p| *p != bid).collect();
            let d = spec.score_detail(bid, &others).unwrap();
            let bench = (6365.0 / 6.0) * 0.485 + 570.0;
            let hand = 69.0 - ((bid - bench) / bench).abs() * 100.0
                * if bid > bench { 0.9 } else { 0.6 };
            assert!((d.score - hand).abs() < 1e-9, "{bid}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| 东南大学 | 基准 1084.504167（=A×0.485+B×0.475） | 报价 {bid:.2} | 偏离 {:+.6}% | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                d.deviation * 100.0, d.score
            );
        }
        let opt = spec.optimal_price(&[980.0, 1010.0, 1035.0, 1060.0, 1090.0, 1120.0, 1160.0]).unwrap();
        assert!((opt - 1087.538531).abs() < 1e-5, "最优反推呈现: {opt:.2}");
        println!("| 东南大学 | 最优报价反推（带内不动点） | — | — | 手工 1087.538531 | 引擎 {opt:.6} | 一致 |");

        // —— 玉带河方法一：5 家只剔最高，K=97%，84 分（高1.0/低0.5）——
        let spec = composite_spec(1, 0, 0.0, 1.0, 0.97, 84.0, None, 1.0, 0.5);
        let all = [2000.0, 2100.0, 2150.0, 2250.0, 2400.0];
        for bid in all {
            let others: Vec<f64> = all.iter().cloned().filter(|p| *p != bid).collect();
            let d = spec.score_detail(bid, &others).unwrap();
            let hand = 84.0 - ((bid - 2061.25) / 2061.25).abs() * 100.0
                * if bid > 2061.25 { 1.0 } else { 0.5 };
            assert!((d.score - hand).abs() < 1e-9, "{bid}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| 玉带河方法一 | 基准 2061.250（=A2125×0.97） | 报价 {bid:.2} | 偏离 {:+.6}% | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                d.deviation * 100.0, d.score
            );
        }
        let opt = spec.optimal_price(&[2000.0, 2150.0, 2250.0, 2400.0]).unwrap();
        assert!((opt - 2048.844884).abs() < 1e-5, "最优反推呈现: {opt:.2}");
        println!("| 玉带河方法一 | 最优报价反推（带内不动点） | — | — | 手工 2048.844884 | 引擎 {opt:.6} | 一致 |");

        // —— 玉带河方法二：7 家去 1+1，K1=96%/Q1=70%/K2=90%，B=2480，84 分 ——
        let spec = composite_spec(1, 1, 0.3, 0.90, 0.96, 84.0, Some(2480.0), 1.0, 0.5);
        let all = [2000.0, 2050.0, 2100.0, 2150.0, 2180.0, 2230.0, 2450.0];
        for bid in all {
            let others: Vec<f64> = all.iter().cloned().filter(|p| *p != bid).collect();
            let d = spec.score_detail(bid, &others).unwrap();
            let hand = 84.0 - ((bid - 2109.024) / 2109.024).abs() * 100.0
                * if bid > 2109.024 { 1.0 } else { 0.5 };
            assert!((d.score - hand).abs() < 1e-9, "{bid}: 引擎 {} vs 手工 {hand}", d.score);
            println!(
                "| 玉带河方法二 | 基准 2109.024（=A2142×0.672+B×0.27） | 报价 {bid:.2} | 偏离 {:+.6}% | 手工 {hand:.6} | 引擎 {:.6} | 一致 |",
                d.deviation * 100.0, d.score
            );
        }
        let opt = spec.optimal_price(&[2000.0, 2050.0, 2100.0, 2180.0, 2230.0, 2450.0]).unwrap();
        assert!((opt - 2102.661744).abs() < 1e-5, "最优反推呈现: {opt:.2}");
        println!("| 玉带河方法二 | 最优报价反推（带内不动点） | — | — | 手工 2102.661744 | 引擎 {opt:.6} | 一致 |");
    }
}
