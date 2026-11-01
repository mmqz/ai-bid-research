//! 条款边界与跨行合并（B6 · #123 任务 1 / G1）。
//!
//! 问题面（真实招标文件实测）：docx 解析按**段**（Para 块）产出，一条条款被
//! 显式分页符/软换行切成多个段时，抽取层按段消费——
//! · 编号头段「3.7.2(8) 投标人应当……」被抽成一条 REQ（**内容不全**，
//!   词表判定只看到前半句）；
//! · 后半段（无编号、无独立信号）落入兜底分支，丢失它属于哪个条款的上下文。
//!
//! 本模块给出**纯函数**的合并规则（任务书口径）：
//!
//! 1. 编号行（`4.2` / `（一）` / `①` / `1.` / `一、`）开启新条款；
//! 2. 非编号且非空的行 → 归入上一条款（续行）；
//! 3. 空行 → 条款结束；
//! 4. 表格行（`| … |` 形态）→ 不参与合并（表格流由 TableRow 通道处理）。
//!
//! 设计纪律：本层只做**边界判定与分组**（确定性、零外部依赖、可单测），
//! 不做分类——分类（否决/资格/评分/章节映射）留在 extract.rs 的既有
//! 优先级链里，用合并后的**完整条款文本**参与判定（内容不全 → 判定不全
//! 是 G1 的根因）。PDF 侧的 P0-A 段落重组（extract_main_pdf）已覆盖
//! 行级续行；本模块的纯函数面同时服务 docx 侧接线与后续编号层级（G5）。

/// 条款边界判定（跨行合并的核心）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClauseBoundary {
    /// 是否为编号开头（4.2 / （一） / ① / a) / 一、 …）
    pub is_numbered: bool,
    /// 编号层级（见 [`clause_level`]）。
    pub level: u8,
}

/// 合并后的条款（行组的确定性视图）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    /// 构成条款的行号（入参下标，含头行，升序）。
    pub lines: Vec<usize>,
    /// 头行的层级（非编号头 = 0）。
    pub level: u8,
    /// 条款全文（组内行序拼接；行文本保持原文——excerpt 保真不改写）。
    pub text: String,
}

/// 编号形态识别（G1 最小面；G5 将扩成 `NumberingStyle` 全枚举）。
/// 返回 Some(level)：多级点分 `1.1.1` = 段点数+1；括号数字 `（一）` = 2；
/// 圈数字 `①` = 3；中文顿号 `一、` = 1；`1.` / `1、` = 1；`a)` = 2。
/// 非编号行返回 None。
pub fn clause_boundary(line: &str) -> Option<ClauseBoundary> {
    let t = line.trim_start();
    // 多级点分编号：1 / 1.1 / 1.1.1 ……（≥1 段点，至少两段才算「多级」；
    // 裸 `4.2` 形态按段点数计层级）
    if let Some(level) = dotted_number_level(t) {
        return Some(ClauseBoundary { is_numbered: true, level });
    }
    // 括号编号：（一）（1）(2)——全角/半角括号
    if regex!(r"^[（(][0-9一二三四五六七八九十]{1,3}[）)]").is_match(t) {
        return Some(ClauseBoundary { is_numbered: true, level: 2 });
    }
    // 圈数字：① ⑴ ⒈
    if regex!(r"^[①②③④⑤⑥⑦⑧⑨⑩⑾⑿⑴⑵⑶⑷⑸⑹⑺⑻⑼⑽⒈⒉⒊⒋⒌⒍⒎⒏⒐]").is_match(t) {
        return Some(ClauseBoundary { is_numbered: true, level: 3 });
    }
    // 中文数字 + 顿号：一、二、
    if regex!(r"^[一二三四五六七八九十]+、").is_match(t) {
        return Some(ClauseBoundary { is_numbered: true, level: 1 });
    }
    // 阿拉伯数字 + 点/顿：1. 2、3．
    if regex!(r"^[0-9]{1,3}[.．、]").is_match(t) {
        return Some(ClauseBoundary { is_numbered: true, level: 1 });
    }
    // 字母 + 括号：a) b)
    if regex!(r"^[a-zA-Z][）)]").is_match(t) {
        return Some(ClauseBoundary { is_numbered: true, level: 2 });
    }
    None
}

/// 多级点分编号层级：`1.1.1` → 3；`4.2` → 2；`1.` → 1（末尾点算一段）。
/// 前提：行首即数字（由调用方保证 trim 过）。
fn dotted_number_level(t: &str) -> Option<u8> {
    let re = regex!(r"^([0-9]{1,3}(?:\.[0-9]{1,3})+)(?:[.．、\s]|\(（|$)");
    let m = re.captures(t)?;
    let segs = m[1].split('.').count() as u8;
    // 「1.」形单段编号由前一段点分支处理；这里 ≥2 段
    (segs >= 2).then_some(segs)
}

/// 局部 once-lock 正则（模块内小工具——构造一次，测试与产线同形）。
macro_rules! regex {
    ($re:expr) => {{
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| regex::Regex::new($re).unwrap())
    }};
}
use regex;

