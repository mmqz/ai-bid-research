//! #425 · 补遗冲突检测按业务 kind 识别（R-10 第三处收口）。
//!
//! 缺陷：`extract_requirements` 的冲突检测用 `name.starts_with("C-")` 识别
//! 补遗件——真实命名的补遗（「澄清修改文件.docx」，kind=补遗）不进冲突面，
//! 主文件 × 补遗同条款号异文静默漏检（旧版条款可能被误响应）。
//! 与 #196（抽取分发）/ #250（合并门禁）同根因，本文件是同族收尾的验收面。
//!
//! 口径：
//! - `Some("补遗")` → 入冲突面（无论文件名形态）；
//! - `Some(其他 kind)`（含答疑）→ 不误入；
//! - kind 缺省（parsed 件不在 store 登记面）→ C- 前缀兜底（夹具约定，
//!   与 `has_extraction_face` 同口径——兜底臂随 #250 同款语义保留）。
//!
//! 夹具纪律：合成 docx（docx-rs，review_table_matrix 同款）——主文件与
//! 补遗件各含同号条款 1.1（异文，应检出）与 1.2（同文，不应误报）。
//! 不依赖 corpus-A 真件（其设计冲突 REQ 的 clause_no 为 null，走不进
//! 条款号匹配环——冲突面的既有夹具零覆盖正是 R-10 第三处存活的原因）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use docx_rs::{Docx, Paragraph, Run};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "mcp-425-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn p_text(s: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(s))
}

/// 主文件：编号条款 1.1 / 1.2（1.1 将被补遗修订；1.2 与补遗同文）。
fn build_main_docx(path: &Path) {
    let mut docx = Docx::new();
    docx = docx.add_paragraph(p_text("第一章 投标人须知"));
    docx = docx.add_paragraph(p_text("1.1 投标人须具备电子与智能化工程专业承包二级（含）以上资质，并提供证书副本复印件。"));
    docx = docx.add_paragraph(p_text("1.2 投标文件须对招标文件的技术要求逐项作出实质性响应。"));
    let mut f = std::fs::File::create(path).expect("建主文件 docx");
    docx.pack(&mut f).expect("写主文件 docx");
}

/// 补遗件：1.1 同号异文（应检出冲突）；1.2 同号同文（不应误报）。
fn build_addendum_docx(path: &Path) {
    let mut docx = Docx::new();
    docx = docx.add_paragraph(p_text("澄清修改文件"));
    docx = docx.add_paragraph(p_text("1.1 原招标文件第一章 1.1 调整为：投标人须具备电子与智能化工程专业承包一级（含）以上资质，并提供证书副本复印件。"));
    docx = docx.add_paragraph(p_text("1.2 投标文件须对招标文件的技术要求逐项作出实质性响应。"));
    let mut f = std::fs::File::create(path).expect("建补遗 docx");
    docx.pack(&mut f).expect("写补遗 docx");
}

/// 建项 + 导入主文件与补遗件（补遗以指定文件名/kind 落盘），跑 extract，
/// 返回 (conflicts, warnings)。
fn extract_with_addendum(
    tag: &str,
    addendum_filename: &str,
    addendum_kind: &str,
) -> (Vec<Value>, Vec<String>) {
    let ws = tmp_ws(tag);
    let mut svc = BidService::open(&ws).expect("打开工作区");
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "补遗冲突检测", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": tag,
    })));
    let main_path = ws.join("A-招标文件-合成.docx");
    build_main_docx(&main_path);
    let addendum_path = ws.join(addendum_filename);
    build_addendum_docx(&addendum_path);
    for (path, filename, kind) in [
        (&main_path, "A-招标文件-合成.docx", "招标文件"),
        (&addendum_path, addendum_filename, addendum_kind),
    ] {
        let out = ok_result(call(&mut svc, "import_tender_files", json!({
            "project_id": "PRJ-0001",
            "files": [{
                "upload_ref": path.to_string_lossy(),
                "filename": filename,
                "kind": kind
            }]
        })));
        assert_eq!(
            out["totals"]["parse_failed"], 0,
            "合成件解析不得失败：{}",
            out["totals"]
        );
    }
    let r = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": "PRJ-0001" })));
    let conflicts = r["conflicts"].as_array().cloned().unwrap_or_default();
    let warnings = r["warnings"]
        .as_array()
        .map(|a| a.iter().filter_map(|w| w.as_str().map(String::from)).collect())
        .unwrap_or_default();
    (conflicts, warnings)
}

/// 验收 1：kind=补遗、文件名**无 C- 前缀**（真实命名形态「澄清修改文件.docx」）
/// 的补遗件与主文件同条款号异文 → 冲突被检出。修复前
/// `starts_with("C-")` 判 false → 静默漏检（旧版条款可能被误响应）。
#[test]
fn real_named_addendum_conflict_detected_by_kind() {
    let (conflicts, warnings) =
        extract_with_addendum("real-name", "澄清修改文件.docx", "补遗");
    assert_eq!(
        conflicts.len(),
        1,
        "真名补遗（kind=补遗）的 1.1 同号异文必须检出（1.2 同文不误报）"
    );
    let c = &conflicts[0];
    assert!(c["a"].is_string() && c["b"].is_string(), "冲突行须带 a/b 双向登记: {c}");
    let reason = c["reason"].as_str().unwrap_or("");
    assert!(reason.contains("1.1"), "reason 须指认条款 1.1：{reason}");
    assert!(
        warnings.iter().any(|w| w.starts_with("CONFLICTS:")),
        "冲突面非空时 warnings 必须带 CONFLICTS 横幅：{warnings:?}"
    );
}

/// 验收 2：既有 C- 前缀夹具行为不变——C- 前缀 + kind=补遗 同样检出且
/// 不重复计数（kind 臂命中，前缀兜底臂不再叠加）。
#[test]
fn c_prefixed_addendum_behavior_unchanged() {
    let (conflicts, warnings) =
        extract_with_addendum("c-prefix", "C-补遗文件01.docx", "补遗");
    assert_eq!(
        conflicts.len(),
        1,
        "C- 前缀 + kind=补遗：恰检出 1.1 一处（1.2 同文不计，无重复登记）"
    );
    assert!(warnings.iter().any(|w| w.starts_with("CONFLICTS:")));
}

/// 验收 3：kind=答疑 不误入补遗面——同一份物理补遗（保留 C- 前缀名）登记为
/// 答疑时，同条款号异文**不得**按补遗冲突登记（答疑不是补遗，
/// TENDER_FILE_KINDS 值域对齐）。修复前 C- 前缀判定会把答疑件误入。
#[test]
fn qa_kind_not_misclassified_as_addendum() {
    let (conflicts, warnings) =
        extract_with_addendum("qa-kind", "C-补遗文件01.docx", "答疑");
    assert!(
        conflicts.is_empty(),
        "kind=答疑 的文件不得进补遗冲突面（误报 = Agent 浪费一轮人工核实）：{conflicts:?}"
    );
    assert!(
        !warnings.iter().any(|w| w.starts_with("CONFLICTS:")),
        "无冲突时不得挂 CONFLICTS 横幅：{warnings:?}"
    );
}
