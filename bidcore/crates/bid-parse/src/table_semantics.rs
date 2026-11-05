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

// ============================================================================
// PDF 布局面（#266）：-layout 线性化文本的语义表区域接管
//
// 真实政采 PDF（黑龙江平台，songling/batch1 五题实测）的商务要求表/
// 技术参数表经 pdftotext -layout → collapse_spaces 后以「词序列」形态
// 流入 PdfLine 块——列位空白已坍缩，行结构仍在：
//
//   表头：`序号 参数性质 类型 要求`（商务表）/ `序号 参数性质 具体技术（参数）要求`
//   行：  `1 ★ 服务期限 2026 年 12 ⽉ 31 ⽇之前完成`（序号 + ★ + 类型 + 要求）
//         `3 ★ 验收要求`（要求列在续行——多行单元格）
//         `16 ★ ★多外置接口开放：…`（hq-dev 行内★变体——★ 前缀参数文本）
//
// 行级正则链对无编号行（服务期限/服务地点/验收要求）零覆盖 → 商务要求 0、
// ★行丢失（#269 批次一五题交叉证实）。本面 = 表头锚定 + 行文法 + 续行吸附
// + 区域接管（#144 review_map 同款契约：条目 + 消费块位，被消费块不走旧链）。
// ============================================================================

/// PDF 布局面语义表条目（一行）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdfSemItem {
    /// 语句面（「类型：要求」或行内参数全文——数值单位完整）
    pub statement: String,
    /// ★实质性条款（参数性质列/行内★——params.star 载体，#276 对齐）
    pub star: bool,
    /// 表序 + 行号（条款号「表N-行M」）
    pub table_no: u32,
    pub row: u32,
    /// 行首块位（含续行的全部块位在 block_idxs——锚面/消费面共用）
    pub block_idxs: Vec<usize>,
    /// 行首块 locator
    pub locator: String,
    /// 表所在小节/章上下文（「三、商务要求」——类别裁决：含「商务」→商务要求，
    /// 否则技术参数。小节级显式命名优先于章级推断——songling REQ-0159/0160
    /// 误归技术参数的根因就是章级映射盖过了「三、商务要求」）
    pub section: String,
}

/// PDF 布局面语义表头识别（norm 后词序无关）。
///
/// 排除面（is_legacy_table 同族纪律）：评审/审查/评分（review_table #144
/// 接管）、条款号（前附表——否决行归旧链）、格式/承诺/履历/业绩（#234
/// 模板面）。识别形态：
/// - `序号 参数性质 [类型] 要求`（政采范本商务表/参数表通用骨架——songling、
///   batch1 五题同形）
/// - `参数名称|参数要求`、`条款名称|内容`（docx 面词集在 PDF 线性文本的投影）
fn pdf_sem_header(t: &str) -> bool {
    let n = crate::norm_n1(t);
    if n.chars().count() > 40 {
        return false;
    }
    if n.contains("评审") || n.contains("审查") || n.contains("评分") || n.contains("条款号")
        || n.contains("格式") || n.contains("承诺") || n.contains("履历") || n.contains("业绩")
        || n.contains("一览表")
    {
        return false;
    }
    let has = |k: &str| n.contains(k);
    // 政采范本四列骨架：序号 + 参数性质 +（类型|要求）
    if has("序号") && has("参数性质") && (has("类型") || has("要求")) {
        return true;
    }
    // docx 面词集投影（参数名称|参数要求 / 条款名称|内容）
    if has("参数名称") && (has("参数要求") || has("参数值")) {
        return true;
    }
    if has("条款名称") && has("内容") {
        return true;
    }
    false
}

