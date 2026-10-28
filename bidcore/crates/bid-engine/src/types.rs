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
//! - `EngineDoc` 增 `comments`（#19）/ `footnotes` / `endnotes` 计数面；
//! - **PR #15 Lead review 整改**：块级 `source` 标记（引擎 PR #25 的
//!   `BlockSource` BREAKING 变更）——`EngineBlockSource` 镜像 + 三块型
//!   source 字段 + `EngineHeaderFooter` 变体承载 + 分流面（Header/Footer
//!   页面装置不进条款抽取流，计数登记不静默丢）。

/// 块来源面镜像（引擎 PR #25 / #22 P2-1：`BlockSource`）。
///
/// 语义二分 = **文档内容** vs **页面装置**：`Body` 覆盖正文及文档附属
/// 内容面（脚注/尾注/批注——承载通道独立于正文块序）；`Header`/`Footer`
/// 仅对应六槽位挂接部件内的块。条款抽取流只吃 `Body`（PR #15 review
/// 要求）；`Header`/`Footer` 是页面装置（页眉项目名/页脚页码），
/// 登记计数但不进抽取流。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EngineBlockSource {
    /// 正文（含脚注/尾注/批注等文档内容面——语义上仍是文档内容）。
    #[default]
    Body,
    /// 页眉部件内块。
    Header,
    /// 页脚部件内块。
    Footer,
}

/// docx 读取结果镜像（对应引擎 `DocxDocument`）。
#[derive(Debug, Clone, Default)]
pub struct EngineDoc {
    /// 正文块流（段落/表格/文本框，文档序）。
    pub blocks: Vec<EngineBlock>,
    /// 页眉（default/first/even 实际挂接者——D3 面：引擎读面已接）。
    pub headers: Vec<EngineHeaderFooter>,
    /// 页脚（同上）。
    pub footers: Vec<EngineHeaderFooter>,
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

impl EngineBlock {
    /// 块来源（PR #15 review：消费方按 source 分流——Body 进条款抽取流，
    /// Header/Footer 页面装置登记不进抽取）。
    pub fn source(&self) -> EngineBlockSource {
        match self {
            EngineBlock::Paragraph(p) => p.source,
            EngineBlock::Table(t) => t.source,
            EngineBlock::TextBox(tb) => tb.source,
        }
    }
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
    /// 块来源（引擎 PR #25：正文块恒 `Body`——镜像保真搬运，
    /// 消费方分流见 [`EngineBlock::source`]）。
    pub source: EngineBlockSource,
}

/// 表格镜像（对应引擎 `TableData`——X3 定谳：扁平无合并结构，设计登记）。
#[derive(Debug, Clone, Default)]
pub struct EngineTable {
    /// 行 × 列扁平文本（行列编址在合并下按引擎读回口径）。
    pub rows: Vec<Vec<String>>,
    /// 块来源（引擎 PR #25：表所在文档位置——表内段落继承表来源）。
    pub source: EngineBlockSource,
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
    /// 公式文本矩阵（**已对齐 values 行列**——底层引擎的 worksheet_formula
    /// 范围只覆盖公式格（包围盒 ≠ values），reader 以 formula_at 绝对坐标
    /// 联接点对齐后填充；空串 = 该格无公式）。
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
    /// 块来源（引擎 PR #25：与宿主块同源——文本框当前仅正文段承载）。
    pub source: EngineBlockSource,
}

// ---------------------------------------------------------------------------
// PR #15 Lead review 整改：页眉/页脚变体承载 + 分流登记面
// ---------------------------------------------------------------------------

/// 页眉/页脚槽位变体镜像（引擎 `HeaderFooterVariant`——透传
/// `headerReference/@w:type`；first/even 槽位激活需节设置，语义解释面外）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EngineHeaderFooterVariant {
    #[default]
    Default,
    First,
    Even,
}

/// 页眉/页脚挂接镜像（引擎 `HeaderFooterData`：变体 + source 标记块序）。
///
/// PR #15 Lead review 整改：此前镜像只留文本行序（`Vec<Vec<String>>`），
/// 变体与块结构面丢失——升级为全字段搬运（与引擎侧验收口径对齐），
/// 文本行序降级为便捷方法 [`EngineHeaderFooter::texts`]。
#[derive(Debug, Clone)]
pub struct EngineHeaderFooter {
    /// 槽位变体（default/first/even）。
    pub variant: EngineHeaderFooterVariant,
    /// 部件内块序（source 恒 `Header`/`Footer`——按部件定值）。
    pub blocks: Vec<EngineBlock>,
}

impl EngineHeaderFooter {
    /// 文本行序（账本/前导展示面——段落文本直连；表格行非空列空格连接）。
    pub fn texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        for b in &self.blocks {
            match b {
                EngineBlock::Paragraph(p) => out.push(p.text.clone()),
                EngineBlock::Table(t) => {
                    for row in &t.rows {
                        out.push(
                            row.iter()
                                .filter(|c| !c.trim().is_empty())
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(" "),
                        );
                    }
                }
                EngineBlock::TextBox(tb) => {
                    for inner in &tb.blocks {
                        if let EngineBlock::Paragraph(p) = inner {
                            out.push(p.text.clone());
                        }
                    }
                }
            }
        }
        out
    }
}

impl EngineDoc {
    /// 条款抽取流：仅 `Body` 块（PR #15 Lead review——Header/Footer 是
    /// 页面装置，不进条款抽取）。引擎面 `blocks` 恒 Body，此处为防御性
    /// 过滤面：引擎未来把页眉块混入流时，消费方不误吃。
    pub fn body_blocks(&self) -> impl Iterator<Item = &EngineBlock> {
        self.blocks
            .iter()
            .filter(|b| b.source() == EngineBlockSource::Body)
    }

    /// 页眉/页脚块登记计数（账本面——分流不静默丢：`(header_blocks,
    /// footer_blocks)`；B3-3 映射时填 `ComponentLedger.extracted` 真实数）。
    pub fn header_footer_block_counts(&self) -> (usize, usize) {
        let h: usize = self.headers.iter().map(|hf| hf.blocks.len()).sum();
        let f: usize = self.footers.iter().map(|hf| hf.blocks.len()).sum();
        (h, f)
    }
}
