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
        // ---- #68 阶段 2：生成质量覆盖度检查（只读诊断 + 报告落盘）----
        "check_bid_quality" => check_bid_quality(svc, args),
        // ---- #80 第三批 MCP 补齐（G1/G2/G3——外部 Agent 纯 MCP 驱动全链）----
        "build_outline" => build_outline(svc, args),
        "register_evidence" => register_evidence(svc, args),
        "bind_evidence" => bind_evidence(svc, args),
        "record_review_gate" => record_review_gate(svc, args),
        // ---- #184：补遗分级阻塞的逐件人工确认（L3 伴生闸）----
        "record_addendum_waiver" => record_addendum_waiver(svc, args),
        // ---- #100 任务 3：B1/B2 接线（能得几分、该报多少价）----
        // score_price 是纯计算器（无 project 路由/状态门——报价推演面）；
        // score_section / score_document 走既有 route_guard（只读诊断）。
        "score_price" => score_price(svc, args),
        "score_section" => score_section(svc, args),
        "score_document" => score_document(svc, args),
        // ---- #145 任务 1：双通道证据检索（契约 §5——朴素实现，真向量库见 M2-b）----
        "search_evidence" => search_evidence(svc, args),
        other => Err(ToolError::ENotFound {
            what: format!("未接入的工具 {other}"),
        }),
    }
}

// ---------------------------------------------------------------- 参数校验助手

/// 契约 §2 `files[].kind` 枚举（MCP 壳校验面；服务层不重复——运行器自有
/// 类别如「证据」不受此限，见 service.rs 分层说明）。
const TENDER_FILE_KINDS: [&str; 7] = [
    "招标文件",
    "附件",
    "图纸",
    "补遗",
    "答疑",
    "评分办法",
    "格式模板",
];

fn req_str(v: &Value, key: &str, at: &str) -> Result<String, ToolError> {
    let s = v
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{at}: {key} 缺失或非字符串"),
        })?;
    if s.trim().is_empty() {
        return Err(ToolError::EUnprocessable {
            what: format!("{at}: {key} 不得为空白"),
        });
    }
    Ok(s.to_string())
}

fn opt_str(v: &Value, key: &str, at: &str) -> Result<Option<String>, ToolError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ToolError::EUnprocessable {
            what: format!("{at}: {key} 须为字符串"),
        }),
    }
}

fn opt_u64(v: &Value, key: &str, at: &str) -> Result<Option<u64>, ToolError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{at}: {key} 须为非负整数"),
            }),
        Some(_) => Err(ToolError::EUnprocessable {
            what: format!("{at}: {key} 须为非负整数"),
        }),
    }
}

/// 可选数值参数（#100 任务 3——score_price/score_document 的报价与规格面）。
fn opt_f64(v: &Value, key: &str, at: &str) -> Result<Option<f64>, ToolError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_f64()
            .map(Some)
            .ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{at}: {key} 须为数值"),
            }),
        Some(_) => Err(ToolError::EUnprocessable {
            what: format!("{at}: {key} 须为数值"),
        }),
    }
}

/// 可选数值数组（others 竞争报价清单——逐元素校验，含非数值即拒）。
fn opt_f64_array(v: &Value, key: &str, at: &str) -> Result<Option<Vec<f64>>, ToolError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => a
            .iter()
            .enumerate()
            .map(|(i, x)| {
                x.as_f64().ok_or_else(|| ToolError::EUnprocessable {
                    what: format!("{at}: {key}[{i}] 须为数值"),
                })
            })
            .collect::<Result<Vec<f64>, _>>()
            .map(Some),
        Some(_) => Err(ToolError::EUnprocessable {
            what: format!("{at}: {key} 须为数值数组"),
        }),
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
            what: format!(
                "project 不存在：{project_id}（当前服务态项目：{}）——{tool}",
                proj.id
            ),
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
        None | Some(Value::Null) => svc.store().project().map_err(ToolError::from)?.status,
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: "project_status 须为字符串".into(),
            });
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
        svc.store()
            .guard_tool("create_bid_project")
            .map_err(ToolError::from)?;
    }
    // 转发（领域：external_key 幂等创建；tender_notice_ref 即招标公告编号 → code 位；
    // T-8：经服务层包装——持久化在场时写后自动落盘）
    let (_, mut out) = svc
        .create_project(&name, &notice_ref.unwrap_or_default(), &mode, &external_key)
        .map_err(ToolError::from)?;
    let mut warnings = Vec::new();
    if deadline < today_ymd() {
        warnings.push(format!(
            "BID_DEADLINE_PAST: 投标截止 {bid_deadline} 已过（契约 §1：创建即警告，不阻断）"
        ));
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
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
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
    // ---- #183 围栏（壳层半边）：无受信根 → 响亮拒（指路 BID_WS）；
    //      有根 → 逐件 resolve_read（工作区相对 / 围栏内绝对；`..`/越界/
    //      symlink 段 → E_UNPROCESSABLE，先于项目路由与状态机——零副作用）。
    //      服务层 import 持根复验（双保险），见 service.rs。
    let fs_roots = svc.fs_roots();
    if fs_roots.is_empty() {
        return Err(ToolError::EUnprocessable {
            what: "导入需要工作区（设 BID_WS 环境变量，或用 BidService::open 打开持久化工作区，或 BID_FS_ROOT 声明受信导入根）".into(),
        });
    }
    let mut specs = Vec::with_capacity(files.len());
    for (i, f) in files.iter().enumerate() {
        let at = format!("files[{i}]");
        let upload_ref = req_str(f, "upload_ref", &at)?;
        let filename = req_str(f, "filename", &at)?;
        let kind = match opt_str(f, "kind", &at)? {
            Some(k) if TENDER_FILE_KINDS.contains(&k.as_str()) => k,
            Some(k) => {
                return Err(ToolError::EUnprocessable {
                    what: format!(
                        "{at}: kind 非法：{k}（合法值：{}）",
                        TENDER_FILE_KINDS.join("|")
                    ),
                });
            }
            None => "附件".to_string(), // 契约 §2 kind 可选——缺省归通用附件
        };
        let expected_pages = opt_u64(f, "expected_pages", &at)?;
        // 围栏解析：相对 = 围栏根锚定（不再按进程 CWD 解析——那是不可控行为）；
        // upload_ref 装载 canonical 绝对路径（缺失件不在围栏层抢答——R-01 逐件
        // 失败语义在服务层 sha256_file，见 fs_fence.rs 模块头）。
        let fenced = bid_tools::fs_fence::resolve_read(
            &fs_roots,
            std::path::Path::new(&upload_ref),
        )
        .map_err(ToolError::from)?;
        specs.push(ImportSpec {
            upload_ref: fenced.absolute(),
            filename,
            kind,
            expected_pages,
        });
    }

    // ---- 项目路由（E_NOTFOUND）
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!(
                "project 不存在：{project_id}（当前服务态项目：{}）",
                proj.id
            ),
        });
    }
    let status_before = proj.status;

    // ---- 状态机裁决（guard_tool——本层不自行判断）
    svc.store()
        .guard_tool("import_tender_files")
        .map_err(ToolError::from)?;
    // ---- 乐观并发（project_version 缺省 = 内部步骤语义，跳过检查）
    svc.store()
        .check_version(project_version)
        .map_err(ToolError::from)?;

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
    let status_after = svc
        .store()
        .project()
        .map(|p| p.status.as_str())
        .unwrap_or("DRAFT");
    let mut warnings = Vec::new();
    if report.new_records > 0 && status_before != ProjectStatus::Draft {
        // 契约 §2：非 DRAFT 追加新文件 → 允许但标 dirty（纯 no-op 重放不标——
        // content_version 未动，无物可脏；dirty 的并发语义即版本推进）
        warnings.push(format!(
            "DIRTY_APPEND: 项目已过 DRAFT（当前 {}）追加新文件——下游抽取/矩阵面需重算（契约 §2 dirty 波及）",
            status_before.as_str()
        ));
    }
    // ---- #184：补遗就绪度接线（非 Ready 清单不留在死代码里）----
    // import 响应是 Agent 看到坏补遗的第一现场（E_PARTIAL 的 what 只说
    // 「N 件导入失败」，不说补遗语义）；把逐件非 Ready 清单 + 指路告警放
    // 这里，Agent 拿 file_id 即可闭环（重传或 record_addendum_waiver）。
    // 与 draft 拒绝理由（阻塞消息）同一真源（readiness::addendum_statuses）。
    let addendum_readiness = addendum_readiness_block(svc);
    for s in addendum_readiness["not_ready"].as_array().unwrap_or(&Vec::new()) {
        if s["waived"].as_bool().unwrap_or(false) {
            continue; // 已确认件不重复告警（审计面在清单里，噪音面不在）
        }
        warnings.push(format!(
            "ADDENDUM_NOT_READY: 补遗 {} 未就绪（{}）——补遗可能改控制价/截止时间/评分办法，请修复重传或人工确认后放行（record_addendum_waiver）",
            s["name"].as_str().unwrap_or("?"),
            s["reason"].as_str().unwrap_or("未知原因"),
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
        // #184：补遗分级就绪报告（total/ready/not_ready 逐件含 waived 标记）
        "addendum_readiness": addendum_readiness,
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
                let missing: Vec<String> = ((x + 1)..y)
                    .map(|n| {
                        let mut seg = a[..a.len() - 1].to_vec();
                        seg.push(n);
                        seg.iter()
                            .map(|d| d.to_string())
                            .collect::<Vec<_>>()
                            .join(".")
                    })
                    .collect();
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
        return Err(ToolError::EUnprocessable {
            what: "options 必须为对象".into(),
        });
    }
    let project_version = opt_u64(&args, "project_version", "extract_requirements")?;

    // ---- 项目路由 + 状态机裁决（E_NOTFOUND / E_STATE）
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!(
                "project 不存在：{project_id}（当前服务态项目：{}）",
                proj.id
            ),
        });
    }
    svc.store()
        .guard_tool("extract_requirements")
        .map_err(ToolError::from)?;
    svc.store()
        .check_version(project_version)
        .map_err(ToolError::from)?;

    // ---- 前置：导入面必须有成功解析件（空抽取面 = E_STATE 语义前置）
    let parsed_empty = svc.parsed().is_empty();
    if parsed_empty {
        return Err(ToolError::EState {
            what: format!(
                "{} 态无已解析文件——先 import_tender_files",
                proj.status.as_str()
            ),
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
    let main_names: Vec<&str> = svc.parsed().iter().map(|p| p.name.as_str()).collect();
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
                        r_ad.anchors
                            .first()
                            .map(|a| a.excerpt.as_str())
                            .unwrap_or(""),
                        r_main
                            .anchors
                            .first()
                            .map(|a| a.excerpt.as_str())
                            .unwrap_or(""),
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
    let (total, added) = svc.set_requirements(reqs).map_err(ToolError::from)?;

    // ---- counts 组装
    let mut by_category: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
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
    let row_by_req: HashMap<&str, &bid_domain::matrix::MatrixRow> = view
        .iter()
        .map(|m| (m.requirement_id.as_str(), m))
        .collect();
    let reqs = svc.store().requirements();
    let mut matched: Vec<usize> = Vec::with_capacity(reqs.len());
    for (i, r) in reqs.iter().enumerate() {
        if let Some(c) = &filter_category {
            if category_key(&r.category) != c && r.category != *c {
                continue;
            }
        }
        // T-2 四态（派生：override > 证据+章节）；旧 B5 字面量 unanswered → NOT_RESPONDED
        let status = row_by_req
            .get(r.id.as_str())
            .map(|m| m.status.as_str())
            .unwrap_or("NOT_RESPONDED");
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
    let rows: Vec<Value> = matched[page.start..page.end]
        .iter()
        .map(|&i| {
            let r = &reqs[i];
            let row = row_by_req.get(r.id.as_str());
            let status = row.map(|m| m.status.as_str()).unwrap_or("NOT_RESPONDED");
            let anchor = r.anchors.first();
            // 契约 §4 evidence 列接真值（T-1 绑定 + 台账状态/有效期随行——过期证据
            // 绑定已被 matrix_view 过滤，不进响应面）
            let evidence = match row {
                Some(m) => {
                    bid_domain::matrix::evidence_column(svc.store().evidences(), &m.evidence_ids)
                }
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
        })
        .collect();

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
/// `at` 是调用方工具名（错误文案定位面——list_evidence / register_evidence 共用）。
fn parse_evidence_kind(s: &str, at: &str) -> Result<EvidenceKind, ToolError> {
    if let Ok(k) = serde_json::from_value::<EvidenceKind>(Value::String(s.to_string())) {
        return Ok(k);
    }
    for k in [
        EvidenceKind::Qualification,
        EvidenceKind::Performance,
        EvidenceKind::Personnel,
        EvidenceKind::Certificate,
        EvidenceKind::ProductDoc,
        EvidenceKind::SolutionModule,
        EvidenceKind::Contract,
    ] {
        if k.as_str() == s {
            return Ok(k);
        }
    }
    Err(ToolError::EUnprocessable {
        what: format!(
            "{at}: kind 非法：{s}（合法值：qualification|performance|personnel|certificate|product_doc|solution_module|contract 或对应中文展示名）"
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
        Some(k) => Some(parse_evidence_kind(k, "list_evidence")?),
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
    let mut matched: Vec<&bid_domain::evidence::Evidence> = svc
        .store()
        .evidences()
        .iter()
        .filter(|e| kind.map_or(true, |k| e.kind == k))
        .filter(|e| status.map_or(true, |st| e.status == st))
        .collect();
    matched.sort_by(|a, b| a.id.cmp(&b.id));
    let total = matched.len();

    // ---- 切片 + 行组装（仅窗口内构建）
    let page = cursor::slice(&window, &fp, total)?;
    let items: Vec<Value> = matched[page.start..page.end]
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "kind": kind_machine(e.kind),
                "title": e.title,
                "status": e.status.as_str(),
                "subject_name": e.subject_name,
                "valid_until": e.valid_until,
            })
        })
        .collect();

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
            what: format!(
                "list_tasks: filters.status 非法：{s}（合法值：ASSIGNED|IN_PROGRESS|DONE）"
            ),
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
    let items: Vec<Value> = matched[page.start..page.end]
        .iter()
        .map(|a| {
            json!({
                "target_id": a.target_id,
                "owner": a.owner,
                "deadline": a.deadline,
                "status": a.status.as_str(),
            })
        })
        .collect();

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

// ================================================================ search_evidence（契约 §5——双通道检索，#145 任务 1）

/// exact 通道五字段（契约 §5 enum 单一真源——B 侧 #146 inputSchema 同一口径）。
/// #147 起 cert/amount/date 三字段优先查 `Evidence.fields` 结构化面。
const SEARCH_EXACT_FIELDS: [&str; 5] = ["model", "amount", "date", "clause_no", "cert"];
/// top_k 上界（与 cursor::LIMIT_MAX 同护栏口径——检索 hits 面同属大结果列表，
/// 朴素实现以 top_k 收口；M2-b 落地时升级 cursor.rs 游标信封，见契约 §5 注）。
const SEARCH_TOPK_MAX: u64 = 1000;
/// top_k 缺省（契约未定缺省——检索场景惯例 20）。
const SEARCH_TOPK_DEFAULT: u64 = 20;

/// 解析后的 exact 子句（参数校验已完成——匹配阶段零分支错误）。
enum ExactSpec {
    /// model / clause_no：有界短语包含匹配 title + scope（值可含
    /// `.`/`-` 等分隔符——`6.3.2`、`DS-KD6900` 都是合法查询值）。
    Text(String),
    /// cert（#147）：结构化优先——`fields.cert_name` / `fields.cert_no` 有界短语
    /// 包含；无结构化字段时回落 title + scope 文本匹配（既有行为不变）。
    Cert(String),
    /// amount：`fields.amount_wan`（结构化，优先）或顶层 `amount_wan`（#117 业绩面）
    /// 数值全等。
    Amount(f64),
    /// date：#147 起——`fields.signed_at` / `fields.finished_at` 优先；
    /// 再落既有面：completed_on / valid_from / valid_until 归一后全等或 YYYY-MM 前缀。
    Date(String),
}

/// 朴素分词（#145：semantic 通道「子串/分词」——真向量库是 M2-b）：
/// 按字母/数字/CJK 连续段切 token 并小写化；查询侧丢单字噪声（「的」「a」命中一切）。
fn tokens_of(s: &str, min_len: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if cur.chars().count() >= min_len && !cur.is_empty() {
            out.push(cur.to_lowercase());
        }
        cur.clear();
    };
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            cur.push(ch);
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// 有界短语包含（exact 通道的「精确」口径）：值在文本中以**完整段**出现——
/// 前后邻都不是字母/数字（大小写不敏感）。
/// `ISO` 不命中 `ISO9001`（后邻 `9` 是字母数字）；`6.3.2` 命中「条款 6.3.2）」
/// （`）` 是天然边界）；`DS-KD6900` 命中标题同名段（`-` 在值内部不影响）。
fn phrase_hit(text: &str, value_lower: &str) -> bool {
    if value_lower.is_empty() {
        return false;
    }
    let hay = text.to_lowercase();
    let needle = value_lower.to_lowercase();
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find(needle.as_str()) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = hay[..start]
            .chars()
            .next_back()
            .map_or(true, |c| !c.is_alphanumeric());
        let after_ok = hay[end..]
            .chars()
            .next()
            .map_or(true, |c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = end.max(start + 1);
    }
    false
}

/// 日期口径归一：'.' → '-'（台账 `completed_on` 允许「2024.06」形态，#117）。
fn norm_date(s: &str) -> String {
    s.trim().replace('.', "-")
}

/// exact 通道单证据匹配（纯函数）。
///
/// #147 结构化字段优先策略（不破坏既有行为）：
/// - cert / amount：结构化命中优先；**无结构化字段**（`fields.is_empty()`）时回落
///   既有口径（cert → title+scope 文本；amount → 顶层 `Evidence.amount_wan`）。
///   有结构化字段却不命中 → 不回落（结构化记录以结构化面为准，避免「证书号查不到
///   却按标题误中」的假阳性）。
/// - date：`fields.signed_at` / `fields.finished_at` 命中即中；未中再查既有
///   completed_on / valid_from / valid_until（同一条记录可能只有后者，回落无害且
///   不改变旧快照结论）。
fn exact_hit(ev: &bid_domain::evidence::Evidence, spec: &ExactSpec) -> bool {
    let f = &ev.fields;
    match spec {
        ExactSpec::Text(v) => {
            let vl = v.trim().to_lowercase();
            !vl.is_empty() && (phrase_hit(&ev.title, &vl) || phrase_hit(&ev.scope, &vl))
        }
        ExactSpec::Cert(v) => {
            let vl = v.trim().to_lowercase();
            if vl.is_empty() {
                return false;
            }
            let structured_hit = f
                .cert_name
                .as_deref()
                .is_some_and(|s| phrase_hit(s, &vl))
                || f.cert_no.as_deref().is_some_and(|s| phrase_hit(s, &vl));
            if structured_hit {
                return true;
            }
            if !f.is_empty() {
                return false; // 结构化记录：以结构化面为准，不回落文本
            }
            phrase_hit(&ev.title, &vl) || phrase_hit(&ev.scope, &vl)
        }
        ExactSpec::Amount(v) => {
            if f.amount_wan.is_some_and(|a| a == *v) {
                return true;
            }
            if !f.is_empty() {
                return false;
            }
            ev.amount_wan.is_some_and(|a| a == *v)
        }
        ExactSpec::Date(d) => {
            // 归一（'.'→'-'）后：全等，或值为 YYYY-MM 时按月前缀命中
            // （「2024」前缀亦命中——调用方口径见 parse_exact_clause 文案）
            let ym = norm_date(d);
            let hit = |raw: &str| {
                let n = norm_date(raw);
                // 全等，或值为「前缀段」（YYYY / YYYY-MM）时按前缀命中
                // （「2024」↔「2024-06-15」；「2024-06」↔「2024-06-15」）
                n == ym
                    || (matches!(ym.len(), 4 | 7)
                        && n.len() > ym.len()
                        && n.starts_with(ym.as_str())
                        && n.as_bytes().get(ym.len()) == Some(&b'-'))
            };
            if [f.signed_at.as_deref(), f.finished_at.as_deref()]
                .into_iter()
                .flatten()
                .any(hit)
            {
                return true;
            }
            [
                ev.completed_on.as_deref(),
                ev.valid_from.as_deref(),
                ev.valid_until.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(hit)
        }
    }
}

/// 朴素 chunk 文案（pre-M2-b：整件为一块——title/scope/subject/client 直通拼接；
/// M2-b chunk 化后由真实分块文本承载）。
fn evidence_content(ev: &bid_domain::evidence::Evidence) -> String {
    [Some(ev.title.as_str()), Some(ev.scope.as_str()), Some(ev.subject_name.as_str()), ev.client.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("；")
}

/// exact 子句解析（形态校验 + 与 mode 的互斥/前置约束，fail-closed）。
fn parse_exact_clause(
    args: &Value,
    mode: &str,
) -> Result<Option<ExactSpec>, ToolError> {
    const TOOL: &str = "search_evidence";
    let Some(c) = args.get("exact").filter(|v| !v.is_null()) else {
        if mode == "exact" {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: mode=exact 必须带 exact 子句 {{field, value}}（契约 §5）"),
            });
        }
        return Ok(None);
    };
    if !c.is_object() {
        return Err(ToolError::EUnprocessable {
            what: format!("{TOOL}: exact 须为对象 {{field, value}}（收到 {c}）"),
        });
    }
    if mode == "semantic" {
        return Err(ToolError::EUnprocessable {
            what: format!("{TOOL}: mode=semantic 与 exact 子句互斥（语义检索不吃结构化字段——要字段过滤请用 hybrid 或 exact）"),
        });
    }
    let field = c
        .get("field")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{TOOL}: exact.field 必填（{SEARCH_EXACT_FIELDS:?} 五字段——契约 §5）"),
        })?;
    if !SEARCH_EXACT_FIELDS.contains(&field) {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "{TOOL}: exact.field={field} 非法（合法：{SEARCH_EXACT_FIELDS:?}——契约 §5；amount/date/cert 优先查结构化字段 fields）"
            ),
        });
    }
    let value = c
        .get("value")
        .filter(|v| !v.is_null())
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{TOOL}: exact.value 必填（与 exact.field 成对——契约 §5）"),
        })?;
    let spec = match field {
        "amount" => match value {
            Value::Number(n) => n.as_f64().map(ExactSpec::Amount).ok_or_else(|| {
                ToolError::EUnprocessable {
                    what: format!("{TOOL}: exact.value 数值不可表示为 f64（万元口径）"),
                }
            })?,
            Value::String(s) => s
                .trim()
                .parse::<f64>()
                .map(ExactSpec::Amount)
                .map_err(|_| ToolError::EUnprocessable {
                    what: format!(
                        "{TOOL}: amount 的 value 须为数值或数字字符串（收到 {s:?}——万元口径）"
                    ),
                })?,
            _ => {
                return Err(ToolError::EUnprocessable {
                    what: format!("{TOOL}: amount 的 value 须为数值或数字字符串（万元口径）"),
                })
            }
        },
        "date" => match value {
            // 归一（'.'→'-'）：fields.signed_at/finished_at 优先；既有面 completed_on
            // 允许「2024.06」形态；YYYY-MM（及 YYYY 年前缀）月前缀
            Value::String(s) if !s.trim().is_empty() => ExactSpec::Date(norm_date(s)),
            _ => {
                return Err(ToolError::EUnprocessable {
                    what: format!("{TOOL}: date 的 value 须为非空字符串（YYYY-MM-DD / YYYY-MM / YYYY——先比对 fields.signed_at、fields.finished_at，再比对 completed_on/valid_from/valid_until）"),
                })
            }
        },
        // #147：cert 走结构化面（fields.cert_name / fields.cert_no），无结构化字段时
        // 回落 title+scope 文本（见 exact_hit）
        "cert" => match value {
            Value::String(s) if !s.trim().is_empty() => ExactSpec::Cert(s.trim().to_string()),
            _ => {
                return Err(ToolError::EUnprocessable {
                    what: format!("{TOOL}: cert 的 value 须为非空字符串（比对 fields.cert_name / fields.cert_no；无结构化字段时回落 title/scope 文本匹配）"),
                })
            }
        },
        _ => match value {
            Value::String(s) if !s.trim().is_empty() => ExactSpec::Text(s.trim().to_string()),
            _ => {
                return Err(ToolError::EUnprocessable {
                    what: format!("{TOOL}: {field} 的 value 须为非空字符串（契约 §5）"),
                })
            }
        },
    };
    Ok(Some(spec))
}

