//! #100 任务 3 · score_price / score_section / score_document 接线验收。
//!
//! 验收面（issue #100 §3.1/§3.2/§3.3）：
//! - **score_price**：价格分试算 + 最优报价反推（「该报多少价」）——基准价法
//!   issue 示例口径 + 平均价法与 B1 引擎双跑对账 + 参数校验响亮拒；
//! - **score_section**：章节自评（「这一章能得几分」）——起草后可评、
//!   未起草/不存在响亮拒、价格类叶跳过内容自评；
//! - **score_document**：整本自评（「这份标书能得几分」）——没报价 →
//!   price=null 不伪装 0；报价在场 → B1 实算计入 total；target/gap/by_item
//!   逐项明细（大纲联结：params.section 缺省时按 REQ 所在叶补映射）；
//! - 状态门：score_section/score_document 走 guard_tool（MATRIX_READY+）；
//!   score_price 纯计算器无门（无项目也能算）。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
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

fn err_parts(reply: &ToolReply) -> (String, String) {
    match reply {
        ToolReply::Err { error, .. } => (error.code().to_string(), error.to_string()),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// REQ 夹具（评分项带 score 客观位；否决项；技术参数）。
fn mk_req(id: &str, category: &str, excerpt: &str, score: Option<f64>) -> bid_domain::Requirement {
    use bid_domain::{Anchor, Requirement};
    let mut params = std::collections::HashMap::new();
    if let Some(s) = score {
        params.insert("score".to_string(), serde_json::json!(s));
    }
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第四章 评标办法".into(),
            excerpt: excerpt.into(),
        }],
        params,
        source_file: "A-招标文件.docx".into(),
    }
}

/// 「建纲 + mock 起草全部叶」服务态（synth 形态：1 价格评分项 + 1 技术评分项
/// + 1 否决项 + 1 技术参数——**评分项 params 不带 section**：
/// 因为 #126 后它们不进正文，也就不会落进任何叶的 requirement_ids——
/// `build_scorer` 的 leaf_of 反查永不命中，section_id 恒 None（合法行为）。
fn svc_drafted(tag: &str, draft_all: bool, chapter_ref: Option<&str>) -> (BidService, String) {
    let mut svc = BidService::new();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "评分接线测试项目", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-score", 10, 1, 50, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    let mut req2 = mk_req("REQ-0002", "评分项",
        "4.2 技术分：技术方案完整性与针对性 20 分，方案须覆盖系统架构与设备选型",
        Some(20.0));
    if let Some(title) = chapter_ref {
        // #170 方案 A：评分项 params.section_title（bid-parse 从评审表行文本
        // 登记「第X章」引用——此处直接模拟登记产物测联结）
        req2.params.insert("section_title".to_string(), serde_json::json!(title));
    }
    svc.set_requirements(vec![
        mk_req("REQ-0001", "评分项",
            "4.1 价格分（30 分）：以通过初步评审的有效投标报价的算术平均值下浮 5% 为基准价，价格分＝（基准价／投标报价）×30",
            Some(30.0)),
        req2,
        mk_req("REQ-0003", "否决项",
            "3.9 投标文件未按规定格式签署盖章的，作否决投标处理", None),
        mk_req("REQ-0004", "技术参数",
            "5.1 核心交换机须支持 48 口千兆光接口，交换容量 ≥ 2.4Tbps", None),
    ]).unwrap();
    let (n_sec, _, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 6_000, max_reqs_per_leaf: 8 })
        .unwrap();
    assert!(n_sec >= 1);
    // 素材面（#110 任务 1 缺失回报闸）：否决项 REQ-0003 须矩阵响应、技术参数
    // REQ-0004 须证据佐证——登记 + 绑定 + 重建矩阵，否则 draft 拒生成
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: "贰级资质证书".into(),
        source_file: "EV-score.pdf".into(),
        source_sha256: "sha-score-ev".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 2, text_fingerprint: "fnv64:sc".into() },
        fingerprint: "fnv64:sc".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某公司".into(),
        scope: "电子与智能化工程".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(), // #147
    }).unwrap();
    for rid in ["REQ-0003", "REQ-0004"] {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    // L3 无补遗阻塞的人工出口（#131）：基座只导入招标文件（无补遗件），
    // #118 合入时漏适配本套件——AddendumWaiver 门确认无补遗后起草放行。
    use bid_domain::review::{Decision, Gate};
    svc.store_mut()
        .record_review(Gate::AddendumWaiver, &bid_domain::TrustedPrincipal::human("score-tool-test"), Decision::Approved, vec![])
        .unwrap();
    if draft_all {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        let leaves: Vec<String> = svc.store().sections().iter()
            .filter(|n| !parents.contains(&n.id))
            .map(|n| n.id.clone()).collect();
        assert!(!leaves.is_empty());
        for leaf in &leaves {
            let r = ok_result(call(&mut svc, "draft_response_section", json!({
                "project_id": &pid, "section_id": leaf, "generator": "mock",
            })));
            assert_eq!(r["section_status"], "DRAFTED");
        }
    }
    (svc, pid)
}

