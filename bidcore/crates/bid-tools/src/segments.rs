//! #254 章节级增量写入——服务层（分段写入的编排半边）。
//!
//! 域层（`bid-domain::segments`）只管账本与正文同进退；本模块补齐「一次
//! 分段提交」的完整编排，与 `draft_section`（整稿路径）同纪律：
//!
//! 1. **readiness 闸**（与整稿同口径——素材不齐照样拒，不因分段放宽）；
//! 2. **pre-flight 引用检查**：未知引用**整段拒收**（`E_UNPROCESSABLE`，
//!    零副作用）——不是整稿路径的「剔除 + 告警」。理由：分段路径若剔除，
//!    合稿 ≠ 段原文拼接，账本与正文的不变量破（域层模块头注释的分工）；
//! 3. 幂等键缺省 = 内容指纹（sha256 前 16 hex）——「同段同内容重提 =
//!    no-op」自动成立（#254 验收 2）；
//! 4. 域层 append/rewrite（CAS / 种子 / 合稿同步——见域层）；
//! 5. `changed = false`（幂等重放）→ 直接返回，不推进状态不落盘；
//! 6. 章节推进 DRAFTED + 项目推进 DRAFTING（与整稿同链）；
//! 7. S-2 落盘（合稿全文经 ChapterAssembler）+ checkpoint；
//! 8. 无锚承诺告警（合稿面——与整稿同口径的响应面素材）。

use crate::draft::{
    advance_project_to_drafting, advance_section_to_drafted, build_section_input,
    count_unanchored_claims, scan_citations, verify_citations,
};
use crate::service::BidService;
use bid_domain::segments::{SegmentChange, SectionSegment};
use bid_domain::ToolError;
use sha2::{Digest, Sha256};

/// append 的响应素材（MCP out 的业务半边）。
#[derive(Debug, Clone)]
pub struct SegmentWrite {
    pub section_id: String,
    pub segment_id: String,
    pub seq: u32,
    pub version: u32,
    /// 本次是否实际变更（false = 幂等重放/同内容 no-op）
    pub changed: bool,
    /// 本段字符数
    pub segment_chars: u32,
    /// 合稿字符数（章节正文现状）
    pub assembled_chars: u32,
    /// 章节配额（0 = 无配额）
    pub target_chars: u32,
    /// 剩余字数（target − 合稿；负值 = 超配）
    pub remaining_chars: i64,
    /// 合稿是否已超 +20% 带
    pub over_target: bool,
    /// 合稿的有效引用（REQ+EV 保序去重）
    pub citations: Vec<String>,
    /// 无锚承诺句数（合稿面）
    pub unanchored_claims: usize,
    pub warnings: Vec<String>,
    /// 段账本总数
    pub segment_count: u32,
}

/// list 的响应素材。
#[derive(Debug, Clone)]
pub struct SegmentInventory {
    pub section_id: String,
    pub segments: Vec<SectionSegment>,
    pub assembled_chars: u32,
    pub target_chars: u32,
    pub remaining_chars: i64,
    pub over_target: bool,
}

/// 内容指纹（幂等键缺省面）：sha256 前 16 hex——同段同内容重提自动 no-op。
fn content_fingerprint(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    format!("fp:{:016x}", u64::from_le_bytes(
        h.finalize()[..8].try_into().expect("sha256 ≥ 8 bytes"),
    ))
}

/// pre-flight 引用检查：段的全部引用 ⊆ 章节已知 REQ/EV 集合？
/// 违例 → E_UNPROCESSABLE（整段拒收，零副作用——不剔除，保不变量）。
/// 入参 `input` 由调用方先行装配（build_section_input 同时承担章节存在
/// 性/叶校验/空响应面——与 draft_section 同序，E_NOTFOUND 优先于 readiness）。
fn preflight_citations(
    input: &crate::draft::SectionDraftInput,
    section_id: &str,
    content: &str,
) -> Result<Vec<String>, ToolError> {
    let known: std::collections::HashSet<String> = input
        .requirements
        .iter()
        .map(|r| r.id.clone())
        .chain(input.evidences.iter().map(|e| e.id.clone()))
        .collect();
    let (valid, unknown) = verify_citations(content, &known);
    if !unknown.is_empty() {
        return Err(ToolError::EUnprocessable {
            reason: format!(
                "段落引用回查失败：{:?} 不在章节 {section_id} 的已知 REQ/EV 集合——\
                 分段路径不自动剔除（保「合稿 == 段拼接」不变量），请修正引用后重提",
                unknown
            ),
        });
    }
    Ok(valid)
}

