//! export.rs —— S-6 组装与流式导出（issue #11 批次 v3 / V3-5）。
//!
//! 定位：[`crate::assembly::ChapterAssembler`] 工作区（`chapters/*.json`）→
//! 标书 docx 的**只读消费半边**。组装层保证「每章生成完立即落盘、LLM 生成
//! 的代价永远不重复」；本模块把这条纪律延伸到导出层：**不重新生成任何章
//! 内容**——只读已提交的 ChapterRecord（`ChapterAssembler::open` 装载，零
//! generator 调用），组装失败/未完成时 fail-closed 拒绝导出，绝不静默跳章。
//!
//! ## 流式纪律（500 章不能拼成巨串）
//!
//! Lead 裁决（架构决策，不再发明）：**标书侧自己组装，不给引擎提增量写
//! API**——spike 实测 500 章百万字 docx-rs 内存模型 ~31MB ≪ 500MB。落地：
//! 1. 章内容按文档序**逐章** `add_paragraph` 进 docx-rs 模型（每章自带独立
//!    String，永不先拼一个百万字巨 String 再塞）；
//! 2. [`docx_rs::Docx::pack`] 直接把各 XML 部件流式写进 zip writer（0.4.22
//!    的流式通道——`zipper::zip_docx` 逐部件 `build_to(stream)`，不把整包
//!    XML 物化成 `Vec<u8>`）；
//! 3. 峰值内存 = 工作区记录（~3MB 文本）+ docx-rs 模型（~31MB spike 实测）
//!    ——硬验收 < 1GB 的余量三个数量级（export_stream.rs VmHWM 实测口径）。
//!
//! ## TOC 域（调研结论：docx-rs 0.4.22 原生支持，无需手写 fldSimple 注入）
//!
//! `docx_rs::TableOfContents` + `InstrToC` 是完整 TOC 域类型面：
//! `TableOfContents::new().heading_styles_range(1,3).hyperlink()
//! .use_applied_paragraph_line_level()` 渲染出（vendored 源
//! `documents/elements/table_of_contents.rs::test_toc` 为证）：
//! `w:fldChar begin(dirty=true)` + `w:instrText` = `TOC \o "1-3" \h \u` +
//! `separate` … `end`——正是规格要求的域指令（Word 打开后 F9 更新出目录；
//! `w:dirty="true"` 让 Word 打开时即提示/自动重算该域）。开关面沿用规格
//! 原文 `\o "1-3" \h \u`：标题层级 1-3 进目录（T-5 树最深 4 级——4 级标题
//! 仍是合法 Heading4 段落，只是不进目录，口径与规格字面一致）。各级标题用
//! Heading1..4 段落样式（`w:name` = "Heading 1".."Heading 4"——docx-rs
//! `Name::is_heading` 与 Word 内建样式名同口径，`TOC \o` 才能识别层级）。
//!
//! ## 原子产出（borrow：mdcore `sidecar.rs::SidecarStore::save` 的 tmp+rename
//! ## 纪律，经本仓 assembly.rs / store_io.rs 同款落地）
//!
//! 写 `{out}.docx.tmp` 全量 → `fs::rename` 替换（POSIX 原子；Windows 上
//! std::fs::rename 同为替换式）。**rename 返回即导出安全**——此后进程随时
//! 可死，产物完整；中断残骸只可能是 `*.docx.tmp`（下次导出启动时静默丢弃
//! ——borrow `interrupted_publish_leftover_does_not_break_recovery`）。幂等
//! 导出清单 `export-manifest.json`（与 chapters/ 平级，同款 tmp+rename）。
//!
//! ## 幂等导出清单（「已写入的章不重写」在导出层的落点）
//!
//! `export-manifest.json` 记录 { 源章指纹列表 (section_id + attempt +
//! content sha256，按文档序), 导出时间戳, 输出 sha256, 上次 ExportReport }。
//! 重跑 export：输入指纹全等 **且** 输出存在且 sha 吻合 → **no-op 直返上次
//! 报告**（输出一个字节不重写——产物 sha 对账 borrow `bid-domain::
//! export_gate.rs` 的「EXPORTED 以产物 sha256 落库为界」语义）；输入变了
//! （任一章 attempt/内容指纹漂移、增删章）→ 全量重建。manifest 是**导数**
//! 不是源数据：损坏/缺字段/未来版本 → 当作无 manifest 全量重建（真源
//! chapters/*.json 的 fail-closed 已在 `ChapterAssembler::open` 拦截——
//! 主数据严格、导数宽容，两层面各司其职）。
//!
//! ## fail-closed 导出门（半个文档不猜测——borrow mdcore 哲学）
//!
//! 1. 树为空 / 树不可全序覆盖（孤儿、环、重复 id——DFS 丢节点 = 静默半个
//!    文档，必须显式拒）；
//! 2. 已提交记录不在树中 → `GhostSection` 拒（重开纲后旧工作区不匹配，
//!    语义 borrow [`AssemblerError::GhostSection`]）；
//! 3. 记录挂在**结构节点**（非叶）上 → 拒（assemble_all 只对叶提交——父
//!    节点出现记录即工作区与树不匹配的怪态）；
//! 4. 叶章无 ChapterRecord → `MissingRecord` 拒（缺内容章绝不静默跳过）；
//! 5. 叶章状态未达 DRAFTED → `UndraftedSection` 拒（PENDING/OUTLINING/
//!    DRAFTING 都算「半个文档」；DRAFTED 及其后 VALIDATING/REVISING/
//!    ACCEPTED 均为已定稿面，放行）。「未 DRAFTED」只查**叶**——父节点是
//!    结构容器，assemble_all 不推进其状态（查全树会与 S-2 语义矛盾）。
//!
//! crate 纪律不变：时间戳用 SystemTime→UNIX_EPOCH 秒（u64，no-chrono）。

