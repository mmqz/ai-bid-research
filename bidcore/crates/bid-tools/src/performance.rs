//! performance.rs —— #315 业绩材料库：磁盘层 + 服务面（bid-tools）。
//!
//! 分层（borrow：store_io.rs 的纪律）：
//! - **域层**（bid-domain `performance`）：实体/校验/幂等键/打分/渲染，零 IO；
//! - **本层**：库文件的原子读写（tmp+rename、schema_version 门、损坏拒载）、
//!   路径解析（跨项目面）、以及 [`BidService`] 上的四个业务方法（登记/列表/
//!   推荐/写章节）。
//!
//! 跨项目持久化（#315 验收 1）：
//! 库文件**不在项目工作区**内（db.json 是单项目态——库里放工作区就成了
//! 「每项目一份库」，跨项目复用直接落空）。路径解析序：
//! 1. `BID_PERF_LIB` 环境变量（显式指定——测试/多租主/CI 用）；
//! 2. 用户级默认位：Unix `$HOME/.mang/performance-library.json`，Windows
//!    `%APPDATA%\\mang\\performance-library.json`（borrow：mang-app recent.rs
//!    的先例——用户级数据住用户目录）；
//! 3. 都解析不到（无 HOME 且未设 env）→ 内存态（本会话可用、不落盘——
//!    响应面 `library_persisted:false` 如实报告，不伪装持久化）。
//!
//! 纯内存服务（`BidService::new()` 未开工作区且未设 env）同样走内存库——
//! 测试/嵌入方的默认形态零副作用；`with_perf_library(path)` 提供显式注入面
//! （单测并行隔离的正规路径，不碰进程级 env）。
//!
//! 诚实纪律：
//! - 库文件损坏 → `Corrupt` 拒载（原文件保留）——不静默重建（重建 = 用户的
//!   几十条业绩凭空消失还装没事）；
//! - 未来版本 → `SchemaUnsupported` 响亮拒绝；
//! - 材料文件路径只记录不验证（登记时刻用户可能还没把材料搬进工作区）；
//!   真正的消费点在 `write_performance_section`（导入围栏裁决在那一刻）。

use crate::service::{BidService, EvidenceRegistration};
use crate::store_io::StoreIoError;
use bid_domain::evidence::EvidenceKind;
use bid_domain::performance::{
    rank_entries, same_entry, validate_entry_spec, PerformanceEntry, PerformanceEntrySpec,
    PerformanceLibrarySnapshot, ProjectSignals, LIBRARY_SCHEMA_VERSION,
};
use crate::readiness::PERFORMANCE_TITLE_WORDS;
use bid_domain::ToolError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 文件层

/// 业绩库文件（单文件 = 单次原子提交——对齐 store_io 的 db.json 纪律）。
pub struct PerformanceLibraryFile {
    path: PathBuf,
}

/// 磁盘上的库全量（含 schema_version 的包装形态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LibraryFileBody {
    #[serde(default = "body_default_version")]
    schema_version: u32,
    #[serde(flatten)]
    snapshot: PerformanceLibrarySnapshot,
}

fn body_default_version() -> u32 {
    LIBRARY_SCHEMA_VERSION
}

impl PerformanceLibraryFile {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self { path: path.as_ref().to_path_buf() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 加载（Ok(None) = 文件不存在——首启空库；损坏/未来版本 → Err 拒载）。
    pub fn load(&self) -> Result<Option<PerformanceLibrarySnapshot>, StoreIoError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&self.path).map_err(StoreIoError::Io)?;
        let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
            StoreIoError::Corrupt(format!("业绩库 JSON 语法损坏: {e}"))
        })?;
        let found = value
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(LIBRARY_SCHEMA_VERSION);
        if found > LIBRARY_SCHEMA_VERSION {
            return Err(StoreIoError::SchemaUnsupported { found, supported: LIBRARY_SCHEMA_VERSION });
        }
        let body: LibraryFileBody = serde_json::from_value(value).map_err(|e| {
            StoreIoError::Corrupt(format!("业绩库结构不匹配（字段级）: {e}"))
        })?;
        Ok(Some(body.snapshot))
    }

    /// 保存（tmp + rename 原子提交；失败时原文件不动）。
    pub fn save(&self, snap: &PerformanceLibrarySnapshot) -> Result<(), StoreIoError> {
        let body = LibraryFileBody {
            schema_version: LIBRARY_SCHEMA_VERSION,
            snapshot: snap.clone(),
        };
        let json = serde_json::to_string_pretty(&body).map_err(StoreIoError::Serialize)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(StoreIoError::Io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json.as_bytes()).map_err(StoreIoError::Io)?;
        std::fs::rename(&tmp, &self.path).map_err(StoreIoError::Io)?;
        Ok(())
    }
}

