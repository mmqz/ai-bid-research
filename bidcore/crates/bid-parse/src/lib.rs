//! bid-parse —— 确定性解析层（AdapterA 的读面）。
//!
//! B4 收敛（issue #11 任务 3）：docx/xlsx 自研解析已删——引擎（bid-engine
//! → mdc-office-io）是 OOXML 读面的单一真源，本 crate 不再持有 zip/quick-xml/
//! calamine 解析栈；消费方经扩展名路由直调 `bid_engine::parse_docx_via_engine`
//! / `parse_xlsx_via_engine`（`bid-parse` dev-dep 提供，service.rs 同款）。
//! 保留面：
//! - PDF 文本层：poppler `pdftotext -layout`（外部引擎适配器，版本由部署清单钉住）+
//!   PDF 扫描层：`pdftoppm` 渲染 → tesseract chi_sim（OCR 分档）——`parse_file`
//!   的 pdf 臂与回退路径（pdfium 引擎优先，映射表 §3）；
//! - **pdf_text.rs（纯 Rust 兜底——第三条降级路）**：pdfium 与 poppler 都不可用时，
//!   用零外部二进制的最小 PDF 文本提取器兜底（补遗/澄清函/更正公告这类
//!   短文本 PDF 是甜区）。实测背景：两条路全断时补遗会**静默丢失**，而补遗里的
//!   新控制价/新截止时间没进抽取面时校验仍然 blocker=0——「不知道补遗存在」
//!   与「补遗没问题」是两种完全不同的状态。兜底也失败时**两条原因都报出来**；
//! - guard.rs：pdf 字节门 / 子进程治理（T-04）/ 流式哈希；
//! - extract.rs：条款抽取业务规则（消费 locator 字符串，与解析器解耦）；
//! - clause.rs：条款边界与跨行合并（B6·G1 纯函数面——docx 续行吸收的
//!   边界判定单一真源）；
//! - numbering.rs：多级编号形态识别（B6·G5 七形态全枚举 + 混排语境修正——
//!   clause_boundary 的编号判定底层）；
//! - nesting.rs：引号/括号嵌套配对与保护切分（B6·G6 纯函数面——
//!   find_matching + split_respecting_nesting，未配对优雅降级）。
//!
//! 引擎适配器约定（组件版本矩阵的钉住点，test-plan §5.2）：
//! - 解析失败/无文本层绝不静默：无文本层 → `has_text_layer=false` + OCR 通道；
//!   OCR 失败 → 保留空 ocr_blocks 并在 parse_summary 说明（诚实状态）。
//!
//! 确定性纪律：同输入文件 → 同输出 blocks（顺序、locator、文本逐字节一致；
//! 外部引擎版本变化入组件版本矩阵，不属本层自由度）。

use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod clause;
pub mod env_preflight;
pub mod extract;
pub mod guard;
pub mod merge;
pub mod nesting;
pub mod numbering;
pub mod pdf;
pub mod pdf_text;
pub mod review_table;
pub mod veto_lexicon;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Para,     // DOCX 正文段落
    TableRow, // DOCX 表格行（cells 逐列）
    PdfLine,  // PDF 文本层行（pdftotext -layout）
    OcrLine,  // 扫描件 OCR 行
    XlsxRow,  // Excel 行
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub idx: usize, // 文件内序号（chunk_id 稳定性的组成）
    pub kind: BlockKind,
    pub locator: String, // 章节/页/行 定位描述
    pub text: String,    // 行/段落全文（表格行 = 列文本以空格连接）
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedFile {
    pub name: String,
    pub sha256: String,
    pub pages: usize,
    pub tables: usize,
    pub has_text_layer: bool,
    pub ocr_blocks: usize,
    pub blocks: Vec<Block>,
    pub parse_summary: String,
    /// SC-02 部件对账账本：部件在位数 vs 进入输出流提取数——present>0 而
    /// extracted==0 的部件必须带 note 显式登记（登记不伪装面扩展到部件级，
    /// G7 零提取面逐文件可见化）。非 docx 解析面为空账本（部件概念不适用）。
    #[serde(default)]
    pub components: Vec<ComponentLedger>,
}

/// SC-02 部件对账账本行（cpa-multi-plugins issue #38 吸收：docx 进 N 个部件，
/// 出站必须有清单；任何部件 0 提取必留显式事件）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentLedger {
    pub component: String, // 正文/页眉/页脚/文本框/批注/脚注尾注/修订插入/修订删除
    pub present: usize,    // 在位部件数（或出现次数）
    pub extracted: usize,  // 进入独立输出块的部件数（0 = 零提取，必须带 note）
    #[serde(default)]
    pub note: String,
}

