//! #131 任务 1：真实语料全链验证——「合成通过 ≠ 真实通过」的检验。
//!
//! 与 e2e_artifacts（synth-tender 合成夹具，115 REQ）不同，本测试的载体是
//! **3 件真实水科 PDF**（real-corpus/water/files，283/142/76 页文本层）：
//!
//! ```text
//! 每件独立项目（不同标段的招标文件）跑同一套全链：
//!   导入 → 抽取 → GT 对照（L1/P 红线）→ 大纲 → 证据 → 起草(mock) → 质量 → 校验 → 导出
//! ```
//!
//! - **GT 红线**（issue #123 Lead 定下的纪律，逐位不退化）：
//!   L1 抽取召回 ≥ 309/318 = 0.9717、否决类精确率 ≥ 291/302 = 0.9636。
//!   匹配口径与 scripts/veto_measure 完全一致（三通道：块对齐 / 拼接包含 /
//!   ≥6 字子串；规范化 = 去全部空白）——测试内嵌同一算法，红线进 CI，
//!   不依赖外部工具人肉跑。GT 文件名 ↔ 本地文件名按 sha256 对应（A-01/A-04/A-05）。
//! - **全链行为面**（合成 e2e 断言在真实件上的重放）：6 表 + 7 格式件进 docx、
//!   <w:tbl> 数字节验证、零 markdown 星号、大纲不退化成「其他（第N块）」。
//! - **产物落盘** artifacts/real-corpus-e2e/latest/<件名>/（01~10 号 + report.md）。
//!
//! 语料缺失（real-corpus 不在仓的克隆）→ SKIP 横幅提前返回（issue #9 纪律：
//! 缺件不是业务失败，也绝不静默假装通过）。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

fn repo_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    // #183 围栏：词法 `..` 会被围栏预拒——受信夹具根统一 canonical 绝对路径
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn files_dir() -> PathBuf {
    repo_root().join("real-corpus/water/files")
}

fn gt_path() -> PathBuf {
    repo_root().join("artifacts/veto-gt/ground-truth.json")
}


fn out_root() -> PathBuf {
    repo_root().join("artifacts/real-corpus-e2e")
}

/// 3 件真实 PDF（本地名）↔ GT 名（sha256 对应——A-01/A-04/A-05）。
const CORPUS: [(&str, &str); 3] = [
    (
        "A-塔里木水库除险加固招标文件.pdf",
        "A-01_塔里木河希尼尔水库坝基防渗施工标_招标文件.pdf",
    ),
    (
        "A-某水利施工标段招标文件.pdf",
        "A-04_某水利工程施工标段_施工招标文件.pdf",
    ),
    (
        "A-伊犁水库质量检测招标文件.pdf",
        "A-05_伊犁州奎屯东郊水库质量检测_招标文件.pdf",
    ),
];

/// #123 红线基线（veto-measure 于 main 51874cd 实测，逐位一致）：
/// covered 309/318、veto_tp 291/302。
const L1_BASELINE: f64 = 309.0 / 318.0;
const P_BASELINE: f64 = 291.0 / 302.0;

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut()
            .unwrap()
            .insert("generator".into(), json!("mock"));
    }
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("real-corpus-e2e 步骤失败: {error}"),
    }
}

fn write_json(path: &std::path::Path, v: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(f, "{}", serde_json::to_string_pretty(v).unwrap()).unwrap();
}

// ---------------------------------------------------------------- GT 匹配（veto-measure 同口径）

/// GT 标注（ground-truth.json annotations 的宽松子集——只取测量需要的面）。
struct GtAnn {
    file: String,
    text: String,
    is_veto: bool,
    clause_blocks: Vec<u64>,
    main_block_idx: Option<u64>,
}

fn load_gt() -> Vec<GtAnn> {
    let raw = std::fs::read_to_string(gt_path()).expect("ground-truth.json 在仓");
    let v: Value = serde_json::from_str(&raw).expect("GT 合法 JSON");
    v["annotations"]
        .as_array()
        .expect("GT annotations 数组")
        .iter()
        .filter(|a| a["is_veto"].as_bool().unwrap_or(false))
        .map(|a| {
            let idx_of = |x: &Value| -> Option<u64> {
                match x {
                    Value::Number(n) => n.as_u64(),
                    Value::String(s) => s.trim().parse().ok(),
                    _ => None,
                }
            };
            GtAnn {
                file: a["file"].as_str().unwrap_or_default().to_string(),
                text: a["text"].as_str().unwrap_or_default().to_string(),
                is_veto: a["is_veto"].as_bool().unwrap_or(false),
                clause_blocks: a["clause_blocks"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(idx_of).collect())
                    .unwrap_or_default(),
                main_block_idx: idx_of(&a["main_block_idx"]),
            }
        })
        .filter(|a| a.is_veto)
        .collect()
}

