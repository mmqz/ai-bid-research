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
use bid_domain::matrix::ResponseStatus;
use bid_domain::{ParseStatus, ProjectStatus};
use bid_tools::service::{BidService, ImportSpec};

/// 按工具名分派。调用方保证 name 已在 [`crate::TOOL_NAMES`] 内。
pub(crate) fn invoke(name: &str, args: Value, svc: &mut BidService) -> ToolResult {
    match name {
        "get_bid_project_state" => get_bid_project_state(svc, args),
        // ---- #42 阶段 10：AI 主动存版本（skill 纪律「每完成一个章节调用」）----
        // 与桌面壳 snapshots.save 同一真源（mang-snap）；不走项目状态机（版本
        // 快照与项目状态正交，任何阶段都允许存）。
        "save_version" => save_version(svc, args),
        "create_bid_project" => create_bid_project(svc, args),
        "import_tender_files" => import_tender_files(svc, args),
        "extract_requirements" => extract_requirements(svc, args),
        "get_compliance_matrix" => get_compliance_matrix(svc, args),
        // ---- S-4 大结果分页（V3-5）：只读列表工具，游标信封见 cursor.rs ----
        "list_evidence" => list_evidence(svc, args),
        "list_tasks" => list_tasks(svc, args),
        // ---- #59 阶段 1：单章起草（契约 §6——mock 生成 + 引用回查 + CAS 写入）----
        "draft_response_section" => draft_response_section(svc, args),
        // ---- #254 章节级增量写入（分段追加/局部修订/账本发现）----
        "append_section_segment" => append_section_segment(svc, args),
        "rewrite_section_segment" => rewrite_section_segment(svc, args),
        "list_section_segments" => list_section_segments(svc, args),
        // ---- #319 补遗后增量重写（变更登记→指令→局部改写+局部复验）----
        "register_addendum_changes" => register_addendum_changes(svc, args),
        "rewrite_section_partial" => rewrite_section_partial(svc, args),
        // ---- #59 阶段 2：导出（契约 §8——双前置门 + format-spec + 暗标）----
        "export_bid_document" => export_bid_document(svc, args),
        // ---- #59 阶段 3：双层校验（契约 §7——确定性规则先行 + 模型复核注入）----
        "validate_bid_document" => validate_bid_document(svc, args),
        // ---- #68 阶段 2：生成质量覆盖度检查（只读诊断 + 报告落盘）----
        "check_bid_quality" => check_bid_quality(svc, args),
        // ---- #80 第三批 MCP 补齐（G1/G2/G3——外部 Agent 纯 MCP 驱动全链）----
        "build_outline" => build_outline(svc, args),
        // ---- #287 篇幅档位推荐（只读——P3 建纲前：建议+理由+alternatives 交用户确认） ----
        "recommend_length_profile" => recommend_length_profile(svc, args),
        "register_evidence" => register_evidence(svc, args),
        "bind_evidence" => bind_evidence(svc, args),
        "record_review_gate" => record_review_gate(svc, args),
        // ---- #184：补遗分级阻塞的逐件人工确认（L3 伴生闸）----
        "record_addendum_waiver" => record_addendum_waiver(svc, args),
        // ---- #226 层 1：否决项死锁的人工出口（规则说明标 NOT_APPLICABLE）----
        "set_matrix_override" => set_matrix_override(svc, args),
        // ---- #243：历史异常态（VALIDATING+缺章）的受支持恢复出口 ----
        "recover_validating_state" => recover_validating_state(svc, args),
        // ---- #100 任务 3：B1/B2 接线（能得几分、该报多少价）----
        // score_price 是纯计算器（无 project 路由/状态门——报价推演面）；
        // score_section / score_document 走既有 route_guard（只读诊断）。
        "score_price" => score_price(svc, args),
        // ---- #219 全块覆盖：引擎切块 + Agent 标区间（替代正则猜边界） ----
        "get_document_chunks" => crate::coverage_tools::get_document_chunks(svc, args),
        "submit_block_marks" => crate::coverage_tools::submit_block_marks(svc, args),
        "get_coverage_report" => crate::coverage_tools::get_coverage_report(svc, args),
        // ---- #285 ① ★漏标检测（只读——★字面真值 vs 已抽 REQ 差集，只报不补） ----
        "get_star_coverage" => crate::coverage_tools::get_star_coverage(svc, args),
        // ---- #285 ② 块级双向索引三工具（只读——REQ↔块双向反查 + overlay）----
        "get_requirement_provenance" => crate::bidi_tools::get_requirement_provenance(svc, args),
        "get_block_requirements" => crate::bidi_tools::get_block_requirements(svc, args),
        "get_annotation_overlay" => crate::bidi_tools::get_annotation_overlay(svc, args),
        "score_section" => score_section(svc, args),
        "score_document" => score_document(svc, args),
        // ---- #145 任务 1：双通道证据检索（契约 §5——朴素实现，真向量库见 M2-b）----
        "search_evidence" => search_evidence(svc, args),
        // ---- #279：行协议发现面（无状态前置的只读握手——工具清单 + 契约摘要）----
        "list_tools" => list_tools(),
        other => Err(ToolError::ENotFound {
            what: format!("未接入的工具 {other}"),
        }),
    }
}

// ---------------------------------------------------------------- #279 list_tools

/// `list_tools` —— 行协议发现面（#279 第 1 项）。
///
/// 背景：#269 六场黑盒考验里两个独立代理都试探了 list_tools（Agent 的合理
/// 预期：先握手再干活）——行协议无此工具只能报 E_NOTFOUND，代理只能盲猜
/// 信封结构（danqing / hc-cluster 各 5 连败的间接成因）。标准 MCP JSON-RPC
/// 客户端有 tools/list 方法等价获取，本工具补行协议的缺口。
///
/// 纯只读、无参数、无状态前置（建项目前即可调）：返回工具清单（单一真源
/// [`crate::TOOL_NAMES`] + schema 描述）+ 行协议契约摘要（信封形态/错误码
/// 面/CAS 纪律/断点恢复入口）——把「靠猜」变成「一次调用可知」。
fn list_tools() -> ToolResult {
    let tools: Vec<Value> = crate::TOOL_NAMES
        .iter()
        .map(|name| {
            json!({
                "name": name,
                "summary": crate::schema::describe(name),
            })
        })
        .collect();
    Ok(json!({
        "count": tools.len(),
        "tools": tools,
        "line_protocol": {
            "request": "每行一个 JSON 对象：{\"tool\":\"<工具名>\",\"args\":{<参数>}}",
            "response": "每行一个 JSON 应答：{\"ok\":true,\"tool\":..,\"data\":..} 或 {\"ok\":false,\"tool\":..,\"error\":{\"code\":..,\"what\":..}}",
            "notes": [
                "错误按 error.code 分支（E_STATE/E_NOTFOUND/E_UNPROCESSABLE/E_CONFLICT/...），不要解析 what 文案",
                "参数必须嵌套在 args 下——顶层参数会报信封结构错误（#279）",
                "project_version 乐观并发：传上次响应的数字（强一致，过期 E_CONFLICT）或 \"latest\"（显式声明无并发）；缺省跳过检查",
                "断点恢复先调 get_bid_project_state",
                "工具契约详情：docs/bid-mcp-contract.md"
            ]
        }
    }))
}

// ---------------------------------------------------------------- 参数校验助手

/// `project_version` 乐观并发参数解析（#279 第 3 项——CAS 心智负担）。
///
/// 形态三选一：
/// - 非负整数 → `Some(n)`：强一致 CAS，过期 `E_CONFLICT`（现状不变——
///   报错已带对齐建议，#269 三场全部自愈，**不动**）；
/// - 字符串 `"latest"` → `None`：显式声明「我确认无并发」——与缺省同语义
///   （跳过检查），但**意图可见**：单代理顺序作业时调用方不必追踪版本号，
///   也不再因为揣着过期数字吃 E_CONFLICT 重试；误用时审计面上可辨认；
/// - 缺省 / null → `None`：内部步骤语义（现状不变）。
fn opt_cas_version(v: &Value, at: &str) -> Result<Option<u64>, ToolError> {
    match v.get("project_version") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{at}: project_version 须为非负整数或 \"latest\""),
        }),
        Some(Value::String(s)) if s == "latest" => Ok(None),
        Some(_) => Err(ToolError::EUnprocessable {
            what: format!("{at}: project_version 须为非负整数或 \"latest\"（显式声明无并发）"),
        }),
    }
}

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

pub(crate) fn req_str(v: &Value, key: &str, at: &str) -> Result<String, ToolError> {
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

pub(crate) fn opt_str(v: &Value, key: &str, at: &str) -> Result<Option<String>, ToolError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ToolError::EUnprocessable {
            what: format!("{at}: {key} 须为字符串"),
        }),
    }
}

