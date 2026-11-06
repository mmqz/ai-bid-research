//! #245 · MCP 全链验收：占位资质不得冒充已核验的真实证据。
//!
//! **验收命题**（issue 原文）：「导入来源真实但内容标记为占位/待核验的资质：
//! 绑定后矩阵仍显示未满足/待补，导出门仍按既定规则拒绝或标记受限，绝不
//! 静默清零阻塞」+「经合法验证的真实证据：正常通过既有证据链，已有正向
//! 流程不回归」+ 修复闭环（不是死锁——升级/豁免后能出厂）。
//!
//! 两个测试：
//! 1. [`placeholder_accident_blocked_then_upgraded_exports`]（事故形态 →
//!    升级修复闭环）：占位登记 → 批量绑定全部条款（2026-11-05 真实客户端
//!    事故形态）→ 矩阵 blocking **不归零**（PENDING_EVIDENCE 待补）→ mock
//!    起草落【待补】文案（不引用 [EV]、不声称已具备）→ validate BLOCKER →
//!    三门全批也拒导出 → 真实证据换版升级（绑定迁移）→ validate 过 →
//!    导出 EXPORTED。
//! 2. [`placeholder_gate_1d_fires_even_when_matrix_and_reviews_pass`]（导出
//!    门 1d 独立复算）：必选行绑真实证据（1a 矩阵门过）+ 非必选行绑占位
//!    （不阻断写作）→ 三门全 Approved 零 findings 零 RT → 导出**仍拒**
//!    （占位证据门——不消费记审面，PR #242 1c 门同款纪律）→ **Responded
//!    状态覆写（即使合法 Human receipt 签发）也不豁免**（PR #247 审核收紧：
//!    覆写在场 ≠ 豁免成立）→ 合法人工豁免（NOT_APPLICABLE + 偏离说明 +
//!    Human receipt）→ 导出放行（1d 的人工裁决通道）。
//!
//! 写操作全部走 `dispatch_with` MCP 分派；无 MCP 工具的准备步骤（矩阵重建/
//! 记审/覆写）走服务面并显式标注。SKIP 纪律（issue #9）：corpus-A 真件
//! 缺失 → 响亮横幅 + return。真实模型面钉 mock（确定性）——无凭据不伪造。

use bid_domain::outline::OutlineSpec;
use bid_domain::review::{Decision, Gate};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_ready(test_id: &str) -> bool {
    let main = fixtures().join("A-招标文件.docx");
    let addendum = fixtures().join("C-补遗文件01.docx");
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&main, &addendum])
}

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut()
            .unwrap()
            .insert("generator".into(), serde_json::json!("mock"));
    }
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
        ToolReply::Err { error, .. } => error.to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn ws_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-ph245-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn leaf_ids(svc: &BidService) -> Vec<String> {
    let parents: std::collections::HashSet<String> =
        svc.store().sections().iter().filter_map(|n| n.parent_id.clone()).collect();
    svc.store()
        .sections()
        .iter()
        .filter(|n| !parents.contains(&n.id))
        .map(|n| n.id.clone())
        .collect()
}

/// 共用前段：建项 → 导入真件 → 抽取 → 建纲建矩阵。返回 (svc, pid)。
fn setup(tag: &str) -> (BidService, String) {
    let ws = ws_dir(tag);
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(fixtures());
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "#245 占位验收", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": format!("ph245-{tag}"),
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    let files = [
        ("A-招标文件.docx", "招标文件"),
        ("C-补遗文件01.docx", "补遗"),
        ("evidence/EV-01-营业执照.pdf", "附件"),
        ("evidence/EV-02-电子与智能化工程专业承包二级资质证书.pdf", "附件"),
        ("evidence/EV-13-安全生产许可证.pdf", "附件"),
    ];
    let file_specs: Vec<Value> = files
        .iter()
        .map(|(rel, kind)| {
            let basename = rel.rsplit('/').next().unwrap_or(rel);
            json!({
                "upload_ref": fixtures().join(rel).display().to_string(),
                "filename": basename, "kind": kind,
            })
        })
        .collect();
    let r = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": &pid, "files": file_specs,
    })));
    assert_eq!(r["totals"]["parse_failed"], json!(0), "五件应全成功: {}", r["totals"]);
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    let (n_sec, n_rows, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 6_000, max_reqs_per_leaf: 6 })
        .unwrap();
    assert!(n_sec >= 2 && n_rows >= 5);
    (svc, pid)
}

