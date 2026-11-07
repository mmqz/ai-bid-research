//! performance.rs —— #315 业绩材料库（跨项目复用）的领域纯函数面。
//!
//! 定位：业绩库的**实体 + 校验 + 幂等键 + 匹配打分 + 正文渲染**全部住本模块
//! （borrow：evidence.rs 的分层纪律——域层纯逻辑零 IO，磁盘读写住 bid-tools
//! 的 `performance.rs`）。跨项目持久化由服务层把 [`PerformanceLibrarySnapshot`]
//! 落到**项目工作区之外**的库文件（BID_PERF_LIB / 家目录默认位），本层只定义
//! 快照形态与语义。
//!
//! 匹配算法（#315 任务②——确定性、可解释、无随机）：
//! 四因子加权，缺信号**重归一**（不静默按 0 计——「不知道预算」不该惩罚业绩）：
//! - 标签重叠 40%：entry.categories ∩ 招标语料（REQ 陈述/评分项/项目名的
//!   子串命中——中文无分词的诚实替代：标签命中 = 标签字符串在语料中出现）；
//! - 金额规模 30%：min/max 比值（0.5 以上满分——五千万项目配五百万到五亿
//!   业绩都算同量级；低于 0.1 记 0——量级差太远对评标无说服力）；
//! - 时间新旧 20%：完成日期距基准日 3 年内满分（多数招标「近三年类似项目」
//!   门槛）、3-5 年半分、更早 0 分；
//! - 客户类型 10%：政府/国企/民营三方推断（名称关键词规则），同型满分、
//!   异型 0 分、任一侧未知 0.5（弃权——不惩罚不确定）。
//!
//! 诚实纪律：
//! - 分数是**排序依据**不是「匹配正确率」——reasons 逐因子给出人读依据，
//!   最终选哪几条进标书由 Agent/用户裁决；
//! - 渲染只吃库内登记数据（name/client/amount_wan/completed_on/key_points），
//!   不编造未登记的细节（无 key_points 的业绩描述段只写事实行）。

use crate::dates;
use crate::ToolError;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// 业绩标签数上界（防把整篇标书塞进 categories——标签是「形态」不是全文）。
pub const MAX_CATEGORIES: usize = 20;
/// 材料路径数上界（一份业绩的合同/验收单/中标通知书等，正常 < 10）。
pub const MAX_MATERIALS: usize = 20;
/// 亮点条数上界。
pub const MAX_KEY_POINTS: usize = 20;
/// 标签/亮点/材料路径单条长度上界（字符）。
const MAX_ITEM_LEN: usize = 500;
/// 「近三年」窗口（天）——时间因子满分线（评标口径的通行门槛）。
pub const RECENT_WINDOW_DAYS: i64 = 365 * 3;
/// 「可接受」窗口（天）——时间因子半分线。
pub const ACCEPTABLE_WINDOW_DAYS: i64 = 365 * 5;
/// 金额因子满分线（min/max ≥ 0.5）。
const AMOUNT_FULL_RATIO: f64 = 0.5;
/// 金额因子零分线（min/max < 0.1）。
const AMOUNT_ZERO_RATIO: f64 = 0.1;
/// 标签因子满分命中数（命中 3 个标签即满分——多标签命中边际递减）。
const TAG_FULL_HITS: usize = 3;

// ---------------------------------------------------------------- 实体面

