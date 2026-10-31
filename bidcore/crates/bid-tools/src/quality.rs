//! quality.rs —— #68 阶段 2：生成质量验证（覆盖度检查 + 报告落盘）。
//!
//! 分层：
//! - **单章覆盖度**（[`check_section`]）：已起草章节的自动检查——
//!   ① REQ 覆盖（章节响应面内每条要求是否被正文引用回应——issue §2.1）；
//!   ② EV 引用核对（援引的证据是否在矩阵绑定集内——幻觉引用单列）；
//!   ③ 字数带宽（`target_chars` ±20%——Lead 既有验收线口径）；
//!   ④ 无锚承诺（复用 draft::count_unanchored_claims）；
//! - **项目报告**（[`build_quality_report`]）：全叶章节逐章检查 + 汇总
//!   （FAILED / PENDING / 未覆盖 REQ 总数 / 带外章数 / needs_human 清单
//!   ——issue §2.2「最终报告：哪些章节需要人工处理」）；
//! - **落盘**（[`write_report`]）：`{workspace}/artifacts/quality-report.json`
//!   （serde JSON——验收线「报告落盘」）；失败章错误明细从 S-2 的
//!   `failed.json` 边车并入（assembler 在场时）。
//!
//! 纪律：只读诊断（D2 同语义——报告是导数不是源数据），不改稿不落库
//! 不推状态。
//!
//! # B3 质量检查升级（issue #97——针对性/实质性/重复度）
//!
//! 既有四面（覆盖/引用/字数/无锚）是**形式合规**面；B3 补**内容质量**面：
//! - **针对性**（[`assess_specificity`]）：是否照抄模板——项目名/招标人名
//!   专名锚 + 具体参数（数字+单位）密度 + 套话占比；
//! - **实质性**（[`assess_substance`]）：是否空话——空话句占比（句含套话
//!   短语且无具体锚）+ 可验证陈述数；
//! - **重复度**（[`detect_duplication`]）：跨章节整段复制——句级 Jaccard
//!   + 包含率两形态。
//!
//! 三者均为纯内容函数（零 IO、确定性——同输入同输出），报告层接线见
//! `check_section_with`/`build_quality_report_with`（与 B2 自评联动）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::scoring::{SectionScore, SectionScorer};
use bid_domain::DomainStore;
use serde::{Deserialize, Serialize};

use crate::assembly::ChapterAssembler;
use crate::draft::{count_unanchored_claims, scan_citations};

/// 单章质量面（报告的逐章条目）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionQuality {
    pub section_id: String,
    pub title: String,
    /// 域状态字面量（DRAFTED / FAILED / PENDING / …）。
    pub status: String,
    /// 正文长度（chars().count() 中文口径；未起草 = 0）。
    pub chars: usize,
    pub target_chars: u32,
    /// 字数带宽（±20%）：Some(true) 带内 / Some(false) 带外 / None 不评
    /// （未起草或 target=0——诚实面：无目标不判带）。
    pub within_band: Option<bool>,
    /// 正文已回应的 REQ。
    pub covered_reqs: Vec<String>,
    /// 响应面内**未被回应**的 REQ（issue §2.1 核心清单）。
    pub uncovered_reqs: Vec<String>,
    /// 章节绑定面外的 REQ（幽灵——抽取漂移，如实报不猜）。
    pub ghost_reqs: Vec<String>,
    /// 正文援引且在绑定集内的 EV。
    pub cited_evs: Vec<String>,
    /// 绑定集内未被援引的 EV（信息面——非全部证据都必须引用）。
    pub uncited_evs: Vec<String>,
    /// 正文引用了但不在已知集的编号（幻觉面——draft 时已剔除，>0 即异常）。
    pub unknown_citations: Vec<String>,
    pub unanchored_claims: usize,
    /// 章节绑定的 REQ 全集（#86 摩擦 #4——章节→REQ 归属的正式查询口：
    /// 无需从矩阵反推，check_bid_quality 直接给出）。与 covered/uncovered/
    /// ghost 的并集一致（三者在响应面内互斥分桶）。仅叶章节有效。
    pub requirement_ids: Vec<String>,
    /// FAILED 章的错误明细（failed.json 边车在场时）。
    pub error: Option<String>,
    /// 需人工处理的判定（FAILED / 有未覆盖 REQ / 幻觉引用残留 / 带外字数
    /// / 严重跨章重复）。
    pub needs_human: bool,
    // ---- B3 联动面（issue #97 §与自评的联动：一个章节的检查报告里同时
    //      有响应性/质量/得分；serde default 兼容旧报告 JSON）----
    /// 针对性（B3 维度 1——ctx 在场且已起草才评；None = 不评）。
    #[serde(default)]
    pub specificity: Option<SpecificityScore>,
    /// 实质性（B3 维度 2——同上）。
    #[serde(default)]
    pub substance: Option<SubstanceScore>,
    /// B2 自评得分（ctx 在场且已起草才评——"能得几分"面）。
    #[serde(default)]
    pub score: Option<SectionScore>,
    /// 与其他章节的重复对端（章节 id + 相似度——报告级重复检测的章节视图；
    /// 空表 = 无重复对端）。
    #[serde(default)]
    pub dup_with: Vec<(String, f64)>,
}