/// 叶章节 id 清单（已起草面断言用）。
fn leaf_ids(svc: &BidService) -> Vec<String> {
    let parents: std::collections::HashSet<String> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.clone()).collect();
    svc.store().sections().iter()
        .filter(|n| !parents.contains(&n.id))
        .map(|n| n.id.clone()).collect()
}

// ---------------------------------------------------------------- score_price

/// 基准价法（issue #100 §3.3 示例口径）：控制价 1650 万 × 0.95 = 最优 1567.5 万
/// ——命中基准价得满分；偏离侧试算同行。
#[test]
fn score_price_benchmark_issue_example() {
    let mut svc = BidService::new();
    let r = ok_result(call(&mut svc, "score_price", json!({
        "method": "benchmark", "coefficient": 0.95, "budget": 16_500_000.0, "max_score": 40.0,
        "bid_price": 15_675_000.0,
        "others": [16_000_000.0, 15_800_000.0],
    })));
    // 纯计算器：无项目也能算（无路由/状态门）
    assert_eq!(r["optimal_price"].as_f64().unwrap(), 15_675_000.0, "最优 = 控制价 × 0.95");
    assert_eq!(r["expected_score_at_optimal"].as_f64().unwrap(), 40.0, "命中基准价得满分");
    assert_eq!(r["score"].as_f64().unwrap(), 40.0, "bid 恰为基准价 → 满分");
    let d = &r["detail"];
    assert_eq!(d["benchmark"].as_f64().unwrap(), 15_675_000.0);
    assert!(d["deviation"].as_f64().unwrap().abs() < 1e-9, "偏离为零");
    // issue §3.3 的四件面：budget / recommended / expected / max（字段名对齐 out）
    assert_eq!(r["spec"]["budget"].as_f64().unwrap(), 16_500_000.0);
    assert_eq!(r["spec"]["max_score"].as_f64().unwrap(), 40.0);
    assert!(r["analysis"].as_str().unwrap().contains("基准价法"), "人读分析在场");
}

/// 平均价法（synth 4.1 口径）：与 B1 引擎双跑对账（工具壳不得改变引擎语义）。
#[test]
fn score_price_average_matches_b1_engine() {
    use bid_domain::pricing::{DeviationRule, PriceScoreMethod, PriceScoreSpec};
    let spec = PriceScoreSpec {
        method: PriceScoreMethod::AveragePrice { coefficient: 0.95 },
        max_score: 30.0,
        budget: None,
        deviation_rule: DeviationRule::Ratio,
    };
    let others = [1_600_000.0, 1_580_000.0, 1_560_000.0];
    let bid = 1_550_000.0;
    let expect_score = spec.score(bid, &others).unwrap();
    let expect_opt = spec.optimal_price(&others).unwrap();

    let mut svc = BidService::new();
    let r = ok_result(call(&mut svc, "score_price", json!({
        "method": "average", "coefficient": 0.95, "max_score": 30, "deviation": "ratio",
        "bid_price": bid, "others": others,
    })));
    assert!((r["score"].as_f64().unwrap() - expect_score).abs() < 1e-9, "试算分与引擎一致");
    assert!((r["optimal_price"].as_f64().unwrap() - expect_opt).abs() < 1e-9, "最优价与引擎闭式解一致");
    assert!((r["expected_score_at_optimal"].as_f64().unwrap() - 30.0).abs() < 1e-9, "最优价命中基准 → 满分");
}

