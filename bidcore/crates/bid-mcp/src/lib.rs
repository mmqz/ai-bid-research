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

pub mod error;
pub mod schema;
pub mod tools;
pub mod transport;
mod state;

pub use bid_tools::service::BidService;
pub use error::{ToolError, ToolResult};
pub use schema::{ToolCall, ToolOutcome, ToolReply};
pub use transport::Transport;

use serde_json::Value;

/// 9 个工具名（单一真源；schema 声明与分发都从这里取）。
pub const TOOL_NAMES: [&str; 9] = [
    "create_bid_project",
    "import_tender_files",
    "extract_requirements",
    "get_compliance_matrix",
    "search_evidence",
    "draft_response_section",
    "validate_bid_document",
    "export_bid_document",
    "get_bid_project_state",
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

/// wasm / FFI 友好的字符串入口（std::io 之外的传输面复用）。
#[must_use]
pub fn dispatch_json(input: &str) -> String {
    match serde_json::from_str::<ToolCall>(input) {
        Ok(call) => serde_json::to_string(&dispatch(call)).unwrap_or_else(|e| {
            serde_json::json!({
                "ok": false,
                "error": {"code": "E_UNPROCESSABLE", "what": format!("reply 序列化失败: {e}")}
            })
            .to_string()
        }),
        Err(e) => serde_json::json!({
            "ok": false,
            "error": {"code": "E_UNPROCESSABLE", "what": format!("ToolCall 解析失败: {e}")}
        })
        .to_string(),
    }
}

/// 工具 schema 声明（MCP `tools/list` 应答体；当前为静态描述）。
#[must_use]
pub fn tools_schema() -> Value {
    schema::tools_schema()
}
