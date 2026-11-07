//! rewrite.rs —— #319 补遗后增量重写（服务层：登记编排 + 局部改写编排）。
//!
//! 域层（`bid-domain::rewrite`）只管账本与状态；本模块补齐编排半边：
//!
//! 1. **登记编排**（[`BidService::register_addendum_changes`]）：
//!    域层批量原子登记（条款集/章节树/DIRTY）→ **指令生成**（影响面 × 段账本
//!    匹配：段引用该 REQ ∪ 段含 old_text——#254 段是真增量单元）→ 矩阵幂等
//!    重算（一次）→ checkpoint；
//! 2. **局部改写编排**（[`BidService::rewrite_section_partial`]）：
//!    指令守卫（Pending / rewrites ⊆ 影响面）→ **改动前预告**（dry_run 纯
//!    预览：「将修改 X 段，Y 段不动」；真跑响应同面回显）→ pre-flight 全过
//!    才落笔（引用回查 + CAS——与 #254 rewrite_segment 同纪律，多段一次性
//!    预检 = 原子面）→ 段账本重写（兄弟段零触碰）→ 合稿落盘 → 指令处置
//!    （全部处置完清 DIRTY）→ **局部复验**（只校验受影响章节——
//!    `validate_document(Some(&[section_id]))`，未受影响章节零重跑）。
//!
//! ## 层级纪律
//!
//! - 段写入只走域层 `rewrite_section_segment`（不另起第二套写路径）；
//! - 复验只走 `validate_document` 的 scope 面（不另起校验器）；
//! - 「改了 N 段，M 条 REQ 重新响应」是派生统计（纯读——响应数 = 新内容
//!   引用的 distinct REQ 数）。

use crate::draft::{advance_section_to_drafted, build_section_input, scan_citations};
use crate::service::BidService;
use bid_domain::rewrite::{
    AddendumChangeRecord, AffectedSegment, ChangeSpec, ChangeType, DirectiveStatus,
    RegisteredChange, RewriteDirective,
};
use bid_domain::segments::SegmentChange;
use bid_domain::ToolError;

/// 登记编排的完整响应素材。
pub struct RegisterChangesOut {
    pub changes: Vec<RegisteredChange>,
    pub directives: Vec<RewriteDirective>,
    /// 受影响章节的去重清单（DIRTY 已打标——发现面）
    pub dirty_sections: Vec<String>,
    /// 指令级告警（无段账本/未匹配到段等——与 directives 同序归集）
    pub directive_warnings: Vec<String>,
    /// 矩阵重算后的 (行数, blocking 行数)
    pub matrix_rows: usize,
    pub matrix_blocking: usize,
}

/// 局部改写的一段（MCP 入参形态——服务层翻译）。
#[derive(Debug, Clone)]
pub struct SegmentRewrite {
    pub segment_id: String,
    pub content: String,
    pub expected_version: Option<u32>,
}

