//! S-4 · 游标分页硬验收（issue #11 批次 v3 / V3-5）。
//!
//! 验收口径（issue 原文）：12,000 行矩阵 / 10,000 条证据不能一次全返回；
//! 12,000 行矩阵分页取完，总条数正确，**无重复无遗漏**。所有用例走
//! `dispatch_with`（工具面全链路——参数校验 → 游标解析 → 过滤 → 切片 →
//! 信封组装），不绕过壳层直测内部函数。
//!
//! 双轨（issue #11 测试要求）：
//! - **自研轨**：12k 行矩阵取完 / 过滤+分页组合（先过滤后分页）/
//!   跨过滤器·跨工具游标互斥 / 坏游标 fail-closed / limit 边界（默认·上下界·
//!   超 total）/ list_evidence·list_tasks 同口径验收；
//! - **借鉴轨**：
//!   - 确定性（borrow：mdcore 跨进程确定性口径——同快照、同游标序列，两次
//!     独立调用 serde_json 串**逐字节相等**）；
//!   - fail-closed 三态（borrow：mdcore `workbench/tests/persistence_recovery.rs`
//!     的损坏输入响亮拒绝范式——坏编码/坏版本/越界全拒 E_UNPROCESSABLE，
//!     不猜测、不 panic）。
//!
//! 夹具纪律：大 store 一律**循环生成**（12,000 条 REQ 不落文本 fixture）；
//! 锚点 excerpt 逐条互异，保证 `set_requirements` 的 dedupe_hash 不归并。

use base64::Engine as _;
use bid_domain::assignment::TaskStatus;
use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::{Anchor, ParseStatus, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

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

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// 游标 payload 锻造（绕过工具正常签发——专测坏输入面）。
fn forge(payload: Value) -> String {
    base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_string(&payload).unwrap().as_bytes())
}

