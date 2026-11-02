//! #80 第三批 · 任务 A'：子代理 LLM 多章实测——skill 偏移 + MCP 缺口双评估。
//!
//! 与第二批（real_llm_path.rs「人肉 LLM」）的差异：
//! - **作者换成无上下文子代理**：正文不再由开发者本人撰写，而是并行派遣的
//!   子代理作者——它们只拿到 RealGenerator 实发的 prompt（system + user），
//!   没有任何仓库/引擎上下文。这才是对「skill 是否成功可用不偏移」的真检验：
//!   prompt 就是章节作者的 skill 面，作者偏移 = skill 有缺口。
//! - **多章并行**：目标章数可配（缺省 3——确定性选法：REQ 数降序、id 升序
//!   取前 N），中继按「章节标题」路由（--content-map）——对应 #80 的
//!   「子 Agent = 各章节作者」架构（主 Agent = 总工：分派 + 验收）。
//! - **规模模式**（Lead 反馈：3 章 ~4.6k 字不构成对 skill 的规模检验）：
//!   `BID_SUBAGENT_N_TARGETS=0`（全部叶章）+ `BID_SUBAGENT_TOTAL_CHARS` /
//!   `BID_SUBAGENT_MAX_REQS_PER_LEAF`（建纲配额）——synth-tender 全量
//!   ~108 叶 × ~7.4k 字/叶 ≈ 80 万字全部由子代理作者撰写。字数带外率
//!   容差 `BID_SUBAGENT_BAND_TOLERANCE`（缺省严格逐章 ±20%；规模模式设
//!   如 0.15——偏移是评估数据，记录不中断，带外率超容差才 fail）。
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
//! **MCP 工具面全贯通（G1/G2/G3 补齐后）**：建纲建矩阵（build_outline §10）、
//! 证据登记与绑定（register_evidence / bind_evidence §11）、审批门
//! （record_review_gate ×3 §12）全部走【MCP 工具面】（进程内 dispatch_with
//! ——与 stdio 同一壳层裁决）；起草/校验/导出/save_version 走【真实 stdio
//! 子进程】。G4 编排纪律：serve 相位起草完成后调 check_bid_quality。
//! 详见 docs/subagent-llm-assessment.md。
//!
//! 传输面：draft/校验/导出/save_version 走【真实 stdio 子进程】（BID_WS
//! 持久化工作区）——与 Claude Desktop 同款接线面。
//!
//! SKIP 纪律（issue #9）：未设 `BID_SUBAGENT_PATH_WS` + `BID_SUBAGENT_PATH_PHASE`
//! （CI / 普通开发面）→ 响亮横幅 + return。驱动脚本：
//! `scripts/e2e_bid/run_subagent_path.sh`（phase1 抓 prompt 派遣作者 / phase2 回灌）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

/// 目标章数（缺省形态）：多章并行的规模面（3 = 一位子代理作者一章的最小
/// 可信并行度）。规模实验用 BID_SUBAGENT_N_TARGETS=0（全部叶章）覆盖。
const N_TARGETS: usize = 3;

// ---------------------------------------------------------------- 规模旋钮

/// 环境变量读 u64（缺省回退）——规模实验的驱动面，默认值保持第三批任务 A'
/// 的 3 章形态（向后兼容：不设旋钮时行为不变）。
fn env_u64_or(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

/// 建纲配额：目标总字数（BID_SUBAGENT_TOTAL_CHARS，缺省 20,000——与旧口径
/// 一致；规模实验设 800,000 进入 50w-100w 字区间）。
fn outline_total_chars() -> u64 {
    env_u64_or("BID_SUBAGENT_TOTAL_CHARS", 20_000)
}

/// 建纲配额：每叶 REQ 上限（BID_SUBAGENT_MAX_REQS_PER_LEAF，缺省 8；规模
/// 实验设 1——每章单条款，108 叶 × ~7.4k 字，配额在单次补全能力带内）。
fn outline_max_reqs_per_leaf() -> u64 {
    env_u64_or("BID_SUBAGENT_MAX_REQS_PER_LEAF", 8)
}

/// 目标章数（BID_SUBAGENT_N_TARGETS，缺省 3；0 = 全部叶章——规模实验）。
fn n_targets_cfg() -> usize {
    env_u64_or("BID_SUBAGENT_N_TARGETS", N_TARGETS as u64) as usize
}

/// 字数带外率容差（BID_SUBAGENT_BAND_TOLERANCE，如 0.15；缺省 None =
/// 严格逐章 ±20% 断言——旧口径。规模模式给容差：偏移是评估数据，记录
/// 不中断，带外率超容差才 fail）。
fn band_tolerance() -> Option<f64> {
    std::env::var("BID_SUBAGENT_BAND_TOLERANCE")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|t| (0.0..=1.0).contains(t))
}

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