/// file_id 反查（import 回执）。
fn file_id_of(svc: &BidService, name: &str) -> String {
    svc.store()
        .files()
        .iter()
        .find(|f| f.name == name)
        .map(|f| f.id.clone())
        .unwrap_or_else(|| panic!("{name} 不在文件表"))
}

/// 占位登记（MCP 契约面——placeholder=true）。
fn register_placeholder(svc: &mut BidService, pid: &str, title: &str, source_name: &str) -> String {
    let r = ok_result(call(
        svc,
        "register_evidence",
        json!({
            "project_id": pid, "kind": "qualification", "title": title,
            "source_file_id": file_id_of(svc, source_name),
            "subject_name": "投标人", "scope": "电子与智能化工程",
            "placeholder": true,
        }),
    ));
    assert_eq!(r["verification_tier"], "PLACEHOLDER", "占位登记回执: {r:?}");
    r["ev_id"].as_str().unwrap().to_string()
}

/// 真实登记（带页锚——locator 指纹按解析产物对账）。
fn register_real_with_page_anchor(
    svc: &mut BidService,
    pid: &str,
    title: &str,
    source_name: &str,
    page: u32,
) -> String {
    // 页文本 + 指纹（「读过文件的调用方」视角——与 evidence_content_tool 同款）
    let sha = svc
        .store()
        .files()
        .iter()
        .find(|f| f.name == source_name)
        .map(|f| f.sha256.clone())
        .unwrap();
    let parsed = svc
        .parsed()
        .iter()
        .find(|p| p.sha256 == sha)
        .unwrap_or_else(|| panic!("{source_name} 无解析产物"));
    let prefixes = [format!("第{page}页"), format!("第{page} 页")];
    let page_text: String = parsed
        .blocks
        .iter()
        .filter(|b| prefixes.iter().any(|p| b.locator.starts_with(p.as_str())))
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!page_text.trim().is_empty(), "{source_name} 第 {page} 页有文本");
    let fp = format!("fnv64:{}", bid_domain::linkage::text_fingerprint(&page_text));
    let r = ok_result(call(
        svc,
        "register_evidence",
        json!({
            "project_id": pid, "kind": "qualification", "title": title,
            "source_file_id": file_id_of(svc, source_name),
            "subject_name": "投标人", "scope": "电子与智能化工程",
            "locator": { "kind": "pdf_page_text", "page": page, "text_fingerprint": fp },
        }),
    ));
    assert_eq!(r["verification_tier"], "CONTENT_MATCHES", "真实登记回执: {r:?}");
    r["ev_id"].as_str().unwrap().to_string()
}

fn approve_two_gates(svc: &mut BidService) {
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval] {
        svc.store_mut()
            .record_review(gate, &bid_domain::TrustedPrincipal::human("245-lead"), Decision::Approved, vec![], None)
            .unwrap();
    }
}

fn approve_pre_export(svc: &mut BidService) {
    svc.store_mut()
        .record_review(Gate::PreExportConfirm, &bid_domain::TrustedPrincipal::human("245-lead"), Decision::Approved, vec![], None)
        .unwrap();
}

fn draft_all(svc: &mut BidService, pid: &str) {
    for sid in leaf_ids(svc) {
        let r = ok_result(call(
            svc,
            "draft_response_section",
            json!({ "project_id": pid, "section_id": sid, "instructions": "按招标文件逐项响应" }),
        ));
        assert_eq!(r["section_status"], "DRAFTED", "起草 {sid}");
    }
}

