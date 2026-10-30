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
//!
//! V3-5（issue #11 批次 v3 / S-4 游标分页）：§4 族只读列表工具
//! （get_compliance_matrix / list_evidence / list_tasks）统一走 [`crate::cursor`]
//! 的游标信封——先过滤后分页、行序确定性、游标与过滤器绑定（fail-closed）。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::cursor;
use crate::error::{ToolError, ToolResult};
use crate::state::next_request_id;
use bid_domain::evidence::{EvidenceKind, EvidenceStatus};
use bid_domain::{ParseStatus, ProjectStatus};
use bid_tools::service::{BidService, ImportSpec};

/// 按工具名分派。调用方保证 name 已在 [`crate::TOOL_NAMES`] 内。
pub(crate) fn invoke(name: &str, args: Value, svc: &mut BidService) -> ToolResult {
    match name {
        "get_bid_project_state" => get_bid_project_state(svc, args),
        // ---- #42 阶段 10：AI 主动存版本（skill 纪律「每完成一个章节调用」）----
        // 与桌面壳 snapshots.save 同一真源（mang-snap）；不走项目状态机（版本
        // 快照与项目状态正交，任何阶段都允许存）。
        "save_version" => save_version(args),
        "create_bid_project" => create_bid_project(svc, args),
        "import_tender_files" => import_tender_files(svc, args),
        "extract_requirements" => extract_requirements(svc, args),
        "get_compliance_matrix" => get_compliance_matrix(svc, args),
        // ---- S-4 大结果分页（V3-5）：只读列表工具，游标信封见 cursor.rs ----
        "list_evidence" => list_evidence(svc, args),
        "list_tasks" => list_tasks(svc, args),
        // ---- #59 阶段 1：单章起草（契约 §6——mock 生成 + 引用回查 + CAS 写入）----
        "draft_response_section" => draft_response_section(svc, args),
        // ---- #59 阶段 2：导出（契约 §8——双前置门 + format-spec + 暗标）----
        "export_bid_document" => export_bid_document(svc, args),
        // ---- #59 阶段 3：双层校验（契约 §7——确定性规则先行 + 模型复核注入）----
        "validate_bid_document" => validate_bid_document(svc, args),
        // ---- M0 骨架：以下工具待各自里程碑接线 ----
        "search_evidence" => Err(ToolError::unimplemented(name, "M2-b（双通道检索——#59 已降级可选，素材库上来再启）")),
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

/// filters 对象解包（契约 §4 族列表工具共用）：缺席/null → None；非对象 →
/// E_UNPROCESSABLE（S-4 收紧面：把 filters 传成字符串/数组被静默吞掉是
/// B5 期的宽松遗留，fail-closed 纪律下显式拒）。
fn filters_of<'a>(args: &'a Value, tool: &str) -> Result<Option<&'a Value>, ToolError> {
    match args.get("filters") {
        None | Some(Value::Null) => Ok(None),
        Some(v @ Value::Object(_)) => Ok(Some(v)),
        Some(_) => Err(ToolError::EUnprocessable {
            what: format!("{tool}: filters 须为对象（如 {{\"category\": \"scoring\"}}）"),
        }),
    }
}

/// filters 内的单字符串过滤位（缺席/null → None；非字符串 → 拒）。
fn filter_str(filters: Option<&Value>, key: &str, tool: &str) -> Result<Option<String>, ToolError> {
    match filters.and_then(|f| f.get(key)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ToolError::EUnprocessable {
            what: format!("{tool}: filters.{key} 须为字符串"),
        }),
    }
}

/// 项目路由（§4 族只读列表工具共用）：project_id 与服务态项目不符 →
/// E_NOTFOUND（只读工具无状态机裁决——任何阶段可调）。
fn route_project(svc: &BidService, project_id: &str, tool: &str) -> Result<(), ToolError> {
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!("project 不存在：{project_id}（当前服务态项目：{}）——{tool}", proj.id),
        });
    }
    Ok(())
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

