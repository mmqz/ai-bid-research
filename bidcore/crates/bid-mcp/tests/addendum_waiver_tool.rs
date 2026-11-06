//! addendum_waiver_tool.rs —— #184 补遗分级阻塞的 MCP 面契约测试。
//!
//! 覆盖：
//! - `record_addendum_waiver` 工具：happy path（记录 + 全量可查 + 就绪度
//!   回显）、负例（未知 file_id / 旧 principal 参数 / 伪造·错域 receipt /
//!   DRAFT 越级——#182 receipt 验签面）；
//! - `draft_response_section` 起草闸：部分补遗损坏 → E_UNPROCESSABLE
//!   （逐件点名）→ 逐件 waiver 后放行（验收 A→C 的 MCP 全链）；
//! - `import_tender_files` 响应接线：`addendum_readiness` 块 +
//!   `ADDENDUM_NOT_READY` 告警（真件导入，截断 docx 作坏补遗——确定性
//!   失败面与 import_tender_files_tool.rs 同款）。
//!
//! 并行安全：全部经 `dispatch_with` 注入独立 `BidService`。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
        });
    // #183 围栏：词法 `..` 会被围栏预拒——受信夹具根统一 canonical 绝对路径
    std::fs::canonicalize(&p).unwrap_or(p)
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

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// 截断 docx（真实 word-complex.docx 前 1200 字节——EOCD 缺失 → zip 防护
/// 拒绝，确定性失败面；import_tender_files_tool.rs 同款手法）。生成于
/// 受信根内（#183——「用户上传区」语义，坏件走 R-01 逐件失败）。
fn truncated_addendum_at(root: &std::path::Path, tag: &str) -> PathBuf {
    let src = fixtures().join("engine-borrowed/TR2/word-complex.docx");
    if !src.exists() {
        return src; // 调用方用 skip_if_fixture_missing 前置守卫
    }
    let p = root.join(format!("add_waiver_trunc_{tag}.docx"));
    let bytes = std::fs::read(&src).expect("word-complex.docx 夹具缺失");
    std::fs::write(&p, &bytes[..1200]).unwrap();
    p
}

/// 造一个「MATRIX_READY + 建纲 + 素材齐」但补遗**部分损坏**的服务态
/// （draft_tool.rs svc_ready 的 #184 变体：两份补遗一份 Ok 一份 Failed）。
fn svc_with_partial_bad_addendum(tag: &str) -> (BidService, String, String, String) {
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "#184 补遗分级测试", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": tag,
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", &format!("sha-aw-t-{tag}"), 10, 1, 50, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    // 两份补遗：一份就绪、一份损坏（3 份坏 1 份的最小化形态——部分失败）
    svc.store_mut().upsert_file(
        "C-补遗01.docx", "补遗", &format!("sha-aw-a1-{tag}"), 5, 0, 20, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    let (fid_bad, _) = svc
        .store_mut()
        .upsert_file(
            "C-补遗02.pdf", "补遗", &format!("sha-aw-a2-{tag}"), 0, 0, 0, 0,
            bid_domain::ParseStatus::Failed, "ToUnicode CMap 缺失：文本层无法解码",
        )
        .unwrap();
    svc.set_requirements(vec![bid_domain::Requirement {
        id: "REQ-0001".into(),
        category: "资格条件".into(),
        clause_no: None,
        anchors: vec![bid_domain::Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第二章/1.1".into(),
            excerpt: "投标人须具备电子与智能化工程专业承包二级及以上资质".into(),
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }]).unwrap();
    let (n_sec, _, _) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 3_000, max_reqs_per_leaf: 2 })
        .unwrap();
    assert!(n_sec >= 1);
    // 素材面：资格条件须可用资质 + 矩阵响应（否则 readiness 的其它 Blocker
    // 会混进断言——本套件隔离 L3 补遗面）
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: "电子与智能化工程专业承包二级资质证书".into(),
        source_file: "EV-aw.pdf".into(),
        source_sha256: format!("sha-aw-ev-{tag}"),
        locator: EngineLocator::DocxParagraph { paragraph_index: 3, text_fingerprint: "fnv64:aw".into() },
        fingerprint: "fnv64:aw".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某科技有限公司".into(),
        scope: "电子与智能化工程".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    svc.store_mut().bind_evidence(&eid, "REQ-0001", "2026-12-31").unwrap();
    let _ = svc.store_mut().build_matrix().unwrap();
    let parents: std::collections::HashSet<&str> = svc.store().sections().iter()
        .filter_map(|n| n.parent_id.as_deref()).collect();
    let leaf = svc.store().sections().iter()
        .find(|n| !parents.contains(&n.id[..])).unwrap().id.clone();
    (svc, pid, leaf, fid_bad)
}

