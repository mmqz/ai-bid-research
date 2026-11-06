//! #254 章节级增量写入——MCP 全链验收。
//!
//! 载体：corpus-A 真件（A 主文件 + C 补遗 + EV 证据）+ 服务面准备（与
//! e2e_pipeline 同款：建纲 → 证据登记/绑定 → 两门 → blocking 清零），
//! 之后**全部分段操作走 MCP 工具面**（append / rewrite / list）。
//!
//! 覆盖 issue #254 验收条件：
//! 1. 多段（每段 < 单次输出上限）写超限章节 → 重启后段仍在、顺序正确、
//!    无丢段/重复段（合稿 = 有序拼接）；
//! 2. 同段同内容重提 = 幂等 no-op（缺省指纹键）；
//! 3. 改中间段不动其他段（局部修订）；
//! 4. 过期 expected_version → E_CONFLICT 且无部分写入；
//! 5. `E_CHARS_BELOW_MIN` 按合稿计算（单段皆低于门，合稿过门 → 无 finding）；
//!    pre-flight 引用拒收（未知引用整段拒，账本不动）；
//! 6. 整稿 `draft_response_section` 向后兼容：照常工作，且与分段账本互斥
//!    （整稿不同内容 → 账本重置单段）。

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
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn err_text(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => format!("{error}"),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn seg_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-seg-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 全链夹具：corpus-A 四件 → 抽取 → 建纲（6,000 配额）→ 证据登记/绑定 →
/// 两门 → blocking 清零（与 e2e_pipeline ⑤ 同款——readiness 可过的前提）。
/// 返回 (svc, pid, 工作区路径)。
fn prepared(tag: &str) -> (BidService, String, PathBuf) {
    let ws = seg_ws(tag);
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(fixtures());
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "分段写作测试——智慧园区弱电工程",
        "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"]
        .as_str()
        .unwrap()
        .to_string();

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
    ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": &pid, "files": file_specs,
    })));
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));

    svc.prepare_outline_and_matrix(OutlineSpec { target_total_chars: 6_000, max_reqs_per_leaf: 6 })
        .unwrap();

    use bid_domain::evidence::EvidenceSpec;
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc
        .store_mut()
        .register_evidence(EvidenceSpec {
            kind: bid_domain::evidence::EvidenceKind::Qualification,
            title: "电子与智能化工程专业承包二级资质证书".into(),
            source_file: "EV-02-电子与智能化工程专业承包二级资质证书.pdf".into(),
            source_sha256: "sha-seg-ev02".into(),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 0,
                text_fingerprint: "fnv64:seg".into(),
            },
            fingerprint: "fnv64:seg".into(),
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
        .filter(|r| {
            r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数"
        })
        .map(|r| r.id.clone())
        .collect();
    assert!(!qual_ids.is_empty(), "资格/否决 REQ 非空");
    for rid in &qual_ids {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval] {
        svc.store_mut()
            .record_review(gate, &bid_domain::TrustedPrincipal::human("seg-lead"), Decision::Approved, vec![], None)
            .unwrap();
    }
    let blocking = svc.store().matrix().iter().filter(|r| r.blocking).count();
    assert_eq!(blocking, 0, "夹具 blocking 清零（否则起草门不放行）");
    (svc, pid, ws)
}

/// 目标叶章节：含 REQ 且 level 最深者（其 REQ id 用于段内引用）。
fn target_leaf(svc: &BidService) -> (String, String) {
    let node = svc
        .store()
        .sections()
        .iter()
        .filter(|n| !svc.store().sections().iter().any(|c| c.parent_id.as_deref() == Some(n.id.as_str())))
        .max_by_key(|n| n.requirement_ids.len())
        .expect("叶章节非空");
    let rid = node.requirement_ids.first().cloned().expect("叶必绑 REQ");
    (node.id.clone(), rid)
}

/// 填充正文（n 个中文字符 + 引用锚）。
fn filler(n_chars: usize, req: &str) -> String {
    let mut s = format!("本段逐项响应 [{req}] 的要求，结合园区弱电工程实际组织实施。");
    while s.chars().count() < n_chars {
        s.push_str("覆盖实施方案、人员配置与质量保障措施。");
    }
    s
}

// ---------------------------------------------------------------- 主链

