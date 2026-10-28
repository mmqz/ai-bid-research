//! bid-domain —— 标书领域模型与权威状态机（docs/bid-domain-model.md 的 Rust 落地）。
//!
//! 纪律（对齐设计文档）：
//! - 状态转换由本 crate 裁决，工具层只能申请（越级/非法 → `E_STATE`，零副作用）；
//! - ID 命名空间隔离：PRJ/TF/REQ/EV/CMP/SEC/VF/RT 由业务库签发，DOC-* 属文档内核（本 crate 只存引用）；
//! - 诚实状态：未确认/缺证据一律显式枚举，禁止用空值或 0 默认替代；
//! - 本 crate 零 IO（纯逻辑 + 内存态），持久化由 bid-tools 层承接。
//!
//! M1 范围：项目状态机前段（DRAFT→FILES_IMPORTED→REQUIREMENTS_EXTRACTED）+
//! 幂等键存储 + content_version 乐观并发 + E_STATE 越级防护（TC-11）+
//! 幂等重放（TC-07）。矩阵/起草/审校/导出的后段状态在 M2+ 扩展。
//!
//! M1.5 扩展（V2/V3 SEMUL 语义实装——四关键案例契约测试的载体）：
//! - [`impact`]：影响面引擎（补遗级联/DIRTY/content_version/终裁门禁/幂等）；
//! - [`linkage`]：证据链状态账本（只降不升 + 显式重验 + 外部修改不自动 adopt）；
//! - [`export_gate`]：两阶段导出门（EXPORTING → sha256 落库为界 → EXPORTED）。
//!
//! V3-1 扩展（issue #11 批次 v3——三份台账起步 + 持久化地基）：
//! - [`locator`]：引擎定位器七变体镜像（serde 形态与 mdcore 逐字节一致）；
//! - [`dates`]：ISO 日期工具（有效期窗口推算的单一真源）；
//! - [`evidence`]：T-1 Evidence 证据台账（只降不升 + R-03 过期禁绑 + 换版 Superseded）；
//! - [`outline`]：T-5 OutlineNode 章节树（九不变量 + 从 REQ 推导 + 配额最大余数法）；
//! - [`StoreSnapshot`]：T-8 持久化快照 DTO（本 crate 仍零 IO——磁盘读写住 bid-tools）。
//!
//! V3-2 扩展（issue #11 批次 v3——三份台账收尾）：
//! - [`matrix`]：T-2 MatrixRow 需求响应矩阵（四态派生 + blocking 导出闸门 R-03/R-04）；
//! - [`scoring`]：T-3 ScoringItem 评分矩阵（分值/标准原文/硬门三维结构，派生态）。

pub mod dates;
pub mod evidence;
pub mod export_gate;
pub mod impact;
pub mod linkage;
pub mod locator;
pub mod matrix;
pub mod outline;
pub mod scoring;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------- 错误码（mcp-contract §0）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolError {
    /// 状态机拒绝越级或非法转换（无副作用）
    EState { current: ProjectStatus, attempted: &'static str },
    /// project_version 过期（乐观并发冲突）
    EConflict { expected: u64, got: u64 },
    /// 对象不存在
    ENotFound { what: String },
    /// 参数不可用/不可恢复
    EUnprocessable { reason: String },
    /// 部分条目失败
    EPartial { item_errors: Vec<String> },
}

impl ToolError {
    pub fn code(&self) -> &'static str {
        match self {
            ToolError::EState { .. } => "E_STATE",
            ToolError::EConflict { .. } => "E_CONFLICT",
            ToolError::ENotFound { .. } => "E_NOTFOUND",
            ToolError::EUnprocessable { .. } => "E_UNPROCESSABLE",
            ToolError::EPartial { .. } => "E_PARTIAL",
        }
    }
}