// ---------------------------------------------------------------- 1. 参数与门校验

#[test]
fn waiver_tool_param_and_gate_validation() {
    // 缺参 / 空白 → E_UNPROCESSABLE（先于路由——参数校验面）
    let mut svc = BidService::new().with_fs_root(fixtures());
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "参数面", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "aw-param",
    })));
    svc.store_mut().upsert_file(
        "A-招标文件.docx", "招标文件", "sha-aw-p1", 1, 0, 5, 0,
        bid_domain::ParseStatus::Ok, "",
    ).unwrap();
    let pid = svc.store().project().unwrap().id.clone();
    // #182：宿主签发面（测试进程即宿主——持 svc 即持钥，内存态密钥不落盘）
    let r_tf2 = svc.issue_approval_receipt("aw-lead", "addendum_waiver:TF-0002");
    let r_tf3 = svc.issue_approval_receipt("aw-lead", "addendum_waiver:TF-0003");
    // 伪造样本：真 receipt 篡改末段 mac（验签必拒——Agent 拿不到宿主密钥）
    let mut r_forged = svc.issue_approval_receipt("aw-lead", "addendum_waiver:TF-0002");
    let last = if r_forged.ends_with('0') { "1" } else { "0" };
    r_forged.replace_range(r_forged.len() - 1.., last);
    let cases: Vec<(Value, &str, &str)> = vec![
        (json!({ "project_id": &pid }), "E_UNPROCESSABLE", "缺 file_id/reason/receipt"),
        (json!({ "project_id": &pid, "file_id": "TF-0002", "reason": "r" }), "E_UNPROCESSABLE", "缺 receipt"),
        // #182：旧 principal 参数显式拒（参数面先于验签面——防假成功）
        (json!({ "project_id": &pid, "file_id": "TF-0002", "reason": "r", "principal": "human:x" }), "E_UNPROCESSABLE", "旧 principal 参数"),
        (json!({ "project_id": &pid, "file_id": "TF-0002", "reason": "  ", "receipt": r_tf2 }), "E_UNPROCESSABLE", "空白 reason"),
        (json!({ "project_id": &pid, "file_id": "TF-0002", "reason": "r", "receipt": r_forged }), "E_UNPROCESSABLE", "伪造 receipt（坏签名）"),
        // 错域 receipt：签给 TF-0003 的 receipt 免不了 TF-0002（scope 绑定）
        (json!({ "project_id": &pid, "file_id": "TF-0002", "reason": "r", "receipt": r_tf3 }), "E_UNPROCESSABLE", "错域 receipt（scope 绑定）"),
        // 未知 file_id：域层 ENotFound（对不存在的件做确认 = 凭空免责——
        // receipt 本身对 TF-9999 签发合法，验签过后由域层拦）
        (json!({ "project_id": &pid, "file_id": "TF-9999", "reason": "r",
                  "receipt": svc.issue_approval_receipt("aw-lead", "addendum_waiver:TF-9999") }),
         "E_NOTFOUND", "未知 file_id"),
        (json!({ "project_id": "PRJ-9999", "file_id": "TF-0002", "reason": "r",
                  "receipt": svc.issue_approval_receipt("aw-lead", "addendum_waiver:TF-0002") }),
         "E_NOTFOUND", "未知 project"),
    ];
    for (args, want_code, why) in cases {
        let reply = call(&mut svc, "record_addendum_waiver", args);
        assert_eq!(err_code(&reply), want_code, "{why}：{reply:?}");
        assert!(
            svc.store().addendum_waivers().is_empty(),
            "{why} 拒绝路径零副作用"
        );
    }

    // DRAFT 态（无文件）→ E_STATE（FilesImported+ 才放行——与 AddendumWaiver
    // 门「导入后任意时刻」同窗口）
    let mut draft_svc = BidService::new().with_fs_root(fixtures());
    ok_result(call(&mut draft_svc, "create_bid_project", json!({
        "name": "草稿态", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "aw-draft",
    })));
    let draft_pid = draft_svc.store().project().unwrap().id.clone();
    let r_draft = draft_svc.issue_approval_receipt("aw-lead", "addendum_waiver:TF-0001");
    let reply = call(&mut draft_svc, "record_addendum_waiver", json!({
        "project_id": &draft_pid, "file_id": "TF-0001", "reason": "r", "receipt": r_draft,
    }));
    assert_eq!(err_code(&reply), "E_STATE", "DRAFT 态必须 E_STATE");
}

