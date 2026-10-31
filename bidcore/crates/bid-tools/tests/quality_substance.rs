//! quality_substance.rs —— B3 维度 2：实质性（issue #97）。
//!
//! 验收线（issue #97 §B3 验收）：
//! - 空话章节 substance < 0.4（套话堆砌 + 零可验证陈述）；
//! - 好章节高得分（可验证陈述 ≥8 满分面）；
//! - 手工 vs 引擎对照：混合章节按公式逐位核对（评标纪律——不是「应该
//!   能行」，是手算数字与引擎数字一致）。
//!
//! 公式（quality.rs `assess_substance` doc）：
//! `score = (1 - 空话句占比) × 0.6 + min(可验证陈述数 / 8, 1) × 0.4`

use bid_tools::quality::{assess_substance, SubstanceScore, VERIFIABLE_CLAIMS_FULL};

/// 纯空话章节（B2 scoring.rs filler_section 同款句式——套话堆砌）。
fn empty_talk_section() -> String {
    let mut s = String::from(
        "我司高度重视本项目建设，将本着诚信原则组建优秀团队。\
        我公司将秉承客户至上宗旨，提供一流的服务。\
        我公司已建立健全各项管理制度，切实可行，严格执行招标文件要求。\
        我司将秉承科学管理理念，精心组织施工，全力以赴，确保万无一失。\
        我司经验丰富，技术力量雄厚，深刻理解招标人意图，积极响应各项要求。\
        自始至终保持优质高效状态，圆满完成本工程全部建设任务，圆满履约。",
    );
    while s.chars().count() < 500 {
        s.push_str("我司高度重视，科学管理，精心组织，优质高效，圆满完成各项任务。");
    }
    s
}

/// 好章节：句句可验证（数字+单位 / 标准编号 / REQ-EV 引用）。
fn substantial_section() -> String {
    [
        "本项目管理机构配置管理人员 28 人，其中一级注册建造师 3 名。",
        "质量管理体系通过 ISO9001 认证，环境管理体系通过 ISO14001 认证。",
        "施工总工期 180 日历天，计划开工日期 2026 年 5 月 1 日。",
        "大坝加固混凝土浇筑总量 1.2 万立方米，分 3 个仓面流水施工。",
        "防渗墙最大处理深度 42 米，槽段搭接厚度 0.8 米。",
        "围堰按 20 年一遇洪水标准设计，堰顶高程 1045 米。",
        "同类工程业绩见 [EV-0001] 至 [EV-0005]，均为近 5 年完工项目。",
        "针对 [REQ-0012] 的响应：缺陷责任期 2 年，24 小时内响应质量问题。",
        "主要施工设备 45 台套，含振动沉模机 2 台、混凝土拌合站 2 座。",
        "单位工程合格率目标 100%，优良率 85% 以上。",
    ]
    .join("。")
    + "。"
}

// ---------------------------------------------------------------- 验收线

#[test]
fn empty_talk_section_below_0_4() {
    let st = assess_substance(&empty_talk_section());
    assert!(
        st.empty_sentence_ratio > 0.9,
        "空话句占比应近满（实测 {:.3}）",
        st.empty_sentence_ratio
    );
    assert_eq!(st.verifiable_claims, 0, "零可验证陈述：{st:?}");
    assert!(
        st.score < 0.4,
        "验收线：空话章节 substance < 0.4（实测 {:.3}）",
        st.score
    );
}

#[test]
fn substantial_section_scores_high() {
    let st = assess_substance(&substantial_section());
    assert_eq!(st.empty_sentence_ratio, 0.0, "无空话句：{st:?}");
    assert!(
        st.verifiable_claims >= VERIFIABLE_CLAIMS_FULL,
        "可验证陈述 ≥8（实测 {}）",
        st.verifiable_claims
    );
    assert!(st.score > 0.9, "好章节接近满分（实测 {:.3}）", st.score);
}

// ---------------------------------------------------------------- 手工对照