/// 从真游标里提取过滤器指纹（合法路径签发——用于锻造「指纹正确但 offset 越界」
/// 的坏游标，覆盖 slice 层的越界门）。
fn fp_of(cursor: &str) -> String {
    let bytes = base64::engine::general_purpose::STANDARD.decode(cursor).unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    v["fp"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------- 夹具

/// 四类条款轮转（技术参数 = CATS[1]、评分项 = CATS[2]——过滤用）。
const CATS: [&str; 4] = ["资格条件", "技术参数", "评分项", "商务要求"];
/// 证据七类全轮转（T-1 规格）。
const KINDS: [EvidenceKind; 7] = [
    EvidenceKind::Qualification,
    EvidenceKind::Performance,
    EvidenceKind::Personnel,
    EvidenceKind::Certificate,
    EvidenceKind::ProductDoc,
    EvidenceKind::SolutionModule,
    EvidenceKind::Contract,
];

/// S-4 矩阵夹具：项目 + 主文件 + n 条 REQ（唯一锚——dedupe_hash 不互撞；
/// 类别 4 轮转 → 每类恰 n/4，过滤 oracle 可静态推导）。
fn matrix_store(n: usize) -> (BidService, String) {
    let mut svc = BidService::new();
    let pid = {
        let st = svc.store_mut();
        let (pid, _) = st
            .create_project("S-4 分页验收", "ZFCG-2026-S4", "明标", "s4-matrix-key")
            .unwrap();
        st.upsert_file("A-招标文件.docx", "招标文件", "sha-s4-matrix", 31, 12, 120, 0,
                       ParseStatus::Ok, "").unwrap();
        let reqs: Vec<Requirement> = (0..n)
            .map(|i| Requirement {
                id: format!("REQ-{:04}", i + 1),
                category: CATS[i % CATS.len()].into(),
                clause_no: Some(format!("3.{}", i + 1)),
                anchors: vec![Anchor {
                    file: "A".into(),
                    kind: "para".into(),
                    locator: format!("第三章 > 3.{}", i + 1),
                    excerpt: format!("S-4 合成条款 {i}（唯一锚——dedupe_hash 不互撞）"),
                        block_idx: None,
                        page: None,
                        line: None
                }],
                params: HashMap::new(),
                source_file: "A".into(),
            })
            .collect();
        st.set_requirements(reqs).unwrap();
        pid
    };
    (svc, pid)
}

/// S-4 证据夹具：项目 + n 条证据（七类轮转；有效期三分：远期 Active /
/// 开标后 90 天窗口内 Expiring / 已过 Expired；每 11 条换版一次 → 旧记录
/// SUPERSEDED）。状态终局由 `apply_validity("2026-06-01")` 落位。
fn evidence_store(n: usize) -> (BidService, String) {
    let mut svc = BidService::new();
    let pid = {
        let st = svc.store_mut();
        let (pid, _) = st
            .create_project("S-4 证据分页", "ZFCG-2026-S4-EV", "明标", "s4-evidence-key")
            .unwrap();
        for i in 0..n {
            let valid_until = match i % 3 {
                0 => "2027-06-01", // 远期——不翻转，Active
                1 => "2026-07-01", // 开标日 2026-06-01 后 90 天窗口内——Expiring
                _ => "2025-06-01", // 已过——Expired
            };
            st.register_evidence(EvidenceSpec {
                kind: KINDS[i % KINDS.len()],
                title: format!("S-4 证据 {i:04}"),
                source_file: "EV-证据卷.pdf".into(),
                source_sha256: format!("sha-ev-{i:06}"),
                locator: EngineLocator::DocxParagraph {
                    paragraph_index: i as u32,
                    text_fingerprint: format!("fnv64:{i:016x}"),
                },
                fingerprint: format!("fnv64:{i:016x}"),
                valid_from: Some("2024-01-01".into()),
                valid_until: Some(valid_until.into()),
                subject_name: "投标人甲".into(),
                scope: "全类型".into(),
                client: None,
                amount_wan: None,
                completed_on: None,
                fields: Default::default(), // #147
                excerpt: None,
                excerpt_locator: None,
                verification_tier: None,
            })
            .unwrap();
        }
        pid
    };
    // 换版：同语义位（kind/title/subject/scope 四元组）换指纹重登 → 旧记录
    // SUPERSEDED、新记录 version=2（台账只降不升纪律的换版通道）
    for i in (0..n).step_by(11) {
        svc.store_mut()
            .register_evidence(EvidenceSpec {
                kind: KINDS[i % KINDS.len()],
                title: format!("S-4 证据 {i:04}"),
                source_file: "EV-证据卷.pdf".into(),
                source_sha256: format!("sha-ev-{i:06}"),
                locator: EngineLocator::DocxParagraph {
                    paragraph_index: i as u32,
                    text_fingerprint: format!("fnv64:rev{i:013x}"),
                },
                fingerprint: format!("fnv64:rev{i:013x}"),
                valid_from: Some("2024-01-01".into()),
                valid_until: Some("2027-06-01".into()),
                subject_name: "投标人甲".into(),
                scope: "全类型".into(),
                client: None,
                amount_wan: None,
                completed_on: None,
                fields: Default::default(), // #147
                excerpt: None,
                excerpt_locator: None,
                verification_tier: None,
            })
            .unwrap();
    }
    svc.store_mut().apply_validity("2026-06-01").unwrap();
    (svc, pid)
}

/// S-4 任务夹具：项目 + 主文件 + n 条 REQ + 全量派活（三执行人轮派 REQ 目标，
/// 截止日隔人交替）+ 前 n/3 推进 IN_PROGRESS、其中前 n/9 推进 DONE。
fn task_store(n: usize) -> (BidService, String) {
    let mut svc = BidService::new();
    let pid = {
        let st = svc.store_mut();
        let (pid, _) = st
            .create_project("S-4 任务分页", "ZFCG-2026-S4-TK", "明标", "s4-task-key")
            .unwrap();
        st.upsert_file("A-招标文件.docx", "招标文件", "sha-s4-task", 31, 12, 120, 0,
                       ParseStatus::Ok, "").unwrap();
        let reqs: Vec<Requirement> = (0..n)
            .map(|i| Requirement {
                id: format!("REQ-{:04}", i + 1),
                category: CATS[i % CATS.len()].into(),
                clause_no: Some(format!("3.{}", i + 1)),
                anchors: vec![Anchor {
                    file: "A".into(),
                    kind: "para".into(),
                    locator: format!("第三章 > 3.{}", i + 1),
                    excerpt: format!("S-4 任务条款 {i}（唯一锚——dedupe_hash 不互撞）"),
                        block_idx: None,
                        page: None,
                        line: None
                }],
                params: HashMap::new(),
                source_file: "A".into(),
            })
            .collect();
        st.set_requirements(reqs).unwrap();
        pid
    };
    {
        let st = svc.store_mut();
        let owners = ["writer:zhang", "writer:li", "reviewer:wang"];
        for (oi, owner) in owners.iter().enumerate() {
            let targets: Vec<String> = (oi..n)
                .step_by(owners.len())
                .map(|i| format!("REQ-{:04}", i + 1))
                .collect();
            st.batch_assign(&targets, owner, if oi % 2 == 0 { Some("2026-11-30") } else { None })
                .unwrap();
        }
        // 状态推进（线性链：先全员 IN_PROGRESS，再头部 DONE——越级会被拒）
        let head: Vec<String> = (0..n / 3).map(|i| format!("REQ-{:04}", i + 1)).collect();
        for t in &head {
            st.advance_task(t, TaskStatus::InProgress).unwrap();
        }
        for t in &head[..n / 9] {
            st.advance_task(t, TaskStatus::Done).unwrap();
        }
    }
    (svc, pid)
}

// ---------------------------------------------------------------- 1. 12k 硬验收

/// S-4 规格硬验收（issue 原文）：「12,000 行矩阵分页取完，总条数正确，
/// 无重复无遗漏」。limit=100 循环 120 页直到 next_cursor=null，收集全部
/// cmp_id：总数=12000、无重复（HashSet 插入恒新）、无遗漏（与全量 id
/// 序列精确相等——同格式构造，序也须一致）。
#[test]
fn matrix_12k_rows_paged_exactly_once() {
    const N: usize = 12_000;
    const LIMIT: usize = 100;
    let (mut svc, pid) = matrix_store(N);

    let mut got: Vec<String> = Vec::with_capacity(N);
    let mut cursor_opt: Option<String> = None;
    let mut pages = 0usize;
    loop {
        pages += 1;
        assert!(pages <= N / LIMIT + 2, "分页循环异常（页数超界——防死循环护栏）");
        let mut args = json!({ "project_id": pid, "limit": LIMIT });
        if let Some(c) = &cursor_opt {
            args["cursor"] = json!(c);
        }
        let r = ok_result(call(&mut svc, "get_compliance_matrix", args));
        // 信封：total 恒为全量 12000（无过滤）；limit 回显生效值
        assert_eq!(r["total"], json!(N), "每页 total 恒为 12000");
        assert_eq!(r["limit"], json!(LIMIT), "limit 回显生效页宽");
        if let Some(prev) = &cursor_opt {
            assert_eq!(
                r["cursor"].as_str().unwrap(),
                prev,
                "信封自洽：本次生效游标 = 上一页回传的 next_cursor"
            );
        }
        for row in r["rows"].as_array().unwrap() {
            got.push(row["cmp_id"].as_str().unwrap().to_string());
        }
        match r["next_cursor"].as_str() {
            None => break,
            Some(nc) => cursor_opt = Some(nc.to_string()),
        }
    }
    assert_eq!(pages, N / LIMIT, "12000/100 = 120 页整");
    // 无重复
    let uniq: HashSet<&String> = got.iter().collect();
    assert_eq!(uniq.len(), N, "无重复（集合基数 = 12000）");
    // 无遗漏 + 行序稳定（与全量 cmp_id 序列精确相等——同格式构造）
    let expect: Vec<String> = (1..=N).map(|i| format!("CMP-{i:04}")).collect();
    assert_eq!(got, expect, "无遗漏且行序 = 条款位置序（CMP-0001..N）");
}

// ---------------------------------------------------------------- 2. 过滤 + 分页 + 游标互斥

/// 过滤器 + 分页组合：category 过滤后 total=过滤数、分页取完无重无漏、
/// 行序稳定；**跨过滤器游标互斥**：A 过滤器的 cursor 配 B 过滤器（或无过滤）
/// → E_UNPROCESSABLE（fail-closed，不许静默给错数据）。
#[test]
fn matrix_filter_paging_and_cross_filter_cursor_rejected() {
    let (mut svc, pid) = matrix_store(2_000);
    // tech_param = CATS[1] → i%4==1 → 500 行
    let mut got: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = json!({
            "project_id": pid,
            "filters": { "category": "tech_param" },
            "limit": 60,
        });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let r = ok_result(call(&mut svc, "get_compliance_matrix", args));
        assert_eq!(r["total"], json!(500), "total = 过滤后总数（先过滤后分页）");
        for row in r["rows"].as_array().unwrap() {
            assert_eq!(row["requirement"]["category_key"], "tech_param", "过滤后行全部命中");
            got.push(row["cmp_id"].as_str().unwrap().to_string());
        }
        match r["next_cursor"].as_str() {
            None => break,
            Some(nc) => cursor = Some(nc.into()),
        }
    }
    // 与独立 oracle 精确相等（序 + 集）
    let expect: Vec<String> = (0..2_000)
        .filter(|i| i % 4 == 1)
        .map(|i| format!("CMP-{:04}", i + 1))
        .collect();
    assert_eq!(got.len(), 500);
    assert_eq!(got, expect, "过滤分页无重无漏、行序稳定");
    // 中文类别名同集（双形态口径——与既有矩阵过滤行为一致）
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "filters": { "category": "技术参数" }, "limit": 1000,
    })));
    assert_eq!(r["total"], json!(500), "中文类别名过滤同集");

    // ---- 跨过滤器游标互斥（fail-closed）----
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "filters": { "category": "tech_param" }, "limit": 10,
    })));
    let nc = r["next_cursor"].as_str().unwrap().to_string();
    for other in [
        json!({ "category": "qualification" }), // B 过滤器
        json!({}),                              // 无过滤
        json!({ "status": "NOT_RESPONDED" }),   // 另一过滤维
    ] {
        let reply = call(&mut svc, "get_compliance_matrix", json!({
            "project_id": pid, "filters": other, "limit": 10, "cursor": nc,
        }));
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "跨过滤器游标必须拒（filters={other}）");
    }
    // 跨工具互斥：矩阵游标喂 list_evidence（fp 含工具名——不同列表的 offset 不可比）
    let reply = call(&mut svc, "list_evidence", json!({
        "project_id": pid, "cursor": nc, "limit": 10,
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "跨工具游标必须拒");
}

// ---------------------------------------------------------------- 3. 坏游标 fail-closed

/// 坏游标 fail-closed 三态（borrow：mdcore `workbench/tests/persistence_recovery.rs`
/// 的损坏输入响亮拒绝范式）：坏编码 / 坏版本 / 越界——全拒 E_UNPROCESSABLE，
/// 绝不 panic、绝不静默给错页；完好游标照常通过（坏输入拒绝不是无差别拒绝）。
/// 附真实缩水场景：同 project_id 的同构小 store 复用大 store 的游标 →
/// offset 越界拒（不静默给空页——空页会被误读为「取完了」）。
#[test]
fn matrix_bad_cursors_fail_closed_no_panic() {
    let (mut svc, pid) = matrix_store(30);
    let first = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 10,
    })));
    let good = first["next_cursor"].as_str().unwrap().to_string();
    let fp = fp_of(&good);

    let cases: Vec<(&str, String)> = vec![
        ("坏编码：非法 base64 字母表", "!!!@@@###".into()),
        ("坏编码：合法 base64 包非 JSON", {
            let raw: &[u8] = &[0x00, 0xff, 0xfe, b'g', b'a', b'r', b'b', b'a', b'g', b'e'];
            base64::engine::general_purpose::STANDARD.encode(raw)
        }),
        ("坏编码：截断丢 padding", good[..good.len() - 6].to_string()),
        ("坏编码：空串", String::new()),
        ("坏版本：v=99（未来版本）", forge(json!({ "v": 99, "offset": 5, "fp": fp }))),
        ("坏版本：v 为字符串", forge(json!({ "v": "1", "offset": 5, "fp": fp }))),
        ("负 offset（u64 反序列化拒）", forge(json!({ "v": 1, "offset": -5, "fp": fp }))),
        ("offset 溢出（> 过滤后总数）", forge(json!({ "v": 1, "offset": 999_999, "fp": fp }))),
        ("fp 缺失", forge(json!({ "v": 1, "offset": 5 }))),
        ("fp 错配（跨过滤器伪造）", forge(json!({ "v": 1, "offset": 5, "fp": "deadbeefdeadbeef" }))),
    ];
    for (why, cursor) in cases {
        let reply = call(&mut svc, "get_compliance_matrix", json!({
            "project_id": pid, "limit": 10, "cursor": cursor,
        }));
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "{why} 必须 fail-closed 拒（不 panic）");
    }
    // 对照面：完好游标照常翻页（拒绝面不是无差别拒绝）
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 10, "cursor": good,
    })));
    assert_eq!(r["rows"].as_array().unwrap().len(), 10, "完好游标正常翻页");

    // 数据缩水面：同构小 store（同 project_id/同过滤器 → fp 相同）行数缩水，
    // 大 store 的 offset=10 游标越界 → 拒（不静默给空页）
    let (mut small, small_pid) = matrix_store(5);
    assert_eq!(small_pid, pid, "同构夹具：确定性重建（PRJ id 序列一致）");
    let reply = call(&mut small, "get_compliance_matrix", json!({
        "project_id": small_pid, "limit": 10, "cursor": good,
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "底层列表缩水 → offset 越界拒");
}

// ---------------------------------------------------------------- 3.5 C-14 增长位移守卫

/// **C-14 增长位移守卫（#399）**：两页之间行集**增长**（重跑 extract 整集
/// 替换追加 REQ——#219 工作流「翻页标注 → 重跑 extract → 再看矩阵」的现实
/// 场景）时 offset ≤ total 仍成立，旧防线（越界拒）不触发——游标 v2 的
/// total 绑定必须拦下：E_UNPROCESSABLE（fail-closed，不静默给移位窗口）；
/// 从首页重取后照常翻页（拒绝面不是无差别拒绝）。
#[test]
fn matrix_growth_between_pages_rejected_not_silently_shifted() {
    let (mut svc, pid) = matrix_store(100);
    // 第 1 页：total=100，next_cursor 锚定 (offset=50, total=100)
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 50,
    })));
    assert_eq!(r["total"], json!(100));
    let nc = r["next_cursor"].as_str().unwrap().to_string();

    // 两页之间行集增长：重跑合并（set_requirements 按 dedupe_hash 幂等合并——
    // 同哈希覆盖原位、新件追加）——前 100 条与首轮完全同构（同 locator/excerpt
    // → 同哈希 → 覆盖不增行），追加 5 条新哈希 → total 105。fp 只绑
    // 工具/项目/过滤器，与行数无关，指纹门不拦，只能靠总数绑定拦。
    {
        let st = svc.store_mut();
        let reqs: Vec<Requirement> = (0..105)
            .map(|i| Requirement {
                id: format!("REQ-{:04}", i + 1),
                category: CATS[i % CATS.len()].into(),
                clause_no: Some(format!("3.{}", i + 1)),
                anchors: vec![Anchor {
                    file: "A".into(),
                    kind: "para".into(),
                    locator: format!("第三章 > 3.{}", i + 1),
                    excerpt: format!("S-4 合成条款 {i}（唯一锚——dedupe_hash 不互撞）"),
                    block_idx: None,
                    page: None,
                    line: None,
                }],
                params: HashMap::new(),
                source_file: "A".into(),
            })
            .collect();
        let (total, added) = st.set_requirements(reqs).unwrap();
        assert_eq!((total, added), (105, 5), "增长夹具自检：100 覆盖 + 5 追加");
    }

    // 旧游标翻页 → E_UNPROCESSABLE（增长位移 fail-closed，不静默翻错页）
    let reply = call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 50, "cursor": nc,
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "行集增长位移必须拒");

    // 从首页重取：新游标锚定 total=105，第二页照常翻（守卫拒绝的是漂移游标，
    // 不是翻页本身）
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 50,
    })));
    assert_eq!(r["total"], json!(105), "增长后 total=105");
    let nc2 = r["next_cursor"].as_str().unwrap().to_string();
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 50, "cursor": nc2,
    })));
    assert_eq!(r["rows"].as_array().unwrap().len(), 50, "新首页游标正常翻页");
    // 第二页首行 = 全量第 51 条（无跳行——位移守卫下的行序连续性）
    assert_eq!(
        r["rows"][0]["cmp_id"].as_str().unwrap(),
        "CMP-0051",
        "第二页从第 51 行起（无位移）"
    );
}

