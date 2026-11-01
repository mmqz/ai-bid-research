//! #59 阶段 3 · validate_bid_document 测试（8 条确定性规则 + scope + 契约形状）。
//!
//! Lead 验收面：防废标最后一道关——否决项响应 / 资格满足 / 必填章节 /
//! 字数达标 / 格式核对 + 契约 §7 输出形状（findings + counts + ok）。

use bid_domain::evidence::EvidenceSpec;
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineSpec;
use bid_domain::{Anchor, ParseStatus, ProjectStatus, Requirement};
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

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn mk_req(id: &str, category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第二章/1.1".into(),
            excerpt: excerpt.into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-validate-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 夹具变体（#126 适配）：svc_base + 一条「格式要求」REQ——资格/否决合并叶
/// 之外再出一个可零绑定起草的格式叶，让「未响应面由 validate 报 BLOCKER」
/// 的前提（进 DRAFTING 后才准校验）重新成立。validate_contract_shape /
/// rule_veto_and_qual 专用（其余用 svc_base 的测试断言面不受 REQ 集变化影响）。
fn svc_base_grouped(tag: &str) -> (BidService, String) {
    // 与 svc_base 同构，仅 REQ 集追加 REQ-0004 格式要求（svc_base 已推
    // MATRIX_READY 不能改条款集，故独立重建而非复用）。
    let mut svc = BidService::open(tmp_dir(&format!("{tag}-grp"))).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "校验测试", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": format!("{tag}-grp"),
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", &format!("sha-v-{tag}-grp"), 10, 1, 50, 0, ParseStatus::Ok, "",
    ).unwrap();
    svc.store_mut().upsert_file(
        "C-补遗文件01.docx", "补遗", &format!("sha-v-add-{tag}-grp"), 5, 0, 20, 0, ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "具备水利水电工程施工总承包贰级资质"),
        mk_req("REQ-0002", "资格条件", "项目经理具备一级建造师注册证书"),
        mk_req("REQ-0003", "否决项", "投标文件未经法定代表人签字作否决处理"),
        mk_req("REQ-0004", "格式要求", "正文用仿宋三号字，行距为 1.5 倍"),
    ]).unwrap();
    let _ = svc.prepare_outline_and_matrix(
        OutlineSpec { target_total_chars: 2_000, max_reqs_per_leaf: 8 },
    ).unwrap();
    (svc, pid)
}

/// 基础夹具：extract → outline → matrix（DRAFTING 前状态——MATRIX_READY）。
/// REQ 构成：资格 2 + 否决 1。
///
/// #131 适配：基座补一件补遗件（draft_tool.rs 同款）——否则 L3「无补遗阻塞」
/// readiness 闸（MissingAddendum）会拦下后续所有 draft 调用，本套件的校验
/// 规则面测试根本走不到（#131 合入时漏适配，13 例连锁红）。
fn svc_base(tag: &str) -> (BidService, String) {
    let mut svc = BidService::open(tmp_dir(tag)).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "校验测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", &format!("sha-v-{tag}"), 10, 1, 50, 0, ParseStatus::Ok, "",
    ).unwrap();
    svc.store_mut().upsert_file(
        "C-补遗文件01.docx", "补遗", &format!("sha-v-add-{tag}"), 5, 0, 20, 0, ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "具备水利水电工程施工总承包贰级资质"),
        mk_req("REQ-0002", "资格条件", "项目经理具备一级建造师注册证书"),
        mk_req("REQ-0003", "否决项", "投标文件未经法定代表人签字作否决处理"),
    ]).unwrap();
    let _ = svc.prepare_outline_and_matrix(
        OutlineSpec { target_total_chars: 2_000, max_reqs_per_leaf: 8 },
    ).unwrap();
    (svc, pid)
}

fn leaf_ids(svc: &BidService) -> Vec<String> {
    let parents: std::collections::HashSet<String> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.clone()).collect();
    svc.store().sections().iter()
        .filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect()
}