/// 文件类别权重：精确检索的确定性排序因子（主文件权威性优先，防附件/证据同值噪声抢占命中）。
pub fn kind_rank(file_name: &str) -> u8 {
    if file_name.starts_with("A-") {
        0 // 主招标文件
    } else if file_name.starts_with("C-") {
        1 // 补遗（后到版本权威）
    } else if file_name.starts_with("B") {
        2 // 附件/图纸
    } else {
        3 // 证据库
    }
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    // T-03：流式哈希（原 fs::read 全量内存——大文件面顺带根治）
    guard::sha256_streaming(path)
}

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
}

/// 统一分发：按扩展名选择解析器。kind（业务类别）仅作语义提示。
///
/// B4 后仅剩 **pdf 面**（文本层/扫描 OCR 补遗链——映射表 §3 的标书侧保留面）；
/// docx/xlsx 由消费方扩展名路由直调 bid-engine 门面（service.rs 同款），
/// 直入本函数会得到「不支持的文件类型」显式拒绝——不留静默降级。
pub fn parse_file(path: &Path, name: &str) -> Result<ParsedFile, String> {
    let sha256 = sha256_file(path).map_err(|e| format!("读取失败 {name}: {e}"))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "pdf" => {
            // 第三条降级路（pdf_text 纯 Rust 兜底）。
            //
            // 实测背景：未装 pdfium 且未装 poppler 的环境里，首选路（pdfium）
            // 与回退路（pdftotext）全断——补遗文件会静默丢失，后续抽取全部基于
            // 旧文件且校验仍然 blocker=0。此处先走原链路，两条都失败时用纯 Rust
            // 提取兜底（甜区：短文本 PDF——补遗/澄清函/更正公告正好在此）。
            // 兜底也失败→ 仍然明说失败（两条原因都报出来），绝不静默降级。
            //
            // #169 防线（乱码降级 + fail-loud）：首选路「成功」也可能吐乱码——
            // Windows CI 实测：缺 poppler-data（CMap 资源包）的 pdftotext 对
            // 「非嵌入 CID 字体 + UniGB-UCS2-H」的中文 PDF 输出 Latin-1 化残渣
            // （`原招标文件`→`SbheN`），has_text_layer=true、流程「成功」——
            // 静默产出垃圾标书比解析失败更危险。语义判据探测（PDF 声明 CJK
            // 字体而输出 CJK 占比≈0 → 必乱码，纯英文件永不触发）：
            //   首选路乱码 → 降级纯 Rust 兜底重解（summary 诚实自报）；
            //   兜底也乱   → 显式报错（fail-loud：宁可失败不产出垃圾）。
            let raw = std::fs::read(path).map_err(|e| format!("读取失败 {name}: {e}"))?;
            // #200 R1 的 CID 在用门与 #232 的分页多数决共同决定是否降级；
            // 全页探测仍采用严格多数，平票保持 fail-loud。
            let (mut pf, outcome) = match pdf::parse(path) {
                Ok(primary) => {
                    let primary_text = pdf_text::parsed_text_of(&primary);
                    let primary_probe = pdf_text::probe_cjk_garble_pages(
                        &raw,
                        &pdf_text::parsed_pages_of(&primary),
                    );
                    match primary_probe.verdict {
                        pdf_text::CjkGarbleProbe::Garbled => {
                            match pdf_text::parse_via_fallback(path, name) {
                                Ok(fb) => {
                                    let fb_text = pdf_text::parsed_text_of(&fb);
                                    let fb_probe = pdf_text::probe_cjk_garble_pages(
                                        &raw,
                                        &pdf_text::parsed_pages_of(&fb),
                                    );
                                    match fb_probe.verdict {
                                        // 兜底重解出 CJK——#169 降级链主路径
                                        pdf_text::CjkGarbleProbe::Clean => (
                                            fb,
                                            PdfPathOutcome::PrimaryGarbledFallbackRecovered,
                                        ),
                                        _ => {
                                            // 兜底也无 CJK：双路对比（#200 R1 修正）
                                            // 一致 → 内容确无 CJK（字体声明与内容
                                            // 语言不符的模板件），双路互证采信；
                                            // 分叉 → 两路各乱各的，fail-loud。
                                            if pdf_text::texts_corroborate(&primary_text, &fb_text) {
                                                (
                                                    fb,
                                                    PdfPathOutcome::PrimaryGarbledFallbackCorroborated,
                                                )
                                            } else {
                                                let hint = pdf_text::cjk_garble_reason_hint(&raw)
                                                    .map(|h| format!("。{h}"))
                                                    .unwrap_or_default();
                                                let pages = fb_probe
                                                    .garbled_pages
                                                    .iter()
                                                    .map(|p| p.to_string())
                                                    .collect::<Vec<_>>()
                                                    .join("、");
                                                let pages_info = if pages.is_empty() {
                                                    String::new()
                                                } else {
                                                    format!("（兜底疑似乱码页：{pages}）")
                                                };
                                                return Err(format!(
                                                    "[pdf-dual-garbled] pdf 两条路输出均为疑似 CJK 乱码且互不一致（PDF 使用 CID-CJK 字体而两路文本均无 CJK 且不相同）{pages_info}：{name}——拒绝静默产出垃圾文本（issue #169）{hint}"
                                                ));
                                            }
                                        }
                                    }
                                }
                                Err(fb_err) => {
                                    // #200 能力边界：乱码拒收时点名原因与绕过（疑因线索）
                                    let hint = pdf_text::cjk_garble_reason_hint(&raw)
                                        .map(|h| format!("。{h}"))
                                        .unwrap_or_default();
                                    return Err(format!(
                                        "[pdf-primary-garbled-fallback-failed] pdf 首选路输出疑似 CJK 乱码（PDF 使用 CID-CJK 字体而文本无 CJK——典型为 poppler 缺 CMap 资源包），纯 Rust 兜底也失败：{fb_err}{hint}"
                                    ))
                                }
                            }
                        }
                        _ => (primary, PdfPathOutcome::PrimaryVerified),
                    }
                }
                Err(chain_err) => match pdf_text::parse_via_fallback(path, name) {
                    Ok(fb) => {
                        let fb_text = pdf_text::parsed_text_of(&fb);
                        let fb_probe = pdf_text::probe_cjk_garble_pages(
                            &raw,
                            &pdf_text::parsed_pages_of(&fb),
                        );
                        match fb_probe.verdict {
                            pdf_text::CjkGarbleProbe::Garbled => {
                                // 单路疑似、无第二路可互证：看残渣特征（#200 R1）。
                                // 非 ASCII 占比高 = 多字节误判残渣（如 GBK 被逐字节
                                // Latin-1 化）→ fail-loud；占比低 = 内容本就单字节
                                // （真英文/拉丁模板件）→ 按「内容确无 CJK」采信。
                                let residue = pdf_text::non_ascii_ratio(&fb_text);
                                if residue >= 0.3 {
                                    let hint = pdf_text::cjk_garble_reason_hint(&raw)
                                        .map(|h| format!("。{h}"))
                                        .unwrap_or_default();
                                    let pages = fb_probe
                                        .garbled_pages
                                        .iter()
                                        .map(|p| p.to_string())
                                        .collect::<Vec<_>>()
                                        .join("、");
                                    let pages_info = if pages.is_empty() {
                                        String::new()
                                    } else {
                                        format!("（疑似乱码页：{pages}）")
                                    };
                                    return Err(format!(
                                        "[pdf-fallback-only-garbled] pdf 首选路不可用（{chain_err}）；纯 Rust 兜底单路输出疑似 CJK 乱码且非 ASCII 占比 {residue:.2}（多字节误判残渣特征）{pages_info}，无第二路可互证——拒绝静默产出垃圾文本（issue #169/#200）{hint}"
                                    ));
                                }
                                (
                                    fb,
                                    PdfPathOutcome::FallbackOnlyLowResidue,
                                )
                            }
                            _ => (fb, PdfPathOutcome::PrimaryUnavailableFallbackClean),
                        }
                    }
                    Err(fb_err) => {
                        return Err(format!(
                            "[pdf-both-failed] pdf 解析失败 {name}: {chain_err}；纯 Rust 兜底也失败：{fb_err}"
                        ))
                    }
                },
            };
            // 判决 → summary 诚实自报（对账能区分走了哪条路、为何采信）。
            match outcome {
                PdfPathOutcome::PrimaryVerified
                | PdfPathOutcome::PrimaryUnavailableFallbackClean => {}
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
            }
            // 多数页正常时允许局部异常页，但必须在摘要中点名，便于定位。
            let page_probe =
                pdf_text::probe_cjk_garble_pages(&raw, &pdf_text::parsed_pages_of(&pf));
            if !page_probe.garbled_pages.is_empty() {
                let pages = page_probe
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
            pf.name = name.to_string();
            pf.sha256 = sha256;
            Ok(pf)
        }
        other => Err(format!("不支持的文件类型 .{other}（{name}）")),
    }
}

// ---------------------------------------------------------------- 归一化（对齐/检索共用）
/// N1 归一化：NFKC（全角→半角）+ 去全部空白。保留标点（代码/金额语境需要）。
pub fn norm_n1(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let nfkc: String = s.nfkc().collect();
    nfkc.chars().filter(|c| !c.is_whitespace()).collect()
}

/// N2 归一化：N1 + 去标点（OCR 噪声容忍对齐用——标点在 OCR 下不可靠）。
pub fn norm_n2(s: &str) -> String {
    let n1 = norm_n1(s);
    n1.chars().filter(|c| c.is_alphanumeric()).collect()
}
