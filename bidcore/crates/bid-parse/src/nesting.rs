//! 引号/括号嵌套配对与保护切分（B6 · #123 任务 3 / G6）。
//!
//! 问题面（任务书口径）：条款内容里的引号和括号会干扰正则切分——
//!
//! ```text
//! 「投标人应当提供」营业执照」（复印件加盖公章）
//! ```
//!
//! 朴素地按引号/括号字符切分会切错：配对结构的内容必须当**整体**。
//! 本模块提供两个纯函数（确定性、零拷贝、未配对优雅降级）：
//!
//! - [`find_matching`]：从开字符位置找**同类型**配对闭字符（跨类型嵌套
//!   透明——「（内部）」的 」 直接配 「，中间的 （） 不干扰计数）；
//! - [`split_respecting_nesting`]：按分隔符切分，但任何配对结构
//!   **内部**的分隔符不参与切分（结构是原子；配对字符本身也永不
//!   作为分隔符——要提取括号内容请先用 [`find_matching`] 定位）。
//!
//! 配对字符表（任务书面 + extract.rs 既有混用现实）：
//!
//! | 开 | 闭 | 说明 |
//! |---|---|---|
//! | `「` | `」` | 中文直角引号（一级） |
//! | `『` | `』` | 中文直角引号（二级） |
//! | `（` | `）` | 全角括号 |
//! | `【` | `】` | 方头括号 |
//! | `(` | `)` | 半角括号（条款文本全/半角混用是常态——extract.rs 正则双通道 `[（(]` 同款现实） |
//!
//! 未纳入：`《》`（书名号——引用书名语境，结构保护价值低且与切分需求无交集，
//! 如实登记；需要时在 [`PAIRS`] 加一行即可）。
//!
//! 降级语义（未配对不 panic，方向是「宁可少切，不可切错」）：
//! - 开字符无配对闭字符 → [`find_matching`] 返回 `None`；
//!   [`split_respecting_nesting`] 中该结构保护到文本末尾（尾段整体不切）；
//! - 孤立闭字符（前无开字符，OCR 损伤常见）→ 深度钳制在 0，后续切分不受
//!   破坏；该字符本身按普通字符处理。

/// 配对字符表：（开, 闭）。
pub const PAIRS: &[(char, char)] = &[
    ('「', '」'),
    ('『', '』'),
    ('（', '）'),
    ('【', '】'),
    ('(', ')'),
];

/// 开字符 → 对应闭字符。
pub fn closing_of(open: char) -> Option<char> {
    PAIRS.iter().find(|(o, _)| *o == open).map(|(_, c)| *c)
}

/// 闭字符 → 对应开字符。
fn opening_of(close: char) -> Option<char> {
    PAIRS.iter().find(|(_, c)| *c == close).map(|(o, _)| *o)
}

/// 从开字符位置找配对闭字符的位置（byte 下标）。
///
/// 语义（验收：中文引号「」『』/括号（）【】/半角 () 正确配对 + 嵌套 +
/// 未配对降级）：
///
/// - `open_pos` 是 `text` 的 **byte** 下标，必须落在 char 边界且为
///   [`PAIRS`] 中的开字符，否则返回 `None`（多字节字符中间不 panic）；
/// - **单类型深度计数**：只跟踪 `open_pos` 那个字符类型——跨类型嵌套
///   透明（「（内部）」的 」 直接配 「，`（`）` 不干扰计数）；
///   同型嵌套（「「a」b」）按深度正确配最外层；
/// - 返回配对闭字符的**起始 byte 下标**；扫描到末尾未配对 → `None`。
///
/// ```text
/// find_matching("「投标人应当提供」营业执照", 0)  → Some(9)   // 」 的位置
/// find_matching("「（内部）」", 0)               → Some(「（内部）」的 」)
/// find_matching("「abc", 0)                       → None
/// ```
pub fn find_matching(text: &str, open_pos: usize) -> Option<usize> {
    if !text.is_char_boundary(open_pos) {
        return None;
    }
    let open = text[open_pos..].chars().next()?;
    let close = closing_of(open)?;
    // 单类型深度：起点深度 1；同型开 +1、同型闭 -1；归零即配对完成
    let mut depth = 0usize;
    for (i, c) in text[open_pos..].char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(open_pos + i);
            }
        }
    }
    None
}

