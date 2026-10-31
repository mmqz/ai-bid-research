//! quality_duplication.rs —— B3 维度 3：跨章节重复度（issue #97）。
//!
//! 验收线（issue #97 §B3 验收）：
//! - 故意复制一段 → 检测出 > 0.8（Jaccard 口径）；
//! - 手工 vs 引擎对照：句集基数、Jaccard、包含率逐位核对；
//! - Jaccard 盲区实证：子集复制（B 章每句都出自更长的 A 章）Jaccard 被
//!   长度差稀释 < 0.8，包含率如实暴露 → 补盲面的存在依据。

use bid_tools::quality::{detect_duplication, DuplicationReport};

/// 独立章节甲（8 句内容句——大坝施工面）。
fn section_a() -> String {
    [
        "本节阐述大坝坝体加固的施工方法与工艺流程。",
        "坝顶全长 1850 米，加固厚度 0.6 米，分三个仓面流水施工。",
        "混凝土浇筑总量 1.2 万立方米，采用二级配泵送混凝土。",
        "防渗墙最大处理深度 42 米，槽段搭接厚度 0.8 米。",
        "施工围堰按 20 年一遇洪水标准设计，堰顶高程 1045 米。",
        "溢洪道进口闸孔 3 孔，单孔净宽 8 米，设计泄量 1200 立方米每秒。",
        "主要施工设备 45 台套，含振动沉模机 2 台、混凝土拌合站 2 座。",
        "同类工程业绩见 [EV-0001] 至 [EV-0005]，均为近 5 年完工项目。",
    ]
    .join("。")
    + "。"
}

/// 独立章节乙（8 句——与甲零交集的机电安装面）。
fn section_b() -> String {
    [
        "本节阐述机电安装工程的施工组织安排。",
        "桥式起重机 2 台，单台起重量 16 吨，轨顶高程 1032 米。",
        "闸门启闭机 6 台套，行程 12 米，启闭速度 0.5 米每分钟。",
        "水轮发电机组 3 台，单机容量 800 千瓦，额定电压 10 千伏。",
        "电气主接线采用单母线分段，出线 4 回。",
        "计算机监控系统按无人值守设计，现地控制单元 8 套。",
        "电缆敷设总量约 35 公里，其中动力电缆 12 公里。",
        "机电设备安装工期 90 日历天，与土建交叉作业。",
    ]
    .join("。")
    + "。"
}

// ---------------------------------------------------------------- 验收线

#[test]
fn verbatim_copy_detected_above_0_8() {
    // issue 验收：故意复制一段 → >0.8（实测 Jaccard = 1.0）
    let sections = vec![
        ("SEC-A".to_string(), section_a()),
        ("SEC-A-COPY".to_string(), section_a()),
    ];
    let r = detect_duplication(&sections);
    assert_eq!(r.severe, 1, "整章复制 = 严重重复：{r:?}");
    assert_eq!(r.pairs.len(), 1);
    let (a, b, j) = &r.pairs[0];
    assert_eq!((a.as_str(), b.as_str()), ("SEC-A", "SEC-A-COPY"));
    assert!(*j > 0.8, "验收线 > 0.8（实测 Jaccard = {j}）");
    assert!(*j - 1.0 < 1e-9, "整章复制 Jaccard = 1.0");
    assert_eq!(r.contained[0].2, 1.0, "包含率同为 1.0");
}

#[test]
fn distinct_sections_report_nothing() {
    let sections = vec![
        ("SEC-A".to_string(), section_a()),
        ("SEC-B".to_string(), section_b()),
    ];
    let r = detect_duplication(&sections);
    assert_eq!(r.severe, 0, "零交集不报：{r:?}");
    assert!(r.pairs.is_empty());
    assert!(r.contained.is_empty());
}

// ---------------------------------------------------------------- 手工对照