/// 合稿统计（list / write 共用）。
fn assembly_stats(svc: &BidService, section_id: &str) -> Result<(u32, u32, i64, bool), ToolError> {
    let node = svc
        .store()
        .sections()
        .iter()
        .find(|n| n.id == section_id)
        .ok_or_else(|| ToolError::ENotFound {
            what: format!("section {section_id} 不存在"),
        })?;
    let assembled = node.content.as_ref().map(|c| c.chars().count()).unwrap_or(0);
    let target = node.target_chars;
    let remaining = i64::from(target) - assembled as i64;
    let over = target > 0 && assembled as f64 > f64::from(target) * 1.20;
    Ok((assembled as u32, target, remaining, over))
}

impl BidService {
    /// 追加一段（#254 主路径——长章节的安全续写面）。
    pub fn append_segment(
        &mut self,
        section_id: &str,
        content: &str,
        idem_key: Option<&str>,
    ) -> Result<SegmentWrite, ToolError> {
        if content.trim().is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: format!("{section_id} 段内容不得为空白（空段无提交意义）"),
            });
        }
        // ---- 1. 章节定位 + 输入装配（E_NOTFOUND / 结构节点 / 空响应面——
        //         与 draft_section 同序，存在性错误优先于 readiness 阻塞）
        let (input, _) = build_section_input(self.store(), section_id, None, None, None)?;
        // ---- 2. readiness 闸（与 draft_section 同口径）
        let readiness = crate::readiness::check_draft_readiness(self.store(), section_id);
        if !readiness.ready {
            return Err(ToolError::EUnprocessable {
                reason: format!("章节 {section_id} {}", readiness.to_message()),
            });
        }
        // ---- 3. pre-flight 引用检查（未知引用整段拒——零副作用）
        preflight_citations(&input, section_id, content)?;
        // ---- 3. 幂等键缺省 = 内容指纹
        let key = idem_key
            .map(str::to_string)
            .unwrap_or_else(|| content_fingerprint(content));
        // ---- 4. 域层追加（幂等/CAS/种子/合稿同步）
        let SegmentChange { segment, changed } =
            self.store_mut()
                .append_section_segment(section_id, content.to_string(), &key)?;
        let section_id = segment.section_id.clone();
        if !changed {
            // 幂等重放：零副作用直返（不推进状态、不落盘）
            let (assembled, target, remaining, over) = assembly_stats(self, &section_id)?;
            return Ok(SegmentWrite {
                segment_id: segment.id,
                seq: segment.seq,
                version: segment.version,
                changed: false,
                segment_chars: content.chars().count() as u32,
                assembled_chars: assembled,
                target_chars: target,
                remaining_chars: remaining,
                over_target: over,
                citations: scan_citations(content),
                unanchored_claims: count_unanchored_claims(content),
                warnings: vec!["幂等重放（同幂等键）——账面零变动".into()],
                segment_count: self.store().list_section_segments(&section_id).len() as u32,
                section_id,
            });
        }
        // ---- 5. 状态推进（与整稿同链）
        advance_section_to_drafted(self.store_mut(), &section_id)?;
        advance_project_to_drafting(self.store_mut())?;
        // ---- 6. 合稿落盘（ChapterAssembler——与 draft_section 同一真源面）
        self.commit_assembled(&section_id)?;
        let out = self.segment_write_out(&section_id, segment, content)?;
        Ok(out)
    }

    /// 重写指定段（局部修订——不动兄弟段；CAS 过期 → E_CONFLICT）。
    pub fn rewrite_segment(
        &mut self,
        segment_id: &str,
        content: &str,
        expected_version: Option<u32>,
    ) -> Result<SegmentWrite, ToolError> {
        if content.trim().is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: format!("{segment_id} 重写内容不得为空白"),
            });
        }
        // 段定位（章节归属先查——pre-flight 需要章节上下文；E_NOTFOUND 优先）
        let section_id = self
            .store()
            .section_segment(segment_id)?
            .section_id
            .clone();
        let (input, _) = build_section_input(self.store(), &section_id, None, None, None)?;
        // ---- pre-flight（新内容的引用必须全合法——零副作用拒）
        preflight_citations(&input, &section_id, content)?;
        // ---- 域层重写（CAS：过期 version → E_CONFLICT 无部分写入）
        let SegmentChange { segment, changed } = self.store_mut().rewrite_section_segment(
            segment_id,
            content.to_string(),
            expected_version,
        )?;
        if !changed {
            let (assembled, target, remaining, over) = assembly_stats(self, &section_id)?;
            return Ok(SegmentWrite {
                segment_id: segment.id,
                seq: segment.seq,
                version: segment.version,
                changed: false,
                segment_chars: content.chars().count() as u32,
                assembled_chars: assembled,
                target_chars: target,
                remaining_chars: remaining,
                over_target: over,
                citations: scan_citations(content),
                unanchored_claims: count_unanchored_claims(content),
                warnings: vec!["同内容重写——账面零变动".into()],
                segment_count: self.store().list_section_segments(&section_id).len() as u32,
                section_id,
            });
        }
        advance_section_to_drafted(self.store_mut(), &section_id)?;
        self.commit_assembled(&section_id)?;
        let out = self.segment_write_out(&section_id, segment, content)?;
        Ok(out)
    }

    /// 段账本清单 + 合稿统计（Agent 续写前的发现面——只读零副作用）。
    pub fn list_segments(&self, section_id: &str) -> Result<SegmentInventory, ToolError> {
        let segments: Vec<SectionSegment> = self
            .store()
            .list_section_segments(section_id)
            .into_iter()
            .cloned()
            .collect();
        if segments.is_empty() {
            // 章节存在性守卫（无段也允许查——空账本返回空清单）
            self.store()
                .sections()
                .iter()
                .find(|n| n.id == section_id)
                .ok_or_else(|| ToolError::ENotFound {
                    what: format!("section {section_id} 不存在"),
                })?;
        }
        let (assembled, target, remaining, over) = assembly_stats(self, section_id)?;
        Ok(SegmentInventory {
            section_id: section_id.to_string(),
            segments,
            assembled_chars: assembled,
            target_chars: target,
            remaining_chars: remaining,
            over_target: over,
        })
    }

    // ---------------- 内部

    /// 合稿落盘 + checkpoint（与 draft_section 的 S-2 半边同源）。
    fn commit_assembled(&mut self, section_id: &str) -> Result<(), ToolError> {
        if let Some(dir) = self.workspace_dir() {
            let mut asm = crate::assembly::ChapterAssembler::open(&dir).map_err(|e| {
                ToolError::EUnprocessable {
                    reason: format!("章节工作区打开失败: {e}"),
                }
            })?;
            let merged = self
                .store()
                .assemble_section_segments(section_id)
                .unwrap_or_default();
            let citations = scan_citations(&merged);
            asm.commit_section_with_note(section_id, &merged, citations, None)
                .map_err(|e| ToolError::EUnprocessable {
                    reason: format!("章节工作区提交失败: {e}"),
                })?;
        }
        self.checkpoint().map_err(|e| ToolError::EUnprocessable {
            reason: format!("checkpoint 落盘失败（内存已变更、磁盘未跟上）: {e}"),
        })?;
        Ok(())
    }

    /// write 路径的响应组装（合稿面的统计 + 告警）。
    fn segment_write_out(
        &self,
        section_id: &str,
        segment: SectionSegment,
        _content: &str,
    ) -> Result<SegmentWrite, ToolError> {
        let (assembled, target, remaining, over) = assembly_stats(self, section_id)?;
        let merged = self
            .store()
            .assemble_section_segments(section_id)
            .unwrap_or_default();
        let mut warnings = Vec::new();
        let unanchored = count_unanchored_claims(&merged);
        if unanchored > 0 {
            warnings.push(format!(
                "无锚承诺 {unanchored} 句（合稿件——含承诺词但同句无 [REQ-*]/[EV-*] 引用）——\
                 建议补引用或改写；是否阻断由 validate 裁决"
            ));
        }
        if over {
            warnings.push(format!(
                "合稿 {assembled} 字已超配额 {target} 的 +20% 带——续写请节制（validate 上界为 MINOR 提示）"
            ));
        }
        Ok(SegmentWrite {
            section_id: section_id.to_string(),
            segment_id: segment.id,
            seq: segment.seq,
            version: segment.version,
            changed: true,
            segment_chars: segment.content.chars().count() as u32,
            assembled_chars: assembled,
            target_chars: target,
            remaining_chars: remaining,
            over_target: over,
            citations: scan_citations(&merged),
            unanchored_claims: unanchored,
            warnings,
            segment_count: self.store().list_section_segments(section_id).len() as u32,
        })
    }
}
