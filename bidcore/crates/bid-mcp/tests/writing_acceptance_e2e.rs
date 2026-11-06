//! 1.0 写作验收 · MCP 全流程可重复验收 + 篇幅门回归。
//!
//! **验收命题**（任务指令原文）：「不足时不能把整份投标文件报告为达标；
//! 同时确保每个必写叶章节都有正文」+「一条可重复的 MCP 全流程验收：
//! 导入 → 抽取 → 建纲 → 起草 → 验证 → DOCX 导出，报告目标字数、实际
//! 字数、章节完成数和导出校验结果」。
//!
//! 两个测试：
//! 1. [`writing_acceptance_full_flow_docx_out`]（篇幅达标可导出）：corpus-A
//!    真件全链，mock 生成器（pad_to_target 补到下限——CI 确定性），
//!    断言 validate ok=true（零 `E_CHARS_BELOW_MIN`——最低篇幅达标；超带
//!    只允许 `W_CHARS_OFF_TARGET` MINOR 提示）+ 每个必写叶有正文 + 导出
//!    EXPORTED + 产物四检（sha256 / TOC 域 / 章数 / 体积），并汇总验收
//!    报告（目标字数/实际字数/章节完成数/导出校验）。
//! 2. [`writing_acceptance_short_draft_blocked_from_export`]（篇幅不足拒绝
//!    达标/导出）：mock 起草后把一叶改短稿 → validate ok=false +
//!    `E_CHARS_BELOW_MIN` BLOCKER（整份不得报告达标）→ PreExportConfirm
//!    Rejected 派生 RT → 导出双拒面（未过门 E_STATE；即使门已过、RT 未
//!    关单仍 E_UNPROCESSABLE）→ 修复闭环（补写回带内 → 关 RT → 重审 →
//!    导出成功 EXPORTED）。
//! 3. [`writing_acceptance_export_independent_of_review_approval`]（导出
//!    门独立复算——Lead 问询补明 2026-11）：篇幅不足时即使有人直接批准
//!    审核（三门全 Approved、零 findings、零 RT）仍禁止导出——前置门 1c
//!    不消费记审面；含空叶变体与修复闭环（修好重审就能出厂，不是死锁）。
//!
//! 写操作全部走 `dispatch_with` MCP 分派（不绕开契约面）；无 MCP 工具的
//! 准备步骤（证据绑定/记审/RT 关单）走服务面并显式标注——e2e_pipeline
//! 同款纪律。
//!
//! SKIP 纪律（issue #9）：corpus-A 真件缺失 → 响亮横幅 + return。
//!
//! 真实模型面：本套件钉 mock（确定性）。真实 LLM 全链走 real_llm_path.rs
//! 的 BID_REAL_PATH_WS 两相位（人肉 LLM 中继）——无凭据时不伪造通过。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files");
    // #183 围栏：词法 `..` 会被围栏预拒——受信夹具根统一 canonical 绝对路径
    std::fs::canonicalize(&p).unwrap_or(p)
}

/// 真件在位预检（issue #9 SKIP 机制——缺件响亮横幅，调用方立即 return）。
fn corpus_ready(test_id: &str) -> bool {
    let main = fixtures().join("A-招标文件.docx");
    let addendum = fixtures().join("C-补遗文件01.docx");
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&main, &addendum])
}

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    // 确定性套件钉 mock（e2e_pipeline 同款——防 dev 机配了 BID_LLM_API_KEY
    // 后 auto 走真实调用）。
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

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
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
        "bid-writing-accept-{tag}-{}-{}",
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