use crate::assembly::{AssemblerError, ChapterAssembler};
use bid_domain::outline::{OutlineNode, SectionStatus};
use docx_rs::{Docx, DocumentChild, Paragraph, Run, Style, StyleType, TableOfContents};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// 幂等导出清单文件名（与 `chapters/` 平级：`{dir}/export-manifest.json`）。
pub const MANIFEST_FILE: &str = "export-manifest.json";

/// 导出清单 schema 版本（结构性变更时 +1——当前仅 v1）。
const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// TOC 域开关（规格原文）：`TOC \o "1-3" \h \u`——标题层级 1-3 + 超链接 +
/// 应用段落大纲级别。构造常量函数，唯一真源。
fn toc_field() -> TableOfContents {
    // with_instr_text 直接吃规格原文（docx-rs 的 FromStr 解析 \o/\h/\u 开关
    // —— vendored `instr_toc.rs::with_instr_text` 单元测试锚定此行为：
    // `TOC \o "1-3" \h \z \u` 解析出 heading_styles_range(1,3)+hyperlink+
    // use_applied_paragraph_line_level）。相比链式 builder（TableOfContents 只
    // 暴露 heading_styles_range/hyperlink 两个开关，缺 \u），字符串面是唯一
    // 能落齐规格三开关的入口。
    TableOfContents::with_instr_text(r#"TOC \o "1-3" \h \u"#)
}

// ---------------------------------------------------------------- 导出报告与清单

/// 导出报告（对账面——一轮 export 的产物账）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    /// 写入 docx 的章数（= 叶章数——每叶一标题 + 正文段落组）
    pub chapters_written: usize,
    /// 全部已写章正文字数和（char_count 口径，与 S-2 账面同源）
    pub total_chars: u64,
    /// 本次构建耗时（毫秒；no-op 直返上次值——报告全等性的一部分）。
    /// no-op 路径不重新计时（重计会让「输入未变 → 报告相等」永远失败）。
    pub duration_ms: Option<u64>,
    /// 输出 docx 的 sha256（十六进制小写）——产物指纹（export_gate 语义）
    pub output_sha256: String,
    /// TOC 域是否进正文（构造面断言：本导出器恒插 TOC 域，见 toc_field；
    /// 文件级验证在 export_stream.rs 以「document.xml 含 `TOC \o` 字样」
    /// 为准——报告位与测试断言双面同源）。
    pub toc_field_present: bool,
}

/// 源章指纹（幂等对账的最小充分量：同章同版本同内容 → 三元组全等）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterFingerprint {
    pub section_id: String,
    /// ChapterRecord 的 attempt（内容修订 → +1 → 指纹变 → 全量重建）
    pub attempt: u32,
    /// content 的 sha256（十六进制小写）
    pub content_sha256: String,
}

