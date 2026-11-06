//! #80 第二批 · 任务 A：真实生成通路跑通一章（人肉 LLM——无 API key）。
//!
//! **这是 issue #80 的核心验收面**：RealGenerator 的真实代码路径
//! （LlmConfig::resolve → HttpTransport/ureq → Bearer → JSON envelope 容错
//! 解析 → 失控检查 → 用量台账）此前从未执行过。本测试用「人肉 LLM 中继」
//! （scripts/e2e_bid/human_llm_relay.py，OpenAI 兼容本地端点）把补全作者换成
//! 开发者本人——生成器、引用回查、CAS、校验、导出全部走真实链路。
//!
//! 两相位设计（draft 失败零副作用——生成器报错在 CAS 写入之前返回，可安全重试）：
//! - **capture**：中继回 HTTP 503 → 本测试断言真实传输面 + 错误语义
//!   （E_UNPROCESSABLE 且 what 含「LLM HTTP 503」），章节状态零变化；
//!   prompt 由中继落盘，开发者离线撰写章节正文（.mang/human-chapter.md）。
//! - **serve**：中继返回开发者正文 → draft 走完整真实链路（generator=real），
//!   断言引用/覆盖度/字数；其余章节 mock 补齐 → 校验 blocker=0 → 导出 docx。
//!
//! 传输面：draft/校验/导出走【真实 stdio 子进程】（BID_WS 持久化工作区——
//! state.rs），不是进程内直调——Claude Desktop 同款接线面。
//!
//! SKIP 纪律（issue #9）：未设 `BID_REAL_PATH_WS` + `BID_REAL_PATH_PHASE`
//! （CI / 普通开发面）→ 响亮横幅 + return。驱动脚本：
//! `scripts/e2e_bid/run_real_llm_path.sh`（phase1 抓 prompt / phase2 喂正文）。

use bid_domain::outline::OutlineSpec;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

fn repo_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    // #183 围栏：词法 `..` 会被围栏预拒——受信夹具根统一 canonical 绝对路径
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn files_dir() -> PathBuf {
    repo_root().join("fixtures/synth-tender/招标文件")
}

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut()
            .unwrap()
            .insert("generator".into(), json!("mock"));
    }
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
        ToolReply::Err { error, .. } => panic!("real-path 步骤失败: {error}"),
    }
}

/// 地图 P0-1 降级路径：语料含已知坏件（D-图纸说明.md——fail-closed 拒 .md
/// 后按 R-01 登记 Failed；无 OCR 环境下 E-扫描件亦然）。输入完整性门禁起
/// 有 Failed 件不得完整起草——逐件人工确认（#182 receipt）放行，后续起草
/// 走「受限起草」（input_integrity.status=RESTRICTED，不静默当完整输入）。
fn waive_unready_inputs(svc: &mut BidService, pid: &str, actor: &str) -> usize {
    let unready: Vec<String> = svc
        .store()
        .files()
        .iter()
        .filter(|f| bid_tools::readiness::is_tender_package_file(f))
        .filter(|f| {
            bid_tools::readiness::addendum_readiness(f)
                != bid_tools::readiness::AddendumReadiness::Ready
        })
        .filter(|f| !svc.store().addendum_waived(&f.id))
        .map(|f| f.id.clone())
        .collect();
    let n = unready.len();
    for fid in unready {
        let receipt = svc.issue_approval_receipt(actor, &format!("addendum_waiver:{fid}"));
        ok_result(call(
            svc,
            "record_addendum_waiver",
            json!({
                "project_id": pid, "file_id": &fid,
                "reason": "e2e 已知坏件（.md 不支持/扫描件无 OCR）人工确认放行——受限起草",
                "receipt": receipt,
            }),
        ));
    }
    n
}

// ---------------------------------------------------------------- stdio 子进程