/// ①-⑤ 共用前段：建项目 → 导入真件 → 抽取 → 建纲建矩阵 → 证据绑定 +
/// 前两门过审（MatrixApproval / OutlineApproval）。返回 (svc, pid, ws)。
fn acceptance_setup(tag: &str) -> (BidService, String, PathBuf) {
    let ws = ws_dir(tag);
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(fixtures());

    // ① 建项目（MCP §1）
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "1.0 写作验收", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": format!("wa-{tag}"),
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();

    // ② 导入真件（MCP §2——A 主文件 + C 补遗 + EV 证据两件）
    let files = [
        ("A-招标文件.docx", "招标文件"),
        ("C-补遗文件01.docx", "补遗"),
        ("evidence/EV-01-营业执照.pdf", "附件"),
        ("evidence/EV-02-电子与智能化工程专业承包二级资质证书.pdf", "附件"),
    ];
    let file_specs: Vec<Value> = files
        .iter()
        .map(|(name, kind)| json!({
            "upload_ref": fixtures().join(name).display().to_string(),
            "filename": name, "kind": kind,
        }))
        .collect();
    let r = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": &pid, "files": file_specs,
    })));
    assert!(
        r["totals"]["files"].as_u64().unwrap_or(0) >= 4,
        "② 导入面: {}",
        r["totals"]
    );

    // ③ 抽取（MCP §3）
    let r = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    let n_reqs = r["counts"]["total"].as_u64().unwrap_or(0);
    assert!(n_reqs >= 5, "③ 抽取面: {n_reqs}");

    // ④ 建纲建矩阵（服务面——无 MCP 工具的步骤，显式标注）
    let (n_sec, n_rows, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 6_000, max_reqs_per_leaf: 6 })
        .unwrap();
    assert!(n_sec >= 2, "④ 章节树: {n_sec}");
    assert!(n_rows >= 5, "④ 矩阵: {n_rows}");

    // ⑤ 证据登记 + 绑定 + 前两门（服务面）
    use bid_domain::evidence::EvidenceSpec;
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "电子与智能化工程专业承包二级资质证书".into(),
        source_file: "EV-02-电子与智能化工程专业承包二级资质证书.pdf".into(),
        source_sha256: "sha-wa-ev02".into(),
        locator: EngineLocator::DocxParagraph {
            paragraph_index: 0,
            text_fingerprint: "fnv64:wa".into(),
        },
        fingerprint: "fnv64:wa".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "投标人".into(),
        scope: "电子与智能化工程".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    let bind_ids: Vec<String> = svc
        .store()
        .requirements()
        .iter()
        .filter(|r| {
            r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数"
        })
        .map(|r| r.id.clone())
        .collect();
    assert!(!bind_ids.is_empty(), "⑤ 可绑定 REQ 非空");
    for rid in &bind_ids {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval] {
        svc.store_mut()
            .record_review(gate, &bid_domain::TrustedPrincipal::human("wa-lead"), Decision::Approved, vec![], None)
            .unwrap();
    }
    (svc, pid, ws)
}

// ---------------------------------------------------------------- 1. 篇幅达标可导出

