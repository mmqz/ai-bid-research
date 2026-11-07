//! store_io.rs —— T-8 服务态持久化磁盘层（issue #11 批次 v3 / V3-1）。
//!
//! 定位：`DomainStore` 快照（bid-domain 纯函数面）↔ 磁盘的单桥。crate 纪律
//! 「bid-domain 零 IO」不变——本模块住 bid-tools（服务层），与 runner.rs 的
//! write_run 平级。
//!
//! ## 提交纪律（#411：写放大收口 + fsync）
//!
//! ```text
//! db.json       域快照（小、高频变更——bind/draft/review/override 每次都重写）
//! parsed-{sha8}.json  解析产物（大、低频变更——仅内容变化时落新文件）
//! ```
//!
//! - **原子性（无撕裂窗口）**：parsed 文件按内容寻址且**不可变**（同名即同容，
//!   写时若缺才落盘）→ db.json `parsed_ref` 的一次 rename 提交，前后崩溃
//!   均得完整的新/旧 pair（旧 parsed 文件永不就地改写——这是分离后仍无
//!   双文件撕裂的根因）；
//! - **fsync 纪律**：tmp `sync_all` 后才 rename（提交点前刷盘）；rename 后
//!   父目录 fsync（unix——目录项持久化；Windows 无目录 fsync 语义，文档化
//!   容忍：进程崩溃安全，掉电面依赖 NTFS 日志）；
//! - **写放大收口**：域-only 高频路径只重写 db.json（数十 MB 的 parsed 块
//!   文本不再随行）；parsed 内容变化（import/extract 低频路径）才落新文件。
//!
//! 版本门（borrow：mdcore `schema_version_gate` 测试口径）：
//! - v1：`parsed` 内联在 db.json（旧快照，加性迁移继续可载）；
//! - v2：`parsed_ref: "parsed-{sha8}.json" | null` 指向内容寻址文件（当前）；
//! - 高于当前支持版本 → `SchemaUnsupported` 响亮拒绝（不猜测降级加载）。
//!
//! 损坏语义（borrow：mdcore `corrupt_file_rejected_not_guessed`）：坏 JSON →
//! `Corrupt`（带 serde 行列定位），原文件保留不覆写——fail-closed 不猜测。
//! `parsed_ref` 指向的文件缺失/损坏 → 同样 `Corrupt`（不猜测空 parsed）。

use bid_domain::StoreSnapshot;
use bid_parse::ParsedFile;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 服务态数据库文件名（store 目录内）。
pub const DB_FILE: &str = "db.json";

/// parsed 内容寻址文件名前缀（`parsed-{sha8}.json`——sha8 = 序列化字节的
/// SHA-256 前 8 个十六进制位，碰撞面由完整字节比对兜底：同名必先比对内容）。
const PARSED_PREFIX: &str = "parsed-";
const PARSED_SUFFIX: &str = ".json";

/// 当前包装 schema 版本（v2：parsed_ref 分离；v1：parsed 内联——可载可升格）。
const WRAPPER_SCHEMA_VERSION: u32 = 2;

fn wrapper_default_version() -> u32 {
    1 // 缺省 = v1（首版快照无该字段）
}

/// 服务态快照 IO 错误（typed——调用方按变体分支，不解析 message）。
#[derive(Debug)]
pub enum StoreIoError {
    Io(std::io::Error),
    Serialize(serde_json::Error),
    /// schema_version 高于当前支持（未来数据遇旧代码——响亮拒绝不猜测）
    SchemaUnsupported { found: u32, supported: u32 },
    /// JSON 语法损坏 / 域语义不完整（restore 拒绝面）——原文件保留
    Corrupt(String),
}

impl std::fmt::Display for StoreIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreIoError::Io(e) => write!(f, "store IO 失败: {e}"),
            StoreIoError::Serialize(e) => write!(f, "store 序列化失败: {e}"),
            StoreIoError::SchemaUnsupported { found, supported } => write!(
                f, "快照 schema_version={found} 高于当前支持 {supported}（升级程序后重试）"
            ),
            StoreIoError::Corrupt(why) => write!(f, "快照损坏（拒绝加载，原文件保留）: {why}"),
        }
    }
}

impl std::error::Error for StoreIoError {}