fn bind_evidence_for(svc: &mut BidService, req_ids: &[&str]) {
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "贰级资质证书".into(),
        source_file: "EV.pdf".into(),
        source_sha256: format!("sha-{}", req_ids.len()),
        locator: EngineLocator::DocxParagraph { paragraph_index: 1, text_fingerprint: "fnv64:aa".into() },
        fingerprint: "fnv64:aa".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某公司".into(),
        scope: "水利水电".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(), // #147
    }).unwrap();
    for rid in req_ids {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
}

/// 台账面登记一条可用资质（**不绑定**——资格 REQ 的未响应面留给 validate
/// 规则去报；只为清「资格条件」叶标题词面的 MissingQualification 闸）。
/// #110 任务 1 后，「零绑定零覆写」的夹具只有配合它才能走到 DRAFTING。
fn register_qual_ledger_only(svc: &mut BidService) {
    let _ = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "水利水电工程施工总承包贰级资质证书".into(),
        source_file: "EV-ledger.pdf".into(),
        source_sha256: "sha-ledger".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 7, text_fingerprint: "fnv64:lg".into() },
        fingerprint: "fnv64:lg".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某公司".into(),
        scope: "水利水电".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(), // #147
    }).unwrap();
}

/// 只起草就绪叶（缺失回报闸语义——#110 任务 1）：零绑定零覆写时，含未响应
/// 否决项的叶拒生成（E_UNPROCESSABLE，闸特性不是回归），保持 Pending——
/// 「已起草正文 + 否决项未响应」这个废标态从此不可达；未响应面由 validate
/// 从矩阵报 BLOCKER（导出门把关）。返回就绪叶清单。
fn draft_ready_leaves(svc: &mut BidService) -> Vec<String> {
    let leaves = leaf_ids(svc);
    let mut ready = Vec::new();
    for sid in &leaves {
        match svc.draft_section(sid, None, None, None, &mut bid_tools::draft::MockGenerator) {
            Ok(_) => ready.push(sid.clone()),
            Err(e) => assert_eq!(
                e.code(), "E_UNPROCESSABLE",
                "零绑定时只有含未响应否决项的叶会拒（闸语义）: {e:?}"
            ),
        }
    }
    ready
}

/// 校验面调用（scope 参数便捷面）。
fn validate(svc: &mut BidService, pid: &str) -> Value {
    ok_result(call(svc, "validate_bid_document", json!({ "project_id": pid, "scope": "all" })))
}

// ---------------------------------------------------------------- 1. 契约形状与状态推进

#[test]
fn validate_contract_shape_and_promotes_validating() {
    // #126 适配：大纲分组把资格+否决合并进同一叶（否决项并入「资格与响应」
    // 章节组）——svc_base 的 3 REQ 只出一个叶且被否决项闸拦死，进不了
    // DRAFTING。改用 grouped 夹具（追加格式 REQ → 另出一个可零绑定起草的叶）。
    let (mut svc, pid) = svc_base_grouped("shape");
    // 缺失回报闸（#110 任务 1）：登记可用资质（不绑——未响应面留给 validate），
    // 只起草就绪叶；含未响应否决项（REQ-0003）的叶拒生成、保持 Pending
    register_qual_ledger_only(&mut svc);
    let ready = draft_ready_leaves(&mut svc);
    assert!(!ready.is_empty(), "至少资格叶可起草（进 DRAFTING）");
    let r = validate(&mut svc, &pid);
    assert_eq!(r["ok"], json!(false), "有 BLOCKER 时 ok=false（未响应×3 + 未起草叶）");
    assert!(r["findings"].as_array().unwrap().len() >= 3, "未响应 3 条");
    assert!(r["counts"]["blocker"].as_u64().unwrap() >= 3);
    assert!(r["counts"]["deterministic"].as_u64().unwrap() >= 3);
    assert_eq!(r["counts"]["model"], json!(0), "NoModel 占位——模型面恒 0");
    assert_eq!(r["sections_checked"].as_u64().unwrap() >= 1, true);
    assert_eq!(r["project_status"], "VALIDATING", "DRAFTING → VALIDATING 推进");
    // 复跑幂等（VALIDATING 态再校验——guard 允许 + 推进 no-op）
    let r2 = validate(&mut svc, &pid);
    assert_eq!(r2["counts"], r["counts"], "同输入复校结果全等（纯读诊断）");
}

