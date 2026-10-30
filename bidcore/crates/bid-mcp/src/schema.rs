//! 传输面数据形状（MCP 附近的 serde 契约）。
//!
//! 形状刻意贴 MCP 的 `tools/call` 语义（`{tool, args}` 入，`{ok, data|error}` 出——
//! 契约 §0 通用响应包装），但**不与任何传输实现绑定**——stdio 与未来
//! HTTP / wasm 共用同一组类型。

use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
///
/// 线格式（契约 §0 通用响应包装，B-1 起对齐）：
/// `{"ok":true, "tool":.., "data":..}` / `{"ok":false, "tool":.., "error":{code,..}}`。
/// 手写 serde：`ok` 必须是**布尔**（serde 内部标签只能出字符串——M0 期的
/// `#[serde(tag="ok")]` 会发出 `"ok":"true"`，与契约及 dispatch_json 兜底
/// 路径的布尔口径不一致，B-1 修正）；成功载荷键名对齐契约的 `data`。
#[derive(Debug, Clone)]
pub enum ToolReply {
    Ok(ToolOutcome),
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

impl Serialize for ToolReply {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(3))?;
        match self {
            Self::Ok(out) => {
                map.serialize_entry("ok", &true)?;
                map.serialize_entry("tool", &out.tool)?;
                map.serialize_entry("data", &out.result)?;
            }
            Self::Err { tool, error } => {
                map.serialize_entry("ok", &false)?;
                map.serialize_entry("tool", tool)?;
                map.serialize_entry("error", error)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ToolReply {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            ok: bool,
            tool: String,
            data: Option<Value>,
            error: Option<ToolError>,
        }
        let raw = Raw::deserialize(deserializer)?;
        match (raw.ok, raw.data, raw.error) {
            (true, Some(result), _) => Ok(Self::Ok(ToolOutcome { tool: raw.tool, result })),
            (true, None, _) => Err(serde::de::Error::custom("ok=true 但缺 data")),
            (false, _, Some(error)) => Ok(Self::Err { tool: raw.tool, error }),
            (false, _, None) => Err(serde::de::Error::custom("ok=false 但缺 error")),
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
        "get_compliance_matrix" => "只读：响应矩阵与资格三态（S-4 游标分页）",
        "list_evidence" => "只读：证据台账列表（kind/status 过滤 + 游标分页）",
        "list_tasks" => "只读：任务分工列表（owner/status 过滤 + 游标分页）",
        "search_evidence" => "双通道证据检索（M2-b 落地时必须走 cursor.rs 游标模块）",
        "draft_response_section" => "单章起草（引用回查后写内核）",
        "validate_bid_document" => "双层校验（确定性规则 + 模型复核）",
        "export_bid_document" => "导出（含暗标模式）",
        "get_bid_project_state" => "只读：项目状态与 allowed_next_tools",
        "save_version" => "AI 主动存版本快照（mang-snap 同源）",
        "check_bid_quality" => "只读：生成质量覆盖度检查 + 报告落盘（#68 阶段 2）",
        "build_outline" => "建纲 + 矩阵首建（G1——#80 第三批补齐；REQUIREMENTS_EXTRACTED 态）",
        "register_evidence" => "证据登记（G2——语义位查重/换版；MATRIX_READY+）",
        "bind_evidence" => "证据批量绑定 + 矩阵重算（G2——EXPIRED 禁绑；MATRIX_READY+）",
        "record_review_gate" => "三门记审（G3——matrix/outline/pre_export；reviewer 须 human: 前缀）",
        _ => "unknown",
    }
}

fn is_implemented(name: &str) -> bool {
    matches!(
        name,
        "get_bid_project_state"
            | "create_bid_project"
            | "import_tender_files"
            | "extract_requirements"
            | "get_compliance_matrix"
            | "list_evidence"
            | "list_tasks"
            | "draft_response_section"
            | "export_bid_document"
            | "validate_bid_document"
            | "check_bid_quality"
            | "build_outline"
            | "register_evidence"
            | "bind_evidence"
            | "record_review_gate"
    )
}
