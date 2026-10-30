//! #80 第三批 · 任务 A'：子代理 LLM 多章实测——skill 偏移 + MCP 缺口双评估。
//!
//! 与第二批（real_llm_path.rs「人肉 LLM」）的差异：
//! - **作者换成无上下文子代理**：正文不再由开发者本人撰写，而是并行派遣的
//!   子代理作者——它们只拿到 RealGenerator 实发的 prompt（system + user），
//!   没有任何仓库/引擎上下文。这才是对「skill 是否成功可用不偏移」的真检验：
//!   prompt 就是章节作者的 skill 面，作者偏移 = skill 有缺口。
//! - **多章并行**：3 个目标章（确定性选法：REQ 数降序、id 升序取前三），
//!   中继按「章节标题」路由（--content-map）——对应 #80 的「子 Agent =
//!   各章节作者」架构（主 Agent = 总工：分派 + 验收）。
//! - **skill 纪律 8 落地**：每章完成后调 save_version 存版本（作者手稿
//!   <ws>/manuscripts/<SEC>.md——文本面可快照）。
//!
//! 两相位（同第二批——draft 失败零副作用，可安全重试）：
//! - **capture**：中继回 HTTP 503 → 3 次 draft 各自断言真实传输面 + 错误语义
//!   （E_UNPROCESSABLE 且 what 含「LLM HTTP 503」），章节状态零变化；
//!   prompt 逐个落盘，主代理据此派遣子代理作者（manuscripts/<SEC>.md）。
//! - **serve**：中继按标题回灌 3 份子代理正文 → 每章走完整真实链路
//!   （generator=real + 引用回查 + 覆盖度 + 字数 ±20% + save_version）→
//!   其余章节 mock 补齐 → 校验 blocker=0 → 导出 docx → 用量台账 → 重启恢复。
//!
//! **MCP 缺口（诚实面，评估报告的实证来源）**：本测试的建纲/建矩阵
//! （prepare_outline_and_matrix）、证据登记与绑定（register_evidence /
//! bind_evidence）、审批门（record_review ×3）走【服务面直调】——MCP 工具面
//! （契约 12 工具）没有对应工具，外部 Agent 纯 MCP 驱动不了全链。
//! 详见 docs/subagent-llm-assessment.md 缺口清单。
//!
//! 传输面：draft/校验/导出/save_version 走【真实 stdio 子进程】（BID_WS
//! 持久化工作区）——与 Claude Desktop 同款接线面。
//!
//! SKIP 纪律（issue #9）：未设 `BID_SUBAGENT_PATH_WS` + `BID_SUBAGENT_PATH_PHASE`
//! （CI / 普通开发面）→ 响亮横幅 + return。驱动脚本：
//! `scripts/e2e_bid/run_subagent_path.sh`（phase1 抓 prompt 派遣作者 / phase2 回灌）。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

/// 目标章数：多章并行的规模面（3 = 一位子代理作者一章的最小可信并行度）。
const N_TARGETS: usize = 3;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn files_dir() -> PathBuf {
    repo_root().join("fixtures/synth-tender/招标文件")
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
        ToolReply::Err { error, .. } => panic!("subagent-path 步骤失败: {error}"),
    }
}

// ---------------------------------------------------------------- stdio 子进程