/// stdio 二进制定位：驱动脚本显式传入 > cargo 注入位 > 测试 exe 同 target
/// 回退（`cargo test --test X` 过滤面不构建 bin——cargo 也不注入 CARGO_BIN_EXE，
/// 故驱动脚本先 `cargo build --bin bid-mcp-stdio` 再传 BID_REAL_PATH_STDIO_BIN）。
fn stdio_bin() -> PathBuf {
    if let Ok(p) = std::env::var("BID_REAL_PATH_STDIO_BIN") {
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
             或设 BID_REAL_PATH_STDIO_BIN 指向它"
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
fn target_marker(ws: &Path) -> PathBuf {
    ws.join(".mang").join("real-path-target.json")
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

/// 准备链（幂等：工作区已有项目 → 跳过）。返回 (project_id, 目标 section_id)。
fn ensure_prepared(svc: &mut BidService, ws: &Path) -> (String, String) {
    if let Ok(proj) = svc.store().project() {
        // 工作区已就绪（serve 相位 / 断点续跑）——读标记
        let m: Value = serde_json::from_str(
            &std::fs::read_to_string(target_marker(ws)).expect("real-path-target.json 在场"),
        )
        .expect("标记 JSON 合法");
        return (
            proj.id.clone(),
            m["section_id"].as_str().unwrap().to_string(),
        );
    }

    // ---- 建项目（MCP §1）
    let pid = ok_result(call(
        svc,
        "create_bid_project",
        json!({
            "name": "真实生成通路——智慧病房物联网平台（人肉 LLM）",
            "bid_deadline": "2026-11-30", "mode": "明标", "external_key": "synth-real-path",
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

    // 地图 P0-1：D 件已按 R-01 登记 Failed——起草闸（输入完整性）要求逐件
    // 确认放行（后续起草为受限起草 RESTRICTED，响应面可见不静默）
    let waived_n = waive_unready_inputs(svc, &pid, "real-path-lead");
    assert!(waived_n >= 1, "至少 D 件需逐件确认（受限起草面）：{waived_n}");

    // ---- 抽取（MCP §3）
    let extract = ok_result(call(
        svc,
        "extract_requirements",
        json!({ "project_id": &pid }),
    ));
    let n_reqs = extract["counts"]["total"].as_u64().unwrap_or(0);
    assert!(n_reqs >= 60, "抽取面规模: {n_reqs}");

    // ---- 建纲建矩阵（服务面——无 MCP 工具，显式标注）
    svc.prepare_outline_and_matrix(OutlineSpec {
        target_total_chars: 20_000,
        max_reqs_per_leaf: 8,
    })
    .expect("建纲建矩阵");

    // ---- 证据登记 + 绑定 + 三门（服务面）
    use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
    use bid_domain::locator::EngineLocator;
    let (eid, _) = svc
        .store_mut()
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "电子与智能化工程专业承包二级资质证书（synth·real-path）".into(),
            source_file: "SYN-EV-资质证书.pdf".into(),
            source_sha256: "sha-real-path".into(),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 0,
                text_fingerprint: "fnv64:rp".into(),
            },
            fingerprint: "fnv64:rp".into(),
            valid_from: None,
            valid_until: None,
            subject_name: "投标人".into(),
            scope: "电子与智能化工程".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: Default::default(), // #147
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        })
        .unwrap();
    let qual_ids: Vec<String> = svc
        .store()
        .requirements()
        .iter()
        // #110 任务 1 缺失回报闸：技术参数类也须证据佐证（否则 draft 拒生成）
        .filter(|r| r.category == "资格条件" || r.category == "否决项" || r.category == "技术参数")
        .map(|r| r.id.clone())
        .collect();
    for rid in &qual_ids {
        svc.store_mut()
            .bind_evidence(&eid, rid, "2026-11-30")
            .unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    use bid_domain::review::{Decision, Gate};
    // #180 审核内容绑定：矩阵/大纲批的是响应面与结构面，保持起草前原位；
    // PreExportConfirm 绑定章节正文摘要，挑至全部叶起草完成后记审（见 serve
    // 相位导出前——与人类流程同序：审核定稿内容）
    for gate in [Gate::MatrixApproval, Gate::OutlineApproval] {
        svc.store_mut()
            .record_review(gate, &bid_domain::TrustedPrincipal::human("real-path-lead"), Decision::Approved, vec![], None)
            .unwrap();
    }
    let n_blocking = svc.store().matrix().iter().filter(|r| r.blocking).count();
    assert_eq!(n_blocking, 0, "绑定后 blocking 清零: 剩 {n_blocking}");

    // ---- 选目标章节（确定性：REQ 最多者，平局取 id 序）——人肉 LLM 的起草对象
    let leaves = leaf_ids_of(svc);
    assert!(!leaves.is_empty(), "章节树非空");
    let target = leaves
        .iter()
        .max_by_key(|sid| {
            let n = svc
                .store()
                .sections()
                .iter()
                .find(|n| &n.id == *sid)
                .unwrap()
                .requirement_ids
                .len();
            (n, std::cmp::Reverse((*sid).clone()))
        })
        .unwrap()
        .clone();
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == target)
        .unwrap();
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
    let marker = json!({
        "section_id": target, "title": node.title, "target_chars": node.target_chars,
        "requirements": reqs,
    });
    std::fs::create_dir_all(ws.join(".mang")).unwrap();
    std::fs::write(
        target_marker(ws),
        serde_json::to_string_pretty(&marker).unwrap(),
    )
    .unwrap();

    svc.checkpoint()
        .expect("准备链后显式落盘（子进程要从盘上恢复）");
    (pid, target)
}

// ---------------------------------------------------------------- 主测试

#[test]
fn real_llm_human_relay_path() {
    let (Some(ws_s), Some(phase)) = (
        std::env::var("BID_REAL_PATH_WS")
            .ok()
            .filter(|s| !s.trim().is_empty()),
        std::env::var("BID_REAL_PATH_PHASE").ok(),
    ) else {
        println!(
            "================ SKIP real_llm_path ================\n\
             未设 BID_REAL_PATH_WS / BID_REAL_PATH_PHASE（CI 默认跳过——人肉 LLM\n\
             通路需要中继服务与开发者撰写面）。驱动入口：\n\
             scripts/e2e_bid/run_real_llm_path.sh phase1|phase2\n\
             ===================================================="
        );
        return;
    };
    let ws = PathBuf::from(ws_s.trim());
    assert!(ws.is_dir(), "BID_WS 工作区不存在: {}", ws.display());
    assert!(
        matches!(phase.as_str(), "capture" | "serve"),
        "BID_REAL_PATH_PHASE 须为 capture|serve，实得 {phase}"
    );

    // ---- llm.json 在场性（真实通路的配置面——由驱动脚本写入，指向本地中继）
    let llm_json = ws.join(".mang").join("llm.json");
    assert!(
        llm_json.is_file(),
        "缺 {}（真实通路前置——run_real_llm_path.sh 负责写入）",
        llm_json.display()
    );

    // ---- 准备链（幂等）
    let t0 = Instant::now();
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(files_dir());
    let (_pid, target) = ensure_prepared(&mut svc, &ws);
    let leaves = leaf_ids_of(&svc);
    println!(
        "[real-path] 准备链 {}ms｜目标章节 {target}｜叶 {} 个",
        t0.elapsed().as_millis(),
        leaves.len()
    );

    // ---- 相位分派：draft 调用走真实 stdio 子进程
    let draft_args = json!({
        "project_id": svc.store().project().unwrap().id,
        "section_id": target,
        "instructions": "按 synth-tender 招标文件逐项响应：资格与类似业绩逐条对应 REQ 引用；\
                         实施方案给出可核验的量化承诺（进度/人员/设备），不得空话",
        "generator": "real",
    });
    let mut proc = StdioProc::spawn(&ws);
    let t_call = Instant::now();

    if phase == "capture" {
        // ================= capture：真实传输面 + 错误语义 + 零副作用 ================
        let reply = proc.call("draft_response_section", draft_args);
        assert_eq!(
            reply["ok"],
            json!(false),
            "capture 相位应失败（503）: {reply}"
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
        // 零副作用：章节仍非 DRAFTED / 项目无内容版本 bump 面
        drop(proc);
        let svc2 = BidService::open(&ws).unwrap();
        let node = svc2
            .store()
            .sections()
            .iter()
            .find(|n| n.id == target)
            .unwrap();
        assert_ne!(
            node.status.as_str(),
            "DRAFTED",
            "失败调用不得推进章节状态（生成器报错在 CAS 写入之前返回）"
        );
        println!(
            "[real-path] capture ✅ 真实传输面（ureq → 127.0.0.1 中继 → HTTP 503 错误语义）｜{}ms\n\
             [real-path] prompt 已由中继落盘——开发者撰写正文后跑 serve 相位",
            t_call.elapsed().as_millis()
        );
        return;
    }

    // ================= serve：完整真实链路（开发者正文 → 真实生成器） ================
    let reply = proc.call("draft_response_section", draft_args);
    assert_eq!(reply["ok"], json!(true), "serve 相位 draft 应成功: {reply}");
    let d = reply_data(&reply);
    assert_eq!(d["stats"]["generator"], json!("real"), "生成器标记面: {d}");
    assert_eq!(d["section_status"], json!("DRAFTED"));
    let citations = d["stats"]["citations"].as_u64().unwrap();
    assert!(citations > 0, "引用面（开发者正文带 [REQ-xxxx]）: {d}");
    let uncovered = d["stats"]["uncovered_requirements"].as_array().unwrap();
    assert!(
        uncovered.is_empty(),
        "覆盖度：开发者正文必须回应全部 REQ（缺失 {uncovered:?}）"
    );
    let real_words = d["stats"]["words"].as_u64().unwrap();
    // 字数 ±20%（与 validate 规则 4 对称的验收口径——作者面的硬标准）
    let target_chars: u64 = {
        let m: Value =
            serde_json::from_str(&std::fs::read_to_string(target_marker(&ws)).unwrap()).unwrap();
        m["target_chars"].as_u64().unwrap()
    };
    let dev = (real_words as f64 - target_chars as f64).abs() / target_chars as f64;
    assert!(
        dev <= 0.20,
        "字数偏离 {dev:.1}% 超 ±20%（{real_words} vs {target_chars}）"
    );
    println!(
        "[real-path] serve ✅ 真实生成链路（generator=real｜{real_words} 字｜引用 {citations}｜偏离 {:.1}%）｜{}ms",
        dev * 100.0,
        t_call.elapsed().as_millis()
    );

    // ---- 其余章节 mock 补齐（同一 stdio 子进程——完成文档面）
    let t_mock = Instant::now();
    for sid in &leaves {
        if sid == &target {
            continue;
        }
        let r = proc.call(
            "draft_response_section",
            json!({
                "project_id": svc.store().project().unwrap().id,
                "section_id": sid,
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
        "[real-path] mock 补齐 {} 章｜{}ms",
        leaves.len() - 1,
        t_mock.elapsed().as_millis()
    );

    // ---- 校验（MCP §7——BLOCKER 清零）
    let v = proc.call(
        "validate_bid_document",
        json!({
            "project_id": svc.store().project().unwrap().id, "scope": "all",
        }),
    );
    assert_eq!(reply_data(&v)["ok"], json!(true), "校验未过: {v}");
    assert_eq!(reply_data(&v)["counts"]["blocker"], json!(0));

    // ---- PreExportConfirm 记审（#180：定稿内容过门——全部叶起草（开发者
    //      正文 + mock 补齐）完成后、导出前，绑定的才是将进产物的摘要）
    // #182：宿主签发面（svc 与 stdio 子进程同工作区同钥——测试进程即宿主）
    let r_precx = svc
        .issue_approval_receipt("real-path-lead", "review_gate:pre_export_confirm");
    let g = proc.call(
        "record_review_gate",
        json!({
            "project_id": svc.store().project().unwrap().id, "gate": "pre_export_confirm",
            "decision": "APPROVED", "receipt": r_precx,
        }),
    );
    assert_eq!(g["ok"], json!(true), "pre_export_confirm 记审: {g}");

    // ---- 导出（MCP §8——真实章节进成品 docx）
    let e = proc.call(
        "export_bid_document",
        json!({
            "project_id": svc.store().project().unwrap().id,
            "format": "docx", "output_path": "out/真实通路-投标文件-技术标.docx",
        }),
    );
    let url = reply_data(&e)["artifact"]["url"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        url.ends_with("真实通路-投标文件-技术标.docx"),
        "产物名: {url}"
    );
    let size = reply_data(&e)["artifact"]["size"].as_u64().unwrap();
    assert!(size > 5_000, "产物 >5KB: {size}");

    // ---- 用量台账（真实调用面的审计产物）
    drop(proc);
    let usage_path = ws.join(".mang").join("llm-usage.json");
    assert!(
        usage_path.is_file(),
        "llm-usage.json 应落盘（真实调用记账）"
    );
    let usage: Value =
        serde_json::from_str(&std::fs::read_to_string(&usage_path).unwrap()).unwrap();
    assert!(
        usage["calls"].as_u64().unwrap() >= 1,
        "台账 calls >= 1: {usage}"
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
        "[real-path] serve 全链 ✅ 导出 {}（{} 字节）｜台账 {} 次调用｜目标章节 {target}",
        url, size, usage["calls"]
    );
    println!("[real-path] 成品中第 {target} 章为【开发者真实撰写】（人肉 LLM），其余为 mock 补齐");
}