#[test]
fn hand_computed_subset_copy_matches_engine() {
    // 手工构造：A = 4 句，B = A 的前 2 句原样 + 2 句新句
    // 手算：|A|=4, |B|=4, |A∩B|=2
    //   Jaccard = 2/(4+4-2) = 2/6 = 0.3333
    //   包含率 = 2/min(4,4) = 0.5
    //   严重判定：0.3333 < 0.8 且 0.5 < 0.9 → 非严重（如实：半数句子
    //   撞车值得报告，但不到「整段复制」的严重线）
    let a = "坝顶全长 1850 米，加固厚度 0.6 米。混凝土浇筑总量 1.2 万立方米。\
             防渗墙最大处理深度 42 米。围堰按 20 年一遇洪水标准设计。";
    let b = "坝顶全长 1850 米，加固厚度 0.6 米。混凝土浇筑总量 1.2 万立方米。\
             机电安装工期 90 日历天。单机容量 800 千瓦。";
    let r = detect_duplication(&[
        ("SEC-A".to_string(), a.to_string()),
        ("SEC-B".to_string(), b.to_string()),
    ]);
    assert_eq!(r.severe, 0, "半数撞车非严重：{r:?}");
    assert_eq!(r.pairs.len(), 1);
    assert!((r.pairs[0].2 - 2.0 / 6.0).abs() < 1e-4, "Jaccard 手算 = 引擎（2/6）");
    assert!((r.contained[0].2 - 0.5).abs() < 1e-4, "包含率手算 = 引擎（0.5）");
}

#[test]
fn subset_copy_is_jaccard_blind_spot_containment_catches() {
    // Jaccard 盲区实证（DUP_SEVERE_CONTAINMENT doc 的数值例）：
    // B 章 4 句全部出自 A 章 8 句（B 是 A 的真子集）
    // 手算：|A|=8, |B|=4, |A∩B|=4
    //   Jaccard = 4/8 = 0.5（< 0.8——被长度差稀释，issue 单一口径会漏报）
    //   包含率 = 4/4 = 1.0（> 0.9——如实暴露「B 整段抄自 A」）→ 严重
    let a = section_a();
    let sentences: Vec<&str> = a
        .split('。')
        .filter(|s| !s.trim().is_empty())
        .collect();
    assert_eq!(sentences.len(), 8, "夹具前提：A 章 8 句");
    let b: String = sentences[..4].join("。") + "。";
    let r = detect_duplication(&[
        ("SEC-A".to_string(), a),
        ("SEC-B".to_string(), b),
    ]);
    assert_eq!(r.severe, 1, "子集复制 = 严重（包含率口径）：{r:?}");
    let jaccard = r.pairs.iter().find(|p| p.1 == "SEC-B").unwrap().2;
    let cont = r.contained.iter().find(|p| p.1 == "SEC-B").unwrap().2;
    assert!((jaccard - 0.5).abs() < 1e-4, "Jaccard 被稀释至 0.5（盲区实证）");
    assert!((cont - 1.0).abs() < 1e-4, "包含率 1.0 暴露整段抄");
}

// ---------------------------------------------------------------- 归一化面

#[test]
fn renumbered_copy_is_still_detected() {
    // 整段复制后重新编号（最常见的改动）：1./2./3. ↔ （一）（二）（三）
    let a = "1. 我方针对本条要求作出完全响应并无任何偏离。\n\
             2. 施工组织设计按招标文件规定的格式与顺序编制。\n\
             3. 项目管理机构配置满足并优于招标文件要求。";
    let b = "（一）我方针对本条要求作出完全响应并无任何偏离。\n\
             （二）施工组织设计按招标文件规定的格式与顺序编制。\n\
             （三）项目管理机构配置满足并优于招标文件要求。";
    let r = detect_duplication(&[
        ("SEC-A".to_string(), a.to_string()),
        ("SEC-B".to_string(), b.to_string()),
    ]);
    assert_eq!(r.severe, 1, "重编号不改内容句同一性：{r:?}");
    assert!((r.pairs[0].2 - 1.0).abs() < 1e-4, "归一化后 Jaccard = 1.0");
}