fn stdio_bin() -> PathBuf {
    if let Ok(p) = std::env::var("BID_SUBAGENT_STDIO_BIN") {
        assert!(!p.trim().is_empty());
        return PathBuf::from(p.trim());
    }
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_bid_mcp_stdio") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().expect("current_exe");
    exe.parent()
        .and_then(|d| d.parent())
        .map(|d| d.join("bid-mcp-stdio"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| {
            panic!(
                "找不到 bid-mcp-stdio 二进制——先 cargo build -p bid-mcp --bin bid-mcp-stdio，\
             或设 BID_SUBAGENT_STDIO_BIN 指向它"
            )
        })
}

/// 真实 stdio 传输面子进程（BID_WS 持久化——与 Claude Desktop 同款接线）。
struct StdioProc {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl StdioProc {
    fn spawn(ws: &Path) -> Self {
        let mut child = Command::new(stdio_bin())
            .env("BID_WS", ws)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("启动 bid-mcp-stdio 子进程失败");
        let reader = BufReader::new(child.stdout.take().expect("stdout pipe"));
        Self { child, reader }
    }

    /// 单调用（一行进一行出——stdio.rs 契约）。
    fn call(&mut self, tool: &str, args: Value) -> Value {
        let req = json!({ "tool": tool, "args": args }).to_string();
        writeln!(self.child.stdin.as_mut().unwrap(), "{req}").expect("写 stdin");
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .flush()
            .expect("flush stdin");
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("读 stdout 应答");
        serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("stdio 应答不是 JSON（{e}）: {line}"))
    }
}

impl Drop for StdioProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn reply_data(reply: &Value) -> &Value {
    &reply["data"]
}

fn reply_error_what(reply: &Value) -> String {
    reply["error"]["what"]
        .as_str()
        .unwrap_or("(无 what 字段)")
        .to_string()
}

// ---------------------------------------------------------------- 准备链

/// 目标章节标记（capture 相位写入，serve 相位读取——选择结果跨相位落盘）。
fn targets_marker(ws: &Path) -> PathBuf {
    ws.join(".mang").join("subagent-targets.json")
}

/// 作者手稿路径（save_version 的文本面——skill 纪律 8 的版本对象）。
fn manuscript(ws: &Path, sec: &str) -> PathBuf {
    ws.join("manuscripts").join(format!("{sec}.md"))
}

fn leaf_ids_of(svc: &BidService) -> Vec<String> {
    let parents: std::collections::HashSet<String> = svc
        .store()
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.clone())
        .collect();
    svc.store()
        .sections()
        .iter()
        .filter(|n| !parents.contains(&n.id))
        .map(|n| n.id.clone())
        .collect()
}

/// 确定性选目标章：REQ 数降序、id 升序取前 N——规模面最大的章先被检验。
fn top_targets(svc: &BidService, n: usize) -> Vec<String> {
    let mut leaves: Vec<(usize, String)> = leaf_ids_of(svc)
        .into_iter()
        .map(|id| {
            let cnt = svc
                .store()
                .sections()
                .iter()
                .find(|x| x.id == id)
                .unwrap()
                .requirement_ids
                .len();
            (cnt, id)
        })
        .collect();
    leaves.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    leaves.into_iter().take(n).map(|(_, id)| id).collect()
}

