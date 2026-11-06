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
//! **工具数口径（如实登记）**：`TOOL_NAMES` 在 #145 时点共 **20** 个，
//! `search_evidence` 本来就在声明面里（#139 报告「20 声明 / 19 实装」）——
//! 那一轮把它从 `UNIMPLEMENTED_INTERNAL` 变实装，声明数=实装数=20。#145
//! 任务书写的「21 个工具」来自把「20 实装 + 1 新增」的误算：不为凑数往
//! TOOL_NAMES 里塞第 21 个工具（单一真源不改口径）。
//! （#184 起新增 `record_addendum_waiver`——工具面变 21，非凑数：分级
//! 阻塞的伴生确认面，见 dispatch_contract.rs。）
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
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
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

/// tools/list 声明 + implemented 双面（工具数口径见模块注释——#145 时点
/// 20，#184 增 record_addendum_waiver 后 21，#226 增 set_matrix_override
/// 后 22，如实）。
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
    // 声明数 = 实装数（#145 时点 20；#184 增 record_addendum_waiver → 21；
    // #219 增三覆盖工具 → 24；#226 增 set_matrix_override → 25；
    // #243 增 recover_validating_state → 26；#254 增分段三工具 → 29；
    // #285 ① 增 get_star_coverage → 30）
    let n_impl = tools.iter().filter(|t| t["implemented"].as_bool() == Some(true)).count();
    assert_eq!(names.len(), 30, "TOOL_NAMES 声明数应为 30（21 + #219 全块覆盖三工具 + #226 确认面 + #243 恢复出口 + #254 分段三工具 + #285 ★漏标检测）：{names:?}");
    assert_eq!(n_impl, 30, "实装数应与声明数一致（30）：{names:?}");
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

// ---------------------------------------------------------------- #147 结构化字段（EvidenceFields）

/// #147 夹具：项目 + 三条带结构化字段的证据（合同/证书/人员各一），
/// 以及一条**无结构化字段**的老式业绩（验证回落）。
fn fields_store() -> (BidService, String) {
    let mut svc = BidService::new();
    let pid = {
        let st = svc.store_mut();
        let (pid, _) = st
            .create_project("#147 结构化字段检索", "ZFCG-2026-SE3", "明标", "fields-key")
            .unwrap();
        // (a) 合同类：结构化金额/日期/证书号齐备
        let mut a = ev(
            EvidenceKind::Contract,
            "智慧园区一期智能化工程合同",
            "智能化工程",
            None,
            &[],
        );
        a.fields.project_name = Some("智慧园区一期智能化工程".into());
        a.fields.client = Some("智慧园区管委会".into());
        a.fields.amount_wan = Some(1280.5);
        a.fields.signed_at = Some("2022-03-15".into());
        a.fields.finished_at = Some("2023-01-20".into());
        st.register_evidence(a).unwrap();
        // (b) 证书类：结构化证书号 / 名称 / 发证机关
        let mut b = ev(
            EvidenceKind::Certificate,
            "建筑企业资质证书（结构化）",
            "电子与智能化",
            None,
            &[],
        );
        b.fields.cert_name = Some("电子与智能化工程专业承包二级资质证书".into());
        b.fields.cert_no = Some("D234000123".into());
        b.fields.issuer = Some("省住房和城乡建设厅".into());
        st.register_evidence(b).unwrap();
        // (c) 人员类：结构化姓名/职称/年限（日期面留空——不参与 date 查询）
        let mut c = ev(
            EvidenceKind::Personnel,
            "项目负责人注册建造师",
            "机电工程",
            None,
            &[],
        );
        c.fields.person_name = Some("张三".into());
        c.fields.person_title = Some("高级工程师".into());
        c.fields.years = Some(12);
        st.register_evidence(c).unwrap();
        // (d) 老式业绩：无结构化字段（amount_wan/completed_on 走顶层——回落面）
        st.register_evidence(ev(
            EvidenceKind::Performance,
            "智慧园区弱电工程合同（老式）",
            "弱电工程",
            None,
            &[
                ("amount_wan", json!(920.0)),
                ("completed_on", json!("2024.06")),
            ],
        ))
        .unwrap();
        pid
    };
    (svc, pid)
}

