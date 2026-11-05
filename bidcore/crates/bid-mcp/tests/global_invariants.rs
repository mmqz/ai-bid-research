//! #171 · 跨模块全局不变式（global invariants）——「模块之间对不对得上」的验收面。
//!
//! 现有测试分模块（bid-mcp / bid-tools / bid-parse 各自成册），**没人测模块之间
//! 的接缝**。历史 bug #117「表格/格式件做完了但没接线到导出链」就是这么漏的：
//! 每模块单测全绿，只有把「导入 → 抽取 → 建纲 → 证据绑定 → 起草 → 校验 →
//! 导出」整条链跑完、再拿产物跟上游账面对账，才发现产物里根本没有表格。
//!
//! 本文件只测**接缝**，不复测各模块内部语义：
//!
//! | # | 不变式 | 对账的两个面 |
//! |---|---|---|
//! | ① | outline 叶数 == 导出 docx 的 Heading2 段数 | 章节树 ↔ docx 渲染面 |
//! | ② | matrix 每个 mandatory REQ 都有正文落点或响应节 | 合规矩阵 ↔ 章节树 ↔ 已起草正文 |
//! | ③ | evidence 绑定数 == matrix 已响应数 | 证据绑定台账 ↔ 合规矩阵 |
//! | ④ | score_document by_item == 全部评分项（不漏不重） | 自评面 ↔ 条款集 |
//! | ⑤ | 导出 docx 章节标题**逐字** == outline 标题（不是包含） | 章节树 ↔ docx 渲染面 |
//!
//! 夹具纪律：只用仓库内的合成件（fixtures/synth-tender/招标文件 的 docx 两件
//! ——纯 docx 面，不依赖 poppler/pdfium/tesseract，CI 缺件也不会误 SKIP）。
//! 夹具缺失 → 响亮横幅 + return（issue #9 口径：缺件不是业务失败，绝不静默
//! 假装通过）。
//!
//! 反例纪律：五条不变式的判定面全部抽成【纯数据函数】（check_*），主链断言
//! 它们全 Ok，反例用例逐条注入缺陷断言必红——**防止断言本身空转**（一条永远
//! 不红的断言等于没有断言）。

use bid_domain::matrix::{is_mandatory, MatrixRow, ResponseStatus};
use bid_domain::outline::{OutlineNode, OutlineSpec};
use bid_domain::Requirement;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;

// ---------------------------------------------------------------- 夹具与环境

fn repo_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    // #183 围栏：词法 `..` 会被围栏预拒——受信夹具根统一 canonical 绝对路径
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn files_dir() -> PathBuf {
    repo_root().join("fixtures/synth-tender/招标文件")
}

/// 夹具在位预检（issue #9：缺件 → 响亮横幅 + 调用方 return）。
fn fixtures_ready(test_id: &str) -> bool {
    let a = files_dir().join("A-招标文件.docx");
    let b = files_dir().join("B-技术规范书.docx");
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&a, &b])
}

/// 独立工作区（每次运行唯一——并发/重跑互不污染）。
fn ws_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-gi-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// MCP 分派（写操作全部走工具面，不旁路）。
fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    // 确定性套件钉 mock（防 dev 机配了 BID_LLM_API_KEY 后 auto 走真实调用）。
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
        ToolReply::Err { error, .. } => panic!("global-invariants 链步骤失败: {error}"),
    }
}

// ---------------------------------------------------------------- docx 解包面

/// 取 docx 的 word/document.xml（「Word 打开」的机器等价面）。
fn document_xml(path: &std::path::Path) -> String {
    let bytes = std::fs::read(path).expect("docx 可读");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("docx 合法 zip");
    let mut s = String::new();
    zip.by_name("word/document.xml")
        .expect("word/document.xml 在位")
        .read_to_string(&mut s)
        .unwrap();
    s
}