/// 待绑 REQ 发现（MCP §4 矩阵分页——类目过滤 + cursor 翻页，外部 Agent
/// 同款视角；与 mcp_scaffold_tools 同构）。类目用矩阵行自己的 category 面
/// （qualification / veto —— 契约 §4 filters 口径）。
fn matrix_reqs(svc: &mut BidService, pid: &str, category: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args =
            json!({ "project_id": pid, "filters": { "category": category }, "limit": 10 });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let r = ok_result(call(svc, "get_compliance_matrix", args));
        for row in r["rows"].as_array().expect("rows 数组") {
            out.push(row["requirement"]["id"].as_str().unwrap().to_string());
        }
        match r["next_cursor"].as_str() {
            Some(c) if !c.is_empty() => cursor = Some(c.to_string()),
            _ => break,
        }
    }
    out
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

    // ---- 建纲建矩阵（MCP §10——G1 工具面；配额旋钮：规模实验用环境变量覆盖）
    let bo = ok_result(call(
        svc,
        "build_outline",
        json!({
            "project_id": &pid,
            "target_total_chars": outline_total_chars(),
            "max_reqs_per_leaf": outline_max_reqs_per_leaf(),
        }),
    ));
    assert_eq!(
        bo["status"],
        json!("MATRIX_READY"),
        "建纲后状态（G1 工具面）: {bo:?}"
    );

    // ---- 待绑 REQ 发现（MCP §4 矩阵分页——资格 + 否决 + 技术参数，外部 Agent 同款视角；
    // 技术参数也须佐证——#110 任务 1 缺失回报闸，否则 draft 拒生成）
    let mut bind_ids = matrix_reqs(svc, &pid, "qualification");
    let n_qual = bind_ids.len();
    bind_ids.extend(matrix_reqs(svc, &pid, "veto"));
    assert!(n_qual >= 5, "资格条件规模: {n_qual}");
    assert!(
        bind_ids.len() > n_qual,
        "否决项并入: {}",
        bind_ids.len() - n_qual
    );
    bind_ids.extend(matrix_reqs(svc, &pid, "tech_param"));

    // ---- 证据登记 + 绑定（MCP §11——G2 工具面）
    let reg = ok_result(call(
        svc,
        "register_evidence",
        json!({
            "project_id": &pid, "kind": "qualification",
            "title": "电子与智能化工程专业承包二级资质证书（synth·subagent-path）",
            "source_file": "SYN-EV-资质证书.pdf",
            "source_sha256": "sha-subagent-path",
            "subject_name": "投标人", "scope": "电子与智能化工程",
        }),
    ));
    let eid = reg["ev_id"].as_str().unwrap().to_string();
    assert!(eid.starts_with("EV-"), "EV 签发（G2 工具面）: {reg:?}");
    let bind = ok_result(call(
        svc,
        "bind_evidence",
        json!({
            "project_id": &pid, "ev_id": &eid,
            "requirement_ids": bind_ids, "bid_open_date": "2026-11-30",
        }),
    ));
    assert_eq!(
        bind["matrix"]["blocking"],
        json!(0),
        "绑定后 blocking 清零（G2 工具面）: {bind:?}"
    );

    // ---- 两门审批（MCP §12——G3 工具面；门序：矩阵 → 大纲）
    // #180 审核内容绑定：pre_export_confirm 绑定章节正文摘要，挑至全部叶
    // 起草完成后记审（见 serve 相位导出前——与人类流程同序：审核定稿内容）
    for gate in ["matrix_approval", "outline_approval"] {
        let g = ok_result(call(
            svc,
            "record_review_gate",
            json!({
                "project_id": &pid, "gate": gate, "decision": "APPROVED",
                "reviewer": "human:subagent-path-lead",
            }),
        ));
        assert_eq!(
            g["receipt"]["decision"],
            json!("APPROVED"),
            "门 {gate}: {g:?}"
        );
    }

    // ---- 选目标章节（确定性：REQ 数降序、id 升序前 N；N=0 = 全部叶——规模模式）
    let n_targets = n_targets_cfg();
    let targets = if n_targets == 0 {
        let mut all = top_targets(svc, usize::MAX);
        all.sort(); // 全量模式按 id 序（分派顺序稳定可复现）
        all
    } else {
        top_targets(svc, n_targets)
    };
    assert!(!targets.is_empty(), "目标章数量（旋钮 N={n_targets}）");

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

    // 字数带分布记录（评估数据——严格模式逐章断言，容差模式汇总裁决）
    let strict_band = band_tolerance().is_none();
    let mut band_rows: Vec<Value> = Vec::new();
    let mut total_words: u64 = 0;
    let mut out_of_band: Vec<Value> = Vec::new();

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
        let within = dev <= 0.20;
        total_words += words;
        band_rows.push(json!({
            "section_id": sec, "words": words, "target_chars": tc,
            "dev_pct": (dev * 1000.0).round() / 10.0, "within_band": within,
            "citations": citations,
        }));
        if !within {
            out_of_band.push(json!({ "section_id": sec, "words": words,
                "target_chars": tc, "dev_pct": (dev * 1000.0).round() / 10.0 }));
        }
        if strict_band {
            assert!(
                dev <= 0.20,
                "{sec} 字数偏离 {dev:.1}% 超 ±20%（{words} vs {tc}）——skill prompt 的字数面失守"
            );
        }

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
            "[subagent-path] serve {sec} {}（{words} 字｜引用 {citations}｜偏离 {:.1}%｜版本 {}）｜{}ms",
            if within { "✅" } else { "⚠️ 带外" },
            dev * 100.0,
            sv["data"]["snapshot"].as_str().unwrap_or("?"),
            t_one.elapsed().as_millis()
        );
    }

    // ---- 字数带汇总（容差模式：带外率超容差才 fail；分布与带外清单落盘）
    let n = targets.len();
    let oob_rate = out_of_band.len() as f64 / n as f64;
    if let Some(tol) = band_tolerance() {
        assert!(
            oob_rate <= tol,
            "字数带外率 {:.1}% 超容差 {:.1}%（{}/{} 章）——skill 字数面在规模下失守，见 band-report",
            oob_rate * 100.0,
            tol * 100.0,
            out_of_band.len(),
            n
        );
    }
    let devs: Vec<f64> = band_rows
        .iter()
        .map(|r| r["dev_pct"].as_f64().unwrap())
        .collect();
    let dev_max = devs.iter().cloned().fold(0.0, f64::max);
    let dev_mean = devs.iter().sum::<f64>() / n as f64;
    let band_report = json!({
        "n_chapters": n,
        "total_words": total_words,
        "total_target_chars": target_chars.values().sum::<u64>(),
        "within_band": n - out_of_band.len(),
        "out_of_band": out_of_band,
        "dev_pct": { "mean": (dev_mean * 10.0).round() / 10.0,
                     "max": (dev_max * 10.0).round() / 10.0 },
        "chapters": band_rows,
    });
    let band_path = ws.join("band-report.json");
    std::fs::write(
        &band_path,
        serde_json::to_string_pretty(&band_report).unwrap(),
    )
    .unwrap();
    println!(
        "[subagent-path] 字数带：{}/{} 带内（带外率 {:.1}%）｜偏离均值 {:.1}%、最大 {:.1}%｜总字数 {total_words}｜分布 → {}",
        n - out_of_band.len(),
        n,
        oob_rate * 100.0,
        dev_mean,
        dev_max,
        band_path.display()
    );

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

    // ---- G4 编排纪律（评估报告登记项）：起草完成后总工调 check_bid_quality
    //      （章节发现面 + 质量分布——外部 Agent 同款只读视角）
    let q = proc.call("check_bid_quality", json!({ "project_id": &pid }));
    assert_eq!(q["ok"], json!(true), "check_bid_quality 应成功: {q}");
    let qd = reply_data(&q);
    let q_secs = qd["sections"].as_array().map(|a| a.len()).unwrap_or(0);
    assert!(
        q_secs >= n,
        "质量面章节发现 ≥ 目标章数（{q_secs} vs {n}）: {q}"
    );
    let q_oob = qd["summary"]["out_of_band"].as_u64().unwrap_or(0);
    println!(
        "[subagent-path] G4 check_bid_quality ✅ 章节发现 {q_secs}｜带外 {q_oob}｜needs_human {}",
        qd["needs_human"].as_u64().unwrap_or(0)
    );

    // ---- 校验（MCP §7——BLOCKER 清零）
    let v = proc.call(
        "validate_bid_document",
        json!({ "project_id": &pid, "scope": "all" }),
    );
    assert_eq!(reply_data(&v)["ok"], json!(true), "校验未过: {v}");
    assert_eq!(reply_data(&v)["counts"]["blocker"], json!(0));

    // ---- PreExportConfirm 记审（#180：定稿内容过门——全部叶起草（子代理
    //      真实正文 + mock 补齐）完成后、导出前，绑定的才是将进产物的摘要）
    let g = proc.call(
        "record_review_gate",
        json!({
            "project_id": &pid, "gate": "pre_export_confirm", "decision": "APPROVED",
            "reviewer": "human:subagent-path-lead",
        }),
    );
    assert_eq!(g["ok"], json!(true), "pre_export_confirm 记审: {g}");

    // ---- 导出（MCP §8——全部子代理真实内容进成品 docx）
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

    // ---- 用量台账（真实调用面的审计产物——N 位作者 = ≥N 次调用）
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
        "[subagent-path] serve 全链 ✅ 导出 {}（{} 字节）｜台账 {} 次调用｜真实章节 {} 章（其余 mock {} 章）",
        url, size, usage["calls"], targets.len(), n_mock
    );
    println!(
        "[subagent-path] 成品中上述 {} 章为【子代理作者真实撰写】共 {total_words} 字",
        targets.len()
    );
}
