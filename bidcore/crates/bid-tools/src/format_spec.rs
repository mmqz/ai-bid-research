//! format_spec.rs —— #59 阶段 4（提前到 export 前）：`.mang/format-spec.json`
//! 的解析 / 验证 / 从招标文件提取 / docx 应用桥。
//!
//! Lead 铁律：**不准硬编码格式**——格式必须从招标文件读（每份标书要求不同）。
//! 链路：
//! ```text
//! 招标文件里的格式条款（「正文用仿宋三号，行距 1.5 倍」）
//!     ↓ derive_from_requirements（确定性模式匹配——候选面是 extract 的
//!       「格式要求」类 + 格式字眼兜底）
//! FormatSpec（.mang/format-spec.json 的 serde 面）
//!     ↓ validate（页面枚举 / 字号行距边距的合理域——防手写 spec 打出怪物文档）
//! apply_to_docx（docx-rs 面：页面 / Normal 样式字体行距 / 页眉页脚）
//! ```
//!
//! 降级面：招标文件无格式条款（或 spec 文件缺失）→ **工程惯例基线**
//! [`baseline`]（A4 / 35-30-28-26mm / 仿宋_GB2312 小四 / 1.5 倍行距——水利
//! 标书主流口径）+ warning「按基线导出」。降级不伪装成提取成功。
//!
//! 单位制（docx XML 的物理单位——集中一处，全模块唯一真源）：
//! - 页面与边距：twips（1/20 pt；1 mm = 1440/25.4 ≈ 56.6929 twips）；
//! - 字号：半点 half-points（1 pt = 2 hp）；
//! - 行距（`w:spacing w:line` + `lineRule=auto`）：240 = 单倍行距——倍数 × 240。
//!
//! per_section 覆盖面：key 是章节标题的**包含匹配**（标题含 key 即命中；
//! BTreeMap 键序确定——同命中时字典序首个生效，跨进程可重放）。覆盖面只
//! 允许字体三件套（加性位）；页面/页眉页脚是文档级全局位——Word 单节内
//! 混排页面方向是排版事故，不做。

use std::collections::BTreeMap;

use bid_domain::Requirement;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- serde 面

/// 格式规格（`.mang/format-spec.json` 顶层）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatSpec {
    pub global: GlobalFormat,
    /// 章节级字体覆盖（key = 章节标题包含匹配；值结构对齐 Lead #59 样例
    /// 的 `{ "font": {...} }` 包裹面——加性位，未覆盖项继承 global）。
    #[serde(default)]
    pub per_section: BTreeMap<String, SectionOverride>,
}

/// 章节级覆盖（包裹面——Lead 样例结构；未来可加性扩 heading 等位）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SectionOverride {
    #[serde(default)]
    pub font: Option<FontSpec>,
}

/// 文档级全局格式。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalFormat {
    pub page: PageSpec,
    pub font: FontSpec,
    pub header: HeaderFooterSpec,
    pub footer: HeaderFooterSpec,
}

/// 页面规格。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageSpec {
    /// 纸张尺寸枚举："A4" | "A3" | "B5" | "Letter"（validate 把关）。
    pub size: String,
    /// 页边距（毫米——人写面；twips 换算在 [`PageSpec::size_twips`] 等）。
    pub margin: MarginSpec,
}

/// 页边距（毫米）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MarginSpec {
    pub top: u32,
    pub bottom: u32,
    pub left: u32,
    pub right: u32,
}

/// 字体三件套（正文/标题共用结构——覆盖面最小充分量）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSpec {
    /// 中文字体名（如「仿宋_GB2312」「方正小标宋简体」）。
    pub body: String,
    /// 字号（pt——人写面；半点换算在 [`FontSpec::size_half_points`]）。
    pub size: f64,
    /// 行距（倍数，如 1.5；240 单位换算在 [`FontSpec::line_units`]）。
    /// serde 驼峰别名（Lead #59 样例用 `lineHeight`——手写 spec 的真实用户面）。
    #[serde(alias = "lineHeight")]
    pub line_height: f64,
}

/// 页眉/页脚规格。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeaderFooterSpec {
    pub enabled: bool,
    /// 内容模板：`{project_name}` / `{page}` / `{pages}` 占位符（应用时替换）。
    pub content: String,
}

