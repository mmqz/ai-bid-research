//! engines/mod.rs —— PDF 引擎注册表 + 仲裁器（#200 R2）。
//!
//! ## 架构（owner issue #200 指令）
//!
//! ```text
//! parse_file(pdf) ─→ parse_pdf（本模块，仲裁器）
//!                      ├─ ① poppler 文本层（engines/poppler.rs）
//!                      ├─ ② tesseract 扫描层（engines/tesseract.rs）
//!                      └─ ③ rust-fallback 纯 Rust 兜底（engines/rust_fallback.rs）
//! ```
//!
//! ## 仲裁策略
//!
//! R2 落地 `FirstSuccess`（保持迁移前行为——零回归）+ 两条 **OCR 救援**
//! 路径（owner 点名的「最大即时收益」：文本层乱码时 OCR 也会跑）：
//!
//! 1. **文本层路由**：poppler 非空白字符 ≥16 → 文本层；否则扫描层 OCR
//!    （与旧 pdf::parse 同款门）。
//! 2. **乱码判决**（#169 防线 + #200 R1 修正，逻辑自 lib.rs 迁入未改）：
//!    文本层疑似乱码 → 兜底重解 → Clean 采信 / 一致互证 / 分叉 fail-loud；
//!    首选路不可用 → 兜底单路 → 低残渣采信 / 高残渣 fail-loud。
//! 3. **OCR 救援**（R2 新增）：上述两个 fail-loud 点先试 OCR——OCR 是
//!    **渲染面**独立声音（与前两路的文本面解码无关），chi_sim 就绪且
//!    OCR 文本过乱码探测 → 采信 OCR（summary 自报 `[pdf-ocr-recovered]`）；
//!    OCR 缺件/无产物 → 落回既有 fail-loud（消息不变——救援绝不弱化防线）。
//!
//! R3（CrossValidate 逐页对比）与 R4（Docling/MinerU 外部引擎）在此注册表
//! 上扩展，不动各引擎。

mod poppler;
mod rust_fallback;
mod tesseract;

use crate::engine::PdfExtractor;
use crate::guard::{self, LIMIT_INPUT_BYTES_PDF};
use crate::pdf_text::{self, CjkGarbleProbe};
use crate::ParsedFile;
use poppler::PopplerText;
use rust_fallback::RustFallback;
use std::path::Path;
use tesseract::TesseractOcr;

/// PDF 双路判决结果（#200 R1：取代旧 used_fallback 布尔——状态混用不可判读）。
///
/// 判决驱动 parse_summary 前缀与错误标签；测试断言绑定 [pdf-*] 标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PdfPathOutcome {
    /// 首选路输出通过乱码探测——直接采信。
    PrimaryVerified,
    /// 首选路不可用（外部工具缺失/失败），兜底顶上且通过探测。
    PrimaryUnavailableFallbackClean,
    /// 首选路疑似乱码，兜底重解后 CJK 正常——采信兜底（#169 降级链主路径）。
    PrimaryGarbledFallbackRecovered,
    /// 首选路疑似乱码，兜底输出与首选路**一致**——内容确无 CJK（字体声明
    /// 与内容语言不符的模板件），双路互证采信（#200 R1 修正：不得误杀）。
    PrimaryGarbledFallbackCorroborated,
    /// 首选路不可用，兜底单路输出疑似但残渣特征不成立（非 ASCII 占比低）
    /// ——按「内容确无 CJK」采信（#200 R1）。
    FallbackOnlyLowResidue,
    /// 双路皆疑且分叉，但 OCR（渲染面）重解出正常文本——采信 OCR（#200 R2）。
    OcrRecovered,
}

