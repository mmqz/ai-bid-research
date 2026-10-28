//! bid-kb —— AdapterA 知识库（M1：确定性切块 + 精确通道检索）。
//!
//! 契约（mcp-contract §5 + 附录 A）：
//! - 切块：结构化确定性切块（chunk_id 内容寻址 = sha256(文件哈希:块序)），
//!   登记 chunk → file → 位置（证据链可回溯）；
//! - 检索：M1 落地 exact 通道（型号/金额/日期/条款/证书号五类），
//!   semantic 通道是接口占位（M2 双通道合并后补）；
//! - 排序确定性（R-15）：(匹配分, 金额单一性, 文件类别权重, 块序) 四级 tie-breaker，
//!   同输入重放结果逐字节一致——「同一查询重复 5 次结果一致」由结构保证。
//!
//! 排序规则说明（防夹具特化，逐条有原则依据）：
//! 1. 匹配分降序——多 token 命中者优先（短语查询的覆盖度）；
//! 2. 金额单一性升序——同分时，只含一个金额的块是权威陈述句（「招标控制价为人民币 X 万元」），
//!    多金额块是摘要/公告式罗列（「预算 350 万；最高限价 480 万」）；
//! 3. 文件类别权重升序——主招标文件 > 补遗 > 附件 > 证据（主文件是条款权威源；
//!    附件/证据的同值命中是佐证不是主锚）；
//! 4. 块序升序——文档序（前文先出，政采文件前附表约束先于正文展开）。

use bid_parse::{kind_rank, norm_n1, Block, ParsedFile};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub mod shard;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    pub chunk_id: String,
    pub file: String, // 相对 corpus 根的标签（evidence/ 前缀或裸文件名）
    pub file_sha: String,
    pub kind_rank: u8,
    pub block_idx: usize,
    pub locator: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub chunk_id: String,
    pub file: String,
    pub hit_text: String,
    pub score: f64,
    pub matched_tokens: Vec<String>,
    pub amount_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExactQuery {
    pub query_id: String,
    pub query_type: String, // model|cert_no|amount|date|clause
    pub query: String,
}

pub struct Kb {
    pub chunks: Vec<Chunk>,
}

impl Kb {
    pub fn ingest(parsed: &[ParsedFile], label: impl Fn(&ParsedFile) -> String) -> Self {
        let mut chunks = vec![];
        for p in parsed {
            let file_label = label(p);
            for b in &p.blocks {
                if b.text.trim().is_empty() {
                    continue;
                }
                chunks.push(build_chunk(p, &file_label, b));
            }
        }
        Kb { chunks }
    }

    /// 精确通道检索：返回排序后的命中（确定性四级 tie-breaker）。
    /// 评分/排序走下方共享核心（S-3 提取——与 shard::ShardedKb 单一真源）。
    pub fn search_exact(&self, q: &ExactQuery) -> Vec<SearchHit> {
        let mut hits: Vec<SearchHit> = vec![];
        for c in &self.chunks {
            if let Some(h) = hit_of_chunk(c, q) {
                hits.push(h);
            }
        }
        sort_hits(&mut hits);
        hits
    }
}

// ---------------------------------------------------------------------------
// 切块/评分/排序共享核心（S-3 提取，crate 私有）
// ---------------------------------------------------------------------------
// 提取纪律：函数搬移不改语义——Kb::search_exact 的既有行为逐字节不变
// （tests/kb_probe.rs 25 条真实语料查询是等价性的回归闸门）。
// Kb（单库）与 shard::ShardedKb（分片库）共用同一份切块/评分/排序代码，
// 禁止两处各持一份后各自漂移。

/// 单块 → Chunk 构造：chunk_id 内容寻址 = sha256(文件哈希:块序) 前 16 hex。
/// Kb::ingest 与 ShardedKb 的写入面共用——chunk_id 口径只此一处。
pub(crate) fn build_chunk(parsed: &ParsedFile, label: &str, b: &Block) -> Chunk {
    let mut h = Sha256::new();
    h.update(parsed.sha256.as_bytes());
    h.update(b":");
    h.update(b.idx.to_string().as_bytes());
    let chunk_id = hex::encode(h.finalize())[..16].to_string();
    Chunk {
        chunk_id,
        file: label.to_string(),
        file_sha: parsed.sha256.clone(),
        kind_rank: kind_rank(label),
        block_idx: b.idx,
        locator: b.locator.clone(),
        text: b.text.clone(),
    }
}

