//! PDF 解析：文本层（poppler pdftotext）+ 扫描层（pdftoppm → tesseract chi_sim）。
//! 无文本层 → OCR 通道（`has_text_layer=false` + OcrLine 块）；两层皆空 → 诚实留空并说明。
//!
//! T-04 子进程治理（guard::run_limited）：三外部命令全部受治——超时杀进程组、
//! stdout 上限、结果五分类（超时/溢出/非零/Busy/Spawn 均**不误标成功**）；
//! 临时 PNG 区 TmpGuard RAII 统一回收；扫描页数超 [`MAX_OCR_PAGES`] 整体显式
//! 拒绝（不留可被误认为完整 OCR 的部分结果）。

use crate::guard::{
    self, RunOutcome, TmpGuard, LIMIT_INPUT_BYTES_PDF, LIMIT_PDFTOTEXT_STDOUT,
    LIMIT_TESSERACT_STDOUT, MAX_OCR_PAGES, TIMEOUT_PDFTOPPM, TIMEOUT_PDFTOTEXT, TIMEOUT_TESSERACT,
};
use crate::{Block, BlockKind, ParsedFile};
use crate::pdf_text::sanitize_line;
use std::path::Path;
use std::process::Command;

pub fn parse(path: &Path) -> Result<ParsedFile, String> {
    // R3-S3-2：PDF 输入字节门（docx/xlsx 有门而 PDF 曾无门——与 guard 家族对齐）
    guard::check_input_size(path, LIMIT_INPUT_BYTES_PDF)
        .map_err(|e| format!("pdf 防护拒绝: {e}"))?;
    let mut text_cmd = Command::new("pdftotext");
    text_cmd.arg("-layout").arg(path.as_os_str()).arg("-");
    let text =
        guard::run_limited(text_cmd, TIMEOUT_PDFTOTEXT, LIMIT_PDFTOTEXT_STDOUT).into_result()?;
    // pdftotext 以 \f 分页且末页后常带尾随 \f——按「非空分片」计页
    let pages = text
        .split('\u{c}')
        .filter(|seg| !seg.trim().is_empty())
        .count()
        .max(1);
    let non_ws: usize = text.chars().filter(|c| !c.is_whitespace()).count();

    let mut blocks: Vec<Block> = Vec::new();
    let mut has_text_layer = non_ws >= 16; // 空白/仅页码层判为无文本层
    let mut ocr_blocks = 0usize;
    let mut summary = String::new();
    // pages 口径：文本层 = 非空分页数；扫描层 = 实际渲染页数（P2 真实文件驱动修正：
    // 多页扫描件的 pdftotext 空文本不得把页数错报为 1——合成夹具 B2 单页曾掩盖）
    let mut pages = pages;

    if has_text_layer {
        for (pi, page) in text.split('\u{c}').enumerate() {
            let mut line_no = 0usize;
            for line in page.lines() {
                let l = line.trim();
                if l.is_empty() {
                    continue;
                }
                line_no += 1;
                // #223 对称防护：首选路同款行级控制字符清洗（pdftotext 经
                // from_utf8_lossy 极少产出 C1 控制符，加了零成本兜住）
                blocks.push(Block {
                    idx: blocks.len(),
                    kind: BlockKind::PdfLine,
                    locator: format!("第{}页 行{}", pi + 1, line_no),
                    text: collapse_spaces(&sanitize_line(l)),
                    cells: vec![],
                });
            }
        }
        summary = format!("pdf 文本层: {pages} 页 {} 行块", blocks.len());
    } else {
        // T-04：扫描页数门——超限整体显式拒绝（渲染/OCR 成本与磁盘占用上限；
        // 拒绝整个文件，绝不产出「只 OCR 了前 N 页」的部分结果）。
        // R3-S1-1 修正：pdftotext 对扫描件输出近空文本，非空分页数恒为 1——
        // 门不得依赖该值；真实页数以 pdfinfo 探测为准，渲染层另加 -l 上限双保险。
        let real_pages = probe_pdf_pages(path).unwrap_or(pages);
        if real_pages > MAX_OCR_PAGES {
            return Err(format!(
                "pdf 扫描层 {real_pages} 页超过 OCR 上限 {MAX_OCR_PAGES}（拒绝处理——不留部分结果，请拆分文件）"
            ));
        }
        has_text_layer = false;
        // T-04：TmpGuard RAII——超时被杀/任意 Err 提前返回路径均统一回收渲染 PNG
        let tmp = TmpGuard::new("ocr").map_err(|e| format!("临时目录创建失败: {e}"))?;
        let prefix = tmp.0.join("page");
        let mut render_cmd = Command::new("pdftoppm");
        render_cmd
            .args(["-r", "200", "-gray", "-png"])
            .args(["-l", &real_pages.to_string()]) // 渲染上限双保险（R3-S1-1：探测与实际不一致时仍不超渲染）
            .arg(path.as_os_str())
            .arg(&prefix);
        let render = guard::run_limited(render_cmd, TIMEOUT_PDFTOPPM, 4096);
        if !render.is_success() {
            return Err(format!("pdftoppm 失败: {}", short_err(&render)));
        }
        let mut pngs: Vec<std::path::PathBuf> = std::fs::read_dir(&tmp.0)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("png"))
            .collect();
        pngs.sort();
        if !pngs.is_empty() {
            pages = pngs.len(); // 扫描件页数以实际渲染张数为准（诚实页数，非文本层猜测）
        }
        let mut ocr_fail_pages = 0usize; // NonZeroExit 页计（环境类失败与数据面残缺的分频面）
        for (pi, png) in pngs.iter().enumerate() {
            let tessdata =
                std::path::PathBuf::from(std::env::var("TESSDATA_PREFIX").unwrap_or_else(|_| {
                    format!("{}/.tessdata", std::env::var("HOME").unwrap_or_default())
                }));
            let mut t_cmd = Command::new("tesseract");
            t_cmd
                .arg(png.as_os_str())
                .arg("stdout")
                .args(["-l", "chi_sim", "--psm", "6"])
                .env("TESSDATA_PREFIX", &tessdata);
            let ocr = guard::run_limited(t_cmd, TIMEOUT_TESSERACT, LIMIT_TESSERACT_STDOUT);
            if !ocr.is_success() {
                // 单页 OCR 失败：诚实降级说明（不静默、不中断整档——扫描页残缺属
                // 数据面，parse_summary 承载原因）；超时/溢出类治理失败向上传播拒绝
                match ocr {
                    RunOutcome::Timeout { .. }
                    | RunOutcome::OutputOverflow { .. }
                    | RunOutcome::Busy { .. }
                    | RunOutcome::Spawn { .. } => {
                        return Err(format!("tesseract 治理失败: {}", short_err(&ocr)));
                    }
                    RunOutcome::NonZeroExit { stderr_head, .. } => {
                        ocr_fail_pages += 1;
                        summary = format!("pdf 扫描层: {pages} 页，OCR 失败（{stderr_head}）");
                        break;
                    }
                    RunOutcome::Success { .. } => unreachable!(),
                }
            }
            let page_text = match ocr {
                RunOutcome::Success { stdout } => stdout,
                _ => unreachable!(),
            };
            let mut line_no = 0usize;
            for line in page_text.lines() {
                let l = line.trim();
                if l.chars().filter(|c| !c.is_whitespace()).count() < 2 {
                    continue; // 碎屑噪声行
                }
                line_no += 1;
                ocr_blocks += 1;
                blocks.push(Block {
                    idx: blocks.len(),
                    kind: BlockKind::OcrLine,
                    locator: format!("第{}页 OCR 行{}", pi + 1, line_no),
                    text: collapse_spaces(l),
                    cells: vec![],
                });
            }
        }
        if summary.is_empty() {
            summary = format!("pdf 扫描层 OCR: {pages} 页，{ocr_blocks} 行块（chi_sim）");
        }
        // 堵漏（#80 第三批实测发现·环境 C：tesseract 在位而 chi_sim 缺失）：
        // NonZeroExit 走「诚实降级」后，扫描件以 0 块【静默导入成功】——内容
        // 整档丢失却无任何失败信号（issue #9「环境缺失伪装成业务行为」的变体）。
        // 规则：OCR 页失败且零行块 = 无可用产物 → 解析失败（E_PARTIAL 让调用方
        // 看见）；部分页成功（ocr_blocks > 0）仍走降级（扫描页残缺属数据面，
        // 语义不变）；全页 OCR 成功但空文本（白扫）不在此列——那是诚实空。
        if ocr_fail_pages > 0 && ocr_blocks == 0 {
            return Err(format!(
                "pdf 扫描层 OCR 失败（{ocr_fail_pages} 页报错、零行块产出）——\
                 疑似 OCR 环境缺失：检查 tesseract 与 chi_sim 语言数据\
                 （TESSDATA_PREFIX 或 ~/.tessdata）；本文件按失败处理，不留空内容伪装成功"
            ));
        }
    }

    let tables = blocks
        .iter()
        .filter(|b| b.kind == BlockKind::PdfLine && b.text.contains("  "))
        .count(); // -layout 下列对齐的多列表征（粗粒度——PDF 表格结构归 D1 面）
    Ok(ParsedFile {
        name: String::new(),
        sha256: String::new(),
        pages,
        tables,
        has_text_layer,
        ocr_blocks,
        blocks,
        parse_summary: summary,
        components: vec![], // SC-02 部件账本属 docx 解析面（PDF 无 zip 部件结构）
    })
}

