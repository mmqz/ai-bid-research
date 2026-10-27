//! 引擎公开形状的**标书面镜像类型**（T1 定义 + B3 加性扩面）。
//!
//! 形状锚点（mdcore @ 2d014445 固定 rev 核对，含 #17/#18/#19/A1/A2/P2-1）：
//! - docx：`office-io/src/docx.rs`（ParagraphData / Block / TableData /
//!   TextBoxData / CommentData / HeaderFooterData / DocxDocument）
//! - xlsx：`office-io/src/io.rs`（SheetData / Book）
//! - pdf：`mdc-pdf/src/command.rs`（Command::ExtractText → Outcome::ExtractedText）
//!
//! 镜像而非 re-export 的理由：bid 侧消费面（bid-parse 之后的 chunk/extract/
//! search）只依赖本 crate 的稳定形状——引擎类型演进（字段增删）时变化被
//! 封闭在 B3 适配器一处，不击穿标书侧全部下游。
//!
//! B3 加性扩面（本 rev 已含全部前置修复——T1 注记中的「修复前恒 None/空」
//! 语义已过时，按映射表「已修复验收」列更新）：
//! - `EngineParagraph` 增 `heading_level`（A1）/ `fields_dropped`（A2）/
//!   `footnote_refs`（#19）；
//! - `EngineBlock` 增 `TextBox` 变体（#18——内层块与正文同构）；
//! - `EngineDoc` 增 `comments`（#19）/ `footnotes` / `endnotes` 计数面。

/// docx 读取结果镜像（对应引擎 `DocxDocument`）。
#[derive(Debug, Clone, Default)]
pub struct EngineDoc {
    /// 正文块流（段落/表格/文本框，文档序）。
    pub blocks: Vec<EngineBlock>,
    /// 页眉（default/first/even 实际挂接者——D3 面：引擎读面已接）。
    pub headers: Vec<Vec<String>>,
    /// 页脚（同上）。
    pub footers: Vec<Vec<String>>,
    /// 媒体清单（容器级枚举——块序内图片位置引擎不入块模型，登记面）。
    pub images: Vec<EngineImage>,
    /// 引擎兼容性警告（B3 必须透传到 `ParsedFile.components` 账本 note）。
    pub warnings: Vec<EngineWarning>,
    /// 批注（#19——账本 present 面与 M2 锚定消费）。
    pub comments: Vec<EngineComment>,
    /// 脚注部件计数（分隔符已滤——账本 present 面）。
    pub footnotes: usize,
    /// 尾注部件计数（同上）。
    pub endnotes: usize,
}

/// docx 块镜像（对应引擎 `Block::Paragraph` / `Block::Table` / `Block::TextBox`）。
#[derive(Debug, Clone)]
pub enum EngineBlock {
    Paragraph(EngineParagraph),
    Table(EngineTable),
    /// 文本框（#18——内层块与正文同构；映射层拍平入流，镜像保结构供差分）。
    TextBox(EngineTextBox),
}

/// 段落镜像（对应引擎 `ParagraphData`——`text()` 为 run 直连拼接）。
#[derive(Debug, Clone, Default)]
pub struct EngineParagraph {
    /// 样式 ID（`Heading1`/`Normal`/真 Word 中文版 `"1"`…——语义解析在 heading_level）。
    pub style: Option<String>,
    /// 标题层级（A1 已修复 @ a85decd：四链解析 1-9 域，None = 非标题段）——
    /// B3 后作为标书章节上下文的冗余加固面（文本正则主通道，B2.5 已证免疫）。
    pub heading_level: Option<usize>,
    /// 段落拼接文本（run 直连——含 w:ins 内当前生效文本，#17 已修复）。
    pub text: String,
    /// 列表编号在位（`w:numId`——A2 已修复 @ 338acdc：表现型域缓存结果剥离）。
    pub has_numbering: bool,
    /// 本段剥离的表现型域缓存结果计数（A2——透传账本 note）。
    pub fields_dropped: usize,
    /// 本段引用的脚注 id 序（#19——段落粒度标注面）。
    pub footnote_refs: Vec<i64>,
    /// 修订插入文本（#17 已修复：w:ins 内 run 标记 + 文本全入读回）。
    pub revision_inserted: Vec<String>,
    /// 修订删除历史文本（#17 口径：delText 不入正文，计数走 warnings——
    /// 此字段恒空，保留为语义占位以免与「引擎缺口」混淆）。
    pub revision_deleted: Vec<String>,
}