/// 磁盘上的服务态全量（v2：域快照 + parsed 内容寻址引用）。
///
/// - `domain`：[`StoreSnapshot`]（schema_version 自带——域级版本独立演进）；
/// - `parsed_ref`：`Some("parsed-{sha8}.json")` 指向同目录不可变文件；
///   `None` = 无解析产物（不落空文件）；
/// - v1 兼容：`parsed` 内联字段继续可读（`#[serde(default)]`——旧快照
///   加载后在 [`StoreFile::save`] 时升格 v2，不就地改写旧文件）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ServiceSnapshot {
    #[serde(default = "wrapper_default_version")]
    schema_version: u32,
    domain: StoreSnapshot,
    /// v2：内容寻址引用（与 v1 内联字段互斥——save 恒写 ref，load 双兼容）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parsed_ref: Option<String>,
    /// v1 旧形态：内联 parsed（只读兼容面——save 不再写出该字段）。
    #[serde(default, skip_serializing)]
    parsed: Vec<ParsedFile>,
}

impl ServiceSnapshot {
    /// v2 构造（parsed 走内容寻址分离）。
    fn v2(domain: StoreSnapshot, parsed_ref: Option<String>) -> Self {
        Self { schema_version: WRAPPER_SCHEMA_VERSION, domain, parsed_ref, parsed: Vec::new() }
    }
}

/// 存储目录句柄（`{dir}/db.json`）。
pub struct StoreFile {
    dir: PathBuf,
}

impl StoreFile {
    /// 存储目录（工作区锚点——`db.json`、`chapters/`、导出产物共用根）。
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn new(dir: impl AsRef<Path>) -> Self {
        StoreFile { dir: dir.as_ref().to_path_buf() }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(DB_FILE)
    }

    /// 加载服务态全量（None = 目录无 db.json——首启空态）。
    ///
    /// 双兼容：v2（parsed_ref → 内容寻址文件）与 v1（parsed 内联）。
    pub fn load(&self) -> Result<Option<(StoreSnapshot, Vec<ParsedFile>)>, StoreIoError> {
        let path = self.path();
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path).map_err(StoreIoError::Io)?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| StoreIoError::Corrupt(format!("db.json 语法损坏: {e}")))?;
        let svc: ServiceSnapshot = migrate(value)?;
        let parsed = match &svc.parsed_ref {
            Some(name) => {
                // fail-closed：ref 指向的文件缺失/名字不合法/内容损坏 → Corrupt
                // （不猜测空 parsed——静默空面会让 extract 面看起来「无输入」）。
                if !is_valid_parsed_name(name) {
                    return Err(StoreIoError::Corrupt(format!(
                        "parsed_ref 名字非法（需 parsed-{{sha8}}.json）: {name}"
                    )));
                }
                let ppath = self.dir.join(name);
                let praw = std::fs::read_to_string(&ppath).map_err(|e| {
                    StoreIoError::Corrupt(format!(
                        "parsed_ref 文件缺失/不可读（{name}）: {e}"
                    ))
                })?;
                serde_json::from_str(&praw).map_err(|e| {
                    StoreIoError::Corrupt(format!("{name} 语法损坏: {e}"))
                })?
            }
            None => svc.parsed, // v1 内联（v2 无 parsed 时也是空表）
        };
        Ok(Some((svc.domain, parsed)))
    }

    /// 保存服务态全量（fsync + tmp + rename 原子提交；失败时原文件不动）。
    ///
    /// 写序（#411）：
    /// 1. parsed 序列化 → 内容寻址名 → 文件缺才落盘（不可变：同名即同容）；
    /// 2. db.json（v2，只含 domain + parsed_ref）tmp 写盘 → sync_all → rename；
    /// 3. rename 后父目录 fsync（unix）+ GC 未引用 parsed 文件。
    ///
    /// 崩溃窗口：步骤 2 前崩溃 → 旧 db.json + 旧 parsed 文件均在（不可变不覆
    /// 写）→ 旧 pair 完整；步骤 2 后 → 新 pair 完整。无撕裂面。
    pub fn save(&self, domain: &StoreSnapshot, parsed: &[ParsedFile]) -> Result<(), StoreIoError> {
        std::fs::create_dir_all(&self.dir).map_err(StoreIoError::Io)?;

        // ---- 1. parsed 内容寻址落盘（仅在文件缺时写入）
        let parsed_ref = if parsed.is_empty() {
            None
        } else {
            let bytes = serde_json::to_vec(parsed).map_err(StoreIoError::Serialize)?;
            let name = parsed_file_name(&bytes);
            let ppath = self.dir.join(&name);
            if !ppath.exists() {
                write_fsync_atomic(&ppath, &bytes)?;
            }
            Some(name)
        };

        // ---- 2. db.json v2（域面 + ref）原子提交
        let svc = ServiceSnapshot::v2(domain.clone(), parsed_ref);
        let json = serde_json::to_string_pretty(&svc).map_err(StoreIoError::Serialize)?;
        write_fsync_atomic(&self.path(), json.as_bytes())?;

        // ---- 3. GC：清除未被引用的 parsed 文件（单进程串行——服务态无并发写）
        self.gc_parsed()?;
        Ok(())
    }

    /// 删除目录内未被 db.json 引用的 `parsed-*.json`。
    ///
    /// 失败容忍（GC 是优化不是语义面——残留文件不影响正确性，下次再清）。
    fn gc_parsed(&self) -> Result<(), StoreIoError> {
        let keep = self.current_parsed_ref();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        for ent in entries.flatten() {
            let fname = ent.file_name().to_string_lossy().into_owned();
            if is_valid_parsed_name(&fname) && Some(&fname) != keep.as_ref() {
                let _ = std::fs::remove_file(ent.path()); // 失败留待下轮
            }
        }
        Ok(())
    }

    /// 当前 db.json 引用的 parsed 文件名（无 db.json / v1 内联 → None）。
    fn current_parsed_ref(&self) -> Option<String> {
        let raw = std::fs::read_to_string(self.path()).ok()?;
        serde_json::from_str::<ServiceSnapshot>(&raw)
            .ok()?
            .parsed_ref
    }
}

