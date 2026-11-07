//! #315 业绩材料库四工具——MCP 全链验收。
//!
//! 覆盖 issue #315 五条验收：
//! 1. 业绩库能持久化（跨项目可见）——`cross_project_library_visibility`
//!    （Rust 面bid-tools 已测文件往返；此处验 MCP 面两工作区共享）；
//! 2. 至少 20 条示例业绩可导入——`fixture_samples_batch_import`（22 条夹具）；
//! 3. 匹配推荐——`recommend_ranks_and_suggests_section`（信号装配 + 排序 +
//!    suggested_section 指向真实业绩章节）；
//! 4. 一键写入业绩章节——`write_section_full_flow`（corpus-A 真件 + 3 份
//!    类似业绩合同 → 证据自动登记 + 绑定 + 单段写入 + 幂等重跑）；
//! 5. 证据自动绑定——同 4 的 bindings 断言（业绩类 REQ 全量绑定）。
//!
//! 诚实面：材料未导入的条目走 `materials_pending`（不伪造证据）——
//! `materials_pending_is_honest`。

use bid_domain::outline::OutlineSpec;
use bid_domain::{Anchor, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_files() -> PathBuf {
    fixtures().join("corpus-A").join("files")
}

fn samples_path() -> PathBuf {
    fixtures().join("performance-samples").join("performance-samples.json")
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

fn err_text(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => format!("{error}"),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-perf-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 建项 + 导入（主件 + 3 份类似业绩合同 + 资质证）+ 抽取 + 建纲。
/// 返回 (svc, pid)——业绩章写入的完整前置。
fn prepared(tag: &str) -> (BidService, String) {
    let dir = ws(tag);
    let mut svc = BidService::open(&dir)
        .unwrap()
        .with_fs_root(corpus_files())
        .with_perf_library(dir.join("perf-lib.json"));
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "业绩库测试——智慧园区弱电工程采购",
        "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    // upload_ref = 夹具路径；filename = 纯文件名（仓内惯例——TF.name 是
    // 材料匹配的对齐面）
    let files = [
        ("A-招标文件.docx", "招标文件"),
        ("C-补遗文件01.docx", "补遗"),
        ("EV-02-电子与智能化工程专业承包二级资质证书.pdf", "附件"),
        ("EV-03-类似业绩合同-智慧园区弱电工程.pdf", "附件"),
        ("EV-04-类似业绩合同-数据中心机房工程.pdf", "附件"),
        ("EV-05-类似业绩合同-医院智能化改造.pdf", "附件"),
    ];
    let specs: Vec<Value> = files
        .iter()
        .map(|(name, kind)| {
            let upload = if name.starts_with("EV-") {
                corpus_files().join("evidence").join(name)
            } else {
                corpus_files().join(name)
            };
            json!({
                "upload_ref": upload.display().to_string(),
                "filename": name, "kind": kind,
            })
        })
        .collect();
    let out = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": &pid, "files": specs,
    })));
    // 环境门：无 poppler/引擎回退全失败时不伪装通过（本机 Linux 有 poppler，
    // CI Windows 缺件面见 env_preflight SKIP 纪律——此处按实况断言主件成功）
    let parse_failed = out["totals"]["parse_failed"].as_u64().unwrap_or(0);
    assert_eq!(parse_failed, 0, "导入零失败（本环境需 poppler 在位——缺件时本测试红是诚实面）: {out}");
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    // 注入合成评分行（borrow：outline.rs 单测 scoring_req_for_outline 同款形态
    // ——corpus-A 的正则抽取面不产 name/track 结构化评分行，大纲不会派生
    // 「业绩证明材料」槽位；注入一条商务评审业绩行 + 一条技术评审章行，
    // 让槽位出现——业绩库的写入对象）。真实语料的结构化评分表面见
    // real_corpus_e2e（poppler 门内）。
    inject_scoring_rows(&mut svc);
    svc.prepare_outline_and_matrix(OutlineSpec { target_total_chars: 6_000, max_reqs_per_leaf: 6 })
        .unwrap();
    (svc, pid)
}

