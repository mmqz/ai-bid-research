//! 否决语义判定词表与规则（issue #35 P0-B——DOCX 分类信号源修复）。
//!
//! 背景：`extract_main_docx` 原只走章节上下文映射（`category_for`），
//! DOCX 组否决分类召回 0.0000——PDF 版 R3-S3-5 的「强否决短语 > 资格面 >
//! 章节上下文」优先级链在 DOCX 侧整体缺失。本模块把判定原语集中：
//!
//! - [`is_veto`]：强否决信号（处置短语/负面清单词 + 例外从句排除）
//! - [`is_qual`]：资格面（正面要求语境——不同于否决）
//! - [`opens_veto_context`]：否决语境开启句（「下列情形之一的，…否决：」）
//! - [`is_list_item`]：列举条目（「（N）…」/「N、…」——语境传播对象）
//! - [`is_placeholder`]：待填占位（□不采用□采用 等——排除面）
//!
//! 确定性纪律：零模型调用、纯词表/正则、同输入同输出。
//! 位置权重（issue #35 表格）：句尾处置动词最强（…，招标人有权拒收。），
//! 强短语任意位置命中即判（招标文件正文无句子级分词依赖）。

use regex::Regex;
use std::sync::OnceLock;

/// 强否决处置短语（判定动词面——正文提及"否决"的弱引用不算）。
/// 对齐 PDF 版 `re_veto` 并扩展（P0-B 面：missed 样例驱动）。
fn re_veto_strong() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "将被否决|否决其投标|否决该投标|否决投标处理|否决所有投标|否决条件|否决投标条件|",
            "视为无效|无效投标|按无效标处理|作无效标处理|认定为无效|",
            "取消其中标|取消中标资格|取消投标资格|取消其投标资格|",
            "没收其投标保证金|废标|",
            "拒收|不予受理|不予接收|不予接受|不予开标|不予评审|",
            "有权拒收|有权取消|有权否决",
        ))
        .unwrap()
    })
}

/// 资格负面清单词（负面情形列举——"有下列情形之一的"语境的子项信号，
/// 也可独立出现在审查表行）。正面资格要求（应具备/须提供）**不在**此面。
fn re_negative_list() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "骗取中标|骗取|严重违约|重大工程质量问题|重大安全事故|",
            "行政处罚|刑事处罚|行贿|受贿|",
            "失信|黑名单|不良行为|不良记录|",
            "弄虚作假|串通投标|相互串通|私下接触|",
            "不是本单位人员|非本单位人员|允许他人以本单位名义|",
            "被责令停业|被逐|合同被解除|违约或被逐",
        ))
        .unwrap()
    })
}

/// 否决语境开启句：冒号前的总括句（其后列举条目继承否决语义）。
fn re_veto_context_open() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "下列情形之一|下列行为之一|下列情况之一|下列情形|下列行为|",
            "视为允许他人以本单位名义|视为不具备承包能力|视为无效标|",
            "其投标将被否决|否决其投标|",
            "资格审查不合格|评审不合格的",
        ))
        .unwrap()
    })
}

/// 例外从句（排除面——「除不可抗力外」不是否决信号）。
/// 命中例外从句的片段不算否决：对整段文本，先剥离例外从句再判强词。
fn re_exception_clause() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"除[^，。；]{0,24}外(，|以外)?").unwrap())
}

/// 资格面（要求/证书/审查语境——正面要求，弱提及不算）。
fn re_qual() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            concat!(
                "资质等级|资格审查|资格后审|资格预审|资质证书|资质条件|",
                "营业执照|安全生产许可证|类似工程|项目负责人|项目总监|",
                "造价工程师|注册建造师|监理工程师|具备.*资质|须具备",
            ),
        )
        .unwrap()
    })
}

/// 待填占位（示范文本 □ 复选/下划线留白——不构成可执行否决条件）。
fn re_placeholder() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"□|_{3,}|…{2,}").unwrap())
}

/// 列举条目：「（N）」「（一）」「N、」「N.」开头（语境传播对象）。
fn re_list_item() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[（(]?[0-9一二三四五六七八九十]{1,3}[）)]|^[①②③④⑤⑥⑦⑧⑨⑩]|^[0-9]{1,2}[、.]")
            .unwrap()
    })
}

/// 剥离例外从句后的判定文本（「除…外」不参与强词匹配）。
fn strip_exceptions(text: &str) -> String {
    re_exception_clause().replace_all(text, "").into_owned()
}

/// 强否决判定：处置短语或负面清单词命中（例外从句剥离后）。
/// 正面资格要求（应具备资质/须提供报表）不会命中此面。
pub fn is_veto(text: &str) -> bool {
    let stripped = strip_exceptions(text);
    re_veto_strong().is_match(&stripped) || re_negative_list().is_match(&stripped)
}

