//! 三张核心审查表的结构化抽取（#144）——资格性 / 符合性 / 详细评审。
//!
//! 真实招标文件第 6 章「评审」（或评标办法附表区）有三张核心审查表，
//! 是废标判定的权威面（issue #144 原文）：
//!
//! ```text
//! 表7 资格性审查表（评审内容 + 合格条件）       不满足 = 废标
//! 表8 符合性审查表（评审内容 + 合格条件×2 同名）  不满足 = 废标
//! 表9 详细评审表（序号 + 评审项目 + 标准分 + 评分细则）  评分减分不阻断
//! ```
//!
//! 本模块是**纯识别与行装配层**（零模型、零 I/O、确定性）：
//! - DOCX 侧（[`extract_docx_tables`]）：`TableRow` 块按 locator 表键分组，
//!   首行做表头形状识别（[`recognize_header`]），数据行装配为 [`ReviewItem`]
//!   （表8 双列同名「合格条件」怪癖：多列收录 + 装配去重合并）；
//! - PDF 侧（[`extract_pdf_linearized`]）：`PdfLine` 线性化审查表（附表区
//!   「形式评审表 / 初评表」等）的编号行 + 续行装配——pdftotext 把表格行
//!   拆成多条物理行（含「内容行 → 孤立序号行 → 续行」的真实件乱序形态，
//!   见 A-01/A-05 形式评审表），序号行只作行号不进锚文本（锚保真）。
//!
//! 与 extract.rs 的分工：本模块只产出结构化条目与消费块位；REQ 化
//! （类别/条款号/params.score）由 extract.rs 统一接线——表7/表8 → 必选
//! 废标面（矩阵 blocking），表9 → 评分项 + params.score 承载标准分。
//!
//! 实测背景（A-01/A-05 形式评审表漏抽根因）：行文含「单位负责人」
//! 「技术负责人」等**名词**，被既有 re_procedure 守卫的「负责」动词词形
//! 误杀（extract.rs 配套修复——名词剥离后再匹配）。

use crate::{Block, BlockKind, ParsedFile};

/// 审查表类型（#144 任务书 API）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewTableKind {
    /// 资格性审查表（表7 形态；不满足 = 废标）
    Qualification,
    /// 符合性审查表（表8 形态；两列同名「合格条件」怪癖；不满足 = 废标）
    Conformance,
    /// 详细评审表（表9 形态；标准分承载面——评分减分不 blocking）
    Detailed,
}

/// 审查表条目（一行）。#144 任务书 API + 装配扩展位（row / table_name /
/// block_idxs——行号进条款号「形式评审表-4」，块位供 extract.rs 锚定原文
/// 与区域接管）。
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewItem {
    pub kind: ReviewTableKind,
    /// 评审内容 / 评审项目（第一有效列；PDF 线性化侧 = 整行文本）
    pub item: String,
    /// 合格条件（第二列起拼接；表8 双列同名 → 去重合并）
    pub criteria: String,
    /// 标准分（仅 Detailed）
    pub score: Option<f64>,
    /// 评分细则（仅 Detailed）
    pub score_rules: Option<String>,
    /// 原文定位（首块 locator）
    pub locator: String,
    /// 表内行号（表9 序号列为准——真实件序号跳 6（1/2/3/4/5/7/8）按原样
    /// 保留；无序号面按数据行计数 1 起）
    pub row: u32,
    /// 表名核心（条款号前缀：「资格性审查表」/「形式评审表」——去掉
    /// 「表7 / 附表三：」等装饰前缀）
    pub table_name: String,
    /// 锚定块位（行装配消费的块 idx 序——excerpt 保真锚定；PDF 侧不含
    /// 孤立序号行（序号只进 row，不污染锚文本——GT 对齐口径））
    pub block_idxs: Vec<usize>,
}

/// 表头形状（识别产物）。表7/表8 表头**完全同形**（评审内容+合格条件），
/// 二者区分靠表名（资格性/资格审查 vs 符合性）——见 [`kind_from_title`]。
#[derive(Debug, Clone, PartialEq)]
pub enum HeaderShape {
    /// 评审内容 + 合格条件（表7/表8 形态）。criteria_cols 收录**全部**
    /// 「合格条件」列（表8 双列同名怪癖按多列收录，装配时去重合并）。
    QualConf {
        item_col: usize,
        criteria_cols: Vec<usize>,
    },
    /// 评审项目 + 标准分（或分值）——表9 形态。
    Detailed {
        seq_col: Option<usize>,
        item_col: usize,
        score_col: usize,
        rules_col: Option<usize>,
    },
}

/// 单元格匹配口径：NFKC + 去全部空白后的包含（lib.rs `norm_n1`——与 GT
/// 对齐判分同款规范化；康熙部首污染在 extract 入口已统一归一，此处双保险）。
fn cell_has(cell: &str, key: &str) -> bool {
    crate::norm_n1(cell).contains(key)
}

