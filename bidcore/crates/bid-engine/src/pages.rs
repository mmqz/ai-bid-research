//! pages.rs —— #110 任务 2：分页预估接线（`mdcore::pagination` 门面）。
//!
//! ## 用例（issue #110 原文）
//!
//! 「pagination → 估算页数（投标文件页数限制）」——招标文件常规定
//! 「投标文件不超过 N 页 / N 字」，超页 = 形式评审扣分甚至废标。导出前
//! 用引擎分页模型预估页数，超限即预警（人在回路），而不是印出来才知道。
//!
//! ## 引擎语义（只调用，零自研——issue #2 §1 约束 2）
//!
//! [`mdcore::pagination`] 是引擎侧分页预估的**单一真源**（N-5：块树持有
//! 全部结构真相，页数口径钉在核心）。本模块只做三件事：
//! 1. **入参换算**：标书面排版参数（毫米/磅——`format_spec.rs` 的单位制）
//!    → 引擎域 [`PaginationParams`]（twips）；
//! 2. **文本入树**：markdown（或纯文本——纯文本 = 全段落树）经引擎
//!    [`mdcore::markdown::markdown_to_tree`] 入块树；
//! 3. **调用**：[`mdcore::pagination::estimate`] → [`PageEstimate`] 镜像。
//!
//! ## 精度口径（引擎校准面，诚实登记）
//!
//! 引擎校准对象 = mdcore 自家导出链（LO 25.2.3.2 + Noto CJK 实测）：
//! 40 万字档误差缺省 +2.7% / **中文标书档 -1.1%**（A4 + 小四 + 1.5 倍
//! 行距——与本模块 [`bid_baseline_params`] 同构）。标书导出链（bid-tools
//! `export.rs`，docx-rs 直写）与引擎导出链**不同实现**，同参数下版心/
//! 行高模型一致（docx 同一 OOXML 语义面），残差由 [`PageEstimate::
//! error_margin_pct`] 自报（纯段落 ≈±5%；重表格/代码档 ≈±10%）——消费方
//! 按「预估 + 误差带」做软预警，不做硬裁剪。
//!
//! ## 消费方（下一轮接线登记）
//!
//! 本轮交付 bid-engine 门面（工作范围 = `bid-engine/**`）；`export.rs`
//! 的导出前预警接线在 bid-tables 冻结面解除后跟进（PR 登记）。

use mdcore::markdown::markdown_to_tree;
use mdcore::pagination::{PageEstimate as EnginePageEstimate, PaginationParams};

/// 页数预估结果（引擎 [`EnginePageEstimate`] 的标书面镜像——引擎类型
/// 不出 bid-engine 边界，与 `types.rs` 镜像纪律同源）。
///
/// 字段语义见引擎模块文档（`mdcore::pagination::PageEstimate`）；
/// `error_margin_pct` 是**模型自报误差带**（±%），消费方软预警用。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BidPageEstimate {
    /// 估算页数（向上取整，最小 1）。
    pub pages: u32,
    /// 正文每行全角字数。
    pub chars_per_line: u32,
    /// 每页行数。
    pub lines_per_page: u32,
    /// 段落行消耗（含列表/引用内段落）。
    pub paragraph_lines: u32,
    /// 标题行消耗（当前链标题按正文行计）。
    pub heading_lines: u32,
    /// 表格行消耗。
    pub table_lines: u32,
    /// 代码块行消耗。
    pub code_lines: u32,
    /// 模型自报误差带（±%——纯段落 ≈5；重结构 ≈10）。
    pub error_margin_pct: f64,
}

impl From<EnginePageEstimate> for BidPageEstimate {
    fn from(e: EnginePageEstimate) -> Self {
        Self {
            pages: e.pages,
            chars_per_line: e.chars_per_line,
            lines_per_page: e.lines_per_page,
            paragraph_lines: e.paragraph_lines,
            heading_lines: e.heading_lines,
            table_lines: e.table_lines,
            code_lines: e.code_lines,
            error_margin_pct: e.error_margin_pct,
        }
    }
}

/// 毫米 → twips（1 mm = 1440/25.4 twips，四舍五入——`format_spec.rs` 同式）。
const MM_TO_TWIPS: f64 = 1440.0 / 25.4;

/// 磅 → twips（1 pt = 20 twips）。
const PT_TO_TWIPS: f64 = 20.0;

