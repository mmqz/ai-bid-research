//! engines/tesseract.rs —— 扫描层 OCR 引擎（pdftoppm → tesseract chi_sim，
//! #200 R2 自 pdf.rs 迁入）。
//!
//! 行为口径与迁移前逐字段一致（零回归）：pdfinfo 真实页数探测、
//! MAX_OCR_PAGES 页数门（渲染前拒绝、不留部分结果）、TmpGuard RAII 回收
//! 渲染 PNG、单页 OCR 失败诚实降级、全失败零块 fail-loud（#80 堵漏）、
//! summary「pdf 扫描层 OCR: N 页 M 行块（chi_sim）」。
//!
//! R2 新增：`probe()`（chi_sim 可用性，进程级缓存）——供仲裁器在
//! 「文本层乱码 OCR 救援」路径决定是否值得起渲染；**扫描件主路径不走
//! probe 门**（保持既有失败语义：环境缺件在逐页执行中暴露，消息不变）。

use crate::engine::{EngineError, EngineStatus, PageText, PdfExtractor};
use crate::guard::{
    self, RunOutcome, TmpGuard, LIMIT_TESSERACT_STDOUT, MAX_OCR_PAGES, TIMEOUT_PDFTOPPM,
    TIMEOUT_PDFTOTEXT, TIMEOUT_TESSERACT,
};
use crate::pdf_text::sanitize_line;
use crate::{Block, BlockKind, ParsedFile};
use std::path::Path;

/// 扫描层 OCR 引擎。
pub struct TesseractOcr;

impl PdfExtractor for TesseractOcr {
    fn name(&self) -> &'static str {
        "tesseract"
    }

    fn probe(&self) -> EngineStatus {
        match chi_sim_available() {
            Ok(true) => EngineStatus::Ready,
            Ok(false) => EngineStatus::Missing {
                what: "tesseract 已安装但缺 chi_sim 语言数据".to_string(),
                hint: "安装 tesseract-ocr-chi（或设 TESSDATA_PREFIX 指向含 chi_sim.traineddata 的目录）".to_string(),
            },
            Err(why) => EngineStatus::Missing {
                what: format!("tesseract 无法启动: {why}"),
                hint: "安装 tesseract-ocr 及 chi_sim 语言数据".to_string(),
            },
        }
    }

    fn extract(&self, path: &Path) -> Result<Vec<PageText>, EngineError> {
        let pf = self
            .ocr_parsed(path)
            .map_err(|detail| EngineError::FileBroken {
                engine: "tesseract",
                detail,
            })?;
        // 按页聚合 OCR 行块（OcrLine 的定位器含页号——page 从 locator 还原）
        let mut pages: Vec<PageText> = Vec::new();
        for b in &pf.blocks {
            let page = b
                .locator
                .split_once('页')
                .and_then(|(head, _)| head.trim_start_matches('第').parse::<usize>().ok())
                .unwrap_or(1);
            let text = b.text.clone();
            match pages.iter_mut().find(|p| p.page == page) {
                Some(p) => {
                    p.text.push('\n');
                    p.text.push_str(&text);
                }
                None => pages.push(PageText {
                    page,
                    text,
                    confidence: 1.0,
                    diagnostics: String::new(),
                }),
            }
        }
        Ok(pages)
    }
}