/// 表头识别（纯函数）：表格首行 cells → 形状。
///
/// 规则（issue #144 原文）：
/// - 含「评审内容」+「合格条件」→ [`HeaderShape::QualConf`]；
/// - 含「评审项目」+「标准分」（或「分值」）→ [`HeaderShape::Detailed`]
///   （评分细则列名容差：评分细则/评分标准；序号列可选）；
/// - 其余（前附表「条款名|内容」、参数表、格式模板表头等）→ None。
pub fn recognize_header(cells: &[String]) -> Option<HeaderShape> {
    // ── 表9 形态：评审项目 + 标准分/分值 ──
    let item9 = cells.iter().position(|c| cell_has(c, "评审项目"));
    let score9 = cells
        .iter()
        .position(|c| cell_has(c, "标准分") || cell_has(c, "分值"));
    if let (Some(item_col), Some(score_col)) = (item9, score9) {
        // 标准分列不得与评审项目列同格（表头粘连防呆——同格即非四列表头）
        if item_col != score_col {
            return Some(HeaderShape::Detailed {
                seq_col: cells.iter().position(|c| cell_has(c, "序号")),
                item_col,
                score_col,
                rules_col: cells
                    .iter()
                    .position(|c| cell_has(c, "评分细则") || cell_has(c, "评分标准")),
            });
        }
    }
    // ── 表7/表8 形态：评审内容 + 合格条件 ──
    let item7 = cells.iter().position(|c| cell_has(c, "评审内容"));
    let crit7 = cells.iter().position(|c| cell_has(c, "合格条件"));
    if let (Some(item_col), Some(first_criteria)) = (item7, crit7) {
        if item_col != first_criteria {
            let criteria_cols: Vec<usize> = cells
                .iter()
                .enumerate()
                .filter(|(i, c)| *i != item_col && cell_has(c, "合格条件"))
                .map(|(i, _)| i)
                .collect();
            return Some(HeaderShape::QualConf {
                item_col,
                criteria_cols,
            });
        }
    }
    None
}

/// 表名 → 表类型（表7/表8 表头同形的区分面）。
///
/// - 名含「资格性」或「资格审查」→ [`ReviewTableKind::Qualification`]；
/// - 名含「符合性」→ [`ReviewTableKind::Conformance`]；
/// - 名含「详细评审」→ [`ReviewTableKind::Detailed`]；
/// - 「形式评审表 / 初评表」（附表区资格性面的常见别名）→ Qualification；
/// - 同形无名（表名缺失）→ 默认 Qualification（表7/表8 同为必选废标面，
///   分类粒度损失不进矩阵语义——mandatory/blocking 同构）。
pub fn kind_from_title(title: &str) -> ReviewTableKind {
    let t = crate::norm_n1(title);
    if t.contains("详细评审") {
        ReviewTableKind::Detailed
    } else if t.contains("符合性") {
        ReviewTableKind::Conformance
    } else if t.contains("资格性") || t.contains("资格审查") {
        ReviewTableKind::Qualification
    } else {
        // 形式评审表/初评表等附表形态（资格性面）——同形无名默认面。
        ReviewTableKind::Qualification
    }
}

/// 表名核心化：去「表7 / 附表三：」等装饰前缀 + 去全部空白
/// （条款号前缀用：「表7 资格性审查表」→「资格性审查表」）。
pub fn table_name_core(title: &str) -> String {
    let mut s = title.trim().to_string();
    loop {
        let Some(rest) = strip_decorative_prefix(&s) else {
            break;
        };
        s = rest;
    }
    crate::norm_n1(&s)
}

/// 单轮装饰前缀剥离：表N（后随空白）/ 附表N（可选冒号）。
fn strip_decorative_prefix(s: &str) -> Option<String> {
    let re = regex::Regex::new(concat!(
        r"^(?:附\s*表\s*[0-9一二三四五六七八九十百]+\s*[：:、]?\s*",
        r"|表\s*[0-9一二三四五六七八九十百]+\s+)"
    ))
    .unwrap();
    let m = re.find(s)?;
    Some(s[m.end()..].to_string())
}

// ================================================================ DOCX 表格侧

/// 从 `ParsedFile` 识别并抽取三张审查表（#144 任务书 API）。
///
/// M1 纪律：零模型、零 I/O——纯块面识别与行装配。DOCX 表格侧（TableRow
/// 块）与 PDF 线性化附表侧（PdfLine 块）双形态自动路由；非审查表面
/// （前附表/参数表/格式模板）返回空。
pub fn extract_review_tables(f: &ParsedFile) -> Vec<ReviewItem> {
    let has_table_rows = f
        .blocks
        .iter()
        .any(|b| matches!(b.kind, BlockKind::TableRow));
    if has_table_rows {
        extract_docx_tables(f)
    } else {
        Vec::new() // PDF 线性化侧由 extract_pdf_linearized 单独驱动（带消费位）
    }
}

/// DOCX 表格侧：按 locator 表键分组（表键变化或表头形行出现即分界）→
/// 首行表头识别 → 数据行装配。
pub fn extract_docx_tables(f: &ParsedFile) -> Vec<ReviewItem> {
    let mut items = Vec::new();
    let mut group: Vec<&Block> = Vec::new();
    let mut cur_key: Option<String> = None;
    let mut flush = |group: &mut Vec<&Block>, items: &mut Vec<ReviewItem>| {
        if !group.is_empty() {
            if let Some(more) = assemble_docx_group(f, group) {
                items.extend(more);
            }
            group.clear();
        }
    };
    for b in &f.blocks {
        if b.kind != BlockKind::TableRow {
            flush(&mut group, &mut items); // 表格流结束（段落出现）
            cur_key = None;
            continue;
        }
        let key = table_key(&b.locator);
        let key_changed = cur_key.as_deref() != Some(key.as_str());
        // 分界：表键变化，或同键流中再次出现表头形行（引擎表号缺失时的
        // 相邻表防粘连——已有数据行后紧跟新表头 = 新表开始）
        let header_like = recognize_header(&b.cells).is_some();
        if key_changed || (header_like && !group.is_empty()) {
            flush(&mut group, &mut items);
            cur_key = Some(key);
        } else if cur_key.is_none() {
            cur_key = Some(key);
        }
        group.push(b);
    }
    flush(&mut group, &mut items);
    items
}

