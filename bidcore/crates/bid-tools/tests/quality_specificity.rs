//! quality_specificity.rs —— B3 维度 1：针对性（issue #97）。
//!
//! 验收线（issue #97 §B3 验收）：
//! - 好章节 specificity > 0.7（真项目名 + ≥6 处具体参数 + 低套话）；
//! - 凑数/照抄模板章节 < 0.4（占位符「XX 项目」+ 套话堆砌 + 无参数）；
//! - 真实文本锚定：政府示范文本（北京水利 2025）的投标函功能句**不是**
//!   空话（有数字锚——锚规则压假阳性的实证）。
//!
//! 夹具诚实边界：本仓语料为**招标侧**（real-corpus 全为招标文件），无真实
//! 投标正文；好/差章节以真实项目名（real-corpus/water 塔里木水库除险加固、
//! 伊犁水库质量检测——PROVENANCE.md 登记的真实政府公开项目）构造，套话
//! 短语表的真实语料实测频次见 quality.rs EMPTY_PHRASES 逐条注释。

use bid_tools::quality::{assess_specificity, ProjectContext};

/// 真实项目上下文（项目名 = real-corpus/water 真实公开项目；招标人名 =
/// 测试注入值——store 无此字段，验证机制面）。
fn water_project() -> ProjectContext {
    ProjectContext {
        project_name: "塔里木水库除险加固".into(),
        tenderer_name: "塔里木河流域管理局".into(),
        extra_terms: vec!["SL-2026-018".into()],
    }
}

/// 好章节：真项目名 + 招标人名 + 6 处数字+单位参数 + 证据引用。
fn good_section() -> String {
    let mut s = String::from(
        "# 除险加固施工方案\n\n\
        本方案针对塔里木水库除险加固项目编制，招标人为塔里木河流域管理局，\
        依据招标文件及技术规范书逐条响应。\n\n\
        大坝坝体加固：坝顶全长 1850 米，加固厚度 0.6 米，混凝土浇筑总量 1.2 万立方米，\
        施工工期 180 日历天。防渗墙处理深度 42 米，成墙面积 8600 平方米，\
        槽段搭接厚度 0.8 米。\n\n\
        溢洪道改造：进口闸孔 3 孔，单孔净宽 8 米，设计泄量 1200 立方米每秒，\
        闸门启闭机 2 台套。施工围堰按 20 年一遇洪水标准设计，堰顶高程 1045 米。\n\n\
        同类业绩与资质证明见 [EV-0001] 至 [EV-0005]，均为近 5 年内完工项目。\
        质量目标：单位工程合格率 100%，优良率 85% 以上，土建缺陷责任期 1 年。",
    );
    s.push_str("\n\n本工程实施期间拟投入管理人员 28 人，其中一级注册建造师 3 名；\
        主要施工设备 45 台套，含振动沉模机 2 台、混凝土拌合站 2 座。");
    s
}

/// 凑数章节（照抄模板形态）：占位符「XX」+ 套话堆砌 + 零参数零引用。
fn filler_section() -> String {
    let mut s = String::from(
        "# 施工组织设计\n\n\
        我司高度重视本项目的建设工作，将本着诚信原则，组建经验丰富的精英团队，\
        秉持先进理念，科学管理，精心组织，全力以赴确保工程顺利实施。\n\n\
        我公司将秉承客户至上的服务宗旨，提供一流的服务和一流的管理，\
        自始至终保持优质高效的施工状态，圆满完成各项建设任务。\n\n\
        我公司已建立健全质量保证体系，管理制度切实可行，严格执行招标文件的\
        各项要求，确保万无一失，圆满履约。",
    );
    while s.chars().count() < 600 {
        s.push_str("\n\n我司将秉承科学合理的管理思路，深刻理解招标人的意图，\
            积极响应各项号召，雄厚的技术力量是本工程顺利实施的坚强保障。");
    }
    s
}

/// 政府示范文本（北京水利 2025）投标函真实句——有数字锚，非空话。
const REAL_BID_LETTER: &str = "我方已仔细研究招标文件的全部内容，愿意以人民币（大写）\
壹仟万元（¥10000000 元）的投标总报价，工期 180 日历天，按合同约定实施和完成承包工程，\
修补工程中的任何缺陷。我方承诺投标有效期为自投标截止日起 90 天。";

// ---------------------------------------------------------------- 验收线

#[test]
fn good_section_scores_above_0_7() {
    let sp = assess_specificity(&good_section(), &water_project());
    assert!(sp.has_project_name, "含真项目名：{sp:?}");
    assert!(sp.has_tenderer_name, "含招标人名：{sp:?}");
    assert!(sp.concrete_params >= 6, "具体参数 ≥6 处：{sp:?}");
    assert!(sp.boilerplate_ratio < 0.20, "套话占比 <20%：{sp:?}");
    assert!(
        sp.score > 0.7,
        "验收线：好章节 specificity > 0.7（实测 {:.3}）",
        sp.score
    );
}

#[test]
fn filler_section_scores_below_0_4() {
    let sp = assess_specificity(&filler_section(), &water_project());
    assert!(!sp.has_project_name, "无项目名（占位符不算）：{sp:?}");
    assert!(!sp.has_tenderer_name, "无招标人名：{sp:?}");
    assert_eq!(sp.concrete_params, 0, "零参数：{sp:?}");
    assert!(sp.boilerplate_ratio > 0.5, "套话占比过半：{sp:?}");
    assert!(
        sp.score < 0.4,
        "验收线：凑数章节 specificity < 0.4（实测 {:.3}）",
        sp.score
    );
}