/// 幂等导出清单（`{dir}/export-manifest.json`，tmp+rename 原子落盘）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportManifest {
    pub schema_version: u32,
    /// 按文档序的源章指纹列表（顺序确定——Vec 全等比较含序）
    pub fingerprints: Vec<ChapterFingerprint>,
    /// UNIX_EPOCH 秒（no-chrono 纪律）
    pub exported_at: u64,
    /// 上次导出报告（no-op 直返面——「一个字节不重写」的对价是报告可复现）
    pub report: ExportReport,
}

// ---------------------------------------------------------------- 错误面（typed）

/// 导出错误（typed——调用方按变体分支，不解析 message；变体风格与
/// [`AssemblerError`] / StoreIoError 同族）。
#[derive(Debug)]
pub enum ExportError {
    Io(std::io::Error),
    Serialize(serde_json::Error),
    /// docx 打包失败（docx-rs `Docx::pack` 的 ZipError 透传——终端 IO 面，
    /// 文本承载即可：zip 错误类型不在本 crate 的依赖面）
    Pack(String),
    /// 章节树为空（无内容可导出——不产出空文档假装成功）
    EmptyTree,
    /// 树中存在 DFS 全序不可达的节点（孤儿父引用/环/重复 id——静默丢节点
    /// 等于导出半个文档，必须显式拒）
    UnreachableSection { section_id: String },
    /// 叶章状态未达 DRAFTED（PENDING/OUTLINING/DRAFTING——半个文档不猜测）
    UndraftedSection { section_id: String, status: String },
    /// 叶章无 ChapterRecord（未提交——内容缺失，绝不静默跳章）
    MissingRecord { section_id: String },
    /// 已提交记录不在章节树中（幽灵章——语义 borrow AssemblerError::GhostSection）
    GhostSection { section_id: String },
    /// 记录挂在结构节点（非叶）上——assemble_all 只对叶提交，父节点出现
    /// 记录即工作区与树不匹配的怪态，fail-closed 拒
    RecordOnStructuralNode { section_id: String },
    /// 组装工作区打开/校验失败（S-2 fail-closed 面透传——Corrupt 等原样上抛）
    Assembler(AssemblerError),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io(e) => write!(f, "导出 IO 失败: {e}"),
            ExportError::Serialize(e) => write!(f, "导出清单序列化失败: {e}"),
            ExportError::Pack(why) => write!(f, "docx 打包失败（zip 写入）: {why}"),
            ExportError::EmptyTree => write!(f, "章节树为空——无内容可导出（半个文档不猜测）"),
            ExportError::UnreachableSection { section_id } => write!(
                f,
                "章节 {section_id} 从根不可达（孤儿父引用/环/重复 id）——静默丢节点等于半个文档，显式拒绝"
            ),
            ExportError::UndraftedSection { section_id, status } => write!(
                f,
                "章节 {section_id} 状态为 {status}（未 DRAFTED）——半个文档不猜测，完成该章后重导"
            ),
            ExportError::MissingRecord { section_id } => write!(
                f,
                "章节 {section_id} 无已提交内容（ChapterRecord 缺失）——绝不静默跳章，续跑 assemble 后重导"
            ),
            ExportError::GhostSection { section_id } => write!(
                f,
                "已提交章 {section_id} 不在当前章节树中（幽灵章——重开纲后旧工作区不匹配，显式报错不猜测）"
            ),
            ExportError::RecordOnStructuralNode { section_id } => write!(
                f,
                "章节 {section_id} 是结构节点（非叶）却有已提交记录——工作区与树不匹配，显式报错不猜测"
            ),
            ExportError::Assembler(e) => write!(f, "组装工作区校验失败: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}

// ---------------------------------------------------------------- 导出器

/// 组装产物导出器：章节树（文档序由内部 DFS 全序化）+ ChapterAssembler
/// 工作区 → 标书 docx。
///
/// 用法：`DocumentExporter::new(store.sections()).export(dir, out_path)`。
/// 树是**借用输入**（不可变——导出绝不改调度状态）；`export` 可反复调用
/// （幂等面由 manifest 承载）。
pub struct DocumentExporter<'a> {
    /// 章节树全集（任意序传入，内部按「根起 DFS + 兄弟按 order 升序」全序化）
    nodes: &'a [OutlineNode],
}

/// 导出选项（#59 阶段 2：format-spec 应用 + 暗标模式）。
///
/// - `format_spec`：`None` = 纯默认面（无页眉页脚、docx-rs 默认 A4——
///   既有测试的零变化通道）；`Some` = 应用规格（页面/边距/Normal 字体行距/
///   页眉页脚，见 [`crate::format_spec::apply_to_docx`]）。
/// - `project_name`：页眉占位符替换素材。
/// - `anonymize`：暗标模式（Some(bidder_name) = 公司名 → 「投标人」——
///   正文与章标题同步替换；None = 明标）。
#[derive(Debug, Clone, Default)]
pub struct ExportOptions<'a> {
    pub format_spec: Option<&'a crate::format_spec::FormatSpec>,
    pub project_name: String,
    pub anonymize: Option<&'a str>,
}