/// PDF 布局面行解析：`[★] 序号 [★] [标签] [:：] 内容` 的容错文法。
///
/// 返回 `(seq, star, label, content)`；label 允许为空（hq-dev 行内★参数
/// 文本形——`★11.偏振光镜头罩须通过…` 无标签/内容切分点，整行入内容）。
/// 不匹配（散文/表头/小节头等）返回 None。
fn parse_pdf_sem_row(t: &str) -> Option<(u32, bool, String, String)> {
    let star = t.contains('★');
    let lead_star = t.trim_start().starts_with('★');
    // 去前导 ★（行内★参数形）与首尾空白
    let s: String = t
        .trim()
        .trim_start_matches('★')
        .trim_start()
        .to_string();
    if s.is_empty() {
        return None;
    }
    // 序号面：1-3 位数字（可带 .、、尾）
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let (seq, rest) = if !digits.is_empty() && digits.len() <= 3 {
        let after = s[digits.len()..].trim_start_matches(['.', '、', '．']);
        // 剥序号后的独立 ★ token（参数性质列——`1 ★ 服务期限 …`）；
        // 行内 ★（`★多外置…`——★ 后无空白）不在此剥，属标签本体
        let mut r = after.to_string();
        loop {
            match r.strip_prefix('★') {
                Some(r2) if r2.starts_with(char::is_whitespace) || r2.is_empty() => {
                    r = r2.trim_start().to_string();
                }
                _ => break,
            }
        }
        // 无分隔连排（`24投标文件…` 前附表形态）非本面；★ 前缀行
        // （`★11.偏振光…`）——前导 ★ 已证明是行，放行
        if r == after
            && !after.starts_with(char::is_whitespace)
            && !after.starts_with('★')
            && !lead_star
        {
            return None;
        }
        (digits.parse().unwrap_or(0), r.trim_start().to_string())
    } else if lead_star {
        // 行内★参数形（无序号）：整行入内容（标签/内容切分交给下段）
        (0u32, s.trim().to_string())
    } else {
        return None;
    };
    if rest.is_empty() {
        return None;
    }
    // 标签/内容切分：首个 ：/： 前（≤24 字）是标签；否则首 token（≤16 字、
    // 非数字头）为标签。标签内行内★前缀剥除（语句面干净）。
    let rest = rest.trim_start_matches('★').trim_start().to_string();
    if rest.is_empty() {
        return None;
    }
    let (label, content) = match rest.find(['：', ':']) {
        Some(ci) if ci <= 24 => {
            let sep_len = rest[ci..].chars().next().map(char::len_utf8).unwrap_or(1);
            let l = rest[..ci].trim().to_string();
            let c = rest[ci + sep_len..].to_string();
            // 标签面校验：非空、非数字头、无流标点（「期次1，说明」是单元格
            // 内容的编号子项冒号形，非行标签——lenient 判负落入续行）
            let l_ok = !l.is_empty()
                && l.chars().next().map(|ch| !ch.is_ascii_digit()).unwrap_or(false)
                && !l.contains('，') && !l.contains('。') && !l.contains('；');
            if l_ok {
                (l, c)
            } else {
                (String::new(), rest.trim().to_string())
            }
        }
        _ => {
            let mut it = rest.splitn(2, char::is_whitespace);
            let first = it.next().unwrap_or("").to_string();
            let second = it.next().unwrap_or("").trim().to_string();
            let label_ok = first.chars().count() <= 16
                && first.chars().next().map(|c| !c.is_ascii_digit()).unwrap_or(false);
            if label_ok {
                (first, second)
            } else {
                // 无干净标签切分（行内参数全文）——整行入内容
                (String::new(), rest.trim().to_string())
            }
        }
    };
    // 纯序号行（`1 ★`——★后无内容）：占位行不产条目但也不终结区域
    if label.is_empty() && content.is_empty() {
        return None;
    }
    // 无标签的序号形态（`1、进度款，从…`——多行单元格内容的编号子项）不是
    // 新行：-layout 交错下单元格首行先于行标签出现，无 ★ 且无干净标签的
    // 序号文本一律视为续行（star 行除外——★ 在场即行标记）。
    if label.is_empty() && !star && seq > 0 {
        return None;
    }
    Some((seq, star, label, content))
}

/// 页脚页码行（`-第18⻚-` / `第 18 页`——collapse 后无空白变体也在面内）。
fn is_page_marker(t: &str) -> bool {
    let n: String = t
        .trim()
        .trim_start_matches(['-', '—', ' '])
        .trim_end_matches(['-', '—', ' '])
        .to_string();
    let b = n.trim_start_matches("第").trim_end_matches("⻚").trim_end_matches("页");
    !b.is_empty() && b.chars().all(|c| c.is_ascii_digit())
}

