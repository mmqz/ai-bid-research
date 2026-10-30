//! mang 版本快照核心（issue #42 阶段 7/10）：`.mang/history/` 写端**单一真源**。
//!
//! 两个消费方共用本 crate，index.json 格式只有一个实现：
//!   * `mang-app`（桌面壳）：IPC `snapshots.check/save/read/rollback` 的薄适配
//!   * `bid-mcp`（AI 通道）：`save_version` 工具（skill 纪律「每完成一个章节
//!     调用 save_version 存版本」的落点）
//!
//! 读端（`history.list`）在 mang-app 的 `recent::history_list`——本 crate 只管写。
//!
//! 布局（issue #42 §八）：
//! ```text
//! <root>/.mang/history/index.json                  元数据（数组形态）
//! <root>/.mang/history/<key>/<ts>-<seq>.md         快照正文
//! ```
//!
//! 防线：只快照文本（.md/.txt/.markdown——源头是 md，二进制不做）；
//! 8MB 上限；每文件 200 份剪枝（auto 优先淘汰）；`.tmp`+rename 原子写；
//! index 读改写全程持锁；同毫秒两笔保存不互覆（原子序号）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

/// 每个文件最多保留多少份快照（30s 一档的自动快照，200 份够回看一整天）
pub const PER_FILE_MAX: usize = 200;
/// 快照正文超过 8MB 不存（防御异常大文件把 history 撑爆）
pub const MAX_CONTENT: u64 = 8 * 1024 * 1024;
/// key 消毒后的名字段上限（Windows 路径长度防线）
const KEY_STEM_MAX: usize = 60;

static INDEX_LOCK: Mutex<()> = Mutex::new(());
/// 快照文件名的去重序号：同名同毫秒的两笔保存（如 check 后紧跟手动 save）不得互相覆盖
static NAME_SEQ: AtomicU64 = AtomicU64::new(0);

/// 错误码固定集合（消费方映射到各自的 IPC/工具错误类型）
#[derive(Debug, Clone)]
pub struct SnapError {
    pub code: &'static str,
    pub message: String,
}

impl SnapError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

/// 快照种类（写入 index 的 kind 字段；读端按此分徽章）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Auto,
    Manual,
    Ai,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Auto => "auto",
            Kind::Manual => "manual",
            Kind::Ai => "ai",
        }
    }
}

/// 一笔已存快照
#[derive(Debug, Clone)]
pub struct SavedEntry {
    pub file: String,
    pub ts: u64,
    pub snapshot: String,
    pub bytes: usize,
}

/// check 的单文件结论
#[derive(Debug, Clone)]
pub enum FileOutcome {
    Saved(SavedEntry),
    Skipped { file: String, reason: String },
}

/// 回滚结果
#[derive(Debug, Clone)]
pub struct RollbackOutcome {
    pub file: String,
    pub bytes: usize,
    /// 回滚前自动备份的 ts（0 = 目标文件原本为空，没备份）
    pub backup_ts: u64,
}

/// 只有文本文件入快照（issue #42 §八：源头是 md，二进制不做）
pub fn is_snapshotable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("md") | Some("markdown") | Some("txt")
    )
}

/// MCP 侧的工作区根发现：从 file 向上找最近的 `.mang` 目录；找不到用文件父目录。
/// （桌面壳不走这里——它有自己的 --workspace）
pub fn find_root(file: &Path) -> PathBuf {
    let mut dir = file.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    loop {
        if dir.join(".mang").is_dir() {
            return dir;
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => return file.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")),
        }
    }
}

fn rel_of(root: &Path, abs: &Path) -> Result<String, SnapError> {
    let c = std::fs::canonicalize(abs)
        .map_err(|e| SnapError::new("io", format!("路径不存在：{}（{e}）", abs.display())))?;
    let rc = std::fs::canonicalize(root)
        .map_err(|e| SnapError::new("io", format!("工作区不可用：{}（{e}）", root.display())))?;
    let rel = c
        .strip_prefix(&rc)
        .map_err(|_| SnapError::new("denied", format!("路径不在工作区内：{}", abs.display())))?;
    let joined = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    Ok(if joined.is_empty() { rel.to_string_lossy().replace('\\', "/") } else { joined })
}

