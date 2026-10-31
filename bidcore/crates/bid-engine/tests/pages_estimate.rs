//! #110 任务 2：分页预估接线测试（`pages.rs` 门面）。
//!
//! 锚点纪律（与 `mapping_docx.rs` 同族）：期望值**手算钉定**（公式 =
//! `mdcore::pagination` 模块文档的模型 v2），不抄实现输出——引擎 bump
//! 或映射回归的防回归锚。手算底稿（bid_baseline_params）：
//!
//! ```text
//! text_width   = round(210mm×1440/25.4) − round(28mm×…) − round(26mm×…)
//!              = 11906 − 1587 − 1474 = 8845 twips
//! text_height  = 16838 − 1984 − 1701 = 13153 twips
//! body_size    = 12pt × 20 = 240 twips（小四）
//! chars_per_line = floor(8845 / 240) = 36（全角位）
//! lh_body      = 240 × (0.3115 + 1.15×1.5) = 240 × 2.0365 = 488.76
//! lines_per_page = floor(13153 / 488.76) = 26
//! ```
//!
//! 引擎语义面（`markdown_to_tree` v0 规则）：**一行一块**（paragraph/
//! heading/…）——测试文本按行构造，行数即段数。

use bid_engine::{bid_baseline_params, check_page_limit, estimate_pages, params_from_geometry};