impl TesseractOcr {
    /// 扫描层整档解析（自旧 pdf::parse 无文本层分支逐字段迁入——零回归）。
    pub fn ocr_parsed(&self, path: &Path) -> Result<ParsedFile, String> {
        let mut blocks: Vec<Block> = Vec::new();
        let mut ocr_blocks = 0usize;
        let mut pages = 1usize; // pdftotext 分页口径的保守回退值（旧代码同款）
        let mut summary = String::new();
        // T-04：扫描页数门——超限整体显式拒绝（渲染/OCR 成本与磁盘占用上限；
        // 拒绝整个文件，绝不产出「只 OCR 了前 N 页」的部分结果）。
        // R3-S1-1 修正：真实页数以 pdfinfo 探测为准，渲染层另加 -l 上限双保险。
        let real_pages = probe_pdf_pages(path).unwrap_or(1);
        if real_pages > MAX_OCR_PAGES {
            return Err(format!(
                "pdf 扫描层 {real_pages} 页超过 OCR 上限 {MAX_OCR_PAGES}（拒绝处理——不留部分结果，请拆分文件）"
            ));
        }
        // T-04：TmpGuard RAII——超时被杀/任意 Err 提前返回路径均统一回收渲染 PNG
        let tmp = TmpGuard::new("ocr").map_err(|e| format!("临时目录创建失败: {e}"))?;
        let prefix = tmp.0.join("page");
        let mut render_cmd = std::process::Command::new("pdftoppm");
        render_cmd
            .args(["-r", "200", "-gray", "-png"])
            .args(["-l", &real_pages.to_string()]) // 渲染上限双保险（R3-S1-1）
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
            pages = pngs.len(); // 扫描件页数以实际渲染张数为准（诚实页数）
        }
        let mut ocr_fail_pages = 0usize; // NonZeroExit 页计（环境类失败与数据面残缺的分频面）
        for (pi, png) in pngs.iter().enumerate() {
            let tessdata =
                std::path::PathBuf::from(std::env::var("TESSDATA_PREFIX").unwrap_or_else(|_| {
                    format!("{}/.tessdata", std::env::var("HOME").unwrap_or_default())
                }));
            let mut t_cmd = std::process::Command::new("tesseract");
            t_cmd
                .arg(png.as_os_str())
                .arg("stdout")
                .args(["-l", "chi_sim", "--psm", "6"])
                .env("TESSDATA_PREFIX", &tessdata);
            let ocr = guard::run_limited(t_cmd, TIMEOUT_TESSERACT, LIMIT_TESSERACT_STDOUT);
            let page_text = match ocr {
                RunOutcome::Success { stdout } => stdout,
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
            };
            let mut line_no = 0usize;
            for line in page_text.lines() {
                let l = line.trim();
                let cleaned = sanitize_line(l);
                if cleaned.chars().filter(|c| !c.is_whitespace()).count() < 2 {
                    continue; // 碎屑噪声行或控制字符清洗后的空行
                }
                line_no += 1;
                ocr_blocks += 1;
                blocks.push(Block {
                    idx: blocks.len(),
                    kind: BlockKind::OcrLine,
                    locator: format!("第{}页 OCR 行{}", pi + 1, line_no),
                    text: super::poppler::collapse_spaces(&cleaned),
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
        Ok(ParsedFile {
            name: String::new(),
            sha256: String::new(),
            pages,
            tables: 0,
            has_text_layer: false,
            ocr_blocks,
            blocks,
            parse_summary: summary,
            components: vec![],
        })
    }
}

/// chi_sim 可用性（进程级缓存——`tesseract --list-langs` 一次探测）。
///
/// 探测命令与 OCR 执行同款 TESSDATA_PREFIX 解析口径（自定义目录不漏判）。
fn chi_sim_available() -> Result<bool, String> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Result<bool, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let tessdata =
                std::path::PathBuf::from(std::env::var("TESSDATA_PREFIX").unwrap_or_else(|_| {
                    format!("{}/.tessdata", std::env::var("HOME").unwrap_or_default())
                }));
            let mut c = std::process::Command::new("tesseract");
            c.arg("--list-langs").env("TESSDATA_PREFIX", &tessdata);
            match guard::run_limited(c, TIMEOUT_TESSERACT, 64 * 1024) {
                RunOutcome::Success { stdout } => Ok(stdout.contains("chi_sim")),
                RunOutcome::Spawn { why, .. } => Err(why),
                _ => Ok(false), // 在位但异常退出：按「缺 chi_sim」处理（诚实可补）
            }
        })
        .clone()
}

/// 治理失败短描述（自 pdf.rs 迁入，未改动）。
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
/// 探测失败回退保守值 1（渲染层 -l 上限双保险兜底）。
fn probe_pdf_pages(path: &Path) -> Option<usize> {
    let mut c = std::process::Command::new("pdfinfo");
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
    use crate::guard::{
        LIMIT_PDFTOTEXT_STDOUT, MAX_OCR_PAGES, TIMEOUT_PDFTOPPM, TIMEOUT_PDFTOTEXT,
        TIMEOUT_TESSERACT,
    };

    /// 治理参数语义测试（不依赖外部 PDF 工具——CI 环境兼容）：
    /// 超时/溢出上限值是契约的一部分（变更需过组件版本矩阵审查）。
    /// （自 pdf.rs 迁入——零回归钉子）
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
    /// （自 pdf.rs 迁入——零回归钉子；parse 换为 engines::parse_pdf）
    #[test]
    fn oversized_scanned_pdf_rejected_before_render() {
        // 持子进程测试锁（python3 生成 + parse_pdf 均占并发席——#200 R2 竞态治本）
        let _g = crate::guard::SUBPROC_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
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
                let r = super::super::parse_pdf(&pdf, "oversize_201p.pdf");
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
