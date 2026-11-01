//! #59 阶段 6 · 1330 页真实规模验证：50 万字 / ~250 章全链性能红线。
//!
//! Lead 红线（原文）：
//! ```text
//! 120s 内完成一次完整导出
//! 内存峰值 < 2GB
//! 不 OOM
//! ```
//!
//! 规模口径（Lead 4.1「或直接生成 50 万字的合成文档」——合成面允许）：
//! - REQ 面：1,500 条（六类轮换——1330 页招标文件的抽取量级）
//! - 配额面：500,000 字（target_total_chars）
//! - 章节面：每叶 ≤ 6 REQ → ~250 叶（500 章口径的一半——用 1,500 REQ
//!   而非 12,000，测试跑得动的前提下覆盖调度/落盘/导出的规模行为；
//!   12,000 REQ 全量是 CI 的活，不是单测的活）
//!
//! 计时面：每步 Instant 计时（`--nocapture` 逐步可见——Lead「记录每一步
//! 耗时」）；内存面：`/proc/self/status` VmHWM（Linux——S-5 check.rs 同款
//! 口径，进程生命周期峰值）。
//!
//! SKIP 纪律：非 Linux（无 /proc）→ 响亮横幅跳过（内存断言不可得——
//! 不假装通过）。

use bid_domain::outline::OutlineSpec;
use bid_domain::{Anchor, ParseStatus, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::time::Instant;

/// VmHWM 千字节（Linux /proc——进程生命周期内存峰值）。
fn vm_hwm_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(v) = line.strip_prefix("VmHWM:") {
            return v.trim().trim_end_matches("kB").trim().parse().ok();
        }
    }
    None
}

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    // #68 阶段 1：确定性套件钉 mock（防 dev 机配了 BID_LLM_API_KEY 后 auto
    // 走真实调用——测试断言全部基于 mock 的确定性内容面）。
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut().unwrap().insert("generator".into(), serde_json::json!("mock"));
    }
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("规模链步骤失败: {error}"),
    }
}

/// 1,500 条合成 REQ（六类轮换——编号条款形态贴近真实抽取面）。
fn synth_requirements(n: usize) -> Vec<Requirement> {
    let cats = ["资格条件", "否决项", "评分项", "技术参数", "商务要求", "格式要求"];
    (0..n)
        .map(|i| Requirement {
            id: format!("REQ-{:04}", i + 1),
            category: cats[i % cats.len()].into(),
            clause_no: Some(format!("{}.{}.{}", 2 + i / 100, i % 100 + 1, (i % 5) + 1)),
            anchors: vec![Anchor {
                file: "A-合成招标文件.docx".into(),
                kind: "para".into(),
                locator: format!("第三章/{}", i + 1),
                excerpt: format!(
                    "第 {} 条：投标人应满足第 {} 项要求——{}（合成条款，用于规模验证）",
                    i + 1, i % 9 + 1, cats[i % cats.len()],
                ),
            }],
            params: Default::default(),
            source_file: "A-合成招标文件.docx".into(),
        })
        .collect()
}

