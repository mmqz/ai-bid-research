//! #145 任务 1：search_evidence 工具验收（契约 §5 双通道检索——朴素实现）。
//!
//! 验收口径（issue #145 原文）：
//! ```text
//! [x] search_evidence 出现在 tools/list 且 implemented（工具数口径见下）
//! [x] exact 模式能查（cert=ISO9001）
//! [x] semantic 模式能查（query="营业执照"）
//! [x] hybrid 合并去重（match_type=both / ev_id 唯一）
//! [x] 过期证据降权（score×0.5）+ warning
//! [x] 排序确定性（score DESC, ev_id ASC——两次调用 serde 全等，R-15 可重放）
//! ```
//!
//! **工具数口径（如实登记）**：`TOOL_NAMES` 共 **20** 个，`search_evidence`
//! 本来就在声明面里（#139 报告「20 声明 / 19 实装」）——本轮把它从
//! `UNIMPLEMENTED_INTERNAL` 变实装，**声明数=实装数=20**。#145 任务书写的
//! 「21 个工具」来自把「20 实装 + 1 新增」的误算：不为凑数往 TOOL_NAMES
//! 里塞第 21 个工具（单一真源不改口径）。
//!
//! 全部用例走 `dispatch_with`（工具面全链：参数校验 → 通道计算 → 降权 →
//! 排序 → 截断 → 信封组装），不绕过壳层直测内部函数。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};

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

fn search(svc: &mut BidService, pid: &str, args: Value) -> Value {
    let mut full = json!({ "project_id": pid, "query": args.get("query").cloned().unwrap_or(json!("(默认查询)")) });
    if let Some(obj) = args.as_object() {
        for (k, v) in obj {
            if k != "query" {
                full[k.as_str()] = v.clone();
            }
        }
    }
    ok_result(call(svc, "search_evidence", full))
}

fn hit_ids(r: &Value) -> Vec<String> {
    r["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["ev_id"].as_str().unwrap().to_string())
        .collect()
}

fn hit_of<'a>(r: &'a Value, ev_id: &str) -> &'a Value {
    r["hits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["ev_id"].as_str() == Some(ev_id))
        .unwrap_or_else(|| panic!("hits 里找不到 {ev_id}: {}", r["hits"]))
}

// ---------------------------------------------------------------- 夹具

