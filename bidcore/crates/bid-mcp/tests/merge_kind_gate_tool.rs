//! #250 工具面回归：合并门禁按 kind——真实文件名与夹具文件名同路径。
//!
//! issue #250 实测根因：`has_extraction_face` 按前缀判定 → 真实用户文件
//! （无 A- 前缀）被 filter → faces 空 → 67 条 Agent 提交与 76 个区间标记
//! 永不消费（技术参数 1/商务要求 0——全部核心技术要求与商务条款丢失）。
//!
//! 本测试用【同一物理文件】（corpus-A 主文件 docx）以两种文件名导入：
//! - 夹具名 `A-招标文件.docx`（有前缀）；
//! - 真实名 `光纤租赁费招标文件-文本修复版v5w.docx`（无前缀——issue #250
//!   的真实用户文件名）；
//! 两者 kind 均登记「招标文件」，提交同一组 67 条 Agent 关键点（issue
//! 给定分布：资格 9 / 否决 13 / 评分 9 / 技术 11 / 商务 13 / 格式 9 /
//! 其他 3），断言：
//! 1. 走完全相同的合并路径——`merge` 统计字段两组逐项一致（验收 1）；
//! 2. 67 条全部被消费，技术参数 ≥11、商务要求 ≥13（验收 2——issue
//!    给定基线：修复前技术 1、商务 0）；
//! 3. 关键参数可检索且数值单位完整（验收 3）；
//! 4. 来源字段对称：Agent 项 `marked_by=agent`、正则保留项
//!    `marked_by=regex`（验收 4）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files");
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn corpus_ready() -> bool {
    let main = fixtures().join("A-招标文件.docx");
    !bid_parse::env_preflight::skip_if_fixture_missing("merge_kind_gate_tool", &[&main])
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
        "bid-merge-gate-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// issue #250 给定的 67 条 Agent 提交（分类分布逐字对齐：
/// 资格 9 / 否决 13 / 评分 9 / 技术 11 / 商务 13 / 格式 9 / 其他 3；
/// statement 素材取自 issue 丢失清单——完好率/360分钟/295条30M/s/
/// 15日历天/保证金/预算等）。
fn agent_submissions() -> Vec<(Value, String)> {
    let tech = [
        "295 条 30M/s 上行光纤链路",
        "光缆链路完好率 ≥95%/月",
        "单条链路平均历时 ≤360 分钟",
        "签订合同之日起 15 日历天内调测完毕",
        "上行带宽可平滑升级至 50M/s",
        "汇聚端采用双电源冗余设备",
        "单模光纤平均接头损耗 ≤0.2dB/个",
        "★服务地点：黑龙江省大兴安岭地区松岭区",
        "★验收要求（实质性条款）",
        "服务期限 365 天",
        "上行带宽 ≥30M/s",
    ];
    let comm = [
        "投标保证金 17,000.00 元（A级 8,500）",
        "采购代理服务费 20,000 元",
        "预算/最高限价 1,195,800.00 元",
        "履约保证金：不缴纳",
        "投标有效期 90 日",
        "付款方式：按季度支付",
        "交付地点：黑龙江省大兴安岭地区松岭区",
        "响应时间：7×24 小时",
        "故障恢复时限：4 小时内",
        "服务质量保证金：无",
        "合同定价方式：固定总价",
        "验收周期：每季度验收一次",
        "不接受联合体投标",
    ];
    let qual = [
        "投标人须具备有效营业执照",
        "投标人须具备基础电信业务经营许可证",
        "投标人须具备安全生产许可证",
        "项目负责人须具备通信专业中级及以上职称",
        "投标人近三年无重大违法记录",
        "投标人未处于被责令停业状态",
        "投标人须具备一般纳税人资格",
        "近三年类似项目业绩不少于 2 项",
        "投标人须提供财务审计报告",
    ];
    let veto = [
        "未按要求密封的投标文件将被否决",
        "逾期送达的投标文件将被拒收",
        "未按规定格式编制的投标文件将被否决",
        "投标报价超过最高限价的作否决处理",
        "未提供有效授权委托书的其投标无效",
        "串通投标的取消中标资格",
        "弄虚作假的取消中标资格并没收保证金",
        "不满足实质性条款的作否决处理",
        "未按要求缴纳投标保证金的不予受理",
        "投标文件字迹模糊无法辨认的作否决处理",
        "联合体投标的按否决处理",
        "报价低于成本的其投标将被否决",
        "未按要求签章的投标文件将被否决",
    ];
    let score = [
        "技术方案完整性（10 分）",
        "实施方案可行性（10 分）",
        "人员配备合理性（10 分）",
        "业绩与信誉（15 分）",
        "售后服务方案（10 分）",
        "应急保障措施（10 分）",
        "设备技术参数（10 分）",
        "施工组织设计（10 分）",
        "报价（65 分）",
    ];
    let fmt = [
        "投标函格式要求",
        "法定代表人身份证明格式",
        "授权委托书格式",
        "投标保证金交纳凭证格式",
        "商务条款偏离表格式",
        "技术条款偏离表格式",
        "资格审查资料格式",
        "业绩一览表格式",
        "售后服务承诺格式",
    ];
    let other = [
        "项目概况说明",
        "评标办法：综合评分法",
        "开标时间与地点",
    ];
    let mut out: Vec<(Value, String)> = Vec::new();
    let mut push = |arr: &[&str], cat: &str| {
        for s in arr {
            out.push((json!(s), cat.to_string()));
        }
    };
    push(&qual, "资格条件");
    push(&veto, "否决项");
    push(&score, "评分项");
    push(&tech, "技术参数");
    push(&comm, "商务要求");
    push(&fmt, "格式要求");
    push(&other, "其他");
    assert_eq!(out.len(), 67, "issue #250 给定分布 67 条");
    out
}

/// 单组全链：导入（指定文件名）→ 可选 67 条提交 → 抽取。
/// 返回 (extract 响应, REQ 库快照)。`with_marks=false` 为无标记组
/// （GT 红线 no-op 面——正则产物全量保留）。
fn run_pipeline(tag: &str, filename: &str, with_marks: bool) -> (Value, Vec<bid_domain::Requirement>) {
    let ws = tmp_ws(tag);
    let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(fixtures());
    ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "门禁测试", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": format!("merge-gate-{tag}")
    })));
    let main = fixtures().join("A-招标文件.docx");
    let out = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{
            "upload_ref": main.to_string_lossy(),
            "filename": filename,
            "kind": "招标文件"
        }]
    })));
    let fid = out["file_ids"][0].as_str().expect("file_id").to_string();
    let total = out["per_file"][0]["chunks"].as_u64().expect("chunks") as usize;
    assert!(total >= 8, "corpus-A 块面: {total}");

    // 标记：前 1/4 noise（公告说明区），其余 requirement（正文区）——
    // 67 条关键点全部挂 requirement 区间（validate_submitted 域）
    if with_marks {
        let k = total / 4;
        let subs = agent_submissions();
        let req_span = (total - k - 1).max(1);
        let reqs_json: Vec<Value> = subs
            .iter()
            .enumerate()
            .map(|(i, (stmt, cat))| json!({
                "block_idx": k + 1 + (i % req_span),
                "statement": stmt,
                "category": cat,
                "mandatory": false
            }))
            .collect();
        ok_result(call(&mut svc, "submit_block_marks", json!({
            "project_id": "PRJ-0001", "file_id": fid,
            "marks": [
                {"from": 0, "to": k, "verdict": "noise", "reason": "前置公告说明区"},
                {"from": k + 1, "to": total - 1, "verdict": "requirement", "reason": "正文要求区"}
            ],
            "requirements": reqs_json
        })));
    }

    let r = ok_result(call(&mut svc, "extract_requirements", json!({
        "project_id": "PRJ-0001"
    })));
    let store_reqs = svc.store().requirements().to_vec();
    let _ = std::fs::remove_dir_all(&ws);
    (r, store_reqs)
}

