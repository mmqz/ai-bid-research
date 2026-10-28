//! S-5 审核分片 + 全局一致性单列验收（issue #11 批次 v3 / V3-4）。
//!
//! 口径（编排者裁决，写死在本文件）：「6 类冲突」= 4 类全局（金额/工期/术语/
//! 承诺）+ 2 类局部（占位符残留/引用悬空），各植 1 处全检出。
//!
//! 夹具：合成 500 章（每章恰 2000 字中文正文——轮转携带金额/工期/术语/承诺
//! 四类维度特征之一且值全一致；citations 引真实 REQ id 池）。
//!
//! 断言面：
//! - 规格硬验收：6 类冲突各 1 处全检出 + 无误报（500 章恰 6 处）；
//! - 内存硬标准：500 章（≈100 万字）跑完，进程峰值 RSS（VmHWM）< 200MB；
//! - 特征元组内存纪律：特征集合序列化 < 100KB（不保留原文的结构证据）；
//! - 确定性：并行分片与串行收集逐字节全等；同输入 5 连跑全等（R-15 可重放）；
//! - 规模门（borrow：mdcore `r11_stress.rs`——大合成语料 + 完成性断言，
//!   不做脆弱的精确时间断言）：1000 章跑通 + 植入必检出 + 无 panic。
//!
//! SKIP 面（issue #9 纪律）：非 Linux 平台无 /proc/self/status 的 VmHWM 口径 →
//! 响亮横幅 + 提前 return（环境缺件不是业务失败，也绝不静默假装通过）。

use bid_tools::check::{
    check_global, check_local, extract_features, run_review, ChapterInput, CheckFinding, CheckKind,
    CheckSeverity, FeatureTuple, ReviewOutcome,
};
use std::collections::HashSet;

// ---------------------------------------------------------------- 夹具

/// 合成语料的章配额（也是 target_chars——夹具全部精确达标，格式面零噪声）。
const TARGET: usize = 2000;

/// 拥有态章（测试夹具的存储面——ChapterInput 是借用面，装配在 owned 记录上）。
struct OwnedChapter {
    section_id: String,
    content: String,
    citations: Vec<String>,
    target_chars: Option<u32>,
}

/// 拥有态章 → 借用输入面（零拷贝映射——与存储层解耦的证明）。
fn as_input(c: &OwnedChapter) -> ChapterInput<'_> {
    ChapterInput {
        section_id: &c.section_id,
        content: &c.content,
        citations: &c.citations,
        target_chars: c.target_chars,
    }
}

/// 合成章（0 基索引 i → 1 基章号 SEC-{i+1:04}）：extra 植入句置首 + 轮转维度
/// 句（i%4：金额/工期/术语/承诺——各维度 125 章、值全一致）+ 中文填充到恰好
/// 2000 字。填充句零数字、零维度词、零括号——干净章节零发现的前提。
fn mk_chapter(i: usize, extra: &str) -> OwnedChapter {
    let dim = match i % 4 {
        0 => "投标保证金：50万元，以保函形式提交。",
        1 => "工期：60天完成全部实施内容。",
        2 => "本项目依据ISO（国际标准化组织）相关标准执行。",
        _ => "运维阶段提供7×24小时值守支持。",
    };
    let mut content = String::with_capacity(TARGET * 3);
    content.push_str(extra);
    content.push_str(dim);
    let filler = "本节围绕实施方案与现场条件展开论述，逐项明确人员配置、质量保障、安全文明施工等要求，确保各项措施责任到人、落地可行。";
    while content.chars().count() < TARGET {
        let need = TARGET - content.chars().count();
        content.extend(filler.chars().take(need));
    }
    let citations = vec![format!("REQ-{:04}", 1 + i % 400), format!("REQ-{:04}", 1 + (i + 7) % 400)];
    OwnedChapter { section_id: format!("SEC-{:04}", i + 1), content, citations, target_chars: Some(TARGET as u32) }
}