impl<'a> ExportOptions<'a> {
    /// 暗标替换（公司名 → 「投标人」；bidder 为空则原文不动——无名字可替
    /// 不是错误，但导出报告需告警，见服务层）。
    fn scrub(&self, text: &str) -> String {
        match self.anonymize {
            Some(bidder) if !bidder.is_empty() => text.replace(bidder, "投标人"),
            _ => text.to_string(),
        }
    }
}

impl<'a> DocumentExporter<'a> {
    /// 以章节树构造导出器。`nodes` 为空 → export 时 [`ExportError::EmptyTree`]。
    pub fn new(nodes: &'a [OutlineNode]) -> Self {
        Self { nodes }
    }

    /// 导出标书 docx（唯一入口）。
    ///
    /// 步骤（顺序即注释里的序号——fail-closed 门全部在写盘**之前**）：
    /// ① tmp 残骸清理 → ② 工作区装载（S-2 fail-closed 透传）→ ③ 文档序
    /// 全序 + 树完整性 → ④ 导出门五查 → ⑤ 幂等 no-op 判定 → ⑥ 逐章构建
    /// docx 模型 → ⑦ tmp+rename 原子落盘 → ⑧ sha256 + manifest 原子落盘。
    pub fn export(
        &self,
        dir: impl AsRef<Path>,
        out_path: impl AsRef<Path>,
    ) -> Result<ExportReport, ExportError> {
        self.export_with(dir, out_path, &ExportOptions::default())
    }

    /// 带选项导出（#59 阶段 2：format-spec + 暗标）。
    ///
    /// 与 [`Self::export`] 同门同幂等面（manifest 指纹含 attempt + content
    /// sha——同指纹重导 no-op 直返）；差异仅在构建面：
    /// - format_spec 在场 → 页面/边距/Normal 字体行距/页眉页脚叠加；
    /// - anonymize 在场 → 正文与标题的公司名 → 「投标人」（暗标评审禁
    ///   公司名/logo——文本面替换；图片水印面属阶段 7 PDF 通道）。
    pub fn export_with(
        &self,
        dir: impl AsRef<Path>,
        out_path: impl AsRef<Path>,
        opts: &ExportOptions<'_>,
    ) -> Result<ExportReport, ExportError> {
        let started = Instant::now();
        let dir = dir.as_ref();
        let out_path = out_path.as_ref();

        // ① 中断残骸静默丢弃：上一次 export 死在 rename 前只可能留下 tmp。
        // （borrow：mdcore interrupted_publish_leftover_does_not_break_recovery
        // ——残骸不影响主文件，下次写入原子顶替；本导出器在入口先行清扫，
        // 让「no-op 直返」路径同样不留垃圾。）
        let tmp_path = sibling_tmp(out_path);
        match std::fs::remove_file(&tmp_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ExportError::Io(e)),
        }

        // ② 工作区装载：损坏/截断/char_count 失配 → S-2 Corrupt 透传（主数据
        // fail-closed——导出层不重复实现，单一真源）。
        let asm = ChapterAssembler::open(dir).map_err(ExportError::Assembler)?;