/// 项目级质量报告（artifacts/quality-report.json 的落盘形态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityReport {
    pub project_id: String,
    pub generated_at: u64,
    pub sections: Vec<SectionQuality>,
    pub summary: QualitySummary,
    /// 跨章重复检测（B3 维度 3——ctx 在场且已起草章 ≥2 才算；None = 未算）。
    #[serde(default)]
    pub duplication: Option<DuplicationReport>,
}

/// 汇总面。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualitySummary {
    pub total: usize,
    pub drafted: usize,
    pub failed: usize,
    pub pending: usize,
    /// 全项目未覆盖 REQ 计数（Σ 逐章 uncovered）。
    pub uncovered_req_total: usize,
    /// 字数带外章数。
    pub out_of_band: usize,
    pub unanchored_total: usize,
    /// 需人工处理清单（issue §2.2「最终报告」）。
    pub needs_human: Vec<String>,
    // ---- B3 联动汇总面（serde default 兼容旧报告 JSON）----
    /// 针对性均值（已评章节；None = 无可评面）。
    #[serde(default)]
    pub avg_specificity: Option<f64>,
    /// 实质性均值（同上）。
    #[serde(default)]
    pub avg_substance: Option<f64>,
    /// 自评得分率均值（estimated/max，max>0 的已评章节——B2 联动面）。
    #[serde(default)]
    pub avg_score_ratio: Option<f64>,
    /// 严重重复对数（Jaccard > 0.8 ∪ 包含率 > 0.9）。
    #[serde(default)]
    pub severe_dup_pairs: usize,
}

// ---------------------------------------------------------------- 单章检查

/// 质量检查扩展上下文（B3 三维度 + B2 自评的接线面）。
///
/// `None`（旧调用面）= 只做形式合规检查（覆盖/引用/字数/无锚）；
/// `Some` = 追加内容质量面（针对性/实质性/自评得分）。
pub struct QualityContext<'a> {
    /// B2 章节自评器（bid-domain 冻结面——只调用不改）。
    pub scorer: &'a SectionScorer,
    /// B3 针对性的专名锚源。
    pub project: &'a ProjectContext,
}

/// 单章覆盖度检查（旧调用面——等价 `check_section_with(store, node, None)`）。
pub fn check_section(store: &DomainStore, node: &OutlineNode) -> SectionQuality {
    check_section_with(store, node, None)
}

/// 单章检查（扩展面——issue #97 §与自评的联动：一个章节的检查报告里
/// 同时有响应性/质量/得分）。
///
/// - 未起草章不评质量/得分（None——没写字不判内容，报告不伤噪）；
/// - `dup_with` 由报告层（[`build_quality_report_with`]）四填——单章无
///   跨章视野。
pub fn check_section_with(
    store: &DomainStore,
    node: &OutlineNode,
    ctx: Option<&QualityContext<'_>>,
) -> SectionQuality {
    let content = node.content.as_deref().unwrap_or("");
    let cited: Vec<String> = scan_citations(content);

    // ---- REQ 面：响应面（node.requirement_ids）逐条核对
    let known_req: HashSet<&str> =
        store.requirements().iter().map(|r| r.id.as_str()).collect();
    let mut covered_reqs = Vec::new();
    let mut uncovered_reqs = Vec::new();
    let mut ghost_reqs = Vec::new();
    let has_content = !content.trim().is_empty();
    for rid in &node.requirement_ids {
        if !known_req.contains(&rid[..]) {
            ghost_reqs.push(rid.clone()); // 抽取漂移——不是质量问题是数据面问题
            continue;
        }
        // 未起草章不算未覆盖（没有正文就没有「未回应」可言——pending 计数
        // 已单独承载「还没写到」，needs_human 不收批次进行中的噪声）
        if !has_content {
            continue;
        }
        if cited.iter().any(|c| c == rid) {
            covered_reqs.push(rid.clone());
        } else {
            uncovered_reqs.push(rid.clone());
        }
    }

    // ---- EV 面：绑定集（与 draft::bound_evidence_ids 同口径——本章节 REQ ↔ EV）
    let req_set: HashSet<&str> = node.requirement_ids.iter().map(|s| &s[..]).collect();
    let mut known_ev: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for b in store.evidence_bindings() {
        if req_set.contains(&b.requirement_id[..]) && seen.insert(b.evidence_id.clone()) {
            known_ev.push(b.evidence_id.clone());
        }
    }
    let known_all: HashSet<&str> = known_req.iter().copied()
        .chain(known_ev.iter().map(|s| &s[..])).collect();
    let (cited_evs, unknown_citations): (Vec<String>, Vec<String>) = {
        let mut evs = Vec::new();
        let mut unk = Vec::new();
        for c in &cited {
            if c.starts_with("EV-") {
                evs.push(c.clone());
            }
            if !known_all.contains(&c[..]) {
                unk.push(c.clone());
            }
        }
        (evs, unk)
    };
    let uncited_evs: Vec<String> = known_ev.iter()
        .filter(|e| !cited_evs.contains(e)).cloned().collect();

    // ---- 字数带宽（±20%——target=0 或未起草不评）
    let chars = content.chars().count();
    let within_band = if node.target_chars == 0 || content.trim().is_empty() {
        None
    } else {
        let t = f64::from(node.target_chars);
        Some((chars as f64) >= t * 0.80 && (chars as f64) <= t * 1.20)
    };

    let unanchored = if has_content { count_unanchored_claims(content) } else { 0 };
    let needs_human = node.status == SectionStatus::Failed
        || (has_content && (!uncovered_reqs.is_empty()
            || !unknown_citations.is_empty()
            || within_band == Some(false)));

    // ---- B3 联动面：ctx 在场且已起草才评（质量/得分）
    let (specificity, substance, score) = if let Some(c) = ctx {
        if has_content {
            (
                Some(assess_specificity(content, c.project)),
                Some(assess_substance(content)),
                Some(c.scorer.score_section(&node.id, content)),
            )
        } else {
            (None, None, None)
        }
    } else {
        (None, None, None)
    };

    SectionQuality {
        section_id: node.id.clone(),
        title: node.title.clone(),
        status: node.status.as_str().to_string(),
        chars,
        target_chars: node.target_chars,
        within_band,
        covered_reqs,
        uncovered_reqs,
        ghost_reqs,
        cited_evs,
        uncited_evs,
        unknown_citations,
        unanchored_claims: unanchored,
        requirement_ids: node.requirement_ids.clone(),
        error: None, // 调用方（build_quality_report）从边车并入
        needs_human,
        specificity,
        substance,
        score,
        dup_with: Vec::new(), // 报告层回填（单章无跨章视野）
    }
}