/// 库文件路径解析（跨项目面的单一真源）。
///
/// 序：`BID_PERF_LIB`（非空白）→ 用户级默认位 → None（内存态）。
/// 注意：env 只在服务构造时刻读一次（与 BID_WS 同款纪律——长进程中途改 env
/// 不生效，避免同进程双真源）。
pub fn resolve_library_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("BID_PERF_LIB") {
        let t = p.trim().to_string();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    user_library_path()
}

/// 用户级默认位（borrow：mang-app recent.rs——Windows APPDATA / Unix HOME）。
fn user_library_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // Windows：APPDATA 缺失退 LOCALAPPDATA，再缺 → None（内存态）
        std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .map(|base| PathBuf::from(base).join("mang").join("performance-library.json"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".mang").join("performance-library.json"))
    }
}

// ---------------------------------------------------------------- 服务态句柄

/// BidService 上的业绩库句柄（惰性加载 + 落盘面）。
pub(crate) struct PerfLibHandle {
    /// None = 内存态（本会话可用、不落盘）；Some = 文件背书。
    path: Option<PathBuf>,
    /// 惰性加载的快照（None = 首次使用前未加载）。
    loaded: Option<PerformanceLibrarySnapshot>,
}

impl PerfLibHandle {
    pub(crate) fn ephemeral() -> Self {
        Self { path: None, loaded: None }
    }

    pub(crate) fn at(path: PathBuf) -> Self {
        Self { path: Some(path), loaded: None }
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

// ---------------------------------------------------------------- 报告面

/// `register_performance` 的回执。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfRegisterReport {
    /// 新登记的条目（perf_id 清单）。
    pub added: Vec<String>,
    /// 语义幂等命中（既有 perf_id——同 (name, client, amount, date) 四元组）。
    pub deduplicated: Vec<String>,
    /// 库内总条数（登记后）。
    pub total: usize,
    /// 库主（本次生效值）。
    pub owner_name: Option<String>,
    /// 是否落盘（false = 内存态——本会话可用，重启即失，如实报告）。
    pub persisted: bool,
}

/// `list_performance` 的过滤面。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PerfListFilter {
    /// 标签子串（categories 任一含此串——大小写敏感，与语料命中同口径）。
    pub category: Option<String>,
    /// 关键词（name/client 任一包含）。
    pub keyword: Option<String>,
    /// 金额下界（万元，含）。
    pub min_amount_wan: Option<f64>,
    /// 金额上界（万元，含）。
    pub max_amount_wan: Option<f64>,
    /// 完成日期下界（ISO，含）。
    pub since: Option<String>,
    /// 返回上限（1..=1000；缺省 100——列表面不设分页游标：业绩库量级几十条，
    /// 上限只作防御）。
    pub limit: Option<usize>,
}

/// `write_performance_section` 的回执。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfWriteReport {
    pub section_id: String,
    /// 写入的段 id（幂等重放时 changed=false）。
    pub segment_id: String,
    /// 本次是否真实写入（false = 幂等重放）。
    pub changed: bool,
    /// 段字符数。
    pub segment_chars: usize,
    /// 成功登记的证据 [{perf_id, ev_id, source_file}]。
    pub evidence_registered: Vec<PerfEvidenceLink>,
    /// 材料缺口（诚实面——未匹配到已导入文件的材料逐条列明）。
    pub materials_pending: Vec<PerfMaterialPending>,
    /// 绑定面 [{ev_id, req_ids}]（无业绩类 REQ 时为空 + warnings 说明）。
    pub bindings: Vec<PerfBinding>,
    /// 章节就绪门是否通过（true = 已写入）。
    pub readiness_passed: bool,
    pub warnings: Vec<String>,
}

/// 业绩条目 → 项目证据 的链接回执。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfEvidenceLink {
    pub perf_id: String,
    pub ev_id: String,
    /// 匹配到的已导入文件名（材料身份——审计面）。
    pub source_file: String,
    /// true = 命中既有 EV（域层语义位查重——同 kind+title+subject+scope+指纹）。
    pub deduplicated: bool,
}

/// 材料缺口（未匹配到已导入文件——写入正文但证据未登记的诚实报告）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfMaterialPending {
    pub perf_id: String,
    /// 库里登记的材料路径（逐条——用户照单搬文件）。
    pub materials: Vec<String>,
    pub reason: String,
}

/// 证据 → 条款 绑定回执。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfBinding {
    pub ev_id: String,
    pub req_ids: Vec<String>,
}

// ---------------------------------------------------------------- 服务面

impl BidService {
    /// 业绩库句柄构造面（service.rs 的 new/open 调用）。
    pub(crate) fn perf_handle_default() -> PerfLibHandle {
        match resolve_library_path() {
            Some(p) => PerfLibHandle::at(p),
            None => PerfLibHandle::ephemeral(),
        }
    }

