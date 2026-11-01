//! 多级编号形态识别（B6 · #123 任务 2 / G5）。
//!
//! 中国招标文件的编号格式极其混乱（任务书原话），同一份文件里七种形态
//! 混排是常态。本模块把 G1 在 [`crate::clause`] 里的最小编号面（6 形态、
//! 仅判层级）扩成**全枚举 + 显式层级推断**：
//!
//! | 形态 | 例 | 基线层级 | 依据 |
//! |---|---|---|---|
//! | [`NumberingStyle::ChineseNumeral`] | `一、` | 1 | 公文首级惯例 |
//! | [`NumberingStyle::ChineseParen`] | `（一）`/`(一)`/`（1）` | 2 | 公文次级惯例 |
//! | [`NumberingStyle::ArabicDot`] | `1.` / `1、` / `2．` | 1 | 纯阿拉伯体系首级（形态歧义，见下） |
//! | [`NumberingStyle::ArabicMultiLevel`] | `1.1` / `4.2` / `1.1.1` | 段点数 | 自描述（`1.1.1` = 3 级） |
//! | [`NumberingStyle::Circled`] | `①` / `⑸` / `⒈` | 3 | 圈号惯例（Unicode 数字圈四段区间，见 [`is_circled_digit`]） |
//! | [`NumberingStyle::AlphaParen`] | `a)` / `B）` | 2 | 字母列举惯例 |
//! | [`NumberingStyle::Bullet`] | `●` / `○` / `■` | 4 | 项目符号为最深列举层 |
//!
//! 基线层级 = **无状态形态先验**：同一形态在任何行上给出同一层级（确定性
//! 纪律——同输入同输出）。混排从属的正确性由形态间的基线差天然保证：
//! `一、`(1) 下面跟 `1.1`(2)、`1.1.1`(3)，从属关系不乱。
//!
//! **已知歧义（如实登记）**：`ArabicDot`（`1.`）在公文体系（一、→（一）→ 1.）
//! 里是第 3 级，在纯阿拉伯体系（1. → 1.1 → 1.1.1）里是第 1 级——单行信息
//! 不足以分辨。本模块给基线 1（纯阿拉伯体系），并提供 [`NumberingContext`]
//! 做**可选的**语境修正：见过中文形态后的 `1.` 按公文惯例重定级。真实语料
//! 实证（3 件水科 PDF）：`一、`（文件组成清单）与 `1、`（章节编号）平级
//! 并存的文件真实存在——语境修正对这类文件会**高估**层级，故它是增强面
//! 而非默认面，抽取主链不接线（G5 验收：不扰红线）。
//!
//! 零外部依赖：仅 `regex`（与 [`crate::clause`] 同栈）。

/// 编号形态（任务书七形态枚举）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NumberingStyle {
    /// 中文数字 + 顿号：`一、` `十二、`
    ChineseNumeral,
    /// 括号包数字（全/半角括号，阿拉伯或中文数字）：`（一）` `(1)` `（2）`
    ChineseParen,
    /// 阿拉伯数字 + 点/顿（全/半角）：`1.` `2、` `3．`
    ArabicDot,
    /// 多级点分（≥2 段）：`1.1` `4.2` `1.1.1`
    ArabicMultiLevel,
    /// 圈数字（U+2460..=U+24BF：①-⑳ ⑴-⑳ ⒈-⒛ ㉑-㊿）：`①` `⑸` `⒈`
    Circled,
    /// 字母 + 括号（全/半角）：`a)` `B）`
    AlphaParen,
    /// 项目符号：`●` `○` `■`
    Bullet,
}

impl NumberingStyle {
    /// 形态基线层级（无状态先验，见模块表）。
    pub fn baseline_level(self) -> u8 {
        match self {
            NumberingStyle::ChineseNumeral => 1,
            NumberingStyle::ChineseParen => 2,
            NumberingStyle::ArabicDot => 1,
            // 多级点分自描述，但段点数依赖行内容——baseline 给保守下限 2，
            // 真实层级由 detect_numbering 按段点数给出（≥2 恒成立）
            NumberingStyle::ArabicMultiLevel => 2,
            NumberingStyle::Circled => 3,
            NumberingStyle::AlphaParen => 2,
            NumberingStyle::Bullet => 4,
        }
    }
}

