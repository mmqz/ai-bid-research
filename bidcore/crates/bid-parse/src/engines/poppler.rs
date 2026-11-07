//! engines/poppler.rs —— poppler pdftotext 文本层引擎（#200 R2 自 pdf.rs 迁入）。
//!
//! 行为口径与迁移前逐字段一致（零回归）：`pdftotext -layout` 受治执行、
//! `\f` 分页、行块 `PdfLine` + 定位器「第X页 行Y」、summary
//! 「pdf 文本层: N 页 M 行块」。文本层/扫描层的路由裁决（非空白字符
//! ≥16 判有文本层）归 [`super::parse_pdf`] 仲裁器，不在本引擎内。

use crate::engine::{EngineError, EngineStatus, PageText, PdfExtractor};
use crate::guard::{self, RunOutcome, LIMIT_PDFTOTEXT_STDOUT, TIMEOUT_PDFTOTEXT};
use crate::pdf_text::sanitize_line;
use crate::{Block, BlockKind, ParsedFile};
use std::path::Path;

use super::tool_abs_path;

/// poppler pdftotext 文本层引擎。
pub struct PopplerText;

impl PdfExtractor for PopplerText {
    fn name(&self) -> &'static str {
        "poppler"
    }

    fn probe(&self) -> EngineStatus {
        // pdftotext -v 退出码非 0 但能打印版本即视为在位（poppler 惯例）；
        // 只有「无法启动」才算缺件。
        let mut c = std::process::Command::new("pdftotext");
        c.arg("-v");
        match guard::run_limited(c, TIMEOUT_PDFTOTEXT, 64 * 1024) {
            RunOutcome::Spawn { why, .. } => EngineStatus::Missing {
                what: format!("pdftotext 无法启动: {why}"),
                hint: "安装 poppler-utils（Debian/Ubuntu: apt install poppler-utils）".to_string(),
            },
            _ => EngineStatus::Ready,
        }
    }

    fn extract(&self, path: &Path) -> Result<Vec<PageText>, EngineError> {
        let mut text_cmd = std::process::Command::new("pdftotext");
        // B-25：路径经 tool_abs_path 绝对化（纵深防御——见 engines/mod.rs）
        text_cmd.arg("-layout").arg(tool_abs_path(path)).arg("-");
        let text = guard::run_limited(text_cmd, TIMEOUT_PDFTOTEXT, LIMIT_PDFTOTEXT_STDOUT)
            .into_result()
            .map_err(map_run_err)?;
        // \f 分页保持分段序（含空段——块定位器的页号口径依赖它）
        Ok(text
            .split('\u{c}')
            .enumerate()
            .map(|(pi, seg)| PageText {
                page: pi + 1,
                text: seg.to_string(),
                confidence: 1.0,
                diagnostics: String::new(),
            })
            .collect())
    }
}

/// 治理五分类 → 引擎错误三分类（消息文本与旧 pdf::parse 完全一致——零回归）。
fn map_run_err(detail: String) -> EngineError {
    // 「无法启动」= 环境缺件；超时/溢出/满载 = 治理拒绝；非零退出 = 文件面
    if detail.contains("无法启动") {
        EngineError::EnvMissing {
            engine: "poppler",
            detail,
            hint: "安装 poppler-utils".to_string(),
        }
    } else if detail.contains("退出码") {
        EngineError::FileBroken {
            engine: "poppler",
            detail,
        }
    } else {
        EngineError::Governed {
            engine: "poppler",
            detail,
        }
    }
}

/// 文本层行块装配（自旧 pdf::parse 文本层分支逐字段迁入——零回归）。
pub(super) fn text_layer_parsed(pages: &[PageText]) -> ParsedFile {
    let mut blocks: Vec<Block> = Vec::new();
    // #324（R-12/B-04）：表格行计数在折叠前判定——原实现先 collapse_spaces
    // 再 filter(text.contains("  "))，折叠后双空格恒假 → tables 恒 0（死指标）。
    // 判定移入行循环、折叠前（cleaned 保留了 -layout 列对齐多空格）。
    let mut tables = 0usize;
    // 页数口径：非空分段数（pdftotext 末页后常带尾随 \f）
    let page_count = pages
        .iter()
        .filter(|p| !p.text.trim().is_empty())
        .count()
        .max(1);
    for pt in pages {
        let mut line_no = 0usize;
        for line in pt.text.lines() {
            let l = line.trim();
            let cleaned = sanitize_line(l);
            if cleaned.is_empty() {
                continue;
            }
            line_no += 1;
            // 折叠前判定（本函数内块全为 PdfLine，kind 过滤等价省略）
            if cleaned.contains("  ") {
                tables += 1;
            }
            blocks.push(Block {
                idx: blocks.len(),
                kind: BlockKind::PdfLine,
                locator: format!("第{}页 行{}", pt.page, line_no),
                text: collapse_spaces(&cleaned),
                cells: vec![],
            });
        }
    }
    let n_blocks = blocks.len();
    ParsedFile {
        name: String::new(),
        sha256: String::new(),
        pages: page_count,
        tables,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary: format!("pdf 文本层: {page_count} 页 {n_blocks} 行块"),
        components: vec![],
    }
}

/// 空白折叠（自 pdf.rs 迁入，未改动）。
pub(super) fn collapse_spaces(s: &str) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim_end().to_string()
}

/// 真实页数探测（pdfinfo——poppler 家族工具，自 tesseract.rs 收编为单一真源，
/// #424 混合路由的尾随空段判伪也消费）。
///
/// 探测失败返回 None（调用方各自决策：整档 OCR 路径走门值回退，
/// 混合路由保守不剔除尾随空段）。
pub(super) fn probe_pdf_pages(path: &Path) -> Option<usize> {
    let mut c = std::process::Command::new("pdfinfo");
    // B-25：路径经 tool_abs_path 绝对化（纵深防御——见 engines/mod.rs）；
    // #424 自 tesseract.rs 收编为单一真源时保留该加固
    c.arg(tool_abs_path(path));
    let out = guard::run_limited(c, TIMEOUT_PDFTOTEXT, 64 * 1024);
    if !out.is_success() {
        return None;
    }
    if let RunOutcome::Success { stdout } = out {
        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix("Pages:") {
                if let Ok(n) = rest.trim().parse::<usize>() {
                    return Some(n);
                }
            }
        }
    }
    None
}
