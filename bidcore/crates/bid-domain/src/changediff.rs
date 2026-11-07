//! changediff.rs —— 多轮改标对比（#320）：公告版本链 + 累积 diff + 最终态报告。
//!
//! ## 与 impact.rs 的分工（高内聚、低耦合）
//!
//! ```text
//! impact.rs     回答「这一轮改了什么、项目哪些面被弄脏」（单轮影响面）
//! changediff.rs 回答「N 轮下来，每条要求的最终状态与变更轨迹」（跨轮累积对比）
//! ```
//!
//! 两模块只共享 ReqV2 快照形态（[`snapshot_from_reqs`] 适配）——
//! diff 语义不依赖引擎内部状态，M2+ 补遗解析器落地后可直接由解析产物供给快照。
//!
//! ## 数据形态
//!
//! 按轮快照：`clause_id → params`（BTreeMap——输出顺序确定，报告可复核）。
//! 链节点 = 一轮公告（招标/更正/中标），按日期升序排列，轮号 1..=N 由
//! 日期序决定（乱序 push 自动重排——真实公告抓取顺序不保证时间序）。
//!
//! ## diff 语义（V_from → V_to）
//!
//! ```text
//! Modified  双侧在场且 params 有键差异（逐键 old/new + 变更轮轨迹）
//! Added     仅 to 侧在场（首次出现轮）
//! Deleted   仅 from 侧在场（消失轮）
//! 不变      双侧同值——不入 diff（GT「不变陷阱」口径：明示不变的节点
//!           被动即为失实，见 cuc-028 GT-M17-14）
//! ```
//!
//! ## 幂等口径
//!
//! 与引擎 `imported`（fname→sha）对齐：同 sha 的轮重复 push → 拒绝（Err）。
//!
//! 验证状态：#320 验收 1-3 由 tests/changediff_chain.rs 合成链覆盖，
//! 验收 4（真实多轮案例）由同文件 real_cuc028 三节点链覆盖
//! （数据 = real-corpus/gov-procurement cuc-028 真实公告日期与 GT 变更值）。

use std::collections::BTreeMap;

use serde_json::Value;

use crate::dates::is_valid_ymd;
use crate::impact::{ImpactEngine, ReqV2};

/// 公告版本种类（链节点语义——真实链 cuc-028 的三类节点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionKind {
    /// 招标公告（链首——初始要求集）
    Tender,
    /// 更正公告（变更轮——diff 的主来源）
    Correction,
    /// 中标公告（链尾——通常无要求变更，但占版本链位）
    Award,
}

impl VersionKind {
    /// 稳定字面量（报告/JSON 面）。
    pub fn as_str(&self) -> &'static str {
        match self {
            VersionKind::Tender => "tender",
            VersionKind::Correction => "correction",
            VersionKind::Award => "award",
        }
    }
}

/// 一轮公告版本（版本链节点）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnounceVersion {
    /// 轮号（1 起，日期序）。
    pub round: u32,
    /// 公告日期（YYYY-MM-DD——与仓内 ISO 字典序==时间序口径一致）。
    pub date: String,
    /// 来源标识（公告文件名 / URL slug）。
    pub source: String,
    /// 内容指纹（幂等去重——与引擎 imported 的 sha 口径一致）。
    pub sha256: String,
    /// 公告种类。
    pub kind: VersionKind,
}

/// 一轮的条款快照：`clause_id → params`（键值均有序——输出确定）。
pub type ClauseSnapshot = BTreeMap<String, BTreeMap<String, Value>>;