/// 按分隔符切分，嵌套结构内的分隔符不参与切分（验收：嵌套时不错切）。
///
/// - 任何配对字符（[`PAIRS`] 全表）开启保护：深度 > 0 期间遇到的
///   分隔符不切；配对字符**本身**也永不作为切分点（结构是原子——
///   朴素「按「」或（）切分会切错」正是本函数要替代的用法）；
/// - 深度按**全部配对类型的合计**计算（开 +1 / 闭 -1，钳制下限 0）；
/// - 孤立闭字符钳制在 0 不破坏后续切分；未闭合结构保护到末尾
///   （尾段整体保留——宁可少切不可切错）；
/// - 连续分隔符产生空段（与 `str::split` 同语义，调用方按需 filter）；
///   `separators` 为空或无命中 → 整段单元素返回。
///
/// ```text
/// split_respecting_nesting("第一条款；第二（含；子句）条款；第三", &['；'])
/// → ["第一条款", "第二（含；子句）条款", "第三"]
/// split_respecting_nesting("「a，b」c，d", &['，']) → ["「a，b」c", "d"]
/// ```
pub fn split_respecting_nesting<'a>(text: &'a str, separators: &[char]) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, c) in text.char_indices() {
        if closing_of(c).is_some() {
            depth += 1; // 开字符：进入保护（本身不切）
        } else if opening_of(c).is_some() {
            depth = depth.saturating_sub(1); // 闭字符：退出保护（本身不切）
        } else if depth == 0 && separators.contains(&c) {
            out.push(&text[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&text[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验收 1：中文引号「」『』正确配对（含嵌套）。
    #[test]
    fn chinese_quotes_pair_correctly() {
        // 「」基础配对：」 在 byte 9（「3 + 投标人应当提供 21 = 24？逐字算：
        // 「=3B，投标人应当提供=7 字 21B，」=3B → 」 起始 = 3+21 = 24）
        let t = "「投标人应当提供」营业执照";
        assert_eq!(find_matching(t, 0), Some(24));
        // 『』
        let t2 = "『内部』";
        assert_eq!(find_matching(t2, 0), Some(t2.len() - 3));
        // 嵌套：「『内』外」——中间的 『』 透明，」 配最外 「
        let t3 = "「『内』外」";
        assert_eq!(find_matching(t3, 0), Some(t3.len() - 3));
        // 同型嵌套：「「a」b」——第二个 「 加深度，第二个 」 归零
        let t4 = "「「a」b」";
        assert_eq!(find_matching(t4, 0), Some(t4.len() - 3));
        // 内层起点：从第二个 「 出发配第一个 」
        let t5 = "「「a」b」";
        // byte 布局：「(0)「(3)a(6,1B)」(7)b(10,1B)」(11)——ASCII 单字节
        assert_eq!(find_matching(t5, 3), Some(7));
    }

    /// 验收 2：中文括号（）【】（及半角 ()）正确配对。
    #[test]
    fn brackets_pair_correctly() {
        let t = "（复印件加盖公章）";
        assert_eq!(find_matching(t, 0), Some(t.len() - 3));
        let t2 = "【重点条款】";
        assert_eq!(find_matching(t2, 0), Some(t2.len() - 3));
        let t3 = "(half-width)";
        assert_eq!(find_matching(t3, 0), Some(t3.len() - 1));
        // 跨类型嵌套：（【内】外）——【】 透明
        let t4 = "（【内】外）";
        assert_eq!(find_matching(t4, 0), Some(t4.len() - 3));
        // 从内层【出发：配 】（外层 （） 透明——单类型计数）
        assert_eq!(find_matching(t4, 3), Some(3 + "【内】".len() - 3));
    }

    /// 验收 3：嵌套时不错切（结构内分隔符被保护）。
    #[test]
    fn nesting_not_split_inside() {
        // 分号切分：括号内的分号不切
        let t = "第一条款；第二（含；子句）条款；第三";
        assert_eq!(
            split_respecting_nesting(t, &['；']),
            vec!["第一条款", "第二（含；子句）条款", "第三"]
        );
        // 引号内逗号不切
        let t2 = "「a，b」c，d";
        assert_eq!(
            split_respecting_nesting(t2, &['，']),
            vec!["「a，b」c", "d"]
        );
        // 任务书例：整个脏文本（含孤立 」）按逗号切——无外层逗号则整体
        let t3 = "「投标人应当提供」营业执照」（复印件加盖公章）";
        assert_eq!(split_respecting_nesting(t3, &['，']), vec![t3]);
        // 深嵌套：【（「x，y」）】z，w
        let t4 = "【（「x，y」）】z，w";
        assert_eq!(
            split_respecting_nesting(t4, &['，']),
            vec!["【（「x，y」）】z", "w"]
        );
        // 配对字符本身是分隔符也不切（结构是原子）
        let t5 = "前段（保护内）后段";
        assert_eq!(split_respecting_nesting(t5, &['（', '）']), vec![t5]);
    }

    /// 验收 4：未配对优雅降级（不 panic）。
    #[test]
    fn unpaired_degrades_gracefully() {
        // find_matching：未闭合 / 非开字符 / 非边界
        assert_eq!(find_matching("「abc", 0), None);
        assert_eq!(find_matching("abc", 0), None); // 不是开字符
        assert_eq!(find_matching("（abc", 1), None); // byte 落在字符中间
        assert_eq!(find_matching("", 0), None);
        assert_eq!(find_matching("」」", 0), None); // 闭字符不是开字符
                                                    // split：未闭合结构保护到末尾（宁可少切不可切错）
        let t = "a（b，c";
        assert_eq!(split_respecting_nesting(t, &['，']), vec![t]);
        // split：孤立闭字符钳制 0，后续切分不受破坏
        assert_eq!(
            split_respecting_nesting("a）b，c", &['，']),
            vec!["a）b", "c"]
        );
        // 空 separators / 空文本
        assert_eq!(split_respecting_nesting("a，b", &[]), vec!["a，b"]);
        assert_eq!(split_respecting_nesting("", &['，']), vec![""]);
        // 连续分隔符（与 str::split 同语义——空段保留）
        assert_eq!(
            split_respecting_nesting("a，，b", &['，']),
            vec!["a", "", "b"]
        );
    }

    /// 半角括号与全角混用（真实条款常态——extract.rs 双通道同款现实）。
    #[test]
    fn half_width_mixes_with_full() {
        let t = "（见第3条(细则)规定）";
        assert_eq!(find_matching(t, 0), Some(t.len() - 3)); // 外层全角配对
        let inner = t.find('(').unwrap();
        assert_eq!(find_matching(t, inner), Some(t.rfind(')').unwrap()));
        // 保护切分：内外两层内的分隔符都不切
        assert_eq!(
            split_respecting_nesting("x（y(z；w)y）x；z", &['；']),
            vec!["x（y(z；w)y）x", "z"]
        );
    }

    /// 确定性：同输入两次调用同输出。
    #[test]
    fn deterministic() {
        let t = "「a（b，c）d」；e";
        assert_eq!(
            split_respecting_nesting(t, &['；']),
            split_respecting_nesting(t, &['；'])
        );
        assert_eq!(find_matching(t, 0), find_matching(t, 0));
    }
}