        // ③ 文档序全序化 + 树完整性（孤儿/环/重复 → 显式拒，见 order_tree）。
        let (ordered, children) = order_tree(self.nodes)?;

        // ④ 导出门（fail-closed——半页纸都不猜测）：
        let tree_ids: HashSet<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();
        let leaf_ids: HashSet<&str> =
            ordered.iter().filter(|n| is_leaf(n, &children)).map(|n| n.id.as_str()).collect();
        for id in asm.committed_ids() {
            if !tree_ids.contains(id) {
                return Err(ExportError::GhostSection { section_id: id.to_string() });
            }
        }
        for id in asm.committed_ids() {
            if !leaf_ids.contains(id) {
                return Err(ExportError::RecordOnStructuralNode { section_id: id.to_string() });
            }
        }
        for node in &ordered {
            if !is_leaf(node, &children) {
                continue; // 结构节点：标题面，不查内容/状态（见模块头 fail-closed 第 5 条）
            }
            if !asm.has_committed(&node.id) {
                return Err(ExportError::MissingRecord { section_id: node.id.clone() });
            }
            if !is_drafted(node.status) {
                return Err(ExportError::UndraftedSection {
                    section_id: node.id.clone(),
                    status: node.status.as_str().to_string(),
                });
            }
        }

        // ⑤ 幂等 no-op：指纹全等 + 输出在位且 sha 吻合 → 上次报告直返
        // （输出零重写——「已写入的章不重写」的导出层落点）。
        let fingerprints: Vec<ChapterFingerprint> = ordered
            .iter()
            .filter(|n| is_leaf(n, &children))
            .map(|n| {
                // ④ 已保证叶有记录——unwrap 由前置门构造保证（不可达面）
                let rec = asm.record(&n.id).expect("叶记录已在导出门验证");
                ChapterFingerprint {
                    section_id: n.id.clone(),
                    attempt: rec.attempt,
                    content_sha256: sha256_hex(rec.content.as_bytes()),
                }
            })
            .collect();
        let manifest_path = dir.join(MANIFEST_FILE);
        if let Some(manifest) = read_manifest(&manifest_path)? {
            if manifest.fingerprints == fingerprints
                && out_path.is_file()
                && sha256_file(out_path)? == manifest.report.output_sha256
            {
                return Ok(manifest.report);
            }
        }

        // ⑥ 逐章构建 docx 模型（流式纪律：每章独立 add，永不拼巨串）。
        let mut docx = Docx::new();
        // Heading1..4 段落样式（w:name = "Heading 1".."Heading 4"——Word 内建
        // 样式名 + docx-rs Name::is_heading 同口径，TOC \o 识别层级的前提）。
        for lvl in 1..=4u8 {
            docx = docx.add_style(
                Style::new(format!("Heading{lvl}"), StyleType::Paragraph)
                    .name(format!("Heading {lvl}")),
            );
        }
        // #59 阶段 2：format-spec 应用（页面/边距/Normal 字体行距/页眉页脚
        // ——加性桥，不触碰下文的段落构建序）。None = 纯默认面（既有行为
        // 零变化——测试与旧调用方的兼容通道）。
        if let Some(spec) = opts.format_spec {
            docx = crate::format_spec::apply_to_docx(docx, spec, &opts.project_name);
        }
        // TOC 域插正文最前（Word F9 更新出目录；开关面见 toc_field 注释）。
        docx = docx.add_table_of_contents(toc_field());
        let mut chapters_written = 0usize;
        let mut total_chars = 0u64;
        for node in &ordered {
            // 标题段：Heading{level}（level 由 set_outline 保证 1..=4；防御性
            // clamp——手工构造的树不因此产出悬空 pStyle）。暗标：标题同步
            // 脱敏（章标题含公司名同样是暗标度标识别点）。
            let lvl = node.level.clamp(1, 4);
            let title = opts.scrub(node.title.as_str());
            let mut heading = Paragraph::new()
                .add_run(Run::new().add_text(title.as_str()))
                .style(&format!("Heading{lvl}"));
            if lvl == 1 {
                heading = heading.page_break_before(true); // 一级部件换页起（标书惯例）
            }
            docx = docx.add_paragraph(heading);
            if is_leaf(node, &children) {
                let rec = asm.record(&node.id).expect("叶记录已在导出门验证");
                // 正文段：按换行切段（LLM 章内容的段落结构保留；空行跳过——
                // 段落边界是排版面，不是内容面）。暗标：正文脱敏。
                for line in content_paragraphs(&rec.content) {
                    let line = opts.scrub(line);
                    docx = docx.add_paragraph(
                        Paragraph::new().add_run(Run::new().add_text(line.as_str())),
                    );
                }
                chapters_written += 1;
                total_chars += u64::from(rec.char_count);
            }
        }

