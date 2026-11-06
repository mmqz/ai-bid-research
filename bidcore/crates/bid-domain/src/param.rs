//! param.rs —— T-4 ParamResponse 技术参数响应表（issue #11 批次 v3 / V3-3）。
//!
//! 定位：技术参数**逐行比对账本**——招标要求（required_value）× 投标响应
//! （response_value）在参数粒度上的偏离判定面。**负偏离 = 废标级**：技术参数
//! 不达标是评标现场最高频的废标原因之一，必须自动检出（[`DomainStore::negative_deviations`]
//! 全量列出——漏检一行就是放一行废标风险过闸）。
//!
//! 偏离派生纪律（单一真源 [`derive_deviation`]——写入/查询/持久化共用）：
//! - **数值主判**：两侧各提取首个数值（千分位逗号「1,000」、中文单位「万」
//!   （×10000）、前后缀文字「≥100GB」「不低于 3 年」均容忍），**数值大 = 优**
//!   （本批默认口径：技术参数主流是容量/年限/人数/功率等大者优；方向敏感参数
//!   （响应时间/报价等低者优）留待 T-7 审核闭环加 direction 覆写面——本批不猜测方向）；
//! - **语义标记**（response 侧，数值比对不可用时）：含「满足/符合/响应/达到/
//!   无偏离」→ 无偏离；含「优于/超过/高于/超出」→ 正偏离；含「低于/劣于/不足」
//!   → 负偏离；
//! - **不可比**（Incomparable）：双方无数值可对、无标记可依 → 诚实态，留给
//!   人工裁决（[`DomainStore::incomparable_deviations`] 即裁决工作清单）——禁止猜测。
//!
//! 与 MatrixRow（matrix.rs）的关系：矩阵管**条款级**响应状态（有没有证据、有
//! 没有章节落位），本表管**参数级**数值偏离（值达没达标）——一个条款可挂多个
//! 参数行（幂等合并键 = requirement_id × param_name），偏离在参数粒度裁决；
//! 导出闸门（blocking）语义仍住在 matrix.rs，本表不越界。

use crate::{DomainStore, ProjectStatus, ToolError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ---------------------------------------------------------------- 类型面
/// 偏离判定（T-4 规格：无偏离|正偏离|负偏离 + 诚实态不可比）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Deviation {
    /// 无偏离（响应满足要求）
    None,
    /// 正偏离（优于要求）
    Positive,
    /// 负偏离（**废标级**——技术参数不达标）
    Negative,
    /// 不可比（双方无法数值/语义比对——诚实态，人工裁决，禁止猜测）
    Incomparable,
}

impl Deviation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Deviation::None => "NONE",
            Deviation::Positive => "POSITIVE",
            Deviation::Negative => "NEGATIVE",
            Deviation::Incomparable => "INCOMPARABLE",
        }
    }

    /// 废标级口径：负偏离 → true（技术参数不达标 = 评标废标高频原因）。
    /// 其余态一律 false——不可比是**待裁决**不是不达标，不得伪装成废标级
    /// （诚实状态纪律：不确定 ≠ 不合格）。
    pub fn is_bid_fatal(&self) -> bool {
        matches!(self, Deviation::Negative)
    }
}

/// 参数响应行（T-4 规格 struct 的 Rust 落地——字段对齐 issue #11 批次 v3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamResponse {
    pub id: String, // PR-xxxx（store 单序列签发）
    pub requirement_id: String,
    pub param_name: String,
    /// 招标要求原文（如「≥100GB」「不低于 3 年」）
    pub required_value: String,
    /// 投标响应原文（如「120GB」「5年」）
    pub response_value: String,
    /// 偏离判定（**派生态**——set_param_responses 用 derive_deviation 覆写，
    /// 调用方传入的不算数——单一真源不被劫持）
    pub deviation: Deviation,
    /// 佐证证据（EV id——参数行的证明材料引用，可空）
    pub evidence_id: Option<String>,
}

/// 写入面（**不含 deviation**——偏离由 store 派生覆写；matrix.rs 派生态纪律
/// 的同款应用：人工/工具面只提供事实（要求/响应原文），判定归领域单一真源）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamInput {
    pub requirement_id: String,
    pub param_name: String,
    pub required_value: String,
    pub response_value: String,
    pub evidence_id: Option<String>,
}