/// 局部 once-lock 正则（与 clause.rs 同款小工具——构造一次，测试与产线同形）。
macro_rules! regex {
    ($re:expr) => {{
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| regex::Regex::new($re).unwrap())
    }};
}
use regex;

/// 识别行首编号形态与层级（无状态；任务书面）。
///
/// 返回 `Some((style, level))`：level 为基线层级，唯一例外是
/// [`NumberingStyle::ArabicMultiLevel`]——按段点数自描述（`1.1`→2、
/// `1.1.1`→3、`4.2`→2）。非编号行（含空行、普通句子、`式中：△P` 这类
/// 符号用法）返回 `None`。
///
/// 判定顺序与 G1 [`crate::clause::clause_boundary`] 完全一致（多级点分 →
/// 括号数字 → 圈数字 → 中文顿号 → 阿拉伯点顿 → 字母括号 → 项目符号），
/// 保证两 API 对任何输入同判（clause_boundary 是本函数的薄封装）。
pub fn detect_numbering(line: &str) -> Option<(NumberingStyle, u8)> {
    let t = line.trim_start();
    if t.is_empty() {
        return None;
    }
    // 多级点分编号：1 / 1.1 / 1.1.1 ……（≥1 段点，至少两段才算「多级」；
    // 裸 `4.2` 形态按段点数计层级）。后随字符限定防误吞数值/日期
    // （`2026.11` 六位不匹配；`3.5.2 条款` 后随空格 ✓）。
    if let Some(level) = dotted_number_level(t) {
        return Some((NumberingStyle::ArabicMultiLevel, level));
    }
    // 括号编号：（一）（1）(2)——全角/半角括号
    if regex!(r"^[（(][0-9一二三四五六七八九十]{1,3}[）)]").is_match(t) {
        return Some((NumberingStyle::ChineseParen, 2));
    }
    // 圈数字：Unicode 数字圈四段区间（见 [`is_circled_digit`]）。
    // G1 只枚举到 ⑿/⑽/⒐；真实语料（塔里木 570 行）证实 ⑸ 等带括圈号
    // 高频在位——区间判定覆盖 11-50 的扩展序号（纯增益，语料零扰动）。
    if t.chars().next().is_some_and(is_circled_digit) {
        return Some((NumberingStyle::Circled, 3));
    }
    // 中文数字 + 顿号：一、二、
    if regex!(r"^[一二三四五六七八九十]+、").is_match(t) {
        return Some((NumberingStyle::ChineseNumeral, 1));
    }
    // 阿拉伯数字 + 点/顿：1. 2、3．（1-3 位——`2026.` 年份形态不吞）
    if regex!(r"^[0-9]{1,3}[.．、]").is_match(t) {
        return Some((NumberingStyle::ArabicDot, 1));
    }
    // 字母 + 括号：a) b)
    if regex!(r"^[a-zA-Z][）)]").is_match(t) {
        return Some((NumberingStyle::AlphaParen, 2));
    }
    // 项目符号：● ○ ■（任务书七形态清单；△/▲ 是几何/数学符号——
    // 真实语料「式中：△P」为价格差额记号，不属列举，不入面）
    if regex!(r"^[●○■]").is_match(t) {
        return Some((NumberingStyle::Bullet, 4));
    }
    None
}

/// 多级点分编号层级：`1.1.1` → 3；`4.2` → 2；`1.` → 不匹配（单段由
/// `ArabicDot` 分支处理，这里要求 ≥2 段）。后随字符限定：段点串之后必须是
/// 点/顿/空白/开括号/行尾——`3.5.2` ✓、`1.1.1.2` ✓、`2026.11.01`（4 段数字
/// 超长不匹配第一段）不吞。
fn dotted_number_level(t: &str) -> Option<u8> {
    let re = regex!(r"^([0-9]{1,3}(?:\.[0-9]{1,3})+)(?:[.．、\s]|\(（|$)");
    let m = re.captures(t)?;
    let segs = m[1].split('.').count() as u8;
    (segs >= 2).then_some(segs)
}

