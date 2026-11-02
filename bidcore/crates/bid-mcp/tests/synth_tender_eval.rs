//! #80 路线 1 · synth-tender 夹具评测：导入 → 抽取 → 大纲/格式 → 全链 → 对照参考答案。
//!
//! 与 e2e_pipeline（corpus-A 全链断言）不同，本评测的核心是【对照人工核对参考
//! 答案的差距评测】——已知缺口（G1-G7，见 fixtures/synth-tender/README.md）是
//! 【登记在案的行为】，不是回归红灯：
//!
//! - **红线断言**（挂了=退化）：五件导入不炸 + D 件显式拒绝；REQ 规模与类目
//!   下限；坑位最小集（T1 前附表行 / T2 分值形态 / T6 语境传播 / T3 格式条款）；
//!   format-spec 字体提取；全链 blocker 清零 + 导出产物在盘。
//! - **差距报告**（落盘 run/latest/）：expected-requirements 逐条 diff（命中/
//!   缺失/分类偏差/条数偏差/推演准确度）、format-spec 对照、outline 对照、
//!   引擎输出中未被设计条款覆盖的 extras。
//!
//! 环境纪律（issue #9）：PDF 链工具缺失（无 poppler/tesseract 的机器）→ C/E 面
//! 软门（报告 ENV_MISSING，不判红）——A 件 docx 面不依赖外部工具，评测照跑。
//!
//! 产物（artifacts/synth-tender/run/latest/）：
//! 01_imported / 02_extracted / 03_outline / 04_format-spec / 05_validation /
//! 06_export-meta / 07_diff / report.md

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn files_dir() -> PathBuf {
    repo_root().join("fixtures/synth-tender/招标文件")
}

fn answers_dir() -> PathBuf {
    repo_root().join("fixtures/synth-tender/参考答案")
}

fn run_dir() -> PathBuf {
    repo_root().join("artifacts/synth-tender/run/latest")
}

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut().unwrap().insert("generator".into(), json!("mock"));
    }
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("synth-tender 评测步骤失败: {error}"),
    }
}

fn write_json(path: PathBuf, v: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(f, "{}", serde_json::to_string_pretty(v).unwrap()).unwrap();
}

// ---------------------------------------------------------------- 参考答案装载

struct Expected {
    syn_id: String,
    ideal_category: String,
    source_file: String,
    excerpt_key: String,
    trap: Option<String>,
    occurrences: u64,
    will_extract: bool,
    engine_category: String,
    note: String,
}

fn load_expected() -> Vec<Expected> {
    let raw = std::fs::read_to_string(answers_dir().join("expected-requirements.json")).unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    v["requirements"].as_array().unwrap().iter().map(|e| Expected {
        syn_id: e["syn_id"].as_str().unwrap().into(),
        ideal_category: e["ideal_category"].as_str().unwrap().into(),
        source_file: e["source_file"].as_str().unwrap().into(),
        excerpt_key: e["excerpt_key"].as_str().unwrap().into(),
        trap: e["trap"].as_str().map(String::from),
        occurrences: e["expect_engine_occurrences"].as_u64().unwrap_or(1),
        will_extract: e["engine_expect"]["will_extract"].as_bool().unwrap_or(true),
        engine_category: e["engine_expect"]["category"].as_str().unwrap_or("").into(),
        note: e["engine_expect"]["note"].as_str().unwrap_or("").into(),
    }).collect()
}

/// 命中判定：同源文件 + 任一锚文本包含 excerpt_key（人工核对键是源文本子串）。
fn matches_of<'r>(
    reqs: &'r [bid_domain::Requirement], e: &Expected,
) -> Vec<&'r bid_domain::Requirement> {
    reqs.iter()
        .filter(|r| {
            r.source_file == e.source_file
                && r.anchors.iter().any(|a| a.excerpt.contains(&e.excerpt_key))
        })
        .collect()
}