/// 单表装配：首行表头识别 + 表名回溯（表前最近段落）→ 数据行 ReviewItem。
fn assemble_docx_group(f: &ParsedFile, rows: &[&Block]) -> Option<Vec<ReviewItem>> {
    let header = rows.first()?;
    let shape = recognize_header(&header.cells)?;
    let title = lookback_title(f, header.idx);
    let kind = match shape {
        HeaderShape::QualConf { .. } => kind_from_title(&title),
        HeaderShape::Detailed { .. } => ReviewTableKind::Detailed,
    };
    let table_name = if crate::norm_n1(&title).is_empty() {
        default_table_name(kind)
    } else {
        table_name_core(&title)
    };
    let mut items = Vec::new();
    let mut data_row_no = 0u32;
    for row in rows.iter().skip(1) {
        let cells: Vec<&str> = row.cells.iter().map(|c| c.trim()).collect();
        if cells.iter().all(|c| c.is_empty()) {
            continue; // 空排版行
        }
        let (item_text, criteria, score, rules, seq_no) = match &shape {
            HeaderShape::QualConf {
                item_col,
                criteria_cols,
            } => {
                let item = cells.get(*item_col).copied().unwrap_or("");
                // 表8 双列同名怪癖：多列收录 + 逐列去重合并（同名同值 → 取一；
                // 同名异值（真实件罕见）→ 分号拼接不丢内容）
                let mut merged: Vec<&str> = Vec::new();
                for ci in criteria_cols {
                    if let Some(v) = cells.get(*ci) {
                        if !v.is_empty() && !merged.iter().any(|m| *m == *v) {
                            merged.push(v);
                        }
                    }
                }
                (item, merged.join("；"), None, None, None)
            }
            HeaderShape::Detailed {
                seq_col,
                item_col,
                score_col,
                rules_col,
            } => {
                let item = cells.get(*item_col).copied().unwrap_or("");
                let score = cells
                    .get(*score_col)
                    .and_then(|v| parse_score(v))
                    .or_else(|| cells.iter().find_map(|c| parse_score(c)));
                let rules = rules_col
                    .and_then(|ci| cells.get(ci))
                    .map(|s| s.to_string());
                let seq = seq_col
                    .and_then(|ci| cells.get(ci))
                    .and_then(|v| v.trim().parse::<u32>().ok());
                (item, String::new(), score, rules, seq)
            }
        };
        if item_text.is_empty() && criteria.is_empty() && score.is_none() {
            continue; // 无有效内容的排版行
        }
        data_row_no += 1;
        let row_no = seq_no.unwrap_or(data_row_no);
        items.push(ReviewItem {
            kind,
            item: item_text.to_string(),
            criteria,
            score,
            score_rules: rules,
            locator: row.locator.clone(),
            row: row_no,
            table_name: table_name.clone(),
            block_idxs: vec![row.idx],
        });
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

/// 表首行向前回溯表名（最近 ≤3 个非空段落；含「审查表/评审表」字样优先，
/// 否则取最近一段——空表名走 [`default_table_name`]）。
fn lookback_title(f: &ParsedFile, header_idx: usize) -> String {
    let mut seen = 0;
    let mut fallback = String::new();
    for b in f.blocks.iter().take(header_idx).rev() {
        if b.kind != BlockKind::Para {
            continue;
        }
        let t = b.text.trim();
        if t.is_empty() {
            continue;
        }
        seen += 1;
        if seen > 3 {
            break;
        }
        if t.contains("审查表") || t.contains("评审表") {
            return t.to_string();
        }
        if fallback.is_empty() {
            fallback = t.to_string();
        }
    }
    fallback
}

fn default_table_name(kind: ReviewTableKind) -> String {
    match kind {
        ReviewTableKind::Qualification => "资格性审查表".to_string(),
        ReviewTableKind::Conformance => "符合性审查表".to_string(),
        ReviewTableKind::Detailed => "详细评审表".to_string(),
    }
}

/// 表键：locator 去「 行N」后缀（extract.rs table_key_of 同款口径）。
fn table_key(locator: &str) -> String {
    match locator.find(" 行") {
        Some(i) => locator[..i].to_string(),
        None => locator.to_string(),
    }
}

/// 标准分解析：纯数字格（「30」「3.5」）或「30 分」形态。
fn parse_score(cell: &str) -> Option<f64> {
    let t = cell.trim().trim_end_matches("分").trim();
    t.parse::<f64>().ok()
}

// ================================================================ PDF 线性化附表侧

/// PDF 线性化审查表抽取（附表区「形式评审表/初评表」等）。
///
/// 返回（条目集, 消费块位集）——消费位供 extract.rs 区域接管（被装配的
/// 行不再走既有逐行分支，防双抽）；孤立序号行（「内容 → 3 → 续行」真实件
/// 乱序形态）进消费位但**不进锚**（序号只承载行号——GT 对齐口径）。
pub fn extract_pdf_linearized(f: &ParsedFile) -> (Vec<ReviewItem>, Vec<usize>) {
    let mut items: Vec<ReviewItem> = Vec::new();
    let mut consumed: Vec<usize> = Vec::new();
    let lines: Vec<(usize, &Block)> = f
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine))
        .collect();
    let mut i = 0usize;
    while i < lines.len() {
        let (idx, b) = lines[i];
        let t = b.text.trim();
        if let Some(kind) = linearized_table_title_kind(t) {
            let name = table_name_core(t);
            let (next, new_items, used) = assemble_linearized_rows(&lines, i + 1, kind, &name);
            if new_items.is_empty() {
                i += 1; // 表名后无数据行——只推进一行（可能是引用行/相邻表名行）
                continue;
            } else {
                items.extend(new_items);
                // 表名行**不进消费集**——留给 extract.rs 旧链开审查语境
                // （re_review_open「形式评审」——与基线行为一致）；消费集只含
                // 数据行（含孤立序号行）。
                consumed.extend(used.iter().copied());
                i = next;
                continue;
            }
        }
        i += 1;
    }
    (items, consumed)
}

