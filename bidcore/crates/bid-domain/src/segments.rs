//! #254 章节级增量写入——分段账本（域层）。
//!
//! ## 定位（与既有面的分工）
//!
//! - **`OutlineNode.content` 仍是章节正文的单一真相**：引用回查、无锚承诺
//!   检查、字符统计、`E_CHARS_BELOW_MIN` 门禁、checkpoint、导出——全部
//!   只看 `content`（合稿），不看单段；
//! - **segments 是「怎么写出来」的账本**：每段稳定 ID + 顺序 + 幂等键 +
//!   版本；合稿 = 有序段 `join("\n\n")`；
//! - **核心不变量**：`segments(section) 非空 ⇒ node.content ==
//!   assemble(segments)`。账本与正文同进退（append/rewrite 先改账本再同步
//!   正文；整稿提交与合稿不同 ⇒ 账本重置为单段——见
//!   [`DomainStore::reset_section_segments_to_single`]）。
//!
//! ## 为什么 reject 而不是剔除（与整稿路径的语义差异）
//!
//! `draft_section` 对幻觉引用是「剔除 + 告警」（正文可修复后落库）；分段
//! 路径若也剔除，合稿 ≠ 段拼接，不变量破——所以服务层在入账**前**预检
//! 引用，未知引用整段拒收（`E_UNPROCESSABLE`，零副作用）。见
//! bid-tools `segments.rs` 的 pre-flight。
//!
//! ## 持久化（加性迁移）
//!
//! `StoreSnapshot.segments`（`#[serde(default)]`）——旧快照空集合加载，
//! 行为与旧版一致；`schema_version` 不 bump（头注释的加性演进纪律）。

use crate::{DomainStore, ToolError};
use serde::{Deserialize, Serialize};

/// 章节正文分段（SEG-xxxx）。合稿语义见模块头注释。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionSegment {
    /// 稳定 ID（id_seq 签发，SEG-0001 起）
    pub id: String,
    /// 所属叶章节（SEC-xxxx）
    pub section_id: String,
    /// 章节内顺序（1 起，追加递增；合稿按 seq 升序组装）
    pub seq: u32,
    /// 段正文（合稿的组成单元——不单独过门禁）
    pub content: String,
    /// 幂等键（同章节同 key 重放 = no-op；缺省由服务层按内容指纹生成）
    pub idem_key: String,
    /// 段版本（1 起；rewrite 不同内容 +1——审计面）
    pub version: u32,
    /// 创建时间（epoch 秒——审计面，不参与语义）
    pub created_at: u64,
}

/// append/rewrite 的账本结果（`changed=false` = 幂等重放/同内容 no-op）。
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentChange {
    pub segment: SectionSegment,
    pub changed: bool,
}

/// 段间分隔符（合稿组装口径——两个全角换行，与中文正文自然段一致）。
pub const SEGMENT_JOIN: &str = "\n\n";

impl DomainStore {
    // ------------------------------------------------------------ 读面

    /// 指定章节的分段清单（按 seq 升序；无段 = 空表）。
    pub fn list_section_segments(&self, section_id: &str) -> Vec<&SectionSegment> {
        let mut rows: Vec<&SectionSegment> = self
            .segments
            .iter()
            .filter(|s| s.section_id == section_id)
            .collect();
        rows.sort_by_key(|s| s.seq);
        rows
    }

    /// 段ByID（rewrite 的定位面）。
    pub fn section_segment(&self, segment_id: &str) -> Result<&SectionSegment, ToolError> {
        self.segments
            .iter()
            .find(|s| s.id == segment_id)
            .ok_or_else(|| ToolError::ENotFound {
                what: format!("segment {segment_id} 不存在"),
            })
    }

    /// 合稿：有序段 `join(SEGMENT_JOIN)`；无段 = None。
    /// （单一组装口径——服务层/工具面/测试都走这里，不得各自拼接。）
    pub fn assemble_section_segments(&self, section_id: &str) -> Option<String> {
        let rows = self.list_section_segments(section_id);
        if rows.is_empty() {
            return None;
        }
        Some(
            rows.into_iter()
                .map(|s| s.content.as_str())
                .collect::<Vec<_>>()
                .join(SEGMENT_JOIN),
        )
    }