        // ⑦ 段落 id 确定化：docx-rs 的 paraId 在 `Paragraph::new()` 时由**线程
        // 局部自增计数器**分配（vendored `documents/paragraph_id.rs`）——同进程
        // 内第二次构建从上次的计数值继续，`pack` 内部的 `reset_para_id` 只影响
        // 其后**新建**的段落（auto-TOC 扩展面），不回收已建段——document.xml
        // 因此逐字节不可复现（实测两次导出 paraId 面 `00000001..` vs
        // `000005A7..`）。导出层在 pack 前按**文档序**统一重排 id：
        // `00000001` 起单调递增——同输入恒同输出（S-6 确定性硬验收的前提；
        // id 唯一性由单调保证，`refresh_duplicate_para_ids` 零重派——vendored
        // 逐源核读，`auto=false` 的域式 TOC 不走 TocKey/bookmark 计数器面）。
        let mut next_para_id: u32 = 1;
        for child in docx.document.children.iter_mut() {
            if let DocumentChild::Paragraph(p) = child {
                p.id = format!("{next_para_id:08x}");
                next_para_id += 1;
            }
        }

        // ⑧ 原子落盘：tmp 写全量 → rename 替换（borrow：mdcore sidecar
        // tmp+rename 纪律）。pack 把 XML 部件流式写进 zip——rename 返回即
        // 导出安全，此后进程随时可死。
        let file = std::fs::File::create(&tmp_path).map_err(ExportError::Io)?;
        docx.pack(file).map_err(|e| ExportError::Pack(e.to_string()))?;
        std::fs::rename(&tmp_path, out_path).map_err(ExportError::Io)?;

        // ⑨ 产物指纹 + 幂等清单落盘（导出后产物不可变——sha 是纯读）。
        let output_sha256 = sha256_file(out_path)?;
        let report = ExportReport {
            chapters_written,
            total_chars,
            duration_ms: Some(started.elapsed().as_millis() as u64),
            output_sha256: output_sha256.clone(),
            toc_field_present: true, // 构造面：toc_field() 恒插（见字段注释）
        };
        let manifest = ExportManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            fingerprints,
            exported_at: now_secs(),
            report: report.clone(),
        };
        write_manifest_atomic(&manifest_path, &manifest)?;

        Ok(report)
    }
}

// ---------------------------------------------------------------- 私有辅助

/// 叶判定（与 assemble_all 同口径：无子节点者）——O(1) 查 children 索引。
fn is_leaf(node: &OutlineNode, children: &BTreeMap<&str, Vec<&OutlineNode>>) -> bool {
    !children.contains_key(&node.id[..])
}

/// 状态是否已达 DRAFTED（含 VALIDATING/REVISING/ACCEPTED——均为已定稿面；
/// 线性链位次 ≥ Drafted）。
fn is_drafted(s: SectionStatus) -> bool {
    matches!(
        s,
        SectionStatus::Drafted
            | SectionStatus::Validating
            | SectionStatus::Revising
            | SectionStatus::Accepted
    )
}

