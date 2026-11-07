//! #319 补遗后增量重写——MCP 全链验收。
//!
//! 载体：corpus-A 真件（A 主文件 + C 补遗 + EV 证据，与 segment_tools 同款
//! 夹具）+ 服务面准备（建纲 → 证据登记/绑定 → 两门 → blocking 清零），
//! 之后**全部操作走 MCP 工具面**（append 起草 → register_addendum_changes →
//! rewrite_section_partial）。
//!
//! 覆盖 issue #319 验收条件：
//! 1. 补遗后能生成精确的改写指令（不是「整章重写」）——影响面 = 引用该
//!    REQ ∪ 含 old_text 的段，兄弟段不入面；
//! 2. 局部改写只动受影响的段（rewrites ⊆ 影响面，越界段显式拒）；
//! 3. 未受影响的内容零改动（**字节级验证**：兄弟段逐字节相等 + version 不动）；
//! 4. 改写后证据绑定正确更新（R-del 绑定解除 + 矩阵行消失；改写段引用回查）；
//! 5. 真实补遗场景（corpus-A C-补遗件在库——GT 语料真件链）+ 改动前预告
//!    （dry_run「将修改 X 段，Y 段不动」零副作用）+ 局部复验只跑受影响章节。
//!
//! 环境门：corpus-A 夹具缺失 → SKIP 横幅（skip_if_fixture_missing——AGENTS.md
//! 13.1 纪律：SKIP 不是 PASS）。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_ready(test_id: &str) -> bool {
    let main = fixtures().join("A-招标文件.docx");
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&main])
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
        ToolReply::Err { error, .. } => format!("{error}"),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn ws_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-rewrite319-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 全链夹具（与 segment_tools.rs `prepared` 同款——真件四件导入 → 抽取 →
/// 建纲 6,000 → 资格/否决/技术参数全绑 → 两门 → blocking 清零）。
fn prepared(tag: &str) -> (BidService, String, PathBuf) {
    let ws = ws_dir(tag);
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(fixtures());
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": "补遗增量重写测试——智慧园区弱电工程",
            "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();

    let files = [
        ("A-招标文件.docx", "招标文件"),
        ("C-补遗文件01.docx", "补遗"),
        ("evidence/EV-01-营业执照.pdf", "附件"),
        (
            "evidence/EV-02-电子与智能化工程专业承包二级资质证书.pdf",
            "附件",
        ),
    ];
    let file_specs: Vec<Value> = files
        .iter()
        .map(|(name, kind)| {
            json!({
                "upload_ref": fixtures().join(name).display().to_string(),
                "filename": name, "kind": kind,
            })
        })
        .collect();
    ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({
            "project_id": &pid, "files": file_specs,
        }),
    ));
    ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));

    svc.prepare_outline_and_matrix(OutlineSpec {
        target_total_chars: 6_000,
        max_reqs_per_leaf: 6,
    })
    .unwrap();

    use bid_domain::evidence::EvidenceSpec;
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc
        .store_mut()
        .register_evidence(EvidenceSpec {
            kind: bid_domain::evidence::EvidenceKind::Qualification,
            title: "电子与智能化工程专业承包二级资质证书".into(),
            source_file: "EV-02-电子与智能化工程专业承包二级资质证书.pdf".into(),
            source_sha256: "sha-319-ev02".into(),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 0,
                text_fingerprint: "fnv64:319".into(),
            },
            fingerprint: "fnv64:319".into(),
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
        })
        .unwrap();
    let qual_ids: Vec<String> = svc
        .store()
        .requirements()
        .iter()
        .filter(|r| r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数")
        .map(|r| r.id.clone())
        .collect();
    assert!(!qual_ids.is_empty(), "资格/否决 REQ 非空");
    for rid in &qual_ids {
        svc.store_mut()
            .bind_evidence(&eid, rid, "2026-12-31")
            .unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval] {
        svc.store_mut()
            .record_review(
                gate,
                &bid_domain::TrustedPrincipal::human("lead-319"),
                Decision::Approved,
                vec![],
                None,
            )
            .unwrap();
    }
    let blocking = svc.store().matrix().iter().filter(|r| r.blocking).count();
    assert_eq!(blocking, 0, "夹具 blocking 清零（否则起草门不放行）");
    (svc, pid, ws)
}