/// 表格镜像（对应引擎 `TableData`——X3 定谳：扁平无合并结构，设计登记）。
#[derive(Debug, Clone, Default)]
pub struct EngineTable {
    /// 行 × 列扁平文本（行列编址在合并下按引擎读回口径）。
    pub rows: Vec<Vec<String>>,
}

/// 图片镜像（对应引擎 `ImageAsset`——容器级枚举字段子集，B3 按需扩）。
#[derive(Debug, Clone, Default)]
pub struct EngineImage {
    pub name: String,
    pub bytes: usize,
    pub content_type: String,
}

/// 引擎兼容性警告镜像（对应 `CompatibilityWarning`——文本面足够账本登记用）。
#[derive(Debug, Clone, Default)]
pub struct EngineWarning {
    pub code: String,
    pub message: String,
}

/// xlsx 读取结果镜像（对应引擎 `Book`）。
#[derive(Debug, Clone, Default)]
pub struct EngineBook {
    pub sheets: Vec<EngineSheet>,
    pub warnings: Vec<EngineWarning>,
}

/// 工作表镜像（对应引擎 `SheetData`——X3 探针实测 merges API 生效）。
#[derive(Debug, Clone, Default)]
pub struct EngineSheet {
    pub name: String,
    /// 值矩阵（行序；空格为空串）。
    pub values: Vec<Vec<String>>,
    /// 公式文本矩阵（与 values 同包围盒——`None` 位置以空串表示）。
    pub formulas: Vec<Vec<String>>,
    /// 合并区（r1c1-r2c2 序对，1 基）。
    pub merges: Vec<(usize, usize, usize, usize)>,
}

/// pdf 文本抽取镜像（对应 `Command::ExtractText` → `Outcome::ExtractedText`）。
#[derive(Debug, Clone, Default)]
pub struct EnginePdf {
    /// 页文本（页号 1 基升序——P1 定谳：文本层件与 pdftotext 非空白字符比 1.000）。
    pub pages: Vec<String>,
    /// 全部页零文本 → false（bid 侧 has_text_layer 口径；部分页无文本层 = partial，
    /// 逐页判定由映射层消费 D5b 语义）。
    pub any_text_layer: bool,
}

/// 适配层错误（T1：传输级；语义缺口不入此——走 warnings/账本登记）。
#[derive(Debug, Clone)]
pub enum EngineError {
    /// 引擎拒绝（三门/良构拒绝——错误原文透传，禁止改写语义）。
    EngineRejected(String),
    /// 引擎调用失败（panic 边界/IO）。
    EngineFailed(String),
    /// 映射缺口（引擎类型形状超出镜像面——B3 演进时显式失败，不静默丢弃）。
    MappingGap(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::EngineRejected(s) => write!(f, "engine rejected: {s}"),
            EngineError::EngineFailed(s) => write!(f, "engine failed: {s}"),
            EngineError::MappingGap(s) => write!(f, "mapping gap: {s}"),
        }
    }
}

impl std::error::Error for EngineError {}

// ---------------------------------------------------------------------------
// B3 扩面（T1 冻结后的加性扩展——映射表「缺失处置」列所需的账本面）
// ---------------------------------------------------------------------------

/// 批注镜像（#19：`DocxDocument.comments` 元素——账本 present 面与 M2
/// 锚定消费所需的最小字段集；批注内容块文本以段落文本序承载）。
#[derive(Debug, Clone, Default)]
pub struct EngineComment {
    pub id: usize,
    pub author: String,
    pub date: String,
    /// 锚定的引擎块索引（`commentRangeStart` 首现块；None = 孤儿批注）。
    pub anchor_block_idx: Option<usize>,
    /// 批注内容段落文本（文档序）。
    pub texts: Vec<String>,
}

/// 文本框镜像（#18：`Block::TextBox`——内层块与正文同构，映射层按
/// 「块序直连」拍平；镜像保持层级以便差分工具区分结构面）。
#[derive(Debug, Clone, Default)]
pub struct EngineTextBox {
    pub blocks: Vec<EngineBlock>,
}