/// 线性化表名行识别（独立成行、短、无尾标点——「形式评审表」「初评表 1」
/// 「表7 资格性审查表」形态；「2.1.1 形式评审标准」是条款行不是表名——
/// 「标准」≠「表」，天然不误配）。
fn linearized_table_title_kind(t: &str) -> Option<ReviewTableKind> {
    let n = crate::norm_n1(t);
    // 长度门：真实表名行（含「初评表1（工程名称）（标段名称）」形）≤18 字；
    // 「详见…形式评审表的相关规定说明」类引用句更长且含「详见」——都不是表名行
    if n.chars().count() > 18 || n.is_empty() || n.contains("详见") {
        return None;
    }
    if t.contains('：') || t.contains(':') {
        // 「5.1.2 资格评审标准：」类条款行——冒号结尾不是表名行
        return None;
    }
    if n.contains("工程名称") || n.contains("标段名称") {
        // 「初评表 1 （工程名称）（标段名称）」类附表编号行——带表头装饰，
        // 真表名在其上一行（「形式评审表」）；归入噪声面（is_furniture）
        return None;
    }
    if n.contains("详细评审表") {
        Some(ReviewTableKind::Detailed)
    } else if n.contains("符合性审查表") || n.contains("符合性评审表") {
        Some(ReviewTableKind::Conformance)
    } else if n.contains("资格性审查表") || n.contains("资格性评审表") {
        Some(ReviewTableKind::Qualification)
    } else if n.contains("形式评审表") || n.contains("初评表") {
        Some(ReviewTableKind::Qualification)
    } else {
        None
    }
}

/// 表名行之后的编号行装配（状态机）。
///
/// 真实件形态（A-01/A-05 形式评审表）三合一：
/// - 规范形：`N 内容`（编号行）；
/// - 乱序形：`内容` → `N`（孤立序号行）→ `续内容`（序号行在行中间）；
/// - 跨行形：`N 内容`（无尾标点）→ `续内容` → …… → `；`。
///
/// 噪声面（跳过不装配）：线性化表头（序号/评审项目/投标人/备注/ABC…）、
/// 页面装饰（招标文件/第一卷/裸页码/工程名称·标段名称/评标表-N 附表号）。
/// 终止面（表区结束）：结论/签名/注/新表名/编号条款头（x.y.z）/章节头。
fn assemble_linearized_rows(
    lines: &[(usize, &Block)],
    start: usize,
    kind: ReviewTableKind,
    name: &str,
) -> (usize, Vec<ReviewItem>, Vec<usize>) {
    let re_row = regex::Regex::new(r"^([0-9]{1,2})\s*[、.．]?\s+(\S.*)$").unwrap();
    let re_seq_alone = regex::Regex::new(r"^[0-9]{1,2}\s*[、.．]?$").unwrap();
    let re_clause = regex::Regex::new(r"^\d+(?:\.\d+)+").unwrap();
    let re_chapter = regex::Regex::new(r"^第\s*[0-9一二三四五六七八九十百]+\s*章").unwrap();
    let re_terminator = regex::Regex::new(concat!(
        "结论[:：]|评委.{0,6}签名|确认签名|监督人员|技术组评委|商务组评委|",
        "记名投票|记录人"
    ))
    .unwrap();
    let mut items: Vec<ReviewItem> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    // 待定行：(行号 or None, 文本, 锚块位（不含孤立序号行）, 首锚 locator)
    let mut pending: Option<(Option<u32>, String, Vec<usize>, String)> = None;
    let mut i = start;
    while i < lines.len() {
        let (idx, b) = lines[i];
        let t = b.text.trim();
        let n = crate::norm_n1(t);
        if n.is_empty() {
            i += 1;
            continue;
        }
        // ---- 终止面（先于噪声——结论/签名类到达即收表）----
        if re_terminator.is_match(t)
            || t.starts_with("注：")
            || t.starts_with("注:")
            || linearized_table_title_kind(t).is_some()
            // #252：综合评分表是独立表——上一张审查表在此收表（旧实现把
            // 整张评分表吞进最后一行：#251 实测 REQ-0297 单锚 1933 字符）。
            // 评分表本体由 scoring_table 专路装配。
            || crate::scoring_table::is_scoring_table_title(t)
            || re_clause.is_match(t)
            || re_chapter.is_match(t)
        {
            break;
        }
        // ---- 孤立序号行：给未编号 pending 赋行号 / 开新行 ----
        // （先于页码噪声判定——表区内孤立 1-2 位数字行 overwhelmingly 是
        // 乱序序号（A-05「3」），非页码；3 位以上才是页码面）
        if re_seq_alone.is_match(t) {
            let no: u32 = n.parse().unwrap_or(0);
            if let Some((row_no, text, _idxs, _locator)) = pending.as_mut() {
                if row_no.is_none() {
                    *row_no = Some(no); // 乱序形：序号行在行中间——只赋号不进锚
                } else {
                    flush_pending(&mut pending, &mut items, name, kind);
                    pending = Some((Some(no), String::new(), Vec::new(), b.locator.clone()));
                }
            } else {
                pending = Some((Some(no), String::new(), Vec::new(), b.locator.clone()));
            }
            used.push(idx);
            i += 1;
            continue;
        }
        // ---- 噪声面（页面装饰/线性化表头——跳过不装配）----
        if is_furniture(&n) || is_linearized_header(&n, &re_row) {
            i += 1;
            continue;
        }
        // ---- 编号行：N + 内容（开新行）----
        if let Some(c) = re_row.captures(t) {
            let no: u32 = c[1].parse().unwrap_or(0);
            let body = c
                .get(2)
                .map(|m| m.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            flush_pending(&mut pending, &mut items, name, kind);
            pending = Some((Some(no), body, vec![idx], b.locator.clone()));
            used.push(idx);
            i += 1;
            continue;
        }
        // ---- 内容行/续行 ----
        let pending_open = pending.is_some();
        let pending_terminated = pending
            .as_ref()
            .map(|(_, text, _, _)| {
                text.chars()
                    .last()
                    .map(|c| "。；！？：".contains(c))
                    .unwrap_or(true)
            })
            .unwrap_or(true);
        if pending_open && !pending_terminated {
            // 跨行形：当前行未终结 → 续行并入（含 A-01「只能有一个报价…」的
            // 孤立「；」尾行）
            let (_, text, idxs, _) = pending.as_mut().unwrap();
            text.push_str(t);
            idxs.push(idx);
            used.push(idx);
            i += 1;
            continue;
        }
        if pending_open && pending_terminated && pending.as_ref().unwrap().0.is_none() {
            // 同一行内多句（首句已终结但行号未定——如「…规定；」后接续句）：
            // 继续并入本行（序号未定前不切行）
            let (_, text, idxs, _) = pending.as_mut().unwrap();
            text.push_str(t);
            idxs.push(idx);
            used.push(idx);
            i += 1;
            continue;
        }
        // 无 pending（或已编号已终结）的内容行 → 新的未编号行（乱序形首段）
        flush_pending(&mut pending, &mut items, name, kind);
        pending = Some((None, t.to_string(), vec![idx], b.locator.clone()));
        used.push(idx);
        i += 1;
    }
    flush_pending(&mut pending, &mut items, name, kind);
    (i, items, used)
}

/// 页面装饰噪声：招标文件/第一卷/裸页码/工程名称·标段名称行/评标表-N
/// 附表号/项目名+卷次组合行。
fn is_furniture(n: &str) -> bool {
    if n == "招标文件" || n == "第一卷" || n == "招标文件第一卷" {
        return true;
    }
    // 裸页码（3 位数字——1-2 位已被孤立序号分支先行消费）
    if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && n.len() >= 3 {
        return true;
    }
    // 评标表-N（附表编号行——真表名在其后：「形式评审表」）
    if regex_is_table_ref(n) {
        return true;
    }
    // 工程名称/标段名称表头装饰行
    if (n.contains("工程名称") || n.contains("标段名称")) && n.chars().count() <= 30 {
        return true;
    }
    // A B C…… 评委列头
    if n.chars()
        .all(|c| matches!(c, 'A' | 'B' | 'C' | 'D' | '…' | '.' | '、'))
        && !n.is_empty()
    {
        return true;
    }
    false
}

fn regex_is_table_ref(n: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"^评标表[-－—–—−]*[0-9]{1,2}$").unwrap());
    re.is_match(n)
}