#[test]
fn segments_lifecycle_multibatch_persist_and_assembly() {
    if !corpus_ready("segments-lifecycle") {
        return;
    }
    let (mut svc, pid, ws) = prepared("seg-lifecycle");
    let (leaf, rid) = target_leaf(&svc);
    let target = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == leaf)
        .unwrap()
        .target_chars;
    assert!(target > 100, "夹具配额合理（实际 {target}）");

    // ---- ① 多段追加（每段都远小于配额——模拟分批写作）
    let s1 = filler(60, &rid);
    let r1 = ok_result(call(&mut svc, "append_section_segment", json!({
        "project_id": &pid, "section_id": &leaf, "content": &s1,
    })));
    assert_eq!(r1["changed"], json!(true), "首段 changed");
    assert_eq!(r1["segment"]["seq"], json!(1));
    let seg1 = r1["segment"]["id"].as_str().unwrap().to_string();

    let s2 = filler(50, &rid);
    let r2 = ok_result(call(&mut svc, "append_section_segment", json!({
        "project_id": &pid, "section_id": &leaf, "content": &s2, "idem_key": "batch-2",
    })));
    assert_eq!(r2["segment"]["seq"], json!(2), "seq 接续");
    let seg2 = r2["segment"]["id"].as_str().unwrap().to_string();

    let s3 = filler(80, &rid);
    let r3 = ok_result(call(&mut svc, "append_section_segment", json!({
        "project_id": &pid, "section_id": &leaf, "content": &s3, "idem_key": "batch-3",
    })));
    assert_eq!(r3["segment"]["seq"], json!(3));
    assert_eq!(r3["segment_count"], json!(3));
    assert_eq!(r3["assembled_chars"].as_u64().unwrap(),
        (s1.chars().count() + 2 + s2.chars().count() + 2 + s3.chars().count()) as u64,
        "合稿字数 = 三段拼接（含分隔符）");

    // 合稿 == 有序拼接（真源一致）
    let merged = svc.store().assemble_section_segments(&leaf).unwrap();
    assert_eq!(merged, format!("{s1}\n\n{s2}\n\n{s3}"), "合稿顺序正确无丢段");
    // 章节状态推进（与整稿同链）
    let status = svc.store().sections().iter().find(|n| n.id == leaf).unwrap().status.as_str();
    assert_eq!(status, "DRAFTED", "分段推进章节状态");

    // ---- ② 幂等：同段同内容重提（缺省指纹键）→ no-op
    let version_before = svc.store().content_version();
    let dup = ok_result(call(&mut svc, "append_section_segment", json!({
        "project_id": &pid, "section_id": &leaf, "content": &s1,
    })));
    assert_eq!(dup["changed"], json!(false), "同内容重提 → deduplicated");
    assert_eq!(dup["segment"]["seq"], json!(1), "命中已有段（非新段）");
    assert_eq!(dup["segment_count"], json!(3), "段数不变");
    assert_eq!(svc.store().content_version(), version_before, "幂等零副作用");
    // 显式 idem_key 重放（batch-2 重提、内容不同）→ 仍 no-op
    let dup2 = ok_result(call(&mut svc, "append_section_segment", json!({
        "project_id": &pid, "section_id": &leaf, "content": "完全不同的内容", "idem_key": "batch-2",
    })));
    assert_eq!(dup2["changed"], json!(false), "同 idem_key 重放 → no-op（网络重试面）");
    assert_eq!(svc.store().content_version(), version_before, "重放零副作用");

    // ---- ③ 局部修订：改中间段，兄弟段不动
    let s2b = filler(50, &rid).replace("组织实施", "组织修订实施");
    let before: Vec<(u32, String, u32)> = svc
        .store()
        .list_section_segments(&leaf)
        .into_iter()
        .map(|x| (x.seq, x.content.clone(), x.version))
        .collect();
    let rw = ok_result(call(&mut svc, "rewrite_section_segment", json!({
        "project_id": &pid, "segment_id": &seg2, "content": &s2b, "expected_version": 1,
    })));
    assert_eq!(rw["changed"], json!(true));
    assert_eq!(rw["segment"]["version"], json!(2), "version +1");
    let after: Vec<(u32, String, u32)> = svc
        .store()
        .list_section_segments(&leaf)
        .into_iter()
        .map(|x| (x.seq, x.content.clone(), x.version))
        .collect();
    assert_eq!(after[0], before[0], "段 1 原样");
    assert_eq!(after[2], before[2], "段 3 原样（局部修订不越界）");
    assert_eq!(after[1].1, s2b, "段 2 已更新");

    // ---- ④ CAS：过期 expected_version → E_CONFLICT 无部分写入
    let merged_before = svc.store().assemble_section_segments(&leaf).unwrap();
    let err = err_text(&call(&mut svc, "rewrite_section_segment", json!({
        "project_id": &pid, "segment_id": &seg2, "content": "stale 写入",
        "expected_version": 1,
    })));
    assert!(err.contains("E_CONFLICT"), "过期 version 显式拒: {err}");
    assert_eq!(
        svc.store().assemble_section_segments(&leaf).unwrap(),
        merged_before,
        "E_CONFLICT 无部分写入（合稿不动）"
    );

    // ---- ⑤ pre-flight 引用拒收：未知引用整段拒（账本不动）
    let n_before = svc.store().list_section_segments(&leaf).len();
    let err = err_text(&call(&mut svc, "append_section_segment", json!({
        "project_id": &pid, "section_id": &leaf,
        "content": "幻觉引用 [REQ-9999] 的段落",
    })));
    assert!(err.contains("引用回查失败") && err.contains("REQ-9999"), "pre-flight 点名: {err}");
    assert_eq!(svc.store().list_section_segments(&leaf).len(), n_before, "拒收零副作用");

    // ---- ⑥ 重启持久化：重开工作区，段仍在、顺序正确、合稿不变
    drop(svc);
    let mut svc2 = BidService::open(&ws).unwrap().with_fs_root(fixtures());
    let list = ok_result(call(&mut svc2, "list_section_segments", json!({
        "project_id": &pid, "section_id": &leaf,
    })));
    let segs = list["segments"].as_array().unwrap();
    assert_eq!(segs.len(), 3, "重启后三段全在");
    let seqs: Vec<u64> = segs.iter().map(|s| s["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, vec![1, 2, 3], "重启后顺序正确");
    assert_eq!(
        svc2.store().assemble_section_segments(&leaf).unwrap(),
        merged_before,
        "重启后合稿不变（无丢段/重复段）"
    );
    // 断点续写：重启后继续 append → seq 接续
    let s4 = filler(40, &rid);
    let r4 = ok_result(call(&mut svc2, "append_section_segment", json!({
        "project_id": &pid, "section_id": &leaf, "content": &s4, "idem_key": "batch-4",
    })));
    assert_eq!(r4["segment"]["seq"], json!(4), "续写 seq 接续（id 水位防碰撞）");

    // ---- ⑦ E_CHARS_BELOW_MIN 按合稿：单段皆低于 80% 门，合稿补足 → 无该 finding
    //      （若门按单段算，每段都会误报——这是「门在合稿面」的直接证据）
    let target = svc2.store().sections().iter().find(|n| n.id == leaf).unwrap().target_chars;
    let need = (f64::from(target) * 0.85).ceil() as usize; // 补到 85%（> 80% 门）
    let have = svc2.store().assemble_section_segments(&leaf).unwrap().chars().count();
    if have < need {
        let pad_len = need - have + 10;
        let pad = format!("[{rid}] {pad_len} 深化响应与保障细则：{}", "质量与进度保障逐条落实到位。".repeat(pad_len / 14 + 1));
        ok_result(call(&mut svc2, "append_section_segment", json!({
            "project_id": &pid, "section_id": &leaf, "content": &pad, "idem_key": "batch-fill",
        })));
    }
    // 章节级校验（scope=sections——#243 全稿门要求全部叶 DRAFTED，本测试
    // 只对本章节验证「门按合稿」语义，全稿链路由 e2e_pipeline 覆盖）
    let v = ok_result(call(&mut svc2, "validate_bid_document", json!({
        "project_id": &pid, "scope": "sections", "section_ids": [&leaf],
    })));
    let findings = v["findings"].as_array().cloned().unwrap_or_default();
    let below_min: Vec<&Value> = findings
        .iter()
        .filter(|f| f["rule_code"] == "E_CHARS_BELOW_MIN" && f["section_id"] == json!(leaf))
        .collect();
    assert!(
        below_min.is_empty(),
        "合稿过 80% 门后不得再报 E_CHARS_BELOW_MIN（门按合稿，不按单段）: {below_min:?}"
    );

    // ---- ⑧ 整稿向后兼容 + 账本互斥：draft_response_section 不同内容 → 账本重置单段
    let r = ok_result(call(&mut svc2, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "generator": "mock",
    })));
    assert_eq!(r["section_status"], json!("DRAFTED"), "整稿路径照常工作（向后兼容）");
    let after_full = ok_result(call(&mut svc2, "list_section_segments", json!({
        "project_id": &pid, "section_id": &leaf,
    })));
    let segs_after = after_full["segments"].as_array().unwrap();
    assert_eq!(segs_after.len(), 1, "整稿不同内容 → 账本重置单段（陈账不保留）");
    assert_eq!(segs_after[0]["seq"], json!(1), "重置段 seq=1");
    let content_now = svc2
        .store()
        .sections()
        .iter()
        .find(|n| n.id == leaf)
        .unwrap()
        .content
        .clone()
        .unwrap();
    assert_eq!(segs_after[0]["chars"].as_u64().unwrap(), content_now.chars().count() as u64,
        "重置段 = 整稿全文（账本与正文一致）");

    // 同内容整稿重提（幂等 no-op）→ 账本不动
    let r2 = ok_result(call(&mut svc2, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "generator": "mock",
    })));
    assert_eq!(r2["section_status"], json!("DRAFTED"));
    let still_one = ok_result(call(&mut svc2, "list_section_segments", json!({
        "project_id": &pid, "section_id": &leaf,
    })));
    assert_eq!(still_one["segments"].as_array().unwrap().len(), 1, "同内容 no-op 不动账本");

    let _ = seg1;
    let _ = std::fs::remove_dir_all(&ws);
}