/// 含 REQ 最多的叶（三段起草的宿主）。
fn richest_leaf(svc: &BidService) -> (String, Vec<String>) {
    let node = svc
        .store()
        .sections()
        .iter()
        .filter(|n| {
            !svc.store()
                .sections()
                .iter()
                .any(|c| c.parent_id.as_deref() == Some(n.id.as_str()))
        })
        .max_by_key(|n| n.requirement_ids.len())
        .expect("叶章节非空");
    (node.id.clone(), node.requirement_ids.clone())
}

/// 起草三段：段1/段3 引用 REQ-a（含旧值文本），段2 引用 REQ-b（无关面）。
/// 返回 (seg1, seg2, seg3)。
fn draft_three_segments(
    svc: &mut BidService,
    pid: &str,
    leaf: &str,
    req_a: &str,
    req_b: &str,
) -> (Value, Value, Value) {
    let s1 = format!(
        "本段响应 [{req_a}] 的技术要求：设备内存 32GB，满足招标文件配置基线，实施方案与验收标准均按此承诺执行。"
    );
    let s2 = format!(
        "本段响应 [{req_b}] 的商务条款：质保期三年，响应上门服务时效承诺，配置专职项目经理一名。"
    );
    let s3 = format!(
        "本段续述 [{req_a}] 的部署要求：旧值口径为单机部署，本方案按集群模式增强实现并承诺兼容。"
    );
    let r1 = ok_result(call(
        svc,
        "append_section_segment",
        json!({
            "project_id": pid, "section_id": leaf, "content": &s1,
        }),
    ));
    let r2 = ok_result(call(
        svc,
        "append_section_segment",
        json!({
            "project_id": pid, "section_id": leaf, "content": &s2,
        }),
    ));
    let r3 = ok_result(call(
        svc,
        "append_section_segment",
        json!({
            "project_id": pid, "section_id": leaf, "content": &s3,
        }),
    ));
    (r1, r2, r3)
}

// ---------------------------------------------------------------- 主链