// ---------------------------------------------------------------- 项目状态机（domain-model §2）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectStatus {
    Draft,
    FilesImported,
    RequirementsExtracted,
    MatrixReady,
    EvidenceBound,
    Drafting,
    Validating,
    Reviewing,
    ReadyToExport,
    Exported,
}

impl ProjectStatus {
    /// 合法前向转换表（权威状态机——唯一真源）。
    pub fn next_allowed(&self) -> &'static [ProjectStatus] {
        use ProjectStatus::*;
        match self {
            Draft => &[FilesImported],
            FilesImported => &[RequirementsExtracted],
            RequirementsExtracted => &[MatrixReady],
            MatrixReady => &[EvidenceBound],
            EvidenceBound => &[Drafting],
            Drafting => &[Validating],
            Validating => &[Reviewing],
            Reviewing => &[ReadyToExport],
            ReadyToExport => &[Exported],
            Exported => &[],
        }
    }

    /// 每个状态允许调用的工具（mcp-contract 各工具「状态」行——越级防护的裁决表）。
    pub fn allowed_tools(&self) -> &'static [&'static str] {
        use ProjectStatus::*;
        match self {
            Draft => &["create_bid_project", "import_tender_files", "get_bid_project_state"],
            FilesImported => &["import_tender_files", "extract_requirements",
                "get_compliance_matrix", "search_evidence", "get_bid_project_state"],
            RequirementsExtracted => &["import_tender_files", "extract_requirements",
                "get_compliance_matrix", "search_evidence", "get_bid_project_state"],
            // M2+ 状态的工具面按里程碑补全；先登记已知只读面
            MatrixReady | EvidenceBound | Drafting | Validating | Reviewing | ReadyToExport
            | Exported => &["get_compliance_matrix", "search_evidence", "get_bid_project_state"],
        }
    }

    pub fn as_str(&self) -> &'static str {
        use ProjectStatus::*;
        match self {
            Draft => "DRAFT",
            FilesImported => "FILES_IMPORTED",
            RequirementsExtracted => "REQUIREMENTS_EXTRACTED",
            MatrixReady => "MATRIX_READY",
            EvidenceBound => "EVIDENCE_BOUND",
            Drafting => "DRAFTING",
            Validating => "VALIDATING",
            Reviewing => "REVIEWING",
            ReadyToExport => "READY_TO_EXPORT",
            Exported => "EXPORTED",
        }
    }
}