/// search_evidence 的 filters 解析（契约 §5 四键——fail-closed：未知键/未支持键响亮拒，
/// 静默丢过滤器会让调用方以为查过）。
/// 返回 (source_type, valid_at=开标日, min_valid_until)。
fn parse_search_filters(
    args: &Value,
) -> Result<(Option<String>, Option<String>, Option<String>), ToolError> {
    const TOOL: &str = "search_evidence";
    let Some(f) = args.get("filters").filter(|v| !v.is_null()) else {
        return Ok((None, None, None));
    };
    let Value::Object(map) = f else {
        return Err(ToolError::EUnprocessable {
            what: format!("{TOOL}: filters 须为对象（契约 §5 四键：source_type|tags|valid_at|min_valid_until）"),
        });
    };
    for key in map.keys() {
        match key.as_str() {
            "source_type" | "tags" | "valid_at" | "min_valid_until" => {}
            other => {
                return Err(ToolError::EUnprocessable {
                    what: format!(
                        "{TOOL}: filters.{other} 不是合法过滤键（合法：source_type|tags|valid_at|min_valid_until——未知键显式拒，不静默忽略）"
                    ),
                })
            }
        }
    }
    if map.get("tags").is_some_and(|v| !v.is_null()) {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "{TOOL}: filters.tags 暂不支持——证据台账无标签面（M2-b 知识库落地后开；台账清单过滤请用 list_evidence）"
            ),
        });
    }
    let str_field = |key: &str| -> Result<Option<String>, ToolError> {
        match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
            Some(_) => Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: filters.{key} 须为非空字符串"),
            }),
        }
    };
    let source_type = str_field("source_type")?;
    let valid_at = str_field("valid_at")?;
    let min_valid_until = str_field("min_valid_until")?;
    for (key, v) in [("valid_at", &valid_at), ("min_valid_until", &min_valid_until)] {
        if let Some(d) = v {
            if !bid_domain::dates::is_valid_ymd(d) {
                return Err(ToolError::EUnprocessable {
                    what: format!("{TOOL}: filters.{key}={d:?} 非合法 ISO 日期（YYYY-MM-DD）"),
                });
            }
        }
    }
    Ok((source_type, valid_at, min_valid_until))
}