/// 业绩材料库条目（#315 任务①规格的仓内口径落地）。
///
/// 与 issue 草案的差异（按仓内既定约定）：
/// - `amount: Decimal` → `amount_wan: f64`（万元——与 EvidenceFields.amount_wan
///   / Evidence.amount_wan 同口径，仓内无 decimal 依赖）；
/// - `date: NaiveDate` → `completed_on: String`（ISO YYYY-MM-DD 字符串——
///   dates.rs 的既定口径，见 evidence.rs 同字段）；
/// - `materials: Vec<PathBuf>` → `Vec<String>`（库文件是 JSON 跨平台形态；
///   路径有效性由服务层在使用时刻裁决——登记面只做长度/空白校验）；
/// - 增补 `subject_name`（乙方主体——证据登记面需要，见 write_performance_
///   section）与 `created_at`（审计面）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceEntry {
    /// 库内稳定 id（PERF-0001 起单调签发——库文件单一序列）。
    pub id: String,
    /// 项目名称（业绩合同标的项目——不可空）。
    pub name: String,
    /// 客户/发包人（甲方——不可空）。
    pub client: String,
    /// 合同金额（万元，>= 0 有限数）。
    pub amount_wan: f64,
    /// 完成日期（ISO YYYY-MM-DD）。
    pub completed_on: String,
    /// 行业/类型标签（匹配面——「医疗设备」「超融合」这类短标签）。
    #[serde(default)]
    pub categories: Vec<String>,
    /// 材料文件路径（合同扫描/验收单——登记面只记录，导入/绑定面才验证）。
    #[serde(default)]
    pub materials: Vec<String>,
    /// 亮点（写给正文用的事实点——「3 个月交付」「一次验收通过」）。
    #[serde(default)]
    pub key_points: Vec<String>,
    /// 乙方主体（我方签约主体——业绩证据 subject_name 的来源；缺省继承
    /// 库级 owner_name）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_name: Option<String>,
    /// 登记时间（ISO 日期——审计面）。
    pub created_at: String,
}

/// 条目登记规格（无 id/created_at——由库层签发）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PerformanceEntrySpec {
    pub name: String,
    pub client: String,
    pub amount_wan: f64,
    pub completed_on: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub materials: Vec<String>,
    #[serde(default)]
    pub key_points: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_name: Option<String>,
}

/// 条目校验（纯函数——错误文案即 E_UNPROCESSABLE reason 素材）。
///
/// - name/client 非空白；
/// - amount_wan 有限且 >= 0；
/// - completed_on 严格 ISO（dates::is_valid_ymd）；
/// - categories/materials/key_points 条数上界 + 单条非空白 + 单条长度上界；
/// - subject_name 在场时非空白。
pub fn validate_entry_spec(spec: &PerformanceEntrySpec) -> Result<(), ToolError> {
    let un = |reason: String| ToolError::EUnprocessable { reason };
    if spec.name.trim().is_empty() {
        return Err(un("业绩条目 name（项目名称）不可为空".into()));
    }
    if spec.client.trim().is_empty() {
        return Err(un("业绩条目 client（客户/发包人）不可为空".into()));
    }
    if !spec.amount_wan.is_finite() || spec.amount_wan < 0.0 {
        return Err(un(format!(
            "业绩条目 amount_wan 非法: {}（须为 >= 0 的有限数，万元口径）",
            spec.amount_wan
        )));
    }
    if !dates::is_valid_ymd(&spec.completed_on) {
        return Err(un(format!(
            "业绩条目 completed_on 非法 ISO 日期: {:?}（须 YYYY-MM-DD）",
            spec.completed_on
        )));
    }
    for (label, list, cap) in [
        ("categories", &spec.categories, MAX_CATEGORIES),
        ("materials", &spec.materials, MAX_MATERIALS),
        ("key_points", &spec.key_points, MAX_KEY_POINTS),
    ] {
        if list.len() > cap {
            return Err(un(format!("{label} 条数越界: {}（须 <= {cap}）", list.len())));
        }
        for item in list {
            if item.trim().is_empty() {
                return Err(un(format!("{label} 含空白条目（空字符串不是合法标签/路径/亮点）")));
            }
            if item.chars().count() > MAX_ITEM_LEN {
                return Err(un(format!(
                    "{label} 单条超长: {} 字符（上界 {MAX_ITEM_LEN}）",
                    item.chars().count()
                )));
            }
        }
    }
    if let Some(s) = &spec.subject_name {
        if s.trim().is_empty() {
            return Err(un("subject_name 在场时不可为空白（要么不传、要么给真实主体名）".into()));
        }
    }
    Ok(())
}

