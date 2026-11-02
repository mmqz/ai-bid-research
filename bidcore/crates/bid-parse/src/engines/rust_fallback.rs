//! engines/rust_fallback.rs —— 纯 Rust 兜底引擎（#200 R2：包装 pdf_text）。
//!
//! 零依赖（无外部二进制），覆盖面与诚实自报口径见 [`crate::pdf_text`]。
//! 引擎包装层不重复实现任何解析逻辑——`extract_parsed` 直接委托
//! `pdf_text::parse_via_fallback`（summary 自报「纯 Rust PDF 文本兜底：…」，
//! addendum 回归在此消息上钉着）。

use crate::engine::{EngineError, EngineStatus, PageText, PdfExtractor};
use crate::pdf_text;
use crate::ParsedFile;
use std::path::Path;

/// 纯 Rust 兜底引擎（pdf_text 包装）。
pub struct RustFallback;

impl PdfExtractor for RustFallback {
    fn name(&self) -> &'static str {
        "rust-fallback"
    }

    fn probe(&self) -> EngineStatus {
        // 纯 Rust，无外部依赖——恒可用。
        EngineStatus::Ready
    }

    fn extract(&self, path: &Path) -> Result<Vec<PageText>, EngineError> {
        let bytes = std::fs::read(path)
            .map_err(|e| EngineError::FileBroken {
                engine: self.name(),
                detail: format!("读取失败: {e}"),
            })?;
        let t = pdf_text::extract(&bytes).map_err(map_pdf_text_err)?;
        Ok(t
            .pages
            .iter()
            .enumerate()
            .map(|(pi, lines)| PageText {
                page: pi + 1,
                text: lines.join("\n"),
                confidence: 1.0,
                diagnostics: String::new(),
            })
            .collect())
    }
}

impl RustFallback {
    /// 整档解析（仲裁器采用兜底产物时的入口——委托既有 parse_via_fallback，
    /// summary 自报与块定位器口径零回归）。
    pub fn extract_parsed(&self, path: &Path, name: &str) -> Result<ParsedFile, String> {
        pdf_text::parse_via_fallback(path, name)
    }
}

/// PdfTextError → 引擎错误三分类（owner 约束：损坏/缺件/不支持不混谈）。
fn map_pdf_text_err(e: pdf_text::PdfTextError) -> EngineError {
    use pdf_text::PdfTextError;
    match e {
        PdfTextError::NotPdf => EngineError::FileBroken {
            engine: "rust-fallback",
            detail: "不是合法 PDF（缺 %PDF- 头）".to_string(),
        },
        PdfTextError::Encrypted => EngineError::Unsupported {
            engine: "rust-fallback",
            what: "加密 PDF——本模块不解密".to_string(),
        },
        PdfTextError::NoTextLayer => EngineError::Unsupported {
            engine: "rust-fallback",
            what: "无文本层（纯 Rust 兜底不做 OCR）".to_string(),
        },
        PdfTextError::Unsupported(why) => EngineError::Unsupported {
            engine: "rust-fallback",
            what: format!("流滤链不支持：{why}"),
        },
    }
}