/// `search_evidence` —— 契约 §5（#145 任务 1 接线，朴素实现）。
///
/// 双通道检索：
/// - **exact 通道**：五字段（model/amount/date/clause_no/cert）结构化精确匹配
///   ——model/clause_no 为 title+scope 的**有界短语包含**（`ISO` 不命中
///   `ISO9001`；`6.3.2`/`DS-KD6900` 可命中同名段）。
///   **#147 结构化字段优先**：cert → `fields.cert_name` / `fields.cert_no` 有界
///   短语包含；amount → `fields.amount_wan`（万元）数值全等；date 归一
///   （`.`→`-`）后与 `fields.signed_at` / `fields.finished_at` 全等或月/年前缀。
///   无结构化字段（`fields` 为空）时回落既有口径：cert → title+scope 文本；
///   amount → 顶层 `Evidence.amount_wan`（#117 业绩面）；date 另查
///   completed_on/valid_from/valid_until。命中分 1.0。
/// - **semantic 通道**（朴素：子串/分词——真向量库 M2-b）：查询按字母/数字/CJK
///   连续段切 token（单字噪声丢弃），token 在证据文案（title/scope/subject/client）
///   中子串命中计分 score = 命中 token 数 / 查询 token 数。
/// - **hybrid**（缺省）：按 ev_id 合并去重，双通道同中 → match_type="both"
///   （分取 exact 的 1.0——资格类查询 exact 优先，语义覆盖率不叠加假精度）。
///
/// 硬规则（契约 §5 + #145）：
/// - `filters.valid_at`（开标日）给定时：`valid_until < 开标日` → **score×0.5 +
///   EVIDENCE_EXPIRED warning**；开标日 ≤ valid_until < 开标日+90 天 →
///   EVIDENCE_EXPIRING warning（不降权——#145 只要求过期降权）。开标日口径归
///   调用方（不给则不做时间面降权，与导出层「开标日不替调用方定」同纪律）。
/// - 排序确定性（R-15 可重放）：score DESC, ev_id ASC；`threshold`（0..=1）过滤
///   降权后分值，`top_k`（1..=1000，缺省 20）截断。
/// - 换版旧证（SUPERSEDED）不入检索面（找的是可用证据——历史版本走 list_evidence）。
///
/// 已知边界（如实）：M2-b 前无 chunk 化——chunk_id 恒 `{ev_id}#whole`、content 为
/// 台账文案直通；tags 过滤显式拒；hits 面以 top_k 收口（游标信封留 M2-b）。
fn search_evidence(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "search_evidence";
    // ---- 参数校验
    let project_id = req_str(&args, "project_id", TOOL)?;
    let query = req_str(&args, "query", TOOL)?;
    let mode = match opt_str(&args, "mode", TOOL)? {
        None => "hybrid".to_string(),
        Some(m) if matches!(m.as_str(), "hybrid" | "semantic" | "exact") => m,
        Some(m) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: mode={m:?} 非法（合法：hybrid|semantic|exact——契约 §5）"),
            })
        }
    };
    let exact_spec = parse_exact_clause(&args, &mode)?;
    let (f_source_type, f_valid_at, f_min_valid_until) = parse_search_filters(&args)?;
    let top_k = match args.get("top_k") {
        None | Some(Value::Null) => SEARCH_TOPK_DEFAULT,
        Some(Value::Number(n)) => {
            let v = n.as_u64().ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{TOOL}: top_k 须为 1..={SEARCH_TOPK_MAX} 的整数"),
            })?;
            if !(1..=SEARCH_TOPK_MAX).contains(&v) {
                return Err(ToolError::EUnprocessable {
                    what: format!(
                        "{TOOL}: top_k={v} 越界（合法 1..={SEARCH_TOPK_MAX}，缺省 {SEARCH_TOPK_DEFAULT}——显式拒绝，不静默 clamp）"
                    ),
                });
            }
            v
        }
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: top_k 须为 1..={SEARCH_TOPK_MAX} 的整数"),
            })
        }
    };
    let threshold = match args.get("threshold") {
        None | Some(Value::Null) => 0.0f64,
        Some(Value::Number(n)) => {
            let v = n.as_f64().unwrap_or(f64::NAN);
            if !(0.0..=1.0).contains(&v) {
                return Err(ToolError::EUnprocessable {
                    what: format!("{TOOL}: threshold={v} 越界（合法 0.0..=1.0——作用于降权后的分值）"),
                });
            }
            v
        }
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: threshold 须为 0.0..=1.0 的数值"),
            })
        }
    };

    // ---- 项目路由（只读：任何阶段可调；E_STATE 资料库绑定面在朴素实现下
    //      不存在独立绑定——项目路由即总闸，M2-b 接知识库时补）
    route_project(svc, &project_id, TOOL)?;

    // ---- 过滤面（先过滤后检索）
    let pool: Vec<&bid_domain::evidence::Evidence> = svc
        .store()
        .evidences()
        .iter()
        .filter(|e| e.status != EvidenceStatus::Superseded)
        .filter(|e| {
            f_source_type.as_deref().map_or(true, |st| {
                e.source_file.to_lowercase().contains(&st.to_lowercase())
            })
        })
        .filter(|e| {
            f_min_valid_until.as_deref().map_or(true, |mv| {
                // 长期证（valid_until=None）不设有效期下限——口径与台账一致
                e.valid_until.as_deref().map_or(true, |vu| norm_date(vu).as_str() >= mv)
            })
        })
        .collect();

    // ---- 双通道
    let runs_exact = mode != "semantic";
    let runs_semantic = mode != "exact";
    let query_tokens = if runs_semantic { tokens_of(&query, 2) } else { Vec::new() };
    let mut exact_idx: Vec<usize> = Vec::new();
    let mut semantic: Vec<(usize, f64)> = Vec::new();
    for (i, e) in pool.iter().enumerate() {
        if runs_exact && exact_spec.as_ref().is_some_and(|spec| exact_hit(e, spec)) {
            exact_idx.push(i);
        }
        if runs_semantic && !query_tokens.is_empty() {
            let content = evidence_content(e).to_lowercase();
            let matched = query_tokens
                .iter()
                .filter(|t| content.contains(t.as_str()))
                .count();
            if matched > 0 {
                semantic.push((i, matched as f64 / query_tokens.len() as f64));
            }
        }
    }
    let semantic_k = semantic.len();
    let exact_k = exact_idx.len();

    // ---- 合并去重（hybrid：ev_id 为键；双中 → both，分取 exact 的 1.0）
    let mut merged: Vec<(usize, f64, u8)> = exact_idx
        .iter()
        .map(|&i| (i, 1.0f64, 1u8))
        .collect();
    for &(i, s) in &semantic {
        match merged.iter_mut().find(|(j, _, _)| *j == i) {
            Some(h) => {
                h.2 = 3; // both
            }
            None => merged.push((i, s, 2)),
        }
    }

    // ---- 时间面降权 + warning（开标日由 filters.valid_at 给定才启用）
    let mut warnings: Vec<String> = Vec::new();
    if let Some(va) = &f_valid_at {
        for h in &mut merged {
            let e = pool[h.0];
            match bid_domain::evidence::validity_flip(e.valid_until.as_deref(), va) {
                Some(EvidenceStatus::Expired) => {
                    h.1 *= 0.5;
                    warnings.push(format!(
                        "EVIDENCE_EXPIRED: {}（{}）有效期至 {}，早于开标日 {va}——已降权 score×0.5（禁绑口径见 bind_evidence）",
                        e.id, e.title, e.valid_until.as_deref().unwrap_or("(无)")
                    ));
                }
                Some(EvidenceStatus::Expiring) => {
                    warnings.push(format!(
                        "EVIDENCE_EXPIRING: {}（{}）有效期至 {}，距开标日 {va} 不足 90 天——可绑但请尽快换证",
                        e.id, e.title, e.valid_until.as_deref().unwrap_or("(无)")
                    ));
                }
                _ => {}
            }
        }
    }

    // ---- 排序（score DESC, ev_id ASC——R-15 可重放）+ threshold + top_k
    merged.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| pool[a.0].id.cmp(&pool[b.0].id))
    });
    merged.retain(|(_, s, _)| *s >= threshold);
    let total = merged.len();
    let page_end = total.min(top_k as usize);
    let hits: Vec<Value> = merged[..page_end]
        .iter()
        .map(|&(i, score, mt)| {
            let e = pool[i];
            json!({
                "ev_id": e.id,
                "chunk_id": format!("{}#whole", e.id),
                "content": evidence_content(e),
                "source": {
                    "file": e.source_file,
                    "position": serde_json::to_string(&e.locator)
                        .unwrap_or_else(|_| "\"(locator 序列化失败)\"".to_string()),
                },
                "score": score,
                "match_type": match mt { 1 => "exact", 2 => "semantic", _ => "both" },
                // 台账直通附加面（pre-M2-b：调用方选证/续绑决策需要；chunk 化后由 chunk 元数据承载）
                "kind": kind_machine(e.kind),
                "title": e.title,
                "status": e.status.as_str(),
                "valid_until": e.valid_until,
            })
        })
        .collect();

    Ok(json!({
        "hits": hits,
        "total": total,
        "top_k": top_k,
        "trace": {
            "semantic_k": semantic_k,
            "exact_k": exact_k,
            "merged_by": "ev_id",
            "engine": "naive-token-v1（pre-M2-b：子串/整 token——真向量库见 M2-b）",
        },
        "warnings": warnings,
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
    let saved = mang_snap::save(&root, &path, mang_snap::Kind::Ai, message).map_err(|e| {
        ToolError::EUnprocessable {
            what: format!("save_version: {}（snap:{})", e.message, e.code),
        }
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
/// 结果；external 稿另计入 content 指纹）→ 转发 `BidService::draft_section`
/// → 契约 §6 out 组装。
///
/// 生成器（#68 阶段 1 + #100 任务 2，四态）：
/// - `auto`（缺省）：有 api_key → RealGenerator；无 → MockGenerator + 降级告警；
/// - `real`：无 key → E_UNPROCESSABLE（显式要求真实生成而配置缺失，要响亮）；
/// - `mock`：强制确定性生成（e2e/复算面——无告警，是显式选择）；
/// - `external`：正文来自 `content` 参数（外部作者手稿——子代理/人工写好
///   直注，免中继）；引擎不生成只提交，引用回查/CAS/落盘全走既有路径。
///   `content` 在场即 external 语义（与 generator=mock/real 显式互斥——两个
///   生成源打架要响亮拒）。
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
    // #68 阶段 1：生成器选择（auto = 有 key 走真实 LLM，无 key 降级 mock 并告警）
    let generator_mode = opt_generator(&args, "generator", TOOL)?;
    // #100 任务 2：外部手稿（generator=external 的正文来源）。
    // 互斥裁决前置到路由/状态机之前（参数面错误零副作用，响亮拒）
    let external_content = opt_str(&args, "content", TOOL)?;
    if let Some(c) = &external_content {
        if c.trim().is_empty() {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: content 不得为空白（外部手稿全文——空稿无提交意义）"),
            });
        }
        if matches!(generator_mode.as_deref(), Some("mock") | Some("real")) {
            return Err(ToolError::EUnprocessable {
                what: format!(
                    "{TOOL}: content（外部手稿）与 generator={} 互斥——外部稿直注请用 \
                     generator=external（或缺省 content 即 external 语义）",
                    generator_mode.as_deref().unwrap_or("?"),
                ),
            });
        }
    }
    if generator_mode.as_deref() == Some("external") && external_content.is_none() {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "{TOOL}: generator=external 需要 content 参数（外部作者的手稿全文）——\
                 无手稿请用 auto/mock/real",
            ),
        });
    }

    // ---- 项目路由 + 状态机裁决 + 乐观并发
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!(
                "project 不存在：{project_id}（当前服务态项目：{}）",
                proj.id
            ),
        });
    }
    svc.store().guard_tool(TOOL).map_err(ToolError::from)?;
    let version = svc
        .store()
        .check_version(project_version)
        .map_err(ToolError::from)?;

    // ---- 前置：章节树必须已建（无纲即无起草对象——契约 §6 前置的本地化）
    if svc.store().sections().is_empty() {
        return Err(ToolError::EState {
            what: format!(
                "{} 态无章节树——先经 store 推导大纲（derive_and_set_outline）再起草",
                proj.status.as_str()
            ),
        });
    }

    // ---- 幂等前置：(section_id, instructions_hash, content_version) 命中 → 直返首次结果
    // external 稿的正文是第一输入（同章节同指令不同稿 = 不同请求）——幂等键
    // 追加 content 指纹；同稿重提仍命中（CAS 同内容 no-op 语义一致）
    let mut ikey = format!(
        "draft:{section_id}:{}",
        instructions_hash(instructions.as_deref())
    );
    if let Some(c) = &external_content {
        ikey.push_str(&format!(":ext-{}", content_hash(c)));
    }
    if let Some(first) = svc.store().idempotent_hit(&ikey).cloned() {
        // 重放不带 project_version 检查面（version 已在上面对账）——直返
        let mut out = first;
        if let Some(o) = out.as_object_mut() {
            o.insert("deduplicated".into(), json!(true));
        }
        return Ok(out);
    }

    // ---- 转发业务（bid-tools service::draft_section——生成/回查/CAS/落盘/推进）
    // 生成器选择面（#68 阶段 1 issue §1.2 + #100 任务 2——auto/real/mock/external 四态）：
    //   auto  = 有 api_key → RealGenerator；无 → MockGenerator + 降级告警
    //   real  = 无 key → E_UNPROCESSABLE（显式要求真实生成而配置缺失，要响亮）
    //   mock  = 强制确定性生成（e2e/复算面——无告警，是显式选择）
    //   external = content 手稿直注（引擎不生成只提交——与 mock/real 提交面一致）
    let llm_cfg =
        bid_tools::llm::LlmConfig::resolve(svc.workspace_dir().as_deref()).map_err(|e| {
            ToolError::EUnprocessable {
                what: format!("LLM 配置无效（.mang/llm.json 或 BID_LLM_*）: {e}"),
            }
        })?;
    let mut gen_warnings: Vec<String> = Vec::new();
    let gen_kind;
    let mut gen: Box<dyn bid_tools::draft::SectionGenerator> =
        if let Some(content) = external_content {
            gen_kind = "external";
            gen_warnings.push(
                "external 模式：正文为外部作者手稿（引擎不生成只提交）——引用回查、\
             CAS 写入、状态推进、落盘与 mock/real 完全同路径"
                    .into(),
            );
            Box::new(bid_tools::draft::ExternalGenerator { content })
        } else {
            match generator_mode.as_deref() {
                Some("mock") => {
                    gen_kind = "mock";
                    Box::new(bid_tools::draft::MockGenerator)
                }
                Some("real") => match llm_cfg {
                    Some(cfg) => {
                        gen_kind = "real";
                        Box::new(bid_tools::llm::RealGenerator::new(cfg, svc.workspace_dir()))
                    }
                    None => return Err(ToolError::EUnprocessable {
                        what:
                            "generator=real 但未配置 api_key（.mang/llm.json 或 BID_LLM_API_KEY）\
                               ——要么配置 key，要么用 auto/mock"
                                .into(),
                    }),
                },
                _ => match llm_cfg {
                    Some(cfg) => {
                        gen_kind = "real";
                        Box::new(bid_tools::llm::RealGenerator::new(cfg, svc.workspace_dir()))
                    }
                    None => {
                        gen_kind = "mock";
                        gen_warnings.push(
                            "未配置 LLM api_key——降级 mock 生成（内容为确定性模板，非真实 AI）；\
                         配置 .mang/llm.json 或 BID_LLM_API_KEY 后走真实生成"
                                .into(),
                        );
                        Box::new(bid_tools::draft::MockGenerator)
                    }
                },
            }
        };
    let outcome = svc
        .draft_section(
            &section_id,
            req_ids.as_deref(),
            ev_ids.as_deref(),
            instructions.as_deref(),
            gen.as_mut(),
        )
        .map_err(ToolError::from)?;
    let mut warnings = outcome.warnings;
    warnings.append(&mut gen_warnings);

    // ---- 配额账面（issue：Agent 无从得知本节该写多少字——24 条 MINOR 字数
    // 偏离的根因）。给 Agent 三个数：target_chars（目标）/ written_chars（已写）
    // / remaining（还差）；外加 min_segments = 按 SEGMENT_CHARS 折算的批次数。
    //
    // 口径：target_chars 取【本节叶节点配额】（父 = 子配额和，叶才是内容单元）；
    // written_chars 取本次起草后的正文字符数（含重写后的全量）。
    const SEGMENT_CHARS: u64 = 4000; // 单次生成的安全字数量（模型上下文漂移阈值）
    let quota = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == section_id)
        .map(|n| {
            let target = u64::from(n.target_chars);
            let written = n
                .content
                .as_deref()
                .map(|c| c.chars().count() as u64)
                .unwrap_or(0);
            let remaining = target.saturating_sub(written);
            serde_json::json!({
                "target_chars": target,
                "written_chars": written,
                "remaining": remaining,
                // 按 4000 字/批折算还需要几次「续写」；已达标 = 0
                "min_segments": target.div_ceil(SEGMENT_CHARS),
                "remaining_segments": remaining.div_ceil(SEGMENT_CHARS),
                // 完成度（0-100；>100 表示超配额，Agent 应精简）
                "progress_pct": if target == 0 {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(written * 100 / target)
                },
            })
        })
        .unwrap_or(serde_json::Value::Null);
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
            // #68 阶段 1：本次实际使用的生成器（real/mock——Agent 可据此判断降级面）
            "generator": gen_kind,
            // #68 阶段 2：覆盖度自动检查（issue §2.1——未回应 REQ 清单）
            "uncovered_requirements": outcome.uncovered_requirements,
        },
        // #127 P0-2：起草指令 + 缺失回报的留档账面（与正文分离——正文渲染
        // 面零「起草指令/缺失回报」字样；Agent/人工从这里看本章起草时的
        // 指令与素材缺口，随 chapters/{id}.json 持久化）
        "quota": quota,
        "draft_note": outcome.draft_note,
        "section_status": "DRAFTED",
        "project_status": svc.store().project().map(|p| p.status.as_str()).unwrap_or("UNKNOWN"),
        "project_version": svc.store().content_version(),
        "warnings": warnings,
        "request_id": next_request_id(),
    });
    // 幂等登记（重放面——content_version 已含本次写入的 bump，重放不重执行）
    let _ = version;
    svc.store_mut().record_idempotent(ikey, out.clone());
    Ok(out)
}