/// 参数校验响亮拒：未知 method / benchmark 缺控制价 / deviation=price 缺扣分参数。
#[test]
fn score_price_param_validation_is_loud() {
    let mut svc = BidService::new();
    let (code, msg) = err_parts(&call(&mut svc, "score_price", json!({
        "method": "gpt4",
    })));
    assert_eq!(code, "E_UNPROCESSABLE");
    assert!(msg.contains("benchmark | lowest | average"), "{msg}");

    // benchmark 缺 budget → B1 validate 拒（控制价是该方法的必需参数）
    let (code, msg) = err_parts(&call(&mut svc, "score_price", json!({
        "method": "benchmark", "bid_price": 100.0,
    })));
    assert_eq!(code, "E_UNPROCESSABLE", "{msg}");

    // deviation=price 缺 deduct_high → 壳层响亮拒（提示缺什么）
    let (code, msg) = err_parts(&call(&mut svc, "score_price", json!({
        "method": "average", "deviation": "price", "bid_price": 100.0, "others": [98.0],
    })));
    assert_eq!(code, "E_UNPROCESSABLE");
    assert!(msg.contains("deduct_high"), "{msg}");

    // deviation=price 全参 → 通过（含 deduct 双侧）
    let r = ok_result(call(&mut svc, "score_price", json!({
        "method": "lowest", "deviation": "price", "deduct_high": 0.2, "deduct_low": 0.1,
        "max_score": 44, "bid_price": 100.0, "others": [100.0, 102.0],
    })));
    assert!(r["score"].as_f64().is_some(), "最低价基准 + 价格扣分口径可用");
}

// ---------------------------------------------------------------- score_section

/// 章节自评主链：起草后可评（价格类叶跳过内容自评 + 技术项四维判定 +
/// 形状面 + 已起草前置（#126 后评分项不进正文，item_scores 可为空）。
#[test]
fn score_section_on_drafted_leaf() {
    let (mut svc, pid) = svc_drafted("score-sec-main", true, None);
    // 【#126 语义变更后的口径】评分项**不进正文**（outline.rs:324——评分方法是
    // 评标委员会的算分规则，正文写「价格分 30 分」等于把报价策略和评标办法混谈）。
    //
    // 所以 score_section 的入参叶**不可能**是「评分项 REQ 所在叶」——那叶不存在。
    // 改用【任意已起草叶】：score_section 的自评口径是「这一节正文能拿几分」，
    // （#126 后「大纲联结」对评分项已失效——评分项不属于任何正文节。）
    let target_leaf = leaf_ids(&svc).first().cloned()
        .expect("至少一个叶（svc_drafted 已建纲）");
    let r = ok_result(call(&mut svc, "score_section", json!({
        "project_id": &pid, "section_id": &target_leaf,
    })));
    assert_eq!(r["section_id"], target_leaf.as_str());
    // #186 语义降级标记：规则诊断非评标模拟——两个标记必须恒在场
    assert_eq!(r["diagnostic_only"], json!(true), "diagnostic_only=true: {r:?}");
    assert_eq!(r["result_kind"], "rule_heuristic", "result_kind=rule_heuristic: {r:?}");
    // 形状面：max / estimated / item_scores / rationale 都在场
    assert!(r["max"].as_f64().is_some(), "max 在场（数值）: {r:?}");
    assert!(r["estimated"].as_f64().is_some(), "estimated 在场: {r:?}");
    let items = r["item_scores"].as_array().unwrap();
    // 【#126 后】评分项不进正文 → leaf_of 反查不命中；本夹具的评分项
    // 无 params.section_title（评审表行无「第X章」引用）→ 章引用联结也
    // 不触发 → item_scores **合法为空**。只钉形状（是数组）；章引用映射
    // 的语义验收见 score_section_maps_scoring_item_via_chapter_ref。
    assert!(r["item_scores"].is_array(), "item_scores 是数组: {r:?}");
    // 可追溯理由必须非空。两条合法文案：
    //   · 有映射项 → "[REQ-xxxx] 覆盖 h/H=xx% ..."
    //   · 无映射项（#126 后评分项不进正文 → 本节无映射）→ "无映射到本章节的评分项..."
    // 只钉「非空且可读」，不钉具体措辞（措辞演进不该红测试）。
    let rationale = r["rationale"].as_str().unwrap_or("");
    assert!(!rationale.trim().is_empty(), "可追溯理由非空: {r:?}");
    assert!(
        rationale.contains("覆盖") || rationale.contains("无映射"),
        "理由要么是覆盖度明细、要么是「无映射」显式说明: {rationale}"
    );
    // 逐项明细的 item_id 非空（映射面完整性）
    assert!(items.iter().all(|i| !i["item_id"].as_str().unwrap().is_empty()));
}

