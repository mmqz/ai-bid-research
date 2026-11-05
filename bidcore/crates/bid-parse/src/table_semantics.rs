//! #250 表格抽取专路——参数表/商务表的「表头 + 数据行」结构化装配。
//!
//! 现状（issue #250 实测，真实光纤租赁项目 TF-0003：1418 块 / 26 表）：
//! 招标文件的实质条款大量在表格里（技术参数表、商务要求表、资格审查表、
//! 评分表），而 `TableRow` 的既有抽取分支只覆盖型号谱系参数表（`re_model`
//! ——「GW-8000-T4」类）与三张审查表（review_table.rs）。「295 条 30M/s」
//! 「光缆链路完好率 ≥95%/月」「≤360 分钟」这类【数量+单位】参数全漏，
//! 商务表分支不存在——实测技术参数 1 条（仅表头行）、商务要求 0 条。
//!
//! 本模块是**纯识别与行装配层**（零模型、零 I/O——review_table.rs 同款
//! 纪律）：
//! - 表头形状识别（[`recognize_param_header`] / [`recognize_commercial_header`]）：
//!   参数表 = 「参数名称 | 参数要求」类双列；商务表 = 「条款名称 | 内容」
//!   类双列 + 章节商务守卫。**前附表（含「条款号」列）与审查表（含
//!   「评审」列）显式排除**——前附表否决行归既有链（`前附表-N` 条款号
//!   语义），审查表归 review_table（#144 红线）；
//! - 数据行按列语义装配（一行一条参数/要求）：语句面 `params.statement`
//!   =「参数名：参数值」（数值与单位不丢——#250 验收），★实质性条款入
//!   `params.star`；锚面 excerpt 保真（块原文，创建时固化——GT 对齐纪律）；
//! - 章节守卫：参数表须在「技术/采购需求」章、商务表须在「商务」章；
//!   格式章（#234 纪律）与前附表 locator 显式排除。
//!
//! 与 extract.rs 的分工：本模块只产出结构化条目与消费块位；REQ 化
//! （分类/条款号/marked_by）由 extract.rs 统一接线——**区域接管**
//! （#144 review_map 同款：被接管的块位不走旧碎片链，防双抽）。

use crate::{Block, BlockKind, ParsedFile};
use std::collections::HashMap;

/// 表格语义条目（一行——技术参数或商务要求）。
#[derive(Debug, Clone, PartialEq)]
pub struct TableItem {
    /// 语义组装语句（「参数名：参数值」——数值单位完整，#250 验收面）
    pub statement: String,
    /// ★实质性条款（单元格内嵌 ★——废标线信号，入 params.star）
    pub star: bool,
    /// 表内行号（条款号后缀）
    pub row: u32,
    /// 原文定位（块 locator）
    pub locator: String,
    /// 锚定块位（区域接管的消费键）
    pub block_idx: usize,
}

/// 全文件语义表装配产物（块位 → 条目）。
#[derive(Debug, Clone, Default)]
pub struct SemanticTables {
    /// 参数表条目——分类「技术参数」
    pub params: HashMap<usize, TableItem>,
    /// 商务表条目——分类「商务要求」
    pub commercial: HashMap<usize, TableItem>,
}

/// 参数表的名称列关键词（政采范本常见表头）。
fn is_param_name_header(cell: &str) -> bool {
    let c = crate::norm_n1(cell);
    ["参数名称", "参数要求", "技术参数", "技术要求", "服务要求", "服务项目", "指标要求", "服务标准"]
        .iter()
        .any(|k| c.contains(k))
}

/// 参数表的取值列关键词。
fn is_param_value_header(cell: &str) -> bool {
    let c = crate::norm_n1(cell);
    ["参数值", "参数要求", "具体要求", "招标要求", "要求", "标准", "数值", "内容", "服务标准"]
        .iter()
        .any(|k| c.contains(k))
}

/// 商务表的名称列关键词。
fn is_comm_name_header(cell: &str) -> bool {
    let c = crate::norm_n1(cell);
    ["条款名称", "商务条款", "商务要求", "项目名称"]
        .iter()
        .any(|k| c.contains(k))
}

/// 商务表的取值列关键词。
fn is_comm_value_header(cell: &str) -> bool {
    let c = crate::norm_n1(cell);
    ["条款内容", "内容", "要求", "说明", "具体内容"]
        .iter()
        .any(|k| c.contains(k))
}

/// 序号/备注列（不参与 name/value 语义对——装配时忽略）。
fn is_meta_header(cell: &str) -> bool {
    let c = crate::norm_n1(cell);
    c == "序号" || c.contains("备注") || c.contains("评审") || c.contains("条款号")
}