// ---------------------------------------------------------------- #68 阶段 2 check_bid_quality

/// `check_bid_quality` —— 生成质量覆盖度检查（只读诊断 + 报告落盘）。
///
/// 壳层职责：参数校验 → 项目路由 → 状态机裁决（guard_tool——MatrixReady+，
/// 起草中即可查进度）→ 报告构建（bid-tools quality——覆盖度/带宽/无锚/
/// FAILED 边车）→ 落盘 `artifacts/quality-report.json`（workspace 在场）→
/// 契约 out 组装。
///
/// D2 同语义：只读不改稿不落库不推状态——报告是导数，真源在域/工作区。
/// 工作区打不开（章节文件损坏）→ **不阻断**（降级无失败明细 + warning——
/// 组装主流程会 fail-closed，诊断面不该比被诊断者更脆）。
fn check_bid_quality(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "check_bid_quality";
    let project_id = req_str(&args, "project_id", TOOL)?;

    // ---- 项目路由 + 状态机裁决
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!(
                "project 不存在：{project_id}（当前服务态项目：{}）",
                proj.id
            ),
        });
    }
    svc.store().guard_tool(TOOL).map_err(ToolError::from)?;

    // ---- 前置：章节树必须已建（无纲无检查对象）
    if svc.store().sections().is_empty() {
        return Err(ToolError::EState {
            what: format!("{} 态无章节树——先建纲再查质量", proj.status.as_str()),
        });
    }

    // ---- 报告构建（assembler 在场时并入 failed.json 失败明细）
    //
    // B3 三维度评分接线（#131 任务 3 发现的真实性缺口）：此前本工具走
    // 旧调用面 build_quality_report（ctx=None）——specificity / substance /
    // score / duplication 四面全 None，评分器只在 quality_integrate 测试里
    // 接过，MCP 生产路径从未评过（真实语料 3 件全链的质量报告 score 全
    // null 即此缺口的现场）。ctx 构造与测试同源：ProjectContext::from_store
    // + SectionScorer（derive_scoring_tree——真实评分项 REQ 驱动，无评分项
    // 的真实件 scorer 空树，评分面诚实 None 化不伪装）。
    let mut warnings: Vec<String> = Vec::new();
    let asm = svc
        .workspace_dir()
        .map(
            |dir| match bid_tools::assembly::ChapterAssembler::open(&dir) {
                Ok(a) => Some(a),
                Err(e) => {
                    warnings.push(format!("章节工作区打开失败（{}）——失败明细未并入报告", e));
                    None
                }
            },
        )
        .flatten();
    use bid_domain::scoring::{derive_scoring_tree, SectionScorer};
    use bid_tools::quality::{ProjectContext, QualityContext};
    let project_ctx = ProjectContext::from_store(svc.store());
    let scorer = SectionScorer::new(
        derive_scoring_tree(svc.store().requirements()),
        project_ctx.scorer_terms(),
    );
    let ctx = QualityContext {
        scorer: &scorer,
        project: &project_ctx,
    };
    let report =
        bid_tools::quality::build_quality_report_with(svc.store(), asm.as_ref(), Some(&ctx));

    // ---- 落盘（workspace 在场；纯内存态降级返回 + 告警）
    let artifact = match svc.workspace_dir() {
        Some(ws) => match bid_tools::quality::write_report(&report, &ws) {
            Ok(path) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                serde_json::json!({
                    "path": path.to_string_lossy(),
                    "size": size,
                })
            }
            Err(e) => {
                warnings.push(format!("质量报告落盘失败: {e}"));
                Value::Null
            }
        },
        None => {
            warnings.push("纯内存态（无持久化工作区）——报告只返未落盘".into());
            Value::Null
        }
    };

    // ---- 跨章查重摘要（#145 任务 3——Agent 不必自己遍历 sections 算）
    //
    // 口径（与 SKILL.md「跨章查重」表一致）：
    // - 严重线 0.8 取自 bid-tools 的 `DUP_SEVERE_JACCARD`（单一真源——壳层
    //   不另写常数，引擎调线时摘要跟着动）；
    // - 对按无序计一次（a.id < b.id——dup_with 是双向回填，直接遍历会双计）；
    // - 多对端取该对的最大相似度（本章对 X 可能在 jaccard/containment 两
    //   口径各一条——同一对只该计一次，取大如实反映「最像」）；
    // - 不收报告的 < 0.3 对天然不进 dup_with（报告层已过滤）。
    const DUP_PAIR_LINE: f64 = bid_tools::quality::DUP_SEVERE_JACCARD;
    let mut dup_max_similarity = 0.0f64;
    for q in &report.sections {
        for (_, sim) in &q.dup_with {
            if sim.is_finite() && *sim > dup_max_similarity {
                dup_max_similarity = *sim;
            }
        }
    }
    let mut pair_sims: std::collections::BTreeMap<(&str, &str), f64> =
        std::collections::BTreeMap::new();
    for q in &report.sections {
        for (other, sim) in &q.dup_with {
            if !sim.is_finite() {
                continue;
            }
            let (a, b) = if q.section_id.as_str() < other.as_str() {
                (q.section_id.as_str(), other.as_str())
            } else {
                (other.as_str(), q.section_id.as_str())
            };
            if a == b {
                continue; // 自反（理论不入——防御）
            }
            pair_sims
                .entry((a, b))
                .and_modify(|v| {
                    if *sim > *v {
                        *v = *sim;
                    }
                })
                .or_insert(*sim);
        }
    }
    let dup_pairs = pair_sims.values().filter(|s| **s >= DUP_PAIR_LINE).count();

    // ---- 篇幅核对（全局账：已写 vs 配额）
    //
    // 已写 = 叶节点 content 字符数之和（node.content 是正文真源——叶判定同
    // 报告层：无子者即叶）；目标 = outline 根节点配额和（outline_total_chars
    // 本身就按根求和，全树求和会双计）。
    let char_total: usize = {
        use std::collections::HashSet;
        let parents: HashSet<&str> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.as_deref()).collect();
        svc.store().sections().iter()
            .filter(|n| !parents.contains(n.id.as_str()))
            .map(|n| n.content.as_deref().map_or(0, |c| c.chars().count()))
            .sum()
    };
    let char_target = svc.store().outline_total_chars();
    // 偏离百分比：(total-target)/target*100，保留 1 位小数；target=0 → null
    // （无配额 = 无「偏离」可言，null 是诚实面，不是 0.0）
    let char_deviation_pct: Option<f64> = if char_target == 0 {
        None
    } else {
        let pct = (char_total as f64 - char_target as f64) / char_target as f64 * 100.0;
        Some((pct * 10.0).round() / 10.0)
    };

    // ---- out 组装（sections 内联——全量诊断一次到位；大文档以 artifact 为准）
    let sm = &report.summary;
    Ok(serde_json::json!({
        "project_id": report.project_id,
        "summary": {
            "total": sm.total,
            "drafted": sm.drafted,
            "failed": sm.failed,
            "pending": sm.pending,
            "uncovered_req_total": sm.uncovered_req_total,
            "out_of_band": sm.out_of_band,
            "unanchored_total": sm.unanchored_total,
            "dup_pairs": dup_pairs,
            "dup_max_similarity": dup_max_similarity,
            "char_total": char_total,
            "char_target": char_target,
            "char_deviation_pct": char_deviation_pct,
        },
        "needs_human": sm.needs_human,
        "sections": report.sections,
        "artifact": artifact,
        "warnings": warnings,
        "request_id": next_request_id(),
    }))
}

/// `generator` 参数（#68 阶段 1 + #100 任务 2——缺省 auto；非四态拒）。
// ---------------------------------------------------------------- #100 任务 3：B1/B2 接线（score_price / score_section / score_document）

/// `score_price` —— 价格分试算 + 最优报价反推（issue #100 §3.1/§3.3）。
///
/// 纯计算器：无 project 路由/状态门（报价决策的推演面——投不投、报多少，
/// 发生在起草之前也成立）。B1 [`bid_domain::pricing::PriceScoreSpec`] 只调用不改。
///
/// 参数面：
/// - `method`：字符串糖 `benchmark` | `lowest` | `average`（缺省 average——
///   synth 4.1 口径）或完整 serde 对象（`{"COMPOSITE":{…}}` /
///   `{"DISCOUNT_RATE":{"base":{"FIXED":0.04}}}`——复合法/下浮率法全参数面）；
/// - `coefficient`（benchmark/average 的下浮系数；缺省 1.0/0.95）、
///   `max_score`（缺省 30）、`budget`（控制价——benchmark 必需，B1 validate 裁决）；
/// - `deviation`：`ratio`（缺省——87 号令第五十五条口径）| `price`（需
///   `deduct_high`/`deduct_low`）| `rate`（需 `deduct`）或完整对象；
/// - `bid_price`（缺省 = 纯推荐模式）+ `others`（竞争报价——基准价计算面）。
///
/// out：试算面（score/detail）+ 推荐面（optimal_price/expected_score_at_optimal）
/// + 人读 analysis；两者可同场（「这么报得几分 + 最优该报多少」一次回答）。
fn score_price(_svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "score_price";
    let spec = parse_price_spec(&args, TOOL)?;
    // 规格合法性（单一真源——B1 引擎的 validate 面，壳层不重复裁决逻辑）
    spec.validate().map_err(ToolError::from)?;
    let bid_price = opt_f64(&args, "bid_price", TOOL)?;
    let others = opt_f64_array(&args, "others", TOOL)?.unwrap_or_default();

    // ---- 试算面（bid_price 在场——「这么报价能得几分」）
    let detail = match bid_price {
        Some(p) => Some(spec.score_detail(p, &others).map_err(ToolError::from)?),
        None => None,
    };
    // ---- 推荐面（optimal_price——「该报多少价」；竞争报价不足/方法无界时
    //      B1 拒绝：纯推荐模式响亮透传，试算在场则降级为告警——半答案优于没答案）
    let mut warnings: Vec<String> = Vec::new();
    let (optimal, at_optimal): (Option<f64>, Option<f64>) = match spec.optimal_price(&others) {
        Ok(op) => {
            let sc = spec.score(op, &others).map_err(ToolError::from)?;
            (Some(op), Some(sc))
        }
        Err(e) => {
            if bid_price.is_none() {
                return Err(ToolError::from(e));
            }
            warnings.push(format!(
                "最优报价反推失败（{e:?}）——竞争报价不足或方法无界，试算结果仍有效"
            ));
            (None, None)
        }
    };

    // ---- 人读分析（确定性拼装——方法口径 + 最优 + 当前试算）
    use bid_domain::pricing::{DiscountBase, PriceScoreMethod};
    let method_desc = match &spec.method {
        PriceScoreMethod::BenchmarkPrice { coefficient } => {
            format!("基准价法：评标基准价 = 控制价 × {coefficient}")
        }
        PriceScoreMethod::LowestPrice => {
            "最低价法：评标基准价 = 全部有效报价（含己方）的最低值".to_string()
        }
        PriceScoreMethod::AveragePrice { coefficient } => {
            format!("平均价法：评标基准价 = 有效报价（含己方）算术平均 × {coefficient}")
        }
        PriceScoreMethod::Composite {
            trim_high,
            trim_low,
            budget_weight,
            ..
        } => format!(
            "复合法：剔高 {trim_high}/剔低 {trim_low} 后均价与控制价按 {budget_weight} 加权"
        ),
        PriceScoreMethod::DiscountRate { base } => match base {
            DiscountBase::Fixed(r) => {
                format!("下浮率法：基准下浮率固定 {r}（下浮率 =（控制价−报价）/控制价）")
            }
            DiscountBase::FieldAverage => {
                "下浮率法：基准下浮率 = 全场有效报价的平均下浮率".to_string()
            }
        },
    };
    let mut analysis = format!(
        "评标办法：{method_desc}；价格分满分 {} 分。",
        spec.max_score
    );
    if let (Some(op), Some(sc)) = (optimal, at_optimal) {
        analysis.push_str(&format!(
            "最优报价 {op:.2}（期望得分 {sc:.2}/{}，命中基准价）。",
            spec.max_score
        ));
    }
    if let Some(d) = &detail {
        analysis.push_str(&format!(
            "当前报价（bid_price={:.2}）：基准价 {:.2}，偏离 {:+.2}%，扣 {:.2} 分，得分 {:.2}。",
            bid_price.unwrap_or(0.0),
            d.benchmark,
            d.deviation * 100.0,
            d.deduction,
            d.score
        ));
    }
    if others.is_empty() {
        analysis.push_str(
            "提示：未提供 others（竞争报价）——基准价按己方单家口径计算，实际以开标全场为准。",
        );
    }

    Ok(json!({
        "spec": {
            "method": serde_json::to_value(&spec.method).unwrap_or(Value::Null),
            "max_score": spec.max_score,
            "budget": spec.budget,
            "deviation_rule": serde_json::to_value(&spec.deviation_rule).unwrap_or(Value::Null),
        },
        "bid_price": bid_price,
        "others": others,
        "score": detail.as_ref().map(|d| d.score),
        "detail": detail,
        "optimal_price": optimal,
        "expected_score_at_optimal": at_optimal,
        "analysis": analysis,
        "warnings": warnings,
        "request_id": next_request_id(),
    }))
}

