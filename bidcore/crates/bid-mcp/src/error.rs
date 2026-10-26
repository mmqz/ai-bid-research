//! typed 错误面——与 `bid-domain::ToolError` 共用一个错误词典，不另造一套。

use serde::{Deserialize, Serialize};

pub type ToolResult = Result<serde_json::Value, ToolError>;

/// MCP 工具错误。`code` 是稳定契约字面量，调用方按 code 分支，不解析 message。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code")]
pub enum ToolError {
    /// 状态机越级：当前项目状态不允许调用该工具（零副作用）。
    EState { what: String },
    /// 乐观并发冲突。
    EConflict { what: String },
    /// 目标不存在。
    ENotFound { what: String },
    /// 入参不合法 / 无法处理。
    EUnprocessable { what: String },
    /// 部分成功（需调用方逐件核对）。
    EPartial { what: String },
    /// 该工具在 M0 骨架期未落地——**不表示失败，表示能力尚未接线**。
    UnimplementedInternal { tool: String, milestone: String },
}

impl ToolError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::EState { .. } => "E_STATE",
            Self::EConflict { .. } => "E_CONFLICT",
            Self::ENotFound { .. } => "E_NOTFOUND",
            Self::EUnprocessable { .. } => "E_UNPROCESSABLE",
            Self::EPartial { .. } => "E_PARTIAL",
            Self::UnimplementedInternal { .. } => "UNIMPLEMENTED_INTERNAL",
        }
    }

    /// 未实现工具的快捷构造（骨架期统一出口）。
    #[must_use]
    pub fn unimplemented(tool: &str, milestone: &str) -> Self {
        Self::UnimplementedInternal {
            tool: tool.to_string(),
            milestone: milestone.to_string(),
        }
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EState { what } => write!(f, "E_STATE: {what}"),
            Self::EConflict { what } => write!(f, "E_CONFLICT: {what}"),
            Self::ENotFound { what } => write!(f, "E_NOTFOUND: {what}"),
            Self::EUnprocessable { what } => write!(f, "E_UNPROCESSABLE: {what}"),
            Self::EPartial { what } => write!(f, "E_PARTIAL: {what}"),
            Self::UnimplementedInternal { tool, milestone } => {
                write!(f, "UNIMPLEMENTED_INTERNAL: {tool} 待 {milestone} 接线")
            }
        }
    }
}

impl std::error::Error for ToolError {}

/// 从 `bid_domain::ToolError` 单向转换——本 crate 不持有自己的状态机。
///
/// 转换保留 domain 侧的原始字段（current/attempted/expected/got/reason/item_errors），
/// 不做信息压缩——调用方需要原始上下文来生成用户可读的引导。
impl From<bid_domain::ToolError> for ToolError {
    fn from(e: bid_domain::ToolError) -> Self {
        use bid_domain::ToolError as D;
        match e {
            D::EState { current, attempted } => Self::EState {
                what: format!("当前状态 {} 不允许调用 {attempted}", current.as_str()),
            },
            D::EConflict { expected, got } => Self::EConflict {
                what: format!("project_version 冲突：期望 {expected}，实际 {got}"),
            },
            D::ENotFound { what } => Self::ENotFound { what },
            D::EUnprocessable { reason } => Self::EUnprocessable { what: reason },
            D::EPartial { item_errors } => Self::EPartial {
                what: format!("{} 条条目失败：{}", item_errors.len(), item_errors.join("; ")),
            },
        }
    }
}
