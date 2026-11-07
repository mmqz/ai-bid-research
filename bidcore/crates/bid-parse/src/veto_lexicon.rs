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
            "投标无效|投标均无效|",
            "取消其中标|取消中标资格|取消投标资格|取消其投标资格|",
            "没收其投标保证金|废标|",
            "拒收|不予受理|不予接收|不予接受|不予开标|不予评审|",
            "有权拒收|有权取消|有权否决|",
            // PDF 侧（extract_main_pdf re_veto）平级对齐——P0-A 已收、
            // DOCX 侧缺失的处置短语（missed 样例：「投标文件不允许偏离
            // 招标文件的实质性要求和条件」「否则各相关投标均无效」）
            "不允许偏离|实质性要求和条件|将被拒绝",
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

/// #345 否定框架（负面清单词的排除面）：「未(被)列入/纳入/进入/出现/
/// 处于…」框架内的负面词是**正面资格诉求**，不是负面情形断言。
///
/// 语料实证（corpus-A GT-06）：「3.2.6 投标人须提供未被列入失信被执行人、
/// 重大税收违法案件当事人名单的书面承诺及信用中国网站查询记录截图」
/// ——指令本体是「须提供承诺+截图」（资格条件），「失信」在「未被列入」
/// 否定框架内描述的是**要求保持的清白状态**；负面清单词表设计目标
/// （re_negative_list 注释）是「有下列情形之一的」类**负面情形列举**，
/// 框架内的命中是误报（e2e_m1 TC-01 G1_recall 类别错配三之一）。
///
/// 窄口径纪律：仅「未+列入族动词」——「不是本单位人员」（负面清单
/// 本词）与「未提供/未按要求」（违例条件形态，GT-11/真否决族）都
/// **不在**剥离面。跨度截断：顿号（名单列举）、且/但（并列连词——
/// 「未被列入失信名单且其投标将被否决」的处置词不得被吞）。
fn re_negated_frame() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"未(被)?(列入|纳入|进入|出现|处于)[^，。；：且但]{0,30}").unwrap()
    })
}

/// 剥离否定框架后的判定文本（「未被列入失信…名单」不参与负面词匹配）。
/// PDF 侧 veto_hit（融合词表）同用本函数——跨格式对称（#233 纪律）。
pub fn strip_negated_frames(text: &str) -> String {
    re_negated_frame().replace_all(text, "").into_owned()
}

/// 强否决判定：处置短语或负面清单词命中（例外从句剥离后）。
/// 正面资格要求（应具备资质/须提供报表）不会命中此面。
/// #345：负面清单词在「未(被)列入…」否定框架内不判（正面资格诉求面
/// ——GT-06 实锤，见 re_negated_frame 注释）；处置短语不受框架影响
/// （「未被列入失信名单且其投标将被否决」仍判——强词在框架外）。
pub fn is_veto(text: &str) -> bool {
    let stripped = strip_exceptions(text);
    if re_veto_strong().is_match(&stripped) {
        return true;
    }
    let no_frames = strip_negated_frames(&stripped);
    re_negative_list().is_match(&no_frames)
}

/// #345 制裁尾形态：指令本体之后的后果从句。否决处置词出现在此处时
/// 是「违反指令的处罚预告」，不是条款的响应类别。
///
/// 两种形态（corpus-A 实锤）：
/// - GT-24「…不得出现投标人名称…的标记，**否则作否决投标处理**。」
///   →「否则」标记尾（暗标格式指令本体在句首）
/// - GT-25「投标文件总页数不得超过 200 页（…），**超过页数上限的作否决
///   投标处理**。」→「，<违限条件>的+<处置>」融合尾（的与处置间**无
///   标点**——页数上限指令本体在逗号前）
///
/// 反例面（**不得**剥离——剥离即回归既有裁定）：
/// - 「投标人未提供有效营业执照的，其投标将被否决」——违例条件与处置
///   **分句**（的后隔逗号）→ 条款本体即否决规则（veto_lexicon 测试锁定）
/// - T2b「出现实质性负偏离的，作否决投标处理」（#110 G4）——同分句形
/// - A-17「偏差大于 5%，按废标处理」（GT true）——处置前无「的」
/// - 「不包含运输费用的报价将被否决」——无前置指令逗号（整句即违例
///   条件）→ 融合尾不激活（无 [，；] 锚）
fn re_sanction_tail() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            // ① 「否则」标记尾：否则起至句边界（。；）
            "否则[^。；]*",
            "|",
            // ② 融合尾：[，；] 锚后「<条件>的<处置>」——的与处置动词间无
            //    标点（分句形「的，处置」是违例条件主体，不剥）；处置
            //    动词收「作/按/视为/予以」处理族（「的+将被否决」形无
            //    语料证据，不剥——保守面）
            "[，；][^，。；]*的(其投标|其报价|投标)?(作|按|视为|予以)(否决|无效|废标)[^。；]*",
        ))
        .unwrap()
    })
}