/// 准备链（幂等：工作区已有项目 → 跳过）。返回 (project_id, 目标章节列表)。
fn ensure_prepared(svc: &mut BidService, ws: &Path) -> (String, Vec<String>) {
    if let Ok(proj) = svc.store().project() {
        // 工作区已就绪（serve 相位 / 断点续跑）——读标记
        let m: Value = serde_json::from_str(
            &std::fs::read_to_string(targets_marker(ws)).expect("subagent-targets.json 在场"),
        )
        .expect("标记 JSON 合法");
        let targets: Vec<String> = m["targets"]
            .as_array()
            .expect("targets 数组")
            .iter()
            .map(|t| t["section_id"].as_str().unwrap().to_string())
            .collect();
        return (proj.id.clone(), targets);
    }

    // ---- 建项目（MCP §1）
    let pid = ok_result(call(
        svc,
        "create_bid_project",
        json!({
            "name": "子代理 LLM 多章实测——智慧病房物联网平台",
            "bid_deadline": "2026-11-30", "mode": "明标", "external_key": "synth-subagent-path",
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();

    // ---- 导入（MCP §2——A/B/C/E 批 + D 单独批，E_PARTIAL 容忍同 synth 评测）
    let names = [
        ("A-招标文件.docx", "招标文件"),
        ("B-技术规范书.docx", "招标文件"),
        ("C-补遗文件.pdf", "补遗"),
        ("E-原招标公告盖章页-扫描件.pdf", "附件"),
    ];
    let specs: Vec<Value> = names
        .iter()
        .map(|(n, k)| {
            json!({
                "upload_ref": files_dir().join(n).display().to_string(), "filename": n, "kind": k
            })
        })
        .collect();
    match call(
        svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": specs }),
    ) {
        ToolReply::Ok(_)
        | ToolReply::Err {
            error: bid_mcp::ToolError::EPartial { .. },
            ..
        } => {}
        ToolReply::Err { error, .. } => panic!("导入意外错误: {error}"),
    }
    match call(
        svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [{
            "upload_ref": files_dir().join("D-图纸说明.md").display().to_string(),
            "filename": "D-图纸说明.md", "kind": "图纸",
        }]}),
    ) {
        ToolReply::Err {
            error: bid_mcp::ToolError::EPartial { .. },
            ..
        } => {} // 预期 fail-closed
        other => panic!("D 件应 E_PARTIAL: {other:?}"),
    }

    // ---- 抽取（MCP §3）
    let extract = ok_result(call(
        svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    let n_reqs = extract["counts"]["total"].as_u64().unwrap_or(0);
    assert!(n_reqs >= 60, "抽取面规模: {n_reqs}");

    // ---- 建纲建矩阵（服务面——MCP 无对应工具【缺口 G1】，评估报告实证）
    svc.prepare_outline_and_matrix(OutlineSpec {
        target_total_chars: 20_000,
        max_reqs_per_leaf: 8,
    })
    .expect("建纲建矩阵");

    // ---- 证据登记 + 绑定（服务面——MCP 无对应工具【缺口 G2】）
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc
        .store_mut()
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "电子与智能化工程专业承包二级资质证书（synth·subagent-path）".into(),
            source_file: "SYN-EV-资质证书.pdf".into(),
            source_sha256: "sha-subagent-path".into(),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 0,
                text_fingerprint: "fnv64:sp".into(),
            },
            fingerprint: "fnv64:sp".into(),
            valid_from: None,
            valid_until: None,
            subject_name: "投标人".into(),
            scope: "电子与智能化工程".into(),
        })
        .unwrap();
    let qual_ids: Vec<String> = svc
        .store()
        .requirements()
        .iter()
        .filter(|r| r.category == "资格条件" || r.category == "否决项")
        .map(|r| r.id.clone())
        .collect();
    for rid in &qual_ids {
        svc.store_mut()
            .bind_evidence(&eid, rid, "2026-11-30")
            .unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();

    // ---- 三门审批（服务面——MCP 无对应工具【缺口 G3】）
    use bid_domain::review::{Decision, Gate};
    for gate in [
        Gate::MatrixApproval,
        Gate::OutlineApproval,
        Gate::PreExportConfirm,
    ] {
        svc.store_mut()
            .record_review(gate, "human:subagent-path-lead", Decision::Approved, vec![])
            .unwrap();
    }
    let n_blocking = svc.store().matrix().iter().filter(|r| r.blocking).count();
    assert_eq!(n_blocking, 0, "绑定后 blocking 清零: 剩 {n_blocking}");

    // ---- 选目标章节（确定性：REQ 数降序、id 升序前 N）
    let targets = top_targets(svc, N_TARGETS);
    assert_eq!(targets.len(), N_TARGETS, "目标章数量");

    // ---- 标记落盘（标题是中继路由键——必须唯一，否则多章回灌会串章）
    let mut entries: Vec<Value> = Vec::new();
    let mut titles: Vec<String> = Vec::new();
    for sid in &targets {
        let node = svc
            .store()
            .sections()
            .iter()
            .find(|n| n.id == *sid)
            .unwrap();
        if titles.contains(&node.title) {
            panic!("目标章标题重复（{}）——中继按标题路由会串章", node.title);
        }
        titles.push(node.title.clone());
        let reqs: Vec<Value> = node
            .requirement_ids
            .iter()
            .map(|rid| {
                let r = svc
                    .store()
                    .requirements()
                    .iter()
                    .find(|r| &r.id == rid)
                    .unwrap();
                json!({ "id": r.id, "category": r.category, "clause_no": r.clause_no,
                    "excerpt": r.anchors.first().map(|a| a.excerpt.clone()).unwrap_or_default() })
            })
            .collect();
        entries.push(json!({
            "section_id": sid, "title": node.title, "target_chars": node.target_chars,
            "requirements": reqs,
        }));
    }
    let marker = json!({ "targets": entries });
    std::fs::create_dir_all(ws.join(".mang")).unwrap();
    std::fs::write(
        targets_marker(ws),
        serde_json::to_string_pretty(&marker).unwrap(),
    )
    .unwrap();

    svc.checkpoint()
        .expect("准备链后显式落盘（子进程要从盘上恢复）");
    (pid, targets)
}

// ---------------------------------------------------------------- 主测试

#[test]
fn subagent_llm_multi_chapter_path() {
    let (Some(ws_s), Some(phase)) = (
        std::env::var("BID_SUBAGENT_PATH_WS")
            .ok()
            .filter(|s| !s.trim().is_empty()),
        std::env::var("BID_SUBAGENT_PATH_PHASE").ok(),
    ) else {
        println!(
            "================ SKIP subagent_llm_path ================\n\
             未设 BID_SUBAGENT_PATH_WS / BID_SUBAGENT_PATH_PHASE（CI 默认跳过——子代理\n\
             LLM 通路需要中继服务与子代理作者撰写面）。驱动入口：\n\
             scripts/e2e_bid/run_subagent_path.sh phase1|phase2\n\
             ========================================================"
        );
        return;
    };
    let ws = PathBuf::from(ws_s.trim());
    assert!(ws.is_dir(), "BID_WS 工作区不存在: {}", ws.display());
    assert!(
        matches!(phase.as_str(), "capture" | "serve"),
        "BID_SUBAGENT_PATH_PHASE 须为 capture|serve，实得 {phase}"
    );

    // ---- llm.json 在场性（真实通路的配置面——由驱动脚本写入，指向本地中继）
    let llm_json = ws.join(".mang").join("llm.json");
    assert!(
        llm_json.is_file(),
        "缺 {}（真实通路前置——run_subagent_path.sh 负责写入）",
        llm_json.display()
    );

    // ---- 准备链（幂等）
    let t0 = Instant::now();
    let mut svc = BidService::open(&ws).expect("打开工作区");
    let (pid, targets) = ensure_prepared(&mut svc, &ws);
    let leaves = leaf_ids_of(&svc);
    println!(
        "[subagent-path] 准备链 {}ms｜目标章节 {:?}｜叶 {} 个",
        t0.elapsed().as_millis(),
        targets,
        leaves.len()
    );

    // ---- 起草指令（总工面——对所有目标章一致的通用指令，不含逐章提示）
    let instructions = "按 synth-tender 招标文件逐项响应：资格与类似业绩逐条对应 REQ 引用；\
                        实施方案给出可核验的量化承诺（进度/人员/设备），不得空话";

    let mut proc = StdioProc::spawn(&ws);
    let t_call = Instant::now();

    if phase == "capture" {
        // ================= capture：真实传输面 + 错误语义 + 零副作用 ================
        for (i, sec) in targets.iter().enumerate() {
            let reply = proc.call(
                "draft_response_section",
                json!({
                    "project_id": &pid, "section_id": sec,
                    "instructions": instructions, "generator": "real",
                }),
            );
            assert_eq!(
                reply["ok"],
                json!(false),
                "capture 相位第 {i} 章应失败（503）: {reply}"
            );
            let what = reply_error_what(&reply);
            assert_eq!(
                reply["error"]["code"],
                json!("E_UNPROCESSABLE"),
                "错误码面: {reply}"
            );
            assert!(
                what.contains("LLM HTTP 503"),
                "错误语义应含「LLM HTTP 503」（真实 HttpTransport 走通）: {what}"
            );
            println!("[subagent-path] capture {sec} → 503（prompt 已落盘 prompt-{i}.json）");
        }
        // 零副作用：全部目标章仍非 DRAFTED（生成器报错在 CAS 写入之前返回）
        drop(proc);
        let svc2 = BidService::open(&ws).unwrap();
        for sec in &targets {
            let node = svc2
                .store()
                .sections()
                .iter()
                .find(|n| n.id == *sec)
                .unwrap();
            assert_ne!(
                node.status.as_str(),
                "DRAFTED",
                "失败调用不得推进章节状态（生成器报错在 CAS 写入之前返回）"
            );
        }
        println!(
            "[subagent-path] capture ✅ {} 章真实传输面（ureq → 中继 → 503 错误语义）｜{}ms\n\
             [subagent-path] prompt 已由中继落盘——主代理派遣子代理作者后跑 serve 相位",
            targets.len(),
            t_call.elapsed().as_millis()
        );
        return;
    }

    // ================= serve：完整真实链路（子代理正文 → 真实生成器） ================
    // 标记里带每章 target_chars（字数带宽断言用）
    let marker: Value =
        serde_json::from_str(&std::fs::read_to_string(targets_marker(&ws)).unwrap()).unwrap();
    let target_chars: std::collections::HashMap<String, u64> = marker["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["section_id"].as_str().unwrap().to_string(),
                t["target_chars"].as_u64().unwrap(),
            )
        })
        .collect();

    for sec in &targets {
        let t_one = Instant::now();
        let reply = proc.call(
            "draft_response_section",
            json!({
                "project_id": &pid, "section_id": sec,
                "instructions": instructions, "generator": "real",
            }),
        );
        assert_eq!(
            reply["ok"],
            json!(true),
            "serve 相位 {sec} draft 应成功: {reply}"
        );
        let d = reply_data(&reply);
        assert_eq!(d["stats"]["generator"], json!("real"), "生成器标记面: {d}");
        assert_eq!(d["section_status"], json!("DRAFTED"));
        let citations = d["stats"]["citations"].as_u64().unwrap();
        assert!(citations > 0, "引用面（子代理正文带 [REQ-xxxx]）: {d}");
        let uncovered = d["stats"]["uncovered_requirements"].as_array().unwrap();
        assert!(
            uncovered.is_empty(),
            "覆盖度：子代理正文必须回应全部 REQ（{sec} 缺失 {uncovered:?}）"
        );
        let words = d["stats"]["words"].as_u64().unwrap();
        let tc = *target_chars.get(sec).unwrap();
        let dev = (words as f64 - tc as f64).abs() / tc as f64;
        assert!(
            dev <= 0.20,
            "{sec} 字数偏离 {dev:.1}% 超 ±20%（{words} vs {tc}）——skill prompt 的字数面失守"
        );

        // ---- skill 纪律 8：每完成一章 save_version（作者手稿文本面）
        let ms = manuscript(&ws, sec);
        assert!(
            ms.is_file(),
            "作者手稿应在场（中继 serve 的正文本体）: {}",
            ms.display()
        );
        let sv = proc.call(
            "save_version",
            json!({
                "file_path": ms.display().to_string(),
                "message": format!("{sec} 章节完成——子代理作者撰写（{words} 字 / {citations} 引用 / 偏离 {:.1}%）", dev * 100.0),
            }),
        );
        assert_eq!(sv["ok"], json!(true), "save_version 应成功: {sv}");
        println!(
            "[subagent-path] serve {sec} ✅ 真实链路（{words} 字｜引用 {citations}｜偏离 {:.1}%｜版本 {}）｜{}ms",
            dev * 100.0,
            sv["data"]["snapshot"].as_str().unwrap_or("?"),
            t_one.elapsed().as_millis()
        );
    }

    // ---- 其余章节 mock 补齐（同一 stdio 子进程——完成文档面）
    let t_mock = Instant::now();
    let n_mock = leaves.iter().filter(|s| !targets.contains(*s)).count();
    for sid in &leaves {
        if targets.contains(sid) {
            continue;
        }
        let r = proc.call(
            "draft_response_section",
            json!({
                "project_id": &pid, "section_id": sid,
                "instructions": "按 synth-tender 招标文件逐项响应，突出同类业绩与实施方案",
                "generator": "mock",
            }),
        );
        assert_eq!(
            reply_data(&r)["section_status"],
            json!("DRAFTED"),
            "mock 章节 {sid}: {r}"
        );
    }
    println!(
        "[subagent-path] mock 补齐 {n_mock} 章｜{}ms",
        t_mock.elapsed().as_millis()
    );

    // ---- 校验（MCP §7——BLOCKER 清零）
    let v = proc.call(
        "validate_bid_document",
        json!({ "project_id": &pid, "scope": "all" }),
    );
    assert_eq!(reply_data(&v)["ok"], json!(true), "校验未过: {v}");
    assert_eq!(reply_data(&v)["counts"]["blocker"], json!(0));

    // ---- 导出（MCP §8——3 章子代理真实内容进成品 docx）
    let e = proc.call(
        "export_bid_document",
        json!({
            "project_id": &pid,
            "format": "docx", "output_path": "out/子代理通路-投标文件-技术标.docx",
        }),
    );
    let url = reply_data(&e)["artifact"]["url"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        url.ends_with("子代理通路-投标文件-技术标.docx"),
        "产物名: {url}"
    );
    let size = reply_data(&e)["artifact"]["size"].as_u64().unwrap();
    assert!(size > 5_000, "产物 >5KB: {size}");

    // ---- 用量台账（真实调用面的审计产物——3 位作者 = ≥3 次调用）
    drop(proc);
    let usage_path = ws.join(".mang").join("llm-usage.json");
    assert!(
        usage_path.is_file(),
        "llm-usage.json 应落盘（真实调用记账）"
    );
    let usage: Value =
        serde_json::from_str(&std::fs::read_to_string(&usage_path).unwrap()).unwrap();
    assert!(
        usage["calls"].as_u64().unwrap() >= targets.len() as u64,
        "台账 calls >= {}: {usage}",
        targets.len()
    );

    // ---- 重启恢复（EXPORTED + 全章在盘）
    let svc2 = BidService::open(&ws).unwrap();
    assert_eq!(
        svc2.store().project().unwrap().status.as_str(),
        "EXPORTED",
        "重启后 EXPORTED 完整恢复"
    );
    let asm = bid_tools::assembly::ChapterAssembler::open(&ws).unwrap();
    assert_eq!(asm.committed_count(), leaves.len(), "重启后工作区全量恢复");

    println!(
        "[subagent-path] serve 全链 ✅ 导出 {}（{} 字节）｜台账 {} 次调用｜真实章节 {:?}",
        url, size, usage["calls"], targets
    );
    println!(
        "[subagent-path] 成品中上述 {} 章为【子代理作者真实撰写】，其余为 mock 补齐",
        targets.len()
    );
}
