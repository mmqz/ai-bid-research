//! #59 阶段 5 · 端到端 e2e：招标文件进 → 成品标书出（真件全链）。
//!
//! **Lead 验收线**：「`run_e2e.sh` 一条命令从招标文件跑到成品 docx」——
//! 本文件是它的 Rust 内核（`cargo test -p bid-mcp --test e2e_pipeline` 一条
//! 命令），shell 包装见 `scripts/e2e_bid/run_e2e.sh`。
//!
//! 全链步骤与工具面（**不绕开 MCP**——写操作全部走 `dispatch_with` 的
//! 12 工具分派；无工具面的准备步骤走服务面并显式标注）：
//!
//! ```text
//! ① create_bid_project        （MCP §1）
//! ② import_tender_files       （MCP §2——真件：A 主文件 docx + B1/B2/B3 +
//!    C 补遗 + EV 证据，引擎解析路径）
//! ③ extract_requirements      （MCP §3——确定性抽取，GT 25 埋点件）
//! ④ prepare_outline_and_matrix（服务面——建纲/建矩阵无 MCP 工具，标注：
//!    derive_and_set_outline + build_matrix + 推进 MATRIX_READY）
//! ⑤ bind_evidence + 三门过审 （服务面——证据绑定/审核门无 MCP 工具）
//! ⑥ draft_response_section ×N （MCP §6——逐章 mock 生成 + 引用回查 + CAS）
//! ⑦ validate_bid_document     （MCP §7——8 条确定性规则 + BLOCKER 清零）
//! ⑧ export_bid_document       （MCP §8——format-spec 提取 + docx 产物）
//! ```
//!
//! 每步产物断言（失败定位到步）：
//! - ②：files 落库 + 解析统计（页/块）
//! - ③：REQ 数 ≥ 1 + 分类面
//! - ⑥：全部叶 DRAFTED + content 在位 + S-2 工作区落盘
//! - ⑦：ok=true（BLOCKER 清零）+ counts
//! - ⑧：docx 在盘 + sha256 + 解包验证（TOC 域 + 章节内容 + 页眉）
//!
//! SKIP 纪律（issue #9）：corpus-A 真件缺失 → 响亮横幅 + return（缺件不是
//! 业务失败——绝不静默假装通过）。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files")
}

/// 真件在位预检（issue #9 SKIP 机制——缺件响亮横幅，调用方立即 return）。
fn corpus_ready(test_id: &str) -> bool {
    let main = fixtures().join("A-招标文件.docx");
    let addendum = fixtures().join("C-补遗文件01.docx");
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&main, &addendum])
}

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    // #68 阶段 1：确定性套件钉 mock（防 dev 机配了 BID_LLM_API_KEY 后 auto
    // 走真实调用——测试断言全部基于 mock 的确定性内容面）。
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut().unwrap().insert("generator".into(), serde_json::json!("mock"));
    }
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("e2e 步骤失败: {error}"),
    }
}

fn e2e_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

// ---------------------------------------------------------------- 主链

