//! #271 附件格式件识别——内蒙古文件集（主件 + `附件/` 目录 22-26 个官方模板）。
//!
//! 四面验证（issue #271 验收 1-5）：
//! 1. 主件正常抽取 + 附件被识别为格式件（kind=格式模板）
//! 2. 附件不产 REQ（requirements.source_file 全部来自主件；矩阵行不因附件增加）
//! 3. 附件清单在响应面可见（forms_manifest.count/items；per_file.kind 回显）
//! 4. 黑龙江文件集（无附件）行为不变（forms_manifest 键省略）
//! 5. 附件不污染矩阵/阻塞面（嵌套 zip 模板 Failed 登记但不产生 UnreadyInput）

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn batch2() -> PathBuf {
    repo_root().join("fixtures/real-tender-batch2")
}

fn temp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "forms-import-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok(svc: &mut BidService, tool: &str, args: Value) -> Value {
    match call(svc, tool, args) {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("{tool} 应成功但失败: {error}"),
    }
}

fn new_project(svc: &mut BidService, tag: &str) -> String {
    ok(svc, "create_bid_project", json!({
        "name": format!("格式件-{tag}"),
        "bid_deadline": "2026-12-31",
        "mode": "明标",
        "external_key": format!("forms-import-{tag}"),
    }))["project_id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// 附件目录清单（以磁盘为准——manifest 只登记 6 个 sample，全清单在目录）。
fn attachments_of(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir.join("附件"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    v
}

/// 单文件集导入（主件 kind=招标文件 + 附件全量 kind=格式模板）。
/// 返回 (data, 主件名)——E_PARTIAL 时 data 随错误携带（错误码不吞成功面）。
fn import_set(svc: &mut BidService, pid: &str, dir: &Path) -> (Value, String) {
    let main = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|e| e == "pdf").unwrap_or(false))
        .expect("主件 PDF 不在位");
    let main_name = main.file_name().unwrap().to_string_lossy().into_owned();
    let mut files = vec![json!({
        "upload_ref": main.display().to_string(),
        "filename": main_name, "kind": "招标文件",
    })];
    for a in attachments_of(dir) {
        files.push(json!({
            "upload_ref": a.display().to_string(),
            "filename": a.file_name().unwrap().to_string_lossy(),
            "kind": "格式模板",
        }));
    }
    let reply = call(svc, "import_tender_files", json!({ "project_id": pid, "files": files }));
    match reply {
        ToolReply::Ok(out) => (out.result, main_name),
        // 嵌套 zip 模板（property-mgmt）Failed → E_PARTIAL；data 随错误携带（错误码不吞成功面）
        ToolReply::Err { error, .. } => {
            use bid_mcp::ToolError;
            match error {
                ToolError::EPartial { data: Some(data), .. } => (data, main_name),
                other => panic!("导入应成功或 E_PARTIAL（携 data），实际: {other}"),
            }
        }
    }
}

/// 面 1+2+3：最小件（nm-health-terminal 44 页）全链——主件抽取正常 +
/// 附件零 REQ + forms_manifest 可见 + REQ 关联（尽力而为）。
#[test]
fn nm_forms_identified_not_requirements() {
    let dir = batch2().join("nm-health-terminal");
    if !dir.is_dir() {
        eprintln!("SKIP: fixtures/real-tender-batch2/nm-health-terminal 不在仓");
        return;
    }
    let attachments = attachments_of(&dir);
    assert!(!attachments.is_empty(), "附件目录应在位");
    let ws = temp_ws("nm-health");
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(batch2());
    let pid = new_project(&mut svc, "nm-health");

    let (data, main_name) = import_set(&mut svc, &pid, &dir);
    // 面 3：forms_manifest 可见（26 个附件；含 Failed 嵌套件也逐件可见）
    let fm = &data["forms_manifest"];
    let count = fm["count"].as_u64().unwrap_or(0);
    assert_eq!(
        count,
        attachments.len() as u64,
        "forms_manifest.count 应等于附件数（实际 {} 期望 {}）",
        count,
        attachments.len()
    );
    for item in fm["items"].as_array().unwrap() {
        assert!(item["file_id"].as_str().unwrap_or("").starts_with("TF-"), "清单项应有 file_id");
    }
    // per_file.kind 回显
    let per_file = data["per_file"].as_array().unwrap();
    let kinds: Vec<&str> = per_file.iter().filter_map(|p| p["kind"].as_str()).collect();
    assert!(kinds.iter().any(|k| *k == "招标文件"), "per_file 应含主件 kind");
    assert!(
        kinds.iter().filter(|k| **k == "格式模板").count() >= 20,
        "per_file 应含格式模板 kind 回显"
    );

    // 面 1：主件正常抽取（附件不产 REQ——source_file 全部来自主件）
    let ex = ok(
        &mut svc,
        "extract_requirements",
        json!({ "project_id": pid }),
    );
    let reqs: Vec<_> = svc.store().requirements().to_vec();
    assert!(!reqs.is_empty(), "主件应产出 REQ");
    let foreign: Vec<&str> = reqs
        .iter()
        .filter(|r| r.source_file != main_name)
        .map(|r| r.source_file.as_str())
        .collect();
    assert!(
        foreign.is_empty(),
        "附件不得产 REQ（外来 REQ 来源: {foreign:?}）"
    );
    // 面 4（尽力而为关联）：格式类 REQ 应有 form_file_id 指向模板（至少一条）
    let linked = reqs.iter().filter(|r| r.params.contains_key("form_file_id")).count();
    eprintln!("REQ 总数 {}（全部来自主件 {main_name}）；form_file_id 关联 {linked} 条", reqs.len());
    assert!(linked > 0, "至少一条格式类 REQ 应关联模板（FORMS_LINKED 告警也应出现）");
    assert!(
        ex["warnings"].to_string().contains("FORMS_LINKED"),
        "extract 响应应含 FORMS_LINKED 告警"
    );

    // 面 2：矩阵行不因附件增加——每行 requirement 均可溯源到主件 REQ
    // （附件不产 REQ 已由 foreign 断言闭环；矩阵行均指向 REQ id 即不可能是附件行）
    ok(&mut svc, "build_outline", json!({ "project_id": pid }));
    let matrix = ok(
        &mut svc,
        "get_compliance_matrix",
        json!({ "project_id": pid }),
    );
    let rows = matrix["rows"].as_array().cloned().unwrap_or_default();
    let req_ids: std::collections::HashSet<&str> = reqs.iter().map(|r| r.id.as_str()).collect();
    assert!(!rows.is_empty(), "矩阵应有行（主件 REQ 在场）");
    for row in &rows {
        let rid = row["requirement"]["id"].as_str().unwrap_or("");
        assert!(
            req_ids.contains(rid),
            "矩阵行 {rid} 不可溯源到主件 REQ（附件不得进矩阵）"
        );
    }
    let _ = std::fs::remove_dir_all(&ws);
}

/// 面 5：嵌套 zip 模板 Failed 登记但不产生 UnreadyInput（不阻塞起草）。
#[test]
fn nested_zip_form_fails_visible_but_not_blocking() {
    let dir = batch2().join("nm-property-mgmt");
    if !dir.is_dir() {
        eprintln!("SKIP: fixtures/real-tender-batch2/nm-property-mgmt 不在仓");
        return;
    }
    let ws = temp_ws("nm-prop");
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(batch2());
    let pid = new_project(&mut svc, "nm-prop");
    let (data, _main) = import_set(&mut svc, &pid, &dir);
    // 22 附件全在清单（含 zip Failed 件）
    let fm = &data["forms_manifest"];
    assert_eq!(
        fm["count"].as_u64().unwrap_or(0),
        attachments_of(&dir).len() as u64
    );
    // 嵌套 zip 模板应显式 Failed（不静默）且在清单可见
    let zip_items: Vec<&Value> = fm["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["name"].as_str().unwrap_or("").ends_with(".zip"))
        .collect();
    assert!(!zip_items.is_empty(), "嵌套 zip 模板应在清单内");
    assert!(
        zip_items
            .iter()
            .all(|i| i["parse_status"].as_str().unwrap_or("").eq_ignore_ascii_case("failed")),
        "zip 模板应显式 FAILED: {zip_items:?}"
    );
    // input_integrity：格式模板不在面（不产生 unready——起草不被模板坏件阻塞）
    let integrity = &data["input_integrity"];
    let unready_names: Vec<&str> = integrity["unready"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["name"].as_str())
        .collect();
    for i in fm["items"].as_array().unwrap() {
        let name = i["name"].as_str().unwrap_or("");
        assert!(
            !unready_names.contains(&name),
            "格式模板 {name} 不得出现在 input_integrity.unready（模板不在阻塞面）: {unready_names:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&ws);
}

/// 面 4：黑龙江形态（无附件）零扰动——forms_manifest 键省略。
#[test]
fn hlj_single_pdf_no_forms_zero_perturbation() {
    let dir = batch2().join("hlj-inquiry");
    let base = if dir.is_dir() {
        dir
    } else {
        // batch1 任一目录（单 PDF 无附件）——repo 根 fixtures/real-tender-batch1
        let b1 = repo_root().join("fixtures/real-tender-batch1");
        let mut d = None;
        if b1.is_dir() {
            d = std::fs::read_dir(&b1)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.is_dir());
        }
        match d {
            Some(d) => d,
            None => {
                eprintln!("SKIP: 黑龙江对照 fixture 不在仓");
                return;
            }
        }
    };
    let pdf = std::fs::read_dir(&base)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|e| e == "pdf").unwrap_or(false));
    let Some(pdf) = pdf else {
        eprintln!("SKIP: 对照目录无 PDF（{base:?}）");
        return;
    };
    let ws = temp_ws("hlj");
    let root = pdf.parent().unwrap();
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(root);
    let pid = new_project(&mut svc, "hlj");
    let out = match call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": pid, "files": [{
            "upload_ref": pdf.display().to_string(),
            "filename": pdf.file_name().unwrap().to_string_lossy(),
            "kind": "招标文件",
        }]}),
    ) {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("单 PDF 导入应成功: {error}"),
    };
    assert!(
        out.get("forms_manifest").is_none(),
        "无格式件时 forms_manifest 键应省略（黑龙江形态零扰动）: {}",
        out["forms_manifest"]
    );
    assert_eq!(out["per_file"][0]["kind"], json!("招标文件"), "kind 回显");
    let _ = std::fs::remove_dir_all(&ws);
}