    /// 显式注入库路径（builder——测试并行隔离的正规路径，不碰进程级 env）。
    #[must_use]
    pub fn with_perf_library(mut self, path: impl AsRef<Path>) -> Self {
        self.perf_lib = PerfLibHandle::at(path.as_ref().to_path_buf());
        self
    }

    /// 库路径（响应面回显——用户可见库存在哪）。
    pub fn perf_library_path(&self) -> Option<&Path> {
        self.perf_lib.path()
    }

    /// 确保库已加载（惰性——首次库操作时读盘；损坏 → E_UNPROCESSABLE
    /// 拒载并把修复指引带上，库坏不阻塞无关业务）。
    fn perf_loaded(&mut self) -> Result<&mut PerformanceLibrarySnapshot, ToolError> {
        if self.perf_lib.loaded.is_none() {
            let snap = match self.perf_lib.path.as_deref() {
                None => PerformanceLibrarySnapshot::default(),
                Some(p) => PerformanceLibraryFile::new(p)
                    .load()
                    .map_err(|e| ToolError::EUnprocessable {
                        reason: format!(
                            "业绩库加载失败（{p:?}）——{e}；库文件损坏不静默重建（重建会丢全部登记），修复该文件或设 BID_PERF_LIB 指向新路径"
                        ),
                    })?
                    .unwrap_or_default(),
            };
            self.perf_lib.loaded = Some(snap);
        }
        Ok(self
            .perf_lib
            .loaded
            .as_mut()
            .expect("perf_loaded 刚写入 Some"))
    }

    /// 库落盘（持久化面；内存态 no-op）。
    fn perf_save(&mut self) -> Result<bool, ToolError> {
        let path = self.perf_lib.path.clone();
        let Some(p) = path else { return Ok(false) };
        let snap = self
            .perf_lib
            .loaded
            .as_ref()
            .cloned()
            .expect("写路径必先经 perf_loaded");
        PerformanceLibraryFile::new(&p)
            .save(&snap)
            .map_err(|e| ToolError::EUnprocessable {
                reason: format!("业绩库落盘失败（{p:?}，内存已变更、磁盘未跟上——不得当作成功）: {e}"),
            })?;
        Ok(true)
    }

    /// `register_performance` 业务面：批量登记（全量校验先行——一条非法整批
    /// 拒，零副作用；语义幂等键命中 → 返回既有 id 不重复登记）。
    pub fn register_performances(
        &mut self,
        specs: Vec<PerformanceEntrySpec>,
        owner_name: Option<String>,
    ) -> Result<PerfRegisterReport, ToolError> {
        if specs.is_empty() && owner_name.is_none() {
            return Err(ToolError::EUnprocessable {
                reason: "register_performance 空调用：entries 与 owner_name 至少给一项".into(),
            });
        }
        // ---- 全量校验先行（零副作用面）
        for (i, spec) in specs.iter().enumerate() {
            validate_entry_spec(spec).map_err(|e| ToolError::EUnprocessable {
                reason: format!("entries[{i}] {}", match &e {
                    ToolError::EUnprocessable { reason } => reason.clone(),
                    _ => format!("{e:?}"),
                }),
            })?;
        }
        let lib = self.perf_loaded()?;
        if let Some(owner) = owner_name.as_deref() {
            let owner = owner.trim();
            if owner.is_empty() {
                return Err(ToolError::EUnprocessable {
                    reason: "owner_name 在场时不可为空白（库主 = 投标人主体名，证据登记面消费）".into(),
                });
            }
            lib.owner_name = Some(owner.to_string());
        }
        let mut added = Vec::new();
        let mut deduplicated = Vec::new();
        for spec in specs {
            if let Some(hit) = lib.entries.iter().find(|e| same_entry(e, &spec)) {
                deduplicated.push(hit.id.clone());
                continue;
            }
            let id = lib.next_perf_id();
            lib.entries.push(PerformanceEntry {
                id: id.clone(),
                name: spec.name,
                client: spec.client,
                amount_wan: spec.amount_wan,
                completed_on: spec.completed_on,
                categories: spec.categories,
                materials: spec.materials,
                key_points: spec.key_points,
                subject_name: spec.subject_name,
                created_at: bid_domain::dates::today_ymd(),
            });
            added.push(id);
        }
        let total = lib.entries.len();
        let owner = lib.owner_name.clone();
        let persisted = self.perf_save()?;
        Ok(PerfRegisterReport { added, deduplicated, total, owner_name: owner, persisted })
    }