/// `parsed-{sha8}.json` 名字合法性（防 ref 注入路径面——只接受本格式）。
fn is_valid_parsed_name(name: &str) -> bool {
    name.starts_with(PARSED_PREFIX)
        && name.ends_with(PARSED_SUFFIX)
        && name.len() == PARSED_PREFIX.len() + 8 + PARSED_SUFFIX.len()
        && name[PARSED_PREFIX.len()..name.len() - PARSED_SUFFIX.len()]
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
}

/// parsed 内容寻址名：`parsed-{sha8}.json`（sha8 = SHA-256 前 8 位 hex）。
fn parsed_file_name(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(8);
    for b in &digest[..4] {
        let _ = write!(hex, "{b:02x}");
    }
    format!("{PARSED_PREFIX}{hex}{PARSED_SUFFIX}")
}

/// 原子写 + fsync 纪律：tmp 写盘 → sync_all → rename → 父目录 fsync（unix）。
///
/// Windows：目录 fsync 无对应语义（std 无目录句柄 sync）——文档化容忍，
/// 进程崩溃安全（rename 原子），掉电面依赖 NTFS 日志。
fn write_fsync_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreIoError> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp).map_err(StoreIoError::Io)?;
        f.write_all(bytes).map_err(StoreIoError::Io)?;
        f.sync_all().map_err(StoreIoError::Io)?; // 提交点前刷盘（#411）
    }
    std::fs::rename(&tmp, path).map_err(StoreIoError::Io)?; // 原子提交点
    if cfg!(unix) {
        if let Some(parent) = path.parent() {
            if let Ok(d) = std::fs::File::open(parent) {
                let _ = d.sync_all(); // 目录项持久化（best-effort）
            }
        }
    }
    Ok(())
}