/// #345：否决信号是否**只**存在于制裁尾（否则尾/融合处置尾）。
///
/// 返回 true 的语义：剥离制裁尾后的指令本体不再含任何否决信号——
/// 条款的 operative directive（响应对象）是本体的要求（暗标标记/
/// 页数上限），处置词只是违反后果。此时分类仲裁面（extract 编号分支）
/// 可让类别随章节上下文走。
///
/// 返回 false 的情形：无可剥离尾 / 剥离后本体仍含否决信号（否决语义
/// 不只在尾）/ 剥离后本体过短（纯制裁句——「否则作否决投标处理」
/// 单独成段时无指令本体可言，保守不仲裁）。
pub fn veto_confined_to_sanction_tail(text: &str) -> bool {
    let stripped = strip_exceptions(text);
    let rest = re_sanction_tail().replace_all(&stripped, "");
    if rest == stripped {
        return false; // 没有可剥离的制裁尾
    }
    let body: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
    if body.chars().count() < 6 {
        return false; // 剥离后无指令本体（纯制裁句）——不仲裁
    }
    !is_veto(&rest)
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

// ================================================================
// #52 引擎排除面（精确率 0.7519 → 0.85+）：
// 「否决类误报」的形态锁排除——每个族一条可解释规则。
//
// 方法论（issue #52 三步走的第 1/2 步产物）：
//   1. 493 条 FP 全量归因（scripts/fp_attribution.py 分桶）
//   2. TP 误伤面扫描（scripts/tp_collateral_scan.py）——每条规则
//      对 GT 1752 true 的命中数必须为 0（红线：召回不能掉）
//
// 语义原则：排除的是「形态上面像否决、语义上不是投标人否决条件」的
// 六类内容：评分算法 / 流标程序 / 拟制修正 / 钱面处置 / 合同条款 /
// 模板占位。强否决处置词（否决|无效|废标|拒收|弄虚作假…）与排除
// 形态并存时**保留否决判定**（保护词机制——分值行含「按废标处理」
// 仍是真否决，如 A-17「扣1分…按废标处理」GT true）。
// ================================================================

/// 强否决处置句式（句级放行面——「将被否决/按废标处理/弄虚作假」
/// 等处置形态句是真否决句，段内存在则整段保留）。
/// 注意词形：①「拒收」仅授权形态（有权拒收/拒收投标文件），
/// 「不得拒收」是义务方向（来往函件 FP）不放行；②「否决」需
/// 处置句式（将被否决/否决其投标），「否决投标情况描述」名词
/// 修饰形态不放行（表头 FP）。
fn re_disposal_protect() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "将被否决|否决其投标|否决该投标|其投标将被|予以否决|否决投标处理|",
            "视为无效|无效投标|按无效|作无效|按废标|作废标|废标处理|",
            "弄虚作假|虚假|串通|骗取|取消投标资格|取消其投标资格|",
            "有权拒收|将拒收|予以拒收|拒收投标文件|不予接收|不予受理"
        ))
        .unwrap()
    })
}

/// 分值形态（评分语义）：「N分」且非「N分钟」——regex crate 无
/// look-ahead，分钟排除在 [`exclusion_reason`] 代码级判定。
fn re_score_value() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d+(?:\.\d+)?\s*分").unwrap())
}

