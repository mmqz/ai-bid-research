//! assembly.rs —— S-2 断点续跑章节组装（issue #11 批次 v3 / V3-3）。
//!
//! 定位：百万字标书的**章节级组装工作区**——「每章生成完立即落盘，进程随时
//! 可死」（Lead 原文）的持久化半边。与 store_io.rs（域快照单文件 db.json）
//! 平级互补：db.json 存「调度状态」（树/状态机/台账），本模块存「章内容本体」
//! （每章一文件）——500 章 × 2,000 字的体量下，单文件快照每章重写全量是
//! O(N²) 写放大；每章一文件把提交成本摊平为 O(1)，这是断点续跑的物理前提。
//!
//! 原子提交（borrow：mdcore `sidecar.rs::SidecarStore::save` 的 tmp+rename
//! 纪律，经本仓 store_io.rs 同款落地）：每章 `chapters/{section_id}.json.tmp`
//! 写全量 → `rename` 替换（POSIX 原子；Windows 上 std::fs::rename 语义同为
//! 替换式）。**commit_section 是唯一的持久化点**——rename 返回即该章内容
//! 安全，此后进程随时可死。
//!
//! 前 N 章字节不变保证（硬验收核心）：已完成章**永不重写**——同
//! content+citations 重提交是 no-op（文件一个字节都不动）；断点续跑的跳过
//! 路径（assemble_all 对 has_committed 章）连 commit_section 都不进。因此
//! 「生成到第 300 章强杀 → 重启 resume → 前 299 章逐字节不变」由构造保证，
//! 不靠事后对账修复。
//!
//! 断点语义：断点 = **文档序首个未完成章**（resume_plan 返回的 todo[0]）；
//! 已提交但不在章节树中 → GhostSection 显式拒（树被重开纲后旧工作区不匹配
//! ——半个工作区不猜测，borrow mdcore fail-closed 哲学）。
//!
//! 损坏语义（borrow：mdcore `persistence_recovery.rs::
//! corrupt_truncated_and_missing_sidecar_fail_closed_on_open`）：章节文件
//! 截断/坏 JSON/缺字段/char_count 失配 → `Corrupt` 拒载且**原文件保留**
//! （不覆写——现场可勘查可修复）；`*.json.tmp` 残留 → 静默丢弃（中断写入
//! 的残骸，下次提交原子覆盖——borrow `interrupted_publish_leftover_does_
//! not_break_recovery`）。
//!
//! 状态推进与持久化的先后序：**先持久化后改状态**（assemble_all 内
//! generate → commit → advance DRAFTED）——死在 commit 与 advance 之间最多
//! 丢一次状态推进，章内容已安全；重启后跳过路径把落后状态补齐到 DRAFTED
//! （幂等——已在 DRAFTED 或更前者 no-op，不触发「重复推进被拒」）。
//!
//! crate 纪律不变：bid-domain 零 IO，本模块住 bid-tools（服务层）；时间戳
//! 用 SystemTime→UNIX_EPOCH 秒（u64，no-chrono）。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{DomainStore, ToolError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 章节工作区子目录名（`{dir}/chapters/{section_id}.json`）。
pub const CHAPTERS_SUBDIR: &str = "chapters";

/// 单章落盘记录（S-2 规格：content + char_count + attempt + citations + 时间戳）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterRecord {
    /// SEC-xxxx（章节树叶节点 id）
    pub section_id: String,
    /// 章正文
    pub content: String,
    /// content.chars().count()（中文字数口径——N-3 word_count 同源语义）
    pub char_count: u32,
    /// 本内容是第几次提交的版本（1 起；重提交不同内容 +1）
    pub attempt: u32,
    /// 引用的 REQ/EV id 清单
    pub citations: Vec<String>,
    /// UNIX_EPOCH 秒——时间戳（no-chrono 纪律）
    pub updated_at: u64,
}