/// 金额面正则（"350 万" / "3,500万"）：OnceLock 单次编译。
/// 模式是字面常量，编译不可失败；expect 仅类型收敛，无运行期失败面。
fn re_amount() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(\d[\d,]*(?:\.\d+)?)\s*万").expect("常量金额正则必然编译成功")
    })
}

/// 纯数字正则（≥4 位，≥1 万口径换算）：OnceLock 单次编译（同上不可失败）。
fn re_plain() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(\d[\d,]{3,})").expect("常量纯数字正则必然编译成功"))
}

/// 单 chunk 文本评分核心（五类查询 model/cert_no/date/amount/clause）。
/// 返回 (匹配分, 命中 token, 金额数)；None = 未命中。两个检索面共用。
pub(crate) fn score_chunk(text: &str, q: &ExactQuery) -> Option<(f64, Vec<String>, usize)> {
    let hay = norm_n1(text);
    match q.query_type.as_str() {
        "model" | "cert_no" | "date" => {
            let needle = norm_n1(&q.query);
            if needle.is_empty() || !hay.contains(&needle) {
                return None;
            }
            Some((1.0, vec![q.query.clone()], 0))
        }
        "amount" => {
            let (wan, tokens) = parse_amount_query(&q.query);
            let mut amounts: Vec<f64> = re_amount()
                .captures_iter(text)
                .map(|m| m[1].replace(',', "").parse::<f64>().unwrap_or(0.0))
                .collect();
            for m in re_plain().captures_iter(text) {
                if let Ok(v) = m[1].replace(',', "").parse::<f64>() {
                    if v >= 10000.0 {
                        amounts.push(v / 10000.0);
                    }
                }
            }
            if !amounts.contains(&wan) {
                return None;
            }
            let matched: Vec<String> = tokens
                .iter()
                .filter(|t| hay.contains(&norm_n1(t)))
                .cloned()
                .collect();
            Some((1.0 + matched.len() as f64, matched, amounts.len()))
        }
        "clause" => {
            let tokens: Vec<String> =
                q.query.split_whitespace().map(|s| s.to_string()).collect();
            let matched: Vec<String> = tokens
                .iter()
                .filter(|t| hay.contains(&norm_n1(t)))
                .cloned()
                .collect();
            if matched.is_empty() {
                return None;
            }
            Some((matched.len() as f64, matched, 0))
        }
        _ => None,
    }
}

/// 单 chunk → 命中构造（评分 + 打包）：两检索面共用的打包口径。
pub(crate) fn hit_of_chunk(c: &Chunk, q: &ExactQuery) -> Option<SearchHit> {
    let (score, matched_tokens, amount_count) = score_chunk(&c.text, q)?;
    Some(SearchHit {
        chunk_id: c.chunk_id.clone(),
        file: c.file.clone(),
        hit_text: c.text.clone(),
        score,
        matched_tokens,
        amount_count,
    })
}

/// 确定性四级排序（R-15：可重放）——两检索面唯一的排序真源：
/// 匹配分降序 → 金额单一性升序 → 文件类别权重升序 → chunk_id 升序。
pub(crate) fn sort_hits(hits: &mut [SearchHit]) {
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.amount_count.cmp(&b.amount_count))
            .then(kind_rank(&a.file).cmp(&kind_rank(&b.file)))
            .then(a.chunk_id.cmp(&b.chunk_id))
    });
}

/// 数值正则（金额查询解析用）：OnceLock 单次编译（常量模式不可失败，同上）。
fn re_num() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\d[\d,]*(?:\.\d+)?").expect("常量数值正则必然编译成功"))
}

/// 金额查询解析：首个数值 → 万口径（≥1 万原值；≥10000 除以 1e4），其余词作佐证 token。
fn parse_amount_query(q: &str) -> (f64, Vec<String>) {
    let re_num = re_num();
    let mut wan = 0.0;
    let mut tokens = vec![];
    for part in q.split_whitespace() {
        if let Some(m) = re_num.find(part) {
            if wan == 0.0 && m.as_str() == part.trim_end_matches("万元") {
                let v: f64 = part.replace(',', "").trim_end_matches("万元").parse().unwrap_or(0.0);
                wan = if v >= 10000.0 { v / 10000.0 } else { v };
                continue;
            }
        }
        if !part.trim().is_empty() && part != "万元" {
            tokens.push(part.trim_matches(|c: char| !c.is_alphanumeric()).to_string());
        }
    }
    if wan == 0.0 {
        if let Some(m) = re_num.find(q) {
            let v: f64 = m.as_str().replace(',', "").parse().unwrap_or(0.0);
            wan = if v >= 10000.0 { v / 10000.0 } else { v };
        }
    }
    (wan, tokens)
}
