//! typed 错误面——与 `bid-domain::ToolError` 共用一个错误词典，不另造一套。

use serde::{Deserialize, Serialize};

pub type ToolResult = Result<serde_json::Value, ToolError>;

/// MCP 工具错误。`code` 是稳定契约字面量，调用方按 code 分支，不解析 message。
///
/// serde：`rename_all = "SCREAMING_SNAKE_CASE"` 使内部标签 `code` 落位契约
/// 字面量（`E_STATE` / `E_UNPROCESSABLE` / ...）——M0 期缺 rename 时线格式
/// 曾输出变体名（`"code":"EUnprocessable"`），B-1 修正对齐契约 §0。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolError {
    /// 状态机越级：当前项目状态不允许调用该工具（零副作用）。
    EState { what: String },
    /// 乐观并发冲突。
    EConflict { what: String },
    /// 目标不存在。
    ENotFound { what: String },
    /// 入参不合法 / 无法处理。
    EUnprocessable { what: String },
    /// 部分成功（需调用方逐件核对）。`item_errors` 是契约 §0 错误形状的逐件
    /// 失败清单；`data` 承载已成功条目的完整工具 out（如契约 §2 的
    /// file_ids/per_file/totals）——R-01：Agent 要「核对返回统计并重试失败
    /// 件」，错误码不吞成功面。`hint` 给出可执行的重试指引。
    EPartial {
        what: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        item_errors: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    /// 该工具在 M0 骨架期未落地——**不表示失败，表示能力尚未接线**。
    UnimplementedInternal { tool: String, milestone: String },
    /// #243：全稿最终校验前置拒——起草未完成（存在未达 DRAFTED 的叶）。
    /// 零副作用（状态/版本/审核记录均不变）；`missing` = 未完成叶的
    /// section_id 清单（可操作：逐叶补起草，或改 scope=sections 章节级检查）。
    EDraftIncomplete {
        missing: Vec<String>,
    },
    /// #244：建纲前覆盖闭环前置拒——存在「已启用标记流程但未判完」的块
    /// （C 态：遗漏待审）。零副作用（状态/版本/标记集均不变）。
    ///
    /// 与 [`Self::EDraftIncomplete`] 同构的门禁面：结构化清单 + Display
    /// 人读指引。`gaps` 每元素 = 一个未判定区间的人读描述（文件名 /
    /// 区间 / locator / 块内容摘要——Agent 得知道是什么内容才能判，
    /// 只给区间号等于没给）。
    ///
    /// 门只拦「半途而废」（marks 非空且有 C 态块）；零标记文件走正则
    /// 兜底不受此门限制（GT 红线保护——详见 tools.rs `coverage_gate`）。
    ECoverageIncomplete {
        /// 全项目 C 态块总数（有抽取面文件合计）。
        unjudged_blocks: usize,
        /// 未判定区间清单（每文件每区间一条，含定位与块内容摘要）。
        gaps: Vec<String>,
    },
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
            Self::EDraftIncomplete { .. } => "E_DRAFT_INCOMPLETE",
            Self::ECoverageIncomplete { .. } => "E_COVERAGE_INCOMPLETE",
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
            Self::EPartial { what, .. } => write!(f, "E_PARTIAL: {what}"),
            Self::UnimplementedInternal { tool, milestone } => {
                write!(f, "UNIMPLEMENTED_INTERNAL: {tool} 待 {milestone} 接线")
            }
            Self::EDraftIncomplete { missing } => write!(
                f,
                "E_DRAFT_INCOMPLETE: 起草未完成（{} 个叶未达 DRAFTED：{}）——先补起草再全稿校验，或改用 scope=sections 章节级检查",
                missing.len(),
                missing.join("、")
            ),
            Self::ECoverageIncomplete { unjudged_blocks, gaps } => write!(
                f,
                "E_COVERAGE_INCOMPLETE: 还有 {unjudged_blocks} 块未判定，不能建纲\n\n未判定区间：\n{}\n\n处置：submit_block_marks 补标（要求块 verdict=requirement + requirements 提取关键点；非要求块 verdict=noise|rule + reason）→ 重跑 extract_requirements → build_outline。未标记过的文件走正则兜底，不受此门限制。",
                gaps.join("\n")
            ),
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
                item_errors,
                hint: None,
                data: None,
            },
            D::EDraftIncomplete { missing } => Self::EDraftIncomplete { missing },
        }
    }
}