/// 语义幂等键（同键重登 = 幂等返回已有 id——borrow：create_bid_project
/// 的 external_key 语义；不引入自增碰撞面）。
///
/// 四元组：(name 归一空白, client 归一空白, amount_wan, completed_on)。
/// 归一 = 去首尾空白 + 连续空白折叠（登记面的人手笔误容忍，不改语义）。
pub fn entry_dedupe_key(spec: &PerformanceEntrySpec) -> (String, String, f64, String) {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    (
        norm(&spec.name),
        norm(&spec.client),
        spec.amount_wan,
        spec.completed_on.clone(),
    )
}

/// 已入库条目与规格是否同幂等键（重放判定）。
pub fn same_entry(entry: &PerformanceEntry, spec: &PerformanceEntrySpec) -> bool {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    norm(&entry.name) == norm(&spec.name)
        && norm(&entry.client) == norm(&spec.client)
        && entry.amount_wan == spec.amount_wan
        && entry.completed_on == spec.completed_on
}

// ---------------------------------------------------------------- 客户类型

/// 客户类型（匹配因子④——名称关键词推断，确定性规则）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientType {
    /// 政府/事业单位（局/委/办/中心/政府/医院/学校/研究院…）。
    Government,
    /// 国有企业（集团/有限 + 国有信号，或「国有」明示）。
    Soe,
    /// 民营企业。
    Private,
    /// 无法判定（弃权——匹配因子记 0.5 不惩罚）。
    Other,
}

impl ClientType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ClientType::Government => "government",
            ClientType::Soe => "soe",
            ClientType::Private => "private",
            ClientType::Other => "other",
        }
    }

    /// 展示名（人读面）。
    pub fn label_zh(&self) -> &'static str {
        match self {
            ClientType::Government => "政府/事业单位",
            ClientType::Soe => "国有企业",
            ClientType::Private => "民营企业",
            ClientType::Other => "未判定",
        }
    }
}

/// 客户类型推断（纯函数——关键词规则，先政府后国企后民营）。
///
/// 规则（形态不是字面白名单——「XX市卫生健康局」与「局」都命中）：
/// - 政府：局|委|办|政府|中心|医院|学校|大学|学院|研究院|研究所|站|队
/// - 国企：国有|国投|城投|交投|水投|建投（地方平台公司常见前缀）
/// - 民营：有限公司|有限责任公司|股份（无国有信号时的默认商业形态）
/// - 其余 → Other（弃权）
pub fn infer_client_type(name: &str) -> ClientType {
    let gov = [
        "局", "委员会", "委", "办公室", "办", "政府", "中心", "医院", "学校", "大学",
        "学院", "研究院", "研究所", "监测站", "站", "大队", "队",
    ];
    let soe = ["国有", "国投", "城投", "交投", "水投", "建投", "供销"];
    if gov.iter().any(|k| name.contains(k)) {
        return ClientType::Government;
    }
    if soe.iter().any(|k| name.contains(k)) {
        return ClientType::Soe;
    }
    if name.contains("有限公司") || name.contains("股份") || name.contains("集团") {
        return ClientType::Private;
    }
    ClientType::Other
}

// ---------------------------------------------------------------- 库快照

/// 业绩库快照（跨项目持久化的数据形态——域层纯数据，磁盘读写住 bid-tools）。
///
/// 与项目态（DomainStore）**正交**：库不属于任何项目，多个项目共用同一份
/// 库文件（#315 的核心诉求——业绩是投标人的长期资产，不是单项目的输入）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceLibrarySnapshot {
    /// 库 schema 版本（高于当前支持的版本 → 拒载不猜测——对齐 store_io 纪律）。
    #[serde(default = "library_default_version")]
    pub schema_version: u32,
    /// 库主（投标人主体名——write_performance_section 的证据 subject_name
    /// 默认源；None = 未登记）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// 条目（PERF-0001 起单调签发，按登记序）。
    #[serde(default)]
    pub entries: Vec<PerformanceEntry>,
    /// id 序列（下一个 PERF 编号——快照往返保持）。
    #[serde(default)]
    pub id_seq: u64,
}

/// 当前库 schema 版本。
pub const LIBRARY_SCHEMA_VERSION: u32 = 1;

fn library_default_version() -> u32 {
    LIBRARY_SCHEMA_VERSION
}