/// `score_section` —— 章节自评（issue #100 §3.1——「这一章能得几分」）。
///
/// 壳层职责：参数校验 → route_guard（只读诊断，MatrixReady+ 窗口）→ 前置
/// （章节在场 + 已起草——无正文的自评是空转）→ B2 [`SectionScorer`] 只调用
/// 不改 → out 组装（item_scores 逐项明细 + rationale 可追溯面）。
fn score_section(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "score_section";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let section_id = req_str(&args, "section_id", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;
    route_guard(svc, &project_id, TOOL, project_version)?;
    // 前置：章节在场 + 已起草
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == section_id)
        .cloned()
        .ok_or_else(|| ToolError::ENotFound {
            what: format!("section {section_id} 不存在（先 build_outline）"),
        })?;
    let content = node.content.clone().ok_or_else(|| {
        let has_child = svc
            .store()
            .sections()
            .iter()
            .any(|n| n.parent_id.as_deref() == Some(section_id.as_str()));
        ToolError::EUnprocessable {
            what: if has_child {
                format!("{section_id} 是结构节点（有子）——正文只住叶，请对叶章节自评")
            } else {
                format!(
                    "section {section_id} 未起草——先 draft_response_section（无正文的自评无意义）"
                )
            },
        }
    })?;
    let (scorer, _items) = build_scorer(svc.store());
    let s = scorer.score_section(&section_id, &content);
    Ok(json!({
        "project_id": project_id,
        "section_id": s.section_id,
        "section_title": node.title,
        "chars": content.chars().filter(|c| !c.is_whitespace()).count(),
        "estimated": s.estimated,
        "max": s.max,
        "covered_items": s.covered_items,
        "missing_items": s.missing_items,
        "rationale": s.rationale,
        "item_scores": s.item_scores,
        "request_id": next_request_id(),
    }))
}

/// `score_document` —— 整本自评（issue #100 §3.2——「这份标书能得几分」）。
///
/// 聚合面（B2 `score_document` 的三段口径）：技术（主观叶）/ 商务（客观叶）/
/// 价格（`bid_price` 在场才计——**没报价就不算**，price 出 null 而非 0）。
/// `target` = 非硬门可达叶的满分合计（诚实满分——含价格项与未映射项，
/// 不因「没写」而缩小分母）；`gap` = target − total；`by_item` 为全量
/// 逐项明细（未覆盖项 estimated=0 如实呈现）。
fn score_document(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "score_document";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;
    route_guard(svc, &project_id, TOOL, project_version)?;
    // 前置：章节树在场（与 check_bid_quality 同口径）
    if svc.store().sections().is_empty() {
        return Err(ToolError::EState {
            what: format!(
                "{} 态无章节树——先 build_outline 再自评",
                svc.store()
                    .project()
                    .map(|p| p.status.as_str())
                    .unwrap_or("?")
            ),
        });
    }
    // 已起草叶 → (section_id, content)（正文只住叶——与 assemble/export 同口径）
    let parents: std::collections::HashSet<&str> = svc
        .store()
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.as_deref())
        .collect();
    let sections: Vec<(String, String)> = svc
        .store()
        .sections()
        .iter()
        .filter(|n| !parents.contains(n.id.as_str()))
        .filter_map(|n| n.content.clone().map(|c| (n.id.clone(), c)))
        .collect();
    // 价格面（bid_price 缺省 = 不计价——价格参数仅在报价在场时消费）
    let bid_price = opt_f64(&args, "bid_price", TOOL)?;
    let others = opt_f64_array(&args, "others", TOOL)?.unwrap_or_default();
    let mut spec_echo = Value::Null;
    let spec_opt = match bid_price {
        Some(_) => {
            let spec = parse_price_spec(&args, TOOL)?;
            spec.validate().map_err(ToolError::from)?;
            spec_echo = json!({
                "method": serde_json::to_value(&spec.method).unwrap_or(Value::Null),
                "max_score": spec.max_score,
                "budget": spec.budget,
                "deviation_rule": serde_json::to_value(&spec.deviation_rule).unwrap_or(Value::Null),
            });
            Some(spec)
        }
        None => None,
    };
    let price_input = spec_opt
        .as_ref()
        .map(|spec| bid_domain::scoring::PriceInput {
            spec,
            bid_price: bid_price.unwrap_or(0.0),
            others,
        });
    let (scorer, items) = build_scorer(svc.store());
    let doc = scorer.score_document(&sections, price_input);
    // ---- by_item + target：全量非硬门可达叶（叶 = 未被任何项引用为 parent）
    let parent_ids: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|i| i.parent_id.as_deref())
        .collect();
    let mut by_item: Vec<Value> = Vec::new();
    let mut target = 0.0f64;
    for it in items
        .iter()
        .filter(|i| !i.is_hard_gate && !parent_ids.contains(i.requirement_id.as_str()))
    {
        let max = it.max_score.unwrap_or(0.0);
        target += max;
        if is_price_item_mirror(it) {
            // 价格类叶：内容自评无意义（价格分 = f(报价)）——estimated 取 B1 实算值
            by_item.push(json!({
                "item_id": it.requirement_id,
                "criteria": it.criteria,
                "max": max,
                "estimated": doc.price,
                "tier": "价格分（f(报价)——score_price/B1 计算）",
                "coverage": Value::Null,
                "section_id": it.section_id,
            }));
            continue;
        }
        let est = doc
            .breakdown
            .iter()
            .flat_map(|s| s.item_scores.iter())
            .find(|isc| isc.item_id == it.requirement_id);
        by_item.push(json!({
            "item_id": it.requirement_id,
            "criteria": it.criteria,
            "max": max,
            "estimated": est.map(|e| e.estimated).unwrap_or(0.0),
            "tier": est.map(|e| e.tier.label()).unwrap_or("未覆盖"),
            "coverage": est.map(|e| e.coverage).unwrap_or(0.0),
            "section_id": it.section_id,
        }));
    }
    let gap = ((target - doc.total) * 100.0).round() / 100.0;
    Ok(json!({
        "project_id": project_id,
        "technical": doc.technical,
        // 没报价就不算（issue §3.2：price null 而非伪装 0）
        "price": if bid_price.is_some() { json!(doc.price) } else { json!(null) },
        "commercial": doc.commercial,
        "total": doc.total,
        "target": (target * 100.0).round() / 100.0,
        "gap": gap,
        "by_item": by_item,
        "breakdown": doc.breakdown,
        "notes": doc.notes,
        "price_input": spec_echo,
        "sections_scored": sections.len(),
        "request_id": next_request_id(),
    }))
}

/// 评分树 + 大纲联结（#100 任务 3 的接线件）：评分项 REQ 的 `params.section`
/// 缺省时按大纲绑定关系补映射（REQ 所在**叶**章节——正文只住叶，与导出
/// 面同口径）。解析层未填 section 槽位是常态（synth/real 均如此），不联结
/// 则自评恒 UNMAPPED——按「条款被响应的章节」联结是确定性兜底（无 LLM），
/// 显式 params.section 优先（解析层/人工映射不被覆盖）。
///
/// 返回 (scorer, 联结后首现序 items)——items 供 by_item/target 的全量
/// 「第X章」章号归一（#170 方案 A）：中文/阿拉伯数字统一为数值——
/// 「第8章」「第八章」「第 8 章」→ 8。中文数字支持到千（真实招标文件
/// 章节数域：水利标书最多见「第 24 章」，千位余量足够）。非章序数
/// （「第1页」「第2行」）因后随字符非「章」不会误触。
fn chapter_no_of(text: &str) -> Option<u32> {
    const CN: &[(char, u32)] = &[
        ('零', 0), ('〇', 0), ('两', 2),
        ('一', 1), ('二', 2), ('三', 3), ('四', 4), ('五', 5),
        ('六', 6), ('七', 7), ('八', 8), ('九', 9),
        ('十', 10), ('百', 100), ('千', 1000),
    ];
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < cs.len() {
        if cs[i] != '第' {
            i += 1;
            continue;
        }
        // 收集「第」与「章」之间的数字串（中文/阿拉伯分别累计）
        let mut j = i + 1;
        let mut has_cn = false;
        let mut arabic = String::new();
        while j < cs.len() {
            let c = cs[j];
            if c.is_ascii_digit() || ('０'..='９').contains(&c) {
                arabic.push(c);
                j += 1;
            } else if CN.iter().any(|(cn, _)| *cn == c) {
                has_cn = true;
                j += 1;
            } else if c.is_whitespace() {
                // 章号边缘/内部的空白跳过（实测形态「第 8 章」——塔里木件）。
                // 「第 1 页」类不受影响：'页' 非 '章'，整段判 None。
                j += 1;
            } else {
                break;
            }
        }
        let parsed = if j < cs.len() && cs[j] == '章' && j > i + 1 {
            if !arabic.is_empty() && !has_cn {
                // 阿拉伯段：「第 8 章」「第24章」
                arabic.parse::<u32>().ok()
            } else if has_cn {
                // 中文数字段 [i+1, j)：乘法加法累计（「二十四」=24、「一百零八」=108）
                let mut total: u32 = 0;
                let mut current: u32 = 0;
                for &c in &cs[i + 1..j] {
                    let v = CN.iter().find(|(cn, _)| *cn == c).map(|(_, v)| *v).unwrap_or(0);
                    match v {
                        10 | 100 | 1000 => {
                            // 十/百/千：单位——current 为 0 时按 1（「十」=10）
                            total += current.max(1) * v;
                            current = 0;
                        }
                        _ => current = current * 10 + v,
                    }
                }
                Some(total + current)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(no) = parsed {
            return Some(no);
        }
        i += 1;
    }
    None
}

/// 节点子树的首片叶（DFS 首现序——确定性）。自身无子即自身是叶。
fn first_leaf_under<'a>(
    store: &'a bid_domain::DomainStore,
    root: &str,
) -> Option<&'a str> {
    let children_of = |id: &str| -> Vec<&bid_domain::outline::OutlineNode> {
        store
            .sections()
            .iter()
            .filter(|n| n.parent_id.as_deref() == Some(id))
            .collect()
    };
    let mut stack = vec![root.to_string()];
    while let Some(id) = stack.pop() {
        let kids = children_of(&id);
        if kids.is_empty() {
            // 自身是叶（根 id 在 sections 里存在性由调用方保证——不在则 None）
            if store.sections().iter().any(|n| n.id == id) {
                return Some(store.sections().iter().find(|n| n.id == id)?.id.as_str());
            }
            return None;
        }
        // DFS 首序：子节点逆序入栈（pop 时正序）
        for k in kids.into_iter().rev() {
            stack.push(k.id.clone());
        }
    }
    None
}

/// 逃代（derive_scoring_items 首现序，确定性；HashMap 索引无序不可逃代）。
fn build_scorer(
    store: &bid_domain::DomainStore,
) -> (
    bid_domain::scoring::SectionScorer,
    Vec<bid_domain::scoring::ScoringItem>,
) {
    let parents: std::collections::HashSet<&str> = store
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.as_deref())
        .collect();
    let mut leaf_of: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for n in store.sections() {
        if !parents.contains(n.id.as_str()) {
            for rid in &n.requirement_ids {
                leaf_of.insert(rid.as_str(), n.id.as_str());
            }
        }
    }
    // #170 方案 A（联结侧）：评分项不进正文（#126）→ leaf_of 反查对评分项
    // 永不命中（老路径保留——对正文类目 REQ 仍有效）。新增评审表章引用联结：
    // params.section_title（bid-parse 从评审表行文本登记的「第X章…」引用）
    // → 章号归一定位「被引用章的正文叶」——「详见第X章」的语义是第X章的
    // **内容**，而大纲是类目重组（技术参数/商务要求…横跨物理章），故匹配
    // 面取两路（先后优先）：
    //   ① 锚在该章的正文 REQ 所住的叶（leaf_of 反查——语义最准：被评内容
    //      就是那一章的条款）；
    //   ② 大纲节点 title 自带章号（「其他」类目按章分组的形态）→ 子树首叶。
    // 两路都取首现序（确定性可重放）。「评分项按章节自评」由此恢复；无引用
    // 的评分项维持 section=None（数据缺引用时不虚构映射——score_section 对
    // 该条恒空是诚实行为）。
    let chapter_leaf: std::collections::HashMap<u32, &str> = {
        let mut m: std::collections::HashMap<u32, &str> = std::collections::HashMap::new();
        for r in store.requirements() {
            // 只认正文类目：否决/资格的 REQ 住「资格与响应」响应叶——那是
            // 「对条款的应答」的家，不是「被评内容」的家（评分项引用第X章
            // 指的是该章的内容条款）。
            if !bid_domain::outline::is_body_category(&r.category) {
                continue;
            }
            let Some(leaf) = leaf_of.get(r.id.as_str()) else {
                continue;
            };
            for a in &r.anchors {
                for seg in a.locator.split(['>', '＞']) {
                    if let Some(no) = chapter_no_of(seg.trim()) {
                        m.entry(no).or_insert(leaf);
                        break;
                    }
                }
            }
        }
        for n in store.sections() {
            if let Some(no) = chapter_no_of(&n.title) {
                if let Some(leaf) = first_leaf_under(store, &n.id) {
                    m.entry(no).or_insert(leaf);
                }
            }
        }
        m
    };
    let mut reqs = store.requirements().to_vec();
    for r in &mut reqs {
        if r.category == "评分项" && r.params.get("section").is_none() {
            if let Some(sec) = leaf_of.get(r.id.as_str()) {
                r.params.insert("section".into(), serde_json::json!(sec));
            } else if let Some(title) = r.params.get("section_title").and_then(|v| v.as_str()) {
                if let Some(no) = chapter_no_of(title) {
                    if let Some(sec) = chapter_leaf.get(&no) {
                        r.params.insert("section".into(), serde_json::json!(sec));
                    }
                }
            }
        }
    }
    // 项目专名锚（B3 ProjectContext 同源——具体性判定的锚词面）
    let terms = bid_tools::quality::ProjectContext::from_store(store).scorer_terms();
    let items = bid_domain::scoring::derive_scoring_items(&reqs);
    let tree = bid_domain::scoring::derive_scoring_tree(&reqs);
    (bid_domain::scoring::SectionScorer::new(tree, terms), items)
}

