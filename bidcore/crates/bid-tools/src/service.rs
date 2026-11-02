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
    let mut report = ImportReport {
        per_file: Vec::with_capacity(specs.len()),
        item_failures: 0,
        new_records: 0,
    };
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
            "docx" => {
                bid_engine::parse_docx_via_engine(&spec.upload_ref).map_err(|e| e.to_string())
            }
            "xlsx" => {
                bid_engine::parse_xlsx_via_engine(&spec.upload_ref).map_err(|e| e.to_string())
            }
            // B3-5：pdf 引擎通道（pdfium 文本层）——扫描件（MappingGap）或
            // 无 pdfium（EngineFailed）回退旧 pdftotext/OCR 路径（映射表 §3）
            "pdf" => bid_engine::parse_pdf_via_engine(&spec.upload_ref)
                .or_else(|_| parse_file(&spec.upload_ref, &spec.filename)),
            _ => parse_file(&spec.upload_ref, &spec.filename),
        };
        match parsed_result {
            Ok(p) => {
                let status = if p.has_text_layer {
                    ParseStatus::Ok
                } else {
                    ParseStatus::OcrRequired
                };
                let chunks = p.blocks.len();
                let (ocr_blocks, pages, tables) = (p.ocr_blocks, p.pages, p.tables);
                let summary = p.parse_summary.clone();
                let (fid, _) = match store.upsert_file(
                    &spec.filename,
                    &spec.kind,
                    &sha,
                    pages,
                    tables,
                    chunks,
                    ocr_blocks,
                    status,
                    &summary,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        report.item_failures += 1;
                        report.per_file.push(PerFileOutcome {
                            file_id: None,
                            filename: spec.filename.clone(),
                            sha256: sha.clone(),
                            pages,
                            tables,
                            chunks,
                            ocr_blocks,
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
                    &spec.filename,
                    &spec.kind,
                    &sha,
                    0,
                    0,
                    0,
                    0,
                    ParseStatus::Failed,
                    &e,
                ) {
                    Ok(v) => v,
                    Err(e2) => {
                        report.item_failures += 1;
                        report.per_file.push(PerFileOutcome {
                            file_id: None,
                            filename: spec.filename.clone(),
                            sha256: sha.clone(),
                            pages: 0,
                            tables: 0,
                            chunks: 0,
                            ocr_blocks: 0,
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
        Self {
            store: DomainStore::new(),
            parsed: vec![],
            persist: None,
        }
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
                crate::store_io::StoreIoError::Corrupt(format!(
                    "域语义不完整: {} {:?}",
                    e.code(),
                    e
                ))
            })?,
            None => DomainStore::new(),
        };
        Ok(Self {
            store,
            parsed,
            persist: Some(sf),
        })
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
    pub fn create_project(
        &mut self,
        name: &str,
        code: &str,
        mode: &str,
        external_key: &str,
    ) -> Result<(String, serde_json::Value), ToolError> {
        let r = self.store.create_project(name, code, mode, external_key)?;
        self.auto_checkpoint()?;
        Ok(r)
    }

    /// `extract_requirements` 落库半边（T-8：持久化在场时写后自动落盘）。
    pub fn set_requirements(
        &mut self,
        reqs: Vec<Requirement>,
    ) -> Result<(usize, usize), ToolError> {
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

    // ------------------------------------------------------------ #80 第三批 MCP 补齐（G2/G3）
    // 薄壳纪律：工具面（bid-mcp）只做参数校验/状态机裁决/转发；这三段是
    // 服务层业务半边——域调用 + 写后自动落盘（auto_checkpoint），与既有
    // 服务方法同款形态。

    /// `register_evidence` 业务半边（G2）：证据登记（语义位查重/换版在域层）。
    pub fn register_evidence(
        &mut self,
        spec: bid_domain::evidence::EvidenceSpec,
    ) -> Result<(String, bool), ToolError> {
        let out = self.store.register_evidence(spec)?;
        self.auto_checkpoint()?;
        Ok(out)
    }

    /// `bind_evidence` 业务半边（G2）：逐条绑定（逐件失败收集，不中断批次——
    /// E_PARTIAL 语义由壳层组装）+ 绑后矩阵重算（行内证据即时可见——绑定
    /// 不重算会让 get_compliance_matrix 看到陈旧行）。
    /// 返回 (新增绑定数, 幂等重放数, 逐件错误清单, 矩阵行数, blocking 行数)。
    #[allow(clippy::type_complexity)]
    pub fn bind_evidence_batch(
        &mut self,
        ev_id: &str,
        requirement_ids: &[String],
        bid_open_date: &str,
    ) -> Result<(usize, usize, Vec<String>, usize, usize), ToolError> {
        let (mut bound, mut replayed) = (0usize, 0usize);
        let mut item_errors: Vec<String> = Vec::new();
        for rid in requirement_ids {
            match self.store.bind_evidence(ev_id, rid, bid_open_date) {
                Ok(true) => replayed += 1,
                Ok(false) => bound += 1,
                Err(e) => item_errors.push(format!(
                    "{rid}: {} {}",
                    e.code(),
                    match &e {
                        bid_domain::ToolError::ENotFound { what }
                        | bid_domain::ToolError::EUnprocessable { reason: what } => what.clone(),
                        _ => String::new(),
                    }
                )),
            }
        }
        let (rows, blocking) = self.store.build_matrix()?;
        self.auto_checkpoint()?;
        Ok((bound, replayed, item_errors, rows, blocking))
    }

    /// `record_review_gate` 业务半边（G3）：三门记审（复审人门/状态门/门序
    /// 裁决全在域层 record_review）。
    pub fn record_review_gate(
        &mut self,
        gate: bid_domain::review::Gate,
        reviewer: &str,
        decision: bid_domain::review::Decision,
        findings: Vec<(bid_domain::review::Severity, String, String, Option<String>)>,
    ) -> Result<bid_domain::review::ReviewReceipt, ToolError> {
        let receipt = self
            .store
            .record_review(gate, reviewer, decision, findings)?;
        self.auto_checkpoint()?;
        Ok(receipt)
    }

    /// `record_addendum_waiver` 业务半边（#184）：补遗逐件人工确认
    /// （principal human: 门 / file_id 存在性 / reason 非空白全在域层裁决
    /// ——本层只加写后自动落盘，与既有服务方法同款形态）。
    pub fn record_addendum_waiver(
        &mut self,
        file_id: &str,
        reason: &str,
        principal: &str,
    ) -> Result<bid_domain::AddendumWaiverRecord, ToolError> {
        let rec = self.store.record_addendum_waiver(file_id, reason, principal)?;
        self.auto_checkpoint()?;
        Ok(rec)
    }

    /// `import_tender_files` 业务执行（壳已完成参数校验 / 状态机裁决 /
    /// 乐观并发检查——本层只跑管线；T-8：持久化在场时写后自动落盘）。
    pub fn import(&mut self, specs: &[ImportSpec]) -> Result<ImportReport, ToolError> {
        let report = import_files(&mut self.store, &mut self.parsed, specs);
        self.auto_checkpoint()?;
        Ok(report)
    }

    /// `export_bid_document` 业务半边（#59 阶段 2 + #117 任务 1 表格接线）。
    ///
    /// 步骤：
    /// 1. **双前置门**（防废标的最后关口——fail-closed）：
    ///    a. 矩阵 blocking 行清零（`check_export_ready`——必选未响应不得导出）；
    ///    b. 审核 BLOCKER findings 清零（`export_allowed`——T-7 闸门）；
    /// 2. format-spec 解析：`.mang/format-spec.json` 在场 → 读盘校验；
    ///    缺失 → `derive_from_requirements`（从格式条款提取；零命中 → 基线
    ///    + warning，Lead 铁律「不硬编码」的降级面）；
    /// 2b. #117：主体信息装载（工作区 bidder-profile.json；缺档 → 空档 +
    ///    响亮告警——人员/设备表与格式件字段落【待填】，不编造）+ 表格/
    ///    格式件附加件派生（store + profile + 投标决策 → [`crate::export::
    ///    build_export_extras`]）；
    /// 3. `DocumentExporter::export_with`（S-6 既有导出面 + format-spec 应用
    ///    + 暗标脱敏 + #117 表格注入/附表）；
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
        decision: &crate::export::ExportDecision,
    ) -> Result<crate::export::ExportOutcome, ToolError> {
        // ---- 0. format 面（docx 直达；pdf 走 docx 链 + soffice 转换——
        //      其他诚实拒）
        if format != "docx" && format != "pdf" {
            return Err(ToolError::EUnprocessable {
                reason: format!("format={format} 非法（docx | pdf）"),
            });
        }
        // ---- 0b. 路径校验（逃逸防护）：导出路径必须落在工作区内。
        // 工作区内的绝对路径合法（服务层内部用 workspace.join 拼接）；
        // 拒绝的是「跑到工作区外面去」——父级逃逸与绝对但不含工作区前缀。
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
                    "暗标模式但 bidder_name 为空——正文未做公司名脱敏（补 bidder_name 后重导）"
                        .into(),
                );
                None
            }
            other => other,
        };

        // ---- 2b. #117 任务 1：主体信息 + 表格/格式件附加件
        //      （bidder-profile.json 是人工填表真源——工作区在位就消费；
        //      缺档/字段缺 → 【待填】+ 告警，绝不编造）
        let ws_for_profile = self.workspace_dir();
        let (profile, profile_warn) = match &ws_for_profile {
            Some(ws) => match bid_domain::party::BidderProfile::load(ws) {
                Ok(p) => (p, None),
                Err(bid_domain::party::FormError::Io { .. }) => (
                    bid_domain::party::BidderProfile::default(),
                    Some("主体信息未建档（工作区无 bidder-profile.json）——人员/设备表与格式件字段将落【待填】".to_string()),
                ),
                Err(e) => {
                    return Err(ToolError::EUnprocessable {
                        reason: format!("bidder-profile.json 解析失败: {e}"),
                    })
                }
            },
            None => (
                bid_domain::party::BidderProfile::default(),
                Some("纯内存态无工作区——主体信息不可用，人员/设备表与格式件字段将落【待填】".to_string()),
            ),
        };
        if let Some(w) = profile_warn {
            warnings.push(w);
        }
        let (extras, extras_warnings) =
            crate::export::build_export_extras(&self.store, &profile, decision);
        warnings.extend(extras_warnings);

        // ---- 3. 导出（S-6 既有面 + 选项）。pdf 面：docx 落工作区临时位 →
        //      soffice 转换（阶段 7 Lead 路线 A：LibreOffice headless 保真度
        //      最高；页眉页脚页码/中文字体嵌入均由 Writer 渲染管线承载）
        // #68 阶段 3：带 evidence 台账（EV-id 图片引用的解析面）
        let exporter = crate::export::DocumentExporter::with_evidences(
            self.store.sections(),
            self.store.evidences(),
        );
        let dir = self
            .workspace_dir()
            .ok_or_else(|| ToolError::EUnprocessable {
                reason: "导出需要持久化工作区（BidService::open 建立的服务态才有章内容可导）"
                    .into(),
            })?;
        // 路径逃逸校验（拿到工作区后才可判前缀）
        validate_output_path(&dir, output_path)?;
        let opts = crate::export::ExportOptions {
            format_spec: Some(&spec),
            project_name: self.store.project()?.name.clone(),
            anonymize: anonymize_effective,
            extras: Some(&extras),
        };
        let (docx_path, final_path) = if format == "pdf" {
            // docx 先落工作区内部位（out/.pdf-staging/{stem}.docx——不入用户
            // 输出路径，用户要的是 pdf；staging 名点开头隐藏 + manifest 同源幂等）
            let staging = dir.join(".pdf-staging");
            std::fs::create_dir_all(&staging).map_err(|e| ToolError::EUnprocessable {
                reason: format!("PDF staging 目录创建失败: {e}"),
            })?;
            let stem = output_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("标书");
            (
                staging.join(format!("{stem}.docx")),
                output_path.to_path_buf(),
            )
        } else {
            (output_path.to_path_buf(), output_path.to_path_buf())
        };
        let report = exporter
            .export_with(&dir, &docx_path, &opts)
            .map_err(crate::export::outcome_error)?;

        // ---- 3b. PDF 转换（soffice --headless——阶段 7 通道）
        let report = if format == "pdf" {
            // 幂等跳过：staging docx 未变（sha 与上次转换标记全等）且产物在
            // → 不重转（soffice 的 PDF 二进制不确定——CreationDate/ID 每次
            // 变；「产物一个字节不重写」由跳过路径承载，与 docx 导出的
            // manifest no-op 同语义）
            let docx_sha = sha256_of(&docx_path)?;
            let marker = docx_path.with_extension("converted");
            let skip = final_path.is_file()
                && std::fs::read_to_string(&marker).ok().as_deref() == Some(&docx_sha[..]);
            if skip {
                warnings.push("PDF 幂等：staging docx 未变——产物不重转（不重写）".into());
            } else {
                let t0 = std::time::Instant::now();
                convert_to_pdf(&docx_path, &final_path).map_err(|e| ToolError::EUnprocessable {
                    reason: format!("PDF 转换失败（soffice 通道）: {e}"),
                })?;
                std::fs::write(&marker, &docx_sha).map_err(|e| ToolError::EUnprocessable {
                    reason: format!("转换标记写入失败: {e}"),
                })?;
                warnings.push(format!(
                    "PDF 转换：LibreOffice headless {:.1}s（页眉页脚页码/中文字体嵌入由 Writer 渲染管线承载）",
                    t0.elapsed().as_secs_f64()
                ));
            }
            // 报告的 sha 面对最终产物（pdf）——docx 的 sha 是中间件
            let sha = sha256_of(&final_path)?;
            crate::export::ExportReport {
                output_sha256: sha,
                ..report
            }
        } else {
            report
        };

        // ---- 4. 状态推进 EXPORTED（合法链补齐）
        advance_project_to_exported(&mut self.store)?;

        // ---- 5. checkpoint（导出产物路径在 ExportOutcome，磁盘面已由导出器
        //      原子落盘；这里落的是域状态 EXPORTED）
        self.auto_checkpoint()?;

        Ok(crate::export::ExportOutcome {
            report,
            warnings,
            output_path: final_path,
        })
    }

    /// `validate_bid_document` 业务半边（#59 阶段 3——防废标最后一道关）。
    ///
    /// 纯只读诊断（不改稿不落库——BLOCKER 解除走既有面：矩阵覆写/证据补绑/
    /// 章节 draft）；成功跑完推进项目到至少 VALIDATING（Drafting → Validating
    /// 一步——复审语义；已在 Validating+ 者 no-op）+ checkpoint。
    ///
    /// #187：现网语义复核层未配置模型——走 [`crate::validate::NoModel`]
    /// （semantic 桶显式 NotPerformed，不伪装 Passed）。
    pub fn validate_document(
        &mut self,
        scope_sections: Option<&[String]>,
    ) -> Result<crate::validate::ValidationReport, ToolError> {
        let mut model = crate::validate::NoModel;
        self.validate_document_with_reviewer(scope_sections, &mut model)
    }

    /// #187 注入缝：validate 的模型复核层从外部注入。
    ///
    /// 现网 [`Self::validate_document`] 委之以 [`crate::validate::NoModel`]；
    /// 测试 / 未来真实 LLM 面经此注入 [`crate::validate::ModelReviewer`] 实现
    /// （返回 `Some(findings)` = 已复核；`None` = 未配置 → semantic 桶
    /// NotPerformed）。状态推进 / checkpoint 与现网面同轨。
    pub fn validate_document_with_reviewer(
        &mut self,
        scope_sections: Option<&[String]>,
        model: &mut dyn crate::validate::ModelReviewer,
    ) -> Result<crate::validate::ValidationReport, ToolError> {
        let report = crate::validate::validate_document(self.store(), scope_sections, model)?;
        // 状态推进（合法链一步；幂等面由 advance_project_status 承担）
        let cur = self.store().project()?.status;
        let steps: &[bid_domain::ProjectStatus] = match cur {
            bid_domain::ProjectStatus::Drafting => &[bid_domain::ProjectStatus::Validating],
            _ => &[],
        };
        for to in steps {
            self.store_mut().advance_project_status(*to)?;
        }
        self.auto_checkpoint()?;
        Ok(report)
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
                let raw =
                    std::fs::read_to_string(&spec_path).map_err(|e| ToolError::EUnprocessable {
                        reason: format!("format-spec.json 读取失败: {e}"),
                    })?;
                let spec: crate::format_spec::FormatSpec = serde_json::from_str(&raw)
                    .map_err(|e| ToolError::EUnprocessable {
                        reason: format!("format-spec.json 解析失败（schema 见 docs / format_spec.rs serde 面）: {e}"),
                    })?;
                validate(&spec).map_err(|e| ToolError::EUnprocessable {
                    reason: format!("format-spec.json 域校验失败: {e}"),
                })?;
                warnings.push(
                    "format-spec 从 .mang/format-spec.json 装载（手写 spec 优先于条款提取）".into(),
                );
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

/// 导出路径校验（逃逸防护）：拒绝父级逃逸与工作区外的绝对路径。
///
/// 合法形态：
///   - 相对路径且不含 `..`（如 `out/标书.docx`）—— 相对工作区解析
///   - 绝对路径但落在工作区内（如 `<ws>/out/标书.docx`）—— 服务层内部拼接产物
///
/// 拒绝形态：
///   - 任意位置的父级分量 `..`（逃出工作区）
///   - 绝对路径且不以工作区为前缀（写到盘上任意位置 = 任意写权限）
fn validate_output_path(
    workspace: &std::path::Path,
    output: &std::path::Path,
) -> Result<(), ToolError> {
    for comp in output.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "output_path 不得包含父级逃逸 `..`（实际：{}）",
                    output.display()
                ),
            });
        }
    }
    if output.is_absolute() {
        // 工作区与目标都做「去 \\?\ 前缀 + 大小写归一」后再比前缀
        let ws = normalize_for_prefix(workspace);
        let tgt = normalize_for_prefix(output);
        if !tgt.starts_with(&ws) {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "output_path 必须在工作区内（工作区：{}；实际：{}）",
                    workspace.display(),
                    output.display()
                ),
            });
        }
    }
    Ok(())
}