// ---------------------------------------------------------------- 4. limit 边界

/// limit 边界：0 / 1001 / 非数字 → 显式拒（不静默 clamp）；缺省 → 100；
/// limit > total → 单页取完 next_cursor=null；边界值 1 与 1000 本身合法。
#[test]
fn limit_boundaries_default_and_rejects() {
    let (mut svc, pid) = matrix_store(250);
    // 越界 / 非整数 / 类型错 → E_UNPROCESSABLE（显式拒，不 clamp）
    for bad in [
        json!(0),
        json!(1001),
        json!(u64::MAX),
        json!("50"),
        json!(1.5),
        json!(-3),
        json!([100]),
    ] {
        let reply = call(&mut svc, "get_compliance_matrix", json!({
            "project_id": pid, "limit": bad,
        }));
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "limit={bad} 必须显式拒");
    }
    // 不给 limit → 默认 100（limit 回显 + 页宽 + counts.page_size 同口径）
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({ "project_id": pid })));
    assert_eq!(r["limit"], json!(100), "缺省 limit=100");
    assert_eq!(r["rows"].as_array().unwrap().len(), 100);
    assert_eq!(r["counts"]["page_size"], json!(100));
    assert_eq!(r["counts"]["returned"], json!(100));
    // limit=1000（上界）> total=250 → 单页取完 next_cursor=null
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 1000,
    })));
    assert_eq!(r["total"], json!(250));
    assert_eq!(r["rows"].as_array().unwrap().len(), 250);
    assert!(r["next_cursor"].is_null(), "limit>total 单页取完 next_cursor=null");
    // 下界 limit=1 合法（120 页的极端页宽不在本测——只验边界放行 + 翻页正确性）
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 1,
    })));
    assert_eq!(r["rows"].as_array().unwrap().len(), 1);
    assert_eq!(r["rows"][0]["cmp_id"], "CMP-0001");
    assert!(r["next_cursor"].is_string(), "还有余量 → next_cursor 在位");

    // page 兼容路径：page=3 & limit=50 → offset=100（第 101 行起）；
    // page=0 → 拒（旧 .max(1) clamp 习惯已改显式拒）
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page": 3, "limit": 50,
    })));
    assert_eq!(r["rows"][0]["cmp_id"], "CMP-0101", "page 映射 offset=(page-1)*limit");
    let reply = call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page": 0,
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "page=0 显式拒（不 clamp）");
}

