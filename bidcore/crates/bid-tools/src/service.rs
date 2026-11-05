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
//!   不落库（空 sha 会污染幂等键空间），逐件失败登记；
//! - 证据来源验证（#185 缺口 1）：外部面的证据登记走
//!   [`BidService::register_evidence_verified`]（source_file_id → 库内
//!   TenderFile → 解析产物对账 → 系统生成指纹），域层 `register_evidence`
//!   保持纯逻辑签名不动（域测试伪造 spec 合法）。

use bid_domain::evidence::{EvidenceFields, EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::principal::{self, TrustedPrincipal};
use bid_domain::review::Gate;
use bid_domain::{DomainStore, ParseStatus, Requirement, ToolError};
use bid_parse::{parse_file, sha256_file, BlockKind, ParsedFile};
use std::path::{Path, PathBuf};

/// 单文件导入规格（MCP 壳已完成契约校验后传入）。
#[derive(Debug, Clone)]
pub struct ImportSpec {
    /// 上传工件引用。AdapterA 本实现 = 文件系统路径。**#183 围栏口径**：
    /// 壳层/服务层会把本字段过 [`crate::fs_fence::resolve_read`]（工作区
    /// 相对或围栏内绝对路径，越界/`..`/symlink 段 → E_UNPROCESSABLE）并
    /// 替换为围栏内 canonical 绝对路径；未来对象存储句柄在同位替换，
    /// 管线不变。
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

/// 证据登记规格（**来源验证面**，#185 缺口 1）——MCP / 外部调用方的入参。
///
/// 与域层 [`bid_domain::evidence::EvidenceSpec`] 的差异：来源三件套
/// （`source_file` / `source_sha256` / `fingerprint`）**不在入参里**——由
/// [`BidService::register_evidence_verified`] 从 TenderFile 库内记录与解析
/// 产物生成（调用方自报的来源身份不可信：可以登记「不存在的合同.pdf」+
/// 任意 hex sha 骗过格式校验成为「可信证据」——不是密码学问题，是来源
/// 身份没有验证）。
///
/// `locator: None` = 文件级锚（不携带引擎定位器——指纹退 `sha256:<库内 sha>`，
/// 诚实标记登记面，不冒充引擎抽取）；`Some(loc)` = 必须与该文件的解析产物
/// 对账（见 [`verify_locator_against`]）。
#[derive(Debug, Clone)]
pub struct EvidenceRegistration {
    /// 来源文件（TF-xxxx——必须是已导入的 TenderFile；不存在/解析失败/无
    /// 解析产物一律拒，见 [`BidService::register_evidence_verified`]）。
    pub source_file_id: String,
    pub kind: EvidenceKind,
    pub title: String,
    /// 引擎定位器（`None` = 文件级锚）；`Some` 时须与解析产物对账。
    pub locator: Option<EngineLocator>,
    /// #245 占位声明：`true` = 本次登记为占位/待核验材料（tier=PLACEHOLDER）。
    /// 调用方**可以自报占位**（诚实方向——从低不从高），但不得自报任何
    /// 已验证态（verification_tier 不在参数面，升级只能经换版重登）。
    /// 占位证据：可绑定（待补可见）、可进起草（生成【待补】文案），
    /// 但不计为有效证据、不清合规阻塞、导出门独立复算拒。
    pub placeholder: bool,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub subject_name: String,
    pub scope: String,
    // ---- #117 任务 1：业绩业务面（None = 未录入；非业绩类证据不消费）
    pub client: Option<String>,
    pub amount_wan: Option<f64>,
    pub completed_on: Option<String>,
    // ---- #147：结构化字段（search_evidence exact 通道的精确查询面）
    pub fields: EvidenceFields,
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

/// 定位内容（#185 缺口 1 对账 + #217 缺口 2 抽取一体）。
///
/// 验证基准 = 本仓解析产物（`ParsedFile.blocks`——服务态唯一的内容真源）：
/// - `DocxParagraph { paragraph_index, text_fingerprint }`：`paragraph_index`
///   是**正文段块** 0 起序（仅计 `BlockKind::Para`，表格行不占号——与域层
///   「仅计段落块」口径一致）；越界 → 拒；指纹与实际段落文本不符 → 拒
///   （证明调用方确实看过该内容，而非首址乱填）；
/// - `PdfPageText { page, text_fingerprint }`：页号 1 起须落在解析页数内；
///   页文本 = 该页全部行块拼接（引擎路径 `第N页 行M` / OCR 路径
///   `第N页 OCR 行M` 同前缀收敛）；指纹同口径比对；
/// - 其余形态（xlsx_cell / xlsx_region / docx_table_cell / markdown_block /
///   canvas_object）在标书解析产物无对应对账面 → **明确报错**（fail-closed，
///   不静默跳过——静默跳过 = 回到「未验证来源」的老润洞）。
///
/// 返回定位结果：系统从实际内容生成的指纹（`fnv64:<hex>`——定位文本归一化后
/// FNV-1a 64，与既有 text_fingerprint 同族）+ **定位文本本身**（#217 摘录
/// 入账的素材——从解析产物逐字截取，非文件名/标题转写）+ 人话定位
/// （excerpt_locator——生成器/导出的可读引用面）。
struct LocatedContent {
    fingerprint: String,
    text: String,
    locator_display: String,
}

fn locate_content(parsed: &ParsedFile, loc: &EngineLocator) -> Result<LocatedContent, ToolError> {
    let un = |reason: String| ToolError::EUnprocessable { reason };
    match loc {
        EngineLocator::DocxParagraph {
            paragraph_index,
            text_fingerprint,
        } => {
            let paras: Vec<&str> = parsed
                .blocks
                .iter()
                .filter(|b| b.kind == BlockKind::Para)
                .map(|b| b.text.as_str())
                .collect();
            let idx = *paragraph_index as usize;
            let Some(text) = paras.get(idx) else {
                return Err(un(format!(
                    "locator 段落越界：paragraph_index={idx}，文件「{}」正文段共 {} 个（0 起）",
                    parsed.name,
                    paras.len()
                )));
            };
            let actual = content_fingerprint(text);
            if *text_fingerprint != actual {
                return Err(un(format!(
                    "locator 指纹不匹配：paragraph_index={idx} 实际内容指纹 {actual}，登记值 {text_fingerprint}（内容已变更或未按实际内容登记）"
                )));
            }
            Ok(LocatedContent {
                fingerprint: actual,
                text: (*text).to_string(),
                locator_display: format!("《{}》正文段 {}（0 起）", parsed.name, idx),
            })
        }
        EngineLocator::PdfPageText {
            page,
            text_fingerprint,
        } => {
            if *page == 0 || *page as usize > parsed.pages {
                return Err(un(format!(
                    "locator 页号越界：page={page}，文件「{}」共 {} 页（1 起）",
                    parsed.name, parsed.pages
                )));
            }
            // locator 前缀**两条解析路不同**（只认一种会导致：「有 poppler 的环境绿、
            // 无 poppler 的环境全红」——CI 无 poppler 实测）：
            //   · poppler 首选路：`第1页 行1`（页后无空格）
            //   · 纯 Rust 兜底路：`第1 页 > 第1 行`（页后空格在「页」前）
            let prefixes = [format!("第{page}页"), format!("第{page} 页")];
            let page_text = parsed
                .blocks
                .iter()
                .filter(|b| prefixes.iter().any(|p| b.locator.starts_with(p.as_str())))
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let actual = content_fingerprint(&page_text);
            if *text_fingerprint != actual {
                return Err(un(format!(
                    "locator 指纹不匹配：第 {page} 页实际内容指纹 {actual}，登记值 {text_fingerprint}（内容已变更或未按实际内容登记）"
                )));
            }
            Ok(LocatedContent {
                fingerprint: actual,
                text: page_text,
                locator_display: format!("《{}》第 {page} 页", parsed.name),
            })
        }
        other => Err(un(format!(
            "locator 形态 {} 暂不支持来源验证（当前支持 docx_paragraph / pdf_page_text——按解析产物对账；不带 locator 登记则为文件级锚）",
            locator_kind_name(other)
        ))),
    }
}

/// locator 变体名（错误文案面——serde tag 同源）。
fn locator_kind_name(loc: &EngineLocator) -> &'static str {
    match loc {
        EngineLocator::XlsxCell { .. } => "xlsx_cell",
        EngineLocator::XlsxRegion { .. } => "xlsx_region",
        EngineLocator::DocxParagraph { .. } => "docx_paragraph",
        EngineLocator::DocxTableCell { .. } => "docx_table_cell",
        EngineLocator::MarkdownBlock { .. } => "markdown_block",
        EngineLocator::PdfPageText { .. } => "pdf_page_text",
        EngineLocator::CanvasObject { .. } => "canvas_object",
    }
}

/// 定位文本 → 内容指纹（`fnv64:<hex>`——归一化去空白后 FNV-1a 64，
/// [`bid_domain::linkage::text_fingerprint`] 单一真源，与既有
/// text_fingerprint 同族；算法前缀纪律见 `fingerprint_algorithm`）。
fn content_fingerprint(text: &str) -> String {
    format!("fnv64:{}", bid_domain::linkage::text_fingerprint(text))
}

/// 摘录入账长度上界（#217）：定位文本可能整页上千字，台账/prompt 面截到
/// 500 字——金额/日期/范围等实料通常在前部；截断以「……」标记（诚实面：
/// 可辨识这不是全文，全文回查走 locator）。
const EXCERPT_MAX_CHARS: usize = 500;

fn truncate_excerpt(text: &str) -> String {
    if text.chars().count() <= EXCERPT_MAX_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(EXCERPT_MAX_CHARS).collect();
    format!("{cut}……")
}

/// 导入管线（契约 §2 业务半边）——**受信内部 API**（#183：无围栏）。
///
/// 调用方：[`BidService::import`]（服务层，带围栏复验+限额——MCP 壳的正道）
/// 与 runner.rs 场景运行器（files_dir 受信内部目录）。外部输入必须先过
/// 围栏（见 [`crate::fs_fence`]），不要直接把 MCP 调用者的路径喂进本函数。
///
/// 逐件：sha256 幂等（已存在 → no-op 返回原 file_id，`deduplicated:true`；
/// #188：同内容**异身份**（kind/文件名不同）导入不吞角色——登记
/// `FileRole` + metadata_conflict 告警，完全相同的重放保持纯 no-op）→
/// `parse_file` → `upsert_file` 落库；成功件的解析产物累积进 `parsed`
/// （M1-b `extract_requirements` 的输入面）。单件失败 → `Failed` 显式登记 +
/// 继续（R-01），绝不整批中止。
pub fn import_files(
    store: &mut DomainStore,
    parsed: &mut Vec<ParsedFile>,
    specs: &[ImportSpec],
    convert_dir: Option<&std::path::Path>,
) -> ImportReport {
    // #270：convert_dir = .doc→docx 转换产物目录（有工作区的调用方传
    // `<ws>/.mang/convert/`；无工作区形态（runner 批处理）传 None → 服务层
    // 降级系统 temp——两种位置都不写用户目录。
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
        // 幂等：sha256 已存在 → no-op 返回原 file_id（契约 §2）。
        // #188：去重不吞业务角色——同 SHA 异身份（kind/文件名不同）导入
        // 登记 FileRole（追加不覆盖）+ metadata_conflict 告警（告警不失败，
        // 验收 C）；完全相同的重放（同 SHA 同 kind 同名）纯 no-op 无告警
        //（验收 B）；补遗/正本判定从此看任一角色（readiness.rs，验收 A）。
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
            // #188 角色对账：本次 (kind, filename) 是否已在已知身份里。
            // 初始身份 = 首次导入的 TenderFile.kind/name（upsert_file 落档）；
            // 后续身份 = FileRole 登记。命中任一 → 幂等 no-op。
            let identity_known = (f.kind == spec.kind && f.name == spec.filename)
                || store.file_roles(&f.id).iter().any(|r| {
                    r.kind == spec.kind && r.declared_name == spec.filename
                });
            let mut warnings = Vec::new();
            if !identity_known {
                match store.register_file_role(&f.id, &spec.kind, &spec.filename, now_secs()) {
                    Ok(_) => warnings.push(format!(
                        "METADATA_CONFLICT: 同 SHA 文件以不同身份再次导入（原 {}「{}」，本次 {}「{}」）——已登记新业务角色（追加不覆盖），补遗/正本判定按任一角色",
                        f.kind, f.name, spec.kind, spec.filename
                    )),
                    Err(e) => errors.push(format!("业务角色登记失败: {e:?}")),
                }
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
                warnings,
            });
            continue;
        }
        // B4 收敛（issue #11 任务 3）：docx 与 xlsx 走 bid-engine 引擎路径
        // （自研 docx/xlsx 解析已删——引擎是 OOXML 读面单一真源；B3-3 r3 差分
        // 验收 EQ+ED 91.57%，A 件等抽样 100% EQ）；pdf 引擎优先（pdfium 文本层）
        // + 旧路径回退（pdftotext/OCR 补遗链——`parse_file` B4 后仅剩 pdf 面，
        // 见 bid-parse/src/lib.rs 模块头）；.doc（#270）→ 环境转换通道（Word COM
        // / LibreOffice 任一：有则自动转 docx 解析+标注，无则引导报错——两层
        // 设计详见文末 #270 区块）；.xls/.ods 等其它扩展名 → `parse_file` 显式
        // 拒绝（「不支持的文件类型」——zip 容器载具不误解析）。
        // 已知引擎缺口登记不伪装：域剥离边界（mdcore#40）——详见
        // docs/b3-3-r3-acceptance-report.md（嵌套表 mdcore#39 已修复）。
        let ext = file_extension(&spec.upload_ref, &spec.filename);
        let mut converted_note: Option<&'static str> = None;
        let parsed_result = match ext.as_str() {
            "docx" => {
                // #270 顺带修复：.doc 直接改名 .docx（OLE2 容器）原报
                // 「container corrupt: Could not find EOCD」误导用户以为文件
                // 损坏——路由面前置魔数识别 + 旧格式引导（PK 头异常仍走引擎）
                if is_ole_container(&spec.upload_ref) {
                    Err(docx_lie_message())
                } else {
                    bid_engine::parse_docx_via_engine(&spec.upload_ref).map_err(|e| e.to_string())
                }
            }
            "xlsx" => {
                bid_engine::parse_xlsx_via_engine(&spec.upload_ref).map_err(|e| e.to_string())
            }
            // B3-5：pdf 引擎通道（pdfium 文本层）——扫描件（MappingGap）或
            // 无 pdfium（EngineFailed）回退旧 pdftotext/OCR 路径（映射表 §3）
            "pdf" => bid_engine::parse_pdf_via_engine(&spec.upload_ref)
                .or_else(|_| parse_file(&spec.upload_ref, &spec.filename)),
            // #270：.doc（Word 97-2003）——两层设计：有转换器自动转换（sha 记
            // 原件 + 响应面标注），无转换器引导报错（Skill 侧接管）
            "doc" => {
                converted_note = Some("CONVERTED_FROM_DOC");
                import_doc_converted(&spec.upload_ref, &spec.filename, &sha, convert_dir)
            }
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
                // #270：.doc 经转换解析的响应面标注（per_file.warnings 透出——
                // 与 parse_summary 落库标注双通道，Agent/用户任一面可见）
                if let Some(note) = converted_note {
                    debug_assert_eq!(note, "CONVERTED_FROM_DOC");
                    warnings.push(format!(
                        "CONVERTED_FROM_DOC: 本件经 .doc→.docx 转换后解析（转换器：{}；sha256 记原件，转换产物在工作区内部不外泄）",
                        doc_converter_name()
                    ));
                }
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
    /// #182 人工审核 receipt 的 HMAC 签发/验签密钥（32 字节）。
    /// 持久化工作区（open）：`<dir>/.mang/approval.key`（存在读/不存在生成
    /// 并写，权限 0600）；纯内存（new）：内存生成不落盘。**不进 MCP 工具
    /// 面**——宿主持钥，Agent 拿不到（这正是 #182 的信任根）。
    approval_key: [u8; 32],
    /// 文件访问围栏根（#183/#189：import upload_ref / export output_path 的
    /// 受信边界）。`open(dir)` 默认 = 工作区目录；`with_fs_root` 覆盖
    /// （BID_FS_ROOT / 测试夹具声明更广的受信根）；`new()` = None →
    /// 导入响亮拒（见 [`Self::import`]）。持久化目录始终隐式受信（服务的
    /// 自有存储——章节/导出/manifest 都落在那里，与 fs_root 是否覆盖无关）。
    fs_root: Option<PathBuf>,
}

/// #182：32 字节随机密钥（getrandom——OS 熵源，不手拼）。
fn random_approval_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key)
        .expect("approval_key 生成需要 OS 熵源（getrandom 失败 = 宿主环境不可用，fail-fast）");
    key
}