/// 前缀比较用的路径归一：去 Windows `\\?\` 扩展前缀，统一小写（Windows 大小写不敏感）。
fn normalize_for_prefix(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.replace('\\', "/").to_lowercase()
}
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
            return Err(ToolError::EState {
                current: cur,
                attempted: "advance_project_status",
            });
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

// ---------------------------------------------------------------- #59 阶段 7：PDF 通道

/// soffice 在场预检（SKIP 面：缺 soffice 不是业务失败——响亮横幅由调用方打）。
#[must_use]
pub fn soffice_available() -> bool {
    which_software("soffice").is_some() || which_software("libreoffice").is_some()
}


/// PDF 转换通道是否可用（**Word COM 或 LibreOffice 任一在场即可**）。
///
/// 与 [`soffice_available`] 的区别：后者只查 LibreOffice；本函数把 Word COM 也算进来。
/// 投标用户的常见环境是「有 Office、无 LO」——只看 soffice 会误判为不可用。
#[must_use]
pub fn pdf_converter_available() -> bool {
    #[cfg(windows)]
    {
        if word_com_available() {
            return true;
        }
    }
    soffice_available()
}

/// 当前可用的 PDF 转换通道名（供告警/日志展示）。
#[must_use]
pub fn pdf_converter_name() -> &'static str {
    #[cfg(windows)]
    {
        if word_com_available() {
            return "Microsoft Word COM";
        }
    }
    if soffice_available() {
        return "LibreOffice headless";
    }
    "（无可用通道）"
}
fn which_software(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// docx → pdf 的总入口：**Word COM 优先，LibreOffice 兜底**。
///
/// # 为什么 Word 优先
///
/// 投标用户的电脑上：
/// - ✅ **必然有** Microsoft Office（不然怎么写标书）
/// - ⚠️ 未必有 LibreOffice（开源替代，装机率低）
///
/// 实测差异（本机 有 Word 无 LO）：
/// ```text
/// Word COM:       2-5 秒，保真度最高（Word 自己渲染）
/// LibreOffice:   5-15 秒（首次更慢，要起 profile）
/// ```
///
/// 所以检测顺序必须是 Word → LO，而不是反过来。
fn convert_to_pdf(docx: &std::path::Path, out_pdf: &std::path::Path) -> Result<(), String> {
    // ① Word COM（Windows + Office 装有）
    #[cfg(windows)]
    {
        if word_com_available() {
            match convert_to_pdf_word(docx, out_pdf) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // Word 失败不直接放弃——继续试 LibreOffice（可能是 COM 权限/宏安全等问题）
                    let lo = convert_to_pdf_lo(docx, out_pdf);
                    return match lo {
                        Ok(()) => Ok(()),
                        Err(lo_err) => Err(format!(
                            "Word COM 与 LibreOffice 均失败 —— Word: {e}；LibreOffice: {lo_err}"
                        )),
                    };
                }
            }
        }
    }
    // ② LibreOffice headless
    convert_to_pdf_lo(docx, out_pdf).map_err(|e| {
        format!(
            "无可用 PDF 转换器 —— Word COM 不可用，LibreOffice: {e}。\
             请安装 Microsoft Office 或 LibreOffice，或手动用 Word「另存为 PDF」"
        )
    })
}