impl Default for PerformanceLibrarySnapshot {
    fn default() -> Self {
        Self {
            schema_version: LIBRARY_SCHEMA_VERSION,
            owner_name: None,
            entries: Vec::new(),
            id_seq: 0,
        }
    }
}

impl PerformanceLibrarySnapshot {
    /// 签发下一个 PERF id（纯函数——序号自增，PERF-0001 起）。
    pub fn next_perf_id(&mut self) -> String {
        self.id_seq += 1;
        format!("PERF-{:04}", self.id_seq)
    }

    /// 按 id 查条目。
    pub fn entry(&self, id: &str) -> Option<&PerformanceEntry> {
        self.entries.iter().find(|e| e.id == id)
    }
}

// ---------------------------------------------------------------- 匹配面

/// 匹配信号（服务层从项目态提炼——域层不碰存储，只吃纯数据）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectSignals {
    /// 招标语料（REQ 陈述 + 评分项 + 项目名拼接——标签子串命中的对象）。
    pub corpus: String,
    /// 预算（万元——金额因子；None = 未知（缺信号重归一））。
    pub budget_wan: Option<f64>,
    /// 时间基准日（YYYY-MM-DD——近三年判定；服务层给开标日/参数日/今日，
    /// 并在响应面注明来源）。
    pub base_date: String,
    /// 采购人类型（None = 未知）。
    pub client_type: Option<ClientType>,
}

/// 单条匹配结果（分数 + 逐因子依据——分数供排序，reasons 供人读裁决）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceMatch {
    pub perf_id: String,
    pub name: String,
    /// 0..=1 加权分（四因子缺信号重归一后的可比分数）。
    pub score: f64,
    /// 逐因子明细（factor → (得分 0..=1, 该因子是否在场)）。
    pub factors: Vec<MatchFactor>,
    /// 人读理由（每因子一条——「标签命中：医疗设备、CT」「近三年内（2024-06 完成，基准 2026-11）」）。
    pub reasons: Vec<String>,
}

/// 单因子得分（诚实面——不在场的因子标 available=false，不冒充 0 分）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchFactor {
    /// 因子名（tag|amount|recency|client——String 面：serde 友好）。
    pub factor: String,
    pub score: f64,
    pub available: bool,
    pub note: String,
}

/// 标签因子打分：categories 在语料中的子串命中数。
fn tag_factor(entry: &PerformanceEntry, signals: &ProjectSignals) -> MatchFactor {
    let mut hits: Vec<&str> = Vec::new();
    for cat in &entry.categories {
        let c = cat.trim();
        if !c.is_empty() && signals.corpus.contains(c) {
            hits.push(c);
        }
    }
    let score = (hits.len() as f64 / TAG_FULL_HITS as f64).min(1.0);
    let note = if entry.categories.is_empty() {
        "业绩无标签（登记时补 categories 可提升匹配精度）".to_string()
    } else if hits.is_empty() {
        format!("标签未命中（标签：{}；语料未见）", entry.categories.join("、"))
    } else {
        format!("标签命中：{}", hits.join("、"))
    };
    MatchFactor { factor: "tag".into(), score, available: true, note }
}

/// 金额因子打分：min/max 比值阶梯。
fn amount_factor(entry: &PerformanceEntry, signals: &ProjectSignals) -> MatchFactor {
    match signals.budget_wan {
        None => MatchFactor {
            factor: "amount".into(),
            score: 0.0,
            available: false,
            note: "项目预算未知（缺信号——本因子不参与计分）".into(),
        },
        Some(budget) if budget <= 0.0 || entry.amount_wan <= 0.0 => MatchFactor {
            factor: "amount".into(),
            score: 0.0,
            available: true,
            note: "金额不可比（业绩或预算为 0）".into(),
        },
        Some(budget) => {
            let ratio = (entry.amount_wan.min(budget) / entry.amount_wan.max(budget)) as f64;
            let score = if ratio >= AMOUNT_FULL_RATIO {
                1.0
            } else if ratio <= AMOUNT_ZERO_RATIO {
                0.0
            } else {
                (ratio - AMOUNT_ZERO_RATIO) / (AMOUNT_FULL_RATIO - AMOUNT_ZERO_RATIO)
            };
            MatchFactor {
                factor: "amount".into(),
                score,
                available: true,
                note: format!(
                    "规模比 {ratio:.2}（业绩 {:.0} 万 vs 预算 {budget:.0} 万）",
                    entry.amount_wan
                ),
            }
        }
    }
}