#[test]
fn real_filename_merges_through_same_path_as_fixture_name() {
    if !corpus_ready() {
        return;
    }
    // 两组：夹具名（有前缀）与真实名（无前缀）——同一物理文件
    let (r_fix, reqs_fix) = run_pipeline("fixture-name", "A-招标文件.docx", true);
    let (r_real, reqs_real) =
        run_pipeline("real-name", "光纤租赁费招标文件-文本修复版v5w.docx", true);
    // 无标记组（真实名）：正则产物全量保留（no-op 面）——来源字段 regex 验证面
    let (r_plain, reqs_plain) =
        run_pipeline("plain-regex", "光纤租赁费招标文件-文本修复版v5w.docx", false);

    // ---- 验收 1：merge 统计字段逐项一致（同一合并路径）
    for field in [
        "regex_total",
        "regex_dropped_covered",
        "agent_requirements",
        "uncovered_blocks",
        "coverage_complete",
    ] {
        assert_eq!(
            r_fix["merge"][field], r_real["merge"][field],
            "merge.{field} 两组必须一致（同路径）"
        );
    }
    assert_eq!(
        r_fix["merge"]["agent_requirements"].as_u64().unwrap(),
        67,
        "67 条 Agent 提交必须全部被消费（修复前 0——faces 空门禁拦截）"
    );

    // ---- 验收 2：类别计数（Agent 项 + 正则兜底 ≥ issue 基线）
    let by_cat_real = |c: &str| reqs_real.iter().filter(|r| r.category == c).count();
    let tech_real = by_cat_real("技术参数");
    let comm_real = by_cat_real("商务要求");
    assert!(
        tech_real >= 11,
        "技术参数 ≥11（issue 基线：修复前 1）: {tech_real}"
    );
    assert!(
        comm_real >= 13,
        "商务要求 ≥13（issue 基线：修复前 0——类别不存在）: {comm_real}"
    );

    // ---- 验收 3：关键参数可检索（数值单位完整）——REQ 库联合检索面
    //（params.statement + anchors.excerpt）
    let searchable: String = reqs_real
        .iter()
        .map(|r| {
            let stmt = r
                .params
                .get("statement")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let exc: String = r.anchors.iter().map(|a| a.excerpt.as_str()).collect();
            format!("{stmt}{exc}")
        })
        .collect();
    for kw in ["完好率", "360 分钟", "295", "30M/s", "15 日历天"] {
        assert!(
            searchable.contains(kw),
            "关键参数「{kw}」在抽取结果中不可检索（数值单位丢失 = 修复未达）"
        );
    }

    // ---- 验收 4：来源字段对称
    //   Agent 项（标记组）：marked_by=agent 恰 67 条；
    //   正则项（无标记组——全覆盖标记下正则按 Agent 优先全丢弃是预期行为，
    //   故 regex 面在 no-op 组验证）：全量 marked_by=regex。
    let agent_n = reqs_real
        .iter()
        .filter(|r| r.params.get("marked_by").and_then(|v| v.as_str()) == Some("agent"))
        .count();
    assert_eq!(agent_n, 67, "Agent 项 marked_by=agent 恰 67 条");
    assert_eq!(
        r_plain["merge"]["regex_dropped_covered"].as_u64().unwrap(),
        0,
        "无标记组零丢弃（no-op 面）"
    );
    assert_eq!(
        r_plain["counts"]["total"].as_u64().unwrap(),
        r_plain["merge"]["regex_total"].as_u64().unwrap(),
        "无标记组 total == 正则总数"
    );
    assert!(
        !reqs_plain.is_empty(),
        "corpus-A 必有正则产物（GT 25 埋点件）"
    );
    assert_eq!(
        reqs_plain
            .iter()
            .filter(|r| r.params.get("marked_by").and_then(|v| v.as_str()) == Some("regex"))
            .count(),
        reqs_plain.len(),
        "无标记组全量 marked_by=regex（来源字段完整化）"
    );
    // 全量 REQ 无缺来源字段（两组）
    assert!(
        reqs_real
            .iter()
            .chain(reqs_plain.iter())
            .all(|r| r.params.get("marked_by").is_some()),
        "全量 REQ 均带来源字段（regex/agent）"
    );

    // ---- Agent 项溯源面：锚 = 块原文 + locator 表位（验收 4 定位面）
    let a1 = reqs_real
        .iter()
        .find(|r| r.id == "REQ-A0001")
        .expect("REQ-A0001 在库");
    assert!(!a1.anchors[0].excerpt.is_empty(), "Agent 项锚非空（块原文）");
    assert!(!a1.anchors[0].locator.is_empty(), "Agent 项 locator 在位");

    // ---- 两组 REQ 库逐位一致（类别 + statement 面——id 因文件名无关
    //      但 agent 序号全局递增，两组应完全同构）
    assert_eq!(reqs_fix.len(), reqs_real.len(), "两组 REQ 库同构");
    for (a, b) in reqs_fix.iter().zip(reqs_real.iter()) {
        assert_eq!(a.id, b.id, "id 同构");
        assert_eq!(a.category, b.category, "类别同构");
        assert_eq!(
            a.params.get("statement"),
            b.params.get("statement"),
            "statement 同构"
        );
    }
}