// ---------------------------------------------------------------- 领域对象（M1 子集）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BidProject {
    pub id: String, // PRJ-xxxx
    pub name: String,
    pub code: String,
    pub mode: String, // 明标 | 暗标（M1 透传，导出面在 M4 消费）
    pub status: ProjectStatus,
    pub content_version: u64,
    pub external_key: String, // 创建幂等键
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseStatus {
    Ok,
    OcrRequired, // 无文本层扫描件——OCR 分档输入（不算失败）
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TenderFile {
    pub id: String, // TF-xxxx
    pub name: String,
    pub kind: String,      // 招标文件|附件|图纸|补遗|证据
    pub sha256: String,    // 天然幂等键
    pub pages: usize,
    pub tables: usize,
    pub chunks: usize,
    pub ocr_blocks: usize, // OCR 提取块数（非扫描件为 0）
    pub parse_status: ParseStatus,
    pub parse_summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String, // REQ-xxxx
    pub category: String,     // 资格条件|否决项|评分项|技术参数|商务要求|格式要求|其他
    pub clause_no: Option<String>,
    pub anchors: Vec<Anchor>, // 溯源锚（可多文件多锚点——GT-07 双锚点形态）
    pub params: HashMap<String, serde_json::Value>,
    pub source_file: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub file: String,
    pub kind: String,    // para|table|ocr|xlsx_cell
    pub locator: String, // 章节路径 / 表行 / 页
    pub excerpt: String, // 原文引用片段（对齐判分依据——创建时固化，不可篡改）
}

// ---------------------------------------------------------------- 权威状态机存储
/// 领域库（内存态）：项目/文件/条款 + 幂等键存储 + content_version。
/// 所有写入经 `apply` 走状态机裁决——直接改字段绕过状态机是不可能的（字段私有 + 仅经受审方法）。
///
/// V3-1（T-8 扩面）：证据台账 / 绑定 / 章节树三集合入列。快照互操作经
/// [`DomainStore::to_snapshot`] / [`DomainStore::restore`]（纯函数——磁盘 IO 在
/// bid-tools `store_io`，本 crate 零 IO 纪律不变）。
pub struct DomainStore {
    project: Option<BidProject>,
    files: Vec<TenderFile>,
    requirements: Vec<Requirement>,
    /// T-1 证据台账（EV-xxxx——登记/换版/有效期裁决见 evidence.rs 的 impl 块）
    evidences: Vec<evidence::Evidence>,
    /// T-1 证据↔条款绑定（T-2 MatrixRow 落地后由矩阵行消费）
    evidence_bindings: Vec<evidence::EvidenceBinding>,
    /// T-5 章节树（SEC-xxxx——树校验/推导/状态推进见 outline.rs 的 impl 块）
    sections: Vec<outline::OutlineNode>,
    /// T-2 矩阵行（CMP-xxxx——派生态，build_matrix 幂等重建；blocking = 导出闸门）
    matrix: Vec<matrix::MatrixRow>,
    /// T-2 人工覆写（不适用/偏离——派生态之上的唯一人工输入）
    matrix_overrides: HashMap<String, matrix::MatrixOverride>,
    /// 幂等键 → 首次结果 JSON（同 key 重放返回首次结果，不重复执行）
    idempotency: HashMap<String, serde_json::Value>,
    id_seq: u64,
}

impl DomainStore {
    pub fn new() -> Self {
        Self { project: None, files: vec![], requirements: vec![],
               evidences: vec![], evidence_bindings: vec![], sections: vec![],
               matrix: vec![], matrix_overrides: HashMap::new(),
               idempotency: HashMap::new(), id_seq: 0 }
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.id_seq += 1;
        let n = self.id_seq;
        format!("{prefix}-{n:04}")
    }

    pub fn project(&self) -> Result<&BidProject, ToolError> {
        self.project.as_ref().ok_or_else(|| ToolError::ENotFound {
            what: "project 未创建".into(),
        })
    }

    pub fn files(&self) -> &[TenderFile] {
        &self.files
    }

    pub fn requirements(&self) -> &[Requirement] {
        &self.requirements
    }

    pub fn content_version(&self) -> u64 {
        self.project.as_ref().map(|p| p.content_version).unwrap_or(0)
    }

    /// 幂等键查询：命中 → 返回首次结果（调用方必须直接复用，不得再执行）。
    pub fn idempotent_hit(&self, key: &str) -> Option<&serde_json::Value> {
        self.idempotency.get(key)
    }

    pub fn record_idempotent(&mut self, key: String, result: serde_json::Value) {
        self.idempotency.insert(key, result);
    }

    // ------------------------------------------------------------ create_bid_project（契约 §1）
    pub fn create_project(&mut self, name: &str, code: &str, mode: &str,
                          external_key: &str) -> Result<(String, serde_json::Value), ToolError> {
        if let Some(p) = &self.project {
            if p.external_key == external_key {
                // 契约：external_key 相同 → 静默幂等返回已有 project_id
                let r = serde_json::json!({ "project_id": p.id, "status": p.status.as_str(),
                                             "deduplicated": true });
                return Ok((p.id.clone(), r));
            }
            return Err(ToolError::EUnprocessable {
                reason: "store 已持有不同 external_key 的项目（M1 单项目存储）".into(),
            });
        }
        let id = self.next_id("PRJ");
        let proj = BidProject {
            id: id.clone(),
            name: name.into(),
            code: code.into(),
            mode: mode.into(),
            status: ProjectStatus::Draft,
            content_version: 1,
            external_key: external_key.into(),
        };
        let r = serde_json::json!({ "project_id": id, "status": "DRAFT" });
        self.project = Some(proj);
        Ok((id, r))
    }

    // ------------------------------------------------------------ 工具准入裁决（E_STATE 门）
    /// TC-11：状态机越级防护——attempted 工具不在当前状态允许清单 → E_STATE，零副作用。
    pub fn guard_tool(&self, tool: &str) -> Result<(), ToolError> {
        let p = self.project.as_ref().ok_or_else(|| ToolError::ENotFound {
            what: "project 未创建".into(),
        })?;
        if p.status.allowed_tools().contains(&tool) {
            Ok(())
        } else {
            Err(ToolError::EState { current: p.status, attempted: tool_name(tool) })
        }
    }

    /// 乐观并发检查（契约 §0：project_version 过期 → E_CONFLICT）。
    /// M1 工具调用方传 `None` 表示跳过检查（后端内部步骤）；外部调用必须传。
    pub fn check_version(&self, expected: Option<u64>) -> Result<u64, ToolError> {
        let p = self.project()?;
        let cur = p.content_version;
        if let Some(exp) = expected {
            if exp != cur {
                return Err(ToolError::EConflict { expected: exp, got: cur });
            }
        }
        Ok(cur)
    }

    // ------------------------------------------------------------ import_tender_files（契约 §2）
    /// 文件按 sha256 天然幂等：已存在 → no-op 返回原 file_id（deduplicated）。
    pub fn upsert_file(&mut self, name: &str, kind: &str, sha256: &str,
                       pages: usize, tables: usize, chunks: usize, ocr_blocks: usize,
                       parse_status: ParseStatus, parse_summary: &str)
                       -> Result<(String, bool), ToolError> {
        if let Some(f) = self.files.iter().find(|f| f.sha256 == sha256) {
            return Ok((f.id.clone(), true)); // deduplicated: true
        }
        let id = self.next_id("TF");
        self.files.push(TenderFile {
            id: id.clone(),
            name: name.into(),
            kind: kind.into(),
            sha256: sha256.into(),
            pages, tables, chunks, ocr_blocks,
            parse_status,
            parse_summary: parse_summary.into(),
        });
        if let Some(p) = self.project.as_mut() {
            if p.status == ProjectStatus::Draft {
                p.status = ProjectStatus::FilesImported;
            }
            p.content_version += 1; // 内容变化（追加文件属允许的脏追加，契约 §2）
        }
        Ok((id, false))
    }

    // ------------------------------------------------------------ extract_requirements（契约 §3）
    /// 写入条款集（dedupe_hash 幂等合并：同源锚+同陈述 → 复用既有 REQ）。
    pub fn set_requirements(&mut self, reqs: Vec<Requirement>) -> Result<(usize, usize), ToolError> {
        let p = self.project.as_ref().ok_or_else(|| ToolError::ENotFound {
            what: "project 未创建".into(),
        })?;
        if p.status != ProjectStatus::FilesImported && p.status != ProjectStatus::RequirementsExtracted {
            return Err(ToolError::EState {
                current: p.status,
                attempted: "extract_requirements",
            });
        }
        // 幂等合并：dedupe_hash = sha256(排序后 (file, locator, excerpt, statement_norm))
        let mut merged: Vec<Requirement> = self.requirements.clone();
        let before = merged.len();
        for r in reqs {
            let h = requirement_hash(&r);
            let existing = merged.iter().position(|x| requirement_hash(x) == h);
            match existing {
                Some(i) => merged[i] = r, // 重跑覆盖（同 hash → 稳定）
                None => merged.push(r),
            }
        }
        let added = merged.len() - before;
        self.requirements = merged;
        let p = self.project.as_mut().unwrap();
        if p.status == ProjectStatus::FilesImported {
            p.status = ProjectStatus::RequirementsExtracted;
        }
        p.content_version += 1;
        Ok((self.requirements.len(), added))
    }
}

impl Default for DomainStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- T-8 持久化快照（纯函数面）
/// 快照 schema 版本（加性演进：新字段一律 `#[serde(default)]`，旧数据可加载；
/// 结构性变更 → 版本号 +1 + `migrate`（bid-tools store_io）——参考 mdcore sidecar）。
pub const STORE_SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    STORE_SCHEMA_VERSION
}