/// 局部改写的响应素材。
pub struct PartialRewriteOut {
    pub directive: RewriteDirective,
    /// 将修改的段数（= rewrites 数）
    pub segments_to_modify: usize,
    /// 不动的段数（段账本总数 − 修改数）
    pub segments_untouched: usize,
    pub dry_run: bool,
    /// 实际改动的段（id / seq / 版本推进 / 字数）
    pub modified: Vec<ModifiedSegment>,
    pub untouched_segment_ids: Vec<String>,
    pub assembled_chars: u32,
    pub target_chars: u32,
    pub remaining_chars: i64,
    pub over_target: bool,
    /// 局部复验（只跑受影响章节）
    pub revalidation: Revalidation,
    pub remaining_pending: Vec<String>,
    pub dirty_cleared: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ModifiedSegment {
    pub segment_id: String,
    pub seq: u32,
    pub version_before: u32,
    pub version_after: u32,
    pub chars: u32,
}

/// 局部复验摘要（validate 的 scope 面投影）。
#[derive(Debug, Clone)]
pub struct Revalidation {
    pub section_id: String,
    pub ok: bool,
    pub blocker: usize,
    pub major: usize,
    pub minor: usize,
    /// 复验面 = 章节责任条款数（scope 命中章节的 requirement_ids 去重）
    pub reqs_revalidated: usize,
    /// 重新响应 = 新内容引用的 distinct REQ 数
    pub reqs_responded: usize,
}

impl BidService {
    /// #319 ①+② 登记：变更落账 + 指令生成 + 矩阵重算。
    pub fn register_addendum_changes(
        &mut self,
        specs: Vec<ChangeSpec>,
    ) -> Result<RegisterChangesOut, ToolError> {
        // ---- 1. 域层批量原子登记（含条款集/章节树维护 + DIRTY 打标）
        let changes = self.store_mut().register_addendum_changes(&specs)?;
        // ---- 2. 指令生成（变更 × 受影响章节 × 段账本匹配）
        let mut directives: Vec<RewriteDirective> = Vec::new();
        let mut dirty_sections: Vec<String> = Vec::new();
        let mut directive_warnings: Vec<String> = Vec::new();
        for reg in &changes {
            for sec in &reg.affected_sections {
                if !dirty_sections.contains(sec) {
                    dirty_sections.push(sec.clone());
                }
                let affected = self.match_affected_segments(&reg.change, sec);
                let d =
                    self.store_mut()
                        .store_rewrite_directive(&reg.change, sec, affected.segments);
                if let Some(w) = affected.warning {
                    // 指令级告警归集在响应面（账本只存事实——warnings 不入账）
                    directive_warnings.push(format!("指令 {}（{}）：{w}", d.id, sec));
                }
                directives.push(d);
            }
        }
        // ---- 3. 矩阵幂等重算（一次——避免逐变更重算的版本抖动；
        //         行集变化（add/del）即时反映，blocking 派生同步刷新）
        let (matrix_rows, matrix_blocking) = self.store_mut().build_matrix()?;
        // ---- 4. checkpoint（登记是写路径——落盘纪律与其它写工具一致）
        self.auto_checkpoint()?;
        Ok(RegisterChangesOut {
            changes,
            directives,
            dirty_sections,
            directive_warnings,
            matrix_rows,
            matrix_blocking,
        })
    }

    /// 段匹配（#319 ①：影响面 → 受影响段）。
    ///
    /// 匹配腿：段引用该 REQ（`[REQ-xxxx]`/裸 token——`scan_citations` 同源）
    /// ∪ 段含 old_text（词面定位——补遗删改的旧文本）。新增要求恒空
    /// （无既有段可匹配——响应走 append 新段）。
    fn match_affected_segments(
        &self,
        change: &AddendumChangeRecord,
        section_id: &str,
    ) -> MatchedSegments {
        if change.change_type == ChangeType::RequirementAdded {
            return MatchedSegments {
                segments: vec![],
                warning: Some(
                    "新增要求无既有段——响应走 append_section_segment 新增段（引用该 REQ 过 pre-flight）"
                        .into(),
                ),
            };
        }
        let segments = self.store().list_section_segments(section_id);
        if segments.is_empty() {
            let drafted = self
                .store()
                .sections()
                .iter()
                .find(|n| n.id == section_id)
                .map(|n| n.content.as_deref().map(str::trim).unwrap_or("").len() > 0)
                .unwrap_or(false);
            let hint = if drafted {
                "章节正文无段账本（整稿路径写的）——局部改写需要段账本：重写受影响内容为分段（append+rewrite），或整章重写后本指令可 dismiss"
            } else {
                "章节未起草——后续起草自然消费最新要求（params.addendum_requirement），无需改写指令，可 dismiss"
            };
            return MatchedSegments {
                segments: vec![],
                warning: Some(hint.into()),
            };
        }
        let old_text = change.old_text.trim();
        let mut out = Vec::new();
        for seg in segments {
            let cites = scan_citations(&seg.content);
            let by_citation = cites.iter().any(|c| c == &change.req_id);
            let by_old_text = !old_text.is_empty() && seg.content.contains(old_text);
            if by_citation || by_old_text {
                let matched_by = match (by_citation, by_old_text) {
                    (true, true) => "citation+old_text",
                    (true, false) => "citation",
                    _ => "old_text",
                };
                out.push(AffectedSegment {
                    segment_id: seg.id.clone(),
                    seq: seg.seq,
                    matched_by: matched_by.into(),
                });
            }
        }
        let warning = if out.is_empty() {
            Some(format!(
                "未匹配到受影响段（正文未引用 {} 且旧文本不在任何段中）——请人工核对新要求落点：若已有内容实际覆盖新要求，dismiss 本指令",
                change.req_id
            ))
        } else {
            None
        };
        MatchedSegments {
            segments: out,
            warning,
        }
    }