// ---------------------------------------------------------------- #425

/// #425：补遗冲突检测按 kind 识别——真实命名的补遗（无 C- 前缀）不再漏检。
///
/// 夹具：`澄清修改文件-3.1.1.docx`（真实命名——无 C- 前缀，条款 3.1.1/3.2.1
/// 与主文件同号异文——冲突面真源；由 scripts/make_425_addendum.py 生成）。
/// - kind=补遗 导入 → 主文件 × 补遗同条款号异文的冲突**必须检出**（修复前：
///   `C-` 前缀过滤漏检——#196/#250 同根因第三处）；
/// - 对照组：同件 + kind=答疑 → 不入补遗面（kind 值域 guard），冲突恒空。
#[test]
fn addenda_conflict_identifies_by_kind_not_prefix() {
    let main = fixtures().join("A-招标文件.docx");
    let addendum = fixtures().join("澄清修改文件-3.1.1.docx");
    if bid_parse::env_preflight::skip_if_fixture_missing(
        "merge_kind_gate_425_addenda_kind",
        &[&main, &addendum],
    ) {
        return; // SKIP 横幅已打出
    }

    // ---- 组 A：真实命名（无前缀）+ kind=补遗 → 冲突必须检出
    {
        let ws = tmp_ws("addenda-kind-a");
        let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(&fixtures());
        ok_result(call(&mut svc, "create_bid_project", json!({
            "name": "#425 补遗 kind 识别——真实命名", "bid_deadline": "2026-12-31",
            "mode": "明标", "external_key": "addenda-kind-a",
        })));
        ok_result(call(&mut svc, "import_tender_files", json!({
            "project_id": "PRJ-0001",
            "files": [
                { "upload_ref": main.display().to_string(), "filename": "A-招标文件.docx", "kind": "招标文件" },
                { "upload_ref": addendum.display().to_string(), "filename": "澄清修改文件-3.1.1.docx", "kind": "补遗" },
            ],
        })));
        let r = ok_result(call(&mut svc, "extract_requirements", json!({
            "project_id": "PRJ-0001"
        })));
        let conflicts = r["conflicts"].as_array().expect("conflicts 字段在位");
        // 本组唯一的补遗面就是无前缀真实命名件——冲突非空即归因于它
        //（冲突只能从 addenda_names 循环里产生）；夹具条款 3.1.1/3.2.1 两对
        // 同号异文 → 恰 2 条冲突。
        assert_eq!(
            conflicts.len(),
            2,
            "无 C- 前缀的补遗（kind=补遗）冲突必须检出——修复前按前缀过滤静默漏检（#425）：{conflicts:?}"
        );
        assert!(
            conflicts.iter().all(|c| {
                let reason = c["reason"].as_str().unwrap_or("");
                reason.contains("3.1.1") || reason.contains("3.2.1")
            }),
            "冲突理由应指向同号异文条款：{conflicts:?}"
        );
        let _ = std::fs::remove_dir_all(ws);
    }

    // ---- 组 B（对照）：同物理件 + kind=答疑 → 不入补遗面，冲突恒空
    {
        let ws = tmp_ws("addenda-kind-b");
        let mut svc = BidService::open(&ws).expect("打开工作区").with_fs_root(&fixtures());
        ok_result(call(&mut svc, "create_bid_project", json!({
            "name": "#425 对照——答疑不入补遗面", "bid_deadline": "2026-12-31",
            "mode": "明标", "external_key": "addenda-kind-b",
        })));
        ok_result(call(&mut svc, "import_tender_files", json!({
            "project_id": "PRJ-0001",
            "files": [
                { "upload_ref": main.display().to_string(), "filename": "A-招标文件.docx", "kind": "招标文件" },
                { "upload_ref": addendum.display().to_string(), "filename": "澄清修改文件-3.1.1.docx", "kind": "答疑" },
            ],
        })));
        let r = ok_result(call(&mut svc, "extract_requirements", json!({
            "project_id": "PRJ-0001"
        })));
        let conflicts = r["conflicts"].as_array().expect("conflicts 字段在位");
        assert!(
            conflicts.is_empty(),
            "kind=答疑 不入补遗面（值域 guard——冲突只能从补遗面产生）：{conflicts:?}"
        );
        let _ = std::fs::remove_dir_all(ws);
    }
}