/// 资格面判定（正面要求语境——分类为「资格条件」，非否决）。
pub fn is_qual(text: &str) -> bool {
    re_qual().is_match(text)
}

/// 否决语境开启句（父条款——其后列举条目继承否决语义）。
pub fn opens_veto_context(text: &str) -> bool {
    let stripped = strip_exceptions(text);
    re_veto_context_open().is_match(&stripped)
}

/// 列举条目（语境传播对象）。
/// 注意：多级条款号「3.5.2」不是列举条目——regex crate 无 look-ahead，
/// 数字分支用代码级判定（「N、」恒列举；「N.」后跟数字即条款号，排除）。
pub fn is_list_item(text: &str) -> bool {
    let t = text.trim_start();
    let chars: Vec<char> = t.chars().collect();
    let n_digits = chars.iter().take_while(|c| c.is_ascii_digit()).count();
    if n_digits > 0 && n_digits <= 2 {
        let sep = chars.get(n_digits);
        let after = chars.get(n_digits + 1);
        match (sep, after) {
            (Some('、'), _) => return true,                    // 「12、」
            (Some('.'), Some(c)) if !c.is_ascii_digit() => return true, // 「3.文」
            (Some('.'), None) => return true,                  // 「3.」结尾
            _ => {}
        }
        if n_digits > 0 && sep.is_none() {
            return false; // 纯数字行非列举
        }
        if matches!(sep, Some('.')) {
            return false; // 「3.5.2」条款号形态
        }
    }
    re_list_item().is_match(t)
}

/// 待填占位（排除面——不抽）。
pub fn is_placeholder(text: &str) -> bool {
    re_placeholder().is_match(text)
}


/// 表格审查语境开启词（评审标准表/资格审查表/初步审查表的特征行）。
/// 模板表（否决通知/澄清通知的「情况描述/依据」表头）**不**在此面。
fn re_review_context_open() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "评审标准|评审因素|资格审查|初步评审|形式评审|",
            "响应评审|审查表|评标办法",
        ))
        .unwrap()
    })
}

/// 表格审查语境开启判定（评审标准/资格审查表内行）。
pub fn opens_review_context(text: &str) -> bool {
    re_review_context_open().is_match(text)
}


/// 评审标准行的符合性句式（审查表语境内的行级过滤——参数行
/// 「投标截止时间/份数/交易服务费」无符合性语义，排除）。
fn re_conformance() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "应与|符合|相一致|一致|唯一|不存在|",
            "应为|须为|须具备|具备有效|满足",
        ))
        .unwrap()
    })
}

/// 评审标准行判定（审查表语境内：符合性句式 或 列举条目）。
pub fn is_review_standard(text: &str) -> bool {
    re_conformance().is_match(text) || is_list_item(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn veto_strong_phrases() {
        assert!(is_veto("3.4.2 投标人未提供有效营业执照的，其投标将被否决"));
        assert!(is_veto("未按要求密封的投标文件，招标人有权拒收"));
        assert!(is_veto("不包含运输费用的报价将被否决"));
        assert!(is_veto("10.5 招标人拒收投标文件的其他情形"));
        assert!(is_veto("（12）在最近三年内有骗取中标或严重违约或重大工程质量问题的。"));
        assert!(is_veto("（4）曾因在招标、评标以及其他与招标投标有关活动中从事违法行为而受过行政处罚或刑事处罚的；"));
    }

    #[test]
    fn exception_clause_not_veto() {
        // 例外从句内的处置词不算（防误报——issue #35 表格第 2 行）
        assert!(!is_veto("除不可抗力外，投标人应按期完成"));
        // 例外从句外仍有强词则命中
        assert!(is_veto("除不可抗力外，投标人不得延期，否则其投标将被否决"));
    }

    #[test]
    fn positive_qual_not_veto() {
        // 正面资格要求不命中否决面
        assert!(!is_veto("（1）资质条件：投标人应具备水利水电工程施工总承包贰级资质"));
        assert!(is_qual("（1）资质条件：投标人应具备水利水电工程施工总承包贰级资质"));
    }

    #[test]
    fn context_open_and_list() {
        assert!(opens_veto_context("有下列情形之一的，其投标将被否决："));
        assert!(opens_veto_context("9.2.2 下列行为，视为允许他人以本单位名义承揽工程："));
        assert!(is_list_item("（1）投标人的法定代表人的委托代理人不是投标人本单位人员；"));
        assert!(is_list_item("12、相互串通投标报价的；"));
        assert!(is_list_item("①投标文件中的大写金额与小写金额不一致的，以大写金额为准；"));
        assert!(!is_list_item("投标人须具备贰级资质"));
    }

    #[test]
    fn placeholder_excluded() {
        assert!(is_placeholder("□不接受\n□接受，应满足下列要求："));
        assert!(!is_placeholder("其投标将被否决"));
    }
}