// ---------------------------------------------------------------- 项目报告

/// 全项目质量报告（旧调用面——等价 `build_quality_report_with(store, asm, None)`）。
pub fn build_quality_report(
    store: &DomainStore,
    asm: Option<&ChapterAssembler>,
) -> QualityReport {
    build_quality_report_with(store, asm, None)
}

/// 全项目质量报告（扩展面——B3 三维度 + B2 自评联动）。
///
/// 联动语义（issue #97 §与自评的联动）：
/// - 逐章 `check_section_with`：响应性 + 质量 + 得分三面并存；
/// - 跨章重复：已起草叶两两比对 → 报告级 [`DuplicationReport`] + 逐章
///   `dup_with` 回填（对端 id + 两口径最大相似度）；
/// - **needs_human 扩展**：严重重复（Jaccard > 0.8 ∪ 包含率 > 0.9）纳入
///   ——依据：严重重复是**结构性缺陷**（两章整段互拷，删哪段改哪段须
///   人工裁决）；而低针对性/低实质性是 B4 改写循环的**输入**（自动化可
///   救），不设人工闸门——两类问题的处置路径不同；
/// - 汇总：avg_specificity / avg_substance / avg_score_ratio（已评章节
///   均值——`None` = 无可评面，不伪装 0）+ severe_dup_pairs。
pub fn build_quality_report_with(
    store: &DomainStore,
    asm: Option<&ChapterAssembler>,
    ctx: Option<&QualityContext<'_>>,
) -> QualityReport {
    let failed_map = asm.map(|a| a.failed_records()).unwrap_or_default();
    let parents: HashSet<&str> =
        store.sections().iter().filter_map(|n| n.parent_id.as_deref()).collect();
    let mut leaves: Vec<&OutlineNode> = store.sections().iter()
        .filter(|n| !parents.contains(&n.id[..])).collect();
    leaves.sort_by_key(|n| n.order);

    let mut sections = Vec::with_capacity(leaves.len());
    let mut drafted = 0usize;
    let mut failed = 0usize;
    let mut pending = 0usize;
    let mut uncovered_req_total = 0usize;
    let mut out_of_band = 0usize;
    let mut unanchored_total = 0usize;
    let mut needs_human = Vec::new();
    let total = leaves.len();
    for node in &leaves {
        let mut q = check_section_with(store, node, ctx);
        if let Some(rec) = failed_map.get(&node.id) {
            q.error = Some(rec.error.clone());
        }
        match node.status {
            SectionStatus::Drafted | SectionStatus::Validating
            | SectionStatus::Revising | SectionStatus::Accepted => drafted += 1,
            SectionStatus::Failed => failed += 1,
            _ => pending += 1,
        }
        uncovered_req_total += q.uncovered_reqs.len();
        if q.within_band == Some(false) {
            out_of_band += 1;
        }
        unanchored_total += q.unanchored_claims;
        if q.needs_human {
            needs_human.push(node.id.clone());
        }
        sections.push(q);
    }

    // ---- B3 维度 3：跨章重复（ctx 在场且已起草叶 ≥2 才有比对面）
    let drafted_pairs: Vec<(String, String)> = leaves
        .iter()
        .filter(|n| n.content.as_deref().is_some_and(|c| !c.trim().is_empty()))
        .map(|n| (n.id.clone(), n.content.clone().unwrap_or_default()))
        .collect();
    let duplication = if ctx.is_some() && drafted_pairs.len() >= 2 {
        Some(detect_duplication(&drafted_pairs))
    } else {
        None
    };

    // 逐章回填 dup_with + 严重重复对的 needs_human
    let severe_count = duplication.as_ref().map_or(0, |d| d.severe);
    if let Some(d) = &duplication {
        // 对端 → 最大相似度（Jaccard/包含率两口径取大——单一呈现面）
        let mut partners: HashMap<&str, HashMap<&str, f64>> = HashMap::new();
        fn bump<'a>(
            a: &'a str,
            b: &'a str,
            sim: f64,
            partners: &mut HashMap<&'a str, HashMap<&'a str, f64>>,
        ) {
            partners
                .entry(a)
                .or_default()
                .entry(b)
                .and_modify(|v| {
                    if sim > *v {
                        *v = sim;
                    }
                })
                .or_insert(sim);
        }
        for (a, b, jac) in &d.pairs {
            bump(a, b, *jac, &mut partners);
            bump(b, a, *jac, &mut partners);
        }
        for (a, b, cont) in &d.contained {
            bump(a, b, *cont, &mut partners);
            bump(b, a, *cont, &mut partners);
        }
        // 严重对身份（severe 计数之外的可操作面）：Jaccard > 0.8 或包含率 > 0.9
        let mut severe_ids: HashSet<&str> = HashSet::new();
        for (a, b, j) in &d.pairs {
            if *j > DUP_SEVERE_JACCARD {
                severe_ids.insert(a);
                severe_ids.insert(b);
            }
        }
        for (a, b, c) in &d.contained {
            if *c > DUP_SEVERE_CONTAINMENT {
                severe_ids.insert(a);
                severe_ids.insert(b);
            }
        }
        for q in &mut sections {
            if let Some(p) = partners.get(q.section_id.as_str()) {
                let mut v: Vec<(String, f64)> = p.iter()
                    .map(|(id, sim)| ((*id).to_string(), *sim)).collect();
                v.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
                q.dup_with = v;
            }
            if severe_ids.contains(q.section_id.as_str()) {
                q.needs_human = true;
                if !needs_human.contains(&q.section_id) {
                    needs_human.push(q.section_id.clone());
                }
            }
        }
    }

    // ---- B3/B2 联动汇总（已评章节均值——None = 无可评面）
    let round4 = |v: f64| (v * 10_000.0).round() / 10_000.0;
    let avg_specificity = {
        let v: Vec<f64> = sections.iter().filter_map(|q| q.specificity.as_ref())
            .map(|s| s.score).collect();
        (!v.is_empty()).then(|| round4(v.iter().sum::<f64>() / v.len() as f64))
    };
    let avg_substance = {
        let v: Vec<f64> = sections.iter().filter_map(|q| q.substance.as_ref())
            .map(|s| s.score).collect();
        (!v.is_empty()).then(|| round4(v.iter().sum::<f64>() / v.len() as f64))
    };
    let avg_score_ratio = {
        let v: Vec<f64> = sections.iter().filter_map(|q| q.score.as_ref())
            .filter(|s| s.max > 0.0)
            .map(|s| s.estimated / s.max).collect();
        (!v.is_empty()).then(|| round4(v.iter().sum::<f64>() / v.len() as f64))
    };

    QualityReport {
        project_id: store.project().map(|p| p.id.clone()).unwrap_or_default(),
        generated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        sections,
        duplication,
        summary: QualitySummary {
            total,
            drafted,
            failed,
            pending,
            uncovered_req_total,
            out_of_band,
            unanchored_total,
            needs_human,
            avg_specificity,
            avg_substance,
            avg_score_ratio,
            severe_dup_pairs: severe_count,
        },
    }
}

