//! perf_tools.rs —— #315 业绩材料库四工具（MCP 面）。
//!
//! | 工具 | 职责 | 项目路由 |
//! |---|---|---|
//! | `register_performance` | 批量登记业绩（语义幂等）+ 库主登记 | **无**——库是跨项目资产，建项目前就能维护 |
//! | `list_performance` | 只读列表（标签/关键词/金额窗/日期过滤） | **无**——同上 |
//! | `recommend_performance` | 只读匹配推荐（四因子打分 + 建议章节） | 有——信号来自项目态（REQUIREMENTS_EXTRACTED+） |
//! | `write_performance_section` | 一键写入：证据自动登记绑定 + 业绩表/描述段单段写入 | 有——写面（MATRIX_READY+，与 append_segment 同窗口） |
//!
//! 分层纪律（borrow：coverage_tools）：参数校验 + 路由 + 状态机裁决在本层；
//! 打分/渲染/持久化全在 bid-domain `performance` / bid-tools `performance`。

use bid_domain::performance::{ClientType, PerformanceEntrySpec};
use serde_json::{json, Value};

use crate::error::{ToolError, ToolResult};
use crate::state::next_request_id;
use crate::tools::{opt_cas_version, opt_f64, opt_str, opt_str_array, req_str, route_guard};
use bid_tools::performance::PerfListFilter;
use bid_tools::service::BidService;

/// 入参 entries 数组单批上限（防一次灌几千条把库写爆——分批登记）。
const MAX_BATCH_ENTRIES: usize = 200;
/// top_k 值域上界（推荐面——取太多没有裁决价值，前 20 足够人工挑）。
const MAX_TOP_K: u64 = 20;

// ---------------------------------------------------------------- 登记面

/// `entries[]` 单条解析（数组元素 → PerformanceEntrySpec；类型面响亮拒）。
fn parse_entry(v: &Value, at: &str) -> Result<PerformanceEntrySpec, ToolError> {
    let obj = v.as_object().ok_or_else(|| ToolError::EUnprocessable {
        what: format!("{at} 须为对象（收到 {v}）"),
    })?;
    let need_str = |k: &str| -> Result<String, ToolError> {
        obj.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{at}.{k} 必填非空白"),
            })
    };
    let opt_strs = |k: &str| -> Result<Vec<String>, ToolError> {
        match obj.get(k) {
            None | Some(Value::Null) => Ok(vec![]),
            Some(Value::Array(a)) => a
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    x.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| ToolError::EUnprocessable {
                            what: format!("{at}.{k}[{i}] 须为字符串"),
                        })
                })
                .collect(),
            Some(_) => Err(ToolError::EUnprocessable {
                what: format!("{at}.{k} 须为字符串数组"),
            }),
        }
    };
    let amount = obj
        .get("amount_wan")
        .and_then(Value::as_f64)
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{at}.amount_wan 必填（数值，万元）"),
        })?;
    Ok(PerformanceEntrySpec {
        name: need_str("name")?,
        client: need_str("client")?,
        amount_wan: amount,
        completed_on: need_str("completed_on")?,
        categories: opt_strs("categories")?,
        materials: opt_strs("materials")?,
        key_points: opt_strs("key_points")?,
        subject_name: obj.get("subject_name").and_then(Value::as_str).map(str::to_string),
    })
}

/// `register_performance` —— 批量登记 + 库主登记（无项目路由——建项目前可用）。
///
/// 幂等：语义四元组 (name, client, amount_wan, completed_on) 命中既有条目 →
/// deduplicated 回显既有 id（不重复登记）；一条非法 → 整批拒（零副作用）。
pub fn register_performance(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "register_performance";
    let owner_name = opt_str(&args, "owner_name", TOOL)?;
    let mut specs: Vec<PerformanceEntrySpec> = Vec::new();
    match args.get("entries") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) => {
            if a.len() > MAX_BATCH_ENTRIES {
                return Err(ToolError::EUnprocessable {
                    what: format!(
                        "{TOOL}: entries 单批上限 {MAX_BATCH_ENTRIES} 条（收到 {}）——分批登记",
                        a.len()
                    ),
                });
            }
            for (i, v) in a.iter().enumerate() {
                specs.push(parse_entry(v, &format!("{TOOL}: entries[{i}]"))?);
            }
        }
        Some(other) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: entries 须为对象数组（收到 {other}）"),
            })
        }
    }
    if specs.is_empty() && owner_name.is_none() {
        return Err(ToolError::EUnprocessable {
            what: format!(
                "{TOOL}: 空调用——entries（1..={MAX_BATCH_ENTRIES} 条）与 owner_name 至少给一项"
            ),
        });
    }
    let report = svc.register_performances(specs, owner_name)?;
    Ok(json!({
        "added": report.added,
        "added_count": report.added.len(),
        "deduplicated": report.deduplicated,
        "total_entries": report.total,
        "owner_name": report.owner_name,
        "library_path": svc.perf_library_path().map(|p| p.display().to_string()),
        "library_persisted": report.persisted,
        "next": if report.persisted {
            "recommend_performance(project_id=…) 匹配挑选 → write_performance_section 一键写入"
        } else {
            "内存态业绩库（未设 BID_PERF_LIB 且无用户目录）——本会话可用，重启即失；持久化设 BID_PERF_LIB 或用工作区服务"
        },
        "request_id": next_request_id(),
    }))
}

