//! scoring_coverage_tool.rs —— #318 评标细则逐条响应覆盖检查（MCP 工具层）。
//!
//! 验收对照（issue #318）：
//! 1. 细则拆点 + 覆盖检查 + 报告（合成夹具——issue 原文例的 A/B/C 三点，
//!    正文写 A/B 漏 C → 2/3 覆盖、建议扣分 10/3、漏点归因到点）；
//! 2. **songling 真件门**（环境门 poppler + fixture）：真实 20 条 L2 细则
//!    走全链（导入 → 抽取 → 评分驱动建纲 → 覆盖检查）——拆点与报告结构
//!    在真实评分表上成立（映射在场、点形态合法、恒等式不漏不重）。
//!
//! 只报不补纪律（#285 同源）：本工具是诊断面——不自动改稿。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, ParseStatus, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use bid_parse::env_preflight::{skip_if_env_missing, skip_if_fixture_missing, POPPLER_TEXT_TOOLS};
use bid_tools::service::ImportSpec;
use serde_json::{json, Value};

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}
fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-318-mcp-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 合成夹具：一条评分细则（issue 原文例）+ 映射章节 + 半覆盖正文。
fn synthetic_project(tag: &str) -> (BidService, String) {
    let dir = tmp_dir(tag);
    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "318 细则覆盖", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": &format!("ext-318-{tag}"),
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut()
        .upsert_file("A-招标文件.docx", "招标文件", &format!("sha-318-{tag}"), 10, 1, 50, 0, ParseStatus::Ok, "")
        .unwrap();
    let req = Requirement {
        id: "REQ-0001".into(),
        category: "评分项".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "table".into(),
            locator: "评分表/行3".into(),
            excerpt: "技术方案完整性".into(),
        }],
        params: [
            ("score".to_string(), json!(10.0)),
            (
                "score_rules".to_string(),
                json!("技术方案完整性（10分）：需包含总体架构、实施步骤、风险应对。"),
            ),
            ("section".to_string(), json!("SEC-0001")),
        ]
        .into_iter()
        .collect(),
        source_file: "A-招标文件.docx".into(),
    };
    // 否决项一条（无分值——不计建议扣分但如实进报告）
    let veto = Requirement {
        id: "REQ-0002".into(),
        category: "否决项".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第一章/2".into(),
            excerpt: "投标保证金".into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    };
    svc.set_requirements(vec![req, veto]).unwrap();
    svc.store_mut().set_outline(vec![
        OutlineNode {
            id: "SEC-0001".into(), parent_id: None, title: "技术方案".into(),
            level: 1, order: 1, requirement_ids: vec!["REQ-0001".into()],
            target_chars: 500, status: SectionStatus::Pending, dirty: false,
            // 正文写 A/B 漏 C（issue 例的漏点面）
            content: Some("3.1 总体架构：本章给出系统总体架构设计。3.2 实施步骤：分三期推进。".into()),
            content_attempt: 1,
        },
    ]).unwrap();
    svc.store_mut().build_matrix().unwrap();
    (svc, pid)
}

// ---------------------------------------------------------------- 1. 合成夹具：拆点/覆盖/报告

#[test]
fn synthetic_full_coverage_semantics() {
    let (mut svc, pid) = synthetic_project("syn");

    let r = ok_result(call(&mut svc, "check_scoring_coverage", json!({
        "project_id": &pid,
    })));
    let summary = &r["summary"];
    assert_eq!(summary["criteria_total"], json!(2), "评分项+否决项都进报告");
    assert_eq!(summary["criteria_with_points"], json!(1), "否决项无枚举形态");
    assert_eq!(summary["points_total"], json!(3), "issue 例三点全拆出");
    assert_eq!(summary["points_covered"], json!(2), "A/B 命中、C 漏");
    let ratio = summary["coverage_ratio"].as_f64().unwrap();
    assert!((ratio - 2.0 / 3.0).abs() < 1e-9, "覆盖率 2/3：{ratio}");
    let deduction = summary["suggested_deduction"].as_f64().unwrap();
    assert!((deduction - 10.0 / 3.0).abs() < 1e-9, "建议扣分 10×1/3：{deduction}");

    // 逐点断言（issue 验收 3/4/5：哪个点在哪个章节 / 未覆盖明确列出）
    let c = r["criteria"].as_array().unwrap().iter()
        .find(|c| c["criterion_id"] == json!("REQ-0001")).unwrap();
    let pts = c["points"].as_array().unwrap();
    assert_eq!(pts[0]["point"], json!("总体架构"));
    assert_eq!(pts[0]["hit_section"], json!("SEC-0001"));
    assert_eq!(pts[1]["point"], json!("实施步骤"));
    assert_eq!(pts[1]["hit_section"], json!("SEC-0001"));
    assert_eq!(pts[2]["point"], json!("风险应对"));
    assert_eq!(pts[2]["hit_section"], Value::Null, "漏点面：C 未命中");
    assert_eq!(c["mapped_sections"], json!(["SEC-0001"]), "RuleParsed 映射在场");
    // notes：口径与局限明示（诚实面）
    let notes = r["notes"].as_array().unwrap();
    assert!(notes.iter().any(|n| n.as_str().unwrap().contains("字面")));
    assert!(notes.iter().any(|n| n.as_str().unwrap().contains("未拆出点")));
    // next 指引漏点处置
    assert!(r["next"].as_str().unwrap().contains("漏点"));
}