/// 合并跨行条款：编号行开头 + 后续非编号行 = 一条（任务书四规则）。
///
/// * 规则 1：编号行开启新条款
/// * 规则 2：非编号且非空的行 → 归入上一条款（无上一条款时自成一条，level 0）
/// * 规则 3：空行 → 条款结束（后续非编号行**不**再粘回前条）
/// * 规则 4：表格行（`| … |`）→ 不参与合并（原样单列，供调用方分流）
///
/// 确定性：同输入同输出（行序、分组、拼接逐字节一致）。
pub fn merge_multiline_clauses(lines: &[&str]) -> Vec<Clause> {
    let mut out: Vec<Clause> = Vec::new();
    let mut open = false; // 空行终结后不再吸收非编号行
    for (i, raw) in lines.iter().enumerate() {
        let t = raw.trim();
        let is_table_row = t.starts_with('|') && t.ends_with('|') && t.len() >= 2;
        if t.is_empty() {
            open = false; // 规则 3
            continue;
        }
        if is_table_row {
            open = false; // 表格流与条款流隔离
            out.push(Clause { lines: vec![i], level: 0, text: t.to_string() });
            continue;
        }
        if let Some(b) = clause_boundary(t) {
            out.push(Clause { lines: vec![i], level: b.level, text: t.to_string() });
            open = true; // 规则 1
        } else if open {
            // 规则 2：续行归入上一条款
            if let Some(last) = out.last_mut() {
                last.lines.push(i);
                last.text.push_str(t);
            }
        } else {
            // 无上位条款的孤儿散文行：自成一条（level 0——调用方按兜底面处理）
            out.push(Clause { lines: vec![i], level: 0, text: t.to_string() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[&str]) -> Vec<String> {
        merge_multiline_clauses(lines).iter().map(|c| c.text.clone()).collect()
    }

    /// 任务书原型：编号头 + 子项（子项是编号行——各自成条）+ 尾注续行归头。
    #[test]
    fn taskbook_example_numbered_head_absorbs_prose() {
        let lines = [
            "4.2 投标人应当具备下列条件：",
            "且满足本章其他要求，",
            "（1）具有独立承担民事责任的能力；",
            "（2）具有良好的商业信誉和健全的财务会计制度；",
        ];
        let clauses = merge_multiline_clauses(&lines);
        // 三个条款：4.2（含续行）、（1）、（2）
        assert_eq!(clauses.len(), 3, "实际 {clauses:?}");
        assert_eq!(clauses[0].level, 2);
        assert!(clauses[0].text.contains("且满足本章其他要求"), "续行并入头行");
        assert_eq!(clauses[1].lines, vec![2]);
        assert_eq!(clauses[2].lines, vec![3]);
    }

    /// 规则 2 纯形：编号行 + 无编号续行 = 一条。
    #[test]
    fn prose_continuation_joins_head() {
        let lines = ["3.7.2(8) 投标人应当", "在投标截止时间前递交保证金", "并附证明材料。"];
        let clauses = merge_multiline_clauses(&lines);
        assert_eq!(clauses.len(), 1);
        assert!(clauses[0].text.contains("递交保证金"));
        assert_eq!(clauses[0].lines, vec![0, 1, 2]);
    }

    /// 规则 3：空行终结——空行后的散文不粘回前条。
    #[test]
    fn blank_line_ends_clause() {
        let lines = ["1.1 适用范围。", "", "孤儿散文行（不粘回 1.1）"];
        let clauses = merge_multiline_clauses(&lines);
        assert_eq!(clauses.len(), 2);
        assert_eq!(clauses[0].lines, vec![0]);
        assert_eq!(clauses[1].level, 0);
    }

    /// 规则 4：表格行不参与合并。
    #[test]
    fn table_rows_stay_out() {
        let lines = ["4.1 设备参数：", "| 型号 | 数量 |", "| GW-8000 | 2 |", "其余要求如下"];
        let clauses = merge_multiline_clauses(&lines);
        // 表格行单列；「其余要求如下」不粘表格行，也不粘回 4.1（表格已断流）
        assert_eq!(clauses.len(), 4, "实际 {clauses:?}");
        assert!(clauses.iter().all(|c| c.lines.len() == 1));
    }

    /// 七种编号头各自开条 + 层级。
    #[test]
    fn numbering_styles_open_new_clause() {
        let lines = [
            "一、总则",
            "（一）投标人应具备资格",
            "1. 报价一次包干",
            "2.1 按投标报价评分",
            "①现场核查",
            "a)副本与正本一致",
        ];
        let clauses = merge_multiline_clauses(&lines);
        assert_eq!(clauses.len(), 6);
        let levels: Vec<u8> = clauses.iter().map(|c| c.level).collect();
        assert_eq!(levels, vec![1, 2, 1, 2, 3, 2], "实际 {levels:?}");
    }

    /// 1.1.1 三级 + 深层点分。
    #[test]
    fn dotted_levels() {
        let b = clause_boundary("1.1.1.2 详细评审").unwrap();
        assert!(b.is_numbered && b.level == 4);
        let b2 = clause_boundary("4.2 投标人应当具备下列条件").unwrap();
        assert_eq!(b2.level, 2);
        assert!(clause_boundary("投标报价为一次包干总价").is_none());
    }

    /// 空输入 / 全空行不炸。
    #[test]
    fn degenerate_inputs() {
        assert!(merge_multiline_clauses(&[]).is_empty());
        assert!(merge_multiline_clauses(&["", "  ", ""]).is_empty());
    }

    /// 确定性：同输入两次调用同输出。
    #[test]
    fn deterministic() {
        let lines = ["1.头", "续一", "续二", "2.次条"];
        assert_eq!(texts(&lines), texts(&lines));
    }
}