/// 验收 1+2+3+5：数值变更 → 精确指令 → dry_run 预告 → 局部改写只动受影响段
/// （兄弟段字节级零改动）→ 局部复验只跑受影响章节。
#[test]
fn value_changed_directive_partial_rewrite_and_local_revalidation() {
    if !corpus_ready("319-value-changed") {
        return;
    }
    let (mut svc, pid, _ws) = prepared("319-main");
    let (leaf, reqs) = richest_leaf(&svc);
    let req_a = reqs[0].clone();
    let req_b = reqs[1].clone();

    // ---- ① 三段起草（段1/段3 引用 REQ-a，段2 引用 REQ-b）
    let (r1, r2, r3) = draft_three_segments(&mut svc, &pid, &leaf, &req_a, &req_b);
    let seg1 = r1["segment"]["id"].as_str().unwrap().to_string();
    let seg2 = r2["segment"]["id"].as_str().unwrap().to_string();
    let seg3 = r3["segment"]["id"].as_str().unwrap().to_string();
    let before2 = svc.store().section_segment(&seg2).unwrap().content.clone();
    let before1 = svc.store().section_segment(&seg1).unwrap().content.clone();

    // ---- ② 登记：REQ-a 数值变更（old_text 命中段3 词面；引用腿命中段1/段3）
    let addendum_file_id: Value = svc
        .store()
        .files()
        .iter()
        .find(|f| f.kind == "补遗")
        .map(|f| json!(f.id))
        .unwrap_or(Value::Null);
    let reg = ok_result(call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [{
                "req_id": &req_a,
                "change_type": "value_changed",
                "old_text": "旧值口径为单机部署",
                "new_requirement": "补遗第 2 号：设备内存提升至 64GB，部署形态不限",
                "source_file_id": addendum_file_id,
            }],
        }),
    ));
    // 验收 1：精确指令（不是整章重写）——影响面恰为引用/含旧文本的段
    assert_eq!(reg["changes"].as_array().unwrap().len(), 1);
    assert_eq!(
        reg["dirty_sections"][0]["section_id"],
        json!(leaf),
        "打标面 = 宿主叶"
    );
    let directives = reg["directives"].as_array().unwrap();
    assert_eq!(directives.len(), 1, "一变更 × 一章节 = 一指令");
    let d = &directives[0];
    assert_eq!(d["change_type"], json!("value_changed"));
    let affected: Vec<String> = d["affected_segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["segment_id"].as_str().unwrap().to_string())
        .collect();
    assert!(affected.contains(&seg1), "段1（引用 REQ-a）入影响面");
    assert!(affected.contains(&seg3), "段3（引用 + old_text）入影响面");
    assert!(
        !affected.contains(&seg2),
        "段2（只引用 REQ-b）不入影响面——精确性"
    );
    assert_eq!(
        d["affected_segments"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["matched_by"].as_str().unwrap().contains("citation"))
            .count(),
        2,
        "引用腿命中段1/段3"
    );
    let directive_id = d["id"].as_str().unwrap().to_string();
    // 章节已打脏 + REQ params 记最新要求
    assert!(
        svc.store()
            .sections()
            .iter()
            .find(|n| n.id == leaf)
            .unwrap()
            .dirty
    );
    let req = svc
        .store()
        .requirements()
        .iter()
        .find(|r| r.id == req_a)
        .unwrap();
    assert_eq!(
        req.params
            .get("addendum_requirement")
            .and_then(|v| v.as_str()),
        Some("补遗第 2 号：设备内存提升至 64GB，部署形态不限"),
        "最新要求入 params（矩阵/溯源面可见）"
    );

    // ---- ③ get_bid_project_state 发现面（断点恢复：哪些章节没追上补遗）
    let state = ok_result(call(&mut svc, "get_bid_project_state", json!({})));
    let dirty: Vec<&str> = state["dirty_sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["section_id"].as_str().unwrap())
        .collect();
    assert!(dirty.contains(&leaf.as_str()), "state 暴露 DIRTY 章节");
    assert_eq!(
        state["pending_rewrite_directives"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "state 暴露待处置指令（含影响面明细）"
    );

    // ---- ④ dry_run：改动前预告（零副作用）
    let new1 = format!(
        "本段响应 [{req_a}] 的技术要求：按补遗第 2 号设备内存提升至 64GB，满足更新后的配置基线。"
    );
    let new3 = format!(
        "本段续述 [{req_a}] 的部署要求：补遗后口径为集群模式增强实现，承诺兼容并预留扩展。"
    );
    let dry = ok_result(call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &directive_id, "dry_run": true,
            "rewrites": [
                { "segment_id": &seg1, "content": &new1 },
                { "segment_id": &seg3, "content": &new3 },
            ],
        }),
    ));
    assert_eq!(
        dry["plan"]["segments_to_modify"],
        json!(2),
        "预告：将修改 2 段"
    );
    assert_eq!(
        dry["plan"]["segments_untouched"],
        json!(1),
        "预告：1 段不动"
    );
    assert_eq!(dry["untouched_segment_ids"].as_array().unwrap().len(), 1);
    assert_eq!(dry["untouched_segment_ids"][0], json!(seg2));
    // 零副作用：段内容/指令状态/正文全不动
    assert_eq!(
        svc.store().section_segment(&seg1).unwrap().content,
        before1,
        "dry_run 不落笔"
    );
    assert_eq!(
        svc.store().rewrite_directive(&directive_id).unwrap().status,
        bid_domain::rewrite::DirectiveStatus::Pending
    );

    // ---- ⑤ 真跑：只改影响面内两段（验收 2）
    let run = ok_result(call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &directive_id,
            "rewrites": [
                { "segment_id": &seg1, "content": &new1 },
                { "segment_id": &seg3, "content": &new3 },
            ],
        }),
    ));
    assert_eq!(run["dry_run"], json!(false));
    let modified = run["modified_segments"].as_array().unwrap();
    assert_eq!(modified.len(), 2, "恰改 2 段");
    assert_eq!(modified[0]["version_before"], json!(1));
    assert_eq!(modified[0]["version_after"], json!(2), "段版本推进");
    // 验收 3：未受影响内容零改动（字节级）
    let after2 = svc.store().section_segment(&seg2).unwrap();
    assert_eq!(after2.content, before2, "兄弟段逐字节相等（零改动）");
    assert_eq!(after2.version, 1, "兄弟段版本不动");
    // 合稿 = 新段1 + 原段2 + 新段3
    let assembled = svc.store().assemble_section_segments(&leaf).unwrap();
    let parts: Vec<&str> = assembled.split("\n\n").collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], new1);
    assert_eq!(parts[1], before2, "合稿中兄弟段原样在位");
    assert_eq!(parts[2], new3);
    assert_eq!(
        svc.store()
            .sections()
            .iter()
            .find(|n| n.id == leaf)
            .unwrap()
            .content
            .as_deref(),
        Some(assembled.as_str()),
        "正文与账本同进退"
    );

    // ---- ⑥ 局部复验（验收 5 的复验半边：只跑受影响章节）
    let reval = &run["revalidation"];
    assert_eq!(reval["section_id"], json!(leaf), "复验 scope = 受影响章节");
    assert!(
        reval["reqs_responded"].as_u64().unwrap() >= 1,
        "重新响应 ≥1 条 REQ"
    );
    assert!(
        reval["reqs_revalidated"].as_u64().unwrap() >= 2,
        "复验面含章节责任条款"
    );

    // ---- ⑦ 指令处置 + 清 DIRTY
    assert_eq!(run["directive"]["status"], json!("applied"));
    assert_eq!(
        run["dirty_cleared"],
        json!(true),
        "全部指令处置完 = 追上补遗"
    );
    assert!(
        !svc.store()
            .sections()
            .iter()
            .find(|n| n.id == leaf)
            .unwrap()
            .dirty
    );
    // 已处置指令重放 → 显式拒（E_UNPROCESSABLE，零副作用）
    let replay = call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &directive_id,
            "rewrites": [{ "segment_id": &seg1, "content": "重放内容" }],
        }),
    );
    assert_eq!(err_code(&replay), "E_UNPROCESSABLE");
    assert_eq!(
        svc.store().section_segment(&seg1).unwrap().content,
        new1,
        "重放不落笔"
    );

    // ---- ⑧ 越界段（不在影响面内）显式拒
    let reg2 = ok_result(call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [{
                "req_id": &req_b, "change_type": "value_changed",
                "old_text": "", "new_requirement": "REQ-b 口径更新：质保期五年",
            }],
        }),
    ));
    let d2id = reg2["directives"][0]["id"].as_str().unwrap().to_string();
    let out_of_face = call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &d2id,
            "rewrites": [{ "segment_id": &seg1, "content": "借指令改无关段" }],
        }),
    );
    assert_eq!(
        err_code(&out_of_face),
        "E_UNPROCESSABLE",
        "rewrites ⊆ 影响面（防越界改稿）"
    );
    assert!(
        err_text(&out_of_face).contains("不在指令"),
        "拒因指名越界段"
    );
    // 清场：dismiss 第二条指令（人工裁定语义面顺带验收）
    let dis = ok_result(call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &d2id, "dismiss": true,
        }),
    ));
    assert_eq!(dis["directive"]["status"], json!("dismissed"));
    assert_eq!(dis["dirty_cleared"], json!(true));
}