/// 报告落盘：`{ws}/artifacts/quality-report.json`（tmp + rename 原子写）。
///
/// 返回落盘路径（调用方进 MCP out 的 artifact 面）。
pub fn write_report(report: &QualityReport, ws: &Path) -> Result<PathBuf, String> {
    let dir = ws.join("artifacts");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建 artifacts 目录失败: {e}"))?;
    let path = dir.join("quality-report.json");
    let tmp = dir.join("quality-report.json.tmp");
    let body = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).map_err(|e| format!("写报告失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("报告原子替换失败: {e}"))?;
    Ok(path)
}

// ---------------------------------------------------------------- B3 共用底表

/// 空话短语表（B3 针对性「套话占比」与实质性「空话句」的共用底表）。
///
/// 选表依据（issue #97 §六-3「用真实标书的套话，不要凭感觉」——逐条标注
/// 出处，可复核）：
/// - `issue` = issue #97 Lead 初版表原样保留（9 条）；
/// - `实测` = 本仓真实语料 grep 实测出现（`real-samples/北京水利施工示范
///   文本2025-LO.docx` + `real-corpus/telecom/parse-input/` 17 件——频次见
///   PR 描述，如「郑重承诺」在 unicom/songjiang/shanxi 三件合计 16 处）；
/// - `行业` = 施工组织设计/服务承诺章的通行句式（本仓语料为招标侧、无
///   投标侧正文锚点——如实标注低置信；检测面要求「句内无具体锚」才计
///   空话，短语本身不单独定罪，假阳性被锚规则压住）。
///
/// 全表条目 ≥4 字（「确保」「圆满」这类单词过于通用，不入表——有锚规则
/// 也压不住其触发面）。
pub const EMPTY_PHRASES: &[&str] = &[
    // ---- issue #97 初版（Lead 给定）----
    "我公司将秉承",   // issue
    "本着诚信原则",   // issue
    "严格按照要求",   // issue（实测：unicom 主文件 1 处）
    "确保万无一失",   // issue
    "全力以赴",       // issue
    "高度重视",       // issue
    "建立健全",       // issue（实测：示范文本 1 / 电信 2 处）
    "切实可行",       // issue（实测：示范文本 1 / 灵枢智关 1 处）
    "严格执行",       // issue（实测：示范文本 2 / 电信 7 处）
    // ---- 本仓真实语料实测 ----
    "郑重承诺",       // 实测（电信 17 件中 3 件合计 16 处）
    "严格按照",       // 实测（示范文本 3 处）
    // ---- 行业通行句式（低置信——句级无锚才计）----
    "我司将秉承",     // 行业（与 issue「我公司将秉承」同族的主体变体）
    "我司高度重视",   // 行业
    "我公司高度重视", // 行业
    "精心组织",       // 行业
    "科学管理",       // 行业
    "优质高效",       // 行业
    "圆满完成",       // 行业
    "圆满履约",       // 行业
    "自始至终",       // 行业
    "一流的服务",     // 行业
    "一流的管理",     // 行业
    "经验丰富",       // 行业
    "雄厚的技术",     // 行业
    "深刻理解",       // 行业
    "充分理解",       // 行业
    "积极响应",       // 行业
    "先进理念",       // 行业
    "科学合理",       // 行业
];

/// 具体参数单位表（「数字+单位」形态的计量面）。
///
/// B2 `scoring.rs::SPECIFIC_UNITS` 的消费侧扩展（bid-domain 冻结不改）：
/// 追加标书高频工程/商务单位（吨/张/座/处/米/立方/kW 等），其余条目与
/// B2 一致——B3 的参数计数口径 ⊇ B2 具体性门槛口径，两侧判定不矛盾。
const PARAM_UNITS: &[&str] = &[
    // 与 B2 SPECIFIC_UNITS 一致的基底
    "万元", "亿", "台", "套", "件", "人", "年", "日历天", "%", "Gb", "GB",
    "MB", "小时", "分钟", "天", "平方米", "公里", "家",
    // B3 扩展（工程/商务高频）
    "平方公里", "立方", "千米", "吨", "张", "座", "处", "名", "万", "元",
    "米", "平方", "TB", "Tb", "GHz", "MHz", "kW", "个", "台班",
];

/// 句子切分（。；！？与 ASCII 句读 + 换行——B3 三维度共用的最小单元）。
///
/// 纯内容函数：切分保序，空段/纯空白段丢弃。
fn split_sentences(content: &str) -> Vec<&str> {
    content
        .split(['。', '；', '！', '？', '!', '?', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// 「数字+单位」计数（与 B2 `count_number_unit` 同构——bid-domain 私有面
/// 冻结，消费侧重实现；数字序列（含小数点）后紧跟单位即计 1）。
fn count_number_unit(content: &str) -> usize {
    let chars: Vec<char> = content.chars().collect();
    let mut n = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let rest: String = chars[i..].iter().take(4).collect();
            if PARAM_UNITS.iter().any(|u| rest.starts_with(u)) {
                n += 1;
            }
        } else {
            i += 1;
        }
    }
    n
}

/// 标准编号形态（可验证陈述的形态之二）。
///
/// 覆盖真实标书高频形态：`ISO9001` / `GB/T19001` / `GB50205` / `JGJ46-2005`
/// / `DB11/T` / `CJJ/T77`——前缀字母 + 可选 `/T` 分段 + 数字。
/// `OnceLock` 缓存编译（regex::Regex Send+Sync——首次调用后零重复编译）。
fn standard_id_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        // 不用 \b 词边界：rust regex 的 \w 含 CJK，「符合GB50205」这种
        // 无空格紧邻形态会因「中文|字母均为词字符」而无边界可断——漏检
        // 常见形态；裸前缀匹配的误报面（RGB565 之类）在标书语料可忽略。
        regex::Regex::new(
            r"(?:ISO|GB|JGJ|CJJ|DB|CQC|OHSAS|ITU|IEC|IEEE)(?:/[A-Z0-9]{1,3})?[/-]?[0-9]",
        )
        .expect("标准编号正则是常量——编译不会失败")
    })
}

