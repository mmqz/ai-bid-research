//! engine.rs —— PDF 提取引擎统一接口（#200 R2：可插拔多引擎架构的地基）。
//!
//! 设计来源：owner 在 issue #200 的架构指令——把 PDF 解析从「硬编码
//! if-else 链」改成可插拔多引擎 + 逐页交叉校验。R2 落地接口与内置三路
//! （poppler 文本层 / tesseract 扫描层 / 纯 Rust 兜底）；R3 在此之上加
//! CrossValidate 逐页对比；R4 加外部引擎（Docling / MinerU）子进程适配。
//!
//! 关键约束（owner 原文）：
//! - 每路引擎独立实现，互不知晓；
//! - `probe()` 不可用时返回具体原因——**不要静默返回 false**；
//! - 状态诚实：每页必须能答「这段文字来自哪条路」（name 进追溯与 summary）；
//! - fail-loud 保留：全部失败仍拒绝，但必须准确说明是**文件损坏 /
//!   环境缺件 / 解析器不支持**——三者不能混为一谈（见 [`EngineError`]）。

use std::path::Path;

/// 单页提取产物（R3 逐页交叉校验的粒度单位）。
#[derive(Debug, Clone)]
pub struct PageText {
    /// 页号（1 起；文本层按 \f 分段序，扫描层按渲染张序——与块定位器同口径）。
    pub page: usize,
    /// 该页全文（行以 \n 连接）。
    pub text: String,
    /// 引擎自评置信度（0.0-1.0；无自评能力填 1.0）。
    pub confidence: f32,
    /// 诊断信息（如 CMap 命中率、OCR 置信度分布、流解码计数）。
    pub diagnostics: String,
}

/// 引擎可用性探测结论（不静默——缺什么、怎么补，写清楚）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineStatus {
    /// 可用。
    Ready,
    /// 外部工具/依赖没装：缺什么 + 怎么补（hint）。
    Missing { what: String, hint: String },
    /// 能力不支持（如无 CMap 映射的编码形态）。
    Unsupported { what: String },
}

/// 引擎失败的三分类（owner 约束：三者不能混为一谈）。
#[derive(Debug, Clone)]
pub enum EngineError {
    /// **环境缺件**：外部工具没装/语言数据缺失——装上就能好。
    EnvMissing {
        engine: &'static str,
        detail: String,
        hint: String,
    },
    /// **文件损坏**：工具在位但文件读不了（坏 xref / 非法结构 / 加密）。
    FileBroken { engine: &'static str, detail: String },
    /// **解析器不支持**：文件合法但超出本引擎能力（滤链/编码形态）。
    Unsupported { engine: &'static str, what: String },
    /// **治理拒绝**：超时/输出超限/并发满载——不重试不降级，明说。
    Governed { engine: &'static str, detail: String },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::EnvMissing { engine, detail, hint } => {
                write!(f, "{detail}（{engine} 引擎环境缺件：{hint}）")
            }
            EngineError::FileBroken { engine, detail } => {
                write!(f, "{detail}（{engine} 引擎判定文件损坏/不可读）")
            }
            EngineError::Unsupported { engine, what } => {
                write!(f, "{engine} 引擎不支持：{what}")
            }
            EngineError::Governed { engine, detail } => {
                write!(f, "{detail}（{engine} 引擎治理拒绝）")
            }
        }
    }
}

/// PDF 提取引擎统一接口。每路引擎独立实现，互不知晓。
pub trait PdfExtractor: Send + Sync {
    /// 引擎名（写进每页来源追溯与 parse_summary）。
    fn name(&self) -> &'static str;

    /// 是否可用（检测外部工具/依赖）。不可用时返回具体原因，不静默。
    fn probe(&self) -> EngineStatus;

    /// 提取全篇（按页返回——逐页对比需要页级粒度）。
    fn extract(&self, path: &Path) -> Result<Vec<PageText>, EngineError>;
}