#[test]
fn template_copy_with_real_params_still_flagged_on_names() {
    // 照抄模板的典型形态：参数是从别的项目抄来的（有数字），
    // 但项目名/招标人名忘了替换 → 专名面全失
    let content = "# 施工方案\n\n本工程总投资 3.5 亿元，工期 540 日历天。\
        我司高度重视，将全力以赴，科学管理，精心组织。";
    let sp = assess_specificity(content, &water_project());
    assert!(!sp.has_project_name && !sp.has_tenderer_name);
    // 有 2 处参数（<3 → 0 分）+ 套话 50% 左右 → 总分低
    assert!(sp.score < 0.4, "抄参数不抄专名仍是低针对性：{sp:?}");
}

// ---------------------------------------------------------------- 权重面

#[test]
fn unknown_tenderer_normalizes_instead_of_capping() {
    // 调整①：招标人名未知 → 0.2 权重退出分母（满分仍可达 1.0，
    // 而非被压到 0.8 的假上限）
    let project = ProjectContext {
        project_name: "塔里木水库除险加固".into(),
        tenderer_name: String::new(), // 未知——BidProject 无此字段的真实面
        extra_terms: vec![],
    };
    let sp = assess_specificity(&good_section(), &project);
    assert!(!sp.has_tenderer_name);
    assert!(
        sp.score > 0.7,
        "归一后好章节仍过验收线（实测 {:.3}）",
        sp.score
    );
    // 满配对照：项目名 + 参数满额 + 无套话 → 0.3+0.3+0.2 = 0.8/0.8 = 1.0
    let perfect = "塔里木水库除险加固工程坝顶全长 1850 米，加固厚度 0.6 米，\
        混凝土 1.2 万立方米，工期 180 日历天，堰顶高程 1045 米，设备 45 台套，\
        人员 28 人。";
    let sp2 = assess_specificity(perfect, &project);
    assert!((sp2.score - 1.0).abs() < 1e-9, "满配 = 1.0（实测 {:.3}）", sp2.score);
}

#[test]
fn param_ladder_smooths_boundary() {
    // 调整②：3~5 处参数给半权（0.15），不是 0/0.3 二元跳变
    let project = ProjectContext {
        project_name: "甲项目".into(),
        tenderer_name: String::new(),
        extra_terms: vec![],
    };
    let base = "本章说明甲项目的实施安排。"; // 0 参数
    let mid = "甲项目工期 180 日历天，人员 28 人，设备 45 台套。"; // 3 参数
    let full = format!("{mid} 坝顶 1850 米，高程 1045 米，混凝土 1.2 万立方米。"); // 6 参数
    let s0 = assess_specificity(base, &project);
    let s1 = assess_specificity(mid, &project);
    let s2 = assess_specificity(&full, &project);
    assert_eq!(s0.concrete_params, 0);
    assert_eq!(s1.concrete_params, 3);
    assert_eq!(s2.concrete_params, 6);
    // 单调：0 < 3~5 < 6+（阶梯全距）
    assert!(s0.score < s1.score && s1.score < s2.score, "{s0:?} {s1:?} {s2:?}");
    // 半权档与满权档的差 ≈ 0.15/0.8 ≈ 0.1875（不是 0.375 的二元跳变）
    let gap = s2.score - s1.score;
    assert!((gap - 0.15 / 0.8).abs() < 0.02, "阶梯差 ≈ 0.1875（实测 {gap:.4}）");
}

#[test]
fn empty_content_scores_zero() {
    let sp = assess_specificity("", &water_project());
    assert_eq!(sp.score, 0.0, "空内容诚实 0 分（不白给无套话分）：{sp:?}");
    let sp2 = assess_specificity("   \n  ", &water_project());
    assert_eq!(sp2.score, 0.0);
}

// ---------------------------------------------------------------- 真实文本锚定

#[test]
fn real_bid_letter_sentences_are_not_empty() {
    // 政府示范文本的承诺句有数字锚（报价/工期/有效期）——锚规则压假阳性：
    // 句含「我方承诺」类词但可验证，不计空话 → 套话占比 0
    let sp = assess_specificity(REAL_BID_LETTER, &water_project());
    assert_eq!(
        sp.boilerplate_ratio, 0.0,
        "有锚承诺句不是空话（政府示范文本实证）：{sp:?}"
    );
    // 投标函无项目名/招标人名（抬头另列）→ 专名面失分是如实的
    assert!(!sp.has_project_name);
    assert!(sp.concrete_params >= 3, "报价/工期/有效期数字锚：{sp:?}");
}

#[test]
fn anchored_boilerplate_is_not_counted_twice() {
    // 同一句套话 + 数字锚 → 非空话句（针对性 boilerplate_ratio 不计）
    let content = "我司高度重视本工程质量，承诺缺陷责任期 2 年，\
        在 24 小时内响应质量问题。";
    let sp = assess_specificity(content, &water_project());
    assert_eq!(sp.boilerplate_ratio, 0.0, "有锚即非空话：{sp:?}");
    assert!(sp.concrete_params >= 2);
}

#[test]
fn specificity_is_deterministic() {
    let a = assess_specificity(&good_section(), &water_project());
    let b = assess_specificity(&good_section(), &water_project());
    assert_eq!(a, b, "纯内容函数——同输入同输出（可 diff 可重放）");
}
