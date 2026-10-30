//! service.rs —— MCP 后端服务层（薄壳纪律的「业务」半边，issue #11 B-1）。
//!
//! 由 `runner.rs` 的私有 `import_files` 提升而来：同一套
//! sha256 幂等 → `bid_parse::parse_file` 解析 → `DomainStore::upsert_file` 落库 →
//! 失败件显式登记（R-01）的确定性管线，供 MCP 壳（`bid_mcp::dispatch_with`）与
//! 场景运行器（runner）共用——单一真源，两边不各自维护副本。
//!
//! 分层纪律（docs/bid-mcp-contract.md §0）：
//! - 本层不做 MCP 参数校验（壳的职责），也不做状态机裁决（`guard_tool`，壳调用）；
//! - kind 枚举约束属 MCP 契约面（§2 七类），本层不重复——运行器可传自有类别
//!   （如「证据」），校验放壳层可让两侧各得其所；
//! - 失败语义（R-01 / R3-S2-4）：单件解析失败 → `Failed` 落库 + 禁引 + 继续，
//!   一个损坏件不得阻断整批导入；不可读 upload_ref 无文件身份（无 sha 可言），
//!   不落库（空 sha 会污染幂等键空间），逐件失败登记。

use bid_domain::{DomainStore, ParseStatus, Requirement, ToolError};
use bid_parse::{parse_file, sha256_file, ParsedFile};
use std::path::{Path, PathBuf};

/// 单文件导入规格（MCP 壳已完成契约校验后传入）。
#[derive(Debug, Clone)]
pub struct ImportSpec {
    /// 上传工件引用。AdapterA 本实现 = 文件系统路径（绝对路径稳妥——相对路径
    /// 按进程 CWD 解析）；未来对象存储句柄在同位替换，管线不变。
    pub upload_ref: PathBuf,
    /// 逻辑文件名（落库 `TenderFile.name` 与解析标签——与 upload_ref 解耦，
    /// 供 item_errors 对账与证据锚溯源）。
    pub filename: String,
    /// 业务类别（招标文件|附件|图纸|补遗|答疑|评分办法|格式模板；运行器侧
    /// 可用「证据」等自有类别——枚举校验属 MCP 壳，本层透传）。
    pub kind: String,
    /// 期望页数对账（契约 §2 `files[].expected_pages`；`None` = 不对账）。
    pub expected_pages: Option<u64>,
}

/// 单件导入结果（对账面：deduplicated / errors / warnings 逐件可见）。
#[derive(Debug, Clone)]
pub struct PerFileOutcome {
    /// 落库回执 file_id（`None` = 无文件身份——upload_ref 不可读，无档可报）。
    pub file_id: Option<String>,
    pub filename: String,
    pub sha256: String,
    pub pages: usize,
    pub tables: usize,
    pub chunks: usize,
    pub ocr_blocks: usize,
    /// OCR 分档（契约 §2 `per_file.ocr_tier`，见 [`ocr_tier_of`]）。
    pub ocr_tier: &'static str,
    pub parse_status: ParseStatus,
    pub deduplicated: bool,
    /// 逐件错误（解析失败 / 不可读 / 已登记失败件的重放）——E_PARTIAL 的
    /// item_errors 素材。
    pub errors: Vec<String>,
    /// 逐件对账告警（expected_pages 不符等——不阻断）。
    pub warnings: Vec<String>,
}

/// 一批导入的对账报告（壳层据此组装契约 §2 out 与 E_PARTIAL）。
#[derive(Debug, Clone)]
pub struct ImportReport {
    pub per_file: Vec<PerFileOutcome>,
    /// 本批条目失败数（解析失败 + 不可读 + 已登记失败件的重放）——
    /// 契约 §2 `totals.partial` 与 E_PARTIAL 判据。
    pub item_failures: usize,
    /// 本批新建档数（含 Failed 档）。> 0 且项目非 DRAFT → 契约 §2 dirty 追加
    /// （纯 no-op 重放不标 dirty——content_version 未动，无物可脏）。
    pub new_records: usize,
}

