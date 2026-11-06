//! bid-mcp —— MCP 工具解析与分发骨架（M0）。
//!
//! 纪律（docs/bid-mcp-contract.md §0 通用约定）：
//! - 工具只做参数校验、状态机裁决与转发；业务逻辑在 domain/parse/kb；
//! - 状态机裁决**一律**走 `bid_domain` 的 `guard_tool`，本层不自行判断；
//! - 未实现 ≠ 成功：返回 typed `UNIMPLEMENTED_INTERNAL`，绝不静默过绿。
//!
//! B-1（issue #11）起写面工具接线：`create_bid_project` / `import_tender_files`
//! 落地，服务态由 bid-tools 的 `BidService` 承载（进程内单例见 `state`；
//! 测试/嵌入方用 [`dispatch_with`] 注入独立实例）。传输层只负责把原始字节
//! 变成 [`ToolCall`]，语义裁决全在本层。
//!
//! V3-5（issue #11 批次 v3 / S-4 游标分页）：新增 [`cursor`] 模块（opaque
//! 游标核心）与两个只读列表工具 `list_evidence` / `list_tasks`；
//! `get_compliance_matrix` 升级为游标分页信封（详见 docs/bid-mcp-contract.md
//! §0「大结果分页」与 §4/§4-A/§4-B）。
//!
//! #137（MCP 协议层）：新增 [`protocol`] 模块——JSON-RPC 2.0 + `initialize` /
//! `tools/list` / `tools/call`（标准 MCP 客户端可直接连接；新 bin
//! `bid-mcp-server`）。旧逐行裸 JSON 路径（[`dispatch_json`] +
//! `bid-mcp-stdio`）一字未动。

pub mod coverage_tools;
pub mod cursor;
pub mod error;
pub mod protocol;
pub mod schema;
pub mod tools;
pub mod transport;
mod state;

pub use bid_tools::service::{BidService, matrix_override_scope};
pub use error::{ToolError, ToolResult};
pub use protocol::McpSession;
pub use schema::{ToolCall, ToolOutcome, ToolReply};
pub use transport::Transport;

use serde_json::Value;

/// 32 个工具名（单一真源；schema 声明与分发都从这里取）。

///
/// V3-5（S-4 游标分页）新增 `list_evidence` / `list_tasks` 两个只读列表工具
/// （原 8 工具契约 + `get_bid_project_state` + 2 = 11）；#42 阶段 10 增
/// `save_version`；#68 阶段 2 增 `check_bid_quality`；#80 第三批 MCP 补齐
/// 增 4 个：`build_outline` / `register_evidence` / `bind_evidence` /
/// `record_review_gate`（G1/G2/G3——外部 Agent 纯 MCP 驱动全链的缺口面）；
/// #100 任务 3 增 3 个：`score_price` / `score_section` / `score_document`
/// （B1 价格分 + B2 自评的 MCP 接线——「能得几分、该报多少价」）；
/// #184 增 1 个：`record_addendum_waiver`（补遗分级阻塞的逐件人工确认）；
/// #219 增 3 个：全块覆盖三工具（get_document_chunks/submit_block_marks/
/// get_coverage_report）；#226 增 1 个：`set_matrix_override`（否决项
/// 死锁的人工出口）；#243 增 1 个：`recover_validating_state`（历史
/// 异常态 VALIDATING+缺章的受支持恢复出口）= 26；#254 增 3 个：分段三工具
/// = 29；#279 增 1 个：`list_tools`（行协议发现面——无上下文代理的握手入口）
/// = 30；#287 增 1 个：`recommend_length_profile`（篇幅档位推荐——P3 建纲
/// 前只读：信号面细则数/★条款数/预算/采购类型/明写篇幅）= 31；#285 ① 增 1 个：
/// `get_star_coverage`（★漏标检测——字面★真值 vs 已抽 REQ 差集；六场黑盒
/// 实测捕获率 1/5~5/5 波动的对账面）= 32。
pub const TOOL_NAMES: [&str; 32] = [

    "create_bid_project",
    "import_tender_files",
    "extract_requirements",
    "get_compliance_matrix",
    "list_evidence",
    "list_tasks",
    "search_evidence",
    "draft_response_section",
    "validate_bid_document",
    "export_bid_document",
    "get_bid_project_state",
    "save_version",
    "check_bid_quality",
    "build_outline",
    "register_evidence",
    "bind_evidence",
    "record_review_gate",
    "record_addendum_waiver",
    "score_price",
    "score_section",
    "score_document",
    // #219 全块覆盖三工具
    "get_document_chunks",
    "submit_block_marks",
    "get_coverage_report",
    // #285 ① ★漏标检测（只读对账——★字面真值 vs 已抽 REQ）
    "get_star_coverage",
    // #226 层 1：否决项死锁的人工出口
    "set_matrix_override",
    // #243：历史异常态（VALIDATING+缺章）的受支持恢复出口
    "recover_validating_state",
    // #254 章节级增量写入（分段追加/局部修订/账本发现——Skill 分批写作的
    // 引擎对偶面：段是真增量，整稿替换不再是长章节的唯一写法）
    "append_section_segment",
    "rewrite_section_segment",
    "list_section_segments",
    // #287 篇幅档位推荐（只读——P3 建纲前调，建议+理由+alternatives 交用户确认）
    "recommend_length_profile",
    // #279 行协议发现面：无上下文代理合理预期先握手（六场实测两个独立代理
    // 都试探了 list_tools）——返回工具清单 + 行协议契约摘要；JSON-RPC 客户端
    // 用 tools/list 等价获取，本工具是行协议（stdio 裸 JSON）的握手入口
    "list_tools",
];