/// 规范化：去全部空白字符（含全角空格）——与 GT 标注跨行拼接口径一致。
fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// 单件测量（veto-measure 三通道完整口径）：
/// 返回 (gt_true, covered_L1, veto_cls_L2, veto_req, veto_tp)。
#[allow(clippy::type_complexity)]
fn measure_one(
    reqs: &[bid_domain::Requirement],
    parsed: &[bid_parse::ParsedFile],
    gt_file: &str,
    local_name: &str,
) -> (usize, usize, usize, usize, usize) {
    let all_gt = gt_all();
    let gts: Vec<&GtAnn> = all_gt.iter().filter(|a| a.file == gt_file).collect();

    // 块对齐表：anchor (locator, excerpt) → 块 idx（与解析器固化序号同源）
    let blocks: Vec<&bid_parse::Block> = parsed
        .iter()
        .filter(|p| p.name == local_name)
        .flat_map(|p| p.blocks.iter())
        .collect();
    let mut anchor_idx: BTreeSet<u64> = BTreeSet::new();
    for r in reqs {
        for a in &r.anchors {
            if let Some(b) = blocks.iter().find(|b| b.locator == a.locator && b.text == a.excerpt) {
                anchor_idx.insert(b.idx as u64);
            }
        }
    }
    // REQ 预计算
    struct R {
        joined: String,
        anchors: Vec<String>,
        cat: String,
    }
    let rs: Vec<R> = reqs
        .iter()
        .map(|r| R {
            joined: norm(&r.anchors.iter().map(|a| a.excerpt.as_str()).collect::<String>()),
            anchors: r.anchors.iter().map(|a| norm(&a.excerpt)).collect(),
            cat: r.category.clone(),
        })
        .collect();

    let mut cov = 0usize;
    let mut l2 = 0usize;
    for a in &gts {
        let g = norm(&a.text);
        let gt_block_ids: Vec<u64> = a
            .clause_blocks
            .iter()
            .copied()
            .chain(a.main_block_idx)
            .collect();
        let block_hit = gt_block_ids.iter().any(|i| anchor_idx.contains(i));
        let mut hit_veto = false;
        let mut covered_flag = block_hit;
        for r in &rs {
            let full = r.joined.contains(&g);
            let frag = r
                .anchors
                .iter()
                .any(|an| char_len(an) >= 6 && g.contains(an));
            if full || frag {
                covered_flag = true;
                if r.cat == "否决项" {
                    hit_veto = true;
                }
            }
        }
        if covered_flag {
            cov += 1;
            if hit_veto {
                l2 += 1;
            }
        }
    }
    // 精确率（否决类 REQ 的 TP）
    let mut veto_n = 0usize;
    let mut veto_tp = 0usize;
    for r in &rs {
        if r.cat != "否决项" {
            continue;
        }
        veto_n += 1;
        if char_len(&r.joined) < 6 {
            continue; // 碎屑不算 TP
        }
        let tp = gts.iter().any(|a| {
            let g = norm(&a.text);
            g.contains(&r.joined) || (r.joined.contains(&g) && char_len(&g) >= 6)
        });
        if tp {
            veto_tp += 1;
        }
    }
    (gts.len(), cov, l2, veto_n, veto_tp)
}

static GT_STORE: std::sync::OnceLock<Vec<GtAnn>> = std::sync::OnceLock::new();

fn gt_all() -> &'static [GtAnn] {
    GT_STORE.get_or_init(load_gt)
}

// ---------------------------------------------------------------- docx 解包（e2e_artifacts 同款）

fn docx_document_xml(path: &std::path::Path) -> String {
    use std::io::Read;
    let f = std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("打开 docx 失败（{e}）: {}", path.display()));
    let mut zip = zip::ZipArchive::new(f).expect("docx 是合法 zip 包");
    let mut xml = String::new();
    for i in 0..zip.len() {
        if zip.by_index(i).unwrap().name() == "word/document.xml" {
            zip.by_index(i).unwrap().read_to_string(&mut xml).unwrap();
        }
    }
    assert!(!xml.is_empty(), "document.xml 必须在包内: {}", path.display());
    xml
}

