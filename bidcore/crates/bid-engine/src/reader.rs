//! 引擎读取 trait——B3 的**唯一实现点**（T1：只定义）。
//!
//! B3 实现将包装（mdcore 固定 rev）：
//! - [`EngineReader::read_docx`] → `mdc_office_io::docx::read_docx_from_bytes`
//!   （容器三门 → 自有关系层 → docx-rs 模型 → 联接提取，全链在引擎侧）
//! - [`EngineReader::read_xlsx`] → `mdc_office_io::io::read_book_from_bytes`
//! - [`EngineReader::extract_pdf_text`] → `mdc-pdf Command::ExtractText`
//!   （注入 pdfium 引擎后页粒度抽取；OCR 分支不在此面——扫描件走标书侧
//!   tesseract 补遗通道，`docs/engine-bid-mapping.md` §pdf 处置列）
//!
//! 实现纪律（issue #2 §1 约束 1/2）：只调用、零自研语义——zip 门/实体门/
//! 指纹/良构校验全部由引擎执行；本 trait 的 bytes 入参即唯一信任边界。

use crate::types::{EngineBook, EngineDoc, EngineError, EnginePdf};

/// 引擎读取接触面（docx / xlsx / pdf-text 三通道）。
pub trait EngineReader {
    /// 读取 docx（正文块流 + 页眉页脚 + 媒体 + 警告）。
    fn read_docx(&self, bytes: &[u8]) -> Result<EngineDoc, EngineError>;

    /// 读取 xlsx（逐 sheet 值/公式/合并区）。
    fn read_xlsx(&self, bytes: &[u8]) -> Result<EngineBook, EngineError>;

    /// 抽取 pdf 文本（页粒度；无文本层件返回空页集 + `any_text_layer=false`）。
    fn extract_pdf_text(&self, bytes: &[u8]) -> Result<EnginePdf, EngineError>;
}

// ---------------------------------------------------------------------------
// B3 实装：MdcoreReader（mdc-office-io 固定 rev 直连——唯一引擎接触点）
// ---------------------------------------------------------------------------

use crate::types::{
    EngineBlock, EngineBlockSource, EngineComment, EngineHeaderFooter,
    EngineHeaderFooterVariant, EngineImage, EngineParagraph, EngineTable, EngineTextBox,
    EngineWarning,
};
use mdc_office_io::docx::{self, DocxError};
use mdc_office_io::io;

/// 引擎直连 reader（零自有语义——容器三门/关系层/模型解析全在引擎侧）。
///
/// 实现纪律（issue #2 §1 约束 1/2）：本结构体只做**调用与字段搬运**；
/// 错误透传不改写语义（`EngineRejected` = 引擎 typed 拒绝原文）。
#[derive(Debug, Clone, Default)]
pub struct MdcoreReader;

impl MdcoreReader {
    pub fn new() -> Self {
        Self
    }
}

/// docx 错误透传：容器门拒绝（门 1/2/3 + .rels）→ `EngineRejected`（安全语义，
/// 调用方依赖此分类做敌意面差分）；模型/IO → `EngineFailed`。
fn map_docx_error(e: DocxError) -> EngineError {
    match e {
        DocxError::Container(c) => EngineError::EngineRejected(c.to_string()),
        other => EngineError::EngineFailed(other.to_string()),
    }
}

/// xlsx 读错误映射（`ReadError` 四变体——typed reject 原文透传不改写语义）。
fn map_book_error(e: io::ReadError) -> EngineError {
    match e {
        io::ReadError::PrescanRejected { reason, entry } => EngineError::EngineRejected(format!(
            "prescan rejected: {reason}{}", entry.map(|s| format!(" ({s})")).unwrap_or_default()
        )),
        io::ReadError::ContainerCorrupt(c) => EngineError::EngineRejected(c),
        io::ReadError::InvalidPart { part, detail } => {
            EngineError::EngineRejected(format!("invalid part {part}: {detail}"))
        }
        other => EngineError::EngineFailed(other.to_string()),
    }
}

/// calamine `Data` → 单元格字符串（**旧实现口径对齐**——`bid-parse/src/xlsx.rs`
/// 同款规则：整值 Float 压 i64、String 修剪、Error 带标记。B4 删旧实现后
/// 本函数即唯一口径真源——块流等价的验收锚）。
fn cell_to_string(c: &calamine::Data) -> String {
    use calamine::Data;
    match c {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        Data::Int(i) => format!("{i}"),
        Data::DateTime(d) => d.to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Bool(b) => format!("{b}"),
        Data::Error(e) => format!("<err {e:?}>"),
    }
}

/// 引擎块 → 镜像块（递归——文本框内层块与正文同构，#18）。
fn convert_block(b: &docx::Block) -> EngineBlock {
    match b {
        docx::Block::Paragraph(p) => EngineBlock::Paragraph(EngineParagraph {
            style: p.style.clone(),
            heading_level: p.heading_level,
            text: p.text(),
            has_numbering: p.numbering.is_some(),
            fields_dropped: p.fields_dropped,
            footnote_refs: p.footnote_refs.clone(),
            revision_inserted: p
                .runs
                .iter()
                .filter(|r| r.revision_inserted)
                .map(|r| r.text.clone())
                .collect(),
            revision_deleted: Vec::new(), // #17 口径：delText 不入正文，计数走 warnings
            source: map_source(p.source),
        }),
        docx::Block::Table(t) => EngineBlock::Table(EngineTable {
            rows: t.rows.clone(),
            source: map_source(t.source),
        }),
        docx::Block::TextBox(tb) => EngineBlock::TextBox(EngineTextBox {
            blocks: tb.blocks.iter().map(convert_block).collect(),
            source: map_source(tb.source),
        }),
    }
}