// ---------------------------------------------------------------- 基线与单位换算

/// 纸张尺寸（毫米）——validate 枚举的真源。
#[must_use]
pub fn paper_mm(size: &str) -> Option<(u32, u32)> {
    match size {
        "A4" => Some((210, 297)),
        "A3" => Some((297, 420)),
        "B5" => Some((176, 250)),
        "Letter" => Some((216, 279)),
        _ => None,
    }
}

/// 工程惯例基线（水利标书主流口径——降级面与测试基线）。
#[must_use]
pub fn baseline() -> FormatSpec {
    FormatSpec {
        global: GlobalFormat {
            page: PageSpec {
                size: "A4".into(),
                margin: MarginSpec { top: 35, bottom: 30, left: 28, right: 26 },
            },
            font: FontSpec { body: "仿宋_GB2312".into(), size: 12.0, line_height: 1.5 },
            header: HeaderFooterSpec { enabled: true, content: "{project_name}".into() },
            footer: HeaderFooterSpec {
                enabled: true,
                content: "第 {page} 页 共 {pages} 页".into(),
            },
        },
        per_section: BTreeMap::new(),
    }
}

/// 毫米 → twips（1 mm = 1440/25.4 twips）。
#[must_use]
pub fn mm_to_twips(mm: u32) -> u32 {
    (f64::from(mm) * 1440.0 / 25.4).round() as u32
}

/// pt → 半点（docx `w:sz` 口径）。
#[must_use]
pub fn pt_to_half_points(pt: f64) -> u32 {
    (pt * 2.0).round() as u32
}

/// 行距倍数 → `w:spacing w:line`（lineRule=auto：240 = 单倍）。
#[must_use]
pub fn line_multiple_to_units(mult: f64) -> i32 {
    (mult * 240.0).round() as i32
}

impl PageSpec {
    /// 页面尺寸（twips）。
    #[must_use]
    pub fn size_twips(&self) -> (u32, u32) {
        let (w, h) = paper_mm(&self.size).unwrap_or((210, 297)); // validate 已把关——防御回退 A4
        (mm_to_twips(w), mm_to_twips(h))
    }
}

impl FormatSpec {
    /// 章节字体解析：per_section 的包含匹配（标题含 key 即命中）→ 覆盖值与
    /// global 的逐字段合并（覆盖面只动显式位——serde 无「部分 FontSpec」，
    /// 以 `SectionOverride.font` 整体在场为粒度；标题无命中 → global 原样）。
    ///
    /// 同标题多 key 命中时字典序首个生效（BTreeMap 迭代序——跨进程确定）。
    #[must_use]
    pub fn font_for_section(&self, title: &str) -> FontSpec {
        for (key, over) in &self.per_section {
            if let Some(f) = &over.font {
                if title.contains(key.as_str()) {
                    return f.clone();
                }
            }
        }
        self.global.font.clone()
    }
}

impl FontSpec {
    /// 字号（半点）。
    #[must_use]
    pub fn size_half_points(&self) -> u32 {
        pt_to_half_points(self.size)
    }

    /// 行距（w:line 单位）。
    #[must_use]
    pub fn line_units(&self) -> i32 {
        line_multiple_to_units(self.line_height)
    }
}

// ---------------------------------------------------------------- 验证

