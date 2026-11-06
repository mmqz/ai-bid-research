//! #244 建纲覆盖门 × 多平台真实语料验收（batch1 + batch2）。
//!
//! issue #244 背景与 Lead 验收 6：**新门不能卡死正常流程**——
//! 10+ 份真实语料、3 个平台来源（黑龙江政采 / 内蒙 / 询价形态），
//! 覆盖闭环必须在多平台样本上都成立，不能只针对「章-节」结构：
//!
//! - batch1（5 份黑龙江政采统一导出 PDF，「第X章」结构）
//! - batch2（6 份跨平台：.doc / .docx / 询价件（数字编号，无「第X章」））
//!
//! 每份走完整 P2 流程（Agent 标满的模拟）：
//! 导入 → submit_block_marks（noise 前区 + requirement 主体 + 关键点）
//! → extract_requirements → build_outline → **应能建纲**。
//! 询价件另走「故意漏标」变体 → **应被拦住并给出带内容的清单**。
//!
//! 缺件纪律（issue #9：缺件不是业务失败，也绝不静默假装通过）：
//! - 语料缺失 → SKIP 横幅提前返回；
//! - 环境缺 poppler（Windows CI）→ batch1/batch2 整体 SKIP——PR #292
//!   三次 CI 红的教训（AGENTS.md 三之二铁律：缺环境不得伪装成业务 FAIL）；
//! - .doc 件缺转换器（Word COM / LibreOffice 均不在场）→ 该件 SKIP 横幅、
//!   其余件继续真跑（Lead 方案 B：保住 PDF/docx 份的覆盖面）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use bid_parse::env_preflight::{skip_if_env_missing, POPPLER_TEXT_TOOLS};
use bid_tools::service::doc_converter_available;
use serde_json::{json, Value};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn batch(n: u8) -> PathBuf {
    repo_root().join(format!("fixtures/real-tender-batch{n}"))
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

fn err_text(reply: ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.to_string(),
        ToolReply::Ok(out) => panic!("预期失败，实际成功: {}", out.result),
    }
}

fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-covcorpus-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// batch 目录下每个 tender 子目录的主文件（manifest 的 main_file/original
/// 同名落盘；从 manifest 读文件名，防目录扫描遇到「附件」子目录误入）。
fn tender_mains(n: u8) -> Vec<(String, PathBuf)> {
    let manifest = batch(n).join("manifest.json");
    let Ok(raw) = std::fs::read_to_string(&manifest) else {
        return Vec::new();
    };
    let v: Value = serde_json::from_str(&raw).expect("manifest 合法 JSON");
    v["tenders"]
        .as_array()
        .expect("tenders 数组")
        .iter()
        .filter_map(|t| {
            let id = t["id"].as_str()?.to_string();
            let name = t["main_file"].as_str().or_else(|| t["path"].as_str())?;
            // path 可能含目录前缀（batch1）/ main_file 是纯文件名（batch2）
            let p = batch(n).join(&id).join(name.rsplit('/').next().unwrap_or(name));
            Some((id, p))
        })
        .filter(|(_, p)| p.is_file())
        .collect()
}

/// 一份语料的「Agent 标满 → 建纲」全链（返回建纲响应）。
///
/// 标满模拟（P2 流程的程序化等价）：前 1/3 noise（公告/目录区）+
/// 其余 requirement（正文）+ 1 条关键点。块面 < 12 的极小件全 requirement。
fn full_marks_flow(svc: &mut BidService, pid: &str, fid: &str, total: usize) -> Value {
    let k = if total >= 12 { total / 3 } else { 0 };
    let mut marks = Vec::new();
    if k > 0 {
        marks.push(json!({"from": 0, "to": k - 1, "verdict": "noise", "reason": "公告与目录区（程序化标满）"}));
    }
    marks.push(json!({"from": k, "to": total - 1, "verdict": "requirement", "reason": "正文要求区（程序化标满）"}));
    ok_result(call(svc, "submit_block_marks", json!({
        "project_id": pid, "file_id": fid,
        "marks": marks,
        "requirements": [{
            "block_idx": k, "statement": "程序化标满路径的样例要求", "category": "其他"
        }]
    })));
    let r = ok_result(call(svc, "extract_requirements", json!({ "project_id": pid })));
    assert_eq!(r["status"], "REQUIREMENTS_EXTRACTED");
    assert!(
        r["counts"]["total"].as_u64().unwrap() > 0,
        "标满后条款面非空（正则兜底 + Agent 承接）"
    );
    ok_result(call(svc, "build_outline", json!({ "project_id": pid })))
}

/// 单份语料：独立项目 → 导入 → 标满全链 → 断言建纲成功 + 覆盖回执。
fn assert_one_buildable(tag: &str, path: &PathBuf) {
    let ws = tmp_ws(tag);
    let root = path.parent().unwrap().to_path_buf();
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(&root);
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "多平台覆盖门验收", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": format!("covcorpus-{tag}")
    })));
    let out = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{
            "upload_ref": path.to_string_lossy(),
            "filename": path.file_name().unwrap().to_string_lossy(),
            "kind": "招标文件"
        }]
    })));
    let Some(fid) = out["file_ids"][0].as_str() else {
        panic!("导入未返回 file_id：{out}");
    };
    let total = out["per_file"][0]["chunks"].as_u64().unwrap_or(0) as usize;
    assert!(total > 0, "解析块面为空：{}", path.display());

    let o = full_marks_flow(&mut svc, "PRJ-0001", fid, total);
    assert_eq!(
        o["status"], "MATRIX_READY",
        "标满（A+B 态闭环）→ 新门放行 → 建纲推进：{}", path.display()
    );
    assert!(o["counts"]["sections"].as_u64().unwrap() > 0, "大纲非空");
    assert_eq!(o["coverage"]["unjudged_blocks"].as_u64().unwrap(), 0);
    assert_eq!(o["coverage"]["marked_files"].as_u64().unwrap(), 1);
    let _ = std::fs::remove_dir_all(&ws);
}