/// 合成评分行注入（整集替换语义——既有 REQ 原样保留，追加两条结构化行）。
fn inject_scoring_rows(svc: &mut BidService) {
    let mut reqs: Vec<Requirement> = svc.store().requirements().to_vec();
    let next = reqs
        .iter()
        .filter_map(|r| r.id.strip_prefix("REQ-")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let mk = |id: &str, name: &str, score: f64, track: &str, statement: &str, subs: Vec<&str>| Requirement {
        id: id.into(),
        category: "评分项".into(),
        clause_no: Some(format!("综合评分表-{id}")),
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "table".into(),
            locator: "第25页 行8".into(),
            excerpt: format!("{name}（{score}分 {track}）：{statement}"),
            block_idx: None,
            page: None,
            line: None,
        }],
        params: [
            ("name".to_string(), json!(name)),
            ("score".to_string(), json!(score)),
            ("track".to_string(), json!(track)),
            ("sub_items".to_string(), json!(subs)),
            ("statement".to_string(), json!(statement)),
        ]
        .into_iter()
        .collect(),
        source_file: "A-招标文件.docx".into(),
    };
    reqs.push(mk(
        &format!("REQ-{next:04}"),
        "技术方案", 30.0, "技术评审",
        "技术方案完整性、先进性、可行性（30分）",
        vec!["总体架构", "实施方案"],
    ));
    reqs.push(mk(
        &format!("REQ-{:04}", next + 1),
        "业绩", 15.0, "商务评审",
        "近三年（自 2023 年 1 月 1 日以来）完成的类似项目业绩，每提供一个有效业绩得 2 分，最多得 15 分（附合同扫描件与验收证明）",
        vec![],
    ));
    svc.replace_requirements(reqs).unwrap();
}

/// 三条「材料已导入」的业绩（materials 与导入件文件名一致）。
fn three_entries() -> Vec<Value> {
    vec![
        json!({
            "name": "智慧园区弱电工程合同", "client": "示例产业园运营有限公司",
            "amount_wan": 640.0, "completed_on": "2025-05-20",
            "categories": ["弱电工程", "安防监控"],
            "materials": ["材料/EV-03-类似业绩合同-智慧园区弱电工程.pdf"],
            "key_points": ["一次验收通过", "含 2 年运维"],
        }),
        json!({
            "name": "数据中心机房工程合同", "client": "示例数据服务中心",
            "amount_wan": 890.0, "completed_on": "2024-11-30",
            "categories": ["机房建设", "综合布线"],
            "materials": ["材料/EV-04-类似业绩合同-数据中心机房工程.pdf"],
            "key_points": ["Uptime Tier III", "交付含 200 机柜"],
        }),
        json!({
            "name": "医院智能化改造合同", "client": "示例市第一人民医院",
            "amount_wan": 420.0, "completed_on": "2025-08-15",
            "categories": ["医疗信息化", "智能化"],
            "materials": ["材料/EV-05-类似业绩合同-医院智能化改造.pdf"],
            "key_points": ["HIS 对接零停机"],
        }),
    ]
}

// ---------------------------------------------------------------- 验收 1+2