/// 造一条证据（语义位四元组互异——不触发查重/换版）。
fn ev(
    kind: EvidenceKind,
    title: &str,
    scope: &str,
    valid_until: Option<&str>,
    extra: &[(&str, Value)],
) -> EvidenceSpec {
    let tag = title; // 标题唯一 → sha 也唯一
    let mut spec = EvidenceSpec {
        kind,
        title: title.to_string(),
        source_file: format!("EV-{tag}.pdf"),
        source_sha256: format!("sha-{tag}"),
        locator: EngineLocator::DocxParagraph {
            paragraph_index: 1,
            text_fingerprint: format!("fnv64:{tag}"),
        },
        fingerprint: format!("fnv64:{tag}"),
        valid_from: Some("2023-01-01".into()),
        valid_until: valid_until.map(str::to_string),
        subject_name: "投标人甲".into(),
        scope: scope.to_string(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(), // #147
    };
    for (k, v) in extra {
        match *k {
            "client" => spec.client = v.as_str().map(str::to_string),
            "amount_wan" => spec.amount_wan = v.as_f64(),
            "completed_on" => spec.completed_on = v.as_str().map(str::to_string),
            _ => panic!("未知 extra 键 {k}"),
        }
    }
    spec
}

/// 检索验收夹具：项目 + 六条证据（覆盖五 exact 字段 / 三态有效期 / 业绩业务面）。
///
/// | 证据 | kind | title | 有效期口径 |
/// |------|------|-------|-----------|
/// | ISO9001 证书 | certificate | ISO9001 质量管理体系认证证书 | 2027-06-30（远期） |
/// | 营业执照 | qualification | 营业执照 | 无（长期证） |
/// | 弱电业绩 | performance | 智慧园区弱电工程合同（含 920 万合同额） | 无 |
/// | 设备手册 | product_doc | DS-KD6900 可视对讲室内机 产品手册 | 无 |
/// | 售后方案 | solution_module | 售后服务方案（响应条款 6.3.2） | 无 |
/// | ISO14001 证书 | certificate | ISO14001 环境管理体系认证证书 | 2026-11-30（对开标日 2026-12-31 已过期） |
fn search_store() -> (BidService, String) {
    let mut svc = BidService::new();
    let pid = {
        let st = svc.store_mut();
        let (pid, _) = st
            .create_project("#145 检索验收", "ZFCG-2026-SE", "明标", "search-evidence-key")
            .unwrap();
        st.register_evidence(ev(
            EvidenceKind::Certificate,
            "ISO9001 质量管理体系认证证书",
            "质量管理体系",
            Some("2027-06-30"),
            &[],
        ))
        .unwrap();
        st.register_evidence(ev(
            EvidenceKind::Qualification,
            "营业执照",
            "中华人民共和国境内注册的法人",
            None,
            &[],
        ))
        .unwrap();
        st.register_evidence(
            ev(
                EvidenceKind::Performance,
                "智慧园区弱电工程合同",
                "弱电工程",
                None,
                &[
                    ("client", json!("某某工业园区管委会")),
                    ("amount_wan", json!(920.0)),
                    ("completed_on", json!("2024.06")),
                ],
            ),
        )
        .unwrap();
        st.register_evidence(ev(
            EvidenceKind::ProductDoc,
            "DS-KD6900 可视对讲室内机 产品手册",
            "可视对讲设备",
            None,
            &[],
        ))
        .unwrap();
        st.register_evidence(ev(
            EvidenceKind::SolutionModule,
            "售后服务方案（响应条款 6.3.2）",
            "售后服务",
            None,
            &[],
        ))
        .unwrap();
        st.register_evidence(ev(
            EvidenceKind::Certificate,
            "ISO14001 环境管理体系认证证书",
            "环境管理体系",
            Some("2026-11-30"),
            &[],
        ))
        .unwrap();
        pid
    };
    (svc, pid)
}

// ---------------------------------------------------------------- 验收用例

/// tools/list 声明 + implemented 双面（工具数口径见模块注释——20 不是 21，如实）。
#[test]
fn tools_list_declares_search_evidence_implemented() {
    let schema = bid_mcp::tools_schema();
    let tools = schema["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(
        names.contains(&"search_evidence"),
        "tools/list 缺 search_evidence：{names:?}"
    );
    let se = tools.iter().find(|t| t["name"].as_str() == Some("search_evidence")).unwrap();
    assert_eq!(se["implemented"].as_bool(), Some(true), "implemented 必须 true：{se}");
    // 声明数 = 实装数 = 20（#145 任务书的「21」是误算——见模块注释）
    let n_impl = tools.iter().filter(|t| t["implemented"].as_bool() == Some(true)).count();
    assert_eq!(names.len(), 20, "TOOL_NAMES 声明数应为 20：{names:?}");
    assert_eq!(n_impl, 20, "实装数应与声明数一致（20）：{names:?}");
}

/// 验收①：exact 模式 cert=ISO9001（有界短语——不误报 ISO14001）。
#[test]
fn exact_mode_cert_query() {
    let (mut svc, pid) = search_store();
    let r = search(&mut svc, &pid, json!({
        "query": "ISO9001 质量管理体系认证",
        "mode": "exact",
        "exact": { "field": "cert", "value": "ISO9001" },
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "cert=ISO9001 应恰命中 1 条（不含 ISO14001）：{ids:?}");
    let h = &r["hits"][0];
    assert_eq!(h["match_type"].as_str(), Some("exact"));
    assert_eq!(h["score"].as_f64(), Some(1.0));
    assert_eq!(h["kind"].as_str(), Some("certificate"));
    assert_eq!(h["chunk_id"].as_str().unwrap(), format!("{}#whole", ids[0]));
    assert_eq!(r["trace"]["exact_k"].as_u64(), Some(1));
    assert_eq!(r["trace"]["semantic_k"].as_u64(), Some(0), "exact 模式不跑语义通道");
    assert_eq!(r["trace"]["merged_by"].as_str(), Some("ev_id"));
    // 前缀不误报：ISO 是 ISO9001/ISO14001 的前缀，但不是有界短语
    let r2 = search(&mut svc, &pid, json!({
        "query": "ISO", "mode": "exact",
        "exact": { "field": "cert", "value": "ISO" },
    }));
    assert_eq!(hit_ids(&r2).len(), 0, "cert=ISO 不应命中 ISO9001/ISO14001（有界短语口径）");
}

/// 验收①补：五字段全覆盖——model / clause_no / amount / date。
#[test]
fn exact_mode_all_five_fields() {
    let (mut svc, pid) = search_store();
    // model：产品型号（值含 '-'，有界短语可命中）
    let r = search(&mut svc, &pid, json!({
        "query": "设备型号", "mode": "exact",
        "exact": { "field": "model", "value": "DS-KD6900" },
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "model=DS-KD6900 应命中产品手册：{ids:?}");
    // clause_no：条款号（值含 '.'）
    let r = search(&mut svc, &pid, json!({
        "query": "条款响应", "mode": "exact",
        "exact": { "field": "clause_no", "value": "6.3.2" },
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "clause_no=6.3.2 应命中售后方案：{ids:?}");
    // amount：合同金额（万元）数值全等——920 命中 / 921 不中 / 字符串 "920" 也中
    let r = search(&mut svc, &pid, json!({
        "query": "合同金额", "mode": "exact",
        "exact": { "field": "amount", "value": 920.0 },
    }));
    assert_eq!(hit_ids(&r).len(), 1, "amount=920 应命中业绩合同");
    let r = search(&mut svc, &pid, json!({
        "query": "合同金额", "mode": "exact",
        "exact": { "field": "amount", "value": 921.0 },
    }));
    assert_eq!(hit_ids(&r).len(), 0, "amount=921 不应命中");
    let r = search(&mut svc, &pid, json!({
        "query": "合同金额", "mode": "exact",
        "exact": { "field": "amount", "value": "920" },
    }));
    assert_eq!(hit_ids(&r).len(), 1, "amount=\"920\"（数字字符串）应命中");
    // date：completed_on「2024.06」归一后与 YYYY-MM 月前缀互认；valid_until 全等
    let r = search(&mut svc, &pid, json!({
        "query": "完成日期", "mode": "exact",
        "exact": { "field": "date", "value": "2024-06" },
    }));
    assert_eq!(hit_ids(&r).len(), 1, "date=2024-06 应命中 completed_on=2024.06 的业绩");
    let r = search(&mut svc, &pid, json!({
        "query": "有效期", "mode": "exact",
        "exact": { "field": "date", "value": "2027-06-30" },
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "date=2027-06-30 应命中 ISO9001 的 valid_until：{ids:?}");
}

/// 验收②：semantic 模式 query="营业执照"（朴素子串/分词——覆盖分计分）。
#[test]
fn semantic_mode_query() {
    let (mut svc, pid) = search_store();
    let r = search(&mut svc, &pid, json!({
        "query": "营业执照", "mode": "semantic",
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "semantic「营业执照」应命中 1 条：{ids:?}");
    let h = &r["hits"][0];
    assert_eq!(h["match_type"].as_str(), Some("semantic"));
    assert_eq!(h["score"].as_f64(), Some(1.0), "单 token 全中 → 1.0");
    assert_eq!(r["trace"]["semantic_k"].as_u64(), Some(1));
    assert_eq!(r["trace"]["exact_k"].as_u64(), Some(0));

    // 覆盖分：两 token 中一（「弱电工程」中、「业绩」不在文案里）→ 0.5
    let r = search(&mut svc, &pid, json!({
        "query": "弱电工程 业绩", "mode": "semantic",
    }));
    let h = hit_of(&r, &hit_ids(&r)[0]);
    assert_eq!(
        h["score"].as_f64(),
        Some(0.5),
        "两 token 中一 → 0.5（命中 token 数 / 查询 token 数）：{}",
        h["score"]
    );
    // 单字噪声丢弃：query 只有单字 → 无有效 token → 0 hits（不报错）
    let r = search(&mut svc, &pid, json!({ "query": "的", "mode": "semantic" }));
    assert_eq!(hit_ids(&r).len(), 0, "单字 token 应被丢弃（无有效查询面）");
}

/// 验收③：hybrid 合并去重——同证据双通道命中 → match_type="both"、分取 exact 1.0；
/// ev_id 全局唯一（去重不重排）。
#[test]
fn hybrid_merge_dedup() {
    let (mut svc, pid) = search_store();
    let r = search(&mut svc, &pid, json!({
        "query": "ISO9001 质量管理体系",
        "exact": { "field": "cert", "value": "ISO9001" },
        // mode 缺省 = hybrid
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "hybrid 应合并去重为 1 条：{ids:?}");
    let h = &r["hits"][0];
    assert_eq!(h["match_type"].as_str(), Some("both"), "双通道同中 → both");
    assert_eq!(h["score"].as_f64(), Some(1.0), "both 分取 exact 的 1.0（exact 优先）");
    assert_eq!(r["trace"]["semantic_k"].as_u64(), Some(1));
    assert_eq!(r["trace"]["exact_k"].as_u64(), Some(1));
    // 双通道并集（各中不同证据）：语义中营业执照 + exact 中 ISO14001 → 两条
    let r = search(&mut svc, &pid, json!({
        "query": "营业执照",
        "exact": { "field": "cert", "value": "ISO14001" },
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 2, "hybrid 并集应有 2 条：{ids:?}");
    let mt: Vec<&str> = r["hits"].as_array().unwrap().iter().map(|h| h["match_type"].as_str().unwrap()).collect();
    assert!(mt.contains(&"exact") && mt.contains(&"semantic"), "应各留本通道标记：{mt:?}");
}

/// 验收④：过期证据降权 + warning（开标日 = filters.valid_at）。
///
/// - ISO14001（valid_until=2026-11-30 < 开标日 2026-12-31）→ score×0.5 +
///   EVIDENCE_EXPIRED warning；
/// - ISO9001（2027-06-30，远期）→ 1.0 无告警；
/// - 不给 valid_at → 不做时间面降权（开标日口径归调用方）。
#[test]
fn expired_downweight_and_warning() {
    let (mut svc, pid) = search_store();
    let q = json!({ "query": "管理体系认证证书", "mode": "semantic" });
    // （夹具中标题含「管理体系认证证书」的：ISO9001 与 ISO14001 两条）

    // ---- 不给开标日：都按原分
    let r0 = search(&mut svc, &pid, q.clone());
    for x in r0["hits"].as_array().unwrap() {
        assert_eq!(x["score"].as_f64(), Some(1.0), "无 valid_at 不降权：{}", x["ev_id"]);
    }
    assert_eq!(
        r0["warnings"].as_array().map(Vec::len),
        Some(0),
        "无 valid_at 不产时间面告警"
    );

    // ---- 给开标日 2026-12-31：ISO14001 过期降权 + EXPIRED warning
    let mut q2 = q.clone();
    q2["filters"] = json!({ "valid_at": "2026-12-31" });
    let r = search(&mut svc, &pid, q2);
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 2, "两条体系证书都应命中：{ids:?}");
    // 找出谁是 ISO14001（按 title 判——不依赖 EV 序脆弱断言）
    let (mut s_9001, mut s_14001) = (0.0, 0.0);
    for x in r["hits"].as_array().unwrap() {
        let t = x["title"].as_str().unwrap();
        if t.contains("ISO9001") {
            s_9001 = x["score"].as_f64().unwrap();
            assert_eq!(x["match_type"].as_str(), Some("semantic"));
        }
        if t.contains("ISO14001") {
            s_14001 = x["score"].as_f64().unwrap();
        }
    }
    assert_eq!(s_9001, 1.0, "远期证据不降权");
    assert_eq!(s_14001, 0.5, "过期证据应降权 ×0.5（1.0 → 0.5）");
    // 排序：score DESC——ISO9001(1.0) 在 ISO14001(0.5) 前
    let titles: Vec<&str> = r["hits"].as_array().unwrap().iter().map(|h| h["title"].as_str().unwrap()).collect();
    let p9 = titles.iter().position(|t| t.contains("ISO9001")).unwrap();
    let p14 = titles.iter().position(|t| t.contains("ISO14001")).unwrap();
    assert!(p9 < p14, "score DESC：ISO9001(1.0) 应排在 ISO14001(0.5) 前：{titles:?}");
    // warning 面
    let warns: Vec<String> = r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    let expired_ev = {
        let mut e = String::new();
        for x in r["hits"].as_array().unwrap() {
            if x["title"].as_str().unwrap().contains("ISO14001") {
                e = x["ev_id"].as_str().unwrap().to_string();
            }
        }
        e
    };
    assert!(
        warns.iter().any(|w| w.starts_with("EVIDENCE_EXPIRED:") && w.contains(&expired_ev) && w.contains("×0.5")),
        "应携带指认 {expired_ev} 的 EVIDENCE_EXPIRED 降权告警：{warns:?}"
    );
}

/// 验收④补：临期（开标日 ≤ valid_until < 开标日+90 天）→ 只告警不降权。
#[test]
fn expiring_warns_without_downweight() {
    let (mut svc, pid) = {
        let (mut s, p) = search_store();
        // 追加一条临期证据：valid_until=2027-01-15（开标日 2026-12-31 后 15 天，<90）
        s.store_mut().register_evidence(ev(
            EvidenceKind::Qualification,
            "电子与智能化工程专业承包二级资质证书",
            "电子与智能化工程",
            Some("2027-01-15"),
            &[],
        )).unwrap();
        (s, p)
    };
    let r = search(&mut svc, &pid, json!({
        "query": "专业承包二级资质证书", "mode": "semantic",
        "filters": { "valid_at": "2026-12-31" },
    }));
    let h = &r["hits"][0];
    assert_eq!(h["score"].as_f64(), Some(1.0), "临期不降权（#145 只要求过期降权）");
    let warns: Vec<&str> = r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(
        warns.iter().any(|w| w.starts_with("EVIDENCE_EXPIRING:") && w.contains(h["ev_id"].as_str().unwrap())),
        "应携带 EVIDENCE_EXPIRING 告警：{warns:?}"
    );
}

/// 验收⑤：排序确定性（R-15 可重放）——同参两次调用，hits 数组 serde 全等。
#[test]
fn deterministic_replay() {
    let (mut svc, pid) = search_store();
    let q = json!({
        "query": "体系 认证 证书 工程",
        "filters": { "valid_at": "2026-12-31" },
    });
    let r1 = search(&mut svc, &pid, q.clone());
    let r2 = search(&mut svc, &pid, q.clone());
    assert_eq!(r1["hits"], r2["hits"], "同参两次调用 hits 应 serde 全等（可重放）");
    // 排序不变式：score DESC, ev_id ASC
    let hits = r1["hits"].as_array().unwrap();
    for w in hits.windows(2) {
        let (a, b) = (w[0].clone(), w[1].clone());
        let (sa, sb) = (a["score"].as_f64().unwrap(), b["score"].as_f64().unwrap());
        let (ia, ib) = (a["ev_id"].as_str().unwrap(), b["ev_id"].as_str().unwrap());
        assert!(
            sa > sb || (sa == sb && ia < ib),
            "排序违例（score DESC, ev_id ASC）：{ia}({sa}) → {ib}({sb})"
        );
    }
}

/// 换版旧证（SUPERSEDED）不入检索面——同语义位换指纹重登后只查到新版。
#[test]
fn superseded_excluded_from_search() {
    let (mut svc, pid) = search_store();
    // ISO9001 换版：同语义位（kind/title/subject/scope 四元组）换指纹 →
    // 旧记录单向降级 SUPERSEDED、新记录 2028-06-30 生效
    {
        let st = svc.store_mut();
        let mut spec = ev(
            EvidenceKind::Certificate,
            "ISO9001 质量管理体系认证证书",
            "质量管理体系",
            Some("2028-06-30"),
            &[],
        );
        spec.source_sha256 = "sha-iso9001-v2".into();
        spec.fingerprint = "fnv64:iso9001v2".into();
        st.register_evidence(spec).unwrap();
    }
    let r = search(&mut svc, &pid, json!({
        "query": "ISO9001", "mode": "exact",
        "exact": { "field": "cert", "value": "ISO9001" },
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "换版后 cert=ISO9001 应只命中最新版：{ids:?}");
    let new_ev = &r["hits"][0];
    assert_eq!(new_ev["valid_until"].as_str(), Some("2028-06-30"), "应命中新版（2028-06-30），旧版 SUPERSEDED 不入检索面");
}

/// filters 面：source_type（文件名包含）+ min_valid_until（长期证放行）。
#[test]
fn filters_source_type_and_min_valid_until() {
    let (mut svc, pid) = search_store();
    // source_type：命中 source_file 包含「弱电工程」的业绩合同
    let r = search(&mut svc, &pid, json!({
        "query": "合同", "mode": "semantic",
        "filters": { "source_type": "弱电工程" },
    }));
    let ids = hit_ids(&r);
    assert!(ids.len() == 1, "source_type=弱电工程 应只留业绩合同：{ids:?}");
    // min_valid_until=2027-01-01：ISO14001(2026-11-30) 滤除；
    // ISO9001(2027-06-30) 保留；营业执照/业绩（无有效期=长期证）放行
    let r = search(&mut svc, &pid, json!({
        "query": "证书 合同 营业执照 方案 手册 认证",
        "mode": "semantic",
        "filters": { "min_valid_until": "2027-01-01" },
    }));
    let titles: Vec<&str> = r["hits"].as_array().unwrap().iter().map(|h| h["title"].as_str().unwrap()).collect();
    assert!(
        titles.iter().all(|t| !t.contains("ISO14001")),
        "ISO14001（2026-11-30 < 2027-01-01）应被滤除：{titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.contains("ISO9001")),
        "ISO9001（2027-06-30）应保留：{titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.contains("营业执照")),
        "长期证（无 valid_until）应放行：{titles:?}"
    );
}

/// top_k / threshold 面：截断与阈值过滤（作用于降权后的分值）。
#[test]
fn topk_and_threshold() {
    let (mut svc, pid) = search_store();
    // 全中面：6 条证据都含「证书/合同/方案/手册/认证」之一 … 用宽查询全捞
    let r = search(&mut svc, &pid, json!({
        "query": "证书 合同 方案 手册 认证 营业执照",
        "mode": "semantic",
        "top_k": 2,
    }));
    assert_eq!(hit_ids(&r).len(), 2, "top_k=2 截断");
    assert!(r["total"].as_u64().unwrap() > 2, "total 应为截断前总数：{}", r["total"]);
    assert_eq!(r["top_k"].as_u64(), Some(2));
    // threshold：弱电工程两 token 中一 → 0.5；阈值 0.6 应滤掉
    let r = search(&mut svc, &pid, json!({
        "query": "弱电工程 业绩", "mode": "semantic", "threshold": 0.6,
    }));
    assert_eq!(hit_ids(&r).len(), 0, "threshold=0.6 应滤掉 0.5 分命中");
    // 上界护栏：top_k 1001 拒、1000 过
    let bad = call(&mut svc, "search_evidence", json!({
        "project_id": pid, "query": "x", "top_k": 1001,
    }));
    assert_eq!(err_code(&bad), "E_UNPROCESSABLE");
    let good = call(&mut svc, "search_evidence", json!({
        "project_id": pid, "query": "x", "top_k": 1000,
    }));
    assert!(matches!(good, ToolReply::Ok(_)), "top_k=1000 应放行");
}

/// 错误面（fail-closed 全家福）：mode/exact/filters/top_k/threshold/路由/缺参。
#[test]
fn error_faces() {
    let (mut svc, pid) = search_store();
    let cases: Vec<(&str, Value)> = vec![
        ("mode 非法", json!({ "query": "x", "mode": "fuzzy" })),
        ("exact 模式缺 exact 子句", json!({ "query": "x", "mode": "exact" })),
        (
            "semantic 与 exact 子句互斥",
            json!({ "query": "x", "mode": "semantic", "exact": { "field": "cert", "value": "ISO9001" } }),
        ),
        (
            "exact.field 非法",
            json!({ "query": "x", "exact": { "field": "foo", "value": "1" } }),
        ),
        ("exact 缺 value", json!({ "query": "x", "exact": { "field": "cert" } })),
        (
            "cert value 非字符串",
            json!({ "query": "x", "exact": { "field": "cert", "value": 42 } }),
        ),
        (
            "amount value 非数值",
            json!({ "query": "x", "exact": { "field": "amount", "value": "九百二" } }),
        ),
        ("filters.tags 不支持", json!({ "query": "x", "filters": { "tags": ["a"] } })),
        ("filters 未知键", json!({ "query": "x", "filters": { "kind": "certificate" } })),
        ("valid_at 非 ISO", json!({ "query": "x", "filters": { "valid_at": "2026/12/31" } })),
        ("min_valid_until 非 ISO", json!({ "query": "x", "filters": { "min_valid_until": "2026-13-01" } })),
        ("top_k=0", json!({ "query": "x", "top_k": 0 })),
        ("threshold 越界", json!({ "query": "x", "threshold": 1.5 })),
        ("threshold 非数值", json!({ "query": "x", "threshold": "高" })),
    ];
    for (name, extra) in cases {
        let mut args = json!({ "project_id": pid });
        for (k, v) in extra.as_object().unwrap() {
            args[k.as_str()] = v.clone();
        }
        let reply = call(&mut svc, "search_evidence", args);
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "「{name}」应 E_UNPROCESSABLE");
    }
    // 缺 query / 缺 project_id / 项目路由
    let r = call(&mut svc, "search_evidence", json!({ "project_id": pid }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE", "缺 query 应拒");
    let r = call(&mut svc, "search_evidence", json!({ "query": "x" }));
    assert_eq!(err_code(&r), "E_UNPROCESSABLE", "缺 project_id 应拒");
    let r = call(&mut svc, "search_evidence", json!({ "project_id": "PRJ-9999", "query": "x" }));
    assert_eq!(err_code(&r), "E_NOTFOUND", "项目不存在应 E_NOTFOUND");
}

/// 只读面：DRAFT 态（未导入未抽取）即可调——检索不进状态机（同 list_evidence 口径）。
#[test]
fn searchable_in_draft_state() {
    let mut svc = BidService::new();
    let pid = {
        let st = svc.store_mut();
        let (pid, _) = st
            .create_project("#145 DRAFT 态检索", "ZFCG-2026-SE2", "明标", "search-draft-key")
            .unwrap();
        st.register_evidence(ev(
            EvidenceKind::Qualification,
            "营业执照",
            "法人资格",
            None,
            &[],
        ))
        .unwrap();
        pid
    };
    let r = search(&mut svc, &pid, json!({ "query": "营业执照", "mode": "semantic" }));
    assert_eq!(hit_ids(&r).len(), 1, "DRAFT 态检索应可用（只读无状态机裁决）");
    // 空库检索 → 0 hits 不报错（新项目无证据面）
    let mut svc2 = BidService::new();
    let pid2 = {
        let st = svc2.store_mut();
        let (p, _) = st.create_project("空库", "Z3", "明标", "empty-key").unwrap();
        p
    };
    let r = search(&mut svc2, &pid2, json!({ "query": "营业执照" }));
    assert_eq!(hit_ids(&r).len(), 0, "空库应 0 hits 而非报错");
    assert_eq!(r["warnings"].as_array().map(Vec::len), Some(0));
}