/// 文档序全序化：根集合 + 兄弟按 order 升序 → DFS 前序遍历。
///
/// 返回 (全序节点列表, children 索引)。父节点必先于子节点出现（DFS 前序
/// 保证——plain order 排序在「父 order > 子 order」的树上会输出子先于父的
/// 坏文档，DFS 无此风险）。**完整性对账**：遍历输出数 ≠ 输入数 → 有节点
/// 从根不可达（孤儿父引用 / 环 / 重复 id）→ [`ExportError::UnreachableSection`]
/// 定位到第一个失踪者（静默丢节点 = 导出半个文档，fail-closed）。
fn order_tree<'a>(
    nodes: &'a [OutlineNode],
) -> Result<(Vec<&'a OutlineNode>, BTreeMap<&'a str, Vec<&'a OutlineNode>>), ExportError> {
    if nodes.is_empty() {
        return Err(ExportError::EmptyTree);
    }
    // 重复 id 防御（set_outline 已拦——手工构造面的兜底；重复会让 DFS 双计
    // 或错挂，完整性对账只能发现数量对不上，先在此显式拒）。
    let mut seen: HashSet<&str> = HashSet::new();
    for n in nodes {
        if !seen.insert(&n.id[..]) {
            return Err(ExportError::UnreachableSection { section_id: n.id.clone() });
        }
    }
    let mut children: BTreeMap<&str, Vec<&OutlineNode>> = BTreeMap::new();
    let mut roots: Vec<&OutlineNode> = Vec::new();
    for n in nodes {
        match n.parent_id.as_deref() {
            None => roots.push(n),
            Some(p) => children.entry(p).or_default().push(n),
        }
    }
    roots.sort_by_key(|n| n.order);
    for v in children.values_mut() {
        v.sort_by_key(|n| n.order); // 兄弟序 = order 升序（order 全局唯一——稳定）
    }
    // DFS 前序（迭代式——显式栈，树深 ≤4 但迭代写法与数据规模解耦）。
    let mut ordered: Vec<&OutlineNode> = Vec::with_capacity(nodes.len());
    let mut stack: Vec<&OutlineNode> = roots.into_iter().rev().collect();
    while let Some(n) = stack.pop() {
        ordered.push(n);
        if let Some(kids) = children.get(&n.id[..]) {
            stack.extend(kids.iter().rev()); // 逆序入栈 → 弹出即文档序
        }
    }
    if ordered.len() != nodes.len() {
        // 定位首个失踪者：输入里第一个不在全序中的 id（确定性诊断面）
        let in_order: HashSet<&str> = ordered.iter().map(|n| &n.id[..]).collect();
        let missing = nodes
            .iter()
            .find(|n| !in_order.contains(&n.id[..]))
            .expect("数量不等必有失踪者");
        return Err(ExportError::UnreachableSection { section_id: missing.id.clone() });
    }
    Ok((ordered, children))
}

/// 章正文切段（\n 为界，剥 \r，跳过空段）——迭代器借用面，零复制。
fn content_paragraphs(content: &str) -> impl Iterator<Item = &str> {
    content
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.is_empty())
}

/// `out` + ".tmp" 的同目录兄弟路径（纯后缀追加——不猜调用方的扩展名形态）。
fn sibling_tmp(out: &Path) -> PathBuf {
    let mut s = out.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

/// 当前 UNIX_EPOCH 秒（no-chrono 纪律；时钟异常 → 0——单调性由 attempt 承载）。
fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// sha256（十六进制小写）——字节串一次性口径（章内容 ≤ 数 KB）。
fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// 文件流式 sha256（64KiB 块——产物 MB 序，不整读进内存）。
fn sha256_file(path: &Path) -> Result<String, ExportError> {
    let mut f = std::fs::File::open(path).map_err(ExportError::Io)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf).map_err(ExportError::Io)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// 读 manifest（**导数宽容**面：缺失/损坏/版本不识别 → None 全量重建；
/// 主数据的 fail-closed 在 ChapterAssembler::open——两层面各司其职，见
/// 模块头「幂等导出清单」节）。
fn read_manifest(path: &Path) -> Result<Option<ExportManifest>, ExportError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ExportError::Io(e)),
    };
    match serde_json::from_str::<ExportManifest>(&raw) {
        Ok(m) if m.schema_version == MANIFEST_SCHEMA_VERSION => Ok(Some(m)),
        _ => Ok(None), // 损坏/未来版本 → 无清单重建（导数不挡路）
    }
}

