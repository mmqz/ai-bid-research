//! 「规则说明」识别（#226 层 2）：区分**招标人的规则陈述**与**投标人的响应要求**。
//!
//! # 问题
//!
//! `re_veto` 是词表匹配（含「不予受理」「不接受联合体」等），它分不清：
//!
//! ```text
//! 3.2 本次招标不接受联合体投标。          ← 招标人声明规则（我方遵守即可）
//! 3.1 投标人须具备水利水电施工总承包一级资质。 ← 响应要求（我方要证明）
//! ```
//!
//! 两条都命中「否决/不接受」词面，但只有第二条该产生 blocking 的响应义务。
//! 第一条被判否决项 → 永远无法「响应」→ 章节永久不可起草（#226 死锁）。
//!
//! # 判据（保守优先——宁可不判，不可错判）
//!
//! 只在**同时满足**下面三条时才判为规则说明：
//!
//! 1. 命中**规则句式**：主语是招标人/招标代理机构/本次招标/本招标项目，
//!    或句式是「逾期…不予受理」这类程序后果条款；
//! 2. **不含**响应句式的强义务词（投标人应/须/必须/应当 + 动作）；
//! 3. **不含**资格证明材料诉求（资质证书/营业执照/业绩/财务…）——
//!    这类即使主语是「投标人」也必须保留为响应要求。
//!
//! 三条里第 3 条是安全网：`资格条件`类必须继续 blocking（GT 红线：
//! 资格类否决一旦漏判 = 废标风险），所以宁可漏判规则说明，不可吃掉资格要求。
//!
//! # 为什么单独成模块
//!
//! 判定逻辑必须能被**逐条断言**（真实语料的红线对照跑不动单测）。放独立
//! 模块后，规则句式/反例/边界都能直接写测试，接进 `extract.rs` 只改两行。

/// 规则说明判定（纯函数；`true` = 这是招标人的规则陈述，不是响应要求）。
///
/// 调用方只在 `veto_hit()` 已命中的段落上调用——本函数不负责任何否决判定，
/// 只回答「这条否决信号是不是规则说明」。
///
/// # 收紧过程（真实语料复核驱动，如实登记）
///
/// 第一版只用「规则主语 + 无投标人义务」两条，跑真实语料判出 86 条，
/// 人工逐条复核发现**判宽了 21 条**，其中包含 GT 明确标为否决的关键条目：
///
/// - 「（1）招标人在开标前开启投标文件…泄露给其他投标人」← 串通投标情形
/// - 「（4）与招标人存在利害关系…不得参加投标」← 资格性否决
/// - 「中标人无正当理由拒签合同的，招标人取消其中标资格」← 处置条款
/// - 「投标人递交的投标文件…均应当使用中文」← 投标人义务
///
/// 这些的共同点：**主语含「招标人/中标人」但句子讲的是「会有什么后果」**，
/// 或者干脆是被拆碎的列举条目（「（1）逾期送达的；」本身没有主语）。
/// 于是补两条硬排除 + 一条长度闸：
///
/// 3. **列举条目形态**（以（N）/N、/⑴ 开头）→ 不是规则说明——它是某个
///    总括句（「下列情形之一，其投标将被否决：」）的条目，语义由总括句定，
///    单独看永远看不出是不是规则；而总括句本身已被前面的分支处理。
/// 4. **约束他人行为**（泄露/串通/内定/不得参加/拒签/取消…）→ 不是规则说明；
///    这些是「谁做了会怎样」的处置/禁止条款，GT 一律标否决。
/// 5. **过短文本**（去空白后 < 8 字）→ 不是规则说明：真实规则陈述是完整句子，
///    短句多是抽取碎片（「1.1.2招标人」「1.5招标方式」），判定无意义。
pub fn is_rule_statement(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    // ── 闸 5：抽取碎片不判 ──
    // 先剥掉条款号前缀（「1.1.2」「3.2」）再数字数——否则「1.1.2招标人」
    // 光条款号就 5 个字符，闸门形同虚设。
    if strip_clause_no(t).chars().filter(|c| !c.is_whitespace()).count() < 8 {
        return false;
    }
    // ── 闸 0：自带否决处置词 → 一律不是规则说明（最终安全网）──
    // 复核中仍有标题恰为「否决项」的条款被判成规则说明（「3.3.3 招标人设有
    // 最高投标限价的…否则其投标将被否决」——主语是招标人，但它规定了处置
    // 后果，是要投标人当真的事）。**带处置后果的条款永远不可豁免**：
    // 漏判否决 = 废标风险，代价远大于多留一条 blocking。
    if has_self_veto_consequence(t) {
        return false;
    }
    // ── 闸 3：列举条目（语义由总括句决定，单看不判）──
    if is_list_item(t) {
        return false;
    }
    // ── 安全网 1：投标人响应义务词在场 → 不是规则说明 ──
    if has_bidder_obligation(t) {
        return false;
    }
    // ── 安全网 2：资格/证明材料诉求在场 → 必须保留为响应要求 ──
    if has_qualification_demand(t) {
        return false;
    }
    // ── 安全网 4：约束/处置他人行为 → 不是规则说明 ──
    if has_misconduct_clause(t) {
        return false;
    }

    has_rule_pattern(t)
}

