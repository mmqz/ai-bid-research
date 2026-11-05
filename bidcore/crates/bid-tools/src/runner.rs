//! 场景运行器：在 corpus-A 真实夹具上执行工具序列，产出 runs/<scenario>/{db.json, trace.jsonl}。
//!
//! 工具薄壳纪律（mcp-contract 总则）：参数校验 → 状态机裁决（bid-domain，E_STATE/E_CONFLICT）
//! → 转发业务逻辑（解析/切块/检索在 bid-parse/bid-kb）→ 记 trace → 幂等键语义由 DomainStore 承载。
//! M1 范围场景：TC-01（导入+抽取）/ TC-02（25 查询×5）/ TC-07（幂等重放）/ TC-11（越级防护）。

use crate::schema::*;
use crate::service::ImportSpec;
use bid_domain::{DomainStore, ParseStatus, ProjectStatus};
use bid_kb::{ExactQuery, Kb};
use bid_parse::extract::extract_requirements;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct RunOutput {
    pub db: RunDb,
    pub trace: Vec<TraceEntry>,
}

struct Ctx {
    store: DomainStore,
    kb: Kb,
    parsed: Vec<bid_parse::ParsedFile>,
    trace: Vec<TraceEntry>,
    last_error: Option<String>,
}

impl Ctx {
    fn note(&mut self, tool: &str, ok: bool, error: Option<String>, note: String) {
        let v = self.store.content_version();
        self.last_error = error.clone();
        self.trace.push(TraceEntry {
            tool: tool.to_string(),
            ok,
            error,
            project_version: v,
            note,
        });
    }
}

fn import_files(ctx: &mut Ctx, files_dir: &Path, rels: &[(&str, &str)]) -> Result<(), String> {
    // B-1 服务化：导入管线单一真源在 service::import_files（MCP 壳同源复用）——
    // 本函数只做规格组装与 trace 记账（run 产物面），不再自持解析/落库逻辑。
    let specs: Vec<ImportSpec> = rels
        .iter()
        .map(|(rel, kind)| ImportSpec {
            upload_ref: files_dir.join(rel),
            filename: (*rel).to_string(),
            kind: (*kind).to_string(),
            expected_pages: None,
        })
        .collect();
    // #270：runner 形态无持久化工作区 → convert_dir=None（.doc 转换产物由
    // 服务层降级落系统 temp——不写用户目录）
    let report = crate::service::import_files(&mut ctx.store, &mut ctx.parsed, &specs, None);
    for o in &report.per_file {
        match (o.file_id.as_deref(), o.parse_status, o.deduplicated) {
            // 幂等重放：sha256 命中 → no-op（与原实现同口径：dedup 优先于失败面呈现）
            (Some(fid), _, true) => ctx.note("import_tender_files", true, None,
                format!("deduplicated: {} → {}", o.filename, fid)),
            // R3-S2-4：单件解析失败 → Failed 落库 + 禁引 + 继续（R-01：失败件显式
            // 登记，不整批中止——否则一个损坏件阻断全部导入）
            (Some(fid), ParseStatus::Failed, _) => ctx.note("import_tender_files", false,
                Some("E_UNPROCESSABLE".into()),
                format!("{fid} {} 解析失败，产物禁引：{}", o.filename, o.errors.join("; "))),
            (Some(fid), _, _) => {
                let st = format!("{:?}", o.parse_status).to_lowercase();
                ctx.note("import_tender_files", true, None,
                    format!("{fid} {} chunks={} status={}", o.filename, o.chunks, st));
            }
            // 不可读 upload_ref：无文件身份，不落库——逐件失败登记（服务层语义，
            // 原实现的整批 Err 上抛在此对齐 R-01：不因单件阻断整批）
            (None, _, _) => ctx.note("import_tender_files", false, Some("E_UNPROCESSABLE".into()),
                format!("{} 不可读，产物禁引：{}", o.filename, o.errors.join("; "))),
        }
    }
    Ok(())
}

/// 条款编号序列完整性（契约 §3 硬规则）：同前缀编号族须连续无缺口。
fn check_missing_sequence(parsed: &[bid_parse::ParsedFile]) -> Vec<MissingSeq> {
    use std::collections::BTreeMap;
    let re = regex::Regex::new(r"^(\d+(?:\.\d+)*)\s").unwrap();
    let mut out = vec![];
    for p in parsed {
        if !p.name.starts_with("A-") {
            continue; // 补遗/附件编号体系独立（D3/D4 面）
        }
        let mut families: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        for b in &p.blocks {
            if let Some(m) = re.captures(&b.text) {
                let no = m[1].to_string();
                if let Some((prefix, last)) = no.rsplit_once('.') {
                    if let Ok(n) = last.parse::<u64>() {
                        families.entry(prefix.to_string()).or_default().push(n);
                    }
                }
            }
        }
        for (prefix, mut nums) in families {
            nums.sort_unstable();
            nums.dedup();
            if nums.is_empty() {
                continue;
            }
            let lo = *nums.first().unwrap();
            let hi = *nums.last().unwrap();
            // 连续性：期望区间长度 == 实际个数（允许 0 起始，如 5.0）
            if nums.len() as u64 != hi - lo + 1 {
                let expected: Vec<String> = (lo..=hi).map(|n| format!("{prefix}.{n}")).collect();
                out.push(MissingSeq {
                    file_id: p.name.clone(),
                    prefix,
                    expected: expected.join(","),
                    found: nums.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(","),
                });
            }
        }
    }
    out
}