// ---------------------------------------------------------------- 错误面与状态门

#[test]
fn segment_tools_error_surface() {
    if !corpus_ready("segments-errors") {
        return;
    }
    let (mut svc, pid, ws) = prepared("seg-errors");
    let (leaf, _rid) = target_leaf(&svc);

    // 未知章节 → E_NOTFOUND
    let e = err_text(&call(&mut svc, "append_section_segment", json!({
        "project_id": &pid, "section_id": "SEC-9999", "content": "x",
    })));
    assert!(e.contains("E_NOTFOUND"), "{e}");

    // 空白段 → E_UNPROCESSABLE
    let e = err_text(&call(&mut svc, "append_section_segment", json!({
        "project_id": &pid, "section_id": &leaf, "content": "   ",
    })));
    assert!(e.contains("E_UNPROCESSABLE") && e.contains("空白"), "{e}");

    // 结构节点（父）→ E_UNPROCESSABLE
    let parent = svc
        .store()
        .sections()
        .iter()
        .find(|n| svc.store().sections().iter().any(|c| c.parent_id.as_deref() == Some(n.id.as_str())))
        .map(|n| n.id.clone())
        .expect("夹具必有结构节点");
    let e = err_text(&call(&mut svc, "append_section_segment", json!({
        "project_id": &pid, "section_id": &parent, "content": "x",
    })));
    assert!(e.contains("E_UNPROCESSABLE") && e.contains("结构节点"), "{e}");

    // rewrite 不存在的段 → E_NOTFOUND
    let e = err_text(&call(&mut svc, "rewrite_section_segment", json!({
        "project_id": &pid, "segment_id": "SEG-9999", "content": "x",
    })));
    assert!(e.contains("E_NOTFOUND"), "{e}");

    // list 未知章节 → E_NOTFOUND
    let e = err_text(&call(&mut svc, "list_section_segments", json!({
        "project_id": &pid, "section_id": "SEC-9999",
    })));
    assert!(e.contains("E_NOTFOUND"), "{e}");

    // 状态门：DRAFT 态项目（无章节树）调分段工具 → E_STATE
    let ws2 = seg_ws("seg-state");
    let mut svc2 = BidService::open(&ws2).unwrap().with_fs_root(fixtures());
    let pid2 = ok_result(call(&mut svc2, "create_bid_project", json!({
        "name": "状态门测试", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": "seg-state",
    })))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    let e = err_text(&call(&mut svc2, "append_section_segment", json!({
        "project_id": &pid2, "section_id": "SEC-0001", "content": "x",
    })));
    assert!(e.contains("E_STATE") || e.contains("E_NOTFOUND"), "DRAFT 态分段工具被状态门拦: {e}");

    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&ws2);
}
