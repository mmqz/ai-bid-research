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
    EngineBlock, EngineComment, EngineImage, EngineParagraph, EngineTable, EngineTextBox,
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
        }),
        docx::Block::Table(t) => EngineBlock::Table(EngineTable {
            rows: t.rows.clone(),
        }),
        docx::Block::TextBox(tb) => EngineBlock::TextBox(EngineTextBox {
            blocks: tb.blocks.iter().map(convert_block).collect(),
        }),
    }
}

/// 页眉/页脚部件块 → 文本行序（段落文本直连；表格行以空格连接——
/// 账本/前导展示面，非块流口径）。
fn hf_texts(blocks: &[docx::Block]) -> Vec<String> {
    let mut out = Vec::new();
    for b in blocks {
        match b {
            docx::Block::Paragraph(p) => out.push(p.text()),
            docx::Block::Table(t) => {
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
            docx::Block::TextBox(tb) => {
                for inner in &tb.blocks {
                    if let docx::Block::Paragraph(p) = inner {
                        out.push(p.text());
                    }
                }
            }
        }
    }
    out
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
            headers: doc.headers.iter().map(|h| hf_texts(&h.blocks)).collect(),
            footers: doc.footers.iter().map(|f| hf_texts(&f.blocks)).collect(),
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

    fn read_xlsx(&self, _bytes: &[u8]) -> Result<crate::types::EngineBook, EngineError> {
        // B3-4 批次实装（本 PR 仅 docx 通道——保持每批可验收）。
        Err(EngineError::MappingGap(
            "read_xlsx: B3-4 未实装（本批次 docx 通道先行）".to_string(),
        ))
    }

    fn extract_pdf_text(&self, _bytes: &[u8]) -> Result<crate::types::EnginePdf, EngineError> {
        // B3-5 批次实装（pdfium 可用性探测先行）。
        Err(EngineError::MappingGap(
            "extract_pdf_text: B3-5 未实装（pdfium 通道探测先行）".to_string(),
        ))
    }
}