/// 引擎 `BlockSource` → 镜像（PR #15 review：三块型 source 字段搬运）。
fn map_source(s: docx::BlockSource) -> EngineBlockSource {
    match s {
        docx::BlockSource::Body => EngineBlockSource::Body,
        docx::BlockSource::Header => EngineBlockSource::Header,
        docx::BlockSource::Footer => EngineBlockSource::Footer,
    }
}

/// 页眉/页脚挂接镜像（变体 + source 标记块序——文本行序便捷面在
/// `EngineHeaderFooter::texts`，块结构面保真供 B3-3 账本映射）。
fn convert_hf(hf: &docx::HeaderFooterData) -> EngineHeaderFooter {
    EngineHeaderFooter {
        variant: match hf.variant {
            docx::HeaderFooterVariant::Default => EngineHeaderFooterVariant::Default,
            docx::HeaderFooterVariant::First => EngineHeaderFooterVariant::First,
            docx::HeaderFooterVariant::Even => EngineHeaderFooterVariant::Even,
        },
        blocks: hf.blocks.iter().map(convert_block).collect(),
    }
}


fn convert_warning(w: &io::CompatibilityWarning) -> EngineWarning {
    let code = match w.kind {
        io::WarningKind::MalformedXml => "MalformedXml",
        io::WarningKind::ControlChars => "ControlChars",
        io::WarningKind::BrokenReference => "BrokenReference",
        io::WarningKind::RevisionDelete => "RevisionDelete",
    };
    EngineWarning {
        code: code.to_string(),
        message: format!("{}: {}", w.part, w.detail),
    }
}

fn convert_comment(c: &docx::CommentData) -> EngineComment {
    EngineComment {
        id: c.id,
        author: c.author.clone(),
        date: c.date.clone(),
        anchor_block_idx: c.anchor_block_idx,
        texts: c
            .blocks
            .iter()
            .filter_map(|b| match b {
                docx::Block::Paragraph(p) => Some(p.text()),
                _ => None,
            })
            .collect(),
    }
}

impl EngineReader for MdcoreReader {
    fn read_docx(&self, bytes: &[u8]) -> Result<crate::types::EngineDoc, EngineError> {
        let doc = docx::read_docx_from_bytes(bytes).map_err(map_docx_error)?;

        Ok(crate::types::EngineDoc {
            blocks: doc.blocks.iter().map(convert_block).collect(),
            headers: doc.headers.iter().map(convert_hf).collect(),
            footers: doc.footers.iter().map(convert_hf).collect(),
            images: doc
                .images
                .iter()
                .map(|i| EngineImage {
                    name: i.part_name.clone(),
                    bytes: i.bytes.len(),
                    content_type: i.content_type.clone().unwrap_or_default(),
                })
                .collect(),
            warnings: doc.warnings.iter().map(convert_warning).collect(),
            comments: doc.comments.iter().map(convert_comment).collect(),
            footnotes: doc.footnotes.len(),
            endnotes: doc.endnotes.len(),
        })
    }

    fn read_xlsx(&self, bytes: &[u8]) -> Result<crate::types::EngineBook, EngineError> {
        // B3-4 实装：门 0（输入字节）→ 门 1（声明值预扫）→ 门 2+3（尺寸/部件
        // 融合验证）→ calamine 五面提取 + 提取期限额（表数/累计稠密面积）——
        // 全链在引擎侧（issue #2 §1 约束 1/2：标书侧零自研守卫）。
        let book = io::read_book_from_bytes(bytes).map_err(map_book_error)?;
        Ok(crate::types::EngineBook {
            sheets: book
                .sheets
                .iter()
                .map(|s| {
                    // 值面矩阵（Range 相对迭代）+ 公式面联接：
                    // 引擎 SheetData::formula_at 是**绝对坐标**入参的官方安全
                    // 联接点（公式范围与值范围包围盒不同——worksheet_formula
                    // 只覆盖公式格；镜像注释「同包围盒」不成立，此处以
                    // values.start() 锚定换算，空串 = 无公式）。
                    let vstart = s.values.start();
                    let mut values: Vec<Vec<String>> = Vec::new();
                    let mut formulas: Vec<Vec<String>> = Vec::new();
                    for (ri, vrow) in s.values.rows().enumerate() {
                        values.push(vrow.iter().map(cell_to_string).collect());
                        let mut frow = vec![String::new(); vrow.len()];
                        if let Some((sr, sc)) = vstart {
                            for (ci, _) in vrow.iter().enumerate() {
                                if let Some(f) = s.formula_at(sr + ri as u32, sc + ci as u32) {
                                    frow[ci] = f.trim().to_string();
                                }
                            }
                        }
                        formulas.push(frow);
                    }
                    crate::types::EngineSheet {
                        name: s.name.clone(),
                        values,
                        formulas,
                        // calamine Dimensions：pub 字段 (row, col) 0 基 → 1 基
                        merges: s
                            .merges
                            .iter()
                            .map(|d| {
                                (d.start.0 as usize + 1, d.start.1 as usize + 1,
                                 d.end.0 as usize + 1, d.end.1 as usize + 1)
                            })
                            .collect(),
                    }
                })
                .collect(),
            warnings: book.warnings.iter().map(convert_warning).collect(),
        })
    }

    fn extract_pdf_text(&self, _bytes: &[u8]) -> Result<crate::types::EnginePdf, EngineError> {
        // B3-5 批次实装（pdfium 可用性探测先行）。
        Err(EngineError::MappingGap(
            "extract_pdf_text: B3-5 未实装（pdfium 通道探测先行）".to_string(),
        ))
    }
}