fn base_db(scenario: &str, ctx: &Ctx) -> RunDb {
    let files: Vec<FileInfo> = ctx
        .store
        .files()
        .iter()
        .map(|f| FileInfo {
            name: f.name.clone(),
            kind: f.kind.clone(),
            sha256: f.sha256.clone(),
            pages: f.pages,
            tables: f.tables,
            chunks: f.chunks,
            ocr_blocks: f.ocr_blocks,
            parse_status: format!("{:?}", f.parse_status).to_lowercase(),
            parse_summary: f.parse_summary.clone(),
        })
        .collect();
    let totals = Totals {
        files: files.len(),
        chunks: files.iter().map(|f| f.chunks).sum(),
        parse_failed: files.iter().filter(|f| f.parse_status == "failed").count(),
        ocr_required: files.iter().filter(|f| f.parse_status == "ocr_required").count(),
    };
    let requirements = ctx
        .store
        .requirements()
        .iter()
        .map(|r| ReqInfo {
            req_id: r.id.clone(),
            gt_ids: vec![], // 反标记纪律：运行产物不自报 GT 覆盖——判分只认对齐器
            category: r.category.clone(),
            clause_no: r.clause_no.clone(),
            anchors: r
                .anchors
                .iter()
                .map(|a| AnchorInfo {
                    file: a.file.clone(),
                    kind: a.kind.clone(),
                    locator: a.locator.clone(),
                    excerpt: a.excerpt.clone(),
                })
                .collect(),
            params: r.params.clone(),
            source_file: r.source_file.clone(),
        })
        .collect();
    let project = ctx.store.project().ok();
    RunDb {
        scenario: scenario.to_string(),
        milestone: "M1".to_string(),
        project: ProjectInfo {
            status: project
                .map(|p| p.status.as_str().to_string())
                .unwrap_or_default(),
            content_version: ctx.store.content_version(),
        },
        files,
        totals,
        parse: ParseInfo { missing_sequence: check_missing_sequence(&ctx.parsed) },
        requirements,
        matrix: vec![],
        citation: None,
        findings: vec![],
        exact: None,
        conflicts: vec![],
        idempotency: None,
        sections: vec![],
        recheck: Default::default(),
        export: None,
        resume_plan: None,
        stability: None,
        last_tool_result: LastToolResult { error: ctx.last_error.clone() },
    }
}

fn evidence_rels(files_dir: &Path) -> Vec<(String, String)> {
    let mut v = vec![];
    if let Ok(rd) = std::fs::read_dir(files_dir.join("evidence")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("pdf") {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                v.push((format!("evidence/{name}"), "证据".to_string()));
            }
        }
    }
    v.sort();
    v
}

