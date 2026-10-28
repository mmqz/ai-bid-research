//! shard.rs —— S-3 索引分片（issue #11 批次 v3 / V3-3）。
//!
//! 为什么分片：百万字标书文档不能每次全量重建索引（issue #11 S-3 验收原文：
//! 「新增 30 页补遗 → 索引更新 < 2 秒（不是全量重建）」）。把索引按文件切成
//! 独立分片：新增补遗只建补遗那一片、文件换版只 diff 换版那一片，其余片零触碰。
//!
//! 双维度分片：
//! - **文件级分片（主键）**：主招标文件 / 附件 / 补遗 / 证据各自一片，
//!   分片键 = file label（kind_rank 的类别语义保留在片内 chunk 上——
//!   排序第三级不受分片影响）；
//! - **章节级子指纹**：文件内按 locator 首个路径段分组为「段」
//!   （「第三章/3.2.1」→ 段键「第三章」；无分隔符 → 整串），段内容指纹不变
//!   则该段 chunk 不重建——把换版时的重建粒度从「整片」压到「段」。
//!
//! 增量判定（两级闸门，由外到内）：
//! 1. 文件 sha256 不变 → 整片 no-op（`ShardUpdate::Unchanged`，零触碰）；
//! 2. sha 变（补遗换版 / 文件修订）→ 只做该片的**段级 diff**：新解析件逐段算
//!    内容指纹（FNV-1a 64 over 逐 chunk 的 (idx, text)），与旧片
//!    `section_fingerprints` 账本对比——指纹相同的段保留原 Chunk（零重建；
//!    chunk_id 内容寻址 = sha256(file_sha:idx)，旧段保留即旧 chunk_id 保留，
//!    证据链稳定），指纹不同 / 新增的段重建该段，消失的段随旧片丢弃；
//!    其他片零触碰。
//!
//! 检索等价（R-15 不变）：跨片合并 = 线性扫各片 chunk 后走与 `Kb::search_exact`
//! 同一份评分 + 四级排序核心（lib.rs 的 crate 私有共享函数——单一真源，两处
//! 检索面禁止各持一份排序代码后各自漂移）。四级排序的末级 tie-breaker 是
//! chunk_id（内容寻址，语料内唯一），合并前的遍历序（片间 BTreeMap 标签序、
//! 片内文档序）不进入输出——同一查询在单库 Kb 与 ShardedKb 上命中逐字节一致。
//!
//! 确定性纪律：分片表与段指纹账本一律 BTreeMap（不用 HashMap——跨运行 / 跨
//! 进程迭代序确定，R-15 可重放的结构前提）；生产路径零 unwrap / 零 panic。

use crate::{build_chunk, hit_of_chunk, sort_hits, Chunk, ExactQuery, SearchHit};
use bid_parse::ParsedFile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 文件级分片（主键 = file label，BTreeMap 确定性迭代）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileShard {
    pub file: String,      // 分片键（文件标签）
    pub file_sha: String,  // 文件指纹（sha256——增量判定的主闸门）
    pub chunk_count: usize,
    /// 段级指纹账本：章节键 → 该段全部 chunk 的内容指纹（fnv64——borrow
    /// mdcore fingerprint 语义，见下方 FNV 注释块）。键 = chunk.locator 的
    /// 首个路径段（"第三章/3.2.1" → "第三章"；无分隔符 → 整串）。
    pub section_fingerprints: BTreeMap<String, u64>,
    /// 本片 chunk 全量（检索面——search 时线性扫，与旧 Kb 同复杂度量级）。
    pub chunks: Vec<Chunk>,
}

/// 分片化知识库（drop-in 检索面：与 Kb::search_exact 等价）。
pub struct ShardedKb {
    /// 分片主键 = file label；BTreeMap——标签序迭代确定（跨运行/跨进程）。
    shards: BTreeMap<String, FileShard>,
    total_chunks: usize,
}

/// 增量更新对账（验收证据面）：每次 upsert 返回「实际做了什么」——
/// 「新增 30 页补遗 < 2 秒」的增量性由 `Rebuilt.chunks_built` 只含新片
/// 块数（而非全库块数）作结构性证明。
#[derive(Debug, Clone, PartialEq)]
pub enum ShardUpdate {
    /// 文件指纹未变：整片零触碰。
    Unchanged { file: String },
    /// 新分片：整片新建。
    Rebuilt {
        file: String,
        chunks_built: usize,
        sections_touched: usize,
    },
    /// 文件换版：段级 diff——只重建指纹变化的段，保留段零重建。
    Partial {
        file: String,
        sections_rebuilt: usize,
        sections_kept: usize,
        chunks_built: usize,
    },
}