/// 前附表/审查表守卫：这些表归既有链（前附表否决行语义 / review_table
/// #144 接管），不走本专路——防双抽与语义降级。
fn is_legacy_table(cells: &[String]) -> bool {
    let c: Vec<String> = cells.iter().map(|x| crate::norm_n1(x)).collect();
    // 前附表：含「条款号」列（条款号|条款名称|内容 三列形态）
    if c.iter().any(|x| x.contains("条款号")) {
        return true;
    }
    // 审查表：review_table 的两形状（评审内容+合格条件 / 评审项目+标准分）
    if crate::review_table::recognize_header(cells).is_some() {
        return true;
    }
    // 评分细则类列（评分细则/评分标准）
    c.iter().any(|x| x.contains("评分细则") || x.contains("评分标准"))
}

/// 参数表表头识别：name 列 + value 列（i≠j，均为参数面关键词；序号/
/// 备注/评审列不参与）。返回 (name_col, value_col)。
pub fn recognize_param_header(cells: &[String]) -> Option<(usize, usize)> {
    if is_legacy_table(cells) {
        return None;
    }
    let name = cells
        .iter()
        .position(|c| !is_meta_header(c) && is_param_name_header(c))?;
    let value = cells
        .iter()
        .position(|c| !is_meta_header(c) && is_param_value_header(c))?;
    if name == value {
        return None;
    }
    Some((name, value))
}

/// 商务表表头识别：name 列 + value 列（i≠j）。前附表「条款号|条款名称|
/// 内容」被 is_legacy_table 排除——政采前附表恰是商务条款的索引家，但
/// 其否决行语义归既有链（`前附表-N`），不得被本专路接管降级。
pub fn recognize_commercial_header(cells: &[String]) -> Option<(usize, usize)> {
    if is_legacy_table(cells) {
        return None;
    }
    let name = cells
        .iter()
        .position(|c| !is_meta_header(c) && is_comm_name_header(c))?;
    let value = cells
        .iter()
        .position(|c| !is_meta_header(c) && is_comm_value_header(c))?;
    if name == value {
        return None;
    }
    Some((name, value))
}

/// 表键（locator 去「 行N」尾——同表行分组键，extract.rs table_key_of 同款）。
fn table_key_of(locator: &str) -> &str {
    match locator.find(" 行") {
        Some(i) => &locator[..i],
        None => locator,
    }
}

/// 表起章节名（table_key 去「 表N」尾）。
fn chapter_of_key(key: &str) -> &str {
    match key.find(" 表") {
        Some(i) => &key[..i],
        None => key,
    }
}