/// 组装工作区错误（typed——调用方按变体分支，不解析 message；写法与中文
/// 错误消息参照 StoreIoError）。
#[derive(Debug)]
pub enum AssemblerError {
    Io(std::io::Error),
    Serialize(serde_json::Error),
    /// 章节文件损坏（截断/坏 JSON/结构缺字段/char_count 失配）——fail-closed
    /// 拒载，原文件保留（borrow：mdcore persistence_recovery 的
    /// corrupt_*_fail_closed_on_open 模式）
    Corrupt { path: PathBuf, why: String },
    /// 引用了章节树里不存在的 section_id（幽灵章——半个工作区）
    GhostSection { section_id: String },
    /// 章节生成失败（generator 返回 Err——该章未提交，下次续跑从此章重来）。
    /// 规格枚举面之外的补充变体：assemble_all 必须把 generator 失败透传为
    /// typed 错误，而既有变体无一可承载（Domain 只装 bid_domain::ToolError
    /// 的域裁决错——生成失败不是状态机拒绝）。
    Generator { section_id: String, why: String },
    /// 状态机拒绝（透传 bid_domain::ToolError）
    Domain(ToolError),
}

impl std::fmt::Display for AssemblerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssemblerError::Io(e) => write!(f, "章节工作区 IO 失败: {e}"),
            AssemblerError::Serialize(e) => write!(f, "章节记录序列化失败: {e}"),
            AssemblerError::Corrupt { path, why } => write!(
                f,
                "章节文件损坏（fail-closed 拒载，原文件保留）: {}（{why}）",
                path.display()
            ),
            AssemblerError::GhostSection { section_id } => write!(
                f,
                "已提交章 {section_id} 不在当前章节树中（幽灵章——重开纲后旧工作区不匹配，显式报错不猜测）"
            ),
            AssemblerError::Generator { section_id, why } => write!(
                f,
                "章节 {section_id} 生成失败（未提交——下次续跑从此章重来）: {why}"
            ),
            AssemblerError::Domain(e) => write!(f, "域状态机拒绝: {e:?}"),
        }
    }
}

impl std::error::Error for AssemblerError {}

/// 当前 UNIX_EPOCH 秒（no-chrono 纪律；时钟异常 → 0——版本单调性由 attempt
/// 承载，updated_at 只做展示/排查面）。
fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 章节组装工作区句柄（`{dir}/chapters/*.json`）。
///
/// 生命周期：`open` 装载既有提交（首启 = 空工作区）→ 反复 `commit_section`
/// （唯一持久化点）→ `resume_plan` 对账断点。内部 `records` 用 BTreeMap——
/// 迭代序确定（跨进程对账/报告可重放的前提）。
pub struct ChapterAssembler {
    /// 工作区根（store 目录；章节文件住 `dir/chapters/`）
    dir: PathBuf,
    /// section_id → 已提交记录（BTreeMap——确定性迭代）
    records: BTreeMap<String, ChapterRecord>,
}

