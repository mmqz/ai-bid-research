//! #217（#185 缺口 2）验收：证据正文链路（MCP 工具面全链）。
//!
//! 覆盖 issue 验收 E/F 的工具面半边：
//! - **register 出参**：verification_tier / excerpt / excerpt_locator 随回执
//!   返回（CONTENT_MATCHES 带摘录；FILE_EXISTS 摘录 null）；
//! - **search_evidence 正文检索**：正文含词但标题/范围/主体/客户全不含 →
//!   命中（semantic 通道检索面已含 excerpt——#217）；
//! - **hits 出参**：excerpt / excerpt_locator / verification_tier 三面在场；
//! - **list_evidence**：行含 verification_tier。
//!
//! 全部用例走 `dispatch_with`（参数校验 → 状态机 → 服务层四态裁决 →
//! 域层登记/检索），不绕过壳层直测内部函数。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

// ---------------------------------------------------------------- 调用助手

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn fixtures_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_evidence(rel: &str) -> PathBuf {
    fixtures_root().join("corpus-A/files/evidence").join(rel)
}

fn fp_of(text: &str) -> String {
    format!("fnv64:{}", bid_domain::linkage::text_fingerprint(text))
}

/// PDF 页文本（两条解析路前缀兼容——poppler `第1页 行1` / 兜底 `第1 页 > 第1 行`）。
fn pdf_page_text(svc: &BidService, sha: &str, page: u32) -> String {
    let parsed = svc
        .parsed()
        .iter()
        .find(|p| p.sha256 == sha)
        .unwrap_or_else(|| panic!("解析产物缺 sha={sha}"));
    let prefixes = [format!("第{page}页"), format!("第{page} 页")];
    parsed
        .blocks
        .iter()
        .filter(|b| prefixes.iter().any(|p| b.locator.starts_with(p.as_str())))
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 建项 + 导入（A-招标文件.docx + C-补遗文件.pdf + EV-03 业绩合同 +
/// EV-13 安全生产许可证）+ 抽取 + 建纲 → (svc, pid, EV-03 fid, EV-13 fid)。
/// register_evidence 需要 MATRIX_READY（G2 状态门）；C 补遗清 readiness
/// 的「未发现补遗」阻塞（有补遗且解析成功 = READY，无需豁免）；
/// A 件是条款源（EV 件不产生 REQ，抽取后条款集为空会卡状态机）。
fn matrix_ready_store(tag: &str) -> (BidService, String, String, String) {
    let mut svc = BidService::new().with_fs_root(fixtures_root());
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": format!("#217 正文链路 {tag}"), "bid_deadline": "2026-11-30",
                "mode": "明标", "external_key": format!("evidence-content-{tag}") }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = fixtures_root().join("synth-tender/招标文件/A-招标文件.docx");
    let c = fixtures_root().join("synth-tender/招标文件/C-补遗文件.pdf");
    let ev03 = corpus_evidence("EV-03-类似业绩合同-智慧园区弱电工程.pdf");
    let ev13 = corpus_evidence("EV-13-安全生产许可证.pdf");
    for f in [&a, &c, &ev03, &ev13] {
        if !f.is_file() {
            panic!("夹具缺失: {}", f.display());
        }
    }
    let import = ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [
            { "upload_ref": a.display().to_string(),
              "filename": "A-招标文件.docx", "kind": "招标文件" },
            { "upload_ref": c.display().to_string(),
              "filename": "C-补遗文件.pdf", "kind": "补遗" },
            { "upload_ref": ev03.display().to_string(),
              "filename": "EV-03-类似业绩合同-智慧园区弱电工程.pdf", "kind": "附件" },
            { "upload_ref": ev13.display().to_string(),
              "filename": "EV-13-安全生产许可证.pdf", "kind": "附件" } ] }),
    ));
    assert_eq!(import["totals"]["parse_failed"], json!(0), "四件应全成功: {import:?}");
    let fid = |name: &str| {
        import["per_file"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["filename"].as_str() == Some(name))
            .and_then(|p| p["file_id"].as_str())
            .unwrap()
            .to_string()
    };
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    ok_result(call(&mut svc, "build_outline", json!({ "project_id": &pid })));
    (
        svc,
        pid,
        fid("EV-03-类似业绩合同-智慧园区弱电工程.pdf"),
        fid("EV-13-安全生产许可证.pdf"),
    )
}