#[test]
fn whitespace_reformat_is_still_detected() {
    // 排版重排（加空格）不影响内容句同一性
    let a = "本项目混凝土浇筑总量 1.2 万立方米，分三个仓面流水施工。工期 180 日历天。";
    let b = "本项目混凝土浇筑总量1.2万立方米，分三个仓面流水施工。工期180日历天。";
    let r = detect_duplication(&[
        ("SEC-A".to_string(), a.to_string()),
        ("SEC-B".to_string(), b.to_string()),
    ]);
    assert_eq!(r.severe, 1, "空白差异归一：{r:?}");
}

#[test]
fn short_structural_phrases_do_not_count() {
    // 两章共享的只有结构性短短语（标题/引导句）——不算重复
    let a = "详见下表。\n本项目混凝土浇筑总量 1.2 万立方米，工期 180 日历天。";
    let b = "详见下表。\n机电安装工期 90 日历天，单机容量 800 千瓦。";
    let r = detect_duplication(&[
        ("SEC-A".to_string(), a.to_string()),
        ("SEC-B".to_string(), b.to_string()),
    ]);
    assert_eq!(r.severe, 0);
    assert!(r.pairs.is_empty(), "短短语不入集：{r:?}");
}

// ---------------------------------------------------------------- 边界面

#[test]
fn empty_sections_report_nothing() {
    let r = detect_duplication(&[
        ("SEC-A".to_string(), String::new()),
        ("SEC-B".to_string(), "   \n  ".to_string()),
    ]);
    assert_eq!(r.severe, 0);
    assert!(r.pairs.is_empty() && r.contained.is_empty());
}

#[test]
fn within_section_repetition_does_not_self_inflate() {
    // 同章内句子重复（mock 填充句循环）——集合语义自然去重，
    // 不产生「与自己重复」的报告（跨章比对，无自配对）
    let mut s = String::new();
    for _ in 0..5 {
        s.push_str("我司高度重视本项目建设，将组建优秀团队确保履约。");
    }
    let r = detect_duplication(&[("SEC-A".to_string(), s)]);
    assert_eq!(r.severe, 0);
    assert!(r.pairs.is_empty());
}

#[test]
fn three_sections_pairwise_all_reported_desc_order() {
    // 三章：A 与 B 全同；C = A 的前 4 句 + 2 句新句
    // 手算：|A|=|B|=8, |C|=6, |A∩C|=4 → A-C 的 Jaccard = 4/10 = 0.4（报告
    // 但非严重）；A-B = 1.0（严重）——两两报告，降序（最严重排最前）
    let a = section_a();
    let c = format!(
        "{}。\n机电安装工期 90 日历天，单机容量 800 千瓦。电缆敷设总量约 35 公里。",
        a.split('。').filter(|s| !s.trim().is_empty()).take(4)
            .collect::<Vec<_>>().join("。")
    );
    let r = detect_duplication(&[
        ("SEC-A".to_string(), a.clone()),
        ("SEC-B".to_string(), a),
        ("SEC-C".to_string(), c),
    ]);
    assert_eq!(r.severe, 1, "只有全同对严重：{r:?}");
    assert_eq!(r.pairs.len(), 3, "两两全覆盖：{:?}", r.pairs);
    let ac = r.pairs.iter().find(|p| (p.0 == "SEC-A" && p.1 == "SEC-C")
        || (p.0 == "SEC-C" && p.1 == "SEC-A")).unwrap();
    assert!((ac.2 - 0.4).abs() < 1e-4, "A-C Jaccard = 4/10（实测 {:.4}）", ac.2);
    // 降序：首对 = 全同对（报告的行动优先级——最严重排最前）
    assert_eq!(r.pairs[0].2, r.pairs.iter().map(|p| p.2).fold(f64::MIN, f64::max));
}

#[test]
fn duplication_is_deterministic() {
    let mk = || {
        vec![
            ("SEC-A".to_string(), section_a()),
            ("SEC-B".to_string(), section_b()),
            ("SEC-C".to_string(), section_a()),
        ]
    };
    let r1: DuplicationReport = detect_duplication(&mk());
    let r2 = detect_duplication(&mk());
    assert_eq!(r1, r2, "纯内容函数——同输入同输出（HashSet 迭代序不影响：输出前排序）");
}