/// 验收夹具：500 章干净语料 + 六类植入各 1 处（1 基章号 → 0 基索引 N-1）。
fn acceptance_corpus() -> (Vec<OwnedChapter>, HashSet<String>) {
    let refs: HashSet<String> = (1..=400).map(|i| format!("REQ-{i:04}")).collect();
    let mut corpus: Vec<OwnedChapter> = (0..500).map(|i| mk_chapter(i, "")).collect();
    // 六类植入（口径：4 全局 + 2 局部——编排者裁决）：
    corpus[50] = mk_chapter(50, "本段待补充。"); // 第 51 章：占位符残留（局部）
    corpus[101].citations.push("REQ-9999".into()); // 第 102 章：引用悬空（局部）
    corpus[202] = mk_chapter(202, "投标保证金80万元，请以此为准。"); // 第 203 章：金额冲突（全局）
    corpus[303] = mk_chapter(303, "工期90天完成全部部署。"); // 第 304 章：工期冲突（全局）
    corpus[404] = mk_chapter(404, "通过ISO（国际化标准组织）认证。"); // 第 405 章：术语冲突（全局）
    corpus[455] = mk_chapter(455, "服务窗口为5×8。"); // 第 456 章：承诺冲突（全局）
    (corpus, refs)
}

/// 定制小夹具（无配额、无引用——测试 3/4 的裸内容面）。
fn mk_bare(id: &str, content: &str) -> OwnedChapter {
    OwnedChapter { section_id: id.to_string(), content: content.to_string(), citations: vec![], target_chars: None }
}

fn inputs_of(corpus: &[OwnedChapter]) -> Vec<ChapterInput<'_>> {
    corpus.iter().map(as_input).collect()
}

/// 合并排序口径（与 run_review 生产面同一比较器——测试的串行参考必须复用，
/// 否则比的是排序差异不是结果差异）。
fn sort_key(f: &CheckFinding) -> (String, CheckKind, String) {
    (f.section_id.clone(), f.kind, f.message.clone())
}

/// Linux /proc/self/status 的 VmHWM（进程峰值 RSS，单位 kB）。
/// 非 Linux / 无 procfs → None（SKIP 面——issue #9 纪律，不伪装通过）。
fn vmhwm_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|l| {
        l.strip_prefix("VmHWM:")
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse().ok())
    })
}

// ---------------------------------------------------------------- 1. 规格硬验收