#[test]
fn validate_guards_pre_drafting_states() {
    // MATRIX_READY（未起草）直接校验 → E_STATE（validate 的 guard 面）
    let (mut svc, pid) = svc_base("guard");
    let reply = call(&mut svc, "validate_bid_document", json!({ "project_id": pid }));
    assert_eq!(err_code(&reply), "E_STATE", "未起草（MATRIX_READY）校验应被状态机拒");
}

#[test]
fn validate_after_draft_runs_and_reports() {
    // draft 后（DRAFTING）→ 校验放行 + 推进 VALIDATING
    let (mut svc, pid) = svc_base("after-draft");
    // 绑证据（资格×2 + 否决×1 全绑——消除矩阵 BLOCKER 面）
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    let r = validate(&mut svc, &pid);
    // 起草完成 + 绑定齐全：BLOCKER 应只剩「格式基线」以外的零
    assert_eq!(r["project_status"], "VALIDATING");
    // errors 面收敛（未响应已消除）
    let errors = r["errors"].as_array().unwrap();
    assert!(
        errors.iter().all(|e| e["rule_code"].as_str().unwrap() != "E_VETO_UNRESPONDED"
            && e["rule_code"].as_str().unwrap() != "E_QUAL_UNMET"),
        "全绑定后未响应类应清零: {errors:?}"
    );
    // 格式基线告警在（无格式条款——降级告知）
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("基线")));
}

// ---------------------------------------------------------------- 2. 规则 1/2：矩阵未响应