// ---------------------------------------------------------------- 2. A→C 全链（draft 闸）

#[test]
fn draft_blocks_on_partial_bad_addendum_until_per_file_waiver() {
    let (mut svc, pid, leaf, fid_bad) =
        svc_with_partial_bad_addendum("aw-chain");

    // ---- A：起草被拒，E_UNPROCESSABLE 且逐件点名（部分失败也阻塞）
    let reply = call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "instructions": "#184 验收 A",
    }));
    match &reply {
        ToolReply::Err { error, .. } => {
            assert_eq!(error.code(), "E_UNPROCESSABLE");
            let msg = error.to_string();
            assert!(msg.contains("C-补遗02.pdf"), "拒稿理由要点名坏件：{msg}");
            assert!(msg.contains("CMap"), "拒稿理由要点明原因：{msg}");
            assert!(!msg.contains("C-补遗01.docx"), "好件不进拒稿理由：{msg}");
        }
        ToolReply::Ok(o) => panic!("部分补遗损坏必须拒稿（#184 A 验收），实际成功: {}", o.result),
    }

    // ---- C：逐件 waiver（MCP 工具面）→ 放行 + 记录可查
    //（#182：receipt 件级签域 addendum_waiver:{file_id}——宿主签发）
    let r_waiver = svc.issue_approval_receipt("184-lead", &format!("addendum_waiver:{fid_bad}"));
    let w = ok_result(call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid_bad,
        "reason": "已人工核对纸质原件：本件仅重申开标地点，条款无实质变化",
        "receipt": r_waiver,
    })));
    assert_eq!(w["recorded"]["file_id"], json!(&fid_bad));
    assert!(w["recorded"]["at"].as_u64().unwrap() > 0, "落钟在座");
    // 可查面：全部确认记录（验收 C 的「记录可查」）
    let ws = w["addendum_waivers"].as_array().unwrap();
    assert_eq!(ws.len(), 1);
    assert_eq!(ws[0]["file_id"], json!(&fid_bad));
    assert_eq!(ws[0]["principal"], json!("human:184-lead"));
    assert!(ws[0]["reason"].as_str().unwrap().contains("人工核对"));
    // #182 可信身份回显：actor_type=Human + HMAC 认证方式标记
    assert_eq!(w["recorded"]["actor"]["actor_type"], json!("human"), "actor_type=Human: {w}");
    assert_eq!(
        w["recorded"]["actor"]["authentication_method"], json!("hmac_receipt_v1"),
        "认证方式标记: {w}"
    );
    // 就绪度回显：坏件 waived=true，不再构成阻塞
    let nr = w["addendum_readiness"]["not_ready"].as_array().unwrap();
    assert_eq!(nr.len(), 1, "报告面仍含该件（审计面）：{nr:?}");
    assert_eq!(nr[0]["waived"], json!(true));
    assert_eq!(nr[0]["readiness"], json!("FAILED"));

    // waiver 后起草放行（mock 生成器——draft 成功即闸已开）
    let d = ok_result(call(&mut svc, "draft_response_section", json!({
        "project_id": &pid, "section_id": &leaf, "instructions": "#184 验收 C",
    })));
    assert!(d["section_id"].is_string() || d["status"].is_string(), "起草成功面：{d}");
}

/// 好补遗真件（corpus-R 澄清件——与主文件不同 sha，解析 Ok；名字含
/// 「澄清」正是 is_addendum_file 名字面的设计场景）。
fn good_addendum_docx() -> PathBuf {
    fixtures().join("corpus-R/D6-revisions/R6a-纯插入澄清.docx")
}

// ---------------------------------------------------------------- 3. import 响应接线