/// 句内具体锚：数字+单位 / 标准编号（regex 形态）/ [REQ-*][EV-*] 引用。
///
/// 「有锚的承诺句不是空话」——政府示范文本投标函的「我方承诺投标有效期
/// 为自投标截止日起 90 天」有数字锚，是功能句不是空话（锚规则压假阳性）。
fn has_concrete_anchor(sentence: &str) -> bool {
    count_number_unit(sentence) > 0
        || standard_id_regex().is_match(sentence)
        || sentence.contains("REQ-")
        || sentence.contains("EV-")
}

/// 空话句判定：句含空话短语 **且** 句内无具体锚。
fn is_empty_sentence(sentence: &str) -> bool {
    if !EMPTY_PHRASES.iter().any(|p| sentence.contains(p)) {
        return false;
    }
    !has_concrete_anchor(sentence)
}

/// 套话占比（空话句字符数 / 全句字符数——无句面 0.0）。
///
/// 字符加权（非句数占比）：整段复制的套话句通常更长，字符口径对
/// 「这章有多少篇幅在说空话」的直觉更诚实。
fn boilerplate_ratio(content: &str) -> f64 {
    let sentences = split_sentences(content);
    let total: usize = sentences.iter().map(|s| s.chars().count()).sum();
    if total == 0 {
        return 0.0;
    }
    let empty: usize = sentences
        .iter()
        .filter(|s| is_empty_sentence(s))
        .map(|s| s.chars().count())
        .sum();
    (empty as f64 / total as f64 * 10_000.0).round() / 10_000.0
}