/// 标书工程惯例基线（水利口径——镜像 `bid-tools::format_spec::baseline`：
/// A4 / 35-30-28-26 mm / 小四 12pt / 1.5 倍行距）。
///
/// 两处基线的**镜像关系**是显式契约：bid-engine 不依赖 bid-tools
/// （依赖方向 bid-tools → bid-engine，反向 = 环），`format_spec.rs`
/// 的 baseline 改版时本函数须同步（跨 crate 等值断言登记在 PR 跟进项
/// ——本轮工作范围限 bid-engine/**）。引擎 N-5 校准的「中文标书档」
/// 与此参数同构（实测误差 -1.1%）。
#[must_use]
pub fn bid_baseline_params() -> PaginationParams {
    params_from_geometry(210, 297, 35, 30, 28, 26, 12.0, 1.5)
}

/// 排版几何 → 引擎分页参数（标书面单位制 → 引擎域 twips 的唯一换算点）。
///
/// - 页面与边距：毫米（`format_spec.rs` 的 `PageSpec`/`MarginSpec` 单位）；
/// - 字号：磅（`FontSpec::size` 单位——引擎 `body_size_twips` = 半点×10 = 磅×20）；
/// - 行距：倍数（`FontSpec::line_height` 单位）；
/// - 段前/段后距：标书导出链不设段距（`format_spec` 无此项——Word 缺省 0），
///   固定 0（引擎缺省档同值——不加不存在的排版面）。
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn params_from_geometry(
    page_w_mm: u32,
    page_h_mm: u32,
    margin_top_mm: u32,
    margin_bottom_mm: u32,
    margin_left_mm: u32,
    margin_right_mm: u32,
    body_size_pt: f64,
    line_multiple: f64,
) -> PaginationParams {
    PaginationParams {
        text_width: (page_w_mm as f64 * MM_TO_TWIPS).round() as u32
            - (margin_left_mm as f64 * MM_TO_TWIPS).round() as u32
            - (margin_right_mm as f64 * MM_TO_TWIPS).round() as u32,
        text_height: (page_h_mm as f64 * MM_TO_TWIPS).round() as u32
            - (margin_top_mm as f64 * MM_TO_TWIPS).round() as u32
            - (margin_bottom_mm as f64 * MM_TO_TWIPS).round() as u32,
        body_size_twips: (body_size_pt * PT_TO_TWIPS).round() as u32,
        line_multiple,
        spacing_before: 0,
        spacing_after: 0,
    }
}

/// markdown（或纯文本）→ 页数预估（引擎门面：入树 + 估算）。
///
/// 纯文本 = 全段落树（无 markdown 语法即无标题/表格块）；章节起草产物
/// 拼接为 markdown 时标题/表格/列表进对应块型（引擎 `markdown` 模块
/// 语义——本模块零自研解析）。
#[must_use]
pub fn estimate_pages(md: &str, params: &PaginationParams) -> BidPageEstimate {
    let tree = markdown_to_tree(md);
    mdcore::pagination::estimate(&tree, params).into()
}

/// 页数限制裁决（「投标文件不超过 N 页」的软预警面）。
///
/// 设计：**软预警不硬裁**——预估带误差（`error_margin_pct` 自报），
/// `over_limit` 按点估计判；`at_risk` = 误差带下界也超限（确定超），
/// `within_margin` = 点估计未超但误差带上界超限（边界态，建议人工复核）。
/// 消费方按三态决定「放行 / 预警 / 拦截」——拦截策略是调用方语义，
/// 本层只给诚实数字。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageLimitVerdict {
    /// 页数预估（点估计 + 行构成明细）。
    pub estimate: BidPageEstimate,
    /// 限制页数（招标文件规定值）。
    pub limit_pages: u32,
    /// 点估计超限（true = 预估页数 > 限制）。
    pub over_limit: bool,
    /// **确定超限**：误差带下界（最小可能页数）也超限。
    pub definitely_over: bool,
    /// **边界态**：点估计未超，但误差带上界超限（建议人工复核）。
    pub within_margin: bool,
    /// 剩余页数余量（点估计口径；负值 = 超出页数）。
    pub headroom_pages: i64,
}

/// 页数限制检查（`estimate_pages` + 三态裁决，见 [`PageLimitVerdict`]）。
#[must_use]
pub fn check_page_limit(md: &str, limit_pages: u32, params: &PaginationParams) -> PageLimitVerdict {
    let estimate = estimate_pages(md, params);
    // 误差带换算到页数（下界 = 页数下限；至少 1 页）。
    let margin = estimate.error_margin_pct.max(0.0) / 100.0;
    let lower = ((estimate.pages as f64) * (1.0 - margin)).ceil().max(1.0) as u32;
    let upper = ((estimate.pages as f64) * (1.0 + margin)).ceil() as u32;
    let over_limit = estimate.pages > limit_pages;
    PageLimitVerdict {
        definitely_over: over_limit && lower > limit_pages,
        within_margin: !over_limit && upper > limit_pages,
        over_limit,
        headroom_pages: limit_pages as i64 - estimate.pages as i64,
        estimate,
        limit_pages,
    }
}
