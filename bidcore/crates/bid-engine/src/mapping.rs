//! EngineX → `bid_parse::ParsedFile` 字段级映射签名（T1：只定义，B3 实现）。
//!
//! 单一真源文档：`docs/engine-bid-mapping.md`（引擎字段 × bid 字段 ×
//! 转换规则 × 缺失处置四列表——本模块签名与该文档一一对应，B3 实现时
//! 以文档为验收对照）。
//!
//! B3 实现验收（issue #2 §B3）：≥90% 文件 blocks 完全等价；不等价每条
//! 有归类结论（等价 / 引擎缺失 / 旧实现 bug / 预期差异）——归类由
//! `scripts/engine_diff.py` 双跑差分工具产出。

use crate::types::{EngineBook, EngineDoc, EngineError, EnginePdf};

/// 调用方供给的文件元数据（sha256 由门面层计算——引擎不做文件级指纹）。
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub name: String,
    pub sha256: String,
    pub bytes: usize,
}

/// docx：EngineDoc → ParsedFile。
///
/// 映射要点（详见 engine-bid-mapping.md §docx）：
/// - `EngineBlock::Paragraph` → `BlockKind::Para`（文本直连；locator 按引擎
///   块序重建——引擎无「章节/页/行」描述面）；
/// - `EngineBlock::Table` → `BlockKind::TableRow`（每行一块；cells=rows[i]，
///   text=非空列以空格连接——与旧实现口径逐字节对齐见映射表）；
/// - 页眉/页脚/媒体/警告 → `ParsedFile.components` 账本行（SC-02 对账：
///   present/extracted/note——引擎警告透传 note，缺口显式登记不伪装）；
/// - `revision_inserted/revision_deleted`：#17 修复前引擎返回空 →
///   账本登记「引擎缺口 D6」；**禁止映射层自研提取**（约束 3）。
#[allow(clippy::needless_pass_by_value)]
pub fn docx_to_parsed_file(
    _doc: EngineDoc,
    _meta: &FileMeta,
) -> Result<bid_parse::ParsedFile, EngineError> {
    // B3 实现（issue #2 §B3）——T1 只定义签名。
    Err(EngineError::MappingGap(
        "docx_to_parsed_file: B3 未实装（T1 只定义）".to_string(),
    ))
}

/// xlsx：EngineBook → ParsedFile。
///
/// 映射要点：逐 sheet 逐行 → `BlockKind::XlsxRow`（locator=`sheet名 行N`）；
/// 值/公式双通道（公式格 text=值，公式入账本 note——与旧实现 D1b 口径对齐）。
#[allow(clippy::needless_pass_by_value)]
pub fn xlsx_to_parsed_file(
    _book: EngineBook,
    _meta: &FileMeta,
) -> Result<bid_parse::ParsedFile, EngineError> {
    Err(EngineError::MappingGap(
        "xlsx_to_parsed_file: B3 未实装（T1 只定义）".to_string(),
    ))
}

/// pdf：EnginePdf → ParsedFile。
///
/// 映射要点：页文本 → 行块（`BlockKind::PdfLine`——**行切分规则**在 B3 对齐
/// `pdftotext -layout` 口径：P1 定谳非空白字符比 1.000，仅列对齐空白差异，
/// 归一规则入映射表）；`any_text_layer=false` → `has_text_layer=false` +
/// OCR 分支路由（扫描件补遗通道不在引擎面，映射层只判型不跑 OCR）。
#[allow(clippy::needless_pass_by_value)]
pub fn pdf_to_parsed_file(
    _pdf: EnginePdf,
    _meta: &FileMeta,
) -> Result<bid_parse::ParsedFile, EngineError> {
    Err(EngineError::MappingGap(
        "pdf_to_parsed_file: B3 未实装（T1 只定义）".to_string(),
    ))
}