    // ------------------------------------------------------------ 写面

    /// 追加段（幂等：同章节同 `idem_key` → 重放不追加，`changed=false`）。
    ///
    /// - 章节不存在 → E_NOTFOUND；结构节点（有子）→ E_UNPROCESSABLE
    ///   （正文只住叶——与 `set_section_content` 同口径）；
    /// - **种子语义**：首段追加到「无账本但正文非空」的章节（此前经整稿
    ///   路径写过）→ 既有正文自动入册为第 1 段（`seed` 键），新段为第 2 段
    ///   ——**绝不静默丢正文**（#254 的核心诉求）；
    /// - 账本与正文同进退：追加后立即按合稿同步 `set_section_content`。
    pub fn append_section_segment(
        &mut self,
        section_id: &str,
        content: String,
        idem_key: &str,
    ) -> Result<SegmentChange, ToolError> {
        self.guard_segment_section(section_id)?;
        // 幂等重放（同章节同 key——账本不动，正文也不动）
        if let Some(hit) = self
            .segments
            .iter()
            .find(|s| s.section_id == section_id && s.idem_key == idem_key)
        {
            let seg = hit.clone();
            return Ok(SegmentChange { segment: seg, changed: false });
        }
        // 种子：无账本 + 正文非空 → 既有正文入册为第 1 段
        if self.list_section_segments(section_id).is_empty() {
            if let Some(existing) = self
                .sections
                .iter()
                .find(|n| n.id == section_id)
                .and_then(|n| n.content.clone())
                .filter(|c| !c.trim().is_empty())
            {
                let seed = SectionSegment {
                    id: self.next_id("SEG"),
                    section_id: section_id.to_string(),
                    seq: 1,
                    content: existing,
                    idem_key: "seed:existing-content".to_string(),
                    version: 1,
                    created_at: now_epoch(),
                };
                self.segments.push(seed);
            }
        }
        let seq = self
            .list_section_segments(section_id)
            .last()
            .map(|s| s.seq.saturating_add(1))
            .unwrap_or(1);
        let segment = SectionSegment {
            id: self.next_id("SEG"),
            section_id: section_id.to_string(),
            seq,
            content,
            idem_key: idem_key.to_string(),
            version: 1,
            created_at: now_epoch(),
        };
        self.segments.push(segment.clone());
        self.sync_section_content_from_segments(section_id)?;
        Ok(SegmentChange { segment, changed: true })
    }

    /// 重写指定段（同内容 no-op；不同内容 → `version`+1，其余段不动）。
    /// 账本与正文同进退：改后按合稿同步 `set_section_content`。
    ///
    /// CAS（#254 验收 4——并发/过期版本写入明确拒绝且无部分写入）：
    /// `expected_version` 与段当前 version 不匹配 → `E_CONFLICT`。
    /// 校验全过才落账（段内容 / 合稿正文 / content_version 均不动）。
    pub fn rewrite_section_segment(
        &mut self,
        segment_id: &str,
        content: String,
        expected_version: Option<u32>,
    ) -> Result<SegmentChange, ToolError> {
        let segment = self.section_segment(segment_id)?;
        // 过期版本显式拒（并发写作面——拿旧 version 覆盖新账 = 丢修订）
        if let Some(exp) = expected_version {
            if segment.version != exp {
                return Err(ToolError::EConflict {
                    expected: u64::from(exp),
                    got: u64::from(segment.version),
                });
            }
        }
        if segment.content == content {
            let seg = segment.clone();
            return Ok(SegmentChange { segment: seg, changed: false });
        }
        let idx = self.segments.iter().position(|s| s.id == segment_id)
            .expect("section_segment 已定位");
        self.segments[idx].content = content;
        self.segments[idx].version = self.segments[idx].version.saturating_add(1);
        let seg = self.segments[idx].clone();
        self.sync_section_content_from_segments(&seg.section_id)?;
        Ok(SegmentChange { segment: seg, changed: true })
    }