/// 最小 XML 实体还原（标题里的 & < > 会被 docx-rs 转义——逐字对账前先还原）。
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// 抽取文档里全部 Heading1..4 段落的 (层级, 全文)。
///
/// 逐段取而非全文 contains——⑤ 要求【逐字】对账，contains 会让「标题被截断」
/// 与「正文里恰好出现同样字符串」两种缺陷静默通过（#127 P0-3 ① 的教训）。
fn heading_paragraphs(xml: &str) -> Vec<(u8, String)> {
    let mut out: Vec<(u8, String)> = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = xml[cursor..].find("w:val=\"Heading") {
        let at = cursor + rel;
        let lvl_at = at + "w:val=\"Heading".len();
        let lvl = xml[lvl_at..]
            .chars()
            .next()
            .and_then(|c| c.to_digit(10))
            .unwrap_or(0) as u8;
        let end = xml[at..].find("</w:p>").map(|d| at + d).unwrap_or(xml.len());
        let start = xml[..at].rfind("<w:p").unwrap_or(0);
        // 段内全部 <w:t> 文本拼接（标题可能被多 run 切分）
        let body = &xml[start..end];
        let mut text = String::new();
        let mut rest = body;
        while let Some(t) = rest.find("<w:t") {
            let after = &rest[t..];
            let open_end = after.find('>').unwrap_or(after.len());
            let close = after[open_end..].find("</w:t>").unwrap_or(after.len());
            text.push_str(&after[open_end + 1..open_end + close]);
            rest = &rest[t + open_end + close + "</w:t>".len()..];
        }
        out.push((lvl, unescape(&text)));
        cursor = end;
    }
    out
}

// ================================================================ 五条判定面
//
// 全部为【纯数据函数】（不碰服务/IO）——主链与反例用例共用同一份判定逻辑，
// 反例用例因此能证明「主链的红是真的红」。

/// ① outline 叶数 == docx Heading2 段数。
///
/// 导出层在叶章之外还会给**七件格式件**各发一个 Heading2（export::append_form
/// 的渲染口径），故对账口径 = 「扣掉格式件标题后的 Heading2 段集」与「叶标题集」
/// **逐字全等**（多重集——防重发/漏发：计数相等但错位也算破）。
fn check_leaf_vs_heading2(
    leaf_titles: &[String],
    headings: &[(u8, String)],
    form_titles: &[String],
) -> Result<(), String> {
    if leaf_titles.is_empty() {
        return Err("outline 无叶——空大纲何谈对账（夹具或建纲面已破）".into());
    }
    let mut chapter_h2: Vec<String> = headings
        .iter()
        .filter(|(lvl, _)| *lvl == 2)
        .map(|(_, t)| t.clone())
        .filter(|t| !form_titles.contains(t))
        .collect();
    let mut expect = leaf_titles.to_vec();
    chapter_h2.sort();
    expect.sort();
    if chapter_h2 != expect {
        return Err(format!(
            "① 叶↔Heading2 对账破：outline 叶 {} 个 {:?}；docx 章节 Heading2 {} 个 {:?}（已扣格式件标题 {} 个）",
            expect.len(),
            expect,
            chapter_h2.len(),
            chapter_h2,
            form_titles.len()
        ));
    }
    // 逐字 + 恰一次（多重集相等已含，此处显式给出定位信息）
    for t in leaf_titles {
        let n = chapter_h2.iter().filter(|x| *x == t).count();
        if n != 1 {
            return Err(format!("① 叶标题「{t}」在 docx 中出现 {n} 次（应为恰 1 次）"));
        }
    }
    Ok(())
}