/// 全流程验收（mock 确定性面）：导入 → 抽取 → 建纲 → 起草 → 验证 →
/// DOCX 导出。汇总报告：目标字数 / 实际字数 / 章节完成数 / 导出校验。
#[test]
fn writing_acceptance_full_flow_docx_out() {
    if !corpus_ready("wa-full") {
        return; // SKIP 横幅已由 skip_if_fixture_missing 打出
    }
    let (mut svc, pid, ws) = acceptance_setup("full");

    // ⑥ 逐章起草（MCP §6——mock 确定性生成，pad_to_target 保下限；
    //    quota 面逐章回传 #151）
    let leaves = leaf_ids(&svc);
    assert!(!leaves.is_empty());
    let mut quota_target = 0u64;
    let mut quota_written = 0u64;
    for sid in &leaves {
        let r = ok_result(call(&mut svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": sid,
            "instructions": "按招标文件逐项响应，突出同类业绩与实施方案",
        })));
        assert_eq!(r["section_status"], "DRAFTED", "⑥ {sid}");
        quota_target += r["quota"]["target_chars"].as_u64().unwrap_or(0);
        quota_written += r["quota"]["written_chars"].as_u64().unwrap_or(0);
    }

    // 每个必写叶章节都有正文（1.0 验收硬线——空叶由 E_SECTION_EMPTY 兜底，
    // 此处显式断言正面：全部叶 content 非空 + 状态达 DRAFTED）
    let mut drafted_count = 0u64;
    for sid in &leaves {
        let node = svc.store().sections().iter().find(|n| n.id == *sid).unwrap();
        let has_body = node
            .content
            .as_deref()
            .map(|c| !c.trim().is_empty())
            .unwrap_or(false);
        assert!(has_body, "⑥ 必写叶 {sid}「{}」无正文", node.title);
        assert_eq!(node.status.as_str(), "DRAFTED", "⑥ {sid} 状态");
        drafted_count += 1;
    }
    assert_eq!(drafted_count, leaves.len() as u64, "⑥ 全部叶完成");

    // ⑦ 校验（MCP §7）：ok=true + 最低篇幅零发现（E_CHARS_BELOW_MIN 是
    // BLOCKER——不足则 ok 翻 false；超带 W_CHARS_OFF_TARGET 是 MINOR
    // 提示，分治语义在下方单独锁定）
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": &pid, "scope": "all",
    })));
    assert_eq!(r["ok"], json!(true), "⑦ 篇幅达标面校验必须过: {}", r["errors"]);
    assert_eq!(r["counts"]["blocker"], json!(0), "⑦ BLOCKER 清零");
    let codes: Vec<&str> = r["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule_code"].as_str().unwrap())
        .collect();
    // 最低篇幅达标（1.0 验收核心线）：零 E_CHARS_BELOW_MIN——每个必写叶
    // 的正文都在配额 −20% 容差下限之上。
    assert!(!codes.contains(&"E_CHARS_BELOW_MIN"), "⑦ 篇幅不足发现: {codes:?}");
    // 上限分治语义回归锁：mock 生成器基础内容天然可超带（pad_to_target
    // 只补下限——既有行为），超带必须是 **MINOR 提示**（进 warnings、不进
    // errors、不翻 ok）——若升格为 BLOCKER 会在本断言前先炸 ok=true。
    let over_band = codes.iter().filter(|c| **c == "W_CHARS_OFF_TARGET").count();
    assert!(
        r["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["rule_code"] == "W_CHARS_OFF_TARGET")
            .all(|f| f["severity"] == "MINOR"),
        "⑦ 超带只能是 MINOR 提示: {}",
        r["findings"]
    );
    assert_eq!(r["project_status"], "VALIDATING");

    // ⑦' PreExportConfirm 记审（#180：定稿内容过门——服务面）
    use bid_domain::review::{Decision, Gate};
    svc.store_mut()
        .record_review(Gate::PreExportConfirm, &bid_domain::TrustedPrincipal::human("wa-lead"), Decision::Approved, vec![], None)
        .unwrap();

    // ⑧ 导出（MCP §8——成品 docx）
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-写作验收.docx",
    })));
    let url = r["artifact"]["url"].as_str().unwrap().to_string();
    assert!(url.ends_with("投标文件-写作验收.docx"), "⑧ 产物名: {url}");
    let sha256 = r["artifact"]["sha256"].as_str().unwrap().to_string();
    assert_eq!(sha256.len(), 64, "⑧ sha256 十六进制 64 位");
    let size = r["artifact"]["size"].as_u64().unwrap_or(0);
    assert!(size > 5_000, "⑧ 产物 >5KB: {size}");
    assert_eq!(r["chapters_written"].as_u64().unwrap(), leaves.len() as u64, "⑧ 全部落章");
    assert_eq!(r["project_status"], "EXPORTED");

    // 产物深检（解包——TOC 域 + 首叶标题进正文）
    let bytes = std::fs::read(&url).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("⑧ docx 合法 zip");
    let mut doc_xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut doc_xml).unwrap();
    assert!(doc_xml.contains("TOC \\o"), "⑧ TOC 域在位");
    let first_title = svc.store().sections().iter().find(|n| n.id == leaves[0]).unwrap().title.clone();
    assert!(doc_xml.contains(&first_title), "⑧ 章标题「{first_title}」进正文");

    // ---- 验收报告（可重复执行的面板：目标/实际/完成数/导出校验）
    //      quota 口径 = #151 draft 返回的逐章配额累加（target/written）。
    let total_target: u64 = svc
        .store()
        .sections()
        .iter()
        .filter(|n| leaves.contains(&n.id))
        .map(|n| u64::from(n.target_chars))
        .sum();
    let total_written: u64 = svc
        .store()
        .sections()
        .iter()
        .filter(|n| leaves.contains(&n.id))
        .map(|n| n.content.as_deref().map(|c| c.chars().count() as u64).unwrap_or(0))
        .sum();
    assert_eq!(quota_target, total_target, "报告面：quota 累加与纲配额一致");
    assert_eq!(quota_written, total_written, "报告面：quota 已写与正文实测一致");
    let in_band = total_written >= total_target * 80 / 100;
    assert!(in_band, "报告面：实际字数 {total_written} ≥ 目标 {total_target} 的 80%");
    println!(
        "\n[1.0 写作验收] 目标字数={total_target} 实际字数={total_written}（{:.1}%）章节完成={drafted_count}/{} 超带提示叶数={over_band} 导出=EXPORTED sha256={}.. size={}B TOC=在位\n",
        if total_target > 0 { total_written as f64 * 100.0 / total_target as f64 } else { 0.0 },
        leaves.len(),
        &sha256[..12],
        size,
    );
    let _ = &ws;
}