/// Unicode 数字圈四段区间（编号用途；字母圈 U+249C..=U+24E9 排除在外——
/// ⒜ 是装饰不是序号）：
///
/// - U+2460..=U+249B：①-⑳ / ⑴-⑳ / ⒈-⒛（三套 1-20）
/// - U+24EA..=U+24FF：⓪ ⓫-⓴ ⓵-⓾ ⓿（零 / 反白 11-20 / 双圈 1-10）
/// - U+3251..=U+325F：㉑-㉟（21-35）
/// - U+32B1..=U+32BF：㊱-㊿（36-50）
fn is_circled_digit(c: char) -> bool {
    matches!(
        c,
        '\u{2460}'..='\u{249B}'
            | '\u{24EA}'..='\u{24FF}'
            | '\u{3251}'..='\u{325F}'
            | '\u{32B1}'..='\u{32BF}'
    )
}

/// 混排语境层级修正（可选增强——确定性状态机）。
///
/// 唯一修正规则（公文惯例）：`ArabicDot`（`1.`）在中文形态
/// （[`NumberingStyle::ChineseNumeral`] / [`NumberingStyle::ChineseParen`]）
/// 出现过之后，重定级为「最近中文形态层级 + 1」——对应公文序列
/// `一、`(1) → `（一）`(2) → `1.`(3)。其余形态恒取基线。
///
/// **适用边界（如实登记）**：真实语料实证存在 `一、`（文件组成清单）与
/// `1、`（章节编号）**平级并列**的文件（3 件水科 PDF 目录页均为此形态）——
/// 对这类文件语境修正会高估 `1、` 的层级。因此本类型是显式可选面：
/// 需要公文语境的消费方（如结构化大纲）选用；抽取主链不接线。
///
/// 章节边界由调用方 [`NumberingContext::reset`] 重置（`第X章` 形态的
/// 自动探测属结构层职责，不在本模块）。
#[derive(Debug, Clone, Default)]
pub struct NumberingContext {
    /// 最近一次中文形态的层级（未见过 = None）
    last_cjk_level: Option<u8>,
}