/// 工具解析与分发入口（进程级全局服务态——stdio 默认传输面）。
///
/// 传输层（stdio / HTTP）只负责把原始字节变成 [`ToolCall`]，语义裁决全在此。
#[must_use]
pub fn dispatch(call: ToolCall) -> ToolReply {
    state::with_global(|service| dispatch_with(service, call))
}

/// 显式服务态的分发入口（测试 / 嵌入方注入独立 [`BidService`]——并行隔离，
/// 不共享全局；语义与 [`dispatch`] 完全一致）。
#[must_use]
pub fn dispatch_with(service: &mut BidService, call: ToolCall) -> ToolReply {
    let name = call.tool.as_str();
    if !TOOL_NAMES.contains(&name) {
        return ToolReply::error(
            name,
            ToolError::ENotFound {
                what: format!("未知工具 {name}（合法工具面见 TOOL_NAMES / docs/bid-mcp-contract.md）"),
            },
        );
    }
    match tools::invoke(name, call.args, service) {
        Ok(out) => ToolReply::ok(name, out),
        Err(e) => ToolReply::error(name, e),
    }
}

/// wasm / FFI 友好的字符串入口（std::io 之外的传输面复用；stdio 行协议同走此处）。
///
/// #279 第 2 项（信封结构错误）：ToolCall 的 `args` 带 `#[serde(default)]` 且
/// serde 静默忽略未知字段——代理把工具参数放**顶层**（`{"tool":..,"name":..}`
/// 而非 `{"tool":..,"args":{"name":..}}`）时解析**不报错**，`args` 落空值直达
/// 工具层，报出误导性的「name 缺失或非字符串」（#269 六场实测：danqing /
/// hc-cluster 各 5 连败后靠猜悟出信封结构）。本入口在分发前对原始 JSON 做
/// 信封检查：顶层出现**该工具已知参数名**（单一真源 [`schema::tools_schema`]
/// 的 inputSchema properties）→ 直接报「参数须嵌套在 "args" 下」的结构错误，
/// 不再让误导性字段缺失错误出场。未知顶层字段（如调试用的 id）不触发——
/// 维持原语义（工具层自报缺失，调用方真正没传参数）。
#[must_use]
pub fn dispatch_json(input: &str) -> String {
    // 先解析为 Value 复用：serde 结构化解析失败时仍能对顶层字段做信封诊断
    let raw: Option<Value> = serde_json::from_str(input).ok();
    match serde_json::from_str::<ToolCall>(input) {
        Ok(call) => {
            // 信封检查（解析成功路径）：args 为空值 + 顶层有该工具已知参数名
            if let Some(envelope_error) = top_level_args_envelope_error(raw.as_ref(), &call) {
                return serde_json::json!({
                    "ok": false,
                    "error": {"code": "E_UNPROCESSABLE", "what": envelope_error}
                })
                .to_string();
            }
            serde_json::to_string(&dispatch(call)).unwrap_or_else(|e| {
                serde_json::json!({
                    "ok": false,
                    "error": {"code": "E_UNPROCESSABLE", "what": format!("reply 序列化失败: {e}")}
                })
                .to_string()
            })
        }
        Err(e) => {
            // 信封检查（解析失败路径）：连 tool 字段都没有，但顶层躺着已知工具
            // 参数名 → 报结构问题而非裸 serde 错误（「missing field `tool`」对
            // 把参数全放顶层的调用方没有指引价值）
            if let Some(what) = parse_failure_envelope_hint(raw.as_ref(), &e) {
                return serde_json::json!({
                    "ok": false,
                    "error": {"code": "E_UNPROCESSABLE", "what": what}
                })
                .to_string();
            }
            serde_json::json!({
                "ok": false,
                "error": {"code": "E_UNPROCESSABLE", "what": format!("ToolCall 解析失败: {e}")}
            })
            .to_string()
        }
    }
}