    /// 整稿替换联动：该章节分段账本重置为单段（内容 = 整稿正文）。
    ///
    /// 调用方：`draft_section` 在「有账本且合稿 ≠ 本次整稿内容」时调用
    /// （整稿替换语义——旧账本与新正文不一致就是陈账，不静默保留）。
    /// **不**在此同步正文——调用方（draft_section）随后的
    /// `set_section_content` 负责落正文（同值 no-op/异值 CAS）。
    pub fn reset_section_segments_to_single(
        &mut self,
        section_id: &str,
        content: &str,
        idem_key: &str,
    ) -> Result<SectionSegment, ToolError> {
        self.guard_segment_section(section_id)?;
        self.segments
            .retain(|s| s.section_id != section_id);
        let segment = SectionSegment {
            id: self.next_id("SEG"),
            section_id: section_id.to_string(),
            seq: 1,
            content: content.to_string(),
            idem_key: idem_key.to_string(),
            version: 1,
            created_at: now_epoch(),
        };
        self.segments.push(segment.clone());
        Ok(segment)
    }

    // ------------------------------------------------------------ 内部

    /// 分段写入的章节守卫：存在 + 叶（与 `set_section_content` 同口径——
    /// 结构节点是调度聚合单元，不承载内容）。
    fn guard_segment_section(&self, section_id: &str) -> Result<(), ToolError> {
        self.sections.iter().find(|n| n.id == section_id).ok_or_else(|| {
            ToolError::ENotFound { what: format!("section {section_id} 不存在") }
        })?;
        let has_child = self.sections.iter().any(|n| n.parent_id.as_deref() == Some(section_id));
        if has_child {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "{section_id} 是结构节点（有子节点）——正文只住叶节点，分段不适用"
                ),
            });
        }
        Ok(())
    }

    /// 账本 → 正文同步（append/rewrite 的「同进退」半边）。
    /// 走既有 `set_section_content`：CAS/attempt/content_version 语义不变。
    fn sync_section_content_from_segments(&mut self, section_id: &str)
        -> Result<(u32, u32), ToolError> {
        let merged = self
            .assemble_section_segments(section_id)
            .expect("同步前必有段（调用点保证）");
        self.set_section_content(section_id, merged)
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- 单元测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::{OutlineNode, SectionStatus};
    use crate::{ParseStatus, Requirement};

    /// 基座：项目 + 文件 + 1 条 REQ + 单叶树（与 outline.rs `store_with_reqs`
    /// 同款入口——受审方法链，不绕过状态机）。
    fn store_with_leaf(content: Option<&str>) -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("测试项目", "ZFCG-2026-GK-0311", "明标", "ext-key-seg")
            .unwrap();
        s.upsert_file("A-招标文件.docx", "招标文件", "aa", 31, 12, 120, 0,
                      ParseStatus::Ok, "").unwrap();
        s.set_requirements(vec![Requirement {
            id: "REQ-0003".into(),
            category: "商务要求".into(),
            clause_no: Some("3.1".into()),
            anchors: vec![],
            params: std::collections::HashMap::new(),
            source_file: "A".into(),
        }])
        .unwrap();
        let node = OutlineNode {
            id: "SEC-0001".into(),
            parent_id: None,
            title: "叶章节".into(),
            level: 1,
            order: 1,
            requirement_ids: vec!["REQ-0003".into()],
            target_chars: 100,
            status: SectionStatus::Pending,
            dirty: false,
            content: content.map(str::to_string),
            content_attempt: 0,
        };
        s.set_outline(vec![node]).unwrap();
        s
    }

    /// 顺序/合稿/幂等三合一主路径。
    #[test]
    fn append_assembles_in_seq_order_and_is_idempotent() {
        let mut s = store_with_leaf(None);
        let a = s
            .append_section_segment("SEC-0001", "第一段".into(), "k1")
            .unwrap();
        assert!(a.changed && a.segment.seq == 1);
        let b = s
            .append_section_segment("SEC-0001", "第二段".into(), "k2")
            .unwrap();
        assert!(b.changed && b.segment.seq == 2);
        // 合稿 = 有序拼接
        assert_eq!(
            s.assemble_section_segments("SEC-0001").unwrap(),
            "第一段\n\n第二段"
        );
        // 正文与账本同进退（核心不变量）
        assert_eq!(
            s.sections.iter().find(|n| n.id == "SEC-0001").unwrap().content.as_deref(),
            Some("第一段\n\n第二段")
        );
        // 同 key 重放：账面零变动
        let replay = s
            .append_section_segment("SEC-0001", "重复段".into(), "k1")
            .unwrap();
        assert!(!replay.changed, "同 idem_key 重放不追加");
        assert_eq!(s.list_section_segments("SEC-0001").len(), 2);
        assert_eq!(
            s.assemble_section_segments("SEC-0001").unwrap(),
            "第一段\n\n第二段",
            "重放不改变合稿"
        );
    }

    /// 种子语义：整稿写过的章节首段追加 → 既有正文入册为第 1 段（不丢）。
    #[test]
    fn first_append_seeds_existing_content_as_segment_one() {
        let mut s = store_with_leaf(Some("既有整稿正文"));
        let a = s
            .append_section_segment("SEC-0001", "新增段".into(), "k1")
            .unwrap();
        assert_eq!(a.segment.seq, 2, "既有正文为第 1 段，新段第 2 段");
        let rows = s.list_section_segments("SEC-0001");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].content, "既有整稿正文");
        assert_eq!(rows[0].idem_key, "seed:existing-content");
        assert_eq!(
            s.assemble_section_segments("SEC-0001").unwrap(),
            "既有整稿正文\n\n新增段"
        );
    }

    /// 重写首段：只动目标段（version+1），后续段与顺序不变。
    #[test]
    fn rewrite_first_segment_touches_only_target() {
        let mut s = store_with_leaf(None);
        let a = s.append_section_segment("SEC-0001", "甲".into(), "k1").unwrap();
        s.append_section_segment("SEC-0001", "乙".into(), "k2").unwrap();
        s.append_section_segment("SEC-0001", "丙".into(), "k3").unwrap();
        let r = s
            .rewrite_section_segment(&a.segment.id, "甲改".into(), None)
            .unwrap();
        assert!(r.changed && r.segment.version == 2, "不同内容 → version+1");
        assert_eq!(
            s.assemble_section_segments("SEC-0001").unwrap(),
            "甲改\n\n乙\n\n丙",
            "只改首段，后续段不动"
        );
        // 同内容重写 = no-op（version 不动）
        let again = s
            .rewrite_section_segment(&a.segment.id, "甲改".into(), None)
            .unwrap();
        assert!(!again.changed && again.segment.version == 2);
    }

    /// 整稿替换联动：重置为单段 + 旧账清空。
    #[test]
    fn reset_to_single_replaces_ledger() {
        let mut s = store_with_leaf(None);
        s.append_section_segment("SEC-0001", "旧一".into(), "k1").unwrap();
        s.append_section_segment("SEC-0001", "旧二".into(), "k2").unwrap();
        let seg = s
            .reset_section_segments_to_single("SEC-0001", "全新整稿", "full:hash1")
            .unwrap();
        assert_eq!(seg.seq, 1);
        assert_eq!(s.list_section_segments("SEC-0001").len(), 1);
        assert_eq!(s.assemble_section_segments("SEC-0001").unwrap(), "全新整稿");
    }

    /// 结构节点拒绝分段（与 set_section_content 同口径）。
    #[test]
    fn structural_node_rejects_segments() {
        let mut s = store_with_leaf(None);
        let parent = OutlineNode {
            id: "SEC-P".into(),
            parent_id: None,
            title: "父".into(),
            level: 1,
            order: 1,
            requirement_ids: vec!["REQ-0003".into()],
            target_chars: 100, // 结构节点配额 = 子配额和（账平校验）
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        };
        let child = OutlineNode {
            id: "SEC-C".into(),
            parent_id: Some("SEC-P".into()),
            title: "子".into(),
            level: 2,
            order: 2,
            requirement_ids: vec!["REQ-0003".into()],
            target_chars: 100,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        };
        s.set_outline(vec![parent, child]).unwrap();
        let err = s
            .append_section_segment("SEC-P", "内容".into(), "k1")
            .unwrap_err();
        assert!(matches!(err, ToolError::EUnprocessable { .. }), "结构节点要响亮拒: {err:?}");
    }

    /// 未知章节 → E_NOTFOUND。
    #[test]
    fn unknown_section_is_not_found() {
        let mut s = store_with_leaf(None);
        let err = s
            .append_section_segment("SEC-9999", "内容".into(), "k1")
            .unwrap_err();
        assert!(matches!(err, ToolError::ENotFound { .. }));
    }

    /// 快照往返：segments 全保真（加性迁移面）。
    #[test]
    fn snapshot_roundtrip_preserves_segments() {
        let mut s = store_with_leaf(None);
        s.append_section_segment("SEC-0001", "甲".into(), "k1").unwrap();
        s.append_section_segment("SEC-0001", "乙".into(), "k2").unwrap();
        let snap = s.to_snapshot();
        assert_eq!(snap.segments.len(), 2);
        let restored = DomainStore::restore(snap).unwrap();
        assert_eq!(
            restored.assemble_section_segments("SEC-0001").unwrap(),
            "甲\n\n乙"
        );
    }