// ---------------------------------------------------------------- 列表面

/// `list_performance` —— 只读列表（无项目路由）。
pub fn list_performance(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "list_performance";
    let filters = match args.get("filters") {
        None | Some(Value::Null) => None,
        Some(v @ Value::Object(_)) => Some(v),
        Some(other) => {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: filters 须为对象（收到 {other}）"),
            })
        }
    };
    let fstr = |k: &str| -> Result<Option<String>, ToolError> {
        filters
            .and_then(|f| f.get(k))
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| ToolError::EUnprocessable {
                        what: format!("{TOOL}: filters.{k} 须为字符串"),
                    })
            })
            .transpose()
    };
    let fnum = |k: &str| -> Result<Option<f64>, ToolError> {
        filters
            .and_then(|f| f.get(k))
            .map(|v| {
                v.as_f64()
                    .ok_or_else(|| ToolError::EUnprocessable {
                        what: format!("{TOOL}: filters.{k} 须为数值（万元）"),
                    })
            })
            .transpose()
    };
    let since = fstr("since")?;
    if let Some(s) = &since {
        if !bid_domain::dates::is_valid_ymd(s) {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: filters.since 非法 ISO 日期：{s:?}（须 YYYY-MM-DD）"),
            });
        }
    }
    let limit = match args.get("limit") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{TOOL}: limit 须为整数 1..=1000"),
            })?;
            if !(1..=1000).contains(&n) {
                return Err(ToolError::EUnprocessable {
                    what: format!("{TOOL}: limit 值域 1..=1000（收到 {n}——显式拒，不静默 clamp）"),
                });
            }
            Some(n as usize)
        }
    };
    let filter = PerfListFilter {
        category: fstr("category")?,
        keyword: fstr("keyword")?,
        min_amount_wan: fnum("min_amount_wan")?,
        max_amount_wan: fnum("max_amount_wan")?,
        since,
        limit,
    };
    let (rows, owner, persisted) = svc.list_performances(&filter)?;
    let entries: Vec<Value> = rows
        .iter()
        .map(|e| {
            json!({
                "perf_id": e.id,
                "name": e.name,
                "client": e.client,
                "amount_wan": e.amount_wan,
                "completed_on": e.completed_on,
                "categories": e.categories,
                "materials_count": e.materials.len(),
                "key_points": e.key_points,
                "client_type": bid_domain::performance::infer_client_type(&e.client).as_str(),
            })
        })
        .collect();
    Ok(json!({
        "entries": entries,
        "returned": rows.len(),
        "owner_name": owner,
        "library_path": svc.perf_library_path().map(|p| p.display().to_string()),
        "library_persisted": persisted,
        "request_id": next_request_id(),
    }))
}

// ---------------------------------------------------------------- 推荐面

/// `client_type` 参数解析（government|soe|private|other——或中文展示名）。
fn parse_client_type(s: &str, at: &str) -> Result<ClientType, ToolError> {
    match s.trim() {
        "government" | "政府" | "政府/事业单位" => Ok(ClientType::Government),
        "soe" | "国企" | "国有企业" => Ok(ClientType::Soe),
        "private" | "民营" | "民营企业" => Ok(ClientType::Private),
        "other" | "未判定" => Ok(ClientType::Other),
        other => Err(ToolError::EUnprocessable {
            what: format!(
                "{at}: client_type 非法：{other}（合法值：government|soe|private|other——采购人类型，不传则该因子弃权）"
            ),
        }),
    }
}