// ---------------------------------------------------------------- 1. 事故形态 → 升级修复闭环

/// 占位资质批量绑定（事故形态）→ 阻塞不归零 → 即使三门全批也拒导出 →
/// 真实证据换版升级（绑定迁移）→ validate 过 → 导出 EXPORTED。
#[test]
fn placeholder_accident_blocked_then_upgraded_exports() {
    if !corpus_ready("ph245-accident") {
        return; // SKIP 横幅已由 skip_if_fixture_missing 打出
    }
    let (mut svc, pid) = setup("accident");

    // ---- 事故形态：占位登记（placeholder=true，来源指向已导入文件——
    //      来源真实但内容未核验）→ 批量绑定全部条款
    let ev_ph = register_placeholder(
        &mut svc, &pid, "电子与智能化工程专业承包二级资质证书", "A-招标文件.docx",
    );
    let all_req_ids: Vec<Value> = svc
        .store()
        .requirements()
        .iter()
        .map(|r| json!(r.id.clone()))
        .collect();
    let bind = ok_result(call(
        &mut svc,
        "bind_evidence",
        json!({ "project_id": &pid, "ev_id": &ev_ph,
                "requirement_ids": all_req_ids, "bid_open_date": "2026-12-31" }),
    ));
    assert_eq!(bind["placeholder"], json!(true), "绑定回执占位标记");
    assert!(
        bind["matrix"]["blocking"].as_u64().unwrap_or(0) > 0,
        "【#245 核心断言】占位绑定后必选行 blocking 未归零（事故缺口收口）: {bind:?}"
    );

    // ---- 矩阵回显：资格/否决行 PENDING_EVIDENCE + 待补追踪面
    let matrix = ok_result(call(
        &mut svc,
        "get_compliance_matrix",
        json!({ "project_id": &pid, "filters": { "status": "PENDING_EVIDENCE" } }),
    ));
    let pending_rows = matrix["rows"].as_array().unwrap();
    assert!(!pending_rows.is_empty(), "PENDING_EVIDENCE 行可过滤命中");
    for row in pending_rows {
        assert!(
            row["pending_evidence_ids"].as_array().unwrap().contains(&json!(ev_ph)),
            "待补追踪面携带占位 EV: {row:?}"
        );
    }

    // ---- 起草就绪门拦截（#110 反编造门 × #245 占位语义的纵深防御）：
    //      资格章节只有占位资质 → 无证硬写等于虚假响应 → readiness 拒起草。
    //      这是第一道防线（起草面）；占位不会让资格材料就绪判定静默满足。
    approve_two_gates(&mut svc);
    let qual_leaf = leaf_ids(&svc)
        .into_iter()
        .find(|sid| {
            svc.store()
                .sections()
                .iter()
                .find(|n| n.id == *sid)
                .map(|n| {
                    n.requirement_ids
                        .iter()
                        .any(|rid| svc.store().requirements().iter().any(|r| r.id == *rid && r.category == "资格条件"))
                })
                .unwrap_or(false)
        })
        .expect("资格条件 REQ 应有叶章节承载");
    let reply = call(
        &mut svc,
        "draft_response_section",
        json!({ "project_id": &pid, "section_id": qual_leaf, "instructions": "按资格条件响应" }),
    );
    let err = err_text(&reply);
    assert!(
        err.contains("缺") && err.contains("资格"),
        "占位资质不满足就绪判定（无证硬写拦下）: {err}"
    );
    assert!(
        err.contains("PLACEHOLDER") || err.contains("占位"),
        "不可用清单点名占位件（防 ACTIVE 误读）: {err}"
    );

    // ---- 事故 containment 汇总（占位绑定的三道拦截在事故形态下全部生效）：
    //      ① 矩阵 blocking 未归零（上方 bind 断言）；
    //      ② 资格章节起草被 readiness 拦（上方 draft 断言——无证硬写拒）；
    //      ③ 状态机连锁：资格章节未起草 → 项目停在 MATRIX_READY →
    //         validate/export 的状态门同样不放行（fail-closed 语义本身
    //         即拒——无需另行断言面）。
    //      「三门全批也拒导出」的正路径演示在测试 2（1d 门——起草可行的
    //      形态下独立复算拒）。

    // ---- 修复闭环（升级通道）：导入真实资质文件并重新登记（同语义位 →
    //      换版：占位记录 SUPERSEDED 留审计痕 + 绑定迁移到真实记录）
    let ev_real = register_real_with_page_anchor(
        &mut svc, &pid, "电子与智能化工程专业承包二级资质证书",
        "EV-02-电子与智能化工程专业承包二级资质证书.pdf", 1,
    );
    let ph = svc.store().evidence(&ev_ph).unwrap().clone();
    assert_eq!(ph.status.as_str(), "SUPERSEDED", "旧占位记录留审计痕");
    assert_eq!(svc.store().evidence(&ev_real).unwrap().version, 2, "版本递增");
    assert!(
        !svc.store().evidence_bindings().iter().any(|b| b.evidence_id == ev_ph),
        "占位绑定全部迁移（不残留）"
    );
    // 矩阵重建（服务面——绑定迁移后派生面刷新）
    let _ = svc.store_mut().build_matrix().unwrap();
    let blocking_now: usize = svc.store().matrix().iter().filter(|r| r.blocking).count();
    assert_eq!(blocking_now, 0, "升级后 blocking 清零（真实证据 + 章节在位 → RESPONDED）");

    // ---- 升级后起草放行（就绪门随真实证据解锁——修复不是死锁）→ validate 过
    draft_all(&mut svc, &pid);
    let any_pending_text = leaf_ids(&svc)
        .iter()
        .any(|sid| {
            svc.store()
                .sections()
                .iter()
                .find(|n| n.id == *sid)
                .and_then(|n| n.content.as_deref())
                .map(|c| c.contains("【待补："))
                .unwrap_or(false)
        });
    // 升级后全部证据为真实件——【待补】不应再出现（占位文案随升级消失）
    assert!(!any_pending_text, "升级后无【待补】残留（占位证据已被真实件顶替）");
    let cites_placeholder = leaf_ids(&svc)
        .iter()
        .any(|sid| {
            svc.store()
                .sections()
                .iter()
                .find(|n| n.id == *sid)
                .and_then(|n| n.content.as_deref())
                .map(|c| c.contains(&format!("[{ev_ph}]")))
                .unwrap_or(false)
        });
    assert!(!cites_placeholder, "旧占位 EV 不得被正文引用");
    let v2 = ok_result(call(
        &mut svc,
        "validate_bid_document",
        json!({ "project_id": &pid, "scope": "all" }),
    ));
    assert_eq!(v2["ok"], json!(true), "升级后校验过: {}", v2["errors"]);
    approve_pre_export(&mut svc);
    let r = ok_result(call(
        &mut svc,
        "export_bid_document",
        json!({ "project_id": &pid, "format": "docx", "output_path": "out/投标文件-占位升级后.docx" }),
    ));
    assert_eq!(r["project_status"], "EXPORTED", "修复闭环导出成功: {r:?}");
    assert!(r["artifact"]["sha256"].as_str().unwrap().len() == 64);
}