// ---------------------------------------------------------------------------
// FNV-1a 64 段指纹（手写约 8 行，零新依赖）
// 口径 borrow：mdcore crates/mdcore/src/fingerprint.rs——
//   offset basis = 0xcbf2_9ce4_8422_2325，prime = 0x0000_0100_0000_01b3，
//   域标签隔离（同其 Hasher::new(tag)）+ 写入侧长度前缀防连拼歧义。
// ---------------------------------------------------------------------------

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// 段指纹域标签（与 mdcore 结构域 0xB7 / 内容域 0xC1 同语义：域隔离防串味）。
const SECTION_FP_TAG: u8 = 0x5A;

/// FNV-1a 64 单字节步进（wrapping 乘是 FNV 定义行为，非溢出事故）。
fn fnv_byte(h: u64, b: u8) -> u64 {
    (h ^ b as u64).wrapping_mul(FNV_PRIME)
}

/// FNV-1a 64 写 u64（LE 字节序——mdcore write_u64 同口径）。
fn fnv_u64(mut h: u64, v: u64) -> u64 {
    for b in v.to_le_bytes() {
        h = fnv_byte(h, b);
    }
    h
}

/// 段内容指纹：fnv64 over 逐 chunk 的 (idx, text)。
/// - idx 取 chunk.block_idx（文件内块序——段身份的组成部分）；
/// - text 写入前带字节长度前缀（防 (3,"ab") 与 (3,"a")(?,…) 类连拼歧义——
///   mdcore 口径：调用方负责长度前缀）；
/// - 首字节写域标签（段指纹域与未来其他指纹域互不串味）。
fn section_fingerprint(chunks: &[&Chunk]) -> u64 {
    let mut h = fnv_byte(FNV_OFFSET_BASIS, SECTION_FP_TAG);
    for c in chunks {
        h = fnv_u64(h, c.block_idx as u64);
        h = fnv_u64(h, c.text.len() as u64);
        for &b in c.text.as_bytes() {
            h = fnv_byte(h, b);
        }
    }
    h
}

/// 章节键：locator 首个路径段（"第三章/3.2.1" → "第三章"；无分隔符 → 整串）。
fn section_key(locator: &str) -> &str {
    locator.split('/').next().unwrap_or(locator)
}

impl ShardedKb {
    pub fn new() -> Self {
        ShardedKb {
            shards: BTreeMap::new(),
            total_chunks: 0,
        }
    }

