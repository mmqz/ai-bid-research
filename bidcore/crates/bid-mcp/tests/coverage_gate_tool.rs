//! #244 建纲前覆盖闭环门（coverage_gate）契约测试。
//!
//! 三态语义：每块要么 A（要求→REQ）/ B（noise·rule 排除，理由可追溯）、
//! 要么 C（未判定）→ 阻断建纲并给出**带块内容摘要**的清单。
//!
//! 门只拦「半途而废」（marks 非空且有 C 态块）；零标记文件走正则兜底
//! 放行（GT 红线保护——real_corpus_e2e 纯正则全链零位移）。
//!
//! 验收对照（issue #244）：
//! 1. 疑似要求未判定 → 建纲被拦并指明该块（含内容摘要）
//! 2. 明确噪声排除项 → 不误阻断，理由可查
//! 3. 补标后重跑 → REQ 正确进入、建纲继续、无重复/幽灵 REQ
//! 5. 重启后处置状态持久化（门行为一致）
//! 6. 正常流程不被新门卡死（零标记 / 标满两条正路都放行）
//!
//! 真件：corpus-A 主文件（docx，真实块面）。

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

fn err_parts(reply: ToolReply) -> (String, String) {
    match reply {
        ToolReply::Err { error, .. } => (error.code().to_string(), error.to_string()),
        ToolReply::Ok(out) => panic!("预期失败，实际成功: {}", out.result),
    }
}

/// 与 tools.rs `truncate` 同款（门的内容摘要口径——48 字）。
fn truncate48(s: &str) -> String {
    if s.chars().count() <= 48 {
        s.to_string()
    } else {
        s.chars().take(48).collect()
    }
}

fn tmp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-covgate-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 建项目 + 导入主文件，返回 (svc, file_id, total_blocks)。
fn setup(tag: &str) -> Option<(BidService, String, usize)> {
    let ws = tmp_ws(tag);
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "覆盖门测试", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": format!("covgate-{tag}")
    })));
    let main = fixtures().join("A-招标文件.docx");
    let out = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{
            "upload_ref": main.to_string_lossy(),
            "filename": "A-招标文件.docx",
            "kind": "招标文件"
        }]
    })));
    let fid = out["file_ids"][0].as_str()?.to_string();
    let blocks = out["per_file"][0]["chunks"].as_u64()? as usize;
    Some((svc, fid, blocks))
}

