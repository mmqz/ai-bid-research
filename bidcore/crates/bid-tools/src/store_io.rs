//! store_io.rs —— T-8 服务态持久化磁盘层（issue #11 批次 v3 / V3-1）。
//!
//! 定位：`DomainStore` 快照（bid-domain 纯函数面）↔ 磁盘的单桥。crate 纪律
//! 「bid-domain 零 IO」不变——本模块住 bid-tools（服务层），与 runner.rs 的
//! write_run 平级。
//!
//! 原子性（borrow：mdcore `sidecar.rs::SidecarStore::save` + 本仓 export_gate
//! tmp+rename 纪律）：**单一 `db.json` 包装文件**承载域快照 + 解析产物——
//! 一次 `write(tmp) → rename` 即提交，无双文件撕裂窗口。写序：tmp 写全量 →
//! rename 替换（POSIX 原子；Windows 上 std::fs::rename 语义同为替换式）。
//!
//! 版本门（borrow：mdcore `schema_version_gate` 测试口径）：
//! - `schema_version` 缺省 = 1（首版快照无该字段——加性迁移的向后兼容面）；
//! - 高于当前支持版本 → `SchemaUnsupported` **响亮拒绝**（不猜测降级加载）；
//! - 未来结构性变更 → 版本号 +1 + `migrate` 逐级升格（本版本仅 v1，直通）。
//!
//! 损坏语义（borrow：mdcore `corrupt_file_rejected_not_guessed`）：坏 JSON →
//! `Corrupt`（带 serde 行列定位），原文件保留不覆写——fail-closed 不猜测。

use bid_domain::StoreSnapshot;
use bid_parse::ParsedFile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 服务态数据库文件名（store 目录内）。
pub const DB_FILE: &str = "db.json";

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

/// 磁盘上的服务态全量（单文件 = 单次原子提交）。
///
/// 嵌套面：
/// - `domain`：[`StoreSnapshot`]（schema_version 自带——域级版本独立演进）；
/// - `parsed`：解析产物（`extract_requirements` 的输入面——重启续跑必须随行；
///   体量边界：S-6 组装导出批次再上分片存储，本版单文件承载）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ServiceSnapshot {
    #[serde(default = "wrapper_default_version")]
    schema_version: u32,
    domain: StoreSnapshot,
    #[serde(default)]
    parsed: Vec<ParsedFile>,
}

const WRAPPER_SCHEMA_VERSION: u32 = 1;

fn wrapper_default_version() -> u32 {
    WRAPPER_SCHEMA_VERSION
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
    pub fn load(&self) -> Result<Option<(StoreSnapshot, Vec<ParsedFile>)>, StoreIoError> {
        let path = self.path();
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path).map_err(StoreIoError::Io)?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| StoreIoError::Corrupt(format!("db.json 语法损坏: {e}")))?;
        let svc: ServiceSnapshot = migrate(value)?;
        Ok(Some((svc.domain, svc.parsed)))
    }

    /// 保存服务态全量（tmp + rename 原子提交；失败时原文件不动）。
    pub fn save(&self, domain: &StoreSnapshot, parsed: &[ParsedFile]) -> Result<(), StoreIoError> {
        let svc = ServiceSnapshot {
            schema_version: WRAPPER_SCHEMA_VERSION,
            domain: domain.clone(),
            parsed: parsed.to_vec(),
        };
        let json = serde_json::to_string_pretty(&svc).map_err(StoreIoError::Serialize)?;
        std::fs::create_dir_all(&self.dir).map_err(StoreIoError::Io)?;
        let tmp = self.dir.join(format!("{DB_FILE}.tmp"));
        std::fs::write(&tmp, json.as_bytes()).map_err(StoreIoError::Io)?;
        std::fs::rename(&tmp, self.path()).map_err(StoreIoError::Io)?; // 原子提交点
        Ok(())
    }
}

/// 版本门 + 迁移（v1 直通；未来版本在此逐级升格）。
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
            Err(StoreIoError::SchemaUnsupported { found: 99, supported: 1 }) => (),
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
}
