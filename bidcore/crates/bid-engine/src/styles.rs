//! styles.rs —— #110 任务 2：样式模板接线（`mdc_office_io::style` 门面）。
//!
//! ## 用例（issue #110 原文）
//!
//! 「style → 样式模板（固定格式件用）」——固定格式件（投标函/授权书/
//! 声明等 7 件，`bid-domain::forms`）与正文按招标文件的格式条款排版；
//! 业主常附**样式模板 docx**（或以历史标书为格式基准）。读取模板的
//! 样式树（docDefaults → basedOn 链 → 派生样式的有效格式 + 主题色），
//! 导出链照单应用——「格式跟模板走」而不是硬编码（Lead 铁律：
//! **不准硬编码格式**，`format_spec.rs` 同源纪律）。
//!
//! ## 引擎语义（只调用，零自研——issue #2 §1 约束 2）
//!
//! [`mdc_office_io::style`]（#103 S1-t1）是引擎侧样式读取的单一真源：
//! 容器三门 + `word/styles.xml` 解析 + basedOn 链解析 + 主题色表。
//! 本模块的边界：
//! - **读**：[`StyleTemplate::from_bytes`] 直调引擎
//!   `read_docx_styles_from_bytes`（字节入参 = 唯一信任边界，与
//!   `reader.rs` 的 `EngineReader` 同族纪律）；
//! - **查询**：[`StyleTemplate::effective`] 把引擎 `EffectiveFormat`
//!   （docDefaults + basedOn 链合并结果）投影为标书面镜像
//!   [`EngineStyleFormat`]（中文排版关心的位：east_asia 字体/字号磅/
//!   粗斜体/对齐/outline 级）——引擎类型零泄漏（`bundle` 私有承载）；
//! - **不做**：样式写入（引擎 `write.rs` 的 `StyleDef` 写面）、样式
//!   匹配策略（「模板的哪个样式对应固定格式件的哪一段」是消费方语义，
//!   登记在导出链接线轮）。
//!
//! ## 半点/磅换算
//!
//! OOXML `w:sz` 单位 = 半点（24 = 12pt）；标书面 [`EngineStyleFormat::
//! size_pt`] 用磅（与 `format_spec::FontSpec::size` 同单位制）。

use mdc_office_io::style::{self, DocxStyleError};

use crate::types::EngineError;

/// 样式有效格式（引擎 `EffectiveFormat` 的标书面投影——中文排版位）。
///
/// `None` = 模板未设（继承下层/渲染器缺省）——诚实承载，不填默认值
/// （填默认 = 映射层自研语义，违反约束 3）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineStyleFormat {
    /// 中文字体（`w:rFonts/@w:eastAsia`——固定格式件正文的决定位）。
    pub east_asia_font: Option<String>,
    /// 西文字体（`w:rFonts/@w:ascii`）。
    pub ascii_font: Option<String>,
    /// 字号（磅——半点 ÷ 2；`format_spec::FontSpec::size` 同单位制）。
    pub size_pt: Option<f64>,
    /// 粗体（三态：None = 未设）。
    pub bold: Option<bool>,
    /// 斜体（三态：None = 未设）。
    pub italic: Option<bool>,
    /// 段落对齐（`w:jc/@w:val` 原值透传：`left`/`center`/`right`/`both`…）。
    pub alignment: Option<String>,
    /// outline 级（0-8——`w:outlineLvl`；0 = 正文级标题语义）。
    pub outline_level: Option<usize>,
}

/// 样式模板（引擎 `StyleBundle` 的私有承载 + 查询面——引擎类型不出
/// bid-engine 边界）。
///
/// 查询面按用例收窄：`style_ids`（样式清单）/ `effective`（有效格式）/
/// `doc_default`（docDefaults 投影）/ `theme_color`（主题色解析）/
/// `warnings`（容器门面警告透传——消费方计入日志，不静默吞）。
pub struct StyleTemplate {
    bundle: style::StyleBundle,
}