/// 规格硬验收：500 章正常内容（四类维度值全一致）+ 植入 6 类冲突各 1 处 →
/// 全检出（逐 kind 断言 section_id 正确）且无误报（恰 6 处、全 Blocker、
/// 零 Warning——干净章节零发现）。
#[test]
fn acceptance_500_chapters_six_conflicts_all_detected() {
    let (corpus, refs) = acceptance_corpus();
    let outcome = run_review(&inputs_of(&corpus), &refs);
    let findings = &outcome.findings;

    // 无误报面：504 章恰好 6 处发现，全部 Blocker（格式面零噪声——夹具精确达标）
    assert_eq!(findings.len(), 6, "500 章 + 6 处植入 = 恰 6 处发现，多一处即误报: {findings:?}");
    assert!(findings.iter().all(|f| f.severity == CheckSeverity::Blocker), "六类口径全 Blocker");

    // 逐 kind 断言：发现类别 + 定位章正确（局部 = 所属章；全局 = 少数派锚点章）
    let find = |kind: CheckKind| findings.iter().find(|f| f.kind == kind).unwrap_or_else(|| panic!("漏检 {kind:?}"));
    assert_eq!(find(CheckKind::Placeholder).section_id, "SEC-0051", "第 51 章占位符");
    assert_eq!(find(CheckKind::DanglingRef).section_id, "SEC-0102", "第 102 章引用悬空");
    assert_eq!(find(CheckKind::AmountConflict).section_id, "SEC-0203", "第 203 章金额冲突");
    assert_eq!(find(CheckKind::ScheduleConflict).section_id, "SEC-0304", "第 304 章工期冲突");
    assert_eq!(find(CheckKind::TermConflict).section_id, "SEC-0405", "第 405 章术语冲突");
    assert_eq!(find(CheckKind::CommitmentConflict).section_id, "SEC-0456", "第 456 章承诺冲突");

    // 消息面：归一化值 + 冲突双方位置（可定位可复核）
    let amt = find(CheckKind::AmountConflict);
    assert!(amt.message.contains("投标保证金"), "消息含维度键: {}", amt.message);
    assert!(amt.message.contains("SEC-0001=500000"), "多数派首现+归一化值: {}", amt.message);
    assert!(amt.message.contains("SEC-0203=800000"), "少数派位置+归一化值: {}", amt.message);
    let sch = find(CheckKind::ScheduleConflict);
    assert!(sch.message.contains("SEC-0002=60") && sch.message.contains("SEC-0304=90"), "工期归一化天数双方: {}", sch.message);
    let term = find(CheckKind::TermConflict);
    assert!(term.message.contains("SEC-0003=国际标准化组织") && term.message.contains("SEC-0405=国际化标准组织"), "术语全称双方: {}", term.message);
    let com = find(CheckKind::CommitmentConflict);
    assert!(com.message.contains("SEC-0004=7x24") && com.message.contains("SEC-0456=5x8"), "承诺归一值双方: {}", com.message);

    // 编排证据面：500 轮转特征 + 4 植入 = 504（特征侧对账——无误抽）；500/25 = 20 片
    assert_eq!(outcome.feature_count, 504, "特征数 = 500 轮转 + 4 植入");
    assert_eq!(outcome.peak_shards, 20, "500 章按 25 一片 = 20 片（线程上限 8）");
}

// ---------------------------------------------------------------- 2. 局部纯度

/// 同一章单独查与批查结果全等（局部零跨章依赖的证明）+ 格式偏差 Warning 面
/// （偏差 > 50% 报 Warning；target=None 不查）。
#[test]
fn local_checks_are_pure_per_chapter() {
    let refs: HashSet<String> = ["REQ-0001".to_string()].into_iter().collect();
    // 三章定制夹具：干净达标章 / 占位符+超偏差章 / 悬空引用且无配额章
    let filler = "本节围绕实施方案与现场条件展开论述，逐项明确人员配置、质量保障、安全文明施工等要求，确保各项措施责任到人、落地可行。";
    let mut clean = String::new();
    while clean.chars().count() < TARGET {
        let need = TARGET - clean.chars().count();
        clean.extend(filler.chars().take(need));
    }
    let mut deviant = String::from("此处待补充。");
    while deviant.chars().count() < 800 {
        let need = 800 - deviant.chars().count();
        deviant.extend(filler.chars().take(need));
    }
    let corpus = vec![
        OwnedChapter { section_id: "SEC-0001".into(), content: clean.clone(), citations: vec!["REQ-0001".into()], target_chars: Some(TARGET as u32) },
        OwnedChapter { section_id: "SEC-0002".into(), content: deviant, citations: vec![], target_chars: Some(TARGET as u32) },
        OwnedChapter { section_id: "SEC-0003".into(), content: clean, citations: vec!["REQ-8888".into()], target_chars: None },
    ];
    let inputs = inputs_of(&corpus);
    let outcome = run_review(&inputs, &refs);

    // 纯度面：每章单独 check_local 的输出与批查（run_review）中该章的局部发现全等
    for ch in &corpus {
        let solo = check_local(&as_input(ch), &refs);
        let mut batch: Vec<CheckFinding> = outcome
            .findings
            .iter()
            .filter(|f| {
                f.section_id == ch.section_id
                    && matches!(f.kind, CheckKind::Placeholder | CheckKind::DanglingRef | CheckKind::FormatDeviation)
            })
            .cloned()
            .collect();
        batch.sort_by_key(sort_key);
        let mut solo_sorted = solo;
        solo_sorted.sort_by_key(sort_key);
        assert_eq!(solo_sorted, batch, "章 {} 单查与批查全等（局部零跨章依赖）", ch.section_id);
    }

    // Warning 面：全语料唯一 Warning 是第 2 章格式偏差（800/2000 偏差 60% > 50%）
    let warnings: Vec<&CheckFinding> = outcome.findings.iter().filter(|f| f.severity == CheckSeverity::Warning).collect();
    assert_eq!(warnings.len(), 1, "恰一条 Warning: {warnings:?}");
    assert_eq!(warnings[0].kind, CheckKind::FormatDeviation);
    assert_eq!(warnings[0].section_id, "SEC-0002");
    assert!(warnings[0].message.contains("800") && warnings[0].message.contains("2000"), "消息含实际与配额: {}", warnings[0].message);

    // None 面不查：第 3 章无配额 → 无格式偏差发现（即使正文同为 2000 字也无从比较）
    assert!(
        outcome.findings.iter().all(|f| !(f.section_id == "SEC-0003" && f.kind == CheckKind::FormatDeviation)),
        "target=None 不做格式检查"
    );

    // 干净达标章零发现
    assert!(outcome.findings.iter().all(|f| f.section_id != "SEC-0001"), "干净章零发现");
}