/// 价格类评分项判定（`bid_domain::scoring::is_price_item` 的镜像——该函数
/// 未 pub；关键词面同步自同一真源，两处修改必须一起）。
fn is_price_item_mirror(it: &bid_domain::scoring::ScoringItem) -> bool {
    let c = &it.criteria;
    c.contains("价格分") || c.contains("投标报价") || c.contains("下浮率") || c.contains("基准价")
}

/// 价格分规格解析（score_price / score_document 共用——字符串糖 + 完整
/// serde 对象双形态；见 `score_price` 的参数面说明）。
fn parse_price_spec(v: &Value, at: &str) -> Result<bid_domain::pricing::PriceScoreSpec, ToolError> {
    use bid_domain::pricing::{DeviationRule, PriceScoreMethod, PriceScoreSpec};
    let coefficient = opt_f64(v, "coefficient", at)?;
    let method = match v.get("method") {
        None | Some(Value::Null) => PriceScoreMethod::AveragePrice {
            coefficient: coefficient.unwrap_or(0.95),
        },
        Some(Value::String(s)) => match s.as_str() {
            "benchmark" => PriceScoreMethod::BenchmarkPrice {
                coefficient: coefficient.unwrap_or(1.0),
            },
            "lowest" => PriceScoreMethod::LowestPrice,
            "average" => PriceScoreMethod::AveragePrice {
                coefficient: coefficient.unwrap_or(0.95),
            },
            other => return Err(ToolError::EUnprocessable {
                what: format!(
                    "{at}: method 须为 benchmark | lowest | average 或完整对象形态（实际: {other}）"
                ),
            }),
        },
        Some(obj @ Value::Object(_)) => serde_json::from_value(obj.clone()).map_err(|e| {
            ToolError::EUnprocessable {
                what: format!(
                    "{at}: method 完整对象形态解析失败（如 {{\"BENCHMARK_PRICE\":{{\"coefficient\":0.97}}}}）: {e}"
                ),
            }
        })?,
        Some(_) => return Err(ToolError::EUnprocessable {
            what: format!("{at}: method 须为字符串或对象"),
        }),
    };
    let deviation_rule = match v.get("deviation") {
        None | Some(Value::Null) => DeviationRule::Ratio,
        Some(Value::String(s)) => match s.as_str() {
            "ratio" => DeviationRule::Ratio,
            "price" => DeviationRule::Price {
                deduct_high: opt_f64(v, "deduct_high", at)?.ok_or_else(|| {
                    ToolError::EUnprocessable {
                        what: format!("{at}: deviation=price 需要 deduct_high（每高于基准 1% 扣分）"),
                    }
                })?,
                deduct_low: opt_f64(v, "deduct_low", at)?.ok_or_else(|| {
                    ToolError::EUnprocessable {
                        what: format!("{at}: deviation=price 需要 deduct_low（每低于基准 1% 扣分，0 = 不罚）"),
                    }
                })?,
            },
            "rate" => DeviationRule::Rate {
                deduct: opt_f64(v, "deduct", at)?.ok_or_else(|| {
                    ToolError::EUnprocessable {
                        what: format!("{at}: deviation=rate 需要 deduct（每偏离 1 个百分点扣分）"),
                    }
                })?,
            },
            other => return Err(ToolError::EUnprocessable {
                what: format!(
                    "{at}: deviation 须为 ratio | price | rate 或完整对象形态（实际: {other}）"
                ),
            }),
        },
        Some(obj @ Value::Object(_)) => serde_json::from_value(obj.clone()).map_err(|e| {
            ToolError::EUnprocessable {
                what: format!(
                    "{at}: deviation 完整对象形态解析失败（如 {{\"PRICE\":{{\"deduct_high\":0.2,\"deduct_low\":0.1}}}}）: {e}"
                ),
            }
        })?,
        Some(_) => return Err(ToolError::EUnprocessable {
            what: format!("{at}: deviation 须为字符串或对象"),
        }),
    };
    Ok(PriceScoreSpec {
        method,
        max_score: opt_f64(v, "max_score", at)?.unwrap_or(30.0),
        budget: opt_f64(v, "budget", at)?,
        deviation_rule,
    })
}

/// `generator` 参数（#68 阶段 1——缺省 auto；非 auto/mock/real 拒）。
fn opt_generator(v: &Value, key: &str, at: &str) -> Result<Option<String>, ToolError> {
    match opt_str(v, key, at)? {
        None => Ok(None),
        Some(g) if matches!(g.as_str(), "auto" | "mock" | "real" | "external") => Ok(Some(g)),
        Some(g) => Err(ToolError::EUnprocessable {
            what: format!("{at}: {key} 须为 auto | mock | real | external（实际: {g}）"),
        }),
    }
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
                    None => {
                        return Err(ToolError::EUnprocessable {
                            what: format!("{at}: {key} 须为字符串数组"),
                        })
                    }
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
    content_hash(s.unwrap_or(""))
}

/// 正文指纹（#100 任务 2——external 稿的幂等键面；与 instructions_hash
/// 同源同口径：SHA-256 前 8 字节十六进制）。
fn content_hash(s: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let d = h.finalize();
    let mut hex = String::with_capacity(16);
    for b in &d[..8] {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

// ---------------------------------------------------------------- §8 export_bid_document（#59 阶段 2）

/// #117 任务 1：从 export_bid_document 的 profile 参数里解析投标决策
/// （报价面——bid_amount + price_breakdown）。类型不对 → 响亮拒（静默吞
/// 掉拼错的字段 = 导出一份不自知的标书）。
fn parse_export_decision(profile: &Value) -> Result<bid_tools::export::ExportDecision, ToolError> {
    const TOOL: &str = "export_bid_document";
    let mut d = bid_tools::export::ExportDecision::default();
    match profile.get("bid_amount") {
        None | Some(Value::Null) => {}
        Some(v) if v.is_number() => d.bid_amount = v.as_f64(),
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: profile.bid_amount 须为数值（元，如 15800000）"),
            })
        }
    }
    // #117 任务 2：格式件决策面（全部可选；类型不对响亮拒）
    d.project_name = opt_str(profile, "project_name", TOOL)?;
    match profile.get("duration_days") {
        None | Some(Value::Null) => {}
        Some(v) if v.is_u64() => {
            d.duration_days = Some(v.as_u64().unwrap().min(u32::MAX as u64) as u32)
        }
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: profile.duration_days 须为正整数（日历天）"),
            })
        }
    }
    match profile.get("validity_days") {
        None | Some(Value::Null) => {}
        Some(v) if v.is_u64() => {
            d.validity_days = Some(v.as_u64().unwrap().min(u32::MAX as u64) as u32)
        }
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: profile.validity_days 须为正整数（日历天）"),
            })
        }
    }
    match profile.get("deposit_amount") {
        None | Some(Value::Null) => {}
        Some(v) if v.is_number() => d.deposit_amount = v.as_f64(),
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: profile.deposit_amount 须为数值（元）"),
            })
        }
    }
    d.legal_rep_gender = opt_str(profile, "legal_rep_gender", TOOL)?;
    match profile.get("price_breakdown") {
        None | Some(Value::Null) => {}
        Some(v) if v.is_array() => {
            for (i, item) in v.as_array().unwrap().iter().enumerate() {
                let at = format!("{TOOL}: profile.price_breakdown[{i}]");
                let get_str = |k: &str| -> Result<String, ToolError> {
                    item.get(k)
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .filter(|s| !s.trim().is_empty())
                        .ok_or_else(|| ToolError::EUnprocessable {
                            what: format!("{at}: {k} 必填（非空字符串）"),
                        })
                };
                let get_num = |k: &str| -> Result<f64, ToolError> {
                    item.get(k)
                        .and_then(Value::as_f64)
                        .ok_or_else(|| ToolError::EUnprocessable {
                            what: format!("{at}: {k} 必填（数值）"),
                        })
                };
                d.price_breakdown.push(bid_tools::export::PriceItem {
                    item: get_str("item")?,
                    unit: get_str("unit")?,
                    quantity: get_num("quantity")?,
                    unit_price: get_num("unit_price")?,
                    note: item
                        .get("note")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
        }
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!(
                "{TOOL}: profile.price_breakdown 须为数组（逐项 item/unit/quantity/unit_price）"
            ),
            })
        }
    }
    Ok(d)
}

/// `export_bid_document` —— 导出（契约 §8 + Lead #59 阶段 2 指令）。
///
/// 壳层职责：参数校验 → 项目路由 → 状态机裁决（Drafting+ / EXPORTED——
/// 导出的硬闸门在服务层**双前置检查**，见 tools 分派注释）→ 乐观并发 →
/// 幂等前置 → 转发 `BidService::export_document` → 契约 §8 out 组装。
///
/// format：`"docx"`（PDF 是阶段 7 的 LibreOffice 通道——本面诚实拒，不产
/// 半吊子）。暗标：`profile.dark_bid` + `profile.bidder_name`（公司名 →
/// 「投标人」；bidder 缺失响亮 warning 不静默）。
///
/// #117 任务 1/2：`profile` 携带投标决策——`bid_amount`（总报价，元）+
/// `price_breakdown`（分项报价数组）+ `project_name`（招标官方项目名）+
/// `duration_days`/`validity_days`（工期/有效期）+ `deposit_amount`
/// （保证金）+ `legal_rep_gender`（法定代表人性别）。全部可选；缺省 →
/// 表格/格式件相应栏落【待填】+ 告警，进幂等键（换决策重导不吃旧缓存）。
/// 接线详见 `bid_tools::export::build_export_extras`。
fn export_bid_document(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "export_bid_document";
    // ---- 参数校验（契约 §8 in）
    let project_id = req_str(&args, "project_id", TOOL)?;
    let format = req_str(&args, "format", TOOL)?;
    let output_path = req_str(&args, "output_path", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;
    // profile：{ dark_bid?: bool, bidder_name?: string, template? }
    let profile = args
        .get("profile")
        .filter(|v| v.is_object() || v.is_null())
        .cloned();
    let dark_bid = profile
        .as_ref()
        .and_then(|p| p.get("dark_bid"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let bidder_name = profile
        .as_ref()
        .and_then(|p| p.get("bidder_name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    // options：tracked_changes / watermark（阶段 7 面占位——truthy 才告警未支持）
    let mut warnings_pre: Vec<String> = Vec::new();
    if let Some(opts) = args.get("options").filter(|v| v.is_object()) {
        if opts
            .get("tracked_changes")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            warnings_pre
                .push("options.tracked_changes 暂不支持（阶段 7 面）——按普通模式导出".into());
        }
        // #86 摩擦 #14：仅 truthy 告警（watermark:false 是显式关闭，不该叫）
        if opts
            .get("watermark")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            warnings_pre
                .push("options.watermark 暂不支持（阶段 7 PDF 通道面）——按无水印导出".into());
        }
    }

    if format != "docx" && format != "pdf" {
        return Err(ToolError::EUnprocessable {
            what: format!("format={format} 非法（docx | pdf——PDF 走 LibreOffice headless 通道）"),
        });
    }
    // PDF 通道的环境预检（soffice 缺失 = 环境面——响亮拒，不静默降级 docx）
    if format == "pdf" && !bid_tools::service::soffice_available() {
        return Err(ToolError::EUnprocessable {
            what: "PDF 导出需要 soffice/libreoffice（环境缺失——安装 LibreOffice 后重试；docx 通道不受影响）".into(),
        });
    }
    // 输出路径安全面：必须落在工作区内（防 ../ 逃逸——导出产物不该写任意盘位）
    let out = std::path::PathBuf::from(&output_path);
    if out.is_absolute()
        || out
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ToolError::EUnprocessable {
            what: format!("output_path 须为工作区相对路径（不得绝对路径/含 ..）：{output_path}"),
        });
    }

    // ---- 项目路由 + 状态机裁决 + 乐观并发
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!(
                "project 不存在：{project_id}（当前服务态项目：{}）",
                proj.id
            ),
        });
    }
    svc.store().guard_tool(TOOL).map_err(ToolError::from)?;
    svc.store()
        .check_version(project_version)
        .map_err(ToolError::from)?;

    // ---- #117 任务 1：投标决策（报价面）——从 profile 解析，进幂等键（不同的
    //      报价决策 ≠ 同一次导出，不能吃旧缓存）
    let decision = parse_export_decision(profile.as_ref().unwrap_or(&Value::Null))?;
    let decision_fp = {
        // 短指纹（前 8 hex）：决策相同 → 同键；决策不同 → 不同键
        use sha2::Digest;
        let json = serde_json::to_string(&decision).unwrap_or_default();
        let digest = sha2::Sha256::digest(json.as_bytes());
        digest
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };

    // ---- 幂等前置：(project_id, content_version, format, dark_bid, output_path, decision)
    //      相同 → 直返首次结果（产物 sha 已在首次导出对账；重放零 IO）。
    //      #117：决策指纹入键——换了报价重导是新导出（否则返回旧报价的缓存）。
    let ikey = format!(
        "export:{project_id}:{}:{format}:{}:{output_path}:{decision_fp}",
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
    if dark_bid
        && bidder_name
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    {
        warnings_pre.push(
            "暗标模式但 profile.bidder_name 缺失——正文未做公司名脱敏（补 bidder_name 后重导）"
                .into(),
        );
    }
    let anonymize = if dark_bid {
        bidder_name.as_deref()
    } else {
        None
    };
    let ws = svc
        .workspace_dir()
        .ok_or_else(|| ToolError::EUnprocessable {
            what: "导出需要持久化服务态（BidService::open 建立的工作区）——纯内存态无章内容可导"
                .into(),
        })?;
    let abs_out = ws.join(&out);
    if let Some(parent) = abs_out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ToolError::EUnprocessable {
            what: format!("输出目录创建失败（{}）: {e}", parent.display()),
        })?;
    }
    let outcome = svc
        .export_document(&format, anonymize, &abs_out, &decision)
        .map_err(ToolError::from)?;

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
        "tables_written": outcome.report.tables_written,
        "table_kinds": outcome.report.table_kinds,
        "forms_written": outcome.report.forms_written,
        "form_titles": outcome.report.form_titles,
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