/// #254 验收 4（CAS）：过期 version 重写 → E_CONFLICT 且无部分写入
/// （段内容 / 合稿正文 / content_version 三面全不动）。
#[test]
fn rewrite_with_stale_version_is_conflict_and_atomic() {
    let mut s = store_with_leaf(None);
    s.append_section_segment("SEC-0001", "段一".into(), "k1").unwrap();
    let a = s.append_section_segment("SEC-0001", "段二".into(), "k2").unwrap();
    // 并发写作已把段二推到 version 2
    s.rewrite_section_segment(&a.segment.id, "段二 v2".into(), Some(1)).unwrap();
    // stale 基线（v2 已合法落账）
    let before_content = s.sections.iter().find(|n| n.id == "SEC-0001")
        .unwrap().content.clone().unwrap();
    let before_version = s.project().unwrap().content_version;
    // 拿旧 version 1 再写 → E_CONFLICT
    let err = s
        .rewrite_section_segment(&a.segment.id, "段二 v3（stale）".into(), Some(1))
        .err().expect("过期 version 必须拒");
    assert_eq!(err.code(), "E_CONFLICT", "显式 CAS 拒: {err:?}");
    // 无部分写入：三面全不动
    let seg = s.section_segment(&a.segment.id).unwrap();
    assert_eq!(seg.content, "段二 v2", "段内容不动");
    assert_eq!(seg.version, 2, "version 不动");
    assert_eq!(
        s.sections.iter().find(|n| n.id == "SEC-0001").unwrap().content.as_deref(),
        Some(before_content.as_str()),
        "合稿正文不动"
    );
    assert_eq!(s.project().unwrap().content_version, before_version,
        "content_version 不动（零副作用）");
}