/// 小节/章头行（区域终结信号）——`三、商务要求` / `第三章 …`（含康熙部首
/// 归一形态——章节正则与 extract.rs 同口径走 NFKC 匹配面）。
fn is_section_boundary(t: &str) -> Option<bool> {
    let n = crate::norm_n1(t);
    if n.is_empty() || n.chars().count() > 40 {
        return None;
    }
    let is_sub = n.starts_with('一') || n.starts_with('二') || n.starts_with('三')
        || n.starts_with('四') || n.starts_with('五') || n.starts_with('六')
        || n.starts_with('七') || n.starts_with('八') || n.starts_with('九')
        || n.starts_with('十');
    let sub_form = t.contains('、') && is_sub;
    let chapter_form = t.contains("第") && (t.contains("章") || n.contains("章"));
    Some(sub_form || chapter_form)
}

/// PDF 布局面语义表区域接管（review_map 同款契约）。
///
/// 返回 (条目, 消费块位)。消费块位含表头与全部数据/续行块——被消费块不走
/// 旧链（extract.rs 主循环跳过），防双抽（`1、期次1…` 编号单元格旧路
/// REQ-0159/0160 由装配行整体取代）。`skip` 为上游接管（review_table）
/// 已消费的块位——语义面不与其重叠。
pub fn extract_pdf_semantic_tables(
    f: &ParsedFile,
    skip: &std::collections::HashSet<usize>,
) -> (Vec<PdfSemItem>, Vec<usize>) {
    let mut items: Vec<PdfSemItem> = Vec::new();
    let mut consumed: Vec<usize> = Vec::new();
    let lines: Vec<(usize, &Block)> = f
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, b)| {
            !skip.contains(i) && matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine)
        })
        .collect();
    let mut table_no = 0u32;
    let mut section = String::new();
    let mut format_zone = false;
    let mut front_zone = false;
    let mut i = 0usize;
    while i < lines.len() {
        let (idx, b) = lines[i];
        let t = b.text.trim();
        // 上下文追踪（表外）：小节/章头更新 section；格式章整段跳过（#234）
        if is_page_marker(t) {
            i += 1;
            continue;
        }
        if let Some(true) = is_section_boundary(t) {
            section = t.to_string();
            let n = crate::norm_n1(t);
            format_zone = n.contains("格式");
            // #266 前附表纪律：须知/公告区的前附表归值行面/旧链（否决行语义）
            // ——语义表不接管该区（防「投标有效期/保证金」类值行落错类）
            front_zone = n.contains("须知") || n.contains("公告") || n.contains("前附");
            i += 1;
            continue;
        }
        if format_zone || front_zone {
            i += 1;
            continue;
        }
        if pdf_sem_header(t) {
            // 表头锚定：向后装配行与续行
            let (next, new_items, used) =
                assemble_pdf_sem_rows(&lines, i + 1, table_no + 1, &section);
            if new_items.is_empty() {
                i += 1; // 表头后无数据行（引用行/孤立表头）——只推进
                continue;
            }
            table_no += 1;
            items.extend(new_items);
            consumed.push(idx); // 表头块入消费集（旧链不再见它）
            consumed.extend(used.iter().copied());
            i = next;
            continue;
        }
        i += 1;
    }
    (items, consumed)
}