/// 引擎样式错误透传（容器门拒绝 → `EngineRejected`——与 `reader.rs`
/// 的 `map_book_error` 同分类纪律：prescan/容器损坏/部件无效 = 安全语义
/// 拒绝；解析/IO → `EngineFailed`）。
fn map_style_error(e: DocxStyleError) -> EngineError {
    match e {
        DocxStyleError::Container(io_err) => match io_err {
            mdc_office_io::io::ReadError::PrescanRejected { reason, entry } => {
                EngineError::EngineRejected(format!(
                    "prescan rejected: {reason}{}",
                    entry.map(|s| format!(" ({s})")).unwrap_or_default()
                ))
            }
            mdc_office_io::io::ReadError::ContainerCorrupt(c) => EngineError::EngineRejected(c),
            mdc_office_io::io::ReadError::InvalidPart { part, detail } => {
                EngineError::EngineRejected(format!("invalid part {part}: {detail}"))
            }
            other => EngineError::EngineFailed(other.to_string()),
        },
        other => EngineError::EngineFailed(other.to_string()),
    }
}

impl StyleTemplate {
    /// 从 docx 字节读样式模板（容器三门在引擎侧——本层零守卫）。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EngineError> {
        Ok(Self {
            bundle: style::read_docx_styles_from_bytes(bytes).map_err(map_style_error)?,
        })
    }

    /// 样式 id 清单（`styles.xml` 出现序——消费方 UI/匹配按原序）。
    pub fn style_ids(&self) -> Vec<String> {
        self.bundle.tree.style_ids().to_vec()
    }

    /// 样式 id 的有效格式（docDefaults → basedOn 链 → 派生合并——
    /// 引擎 `StyleTree::effective_format` 单一真源）。
    ///
    /// 未知 id → `None`（诚实缺失——消费方决定回退策略）。
    pub fn effective(&self, style_id: &str) -> Option<EngineStyleFormat> {
        let fmt = self.bundle.tree.effective_format(style_id)?;
        Some(project_format(&fmt))
    }

    /// 文档默认格式（`w:docDefaults` 投影——无 `pStyle` 段落的生效面）。
    pub fn doc_default(&self) -> EngineStyleFormat {
        project_format(&style::EffectiveFormat {
            run: self.bundle.tree.doc_defaults.run.clone(),
            para: self.bundle.tree.doc_defaults.para.clone(),
        })
    }

    /// 主题色解析（themeColor token → `RRGGBB` 大写；未命中 → `None`）。
    ///
    /// 引擎 `ThemeColors::lookup` 单一真源（`dark1`→`dk1` 映射 +
    /// `text1`≡`dark1` 别名——引擎语义，本层零映射）。
    pub fn theme_color(&self, token: &str) -> Option<String> {
        self.bundle.theme.lookup(token)
    }

    /// 容器门面警告（引擎 `CompatibilityWarning` 透传——`reader.rs`
    /// 同款镜像）。
    pub fn warnings(&self) -> Vec<crate::types::EngineWarning> {
        self.bundle
            .warnings
            .iter()
            .map(|w| {
                let code = match w.kind {
                    mdc_office_io::io::WarningKind::MalformedXml => "MalformedXml",
                    mdc_office_io::io::WarningKind::ControlChars => "ControlChars",
                    mdc_office_io::io::WarningKind::BrokenReference => "BrokenReference",
                    mdc_office_io::io::WarningKind::RevisionDelete => "RevisionDelete",
                    mdc_office_io::io::WarningKind::SkippedElement => "SkippedElement",
                };
                crate::types::EngineWarning {
                    code: code.to_string(),
                    message: format!("{}: {}", w.part, w.detail),
                }
            })
            .collect()
    }
}

/// 引擎有效格式 → 标书面投影（纯字段搬运 + 半点→磅换算）。
fn project_format(fmt: &style::EffectiveFormat) -> EngineStyleFormat {
    EngineStyleFormat {
        east_asia_font: fmt.run.fonts.as_ref().and_then(|f| f.east_asia.clone()),
        ascii_font: fmt.run.fonts.as_ref().and_then(|f| f.ascii.clone()),
        size_pt: fmt.run.size.map(|half_points| half_points as f64 / 2.0),
        bold: fmt.run.bold,
        italic: fmt.run.italic,
        alignment: fmt.para.alignment.clone(),
        outline_level: fmt.para.outline_lvl,
    }
}
