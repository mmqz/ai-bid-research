//! tables.rs —— 标书六种结构化表格（issue #103 子任务 B）。
//!
//! 定位：**标书正文里的表格不是装饰，是评委打分的读取面**。实测导出
//! docx `<w:tbl>` 数量 = 0——正文全是自然段，而真实标书的评分点（技术
//! 参数逐条响应、人员配置、设备清单、业绩、报价明细）几乎全部落在表格
//! 单元格里。本模块补齐「结构化表格 → markdown → 现有导出链 → docx 真
//! 表格」这一段，并把**负偏离标红**钉进导出层。
//!
//! ## 分工：本模块只负责「struct → markdown」
//!
//! 关键调研结论（issue §关键技术点 1 的答案）：`bid-tools/src/export.rs`
//! 在 #68 阶段 3 已经能解析 md 表格（`parse_blocks` → `Block::Table` →
//! `build_table` → `docx_rs::Table`）。因此本模块**不需要**重新实现
//! docx 表格构建——[`BidTable::to_markdown`] 产出 GFM 表格文本，交给
//! 既有导出链即可得到真正的 `<w:tbl>`。
//!
//! 唯一需要扩展 export.rs 的是**着色**：既有 `build_table` 只做「首行
//! 加粗」，单元格内没有任何 run 样式通道。本模块约定一条**行内标记**
//! 语法 `{{r:文本}}`（red），由 `export::build_table` 识别并把该 run
//! 渲染为红色。这样「负偏离标红」这个表达力走的是**数据面**而不是一个
//! 新的 docx 构建 API——markdown 仍然自足（纯文本阅读时标记可读、可诊断），
//! 导出层只是把它渲染出来。
//!
//! 注：issue 里提到「office-io 的 `RunSpec::styled()`」——调研结论是
//! **导出层不走 office-io**：`export.rs` 直接依赖 `docx-rs 0.4.22` 构建
//! 文档；office-io 的写面 `DocxWriter` 在 bid-tools 依赖图里不可达，且
//! 其 `RunSpec` 无颜色位（全文只有两处写死 `color("auto")`）。等价表达
//! 力在 docx-rs 上是 `Run::color("#FF0000")`——本模块的红标即落此。
//!
//! ## 六种表格与字段口径
//!
//! | 表 | 典型用法 | 负偏离面 |
//! |---|---|---|
//! | [`BidTable::ProjectOverview`] | 项 / 值 两列直读 | — |
//! | [`BidTable::TechDeviation`] | **评委打分主要依据** | ★ 负偏离整行标红 |
//! | [`BidTable::Personnel`] | 拟派团队配置 | — |
//! | [`BidTable::Equipment`] | 拟投入设备清单 | — |
//! | [`BidTable::Performance`] | 类似业绩一览 | — |
//! | [`BidTable::PriceBreakdown`] | 分项报价明细（含合计） | — |
//!
//! 纪律（与 bid-domain 同族）：
//! - **诚实态**：不可比（Incomparable）不伪装成「无偏离」——沿用
//!   [`bid_domain::param::Deviation`] 的四态，不砍成三态；
//! - **单元格转义**：正文里的 `|` 与换行会破坏 GFM 表格结构（`|` 会被
//!   切成新列——评标现场看到错位表格比没表格更糟），渲染时统一转义；
//! - **纯函数**：同输入恒同字节，无 IO、无时间、无随机。

use bid_domain::param::Deviation;
use serde::Serialize;
use std::fmt::Write as _;

// ---------------------------------------------------------------- 行内标记

