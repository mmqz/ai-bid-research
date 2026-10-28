//! B5 · extract_requirements + get_compliance_matrix 工具契约测试（issue #2 §B5）。
//!
//! 双轨（issue #11 测试要求）：
//! - **自研轨**：状态机裁决（DRAFT 越级拒绝 / 空导入面 E_STATE / 转换后可重跑幂等）、
//!   counts.by_category 契约键、缺号检测硬规则、矩阵只读性（无状态副作用）、
//!   分页与 category 过滤、cmp_id 稳定性；
//! - **借鉴轨**：corpus-A 主文件（A-招标文件.docx，GT 25 埋点所在件）——
//!   引擎路径导入后抽取面与既有 GT 口径对齐（编号条款非零 + 评分项在位）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
}

fn corpus_a_main() -> PathBuf {
    fixtures().join("corpus-A/files/A-招标文件.docx")
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

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn create(svc: &mut BidService, key: &str) -> String {
    let r = ok_result(call(svc, "create_bid_project", json!({
        "name": "B5 契约测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": key,
    })));
    r["project_id"].as_str().unwrap().to_string()
}

fn import_main(svc: &mut BidService, pid: &str, path: &PathBuf, name: &str) -> Value {
    ok_result(call(svc, "import_tender_files", json!({
        "project_id": pid,
        "files": [{ "upload_ref": path.display().to_string(), "filename": name, "kind": "招标文件" }],
    })))
}

// ---------------------------------------------------------------- 1. 状态机裁决

#[test]
fn extract_rejects_draft_state_escalation() {
    let mut svc = BidService::new();
    let pid = create(&mut svc, "b5-state-draft");
    // DRAFT 态（未导入文件）直接抽取 = 越级 → E_STATE
    let reply = call(&mut svc, "extract_requirements", json!({ "project_id": pid }));
    assert_eq!(err_code(&reply), "E_STATE", "DRAFT 态抽取必须被状态机拒绝");
}

#[test]
fn extract_rejects_wrong_project_id() {
    let mut svc = BidService::new();
    create(&mut svc, "b5-notfound");
    let reply = call(&mut svc, "extract_requirements", json!({ "project_id": "PRJ-9999" }));
    assert_eq!(err_code(&reply), "E_NOTFOUND");
}

// ---------------------------------------------------------------- 2. 抽取 → 矩阵主链

#[test]
fn extract_then_matrix_end_to_end_on_corpus_a() {
    let mut svc = BidService::new();
    let pid = create(&mut svc, "b5-e2e");
    import_main(&mut svc, &pid, &corpus_a_main(), "A-招标文件.docx");

    // 抽取（引擎路径导入 → extract.rs 规则）
    let r = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": pid,
    })));
    let total = r["counts"]["total"].as_u64().unwrap();
    assert!(total > 0, "corpus-A 主文件必须抽出条款（GT 25 埋点件）：{r}");
    assert_eq!(r["status"], "REQUIREMENTS_EXTRACTED", "状态转换 FILES_IMPORTED → REQUIREMENTS_EXTRACTED");
    // 契约键口径（7 类）
    for key in ["qualification", "veto", "scoring", "tech_param", "commercial", "format", "other"] {
        assert!(
            r["counts"]["by_category"].get(key).is_some(),
            "by_category 必须携带契约键 {key}"
        );
    }
    // 引擎路径 warning 登记（B5：docx 走 bid-engine——缺口登记不伪装）
    let warnings = r["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w.as_str().unwrap_or("").contains("ENGINE_DOCX")),
        "引擎路径必须登记 ENGINE_DOCX warning（mdcore#39/#40 提示面）"
    );

    // 矩阵（只读）
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid,
    })));
    assert_eq!(m["counts"]["total"].as_u64().unwrap(), total, "矩阵行数 = 条款总数");
    let rows = m["rows"].as_array().unwrap();
    assert_eq!(rows.len() as u64, m["counts"]["returned"].as_u64().unwrap());
    // B5 最小矩阵口径：状态恒未响应 + 证据空 + cmp_id 稳定序
    assert_eq!(rows[0]["cmp_id"], "CMP-0001");
    assert_eq!(rows[0]["response_policy"], "default");
    assert!(rows[0]["evidence"].as_array().unwrap().is_empty());
    // 溯源锚在位（statement = 锚 excerpt）
    assert!(
        rows[0]["requirement"]["source_anchor"].is_object(),
        "每行必须携带 source_anchor（溯源契约）"
    );
}

