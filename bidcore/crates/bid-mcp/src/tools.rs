//! 工具实现分派表。
//!
//! **纪律**：本模块只做参数解包 + 状态机裁决 + 转发。任何业务判断都应下沉到
//! `bid-domain` / `bid-parse` / `bid-kb` / `bid-tools::service`。骨架期未落地的
//! 工具一律返回 `UNIMPLEMENTED_INTERNAL`（带目标里程碑），**不得返回空成功**。
//!
//! B-1（issue #11）落地 `create_bid_project`（§1，import 的 stdio 验收前置——
//! 无项目则导入无处落库）与 `import_tender_files`（§2 主任务）。两者的业务
//! 管线在 `bid_tools::service`（由 runner.rs 提升的单一真源），本层只做：
//! 参数校验（E_UNPROCESSABLE）→ 项目路由（E_NOTFOUND）→ 状态机裁决
//! （`guard_tool`，E_STATE）→ 乐观并发（E_CONFLICT）→ 转发 → 响应组装
//! （契约 §2 out + 包装字段 project_version / request_id / warnings）。

use serde_json::{json, Value};
use std::path::PathBuf;

use crate::error::{ToolError, ToolResult};
use crate::state::next_request_id;
use bid_domain::{ParseStatus, ProjectStatus};
use bid_tools::service::{BidService, ImportSpec};

/// 按工具名分派。调用方保证 name 已在 [`crate::TOOL_NAMES`] 内。
pub(crate) fn invoke(name: &str, args: Value, svc: &mut BidService) -> ToolResult {
    match name {
        "get_bid_project_state" => get_bid_project_state(svc, args),
        "create_bid_project" => create_bid_project(svc, args),
        "import_tender_files" => import_tender_files(svc, args),
        "extract_requirements" => extract_requirements(svc, args),
        "get_compliance_matrix" => get_compliance_matrix(svc, args),
        // ---- M0 骨架：以下工具待各自里程碑接线 ----
        "search_evidence" => Err(ToolError::unimplemented(name, "M2-b（双通道检索）")),
        "draft_response_section" => Err(ToolError::unimplemented(name, "M3（起草 + 内核 CAS 写入）")),
        "validate_bid_document" => Err(ToolError::unimplemented(name, "M4（规则注册表）")),
        "export_bid_document" => Err(ToolError::unimplemented(name, "M5（内核导出适配器）")),
        other => Err(ToolError::ENotFound {
            what: format!("未接入的工具 {other}"),
        }),
    }
}

// ---------------------------------------------------------------- 参数校验助手

/// 契约 §2 `files[].kind` 枚举（MCP 壳校验面；服务层不重复——运行器自有
/// 类别如「证据」不受此限，见 service.rs 分层说明）。
const TENDER_FILE_KINDS: [&str; 7] =
    ["招标文件", "附件", "图纸", "补遗", "答疑", "评分办法", "格式模板"];

fn req_str(v: &Value, key: &str, at: &str) -> Result<String, ToolError> {
    let s = v.get(key).and_then(Value::as_str).ok_or_else(|| ToolError::EUnprocessable {
        what: format!("{at}: {key} 缺失或非字符串"),
    })?;
    if s.trim().is_empty() {
        return Err(ToolError::EUnprocessable { what: format!("{at}: {key} 不得为空白") });
    }
    Ok(s.to_string())
}

fn opt_str(v: &Value, key: &str, at: &str) -> Result<Option<String>, ToolError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ToolError::EUnprocessable { what: format!("{at}: {key} 须为字符串") }),
    }
}

fn opt_u64(v: &Value, key: &str, at: &str) -> Result<Option<u64>, ToolError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{at}: {key} 须为非负整数"),
        }),
        Some(_) => Err(ToolError::EUnprocessable { what: format!("{at}: {key} 须为非负整数") }),
    }
}

fn parse_status_str(s: ParseStatus) -> &'static str {
    match s {
        ParseStatus::Ok => "ok",
        ParseStatus::OcrRequired => "ocr_required",
        ParseStatus::Failed => "failed",
    }
}

// ---------------------------------------------------------------- §0 只读面

