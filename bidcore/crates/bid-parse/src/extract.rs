//! 确定性条款抽取（extract_requirements 的「确定性先行」层——mcp-contract §3）。
//!
//! M1 纪律：零模型调用。类别判定 = 章节上下文映射（章/节标题 → 类别），
//! 参数抽取 = 正则槽位（型号/金额/日期），条款号 = 编号头解析 + 表格行位。
//! 模型补全面（语义归类、陈述规范化）属 M2+——本层输出即 REQ 的稳定基座。
//!
//! 类别映射（章节上下文——政采范本结构惯例，规则钉死可审计）：
//!   评标办法→评分项；技术需求→技术参数；商务要求→商务要求；
//!   须知+资格→资格条件；须知+否决→否决项；须知+编制→格式要求；其余→其他。

use crate::{Block, BlockKind, ParsedFile};
use bid_domain::{Anchor, Requirement};
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

/// 型号谱系正则：字母开头（≥1 位）+ 连字符分段（防误配纯数字编号如「附件 7-3」）。
/// 覆盖实测谱系：GW-8000-T4 / S5735-L48T4X-A1 / USG6650E-AC550 / IPC-4T-IR-D2 / NVR-64H-8P / DCS-4000。
fn model_regex() -> Regex {
    Regex::new(r"[A-Z][A-Z0-9]*(?:-[A-Z0-9]+)+").unwrap()
}

pub fn extract_requirements(files: &[ParsedFile]) -> Vec<Requirement> {
    let mut reqs: Vec<Requirement> = Vec::new();
    let mut seq = 0usize;
    for f in files {
        // 主文件/补遗按块形态分发：DOCX 块（Para/TableRow）→ docx 版；PDF 块
        //（PdfLine/OcrLine）→ pdf 版（真实招标文件主文件为 PDF 的常态面）
        let is_pdf_blocks = f
            .blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine));
        match () {
            _ if f.name.starts_with("A-") && is_pdf_blocks =>
                extract_main_pdf(f, &mut reqs, &mut seq),
            _ if f.name.starts_with("C-") && is_pdf_blocks =>
                extract_main_pdf(f, &mut reqs, &mut seq),
            _ if f.name.starts_with("A-") => extract_main_docx(f, &mut reqs, &mut seq),
            _ if f.name.starts_with("C-") => extract_main_docx(f, &mut reqs, &mut seq), // 补遗同构（增量面属 V2）
            _ if f.name.starts_with("B1") => extract_b1_lines(f, &mut reqs, &mut seq),
            _ if f.name.starts_with("B2") => extract_b2_ocr(f, &mut reqs, &mut seq),
            _ => {} // B3 图纸目录/证据件：无条款抽取面（证据入库走 chunk 索引）
        }
    }
    reqs
}