#[test]
fn rule_veto_and_qual_unresponded_are_blockers() {
    // #126 适配：同 validate_contract_shape——grouped 夹具保证有可起草叶
    // （格式叶零绑定可起草 → 进 DRAFTING），资格+否决合并叶保持 Pending，
    // 未响应面由 validate 从矩阵报 BLOCKER（原测试意图不变）。
    let (mut svc, pid) = svc_base_grouped("veto-qual");
    // 缺失回报闸（#110 任务 1）：否决叶（REQ-0003 未响应）拒生成保持 Pending；
    // 资格叶登记台账资质后可起草。未响应面由 validate 从矩阵报 BLOCKER
    register_qual_ledger_only(&mut svc);
    let _ = draft_ready_leaves(&mut svc);
    let r = validate(&mut svc, &pid);
    let codes: Vec<&str> = r["findings"].as_array().unwrap().iter()
        .map(|f| f["rule_code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"E_VETO_UNRESPONDED"), "否决项未响应必须 BLOCKER: {codes:?}");
    assert!(codes.contains(&"E_QUAL_UNMET"), "资格未满足必须 BLOCKER: {codes:?}");
    // 消息携带 REQ id（定位面）
    let veto = r["findings"].as_array().unwrap().iter()
        .find(|f| f["rule_code"] == "E_VETO_UNRESPONDED").unwrap();
    assert_eq!(veto["requirement_id"], json!("REQ-0003"));
}

// ---------------------------------------------------------------- 3. 规则 3：空章节

#[test]
fn rule_section_empty_is_blocker() {
    // draft 后人为清空一章的 content（模拟起草中断）
    let (mut svc, pid) = svc_base("empty-sec");
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    let sids = leaf_ids(&svc);
    for sid in &sids {
        svc.draft_section(sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    // 清空第一章（绕过 CAS 直接改 store 是测试特权——真实面 draft 失败即空）
    svc.store_mut().set_section_content(&sids[0], String::new()).unwrap();
    let r = validate(&mut svc, &pid);
    let codes: Vec<&str> = r["findings"].as_array().unwrap().iter()
        .map(|f| f["rule_code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"E_SECTION_EMPTY"), "空章节必须 BLOCKER: {codes:?}");
}

// ---------------------------------------------------------------- 4. 规则 4：字数偏离

#[test]
fn rule_chars_off_target_warns() {
    let (mut svc, pid) = svc_base("chars");
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    // 人工超短稿（偏离 ±20%）
    let sids = leaf_ids(&svc);
    struct Tiny;
    impl bid_tools::draft::SectionGenerator for Tiny {
        fn generate(&mut self, i: &bid_tools::draft::SectionDraftInput)
            -> Result<bid_tools::draft::GeneratedDraft, String> {
            Ok(bid_tools::draft::GeneratedDraft {
                content: format!("{}短稿。", i.requirements.iter()
                    .map(|r| format!("[{}]", r.id)).collect::<Vec<_>>().join("")),
            })
        }
    }
    svc.draft_section(&sids[0], None, None, Some("超短稿"), &mut Tiny).unwrap();
    let r = validate(&mut svc, &pid);
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("偏离配额")),
        "字数偏离必须 MINOR 告警: {}", r["warnings"]);
}

// ---------------------------------------------------------------- 5. 规则 6：占位符

#[test]
fn rule_placeholder_is_major() {
    let (mut svc, pid) = svc_base("placeholder");
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    struct WithTodo;
    impl bid_tools::draft::SectionGenerator for WithTodo {
        fn generate(&mut self, i: &bid_tools::draft::SectionDraftInput)
            -> Result<bid_tools::draft::GeneratedDraft, String> {
            let base = bid_tools::draft::generate_mock(i);
            Ok(bid_tools::draft::GeneratedDraft {
                content: format!("{base}\n\nTODO：此处补充业绩清单。"),
            })
        }
    }
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut WithTodo).unwrap();
    }
    let r = validate(&mut svc, &pid);
    let errors = r["errors"].as_array().unwrap();
    assert!(errors.iter().any(|e| e["rule_code"] == "W_PLACEHOLDER"),
        "占位符残留必须 MAJOR（errors 面）: {errors:?}");
}

// ---------------------------------------------------------------- 6. 规则 7：幻觉引用

#[test]
fn rule_citation_ghost_is_blocker() {
    let (mut svc, pid) = svc_base("ghost");
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    struct Hallu;
    impl bid_tools::draft::SectionGenerator for Hallu {
        fn generate(&mut self, i: &bid_tools::draft::SectionDraftInput)
            -> Result<bid_tools::draft::GeneratedDraft, String> {
            let base = bid_tools::draft::generate_mock(i);
            // 绕过 draft 回查的形态：draft 只剔除「不在本章响应面」的引用——
            // 本测试用 store 直写（模拟上游数据损坏面）
            Ok(bid_tools::draft::GeneratedDraft { content: base })
        }
    }
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut Hallu).unwrap();
    }
    // 直写幻觉引用（测试特权——模拟 AI 面直改库的损坏态）
    let sids = leaf_ids(&svc);
    let node = svc.store().sections().iter().find(|n| n.id == sids[0]).unwrap();
    let poisoned = format!("{}\n\n依据 [EV-4242] 的业绩佐证。", node.content.clone().unwrap());
    let (attempt, _) = svc.store_mut().set_section_content(&sids[0], poisoned).unwrap();
    assert!(attempt >= 1);
    let r = validate(&mut svc, &pid);
    let errors = r["errors"].as_array().unwrap();
    assert!(errors.iter().any(|e| e["rule_code"] == "E_CITATION_GHOST"),
        "幻觉引用必须 BLOCKER: {errors:?}");
    assert_eq!(r["ok"], json!(false));
}

// ---------------------------------------------------------------- 7. 规则 8：失效证据