/// batch1：5 份黑龙江政采统一导出 PDF——同一套标满/建纲逻辑在跨题目上的
/// 稳定性（验收 6：新门不卡死正常流程）。
#[test]
fn batch1_full_marks_all_buildable() {
    // 环境门（AGENTS.md 三之二）：CI（Windows）无 poppler → PDF 抽取链
    // 降级 → E_PARTIAL 必红；本机 Linux（poppler 在位）真跑。
    if skip_if_env_missing("coverage_gate_batch1", POPPLER_TEXT_TOOLS) {
        return;
    }
    let mains = tender_mains(1);
    if mains.is_empty() {
        eprintln!("SKIP: fixtures/real-tender-batch1 语料不在仓");
        return;
    }
    assert!(mains.len() >= 5, "batch1 应有 5 份语料，实际 {}", mains.len());
    for (id, p) in &mains {
        eprintln!("-- batch1 {id}: {}", p.display());
        assert_one_buildable(&format!("b1-{id}"), p);
    }
}

/// batch2：6 份跨平台形态（.doc / .docx / 询价 PDF）——数字编号、无
/// 「第X章」的结构也必须走通同一条覆盖闭环。
#[test]
fn batch2_full_marks_all_buildable() {
    // 环境门（同 batch1）：4 份 PDF 需要 poppler，缺则整体 SKIP。
    if skip_if_env_missing("coverage_gate_batch2", POPPLER_TEXT_TOOLS) {
        return;
    }
    let mains = tender_mains(2);
    if mains.is_empty() {
        eprintln!("SKIP: fixtures/real-tender-batch2 语料不在仓");
        return;
    }
    assert!(mains.len() >= 6, "batch2 应有 6 份语料，实际 {}", mains.len());
    for (id, p) in &mains {
        // .doc 件逐件判定（Lead 方案 B）：转换器（Windows Word COM /
        // 其他平台 LibreOffice）均不在场时跳过该件并打响亮横幅——环境
        // 缺件不是业务失败（issue #9），其余 PDF/docx 件继续真跑保覆盖面。
        // BID_NO_DOC_CONVERTER=1 可在转换器在位的机器上模拟此路径。
        if p.extension().map(|e| e.eq_ignore_ascii_case("doc")).unwrap_or(false)
            && !doc_converter_available()
        {
            eprintln!();
            eprintln!("====================================================================");
            eprintln!("SKIP: batch2 {id}（.doc 转换器不可用）: {}", p.display());
            eprintln!("reason: ENV_MISSING: .doc 需 Word COM（Windows）或 LibreOffice，本机均不可用");
            eprintln!("（环境缺件，非业务失败——issue #9；其余件继续真跑）");
            eprintln!("====================================================================");
            continue;
        }
        eprintln!("-- batch2 {id}: {}", p.display());
        assert_one_buildable(&format!("b2-{id}"), p);
    }
}

/// Lead 指定变体（询价件——issue 背景：数字编号，无「第X章」，块结构
/// 可能完全不同）：故意漏标尾部几个块 → 建纲被拦 + 清单带块内容。
#[test]
fn inquiry_partial_marks_blocked_with_detail() {
    let dir = batch(2).join("hlj-inquiry");
    let Some(pdf) = std::fs::read_dir(&dir)
        .ok()
        .and_then(|it| {
            it.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                p.extension().map(|e| e == "pdf").unwrap_or(false)
            })
        })
    else {
        eprintln!("SKIP: fixtures/real-tender-batch2/hlj-inquiry 不在仓");
        return;
    };
    let ws = tmp_ws("hlj-partial");
    let root = dir.clone();
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(&root);
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "询价漏标门验收", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": "covcorpus-hlj-partial"
    })));
    let out = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{
            "upload_ref": pdf.to_string_lossy(),
            "filename": pdf.file_name().unwrap().to_string_lossy(),
            "kind": "招标文件"
        }]
    })));
    let fid = out["file_ids"][0].as_str().unwrap().to_string();
    let total = out["per_file"][0]["chunks"].as_u64().unwrap() as usize;
    assert!(total >= 12, "询价件块面太小，测试前提不成立：{total}");
    let half = total / 2;

    // 故意漏标：只标前半，尾部 [half+1..] 未判定
    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": half, "verdict": "requirement", "reason": "前半要求区（故意漏标变体）"}
        ],
        "requirements": [{"block_idx": 0, "statement": "漏标变体的已知要求", "category": "其他"}]
    })));
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": "PRJ-0001" })));

    let msg = err_text(call(&mut svc, "build_outline", json!({ "project_id": "PRJ-0001" })));
    assert!(
        msg.starts_with("E_COVERAGE_INCOMPLETE"),
        "询价件漏标 → 门拦（数字编号结构同样成立）：{msg}"
    );
    assert!(msg.contains(&format!("[{u}..{v}]", u = half + 1, v = total - 1)), "指明区间: {msg}");
    // 清单带块内容（不能只给区间号）
    let chunks = ok_result(call(&mut svc, "get_document_chunks", json!({
        "project_id": "PRJ-0001", "file_id": fid, "from": half + 1, "size": 1
    })));
    let text = chunks["chunks"][0]["text"].as_str().unwrap();
    let head: String = text.chars().take(48).collect();
    assert!(msg.contains(&head), "清单带块内容摘要：\n错误：{msg}\n块文本：{text}");
    let _ = std::fs::remove_dir_all(&ws);
}