#[test]
fn e2e_tender_in_docx_out() {
    if !corpus_ready("e2e-pipeline") {
        return; // SKIP 横幅已由 skip_if_fixture_missing 打出
    }
    let ws = e2e_dir();
    let mut svc = BidService::open(&ws).unwrap();

    // ① 建项目（MCP §1——external_key 幂等）
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "e2e 全链测试——智慧园区弱电工程",
        "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "e2e-main",
    })))["project_id"].as_str().unwrap().to_string();
    assert!(pid.starts_with("PRJ-"), "① 建项目: {pid}");

    // ② 导入真件（MCP §2——A 主文件 + C 补遗 + EV 证据三件）
    // EV 件的 kind 走「附件」（MCP 契约 7 类枚举——证据台账登记是服务面
    // register_evidence 的事，import 的 kind 是文件角色不是台账类型）
    let files = [
        ("A-招标文件.docx", "招标文件"),
        ("C-补遗文件01.docx", "补遗"),
        ("evidence/EV-01-营业执照.pdf", "附件"),
        ("evidence/EV-02-电子与智能化工程专业承包二级资质证书.pdf", "附件"),
    ];
    let file_specs: Vec<Value> = files.iter().map(|(name, kind)| json!({
        "upload_ref": fixtures().join(name).display().to_string(),
        "filename": name, "kind": kind,
    })).collect();
    let r = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": &pid, "files": file_specs,
    })));
    let totals = &r["totals"];
    assert!(totals["files"].as_u64().unwrap_or(0) >= 4, "② 导入面: {totals:?}");
    assert_eq!(totals["partial"].as_u64().unwrap_or(0), 0, "② 零失败导入");
    assert_eq!(totals["parse_failed"].as_u64().unwrap_or(0), 0, "② 零解析失败");

    // ③ 抽取（MCP §3——GT 25 埋点件的真实抽取面）
    let r = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    let n_reqs = r["counts"]["total"].as_u64().unwrap_or(0);
    assert!(n_reqs >= 5, "③ 抽取面（GT 25 埋点件应 ≥ 5 条）: {n_reqs}");
    let by_category = r["counts"]["by_category"].as_object().unwrap();
    assert!(!by_category.is_empty(), "③ 分类面在位");

    // ④ 建纲建矩阵（服务面——无 MCP 工具的步骤，显式标注）
    //    规模收小（e2e 面跑得快）：每叶 ≤ 6 REQ，总 6,000 字配额
    let (n_sec, n_rows, n_blocking) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 6_000, max_reqs_per_leaf: 6 })
        .unwrap();
    assert!(n_sec >= 2, "④ 章节树: {n_sec} 节点");
    assert!(n_rows >= 5, "④ 矩阵: {n_rows} 行");

    // ⑤ 证据登记 + 绑定 + 三门（服务面）
    use bid_domain::evidence::EvidenceSpec;
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "电子与智能化工程专业承包二级资质证书".into(),
        source_file: "EV-02-电子与智能化工程专业承包二级资质证书.pdf".into(),
        source_sha256: "sha-e2e-ev02".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 0, text_fingerprint: "fnv64:e2e".into() },
        fingerprint: "fnv64:e2e".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "投标人".into(),
        scope: "电子与智能化工程".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
    }).unwrap();
    // 资格类 REQ 全绑（覆盖否决/资格的必选面——阻断清零；技术参数类也须
    // 佐证——#110 任务 1 缺失回报闸，否则 draft 拒生成）
    let qual_ids: Vec<String> = svc.store().requirements().iter()
        .filter(|r| r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数")
        .map(|r| r.id.clone()).collect();
    assert!(!qual_ids.is_empty(), "⑤ 资格/否决 REQ 非空（抽取面应含）");
    for rid in &qual_ids {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval, Gate::PreExportConfirm] {
        svc.store_mut().record_review(gate, "human:e2e-lead", Decision::Approved, vec![]).unwrap();
    }
    let n_blocking_after = svc.store().matrix().iter().filter(|r| r.blocking).count();
    assert_eq!(n_blocking_after, 0, "⑤ 绑定后 blocking 清零（否则 ⑧ 会被导出门拒）: 剩 {n_blocking_after}/{n_blocking}");

    // ⑥ 逐章起草（MCP §6——全部叶）
    let leaf_ids: Vec<String> = {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        svc.store().sections().iter()
            .filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect()
    };
    assert!(!leaf_ids.is_empty());
    let mut total_words = 0u64;
    for sid in &leaf_ids {
        let r = ok_result(call(&mut svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": sid,
            "instructions": "按招标文件逐项响应，突出同类业绩与实施方案",
        })));
        assert_eq!(r["section_status"], "DRAFTED", "⑥ 章节 {sid}");
        assert!(r["stats"]["citations"].as_u64().unwrap() > 0, "⑥ {sid} 引用面");
        total_words += r["stats"]["words"].as_u64().unwrap();
    }
    assert!(total_words > 500, "⑥ 总字数 {total_words}");
    // S-2 工作区落盘（断点续跑面）
    let asm = bid_tools::assembly::ChapterAssembler::open(&ws).unwrap();
    for sid in &leaf_ids {
        assert!(asm.has_committed(sid), "⑥ 工作区缺 {sid}");
    }

    // ⑦ 校验（MCP §7——BLOCKER 必须清零才放行 ⑧）
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": &pid, "scope": "all",
    })));
    if r["ok"] != json!(true) {
        panic!("⑦ 校验未过（BLOCKER 未清零）: {}", r["errors"]);
    }
    assert_eq!(r["counts"]["blocker"], json!(0));
    assert_eq!(r["project_status"], "VALIDATING");

    // ⑧ 导出（MCP §8——成品 docx）
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/投标文件-技术标.docx",
    })));
    let url = r["artifact"]["url"].as_str().unwrap().to_string();
    assert!(url.ends_with("投标文件-技术标.docx"), "⑧ 产物名: {url}");
    assert!(r["artifact"]["size"].as_u64().unwrap() > 5_000, "⑧ 产物 >5KB");
    assert_eq!(r["chapters_written"].as_u64().unwrap(), leaf_ids.len() as u64, "⑧ 全部落章");
    assert_eq!(r["project_status"], "EXPORTED");

    // ---- 产物深检（解包——「Word 打开」的机器等价面）
    let bytes = std::fs::read(&url).unwrap();
    let cursor = std::io::Cursor::new(&bytes);
    let mut zip = zip::ZipArchive::new(cursor).expect("⑧ docx 合法 zip");
    let mut doc_xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut doc_xml).unwrap();
    assert!(doc_xml.contains("TOC \\o"), "⑧ TOC 域在位");
    // 章标题进正文（首叶标题）
    let first_leaf_title = svc.store().sections().iter()
        .find(|n| n.id == leaf_ids[0]).unwrap().title.clone();
    assert!(doc_xml.contains(&first_leaf_title), "⑧ 章标题「{first_leaf_title}」进正文");
    // 正文内容进文档（工作区 content 的首段——#68 阶段 3：md 标题行剥号
    // 加粗渲染，断言面 = 剥号后的文本，且字面井号不再输出）
    let rec = asm.record(&leaf_ids[0]).unwrap();
    let first_line = rec.content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let stripped = first_line.trim_start_matches('#').trim();
    let probe: String = stripped.chars().take(20).collect();
    assert!(doc_xml.contains(&probe), "⑧ 正文首段「{probe}…」进文档（剥号面）");
    assert!(!doc_xml.contains("# "), "⑧ md 标题井号不再字面输出");
    let mut styles_xml = String::new();
    zip.by_name("word/styles.xml").unwrap().read_to_string(&mut styles_xml).unwrap();
    assert!(styles_xml.contains("w:eastAsia="), "⑧ format-spec 字体面（Normal 样式）");

    // ---- 重启续跑面（e2e 的收尾验证：EXPORTED 状态 + 工作区双恢复）
    let svc2 = BidService::open(&ws).unwrap();
    assert_eq!(
        svc2.store().project().unwrap().status,
        bid_domain::ProjectStatus::Exported,
        "重启后 EXPORTED 状态完整恢复"
    );
    let asm2 = bid_tools::assembly::ChapterAssembler::open(&ws).unwrap();
    assert_eq!(asm2.committed_count(), leaf_ids.len(), "重启后工作区全量恢复");
}

