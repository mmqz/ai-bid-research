//! scoring_table.rs —— 综合评分表专路（issue #252 第二步：评分表驱动的输入面）。
//!
//! ## 为什么需要专路
//!
//! 真实政采 PDF（松岭 TF-0002 第 25-28 页）的《综合评分表》被 pdftotext
//! 线性化后是「列文本按行交错」的扁平流：
//!
//! ```text
//! 1.根据本项目的具体情况，供应商需提供详
//! 细、完善、合理可行的服务方案 , 包含：①
//! 整体服务方案（对本项目建设、运行维护服
//! …
//! 服务方案 每少一项扣5分，每有一项表述存在缺陷的 25.00 主观
//! 扣1分，扣完为止。（以上缺陷是指：…
//! 只有简单描述无实质性内容）
//! ```
//!
//! 既有两条路径都吃不下它：
//! - `review_table::extract_pdf_linearized` 只认「N + 内容」编号行——评分表
//!   行首是细则文本不是序号，整个表被吞进**上一张表**（符合性审查表）的
//!   最后一行（#251 实测：REQ-0297 单锚 1933 字符装下全表）；
//! - `table_semantics` 参数表专路不进审查/评分表。
//!
//! 本模块按评分表自己的形态装配：
//! - **行锚 = 分值+主客观 token**（`N.NN 主观|客观` 每行恰好一次）；
//! - **行终** = 缺陷说明样板段的闭括号（「以上缺陷是指…）」），或
//!   「不得分。」句，或区域末（下一章头）；
//! - **评审项名** = 分值行上/邻行的独立列块（空白分隔短段，优先级链见
//!   [`extract_name`]——#251 实测 8 行全验证）；
//! - **细则 ①-⑳** 在行文本内按圈号切分（**切分必须在 NFKC 之前**——NFKC
//!   会把 ① 归一成 "1"）。
//!
//! ## 产出语义（与 #236/#252 消费面对齐）
//!
//! 每行 → 一条 `评分项` REQ（clause_no `综合评分表-N`），params 携带
//! `score` / `objective` / `track` / `name` / `sub_items`。锚 excerpt =
//! `"{name}（{score}分 {track}）：{行文本}"`——前缀规范名是**重建文本的
//! 自描述头**（本路径的 excerpt 本来就是多列重建产物而非逐字摘录；NFKC
//! 归一——康熙部首变体统一——是重建的一部分，保证检索面可用）。
//!
//! ## 已知边界（如实登记，不伪装）
//!
//! - 只覆盖 PDF 线性化形态；DOCX 真表格路径（corpus-A 类）不经过本模块
//!   （其评分表消费待层2 素材对账后立项）；
//! - 行终判据（缺陷样板/不得分句）是真实政采评分表的常见收尾形态，非
//!   评分表通用文法——异常形态退化为「区域末一行」，分值 token 保证行
//!   不丢（fail-visible：装配缺陷在 [`extract_name`] 兜底名与测试面可见）。

use crate::{Block, BlockKind, ParsedFile};

/// 综合评分表行（装配产物——extract.rs 消费为评分项 REQ）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScoringRow {
    /// 表内行号（数据行计数 1 起）
    pub row: u32,
    /// 评审项名（NFKC + 去空白——如「服务方案」「业绩」）
    pub name: String,
    /// 分值（分值列）
    pub score: f64,
    /// 客观分（客观/主观列：客观=true）
    pub objective: bool,
    /// 评审轨（技术评审|商务评审|价格评审）
    pub track: String,
    /// 锚 excerpt（`{name}（{score}分 {track}）：{行文本}`——NFKC）
    pub criteria: String,
    /// 细则文本（①-⑳切分，NFKC + 去空白；无圈号 → 空）
    pub sub_items: Vec<String>,
    /// 锚定块位（行装配消费的块 idx 序——页脚除外）
    pub block_idxs: Vec<usize>,
    /// 首块定位（locator）
    pub locator: String,
}