// ---------------------------------------------------------------- 5. 信封自洽 / cursor 优先 / 空过滤集

/// 信封自洽：cursor 回放 = 同一页（确定性）；cursor 与 page 并存 → cursor
/// 优先；空过滤结果集（RESPONDED 在裸矩阵上恒空）→ total=0、rows=[]、
/// next_cursor=null、cursor 字段仍在位（可回放空页）。
#[test]
fn envelope_cursor_priority_and_empty_filter_result() {
    let (mut svc, pid) = matrix_store(60);
    // page 路径取第 3 页（limit 25 → offset 50）
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "page": 3, "limit": 25,
    })));
    assert_eq!(r["rows"][0]["cmp_id"], "CMP-0051");
    // 信封自洽：用返回的「本次生效游标」重放 → 同一页
    let c = r["cursor"].as_str().unwrap().to_string();
    let r2 = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "cursor": c, "limit": 25,
    })));
    assert_eq!(r["rows"], r2["rows"], "cursor 回放 = 同一页（确定性）");
    // cursor 优先于 page（并存时 page 被忽略）
    let r3 = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "cursor": c, "page": 9, "limit": 25,
    })));
    assert_eq!(r2["rows"], r3["rows"], "cursor 与 page 并存 → cursor 优先");
    // 空过滤结果集
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "filters": { "status": "RESPONDED" },
    })));
    assert_eq!(r["total"], json!(0), "裸矩阵无已响应行");
    assert!(r["rows"].as_array().unwrap().is_empty());
    assert!(r["next_cursor"].is_null(), "空集取完 next_cursor=null");
    assert!(r["cursor"].is_string(), "空集 cursor 字段仍在位（可回放）");
    // 状态过滤正面：NOT_RESPONDED 全命中（无绑定/章节的裸矩阵）
    let r = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "filters": { "status": "NOT_RESPONDED" }, "limit": 1000,
    })));
    assert_eq!(r["total"], json!(60), "status 过滤全命中");
}