/// OCR 分档（契约 §2 `per_file.ocr_tier`）——从解析面信号推导的确定性档位：
/// - `"none"`：文本层在位，无需 OCR；
/// - `"ocr"`：无文本层但 OCR 已产出（`ocr_blocks > 0`）；
/// - `"pending"`：无文本层且 OCR 未产出（登记不伪装——待 OCR 通道补齐）；
/// - `"n/a"`：解析失败件，无内容面，档位不适用。
fn ocr_tier_of(status: ParseStatus, ocr_blocks: usize) -> &'static str {
    match status {
        ParseStatus::Failed => "n/a",
        ParseStatus::Ok => "none",
        ParseStatus::OcrRequired => {
            if ocr_blocks > 0 {
                "ocr"
            } else {
                "pending"
            }
        }
    }
}

/// 导入管线（契约 §2 业务半边）。
///
/// 逐件：sha256 幂等（已存在 → no-op 返回原 file_id，`deduplicated:true`）→
/// `parse_file` → `upsert_file` 落库；成功件的解析产物累积进 `parsed`
/// （M1-b `extract_requirements` 的输入面）。单件失败 → `Failed` 显式登记 +
/// 继续（R-01），绝不整批中止。
pub fn import_files(
    store: &mut DomainStore,
    parsed: &mut Vec<ParsedFile>,
    specs: &[ImportSpec],
) -> ImportReport {
    let mut report = ImportReport { per_file: Vec::with_capacity(specs.len()), item_failures: 0, new_records: 0 };
    for spec in specs {
        // 不可读 upload_ref：无文件身份——不落库（空 sha 会污染幂等键空间），逐件失败登记
        let sha = match sha256_file(&spec.upload_ref) {
            Ok(s) => s,
            Err(e) => {
                report.item_failures += 1;
                report.per_file.push(PerFileOutcome {
                    file_id: None,
                    filename: spec.filename.clone(),
                    sha256: String::new(),
                    pages: 0,
                    tables: 0,
                    chunks: 0,
                    ocr_blocks: 0,
                    ocr_tier: "n/a",
                    parse_status: ParseStatus::Failed,
                    deduplicated: false,
                    errors: vec![format!("upload_ref 不可读: {e}")],
                    warnings: vec![],
                });
                continue;
            }
        };
        // 幂等：sha256 已存在 → no-op 返回原 file_id（契约 §2）
        if let Some(f) = store.files().iter().find(|f| f.sha256 == sha).cloned() {
            let mut errors = Vec::new();
            if f.parse_status == ParseStatus::Failed {
                // 已登记失败件的重放：仍是失败面（同内容确定性重解析必同果）——
                // 如实计入条目失败；修复后的新内容 = 新 sha = 新档
                errors.push(format!(
                    "sha256 已登记为解析失败件（{}）——产物禁引；修复后以新内容重传",
                    f.parse_summary
                ));
            }
            report.item_failures += usize::from(!errors.is_empty());
            report.per_file.push(PerFileOutcome {
                file_id: Some(f.id),
                filename: f.name,
                sha256: f.sha256,
                pages: f.pages,
                tables: f.tables,
                chunks: f.chunks,
                ocr_blocks: f.ocr_blocks,
                ocr_tier: ocr_tier_of(f.parse_status, f.ocr_blocks),
                parse_status: f.parse_status,
                deduplicated: true,
                errors,
                warnings: vec![],
            });
            continue;
        }
        // B4 收敛（issue #11 任务 3）：docx 与 xlsx 走 bid-engine 引擎路径
        // （自研 docx/xlsx 解析已删——引擎是 OOXML 读面单一真源；B3-3 r3 差分
        // 验收 EQ+ED 91.57%，A 件等抽样 100% EQ）；pdf 引擎优先（pdfium 文本层）
        // + 旧路径回退（pdftotext/OCR 补遗链——`parse_file` B4 后仅剩 pdf 面，
        // 见 bid-parse/src/lib.rs 模块头）；.xls/.ods 等其它扩展名 → `parse_file`
        // 显式拒绝（「不支持的文件类型」——zip 容器载具不误解析）。
        // 已知引擎缺口登记不伪装：域剥离边界（mdcore#40）——详见
        // docs/b3-3-r3-acceptance-report.md（嵌套表 mdcore#39 已修复）。
        let ext = file_extension(&spec.upload_ref, &spec.filename);
        let parsed_result = match ext.as_str() {
            "docx" => bid_engine::parse_docx_via_engine(&spec.upload_ref)
                .map_err(|e| e.to_string()),
            "xlsx" => bid_engine::parse_xlsx_via_engine(&spec.upload_ref)
                .map_err(|e| e.to_string()),
            // B3-5：pdf 引擎通道（pdfium 文本层）——扫描件（MappingGap）或
            // 无 pdfium（EngineFailed）回退旧 pdftotext/OCR 路径（映射表 §3）
            "pdf" => bid_engine::parse_pdf_via_engine(&spec.upload_ref)
                .or_else(|_| parse_file(&spec.upload_ref, &spec.filename)),
            _ => parse_file(&spec.upload_ref, &spec.filename),
        };
        match parsed_result {
            Ok(p) => {
                let status = if p.has_text_layer { ParseStatus::Ok } else { ParseStatus::OcrRequired };
                let chunks = p.blocks.len();
                let (ocr_blocks, pages, tables) = (p.ocr_blocks, p.pages, p.tables);
                let summary = p.parse_summary.clone();
                let (fid, _) = match store.upsert_file(
                    &spec.filename, &spec.kind, &sha, pages, tables, chunks, ocr_blocks, status, &summary,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        report.item_failures += 1;
                        report.per_file.push(PerFileOutcome {
                            file_id: None,
                            filename: spec.filename.clone(),
                            sha256: sha.clone(),
                            pages, tables, chunks, ocr_blocks,
                            ocr_tier: ocr_tier_of(status, ocr_blocks),
                            parse_status: status,
                            deduplicated: false,
                            errors: vec![format!("落库被拒: {}", e.code())],
                            warnings: vec![],
                        });
                        continue;
                    }
                };
                // 解析失败禁引（R-01）：failed 件不进 parsed 累积（不入抽取/索引面）
                parsed.push(p);
                let mut warnings = Vec::new();
                if let Some(exp) = spec.expected_pages {
                    if exp != pages as u64 {
                        warnings.push(format!(
                            "EXPECTED_PAGES_MISMATCH: 期望 {exp} 页，实际 {pages} 页（页数口径因格式而异——docx 为结构页 1+章数，pdf 为文本层/渲染页）"
                        ));
                    }
                }
                report.new_records += 1;
                report.per_file.push(PerFileOutcome {
                    file_id: Some(fid),
                    filename: spec.filename.clone(),
                    sha256: sha.clone(),
                    pages,
                    tables,
                    chunks,
                    ocr_blocks,
                    ocr_tier: ocr_tier_of(status, ocr_blocks),
                    parse_status: status,
                    deduplicated: false,
                    errors: vec![],
                    warnings,
                });
            }
            Err(e) => {
                // R3-S2-4 / R-01：失败件显式登记（Failed + 禁引）+ 继续——不整批中止
                let (fid, _) = match store.upsert_file(
                    &spec.filename, &spec.kind, &sha, 0, 0, 0, 0, ParseStatus::Failed, &e,
                ) {
                    Ok(v) => v,
                    Err(e2) => {
                        report.item_failures += 1;
                        report.per_file.push(PerFileOutcome {
                            file_id: None,
                            filename: spec.filename.clone(),
                            sha256: sha.clone(),
                            pages: 0, tables: 0, chunks: 0, ocr_blocks: 0,
                            ocr_tier: "n/a",
                            parse_status: ParseStatus::Failed,
                            deduplicated: false,
                            errors: vec![format!("失败登记被拒: {}", e2.code())],
                            warnings: vec![],
                        });
                        continue;
                    }
                };
                report.item_failures += 1;
                report.new_records += 1; // Failed 档也是新档（content_version 推进 + dirty 面）
                report.per_file.push(PerFileOutcome {
                    file_id: Some(fid),
                    filename: spec.filename.clone(),
                    sha256: sha.clone(),
                    pages: 0,
                    tables: 0,
                    chunks: 0,
                    ocr_blocks: 0,
                    ocr_tier: "n/a",
                    parse_status: ParseStatus::Failed,
                    deduplicated: false,
                    errors: vec![e],
                    warnings: vec![],
                });
            }
        }
    }
    report
}

