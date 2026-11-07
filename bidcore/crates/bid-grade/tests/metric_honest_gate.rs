//! #481（A-20①）：metric 断言诚实面门禁——恒真洗白的回归锁。
//!
//! 背景：`interpret_assert` 的 metric 分支曾恒真 `pass(true)`——TC-10 的
//! A/B 检索质量断言从未验证却计入全绿（与 Python 判分器 grade.py:441
//! 的 `"pass": None` 口径分裂）。#481 对齐诚实口径：`pass: null` +
//! `skipped: true` + 计入 summary.skipped——未判定不得洗白成通过，
//! 也不作发布门禁（TC-10 夹具自注「归档型用例」）。

use bid_grade::fixtures::Fixtures;
use bid_grade::grade::grade_transitions;
use bid_tools::schema::RunDb;
use serde_json::json;

fn fixtures_loaded() -> Option<Fixtures> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let fixtures_dir = std::env::var("BID_FIXTURES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| root.join("fixtures"));
    let corpus_a = fixtures_dir.join("corpus-A/files");
    if bid_parse::env_preflight::skip_if_fixture_missing("metric_honest_gate", &[&corpus_a]) {
        return None;
    }
    Some(Fixtures::load(&fixtures_dir).expect("夹具装载失败"))
}

/// 最小 RunDb（TC-10 场景）——metric 分支不消费 db 字段，只需构造合法。
fn tc10_db() -> RunDb {
    serde_json::from_value(json!({
        "scenario": "TC-10", "milestone": "full",
        "project": {"status": "EXPORTED", "content_version": 2},
        "files": [], "totals": {"files": 0, "chunks": 0, "parse_failed": 0, "ocr_required": 0},
        "parse": {"missing_sequence": []},
        "requirements": [], "matrix": [], "findings": [], "conflicts": [],
        "sections": [], "recheck": {},
        "last_tool_result": {"tool": "search_evidence", "ok": true}
    }))
    .expect("最小 RunDb 构造")
}

/// 主断言：TC-10 的 metric 断言 → pass=null + skipped=true；
/// summary.pass 不含 metric、summary.skipped 含 metric。
#[test]
fn tc10_metric_assert_is_null_pass_and_skipped() {
    let Some(fx) = fixtures_loaded() else { return };
    assert!(
        fx.scenarios.iter().any(|s| s.tc_id == "TC-10"),
        "夹具须含 TC-10 场景（expected-transitions/TC-10.json）"
    );
    let db = tc10_db();
    let out = grade_transitions(&db, &fx, Some("TC-10"));
    let detail = out.detail();
    let asserts = detail["asserts"].as_array().expect("asserts 数组");
    let metrics: Vec<&serde_json::Value> =
        asserts.iter().filter(|a| a["kind"] == "metric").collect();
    assert!(
        !metrics.is_empty(),
        "TC-10 须含 metric 断言（夹具契约）——实际 asserts: {asserts:?}"
    );
    for m in &metrics {
        assert_eq!(
            m["pass"], serde_json::Value::Null,
            "metric 断言 pass 须为 null（未判定）——不得恒真洗白。实际: {m:?}"
        );
        assert_eq!(m["skipped"], json!(true), "metric 须显式标 skipped：{m:?}");
    }
    // summary：pass 计数不含 metric；skipped 计数含
    let summary = &detail["summary"];
    let metric_n = metrics.len() as u64;
    assert_eq!(
        summary["skipped"].as_u64().unwrap_or(0), metric_n,
        "summary.skipped 须计入全部 metric 断言——实际 summary: {summary:?}"
    );
    assert_eq!(
        summary["pass"].as_u64().unwrap_or(0),
        (asserts.len() as u64) - metric_n,
        "summary.pass 只计非 metric 断言（未判定不洗白）——实际: {summary:?}"
    );
    // 恒真回归锁：整体不为「全 pass」——skipped 在场必须可见
    assert!(
        out.detail()["skipped"].as_array().map(|a| a.len()).unwrap_or(0) >= 1,
        "外层 skipped 清单须登记 metric（显式跳过非静默）"
    );
}

/// 口径对齐锁：TC-10 场景不被 metric 拖成 FAIL（null 不作门禁）——
/// 「归档型用例不作发布门禁」的既有语义保持。
#[test]
fn tc10_metric_null_does_not_fail_the_dimension() {
    let Some(fx) = fixtures_loaded() else { return };
    let db = tc10_db();
    let out = grade_transitions(&db, &fx, Some("TC-10"));
    assert!(
        out.is_pass(),
        "TC-10 维度不应被 null-pass 的 metric 判负（未判定 ≠ 失败）——实际 {:?}",
        out.detail()["summary"]
    );
}