    /// #319 ②+③ 局部改写 + 局部复验。
    ///
    /// `dry_run = true`：纯预览（「将修改 X 段，Y 段不动」+ 逐段对照），
    /// 零副作用；`rewrites` 为空且 `dismiss = false`：确认无需改写 → 指令
    /// 直接处置（Applied，零段改动）；`dismiss = true`：人工裁定不适用。
    pub fn rewrite_section_partial(
        &mut self,
        directive_id: &str,
        rewrites: Vec<SegmentRewrite>,
        dry_run: bool,
        dismiss: bool,
    ) -> Result<PartialRewriteOut, ToolError> {
        // ---- 0. 指令守卫（E_NOTFOUND / 状态）
        let directive = self.store().rewrite_directive(directive_id)?.clone();
        if directive.status != DirectiveStatus::Pending {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "指令 {directive_id} 状态为 {}（仅 pending 可执行——重放不是错误但不再改稿）",
                    directive.status.as_str()
                ),
            });
        }
        let section_id = directive.section_id.clone();
        if dismiss {
            if !rewrites.is_empty() {
                return Err(ToolError::EUnprocessable {
                    reason: "dismiss 与 rewrites 互斥（裁定不适用就不改段——rewrites 必须为空）"
                        .into(),
                });
            }
            let done = self.store_mut().dismiss_rewrite_directive(directive_id)?;
            self.auto_checkpoint()?;
            return Ok(PartialRewriteOut {
                segments_to_modify: 0,
                segments_untouched: self.store().list_section_segments(&section_id).len(),
                dry_run: false,
                directive: done,
                modified: vec![],
                untouched_segment_ids: self
                    .store()
                    .list_section_segments(&section_id)
                    .into_iter()
                    .map(|s| s.id.clone())
                    .collect(),
                assembled_chars: 0,
                target_chars: 0,
                remaining_chars: 0,
                over_target: false,
                revalidation: Revalidation {
                    section_id: section_id.clone(),
                    ok: true,
                    blocker: 0,
                    major: 0,
                    minor: 0,
                    reqs_revalidated: 0,
                    reqs_responded: 0,
                },
                remaining_pending: self
                    .store()
                    .pending_rewrite_directives_of(&section_id)
                    .into_iter()
                    .map(|d| d.id.clone())
                    .collect(),
                dirty_cleared: self
                    .store()
                    .pending_rewrite_directives_of(&section_id)
                    .is_empty(),
                warnings: vec!["人工裁定不适用（Dismissed）——零段改动".into()],
            });
        }

        // ---- 1. rewrites ⊆ 指令影响面（越界段显式拒——防「借指令改无关段」）
        let allowed: std::collections::HashSet<&str> = directive
            .affected_segments
            .iter()
            .map(|a| a.segment_id.as_str())
            .collect();
        for r in &rewrites {
            if !allowed.contains(r.segment_id.as_str()) {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "段 {} 不在指令 {} 的影响面内（{:?}）——局部改写只许改受影响段",
                        r.segment_id,
                        directive_id,
                        directive
                            .affected_segments
                            .iter()
                            .map(|a| a.segment_id.clone())
                            .collect::<Vec<_>>()
                    ),
                });
            }
            if r.content.trim().is_empty() {
                return Err(ToolError::EUnprocessable {
                    reason: format!("段 {} 的重写内容不得为空白", r.segment_id),
                });
            }
        }

        // ---- 2. 段定位 + pre-flight 全过才落笔（原子面：引用回查 + CAS）
        let section_id = directive.section_id.clone();
        let (input, _) = build_section_input(self.store(), &section_id, None, None, None)?;
        let mut pre_checked: Vec<(&SegmentRewrite, u32)> = Vec::with_capacity(rewrites.len());
        for r in &rewrites {
            let seg = self.store().section_segment(&r.segment_id)?;
            if seg.section_id != section_id {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "段 {} 属于章节 {}（指令针对 {section_id}）——错位段拒绝",
                        r.segment_id, seg.section_id
                    ),
                });
            }
            // CAS 预检（与 #254 同语义：过期 version → E_CONFLICT，零部分写入）
            if let Some(exp) = r.expected_version {
                if seg.version != exp {
                    return Err(ToolError::EConflict {
                        expected: u64::from(exp),
                        got: u64::from(seg.version),
                    });
                }
            }
            // 引用回查（新内容的引用必须全合法——整段拒，零副作用）
            let known: std::collections::HashSet<String> = input
                .requirements
                .iter()
                .map(|q| q.id.clone())
                .chain(input.evidences.iter().map(|e| e.id.clone()))
                .collect();
            let (_, unknown) = crate::draft::verify_citations(&r.content, &known);
            if !unknown.is_empty() {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "段 {} 新内容引用回查失败：{:?} 不在章节 {section_id} 的已知 REQ/EV 集合——修正引用后重提",
                        r.segment_id, unknown
                    ),
                });
            }
            pre_checked.push((r, seg.version));
        }

        let total_segments = self.store().list_section_segments(&section_id).len();
        let untouched_ids: Vec<String> = self
            .store()
            .list_section_segments(&section_id)
            .into_iter()
            .filter(|s| !rewrites.iter().any(|r| r.segment_id == s.id))
            .map(|s| s.id.clone())
            .collect();

        // ---- 3. dry_run：改动前预告（纯预览，零副作用）
        let untouched_ids_len = untouched_ids.len();
        if dry_run {
            return Ok(PartialRewriteOut {
                segments_to_modify: rewrites.len(),
                segments_untouched: untouched_ids.len(),
                dry_run: true,
                directive: directive.clone(),
                modified: vec![],
                untouched_segment_ids: untouched_ids,
                assembled_chars: 0,
                target_chars: 0,
                remaining_chars: 0,
                over_target: false,
                revalidation: Revalidation {
                    section_id: section_id.clone(),
                    ok: true,
                    blocker: 0,
                    major: 0,
                    minor: 0,
                    reqs_revalidated: 0,
                    reqs_responded: 0,
                },
                remaining_pending: self
                    .store()
                    .pending_rewrite_directives_of(&section_id)
                    .into_iter()
                    .map(|d| d.id.clone())
                    .collect(),
                dirty_cleared: false,
                warnings: vec![format!(
                    "DRY RUN：将修改 {} 段，{} 段不动（未落笔——确认后去掉 dry_run 重提）",
                    rewrites.len(),
                    untouched_ids_len
                )],
            });
        }

        // ---- 4. 落笔（逐段走域层 rewrite——pre-flight 已全过，此处不再失败）
        let mut modified = Vec::with_capacity(rewrites.len());
        let mut applied_ids = Vec::with_capacity(rewrites.len());
        for (r, version_before) in pre_checked {
            let SegmentChange { segment, changed } = self.store_mut().rewrite_section_segment(
                &r.segment_id,
                r.content.clone(),
                r.expected_version,
            )?;
            applied_ids.push(segment.id.clone());
            modified.push(ModifiedSegment {
                segment_id: segment.id.clone(),
                seq: segment.seq,
                version_before,
                version_after: segment.version,
                chars: segment.content.chars().count() as u32,
            });
            if !changed {
                // 同内容 no-op：不算改段（预告面如实反映）
                modified.pop();
                applied_ids.pop();
            }
        }
        debug_assert_eq!(
            total_segments,
            rewrites.len() + untouched_ids.len(),
            "段账本守恒：局部改写不增删段（改写面 + 不动面 = 段账本全量）"
        );

        // ---- 5. 状态推进 + 合稿落盘（与 #254 rewrite_segment 同链）
        if !modified.is_empty() {
            advance_section_to_drafted(self.store_mut(), &section_id)?;
            self.commit_assembled(&section_id)?;
        }

        // ---- 6. 指令处置（全部处置完 → 清 DIRTY）
        let done = self
            .store_mut()
            .apply_rewrite_directive(directive_id, &applied_ids)?;

        // ---- 7. 局部复验（只跑受影响章节——未受影响章节零重跑）
        let report = self.validate_document(Some(&[section_id.clone()]))?;
        let node = self
            .store()
            .sections()
            .iter()
            .find(|n| n.id == section_id)
            .ok_or_else(|| ToolError::ENotFound {
                what: format!("section {section_id} 不存在"),
            })?;
        let reqs_revalidated = node.requirement_ids.len();
        let reqs_responded = {
            let mut ids: Vec<String> = Vec::new();
            for m in &modified {
                for c in scan_citations(
                    &self
                        .store()
                        .section_segment(&m.segment_id)
                        .map(|s| s.content.clone())
                        .unwrap_or_default(),
                ) {
                    if !ids.contains(&c) {
                        ids.push(c);
                    }
                }
            }
            ids.len()
        };
        self.auto_checkpoint()?;

        let assembled = node
            .content
            .as_ref()
            .map(|c| c.chars().count())
            .unwrap_or(0) as u32;
        let target = node.target_chars;
        let remaining = i64::from(target) - assembled as i64;
        let over = target > 0 && assembled as f64 > f64::from(target) * 1.20;
        let remaining_pending: Vec<String> = self
            .store()
            .pending_rewrite_directives_of(&section_id)
            .into_iter()
            .map(|d| d.id.clone())
            .collect();
        let dirty_cleared = remaining_pending.is_empty();
        let mut warnings = Vec::new();
        if modified.is_empty() {
            warnings.push("零段实际改动（同内容重写/空 rewrites）——指令已处置，正文未变".into());
        }
        if report.counts.blocker > 0 {
            warnings.push(format!(
                "局部复验发现 {} 条 BLOCKER（scope={section_id}）——按 findings 修稿后重验",
                report.counts.blocker
            ));
        }
        Ok(PartialRewriteOut {
            directive: done,
            segments_to_modify: rewrites.len(),
            segments_untouched: untouched_ids.len(),
            dry_run: false,
            modified,
            untouched_segment_ids: untouched_ids,
            assembled_chars: assembled,
            target_chars: target,
            remaining_chars: remaining,
            over_target: over,
            revalidation: Revalidation {
                section_id: section_id.clone(),
                ok: report.ok,
                blocker: report.counts.blocker,
                major: report.counts.major,
                minor: report.counts.minor,
                reqs_revalidated,
                reqs_responded,
            },
            remaining_pending,
            dirty_cleared,
            warnings,
        })
    }
}

/// 段匹配的内部产物。
struct MatchedSegments {
    segments: Vec<AffectedSegment>,
    warning: Option<String>,
}