// ---------------------------------------------------------------- 6. list_evidence 验收

/// list_evidence：kind/status 过滤 + 分页取完无重无漏 + 行序（id 字典序）
/// + 游标互斥 + 非法过滤值拒 + 项目路由 E_NOTFOUND。
#[test]
fn evidence_list_filter_paging_and_cursor_exclusivity() {
    let (mut svc, pid) = evidence_store(700);
    // 独立 oracle：直读台账镜像过滤（不经过工具实现）
    let (expected_perf, expected_expired, expected_all): (HashSet<String>, HashSet<String>, HashSet<String>) = {
        let st = svc.store();
        let perf = st.evidences().iter()
            .filter(|e| e.kind == EvidenceKind::Performance)
            .map(|e| e.id.clone()).collect();
        let expired = st.evidences().iter()
            .filter(|e| e.status == bid_domain::evidence::EvidenceStatus::Expired)
            .map(|e| e.id.clone()).collect();
        let all = st.evidences().iter().map(|e| e.id.clone()).collect();
        (perf, expired, all)
    };
    assert!(!expected_perf.is_empty() && !expected_expired.is_empty());
    // 换版双记录在账：总基数 = 原始 n + 每 11 条一次的重登
    assert_eq!(expected_all.len(), 700 + (0..700).step_by(11).count());

    // ---- kind=performance 分页取完（limit 37——非整除页宽）
    let mut got: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = json!({
            "project_id": pid, "filters": { "kind": "performance" }, "limit": 37,
        });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let r = ok_result(call(&mut svc, "list_evidence", args));
        assert_eq!(r["total"], json!(expected_perf.len()), "total = 过滤后总数");
        for it in r["items"].as_array().unwrap() {
            assert_eq!(it["kind"], "performance", "机器形态 kind 与过滤入参同形");
            assert!(it["status"].is_string() && it["subject_name"].is_string());
            got.push(it["id"].as_str().unwrap().to_string());
        }
        match r["next_cursor"].as_str() {
            None => break,
            Some(nc) => cursor = Some(nc.into()),
        }
    }
    let uniq: HashSet<&String> = got.iter().collect();
    assert_eq!(uniq.len(), got.len(), "无重复");
    assert_eq!(
        got.iter().collect::<HashSet<_>>(),
        expected_perf.iter().collect::<HashSet<_>>(),
        "无遗漏（与 oracle 集合精确相等）"
    );
    assert!(
        got.windows(2).all(|w| w[0] < w[1]),
        "行序 = id 字典序（确定全序——分页稳定前提）"
    );
    // 中文类别名同集（GT 夹具口径——双形态与矩阵 category 一致）
    let r = ok_result(call(&mut svc, "list_evidence", json!({
        "project_id": pid, "filters": { "kind": "业绩" }, "limit": 1000,
    })));
    assert_eq!(r["total"], json!(expected_perf.len()), "中文 kind 过滤同集");

    // ---- status=EXPIRED 分页取完（limit 100）
    let mut got2: HashSet<String> = HashSet::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = json!({
            "project_id": pid, "filters": { "status": "EXPIRED" }, "limit": 100,
        });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let r = ok_result(call(&mut svc, "list_evidence", args));
        assert_eq!(r["total"], json!(expected_expired.len()));
        for it in r["items"].as_array().unwrap() {
            assert_eq!(it["status"], "EXPIRED");
            got2.insert(it["id"].as_str().unwrap().to_string());
        }
        match r["next_cursor"].as_str() {
            None => break,
            Some(nc) => cursor = Some(nc.into()),
        }
    }
    assert_eq!(got2, expected_expired, "status 过滤分页无重无漏");

    // ---- 游标互斥：performance 游标 × 其他过滤器 → 拒
    let r = ok_result(call(&mut svc, "list_evidence", json!({
        "project_id": pid, "filters": { "kind": "performance" }, "limit": 10,
    })));
    let nc = r["next_cursor"].as_str().unwrap().to_string();
    for other in [
        json!({ "kind": "certificate" }),
        json!({ "status": "ACTIVE" }),
        json!({ "kind": "performance", "status": "ACTIVE" }),
        json!({}),
    ] {
        let reply = call(&mut svc, "list_evidence", json!({
            "project_id": pid, "filters": other, "limit": 10, "cursor": nc,
        }));
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "跨过滤器游标必须拒（filters={other}）");
    }

    // ---- 非法过滤值 / 非法入参类型 → 拒
    for bad in [json!({ "kind": "nope" }), json!({ "status": "ok" })] {
        let reply = call(&mut svc, "list_evidence", json!({
            "project_id": pid, "filters": bad, "limit": 10,
        }));
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "非法过滤值 {bad} 拒");
    }
    let reply = call(&mut svc, "list_evidence", json!({
        "project_id": pid, "filters": "performance",
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "filters 非对象拒（fail-closed 收紧面）");

    // ---- 项目路由 E_NOTFOUND
    let reply = call(&mut svc, "list_evidence", json!({ "project_id": "PRJ-9999" }));
    assert_eq!(err_code(&reply), "E_NOTFOUND");

    // ---- 无过滤全量分页取完（换版记录也在台账——总数含 SUPERSEDED）
    let mut got_all: HashSet<String> = HashSet::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = json!({ "project_id": pid, "limit": 90 });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let r = ok_result(call(&mut svc, "list_evidence", args));
        assert_eq!(r["total"], json!(expected_all.len()));
        for it in r["items"].as_array().unwrap() {
            got_all.insert(it["id"].as_str().unwrap().to_string());
        }
        match r["next_cursor"].as_str() {
            None => break,
            Some(nc) => cursor = Some(nc.into()),
        }
    }
    assert_eq!(got_all, expected_all, "全量分页无重无漏（含换版双记录）");
}