/// ② matrix 里每个 mandatory（资格条件/否决项）REQ 都有响应节，且该节有正文落点。
///
/// mandatory 口径与 matrix::is_mandatory 同源（不另立一套——#121 P0-1 后否决项/
/// 资格条件进「资格与响应」组：**不写规则原文，但必须有响应章节**，否则合规矩阵
/// 立刻报 BLOCKER、导出门直接拒）。
fn check_mandatory_has_response(
    reqs: &[Requirement],
    rows: &[MatrixRow],
    sections: &[OutlineNode],
) -> Result<(), String> {
    let cat: HashMap<&str, &str> = reqs.iter().map(|r| (r.id.as_str(), r.category.as_str())).collect();
    let by_id: HashMap<&str, &OutlineNode> = sections.iter().map(|n| (n.id.as_str(), n)).collect();
    // 出现在别人 parent_id 里的节点 = 有子节点（父自身不承载内容）
    let has_child: HashSet<&str> = sections.iter().filter_map(|n| n.parent_id.as_deref()).collect();
    // 已起草叶（正文落点）
    let drafted_leaf: HashSet<&str> = sections
        .iter()
        .filter(|n| !has_child.contains(n.id.as_str()))
        .filter(|n| !n.content.as_deref().unwrap_or("").trim().is_empty())
        .map(|n| n.id.as_str())
        .collect();

    let mut checked = 0usize;
    for row in rows {
        let c = *cat
            .get(row.requirement_id.as_str())
            .ok_or_else(|| format!("② 矩阵行 {} 的 REQ {} 不在条款集（幽灵行）", row.id, row.requirement_id))?;
        if !is_mandatory(c) {
            continue;
        }
        checked += 1;
        let sid = row.response_section.as_deref().ok_or_else(|| {
            format!(
                "② 必选 REQ {}（{}）矩阵行 {} 无响应节（合规矩阵与章节树对不上——导出门必拒）",
                row.requirement_id, c, row.id
            )
        })?;
        let node = *by_id.get(sid).ok_or_else(|| {
            format!("② 必选 REQ {} 的响应节 {sid} 不在章节树（悬空 SEC 引用）", row.requirement_id)
        })?;
        // 正文落点：叶自身有内容，或（响应位落在父节点时）子树内有已起草叶
        let landed = if has_child.contains(node.id.as_str()) {
            sections
                .iter()
                .any(|n| n.parent_id.as_deref() == Some(sid) && drafted_leaf.contains(n.id.as_str()))
        } else {
            drafted_leaf.contains(node.id.as_str())
        };
        if !landed {
            return Err(format!(
                "② 必选 REQ {} 的响应节 {sid}（「{}」）无正文落点（章节未起草——响应位空转）",
                row.requirement_id, node.title
            ));
        }
    }
    if checked == 0 {
        return Err("② 矩阵里没有必选（资格条件/否决项）行——对账空转（夹具或抽取面已破）".into());
    }
    Ok(())
}

/// ③ evidence 绑定数 == matrix 已响应数（不漏不重）。
///
/// RESPONDED 的派生口径是「证据在绑 **且** 章节在位」（matrix.rs）——所以绑定
/// 台账与矩阵是同一件事的两个视图，逐条对得上才叫接线成功；绑了却不成
/// RESPONDED = 绑定面与响应面脱节（PARTIAL 的常态是材料齐、正文未落）。
fn check_evidence_bindings_vs_responded(
    bindings: &[(String, String)],
    rows: &[MatrixRow],
) -> Result<(), String> {
    if bindings.is_empty() {
        return Err("③ 零绑定——对账空转（夹具或绑定面已破）".into());
    }
    let bound: HashSet<&str> = bindings.iter().map(|(_, r)| r.as_str()).collect();
    let responded: Vec<&MatrixRow> = rows
        .iter()
        .filter(|r| r.status == ResponseStatus::Responded)
        .collect();
    if responded.len() != bound.len() {
        return Err(format!(
            "③ 绑定↔已响应 对账破：绑定 {} 条 REQ，矩阵 RESPONDED {} 行（多/少即接线脱节）",
            bound.len(),
            responded.len()
        ));
    }
    for r in &responded {
        if !bound.contains(r.requirement_id.as_str()) {
            return Err(format!(
                "③ 矩阵行 {} 标 RESPONDED 但 REQ {} 无绑定（凭空响应）",
                r.id, r.requirement_id
            ));
        }
        if r.evidence_ids.is_empty() {
            return Err(format!(
                "③ 矩阵行 {} 标 RESPONDED 但 evidence_ids 为空（状态派生与证据面脱节）",
                r.id
            ));
        }
    }
    // 逐条（不是逐 REQ）：绑定总条数 == 矩阵行里挂的证据 id 总数
    let carried: usize = rows.iter().map(|r| r.evidence_ids.len()).sum();
    if carried != bindings.len() {
        return Err(format!(
            "③ 绑定 {} 条 vs 矩阵行携带证据 id {carried} 个（有绑定未进矩阵行）",
            bindings.len()
        ));
    }
    Ok(())
}