// ---------------------------------------------------------------- 2. 篇幅不足拒绝达标/导出

/// 短稿闭环：validate 拒达标（ok=false；#253 门禁分级后 BLOCKER 面 =
/// E_TOTAL_CHARS_BELOW_MIN 全稿总量硬门 + 短稿叶 W_CHARS_BELOW_MIN MAJOR）→
/// PreExportConfirm Rejected 派生 RT → 导出双拒（未过门 E_STATE；门过了
/// RT 未关单仍 E_UNPROCESSABLE）→ 补写回带内 → 关 RT → 重审 → 导出成功。
///
/// #253 语义更新：短稿叶（corpus-A 无评分章节）的章节级码从
/// E_CHARS_BELOW_MIN BLOCKER 改为 W_CHARS_BELOW_MIN MAJOR（不逐章阻断）；
/// 「篇幅不足不得达标/导出」的硬线由 E_TOTAL_CHARS_BELOW_MIN（全稿总量
/// < 配额总和，1000 页硬指标）承担——原验收命题不变，把关面上收。
#[test]
fn writing_acceptance_short_draft_blocked_from_export() {
    if !corpus_ready("wa-short") {
        return;
    }
    let (mut svc, pid, ws) = acceptance_setup("short");

    // ⑥ mock 全叶起草（pad_to_target 保下限）
    let leaves = leaf_ids(&svc);
    for sid in &leaves {
        let r = ok_result(call(&mut svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": sid,
            "instructions": "按招标文件逐项响应",
        })));
        assert_eq!(r["section_status"], "DRAFTED");
    }

    // ⑥' 全叶改短稿（测试特权——服务面直调自定义生成器，模拟 LLM 偷工）。
    //    #253 后总门语义：小目标纲（本夹具 6,000）的叶配额 ~500 级，单叶
    //    缺口可被其余叶的带内超写互偿（总量不必然塌）；全叶短稿保证
    //    E_TOTAL_CHARS_BELOW_MIN 确定性触发——RT 闭环走总门 BLOCKER 面
    struct Tiny;
    impl bid_tools::draft::SectionGenerator for Tiny {
        fn generate(&mut self, i: &bid_tools::draft::SectionDraftInput)
            -> Result<bid_tools::draft::GeneratedDraft, String> {
            Ok(bid_tools::draft::GeneratedDraft {
                content: format!(
                    "{}短稿。",
                    i.requirements.iter().map(|r| format!("[{}]", r.id)).collect::<Vec<_>>().join("")
                ),
            })
        }
    }
    for sid in &leaves {
        svc.draft_section(sid, None, None, Some("超短稿"), &mut Tiny).unwrap();
    }
    let victim = leaves[0].clone();
    let victim_target: u64 = u64::from(
        svc.store().sections().iter().find(|n| n.id == victim).unwrap().target_chars,
    );
    assert!(victim_target > 0, "被测叶须有配额");

    // ⑦ validate：整份标书不得报告达标（ok=false）。#253 门禁分级：
    //    短稿叶（非评分）→ W_CHARS_BELOW_MIN MAJOR；全稿总量不足 →
    //    E_TOTAL_CHARS_BELOW_MIN BLOCKER（1000 页硬指标硬比较）
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": &pid, "scope": "all",
    })));
    assert_eq!(r["ok"], json!(false), "篇幅不足时整份标书不得报告达标: {}", r["errors"]);
    let errors = r["errors"].as_array().unwrap();
    let total_gate = errors
        .iter()
        .find(|e| e["rule_code"] == "E_TOTAL_CHARS_BELOW_MIN")
        .expect("E_TOTAL_CHARS_BELOW_MIN 必须在 errors 面（全稿总量硬门）");
    assert_eq!(total_gate["severity"], "BLOCKER");
    let low = errors
        .iter()
        .find(|e| e["rule_code"] == "W_CHARS_BELOW_MIN")
        .expect("W_CHARS_BELOW_MIN 必须在 errors 面（短稿叶 MAJOR）");
    assert_eq!(low["severity"], "MAJOR");
    assert_eq!(low["section_id"].as_str().unwrap(), victim, "锚到短稿章节");

    // ⑦' PreExportConfirm Rejected（携带 BLOCKER finding——服务面）→ RT 派生
    use bid_domain::review::{Decision, Gate, Severity};
    let receipt = svc
        .store_mut()
        .record_review(
            Gate::PreExportConfirm,
            &bid_domain::TrustedPrincipal::human("wa-lead"),
            Decision::Rejected,
            vec![(
                Severity::Blocker,
                "篇幅".into(),
                format!("章节 {victim} 正文低于最低篇幅（target={victim_target} 的 −20% 下限）"),
                Some(victim.clone()),
            )],
         None)
        .unwrap();
    assert_eq!(receipt.spawned_rt_ids.len(), 1, "BLOCKER 强制派生 1 个 RT");
    let rt_id = receipt.spawned_rt_ids[0].clone();

    // ⑧ 导出被拒（面 1：PreExportConfirm 未 Approved → E_STATE）
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-短稿.docx",
    }));
    assert_eq!(err_code(&reply), "E_STATE", "未过导出确认门: {reply:?}");

    // ⑧' 导出被拒（面 2：即使门被误批，RT 未关单仍拒——E_UNPROCESSABLE）
    //      测试特权：补记一条同门 Approved（内容未变），单独验证 RT 闸面。
    svc.store_mut()
        .record_review(Gate::PreExportConfirm, &bid_domain::TrustedPrincipal::human("wa-lead"), Decision::Approved, vec![], None)
        .unwrap();
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-短稿.docx",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "RT 未关单必须拒导出: {reply:?}");
    let et = err_text(&reply);
    assert!(et.contains("未关单") || et.contains("RevisionTask"), "拒因点名 RT: {et}");
    // 零产物（拒在写盘之前）
    assert!(
        !ws.join("out/投标文件-短稿.docx").exists(),
        "拒导出时不得留半成品产物"
    );

    // ---- 修复闭环：全叶补写回带内（mock 重起草——面向目标量）→ validate 复绿
    //      （VALIDATING 态 MCP draft 被状态机拒——服务面直调是测试特权，
    //      合法窗口：advance_project_to_drafting 对 Drafting+ no-op）
    for sid in &leaves {
        svc.draft_section(sid, None, None, Some("补写至配额带内"), &mut bid_tools::draft::MockGenerator)
            .unwrap();
    }
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": &pid, "scope": "all",
    })));
    assert_eq!(r["ok"], json!(true), "补写后复绿: {}", r["errors"]);
    assert_eq!(r["counts"]["blocker"], json!(0));

    // ---- 关 RT（Open → InProgress → Done，关单说明必填——服务面）
    svc.store_mut()
        .advance_rt_status(&rt_id, bid_domain::review::RTStatus::InProgress)
        .unwrap();
    svc.store_mut()
        .complete_revision_task(&rt_id, "已补写至配额带内（mock 重起草），validate 复绿 blocker=0")
        .unwrap();

    // ---- 重审过门（内容已变——新摘要）→ 导出成功
    svc.store_mut()
        .record_review(Gate::PreExportConfirm, &bid_domain::TrustedPrincipal::human("wa-lead"), Decision::Approved, vec![], None)
        .unwrap();
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-修复后.docx",
    })));
    assert_eq!(r["project_status"], "EXPORTED", "修复闭环后导出成功");
    assert_eq!(r["chapters_written"].as_u64().unwrap(), leaves.len() as u64);
}

