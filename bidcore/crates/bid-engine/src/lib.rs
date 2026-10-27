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
    EngineBlock, EngineBook, EngineDoc, EngineError, EngineImage, EngineParagraph, EnginePdf,
    EngineSheet, EngineTable, EngineWarning,
};