/// 评分算法词（评标基准价/算式/计算过程——算法描述非否决条件）。
fn re_score_algo() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "式中|评标基准价|评标基准单价|算术平均值|下浮值|下浮率|权重计算|",
            "计算出?.{0,14}得分|评分值|最佳报价|报价合理性|评分标准|评分办法|",
            "计算公式|计算完成|计算过程|算术错误检查|算术性错误检查"
        ))
        .unwrap()
    })
}

/// 流标/重新招标（招标人程序行为——否决对象是「全部投标」非单个投标人）。
fn re_fail_bid() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new("否决所有投标|否决全部投标|投标人少于3|有效投标不足3|重新招标|重新决标")
            .unwrap()
    })
}

/// 拟制修正（算术错误修正口径——更正规则；「投标人不接受修正才否决」
/// 的引导句不含这些子项词形，TP 已验证 0 误伤）。
fn re_arithmetic() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "大写金额|小写金额|以大写金额为准|以单价金额为准|以开标报价为准|",
            "修正总价|修正投标报价|缺漏项|细微偏差|偏差幅度"
        ))
        .unwrap()
    })
}

/// 钱面处置直接形态（中标后场景——拒签合同/履约担保/保证金配置）。
fn re_bond_direct() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "拒签合同|不与招标人订立合同|在签订合同时向招标人提出附加条件|",
            "履约保证金|履约担保|中标价的"
        ))
        .unwrap()
    })
}

/// 钱面处置弱形态（保证金没收/不予退还——需保护词共检）。
fn re_bond_weak() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new("不予退还|不予退并|没收其?投标保证金|取消其中标资格|取消中标资格").unwrap()
    })
}

/// 评委回避（处置对象是评标委员会成员——收窄面：仅近亲属/主管部门
/// 人员两项；「经济利益关系/其他利害关系」在 PDF 侧为 GT true，不排）。
fn re_recuse() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("近亲属|主管部门或者行政监督部门的人员").unwrap())
}

/// 合同解除/违约/廉洁条款（合同期处置——监理人/勘察人/设计人/发包人面）。
fn re_contract() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "无法履行或停止履行合同|不履行合同约定的其他义务|无法继续履行|",
            "转包、违法分包|擅自分包|转让监理工作|未能提供令对方满意|",
            "合同一方当事人出现破产|",
            "文件不符合规范标准以及合同约定|文件不符合法律以及合同约定|",
            "发包人未能按合同约定支付|拖延、拒绝批准付款|",
            "发包人原因造成|委托人原因造成|复工指示|乙方不得以任何理由向甲方"
        ))
        .unwrap()
    })
}

/// 空模板/表头/前附表配置行（示范文本待填占位——无可执行否决语义）。
fn re_template() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "其他情形：。|其他：。|",
            "否决投标情况描述|否决投标的依据|",
            "序号.{0,12}实质性要求和条件.{0,12}(投标)?响应内容|",
            "其他符合招标文件中规定的其他.{0,6}(评审|审查)要求|",
            "无行贿犯罪记录查询开始时间|解释权构成|",
            "条款号条款名称编列内容|实质性要求和条件（1）招标范围"
        ))
        .unwrap()
    })
}

/// 评标/递交/澄清程序（流程性规定——非否决条件）。
fn re_procedure_program() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "来往函件|拒收的?，?视为(委托人|发包人|招标人)已经接收|",
            "澄清通知和.{0,12}澄清函是.{0,10}组成部分|",
            "评标程序|评标工作一般按以下程序|召开预备会|评标委员会成员熟悉|",
            "初步评审，包括|详细评审。?包括|技术标评审；|商务资信标评审|",
            "技术组评审内容中需要商务组配合|完成评标报告|",
            "文件报送程序|视为无效文件|备选方案外|",
            "逾期提出的，招标人及代理机构不予受理|",
            "不得通过补充、修改或撤消投标文件|不提供书面答复的，由评标委员会|",
            "投标人应对其情况专题说明|",
            "投标文件的修改与撤回|投标人可以修改或撤回|CA数字证书登录|",
            "资格文件将被保密|招标人将审查投标人提交的资格文件|",
            "投标文件的符合性审查|没有组成联合体"
        ))
        .unwrap()
    })
}