/// 自带否决后果的条款（「其投标将被否决」「按无效标处理」…）。
///
/// 这是**最终安全网**：只要句子里出现否决处置后果，无论主语是谁、句式
/// 多像流程说明，都不能当规则说明豁免——漏判一条否决的代价是废标。
fn has_self_veto_consequence(t: &str) -> bool {
    const CONSEQUENCE: [&str; 8] = [
        "将被否决",
        "否决其投标",
        "否决该投标",
        "否决投标处理",
        "无效标",
        "无效投标",
        "作废标",
        "予以否决",
    ];
    CONSEQUENCE.iter().any(|k| t.contains(k))
}

/// 剥掉开头的条款号（「1.1.2」「3.2」「5.」），返回剩余正文。
///
/// 只剥「数字+点」组成的编号段；遇到非编号字符即停。
fn strip_clause_no(t: &str) -> &str {
    let s = t.trim_start();
    let mut end = 0usize;
    let mut seen_dot = false;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() {
            end = i + c.len_utf8();
        } else if c == '.' || c == '．' {
            seen_dot = true;
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if seen_dot { &s[end..] } else { s }
}

/// 列举条目形态（全角/半角括号圈号、中文顿号序号、⑴⑵ 类）。
fn is_list_item(t: &str) -> bool {
    let s = t.trim_start();
    let mut cs = s.chars().peekable();
    if let Some(&c) = cs.peek() {
        // 「（1）」「(1)」
        if c == '（' || c == '(' {
            return cs.clone().nth(1).is_some_and(|d| d.is_ascii_digit());
        }
        // 「⑴」「①」等带圈序号
        if ('⑴'..='⒇').contains(&c) || ('①'..='⑳').contains(&c) {
            return true;
        }
        // 「1、」是列举序号；「3.2」「1.5.1」是**条款号**不是列举序号——
        // 把条款号当条目排除，issue #226 的三条原文就全被判掉了（本站踩过）。
        if c.is_ascii_digit() {
            let rest: String = cs
                .clone()
                .skip_while(|d| d.is_ascii_digit())
                .take(1)
                .collect();
            if rest == "、" {
                return true;
            }
        }
        // 「一、」
        if ('一'..='十').contains(&c) {
            let rest: String = cs.clone().skip(1).take(1).collect();
            if rest == "、" {
                return true;
            }
        }
    }
    false
}

/// 约束/处置类条款（谁做了会怎样——GT 一律标否决，不可豁免）。
///
/// 复核中误判的典型：「招标人在开标前开启投标文件…泄露」「与招标人存在
/// 利害关系…不得参加投标」「中标人无正当理由拒签合同的，招标人取消其中
/// 标资格」。它们的共性是**带处置动词或禁止短语**。
fn has_misconduct_clause(t: &str) -> bool {
    const MISCONDUCT: [&str; 14] = [
        "串通",
        "泄露",
        "行贿",
        "内定",
        "弄虚作假",
        "不得参加",
        "不得参与",
        "取消",
        "拒签",
        "放弃中标",
        "撤销",
        "视为",
        "属于",
        "利害关系",
    ];
    MISCONDUCT.iter().any(|k| t.contains(k))
}
/// 投标人侧强义务词。
///
/// 只收「投标人/供应商/承包人」+「应/须/必须/应当/需」的紧邻组合，避免
/// 「应当」出现在招标人句里也误伤（「招标人应当拒收」是规则说明，不是义务）。
fn has_bidder_obligation(t: &str) -> bool {
    const SUBJECTS: [&str; 4] = ["投标人", "供应商", "投标单位", "承包人"];
    const MODALS: [&str; 5] = ["应当", "必须", "须", "应", "需"];
    SUBJECTS.iter().any(|s| {
        t.match_indices(s).any(|(i, m)| {
            // 按**分句**判：主语与模态词只要在同一个分句里就算义务。
            // 用固定字数窗口不行——「投标人与招标人就有关投标的所有往来函电
            // 均应当使用中文」里「均应当」离主语 15 个字，但它确实是投标人
            // 的义务（主语一直管到句号）。
            let tail = &t[i + m.len()..];
            let clause: String = tail
                .chars()
                .take_while(|c| !"，。；、！？,;!?".contains(*c))
                .collect();
            MODALS.iter().any(|mo| clause.contains(mo))
        })
    })
}

/// 资格/证明材料诉求词（出现即不可豁免）。
fn has_qualification_demand(t: &str) -> bool {
    const DEMANDS: [&str; 15] = [
        // 注意：不收「资格」本身——「实行资格后审」是流程说明，不是要交材料。
        "资质等级",
        "资质证书",
        "营业执照",
        "安全生产许可",
        "业绩",
        "财务报表",
        "审计报告",
        "证书",
        "注册建造师",
        "注册造价",
        "社保证明",
        "信用中国",
        "缴纳",
        "提供",
        "提交",
    ];
    DEMANDS.iter().any(|d| t.contains(d))
}

/// 规则句式（主语是招标人侧，或程序后果条款）。
fn has_rule_pattern(t: &str) -> bool {
    const RULE_SUBJECTS: [&str; 8] = [
        "本次招标",
        "本项目招标",
        "本招标项目",
        "招标人",
        "招标代理机构",
        "采购人",
        "招标方",
        "本工程",
    ];
    if RULE_SUBJECTS.iter().any(|s| t.contains(s)) {
        return true;
    }
    // 程序后果条款（主语省略但语义是「会发生什么」，不是「你要做什么」）
    const PROCEDURE: [&str; 6] = [
        "逾期送达",
        "逾期提交",
        "逾期未",
        "不予受理",
        "不予接收",
        "不予开标",
    ];
    PROCEDURE.iter().any(|s| t.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── #226 issue 原文的三条（必须判为规则说明）──────────────

    #[test]
    fn issue_3_2_no_consortium_is_rule() {
        assert!(is_rule_statement(
            "3.2本次招标不接受联合体投标。联合体投标的，应满足下列要求：/。"
        ));
    }

    #[test]
    fn issue_3_3_post_qualification_is_rule() {
        // 容易误判：句中有「招标文件」「资格审查」但主语是「本次招标」。
        // 注意它含「资格审查的具体要求见招标文件」——「要求」不在义务词表里，
        // 主语仍是招标人侧 → 规则说明。
        assert!(is_rule_statement(
            "3.3本次招标实行资格后审，资格审查的具体要求见招标文件。"
        ));
    }

    #[test]
    fn issue_5_2_late_submission_is_rule() {
        assert!(is_rule_statement(
            "5.2逾期送达的或者未送达指定地点的投标文件，招标人不予受理。"
        ));
    }

    // ── 反例：响应要求必须留住 ──────────────────────────────

    #[test]
    fn bidder_must_have_qualification_is_not_rule() {
        // 命中「不接受」词面但没有；主语是投标人 + 须 + 资质 → 必须响应
        assert!(!is_rule_statement(
            "3.1投标人须具备水利水电工程施工总承包一级及以上资质。"
        ));
    }

    #[test]
    fn bidder_shall_provide_is_not_rule() {
        assert!(!is_rule_statement(
            "投标人应当提供近三年经审计的财务报表。"
        ));
    }

    #[test]
    fn qualification_safety_net_blocks_rule_verdict() {
        // 主语写成「招标人要求」但诉求是证明材料 → 安全网拦住，仍要响应。
        assert!(!is_rule_statement(
            "招标人要求投标人提供营业执照副本复印件。"
        ));
    }

    #[test]
    fn supplier_must_is_not_rule() {
        assert!(!is_rule_statement(
            "供应商必须提交投标保证金缴纳凭证。"
        ));
    }

    // ── 边界：不该乱判 ─────────────────────────────────────

    #[test]
    fn plain_statement_without_rule_subject_is_not_rule() {
        // 既无规则主语也无义务词——保守判「不是规则说明」（交给原有逻辑）。
        assert!(!is_rule_statement("投标文件应当密封。"));
    }

    #[test]
    fn empty_and_whitespace_are_not_rule() {
        assert!(!is_rule_statement(""));
        assert!(!is_rule_statement("   "));
    }

    #[test]
    fn bidder_obligation_checked_within_six_chars() {
        // 「投标 人 均 应」这类带空格的写法也要认出来（PDF 抽取常见）
        assert!(!is_rule_statement("本次招标中，投标人均应提交资格审查资料。"));
    }

    // ── 招标人侧程序条款（主语是招标人，不构成投标人义务）──────

    #[test]
    fn agent_rejecting_late_bid_is_rule() {
        assert!(is_rule_statement(
            "招标代理机构对逾期递交的投标文件不予接收。"
        ));
    }

    #[test]
    fn purchaser_not_accepting_alternative_is_rule() {
        assert!(is_rule_statement("采购人不接受备选投标方案。"));
    }

    // ── 收紧回归（真实语料复核出来的误判，逐条钉死）───────────

    #[test]
    fn collusion_clause_is_not_rule() {
        // 主语是招标人，但讲的是串通情形——GT 标否决。
        assert!(!is_rule_statement(
            "（1）招标人在开标前开启投标文件并将有关信息泄露给其他投标人；"
        ));
    }

    #[test]
    fn conflict_of_interest_clause_is_not_rule() {
        assert!(!is_rule_statement(
            "（4）与招标人存在利害关系可能影响招标公正性的法人、其他组织或者个人不得参加投标；"
        ));
    }

    #[test]
    fn winner_breach_penalty_is_not_rule() {
        // 「取消中标资格」是处置条款
        assert!(!is_rule_statement(
            "8.2.2中标人无正当理由拒签合同的，招标人取消其中标资格，其投标保证金不予退还"
        ));
    }

    #[test]
    fn bidder_language_duty_is_not_rule() {
        assert!(!is_rule_statement(
            "3.2.3投标人递交的投标文件以及投标人与招标人就有关投标的所有往来函电均应当使用中文"
        ));
    }

    #[test]
    fn list_item_alone_is_not_rule() {
        // 列举条目语义由总括句定，单看不判（避免把否决清单拆散）
        assert!(!is_rule_statement("（1）逾期送达或者未送达指定地点的；"));
        assert!(!is_rule_statement("(2)招标人向投标人泄露标底；"));
        assert!(!is_rule_statement("⑴未按招标文件要求密封的。"));
        assert!(!is_rule_statement("1、投标文件未加盖公章的。"));
    }

    #[test]
    fn fragment_too_short_is_not_rule() {
        // 抽取碎片（「1.1.2招标人」）不判规则说明
        assert!(!is_rule_statement("1.1.2招标人"));
        assert!(!is_rule_statement("1.5招标方式"));
    }

    #[test]
    fn clause_with_veto_consequence_is_never_rule() {
        // 主语是招标人，但规定了处置后果 → 必须保留为否决项。
        assert!(!is_rule_statement(
            "3.3.3招标人设有最高投标限价的，投标人的投标报价不得超过最高投标限价，否则其投标将被否决"
        ));
        assert!(!is_rule_statement(
            "招标人不接受低于成本的报价，否则按无效标处理。"
        ));
    }

    #[test]
    fn genuine_rule_statement_still_passes_after_tightening() {
        // 收紧后 issue 的三条必须仍然判为规则说明（这是本模块的存在理由）
        assert!(is_rule_statement(
            "3.2本次招标不接受联合体投标。联合体投标的，应满足下列要求：/。"
        ));
        assert!(is_rule_statement(
            "3.3本次招标实行资格后审，资格审查的具体要求见招标文件。"
        ));
        assert!(is_rule_statement(
            "5.2逾期送达的或者未送达指定地点的投标文件，招标人不予受理。"
        ));
    }
}