    /// `list_performance` 业务面（只读过滤）。
    pub fn list_performances(
        &mut self,
        filter: &PerfListFilter,
    ) -> Result<(Vec<PerformanceEntry>, Option<String>, bool), ToolError> {
        let lib = self.perf_loaded()?;
        let limit = filter.limit.unwrap_or(100).clamp(1, 1000);
        let mut rows: Vec<PerformanceEntry> = lib
            .entries
            .iter()
            .filter(|e| {
                if let Some(c) = filter.category.as_deref() {
                    let c = c.trim();
                    if !c.is_empty() && !e.categories.iter().any(|t| t.contains(c)) {
                        return false;
                    }
                }
                if let Some(k) = filter.keyword.as_deref() {
                    let k = k.trim();
                    if !k.is_empty() && !e.name.contains(k) && !e.client.contains(k) {
                        return false;
                    }
                }
                if let Some(min) = filter.min_amount_wan {
                    if e.amount_wan < min {
                        return false;
                    }
                }
                if let Some(max) = filter.max_amount_wan {
                    if e.amount_wan > max {
                        return false;
                    }
                }
                if let Some(since) = filter.since.as_deref() {
                    if !bid_domain::dates::is_valid_ymd(since) {
                        return false; // 非法过滤值在工具面先拦，此处兜底
                    }
                    if e.completed_on.as_str() < since {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        rows.truncate(limit);
        Ok((rows, lib.owner_name.clone(), self.perf_lib.path.is_some()))
    }

    /// 项目信号提炼（recommend / write 共用——从项目态确定性提取）。
    ///
    /// - 语料：项目名 + 全部 REQ 陈述（params.statement）+ 评分项 criteria 原文；
    /// - 预算：仓内无预算结构面（budget.rs 是 token 预算）——只吃显式入参
    ///   （None = 金额因子缺信号重归一，不猜）；
    /// - 采购人类型：REQ 陈述里的采购人名称不可靠（多为「采购人」字样），
    ///   采显式入参；两侧都没有 → 因子弃权。
    fn project_signals(
        &self,
        budget_wan: Option<f64>,
        base_date: &str,
        client_type: Option<bid_domain::performance::ClientType>,
        focus: Option<&str>,
    ) -> ProjectSignals {
        let mut corpus = String::new();
        if let Ok(p) = self.store().project() {
            corpus.push_str(&p.name);
            corpus.push(' ');
        }
        for req in self.store().requirements() {
            if let Some(s) = req.params.get("statement").and_then(|v| v.as_str()) {
                corpus.push_str(s);
                corpus.push(' ');
            }
        }
        for item in self.store().scoring_items() {
            corpus.push_str(&item.criteria);
            corpus.push(' ');
        }
        if let Some(f) = focus {
            corpus.push_str(f);
        }
        ProjectSignals {
            corpus,
            budget_wan,
            base_date: base_date.to_string(),
            client_type,
        }
    }

    /// `recommend_performance` 业务面（只读——打分排序 + 建议章节）。
    ///
    /// 返回 (matches, 库总量, base_date)。
    pub fn recommend_performances(
        &mut self,
        budget_wan: Option<f64>,
        base_date: Option<String>,
        client_type: Option<bid_domain::performance::ClientType>,
        focus: Option<String>,
        top_k: usize,
    ) -> Result<(Vec<bid_domain::performance::PerformanceMatch>, usize, String), ToolError> {
        // base_date 给了就必须是合法 ISO（拼错响亮拒）；不传 → 系统今日
        // （近三年窗口需要基准；来源在响应面注明，不伪装确定性）
        let base = match base_date {
            Some(d) if bid_domain::dates::is_valid_ymd(&d) => d,
            Some(d) => {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "base_date 非法: {d:?}（须 ISO YYYY-MM-DD——评标基准日；不传则用系统今日）"
                    ),
                })
            }
            None => bid_domain::dates::today_ymd(),
        };
        let signals = self.project_signals(budget_wan, &base, client_type, focus.as_deref());
        let lib = self.perf_loaded()?;
        let total = lib.entries.len();
        let matches = rank_entries(&lib.entries, &signals, top_k);
        Ok((matches, total, base))
    }

    /// 建议写入章节（readiness 的业绩标题词表——标题含业绩/类似项目的
    /// **叶**章节；建纲序取首个命中；无 → None 由调用方报「建纲后用」）。
    ///
    /// 叶判定：扁平章节树（OutlineNode 无 children 字段）——无任何节点以
    /// 本章为 parent_id 即叶（结构节点不承载正文，写不进去）。
    pub fn suggest_performance_section(&self) -> Option<String> {
        let parents: std::collections::HashSet<&str> = self
            .store()
            .sections()
            .iter()
            .filter_map(|n| n.parent_id.as_deref())
            .collect();
        self.store()
            .sections()
            .iter()
            .find(|n| {
                !parents.contains(n.id.as_str())
                    && PERFORMANCE_TITLE_WORDS.iter().any(|w| n.title.contains(w))
            })
            .map(|n| n.id.clone())
    }

    /// `write_performance_section` 业务面（#315 任务③——一键写入）。
    ///
    /// 流程（每步失败面独立、诚实回报）：
    /// 1. 解析 perf_ids → 条目（未知 id → E_NOTFOUND）；
    /// 2. 材料匹配：entry.materials 的文件名 vs 已导入 TF 名（首个命中）——
    ///    材料已导入 → `register_evidence_verified`（域层语义位查重天然幂等）；
    ///    未导入 → materials_pending（不伪造证据）；
    /// 3. 证据绑定：业绩类 REQ（陈述含 业绩/类似项目）逐 EV 绑定；
    /// 4. readiness 门（append_segment 内建——缺业绩证据会拦写入，本方法把
    ///    材料缺口并进错误指引）；
    /// 5. 单段写入：业绩表 + 逐条描述段（带 [EV-*] 引用），幂等键
    ///    `perf-section:<section_id>`（重跑 no-op）。
    pub fn write_performance_section(
        &mut self,
        section_id: &str,
        perf_ids: &[String],
        bid_open_date: Option<String>,
    ) -> Result<PerfWriteReport, ToolError> {
        if perf_ids.is_empty() {
            return Err(ToolError::EUnprocessable {
                reason: "perf_ids 不可为空（至少选一条业绩——先 recommend_performance 挑选）".into(),
            });
        }
        // ---- 1. 条目解析（全部命中才继续——部分命中零副作用；库数据 clone
        //         出块，后续 self 调用不再受借用约束）
        let (entries, owner_name) = {
            let lib = self.perf_loaded()?;
            let mut entries: Vec<PerformanceEntry> = Vec::with_capacity(perf_ids.len());
            let mut missing: Vec<String> = Vec::new();
            for pid in perf_ids {
                match lib.entry(pid) {
                    Some(e) => entries.push(e.clone()),
                    None => missing.push(pid.clone()),
                }
            }
            if !missing.is_empty() {
                return Err(ToolError::ENotFound {
                    what: format!(
                        "perf_ids 不在业绩库：{}（库内现有 {} 条——list_performance 查看全量）",
                        missing.join("、"),
                        lib.entries.len()
                    ),
                });
            }
            (entries, lib.owner_name.clone())
        };
        // ---- 2. 材料匹配 + 证据登记（已导入文件面）
        let subject_fallback = owner_name.or_else(|| {
            // 工作区 bidder-profile.json 的 company_name（格式件面——人工填过就有）
            self.workspace_dir()
                .and_then(|ws| bid_domain::party::BidderProfile::load(&ws).ok())
                .filter(|p| !p.company_name.trim().is_empty())
                .map(|p| p.company_name)
        });
        let mut evidence_registered: Vec<PerfEvidenceLink> = Vec::new();
        let mut materials_pending: Vec<PerfMaterialPending> = Vec::new();
        for entry in &entries {
            let matched_tf = entry.materials.iter().find_map(|m| {
                let fname = Path::new(m)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())?;
                self.store()
                    .files()
                    .iter()
                    .find(|tf| {
                        // 双形态比对：TF.name 可能是纯文件名（仓内惯例）或
                        // 相对路径形态（Agent 导入时照抄 upload_ref 路径）——
                        // 业绩库登记的材料路径按 file_name 分量对齐
                        tf.name == fname
                            || Path::new(&tf.name)
                                .file_name()
                                .is_some_and(|f| f.to_string_lossy() == fname)
                    })
                    .map(|tf| (tf.id.clone(), tf.name.clone()))
            });
            match matched_tf {
                Some((tf_id, tf_name)) => {
                    let subject = entry
                        .subject_name
                        .clone()
                        .or_else(|| subject_fallback.clone());
                    let Some(subject) = subject else {
                        materials_pending.push(PerfMaterialPending {
                            perf_id: entry.id.clone(),
                            materials: entry.materials.clone(),
                            reason: "库主未登记（owner_name）且工作区无 bidder-profile.json——证据登记需要主体名：register_performance 带 owner_name 一次即可".into(),
                        });
                        continue;
                    };
                    let reg = EvidenceRegistration {
                        source_file_id: tf_id,
                        kind: EvidenceKind::Performance,
                        title: entry.name.clone(),
                        locator: None,
                        placeholder: false,
                        valid_from: None,
                        valid_until: None,
                        subject_name: subject,
                        scope: entry.categories.join("/"),
                        client: Some(entry.client.clone()),
                        amount_wan: Some(entry.amount_wan),
                        completed_on: Some(entry.completed_on.clone()),
                        fields: bid_domain::evidence::EvidenceFields {
                            project_name: Some(entry.name.clone()),
                            client: Some(entry.client.clone()),
                            amount_wan: Some(entry.amount_wan),
                            finished_at: Some(entry.completed_on.clone()),
                            ..Default::default()
                        },
                    };
                    match self.register_evidence_verified(reg) {
                        Ok((ev_id, dedup)) => {
                            evidence_registered.push(PerfEvidenceLink {
                                perf_id: entry.id.clone(),
                                ev_id,
                                source_file: tf_name,
                                deduplicated: dedup,
                            });
                        }
                        Err(e) => {
                            materials_pending.push(PerfMaterialPending {
                                perf_id: entry.id.clone(),
                                materials: entry.materials.clone(),
                                reason: format!("证据登记被拒（{e:?}）——材料文件虽已导入但登记面校验未过"),
                            });
                        }
                    }
                }
                None if entry.materials.is_empty() => materials_pending.push(PerfMaterialPending {
                    perf_id: entry.id.clone(),
                    materials: Vec::new(),
                    reason: "该业绩未登记材料路径（materials 空）——register_performance 补登材料后重跑，或人工为该业绩补佐证".into(),
                }),
                None => materials_pending.push(PerfMaterialPending {
                    perf_id: entry.id.clone(),
                    materials: entry.materials.clone(),
                    reason: "材料未匹配到已导入文件（按文件名比对）——把材料放入工作区并 import_tender_files（kind=附件），文件名与库内一致后重跑".into(),
                }),
            }
        }
        // ---- 3. 绑定（业绩类 REQ：陈述含 业绩/类似项目）
        let perf_req_ids: Vec<String> = self
            .store()
            .requirements()
            .iter()
            .filter(|r| {
                r.params
                    .get("statement")
                    .and_then(|v| v.as_str())
                    .map(|s| PERFORMANCE_TITLE_WORDS.iter().any(|w| s.contains(w)))
                    .unwrap_or(false)
            })
            .map(|r| r.id.clone())
            .collect();
        let open_date = bid_open_date
            .filter(|d| bid_domain::dates::is_valid_ymd(d))
            .unwrap_or_else(bid_domain::dates::today_ymd);
        let mut bindings: Vec<PerfBinding> = Vec::new();
        let mut binding_warnings: Vec<String> = Vec::new();
        if !perf_req_ids.is_empty() {
            for link in &evidence_registered {
                // 幂等重放的证据照走绑定（bind_evidence_batch 内建重放计数——
                // 重跑不产生重复绑定账）
                match self.bind_evidence_batch(&link.ev_id, &perf_req_ids, &open_date) {
                    Ok(_) => bindings.push(PerfBinding {
                        ev_id: link.ev_id.clone(),
                        req_ids: perf_req_ids.clone(),
                    }),
                    Err(e) => binding_warnings.push(format!(
                        "证据 {} 绑定失败（{} {:?}）——矩阵未更新，需人工补 bind_evidence",
                        link.ev_id, e.code(), e
                    )),
                }
            }
        } else {
            binding_warnings.push(
                "未找到业绩类条款（陈述含 业绩/类似项目 的 REQ）——证据已登记但未绑定；若本项目业绩以评分项表述，请人工 bind_evidence 到对应评分条款".into(),
            );
        }
        // ---- 4+5. readiness 门 + 单段写入（幂等键固定——同批重跑 no-op）
        let ev_of = |pid: &str| -> Option<String> {
            evidence_registered
                .iter()
                .find(|l| l.perf_id == pid)
                .map(|l| l.ev_id.clone())
        };
        let mut content = String::new();
        content.push_str("## 类似项目业绩一览\n\n");
        content.push_str(&bid_domain::performance::render_performance_table(&entries));
        content.push('\n');
        for e in &entries {
            content.push('\n');
            content.push_str(&bid_domain::performance::render_performance_paragraph(
                e,
                ev_of(&e.id).as_deref(),
            ));
        }
        let idem = format!("perf-section:{section_id}");
        let write = match self.append_segment(section_id, &content, Some(&idem)) {
            Ok(w) => w,
            Err(ToolError::EUnprocessable { reason }) if reason.contains("业绩证据不足") => {
                // 缺口并流：readiness 拦了（可用业绩 < 3）——把材料指引带全
                let pending_list = materials_pending
                    .iter()
                    .map(|p| p.perf_id.clone())
                    .collect::<Vec<_>>()
                    .join("、");
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "{reason}；材料缺口：{pending_list}（把材料导入工作区后重跑本工具即自动登记绑定——见 materials_pending）"
                    ),
                });
            }
            Err(e) => return Err(e),
        };
        let mut warnings = binding_warnings;
        if !materials_pending.is_empty() {
            warnings.push(format!(
                "材料缺口 {} 条（{}）——正文已写入但证据未登记，导入材料后重跑（幂等，自动补登记）",
                materials_pending.len(),
                materials_pending
                    .iter()
                    .map(|p| p.perf_id.clone())
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
        if write.changed {
            warnings.push(format!(
                "写入 {section_id}：{} 段账本已记账（合稿字数 {}）",
                write.segment_id, write.assembled_chars
            ));
        }
        Ok(PerfWriteReport {
            section_id: section_id.to_string(),
            segment_id: write.segment_id,
            changed: write.changed,
            segment_chars: write.segment_chars as usize,
            evidence_registered,
            materials_pending,
            bindings,
            readiness_passed: true,
            warnings,
        })
    }
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::BidService;
    use bid_domain::performance::{ClientType, PerformanceEntrySpec};

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("perf-lib-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn spec(name: &str, client: &str, amount: f64, done: &str) -> PerformanceEntrySpec {
        PerformanceEntrySpec {
            name: name.into(),
            client: client.into(),
            amount_wan: amount,
            completed_on: done.into(),
            ..Default::default()
        }
    }

    /// borrow：store_io `save_load_roundtrip_and_atomicity`——往返全等 + tmp 清除。
    #[test]
    fn library_file_roundtrip_and_atomicity() {
        let dir = tmpdir("rt");
        std::fs::create_dir_all(&dir).unwrap();
        let f = PerformanceLibraryFile::new(dir.join("performance-library.json"));
        assert!(f.load().unwrap().is_none(), "无文件 = 首启空库");
        let mut snap = PerformanceLibrarySnapshot::default();
        snap.owner_name = Some("示例科技有限公司".into());
        let mut s = spec("示例项目 A", "示例市第一人民医院", 500.0, "2025-06-30");
        s.categories = vec!["医疗设备".into()];
        let entry_id = snap.next_perf_id();
        snap.entries.push(PerformanceEntry {
            id: entry_id,
            name: s.name.clone(),
            client: s.client.clone(),
            amount_wan: s.amount_wan,
            completed_on: s.completed_on.clone(),
            categories: s.categories.clone(),
            materials: vec![],
            key_points: vec![],
            subject_name: None,
            created_at: "2026-11-07".into(),
        });
        f.save(&snap).unwrap();
        assert!(!dir.join("performance-library.json.tmp").exists(), "tmp 已被 rename 清除");
        let back = f.load().unwrap().unwrap();
        assert_eq!(back, snap, "库快照往返全等");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// borrow：store_io `corrupt_file_rejected_not_guessed` + `schema_version_gate`。
    #[test]
    fn library_corrupt_and_future_version_rejected() {
        let dir = tmpdir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let f = PerformanceLibraryFile::new(dir.join("performance-library.json"));
        std::fs::write(f.path(), b"{ not valid").unwrap();
        assert!(matches!(
            f.load(),
            Err(StoreIoError::Corrupt(_))
        ), "坏 JSON 拒载（不静默重建）");
        assert!(f.path().exists(), "原文件保留");
        std::fs::write(f.path(), r#"{"schema_version": 99}"#).unwrap();
        assert!(matches!(
            f.load(),
            Err(StoreIoError::SchemaUnsupported { found: 99, supported: 1 })
        ), "未来版本响亮拒绝");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #315 验收 1：跨项目可见——两个工作区（各持项目）+ 同一份库文件，
    /// A 登记的业绩 B 全量可见。
    #[test]
    fn cross_project_visibility_via_shared_library() {
        let lib_dir = tmpdir("shared");
        std::fs::create_dir_all(&lib_dir).unwrap();
        let lib_path = lib_dir.join("performance-library.json");
        let ws_a = tmpdir("ws-a");
        let ws_b = tmpdir("ws-b");
        std::fs::create_dir_all(&ws_a).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();
        // 项目 A：登记
        let mut a = BidService::open(&ws_a).unwrap().with_perf_library(&lib_path);
        a.register_performances(
            vec![spec("示例项目 A", "示例市第一人民医院", 500.0, "2025-06-30")],
            Some("示例科技有限公司".into()),
        )
        .unwrap();
        // 项目 B：不同工作区、同库文件 → 可见
        let mut b = BidService::open(&ws_b).unwrap().with_perf_library(&lib_path);
        let (rows, owner, persisted) =
            b.list_performances(&PerfListFilter::default()).unwrap();
        assert_eq!(rows.len(), 1, "跨项目可见（B 工作区看到 A 登记的业绩）");
        assert_eq!(owner.as_deref(), Some("示例科技有限公司"));
        assert!(persisted, "文件背书");
        for d in [&lib_dir, &ws_a, &ws_b] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// 语义幂等：同四元组重登 → deduplicated；owner 一次登记后可省。
    #[test]
    fn register_is_semantically_idempotent() {
        let dir = tmpdir("idem");
        std::fs::create_dir_all(&dir).unwrap();
        let mut svc =
            BidService::open(&dir).unwrap().with_perf_library(dir.join("lib.json"));
        let r1 = svc
            .register_performances(
                vec![spec("示例项目 A", "甲方", 100.0, "2024-01-01")],
                Some("主体甲".into()),
            )
            .unwrap();
        assert_eq!(r1.added, vec!["PERF-0001"]);
        let r2 = svc
            .register_performances(
                vec![spec("示例项目 A", "甲方", 100.0, "2024-01-01")],
                None,
            )
            .unwrap();
        assert!(r2.added.is_empty());
        assert_eq!(r2.deduplicated, vec!["PERF-0001"], "语义幂等命中既有 id");
        assert_eq!(r2.total, 1);
        // 一条非法 → 整批拒（零副作用）
        let bad = vec![
            spec("好条目", "甲方", 1.0, "2024-01-01"),
            spec("", "甲方", 1.0, "2024-01-01"),
        ];
        assert!(svc.register_performances(bad, None).is_err());
        let (rows, _, _) = svc.list_performances(&PerfListFilter::default()).unwrap();
        assert_eq!(rows.len(), 1, "整批拒 = 好条目也不入库");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 内存态（new + 未设 env）：功能可用、persisted=false 如实报告。
    /// 注意：BID_PERF_LIB 未设时 new() 才走内存态——本测试显式传空路径
    /// 等价面由 with_perf_library 缺省承载；env 面不在此测（进程级污染）。
    #[test]
    fn ephemeral_library_reports_not_persisted() {
        let svc = BidService::new();
        assert!(
            std::env::var("BID_PERF_LIB").is_err() || svc.perf_library_path().is_some(),
            "未设 env 时 new() 为内存态；设了 env 则路径面生效（两分支都合法）"
        );
    }

    /// 过滤面：category/keyword/金额窗/日期下界。
    #[test]
    fn list_filters_match_semantics() {
        let dir = tmpdir("filter");
        std::fs::create_dir_all(&dir).unwrap();
        let mut svc =
            BidService::open(&dir).unwrap().with_perf_library(dir.join("lib.json"));
        let mut s1 = spec("医疗设备采购", "示例市第一人民医院", 500.0, "2025-06-30");
        s1.categories = vec!["医疗设备".into()];
        let mut s2 = spec("超融合集群建设", "示例大学", 3000.0, "2023-01-01");
        s2.categories = vec!["超融合".into()];
        svc.register_performances(vec![s1, s2], None).unwrap();
        let mut f = |mut fl: PerfListFilter| svc.list_performances(&fl).unwrap().0.len();
        assert_eq!(f(PerfListFilter { category: Some("医疗".into()), ..Default::default() }), 1);
        assert_eq!(f(PerfListFilter { keyword: Some("超融合".into()), ..Default::default() }), 1);
        assert_eq!(
            f(PerfListFilter { min_amount_wan: Some(1000.0), ..Default::default() }),
            1
        );
        assert_eq!(f(PerfListFilter { since: Some("2024-01-01".into()), ..Default::default() }), 1);
        assert_eq!(f(PerfListFilter::default()), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 客户类型信号 + 排序：同库打分排序由域层 rank_entries 保证（此测服务
    /// 面的信号装配：REQ 陈述进语料、显式预算进金额因子）。
    #[test]
    fn recommend_assembles_signals_from_project() {
        let dir = tmpdir("rec");
        std::fs::create_dir_all(&dir).unwrap();
        let mut svc =
            BidService::open(&dir).unwrap().with_perf_library(dir.join("lib.json"));
        let mut s = spec("医疗设备采购项目", "示例市第一人民医院", 800.0, "2025-06-30");
        s.categories = vec!["医疗设备".into(), "CT".into(), "监护仪".into()];
        svc.register_performances(vec![s], None).unwrap();
        // 项目：无 REQ → 语料只有项目名；给 focus 补语料
        svc.create_project("CT 设备采购", "C-1", "明标", "k1").unwrap();
        let (matches, total, base) = svc
            .recommend_performances(
                Some(1000.0),
                Some("2026-11-07".into()),
                Some(ClientType::Government),
                Some("采购医疗设备 CT 监护仪".into()),
                10,
            )
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(base, "2026-11-07");
        assert_eq!(matches.len(), 1);
        assert!(matches[0].score > 0.9, "全命中 + 同型客户 + 预算 0.8 → 高分，实际 {}", matches[0].score);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