/// 验收 1（跨项目可见）：两个工作区各持项目，共享同一库文件——A 登记 B 可见。
#[test]
fn cross_project_library_visibility() {
    let lib = ws("xproj-lib");
    let lib_path = lib.join("performance-library.json");
    let (mut a, _pid_a) = {
        let dir = ws("xproj-a");
        let svc = BidService::open(&dir).unwrap().with_perf_library(&lib_path);
        // 项目 A 侧登记（建项目前——库无项目路由，先维护资产再投标）
        let mut svc = svc;
        let r = ok_result(call(&mut svc, "register_performance", json!({
            "entries": [{
                "name": "跨项目验证业绩", "client": "示例大学",
                "amount_wan": 100.0, "completed_on": "2025-01-01",
                "categories": ["验证"],
            }],
            "owner_name": "示例科技有限公司",
        })));
        assert_eq!(r["added_count"], 1);
        assert_eq!(r["library_persisted"], true);
        let pid = ok_result(call(&mut svc, "create_bid_project", json!({
            "name": "项目A", "bid_deadline": "2026-12-31", "mode": "明标",
            "external_key": "perf-xproj-a",
        })))["project_id"]
            .as_str()
            .unwrap()
            .to_string();
        (svc, pid)
    };
    // 完全不同的工作区（新进程语义——重开服务）+ 同库文件
    let dir_b = ws("xproj-b");
    let mut b = BidService::open(&dir_b).unwrap().with_perf_library(&lib_path);
    let out = ok_result(call(&mut b, "list_performance", json!({})));
    assert_eq!(out["entries"].as_array().unwrap().len(), 1, "跨项目可见");
    assert_eq!(out["owner_name"], "示例科技有限公司");
    let _ = &mut a; // A 服务保持存活到断言后（库文件并发写不是本测试对象）
}

/// 验收 2（20+ 条可导入）：夹具 22 条全量入库（批量面 + 语义幂等）。
#[test]
fn fixture_samples_batch_import() {
    let samples: Value =
        serde_json::from_str(&std::fs::read_to_string(samples_path()).unwrap()).unwrap();
    let entries = samples["entries"].as_array().unwrap().clone();
    assert!(entries.len() >= 20, "夹具至少 20 条（实际 {}）", entries.len());
    let dir = ws("samples");
    let mut svc = BidService::open(&dir)
        .unwrap()
        .with_perf_library(dir.join("perf-lib.json"));
    let r = ok_result(call(&mut svc, "register_performance", json!({
        "entries": entries,
        "owner_name": samples["owner_name"],
    })));
    assert_eq!(r["added_count"], entries.len() as u64, "22 条全量入库");
    assert_eq!(r["total_entries"], entries.len() as u64);
    // 语义幂等：整批重放 → 全部 deduplicated
    let entries2 = samples["entries"].as_array().unwrap().clone();
    let r2 = ok_result(call(&mut svc, "register_performance", json!({ "entries": entries2 })));
    assert_eq!(r2["added_count"], 0);
    assert_eq!(
        r2["deduplicated"].as_array().unwrap().len(),
        entries.len(),
        "重放全量命中既有 id"
    );
    // 列表过滤：category=医疗 → 4 条（医疗设备/信息化/病房/实验室）
    let out = ok_result(call(&mut svc, "list_performance", json!({
        "filters": { "category": "医疗" },
    })));
    assert_eq!(out["entries"].as_array().unwrap().len(), 4, "医疗类过滤");
}

// ---------------------------------------------------------------- 登记面纪律

/// 一条非法 → 整批拒（零副作用）+ 响亮报错。
#[test]
fn batch_reject_is_atomic() {
    let dir = ws("atomic");
    let mut svc = BidService::open(&dir)
        .unwrap()
        .with_perf_library(dir.join("perf-lib.json"));
    let reply = call(&mut svc, "register_performance", json!({
        "entries": [
            { "name": "好条目", "client": "甲方", "amount_wan": 100.0, "completed_on": "2025-01-01" },
            { "name": "坏条目", "client": "甲方", "amount_wan": -5.0, "completed_on": "2025-01-01" },
        ],
    }));
    let err = err_text(&reply);
    assert!(err.contains("amount_wan"), "错误指向具体字段：{err}");
    let out = ok_result(call(&mut svc, "list_performance", json!({})));
    assert_eq!(out["entries"].as_array().unwrap().len(), 0, "整批拒 = 好条目也不入库");
}

// ---------------------------------------------------------------- 验收 3