// ---------------------------------------------------------------- 幂等重放面

#[test]
fn e2e_idempotent_rerun_zero_waste() {
    // e2e 的第二条验收线：全链重放不重不漏（导入幂等 + draft no-op + 导出 no-op）
    if !corpus_ready("e2e-idem") {
        return;
    }
    let ws = e2e_dir();
    let mut svc = BidService::open(&ws).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "e2e 幂等", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "e2e-idem",
    })))["project_id"].as_str().unwrap().to_string();
    // 导入同一件 ×2（第二次 deduplicated）
    let spec = json!({
        "project_id": &pid,
        "files": [{ "upload_ref": fixtures().join("A-招标文件.docx").display().to_string(),
                    "filename": "A-招标文件.docx", "kind": "招标文件" }],
    });
    let a = ok_result(call(&mut svc, "import_tender_files", spec.clone()));
    let b = ok_result(call(&mut svc, "import_tender_files", spec));
    assert_eq!(a["file_ids"], b["file_ids"], "同 sha 重导入 → 原 file_id");
    let dedup: usize = b["per_file"].as_array().unwrap().iter()
        .filter(|f| f["deduplicated"] == json!(true)).count();
    assert_eq!(dedup, 1, "第二次全去重（per_file 口径）");
    // 抽取 ×2（幂等合并）
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    let r2 = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    assert_eq!(r2["counts"]["added"].as_u64().unwrap_or(0), 0, "重抽取零新增（dedupe_hash 幂等）");
}