/// `get_bid_project_state` —— 只读状态查询。
///
/// - 显式 `project_status` → M0 纯函数面（调用方带入假想状态做前置探测，
///   不触服务态——骨架期既有契约，测试与 MCP 客户端的 dry-run 依赖它）；
/// - 缺省 → 读服务态实况（B-1 起写面在位，不再以 DRAFT 缺省伪造未存储的
///   项目——无项目 → E_NOTFOUND）。
///
/// 返回：`{ status, allowed_next_tools, terminal }`
fn get_bid_project_state(svc: &mut BidService, args: Value) -> ToolResult {
    let status = match args.get("project_status") {
        Some(Value::String(raw)) => parse_status(raw).ok_or_else(|| ToolError::EUnprocessable {
            what: format!("无法识别的 project_status：{raw}（合法值见 bid-domain ProjectStatus）"),
        })?,
        None | Some(Value::Null) => svc
            .store()
            .project()
            .map_err(ToolError::from)?
            .status,
        Some(_) => {
            return Err(ToolError::EUnprocessable { what: "project_status 须为字符串".into() });
        }
    };

    Ok(json!({
        "status": status.as_str(),
        "allowed_next_tools": status.allowed_tools(),
        "terminal": status.next_allowed().is_empty(),
    }))
}

fn parse_status(raw: &str) -> Option<ProjectStatus> {
    use ProjectStatus::*;
    let map = [
        ("DRAFT", Draft),
        ("FILES_IMPORTED", FilesImported),
        ("REQUIREMENTS_EXTRACTED", RequirementsExtracted),
        ("MATRIX_READY", MatrixReady),
        ("EVIDENCE_BOUND", EvidenceBound),
        ("DRAFTING", Drafting),
        ("VALIDATING", Validating),
        ("REVIEWING", Reviewing),
        ("READY_TO_EXPORT", ReadyToExport),
        ("EXPORTED", Exported),
    ];
    map.iter().find(|(k, _)| *k == raw).map(|(_, v)| *v)
}

// ---------------------------------------------------------------- §1 create_bid_project

/// `create_bid_project` —— 契约 §1（B-1 最小前置）。
///
/// 领域语义（external_key 幂等）`DomainStore::create_project` 早已承载；
/// 本壳只做参数校验 + 幂等前置（重放不是状态转换，先于 guard）+ 转发 +
/// 包装字段。deadline 已过 → 创建即警告 `BID_DEADLINE_PAST`（契约 §1，
/// 不阻断）。
fn create_bid_project(svc: &mut BidService, args: Value) -> ToolResult {
    let name = req_str(&args, "name", "create_bid_project")?;
    let bid_deadline = req_str(&args, "bid_deadline", "create_bid_project")?;
    let mode = req_str(&args, "mode", "create_bid_project")?;
    if !matches!(mode.as_str(), "明标" | "暗标") {
        return Err(ToolError::EUnprocessable {
            what: format!("mode 须为 明标|暗标（契约 §1），实际：{mode}"),
        });
    }
    let external_key = req_str(&args, "external_key", "create_bid_project")?;
    let notice_ref = opt_str(&args, "tender_notice_ref", "create_bid_project")?;
    let deadline = parse_ymd(&bid_deadline).ok_or_else(|| ToolError::EUnprocessable {
        what: format!("bid_deadline 须为 YYYY-MM-DD，实际：{bid_deadline}"),
    })?;

    // 状态机裁决：create 是状态机的**入口边**（→ DRAFT）——无项目时无可裁决
    // 的状态（guard_tool 的裁决域是「已存在项目的当前状态」），入口由
    // DomainStore::create_project 自身承载（幂等 + 单项目约束）。已有项目时：
    // 同 key 重放 → 静默幂等（§1，先于 guard——重放不是状态转换）；异 key →
    // guard_tool 裁决（非 DRAFT 态再建 = 越级 → E_STATE 零副作用）。
    if let Ok(p) = svc.store().project() {
        if p.external_key == external_key {
            return Ok(json!({
                "project_id": p.id,
                "status": p.status.as_str(),
                "deduplicated": true,
                "project_version": svc.store().content_version(),
                "request_id": next_request_id(),
                "warnings": [],
            }));
        }
        svc.store().guard_tool("create_bid_project").map_err(ToolError::from)?;
    }
    // 转发（领域：external_key 幂等创建；tender_notice_ref 即招标公告编号 → code 位；
    // T-8：经服务层包装——持久化在场时写后自动落盘）
    let (_, mut out) = svc
        .create_project(&name, &notice_ref.unwrap_or_default(), &mode, &external_key)
        .map_err(ToolError::from)?;
    let mut warnings = Vec::new();
    if deadline < today_ymd() {
        warnings.push(format!("BID_DEADLINE_PAST: 投标截止 {bid_deadline} 已过（契约 §1：创建即警告，不阻断）"));
    }
    out["project_version"] = json!(svc.store().content_version());
    out["request_id"] = json!(next_request_id());
    out["warnings"] = json!(warnings);
    Ok(out)
}