fn short_err(out: &RunOutcome) -> String {
    match out {
        RunOutcome::Timeout { program, secs } => format!("子进程超时被杀（{program} {secs}s）"),
        RunOutcome::OutputOverflow { program, limit } => {
            format!("输出超限（{program} > {limit} 字节）")
        }
        RunOutcome::NonZeroExit {
            program,
            code,
            stderr_head,
        } => {
            format!("{program} 退出码 {code:?}: {stderr_head}")
        }
        RunOutcome::Busy { program } => format!("并发满载（{program}）"),
        RunOutcome::Spawn { program, why } => format!("{program} 无法启动: {why}"),
        RunOutcome::Success { .. } => "成功".to_string(),
    }
}

/// 真实页数探测（pdfinfo，poppler 自带）——扫描件页数的诚实口径；
/// 探测失败回退 pdftotext 分页数（保守场景由渲染 -l 上限兕底）。
fn probe_pdf_pages(path: &Path) -> Option<usize> {
    let mut c = Command::new("pdfinfo");
    c.arg(path.as_os_str());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 治理参数语义测试（不依赖外部 PDF 工具——CI 环境兼容）：
    /// 超时/溢出上限值是契约的一部分（变更需过组件版本矩阵审查）。
    #[test]
    fn governance_params_are_pinned() {
        use std::time::Duration;
        assert_eq!(TIMEOUT_PDFTOTEXT, Duration::from_secs(60));
        assert_eq!(TIMEOUT_PDFTOPPM, Duration::from_secs(180));
        assert_eq!(TIMEOUT_TESSERACT, Duration::from_secs(30));
        assert_eq!(LIMIT_PDFTOTEXT_STDOUT, 64 * 1024 * 1024);
        assert_eq!(LIMIT_TESSERACT_STDOUT, 1024 * 1024);
        assert_eq!(MAX_OCR_PAGES, 200);
    }

    /// R3-S1-1 回归门：超页扫描件必须被页数门拒绝（绝不开始渲染）。
    /// 201 页空白扫描 PDF 由 python3+reportlab 现场生成（环境无 reportlab 则跳过）。
    /// 生成器调用受治（PR #24 修复批次）：`run_limited` 30s 超时——旧 `.output()`
    /// 无限期等待外部进程（Lead 指令：所有等外部进程的地方必须有超时）。
    #[test]
    fn oversized_scanned_pdf_rejected_before_render() {
        let tmp = std::env::temp_dir().join(format!("bidparse_ocr_limit_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let pdf = tmp.join("oversize_201p.pdf");
        let mut gen = std::process::Command::new("python3");
        gen.arg("-c")
            .arg(
                "from reportlab.pdfgen import canvas; \
                 import sys; \
                 c = canvas.Canvas(sys.argv[1]); \
                 [c.showPage() for _ in range(201)]; \
                 c.save()",
            )
            .arg(&pdf);
        let gen = guard::run_limited(gen, std::time::Duration::from_secs(30), 4096);
        match gen {
            RunOutcome::Success { .. } => {
                let started = std::time::Instant::now();
                let r = parse(&pdf);
                match r {
                    Err(e) => {
                        assert!(e.contains("超过 OCR 上限"), "必须是页数门拒绝，实际 {e}");
                        // 门在渲染前生效：不得耗 OCR 时间（201 页 OCR 需数百秒）
                        assert!(
                            started.elapsed().as_secs() < 30,
                            "拒绝必须在渲染前返回，实际 {:?}",
                            started.elapsed()
                        );
                    }
                    Ok(_) => panic!("201 页扫描 PDF 必须被拒绝（R3-S1-1 门失效）"),
                }
            }
            _ => eprintln!("跳过：环境无 python3+reportlab（生成器缺件）"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

fn collapse_spaces(s: &str) -> String {
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