/// ④ score_document 的 by_item == 全部评分项（不漏不重）。
///
/// 期望集 = **全部「评分项」类 REQ** 减去「被别的评分项引用为 parent 者」
/// （工具的 by_item 只列非硬门、非父项的叶级项——tools.rs::score_document）。
/// 「否决项」是硬门（is_hard_gate）不入列，这是语义不是缺陷。
fn check_by_item_covers_all_score_items(
    reqs: &[Requirement],
    by_item_ids: &[String],
) -> Result<(), String> {
    let items = bid_domain::scoring::derive_scoring_items(reqs);
    let parents: HashSet<&str> = items.iter().filter_map(|i| i.parent_id.as_deref()).collect();
    let expected: Vec<&str> = reqs
        .iter()
        .filter(|r| r.category == "评分项")
        .filter(|r| !parents.contains(r.id.as_str()))
        .map(|r| r.id.as_str())
        .collect();
    if expected.is_empty() {
        return Err("④ 夹具无评分项——对账空转（抽取面已破）".into());
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for id in by_item_ids {
        if !seen.insert(id.as_str()) {
            return Err(format!("④ by_item 里 {id} 重复出现（自评面重发同一项）"));
        }
    }
    let actual: HashSet<&str> = by_item_ids.iter().map(|s| s.as_str()).collect();
    let expect: HashSet<&str> = expected.iter().copied().collect();
    let missing: Vec<&str> = expect.difference(&actual).copied().collect();
    let extra: Vec<&str> = actual.difference(&expect).copied().collect();
    if !missing.is_empty() || !extra.is_empty() {
        return Err(format!(
            "④ 自评 by_item 与评分项对账破：漏 {missing:?}；多 {extra:?}（评分项 {} 条，by_item {} 条）",
            expect.len(),
            actual.len()
        ));
    }
    Ok(())
}

/// ⑤ 导出 docx 的章节标题**逐字** == outline 标题（不是包含）。
///
/// 双向：
/// - 每个 outline 节点 → docx 里恰有一个 Heading{level} 段落，全文**相等**；
/// - 每个 Heading 段落的全文 ∈ {outline 标题} ∪ 导出层追加部分（附表/格式件）。
fn check_headings_verbatim(
    nodes: &[OutlineNode],
    headings: &[(u8, String)],
    extra_allowed: &[String],
) -> Result<(), String> {
    if nodes.is_empty() {
        return Err("⑤ outline 空——对账空转".into());
    }
    for n in nodes {
        let hit = headings
            .iter()
            .filter(|(lvl, t)| *lvl == n.level && t == &n.title)
            .count();
        if hit != 1 {
            return Err(format!(
                "⑤ outline 节点 {}「{}」(Heading{}) 在 docx 中逐字命中 {hit} 次（应为恰 1）——相近段: {:?}",
                n.id,
                n.title,
                n.level,
                headings
                    .iter()
                    .filter(|(_, t)| t.contains(n.title.as_str()) || n.title.contains(t.as_str()))
                    .map(|(l, t)| format!("H{l}:{t}"))
                    .collect::<Vec<_>>()
            ));
        }
    }
    let known: HashSet<&str> = nodes.iter().map(|n| n.title.as_str()).collect();
    for (lvl, t) in headings {
        if known.contains(t.as_str()) || extra_allowed.contains(t) {
            continue;
        }
        return Err(format!(
            "⑤ docx 出现 outline 之外的 Heading{lvl} 标题「{t}」（导出层多了不认识的标题）"
        ));
    }
    Ok(())
}

// ================================================================ 主链

#[test]
fn gi_five_global_invariants() {
    if !fixtures_ready("global-invariants") {
        return; // SKIP 横幅已由 skip_if_fixture_missing 打出
    }
    let ws = ws_dir("main");
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(files_dir());

    // ---- ① 建项目（MCP §1）
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": "全局不变式对账——智慧病房物联网平台",
            "bid_deadline": "2026-11-30", "mode": "明标",
            "external_key": "gi-main",
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();

    // ---- ② 导入（纯 docx 两件——不依赖 PDF 链工具，CI 缺件也不误 SKIP）
    let specs: Vec<Value> = [
        ("A-招标文件.docx", "招标文件"),
        ("B-技术规范书.docx", "招标文件"),
    ]
    .iter()
    .map(|(n, k)| {
        json!({ "upload_ref": files_dir().join(n).display().to_string(), "filename": n, "kind": k })
    })
    .collect();
    let imp = ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": specs }),
    ));
    assert_eq!(imp["totals"]["files"].as_u64().unwrap_or(0), 2, "② 两件全落库");
    assert_eq!(imp["totals"]["parse_failed"].as_u64().unwrap_or(0), 0, "② 零解析失败（纯 docx 面）");

    // ---- ③ 抽取（MCP §3）
    let ext = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    let n_reqs = ext["counts"]["total"].as_u64().unwrap_or(0);
    assert!(n_reqs >= 30, "③ 抽取面（synth A+B 应远多于 30 条）: {n_reqs}");

    // ---- ④ 建纲 + 建矩阵（服务面——无 MCP 工具面，显式标注）
    let (n_sec, n_rows, _n_blocking) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 12_000, max_reqs_per_leaf: 6 })
        .unwrap();
    assert!(n_sec >= 6, "④ 章节树: {n_sec} 节点");
    assert!(n_rows >= 30, "④ 矩阵: {n_rows} 行");

    // ---- ⑤ 证据登记 + 绑定 + 三门（服务面）
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc
        .store_mut()
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "电子与智能化工程专业承包二级资质证书（gi）".into(),
            source_file: "GI-EV-资质证书.pdf".into(),
            source_sha256: "sha-gi-ev".into(),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 0,
                text_fingerprint: "fnv64:gi".into(),
            },
            fingerprint: "fnv64:gi".into(),
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
    // 资格/否决/技术参数全绑（#110 缺失回报闸：技术参数类也须证据佐证，否则
    // draft 拒生成；本用例的 ③ 对账也建立在这批绑定上）
    let bind_ids: Vec<String> = svc
        .store()
        .requirements()
        .iter()
        .filter(|r| r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数")
        .map(|r| r.id.clone())
        .collect();
    assert!(!bind_ids.is_empty(), "⑤ 待绑 REQ 非空");
    for rid in &bind_ids {
        svc.store_mut().bind_evidence(&eid, rid, "2026-11-30").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    // 【#131 无补遗阻塞的人工出口】基座只导入招标文件（无补遗件），
    // AddendumWaiver 门确认无补遗后起草才放行——漏记则 ⑥ 被
    // E_UNPROCESSABLE「未发现补遗/澄清文件」拒（实测踩到）。
    // #180 审核内容绑定：MatrixApproval/OutlineApproval 批矩阵/大纲，保持
    // 起草前原位；PreExportConfirm 绑定章节正文摘要，挪至全部叶起草完成后
    // 记审（见 ⑦' ——过门时摘要还是空正文态，起草后即失效）。
    for gate in [
        Gate::AddendumWaiver,
        Gate::MatrixApproval,
        Gate::OutlineApproval,
    ] {
        svc.store_mut()
            .record_review(gate, &bid_domain::TrustedPrincipal::human("gi-lead"), Decision::Approved, vec![])
            .unwrap();
    }
    assert_eq!(
        svc.store().matrix().iter().filter(|r| r.blocking).count(),
        0,
        "⑤ 绑定后 blocking 清零（否则 ⑧ 被导出门拒）"
    );

    // ---- ⑥ 逐叶起草（MCP §6）
    let parents: HashSet<String> = svc
        .store()
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.clone())
        .collect();
    let leaf_ids: Vec<String> = svc
        .store()
        .sections()
        .iter()
        .filter(|n| !parents.contains(&n.id))
        .map(|n| n.id.clone())
        .collect();
    assert!(!leaf_ids.is_empty(), "⑥ 叶非空");
    for sid in &leaf_ids {
        let r = ok_result(call(
            &mut svc,
            "draft_response_section",
            json!({
                "project_id": &pid, "section_id": sid,
                "instructions": "按招标文件逐项响应，突出同类业绩与实施方案",
            }),
        ));
        assert_eq!(r["section_status"], "DRAFTED", "⑥ 章节 {sid}");
    }

    // ---------------- 不变式 ④：自评 by_item ↔ 全部评分项 ----------------
    let score = ok_result(call(&mut svc, "score_document", json!({ "project_id": &pid })));
    let by_item = score["by_item"].as_array().expect("by_item 在场");
    let by_item_ids: Vec<String> = by_item
        .iter()
        .map(|i| i["item_id"].as_str().unwrap_or("").to_string())
        .collect();
    let reqs_snapshot = svc.store().requirements().to_vec();
    check_by_item_covers_all_score_items(&reqs_snapshot, &by_item_ids)
        .unwrap_or_else(|e| panic!("{e}"));

    // ---------------- 不变式 ②③：矩阵 ↔ 章节树/正文 ↔ 证据绑定 ----------------
    let rows = svc.store().matrix().to_vec();
    let sections_snapshot = svc.store().sections().to_vec();
    let bindings: Vec<(String, String)> = svc
        .store()
        .evidence_bindings()
        .iter()
        .map(|b| (b.evidence_id.clone(), b.requirement_id.clone()))
        .collect();
    check_mandatory_has_response(&reqs_snapshot, &rows, &sections_snapshot)
        .unwrap_or_else(|e| panic!("{e}"));
    check_evidence_bindings_vs_responded(&bindings, &rows).unwrap_or_else(|e| panic!("{e}"));

    // ---- ⑦ 校验（MCP §7）
    let val = ok_result(call(
        &mut svc,
        "validate_bid_document",
        json!({ "project_id": &pid, "scope": "all" }),
    ));
    assert_eq!(val["ok"], json!(true), "⑦ 校验未过: {}", val["errors"]);
    assert_eq!(val["counts"]["blocker"], json!(0));

    // ---- ⑦' PreExportConfirm 记审（#180：定稿内容过门——全部叶起草完成
    //      后、导出前，绑定的才是将进产物的正文摘要）
    svc.store_mut()
        .record_review(Gate::PreExportConfirm, &bid_domain::TrustedPrincipal::human("gi-lead"), Decision::Approved, vec![])
        .unwrap();

    // ---- ⑧ 导出（MCP §8）
    let exp = ok_result(call(
        &mut svc,
        "export_bid_document",
        json!({
            "project_id": &pid, "format": "docx",
            "output_path": "out/gi-投标文件-技术标.docx",
        }),
    ));
    let url = exp["artifact"]["url"].as_str().unwrap().to_string();
    assert_eq!(
        exp["chapters_written"].as_u64().unwrap(),
        leaf_ids.len() as u64,
        "⑧ 全部落章"
    );
    let forms_written = exp["forms_written"].as_u64().unwrap_or(0) as usize;
    assert!(forms_written > 0, "⑧ 格式件已接线（#117 漏接线的正是这一面）: {forms_written}");
    let form_titles: Vec<String> = exp["form_titles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap_or("").to_string())
        .collect();

    // ---------------- 不变式 ①⑤：章节树 ↔ docx 渲染面 ----------------
    let xml = document_xml(std::path::Path::new(&url));
    let headings = heading_paragraphs(&xml);
    let leaf_titles: Vec<String> = svc
        .store()
        .sections()
        .iter()
        .filter(|n| !parents.contains(&n.id))
        .map(|n| n.title.clone())
        .collect();
    check_leaf_vs_heading2(&leaf_titles, &headings, &form_titles)
        .unwrap_or_else(|e| panic!("{e}"));

    let mut extra_allowed = form_titles.clone();
    extra_allowed.push("附表".to_string());
    extra_allowed.push("投标文件格式件".to_string());
    check_headings_verbatim(&sections_snapshot, &headings, &extra_allowed)
        .unwrap_or_else(|e| panic!("{e}"));

    println!(
        "global-invariants OK：叶 {} / Heading2 {}（含格式件 {}）｜矩阵 {} 行（RESPONDED {}）｜绑定 {} 条｜评分项 by_item {}｜Heading 段 {}",
        leaf_titles.len(),
        headings.iter().filter(|(l, _)| *l == 2).count(),
        forms_written,
        rows.len(),
        rows.iter().filter(|r| r.status == ResponseStatus::Responded).count(),
        bindings.len(),
        by_item_ids.len(),
        headings.len(),
    );
    let _ = std::fs::remove_dir_all(&ws);
}

// ================================================================ 反例面
//
// 每条不变式注入一个缺陷 → 断言判定面**必红**。没有这一节，五条断言随时可能
// 退化成空转（比如 XML 提取器写错 → 恒空集 → ①⑤ 恒绿）。

fn synth_req(id: &str, category: &str) -> Requirement {
    let mut params = HashMap::new();
    if category == "评分项" {
        params.insert("score".into(), json!(10.0));
    }
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: Some("1.1".into()),
        anchors: vec![bid_domain::Anchor {
            file: "A".into(),
            kind: "para".into(),
            locator: "第一章".into(),
            excerpt: format!("{id} 条款原文（反例件）"),
        }],
        params,
        source_file: "A".into(),
    }
}