/// 表头后的行装配：行（序号/★文法）开新条目，非行文本吸附为续行（多行
/// 单元格），区域在 小节/章头 / 表头 / ★脚注 / 长散文 处终结。
fn assemble_pdf_sem_rows(
    lines: &[(usize, &Block)],
    start: usize,
    tno: u32,
    section: &str,
) -> (usize, Vec<PdfSemItem>, Vec<usize>) {
    let mut items: Vec<PdfSemItem> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    let mut cur: Option<(u32, bool, String, String, Vec<usize>, String)> = None; // (seq,star,label,content,idxs,locator)
    let mut i = start;
    let mut no_row_streak = 0u32;
    while i < lines.len() {
        let (idx, b) = lines[i];
        let t = b.text.trim();
        if is_page_marker(t) {
            i += 1; // 页码行既不终结也不吸附（跨页表）
            continue;
        }
        // 终结信号：小节/章头 / 新表头 / ★脚注（「打★号条款为实质性条款…」）
        if is_section_boundary(t) == Some(true) || pdf_sem_header(t) || t.contains("实质性条款") {
            break;
        }
        match parse_pdf_sem_row(t) {
            Some((seq, star, label, content)) => {
                if let Some((s, st, l, c, idxs, loc)) = cur.take() {
                    push_item(&mut items, tno, s, st, l, c, idxs, loc, section);
                }
                cur = Some((seq, star, label, content, vec![idx], b.locator.clone()));
                used.push(idx);
                no_row_streak = 0;
            }
            None => {
                // 续行吸附：短/中断句文本进当前行内容；连续两条无主文本
                // （长散文——表后正文）终结区域
                let can_attach = cur.is_some()
                    && t.chars().filter(|c| !c.is_whitespace()).count() >= 2
                    && t.chars().count() <= 60;
                if can_attach {
                    let (_, _, _, c, idxs, _) = cur.as_mut().unwrap();
                    if !c.is_empty() {
                        c.push(' ');
                    }
                    c.push_str(t);
                    idxs.push(idx);
                    used.push(idx);
                    if t.chars().count() >= 40 && t.ends_with('。') {
                        // 长完整句收尾——本单元格大概率结束（防过度吸附）
                        if let Some((s, st, l, cc, idxs2, loc)) = cur.take() {
                            push_item(&mut items, tno, s, st, l, cc, idxs2, loc, section);
                        }
                    }
                } else {
                    no_row_streak += 1;
                    if no_row_streak >= 2 || cur.is_none() {
                        break;
                    }
                }
            }
        }
        i += 1;
    }
    if let Some((s, st, l, c, idxs, loc)) = cur.take() {
        push_item(&mut items, tno, s, st, l, c, idxs, loc, section);
    }
    (i, items, used)
}

fn push_item(
    items: &mut Vec<PdfSemItem>,
    tno: u32,
    seq: u32,
    star: bool,
    label: String,
    content: String,
    idxs: Vec<usize>,
    locator: String,
    section: &str,
) {
    // 占位行（无标签无内容）不产条目
    if label.is_empty() && content.trim().is_empty() {
        return;
    }
    let statement = if label.is_empty() {
        content.trim().to_string()
    } else if content.trim().is_empty() {
        // `3 ★ 验收要求`——内容全在标签（后续行若未吸附，语句面=标签）
        label.clone()
    } else {
        format!("{label}：{content}")
    };
    items.push(PdfSemItem {
        statement,
        star,
        table_no: tno,
        row: if seq > 0 { seq } else { items.len() as u32 + 1 },
        block_idxs: idxs,
        locator,
        section: section.to_string(),
    });
}

// ============================================================================
// PDF 值行面（#266-B2）：前附表/公告区的关键值行参数化
//
// 前附表行（`5 投标有效期 ⾃提交投标⽂件的截⽌之⽇起90⽇`、`18 履约保证⾦
// 采购包1：不缴纳`）与公告区值行（`4.预算⾦额： 1,195,800.00元`）承载
// 废标关键值（保证金/有效期/预算/限价/代理费——songling 金标准 GR-010~
// 014 的值面），但无编号形态不进任何行级正则分支。本面 = 关键标签词表 +
// 行首锚定 + 值含数字守卫的定向抽取（不加类别噪声——词表仅收商务值标签）。
// ============================================================================

/// 关键值标签词表（政采范本前附表/公告区标准字段——全国通用，非省特异）。
const PDF_VALUE_LABELS: &[&str] = &[
    "投标有效期",
    "投标保证金",
    "履约保证金",
    "保证金",
    "预算金额",
    "最高限价",
    "采购包预算",
    "代理服务费",
    "服务费收费标准",
    "报价形式",
    "中标后分包",
];

/// PDF 值行条目。
#[derive(Debug, Clone, PartialEq)]
pub struct PdfValueItem {
    /// 语句面（整行原文——值与单位保真）
    pub statement: String,
    /// 命中标签（条款号用「值-{标签}」）
    pub label: String,
    /// 行块位
    pub block_idx: usize,
    pub locator: String,
}

