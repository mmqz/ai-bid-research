//! #472（B-08）：extract_requirements 的 per-file 零抽取可见性（工具面）。
//!
//! 背景：`ZERO_REQUIREMENTS` 只在整批 total==0 时响——混合批次
//! （清单型 xlsx 主件 + docx 主件）中单个文件 0 REQ 时完全静默
//! （audit B-08 的核心信号面）。#472 补 `SOURCE_FILE_ZERO_REQ`：
//! 有抽取面的文件逐件对账 source_file，0 条 → 逐文件警告。
//!
//! 用【corpus-A 真实夹具】构造混合批次：A-招标文件.docx（有产出）+
//! B3-图纸目录.xlsx（登记为「采购需求清单.xlsx」、kind=招标文件——
//! 模拟真实命名的清单型主件；图纸目录无条款语义 → 0 REQ）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use bid_parse::env_preflight::skip_if_fixture_missing;
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn temp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-sfzr-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn import(svc: &mut BidService, pid: &str, path: &std::path::Path, name: &str, kind: &str) -> ToolReply {
    call(
        svc,
        "import_tender_files",
        json!({
            "project_id": pid,
            "files": [{
                "upload_ref": path.display().to_string(),
                "filename": name,
                "kind": kind,
            }],
        }),
    )
}

/// 混合批次：docx 主件有产出 + 清单型 xlsx 主件 0 REQ →
/// 响应必须携带点名该 xlsx 的 SOURCE_FILE_ZERO_REQ 警告
/// （修复前：total>0 → 零信号，xlsx 零贡献不可见）。
#[test]
fn mixed_batch_zero_req_xlsx_is_named_in_warnings() {
    let docx = fixtures().join("A-招标文件.docx");
    let xlsx = fixtures().join("B3-图纸目录.xlsx");
    if skip_if_fixture_missing("mixed_batch_zero_req_xlsx", &[&docx, &xlsx]) {
        return;
    }

    let ws = temp_ws("mixed");
    // 物理名与登记名保持一致（ParsedFile.name = 物理 basename——引擎
    // parse_office_via_engine 的既有口径；kind 分发按 store 登记名查
    // kind_map，两口径同名才对齐）。staging 放工作区内（#183 围栏：
    // 持久化目录隐式受信——fixtures 只读不落副本）。
    let staging = ws.join("uploads");
    std::fs::create_dir_all(&staging).unwrap();
    let xlsx_named = staging.join("采购需求清单.xlsx");
    std::fs::copy(&xlsx, &xlsx_named).expect("复制 xlsx 夹具");
    let docx_named = staging.join("光纤租赁费招标文件-主件.docx");
    std::fs::copy(&docx, &docx_named).expect("复制 docx 夹具");

    let mut svc = BidService::open(&ws).unwrap().with_fs_root(fixtures());
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "#472 混合批次信号", "bid_deadline": "2026-12-31",
        "mode": "明标", "external_key": "sfzr-mixed",
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();

    // docx 主件（真实命名——#196 口径，不依赖前缀）
    import(&mut svc, &pid, &docx_named, "光纤租赁费招标文件-主件.docx", "招标文件");
    // 清单型 xlsx 主件：真实命名 + kind=招标文件（B-08 场景——
    // XlsxRow 块，图纸目录语义无可抽条款）
    import(&mut svc, &pid, &xlsx_named, "采购需求清单.xlsx", "招标文件");

    let out = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": pid,
    })));

    let total = out["counts"]["total"].as_u64().unwrap_or(0);
    assert!(total > 0, "docx 主件须有产出（corpus-A 基线）——total={total}");

    let warnings: Vec<String> = out["warnings"]
        .as_array()
        .map(|a| a.iter().filter_map(|w| w.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let hit = warnings
        .iter()
        .find(|w| w.starts_with("SOURCE_FILE_ZERO_REQ"))
        .unwrap_or_else(|| panic!("混合批次须含 SOURCE_FILE_ZERO_REQ 警告——实际 warnings: {warnings:?}"));
    assert!(
        hit.contains("采购需求清单.xlsx"),
        "警告须点名零贡献文件——实际: {hit}"
    );
    assert!(
        !hit.contains("光纤租赁费招标文件-主件.docx"),
        "有产出的文件不得误入零贡献清单——实际: {hit}"
    );
}

/// 对照面：单一主件有产出（无零贡献文件）→ 不得出 SOURCE_FILE_ZERO_REQ
///（防误报回归锁——正常批次零噪音）。
#[test]
fn single_productive_main_file_has_no_zero_req_warning() {
    let docx = fixtures().join("A-招标文件.docx");
    if skip_if_fixture_missing("single_main_no_zero_req", &[&docx]) {
        return;
    }
    let ws = temp_ws("single");
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(fixtures());
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "#472 对照面", "bid_deadline": "2026-12-31",
        "mode": "明标", "external_key": "sfzr-single",
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    import(&mut svc, &pid, &docx, "招标文件主件.docx", "招标文件");

    let out = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": pid,
    })));
    let total = out["counts"]["total"].as_u64().unwrap_or(0);
    assert!(total > 0);
    let warnings: Vec<String> = out["warnings"]
        .as_array()
        .map(|a| a.iter().filter_map(|w| w.as_str().map(String::from)).collect())
        .unwrap_or_default();
    assert!(
        !warnings.iter().any(|w| w.starts_with("SOURCE_FILE_ZERO_REQ")),
        "单一有产出主件不得误报——实际 warnings: {warnings:?}"
    );
}