// ---------------------------------------------------------------- 7. list_tasks 验收

/// list_tasks：owner/status 双过滤 + 分页取完无重无漏 + 行序（target_id
/// 字典序）+ 游标互斥 + 非法值拒 + 项目路由 E_NOTFOUND。
#[test]
fn tasks_list_filter_paging_and_cursor_exclusivity() {
    let (mut svc, pid) = task_store(240);
    // 独立 oracle：直读 domain 双过滤（list_tasks 的 BTree 语义确定序）
    let (expected_zhang, expected_zhang_prog, expected_all): (HashSet<String>, HashSet<String>, HashSet<String>) = {
        let st = svc.store();
        let zhang = st.list_tasks(Some("writer:zhang"), None).iter()
            .map(|a| a.target_id.clone()).collect();
        let zp = st.list_tasks(Some("writer:zhang"), Some(TaskStatus::InProgress)).iter()
            .map(|a| a.target_id.clone()).collect();
        let all = st.list_tasks(None, None).iter().map(|a| a.target_id.clone()).collect();
        (zhang, zp, all)
    };
    assert!(!expected_zhang.is_empty() && !expected_zhang_prog.is_empty() && expected_all.len() == 240);

    // ---- owner 过滤分页取完（limit 25）
    let mut got: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = json!({
            "project_id": pid, "filters": { "owner": "writer:zhang" }, "limit": 25,
        });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let r = ok_result(call(&mut svc, "list_tasks", args));
        assert_eq!(r["total"], json!(expected_zhang.len()), "total = 过滤后总数");
        for it in r["items"].as_array().unwrap() {
            assert_eq!(it["owner"], "writer:zhang");
            assert!(it["deadline"].is_string() || it["deadline"].is_null());
            got.push(it["target_id"].as_str().unwrap().to_string());
        }
        match r["next_cursor"].as_str() {
            None => break,
            Some(nc) => cursor = Some(nc.into()),
        }
    }
    let uniq: HashSet<&String> = got.iter().collect();
    assert_eq!(uniq.len(), got.len(), "无重复");
    assert_eq!(
        got.iter().collect::<HashSet<_>>(),
        expected_zhang.iter().collect::<HashSet<_>>(),
        "无遗漏（与 oracle 集合精确相等）"
    );
    assert!(
        got.windows(2).all(|w| w[0] < w[1]),
        "行序 = target_id 字典序（domain BTree 语义）"
    );

    // ---- owner × status 组合过滤（单页小集直接取完）
    let r = ok_result(call(&mut svc, "list_tasks", json!({
        "project_id": pid, "filters": { "owner": "writer:zhang", "status": "IN_PROGRESS" },
        "limit": 1000,
    })));
    assert_eq!(r["total"], json!(expected_zhang_prog.len()));
    assert!(r["next_cursor"].is_null(), "limit>total 单页取完");
    let ids: HashSet<String> = r["items"].as_array().unwrap()
        .iter().map(|it| it["target_id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids, expected_zhang_prog, "组合过滤精确命中");

    // ---- 游标互斥
    let r = ok_result(call(&mut svc, "list_tasks", json!({
        "project_id": pid, "filters": { "owner": "writer:zhang" }, "limit": 10,
    })));
    let nc = r["next_cursor"].as_str().unwrap().to_string();
    for other in [
        json!({ "owner": "writer:li" }),
        json!({ "owner": "writer:zhang", "status": "IN_PROGRESS" }),
        json!({ "status": "DONE" }),
        json!({}),
    ] {
        let reply = call(&mut svc, "list_tasks", json!({
            "project_id": pid, "filters": other, "limit": 10, "cursor": nc,
        }));
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "跨过滤器游标必须拒（filters={other}）");
    }

    // ---- 非法值拒 + 项目路由
    let reply = call(&mut svc, "list_tasks", json!({
        "project_id": pid, "filters": { "status": "DONE!" },
    }));
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "非法 status 拒");
    let reply = call(&mut svc, "list_tasks", json!({ "project_id": "PRJ-9999" }));
    assert_eq!(err_code(&reply), "E_NOTFOUND");

    // ---- 无过滤全量分页取完
    let mut got_all: HashSet<String> = HashSet::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = json!({ "project_id": pid, "limit": 100 });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let r = ok_result(call(&mut svc, "list_tasks", args));
        assert_eq!(r["total"], json!(240));
        for it in r["items"].as_array().unwrap() {
            got_all.insert(it["target_id"].as_str().unwrap().to_string());
        }
        match r["next_cursor"].as_str() {
            None => break,
            Some(nc) => cursor = Some(nc.into()),
        }
    }
    assert_eq!(got_all, expected_all, "全量分页无重无漏");
}