/// 值行抽取：行首（可带 1-3 位序号/点号）紧跟关键标签 + 值段含数字。
/// `skip` = 语义表/审查表已消费块位——不重叠。
pub fn extract_pdf_value_rows(
    f: &ParsedFile,
    skip: &std::collections::HashSet<usize>,
) -> Vec<PdfValueItem> {
    let mut items = Vec::new();
    for (i, b) in f.blocks.iter().enumerate() {
        if skip.contains(&i) || !matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine) {
            continue;
        }
        let t = b.text.trim();
        if t.is_empty() || t.chars().count() > 120 {
            continue;
        }
        // 标签匹配在 NFKC 归一化副本上（真实 PDF 文本层的康熙部首 ⾦/⽇ 等
        // ——#267 面；语句面保原文）。归一仅取行首头部（标签区+值段开头）。
        let head_bytes: usize = t.chars().take(40).map(char::len_utf8).sum();
        let head = crate::norm_n1(&t[..head_bytes.min(t.len())]);
        let hs = head.as_str();
        // 去可选序号前缀（`5 ` / `4.` / `18、`）与采购包前缀（`采购包1保证金…`）
        let mut s = hs;
        let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && digits.len() <= 3 {
            s = s[digits.len()..]
                .trim_start_matches(['.', '、', '．', ' ', '　']);
        }
        if let Some(rest) = s.strip_prefix("采购包") {
            let rd: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !rd.is_empty() && rd.len() <= 2 {
                s = rest[rd.len()..].trim_start_matches(['：', ':', ' ', '　']);
            }
        }
        let Some(label) = PDF_VALUE_LABELS.iter().find(|l| s.starts_with(**l)) else {
            continue;
        };
        // 值段须含数字（日期/金额/期限——纯文字说明行不进面）；「不缴纳」
        // 类无数字但同为废标关键值——标签词表内标签 + 「缴纳」词也放行
        let value_part = &s[label.len()..];
        let has_digit = value_part.chars().any(|c| c.is_ascii_digit());
        let bujiao = value_part.contains("不缴纳") || value_part.contains("不予缴纳");
        // 排除面：见前附表类引用行（值在别处）与超短断头
        if !(has_digit || bujiao)
            || value_part.trim_start_matches(['：', ':', ' ']).len() < 3
        {
            continue;
        }
        items.push(PdfValueItem {
            statement: t.to_string(),
            label: (*label).to_string(),
            block_idx: i,
            locator: b.locator.clone(),
        });
    }
    items
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

    // ==================== PDF 布局面（#266） ====================

    fn pdfline(idx: usize, locator: &str, text: &str) -> Block {
        Block {
            idx,
            kind: BlockKind::PdfLine,
            locator: locator.to_string(),
            text: text.to_string(),
            cells: vec![],
        }
    }

    fn pdfpf(blocks: Vec<Block>) -> ParsedFile {
        ParsedFile {
            name: "光纤租赁费招标文件.pdf".into(),
            sha256: "test".into(),
            pages: 1,
            tables: 0,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks,
            parse_summary: "test".into(),
            components: vec![],
        }
    }

    /// songling 商务表真实形态（collapse 后词序列——#269 批次一五题同形）：
    /// 无编号行（服务期限/服务地点/验收要求）+ 编号多行单元格（行4/5）+ ★脚注。
    #[test]
    fn pdf_sem_assembles_commercial_table_rows() {
        let blocks = vec![
            pdfline(0, "第17页 行38", "序号 参数性质 类型 要求"),
            pdfline(1, "第17页 行39", "1 ★ 服务期限 2026 年 12 ⽉ 31 ⽇之前完成"),
            pdfline(2, "第17页 行40", "2 ★ 服务地点 ⿊⻰江省⼤兴安岭地区松岭区"),
            pdfline(3, "第18页 行2", "3 ★ 验收要求"),
            pdfline(4, "第18页 行3", "符合国家、省、⾏业技术标准的相关要求以及招标⼈验收合格标准，强化服务"),
            pdfline(5, "第18页 行4", "⽔平，实现对⽹络运⾏状态全时段感知和信息共享。"),
            pdfline(6, "第18页 行5", "4 ★ 履约验收⽅式 1、期次1，说明：按要求验收"),
            pdfline(7, "第18页 行6", "5 ★ 合同⽀付⽅式 完成服务验收合格后⽀付"),
            pdfline(8, "第18页 行7", "打“★”号条款为实质性条款，若有⼀条不响应或不满⾜则导致响应⽆效。"),
        ];
        let out = extract_pdf_semantic_tables(&pdfpf(blocks), &Default::default());
        assert_eq!(out.0.len(), 5, "五行全装配（含无编号行与多行单元格）");
        let r1 = &out.0[0];
        assert!(r1.star, "★实质性标记入位");
        assert_eq!(r1.statement, "服务期限：2026 年 12 ⽉ 31 ⽇之前完成");
        let r3 = &out.0[2];
        assert_eq!(r3.statement, "验收要求：符合国家、省、⾏业技术标准的相关要求以及招标⼈验收合格标准，强化服务 ⽔平，实现对⽹络运⾏状态全时段感知和信息共享。");
        assert!(r3.block_idxs.len() >= 2, "多行单元格续行入锚面");
        // 消费集：表头 + 全部行/续行；★脚注行不消费（非条款——留给旧链终止语境）
        assert!(out.1.contains(&0) && out.1.contains(&4), "表头与续行入消费集");
    }

    /// hq-dev 行内★变体：`16 ★ ★多外置接口开放：…` / `★11.偏振光…`
    #[test]
    fn pdf_sem_inline_star_param_form() {
        let blocks = vec![
            pdfline(0, "第17页 行29", "序号 参数性质 具体技术（参数）要求"),
            pdfline(1, "第17页 行30", "16 ★ ★多外置接⼝开放：不需要借助⼯作站，⾎压计⾃⾝可直接连接扫码枪"),
            pdfline(2, "第17页 行31", "★11.偏振光镜头罩须通过细胞毒性、⽪肤刺激检验。"),
        ];
        let out = extract_pdf_semantic_tables(&pdfpf(blocks), &Default::default());
        assert_eq!(out.0.len(), 2, "行内★两形态均装配");
        assert!(out.0.iter().all(|i| i.star), "行内★标记入位");
        assert!(out.0[0].statement.contains("多外置接⼝开放：不需要"),
            "行内★标签：内容切分保真");
    }

    /// 排除面：审查表/前附表/格式模板表头不接管（review_table #144 与 #234 纪律）。
    #[test]
    fn pdf_sem_excludes_legacy_and_review_tables() {
        for header in [
            "序号 评审项目 评审内容 合格条件",
            "序号 条款号 条款名称 内容",
            "序号 评分细则 分值",
        ] {
            assert!(!pdf_sem_header(header), "排除表头：{header}");
        }
        assert!(pdf_sem_header("序号 参数性质 类型 要求"));
        assert!(pdf_sem_header("序号 参数性质 具体技术（参数）要求"));
    }

    /// 值行面：前附表/公告区关键值（songling 金标准 GR-010~014 值面）。
    #[test]
    fn pdf_value_rows_extract_key_amounts() {
        let blocks = vec![
            pdfline(0, "第1页 行29", "4.预算⾦额： 1,195,800.00元"),
            pdfline(1, "第2页 行10", "采购包1保证1保证⾦⾦额：17,000.00元"),
            pdfline(2, "第3页 行14", "代理服务费收费标准：本项⽬代理服务费：20000元（贰万元整）"),
            pdfline(3, "第4页 行30", "5 投标有效期 ⾃提交投标⽂件的截⽇之⽇起90⽇"),
            pdfline(4, "第4页 行33", "18 履约保证⾦ 采购包1：不缴纳"),
            // 反例：引用行（无值）与超长说明行不进面
            pdfline(5, "第8页 行3", "投标保证⾦⾦额及要求⻅投标人须知前附表"),
        ];
        let out = extract_pdf_value_rows(&pdfpf(blocks), &Default::default());
        // 行1「采购包1保证⾦⾦额」——保证金标签在「采购包1」前缀后，不锚定行首
        // （前缀剥离只去数字序号）——该形态由公告区正则链兜底或后续扩面，如实计数
        assert!(out.iter().any(|v| v.statement.contains("1,195,800")), "预算金额");
        assert!(out.iter().any(|v| v.statement.contains("20000元")), "代理费");
        assert!(out.iter().any(|v| v.statement.contains("90⽇")), "投标有效期");
        assert!(!out.iter().any(|v| v.statement.contains("⻅投标人须知前附表")), "引用行不进面");
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