/// `YYYY-MM-DD` 解析（前缀口径——截止时刻比对以日为粒度）。
fn parse_ymd(s: &str) -> Option<(i64, u32, u32)> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

/// 天数 → 公历（Howard Hinnant civil_from_days；壳层手写以避免为一个
/// 日期比较引入 chrono 重依赖——薄壳零冗余纪律）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;                                  // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);            // [0, 365]
    let mp = (5 * doy + 2) / 153;                                 // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;                // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;       // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn today_ymd() -> (i64, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days(secs.div_euclid(86_400))
}

// ---------------------------------------------------------------- §2 import_tender_files

/// `import_tender_files` —— 契约 §2（B-1 主任务）。
///
/// 状态机口径（契约 §2「错误: E_STATE」行 + 任务裁决）：DRAFT 首导 →
/// FILES_IMPORTED；非 DRAFT 合法追加（FILES_IMPORTED / REQUIREMENTS_EXTRACTED）
/// → 允许 + dirty 告警（不阻断）；MATRIX_READY+ 越级 → E_STATE（guard_tool
/// 裁决，零副作用）。部分文件 parse FAILED → E_PARTIAL（失败件全程禁引，
/// 成功面统计随 `data` 携带——R-01）。
fn import_tender_files(svc: &mut BidService, args: Value) -> ToolResult {
    // ---- 参数校验（契约 §2 in）
    let project_id = req_str(&args, "project_id", "import_tender_files")?;
    let files = args.get("files").and_then(Value::as_array).ok_or_else(|| ToolError::EUnprocessable {
        what: "files 缺失或非数组（契约 §2：files:[{upload_ref, filename, kind?, expected_pages?}]）".into(),
    })?;
    if files.is_empty() {
        return Err(ToolError::EUnprocessable {
            what: "files 不得为空（空批次无导入语义——查状态用 get_bid_project_state）".into(),
        });
    }
    let project_version = opt_u64(&args, "project_version", "import_tender_files")?;
    let mut specs = Vec::with_capacity(files.len());
    for (i, f) in files.iter().enumerate() {
        let at = format!("files[{i}]");
        let upload_ref = req_str(f, "upload_ref", &at)?;
        let filename = req_str(f, "filename", &at)?;
        let kind = match opt_str(f, "kind", &at)? {
            Some(k) if TENDER_FILE_KINDS.contains(&k.as_str()) => k,
            Some(k) => {
                return Err(ToolError::EUnprocessable {
                    what: format!("{at}: kind 非法：{k}（合法值：{}）", TENDER_FILE_KINDS.join("|")),
                });
            }
            None => "附件".to_string(), // 契约 §2 kind 可选——缺省归通用附件
        };
        let expected_pages = opt_u64(f, "expected_pages", &at)?;
        specs.push(ImportSpec { upload_ref: PathBuf::from(upload_ref), filename, kind, expected_pages });
    }

    // ---- 项目路由（E_NOTFOUND）
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!("project 不存在：{project_id}（当前服务态项目：{}）", proj.id),
        });
    }
    let status_before = proj.status;

    // ---- 状态机裁决（guard_tool——本层不自行判断）
    svc.store().guard_tool("import_tender_files").map_err(ToolError::from)?;
    // ---- 乐观并发（project_version 缺省 = 内部步骤语义，跳过检查）
    svc.store().check_version(project_version).map_err(ToolError::from)?;

    // ---- 转发业务（服务层：sha256 幂等 + 解析 + DomainStore 落库 + 失败登记；
    // T-8：持久化在场时写后自动落盘——落盘失败与业务失败同报，不吞面）
    let report = svc.import(&specs).map_err(ToolError::from)?;

    // ---- 响应组装（契约 §2 out + 包装字段）
    let mut file_ids: Vec<String> = Vec::with_capacity(report.per_file.len());
    let mut per_file: Vec<Value> = Vec::with_capacity(report.per_file.len());
    let mut item_errors: Vec<String> = Vec::new();
    let (mut totals_files, mut totals_chunks, mut parse_failed) = (0usize, 0usize, 0usize);
    for o in &report.per_file {
        if let Some(fid) = &o.file_id {
            // per_file 仅含有文件身份的条目（不可读 upload_ref 无档可报——入 item_errors）
            file_ids.push(fid.clone());
            totals_files += 1;
            totals_chunks += o.chunks;
            if o.parse_status == ParseStatus::Failed {
                parse_failed += 1;
            }
            let mut entry = json!({
                "file_id": fid,
                "filename": o.filename,
                "sha256": o.sha256,
                "pages": o.pages,
                "tables": o.tables,
                "chunks": o.chunks,
                "ocr_tier": o.ocr_tier,
                "parse_status": parse_status_str(o.parse_status),
                "deduplicated": o.deduplicated,
            });
            if !o.errors.is_empty() {
                entry["errors"] = json!(o.errors);
            }
            if !o.warnings.is_empty() {
                entry["warnings"] = json!(o.warnings);
            }
            per_file.push(entry);
        }
        for e in &o.errors {
            item_errors.push(format!("{}: {e}", o.filename));
        }
    }
    let status_after = svc.store().project().map(|p| p.status.as_str()).unwrap_or("DRAFT");
    let mut warnings = Vec::new();
    if report.new_records > 0 && status_before != ProjectStatus::Draft {
        // 契约 §2：非 DRAFT 追加新文件 → 允许但标 dirty（纯 no-op 重放不标——
        // content_version 未动，无物可脏；dirty 的并发语义即版本推进）
        warnings.push(format!(
            "DIRTY_APPEND: 项目已过 DRAFT（当前 {}）追加新文件——下游抽取/矩阵面需重算（契约 §2 dirty 波及）",
            status_before.as_str()
        ));
    }
    let data = json!({
        "file_ids": file_ids,
        "per_file": per_file,
        "totals": {
            "files": totals_files,
            "chunks": totals_chunks,
            "parse_failed": parse_failed,
            "partial": report.item_failures,
        },
        "status": status_after,
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
        "warnings": warnings,
    });

    if report.item_failures > 0 {
        // 契约 §2 错误面：E_PARTIAL（部分文件 parse FAILED；失败件内容全程禁引）。
        // data 随错误携带——R-01：Agent 核对返回统计并重试失败件，错误码不吞成功面。
        return Err(ToolError::EPartial {
            what: format!("{} 件导入失败（失败件内容全程禁引）", report.item_failures),
            item_errors,
            hint: Some("失败件修复后以新内容重传（新 sha256 即新档）；成功件已落库可用".into()),
            data: Some(data),
        });
    }
    Ok(data)
}

