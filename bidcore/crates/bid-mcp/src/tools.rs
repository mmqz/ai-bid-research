//! 工具实现分派表。
//!
//! **纪律**：本模块只做参数解包 + 状态机裁决 + 转发。任何业务判断都应下沉到
//! `bid-domain` / `bid-parse` / `bid-kb`。骨架期未落地的工具一律返回
//! `UNIMPLEMENTED_INTERNAL`（带目标里程碑），**不得返回空成功**。

use serde_json::Value;

use crate::error::{ToolError, ToolResult};

/// 按工具名分派。调用方保证 name 已在 [`crate::TOOL_NAMES`] 内。
pub(crate) fn invoke(name: &str, args: Value) -> ToolResult {
    match name {
        "get_bid_project_state" => get_bid_project_state(args),
        // ---- M0 骨架：以下工具待各自里程碑接线 ----
        "create_bid_project" => Err(ToolError::unimplemented(name, "M0-d（领域持久化）")),
        "import_tender_files" => Err(ToolError::unimplemented(name, "M1-a（bid-parse 接线）")),
        "extract_requirements" => Err(ToolError::unimplemented(name, "M1-b（抽取接线）")),
        "get_compliance_matrix" => Err(ToolError::unimplemented(name, "M2-a（矩阵构建）")),
        "search_evidence" => Err(ToolError::unimplemented(name, "M2-b（双通道检索）")),
        "draft_response_section" => Err(ToolError::unimplemented(name, "M3（起草 + 内核 CAS 写入）")),
        "validate_bid_document" => Err(ToolError::unimplemented(name, "M4（规则注册表）")),
        "export_bid_document" => Err(ToolError::unimplemented(name, "M5（内核导出适配器）")),
        other => Err(ToolError::ENotFound {
            what: format!("未接入的工具 {other}"),
        }),
    }
}

/// `get_bid_project_state` —— 只读状态查询（M0 已落地的最小可服务工具）。
///
/// 入参：`{"project_status": "DRAFT"}`（可选；缺省按 DRAFT 处理——骨架期无持久化层，
/// 状态由调用方显式带入，**不伪造已存储的项目**）。
///
/// 返回：`{ status, allowed_next_tools, terminal }`
fn get_bid_project_state(args: Value) -> ToolResult {
    let raw = args
        .get("project_status")
        .and_then(Value::as_str)
        .unwrap_or("DRAFT");

    let status = parse_status(raw).ok_or_else(|| ToolError::EUnprocessable {
        what: format!("无法识别的 project_status：{raw}（合法值见 bid-domain ProjectStatus）"),
    })?;

    Ok(serde_json::json!({
        "status": status.as_str(),
        "allowed_next_tools": status.allowed_tools(),
        "terminal": status.next_allowed().is_empty(),
    }))
}

fn parse_status(raw: &str) -> Option<bid_domain::ProjectStatus> {
    use bid_domain::ProjectStatus::*;
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