// ---------------------------------------------------------------- 3. 全局两位置门槛

/// 单章内同维度两值（章内自冲突）也检出；只出现一次的值（无第二取值）不报。
#[test]
fn global_conflict_requires_two_positions() {
    // 章内自冲突：金额同章两值；响应时间同章两值
    let a = mk_bare("SEC-0001", "投标保证金50万元，开户后缴纳。工期无关句。投标保证金80万元，验收后退还。");
    let b = mk_bare("SEC-0002", "故障响应时间2小时；重大故障响应时间4小时。");
    // 单一取值：项目预算/服务费各只出现一个值 → 不报（只出现一次的值不是冲突）
    let c = mk_bare("SEC-0003", "项目预算：100万元。服务费：10万元。");
    let refs: HashSet<String> = HashSet::new();
    let outcome = run_review(&inputs_of(&[a, b, c]), &refs);

    assert_eq!(outcome.findings.len(), 2, "恰两处章内自冲突，单一取值不报: {:#?}", outcome.findings);
    let amt = outcome.findings.iter().find(|f| f.kind == CheckKind::AmountConflict).unwrap();
    assert_eq!(amt.section_id, "SEC-0001");
    assert!(amt.message.contains("SEC-0001=500000") && amt.message.contains("SEC-0001=800000"), "同章双位置双值: {}", amt.message);
    let com = outcome.findings.iter().find(|f| f.kind == CheckKind::CommitmentConflict).unwrap();
    assert_eq!(com.section_id, "SEC-0002");
    assert!(com.message.contains("2小时") && com.message.contains("4小时"), "响应时间归一值: {}", com.message);
}

// ---------------------------------------------------------------- 4. 归一化口径