/// 线性化表头噪声：序号列头/评审项目列头/投标人·备注列头（短行、无编号
/// 内容形态）。
fn is_linearized_header(n: &str, re_row: &regex::Regex) -> bool {
    let short = n.chars().count() <= 16;
    if !short {
        return false;
    }
    if n.starts_with("序号") && (n.contains("评审项目") || n.len() <= 6) {
        return true;
    }
    if n.contains("评审项目") && n.chars().count() <= 10 {
        return true;
    }
    if n == "投标人" || n == "备注" || n.contains("投标人") && n.contains("备注") {
        return true;
    }
    // 评委/记录人列头（无 terminator 词命中时——「评委」独立行）
    if n == "评委" || n == "技术组评委" || n == "商务组评委" {
        return true;
    }
    // 防误伤：形如「1 投标人名称…」的编号数据行不是表头（re_row 兜底）
    re_row.is_match(n) == false && false
}

fn flush_pending(
    pending: &mut Option<(Option<u32>, String, Vec<usize>, String)>,
    items: &mut Vec<ReviewItem>,
    name: &str,
    kind: ReviewTableKind,
) {
    let Some((row_no, text, idxs, locator)) = pending.take() else {
        return;
    };
    let t = text.trim();
    if t.chars().filter(|c| !c.is_whitespace()).count() < 6 {
        return; // 碎片行（<6 字）不成条目
    }
    // 未获行号的行（表尾截断/无序号形态）→ 数据行序号兜底
    let row = row_no.unwrap_or_else(|| items.len() as u32 + 1);
    items.push(ReviewItem {
        kind,
        item: t.to_string(),
        criteria: String::new(),
        score: None,
        score_rules: None,
        locator,
        row,
        table_name: name.to_string(),
        block_idxs: idxs,
    });
}