/// 规格域校验（手写 spec / 提取结果共用——防打出怪物文档）。
///
/// - 纸张必须在 [`paper_mm`] 枚举内；
/// - 字号 5.0–42.0 pt（初号 42 封顶）；
/// - 行距 1.0–3.0 倍；
/// - 边距 0–100 mm；
/// - 字体名非空且 ≤ 32 字符（Word 字体名长度域）。
pub fn validate(spec: &FormatSpec) -> Result<(), String> {
    let g = &spec.global;
    if paper_mm(&g.page.size).is_none() {
        return Err(format!("纸张尺寸 {} 不在枚举（A4/A3/B5/Letter）内", g.page.size));
    }
    if !(5.0..=42.0).contains(&g.font.size) {
        return Err(format!("正文字号 {} pt 越域（5.0–42.0）", g.font.size));
    }
    if !(1.0..=3.0).contains(&g.font.line_height) {
        return Err(format!("正文行距 {} 倍越域（1.0–3.0）", g.font.line_height));
    }
    if g.font.body.trim().is_empty() || g.font.body.chars().count() > 32 {
        return Err(format!("正文字体名 {:?} 非法（非空且 ≤32 字符）", g.font.body));
    }
    let m = &g.page.margin;
    for (name, v) in [("上", m.top), ("下", m.bottom), ("左", m.left), ("右", m.right)] {
        if v > 100 {
            return Err(format!("{name}边距 {v} mm 越域（0–100）"));
        }
    }
    for (key, over) in &spec.per_section {
        if let Some(f) = &over.font {
            if !(5.0..=42.0).contains(&f.size) {
                return Err(format!("per_section[{key}] 字号 {} pt 越域", f.size));
            }
            if f.body.trim().is_empty() {
                return Err(format!("per_section[{key}] 字体名为空"));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 从招标文件提取

/// 中文字号 → pt（GB/T 字号口径——提取面与手工核对共用的唯一真源）。
#[must_use]
pub fn chinese_font_size_pt(name: &str) -> Option<f64> {
    let pt = match name {
        "初号" => 42.0,
        "小初" => 36.0,
        "一号" => 26.0,
        "小一" => 24.0,
        "二号" => 22.0,
        "小二" => 18.0,
        "三号" => 16.0,
        "小三" => 15.0,
        "四号" => 14.0,
        "小四" => 12.0,
        "五号" => 10.5,
        "小五" => 9.0,
        "六号" => 7.5,
        "小六" => 6.5,
        _ => return None,
    };
    Some(pt)
}

/// 从格式条款推导规格（Lead 铁律的落地面——**不硬编码，读招标文件**）。
///
/// 候选面：`category == "格式要求"` 的 REQ 摘录 + 全类别「格式字眼」兜底
/// （部分招标文件把格式条款放须知前附表，分类器可能落「其他」——漏检比
/// 误检代价高：格式错 = 形式评审扣分/废标风险）。
///
/// 识别模式（确定性，逐模式独立命中独立覆写）：
/// - 字号：`三号/小四/五号…` 中文枚举或 `N pt / N 磅`；
/// - 字体：`仿宋_GB2312 / 仿宋 / 宋体 / 黑体 / 楷体 / 楷体_GB2312 /
///   方正小标宋简体 / 华文中宋 / Times New Roman`（具体变体优先）；
/// - 行距：含「行距/行间距」+ `倍` 前数字；
/// - 边距：方位词（上/下/左/右）后 12 字符窗内的首个整数（mm）。
///
/// 多条命中**后到优先**（REQ 序 = 文档序——补遗在后，补遗覆写主文件是
/// 招标语义）；零命中 → 基线 + warning（降级不伪装）。
pub fn derive_from_requirements(reqs: &[Requirement]) -> (FormatSpec, Vec<String>) {
    let mut spec = baseline();
    let mut warnings = Vec::new();
    let mut hits = 0usize;

    let candidates: Vec<String> = reqs
        .iter()
        .filter(|r| r.category == "格式要求" || has_format_words(&join_text(r)))
        .map(|r| join_text(r))
        .collect();

    for text in &candidates {
        if let Some(pt) = detect_font_size(text) {
            if pt != spec.global.font.size {
                spec.global.font.size = pt;
                hits += 1;
            }
        }
        if let Some(font) = detect_font_family(text) {
            if font != spec.global.font.body {
                spec.global.font.body = font;
                hits += 1;
            }
        }
        if let Some(mult) = detect_line_height(text) {
            if (mult - spec.global.font.line_height).abs() > f64::EPSILON {
                spec.global.font.line_height = mult;
                hits += 1;
            }
        }
        if let Some(mm) = detect_margin(text, "上") {
            if mm != spec.global.page.margin.top { spec.global.page.margin.top = mm; hits += 1; }
        }
        if let Some(mm) = detect_margin(text, "下") {
            if mm != spec.global.page.margin.bottom { spec.global.page.margin.bottom = mm; hits += 1; }
        }
        if let Some(mm) = detect_margin(text, "左") {
            if mm != spec.global.page.margin.left { spec.global.page.margin.left = mm; hits += 1; }
        }
        if let Some(mm) = detect_margin(text, "右") {
            if mm != spec.global.page.margin.right { spec.global.page.margin.right = mm; hits += 1; }
        }
    }

    if hits == 0 {
        if candidates.is_empty() {
            warnings.push(
                "招标文件中未发现格式条款（格式要求类 0 条、格式字眼兜底 0 条）——\
按工程惯例基线导出（A4/仿宋_GB2312 小四/1.5 倍行距）".into(),
            );
        } else {
            warnings.push(format!(
                "发现 {} 条格式候选条款但无字号/字体/行距/边距可识别——按工程惯例基线导出",
                candidates.len()
            ));
        }
    } else {
        warnings.push(format!(
            "从招标文件格式条款提取 {hits} 项格式参数（未识别项保持基线）"
        ));
    }
    (spec, warnings)
}

fn join_text(r: &Requirement) -> String {
    r.anchors.iter().map(|a| a.excerpt.as_str()).collect::<Vec<_>>().join("；")
}

fn has_format_words(text: &str) -> bool {
    (text.contains("字体") || text.contains("字号") || text.contains("行距")
        || text.contains("页边距") || text.contains("边距") || text.contains("版式"))
        && (text.contains("号") || text.contains("pt") || text.contains("磅")
            || text.contains("倍") || text.contains("mm") || text.contains("毫米"))
}

/// 字号识别（中文枚举优先 → `N pt / N 磅`）。
fn detect_font_size(text: &str) -> Option<f64> {
    // ① 中文枚举（长词在前防子串误吞：「小四」先于「四号」无关——枚举词
    // 互不为前缀；但「初号」会命中「小初号」罕见形态，容忍）
    for name in [
        "初号", "小初", "一号", "小一", "二号", "小二", "三号", "小三",
        "四号", "小四", "五号", "小五", "六号", "小六",
    ] {
        if text.contains(name) {
            if let Some(pt) = chinese_font_size_pt(name) {
                return Some(pt);
            }
        }
    }
    // ② N pt / N 磅
    for unit in ["pt", "磅", "PT"] {
        if let Some(n) = number_before_unit(text, unit) {
            if (5.0..=42.0).contains(&n) {
                return Some(n);
            }
        }
    }
    None
}

/// 字体族识别（具体变体优先——「仿宋_GB2312」先于「仿宋」命中）。
fn detect_font_family(text: &str) -> Option<String> {
    for name in [
        "仿宋_GB2312", "方正小标宋简体", "华文中宋", "楷体_GB2312",
        "仿宋", "宋体", "黑体", "楷体", "Times New Roman",
    ] {
        if text.contains(name) {
            return Some(name.to_string());
        }
    }
    None
}

/// 行距识别（含「行距/行间距」的句子里「倍」前的数字；多命中取最后）。
fn detect_line_height(text: &str) -> Option<f64> {
    if !text.contains("行距") && !text.contains("行间距") {
        return None;
    }
    let mut best: Option<f64> = None;
    for (idx, _) in text.match_indices("倍") {
        // 「倍」前向扫连续的 ASCII 数字与小数点（字符级——无字节边界风险）
        let head: String = text[..idx].chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if let Ok(v) = head.parse::<f64>() {
            if (1.0..=3.0).contains(&v) {
                best = Some(v); // 后到优先（多命中取最后——REQ 序 = 补遗序）
            }
        }
    }
    best
}

/// 边距识别（方位词后 12 字符窗内的首个整数，mm 口径）。
fn detect_margin(text: &str, word: &str) -> Option<u32> {
    let mut best: Option<u32> = None;
    for (idx, _) in text.match_indices(word) {
        let after: String = text[idx + word.len()..].chars().take(12).collect();
        for tok in after.split(|c: char| !c.is_ascii_digit()) {
            if tok.is_empty() {
                continue;
            }
            if let Ok(v) = tok.parse::<u32>() {
                if v <= 100 {
                    best = Some(v); // 后到优先
                }
                break; // 窗内首个整数（后续数字是别的内容——如「35 号」）
            }
        }
    }
    best
}

/// `N unit` 形态提取（unit 前最近的数字；多命中取最后）。
fn number_before_unit(text: &str, unit: &str) -> Option<f64> {
    let mut best: Option<f64> = None;
    for (idx, _) in text.match_indices(unit) {
        let num: String = text[..idx].chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if let Ok(v) = num.parse::<f64>() {
            best = Some(v);
        }
    }
    best
}

// ---------------------------------------------------------------- docx 应用桥

/// 把规格应用到 docx 构建器（页面 / Normal 样式 / 页眉页脚）。
///
/// 占位符：`{project_name}`（页眉/页脚文本）、`{page}`/`{pages}`（页脚页码
/// 域——Word 打开后按实际页数展开）。
///
/// **加性桥**：只叠加 sectPr/styles/header/footer，不触碰调用方已加入的
/// 段落内容——export.rs 的既有构建顺序（add_paragraph）不受影响。
#[must_use]
pub fn apply_to_docx(mut docx: docx_rs::Docx, spec: &FormatSpec, project_name: &str) -> docx_rs::Docx {
    use docx_rs::*;

    // ---- 页面 + 边距（sectPr）
    let (w, h) = spec.global.page.size_twips();
    let m = spec.global.page.margin;
    docx = docx.add_section(
        Section::new()
            .page_size(PageSize::new().size(w, h))
            .page_margin(
                PageMargin::new()
                    .top(mm_to_twips(m.top) as i32)
                    .bottom(mm_to_twips(m.bottom) as i32)
                    .left(mm_to_twips(m.left) as i32)
                    .right(mm_to_twips(m.right) as i32),
            ),
    );

    // ---- Normal 样式（正文字体 / 字号 / 行距——全文档默认面）
    let f = &spec.global.font;
    let normal = Style::new("Normal", StyleType::Paragraph)
        .name("Normal")
        .size(f.size_half_points() as usize)
        .fonts(
            RunFonts::new()
                .ascii(&f.body)
                .hi_ansi(&f.body)
                .east_asia(&f.body)
                .cs(&f.body),
        )
        .line_spacing(
            LineSpacing::new()
                .line_rule(LineSpacingType::Auto)
                .line(f.line_units()),
        );
    docx = docx.add_style(normal);

    // ---- 页眉（{project_name} 替换；无占位符按字面）
    if spec.global.header.enabled {
        let text = spec.global.header.content.replace("{project_name}", project_name);
        let p = Paragraph::new().add_run(Run::new().add_text(text));
        docx = docx.header(Header::new().add_paragraph(p));
    }

    // ---- 页脚（{page}/{pages} 域展开；混排文本按段内顺序）
    if spec.global.footer.enabled {
        let mut p = Paragraph::new();
        for part in split_placeholders(&spec.global.footer.content) {
            p = match part {
                FooterPart::Text(t) => p.add_run(Run::new().add_text(t)),
                FooterPart::Page => p.add_page_num(PageNum::new()),
                FooterPart::Pages => p.add_num_pages(NumPages::new()),
            };
        }
        docx = docx.footer(Footer::new().add_paragraph(p));
    }

    docx
}

/// 页脚模板切段（`{page}`/`{pages}` 占位符 → 域；其余字面）。
enum FooterPart {
    Text(String),
    Page,
    Pages,
}

fn split_placeholders(template: &str) -> Vec<FooterPart> {
    let mut parts = Vec::new();
    let mut rest = template;
    while let Some(pos) = rest.find('{') {
        let end = rest[pos..].find('}').map(|e| pos + e + 1);
        match end {
            Some(e) => {
                let (head, ph) = (&rest[..pos], &rest[pos..e]);
                if !head.is_empty() {
                    parts.push(FooterPart::Text(head.to_string()));
                }
                parts.push(match ph {
                    "{page}" => FooterPart::Page,
                    "{pages}" => FooterPart::Pages,
                    other => FooterPart::Text(other.to_string()), // 非法占位符按字面（不吞用户文本）
                });
                rest = &rest[e..];
            }
            None => break, // 尾部残缺 '{'——剩余全按字面
        }
    }
    if !rest.is_empty() {
        parts.push(FooterPart::Text(rest.to_string()));
    }
    parts
}