pub fn run_scenario(scenario: &str, fixtures_dir: &Path) -> Result<RunOutput, String> {
    let files_dir = fixtures_dir.join("corpus-A/files");
    let mut ctx = Ctx {
        store: DomainStore::new(),
        kb: Kb { chunks: vec![] },
        parsed: vec![],
        trace: vec![],
        last_error: None,
    };

    match scenario {
        "TC-01" | "TC-02" | "TC-07" => {
            // create_bid_project
            let (_, r) = ctx
                .store
                .create_project("某市政务服务中心智能化系统工程（货物及服务）",
                                 "ZFCG-2026-GK-0311", "明标（技术标暗标评审）", "ext-corpus-A")
                .map_err(|e| e.code().to_string())?;
            ctx.note("create_bid_project", true, None, r.to_string());

            // import 主文件 4 件
            let main: Vec<(&str, &str)> = vec![
                ("A-招标文件.docx", "招标文件"),
                ("B1-设备清单.pdf", "附件"),
                ("B2-投标函格式页-扫描件.pdf", "附件"),
                ("B3-图纸目录.xlsx", "图纸"),
            ];
            import_files(&mut ctx, &files_dir, &main)?;
            assert_eq!(ctx.store.project().unwrap().status, ProjectStatus::FilesImported);

            // import 企业资料库 13 件
            let ev = evidence_rels(&files_dir);
            let ev_rels: Vec<(&str, &str)> = ev.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
            import_files(&mut ctx, &files_dir, &ev_rels)?;
            let ev_count = ctx.store.files().iter().filter(|f| f.kind == "证据").count();

            // TC-02 补充：补遗 C 入库（Q-04/Q-10 命中面）
            if scenario == "TC-02" {
                import_files(&mut ctx, &files_dir, &[("C-补遗文件01.docx", "补遗")])?;
            }

            // extract_requirements（确定性层）
            ctx.store
                .set_requirements(extract_requirements(&ctx.parsed))
                .map_err(|e| e.code().to_string())?;
            ctx.note("extract_requirements", true, None,
                     format!("requirements={} evidence_imported={}", ctx.store.requirements().len(), ev_count));

            // KB 入索引（全部已解析文件）
            let parsed = ctx.parsed.clone();
            ctx.kb = Kb::ingest(&parsed, |p| p.name.clone());

            let mut db = base_db(scenario, &ctx);

            if scenario == "TC-02" {
                // 25 查询 × 5 次重放（稳定性断言）
                let queries = load_queries(fixtures_dir)?;
                let mut per = vec![];
                let mut all_consistent = true;
                for qv in &queries {
                    let q = ExactQuery {
                        query_id: qv["query_id"].as_str().unwrap().into(),
                        query_type: qv["query_type"].as_str().unwrap().into(),
                        query: qv["query"].as_str().unwrap().into(),
                    };
                    let mut texts = vec![];
                    for _ in 0..5 {
                        let hits = ctx.kb.search_exact(&q);
                        texts.push(hits.first().map(|h| format!("{}|{}", h.file, h.hit_text)));
                    }
                    let consistent = texts.windows(2).all(|w| w[0] == w[1]);
                    all_consistent &= consistent;
                    let top = ctx.kb.search_exact(&q);
                    let (hit, hit_text, hit_file) = match top.first() {
                        Some(h) => (true, h.hit_text.clone(), h.file.clone()),
                        None => (false, String::new(), String::new()),
                    };
                    per.push(PerQuery { query_id: q.query_id.clone(), hit, hit_text, hit_file,
                                        repeats_consistent: consistent });
                }
                let hits = per.iter().filter(|p| p.hit).count();
                db.exact = Some(ExactInfo { hits, per_query: per });
                db.stability = Some(StabilityInfo { repeat_consistent: all_consistent });
            }

            if scenario == "TC-07" {
                // 幂等重放：同 sha256 再导入 A（no-op）
                let before_files = db.totals.files as u64;
                let before_chunks = db.totals.chunks as u64;
                import_files(&mut ctx, &files_dir, &[("A-招标文件.docx", "招标文件")])?;
                let after = ctx.store.files();
                let after_files = after.len() as u64;
                let after_chunks = after.iter().map(|f| f.chunks as u64).sum::<u64>();
                db = base_db(scenario, &ctx);
                db.idempotency = Some(IdempotencyInfo {
                    files_before: before_files,
                    files_after: after_files,
                    totals_before: before_chunks,
                    totals_after: after_chunks,
                });
            }
            Ok(RunOutput { db, trace: ctx.trace })
        }
        "TC-11" => {
            let (_, r) = ctx
                .store
                .create_project("某市政务服务中心智能化系统工程（货物及服务）",
                                 "ZFCG-2026-GK-0311", "明标（技术标暗标评审）", "ext-corpus-A")
                .map_err(|e| e.code().to_string())?;
            ctx.note("create_bid_project", true, None, r.to_string());
            import_files(&mut ctx, &files_dir, &[("A-招标文件.docx", "招标文件")])?;
            // 越级调用 draft_response_section → E_STATE（状态机裁决，零副作用）
            match ctx.store.guard_tool("draft_response_section") {
                Err(e) => {
                    let code = e.code().to_string();
                    ctx.note("draft_response_section", false, Some(code.clone()),
                             format!("状态机拒绝越级：{e:?}"));
                }
                Ok(()) => return Err("TC-11 预期 E_STATE 但被放行——状态机越级防护失效".into()),
            }
            let mut db = base_db(scenario, &ctx);
            db.last_tool_result = LastToolResult { error: ctx.last_error.clone() };
            Ok(RunOutput { db, trace: ctx.trace })
        }
        other => Err(format!("未知场景 {other}（M1 支持：TC-01/TC-02/TC-07/TC-11）")),
    }
}

fn load_queries(fixtures_dir: &Path) -> Result<Vec<Value>, String> {
    let p: PathBuf = fixtures_dir.join("queries/exact-25.jsonl");
    let txt = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    txt.lines().filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| e.to_string()))
        .collect()
}

/// 写运行产物：runs/<scenario>/{db.json, trace.jsonl}
pub fn write_run(out_dir: &Path, out: &RunOutput) -> Result<PathBuf, String> {
    let dir = out_dir.join(&out.db.scenario);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let db_path = dir.join("db.json");
    std::fs::write(&db_path, serde_json::to_vec_pretty(&out.db).unwrap())
        .map_err(|e| e.to_string())?;
    let mut trace_txt = String::new();
    for t in &out.trace {
        trace_txt.push_str(&serde_json::to_string(t).unwrap());
        trace_txt.push('\n');
    }
    std::fs::write(dir.join("trace.jsonl"), trace_txt).map_err(|e| e.to_string())?;
    Ok(db_path)
}