/// 投标函声明/格式装订要求（格式范本——投标人自述与编制规则）。
fn re_declare() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "我方在此声明|我承诺|我方承诺|所递交的投标文件及有关资料内容完整|",
            "投标函附录属于合同文件的组成部分|我单位保证上述承诺事项|",
            "装订顺序必须与|基本账户开户许可证复印件|资格文件的签署应保证|",
            "生产厂家并分别填写上表|选型原则|投标辅助资料"
        ))
        .unwrap()
    })
}

/// 现场服务/培训/人员要求（合同技术条款——非投标否决）。
fn re_field_service() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            "现场服务人员|服务人员的|技术服务人员|技术交底|人天数|",
            "有较强的责任感和事业心|身体健康，适应现场工作|",
            "了解合同设备的设计|遵守现场的各项规章"
        ))
        .unwrap()
    })
}

/// 投标撤销/修改（投标人主动行为——处置面是保证金 BOND，GT 判 false）。
fn re_withdraw() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new("撤销或修改其?投标文件|撤销投标文件|修改或撤回已传输").unwrap()
    })
}

/// 跨章节负面词误触发（第五章技术内容的行政职能描述——「行政处罚」
/// 词面在水政监察语境非投标限制）。
fn re_neg_context() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("水政监察|对管辖范围内公民").unwrap())
}

/// 投标人自述句头（「1、我将严格遵守…」——数字序号 + 第一人称）。
fn re_self_declare_head() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[0-9①-⑳]{1,3}[、.]我(将|们|方|单位)").unwrap())
}

/// #52 排除面统一入口（句级算法）：命中返回排除原因，未命中 None。
///
/// # 算法（段落重组后的段内异质句问题——A-05-GT-163 误伤驱动）
///
/// 段落按句读（。；？！）拆句，逐句四类判定（优先级从上到下）：
///
/// 1. **流标句**（否决所有/全部投标）→ 无条件排除句——否决对象是
///    「全部投标」（程序形态）与单个投标人否决形态互斥；
/// 2. **处置形态句**（将被否决/按废标/弄虚作假/有权拒收…）→ 放行句；
/// 3. **排除形态句**（评分/修正/钱面/回避/合同/模板/程序/声明/服务/
///    撤回/跨章节）→ 排除句；
/// 4. **强词句**（`is_veto` 命中）→ 放行句；其余为中性句。
///
/// 段落级裁决：存在放行句 ⇒ 整段保留（None）；否则存在排除句 ⇒
/// 整段排除（首个原因）；全中性 ⇒ 保留。
///
/// 这样「（2）初步评审，包括…；在初步评审中投标人有一项不符合评审
/// 标准的，其投标将被否决。」（程序句+否决句拼段）保留，而
/// 「7.4.1 …订立书面合同。中标人无正当理由拒签合同的…」（中性句+
/// 钱面句）排除。零误伤不变式：真否决条款必然含处置形态句或强词句。
pub fn exclusion_reason(text: &str) -> Option<&'static str> {
    let mut has_veto_sentence = false;
    let mut first_reason: Option<&'static str> = None;
    let mut cur = String::new();
    for ch in text.chars() {
        cur.push(ch);
        if "。；？！;!?".contains(ch) {
            if let Some(r) = judge_sentence(&cur, &mut has_veto_sentence) {
                if first_reason.is_none() {
                    first_reason = Some(r);
                }
            }
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        if let Some(r) = judge_sentence(&cur, &mut has_veto_sentence) {
            if first_reason.is_none() {
                first_reason = Some(r);
            }
        }
    }
    if has_veto_sentence {
        return None;
    }
    first_reason
}

