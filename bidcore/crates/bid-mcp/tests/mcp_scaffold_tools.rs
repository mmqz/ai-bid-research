//! #80 第三批 · 任务 B：MCP 缺口补齐（G1/G2/G3）的验收面。
//!
//! **核心断言：外部 Agent 纯 MCP 工具驱动全链**——create → import → extract
//! → build_outline（G1）→ register_evidence / bind_evidence（G2）→
//! record_review_gate ×3（G3）→ check_bid_quality（章节发现面——叶清单）→
//! draft → validate → export。**零服务面直调**（此前 e2e 的建纲/证据/三门
//! 全走 `prepare_outline_and_matrix` / `store_mut()`——外部 Agent 在 P3 前
//! 断链，评估见 docs/subagent-llm-assessment.md §四）。
//!
//! 章节发现面说明：`get_bid_project_state` 只回状态不回章节清单；叶清单的
//! 只读发现面是 `check_bid_quality`（契约 §9 sections——只列叶，按 order
//! 排序）——本测试按外部 Agent 视角走这条路。
//!
//! 载体：synth-tender A+B 双 docx（引擎解析路径——无 OCR 环境依赖，A+B
//! 实测抽取 108 条：资格 8 / 否决 48 / 评分 6 / 技术参数 15 / 格式 4 / 其他 27）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn files_dir() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/synth-tender/招标文件");
    // #183 围栏：词法 `..` 会被围栏预拒——受信夹具根统一 canonical 绝对路径
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(
        svc,
        ToolCall {
            tool: tool.to_string(),
            args,
        },
    )
}

fn ok(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("MCP 步骤失败: {error}"),
    }
}

fn err_of(reply: ToolReply) -> bid_mcp::ToolError {
    match reply {
        ToolReply::Err { error, .. } => error,
        ToolReply::Ok(out) => panic!("应失败却成功: {out:?}"),
    }
}

/// 游标翻页收集某类目的全部 REQ-id（契约 §4 过滤 + 分页——外部 Agent 的
/// 发现面走法；limit=10 强制多页——48 条否决项 = 5 页）。
fn matrix_reqs(svc: &mut BidService, pid: &str, category: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args =
            json!({ "project_id": pid, "filters": { "category": category }, "limit": 10 });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let r = ok(call(svc, "get_compliance_matrix", args));
        for row in r["rows"].as_array().expect("rows 数组") {
            out.push(row["requirement"]["id"].as_str().unwrap().to_string());
        }
        match r["next_cursor"].as_str() {
            Some(c) if !c.is_empty() => cursor = Some(c.to_string()),
            _ => break,
        }
    }
    out
}