// ---------------------------------------------------------------- 3. 导出门独立复算（即使直接批准也拒）

/// 导出门独立复算（Lead 问询补明 2026-11：「篇幅不足时，即使有人直接
/// 批准审核，是否仍应禁止导出？」——**是**）。
///
/// 写作验收门（`export_document` 前置门 1c，消费
/// `validate::writing_acceptance_findings`）不消费记审面：审核记审记录是
/// 流程事实不是质量事实，直接 Approved（零 findings 零 RT）不构成篇幅豁免。
///
/// - 面A：全叶短稿 + PreExportConfirm 直接批准（零 findings）→ 记审面
///   全绿（三门过 + 零 RT，断言锁死）→ 导出仍拒（E_UNPROCESSABLE，拒因
///   点名写作验收门 / E_TOTAL_CHARS_BELOW_MIN——#253 后全稿总量硬门是
///   篇幅把关的 BLOCKER 面；corpus-A 无评分章节，章节级 E_CHARS_BELOW_MIN
///   不在场）→ 零半成品；
/// - 面B：补写回带内 → validate 复绿 → 重走 pre_export_confirm（#180 新
///   摘要）→ 导出 EXPORTED（门禁不是死锁：修好就能出厂）；
/// - 面C：空叶 + 直接批准 → 同门拒，点名 E_SECTION_EMPTY（必写叶无
///   正文与篇幅下限同属写作验收 BLOCKER）。
#[test]
fn writing_acceptance_export_independent_of_review_approval() {
    if !corpus_ready("wa-direct") {
        return;
    }

    // ---- 面A：短稿 + 直接批准审核 → 导出仍拒
    let (mut svc, pid, ws) = acceptance_setup("direct");
    let leaves = leaf_ids(&svc);
    assert!(!leaves.is_empty());
    // Tiny 生成器：全部叶短稿（模拟 LLM 偷工——有正文但远低于下限）
    struct Tiny;
    impl bid_tools::draft::SectionGenerator for Tiny {
        fn generate(&mut self, i: &bid_tools::draft::SectionDraftInput)
            -> Result<bid_tools::draft::GeneratedDraft, String> {
            Ok(bid_tools::draft::GeneratedDraft {
                content: format!(
                    "{}短稿。",
                    i.requirements.iter().map(|r| format!("[{}]", r.id)).collect::<Vec<_>>().join("")
                ),
            })
        }
    }
    for sid in &leaves {
        svc.draft_section(sid, None, None, Some("直接批准场景"), &mut Tiny).unwrap();
    }
    // 「有人直接批准审核」：PreExportConfirm Approved 零 findings（不带
    // validate 发现、零 RT——记审面全绿）
    use bid_domain::review::{Decision, Gate};
    svc.store_mut()
        .record_review(
            Gate::PreExportConfirm,
            &bid_domain::TrustedPrincipal::human("wa-lead"),
            Decision::Approved,
            vec![],
         None)
        .unwrap();
    // 记审面确实全绿（三门过 + RT 闸清空——`open_blockers` 是 export_allowed
    // 的 RT 判据面）——拒力只能来自独立复算门
    assert!(svc.store().gate_approved(Gate::PreExportConfirm), "确认门已过");
    assert!(svc.store().open_blockers().is_empty(), "零 findings 记审不派生 RT——RT 闸面为空");

    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-直接批准.docx",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "篇幅不足即使直接批准也拒: {reply:?}");
    let et = err_text(&reply);
    assert!(et.contains("写作验收门"), "拒因点名写作验收门: {et}");
    assert!(et.contains("E_TOTAL_CHARS_BELOW_MIN"), "拒因点名全稿总量硬门: {et}");
    assert!(!ws.join("out/投标文件-直接批准.docx").exists(), "拒导出时不得留半成品");

    // ---- 面B：补写回带内 → validate 复绿 → 重走确认门 → 导出成功
    for sid in &leaves {
        svc.draft_section(sid, None, None, Some("补写至配额带内"), &mut bid_tools::draft::MockGenerator)
            .unwrap();
    }
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": &pid, "scope": "all",
    })));
    assert_eq!(r["ok"], json!(true), "补写后复绿: {}", r["errors"]);
    // 内容已变 → #180 要求重走 pre_export_confirm（新摘要绑定）
    svc.store_mut()
        .record_review(
            Gate::PreExportConfirm,
            &bid_domain::TrustedPrincipal::human("wa-lead"),
            Decision::Approved,
            vec![],
         None)
        .unwrap();
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-直接批准-修复.docx",
    })));
    assert_eq!(r["project_status"], "EXPORTED", "修复重审后导出成功");
    assert_eq!(r["chapters_written"].as_u64().unwrap(), leaves.len() as u64);

    // ---- 面C：空叶 + 直接批准 → 同门拒（E_SECTION_EMPTY 同属写作验收）
    //      半个文档场景：只起草一叶（in-band，项目进 Drafting——空项目会被
    //      状态面先行 E_STATE 拒，与本门无关），其余叶保持无正文。
    let (mut svc, pid, ws) = acceptance_setup("direct-empty");
    let leaves = leaf_ids(&svc);
    assert!(leaves.len() >= 2, "面C 需 ≥2 叶（一半有文一半空）: {}", leaves.len());
    svc.draft_section(&leaves[0], None, None, Some("仅此一叶起草"), &mut bid_tools::draft::MockGenerator)
        .unwrap();
    svc.store_mut()
        .record_review(
            Gate::PreExportConfirm,
            &bid_domain::TrustedPrincipal::human("wa-lead"),
            Decision::Approved,
            vec![],
         None)
        .unwrap();
    let reply = call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-空叶.docx",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "空叶即使直接批准也拒: {reply:?}");
    let et = err_text(&reply);
    assert!(et.contains("E_SECTION_EMPTY"), "拒因点名空章节规则: {et}");
    assert!(et.contains("写作验收门"), "同门拒（非矩阵/审核面）: {et}");
    assert!(!et.contains("E_CHARS_BELOW_MIN"), "已起草叶 in-band，不误报篇幅: {et}");
    assert!(!ws.join("out/投标文件-空叶.docx").exists(), "零半成品");
}