#[test]
fn rule_evidence_expired_is_blocker() {
    let (mut svc, pid) = svc_base("expired");
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    // 真实风险面：bind 时在效（2030 > 开标日 2026）→ 时间流逝/开标日修正后
    // apply_validity 翻转 EXPIRED（bind 时的 R-03 拦不住后置翻转）
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Certificate,
        title: "安全生产许可证".into(),
        source_file: "EV-old.pdf".into(),
        source_sha256: "sha-old".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 9, text_fingerprint: "fnv64:old".into() },
        fingerprint: "fnv64:old".into(),
        valid_from: None,
        valid_until: Some("2030-06-30".into()),
        subject_name: "某公司".into(),
        scope: "全国".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(), // #147
    }).unwrap();
    svc.store_mut().bind_evidence(&eid, "REQ-0003", "2026-12-31").unwrap(); // 此刻 Active 可绑
    svc.store_mut().apply_validity("2031-01-01").unwrap(); // 开标日修正到 2031 → 2030 < 2031 翻转 EXPIRED
    let _ = svc.store_mut().build_matrix().unwrap();
    let r = validate(&mut svc, &pid);
    let errors = r["errors"].as_array().unwrap();
    assert!(errors.iter().any(|e| e["rule_code"] == "E_EVIDENCE_EXPIRED"),
        "失效证据绑定必须 BLOCKER: {errors:?}");
}

// ---------------------------------------------------------------- 8. scope 面

#[test]
fn scope_sections_filters_and_unknown_warns() {
    let (mut svc, pid) = svc_base("scope");
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    let sids = leaf_ids(&svc);
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": pid, "scope": "sections", "section_ids": [sids[0], "SEC-9999"],
    })));
    assert_eq!(r["sections_checked"], json!(1), "scope 只校验指定叶");
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("SEC-9999")),
        "未知 scope id 告警不静默: {}", r["warnings"]);
}

#[test]
fn scope_sections_requires_ids() {
    let (mut svc, pid) = svc_base("scope-req");
    let reply = call(&mut svc, "validate_bid_document", json!({
        "project_id": pid, "scope": "sections",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
}

// ---------------------------------------------------------------- 9. 参数与路由

#[test]
fn validate_rejects_bad_params_and_routes() {
    let (mut svc, pid) = svc_base("params");
    let reply = call(&mut svc, "validate_bid_document", json!({ "project_id": pid, "scope": "nope" }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
    let reply = call(&mut svc, "validate_bid_document", json!({ "project_id": pid, "rules_profile": "mega" }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE");
    let reply = call(&mut svc, "validate_bid_document", json!({ "project_id": "PRJ-9999" }));
    assert_eq!(err_code(&reply), "E_NOTFOUND");
    // dark_bid 告警（词表面阶段 7——告知不静默）——素材面齐备后全叶可起草
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": pid, "rules_profile": "dark_bid",
    })));
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("dark_bid")));
}

#[test]
fn green_path_ok_true_when_no_blockers() {
    // 全绿面：绑定 + draft + 格式条款齐备 → ok=true（仅余字数/基线类 MINOR）
    // 4 条 REQ 一次到位（含格式条款——green 面不触发基线告警）
    let mut svc = BidService::open(tmp_dir("green")).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "全绿测试", "bid_deadline": "2030-12-31", "mode": "明标", "external_key": "green",
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-green", 10, 1, 50, 0, ParseStatus::Ok, "",
    ).unwrap();
    // #131 适配：补遗件（否则 MissingAddendum 闸拦下起草，见 svc_base 注）
    svc.store_mut().upsert_file(
        "C-补遗文件01.docx", "补遗", "sha-green-add", 5, 0, 20, 0, ParseStatus::Ok, "",
    ).unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "具备贰级资质"),
        mk_req("REQ-0002", "资格条件", "项目经理一级建造师"),
        mk_req("REQ-0003", "否决项", "未签字作否决"),
        mk_req("REQ-0004", "格式要求", "正文用仿宋三号字，行距为 1.5 倍"),
    ]).unwrap();
    let _ = svc.prepare_outline_and_matrix(
        OutlineSpec { target_total_chars: 2_000, max_reqs_per_leaf: 8 },
    ).unwrap();
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    let r = validate(&mut svc, &pid);
    assert_eq!(r["ok"], json!(true), "无 BLOCKER 时 ok=true: {}", r["findings"]);
    assert_eq!(r["counts"]["blocker"], json!(0));
    assert_eq!(r["project_status"], "VALIDATING");
}