/// 时间因子打分：完成日期距基准日的窗口阶梯。
fn recency_factor(entry: &PerformanceEntry, signals: &ProjectSignals) -> MatchFactor {
    let Some((dy, dm, dd)) = dates::parse_ymd(&entry.completed_on) else {
        return MatchFactor {
            factor: "recency".into(),
            score: 0.0,
            available: true,
            note: "完成日期不可解析（按最旧计）".into(),
        };
    };
    let Some((by, bm, bd)) = dates::parse_ymd(&signals.base_date) else {
        return MatchFactor {
            factor: "recency".into(),
            score: 0.0,
            available: false,
            note: "基准日不可解析（缺信号——本因子不参与计分）".into(),
        };
    };
    let days =
        dates::days_from_civil(by, bm, bd) - dates::days_from_civil(dy, dm, dd);
    let (score, note) = if days < 0 {
        // 完成日期晚于基准日（录入错误或在建项目）——诚实报错位，不给分
        (0.0, format!("完成日期晚于基准日 {} 天（数据可疑——检查录入）", -days))
    } else if days <= RECENT_WINDOW_DAYS {
        (1.0, format!("近三年内（{days} 天前完成，基准 {base_date}）", base_date = signals.base_date))
    } else if days <= ACCEPTABLE_WINDOW_DAYS {
        (0.5, format!("{} 天前完成（3-5 年——半分；多数招标要求近三年）", days))
    } else {
        (0.0, format!("{days} 天前完成（超过 5 年——0 分）"))
    };
    MatchFactor { factor: "recency".into(), score, available: true, note }
}

/// 客户类型因子：同型 1 / 异型 0 / 任一侧未知 0.5（弃权）。
fn client_factor(entry: &PerformanceEntry, signals: &ProjectSignals) -> MatchFactor {
    let entry_type = infer_client_type(&entry.client);
    match signals.client_type {
        None => MatchFactor {
            factor: "client".into(),
            score: 0.0,
            available: false,
            note: "采购人类型未知（缺信号——本因子不参与计分）".into(),
        },
        Some(p) if p == ClientType::Other || entry_type == ClientType::Other => MatchFactor {
            factor: "client".into(),
            score: 0.5,
            available: true,
            note: format!(
                "客户类型不可比（业绩侧 {} / 采购人侧 {}——弃权 0.5）",
                entry_type.label_zh(),
                p.label_zh()
            ),
        },
        Some(p) => MatchFactor {
            factor: "client".into(),
            score: if p == entry_type { 1.0 } else { 0.0 },
            available: true,
            note: format!(
                "客户类型{}（业绩 {} / 采购人 {}）",
                if p == entry_type { "一致" } else { "不一致" },
                entry_type.label_zh(),
                p.label_zh()
            ),
        },
    }
}

/// 因子权重（tag/amount/recency/client——和恒为 1.0）。
fn factor_weight(factor: &str) -> f64 {
    match factor {
        "tag" => 0.4,
        "amount" => 0.3,
        "recency" => 0.2,
        _ => 0.1,
    }
}

/// 单条打分（纯函数）：四因子加权 + 缺信号重归一（available 因子的权重和
/// 作分母——全缺时 0 分且 reasons 说明）。
pub fn match_entry(entry: &PerformanceEntry, signals: &ProjectSignals) -> PerformanceMatch {
    let factors = vec![
        tag_factor(entry, signals),
        amount_factor(entry, signals),
        recency_factor(entry, signals),
        client_factor(entry, signals),
    ];
    let available_weight: f64 = factors
        .iter()
        .filter(|f| f.available)
        .map(|f| factor_weight(&f.factor))
        .sum();
    let score = if available_weight > 0.0 {
        factors
            .iter()
            .filter(|f| f.available)
            .map(|f| f.score * factor_weight(&f.factor))
            .sum::<f64>()
            / available_weight
    } else {
        0.0
    };
    let reasons: Vec<String> = factors.iter().map(|f| f.note.clone()).collect();
    PerformanceMatch {
        perf_id: entry.id.clone(),
        name: entry.name.clone(),
        score: (score * 10_000.0).round() / 10_000.0, // 4 位小数——排序稳定且人读友好
        factors,
        reasons,
    }
}