fn tmp_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-scale-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn scale_500k_chars_full_pipeline_under_redlines() {
    // ---- SKIP 预检（非 Linux 无 VmHWM——内存红线不可断言，响亮跳过）
    let Some(hwm0) = vm_hwm_kb() else {
        eprintln!(
            "SKIP: scale_500k_chars_full_pipeline_under_redlines——非 Linux（无 /proc/self/status VmHWM），内存红线不可断言"
        );
        return;
    };

    let ws = tmp_dir();
    let mut svc = BidService::open(&ws).unwrap();
    let t_all = Instant::now();

    // ① 项目 + 抽取面注入（导入解析在 e2e #64 真件验证——规模测试聚焦
    //    调度/生成/落盘/导出，解析合成注入）
    let t0 = Instant::now();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "规模验证——50 万字合成标书", "bid_deadline": "2026-12-31",
        "mode": "明标", "external_key": "scale-500k",
    })))["project_id"].as_str().unwrap().to_string();
    svc.store_mut().upsert_file(
        "A-合成招标文件.docx", "招标文件", "sha-scale", 1330, 40, 8_000, 0,
        ParseStatus::Ok, "",
    ).unwrap();
    let reqs = synth_requirements(1_500);
    svc.set_requirements(reqs).unwrap();
    println!("[规模] ①建项+注入 1,500 REQ: {}ms", t0.elapsed().as_millis());

    // ② 建纲（~250 叶 / 500,000 字配额）
    let t = Instant::now();
    let (n_sec, n_rows, n_blocking) = svc
        .prepare_outline_and_matrix(OutlineSpec { target_total_chars: 500_000, max_reqs_per_leaf: 6 })
        .unwrap();
    let n_leaves = {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        svc.store().sections().iter().filter(|n| !parents.contains(&n.id)).count()
    };
    println!("[规模] ②建纲建矩阵: {}ms（{n_sec} 节点/{n_leaves} 叶/{n_rows} 行/blocking {n_blocking}）",
        t.elapsed().as_millis());
    assert!(n_leaves >= 200, "叶数 {n_leaves}（1,500 REQ ÷ 6/叶 ≈ 250）");

    // ③ 绑定面（矩阵 blocking 清零——export 前置）：资格+否决类全绑 + 三门
    let t = Instant::now();
    use bid_domain::evidence::EvidenceSpec;
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc.store_mut().register_evidence(EvidenceSpec {
        kind: bid_domain::evidence::EvidenceKind::Qualification,
        title: "规模验证合成资质".into(),
        source_file: "EV.pdf".into(),
        source_sha256: "sha-scale-ev".into(),
        locator: EngineLocator::DocxParagraph { paragraph_index: 0, text_fingerprint: "fnv64:s".into() },
        fingerprint: "fnv64:s".into(),
        valid_from: None, valid_until: None,
        subject_name: "投标人".into(), scope: "合成".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
    }).unwrap();
    let must: Vec<String> = svc.store().requirements().iter()
        // #110 任务 1 缺失回报闸：技术参数类也须证据佐证（否则 draft 拒生成）
        .filter(|r| matches!(r.category.as_str(), "资格条件" | "否决项" | "技术参数"))
        .map(|r| r.id.clone()).collect();
    for rid in &must {
        svc.store_mut().bind_evidence(&eid, rid, "2026-12-31").unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval, Gate::PreExportConfirm] {
        svc.store_mut().record_review(gate, "human:scale-lead", Decision::Approved, vec![]).unwrap();
    }
    println!("[规模] ③绑定（{} 条）+三门: {}ms", must.len(), t.elapsed().as_millis());

    // ④ 逐章起草（~250 叶 mock 生成 + S-2 逐章落盘——导出前的最大耗时面）
    let t = Instant::now();
    let leaf_ids: Vec<String> = {
        let parents: std::collections::HashSet<String> = svc.store().sections().iter()
            .filter_map(|n| n.parent_id.clone()).collect();
        let mut v: Vec<String> = svc.store().sections().iter()
            .filter(|n| !parents.contains(&n.id)).map(|n| n.id.clone()).collect();
        v.sort();
        v
    };
    let mut total_words = 0u64;
    for sid in &leaf_ids {
        let r = ok_result(call(&mut svc, "draft_response_section", json!({
            "project_id": &pid, "section_id": sid, "instructions": "规模批次",
        })));
        total_words += r["stats"]["words"].as_u64().unwrap();
    }
    let draft_ms = t.elapsed().as_millis();
    println!("[规模] ④起草 {n_leaves} 章: {draft_ms}ms（总 {total_words} 字，均 {:.0} 字/章）",
        total_words / n_leaves.max(1) as u64);
    assert!(total_words >= 350_000, "总字数 {total_words}（500k 配额 ±20% 下界 400k——合成 REQ 少的叶按内容完整性可低于带，粗界 350k）");

    // ⑤ 校验（全量 8 规则 + ~250 章扫描）
    let t = Instant::now();
    let r = ok_result(call(&mut svc, "validate_bid_document", json!({
        "project_id": &pid, "scope": "all",
    })));
    assert_eq!(r["ok"], json!(true), "规模面校验必须过（BLOCKER 清零）: {}", r["errors"]);
    println!("[规模] ⑤校验: {}ms（findings: {}）", t.elapsed().as_millis(),
        r["findings"].as_array().unwrap().len());

    // ⑥ 导出（Lead 红线：< 120s）——含内存峰值断言（< 2GB）
    let t = Instant::now();
    let r = ok_result(call(&mut svc, "export_bid_document", json!({
        "project_id": &pid, "format": "docx", "output_path": "out/规模验证-50万字.docx",
    })));
    let export_ms = t.elapsed().as_millis();
    let url = r["artifact"]["url"].as_str().unwrap().to_string();
    let bytes = r["artifact"]["size"].as_u64().unwrap();
    println!("[规模] ⑥导出: {export_ms}ms（{bytes} 字节，{} 章落盘）",
        r["chapters_written"].as_u64().unwrap());
    assert!(std::path::Path::new(&url).is_file(), "产物在盘: {url}");

    let Some(hwm1) = vm_hwm_kb() else { unreachable!("hwm0 在场则 hwm1 在场") };
    let peak_mb = hwm1 as f64 / 1024.0;
    println!(
        "[规模] 内存峰值 VmHWM: {:.0}MB（基线 {:.0}MB，增量 {:.0}MB）",
        peak_mb, hwm0 as f64 / 1024.0, (hwm1 - hwm0) as f64 / 1024.0
    );

    // ---- 红线断言（Lead 4.2 原文）
    assert!(export_ms < 120_000, "红线①：导出 {export_ms}ms < 120s");
    assert!(hwm1 < 2 * 1024 * 1024, "红线②：内存峰值 {peak_mb:.0}MB < 2GB");
    // 红线③「不 OOM」：跑到这里即是证明（进程存活 + 全断言通过）
    println!(
        "[规模] 全链 {}ms（红线全过：导出 {:.1}s / 峰值 {:.0}MB / 零 OOM）",
        t_all.elapsed().as_millis(), export_ms as f64 / 1000.0, peak_mb
    );

    // ---- 重启恢复面（规模下的断点续跑语义）
    let svc2 = BidService::open(&ws).unwrap();
    assert_eq!(
        svc2.store().project().unwrap().status,
        bid_domain::ProjectStatus::Exported,
        "规模面重启恢复"
    );
    let asm2 = bid_tools::assembly::ChapterAssembler::open(&ws).unwrap();
    assert_eq!(asm2.committed_count(), n_leaves, "工作区全量恢复（{} 章）", n_leaves);
}