/// PDF 主文件/补遗：行级解析——章节标题行（第X章/中文序号）更新上下文，
/// 编号条款行 → REQ（分类同 docx 版）。
/// P2 真实文件驱动扩展（风险不对称：漏否决/资格条款 = 废标风险 > 误报可过滤——
/// M1 层宁多勿漏，精化归 M2 语义层）：
/// - 强否决短语行（将被否决/视为无效/取消中标资格/没收…）→ 否决项（无编号也抽）
/// - 资格面行（资质等级/资格后审/资质证书/营业执照…）→ 资格条件
/// - 评分细则行（评分办法章内 "N、报价：65 分"）→ 评分项
/// - 续行窗 1 行：跨行条款还原（同 extract_b2_ocr 纪律）
fn extract_main_pdf(f: &ParsedFile, reqs: &mut Vec<Requirement>, seq: &mut usize) {
    let re_clause = Regex::new(r"^(\d+(?:\.\d+)+)\s*").unwrap();
    // 真实文件驱动修正（P2）：真实水利招标文件用阿拉伯数字章节（"第2章 投标人
    // 须知"）——与合成夹具/政采范本的中文数字（"第三章"）并存，双通道容忍空格
    let re_chapter = Regex::new(r"^第\s*([0-9一二三四五六七八九十百]+)\s*章").unwrap();
    let re_subsection = Regex::new(r"^[一二三四五六七八九十]+、").unwrap();
    let re_model = model_regex();
    let re_amount = Regex::new(r"(\d+(?:\.\d+)?)\s*万\s*元").unwrap();
    let re_date = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
    // 强否决短语（判定动词面——正文提及"否决"的弱引用不算）
    let re_veto = Regex::new(
        r"将被否决|否决投标处理|否决所有投标|视为无效|无效投标|取消其中标|没收其投标保证金|废标",
    )
    .unwrap();
    // 资格面（要求/证书/审查语境——弱提及不算）
    let re_qual = Regex::new(
        r"资质等级|资格审查|资格后审|资格预审|资质证书|营业执照|安全生产许可证|类似工程",
    )
    .unwrap();
    // 评分细则行：数字开头 + 短语 + "N 分"（评分办法章内）
    let re_score_line = Regex::new(r"^\d+[、.．]\s*\S{2,30}[：:，,]?\s*\S{0,16}\d+(?:\.\d+)?\s*分").unwrap();
    // 评分算法行：评标基准价/下浮率（强算法词免章门——基准价算法常在须知章）
    let re_score_algo = Regex::new(r"评标基准价|下浮率\s*K|基准价.{0,12}确定").unwrap();
    let mut chapter = String::new();
    let mut subsection = String::new();

    let blocks = &f.blocks;
    for (i, b) in blocks.iter().enumerate() {
        if !matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine) {
            continue;
        }
        let t = b.text.trim();
        // 目录行防御：带页码引导点（……/....）的目录条目不得建立章节上下文
        if t.matches('.').count() >= 6 || t.contains("……") {
            continue;
        }
        if re_chapter.is_match(t) {
            chapter = t.to_string();
            subsection.clear();
            continue;
        }
        if re_subsection.is_match(t) && t.chars().count() <= 40 {
            subsection = t.to_string();
            continue;
        }
        let mut anchor_text = t.to_string();
        // 续行窗：行短且下一行同为 PDF/OCR 行 → 拼接（跨行条款还原）
        if let Some(next) = blocks.get(i + 1) {
            if matches!(next.kind, BlockKind::PdfLine | BlockKind::OcrLine)
                && t.chars().count() < 30
                && next.text.trim().chars().count() >= 2
                && !re_chapter.is_match(next.text.trim())
                && next.text.trim().matches('.').count() < 6
            {
                anchor_text.push_str(next.text.trim());
            }
        }
        if let Some(m) = re_clause.captures(t) {
            let mut params = HashMap::new();
            fill_params(t, &re_model, &re_amount, &re_date, &re_date, &mut params);
            // 编号行归类优先级（R3-S3-5）：强否决短语 > 资格面 > 章节上下文——
            // 编号否决条款（如"3.4.2 …将被否决"）不得落"其他"（漏否决=废标风险）
            let cat = if re_veto.is_match(&anchor_text) {
                "否决项"
            } else if re_qual.is_match(&anchor_text) {
                "资格条件"
            } else {
                category_for(&chapter, &subsection)
            };
            push_req(reqs, seq, cat, Some(m[1].to_string()),
                     vec![anchor(f, b)], params, &f.name);
        } else if re_veto.is_match(&anchor_text) {
            // 强否决短语行（无编号面——前附表/正文流条款）
            push_req(reqs, seq, "否决项", None, vec![anchor(f, b)], HashMap::new(), &f.name);
        } else if re_qual.is_match(&anchor_text) && !anchor_text.starts_with("见") {
            // 资格面行："见投标人须知前附表"类引用行排除
            push_req(reqs, seq, "资格条件", None, vec![anchor(f, b)], HashMap::new(), &f.name);
        } else if (chapter.contains("评标办法") && re_score_line.is_match(t))
            || re_score_algo.is_match(&anchor_text)
        {
            // 评分细则行（中文序号/数字序号分值行）+ 评分算法行（评标基准价/下浮率——
            // P2：基准价算法与下浮率 K 常在须知章 10.x 而非评标办法章，强算法词免章门）
            push_req(reqs, seq, "评分项", None, vec![anchor(f, b)], HashMap::new(), &f.name);
        }
    }
}

fn category_for(chapter: &str, subsection: &str) -> &'static str {
    if chapter.contains("评标办法") {
        "评分项"
    } else if chapter.contains("技术需求") || chapter.contains("技术规范") {
        "技术参数"
    } else if chapter.contains("商务要求") {
        "商务要求"
    } else if chapter.contains("须知") && subsection.contains("资格") {
        "资格条件"
    } else if chapter.contains("须知") && subsection.contains("否决") {
        "否决项"
    } else if chapter.contains("须知") && subsection.contains("编制") {
        "格式要求"
    } else {
        "其他"
    }
}