/// 推荐排序（纯函数）：分数降序，同分按 id 字典序（确定性——同输入恒同序）。
pub fn rank_entries<'a>(
    entries: &'a [PerformanceEntry],
    signals: &ProjectSignals,
    top_k: usize,
) -> Vec<PerformanceMatch> {
    let mut matches: Vec<PerformanceMatch> =
        entries.iter().map(|e| match_entry(e, signals)).collect();
    matches.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.perf_id.cmp(&b.perf_id))
    });
    matches.truncate(top_k.max(1));
    matches
}

// ---------------------------------------------------------------- 渲染面

/// 业绩表渲染（GFM——序号/项目名称/合同额(万元)/完成日期/客户/业主）。
///
/// 纯函数：单元格 `|` 与换行转义（borrow：tables.rs 的单元格纪律——错位表格
/// 比没表格更糟）。
pub fn render_performance_table(entries: &[PerformanceEntry]) -> String {
    let esc = |s: &str| s.replace('|', "\\|").replace('\n', " ");
    let mut out = String::from("| 序号 | 项目名称 | 合同额（万元） | 完成日期 | 客户 |\n");
    out.push_str("|---|---|---|---|---|\n");
    for (i, e) in entries.iter().enumerate() {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            i + 1,
            esc(&e.name),
            e.amount_wan,
            esc(&e.completed_on),
            esc(&e.client),
        );
    }
    out
}