#[test]
fn hand_computed_mixed_section_matches_engine() {
    // 手工构造：2 句——1 句空话（16 字）+ 1 句可验证（30 字）
    // 手算：占比 = 16/46 ≈ 0.3478；陈述 = 1；
    //       score = (1-0.3478)×0.6 + min(1/8,1)×0.4 = 0.3913 + 0.05 = 0.441
    let content = "我司高度重视本项目建设，全力以赴。\
                   本项目混凝土浇筑总量 1.2 万立方米，工期 180 日历天。";
    let empty_chars = "我司高度重视本项目建设，全力以赴".chars().count();
    let full_chars = "本项目混凝土浇筑总量 1.2 万立方米，工期 180 日历天".chars().count();
    let st = assess_substance(content);
    let expect_ratio =
        ((empty_chars as f64 / (empty_chars + full_chars) as f64) * 10_000.0).round() / 10_000.0;
    let expect_score =
        (((1.0 - expect_ratio) * 0.6 + (1.0 / 8.0) * 0.4) * 1000.0).round() / 1000.0;
    assert_eq!(st.empty_sentence_ratio, expect_ratio, "空话占比手工=引擎");
    assert_eq!(st.verifiable_claims, 1, "可验证陈述手工 = 引擎");
    assert_eq!(st.score, expect_score, "总分手工 = 引擎（0.441）");
    assert!((st.score - 0.441).abs() < 1e-9, "锚定值 0.441（实测 {:.3}）", st.score);
}

#[test]
fn claims_above_full_line_saturate_not_inflate() {
    // 12 句可验证（> 8 满分线）→ 陈述面饱和 0.4，不虚涨
    let mut content = String::new();
    for i in 0..12 {
        content.push_str(&format!("第{i} 项配置设备 {i} 台，人员 {i} 名。"));
    }
    let st = assess_substance(&content);
    assert_eq!(st.verifiable_claims, 12);
    assert!((st.score - 1.0).abs() < 1e-9, "饱和于 1.0（实测 {:.3}）", st.score);
}

#[test]
fn empty_content_scores_zero() {
    let st = assess_substance("");
    assert_eq!(st.score, 0.0, "空内容诚实 0 分：{st:?}");
    let st2 = assess_substance("  \n  ");
    assert_eq!(st2.score, 0.0);
}

// ---------------------------------------------------------------- 真实文本锚定

/// 政府示范文本（北京水利 2025）投标函真实句——报价/工期/有效期全带数字锚。
const REAL_BID_LETTER: &str = "我方已仔细研究招标文件的全部内容，愿意以人民币（大写）\
壹仟万元（¥10000000 元）的投标总报价，工期 180 日历天，按合同约定实施和完成承包工程，\
修补工程中的任何缺陷。我方承诺投标有效期为自投标截止日起 90 天。";

#[test]
fn real_bid_letter_is_substantial() {
    let st = assess_substance(REAL_BID_LETTER);
    assert_eq!(st.empty_sentence_ratio, 0.0, "真实投标函无空话句：{st:?}");
    assert!(st.verifiable_claims >= 2, "报价/工期句可验证：{st:?}");
    assert!(st.score > 0.5, "真实功能文本得分中上（实测 {:.3}）", st.score);
}

#[test]
fn substance_is_deterministic() {
    let a: SubstanceScore = assess_substance(&empty_talk_section());
    let b = assess_substance(&empty_talk_section());
    assert_eq!(a, b, "纯内容函数——同输入同输出");
}

#[test]
fn verifiable_claims_count_standard_ids() {
    // 标准编号形态（无数字+单位也能锚定）
    let content = "质量管理体系满足 GB/T19001 要求。信息安全满足 ISO27001。\
        施工规范执行 JGJ46。其余按招标文件执行。";
    let st = assess_substance(content);
    assert_eq!(st.verifiable_claims, 3, "GB/T19001 + ISO27001 + JGJ46：{st:?}");
}

#[test]
fn standard_id_no_space_cjk_form_still_matches() {
    // 中文紧邻无空格形态（常见写法）：「符合GB50205标准」「执行CJJ/T77规范」
    // ——rust regex 的 \b 在 CJK 语境失效，裸前缀匹配保住该形态（见
    // standard_id_regex 注释的取舍说明）
    let content = "本工程执行GB50205标准及CJJ/T77规范，验收按DB11/T1000执行。";
    let st = assess_substance(content);
    assert_eq!(st.verifiable_claims, 1, "单句——锚定一次即计一句：{st:?}");
    assert_eq!(st.empty_sentence_ratio, 0.0);
}
