//! #80 第二批 · 任务 B：e2e 全链 + 10 产物落盘 + 耗时记录。
//!
//! 与 e2e_pipeline（corpus-A 断言面）/ synth_tender_eval（差距评测面）不同，
//! 本测试的产出是 issue #80 §7 的【产物清单】——每步必须落盘，不能只报成功：
//!
//! ```text
//! artifacts/e2e/<run-id>/
//!   01_project.json        项目创建
//!   02_imported.json       导入结果
//!   03_requirements.json   抽取的条款
//!   04_outline.json        目录树
//!   05_drafts/             各章内容
//!   06_format-spec.json    格式规范
//!   07_export.docx         成品 docx
//!   08_export.pdf          成品 pdf
//!   09_validation.json     合规校验
//!   10_report.md           总结（耗时/字数/警告）
//! ```
//!
//! - 载体：synth-tender 夹具（仓库自带——不依赖外部真件，CI 可跑）；
//! - 起草：mock 确定性生成（真 LLM 面见 real_llm_path——那是人肉通路跑法）；
//! - PDF 软门（issue #9）：soffice 缺失 → 08 号产物记 ENV_MISSING 不判红；
//! - run-id：时间戳；另刷新 `artifacts/e2e/latest/`（同 synth-tender 惯例）；
//! - 耗时：每步 Instant 计时，进 10_report.md（Lead 要求的耗时记录面）。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn files_dir() -> PathBuf {
    repo_root().join("fixtures/synth-tender/招标文件")
}

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut()
            .unwrap()
            .insert("generator".into(), json!("mock"));
    }
    dispatch_with(
        svc,
        ToolCall {
            tool: tool.to_string(),
            args,
        },
    )
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("e2e-artifacts 步骤失败: {error}"),
    }
}

fn write_json(path: &std::path::Path, v: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(f, "{}", serde_json::to_string_pretty(v).unwrap()).unwrap();
}