/// manifest 原子落盘（tmp+rename——borrow：mdcore sidecar save 同款纪律）。
fn write_manifest_atomic(path: &Path, manifest: &ExportManifest) -> Result<(), ExportError> {
    let json = serde_json::to_string_pretty(manifest).map_err(ExportError::Serialize)?;
    let tmp = sibling_tmp(path);
    std::fs::write(&tmp, json.as_bytes()).map_err(ExportError::Io)?;
    std::fs::rename(&tmp, path).map_err(ExportError::Io) // 原子提交点
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TOC 域开关渲染面（vendored docx-rs 单元测试同款断言——`TOC \o "1-3"
    /// \h \u` 的 XML 字面）。锚定「构造的域指令 == 规格原文」。
    #[test]
    fn toc_field_renders_spec_instruction() {
        use docx_rs::BuildXML as _;
        let xml = toc_field().instr.build();
        let s = String::from_utf8(xml).unwrap();
        assert!(
            s.contains(r#"TOC \o &quot;1-3&quot; \h \u"#),
            "TOC 域指令必须是规格原文（TOC \\o \"1-3\" \\h \\u，XML 引号转义形态）: {s}"
        );
    }

    /// 文档序全序化：兄弟按 order、父先于子；孤儿（父引用不存在）→ 显式拒。
    #[test]
    fn order_tree_orders_and_rejects_orphan() {
        let mk = |id: &str, parent: Option<&str>, level: u8, order: u32| OutlineNode {
            id: id.into(),
            parent_id: parent.map(str::to_string),
            title: format!("{id} 标题"),
            level,
            order,
            requirement_ids: vec!["REQ-0001".into()],
            target_chars: 100,
            status: SectionStatus::Drafted,
            dirty: false,
            content: None, content_attempt: 0,
        };
        // 正常态：根 A(order 1) → 子 A1(order 3)、A2(order 2)；根 B(order 4)。
        // A2 的 order < A1 但同为 A 的子——兄弟序按 order：A1 前还是后？
        // A1 order=3、A2 order=2 → 兄弟序 [A2, A1]（order 升序，非插入序）。
        let nodes = vec![
            mk("A", None, 1, 1),
            mk("A1", Some("A"), 2, 3),
            mk("A2", Some("A"), 2, 2),
            mk("B", None, 1, 4),
        ];
        let (ordered, children) = order_tree(&nodes).unwrap();
        let ids: Vec<&str> = ordered.iter().map(|n| &n.id[..]).collect();
        assert_eq!(ids, vec!["A", "A2", "A1", "B"], "父先于子 + 兄弟 order 升序");
        assert!(!is_leaf(&nodes[0], &children), "A 是结构节点");
        assert!(is_leaf(&nodes[1], &children), "A1 是叶");
        assert!(is_leaf(&nodes[3], &children), "B 是叶");

        // 孤儿：父引用不存在 → DFS 不可达 → 显式拒（不静默丢）
        let orphan = vec![mk("A", None, 1, 1), mk("X", Some("NOPE"), 2, 2)];
        match order_tree(&orphan) {
            Err(ExportError::UnreachableSection { section_id }) => {
                assert_eq!(section_id, "X", "报错定位到孤儿节点");
            }
            other => panic!("孤儿必须 UnreachableSection 拒，实际: {:?}", other.err()),
        }

        // 重复 id → 显式拒（手工构造面兜底）
        let dup = vec![mk("A", None, 1, 1), mk("A", None, 1, 2)];
        assert!(
            matches!(order_tree(&dup), Err(ExportError::UnreachableSection { .. })),
            "重复 id 必须拒"
        );

        // 空树 → EmptyTree
        assert!(matches!(order_tree(&[]), Err(ExportError::EmptyTree)));
    }
}

// ---------------------------------------------------------------- #59 阶段 2：服务面产物

/// `export_bid_document` 的服务层产物（MCP 壳组装契约 §8 out 的素材面）。
#[derive(Debug, Clone)]
pub struct ExportOutcome {
    pub report: ExportReport,
    /// 服务层告警（format-spec 降级 / 暗标漏配 bidder_name 等）
    pub warnings: Vec<String>,
    /// 产物绝对路径（壳层补 url/size 面）
    pub output_path: PathBuf,
}

/// `ExportError` → `bid_domain::ToolError`（服务层转发面——导出失败统一
/// E_UNPROCESSABLE 带原文；导出门（blocking/幽灵/半个文档）都是「不可处理
/// 的状态」，不是「对象不存在」或「状态机越级」）。
pub fn outcome_error(e: ExportError) -> bid_domain::ToolError {
    bid_domain::ToolError::EUnprocessable { reason: e.to_string() }
}