/// 归一化验收：「3,500万元」≡「35000000元」（同值不冲突）；「2个月」≡「60天」；
/// 「50万」≡「500000」；真不同才报。不同写法对面分开跑——面内同值、面间真不同。
#[test]
fn normalization_amount_and_schedule() {
    let refs: HashSet<String> = HashSet::new();

    // 面一：3,500万元 ≡ 35000000元（逗号剥离 + 万 ×10000）
    let pair1 = vec![mk_bare("SEC-0001", "投标保证金3,500万元，银行保函。"), mk_bare("SEC-0002", "投标保证金35000000元，银行保函。")];
    assert_eq!(run_review(&inputs_of(&pair1), &refs).findings.len(), 0, "同值不同写法零冲突");
    let f1 = extract_features(&as_input(&pair1[0]));
    let f2 = extract_features(&as_input(&pair1[1]));
    assert_eq!(f1[0].value, "35000000", "3,500万 → 35000000");
    assert_eq!(f2[0].value, "35000000", "35000000元 → 35000000");

    // 面二：2个月 ≡ 60天（30 天口径）
    let pair2 = vec![mk_bare("SEC-0001", "工期2个月完成部署。"), mk_bare("SEC-0002", "工期60天完成部署。")];
    assert_eq!(run_review(&inputs_of(&pair2), &refs).findings.len(), 0, "同值不同单位零冲突");
    let f3 = extract_features(&as_input(&pair2[0]));
    let f4 = extract_features(&as_input(&pair2[1]));
    assert_eq!(f3[0].value, "60", "2个月 → 60 天");
    assert_eq!(f4[0].value, "60", "60天 → 60 天");

    // 面三：50万 ≡ 500000（裸数字口径）
    let pair3 = vec![mk_bare("SEC-0001", "投标保证金50万。"), mk_bare("SEC-0002", "投标保证金500000。")];
    assert_eq!(run_review(&inputs_of(&pair3), &refs).findings.len(), 0, "裸数字与万写法同值零冲突");

    // 面四（真不同才报）：50万 vs 60万 → 金额冲突
    let pair4 = vec![mk_bare("SEC-0001", "投标保证金50万元。"), mk_bare("SEC-0002", "投标保证金60万元。")];
    let out4 = run_review(&inputs_of(&pair4), &refs);
    assert_eq!(out4.findings.len(), 1, "真不同必报");
    assert_eq!(out4.findings[0].kind, CheckKind::AmountConflict);
    assert!(out4.findings[0].message.contains("500000") && out4.findings[0].message.contains("600000"), "消息含双方归一值: {}", out4.findings[0].message);
}

// ---------------------------------------------------------------- 5. 确定性（R-15）

/// run_review（并行分片）与逐章串行收集 + 同口径排序的结果逐字节全等；
/// 同输入连跑 5 次全等（R-15 可重放——分片认领顺序不进结果面）。
#[test]
fn determinism_parallel_vs_sequential() {
    let (corpus, refs) = acceptance_corpus();
    let inputs = inputs_of(&corpus);

    // 串行参考：逐章局部 + 全量特征 + 全局比对——与并行分片合流的两阶段同构
    let mut seq: Vec<CheckFinding> = Vec::new();
    let mut feats: Vec<FeatureTuple> = Vec::new();
    for ch in &inputs {
        seq.extend(check_local(ch, &refs));
        feats.extend(extract_features(ch));
    }
    seq.extend(check_global(&feats));
    seq.sort_by_key(sort_key);

    let outcome: ReviewOutcome = run_review(&inputs, &refs);
    assert_eq!(outcome.findings, seq, "并行分片与串行收集逐字节全等");

    // 同输入 5 连跑——同发现集（线程认领顺序的任意性被合并排序吸收）
    let first = outcome.findings.clone();
    for run in 1..=5 {
        assert_eq!(run_review(&inputs, &refs).findings, first, "第 {run} 次重跑与首次全等");
    }
}

// ---------------------------------------------------------------- 6. 特征元组内存纪律

/// run_review 内部只持有特征元组（结构断言）：对 500×2000 字输入，特征集合
/// 序列化后 < 100KB——不保留原文的数字证据（正文面是 MB 序）。
#[test]
fn feature_only_memory_discipline() {
    let (corpus, refs) = acceptance_corpus();
    let inputs = inputs_of(&corpus);
    let outcome = run_review(&inputs, &refs);

    assert!(outcome.feature_count > 0, "特征面非空（内存纪律的前提是有东西可攒）");
    assert_eq!(outcome.feature_count, 504, "特征数与验收测试对账（500 轮转 + 4 植入）");

    // 特征集合序列化（与 run_review 内部同源：逐章 extract_features 的合集）
    let mut all: Vec<FeatureTuple> = Vec::new();
    for ch in &inputs {
        all.extend(extract_features(ch));
    }
    assert_eq!(all.len(), outcome.feature_count, "串行特征合计与并行分片合流一致");
    let json = serde_json::to_string(&all).unwrap();
    let content_bytes: usize = corpus.iter().map(|c| c.content.len()).sum();
    eprintln!(
        "[S-5 内存纪律] 特征序列化 {}B / 正文 {}B（{:.2}%）",
        json.len(),
        content_bytes,
        json.len() as f64 * 100.0 / content_bytes as f64
    );
    assert!(json.len() < 100_000, "特征集合序列化 {}B ≥ 100KB——违反不保留原文的结构纪律", json.len());
    assert!(
        (json.len() as f64) < content_bytes as f64 / 10.0,
        "特征集合应比正文低一个量级以上（结构保证，非调参结果）"
    );
}