// ---------------------------------------------------------------- 偏离派生（纯函数，单一真源）
/// 偏离判定（确定性——同输入恒同输出；判定按序：数值主判 > 语义标记 > 不可比）。
///
/// 为什么是纯函数：写入（set_param_responses）、查询、快照往返共用同一判定，
/// 任何一处的偏离都可在别处重算复核——可审计性来自单一真源。
pub fn derive_deviation(required: &str, response: &str) -> Deviation {
    // 1. 数值主判：两侧都拿到数值才可比（单侧有数值 ≠ 可比——拿 5 去对
    //    「详见技术白皮书」是猜测，不是判定）
    if let (Some(req_n), Some(resp_n)) = (extract_number(required), extract_number(response)) {
        return if resp_n == req_n {
            Deviation::None
        } else if resp_n > req_n {
            Deviation::Positive
        } else {
            Deviation::Negative
        };
    }
    // 2. 语义标记（response 侧）：词表按序短路，确定性优先于「语义直觉」
    semantic_deviation(response).unwrap_or(Deviation::Incomparable)
}

/// response 侧语义标记词表（数值比对不可用时的次级判定面）。
/// 词表命中顺序即判定顺序（满足类 → 正偏离类 → 负偏离类）——同文本多标记
/// 时取首命中，保证确定性。
fn semantic_deviation(response: &str) -> Option<Deviation> {
    const SATISFY: [&str; 5] = ["满足", "符合", "响应", "达到", "无偏离"];
    const BETTER: [&str; 4] = ["优于", "超过", "高于", "超出"];
    const WORSE: [&str; 3] = ["低于", "劣于", "不足"];
    if SATISFY.iter().any(|m| response.contains(m)) {
        Some(Deviation::None)
    } else if BETTER.iter().any(|m| response.contains(m)) {
        Some(Deviation::Positive)
    } else if WORSE.iter().any(|m| response.contains(m)) {
        Some(Deviation::Negative)
    } else {
        None
    }
}

/// 首个数值提取（手写扫描器——bid-domain 零外部依赖纪律，不引 regex）。
///
/// 容忍面（技术参数表的现实写法）：
/// - 千分位逗号：「1,000,000」→ 1000000（收集后剥除）；
/// - 中文单位「万」：数值段**紧邻**的下一个字符是「万」→ ×10000（「100万」
///   → 1000000；带空格的「100 万」不算紧邻——宁可不放大，不猜）；
/// - 前后缀文字：「≥100GB」「不低于 3 年」「3.5万次」——扫描器从首个数字
///   起收集 [0-9 , .] 连续段，其余字符一律是终止符（含 %/％——「95%」取 95，
///   两侧同口径比对不受影响）；
/// - 解析失败（如「192.168.1.1」多点段）→ None（诚实态：提取不到就不判）。
fn extract_number(s: &str) -> Option<f64> {
    // 首个 ASCII 数字的位置（全角数字不在容忍面——招标参数表罕见，不扩）
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let bytes = s.as_bytes();
    let mut end = start;
    while end < bytes.len()
        && (bytes[end].is_ascii_digit() || bytes[end] == b',' || bytes[end] == b'.')
    {
        end += 1;
    }
    let run = &s[start..end];
    let mut value: f64 = run.replace(',', "").parse().ok()?;
    // 「万」紧邻判定：收集段终止后的下一个字符（end 是新字符的起点——
    // 收集段全 ASCII，边界必然合法）
    if s[end..].starts_with('万') {
        value *= 10_000.0;
    }
    Some(value)
}

// ---------------------------------------------------------------- DomainStore 扩面（T-4 参数表操作）
impl DomainStore {
    /// 参数响应表只读面（空 = 未录入——set_param_responses 幂等填充）。
    pub fn param_responses(&self) -> &[ParamResponse] {
        &self.params
    }