/// 单句判定：返回排除原因（排除句）；放行句置位 veto_flag；中性句 None。
fn judge_sentence(s: &str, veto_flag: &mut bool) -> Option<&'static str> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // 1. 流标句：无条件排除（形态互斥——见 fn 文档）
    if re_fail_bid().is_match(t) {
        return Some("流标/重新招标——招标人程序行为");
    }
    // 2. 处置形态句：放行
    if re_disposal_protect().is_match(t) {
        *veto_flag = true;
        return None;
    }
    // 3. 排除形态句（各族）
    let score = re_score_value()
        .find_iter(t)
        .any(|m| !t[m.end()..].starts_with('钟'))
        || re_score_algo().is_match(t);
    if score {
        return Some("评分公式/算式形态——算法描述非否决条件");
    }
    if re_arithmetic().is_match(t) {
        return Some("算术错误修正口径——更正规则非否决");
    }
    if re_bond_direct().is_match(t) {
        return Some("中标后钱面处置（拒签/履约担保）");
    }
    if re_bond_weak().is_match(t) {
        return Some("保证金/中标资格处置（钱面）");
    }
    if re_recuse().is_match(t) {
        return Some("评标委员会成员回避情形——处置对象是评委");
    }
    if re_contract().is_match(t) {
        return Some("合同解除/违约/廉洁条款——合同期处置");
    }
    if re_template().is_match(t) {
        return Some("空模板占位/表头/配置行——无可执行否决语义");
    }
    if re_procedure_program().is_match(t) {
        return Some("评标/递交/澄清程序——流程性规定");
    }
    if re_self_declare_head().is_match(t) || re_declare().is_match(t) {
        return Some("投标函声明/格式装订要求——格式范本");
    }
    if re_field_service().is_match(t) {
        return Some("现场服务/培训/人员要求——合同技术条款");
    }
    if re_withdraw().is_match(t) {
        return Some("投标撤销/修改——投标人主动行为（BOND 面）");
    }
    if re_neg_context().is_match(t) {
        return Some("跨章节负面词误触发（技术章行政职能）");
    }
    // 4. 强词句：放行
    if is_veto(t) {
        *veto_flag = true;
    }
    None
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

    // ============ #52 排除面测试 ============
    // FP 样例（归因桶逐族抽样——每族一条可解释排除）+ GT true 防误伤样例

    #[test]
    fn exclusion_score_family() {
        // 评分公式：分值形态 / 算式 / 算法词
        assert!(exclusion_reason("⑵施工程序、工艺符合工程实际和有关施工规程规范,且投入的设备和人力计划安排合理的，得3分，基本合理的，得1分，否则得0～1分；").is_some());
        assert!(exclusion_reason("⑴确定招标控制单价的下浮值R=P×（1−K）式中：R为下浮后的招标控制单价；").is_some());
        assert!(exclusion_reason("⑶投标报价的算术错误检查、总报价评分，由招标人或者招标代理机构工作人员依据本标准计算完成。").is_some());
        // 「N分钟」不是分值形态
        assert!(exclusion_reason("解密时间从开标时间起 60 分钟内完成，超过规定时间解密的投标文件不予接收。").is_none());
        // 保护词：分值行含强处置词 → 保留否决（A-17 GT true）
        assert!(exclusion_reason("每有一项错误扣1分，最多扣5分，修正后的报价与投标报价偏差大于 5%，按废标处理。").is_none());
    }

    #[test]
    fn exclusion_fail_bid_family() {
        assert!(exclusion_reason("⑵经评标委员会评审后否决所有投标的；").is_some());
        assert!(exclusion_reason("⑶评标委员会否决不合格投标或者界定为废标后因有效投标不足3个使得投标明显缺乏竞争，评标委员会决定否决全部投标的；").is_some());
    }

    #[test]
    fn exclusion_arith_family() {
        assert!(exclusion_reason("①投标文件中的大写金额与小写金额不一致的，以大写金额为准；").is_some());
        assert!(exclusion_reason("（4）如果分项报价中存在缺漏项，则视为缺漏项价格已包含在其他分项报价之中。").is_some());
        // GT true 引导句（不接受修正→否决）不受影响
        assert!(exclusion_reason("3.1.3 投标报价有算术错误的，评标委员会按以下原则对投标报价进行修正，修正的价格经投标人书面确认后具有约束力。投标人不接受修正价格的，其投标将被否决。").is_none());
    }

    #[test]
    fn exclusion_bond_family() {
        // 直接形态：拒签/履约
        assert!(exclusion_reason("7.4.1招标人和中标人应当自中标通知书发出之日起30天内订立书面合同。中标人无正当理由拒签合同的，招标人取消其中标资格，其投标保证金不予退还；").is_some());
        assert!(exclusion_reason("7.3.2中标人不能按本章第7.3.1项要求提交履约担保的，视为放弃中标，其投标保证金不予退并列入不良行为记录。").is_some());
        // 弱形态 + 无保护词
        assert!(exclusion_reason("3.4.4 有下列情形之一的，投标保证金将不予退还：").is_some());
        // 保护词：弄虚作假→没收保证金是真否决的组成部分（A-15 GT true）
        assert!(exclusion_reason("若评标过程中查出有虚假，同意作无效投标文件处理并被没收投标保证金。").is_none());
    }

    #[test]
    fn exclusion_recuse_and_contract() {
        assert!(exclusion_reason("（1）投标人或投标人主要负责人的近亲属；").is_some());
        assert!(exclusion_reason("（2）项目主管部门或者行政监督部门的人员；").is_some());
        // 「经济利益关系/其他利害关系」PDF 侧 GT true——不排（跨文档不一致族）
        assert!(exclusion_reason("（5）与投标人有其他利害关系。").is_none());
        // 合同解除/违约
        assert!(exclusion_reason("（4）监理人无法履行或停止履行合同；").is_some());
        assert!(exclusion_reason("（1）勘察文件不符合法律以及合同约定；").is_some());
    }

    #[test]
    fn exclusion_template_program() {
        assert!(exclusion_reason("（6）其他情形：。").is_some());
        assert!(exclusion_reason("否决投标情况描述").is_some());
        assert!(exclusion_reason("其他符合招标文件中规定的其他形式评审要求").is_some());
        assert!(exclusion_reason("10.6解释权构成本招标文件的各个组成文件应互为解释，互为说明；").is_some());
        // 程序族
        assert!(exclusion_reason("1.7.3来往函件均应按合同约定的期限及时发出和答复，不得无故扣压和拖延，亦不得拒收。").is_some());
        assert!(exclusion_reason("（1）召开预备会，评标委员会成员熟悉招标文件内容；").is_some());
        assert!(exclusion_reason("4.2.7投标人应当在投标截止时间前，通过互联网使用CA数字证书登录上传，未在投标截止时间前完成上传的，视为逾期送达。").is_some());
        // 「不接受联合体投标」决定句是真否决（GT true）；「没有组成联合体」文件说明句才排
        assert!(exclusion_reason("3.2 本次招标不接受联合体投标。").is_none());
        assert!(exclusion_reason("3.1.2投标人须知前附表规定不接受联合体投标的，或投标人没有组成联合体的，投标文件不包括本章所指的联合体协议书。").is_some());
    }

    #[test]
    fn exclusion_declare_svc_withdraw() {
        assert!(exclusion_reason("1、我将严格遵守《中华人民共和国招标投标法》及有关法规和规章制度；").is_some());
        assert!(exclusion_reason("2、所附业绩证明文件的装订顺序必须与上表所列业绩项目编号的顺序相一致。").is_some());
        assert!(exclusion_reason("⑵随同本投标函递交的投标函附录属于合同文件的组成部分；").is_some());
        assert!(exclusion_reason("⑷未经招标人同意，承包人不得随意更换现场服务人员。").is_some());
        assert!(exclusion_reason("（1）投标人在规定的投标有效期内撤销或修改其投标文件；").is_some());
        // 跨章节负面词（水政监察——技术章行政职能）
        assert!(exclusion_reason("4.5.2.6水政监察主要是依法对管辖范围内公民、法人或其他组织违反水法律法规的行为实施行政处罚。").is_some());
    }

    #[test]
    fn exclusion_keeps_core_veto() {
        // GT true 核心否决形态——排除面必须全放行（零误伤回归）
        assert!(exclusion_reason("3.4.2 投标人未提供有效营业执照的，其投标将被否决").is_none());
        assert!(exclusion_reason("（1）投标文件没有对招标文件的实质性要求和条件作出响应，或者对招标文件的偏差超出招标文件规定的偏差范围或最高项数；").is_none());
        assert!(exclusion_reason("（12）在最近三年内有骗取中标或严重违约或重大工程质量问题的。").is_none());
        assert!(exclusion_reason("（4）曾因在招标、评标以及其他与招标投标有关活动中从事违法行为而受过行政处罚或刑事处罚的；").is_none());
        assert!(exclusion_reason("⑵ 与招标人存在利害关系可能影响招标公正性的法人、其他组织或者个人不得参加投标；").is_none());
        assert!(exclusion_reason("⑶ 与本招标项目的其他投标人为同一个单位负责人；").is_none());
        assert!(exclusion_reason("10.5 招标人拒收投标文件的其他情形").is_none());
    }

    // ============ #345 测试（e2e_m1 G1_recall 类别错配三例 + 反例锁） ============

    /// GT-06：负面词在「未被列入」否定框架内 → 正面资格诉求，非否决。
    #[test]
    fn negated_frame_suppresses_negative_list() {
        // GT-06 原文（corpus-A A-招标文件.docx 3.2.6——GT 类别资格条件）
        assert!(!is_veto(
            "3.2.6 投标人须提供未被列入失信被执行人、重大税收违法案件当事人名单的书面承诺及信用中国网站查询记录截图。"
        ));
        // 未列入（无「被」）同族形态
        assert!(!is_veto("投标人未列入严重失信名单的证明材料。"));
        // 框架外仍有强处置词 → 仍判否决（框架剥离不得吞框架外信号）
        assert!(is_veto("投标人未被列入失信名单且其投标将被否决。"));
        assert!(is_veto("未被列入失信名单的，其投标将被否决"));
        // 反例锁：无框架的负面情形断言不受影响
        assert!(is_veto("（12）在最近三年内有骗取中标或严重违约或重大工程质量问题的。"));
        assert!(is_veto("投标人被列入失信被执行人名单的，取消其投标资格。"));
        // 反例锁：「未提供/未按要求」违例条件形态不在剥离面（GT-11 族真否决）
        assert!(is_veto("未按要求密封的投标文件，招标人有权拒收"));
    }

    /// GT-24/25：处置词只在制裁尾 → veto_confined_to_sanction_tail = true。
    #[test]
    fn sanction_tail_confined_true() {
        // GT-24 原文（否则标记尾——暗标格式指令本体在句首）
        assert!(veto_confined_to_sanction_tail(
            "3.3.3 技术标部分采用暗标评审：技术标正文中不得出现投标人名称、投标人 Logo、项目班子成员姓名及其他可识别投标人身份的标记，否则作否决投标处理。"
        ));
        // GT-25 原文（，<违限条件>的+<处置>融合尾——页数上限指令在逗号前）
        assert!(veto_confined_to_sanction_tail(
            "3.3.4 投标文件总页数不得超过 200 页（含封面、目录及附件），超过页数上限的作否决投标处理。"
        ));
    }

    /// 反例锁：制裁尾剥离不得激活于违例条件主体形（既有裁定——#110
    /// T2b / A-17 GT true / 无前置指令融合形 / 分句处置形）。
    #[test]
    fn sanction_tail_confined_false_counterexamples() {
        // 「…的，其投标将被否决」——违例条件与处置分句，条款本体即否决规则
        assert!(!veto_confined_to_sanction_tail(
            "3.4.2 投标人未提供有效营业执照的，其投标将被否决。"
        ));
        // #110 T2b：分句形「…的，作否决投标处理」（的与处置隔逗号）→ 不剥
        assert!(!veto_confined_to_sanction_tail(
            "4.6 投标文件技术参数出现一般负偏离的，每项扣 1 分（最多扣 5 分）；出现实质性负偏离的，作否决投标处理。"
        ));
        // A-17（GT true）：处置前无「的」
        assert!(!veto_confined_to_sanction_tail(
            "4.7 报价修正：每有一项错误扣1分，最多扣5分，修正后的报价与投标报价偏差大于 5%，按废标处理。"
        ));
        // 无前置指令（整句即违例条件，无 [，；] 锚）→ 融合尾不激活
        assert!(!veto_confined_to_sanction_tail("不包含运输费用的报价将被否决。"));
        // GT-11 形（前附表否决行——处置分句无的）
        assert!(!veto_confined_to_sanction_tail(
            "未按本表第 9 项规定格式填写技术偏离表的，其投标作否决投标处理"
        ));
        // 无制裁尾的普通否决句
        assert!(!veto_confined_to_sanction_tail("投标报价超过招标控制价（最高限价）的，否决其投标。"));
        // 剥离后本体仍含否决信号（否决语义不只在尾）→ 不仲裁
        assert!(!veto_confined_to_sanction_tail(
            "投标人串通投标报价的，其投标作废标处理，否则追究法律责任。"
        ));
    }
}