/// 红标行内标记的语法：`{{r:文本}}`。
///
/// 为什么是它：GFM 表格单元格内不能放块级语法，而 `**粗体**` 在现有
/// 导出链里也不解析（`build_table` 只按整格加粗首行）。用一对**不会与
/// 中文正文/参数原文撞车**的定界符承载「这一格要标红」，让 markdown
/// 自足、导出层专注渲染。
///
/// **分隔符必须是 `:` 不能是 `|`**：`|` 是 markdown 表格的列分隔符，
/// `{{r|2年}}` 到了按列切分的那一层会被劈成 `{{r` + `2年}}` 两格——
/// 表格多一列且标红静默失效。`:` 在单元格正文里几乎不出现该形态，
/// 与表格语法零冲突。
pub const RED_MARK_OPEN: &str = "{{r:";
/// 红标行内标记闭合符（配对 [`RED_MARK_OPEN`]）。
pub const RED_MARK_CLOSE: &str = "}}";

/// 数值未知时的待填占位（#117：诚实态——不落 0/空白，落【待填】）。
///
/// 与 bid-domain `party::Blank` 同口径（全角方括号）；不直接引用是为了
/// 本模块零业务依赖（表格层自己能独立说清「缺就是缺」）。
pub const PENDING_FILL: &str = "【待填】";

/// 把文本包成红标 run（导出层渲染为红色；纯文本阅读时仍可读）。
pub fn red(text: impl AsRef<str>) -> String {
    format!("{RED_MARK_OPEN}{}{RED_MARK_CLOSE}", text.as_ref())
}
// ---------------------------------------------------------------- 行类型

/// 技术参数偏离表的一行（★ 评委打分主要依据）。
///
/// `deviation` 取 [`bid_domain::param::Deviation`] 的**四态**（含
/// Incomparable）而不是 issue 草案的三态：不可比是「待人工裁决」，
/// 把它压成「无偏离」就是拿诚实态换太平——评标现场最危险的那种假。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TechDeviationRow {
    /// 序号（1 起，渲染时原样输出——评委按序号定位）
    pub seq: usize,
    /// 招标要求原文（如「≥100GB」「不低于 3 年」）
    pub requirement: String,
    /// 投标响应原文（如「120GB」）
    pub response: String,
    /// 偏离判定（负偏离 ★危险——整行标红）
    pub deviation: Deviation,
    /// 说明（偏离理由/佐证位置；空串 = 无说明）
    pub note: String,
}

impl TechDeviationRow {
    /// 便捷构造（序号 + 要求 + 响应 + 偏离，说明留空）。
    pub fn new(
        seq: usize,
        requirement: impl Into<String>,
        response: impl Into<String>,
        deviation: Deviation,
    ) -> Self {
        Self {
            seq,
            requirement: requirement.into(),
            response: response.into(),
            deviation,
            note: String::new(),
        }
    }

    /// 带说明构造。
    pub fn with_note(
        seq: usize,
        requirement: impl Into<String>,
        response: impl Into<String>,
        deviation: Deviation,
        note: impl Into<String>,
    ) -> Self {
        Self {
            seq,
            requirement: requirement.into(),
            response: response.into(),
            deviation,
            note: note.into(),
        }
    }

    /// 从招标参数行派生偏离表行（单一真源复用——偏离判定归
    /// [`bid_domain::param::derive_deviation`]，本模块不重算）。
    pub fn from_param(seq: usize, param_name: &str, required: &str, response: &str) -> Self {
        Self::new(
            seq,
            format!("{param_name}：{required}"),
            response,
            bid_domain::param::derive_deviation(required, response),
        )
    }

    /// 偏离中文标签（评标文档口径——不输出 NEGATIVE 这类枚举名）。
    pub fn deviation_label(&self) -> &'static str {
        match self.deviation {
            Deviation::None => "无偏离",
            Deviation::Positive => "正偏离",
            Deviation::Negative => "负偏离",
            Deviation::Incomparable => "不可比",
        }
    }

    /// 是否负偏离（★废标级）。
    pub fn is_negative(&self) -> bool {
        self.deviation == Deviation::Negative
    }
}