// ---------------------------------------------------------------- 7. 内存峰值硬标准

/// 规格硬标准：500 章 × 2000 字（≈100 万字）跑完，进程峰值 RSS（VmHWM）
/// < 200MB（204800kB，MiB 口径）。VmHWM 是进程历史峰值（含测试 harness 自身
/// 与同进程其他测试的驻留）——用绝对值口径（规格原文「内存峰值 < 200MB」），
/// 夹具控制在最小占用（无 db.json 全量快照等无关大对象）；基线先行记录，
/// 增量面一并打印供归因。非 Linux：SKIP 横幅（issue #9 纪律——不伪装通过）。
#[test]
fn acceptance_peak_memory_under_200mb() {
    let Some(baseline) = vmhwm_kb() else {
        eprintln!("[SKIP] ENV_MISSING: /proc/self/status——非 Linux 平台无 VmHWM 口径");
        return;
    };
    let (corpus, refs) = acceptance_corpus();
    let inputs = inputs_of(&corpus);
    let outcome = run_review(&inputs, &refs);

    // 夹具与验收测试同构：6 处植入全检出（跑通面与正确面同判）
    assert_eq!(outcome.findings.len(), 6, "内存验收夹具与验收测试同构——6 处植入全检出");

    let peak = vmhwm_kb().unwrap_or(baseline);
    eprintln!(
        "[S-5 内存验收] 基线 VmHWM={baseline}kB → 跑后 VmHWM={peak}kB（阈值 204800kB / 200MB，增量 {}kB）",
        peak.saturating_sub(baseline)
    );
    assert!(peak < 204_800, "内存峰值 {peak}kB ≥ 200MB（204800kB）——违反 S-5 规格硬标准");
}

// ---------------------------------------------------------------- 8. 规模门（borrow：mdcore r11_stress）

/// 1000 章规模跑通（borrow：mdcore `r11_stress.rs` 的压力测试结构——大合成
/// 语料 + 完成性断言，不做脆弱的精确时间断言）：植入 1 处金额冲突必检出、
/// 无误报、无 panic；分片数随规模伸缩（1000/25 = 40 片）而线程上限恒 8。
#[test]
fn large_corpus_scale_gate() {
    let refs: HashSet<String> = (1..=400).map(|i| format!("REQ-{i:04}")).collect();
    let mut corpus: Vec<OwnedChapter> = (0..1000).map(|i| mk_chapter(i, "")).collect();
    corpus[776] = mk_chapter(776, "投标保证金99万元，补充说明。"); // 第 777 章：植入 1 处金额冲突
    let outcome = run_review(&inputs_of(&corpus), &refs);

    assert_eq!(outcome.findings.len(), 1, "1000 章规模：恰 1 处发现（植入必检出 + 无误报）: {:#?}", outcome.findings);
    let f = &outcome.findings[0];
    assert_eq!(f.kind, CheckKind::AmountConflict);
    assert_eq!(f.section_id, "SEC-0777", "少数派锚点直指植入章");
    assert_eq!(f.severity, CheckSeverity::Blocker);
    assert_eq!(outcome.peak_shards, 40, "1000/25 = 40 片（线程并发上限 8 不变）");
    assert_eq!(outcome.feature_count, 1001, "1000 轮转 + 1 植入");
}