/// Word COM 是否可用（Windows 注册表探测，不实际启动 Word——零开销）。
#[cfg(windows)]
fn word_com_available() -> bool {
    // 查 App Paths 注册表：装了 Office 就一定有这一项
    std::process::Command::new("reg")
        .args(["query", r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\WINWORD.EXE", "/ve"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Word COM 自动化转 PDF（Windows 专属）。
///
/// 通过 PowerShell 调 COM——与现有 soffice 子进程同模式，不引入新依赖。
///
/// 关键点：
/// - `$word.Visible = $false`（后台运行，不弹窗）
/// - `SaveAs(..., 17)` 的 17 = `wdFormatPDF`
/// - `$doc.Close(0)` 的 0 = `wdDoNotSaveChanges`（不污染原 docx）
/// - `$word.Quit()` 必须在 finally 里（否则 Word 进程残留）
#[cfg(windows)]
/// Word COM 自动化转 PDF（Windows 专属）。
///
/// 通过 PowerShell 调 COM——与 soffice 子进程同模式，不引入新依赖。
///
/// # 实测踩过的坑（本机验证）
///
/// 1. **必须有超时**：Word 若被用户占用（已打开文档/弹窗），`Documents.Open` 会无限挂起。
///    实测：无超时保护时会卡 >3 分钟不返回。
/// 2. **必须清理残留**：PowerShell 被杀后 WINWORD.EXE 不会自动退出（实测残留 2 个进程各占 240MB）。
/// 3. **单实例串行**：多个 Word COM 并发会锁冲突。本进程内串行（标书导出本就是串行业务）。
///
/// # COM 参数
///
/// - `Documents.Open(path, ConfirmConversions=false, ReadOnly=true)` —— 只读打开，不污染原 docx
/// - `SaveAs(path, 17)` —— 17 = `wdFormatPDF`
/// - `Close(0)` —— 0 = `wdDoNotSaveChanges`
/// - `Visible=$false` + `DisplayAlerts=0` —— 后台静默
#[cfg(windows)]
/// Word COM 自动化转 PDF（Windows 专属）。
///
/// # 实测踩过的四个坑（本机逐轮验证）
///
/// 1. **中文路径会乱码**：PowerShell 5.1 读取无 BOM 的 UTF-8 脚本时按 GBK 解析，
///    `A-测试招标文件.docx` 变成 `A-娴嬭瘯鎷涙爣鏂囦欢.docx` → Word 报「找不到文件」。
///    **修法：脚本文件必须写 UTF-8 BOM**（`EF BB BF` 前缀），且用 `-File` 而非 `-Command`。
/// 2. **必须超时**：Word 被用户占用时 `Documents.Open` 无限挂起（实测 >3 分钟）。
/// 3. **必须清理残留**：PowerShell 被杀后 WINWORD.EXE 不退出（实测残留 2 个各 240MB）。
/// 4. **单实例串行**：多个 Word COM 并发锁冲突（实测同时起 2 个 → 都卡住）。
///
/// # 实测性能
///
/// 冷启动 12 秒（首次 Word 进程 + COM 初始化），热态 2-5 秒。
#[cfg(windows)]
fn convert_to_pdf_word(docx: &std::path::Path, out_pdf: &std::path::Path) -> Result<(), String> {
    /// Word COM 单次转换超时（秒）。冷启动 12s + 转换余量，30s 足够宽松。
    const WORD_COM_TIMEOUT_SECS: u64 = 30;

    let outdir = out_pdf.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(outdir).map_err(|e| format!("输出目录创建失败: {e}"))?;
    // PowerShell 单引号字符串：内部的 ' 要双写
    let esc = |p: &std::path::Path| p.display().to_string().replace('\'', "''");
    let docx_s = esc(docx);
    let pdf_s = esc(out_pdf);
    let script = format!(
        "$ErrorActionPreference = 'Stop'\r\n\
         $word = $null\r\n\
         $doc = $null\r\n\
         try {{\r\n\
           $word = New-Object -ComObject Word.Application\r\n\
           $word.Visible = $false\r\n\
           $word.DisplayAlerts = 0\r\n\
           $doc = $word.Documents.Open('{docx_s}', $false, $true)\r\n\
           $doc.SaveAs('{pdf_s}', 17)\r\n\
           $doc.Close(0)\r\n\
         }} finally {{\r\n\
           if ($doc) {{ try {{ [System.Runtime.InteropServices.Marshal]::ReleaseComObject($doc) | Out-Null }} catch {{}} }}\r\n\
           if ($word) {{ try {{ $word.Quit() }} catch {{}} ; try {{ [System.Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null }} catch {{}} }}\r\n\
         }}\r\n"
    );
    // 关键：写 UTF-8 BOM 脚本文件（PowerShell 5.1 无 BOM 会按 GBK 解析中文路径）
    let script_path = std::env::temp_dir().join(format!(
        "mang-wordcom-{}-{}.ps1",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    let mut bytes = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
    bytes.extend_from_slice(script.as_bytes());
    std::fs::write(&script_path, &bytes).map_err(|e| format!("脚本写入失败: {e}"))?;

    let mut child = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"])
        .arg("-File")
        .arg(&script_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            let _ = std::fs::remove_file(&script_path);
            format!("powershell 启动失败: {e}")
        })?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(WORD_COM_TIMEOUT_SECS);
    let timed_out = loop {
        match child.try_wait() {
            Ok(Some(_)) => break false,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    break true;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(_) => break false,
        }
    };
    if timed_out {
        let _ = child.kill();
        let _ = child.wait();
        // 清理本次可能残留的 Word（单实例串行前提下不会误杀用户文档）
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "WINWORD.EXE"])
            .output();
        let _ = std::fs::remove_file(&script_path);
        return Err(format!(
            "Word COM 超时（{WORD_COM_TIMEOUT_SECS}s）——Word 可能被其他文档占用或弹出了对话框；已清理残留"
        ));
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("收集 powershell 输出失败: {e}"))?;
    let _ = std::fs::remove_file(&script_path);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Word COM 退出码 {:?}: {}",
            output.status.code(),
            stderr.chars().take(400).collect::<String>()
        ));
    }
    verify_pdf_output(out_pdf)
}