/// 单条业绩描述段渲染（key_points 进正文；ev_id 在场时附 [EV-*] 引用——
/// 引用回查面只认识已登记 EV）。
pub fn render_performance_paragraph(entry: &PerformanceEntry, ev_id: Option<&str>) -> String {
    let mut out = format!(
        "**{}**（客户：{}；合同额 {} 万元；完成于 {}）",
        entry.name, entry.client, entry.amount_wan, entry.completed_on
    );
    if let Some(ev) = ev_id {
        let _ = write!(out, "，佐证材料见 [{ev}]");
    }
    if entry.key_points.is_empty() {
        out.push_str("。");
    } else {
        let _ = write!(out, "：{}。", entry.key_points.join("；"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, client: &str, amount: f64, done: &str) -> PerformanceEntrySpec {
        PerformanceEntrySpec {
            name: name.into(),
            client: client.into(),
            amount_wan: amount,
            completed_on: done.into(),
            ..Default::default()
        }
    }

    fn entry(id: &str, name: &str, client: &str, amount: f64, done: &str) -> PerformanceEntry {
        PerformanceEntry {
            id: id.into(),
            name: name.into(),
            client: client.into(),
            amount_wan: amount,
            completed_on: done.into(),
            categories: vec![],
            materials: vec![],
            key_points: vec![],
            subject_name: None,
            created_at: "2026-01-01".into(),
        }
    }

    // ---- 校验面

    #[test]
    fn validate_rejects_blank_name_client_bad_date_negative_amount() {
        assert!(validate_entry_spec(&spec("", "甲", 1.0, "2024-01-01")).is_err());
        assert!(validate_entry_spec(&spec("名", " ", 1.0, "2024-01-01")).is_err());
        assert!(validate_entry_spec(&spec("名", "甲", 1.0, "2024-1-1")).is_err());
        assert!(validate_entry_spec(&spec("名", "甲", -1.0, "2024-01-01")).is_err());
        assert!(validate_entry_spec(&spec("名", "甲", f64::NAN, "2024-01-01")).is_err());
        assert!(validate_entry_spec(&spec("名", "甲", 100.0, "2024-01-01")).is_ok());
    }

    #[test]
    fn validate_caps_list_lengths_and_blank_items() {
        let mut s = spec("名", "甲", 1.0, "2024-01-01");
        s.categories = vec!["标签".to_string(); MAX_CATEGORIES + 1];
        assert!(validate_entry_spec(&s).is_err());
        s.categories = vec!["标签".to_string(), "  ".to_string()];
        assert!(validate_entry_spec(&s).is_err());
        s.categories = vec!["标签".to_string()];
        s.key_points = vec!["亮点".to_string()];
        s.materials = vec!["D:/材料/合同.pdf".to_string()];
        assert!(validate_entry_spec(&s).is_ok());
        s.subject_name = Some("  ".into());
        assert!(validate_entry_spec(&s).is_err());
    }

    // ---- 幂等键

    #[test]
    fn dedupe_key_normalizes_whitespace_only() {
        let a = spec("超融合 集群 项目", "甲 方", 100.0, "2024-06-30");
        let mut b = spec("超融合  集群  项目", "甲  方", 100.0, "2024-06-30");
        b.name = " 超融合 集群 项目 ".into();
        assert_eq!(entry_dedupe_key(&a), entry_dedupe_key(&b));
        b.amount_wan = 100.5;
        assert_ne!(entry_dedupe_key(&a), entry_dedupe_key(&b));
        let e = entry("PERF-0001", " 超融合 集群 项目 ", "甲 方", 100.0, "2024-06-30");
        assert!(same_entry(&e, &a));
        assert!(!same_entry(&e, &b));
    }

    // ---- 客户类型

    #[test]
    fn client_type_inference_deterministic_rules() {
        assert_eq!(infer_client_type("黑龙江示例市卫生健康局"), ClientType::Government);
        assert_eq!(infer_client_type("示例市第一人民医院"), ClientType::Government);
        assert_eq!(infer_client_type("示例大学"), ClientType::Government);
        assert_eq!(infer_client_type("示例城投集团有限公司"), ClientType::Soe);
        assert_eq!(infer_client_type("黑龙江省示例科技有限公司"), ClientType::Private);
        assert_eq!(infer_client_type("某合作社"), ClientType::Other);
    }

    // ---- 打分

    fn signals(corpus: &str, budget: Option<f64>, base: &str, ct: Option<ClientType>) -> ProjectSignals {
        ProjectSignals {
            corpus: corpus.into(),
            budget_wan: budget,
            base_date: base.into(),
            client_type: ct,
        }
    }

    #[test]
    fn full_signal_match_scores_higher_than_mismatch() {
        let mut good = entry("PERF-0001", "医疗设备采购项目", "示例市第一人民医院", 800.0, "2025-06-30");
        good.categories = vec!["医疗设备".to_string(), "CT".to_string(), "手术室设备".to_string()];
        let mut bad = entry("PERF-0002", "园林绿化项目", "示例绿化有限公司", 5.0, "2018-01-01");
        bad.categories = vec!["绿化".to_string(), "苗木".to_string()];
        let s = signals(
            "本项目采购 CT 与医疗设备，含手术室设备安装。评标办法：类似项目业绩。",
            Some(1000.0),
            "2026-11-07",
            Some(ClientType::Government),
        );
        let g = match_entry(&good, &s);
        let b = match_entry(&bad, &s);
        assert!(g.score > 0.85, "全信号命中应得高分，实际 {}", g.score);
        assert!(b.score < 0.15, "全失配应得低分，实际 {}", b.score);
        // 逐因子核验（算术对账——不放宽）
        let tag = g.factors.iter().find(|f| f.factor == "tag").unwrap();
        assert_eq!(tag.score, 1.0, "3 标签命中 = 满分");
        let amt = g.factors.iter().find(|f| f.factor == "amount").unwrap();
        assert_eq!(amt.score, 1.0, "800/1000 比值 0.8 ≥ 0.5 = 满分");
        let rec = g.factors.iter().find(|f| f.factor == "recency").unwrap();
        assert_eq!(rec.score, 1.0, "2025-06-30 距 2026-11-07 约 495 天 < 3 年");
        let cli = g.factors.iter().find(|f| f.factor == "client").unwrap();
        assert_eq!(cli.score, 1.0, "医院 vs 政府 = 同型");
    }

    #[test]
    fn missing_signals_renormalize_not_zero() {
        let mut e = entry("PERF-0001", "项目", "示例市第一人民医院", 500.0, "2025-01-01");
        e.categories = vec!["医疗设备".to_string(), "CT".to_string(), "监护仪".to_string()];
        // 无预算 + 无采购人类型：只有 tag(0.4)+recency(0.2) 参与
        let s = signals("采购医疗设备（含 CT、监护仪）。", None, "2026-11-07", None);
        let m = match_entry(&e, &s);
        assert_eq!(m.factors.iter().filter(|f| f.available).count(), 2);
        // tag=1, recency=1 → (0.4+0.2)/0.6 = 1.0
        assert!((m.score - 1.0).abs() < 1e-9, "缺信号重归一后应满分，实际 {}", m.score);
        let unavailable: Vec<_> = m.factors.iter().filter(|f| !f.available).collect();
        assert_eq!(unavailable.len(), 2, "amount/client 缺信号");
    }

    #[test]
    fn amount_ratio_ladder_and_recency_ladder() {
        let e = |amount: f64| entry("PERF-0001", "项目", "甲", amount, "2025-01-01");
        let s = signals("语料", Some(1000.0), "2026-11-07", None);
        assert_eq!(amount_factor(&e(600.0), &s).score, 1.0); // 0.6 ≥ 0.5
        assert_eq!(amount_factor(&e(100.0), &s).score, 0.0); // 0.1 ≤ 0.1 → 0
        let mid = amount_factor(&e(300.0), &s).score; // 0.3 → (0.3-0.1)/0.4 = 0.5
        assert!((mid - 0.5).abs() < 1e-9);
        let old = entry("PERF-0002", "项目", "甲", 100.0, "2020-01-01");
        assert_eq!(recency_factor(&old, &s).score, 0.0, "约 6.8 年前 = 0 分");
        let mid_old = entry("PERF-0003", "项目", "甲", 100.0, "2022-06-01");
        assert_eq!(recency_factor(&mid_old, &s).score, 0.5, "约 4.4 年前 = 半分");
    }

    #[test]
    fn rank_is_deterministic_and_truncates() {
        let mut a = entry("PERF-0002", "乙项目", "甲", 100.0, "2025-01-01");
        a.categories = vec!["医疗".to_string()];
        let mut b = entry("PERF-0001", "甲项目", "甲", 100.0, "2025-01-01");
        b.categories = vec!["医疗".to_string()];
        let s = signals("采购医疗设备（含 CT、监护仪）。", None, "2026-11-07", None);
        let r1 = rank_entries(&[a.clone(), b.clone()], &s, 10);
        let r2 = rank_entries(&[b.clone(), a.clone()], &s, 10);
        assert_eq!(r1, r2, "同分按 id 字典序——输入序不影响输出序");
        assert_eq!(r1[0].perf_id, "PERF-0001");
        assert_eq!(rank_entries(&[a, b], &s, 1).len(), 1, "top_k 截断");
    }

    // ---- 渲染

    #[test]
    fn table_escapes_pipes_and_paragraph_uses_key_points() {
        let mut e = entry("PERF-0001", "带|竖线|项目", "甲", 123.4, "2024-06-01");
        e.key_points = vec!["3 个月交付".to_string(), "一次验收通过".to_string()];
        let table = render_performance_table(std::slice::from_ref(&e));
        assert!(table.contains("\\|"), "竖线必须转义：{table}");
        assert!(table.contains("123.4"));
        assert!(table.starts_with("| 序号 |"));
        let p = render_performance_paragraph(&e, Some("EV-0001"));
        assert!(p.contains("[EV-0001]"), "证据引用必须在段内");
        assert!(p.contains("3 个月交付；一次验收通过"), "key_points 进描述段：{p}");
        let no_kp = render_performance_paragraph(&e, None);
        assert!(no_kp.ends_with("。"));
        assert!(!no_kp.contains("[EV"));
    }
}
