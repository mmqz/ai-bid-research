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