/// 窄域部首归一：CJK 部首区（U+2E80..=U+2FDF——CJK Radicals Supplement +
/// Kangxi Radicals）逐字 NFKC 归一到统一表意文字；**其余字符原样保留**。
///
/// 不用全量 NFKC的原因：它把全角标点（，（）：）折成半角、把圈号 ① 折成
/// "1"——中文存储文本的可读性与下游规则面全部被扰动。真实招标 PDF 文本层
/// 的实际污染面**只有部首区**（实测：⽅⼯⼈⽇⻚等），窄域归一够用且更稳。
fn radical_nfkc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.chars()
        .flat_map(|c| {
            if ('\u{2E80}'..='\u{2FDF}').contains(&c) {
                c.to_string().nfkc().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// 纯窄域归一（不剥空白）——保留列块间的空格结构与全角标点。
fn nfkc(s: &str) -> String {
    radical_nfkc(s)
}

/// 窄域归一 + 去全部空白（名字/细则的存储形）。
fn nfkc_solid(s: &str) -> String {
    radical_nfkc(s)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// 表名行识别：独立成行的《综合评分表》标题（「综合评分表【综合评分法适用】」）。
///
/// 供两处消费：本模块的扫描起点 + `review_table::assemble_linearized_rows`
/// 的终止面（上一张审查表不得吞掉评分表——#251 实测 REQ-0297 的病灶）。
pub fn is_scoring_table_title(t: &str) -> bool {
    let n = crate::norm_n1(t);
    n.contains("综合评分表") && n.chars().count() <= 20
}

/// 分值+主客观 token（每行恰好一次的行锚）。返回 (分值, 是否客观)。
fn score_token(nfkc_line: &str) -> Option<(f64, bool)> {
    for obj_word in ["主观", "客观"] {
        let mut from = 0usize;
        while let Some(p) = nfkc_line[from..].find(obj_word) {
            let at = from + p;
            // 向前扫数字（允许空格间隔：「… 的 25.00 主观」）
            let head = &nfkc_line[..at];
            let mut num_end = head.len();
            while num_end > 0 {
                let c = head[..num_end].chars().last().unwrap();
                if c.is_whitespace() {
                    num_end -= c.len_utf8();
                } else {
                    break;
                }
            }
            let mut num_start = num_end;
            let mut seen_digit = false;
            let mut seen_dot = false;
            while num_start > 0 {
                let c = head[..num_start].chars().last().unwrap();
                if c.is_ascii_digit() {
                    seen_digit = true;
                    num_start -= 1;
                } else if c == '.' && !seen_dot && seen_digit {
                    seen_dot = true;
                    num_start -= 1;
                } else {
                    break;
                }
            }
            if seen_digit {
                if let Ok(v) = head[num_start..num_end].parse::<f64>() {
                    return Some((v, obj_word == "客观"));
                }
            }
            from = at + obj_word.len();
        }
    }
    None
}

/// 页脚（-第N页- 形态；⻚ U+2EDA 无 NFKC 映射，两种形都要认）。
fn is_page_footer(nfkc_solid_line: &str) -> bool {
    let t = nfkc_solid_line.trim_matches('-');
    if let Some(rest) = t.strip_prefix("第") {
        let no = rest.strip_suffix("页").or_else(|| rest.strip_suffix("⻚"));
        if let Some(no) = no {
            return !no.is_empty() && no.chars().all(|c| c.is_ascii_digit());
        }
    }
    false
}

/// 圈号字符（①-⑳，U+2460..=U+2473）。
fn circled_no(c: char) -> Option<u32> {
    let u = c as u32;
    if (0x2460..=0x2473).contains(&u) {
        Some(u - 0x2460 + 1)
    } else {
        None
    }
}

/// 细则切分：①-⑳ 圈号起段，段终=段内首个 ；/；/。。
///
/// 输入是**行文本（未 NFKC）**——NFKC 会把 ① 归一成 "1"，圈号切分必须在
/// 归一前做；每段产出时再 NFKC + 去空白。圈号前的文本（行首粘的表头残段
/// /轨道标签）自然落在首段之外被丢弃。
fn split_sub_items(row_text: &str) -> Vec<String> {
    let chars: Vec<char> = row_text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur: Option<String> = None;
    for &c in &chars {
        if circled_no(c).is_some() {
            if let Some(seg) = cur.take() {
                let s = nfkc_solid(&seg);
                if !s.is_empty() {
                    out.push(s);
                }
            }
            cur = Some(String::new());
            continue;
        }
        if let Some(seg) = cur.as_mut() {
            if c == '；' || c == ';' || c == '。' {
                // 段终标点不入段（细则短形不带尾巴）
                let s = nfkc_solid(seg);
                cur = None;
                if !s.is_empty() {
                    out.push(s);
                }
            } else {
                seg.push(c);
            }
        }
    }
    if let Some(seg) = cur.take() {
        let s = nfkc_solid(&seg);
        if !s.is_empty() {
            out.push(s);
        }
    }
    out
}

/// 评审轨标签（跨行合并单元格的列块值）。
const TRACKS: &[&str] = &["技术评审", "商务评审", "价格评审"];

/// 功能字（列块短段里出现即非评审项名——「每少一项…的」类谓语碎片）。
/// 注意**不含**「应」：「应急方案」的应是词素不是情态动词（#252 实测教训）。
const NAME_FUNC_CHARS: &[char] = &['的', '每', '得', '扣', '将', '或'];

/// 候选名判定（C1/C3/C2 用）：2-20 字、含 CJK、不以标点开头、无功能字。
fn is_name_candidate(seg: &str) -> bool {
    let n: Vec<char> = nfkc_solid(seg).chars().collect();
    if !(2..=20).contains(&n.len()) {
        return false;
    }
    if !n.iter().any(|c| ('\u{4e00}'..='\u{9fff}').contains(c)) {
        return false;
    }
    if matches!(n[0], '，' | ',' | '。' | '；' | '：' | '：' | '、' | '（' | '）' | '(' | ')') {
        return false;
    }
    !n.iter().any(|c| NAME_FUNC_CHARS.contains(c))
}

/// 评审项名提取（分值行 + 行内位置 ± 邻行的列块分析）。
///
/// 优先级链（#251 实测 8 行全验证——见模块测试 `songling_shape_eight_rows`）：
/// 1. **C1** 分值行首列块（「服务方案 每少一项… 25.00 主观」——名字列与
///    分值同行同发）；排除轨道标签与回声（首块出现在上一行文段中 =
///    跨行断字残片）；
/// 2. **C3** 轨道标签让位（「商务评审 业绩 5.00 客观」→ 次列块）；
/// 3. **C2** 分值前列块的冒号后段（「陷 的扣1分…（以上缺陷是指：质量保证
///    20.00 主观」——名字列被粘在谓语碎片尾部）；
/// 4. **C4** 邻行拼合（分值行上/下一行都是 ≤20 字短行——名字列裂成两片：
///    「项目重点、难点分析，应对」+「措施及相关的合理化建议」）；
/// 5. 兜底 `评审项N`（fail-visible——装配缺陷在测试面可见，不虚构名）。
fn extract_name(
    score_line: &str,
    prev_line: Option<&str>,
    next_line: Option<&str>,
    row: u32,
) -> String {
    let tokens: Vec<&str> = score_line.split_whitespace().collect();
    // ---- C1 / C3：首列块（轨道让位）----
    if let Some(first) = tokens.first() {
        let solid = nfkc_solid(first);
        if TRACKS.contains(&solid.as_str()) {
            if let Some(second) = tokens.get(1) {
                let s = nfkc_solid(second);
                if is_name_candidate(&s) {
                    return s;
                }
            }
        } else if is_name_candidate(&solid) {
            let echo = prev_line
                .map(|p| nfkc_solid(p).contains(&solid))
                .unwrap_or(false);
            if !echo {
                return solid;
            }
        }
    }
    // ---- C2：分值前列块（tokens = […, pre, N.NN, 主观|客观]）的冒号后段 ----
    if tokens.len() >= 3 {
        let obj = tokens[tokens.len() - 1];
        let num = tokens[tokens.len() - 2];
        let is_obj = obj == "主观" || obj == "客观";
        let is_num = num.parse::<f64>().is_ok() && num.chars().next().is_some_and(|c| c.is_ascii_digit());
        if is_obj && is_num {
            let pre = tokens[tokens.len() - 3];
            if let Some(colon) = pre.rfind(|c: char| c == '：' || c == ':') {
                let s = nfkc_solid(pre[colon..].trim_start_matches(['：', ':']));
                if is_name_candidate(&s) {
                    return s;
                }
            }
        }
    }
    // ---- C4：邻行拼合（名字列裂成两片——上下行都是短行）----
    if let (Some(p), Some(n)) = (prev_line, next_line) {
        let ps = p.trim();
        let ns = n.trim();
        let pc = ps.chars().count();
        let nc = ns.chars().count();
        if (4..=20).contains(&pc) && (4..=20).contains(&nc) && pc + nc <= 40 {
            let joined = format!("{ps}{ns}");
            let solid = nfkc_solid(&joined);
            let jc = solid.chars().count();
            let cjk = solid.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
            let bad_head = solid
                .chars()
                .next()
                .map(|c| matches!(c, '，' | ',' | '。' | '；' | '：' | ':' | '、' | '（' | '）'))
                .unwrap_or(true);
            if cjk && !bad_head && (2..=40).contains(&jc) {
                return solid;
            }
        }
    }
    format!("评审项{row}")
}

/// 分值行的列块剥离（名字/轨道/分值/主客观是**列块不是行文**——混入会污染
/// 细则文本：松岭实测「④承诺故[服务承诺]障恢复时间：…派出⼯程师[8.00 主观]
/// 到现场…」）。只对分值行做 token 级剥离，其他行不动（名字在其他行的
/// 合法出现——如「供应商需提供…服务方案」——不受影响）。
fn strip_score_line(line: &str, name_solid: &str, track: &str) -> String {
    line.split_whitespace()
        .filter(|tok| {
            let s = nfkc_solid(tok);
            !(s == name_solid
                || s == track
                || s == "主观"
                || s == "客观"
                || (s.parse::<f64>().is_ok() && s.chars().next().is_some_and(|c| c.is_ascii_digit())))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 综合评分表装配（表名行 → 下一章头/EOF）。
///
/// 返回 `(rows, 消费块位集含表名行, 外层扫描游标)`。
fn assemble_scoring_rows(
    lines: &[(usize, &Block)],
    title_at: usize,
) -> (Vec<ScoringRow>, Vec<usize>, usize) {
    // ---- 区域末：下一章头 / 终结样板 / EOF ----
    let re_chapter = regex::Regex::new(r"^第\s*[0-9一二三四五六七八九十百]+\s*章").unwrap();
    let re_terminator = regex::Regex::new(concat!(
        "结论[:：]|评委.{0,6}签名|确认签名|监督人员|技术组评委|商务组评委|",
        "记名投票|记录人"
    ))
    .unwrap();
    let mut region_end = lines.len();
    for (off, (_, b)) in lines.iter().enumerate().skip(title_at + 1) {
        let t = b.text.trim();
        if re_chapter.is_match(&nfkc(t)) || re_terminator.is_match(t) || t.starts_with("注：") {
            region_end = off;
            break;
        }
    }

    // ---- 行装配状态 ----
    struct PendingRow {
        idxs: Vec<usize>,
        texts: Vec<String>, // 行原文（未归一——圈号切分需要在归一前）
        score_pos: Option<usize>,
    }
    let mut rows: Vec<ScoringRow> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    let mut pending: Option<PendingRow> = None;
    let mut pending_score: Option<(f64, bool)> = None;
    let mut in_boilerplate = false;

    // flush：带分值的行 → ScoringRow；无分值的（表头残段）只消费不产出。
    fn flush(
        pending: &mut Option<PendingRow>,
        pending_score: &mut Option<(f64, bool)>,
        rows: &mut Vec<ScoringRow>,
        used: &mut Vec<usize>,
        lines: &[(usize, &Block)],
    ) {
        let Some(p) = pending.take() else { return };
        used.extend(p.idxs.iter().copied());
        let Some((score, objective)) = pending_score.take() else {
            return; // 无分值段：表头/轨道残段——消费不产出
        };
        let row_no = rows.len() as u32 + 1;
        // 分值行与邻行（名字提取的输入面——用剥离前的原行）
        let sp = p.score_pos.unwrap_or(0);
        let score_line = p.texts.get(sp).cloned().unwrap_or_default();
        let prev_line = sp.checked_sub(1).and_then(|i| p.texts.get(i)).cloned();
        let next_line = p.texts.get(sp + 1).cloned();
        let name = extract_name(&score_line, prev_line.as_deref(), next_line.as_deref(), row_no);
        // 轨道：分值行含轨道标签 → 该轨；缺省技术评审（评分表技术主体常态）
        let track = TRACKS
            .iter()
            .find(|t| nfkc_solid(&score_line).contains(*t))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "技术评审".to_string());
        // 行文本：分值行换列块剥离形；纯轨道行剔除；其余原样拼接
        let stripped_score_line = strip_score_line(&score_line, &name, &track);
        let mut body_parts: Vec<String> = Vec::new();
        for (i, t) in p.texts.iter().enumerate() {
            if Some(i) == p.score_pos {
                if !stripped_score_line.is_empty() {
                    body_parts.push(stripped_score_line.clone());
                }
                continue;
            }
            if TRACKS.contains(&nfkc_solid(t).as_str()) {
                continue; // 跨行合并单元格的轨道列块
            }
            body_parts.push(t.clone());
        }
        let joined = body_parts.join("");
        // 表头残段裁剪（首行常粘着「… 客观/主观类」列头）
        let body = match joined.find("客观/主观类") {
            Some(pos) => joined[pos + "客观/主观类".len()..].to_string(),
            None => joined,
        };
        let sub_items = split_sub_items(&body);
        let criteria_body = nfkc(&body).trim().to_string();
        // 锚前缀用规范名（「，」→「及」——与域层 canonical_item_name 同一
        // 规则：章节标题的并列连词形。前缀是重建文本的自描述头，规范形
        // 保证「评分表行 → 章节」的检索面（#252 门 4：REQ 锚含章节标题）
        let canonical = name.replace('，', "及");
        let locator = p
            .idxs
            .first()
            .and_then(|&bi| lines.iter().find(|(i, _)| *i == bi).map(|(_, b)| b.locator.clone()))
            .unwrap_or_default();
        rows.push(ScoringRow {
            row: row_no,
            name: name.clone(),
            score,
            objective,
            track: track.clone(),
            criteria: format!("{canonical}（{score}分 {track}）：{criteria_body}"),
            sub_items,
            block_idxs: p.idxs.clone(),
            locator,
        });
    }

    for off in (title_at + 1)..region_end {
        let (idx, b) = lines[off];
        let t = b.text.trim();
        let n_solid = crate::norm_n1(t);
        if n_solid.is_empty() {
            continue;
        }
        // 页脚：消费但不入行文本（锚面不带页码装饰）
        if is_page_footer(&n_solid) {
            used.push(idx);
            continue;
        }
        let nfkc_line = nfkc(t);
        let tok = score_token(&nfkc_line);
        if tok.is_some() {
            // 新分值行到达而上一行未收尾（异常形态）→ 先冲刷，防两行粘连
            if pending_score.is_some() {
                flush(&mut pending, &mut pending_score, &mut rows, &mut used, lines);
                in_boilerplate = false;
            }
        }
        if pending.is_none() {
            pending = Some(PendingRow { idxs: Vec::new(), texts: Vec::new(), score_pos: None });
        }
        let p = pending.as_mut().unwrap();
        if tok.is_some() {
            p.score_pos = Some(p.texts.len());
            pending_score = tok;
        }
        p.idxs.push(idx);
        p.texts.push(t.to_string());
        // 终结判定：样板段内只认闭括号；「不得分。」句；否则区域末冲刷
        let boilerplate_open = n_solid.contains("以上缺陷是指");
        let terminate = if in_boilerplate {
            n_solid.contains('）') || n_solid.contains(')')
        } else if boilerplate_open {
            false // 样板段开启——同行的「不得分。」不算行终
        } else {
            n_solid.ends_with("不得分。") || n_solid.ends_with("不得分.")
        };
        if boilerplate_open {
            in_boilerplate = true;
        }
        if terminate {
            flush(&mut pending, &mut pending_score, &mut rows, &mut used, lines);
            in_boilerplate = false;
        }
    }
    flush(&mut pending, &mut pending_score, &mut rows, &mut used, lines);
    let mut consumed = vec![lines[title_at].0];
    consumed.extend(used);
    (rows, consumed, region_end)
}

/// PDF 线性化综合评分表抽取（顶层入口——extract_main_pdf 消费）。
///
/// 返回 (行集, 消费块位集)——消费位并入 review_table 的接管集（主循环剔除，
/// 防逐行分支双抽）。表名行计入消费（与 review_table 的「表名留给旧链开
/// 语境」不同：评分表名行在旧链无语境价值，留着会被评分细则行误抽）。
pub fn extract_scoring_table_pdf(f: &ParsedFile) -> (Vec<ScoringRow>, Vec<usize>) {
    let lines: Vec<(usize, &Block)> = f
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine))
        .collect();
    let mut rows: Vec<ScoringRow> = Vec::new();
    let mut consumed: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let (_, b) = lines[i];
        if is_scoring_table_title(b.text.trim()) {
            let (new_rows, used, next) = assemble_scoring_rows(&lines, i);
            if new_rows.is_empty() {
                i += 1; // 表名后无数据行——推进一行（引用行/装饰行形态）
                continue;
            }
            rows.extend(new_rows);
            consumed.extend(used);
            i = next.max(i + 1);
            continue;
        }
        i += 1;
    }
    (rows, consumed)
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;

    fn file_with(lines: &[&str]) -> ParsedFile {
        ParsedFile {
            name: "A-招标文件.pdf".into(),
            sha256: "x".into(),
            pages: 1,
            tables: 1,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks: lines
                .iter()
                .enumerate()
                .map(|(i, t)| Block {
                    idx: i,
                    kind: BlockKind::PdfLine,
                    locator: format!("第25页 行{}", i + 1),
                    text: t.to_string(),
                    cells: vec![],
                })
                .collect(),
            parse_summary: String::new(),
            components: vec![],
        }
    }

    /// #251 实测形态（松岭 TF-0002 第 25-28 页，行切分与康熙部首按 pdftotext
    /// 原样）：8 行（6 技术 + 业绩 + 价格）——名字/分值/细则三面全链验证。
    #[test]
    fn songling_shape_eight_rows() {
        let f = file_with(&[
            "综合评分表【综合评分法适⽤】",
            "采购包1：",
            "评审内容 评审标准",
            "技术部分85.00分",
            "分值构成 商务部分5.00分",
            "报价得分10.00分",
            "评审因素分",
            "评审内容 具体标准和要求 分值 客观/主观",
            "类",
            "1.根据本项⽬的具体情况，供应商需提供详",
            "细、完善、合理可⾏的服务⽅案 , 包含：①",
            "整体服务⽅案（对本项⽬建设、运⾏维护服",
            "务⽅案）；②服务 ⼈员管理规范；③服务阶",
            "段投⼊本项⽬的⼈⼒安排计划；④服务进度",
            "保障 ⽅案；⑤服务时间节点安排；以上内容",
            "每具备⼈员安排；⼀项得5分，满分 25分，",
            "服务⽅案 每少⼀项扣5分，每有⼀项表述存在缺陷的 25.00 主观",
            "扣1分，扣完为⽌。（以上缺陷是指：与本",
            "项⽬采购标的⽆关、内容凭空编造、内容前",
            "后不⼀致、前后逻辑错误、涉及的国家及⾏",
            "业规范及标准引⽤错误或已废⽌、有违法违",
            "规内容陈述、时间地点区域错误、不符合本",
            "项⽬采购需求、只有简单描述⽆实质性内容）",
            "）",
            "-第25⻚-",
            "1.根据本项⽬的具体情况供应商提供项⽬重",
            "点、难点分析，并提供相应的应对措施、解",
            "决⽅案及合理化建议，包含：①项⽬重点、",
            "难点分析；②项⽬实施过程重点、难点应对",
            "措施、解决⽅案及合理化建议。以上内容每",
            "具备⼀项得5分，满分10分，每少⼀项扣5分",
            "项⽬重点、难点分析，应对",
            "，每有⼀项表述存在缺陷的扣1分，扣完为 10.00 主观",
            "措施及相关的合理化建议",
            "⽌。（以上缺陷是指：与本项⽬采购标的⽆",
            "关、内容凭空编造、内容前后不⼀致、前后",
            "逻辑错误、涉及的国家及⾏业规范及标准引",
            "⽤错误或已废⽌、有违法违规内容陈述、时",
            "间地点区域错误、不符合本项⽬采购需求、",
            "只有简单描述⽆实质性内容）",
            "技术评审",
            "-第26⻚-",
            "供应商应结合本项⽬实际情况及特点，提供",
            "针对本项⽬合理可⾏的保密措施⽅案，包括",
            "但不限于：①保密措施；②保密承诺。以上",
            "内容每具备⼀项得5分，满分10分，每少⼀",
            "项扣5分，每有⼀项表述存在缺陷的扣1分，",
            "保密⼯作 扣完为⽌。（以上缺陷是指：与本项⽬采购 10.00 主观",
            "标的⽆关、内容凭空编造、内容前后不⼀致",
            "、前后逻辑错误、涉及的国家及⾏业规范及",
            "标准引⽤错误或已废⽌、有违法违规内容陈",
            "述、时间地点区域错误、不符合本项⽬采购",
            "需求、只有简单描述⽆实质性内容）",
            "①在项⽬实施所在地设⽴相应的技术⽀持及",
            "售后服务机构（提供有效的办公场所证明材",
            "料）； ②对备品配件、设备发⽣故障后的补",
            "救措施； 以上两项内容，每具备⼀项得2分",
            "，满分4分。不提供不得分，每项存在⼀项",
            "表述缺陷的扣1分，扣完为⽌。 ③承诺维修",
            "服务响应时间：响应时间30分钟内得2分，1",
            "⼩时内得1分，超过1⼩时不得分； ④承诺故",
            "服务承诺 障恢复时间：接到故障报修后，派出⼯程师 8.00 主观",
            "到现场并在2⼩时内解决问题故障，得2分，",
            "超过2⼩时不得分。（以上缺陷是指：与本",
            "项⽬采购标的⽆关、内容凭空编造、内容前",
            "后不⼀致、前后逻辑错误、涉及的国家及⾏",
            "业规范及标准引⽤错误或已废⽌、有违法违",
            "规内容陈述、时间地点区域错误、不符合本",
            "项⽬采购需求、只有简单描述⽆实质性内容",
            "）",
            "服务质量保证包含：①服务质量保证体系及",
            "保证措施；②有完整的服务质 量⾃检措施，",
            "整改保障措施；③ ⽇常维护⽅案；④定期巡",
            "检⽅案；以上内 容每具备⼀项得5分，满分2",
            "0分，每少⼀项扣5分，每有⼀项表述存在缺",
            "陷 的扣1分，扣完为⽌。（以上缺陷是指：质量保证 20.00 主观",
            "与本项⽬采购标的⽆关、内容凭空编造、内",
            "容前后不⼀致、前后逻辑错误、涉及的国家",
            "及⾏业规范及标准引⽤错误或已废⽌、有违",
            "法违规内容陈述、时间地点区域错误、不符",
            "合本项⽬采购需求、只有简单描述⽆实质性",
            "内容）",
            "-第27⻚-",
            "根据本项⽬的整体实施要求，供应商须提供",
            "详细、完整、合理、针对本项⽬的服务应急",
            "预案，包含：①⼈员安排；②应急处理预案",
            "；③固定联系⼈ 、联系⽅式。以上内容每具",
            "备⼀项得4分，满分12分，每少⼀项扣4分，",
            "每有⼀项表述存在缺陷的扣1分，扣完为⽌",
            "应急⽅案 12.00 主观",
            "。（以上缺陷是指：与本项⽬采购标的⽆关",
            "、内容凭空编造、内容前后不⼀致、前后逻",
            "辑错误、涉及的国家及⾏业规范及标准引",
            "⽤错误或已废⽌、有违法违规内容陈述、时",
            "间地点区域错误、不符合本项⽬采购需求、",
            "只有简单描述⽆实质性内容）",
            "供应商提供 2022年⾄今同类项⽬的业绩，",
            "投标⽂件中需附有业绩证明材料，业绩以合",
            "商务评审 业绩 5.00 客观",
            "同原件清晰扫描件盖公章为依据，每提供⼀",
            "个得2.5分，满分5分，未提供不得分。",
            "F1指价格项评审因素得分＝（评标基准价/投",
            "标报价）×100×价格项评审因素所占的权重",
            "（注：满⾜招标⽂件要求且投标价格最低的",
            "价格评审 价格评审 投标报价为评标基准价。）最低报价不是中 10.00 客观",
            "标的唯⼀依据。因落实政府采购政策进⾏价",
            "格调整的，以调整后的价格计算评标基准价",
            "和投标报价。",
            "第六章 拟签订的合同⽂本",
        ]);
        let (rows, used) = extract_scoring_table_pdf(&f);
        assert_eq!(rows.len(), 8, "8 行（6 技术 + 业绩 + 价格）——实测 {rows:#?}");
        // ---- 名字面（C1/C3/C2/C4 优先级链）----
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "服务方案",
                "项目重点、难点分析，应对措施及相关的合理化建议",
                "保密工作",
                "服务承诺",
                "质量保证",
                "应急方案",
                "业绩",
                "价格评审",
            ]
        );
        // ---- 分值 / 主客观 / 轨道 ----
        let scores: Vec<f64> = rows.iter().map(|r| r.score).collect();
        assert_eq!(scores, vec![25.0, 10.0, 10.0, 8.0, 20.0, 12.0, 5.0, 10.0]);
        let tracks: Vec<&str> = rows.iter().map(|r| r.track.as_str()).collect();
        assert_eq!(
            tracks,
            vec![
                "技术评审", "技术评审", "技术评审", "技术评审", "技术评审", "技术评审",
                "商务评审", "价格评审"
            ]
        );
        assert!(rows.iter().take(6).all(|r| !r.objective), "技术行主观");
        assert!(rows[6].objective && rows[7].objective, "业绩/价格客观");
        // ---- 细则面（技术行 20 条；业绩/价格 0）----
        let sub_counts: Vec<usize> = rows.iter().map(|r| r.sub_items.len()).collect();
        assert_eq!(
            sub_counts,
            vec![5, 2, 2, 4, 4, 3, 0, 0],
            "细则计数（25÷5 / 10÷2 / 10÷2 / 8÷4 / 20÷4 / 12÷3）"
        );
        assert_eq!(rows[0].sub_items[0], "整体服务方案（对本项目建设、运行维护服务方案）");
        assert_eq!(rows[0].sub_items[3], "服务进度保障方案", "跨行空格已去（服务进度保障 ⽅案）");
        assert_eq!(rows[0].sub_items[4], "服务时间节点安排");
        assert_eq!(
            rows[3].sub_items[0],
            "在项目实施所在地设立相应的技术支持及售后服务机构（提供有效的办公场所证明材料）"
        );
        assert_eq!(
            rows[3].sub_items[2],
            "承诺维修服务响应时间：响应时间30分钟内得2分，1小时内得1分，超过1小时不得分"
        );
        // 列块剥离：分值行的名字/分值/主客观不再污染细则（实测形态：
        // 「④承诺故[服务承诺]障恢复时间：…⼯程师[8.00 主观]到现场…」）
        assert_eq!(
            rows[3].sub_items[3],
            "承诺故障恢复时间：接到故障报修后，派出工程师到现场并在2小时内解决问题故障，得2分，超过2小时不得分"
        );
        // 页脚与轨道列块不入行文本
        for r in &rows {
            assert!(!r.criteria.contains("-第"), "页脚不入锚：{}", r.criteria);
            assert!(!r.criteria.contains("技术评审①"), "纯轨道行剔除：{}", r.criteria);
        }
        assert_eq!(rows[4].sub_items[1], "有完整的服务质量自检措施，整改保障措施");
        assert_eq!(rows[5].sub_items[2], "固定联系人、联系方式");
        // ---- 锚面前缀（门 4 检索面：REQ 锚 excerpt 含规范名）----
        assert!(rows[0].criteria.starts_with("服务方案（25分 技术评审）："));
        assert!(rows[1].criteria.contains("项目重点、难点分析及应对措施及相关的合理化建议（10分 技术评审）："), "锚前缀用规范名（逗号→及）");
        assert!(rows[6].criteria.starts_with("业绩（5分 商务评审）："));
        // ---- 消费面：表名行 + 全区域（含页脚）+ 不含第六章行 ----
        assert!(used.contains(&0), "表名行计入消费");
        let last_block = f.blocks.len() - 1;
        assert!(!used.contains(&last_block), "第六章行不消费（主循环章节语境）");
        assert!(used.contains(&(last_block - 1)), "价格行末行消费");
    }

    /// 名字提取的回声防御：首列块是上一行文段的跨行断字（「陷」形）→ 不取。
    #[test]
    fn name_extraction_units() {
        // C1：名字列与分值同行
        assert_eq!(extract_name("服务⽅案 每少⼀项扣5分，每有⼀项表述存在缺陷的 25.00 主观", Some("满分为25分，"), None, 1), "服务方案");
        // C3：轨道标签让位
        assert_eq!(extract_name("商务评审 业绩 5.00 客观", None, None, 7), "业绩");
        assert_eq!(extract_name("价格评审 价格评审 投标报价为评标基准价。）最低报价不是中 10.00 客观", None, None, 8), "价格评审");
        // C2：冒号后段（名字粘在谓语碎片尾）
        assert_eq!(extract_name("陷 的扣1分，扣完为⽌。（以上缺陷是指：质量保证 20.00 主观", Some("0分，每少⼀项扣5分，每有⼀项表述存在缺"), None, 5), "质量保证");
        // C4：邻行拼合（名字列裂成两片）
        assert_eq!(
            extract_name(
                "，每有⼀项表述存在缺陷的扣1分，扣完为 10.00 主观",
                Some("项⽬重点、难点分析，应对"),
                Some("措施及相关的合理化建议"),
                2
            ),
            "项目重点、难点分析，应对措施及相关的合理化建议"
        );
        // 兜底：无任何可用列块
        assert_eq!(extract_name("10.00 主观", None, None, 9), "评审项9");
    }

    /// 无评分表 → 空产出零消费（corpus-A docx 路径不经过本模块的旁证）。
    #[test]
    fn no_scoring_table_no_output() {
        let f = file_with(&["第一章 招标公告", "本项目为光纤租赁服务。"]);
        let (rows, used) = extract_scoring_table_pdf(&f);
        assert!(rows.is_empty());
        assert!(used.is_empty());
    }

    /// 表名后紧跟章头（空表形态）→ 不产出、不死循环。
    #[test]
    fn empty_table_advances() {
        let f = file_with(&["综合评分表【综合评分法适⽤】", "第六章 合同"]);
        let (rows, _used) = extract_scoring_table_pdf(&f);
        assert!(rows.is_empty());
    }
}