/// 登记页锚证据（「读过文件的调用方」视角：先从解析产物取页文本与指纹）。
fn register_page_anchor(
    svc: &mut BidService,
    pid: &str,
    fid: &str,
    sha: &str,
    page: u32,
    title: &str,
    scope: &str,
) -> Value {
    let page_text = pdf_page_text(svc, sha, page);
    ok_result(call(
        svc,
        "register_evidence",
        json!({
            "project_id": pid, "kind": "performance", "title": title,
            "source_file_id": fid, "subject_name": "投标人甲", "scope": scope,
            "locator": { "kind": "pdf_page_text", "page": page,
                         "text_fingerprint": fp_of(&page_text) },
        }),
    ))
}

// ---------------------------------------------------------------- 验收 E：正文检索

/// 验收 E（#185 原文口径）：`search_evidence` 按正文实词检索——
/// 「综合布线」只出现在 EV-03 的**页文本**里，登记的标题/范围/主体/客户
/// 全不含 → 命中该证据且仅命中它（对照组 EV-13 正文与标题均无该词）。
#[test]
fn search_hits_evidence_by_body_text_not_title() {
    let (mut svc, pid, fid_ev03, fid_ev13) = matrix_ready_store("body-search");
    let sha03 = svc.store().files().iter().find(|f| f.id == fid_ev03).unwrap().sha256.clone();
    let sha13 = svc.store().files().iter().find(|f| f.id == fid_ev13).unwrap().sha256.clone();

    // 正文实词预检：EV-03 首页含「综合布线」，EV-13 不含（夹具语义防线）
    let page03 = pdf_page_text(&svc, &sha03, 1);
    assert!(page03.contains("综合布线"), "EV-03 首页正文含「综合布线」: {page03}");
    let page13 = pdf_page_text(&svc, &sha13, 1);
    assert!(!page13.contains("综合布线"), "EV-13 正文不含该词（对照组干净）");

    // 登记：标题/范围/主体/客户四面都不含「综合布线」
    let r03 = register_page_anchor(
        &mut svc, &pid, &fid_ev03, &sha03, 1,
        "类似项目业绩（智慧园区）", "详见原件",
    );
    let ev03_id = r03["ev_id"].as_str().unwrap().to_string();
    let r13 = register_page_anchor(
        &mut svc, &pid, &fid_ev13, &sha13, 1,
        "安全许可证明", "建筑施工",
    );
    let ev13_id = r13["ev_id"].as_str().unwrap().to_string();
    // 四面干净性防线（登记文案不得污染检索面）
    for (tag, t) in [("title03", "类似项目业绩（智慧园区）"), ("title13", "安全许可证明")] {
        assert!(!t.contains("综合布线"), "{tag} 不得含检索词");
    }

    // 检索：semantic 通道（mode=semantic——隔离 exact 面，纯验正文命中）
    let r = ok_result(call(
        &mut svc,
        "search_evidence",
        json!({ "project_id": &pid, "query": "综合布线 网络安防", "mode": "semantic" }),
    ));
    let ids: Vec<String> = r["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["ev_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        ids,
        vec![ev03_id.clone()],
        "正文含词但标题不含 → 恰命中 EV-03（对照 EV-13 不命中）: {r:?}"
    );
    // hit 的 match_type = semantic（非 exact——不是结构化字段命中）
    assert_eq!(r["hits"][0]["match_type"].as_str(), Some("semantic"));
    // hit 出参三面在场（#217）
    let h = &r["hits"][0];
    assert_eq!(h["verification_tier"].as_str(), Some("CONTENT_MATCHES"));
    assert!(
        h["excerpt"].as_str().unwrap_or("").contains("综合布线"),
        "hit 携带正文摘录: {h:?}"
    );
    assert!(
        h["excerpt_locator"].as_str().unwrap_or("").contains("第 1 页"),
        "hit 携带摘录定位: {h:?}"
    );
    // content 面含摘录（检索文案直通）
    assert!(
        h["content"].as_str().unwrap_or("").contains("综合布线"),
        "content 含正文实料: {h:?}"
    );
    assert_eq!(
        r["trace"]["content_face"].as_str(),
        Some("title+scope+subject+client+excerpt（#217：正文摘录进检索面）")
    );

    // 反向防线：检索 EV-13 的正文词「建筑施工」→ 只命中 EV-13
    let r2 = ok_result(call(
        &mut svc,
        "search_evidence",
        json!({ "project_id": &pid, "query": "建筑施工", "mode": "semantic" }),
    ));
    let ids2: Vec<String> = r2["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["ev_id"].as_str().unwrap().to_string())
        .collect();
    // 「建筑施工」在 EV-13 的 scope 里也有（登记文案）——允许 13 命中；
    // 但 EV-03 的任何面都不含该词，不得命中
    assert!(!ids2.contains(&ev03_id), "EV-03 任何面不含「建筑施工」: {ids2:?}");
    assert!(ids2.contains(&ev13_id), "EV-13 命中（scope+正文）: {ids2:?}");
}

/// register 出参四态面：CONTENT_MATCHES 带 excerpt/excerpt_locator；
/// FILE_EXISTS（文件级锚）摘录 null。
#[test]
fn register_reply_carries_tier_and_excerpt() {
    let (mut svc, pid, fid_ev03, _) = matrix_ready_store("reply");
    let sha03 = svc.store().files().iter().find(|f| f.id == fid_ev03).unwrap().sha256.clone();

    // ---- pdf 页锚：CONTENT_MATCHES + 摘录 + 定位
    let r = register_page_anchor(
        &mut svc, &pid, &fid_ev03, &sha03, 1,
        "类似项目业绩（出参面）", "详见原件",
    );
    assert_eq!(r["verification_tier"].as_str(), Some("CONTENT_MATCHES"), "页锚 → 内容级: {r:?}");
    let excerpt = r["excerpt"].as_str().unwrap_or_default();
    assert!(
        excerpt.contains("合同金额") && excerpt.contains("260"),
        "回执摘录含实料（金额）: {r:?}"
    );
    assert_eq!(
        r["excerpt_locator"].as_str(),
        Some("《EV-03-类似业绩合同-智慧园区弱电工程.pdf》第 1 页"),
        "回执摘录定位: {r:?}"
    );

    // ---- 文件级锚：FILE_EXISTS + 摘录 null
    let r2 = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "contract",
            "title": "框架协议（文件级锚出参面）",
            "source_file_id": &fid_ev03, "subject_name": "投标人甲",
        }),
    ));
    assert_eq!(r2["verification_tier"].as_str(), Some("FILE_EXISTS"), "文件级锚: {r2:?}");
    assert_eq!(r2["excerpt"], Value::Null, "无摘录（正文未定位）: {r2:?}");
    assert_eq!(r2["excerpt_locator"], Value::Null, "无定位: {r2:?}");
}

