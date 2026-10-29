//! bid-engine —— 引擎适配层**唯一接触面**（issue #2 §B3 预演 · T1 批）
//!
//! **本 crate 在 T1 阶段只定义、不实现**：trait / 镜像类型 / 映射函数签名。
//! B3（适配层接通）将：
//! 1. 以**完整 commit rev** 引入 mdcore git 依赖（issue #2 §3.1——禁止 branch 浮动）；
//! 2. 实现 [`EngineReader`]（`mdc-office-io::docx::read_docx_from_bytes` /
//!    `mdc-office-io::io::read_book_from_bytes` / `mdc-pdf Command::ExtractText`）；
//! 3. 实现本模块全部 `mapping` 函数，把引擎类型映射为
//!    `bid_parse::ParsedFile`（**对外接口不变**——issue #2 §B3 约束 3）。
//!
//! 约束（issue #2 §1 强制约束）：
//! - 通用文档语义（解析/安全门/指纹）**不在标书侧新增实现**——本 crate 只做
//!   调用与字段映射，语义一律来自引擎；
//! - 引擎缺口（`docs/engine-gap-requests-m0.5.md`：D6 修订 P0 / D4 文本框 P1 /
//!   D5 批注脚注 P2 / A1–A4）在引擎修复前**不得在标书侧平行补齐**——
//!   映射层遇缺口走「缺失处置」列（见 `docs/engine-bid-mapping.md`），
//!   登记不伪装。
//!
//! 模块：
//! - [`types`]：引擎公开形状的标书面镜像（EngineDoc/EngineBook/EnginePdf）
//! - [`reader`]：引擎读取 trait（B3 唯一实现点）
//! - [`mapping`]：EngineX → `bid_parse::ParsedFile` 字段级映射签名
//!
//! 字段级映射规则与「缺失时怎么办」处置：`docs/engine-bid-mapping.md`（单一真源）。

pub mod mapping;
pub mod reader;
pub mod types;

pub use mapping::{docx_to_parsed_file, pdf_to_parsed_file, xlsx_to_parsed_file, FileMeta};
pub use reader::{EngineReader, MdcoreReader};
pub use types::{
    EngineBlock, EngineBlockSource, EngineBook, EngineDoc, EngineError, EngineHeaderFooter,
    EngineHeaderFooterVariant, EngineImage, EngineParagraph, EnginePdf, EngineSheet,
    EngineTable, EngineWarning,
};

/// 引擎钉住 rev（与 Cargo.toml git 依赖一致——parse_summary 溯源口径）。
/// 升级走独立 PR（`chore(dep): bump mdcore`）并重跑基线差分（issue #2 §3.1）。
pub const MDCORE_REV: &str = "3204342d9a5ee33400aba2b17dad29696e2f0bb8";

/// 门面：读文件 → 引擎解析 → 映射为 `ParsedFile`（B5 import 接线入口，docx 通道）。
///
/// sha256 标书侧算（引擎不做文件级指纹——映射表 §1 末行）；错误分类：
/// 引擎拒绝（三门）→ `EngineError::EngineRejected`（A3 敌意面差分依赖）。
pub fn parse_docx_via_engine(path: &std::path::Path) -> Result<bid_parse::ParsedFile, EngineError> {
    parse_office_via_engine(path, |r, bytes, meta| {
        docx_to_parsed_file(r.read_docx(bytes)?, meta)
    })
}

/// 门面（B3-5）：pdf 文本层通道——pdfium 引擎提取 + 映射。
///
/// **仅文本层件**：扫描件（any_text_layer=false）返回 `MappingGap`——OCR 补遗链
/// 是标书侧保留面（映射表 §3 处置列），调用方（import 路由）回退旧路径。
/// 无 pdfium 二进制的环境返回 `EngineFailed`——调用方同样回退旧路径
/// （pdftotext 通道），测试面走 SKIP（issue #9 口径）。
pub fn parse_pdf_via_engine(path: &std::path::Path) -> Result<bid_parse::ParsedFile, EngineError> {
    parse_office_via_engine(path, |r, bytes, meta| {
        pdf_to_parsed_file(r.extract_pdf_text(bytes)?, meta)
    })
}

/// 门面（B3-4）：xlsx 通道——引擎三门 + calamine 五面提取 + 映射。
///
/// 覆盖 `.xlsx` 容器；`.xls`/`.ods` 旧二进制格式不在引擎能力面
/// （留旧路径 calamine auto——B4 删旧时同步收敛扩展名面）。
pub fn parse_xlsx_via_engine(path: &std::path::Path) -> Result<bid_parse::ParsedFile, EngineError> {
    parse_office_via_engine(path, |r, bytes, meta| {
        xlsx_to_parsed_file(r.read_xlsx(bytes)?, meta)
    })
}

fn parse_office_via_engine(
    path: &std::path::Path,
    f: impl FnOnce(&MdcoreReader, &[u8], &FileMeta) -> Result<bid_parse::ParsedFile, EngineError>,
) -> Result<bid_parse::ParsedFile, EngineError> {
    let bytes = std::fs::read(path).map_err(|e| {
        EngineError::EngineFailed(format!("读取 {} 失败: {e}", path.display()))
    })?;
    let sha = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&bytes);
        hex::encode(h.finalize())
    };
    let reader = MdcoreReader::new();
    let meta = FileMeta {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string()),
        sha256: sha,
        bytes: bytes.len(),
    };
    f(&reader, &bytes, &meta)
}