/// 生成 n 行、每行恰好 `chars` 个全角字的文本（CJK = 1 全角单位/字）。
fn cjk_lines(n: usize, chars: usize) -> String {
    let unit = "施"; // CJK 统一表意
    (0..n)
        .map(|_| unit.repeat(chars))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// 入参换算（format_spec 单位制 → 引擎域 twips）
// ---------------------------------------------------------------------------

/// 基线参数换算锚点（A4 / 35-30-28-26mm / 小四 12pt / 1.5 倍——镜像
/// `bid-tools::format_spec::baseline` 的数值面）。
#[test]
fn bid_baseline_params_conversion_anchor() {
    let p = bid_baseline_params();
    assert_eq!(p.text_width, 8845, "版心宽 = 11906−1587−1474");
    assert_eq!(p.text_height, 13153, "版心高 = 16838−1984−1701");
    assert_eq!(p.body_size_twips, 240, "小四 12pt = 240 twips");
    assert_eq!(p.line_multiple, 1.5);
    assert_eq!(p.spacing_before, 0, "标书导出链不设段前距");
    assert_eq!(p.spacing_after, 0);
}

/// 几何换算的独立性：Letter 纸 + 不同边距/字号走同一换算式。
#[test]
fn params_from_geometry_letter_paper() {
    let p = params_from_geometry(216, 279, 25, 25, 20, 20, 10.5, 1.0);
    // 216mm = 12246（round(12245.67)）、20mm = 1134（round(1133.86)）×2
    assert_eq!(p.text_width, 12246 - 1134 - 1134);
    // 279mm = 15817（round(15817.32)）、25mm = 1417（round(1417.32)）×2
    assert_eq!(p.text_height, 15817 - 1417 - 1417);
    assert_eq!(p.body_size_twips, 210, "五号 10.5pt = 210 twips");
    assert_eq!(p.line_multiple, 1.0);
}

// ---------------------------------------------------------------------------
// 估算锚点（手算钉定——引擎模型 v2）
// ---------------------------------------------------------------------------

/// 100 段 × 恰每行 36 全角字（= 每段 1 行）→ 100 行 × 488.76 / 13153
/// = 3.716 → ceil = **4 页**；纯段落误差带 5%。
#[test]
fn estimate_pages_hand_computed_anchor() {
    let est = estimate_pages(&cjk_lines(100, 36), &bid_baseline_params());
    assert_eq!(est.chars_per_line, 36);
    assert_eq!(est.lines_per_page, 26);
    assert_eq!(est.paragraph_lines, 100, "每段恰 1 行");
    assert_eq!(est.heading_lines, 0);
    assert_eq!(est.table_lines, 0);
    assert_eq!(est.code_lines, 0);
    assert_eq!(est.pages, 4, "ceil(100×488.76 / 13153) = ceil(3.716)");
    assert_eq!(est.error_margin_pct, 5.0, "纯段落档");
}

/// 页边界对偶：26 行恰满 1 页 / 27 行溢出到 2 页（lines_per_page 锚的
/// 直接消费语义——「再写一行就翻页」）。
#[test]
fn estimate_pages_page_boundary_dual() {
    let p = bid_baseline_params();
    let at = estimate_pages(&cjk_lines(26, 36), &p);
    assert_eq!(at.pages, 1, "26 行 = floor(13153/488.76) 恰一页");
    let over = estimate_pages(&cjk_lines(27, 36), &p);
    assert_eq!(over.pages, 2, "27 行溢出");
}

/// 折行语义：72 全角字一段 = 2 行（不是 1 行——宽度按全角单位折行）。
#[test]
fn estimate_pages_wraps_wide_paragraph() {
    let est = estimate_pages(&cjk_lines(1, 72), &bid_baseline_params());
    assert_eq!(est.paragraph_lines, 2, "72 字 / 36 每行 = ceil(2.0) = 2 行");
    assert_eq!(est.pages, 1);
}

/// 空文本 / 极小文本：页数下限 1（`max(1)` 防御——空标书不是 0 页）。
#[test]
fn estimate_pages_floor_is_one() {
    let p = bid_baseline_params();
    assert_eq!(estimate_pages("", &p).pages, 1);
    assert_eq!(estimate_pages("\n\n\n", &p).pages, 1, "空行不产块");
    let one = estimate_pages("资格声明", &p);
    assert_eq!(one.pages, 1);
    assert_eq!(one.paragraph_lines, 1);
}

/// markdown 结构分型计数：标题/表格/代码各入各的账；重结构（表格行
/// 占比 > 20%）误差带升到 10%。
#[test]
fn estimate_pages_counts_markdown_structures() {
    let md = [
        "# 施工组织设计",
        "",
        "正文一段话说明总体安排与资源投入。",
        "",
        "|项目|数值|说明|",
        "|---|---|---|",
        "|工期|365日历天|自开工令下达之日起算|",
        "|质量|合格|符合国家现行验收标准|",
        "|安全|零事故|杜绝重大安全事故|",
        "",
        "```",
        "计划工期：365 日历天",
        "开工日期：以监理开工令为准",
        "```",
    ]
    .join("\n");
    let est = estimate_pages(&md, &bid_baseline_params());
    assert_eq!(est.heading_lines, 1, "# 一级标题 = 1 行");
    assert_eq!(est.paragraph_lines, 1, "一段正文（17 字 ≤ 36/行）= 1 行");
    // 表格：4 数据行（分隔行剔除）；首列 2 字、末列 ≤10 字，3 列
    // cell_cpl = floor((8845/3−216)/240) = floor(11.38) = 11 → 各 1 行。
    assert_eq!(est.table_lines, 4, "4 数据行 × 1 行（分隔行剔除）");
    assert_eq!(est.code_lines, 2, "代码块字面 2 行");
    assert_eq!(
        est.error_margin_pct, 10.0,
        "结构行 6 / 总行 8 = 75% > 20% → 重结构档"
    );
}

// ---------------------------------------------------------------------------
// 页数限制裁决（三态：放行 / 边界 / 确定超限）
// ---------------------------------------------------------------------------

/// 三态对偶：同一段文本，限制收紧 → 放行 → 边界 → 确定超限。
#[test]
fn check_page_limit_three_states() {
    let p = bid_baseline_params();
    let md = cjk_lines(100, 36); // 4 页（点估计）

    // 限制 10 页：点估计 4 ≤ 10，误差上界 ceil(4×1.05)=5 ≤ 10 → 干净放行。
    let ok = check_page_limit(&md, 10, &p);
    assert!(!ok.over_limit);
    assert!(!ok.definitely_over);
    assert!(!ok.within_margin, "上界 5 页 ≤ 限 10——非边界态");
    assert_eq!(ok.headroom_pages, 6);

    // 限制 4 页：点估计 4 ≤ 4 未超；上界 ceil(4×1.05) = 5 > 4 → 边界态。
    let edge = check_page_limit(&md, 4, &p);
    assert!(!edge.over_limit, "点估计恰等于限——未超");
    assert!(edge.within_margin, "误差带上界 5 > 4——建议人工复核");
    assert!(!edge.definitely_over);
    assert_eq!(edge.headroom_pages, 0);

    // 限制 3 页：点估计 4 > 3 超限；下界 ceil(4×0.95) = 4 > 3 → 确定超。
    let over = check_page_limit(&md, 3, &p);
    assert!(over.over_limit);
    assert!(over.definitely_over, "误差带下界 4 > 3——确定超限");
    assert!(!over.within_margin);
    assert_eq!(over.headroom_pages, -1, "超出 1 页（负余量）");
}

/// 大文本超限（页数限制用例的实感面）：600 段 ≈ 23 页，限 20 页 → 超。
#[test]
fn check_page_limit_large_document_over() {
    let p = bid_baseline_params();
    let md = cjk_lines(600, 36);
    let v = check_page_limit(&md, 20, &p);
    // 600 行 × 488.76 / 13153 = 22.30 → 23 页
    assert_eq!(v.estimate.pages, 23);
    assert!(v.over_limit);
    // 下界 ceil(23×0.95) = 22 > 20 → 确定超
    assert!(v.definitely_over);
    assert_eq!(v.headroom_pages, -3);
}