/// MCP 后端服务态：跨工具调用持有 `DomainStore`（权威状态机存储）与解析产物
/// 累积（M1-b 抽取输入面）。
///
/// 默认 stdio 传输用进程级单例（`bid_mcp::state`）；测试与嵌入方经
/// `bid_mcp::dispatch_with` 注入独立实例（并行隔离，不共享全局）。
///
/// T-8 持久化（V3-1）：`open(dir)` 打开（或创建）磁盘服务态——重启续跑；
/// `new()` 仍为纯内存态（35 个既有测试零行为变化）。写后自动 checkpoint
/// （`persist` 在场时）：域状态落盘失败 → 业务结果携带 E_UNPROCESSABLE
/// 响亮报错（内存已变更但磁盘未跟上——调用方不得当作成功）；
/// E_INTERNAL 词典落地时升格错误码（mcp-contract §0 已有，domain 词典暂缺）。
pub struct BidService {
    store: DomainStore,
    parsed: Vec<ParsedFile>,
    persist: Option<crate::store_io::StoreFile>,
}

impl BidService {
    #[must_use]
    pub fn new() -> Self {
        Self { store: DomainStore::new(), parsed: vec![], persist: None }
    }

    /// 打开（或首启创建）持久化服务态：目录有 `db.json` 则恢复（重启续跑——
    /// T-8 验收「建项→导入→抽条款→重启→状态完整恢复」的入口），否则空态起步。
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, crate::store_io::StoreIoError> {
        let sf = crate::store_io::StoreFile::new(dir);
        let (snap, parsed) = match sf.load()? {
            Some((s, p)) => (Some(s), p),
            None => (None, vec![]),
        };
        let store = match snap {
            Some(s) => DomainStore::restore(s).map_err(|e| {
                crate::store_io::StoreIoError::Corrupt(format!("域语义不完整: {} {:?}", e.code(), e))
            })?,
            None => DomainStore::new(),
        };
        Ok(Self { store, parsed, persist: Some(sf) })
    }