/// 步骤计时台账（name → millis）。
struct Timings(Vec<(&'static str, u128)>);
impl Timings {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn mark(&mut self, name: &'static str, t: &Instant) {
        self.0.push((name, t.elapsed().as_millis()));
    }
    fn total_ms(&self) -> u128 {
        self.0.iter().map(|(_, ms)| ms).sum()
    }
}

#[test]
fn e2e_artifacts_synth_tender() {
    let mut tim = Timings::new();

    // ---- 0. 夹具在位 + PDF 链环境探测（软门）
    let files = [
        ("A-招标文件.docx", "招标文件"),
        ("B-技术规范书.docx", "招标文件"),
        ("C-补遗文件.pdf", "补遗"),
        ("E-原招标公告盖章页-扫描件.pdf", "附件"),
        ("D-图纸说明.md", "图纸"),
    ];
    let paths: Vec<PathBuf> = files.iter().map(|(n, _)| files_dir().join(n)).collect();
    let path_refs: Vec<&std::path::Path> = paths.iter().map(std::convert::AsRef::as_ref).collect();
    if bid_parse::env_preflight::skip_if_fixture_missing("e2e-artifacts", &path_refs) {
        return;
    }
    let pdf_chain_missing =
        bid_parse::env_preflight::missing_tools(bid_parse::env_preflight::FULL_PDF_CHAIN);
    let soffice_ok = bid_tools::service::soffice_available();

    let ws = std::env::temp_dir().join(format!(
        "e2e-artifacts-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&ws).unwrap();
    let mut svc = BidService::open(&ws).unwrap();

    // run-id 目录（+ latest 刷新）
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let run_id = format!("e2e-{}", chrono_like_stamp(stamp));
    let art = repo_root().join("artifacts/e2e").join(&run_id);
    std::fs::create_dir_all(&art).unwrap();

    // ---- ① 建项目（MCP §1）
    let t = Instant::now();
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": "e2e 全链产物——智慧病房物联网平台",
            "bid_deadline": "2026-11-30", "mode": "明标", "external_key": "e2e-artifacts",
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    tim.mark("01_建项目", &t);
    let proj_created = json!({
        "project_id": pid, "name": "e2e 全链产物——智慧病房物联网平台",
        "bid_deadline": "2026-11-30", "mode": "明标", "external_key": "e2e-artifacts",
    });

    // ---- ② 导入（MCP §2——A/B/C/E 批 + D 批；E_PARTIAL 容忍 + 记录）
    let t = Instant::now();
    let import_batch = |svc: &mut BidService, names: &[(&str, &str)]| -> (Value, Option<String>) {
        let specs: Vec<Value> = names.iter().map(|(n, k)| json!({
            "upload_ref": files_dir().join(n).display().to_string(), "filename": n, "kind": k
        })).collect();
        match call(
            svc,
            "import_tender_files",
            json!({ "project_id": pid, "files": specs }),
        ) {
            ToolReply::Ok(out) => (out.result, None),
            ToolReply::Err {
                error: bid_mcp::ToolError::EPartial { data, .. },
                ..
            } => (
                data.clone().unwrap_or(Value::Null),
                Some("E_PARTIAL".into()),
            ),
            ToolReply::Err { error, .. } => panic!("导入意外错误: {error}"),
        }
    };
    let (import, err1) = import_batch(&mut svc, &files[..4]);
    let (import_d, err2) = import_batch(&mut svc, &files[4..]);
    assert_eq!(
        err2.as_deref(),
        Some("E_PARTIAL"),
        "② D 件 fail-closed 拒绝"
    );
    let a_ok = import["per_file"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["filename"] == "A-招标文件.docx" && f["parse_status"] == "ok");
    assert!(a_ok, "② A 件必须解析成功: {}", import["totals"]);
    tim.mark("02_导入", &t);
    let imported = json!({
        "pdf_chain": if pdf_chain_missing.is_empty() { json!("full") } else { json!(pdf_chain_missing) },
        "batch_abc_e": { "totals": import["totals"], "error": err1 },
        "batch_d": { "totals": import_d["totals"], "error": "E_PARTIAL（预期——fail-closed 拒 .md）" },
        "per_file": import["per_file"],
    });

    // ---- ③ 抽取（MCP §3）
    let t = Instant::now();
    let extract = ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    let n_reqs = extract["counts"]["total"].as_u64().unwrap_or(0);
    assert!(n_reqs >= 60, "③ REQ 规模: {n_reqs}");
    let reqs = svc.store().requirements().to_vec();
    tim.mark("03_抽取", &t);
    let by_cat = |c: &str| reqs.iter().filter(|r| r.category == c).count();
    let requirements = json!({
        "total": reqs.len(),
        "by_category": {
            "否决项": by_cat("否决项"), "资格条件": by_cat("资格条件"),
            "评分项": by_cat("评分项"), "技术参数": by_cat("技术参数"),
            "格式要求": by_cat("格式要求"), "商务要求": by_cat("商务要求"),
            "其他": by_cat("其他"),
        },
        "requirements": reqs,
    });

    // ---- ④ 建纲建矩阵（服务面——无 MCP 工具，显式标注）
    let t = Instant::now();
    let (n_sec, _rows, _blocking) = svc
        .prepare_outline_and_matrix(OutlineSpec {
            target_total_chars: 12_000,
            max_reqs_per_leaf: 6,
        })
        .unwrap();
    assert!(n_sec >= 12, "④ 章节树: {n_sec}");
    tim.mark("04_建纲建矩阵", &t);
    let outline: Vec<Value> = svc
        .store()
        .sections()
        .iter()
        .map(|n| {
            json!({
                "id": n.id, "parent_id": n.parent_id, "title": n.title, "level": n.level,
                "order": n.order, "reqs": n.requirement_ids.len(), "target_chars": n.target_chars,
                "status": n.status.as_str(),
            })
        })
        .collect();

    // ---- ⑤ 证据 + 三门（服务面）→ 逐章起草（MCP §6 mock）
    let t = Instant::now();
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc
        .store_mut()
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "电子与智能化工程专业承包二级资质证书（synth·e2e-artifacts）".into(),
            source_file: "SYN-EV-资质证书.pdf".into(),
            source_sha256: "sha-e2e-art".into(),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 0,
                text_fingerprint: "fnv64:ea".into(),
            },
            fingerprint: "fnv64:ea".into(),
            valid_from: None,
            valid_until: None,
            subject_name: "投标人".into(),
            scope: "电子与智能化工程".into(),
        })
        .unwrap();
    let qual_ids: Vec<String> = svc
        .store()
        .requirements()
        .iter()
        // #110 任务 1 缺失回报闸：技术参数类也须证据佐证（否则 draft 拒生成）
        .filter(|r| r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数")
        .map(|r| r.id.clone())
        .collect();
    for rid in &qual_ids {
        svc.store_mut()
            .bind_evidence(&eid, rid, "2026-11-30")
            .unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    for gate in [
        Gate::MatrixApproval,
        Gate::OutlineApproval,
        Gate::PreExportConfirm,
    ] {
        svc.store_mut()
            .record_review(gate, "human:e2e-artifacts-lead", Decision::Approved, vec![])
            .unwrap();
    }
    tim.mark("05_证据绑定三门", &t);

    let t = Instant::now();
    let leaves: Vec<String> = {
        let parents: std::collections::HashSet<String> = svc
            .store()
            .sections()
            .iter()
            .filter_map(|n| n.parent_id.clone())
            .collect();
        svc.store()
            .sections()
            .iter()
            .filter(|n| !parents.contains(&n.id))
            .map(|n| n.id.clone())
            .collect()
    };
    let mut draft_stats: Vec<Value> = Vec::new();
    let mut total_words = 0u64;
    let mut all_warnings: Vec<String> = Vec::new();
    for sid in &leaves {
        let r = ok_result(call(
            &mut svc,
            "draft_response_section",
            json!({
                "project_id": &pid, "section_id": sid,
                "instructions": "按 synth-tender 招标文件逐项响应，突出同类业绩与实施方案",
            }),
        ));
        assert_eq!(r["section_status"], "DRAFTED", "⑥ 章节 {sid}");
        total_words += r["stats"]["words"].as_u64().unwrap();
        for w in r["warnings"].as_array().unwrap() {
            if let Some(s) = w.as_str() {
                all_warnings.push(format!("{sid}: {s}"));
            }
        }
        draft_stats.push(json!({
            "section_id": sid, "words": r["stats"]["words"],
            "citations": r["stats"]["citations"],
            "unanchored_claims": r["stats"]["unanchored_claims"],
        }));
    }
    assert!(total_words > 500, "⑥ 总字数 {total_words}");
    tim.mark("06_起草(mock)", &t);

    // ---- ⑦ format-spec 提取（Lead 铁律：读招标文件，不硬编码）
    let t = Instant::now();
    let (fspec, fwarnings) = bid_tools::format_spec::derive_from_requirements(&reqs);
    bid_tools::format_spec::validate(&fspec).expect("format-spec 合法域");
    tim.mark("07_format-spec", &t);
    let format_spec = json!({
        "derived": {
            "page": {"size": fspec.global.page.size,
                     "margin": {"top": fspec.global.page.margin.top, "bottom": fspec.global.page.margin.bottom,
                                "left": fspec.global.page.margin.left, "right": fspec.global.page.margin.right}},
            "font": {"body": fspec.global.font.body, "size": fspec.global.font.size,
                     "line_height": fspec.global.font.line_height},
        },
        "warnings": fwarnings,
    });

    // ---- ⑧ 校验（MCP §7——BLOCKER 清零）
    let t = Instant::now();
    let validation = ok_result(call(
        &mut svc,
        "validate_bid_document",
        json!({
            "project_id": &pid, "scope": "all",
        }),
    ));
    assert_eq!(
        validation["ok"],
        json!(true),
        "⑧ 校验未过: {}",
        validation["errors"]
    );
    assert_eq!(validation["counts"]["blocker"], json!(0));
    tim.mark("08_校验", &t);

    // ---- ⑨ 导出 docx + pdf（MCP §8——pdf 软门）
    let t = Instant::now();
    let export_docx = ok_result(call(
        &mut svc,
        "export_bid_document",
        json!({
            "project_id": &pid, "format": "docx", "output_path": "out/e2e-artifacts-投标文件-技术标.docx",
        }),
    ));
    let docx_url = export_docx["artifact"]["url"].as_str().unwrap().to_string();
    let docx_size = export_docx["artifact"]["size"].as_u64().unwrap();
    assert!(docx_size > 5_000, "⑨ docx >5KB");
    let (pdf_url, pdf_size, pdf_note) = if soffice_ok {
        let e = ok_result(call(
            &mut svc,
            "export_bid_document",
            json!({
                "project_id": &pid, "format": "pdf", "output_path": "out/e2e-artifacts-投标文件-技术标.pdf",
            }),
        ));
        let u = e["artifact"]["url"].as_str().unwrap().to_string();
        (
            u.clone(),
            e["artifact"]["size"].as_u64().unwrap(),
            Value::Null,
        )
    } else {
        (
            String::new(),
            0,
            json!("ENV_MISSING：soffice 不在——08_export.pdf 记缺，不判红（issue #9 软门）"),
        )
    };
    tim.mark("09_导出(docx+pdf)", &t);

    // ---- ⑨.5 整本自评（#100 任务 3.2——B2 接线：e2e 跑完自动调 score_document，
    //      产出 11_self-score.json；无报价 → price=null（issue：「没报价就不算」））
    let t = Instant::now();
    let self_score = ok_result(call(&mut svc, "score_document", json!({
        "project_id": &pid,
    })));
    // 自评产物形状断言（接线验收线：能回答「这份标书能得几分」）
    assert!(self_score["technical"].as_f64().is_some(), "technical 必须在座");
    assert!(self_score["price"].is_null(), "无报价 → price null（不伪装 0）");
    assert!(self_score["target"].as_f64().unwrap_or(0.0) > 0.0, "target（诚实满分）必须 > 0——synth 评分项分值已在场");
    let by_item = self_score["by_item"].as_array().unwrap();
    assert!(!by_item.is_empty(), "by_item 逐项明细非空（大纲联结生效——评分项映射到章节）");
    assert!(by_item.iter().any(|i| i["tier"].as_str().unwrap_or("").contains("价格分")), "价格项在场（f(报价) 标记）");
    // 报价决策演练（#100 §3.3——synth 4.1 口径：平均价法下浮 5%、满分 30、比率。
    //      样本竞争报价为测试数据，不入 11 产物——11 是真自评，价格样本留在测试内）
    let price_advice = ok_result(call(&mut svc, "score_price", json!({
        "method": "average", "coefficient": 0.95, "max_score": 30, "deviation": "ratio",
        "others": [1_600_000.0, 1_580_000.0, 1_560_000.0],
    })));
    assert!(price_advice["optimal_price"].as_f64().is_some(), "最优报价反推在场");
    assert!(price_advice["expected_score_at_optimal"].as_f64().is_some());
    assert!(price_advice["analysis"].as_str().unwrap_or("").contains("平均价法"), "analysis 人读面在场");
    tim.mark("11_自评(score_document+score_price)", &t);

    // ---- ⑩ 产物落盘（11 件套 + latest 刷新 + 10_report.md）
    let t = Instant::now();
    write_json(&art.join("01_project.json"), &proj_created);
    write_json(&art.join("02_imported.json"), &imported);
    write_json(&art.join("03_requirements.json"), &requirements);
    write_json(
        &art.join("04_outline.json"),
        &json!({ "nodes": outline, "leaves": leaves.len() }),
    );
    // 05_drafts/：各章内容（ChapterAssembler 真源面）
    let drafts_dir = art.join("05_drafts");
    std::fs::create_dir_all(&drafts_dir).unwrap();
    let asm = bid_tools::assembly::ChapterAssembler::open(&ws).unwrap();
    let mut drafts_index: Vec<Value> = Vec::new();
    for sid in &leaves {
        let rec = asm.record(sid).expect("章记录在盘");
        std::fs::write(
            drafts_dir.join(format!("{sid}.md")),
            format!(
                "# {sid}\n\n<!-- attempt {}｜citations {:?} -->\n\n{}\n",
                rec.attempt, rec.citations, rec.content
            ),
        )
        .unwrap();
        drafts_index.push(json!({
            "section_id": sid, "attempt": rec.attempt,
            "chars": rec.content.chars().count(),
            "citations": rec.citations,
        }));
    }
    write_json(
        &art.join("05_drafts/index.json"),
        &json!({ "chapters": drafts_index }),
    );
    write_json(&art.join("06_format-spec.json"), &format_spec);
    std::fs::copy(&docx_url, art.join("07_export.docx")).expect("docx 拷贝");
    if soffice_ok {
        std::fs::copy(&pdf_url, art.join("08_export.pdf")).expect("pdf 拷贝");
    } else {
        write_json(&art.join("08_export.pdf.json"), &pdf_note);
    }
    write_json(&art.join("09_validation.json"), &validation);
    write_json(&art.join("11_self-score.json"), &self_score);

    // 10_report.md —— 耗时/字数/警告（人读总结）
    let mut rep = String::new();
    rep.push_str("# e2e 全链产物报告（#80 第二批 · 任务 B）\n\n");
    rep.push_str(&format!(
        "- run-id：**{run_id}**｜载体：synth-tender（仓库自带夹具）\n"
    ));
    rep.push_str(&format!(
        "- 全链总耗时：**{}ms**（{} 步）\n",
        tim.total_ms(),
        tim.0.len()
    ));
    for (name, ms) in &tim.0 {
        rep.push_str(&format!("  - {name}: {ms}ms\n"));
    }
    rep.push_str(&format!(
        "- REQ：{n_reqs} 条（{} 章 / {} 叶）｜mock 总字数：{total_words}\n",
        outline.len(),
        leaves.len()
    ));
    rep.push_str(&format!(
        "- 校验：ok=true blocker=0｜导出：docx {docx_size} 字节{}｜PDF 链 {}\n",
        if soffice_ok {
            format!(" + pdf {pdf_size} 字节")
        } else {
            String::new()
        },
        if pdf_chain_missing.is_empty() {
            "完整".into()
        } else {
            format!("缺失 {pdf_chain_missing:?}")
        },
    ));
    rep.push_str(&format!(
        "- 起草告警：{} 条{}\n\n",
        all_warnings.len(),
        if all_warnings.is_empty() {
            "（零告警）".into()
        } else {
            String::new()
        }
    ));
    if !all_warnings.is_empty() {
        for w in all_warnings.iter().take(30) {
            rep.push_str(&format!("  - {w}\n"));
        }
        if all_warnings.len() > 30 {
            rep.push_str(&format!("  - ……共 {} 条\n", all_warnings.len()));
        }
        rep.push('\n');
    }
    rep.push_str("## 产物清单\n\n| # | 文件 | 说明 |\n|---|---|---|\n");
    rep.push_str("| 01 | 01_project.json | 项目创建 |\n");
    rep.push_str("| 02 | 02_imported.json | 导入结果（含 D 件 fail-closed 预期失败） |\n");
    rep.push_str("| 03 | 03_requirements.json | 抽取的条款（全量） |\n");
    rep.push_str("| 04 | 04_outline.json | 目录树 |\n");
    rep.push_str("| 05 | 05_drafts/ | 各章内容（index.json + 每章 md） |\n");
    rep.push_str("| 06 | 06_format-spec.json | 格式规范（从招标文件提取） |\n");
    rep.push_str("| 07 | 07_export.docx | 成品 docx |\n");
    rep.push_str(&format!(
        "| 08 | {} | 成品 pdf |\n",
        if soffice_ok {
            "08_export.pdf"
        } else {
            "08_export.pdf.json（ENV_MISSING 记录）"
        }
    ));
    rep.push_str("| 09 | 09_validation.json | 合规校验 |\n");
    rep.push_str("| 11 | 11_self-score.json | 整本自评（B2 接线：技术/商务/价格/逐项明细；无报价 → price=null） |\n");
    rep.push_str("| 10 | 10_report.md | 本报告 |\n");
    std::fs::write(art.join("10_report.md"), rep).unwrap();
    tim.mark("10_产物落盘", &t);

    // latest 刷新（同 synth-tender 惯例——快速查看面）
    let latest = repo_root().join("artifacts/e2e/latest");
    let _ = std::fs::remove_dir_all(&latest);
    std::fs::create_dir_all(&latest).unwrap();
    for entry in std::fs::read_dir(&art).unwrap() {
        let p = entry.unwrap().path();
        let dest = latest.join(p.file_name().unwrap());
        if p.is_dir() {
            cp_dir(&p, &dest);
        } else {
            let _ = std::fs::copy(&p, &dest);
        }
    }

    // ---- 重启恢复（产物面的收尾断言）
    let svc2 = BidService::open(&ws).unwrap();
    assert_eq!(svc2.store().project().unwrap().status.as_str(), "EXPORTED");
    assert_eq!(asm_committed(&ws), leaves.len(), "重启后工作区全量恢复");

    println!("==================== e2e-artifacts 摘要 ====================");
    println!(
        "run-id {run_id}｜总耗时 {}ms｜REQ {n_reqs}｜叶 {}｜字数 {total_words}｜docx {docx_size}B",
        tim.total_ms(),
        leaves.len()
    );
    println!("产物: {}", art.display());
}

fn asm_committed(ws: &std::path::Path) -> usize {
    bid_tools::assembly::ChapterAssembler::open(ws)
        .unwrap()
        .committed_count()
}

fn cp_dir(src: &std::path::Path, dest: &std::path::Path) {
    std::fs::create_dir_all(dest).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let p = entry.unwrap().path();
        let d = dest.join(p.file_name().unwrap());
        if p.is_dir() {
            cp_dir(&p, &d);
        } else {
            let _ = std::fs::copy(&p, &d);
        }
    }
}

/// 无 chrono 依赖的时间戳（本地时区口径由报告文字面承担——这里只要稳定可读）。
fn chrono_like_stamp(secs: u64) -> String {
    // UTC+8 固定偏移（用户域 Asia/Shanghai——工具面不引时区库）
    let s = secs + 8 * 3600;
    let days = s / 86400;
    let rem = s % 86400;
    let (h, mi, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days（Howard Hinnant 算法——无依赖的日期换算）
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{sec:02}")
}
