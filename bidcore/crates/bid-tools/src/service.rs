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

use bid_domain::{DomainStore, ParseStatus};
use bid_parse::{parse_file, sha256_file, ParsedFile};
use std::path::PathBuf;

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
        match parse_file(&spec.upload_ref, &spec.filename) {
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
pub struct BidService {
    store: DomainStore,
    parsed: Vec<ParsedFile>,
}

impl BidService {
    #[must_use]
    pub fn new() -> Self {
        Self { store: DomainStore::new(), parsed: vec![] }
    }

    pub fn store(&self) -> &DomainStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut DomainStore {
        &mut self.store
    }

    /// 已成功解析的文件面（`extract_requirements` 输入——M1-b 接线位）。
    pub fn parsed(&self) -> &[ParsedFile] {
        &self.parsed
    }

    /// `import_tender_files` 业务执行（壳已完成参数校验 / 状态机裁决 /
    /// 乐观并发检查——本层只跑管线）。
    pub fn import(&mut self, specs: &[ImportSpec]) -> ImportReport {
        import_files(&mut self.store, &mut self.parsed, specs)
    }
}

impl Default for BidService {
    fn default() -> Self {
        Self::new()
    }
}