// ================================================================ extract_requirements（契约 §3）

/// 类别 → 契约键（counts.by_category 口径）。
fn category_key(cn: &str) -> &'static str {
    match cn {
        "资格条件" => "qualification",
        "否决项" => "veto",
        "评分项" => "scoring",
        "技术参数" => "tech_param",
        "商务要求" => "commercial",
        "格式要求" => "format",
        _ => "other",
    }
}

/// 条款号数值化（`3.2.1` → [3, 2, 1]——缺号检测的序比较基元）。
fn clause_digits(no: &str) -> Option<Vec<u64>> {
    let parts: Vec<&str> = no.split('.').collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    parts.iter().map(|p| p.trim().parse::<u64>().ok()).collect()
}

/// 同前缀下 1..=n 连续性检测 → 缺号清单（契约硬规则：缺号必须报告）。
fn missing_in_run(found: &mut Vec<Vec<u64>>, out: &mut Vec<Value>, file_id: &str) {
    found.sort();
    let mut i = 0usize;
    while i + 1 < found.len() {
        let (a, b) = (&found[i], &found[i + 1]);
        // 同父路径下的相邻兄弟：末位差 >1 → 报缺号区间
        if a.len() == b.len() && a[..a.len() - 1] == b[..b.len() - 1] {
            let (x, y) = (a[a.len() - 1], b[b.len() - 1]);
            if y > x + 1 && y - x <= 50 {
                // 防伪缺号：跳章（y - x > 50）多为主线切换而非真缺（P2 评测口径）
                let missing: Vec<String> = ((x + 1)..y).map(|n| {
                    let mut seg = a[..a.len() - 1].to_vec();
                    seg.push(n);
                    seg.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(".")
                }).collect();
                out.push(json!({
                    "file_id": file_id,
                    "expected_clause_nos": missing,
                    "after": a.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("."),
                }));
            }
        }
        i += 1;
    }
}