/// PDF 解析总入口（仲裁器）——lib.rs 的 parse_file 按扩展名路由到这里。
pub fn parse_pdf(path: &Path, name: &str) -> Result<ParsedFile, String> {
    // 输入字节门（与旧 pdf::parse 同位：外部工具执行前）
    guard::check_input_size(path, LIMIT_INPUT_BYTES_PDF)
        .map_err(|e| format!("pdf 防护拒绝: {e}"))?;
    let raw = std::fs::read(path).map_err(|e| format!("读取失败 {name}: {e}"))?;

    let poppler = PopplerText;
    let tess = TesseractOcr;
    let fallback = RustFallback;

    match poppler.extract(path) {
        Ok(pages) => {
            // 文本层路由门（与旧 pdf::parse 同款：非空白 <16 判无文本层）
            let non_ws: usize = pages
                .iter()
                .map(|p| p.text.chars().filter(|c| !c.is_whitespace()).count())
                .sum();
            if non_ws < 16 {
                // Poppler 可能在合法但它无法解码的 CID 文件上返回空文本且退出码为 0。
                // 先尝试纯 Rust 文本层，避免把可恢复的文本件误送入扫描 OCR；
                // 真正无文本（扫描件）仍进入原 OCR 路径。
                if let Ok(mut fb) = fallback.extract_parsed(path, name) {
                    let fb_text = pdf_text::parsed_text_of(&fb);
                    let fb_non_ws = fb_text.chars().filter(|c| !c.is_whitespace()).count();
                    if fb_non_ws >= 16 {
                        let probe =
                            pdf_text::probe_cjk_garble_pages(&raw, &pdf_text::parsed_pages_of(&fb));
                        match probe.verdict {
                            CjkGarbleProbe::Garbled => {
                                let residue = pdf_text::non_ascii_ratio(&fb_text);
                                if residue >= 0.3 {
                                    if let Some(ocr) = try_ocr_recovery(&tess, path, &raw, name) {
                                        return Ok(ocr);
                                    }
                                    let hint = pdf_text::cjk_garble_reason_hint(&raw)
                                        .map(|h| format!("。{h}"))
                                        .unwrap_or_default();
                                    return Err(format!(
                                        "[pdf-fallback-only-garbled] Poppler 文本层为空；纯 Rust 兜底单路输出疑似 CJK 乱码且非 ASCII 占比 {residue:.2}，无 OCR 可互证——拒绝静默产出垃圾文本（issue #169/#200）{hint}"
                                    ));
                                }
                                apply_outcome(&mut fb, PdfPathOutcome::FallbackOnlyLowResidue);
                            }
                            _ => apply_outcome(
                                &mut fb,
                                PdfPathOutcome::PrimaryUnavailableFallbackClean,
                            ),
                        }
                        append_page_diagnostics(&mut fb, &raw);
                        return Ok(fb);
                    }
                }
                // 纯 Rust 也无足够文本：按扫描件主路径执行 OCR，保留缺件/治理错误。
                return tess.ocr_parsed(path);
            }
            let primary = poppler::text_layer_parsed(&pages);
            let primary_text = pdf_text::parsed_text_of(&primary);
            let primary_probe =
                pdf_text::probe_cjk_garble_pages(&raw, &pdf_text::parsed_pages_of(&primary));
            let (mut pf, outcome) = match primary_probe.verdict {
                CjkGarbleProbe::Garbled => match fallback.extract_parsed(path, name) {
                    Ok(fb) => {
                        let fb_text = pdf_text::parsed_text_of(&fb);
                        let fb_probe =
                            pdf_text::probe_cjk_garble_pages(&raw, &pdf_text::parsed_pages_of(&fb));
                        match fb_probe.verdict {
                            // 兜底重解出 CJK——#169 降级链主路径
                            CjkGarbleProbe::Clean => {
                                (fb, PdfPathOutcome::PrimaryGarbledFallbackRecovered)
                            }
                            _ => {
                                // 兜底也无 CJK：双路对比（#200 R1 修正）
                                if pdf_text::texts_corroborate(&primary_text, &fb_text) {
                                    (fb, PdfPathOutcome::PrimaryGarbledFallbackCorroborated)
                                } else if let Some(ocr) = try_ocr_recovery(&tess, path, &raw, name)
                                {
                                    (ocr, PdfPathOutcome::OcrRecovered)
                                } else {
                                    let hint = pdf_text::cjk_garble_reason_hint(&raw)
                                        .map(|h| format!("。{h}"))
                                        .unwrap_or_default();
                                    return Err(format!(
                                        "[pdf-dual-garbled] pdf 两条路输出均为疑似 CJK 乱码且互不一致（PDF 使用 CID-CJK 字体而两路文本均无 CJK 且不相同）：{name}——拒绝静默产出垃圾文本（issue #169）{hint}"
                                    ));
                                }
                            }
                        }
                    }
                    Err(fb_err) => {
                        if let Some(ocr) = try_ocr_recovery(&tess, path, &raw, name) {
                            (ocr, PdfPathOutcome::OcrRecovered)
                        } else {
                            let hint = pdf_text::cjk_garble_reason_hint(&raw)
                                .map(|h| format!("。{h}"))
                                .unwrap_or_default();
                            return Err(format!(
                                "[pdf-primary-garbled-fallback-failed] pdf 首选路输出疑似 CJK 乱码（PDF 使用 CID-CJK 字体而文本无 CJK——典型为 poppler 缺 CMap 资源包），纯 Rust 兜底也失败：{fb_err}{hint}"
                            ));
                        }
                    }
                },
                _ => (primary, PdfPathOutcome::PrimaryVerified),
            };
            apply_outcome(&mut pf, outcome);
            append_page_diagnostics(&mut pf, &raw);
            Ok(pf)
        }
        Err(chain_err) => match fallback.extract_parsed(path, name) {
            Ok(fb) => {
                let fb_text = pdf_text::parsed_text_of(&fb);
                let fb_probe =
                    pdf_text::probe_cjk_garble_pages(&raw, &pdf_text::parsed_pages_of(&fb));
                let (mut pf, outcome) = match fb_probe.verdict {
                    CjkGarbleProbe::Garbled => {
                        // 单路疑似、无第二路可互证：看残渣特征（#200 R1）。
                        let residue = pdf_text::non_ascii_ratio(&fb_text);
                        if residue >= 0.3 {
                            if let Some(ocr) = try_ocr_recovery(&tess, path, &raw, name) {
                                (ocr, PdfPathOutcome::OcrRecovered)
                            } else {
                                let hint = pdf_text::cjk_garble_reason_hint(&raw)
                                    .map(|h| format!("。{h}"))
                                    .unwrap_or_default();
                                return Err(format!(
                                    "[pdf-fallback-only-garbled] pdf 首选路不可用（{chain_err}）；纯 Rust 兜底单路输出疑似 CJK 乱码且非 ASCII 占比 {residue:.2}（多字节误判残渣特征），无第二路可互证——拒绝静默产出垃圾文本（issue #169/#200）{hint}"
                                ));
                            }
                        } else {
                            (fb, PdfPathOutcome::FallbackOnlyLowResidue)
                        }
                    }
                    _ => (fb, PdfPathOutcome::PrimaryUnavailableFallbackClean),
                };
                apply_outcome(&mut pf, outcome);
                append_page_diagnostics(&mut pf, &raw);
                Ok(pf)
            }
            Err(fb_err) => Err(format!(
                "[pdf-both-failed] pdf 解析失败 {name}: {chain_err}；纯 Rust 兜底也失败：{fb_err}"
            )),
        },
    }
}