/// 从引擎要求集拍快照（适配面——changediff 不依赖引擎内部，只消费 pub 数据）。
pub fn snapshot_from_reqs(reqs: &std::collections::HashMap<String, ReqV2>) -> ClauseSnapshot {
    reqs.iter()
        .map(|(id, r)| {
            (
                id.clone(),
                r.params
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        })
        .collect()
}

/// 版本链错误面（拒绝原因可追溯——不静默吞）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    /// 日期非法（委托单一真源 [`crate::dates::is_valid_ymd`] 裁定；
    /// 月长面随 #381（#375 修复）合入自动变严——不在此重复实现）。
    BadDate(String),
    /// 同 sha 轮已存在（幂等拒绝，与引擎 imported 口径一致）。
    DuplicateSha(String),
    /// 轮号不存在（链长度不足）。
    NoSuchRound(u32),
    /// from >= to（diff 区间退化）。
    EmptyRange { from: u32, to: u32 },
    /// 空链（无轮可 diff / 无快照可报）。
    EmptyChain,
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainError::BadDate(d) => write!(f, "公告日期非法（需 YYYY-MM-DD 且月长合法）: {d}"),
            ChainError::DuplicateSha(s) => write!(f, "同内容轮已存在（sha 重复，幂等拒绝）: {s}"),
            ChainError::NoSuchRound(r) => write!(f, "轮号不存在（链长不足）: {r}"),
            ChainError::EmptyRange { from, to } => {
                write!(f, "diff 区间退化（from={from} >= to={to}）")
            }
            ChainError::EmptyChain => write!(f, "版本链为空"),
        }
    }
}

impl std::error::Error for ChainError {}

struct ChainNode {
    version: AnnounceVersion,
    snapshot: ClauseSnapshot,
}

// Debug 手写：快照可能很大，只报轮号与元信息（避免整表 dump 淹没日志）。
impl std::fmt::Debug for ChainNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainNode")
            .field("version", &self.version)
            .field("snapshot_clauses", &self.snapshot.len())
            .finish()
    }
}

/// 公告版本链：一个项目关联 N 轮公告，按日期升序管理。
///
/// 轮号由日期序决定：乱序 push 自动插入重排（真实抓取顺序不保证时间序），
/// 因此轮号只在 push 之后查询才有意义——链自身不存外部轮号引用。
#[derive(Debug, Default)]
pub struct VersionChain {
    /// 恒按 date 升序（同日按 push 顺序——公告同日多发的保序口径）。
    nodes: Vec<ChainNode>,
}

/// 逐键变更（Modified 的明细）。
#[derive(Debug, Clone, PartialEq)]
pub struct KeyChange {
    pub key: String,
    /// 变更前值（Added 时为 None）。
    pub old: Option<Value>,
    /// 变更后值（Deleted 时为 None）。
    pub new: Option<Value>,
}

/// 变更种类（#320 任务②的三分类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
}

impl ChangeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeKind::Modified => "modified",
            ChangeKind::Added => "added",
            ChangeKind::Deleted => "deleted",
        }
    }
}

/// 一条条款的累积变更（含轨迹）。
#[derive(Debug, Clone, PartialEq)]
pub struct ClauseChange {
    pub clause_id: String,
    pub kind: ChangeKind,
    /// 区间内发生变更的全部轮次（升序；Modified 取值变化轮，Added 取首现轮，
    /// Deleted 取消失轮）。
    pub touched_rounds: Vec<u32>,
    /// 最后一次变更所在轮的日期（报告锚点）。
    pub last_touched_date: String,
    /// Modified：逐键 old/new（from→to 净口径）；Added/Deleted 为空表。
    pub keys: Vec<KeyChange>,
}

/// 累积 diff 结果（V_from → V_to 的净变更 + 三分类）。
#[derive(Debug, Clone, PartialEq)]
pub struct CumulativeDiff {
    pub from_round: u32,
    pub to_round: u32,
    pub modified: Vec<ClauseChange>,
    pub added: Vec<ClauseChange>,
    pub deleted: Vec<ClauseChange>,
}

