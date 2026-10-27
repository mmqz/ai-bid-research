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
        // ---- M0 骨架：以下工具待各自里程碑接线 ----
        "extract_requirements" => Err(ToolError::unimplemented(name, "M1-b（抽取接线）")),
        "get_compliance_matrix" => Err(ToolError::unimplemented(name, "M2-a（矩阵构建）")),
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
    // 转发（领域：external_key 幂等创建；tender_notice_ref 即招标公告编号 → code 位）
    let (_, mut out) = svc
        .store_mut()
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

    // ---- 转发业务（服务层：sha256 幂等 + 解析 + DomainStore 落库 + 失败登记）
    let report = svc.import(&specs);

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