fn count_occurrences(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

// ---------------------------------------------------------------- 主测试

#[test]
fn real_corpus_full_chain() {
    // ---- 0. 语料在位（缺件 → SKIP 横幅，不静默假装通过） ----
    let paths: Vec<PathBuf> = CORPUS.iter().map(|(n, _)| files_dir().join(n)).collect();
    let path_refs: Vec<&std::path::Path> = paths.iter().map(std::convert::AsRef::as_ref).collect();
    if bid_parse::env_preflight::skip_if_fixture_missing("real-corpus-e2e", &path_refs) {
        return;
    }
    // GT 在位性（GT 缺失时 L1/P 红线无从对照——显式失败而非跳过：
    // 本测试的使命就是 GT 对照，缺 GT 等于测试失效）
    assert!(
        gt_path().exists(),
        "GT 不在仓: {}（real-corpus-e2e 的红线对照依赖它）",
        gt_path().display()
    );

    // #169 环境前置：坏 poppler（缺 CMap 资源包，Windows runner 实测形态）
    // 下，加密水科件「首选路乱码 + 兜底不解密」被 parse_file 的 fail-loud
    // 显式拒绝——L1/P 红线在此环境无从对照，SKIP 留横幅（issue #9 口径：
    // 环境差异不伪装成业务回归；非 #169 的解析失败照旧 panic）。
    for (local, _) in CORPUS.iter() {
        if let Err(e) = bid_parse::parse_file(&files_dir().join(local), local) {
            if e.contains("乱码") {
                eprintln!(
                    "【SKIP】{local} 在本环境解不开（#169 乱码 fail-loud：{e}）——坏 poppler（缺 CMap 资源包），红线对照不可信，真实语料验收不伪装通过"
                );
                return;
            }
            panic!("水科 PDF 应可解析: {e}");
        }
    }

    let latest = out_root().join("latest");
    let _ = std::fs::remove_dir_all(&latest);
    std::fs::create_dir_all(&latest).unwrap();

    // 全局累计（L1/P 红线是 3 件合并口径——与 Lead 的 veto-measure 快照一致）
    let (mut tot_gt, mut tot_cov, mut tot_l2, mut tot_veto_req, mut tot_veto_tp) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut per_file: Vec<Value> = Vec::new();

    for (local, gt_name) in CORPUS.iter() {
        let t0 = Instant::now();
        let stem = local.trim_end_matches(".pdf");
        let art = latest.join(stem);
        std::fs::create_dir_all(&art).unwrap();

        // ---------------- ① 建项目 + ② 导入（单件——每件是独立标段）
        let ws = std::env::temp_dir().join(format!(
            "real-corpus-e2e-{}-{}",
            stem.chars().take(8).collect::<String>(),
            std::process::id()
        ));
        std::fs::create_dir_all(&ws).unwrap();
        let mut svc = BidService::open(&ws).unwrap().with_fs_root(files_dir());
        let pid = ok_result(call(
            &mut svc,
            "create_bid_project",
            json!({
                "name": format!("真实语料全链——{stem}"),
                "bid_deadline": "2026-12-31", "mode": "明标",
                "external_key": format!("real-corpus-e2e-{stem}"),
            }),
        ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();

        let import = ok_result(call(
            &mut svc,
            "import_tender_files",
            json!({ "project_id": &pid, "files": [
                { "upload_ref": files_dir().join(local).display().to_string(),
                  "filename": local, "kind": "招标文件" }
            ]}),
        ));
        // 断言 1：3 件真实 PDF 都能导入（parse ok + 页数在合理域）
        let f0 = &import["per_file"][0];
        assert_eq!(
            f0["parse_status"], "ok",
            "② {local} 解析必须成功（真实件导入面）: {f0:?}"
        );
        let pages = f0["pages"].as_u64().unwrap_or(0);
        assert!(pages >= 50, "② {local} 页数 {pages}（真实水科件 76-284 页域）");
        let blocks_n = f0["chunks"].as_u64().unwrap_or(0);
        assert!(blocks_n >= 1000, "② {local} 行块 {blocks_n}（真实件 2272-8107 域）");
        let t_import = t0.elapsed().as_millis();

        // ---------------- ③ 抽取
        let t = Instant::now();
        let extract = ok_result(call(
            &mut svc,
            "extract_requirements",
            json!({ "project_id": &pid }),
        ));
        let n_reqs = extract["counts"]["total"].as_u64().unwrap_or(0);
        let reqs = svc.store().requirements().to_vec();
        let by_cat = |c: &str| reqs.iter().filter(|r| r.category == c).count();
        let t_extract = t.elapsed().as_millis();

        // ---------------- ③.5 GT 对照（红线——veto-measure 同口径）
        let (gt, cov, l2, veto_req, veto_tp) =
            measure_one(&reqs, svc.parsed(), gt_name, local);
        tot_gt += gt;
        tot_cov += cov;
        tot_l2 += l2;
        tot_veto_req += veto_req;
        tot_veto_tp += veto_tp;
        println!(
            "  {local}: REQ {n_reqs}｜gt {gt} cov {cov} l2 {l2}｜veto_req {veto_req} tp {veto_tp}"
        );

        // ---------------- ④ 大纲 + 矩阵
        let t = Instant::now();
        let (n_sec, _rows, _blocking) = svc
            .prepare_outline_and_matrix(OutlineSpec {
                target_total_chars: 60_000,
                max_reqs_per_leaf: 6,
            })
            .unwrap();
        let sections = svc.store().sections().to_vec();
        let t_outline = t.elapsed().as_millis();

        // ---------------- ⑤ 证据绑定（资格/否决/技术参数——#110 缺失回报闸）
        let t = Instant::now();
        use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
        use bid_domain::locator::EngineLocator;
        let (eid, _) = svc
            .store_mut()
            .register_evidence(EvidenceSpec {
                kind: EvidenceKind::Qualification,
                title: "水利水电工程施工总承包二级资质证书（演示证据）".into(),
                source_file: "REAL-EV-资质证书.pdf".into(),
                source_sha256: format!("sha-real-{stem}"),
                locator: EngineLocator::PdfPageText {
                    page: 1,
                    text_fingerprint: format!("fnv64:real-{stem}"),
                },
                fingerprint: format!("fnv64:real-{stem}"),
                valid_from: None,
                valid_until: None,
                subject_name: "投标人".into(),
                scope: "水利水电工程".into(),
                client: None,
                amount_wan: None,
                completed_on: None,
                fields: Default::default(), // #147
            })
            .unwrap();
        let qual_ids: Vec<String> = svc
            .store()
            .requirements()
            .iter()
            .filter(|r| {
                r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数"
            })
            .map(|r| r.id.clone())
            .collect();
        for rid in &qual_ids {
            svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
        }
        // 业绩证据（业绩一览表真源——投标方输入面，3 条过 MIN_PERFORMANCE_COUNT 门槛）
        for i in 0..3 {
            let (_eid2, _) = svc
                .store_mut()
                .register_evidence(EvidenceSpec {
                    kind: EvidenceKind::Performance,
                    title: format!("某水利枢纽除险加固工程施工第{i}标段（演示业绩）"),
                    source_file: format!("REAL-EV-业绩合同-{i}.pdf"),
                    source_sha256: format!("sha-real-perf-{stem}-{i}"),
                    locator: EngineLocator::PdfPageText {
                        page: 1,
                        text_fingerprint: format!("fnv64:real-perf-{stem}-{i}"),
                    },
                    fingerprint: format!("fnv64:real-perf-{stem}-{i}"),
                    valid_from: Some("2021-03-01".into()),
                    valid_until: None,
                    subject_name: "投标人".into(),
                    scope: "水利水电工程".into(),
                    client: Some(match i {
                        0 => "某河流域水利管理处".into(),
                        1 => "某市水库管理中心".into(),
                        _ => "某自治区水利厅建设处".into(),
                    }),
                    amount_wan: Some([920.0, 1150.0, 2380.0][i]),
                    completed_on: Some(["2022-11-30", "2023-08-15", "2024-06-20"][i].into()),
                    fields: Default::default(), // #147
                })
                .unwrap();
        }
        let _ = svc.store_mut().build_matrix().unwrap();
        let t_evidence = t.elapsed().as_millis();

        // ---------------- ⑥ 起草（mock 确定性）
        let t = Instant::now();
        let leaves: Vec<String> = {
            let parents: std::collections::HashSet<String> = sections
                .iter()
                .filter_map(|n| n.parent_id.clone())
                .collect();
            sections
                .iter()
                .filter(|n| !parents.contains(&n.id))
                .map(|n| n.id.clone())
                .collect()
        };
        // ⑥.0 真实语料发现 #1（#131）：单件招标文件（无补遗件）被 L3 的
        //     MissingAddendum 阻塞——synth e2e 因夹具自带 C-补遗件踩不到。
        //     先验证 fail-closed 面（阻塞必须在场），再走人工确认门
        //     （Gate::AddendumWaiver——阻塞提示语承诺的「明确确认本项目
        //     无补遗后再继续」通路）放行。
        assert!(!leaves.is_empty(), "④ {local} 叶章节非空");
        let first_leaf = leaves[0].clone();
        match call(
            &mut svc,
            "draft_response_section",
            json!({
                "project_id": &pid, "section_id": &first_leaf,
                "instructions": "验证无补遗阻塞面",
            }),
        ) {
            ToolReply::Err { error, .. } => {
                let msg = error.to_string();
                assert!(
                    msg.contains("补遗"),
                    "⑥.0 预期 MissingAddendum 阻塞（无补遗件），实际: {msg}"
                );
            }
            ToolReply::Ok(_) => panic!(
                "⑥.0 {local} 无补遗件时起草应被 MissingAddendum 阻塞（fail-closed 面）——L3 语义回归"
            ),
        }
        use bid_domain::review::{Decision, Gate};
        svc.store_mut()
            .record_review(
                Gate::AddendumWaiver,
                "human:real-corpus-lead",
                Decision::Approved,
                vec![],
            )
            .unwrap();
        // 两道前置主门（#180 审核内容绑定：矩阵/大纲批的是响应面与结构面，
        // 保持起草前原位；PreExportConfirm 绑定章节正文摘要，挑至全部叶
        // 起草完成后记审——见 ⑨ 前，与人类流程同序：审核定稿内容）
        for gate in [Gate::MatrixApproval, Gate::OutlineApproval] {
            svc.store_mut()
                .record_review(gate, "human:real-corpus-lead", Decision::Approved, vec![])
                .unwrap();
        }
        let mut total_words = 0u64;
        let mut empty_drafts = 0usize;
        for sid in &leaves {
            let r = ok_result(call(
                &mut svc,
                "draft_response_section",
                json!({
                    "project_id": &pid, "section_id": sid,
                    "instructions": "按招标文件逐项响应，突出同类水利工程业绩与实施方案",
                }),
            ));
            assert_eq!(r["section_status"], "DRAFTED", "⑥ {local} 章节 {sid}");
            let w = r["stats"]["words"].as_u64().unwrap();
            if w == 0 {
                empty_drafts += 1;
            }
            total_words += w;
        }
        let t_draft = t.elapsed().as_millis();

        // ---------------- ⑦ 质量（check_bid_quality——真实抽取面的评分真实性）
        let t = Instant::now();
        let quality = ok_result(call(
            &mut svc,
            "check_bid_quality",
            json!({ "project_id": &pid, "scope": "all" }),
        ));
        let t_quality = t.elapsed().as_millis();

        // ---------------- ⑧ 校验
        let t = Instant::now();
        let validation = ok_result(call(
            &mut svc,
            "validate_bid_document",
            json!({ "project_id": &pid, "scope": "all" }),
        ));
        let t_validate = t.elapsed().as_millis();

        // ---------------- ⑧.5 主体信息档（人员/设备两表真源——投标方输入面；
        //      首轮漏存 profile 导致两表缺席——这是接线遗漏不是产品行为）
        let profile = bid_domain::party::BidderProfile {
            company_name: "某水利水电工程有限公司".into(),
            legal_rep: "周建军".into(),
            legal_rep_id: "650102197309152317".into(),
            registered_capital: 8600.0,
            address: "某省某市滨河路 128 号".into(),
            bank_name: "中国农业银行股份有限公司某市分行".into(),
            bank_account: "65050110036699999".into(),
            tax_no: "91650100MA77XXXXXX".into(),
            contact_person: "高文澜".into(),
            contact_phone: "13900000000".into(),
            legal_rep_position: "董事长".into(),
            agent: "郑宇飞".into(),
            agent_id: "650104199105062315".into(),
            postcode: "830000".into(),
            established_on: "2005-04-12".into(),
            qualification: "水利水电工程施工总承包贰级".into(),
            personnel: vec![
                bid_domain::party::PersonnelFact {
                    position: "项目经理".into(),
                    name: "吴建波".into(),
                    title: "高级工程师 / 一级建造师（水利水电）".into(),
                    major: "水利水电工程".into(),
                    years: Some(18),
                    note: "在岗，社保在本单位".into(),
                },
                bid_domain::party::PersonnelFact {
                    position: "技术负责人".into(),
                    name: "马雪梅".into(),
                    title: "高级工程师".into(),
                    major: "水工结构".into(),
                    years: Some(14),
                    note: String::new(),
                },
                bid_domain::party::PersonnelFact {
                    position: "安全员".into(),
                    name: "艾山江".into(),
                    title: "专职安全员（C 证）".into(),
                    major: "安全工程".into(),
                    years: Some(7),
                    note: String::new(),
                },
            ],
            equipment: vec![
                bid_domain::party::EquipmentFact {
                    name: "液压挖掘机".into(),
                    model: "SY365H-9".into(),
                    quantity: Some(6),
                    unit: "台".into(),
                    origin: "国产".into(),
                    spec: "1.8m³ 斗容，适用土方与围堰开挖".into(),
                    arrival: "进场令后 7 日内".into(),
                },
                bid_domain::party::EquipmentFact {
                    name: "混凝土拌合站".into(),
                    model: "HZS120".into(),
                    quantity: Some(2),
                    unit: "座".into(),
                    origin: "国产".into(),
                    spec: "120m³/h，自动计量".into(),
                    arrival: "进场令后 10 日内".into(),
                },
                bid_domain::party::EquipmentFact {
                    name: "振动碾压设备".into(),
                    model: "XS203J".into(),
                    quantity: Some(4),
                    unit: "台".into(),
                    origin: "国产".into(),
                    spec: "20t 激振力，填筑压实".into(),
                    arrival: "进场令后 10 日内".into(),
                },
            ],
        };
        profile.save(&ws).expect("bidder-profile.json 落盘");

        // ---------------- ⑧' PreExportConfirm 记审（#180：定稿内容过门——
        //      全部叶起草完成后、导出前，绑定的才是将进产物的正文摘要）
        svc.store_mut()
            .record_review(Gate::PreExportConfirm, "human:real-corpus-lead", Decision::Approved, vec![])
            .unwrap();

        // ---------------- ⑨ 导出（报价决策随导出——六表七件接线）
        let t = Instant::now();
        let bid_amount = 9_860_000.0_f64;
        let export = ok_result(call(
            &mut svc,
            "export_bid_document",
            json!({
                "project_id": &pid, "format": "docx",
                "output_path": format!("out/real-corpus-{stem}-投标文件-技术标.docx"),
                "profile": {
                    "bid_amount": bid_amount,
                    "price_breakdown": [
                        { "item": "主体工程", "unit": "项", "quantity": 1.0, "unit_price": 5_600_000.0 },
                        { "item": "临时工程与安全度汛", "unit": "项", "quantity": 1.0, "unit_price": 1_800_000.0 },
                        { "item": "设备采购与安装", "unit": "项", "quantity": 1.0, "unit_price": 1_450_000.0 },
                        { "item": "检测与观测", "unit": "项", "quantity": 1.0, "unit_price": 610_000.0 },
                        { "item": "售后服务（两年质保）", "unit": "项", "quantity": 1.0, "unit_price": 400_000.0 }
                    ],
                    "project_name": stem,
                    "duration_days": 540,
                    "validity_days": 90,
                    "deposit_amount": 200_000.0,
                    "legal_rep_gender": "男",
                },
            }),
        ));
        let docx_url = export["artifact"]["url"].as_str().unwrap().to_string();
        let docx_size = export["artifact"]["size"].as_u64().unwrap();
        let t_export = t.elapsed().as_millis();

        // ---------------- ⑨.5 docx 深断言（不信任工具返回值——解包数字节）
        let docx_xml = docx_document_xml(std::path::Path::new(&docx_url));
        let tables_actual = count_occurrences(&docx_xml, "<w:tbl>");
        let form_probes = [
            "附件一、投标函",
            "附件二、法定代表人身份证明书",
            "附件三、授权委托书",
            "附件四、投标保证金退还申请",
            "附件五、资格声明",
            "附件六、无重大违法记录声明",
            "附件七、廉洁承诺书",
        ];
        let forms_found = form_probes
            .iter()
            .filter(|p| docx_xml.contains(*p))
            .count();
        let asterisk_actual = count_occurrences(&docx_xml, "**");

        // 表格预期（真实水利件的诚实口径——#131 发现 #2）：技术参数偏离表
        // 仅当有「技术参数」类 REQ 才生成；三件真实水利标书技术参数 REQ 均 0
        // （确定性型号正则对水利工程天然失效——real-eval §3 已登记的设计边界，
        // 归 M2 模型辅助归类）。缺席必须伴随响亮告警（不得静默）。其余五表
        // （概况/报价/业绩/人员/设备）由本次全链的真源（REQ + 报价决策 +
        // 业绩证据 + profile）驱动，必须全部在。
        let tech_req_count = by_cat("技术参数");
        let tables_expected = 5 + usize::from(tech_req_count > 0);
        let warnings_text = export["warnings"]
            .as_array()
            .map(|ws| {
                ws.iter()
                    .filter_map(|w| w.as_str())
                    .collect::<Vec<_>>()
                    .join("\u{0000}")
            })
            .unwrap_or_default();
        if tech_req_count == 0 {
            assert!(
                warnings_text.contains("技术参数偏离表未生成"),
                "⑨.5 {local} 技术参数 REQ 为 0 时缺席告警必须在场（不得静默缺表）: {:?}",
                export["warnings"]
            );
        } else {
            assert!(
                !warnings_text.contains("技术参数偏离表未生成"),
                "⑨.5 {local} 有 {tech_req_count} 条技术参数 REQ，偏离表缺席告警不应在场"
            );
        }

        // ---------------- ⑩ 产物落盘
        write_json(&art.join("01_project.json"), &json!({
            "project_id": pid, "name": format!("真实语料全链——{stem}"),
            "source_pdf": local, "gt_file": gt_name,
            "pages": pages, "blocks": blocks_n,
        }));
        write_json(&art.join("02_imported.json"), &import);
        write_json(
            &art.join("03_requirements.json"),
            &json!({
                "total": reqs.len(),
                "by_category": {
                    "否决项": by_cat("否决项"), "资格条件": by_cat("资格条件"),
                    "评分项": by_cat("评分项"), "技术参数": by_cat("技术参数"),
                    "格式要求": by_cat("格式要求"), "商务要求": by_cat("商务要求"),
                    "其他": by_cat("其他"),
                },
                "requirements": reqs,
            }),
        );
        write_json(&art.join("04_outline.json"), &json!({
            "sections": sections.iter().map(|n| json!({
                "id": n.id, "parent_id": n.parent_id, "title": n.title, "level": n.level,
                "reqs": n.requirement_ids.len(), "target_chars": n.target_chars,
            })).collect::<Vec<_>>(),
        }));
        let asm = bid_tools::assembly::ChapterAssembler::open(&ws).unwrap();
        let drafts_dir = art.join("05_drafts");
        std::fs::create_dir_all(&drafts_dir).unwrap();
        let mut drafts_index: Vec<Value> = Vec::new();
        // 逐章 md 只落样例（每件前 3 章——真实件 60-235 叶，全落会把产物面
        // 撑到数百文件；全量正文已在 06_export.docx，逐章元数据在 index）
        for (i, sid) in leaves.iter().enumerate() {
            let rec = asm.record(sid).expect("章记录在盘");
            if i < 3 {
                std::fs::write(
                    drafts_dir.join(format!("{sid}.md")),
                    format!("# {sid}\n\n{}\n", rec.content),
                )
                .unwrap();
            }
            drafts_index.push(json!({
                "section_id": sid, "chars": rec.content.chars().count(),
                "citations": rec.citations,
            }));
        }
        write_json(
            &art.join("05_drafts/index.json"),
            &json!({ "chapters": drafts_index, "sample_md_kept": 3 }),
        );
        write_json(&art.join("07_quality.json"), &quality);
        write_json(&art.join("08_validation.json"), &validation);
        write_json(&art.join("09_export-meta.json"), &json!({
            "url": docx_url, "size": docx_size,
            "tables_written": export["tables_written"], "table_kinds": export["table_kinds"],
            "forms_written": export["forms_written"], "form_titles": export["form_titles"],
            "warnings": export["warnings"],
        }));
        std::fs::copy(&docx_url, art.join("06_export.docx")).expect("docx 拷贝");
        write_json(
            &art.join("10_gt-check.json"),
            &json!({
                "gt_file": gt_name, "gt_true": gt, "covered_L1": cov,
                "veto_cls_L2": l2, "veto_req": veto_req, "veto_tp": veto_tp,
                "l1_recall": if gt == 0 { 0.0 } else { cov as f64 / gt as f64 },
                "veto_precision": if veto_req == 0 { 0.0 } else { veto_tp as f64 / veto_req as f64 },
            }),
        );
        write_json(
            &art.join("11_export-check.json"),
            &json!({
                "tables_total": { "expected": 6, "actual": tables_actual },
                "form_titles_found": forms_found,
                "markdown_asterisk_leak": asterisk_actual,
                "docx_bytes": docx_size,
            }),
        );

        // ---------------- ⑩.5 本件报告
        let mut rep = String::new();
        rep.push_str(&format!("# 真实语料全链——{local}\n\n"));
        rep.push_str(&format!("- GT 对应：`{gt_name}`（sha256 一致）\n"));
        rep.push_str(&format!(
            "- 导入：{pages} 页 / {blocks_n} 行块｜REQ：**{n_reqs}** 条（否决 {}·资格 {}·评分 {}·技术参数 {}·格式 {}·商务 {}·其他 {}）\n",
            by_cat("否决项"), by_cat("资格条件"), by_cat("评分项"),
            by_cat("技术参数"), by_cat("格式要求"), by_cat("商务要求"), by_cat("其他"),
        ));
        rep.push_str(&format!(
            "- GT 红线：covered {cov}/{gt}（L1 {:.4}）｜否决类 TP {veto_tp}/{veto_req}（P {:.4}）\n",
            if gt == 0 { 0.0 } else { cov as f64 / gt as f64 },
            if veto_req == 0 { 0.0 } else { veto_tp as f64 / veto_req as f64 },
        ));
        rep.push_str(&format!(
            "- 大纲：{n_sec} 节点 / {} 叶｜mock 起草总字数 {total_words}（空章 {empty_drafts}）\n",
            leaves.len()
        ));
        rep.push_str(&format!(
            "- 导出：docx {docx_size} 字节｜<w:tbl> {tables_actual}/6｜格式件 {forms_found}/7｜星号残留 {asterisk_actual}\n\n"
        ));
        rep.push_str("## 耗时\n\n");
        rep.push_str(&format!(
            "- 导入 {t_import}ms｜抽取 {t_extract}ms｜大纲 {t_outline}ms｜证据 {t_evidence}ms｜起草 {t_draft}ms｜质量 {t_quality}ms｜校验 {t_validate}ms｜导出 {t_export}ms\n"
        ));
        std::fs::write(art.join("report.md"), rep).unwrap();

        per_file.push(json!({
            "file": local, "gt_file": gt_name, "pages": pages, "blocks": blocks_n,
            "reqs": n_reqs,
            "by_category": {
                "否决项": by_cat("否决项"), "资格条件": by_cat("资格条件"),
                "评分项": by_cat("评分项"), "技术参数": by_cat("技术参数"),
                "其他": by_cat("其他"),
            },
            "gt_true": gt, "covered_L1": cov, "veto_cls_L2": l2,
            "veto_req": veto_req, "veto_tp": veto_tp,
            "sections": n_sec, "leaves": leaves.len(),
            "total_words": total_words, "empty_drafts": empty_drafts,
            "docx_bytes": docx_size, "tables_in_xml": tables_actual,
            "tables_expected": tables_expected, "tech_req_count": tech_req_count,
            "table_kinds": export["table_kinds"],
            "forms_found": forms_found, "asterisk_leak": asterisk_actual,
            "timings_ms": {
                "import": t_import, "extract": t_extract, "outline": t_outline,
                "evidence": t_evidence, "draft": t_draft, "quality": t_quality,
                "validate": t_validate, "export": t_export,
            },
        }));

        let _ = std::fs::remove_dir_all(&ws);
    }

    // ================================ 汇总断言（issue #131 checklist） ================================
    // ---- 前置：解析保真度（决定 L1 红线是否可比）
    //
    // L1 的口径是【GT 逐块对照】：anchor 的 locator/text 必须与解析产出的
    // Block 精确相等才算命中。而 Block 的切分粒度**依赖 PDF 文本层提取器**：
    //
    // ```
    // pdfium（引擎通道）    → 按行切分，GT 就是按它标的
    // poppler（pdftotext）  → 切分不同（CI 实测 REQ 2624 vs 本地 ~900）
    // 纯 Rust 兜底          → 又一种粒度
    // ```
    //
    // 三者互相不可比。若当前环境的解析产物与 GT 标注粒度不符，L1 分母
    // （318 条 GT）仍成立但分子永远对不上——这不是代码退化，是**环境换了解析器**。
    //
    // 判据用【REQ 数量级】而非工具存在性（#156 的 has_pdftotext 判据已证伪：
    // CI 上 poppler 是有的，SKIP 横幅一次没打印，测试照跑照崩）。
    let total_reqs: u64 = per_file.iter().map(|f| f["reqs"].as_u64().unwrap_or(0)).sum();
    const REQ_PLAUSIBLE_MAX: u64 = 1500;
    if total_reqs > REQ_PLAUSIBLE_MAX {
        eprintln!(
            "SKIP: real_corpus_full_chain —— 解析器与 GT 标注不同源\n\
             \n\
             本次解析产出 {total_reqs} 条 REQ，超出 pdfium 实测区间（<{REQ_PLAUSIBLE_MAX}）。\n\
             GT 是按【引擎 pdfium】的块切分标注的；换 poppler/纯 Rust 兜底时\n\
             块粒度不同 → 逐块对齐恒 false → L1 指标不可比（非代码退化）。\n\
             \n\
             这是环境解析器差异，不是业务失败（issue #9 SKIP 纪律）。"
        );
        return;
    }

    // ---- 红线：L1 / P 不退化（#123 基线 309/318、291/302——逐位口径，浮点容差）
    let l1 = if tot_gt == 0 { 0.0 } else { tot_cov as f64 / tot_gt as f64 };
    let p = if tot_veto_req == 0 {
        0.0
    } else {
        tot_veto_tp as f64 / tot_veto_req as f64
    };
    assert!(
        l1 >= L1_BASELINE - 1e-9,
        "红线退化：L1 召回 {l1:.6} < 基线 {L1_BASELINE:.6}（covered {tot_cov}/{tot_gt}，#123 基线 309/318）"
    );
    assert!(
        p >= P_BASELINE - 1e-9,
        "红线退化：否决类精确率 {p:.6} < 基线 {P_BASELINE:.6}（TP {tot_veto_tp}/{tot_veto_req}，#123 基线 291/302）"
    );

    // ---- 抽取规模合理（对照 synth 115 REQ——真实件应为数百级）
    for f in &per_file {
        let n = f["reqs"].as_u64().unwrap();
        assert!(
            n >= 115,
            "抽取规模：{} 仅 {n} 条（synth 合成件都有 115——真实 76-284 页件应数百级）",
            f["file"].as_str().unwrap()
        );
    }

    // ---- 大纲结构合理（不得退化成「其他（第N块）」平铺）
    for f in &per_file {
        let file = f["file"].as_str().unwrap();
        let stem = file.trim_end_matches(".pdf");
        let outline: Value = serde_json::from_str(
            &std::fs::read_to_string(latest.join(stem).join("04_outline.json")).unwrap(),
        )
        .unwrap();
        let nodes = outline["sections"].as_array().unwrap();
        assert!(
            nodes.iter().all(|n| {
                !n["title"].as_str().unwrap_or("").starts_with("其他（")
            }),
            "{file} 大纲出现「其他（第N块）」平铺退化（#100 已修的形态回归）"
        );
        // 响应组在场（真实件必有否决/资格条款——它们的响应去向）
        assert!(
            nodes.iter().any(|n| n["title"]
                .as_str()
                .unwrap_or("")
                .contains("资格与响应")),
            "{file} 大纲缺「资格与响应」组（否决/资格条款无响应章节→BLOCKER 面）"
        );
        // 叶标题多样性：去「（第N块）」后缀后的不同组名 ≥ 3（真实件的类目+章节归组多样性）
        let groups: std::collections::HashSet<String> = nodes
            .iter()
            .filter_map(|n| {
                let t = n["title"].as_str().unwrap_or("");
                let t = t.split("（第").next().unwrap_or(t);
                let lvl = n["level"].as_u64().unwrap_or(0);
                if lvl <= 1 {
                    Some(t.to_string())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            groups.len() >= 3,
            "{file} 大纲根组仅 {} 个（真实件类目多样性不足: {groups:?}）",
            groups.len()
        );
    }

    // ---- 起草产出（不是全空）
    for f in &per_file {
        let file = f["file"].as_str().unwrap();
        let words = f["total_words"].as_u64().unwrap();
        assert!(words >= 1_000, "{file} mock 起草总字数 {words}（全空/近空）");
        assert_eq!(
            f["empty_drafts"].as_u64().unwrap(),
            0,
            "{file} 存在 0 字空章（起草面退化）"
        );
    }

    // ---- 导出：应生成的表全部生成 + 7 格式件（真实件下的接线重放）
    //      表格数 = 5（概况/报价/业绩/人员/设备——本链真源全部在场）
    //      + 技术参数偏离表（仅当技术参数 REQ > 0；三件真实水利件均 0——
    //      缺席告警已在循环内逐件验证在场）
    for f in &per_file {
        let file = f["file"].as_str().unwrap();
        let expected = f["tables_expected"].as_u64().unwrap();
        let actual = f["tables_in_xml"].as_u64().unwrap();
        assert_eq!(
            actual, expected,
            "{file} docx <w:tbl> 数 {actual} ≠ 应生成 {expected}（技术参数 REQ {} 条，表种类 {:?}）",
            f["tech_req_count"].as_u64().unwrap(),
            f["table_kinds"]
        );
        // 五张真源驱动的表必须逐张在座（不救缺席——技术参数面除外）
        for kind in [
            "项目概况表",
            "报价明细表",
            "业绩一览表",
            "人员配置表",
            "设备清单表",
        ] {
            assert!(
                f["table_kinds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|k| k == kind),
                "{file} 缺表 {kind}（真源在场而表缺席——接线前半段断）: {:?}",
                f["table_kinds"]
            );
        }
        assert_eq!(
            f["forms_found"].as_u64().unwrap(),
            7,
            "{file} 格式件关键词 7/7 未命中（七件接线在真实件上断了）"
        );
        assert_eq!(
            f["asterisk_leak"].as_u64().unwrap(),
            0,
            "{file} markdown 星号字面量泄漏进 docx"
        );
    }

    // ---- 汇总产物
    let summary = json!({
        "corpus": per_file,
        "redline": {
            "L1": { "baseline": L1_BASELINE, "actual": l1,
                    "covered": tot_cov, "gt_true_total": tot_gt },
            "precision_veto_cls": { "baseline": P_BASELINE, "actual": p,
                                    "veto_tp": tot_veto_tp, "veto_req": tot_veto_req },
            "L2": { "veto_cls": tot_l2,
                    "recall": if tot_gt == 0 { 0.0 } else { tot_l2 as f64 / tot_gt as f64 } },
        },
        "checklist": {
            "import_ok_files": 3,
            "reqs_per_file_min": per_file.iter().map(|f| f["reqs"].as_u64().unwrap()).min(),
            "outline_no_other_flat": true,
            "draft_words_total": per_file.iter().map(|f| f["total_words"].as_u64().unwrap()).sum::<u64>(),
            "docx_tables_expected_all_written": per_file
                .iter()
                .all(|f| f["tables_in_xml"] == f["tables_expected"]),
            "tech_deviation_table_absent_reason": "三件水利标书技术参数类 REQ = 0（确定性型号正则场景局限，real-eval §3 已登记）——缺席告警在场，非静默",
            "docx_forms_7_of_7": per_file.iter().all(|f| f["forms_found"] == 7),
        },
    });
    write_json(&latest.join("summary.json"), &summary);
    let mut rep = String::new();
    rep.push_str("# 真实语料全链验证（#131 任务 1）\n\n");
    rep.push_str("3 件真实水科 PDF（sha256 与 GT A-01/A-04/A-05 一致），每件独立项目跑全链。\n\n");
    rep.push_str(&format!(
        "## 红线（#123 基线，不退化）\n\n- L1 抽取召回：**{l1:.4}**（基线 {:.4}，covered {tot_cov}/{tot_gt}）\n- 否决类精确率：**{p:.4}**（基线 {:.4}，TP {tot_veto_tp}/{tot_veto_req}）\n- L2 否决分类召回：{:.4}（{tot_l2}/{tot_gt}）\n\n",
        L1_BASELINE, P_BASELINE,
        if tot_gt == 0 { 0.0 } else { tot_l2 as f64 / tot_gt as f64 },
    ));
    rep.push_str("## 每件数据\n\n| 件 | 页/块 | REQ | 大纲节/叶 | 字数 | docx | 表(实/应) | 格式件 |\n|---|---|---|---|---|---|---|---|\n");
    for f in &per_file {
        rep.push_str(&format!(
            "| {} | {}/{} | {} | {}/{} | {} | {}B | {}/{} | {}/7 |\n",
            f["file"].as_str().unwrap(),
            f["pages"].as_u64().unwrap(),
            f["blocks"].as_u64().unwrap(),
            f["reqs"].as_u64().unwrap(),
            f["sections"].as_u64().unwrap(),
            f["leaves"].as_u64().unwrap(),
            f["total_words"].as_u64().unwrap(),
            f["docx_bytes"].as_u64().unwrap(),
            f["tables_in_xml"].as_u64().unwrap(),
            f["tables_expected"].as_u64().unwrap(),
            f["forms_found"].as_u64().unwrap(),
        ));
    }
    rep.push_str("\n技术参数偏离表缺席说明：三件水利标书的技术参数类 REQ 均为 0（水利文件无 IT 型号谱系——确定性分类器场景局限，real-eval §3 已登记，归 M2 模型辅助归类）。缺席以响亮告警回报（非静默），其余五表（概况/报价/业绩/人员/设备）全部在座。\n");
    std::fs::write(latest.join("report.md"), rep).unwrap();

    println!("==================== real-corpus-e2e 摘要 ====================");
    println!("L1 {l1:.4}（基线 {L1_BASELINE:.4}）｜P {p:.4}（基线 {P_BASELINE:.4}）");
    println!("产物: {}", latest.display());
}
