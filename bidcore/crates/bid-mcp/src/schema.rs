//! 传输面数据形状（MCP 附近的 serde 契约）。
//!
//! 形状刻意贴 MCP 的 `tools/call` 语义（`{tool, args}` 入，`{ok, result|error}` 出），
//! 但**不与任何传输实现绑定**——stdio 与未来 HTTP / wasm 共用同一组类型。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ToolError;

/// 一次工具调用请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    #[serde(default)]
    pub args: Value,
}

/// 工具返回体（成功）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub tool: String,
    pub result: Value,
}

/// 工具返回体（成功或失败二选一）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "ok")]
pub enum ToolReply {
    #[serde(rename = "true")]
    Ok(ToolOutcome),
    #[serde(rename = "false")]
    Err { tool: String, error: ToolError },
}

impl ToolReply {
    #[must_use]
    pub fn ok(tool: &str, result: Value) -> Self {
        Self::Ok(ToolOutcome {
            tool: tool.to_string(),
            result,
        })
    }

    #[must_use]
    pub fn error(tool: &str, error: ToolError) -> Self {
        Self::Err {
            tool: tool.to_string(),
            error,
        }
    }
}

/// `tools/list` 应答体的静态描述（参数 schema 由各工具的 milestones 决定）。
#[must_use]
pub fn tools_schema() -> Value {
    let entries: Vec<Value> = crate::TOOL_NAMES
        .iter()
        .map(|name| {
            serde_json::json!({
                "name": name,
                "description": describe(name),
                "implemented": is_implemented(name),
            })
        })
        .collect();
    serde_json::json!({ "tools": entries })
}

fn describe(name: &str) -> &'static str {
    match name {
        "create_bid_project" => "建项目（external_key 幂等）",
        "import_tender_files" => "导入招标文件 + 完整性对账",
        "extract_requirements" => "六类条款抽取 + 覆盖率对账",
        "get_compliance_matrix" => "只读：响应矩阵与资格三态",
        "search_evidence" => "双通道证据检索",
        "draft_response_section" => "单章起草（引用回查后写内核）",
        "validate_bid_document" => "双层校验（确定性规则 + 模型复核）",
        "export_bid_document" => "导出（含暗标模式）",
        "get_bid_project_state" => "只读：项目状态与 allowed_next_tools",
        _ => "unknown",
    }
}

fn is_implemented(name: &str) -> bool {
    matches!(name, "get_bid_project_state")
}