impl ChapterAssembler {
    /// 打开（或首启创建）工作区：扫描 `chapters/*.json` 装载已提交章。
    ///
    /// - 损坏文件 → [`AssemblerError::Corrupt`]（fail-closed 拒载，原文件
    ///   保留——borrow：mdcore `persistence_recovery.rs::
    ///   corrupt_truncated_and_missing_sidecar_fail_closed_on_open`）；
    /// - `*.json.tmp` 残留 → 静默丢弃（中断写入的残骸，下次提交原子覆盖——
    ///   borrow：同文件 `interrupted_publish_leftover_does_not_break_recovery`）；
    /// - chapters 目录不存在 → 首启空态（不是错误——首次组装前没有该目录
    ///   是正常态，与 StoreFile 对缺失 db.json 的处理同语义）。
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, AssemblerError> {
        let dir = dir.as_ref().to_path_buf();
        let mut records = BTreeMap::new();
        let chapters_dir = dir.join(CHAPTERS_SUBDIR);
        if !chapters_dir.exists() {
            return Ok(Self { dir, records }); // 首启：无 chapters 目录
        }
        // 收集后按名排序再装载——目录遍历序不保证，排序让「多个损坏时报
        // 哪个」可重放（确定性诊断面）
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&chapters_dir)
            .map_err(AssemblerError::Io)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue, // 非 UTF-8 文件名——与章节无关，不碰
            };
            if name.ends_with(".json.tmp") {
                continue; // 中断提交残骸——静默丢弃（下次提交原子覆盖）
            }
            if !name.ends_with(".json") {
                continue; // 非章节文件（无关者不碰）
            }
            let raw = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                // 本写入器只产出合法 UTF-8 JSON——InvalidData = 外部截断/污染
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                    return Err(AssemblerError::Corrupt {
                        path: path.clone(),
                        why: format!("非 UTF-8（截断/二进制污染）: {e}"),
                    });
                }
                Err(e) => return Err(AssemblerError::Io(e)),
            };
            let rec: ChapterRecord = serde_json::from_str(&raw).map_err(|e| {
                AssemblerError::Corrupt {
                    path: path.clone(),
                    why: format!("章节 JSON 语法损坏/缺字段: {e}"),
                }
            })?;
            // 结构一致性三重校验（截断后 JSON 仍合法的深损也拦下）：
            // 1. 文件名 stem 必须与记录内 section_id 一致（防错位——半格工作区）；
            let stem = name.trim_end_matches(".json");
            if rec.section_id != stem {
                return Err(AssemblerError::Corrupt {
                    path: path.clone(),
                    why: format!(
                        "文件名 {stem} 与记录内 section_id {} 不一致（错位文件）",
                        rec.section_id
                    ),
                });
            }
            // 2. char_count 与 content 实际字数相符（篡改/截断对账）。
            let actual = rec.content.chars().count();
            if rec.char_count != actual as u32 {
                return Err(AssemblerError::Corrupt {
                    path: path.clone(),
                    why: format!(
                        "char_count={} 与 content 实际 {actual} 不符（记录被截断或篡改）",
                        rec.char_count
                    ),
                });
            }
            // 3. attempt 从 1 起（0 = 非本写入器产出——加性兼容面之外的怪态）。
            if rec.attempt == 0 {
                return Err(AssemblerError::Corrupt {
                    path: path.clone(),
                    why: "attempt=0（非法——本写入器从 1 起签发）".into(),
                });
            }
            records.insert(rec.section_id.clone(), rec);
        }
        Ok(Self { dir, records })
    }

    /// 已提交记录只读面（None = 未提交）。
    pub fn record(&self, section_id: &str) -> Option<&ChapterRecord> {
        self.records.get(section_id)
    }

    /// 该章是否已提交（断点续跑的跳过判据——assemble_all 消费面）。
    pub fn has_committed(&self, section_id: &str) -> bool {
        self.records.contains_key(section_id)
    }

    /// 已提交章总数（进度对账面）。
    pub fn committed_count(&self) -> usize {
        self.records.len()
    }

    /// 已提交章 id 的只读迭代面（BTreeMap 键序 = 字典序，确定可重放）。
    /// 消费方：S-6 DocumentExporter 的全记录对账（幽灵章 / 记录挂结构节点
    /// 检查需要遍历工作区侧的记录全集，而非按树逐个 probing——半个工作区
    /// 的「多出来的记录」只能从这一侧发现）。
    pub fn committed_ids(&self) -> impl Iterator<Item = &str> {
        self.records.keys().map(|s| &s[..])
    }

    /// 提交一章（每章生成完立即落盘——**本函数是唯一的持久化点**）。
    ///
    /// - 同 section_id 且 content+citations 与既有记录全等 → **no-op 返回既有
    ///   记录**（幂等重放——borrow：mdcore `cross_session_idempotent_replay`
    ///   的「首果复用零副作用」；字节稳定保证：文件一个字节都不动）；
    /// - content 或 citations 任一不同 → attempt+1 覆盖重写（修订通道——
    ///   重开后版本续增不回卷，borrow：`source_version_monotonic_across_reopen`）；
    /// - 新章 → attempt=1。
    ///
    /// 落盘：`chapters/{section_id}.json.tmp` 写全量 → rename（POSIX/Windows
    /// 替换式原子——borrow：mdcore sidecar tmp+rename 纪律，同本仓 store_io）。
    /// rename 返回即提交完成——此后进程随时可死，该章内容安全。
    pub fn commit_section(
        &mut self,
        section_id: &str,
        content: &str,
        citations: Vec<String>,
    ) -> Result<ChapterRecord, AssemblerError> {
        // 幂等面：全等重放 → 既有记录直返（零副作用——不碰文件）
        if let Some(existing) = self.records.get(section_id) {
            if existing.content == content && existing.citations == citations {
                return Ok(existing.clone());
            }
        }
        let attempt = match self.records.get(section_id) {
            Some(prev) => prev.attempt.saturating_add(1), // 修订：版本单调续增
            None => 1,                                    // 新章首提交
        };
        let rec = ChapterRecord {
            section_id: section_id.to_string(),
            content: content.to_string(),
            // 中文字数口径（N-3 word_count 同源语义）；2,000 字/章量级距 u32 上限四个数量级
            char_count: content.chars().count() as u32,
            attempt,
            citations,
            updated_at: now_secs(),
        };
        self.write_atomic(&rec)?;
        self.records.insert(rec.section_id.clone(), rec.clone());
        Ok(rec)
    }

    /// 断点计划：给定章节树全集（叶节点按文档序 order 排列），返回
    /// (已完成章 id 列表, 待生成章 id 列表)。断点 = 首个未完成章（todo[0]）。
    ///
    /// 语义：已提交但不在树中 → [`AssemblerError::GhostSection`] 拒（树被
    /// 重开纲后旧工作区不匹配——显式报错不猜测，不给静默续跑半个工作区）；
    /// 树中未提交 → 待生成。
    pub fn resume_plan(
        &self,
        leaf_ids_in_order: &[String],
    ) -> Result<(Vec<String>, Vec<String>), AssemblerError> {
        let tree: HashSet<&str> = leaf_ids_in_order.iter().map(|s| &s[..]).collect();
        for id in self.records.keys() {
            if !tree.contains(&id[..]) {
                return Err(AssemblerError::GhostSection { section_id: id.clone() });
            }
        }
        let mut done = Vec::new();
        let mut todo = Vec::new();
        for id in leaf_ids_in_order {
            if self.records.contains_key(id) {
                done.push(id.clone());
            } else {
                todo.push(id.clone());
            }
        }
        Ok((done, todo))
    }

    /// 单章原子落盘：tmp 写全量 → rename 替换（失败时目标文件不动）。
    fn write_atomic(&self, rec: &ChapterRecord) -> Result<(), AssemblerError> {
        let chapters_dir = self.dir.join(CHAPTERS_SUBDIR);
        std::fs::create_dir_all(&chapters_dir).map_err(AssemblerError::Io)?;
        let final_path = chapters_dir.join(format!("{}.json", rec.section_id));
        let tmp_path = chapters_dir.join(format!("{}.json.tmp", rec.section_id));
        let json = serde_json::to_string_pretty(rec).map_err(AssemblerError::Serialize)?;
        std::fs::write(&tmp_path, json.as_bytes()).map_err(AssemblerError::Io)?;
        std::fs::rename(&tmp_path, &final_path).map_err(AssemblerError::Io) // 原子提交点
    }
}