    /// 批量写入参数响应行（幂等合并 + 偏离派生覆写）。
    ///
    /// - **偏离派生覆写**：入参 [`ParamInput`] 不含 deviation——每行偏离由
    ///   [`derive_deviation`] 现场派生，调用方传入的不算数（单一真源）；
    /// - 状态门：REQUIREMENTS_EXTRACTED / MATRIX_READY / EVIDENCE_BOUND /
    ///   DRAFTING（参数响应是起草素材——条款抽取后任意时刻可补录；
    ///   VALIDATING+ 走 T-7 审核闭环的影响面机制，本批不开门）；
    /// - 幽灵校验：requirement_id 必须在条款集（挂空条款的参数行是幽灵账目）；
    ///   evidence_id 若 Some 必须在证据台账（照 evidence 绑定校验模式）；
    /// - 幂等合并：键 = (requirement_id, param_name)；已存在 → **原位覆盖
    ///   （保留原 id——id 稳定）**，新行 → next_id("PR") 签发追加；
    /// - content_version += 1（整批写入是显式写操作——set_outline 同款口径）。
    ///
    /// 返回 (总行数, 新增行数)。
    pub fn set_param_responses(
        &mut self,
        rows: Vec<ParamInput>,
    ) -> Result<(usize, usize), ToolError> {
        let p = self.project()?;
        let allowed = matches!(
            p.status,
            ProjectStatus::RequirementsExtracted
                | ProjectStatus::MatrixReady
                | ProjectStatus::EvidenceBound
                | ProjectStatus::Drafting
        );
        if !allowed {
            return Err(ToolError::EState {
                current: p.status,
                attempted: "set_param_responses",
            });
        }
        if rows.is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "空参数表（无行可比对——先组织参数行再写入）".into(),
            });
        }
        // 幽灵 REQ（引用完整性——照 set_outline 的幽灵 REQ 模式，先全量校验后写入，
        // 拒绝路径零副作用）
        let known: HashSet<&str> = self.requirements.iter().map(|r| r.id.as_str()).collect();
        for row in &rows {
            if !known.contains(&row.requirement_id[..]) {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "参数行引用了不存在的条款 {}（幽灵参数行）",
                        row.requirement_id
                    ),
                });
            }
        }
        // 证据存在性（照 evidence 绑定校验模式：引用必须落在台账内）
        for row in &rows {
            if let Some(ev_id) = &row.evidence_id {
                if !self.evidences.iter().any(|e| &e.id == ev_id) {
                    return Err(ToolError::EUnprocessable {
                        reason: format!(
                            "参数行（{} / {}）引用的证据 {ev_id} 不在台账（先登记证据）",
                            row.requirement_id, row.param_name
                        ),
                    });
                }
            }
        }
        // 幂等合并（position 取索引而非可变引用——None 分支要签新 id，
        // 避免 next_id 的 &mut self 与迭代借用冲突）
        let mut added = 0usize;
        for row in rows {
            let deviation = derive_deviation(&row.required_value, &row.response_value);
            match self.params.iter().position(|pr| {
                pr.requirement_id == row.requirement_id && pr.param_name == row.param_name
            }) {
                Some(i) => {
                    // 原位覆盖：id 保留（稳定——外部引用不因重写漂移）
                    self.params[i].required_value = row.required_value;
                    self.params[i].response_value = row.response_value;
                    self.params[i].deviation = deviation;
                    self.params[i].evidence_id = row.evidence_id;
                }
                None => {
                    let id = self.next_id("PR");
                    self.params.push(ParamResponse {
                        id,
                        requirement_id: row.requirement_id,
                        param_name: row.param_name,
                        required_value: row.required_value,
                        response_value: row.response_value,
                        deviation,
                        evidence_id: row.evidence_id,
                    });
                    added += 1;
                }
            }
        }
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok((self.params.len(), added))
    }

    /// 负偏离全量清单（**废标级查询面**——T-4 验收：植入的负偏离必须一条不漏；
    /// 导出前的死磕清单从这里来）。
    pub fn negative_deviations(&self) -> Vec<&ParamResponse> {
        self.params
            .iter()
            .filter(|pr| pr.deviation == Deviation::Negative)
            .collect()
    }

    /// 不可比清单（人工裁决工作面——诚实态显式列出，不混入无偏离假装太平）。
    pub fn incomparable_deviations(&self) -> Vec<&ParamResponse> {
        self.params
            .iter()
            .filter(|pr| pr.deviation == Deviation::Incomparable)
            .collect()
    }
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{Evidence, EvidenceKind, EvidenceStatus};
    use crate::locator::EngineLocator;
    use crate::ParseStatus;
    use std::collections::HashMap;

    /// 参数表基座：项目 + 文件 + N 条技术参数类 REQ（REQ-0003 起——extract 产物品格）。
    fn store_with_params_base(n_reqs: usize) -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("参数项目", "C-PRM", "明标", "prm-key")
            .unwrap();
        s.upsert_file(
            "A.docx",
            "招标文件",
            "sha-p",
            5,
            1,
            50,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
        let reqs: Vec<crate::Requirement> = (0..n_reqs)
            .map(|i| crate::Requirement {
                id: format!("REQ-{:04}", i + 3),
                category: "技术参数".into(),
                clause_no: Some(format!("5.{}", i + 1)),
                anchors: vec![crate::Anchor {
                    file: "A".into(),
                    kind: "table".into(),
                    locator: format!("技术参数表 > 第 {} 行", i + 1),
                    excerpt: format!("参数-{i} 的原文摘录"),
                        block_idx: None,
                        page: None,
                        line: None
                }],
                params: HashMap::new(),
                source_file: "A".into(),
            })
            .collect();
        s.set_requirements(reqs).unwrap();
        s
    }

    fn input(
        req: &str,
        name: &str,
        required: &str,
        response: &str,
        evidence: Option<&str>,
    ) -> ParamInput {
        ParamInput {
            requirement_id: req.into(),
            param_name: name.into(),
            required_value: required.into(),
            response_value: response.into(),
            evidence_id: evidence.map(str::to_string),
        }
    }

    /// 台账里塞一条活证据（产品资料类——参数佐证的典型形态）。
    fn push_evidence(s: &mut DomainStore, id: &str) {
        s.evidences.push(Evidence {
            id: id.into(),
            kind: EvidenceKind::ProductDoc,
            title: "产品检测报告".into(),
            source_file: "B.pdf".into(),
            source_sha256: format!("sha-{id}").into(),
            locator: EngineLocator::PdfPageText {
                page: 1,
                text_fingerprint: format!("fnv64:{id}"),
            },
            fingerprint: format!("fnv64:{id}").into(),
            valid_from: None,
            valid_until: None,
            subject_name: "S".into(),
            scope: "".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: Default::default(), // #147 结构化字段（本测试不消费）
            excerpt: None, // #217 正文摘录（本测试不消费）
            excerpt_locator: None,
            verification_tier: None,
            version: 1,
            status: EvidenceStatus::Active,
        });
    }

    // ============================ 自研测试 ============================

    /// **T-4 规格硬验收**：100 行技术参数表逐行比出偏离，植入 3 个负偏离全检出。
    #[test]
    fn acceptance_100_rows_3_negative_all_detected() {
        // 100 行参数表：param_name 四类轮转，required 含数值与单位；
        // 响应值与要求相等或略优（数值 ≥ 要求），另植入恰好 3 行负偏离
        //（i=8/41/86——response < required）。
        const NAMES: [&str; 4] = ["存储容量", "并发用户数", "质保年限", "响应时间"];
        let planted: [(usize, &str); 3] = [(8, "存储容量"), (41, "并发用户数"), (86, "质保年限")];
        let rows: Vec<ParamInput> = (0..100usize)
            .map(|i| {
                let name = NAMES[i % 4];
                let (required, response): (String, String) = match i {
                    // —— 植入的 3 行负偏离（response < required）——
                    8 => ("≥100GB".into(), "60GB".into()),
                    41 => ("不低于 1000 人".into(), "800 人".into()),
                    86 => ("不低于 3 年".into(), "2年".into()),
                    // —— 常规行：相等或略优 ——
                    _ if i % 4 == 0 => (
                        "≥100GB".into(),
                        if (i / 4) % 2 == 0 {
                            "100GB".into()
                        } else {
                            "120GB".into()
                        },
                    ),
                    _ if i % 4 == 1 => (
                        "不低于 1000 人".into(),
                        if (i / 4) % 3 == 0 {
                            "1,000 人".into()
                        } else {
                            "1,200 人".into()
                        },
                    ),
                    _ if i % 4 == 2 => (
                        "不低于 3 年".into(),
                        if (i / 4) % 2 == 0 {
                            "3年".into()
                        } else {
                            "5年".into()
                        },
                    ),
                    // 响应时间（方向敏感参数）：本批默认口径下只造相等——
                    // 不拿「响应快=数值小」制造语义反例（direction 覆写面留待 T-7）
                    _ => ("7×24 小时响应".into(), "7×24 小时响应".into()),
                };
                input(
                    &format!("REQ-{:04}", i + 3),
                    name,
                    &required,
                    &response,
                    None,
                )
            })
            .collect();

        let mut s = store_with_params_base(100);
        let (total, added) = s.set_param_responses(rows).unwrap();
        assert_eq!((total, added), (100, 100), "100 行全量落表");

        // 负偏离全检出：恰好 3 条，且正是植入的那 3 行
        let negatives = s.negative_deviations();
        assert_eq!(negatives.len(), 3, "植入 3 个负偏离必须全部检出（废标级）");
        let got: Vec<(String, String)> = negatives
            .iter()
            .map(|pr| (pr.requirement_id.clone(), pr.param_name.clone()))
            .collect();
        let expect: Vec<(String, String)> = planted
            .iter()
            .map(|&(i, name)| (format!("REQ-{:04}", i + 3), name.to_string()))
            .collect();
        assert_eq!(got, expect, "检出的正是植入的 3 行（REQ × param_name）");
        assert!(
            negatives.iter().all(|pr| pr.deviation.is_bid_fatal()),
            "负偏离全部 is_bid_fatal=true（废标级口径）"
        );

        // 其余 97 行：无偏离或正偏离（不得有漏网的负偏离/不可比）
        let others: Vec<&ParamResponse> = s
            .param_responses()
            .iter()
            .filter(|pr| !negatives.iter().any(|n| n.id == pr.id))
            .collect();
        assert_eq!(others.len(), 97);
        assert!(
            others
                .iter()
                .all(|pr| matches!(pr.deviation, Deviation::None | Deviation::Positive)),
            "非植入行 ∈ 无偏离/正偏离"
        );
        // 夹具自检：97 行里两类都有（相等面与略优面都被覆盖）
        assert!(others.iter().any(|pr| pr.deviation == Deviation::None));
        assert!(others.iter().any(|pr| pr.deviation == Deviation::Positive));
    }

    /// 数值主判变体：千分位/单位前后缀/单侧无数值走语义。
    #[test]
    fn numeric_comparison_variants() {
        // 千分位逗号容忍：1,000 == 1000 → 无偏离
        assert_eq!(
            derive_deviation("1,000", "1000"),
            Deviation::None,
            "千分位等值"
        );
        assert_eq!(
            derive_deviation("1,000", "1,200"),
            Deviation::Positive,
            "千分位比较"
        );
        // 单位前后缀容忍：3年 < 5年 → 正偏离；100GB > 60GB → 负偏离
        assert_eq!(derive_deviation("3年", "5年"), Deviation::Positive);
        assert_eq!(derive_deviation("100GB", "60GB"), Deviation::Negative);
        assert_eq!(
            derive_deviation("≥100GB", "120GB"),
            Deviation::Positive,
            "≥ 前缀容忍"
        );
        // 单侧无数值：response 走语义（「满足」→ 无偏离）
        assert_eq!(
            derive_deviation("不低于 3 年", "满足"),
            Deviation::None,
            "response 无数值走语义标记"
        );
    }

    /// 语义标记三态 + 不可比诚实态 + as_str/is_bid_fatal 口径。
    #[test]
    fn semantic_markers_and_incomparable() {
        assert_eq!(
            derive_deviation("支持国产化适配", "满足招标要求"),
            Deviation::None
        );
        assert_eq!(
            derive_deviation("支持国产化适配", "优于招标要求"),
            Deviation::Positive
        );
        assert_eq!(
            derive_deviation("支持国产化适配", "低于要求"),
            Deviation::Negative
        );
        // 两侧都无数值且无标记词 → 不可比（不猜测）
        assert_eq!(
            derive_deviation("详见技术白皮书", "≥5A 安全等级"),
            Deviation::Incomparable,
            "无数值无标记 → 诚实态不可比"
        );
        // 展示名与废标级口径
        assert_eq!(Deviation::None.as_str(), "NONE");
        assert_eq!(Deviation::Positive.as_str(), "POSITIVE");
        assert_eq!(Deviation::Negative.as_str(), "NEGATIVE");
        assert_eq!(Deviation::Incomparable.as_str(), "INCOMPARABLE");
        assert!(Deviation::Negative.is_bid_fatal(), "负偏离 = 废标级");
        assert!(!Deviation::None.is_bid_fatal());
        assert!(!Deviation::Positive.is_bid_fatal());
        assert!(
            !Deviation::Incomparable.is_bid_fatal(),
            "不可比 ≠ 不合格（待裁决）"
        );
    }

    /// 「万」单位解析（×10000）与千分位互通。
    #[test]
    fn wan_unit_parsing() {
        assert_eq!(
            derive_deviation("100万", "1,000,000"),
            Deviation::None,
            "100万 == 1,000,000（万 ×10000 与千分位等值——两侧对称，排列不敏感）"
        );
        // 规格「"50万" vs "100万" → Negative」的落位：要求 100万、响应 50万
        // （响应不足要求）——负偏离；数值大=优默认口径下这是唯一成立排列
        assert_eq!(
            derive_deviation("100万", "50万"),
            Deviation::Negative,
            "响应 50万 不足要求 100万 → 负偏离（废标级）"
        );
        assert_eq!(
            derive_deviation("50万", "100万"),
            Deviation::Positive,
            "要求 50万、响应 100万 → 正偏离（数值大=优默认口径）"
        );
        assert_eq!(
            derive_deviation("注册资金 100万元", "注册资金 150万"),
            Deviation::Positive,
            "前后缀文字下「万」仍然生效"
        );
    }

    /// 幂等合并 + id 稳定：同批重放零新增零漂移；改值原位覆盖。
    #[test]
    fn idempotent_merge_stable_ids() {
        let mut s = store_with_params_base(3); // REQ-0003..REQ-0005
        let rows = vec![
            input("REQ-0003", "存储容量", "≥100GB", "120GB", None),
            input("REQ-0004", "质保年限", "不低于 3 年", "3年", None),
            input("REQ-0005", "并发用户数", "不低于 1000 人", "2,000 人", None),
        ];
        let (n1, a1) = s.set_param_responses(rows.clone()).unwrap();
        assert_eq!((n1, a1), (3, 3));
        let ids1: Vec<String> = s.param_responses().iter().map(|pr| pr.id.clone()).collect();
        let v1 = s.content_version();

        // 同批重放：行数不变、新增 0、id 集合不变
        let (n2, a2) = s.set_param_responses(rows.clone()).unwrap();
        assert_eq!((n2, a2), (3, 0), "幂等合并零新增");
        let ids2: Vec<String> = s.param_responses().iter().map(|pr| pr.id.clone()).collect();
        assert_eq!(ids1, ids2, "重放后 id 全等（id 稳定）");

        // 改一行 response 值再 set：原位覆盖（id 不变）+ 总数不变 + 版本递增
        let mut rows3 = rows;
        rows3[0].response_value = "60GB".into(); // 120GB → 60GB：正偏离翻负偏离
        let target_id = s.param_responses()[0].id.clone();
        let v2 = s.content_version();
        let (n3, a3) = s.set_param_responses(rows3).unwrap();
        assert_eq!((n3, a3), (3, 0), "覆盖不新增");
        assert_eq!(s.param_responses()[0].id, target_id, "原位覆盖保留原 id");
        assert_eq!(
            s.param_responses()[0].deviation,
            Deviation::Negative,
            "覆盖后偏离重派生（120GB→60GB 翻负）"
        );
        assert_eq!(s.negative_deviations().len(), 1, "翻负立即进入废标级查询面");
        assert_eq!(s.content_version(), v2 + 1, "改值写入 bump content_version");
        assert!(s.content_version() > v1, "版本单调递增");
    }

    /// 幽灵 REQ / 幽灵证据拒绝：零副作用；真证据放行。
    #[test]
    fn ghost_requirement_and_evidence_rejected() {
        let mut s = store_with_params_base(2); // REQ-0003..REQ-0004
                                               // 幽灵 REQ → 拒（零残留）
        let err = s
            .set_param_responses(vec![input("REQ-9999", "存储容量", "≥100GB", "120GB", None)])
            .unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "幽灵条款拒");
        assert_eq!(s.param_responses().len(), 0, "拒绝路径零残留");
        // 幽灵证据 → 拒
        let err = s
            .set_param_responses(vec![input(
                "REQ-0003",
                "存储容量",
                "≥100GB",
                "120GB",
                Some("EV-9999"),
            )])
            .unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "幽灵证据拒");
        assert_eq!(s.param_responses().len(), 0, "拒绝路径零残留");
        // 真证据在台账 → 放行且随行
        push_evidence(&mut s, "EV-0001");
        let (n, added) = s
            .set_param_responses(vec![input(
                "REQ-0003",
                "存储容量",
                "≥100GB",
                "120GB",
                Some("EV-0001"),
            )])
            .unwrap();
        assert_eq!((n, added), (1, 1));
        assert_eq!(
            s.param_responses()[0].evidence_id.as_deref(),
            Some("EV-0001")
        );
    }

    /// 状态门：FILES_IMPORTED（未抽条款）拒；REQUIREMENTS_EXTRACTED 放行。
    #[test]
    fn state_gate_rejects_early() {
        let mut s = DomainStore::new();
        s.create_project("p", "c", "明标", "k-prm").unwrap();
        s.upsert_file("f", "招标文件", "sha-x", 1, 0, 5, 0, ParseStatus::Ok, "")
            .unwrap();
        let err = s
            .set_param_responses(vec![input("REQ-0003", "存储容量", "≥100GB", "120GB", None)])
            .unwrap_err();
        assert_eq!(err.code(), "E_STATE", "FILES_IMPORTED 无条款不可写参数表");
        assert_eq!(s.param_responses().len(), 0, "状态门拒绝零副作用");
        // 推到 REQUIREMENTS_EXTRACTED 后放行
        s.set_requirements(vec![crate::Requirement {
            id: "REQ-0003".into(),
            category: "技术参数".into(),
            clause_no: None,
            anchors: vec![crate::Anchor {
                file: "f".into(),
                kind: "para".into(),
                locator: "第三章".into(),
                excerpt: "技术参数响应表合成件".into(),
                    block_idx: None,
                    page: None,
                    line: None
            }],
            params: HashMap::new(),
            source_file: "f".into(),
        }])
        .unwrap();
        assert!(
            s.set_param_responses(vec![input("REQ-0003", "存储容量", "≥100GB", "120GB", None)])
                .is_ok(),
            "REQUIREMENTS_EXTRACTED 可写参数表"
        );
    }

    /// 确定性：同输入两次调用结果一致（对照 outline derive_is_deterministic 模式）。
    #[test]
    fn derive_deviation_is_deterministic() {
        let cases: &[(&str, &str, Deviation)] = &[
            ("≥100GB", "120GB", Deviation::Positive),
            ("100GB", "60GB", Deviation::Negative),
            ("1,000", "1000", Deviation::None),
            ("100万", "1,000,000", Deviation::None),
            ("不低于 3 年", "满足", Deviation::None),
            ("详见技术白皮书", "≥5A 安全等级", Deviation::Incomparable),
            ("质保期 3 年", "质保期 5 年", Deviation::Positive),
        ];
        for (req, resp, expect) in cases {
            let d1 = derive_deviation(req, resp);
            let d2 = derive_deviation(req, resp);
            assert_eq!(d1, d2, "同输入两次调用必须一致（可复现是审计前提）");
            assert_eq!(d1, *expect, "({req} vs {resp}) 期望 {expect:?}");
        }
    }

    // ============================ 借鉴测试 ============================

    /// 快照往返保真：params 全等 + 负偏离查询面等价 + 重开后 id 序列不回卷。
    ///
    /// borrow：mdcore workbench `persistence_recovery.rs::session_reopen_state_and_continuity`
    /// 的「重开保真」模式——会话结束（drop）→ 从持久化面重开 → 全量状态在位、
    /// 序号单调续增（此处映射为：偏离判定/PR id/废标级清单跨重启无损，
    /// 新签 id 不与恢复的 id 碰撞）。
    #[test]
    fn snapshot_roundtrip_params_survive_restart() {
        let mut s = store_with_params_base(4); // REQ-0003..REQ-0006
        push_evidence(&mut s, "EV-0001");
        let rows = vec![
            input("REQ-0003", "存储容量", "1,000 点", "1000 点", None), // 无偏离
            input("REQ-0004", "并发用户数", "≥100 人", "200 人", None), // 正偏离
            input("REQ-0005", "质保年限", "不低于 3 年", "2年", None),  // 负偏离（废标级）
            input(
                "REQ-0006",
                "安全等级",
                "详见技术白皮书",
                "≥5A 安全等级",
                Some("EV-0001"),
            ), // 不可比 + 带证据
        ];
        s.set_param_responses(rows).unwrap();
        let before = s.param_responses().to_vec();
        assert_eq!(s.negative_deviations().len(), 1);
        assert_eq!(s.incomparable_deviations().len(), 1);

        // —— 会话结束 → 快照 → 重开 ——
        let snap = s.to_snapshot();
        let mut s2 = DomainStore::restore(snap).unwrap();

        // 参数表全量在位（逐行全等——偏离判定跨重启不漂移）
        assert_eq!(s2.param_responses(), before.as_slice(), "params 往返全等");
        // 废标级查询面等价（重开后的负偏离清单不缩水）
        let neg2: Vec<(&str, &str)> = s2
            .negative_deviations()
            .iter()
            .map(|pr| (pr.requirement_id.as_str(), pr.param_name.as_str()))
            .collect();
        assert_eq!(neg2, vec![("REQ-0005", "质保年限")], "负偏离清单往返等价");
        let inc2: Vec<(&str, &str)> = s2
            .incomparable_deviations()
            .iter()
            .map(|pr| (pr.requirement_id.as_str(), pr.param_name.as_str()))
            .collect();
        assert_eq!(inc2, vec![("REQ-0006", "安全等级")], "不可比清单往返等价");
        // 证据随行
        assert_eq!(
            s2.param_responses()
                .iter()
                .find(|pr| pr.param_name == "安全等级")
                .and_then(|pr| pr.evidence_id.clone())
                .as_deref(),
            Some("EV-0001")
        );

        // 重开后继续写入：新签 PR id 单调不碰撞（水位随快照走——mdcore 审计序号同款）
        let old_ids: Vec<String> = s2
            .param_responses()
            .iter()
            .map(|pr| pr.id.clone())
            .collect();
        let (n, added) = s2
            .set_param_responses(vec![input("REQ-0006", "投标保证金", "10万", "10万", None)])
            .unwrap();
        assert_eq!((n, added), (5, 1), "重开后可继续补录");
        let new_id = s2
            .param_responses()
            .iter()
            .find(|pr| pr.param_name == "投标保证金")
            .unwrap()
            .id
            .clone();
        assert!(!old_ids.contains(&new_id), "新签 id 不与恢复的 id 碰撞");
    }

    /// 快照 fail-closed：params 内两条同 id → restore 拒（损坏但语法合法也拒）。
    ///
    /// borrow：mdcor（workbench）`persistence_recovery.rs` 的 fail-closed 模式
    /// ——损坏文件响亮拒绝、不静默加载半个库；此处映射为：重复 PR id 的快照
    /// 即使 JSON 语法合法也必须被语义校验拦下。
    #[test]
    fn restore_rejects_duplicate_param_ids() {
        let mut s = store_with_params_base(2);
        s.set_param_responses(vec![
            input("REQ-0003", "存储容量", "≥100GB", "120GB", None),
            input("REQ-0004", "质保年限", "不低于 3 年", "3年", None),
        ])
        .unwrap();
        let mut snap = s.to_snapshot();
        // 人工破坏：复制首行造重复 id（JSON 完全合法——语义校验面拦截）
        let dup = snap.params[0].clone();
        snap.params.push(dup);
        // DomainStore 无 Debug（库态不可打印——序列化面在快照），unwrap_err 不可用，
        // 手工分出错误分支（零 unwrap 纪律同款）
        let err = match DomainStore::restore(snap) {
            Ok(_) => panic!("重复 PR id 的快照必须被拒（fail-closed）"),
            Err(e) => e,
        };
        assert_eq!(err.code(), "E_UNPROCESSABLE");
        assert!(
            format!("{err:?}").contains("params id 重复"),
            "错误面可辨识: {err:?}"
        );
    }
}