/// 判决 → summary 诚实自报（对账能区分走了哪条路、为何采信）。
fn apply_outcome(pf: &mut ParsedFile, outcome: PdfPathOutcome) {
    match outcome {
        PdfPathOutcome::PrimaryVerified | PdfPathOutcome::PrimaryUnavailableFallbackClean => {
            // 首选路自报「pdf 文本层: …」；兜底自报「纯 Rust PDF 文本
            // 兜底：…」——两者本就诚实，不再叠加前缀（零回归口径）。
        }
        PdfPathOutcome::PrimaryGarbledFallbackRecovered => {
            pf.parse_summary = format!(
                "首选路输出疑似 CJK 乱码已降级（#169）；{}",
                pf.parse_summary
            );
        }
        PdfPathOutcome::PrimaryGarbledFallbackCorroborated => {
            pf.parse_summary = format!(
                "[pdf-corroborated] 首选路与兜底输出一致：内容确无 CJK（CID 字体声明与内容语言不符的模板件/混排低占比页），双路互证采信（#200 R1）；{}",
                pf.parse_summary
            );
        }
        PdfPathOutcome::FallbackOnlyLowResidue => {
            pf.parse_summary = format!(
                "[pdf-single-source] 首选路不可用；兜底单路输出无 CJK 但非 ASCII 占比低（残渣特征不成立——内容确无 CJK），单路采信（#200 R1）；{}",
                pf.parse_summary
            );
        }
        PdfPathOutcome::OcrRecovered => {
            pf.parse_summary = format!(
                "[pdf-ocr-recovered] 文本面两路均疑似乱码，OCR（渲染面）重解为正常文本（#200 R2）；{}",
                pf.parse_summary
            );
        }
    }
}