    /// 显式落盘（幂等纯 IO——外部对账/退出前钩子用；写路径已自动 checkpoint）。
    pub fn checkpoint(&self) -> Result<(), crate::store_io::StoreIoError> {
        match &self.persist {
            Some(sf) => sf.save(&self.store.to_snapshot(), &self.parsed),
            None => Ok(()), // 纯内存态（默认）——no-op
        }
    }

    fn auto_checkpoint(&self) -> Result<(), ToolError> {
        self.checkpoint().map_err(|e| ToolError::EUnprocessable {
            reason: format!("checkpoint 落盘失败（内存已变更、磁盘未跟上——不得当作成功）: {e}"),
        })
    }

    pub fn store(&self) -> &DomainStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut DomainStore {
        &mut self.store
    }

    /// 持久化工作区目录（persist 在场时——章节内容/导出产物的落盘锚点；
    /// 纯内存态返回 None，调用方自行降级）。#59 阶段 1：draft_section 的
    /// S-2 落盘面用。
    pub fn workspace_dir(&self) -> Option<std::path::PathBuf> {
        self.persist.as_ref().map(|sf| sf.dir().to_path_buf())
    }

    /// 已成功解析的文件面（`extract_requirements` 输入——M1-b 接线位）。
    pub fn parsed(&self) -> &[ParsedFile] {
        &self.parsed
    }

    /// `create_bid_project` 业务执行（T-8：持久化在场时写后自动落盘）。
    pub fn create_project(&mut self, name: &str, code: &str, mode: &str,
                          external_key: &str) -> Result<(String, serde_json::Value), ToolError> {
        let r = self.store.create_project(name, code, mode, external_key)?;
        self.auto_checkpoint()?;
        Ok(r)
    }

    /// `extract_requirements` 落库半边（T-8：持久化在场时写后自动落盘）。
    pub fn set_requirements(&mut self, reqs: Vec<Requirement>) -> Result<(usize, usize), ToolError> {
        let r = self.store.set_requirements(reqs)?;
        self.auto_checkpoint()?;
        Ok(r)
    }