/// findings 元素的统一 JSON 形状（平面 findings 清单与 #187 categories 桶
/// 共用——单一真源，防两处形状漂移）。
fn validation_finding_json(f: &bid_tools::validate::ValidationFinding) -> Value {
    json!({
        "rule_code": f.rule_code,
        "kind": if f.rule_code.starts_with("M_") { "MODEL" } else { "DETERMINISTIC" },
        "severity": f.severity,
        "section_id": f.section_id,
        "requirement_id": f.requirement_id,
        "message": f.message,
    })
}

/// #187 categories 桶的 JSON 形状（四桶各自独立状态——「没做」显式报
/// not_performed，不因零发现伪装 passed）：
/// - `{"status": "passed"}`（零发现——无 findings 键，形状稳定）；
/// - `{"status": "failed", "findings": [...]}`（该类有发现，含 MINOR）；
/// - `{"status": "not_performed", "reason": "..."}`（该类未执行）。
fn validation_category_json(cr: &bid_tools::validate::CheckResult) -> Value {
    use bid_tools::validate::CheckResult;
    match cr {
        CheckResult::Passed => json!({ "status": "passed" }),
        CheckResult::Failed(fs) => json!({
            "status": "failed",
            "findings": fs.iter().map(validation_finding_json).collect::<Vec<_>>(),
        }),
        CheckResult::NotPerformed { reason } => json!({
            "status": "not_performed",
            "reason": reason,
        }),
    }
}

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
/// - `categories`（#187 分类返回）= structural / semantic / evidence /
///   compliance 四桶各自独立状态——`ok: true` 只意味着「已执行的检查中
///   BLOCKER == 0」，**不是**「整本标书已通过审核」（semantic 未配置模型时
///   显式 not_performed，#9 口径：没做 ≠ 做对了）。
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
        pre_warnings.push(
            "rules_profile=dark_bid 的暗标禁字清单扫描在阶段 7 词表面——本轮按 standard 规则集执行"
                .into(),
        );
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
            what: format!(
                "project 不存在：{project_id}（当前服务态项目：{}）",
                proj.id
            ),
        });
    }
    svc.store().guard_tool(TOOL).map_err(ToolError::from)?;
    svc.store()
        .check_version(project_version)
        .map_err(ToolError::from)?;

    // ---- 前置：章节树必须已建（无纲无校验对象）
    if svc.store().sections().is_empty() {
        return Err(ToolError::EState {
            what: format!("{} 态无章节树——先建纲再校验", proj.status.as_str()),
        });
    }

    // ---- 转发业务（纯只读诊断 + 推进 VALIDATING）
    let report = svc
        .validate_document(scope_sections.as_deref())
        .map_err(ToolError::from)?;

    // ---- 契约 §7 out（Lead 形状：ok + errors + warnings + findings + counts）
    let errors: Vec<Value> = report
        .findings
        .iter()
        .filter(|f| f.severity != "MINOR")
        .map(|f| {
            json!({
                "rule_code": f.rule_code,
                "severity": f.severity,
                "section_id": f.section_id,
                "requirement_id": f.requirement_id,
                "message": f.message,
            })
        })
        .collect();
    let mut warnings = pre_warnings;
    warnings.extend(
        report
            .findings
            .iter()
            .filter(|f| f.severity == "MINOR")
            .map(|f| f.message.clone()),
    );
    let out = json!({
        "ok": report.ok,
        "errors": errors,
        "warnings": warnings,
        "findings": report.findings.iter().map(validation_finding_json).collect::<Vec<_>>(),
        "counts": {
            "blocker": report.counts.blocker,
            "major": report.counts.major,
            "minor": report.counts.minor,
            "deterministic": report.counts.deterministic,
            "model": report.counts.model,
        },
        // #187 分类返回（纯加法）：四桶各自独立状态。not_performed 不影响
        // ok——它描述「该类检查没做」（如未配置模型），不是缺陷。
        "categories": {
            "structural": validation_category_json(&report.structural),
            "semantic": validation_category_json(&report.semantic),
            "evidence": validation_category_json(&report.evidence),
            "compliance": validation_category_json(&report.compliance),
        },
        "sections_checked": report.sections_checked,
        "project_status": svc.store().project().map(|p| p.status.as_str()).unwrap_or("UNKNOWN"),
        // #80 第三批外部 Agent 实测摩擦 #2：validate 会推进版本（VALIDATING
        // 转换面），响应必须回传新版本号——否则调用方后续带旧版本号 → E_CONFLICT
        // （契约 §0「响应统一携带 project_version」的承诺面）。
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    });
    Ok(out)
}

// ================================================================ #80 第三批 MCP 补齐（G1/G2/G3——契约 §10/§11/§12）

/// 项目路由 + 状态机裁决 + 乐观并发（三件套——四个新工具共用面）。
fn route_guard(
    svc: &BidService,
    project_id: &str,
    tool: &str,
    version: Option<u64>,
) -> Result<(), ToolError> {
    let proj = svc.store().project().map_err(ToolError::from)?;
    if proj.id != project_id {
        return Err(ToolError::ENotFound {
            what: format!(
                "project 不存在：{project_id}（当前服务态项目：{}）",
                proj.id
            ),
        });
    }
    svc.store().guard_tool(tool).map_err(ToolError::from)?;
    svc.store()
        .check_version(version)
        .map_err(ToolError::from)?;
    Ok(())
}

/// `build_outline` —— 契约 §10（G1：建纲 + 矩阵首建，REQUIREMENTS_EXTRACTED 态）。
///
/// 壳层职责：参数校验（spec 可选字段 + 值域）→ 三件套 → 前置（条款面非空）
/// → 转发 `BidService::prepare_outline_and_matrix`（推导大纲树 + 矩阵首建
/// 推进 MATRIX_READY + 写后自动落盘）→ 叶/目标字数统计组装。
///
/// 幂等面：语义位查重在域层 `set_outline`（推导确定性）；MATRIX_READY+ 重复
/// 调用被 guard_tool 拒（E_STATE——重建面属「补遗后增量重抽」的后续需求，
/// 不在首个版本隐式放行）。
fn build_outline(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "build_outline";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;
    let target_total_chars = opt_u64(&args, "target_total_chars", TOOL)?;
    let max_reqs_per_leaf = opt_u64(&args, "max_reqs_per_leaf", TOOL)?;
    if let Some(t) = target_total_chars {
        if !(1_000..=5_000_000).contains(&t) {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: target_total_chars 值域 1,000..=5,000,000（收到 {t}）"),
            });
        }
    }
    if let Some(m) = max_reqs_per_leaf {
        if !(1..=50).contains(&m) {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: max_reqs_per_leaf 值域 1..=50（收到 {m}）"),
            });
        }
    }

    route_guard(svc, &project_id, TOOL, project_version)?;

    // 前置：条款面非空（空纲无意义——与 build_matrix 同口径）
    let n_reqs = svc.store().requirements().len();
    if n_reqs == 0 {
        return Err(ToolError::EState {
            what: format!(
                "{} 态条款集为空——先 extract_requirements",
                svc.store()
                    .project()
                    .map(|p| p.status.as_str())
                    .unwrap_or("?")
            ),
        });
    }

    let spec = bid_domain::outline::OutlineSpec {
        target_total_chars: target_total_chars.unwrap_or(20_000).min(u32::MAX as u64) as u32,
        max_reqs_per_leaf: max_reqs_per_leaf.unwrap_or(8) as usize,
    };
    let (n_sections, n_rows, n_blocking) = svc
        .prepare_outline_and_matrix(spec)
        .map_err(ToolError::from)?;

    // 叶/目标字数统计（leaf = 不作为任何节点 parent 的节点）
    let sections = svc.store().sections().to_vec();
    let parents: std::collections::HashSet<&str> = sections
        .iter()
        .filter_map(|n| n.parent_id.as_deref())
        .collect();
    let leaves: Vec<&bid_domain::outline::OutlineNode> = sections
        .iter()
        .filter(|n| !parents.contains(n.id.as_str()))
        .collect();
    let total_target: u64 = leaves.iter().map(|n| n.target_chars as u64).sum();

    Ok(json!({
        "counts": {
            "sections": n_sections,
            "leaves": leaves.len(),
            "requirements": n_reqs,
            "total_target_chars": total_target,
            "matrix_rows": n_rows,
            "matrix_blocking": n_blocking,
        },
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

/// 注册入参的可选定位器解析（契约 §11——三种最常用形态的扁平面）。
///
/// 缺省（不传 locator）→ `MarkdownBlock { block_id: "mcp-registered:<source_file>" }`
/// ——诚实标记「经 MCP 登记面、无引擎定位器」（审计面可辨识，不冒充引擎抽取）。
fn parse_locator_arg(
    args: &Value,
    source_file: &str,
    fingerprint: &str,
    at: &str,
) -> Result<bid_domain::locator::EngineLocator, ToolError> {
    use bid_domain::locator::EngineLocator;
    let Some(loc) = args.get("locator").filter(|v| !v.is_null()) else {
        return Ok(EngineLocator::MarkdownBlock {
            block_id: format!("mcp-registered:{source_file}"),
            text_fingerprint: fingerprint.to_string(),
        });
    };
    if !loc.is_object() {
        return Err(ToolError::EUnprocessable {
            what: format!("{at}: locator 必须为对象 {{ kind, ... }}（收到 {loc}）"),
        });
    }
    let kind = loc["kind"]
        .as_str()
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{at}: locator.kind 必填（docx_paragraph|pdf_page_text|markdown_block）"),
        })?;
    let fp = |v: &Value| v.as_str().map(str::to_string).filter(|s| !s.is_empty());
    match kind {
        "docx_paragraph" => Ok(EngineLocator::DocxParagraph {
            paragraph_index: loc["paragraph_index"].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32,
            text_fingerprint: fp(&loc["text_fingerprint"]).ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{at}: locator.text_fingerprint 必填（算法前缀:值——如 fnv64:xx）"),
            })?,
        }),
        "pdf_page_text" => Ok(EngineLocator::PdfPageText {
            page: loc["page"].as_u64().unwrap_or(1).max(1).min(u32::MAX as u64) as u32,
            text_fingerprint: fp(&loc["text_fingerprint"]).ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{at}: locator.text_fingerprint 必填（算法前缀:值——如 fnv64:xx）"),
            })?,
        }),
        "markdown_block" => Ok(EngineLocator::MarkdownBlock {
            block_id: fp(&loc["block_id"]).ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{at}: locator.block_id 必填"),
            })?,
            text_fingerprint: fp(&loc["text_fingerprint"]).ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{at}: locator.text_fingerprint 必填（算法前缀:值——如 fnv64:xx）"),
            })?,
        }),
        other => Err(ToolError::EUnprocessable {
            what: format!("{at}: locator.kind 非法：{other}（首版支持 docx_paragraph|pdf_page_text|markdown_block；XLSX/Canvas 面待素材库）"),
        }),
    }
}

/// `register_evidence` 的 `fields` 参数解析（#147 结构化字段）。
///
/// - 缺省 / null → 空结构（向后兼容：旧调用方不传 `fields` 零影响）；
/// - 非对象 → E_UNPROCESSABLE（响亮拒，不静默丢结构化字段）；
/// - 反序列化（含 `amount_wan` 负数/NaN、`years` 非整数等类型面）失败 →
///   E_UNPROCESSABLE；
/// - 业务口径校验（日期形态 / 金额非负有限 / 年限上界）走域层
///   `EvidenceFields::validate`（单一真源——工具面不重复实现阈值），
///   失败 → E_UNPROCESSABLE。
fn parse_evidence_fields(
    args: &Value,
    at: &str,
) -> Result<bid_domain::evidence::EvidenceFields, ToolError> {
    let fields = match args.get("fields") {
        None | Some(Value::Null) => bid_domain::evidence::EvidenceFields::default(),
        Some(obj @ Value::Object(_)) => {
            serde_json::from_value(obj.clone()).map_err(|e| ToolError::EUnprocessable {
                what: format!(
                    "{at}: fields 解析失败: {e}（合法键：project_name|client|amount_wan|signed_at|finished_at|cert_name|cert_no|issuer|person_name|person_title|years）"
                ),
            })?
        }
        Some(other) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{at}: fields 须为对象（收到 {other}）"),
            })
        }
    };
    fields.validate().map_err(|reason| ToolError::EUnprocessable {
        what: format!("{at}: {reason}"),
    })?;
    Ok(fields)
}