// ---------------------------------------------------------------- 2. songling 真件门（环境门）

/// 层1 互斥锁（与 songling_scoring_outline 同因：4G/2核沙箱 PDF 解析内存约束）。
fn engine_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn fixtures() -> std::path::PathBuf {
    let p = std::env::var("BID_FIXTURES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures"));
    std::fs::canonicalize(&p).unwrap_or(p)
}

#[test]
fn songling_real_corpus_scoring_coverage_gate() {
    if skip_if_env_missing("songling_318_scoring_coverage", POPPLER_TEXT_TOOLS) {
        return;
    }
    let _guard = engine_lock().lock().unwrap();
    let pdf = fixtures().join("songling-232762/tender/光纤租赁费招标文件.pdf");
    if skip_if_fixture_missing("songling_318_scoring_coverage", &[&pdf]) {
        return;
    }

    // 全链：导入 → 抽取 → 建矩阵 → 评分驱动建纲（#252——细则带映射）
    let mut svc = BidService::new().with_fs_root(fixtures());
    svc.create_project("松岭细则覆盖", "[232762]LJXM[GK]20260001", "明标", "sl-318")
        .expect("建项");
    let report = svc
        .import(&[ImportSpec {
            upload_ref: pdf,
            filename: "光纤租赁费招标文件.pdf".to_string(),
            kind: "招标文件".to_string(),
            expected_pages: None,
        }])
        .expect("导入");
    assert_eq!(report.item_failures, 0, "导入零失败");
    let reqs = bid_parse::extract::extract_requirements(svc.parsed());
    let _ = svc.set_requirements(reqs).expect("抽取落库");
    svc.store_mut().build_matrix().expect("矩阵");
    let _ = svc
        .store_mut()
        .derive_and_set_outline(bid_domain::outline::OutlineSpec::default())
        .expect("建纲");

    let pid = svc.store().project().unwrap().id.clone();
    let r = ok_result(call(&mut svc, "check_scoring_coverage", json!({
        "project_id": &pid,
    })));
    let summary = &r["summary"];

    // 结构门（真件实测值不锁死数——锁形态与恒等式）：
    let criteria_total = summary["criteria_total"].as_u64().unwrap();
    let with_points = summary["criteria_with_points"].as_u64().unwrap();
    let points_total = summary["points_total"].as_u64().unwrap();
    let covered = summary["points_covered"].as_u64().unwrap();
    assert!(criteria_total >= 20, "评分项 ≥ 20（真值锚见 #252 期望形态）：{criteria_total}");
    assert!(with_points >= 5, "真件细则至少 5 条拆出点（顿号枚举是常见形态）：{with_points}");
    assert!(points_total >= with_points, "恒等式：点数 ≥ 拆出点细则数");
    assert!(covered <= points_total, "恒等式：覆盖 ≤ 总点");
    // 空稿（未起草）→ 覆盖应为 0（标题不含点——如实，不伪装）
    assert_eq!(covered, 0, "未起草全量正文——零覆盖（诚实面）");

    // 点形态合法（2..=12 CJK——无数字/标点残段）
    let mut unmapped_in_list = 0usize;
    for c in r["criteria"].as_array().unwrap() {
        for p in c["points"].as_array().unwrap() {
            let point = p["point"].as_str().unwrap();
            let chars: Vec<char> = point.chars().collect();
            assert!(chars.len() >= 2 && chars.len() <= 12, "点形态：{point:?}");
            assert!(point.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "含 CJK：{point:?}");
        }
        // 未映射细则是既有已知面（#224 unmapped_scoring_items 诊断——业绩/价格
        // 独立响应位等不进章节映射）——只校验 summary 与明细一致，不强制全映射
        if c["mapped_sections"].as_array().unwrap().is_empty() {
            unmapped_in_list += 1;
        }
    }
    assert_eq!(
        unmapped_in_list as u64, summary["criteria_unmapped"].as_u64().unwrap(),
        "summary.criteria_unmapped 与明细一致（不漏不重）"
    );

    // 摘要回显 + 口径 notes 在场
    assert!(r["notes"].as_array().unwrap().len() >= 2, "口径与局限随报告透出");
}