/// 领域库快照（T-8）——`DomainStore` 全量状态的 serde DTO。
///
/// - 全字段 `#[serde(default)]`：旧版快照缺新集合字段 → 空集合加载（加性迁移）；
/// - `content_version` 住在 `BidProject` 内随快照走；
/// - 语义完整性由 [`DomainStore::restore`] 把关（id 唯一 / 序列水位 / 孤儿数据），
///   JSON 语法与版本门在 bid-tools `store_io`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreSnapshot {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub project: Option<BidProject>,
    #[serde(default)]
    pub files: Vec<TenderFile>,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub evidences: Vec<evidence::Evidence>,
    #[serde(default)]
    pub evidence_bindings: Vec<evidence::EvidenceBinding>,
    #[serde(default)]
    pub sections: Vec<outline::OutlineNode>,
    #[serde(default)]
    pub matrix: Vec<matrix::MatrixRow>,
    #[serde(default)]
    pub matrix_overrides: HashMap<String, matrix::MatrixOverride>,
    #[serde(default)]
    pub idempotency: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub id_seq: u64,
}

impl DomainStore {
    /// 全量快照（纯读——checkpoint 的素材面）。
    pub fn to_snapshot(&self) -> StoreSnapshot {
        StoreSnapshot {
            schema_version: STORE_SCHEMA_VERSION,
            project: self.project.clone(),
            files: self.files.clone(),
            requirements: self.requirements.clone(),
            evidences: self.evidences.clone(),
            evidence_bindings: self.evidence_bindings.clone(),
            sections: self.sections.clone(),
            matrix: self.matrix.clone(),
            matrix_overrides: self.matrix_overrides.clone(),
            // HashMap → BTreeMap：持久化 DTO 要求跨进程字节一致（HashMap 迭代序不确定）
            idempotency: self.idempotency.clone().into_iter().collect(),
            id_seq: self.id_seq,
        }
    }