pub(crate) fn opt_u64(v: &Value, key: &str, at: &str) -> Result<Option<u64>, ToolError> {
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
pub(crate) fn route_project(svc: &BidService, project_id: &str, tool: &str) -> Result<(), ToolError> {
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

    // #319 发现面：DIRTY 章节（补遗/重开致脏）+ 待处置改写指令——
    // 断点恢复时「哪些章节还没追上补遗」一眼可见（无脏零指令 = 零成本）
    let dirty_sections: Vec<Value> = svc
        .store()
        .sections()
        .iter()
        .filter(|n| n.dirty)
        .map(|n| {
            json!({
                "section_id": n.id,
                "title": n.title,
                "section_status": n.status.as_str(),
                "pending_directives": svc.store().pending_rewrite_directives_of(&n.id).len(),
            })
        })
        .collect();
    let pending_directives: Vec<Value> = svc
        .store()
        .pending_rewrite_directives()
        .iter()
        .map(|d| {
            json!({
                "id": d.id,
                "section_id": d.section_id,
                "change_id": d.change_id,
                "req_id": d.req_id,
                "change_type": d.change_type.as_str(),
                "old_text": d.old_text,
                "new_requirement": d.new_requirement,
                "affected_segments": d.affected_segments.iter().map(|a| json!({
                    "segment_id": a.segment_id,
                    "seq": a.seq,
                    "matched_by": a.matched_by,
                })).collect::<Vec<_>>(),
                "created_at": d.created_at,
            })
        })
        .collect();

    Ok(json!({
        "status": status.as_str(),
        "allowed_next_tools": status.allowed_tools(),
        "terminal": status.next_allowed().is_empty(),
        "dirty_sections": dirty_sections,
        "pending_rewrite_directives": pending_directives,
        // #277：过门审计面（断点恢复时可见——agent 声明过的门带警告导出；
        // human_review_complete=false 时 export_bid_document 正文插警告横幅）
        "agent_waivers": agent_waivers_json(svc),
        "human_review_complete": svc.store().human_review_complete(),
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

/// JSON 值的类型名（参数校验错误信息用——报「实际类型」比报「非法」可定位）。
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
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
    let project_version = opt_cas_version(&args, "import_tender_files")?;
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
                "kind": svc.store().files().iter().find(|f| f.id == *fid)
                    .map(|f| f.kind.as_str()).unwrap_or(""),
                "sha256": o.sha256,
                "pages": o.pages,
                "tables": o.tables,
                "chunks": o.chunks,
                "ocr_tier": o.ocr_tier,
                "parse_status": parse_status_str(o.parse_status),
                "deduplicated": o.deduplicated,
            });
            // #218 乱码可见性：解析产物自报透出回执（兜底路的 parse_summary
            // 含 CJK 占比与「疑似乱码」标注——文本层质量差的文件 Agent 一眼
            // 可见，不用等条款抽取 0 条再回头排查根因）。
            if let Some(tf) = svc.store().files().iter().find(|f| f.id == *fid) {
                if !tf.parse_summary.is_empty() {
                    entry["parse_summary"] = json!(tf.parse_summary);
                }
            }
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
    // 已在上面 ADDENDUM_NOT_READY 点名的件（file_id 集）——输入完整性告警
    // 跳过它们：同一件事不报两遍（补遗面已有专门的语义化告警）。
    let addendum_warned: std::collections::HashSet<String> = addendum_readiness["not_ready"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter(|s| !s["waived"].as_bool().unwrap_or(false))
        .filter_map(|s| s["file_id"].as_str().map(str::to_string))
        .collect();
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
    // ---- 地图 P0-1：输入完整性接线（主件/附件/图纸坏件的第一现场）----
    // 与 #184 补遗面同构：import 响应给出逐件不就绪清单 + 指路告警，
    // Agent 拿 file_id 即可闭环（重传或 record_addendum_waiver——该确认
    // 面已覆盖全部招标包输入件）。与 draft 拒绝理由同一真源
    // （readiness::input_integrity），不是第二套判定。
    let integrity = bid_tools::readiness::input_integrity(svc.store());
    for f in &integrity.unready {
        if addendum_warned.contains(&f.file_id) {
            continue; // 补遗已由 ADDENDUM_NOT_READY 点名——同一件事不报两遍
        }
        warnings.push(format!(
            "INPUT_FILE_NOT_READY: {} 未就绪（{}）——「已提取出一部分需求」不等于输入完整，起草将被拒；请修复重传或人工确认后放行（record_addendum_waiver）",
            f.name, f.reason,
        ));
    }
    // ---- #271：格式件清单（kind=格式模板 的库内登记——附件/ 下官方模板
    // 逐件可见，Agent 起草时可按 file_id 读模板结构；与 addendum_readiness
    // 同级形态。模板解析失败仍在清单内可见（parse_status），但不阻塞
    // 起草（readiness 不在面）。
    let forms_items: Vec<Value> = svc
        .store()
        .files()
        .iter()
        .filter(|f| f.kind == "格式模板")
        .map(|f| json!({
            "file_id": f.id, "name": f.name,
            "parse_status": parse_status_str(f.parse_status),
            "pages": f.pages, "tables": f.tables,
        }))
        .collect();
    let forms_manifest = if forms_items.is_empty() {
        Value::Null
    } else {
        json!({
            "count": forms_items.len(),
            "items": forms_items,
        })
    };
    let mut data = json!({
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
        // 地图 P0-1：项目级输入完整性（主件/补遗/附件全覆盖——逐件
        // unready/waived_unready，Agent 拿 file_id 即可闭环）
        "input_integrity": {
            "complete": integrity.complete,
            "restricted": integrity.restricted(),
            "unready": integrity.unready.iter().map(|f| json!({
                "file_id": f.file_id, "name": f.name,
                "readiness": f.readiness.as_str(), "reason": f.reason,
            })).collect::<Vec<_>>(),
            "waived_unready": integrity.waived_unready.iter().map(|f| json!({
                "file_id": f.file_id, "name": f.name,
                "readiness": f.readiness.as_str(), "reason": f.reason,
            })).collect::<Vec<_>>(),
        },
        "status": status_after,
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
        "warnings": warnings,
    });
    // #271：格式件清单条件透出（无格式件时键省略——黑龙江形态零扰动）
    if !forms_items.is_empty() {
        data["forms_manifest"] = forms_manifest;
    }

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
    let project_version = opt_cas_version(&args, "extract_requirements")?;

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
    // #196：按【业务 kind】分发，不按文件名前缀。
    // kind 来自 store 里的 TenderFile（导入时登记：招标文件/补遗/附件/图纸/证据），
    // 这样「光纤租赁费招标文件.docx」这类真实文件名也能正常抽取。
    let kind_map: std::collections::HashMap<String, String> = svc
        .store()
        .files()
        .iter()
        .map(|f| (f.name.clone(), f.kind.clone()))
        .collect();
    let reqs = bid_parse::extract::extract_requirements_with_kind(svc.parsed(), |name| {
        kind_map.get(name).cloned()
    });

    // ---- #219 第二步：抽取通路合并（Agent 标记优先 + 正则兜底补漏）
    //
    // 规则：被任何区间标记覆盖的块，正则产物丢弃（Agent 已看过该块并给出
    // 结论——无论 noise/rule/review_table/requirement）；未覆盖块正则保留
    // （兜底）；Agent 提交的关键点转 REQ-Axxxx 追加（冲突以 Agent 为准）。
    //
    // **无标记时严格 no-op**（GT 红线保护）：空 marks + 空 submitted →
    // 输出与旧正则产物逐位一致（real_corpus_e2e 只走正则全链，零位移）。
    // B3（无抽取面）不进合并面——uncovered 语义 = 「抽取面的未覆盖」。
    let tender_files: Vec<(String, String)> = svc
        .store()
        .files()
        .iter()
        .map(|f| (f.id.clone(), f.sha256.clone()))
        .collect();
    // #250：合并面门禁与抽取分发同口径——按【业务 kind】判定，不按文件名
    // 前缀（#196 同根因的第二处修复）。真实用户文件（无 A- 前缀）不再被
    // filter 掉——67 条 Agent 提交与 76 个区间标记的合并消费恢复。kind
    // 缺省（parsed 文件不在 store 登记面）时前缀兜底（夹具约定）。
    let faces: Vec<bid_parse::merge::MergeFace> = svc
        .parsed()
        .iter()
        .filter(|p| {
            bid_parse::merge::has_extraction_face(
                kind_map.get(&p.name).map(String::as_str),
                &p.name,
            )
        })
        .filter_map(|p| {
            let fid = tender_files
                .iter()
                .find(|(_, sha)| sha == &p.sha256)
                .map(|(fid, _)| fid.as_str())?;
            Some(bid_parse::merge::MergeFace {
                file_id: fid,
                name: &p.name,
                blocks: &p.blocks,
                marks: svc.store().block_marks_of(fid),
                submitted: svc.store().submitted_requirements_of(fid),
            })
        })
        .collect();
    let (mut reqs, merge_stats) = bid_parse::merge::merge_extraction(&faces, reqs);
    drop(faces);

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

    // ---- 冲突检测（最小面）：补遗与主文件同条款号不同文本 → 双向登记
    let mut conflicts: Vec<Value> = Vec::new();
    let main_names: Vec<&str> = svc.parsed().iter().map(|p| p.name.as_str()).collect();
    // 补遗件名集合（#425：按业务 kind 判定——kind==补遗；kind 缺省时前缀
    // 兜底（夹具约定）。#196/#250 同根因的第三处收口：真实命名的补遗
    // （「澄清修改文件.pdf」/「补遗通知.docx」）不再漏检冲突。kind 在值域
    // 的其他件（答疑/评分办法/附件…）不误入补遗面。）
    let addenda_names: Vec<String> = svc
        .parsed()
        .iter()
        .filter(|p| match kind_map.get(&p.name).map(String::as_str) {
            Some("补遗") => true,
            Some(_) => false,
            None => p.name.starts_with("C-"),
        })
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

    // ---- #271：格式件与 REQ 关联（尽力而为）——格式件名（去扩展名、≥4 字
    // 守卫）出现在 REQ 锚文里 → params.form_file_id / form_name。条款真源
    // 始终是主件格式章 REQ（「按格式N填写技术偏离表」），关联让起草面能
    // 直接定位官方模板（get_document_chunks 读结构），不再全库猜。
    // 匹配不到就不关联（best-effort，不失败）。
    let form_files: Vec<(String, String)> = svc
        .store()
        .files()
        .iter()
        .filter(|f| f.kind == "格式模板")
        .map(|f| (f.id.clone(), f.name.clone()))
        .collect();
    let mut forms_linked = 0usize;
    if !form_files.is_empty() {
        for r in reqs.iter_mut() {
            let anchor_text: String =
                r.anchors.iter().map(|a| a.excerpt.as_str()).collect();
            let mentions_form = r.category == "格式要求"
                || anchor_text.contains("格式")
                || anchor_text.contains("偏离表")
                || anchor_text.contains("一览表")
                || anchor_text.contains("声明函");
            if !mentions_form {
                continue;
            }
            for (fid, fname) in &form_files {
                let stem = fname.rsplit_once('.').map(|(s, _)| s).unwrap_or(fname);
                if stem.chars().count() >= 4 && anchor_text.contains(stem) {
                    r.params
                        .insert("form_file_id".to_string(), json!(fid));
                    r.params
                        .insert("form_name".to_string(), json!(fname));
                    forms_linked += 1;
                    break;
                }
            }
        }
    }

    // ---- #285 ① ★漏标检测：字面★真值 vs 已抽 REQ（抽取面汇总 + 警告）。
    //
    // 口径与 get_star_coverage 同源（star_scan 三级恒等式）；此处只汇总
    // 抽取面文件（has_extraction_face 同口径——格式模板/证据件不进），
    // 逐文件明细用 get_star_coverage(project_id, file_id) 查。
    // 纯读计算（不落库不 bump）——REQ 产物零位移（GT 红线保护）。
    let (mut star_total, mut star_decl, mut star_captured, mut star_missing) =
        (0usize, 0usize, 0usize, 0usize);
    for p in svc.parsed().iter().filter(|p| {
        bid_parse::merge::has_extraction_face(
            kind_map.get(&p.name).map(String::as_str),
            &p.name,
        )
    }) {
        let cov = bid_parse::star_scan::star_coverage(p, &reqs);
        star_total += cov.truth_stars;
        star_decl += cov.declaration_stars;
        star_captured += cov.captured_stars;
        star_missing += cov.missing_stars;
    }

    // ---- 落库（#219 二步：整集替换 + 状态转换 FILES_IMPORTED →
    // REQUIREMENTS_EXTRACTED；替换而非只增合并——「先 extract → Agent
    // 标 noise → 重跑 extract」时被覆盖块上的旧正则 REQ 必须消失。
    // T-8：经服务层包装——持久化在场时写后自动落盘）
    let before_total = svc.store().requirements().len();
    let total = svc.replace_requirements(reqs).map_err(ToolError::from)?;
    let added = total as isize - before_total as isize;

    // ---- counts 组装
    let mut by_category: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for r in svc.store().requirements() {
        *by_category.entry(category_key(&r.category)).or_insert(0) += 1;
    }
    let mut warnings: Vec<String> = Vec::new();
    if forms_linked > 0 {
        warnings.push(format!(
            "FORMS_LINKED: {forms_linked} 条格式类 REQ 已关联官方格式模板（params.form_file_id——起草时可经 get_document_chunks 读取模板结构，#271）"
        ));
    }
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
    // #285 ① ★漏标警告（不是 BLOCKER——可能误报，逐文件核实面在
    // get_star_coverage；样板句已排除，误报面已收敛到位置对账）
    if star_missing > 0 {
        warnings.push(format!(
            "STAR_MISSING: ★漏标警告：{star_missing} 处★未见对应 REQ——用 get_star_coverage 查看后补标",
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
    // #219 二步：未覆盖块提醒（正则已兜底——Agent 补标后重跑更准）
    if merge_stats.uncovered_blocks > 0 {
        warnings.push(format!(
            "UNCOVERED_BLOCKS: {} 块未标记（正则已兜底补漏——用 get_coverage_report 查看后 submit_block_marks 补标，重跑 extract 更准）",
            merge_stats.uncovered_blocks
        ));
    }
    // Agent 标必选且类别不在类别阻断面（资格条件/否决项）——这些条款经
    // matrix.rs `agent_mandatory` 接线进入矩阵 blocking 面（未响应即拦
    // 起草/导出）。信息性提示：告诉 Agent 这些标记现在有阻断力，重跑
    // build_matrix 后生效，响应（绑证据/覆写）后放行。
    if merge_stats.agent_mandatory_offcategory > 0 {
        warnings.push(format!(
            "AGENT_MANDATORY_OFFCATEGORY: {} 条 Agent 标必选且类别不在资格/否决阻断面——已进入矩阵 blocking 面（重跑 build_matrix 后未响应将拦起草/导出；响应或人工 receipt 覆写后放行）",
            merge_stats.agent_mandatory_offcategory
        ));
    }

    // 【#196 纪律补齐】抽取为 0 条必须响亮告警，不能静默「成功」。
    // 旧行为：counts.total=0 + ok:true + 状态推进 → 下游全部 E_STATE，
    // 而调用方以为自己「抽取完成」——违反 issue #9「缺失不得伪装成功」。
    if total == 0 {
        let names: Vec<&str> = svc.parsed().iter().map(|p| p.name.as_str()).collect();
        warnings.push(format!(
            "ZERO_REQUIREMENTS: 抽取结果为 0 条（输入 {} 个文件：{:?}）——下游建纲/矩阵/起草/导出将全部被拒。常见原因：① 文件 kind 未登记为「招标文件」；② 文本层损坏或为扫描件（需 OCR）；③ 文档不含可识别的条款结构。这不是「抽取完成」，请先排查输入面。",
            names.len(),
            names
        ));
    }
    Ok(json!({
        "counts": {
            "total": total,
            "added": added,
            "by_category": by_category,
        },
        "merge": {
            "regex_total": merge_stats.regex_total,
            "regex_dropped_covered": merge_stats.regex_dropped_covered,
            "agent_requirements": merge_stats.agent_requirements,
            "uncovered_blocks": merge_stats.uncovered_blocks,
            "coverage_complete": merge_stats.coverage_complete,
        },
        // #285 ① ★漏标检测汇总（抽取面文件口径；total 为真值★——样板句
        // 已排除，captured + missing == total 恒成立；逐文件明细走
        // get_star_coverage）
        "star_summary": {
            "total": star_total,
            "captured": star_captured,
            "missing": star_missing,
            "declarations": star_decl,
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
    // 游标窗口（S-4）：fp 绑定 工具名+project_id+过滤参数——跨过滤器游标 fail-closed 拒；
    // cv 绑定当前数据版本（#406/C-14）——翻页期间 store 变更（增长位移）即拒
    let fp = cursor::fingerprint(&[
        Some("get_compliance_matrix"),
        Some(project_id.as_str()),
        filter_category.as_deref(),
        filter_status.as_deref(),
    ]);
    let cv = svc.store().content_version();
    let window = cursor::resolve(&args, "get_compliance_matrix", &fp, cv)?;

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
    let page = cursor::slice(&window, &fp, cv, total_rows)?;

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
                    // 同源真源：类别阻断面（is_mandatory）∪ Agent 必选标记
                    // （agent_mandatory）——与 matrix.rs blocking 派生同一判定面，
                    // 工具面 mandatory 即本门输入（不得在此另写一套字面量）。
                    "mandatory": bid_domain::matrix::is_mandatory(&r.category)
                        || bid_domain::matrix::agent_mandatory(r),
                    "source_anchor": anchor.map(|a| json!({
                        "file": a.file, "kind": a.kind, "locator": a.locator, "excerpt": a.excerpt,
                    })),
                    "params": r.params,
                },
                // #245：PENDING_EVIDENCE（仅占位证据在绑——待补可见不清阻塞）
                // 随 status 面自然回显；pending_evidence_ids 是待补追踪面。
                "status": status,
                "evidence": evidence,
                "pending_evidence_ids": row.map(|m| m.pending_evidence_ids.clone()).unwrap_or_default(),
                // #241 裁定：证据要求与缺口正交标记（与 status 分账——正文
                // 响应是文本事实，缺不缺材料是合规事实，两列各自可查）。
                "evidence_required": row.map(|m| m.evidence_required).unwrap_or(false),
                "evidence_missing": row.map(|m| m.evidence_missing).unwrap_or(false),
                // #276：★实质性条款（识别列——不参与 blocking 派生；统计口见
                // star_summary。★响应 N/M 从外部文本刮变为引擎可编程）
                "substantive": row.map(|m| m.substantive).unwrap_or(false),
                "gap": Value::Null,
                "response_policy": "default",
                "draft_section_id": row.and_then(|m| m.response_section.clone()),
                "blocking": row.map(|m| m.blocking).unwrap_or(false),
            })
        })
        .collect();

    // #276：★实质性条款统计口（全量口径——不受 filters 影响，验收面
    // 「★全部识别并响应（对照 manifest.star_count）」的可编程基础）。
    // responded = RESPONDED（含人工覆写不适用？否——NOT_APPLICABLE 单列，
    // 不计入 responded：不适用是免除响应而非满足响应，验收上应单独对账）。
    let mut star_total = 0usize;
    let mut star_responded = 0usize;
    let mut star_partial = 0usize;
    let mut star_not_responded = 0usize;
    let mut star_not_applicable = 0usize;
    for r in reqs {
        if !bid_domain::matrix::substantive(r) {
            continue;
        }
        star_total += 1;
        let st = row_by_req
            .get(r.id.as_str())
            .map(|m| m.status)
            .unwrap_or(ResponseStatus::NotResponded);
        match st {
            ResponseStatus::Responded => star_responded += 1,
            ResponseStatus::Partial => star_partial += 1,
            ResponseStatus::PendingEvidence => star_not_responded += 1,
            ResponseStatus::NotResponded => star_not_responded += 1,
            ResponseStatus::NotApplicable => star_not_applicable += 1,
        }
    }
    let star_summary = json!({
        "total": star_total,
        "responded": star_responded,
        "partial": star_partial,
        "not_responded": star_not_responded,
        "not_applicable": star_not_applicable,
        // ★响应率（responded/total，无★时为 null——不是 0/0 假象）
        "responded_share": if star_total > 0 {
            json!((star_responded as f64) / (star_total as f64))
        } else {
            Value::Null
        },
        "note": "★实质性条款（params.star）——一条不响应即响应无效；not_responded>0 时不废标风险存在",
    });

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
        // #276：★实质性条款统计口（全量口径——见上方注释）
        "star_summary": star_summary,
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
    let cv = svc.store().content_version();
    let window = cursor::resolve(&args, "list_evidence", &fp, cv)?;

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
    let page = cursor::slice(&window, &fp, cv, total)?;
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
                // #217：内容验证四态（台账清单可见「正文是否已被读取」）
                "verification_tier": e.verification_tier.map(|t| t.as_str()),
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
    let cv = svc.store().content_version();
    let window = cursor::resolve(&args, "list_tasks", &fp, cv)?;

    // ---- 项目路由（只读：任何阶段可调）
    route_project(svc, &project_id, "list_tasks")?;

    // ---- 过滤（domain 双过滤 + target_id 字典序）+ 切片 + 行组装
    let matched = svc.store().list_tasks(f_owner.as_deref(), status);
    let total = matched.len();
    let page = cursor::slice(&window, &fp, cv, total)?;
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

/// 朴素 chunk 文案（pre-M2-b：整件为一块——title/scope/subject/client +
/// #217 正文摘录直通拼接；M2-b chunk 化后由真实分块文本承载）。
fn evidence_content(ev: &bid_domain::evidence::Evidence) -> String {
    let mut parts: Vec<&str> = [
        Some(ev.title.as_str()),
        Some(ev.scope.as_str()),
        Some(ev.subject_name.as_str()),
        ev.client.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.trim().is_empty())
    .collect();
    // #217：正文摘录进检索面（搜「正文含词但标题不含」的证据能命中）
    if let Some(ex) = ev
        .excerpt
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        parts.push(ex);
    }
    parts.join("；")
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
                // #217：正文摘录面（检索命中可直接看实料——非标题转写）
                "excerpt": e.excerpt,
                "excerpt_locator": e.excerpt_locator,
                "verification_tier": e.verification_tier.map(|t| t.as_str()),
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
            "content_face": "title+scope+subject+client+excerpt（#217：正文摘录进检索面）",
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
///
/// #374 围栏（R-27/C-06）：file_path 经 fs_fence（BID_WS / persist /
/// BID_FS_ROOT 围栏根——与 import 的 #183 壳层围栏同一真源）：无根响亮拒；
/// 围栏外 / `..` / symlink 段拒（先于存在性检查——不泄露存在性 oracle）；
/// find_root 的结果必须仍在围栏内（防围栏外祖先 `.mang` 牵引快照越界落盘）。
fn save_version(svc: &BidService, args: Value) -> ToolResult {
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
    // ---- #374 围栏（R-27/C-06）：此前本工具是工具面唯一「绝对路径+
    //      无根校验」的写盘口——Agent 可对围栏外任意可读文本（如
    //      /home/user/notes/secret.md）做快照并在其目录家族落
    //      .mang/history（文件存在性 oracle + 围栏外写入）。对照 import
    //      的 #183 壳层围栏纪律（fs_fence 单一真源）；桌面壳不经工具面
    //      （mang-app snapshots.rs 直调 mang_snap 库），本收紧零产品面影响。
    //      围栏先于存在性检查：围栏外路径统一报围栏错误，不泄露存在性
    //      （关闭 oracle）；围栏内缺失件仍走下方 E_NOTFOUND（import 同款
    //      「缺失件不在围栏层抢答」语义）。
    let fs_roots = svc.fs_roots();
    if fs_roots.is_empty() {
        return Err(ToolError::EUnprocessable {
            what: "save_version 需要工作区（设 BID_WS 环境变量，或用 BidService::open 打开持久化工作区，或 BID_FS_ROOT 声明受信根）——无围栏根则文件面工具不可用（R-27：绝对路径无根校验的写盘口已收口）".into(),
        });
    }
    let fenced = bid_tools::fs_fence::resolve_read_for("save_version", &fs_roots, &path)?;
    let fenced_path = fenced.absolute().to_path_buf();
    if !fenced_path.is_file() {
        return Err(ToolError::ENotFound {
            what: format!("save_version: 文件不存在（{file_path}）"),
        });
    }
    // root 围栏复验：find_root 向上找最近的 .mang 无上界——围栏内文件的
    // 祖先（围栏外）若存在 .mang，root 会越出围栏，快照落到围栏外工作区。
    let root = mang_snap::find_root(&fenced_path);
    if !fs_roots
        .iter()
        .any(|r| bid_tools::fs_fence::is_inside(&root, r))
    {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "save_version: 工作区根发现越界（{} 在围栏外——祖先 .mang 不得牵引快照落到围栏外）",
                root.display()
            ),
        });
    }
    // mang-snap 失败码（empty/too_large/denied/io）都是调用方可理解的面——
    // 契约枚举无 E_INTERNAL，统一 E_UNPROCESSABLE 并在 what 里带核心层码位
    let saved = mang_snap::save(&root, &fenced_path, mang_snap::Kind::Ai, message).map_err(|e| {
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
    let project_version = opt_cas_version(&args, TOOL)?;
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
        // 地图 P0-1：输入完整性面——COMPLETE / RESTRICTED。受限起草 =
        // 有坏件经逐件人工确认放行（放行 ≠ 没问题，机器面显式标注，
        // 不得静默当成完整输入）；waived_files 逐件点名确认来源。
        "input_integrity": {
            "status": if outcome.input_integrity.restricted() { "RESTRICTED" } else { "COMPLETE" },
            "waived_files": outcome.input_integrity.waived_unready.iter().map(|f| json!({
                "file_id": f.file_id,
                "name": f.name,
                "readiness": f.readiness.as_str(),
                "reason": f.reason,
            })).collect::<Vec<_>>(),
        },
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

/// #241 裁定：提交就绪清单项的契约 out 组装（validate / check_bid_quality
/// 共用——单一出口，两工具的 submission_readiness 面永不分叉）。
fn readiness_item_json(i: &bid_tools::validate::ReadinessItem) -> Value {
    json!({
        "requirement_id": i.requirement_id,
        "row_id": i.row_id,
        "category": i.category,
        "excerpt": i.excerpt,
        "message": i.message,
    })
}

/// #241 裁定：提交就绪面的契约 out 组装。
///
/// 门禁分工（Export ≠ Submission Readiness）：导出可过而提交未就绪是
/// **合法状态**（草稿可导出——证据缺口不拦导出；但缺口未清不得正式
/// 提交）。消费方（Agent/Human）按 evidence_missing 清单逐项补件。
fn submission_readiness_json(svc: &BidService) -> Value {
    let r = bid_tools::validate::submission_readiness(svc.store());
    json!({
        "ready": r.ready,
        "not_built": r.not_built,
        "blockers": r.blockers(),
        "unresponded_count": r.unresponded.len(),
        "evidence_missing_count": r.evidence_missing.len(),
        "unresponded": r.unresponded.iter().map(readiness_item_json).collect::<Vec<_>>(),
        "evidence_missing": r.evidence_missing.iter().map(readiness_item_json).collect::<Vec<_>>(),
    })
}

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
        // #241 裁定：提交就绪面（evidence_missing 清单供逐项补件——导出可过
        // 而未就绪是合法状态，两分账在响应面同场可见）
        "submission_readiness": submission_readiness_json(svc),
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
/// #186 语义降级：**规则诊断，非评标模拟**——tier 是关键词覆盖/长度/证据词
/// 存在性的启发式档位（`diagnostic_only=true, result_kind=rule_heuristic`），
/// 不代表评委会会给的档。item_scores 透传 `score_rules` 细则原文 +
/// `rule_checkpoints` 关键条件比对结果（#224 第 3 步规则层可见性——提及
/// 性标记不改判档/估分）；完整语义判档依赖模型。
///
/// 壳层职责：参数校验 → route_guard（只读诊断，MatrixReady+ 窗口）→ 前置
/// （章节在场 + 已起草——无正文的自评是空转）→ B2 [`SectionScorer`] 只调用
/// 不改 → out 组装（item_scores 逐项明细 + rationale 可追溯面）。
fn score_section(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "score_section";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let section_id = req_str(&args, "section_id", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;
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
    let (scorer, _items, _ledger_effective) = build_scorer(svc.store());
    let s = scorer.score_section(&section_id, &content);
    // #224：文档级未映射告警（单章自评时也可见——Agent 需知道全局缺口）
    let unmapped = scorer.unmapped_items();
    Ok(json!({
        "project_id": project_id,
        // #186 语义降级标记：规则诊断非评标模拟（对外承诺必须准确——
        // external Agent 不得把 tier 理解为评委会会给的档）
        "diagnostic_only": true,
        "result_kind": "rule_heuristic",
        "section_id": s.section_id,
        "section_title": node.title,
        "chars": content.chars().filter(|c| !c.is_whitespace()).count(),
        "estimated": s.estimated,
        "max": s.max,
        "covered_items": s.covered_items,
        "missing_items": s.missing_items,
        "rationale": s.rationale,
        "item_scores": s.item_scores,
        // #224：未映射评分项（非空 = 总分可能偏低——响亮告警）
        "unmapped_scoring_items": unmapped,
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
///
/// #186 语义降级：**规则诊断，非评标模拟**——分数与 tier 来自关键词覆盖/
/// 长度阈值/证据词存在性启发式（`diagnostic_only=true,
/// result_kind=rule_heuristic`），不代表评委会评分。by_item 透传 `score_rules`
/// 细则原文与 `rule_checkpoints` 关键条件比对结果（#224 第 3 步——规则层
/// 可见性，SCORE_RULES_CHECKPOINTS note 登记；判档/估分不变，完整语义判档
/// 依赖模型）。
fn score_document(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "score_document";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;
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
    let (scorer, items, ledger_effective) = build_scorer(svc.store());
    let doc = scorer.score_document(&sections, price_input);
    // ---- by_item + target：全量非硬门可达叶（叶 = 未被任何项引用为 parent）
    let parent_ids: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|i| i.parent_id.as_deref())
        .collect();
    // #224 映射报告（验收 1/4：每评分项 → 哪些章节 → 权重多少 → 依据原文哪里；
    // 双向追溯的桥接面——RuleParsed 派生映射锚回 REQ 首锚，账本映射带
    // source_anchor 原文锚 + 来源 + 证据引用）
    let req_anchors: std::collections::HashMap<&str, &bid_domain::Anchor> = svc
        .store()
        .requirements()
        .iter()
        .filter_map(|r| r.anchors.first().map(|a| (r.id.as_str(), a)))
        .collect();
    let ledger: std::collections::HashMap<(&str, &str), &bid_domain::scoring::ScoringMapping> =
        svc.store()
            .scoring_mappings()
            .iter()
            .map(|m| ((m.scoring_req_id.as_str(), m.section_id.as_str()), m))
            .collect();
    let section_title: std::collections::HashMap<&str, &str> = svc
        .store()
        .sections()
        .iter()
        .map(|n| (n.id.as_str(), n.title.as_str()))
        .collect();
    let anchor_json = |a: &bid_domain::Anchor| {
        json!({
            "file": a.file,
            "kind": a.kind,
            "locator": a.locator,
            "excerpt": truncate(&a.excerpt, 80),
        })
    };
    let mut mappings_report: Vec<Value> = Vec::new();
    for it in items.iter().filter(|i| !i.is_hard_gate) {
        if it.mappings.is_empty() {
            continue;
        }
        let mut secs: Vec<Value> = Vec::new();
        for m in &it.mappings {
            // #224 归因（Lead 审查 2）：只认**生效**映射的来源——账本行与派生
            // params.section 同节时被 build_scorer 去重（派生赢、账本行
            // inert），此处若仍按 (req, section) 回查账本，会把 inert 行的
            // source/anchor/evidence 误归因给派生映射。生效键集
            // （ledger_effective）在联结时同步登记，与去重规则同源。
            let ledger_m = if ledger_effective
                .contains(&(it.requirement_id.clone(), m.section_id.clone()))
            {
                ledger.get(&(it.requirement_id.as_str(), m.section_id.as_str()))
            } else {
                None // 派生映射（或防御分支：inert 账本行不归因）
            };
            let (source, anchor, evidence_refs) = match ledger_m {
                Some(lm) => (
                    serde_json::to_value(lm.created_from).unwrap_or(Value::Null),
                    anchor_json(&lm.source_anchor),
                    json!(lm.evidence_refs),
                ),
                // RuleParsed 派生映射：锚回 REQ 首锚（规则原文的真源）
                None => (
                    json!("RULE_PARSED"),
                    req_anchors
                        .get(it.requirement_id.as_str())
                        .map(|a| anchor_json(a))
                        .unwrap_or(Value::Null),
                    json!([]),
                ),
            };
            secs.push(json!({
                "section_id": m.section_id,
                "section_title": section_title.get(m.section_id.as_str()).copied().unwrap_or(""),
                "weight_share": m.weight_share,
                "source": source,
                "source_anchor": anchor,
                "evidence_refs": evidence_refs,
            }));
        }
        mappings_report.push(json!({
            "item_id": it.requirement_id,
            "criteria": it.criteria,
            "weight": it.weight,
            "sections": secs,
        }));
    }
    let mut by_item: Vec<Value> = Vec::new();
    let mut target = 0.0f64;
    for it in items
        .iter()
        .filter(|i| !i.is_hard_gate && !parent_ids.contains(i.requirement_id.as_str()))
    {
        let max = it.max_score.unwrap_or(0.0);
        target += max;
        // #224：section_id = 首映射（契约 §15 旧键保留）；mapping_ids = 全部
        // 映射章节（多对多新面）；weight_share = 首映射摊分比
        let primary = it.mappings.first();
        if is_price_item_mirror(it) {
            // 价格类叶：内容自评无意义（价格分 = f(报价)）——estimated 取 B1 实算值
            by_item.push(json!({
                "item_id": it.requirement_id,
                "criteria": it.criteria,
                "max": max,
                "estimated": doc.price,
                "tier": "价格分（f(报价)——score_price/B1 计算）",
                "coverage": Value::Null,
                "section_id": primary.map(|m| m.section_id.clone()),
                "mapping_ids": it.mappings.iter().map(|m| m.section_id.clone()).collect::<Vec<_>>(),
                "weight_share": primary.map(|m| m.weight_share),
                // 价格分不逐章摊分（f(报价)）——行形状与非价格行保持一致
                "contributions": [],
                "score_rules": it.score_rules,
                // 价格类不比对细则检查点（价格分 = f(报价)，无正文面）
                "rule_checkpoints": [],
                "rule_gaps": 0,
            }));
            continue;
        }
        // #224 多对多聚合（Lead 审查 1）：estimated = 该评分项在**全部映射
        // 章节**的份额贡献之和（与 total 的计入口径同源——此前 .find 只取
        // 首章贡献，多映射时 by_item 低估且与 total 不一致）。
        // tier：单贡献照旧；多贡献同档 → 该档；异档 → 「多章混合」+ 逐档
        // 列举（诚实聚合——档位是逐章判档的产物，跨章不虚构统一档）。
        // coverage：份额加权均值（各章覆盖度按其承载份额加权——单映射
        // 退化为原值）。逐章明细新开 contributions 键（章节/估分/档/覆盖/
        // 摊分比），by_item 自包含、不必回翻 breakdown。
        let contribs: Vec<(String, &bid_domain::scoring::ItemScore)> = doc
            .breakdown
            .iter()
            .flat_map(|s| {
                s.item_scores
                    .iter()
                    .map(move |isc| (s.section_id.clone(), isc))
            })
            .filter(|(_, isc)| isc.item_id == it.requirement_id)
            .collect();
        let estimated =
            (contribs.iter().map(|(_, e)| e.estimated).sum::<f64>() * 100.0).round() / 100.0;
        let tier_label = if contribs.is_empty() {
            "未覆盖".to_string()
        } else {
            let first = contribs[0].1.tier.label();
            if contribs.iter().all(|(_, e)| e.tier.label() == first) {
                first.to_string()
            } else {
                format!(
                    "多章混合（{}）",
                    contribs
                        .iter()
                        .map(|(_, e)| e.tier.label())
                        .collect::<Vec<_>>()
                        .join("+")
                )
            }
        };
        let coverage = {
            let w: f64 = contribs.iter().map(|(_, e)| e.weight_share).sum();
            if w > 1e-9 {
                (contribs.iter().map(|(_, e)| e.coverage * e.weight_share).sum::<f64>() / w
                    * 10000.0)
                    .round()
                    / 10000.0
            } else {
                0.0
            }
        };
        // #224 第 3 步：细则检查点聚合面（跨章任一章提及即提及——合并语义
        // 的单一真源在 bid_domain::scoring::merged_rule_checkpoints，与
        // score_document notes 缺口计数同口径）。未映射/未提供正文章节 →
        // 无比对面，检查点空集（诚实——无正文不虚构比对结果）。
        let cp_rows: Vec<&bid_domain::scoring::ItemScore> =
            contribs.iter().map(|(_, e)| *e).collect();
        let merged = bid_domain::scoring::merged_rule_checkpoints(&cp_rows);
        let rule_gaps = merged.iter().filter(|c| !c.mentioned).count();
        let rule_checkpoints: Vec<Value> = merged
            .iter()
            .map(|c| {
                json!({
                    "text": c.text,
                    "hits": [c.hits.0, c.hits.1],
                    "mentioned": c.mentioned,
                })
            })
            .collect();
        by_item.push(json!({
            "item_id": it.requirement_id,
            "criteria": it.criteria,
            "max": max,
            "estimated": estimated,
            "tier": tier_label,
            "coverage": coverage,
            "section_id": primary.map(|m| m.section_id.clone()),
            // #224 多对多面：全部映射章节 + 首映射摊分比；estimated 为逐章
            // 贡献之和（contributions 逐章明细——多章不取首章）
            "mapping_ids": it.mappings.iter().map(|m| m.section_id.clone()).collect::<Vec<_>>(),
            "weight_share": primary.map(|m| m.weight_share),
            "contributions": contribs.iter().map(|(sec, e)| json!({
                "section_id": sec,
                "estimated": e.estimated,
                "tier": e.tier.label(),
                "coverage": e.coverage,
                "weight_share": e.weight_share,
                "rule_gaps": e.rule_gaps,
            })).collect::<Vec<_>>(),
            // #186 预留面：细则原文透出（#224 第 3 步：检查点比对基准）
            "score_rules": it.score_rules,
            // #224 第 3 步：检查点聚合面（跨章任一章提及即提及——见上方注释）
            "rule_checkpoints": rule_checkpoints,
            "rule_gaps": rule_gaps,
        }));
    }
    let gap = ((target - doc.total) * 100.0).round() / 100.0;
    Ok(json!({
        "project_id": project_id,
        // #186 语义降级标记：规则诊断非评标模拟（对外承诺必须准确——
        // external Agent 不得把 total/tier 理解为评委会会给的分/档）
        "diagnostic_only": true,
        "result_kind": "rule_heuristic",
        "technical": doc.technical,
        // 没报价就不算（issue §3.2：price null 而非伪装 0）
        "price": if bid_price.is_some() { json!(doc.price) } else { json!(null) },
        "commercial": doc.commercial,
        "total": doc.total,
        "target": (target * 100.0).round() / 100.0,
        "gap": gap,
        "by_item": by_item,
        "breakdown": doc.breakdown,
        // #224：映射报告（每评分项 → 章节/权重/来源/原文锚——双向追溯面）
        "mappings": mappings_report,
        // #224：未映射评分项（非空 = 总分可能偏低——响亮告警）
        "unmapped_scoring_items": doc.unmapped_scoring_items,
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
    // #224 追溯面：**生效**账本映射键集 (req_id, section_id)。账本行与派生
    // params.section 同节时被去重（派生赢、账本行 inert 不入 mappings）——
    // 报告只认生效来源：同节 inert 行的 source/anchor/evidence 不得误归因
    // 给派生映射（Lead 审查 2：键集区分「账本生效」与「派生」两路）。
    std::collections::HashSet<(String, String)>,
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
    let mut items = bid_domain::scoring::derive_scoring_items(&reqs);
    let mut tree = bid_domain::scoring::derive_scoring_tree(&reqs);
    // #224 多对多：并入持久化映射（AgentSubmitted/Manual——账本在
    // store.scoring_mappings()，联结进 items/tree.index 两份视图）。
    // 同 (req, section) 键与 params.section 重合时跳过（params 单值 share
    // 恒 1.0——同章重合以账本 share 为准的语义留给后续显式覆写面，本步
    // 保守去重防双重计入）。
    let persistent: Vec<(String, bid_domain::scoring::SectionMapping)> = store
        .scoring_mappings()
        .iter()
        .map(|m| {
            (
                m.scoring_req_id.clone(),
                bid_domain::scoring::SectionMapping {
                    section_id: m.section_id.clone(),
                    weight_share: m.weight_share,
                },
            )
        })
        .collect();
    let apply = |item: &mut bid_domain::scoring::ScoringItem,
                 persistent: &[(String, bid_domain::scoring::SectionMapping)],
                 ledger_effective: &mut std::collections::HashSet<(String, String)>| {
        for (req_id, m) in persistent {
            if *req_id == item.requirement_id
                && !item.mappings.iter().any(|x| x.section_id == m.section_id)
            {
                item.mappings.push(m.clone());
                // 只有真正进 mappings 的账本行才是生效映射（同节被派生
                // 去重的 inert 行不算）——报告归因的判定面。
                ledger_effective.insert((req_id.clone(), m.section_id.clone()));
            }
        }
    };
    let mut ledger_effective: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for it in &mut items {
        apply(it, &persistent, &mut ledger_effective);
    }
    for (_, item) in tree.index.iter_mut() {
        apply(item, &persistent, &mut ledger_effective);
    }
    (
        bid_domain::scoring::SectionScorer::new(tree, terms),
        items,
        ledger_effective,
    )
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
pub(crate) fn opt_str_array(v: &Value, key: &str, at: &str) -> Result<Option<Vec<String>>, ToolError> {
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
    let project_version = opt_cas_version(&args, TOOL)?;
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
    // PDF 通道的环境预检：Word COM 或 LibreOffice 任一可用即可
    // （投标用户常有 Office 无 LO——只查 soffice 会误判）
    if format == "pdf" && !bid_tools::service::pdf_converter_available() {
        return Err(ToolError::EUnprocessable {
            what: "PDF 导出需要 Microsoft Word 或 LibreOffice（均未检测到）——\
                   建议安装 Microsoft Office（转换最快最准）；\
                   或安装 LibreOffice（免费）；\
                   docx 通道不受影响"
                .into(),
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

    // ---- #322/R-05：开标日时效裁决接线（生产锚点 = 导出前——防废标最后关口）。
    // apply_validity 此前全仓生产调用为零（仅测试引用）——时间性过期证据
    // 长期被当 Active 消费。接线语义：
    //   · 提供 bid_open_date → 按开标日单向翻转（翻转后矩阵在 apply_validity
    //     内部重算——R-04，门 1a 读到的必是翻转后状态；翻转非空进 warnings
    //     审计面）；非法日期/非字符串 → 响亮拒（不静默忽略）。
    //   · 未提供 → 有 valid_until 的证据存在时响亮告警「时效未裁决」，
    //     不硬拒（契约兼容：既有调用方不带该参数不应断链）。
    // 幂等面：翻转使 content_version 前进 → 幂等键随之分叉（不同时效态 =
    // 不同导出，不吃旧缓存）；重放时 apply_validity 幂等（同态不动）。
    let bid_open_date: Option<String> = match args.get("bid_open_date") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            if !bid_domain::dates::is_valid_ymd(s) {
                return Err(ToolError::EUnprocessable {
                    what: format!(
                        "{TOOL}: bid_open_date 须为 ISO 日期 YYYY-MM-DD，实际：{s:?}"
                    ),
                });
            }
            Some(s.clone())
        }
        Some(other) => {
            return Err(ToolError::EUnprocessable {
                what: format!(
                    "{TOOL}: bid_open_date 须为 ISO 日期字符串（YYYY-MM-DD），实际类型：{}",
                    json_type_name(other)
                ),
            })
        }
    };
    if let Some(d) = &bid_open_date {
        let flipped = svc
            .store_mut()
            .apply_validity(d)
            .map_err(ToolError::from)?;
        if !flipped.is_empty() {
            warnings_pre.push(format!(
                "证据时效裁决（开标日 {d}）：{} 条翻转——{}",
                flipped.len(),
                flipped.join("; ")
            ));
        }
    } else {
        let with_validity = svc
            .store()
            .evidences()
            .iter()
            .filter(|e| e.valid_until.is_some())
            .count();
        if with_validity > 0 {
            warnings_pre.push(format!(
                "{with_validity} 份证据带 valid_until 但本次导出未提供 bid_open_date——\
                 时效未裁决，按当前登记状态导出（提供 bid_open_date 可按开标日翻转）"
            ));
        }
    }

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
    // anonymize 提前派生（暗标才有；#394/C-07 进幂等键——暗标换投标人名
    // 是不同导出，不能吃旧键缓存返回旧脱敏产物）
    let anonymize = if dark_bid {
        bidder_name.as_deref()
    } else {
        None
    };
    let ikey = format!(
        "export:{project_id}:{}:{format}:{}:{}:{output_path}:{decision_fp}",
        svc.store().content_version(),
        dark_bid,
        anonymize.unwrap_or(""),
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
        // #317 暗标清理报告（dark 导出在场）：blockers 恒 0（复验不过则
        // 导出被拦）；manual_review 是人工确认清单（WARNING 面——不静默跳过）
        "dark_bid": outcome.report.dark_bid.as_ref().map(|r| serde_json::json!({
            "blockers": r.blockers,
            "cleaned": r.cleaned,
            "manual_review": r.post_scan.iter()
                .filter(|h| h.severity == "WARNING")
                .map(|h| serde_json::json!({
                    "location": h.location, "part": h.part,
                    "content": h.content, "action": h.suggested_action,
                }))
                .collect::<Vec<_>>(),
            "pre_scan_count": r.pre_scan.len(),
        })),
        "tables_written": outcome.report.tables_written,
        "table_kinds": outcome.report.table_kinds,
        "forms_written": outcome.report.forms_written,
        "form_titles": outcome.report.form_titles,
        "content_version_snapshot": svc.store().content_version(),
        "project_status": svc.store().project().map(|p| p.status.as_str()).unwrap_or("UNKNOWN"),
        "precheck": { "blockers": 0, "waived": [] },  // 双前置门已过（失败路径在 Err 面）
        // #277 方案 A：agent 声明过门清单 + 人工复核完成度（machine-readable
        // ——导出带警告横幅时，响应面同步携带判据与追认指引）
        "agent_waivers": agent_waivers_json(svc),
        "human_review_complete": svc.store().human_review_complete(),
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
    let project_version = opt_cas_version(&args, TOOL)?;
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
        // #241 裁定：提交就绪面（evidence_missing 清单供逐项补件——ok=true
        // 只表示已执行检查零 BLOCKER，不等于可正式提交；导出可过而提交未
        // 就绪是合法状态，两分账在响应面同场可见）
        "submission_readiness": submission_readiness_json(svc),
        // #80 第三批外部 Agent 实测摩擦 #2：validate 会推进版本（VALIDATING
        // 转换面），响应必须回传新版本号——否则调用方后续带旧版本号 → E_CONFLICT
        // （契约 §0「响应统一携带 project_version」的承诺面）。
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    });
    Ok(out)
}

// ---------------------------------------------------------------- #243 校验状态安全与恢复

/// `recover_validating_state` —— 历史异常态（VALIDATING + 缺章）的受支持恢复。
///
/// 背景（#243）：旧版校验在草稿未完成/校验失败时仍无条件推进 VALIDATING，
/// 且 VALIDATING 态禁起草 → 死锁，用户被迫杀进程手改 db.json。本工具取代
/// 那条不受支持的路径。
///
/// 壳层职责：参数校验（reason 非空白）→ 三件套（路由/guard——仅
/// VALIDATING 态放行/乐观并发）→ 转发 `BidService::recover_validating_state`
///（域层三重约束：仅 VALIDATING / 仅缺章 / 理由非空白；审计账 + 版本 +1 +
/// checkpoint）→ 响应组装（recovery 审计记录 + 新版本号）。
///
/// 重复请求/超时重试语义：恢复成功后项目在 DRAFTING，重试被 guard 拒
///（E_STATE——状态门，非死锁态；起草面照常可用）。并发：MCP 服务串行
/// 处理（&mut 服务态），无交错写。
fn recover_validating_state(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "recover_validating_state";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let reason = req_str(&args, "reason", TOOL)?;
    if reason.trim().is_empty() {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "{TOOL}: reason 不得为空白（审计面：不可审计的恢复不成恢复）——说明为什么需要回到起草面（如：旧版校验在缺章时误推进）"
            ),
        });
    }
    let project_version = opt_cas_version(&args, TOOL)?;
    route_guard(svc, &project_id, TOOL, project_version)?;

    let rec = svc.recover_validating_state(&reason).map_err(ToolError::from)?;
    let out = json!({
        // 恢复后的项目状态（恒 DRAFTING——起草面解锁）
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or("UNKNOWN"),
        // 审计记录（何时/为何/从哪到哪/当时缺哪些章）
        "recovery": {
            "at": rec.at,
            "from": rec.from.as_str(),
            "to": rec.to.as_str(),
            "reason": rec.reason,
            "missing_sections": rec.missing_sections,
        },
        // 契约 §0：响应统一携带 project_version（恢复推进了版本，后续调用
        // 必须用新值，否则 E_CONFLICT）
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    });
    Ok(out)
}

// ================================================================ #80 第三批 MCP 补齐（G1/G2/G3——契约 §10/§11/§12）

/// 项目路由 + 状态机裁决 + 乐观并发（三件套——四个新工具共用面）。
pub(crate) fn route_guard(
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

/// #244 建纲前覆盖闭环门的通过面（信息性回执——零阻断，只记账）。
#[derive(Debug, Clone, Copy, Default)]
struct CoverageGateSummary {
    /// 有抽取面的文件数（附件/图纸/证据/格式模板不在面——与 merge 口径同源）。
    face_files: usize,
    /// 已启用标记流程的文件数（marks 非空）。
    marked_files: usize,
    /// 走正则兜底的文件数（零标记——#219 前的既有合法路径，不在门对象内）。
    regex_fallback_files: usize,
    /// B 态块总数（已排除：noise/rule，理由在 marks.reason 可追溯）。
    excluded_blocks: usize,
}

/// #244 建纲前覆盖闭环门：每个有抽取面的块必居三态之一。
///
/// - **A 已判定为要求**：requirement / review_table 标记（产 REQ / 结构化提取）；
/// - **B 已排除**：noise / rule 标记（理由 `marks.reason`——要求 3 可追溯：
///   块 ID / 文件 / 区间 / 判定来源都在标记集与解析产物里）；
/// - **C 未判定**：压根没标记 → 本门阻断（E_COVERAGE_INCOMPLETE +
///   带块内容摘要的清单——Agent 得知道是什么内容才能判，只给区间号等于没给）。
///
/// **门只拦「半途而废」**：文件标记集非空且存在 C 态块才拦；零标记文件
/// 走正则兜底（#219 前的既有合法路径）——否则 real_corpus_e2e（纯正则
/// 全链）等既有流程被新门卡死（验收 6：新门不能卡死正常流程）。
///
/// **按文件粒度裁决**：「Agent 标了主件、忘了补遗件」时，补遗件零标记
/// → 正则兜底放行（兜底是既有语义），本门只承诺「开始标的必须标完」。
///
/// 零副作用：只读 store + parsed，不落盘、不推版本。
fn coverage_gate(svc: &BidService) -> Result<CoverageGateSummary, ToolError> {
    const TOOL: &str = "build_outline";
    // 每区间展示的块内容摘要条数（够定位 + 不吃爆错误消息）。
    const SAMPLES_PER_GAP: usize = 3;
    // 错误消息里 gap 条数上限（超出汇总——全量用 get_coverage_report 查）。
    const MAX_GAPS_IN_ERROR: usize = 20;

    let kind_map: std::collections::HashMap<&str, &str> = svc
        .store()
        .files()
        .iter()
        .map(|f| (f.name.as_str(), f.kind.as_str()))
        .collect();
    let parsed_by_sha: std::collections::HashMap<&str, &bid_parse::ParsedFile> =
        svc.parsed().iter().map(|p| (p.sha256.as_str(), p)).collect();

    let mut summary = CoverageGateSummary::default();
    let mut gaps: Vec<String> = Vec::new();
    let mut unjudged_total = 0usize;

    for f in svc.store().files() {
        // 与 merge 统计同源的门禁面：无抽取面的文件不进覆盖闭环
        //（附件/图纸/证据/格式模板无条款面，无「遗漏待审」可言）。
        if !bid_parse::merge::has_extraction_face(kind_map.get(f.name.as_str()).copied(), &f.name)
        {
            continue;
        }
        summary.face_files += 1;
        let Some(parsed) = parsed_by_sha.get(f.sha256.as_str()) else { continue };
        let total = parsed.blocks.len();
        let marks = svc.store().block_marks_of(&f.id);
        if marks.is_empty() {
            // 零标记：正则兜底路径（GT 红线保护）——不是本门的对象。
            summary.regex_fallback_files += 1;
            continue;
        }
        summary.marked_files += 1;
        let report = svc.store().coverage_report(&f.id, total);
        summary.excluded_blocks += report.excluded_blocks;
        if !report.has_unjudged() {
            continue; // 标满：A + B 态闭环，放行
        }
        // C 态非零：半途而废——逐区间组装带内容摘要的清单。
        unjudged_total += report.unjudged_blocks;
        for (from, to) in &report.unjudged_ranges {
            if gaps.len() >= MAX_GAPS_IN_ERROR {
                continue;
            }
            let locator = parsed
                .blocks
                .get(*from)
                .map(|b| b.locator.as_str())
                .unwrap_or("");
            let mut gap = format!(
                "  {}（{}）块 [{from}..{to}]（{}）",
                f.name, f.id, if locator.is_empty() { "无定位" } else { locator }
            );
            for b in parsed
                .blocks
                .iter()
                .take(to + 1)
                .skip(*from)
                .take(SAMPLES_PER_GAP)
            {
                gap.push_str(&format!("\n    {}: {}", b.idx, truncate(&b.text, 48)));
            }
            let range_len = to - from + 1;
            if range_len > SAMPLES_PER_GAP {
                gap.push_str(&format!(
                    "\n    ……（区间共 {range_len} 块——get_document_chunks file_id={} from={from} 看全量）",
                    f.id
                ));
            }
            gaps.push(gap);
        }
    }

    if unjudged_total > 0 {
        let extra = report_gap_overflow(gaps.len(), MAX_GAPS_IN_ERROR);
        return Err(ToolError::ECoverageIncomplete {
            unjudged_blocks: unjudged_total,
            gaps: {
                let mut g = gaps;
                if let Some(e) = extra {
                    g.push(e);
                }
                g
            },
        });
    }
    let _ = TOOL; // 错误面自带工具语境（E_COVERAGE_INCOMPLETE 仅建纲门产生）
    Ok(summary)
}

/// gap 数超上限时的汇总尾条（None = 未超）。
fn report_gap_overflow(shown: usize, cap: usize) -> Option<String> {
    (shown >= cap).then(|| {
        format!(
            "  ……（仅列前 {cap} 个区间——get_coverage_report 查全量）"
        )
    })
}

/// `build_outline` —— 契约 §10（G1：建纲 + 矩阵首建，REQUIREMENTS_EXTRACTED 态）。
///
/// 壳层职责：参数校验（spec 可选字段 + 值域）→ 三件套 → 前置（#244 覆盖
/// 闭环门 → 条款面非空）→ 转发 `BidService::prepare_outline_and_matrix`
///（推导大纲树 + 矩阵首建推进 MATRIX_READY + 写后自动落盘）→ 叶/目标
/// 字数统计组装。
///
/// 幂等面：语义位查重在域层 `set_outline`（推导确定性）；MATRIX_READY+ 重复
/// 调用被 guard_tool 拒（E_STATE——重建面属「补遗后增量重抽」的后续需求，
/// 不在首个版本隐式放行）。
fn build_outline(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "build_outline";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;
    let target_total_chars = opt_u64(&args, "target_total_chars", TOOL)?;
    let max_reqs_per_leaf = opt_u64(&args, "max_reqs_per_leaf", TOOL)?;
    // #287 篇幅档位：四档键显式校验（拼错档不得静默落默认档）
    let length_profile = opt_str(&args, "length_profile", TOOL)?;
    if let Some(k) = &length_profile {
        if bid_domain::outline::LengthProfile::from_key(k).is_none() {
            return Err(ToolError::EUnprocessable {
                what: format!(
                    "{TOOL}: length_profile 非法：{k}（合法值：practical|standard|conservative|maximum——对应 400,000/800,000/1,200,000/2,500,000 字）"
                ),
            });
        }
    }
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

    // ---- #244 建纲前覆盖闭环门：标记过的文件必须标完（C 态零容忍），
    // 零标记文件走正则兜底不受限（GT 红线保护）。零副作用前置拒。
    let cov = coverage_gate(svc)?;

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

    // #287 三者优先：target_total_chars 显式给 → 用它；length_profile 显式
    // 给 → 档位字数；都不给 → 默认档（Conservative 1,200,000——单一真源
    // 常量，此处不得另写数字；曾 unwrap_or(20_000) 覆盖域层默认 → 24 页纲）
    let profile_target = length_profile
        .as_deref()
        .and_then(bid_domain::outline::LengthProfile::from_key)
        .map(|p| p.target_total_chars());
    let spec = bid_domain::outline::OutlineSpec {
        target_total_chars: target_total_chars
            .or(profile_target.map(u64::from))
            .unwrap_or(u64::from(bid_domain::outline::DEFAULT_TARGET_TOTAL_CHARS))
            .min(u32::MAX as u64) as u32,
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

    // #287 档位回显：生效档（或 custom）+ 生效目标 + 页数——审计面可核对
    // 「选了什么档、引擎用了多少字」；E_TOTAL_CHARS_BELOW_MIN 门禁随
    // outline_total_chars()（配额总和）动态判——档位化后自动按选中档生效
    let effective_profile = bid_domain::outline::LengthProfile::ALL
        .into_iter()
        .find(|p| p.target_total_chars() == spec.target_total_chars);
    Ok(json!({
        "counts": {
            "sections": n_sections,
            "leaves": leaves.len(),
            "requirements": n_reqs,
            "total_target_chars": total_target,
            "matrix_rows": n_rows,
            "matrix_blocking": n_blocking,
        },
        "length": {
            "profile": effective_profile
                .map(|p| serde_json::to_value(p).unwrap_or(json!("custom")))
                .unwrap_or(json!("custom")),
            "target_total_chars": spec.target_total_chars,
            "approx_pages": (u64::from(spec.target_total_chars)
                + u64::from(bid_domain::outline::CHARS_PER_PAGE)
                - 1)
                / u64::from(bid_domain::outline::CHARS_PER_PAGE),
        },
        // #244 覆盖闭环回执（信息性）：建纲时点的处置面分布——
        // excluded = B 态（已排除）；regex_fallback = 零标记走兜底的文件。
        "coverage": {
            "face_files": cov.face_files,
            "marked_files": cov.marked_files,
            "regex_fallback_files": cov.regex_fallback_files,
            "excluded_blocks": cov.excluded_blocks,
            "unjudged_blocks": 0,
        },
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

// ---------------------------------------------------------------- #287 篇幅档位推荐

/// 采购类型推断（#287 修正项信号——best-effort，关键词面）。
///
/// **仅用项目名**：锚文关键词不可用——通用标书样板词（「同类工程业绩」
/// 「安装调试」「工程量清单」）在任何采购类型都出现（真语料实测：询价
/// 设备件/传媒服务件全被误判工程），项目名才是类型真源。关键词收紧为
/// 明确工程形态；Agent 可通过显式选档覆盖（决策权在用户）。
fn infer_tender_type(project_name: &str) -> bid_domain::outline::TenderType {
    use bid_domain::outline::TenderType;
    let has = |kws: &[&str]| kws.iter().any(|k| project_name.contains(k));
    if has(&["工程施工", "建筑工程", "市政工程", "水利工程", "公路工程",
             "装修工程", "机电工程", "监理", "EPC", "总承包"]) {
        TenderType::Engineering
    } else if has(&["服务", "运维", "运营", "维护", "咨询", "传媒", "传播",
                   "租赁", "培训", "广告", "宣传"]) {
        TenderType::Service
    } else {
        TenderType::Goods
    }
}

/// 招标文件明写篇幅扫描（#287 信号 1——最高优先级）。
///
/// 模式（保守面——只认明确的限值表述；10 份真实语料实测无一条篇幅
/// 限制，本扫描主要服务「有明写时自动采用」）：
/// - 「不超过/不得超过/最多/限制在 … N 页/字」
/// - 「N 页/字 以内/之内」
/// 返回字数（页 × 844 换算）；无 → None。
fn scan_explicit_length_limit(text: &str) -> Option<u32> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut best: Option<u32> = None;
    for i in 0..n {
        // 找数字起点
        if !chars[i].is_ascii_digit() {
            continue;
        }
        let start = i;
        let mut j = i;
        while j < n && chars[j].is_ascii_digit() {
            j += 1;
        }
        let num: String = chars[start..j].iter().collect();
        let Ok(v) = num.parse::<u64>() else { continue };
        if !(50..=5_000_000).contains(&v) {
            continue; // 页/字合理域（防序号/价格误伤）
        }
        let after = &chars[j..(n.min(j + 12))];
        let after_s: String = after.iter().collect();
        let before_win = chars[start.saturating_sub(12)..start]
            .iter()
            .collect::<String>();
        let is_page = after_s.starts_with('页');
        let is_char = after_s.starts_with('字');
        if !is_page && !is_char {
            continue;
        }
        let limit_before = before_win.contains("不超过")
            || before_win.contains("不得超过")
            || before_win.contains("最多")
            || before_win.contains("限制在");
        let limit_after = after_s.contains("以内") || after_s.contains("之内");
        if limit_before || limit_after {
            let chars_value = if is_page {
                v.saturating_mul(u64::from(bid_domain::outline::CHARS_PER_PAGE))
            } else {
                v
            } as u32;
            best = Some(best.map_or(chars_value, |b: u32| b.min(chars_value)));
        }
    }
    best
}

/// `recommend_length_profile`（#287——只读：P3 建纲前先调，把建议+理由+
/// alternatives 呈现给用户确认；**决策权在用户**，Agent 只推荐）。
///
/// 信号面（引擎可编程信号）：
/// - 细则数：`collect_scoring_rows` 的 `sub_items` 总和（#252 预派生——无需先建纲）
/// - ★条款数：REQ `params.star == true` 计数（#276 标记面 ∪ #280 抽取面汇流；
///   #285 star_scan 字面真值口径在其 PR 合并后接入——两口径档位结论等价）
/// - 预算：REQ `params.amount_wan` 最大值（万元 ×10⁴ → 元）
/// - 采购类型：项目名 + 锚文关键词推断（工程/服务/货物）
/// - 明写篇幅：锚文限值模式扫描（页 ×844 换算）
fn recommend_length_profile(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "recommend_length_profile";
    let project_id = req_str(&args, "project_id", TOOL)?;
    route_project(svc, &project_id, TOOL)?;

    let reqs = svc.store().requirements().to_vec();
    if reqs.is_empty() {
        return Err(ToolError::EState {
            what: "条款集为空——先 extract_requirements（推荐信号来自 REQ 面）".into(),
        });
    }

    // ---- #287 信号收集（原始值随行回显——用户可核对）
    let scoring_detail_count: usize = bid_domain::scoring_outline::collect_scoring_rows(&reqs)
        .iter()
        .map(|r| r.sub_items.len())
        .sum();
    // ★计数：params.star 标记面（#276 标记面 ∪ #280 抽取面汇流；#285
    // star_scan 字面真值口径在其 PR 合并后接入——两口径档位结论等价：
    // 真实语料实测 hq-dev 47/53 均>30、nm-health 8/10 均≤30、wenming 5/7
    // 均≤30，修正项触发面一致）
    let star_count: usize = reqs
        .iter()
        .filter(|r| r.params.get("star").and_then(|v| v.as_bool()) == Some(true))
        .count();
    let budget_yuan: Option<u64> = reqs
        .iter()
        .filter_map(|r| {
            r.params
                .get("amount_wan")
                .and_then(|v| v.as_f64())
                .filter(|v| *v > 0.0)
                .map(|v| v as u64)
        })
        .fold(None::<u64>, |acc, v| Some(acc.map_or(v, |a| a.max(v))));
    // 类型：项目名单一真源（锚文样板词污染实测见 infer_tender_type 文档）；
    // 明写篇幅：锚文限值模式扫描（样本面可以更宽——限值表述本身就是稀缺信号）
    let mut text_face = String::new();
    for r in reqs.iter().take(300) {
        for a in r.anchors.iter().take(3) {
            text_face.push_str(&a.excerpt.chars().take(120).collect::<String>());
        }
    }
    let project_name = svc
        .store()
        .project()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let tender_type = infer_tender_type(&project_name);
    let explicit_limit = scan_explicit_length_limit(&text_face);

    let rec = bid_domain::outline::recommend_length(
        scoring_detail_count,
        star_count,
        budget_yuan,
        tender_type,
        explicit_limit,
    );
    let target_chars = rec
        .from_tender_spec
        .unwrap_or(rec.profile.target_total_chars());

    let alternatives: Vec<Value> = bid_domain::outline::LengthProfile::ALL
        .iter()
        .map(|p| {
            json!({
                "profile": serde_json::to_value(p).unwrap(),
                "label": p.label_zh(),
                "chars": p.target_total_chars(),
                "pages": p.approx_pages(),
                "when": p.when_zh(),
            })
        })
        .collect();

    Ok(json!({
        "project_id": project_id,
        "recommended_profile": serde_json::to_value(rec.profile).unwrap(),
        "profile_label": rec.profile.label_zh(),
        "target_chars": target_chars,
        "approx_pages": (u64::from(target_chars)
            + u64::from(bid_domain::outline::CHARS_PER_PAGE) - 1)
            / u64::from(bid_domain::outline::CHARS_PER_PAGE),
        "confidence": serde_json::to_value(rec.confidence).unwrap(),
        "reasons": rec.reasons,
        "from_tender_spec": rec.from_tender_spec,
        "alternatives": alternatives,
        "signals": {
            "scoring_detail_count": scoring_detail_count,
            "star_count": star_count,
            "budget_yuan": budget_yuan,
            "tender_type": serde_json::to_value(tender_type).unwrap(),
            "explicit_limit_chars": explicit_limit,
        },
        "next": if rec.from_tender_spec.is_some() {
            "招标文件明写篇幅——build_outline(target_total_chars=明写值) 直接采用，无需用户选档"
        } else {
            "把建议 + 理由 + alternatives 呈现给用户确认后 build_outline(length_profile=…)——决策权在用户，推荐可被覆盖"
        },
    }))
}

/// 注册入参的可选定位器解析（契约 §11——#185 来源验证后的可验证形态面）。
///
/// 缺省（不传 locator）→ `Ok(None)`：文件级锚——服务层以库内 sha 生成
/// `MarkdownBlock { block_id: "mcp-registered:<文件名>" }`（诚实标记「经
/// 登记面、无引擎定位器」，审计面可辨识，不冒充引擎抽取）。
///
/// 传入 locator → 必须是**能按解析产物对账**的形态（docx_paragraph /
/// pdf_page_text——服务层验段落序/页号与内容指纹）；markdown_block 及
/// XLSX/Canvas 面无对账基准 → 显式拒（fail-closed，不静默降级到未验证面）。
fn parse_locator_arg(
    args: &Value,
    at: &str,
) -> Result<Option<bid_domain::locator::EngineLocator>, ToolError> {
    use bid_domain::locator::EngineLocator;
    let Some(loc) = args.get("locator").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    if !loc.is_object() {
        return Err(ToolError::EUnprocessable {
            what: format!("{at}: locator 必须为对象 {{ kind, ... }}（收到 {loc}）"),
        });
    }
    let kind = loc["kind"]
        .as_str()
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{at}: locator.kind 必填（docx_paragraph|pdf_page_text）"),
        })?;
    let fp = |v: &Value| v.as_str().map(str::to_string).filter(|s| !s.is_empty());
    match kind {
        "docx_paragraph" => Ok(Some(EngineLocator::DocxParagraph {
            paragraph_index: loc["paragraph_index"].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32,
            text_fingerprint: fp(&loc["text_fingerprint"]).ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{at}: locator.text_fingerprint 必填（算法前缀:值——如 fnv64:xx）"),
            })?,
        })),
        "pdf_page_text" => Ok(Some(EngineLocator::PdfPageText {
            page: loc["page"].as_u64().unwrap_or(1).max(1).min(u32::MAX as u64) as u32,
            text_fingerprint: fp(&loc["text_fingerprint"]).ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{at}: locator.text_fingerprint 必填（算法前缀:值——如 fnv64:xx）"),
            })?,
        })),
        other => Err(ToolError::EUnprocessable {
            what: format!(
                "{at}: locator.kind 非法：{other}（#185 来源验证面支持 docx_paragraph|pdf_page_text——须与解析产物对账；不带 locator 则为文件级锚）"
            ),
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
/// 记录：同指纹+同验证态 → 幂等重放；异指纹或异验证态 → 旧记录单向降级
/// Superseded + 签发新 EV，旧绑定迁移到新记录）。
///
/// #185 缺口 1（来源验证）：来源身份由**服务端**裁决——`source_file_id` 必须
/// 指向已导入的 TenderFile（不存在/解析失败 → 拒）；sha256 与指纹由系统从
/// 库内与实际内容生成，**不接受调用方自报**。旧参数 `source_file` /
/// `source_sha256` / `fingerprint` 显式拒绝（E_UNPROCESSABLE + 指路
/// source_file_id——与 #182 reviewer 参数废除同款纪律：防「以为登记成功」
/// 的假成功，不静默忽略）。
///
/// #245 占位声明：`placeholder: true`（可选，默认 false）= 本次登记为占位/
/// 待核验材料（tier=PLACEHOLDER）——调用方**可以自报占位**（从低不从高的
/// 诚实方向），但**不得自报任何已验证态**：`verification_tier` 参数显式拒
/// （tier 只由服务端裁决；占位→真实的升级通道 = 重新登记换版，留审计痕）。
fn register_evidence(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "register_evidence";
    // ---- 旧来源参数显式拒绝（#185：防假成功——静默忽略会让调用方以为已登记）
    for legacy in ["source_file", "source_sha256", "fingerprint"] {
        if args.get(legacy).map(|v| !v.is_null()).unwrap_or(false) {
            return Err(ToolError::EUnprocessable {
                what: format!(
                    "{TOOL}: 参数 {legacy} 已废除（#185 来源验证）——证据来源必须锚定已导入文件：先 import_tender_files（回执 per_file[].file_id），再以 source_file_id 登记；sha256 与指纹由服务端从库内与实际内容生成，不接受调用方自报"
                ),
            });
        }
    }
    // ---- #245：验证态自报显式拒（占位→已验证只能经换版重登——受控、可审计）
    if args.get("verification_tier").map(|v| !v.is_null()).unwrap_or(false) {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "{TOOL}: 参数 verification_tier 不存在（#245：验证态由服务端裁决，不接受调用方自报）——占位材料请传 placeholder=true（诚实方向）；真实证据带 locator 登记即 CONTENT_MATCHES、缺省文件级锚即 FILE_EXISTS；占位→真实升级 = 导入真实材料后重新登记（换版，旧记录留审计痕）"
            ),
        });
    }
    let project_id = req_str(&args, "project_id", TOOL)?;
    let kind = parse_evidence_kind(&req_str(&args, "kind", TOOL)?, TOOL)?;
    let title = req_str(&args, "title", TOOL)?;
    let source_file_id = req_str(&args, "source_file_id", TOOL)?;
    let subject_name = req_str(&args, "subject_name", TOOL)?;
    let scope = opt_str(&args, "scope", TOOL)?.unwrap_or_default();
    let valid_from = opt_str(&args, "valid_from", TOOL)?;
    let valid_until = opt_str(&args, "valid_until", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;
    // #245 占位声明（bool，默认 false；非布尔类型响亮拒）
    let placeholder = match args.get("placeholder") {
        None | Some(Value::Null) => false,
        Some(v) if v.is_boolean() => v.as_bool().unwrap(),
        Some(_) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: placeholder 须为布尔（true = 占位/待核验登记）"),
            })
        }
    };

    // locator 可选（None = 文件级锚）；传入则须可验证（形态面本层拦，
    // 段落序/页号/指纹对账在服务层——按解析产物）
    let locator = parse_locator_arg(&args, TOOL)?;

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

    let reg = bid_tools::service::EvidenceRegistration {
        source_file_id,
        placeholder,
        kind,
        title,
        locator,
        valid_from,
        valid_until,
        subject_name,
        scope,
        client,
        amount_wan,
        completed_on,
        fields,
    };
    let (ev_id, deduplicated) = svc.register_evidence_verified(reg).map_err(ToolError::from)?;
    let ev = svc.store().evidence(&ev_id).ok().cloned();
    let status = ev
        .as_ref()
        .map(|e| e.status.as_str().to_string())
        .unwrap_or_default();
    // #217（#185 缺口 2）：四态验证面 + 正文摘录随回执返回——调用方当场
    // 可见「正文是否已被读取」（CONTENT_MATCHES + excerpt）或
    // 「只验了文件在场」（FILE_EXISTS，摘录为 null——升级路径：带 locator 重登）。
    let (verification_tier, excerpt, excerpt_locator, is_placeholder) = match &ev {
        Some(e) => (
            e.verification_tier.map(|t| t.as_str()),
            e.excerpt.clone(),
            e.excerpt_locator.clone(),
            e.is_placeholder(),
        ),
        None => (None, None, None, false),
    };

    // #245：占位登记的回执面警示——调用方当场可见「这是占位/待核验材料，
    // 不得当作已验证证据」（防误读：占位绑定不清合规阻塞、导出门 1d 拒）。
    let base_hint = if deduplicated {
        "语义位查重命中：同 key 同指纹同验证态 → 幂等重放（原 EV-id 直返，不换版）"
    } else {
        // #80 第三批外部 Agent 实测摩擦 #8：不得断言旧记录已降级——
        // 首登与换版在返回面不可分（都是签发新 EV），措辞改为条件式
        "新 EV 已签发（若为同 key 换版，旧记录已单向降级 SUPERSEDED 且既有绑定迁移到新记录；全新登记则无旧记录）"
    };
    let hint = if is_placeholder {
        format!("{base_hint}；#245 占位提示：本证据为占位/待核验材料（PLACEHOLDER）——可绑定（矩阵呈现 PENDING_EVIDENCE 待补）但不清除合规阻塞、不得声称资质已具备；升级 = 导入真实材料后重新登记（换版）")
    } else {
        base_hint.to_string()
    };

    Ok(json!({
        "ev_id": ev_id,
        "deduplicated": deduplicated,
        "kind": kind_machine(kind),
        "status": status,
        "verification_tier": verification_tier,
        "placeholder": is_placeholder,
        "excerpt": excerpt,
        "excerpt_locator": excerpt_locator,
        "hint": hint,
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
    let project_version = opt_cas_version(&args, TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    // 证据在场性前置（批量前单查——错误定位比逐件更清晰）
    svc.store().evidence(&ev_id).map_err(ToolError::from)?;

    let (bound, replayed, item_errors, rows, blocking) = svc
        .bind_evidence_batch(&ev_id, &req_ids, &bid_open_date)
        .map_err(ToolError::from)?;

    // #245：占位证据绑定回执警示——blocking 面直接可见「未归零」。
    let is_placeholder = svc
        .store()
        .evidence(&ev_id)
        .map(|e| e.is_placeholder())
        .unwrap_or(false);
    let bind_hint = if is_placeholder {
        format!(
            "本证据为占位/待核验材料（PLACEHOLDER）——绑定成功但**不清除合规阻塞**（矩阵呈现 PENDING_EVIDENCE 待补，blocking={}）；升级 = 导入真实材料后 register_evidence 换版（旧绑定自动迁移）",
            blocking
        )
    } else {
        String::new()
    };

    let data = json!({
        "ev_id": ev_id,
        "bound_new": bound,
        "replayed": replayed,
        "requested": req_ids.len(),
        "placeholder": is_placeholder,
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
    if is_placeholder {
        // 占位绑定成功但不清阻塞——回执面向调用方点明（防误读阻塞已清零）
        return Ok(json!({
            "ev_id": ev_id,
            "bound_new": bound,
            "replayed": replayed,
            "requested": req_ids.len(),
            "placeholder": true,
            "matrix": { "rows": rows, "blocking": blocking },
            "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
            "hint": bind_hint,
            "project_version": svc.store().content_version(),
            "request_id": next_request_id(),
        }));
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

/// `record_review_gate` —— 契约 §12（G3：三门记审，MATRIX_READY+；#182
/// receipt 验签）。
///
/// #182：`reviewer` 参数废除（裸身份串可被 Agent 伪造——P0-2 缺口），
/// 改收宿主签发的 `receipt`（服务层验签是 MCP 面的唯一入口；状态门/
/// 门序/身份纵深全在域层 record_review 裁决——壳层零重实现）。
/// 旧参数显式拒绝（不静默忽略——旧调用方快速失败，防“以为过了人工门”的假成功）。
fn record_review_gate(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "record_review_gate";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let gate = parse_gate(&req_str(&args, "gate", TOOL)?, TOOL)?;
    let decision = parse_decision(&req_str(&args, "decision", TOOL)?, TOOL)?;
    // #182：reviewer 参数已废除——显式拒绝（参数面先于验签面）
    if args.get("reviewer").is_some() {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "{TOOL}: reviewer 参数已废除（#182）——人工确认由宿主签发 receipt 提供，Agent 不得伪造"
            ),
        });
    }
    let receipt = req_str(&args, "receipt", TOOL)?;
    let findings = parse_findings(&args, TOOL)?;
    // #277：agent 声明过门的理由（agent receipt 路径必填非空——域层裁决；
    // human 路径可空，receipt 即信任锚）
    let agent_reason = opt_str(&args, "reason", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    let (receipt, principal) = svc
        .record_review_gate(gate, &receipt, decision, findings, agent_reason.as_deref())
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

    // #277：过门方式与审计面回显（agent 声明 → human_confirmed=false——
    // 导出警告判据；追认后最新一条翻 human_receipt）
    let gate_pass = svc.store().latest_gate_pass(gate);

    Ok(json!({
        "receipt": {
            "review_id": receipt.review_id,
            "gate": receipt.gate.as_str(),
            "decision": receipt.decision.as_str(),
            // legacy 回显面：human/agent:{actor_id} 派生值（#182 前口径的兼容载体）
            "reviewer": if principal.actor_type == bid_domain::principal::ActorType::Agent {
                format!("agent:{}", principal.actor_id)
            } else {
                format!("human:{}", principal.actor_id)
            },
            // #182 可信身份回显
            "actor": {
                "actor_id": principal.actor_id,
                "actor_type": principal.actor_type.as_str(),
                "authentication_method": principal.authentication_method,
            },
            // #277 过门方式（agent_declared 时导出带警告）
            "method": gate_pass.as_ref().map(|p| p.method.clone()),
            "human_confirmed": gate_pass.as_ref().map(|p| p.human_confirmed),
            "finding_ids": receipt.finding_ids,
            "spawned_rt_ids": receipt.spawned_rt_ids,
            // #180 审核内容绑定：本次记审针对的章节树内容摘要（export_allowed
            // 比对判据；幂等重放时为既有记录的摘要）
            "reviewed_content_digest": receipt.reviewed_content_digest,
        },
        "gates_approved": gates_approved,
        "agent_waivers": agent_waivers_json(svc),
        "human_review_complete": svc.store().human_review_complete(),
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

/// #277：agent 声明过门清单的响应面形状（record_review_gate / 导出 / 状态
/// 工具共用——单一真源防三处漂移）。
fn agent_waivers_json(svc: &BidService) -> Vec<Value> {
    svc.agent_gate_passes()
        .iter()
        .map(|p| {
            json!({
                "gate": p.gate.as_str(),
                "actor": p.actor_id,
                "actor_type": p.actor_type.as_str(),
                "method": p.method,
                "human_confirmed": p.human_confirmed,
                "at": p.at,
                "reason": p.reason,
            })
        })
        .collect()
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

// ---------------------------------------------------------------- #226 层 1 set_matrix_override

/// `set_matrix_override` —— #226 层 1：否决项死锁的人工出口。
///
/// 背景：真实招标文件里「本次招标不接受联合体投标」这类**规则说明**被判成
/// 否决项 → mandatory → 未响应即 blocking，但它本来就没有可响应的正文，
/// 于是章节永久不可起草。人工标 NOT_APPLICABLE（附理由）是唯一诚实的出路。
///
/// 四条硬约束（都在本层与域层各校一遍，缺一不成）：
/// 1. **NOT_APPLICABLE 必须给非空理由**——不可审计的豁免不算豁免；
/// 2. **requirement_id 必须真实存在**——防手造 id 刷出一堆无效豁免；
/// 3. **状态门**：MATRIX_READY+（矩阵未建时无可覆写的对象）；
/// 4. **receipt 验签（#182）**：人工出口只认真人——宿主签发的审批回执
///    （签域 `matrix_override:{project_id}:{requirement_id}:{status}:{理由
///    sha256}` 四重绑定，Lead 合并门口径 2026-11-04）。Agent 不得伪造：
///    REQ-0003 的 receipt 免不了 REQ-0004，人批「不适用」的签改不成
///    RESPONDED，人批的理由换一个字都过不了验签。
///
/// 生效面：写完覆写**立刻重算矩阵**（`rebuild_matrix_for_override`），
/// 响应里直接回 blocking 行数——调用方不必再拉一次矩阵就知道死锁解没解。
fn set_matrix_override(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "set_matrix_override";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let requirement_id = req_str(&args, "requirement_id", TOOL)?;
    let status_raw = req_str(&args, "status", TOOL)?;
    let deviation = opt_str(&args, "deviation", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;
    // #182：裸身份参数显式拒绝（参数面先于验签面——防未来有人加回
    // operator/principal 直传，把人工出口退化成 Agent 自免单通道）。
    if args.get("principal").is_some() || args.get("operator").is_some() {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "{TOOL}: principal/operator 参数不存在（#182）——人工确认由宿主签发 receipt 提供，Agent 不得伪造"
            ),
        });
    }
    let receipt = req_str(&args, "receipt", TOOL)?;

    let status = match status_raw.as_str() {
        "RESPONDED" => ResponseStatus::Responded,
        "NOT_APPLICABLE" => ResponseStatus::NotApplicable,
        "NOT_RESPONDED" => ResponseStatus::NotResponded,
        "PARTIAL" => ResponseStatus::Partial,
        other => {
            return Err(ToolError::EUnprocessable {
                what: format!(
                    "{TOOL}: status 非法 {other}（合法：RESPONDED | NOT_APPLICABLE | NOT_RESPONDED | PARTIAL）"
                ),
            });
        }
    };
    // 域层也有同一校验；这里先拦一次是为了给出面向工具调用的清晰报错
    // （域层那条是「不适用不给理由 = 不可审计的豁免」，对 Agent 不够直白）。
    if status == ResponseStatus::NotApplicable
        && deviation.as_deref().map(str::trim).unwrap_or("").is_empty()
    {
        return Err(ToolError::EUnprocessable {
            what: format!("{TOOL}: deviation 必填——NOT_APPLICABLE 不给理由 = 不可审计的豁免"),
        });
    }

    route_guard(svc, &project_id, TOOL, project_version)?;

    let (rows, blocking, principal) = svc
        .set_matrix_override(requirement_id.clone(), status, deviation.clone(), &receipt)
        .map_err(ToolError::from)?;

    // 回显该行覆写后的真实状态（从重算后的矩阵取——不是把入参抄回去，
    // 否则「写成功了但没生效」这种 bug 会被响应体掩盖）。
    let row = svc
        .store()
        .matrix()
        .iter()
        .find(|r| r.requirement_id == requirement_id)
        .map(|r| {
            json!({
                "id": r.id,
                "requirement_id": r.requirement_id,
                "status": r.status.as_str(),
                "deviation": r.deviation,
                "blocking": r.blocking,
                "evidence_ids": r.evidence_ids,
            })
        });

    // 覆写审计面（#226/#182：谁豁免了这一行、何时——回溯可查）
    let override_rec = svc
        .store()
        .matrix_overrides()
        .get(&requirement_id)
        .map(|o| {
            json!({
                "principal": o.principal,
                "at": o.at,
            })
        });

    Ok(json!({
        "requirement_id": requirement_id,
        "row": row,
        "override": override_rec,
        // #182 可信身份回显（与 record_addendum_waiver.actor 同款）
        "actor": {
            "actor_id": principal.actor_id,
            "actor_type": principal.actor_type.as_str(),
            "authentication_method": principal.authentication_method,
        },
        "matrix_rows": rows,
        "blocking_rows": blocking,
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

// ---------------------------------------------------------------- #184 record_addendum_waiver

/// `record_addendum_waiver` —— #184 补遗分级阻塞的逐件人工确认
/// （#182：receipt 验签）。
///
/// 语义：任一补遗不处 Ready 时起草闸阻塞（E_UNPROCESSABLE，逐件点名）；
/// 人工继续必须**逐件**确认到具体 file_id（非全局布尔——「3 份坏 1 份」
/// 要对齐到哪一份、为什么故意带病放行）。确认即审计面：
/// `{file_id, reason, at, principal, actor}`。
///
/// #182：`principal` 参数废除（裸身份串可被 Agent 伪造），改收宿主签发的
/// `receipt`（服务层验签，签域 `addendum_waiver:{file_id}` 件级绑定——
/// TF-0003 的 receipt 免不了 TF-0004）；旧参数显式拒绝（同 record_review_gate）。
///
/// 出参：`recorded`（本次确认）+ `addendum_waivers`（全部确认记录——
/// 验收 C 的「记录可查」）+ `addendum_readiness`（就绪度回显，含 waived
/// 标记——报告面仍含该件，但不再构成阻塞）。
fn record_addendum_waiver(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "record_addendum_waiver";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let file_id = req_str(&args, "file_id", TOOL)?;
    let reason = req_str(&args, "reason", TOOL)?;
    // #182：principal 参数已废除——显式拒绝（参数面先于验签面）
    if args.get("principal").is_some() {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "{TOOL}: principal 参数已废除（#182）——人工确认由宿主签发 receipt 提供，Agent 不得伪造"
            ),
        });
    }
    let receipt = req_str(&args, "receipt", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    let (recorded, principal) = svc
        .record_addendum_waiver(&file_id, &reason, &receipt)
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
            // legacy 回显面：human/agent:{actor_id} 派生值（随身份）
            "principal": recorded.principal,
            // #182 可信身份回显
            "actor": {
                "actor_id": principal.actor_id,
                "actor_type": principal.actor_type.as_str(),
                "authentication_method": principal.authentication_method,
            },
            // #277 过门方式（agent_declared 时导出带警告——本记录的
            // human_confirmed=false；追认后最新一条翻 human_receipt）
            "method": svc
                .store()
                .latest_gate_pass(bid_domain::review::Gate::AddendumWaiver)
                .map(|p| p.method.clone()),
            "human_confirmed": svc
                .store()
                .latest_gate_pass(bid_domain::review::Gate::AddendumWaiver)
                .map(|p| p.human_confirmed),
        },
        "addendum_waivers": waivers,
        "addendum_readiness": addendum_readiness_block(svc),
        "agent_waivers": agent_waivers_json(svc),
        "human_review_complete": svc.store().human_review_complete(),
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

// ---------------------------------------------------------------- #254 分段三工具

/// `append_section_segment` —— #254 章节级增量写入（长章节安全续写）。
///
/// 与 `draft_response_section`（整稿替换）的差异：本工具只提交**新段**
/// （`content` = 段正文，非章节全文）；引擎按 seq 追加并合稿——**前段
/// 不丢**。幂等：同章节同 `idem_key` 重提 → no-op（`idem_key` 缺省 =
/// 内容指纹，同段同内容重提自动去重）。
fn append_section_segment(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "append_section_segment";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let section_id = req_str(&args, "section_id", TOOL)?;
    let content = req_str(&args, "content", TOOL)?;
    let idem_key = opt_str(&args, "idem_key", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    let out = svc
        .append_segment(&section_id, &content, idem_key.as_deref())
        .map_err(ToolError::from)?;

    Ok(json!({
        "section_id": out.section_id,
        "segment": {
            "id": out.segment_id,
            "seq": out.seq,
            "version": out.version,
            "chars": out.segment_chars,
        },
        "changed": out.changed,
        "deduplicated": !out.changed,
        "assembled_chars": out.assembled_chars,
        "target_chars": out.target_chars,
        "remaining_chars": out.remaining_chars,
        "over_target": out.over_target,
        "segment_count": out.segment_count,
        "citations": out.citations,
        "unanchored_claims": out.unanchored_claims,
        "warnings": out.warnings,
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

/// `rewrite_section_segment` —— #254 局部修订（改指定段，不动兄弟段）。
///
/// CAS：`expected_version` 与段当前 version 不匹配 → `E_CONFLICT`
/// （并发/过期版本显式拒，无部分写入）。定位用 `segment_id`
/// （`list_section_segments` 的回显）。
fn rewrite_section_segment(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "rewrite_section_segment";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let segment_id = req_str(&args, "segment_id", TOOL)?;
    let content = req_str(&args, "content", TOOL)?;
    // CAS 对照面（可选——缺省跳过版本检查，与 project_version 的乐观
    // 并发模式一致；并发写作面建议携带）
    let expected_version = opt_u64(&args, "expected_version", TOOL)?
        .map(|v| v as u32);
    let project_version = opt_cas_version(&args, TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    let out = svc
        .rewrite_segment(&segment_id, &content, expected_version)
        .map_err(ToolError::from)?;

    Ok(json!({
        "section_id": out.section_id,
        "segment": {
            "id": out.segment_id,
            "seq": out.seq,
            "version": out.version,
            "chars": out.segment_chars,
        },
        "changed": out.changed,
        "deduplicated": !out.changed,
        "assembled_chars": out.assembled_chars,
        "target_chars": out.target_chars,
        "remaining_chars": out.remaining_chars,
        "over_target": out.over_target,
        "segment_count": out.segment_count,
        "citations": out.citations,
        "unanchored_claims": out.unanchored_claims,
        "warnings": out.warnings,
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

/// `list_section_segments` —— #254 段账本发现面（只读：段清单 + 合稿
/// 统计 + 剩余字数——续写前的「已写到哪」对账）。
fn list_section_segments(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "list_section_segments";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let section_id = req_str(&args, "section_id", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    let inv = svc.list_segments(&section_id).map_err(ToolError::from)?;
    let segments: Vec<Value> = inv
        .segments
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "seq": s.seq,
                "version": s.version,
                "chars": s.content.chars().count(),
                "idem_key": s.idem_key,
                "updated_at": s.created_at,
                "preview": s.content.chars().take(40).collect::<String>(),
            })
        })
        .collect();
    Ok(json!({
        "section_id": inv.section_id,
        "segments": segments,
        "assembled_chars": inv.assembled_chars,
        "target_chars": inv.target_chars,
        "remaining_chars": inv.remaining_chars,
        "over_target": inv.over_target,
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

/// `register_addendum_changes` —— #319 ①：补遗变更登记（DIRTY 打标 +
/// 改写指令生成 + 矩阵重算）。起草后补遗到达的主入口——把「补遗改了什么」
/// 翻译成「哪些章节哪些段要改」。
fn register_addendum_changes(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "register_addendum_changes";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let project_version = opt_cas_version(&args, TOOL)?;

    // ---- changes 数组解析（壳校验：结构 + 键值域；业务校验在服务/域层）
    let changes_raw = args
        .get("changes")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{TOOL}: changes 缺失或非数组（补遗变更清单至少一条）"),
        })?
        .clone();
    let mut specs = Vec::with_capacity(changes_raw.len());
    for (i, c) in changes_raw.iter().enumerate() {
        let type_key = c
            .get("change_type")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{TOOL}: changes[{i}].change_type 缺失（value_changed|requirement_added|requirement_removed）"),
            })?;
        let change_type = bid_domain::rewrite::ChangeType::from_key(type_key)
            .ok_or_else(|| ToolError::EUnprocessable {
                what: format!(
                    "{TOOL}: changes[{i}].change_type 非法：{type_key}（合法值：value_changed|requirement_added|requirement_removed）"
                ),
            })?;
        specs.push(bid_domain::rewrite::ChangeSpec {
            req_id: opt_str(c, "req_id", &format!("{TOOL}[{i}]"))?,
            change_type,
            old_text: c
                .get("old_text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            new_requirement: c
                .get("new_requirement")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            category: opt_str(c, "category", &format!("{TOOL}[{i}]"))?,
            clause_no: opt_str(c, "clause_no", &format!("{TOOL}[{i}]"))?,
            section_id: opt_str(c, "section_id", &format!("{TOOL}[{i}]"))?,
            source_file_id: opt_str(c, "source_file_id", &format!("{TOOL}[{i}]"))?,
        });
    }

    route_guard(svc, &project_id, TOOL, project_version)?;

    let out = svc
        .register_addendum_changes(specs)
        .map_err(ToolError::from)?;

    let changes: Vec<Value> = out
        .changes
        .iter()
        .map(|r| {
            json!({
                "id": r.change.id,
                "req_id": r.change.req_id,
                "change_type": r.change.change_type.as_str(),
                "old_text": r.change.old_text,
                "new_requirement": r.change.new_requirement,
                "source_file_id": r.change.source_file_id,
                "registered_at": r.change.registered_at,
                "affected_sections": r.affected_sections,
                "warnings": r.warnings,
            })
        })
        .collect();
    let directives: Vec<Value> = out.directives.iter().map(directive_json).collect();
    let dirty_sections: Vec<Value> = out
        .dirty_sections
        .iter()
        .map(|sec| dirty_section_json(svc, sec))
        .collect();
    Ok(json!({
        "changes": changes,
        "directives": directives,
        "dirty_sections": dirty_sections,
        "directive_warnings": out.directive_warnings,
        "matrix": { "rows": out.matrix_rows, "blocking": out.matrix_blocking },
        "next": "逐指令处置：rewrite_section_partial（dry_run 先预览；新增要求走 append_section_segment 后回本工具 dismiss 或空 rewrites 处置）",
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

/// `rewrite_section_partial` —— #319 ②+③：指令执行（局部改写只动受影响段
/// + 局部复验只跑受影响章节）。dry_run 预览「将修改 X 段，Y 段不动」；
/// dismiss 人工裁定不适用；rewrites 空表 = 确认无需改写直接处置。
fn rewrite_section_partial(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "rewrite_section_partial";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let directive_id = req_str(&args, "directive_id", TOOL)?;
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let dismiss = args
        .get("dismiss")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let project_version = opt_cas_version(&args, TOOL)?;

    // rewrites 数组解析（可选——空表 = 确认无需改写/纯处置）
    let mut rewrites = Vec::new();
    if let Some(arr) = args.get("rewrites").and_then(Value::as_array) {
        for (i, r) in arr.iter().enumerate() {
            rewrites.push(bid_tools::rewrite::SegmentRewrite {
                segment_id: req_str(r, "segment_id", &format!("{TOOL}[{i}]"))?,
                content: req_str(r, "content", &format!("{TOOL}[{i}]"))?,
                expected_version: opt_u64(r, "expected_version", &format!("{TOOL}[{i}]"))?
                    .map(|v| v as u32),
            });
        }
    }

    route_guard(svc, &project_id, TOOL, project_version)?;

    let out = svc
        .rewrite_section_partial(&directive_id, rewrites, dry_run, dismiss)
        .map_err(ToolError::from)?;

    let modified: Vec<Value> = out
        .modified
        .iter()
        .map(|m| {
            json!({
                "segment_id": m.segment_id,
                "seq": m.seq,
                "version_before": m.version_before,
                "version_after": m.version_after,
                "chars": m.chars,
            })
        })
        .collect();
    Ok(json!({
        "directive": directive_json(&out.directive),
        "section_id": out.directive.section_id,
        "dry_run": out.dry_run,
        // 改动前预告面（真跑也回显——「改了 X 段 / Y 段不动」的对账）
        "plan": {
            "segments_to_modify": out.segments_to_modify,
            "segments_untouched": out.segments_untouched,
        },
        "modified_segments": modified,
        "untouched_segment_ids": out.untouched_segment_ids,
        "assembled_chars": out.assembled_chars,
        "target_chars": out.target_chars,
        "remaining_chars": out.remaining_chars,
        "over_target": out.over_target,
        // ③ 局部复验（只跑受影响章节——未受影响章节零重跑）
        "revalidation": {
            "section_id": out.revalidation.section_id,
            "ok": out.revalidation.ok,
            "blocker": out.revalidation.blocker,
            "major": out.revalidation.major,
            "minor": out.revalidation.minor,
            "reqs_revalidated": out.revalidation.reqs_revalidated,
            "reqs_responded": out.revalidation.reqs_responded,
        },
        "remaining_pending_directives": out.remaining_pending,
        "dirty_cleared": out.dirty_cleared,
        "warnings": out.warnings,
        "status": svc.store().project().map(|p| p.status.as_str()).unwrap_or(""),
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}

/// 指令的 JSON 形态（register / partial / state 三面共用）。
fn directive_json(d: &bid_domain::rewrite::RewriteDirective) -> Value {
    json!({
        "id": d.id,
        "section_id": d.section_id,
        "change_id": d.change_id,
        "req_id": d.req_id,
        "change_type": d.change_type.as_str(),
        "old_text": d.old_text,
        "new_requirement": d.new_requirement,
        "affected_segments": d.affected_segments.iter().map(|a| json!({
            "segment_id": a.segment_id,
            "seq": a.seq,
            "matched_by": a.matched_by,
        })).collect::<Vec<_>>(),
        "status": d.status.as_str(),
        "created_at": d.created_at,
        "applied_at": d.applied_at,
    })
}

/// 脏章节的 JSON 形态（标题/状态 + 待处置指令数——发现面）。
fn dirty_section_json(svc: &BidService, section_id: &str) -> Value {
    let node = svc.store().sections().iter().find(|n| n.id == section_id);
    json!({
        "section_id": section_id,
        "title": node.map(|n| n.title.clone()).unwrap_or_default(),
        "section_status": node.map(|n| n.status.as_str()).unwrap_or(""),
        "pending_directives": svc.store().pending_rewrite_directives_of(section_id).len(),
    })
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
