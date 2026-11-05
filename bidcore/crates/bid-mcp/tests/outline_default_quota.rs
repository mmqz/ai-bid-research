//! #253 第一段回归锚：`target_total_chars` 默认值三层统一（单一真源）。
//!
//! 病灶（issue #253 评论「默认值三层不一致」）：MCP 层 `tools.rs` 曾自写
//! `unwrap_or(20_000)`、`schema.rs` 描述同病（「缺省 20000」），而域层
//! `Default` 是 1,000,000——Agent 按工具描述不传参 → 20,000 劫持域层默认
//! → 建纲只出 24 页（844 字符/页口径）。
//!
//! 修复：单一真源 `bid_domain::outline::DEFAULT_TARGET_TOTAL_CHARS`
//! （= 844,000 ≈ 1000 页 × 844 字符/页），三层同引一个常量。
//!
//! 断言面（三层各一）：
//! 1. **schema 层**：build_outline 的入参描述由常量生成（不硬编码数字）；
//! 2. **MCP 接线层**：不传 `target_total_chars` → 配额按 844,000 分配
//!    （容差 ±1,000：按叶数比例分配的整数截断只向下、MIN_LEAF_CHARS=500
//!    地板只向上；旧病灶值 20,000 偏离 42 倍，容差不构成遮蔽）；
//! 3. **显式传参不被默认劫持**：100,000 → ≈100,000。
//!
//! 域层同源锚（`OutlineSpec::default()` == 常量）在 bid-domain
//! `outline.rs` 单元测试面，此处不重复。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

// ---------------------------------------------------------------- 调用助手

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(
        svc,
        ToolCall {
            tool: tool.to_string(),
            args,
        },
    )
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn fixtures_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    std::fs::canonicalize(&p).unwrap_or(p)
}

/// 独立临时工作区（并行安全——与 agent_mandatory_gate 同款）。
fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-outline-default-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 建项 + 导入 synth-tender A 件 + 抽取 → (svc, pid)。
/// 建纲由各断言自行调用（本文件只测 build_outline 工具面的默认值接线）。
fn extracted_project(tag: &str) -> (BidService, String) {
    let ws = tmp_ws(tag);
    let mut svc = BidService::open(&ws)
        .expect("打开工作区")
        .with_fs_root(fixtures_root());
    ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": format!("#253 默认配额 {tag}"), "bid_deadline": "2026-11-30",
            "mode": "明标", "external_key": format!("outline-default-{tag}"),
        }),
    ));
    let pid = "PRJ-0001".to_string();
    let a = fixtures_root().join("synth-tender/招标文件/A-招标文件.docx");
    assert!(a.is_file(), "夹具缺失: {}", a.display());
    let import = ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({
            "project_id": &pid,
            "files": [{ "upload_ref": a.display().to_string(),
                        "filename": "A-招标文件.docx", "kind": "招标文件" }],
        }),
    ));
    assert_eq!(
        import["totals"]["parse_failed"],
        json!(0),
        "A 件应解析成功: {import:?}"
    );
    ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    (svc, pid)
}

// ---------------------------------------------------------------- 断言面

/// 层 1：schema 描述由常量生成——描述与实现同源，不得硬编码数字。
#[test]
fn schema_description_quotes_the_single_source_constant() {
    let schema = bid_mcp::tools_schema();
    let t = schema["tools"]
        .as_array()
        .expect("tools 数组")
        .iter()
        .find(|t| t["name"] == "build_outline")
        .expect("tools/list 必须含 build_outline");
    let desc = t["inputSchema"]["properties"]["target_total_chars"]["description"]
        .as_str()
        .unwrap_or("");
    assert!(
        desc.contains(&bid_domain::outline::DEFAULT_TARGET_TOTAL_CHARS.to_string()),
        "描述必须含常量值 {}（实际 {desc:?}）——#253：描述与实现同源",
        bid_domain::outline::DEFAULT_TARGET_TOTAL_CHARS,
    );
}

/// 层 2：不传 target_total_chars → 按域层默认 844,000 分配（非 20,000）。
#[test]
fn build_outline_without_param_uses_domain_default() {
    let (mut svc, pid) = extracted_project("mcp-default");
    let r = ok_result(call(
        &mut svc,
        "build_outline",
        json!({ "project_id": &pid }),
    ));
    assert_eq!(
        r["status"],
        json!("MATRIX_READY"),
        "建纲应推进 MATRIX_READY: {r:?}"
    );
    let total = r["counts"]["total_target_chars"]
        .as_u64()
        .expect("total_target_chars 在场");
    let d = u64::from(bid_domain::outline::DEFAULT_TARGET_TOTAL_CHARS);
    assert!(
        total >= d - 1_000 && total <= d + 1_000,
        "不传参应按域层默认 {d} 分配（±1,000 容差：整数截断向下 / MIN_LEAF_CHARS 地板向上），\
         实际 {total}（若 ≈20,000 = 病灶复发：MCP 层又自写了数字）"
    );
}

/// 层 3：显式传参不被默认劫持（默认只在缺省时生效）。
#[test]
fn build_outline_explicit_param_still_wins() {
    let (mut svc, pid) = extracted_project("mcp-explicit");
    let r = ok_result(call(
        &mut svc,
        "build_outline",
        json!({
            "project_id": &pid, "target_total_chars": 100_000,
        }),
    ));
    let total = r["counts"]["total_target_chars"]
        .as_u64()
        .expect("total_target_chars 在场");
    assert!(
        total >= 99_000 && total <= 101_000,
        "显式 100,000 应原样生效（±1,000 容差），实际 {total}"
    );
}