/// 坏快照拒载面补充：(section, idem_key) 重复 = 幂等面破坏，拒载；
/// SEG id 水位低于账本 = 重启后 id 重签碰撞，拒载。
#[test]
fn restore_rejects_idem_dup_and_id_watermark_regression() {
    // idem_key 重复（同章节两段同 key——重放将命中错误段）
    let mut s = store_with_leaf(None);
    s.append_section_segment("SEC-0001", "a".into(), "k1").unwrap();
    s.append_section_segment("SEC-0001", "b".into(), "k2").unwrap();
    let mut snap = s.to_snapshot();
    snap.segments[1].idem_key = "k1".into();
    let err = DomainStore::restore(snap).err().expect("idem 重复必须拒载");
    assert_eq!(err.code(), "E_UNPROCESSABLE", "{err:?}");

    // SEG id 水位：id_seq 低于账本最大 SEG 后缀 → 拒
    let mut s2 = store_with_leaf(None);
    s2.append_section_segment("SEC-0001", "a".into(), "k1").unwrap();
    let mut snap = s2.to_snapshot();
    snap.id_seq = 0;
    let err = DomainStore::restore(snap).err().expect("水位破坏必须拒载");
    assert_eq!(err.code(), "E_UNPROCESSABLE", "{err:?}");
}
}
