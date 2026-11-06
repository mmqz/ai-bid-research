//! #285 ② 块级双向索引真件验收。
//!
//! 验收面（issue #285 ② 验收条款逐条）：
//! 1. **每个 REQ 都能拿到 block_idx + page + line**——全量 REQ 走
//!    `get_requirement_provenance`：锚点必带块位（resolved_via=backfilled）；
//!    page/line 与 `page_line_of(locator)` 逐锚一致（确定性），PDF 面锚
//!    （pdf_line）页号必解析
//! 2. **每个块（含无 REQ 的）都能反向查到 req_ids**——`get_block_requirements`
//!    逐块调用（小件全量、大件全量——工具面在内存态，微秒级）；无 REQ 块
//!    req_ids=[] + verdict 诚实口径（无标记=null）
//! 3. suspected_miss（复用 #285① star_scan 真值面——同源数据）：逐件与
//!    `get_star_coverage` 的 missing 对账（块位粒度一致性），danqing 引擎
//!    路专项断言 missing=6（p18 参数性质列×4 + p48 模板×2——① 验收口径）
//! 4. 旧快照反序列化——单测面（bid-domain lib + bidi_tools_tool legacy 腿）
//! 5. 不改现有抽取结果——real_corpus_e2e GT 红线在同套件守门（本测试只读
//!    零副作用，不触碰抽取）
//! 6. 环境门：真件逐件导入失败（.doc 引擎面/兜底路 CID-CJK fail-loud）
//!    → SKIP 横幅跳过该件（环境差异不伪装业务回归）；断言自洽
//!    （抽取与锚来自同一次解析），不依赖布局引擎路。
//!
//! 真件锚定：danqing 五题★ REQ（#276 star=true）provenance 页号必在
//! p18/p19（参数性质列/商务表——① 的侦察口径）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
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

fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-bidi-accept-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 待验收件（batch1 五件 + batch2 全部——PDF/docx/doc 混合面）。
#[derive(Debug)]
struct Tender {
    id: String,
    file: PathBuf,
}

fn load_tenders() -> Vec<Tender> {
    let mut out = Vec::new();
    for batch in ["real-tender-batch1", "real-tender-batch2"] {
        let mf = repo_root().join("fixtures").join(batch).join("manifest.json");
        let Ok(m) = std::fs::read_to_string(&mf) else {
            continue;
        };
        let v: Value = serde_json::from_str(&m).unwrap_or_else(|e| panic!("manifest 解析: {e}"));
        for t in v["tenders"].as_array().unwrap_or(&vec![]) {
            let id = t["id"].as_str().unwrap_or("").to_string();
            let mut file = t["path"].as_str().map(|p| {
                let pb = PathBuf::from(p);
                if pb.is_absolute() { pb } else { repo_root().join(pb) }
            });
            if file.is_none() {
                if let Some(f) = t["main_file"].as_str() {
                    file = Some(repo_root().join("fixtures").join(batch).join(&id).join(f));
                }
            }
            let Some(file) = file else { continue };
            if !file.exists() {
                continue;
            }
            out.push(Tender { id, file });
        }
    }
    out
}

