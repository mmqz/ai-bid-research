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
pub mod engine;
pub mod engines;
pub mod env_preflight;
pub mod extract;
pub mod guard;
pub mod merge;
pub mod nesting;
pub mod numbering;
pub mod pdf_text;
pub mod review_table;
pub mod scoring_table;
pub mod table_semantics;
pub mod veto_lexicon;
// #226 层 2：规则说明识别（区分「招标人规则陈述」与「投标人响应要求」）。
pub mod veto_rule;

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
            // #200 R2：解析链全部迁入 engines（可插拔引擎 + 仲裁器）——
            // 乱码判决/降级/互证/OCR 救援的口径见 engines/mod.rs，此处只做
            // 扩展名路由与 sha256 回填。
            let mut pf = engines::parse_pdf(path, name)?;
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