// ---------------------------------------------------------------- B3 维度 1：针对性

/// 项目上下文（针对性判定的专名锚词源——issue #97 B3 维度 1）。
///
/// 诚实边界：`BidProject` 无「招标人名称」字段（bid-domain 是 B2 成果的
/// 冻结面——只调用不改），[`ProjectContext::from_store`] 的 `tenderer_name`
/// 诚实留空，由上层（MCP 壳/用户指令）注入；留空时 [`assess_specificity`]
/// 按「可用权重归一」处理（不罚未知面）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProjectContext {
    /// 项目名称（如「塔里木水库除险加固工程」——BidProject.name）。
    pub project_name: String,
    /// 招标人名称（如「某水库管理处」——store 无此字段，调用方注入）。
    pub tenderer_name: String,
    /// 其他专名锚（项目编号/标段号——from_store 取 BidProject.code）。
    pub extra_terms: Vec<String>,
}

impl ProjectContext {
    /// 从 store 构造（项目名 + 项目编号——store 里真实存在的专名面；
    /// 招标人名 store 无源，诚实留空）。
    pub fn from_store(store: &DomainStore) -> Self {
        match store.project() {
            Ok(p) => ProjectContext {
                project_name: p.name.clone(),
                tenderer_name: String::new(),
                extra_terms: if p.code.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![p.code.clone()]
                },
            },
            Err(_) => ProjectContext::default(),
        }
    }

    /// B2 `SectionScorer` 的项目专名锚词（`project_terms` 构造参数——
    /// 项目名/招标人/编号全量非空面；B3 与 B2 用同一专名源，两判定的
    /// 锚词口径一致）。
    pub fn scorer_terms(&self) -> Vec<String> {
        let mut v = vec![self.project_name.clone(), self.tenderer_name.clone()];
        v.extend(self.extra_terms.iter().cloned());
        v.into_iter().filter(|t| !t.trim().is_empty()).collect()
    }
}

/// 针对性评分（0.0-1.0——是否照抄模板的量化面）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecificityScore {
    /// 是否含项目名。
    pub has_project_name: bool,
    /// 是否含招标人名称。
    pub has_tenderer_name: bool,
    /// 具体参数出现次数（数字+单位）。
    pub concrete_params: usize,
    /// 通用套话占比（空话句字符 / 全句字符——0-1）。
    pub boilerplate_ratio: f64,
    /// 总分 0-1。
    pub score: f64,
}