/// `recommend_performance` —— 只读匹配推荐（项目路由 + 只读态门）。
///
/// 信号面（诚实：全部可选，缺的因子弃权重归一）：
/// - 语料：项目名 + REQ 陈述 + 评分项原文（服务层装配，无参）；
/// - `budget_wan`：项目预算（万元）——显式入参（仓内无预算结构面，不猜）；
/// - `base_date`：评标基准日（不传 = 系统今日，响应面注明来源）；
/// - `client_type`：采购人类型（不传 = 弃权）；
/// - `focus`：额外关键词（用户/Agent 明示的侧重——拼进语料）。
pub fn recommend_performance(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "recommend_performance";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let budget_wan = opt_f64(&args, "budget_wan", TOOL)?;
    let base_date = opt_str(&args, "base_date", TOOL)?;
    let client_type = match opt_str(&args, "client_type", TOOL)? {
        Some(s) => Some(parse_client_type(&s, TOOL)?),
        None => None,
    };
    let focus = opt_str(&args, "focus", TOOL)?;
    let top_k = match args.get("top_k") {
        None | Some(Value::Null) => 10,
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| ToolError::EUnprocessable {
                what: format!("{TOOL}: top_k 须为整数 1..={MAX_TOP_K}"),
            })?;
            if n == 0 || n > MAX_TOP_K {
                return Err(ToolError::EUnprocessable {
                    what: format!("{TOOL}: top_k 值域 1..={MAX_TOP_K}（收到 {n}）"),
                });
            }
            n as usize
        }
    };
    // 只读路由（无 CAS——读工具不吃版本面）
    crate::tools::route_project(svc, &project_id, TOOL)?;
    svc.store().guard_tool(TOOL).map_err(ToolError::from)?;

    let (matches, total, base) =
        svc.recommend_performances(budget_wan, base_date, client_type, focus.clone(), top_k)?;
    let base_from_param = args.get("base_date").map(|v| !v.is_null()).unwrap_or(false);
    let recommendations: Vec<Value> = matches
        .iter()
        .map(|m| {
            json!({
                "perf_id": m.perf_id,
                "name": m.name,
                "score": m.score,
                "factors": m.factors.iter().map(|f| json!({
                    "factor": f.factor,
                    "score": f.score,
                    "available": f.available,
                    "note": f.note,
                })).collect::<Vec<_>>(),
                "reasons": m.reasons,
            })
        })
        .collect();
    Ok(json!({
        "project_id": project_id,
        "recommendations": recommendations,
        "library_total": total,
        "signals": {
            "base_date": base,
            "base_date_source": if base_from_param { "param" } else { "system_today" },
            "budget_wan": budget_wan,
            "client_type": client_type.map(|c| c.as_str()),
            "focus": focus,
        },
        "suggested_section": svc.suggest_performance_section(),
        "next": if svc.suggest_performance_section().is_some() {
            "write_performance_section(section_id=建议章节, perf_ids=选中条目) 一键写入（证据自动登记绑定）"
        } else {
            "未建纲或纲中无业绩章节——build_outline 后回来看 suggested_section；也可直接用推荐结果人工起草"
        },
        "request_id": next_request_id(),
    }))
}

// ---------------------------------------------------------------- 写入面

/// `write_performance_section` —— 一键写入业绩章节（项目路由 + 写态门 + CAS）。
///
/// 干什么（#315 任务③）：
/// 1. 证据自动登记：perf 条目的材料文件名命中已导入 TF → register_evidence
///    （kind=业绩；域层语义位查重天然幂等——重跑不重复登记）；
/// 2. 证据自动绑定：业绩类 REQ（陈述含 业绩/类似项目）逐 EV 绑定 + 矩阵重算；
/// 3. 单段写入：业绩表（GFM）+ 逐条描述段（key_points + [EV-*] 引用），
///    幂等键 `perf-section:<section_id>`（同章重跑 no-op）。
///
/// 诚实面：材料未导入 → materials_pending 逐条列明（正文照写，证据不伪造）；
/// readiness 门（缺 ≥3 个可用业绩证据）会拦写入并把材料指引带全。
pub fn write_performance_section(svc: &mut BidService, args: Value) -> ToolResult {
    const TOOL: &str = "write_performance_section";
    let project_id = req_str(&args, "project_id", TOOL)?;
    let section_id = req_str(&args, "section_id", TOOL)?;
    let perf_ids = opt_str_array(&args, "perf_ids", TOOL)?
        .ok_or_else(|| ToolError::EUnprocessable {
            what: format!("{TOOL}: perf_ids 必填（PERF-* 清单——recommend_performance 的推荐结果）"),
        })?;
    if perf_ids.is_empty() {
        return Err(ToolError::EUnprocessable {
            what: format!("{TOOL}: perf_ids 不可为空（至少一条业绩）"),
        });
    }
    let bid_open_date = opt_str(&args, "bid_open_date", TOOL)?;
    if let Some(d) = &bid_open_date {
        if !bid_domain::dates::is_valid_ymd(d) {
            return Err(ToolError::EUnprocessable {
                what: format!("{TOOL}: bid_open_date 非法 ISO 日期：{d:?}（证据有效性裁决基准日；不传用系统今日）"),
            });
        }
    }
    let project_version = opt_cas_version(&args, TOOL)?;

    route_guard(svc, &project_id, TOOL, project_version)?;

    let report = svc.write_performance_section(&section_id, &perf_ids, bid_open_date)?;
    Ok(json!({
        "project_id": project_id,
        "section_id": report.section_id,
        "segment_id": report.segment_id,
        "changed": report.changed,
        "deduplicated": !report.changed,
        "segment_chars": report.segment_chars,
        "evidence_registered": report.evidence_registered.iter().map(|l| json!({
            "perf_id": l.perf_id, "ev_id": l.ev_id, "source_file": l.source_file,
            "deduplicated": l.deduplicated,
        })).collect::<Vec<_>>(),
        "materials_pending": report.materials_pending,
        "bindings": report.bindings.iter().map(|b| json!({
            "ev_id": b.ev_id, "req_ids": b.req_ids,
        })).collect::<Vec<_>>(),
        "warnings": report.warnings,
        "project_version": svc.store().content_version(),
        "request_id": next_request_id(),
    }))
}