#[test]
fn import_response_carries_addendum_readiness_block() {
    let word_complex = fixtures().join("engine-borrowed/TR2/word-complex.docx");
    let good_add = good_addendum_docx();
    if bid_parse::env_preflight::skip_if_fixture_missing(
        "import_response_carries_addendum_readiness_block",
        &[&word_complex, &good_add],
    ) {
        return;
    }
    // #183：受信上传区——好件拷入（同 sha），坏件生成在内（R-01 逐件失败）
    let root = std::env::temp_dir().join(format!("aw_uploads_import_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let wc_up = root.join("word-complex.docx");
    std::fs::copy(&word_complex, &wc_up).expect("好件拷入受信根");
    let ga_up = root.join("R6a-纯插入澄清.docx");
    std::fs::copy(&good_add, &ga_up).expect("好补遗拷入受信根");
    let mut svc = BidService::new().with_fs_root(&root);
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "导入就绪度接线", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "aw-import",
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    let bad = truncated_addendum_at(&root, "import");
    // 一件好补遗（真件 Ok）+ 一件坏补遗（截断 docx → Failed）。好补遗必须
    // 与主文件**不同 sha**（同内容不同名会被 sha256 幂等去重——补遗件
    // 就不在库了，清单会漏计）。
    let reply = call(&mut svc, "import_tender_files", json!({
        "project_id": &pid,
        "files": [
            { "upload_ref": wc_up.display().to_string(), "filename": "A-招标文件.docx", "kind": "招标文件" },
            { "upload_ref": ga_up.display().to_string(), "filename": "C-补遗01.docx", "kind": "补遗" },
            { "upload_ref": bad.display().to_string(), "filename": "C-补遗02-坏.docx", "kind": "补遗" },
        ],
    }));
    // 坏补遗 → E_PARTIAL（data 随错误携带——含 addendum_readiness 块）
    let data = match &reply {
        ToolReply::Err { error, .. } => {
            assert_eq!(error.code(), "E_PARTIAL", "截断 docx 必须部分失败：{error}");
            serde_json::to_value(error).unwrap()["data"].clone()
        }
        ToolReply::Ok(o) => panic!("截断补遗在批必须 E_PARTIAL，实际成功: {}", o.result),
    };
    let ar = &data["addendum_readiness"];
    assert_eq!(ar["total"], json!(2), "两件补遗在库：{ar}");
    assert_eq!(ar["ready"], json!(1), "好补遗就绪");
    let nr = ar["not_ready"].as_array().unwrap();
    assert_eq!(nr.len(), 1, "坏补遗进清单：{ar}");
    assert_eq!(nr[0]["name"], json!("C-补遗02-坏.docx"));
    assert_eq!(nr[0]["readiness"], json!("FAILED"));
    assert_eq!(nr[0]["waived"], json!(false));
    assert!(nr[0]["file_id"].as_str().unwrap().starts_with("TF-"), "file_id 在场");
    // 告警面：ADDENDUM_NOT_READY 点名 + 指路 record_addendum_waiver
    let warnings = data["warnings"].as_array().unwrap();
    let hit = warnings.iter().find_map(|w| {
        w.as_str()
            .and_then(|s| s.starts_with("ADDENDUM_NOT_READY").then_some(s.to_string()))
    }).expect("必须有 ADDENDUM_NOT_READY 告警");
    assert!(hit.contains("C-补遗02-坏.docx"), "告警点名坏件：{hit}");
    assert!(hit.contains("record_addendum_waiver"), "告警指路确认工具：{hit}");

    // 拿清单里的 file_id 走 MCP waiver → 该件 waived 翻转（Agent 闭环）
    let fid = nr[0]["file_id"].as_str().unwrap().to_string();
    let r_imp = svc.issue_approval_receipt("import-lead", &format!("addendum_waiver:{fid}"));
    let w = ok_result(call(&mut svc, "record_addendum_waiver", json!({
        "project_id": &pid, "file_id": &fid,
        "reason": "坏件已人工核对，无实质条款变化", "receipt": r_imp,
    })));
    let nr2 = w["addendum_readiness"]["not_ready"].as_array().unwrap();
    assert_eq!(nr2[0]["waived"], json!(true), "确认后 waived 翻转：{nr2:?}");
}

#[test]
fn import_response_all_ready_has_empty_not_ready() {
    // 全部 Ready → 清单空、无告警（D 验收在 import 面的形态）
    let word_complex = fixtures().join("engine-borrowed/TR2/word-complex.docx");
    let good_add = good_addendum_docx();
    if bid_parse::env_preflight::skip_if_fixture_missing(
        "import_response_all_ready_has_empty_not_ready",
        &[&word_complex, &good_add],
    ) {
        return;
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "全就绪导入", "bid_deadline": "2026-12-31", "mode": "明标", "external_key": "aw-import-ok",
    })))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    let r = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": &pid,
        "files": [
            { "upload_ref": word_complex.display().to_string(), "filename": "A-招标文件.docx", "kind": "招标文件" },
            { "upload_ref": good_add.display().to_string(), "filename": "C-补遗01.docx", "kind": "补遗" },
        ],
    })));
    let ar = &r["addendum_readiness"];
    assert_eq!(ar["total"], json!(1));
    assert_eq!(ar["ready"], json!(1));
    assert_eq!(ar["not_ready"].as_array().unwrap().len(), 0, "全就绪 → 空清单：{ar}");
    assert!(
        !r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap_or("").starts_with("ADDENDUM_NOT_READY")),
        "全就绪不得有 ADDENDUM_NOT_READY 告警"
    );
}