/// 验收 6 正路径①：Agent 标满（A 态 requirement + B 态 noise/rule 混合）
/// → extract → build_outline **通过**。新门不能卡死标满的正常流程。
#[test]
fn full_marks_then_outline_builds() {
    if !corpus_ready("full_marks_then_outline_builds") {
        return;
    }
    let Some((mut svc, fid, total)) = setup("full") else { return };
    assert!(total >= 12, "corpus-A 块面太小，测试前提不成立：{total}");
    let k = total / 3;

    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": k - 1, "verdict": "noise", "reason": "公告与目录（B 态排除）"},
            {"from": k, "to": total - 1, "verdict": "requirement", "reason": "正文要求区"}
        ],
        "requirements": [{
            "block_idx": k,
            "statement": "投标人须具备试验检测资质（标满路径）",
            "category": "资格条件",
            "mandatory": true
        }]
    })));

    let r = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(r["status"], "REQUIREMENTS_EXTRACTED");
    assert!(r["counts"]["total"].as_u64().unwrap() > 0);

    let o = ok_result(call(&mut svc, "build_outline", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(o["status"], "MATRIX_READY", "标满 → 门放行 → 建纲推进");
    // 覆盖闭环回执：B 态排除面可见（验收 2「理由可查」的统计面）
    let cov = &o["coverage"];
    assert_eq!(cov["face_files"].as_u64().unwrap(), 1);
    assert_eq!(cov["marked_files"].as_u64().unwrap(), 1);
    assert_eq!(cov["regex_fallback_files"].as_u64().unwrap(), 0);
    assert_eq!(cov["excluded_blocks"].as_u64().unwrap() as usize, k);
    assert_eq!(cov["unjudged_blocks"].as_u64().unwrap(), 0);
}

/// 验收 1：疑似要求未判定 → 建纲被拦，指明该块——不只给区间号，
/// 必须带块内容摘要（Agent 得知道是什么内容才能判）。
#[test]
fn partial_marks_block_outline_with_detail() {
    if !corpus_ready("partial_marks_block_outline_with_detail") {
        return;
    }
    let Some((mut svc, fid, total)) = setup("partial") else { return };
    assert!(total >= 12, "corpus-A 块面太小：{total}");
    let k = total / 3;
    // 半途而废：标 [0..k-1] noise + [k..2k] requirement，尾部 [2k+1..] 漏
    let m = 2 * k;

    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": k - 1, "verdict": "noise", "reason": "公告与目录"},
            {"from": k, "to": m, "verdict": "requirement", "reason": "正文要求区（部分）"}
        ],
        "requirements": [{
            "block_idx": k, "statement": "部分标记时的已知要求", "category": "其他"
        }]
    })));
    ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));

    let (code, msg) = err_parts(call(&mut svc, "build_outline", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(code, "E_COVERAGE_INCOMPLETE");
    assert_eq!(
        msg.lines().next().unwrap(),
        format!("E_COVERAGE_INCOMPLETE: 还有 {} 块未判定，不能建纲", total - 1 - m)
    );
    // 清单含文件与区间
    assert!(msg.contains("A-招标文件.docx"), "指明文件: {msg}");
    assert!(msg.contains(&format!("[{u}..{v}]", u = m + 1, v = total - 1)), "指明区间: {msg}");
    // 内容摘要：错误里的块文本 = 解析产物真实文本（前 48 字）——不能只给区间号
    let chunks = ok_result(call(&mut svc, "get_document_chunks", json!({
        "project_id": "PRJ-0001", "file_id": fid, "from": m + 1, "size": 1
    })));
    let first_text = chunks["chunks"][0]["text"].as_str().unwrap().to_string();
    assert!(
        msg.contains(&truncate48(&first_text)),
        "错误必须带块内容摘要（判定依据）——错误：\n{msg}\n首块文本：{}\n截断：{}",
        first_text,
        truncate48(&first_text)
    );
    assert!(msg.contains(&format!("{}: ", m + 1)), "块 idx 前缀: {msg}");
    // 处置指引
    assert!(msg.contains("submit_block_marks"), "给处置路径: {msg}");

    // 门零副作用：状态不推进、版本不动——补标后同项目可继续
    let st = ok_result(call(&mut svc, "get_coverage_report", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(st["unjudged_blocks"].as_u64().unwrap() as usize, total - 1 - m);
}

/// 验收 2：明确噪声排除项不误阻断——只有 B 态（排除）没有 C 态（遗漏）
/// 时门放行；排除理由经覆盖报告可查（区间/verdict/计数）。
#[test]
fn noise_exclusion_does_not_block() {
    if !corpus_ready("noise_exclusion_does_not_block") {
        return;
    }
    let Some((mut svc, fid, total)) = setup("noise") else { return };
    assert!(total >= 8, "corpus-A 块面太小：{total}");
    let k = total / 2;

    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": k - 1, "verdict": "noise", "reason": "法律依据与术语定义（非要求）"},
            {"from": k, "to": total - 1, "verdict": "requirement", "reason": "要求区"}
        ],
        "requirements": [{"block_idx": k, "statement": "噪声排除路径下的要求", "category": "其他"}]
    })));
    ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));

    // 排除面可查（验收 2「理由可查」）：报告含 verdict 分布与排除计数
    let rep = ok_result(call(&mut svc, "get_coverage_report", json!({
        "project_id": "PRJ-0001"
    })));
    let f = rep["files"].as_array().unwrap().iter()
        .find(|f| f["file_id"].as_str() == Some(fid.as_str())).unwrap();
    assert_eq!(f["excluded_blocks"].as_u64().unwrap() as usize, k);
    assert_eq!(f["unjudged_blocks"].as_u64().unwrap(), 0, "排除不产生未判定");

    let o = ok_result(call(&mut svc, "build_outline", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(o["status"], "MATRIX_READY", "只有排除项 → 不误阻断");
}

/// 验收 6 正路径②（GT 红线等价保护）：零标记（纯正则路径）→ 门不拦。
/// real_corpus_e2e 只走正则全链——若门无差别阻断，既有流程全死。
#[test]
fn zero_marks_regex_path_untouched() {
    if !corpus_ready("zero_marks_regex_path_untouched") {
        return;
    }
    let Some((mut svc, _fid, _total)) = setup("regex") else { return };
    let r = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));
    // no-op 语义（merge.rs）：零标记 → 正则产物零丢弃、零承接
    assert_eq!(r["merge"]["regex_dropped_covered"].as_u64().unwrap(), 0);
    assert_eq!(r["merge"]["agent_requirements"].as_u64().unwrap(), 0);
    assert!(r["counts"]["total"].as_u64().unwrap() > 0, "corpus-A 正则产物非空");

    // 门放行零标记文件（走正则兜底，不在门对象内）
    let o = ok_result(call(&mut svc, "build_outline", json!({
        "project_id": "PRJ-0001"
    })));
    assert_eq!(o["status"], "MATRIX_READY", "零标记（纯正则路径）→ 门放行");
    assert_eq!(o["coverage"]["regex_fallback_files"].as_u64().unwrap(), 1);
    assert_eq!(o["coverage"]["marked_files"].as_u64().unwrap(), 0);
}