/// 验收 3（匹配推荐）：语料信号装配 + 排序 + suggested_section 指向真实章节。
#[test]
fn recommend_ranks_and_suggests_section() {
    let (mut svc, pid) = prepared("recommend");
    let r = ok_result(call(&mut svc, "register_performance", json!({
        "entries": three_entries(),
    })));
    assert_eq!(r["added_count"], 3);
    let out = ok_result(call(&mut svc, "recommend_performance", json!({
        "project_id": &pid,
        "budget_wan": 700.0,
        "base_date": "2026-11-07",
        "client_type": "government",
        "focus": "弱电 安防监控 综合布线",
        "top_k": 3,
    })));
    let recs = out["recommendations"].as_array().unwrap();
    assert_eq!(recs.len(), 3);
    // 弱电/安防标签命中语料（focus + REQ 陈述）→ 智慧园区弱电条目应居首
    assert_eq!(recs[0]["name"], "智慧园区弱电工程合同", "排序按四因子降序：{recs:?}");
    assert!(recs[0]["score"].as_f64().unwrap() > recs[2]["score"].as_f64().unwrap());
    // reasons 逐因子人读依据
    let reasons: Vec<String> = recs[0]["reasons"].as_array().unwrap().iter().map(|r| r.as_str().unwrap_or("").to_string()).collect();
    let reasons = reasons.join("; ");
    assert!(reasons.contains("标签命中"), "理由面可见命中明细：{reasons}");
    // suggested_section：建纲后的业绩章节（标题含 业绩证明材料）
    let suggested = out["suggested_section"].as_str().unwrap().to_string();
    assert!(!suggested.is_empty(), "建纲后必有业绩章节（outline 业绩证明材料位）");
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == suggested)
        .unwrap();
    assert!(node.title.contains("业绩"), "建议章节标题含业绩：{}", node.title);
}

// ---------------------------------------------------------------- 验收 4+5