/// 人员配置表的一行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonnelRow {
    /// 岗位（项目经理/技术负责人/安全员…）
    pub position: String,
    /// 姓名（暗标场景由调用方脱敏）
    pub name: String,
    /// 职称/执业资格（如「高级工程师 / 一级建造师」）
    pub title: String,
    /// 专业
    pub major: String,
    /// 本专业年限（年；None = 未知 → 渲染【待填】——#117：不落「0 年」假值）
    pub years: Option<u32>,
    /// 备注（在岗状态、社保等）
    pub note: String,
}

impl PersonnelRow {
    pub fn new(
        position: impl Into<String>,
        name: impl Into<String>,
        title: impl Into<String>,
        major: impl Into<String>,
        years: Option<u32>,
    ) -> Self {
        Self {
            position: position.into(),
            name: name.into(),
            title: title.into(),
            major: major.into(),
            years,
            note: String::new(),
        }
    }
}

/// 设备清单表的一行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EquipmentRow {
    /// 设备名称
    pub name: String,
    /// 型号规格
    pub model: String,
    /// 数量（None = 未知 → 渲染【待填】——#117）
    pub quantity: Option<u32>,
    /// 单位（台/套/辆…）
    pub unit: String,
    /// 产地/品牌
    pub origin: String,
    /// 主要性能指标（关键参数——评委按此比对招标要求）
    pub spec: String,
    /// 进场时间（如「合同签订后 7 日内」）
    pub arrival: String,
}

impl EquipmentRow {
    pub fn new(
        name: impl Into<String>,
        model: impl Into<String>,
        quantity: Option<u32>,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            model: model.into(),
            quantity,
            unit: unit.into(),
            origin: String::new(),
            spec: String::new(),
            arrival: String::new(),
        }
    }
}

/// 业绩一览表的一行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PerformanceRow {
    /// 项目名称
    pub project: String,
    /// 发包人/建设单位
    pub client: String,
    /// 合同金额（万元；None = 未知 → 渲染【待填】——#117：不落「0.00」假值）
    pub amount_wan: Option<f64>,
    /// 签订日期（ISO 或「2024.05」）
    pub signed: String,
    /// 完成日期
    pub completed: String,
    /// 证明材料（合同/验收单/中标通知书的 EV 编号或名称）
    pub evidence: String,
    /// 备注
    pub note: String,
}

impl PerformanceRow {
    pub fn new(
        project: impl Into<String>,
        client: impl Into<String>,
        amount_wan: Option<f64>,
        signed: impl Into<String>,
        completed: impl Into<String>,
    ) -> Self {
        Self {
            project: project.into(),
            client: client.into(),
            amount_wan,
            signed: signed.into(),
            completed: completed.into(),
            evidence: String::new(),
            note: String::new(),
        }
    }
}

/// 报价明细表的一行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PriceRow {
    /// 序号
    pub seq: usize,
    /// 项目名称/费用构成
    pub item: String,
    /// 单位
    pub unit: String,
    /// 数量
    pub quantity: f64,
    /// 单价（元）
    pub unit_price: f64,
    /// 合价（元）——`None` = 由数量×单价推算（[{`PriceRow::amount`}]）
    pub amount: Option<f64>,
    /// 备注
    pub note: String,
}

impl PriceRow {
    pub fn new(
        seq: usize,
        item: impl Into<String>,
        unit: impl Into<String>,
        quantity: f64,
        unit_price: f64,
    ) -> Self {
        Self {
            seq,
            item: item.into(),
            unit: unit.into(),
            quantity,
            unit_price,
            amount: None,
            note: String::new(),
        }
    }

    /// 合价：显式给定优先，否则数量×单价（两位小数——分位对齐）。
    pub fn amount(&self) -> f64 {
        let v = self.amount.unwrap_or(self.quantity * self.unit_price);
        (v * 100.0).round() / 100.0
    }
}
// ---------------------------------------------------------------- 表格枚举