// ---------------------------------------------------------------- 组装驱动（与 DomainStore 联动）

/// 组装报告（对账面——一轮 assemble_all 的增量账）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssemblyReport {
    /// 树中叶节点总数
    pub chapters_total: usize,
    /// 本次运行新生成并提交的
    pub chapters_generated: usize,
    /// 断点续跑跳过的已完成章（不重跑——总 token 不含重复生成的结构保证）
    pub chapters_skipped: usize,
    /// 全部已提交章字数和（u64——500 章 × 2,000 字 = 1e6 量级，u32 余量四千倍）
    pub total_chars: u64,
}

/// 全量组装驱动：按文档序逐叶「OUTLINING→DRAFTING→生成→提交→DRAFTED」。
///
/// - 已提交章（has_committed）→ 跳过（**不调 generator**——每章恰好生成一次
///   的结构保证，总 token 不含重复生成）；域状态若落后于磁盘（崩溃窗口：
///   提交后未及推进；或重启后 store 重建、状态回 PENDING）→ 补齐推进到
///   DRAFTED（幂等——已是 DRAFTED 或更前者 no-op，不触发「重复推进被拒」）；
/// - 未提交章 → 逐级推进到 DRAFTING → generator → **commit_section 立即
///   落盘** → 推进 DRAFTED（先持久化后改状态——死在中间最多丢一次状态推进，
///   章内容已安全）；
/// - generator 失败 → 立即返回 [`AssemblerError::Generator`]（该章未提交，
///   下次续跑从此章重来）；
/// - 已提交记录不在树中 → [`AssemblerError::GhostSection`] 拒（同 resume_plan
///   语义——重开纲后的旧工作区显式报错）。
///
/// 叶节点判定：sections 中「无子节点者」（用 parent_id 统计子集）；文档序 =
/// order 升序。citations 落该叶绑定的 requirement_ids（章节响应的 REQ 清单
/// ——生成内容与 REQ 的引用关系由调度面固化，generator 只产正文）。
/// GhostSection 等域错误包进 [`AssemblerError::Domain`]。
pub fn assemble_all(
    store: &mut DomainStore,
    asm: &mut ChapterAssembler,
    gen: &mut dyn FnMut(&OutlineNode) -> Result<String, String>,
) -> Result<AssemblyReport, AssemblerError> {
    // 幽灵章防御：每条已提交记录必须都在当前树中（旧工作区遇新树——显式拒）
    let tree_ids: HashSet<&str> = store.sections().iter().map(|n| &n.id[..]).collect();
    for id in asm.records.keys() {
        if !tree_ids.contains(&id[..]) {
            return Err(AssemblerError::GhostSection { section_id: id.clone() });
        }
    }
    // 叶节点 = 无子者（parent_id 出现过的 id 即有子），按文档序（order 升序）
    let parents: HashSet<&str> =
        store.sections().iter().filter_map(|n| n.parent_id.as_deref()).collect();
    let mut leaves: Vec<OutlineNode> = store
        .sections()
        .iter()
        .filter(|n| !parents.contains(&n.id[..]))
        .cloned()
        .collect();
    leaves.sort_by_key(|n| n.order);

    let mut report = AssemblyReport {
        chapters_total: leaves.len(),
        chapters_generated: 0,
        chapters_skipped: 0,
        total_chars: 0,
    };
    for leaf in &leaves {
        if asm.has_committed(&leaf.id) {
            // 跳过面：不调 generator（增量 token = 0）；状态补齐到 DRAFTED（幂等）
            advance_to(store, &leaf.id, SectionStatus::Drafted).map_err(AssemblerError::Domain)?;
            report.chapters_skipped += 1;
        } else {
            advance_to(store, &leaf.id, SectionStatus::Outlining).map_err(AssemblerError::Domain)?;
            advance_to(store, &leaf.id, SectionStatus::Drafting).map_err(AssemblerError::Domain)?;
            let content = gen(leaf).map_err(|why| AssemblerError::Generator {
                section_id: leaf.id.clone(),
                why,
            })?;
            // 唯一持久化点：先落盘后推进状态（顺序不可换——见模块头）
            asm.commit_section(&leaf.id, &content, leaf.requirement_ids.clone())?;
            advance_to(store, &leaf.id, SectionStatus::Drafted).map_err(AssemblerError::Domain)?;
            report.chapters_generated += 1;
        }
    }
    // 字数账：工作区全量已提交章（幽灵已在入口拒——账面即树内账面）
    report.total_chars = asm.records.values().map(|r| u64::from(r.char_count)).sum();
    Ok(report)
}