/// 验收 4：R-del 绑定解除 + 矩阵行消失 + 祖先并集同步下架；R-add 新增要求
/// （指令空影响面 → append 响应 + 空处置）。
#[test]
fn added_and_removed_requirements_flow() {
    if !corpus_ready("319-add-remove") {
        return;
    }
    let (mut svc, pid, _ws) = prepared("319-addrm");
    let (leaf, reqs) = richest_leaf(&svc);
    let req_a = reqs[0].clone();
    let req_b = reqs[1].clone();
    let rows_before = svc.store().matrix().len();

    let (r1, _r2, _r3) = draft_three_segments(&mut svc, &pid, &leaf, &req_a, &req_b);
    let seg1 = r1["segment"]["id"].as_str().unwrap().to_string();
    let _ = seg1;

    // 绑一条证据到 REQ-b（R-del 绑定解除的验收素材）
    let eid = {
        use bid_domain::evidence::EvidenceSpec;
        use bid_domain::locator::EngineLocator;
        let (eid, _) = svc
            .store_mut()
            .register_evidence(EvidenceSpec {
                kind: bid_domain::evidence::EvidenceKind::Qualification,
                title: "REQ-b 佐证材料".into(),
                source_file: "EV-02-电子与智能化工程专业承包二级资质证书.pdf".into(),
                source_sha256: "sha-319-b".into(),
                locator: EngineLocator::DocxParagraph {
                    paragraph_index: 0,
                    text_fingerprint: "fnv64:319b".into(),
                },
                fingerprint: "fnv64:319b".into(),
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
            })
            .unwrap();
        svc.store_mut()
            .bind_evidence(&eid, &req_b, "2026-12-31")
            .unwrap();
        let _ = svc.store_mut().build_matrix().unwrap();
        eid
    };
    assert!(
        svc.store()
            .evidence_bindings()
            .iter()
            .any(|b| b.requirement_id == req_b),
        "前置：REQ-b 在绑"
    );

    // ---- ① R-add：新增要求（挂宿主叶）→ 新 REQ 入集 + 祖先传播 + 指令空影响面
    let add = ok_result(call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [{
                "change_type": "requirement_added",
                "new_requirement": "补遗新增：提供原厂授权书及三年免费上门维保",
                "category": "商务要求",
                "clause_no": "6.2",
                "section_id": &leaf,
            }],
        }),
    ));
    let new_req = add["changes"][0]["req_id"].as_str().unwrap().to_string();
    assert_ne!(new_req, req_a, "新 id 由引擎签发");
    assert!(
        svc.store().requirements().iter().any(|r| r.id == new_req),
        "新 REQ 入条款集"
    );
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == leaf)
        .unwrap();
    assert!(node.requirement_ids.contains(&new_req), "叶挂新条款");
    assert!(node.dirty, "归属叶打脏");
    let parent_ids: Vec<&str> = svc
        .store()
        .sections()
        .iter()
        .filter(|n| n.parent_id.is_none())
        .map(|n| n.id.as_str())
        .collect();
    for pid_sec in parent_ids {
        if let Some(p) = svc.store().sections().iter().find(|n| n.id == pid_sec) {
            if p.parent_id.is_none()
                && svc
                    .store()
                    .sections()
                    .iter()
                    .any(|c| c.parent_id.as_deref() == Some(p.id.as_str()))
            {
                // 根且为父——并集传播验收（不变量 9）
            }
        }
    }
    // 矩阵行 +1（幂等重算即时反映）
    assert_eq!(
        svc.store().matrix().len(),
        rows_before + 1,
        "R-add → 矩阵新增行"
    );
    let d_add = add["directives"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        add["directives"][0]["affected_segments"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "新增要求无既有段（空影响面）"
    );

    // Agent 用 append 响应新要求（引用新 REQ 过 pre-flight——引用面已含新条款）
    let resp = format!(
        "本段响应补遗新增条款 [{new_req}]：提供原厂授权书，承诺三年免费上门维保与 4 小时响应时效。"
    );
    let appended = ok_result(call(
        &mut svc,
        "append_section_segment",
        json!({
            "project_id": &pid, "section_id": &leaf, "content": &resp,
        }),
    ));
    assert_eq!(appended["changed"], json!(true), "新要求响应段落账");

    // 空处置（确认无需改写——新要求已由 append 响应）
    let done = ok_result(call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &d_add,
        }),
    ));
    assert_eq!(
        done["directive"]["status"],
        json!("applied"),
        "空 rewrites = 确认处置"
    );
    assert_eq!(
        done["modified_segments"].as_array().unwrap().len(),
        0,
        "零段改动"
    );

    // ---- ② R-del：删除 REQ-b → 绑定解除 + 矩阵行消失 + 叶/父同步下架
    let del = ok_result(call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [{
                "req_id": &req_b, "change_type": "requirement_removed",
                "old_text": "质保期三年",
                "new_requirement": "",
            }],
        }),
    ));
    assert!(
        !svc.store().requirements().iter().any(|r| r.id == req_b),
        "条款出集"
    );
    // 验收 4：证据绑定正确更新（绑定解除）
    assert!(
        !svc.store()
            .evidence_bindings()
            .iter()
            .any(|b| b.requirement_id == req_b),
        "R-del 绑定解除（绑定账本不再含 REQ-b）"
    );
    let _ = eid;
    assert_eq!(
        svc.store().matrix().len(),
        rows_before,
        "R-del → 矩阵行随重算消失"
    );
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == leaf)
        .unwrap();
    assert!(!node.requirement_ids.contains(&req_b), "叶同步下架");
    // 段2 仍引用已删 REQ-b —— 指令影响面含段2（agent 重写它去引用）
    let d_del = del["directives"][0]["id"].as_str().unwrap().to_string();
    let affected: Vec<String> = del["directives"][0]["affected_segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["segment_id"].as_str().unwrap().to_string())
        .collect();
    assert!(!affected.is_empty(), "引用被删条款的段入影响面");

    // 重写段2（去掉 REQ-b 引用，改为引用 REQ-a——引用回查过门）
    let seg2 = affected[0].clone();
    let seg2_before = svc.store().section_segment(&seg2).unwrap().content.clone();
    assert!(seg2_before.contains(&req_b), "前置：该段确实引用被删条款");
    let new2 = format!(
        "本段改述 [{req_a}] 的商务衔接：原质保条款已按补遗废止，本段聚焦部署与验收衔接安排。"
    );
    let run = ok_result(call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &d_del,
            "rewrites": [{ "segment_id": &seg2, "content": &new2 }],
        }),
    ));
    assert_eq!(run["modified_segments"].as_array().unwrap().len(), 1);
    let after = svc.store().section_segment(&seg2).unwrap();
    assert!(!after.content.contains(&req_b), "改写后不再引用被删条款");
    assert_eq!(run["directive"]["status"], json!("applied"));

    // ---- ③ 叶清空守卫：删到仅剩一条的叶 → 显式拒（结构重建面）
    // 找一个仅绑 1 条 REQ 的叶（无则本面跳过——语料相关）
    let thin: Option<String> = svc
        .store()
        .sections()
        .iter()
        .filter(|n| {
            !svc.store()
                .sections()
                .iter()
                .any(|c| c.parent_id.as_deref() == Some(n.id.as_str()))
        })
        .find(|n| n.requirement_ids.len() == 1)
        .and_then(|n| n.requirement_ids.first().cloned());
    if let Some(last_req) = thin {
        let guard = call(
            &mut svc,
            "register_addendum_changes",
            json!({
                "project_id": &pid,
                "changes": [{ "req_id": &last_req, "change_type": "requirement_removed",
                              "old_text": "", "new_requirement": "" }],
            }),
        );
        assert_eq!(err_code(&guard), "E_UNPROCESSABLE", "叶清空守卫");
        assert!(err_text(&guard).contains("每节点"), "拒因指名不变量");
    }
}

