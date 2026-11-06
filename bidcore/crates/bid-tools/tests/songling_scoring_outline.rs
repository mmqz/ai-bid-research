//! songling_scoring_outline —— #252 评分表驱动大纲的验收断言面（第一步：预期结构）。
//!
//! 与 `songling_baseline.rs`（#251）的分工：
//! - #251 锁**抽取质量**（REQ 数量 / 数值可检索 / 评分分值）与大纲粗形态
//!   （「（第N块）」标题数 / 评分关键词出现数——contains 弱断言）；
//! - 本文件锁**大纲结构正确性**（issue #252 验收条件 1-10 的结构面）：
//!   6 个一级评分章节精确对齐 / 20 个 L2 细则可追溯 / 业绩价格独立响应位 /
//!   ScoringMapping 6/6 / 配额分布（评分 ≥60%、行政 ≤25%）/ 章节绑定覆盖。
//!
//! **三态门语义**（与 #251 `gate_count` 同源，本文件扩展双向门）：
//! - 实测 == 固化基线 → `FAIL(固化基线)` 横幅 + 测试通过（#252 未实现的现状锁定）；
//! - 实测达到目标 → `IMPROVED` 横幅 + 要求更新 manifest 并在 PR 说明原因；
//! - 实测比基线更差 → **assert 红灯**（新回归）。
//!
//! 数据真源：`fixtures/songling-232762/manifest.json` 的 `scoring_outline_baseline`
//! 节（本 PR 固化的 2026-11-05 实测值）；预期形态锚：
//! `expected/outline.expected.json`（6 章节 + 20 L2 + 业绩/价格位 + 第七章 16 项格式序）。
//!
//! 一条命令重跑：`cargo test -p bid-tools --test songling_scoring_outline`。

use bid_domain::outline::{is_admin_group, OutlineSpec};
use bid_parse::env_preflight::{skip_if_env_missing, skip_if_fixture_missing, POPPLER_TEXT_TOOLS};
use bid_parse::extract::extract_requirements;
use bid_tools::service::{BidService, ImportSpec};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// 层1 三个测试互斥（与 #251 同因：4G/2核沙箱 PDF 解析内存约束）。
fn engine_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn fixtures() -> PathBuf {
    let p = std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures"));
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn songling() -> PathBuf {
    fixtures().join("songling-232762")
}

fn load_json(rel: &str) -> Value {
    let path = songling().join(rel);
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture JSON 读取失败 {}: {e}", path.display()));
    serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("fixture JSON 解析失败 {}: {e}", path.display()))
}

fn banner(line: &str) {
    eprintln!();
    eprintln!("====================================================================");
    eprintln!("{line}");
    eprintln!("====================================================================");
}

// ---------------------------------------------------------------- 预期形态（expected 数据的 Rust 视图）

/// expected/outline.expected.json 的 6 个一级评分章节（issue #252 验收 1 的真值锚）。
fn expected_chapters() -> Vec<(String, f64, Vec<String>)> {
    let oe = load_json("expected/outline.expected.json");
    oe["chapters"]
        .as_array()
        .expect("expected.chapters")
        .iter()
        .map(|c| {
            let title = c["title"].as_str().unwrap().to_string();
            let score = c["score"].as_f64().unwrap();
            let l2 = c["l2"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["title"].as_str().unwrap().to_string())
                .collect();
            (title, score, l2)
        })
        .collect()
}

// 行政分类（#253 第二段）：改用域层单一真源 `bid_domain::outline::is_admin_group`
// （ADMIN_GROUP_KEYWORDS——配额三轨/门禁分级/本基线测试三处共用，防口径
// 漂移；域层面比旧测试面多「保证金」：投标保证金组 = 程序性提交要求）。
// 口径：行政章节 = 资格/格式/合同/须知/公告等招标文件自述性、程序性内容；
// 技术参数响应**不在**列（双轨硬约束：技术参数进服务方案，算评分主体）。

// ---------------------------------------------------------------- 三态门（#251 gate 的双向扩展）

/// 固化基线读取（u64）。
fn baseline_u64(key: &str) -> u64 {
    load_json("manifest.json")["scoring_outline_baseline"][key]
        .as_u64()
        .unwrap_or_else(|| panic!("manifest.scoring_outline_baseline.{key} 缺失"))
}

/// 固化基线读取（f64，千分位比例）。
fn baseline_f64(key: &str) -> f64 {
    load_json("manifest.json")["scoring_outline_baseline"][key]
        .as_f64()
        .unwrap_or_else(|| panic!("manifest.scoring_outline_baseline.{key} 缺失"))
}