/// `register_evidence` —— 契约 §11-A（G2：证据登记，MATRIX_READY+）。
///
/// 语义位查重/换版在域层（同 kind+title+subject+scope 的最新非 Superseded
/// 记录：同指纹 → 幂等重放；异指纹 → 旧记录单向降级 Superseded + 签发新 EV）。
/// fingerprint 缺省 = `sha256:<source_sha256>`（内容指纹 = 文件身份——调用方
/// 未给更细粒度时的诚实缺省）。
fn register_evidence(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "register_evidence";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let kind = parse_evidence_kind(&req_str(&args, "kind", TOOL)?, TOOL)?;
    let title = req_str(&args, "title", TOOL)?;
    let source_file = req_str(&args, "source_file", TOOL)?;
    // 无文件身份不入账（域层 validate_spec 同口径——工具面先拦，文案更定位）
    let source_sha256 = req_str(&args, "source_sha256", TOOL)?;
    let subject_name = req_str(&args, "subject_name", TOOL)?;
    let scope = opt_str(&args, "scope", TOOL)?.unwrap_or_default();
    let valid_from = opt_str(&args, "valid_from", TOOL)?;
    let valid_until = opt_str(&args, "valid_until", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;

    // fingerprint：显式 > sha256:<source_sha256>（裸值补前缀——跨算法可比性）
    let fingerprint = match opt_str(&args, "fingerprint", TOOL)? {
        Some(f) => f,
        None => {
            let s = source_sha256.trim();
            if s.contains(':') {
                s.to_string()
            } else {
                format!("sha256:{s}")
            }
        }
    };
    let locator = parse_locator_arg(&args, &source_file, &fingerprint, TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    // #117 任务 1：业绩类证据的业务面（可选——发包人/合同金额/完成日期；
    // 业绩一览表的真源。非字符串/非数值 → 响亮拒，不静默吞）
    let client = opt_str(&args, "client", TOOL)?;
    let amount_wan = match args.get("amount_wan") {
        None | Some(Value::Null) => None,
        Some(v) if v.is_number() => v.as_f64(),
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: amount_wan 须为数值（万元）"),
            })
        }
    };
    let completed_on = opt_str(&args, "completed_on", TOOL)?;

    // #147：结构化字段（可选对象——合同/证书/人员三面的金额、证书号、日期等）。
    // 非对象 / 字段类型不匹配 / 业务口径非法 → 一律 E_UNPROCESSABLE（不静默吞）。
    let fields = parse_evidence_fields(&args, TOOL)?;

    let spec = bid_domain::evidence::EvidenceSpec {
        kind,
        title,
        source_file,
        source_sha256,
        locator,
        fingerprint,
        valid_from,
        valid_until,
        subject_name,
        scope,
        client,
        amount_wan,
        completed_on,
        fields,
    };
    let (ev_id, deduplicated) = svc.register_evidence(spec).map_err(ToolError::from)?;
    let status = svc
        .store()
        .evidence(&ev_id)
        .map(|e| e.status.as_str().to_string())
        .unwrap_or_default();

    Ok(json!({
        "ev_id": ev_id,
        "deduplicated": deduplicated,
        "kind": kind_machine(kind),
        "status": status,
        "hint": if deduplicated {
            "语义位查重命中：同 key 同指纹 → 幂等重放（原 EV-id 直返，不换版）"
        } else {
            // #80 第三批外部 Agent 实测摩擦 #8：不得断言旧记录已降级——
            // 首登与换版在返回面不可分（都是签发新 EV），措辞改为条件式
            "新 EV 已签发（若为同 key 换版，旧记录已单向降级 SUPERSEDED；全新登记则无旧记录）"
        },
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

/// `bind_evidence` —— 契约 §11-B（G2：批量绑定 + 矩阵重算，MATRIX_READY+）。
///
/// 逐件语义：REQ 不存在 → 逐件 E_NOTFOUND；证据 EXPIRED/SUPERSEDED → 逐件
/// E_UNPROCESSABLE（R-03 禁绑）；重复绑定 → 幂等重放。部分失败 → E_PARTIAL
/// （data 携带成功面——与 import_tender_files 同款「错误码不吞成功面」）。
/// 绑后矩阵重算（行内证据即时可见）。
fn bind_evidence(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "bind_evidence";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let ev_id = req_str(&args, "ev_id", TOOL)?;
    let req_ids = opt_str_array(&args, "requirement_ids", TOOL)?.ok_or_else(|| {
        ToolError::EUnprocessable {
            what: format!("{TOOL}: requirement_ids 必填（非空数组——批量绑定面）"),
        }
    })?;
    if req_ids.is_empty() {
        return Err(ToolError::EUnprocessable {
            what: format!("{TOOL}: requirement_ids 不得为空（空批次无意义）"),
        });
    }
    let bid_open_date = req_str(&args, "bid_open_date", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    // 证据在场性前置（批量前单查——错误定位比逐件更清晰）
    svc.store().evidence(&ev_id).map_err(ToolError::from)?;

    let (bound, replayed, item_errors, rows, blocking) = svc
        .bind_evidence_batch(&ev_id, &req_ids, &bid_open_date)
        .map_err(ToolError::from)?;

    let data = json!({
        "ev_id": ev_id,
        "bound_new": bound,
        "replayed": replayed,
        "requested": req_ids.len(),
        "matrix": { "rows": rows, "blocking": blocking },
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    });
    if !item_errors.is_empty() {
        return Err(ToolError::EPartial {
            what: format!("{} 条绑定失败（成功 {} 条已落库）", item_errors.len(), bound + replayed),
            item_errors,
            hint: Some("逐件核对 item_errors：REQ 不存在 → 核对编号；禁绑 → 证据 EXPIRED/SUPERSEDED（续期请 register_evidence 换新版）".into()),
            data: Some(data),
        });
    }
    Ok(data)
}

/// 四门名解析（契约 §12——Gate::as_str 单一真源；第四门 addendum_waiver
/// 为 #131 真实语料验证落地：L3 无补遗阻塞的人工确认出口）。
fn parse_gate(s: &str, at: &str) -> Result<bid_domain::review::Gate, ToolError> {
    use bid_domain::review::Gate;
    match s {
        "matrix_approval" => Ok(Gate::MatrixApproval),
        "outline_approval" => Ok(Gate::OutlineApproval),
        "pre_export_confirm" => Ok(Gate::PreExportConfirm),
        "addendum_waiver" => Ok(Gate::AddendumWaiver),
        other => Err(ToolError::EUnprocessable {
            what: format!("{at}: gate 非法：{other}（合法值：matrix_approval|outline_approval|pre_export_confirm|addendum_waiver）"),
        }),
    }
}

/// 记审裁决解析（大小写双形态——serde SCREAMING_SNAKE_CASE 单一真源）。
fn parse_decision(s: &str, at: &str) -> Result<bid_domain::review::Decision, ToolError> {
    serde_json::from_value::<bid_domain::review::Decision>(Value::String(s.to_uppercase())).map_err(
        |_| ToolError::EUnprocessable {
            what: format!("{at}: decision 非法：{s}（合法值：APPROVED|REJECTED）"),
        },
    )
}

/// 记审 findings 解析：`[{ severity, code, message, section_id? }]` →
/// 域层 `(Severity, code, message, section_id)`。
fn parse_findings(
    args: &Value,
    at: &str,
) -> Result<Vec<(bid_domain::review::Severity, String, String, Option<String>)>, ToolError> {
    use bid_domain::review::Severity;
    let Some(arr) = args.get("findings").filter(|v| !v.is_null()) else {
        return Ok(vec![]);
    };
    let Some(items) = arr.as_array() else {
        return Err(ToolError::EUnprocessable {
            what: format!("{at}: findings 必须为数组（收到 {arr}）"),
        });
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, f) in items.iter().enumerate() {
        let sev = serde_json::from_value::<Severity>(Value::String(
            f["severity"].as_str().unwrap_or("").to_uppercase(),
        ))
        .map_err(|_| ToolError::EUnprocessable {
            what: format!("{at}: findings[{i}].severity 非法（合法值：BLOCKER|WARNING|INFO）"),
        })?;
        let code = f["code"].as_str().unwrap_or("").trim().to_string();
        if code.is_empty() {
            return Err(ToolError::EUnprocessable {
                what: format!("{at}: findings[{i}].code 必填（规则码——RT 追溯面）"),
            });
        }
        let message = f["message"].as_str().unwrap_or("").trim().to_string();
        if message.is_empty() {
            return Err(ToolError::EUnprocessable {
                what: format!("{at}: findings[{i}].message 必填"),
            });
        }
        out.push((
            sev,
            code,
            message,
            f["section_id"].as_str().map(str::to_string),
        ));
    }
    Ok(out)
}

/// `record_review_gate` —— 契约 §12（G3：三门记审，MATRIX_READY+）。
///
/// 复审人门（reviewer 须 `human:` 前缀——borrow mdcore ReviewPolicy：AI 操作者
/// 不得自审自过）、状态门（窗口外拒收）、门序（矩阵先于大纲；导出前门要求
/// 前两门 Approved）全在域层 `record_review` 裁决——壳层零重实现。
fn record_review_gate(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "record_review_gate";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let gate = parse_gate(&req_str(&args, "gate", TOOL)?, TOOL)?;
    let decision = parse_decision(&req_str(&args, "decision", TOOL)?, TOOL)?;
    let reviewer = req_str(&args, "reviewer", TOOL)?;
    let findings = parse_findings(&args, TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    let receipt = svc
        .record_review_gate(gate, &reviewer, decision, findings)
        .map_err(ToolError::from)?;

    // 门序进度（同一真源——域层 gate_approved）
    use bid_domain::review::Gate;
    let gates_approved: Vec<&str> = [
        (Gate::MatrixApproval, "matrix_approval"),
        (Gate::OutlineApproval, "outline_approval"),
        (Gate::PreExportConfirm, "pre_export_confirm"),
        (Gate::AddendumWaiver, "addendum_waiver"),
    ]
    .iter()
    .filter(|(g, _)| svc.store().gate_approved(*g))
    .map(|(_, n)| *n)
    .collect();

    Ok(json!({
        "receipt": {
            "review_id": receipt.review_id,
            "gate": receipt.gate.as_str(),
            "decision": receipt.decision.as_str(),
            "reviewer": reviewer,
            "finding_ids": receipt.finding_ids,
            "spawned_rt_ids": receipt.spawned_rt_ids,
            // #180 审核内容绑定：本次记审针对的章节树内容摘要（export_allowed
            // 比对判据；幂等重放时为既有记录的摘要）
            "reviewed_content_digest": receipt.reviewed_content_digest,
        },
        "gates_approved": gates_approved,
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

// ---------------------------------------------------------------- #184 record_addendum_waiver

/// 补遗就绪度报告块（import 响应 / record_addendum_waiver 出参共用）：
/// `{total, ready, not_ready: [{file_id, name, readiness, waived, reason}]}`。
///
/// 非 Ready 清单含**已确认**件（waived=true——审计面不因放行而消失，
/// Agent 据此知道哪些件是带确认放行的）。
fn addendum_readiness_block(svc: &BidService) -> Value {
    let statuses = bid_tools::readiness::addendum_statuses(svc.store());
    let ready = statuses
        .iter()
        .filter(|s| s.readiness == bid_tools::readiness::AddendumReadiness::Ready)
        .count();
    let not_ready: Vec<Value> = statuses
        .iter()
        .filter(|s| s.readiness != bid_tools::readiness::AddendumReadiness::Ready)
        .map(|s| {
            json!({
                "file_id": s.file_id,
                "name": s.name,
                "readiness": s.readiness.as_str(),
                "waived": s.waived,
                "reason": s.reason,
            })
        })
        .collect();
    json!({ "total": statuses.len(), "ready": ready, "not_ready": not_ready })
}

/// `record_addendum_waiver` —— #184 补遗分级阻塞的逐件人工确认。
///
/// 语义：任一补遗不处 Ready 时起草闸阻塞（E_UNPROCESSABLE，逐件点名）；
/// 人工继续必须**逐件**确认到具体 file_id（非全局布尔——「3 份坏 1 份」
/// 要对齐到哪一份、为什么故意带病放行）。确认即审计面：
/// `{file_id, reason, at, principal}`，principal 须 `human:` 前缀（与
/// record_review_gate 的 reviewer 同口径——AI 不得自审自过）。
///
/// 出参：`recorded`（本次确认）+ `addendum_waivers`（全部确认记录——
/// 验收 C 的「记录可查」）+ `addendum_readiness`（就绪度回显，含 waived
/// 标记——报告面仍含该件，但不再构成阻塞）。
///
/// 注：`principal` 作为 MCP 参数是过渡态——#182 将升级为 TrustedPrincipal
/// 验签面（receipt 级），届时本工具的入参口径随其演进，不另造第二套人设体系。
fn record_addendum_waiver(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "record_addendum_waiver";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let file_id = req_str(&args, "file_id", TOOL)?;
    let reason = req_str(&args, "reason", TOOL)?;
    let principal = req_str(&args, "principal", TOOL)?;
    let project_version = opt_u64(&args, "project_version", TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    let recorded = svc
        .record_addendum_waiver(&file_id, &reason, &principal)
        .map_err(ToolError::from)?;

    let waivers: Vec<Value> = svc
        .store()
        .addendum_waivers()
        .iter()
        .map(|w| {
            json!({
                "file_id": w.file_id,
                "reason": w.reason,
                "at": w.at,
                "principal": w.principal,
            })
        })
        .collect();

    Ok(json!({
        "recorded": {
            "file_id": recorded.file_id,
            "reason": recorded.reason,
            "at": recorded.at,
            "principal": recorded.principal,
        },
        "addendum_waivers": waivers,
        "addendum_readiness": addendum_readiness_block(svc),
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

#[cfg(test)]
mod chapter_no_tests {
    use super::chapter_no_of;

    /// #170 方案 A：章号归一的形态面（中文/阿拉伯/带空白/非章序数）。
    #[test]
    fn chapter_no_forms() {
        assert_eq!(chapter_no_of("第四章 评标办法"), Some(4));
        assert_eq!(chapter_no_of("第 8 章投标文件"), Some(8));
        assert_eq!(chapter_no_of("第24章"), Some(24));
        assert_eq!(chapter_no_of("第十二章"), Some(12));
        assert_eq!(chapter_no_of("第二十四章"), Some(24));
        assert_eq!(chapter_no_of("第一百零八章"), Some(108));
        assert_eq!(chapter_no_of("第三章的所有要求"), Some(3));
        // 非章序数不误触
        assert_eq!(chapter_no_of("第1页 行6"), None);
        assert_eq!(chapter_no_of("无任何章节引用"), None);
    }
}