    /// 从快照恢复（重启续跑的领域半边）。
    ///
    /// 语义完整性把关（损坏但语法合法的 JSON 也拒——fail-closed 不猜测）：
    /// - schema_version 必须等于当前版本（旧版本由 IO 层 migrate 后再进这里）；
    /// - 各集合 id 唯一；
    /// - `id_seq` 水位 ≥ 全体 id 的数字后缀（防重启后 id 重签碰撞）；
    /// - 有数据必有项目（孤儿集合 = 半个库）。
    pub fn restore(snap: StoreSnapshot) -> Result<Self, ToolError> {
        if snap.schema_version != STORE_SCHEMA_VERSION {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "快照 schema_version={} ≠ 当前支持 {}（migrate 先行——bid-tools store_io）",
                    snap.schema_version, STORE_SCHEMA_VERSION
                ),
            });
        }
        let un = |reason: String| ToolError::EUnprocessable { reason };
        // id 唯一性（逐集合）
        let dup_in = |ids: Vec<String>, what: &str| -> Option<String> {
            let mut seen = std::collections::HashSet::new();
            ids.into_iter().find_map(|i| (!seen.insert(i)).then(|| format!("{what} id 重复")))
        };
        if let Some(e) = dup_in(snap.files.iter().map(|f| f.id.clone()).collect(), "files")
            .or_else(|| dup_in(snap.requirements.iter().map(|r| r.id.clone()).collect(), "requirements"))
            .or_else(|| dup_in(snap.evidences.iter().map(|e| e.id.clone()).collect(), "evidences"))
            .or_else(|| dup_in(snap.sections.iter().map(|s| s.id.clone()).collect(), "sections"))
        {
            return Err(un(e));
        }
        // id_seq 水位（防重启后重签碰撞——续跑的第一件事就是签新 id）。
        // 注意：只校验 **store 签发**的命名空间（PRJ/TF/EV/SEC）——REQ id 由
        // extract 层自有计数器签发（每次抽取重置，与 store 序列无关），
        // 不在本水位管辖面。
        let watermark = [
            max_suffix(snap.files.iter().map(|f| f.id.clone())),
            max_suffix(snap.evidences.iter().map(|e| e.id.clone())),
            max_suffix(snap.sections.iter().map(|s| s.id.clone())),
            snap.project.as_ref().map(|p| max_suffix([p.id.clone()].into_iter())).unwrap_or(0),
        ].into_iter().max().unwrap_or(0);
        if snap.id_seq < watermark {
            return Err(un(format!(
                "id_seq={} 低于 id 水位 {}（重启后新 id 会与既有记录碰撞——快照被截断？",
                snap.id_seq, watermark
            )));
        }
        // 孤儿数据（有集合无项目 = 半个库）
        if snap.project.is_none() && (!snap.files.is_empty() || !snap.requirements.is_empty()
            || !snap.evidences.is_empty() || !snap.sections.is_empty())
        {
            return Err(un("快照有数据但无项目（孤儿集合——拒绝加载半个库）".into()));
        }
        Ok(Self {
            project: snap.project,
            files: snap.files,
            requirements: snap.requirements,
            evidences: snap.evidences,
            evidence_bindings: snap.evidence_bindings,
            sections: snap.sections,
            matrix: snap.matrix,
            matrix_overrides: snap.matrix_overrides,
            idempotency: snap.idempotency.into_iter().collect(),
            id_seq: snap.id_seq,
        })
    }
}

