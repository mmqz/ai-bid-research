//! songling_baseline —— #251 真实语料回归 Fixture 的断言面（松岭 TF-0002/TF-0003）。
//!
//! 分层口径（manifest.json 是数据真源，本文件是执行面）：
//! - **层0 fixture 完整性**：PDF sha256/页数 + 评分表/金标准/预期大纲 JSON 结构。
//!   永远运行（纯静态，无引擎依赖——CI 无 poppler/tesseract 也能跑）。
//! - **层1 PDF 引擎基线**（`pdf_engine_*`）：TF-0002 原件单件导入 → 抽取 →
//!   7 维度断言。数字防回归门与失败基线固化，数据源 `manifest.layer1_pdf_baseline`。
//! - **层2 全链路基线**（`fullchain_*`）：TF-0003 docx + Agent 67 条 + 97 页产物。
//!   素材缺件 → 响亮 SKIP（issue #9 口径：缺件不是业务失败，也绝不静默假装通过）。
//!
//! **失败基线固化的语义**（issue #251 §5，与「待修复后删掉」相反）：
//! - 实测值与 `layer1_pdf_baseline` 一致 → 打印 `FAIL(固化基线)` 横幅，测试通过
//!   （当前真实现状的锁定，等待 #250/#252 修复）；
//! - 实测值比基线更差 → **assert 红灯**（新回归）；
//! - 实测值比基线好（目标达成）→ 打印 `IMPROVED` 横幅并要求更新 manifest +
//!   PR 说明原因（防止静默漂移）。
//!
//! 一条命令重跑：`cargo test -p bid-tools --test songling_baseline`。

