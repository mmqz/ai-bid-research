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
/// # 复核工具
///
/// 判据对不对不能看计数——用 `examples/dump_rule_statements.rs` 逐条看：
///
/// ```text
/// cargo run --release -p bid-parse --example dump_rule_statements -- ../../real-corpus
/// ```
///
/// 它打印每条被判为规则说明的条款原文 + 最终分类，供人工逐条审阅。
/// **每加一条分类判据都应该跑一遍并逐条看**（本模块的两轮收紧都是
/// 这么来的）；分类计数改前/改后对照用同目录 `count_categories.rs`。
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
///
/// # 第三轮收紧（PR #230 审核：dump 复核 [35] 误判）
///
/// 复核面 56 条里审出 1 条误判：「1.4 到货要求：★卖方承诺收到招标人
/// 通知后20工作日内到货。」——主句主语是「卖方」（我方），「招标人」
/// 只出现在从句（收到谁的通知），全串搜索把它误判成规则说明，★ 实质
/// 性要求被豁免 = 漏响应风险。共同形态：**规则主语在从句/定语里，
/// 主句主语是投标人侧**（「承包人应在接到招标人开工通知后 7 日内进场」）。
/// 修法：网 0.5（投标人侧主语守卫）+ 规则主语句首位置判定（见下）。
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
    // ── 安全网 0.5：主句主语是投标人侧 → 不是规则说明（#230 审核追修）──
    // 「★卖方承诺收到招标人通知后…到货」：「招标人」在从句，主语是
    // 「卖方」。与网 1 的分工：网 1 查「投标人侧 + 义务词」同分句，
    // 本网查「投标人侧主语在句首」——「承包人负责…」没有义务词时
    // 网 1 不响，这里仍拦得住。
    if has_bidder_side_subject(t) {
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
    const CONSEQUENCE: [&str; 9] = [
        "将被否决",
        "否决其投标",
        "否决该投标",
        "否决投标处理",
        "无效标",
        "无效投标",
        "作废标",
        "予以否决",
        // CI 红线（extract_pdf_l2gap）：「投标人拒绝延长的，其投标失效」
        // 是处置结果（GT 标否决）——与「无效标」同族，永不豁免。
        "投标失效",
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
    const MISCONDUCT: [&str; 15] = [
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
        // #233（DOCX 基线 85 条候选复核抓出）：「不私下接触采购人/
        // 招标代理机构或评标委员会成员，利用人际关系干扰采购活动」——
        // 串通纪律条。DOCX 侧它是独立段（PDF 侧被列举闸拦），「接触采购人」
        // 的宾语命中规则主语窗（快窗「不私下接触采购人」第 6-8 字位）。
        // 「私下接触」是纪律条专用词，不误伤规则说明面（PDF 69 条 dump 零命中）。
        "私下接触",
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

/// 投标人侧主语词表（两句口径，勿混用）：
/// - [`BIDDER_SUBJECTS`]（网 0.5，硬守卫）：不含「投标人」——「招标人
///   将对投标人进行资格审查」「招标人收到投标文件后…」这类真规则句
///   里「投标人」常在句首段，含它会大量误杀规则说明；
/// - [`EXTENDED_GUARD_SUBJECTS`]（扩展窗守卫）：含「投标人」——只用于
///   「规则主语在 12-24 字位」的低置信分支，主句头出现「投标人撤回…」
///   时该句主语是投标人（我方义务），扩展窗不适用。
const BIDDER_SUBJECTS: [&str; 8] =
    ["卖方", "供方", "供应商", "承包人", "施工方", "中标人应", "乙方", "我方"];
const EXTENDED_GUARD_SUBJECTS: [&str; 9] = [
    "卖方", "供方", "供应商", "承包人", "施工方", "中标人应", "乙方", "我方", "投标人",
];

/// 主句头：剥条款号；句首是短标签（「到货要求：」「处理规则：」——
/// 首个 ： 前 ≤ 12 字、无分句标点、冒号后有正文）时从冒号后起算。
/// 标签不是主语：「1.4 到货要求：★卖方承诺…」的主句从「★卖方」起。
fn main_clause(t: &str) -> &str {
    let body = strip_clause_no(t).trim_start();
    if let Some(pos) = body.find('：') {
        let label = &body[..pos];
        let label_ok = label.chars().count() <= 12
            && !label.contains('，')
            && !label.contains('。')
            && !label.contains('；');
        let after = body[pos + '：'.len_utf8()..].trim_start();
        if label_ok && !after.is_empty() {
            return after;
        }
    }
    body
}

/// 头部分句里是否有 `subjects` 中的投标人侧主语（前 20 字）。
fn bidder_subject_in_head(main: &str, subjects: &[&str]) -> bool {
    let head: String = main
        .split(['，', '。', '；', '：'])
        .next()
        .unwrap_or(main)
        .chars()
        .take(20)
        .collect();
    subjects.iter().any(|s| head.contains(s))
}

/// 投标人侧主语守卫（PR #230 审核追修——dump [35] 误判的通用形态）。
///
/// 「卖方承诺收到招标人通知后20工作日内到货」「承包人应在接到招标人
/// 开工通知后 7 日内进场」——规则主语（招标人/采购人）只出现在从句里，
/// 主句主语是投标人侧（卖方/承包人/供应商…）。这类句子是我方义务，
/// 不是规则说明。
///
/// 主语判定：**主句头分句**（第一个 ，。；： 前的片段）的前 20 字——
/// 主语通常在句首；「招标人」出现在「收到/接到/按…要求」从属结构里
/// 时不算主句主语。「中标人应」带义务词：裸「中标人」常出现在规则句
/// 里（「招标人将要求中标人签订合同」），不带「应」不算投标人侧主语。
fn has_bidder_side_subject(t: &str) -> bool {
    bidder_subject_in_head(main_clause(t), &BIDDER_SUBJECTS)
}

/// 规则句式（主语是招标人侧，或程序后果条款）。
///
/// 规则主语用**主句头位置判定**（PR #230 审核追修），两级窗口：
///
/// - 快窗（前 12 字）：主语在句首——高置信，直接判规则说明；
/// - 扩展窗（前 32 字）：主语在句中（「在投标截止时间15天前，招标人
///   可以修改招标文件」「依法必须进行招标的通信工程建设项目，采用
///   公开招标方式的，招标人在收到评标报告…」）——只有主句头**无
///   投标人侧主语**时才认：「投标人撤回…应当书面通知招标人」的主语
///   是投标人，扩展窗一开就会把 [35] 类义务句放回来，所以必须带守卫。
///   两级窗口都只是原全串搜索的收窄——判规则说明的面只会变少不会
///   变多（恢复项也全部来自 v0 已判面），无新增误放行。
///
/// 「招标人」出现在从句（「收到招标人通知后…到货」）两级都不命中——
/// 那是投标人侧义务（dump [35]）。程序后果词保持全串搜索：它们描述
/// 投标文件本身的命运（逾期送达/不予受理），不依赖主语位置。
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
    let main = main_clause(t);
    let head12: String = main.chars().take(12).collect();
    if RULE_SUBJECTS.iter().any(|s| head12.contains(s)) {
        return true;
    }
    let head32: String = main.chars().take(32).collect();
    if RULE_SUBJECTS.iter().any(|s| head32.contains(s))
        && !bidder_subject_in_head(main, &EXTENDED_GUARD_SUBJECTS)
    {
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

    // ── 第三轮收紧（PR #230 审核：dump [35] + 从句形态四例）────────

    #[test]
    fn seller_delivery_promise_is_not_rule() {
        // dump 复核 [35]（Lead 审核抓出）：★ 实质性要求，主语是「卖方」，
        // 「招标人」只在从句（收到谁的通知）——句首位置判定后不再误判。
        assert!(!is_rule_statement(
            "1.4 到货要求：★卖方承诺收到招标人通知后20工作日内到货。"
        ));
    }

    #[test]
    fn contractor_mobilization_after_notice_is_not_rule() {
        // 审核给的从句形态：规则主语出现在「接到…通知后」从属结构里。
        assert!(!is_rule_statement(
            "承包人应在接到招标人开工通知后7日内进场。"
        ));
        // 无义务词形态（网 1 不响，网 0.5 主语守卫拦）：
        assert!(!is_rule_statement(
            "承包人负责在接到招标人开工通知后7日内进场。"
        ));
    }

    #[test]
    fn supplier_shipping_after_order_is_not_rule() {
        assert!(!is_rule_statement("供应商须在采购人下单后3日内发货。"));
        assert!(!is_rule_statement("供方应在收到采购人订单后48小时内发货。"));
    }

    #[test]
    fn bidder_sample_per_tenderer_request_is_not_rule() {
        assert!(!is_rule_statement("投标人应按招标人要求提供样品。"));
    }

    #[test]
    fn rule_subject_at_sentence_head_still_rule() {
        // 主语在句首（含条款号剥离后 12 字窗口内）仍是规则说明——
        // 收紧只针对从句位置，不伤句首真规则。
        assert!(is_rule_statement(
            "1.7.4 采用资格后审的，招标人必须在投标截止时间前完成审查。"
        ));
        assert!(is_rule_statement("招标人不接受低于成本报价的不正当竞争行为。"));
        assert!(is_rule_statement("1.2 资金来源招标人自筹。"));
    }

    #[test]
    fn procedure_word_deep_in_sentence_still_rule() {
        // 程序后果词保持全串搜索（不依赖主语位置）：
        // 「不予受理」在句中后段仍判规则说明。
        assert!(is_rule_statement(
            "投标截止时间后送达的投标文件，招标人不予受理。"
        ));
    }

    #[test]
    fn rejection_word_relies_on_subject_window_not_procedure() {
        // 「拒收」**不在** PROCEDURE 词表（PR #230 审核独立复核确认的边界）：
        // 它与「不予受理」语义同族（描述投标文件的命运），但入表会把
        // 「评标委员会将予以拒收」这类无规则主语的形态也扩进规则说明面——
        // 判面扩宽须重跑 dump 复核（见模块文档「复核工具」节），故保持窄表。
        // 现状登记：
        // 1) 主语是招标人（主句头窗口命中）→ 规则说明；
        assert!(is_rule_statement("未按要求密封的投标文件，招标人有权拒收。"));
        // 2) 主语是「评标委员会」（不在 RULE_SUBJECTS）→ 保守落在否决项
        //    （多留 blocking，不漏否决——与 #226 风险不对称原则一致）。
        assert!(!is_rule_statement(
            "未按要求密封的投标文件，评标委员会将予以拒收。"
        ));
    }

    #[test]
    fn bid_validity_disposal_is_never_rule() {
        // CI 红线（extract_pdf_l2gap）：「其投标失效」是处置结果——
        // 主语是招标人也不能豁免（闸 0 补「投标失效」）。
        assert!(!is_rule_statement(
            "3.3.2 出现特殊情况需要延长投标有效期的，招标人以书面形式通知所有投标人延长投标有效期。投标人拒绝延长的，其投标失效，但投标人有权收回投标保证金。"
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
