//! #285 ② 块级双向索引三工具契约测试。
//!
//! 覆盖（契约面——真件验收见 block_bidi_acceptance.rs）：
//! - 路由/参数校验：未知 project / 未知 req / 未知 file / 越界 block_idx → E_NOTFOUND；
//!   缺 block_idx → E_UNPROCESSABLE
//! - provenance：抽取后每锚带 block_idx（回填值腿）；旧快照锚
//!   （block_idx 剥离——hash 不变幂等覆写）走 fallback_match 腿
//! - block 反查：有 REQ 块回 req_ids + verdict + marked_by；无 REQ 块
//!   req_ids=[] + verdict=null（无结论 ≠ noise——「还没看过」可见）
//! - overlay：total_blocks 对账 + req_bearing_blocks 与逐块反查一致 +
//!   semantic=status=口径 + 只读零副作用
//!
//! 真件：corpus-A 主文件（docx，真实块面；page/line 的 PDF 面验收见
//! block_bidi_acceptance.rs）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_ready(test_id: &str) -> bool {
    let main = fixtures().join("A-招标文件.docx");
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&main])
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

fn err_code(reply: ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期失败，实际成功: {}", out.result),
    }
}

fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-bidi-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 建项目 + 导入 corpus-A 主文件 + 抽取，返回 (svc, pid, fid)。
fn setup_extracted(tag: &str) -> Option<(BidService, String, String)> {
    if !corpus_ready(tag) {
        return None;
    }
    let ws = tmp_ws(tag);
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": "块级双向索引契约测试", "bid_deadline": "2026-12-31", "mode": "明标",
                "external_key": format!("bidi-contract-{tag}") }),
    ))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    let main = fixtures().join("A-招标文件.docx");
    let out = ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [
            { "upload_ref": main.display().to_string(), "filename": "A-招标文件.docx", "kind": "招标文件" }
        ]}),
    ));
    let fid = out["per_file"][0]["file_id"].as_str().unwrap().to_string();
    let extract = ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    assert!(
        extract["counts"]["total"].as_u64().unwrap_or(0) > 0,
        "corpus-A 主文件必须抽出 REQ（验收前提）: {extract}"
    );
    Some((svc, pid, fid))
}

/// 路由/参数校验：四类失败形态各自 Typed Error。
#[test]
fn routing_and_params_rejected() {
    let Some((mut svc, pid, fid)) = setup_extracted("routing") else {
        return;
    };
    // 未知 project → E_NOTFOUND（三工具同路由纪律）
    for tool in ["get_requirement_provenance", "get_block_requirements", "get_annotation_overlay"] {
        assert_eq!(
            err_code(call(&mut svc, tool, json!({ "project_id": "P-不存在", "req_id": "REQ-0001", "file_id": "TF-0001", "block_idx": 0 }))),
            "E_NOTFOUND",
            "{tool} 未知项目（参数全给——路由腿优先可见）"
        );
    }
    // 未知 req → E_NOTFOUND
    assert_eq!(
        err_code(call(
            &mut svc,
            "get_requirement_provenance",
            json!({ "project_id": &pid, "req_id": "REQ-9999" })
        )),
        "E_NOTFOUND"
    );
    // 未知 file → E_NOTFOUND
    assert_eq!(
        err_code(call(
            &mut svc,
            "get_block_requirements",
            json!({ "project_id": &pid, "file_id": "TF-9999", "block_idx": 0 })
        )),
        "E_NOTFOUND"
    );
    // 越界 block_idx → E_NOTFOUND
    assert_eq!(
        err_code(call(
            &mut svc,
            "get_block_requirements",
            json!({ "project_id": &pid, "file_id": &fid, "block_idx": 999_999 })
        )),
        "E_NOTFOUND"
    );
    // 缺 block_idx → E_UNPROCESSABLE（必填参数）
    assert_eq!(
        err_code(call(
            &mut svc,
            "get_block_requirements",
            json!({ "project_id": &pid, "file_id": &fid })
        )),
        "E_UNPROCESSABLE"
    );
}