/// 验收 3 + 要求 4：补标后重跑幂等——REQ 正确进入最终集合、建纲可继续、
/// 无重复/幽灵 REQ（两次 extract 后条款集逐位稳定）。
#[test]
fn replenish_marks_idempotent_no_ghost() {
    if !corpus_ready("replenish_marks_idempotent_no_ghost") {
        return;
    }
    let Some((mut svc, fid, total)) = setup("replenish") else { return };
    assert!(total >= 12, "corpus-A 块面太小：{total}");
    let k = total / 3;
    let m = 2 * k;

    // ① 半途而废 → 被拦
    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": k - 1, "verdict": "noise", "reason": "公告目录"},
            {"from": k, "to": m, "verdict": "requirement", "reason": "部分要求区"}
        ],
        "requirements": [{"block_idx": k, "statement": "补标幂等路径的既有要求", "category": "其他"}]
    })));
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": "PRJ-0001" })));
    let (code, _) = err_parts(call(&mut svc, "build_outline", json!({ "project_id": "PRJ-0001" })));
    assert_eq!(code, "E_COVERAGE_INCOMPLETE");

    // ② 补标漏的区间（整集替换语义：重交全部标记）
    ok_result(call(&mut svc, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": k - 1, "verdict": "noise", "reason": "公告目录"},
            {"from": k, "to": total - 1, "verdict": "requirement", "reason": "完整要求区"}
        ],
        "requirements": [{"block_idx": k, "statement": "补标幂等路径的既有要求", "category": "其他"}]
    })));

    // ③ 重跑 extract → REQ 集合 B；再跑一次 → REQ 集合 C（幂等断言面：
    //    必须在建纲前验证——build_outline 推进 MATRIX_READY 后 extract 被
    //    状态机拒，这是既有状态纪律，不是本门对象）
    let r2 = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": "PRJ-0001" })));
    let t2 = r2["counts"]["total"].as_u64().unwrap();
    assert!(t2 > 0);
    let m2 = ok_result(call(&mut svc, "get_compliance_matrix", json!({ "project_id": "PRJ-0001" })));

    let r3 = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": "PRJ-0001" })));
    let t3 = r3["counts"]["total"].as_u64().unwrap();
    assert_eq!(t3, t2, "重跑幂等：总数不变");
    let m3 = ok_result(call(&mut svc, "get_compliance_matrix", json!({ "project_id": "PRJ-0001" })));
    let ids = |m: &Value| -> Vec<String> {
        m["rows"].as_array().unwrap().iter()
            .map(|r| r["requirement"]["id"].as_str().unwrap().to_string())
            .collect()
    };
    let (i2, i3) = (ids(&m2), ids(&m3));
    assert_eq!(i2, i3, "REQ id 序列逐位一致（无幽灵 REQ）");
    assert_eq!(
        i2.len(),
        i2.iter().collect::<std::collections::HashSet<_>>().len(),
        "无重复 REQ"
    );
    assert!(i2.iter().any(|id| id.starts_with("REQ-A")), "Agent 关键点进入最终集合");

    // ④ 闭环：建纲放行
    let o = ok_result(call(&mut svc, "build_outline", json!({ "project_id": "PRJ-0001" })));
    assert_eq!(o["status"], "MATRIX_READY", "补标后建纲放行");
}