/// #182：加载（或首启生成并落盘）持久化审批密钥。
///
/// - 存在且恰 32 字节 → 读入（重启续跑：旧 receipt 在有效窗内仍可验）；
/// - 不存在 → 生成 + 写入 + unix 下 chmod 0600（密钥文件不给同组/其他读）；
/// - 存在但长度 ≠ 32 → Corrupt（fail-closed 不猜测——半截密钥验不出任何
///   旧 receipt，静默重生 = 无声作废全部在途确认）。
fn load_or_create_approval_key(
    dir: &Path,
) -> Result<[u8; 32], crate::store_io::StoreIoError> {
    let mang = dir.join(".mang");
    std::fs::create_dir_all(&mang).map_err(crate::store_io::StoreIoError::Io)?;
    let path = mang.join("approval.key");
    match std::fs::read(&path) {
        Ok(bytes) => {
            let n = bytes.len();
            bytes.try_into().map_err(|_| {
                crate::store_io::StoreIoError::Corrupt(format!(
                    "approval.key 须恰 32 字节（实际 {n}）——半截密钥验不出任何旧 receipt，拒绝猜测"
                ))
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = random_approval_key();
            std::fs::write(&path, key).map_err(crate::store_io::StoreIoError::Io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o600);
                std::fs::set_permissions(&path, perms)
                    .map_err(crate::store_io::StoreIoError::Io)?;
            }
            Ok(key)
        }
        Err(e) => Err(crate::store_io::StoreIoError::Io(e)),
    }
}

/// #182：过审 receipt 的签域（scope）构造——过审面收到的 receipt 只对
/// 签发时声明的具体门有效（A 门 receipt 过不了 B 门）。
pub fn review_gate_scope(gate: Gate) -> String {
    format!("review_gate:{}", gate.as_str())
}

/// #182：补遗逐件确认 receipt 的签域——绑定具体 file_id（TF-0003 的
/// receipt 免不了 TF-0004）。
pub fn addendum_waiver_scope(file_id: &str) -> String {
    format!("addendum_waiver:{file_id}")
}

/// #226/#182：矩阵覆写逐行确认 receipt 的签域——四重绑定（Lead 合并门
/// 2026-11-04 口径："bound to the project and exact override scope
/// (requirement/status/reason)"）。
///
/// `matrix_override:{project_id}:{requirement_id}:{STATUS}:{sha256(deviation)}`
///
/// - **project**：跨项目不可挪用——同一工作区先后建多个项目时 REQ 编号
///   会撞号，没有项目段的签会到处通用；
/// - **requirement**：行级绑定（REQ-0003 的 receipt 免不了 REQ-0004）；
/// - **status**：人批的是「这一条标不适用」，不是「这一条随便改」——
///   换状态必须换签；
/// - **reason**：人批的是这条偏离说明本身（sha256 摘要绑全文）——偷换
///   理由 = 偷换人批的那件事，验签不过。
///
/// deviation 缺省（非 NOT_APPLICABLE 场景）绑空串摘要——四段恒齐全。
pub fn matrix_override_scope(
    project_id: &str,
    requirement_id: &str,
    status: &str,
    deviation: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let reason_digest = hex::encode(Sha256::digest(deviation.as_bytes()));
    format!("matrix_override:{project_id}:{requirement_id}:{status}:{reason_digest}")
}

impl BidService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: DomainStore::new(),
            parsed: vec![],
            persist: None,
            // #182：纯内存态——内存生成密钥不落盘（测试/嵌入方默认形态）
            approval_key: random_approval_key(),
            fs_root: None,
        }
    }

    /// 打开（或首启创建）持久化服务态：目录有 `db.json` 则恢复（重启续跑——
    /// T-8 验收「建项→导入→抽条款→重启→状态完整恢复」的入口），否则空态起步。
    /// #182：审批密钥从 `<dir>/.mang/approval.key` 装载（缺则生成落盘 0600
    /// ——重启后旧 receipt 在 24h 有效窗内仍可验，跨进程身份连续）。
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, crate::store_io::StoreIoError> {
        let sf = crate::store_io::StoreFile::new(&dir);
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
        let approval_key = load_or_create_approval_key(sf.dir())?;
        Ok(Self {
            store,
            parsed,
            // #183/#189：默认围栏根 = 工作区（导入导出都锁在工作区内；
            // BID_FS_ROOT / with_fs_root 可显式声明更广的受信根）
            fs_root: Some(dir.as_ref().to_path_buf()),
            persist: Some(sf),
            approval_key,
        })
    }

    /// 覆盖文件访问围栏根（#183/#189）。builder 形态：`open(ws).with_fs_root(x)`
    /// / `new().with_fs_root(x)`。根不存在时保留原样（围栏将拒绝一切路径并
    /// 响亮报错，不静默降级）。持久化目录（若在）始终隐式受信——测试夹具
    /// 用本方法声明仓库夹具根后，导出写工作区仍合法。
    #[must_use]
    pub fn with_fs_root(mut self, root: impl AsRef<Path>) -> Self {
        let r = root.as_ref().to_path_buf();
        self.fs_root = Some(std::fs::canonicalize(&r).unwrap_or(r));
        self
    }

    /// 围栏根清单（canonical；去重）：fs_root 在场用 fs_root，否则退回持久化
    /// 目录；持久化目录始终追加（服务自有存储隐式受信）。空清单 = 无围栏
    /// 可言（纯内存态未声明根）→ 导入响亮拒。
    #[must_use]
    pub fn fs_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        let primary = self
            .fs_root
            .clone()
            .or_else(|| self.persist.as_ref().map(|sf| sf.dir().to_path_buf()));
        if let Some(p) = primary {
            let c = std::fs::canonicalize(&p).unwrap_or(p);
            roots.push(c);
        }
        if let Some(sf) = &self.persist {
            let c = std::fs::canonicalize(sf.dir()).unwrap_or_else(|_| sf.dir().to_path_buf());
            if !roots.iter().any(|r| crate::fs_fence::is_inside(&c, r) && crate::fs_fence::is_inside(r, &c)) {
                roots.push(c);
            }
        }
        roots
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

    /// `set_matrix_override` 业务半边（#226 层 1：否决项死锁的人工出口
    /// + #182 receipt 验签）。
    ///
    /// **域层 `DomainStore::set_matrix_override` 只写 overrides 表，不重算矩阵**
    /// ——所以单独调它之后 `export_blockers()` 仍看旧行，死锁根本没解。
    /// 本方法把「验签 → 写覆写 → 重建矩阵 → 落盘」串成一次语义动作，
    /// 调用方看到的 `matrix` 与 `export_blockers()` 才是同一个真源。
    ///
    /// #182：入参带 `receipt`（宿主签发的审批回执，签域四重绑定
    /// `matrix_override:{project_id}:{requirement_id}:{status}:{理由摘要}`，
    /// 见 [`matrix_override_scope`]）。服务层验签是 MCP 面的唯一入口
    /// （单一真源）；验签得 TrustedPrincipal → 域层再拦 actor_type（纵深）。
    /// Agent 不得伪造 receipt 自免单——挪用到别行、换状态、偷换理由都过不了签。
    ///
    /// 前置：矩阵必须已构建（`build_matrix` 过）。未构建时先建再覆写没有
    /// 意义——覆写的对象就是矩阵行。
    /// 返回 (矩阵行数, blocking 行数, 可信身份)——供工具面直接回显。
    pub fn set_matrix_override(
        &mut self,
        requirement_id: String,
        status: bid_domain::matrix::ResponseStatus,
        deviation: Option<String>,
        receipt: &str,
    ) -> Result<(usize, usize, TrustedPrincipal), ToolError> {
        // 签域的项目段取服务态真源（在管项目），不信调用方自报的 id——
        // 签域与实际生效对象恒一致。
        let project_id = self.store.project()?.id.clone();
        let scope = matrix_override_scope(
            &project_id,
            &requirement_id,
            status.as_str(),
            deviation.as_deref().unwrap_or(""),
        );
        let principal = self.verify_approval_receipt(receipt, &scope)?;
        self.store
            .set_matrix_override(requirement_id, status, deviation, &principal)?;
        let (rows, blocking) = self.store.rebuild_matrix_for_override()?;
        self.auto_checkpoint()?;
        Ok((rows, blocking, principal))
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

    /// #219 二步：`submit_block_marks` 落库半边（marks + submitted 原子写）。
    ///
    /// 修 PR #227 的两个缺口：① submitted 关键点回显后丢弃不落库（合并面
    /// 元输入缺失）；② 直改 store 不经服务层——持久化工作区下标记不落盘，
    /// 直到下一次经包装的写操作（半个库状态风险）。
    pub fn set_block_coverage(
        &mut self,
        file_id: &str,
        marks: Vec<bid_domain::coverage::BlockMark>,
        reqs: Vec<bid_domain::coverage::SubmittedRequirement>,
        total_blocks: usize,
    ) -> Result<(), ToolError> {
        self.store
            .set_block_coverage(file_id, marks, reqs, total_blocks)?;
        self.auto_checkpoint()
    }

    /// #219 二步：抽取通路合并后的整集替换落库（语义见域层
    /// `replace_requirements`——Agent 优先要求被覆盖块上的旧正则 REQ
    /// 能从库里消失，只增不删的 set 语义做不到）。
    pub fn replace_requirements(&mut self, reqs: Vec<Requirement>) -> Result<usize, ToolError> {
        let r = self.store.replace_requirements(reqs)?;
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

    /// `register_evidence` 的**来源验证**登记入口（#185 缺口 1）。
    ///
    /// 与 [`Self::register_evidence`]（域层直通面）的分野：
    /// - 域层保持纯逻辑（`EvidenceSpec` 签名不动——域层不知道 TenderFile 表，
    ///   域测试用伪造 spec 测查重/换版/翻转是合法路径）；
    /// - 本入口面向 MCP / 外部调用方：来源身份**服务端验证**，不信调用方自报——
    ///   1. 查 TenderFile 记录（`source_file_id` 不存在 → E_NOTFOUND）；
    ///   2. 解析失败件禁引（parse_status=Failed → E_UNPROCESSABLE，R-01）；
    ///   3. 解析产物必须在场（sha256 对账——否则该件未经 import 管线）；
    ///   4. locator 按解析产物对账（段落越界 / 页号越界 / 指纹不匹配 → 拒；
    ///      不支持的 locator 形态 → 明确报错，不静默跳过）；
    ///   5. `source_file` / `source_sha256` / `fingerprint` 全部取库内与实际
    ///      内容（调用方无从传入——「库内 sha 优先于调用方」由构造保证）。
    ///
    /// 指纹口径（与既有 text_fingerprint 同族）：
    /// - 带 locator → `fnv64:<hex>`（定位文本归一化后 FNV-1a 64，
    ///   [`bid_domain::linkage::text_fingerprint`] 单一真源）；
    /// - 无 locator（文件级锚）→ `sha256:<库内文件 sha>`（内容指纹 = 文件身份，
    ///   与旧缺省行为同语义，但真源是库而非调用方）。
    pub fn register_evidence_verified(
        &mut self,
        reg: EvidenceRegistration,
    ) -> Result<(String, bool), ToolError> {
        // ---- 1. TenderFile 在场性（不存在的文件 = 无来源身份，拒）
        let tf = self
            .store
            .files()
            .iter()
            .find(|f| f.id == reg.source_file_id)
            .cloned()
            .ok_or_else(|| ToolError::ENotFound {
                what: format!(
                    "source_file_id {} 不存在（证据来源必须是已导入文件——先 import_tender_files，以回执 per_file[].file_id 引用）",
                    reg.source_file_id
                ),
            })?;
        // ---- 2. 解析失败件禁引（R-01：失败件内容全程禁引——证据也不例外）
        if tf.parse_status == ParseStatus::Failed {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "source_file_id {} 指向解析失败件（{}）——失败件内容禁引；修复后以新内容重传再登记",
                    tf.id, tf.parse_summary
                ),
            });
        }
        // ---- 3. 解析产物对账（按库内 sha 加入——调用方无从伪造join键）
        let parsed = self
            .parsed
            .iter()
            .find(|p| p.sha256 == tf.sha256)
            .ok_or_else(|| ToolError::EUnprocessable {
                reason: format!(
                    "source_file_id {} 对应文件无解析产物在服务态（须先经 import_tender_files 成功导入）",
                    tf.id
                ),
            })?;
        // ---- 4/5. locator 对账 + 系统生成指纹 + 摘录捕获（#217 四态裁决
        //      +#245 占位态）：
        //      文件级锚（无 locator）→ FILE_EXISTS：来源在库且非失败件，但正文
        //      未定位——摘录面为空（不编造「读过了」的假象）；带 locator 且对账
        //      通过 → CONTENT_MATCHES：定位文本逐字入账为 excerpt。
        //      **placeholder=true（#245）→ 一律 PLACEHOLDER**：占位/待核验材料
        //      的内容未经核验——即便 locator 对账通过（如指向招标文件里的
        //      「资质要求」文本，那是要什么、不是有什么），也不得计为已验证
        //      证据。摘录照记（回溯占位登记指向了什么），升级路径 = 换版重登。
        let located: Option<LocatedContent> = match &reg.locator {
            None => None,
            Some(loc) => Some(locate_content(parsed, loc)?),
        };
        let locator = match (&reg.locator, &located) {
            (None, _) => EngineLocator::MarkdownBlock {
                block_id: format!("mcp-registered:{}", tf.name),
                text_fingerprint: format!("sha256:{}", tf.sha256),
            },
            (Some(loc), _) => loc.clone(),
        };
        let (fingerprint, excerpt, excerpt_locator, verification_tier) = if reg.placeholder {
            // #245 占位登记：指纹仍服务端生成（#185 纪律不变），摘录携带时
            // 记录（占位指向回溯面）；tier 强制 PLACEHOLDER——不因 locator
            // 对账通过而升格（「文件里有这段文本」≠「资质真实具备」）。
            (
                located.as_ref().map(|lc| lc.fingerprint.clone())
                    .unwrap_or_else(|| format!("sha256:{}", tf.sha256)),
                located.as_ref().map(|lc| Some(truncate_excerpt(&lc.text))).unwrap_or(None),
                located.as_ref().map(|lc| Some(lc.locator_display.clone())).unwrap_or(None),
                Some(bid_domain::evidence::VerificationTier::Placeholder),
            )
        } else {
            match &located {
                Some(lc) => (
                    lc.fingerprint.clone(),
                    Some(truncate_excerpt(&lc.text)),
                    Some(lc.locator_display.clone()),
                    Some(bid_domain::evidence::VerificationTier::ContentMatches),
                ),
                None => (
                    format!("sha256:{}", tf.sha256),
                    None,
                    None,
                    Some(bid_domain::evidence::VerificationTier::FileExists),
                ),
            }
        };
        let spec = EvidenceSpec {
            kind: reg.kind,
            title: reg.title,
            source_file: tf.name,
            source_sha256: tf.sha256,
            locator,
            fingerprint,
            valid_from: reg.valid_from,
            valid_until: reg.valid_until,
            subject_name: reg.subject_name,
            scope: reg.scope,
            client: reg.client,
            amount_wan: reg.amount_wan,
            completed_on: reg.completed_on,
            fields: reg.fields,
            excerpt,
            excerpt_locator,
            verification_tier,
        };
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

    /// `record_review_gate` 业务半边（G3 + #182）：三门记审。
    ///
    /// #182：入参从裸 `reviewer` 串改为 `receipt`（宿主签发的审批回执）。
    /// **服务层验签是 MCP 面的唯一入口**（单一真源——壳层不重复实现）：
    /// 签域 `review_gate:{gate}`（门级绑定——A 门 receipt 过不了 B 门）→
    /// 验签得 TrustedPrincipal → 域层再拦 actor_type（纵深）。状态门/门序
    /// 裁决在域层 record_review，不变。
    pub fn record_review_gate(
        &mut self,
        gate: Gate,
        receipt: &str,
        decision: bid_domain::review::Decision,
        findings: Vec<(bid_domain::review::Severity, String, String, Option<String>)>,
    ) -> Result<(bid_domain::review::ReviewReceipt, TrustedPrincipal), ToolError> {
        let principal = self.verify_approval_receipt(receipt, &review_gate_scope(gate))?;
        let receipt = self
            .store
            .record_review(gate, &principal, decision, findings)?;
        self.auto_checkpoint()?;
        Ok((receipt, principal))
    }

    /// `record_addendum_waiver` 业务半边（#184 + #182）：补遗逐件人工确认。
    ///
    /// #182：入参从裸 `principal` 串改为 `receipt`。签域
    /// `addendum_waiver:{file_id}`（件级绑定——TF-0003 的 receipt 免不了
    /// TF-0004）；reason 非空白 / file_id 存在性在域层裁决，不变。
    pub fn record_addendum_waiver(
        &mut self,
        file_id: &str,
        reason: &str,
        receipt: &str,
    ) -> Result<(bid_domain::AddendumWaiverRecord, TrustedPrincipal), ToolError> {
        let principal = self.verify_approval_receipt(receipt, &addendum_waiver_scope(file_id))?;
        let rec = self
            .store
            .record_addendum_waiver(file_id, reason, &principal)?;
        self.auto_checkpoint()?;
        Ok((rec, principal))
    }

    // ------------------------------------------------------------ #182 审批 receipt 面
    /// 宿主签发面（#182——**不进 MCP 工具表，Agent 拿不到**）：真人审核员
    /// 在宿主进程确认后，宿主用服务态密钥签发审批回执。Actor 恒为 Human
    /// （宿主只代真人签发；机器操作面另行留痕，不走过审面）。
    ///
    /// scope 由调用方声明具体用途（`review_gate:{gate}` /
    /// `addendum_waiver:{file_id}`——见 [`review_gate_scope`] /
    /// [`addendum_waiver_scope`]）。
    pub fn issue_approval_receipt(&mut self, actor_id: &str, scope: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        principal::issue_approval_receipt(&self.approval_key, actor_id, scope, now)
    }

    /// 验签面（#182）：receipt → TrustedPrincipal（now 取系统时钟）。
    /// ReceiptError → EUnprocessable 带分类码（E_UNPROCESSABLE 是参数面
    /// 的“不可用/不可恢复”口径——伪造 receipt 是调用方问题不是服务态问题）。
    pub fn verify_approval_receipt(
        &self,
        receipt: &str,
        scope: &str,
    ) -> Result<TrustedPrincipal, ToolError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        principal::verify_approval_receipt(receipt, &self.approval_key, scope, now).map_err(
            |e| ToolError::EUnprocessable {
                reason: format!(
                    "审批 receipt 验签失败（{}——{}）。人工确认由宿主签发 receipt 提供，Agent 不得伪造",
                    e.code(),
                    e
                ),
            },
        )
    }

    /// 审批密钥的宿主侧只读面（#182——审计/测试用：跨 crate 集成测试构造
    /// 「Agent 伪装」「过期重放」等攻击样本需要持钥签发）。**不进 MCP
    /// 工具表**——与 issue_approval_receipt 同一信任边界（宿主进程内）。
    pub fn approval_key(&self) -> &[u8; 32] {
        &self.approval_key
    }

    /// `import_tender_files` 业务执行（壳已完成参数校验 / 状态机裁决 /
    /// 乐观并发检查——本层只跑管线；T-8：持久化在场时写后自动落盘）。
    ///
    /// #183 围栏（双保险的服务层半边）：壳层已逐件 resolve_read 过一次，
    /// 这里持根复验（壳与服务层任一被绕过都不放行）+ 施加大小/件数/累计
    /// 限额（超限 E_UNPROCESSABLE 逐件报因）。upload_ref 会被替换成围栏内
    /// canonical 绝对路径再进管线。
    pub fn import(&mut self, specs: &[ImportSpec]) -> Result<ImportReport, ToolError> {
        use crate::fs_fence::{resolve_read, MAX_BATCH_FILES, MAX_FILE_BYTES, MAX_TOTAL_BYTES};
        let roots = self.fs_roots();
        if roots.is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "导入需要工作区（设 BID_WS 环境变量，或用 BidService::open 打开持久化工作区，或 with_fs_root 声明受信导入根）".into(),
            });
        }
        if specs.len() > MAX_BATCH_FILES {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "单批导入件数超限：{} 件 > {} 件（分批重传）",
                    specs.len(),
                    MAX_BATCH_FILES
                ),
            });
        }
        let mut resolved: Vec<ImportSpec> = Vec::with_capacity(specs.len());
        let mut total: u64 = 0;
        for spec in specs {
            let r = resolve_read(&roots, &spec.upload_ref)?;
            let abs = r.absolute();
            // 大小门（存在才量——缺失件交给下游逐件失败语义，不在这里抢答）
            if let Ok(md) = std::fs::metadata(&abs) {
                if md.len() > MAX_FILE_BYTES {
                    return Err(ToolError::EUnprocessable {
                        reason: format!(
                            "{} 超过单文件导入上限（{} 字节 > {} 字节）",
                            spec.filename,
                            md.len(),
                            MAX_FILE_BYTES
                        ),
                    });
                }
                total = total.saturating_add(md.len());
                if total > MAX_TOTAL_BYTES {
                    return Err(ToolError::EUnprocessable {
                        reason: format!(
                            "批次累计导入超限（截至 {} 已 {} 字节 > {} 字节）",
                            spec.filename, total, MAX_TOTAL_BYTES
                        ),
                    });
                }
            }
            resolved.push(ImportSpec {
                upload_ref: abs,
                filename: spec.filename.clone(),
                kind: spec.kind.clone(),
                expected_pages: spec.expected_pages,
            });
        }
        // #270：.doc 转换产物落工作区内部位 <ws>/.mang/convert/（不写用户目录；
        // 无持久化工作区形态 → None → 服务层降级系统 temp）
        let convert_dir = self
            .workspace_dir()
            .map(|ws| ws.join(".mang").join("convert"));
        let report =
            import_files(&mut self.store, &mut self.parsed, &resolved, convert_dir.as_deref());
        self.auto_checkpoint()?;
        Ok(report)
    }

    /// `export_bid_document` 业务半边（#59 阶段 2 + #117 任务 1 表格接线）。
    ///
    /// 步骤：
    /// 1. **四前置门**（防废标的最后关口——fail-closed）：
    ///    a. 矩阵 blocking 行清零（`check_export_ready`——必选未响应/待补不得
    ///       导出，#245 后 PENDING_EVIDENCE 必选行也在此面）；
    ///    b. 审核 BLOCKER findings 清零（`export_allowed`——T-7 闸门）；
    ///    c. 写作验收复算（1.0——`writing_acceptance_findings`：篇幅不足/
    ///       空章节即使审核已直接批准也不得导出，见该函数门禁语义注记）；
    ///    d. 占位证据复算（#245——`placeholder_evidence_findings`：活跃占位
    ///       绑定存在即拒——占位材料不得冒充已核验证据导出；人工豁免通道
    ///       = 合法人工豁免（NOT_APPLICABLE + 偏离说明 + 可信 Human
    ///       receipt，见 `validate::is_legitimate_placeholder_exemption`），
    ///       与 1c 同为独立复算门、不消费记审面）；
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
        // ---- 1c. 写作验收门（1.0——导出时点独立复算，fail-closed）：
        //      篇幅不足/空章节时，即使有人直接批准审核（三门全 Approved、
        //      零 findings 零 RT）也仍禁止导出——记审面是流程事实不是质量
        //      事实，本门不消费 reviews/RT/gate_approved，只复算写作验收
        //      BLOCKER（E_SECTION_EMPTY / E_CHARS_BELOW_MIN，详见
        //      validate::writing_acceptance_findings 的门禁语义注记）。
        let wa = crate::validate::writing_acceptance_findings(self.store());
        if !wa.is_empty() {
            let detail = wa
                .iter()
                .map(|f| {
                    format!(
                        "{}@{}",
                        f.rule_code,
                        f.section_id.as_deref().unwrap_or("PROJECT")
                    )
                })
                .collect::<Vec<_>>()
                .join("、");
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "写作验收门未过：{} 处 BLOCKER（{detail}）——篇幅不足/空章节即使审核已批准也不得导出（导出门独立复算，不消费记审面）；补写后 validate 复核、重走 pre_export_confirm",
                    wa.len(),
                ),
            });
        }
        // ---- 1d. 占位证据门（#245——导出时点独立复算，fail-closed）：
        //      活跃占位绑定存在 → 占位材料不得冒充已核验证据导出。与 1c
        //      同款纪律：不消费记审面（审核直接批准也不放行）；差异面是
        //      人工豁免通道——占位缺失可经合法人工豁免（NOT_APPLICABLE +
        //      偏离说明 + 可信 Human receipt，见 validate::
        //      is_legitimate_placeholder_exemption）豁免，篇幅不足没有该
        //      通道（只能补写）。**普通状态覆写（Responded 等）不豁免**
        //      ——set_matrix_override 对其不强制偏离说明，覆写在场 ≠
        //      豁免成立（PR #247 审核收紧）。
        let ph = crate::validate::placeholder_evidence_findings(self.store());
        if !ph.is_empty() {
            let detail = ph
                .iter()
                .map(|f| {
                    format!(
                        "{}@{}",
                        f.rule_code,
                        f.requirement_id.as_deref().unwrap_or("PROJECT")
                    )
                })
                .collect::<Vec<_>>()
                .join("、");
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "占位证据门未过：{} 处 BLOCKER（{detail}）——占位/待核验材料不得作为已核验证据导出（即使审核已批准）；导入真实材料重新登记（换版升级）或人工覆写豁免后重导",
                    ph.len(),
                ),
            });
        }

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
        // 路径逃逸校验（#189：升级为围栏 resolve_write——旧 validate_output_path
        // 的字符串前缀比较有 `c:/bid/work-evil` 命中 `c:/bid/work` 的前缀陷阱，
        // 且无符号链接检查；组件级比较 + 逐段 symlink 检查在此拦下越界写）。
        // 返回的 canonical 绝对路径作为后续真实写目标（防检查与写入两套口径）。
        let fenced_out = crate::fs_fence::resolve_write(&self.fs_roots(), output_path)?;
        let fenced_abs = fenced_out.absolute();
        let output_path = fenced_abs.as_path();
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
    /// 章节 draft）；状态推进语义（#243 收紧）：
    /// - **前置闸**：scope=all（全稿最终校验）要求起草完成——存在未达
    ///   DRAFTED 的叶 → `E_DRAFT_INCOMPLETE`（missing 清单）零副作用拒，
    ///   不跑规则、不推进、不落盘。起草期的章节级检查走 scope=sections
    ///   （不受此闸——「起草中/章节级检查」与「全稿最终校验」分面）；
    /// - **仅校验通过才推进**：ok=true 且 scope=all → Drafting→Validating
    ///   （复审语义）+ checkpoint；失败（BLOCKER>0）留在原态——项目保持
    ///   可修订、可重试（修订通道：draft_response_section / set_matrix_override /
    ///   证据补绑），无状态变迁则不 checkpoint（纯读）；
    /// - 已在 Validating+ 者 no-op（重验幂等）。
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
    /// NotPerformed）。状态推进 / checkpoint 与现网面同轨（#243：推进仅
    /// 在「scope=all 且 ok=true」时发生，见 [`Self::validate_document`]）。
    pub fn validate_document_with_reviewer(
        &mut self,
        scope_sections: Option<&[String]>,
        model: &mut dyn crate::validate::ModelReviewer,
    ) -> Result<crate::validate::ValidationReport, ToolError> {
        // ---- #243 前置闸：全稿最终校验要求起草完成（章节级检查不受此闸）
        if scope_sections.is_none() {
            let missing = self.store().incomplete_leaf_ids();
            if !missing.is_empty() {
                return Err(ToolError::EDraftIncomplete { missing });
            }
        }
        let report = crate::validate::validate_document(self.store(), scope_sections, model)?;
        // ---- 状态推进（#243：仅全稿校验通过才推进——失败留在原态可修订
        //      重试；幂等面由 advance_project_status 承担，无变迁不 checkpoint）
        if report.ok && scope_sections.is_none() {
            let cur = self.store().project()?.status;
            let steps: &[bid_domain::ProjectStatus] = match cur {
                bid_domain::ProjectStatus::Drafting => &[bid_domain::ProjectStatus::Validating],
                _ => &[],
            };
            for to in steps {
                self.store_mut().advance_project_status(*to)?;
            }
            self.auto_checkpoint()?;
        }
        Ok(report)
    }

    /// `recover_validating_state` 业务半边（#243）：历史异常态恢复。
    ///
    /// 域层裁决（三重约束：仅 VALIDATING / 仅缺章 / 理由非空白）见
    /// [`DomainStore::recover_validating_to_drafting`]；本层只补持久化面
    /// （恢复后 checkpoint——用户不再需要手改 db.json 才能续跑）。
    pub fn recover_validating_state(
        &mut self,
        reason: &str,
    ) -> Result<bid_domain::StateRecoveryRecord, ToolError> {
        let rec = self.store_mut().recover_validating_to_drafting(reason)?;
        self.auto_checkpoint()?;
        Ok(rec)
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

// ---------------------------------------------------------------- #270：.doc 导入通道
//
// 两层设计（issue #270 指令原文：「检查环境自动转换，没有环境让 AI 用 skill 自处理」）：
//   引擎侧：.doc → 检查环境（Word COM / LibreOffice 任一）
//           有 → 自动转换 docx → 正常解析（用户无感，响应面标注「经转换」）
//           无 → 明确报错 + 两项检测结果 + 三种解决方式（Agent 按 Skill 指引自处理）
//   转换通道与 PDF 导出面（convert_to_pdf）同模式：Word COM 优先（投标用户
//   「有 Office 无 LO」是常态），LibreOffice 兜底；不引入新依赖。

/// #270 测试注入缝：置 `BID_NO_DOC_CONVERTER=1` 强制视为「无转换器」
/// （issue 测试节建议的环境变量模拟——CI 有 soffice 的环境也能测无转换器
/// 报错面；**不在测试体内 set_var**，由子进程 env 注入，避并行竞态）。
fn doc_converter_forced_off() -> bool {
    std::env::var_os("BID_NO_DOC_CONVERTER")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// .doc→docx 转换通道是否可用（Word COM 或 LibreOffice 任一在场——与
/// [`pdf_converter_available`] 同原则：只查 soffice 会把「有 Office 无 LO」
/// 的投标用户误判为不可用）。
#[must_use]
pub fn doc_converter_available() -> bool {
    if doc_converter_forced_off() {
        return false;
    }
    #[cfg(windows)]
    {
        if word_com_available() {
            return true;
        }
    }
    soffice_available()
}

/// 两通道探测明细（供无转换器报错的「检测结果」面——逐项可用性+原因）。
#[cfg(windows)]
fn doc_converter_probe() -> (bool, String, bool, String) {
    let ok = word_com_available();
    let reason = if ok {
        "可用".to_string()
    } else {
        "注册表无 WINWORD.EXE（App Paths）——未安装 Microsoft Office".to_string()
    };
    if ok {
        return (true, reason, false, "未探测（Word COM 已可用）".to_string());
    }
    let (lo, lo_reason) = doc_lo_probe();
    (false, reason, lo, lo_reason)
}

/// 非 Windows：Word COM 通道不适用，只探测 LibreOffice。
#[cfg(not(windows))]
fn doc_converter_probe() -> (bool, String, bool, String) {
    let (lo, lo_reason) = doc_lo_probe();
    (false, "非 Windows 环境，Word COM 通道不适用".to_string(), lo, lo_reason)
}

fn doc_lo_probe() -> (bool, String) {
    if which_software("soffice").is_some() || which_software("libreoffice").is_some() {
        (true, "可用".to_string())
    } else {
        (false, "soffice/libreoffice 不在 PATH——未安装 LibreOffice".to_string())
    }
}

/// 当前可用的 .doc 转换通道名（响应面标注用）。
#[must_use]
pub fn doc_converter_name() -> &'static str {
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

/// 无转换器时的完整引导报错（issue #270 文案模板——Agent 按此走 Skill 侧流程）。
fn doc_unsupported_message() -> String {
    let (w, w_reason, l, l_reason) = doc_converter_probe();
    format!(
        "不支持直接解析 .doc（Word 97-2003 旧格式），且本机未检测到可用的转换器。\n\
         \n\
         检测结果：\n\
         \x20 Word COM:    {w_mark}（{w_reason}）\n\
         \x20 LibreOffice: {l_mark}（{l_reason}）\n\
         \n\
         请任选一种方式处理后重新导入：\n\
         \x20 1. 安装 Microsoft Office 或 LibreOffice（免费，任一即可）\n\
         \x20 2. 自行转换后导入 .docx：\n\
         \x20    Word 打开 → 另存为 → .docx\n\
         \x20    或  soffice --headless --convert-to docx --outdir /tmp \"<文件>.doc\"\n\
         \x20 3. 请用户提供 .docx 版本",
        w_mark = if w { "可用" } else { "不可用" },
        l_mark = if l { "可用" } else { "不可用" },
    )
}

/// docx 产物校验（存在 + PK 魔数）——复用判定/转换后防「空文件伪装产物」。
fn docx_artifact_valid(path: &std::path::Path) -> bool {
    let mut magic = [0u8; 4];
    use std::io::Read;
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && &magic == b"PK\x03\x04"
}

/// .doc→docx 总入口：Word COM 优先，LibreOffice 兜底（与 [`convert_to_pdf`] 同序）。
fn convert_doc_to_docx(doc: &std::path::Path, out_docx: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        if word_com_available() {
            match convert_doc_to_docx_word(doc, out_docx) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // Word 失败不弃——续试 LO（COM 权限/占用等场景，同 PDF 面策略）
                    let lo = convert_doc_to_docx_lo(doc, out_docx);
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
    convert_doc_to_docx_lo(doc, out_docx)
}

/// Word COM 自动化 .doc→.docx（Windows 专属——与 [`convert_to_pdf_word`] 同模式：
/// UTF-8 BOM PowerShell 脚本防中文路径 GBK 乱码、try_wait 循环 30s 超时防占用
/// 挂起、taskkill 清 WINWORD 残留、ReleaseComObject 释放 COM 引用）。
/// `SaveAs2(path, 16)` 的 16 = `wdFormatDocumentDefault`（.docx）。
#[cfg(windows)]
fn convert_doc_to_docx_word(doc: &std::path::Path, out_docx: &std::path::Path) -> Result<(), String> {
    /// Word COM 单次转换超时（秒）。冷启动 12s + 转换余量（PDF 面同款取值）。
    const WORD_COM_TIMEOUT_SECS: u64 = 30;
    let script = format!(
        "$word = $null\r\n\
         $doc = $null\r\n\
         try {{\r\n\
         \x20 $word = New-Object -ComObject Word.Application\r\n\
         \x20 $word.Visible = $false\r\n\
         \x20 $word.DisplayAlerts = 0\r\n\
         \x20 $doc = $word.Documents.Open('{}', $false, $true)\r\n\
         \x20 $doc.SaveAs2('{}', 16)\r\n\
         \x20 $doc.Close(0)\r\n\
         }} finally {{\r\n\
         \x20 if ($doc) {{ try {{ [System.Runtime.InteropServices.Marshal]::ReleaseComObject($doc) | Out-Null }} catch {{}} }}\r\n\
         \x20 if ($word) {{ try {{ $word.Quit() }} catch {{}} ; try {{ [System.Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null }} catch {{}} }}\r\n\
         }}\r\n",
        doc.display(),
        out_docx.display()
    );
    // UTF-8 BOM：PowerShell 5.1 无 BOM 按 GBK 解析——中文路径必乱码（PDF 面实测坑）
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    let script_path = std::env::temp_dir().join(format!(
        "bid-doc2docx-{}-{}.ps1",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
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
    if !docx_artifact_valid(out_docx) {
        return Err("Word COM 声称成功但产物缺失或非 docx（PK 魔数校验失败）".into());
    }
    Ok(())
}

/// .doc→.docx（LibreOffice headless——与 [`convert_to_pdf_lo`] 同模式：profile
/// 隔离防并发锁死、产物名 {stem}.docx 先落转换目录再原子 rename、PK 魔数校验）。
fn convert_doc_to_docx_lo(doc: &std::path::Path, out_docx: &std::path::Path) -> Result<(), String> {
    let soffice = which_software("soffice")
        .or_else(|| which_software("libreoffice"))
        .ok_or_else(|| "soffice/libreoffice 不在 PATH（环境缺失）".to_string())?;
    if let Some(dir) = out_docx.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("输出目录创建失败: {e}"))?;
    }
    let profile = std::env::temp_dir().join(format!(
        "bid-soffice-doc2docx-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    let conv_out = profile.join("out");
    std::fs::create_dir_all(&conv_out).map_err(|e| format!("转换目录创建失败: {e}"))?;
    // file:/// 三斜杠（两斜杠是 host 形式会被静默忽略 → 共享默认 profile → 锁冲突）
    let profile_url = format!("file:///{}", profile.display());
    let status = std::process::Command::new(&soffice)
        .arg("--headless")
        .arg("--norestore")
        .arg(format!("-env:UserInstallation={profile_url}"))
        .arg("--convert-to")
        .arg("docx")
        .arg("--outdir")
        .arg(&conv_out)
        .arg(doc)
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
    let stem = doc.file_stem().and_then(|s| s.to_str()).unwrap_or("标书");
    let produced = conv_out.join(format!("{stem}.docx"));
    if !produced.is_file() {
        let ls = std::fs::read_dir(&conv_out)
            .ok()
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let _ = std::fs::remove_dir_all(&profile);
        return Err(format!(
            "soffice 声称成功但产物缺失（期望 {}；outdir 内容: [{}]；stdout: {}）",
            produced.display(),
            ls,
            stdout.chars().take(300).collect::<String>()
        ));
    }
    if !docx_artifact_valid(&produced) {
        let _ = std::fs::remove_dir_all(&profile);
        return Err("产物不是 docx（PK 魔数校验失败）——soffice 可能输出了错误格式".into());
    }
    std::fs::rename(&produced, out_docx).map_err(|e| format!("产物搬移失败: {e}"))?;
    let _ = std::fs::remove_dir_all(&profile);
    Ok(())
}

/// OLE2/CFB 容器魔数（Word 97-2003 旧格式 .doc 的二进制签名）。
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// 扩展名 .docx 的文件实际是 OLE2 容器（.doc 改名而来）→ 识别 + 友好引导
/// （#270 顺带修复：原来直接喂 zip 引擎报「container corrupt: Could not find
/// EOCD」——用户误以为文件损坏）。语义边界：这是**路由面的格式识别与友好
/// 报错**，不改变「按扩展名路由」的既有设计（PK 头异常内容仍走引擎响亮失败）。
fn is_ole_container(path: &std::path::Path) -> bool {
    let mut magic = [0u8; 8];
    use std::io::Read;
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic == OLE_MAGIC
}

/// .doc 改名 .docx 的引导报错（区别于「容器损坏」——文件没坏，是格式旧）。
fn docx_lie_message() -> String {
    "该文件扩展名是 .docx，但内容是 OLE2 容器（Word 97-2003 旧格式 .doc 的二进制\
     签名）——疑似 .doc 直接改名而来，docx 引擎（zip 容器）无法解析。文件本身未损坏。\
     请任选其一：① 以 .doc 扩展名重新导入（本机有转换器时引擎将自动转换）；\
     ② 用 Word 打开另存为 .docx 后导入；③ soffice --headless --convert-to docx \
     --outdir /tmp \"<文件>\"".to_string()
}

/// .doc 导入的转换+解析半边（#270）：
/// 1. 无转换器 → 引导报错（Skill 侧接管）；
/// 2. 转换产物落 `convert_dir`（有工作区传 `<ws>/.mang/convert/`，`None` 时
///    降级系统 temp——不写用户目录）；产物名 `{原件 sha 前 16 位}.docx`——
///    同件重导不重转（幂等），PK 头校验失败自动重转；
/// 3. **sha256 记原件**（覆写转换产物的 ParsedFile.sha256/name——引擎对传入
///    字节自算 sha 是转换产物的，不覆写会破坏「累积面与落库面同源」不变量）；
/// 4. parse_summary 前缀标注「经转换」（落库面；warnings 面由调用方补）。
fn import_doc_converted(
    doc: &std::path::Path,
    filename: &str,
    orig_sha: &str,
    convert_dir: Option<&std::path::Path>,
) -> Result<bid_parse::ParsedFile, String> {
    if !doc_converter_available() {
        return Err(doc_unsupported_message());
    }
    let dir = convert_dir
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("bid-doc-convert"));
    let target = dir.join(format!("{}.docx", &orig_sha[..16.min(orig_sha.len())]));
    if !docx_artifact_valid(&target) {
        convert_doc_to_docx(doc, &target)?;
    }
    let mut p = bid_engine::parse_docx_via_engine(&target).map_err(|e| e.to_string())?;
    // 不变量覆写：累积面（parsed[]）与落库面（upsert_file 的 sha）必须同源——
    // 都是原件的。name 用 spec.filename（块定位/抽取锚定以它为标签）。
    p.sha256 = orig_sha.to_string();
    p.name = filename.to_string();
    p.parse_summary = format!(
        "[.doc→.docx 经 {} 转换后解析] {}",
        doc_converter_name(),
        p.parse_summary
    );
    Ok(p)
}

/// 当前时刻（unix epoch 秒——#188 FileRole 登记时刻；时钟不可用时 0，
/// 角色的时刻面是审计信息，不参与判定）。
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
