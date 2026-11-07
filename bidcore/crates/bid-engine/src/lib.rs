//! bid-engine —— 引擎适配层**唯一接触面**（issue #2 §B3 预演 · T1 批；
//! #110 任务 2：rev 提级 + pagination/style 接线）。
//! 溯源口径：引擎 rev 单一真源 = Cargo.toml 实钉（常量经 build.rs 注入，#327）。
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
//! - [`pages`]：分页预估门面（#110 任务 2——页数限制软预警）
//! - [`styles`]：样式模板门面（#110 任务 2——固定格式件格式跟模板走）
//!
//! 字段级映射规则与「缺失时怎么办」处置：`docs/engine-bid-mapping.md`（单一真源）。
//!
//! # 引擎能力账本（截至 aa0c859d 登记面——#110 任务 2 如实登记；
//! aa0c859d → f4328ebf 段（93 提交）能力面未重审，bump 时未同步账本，
//! 下次 bump 前不视为已核对口径）
//!
//! ## 已接（本 PR）
//!
//! | 能力 | 引擎模块 | 接线面 | 用例 |
//! |---|---|---|---|
//! | pagination 分页预估 | `mdcore::pagination` | [`pages`] | 投标文件页数限制——导出前软预警（误差带自报，三态裁决） |
//! | style 样式模板 | `mdc_office_io::style` | [`styles`] | 固定格式件格式跟模板 docx 走（docDefaults + basedOn 链 + 主题色） |
//!
//! ## 可用未接（引擎已交付，标书侧无消费点/消费点冻结——各登记理由）
//!
//! | 能力 | 引擎模块 | 未接理由（如实） |
//! |---|---|---|
//! | fingerprint 内容指纹 | `mdcore::fingerprint` | 标书侧文件级指纹用 sha256（`lib.rs` 门面自算——映射表 §1 末行）；**块级**指纹的去重消费点（B3 查重）已用 B3 交付的自有实现（`quality.rs`），切引擎块指纹 = 口径变更，须差分重跑后独立 PR |
//! | word_count 字数口径 | `mdcore::word_count` | 标书侧字数口径在 quota/字数限制面（bid-tools）；工作范围限 bid-engine，接线在下一轮（pages 已同 rev 依赖 mdcore，切入口零成本） |
//! | numbering 多级编号 | `mdc_office_io::numbering` | 消费点 = 条款编号（导出链——`export.rs` 在 bid-tables 冻结面）；读面已验证可接（`read_docx_numbering`），解冻后跟 styles 同模式接 |
//! | docx_edit 外科编辑 | `mdc_office_io::docx_edit` | 标书侧无「改已有 docx」用例（起草链 = 生成新文档）；预留：业主提供底稿在位的场景出现时接 |
//! | evidence 证据台账 | `mdc-evidence` 包 | 标书侧证据模型在 `bid-domain::Requirement`/台账（已交付）；引擎 sidecar 是文档级证据（定位器/复算/审核三态），与标书证据台账语义面不同——接=双台账，须先定归并语义（登记待定，不默接） |
//! | formula 公式引擎 | `mdc-formula` 包 | 标书侧无 xlsx 公式计算用例（B1 价格分是规则计算非公式重算）；预留：报价表公式校验场景出现时接 |
//! | pdf organize 页面组织 | `mdc_pdf::organize` | 消费点 = 标书合并/装订预处理（导出后处理链）；当前导出链单文件产出，无合并用例 |
//! | pdf annotate 标注写回 | `mdc_pdf::annotation` | 消费点 = 评标反馈回写；评标侧（bid-grade）当前无标注回写用例 |
//! | pdf links/layers | `mdc_pdf::{links,layers}` | 读面辅助能力；标书解析不消费链接目标/图层开关 |
//! | pdf ocr 扫描件识别 | `mdc_pdf::ocr` | 扫描件补遗链是标书侧保留面（tesseract 通道已交付——映射表 §pdf 处置列）；切引擎 OCR = 换实现 + 差分，独立 PR 定夺 |
//!
//! 登记纪律：接 = 有真实用例 + 消费点在可改范围；未接 = 理由写透（不用「后续再说」糊弄）——引擎能力不接不丢分，接了没消费点才是负债。

pub mod mapping;
pub mod pages;
pub mod reader;
pub mod styles;
pub mod types;

pub use mapping::{docx_to_parsed_file, pdf_to_parsed_file, xlsx_to_parsed_file, FileMeta};
pub use pages::{
    bid_baseline_params, check_page_limit, estimate_pages, params_from_geometry, BidPageEstimate,
    PageLimitVerdict,
};
pub use reader::{EngineReader, MdcoreReader};
pub use styles::{EngineStyleFormat, StyleTemplate};
pub use types::{
    EngineBlock, EngineBlockSource, EngineBook, EngineDoc, EngineError, EngineHeaderFooter,
    EngineHeaderFooterVariant, EngineImage, EngineParagraph, EnginePdf, EngineSheet, EngineTable,
    EngineWarning,
};

/// 引擎钉住 rev（parse_summary 溯源口径）——**单一真源 = Cargo.toml 实钉 rev**。
/// #327（R-23/B-05）：由 build.rs 解析 Cargo.toml 的 mdcore git 依赖注入
/// （`cargo:rustc-env=BID_MDCORE_REV`），此处 `env!` 取值——**手改字面量的
/// 漂移面不存在**；rev 变更只能来自 Cargo.toml（= 有意 bump，走独立 PR
/// `chore(dep): bump mdcore` 并重跑基线差分，issue #2 §3.1）。
/// 编译期硬约束（见 build.rs）：mdcore 多包 rev 不一致 / 浮动钉扎 / 非完整
/// 40 位 → 编译失败。
/// 历史链：3204342d →（#110 任务 2，87 提交）→ aa0c859d →（后续 bump，
/// 93 提交）→ f4328ebf（第二次 bump 漏改常量即 R-23 事故现场——#327 根治）。
pub const MDCORE_REV: &str = env!("BID_MDCORE_REV");

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
    let bytes = std::fs::read(path)
        .map_err(|e| EngineError::EngineFailed(format!("读取 {} 失败: {e}", path.display())))?;
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