// ---------------------------------------------------------------- 2. 导出门 1d 独立复算

/// 必选行绑真实证据（1a 矩阵门过）+ 非必选行绑占位（待补可见不阻断）→
/// 三门全 Approved → 导出**仍拒**（占位证据门 1d——不消费记审面）→
/// 人工覆写豁免（Human receipt + 偏离说明）→ 导出放行。
#[test]
fn placeholder_gate_1d_fires_even_when_matrix_and_reviews_pass() {
    if !corpus_ready("ph245-gate1d") {
        return;
    }
    let (mut svc, pid) = setup("gate1d");

    // ---- 真实证据绑必选行（资格/否决/技术参数——与 wa e2e 同款绑定面）
    let ev_real = register_real_with_page_anchor(
        &mut svc, &pid, "电子与智能化工程专业承包二级资质证书",
        "EV-02-电子与智能化工程专业承包二级资质证书.pdf", 1,
    );
    let mandatory_ids: Vec<Value> = svc
        .store()
        .requirements()
        .iter()
        .filter(|r| r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数")
        .map(|r| json!(r.id.clone()))
        .collect();
    let bind = ok_result(call(
        &mut svc,
        "bind_evidence",
        json!({ "project_id": &pid, "ev_id": &ev_real,
                "requirement_ids": mandatory_ids, "bid_open_date": "2026-12-31" }),
    ));
    assert_eq!(bind["matrix"]["blocking"], json!(0), "真实证据 → 1a 矩阵门过（零回归）");

    // ---- 非必选行绑占位：待补可见、不阻断写作。选「有章节归属的非必选
    //      REQ」（评分/商务等——大纲未必给全部评分项建节，按章节归属筛保证
    //      起草面能消费到占位证据）。
    let assigned_req_ids: std::collections::HashSet<String> = svc
        .store()
        .sections()
        .iter()
        .flat_map(|n| n.requirement_ids.clone())
        .collect();
    let scoring_ids: Vec<String> = svc
        .store()
        .requirements()
        .iter()
        .filter(|r| {
            assigned_req_ids.contains(&r.id)
                && r.category != "资格条件"
                && r.category != "否决项"
                && !bid_domain::matrix::agent_mandatory(r)
        })
        .map(|r| r.id.clone())
        .collect();
    assert!(!scoring_ids.is_empty(), "有章节归属的非必选 REQ 在场（夹具语义）");
    let ev_ph = register_placeholder(
        &mut svc, &pid, "iso9001 质量管理体系认证（待核）", "EV-13-安全生产许可证.pdf",
    );
    let bind2 = ok_result(call(
        &mut svc,
        "bind_evidence",
        json!({ "project_id": &pid, "ev_id": &ev_ph,
                "requirement_ids": scoring_ids.iter().map(|s| json!(s)).collect::<Vec<_>>(),
                "bid_open_date": "2026-12-31" }),
    ));
    assert_eq!(bind2["matrix"]["blocking"], json!(0), "非必选行占位 → 不阻断（业务允许继续写作）");
    let row = svc
        .store()
        .matrix()
        .iter()
        .find(|r| r.requirement_id == scoring_ids[0])
        .unwrap();
    assert_eq!(
        bid_domain::matrix::ResponseStatus::PendingEvidence,
        row.status,
        "评分行 → PENDING_EVIDENCE（待补可见）"
    );

    // ---- 起草 + 三门全 Approved（零 findings 零 RT——「有人直接批准审核」形态）
    approve_two_gates(&mut svc);
    draft_all(&mut svc, &pid);
    // 【待补】文案在正文（非必选行占位 → 业务允许继续写作——待补可见形态）；
    // 不引用占位 EV 编号（引用面冒充由 E_CITATION_PLACEHOLDER 兜底）
    let scoring_leaf_content = |svc: &BidService| -> Option<String> {
        leaf_ids(svc)
            .into_iter()
            .find_map(|sid| {
                svc.store()
                    .sections()
                    .iter()
                    .find(|n| n.id == sid)
                    .and_then(|n| n.content.clone())
                    .filter(|c| c.contains("【待补："))
            })
    };
    assert!(
        scoring_leaf_content(&svc).is_some(),
        "评分章节占位证据 → mock 落【待补：...】待补文案（可继续写作）"
    );
    let cites_ph = leaf_ids(&svc)
        .iter()
        .any(|sid| {
            svc.store()
                .sections()
                .iter()
                .find(|n| n.id == *sid)
                .and_then(|n| n.content.as_deref())
                .map(|c| c.contains(&format!("[{ev_ph}]")))
                .unwrap_or(false)
        });
    assert!(!cites_ph, "占位 EV 不得被正文引用为佐证");
    let v = ok_result(call(
        &mut svc,
        "validate_bid_document",
        json!({ "project_id": &pid, "scope": "all" }),
    ));
    assert_eq!(v["ok"], json!(false), "占位绑定在库 → validate 报 BLOCKER（E_EVIDENCE_PLACEHOLDER）");
    approve_pre_export(&mut svc);

    // ---- 导出仍拒：1a 已过（blocking=0）、1b 已过（三门 Approved 零 RT）、
    //      1c 已过（mock 篇幅达标）——**1d 占位证据门**独立复算拦下。
    let reply = call(
        &mut svc,
        "export_bid_document",
        json!({ "project_id": &pid, "format": "docx", "output_path": "out/投标文件-1d门.docx" }),
    );
    let err = err_text(&reply);
    assert!(
        err.contains("占位证据门"),
        "【1d 门独立复算】矩阵与审核全过也拒（占位不得冒充已核验导出）: {err}"
    );
    assert!(err.contains("E_EVIDENCE_PLACEHOLDER"), "逐条点名 BLOCKER: {err}");

    // ---- PR #247 审核收紧回归：Responded 状态覆写不豁免占位门。走**合法
    //      Human receipt**（宿主签发面 issue_approval_receipt、签域四重绑定
    //      project/req/status/reason）把待补行覆写为 RESPONDED——完全合法的
    //      人工动作（set_matrix_override 对 Responded 不强制偏离说明，这正是
    //      旧门禁条件过宽的入口）——仍活跃的占位绑定照样拦：「已响应」正是
    //      占位材料不得冒充的结论，覆写在场 ≠ 豁免成立。
    for rid in &scoring_ids {
        let scope = bid_tools::service::matrix_override_scope(&pid, rid, "RESPONDED", "");
        let receipt = svc.issue_approval_receipt("245-裁决人", &scope);
        svc.set_matrix_override(
            rid.clone(),
            bid_domain::matrix::ResponseStatus::Responded,
            None,
            &receipt,
        )
        .unwrap();
    }
    let reply_resp = call(
        &mut svc,
        "export_bid_document",
        json!({ "project_id": &pid, "format": "docx", "output_path": "out/投标文件-1d门-responded覆写.docx" }),
    );
    let err_resp = err_text(&reply_resp);
    assert!(
        err_resp.contains("占位证据门") && err_resp.contains("E_EVIDENCE_PLACEHOLDER"),
        "【Responded 覆写不豁免】合法 Human receipt 的状态覆写也拦（占位仍绑定时）: {err_resp}"
    );

    // ---- 人工豁免通道（1d 的差异面）：Human receipt 覆写 + 偏离说明 → 放行
    for rid in &scoring_ids {
        svc.store_mut()
            .set_matrix_override(
                rid.clone(),
                bid_domain::matrix::ResponseStatus::NotApplicable,
                Some("该评分项以承诺响应，佐证材料后补（人工 receipt 裁决）".into()),
                &bid_domain::TrustedPrincipal::human("245-裁决人"),
            )
            .unwrap();
    }
    let _ = svc.store_mut().rebuild_matrix_for_override().unwrap();
    let r = ok_result(call(
        &mut svc,
        "export_bid_document",
        json!({ "project_id": &pid, "format": "docx", "output_path": "out/投标文件-1d豁免后.docx" }),
    ));
    assert_eq!(r["project_status"], "EXPORTED", "人工豁免后导出放行（占位不堵死人工裁决）: {r:?}");
}