/// A+B 导入 + 抽取（两个测试共用的前段——返回 (svc, pid)）。
fn prepared(ws: &PathBuf, external_key: &str) -> (BidService, String) {
    std::fs::create_dir_all(ws).unwrap();
    let mut svc = BidService::open(ws).unwrap().with_fs_root(files_dir());
    let pid = ok(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": "MCP 纯工具面驱动——智慧病房物联网平台",
            "bid_deadline": "2026-11-30", "mode": "明标", "external_key": external_key,
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();

    let files = json!([
        { "upload_ref": files_dir().join("A-招标文件.docx").display().to_string(),
          "filename": "A-招标文件.docx", "kind": "招标文件" },
        { "upload_ref": files_dir().join("B-技术规范书.docx").display().to_string(),
          "filename": "B-技术规范书.docx", "kind": "招标文件" },
    ]);
    let import = ok(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": files }),
    ));
    assert_eq!(
        import["totals"]["parse_failed"],
        json!(0),
        "A/B 应全成功: {import:?}"
    );

    let extract = ok(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    let total = extract["counts"]["total"].as_u64().unwrap();
    assert!(total >= 60, "A+B 抽取规模（实测 108）: {total}");
    (svc, pid)
}

#[test]
fn mcp_only_full_pipeline_drives_to_export() {
    let ws = std::env::temp_dir().join(format!("mcp-scaffold-e2e-{}", std::process::id()));
    let (mut svc, pid) = prepared(&ws, "mcp-scaffold-e2e");

    // ---- ④ build_outline（G1）：建纲 + 矩阵首建 → MATRIX_READY
    let bo = ok(call(
        &mut svc,
        "build_outline",
        json!({ "project_id": &pid, "target_total_chars": 20_000, "max_reqs_per_leaf": 8 }),
    ));
    assert_eq!(bo["status"], json!("MATRIX_READY"), "建纲后状态: {bo:?}");
    assert!(
        bo["counts"]["leaves"].as_u64().unwrap() >= 5,
        "叶规模: {bo:?}"
    );
    assert!(
        bo["counts"]["matrix_blocking"].as_u64().unwrap() > 0,
        "资格/否决未绑——blocking 应 >0: {bo:?}"
    );

    // ---- ⑤ 矩阵分页发现待绑 REQ（资格 + 否决 + 技术参数——外部 Agent 视角；
    // 技术参数也须佐证——#110 任务 1 缺失回报闸，否则 draft 拒生成）
    let mut bind_ids = matrix_reqs(&mut svc, &pid, "qualification");
    let n_qual = bind_ids.len();
    bind_ids.extend(matrix_reqs(&mut svc, &pid, "veto"));
    assert!(n_qual >= 5, "资格条件规模: {n_qual}");
    assert!(
        bind_ids.len() > n_qual,
        "否决项并入: {}",
        bind_ids.len() - n_qual
    );
    bind_ids.extend(matrix_reqs(&mut svc, &pid, "tech_param"));

    // ---- ⑥ register_evidence（G2）+ 语义位幂等重放
    let reg_args = json!({
        "project_id": &pid, "kind": "qualification",
        "title": "电子与智能化工程专业承包二级资质证书（synth·mcp-scaffold）",
        "source_file": "SYN-EV-资质证书.pdf", "source_sha256": "sha-mcp-scaffold-001",
        "subject_name": "投标人", "scope": "电子与智能化工程",
    });
    let reg = ok(call(&mut svc, "register_evidence", reg_args.clone()));
    let ev_id = reg["ev_id"].as_str().unwrap().to_string();
    assert!(ev_id.starts_with("EV-"), "EV 签发: {reg:?}");
    assert_eq!(reg["deduplicated"], json!(false), "首登非重放: {reg:?}");
    let reg2 = ok(call(&mut svc, "register_evidence", reg_args));
    assert_eq!(
        reg2["deduplicated"],
        json!(true),
        "同 payload 重放: {reg2:?}"
    );
    assert_eq!(reg2["ev_id"], json!(ev_id), "重放回原 id: {reg2:?}");

    // ---- ⑦ bind_evidence（G2）：blocking 清零 + 幂等重放
    let bind_args = json!({
        "project_id": &pid, "ev_id": &ev_id,
        "requirement_ids": bind_ids, "bid_open_date": "2026-11-30",
    });
    let bind = ok(call(&mut svc, "bind_evidence", bind_args.clone()));
    assert_eq!(
        bind["bound_new"].as_u64().unwrap() as usize,
        bind_ids.len(),
        "全部新绑: {bind:?}"
    );
    assert_eq!(
        bind["matrix"]["blocking"],
        json!(0),
        "绑定后 blocking 清零: {bind:?}"
    );
    let bind2 = ok(call(&mut svc, "bind_evidence", bind_args));
    assert_eq!(bind2["bound_new"], json!(0), "重放零新绑: {bind2:?}");
    assert_eq!(
        bind2["replayed"].as_u64().unwrap() as usize,
        bind_ids.len(),
        "重放全命中: {bind2:?}"
    );

    // ---- ⑧ 三门记审（G3）+ 门序进度真源
    let g = ok(call(
        &mut svc,
        "record_review_gate",
        json!({ "project_id": &pid, "gate": "matrix_approval", "decision": "APPROVED",
                "reviewer": "human:mcp-scaffold-lead" }),
    ));
    assert_eq!(g["receipt"]["decision"], json!("APPROVED"));
    assert_eq!(
        g["gates_approved"].as_array().unwrap().len(),
        1,
        "门序进度: {g:?}"
    );
    ok(call(
        &mut svc,
        "record_review_gate",
        json!({ "project_id": &pid, "gate": "outline_approval", "decision": "APPROVED",
                "reviewer": "human:mcp-scaffold-lead" }),
    ));
    // #180 审核内容绑定：pre_export_confirm 绑定章节正文摘要，须在全部叶
    // 起草完成后记审（挑至 ⑪'/⑫ 之间——与人类流程同序：审核定稿内容）。

    // ---- ⑨ 章节发现（check_bid_quality——叶清单只读面）
    let q = ok(call(
        &mut svc,
        "check_bid_quality",
        json!({ "project_id": &pid }),
    ));
    let secs: Vec<String> = q["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["section_id"].as_str().unwrap().to_string())
        .collect();
    assert!(secs.len() >= 5, "叶清单: {q:?}");

    // ---- ⑩ 全叶 mock 起草 → ⑪ 校验 → ⑫ 导出（零服务面）
    // #131：无补遗确认门（L3 人工出口——MCP 面）：导入面只有 A/B 招标文件
    // （无补遗件），起草前须人工确认无补遗；#118 合入时漏适配本套件。
    // 走 MCP 工具面记录（顺带断言第四门的 parse_gate 接线与门序进度）。
    let gw = ok(call(
        &mut svc,
        "record_review_gate",
        json!({ "project_id": &pid, "gate": "addendum_waiver", "decision": "APPROVED",
                "reviewer": "human:mcp-scaffold-lead" }),
    ));
    assert_eq!(gw["receipt"]["gate"], json!("addendum_waiver"));
    assert!(
        gw["gates_approved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g == "addendum_waiver"),
        "第四门进门序进度: {gw:?}"
    );
    for sid in &secs {
        let d = ok(call(
            &mut svc,
            "draft_response_section",
            json!({
                "project_id": &pid, "section_id": sid,
                "instructions": "按 synth-tender 招标文件逐项响应，突出同类业绩与实施方案",
                "generator": "mock",
            }),
        ));
        assert_eq!(d["section_status"], json!("DRAFTED"), "mock {sid}: {d:?}");
    }
    let v = ok(call(
        &mut svc,
        "validate_bid_document",
        json!({ "project_id": &pid, "scope": "all" }),
    ));
    assert_eq!(v["counts"]["blocker"], json!(0), "校验: {v:?}");
    // ---- ⑪' PreExportConfirm 记审（#180：定稿内容过门——全部叶起草完成
    //      后、导出前，绑定的才是将进产物的正文摘要）
    let g3 = ok(call(
        &mut svc,
        "record_review_gate",
        json!({ "project_id": &pid, "gate": "pre_export_confirm", "decision": "APPROVED",
                "reviewer": "human:mcp-scaffold-lead" }),
    ));
    // #180 后记审时序：addendum_waiver 已先于起草进门序进度（len=4），
    // 「三门齐」改逐门点名（弱化计数不是弱化断言——三门缺一必红）
    let gates_now: Vec<&str> = g3["gates_approved"].as_array().unwrap().iter()
        .filter_map(|g| g.as_str()).collect();
    for expected in ["matrix_approval", "outline_approval", "pre_export_confirm"] {
        assert!(gates_now.contains(&expected), "三门齐（缺 {expected}）: {g3:?}");
    }
    let e = ok(call(
        &mut svc,
        "export_bid_document",
        json!({
            "project_id": &pid, "format": "docx",
            "output_path": "out/mcp纯工具面-投标文件-技术标.docx",
        }),
    ));
    assert!(
        e["artifact"]["size"].as_u64().unwrap() > 5_000,
        "产物: {e:?}"
    );
    assert_eq!(
        svc.store().project().unwrap().status.as_str(),
        "EXPORTED",
        "终态"
    );
    println!(
        "[mcp-scaffold] 纯 MCP 全链 ✅ {} 叶｜绑 {} 条（资格 {} + 否决 {}）｜G1/G2/G3 全过→EXPORTED",
        secs.len(), bind_ids.len(), n_qual, bind_ids.len() - n_qual
    );
}

#[test]
fn scaffold_guards_and_error_paths() {
    let ws = std::env::temp_dir().join(format!("mcp-scaffold-err-{}", std::process::id()));
    std::fs::create_dir_all(&ws).unwrap();
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(files_dir());

    // ---- G1 状态门：DRAFT / FILES_IMPORTED 态建纲 → E_STATE（白名单从
    //      REQUIREMENTS_EXTRACTED 才放行 build_outline）
    let pid = ok(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": "MCP 纯工具面驱动——错误路径",
            "bid_deadline": "2026-11-30", "mode": "明标", "external_key": "mcp-scaffold-err",
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    let e = err_of(call(
        &mut svc,
        "build_outline",
        json!({ "project_id": &pid }),
    ));
    assert!(
        matches!(e, bid_mcp::ToolError::EState { .. }),
        "Draft 态建纲应 E_STATE: {e:?}"
    );

    let files = json!([
        { "upload_ref": files_dir().join("A-招标文件.docx").display().to_string(),
          "filename": "A-招标文件.docx", "kind": "招标文件" },
    ]);
    ok(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": files }),
    ));
    let e = err_of(call(
        &mut svc,
        "build_outline",
        json!({ "project_id": &pid }),
    ));
    assert!(
        matches!(e, bid_mcp::ToolError::EState { .. }),
        "FILES_IMPORTED 态建纲应 E_STATE（先 extract）: {e:?}"
    );
    ok(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));

    // ---- G2/G3 状态门：MATRIX_READY 前 → E_STATE
    for tool in ["register_evidence", "bind_evidence", "record_review_gate"] {
        let args = match tool {
            "register_evidence" => json!({ "project_id": &pid, "kind": "qualification",
                "title": "x", "source_file": "f.pdf", "source_sha256": "s",
                "subject_name": "投标人" }),
            "bind_evidence" => json!({ "project_id": &pid, "ev_id": "EV-0001",
                "requirement_ids": ["REQ-0001"], "bid_open_date": "2026-11-30" }),
            _ => json!({ "project_id": &pid, "gate": "matrix_approval",
                "decision": "APPROVED", "reviewer": "human:lead" }),
        };
        let e = err_of(call(&mut svc, tool, args));
        assert!(
            matches!(e, bid_mcp::ToolError::EState { .. }),
            "{tool} 矩阵前应 E_STATE: {e:?}"
        );
    }

    // ---- 建纲后（MATRIX_READY）的错误路径
    ok(call(
        &mut svc,
        "build_outline",
        json!({ "project_id": &pid }),
    ));

    // 参数面：缺 source_sha256（无文件身份不入账）→ E_UNPROCESSABLE
    let e = err_of(call(
        &mut svc,
        "register_evidence",
        json!({ "project_id": &pid, "kind": "qualification", "title": "x",
                "source_file": "f.pdf", "subject_name": "投标人" }),
    ));
    assert!(
        matches!(e, bid_mcp::ToolError::EUnprocessable { .. }),
        "缺 sha: {e:?}"
    );

    // 参数面：kind 非法 → E_UNPROCESSABLE
    let e = err_of(call(
        &mut svc,
        "register_evidence",
        json!({ "project_id": &pid, "kind": "不是类别", "title": "x",
                "source_file": "f.pdf", "source_sha256": "s", "subject_name": "投标人" }),
    ));
    assert!(
        matches!(e, bid_mcp::ToolError::EUnprocessable { .. }),
        "kind 非法: {e:?}"
    );

    // 参数面：locator.kind 首版未支持 → E_UNPROCESSABLE
    let e = err_of(call(
        &mut svc,
        "register_evidence",
        json!({ "project_id": &pid, "kind": "qualification", "title": "x",
                "source_file": "f.pdf", "source_sha256": "s", "subject_name": "投标人",
                "locator": { "kind": "xlsx_cell" } }),
    ));
    assert!(
        matches!(e, bid_mcp::ToolError::EUnprocessable { .. }),
        "locator 未支持: {e:?}"
    );

    // 参数面：bind 空 requirement_ids → E_UNPROCESSABLE
    let e = err_of(call(
        &mut svc,
        "bind_evidence",
        json!({ "project_id": &pid, "ev_id": "EV-0001",
                "requirement_ids": [], "bid_open_date": "2026-11-30" }),
    ));
    assert!(
        matches!(e, bid_mcp::ToolError::EUnprocessable { .. }),
        "空批次: {e:?}"
    );

    // 参数面：gate 名非法 → E_UNPROCESSABLE
    let e = err_of(call(
        &mut svc,
        "record_review_gate",
        json!({ "project_id": &pid, "gate": "final_approval",
                "decision": "APPROVED", "reviewer": "human:lead" }),
    ));
    assert!(
        matches!(e, bid_mcp::ToolError::EUnprocessable { .. }),
        "gate 非法: {e:?}"
    );

    // 登记一条有效证据 + 一条过期证据
    let ev_ok = ok(call(
        &mut svc,
        "register_evidence",
        json!({ "project_id": &pid, "kind": "qualification",
                "title": "有效资质（scaffold-err）", "source_file": "f1.pdf",
                "source_sha256": "sha-ok", "subject_name": "投标人" }),
    ))["ev_id"]
        .as_str()
        .unwrap()
        .to_string();
    let ev_expired = ok(call(
        &mut svc,
        "register_evidence",
        json!({ "project_id": &pid, "kind": "qualification",
                "title": "过期资质（scaffold-err）", "source_file": "f2.pdf",
                "source_sha256": "sha-expired", "subject_name": "投标人",
                "valid_until": "2026-01-01" }),
    ))["ev_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 绑定不存在 REQ → E_PARTIAL（逐件定位 + data 携带成功面）
    let e = err_of(call(
        &mut svc,
        "bind_evidence",
        json!({ "project_id": &pid, "ev_id": &ev_ok,
                "requirement_ids": ["REQ-0001", "REQ-9999"],
                "bid_open_date": "2026-11-30" }),
    ));
    match e {
        bid_mcp::ToolError::EPartial {
            item_errors, data, ..
        } => {
            assert_eq!(item_errors.len(), 1, "一件失败: {item_errors:?}");
            assert!(
                item_errors[0].contains("REQ-9999"),
                "逐件定位: {item_errors:?}"
            );
            assert!(data.is_some(), "E_PARTIAL 不吞成功面: {data:?}");
            assert_eq!(data.unwrap()["bound_new"], json!(1), "成功件已落库");
        }
        other => panic!("应 E_PARTIAL: {other:?}"),
    }

    // R-03 禁绑：开标日视角 EXPIRED → 逐件 E_UNPROCESSABLE → E_PARTIAL
    let e = err_of(call(
        &mut svc,
        "bind_evidence",
        json!({ "project_id": &pid, "ev_id": &ev_expired,
                "requirement_ids": ["REQ-0001"], "bid_open_date": "2026-11-30" }),
    ));
    match e {
        bid_mcp::ToolError::EPartial { item_errors, .. } => {
            assert!(item_errors[0].contains("禁"), "禁绑语义: {item_errors:?}");
        }
        other => panic!("过期证据应 E_PARTIAL: {other:?}"),
    }

    // 复审人门：AI 不得自审自过（reviewer 缺 human: 前缀 → E_UNPROCESSABLE）
    let e = err_of(call(
        &mut svc,
        "record_review_gate",
        json!({ "project_id": &pid, "gate": "matrix_approval",
                "decision": "APPROVED", "reviewer": "ai:agent-01" }),
    ));
    assert!(
        matches!(e, bid_mcp::ToolError::EUnprocessable { .. })
            || matches!(e, bid_mcp::ToolError::EState { .. }),
        "非 human 复审人被拒（域层裁决）: {e:?}"
    );

    // 门序：outline 先于 matrix → E_STATE（域层门序裁决）
    let e = err_of(call(
        &mut svc,
        "record_review_gate",
        json!({ "project_id": &pid, "gate": "outline_approval",
                "decision": "APPROVED", "reviewer": "human:lead" }),
    ));
    assert!(
        matches!(e, bid_mcp::ToolError::EState { .. }),
        "门序（矩阵先于大纲）: {e:?}"
    );

    println!("[mcp-scaffold] 错误路径面 ✅ E_STATE×4 / E_UNPROCESSABLE×5 / E_PARTIAL×2（含 R-03 禁绑 + 逐件定位）");
}