/// #170 方案 A 验收：评分项的 params.section_title（评审表行「第X章」引用）
/// → 章号归一匹配大纲「第四章 评标办法」组 → 子树首叶挂载 →
/// score_section(首叶) 恢复「这一节能得几分」（max=20，item_scores 含 REQ-0002）。
#[test]
fn score_section_maps_scoring_item_via_chapter_ref() {
    let (mut svc, pid) = svc_drafted("score-sec-chref", true, Some("第四章 评标办法"));
    // 章引用联结的语义目标：锚在「第四章 评标办法」的正文 REQ（REQ-0004
    // 技术参数）所住的叶——「详见第四章」= 第四章的内容 = 该叶正文。
    // （大纲是类目重组——「技术参数」组横跨物理章，故目标不是「第四章」
    // 命名的组，而是该章正文条款落的叶。）
    let target_leaf = {
        let secs = svc.store().sections();
        let parents: std::collections::HashSet<&str> = secs
            .iter()
            .filter_map(|n| n.parent_id.as_deref())
            .collect();
        let req4 = svc
            .store()
            .requirements()
            .iter()
            .find(|r| r.id == "REQ-0004")
            .expect("REQ-0004 在库");
        assert!(
            req4.anchors.iter().any(|a| a.locator.contains("第四章")),
            "REQ-0004 锚应在第四章（夹具 locator）"
        );
        secs.iter()
            .find(|n| {
                !parents.contains(n.id.as_str()) && n.requirement_ids.contains(&req4.id)
            })
            .map(|n| n.id.clone())
            .expect("REQ-0004 应住某片叶（正文类目）")
    };
    let r = ok_result(call(&mut svc, "score_section", json!({
        "project_id": &pid, "section_id": &target_leaf,
    })));
    // 语义断言（区别于形状面）：REQ-0002（20 分技术评分项）挂上了本叶
    assert_eq!(r["max"].as_f64(), Some(20.0), "本叶 max = 挂载评分项满分 20：{r:?}");
    let items = r["item_scores"].as_array().expect("item_scores 数组");
    assert!(
        items.iter().any(|i| i["item_id"].as_str() == Some("REQ-0002")),
        "item_scores 应含章引用挂载的 REQ-0002：{items:?}"
    );
    let rationale = r["rationale"].as_str().unwrap_or("");
    assert!(rationale.contains("覆盖"), "有映射项 → 覆盖度明细：{rationale}");
}

/// score_section 拒绝面：未起草（E_UNPROCESSABLE）/ 不存在（E_NOTFOUND）/
/// 建纲前状态门（E_STATE）。
#[test]
fn score_section_rejects_are_loud() {
    let (mut svc, pid) = svc_drafted("score-sec-rej", false, None);
    // 未起草：建纲但没起草任何叶
    let leaf = leaf_ids(&svc)[0].clone();
    let (code, msg) = err_parts(&call(&mut svc, "score_section", json!({
        "project_id": &pid, "section_id": &leaf,
    })));
    assert_eq!(code, "E_UNPROCESSABLE");
    assert!(msg.contains("未起草"), "{msg}");
    // 不存在
    let (code, _msg) = err_parts(&call(&mut svc, "score_section", json!({
        "project_id": &pid, "section_id": "SEC-9999",
    })));
    assert_eq!(code, "E_NOTFOUND");
    // 建纲前（REQUIREMENTS_EXTRACTED 前置不足）——直接建新项目到 FILES_IMPORTED
    let mut fresh = BidService::new();
    let pid2 = ok_result(call(&mut fresh, "create_bid_project", json!({
        "name": "无纲项目", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "score-sec-nogate",
    })))["project_id"].as_str().unwrap().to_string();
    let (code, _msg) = err_parts(&call(&mut fresh, "score_section", json!({
        "project_id": &pid2, "section_id": "SEC-0001",
    })));
    assert_eq!(code, "E_STATE", "FILES_IMPORTED 无章节树——状态门拦");
}

// ---------------------------------------------------------------- score_document