/// 计数门（越大越好）——#251 gate_count 同款语义。
fn gate_count(id: &str, label: &str, actual: u64, known: u64, target: u64) -> bool {
    if actual < known {
        panic!(
            "[{id}] {label} 回归：实测 {actual} < 固化基线 {known}（issue #252：不允许比现状更差——这是新回归，不是基线漂移）"
        );
    }
    if actual >= target {
        banner(&format!(
            "IMPROVED [{id}] {label}: {actual} ≥ 目标 {target}——#252 目标已达成。请更新 manifest.scoring_outline_baseline 并在 PR 说明原因"
        ));
        true
    } else {
        banner(&format!(
            "FAIL(固化基线) [{id}] {label}: 实测 {actual}（基线 {known}，目标 ≥{target}）——issue #252 未实现的现状固化，等待评分驱动大纲落地"
        ));
        false
    }
}

/// 比例门（越大越好，千分位整数避免浮点基线漂移）。
fn gate_ratio_high(id: &str, label: &str, actual: f64, known: f64, target: f64) -> bool {
    if actual < known - 1e-9 {
        panic!(
            "[{id}] {label} 回归：实测 {actual:.3} < 固化基线 {known:.3}（新回归——不允许比现状更差）"
        );
    }
    if actual >= target - 1e-9 {
        banner(&format!(
            "IMPROVED [{id}] {label}: {actual:.3} ≥ 目标 {target:.3}——#252 目标已达成。请更新 manifest 并在 PR 说明原因"
        ));
        true
    } else {
        banner(&format!(
            "FAIL(固化基线) [{id}] {label}: 实测 {actual:.3}（基线 {known:.3}，目标 ≥{target:.3}）——issue #252 现状固化"
        ));
        false
    }
}

/// 比例门（越小越好——行政占比类）。
fn gate_ratio_low(id: &str, label: &str, actual: f64, known: f64, target: f64) -> bool {
    if actual > known + 1e-9 {
        panic!(
            "[{id}] {label} 恶化：实测 {actual:.3} > 固化基线 {known:.3}（新回归——不允许比现状更差）"
        );
    }
    if actual <= target + 1e-9 {
        banner(&format!(
            "IMPROVED [{id}] {label}: {actual:.3} ≤ 目标 {target:.3}——#252 目标已达成。请更新 manifest 并在 PR 说明原因"
        ));
        true
    } else {
        banner(&format!(
            "FAIL(固化基线) [{id}] {label}: 实测 {actual:.3}（基线 {known:.3}，目标 ≤{target:.3}）——issue #252 现状固化"
        ));
        false
    }
}

// ---------------------------------------------------------------- 层0：expected 数据交叉一致性（永远跑）

/// 标题的 2 字关键词（bigram）序列（去标点后滑窗——UTF-8 3字节/汉字 → 6字节窗口）。
/// 用于「expected 规范短形 ↔ 评分表原文」的交叉一致性判定：连接词/冗余成分
/// 差异可容（覆盖率口径），整句漂移不可容。
fn bigrams(t: &str) -> Vec<String> {
    let norm: String = t
        .chars()
        .filter(|c| !matches!(c, '、' | '，' | '。' | '（' | '）' | ' '))
        .collect();
    norm.as_bytes()
        .windows(6)
        .step_by(3)
        .map(|w| String::from_utf8_lossy(w).to_string())
        .filter(|s| s.chars().count() == 2)
        .collect()
}