/// 结构化表格（六种——标书正文的表格面全集）。
///
/// `Serialize`（#394/C-07）：导出幂等输入指纹需要 canonical 序列化表格面
/// ——换表格数据重导不得吃旧 manifest 缓存。仅序列化面（无 Deserialize：
/// 表格由三真源派生，不是落盘数据结构）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum BidTable {
    /// 项目概况表（项 / 值 两列直读）
    ProjectOverview { rows: Vec<(String, String)> },
    /// 技术参数偏离表 ★最重要（评委打分主要依据；负偏离整行标红）
    TechDeviation { rows: Vec<TechDeviationRow> },
    /// 人员配置表
    Personnel { rows: Vec<PersonnelRow> },
    /// 设备清单表
    Equipment { rows: Vec<EquipmentRow> },
    /// 业绩一览表
    Performance { rows: Vec<PerformanceRow> },
    /// 报价明细表（末行自动追合计行）
    PriceBreakdown { rows: Vec<PriceRow> },
}

impl BidTable {
    /// 表格种类名（落标题/日志/报告；中文——评标文档语境）。
    pub fn kind(&self) -> &'static str {
        match self {
            BidTable::ProjectOverview { .. } => "项目概况表",
            BidTable::TechDeviation { .. } => "技术参数偏离表",
            BidTable::Personnel { .. } => "人员配置表",
            BidTable::Equipment { .. } => "设备清单表",
            BidTable::Performance { .. } => "业绩一览表",
            BidTable::PriceBreakdown { .. } => "报价明细表",
        }
    }

    /// 数据行数（不含表头/合计行）——空表告警面。
    pub fn row_count(&self) -> usize {
        match self {
            BidTable::ProjectOverview { rows } => rows.len(),
            BidTable::TechDeviation { rows } => rows.len(),
            BidTable::Personnel { rows } => rows.len(),
            BidTable::Equipment { rows } => rows.len(),
            BidTable::Performance { rows } => rows.len(),
            BidTable::PriceBreakdown { rows } => rows.len(),
        }
    }

    /// 负偏离行数（★废标级告警面——非偏离表恒 0）。
    ///
    /// 供导出前自检：> 0 意味着这份标书按当前响应**不满足**招标要求，
    /// 交出去就是废标（或至少丢技术分）。
    pub fn negative_count(&self) -> usize {
        match self {
            BidTable::TechDeviation { rows } => rows
                .iter()
                .filter(|r| r.deviation == Deviation::Negative)
                .count(),
            _ => 0,
        }
    }

    /// 表头行（六表各一）。
    pub fn header(&self) -> Vec<&'static str> {
        match self {
            BidTable::ProjectOverview { .. } => vec!["项目", "内容"],
            BidTable::TechDeviation { .. } => {
                vec!["序号", "招标技术要求", "投标响应", "偏离情况", "说明"]
            }
            BidTable::Personnel { .. } => {
                vec![
                    "岗位",
                    "姓名",
                    "职称/执业资格",
                    "专业",
                    "本专业年限",
                    "备注",
                ]
            }
            BidTable::Equipment { .. } => vec![
                "设备名称",
                "型号规格",
                "数量",
                "单位",
                "产地/品牌",
                "主要性能指标",
                "进场时间",
            ],
            BidTable::Performance { .. } => vec![
                "项目名称",
                "发包人",
                "合同金额(万元)",
                "签订日期",
                "完成日期",
                "证明材料",
                "备注",
            ],
            BidTable::PriceBreakdown { .. } => {
                vec![
                    "序号",
                    "项目名称",
                    "单位",
                    "数量",
                    "单价(元)",
                    "合价(元)",
                    "备注",
                ]
            }
        }
    }

    /// 数据行（不含表头；报价表含自动合计行）。
    pub fn rows(&self) -> Vec<Vec<String>> {
        match self {
            BidTable::ProjectOverview { rows } => rows
                .iter()
                .map(|(k, v)| vec![k.clone(), v.clone()])
                .collect(),
            BidTable::TechDeviation { rows } => rows
                .iter()
                .map(|r| {
                    let cells = vec![
                        r.seq.to_string(),
                        r.requirement.clone(),
                        r.response.clone(),
                        r.deviation_label().to_string(),
                        r.note.clone(),
                    ];
                    if r.is_negative() {
                        // ★ 负偏离：整行标红（评委一眼看到风险项）
                        cells.into_iter().map(red).collect()
                    } else {
                        cells
                    }
                })
                .collect(),
            BidTable::Personnel { rows } => rows
                .iter()
                .map(|r| {
                    vec![
                        r.position.clone(),
                        r.name.clone(),
                        r.title.clone(),
                        r.major.clone(),
                        // #117：年限未知 = 【待填】（不是「0 年」——那是假值）
                        r.years
                            .map(|y| format!("{y} 年"))
                            .unwrap_or_else(|| PENDING_FILL.to_string()),
                        r.note.clone(),
                    ]
                })
                .collect(),
            BidTable::Equipment { rows } => rows
                .iter()
                .map(|r| {
                    vec![
                        r.name.clone(),
                        r.model.clone(),
                        r.quantity
                            .map(|q| q.to_string())
                            .unwrap_or_else(|| PENDING_FILL.to_string()),
                        r.unit.clone(),
                        r.origin.clone(),
                        r.spec.clone(),
                        r.arrival.clone(),
                    ]
                })
                .collect(),
            BidTable::Performance { rows } => rows
                .iter()
                .map(|r| {
                    vec![
                        r.project.clone(),
                        r.client.clone(),
                        r.amount_wan
                            .map(|v| format!("{v:.2}"))
                            .unwrap_or_else(|| PENDING_FILL.to_string()),
                        r.signed.clone(),
                        r.completed.clone(),
                        r.evidence.clone(),
                        r.note.clone(),
                    ]
                })
                .collect(),
            BidTable::PriceBreakdown { rows } => {
                let mut out: Vec<Vec<String>> = rows
                    .iter()
                    .map(|r| {
                        vec![
                            r.seq.to_string(),
                            r.item.clone(),
                            r.unit.clone(),
                            format_quantity(r.quantity),
                            format_money(r.unit_price),
                            format_money(r.amount()),
                            r.note.clone(),
                        ]
                    })
                    .collect();
                if !rows.is_empty() {
                    // 合计行：合计金额是评标的第一眼数字，不能靠评委自己加。
                    // 空串占位保持列数对齐——GFM 表格列数不齐会整表错位。
                    let total: f64 = rows.iter().map(PriceRow::amount).sum();
                    let total = (total * 100.0).round() / 100.0;
                    out.push(vec![
                        String::new(),
                        "合计".to_string(),
                        String::new(),
                        String::new(),
                        String::new(),
                        format_money(total),
                        String::new(),
                    ]);
                }
                out
            }
        }
    }

    /// 实际渲染的行（= [`Self::rows`]，语义别名——读起来更直白）。
    pub fn rows_rendered(&self) -> Vec<Vec<String>> {
        self.rows()
    }

    /// 渲染为 GFM markdown 表格（走现有导出链 → docx 真表格）。
    ///
    /// 输出形如：
    ///
    /// ```text
    /// | 序号 | 招标技术要求 | 投标响应 | 偏离情况 | 说明 |
    /// | --- | --- | --- | --- | --- |
    /// | 1 | ≥100GB | 120GB | 正偏离 |  |
    /// ```
    ///
    /// 单元格内的 `|` 转义为 `\|`（不转义会被导出链切成新列——表格
    /// 错位；GFM 的 `\|` 在纯文本阅读时仍是 `|`）。
    pub fn to_markdown(&self) -> String {
        let header = self.header();
        let mut out = String::new();
        push_md_row(
            &mut out,
            &header.iter().map(|h| md_escape(h)).collect::<Vec<_>>(),
        );
        push_md_row(&mut out, &vec!["---".to_string(); header.len()]);
        for row in self.rows() {
            let cells: Vec<String> = row.iter().map(|c| md_escape(c)).collect();
            push_md_row(&mut out, &cells);
        }
        out
    }

    /// 带标题的 markdown（粗体标题 + 空行 + 表格）——章正文里直接可用。
    ///
    /// 标题用 `**粗体段**` 而不是 `#`：正文里的章标题层级归章节树
    /// （`export.rs` 把 `#` 行渲染为加粗段、不进 TOC），表格标题只该是
    /// 段落级的强调，不该被当成章节。
    pub fn to_markdown_titled(&self) -> String {
        format!("**{}**\n\n{}", self.kind(), self.to_markdown())
    }
}