fn row(req_id: &str, section: Option<&str>, evs: &[&str], status: ResponseStatus) -> MatrixRow {
    MatrixRow {
        id: format!("CMP-{}", req_id.rsplit('-').next().unwrap_or("0000")),
        requirement_id: req_id.into(),
        response_section: section.map(String::from),
        evidence_ids: evs.iter().map(|s| s.to_string()).collect(),
        pending_evidence_ids: Vec::new(),
        // #241 裁定正交标记：反例面只测不变式判定（不消费证据标记），缺省 false
        evidence_required: false,
        evidence_missing: false,
        status,
        deviation: None,
        blocking: false,
    }
}

fn node(id: &str, title: &str, level: u8) -> OutlineNode {
    OutlineNode {
        id: id.into(),
        parent_id: None,
        title: title.into(),
        level,
        order: 1,
        requirement_ids: vec!["REQ-0001".into()],
        target_chars: 500,
        status: bid_domain::outline::SectionStatus::Drafted,
        dirty: false,
        content: None,
        content_attempt: 0,
    }
}

#[test]
fn gi_invariant_checks_reject_injected_defects() {
    // ①a 少渲染一个叶 → 必红
    assert!(
        check_leaf_vs_heading2(&["甲章".into(), "乙章".into()], &[(2u8, "甲章".into())], &[]).is_err(),
        "① 少一个 Heading2 必须红（漏章渲染）"
    );
    // ①b **包含**而非逐字（标题被截断）→ 必红（正是⑤要防的伪命中）
    assert!(
        check_leaf_vs_heading2(&["第三章 投标人须知".into()], &[(2u8, "第三章".into())], &[]).is_err(),
        "① 标题被截断（contains 会误过）必须红"
    );
    // ①c 格式件标题被正确扣减 → 绿（七件格式件各有独立 Heading2，不是叶）
    assert!(
        check_leaf_vs_heading2(
            &["甲章".into()],
            &[(2u8, "甲章".into()), (2u8, "附件一、投标函".into())],
            &["附件一、投标函".into()],
        )
        .is_ok(),
        "① 扣掉格式件标题后应绿"
    );

    // ② 必选 REQ 无响应节 → 必红
    let reqs = vec![synth_req("REQ-0001", "资格条件")];
    let rows_no_sec = vec![row("REQ-0001", None, &[], ResponseStatus::NotResponded)];
    assert!(
        check_mandatory_has_response(&reqs, &rows_no_sec, &[]).is_err(),
        "② 必选无响应节必须红"
    );
    // ②b 有响应节但该节未起草（无正文落点）→ 必红
    let rows_undrafted = vec![row("REQ-0001", Some("SEC-0002"), &["EV-0001"], ResponseStatus::Responded)];
    assert!(
        check_mandatory_has_response(&reqs, &rows_undrafted, &[node("SEC-0002", "资格与响应", 1)]).is_err(),
        "② 响应节未起草（无正文落点）必须红"
    );
    // ②c 响应节已起草 → 绿
    let mut sec_ok = node("SEC-0002", "资格与响应", 1);
    sec_ok.content = Some("我方满足 1.1 的要求，附件为营业执照复印件。".into());
    assert!(
        check_mandatory_has_response(&reqs, &rows_undrafted, &[sec_ok]).is_ok(),
        "② 响应节已起草应绿"
    );

    // ③ 绑了 2 条但矩阵只认 1 条 RESPONDED（另一条 PARTIAL）→ 必红
    let b3 = vec![
        ("EV-0001".to_string(), "REQ-0001".to_string()),
        ("EV-0001".to_string(), "REQ-0002".to_string()),
    ];
    let rows3 = vec![
        row("REQ-0001", Some("SEC-0002"), &["EV-0001"], ResponseStatus::Responded),
        row("REQ-0002", None, &["EV-0001"], ResponseStatus::Partial),
    ];
    assert!(
        check_evidence_bindings_vs_responded(&b3, &rows3).is_err(),
        "③ 绑定 2 条但只 1 条 RESPONDED 必须红"
    );
    // ③b 全成 RESPONDED → 绿
    let rows3_ok = vec![
        row("REQ-0001", Some("SEC-0002"), &["EV-0001"], ResponseStatus::Responded),
        row("REQ-0002", Some("SEC-0002"), &["EV-0001"], ResponseStatus::Responded),
    ];
    assert!(
        check_evidence_bindings_vs_responded(&b3, &rows3_ok).is_ok(),
        "③ 全成 RESPONDED 应绿"
    );
    // ③c 凭空 RESPONDED（REQ 无绑定）→ 必红
    let rows3_ghost = vec![row("REQ-0009", Some("SEC-0002"), &["EV-0001"], ResponseStatus::Responded)];
    assert!(
        check_evidence_bindings_vs_responded(
            &[("EV-0001".to_string(), "REQ-0001".to_string())],
            &rows3_ghost
        )
        .is_err(),
        "③ 无绑定却 RESPONDED 必须红"
    );

    // ④ 少一项 / 硬门入列 / 重复 → 必红；恰为全部评分项 → 绿
    let score_reqs = vec![
        synth_req("REQ-0001", "评分项"),
        synth_req("REQ-0002", "评分项"),
        synth_req("REQ-0003", "否决项"), // 硬门，不入列（语义不是缺陷）
    ];
    assert!(
        check_by_item_covers_all_score_items(&score_reqs, &["REQ-0001".to_string()]).is_err(),
        "④ 漏一项必须红"
    );
    assert!(
        check_by_item_covers_all_score_items(
            &score_reqs,
            &["REQ-0001".to_string(), "REQ-0002".to_string(), "REQ-0003".to_string()]
        )
        .is_err(),
        "④ 硬门（否决项）入列必须红"
    );
    assert!(
        check_by_item_covers_all_score_items(
            &score_reqs,
            &["REQ-0001".to_string(), "REQ-0001".to_string(), "REQ-0002".to_string()]
        )
        .is_err(),
        "④ 重复必须红"
    );
    assert!(
        check_by_item_covers_all_score_items(
            &score_reqs,
            &["REQ-0001".to_string(), "REQ-0002".to_string()]
        )
        .is_ok(),
        "④ 恰为全部评分项应绿"
    );

    // ⑤ 标题被截断（contains 会误过）→ 必红
    let nodes5 = vec![node("SEC-0002", "第三章 投标人须知", 1)];
    assert!(
        check_headings_verbatim(&nodes5, &[(1u8, "第三章".into())], &[]).is_err(),
        "⑤ 标题截断必须红（逐字对账的意义）"
    );
    // ⑤b 层级错位（level-1 章被渲染成 Heading2）→ 必红
    assert!(
        check_headings_verbatim(&nodes5, &[(2u8, "第三章 投标人须知".into())], &[]).is_err(),
        "⑤ 层级错位必须红"
    );
    // ⑤c 导出层多出不认识的标题 → 必红
    assert!(
        check_headings_verbatim(
            &nodes5,
            &[(1u8, "第三章 投标人须知".into()), (1u8, "来历不明的章".into())],
            &[]
        )
        .is_err(),
        "⑤ 多出不认识的一级标题必须红"
    );
    // ⑤d 白名单内的导出层追加部分（附表/格式件）+ 逐字命中 → 绿
    assert!(
        check_headings_verbatim(
            &nodes5,
            &[
                (1u8, "第三章 投标人须知".into()),
                (1u8, "附表".into()),
                (2u8, "附件一、投标函".into())
            ],
            &["附表".into(), "附件一、投标函".into()]
        )
        .is_ok(),
        "⑤ 白名单内的导出层追加标题应绿"
    );
}