// ---------------------------------------------------------------- #86 摩擦 #13：W_FORMAT_BASELINE 三形态

/// 夹具变体：从零构建（svc_base 已推 MATRIX_READY，不能再改条款集）——
/// 在抽取阶段就带上格式条款（excerpt 决定可否提取参数）。
fn svc_with_format_req(tag: &str, excerpt: &str) -> (BidService, String) {
    let mut svc = BidService::open(tmp_dir(tag)).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "格式基线测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    svc.store_mut()
        .upsert_file(
            "A-招标文件.docx", "招标文件", &format!("sha-vf-{tag}"), 10, 1, 50, 0, ParseStatus::Ok, "",
        )
        .unwrap();
    // #131 适配：补遗件（否则 MissingAddendum 闸拦下起草，见 svc_base 注）
    svc.store_mut()
        .upsert_file(
            "C-补遗文件01.docx", "补遗", &format!("sha-vf-add-{tag}"), 5, 0, 20, 0, ParseStatus::Ok, "",
        )
        .unwrap();
    svc.set_requirements(vec![
        mk_req("REQ-0001", "资格条件", "具备水利水电工程施工总承包贰级资质"),
        mk_req("REQ-0002", "资格条件", "项目经理具备一级建造师注册证书"),
        mk_req("REQ-0003", "否决项", "投标文件未经法定代表人签字作否决处理"),
        mk_req("REQ-0004", "格式要求", excerpt),
    ])
    .unwrap();
    let _ = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 2_000, max_reqs_per_leaf: 8 })
        .unwrap();
    (svc, pid)
}

#[test]
fn format_baseline_not_warned_when_params_extracted() {
    // 形态③：格式条款在场且参数提取成功（小四/仿宋/1.5 倍行距）→ 不得报
    // W_FORMAT_BASELINE（旧实现按「含基线」字样误报「未识别到格式条款」
    // ——文案与事实打架，#86 摩擦 #13）
    let (mut svc, pid) = svc_with_format_req(
        "fmt-hit",
        "投标文件正文采用仿宋小四号字体，1.5 倍行距，页边距上下各 25mm",
    );
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    let r = validate(&mut svc, &pid);
    let warnings = r["warnings"].as_array().unwrap();
    assert!(
        !warnings.iter().any(|w| w.as_str().unwrap().contains("未发现格式条款")),
        "提取到参数不应报「未发现格式条款」: {warnings:?}"
    );
}

#[test]
fn format_baseline_warns_with_accurate_text_when_params_not_detected() {
    // 形态②：格式条款在场但参数提不出 → 告警文案必须如实说「存在格式候选
    // 条款但未提取到…参数」，不得说「未发现格式条款」
    let (mut svc, pid) = svc_with_format_req("fmt-miss", "投标文件排版格式应符合本招标文件相关要求");
    bind_evidence_for(&mut svc, &["REQ-0001", "REQ-0002", "REQ-0003"]);
    for sid in leaf_ids(&svc) {
        svc.draft_section(&sid, None, None, None, &mut bid_tools::draft::MockGenerator).unwrap();
    }
    let r = validate(&mut svc, &pid);
    let warnings = r["warnings"].as_array().unwrap();
    let hit = warnings.iter().find(|w| w.as_str().unwrap().contains("W_FORMAT_BASELINE")
        || w.as_str().unwrap().contains("工程基线"));
    assert!(hit.is_some(), "形态②应告警: {warnings:?}");
    let msg = hit.unwrap().as_str().unwrap();
    assert!(msg.contains("未提取到"), "文案如实（存在候选但参数未提取）: {msg}");
    assert!(!msg.contains("未发现格式条款"), "不得误报零候选: {msg}");
}