impl CumulativeDiff {
    /// 报告面 JSON（未来 MCP 工具接线的序列化口径，先在测试面钉死形状）。
    pub fn to_json(&self) -> Value {
        fn changes_json(cs: &[ClauseChange]) -> Value {
            Value::Array(
                cs.iter()
                    .map(|c| {
                        serde_json::json!({
                            "clause_id": c.clause_id,
                            "kind": c.kind.as_str(),
                            "touched_rounds": c.touched_rounds,
                            "last_touched_date": c.last_touched_date,
                            "keys": c.keys.iter().map(|k| serde_json::json!({
                                "key": k.key,
                                "old": k.old,
                                "new": k.new,
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect(),
            )
        }
        serde_json::json!({
            "from_round": self.from_round,
            "to_round": self.to_round,
            "modified": changes_json(&self.modified),
            "added": changes_json(&self.added),
            "deleted": changes_json(&self.deleted),
        })
    }
}

/// 最终态：一条要求的最终值 + 变更轨迹（投标人视角——「不用自己脑补」）。
#[derive(Debug, Clone, PartialEq)]
pub struct FinalClauseState {
    pub clause_id: String,
    /// 最新轮的完整参数（最终要求）。
    pub params: BTreeMap<String, Value>,
    /// 首次出现轮（Added 条款的来历）。
    pub origin_round: u32,
    /// 全链内发生变更的轮次（升序，含日期）。
    pub touched_rounds: Vec<(u32, String)>,
}

/// 全链报告（#320 任务③：「共 N 轮改标，累计修改 X 条」+ 逐条最终状态）。
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeReport {
    /// 链内轮数。
    pub rounds: u32,
    /// 更正公告轮数（改标轮）。
    pub correction_rounds: u32,
    /// (首轮日期, 末轮日期)。
    pub date_span: (String, String),
    /// V1 → 最新轮的净变更。
    pub cumulative: CumulativeDiff,
    /// 最新轮在场条款的最终态（被删条款见 cumulative.deleted）。
    pub final_states: Vec<FinalClauseState>,
}

impl ChangeReport {
    /// 摘要行（报告头口径——「共 3 轮改标，累计修改 7 条要求」）。
    pub fn summary_line(&self) -> String {
        format!(
            "共 {} 轮公告（{} 轮更正），累计修改 {} 条 / 新增 {} 条 / 删除 {} 条要求",
            self.rounds,
            self.correction_rounds,
            self.cumulative.modified.len(),
            self.cumulative.added.len(),
            self.cumulative.deleted.len()
        )
    }

    /// 报告面 JSON。
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "rounds": self.rounds,
            "correction_rounds": self.correction_rounds,
            "date_span": [self.date_span.0, self.date_span.1],
            "summary": self.summary_line(),
            "cumulative": self.cumulative.to_json(),
            "final_states": self.final_states.iter().map(|s| serde_json::json!({
                "clause_id": s.clause_id,
                "params": s.params,
                "origin_round": s.origin_round,
                "touched_rounds": s.touched_rounds.iter()
                    .map(|(r, d)| serde_json::json!({"round": r, "date": d}))
                    .collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }
}

impl VersionChain {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// 链内轮数。
    pub fn len(&self) -> u32 {
        self.nodes.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 全部轮（日期升序；轮号 = 序 + 1）。
    pub fn rounds(&self) -> Vec<AnnounceVersion> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| AnnounceVersion {
                round: i as u32 + 1,
                ..n.version.clone()
            })
            .collect()
    }

    /// 某轮快照（轮号 1 起）。
    pub fn snapshot(&self, round: u32) -> Result<&ClauseSnapshot, ChainError> {
        self.node(round).map(|n| &n.snapshot)
    }

    fn node(&self, round: u32) -> Result<&ChainNode, ChainError> {
        if round == 0 || round as usize > self.nodes.len() {
            return Err(ChainError::NoSuchRound(round));
        }
        Ok(&self.nodes[round as usize - 1])
    }

    /// 追加一轮（按日期插入排序，返回分配的轮号）。
    ///
    /// 拒绝面：日期非法 / sha 与既有轮重复（幂等——同内容公告不占两轮）。
    pub fn push_round(
        &mut self,
        date: &str,
        source: &str,
        sha256: &str,
        kind: VersionKind,
        snapshot: ClauseSnapshot,
    ) -> Result<u32, ChainError> {
        if !is_valid_ymd(date) {
            return Err(ChainError::BadDate(date.to_string()));
        }
        if self.nodes.iter().any(|n| n.version.sha256 == sha256) {
            return Err(ChainError::DuplicateSha(sha256.to_string()));
        }
        // 插入位：首个「日期严格大于本轮」的节点之前（同日保序——追加重复场景
        // 按公告同日多发处理，不拒）。
        let pos = self
            .nodes
            .iter()
            .position(|n| n.version.date.as_str() > date)
            .unwrap_or(self.nodes.len());
        let round = pos as u32 + 1;
        self.nodes.insert(
            pos,
            ChainNode {
                version: AnnounceVersion {
                    round,
                    date: date.to_string(),
                    source: source.to_string(),
                    sha256: sha256.to_string(),
                    kind,
                },
                snapshot,
            },
        );
        Ok(round)
    }

    /// 引擎侧适配：一轮 import_events 后，用引擎当前要求集拍快照入链。
    ///
    /// sha 口径与引擎 `imported`（fname→sha）一致——由调用方传同一 sha，
    /// 重复登记在链侧被 DuplicateSha 拒（两道幂等闸互不越权）。
    pub fn push_engine_round(
        &mut self,
        date: &str,
        source: &str,
        sha256: &str,
        kind: VersionKind,
        engine: &ImpactEngine,
    ) -> Result<u32, ChainError> {
        let snapshot = snapshot_from_reqs(engine.reqs());
        self.push_round(date, source, sha256, kind, snapshot)
    }

    /// 累积 diff（V_from → V_to 净变更，三分类 + 轨迹）。
    pub fn cumulative_diff(&self, from: u32, to: u32) -> Result<CumulativeDiff, ChainError> {
        if self.nodes.is_empty() {
            return Err(ChainError::EmptyChain);
        }
        if from >= to {
            return Err(ChainError::EmptyRange { from, to });
        }
        let from_snap = self.snapshot(from)?;
        let to_snap = self.snapshot(to)?;

        let mut modified = Vec::new();
        let mut added = Vec::new();
        let mut deleted = Vec::new();

        // 双侧在场：逐键比 from→to；轨迹 = 区间内快照发生变化的轮。
        for (id, to_params) in to_snap {
            if let Some(from_params) = from_snap.get(id) {
                let keys: Vec<KeyChange> = to_params
                    .iter()
                    .filter_map(|(k, new_v)| {
                        let old_v = from_params.get(k);
                        if old_v == Some(new_v) {
                            None // 键未变
                        } else {
                            Some(KeyChange {
                                key: k.clone(),
                                old: old_v.cloned(),
                                new: Some(new_v.clone()),
                            })
                        }
                    })
                    // from 独有键（to 侧消失）= 参数级删除，一并入 Modified 明细。
                    .chain(from_params.iter().filter_map(|(k, old_v)| {
                        if to_params.contains_key(k) {
                            None
                        } else {
                            Some(KeyChange {
                                key: k.clone(),
                                old: Some(old_v.clone()),
                                new: None,
                            })
                        }
                    }))
                    .collect();
                if keys.is_empty() {
                    continue; // 不变陷阱：双侧同值不入 diff
                }
                let touched = self.touched_rounds_between(id, from, to, |a, b| a != b);
                modified.push(ClauseChange {
                    clause_id: id.clone(),
                    kind: ChangeKind::Modified,
                    touched_rounds: touched.clone(),
                    last_touched_date: self
                        .node(*touched.last().unwrap())
                        .map(|n| n.version.date.clone())
                        .unwrap_or_default(),
                    keys,
                });
            } else {
                // Added：首现轮 = 区间内首个含该条款的轮。
                let first = (from + 1..=to)
                    .find(|&r| self.snapshot(r).map(|s| s.contains_key(id)) == Ok(true))
                    .unwrap_or(to);
                added.push(ClauseChange {
                    clause_id: id.clone(),
                    kind: ChangeKind::Added,
                    touched_rounds: vec![first],
                    last_touched_date: self.node(first)?.version.date.clone(),
                    keys: Vec::new(),
                });
            }
        }
        // from 独有：Deleted——消失轮 = 区间内首个不含该条款的轮。
        for id in from_snap.keys() {
            if !to_snap.contains_key(id) {
                let gone = (from + 1..=to)
                    .find(|&r| self.snapshot(r).map(|s| !s.contains_key(id)) == Ok(true))
                    .unwrap_or(to);
                deleted.push(ClauseChange {
                    clause_id: id.clone(),
                    kind: ChangeKind::Deleted,
                    touched_rounds: vec![gone],
                    last_touched_date: self.node(gone)?.version.date.clone(),
                    keys: Vec::new(),
                });
            }
        }

        Ok(CumulativeDiff {
            from_round: from,
            to_round: to,
            modified,
            added,
            deleted,
        })
    }

    /// 全链报告（V1 → 最新轮）：轮数 / 更正轮数 / 日期跨度 / 净变更 / 逐条最终态。
    pub fn final_report(&self) -> Result<ChangeReport, ChainError> {
        if self.nodes.is_empty() {
            return Err(ChainError::EmptyChain);
        }
        let n = self.len();
        let cumulative = self.cumulative_diff(1, n)?;
        let last_snap = self.snapshot(n)?;

        let final_states = last_snap
            .iter()
            .map(|(id, params)| {
                let origin = (1..=n)
                    .find(|&r| self.snapshot(r).map(|s| s.contains_key(id)) == Ok(true))
                    .unwrap_or(1);
                let touched = self.touched_rounds_between(id, 1, n, |a, b| a != b);
                FinalClauseState {
                    clause_id: id.clone(),
                    params: params.clone(),
                    origin_round: origin,
                    touched_rounds: touched
                        .iter()
                        .filter_map(|&r| {
                            self.node(r).ok().map(|n| (r, n.version.date.clone()))
                        })
                        .collect(),
                }
            })
            .collect();

        Ok(ChangeReport {
            rounds: n,
            correction_rounds: self
                .nodes
                .iter()
                .filter(|n| n.version.kind == VersionKind::Correction)
                .count() as u32,
            date_span: (
                self.nodes.first().unwrap().version.date.clone(),
                self.nodes.last().unwrap().version.date.clone(),
            ),
            cumulative,
            final_states,
        })
    }

    /// 条款在 (from, to] 内发生「谓词判定为变更」的轮次（与前一轮快照比）。
    ///
    /// Modified 轨迹：params 变化轮；Added 首现 / Deleted 消失由调用方单点定位。
    fn touched_rounds_between(
        &self,
        id: &str,
        from: u32,
        to: u32,
        changed: impl Fn(
            &BTreeMap<String, Value>,
            &BTreeMap<String, Value>,
        ) -> bool,
    ) -> Vec<u32> {
        let mut out = Vec::new();
        let mut prev: Option<&BTreeMap<String, Value>> = self.snapshot(from).ok().and_then(|s| s.get(id));
        for r in from + 1..=to {
            if let Ok(snap) = self.snapshot(r) {
                let cur = snap.get(id);
                match (prev, cur) {
                    (Some(p), Some(c)) if changed(p, c) => out.push(r),
                    (None, Some(_)) => out.push(r), // 首现
                    (Some(_), None) => out.push(r), // 消失
                    _ => {}
                }
                prev = cur;
            }
        }
        out
    }
}