/// 针对性评估（纯内容函数——同输入同输出）。
///
/// 权重（issue #97 初版，两处调整的依据如下）：
/// - 含项目名 +0.3 / 含招标人名 +0.2 / 具体参数 +0.3 / 套话占比 <20% +0.2；
/// - **调整①（可用权重归一）**：招标人名（或项目名）未知时，其权重退出
///   分母——否则任何章节全场封顶 0.8，验收线 0.7 被压到及格边缘，判别力
///   失真。归一后「未知面」既不奖励也不惩罚；
/// - **调整②（参数阶梯）**：参数 >5 处满权 0.3、3~5 处半权 0.15、<3 处
///   零——初版二元开关（>5 才给分）在 4~6 个参数的真实章节上抖动（差 1
///   个参数 ±0.3 跳变），阶梯平滑且保两端判别力（0 参数 vs 6+ 参数仍全距）；
/// - 空内容（无句面）→ 全项归零（不白给「无套话」的 0.2——没写字不等于
///   没抄模板，诚实 0 分）。
pub fn assess_specificity(section: &str, project: &ProjectContext) -> SpecificityScore {
    let has_project_name = !project.project_name.trim().is_empty()
        && section.contains(project.project_name.trim());
    let has_tenderer_name = !project.tenderer_name.trim().is_empty()
        && section.contains(project.tenderer_name.trim());
    let concrete_params = count_number_unit(section);
    let boilerplate = boilerplate_ratio(section);

    let mut earned = 0.0f64;
    let mut applicable = 0.0f64;
    if !project.project_name.trim().is_empty() {
        applicable += 0.3;
        if has_project_name {
            earned += 0.3;
        }
    }
    if !project.tenderer_name.trim().is_empty() {
        applicable += 0.2;
        if has_tenderer_name {
            earned += 0.2;
        }
    }
    applicable += 0.3;
    earned += if concrete_params > 5 {
        0.3
    } else if concrete_params >= 3 {
        0.15
    } else {
        0.0
    };
    applicable += 0.2;
    let has_sentences = !split_sentences(section).is_empty();
    if has_sentences && boilerplate < 0.20 {
        earned += 0.2;
    }
    // 招标人名/项目名未知 → 权重退出分母（调整①）；空内容 → 诚实 0 分
    let score = if !has_sentences {
        0.0
    } else if applicable > 0.0 {
        (earned / applicable * 1000.0).round() / 1000.0
    } else {
        0.0
    };
    SpecificityScore {
        has_project_name,
        has_tenderer_name,
        concrete_params,
        boilerplate_ratio: boilerplate,
        score,
    }
}

// ---------------------------------------------------------------- B3 维度 2：实质性

/// 可验证陈述的满分线（[`SubstanceScore::score`] 的归一基数）。
///
/// 依据：B2 `WORDS_PER_POINT = 50 字/分` × 30 分满项 = 1500 字的典型章节
/// 约含 15~20 句，8 句 ≈ 过半——「多数句子可验证」的量化线（低于 8 线性
/// 给分，高于 8 满分——短章不罚、长章不虚涨）。
pub const VERIFIABLE_CLAIMS_FULL: usize = 8;

/// 实质性评分（是否空话——信息密度的量化面）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubstanceScore {
    /// 空话句占比（字符加权——句含套话短语且无具体锚；与
    /// [`SpecificityScore::boilerplate_ratio`] 同一底层度量：针对性看
    /// 「模板味」，实质性看「信息密度」）。
    pub empty_sentence_ratio: f64,
    /// 可验证陈述数（含数字+单位 / 标准编号 / REQ-EV 引用的句子数）。
    pub verifiable_claims: usize,
    /// 总分 0-1。
    pub score: f64,
}

/// 实质性评估（纯内容函数——同输入同输出）。
///
/// 计分（issue #97 未给公式，自定并给依据）：
/// ```text
/// score = (1 - 空话句占比) × 0.6 + min(可验证陈述数 / 8, 1) × 0.4
/// ```
/// - **负面面 0.6 > 正面面 0.4**：一句空话对评委的伤害大于一句实话的
///   贡献（空话是扣分证据，可验证是加分证据）——权重不对称是评标直觉
///   的量化；
/// - 空内容（无句面）→ 诚实 0 分（不因「没说空话」白得 0.6）；
/// - 验收锚定：纯空话章节（占比→1、陈述→0）score→0 < 0.4 ✅；好章节
///   （占比 ≈0、陈述 ≥8）→ ≈1.0。
pub fn assess_substance(section: &str) -> SubstanceScore {
    let sentences = split_sentences(section);
    let empty_ratio = boilerplate_ratio(section);
    let verifiable_claims = sentences
        .iter()
        .filter(|s| has_concrete_anchor(s))
        .count();
    let score = if sentences.is_empty() {
        0.0
    } else {
        let density = (1.0 - empty_ratio) * 0.6;
        let claims = (verifiable_claims as f64 / VERIFIABLE_CLAIMS_FULL as f64).min(1.0) * 0.4;
        ((density + claims) * 1000.0).round() / 1000.0
    };
    SubstanceScore {
        empty_sentence_ratio: empty_ratio,
        verifiable_claims,
        score,
    }
}

// ---------------------------------------------------------------- B3 维度 3：重复度

/// 重复对的报告收录下限（Jaccard/包含率低于此的对不进报告——无行动价值）。
pub const DUP_REPORT_FLOOR: f64 = 0.3;
/// 严重重复线（句级 Jaccard > 此值——issue #97 初版 0.8，原样保留）。
pub const DUP_SEVERE_JACCARD: f64 = 0.8;
/// 整段复制线（包含率 > 此值 = 严重）。
///
/// Jaccard 的盲区：B 章 20 句全部出自 A 章 40 句（B 是 A 的子集）时
/// Jaccard = 20/40 = 0.5（被长度差稀释，低于 0.8 漏报），包含率 =
/// 20/20 = 1.0 如实暴露「B 章整段抄自 A 章」。两形态并报（依据见
/// [`detect_duplication`]）。
pub const DUP_SEVERE_CONTAINMENT: f64 = 0.9;
/// 句子入集最短长度（字符）——更短的是结构性短语（标题/编号行），不是
/// 内容句；两章共享短短语不构成「重复」。
const DUP_MIN_SENTENCE_CHARS: usize = 6;