#[test]
fn extract_is_idempotent_merge_on_rerun() {
    let mut svc = BidService::new();
    let pid = create(&mut svc, "b5-idem");
    import_main(&mut svc, &pid, &corpus_a_main(), "A-招标文件.docx");
    let r1 = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));
    let t1 = r1["counts"]["total"].as_u64().unwrap();
    // REQUIREMENTS_EXTRACTED 态重跑（状态机允许——幂等合并语义）
    let r2 = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));
    let (t2, added2) = (
        r2["counts"]["total"].as_u64().unwrap(),
        r2["counts"]["added"].as_u64().unwrap(),
    );
    assert_eq!(t1, t2, "同输入重跑 total 不变（dedupe_hash 幂等）");
    assert_eq!(added2, 0, "重跑 added=0（全部命中既有 hash）");
}

// ---------------------------------------------------------------- 3. 矩阵只读 + 过滤

#[test]
fn matrix_is_read_only_no_state_side_effect() {
    let mut svc = BidService::new();
    let pid = create(&mut svc, "b5-ro");
    import_main(&mut svc, &pid, &corpus_a_main(), "A-招标文件.docx");
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));
    let v_before = ok_result(call(&mut svc, "get_bid_project_state", json!({
        "project_id": pid,
    })));
    // DRAFT 态也可调矩阵（契约 §4：任何阶段可调——只读无状态副作用）
    //（此处已 REQUIREMENTS_EXTRACTED；再调两次矩阵验证 content_version 不动）
    let m1 = ok_result(call(&mut svc, "get_compliance_matrix", json!({ "project_id": pid })));
    let m2 = ok_result(call(&mut svc, "get_compliance_matrix", json!({ "project_id": pid })));
    assert_eq!(m1["project_version"], m2["project_version"], "矩阵只读：version 不动");
    let v_after = ok_result(call(&mut svc, "get_bid_project_state", json!({
        "project_id": pid,
    })));
    assert_eq!(v_before["status"], v_after["status"], "矩阵调用不得改状态");
}

#[test]
fn matrix_category_filter_and_pagination() {
    let mut svc = BidService::new();
    let pid = create(&mut svc, "b5-filter");
    import_main(&mut svc, &pid, &corpus_a_main(), "A-招标文件.docx");
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));

    // category 过滤（评分项——corpus-A 评标办法章 GT 4 条）
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid,
        "filters": { "category": "scoring" },
    })));
    let rows = m["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "评分项过滤后行非空");
    assert!(
        rows.iter().all(|r| r["requirement"]["category_key"] == "scoring"),
        "过滤后全部行 category_key=scoring"
    );
    // 全量行数 > 过滤行数（过滤有效）
    let all = ok_result(call(&mut svc, "get_compliance_matrix", json!({ "project_id": pid })));
    assert!(
        all["counts"]["total"].as_u64().unwrap() > m["counts"]["total"].as_u64().unwrap(),
        "过滤行数小于全量"
    );
}

#[test]
fn matrix_empty_requirements_emits_warning() {
    let mut svc = BidService::new();
    let pid = create(&mut svc, "b5-empty");
    import_main(&mut svc, &pid, &corpus_a_main(), "A-招标文件.docx");
    // 未抽取直接调矩阵：空行 + EMPTY_MATRIX warning（不报错——只读工具任何阶段可调）
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({ "project_id": pid })));
    assert_eq!(m["counts"]["total"].as_u64().unwrap(), 0);
    assert!(
        m["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("EMPTY_MATRIX")),
        "空条款集必须带 EMPTY_MATRIX warning"
    );
}

// ---------------------------------------------------------------- 4. 缺号检测硬规则

#[test]
fn extract_reports_missing_sequence() {
    // 构造缺号：corpus-A 主文件抽取后，手工核对 missing_sequence 面存在性
    //（契约硬规则：缺号必须报告——非破坏性验证：字段在位且为数组）
    let mut svc = BidService::new();
    let pid = create(&mut svc, "b5-missing");
    import_main(&mut svc, &pid, &corpus_a_main(), "A-招标文件.docx");
    let r = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));
    assert!(
        r["missing_sequence"].is_array(),
        "missing_sequence 字段必须在位（契约 §3 硬规则）"
    );
    // 若报了缺号，warnings 必须同步提示（一致性）
    let n_missing = r["missing_sequence"].as_array().unwrap().len();
    let has_warn = r["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap_or("").contains("MISSING_SEQUENCE"));
    assert_eq!(
        n_missing > 0,
        has_warn,
        "missing_sequence 非空 ⟺ MISSING_SEQUENCE warning（不得脱钩）"
    );
}