    /// 增量式文件入片（唯一写入口），三通道（详见模块头「增量判定」）：
    /// - label 在位且 file_sha 相同 → no-op（Unchanged，零触碰）；
    /// - 新 label → 整片新建（Rebuilt）；
    /// - label 在位但 sha 变（补遗换版/文件修订）→ 段级 diff（Partial）。
    pub fn upsert_file(&mut self, parsed: &ParsedFile, label: &str) -> ShardUpdate {
        // 闸门 1（文件级）：指纹未变 → 整片零触碰（百万字文档的 no-op 快路径）。
        if self
            .shards
            .get(label)
            .is_some_and(|s| s.file_sha == parsed.sha256)
        {
            return ShardUpdate::Unchanged { file: label.to_string() };
        }

        // 新解析件切块 + 按章节键分组（BTreeMap：分组迭代序确定）。
        // 空文本块不入索引——与 Kb::ingest 同口径（两库检索等价的前提）。
        let mut new_sections: BTreeMap<String, Vec<Chunk>> = BTreeMap::new();
        for b in &parsed.blocks {
            if b.text.trim().is_empty() {
                continue;
            }
            let chunk = build_chunk(parsed, label, b);
            new_sections
                .entry(section_key(&b.locator).to_string())
                .or_default()
                .push(chunk);
        }

        // 取出旧片（在位 = 换版通道；不在位 = 新建通道）——统一到「无旧片借用」
        // 的单出口写回，避免持有 &self 的同时 insert。
        let old = self.shards.remove(label);

        let (shard, update, delta) = match old {
            None => {
                // 新片：整片新建，全段入指纹账本。
                let mut chunks = Vec::with_capacity(parsed.blocks.len());
                let mut fps = BTreeMap::new();
                for (k, cs) in &new_sections {
                    let refs: Vec<&Chunk> = cs.iter().collect();
                    fps.insert(k.clone(), section_fingerprint(&refs));
                    chunks.extend(cs.iter().cloned());
                }
                chunks.sort_by(|a, b| a.block_idx.cmp(&b.block_idx)); // 文档序
                let sections_touched = fps.len();
                let chunks_built = chunks.len();
                (
                    FileShard {
                        file: label.to_string(),
                        file_sha: parsed.sha256.clone(),
                        chunk_count: chunks_built,
                        section_fingerprints: fps,
                        chunks,
                    },
                    ShardUpdate::Rebuilt {
                        file: label.to_string(),
                        chunks_built,
                        sections_touched,
                    },
                    chunks_built as isize, // 无旧片：纯增
                )
            }
            Some(old) => {
                // 换版：段级 diff。旧片 chunk 先按章节键重组（原对象转移，
                // 保留段直接复用——零重建的结构证据 = chunk_id 原样保留）。
                let mut old_by_section: BTreeMap<String, Vec<Chunk>> = BTreeMap::new();
                for c in old.chunks {
                    old_by_section
                        .entry(section_key(&c.locator).to_string())
                        .or_default()
                        .push(c);
                }
                let mut chunks = Vec::with_capacity(parsed.blocks.len());
                let mut fps = BTreeMap::new();
                let (mut sections_rebuilt, mut sections_kept, mut chunks_built) = (0, 0, 0);
                for (k, cs) in &new_sections {
                    let refs: Vec<&Chunk> = cs.iter().collect();
                    let fp = section_fingerprint(&refs);
                    fps.insert(k.clone(), fp);
                    if old.section_fingerprints.get(k) == Some(&fp) {
                        // 指纹相同 → 保留旧段 chunk（零重建）。
                        sections_kept += 1;
                        if let Some(kept) = old_by_section.get(k) {
                            chunks.extend(kept.iter().cloned());
                        }
                    } else {
                        // 指纹不同 / 新增段 → 重建该段。
                        sections_rebuilt += 1;
                        chunks_built += cs.len();
                        chunks.extend(cs.iter().cloned());
                    }
                }
                // 消失的段：不进新片（其 chunk 随 old_by_section 一并丢弃）。
                chunks.sort_by(|a, b| a.block_idx.cmp(&b.block_idx)); // 恢复文档序
                let chunk_count = chunks.len();
                (
                    FileShard {
                        file: label.to_string(),
                        file_sha: parsed.sha256.clone(),
                        chunk_count,
                        section_fingerprints: fps,
                        chunks,
                    },
                    ShardUpdate::Partial {
                        file: label.to_string(),
                        sections_rebuilt,
                        sections_kept,
                        chunks_built,
                    },
                    chunk_count as isize - old.chunk_count as isize, // 换版：净增减
                )
            }
        };
        self.total_chunks = (self.total_chunks as isize + delta) as usize;
        self.shards.insert(label.to_string(), shard);
        update
    }

    /// 移除一个文件分片（补遗作废通道）：片内 chunk 一并出库。
    pub fn remove_file(&mut self, label: &str) -> Option<FileShard> {
        let shard = self.shards.remove(label)?;
        self.total_chunks -= shard.chunk_count;
        Some(shard)
    }

    pub fn shard(&self, label: &str) -> Option<&FileShard> {
        self.shards.get(label)
    }

    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    pub fn total_chunks(&self) -> usize {
        self.total_chunks
    }

    /// 跨片检索合并（与 Kb::search_exact 逐字节等价——同一查询同一命中同一排序）。
    /// 实现：线性扫各片 chunk（遍历序不影响输出——四级排序末级 chunk_id 内容
    /// 寻址唯一，总序确定），评分与排序走 lib.rs 共享核心（单一真源）。
    pub fn search_exact(&self, q: &ExactQuery) -> Vec<SearchHit> {
        let mut hits: Vec<SearchHit> = vec![];
        for shard in self.shards.values() {
            for c in &shard.chunks {
                if let Some(h) = hit_of_chunk(c, q) {
                    hits.push(h);
                }
            }
        }
        sort_hits(&mut hits);
        hits
    }
}

impl Default for ShardedKb {
    fn default() -> Self {
        Self::new()
    }
}
