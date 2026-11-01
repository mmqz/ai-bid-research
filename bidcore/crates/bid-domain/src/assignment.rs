//! assignment.rs —— T-6 Assignment 任务分工（issue #11 批次 v3 / V3-4）。
//!
//! 定位：章节/条款 → 执行人/截止日的**调度账本**——S-8 并发调度的前置实体。
//! 没有分工面，「500 节点章节树批量派活 + 按人盘点」只能靠会话内存里的口头
//! 约定，断点续跑（S-2）与人工进度对账（S-8）都无从落地。
//!
//! 幂等键 `target_id`（重指派语义）：本实体**无独立 id**——目标本身
//! （SEC-xxxx 章节 / REQ-xxxx 条款）就是自然键，`batch_assign` 按 target_id
//! upsert：同 owner 重批 = 只改截止日（进度轴不动——反复对齐排期是正常操作，
//! 不该清进度）；换 owner 重批 = 状态复位 ASSIGNED（**换人 = 新进度轴**：
//! 旧执行人的半程对新执行人毫无意义，静默继承只会说谎）。
//!
//! 与 SectionStatus 的关系（两轴独立）：`TaskStatus` 是**人的进度**，
//! `SectionStatus` 是**内容的进度**——同一章节完全可以「人已 DONE 但内容
//! 还在 VALIDATING」（交稿 ≠ 过检），反之亦然。两轴互不推导、互不隐式联动；
//! 聚合视图（谁拖了哪章）由上层 S-8 盘点面显式 join，本 crate 不越权代劳。

use crate::{DomainStore, ToolError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------- 状态机
/// 任务状态（T-6 规格：分配后的人侧进度——线性链，越级拒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskStatus {
    Assigned,
    InProgress,
    Done,
}

impl TaskStatus {
    /// 合法前向链（T-6 规格：ASSIGNED → IN_PROGRESS → DONE 的线性推进）。
    pub fn next_allowed(&self) -> &'static [TaskStatus] {
        use TaskStatus::*;
        match self {
            Assigned => &[InProgress],
            InProgress => &[Done],
            // 终态——重开走 batch_assign 重指派显式复位，无静默回退通道
            // （镜像 reopen_section 的 R-06 纪律：离开终态必须显式）。
            Done => &[],
        }
    }

    pub fn as_str(&self) -> &'static str {
        use TaskStatus::*;
        match self {
            Assigned => "ASSIGNED",
            InProgress => "IN_PROGRESS",
            Done => "DONE",
        }
    }
}

// ---------------------------------------------------------------- 实体
/// 任务分工记录（T-6 规格 struct 的 Rust 落地——issue #11 批次 v3）。
///
/// `target_id` 即自然键：SEC-xxxx / REQ-xxxx（无独立 id 字段——幂等按
/// target_id upsert，重指派语义见模块头）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    /// 分工目标（SEC-xxxx 章节 / REQ-xxxx 条款——batch_assign 幽灵门的对象）
    pub target_id: String,
    /// 执行人（人侧标识——与 T-7 的 `human:` 前缀复审人区分，本批不强制前缀）
    pub owner: String,
    /// 截止日（ISO `YYYY-MM-DD`——写入经 `crate::dates::is_valid_ymd` 单一真源
    /// 校验；None = 未定期——诚实状态：没定就是没定，不用魔法日期垫底）
    pub deadline: Option<String>,
    /// 人侧进度（与 SectionStatus 的内容进度两轴独立——见模块头）
    pub status: TaskStatus,
}