fn extract_requirements(svc: &mut BidService, args: Value) -> ToolResult {
    // ---- 参数校验（契约 §3 in）
    let project_id = req_str(&args, "project_id", "extract_requirements")?;
    let scope = opt_str(&args, "scope", "extract_requirements")?.unwrap_or_else(|| "all".into());
    if scope != "all" {
        return Err(ToolError::EUnprocessable {
            what: format!("scope={scope} 暂不支持（B5 最小面：all——文件子集过滤属 M2 扩展）"),
        });
    }
    if args.get("options").map(Value::is_object).unwrap_or(true) == false {
        return Err(ToolError::EUnprocessable { what: "options 必须为对象".into() });
    }
    let project_version = opt_u64(&args, "project_version", "extract_requirements")?;

    // ---- 项目路由 + 状态机裁决（E_NOTFOUND / E_STATE）
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!("project 不存在：{project_id}（当前服务态项目：{}）", proj.id),
        });
    }
    svc.store().guard_tool("extract_requirements").map_err(ToolError::from)?;
    svc.store().check_version(project_version).map_err(ToolError::from)?;

    // ---- 前置：导入面必须有成功解析件（空抽取面 = E_STATE 语义前置）
    let parsed_empty = svc.parsed().is_empty();
    if parsed_empty {
        return Err(ToolError::EState {
            what: format!("{} 态无已解析文件——先 import_tender_files", proj.status.as_str()),
        });
    }

    // ---- 转发业务：确定性先行抽取（extract.rs 规则——输入为引擎/旧路径
    //      产出的同型 ParsedFile，B5 起主文件 docx 已走 bid-engine）
    let reqs = bid_parse::extract::extract_requirements(svc.parsed());

    // ---- 完整性校验：缺号检测（契约硬规则）
    let mut missing_sequence: Vec<Value> = Vec::new();
    let files = svc.store().files().to_vec();
    for f in &files {
        let mut found: Vec<Vec<u64>> = reqs
            .iter()
            .filter(|r| r.source_file == f.name)
            .filter_map(|r| r.clause_no.as_deref().and_then(clause_digits))
            .collect();
        if found.len() >= 2 {
            missing_in_run(&mut found, &mut missing_sequence, &f.id);
        }
    }

    // ---- 冲突检测（最小面）：补遗（C-）与主文件（A-）同条款号不同文本 → 双向登记
    let mut conflicts: Vec<Value> = Vec::new();
    let main_names: Vec<&str> = svc
        .parsed()
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    // 补遗件名集合（C- 前缀——B5 最小冲突面：主文件 × 补遗同条款号异文）
    let addenda_names: Vec<String> = svc
        .parsed()
        .iter()
        .filter(|p| p.name.starts_with("C-"))
        .map(|p| p.name.clone())
        .collect();
    for ad_name in &addenda_names {
        for r_ad in reqs.iter().filter(|r| r.source_file == *ad_name) {
            if let Some(no) = &r_ad.clause_no {
                for r_main in reqs.iter().filter(|r| {
                    r.source_file != *ad_name
                        && main_names.contains(&r.source_file.as_str())
                        && r.clause_no.as_deref() == Some(no.as_str())
                }) {
                    let (ex_ad, ex_main) = (
                        r_ad.anchors.first().map(|a| a.excerpt.as_str()).unwrap_or(""),
                        r_main.anchors.first().map(|a| a.excerpt.as_str()).unwrap_or(""),
                    );
                    if ex_ad != ex_main {
                        conflicts.push(json!({
                            "a": r_main.id, "b": r_ad.id,
                            "reason": format!("条款 {no}：主文件与补遗文本不一致（{} vs {}）",
                                truncate(ex_main, 30), truncate(ex_ad, 30)),
                        }));
                    }
                }
            }
        }
    }

    // ---- 落库（幂等合并 + 状态转换 FILES_IMPORTED → REQUIREMENTS_EXTRACTED；
    // T-8：经服务层包装——持久化在场时写后自动落盘）
    let (total, added) = svc
        .set_requirements(reqs)
        .map_err(ToolError::from)?;

    // ---- counts 组装
    let mut by_category: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for r in svc.store().requirements() {
        *by_category.entry(category_key(&r.category)).or_insert(0) += 1;
    }
    let mut warnings: Vec<String> = Vec::new();
    if !missing_sequence.is_empty() {
        warnings.push(format!(
            "MISSING_SEQUENCE: {} 处条款缺号（完整性校验面——详见 missing_sequence）",
            missing_sequence.len()
        ));
    }
    if !conflicts.is_empty() {
        warnings.push(format!(
            "CONFLICTS: {} 对主文件/补遗条款冲突（双向登记）",
            conflicts.len()
        ));
    }
    for p in svc.parsed() {
        if p.parse_summary.starts_with("engine-") {
            warnings.push(format!(
                "ENGINE_PATH: {} 经引擎路径解析（已知缺口：嵌套表 mdcore#39 / 域边界 mdcore#40——docs/b3-3-acceptance-report.md §6）",
                p.name
            ));
            break;
        }
    }

    Ok(json!({
        "counts": {
            "total": total,
            "added": added,
            "by_category": by_category,
        },
        "conflicts": conflicts,
        "missing_sequence": missing_sequence,
        "warnings": warnings,
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

// ================================================================ get_compliance_matrix（契约 §4，只读）

fn get_compliance_matrix(svc: &mut BidService, args: Value) -> ToolResult {
    // ---- 参数校验（契约 §4 in：project_id + filters + page）
    let project_id = req_str(&args, "project_id", "get_compliance_matrix")?;
    let filter_category = args
        .get("filters")
        .and_then(|f| f.get("category"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let filter_status = args
        .get("filters")
        .and_then(|f| f.get("status"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let page = opt_u64(&args, "page", "get_compliance_matrix")?.unwrap_or(1).max(1);
    let page_size: usize = 200; // B5 最小面：固定页大小（M2 扩展游标分页）

    // ---- 项目路由（只读工具：无状态机裁决——契约 §4 任何阶段可调）
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!("project 不存在：{project_id}（当前服务态项目：{}）", proj.id),
        });
    }

    // ---- 行组装（V3-2：矩阵派生真源——证据列接 T-1 绑定真值，状态接 T-2 四态派生；
    // 只读工具零状态副作用：matrix_view 派生不落库不 bump（与 build_matrix 同源））
    let view = svc.store().matrix_view();
    let reqs = svc.store().requirements().to_vec();
    let mut rows: Vec<Value> = Vec::with_capacity(reqs.len());
    for (i, r) in reqs.iter().enumerate() {
        if let Some(c) = &filter_category {
            if category_key(&r.category) != c && r.category != *c {
                continue;
            }
        }
        let row = view.iter().find(|m| m.requirement_id == r.id);
        // T-2 四态（派生：override > 证据+章节）；旧 B5 字面量 unanswered → NOT_RESPONDED
        let status = row.map(|m| m.status.as_str()).unwrap_or("NOT_RESPONDED");
        if let Some(s) = &filter_status {
            if status != s {
                continue;
            }
        }
        let anchor = r.anchors.first();
        // 契约 §4 evidence 列接真值（T-1 绑定 + 台账状态/有效期随行——过期证据
        // 绑定已被 matrix_view 过滤，不进响应面）
        let evidence = match row {
            Some(m) => bid_domain::matrix::evidence_column(svc.store().evidences(), &m.evidence_ids),
            None => vec![],
        };
        rows.push(json!({
            "cmp_id": format!("CMP-{:04}", i + 1),
            "requirement": {
                "id": r.id,
                "clause_no": r.clause_no,
                "category": r.category,
                "category_key": category_key(&r.category),
                "statement": anchor.map(|a| a.excerpt.as_str()).unwrap_or(""),
                "mandatory": r.category == "否决项" || r.category == "资格条件",
                "source_anchor": anchor.map(|a| json!({
                    "file": a.file, "kind": a.kind, "locator": a.locator, "excerpt": a.excerpt,
                })),
                "params": r.params,
            },
            "status": status,
            "evidence": evidence,
            "gap": Value::Null,
            "response_policy": "default",
            "draft_section_id": row.and_then(|m| m.response_section.clone()),
            "blocking": row.map(|m| m.blocking).unwrap_or(false),
        }));
    }
    let total_rows = rows.len();
    let start = ((page as usize - 1) * page_size).min(total_rows);
    let end = (start + page_size).min(total_rows);
    let rows = rows[start..end].to_vec();

    let mut warnings: Vec<String> = Vec::new();
    if svc.store().requirements().is_empty() {
        warnings.push("EMPTY_MATRIX: 条款集为空（先 extract_requirements）".to_string());
    }

    Ok(json!({
        "rows": rows,
        "counts": {
            "total": total_rows,
            "returned": rows.len(),
            "page": page,
            "page_size": page_size,
        },
        "warnings": warnings,
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}