// ---------------------------------------------------------------- 私有辅助

/// 追加一行 GFM 表格（`| a | b |`）。
fn push_md_row(out: &mut String, cells: &[String]) {
    out.push('|');
    for c in cells {
        let _ = write!(out, " {c} |");
    }
    out.push('\n');
}

/// 单元格转义：`|` → `\|`，换行 → `<br>`（GFM 单元格内换行的标准表达）。
///
/// 不转义的后果是具体的：设备型号「DN100|PN16」会把一行切成两列，
/// 整张表从这一行起**全部错位**——评标现场看到的错表比没表更糟。
///
/// **红标单元格**：整格是 `{{r:...}}` 时，标记外的定界符原样保留（导出
/// 层要认得出来），标记**体内**的 `|` 才做 GFM 转义（那是真内容）。
/// 因为分隔符是 `:`，标记本身永远不会被列切分误伤。
fn md_escape(text: &str) -> String {
    let t = text.trim();
    if let Some(inner) = t
        .strip_prefix(RED_MARK_OPEN)
        .and_then(|r| r.strip_suffix(RED_MARK_CLOSE))
    {
        // 完整红标格子：原样输出（内部若是 `<br>`/转义需求由调用方保证——
        // red() 的输入来自本模块自己的行结构，不含裸 `|`）
        return format!("{}{}{}", RED_MARK_OPEN, escape_plain(inner), RED_MARK_CLOSE);
    }
    escape_plain(t)
}