/// 主文件（A）与补遗（C）：编号条款段落 + 前附表否决行 + 技术需求参数表。
/// DOCX 主文件（合成夹具 corpus-A 形态）；PDF 主文件（真实招标文件形态——P2
/// 评测驱动新增，语义与 DOCX 版一致：章节上下文 → 编号条款抽取）。
fn extract_main_docx(f: &ParsedFile, reqs: &mut Vec<Requirement>, seq: &mut usize) {
    let mut last_numbered_head: Option<String> = None; // 表格行收养最近的编号段（5.x 参数表上下文）
    let re_clause = Regex::new(r"^(\d+(?:\.\d+)+)\s*").unwrap();
    let re_model = model_regex();
    let re_amount = Regex::new(r"(\d+(?:\.\d+)?)\s*万\s*元").unwrap();
    let re_date = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
    let re_date_cn = Regex::new(r"[〇一二三四五六七八九零]{4}年[〇一二三四五六七八九零]{1,2}月[〇一二三四五六七八九零]{1,3}日").unwrap();

    for b in &f.blocks {
        match b.kind {
            BlockKind::Para => {
                if let Some(m) = re_clause.captures(&b.text) {
                    last_numbered_head = Some(m[1].to_string());
                    let (chapter, subsection) = split_locator(&b.locator);
                    let mut params = HashMap::new();
                    fill_params(&b.text, &re_model, &re_amount, &re_date, &re_date_cn, &mut params);
                    push_req(reqs, seq, category_for(chapter, subsection), Some(m[1].to_string()),
                             vec![anchor(f, b)], params, &f.name);
                }
            }
            BlockKind::TableRow => {
                let joined = b.cells.join("");
                if b.locator.contains("前附表") && (joined.contains("否决") || b.text.contains("否决")) {
                    // 前附表否决行：条款号 = 前附表-{序号}（序号列为准；缺序号回退 XML 行号）
                    let item = b.cells.iter()
                        .find(|c| c.chars().all(|x| x.is_ascii_digit()) && !c.is_empty())
                        .and_then(|c| c.parse::<usize>().ok())
                        .unwrap_or_else(|| row_no_of(&b.locator));
                    push_req(reqs, seq, "否决项", Some(format!("前附表-{item}")),
                             vec![anchor(f, b)], HashMap::new(), &f.name);
                } else if (b.locator.contains("技术需求") || b.locator.contains("第五章"))
                    && re_model.is_match(&b.text)
                {
                    // 参数表行：收养最近的 5.x 编号头作条款号
                    let mut params = HashMap::new();
                    fill_params(&b.text, &re_model, &re_amount, &re_date, &re_date_cn, &mut params);
                    push_req(reqs, seq, "技术参数", last_numbered_head.clone(),
                             vec![anchor(f, b)], params, &f.name);
                }
            }
            _ => {}
        }
    }
}

/// B1 设备清单：含型号或中文大写日期的行 → 技术参数 REQ（到货/规格要求行）。
fn extract_b1_lines(f: &ParsedFile, reqs: &mut Vec<Requirement>, seq: &mut usize) {
    let re_model = model_regex();
    let re_date_cn = Regex::new(r"[〇一二三四五六七八九零]{4}年[〇一二三四五六七八九零]{1,2}月[〇一二三四五六七八九零]{1,3}日").unwrap();
    let re_amount = Regex::new(r"(\d+(?:\.\d+)?)\s*万\s*元").unwrap();
    let re_date = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
    for b in &f.blocks {
        if matches!(b.kind, BlockKind::PdfLine)
            && (re_model.is_match(&b.text) || re_date_cn.is_match(&b.text))
        {
            let mut params = HashMap::new();
            fill_params(&b.text, &re_model, &re_amount, &re_date, &re_date_cn, &mut params);
            push_req(reqs, seq, "技术参数", None, vec![anchor(f, b)], params, &f.name);
        }
    }
}