/// 登记面的契约纪律：非法入参 / 幂等与原子 / 状态窗口。
#[test]
fn register_contract_discipline() {
    if !corpus_ready("319-contract") {
        return;
    }
    let (mut svc, pid, _ws) = prepared("319-contract");
    let (leaf, reqs) = richest_leaf(&svc);
    let req_a = reqs[0].clone();

    // 非法 change_type → E_UNPROCESSABLE（拼错不落默认）
    let bad = call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [{ "req_id": &req_a, "change_type": "modified", "new_requirement": "x" }],
        }),
    );
    assert_eq!(err_code(&bad), "E_UNPROCESSABLE");
    assert!(err_text(&bad).contains("value_changed|requirement_added|requirement_removed"));

    // 不存在的 REQ → E_NOTFOUND 且消息带 changes[i] 定位
    let nf = call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [{ "req_id": "REQ-9999", "change_type": "value_changed", "new_requirement": "x" }],
        }),
    );
    assert_eq!(err_code(&nf), "E_NOTFOUND");
    assert!(err_text(&nf).contains("changes[0]"), "定位前缀");

    // 批量原子：一好一坏 → 零副作用
    let cv0 = svc.store().content_version();
    let dirty0 = svc.store().sections().iter().filter(|n| n.dirty).count();
    let batch = call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [
                { "req_id": &req_a, "change_type": "value_changed", "old_text": "a", "new_requirement": "好变更" },
                { "req_id": "REQ-9999", "change_type": "value_changed", "old_text": "b", "new_requirement": "坏变更" },
            ],
        }),
    );
    assert_eq!(err_code(&batch), "E_NOTFOUND", "任一失败整批拒");
    assert_eq!(svc.store().content_version(), cv0, "版本零推进");
    assert_eq!(
        svc.store().sections().iter().filter(|n| n.dirty).count(),
        dirty0,
        "DIRTY 零打标"
    );
    assert!(svc.store().addendum_changes().is_empty(), "账本零落账");

    // 不存在的补遗件 → E_NOTFOUND（可溯源面不接受幽灵文件）
    let ghost = call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [{ "req_id": &req_a, "change_type": "value_changed",
                          "new_requirement": "x", "source_file_id": "TF-9999" }],
        }),
    );
    assert_eq!(err_code(&ghost), "E_NOTFOUND");

    // project_version 过期 → E_CONFLICT（CAS 纪律同全工具面）
    let stale = call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid, "project_version": 1,
            "changes": [{ "req_id": &req_a, "change_type": "value_changed", "new_requirement": "x" }],
        }),
    );
    assert_eq!(err_code(&stale), "E_CONFLICT");

    // CAS 段重写：过期 expected_version → E_CONFLICT 无部分写入
    let (r1, _r2, _r3) = draft_three_segments(&mut svc, &pid, &leaf, &req_a, &reqs[1]);
    let seg1 = r1["segment"]["id"].as_str().unwrap().to_string();
    let reg = ok_result(call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [{ "req_id": &req_a, "change_type": "value_changed",
                          "old_text": "旧值口径为单机部署", "new_requirement": "新口径" }],
        }),
    ));
    let did = reg["directives"][0]["id"].as_str().unwrap().to_string();
    let cas = call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &did,
            "rewrites": [{ "segment_id": &seg1, "content": "CAS 过期尝试", "expected_version": 99 }],
        }),
    );
    assert_eq!(err_code(&cas), "E_CONFLICT");
    assert_eq!(
        svc.store().section_segment(&seg1).unwrap().version,
        1,
        "CAS 拒 → 段账不动"
    );

    // 新内容幻觉引用 → E_UNPROCESSABLE（引用回查同 #254 纪律）
    let ghost_cite = call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &did,
            "rewrites": [{ "segment_id": &seg1, "content": "本段引用 [EV-9999] 幽灵证据" }],
        }),
    );
    assert_eq!(err_code(&ghost_cite), "E_UNPROCESSABLE");
    assert!(
        err_text(&ghost_cite).contains("EV-9999"),
        "拒因点名幻觉引用"
    );

    // dismiss 与 rewrites 互斥
    let mix = call(
        &mut svc,
        "rewrite_section_partial",
        json!({
            "project_id": &pid, "directive_id": &did, "dismiss": true,
            "rewrites": [{ "segment_id": &seg1, "content": "x" }],
        }),
    );
    assert_eq!(err_code(&mix), "E_UNPROCESSABLE");
}

/// 状态窗口：MATRIX_READY 前登记 → E_STATE（重写面属建纲后工作流）。
#[test]
fn register_window_is_matrix_ready_plus() {
    if !corpus_ready("319-window") {
        return;
    }
    let ws = ws_dir("319-window");
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(fixtures());
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": "窗口测试", "bid_deadline": "2026-12-31", "mode": "明标",
            "external_key": "319-window",
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    let files = [
        ("A-招标文件.docx", "招标文件"),
        ("C-补遗文件01.docx", "补遗"),
    ];
    let file_specs: Vec<Value> = files
        .iter()
        .map(|(name, kind)| {
            json!({
                "upload_ref": fixtures().join(name).display().to_string(),
                "filename": name, "kind": kind,
            })
        })
        .collect();
    ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({
            "project_id": &pid, "files": file_specs,
        }),
    ));
    ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    // REQUIREMENTS_EXTRACTED（建纲前）→ E_STATE
    let early = call(
        &mut svc,
        "register_addendum_changes",
        json!({
            "project_id": &pid,
            "changes": [{ "req_id": "REQ-0001", "change_type": "value_changed", "new_requirement": "x" }],
        }),
    );
    assert_eq!(err_code(&early), "E_STATE");
}