/// outline.expected ↔ scoring_table 的交叉一致性（fixture 内部不漂移——
/// 两个文件是 #252 断言的两半真值锚，互相矛盾则断言面全部失真）。
#[test]
fn layer0_expected_cross_consistency() {
    let chapters = expected_chapters();
    assert_eq!(chapters.len(), 6, "6 个一级评分章节（issue #252 验收 1）");

    let st = load_json("expected/scoring_table.json");
    let items = st["items"].as_array().unwrap();
    let by_id = |id: &str| -> &Value {
        items
            .iter()
            .find(|i| i["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("scoring_table 无 {id}"))
    };

    // ① 6 章节按 scoring_item_id 关联评分表：分值一致 + 名称关键 token 覆盖
    // （expected 章节标题是评分表 name 的规范短形——「项目重点、难点分析及应对措施」
    //  vs name「项目重点、难点分析，应对措施及相关的合理化建议」——不要求全等，
    //  要求标题的 2 字关键词全部在 name/short_name 中出现，防 fixture 漂移失锚）
    for (title, score, _) in &chapters {
        let cid = load_json("expected/outline.expected.json")["chapters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["title"].as_str() == Some(title.as_str()))
            .and_then(|c| c["scoring_item_id"].as_str())
            .unwrap_or_else(|| panic!("{title} 缺 scoring_item_id"))
            .to_string();
        let item = by_id(&cid);
        let name = item["name"].as_str().unwrap_or("");
        let short = item["short_name"].as_str().unwrap_or("");
        let expect_score = item["score"].as_f64().unwrap();
        assert!(
            (score - expect_score).abs() < 1e-9,
            "{title} 分值 {score} ≠ 评分表 {cid} 的 {expect_score}"
        );
        let mut missed: Vec<String> = Vec::new();
        for tk in bigrams(title) {
            if !name.contains(&tk) && !short.contains(&tk) {
                missed.push(tk);
            }
        }
        assert!(
            missed.is_empty(),
            "{title} 的关键词 {missed:?} 在评分表 {cid} 的 name/short_name 中缺失——fixture 交叉漂移"
        );
    }

    // ② 20 个 L2 与评分表 sub_items 一一对应（title ⊆ sub.text）
    // 容差依据：expected L2 是细则 text 的**规范化短形**（#251 金标准复核的
    // 改写——如「备品配件与故障补救措施」vs 细则「备品配件、设备故障后的
    // 补救措施」）：子串直接命中，或 2-gram 关键词覆盖率 ≥ 0.7（连接词/
    // 冗余成分差异可容，整句漂移不可容）。
    let mut l2_total = 0usize;
    let oe = load_json("expected/outline.expected.json");
    for c in oe["chapters"].as_array().unwrap() {
        let cid = c["scoring_item_id"].as_str().unwrap();
        let empty: Vec<Value> = Vec::new();
        let subs = by_id(cid)["sub_items"].as_array().unwrap_or(&empty);
        for l2 in c["l2"].as_array().unwrap() {
            let t = l2["title"].as_str().unwrap();
            let hit = subs.iter().any(|s| {
                let text = s["text"].as_str().unwrap_or("");
                if text.contains(t) || t.contains(text) {
                    return true;
                }
                let grams = bigrams(t);
                if grams.is_empty() {
                    return false;
                }
                let covered = grams.iter().filter(|g| text.contains(g.as_str())).count();
                covered * 10 >= grams.len() * 7
            });
            assert!(
                hit,
                "L2「{t}」在评分表 {cid} 的 sub_items 中无对应条目（子串或 2-gram 覆盖 ≥0.7）——fixture 交叉漂移"
            );
            l2_total += 1;
        }
    }
    assert_eq!(l2_total, 20, "20 个 L2 细则小节（与评分表 20 条细则一一对应）");

    // ③ 业绩/价格独立响应位指向商务/价格 track（不混进 85 分技术正文）
    let slots = &oe["independent_response_slots"];
    let biz = slots["业绩"]["scoring_item_id"].as_str().unwrap();
    let price = slots["价格"]["scoring_item_id"].as_str().unwrap();
    assert_eq!(by_id(biz)["track"].as_str().unwrap(), "商务评审", "业绩位指向商务评审项");
    assert_eq!(by_id(price)["track"].as_str().unwrap(), "价格评审", "价格位指向价格评审项");

    // ④ 双轨硬约束的数据面：技术参数去向登记在案（进服务方案/质量保证/服务承诺，
    // 不进附录）——「双轨不是全塞附录」的 fixture 侧锚
    let dual = &oe["dual_track_requirements"];
    let empty_into: Vec<Value> = Vec::new();
    let into = dual["tech_params_into"].as_array().unwrap_or(&empty_into);
    assert!(
        !into.is_empty(),
        "双轨硬约束登记面缺失：dual_track_requirements.tech_params_into 为空"
    );
    for item in into {
        let t = item.as_str().unwrap_or("");
        assert!(
            !t.contains("附录"),
            "技术参数去向不得含附录（双轨硬约束——技术参数进服务方案，不是塞附录）: {t:?}"
        );
    }
    let joined = into
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join("；");
    assert!(
        joined.contains("服务方案"),
        "技术参数去向须含服务方案（双轨硬约束主落点）: {joined:?}"
    );
    assert!(
        !dual["commercial_into"]
            .as_array()
            .unwrap_or(&Vec::new())
            .is_empty(),
        "商务/保证金类去向登记面缺失（进入商务资格响应位置）"
    );
}

// ---------------------------------------------------------------- 层1：评分驱动大纲结构（poppler 门）

/// 引擎态初始化（与 #251 pdf_engine_svc 相同口径：单件 PDF → 抽取 → 建矩阵 → 建纲）。
fn outline_engine_svc() -> BidService {
    let pdf = songling().join("tender/光纤租赁费招标文件.pdf");
    if skip_if_fixture_missing("songling_252_layer1", &[&pdf]) {
        panic!("层1 需要 PDF 原件（层0 已校验在位——此处到达即仓库结构损坏）");
    }
    if skip_if_env_missing("songling_252_scoring_outline", POPPLER_TEXT_TOOLS) {
        panic!("环境 SKIP 应在测试入口提前 return（见各测试首行）");
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    svc.create_project("松岭评分大纲基线", "[232762]LJXM[GK]20260001", "明标", "sl-252")
        .expect("建项");
    let report = svc
        .import(&[ImportSpec {
            upload_ref: pdf,
            filename: "光纤租赁费招标文件.pdf".to_string(),
            kind: "招标文件".to_string(),
            expected_pages: None,
        }])
        .expect("导入");
    assert_eq!(report.item_failures, 0, "导入零失败");
    let reqs = extract_requirements(svc.parsed());
    let _ = svc.set_requirements(reqs).expect("抽取落库");
    svc.store_mut().build_matrix().expect("矩阵");
    let _ = svc
        .store_mut()
        .derive_and_set_outline(OutlineSpec::default())
        .expect("建纲");
    svc
}

/// 结构面四门（issue #252 验收 1-5）：
/// - 6 个一级评分章节标题与评分表原文一致（验收 1）；
/// - 20 个 L2 细则小节可追溯（验收 2——标题对齐 + 归属对应 L1）；
/// - 业绩/价格独立响应位（验收 3）；
/// - ScoringMapping 6/6（验收 4——复用 #236 多对多映射，不另起炉灶）。
#[test]
fn pdf_engine_scoring_driven_structure() {
    if skip_if_env_missing("songling_252_structure", POPPLER_TEXT_TOOLS) {
        return; // SKIP（Windows CI 无 poppler——纯 Rust 兜底对本件乱码，#251 实测）
    }
    let _g = engine_lock().lock().unwrap_or_else(|p| p.into_inner());
    let svc = outline_engine_svc();
    let store = svc.store();
    let sections = store.sections();
    let chapters = expected_chapters();

    // ---- 门 1：6 个一级评分章节（验收 1——标题精确出现在 L1）
    let l1_titles: Vec<&str> = sections.iter().filter(|s| s.level == 1).map(|s| s.title.as_str()).collect();
    eprintln!("[songling-252] L1 标题全集: {l1_titles:?}");
    let mut l1_hits = 0u64;
    for (title, _, _) in &chapters {
        if l1_titles.iter().any(|t| t.contains(title.as_str())) {
            l1_hits += 1;
        }
    }
    gate_count("scoring_l1_chapters", "一级评分章节精确对齐", l1_hits, baseline_u64("scoring_l1_chapters"), 6);

    // ---- 门 2：20 个 L2 细则小节（验收 2——存在 + 归属对应评分 L1）
    let mut l2_hits = 0u64;
    for (title, _, l2s) in &chapters {
        for l2 in l2s {
            let traced = sections.iter().any(|s| {
                s.title.contains(l2.as_str()) && parent_chain_contains(s, sections, title)
            });
            if traced {
                l2_hits += 1;
            }
        }
    }
    gate_count("scoring_l2_details", "L2 细则小节可追溯", l2_hits, baseline_u64("scoring_l2_details"), 20);

    // ---- 门 3：业绩/价格独立响应位（验收 3）
    let has_slot = |kws: &[&str]| -> bool {
        l1_titles.iter().any(|t| kws.iter().any(|k| t.contains(k)))
    };
    let slots = u64::from(has_slot(&["业绩"]))
        + u64::from(has_slot(&["价格", "报价"]));
    gate_count("independent_slots", "业绩/价格独立响应位", slots, baseline_u64("independent_slots"), 2);

    // ---- 门 4：ScoringMapping 6/6（验收 4——#236 多对多，不另起炉灶）
    // 判定：每个评分表评审项都存在映射行，其 scoring REQ 的检索面（statement+anchors）
    // 含该评审项名——映射的是真评分项，不是碰巧的行数。
    let mappings = store.scoring_mappings();
    let req_face = |rid: &str| -> String {
        let mut face = String::new();
        if let Some(r) = store.requirements().iter().find(|r| r.id == rid) {
            for a in &r.anchors {
                face.push_str(&a.excerpt);
                face.push_str(&a.locator);
            }
        }
        face
    };
    let mut mapped = 0u64;
    for (title, _, _) in &chapters {
        let hit = mappings.iter().any(|m| {
            let face = req_face(&m.scoring_req_id);
            face.contains(title.as_str())
        });
        if hit {
            mapped += 1;
        }
    }
    eprintln!("[songling-252] scoring_mappings 总数: {}（非硬门评分项基线 9 条）", mappings.len());
    gate_count("scoring_mapped_6", "评分项 ScoringMapping 映射", mapped, baseline_u64("scoring_mapped_6"), 6);
}

/// s 的父链（含自身标题）上是否有标题包含 expected_chapter_title。
fn parent_chain_contains(
    s: &bid_domain::outline::OutlineNode,
    sections: &[bid_domain::outline::OutlineNode],
    expected_chapter_title: &str,
) -> bool {
    let find = |id: &str| sections.iter().find(|x| x.id == id);
    let mut cur: Option<&bid_domain::outline::OutlineNode> = Some(s);
    while let Some(node) = cur {
        if node.title.contains(expected_chapter_title) {
            return true;
        }
        cur = node.parent_id.as_deref().and_then(find);
    }
    false
}

/// 配额分布门（issue #252 评论的强制项 + 验收 10 的结构前提）：
/// - outline_total_chars ≥ 1,200,000（#287 默认档 Conservative 硬指标——
///   取代 #253 的 844,000/1000 页口径，用户裁定 2026-11-06）；
/// - 评分章节配额合计 ≥ 60%；
/// - 行政章节配额合计 ≤ 25%；
/// - 每个评分 L2 小节配额 ≥ 10,000 字。
#[test]
fn pdf_engine_outline_quota_distribution() {
    if skip_if_env_missing("songling_252_quota", POPPLER_TEXT_TOOLS) {
        return; // SKIP
    }
    let _g = engine_lock().lock().unwrap_or_else(|p| p.into_inner());
    let svc = outline_engine_svc();
    let store = svc.store();
    let sections = store.sections();
    let chapters = expected_chapters();

    // ---- 门 5：总配额 ≥ 1,200,000（恒等守恒断言——derive 的配额总和 = target；
    // 此门防 #253 分配重构时的总量回归；#287 起默认档 1.2M）
    let total = store.outline_total_chars();
    eprintln!("[songling-252] outline_total_chars = {total}");
    if total < 1_200_000 {
        panic!(
            "[outline_total] 总配额 {total} < 1,200,000（#287 默认档 Conservative 硬指标——建纲总量回归，见单一真源常量）"
        );
    }

    // ---- 门 6/7：评分 ≥60% / 行政 ≤25%（千分位比例）
    let l1: Vec<&bid_domain::outline::OutlineNode> =
        sections.iter().filter(|s| s.level == 1).collect();
    let is_scoring = |t: &str| chapters.iter().any(|(ct, _, _)| t.contains(ct.as_str()));
    let is_admin = |t: &str| is_admin_group(t);
    let scoring_chars: u64 = l1
        .iter()
        .filter(|s| is_scoring(&s.title))
        .map(|s| u64::from(s.target_chars))
        .sum();
    let admin_chars: u64 = l1
        .iter()
        .filter(|s| is_admin(&s.title))
        .map(|s| u64::from(s.target_chars))
        .sum();
    let denom = total.max(1) as f64;
    let scoring_share = (scoring_chars as f64 / denom * 1000.0).round() / 1000.0;
    let admin_share = (admin_chars as f64 / denom * 1000.0).round() / 1000.0;
    eprintln!(
        "[songling-252] 配额分布: total={total} scoring={scoring_chars}({scoring_share:.3}) admin={admin_chars}({admin_share:.3})"
    );
    gate_ratio_high(
        "quota_scoring_share",
        "评分章节配额占比",
        scoring_share,
        baseline_f64("quota_scoring_share"),
        0.60,
    );
    gate_ratio_low(
        "quota_admin_share",
        "行政章节配额占比",
        admin_share,
        baseline_f64("quota_admin_share"),
        0.25,
    );

    // ---- 门 8：评分 L2 小节配额 ≥ 10,000 字（可展开的真题目）
    let scoring_l2: Vec<&bid_domain::outline::OutlineNode> = sections
        .iter()
        .filter(|s| {
            s.level == 2
                && s.parent_id
                    .as_deref()
                    .and_then(|pid| sections.iter().find(|x| x.id == pid))
                    .map(|p| is_scoring(&p.title))
                    .unwrap_or(false)
        })
        .collect();
    eprintln!(
        "[songling-252] 评分 L2 小节 {} 个（配额清单: {:?}）",
        scoring_l2.len(),
        scoring_l2.iter().map(|s| (s.title.as_str(), s.target_chars)).collect::<Vec<_>>()
    );
    let l2_floor_hits = scoring_l2
        .iter()
        .filter(|s| s.target_chars >= 10_000)
        .count() as u64;
    gate_count(
        "quota_scoring_l2_floor",
        "评分 L2 配额 ≥10,000 字的小节数",
        l2_floor_hits,
        baseline_u64("quota_scoring_l2_floor"),
        20,
    );
}

/// 覆盖面门（issue #252 验收 7-9——#250 未修复前的预期红，如实固化）：
/// - 关键技术参数在**章节 REQ 绑定集**可检索（验收 8：完好率/360分钟/295条）；
/// - 否决项/资格条件全部有响应位置（验收 7：章节绑定覆盖）。
#[test]
fn pdf_engine_section_binding_coverage() {
    if skip_if_env_missing("songling_252_coverage", POPPLER_TEXT_TOOLS) {
        return; // SKIP
    }
    let _g = engine_lock().lock().unwrap_or_else(|p| p.into_inner());
    let svc = outline_engine_svc();
    let store = svc.store();
    let sections = store.sections();

    // ---- 门 9：关键技术参数的章节绑定检索（验收 8）
    // 检索面 = 章节绑定 REQ 的 anchors（数值在绑定集不丢——双轨硬约束的可验证面）
    let bound_ids: HashSet<&str> = sections
        .iter()
        .flat_map(|s| s.requirement_ids.iter().map(|r| r.as_str()))
        .collect();
    let bound_face: String = store
        .requirements()
        .iter()
        .filter(|r| bound_ids.contains(r.id.as_str()))
        .flat_map(|r| r.anchors.iter().map(|a| format!("{}{}", a.excerpt, a.locator)))
        .collect();
    let needles = ["95%", "360分钟", "295"];
    let mut hits = 0u64;
    for n in needles {
        if bound_face.contains(n) {
            hits += 1;
        }
    }
    gate_count(
        "coverage_key_params_in_sections",
        "关键技术参数章节绑定可检索（完好率/历时/链路数）",
        hits,
        baseline_u64("coverage_key_params_in_sections"),
        3,
    );

    // ---- 门 10：否决项/资格条件的响应位置覆盖（验收 7）
    // 这是**防回归门**（当前已达成——#121 P0-1 的「资格与响应」组成果），
    // 不是失败基线：#252 重构大纲结构时不得丢失既有的响应位置覆盖。
    let need_response: Vec<&bid_domain::Requirement> = store
        .requirements()
        .iter()
        .filter(|r| bid_domain::outline::needs_response_section(&r.category))
        .collect();
    let covered = need_response
        .iter()
        .filter(|r| bound_ids.contains(r.id.as_str()))
        .count() as u64;
    let total_need = need_response.len() as u64;
    eprintln!("[songling-252] 否决/资格响应位: {covered}/{total_need} 条被章节绑定");
    assert_eq!(
        covered, total_need,
        "否决项/资格条件响应位置覆盖回归（issue #252 验收 7：全部须有响应位置——
         当前大纲已达成此面，重构不得丢失）"
    );
}

// ---------------------------------------------------------------- 层2：导出目录对齐（素材缺件 → SKIP）

/// 验收 11（导出目录与大纲结构一致）需要 97 页产物与 docx 全链路素材——
/// issue #251 层2 素材（lead 归档中）到位后实现。
#[test]
fn fullchain_export_toc_alignment() {
    let pdf = songling().join("artifacts/baseline-97p.pdf");
    let docx = songling().join("tender/光纤租赁费招标文件-文本修复版v5w.docx");
    if skip_if_fixture_missing("songling_252_export_toc", &[&pdf, &docx]) {
        return; // SKIP（响亮横幅——层2 素材待 lead 提供，不伪装通过）
    }
    todo!("素材到位后实现：导出目录与大纲 L1/L2 对齐断言 + 页数实测（issue #252 验收 11 + #251 层2）")
}