/// 要求 5：重启后处置状态持久化——门行为跨重启一致（拦还是拦，
/// 错误清单一致；补标后放行的能力不受重启影响）。
#[test]
fn gate_persists_across_restart() {
    if !corpus_ready("gate_persists_across_restart") {
        return;
    }
    let ws = tmp_ws("restart");
    let (total, fid);
    let err_before;
    {
        let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
        ok_result(call(&mut svc, "create_bid_project", json!({
            "name": "重启门测试", "bid_deadline": "2026-12-31", "mode": "明标",
            "external_key": "covgate-restart"
        })));
        let main = fixtures().join("A-招标文件.docx");
        let out = ok_result(call(&mut svc, "import_tender_files", json!({
            "project_id": "PRJ-0001",
            "files": [{"upload_ref": main.to_string_lossy(), "filename": "A-招标文件.docx", "kind": "招标文件"}]
        })));
        fid = out["file_ids"][0].as_str().unwrap().to_string();
        total = out["per_file"][0]["chunks"].as_u64().unwrap() as usize;
        assert!(total >= 12, "corpus-A 块面太小：{total}");
        let k = total / 3;

        // 半途而废标记（不标满）+ extract——处置状态必须随快照落盘
        ok_result(call(&mut svc, "submit_block_marks", json!({
            "project_id": "PRJ-0001", "file_id": fid,
            "marks": [
                {"from": 0, "to": k - 1, "verdict": "noise", "reason": "公告目录"},
                {"from": k, "to": 2 * k, "verdict": "requirement", "reason": "部分"}
            ],
            "requirements": [{"block_idx": k, "statement": "重启前提交的要求", "category": "其他"}]
        })));
        ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": "PRJ-0001" })));
        let (code, msg) = err_parts(call(&mut svc, "build_outline", json!({ "project_id": "PRJ-0001" })));
        assert_eq!(code, "E_COVERAGE_INCOMPLETE");
        err_before = msg;
    } // svc drop（落盘）

    // 重启：门对同一处置状态的裁决一致
    let mut svc2 = BidService::open(&ws).expect("重开工作区").with_fs_root(fixtures());
    let (code2, msg2) = err_parts(call(&mut svc2, "build_outline", json!({ "project_id": "PRJ-0001" })));
    assert_eq!(code2, "E_COVERAGE_INCOMPLETE", "重启后仍拦——处置状态持久化");
    assert_eq!(msg2, err_before, "错误清单跨重启一致（确定性派生）");

    // 重启后补标 → 闭环放行（补标能力不受重启影响）
    ok_result(call(&mut svc2, "submit_block_marks", json!({
        "project_id": "PRJ-0001", "file_id": fid,
        "marks": [
            {"from": 0, "to": total / 3 - 1, "verdict": "noise", "reason": "公告目录"},
            {"from": total / 3, "to": total - 1, "verdict": "requirement", "reason": "补齐"}
        ],
        "requirements": [{"block_idx": total / 3, "statement": "重启后补标的要求", "category": "其他"}]
    })));
    ok_result(call(&mut svc2, "extract_requirements", json!({ "project_id": "PRJ-0001" })));
    let o = ok_result(call(&mut svc2, "build_outline", json!({ "project_id": "PRJ-0001" })));
    assert_eq!(o["status"], "MATRIX_READY", "重启后补标闭环");
}