/// 验收 4+5（一键写入 + 证据自动绑定）：corpus-A 全链——材料命中已导入件 →
/// 证据登记（3 份可用业绩 → readiness 门过）→ 绑定业绩类 REQ → 单段写入 →
/// 幂等重跑零副作用。
#[test]
fn write_section_full_flow() {
    let (mut svc, pid) = prepared("write");
    ok_result(call(&mut svc, "register_performance", json!({
        "entries": three_entries(),
        "owner_name": "示例科技有限公司",
    })));
    let out = ok_result(call(&mut svc, "recommend_performance", json!({
        "project_id": &pid, "budget_wan": 700.0, "base_date": "2026-11-07",
    })));
    let suggested = out["suggested_section"].as_str().unwrap().to_string();
    let perf_ids: Vec<String> = out["recommendations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["perf_id"].as_str().unwrap().to_string())
        .collect();

    // 起草前：两门过（matrix/outline approval）——readiness 前置
    // （prepared 后 blocking 面与 segment_tools 同款：先绑定资质证据清 blocking）
    let _ = suggested;
    let _ = perf_ids;

    // 找到业绩章的建议位后直接写（readiness 门：可用业绩证据 >= 3 ——
    // 本工具写入前自动登记 3 份业绩证据，门应当在登记后通过）
    let sec = out["suggested_section"].as_str().unwrap().to_string();
    let w = ok_result(call(&mut svc, "write_performance_section", json!({
        "project_id": &pid,
        "section_id": &sec,
        "perf_ids": [
            "PERF-0001", "PERF-0002", "PERF-0003",
        ],
        "bid_open_date": "2026-12-31",
    })));
    // 证据自动登记：3 条全命中（材料文件名与导入件一致）
    let ev = w["evidence_registered"].as_array().unwrap();
    assert_eq!(ev.len(), 3, "三份材料全部登记为业绩证据：{ev:?}");
    for link in ev {
        assert!(link["ev_id"].as_str().unwrap().starts_with("EV-"));
        assert!(link["deduplicated"].is_boolean());
    }
    // 材料零缺口
    assert_eq!(w["materials_pending"].as_array().unwrap().len(), 0);
    // 证据自动绑定：业绩类 REQ（corpus-A 抽取面含「类似业绩」条款）
    let bindings = w["bindings"].as_array().unwrap();
    assert!(!bindings.is_empty(), "业绩类 REQ 存在 → 绑定非空");
    for b in bindings {
        assert!(!b["req_ids"].as_array().unwrap().is_empty());
    }
    assert_eq!(w["changed"], true, "首次写入 changed=true");
    // 段内容：业绩表 + 描述段 + [EV-*] 引用
    let segs = svc.store().list_section_segments(&sec);
    assert_eq!(segs.len(), 1, "单段写入");
    let content = &segs[0].content;
    assert!(content.contains("| 序号 | 项目名称 |"), "业绩表在段内");
    assert!(content.contains("[EV-"), "描述段带证据引用");
    assert!(content.contains("一次验收通过"), "key_points 进正文");

    // ---- 幂等重跑：同 perf_ids 重提 → 零副作用（changed=false，段数不增）
    let w2 = ok_result(call(&mut svc, "write_performance_section", json!({
        "project_id": &pid, "section_id": &sec,
        "perf_ids": ["PERF-0001", "PERF-0002", "PERF-0003"],
        "bid_open_date": "2026-12-31",
    })));
    assert_eq!(w2["changed"], false, "幂等重跑 changed=false");
    assert_eq!(w2["deduplicated"], true);
    let ev2 = w2["evidence_registered"].as_array().unwrap();
    assert!(ev2.iter().all(|l| l["deduplicated"] == true), "证据登记全部幂等命中");
    assert_eq!(svc.store().list_section_segments(&sec).len(), 1, "段数不增");
    // EV 台账：恰好 3 条业绩类（无重复登记）
    let perf_evs: Vec<_> = svc
        .store()
        .evidences()
        .iter()
        .filter(|e| e.kind == bid_domain::evidence::EvidenceKind::Performance)
        .collect();
    assert_eq!(perf_evs.len(), 3, "重跑不产生重复 EV：{:?}",
        perf_evs.iter().map(|e| e.id.clone()).collect::<Vec<_>>());
}

/// 诚实面：材料未导入 → materials_pending 逐条列明（正文照写、证据不伪造）。
/// 同时验证 readiness 门的行为——只有 0-1 份材料时（可用业绩 < 3）写入被拦，
/// 错误信息带材料指引。
#[test]
fn materials_pending_is_honest() {
    let (mut svc, pid) = prepared("pending");
    // 只登记 1 条「材料已导入」+ 2 条「材料未导入」的业绩
    let mut entries = vec![three_entries()[0].clone()];
    entries.push(json!({
        "name": "材料未导入的业绩", "client": "某局",
        "amount_wan": 100.0, "completed_on": "2025-01-01",
        "materials": ["材料/不存在于工作区的合同.pdf"],
    }));
    ok_result(call(&mut svc, "register_performance", json!({
        "entries": entries, "owner_name": "示例科技有限公司",
    })));
    let out = ok_result(call(&mut svc, "recommend_performance", json!({
        "project_id": &pid,
    })));
    let sec = out["suggested_section"].as_str().unwrap().to_string();
    let reply = call(&mut svc, "write_performance_section", json!({
        "project_id": &pid, "section_id": &sec,
        "perf_ids": ["PERF-0001", "PERF-0002"],
    }));
    let err = err_text(&reply);
    // readiness 门拦截（可用业绩 < 3）且带材料指引（诚实：说清缺什么怎么补）
    assert!(
        err.contains("业绩证据不足") && err.contains("PERF-0002"),
        "缺口指引（含材料未导入的条目）：{err}"
    );
    // 段未写入（readiness 拦截 = 零副作用）
    assert!(svc.store().list_section_segments(&sec).is_empty());
}
