//! 引擎公开形状的**标书面镜像类型**（T1：只定义）。
//!
//! 形状锚点（mdcore @ f8c860a 只读 worktree 核对，B3 接通时以固定 rev 再核）：
//! - docx：`office-io/src/docx.rs:1468`（ParagraphData）/ `:1488`（Block）/
//!   `:1495`（TableData）/ `:1511`（HeaderFooterData）/ `:1519`（DocxDocument）
//! - xlsx：`office-io/src/io.rs:1112`（SheetData）/ `:1209`（Book）
//! - pdf：`mdc-pdf/src/command.rs:94`（Command::ExtractText）/
//!   `:125`（Outcome::ExtractedText { pages: Vec<(PageIndex, String)> }）
//!
//! 镜像而非 re-export 的理由：bid 侧消费面（bid-parse 之后的 chunk/extract/
//! search）只依赖本 crate 的稳定形状——引擎类型演进（字段增删）时变化被
//! 封闭在 B3 适配器一处，不击穿标书侧全部下游。

/// docx 读取结果镜像（对应引擎 `DocxDocument`）。
#[derive(Debug, Clone, Default)]
pub struct EngineDoc {
    /// 正文块流（段落/表格，文档序）。
    pub blocks: Vec<EngineBlock>,
    /// 页眉（default/first/even 实际挂接者——D3 面：引擎已有，B2 定谳「标书面缺口」）。
    pub headers: Vec<Vec<String>>,
    /// 页脚（同上）。
    pub footers: Vec<Vec<String>>,
    /// 媒体清单（容器级枚举——块序内图片位置引擎不入块模型，登记面）。
    pub images: Vec<EngineImage>,
    /// 引擎兼容性警告（B3 必须透传到 `ParsedFile.components` 账本 note）。
    pub warnings: Vec<EngineWarning>,
}

/// docx 块镜像（对应引擎 `Block::Paragraph` / `Block::Table`）。
#[derive(Debug, Clone)]
pub enum EngineBlock {
    Paragraph(EngineParagraph),
    Table(EngineTable),
}

/// 段落镜像（对应引擎 `ParagraphData`——`text()` 为 run 直连拼接）。
#[derive(Debug, Clone, Default)]
pub struct EngineParagraph {
    /// 样式 ID（`Heading1`/`Normal`…）——A1 修复前恒 `None`（引擎 headings: 0）。
    pub style: Option<String>,
    /// 段落拼接文本（w:ins 内当前生效文本——**D6 引擎缺口修复前缺失**）。
    pub text: String,
    /// 列表编号在位（`w:numId`——A2 修复前引擎读面会把自动编号叠加进文本，
    /// B2.5 实测 docx 条款号污染 99.9%，**B3 硬前置**）。
    pub has_numbering: bool,
    /// 修订插入文本（mdcore#17 修复后填充；修复前为空——缺口走映射层登记）。
    pub revision_inserted: Vec<String>,
    /// 修订删除历史文本（delText——修复前为空；#17 期望为登记面非正文）。
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