/// list_evidence 行含 verification_tier（台账清单四态可见）。
#[test]
fn list_evidence_rows_carry_verification_tier() {
    let (mut svc, pid, fid_ev03, _) = matrix_ready_store("list");
    let sha03 = svc.store().files().iter().find(|f| f.id == fid_ev03).unwrap().sha256.clone();
    let r = register_page_anchor(
        &mut svc, &pid, &fid_ev03, &sha03, 1,
        "类似项目业绩（清单面）", "详见原件",
    );
    let ev_id = r["ev_id"].as_str().unwrap();
    let r2 = ok_result(call(
        &mut svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "contract",
            "title": "框架协议（清单·文件级锚）",
            "source_file_id": &fid_ev03, "subject_name": "投标人甲",
        }),
    ));
    let ev2 = r2["ev_id"].as_str().unwrap();

    let list = ok_result(call(
        &mut svc,
        "list_evidence",
        json!({ "project_id": &pid }),
    ));
    let items = list["items"].as_array().unwrap();
    let find = |id: &str| items.iter().find(|i| i["id"].as_str() == Some(id)).unwrap_or_else(|| panic!("{id} 不在清单: {list:?}"));
    assert_eq!(find(ev_id)["verification_tier"].as_str(), Some("CONTENT_MATCHES"));
    assert_eq!(find(ev2)["verification_tier"].as_str(), Some("FILE_EXISTS"));
}