/// 多数页正常时允许局部异常页，但必须在摘要中点名，便于定位（#232）。
fn append_page_diagnostics(pf: &mut ParsedFile, raw: &[u8]) {
    let probe = pdf_text::probe_cjk_garble_pages(raw, &pdf_text::parsed_pages_of(pf));
    if probe.garbled_pages.is_empty() {
        return;
    }
    let pages = probe
        .garbled_pages
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join("、");
    pf.parse_summary = format!(
        "{}；局部 CJK 疑似乱码页：{}（#232）",
        pf.parse_summary, pages
    );
}

/// OCR 救援（#200 R2）：文本面两路均不可信时，问一次**渲染面**。
///
/// 前提：`probe()` Ready（chi_sim 在位）——缺件不起渲染进程（诚实跳过，
/// 不伪装尝试）；OCR 产物过乱码探测（非 Garbled 即采信——与 parse_pdf
/// 主流程口径一致）；`[pdf-ocr-recovered]` 标签由**采纳面**（parse_pdf
/// 双乱码分叉处）盖章，本函数产物 summary 自报 OCR 通道面。任何失败都
/// 返回 None——落回调用方既有的 fail-loud 消息（救援是加法，绝不弱化
/// #169 防线）。
fn try_ocr_recovery(
    tess: &TesseractOcr,
    path: &Path,
    raw: &[u8],
    name: &str,
) -> Option<ParsedFile> {
    use crate::engine::EngineStatus;
    match tess.probe() {
        EngineStatus::Ready => {}
        EngineStatus::Missing { what, .. } => {
            // 记入诊断但不停在这里——OCR 缺件时 fail-loud 消息已足够诚实
            let _ = (what, name);
            return None;
        }
        EngineStatus::Unsupported { .. } => return None,
    }
    let pf = tess.ocr_parsed(path).ok()?;
    let pages = pdf_text::parsed_pages_of(&pf);
    // #306/FG-11（chi_sim 环境复跑暴露）：接受 Clean **或 NoSignal**，只拒 Garbled
    // ——与 parse_pdf 主流程的 verdict 处理口径一致（主流程 `match verdict {
    // Garbled => …, _ => 放行 }`）。原「必须 Clean」在 #200/#223 给探测加
    // CID 门后成为死路：非 CID 嵌入字体件（reportlab TTF 面、部分 Word 导出
    // 件）恒 NoSignal → 救援恒 None——OCR 救援在分叉路径（texts_corroborate
    // 不一致）对这类件从不生效，且因 chi_sim 缺失从未真跑暴露。
    match pdf_text::probe_cjk_garble_pages(raw, &pages).verdict {
        CjkGarbleProbe::Garbled => None, // OCR 也解不出 CJK——不是可采信的救援产物
        CjkGarbleProbe::Clean | CjkGarbleProbe::NoSignal => Some(pf),
    }
}