    /// `draft_response_section` 的前置准备面（#59 阶段 1：抽取完成后
    /// 「推导大纲 → 构建矩阵」的服务通道——MCP 12 工具面不含建纲/建矩阵
    /// 工具（外部 Agent 经 runner / 测试经本方法准备；矩阵构建会推进项目
    /// 到 MATRIX_READY，draft 的 guard_tool 由此放行）。
    ///
    /// 返回 (章节数, 矩阵行数, blocking 行数)。
    pub fn prepare_outline_and_matrix(
        &mut self,
        spec: bid_domain::outline::OutlineSpec,
    ) -> Result<(usize, usize, usize), ToolError> {
        let (n_sections, _total_chars) = self.store.derive_and_set_outline(spec)?;
        let (n_rows, n_blocking) = self.store.build_matrix()?;
        self.auto_checkpoint()?;
        Ok((n_sections, n_rows, n_blocking))
    }

    /// `import_tender_files` 业务执行（壳已完成参数校验 / 状态机裁决 /
    /// 乐观并发检查——本层只跑管线；T-8：持久化在场时写后自动落盘）。
    pub fn import(&mut self, specs: &[ImportSpec]) -> Result<ImportReport, ToolError> {
        let report = import_files(&mut self.store, &mut self.parsed, specs);
        self.auto_checkpoint()?;
        Ok(report)
    }

    /// `export_bid_document` 业务半边（#59 阶段 2）。
    ///
    /// 步骤：
    /// 1. **双前置门**（防废标的最后关口——fail-closed）：
    ///    a. 矩阵 blocking 行清零（`check_export_ready`——必选未响应不得导出）；
    ///    b. 审核 BLOCKER findings 清零（`export_allowed`——T-7 闸门）；
    /// 2. format-spec 解析：`.mang/format-spec.json` 在场 → 读盘校验；
    ///    缺失 → `derive_from_requirements`（从格式条款提取；零命中 → 基线
    ///    + warning，Lead 铁律「不硬编码」的降级面）；
    /// 3. `DocumentExporter::export_with`（S-6 既有导出面 + format-spec 应用
    ///    + 暗标脱敏）；
    /// 4. 状态推进 EXPORTED（合法链补齐——Drafting 起的完整链）；
    /// 5. checkpoint。
    ///
    /// 暗标（anonymize = Some(bidder)）：正文与章标题的公司名 → 「投标人」。
    /// bidder 为空 → 响亮 warning（无名字可替——用户漏配，不静默）。
    pub fn export_document(
        &mut self,
        format: &str,
        anonymize: Option<&str>,
        output_path: &std::path::Path,
    ) -> Result<crate::export::ExportOutcome, ToolError> {
        // ---- 0. format 面（壳层已拦 pdf——服务层防御性兜底，直调方同规矩）
        if format != "docx" {
            return Err(ToolError::EUnprocessable {
                reason: format!("format={format} 暂不支持（#59 阶段 2 仅 docx——PDF 是阶段 7 LibreOffice 通道）"),
            });
        }
        // ---- 1a. 矩阵门（blocking 行未清零 → 逐行列出，E_UNPROCESSABLE）
        self.store.check_export_ready()?;
        // ---- 1b. 审核门（BLOCKER findings 未清 → E_UNPROCESSABLE）
        self.store.export_allowed().map_err(ToolError::from)?;

        // ---- 2. format-spec（读盘优先，derive 兜底）
        let mut warnings: Vec<String> = Vec::new();
        let spec = self.load_or_derive_format_spec(&mut warnings)?;
        let anonymize_effective = match anonymize {
            Some(bidder) if bidder.trim().is_empty() => {
                warnings.push(
                    "暗标模式但 bidder_name 为空——正文未做公司名脱敏（补 bidder_name 后重导）".into(),
                );
                None
            }
            other => other,
        };

        // ---- 3. 导出（S-6 既有面 + 选项）
        let exporter = crate::export::DocumentExporter::new(self.store.sections());
        let dir = self.workspace_dir().ok_or_else(|| ToolError::EUnprocessable {
            reason: "导出需要持久化工作区（BidService::open 建立的服务态才有章内容可导）".into(),
        })?;
        let opts = crate::export::ExportOptions {
            format_spec: Some(&spec),
            project_name: self.store.project()?.name.clone(),
            anonymize: anonymize_effective,
        };
        let report = exporter
            .export_with(&dir, output_path, &opts)
            .map_err(crate::export::outcome_error)?;

        // ---- 4. 状态推进 EXPORTED（合法链补齐）
        advance_project_to_exported(&mut self.store)?;

        // ---- 5. checkpoint（导出产物路径在 ExportOutcome，磁盘面已由导出器
        //      原子落盘；这里落的是域状态 EXPORTED）
        self.auto_checkpoint()?;

        Ok(crate::export::ExportOutcome {
            report,
            warnings,
            output_path: output_path.to_path_buf(),
        })
    }

