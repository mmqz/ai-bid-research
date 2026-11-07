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
    TIMEOUT_TESSERACT,
};
use crate::pdf_text::sanitize_line;
use crate::{Block, BlockKind, ParsedFile};
use std::path::Path;

use super::tool_abs_path;

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
        let mut failed_stderr = String::new(); // #380（B-09）：末次失败页 stderr 摘要（诊断面）
        // T-04：扫描页数门——超限整体显式拒绝（渲染/OCR 成本与磁盘占用上限；
        // 拒绝整个文件，绝不产出「只 OCR 了前 N 页」的部分结果）。
        // R3-S1-1 修正：真实页数以 pdfinfo 探测为准，渲染层另加 -l 上限双保险。
        // #380（B-10）：探测失败不再静默回退 1——那是「整件只 OCR 第 1 页」
        // 的大面积静默内容丢失（pdfinfo 对部分损坏件失败而 pdftoppm 可渲染；
        // 或 pdfinfo 缺失的畸形部署）。回退门值 MAX_OCR_PAGES + probe_failed
        // 标记（summary 登记 + 渲染后饱和校验）。
        let (real_pages, probe_failed) = render_page_limit(probe_pdf_pages(path));
        if !probe_failed && real_pages > MAX_OCR_PAGES {
            return Err(format!(
                "pdf 扫描层 {real_pages} 页超过 OCR 上限 {MAX_OCR_PAGES}（拒绝处理——不留部分结果，请拆分文件）"
            ));
        }
        // T-04：TmpGuard RAII——超时被杀/任意 Err 提前返回路径均统一回收渲染 PNG
        let tmp = TmpGuard::new("ocr").map_err(|e| format!("临时目录创建失败: {e}"))?;
        let prefix = tmp.0.join("page");
        let mut render_cmd = std::process::Command::new("pdftoppm");
        // B-25：路径经 tool_abs_path 绝对化（纵深防御——见 engines/mod.rs）
        render_cmd
            .args(["-r", "200", "-gray", "-png"])
            .args(["-l", &real_pages.to_string()]) // 渲染上限双保险（R3-S1-1）
            .arg(tool_abs_path(path))
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
        // #380（B-10）渲染后饱和校验：探测失败时真实页数未知——渲染张数
        // 顶到门值 = 可能已被 -l 截断，按页数门语义拒绝（不留部分结果，
        // 与 R3-S1-1 预拒门同口径；探测成功路径的预拒已在上面生效）。
        if probe_failed && pngs.len() >= MAX_OCR_PAGES {
            return Err(format!(
                "pdf 扫描层渲染 {} 页达 OCR 上限 {MAX_OCR_PAGES} 且页数探测失败（pdfinfo 退出/解析失败——真实页数未知，可能已截断）不留部分结果：请修复 pdfinfo 部署或拆分文件",
                pngs.len()
            ));
        }
        let mut ocr_fail_pages = 0usize; // NonZeroExit 页计（环境类失败与数据面残缺的分频面）
        let mut failed_pages: Vec<usize> = Vec::new(); // #380（B-09）：失败页号清单（截断范围可见）
        for (pi, png) in pngs.iter().enumerate() {
            let tessdata =
                std::path::PathBuf::from(std::env::var("TESSDATA_PREFIX").unwrap_or_else(|_| {
                    format!("{}/.tessdata", std::env::var("HOME").unwrap_or_default())
                }));
            let mut t_cmd = std::process::Command::new("tesseract");
            // B-25：png 虽由 TmpGuard 绝对生成，仍统一走 tool_abs_path
            // （统一关闭注入面——防未来重构换相对路径时复活）。
            t_cmd
                .arg(tool_abs_path(png))
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
                RunOutcome::NonZeroExit { mut stderr_head, .. } => {
                    // #380（B-09）：单页失败跳过该页、**继续后续页**——旧实现
                    // break 导致第 N 页失败后其后所有页不再 OCR（100 页件第 5
                    // 页失败 = 95 页静默丢失，远超「该页残缺」的数据面语义）。
                    // 失败页号入清单，截断范围进 summary（部分成功可见）。
                    ocr_fail_pages += 1;
                    failed_pages.push(pi + 1);
                    if stderr_head.trim().len() > 80 {
                        stderr_head.truncate(80);
                    }
                    failed_stderr = stderr_head;
                    continue;
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
        // #380（B-09/B-10）可见性组装：探测失败登记 + 失败页清单（页号 +
        // 成功页数 + 行为说明）。全成功时维持旧文案（零回归）。
        if probe_failed {
            summary.push_str(&format!(
                "pdfinfo 页数探测失败——渲染上限 {MAX_OCR_PAGES} 页，实际渲染 {pages} 页；"
            ));
        }
        if !failed_pages.is_empty() {
            summary.push_str(&ocr_partial_failure_summary(&failed_pages, pages));
            if !failed_stderr.is_empty() {
                summary.push_str(&format!("（末次 stderr：{failed_stderr}）"));
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
/// 渲染上限决策（#380 B-10）：pdfinfo 探测成功 = 真实页数（超限由调用方
/// 预拒——R3-S1-1 门不变）；探测失败 = `MAX_OCR_PAGES` 门值（**不回退 1**——
/// 旧 `unwrap_or(1)` 是「整件只渲染+OCR 第 1 页」的静默内容丢失），返回
/// `probe_failed` 标记供 summary 登记与渲染后饱和校验。
fn render_page_limit(probe: Option<usize>) -> (usize, bool) {
    match probe {
        Some(n) => (n, false),
        None => (MAX_OCR_PAGES, true),
    }
}

/// 部分页失败的 summary 组装（#380 B-09：截断范围可见——失败页号清单 +
/// 成功页数 + 行为说明「失败页跳过、后续页照常 OCR」）。
fn ocr_partial_failure_summary(failed: &[usize], total_pages: usize) -> String {
    format!(
        "pdf 扫描层: {total_pages} 页，OCR 失败页 [{}]（{} 页成功——失败页跳过、后续页照常 OCR）",
        failed
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", "),
        total_pages.saturating_sub(failed.len()),
    )
}

fn probe_pdf_pages(path: &Path) -> Option<usize> {
    // 单一真源收编至 poppler.rs（#424）；B-25 绝对化加固在收编后的真源处生效
    super::poppler::probe_pdf_pages(path)
}

/// 混合件扫描页子集 OCR 的结果（#424）。
///
/// `Done`：OCR 通道可用并执行完毕（部分页失败同整档路径的诚实降级——
/// 失败页号清单在场，不静默）；`Skipped`：OCR 缺件/渲染失败——
/// 调用方（混合路由）保留文本页产物并在 summary 显式登记跳过面
/// （不静默丢页，与 #415/#416 诚实空纪律同源）。
pub(super) enum MixedOcrOutcome {
    Done {
        blocks: Vec<Block>,
        /// 识别失败页号（真实页号）
        failed_pages: Vec<usize>,
        /// 末次失败 stderr 摘要（诊断面）
        stderr_note: Option<String>,
    },
    Skipped {
        reason: String,
        hint: String,
    },
}

impl TesseractOcr {
    /// 混合件的扫描页子集识别（#424）：只渲染/识别指定页
    /// （`pdftoppm -f p -l p`），产出 OcrLine blocks（locator`第{p}页 OCR 行{i}`
    /// ——页号为真实页号，与整档路径同定位器口径）。
    ///
    /// 页数门 = 扫描页数（OCR 预算语义不变）；OCR 缺件 → `Skipped`
    /// （文本页产物保留，跳过面由调用方登记——混合件与整档路径的差异点：
    /// 整档 OCR 缺件是整件失败，混合件有一半真实内容不该陪葬）。
    /// 单页失败跳过续跑（#380 B-09 同款纪律）。
    pub fn ocr_pages(&self, path: &Path, page_nos: &[usize]) -> MixedOcrOutcome {
        if page_nos.len() > MAX_OCR_PAGES {
            return MixedOcrOutcome::Skipped {
                reason: format!(
                    "扫描页 {} 页超过 OCR 上限 {MAX_OCR_PAGES}（文本页已保留，扫描页未解析）",
                    page_nos.len()
                ),
                hint: "拆分文件或调大 MAX_OCR_PAGES 后重导入".to_string(),
            };
        }
        // 缺件预检（probe 缓存进程级）：缺 tesseract/chi_sim 不起渲染进程
        match self.probe() {
            crate::engine::EngineStatus::Missing { what, hint } => {
                return MixedOcrOutcome::Skipped { reason: what, hint }
            }
            crate::engine::EngineStatus::Ready => {}
            crate::engine::EngineStatus::Unsupported { what } => {
                return MixedOcrOutcome::Skipped {
                    reason: what,
                    hint: "扫描页需要 OCR 通道——检查 tesseract 部署".to_string(),
                }
            }
        }
        let tmp = match TmpGuard::new("ocr-mixed") {
            Ok(t) => t,
            Err(e) => {
                return MixedOcrOutcome::Skipped {
                    reason: format!("临时目录创建失败: {e}"),
                    hint: "检查 TMPDIR 磁盘与权限".to_string(),
                }
            }
        };
        let mut blocks: Vec<Block> = Vec::new();
        let mut failed_pages: Vec<usize> = Vec::new();
        let mut stderr_note: Option<String> = None;
        for &p in page_nos {
            // 每页独立子目录：pdftoppm 输出名含页序号填充，隔离后免名字碰撞
            let dir = tmp.0.join(format!("p{p}"));
            if std::fs::create_dir_all(&dir).is_err() {
                failed_pages.push(p);
                continue;
            }
            let prefix = dir.join("page");
            let mut render_cmd = std::process::Command::new("pdftoppm");
            // B-25：路径经 tool_abs_path 绝对化（纵深防御——与整档 OCR 路径同款）
            render_cmd
                .args(["-r", "200", "-gray", "-png"])
                .args(["-f", &p.to_string(), "-l", &p.to_string()])
                .arg(tool_abs_path(path))
                .arg(&prefix);
            let render = guard::run_limited(render_cmd, TIMEOUT_PDFTOPPM, 4096);
            if !render.is_success() {
                failed_pages.push(p);
                stderr_note = Some(short_err(&render));
                continue;
            }
            let png = std::fs::read_dir(&dir)
                .ok()
                .and_then(|entries| {
                    let mut pngs: Vec<std::path::PathBuf> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|q| q.extension().and_then(|x| x.to_str()) == Some("png"))
                        .collect();
                    pngs.sort();
                    pngs.into_iter().next()
                });
            let Some(png) = png else {
                // 页号超出真实页数（如尾随 \f 空段且 pdfinfo 探测失败的保守路径）
                // ——无真实内容可丢，诚实计为无产物页
                failed_pages.push(p);
                continue;
            };
            let tessdata =
                std::path::PathBuf::from(std::env::var("TESSDATA_PREFIX").unwrap_or_else(|_| {
                    format!("{}/.tessdata", std::env::var("HOME").unwrap_or_default())
                }));
            let mut t_cmd = std::process::Command::new("tesseract");
            // B-25：png 虽由 TmpGuard 绝对生成，仍统一走 tool_abs_path
            t_cmd
                .arg(tool_abs_path(&png))
                .arg("stdout")
                .args(["-l", "chi_sim", "--psm", "6"])
                .env("TESSDATA_PREFIX", &tessdata);
            let ocr = guard::run_limited(t_cmd, TIMEOUT_TESSERACT, LIMIT_TESSERACT_STDOUT);
            let page_text = match ocr {
                RunOutcome::Success { stdout } => stdout,
                RunOutcome::NonZeroExit { mut stderr_head, .. } => {
                    failed_pages.push(p);
                    if stderr_head.trim().len() > 80 {
                        stderr_head.truncate(80);
                    }
                    stderr_note = Some(stderr_head);
                    continue;
                }
                other => {
                    failed_pages.push(p);
                    stderr_note = Some(short_err(&other));
                    continue;
                }
            };
            let mut line_no = 0usize;
            for line in page_text.lines() {
                let l = line.trim();
                let cleaned = sanitize_line(l);
                if cleaned.chars().filter(|c| !c.is_whitespace()).count() < 2 {
                    continue;
                }
                line_no += 1;
                blocks.push(Block {
                    idx: 0, // 调用方（混合路由）按页序统一重编
                    kind: BlockKind::OcrLine,
                    locator: format!("第{p}页 OCR 行{line_no}"),
                    text: super::poppler::collapse_spaces(&cleaned),
                    cells: vec![],
                });
            }
        }
        MixedOcrOutcome::Done {
            blocks,
            failed_pages,
            stderr_note,
        }
    }
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

    /// #380（B-10）：探测失败回退**门值**而非 1——回退 1 是整件只 OCR
    /// 第 1 页的静默内容丢失；探测成功 = 真实页数（预拒门语义不变）。
    #[test]
    fn render_page_limit_probe_failure_falls_back_to_gate_not_one() {
        assert_eq!(render_page_limit(Some(7)), (7, false));
        assert_eq!(render_page_limit(Some(MAX_OCR_PAGES)), (MAX_OCR_PAGES, false));
        let (limit, failed) = render_page_limit(None);
        assert_eq!((limit, failed), (MAX_OCR_PAGES, true));
        assert_ne!(limit, 1, "探测失败不得回退 1 页（B-10 静默丢失面）");
    }

    /// #380（B-09）：失败页 summary 必须列出页号清单与成功页数——
    /// 截断范围可见（旧文案只写「OCR 失败」，丢页范围不可见）。
    #[test]
    fn partial_failure_summary_lists_failed_pages_and_survivors() {
        let s = ocr_partial_failure_summary(&[5], 100);
        assert!(s.contains("[5]"), "失败页号清单须可见：{s}");
        assert!(s.contains("99 页成功"), "成功页数须可见：{s}");
        assert!(s.contains("后续页照常 OCR"), "行为说明须在场：{s}");
        assert!(s.contains("100 页"), "总页数须在场：{s}");
        // 多页失败：清单逗号分隔
        let s2 = ocr_partial_failure_summary(&[3, 7], 10);
        assert!(s2.contains("[3, 7]") && s2.contains("8 页成功"), "{s2}");
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