// ---------------------------------------------------------------- 8. 确定性（借鉴轨）

/// 确定性（borrow：mdcore 跨进程确定性口径）：同一 store 快照、同一游标
/// 序列，两次独立调用结果**逐字节相等**（serde_json 串比较）。
///
/// 快照等价性说明：夹具全操作确定性（id 序列 + dedupe 幂等合并均无时间/
/// 随机源），两次重建 = 同一快照（domain 的 snapshot roundtrip 全等测试已
/// 证 restore 保真，此处取其推论）。`request_id` 是进程内单调的包装字段
/// （state.rs 口径），剥离后比较——数据面不含任何非确定源。
#[test]
fn deterministic_pages_byte_equal_across_rebuild() {
    fn walk(svc: &mut BidService, pid: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut args = json!({
                "project_id": pid,
                "filters": { "category": "scoring" }, // 评分项 → 125 行
                "limit": 33,                           // 非整除页宽 + 非满尾页
            });
            if let Some(c) = &cursor {
                args["cursor"] = json!(c);
            }
            let r = ok_result(call(svc, "get_compliance_matrix", args));
            let mut clean = r.clone();
            clean.as_object_mut().unwrap().remove("request_id");
            out.push(serde_json::to_string(&clean).unwrap());
            match r["next_cursor"].as_str() {
                None => break,
                Some(c) => cursor = Some(c.into()),
            }
        }
        out
    }
    let (mut a, pid_a) = matrix_store(500);
    let (mut b, pid_b) = matrix_store(500);
    assert_eq!(pid_a, pid_b, "确定性重建：同构夹具同 project_id");
    let pages_a = walk(&mut a, &pid_a);
    let pages_b = walk(&mut b, &pid_b);
    assert!(pages_a.len() >= 4, "过滤后 125 行 / 33 页宽 → 至少 4 页（含非满尾页）");
    assert_eq!(pages_a, pages_b, "同快照同游标序列 → 逐字节相等（跨进程可重放）");
    // 同一服务上重放同序列也逐字节相等（游标确定性 + 行序稳定）
    assert_eq!(pages_a, walk(&mut a, &pid_a), "同服务重放同序列 → 逐字节相等");
}