/// 普通文本段的转义：`|` → `\|`，换行 → `<br>`。
fn escape_plain(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '|' => {
                // 已经是 `\|` 的不重复加反斜杠（幂等）
                if !s.ends_with('\\') {
                    s.push('\\');
                }
                s.push('|');
            }
            '\r' => {}
            '\n' => s.push_str("<br>"),
            c => s.push(c),
        }
    }
    s
}

/// 金额格式：两位小数 + 千分位（1250000.00 → 「1,250,000.00」）。
fn format_money(v: f64) -> String {
    let neg = v < 0.0;
    let cents = (v.abs() * 100.0).round() as u128;
    let int_part = cents / 100;
    let frac = cents % 100;
    let digits = int_part.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!("{}{grouped}.{frac:02}", if neg { "-" } else { "" })
}

/// 数量格式：整数去小数点（3.0 → 「3」），非整保留两位（1.5 → 「1.50」）。
fn format_quantity(v: f64) -> String {
    if (v.fract()).abs() < f64::EPSILON {
        format!("{}", v as i128)
    } else {
        format!("{v:.2}")
    }
}
// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tech_deviation_markdown_structure_and_red_negative() {
        let t = BidTable::TechDeviation {
            rows: vec![
                TechDeviationRow::new(1, "≥100GB", "120GB", Deviation::Positive),
                TechDeviationRow::new(2, "不低于 3 年", "2年", Deviation::Negative),
                TechDeviationRow::new(3, "详见白皮书", "满足", Deviation::None),
            ],
        };
        let md = t.to_markdown();
        let lines: Vec<&str> = md.lines().collect();
        assert_eq!(
            lines[0], "| 序号 | 招标技术要求 | 投标响应 | 偏离情况 | 说明 |",
            "表头齐备"
        );
        assert!(lines[1].starts_with("| --- |"), "分隔行在位: {}", lines[1]);
        assert_eq!(lines.len(), 5, "表头 + 分隔 + 3 行数据");
        // 负偏离整行标红：负偏离行的每个单元格都被 `{{r|...}}` 包住
        for cell in ["2", "不低于 3 年", "2年", "负偏离"] {
            let marked = red(cell);
            assert!(
                lines[3].contains(marked.as_str()),
                "负偏离单元格 {cell} 应标红: {}",
                lines[3]
            );
        }
        // 标红单元格计数 = 5（整行 5 格，说明列为空也照样占一格）
        assert_eq!(
            lines[3].matches(RED_MARK_OPEN).count(),
            5,
            "负偏离整行 5 格全标红: {}",
            lines[3]
        );
        // 正偏离/无偏离不标红
        assert!(
            !lines[2].contains(RED_MARK_OPEN),
            "正偏离不标红: {}",
            lines[2]
        );
        assert!(
            !lines[4].contains(RED_MARK_OPEN),
            "无偏离不标红: {}",
            lines[4]
        );
        assert_eq!(t.negative_count(), 1);
    }

    /// 数未转义的竖线（`\|` 是单元格内容，不是分隔符）。
    ///
    /// 为什么不用 `matches(" | ")`：红色单元格里的 `" | "` 是内容的一部分，
    /// 会把列数数多——断言本身必须先有正确口径，否则测的是错东西。
    fn unescaped_pipes(line: &str) -> usize {
        let bytes = line.as_bytes();
        line.match_indices('|')
            .filter(|(i, _)| *i == 0 || bytes.get(i - 1) != Some(&b'\\'))
            .count()
    }

    #[test]
    fn cell_pipes_are_escaped_so_table_does_not_shift() {
        let t = BidTable::Equipment {
            rows: vec![EquipmentRow::new("闸阀|球阀", "DN100|PN16", Some(3), "台")],
        };
        let md = t.to_markdown();
        let data_line = md.lines().nth(2).unwrap();
        assert!(data_line.contains(r"闸阀\|球阀"), "竖线转义: {data_line}");
        // 转义后每行列数仍 = 表头列数（7）——切列不会错位
        let cols = unescaped_pipes(data_line) - 1;
        assert_eq!(cols, 7, "列数对齐: {data_line}");
    }

    #[test]
    fn price_breakdown_auto_total_and_formatting() {
        let t = BidTable::PriceBreakdown {
            rows: vec![
                PriceRow::new(1, "设备采购", "批", 1.0, 1_200_000.0),
                PriceRow::new(2, "安装调试", "项", 1.0, 50_000.5),
            ],
        };
        let md = t.to_markdown();
        assert!(md.contains("1,200,000.00"), "千分位金额: {md}");
        assert!(md.contains("50,000.50"), "两位小数: {md}");
        assert!(md.contains("1,250,000.50"), "合计行自动求和: {md}");
        assert_eq!(t.rows().len(), 3, "2 行明细 + 1 行合计");
        assert_eq!(t.row_count(), 2, "row_count 不含合计行");
    }

    #[test]
    fn all_six_kinds_render_well_formed_markdown() {
        let tables = vec![
            BidTable::ProjectOverview {
                rows: vec![
                    ("项目名称".into(), "某水库除险加固工程".into()),
                    ("招标编号".into(), "ZFCG-2026-001".into()),
                ],
            },
            BidTable::TechDeviation {
                rows: vec![TechDeviationRow::new(1, "≥100GB", "100GB", Deviation::None)],
            },
            BidTable::Personnel {
                rows: vec![PersonnelRow::new(
                    "项目经理",
                    "张三",
                    "高级工程师/一级建造师",
                    "水利水电",
                    Some(12),
                )],
            },
            BidTable::Equipment {
                rows: vec![EquipmentRow::new("挖掘机", "PC200", Some(2), "台")],
            },
            BidTable::Performance {
                rows: vec![PerformanceRow::new(
                    "甲水库除险加固",
                    "某水务局",
                    Some(3200.0),
                    "2023-03",
                    "2024-06",
                )],
            },
            BidTable::PriceBreakdown {
                rows: vec![PriceRow::new(1, "建筑工程", "项", 1.0, 8_000_000.0)],
            },
        ];
        for t in &tables {
            let md = t.to_markdown();
            let lines: Vec<&str> = md.lines().collect();
            let cols = t.header().len();
            assert!(lines.len() >= 3, "{} 至少 表头+分隔+1 行", t.kind());
            for (i, line) in lines.iter().enumerate() {
                assert!(
                    line.starts_with('|') && line.ends_with('|'),
                    "{} 第 {i} 行不是表格行: {line}",
                    t.kind()
                );
                // 未转义竖线数 = 列数 + 1（边界两条 + 列间 cols-1 条）
                let unescaped = unescaped_pipes(line);
                assert_eq!(
                    unescaped,
                    cols + 1,
                    "{} 第 {i} 行列数（应 {}）: {line}",
                    t.kind(),
                    cols
                );
            }
            assert!(!t.kind().is_empty());
            let titled = t.to_markdown_titled();
            assert!(titled.starts_with("**"), "带标题版以粗体标题起");
            assert!(titled.contains(&md));
        }
    }

    #[test]
    fn derivation_from_param_reuses_domain_truth() {
        // 偏离判定不重算：走 bid_domain::param::derive_deviation 的同一真源
        let r = TechDeviationRow::from_param(1, "存储容量", "≥100GB", "60GB");
        assert_eq!(r.deviation, Deviation::Negative);
        assert!(r.requirement.contains("存储容量"));
        assert_eq!(r.deviation_label(), "负偏离");
        let ok = TechDeviationRow::from_param(2, "质保年限", "不低于 3 年", "5年");
        assert_eq!(ok.deviation, Deviation::Positive);
    }

    #[test]
    fn money_and_quantity_formatting() {
        assert_eq!(format_money(1_250_000.0), "1,250,000.00");
        assert_eq!(format_money(999.5), "999.50");
        assert_eq!(format_money(0.0), "0.00");
        assert_eq!(format_money(-1234.5), "-1,234.50");
        assert_eq!(format_quantity(3.0), "3");
        assert_eq!(format_quantity(1.5), "1.50");
    }

    /// #117：数值未知（None）→ 【待填】——不落「0 年」「0.00」这类假值
    /// （评标现场读到假值比读到待填更糟：前者是错误响应，后者是待补录）。
    #[test]
    fn unknown_numerics_render_pending_fill_not_fake_zero() {
        let t = BidTable::Personnel {
            rows: vec![PersonnelRow::new(
                "项目经理",
                "张三",
                "高级工程师",
                "机电",
                None,
            )],
        };
        let md = t.to_markdown();
        assert!(md.contains("【待填】"), "未知年限落待填: {md}");
        assert!(!md.contains("0 年"), "绝不落「0 年」假值: {md}");

        let e = BidTable::Equipment {
            rows: vec![EquipmentRow::new("物联网关", "IoT-GW-3000", None, "台")],
        };
        let md = e.to_markdown();
        assert!(md.contains("【待填】"), "未知数量落待填: {md}");

        let p = BidTable::Performance {
            rows: vec![PerformanceRow::new(
                "某医院项目",
                "某医院",
                None,
                "2023-05",
                "2024-08",
            )],
        };
        let md = p.to_markdown();
        assert!(md.contains("【待填】"), "未知金额落待填: {md}");
        assert!(!md.contains("0.00"), "绝不落「0.00」假值: {md}");
        // 列数不受待填影响（仍是 7 列——错位比缺值更糟）
        let data_line = md.lines().nth(2).unwrap();
        assert_eq!(unescaped_pipes(data_line) - 1, 7, "列数对齐: {data_line}");
    }
}