/// 验收 F 的工具面链路（draft 出参佐证摘录经 mock 模板进正文）：
/// 页锚证据绑定到 REQ → draft（mock）→ 正文含摘录实料。
/// （EvBrief 单元面断言见 bid-tools/tests/draft_section.rs——此处走全链。）
#[test]
fn draft_output_carries_bound_evidence_excerpt() {
    let (mut svc, pid, fid_ev03, _) = matrix_ready_store("draft");
    let sha03 = svc.store().files().iter().find(|f| f.id == fid_ev03).unwrap().sha256.clone();
    let r = register_page_anchor(
        &mut svc, &pid, &fid_ev03, &sha03, 1,
        "类似项目业绩（起草链）", "详见原件",
    );
    let ev_id = r["ev_id"].as_str().unwrap().to_string();

    // 抽取后应有 REQ；取矩阵首行（requirement.id + draft_section_id）
    let matrix = ok_result(call(
        &mut svc,
        "get_compliance_matrix",
        json!({ "project_id": &pid }),
    ));
    let first_row = matrix["rows"]
        .as_array()
        .and_then(|rows| rows.first())
        .cloned()
        .unwrap_or_else(|| panic!("矩阵应有行: {matrix:?}"));
    let first_req = first_row["requirement"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("行应含 requirement.id: {first_row:?}"))
        .to_string();
    let section_id = first_row["draft_section_id"]
        .as_str()
        .unwrap_or_else(|| panic!("行应含 draft_section_id: {first_row:?}"))
        .to_string();
    ok_result(call(
        &mut svc,
        "bind_evidence",
        json!({ "project_id": &pid, "ev_id": &ev_id,
                "requirement_ids": [&first_req], "bid_open_date": "2026-11-15" }),
    ));

    // 状态面（推进确认）+ 起草（mock 确定性生成）
    let _state = ok_result(call(&mut svc, "get_bid_project_state", json!({ "project_id": &pid })));
    let draft = ok_result(call(
        &mut svc,
        "draft_response_section",
        json!({ "project_id": &pid, "section_id": &section_id,
                "source_requirements": [&first_req], "evidence_ids": [&ev_id],
                "instructions": "按业绩实料响应", "generator": "mock" }),
    ));
    let sec = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == section_id)
        .unwrap();
    let content = sec.content.as_deref().unwrap_or("");
    assert!(
        content.contains(&format!("依据 [{}]", ev_id)),
        "正文引用证据: {content}"
    );
    assert!(
        content.contains("其原始凭据 《EV-03-类似业绩合同-智慧园区弱电工程.pdf》第 1 页 原文摘录如下"),
        "正文含摘录定位段（#217）: {content}"
    );
    assert!(
        content.contains("合同金额") && content.contains("260"),
        "正文含摘录实料（金额）——生成器引用的是原文而非标题: {content}"
    );
    let _ = draft; // 出参面已由 draft_tool.rs 既有用例覆盖
}