/// B2 扫描件：否决关键词 OCR 行 + 1 行续接窗 → 否决项 REQ（扫描件内条款不丢的抽取面）。
fn extract_b2_ocr(f: &ParsedFile, reqs: &mut Vec<Requirement>, seq: &mut usize) {
    let keywords = ["否决", "签署", "盖章"];
    let blocks = &f.blocks;
    for (i, b) in blocks.iter().enumerate() {
        if !matches!(b.kind, BlockKind::OcrLine) {
            continue;
        }
        if !keywords.iter().any(|k| b.text.contains(k)) {
            continue;
        }
        // 种子行 + 下一行拼接（断行条款还原）；excerpt 同时携带两行原文
        let mut excerpt = b.text.clone();
        let mut locator = b.locator.clone();
        if let Some(next) = blocks.get(i + 1) {
            if matches!(next.kind, BlockKind::OcrLine) && next.text.chars().count() >= 2 {
                excerpt.push_str(&next.text);
                locator.push_str(" + 行接");
            }
        }
        push_req(reqs, seq, "否决项", None,
                 vec![Anchor { file: f.name.clone(), kind: "ocr".into(), locator, excerpt }],
                 HashMap::new(), &f.name);
    }
}

/// 评分项分值提取（T-3 评分矩阵输入面——issue #11 批次 v3）：
/// 评分细则行「N、报价：65 分」/ 表格行「…30 分」→ `params.score`。
/// 纯确定性正则（零模型）；「满分 100 分」类总分层可能误配为单行分值——
/// 领域层 scoring sanity（总分 >100 告警）兑底，不在本层猜测语义。
fn score_of(text: &str) -> Option<f64> {
    static RE_SCORE: OnceLock<Regex> = OnceLock::new();
    RE_SCORE
        .get_or_init(|| Regex::new(r"(\d+(?:\.\d+)?)\s*分").unwrap())
        .captures(text)
        .and_then(|c| c[1].parse().ok())
}

fn push_req(reqs: &mut Vec<Requirement>, seq: &mut usize, category: &str,
            clause_no: Option<String>, anchors: Vec<Anchor>,
            mut params: HashMap<String, serde_json::Value>, source: &str) {
    // T-3：评分项统一补分值槽位（所有推入路径单一插入点——段落/表格/前附表）
    if category == "评分项" && !params.contains_key("score") {
        let joined: String = anchors.iter().map(|a| a.excerpt.as_str()).collect();
        if let Some(s) = score_of(&joined) {
            params.insert("score".into(), serde_json::json!(s));
        }
    }
    *seq += 1;
    reqs.push(Requirement {
        id: format!("REQ-{seq:04}"),
        category: category.to_string(),
        clause_no,
        anchors,
        params,
        source_file: source.to_string(),
    });
}

fn anchor(f: &ParsedFile, b: &Block) -> Anchor {
    Anchor {
        file: f.name.clone(),
        kind: match b.kind {
            BlockKind::Para => "para",
            BlockKind::TableRow => "table",
            BlockKind::PdfLine => "pdf_line",
            BlockKind::OcrLine => "ocr",
            BlockKind::XlsxRow => "xlsx_row",
        }
        .to_string(),
        locator: b.locator.clone(),
        excerpt: b.text.clone(), // 溯源锚原文（创建时固化）
    }
}

fn fill_params(text: &str, re_model: &Regex, re_amount: &Regex, re_date: &Regex,
               re_date_cn: &Regex, params: &mut HashMap<String, serde_json::Value>) {
    if let Some(m) = re_model.find(text) {
        params.insert("model".into(), serde_json::Value::String(m.as_str().to_string()));
    }
    if let Some(m) = re_amount.captures(text) {
        let v: f64 = m[1].parse().unwrap_or(0.0);
        params.insert("amount_wan".into(), serde_json::json!(v));
    }
    if let Some(m) = re_date.find(text) {
        params.insert("date".into(), serde_json::Value::String(m.as_str().to_string()));
    }
    if let Some(m) = re_date_cn.find(text) {
        params.insert("date_cn".into(), serde_json::Value::String(m.as_str().to_string()));
    }
}

fn split_locator(locator: &str) -> (&str, &str) {
    match locator.split_once(" > ") {
        Some((c, s)) => (c, s),
        None => (locator, ""),
    }
}

fn row_no_of(locator: &str) -> usize {
    locator.rsplit("行").next().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}