// ---------------------------------------------------------------- 状态推进辅助（私有）

/// 线性链位次（PENDING=0 → ACCEPTED=6）。VALIDATING 后有分叉
/// （REVISING/ACCEPTED）但都排在 DRAFTED 之后——本模块只用 DRAFTED 前的
/// 线性段做补齐判定，分叉走向归上游显式决策。
fn status_rank(s: SectionStatus) -> u8 {
    use SectionStatus::*;
    match s {
        Pending => 0,
        Outlining => 1,
        Drafting => 2,
        Drafted => 3,
        Validating => 4,
        Revising => 5,
        Accepted => 6,
    }
}

/// 线性段前向单步（Pending→Outlining→Drafting→Drafted）。DRAFTED 及之后
/// 返回 None——advance_to 的 rank 守卫保证不会越段调用（防御性收口，非可达路径）。
fn step_forward(s: SectionStatus) -> Option<SectionStatus> {
    use SectionStatus::*;
    match s {
        Pending => Some(Outlining),
        Outlining => Some(Drafting),
        Drafting => Some(Drafted),
        Drafted | Validating | Revising | Accepted => None,
    }
}

/// 补齐推进（幂等）：沿合法前向链逐级走到 target；当前已在 target 或更前 →
/// no-op。修复两类「磁盘领先于状态」：(a) 崩溃窗口——commit 后未及推进；
/// (b) 重启后 store 重建——状态回 PENDING 而章内容已在盘。逐级单步走
/// advance_section_status（不越级——「重复推进同状态被拒」天然不会发生：
/// rank 守卫先拦）。
fn advance_to(store: &mut DomainStore, id: &str, target: SectionStatus) -> Result<(), ToolError> {
    loop {
        let cur = store
            .sections()
            .iter()
            .find(|n| n.id == id)
            .ok_or_else(|| ToolError::ENotFound { what: format!("section {id} 不存在") })?
            .status;
        if status_rank(cur) >= status_rank(target) {
            return Ok(());
        }
        match step_forward(cur) {
            Some(next) => store.advance_section_status(id, next)?,
            None => return Ok(()), // 理论不可达（rank 守卫）——防御性收口
        }
    }
}