/// FNV-1a 64（防同名撞 key / 生成文件名随机段）
fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// rel 路径 → 快照目录 key：`docs/技术标.md` → `docs__技术标-f1e2d3c4`
pub fn file_key(rel: &str) -> String {
    let flat = rel.replace(['/', '\\'], "__");
    let stem: String = flat
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' || c == '(' || c == ')' || c == ' ' { c } else { '_' })
        .collect();
    let stem = stem.trim_matches(|c: char| c == '.' || c == ' ');
    let clamped: String = if stem.chars().count() > KEY_STEM_MAX {
        stem.chars().take(KEY_STEM_MAX).collect()
    } else {
        stem.to_string()
    };
    format!("{clamped}-{:08x}", (fnv64(rel.as_bytes()) >> 32) as u32)
}

/// epoch ms → UTC `yyyymmdd-HHMMSS`（文件名用；民用历换算，Hinnant 算法）
fn ts_label(ts: u64) -> String {
    let secs = (ts / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", sod / 3600, (sod % 3600) / 60, sod % 60)
}

fn history_root(root: &Path) -> PathBuf {
    root.join(".mang").join("history")
}

/// 快照正文文件名：`<UTC 时间戳>-<序号>.md`（时间戳可读排序，序号防同毫秒碰撞）
fn snapshot_name(rel: &str, ts: u64) -> String {
    let seq = NAME_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut h = fnv64(rel.as_bytes()) ^ ts.rotate_left(17) ^ seq.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = (h ^ (h >> 31)).wrapping_mul(0x94D0_49BB_1331_11EB);
    format!("{}-{:06x}.md", ts_label(ts), (h & 0xff_ffff) as u32)
}

/// index.json → items 数组（坏文件按空处理，绝不向上抛）
fn load_index(root: &Path) -> Vec<Value> {
    let Ok(bytes) = std::fs::read(history_root(root).join("index.json")) else {
        return Vec::new();
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(o)) => o
            .get("items")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default(),
        Ok(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

/// 原子写：`.tmp` + rename（Windows 上 rename 目标存在时失败 → 先删目标）
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), SnapError> {
    let tmp = path.with_extension("tmp");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| SnapError::new("io", format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&tmp, bytes).map_err(|e| SnapError::new("io", format!("{}: {e}", tmp.display())))?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path).map_err(|e| SnapError::new("io", format!("{}: {e}", path.display())))?;
    Ok(())
}

fn entry_file(e: &Value) -> String {
    e.get("file").and_then(|f| f.as_str()).unwrap_or("").replace('\\', "/")
}

fn entry_ts(e: &Value) -> u64 {
    e.get("ts").and_then(|t| t.as_u64()).unwrap_or(0)
}

/// 存一份快照并登记 index（**锁外**入口；锁内版本见 save_locked）。
/// 调用方保证 abs 已解析（canonical）且在工作区内——本函数再做一次围栏校验。
pub fn save(root: &Path, abs: &Path, kind: Kind, message: &str) -> Result<SavedEntry, SnapError> {
    if !is_snapshotable(abs) {
        return Err(SnapError::new(
            "unsupported",
            format!("只快照文本文件（.md/.txt/.markdown）：{}", abs.display()),
        ));
    }
    let rel = rel_of(root, abs)?;
    let cur = std::fs::read(abs).map_err(|e| SnapError::new("io", format!("{}: {e}", abs.display())))?;
    if cur.len() as u64 > MAX_CONTENT {
        return Err(SnapError::new("too_large", "内容超过 8MB，不存快照"));
    }
    let _guard = INDEX_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    save_locked(root, &rel, Some(&cur), kind, message)
}

/// 锁内实现（rollback 在持锁路径复用）
fn save_locked(root: &Path, rel: &str, content: Option<&[u8]>, kind: Kind, message: &str) -> Result<SavedEntry, SnapError> {
    let hroot = history_root(root);
    let key = file_key(rel);
    let bytes = content.unwrap_or_default();
    let ts = now_ms();
    let name = snapshot_name(rel, ts);
    let mut items = load_index(root);

    let snap_file = hroot.join(&key).join(&name);
    if let Some(b) = content {
        if b.is_empty() {
            return Err(SnapError::new("empty", "内容为空，不存快照"));
        }
        atomic_write(&snap_file, b)?;
    }

    let snapshot_rel = format!("{key}/{name}");
    items.push(json!({
        "file": rel,
        "ts": ts,
        "kind": kind.as_str(),
        "message": message,
        "bytes": bytes.len(),
        "snapshot": snapshot_rel,
    }));
    prune_per_file(root, &mut items);

    atomic_write(
        &hroot.join("index.json"),
        serde_json::to_vec_pretty(&json!({ "version": 1, "items": items })).unwrap_or_default().as_slice(),
    )?;
    Ok(SavedEntry { file: rel.to_string(), ts, snapshot: snapshot_rel, bytes: bytes.len() })
}

/// 每文件保留 PER_FILE_MAX 份；超限时按「auto 优先、时间靠前优先」淘汰，并删正文文件。
fn prune_per_file(root: &Path, items: &mut Vec<Value>) {
    use std::collections::HashMap;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for e in items.iter() {
        *counts.entry(entry_file(e)).or_default() += 1;
    }
    let mut doomed: Vec<String> = Vec::new();
    for (file, n) in counts {
        if n <= PER_FILE_MAX {
            continue;
        }
        let excess = n - PER_FILE_MAX;
        // 元组升序排序 false < true：首元用 is_manual（auto → false 排前面，先被淘汰）
        let mut mine: Vec<(bool, u64, usize)> = items
            .iter()
            .enumerate()
            .filter(|(_, e)| entry_file(e) == file)
            .map(|(i, e)| (e.get("kind").and_then(|k| k.as_str()) != Some("auto"), entry_ts(e), i))
            .collect();
        mine.sort();
        for k in 0..excess {
            let (_, _, idx) = mine[k];
            if let Some(s) = items[idx].get("snapshot").and_then(|s| s.as_str()) {
                doomed.push(s.to_string());
            }
            items[idx] = Value::Null;
        }
    }
    if !doomed.is_empty() {
        items.retain(|e| !e.is_null());
        for snap in doomed {
            let p = history_root(root).join(&snap);
            if p.is_file() {
                let _ = std::fs::remove_file(p); // 删失败不阻塞（孤儿文件无害）
            }
        }
    }
}

/// 批量检查：每文件与最新快照逐字节比对，有变化才存 auto 快照（spec：
/// 「每 30 秒检查一次：如果文件内容有变化 → 存一份快照」）。
/// paths 里是**已解析的绝对路径**；不在工作区内/非文本/读不到 → Skipped（不炸批次）。
pub fn check(root: &Path, paths: &[PathBuf]) -> Vec<FileOutcome> {
    let mut out = Vec::with_capacity(paths.len());
    for abs in paths {
        let rel = match rel_of(root, abs) {
            Ok(r) => r,
            Err(e) => {
                out.push(FileOutcome::Skipped { file: abs.display().to_string(), reason: e.message });
                continue;
            }
        };
        if !is_snapshotable(abs) {
            out.push(FileOutcome::Skipped { file: rel, reason: "只快照文本文件（.md/.txt/.markdown）".to_string() });
            continue;
        }
        let cur = match std::fs::read(abs) {
            Ok(b) => b,
            Err(_) => {
                out.push(FileOutcome::Skipped { file: rel, reason: "读取失败（可能已关闭/删除）".to_string() });
                continue;
            }
        };
        if cur.len() as u64 > MAX_CONTENT {
            out.push(FileOutcome::Skipped { file: rel, reason: "超过 8MB".to_string() });
            continue;
        }
        // 最新快照内容一致 → 跳过
        let _guard = INDEX_LOCK.lock().unwrap_or_else(|g| g.into_inner());
        let items = load_index(root);
        let latest = items
            .iter()
            .filter(|e| entry_file(e) == rel && e.get("snapshot").is_some())
            .max_by_key(|e| entry_ts(e));
        let same = latest
            .and_then(|e| e.get("snapshot").and_then(|s| s.as_str()))
            .and_then(|s| std::fs::read(history_root(root).join(s)).ok())
            .is_some_and(|prev| prev == cur);
        if same {
            out.push(FileOutcome::Skipped { file: rel, reason: "无变化".to_string() });
            continue;
        }
        match save_locked(root, &rel, Some(&cur), Kind::Auto, "") {
            Ok(e) => out.push(FileOutcome::Saved(e)),
            Err(err) => out.push(FileOutcome::Skipped { file: rel, reason: err.message }),
        }
    }
    out
}

/// 读一份快照正文（预览）。snapshot 必须登记在 index 里且属于该 file——防目录穿越直读任意文件。
pub fn read(root: &Path, rel_file: &str, snapshot: &str) -> Result<String, SnapError> {
    let rel_file = rel_file.replace('\\', "/");
    let _guard = INDEX_LOCK.lock().unwrap_or_else(|g| g.into_inner());
    let items = load_index(root);
    let ok = items.iter().any(|e| entry_file(e) == rel_file && e.get("snapshot").and_then(|s| s.as_str()) == Some(snapshot));
    if !ok {
        return Err(SnapError::new("denied", "快照不存在或不属于该文件"));
    }
    let p = history_root(root).join(snapshot);
    let bytes = std::fs::read(&p).map_err(|e| SnapError::new("io", format!("{}: {e}", p.display())))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 回滚：安全序 = 先把「当前内容」存成回滚前备份（可再回滚回来），再覆盖目标文件。
pub fn rollback(root: &Path, abs: &Path, rel_file: &str, snapshot: &str) -> Result<RollbackOutcome, SnapError> {
    let rel_file = rel_file.replace('\\', "/");
    if !is_snapshotable(abs) {
        return Err(SnapError::new("unsupported", "只对文本文件回滚"));
    }
    let rel_check = rel_of(root, abs)?;
    if rel_check != rel_file {
        return Err(SnapError::new("denied", "file 与目标路径不一致"));
    }

    let _guard = INDEX_LOCK.lock().unwrap_or_else(|g| g.into_inner());
    let items = load_index(root);
    let ok = items.iter().any(|e| entry_file(e) == rel_file && e.get("snapshot").and_then(|s| s.as_str()) == Some(snapshot));
    if !ok {
        return Err(SnapError::new("denied", "快照不存在或不属于该文件"));
    }
    let src = history_root(root).join(snapshot);
    let content = std::fs::read(&src).map_err(|e| SnapError::new("io", format!("{}: {e}", src.display())))?;

    let backup_ts = match std::fs::read(abs) {
        Ok(cur) if !cur.is_empty() => save_locked(root, &rel_file, Some(&cur), Kind::Auto, "回滚前自动备份")?.ts,
        _ => 0,
    };
    atomic_write(abs, &content)?;
    Ok(RollbackOutcome { file: rel_file, bytes: content.len(), backup_ts: backup_ts })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpws(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mang-snap-core-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn check_saves_on_change_and_skips_when_same() {
        let d = tmpws("check");
        std::fs::write(d.join("a.md"), "v1").unwrap();
        let out = check(&d, &[d.join("a.md")]);
        assert!(matches!(out[0], FileOutcome::Saved(_)));
        let out = check(&d, &[d.join("a.md")]);
        match &out[0] {
            FileOutcome::Skipped { reason, .. } => assert_eq!(reason, "无变化"),
            _ => panic!("应跳过"),
        }
        std::fs::write(d.join("a.md"), "v2").unwrap();
        let out = check(&d, &[d.join("a.md")]);
        assert!(matches!(out[0], FileOutcome::Saved(_)));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn ai_kind_survives_roundtrip() {
        let d = tmpws("kind");
        std::fs::write(d.join("b.md"), "第一章").unwrap();
        check(&d, &[d.join("b.md")]);
        std::fs::write(d.join("b.md"), "第二章完成").unwrap();
        let saved = save(&d, &d.join("b.md"), Kind::Ai, "第二章完成").unwrap();
        assert!(saved.ts > 0);
        let items = load_index(&d);
        let ai = items.iter().find(|x| x["kind"] == json!("ai")).expect("kind=ai 原样入索引");
        assert_eq!(ai["message"], json!("第二章完成"));
        // 读旧版
        let old = items.iter().find(|x| x["kind"] == json!("auto")).unwrap();
        let snap = old["snapshot"].as_str().unwrap();
        assert_eq!(read(&d, "b.md", snap).unwrap(), "第一章");
        // 回滚 + 备份
        let rb = rollback(&d, &d.join("b.md"), "b.md", snap).unwrap();
        assert!(rb.backup_ts > 0);
        assert_eq!(std::fs::read_to_string(d.join("b.md")).unwrap(), "第一章");
        // 越权读
        assert!(read(&d, "别的.md", snap).is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn binary_rejected_and_root_discovery() {
        let d = tmpws("bin");
        std::fs::write(d.join("x.xlsx"), "bin").unwrap();
        assert!(save(&d, &d.join("x.xlsx"), Kind::Manual, "").is_err());
        // find_root：有 .mang 的祖先优先
        std::fs::create_dir_all(d.join(".mang")).unwrap();
        std::fs::create_dir_all(d.join("sub/deep")).unwrap();
        assert_eq!(find_root(&d.join("sub/deep/a.md")), d.canonicalize().unwrap());
        // 没有 .mang：用父目录
        let d2 = tmpws("noroot");
        std::fs::create_dir_all(d2.join("sub")).unwrap();
        assert_eq!(find_root(&d2.join("sub/a.md")), d2.join("sub"));
        let _ = std::fs::remove_dir_all(d);
        let _ = std::fs::remove_dir_all(d2);
    }

    #[test]
    fn key_and_label() {
        assert_eq!(file_key("docs/技术标.md"), file_key("docs/技术标.md"));
        assert_ne!(file_key("a/x.md"), file_key("b/x.md"));
        assert_eq!(ts_label(0), "19700101-000000");
    }
}