#[test]
fn synth_tender_eval() {
    // ---- 0. 夹具在位 + PDF 链环境探测（issue #9 软门纪律） ----
    let files = [
        ("A-招标文件.docx", "招标文件"),
        ("B-技术规范书.docx", "招标文件"),
        ("C-补遗文件.pdf", "补遗"),
        ("E-原招标公告盖章页-扫描件.pdf", "附件"),
        ("D-图纸说明.md", "图纸"),
    ];
    let paths: Vec<std::path::PathBuf> = files.iter().map(|(n, _)| files_dir().join(n)).collect();
    let path_refs: Vec<&std::path::Path> = paths.iter().map(std::convert::AsRef::as_ref).collect();
    if bid_parse::env_preflight::skip_if_fixture_missing("synth-tender-eval", &path_refs) {
        return;
    }
    let pdf_missing = bid_parse::env_preflight::missing_tools(
        bid_parse::env_preflight::FULL_PDF_CHAIN,
    );
    let pdf_env_ok = pdf_missing.is_empty();
    if !pdf_env_ok {
        println!(
            "ENV_MISSING（C/E 面软门）: {:?}——A 件 docx 面继续评测",
            pdf_missing
        );
    }

    let ws = std::env::temp_dir().join(format!("synth-tender-{}", std::process::id()));
    std::fs::create_dir_all(&ws).unwrap();
    let mut svc = BidService::open(&ws).unwrap();

    // ---- ① 建项目 ----
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "synth-tender 评测——智慧病房物联网平台",
        "bid_deadline": "2026-11-30", "mode": "明标", "external_key": "synth-tender-eval",
    })))["project_id"].as_str().unwrap().to_string();

    // ---- ② 导入（契约 §2 语义：失败件走 E_PARTIAL——data 携带成功面统计 R-01） ----
    // 批 1：A/B/C/E（可解析件）。PDF 链环境缺失时 C/E 失败 → E_PARTIAL（软门，
    // data 照常携带 A/B 的成功面——评测继续）。
    // 批 2：D-图纸说明.md 单独——fail-closed 显式拒绝面（预期 E_PARTIAL）。
    let import_batch = |svc: &mut BidService, pid: &str, names: &[(&str, &str)]| -> (Value, Option<String>) {
        let file_specs: Vec<Value> = names
            .iter()
            .map(|(n, k)| json!({"upload_ref": files_dir().join(n).display().to_string(), "filename": n, "kind": k}))
            .collect();
        match call(svc, "import_tender_files", json!({ "project_id": pid, "files": file_specs })) {
            ToolReply::Ok(out) => (out.result, None),
            ToolReply::Err { error, .. } => {
                let data = match &error {
                    bid_mcp::ToolError::EPartial { data, .. } => data.clone().unwrap_or(Value::Null),
                    other => panic!("synth-tender 导入意外错误: {other}"),
                };
                (data, Some(error.code().to_string()))
            }
        }
    };

    let (import, err1) = import_batch(&mut svc, &pid, &files[..4]); // A/B/C/E
    let totals = import["totals"].clone();
    let mut per_file_notes = serde_json::Map::new();
    for f in import["per_file"].as_array().cloned().unwrap_or_default() {
        per_file_notes.insert(
            f["filename"].as_str().unwrap().to_string(),
            json!({
                "parse_status": f["parse_status"], "pages": f["pages"],
                "chunks": f["chunks"], "ocr_tier": f["ocr_tier"],
                "errors": f["errors"],
            }),
        );
    }
    if pdf_env_ok {
        assert_eq!(err1, None, "② A/B/C/E 批应全成功: {totals:?}");
    } else {
        // 环境缺 poppler 时，以前这一批必然 E_PARTIAL。
        //
        // 但 pdf_text 纯 Rust 兜底进来之后，**文本层 PDF 已能不依赖外部工具解析**：
        // C-补遗（文本层）不再失败，E-扫描件（无文本层）仍需 OCR → 仍会失败。
        // 所以断言从「整批必须 E_PARTIAL」改为「**按文件**判」：该成功的成功、
        // 该失败的失败——这比原来更严格（原来只看整批结果，现在逐件验证）。
        let failed: Vec<&str> = import["per_file"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|f| f["parse_status"] == "failed")
                    .filter_map(|f| f["filename"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            !failed.contains(&"C-补遗文件.pdf"),
            "② 含补遗在内的文本层 PDF 必须靠纯 Rust 兜底解决，不可失败（目前失败件: {failed:?}）"
        );
        assert!(
            err1.is_none(),
            "② 文本层件（A/B/C）都应成功，不应报 E_PARTIAL；实际 {err1:?} / {totals:?}"
        );
    }
    let (import_d, err2) = import_batch(&mut svc, &pid, &files[4..]); // D 单独
    assert_eq!(err2.as_deref(), Some("E_PARTIAL"), "② D 件应 E_PARTIAL（fail-closed 拒绝非办公扩展名）");
    assert_eq!(
        import_d["totals"]["parse_failed"].as_u64(),
        Some(1),
        "② D 件失败计数: {import_d:?}"
    );
    let d_file = import_d["per_file"].as_array().unwrap().iter()
        .find(|f| f["filename"].as_str() == Some("D-图纸说明.md")).unwrap();
    assert_eq!(d_file["parse_status"], "failed", "② D 件状态: {d_file:?}");
    per_file_notes.insert(
        "D-图纸说明.md".to_string(),
        json!({"parse_status": d_file["parse_status"], "errors": d_file["errors"]}),
    );

    if pdf_env_ok {
        let c = &per_file_notes["C-补遗文件.pdf"];
        assert_eq!(c["parse_status"], "ok", "② C 件应解析成功: {c}");
        assert_eq!(c["pages"].as_u64(), Some(2), "② C 件 2 页: {c}");
        let e = &per_file_notes["E-原招标公告盖章页-扫描件.pdf"];
        assert_eq!(e["parse_status"], "ocr_required", "② E 件 OCR 面: {e}");
        assert_eq!(e["ocr_tier"], "ocr", "② E 件 OCR 应已产出（chunks>0）: {e}");
        assert!(e["chunks"].as_u64().unwrap_or(0) >= 5, "② E 件 OCR 行块 ≥5: {e}");
    }
    let a = &per_file_notes["A-招标文件.docx"];
    assert_eq!(a["parse_status"], "ok", "② A 件解析: {a}");
    let b = &per_file_notes["B-技术规范书.docx"];
    assert_eq!(b["parse_status"], "ok", "② B 件解析（G1 只影响抽取面不影响解析）: {b}");

    // ---- ③ 抽取 ----
    let extract = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    let n_reqs = extract["counts"]["total"].as_u64().unwrap_or(0);
    assert!(n_reqs >= 60, "③ REQ 规模下限（预测 81）: {n_reqs}");
    let reqs = svc.store().requirements().to_vec();

    // 类目下限（A 件面即可保证；其他 类依赖 C——软门）
    let by_cat = |c: &str| reqs.iter().filter(|r| r.category == c).count();
    assert!(by_cat("否决项") >= 40, "③ 否决项规模: {}", by_cat("否决项"));
    assert!(by_cat("资格条件") >= 6, "③ 资格条件规模: {}", by_cat("资格条件"));
    assert!(by_cat("评分项") >= 4, "③ 评分项规模: {}", by_cat("评分项"));
    assert!(by_cat("技术参数") >= 12, "③ 技术参数规模: {}", by_cat("技术参数"));
    assert!(by_cat("格式要求") >= 1, "③ 格式要求（T3 条款）在场");

    // ---- ④ 对照参考答案（差距报告——红线仅坑位最小集） ----
    let expected = load_expected();
    let mut diff_rows: Vec<Value> = Vec::new();
    let (mut n_hit, mut n_miss, mut n_cat_diff, mut n_occ_diff, mut n_pred_ok) = (0, 0, 0, 0, 0);
    for e in &expected {
        let ms = matches_of(&reqs, e);
        let hit = !ms.is_empty();
        let cat_first = ms.first().map(|r| r.category.clone()).unwrap_or_default();
        if hit {
            n_hit += 1;
        } else {
            n_miss += 1;
        }
        if hit && cat_first != e.ideal_category {
            n_cat_diff += 1;
        }
        if e.will_extract && ms.len() as u64 != e.occurrences {
            n_occ_diff += 1;
        }
        let pred_ok = if e.will_extract {
            hit && cat_first == e.engine_category
        } else {
            ms.is_empty()
        };
        if pred_ok {
            n_pred_ok += 1;
        }
        diff_rows.push(json!({
            "syn_id": e.syn_id, "trap": e.trap, "source_file": e.source_file,
            "excerpt_key": e.excerpt_key,
            "ideal_category": e.ideal_category, "engine_category": e.engine_category,
            "will_extract": e.will_extract,
            "matched": ms.len(), "expect_occurrences": e.occurrences,
            "actual_category": if hit { cat_first.clone() } else { "(缺失)".to_string() },
            "hit": hit, "category_diff": hit && cat_first != e.ideal_category,
            "prediction_ok": pred_ok, "note": e.note,
        }));
    }
    // 红线：坑位最小集
    let must = |key: &str, cat: &str, clause: Option<&str>| {
        let e = Expected {
            syn_id: String::new(), ideal_category: cat.into(), source_file: "A-招标文件.docx".into(),
            excerpt_key: key.into(), trap: None, occurrences: 1, will_extract: true,
            engine_category: String::new(), note: String::new(),
        };
        let ms = matches_of(&reqs, &e);
        assert!(!ms.is_empty(), "④ 坑位红线——「{key}」未被抽取（{cat}）");
        if let Some(c) = clause {
            assert!(
                ms.iter().any(|r| r.clause_no.as_deref() == Some(c)),
                "④ 坑位红线——「{key}」条款号应为 {c}，实得 {:?}",
                ms.iter().map(|r| r.clause_no.clone()).collect::<Vec<_>>()
            );
        }
    };
    must("未按本表第 9 项规定格式填写技术偏离表的", "否决项", Some("前附表-10")); // T1
    must("未按要求提交的，其投标作否决投标处理", "否决项", Some("前附表-6")); // T1b
    must("价格分（30 分）", "评分项", Some("4.1")); // T2
    must("正文字体为宋体、字号为小四、行距为一点五倍行距", "格式要求", None); // T3
    // T6：串通总括 + 8 子项（T7 双生另计）
    for key in [
        "视为投标人相互串通投标，其投标将被否决",
        "（1）不同投标人的投标文件由同一单位或者个人编制的",
        "（2）不同投标人委托同一单位或者个人办理投标事宜的",
        "（3）不同投标人的投标文件载明的项目管理成员为同一人的",
        "（4）不同投标人的投标文件异常一致或者投标报价呈规律性差异的",
        "（5）不同投标人的投标文件相互混装的",
        "（6）不同投标人的投标保证金从同一单位或者个人的账户转出的",
        "（7）投标人之间为谋取中标或者排斥特定投标人而采取的其他联合行动的",
        "（8）投标人之间约定部分投标人放弃投标或者中标的",
    ] {
        must(key, "否决项", None); // T6
    }
    // T2 参数面：4.1 评分项应带 score=30
    let score41 = reqs.iter().find(|r| r.category == "评分项"
        && r.clause_no.as_deref() == Some("4.1")
        && r.anchors.iter().any(|a| a.excerpt.contains("价格分（30 分）")));
    assert!(score41.is_some(), "④ T2 红线——4.1 评分项缺失");
    assert_eq!(
        score41.unwrap().params.get("score").and_then(|v| v.as_f64()),
        Some(30.0),
        "④ T2 红线——4.1 score 参数"
    );

    // extras：引擎输出未被设计条款覆盖的 REQ
    let mut extras: Vec<Value> = Vec::new();
    for r in &reqs {
        let covered = expected.iter().any(|e| {
            e.source_file == r.source_file
                && r.anchors.iter().any(|a| a.excerpt.contains(&e.excerpt_key))
        });
        if !covered {
            extras.push(json!({
                "id": r.id, "category": r.category, "clause_no": r.clause_no,
                "source_file": r.source_file,
                "excerpt": r.anchors.first().map(|a| a.excerpt.chars().take(40).collect::<String>()).unwrap_or_default(),
            }));
        }
    }

    // ---- ⑤ 大纲（结构对照——软门：仅记录差异） ----
    let (n_sec, _n_rows, _n_blocking) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 12_000, max_reqs_per_leaf: 6 })
        .unwrap();
    assert!(n_sec >= 12, "⑤ 章节树规模: {n_sec}");
    let outline_dump: Vec<Value> = svc.store().sections().iter().map(|n| json!({
        "id": n.id, "parent_id": n.parent_id, "title": n.title, "level": n.level,
        "order": n.order, "reqs": n.requirement_ids.len(), "target_chars": n.target_chars,
    })).collect();

    // ---- ⑥ format-spec 提取（Lead 铁律：不硬编码，读招标文件） ----
    let (fspec, fwarnings) = bid_tools::format_spec::derive_from_requirements(&reqs);
    assert_eq!(fspec.global.font.body, "宋体", "⑥ T3 红线——字体应从 3.3.3 提取为宋体");
    assert_eq!(fspec.global.font.size, 12.0, "⑥ T3 红线——字号小四→12pt");
    let _ = bid_tools::format_spec::validate(&fspec).expect("⑥ format-spec 合法域");

    // ---- ⑦ 全链：证据绑定 + 三门 + mock 起草 + 校验 ----
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: "电子与智能化工程专业承包二级资质证书（synth）".into(),
        source_file: "SYN-EV-资质证书.pdf".into(),
        source_sha256: "sha-synth-ev".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 0, text_fingerprint: "fnv64:synth".into() },
        fingerprint: "fnv64:synth".into(),
        valid_from: None, valid_until: None,
        subject_name: "投标人".into(), scope: "电子与智能化工程".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(), // #147
    }).unwrap();
    let qual_ids: Vec<String> = svc.store().requirements().iter()
        // #110 任务 1 缺失回报闸：技术参数类也须证据佐证（否则 draft 拒生成）
        .filter(|r| r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数")
        .map(|r| r.id.clone()).collect();
    assert!(!qual_ids.is_empty(), "⑦ 资格/否决面非空");
    for rid in &qual_ids {
        svc.store_mut().bind_evidence(&eid, rid, "2026-11-30").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    // #180 审核内容绑定：矩阵/大纲批的是响应面与结构面，保持起草前原位；
    // PreExportConfirm 绑定章节正文摘要，挑至全部叶起草完成后记审（见 ⑦'）
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval] {
        svc.store_mut().record_review(gate, &bid_domain::TrustedPrincipal::human("synth-lead"), Decision::Approved, vec![]).unwrap();
    }
    let n_blocking = svc.store().matrix().iter().filter(|r| r.blocking).count();
    assert_eq!(n_blocking, 0, "⑦ 绑定后 blocking 清零: 剩 {n_blocking}");

    let leaf_ids: Vec<String> = {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        svc.store().sections().iter()
            .filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect()
    };
    let mut total_words = 0u64;
    for sid in &leaf_ids {
        let r = ok_result(call(&mut svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": sid,
            "instructions": "按 synth-tender 招标文件逐项响应，突出同类业绩与实施方案",
        })));
        assert_eq!(r["section_status"], "DRAFTED", "⑦ 章节 {sid}");
        total_words += r["stats"]["words"].as_u64().unwrap();
    }
    assert!(total_words > 500, "⑦ 总字数 {total_words}");

    let validation = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": &pid, "scope": "all",
    })));
    assert_eq!(validation["ok"], json!(true), "⑦ 校验未过: {}", validation["errors"]);
    assert_eq!(validation["counts"]["blocker"], json!(0));

    // ---- ⑦' PreExportConfirm 记审（#180：定稿内容过门——全部叶起草完成后、
    //      导出前，绑定的才是将进产物的正文摘要）
    svc.store_mut()
        .record_review(Gate::PreExportConfirm, &bid_domain::TrustedPrincipal::human("synth-lead"), Decision::Approved, vec![])
        .unwrap();

    // ---- ⑧ 导出 ----
    let export = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/synth-tender-投标文件-技术标.docx",
    })));
    let url = export["artifact"]["url"].as_str().unwrap().to_string();
    assert!(url.ends_with("synth-tender-投标文件-技术标.docx"), "⑧ 产物名: {url}");
    assert!(export["artifact"]["size"].as_u64().unwrap() > 5_000, "⑧ 产物 >5KB");
    assert_eq!(export["project_status"], "EXPORTED");

    // ---- ⑨ 产物落盘 + 报告 ----
    let rd = run_dir();
    std::fs::create_dir_all(&rd).unwrap();
    write_json(rd.join("01_imported.json"), &json!({
        "pdf_env": if pdf_env_ok { "full" } else { "missing" },
        "pdf_missing": pdf_missing,
        "batch1_abc_e": { "totals": totals, "error": err1 },
        "batch2_d": { "totals": import_d["totals"], "error": "E_PARTIAL（预期）" },
        "per_file": per_file_notes,
    }));
    write_json(rd.join("02_extracted.json"), &json!({
        "total": reqs.len(),
        "by_category": {
            "否决项": by_cat("否决项"), "资格条件": by_cat("资格条件"),
            "评分项": by_cat("评分项"), "技术参数": by_cat("技术参数"),
            "格式要求": by_cat("格式要求"), "其他": by_cat("其他"),
        },
        "requirements": reqs,
    }));
    write_json(rd.join("03_outline.json"), &json!({ "nodes": outline_dump }));
    write_json(rd.join("04_format-spec.json"), &json!({
        "derived": {
            "page": {"size": fspec.global.page.size,
                     "margin": {"top": fspec.global.page.margin.top, "bottom": fspec.global.page.margin.bottom,
                                "left": fspec.global.page.margin.left, "right": fspec.global.page.margin.right}},
            "font": {"body": fspec.global.font.body, "size": fspec.global.font.size,
                     "line_height": fspec.global.font.line_height},
        },
        "warnings": fwarnings,
        "expected": serde_json::from_str::<Value>(
            &std::fs::read_to_string(answers_dir().join("expected-format-spec.json")).unwrap()
        ).unwrap(),
    }));
    write_json(rd.join("05_validation.json"), &validation);
    write_json(rd.join("06_export-meta.json"), &json!({
        "url": url, "size": export["artifact"]["size"],
        "chapters_written": export["chapters_written"],
        "sha256": export["artifact"].get("sha256"),
    }));
    write_json(rd.join("07_diff.json"), &json!({
        "entries": diff_rows, "extras": extras,
        "summary": {
            "expected_entries": expected.len(), "hit": n_hit, "miss": n_miss,
            "category_diff": n_cat_diff, "occurrence_diff": n_occ_diff,
            "prediction_ok": n_pred_ok, "engine_reqs": reqs.len(),
            "extras_count": extras.len(),
        },
    }));

    // report.md —— 收官测验用的人读报告
    let ideal_miss: Vec<&Value> = diff_rows.iter().filter(|r| !r["hit"].as_bool().unwrap()).collect();
    let cat_diff_rows: Vec<&Value> = diff_rows.iter()
        .filter(|r| r["category_diff"].as_bool().unwrap()).collect();
    let mut rep = String::new();
    rep.push_str("# synth-tender 评测报告（#80 路线 1）\n\n");
    rep.push_str(&format!(
        "- 引擎输出：**{} 条 REQ**（预测 81——系统性偏差：编号条款全量抽取）｜extras（未覆盖设计条款的输出）{} 条\n",
        reqs.len(), extras.len()
    ));
    {
        let mut by: std::collections::BTreeMap<String, usize> = Default::default();
        for x in &extras { *by.entry(x["category"].as_str().unwrap().to_string()).or_default() += 1; }
        let fmt: Vec<String> = by.iter().map(|(k, v)| format!("{k}:{v}")).collect();
        rep.push_str(&format!("  - extras 分类分布：{}（「其他」= 编号条款的宁多勿漏面，L1 设计内）\n", fmt.join("、")));
    }
    rep.push_str(&format!(
        "- 参考答案命中：**{}/{}**（miss {} 条——G1 的 B/E 面占多数）\n",
        n_hit, expected.len(), n_miss
    ));
    rep.push_str(&format!(
        "- 分类偏差（vs ideal）：{} 条｜条数偏差（vs 预判）：{} 条\n", n_cat_diff, n_occ_diff
    ));
    rep.push_str(&format!(
        "- 推演准确度（engine_expect 命中）：**{}/{}**\n", n_pred_ok, expected.len()
    ));
    rep.push_str(&format!(
        "- PDF 环境：{}｜format-spec 提取告警：{}\n",
        if pdf_env_ok { "完整".into() } else { format!("缺失 {pdf_missing:?}") },
        fwarnings.join("；")
    ));
    rep.push_str(&format!(
        "- 全链：校验 ok={} blocker=0｜导出 {} 字节｜mock 总字数 {}\n\n",
        validation["ok"], export["artifact"]["size"], total_words
    ));
    rep.push_str("## 未命中条款（ideal 视角——差距清单）\n\n");
    for m in &ideal_miss {
        rep.push_str(&format!(
            "- [{}] {} 「{}」——{}\n",
            m["syn_id"].as_str().unwrap(),
            m["ideal_category"].as_str().unwrap(),
            m["excerpt_key"].as_str().unwrap_or(""),
            m["note"].as_str().unwrap_or(""),
        ));
    }
    rep.push_str("\n## 分类偏差（ideal vs 引擎）\n\n");
    for m in &cat_diff_rows {
        rep.push_str(&format!(
            "- [{}] ideal={} 引擎={}（预判 {}）\n",
            m["syn_id"].as_str().unwrap(),
            m["ideal_category"].as_str().unwrap(),
            m["actual_category"].as_str().unwrap(),
            m["engine_category"].as_str().unwrap(),
        ));
    }
    if !extras.is_empty() {
        rep.push_str("\n## extras（引擎输出未被设计条款覆盖）\n\n");
        for x in extras.iter().take(20) {
            rep.push_str(&format!(
                "- {} [{}] {}「{}…」\n",
                x["id"].as_str().unwrap(),
                x["category"].as_str().unwrap(),
                x["clause_no"].as_str().map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
                x["excerpt"].as_str().unwrap(),
            ));
        }
    }
    std::fs::write(rd.join("report.md"), rep).unwrap();

    println!("==================== synth-tender 评测摘要 ====================");
    println!("REQ {} 条（预测 81）｜命中 {}/{}｜分类偏差 {}｜推演准确 {}/{}｜extras {}",
        reqs.len(), n_hit, expected.len(), n_cat_diff, n_pred_ok, expected.len(), extras.len());
    println!("报告: {}", rd.join("report.md").display());
    let _ = std::fs::remove_dir_all(&ws);
}