    /// format-spec 装载：`.mang/format-spec.json` 读盘 → validate；缺失 →
    /// 从格式条款 derive（零命中 → 基线 + warning）。
    fn load_or_derive_format_spec(
        &self,
        warnings: &mut Vec<String>,
    ) -> Result<crate::format_spec::FormatSpec, ToolError> {
        use crate::format_spec::{baseline, derive_from_requirements, validate};
        if let Some(dir) = self.workspace_dir() {
            let spec_path = dir.join(".mang").join("format-spec.json");
            if spec_path.is_file() {
                let raw = std::fs::read_to_string(&spec_path).map_err(|e| ToolError::EUnprocessable {
                    reason: format!("format-spec.json 读取失败: {e}"),
                })?;
                let spec: crate::format_spec::FormatSpec = serde_json::from_str(&raw)
                    .map_err(|e| ToolError::EUnprocessable {
                        reason: format!("format-spec.json 解析失败（schema 见 docs / format_spec.rs serde 面）: {e}"),
                    })?;
                validate(&spec).map_err(|e| ToolError::EUnprocessable {
                    reason: format!("format-spec.json 域校验失败: {e}"),
                })?;
                warnings.push("format-spec 从 .mang/format-spec.json 装载（手写 spec 优先于条款提取）".into());
                return Ok(spec);
            }
        }
        let (spec, mut w) = derive_from_requirements(self.store.requirements());
        warnings.append(&mut w);
        let _ = baseline(); // 基线在 derive 内部承载
        Ok(spec)
    }
}

impl Default for BidService {
    fn default() -> Self {
        Self::new()
    }
}

/// 项目推进 EXPORTED 的合法链补齐（#59 阶段 2——export 的成功后半边）。
///
/// guard_tool（壳层）已保证项目在 Drafting+；此处逐态经 `next_allowed`
/// 补齐（Drafting → Validating → Reviewing → ReadyToExport → Exported 的
/// 完整链）；已是 EXPORTED → no-op（重导幂等）。前段态直达 → 状态机错误
/// 透传（防御面——guard 不应放行）。
fn advance_project_to_exported(store: &mut DomainStore) -> Result<(), ToolError> {
    use bid_domain::ProjectStatus::*;
    let cur = store.project()?.status;
    let steps: &[bid_domain::ProjectStatus] = match cur {
        Drafting => &[Validating, Reviewing, ReadyToExport, Exported],
        Validating => &[Reviewing, ReadyToExport, Exported],
        Reviewing => &[ReadyToExport, Exported],
        ReadyToExport => &[Exported],
        Exported => &[],
        Draft | FilesImported | RequirementsExtracted | MatrixReady | EvidenceBound => {
            return Err(ToolError::EState { current: cur, attempted: "advance_project_status" });
        }
    };
    for to in steps {
        store.advance_project_status(*to)?;
    }
    Ok(())
}

/// B3-4：文件扩展名（引擎路径路由——小写归一，upload_ref 优先、filename 兜底）。
fn file_extension(path: &std::path::Path, filename: &str) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .or_else(|| {
            std::path::Path::new(filename)
                .extension()
                .and_then(|e| e.to_str())
        })
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}