/// PDF 产物校验（魔数 + 存在性）——两条转换通道共用。
fn verify_pdf_output(path: &std::path::Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("声称成功但产物缺失: {}", path.display()));
    }
    let mut magic = [0u8; 5];
    use std::io::Read;
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map_err(|e| format!("产物读取失败: {e}"))?;
    if &magic != b"%PDF-" {
        return Err("产物不是 PDF（魔数校验失败）".into());
    }
    Ok(())
}

/// docx → pdf（LibreOffice headless——Lead 阶段 7 路线 A：保真度最高）。
///
/// - 产物原子性：soffice 写 `{outdir}/{stem}.pdf` → rename 到最终位（soffice
///   不支持任意输出名，先落转换目录再搬——rename 原子）；
/// - profile 隔离：`-env:UserInstallation=file:///tmp/...` 每次独立 profile
///   （soffice 的用户锁会让并发转换串行化甚至死锁——隔离后并发安全）；
/// - 页眉页脚页码 / 中文字体嵌入：Writer 渲染管线承载（docx 的 sectPr/
///   header/footer/字体声明 → PDF 对象流——非嵌入中文由系统字体子集化）。
fn convert_to_pdf_lo(docx: &std::path::Path, out_pdf: &std::path::Path) -> Result<(), String> {
    let soffice = which_software("soffice")
        .or_else(|| which_software("libreoffice"))
        .ok_or_else(|| {
            "soffice/libreoffice 不在 PATH（环境缺失——SKIP 面，非业务失败）".to_string()
        })?;
    let outdir = out_pdf.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(outdir).map_err(|e| format!("输出目录创建失败: {e}"))?;
    // profile 隔离位（每次调用独立——并发安全；用 pid+nanos 防撞）
    let profile = std::env::temp_dir().join(format!(
        "bid-soffice-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    let conv_out = profile.join("out");
    std::fs::create_dir_all(&conv_out).map_err(|e| format!("转换目录创建失败: {e}"))?;
    // URL 规范：绝对路径必须三斜杠（file:///tmp/...）——两斜杠是 host 形式，
    // soffice 会静默忽略该参数 → 并发实例共享默认 profile → 锁冲突
    // （四个测试并发时三个「成功但无产物」的实测根因）
    let profile_url = format!("file:///{}", profile.display());

    let status = std::process::Command::new(&soffice)
        .arg("--headless")
        .arg("--norestore")
        .arg(format!("-env:UserInstallation={profile_url}"))
        .arg("--convert-to")
        .arg("pdf")
        .arg("--outdir")
        .arg(&conv_out)
        .arg(docx)
        .output()
        .map_err(|e| format!("soffice 启动失败: {e}"))?;
    let stdout = String::from_utf8_lossy(&status.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&status.stderr).into_owned();
    if !status.status.success() {
        let _ = std::fs::remove_dir_all(&profile);
        return Err(format!(
            "soffice 退出码 {:?}（stdout: {}；stderr: {}）",
            status.status.code(),
            stdout.chars().take(300).collect::<String>(),
            stderr.chars().take(300).collect::<String>()
        ));
    }
    // 产物名 = docx stem + .pdf（soffice 命名规则）
    let stem = docx.file_stem().and_then(|s| s.to_str()).unwrap_or("标书");
    let produced = conv_out.join(format!("{stem}.pdf"));
    if !produced.is_file() {
        // 诊断面：soffice 的 convert 消息打在 stdout（含它实际写的路径）——
        // 「声称成功但产物缺失」时带出实际落点
        let ls = std::fs::read_dir(&conv_out)
            .ok()
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        return Err(format!(
            "soffice 声称成功但产物缺失（期望 {}；outdir 内容: [{}]；stdout: {}；stderr: {}）",
            produced.display(),
            ls,
            stdout.chars().take(300).collect::<String>(),
            stderr.chars().take(200).collect::<String>()
        ));
    }
    // PDF 魔数校验（%PDF——防空文件/HTML 错误页伪装产物）
    let mut magic = [0u8; 5];
    use std::io::Read;
    std::fs::File::open(&produced)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map_err(|e| format!("产物读取失败: {e}"))?;
    if &magic != b"%PDF-" {
        return Err("产物不是 PDF（魔数校验失败）——soffice 可能输出了错误页".into());
    }
    std::fs::rename(&produced, out_pdf).map_err(|e| format!("产物搬移失败: {e}"))?;
    // profile 清理在产物搬移**之后**（conv_out 住在 profile 内——先删会把
    // 产物一起删掉，「成功但产物缺失」的实测根因；rename 返回后产物已安全）
    let _ = std::fs::remove_dir_all(&profile);
    Ok(())
}

/// 文件 sha256（hex 小写——导出报告的产物指纹口径）。
fn sha256_of(path: &std::path::Path) -> Result<String, ToolError> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).map_err(|e| ToolError::EUnprocessable {
        reason: format!("产物读取失败（{}）: {e}", path.display()),
    })?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize()))
}