// ---------------------------------------------------------------- DomainStore 扩面（T-6 任务分工操作）
impl DomainStore {
    /// 任务分工台账只读面。
    pub fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }

    /// 批量分配（T-6 验收主入口：「章节树 500 节点可批量分配」）。
    ///
    /// 返回 `(总任务数, 本批新增数)`——新增数是重指派对账依据（同 owner 重批
    /// 恒为 0 新增，调用方以此区分「派活」与「改期」）。
    ///
    /// 校验面（全部通过才动手——任何一条命中即整批拒绝、零残留，镜像
    /// set_outline 的 all-or-nothing 纪律）：
    /// - targets 非空（空批 → E_UNPROCESSABLE——「分配零个目标」是调用方
    ///   逻辑坏死，静默成功只会掩盖）；
    /// - owner 非空（调度账本不容匿名行）；
    /// - deadline 若 Some → `crate::dates::is_valid_ymd`（dates.rs 的单一真源
    ///   ——本模块不自带第二套日期口径）；
    /// - 幽灵拒绝：`SEC-` 前缀必须存在于章节树、`REQ-` 前缀必须存在于条款集
    ///   （照 set_outline 的幽灵 REQ 模式——挂空目标的任务是幽灵调度单元）；
    ///   其他前缀放行（未来实体面：DOC-* 等，由各自实体落地时补存在性门）。
    ///
    /// upsert 语义（幂等键 target_id）：
    /// - 已存在且同 owner → 只更新 deadline，状态不动（改期不清进度）；
    /// - 已存在且换 owner → 覆盖 owner/deadline 且状态复位 ASSIGNED
    ///   （换人 = 新进度轴——见模块头）；
    /// - 新 target → 追加 status=ASSIGNED。
    pub fn batch_assign(
        &mut self,
        targets: &[String],
        owner: &str,
        deadline: Option<&str>,
    ) -> Result<(usize, usize), ToolError> {
        // 项目存在性：分工是项目内数据（restore 对「有数据无项目」fail-closed，
        // 这里不先拦就会造出 store 自己都还原不了的快照——不变量自洽）。
        self.project()?;
        if targets.is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "空批量分配（targets 为空——镜像 set_outline 空树纪律）".into(),
            });
        }
        if owner.trim().is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "owner 为空（任务必须落到一个具名执行人——调度账本不容匿名行）".into(),
            });
        }
        if let Some(d) = deadline {
            if !crate::dates::is_valid_ymd(d) {
                return Err(ToolError::EUnprocessable {
                    reason: format!("deadline={d} 非 ISO YYYY-MM-DD（dates::is_valid_ymd 口径）"),
                });
            }
        }
        // 幽灵拒绝（校验面先行——整批任一命中即拒，不产生半批分配）
        for t in targets {
            if t.starts_with("SEC-") {
                if !self.sections.iter().any(|n| n.id == *t) {
                    return Err(ToolError::EUnprocessable {
                        reason: format!("target {t} 不存在于章节树（幽灵调度单元）"),
                    });
                }
            } else if t.starts_with("REQ-") {
                if !self.requirements.iter().any(|r| r.id == *t) {
                    return Err(ToolError::EUnprocessable {
                        reason: format!("target {t} 不存在于条款集（幽灵调度单元）"),
                    });
                }
            }
            // 其他前缀放行——未来实体面（DOC-* 等），各自实体落地时补存在性门
        }
        // 变更面：HashMap 位置索引做 O(targets + assignments) upsert，不逐条
        // 线性扫描（镜像 set_requirements 的 PR #24 治本纪律——批量大时
        // O(N²) 是 CI 卡死的老根子）。同批内重复 target 自然落 upsert 分支，
        // added 只计一次。
        let mut index: HashMap<String, usize> = self
            .assignments
            .iter()
            .enumerate()
            .map(|(i, a)| (a.target_id.clone(), i))
            .collect();
        let mut added = 0usize;
        for t in targets {
            match index.get(t.as_str()) {
                Some(&i) => {
                    let a = &mut self.assignments[i];
                    if a.owner == owner {
                        a.deadline = deadline.map(|d| d.to_string()); // 改期不清进度
                    } else {
                        // 换人 = 新进度轴：owner/deadline 覆盖 + 状态复位 ASSIGNED
                        a.owner = owner.to_string();
                        a.deadline = deadline.map(|d| d.to_string());
                        a.status = TaskStatus::Assigned;
                    }
                }
                None => {
                    index.insert(t.clone(), self.assignments.len());
                    self.assignments.push(Assignment {
                        target_id: t.clone(),
                        owner: owner.to_string(),
                        deadline: deadline.map(|d| d.to_string()),
                        status: TaskStatus::Assigned,
                    });
                    added += 1;
                }
            }
        }
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok((self.assignments.len(), added))
    }

    /// 任务清单（T-6 验收面：「list_tasks 按 owner/status 过滤」）。
    ///
    /// 双过滤（None = 不过滤该维）；结果**按 target_id 字典序**——确定性输出
    /// （BTree 语义）：同参数两次调用全等，R-15 可重放口径的对账前提。
    pub fn list_tasks(&self, owner: Option<&str>, status: Option<TaskStatus>) -> Vec<&Assignment> {
        let mut out: Vec<&Assignment> = self
            .assignments
            .iter()
            .filter(|a| owner.map_or(true, |o| a.owner == o))
            .filter(|a| status.map_or(true, |st| a.status == st))
            .collect();
        out.sort_by(|a, b| a.target_id.cmp(&b.target_id));
        out
    }

    /// 任务状态推进（TaskStatus 线性链 + 越级拒——照 advance_section_status
    /// 的消息格式：合法前向列在错误里，调用方不用翻源码）。
    /// 拒绝路径零副作用（content_version 不动）。
    pub fn advance_task(&mut self, target_id: &str, to: TaskStatus) -> Result<(), ToolError> {
        let idx = self
            .assignments
            .iter()
            .position(|a| a.target_id == target_id)
            .ok_or_else(|| ToolError::ENotFound {
                what: format!("assignment target {target_id} 不存在"),
            })?;
        let cur = self.assignments[idx].status;
        if !cur.next_allowed().contains(&to) {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "任务状态 {} 不允许推进到 {}（合法前向: {:?}）",
                    cur.as_str(),
                    to.as_str(),
                    cur.next_allowed()
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                ),
            });
        }
        self.assignments[idx].status = to;
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::{OutlineNode, SectionStatus};
    use crate::{Anchor, ParseStatus, Requirement, StoreSnapshot};

    /// 任务分工基座：项目 + 文件 + N 条唯一锚 REQ + N 节点平铺章节树
    /// （每节点绑 1 REQ——照 outline.rs 尾部夹具模式；T-6 验收面
    /// 「500 节点章节树」的最小可复用形态）。
    fn store_with_flat_tree(n: usize) -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("测试项目", "ZFCG-2026-GK-0311", "明标", "ext-key-t6")
            .unwrap();
        s.upsert_file(
            "A-招标文件.docx",
            "招标文件",
            "sha-t6",
            31,
            12,
            120,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
        let categories = [
            "资格条件",
            "技术参数",
            "评分项",
            "商务要求",
            "格式要求",
            "其他",
        ];
        let reqs: Vec<Requirement> = (0..n)
            .map(|i| Requirement {
                id: format!("REQ-{:04}", i + 1),
                category: categories[i % categories.len()].into(),
                clause_no: Some(format!("3.{}", i + 1)),
                anchors: vec![Anchor {
                    file: "A".into(),
                    kind: "para".into(),
                    locator: "第三章".into(),
                    excerpt: format!("条款-{i} 的原文摘录"), // 唯一锚——dedupe_hash 不互撞
                }],
                params: std::collections::HashMap::new(),
                source_file: "A".into(),
            })
            .collect();
        s.set_requirements(reqs).unwrap();
        // 平铺树：N 个 level-1 根各绑 1 REQ（set_outline 门：树不变量 + 幽灵 REQ 把关）
        let nodes: Vec<OutlineNode> = (0..n)
            .map(|i| OutlineNode {
                id: format!("SEC-{:04}", i + 1),
                parent_id: None,
                title: format!("验收平铺章-{i}"),
                level: 1,
                order: (i + 1) as u32,
                requirement_ids: vec![format!("REQ-{:04}", i + 1)],
                target_chars: 2000,
                status: SectionStatus::Pending,
                dirty: false,
                content: None,
                content_attempt: 0,
            })
            .collect();
        s.set_outline(nodes).unwrap();
        s
    }

    // ============================ 自研测试 ============================

    /// T-6 规格硬验收：「章节树 500 节点可批量分配，list_tasks 按 owner/status 过滤」。
    #[test]
    fn acceptance_500_nodes_batch_assign_and_filter() {
        let mut s = store_with_flat_tree(500);
        // 整树一批派给同一执行人（带截止日）
        let targets: Vec<String> = (1..=500).map(|i| format!("SEC-{i:04}")).collect();
        let (total, added) = s
            .batch_assign(&targets, "writer:zhang", Some("2026-11-30"))
            .unwrap();
        assert_eq!((total, added), (500, 500), "500 节点一批到位");
        assert_eq!(
            s.list_tasks(Some("writer:zhang"), None).len(),
            500,
            "按 owner 过滤全量"
        );
        assert_eq!(
            s.list_tasks(Some("writer:li"), None).len(),
            0,
            "他人名下为空（不串人）"
        );
        // 前 200 个推进 IN_PROGRESS——人侧进度与章节内容状态两轴互不相扰
        for t in &targets[..200] {
            s.advance_task(t, TaskStatus::InProgress).unwrap();
        }
        assert_eq!(s.list_tasks(None, Some(TaskStatus::Assigned)).len(), 300);
        assert_eq!(s.list_tasks(None, Some(TaskStatus::InProgress)).len(), 200);
        // 组合过滤（owner × status 双维交集）
        assert_eq!(
            s.list_tasks(Some("writer:zhang"), Some(TaskStatus::Assigned))
                .len(),
            300
        );
    }

    /// 幂等重批（同 owner）：零新增、已推进状态不回 ASSIGNED、deadline 更新生效。
    #[test]
    fn idempotent_reassign_same_owner_keeps_status() {
        let mut s = store_with_flat_tree(2);
        let targets: Vec<String> = (1..=2).map(|i| format!("SEC-{i:04}")).collect();
        s.batch_assign(&targets, "writer:zhang", Some("2026-11-30"))
            .unwrap();
        s.advance_task("SEC-0001", TaskStatus::InProgress).unwrap();
        // 同 owner 重批同 targets——语义是对齐排期（改期），不是重新派活
        let (total, added) = s
            .batch_assign(&targets, "writer:zhang", Some("2026-12-15"))
            .unwrap();
        assert_eq!((total, added), (2, 0), "同 owner 重批零新增");
        let t1 = s
            .list_tasks(None, None)
            .into_iter()
            .find(|a| a.target_id == "SEC-0001")
            .unwrap();
        assert_eq!(t1.status, TaskStatus::InProgress, "已推进状态不被改期清回");
        assert_eq!(
            t1.deadline.as_deref(),
            Some("2026-12-15"),
            "deadline 更新生效"
        );
        assert_eq!(
            s.list_tasks(None, Some(TaskStatus::Assigned)).len(),
            1,
            "仅 SEC-0002 仍 ASSIGNED"
        );
    }

    /// 重指派（换 owner）：状态复位 ASSIGNED——换人 = 新进度轴（旧执行人的
    /// 半程对新执行人毫无意义，静默继承只会说谎）。
    #[test]
    fn reassign_different_owner_resets_status() {
        let mut s = store_with_flat_tree(2);
        s.batch_assign(
            &["SEC-0001".into(), "SEC-0002".into()],
            "writer:zhang",
            Some("2026-11-30"),
        )
        .unwrap();
        s.advance_task("SEC-0001", TaskStatus::InProgress).unwrap();
        // 只把 SEC-0001 改派给 writer:li——单目标重指派，台账不增行
        let (total, added) = s
            .batch_assign(&["SEC-0001".into()], "writer:li", None)
            .unwrap();
        assert_eq!((total, added), (2, 0), "换人重批零新增（upsert 同一行）");
        let li = s.list_tasks(Some("writer:li"), None);
        assert_eq!(li.len(), 1);
        assert_eq!(li[0].target_id, "SEC-0001");
        assert_eq!(
            li[0].status,
            TaskStatus::Assigned,
            "换人复位 ASSIGNED（新进度轴）"
        );
        assert_eq!(li[0].deadline, None, "换人时 deadline 一并被新参数覆盖");
        // 旧 owner 名下只剩未改派的 SEC-0002（其进度轴不受牵连）
        let zhang = s.list_tasks(Some("writer:zhang"), None);
        assert_eq!(zhang.len(), 1);
        assert_eq!(zhang[0].target_id, "SEC-0002");
        assert_eq!(zhang[0].status, TaskStatus::Assigned);
    }

    /// 幽灵目标拒绝（零残留）+ 未知前缀放行（未来实体面）。
    #[test]
    fn ghost_targets_rejected_zero_residue() {
        let mut s = store_with_flat_tree(2);
        let v = s.content_version();
        // SEC-9999 不在章节树——幽灵调度单元
        let err = s
            .batch_assign(&["SEC-9999".into()], "writer:zhang", None)
            .unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "幽灵 SEC 拒");
        // REQ-9999 不在条款集——同门
        let err = s
            .batch_assign(&["REQ-9999".into()], "writer:zhang", None)
            .unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "幽灵 REQ 拒");
        assert_eq!(s.assignments().len(), 0, "拒绝路径零残留");
        assert_eq!(s.content_version(), v, "拒绝不 bump");
        // 未知前缀放行（DOC-*——未来实体面，由各自实体落地时补存在性门）
        let (total, added) = s
            .batch_assign(&["DOC-1".into()], "writer:zhang", None)
            .unwrap();
        assert_eq!((total, added), (1, 1), "DOC- 前缀放行");
        // 对照面：在位 REQ/SEC 真目标必须能派
        let (total, added) = s
            .batch_assign(
                &["REQ-0001".into(), "SEC-0001".into()],
                "writer:zhang",
                None,
            )
            .unwrap();
        assert_eq!((total, added), (3, 2));
    }

    /// 参数拒绝面：非 ISO 截止日 / 空批 / 空 owner——全部零副作用。
    #[test]
    fn bad_deadline_and_empty_batch_rejected() {
        let mut s = store_with_flat_tree(1);
        let v = s.content_version();
        // 非 ISO（斜杠形态——dates::is_valid_ymd 定长连字符口径拒收）
        let err = s
            .batch_assign(&["SEC-0001".into()], "writer:zhang", Some("2026/11/30"))
            .unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "非 ISO deadline 拒");
        // 空批（镜像 set_outline 空树纪律）
        let err = s.batch_assign(&[], "writer:zhang", None).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "空 targets 拒");
        // 空 owner（含纯空白——调度账本不容匿名行）
        let err = s
            .batch_assign(&["SEC-0001".into()], "  ", None)
            .unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "空 owner 拒");
        assert_eq!(s.assignments().len(), 0, "拒绝路径零残留");
        assert_eq!(s.content_version(), v, "拒绝不 bump");
        // 对照面：合法形态放行
        assert!(s
            .batch_assign(&["SEC-0001".into()], "writer:zhang", Some("2026-11-30"))
            .is_ok());
    }

    /// 任务状态链与门：越级拒（零副作用）、线性链通过、终态拒、不存在 E_NOTFOUND。
    #[test]
    fn advance_task_chain_and_gates() {
        // 链形态（T-6 规格）
        assert_eq!(
            TaskStatus::Assigned.next_allowed(),
            &[TaskStatus::InProgress]
        );
        assert_eq!(TaskStatus::InProgress.next_allowed(), &[TaskStatus::Done]);
        assert_eq!(
            TaskStatus::Done.next_allowed(),
            &[],
            "终态——重开走 batch_assign 显式复位"
        );
        assert_eq!(TaskStatus::Assigned.as_str(), "ASSIGNED");
        assert_eq!(TaskStatus::InProgress.as_str(), "IN_PROGRESS");
        assert_eq!(TaskStatus::Done.as_str(), "DONE");

        let mut s = store_with_flat_tree(1);
        s.batch_assign(&["SEC-0001".into()], "writer:zhang", None)
            .unwrap();
        let v = s.content_version();
        // 越级：ASSIGNED → DONE 拒（合法前向必须列在错误消息里）
        let err = s.advance_task("SEC-0001", TaskStatus::Done).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "越级拒");
        assert!(
            matches!(&err, ToolError::EUnprocessable { reason } if reason.contains("IN_PROGRESS")),
            "错误消息列出合法前向"
        );
        assert_eq!(s.content_version(), v, "拒绝零副作用");
        // 线性链通过
        s.advance_task("SEC-0001", TaskStatus::InProgress).unwrap();
        s.advance_task("SEC-0001", TaskStatus::Done).unwrap();
        assert_eq!(s.list_tasks(None, Some(TaskStatus::Done)).len(), 1);
        // 终态再推进拒（离开终态唯一通道 = batch_assign 重指派显式复位）
        let err = s
            .advance_task("SEC-0001", TaskStatus::InProgress)
            .unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "终态拒");
        // 不存在目标
        assert!(
            matches!(
                s.advance_task("SEC-9999", TaskStatus::InProgress),
                Err(ToolError::ENotFound { .. })
            ),
            "不存在 E_NOTFOUND"
        );
    }

    /// list_tasks 确定性：乱序插入后按 target_id 字典序，同参数两次调用全等。
    #[test]
    fn list_tasks_deterministic_order() {
        let mut s = store_with_flat_tree(3);
        // 乱序混派（含 REQ/DOC 跨类目标——字典序跨前缀也确定）
        let targets: Vec<String> = ["SEC-0003", "REQ-0002", "DOC-9", "SEC-0001", "SEC-0002"]
            .iter()
            .map(|t| t.to_string())
            .collect();
        s.batch_assign(&targets, "writer:zhang", None).unwrap();
        let l1 = s.list_tasks(None, None);
        let ids: Vec<&str> = l1.iter().map(|a| a.target_id.as_str()).collect();
        assert_eq!(
            ids,
            ["DOC-9", "REQ-0002", "SEC-0001", "SEC-0002", "SEC-0003"],
            "结果按 target_id 字典序（BTree 语义——R-15 可重放口径）"
        );
        // 同参数两次调用全等（确定性——对账前提）
        let l2 = s.list_tasks(None, None);
        assert_eq!(l1, l2, "同参数两次调用全等");
        // owner 过滤维不破坏序
        let owned: Vec<&str> = s
            .list_tasks(Some("writer:zhang"), None)
            .iter()
            .map(|a| a.target_id.as_str())
            .collect();
        assert_eq!(
            owned,
            ["DOC-9", "REQ-0002", "SEC-0001", "SEC-0002", "SEC-0003"]
        );
    }

    // ============================ 借鉴测试 ============================

    /// 快照往返：assignments 全量保真 + 重开后 list_tasks/advance 续用不碰撞
    /// （borrow：mdcore `workbench/tests/persistence_recovery.rs::
    /// session_reopen_state_and_continuity` 的重开保真模式——磁盘 IO 住
    /// bid-tools，这里验 DTO 的 serde 往返形态）。
    #[test]
    fn snapshot_roundtrip_assignments_survive_restart() {
        // 夹具纪律：restore 的 id_seq 水位门要求手造 SEC 后缀 ≤ 已签 id 数
        //（本夹具 2 个：PRJ-0001 + TF-0002）——生产面 SEC 由 next_id 签发恒
        // 满足；这里取 2 节点树（SEC-0001/SEC-0002）保持快照可还原。
        let mut s = store_with_flat_tree(2);
        s.batch_assign(
            &["SEC-0001".into(), "SEC-0002".into(), "REQ-0002".into()],
            "writer:zhang",
            Some("2026-11-30"),
        )
        .unwrap();
        s.advance_task("SEC-0001", TaskStatus::InProgress).unwrap();
        let before = s.to_snapshot();
        // JSON 往返（模拟落盘再读——serde 形态是跨进程续跑的契约面）
        let json = serde_json::to_string(&before).unwrap();
        let mut restored =
            DomainStore::restore(serde_json::from_str::<StoreSnapshot>(&json).unwrap()).unwrap();
        // 重开后全量保真（台账 + 推进态 + 截止日）
        assert_eq!(restored.to_snapshot(), before, "快照往返全等");
        let l = restored.list_tasks(Some("writer:zhang"), None);
        assert_eq!(l.len(), 3);
        let t1 = l.iter().find(|a| a.target_id == "SEC-0001").unwrap();
        assert_eq!(t1.status, TaskStatus::InProgress, "推进态跨重开保真");
        assert_eq!(t1.deadline.as_deref(), Some("2026-11-30"), "截止日保真");
        // 续用不碰撞：重开后的推进从恢复态接着走（而非从头重来）
        restored.advance_task("SEC-0001", TaskStatus::Done).unwrap();
        restored
            .advance_task("SEC-0002", TaskStatus::InProgress)
            .unwrap();
        assert_eq!(restored.list_tasks(None, Some(TaskStatus::Done)).len(), 1);
        assert_eq!(
            restored
                .list_tasks(None, Some(TaskStatus::InProgress))
                .len(),
            1
        );
        // 重指派跨重开也续用（target_id 自然键不因重启漂移）
        let (total, added) = restored
            .batch_assign(&["SEC-0001".into()], "writer:li", None)
            .unwrap();
        assert_eq!((total, added), (3, 0), "重开后的 upsert 幂等面不碰撞");
    }

    /// restore 拒重复 target_id（手工构造损坏快照——绕过 batch_assign 的幂等
    /// upsert；borrow：mdcore `workbench/tests/persistence_recovery.rs` 的
    /// fail-closed 模式——损坏数据响亮拒绝，不猜测加载）。
    #[test]
    fn restore_rejects_duplicate_targets() {
        let mut s = store_with_flat_tree(2);
        s.batch_assign(
            &["SEC-0001".into(), "SEC-0002".into()],
            "writer:zhang",
            None,
        )
        .unwrap();
        let mut snap = s.to_snapshot();
        // 注入第二条同 target_id（自然键破坏——模拟磁盘损坏/手改快照）
        snap.assignments.push(snap.assignments[0].clone());
        // restore 返回的 Ok 边是 DomainStore（无 Debug——unwrap_err 不可用），
        // 用 match 显式取错并保住「必须被拒」的断言语义
        let err = match DomainStore::restore(snap) {
            Err(e) => e,
            Ok(_) => panic!("重复 target_id 的快照必须被拒（fail-closed 不猜测）"),
        };
        assert_eq!(err.code(), "E_UNPROCESSABLE");
        assert!(
            matches!(&err, ToolError::EUnprocessable { reason } if reason.contains("target_id")),
            "错误指名真键 target_id（本实体无独立 id）"
        );
    }
}