/// `get_compliance_matrix` —— 契约 §4（只读，任何阶段可调）。
///
/// **S-4 游标分页升级（V3-5）**：
/// - 入参新增 `cursor?` / `limit?`；`page?` 保留兼容旧调用，与 cursor 并存时
///   **cursor 优先**（page 映射 offset=(page-1)*limit 后走同一游标路径）；
/// - 出参信封 `{ rows, total, limit, cursor(本次生效), next_cursor }`——
///   `next_cursor=null` 即取完；`total` 是**过滤后总数**（先过滤后分页）；
/// - 行序 = 条款集位置序（`cmp_id` CMP-0001..N 按全量位置派生——确定性稳定序，
///   分页安全的前提）；12,000 行量级 O(N) 组装（矩阵行反查走 HashMap，
///   不做逐行线性 find）；
/// - 游标与过滤器绑定（cursor.rs 指纹）：跨过滤器复用 → E_UNPROCESSABLE。
fn get_compliance_matrix(svc: &mut BidService, args: Value) -> ToolResult {
    // ---- 参数校验（契约 §4 in：project_id + filters + cursor/limit/page）
    let project_id = req_str(&args, "project_id", "get_compliance_matrix")?;
    let filters = filters_of(&args, "get_compliance_matrix")?;
    let filter_category = filter_str(filters, "category", "get_compliance_matrix")?;
    let filter_status = filter_str(filters, "status", "get_compliance_matrix")?;
    // 游标窗口（S-4）：fp 绑定 工具名+project_id+过滤参数——跨过滤器游标 fail-closed 拒
    let fp = cursor::fingerprint(&[
        Some("get_compliance_matrix"),
        Some(project_id.as_str()),
        filter_category.as_deref(),
        filter_status.as_deref(),
    ]);
    let window = cursor::resolve(&args, "get_compliance_matrix", &fp)?;

    // ---- 项目路由（只读工具：无状态机裁决——契约 §4 任何阶段可调）
    route_project(svc, &project_id, "get_compliance_matrix")?;

    // ---- 过滤（先过滤后分页——total 是过滤后总数）
    // 矩阵派生真源（V3-2）：证据列接 T-1 绑定真值，状态接 T-2 四态派生；
    // 只读工具零状态副作用：matrix_view 派生不落库不 bump（与 build_matrix 同源）。
    // 反查索引（一次建表——12k 行量级禁逐行 find 的 O(N²)）。
    let view = svc.store().matrix_view();
    let row_by_req: HashMap<&str, &bid_domain::matrix::MatrixRow> =
        view.iter().map(|m| (m.requirement_id.as_str(), m)).collect();
    let reqs = svc.store().requirements();
    let mut matched: Vec<usize> = Vec::with_capacity(reqs.len());
    for (i, r) in reqs.iter().enumerate() {
        if let Some(c) = &filter_category {
            if category_key(&r.category) != c && r.category != *c {
                continue;
            }
        }
        // T-2 四态（派生：override > 证据+章节）；旧 B5 字面量 unanswered → NOT_RESPONDED
        let status = row_by_req.get(r.id.as_str()).map(|m| m.status.as_str()).unwrap_or("NOT_RESPONDED");
        if let Some(s) = &filter_status {
            if status != s {
                continue;
            }
        }
        matched.push(i);
    }
    let total_rows = matched.len();

    // ---- 切片（S-4 游标窗口；offset 越界 → E_UNPROCESSABLE fail-closed）
    let page = cursor::slice(&window, &fp, total_rows)?;

    // ---- 本页行组装（仅窗口内的行构建 JSON——大结果不付全量序列化成本）
    let rows: Vec<Value> = matched[page.start..page.end].iter().map(|&i| {
        let r = &reqs[i];
        let row = row_by_req.get(r.id.as_str());
        let status = row.map(|m| m.status.as_str()).unwrap_or("NOT_RESPONDED");
        let anchor = r.anchors.first();
        // 契约 §4 evidence 列接真值（T-1 绑定 + 台账状态/有效期随行——过期证据
        // 绑定已被 matrix_view 过滤，不进响应面）
        let evidence = match row {
            Some(m) => bid_domain::matrix::evidence_column(svc.store().evidences(), &m.evidence_ids),
            None => vec![],
        };
        json!({
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
        })
    }).collect();

    let mut warnings: Vec<String> = Vec::new();
    if svc.store().requirements().is_empty() {
        warnings.push("EMPTY_MATRIX: 条款集为空（先 extract_requirements）".to_string());
    }

    Ok(json!({
        "rows": rows,
        "total": total_rows,
        "limit": window.limit,
        "cursor": page.cursor,
        "next_cursor": page.next_cursor,
        "counts": {
            "total": total_rows,
            "returned": page.end - page.start,
            // page 兼容回显（信息性）：含 offset 的 1 起页号——cursor 路径下
            // offset 未必页对齐，此值仅作旧调用方对照，不构成分页依据
            "page": window.offset / window.limit + 1,
            "page_size": window.limit,
        },
        "warnings": warnings,
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

// ================================================================ list_evidence / list_tasks（契约 §4-A/§4-B，只读）

/// 证据类别过滤值解析（双形态口径——与矩阵 category 过滤一致）：
/// 机器形态 snake_case（serde 单一真源）或中文展示名（GT 夹具口径）。
fn parse_evidence_kind(s: &str) -> Result<EvidenceKind, ToolError> {
    if let Ok(k) = serde_json::from_value::<EvidenceKind>(Value::String(s.to_string())) {
        return Ok(k);
    }
    for k in [
        EvidenceKind::Qualification, EvidenceKind::Performance, EvidenceKind::Personnel,
        EvidenceKind::Certificate, EvidenceKind::ProductDoc, EvidenceKind::SolutionModule,
        EvidenceKind::Contract,
    ] {
        if k.as_str() == s {
            return Ok(k);
        }
    }
    Err(ToolError::EUnprocessable {
        what: format!(
            "list_evidence: filters.kind 非法：{s}（合法值：qualification|performance|personnel|certificate|product_doc|solution_module|contract 或对应中文展示名）"
        ),
    })
}

/// 证据状态过滤值解析（单一真源：serde SCREAMING_SNAKE_CASE 面）。
fn parse_evidence_status(s: &str) -> Result<EvidenceStatus, ToolError> {
    serde_json::from_value::<EvidenceStatus>(Value::String(s.to_string())).map_err(|_| {
        ToolError::EUnprocessable {
            what: format!("list_evidence: filters.status 非法：{s}（合法值：ACTIVE|EXPIRING|EXPIRED|SUPERSEDED）"),
        }
    })
}

/// 证据类别的机器形态（snake_case——出参 items[].kind 与过滤入参同形，
/// 调用方可原样回填过滤器）。
fn kind_machine(k: EvidenceKind) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| k.as_str().to_string())
}

/// `list_evidence` —— 契约 §4-A（S-4 新增只读工具，无状态机裁决）。
///
/// 证据台账列表（T-1 Evidence）：`{ project_id, filters:{kind?, status?},
/// cursor?, limit? }` → `{ items, total, limit, cursor, next_cursor }`。
/// 行序 = **id 字典序**（EV-xxxx 单序列签发、全库唯一——确定全序，
/// 分页稳定前提）。10,000 条量级不允许一次全返回（S-4 原则）。
fn list_evidence(svc: &mut BidService, args: Value) -> ToolResult {
    // ---- 参数校验
    let project_id = req_str(&args, "project_id", "list_evidence")?;
    let filters = filters_of(&args, "list_evidence")?;
    let f_kind = filter_str(filters, "kind", "list_evidence")?;
    let f_status = filter_str(filters, "status", "list_evidence")?;
    let kind = match &f_kind {
        Some(k) => Some(parse_evidence_kind(k)?),
        None => None,
    };
    let status = match &f_status {
        Some(s) => Some(parse_evidence_status(s)?),
        None => None,
    };
    let fp = cursor::fingerprint(&[
        Some("list_evidence"),
        Some(project_id.as_str()),
        f_kind.as_deref(),
        f_status.as_deref(),
    ]);
    let window = cursor::resolve(&args, "list_evidence", &fp)?;

    // ---- 项目路由（只读：任何阶段可调）
    route_project(svc, &project_id, "list_evidence")?;

    // ---- 过滤（先过滤后分页）+ id 字典序（确定全序）
    let mut matched: Vec<&bid_domain::evidence::Evidence> = svc.store().evidences().iter()
        .filter(|e| kind.map_or(true, |k| e.kind == k))
        .filter(|e| status.map_or(true, |st| e.status == st))
        .collect();
    matched.sort_by(|a, b| a.id.cmp(&b.id));
    let total = matched.len();

    // ---- 切片 + 行组装（仅窗口内构建）
    let page = cursor::slice(&window, &fp, total)?;
    let items: Vec<Value> = matched[page.start..page.end].iter().map(|e| json!({
        "id": e.id,
        "kind": kind_machine(e.kind),
        "title": e.title,
        "status": e.status.as_str(),
        "subject_name": e.subject_name,
        "valid_until": e.valid_until,
    })).collect();

    Ok(json!({
        "items": items,
        "total": total,
        "limit": window.limit,
        "cursor": page.cursor,
        "next_cursor": page.next_cursor,
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

/// 任务状态过滤值解析（单一真源：serde SCREAMING_SNAKE_CASE 面）。
fn parse_task_status(s: &str) -> Result<bid_domain::assignment::TaskStatus, ToolError> {
    serde_json::from_value::<bid_domain::assignment::TaskStatus>(Value::String(s.to_string()))
        .map_err(|_| ToolError::EUnprocessable {
            what: format!("list_tasks: filters.status 非法：{s}（合法值：ASSIGNED|IN_PROGRESS|DONE）"),
        })
}

/// `list_tasks` —— 契约 §4-B（S-4 新增只读工具，无状态机裁决）。
///
/// 任务分工列表（T-6 Assignment）：`{ project_id, filters:{owner?, status?},
/// cursor?, limit? }` → `{ items, total, limit, cursor, next_cursor }`。
/// 行序 = **target_id 字典序**（DomainStore::list_tasks 的 BTree 语义——
/// 双过滤 + 确定序，R-15 可重放口径）。
fn list_tasks(svc: &mut BidService, args: Value) -> ToolResult {
    // ---- 参数校验
    let project_id = req_str(&args, "project_id", "list_tasks")?;
    let filters = filters_of(&args, "list_tasks")?;
    let f_owner = filter_str(filters, "owner", "list_tasks")?;
    let f_status = filter_str(filters, "status", "list_tasks")?;
    let status = match &f_status {
        Some(s) => Some(parse_task_status(s)?),
        None => None,
    };
    let fp = cursor::fingerprint(&[
        Some("list_tasks"),
        Some(project_id.as_str()),
        f_owner.as_deref(),
        f_status.as_deref(),
    ]);
    let window = cursor::resolve(&args, "list_tasks", &fp)?;

    // ---- 项目路由（只读：任何阶段可调）
    route_project(svc, &project_id, "list_tasks")?;

    // ---- 过滤（domain 双过滤 + target_id 字典序）+ 切片 + 行组装
    let matched = svc.store().list_tasks(f_owner.as_deref(), status);
    let total = matched.len();
    let page = cursor::slice(&window, &fp, total)?;
    let items: Vec<Value> = matched[page.start..page.end].iter().map(|a| json!({
        "target_id": a.target_id,
        "owner": a.owner,
        "deadline": a.deadline,
        "status": a.status.as_str(),
    })).collect();

    Ok(json!({
        "items": items,
        "total": total,
        "limit": window.limit,
        "cursor": page.cursor,
        "next_cursor": page.next_cursor,
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}


// ---------------------------------------------------------------- save_version

/// `save_version { file_path, message? }` —— AI 主动存版本快照（issue #42 阶段 10）。
///
/// 与桌面壳「改动面板 / 30s 自动快照」写同一份 `.mang/history`（mang-snap 单一真源），
/// kind=ai 在面板显示 AI 徽章。工作区根 = 从 file_path 向上找最近的 `.mang` 目录
/// （桌面壳建过就在），找不到用文件所在目录。
fn save_version(args: Value) -> ToolResult {
    let file_path = req_str(&args, "file_path", "save_version")?;
    let message = opt_str(&args, "message", "save_version")?.unwrap_or_default();
    let message = message.trim();
    if message.is_empty() {
        return Err(ToolError::EUnprocessable {
            what: "save_version: message 不能为空（版本说明是版本列表的核心信息——skill 纪律要求写明完成了什么）".to_string(),
        });
    }
    let path = std::path::PathBuf::from(&file_path);
    if !path.is_absolute() {
        return Err(ToolError::EUnprocessable {
            what: format!("save_version: file_path 必须是绝对路径（收到 {file_path}）"),
        });
    }
    if !mang_snap::is_snapshotable(&path) {
        return Err(ToolError::EUnprocessable {
            what: "save_version: 只支持文本文件（.md/.txt/.markdown）——源头是 md，二进制不做版本（issue #42 §八）".to_string(),
        });
    }
    if !path.is_file() {
        return Err(ToolError::ENotFound {
            what: format!("save_version: 文件不存在（{file_path}）"),
        });
    }
    let root = mang_snap::find_root(&path);
    // mang-snap 失败码（empty/too_large/denied/io）都是调用方可理解的面——
    // 契约枚举无 E_INTERNAL，统一 E_UNPROCESSABLE 并在 what 里带核心层码位
    let saved = mang_snap::save(&root, &path, mang_snap::Kind::Ai, message).map_err(|e| ToolError::EUnprocessable {
        what: format!("save_version: {}（snap:{})", e.message, e.code),
    })?;
    Ok(json!({
        "ok": true,
        "file": saved.file,
        "ts": saved.ts,
        "snapshot": saved.snapshot,
        "bytes": saved.bytes,
        "message": message,
        "kind": "ai",
        "workspace": root.display().to_string(),
        "hint": "版本已存入 .mang/history；桌面壳「改动」面板可见（AI 徽章），支持预览/对比/回滚",
    }))
}

// ---------------------------------------------------------------- §6 draft_response_section（#59 阶段 1）

/// `draft_response_section` —— 单章起草（契约 §6 + Lead #59 重排指令）。
///
/// 壳层职责（薄壳纪律）：参数校验 → 项目路由（E_NOTFOUND）→ 状态机裁决
/// （guard_tool——MatrixReady+，E_STATE）→ 乐观并发（E_CONFLICT）→ 幂等
/// 前置（(section_id, instructions_hash, content_version) 命中 → 直返首次
/// 结果）→ 转发 `BidService::draft_section` → 契约 §6 out 组装。
///
/// 生成器：**mock 确定性生成**（无 API key 的 e2e 面——Lead「无 key 时用
/// mock，否则 e2e 跑不了」）。真实 AI 面经 `SectionGenerator` trait 注入
/// （后续独立 PR 接 LLM 通路），本壳不感知生成器差异。
fn draft_response_section(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "draft_response_section";
    // ---- 参数校验（契约 §6 in）
    let project_id = req_str(&args, "project_id", TOOL)?;
    let section_id = req_str(&args, "section_id", TOOL)?;
    let instructions = opt_str(&args, "instructions", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;
    // source_requirements / evidence_ids：字符串数组（缺省 = 章节绑定集 / 矩阵绑定集）
    let req_ids = opt_str_array(&args, "source_requirements", TOOL)?;
    let ev_ids = opt_str_array(&args, "evidence_ids", TOOL)?;

    // ---- 项目路由 + 状态机裁决 + 乐观并发
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!("project 不存在：{project_id}（当前服务态项目：{}）", proj.id),
        });
    }
    svc.store().guard_tool(TOOL).map_err(ToolError::from)?;
    let version = svc.store().check_version(project_version).map_err(ToolError::from)?;

    // ---- 前置：章节树必须已建（无纲即无起草对象——契约 §6 前置的本地化）
    if svc.store().sections().is_empty() {
        return Err(ToolError::EState {
            what: format!("{} 态无章节树——先经 store 推导大纲（derive_and_set_outline）再起草", proj.status.as_str()),
        });
    }

    // ---- 幂等前置：(section_id, instructions_hash, content_version) 命中 → 直返首次结果
    let ikey = format!("draft:{section_id}:{}", instructions_hash(instructions.as_deref()));
    if let Some(first) = svc.store().idempotent_hit(&ikey).cloned() {
        // 重放不带 project_version 检查面（version 已在上面对账）——直返
        let mut out = first;
        if let Some(o) = out.as_object_mut() {
            o.insert("deduplicated".into(), json!(true));
        }
        return Ok(out);
    }

    // ---- 转发业务（bid-tools service::draft_section——生成/回查/CAS/落盘/推进）
    let mut gen = bid_tools::draft::MockGenerator;
    let outcome = svc.draft_section(
        &section_id,
        req_ids.as_deref(),
        ev_ids.as_deref(),
        instructions.as_deref(),
        &mut gen,
    ).map_err(ToolError::from)?;

    // ---- 契约 §6 out 组装
    let out = json!({
        "section_id": outcome.section_id,
        "content_ref": format!("DOC-{}-v{}", section_id, outcome.attempt),
        "version": outcome.attempt,
        "stats": {
            "words": outcome.chars,
            "citations": outcome.citations.len(),
            "cited_ids": outcome.citations,
            "unanchored_claims": outcome.unanchored_claims,
        },
        "section_status": "DRAFTED",
        "project_status": svc.store().project().map(|p| p.status.as_str()).unwrap_or("UNKNOWN"),
        "project_version": svc.store().content_version(),
        "warnings": outcome.warnings,
        "request_id": next_request_id(),
    });
    // 幂等登记（重放面——content_version 已含本次写入的 bump，重放不重执行）
    let _ = version;
    svc.store_mut().record_idempotent(ikey, out.clone());
    Ok(out)
}

/// 可选字符串数组参数（缺省/null → None；非数组或含非字符串 → 拒）。
fn opt_str_array(v: &Value, key: &str, at: &str) -> Result<Option<Vec<String>>, ToolError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => {
            let mut out = Vec::with_capacity(a.len());
            for item in a {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => return Err(ToolError::EUnprocessable {
                        what: format!("{at}: {key} 须为字符串数组"),
                    }),
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err(ToolError::EUnprocessable {
            what: format!("{at}: {key} 须为字符串数组"),
        }),
    }
}

/// instructions 的稳定哈希（幂等键素材——sha256 前 16 位十六进制）。
fn instructions_hash(s: Option<&str>) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let mut h = Sha256::new();
    h.update(s.unwrap_or("").as_bytes());
    let d = h.finalize();
    let mut hex = String::with_capacity(16);
    for b in &d[..8] {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

// ---------------------------------------------------------------- §8 export_bid_document（#59 阶段 2）

/// `export_bid_document` —— 导出（契约 §8 + Lead #59 阶段 2 指令）。
///
/// 壳层职责：参数校验 → 项目路由 → 状态机裁决（Drafting+ / EXPORTED——
/// 导出的硬闸门在服务层**双前置检查**，见 tools 分派注释）→ 乐观并发 →
/// 幂等前置 → 转发 `BidService::export_document` → 契约 §8 out 组装。
///
/// format：`"docx"`（PDF 是阶段 7 的 LibreOffice 通道——本面诚实拒，不产
/// 半吊子）。暗标：`profile.dark_bid` + `profile.bidder_name`（公司名 →
/// 「投标人」；bidder 缺失响亮 warning 不静默）。
fn export_bid_document(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "export_bid_document";
    // ---- 参数校验（契约 §8 in）
    let project_id = req_str(&args, "project_id", TOOL)?;
    let format = req_str(&args, "format", TOOL)?;
    let output_path = req_str(&args, "output_path", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;
    // profile：{ dark_bid?: bool, bidder_name?: string, template? }
    let profile = args.get("profile").filter(|v| v.is_object() || v.is_null()).cloned();
    let dark_bid = profile.as_ref()
        .and_then(|p| p.get("dark_bid")).and_then(Value::as_bool).unwrap_or(false);
    let bidder_name = profile.as_ref()
        .and_then(|p| p.get("bidder_name")).and_then(Value::as_str).map(str::to_string);
    // options：tracked_changes / watermark（阶段 7 面占位——在场即告警未支持）
    let mut warnings_pre: Vec<String> = Vec::new();
    if let Some(opts) = args.get("options").filter(|v| v.is_object()) {
        if opts.get("tracked_changes").and_then(Value::as_bool).unwrap_or(false) {
            warnings_pre.push("options.tracked_changes 暂不支持（阶段 7 面）——按普通模式导出".into());
        }
        if opts.get("watermark").is_some() {
            warnings_pre.push("options.watermark 暂不支持（阶段 7 PDF 通道面）——按无水印导出".into());
        }
    }

    if format != "docx" {
        return Err(ToolError::EUnprocessable {
            what: format!("format={format} 暂不支持（#59 阶段 2 仅 docx——PDF 是阶段 7 的 LibreOffice headless 通道，届时放开）"),
        });
    }
    // 输出路径安全面：必须落在工作区内（防 ../ 逃逸——导出产物不该写任意盘位）
    let out = std::path::PathBuf::from(&output_path);
    if out.is_absolute() || out.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(ToolError::EUnprocessable {
            what: format!("output_path 须为工作区相对路径（不得绝对路径/含 ..）：{output_path}"),
        });
    }

    // ---- 项目路由 + 状态机裁决 + 乐观并发
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!("project 不存在：{project_id}（当前服务态项目：{}）", proj.id),
        });
    }
    svc.store().guard_tool(TOOL).map_err(ToolError::from)?;
    svc.store().check_version(project_version).map_err(ToolError::from)?;

    // ---- 幂等前置：(project_id, content_version, format, dark_bid, output_path)
    //      相同 → 直返首次结果（产物 sha 已在首次导出对账；重放零 IO）
    let ikey = format!(
        "export:{project_id}:{}:{format}:{}:{output_path}",
        svc.store().content_version(),
        dark_bid,
    );
    if let Some(first) = svc.store().idempotent_hit(&ikey).cloned() {
        let mut out = first;
        if let Some(o) = out.as_object_mut() {
            o.insert("deduplicated".into(), json!(true));
        }
        return Ok(out);
    }

    // ---- 转发业务（服务层：双前置门 + format-spec + 导出 + 推进 EXPORTED）
    if dark_bid && bidder_name.as_deref().map(str::trim).unwrap_or("").is_empty() {
        warnings_pre.push(
            "暗标模式但 profile.bidder_name 缺失——正文未做公司名脱敏（补 bidder_name 后重导）".into(),
        );
    }
    let anonymize = if dark_bid { bidder_name.as_deref() } else { None };
    let ws = svc.workspace_dir().ok_or_else(|| ToolError::EUnprocessable {
        what: "导出需要持久化服务态（BidService::open 建立的工作区）——纯内存态无章内容可导".into(),
    })?;
    let abs_out = ws.join(&out);
    if let Some(parent) = abs_out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ToolError::EUnprocessable {
            what: format!("输出目录创建失败（{}）: {e}", parent.display()),
        })?;
    }
    let outcome = svc.export_document(&format, anonymize, &abs_out).map_err(ToolError::from)?;

    // ---- 契约 §8 out 组装
    let bytes = std::fs::metadata(&abs_out).map(|m| m.len()).unwrap_or(0);
    let mut warnings = warnings_pre;
    warnings.extend(outcome.warnings.iter().cloned());
    let out = json!({
        "artifact": {
            "url": abs_out.display().to_string(),
            "sha256": outcome.report.output_sha256,
            "size": bytes,
        },
        "chapters_written": outcome.report.chapters_written,
        "total_chars": outcome.report.total_chars,
        "toc_field_present": outcome.report.toc_field_present,
        "content_version_snapshot": svc.store().content_version(),
        "project_status": svc.store().project().map(|p| p.status.as_str()).unwrap_or("UNKNOWN"),
        "precheck": { "blockers": 0, "waived": [] },  // 双前置门已过（失败路径在 Err 面）
        "warnings": warnings,
        "request_id": next_request_id(),
    });
    svc.store_mut().record_idempotent(ikey, out.clone());
    Ok(out)
}

// ---------------------------------------------------------------- §7 validate_bid_document（#59 阶段 3）

/// `validate_bid_document` —— 双层校验（契约 §7 + Lead #59 阶段 3 指令）。
///
/// 壳层职责：参数校验 → 路由 → guard（Drafting+）→ 乐观并发 → 转发 →
/// 契约 §7 out（findings + counts）。
///
/// 输出形状对齐 Lead 指令的 `{ok, errors, warnings}`：
/// - `ok` = blocker == 0；
/// - `errors` = BLOCKER + MAJOR 面（废标级 + 重大缺陷——必须处理）；
/// - `warnings` = MINOR 面（告知性）；
/// - `findings`（契约原字段）= 全量清单（调用方按 rule_code 分支）。
fn validate_bid_document(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "validate_bid_document";
    // ---- 参数校验（契约 §7 in）
    let project_id = req_str(&args, "project_id", TOOL)?;
    let scope = opt_str(&args, "scope", TOOL)?.unwrap_or_else(|| "all".into());
    if scope != "all" && scope != "sections" {
        return Err(ToolError::EUnprocessable {
            what: format!("scope={scope} 非法（all | sections）"),
        });
    }
    let project_version = opt_u64(&args, "project_version", TOOL)?;
    // rules_profile：standard | dark_bid（暗标模式额外扫禁字清单——阶段 7 词
    // 表面；在场先告警未生效，不静默吞）
    let profile = opt_str(&args, "rules_profile", TOOL)?.unwrap_or_else(|| "standard".into());
    let mut pre_warnings: Vec<String> = Vec::new();
    if profile == "dark_bid" {
        pre_warnings.push("rules_profile=dark_bid 的暗标禁字清单扫描在阶段 7 词表面——本轮按 standard 规则集执行".into());
    } else if profile != "standard" {
        return Err(ToolError::EUnprocessable {
            what: format!("rules_profile={profile} 非法（standard | dark_bid）"),
        });
    }
    // scope=sections 时的 section_ids
    let section_ids = opt_str_array(&args, "section_ids", TOOL)?;
    let scope_sections: Option<Vec<String>> = match scope.as_str() {
        "sections" => Some(section_ids.ok_or_else(|| ToolError::EUnprocessable {
            what: "scope=sections 必须给 section_ids 数组".to_string(),
        })?),
        _ => None,
    };

    // ---- 路由 + guard + 乐观并发
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!("project 不存在：{project_id}（当前服务态项目：{}）", proj.id),
        });
    }
    svc.store().guard_tool(TOOL).map_err(ToolError::from)?;
    svc.store().check_version(project_version).map_err(ToolError::from)?;

    // ---- 前置：章节树必须已建（无纲无校验对象）
    if svc.store().sections().is_empty() {
        return Err(ToolError::EState {
            what: format!("{} 态无章节树——先建纲再校验", proj.status.as_str()),
        });
    }

    // ---- 转发业务（纯只读诊断 + 推进 VALIDATING）
    let report = svc.validate_document(scope_sections.as_deref()).map_err(ToolError::from)?;

    // ---- 契约 §7 out（Lead 形状：ok + errors + warnings + findings + counts）
    let errors: Vec<Value> = report.findings.iter()
        .filter(|f| f.severity != "MINOR")
        .map(|f| json!({
            "rule_code": f.rule_code,
            "severity": f.severity,
            "section_id": f.section_id,
            "requirement_id": f.requirement_id,
            "message": f.message,
        }))
        .collect();
    let mut warnings = pre_warnings;
    warnings.extend(report.findings.iter().filter(|f| f.severity == "MINOR").map(|f| f.message.clone()));
    let out = json!({
        "ok": report.ok,
        "errors": errors,
        "warnings": warnings,
        "findings": report.findings.iter().map(|f| json!({
            "rule_code": f.rule_code,
            "kind": if f.rule_code.starts_with("M_") { "MODEL" } else { "DETERMINISTIC" },
            "severity": f.severity,
            "section_id": f.section_id,
            "requirement_id": f.requirement_id,
            "message": f.message,
        })).collect::<Vec<_>>(),
        "counts": {
            "blocker": report.counts.blocker,
            "major": report.counts.major,
            "minor": report.counts.minor,
            "deterministic": report.counts.deterministic,
            "model": report.counts.model,
        },
        "sections_checked": report.sections_checked,
        "project_status": svc.store().project().map(|p| p.status.as_str()).unwrap_or("UNKNOWN"),
        "request_id": next_request_id(),
    });
    Ok(out)
}