/// provenance 抽取态：每锚带 block_idx（回填值腿）+ marked_by 口径。
#[test]
fn provenance_returns_backfilled_block_idx() {
    let Some((mut svc, pid, _fid)) = setup_extracted("prov") else {
        return;
    };
    let req_id = svc.store().requirements().first().expect("抽取后必有 REQ").id.clone();
    let out = ok_result(call(
        &mut svc,
        "get_requirement_provenance",
        json!({ "project_id": &pid, "req_id": &req_id }),
    ));
    assert_eq!(out["req_id"], json!(req_id));
    assert!(
        out["category"].is_string() && !out["category"].as_str().unwrap().is_empty()
    );
    assert!(
        out["marked_by"] == json!("regex") || out["marked_by"] == json!("agent"),
        "来源面两值口径: {}",
        out["marked_by"]
    );
    let anchors = out["anchors"].as_array().expect("锚数组");
    assert!(!anchors.is_empty(), "抽取 REQ 必有锚: {out}");
    for a in anchors {
        // corpus-A（docx）：块位回填值必须在场（resolved_via=backfilled）
        assert_eq!(
            a["resolved_via"], json!("backfilled"),
            "新抽取锚走回填腿: {a}"
        );
        assert!(
            a["block_idx"].is_u64(),
            "回填腿必带块位: {a}"
        );
        // docx 章节路径面无页行概念——诚实 null（PDF 面见真件验收）
        assert!(a["page"].is_null() && a["line"].is_null(), "docx 页行 = null: {a}");
    }
}

/// provenance 旧快照腿：剥 block_idx/page/line（hash 不变）幂等覆写后，
/// (locator, excerpt) 兜底解析必须回块位（resolved_via=fallback_match）。
#[test]
fn provenance_legacy_anchor_resolves_via_fallback() {
    let Some((mut svc, pid, _fid)) = setup_extracted("legacy") else {
        return;
    };
    // 模拟旧快照：全量 REQ 锚的块级字段剥回 None（requirement_hash 只含
    // file|locator|excerpt——覆写不增条，同位替换）
    let reqs: Vec<bid_domain::Requirement> = svc
        .store()
        .requirements()
        .to_vec()
        .into_iter()
        .map(|mut r| {
            for a in &mut r.anchors {
                a.block_idx = None;
                a.page = None;
                a.line = None;
            }
            r
        })
        .collect();
    let n = reqs.len();
    svc.store_mut()
        .set_requirements(reqs)
        .expect("幂等覆写（hash 不变）");
    assert_eq!(svc.store().requirements().len(), n, "同位覆写不增条");

    // 找一条 (locator, excerpt) 能兜底命中的锚（docx para 锚 excerpt=块全文）
    let req_ids: Vec<String> = svc.store().requirements().iter().map(|r| r.id.clone()).collect();
    let mut hit = None;
    let mut unresolved = 0usize;
    let mut matched = 0usize;
    for req_id in &req_ids {
        let out = ok_result(call(
            &mut svc,
            "get_requirement_provenance",
            json!({ "project_id": &pid, "req_id": req_id }),
        ));
        for a in out["anchors"].as_array().unwrap() {
            match a["resolved_via"].as_str().unwrap_or("missing") {
                "fallback_match" => {
                    matched += 1;
                    hit = hit.or(Some((req_id.clone(), a.clone())));
                }
                "unresolved" => unresolved += 1,
                other => panic!("旧锚不得走 {other} 腿: {a}"),
            }
        }
    }
    assert!(matched > 0, "corpus-A 抽取锚必须存在可兜底命中的（全 unresolved={unresolved}）");
    let Some((req_id, a)) = hit else {
        panic!(
            "corpus-A 抽取锚必须存在可兜底命中的（全 unresolved={unresolved}——抽查)"
        );
    };
    assert!(a["block_idx"].is_u64(), "兜底腿必回块位: {req_id} {a}");
}