/// id 数字后缀最大值（水位校验：`TF-0007` → 7；非数字尾 → 忽略）。
fn max_suffix(ids: impl Iterator<Item = String>) -> u64 {
    ids.filter_map(|i| i.rsplit('-').next()
        .and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0)
}

fn tool_name(tool: &str) -> &'static str {
    match tool {
        "create_bid_project" => "create_bid_project",
        "import_tender_files" => "import_tender_files",
        "extract_requirements" => "extract_requirements",
        "search_evidence" => "search_evidence",
        "get_bid_project_state" => "get_bid_project_state",
        "draft_response_section" => "draft_response_section",
        "validate_bid_document" => "validate_bid_document",
        "export_bid_document" => "export_bid_document",
        _ => "unknown_tool",
    }
}

pub fn requirement_hash(r: &Requirement) -> String {
    let mut parts: Vec<String> = r.anchors.iter()
        .map(|a| format!("{}|{}|{}", a.file, a.locator, a.excerpt))
        .collect();
    parts.sort();
    let stmt = r.anchors.iter().map(|a| a.excerpt.as_str()).collect::<Vec<_>>().join("");
    let mut hasher = Sha256::new();
    hasher.update(parts.join("\n"));
    hasher.update(&stmt);
    hex::encode(hasher.finalize())[..24].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_files() -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("测试项目", "ZFCG-2026-GK-0311", "明标", "ext-key-1").unwrap();
        s.upsert_file("A-招标文件.docx", "招标文件", "aa", 31, 12, 120, 0,
                      ParseStatus::Ok, "").unwrap();
        s
    }

    #[test]
    fn tc11_state_guard_rejects_skip_ahead_with_no_side_effect() {
        // TC-11：FILES_IMPORTED 阶段直接调 draft_response_section → E_STATE
        let s = store_with_files();
        let err = s.guard_tool("draft_response_section").unwrap_err();
        assert_eq!(err.code(), "E_STATE");
        // 无副作用：项目状态不变、无章节对象（M1 无章节面——以状态+文件数为准）
        assert_eq!(s.project().unwrap().status, ProjectStatus::FilesImported);
        assert_eq!(s.files().len(), 1);
    }

    #[test]
    fn state_sequence_forward_only() {
        let mut s = DomainStore::new();
        s.create_project("p", "c", "明标", "k").unwrap();
        assert_eq!(s.project().unwrap().status, ProjectStatus::Draft);
        s.upsert_file("f1", "招标文件", "sha1", 1, 0, 5, 0, ParseStatus::Ok, "").unwrap();
        assert_eq!(s.project().unwrap().status, ProjectStatus::FilesImported);
        // Draft 之后唯一前向是 FilesImported；FilesImported 的工具面不含 draft_response_section
        assert!(ProjectStatus::FilesImported.next_allowed().contains(&ProjectStatus::RequirementsExtracted));
    }

    #[test]
    fn create_project_idempotent_by_external_key() {
        let mut s = DomainStore::new();
        let (id1, r1) = s.create_project("p", "c", "明标", "key-A").unwrap();
        let (id2, r2) = s.create_project("p2", "c2", "暗标", "key-A").unwrap();
        assert_eq!(id1, id2, "同 external_key 必须返回原 project_id");
        assert_eq!(r2["deduplicated"], true);
        assert_eq!(r1["status"], "DRAFT");
    }

    #[test]
    fn tc07_import_same_sha_is_noop() {
        let mut s = store_with_files();
        let before_files = s.files().len();
        let before_version = s.content_version();
        let (id2, dup) = s.upsert_file("A-招标文件.docx", "招标文件", "aa", 31, 12, 120, 0,
                                        ParseStatus::Ok, "").unwrap();
        assert!(dup, "同 sha256 必须判 deduplicated");
        assert_eq!(s.files().len(), before_files, "无重复文件记录");
        assert_eq!(s.content_version(), before_version, "no-op 不 bump content_version");
        let (id1, _) = s.files().iter().map(|f| (f.id.clone(), ())).next().unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn content_version_bumps_on_write_not_on_read() {
        let mut s = store_with_files();
        let v = s.content_version();
        let _ = s.guard_tool("get_bid_project_state"); // 只读不 bump
        assert_eq!(s.content_version(), v);
        s.upsert_file("B3.xlsx", "图纸", "bb", 2, 1, 20, 0, ParseStatus::Ok, "").unwrap();
        assert_eq!(s.content_version(), v + 1);
    }

    #[test]
    fn extract_requirements_idempotent_merge() {
        let mut s = store_with_files();
        let reqs = vec![Requirement {
            id: "REQ-0001".into(), category: "资格条件".into(), clause_no: Some("3.2.1".into()),
            anchors: vec![Anchor { file: "A".into(), kind: "para".into(), locator: "第三章".into(),
                                    excerpt: "独立承担民事责任能力的法人营业执照".into() }],
            params: HashMap::new(), source_file: "A".into(),
        }];
        let (n1, added1) = s.set_requirements(reqs.clone()).unwrap();
        assert_eq!((n1, added1), (1, 1));
        let (n2, added2) = s.set_requirements(reqs).unwrap();
        assert_eq!((n2, added2), (1, 0), "同 dedupe_hash 重放 → 合并不新增");
        assert_eq!(s.project().unwrap().status, ProjectStatus::RequirementsExtracted);
    }

    #[test]
    fn optimistic_concurrency_conflict() {
        let s = store_with_files();
        assert!(matches!(s.check_version(Some(999)), Err(ToolError::EConflict { .. })));
        let v = s.content_version();
        assert!(s.check_version(Some(v)).is_ok());
        assert!(s.check_version(None).is_ok(), "内部步骤跳过检查");
    }
}