/// 解析成功路径的信封检查：args 缺省/显式 null/空对象，且顶层（tool/args
/// 之外）有该工具 inputSchema 已声明的参数名 → 返回结构错误文案。
fn top_level_args_envelope_error(raw: Option<&Value>, call: &ToolCall) -> Option<String> {
    let args_empty = call.args.is_null()
        || call.args.as_object().is_some_and(serde_json::Map::is_empty);
    if !args_empty {
        return None; // args 带实质内容——参数已正确嵌套，无从误置
    }
    let obj = raw?.as_object()?;
    let extras: Vec<&str> = obj
        .keys()
        .filter(|k| k.as_str() != "tool" && k.as_str() != "args")
        .map(String::as_str)
        .collect();
    if extras.is_empty() {
        return None; // 干净的 {"tool":"x"}——无参工具的合法形态
    }
    let known = schema::tool_arg_fields(&call.tool);
    let misplaced: Vec<String> = extras
        .iter()
        .filter(|k| known.contains(&k.to_string()))
        .map(|k| (*k).to_string())
        .collect();
    if misplaced.is_empty() {
        return None; // 顶层字段不认识（非工具参数）——不越权猜测，维持原语义
    }
    Some(format!(
        "ToolCall 解析失败：参数须嵌套在 \"args\" 下（收到顶层字段 {}，未见 args）——\
         正确形态：{{\"tool\":\"{}\",\"args\":{{...}}}}",
        misplaced.join("、"),
        call.tool
    ))
}

/// 解析失败路径的信封提示：原始 JSON 是对象、无 tool 字段，但顶层有任一工具
/// 的已知参数名（全工具字段并集）→ 附结构指引。避免为「忘了 tool 字段还把
/// 参数放顶层」的调用方报干巴巴的 serde 行列号。
fn parse_failure_envelope_hint(raw: Option<&Value>, e: &serde_json::Error) -> Option<String> {
    let obj = raw?.as_object()?;
    if obj.contains_key("tool") {
        return None; // tool 在场——解析失败另有原因（类型错等），原样透传
    }
    let known = schema::all_tool_arg_fields();
    let misplaced: Vec<String> = obj
        .keys()
        .filter(|k| known.contains(k))
        .cloned()
        .collect();
    if misplaced.is_empty() {
        return None;
    }
    Some(format!(
        "ToolCall 解析失败：{e}——且顶层字段 {} 是工具参数（应嵌套在 \"args\" 下）；\
         正确形态：{{\"tool\":\"工具名\",\"args\":{{...}}}}（工具面见 list_tools）",
        misplaced.join("、")
    ))
}

/// 工具 schema 声明（MCP `tools/list` 应答体；静态描述 + #137 起逐工具
/// `inputSchema`——合法 JSON Schema，官方 SDK Tool 模型直接可用）。
#[must_use]
pub fn tools_schema() -> Value {
    schema::tools_schema()
}