impl NumberingContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// 章节边界重置（新章节的编号体系独立——由调用方在章节头处调用）。
    pub fn reset(&mut self) {
        self.last_cjk_level = None;
    }

    /// 带语境修正的层级判定（输入行序即文档行序）。
    ///
    /// 无状态部分与 [`detect_numbering`] 同判；仅 `ArabicDot` 在中文语境
    /// 下按公文惯例重定级。
    pub fn level_of(&mut self, line: &str) -> Option<(NumberingStyle, u8)> {
        let (style, baseline) = detect_numbering(line)?;
        let level = match (style, self.last_cjk_level) {
            (NumberingStyle::ArabicDot, Some(cjk)) => cjk + 1,
            _ => baseline,
        };
        // 状态更新：中文形态锚点（一、=1 /（一）=2）
        self.last_cjk_level = match style {
            NumberingStyle::ChineseNumeral => Some(1),
            NumberingStyle::ChineseParen => Some(2),
            _ => self.last_cjk_level,
        };
        Some((style, level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style_of(line: &str) -> Option<NumberingStyle> {
        detect_numbering(line).map(|(s, _)| s)
    }

    fn level_of(line: &str) -> Option<u8> {
        detect_numbering(line).map(|(_, l)| l)
    }

    /// 任务书验收 1：七种编号形态都能识别。
    #[test]
    fn all_seven_styles_detected() {
        assert_eq!(style_of("一、总则"), Some(NumberingStyle::ChineseNumeral));
        assert_eq!(
            style_of("（一）投标人应具备资格"),
            Some(NumberingStyle::ChineseParen)
        );
        assert_eq!(style_of("1. 报价一次包干"), Some(NumberingStyle::ArabicDot));
        assert_eq!(
            style_of("1.1.1 详细评审"),
            Some(NumberingStyle::ArabicMultiLevel)
        );
        assert_eq!(style_of("①现场核查"), Some(NumberingStyle::Circled));
        assert_eq!(
            style_of("a)副本与正本一致"),
            Some(NumberingStyle::AlphaParen)
        );
        assert_eq!(style_of("●配套材料清单"), Some(NumberingStyle::Bullet));
    }

    /// 任务书验收 2：层级推断合理（1.1.1 = 三级；段点数自描述）。
    #[test]
    fn dotted_levels_self_describing() {
        assert_eq!(level_of("1.1 项目概况"), Some(2));
        assert_eq!(level_of("1.1.1 详细评审"), Some(3));
        assert_eq!(level_of("4.2 投标人应当具备下列条件"), Some(2));
        assert_eq!(level_of("1.1.1.2 评分细则"), Some(4));
        assert_eq!(level_of("2.1 项目概况：本项目位于伊犁州"), Some(2));
    }

    /// 各形态基线层级（模块表逐值锁定——与 G1 clause_boundary 同值）。
    #[test]
    fn baseline_levels() {
        assert_eq!(level_of("一、总则"), Some(1));
        assert_eq!(level_of("（一）资格"), Some(2));
        assert_eq!(level_of("(一)项目管理机构组成表"), Some(2)); // 半角括号（真实语料形态）
        assert_eq!(level_of("1、总则"), Some(1));
        assert_eq!(level_of("1．总则"), Some(1)); // 全角点（真实语料形态）
        assert_eq!(level_of("①现场核查"), Some(3));
        assert_eq!(level_of("⑸造价执业人员"), Some(3)); // 带括圈号（塔里木高频形态）
        assert_eq!(level_of("a)副本"), Some(2));
        assert_eq!(level_of("●清单"), Some(4));
        assert_eq!(style_of("1、总则"), Some(NumberingStyle::ArabicDot));
    }

    /// 圈数字区间扩展：11-20 序号与 ㉑-㊿（G1 只枚举到 ⑿——纯增益面），
    /// 字母圈（⒜）不入面。
    #[test]
    fn circled_range_covers_high_ordinals() {
        for sample in [
            "⑾", "⑿", "⑬", "⑱", "⑳", "⒀", "⒘", "⒛", "㉑", "㉚", "㊿", "⓫", "⓵", "⓪",
        ] {
            assert_eq!(
                style_of(&format!("{sample}后续条款")),
                Some(NumberingStyle::Circled),
                "{sample} 应识别为圈数字"
            );
        }
        // 字母圈是装饰不是序号
        assert_eq!(detect_numbering("⒜选项说明"), None);
        assert_eq!(detect_numbering("Ⓩ附录"), None);
    }

    /// 非编号反例：普通句子 / 符号用法 / 年份 / 空行。
    #[test]
    fn non_numbering_lines() {
        assert_eq!(detect_numbering("投标报价为一次包干总价"), None);
        assert_eq!(detect_numbering("式中： △P -- 需调整的价格差额；"), None); // △ 是数学记号
        assert_eq!(detect_numbering("2026.11.01 开标"), None); // 4 段数字超长
        assert_eq!(detect_numbering(""), None);
        assert_eq!(detect_numbering("   "), None);
        assert_eq!(detect_numbering("详见第三章规定"), None);
    }

    /// 前导空白容忍（真实 PDF 行常有缩进）。
    #[test]
    fn leading_whitespace_tolerated() {
        assert_eq!(
            style_of("    （1）资质条件"),
            Some(NumberingStyle::ChineseParen)
        );
        assert_eq!(level_of("  1.1 适用范围"), Some(2));
    }

    /// 任务书验收 3：混合编号的文件不乱（一、下面跟 1.1——基线从属正确）。
    #[test]
    fn mixed_numbering_subordination_stable() {
        // 无状态基线：一、(1) → 1.1(2) → 1.1.1(3)，从属单调
        let doc = [
            "一、投标人须知",
            "1.1 项目概况",
            "1.1.1 工期要求",
            "（一）财务要求",
        ];
        let levels: Vec<u8> = doc.iter().filter_map(|l| level_of(l)).collect();
        assert_eq!(levels, vec![1, 2, 3, 2]);
        // 一、 之下的 1.1 必须更深（验收例）
        assert!(level_of("1.1 项目概况").unwrap() > level_of("一、投标人须知").unwrap());
    }

    /// 语境修正（公文惯例）：一、→（一）→ 1. 的 1. 重定级为 3。
    #[test]
    fn context_promotes_arabic_dot_after_cjk() {
        let mut ctx = NumberingContext::new();
        assert_eq!(
            ctx.level_of("一、总则"),
            Some((NumberingStyle::ChineseNumeral, 1))
        );
        assert_eq!(
            ctx.level_of("（一）适用范围"),
            Some((NumberingStyle::ChineseParen, 2))
        );
        assert_eq!(
            ctx.level_of("1. 报价一次包干"),
            Some((NumberingStyle::ArabicDot, 3))
        ); // 2+1
           // 新的一、出现后锚点更新：1. → 2
        assert_eq!(
            ctx.level_of("二、评标办法"),
            Some((NumberingStyle::ChineseNumeral, 1))
        );
        assert_eq!(
            ctx.level_of("1. 评标方法"),
            Some((NumberingStyle::ArabicDot, 2))
        );
    }

    /// 语境修正不碰纯阿拉伯文档（1. → 1.1 → 1.1.1 自洽）。
    #[test]
    fn context_leaves_pure_arabic_alone() {
        let mut ctx = NumberingContext::new();
        assert_eq!(
            ctx.level_of("1. 总则"),
            Some((NumberingStyle::ArabicDot, 1))
        );
        assert_eq!(
            ctx.level_of("1.1 项目概况"),
            Some((NumberingStyle::ArabicMultiLevel, 2))
        );
        assert_eq!(
            ctx.level_of("1.1.1 工期"),
            Some((NumberingStyle::ArabicMultiLevel, 3))
        );
    }

    /// 章节边界重置：跨章编号体系独立。
    #[test]
    fn context_reset_between_chapters() {
        let mut ctx = NumberingContext::new();
        ctx.level_of("一、总则");
        ctx.level_of("（一）适用范围");
        assert_eq!(
            ctx.level_of("1. 报价"),
            Some((NumberingStyle::ArabicDot, 3))
        );
        ctx.reset(); // 第二章起用纯阿拉伯体系
        assert_eq!(
            ctx.level_of("1. 招标文件"),
            Some((NumberingStyle::ArabicDot, 1))
        );
    }

    /// 非编号行不改状态（散文夹在编号之间不冲掉语境锚点）。
    #[test]
    fn prose_keeps_context_state() {
        let mut ctx = NumberingContext::new();
        ctx.level_of("（一）适用范围");
        assert!(ctx.level_of("中间一段普通说明文字").is_none());
        assert_eq!(
            ctx.level_of("1. 报价"),
            Some((NumberingStyle::ArabicDot, 3))
        );
    }

    /// baseline_level 与 detect_numbering 对非歧义形态逐值一致。
    #[test]
    fn baseline_level_matches_detection() {
        for line in ["一、总则", "（一）资格", "①核查", "a)副本", "●清单"] {
            let (s, l) = detect_numbering(line).unwrap();
            assert_eq!(s.baseline_level(), l, "{line}");
        }
    }

    /// 确定性：同输入两次调用同输出。
    #[test]
    fn deterministic() {
        let doc = ["一、总则", "1.1 适用范围", "①核查", "●清单", "a)副本"];
        let a: Vec<_> = doc.iter().filter_map(|l| detect_numbering(l)).collect();
        let b: Vec<_> = doc.iter().filter_map(|l| detect_numbering(l)).collect();
        assert_eq!(a, b);
    }
}