/// 整本自评（issue §3.2 形状）：没报价 → price=null；target/gap/by_item 在场；
/// 报价在场 → B1 实算计入 total（「能得几分 + 该报多少价」一屏可答）。
#[test]
fn score_document_shapes_and_price_semantics() {
    let (mut svc, pid) = svc_drafted("score-doc-main", true, None);
    // ---- 无报价：price null（不伪装 0）
    let r = ok_result(call(&mut svc, "score_document", json!({
        "project_id": &pid,
    })));
    // #186 语义降级标记：规则诊断非评标模拟——两个标记必须恒在场
    assert_eq!(r["diagnostic_only"], json!(true), "diagnostic_only=true: {r:?}");
    assert_eq!(r["result_kind"], "rule_heuristic", "result_kind=rule_heuristic: {r:?}");
    // 【#126 语义变更后的口径】技术项 REQ-0002 是**评分项**，不进正文
    // （outline.rs:324）——它的 estimated 来自「章节正文覆盖度 × 该项分值」，
    // 而覆盖度取决于**本章节的正文**（否决项/技术参数叶的稿）。
    // 所以 technical 可能是 0（本章无评分项映射的正文覆盖）或 >0。
    // 断言改为【形状面】：technical 是数值且 >= 0（不要求 >0）。
    assert!(r["technical"].as_f64().is_some_and(|v| v >= 0.0), "技术分是数值且非负: {r:?}");
    assert!(r["price"].is_null(), "没报价就不算——price null");
    let notes = r["notes"].as_array().unwrap();
    assert!(notes.iter().any(|n| n.as_str().unwrap_or("").contains("NO_PRICE_INPUT")), "{notes:?}");
    // target = 诚实满分（30 价格 + 20 技术 = 50——不因未报价缩分母）
    assert_eq!(r["target"].as_f64().unwrap(), 50.0);
    assert_eq!(r["gap"].as_f64().unwrap(), 50.0 - r["technical"].as_f64().unwrap());
    let by_item = r["by_item"].as_array().unwrap();
    assert_eq!(by_item.len(), 2, "价格项 + 技术项（否决项不计分不入列）");
    let price_item = by_item.iter().find(|i| i["item_id"] == "REQ-0001").unwrap();
    assert!(price_item["tier"].as_str().unwrap().contains("价格分"), "价格项标记 f(报价)");
    assert_eq!(price_item["estimated"].as_f64().unwrap(), 0.0, "未报价 → 价格项估 0（gap 承担）");
    let tech_item = by_item.iter().find(|i| i["item_id"] == "REQ-0002").unwrap();
    // 评分项不进正文 → estimated 可为 0（覆盖度按本章正文算）
    assert!(tech_item["estimated"].as_f64().is_some_and(|v| v >= 0.0), "技术项估分非负");
    // 【#126 后】评分项进不了叶 → leaf_of 反查落空 → section_id 为 None（合法）。
    // 「大纲联结」这条路径对评分项已失效（#126 的必然结果，非缺陷）。

    // ---- 有报价（synth 4.1：平均价法 0.95、满分 30、比率）：价格计入 total
    let r2 = ok_result(call(&mut svc, "score_document", json!({
        "project_id": &pid,
        "bid_price": 1_529_000.0,
        "others": [1_600_000.0, 1_580_000.0, 1_560_000.0],
        "method": "average", "coefficient": 0.95, "max_score": 30, "deviation": "ratio",
    })));
    let price = r2["price"].as_f64().expect("报价在场 → price 数值");
    assert!(price > 0.0 && price <= 30.0, "价格分 ∈ (0, 30]");
    let total = r2["total"].as_f64().unwrap();
    let tech = r2["technical"].as_f64().unwrap();
    assert!((total - tech - price).abs() < 0.01, "total = technical + price（+commercial 若有）");
    assert!(r2["price_input"]["max_score"].as_f64().is_some(), "价格口径 echo 在场");
    assert_eq!(r2["sections_scored"].as_u64().unwrap(), leaf_ids(&svc).len() as u64, "全部叶参评");
}

/// score_document 状态门与空面：无章节树 → E_STATE；建纲未起草 → 诚实零分
/// （notes 登记 SECTION_MISSING/未覆盖，不伪装）。
#[test]
fn score_document_state_gate_and_undrafted_honesty() {
    let (mut svc, pid) = svc_drafted("score-doc-undrafted", false, None);
    let r = ok_result(call(&mut svc, "score_document", json!({
        "project_id": &pid,
    })));
    assert_eq!(r["technical"].as_f64().unwrap(), 0.0, "未起草 → 技术分 0（诚实）");
    assert_eq!(r["sections_scored"].as_u64().unwrap(), 0);
    assert_eq!(r["target"].as_f64().unwrap(), 50.0, "满分照实——不因没写缩分母");
    // 无章节树（新项目）
    let mut fresh = BidService::new();
    let pid2 = ok_result(call(&mut fresh, "create_bid_project", json!({
        "name": "无纲项目2", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "score-doc-nogate",
    })))["project_id"].as_str().unwrap().to_string();
    let (code, _msg) = err_parts(&call(&mut fresh, "score_document", json!({
        "project_id": &pid2,
    })));
    assert_eq!(code, "E_STATE", "FILES_IMPORTED 无章节树——状态门拦");
}