/// 建项目 + 导入单件 + 抽取。导入失败（环境差异）→ None = 该件 SKIP。
fn setup_extracted(t: &Tender) -> Option<(BidService, String, String)> {
    let ws = tmp_ws(&t.id);
    let root = t.file.parent().unwrap();
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(root);
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": format!("②验收——{}", t.id), "bid_deadline": "2026-12-31",
                "mode": "明标", "external_key": format!("bidi-accept-{}", t.id) }),
    ))["project_id"]
    .as_str()
    .unwrap()
    .to_string();
    let import = match call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [{
            "upload_ref": t.file.display().to_string(),
            "filename": t.file.file_name().unwrap().to_string_lossy(),
            "kind": "招标文件"
        }]}),
    ) {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => {
            eprintln!(
                "【SKIP】{} 本环境导入失败（{error}）——环境差异不伪装业务回归",
                t.id
            );
            return None;
        }
    };
    let f0 = &import["per_file"][0];
    if f0["parse_status"] != json!("ok") {
        eprintln!(
            "【SKIP】{} 本环境解析不开（{}）——同上",
            t.id,
            f0["parse_error"]
        );
        return None;
    }
    let fid = f0["file_id"].as_str().unwrap().to_string();
    let extract = ok_result(call(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    let total = extract["counts"]["total"].as_u64().unwrap_or(0);
    if total == 0 {
        eprintln!("【SKIP】{} 抽取 0 条 REQ（{total}）——provenance 面无对象，不伪装", t.id);
        return None;
    }
    Some((svc, pid, fid))
}

/// 验收 1+2+3：全量 REQ provenance + 全量块反查 + overlay 口径。
#[test]
fn block_bidi_acceptance_all_tenders() {
    let tenders = load_tenders();
    assert!(tenders.len() >= 9, "真件清单至少 9 件在位（batch1 五件 + batch2）：{tenders:?}",);

    let mut accepted = 0usize;
    let mut skipped = 0usize;
    for t in &tenders {
        let Some((mut svc, pid, fid)) = setup_extracted(t) else {
            skipped += 1;
            continue;
        };
        accepted += 1;
        eprintln!("【验收】{} …", t.id);

        // ---- ① 全量 REQ：provenance 必带块位；页行与 page_line_of 一致 ----
        let req_ids: Vec<String> = svc.store().requirements().iter().map(|r| r.id.clone()).collect();
        assert!(!req_ids.is_empty(), "{} 必有 REQ（setup 已验）", t.id);
        let mut pdf_face_anchors = 0usize;
        for req_id in &req_ids {
            let prov = ok_result(call(
                &mut svc,
                "get_requirement_provenance",
                json!({ "project_id": &pid, "req_id": req_id }),
            ));
            let anchors = prov["anchors"].as_array().unwrap_or_else(|| {
                panic!("{}：{} 锚数组缺失: {prov}", t.id, req_id)
            });
            assert!(!anchors.is_empty(), "{}：{} 无锚（抽取产物必有锚）", t.id, req_id);
            for a in anchors {
                assert_eq!(
                    a["resolved_via"], json!("backfilled"),
                    "{}：{} 新抽取锚必走回填腿: {a}", t.id, req_id
                );
                assert!(
                    a["block_idx"].is_u64(),
                    "{}：{} 锚必带块位: {a}",
                    t.id,
                    req_id
                );
                // 页行确定性：与 page_line_of(locator) 逐锚一致（单一解析真源）
                let (p, l) = bid_parse::page_line_of(a["locator"].as_str().unwrap_or(""));
                assert_eq!(a["page"], json!(p), "{}：{} 页号一致性: {a}", t.id, req_id);
                assert_eq!(a["line"], json!(l), "{}：{} 行号一致性: {a}", t.id, req_id);
                if a["kind"] == json!("pdf_line") {
                    assert!(p.is_some(), "{}：{} pdf_line 锚页号必解析: {a}", t.id, req_id);
                    pdf_face_anchors += 1;
                }
            }
        }
        // PDF 件必有 pdf_line 锚（docx 件为 0——para/table 面，页行 null 诚实）
        let is_pdf = t.file.extension().map(|e| e == "pdf").unwrap_or(false);
        if is_pdf {
            assert!(
                pdf_face_anchors > 0,
                "{}：PDF 件 REQ 锚必含 pdf_line 面",
                t.id
            );
        }

        // ---- ② 全量块：反向逐块可查（含无 REQ 块）----
        let overlay = ok_result(call(
            &mut svc,
            "get_annotation_overlay",
            json!({ "project_id": &pid, "file_id": &fid }),
        ));
        let total_blocks = overlay["total_blocks"].as_u64().unwrap() as usize;
        let req_bearing = overlay["req_bearing_blocks"].as_u64().unwrap() as usize;
        assert!(total_blocks > 0, "{}：解析产物必有块", t.id);
        assert!(req_bearing > 0, "{}：抽取后必有 REQ 锚定块", t.id);
        let mut req_bearing_seen = 0usize;
        for idx in 0..total_blocks {
            let out = ok_result(call(
                &mut svc,
                "get_block_requirements",
                json!({ "project_id": &pid, "file_id": &fid, "block_idx": idx }),
            ));
            assert_eq!(out["block_idx"], json!(idx), "{}：块位回显", t.id);
            assert!(out["text"].is_string(), "{}：块 {idx} 原文在场", t.id);
            assert!(out["locator"].is_string(), "{}：块 {idx} locator 在场", t.id);
            let req_ids_of_block = out["req_ids"].as_array().unwrap().clone();
            if !req_ids_of_block.is_empty() {
                req_bearing_seen += 1;
                assert_eq!(out["verdict"], json!("requirement"), "{}：REQ 块 verdict", t.id);
                // 双向闭合：REQ 块的 req_id provenance 必含该块位
                for rid in &req_ids_of_block {
                    let prov = ok_result(call(
                        &mut svc,
                        "get_requirement_provenance",
                        json!({ "project_id": &pid, "req_id": rid.as_str().unwrap() }),
                    ));
                    assert!(
                        prov["anchors"].as_array().unwrap().iter().any(|a| a["block_idx"] == json!(idx)),
                        "{}：双向闭合——{} provenance 必含块 {idx}",
                        t.id,
                        rid
                    );
                }
            } else {
                assert!(
                    out["verdict"].is_null() || out["verdict"].is_string(),
                    "{}：无 REQ 块 verdict 口径（无标记=null / 有标记=串）: {out}",
                    t.id
                );
            }
        }
        assert_eq!(
            req_bearing_seen, req_bearing,
            "{}：逐块反查的 REQ 块数 == overlay 计数（不漏不重）",
            t.id
        );

        // ---- ③ suspected_miss 真值面：与 #285① star_coverage 同源对账 ----
        // ★真值块（块位粒度）无 REQ → suspected_miss；① 的 star_missing 按
        // ★字符计数——单★块两口径相等，多★块（hq-dev 形态）块位 ≤ 字符数。
        let star = ok_result(call(
            &mut svc,
            "get_star_coverage",
            json!({ "project_id": &pid, "file_id": &fid }),
        ));
        let star_missing = star["star_missing"].as_u64().unwrap_or(0);
        let overlay_missing = overlay["suspected_miss_blocks"].as_u64().unwrap_or(0);
        assert!(
            overlay_missing <= star_missing,
            "{}：suspected_miss_blocks({overlay_missing}) 不得超过 ① star_missing({star_missing})——块位粒度 ≤ 字符粒度",
            t.id
        );
        if star_missing == 0 {
            assert_eq!(overlay_missing, 0, "{}：① 无漏标 → overlay 无疑似", t.id);
        } else {
            assert!(
                overlay_missing > 0,
                "{}：① 报漏标 {star_missing} → overlay 必有疑似（不静默）",
                t.id
            );
            assert!(
                overlay["warnings"].as_array().unwrap().iter().any(|w| {
                    w.as_str().map(|s| s.contains("suspected_miss")).unwrap_or(false)
                }),
                "{}：漏标 warnings 显式提示",
                t.id
            );
        }
        eprintln!(
            "【验收】{} 通过：REQ {} 条全带块位、块 {}（REQ 块 {}）全量反查",
            t.id,
            req_ids.len(),
            total_blocks,
            req_bearing
        );
        let _ = std::fs::remove_dir_all(tmp_ws(&t.id));
    }
    // 环境守门：本沙箱（poppler 在位）至少 9 件全过；CI 无引擎环境件数
    // 收缩但不为 0（docx 走引擎面可验收）
    assert!(
        accepted >= 5,
        "通过件数 {accepted} 异常收缩（skipped={skipped}）——环境大面积缺件要看得见"
    );
}

/// danqing 交叉锚：五题★ REQ（params.star=true）provenance 页号必在
/// p18/p19（参数性质列/商务表——#285① 侦察口径，块级定位真值检验）。
#[test]
fn danqing_star_reqs_land_on_declared_pages() {
    let Some(t) = load_tenders().into_iter().find(|t| t.id == "danqing-reservoir-230001") else {
        eprintln!("【SKIP】danqing 真件不在位");
        return;
    };
    let Some((mut svc, pid, fid)) = setup_extracted(&t) else {
        return; // SKIP 横幅已打
    };
    let star_reqs: Vec<String> = svc
        .store()
        .requirements()
        .iter()
        .filter(|r| {
            r.params.get("star").and_then(|v| v.as_bool()) == Some(true)
        })
        .map(|r| r.id.clone())
        .collect();
    assert!(
        star_reqs.len() >= 5,
        "danqing 引擎路五题★ REQ 必在场（① 验收口径 REQ-0302..0306）：{}",
        star_reqs.len()
    );
    let mut pages = Vec::new();
    for req_id in &star_reqs {
        let prov = ok_result(call(
            &mut svc,
            "get_requirement_provenance",
            json!({ "project_id": &pid, "req_id": req_id }),
        ));
        for a in prov["anchors"].as_array().unwrap() {
            if let Some(p) = a["page"].as_u64() {
                pages.push(p);
            }
        }
    }
    assert!(
        pages.iter().all(|p| *p == 18 || *p == 19),
        "五题★锚页号必在 p18/p19（实测 {pages:?}）——块级定位与①侦察口径对齐"
    );

    // ---- suspected_miss 真值（① 引擎路口径）：真值 12 / 已捕获 6 / 漏 6
    //      （p18 参数性质列「2★..5★」×4 + p48 偏离表模板×2）----
    let overlay = ok_result(call(
        &mut svc,
        "get_annotation_overlay",
        json!({ "project_id": &pid, "file_id": &fid }),
    ));
    assert_eq!(
        overlay["suspected_miss_blocks"], json!(6),
        "danqing 引擎路 suspected_miss=6（p18×4 + p48×2——① star_coverage 同源）: {}",
        overlay["suspected_miss_blocks"]
    );
    // emphasis / suspected_miss 明细（1688 块全量回显会截断——按页取：
    // p18 参数性质列 / p48 偏离表模板两页即漏标块全集）
    let nfkc = |s: &str| -> String {
        use unicode_normalization::UnicodeNormalization;
        s.nfkc().collect()
    };
    let overlay_page = |svc: &mut BidService, page: u64| -> Value {
        ok_result(call(
            svc,
            "get_annotation_overlay",
            json!({ "project_id": &pid, "file_id": &fid, "page": page }),
        ))
    };
    let p18 = overlay_page(&mut svc, 18);
    let miss18: Vec<String> = p18["overlay"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["suspected_miss"] == json!(true))
        .map(|e| nfkc(e["text"].as_str().unwrap_or("").trim()))
        .collect();
    assert_eq!(
        miss18, vec!["2 ★", "3 ★", "4 ★", "5 ★"],
        "p18 参数性质列漏标块（① 引擎路清单同源）"
    );
    // p18 的「1 ★」经 REQ 锚捕获——emphasis 但非 suspected_miss
    assert!(
        p18["overlay"].as_array().unwrap().iter().any(|e| {
            e["semantic"] == json!("emphasis") && e["suspected_miss"] == json!(false)
        }),
        "p18「1 ★」捕获块：emphasis 语义在座且非疑似漏标"
    );
    let p48 = overlay_page(&mut svc, 48);
    let miss48: Vec<String> = p48["overlay"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["suspected_miss"] == json!(true))
        .map(|e| nfkc(e["text"].as_str().unwrap_or("").trim()))
        .collect();
    assert_eq!(
        miss48, vec!["★ 1.1", "★ 2.1"],
        "p48 偏离表模板漏标块（① 引擎路清单同源）"
    );
    // p19 商务表五题：captured——emphasis 在座且零疑似
    let p19 = overlay_page(&mut svc, 19);
    assert_eq!(p19["suspected_miss_blocks"], json!(0), "五题全捕获（① 口径）");
    assert!(
        p19["overlay"].as_array().unwrap().iter().any(|e| e["semantic"] == json!("emphasis")),
        "p19 商务表★块 emphasis 语义在座"
    );
    let _ = std::fs::remove_dir_all(tmp_ws(&t.id));
}