/// 跨章节重复检测报告（issue #97 B3 维度 3）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DuplicationReport {
    /// 章节两两之间的相似度（句级 Jaccard——降序，只收 ≥ 0.3 的对）。
    pub pairs: Vec<(String, String, f64)>,
    /// 整段复制面（containment = |A∩B| / min(|A|,|B|)——降序，只收
    /// ≥ 0.3 的对）：B 章内容大面积出自 A 章的形态，Jaccard 因两章长度
    /// 差被稀释，此面补盲。
    pub contained: Vec<(String, String, f64)>,
    /// 严重重复对数（Jaccard > 0.8 ∪ 包含率 > 0.9——issue 验收「故意
    /// 复制一段，检测出 >0.8」的判定面）。
    pub severe: usize,
}

/// 枚举标记正则（整段复制后**重新编号**是最常见的改动——剥前导标记让
/// 「1. 我方承诺…」与「（一）我方承诺…」归一为同一句）。
fn enumeration_marker_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(concat!(
            r"^[\s]*",
            r"(?:\d{1,3}[.、）)]|[（(]\d{1,3}[)）]|[（(][一二三四五六七八九十]{1,3}[)）]",
            r"|[①-⑳]|[一二三四五六七八九十]{1,3}[、.．])",
            r"[\s]*",
        ))
        .expect("枚举标记正则是常量——编译不会失败")
    })
}

/// 句子入集形态：剥前导枚举标记 + 去全部空白（排版重排不影响内容句的
/// 同一性——「混凝土 1.2 万立方米」与「混凝土1.2万立方米」是同一句）。
/// 过短（<6 字）返回 None（结构性短语不入集）。
fn normalize_sentence(sentence: &str) -> Option<String> {
    let stripped = enumeration_marker_regex().replace(sentence, "");
    let compact: String = stripped.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.chars().count() < DUP_MIN_SENTENCE_CHARS {
        None
    } else {
        Some(compact)
    }
}

/// 章节的句子集合（归一化 + 章内去重——句集语义，重复检测的计量单位）。
fn sentence_set(content: &str) -> HashSet<String> {
    split_sentences(content)
        .iter()
        .filter_map(|s| normalize_sentence(s))
        .collect()
}

/// 重复检测（纯内容函数——章节 (id, 正文) 序列的两两比对）。
///
/// 算法（issue #97 §B3 维度 3「不要用复杂算法」——句级集合比对）：
/// 1. 每章切句 → 归一化（剥枚举标记/去空白/滤短句）→ 集合；
/// 2. 两两 Jaccard = |A∩B| / |A∪B|（issue 指定口径）；
/// 3. 附加包含率 containment = |A∩B| / min(|A|,|B|)——标书场景「整段
///    复制」的常见形态是**子集复制**（B 章的每句都来自 A 章，但 A 章更长），
///    纯 Jaccard 会被长度差稀释漏报（见 [`DUP_SEVERE_CONTAINMENT`] 的
///    数值例），两形态并报才是对「重复」的诚实覆盖；
/// 4. 严重 = Jaccard > 0.8 ∪ containment > 0.9；报告只收 ≥ 0.3 的对
///    （更低的对没有行动价值——全对收录只会淹没报告）。
pub fn detect_duplication(sections: &[(String, String)]) -> DuplicationReport {
    let sets: Vec<(&str, HashSet<String>)> = sections
        .iter()
        .map(|(id, content)| (id.as_str(), sentence_set(content)))
        .collect();
    let mut pairs: Vec<(String, String, f64)> = Vec::new();
    let mut contained: Vec<(String, String, f64)> = Vec::new();
    let mut severe = 0usize;
    for i in 0..sets.len() {
        for j in (i + 1)..sets.len() {
            let (id_a, a) = &sets[i];
            let (id_b, b) = &sets[j];
            let inter = a.intersection(b).count();
            if inter == 0 {
                continue;
            }
            let union = a.len() + b.len() - inter;
            let jaccard = inter as f64 / union as f64;
            let containment = inter as f64 / a.len().min(b.len()) as f64;
            if jaccard > DUP_SEVERE_JACCARD || containment > DUP_SEVERE_CONTAINMENT {
                severe += 1;
            }
            let round4 = |v: f64| (v * 10_000.0).round() / 10_000.0;
            if jaccard >= DUP_REPORT_FLOOR {
                pairs.push((id_a.to_string(), id_b.to_string(), round4(jaccard)));
            }
            if containment >= DUP_REPORT_FLOOR {
                contained.push((id_a.to_string(), id_b.to_string(), round4(containment)));
            }
        }
    }
    let desc = |x: &(String, String, f64), y: &(String, String, f64)| {
        y.2.partial_cmp(&x.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.0.cmp(&y.0).then_with(|| x.1.cmp(&y.1)))
    };
    pairs.sort_by(desc);
    contained.sort_by(desc);
    DuplicationReport { pairs, contained, severe }
}