/// 引擎注册表（R3 CrossValidate / R4 外部引擎在此扩展）。
///
/// R2 语义：顺序即 FirstSuccess 优先级（poppler → tesseract → rust-fallback）。
pub fn registry() -> Vec<Box<dyn PdfExtractor>> {
    vec![
        Box::new(PopplerText),
        Box::new(TesseractOcr),
        Box::new(RustFallback),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表三路齐备（owner R2：poppler/tesseract/rust-fallback 各自独立实现）。
    ///
    /// 注：本测试不起子进程（poppler/tesseract 的 probe 会 spawn 外部命令——
    /// 与 guard::tests 的并发席位测试互踩，probe 诚实性由 parse_pdf 的
    /// e2e 路径与 OCR 救援测试覆盖）；rust-fallback 纯 Rust 恒 Ready 在此钉死。
    #[test]
    fn registry_has_three_engines_in_first_success_order() {
        let regs = registry();
        assert_eq!(
            regs.len(),
            3,
            "R2 内置三路：poppler/tesseract/rust-fallback"
        );
        let names: Vec<&str> = regs.iter().map(|e| e.name()).collect();
        assert_eq!(
            names,
            vec!["poppler", "tesseract", "rust-fallback"],
            "FirstSuccess 优先级：文本层 → 扫描层 → 纯 Rust 兜底"
        );
        assert_eq!(
            regs[2].probe(),
            crate::engine::EngineStatus::Ready,
            "纯 Rust 兜底无外部依赖，恒可用"
        );
    }

    /// #200 R2 头牌：文本面两路均不可信时，OCR（渲染面）能把文件救回来。
    ///
    /// 直接钉 `try_ocr_recovery` 的契约：chi_sim 就绪 + 渲染面可读（嵌入式
    /// 中文字体）→ OCR 文本过乱码探测 → `[pdf-ocr-recovered]` 采信。
    /// 环境缺件（无 tesseract/chi_sim、无中文字体、无 python3+reportlab）
    /// 一律【SKIP 横幅】——不伪装通过（owner 铁律 4）。
    #[test]
    fn ocr_recovery_rescues_diverged_text_paths() {
        use crate::engine::EngineStatus;
        // 持子进程测试锁（#306/FG-11：chi_sim 就位后本测试真跑——probe /
        // python3 生成器 / pdftoppm / tesseract 均经 run_limited 占并发席。
        // 不持锁时与 guard::tests 的 busy 门测试互踩：本测试占席 → busy 门
        // acquire().unwrap() panic；反向 busy 门占满 4 席 → 本测试 run_limited
        // 返 Busy → try_ocr_recovery None → expect panic——两败俱伤，chi_sim
        // 环境复跑实测复现。锁纪律与 env_preflight/tesseract 同款）。
        let _slots =
            crate::guard::SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tess = TesseractOcr;
        match tess.probe() {
            EngineStatus::Ready => {}
            EngineStatus::Missing { what, .. } => {
                eprintln!("【SKIP】OCR 救援测试缺件（{what}）——本回归不能验证，但不伪装通过");
                return;
            }
            EngineStatus::Unsupported { .. } => return,
        }
        // 嵌入式 NotoSansSC 中文字体 PDF（渲染面可读——OCR 救援的前提）
        let tmp = std::env::temp_dir().join(format!("bidparse-ocr-rescue-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let pdf = tmp.join("embedded-cjk.pdf");
        let script = "from reportlab.pdfbase import pdfmetrics; \
from reportlab.pdfbase.ttfonts import TTFont; \
from reportlab.pdfgen import canvas; \
import sys; \
pdfmetrics.registerFont(TTFont('NotoSC', '/usr/share/fonts/truetype/chinese/NotoSansSC-Regular.ttf')); \
c = canvas.Canvas(sys.argv[1]); \
c.setFont('NotoSC', 16); \
[c.drawString(72, 800 - 20 * i, '本项目为市政基础设施建设项目第%d次澄清' % i) for i in range(6)]; \
c.save()";
        let mut gen = std::process::Command::new("python3");
        gen.arg("-c").arg(script).arg(&pdf);
        let gen = guard::run_limited(gen, std::time::Duration::from_secs(30), 4096);
        if !matches!(gen, guard::RunOutcome::Success { .. }) {
            eprintln!("【SKIP】环境无 python3+reportlab+中文字体（生成器缺件）——不伪装通过");
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        }
        let raw = std::fs::read(&pdf).unwrap();
        let got = super::try_ocr_recovery(&tess, &pdf, &raw, "embedded-cjk.pdf")
            .expect("chi_sim 就绪 + 渲染面可读 → OCR 救援应产出可采信文本");
        // #306/FG-11：`[pdf-ocr-recovered]` 标签由采纳面（parse_pdf 双乱码分叉处）
        // 盖章——本测试直钉 try_ocr_recovery 契约，断言其产物自报 OCR 通道面
        // （原断言 contains("[pdf-ocr-recovered]") 把采纳面职责错安在救援面，
        // chi_sim 环境复跑才暴露）。
        assert!(
            got.parse_summary.contains("chi_sim"),
            "救援产物应自报 OCR 通道面（实际：{}）",
            got.parse_summary
        );
        let all: String = got.blocks.iter().map(|b| b.text.as_str()).collect();
        assert!(
            all.contains("建设项目"),
            "OCR 应读出中文正文（实际：{all}）"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