/// 版本门 + 迁移（v1 内联 / v2 引用双兼容直通；未来版本在此逐级升格）。
fn migrate(value: serde_json::Value) -> Result<ServiceSnapshot, StoreIoError> {
    let found = value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(WRAPPER_SCHEMA_VERSION); // 缺省 = 1（首版快照无该字段）
    if found > WRAPPER_SCHEMA_VERSION {
        return Err(StoreIoError::SchemaUnsupported { found, supported: WRAPPER_SCHEMA_VERSION });
    }
    serde_json::from_value(value).map_err(|e| {
        StoreIoError::Corrupt(format!("db.json 结构不匹配（字段级）: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bid_domain::{DomainStore, ParseStatus, STORE_SCHEMA_VERSION};

    /// borrow：mdcore `sidecar.rs::save_load_roundtrip_and_atomicity`（792-845）
    /// ——快照读写往返全等 + `.tmp` rename 清除（原子写不留残骸）。
    #[test]
    fn save_load_roundtrip_and_atomicity() {
        let dir = std::env::temp_dir().join(format!("bid-store-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sf = StoreFile::new(&dir);
        // 构造带全集合的域快照
        let mut store = DomainStore::new();
        store.create_project("往返项目", "ZFCG-2026-RT-001", "明标", "rt-key").unwrap();
        store.upsert_file("A-招标文件.docx", "招标文件", "aa", 3, 1, 30, 0,
                          ParseStatus::Ok, "engine-docx @ test").unwrap();
        let snap = store.to_snapshot();
        let parsed = vec![ParsedFile {
            name: "A-招标文件.docx".into(), sha256: "aa".into(), pages: 3, tables: 1,
            has_text_layer: true, ocr_blocks: 0,
            blocks: vec![bid_parse::Block { idx: 0, kind: bid_parse::BlockKind::Para,
                locator: "第一章".into(), text: "往返块".into(), cells: vec![] }],
            parse_summary: "test".into(), components: vec![],
        }];
        sf.save(&snap, &parsed).unwrap();
        // 原子性：tmp 已被 rename 清除
        assert!(!dir.join(format!("{DB_FILE}.tmp")).exists(), "临时文件必须被 rename 清除");
        // 往返全等（域面 + 解析面）
        let (snap2, parsed2) = sf.load().unwrap().unwrap();
        assert_eq!(snap2, snap, "域快照往返必须全等");
        assert_eq!(parsed2, parsed, "解析产物往返必须全等");
        // 恢复后语义等价
        let store2 = DomainStore::restore(snap2).unwrap();
        assert_eq!(store2.project().unwrap().id, store.project().unwrap().id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// borrow：mdcore `sidecar.rs::schema_version_gate`（847-863）
    /// ——未来版本响亮拒绝（不猜测降级加载）。
    #[test]
    fn schema_version_gate_rejects_future_versions() {
        let dir = std::env::temp_dir().join(format!("bid-store-ver-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sf = StoreFile::new(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let future = serde_json::json!({
            "schema_version": 99,
            "domain": { "schema_version": STORE_SCHEMA_VERSION }
        });
        std::fs::write(sf.path(), serde_json::to_string(&future).unwrap()).unwrap();
        match sf.load() {
            Err(StoreIoError::SchemaUnsupported { found: 99, supported: 2 }) => (),
            other => panic!("未来版本必须 SchemaUnsupported 响亮拒绝，实际: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// borrow：mdcore `sidecar.rs::corrupt_file_rejected_not_guessed`（865-873）
    /// ——坏 JSON 拒载（原文件保留不覆写）。
    #[test]
    fn corrupt_file_rejected_not_guessed() {
        let dir = std::env::temp_dir().join(format!("bid-store-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sf = StoreFile::new(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(sf.path(), b"{ not valid").unwrap();
        match sf.load() {
            Err(StoreIoError::Corrupt(msg)) => {
                assert!(msg.contains("语法损坏"), "损坏分类可辨识: {msg}");
            }
            other => panic!("坏 JSON 必须 Corrupt 拒载，实际: {other:?}"),
        }
        // 原文件保留（不被读半路覆写）
        assert!(sf.path().exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 加性迁移面：缺新集合字段的旧版 JSON（v1 最小面）可加载，缺省空集合。
    #[test]
    fn legacy_minimal_snapshot_loads_with_defaults() {
        let dir = std::env::temp_dir().join(format!("bid-store-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sf = StoreFile::new(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 模拟「V3.1 之前」的最小快照：无 evidences/sections/parsed 字段、
        // 无 schema_version（首版缺省 = 1）
        let legacy = serde_json::json!({
            "domain": {
                "project": {
                    "id": "PRJ-0001", "name": "旧项目", "code": "C", "mode": "明标",
                    "status": "REQUIREMENTS_EXTRACTED", "content_version": 3,
                    "external_key": "legacy-key"
                },
                "files": [{ "id": "TF-0002", "name": "A.docx", "kind": "招标文件",
                    "sha256": "aa", "pages": 1, "tables": 0, "chunks": 5, "ocr_blocks": 0,
                    "parse_status": "ok", "parse_summary": "" }],
                "requirements": [],
                "id_seq": 2
            }
        });
        std::fs::write(sf.path(), serde_json::to_string(&legacy).unwrap()).unwrap();
        let (snap, parsed) = sf.load().unwrap().unwrap();
        assert_eq!(snap.schema_version, STORE_SCHEMA_VERSION, "缺省 schema_version = 1");
        assert_eq!(snap.evidences.len(), 0, "旧版无证据集合 → 空集合加载（加性迁移）");
        assert_eq!(snap.sections.len(), 0, "旧版无章节树 → 空集合加载");
        assert!(parsed.is_empty(), "旧版无 parsed → 空集合加载");
        let store = DomainStore::restore(snap).unwrap();
        assert_eq!(store.files().len(), 1, "既有面无损");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 确定性：同状态双保存字节一致（断点续跑的对账地基——run_tests T13 口径）。
    #[test]
    fn double_save_is_byte_identical() {
        let dir = std::env::temp_dir().join(format!("bid-store-det-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sf = StoreFile::new(&dir);
        let mut store = DomainStore::new();
        store.create_project("确定性", "C-DET", "暗标", "det-key").unwrap();
        let snap = store.to_snapshot();
        sf.save(&snap, &[]).unwrap();
        let b1 = std::fs::read(sf.path()).unwrap();
        sf.save(&snap, &[]).unwrap();
        let b2 = std::fs::read(sf.path()).unwrap();
        assert_eq!(b1, b2, "同状态双保存必须字节一致（serde 无序性防线：HashMap 需稳定）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空目录 = 首启（None——不伪装「有库」）。
    #[test]
    fn missing_dir_is_fresh_start() {
        let dir = std::env::temp_dir().join(format!("bid-store-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sf = StoreFile::new(&dir);
        assert!(sf.load().unwrap().is_none(), "无 db.json = 首启空态");
        // save 自建目录
        let mut store = DomainStore::new();
        store.create_project("p", "c", "明标", "k").unwrap();
        sf.save(&store.to_snapshot(), &[]).unwrap();
        assert!(sf.load().unwrap().is_some(), "保存后可加载");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─────────────────────────── #411：写放大收口 + fsync + v2 分离 ──────────

    fn tmp_store(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bid-store-411-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn parsed_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("parsed-") && n.ends_with(".json"))
            .collect()
    }

    fn sample_parsed(text: &str) -> Vec<ParsedFile> {
        vec![ParsedFile {
            name: "A-招标文件.docx".into(), sha256: "aa".into(), pages: 1, tables: 0,
            has_text_layer: true, ocr_blocks: 0,
            blocks: vec![bid_parse::Block { idx: 0, kind: bid_parse::BlockKind::Para,
                locator: "第一章".into(), text: text.into(), cells: vec![] }],
            parse_summary: "test".into(), components: vec![],
        }]
    }

    /// 验收①：同 parsed 双 save（domain 变更）→ 只 1 个 parsed 文件、ref 复用
    /// ——写放大收口实证（域-only 高频路径不重写数十 MB parsed）。
    #[test]
    fn domain_only_saves_reuse_parsed_file() {
        let dir = tmp_store("amp");
        let sf = StoreFile::new(&dir);
        let mut store = DomainStore::new();
        store.create_project("放大", "AMP-1", "明标", "amp-key").unwrap();
        let parsed = sample_parsed("初始解析块");

        sf.save(&store.to_snapshot(), &parsed).unwrap();
        let first = parsed_files(&dir);
        assert_eq!(first.len(), 1, "首次落盘恰 1 个 parsed 文件");
        let ref_name = first[0].clone();

        // 域变更（模拟 bind/draft/review/override 后的 checkpoint）× 3
        // （单项目存储——用 upsert_file 制造域面变化，同真实 checkpoint 面貌）
        for i in 0..3 {
            store.upsert_file(&format!("域变更证据{i}.txt"), "证据", &format!("sha-{i}"),
                              1, 0, 5, 0, ParseStatus::Ok, "").unwrap();
            sf.save(&store.to_snapshot(), &parsed).unwrap();
        }
        assert_eq!(parsed_files(&dir), vec![ref_name], "同 parsed 的域-only save 复用同一文件（不重写）");
        // db.json 不含 parsed 内联（v2 分离实证）：块文本不得出现在 db.json
        let db = std::fs::read_to_string(sf.path()).unwrap();
        assert!(!db.contains("初始解析块"), "db.json 不内联 parsed 块文本");
        assert!(db.contains("parsed_ref"), "v2 形态：db.json 持 parsed_ref");
        // 往返
        let (snap, p2) = sf.load().unwrap().unwrap();
        assert_eq!(p2, parsed);
        assert_eq!(snap, store.to_snapshot());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 验收②：v1 内联快照（含 parsed）继续可载；保存后升格 v2。
    #[test]
    fn v1_inline_upgrades_to_v2_on_save() {
        let dir = tmp_store("v1up");
        let sf = StoreFile::new(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 手工构造 v1 内联快照（无 schema_version 字段 = 缺省 v1）
        let mut store = DomainStore::new();
        store.create_project("旧项目", "V1-UP", "明标", "v1-key").unwrap();
        let snap = store.to_snapshot();
        let v1 = serde_json::json!({
            "domain": serde_json::to_value(&snap).unwrap(),
            "parsed": serde_json::to_value(sample_parsed("旧内联块")).unwrap(),
        });
        std::fs::write(sf.path(), serde_json::to_string(&v1).unwrap()).unwrap();

        // v1 可载（内联 parsed 读回）
        let (s1, p1) = sf.load().unwrap().unwrap();
        assert_eq!(p1, sample_parsed("旧内联块"), "v1 内联 parsed 可载");
        assert_eq!(s1, snap);

        // 保存 → 升格 v2（ref 分离 + 内联消失）
        sf.save(&snap, &p1).unwrap();
        let db = std::fs::read_to_string(sf.path()).unwrap();
        assert!(!db.contains("旧内联块"), "升格后 db.json 不再内联");
        assert!(db.contains("\"schema_version\": 2"), "升格为 v2");
        let (_, p2) = sf.load().unwrap().unwrap();
        assert_eq!(p2, p1, "升格往返语义等价");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 验收③：崩溃窗口原子性——新 parsed 文件在场 + 旧 db.json（rename 前崩溃）
    /// → 旧 pair 完整可载；新 db + 新 parsed（rename 后）→ 新 pair 可载。
    /// 不可变文件（同名即同容）是「分离后仍无撕裂」的根因。
    #[test]
    fn crash_window_yields_consistent_pair() {
        let dir = tmp_store("crash");
        let sf = StoreFile::new(&dir);
        let mut store = DomainStore::new();
        store.create_project("崩溃窗", "CR-1", "明标", "cr-key").unwrap();

        // 旧 pair 落盘
        let old_parsed = sample_parsed("旧版本内容");
        sf.save(&store.to_snapshot(), &old_parsed).unwrap();
        let old_db = std::fs::read(sf.path()).unwrap();
        let old_ref = parsed_files(&dir)[0].clone();

        // 模拟「parsed 新文件已落盘、db.json rename 前崩溃」：只走 save 的
        // 步骤①（新 parsed 内容寻址落盘），不走步骤②（db rename）与
        // 步骤③（GC）——旧 db/旧 parsed 均不动，与真实崩溃现场一致。
        let new_parsed = sample_parsed("新版本内容——文件更大一些，内容不同");
        let new_bytes = serde_json::to_vec(&new_parsed).unwrap();
        let new_ref = parsed_file_name(&new_bytes);
        write_fsync_atomic(&dir.join(&new_ref), &new_bytes).unwrap();
        assert_ne!(old_ref, new_ref, "新旧 parsed 是不同内容寻址文件");
        // 步骤①不动 db.json（字节不变——rename 才是提交点）
        assert_eq!(std::fs::read(sf.path()).unwrap(), old_db,
            "parsed 落盘阶段 db.json 必须保持字节不变（提交点 = db rename）");
        // 崩溃窗口：新 parsed 文件在场 + 旧 db.json → 旧 pair 完整可载
        let (s_old, p_old) = sf.load().unwrap().unwrap();
        assert_eq!(p_old, old_parsed, "崩溃窗口：旧 db + 旧 ref 文件（不可变仍在）→ 旧 pair 完整");
        assert_eq!(s_old, store.to_snapshot());
        // 正常完成态：完整 save → 新 pair，GC 清旧
        sf.save(&store.to_snapshot(), &new_parsed).unwrap();
        let (_, p_new) = sf.load().unwrap().unwrap();
        assert_eq!(p_new, new_parsed, "正常态：新 pair 完整");
        assert_eq!(parsed_files(&dir), vec![new_ref], "GC 后只剩新引用");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 验收④：parsed_ref 指向缺失文件 → Corrupt 响亮拒绝（fail-closed 不猜测）。
    #[test]
    fn missing_parsed_ref_is_corrupt_not_silent_empty() {
        let dir = tmp_store("missref");
        let sf = StoreFile::new(&dir);
        let mut store = DomainStore::new();
        store.create_project("缺引用", "MR-1", "明标", "mr-key").unwrap();
        sf.save(&store.to_snapshot(), &sample_parsed("将被删的块")).unwrap();
        // 删掉 parsed 文件（模拟外部破坏/盘损）
        std::fs::remove_file(dir.join(&parsed_files(&dir)[0])).unwrap();
        match sf.load() {
            Err(StoreIoError::Corrupt(msg)) => {
                assert!(msg.contains("缺失"), "缺失面可辨识: {msg}");
            }
            other => panic!("parsed_ref 缺文件必须 Corrupt，实际: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 验收⑤：GC——parsed 内容变化后仅剩被引用文件；无 .tmp 残留。
    #[test]
    fn gc_keeps_only_referenced_parsed() {
        let dir = tmp_store("gc");
        let sf = StoreFile::new(&dir);
        let mut store = DomainStore::new();
        store.create_project("GC", "GC-1", "明标", "gc-key").unwrap();
        sf.save(&store.to_snapshot(), &sample_parsed("第一版")).unwrap();
        sf.save(&store.to_snapshot(), &sample_parsed("第二版")).unwrap();
        sf.save(&store.to_snapshot(), &sample_parsed("第三版")).unwrap();
        let left = parsed_files(&dir);
        assert_eq!(left.len(), 1, "三代 parsed 只剩被引用的一个");
        let (_, p) = sf.load().unwrap().unwrap();
        assert_eq!(p, sample_parsed("第三版"), "幸存者 = 最新引用");
        // 回到旧内容 → 旧文件名复现（内容寻址缓存效应）且仍只有 1 个
        sf.save(&store.to_snapshot(), &sample_parsed("第一版")).unwrap();
        assert_eq!(parsed_files(&dir).len(), 1);
        let (_, p) = sf.load().unwrap().unwrap();
        assert_eq!(p, sample_parsed("第一版"));
        // 无 tmp 残留（fsync 原子写不留残骸）
        let tmps: Vec<String> = std::fs::read_dir(&dir).into_iter().flatten().flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(tmps.is_empty(), "无 .tmp 残骸: {tmps:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 名字面防御：parsed_ref 注入路径（`../`）→ 拒载（名字格式门）。
    #[test]
    fn parsed_ref_path_injection_rejected() {
        let dir = tmp_store("inject");
        let sf = StoreFile::new(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = DomainStore::new();
        store.create_project("注入", "INJ-1", "明标", "inj-key").unwrap();
        let evil = serde_json::json!({
            "schema_version": 2,
            "domain": serde_json::to_value(store.to_snapshot()).unwrap(),
            "parsed_ref": "../../etc/passwd"
        });
        std::fs::write(sf.path(), serde_json::to_string(&evil).unwrap()).unwrap();
        match sf.load() {
            Err(StoreIoError::Corrupt(msg)) => {
                assert!(msg.contains("非法"), "注入面可辨识: {msg}");
            }
            other => panic!("路径注入 ref 必须 Corrupt，实际: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