use bid_domain::outline::OutlineSpec;
use bid_parse::env_preflight::{skip_if_env_missing, skip_if_fixture_missing, POPPLER_TEXT_TOOLS};
use bid_parse::extract::extract_requirements;
use bid_tools::service::{BidService, ImportSpec};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// 层1 四个测试互斥（4G/2核沙箱：PDF 解析耗内存，并行导入会 OOM——
/// 逻辑上四测独立，此锁只串行化引擎部分，不共享状态）。
fn engine_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn fixtures() -> PathBuf {
    let p = std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures"));
    // fs_fence 拒绝 `..` 段（#183/#189）——先 canonicalize（service_import.rs 同惯例）
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

// ---------------------------------------------------------------- 层0：fixture 完整性

#[test]
fn layer0_manifest_and_pdf_integrity() {
    let m = load_json("manifest.json");
    let pdf = songling().join("tender/光纤租赁费招标文件.pdf");
    assert!(pdf.is_file(), "PDF 原件缺失: {}", pdf.display());

    // sha256（issue #250/#251 登记哈希——与用户工作机原件逐字节一致的锚）
    let bytes = std::fs::read(&pdf).expect("PDF 读取失败");
    let mut h = Sha256::new();
    h.update(&bytes);
    let got = hex_sha(&mut h);
    let want = m["files"]["tender_pdf"]["sha256"].as_str().unwrap();
    assert_eq!(got, want, "PDF sha256 漂移（fixture 完整性破坏）");

    assert_eq!(bytes.len() as u64, m["files"]["tender_pdf"]["bytes"].as_u64().unwrap());
    assert_eq!(m["files"]["tender_pdf"]["pages"].as_u64().unwrap(), 65, "页数基线 65");

    // 关键失败基线清单（issue §5 的 10 项必须在 manifest 在位——层1/层2 的对账真源）
    let items = m["known_failure_baseline"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 10, "issue #251 §5 已知失败项 10 条固化");
    // 层1 PDF 基线（2026-11-05 实测）在位
    assert!(m["layer1_pdf_baseline"]["category_counts"].is_object());
}

fn hex_sha(h: &mut Sha256) -> String {
    h.finalize_reset().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn layer0_scoring_table_structure() {
    let st = load_json("expected/scoring_table.json");
    let items = st["items"].as_array().unwrap();
    assert_eq!(items.len(), 8, "评分表 8 项（6 技术 + 业绩 + 价格）");

    let tech: Vec<&Value> = items.iter().filter(|i| i["track"] == "技术评审").collect();
    assert_eq!(tech.len(), 6, "6 个技术评审项");
    let score_sum: f64 = tech.iter().filter_map(|i| i["score"].as_f64()).sum();
    assert!((score_sum - 85.0).abs() < 1e-9, "技术分合计 85（实测 {score_sum}）");
    let grand: f64 = items.iter().filter_map(|i| i["score"].as_f64()).sum();
    assert!((grand - 100.0).abs() < 1e-9, "总分 100（实测 {grand}）");

    // 20 个 L2 细则条目与 #252 预期大纲 20 小节对齐
    let subs: usize = tech.iter().filter_map(|i| i["sub_items"].as_array().map(|a| a.len())).sum();
    assert_eq!(subs, 20, "技术评审 20 个 L2 细则条目");

    // 分值序列与招标文件评分表原文锚定（第 25-28 页）
    let seq: Vec<f64> = tech.iter().filter_map(|i| i["score"].as_f64()).collect();
    assert_eq!(seq, vec![25.0, 10.0, 10.0, 8.0, 20.0, 12.0], "分值序列真值锚定");

    // 量化细则（响应 30 分钟 / 恢复 2 小时）不得丢失
    let sc04 = tech.iter().find(|i| i["id"] == "SC-04").unwrap();
    let texts: String = sc04["criteria"].as_str().unwrap().to_string();
    assert!(texts.contains("30分钟") && texts.contains("2小时"), "SC-04 量化承诺锚（30分钟/2小时）");
}

#[test]
fn layer0_reviewed_requirements_structure() {
    let gr = load_json("expected/requirements.reviewed.json");
    let reqs = gr["requirements"].as_array().unwrap();
    assert_eq!(reqs.len(), 16, "金标准 v1 关键参数 16 条");
    for r in reqs {
        let id = r["id"].as_str().unwrap();
        for field in ["statement", "category", "excerpt", "locator"] {
            let v = r[field].as_str().unwrap_or("");
            assert!(!v.trim().is_empty(), "{id}.{field} 非空（复核记录完整性）");
        }
        assert!(r["page"].as_u64().is_some_and(|p| (1..=65).contains(&p)), "{id} 页码在 1-65");
        assert!(!r["review_note"].as_str().unwrap_or("").is_empty(), "{id} 复核依据在位");
    }
    // 分类计数（技术 7 / 商务 8 / 资格 1）
    let mut cats: BTreeMap<String, usize> = Default::default();
    for r in reqs {
        *cats.entry(r["category"].as_str().unwrap().to_string()).or_default() += 1;
    }
    assert_eq!(
        cats,
        BTreeMap::from([("商务要求".into(), 8), ("技术参数".into(), 7), ("资格条件".into(), 1)]),
        "关键参数分类计数"
    );
    // ★实质性条款 2 条（服务地点 + 验收要求——复核记录 D4）
    let stars = reqs.iter().filter(|r| r["star"] == true).count();
    assert_eq!(stars, 2, "★实质性条款 2 条");
}

#[test]
fn layer0_outline_expected_structure() {
    let oe = load_json("expected/outline.expected.json");
    let chapters = oe["chapters"].as_array().unwrap();
    assert_eq!(chapters.len(), 6, "6 个一级评分章节");
    let l2: usize = chapters.iter().filter_map(|c| c["l2"].as_array().map(|a| a.len())).sum();
    assert_eq!(l2, 20, "20 个 L2 小节（与评分表 20 细则一一对应）");

    // 业绩/价格独立响应位（不混入 85 分正文）
    assert!(oe["independent_response_slots"]["业绩"].is_object());
    assert!(oe["independent_response_slots"]["价格"].is_object());

    // 第七章 16 项强制格式序（强制格式优先于评分驱动）
    let toc = oe["mandatory_format_order"]["chapter7_toc_16_items"].as_array().unwrap();
    assert_eq!(toc.len(), 16, "第七章 16 项格式目录");
    assert_eq!(toc[14].as_str().unwrap(), "十五、项目实施方案、质量保证及售后服务承诺等", "评分主体落笔位 = 格式十五");

    // 页数配额目标锚（#252/#253 的验收口径）
    let q = &oe["page_quota_targets"];
    assert_eq!(q["outline_total_chars_min"].as_u64().unwrap(), 1_200_000, "#287 默认档硬指标 = 1,200,000 字（Conservative ≈ 1422 页，取代 844,000/1000 页）");
    assert_eq!(q["scoring_sections_char_share_min"].as_f64().unwrap(), 0.60);
    assert_eq!(q["admin_sections_char_share_max"].as_f64().unwrap(), 0.25);
}

/// 层0：金标准 needle 形态自洽（#267 全量形态风险扫描的固化）。
///
/// 2026-11-05 扫描结论（16/16 needle）：
/// - 每条 needle 的 N3 形态都能在**自身 excerpt** 的 N3 形态中检索到——
///   金标准数据自洽；层1 的 11 MISS 根因全部是「值不在抽取锚点面」
///   （#266 值行装配，B 面），不是判据形态问题；
/// - needle 全部使用标准字形/半角（判据是人写的检索目标——原文才是
///   康熙部首/全角形态的载体，由 norm_n3 匹配瞬间归一相遇）；
/// - 松岭 excerpt 实测含 19 种康熙部首字符（⾦×11 / ⽇×5 / ⼤×5 …）——
///   部首归一是通用必要，不止 GR-005 一处。
///
/// 本断言防未来金标准再造「needle 形态与摘录脱节」的假阴性
/// （纯静态——CI 无 poppler 也永远跑，与层0 其它面同口径）。
#[test]
fn layer0_golden_needle_form_selfcheck() {
    let gr = load_json("expected/requirements.reviewed.json");
    let is_kangxi = |c: char| ('\u{2F00}'..='\u{2FD5}').contains(&c);
    let is_fullwidth = |c: char| ('\u{FF01}'..='\u{FF5E}').contains(&c);

    let mut kangxi_kinds: std::collections::BTreeMap<char, usize> = Default::default();
    for r in gr["requirements"].as_array().unwrap() {
        let id = r["id"].as_str().unwrap();
        let excerpt = bid_parse::norm_n3(r["excerpt"].as_str().unwrap());

        // excerpt 原文（归一前）的康熙部首分布登记
        for c in r["excerpt"].as_str().unwrap().chars() {
            if is_kangxi(c) {
                *kangxi_kinds.entry(c).or_default() += 1;
            }
        }

        let needles = r["retrieval_needles"].as_array().unwrap();
        assert!(!needles.is_empty(), "{id} retrieval_needles 非空");
        for n in needles {
            let needle = n.as_str().unwrap();
            // 判据纪律：标准字形 + 半角（原文形态由 N3 匹配瞬间归一）
            assert!(
                !needle.chars().any(is_kangxi),
                "{id} needle「{needle}」含康熙部首——判据必须用标准字形"
            );
            assert!(
                !needle.chars().any(is_fullwidth),
                "{id} needle「{needle}」含全角字符——判据必须用半角"
            );
            // 自洽：needle 的 N3 形态可检索于自身 excerpt 的 N3 形态
            assert!(
                excerpt.contains(&bid_parse::norm_n3(needle)),
                "{id} needle「{needle}」N3 形态不在自身 excerpt 中——金标准检索判据与摘录脱节"
            );
        }
    }

    // 19 种康熙部首字符的形态分布快照（分布变化 = fixture 原件替换，
    // 必须在 PR 说明并重扫全部 needle——防静默漂移）
    assert_eq!(kangxi_kinds.len(), 19, "松岭 excerpt 康熙部首种类数（2026-11-05 扫描基线）");
    assert_eq!(kangxi_kinds.get(&'\u{2F47}'), Some(&5), "⽇ U+2F47 出现 5 次（GR-005/GR-014 形态锚）");
    assert_eq!(kangxi_kinds.get(&'\u{2FA6}'), Some(&11), "⾦ U+2FA6 出现 11 次（金额语境高发）");
}

// ---------------------------------------------------------------- 层1：PDF 引擎基线

/// 建一次引擎态（层1 四个测试共享的初始化——每测试独立跑，状态零共享）。
/// 环境 SKIP：无 poppler（Windows CI）时纯 Rust 兜底对本件输出乱码
/// （实测 cjk=0——字体形态在兜底甜区外，engines 会 fail-loud），
/// 层1 引擎面按 issue #9 纪律 SKIP（层0 静态面永远跑）。
fn pdf_engine_svc() -> BidService {
    let pdf = songling().join("tender/光纤租赁费招标文件.pdf");
    if skip_if_fixture_missing("songling_layer1", &[&pdf]) {
        panic!("层1 需要 PDF 原件（层0 已校验在位——此处到达即仓库结构损坏）");
    }
    if skip_if_env_missing("songling_layer1_pdf_engine", POPPLER_TEXT_TOOLS) {
        panic!("环境 SKIP 应在测试入口提前 return（见各测试首行）");
    }
    let mut svc = BidService::new().with_fs_root(fixtures());
    svc.create_project("松岭光纤租赁基线", "[232762]LJXM[GK]20260001", "明标", "sl-base")
        .expect("建项");
    let report = svc
        .import(&[ImportSpec {
            upload_ref: pdf,
            // 真实文件名（无夹具前缀——#250 的教训：真实名与夹具名必须同路径）
            filename: "光纤租赁费招标文件.pdf".to_string(),
            kind: "招标文件".to_string(),
            expected_pages: None,
        }])
        .expect("导入");
    assert_eq!(report.item_failures, 0, "导入零失败");
    let reqs = extract_requirements(svc.parsed());
    let _ = svc.set_requirements(reqs).expect("抽取落库");
    svc
}

fn baseline_u64(key: &str) -> u64 {
    load_json("manifest.json")["layer1_pdf_baseline"][key]
        .as_u64()
        .unwrap_or_else(|| panic!("manifest.layer1_pdf_baseline.{key} 缺失"))
}

/// 防回归门 + 失败基线固化的统一裁决。
///
/// 返回 (actual, improved)。裁决输出三种横幅：
/// - `>= known` 且 `< target`：FAIL(固化基线)——现状锁定，等待修复（测试通过）；
/// - `>= target`：IMPROVED——修复已发生，更新 manifest 需 PR 说明原因（测试通过）；
/// - `< known`：assert 红灯（新回归——issue #251「不允许更多」）。
fn gate_count(id: &str, label: &str, actual: u64, known: u64, target: u64) -> bool {
    if actual < known {
        panic!(
            "[{id}] {label} 回归：实测 {actual} < 固化基线 {known}（issue #251：失败项不允许更多——这是新缺陷，不是基线漂移）"
        );
    }
    if actual >= target {
        banner(&format!("IMPROVED [{id}] {label}: {actual} ≥ 目标 {target}——基线已被超越。请更新 manifest.layer1_pdf_baseline 并在 PR 说明原因"));
        true
    } else {
        banner(&format!("FAIL(固化基线) [{id}] {label}: 实测 {actual}（基线 {known}，目标 ≥{target}）——issue #251 现状固化，待 #250/#252 修复"));
        false
    }
}

#[test]
fn pdf_engine_req_counts_and_categories() {
    if skip_if_env_missing("songling_layer1_counts", POPPLER_TEXT_TOOLS) {
        return; // SKIP（Windows CI 无 poppler——纯 Rust 兜底对本件乱码，实测不可用）
    }
    let _g = engine_lock().lock().unwrap_or_else(|p| p.into_inner());
    let svc = pdf_engine_svc();
    let store = svc.store();
    let total = store.requirements().len() as u64;

    // 召回下限防回归（297 实测基线；上限防重复爆炸——#250 反模式：362 条含重复）
    let known_total = baseline_u64("req_total");
    assert!(
        total >= known_total && total <= known_total + 80,
        "REQ 总量 {total} 偏离基线带 [{known_total}, {}]（下界=召回回归，上界=重复爆炸）",
        known_total + 80
    );

    let mut cats: BTreeMap<String, u64> = Default::default();
    for r in store.requirements() {
        *cats.entry(r.category.clone()).or_default() += 1;
    }
    eprintln!("[songling-layer1] 类别计数: {cats:?}");

    // 技术参数（PDF 路径已达 issue 目标 11——防回归 + 改善检测）
    let tech = *cats.get("技术参数").unwrap_or(&0);
    gate_count("tech_req_count", "技术参数类 REQ", tech, 11, 11);

    // 商务要求（两路径一致为 0——整表丢失，固化失败）
    let biz = *cats.get("商务要求").unwrap_or(&0);
    gate_count("biz_req_count", "商务要求类 REQ", biz, 0, 13);

    // 评分项（PDF 基线 9：对应评分表 6 技术项 + 业绩 + 价格 + 1 细则子项）
    let scoring_req = *cats.get("评分项").unwrap_or(&0);
    gate_count("scoring_req_count", "评分项类 REQ", scoring_req, 9, 8);
}

#[test]
fn pdf_engine_param_retrievability() {
    if skip_if_env_missing("songling_layer1_retrievability", POPPLER_TEXT_TOOLS) {
        return; // SKIP
    }
    let _g = engine_lock().lock().unwrap_or_else(|p| p.into_inner());
    let svc = pdf_engine_svc();
    // 检索面 = REQ 的 anchor excerpt + locator 拼接（数值完整性的判据——
    // issue #250「数值与单位不得丢失」在抽取结果的可检索性口径）。
    // #267 匹配瞬间归一：haystack 与 needle 双侧 norm_n3（NFKC 康熙部首
    // ⽇→日 + 千分位剥离）——「数据不动、匹配归一」纪律，不落库不改真源。
    let haystack: String = svc
        .store()
        .requirements()
        .iter()
        .flat_map(|r| r.anchors.iter().map(|a| format!("{}{}", a.excerpt, a.locator)))
        .collect();
    let hay = bid_parse::norm_n3(&haystack);

    // 16 锚点 = 金标准 v1 的检索判据（retrieval_needles 数组 AND 语义）
    let gr = load_json("expected/requirements.reviewed.json");
    let mut hits = 0u64;
    let mut misses: Vec<String> = Vec::new();
    for r in gr["requirements"].as_array().unwrap() {
        let id = r["id"].as_str().unwrap();
        let needles = r["retrieval_needles"].as_array().unwrap_or_else(|| {
            panic!("{id} 缺 retrieval_needles（金标准检索锚点字段）")
        });
        let all_hit = needles.iter().all(|n| {
            let needle = bid_parse::norm_n3(n.as_str().unwrap());
            hay.contains(&needle)
        });
        if all_hit {
            hits += 1;
        } else {
            let ns: Vec<&str> = needles.iter().map(|n| n.as_str().unwrap()).collect();
            misses.push(format!("{id}:{:?}", ns));
        }
    }

    // 防回归门：4/16 命中基线（12 MISS 是 #250 表格碎片化的固化现状）
    let known = baseline_u64("retrievable_hits_of_16");
    if hits < known {
        panic!(
            "[param_integrity] 关键参数可检索性回归：{hits}/16 < 基线 {known}/16（新 MISS: {misses:?}）"
        );
    }
    if hits >= 16 {
        banner(&format!("IMPROVED [param_integrity] 关键参数可检索 {hits}/16——数值完整性已闭环。请更新 manifest 并在 PR 说明原因"));
    } else {
        banner(&format!(
            "FAIL(固化基线) [param_integrity] 关键参数可检索 {hits}/16（基线 {known}/16）——MISS 清单: {misses:?}（issue #250 表格碎片化现状）"
        ));
    }
}

#[test]
fn pdf_engine_outline_shape() {
    if skip_if_env_missing("songling_layer1_outline", POPPLER_TEXT_TOOLS) {
        return; // SKIP
    }
    let _g = engine_lock().lock().unwrap_or_else(|p| p.into_inner());
    let mut svc = pdf_engine_svc();
    svc.store_mut().build_matrix().expect("矩阵");
    let (nodes, total_chars) = svc
        .store_mut()
        .derive_and_set_outline(OutlineSpec::default())
        .expect("建纲");
    let sections = svc.store().sections();

    // 大纲规模防回归（40 节点实测基线）
    let known_nodes = baseline_u64("outline_nodes");
    assert!(
        nodes as u64 >= known_nodes,
        "[outline] 节点数回归：{nodes} < 基线 {known_nodes}"
    );
    eprintln!("[songling-layer1] outline: nodes={nodes} total_chars={total_chars}");

    // 「（第N块）」无语义标题（PDF 基线 7 处；issue docx 口径 30 处——两处都非 0）
    let block_titles = sections
        .iter()
        .filter(|s| s.title.contains("第") && s.title.contains("块"))
        .count() as u64;
    let known_blocks = baseline_u64("outline_block_titles");
    if block_titles > known_blocks {
        panic!(
            "[outline_block_titles] 机械分块标题回归：{block_titles} > 基线 {known_blocks}"
        );
    }
    if block_titles == 0 {
        banner("IMPROVED [outline_block_titles] 机械分块标题已清零——请更新 manifest 并在 PR 说明原因");
    } else {
        banner(&format!(
            "FAIL(固化基线) [outline_block_titles] 「（第N块）」标题 {block_titles} 处（基线 {known_blocks}，目标 0）——issue #252 §4 机械分块禁令"
        ));
    }

    // 85 分评分主体必须在大纲有对应位置（issue #252 核心矛盾）：
    // 评分表 6 个评审项标题（服务方案/重点难点/保密工作/服务承诺/质量保证/应急方案）
    let titles: Vec<String> = sections.iter().map(|s| s.title.clone()).collect();
    let expected_names = ["服务方案", "保密", "服务承诺", "质量保证", "应急", "重点"];
    let mut found = 0;
    for kw in expected_names {
        if titles.iter().any(|t| t.contains(kw)) {
            found += 1;
        }
    }
    if found < 6 {
        banner(&format!(
            "FAIL(固化基线) [scoring_chapters] 评分章节出现 {found}/6（目标 6/6——85 分技术标主体在大纲零对应位置，issue #252 核心矛盾）"
        ));
    } else {
        banner("IMPROVED [scoring_chapters] 6/6 评分章节已入大纲——请更新 manifest 并在 PR 说明原因");
    }
}

#[test]
fn pdf_engine_scoring_items_and_mapping() {
    if skip_if_env_missing("songling_layer1_scoring", POPPLER_TEXT_TOOLS) {
        return; // SKIP
    }
    let _g = engine_lock().lock().unwrap_or_else(|p| p.into_inner());
    let mut svc = pdf_engine_svc();
    svc.store_mut().build_matrix().expect("矩阵");
    let _ = svc.store_mut().derive_and_set_outline(OutlineSpec::default()).expect("建纲");
    let store = svc.store();

    let scoring = store.scoring_items();
    let with_weight = scoring.iter().filter(|i| i.weight.is_some()).count() as u64;
    let non_hard = scoring.iter().filter(|i| !i.is_hard_gate).count() as u64;
    let hard = scoring.iter().filter(|i| i.is_hard_gate).count() as u64;
    eprintln!("[songling-layer1] scoring_items: total={} hard={} non_hard={} weighted={}", scoring.len(), hard, non_hard, with_weight);

    // 评分项条数（PDF 基线 9：评分表真值 8 项 + 引擎多 1 条——待层2 对账）
    gate_count("scoring_req_count", "评分项条数（非硬门）", non_hard, 9, 8);
    // 带分值评分项（PDF 基线 1：9 条评分项仅 1 条带 weight——分值大量丢失是
    // 独立于条数计数的缺陷面，与评分表真值 8 项带分对照）
    gate_count("scoring_weighted", "带分值评分项", with_weight, 1, 8);

    // 评分项 → 章节映射（issue #252 验收 4：scoring_mappings 评分项全部有 section_id）
    let mappings = store.scoring_mappings().len() as u64;
    if mappings == 0 {
        banner("FAIL(固化基线) [scoring_mapped] 评分映射 0/6（目标 6/6——13 条评分 REQ 全部悬空，issue #252 现状）");
    } else if mappings >= 6 {
        banner(&format!("IMPROVED [scoring_mapped] 评分映射 {mappings} 条——请更新 manifest 并在 PR 说明原因"));
    } else {
        // 1..=5 条：比 0 好（有人在接线）但未达 6——如实报告，不拦（中间态）
        banner(&format!("PARTIAL [scoring_mapped] 评分映射 {mappings}/6（基线 0，目标 6——接线中间态）"));
    }
}

// ---------------------------------------------------------------- 层2：全链路基线（素材缺件 → SKIP）

#[test]
fn fullchain_agent_recall_precision_baseline() {
    // 素材：Agent 67 条 + 76 区间 + TF-0003 docx（issue #251 素材表）
    let submitted = songling().join("agent/submitted_requirements.json");
    let marks = songling().join("agent/block_marks.json");
    let docx = songling().join("tender/光纤租赁费招标文件-文本修复版v5w.docx");
    if skip_if_fixture_missing(
        "songling_fullchain_recall_precision",
        &[&submitted, &marks, &docx],
    ) {
        return; // SKIP（响亮横幅已打印——层2 素材待 lead 提供，不伪装通过）
    }
    // 素材到位后的断言面（召回/精度/去重/溯源/分类五维对账 + 67 条逐条复核 v2）
    todo!("素材到位后实现：67 条召回对账 / 295 条精度抽查 / 同文同源去重 / anchor 页码校验 / 分类对照——issue #251 §4 七维断言")
}

#[test]
fn fullchain_baseline_export_pages() {
    // 素材：97 页产物 PDF（页数 fitz 实测口径——层2 到位后固化 97 基线与 ≥1000 目标）
    let pdf = songling().join("artifacts/baseline-97p.pdf");
    if skip_if_fixture_missing("songling_fullchain_export_pages", &[&pdf]) {
        return; // SKIP
    }
    todo!("素材到位后实现：fitz 实测页数 == 97 固化 / ≥1000 目标 / 评分章节页数占比 ≥60% / 行政 ≤25% / 字符密度 844±10%")
}