/// 验收④：register_evidence 接受 fields（工具面全链）——落账后 search 可精确查到。
#[test]
fn register_evidence_accepts_fields() {
    let (mut svc, pid) = fields_store();
    // 结构化证书号可按 cert 精确命中
    let r = search(&mut svc, &pid, json!({
        "query": "资质证书", "mode": "exact",
        "exact": { "field": "cert", "value": "D234000123" },
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "cert=D234000123 应命中结构化证书：{ids:?}");
    // 落账内容可从台账读回（工具面 → 领域面 → 序列化）
    let ev_rec = svc
        .store()
        .evidences()
        .iter()
        .find(|e| e.id == ids[0])
        .unwrap();
    assert_eq!(ev_rec.fields.cert_no.as_deref(), Some("D234000123"));
    assert_eq!(
        ev_rec.fields.cert_name.as_deref(),
        Some("电子与智能化工程专业承包二级资质证书")
    );
    assert_eq!(ev_rec.fields.issuer.as_deref(), Some("省住房和城乡建设厅"));
}

/// 验收④补：fields 参数校验失败 → E_UNPROCESSABLE，且零入库。
///
/// 走**纯 MCP 工具面全链**（create → import → extract → build_outline →
/// register_evidence）——G2 的 MATRIX_READY 前置未过时先撞 E_STATE，
/// 校验面就测不到（如实：这是踩过的坑，不靠直调 store 绕状态机）。
/// 返回 (svc, pid, A 件 file_id)——#185 后登记须以 source_file_id 锚定已导入文件。
fn matrix_ready_store() -> (BidService, String, String) {
    let ws = std::env::temp_dir().join(format!("search-fields-{}", std::process::id()));
    std::fs::create_dir_all(&ws).unwrap();
    // #183 围栏：受信导入根（canonical——词法 `..` 会被围栏预拒）
    let files_dir = {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/synth-tender/招标文件");
        std::fs::canonicalize(&p).unwrap_or(p)
    };
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(files_dir.clone());
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({ "name": "#147 fields 校验", "bid_deadline": "2026-11-30",
                "mode": "明标", "external_key": "search-fields-err" }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    let import = ok_result(call(
        &mut svc,
        "import_tender_files",
        json!({ "project_id": &pid, "files": [
            { "upload_ref": files_dir.join("A-招标文件.docx").display().to_string(),
              "filename": "A-招标文件.docx", "kind": "招标文件" } ] }),
    ));
    let fid_a = import["per_file"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["filename"].as_str() == Some("A-招标文件.docx"))
        .and_then(|p| p["file_id"].as_str())
        .unwrap()
        .to_string();
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": &pid })));
    ok_result(call(&mut svc, "build_outline", json!({ "project_id": &pid })));
    (svc, pid, fid_a)
}

#[test]
fn register_evidence_rejects_malformed_fields() {
    let (mut svc, pid, fid_a) = matrix_ready_store();
    let n_before = svc.store().evidences().len();
    let base = json!({
        "project_id": &pid, "kind": "certificate",
        "title": "结构化字段非法样例", "source_file_id": &fid_a,
        "subject_name": "投标人甲",
    });
    let cases: Vec<(&str, Value)> = vec![
        ("fields 非对象", json!("not-an-object")),
        ("signed_at 非法日期", json!({ "signed_at": "2024-13-01" })),
        ("finished_at 非法日期", json!({ "finished_at": "2024/06/15" })),
        ("amount_wan 为负", json!({ "amount_wan": -1.0 })),
        ("years 超界", json!({ "years": 101 })),
        ("years 非整数", json!({ "years": 1.5 })),
        ("amount_wan 非数值", json!({ "amount_wan": "一千" })),
    ];
    for (name, fields) in cases {
        let mut args = base.clone();
        args["fields"] = fields.clone();
        let reply = call(&mut svc, "register_evidence", args);
        assert_eq!(
            err_code(&reply),
            "E_UNPROCESSABLE",
            "「{name}」应 E_UNPROCESSABLE（fields={fields}）"
        );
    }
    assert_eq!(svc.store().evidences().len(), n_before, "拒绝路径零入库");

    // 合法 fields：null / 缺省 均可（向后兼容）
    let mut args = base.clone();
    args["fields"] = Value::Null;
    let a = ok_result(call(&mut svc, "register_evidence", args));
    let ev_id = a["ev_id"].as_str().unwrap().to_string();
    assert!(
        svc.store().evidence(&ev_id).unwrap().fields.is_empty(),
        "fields=null → 空结构"
    );

    // 合法 fields：全填 → 落账可读（工具面 → 领域面往返；#185：来源锚 A 件）
    let mut args = base.clone();
    args["title"] = json!("结构化字段合法样例");
    args["fields"] = json!({
        "project_name": "某项目", "client": "某甲方", "amount_wan": 0.0,
        "signed_at": "2024-02-29", "finished_at": "2025-01-01",
        "cert_name": "安全生产许可证", "cert_no": "AQ-2024-001",
        "issuer": "省应急管理厅", "person_name": "李四",
        "person_title": "工程师", "years": 100,
    });
    let b = ok_result(call(&mut svc, "register_evidence", args));
    let ev2 = svc
        .store()
        .evidence(b["ev_id"].as_str().unwrap())
        .unwrap()
        .clone();
    assert_eq!(ev2.fields.cert_no.as_deref(), Some("AQ-2024-001"));
    assert_eq!(ev2.fields.amount_wan, Some(0.0), "0 金额是合法边界");
    assert_eq!(ev2.fields.years, Some(100), "100 年是合法边界");
    assert_eq!(ev2.fields.signed_at.as_deref(), Some("2024-02-29"));
}

/// 验收⑤：exact 通道查结构化字段——cert 走 cert_name/cert_no（有界短语），
/// 且结构化记录不回落文本（不因标题含「资质」就误中）。
#[test]
fn exact_cert_uses_structured_fields() {
    let (mut svc, pid) = fields_store();
    // cert_name 命中（有界短语——「电子与智能化工程专业承包二级资质证书」完整段）
    let r = search(&mut svc, &pid, json!({
        "query": "资质", "mode": "exact",
        "exact": { "field": "cert", "value": "电子与智能化工程专业承包二级资质证书" },
    }));
    assert_eq!(hit_ids(&r).len(), 1, "cert_name 全等短语应命中");
    // 有界短语口径：前缀不成段 → 不命中（结构化面同样遵守）
    let r = search(&mut svc, &pid, json!({
        "query": "资质", "mode": "exact",
        "exact": { "field": "cert", "value": "电子与智能化" },
    }));
    assert_eq!(
        hit_ids(&r).len(),
        0,
        "结构化 cert_name 有界短语：不完整段不命中（且不回落 title 文本——标题含「资质证书」）"
    );
    // 结构化记录不回落：证书标题含「资质证书」，但 cert 查「资质证书」不成有界段
    // （「建筑企业资质证书（结构化）」里的是完整段——改用别的值验证不落文本）
    let r = search(&mut svc, &pid, json!({
        "query": "资质", "mode": "exact",
        "exact": { "field": "cert", "value": "建筑企业资质证书" },
    }));
    assert_eq!(
        hit_ids(&r).len(),
        0,
        "有结构化字段时 cert 不回落 title 文本（「建筑企业资质证书」只在标题里）"
    );
    // 无结构化字段的记录仍走文本（向后兼容：ISO9001 老夹具）
    let (mut svc2, pid2) = search_store();
    let r = search(&mut svc2, &pid2, json!({
        "query": "ISO9001", "mode": "exact",
        "exact": { "field": "cert", "value": "ISO9001" },
    }));
    assert_eq!(
        hit_ids(&r).len(),
        1,
        "无结构化字段的证据仍按 title 文本命中（既有行为不变）"
    );
}

/// 验收⑤补：amount 走 fields.amount_wan（首选），date 走 signed_at/finished_at。
#[test]
fn exact_amount_and_date_use_structured_fields() {
    let (mut svc, pid) = fields_store();
    // amount：结构化 1280.5
    let r = search(&mut svc, &pid, json!({
        "query": "合同额", "mode": "exact",
        "exact": { "field": "amount", "value": 1280.5 },
    }));
    assert_eq!(hit_ids(&r).len(), 1, "amount=1280.5 应命中结构化合同");
    // amount：920 只在老式业绩的顶层 amount_wan（无结构化字段 → 回落命中）
    let r = search(&mut svc, &pid, json!({
        "query": "合同额", "mode": "exact",
        "exact": { "field": "amount", "value": 920.0 },
    }));
    assert_eq!(hit_ids(&r).len(), 1, "无结构化字段 → 回落顶层 amount_wan");
    // 有结构化字段但不等于该值 → 不命中（不回落）
    let r = search(&mut svc, &pid, json!({
        "query": "合同额", "mode": "exact",
        "exact": { "field": "amount", "value": 1280.0 },
    }));
    assert_eq!(hit_ids(&r).len(), 0, "结构化金额不等 → 不命中（不回落）");
    // date：signed_at 年份前缀 2022
    let r = search(&mut svc, &pid, json!({
        "query": "签订日期", "mode": "exact",
        "exact": { "field": "date", "value": "2022" },
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "date=2022 应命中 signed_at=2022-03-15：{ids:?}");
    // date：finished_at 全等
    let r = search(&mut svc, &pid, json!({
        "query": "完成日期", "mode": "exact",
        "exact": { "field": "date", "value": "2023-01-20" },
    }));
    assert_eq!(hit_ids(&r).len(), 1, "date=2023-01-20 应命中 finished_at");
    // date：2024 只命中老式业绩 completed_on=2024.06（回落面仍工作）
    let r = search(&mut svc, &pid, json!({
        "query": "完成日期", "mode": "exact",
        "exact": { "field": "date", "value": "2024" },
    }));
    let ids = hit_ids(&r);
    assert_eq!(ids.len(), 1, "date=2024 应命中 completed_on=2024.06：{ids:?}");
    assert!(ids[0].len() > 0);
}

/// 验收⑤补二：结构化人字段不进 exact 通道（字段枚举未扩——如实登记），
/// 但落账可读；hybrid 语义通道仍可按名称搜到。
#[test]
fn personnel_fields_persisted_but_not_in_exact_enum() {
    let (mut svc, pid) = fields_store();
    let r = call(
        &mut svc,
        "search_evidence",
        json!({
            "project_id": &pid, "query": "张三",
            "exact": { "field": "person_name", "value": "张三" },
        }),
    );
    assert_eq!(
        err_code(&r),
        "E_UNPROCESSABLE",
        "exact.field 枚举未扩（person_name 不在五字段里——显式拒，不静默）"
    );
    let r = search(&mut svc, &pid, json!({ "query": "张三", "mode": "semantic" }));
    assert_eq!(hit_ids(&r).len(), 0, "语义通道只吃 title/scope/subject/client（人物字段不入文案）");
}