/// 行文本中的「第X章」章节引用（#170 方案 A 的抽取侧输入）。
///
/// 真实评审表行的常见形态（实测：塔里木件「按第 8 章投标文件」、
/// 「详见本技术条款第 24 章」；伊犁件「按第五章"投标文件格式"」）：
/// 「第X章」中间可夹空白，章序数中文/阿拉伯皆可。取**首个**命中
/// （多章引用时首章为主章），连同章号后紧邻的章名余文（截至首个
/// 空白/句读，至多 12 字——「第 8 章投标文件」整体保留，便于人工
/// 核对与未来精确匹配）。无引用 → `None`（不虚构）。
pub fn chapter_ref_of(text: &str) -> Option<String> {
    let re = regex::Regex::new(
        r"第\s*([0-9０-９一二三四五六七八九十百零〇两]{1,8})\s*章",
    )
    .unwrap();
    let m = re.find(text)?;
    // 余文（章名）：截至首个空白或句读（，。；、：）——「第 8 章投标文件的
    // 要求」保留「第 8 章投标文件的要求」（章号是匹配核心，余文供人工核对）。
    let rest: String = text[m.end()..]
        .chars()
        .take_while(|c| {
            !c.is_whitespace() && !matches!(c, '，' | '。' | '；' | '、' | '：' | '（' | '）' | '（' | '）' | '"' | '"')
        })
        .take(12)
        .collect();
    Some(format!("{}{}", m.as_str(), rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ---------------- #170 方案 A：chapter_ref_of

    #[test]
    fn chapter_ref_of_real_forms() {
        // 塔里木件实测形态：章号带空格 + 紧邻章名
        assert_eq!(
            chapter_ref_of("按第 8 章投标文件的要求"),
            Some("第 8 章投标文件的要求".to_string())
        );
        // 中文数字
        assert_eq!(
            chapter_ref_of("详见本技术条款第 24 章。"),
            Some("第 24 章".to_string())
        );
        // 伊犁件形态：章名后引号
        assert_eq!(
            chapter_ref_of("按第五章\"投标文件格式\"进行编写"),
            Some("第五章".to_string())
        );
        // 无引用
        assert_eq!(chapter_ref_of("施工组织设计 30 分"), None);
        // 非章序数（第1页）不误触
        assert_eq!(chapter_ref_of("详见第1页的说明"), None);
    }

    #[test]
    fn chapter_ref_of_chinese_numerals() {
        assert_eq!(
            chapter_ref_of("响应第三章的所有要求"),
            Some("第三章的所有要求".to_string())
        );
        assert_eq!(
            chapter_ref_of("见第十二章，并附证明材料"),
            Some("第十二章".to_string())
        );
    }

    #[test]
    fn recognize_t7_header() {
        let shape = recognize_header(&cells(&["评审内容", "合格条件"]));
        assert_eq!(
            shape,
            Some(HeaderShape::QualConf {
                item_col: 0,
                criteria_cols: vec![1]
            })
        );
    }

    #[test]
    fn recognize_t8_header_dual_named_cols() {
        // 表8 怪癖：两列同名「合格条件」→ 双列收录
        let shape = recognize_header(&cells(&["评审内容", "合格条件", "合格条件"]));
        assert_eq!(
            shape,
            Some(HeaderShape::QualConf {
                item_col: 0,
                criteria_cols: vec![1, 2]
            })
        );
    }

    #[test]
    fn recognize_t9_header() {
        let shape = recognize_header(&cells(&["序号", "评审项目", "标准分", "评分细则"]));
        assert_eq!(
            shape,
            Some(HeaderShape::Detailed {
                seq_col: Some(0),
                item_col: 1,
                score_col: 2,
                rules_col: Some(3)
            })
        );
        // 「分值」容差
        let shape2 = recognize_header(&cells(&["评审项目", "分值"]));
        assert!(matches!(
            shape2,
            Some(HeaderShape::Detailed { score_col: 1, .. })
        ));
    }

    #[test]
    fn recognize_non_review_headers_none() {
        // 前附表/参数表/格式模板表头——非审查面
        assert_eq!(recognize_header(&cells(&["条款名", "内容"])), None);
        assert_eq!(
            recognize_header(&cells(&["序号", "设备名称", "数量"])),
            None
        );
        assert_eq!(recognize_header(&cells(&["姓名", "年龄", "学历"])), None);
        // 只有评审内容无合格条件 → 不构成表7/8 表头
        assert_eq!(recognize_header(&cells(&["评审内容", "备注"])), None);
        // 评审项目与标准分同格粘连 → 非四列表头
        assert_eq!(recognize_header(&cells(&["评审项目 标准分"])), None);
    }

    #[test]
    fn kind_from_title_rules() {
        assert_eq!(
            kind_from_title("表7 资格性审查表"),
            ReviewTableKind::Qualification
        );
        assert_eq!(
            kind_from_title("资格审查表"),
            ReviewTableKind::Qualification
        );
        assert_eq!(
            kind_from_title("表8 符合性审查表"),
            ReviewTableKind::Conformance
        );
        assert_eq!(kind_from_title("表9 详细评审表"), ReviewTableKind::Detailed);
        // 形式评审表/初评表 = 资格性面的附表形态
        assert_eq!(
            kind_from_title("形式评审表"),
            ReviewTableKind::Qualification
        );
        assert_eq!(kind_from_title("初评表 1"), ReviewTableKind::Qualification);
        // 同形无名默认
        assert_eq!(kind_from_title(""), ReviewTableKind::Qualification);
    }

    #[test]
    fn table_name_core_strips_decorative_prefix() {
        assert_eq!(table_name_core("表7 资格性审查表"), "资格性审查表");
        assert_eq!(table_name_core("附表三：评标用表"), "评标用表");
        assert_eq!(table_name_core("形式评审表"), "形式评审表");
        assert_eq!(table_name_core("初评表 1"), "初评表1");
    }

    #[test]
    fn parse_score_forms() {
        assert_eq!(parse_score("30"), Some(30.0));
        assert_eq!(parse_score(" 3.5 "), Some(3.5));
        assert_eq!(parse_score("30 分"), Some(30.0));
        assert_eq!(parse_score("低价优先法"), None);
    }

    #[test]
    fn linearized_title_recognition() {
        assert_eq!(
            linearized_table_title_kind("形式评审表"),
            Some(ReviewTableKind::Qualification)
        );
        assert_eq!(
            linearized_table_title_kind("初评表 1"),
            Some(ReviewTableKind::Qualification)
        );
        assert_eq!(
            linearized_table_title_kind("表9 详细评审表"),
            Some(ReviewTableKind::Detailed)
        );
        // 条款行/长行/带冒号行不是表名
        assert_eq!(linearized_table_title_kind("2.1.1 形式评审标准"), None);
        assert_eq!(
            linearized_table_title_kind("详见评标办法前附表形式评审表的相关规定说明"),
            None
        );
        assert_eq!(linearized_table_title_kind("资格评审标准："), None);
        // 附表编号 + 表头装饰行不是表名（真表名在上一行）
        assert_eq!(
            linearized_table_title_kind("初评表 1            （工程名称）   （标段名称）"),
            None
        );
    }

    #[test]
    fn furniture_detection() {
        assert!(is_furniture("招标文件"));
        assert!(is_furniture("第一卷"));
        assert!(is_furniture("143"), "三位页码是噪声");
        assert!(
            !is_furniture("43"),
            "1-2 位数字是序号域（乱序序号行），不是页码"
        );
        assert!(is_furniture("评标表-4"));
        assert!(is_furniture("（工程名称）（标段名称）"));
        assert!(is_furniture("ABC……"));
        assert!(!is_furniture("投标人名称应与营业执照、资质证书一致"));
    }

    // ---- PDF 线性化装配（真实件形态三合一）----

    fn pdf_file(lines: &[&str]) -> ParsedFile {
        let blocks = lines
            .iter()
            .enumerate()
            .map(|(i, t)| Block {
                idx: i,
                kind: BlockKind::PdfLine,
                locator: format!("第1页 行{}", i + 1),
                text: t.to_string(),
                cells: vec![],
            })
            .collect();
        ParsedFile {
            name: "A-test.pdf".into(),
            sha256: String::new(),
            pages: 1,
            tables: 0,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks,
            parse_summary: String::new(),
            components: vec![],
        }
    }

    #[test]
    fn linearized_assembly_a05_form() {
        // A-05 形式评审表形态：规范行 + 乱序行（内容→3→续行）+ 终止
        let f = pdf_file(&[
            "第三章 评标办法（综合评估法）",
            "评标办法前附表",
            "形式评审表",
            "初评表 1                                                （工程名称）   （标段名称）",
            "投标人",
            "序号              评审项目                                                  备注",
            "1    投标人名称应与营业执照、资质证书一致；",
            "2    投标文件的签名盖章符合招标文件规定；",
            "     单位负责人为同一人或者存在控股管理关系的不同单位，不得",
            "3",
            "     参加同一标段或者未划分标段的同一招标项目投标；",
            "     与招标人存在利害关系可能影响招标公正性的法人、其他组织",
            "4",
            "     或者个人不得参加投标；",
            "5    只能有一个报价（含修正报价）；",
            "        结论：该投标人是否通过形式评审",
            "评委确认签名",
            "注：投标文件有任何一项未通过的，评标委员会应当否决其投标。",
        ]);
        let (items, consumed) = extract_pdf_linearized(&f);
        assert_eq!(items.len(), 5, "五条数据行：{items:#?}");
        assert_eq!(items[0].row, 1);
        assert!(items[0].item.contains("投标人名称应与营业执照"));
        assert_eq!(items[2].row, 3, "乱序行：孤立序号 3 赋行号");
        assert!(
            items[2].item.contains("单位负责人为同一人")
                && items[2].item.ends_with("招标项目投标；"),
            "乱序行首尾两段拼接：{}",
            items[2].item
        );
        assert!(
            !items[2].item.contains('3'),
            "孤立序号行不进锚文本：{}",
            items[2].item
        );
        assert_eq!(items[4].row, 5);
        assert_eq!(items[0].table_name, "形式评审表");
        // 消费位：数据行锚块（表名/表头/结论/签名/注行不消费——表名行留给
        // 旧链开审查语境）
        assert!(!consumed.contains(&2), "表名行不消费（旧链开审查语境）");
        assert!(!consumed.contains(&5), "线性化表头行不消费");
        assert!(consumed.contains(&6), "数据行 1 被消费");
        assert!(consumed.contains(&9), "孤立序号行被消费（不进锚）");
        assert!(!consumed.contains(&16), "结论行不消费");
        assert!(!consumed.contains(&18), "注行不消费（走既有否决分支）");
    }

    #[test]
    fn linearized_assembly_a01_form() {
        // A-01 形式评审表形态：跨页装饰（招标文件/页码）+ 孤立「；」尾行
        let f = pdf_file(&[
            "附表三：评标用表",
            "形式评审表",
            "评标表-4",
            "投标人                备注",
            " 序号                         评审项目",
            "  1     投标人名称应当与营业执照、资质证书、安全生产许可证一致；",
            "        投标文件的签名盖章符合规定；施工招标项目的投标人报价文件（包括投标报价书和",
            "  2",
            "        修正报价、已标价的工程量清单、单价分析表）由造价专业人员逐页签名；",
            "  3     投标文件格式符合第 8 章投标文件格式的要求；",
            "  6     只能有一个报价（含修正报价）",
            "                     ；",
            "                                                                                       招标文件",
            "  8     投标文件的印刷与装订符合第 2 章 投标人须知第 3.7.5 款规定。",
            "        结论：该投标人是否通过形式评审",
        ]);
        let (items, _consumed) = extract_pdf_linearized(&f);
        assert_eq!(
            items.len(),
            5,
            "行 1/2/3/6/8 全装配（2 乱序、6 孤立分号）：{items:#?}"
        );
        let it2 = items.iter().find(|i| i.row == 2).unwrap();
        assert!(
            it2.item.starts_with("投标文件的签名盖章符合规定") && it2.item.ends_with("逐页签名；"),
            "乱序行拼接完整：{}",
            it2.item
        );
        let it6 = items.iter().find(|i| i.row == 6).unwrap();
        assert_eq!(
            it6.item, "只能有一个报价（含修正报价）；",
            "孤立分号尾行并入"
        );
        let it8 = items.iter().find(|i| i.row == 8).unwrap();
        assert!(
            it8.item.contains("印刷与装订"),
            "跨页装饰行（招标文件）不打断装配"
        );
    }

    #[test]
    fn linearized_assembly_stops_at_clause_and_note() {
        // 编号条款头/注行终止表区（防越界吞后续正文）
        let f = pdf_file(&[
            "形式评审表",
            "1    投标人名称应与营业执照、资质证书一致；",
            "2.1.4 否决投标情形",
            "  在初步评审表中，有任何一项未响应的，评标委员会应当否决其投标",
        ]);
        let (items, consumed) = extract_pdf_linearized(&f);
        assert_eq!(items.len(), 1);
        assert!(!consumed.contains(&2), "条款头行不被消费");
        assert!(!consumed.contains(&3), "条款正文不被消费");
    }

    // ---- DOCX 表格侧装配 ----

    fn docx_table_file() -> ParsedFile {
        // 三张表（表7/表8/表9）+ 表名段（呼中件结构复刻——验收夹具同构）
        let mut blocks: Vec<Block> = Vec::new();
        let mut idx = 0usize;
        fn push_para(blocks: &mut Vec<Block>, idx: &mut usize, text: &str) {
            blocks.push(Block {
                idx: *idx,
                kind: BlockKind::Para,
                locator: "第六章 评审".into(),
                text: text.into(),
                cells: vec![],
            });
            *idx += 1;
        }
        fn push_row(
            blocks: &mut Vec<Block>,
            idx: &mut usize,
            table_no: usize,
            row_no: usize,
            cells: &[&str],
        ) {
            blocks.push(Block {
                idx: *idx,
                kind: BlockKind::TableRow,
                locator: format!("第六章 评审 表{table_no} 行{row_no}"),
                text: cells.join(" "),
                cells: cells.iter().map(|s| s.to_string()).collect(),
            });
            *idx += 1;
        }
        push_para(&mut blocks, &mut idx, "第六章 评审");
        push_para(
            &mut blocks,
            &mut idx,
            "6.1 投标人须具备电子与智能化工程专业承包二级及以上资质。",
        );
        // 表7（表1）：表头 + 2 数据行（样本）
        push_para(&mut blocks, &mut idx, "表7 资格性审查表");
        push_row(&mut blocks, &mut idx, 1, 1, &["评审内容", "合格条件"]);
        push_row(
            &mut blocks,
            &mut idx,
            1,
            2,
            &[
                "具有独立承担民事责任的能力",
                "在中华人民共和国境内注册的法人，提交有效的营业执照",
            ],
        );
        push_row(
            &mut blocks,
            &mut idx,
            1,
            3,
            &[
                "法定代表人授权书",
                "提供「法定代表人授权书」并按要求签字、加盖公章",
            ],
        );
        // 表8（表2）：两列同名「合格条件」怪癖
        push_para(&mut blocks, &mut idx, "表8 符合性审查表");
        push_row(
            &mut blocks,
            &mut idx,
            2,
            1,
            &["评审内容", "合格条件", "合格条件"],
        );
        push_row(
            &mut blocks,
            &mut idx,
            2,
            2,
            &[
                "投标承诺书",
                "提供标准格式的「投标承诺书」并签字盖章",
                "提供标准格式的「投标承诺书」并签字盖章",
            ],
        );
        // 表9（表3）：序号跳 6 + 标准分
        push_para(&mut blocks, &mut idx, "表9 详细评审表");
        push_row(
            &mut blocks,
            &mut idx,
            3,
            1,
            &["序号", "评审项目", "标准分", "评分细则"],
        );
        push_row(
            &mut blocks,
            &mut idx,
            3,
            2,
            &["1", "投标报价", "30", "采用低价优先法计算价格分"],
        );
        push_row(
            &mut blocks,
            &mut idx,
            3,
            3,
            &["2", "技术条款", "25", "技术偏离表逐项对照"],
        );
        push_row(
            &mut blocks,
            &mut idx,
            3,
            8,
            &["7", "售后服务方案", "10", "响应时限、驻场与备件安排"],
        );
        ParsedFile {
            name: "A-评审章.docx".into(),
            sha256: String::new(),
            pages: 1,
            tables: 3,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks,
            parse_summary: String::new(),
            components: vec![],
        }
    }

    #[test]
    fn docx_assembly_three_tables() {
        let f = docx_table_file();
        let items = extract_review_tables(&f);
        assert_eq!(items.len(), 6, "2+1+3 条数据行：{items:#?}");
        // 表7
        let t7: Vec<&ReviewItem> = items
            .iter()
            .filter(|i| i.table_name == "资格性审查表")
            .collect();
        assert_eq!(t7.len(), 2);
        assert_eq!(t7[0].kind, ReviewTableKind::Qualification);
        assert!(t7[0].item.contains("独立承担民事责任"));
        assert!(t7[0].criteria.contains("提交有效的营业执照"));
        // 表8：双列同名去重合并
        let t8: Vec<&ReviewItem> = items
            .iter()
            .filter(|i| i.table_name == "符合性审查表")
            .collect();
        assert_eq!(t8.len(), 1);
        assert_eq!(t8[0].kind, ReviewTableKind::Conformance);
        assert_eq!(
            t8[0].criteria.matches("投标承诺书").count(),
            1,
            "同名同值双列只留一份：{}",
            t8[0].criteria
        );
        // 表9：序号跳 6 保留 + 标准分承载
        let t9: Vec<&ReviewItem> = items
            .iter()
            .filter(|i| i.table_name == "详细评审表")
            .collect();
        assert_eq!(t9.len(), 3);
        assert_eq!(t9[0].kind, ReviewTableKind::Detailed);
        assert_eq!(t9[0].row, 1, "序号列为准（非物理行序）");
        assert_eq!(t9[0].score, Some(30.0));
        assert!(t9[0].score_rules.as_deref().unwrap().contains("低价优先法"));
        assert_eq!(t9[2].row, 7, "真实件序号跳 6 按原样保留");
        assert_eq!(t9[2].score, Some(10.0));
    }

    #[test]
    fn docx_non_review_tables_ignored() {
        // 前附表/参数表（表头不匹配）不进审查表面
        let blocks: Vec<Block> = vec![
            Block {
                idx: 0,
                kind: BlockKind::Para,
                locator: "第二章 投标人须知".into(),
                text: "投标人须知前附表".into(),
                cells: vec![],
            },
            Block {
                idx: 1,
                kind: BlockKind::TableRow,
                locator: "第二章 投标人须知 表1 行1".into(),
                text: "条款名 条款内容".into(),
                cells: vec!["条款名".into(), "条款内容".into()],
            },
            Block {
                idx: 2,
                kind: BlockKind::TableRow,
                locator: "第二章 投标人须知 表1 行2".into(),
                text: "1.4.1 计划工期".into(),
                cells: vec!["1.4.1".into(), "计划工期：90 日历天".into()],
            },
        ];
        let f = ParsedFile {
            name: "A-x.docx".into(),
            sha256: String::new(),
            pages: 1,
            tables: 1,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks,
            parse_summary: String::new(),
            components: vec![],
        };
        assert!(extract_review_tables(&f).is_empty());
    }
}