/// 行号（locator「行N」尾解析）。
fn row_no_of(locator: &str) -> u32 {
    locator
        .rsplit("行")
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// 章节守卫：参数表须在技术/采购需求章；格式章（#234——模板表不进
/// 抽取面）与空章名排除。
fn param_chapter_ok(chapter: &str) -> bool {
    let c = crate::norm_n1(chapter);
    !c.is_empty() && !c.contains("格式") && (c.contains("技术") || c.contains("采购需求"))
}

/// 章节守卫：商务表须在商务章；格式章排除。
fn comm_chapter_ok(chapter: &str) -> bool {
    let c = crate::norm_n1(chapter);
    !c.is_empty() && !c.contains("格式") && c.contains("商务")
}

/// 单元格有效内容判定（去空白后非空且非纯序号/占位）。
fn meaningful_cell(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && !t.chars().all(|c| c.is_ascii_digit())
}

/// 数据行装配：name/value 两列按语义组装语句（「名：值」），★实质性
/// 判定 = 任一有效格含 ★。返回 None = 该行不产条目（占位/空值/纯序号）。
fn assemble_row(cells: &[String], name_col: usize, value_col: usize) -> Option<(String, bool)> {
    let name = cells.get(name_col).map(|s| s.trim()).unwrap_or("");
    let value = cells.get(value_col).map(|s| s.trim()).unwrap_or("");
    let star = cells.iter().any(|c| c.contains('★'));
    // 占位行（□复选/下划线留白）不抽——veto_lexicon 同款守卫
    if crate::veto_lexicon::is_placeholder(&format!("{name}{value}")) {
        return None;
    }
    let (n_ok, v_ok) = (meaningful_cell(name), meaningful_cell(value));
    // 表头粘连防御：「序号」等 meta 头作为 name/value 命中时无意义
    if is_meta_header(name) && is_meta_header(value) {
        return None;
    }
    if n_ok && v_ok {
        Some((format!("{name}：{value}"), star))
    } else if v_ok && !n_ok {
        // 无名称列形态（单列要求表）：值即语句
        Some((value.to_string(), star))
    } else if n_ok && !v_ok {
        // 名称有值无要求值——「服务地点：见备注」类空值行不产（参数不完整
        // 产碎片——#250 修的就是碎片，不再造新的）
        None
    } else {
        None
    }
}

/// DOCX 语义表全装配：TableRow 按 locator 表键分组 → 首块表头识别 →
/// 数据行逐行装配。审查表/前附表/评分表（is_legacy_table）不进。
pub fn extract_docx_semantic_tables(f: &ParsedFile) -> SemanticTables {
    let mut out = SemanticTables::default();
    // 表键保序（HashMap 无序——行装配需按文档序）
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&Block>> = HashMap::new();
    for b in &f.blocks {
        if b.kind != BlockKind::TableRow || b.locator.is_empty() {
            continue;
        }
        let key = table_key_of(&b.locator).to_string();
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(b);
    }
    for key in order {
        let rows = &groups[&key];
        let Some(head) = rows.first() else { continue };
        let chapter = chapter_of_key(&key);
        // 形状优先级：参数表 > 商务表（关键词面不相交，双保险顺序）
        let shape = if let Some((nc, vc)) = recognize_param_header(&head.cells) {
            if param_chapter_ok(chapter) {
                Some(('p', nc, vc))
            } else {
                None
            }
        } else if let Some((nc, vc)) = recognize_commercial_header(&head.cells) {
            if comm_chapter_ok(chapter) {
                Some(('c', nc, vc))
            } else {
                None
            }
        } else {
            None
        };
        let Some((face, name_col, value_col)) = shape else {
            continue;
        };
        // 数据行（跳过表头行 = 组内首块）
        for b in rows.iter().skip(1) {
            let Some((statement, star)) = assemble_row(&b.cells, name_col, value_col) else {
                continue;
            };
            let item = TableItem {
                statement,
                star,
                row: row_no_of(&b.locator),
                locator: b.locator.clone(),
                block_idx: b.idx,
            };
            if face == 'p' {
                out.params.insert(b.idx, item);
            } else {
                out.commercial.insert(b.idx, item);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(idx: usize, locator: &str, cells: &[&str]) -> Block {
        Block {
            idx,
            kind: BlockKind::TableRow,
            locator: locator.to_string(),
            text: cells
                .iter()
                .filter(|c| !c.trim().is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            cells: cells.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn pf(blocks: Vec<Block>) -> ParsedFile {
        ParsedFile {
            name: "光纤租赁费招标文件-文本修复版v5w.docx".into(),
            sha256: "test".into(),
            pages: 1,
            tables: 2,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks,
            parse_summary: "test".into(),
            components: vec![],
        }
    }

    /// 表头识别：参数表双列（参数名称|参数要求）。
    #[test]
    fn param_header_recognized() {
        let cells = vec!["序号".into(), "参数名称".into(), "参数要求".into()];
        assert_eq!(recognize_param_header(&cells), Some((1, 2)));
    }

    /// 前附表排除：含「条款号」列 → 不识别（既有链保留）。
    #[test]
    fn front_matter_table_excluded() {
        let cells = vec!["条款号".into(), "条款名称".into(), "内容".into()];
        assert!(recognize_param_header(&cells).is_none());
        assert!(recognize_commercial_header(&cells).is_none());
    }

    /// 审查表排除：评审内容+合格条件（review_table #144 接管面）。
    #[test]
    fn review_table_excluded() {
        let cells = vec!["序号".into(), "评审内容".into(), "合格条件".into()];
        assert!(recognize_param_header(&cells).is_none());
        assert!(recognize_commercial_header(&cells).is_none());
    }

    /// 参数表全链：光纤租赁真实形态（完好率/360分钟/295条30M/s——#250
    /// 验收关键词）逐行语义装配，数值单位完整。
    #[test]
    fn param_table_assembles_semantic_rows() {
        let blocks = vec![
            tr(0, "第三章 采购需求 > 二、技术要求 表1 行1",
               &["序号", "参数名称", "参数要求"]),
            tr(1, "第三章 采购需求 > 二、技术要求 表1 行2",
               &["1", "光纤链路数量", "295 条"]),
            tr(2, "第三章 采购需求 > 二、技术要求 表1 行3",
               &["2", "上行带宽", "≥30M/s（可平滑升级至 50M/s）"]),
            tr(3, "第三章 采购需求 > 二、技术要求 表1 行4",
               &["3", "光缆链路完好率", "≥95%/月"]),
            tr(4, "第三章 采购需求 > 二、技术要求 表1 行5",
               &["4", "单条链路平均历时", "≤360 分钟"]),
            tr(5, "第三章 采购需求 > 二、技术要求 表1 行6",
               &["★", "服务地点", "黑龙江省大兴安岭地区松岭区"]),
        ];
        let out = extract_docx_semantic_tables(&pf(blocks));
        assert_eq!(out.params.len(), 5, "表头行不产条目，5 数据行全装配");
        assert!(out.commercial.is_empty());
        let joined: Vec<&str> = out.params.values().map(|i| i.statement.as_str()).collect();
        assert!(joined.contains(&"光缆链路完好率：≥95%/月"), "完好率数值单位完整: {joined:?}");
        assert!(joined.contains(&"单条链路平均历时：≤360 分钟"), "360分钟完整: {joined:?}");
        assert!(joined.contains(&"光纤链路数量：295 条"), "295 条完整: {joined:?}");
        assert!(joined.contains(&"上行带宽：≥30M/s（可平滑升级至 50M/s）"), "30M/s/50M/s 完整: {joined:?}");
        let star_item = out.params.get(&5).unwrap();
        assert!(star_item.star, "★实质性条款入 params.star");
        assert_eq!(star_item.statement, "服务地点：黑龙江省大兴安岭地区松岭区");
    }

    /// 商务表全链：保证金/代理费/预算/履约/有效期（#250 丢失清单）逐条装配。
    #[test]
    fn commercial_table_assembles_semantic_rows() {
        let blocks = vec![
            tr(0, "第三章 采购需求 > 三、商务要求 表2 行1",
               &["序号", "条款名称", "内容"]),
            tr(1, "第三章 采购需求 > 三、商务要求 表2 行2",
               &["1", "投标保证金", "17,000.00 元（A级 8,500）"]),
            tr(2, "第三章 采购需求 > 三、商务要求 表2 行3",
               &["2", "采购代理服务费", "20,000 元"]),
            tr(3, "第三章 采购需求 > 三、商务要求 表2 行4",
               &["3", "预算/最高限价", "1,195,800.00 元"]),
            tr(4, "第三章 采购需求 > 三、商务要求 表2 行5",
               &["4", "履约保证金", "不缴纳"]),
            tr(5, "第三章 采购需求 > 三、商务要求 表2 行6",
               &["5", "投标有效期", "90 日"]),
        ];
        let out = extract_docx_semantic_tables(&pf(blocks));
        assert_eq!(out.commercial.len(), 5);
        assert!(out.params.is_empty());
        let joined: Vec<&str> = out.commercial.values().map(|i| i.statement.as_str()).collect();
        assert!(joined.contains(&"投标保证金：17,000.00 元（A级 8,500）"), "{joined:?}");
        assert!(joined.contains(&"采购代理服务费：20,000 元"), "{joined:?}");
        assert!(joined.contains(&"预算/最高限价：1,195,800.00 元"), "{joined:?}");
        assert!(joined.contains(&"履约保证金：不缴纳"), "{joined:?}");
        assert!(joined.contains(&"投标有效期：90 日"), "{joined:?}");
    }

    /// 章节守卫：格式章（#234 纪律）与无章节的表不装配。
    #[test]
    fn format_chapter_and_untitled_tables_excluded() {
        let blocks = vec![
            tr(0, "第六章 投标文件格式 表1 行1",
               &["序号", "参数名称", "参数要求"]),
            tr(1, "第六章 投标文件格式 表1 行2",
               &["1", "格式项", "按格式填写"]),
            tr(2, "表2 行1", &["序号", "参数名称", "参数要求"]),
            tr(3, "表2 行2", &["1", "无名参数", "无章守卫"]),
        ];
        let out = extract_docx_semantic_tables(&pf(blocks));
        assert!(out.params.is_empty(), "格式章/空章名不进专路");
    }

    /// 数据行守卫：占位行（□/下划线留白）与纯序号行不产碎片。
    #[test]
    fn placeholder_and_seq_only_rows_skipped() {
        let blocks = vec![
            tr(0, "第三章 采购需求 > 二、技术要求 表1 行1",
               &["序号", "参数名称", "参数要求"]),
            tr(1, "第三章 采购需求 > 二、技术要求 表1 行2",
               &["1", "", ""]),
            tr(2, "第三章 采购需求 > 二、技术要求 表1 行3",
               &["2", "参数甲", "□符合 □不符合"]),
            tr(3, "第三章 采购需求 > 二、技术要求 表1 行4",
               &["3", "参数乙", "具体值 X"]),
        ];
        let out = extract_docx_semantic_tables(&pf(blocks));
        assert_eq!(out.params.len(), 1, "纯序号行与占位行不产条目");
        let it = out.params.values().next().unwrap();
        assert_eq!(it.statement, "参数乙：具体值 X");
    }

    /// 商务表章节守卫：非商务章的「条款名称|内容」表（如须知章）不装配。
    #[test]
    fn commercial_shape_requires_commercial_chapter() {
        let blocks = vec![
            tr(0, "第二章 投标人须知 表1 行1",
               &["序号", "条款名称", "内容"]),
            tr(1, "第二章 投标人须知 表1 行2",
               &["1", "踏勘现场", "不组织"]),
        ];
        let out = extract_docx_semantic_tables(&pf(blocks));
        assert!(out.commercial.is_empty(), "须知章的同形表归既有链");
    }
}