/// 块反查双向一致：overlay 的 req_bearing 块逐块对 req_ids；无 REQ 块
/// 照常返回（req_ids=[] + verdict=null——「还没看过」≠noise）。
#[test]
fn block_reverse_query_consistent_with_overlay() {
    let Some((mut svc, pid, fid)) = setup_extracted("reverse") else {
        return;
    };
    let overlay = ok_result(call(
        &mut svc,
        "get_annotation_overlay",
        json!({ "project_id": &pid, "file_id": &fid }),
    ));
    let total = overlay["total_blocks"].as_u64().unwrap();
    assert!(total > 0, "解析产物必有块");
    let req_bearing = overlay["req_bearing_blocks"].as_u64().unwrap();
    assert!(req_bearing > 0, "corpus-A 抽取后必有 REQ 锚定块");

    let entries = overlay["overlay"].as_array().unwrap();
    let mut checked = 0usize;
    for e in entries {
        let block_idx = e["block_idx"].as_u64().unwrap();
        let out = ok_result(call(
            &mut svc,
            "get_block_requirements",
            json!({ "project_id": &pid, "file_id": &fid, "block_idx": block_idx }),
        ));
        assert_eq!(out["block_idx"], json!(block_idx));
        assert_eq!(out["text"], e["text"], "块原文一致");
        assert_eq!(out["req_ids"], e["req_ids"], "块→REQ 与 overlay 一致");
        let req_ids = out["req_ids"].as_array().unwrap();
        if !req_ids.is_empty() {
            assert_eq!(out["verdict"], json!("requirement"), "REQ 块 verdict");
            assert_eq!(out["marked_by"], json!("regex"), "corpus-A 正则抽取面");
        } else {
            assert!(
                out["verdict"].is_null(),
                "无 REQ 无标记块 verdict=null（还没看过≠noise）: {out}"
            );
        }
        // 双向闭合：REQ 块的 req_ids 逐条回 provenance 必含该块位
        for rid in req_ids {
            let prov = ok_result(call(
                &mut svc,
                "get_requirement_provenance",
                json!({ "project_id": &pid, "req_id": rid }),
            ));
            assert!(
                prov["anchors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a["block_idx"] == json!(block_idx)),
                "双向闭合：{rid} provenance 必含块 {block_idx}"
            );
        }
        checked += 1;
        if checked >= 20 {
            break; // 明细抽样 20 块（全量对账见真件验收）
        }
    }
    // overlay 口径：semantic 三值（emphasis=★真值 > requirement > null）+ status；
    // corpus-A 无★字面 → emphasis/suspected_miss 恒零（① 的 corpus-A 口径）
    for e in entries.iter().take(20) {
        let sem = &e["semantic"];
        assert!(
            sem.is_null() || *sem == json!("requirement"),
            "corpus-A 无★——semantic 不得出 emphasis: {e}"
        );
        assert_eq!(e["status"], json!("valid"));
        assert_eq!(e["suspected_miss"], json!(false), "无★件恒 false");
    }
    // 与 #285① 工具同源对账：star_missing == suspected_miss_blocks（块位粒度）
    let star = ok_result(call(
        &mut svc,
        "get_star_coverage",
        json!({ "project_id": &pid, "file_id": &fid }),
    ));
    assert_eq!(
        star["star_truth"], json!(0),
        "corpus-A 主文件无★真值（① 验收口径）: {star}"
    );
    assert_eq!(overlay["suspected_miss_blocks"], json!(0));
}

/// 只读零副作用：三工具查前后 project_version 不动。
#[test]
fn read_only_no_version_bump() {
    let Some((mut svc, pid, fid)) = setup_extracted("readonly") else {
        return;
    };
    let before = ok_result(call(&mut svc, "get_bid_project_state", json!({ "project_id": &pid })))
        ["project_version"]
        .clone();
    let _ = ok_result(call(
        &mut svc,
        "get_annotation_overlay",
        json!({ "project_id": &pid, "file_id": &fid }),
    ));
    let mid = ok_result(call(&mut svc, "get_bid_project_state", json!({ "project_id": &pid })))
        ["project_version"]
        .clone();
    let req_id = svc.store().requirements().first().expect("抽取后必有 REQ").id.clone();
    let _ = ok_result(call(
        &mut svc,
        "get_requirement_provenance",
        json!({ "project_id": &pid, "req_id": req_id }),
    ));
    let _ = ok_result(call(
        &mut svc,
        "get_block_requirements",
        json!({ "project_id": &pid, "file_id": &fid, "block_idx": 0 }),
    ));
    let after = ok_result(call(&mut svc, "get_bid_project_state", json!({ "project_id": &pid })))
        ["project_version"]
        .clone();
    assert_eq!(before, mid, "overlay 只读");
    assert_eq!(before, after, "三工具只读，不得 bump project_version");
}
