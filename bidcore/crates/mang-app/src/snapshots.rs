//! 版本快照（issue #42 阶段 7）：`.mang/history/` 写端。
//!
//! 读端已有（`recent::history_list` 读 index.json），本模块补上生产侧：
//!   * `snapshots.check`   —— UI 每 30s 调一次：打开的文本文件内容有变 → 存快照
//!   * `snapshots.save`    —— 手动/AI 存版本（阶段 10 的 MCP save_version 落到这里）
//!   * `snapshots.read`    —— 预览某份快照内容
//!   * `snapshots.rollback`—— 回滚（先把当前内容存成「回滚前备份」，回滚本身可撤销）
//!
//! 布局（issue #42 §八）：
//! ```text
//! <workspace>/.mang/history/index.json                  元数据（数组形态）
//! <workspace>/.mang/history/<key>/<ts>-<rand>.md        快照正文
//! ```
//! `<key>` = rel 路径消毒（`/`→`__`）+ FNV-8hex 防撞（同名文件在不同子目录）。
//!
//! 纪律：只快照文本（.md/.txt/.markdown——issue 明确「源头是 md」，二进制不做）；
//! 写盘一律 `.tmp` + rename；index 读改写全程持锁（多 tab 并发 check）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::ipc::{now_ms, Args, IpcError};
use crate::paths::{self, Workspace};

/// 每个文件最多保留多少份快照（30s 一档的自动快照，200 份够回看一整天）
const PER_FILE_MAX: usize = 200;
/// 快照正文超过 8MB 不存（防御异常大文件把 history 撑爆）
const MAX_CONTENT: u64 = 8 * 1024 * 1024;
/// key 消毒后的名字段上限（Windows 路径长度防线）
const KEY_STEM_MAX: usize = 60;

static INDEX_LOCK: Mutex<()> = Mutex::new(());
/// 快照文件名的去重序号：同名同毫秒的两笔保存（如 check 后紧跟手动 save）不得互相覆盖
static NAME_SEQ: AtomicU64 = AtomicU64::new(0);

/// 快照正文文件名：`<UTC 时间戳>-<序号>.md`（时间戳可读排序，序号防同毫秒碰撞）
fn snapshot_name(rel: &str, ts: u64) -> String {
    let seq = NAME_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut h = fnv64(rel.as_bytes()) ^ ts.rotate_left(17) ^ seq.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = (h ^ (h >> 31)).wrapping_mul(0x94D0_49BB_1331_11EB);
    format!("{}-{:06x}.md", ts_label(ts), (h & 0xff_ffff) as u32)
}

fn is_snapshotable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("md") | Some("markdown") | Some("txt")
    )
}

/// FNV-1a 64 → 8 位 hex（防同名撞 key / 生成文件名随机段）
fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// rel 路径 → 快照目录 key：`docs/技术标.md` → `docs__技术标-f1e2d3c4`
fn key_for(rel: &str) -> String {
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
    // 1970-01-01 起算的民用历
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
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

fn history_root(ws: &Workspace) -> PathBuf {
    ws.sub(".mang").join("history")
}

fn snapshot_path(ws: &Workspace, key: &str, name: &str) -> PathBuf {
    history_root(ws).join(key).join(name)
}

/// index.json → items 数组（坏文件按空处理，绝不向上抛）
fn load_index(root: &Path) -> Vec<Value> {
    let Ok(bytes) = std::fs::read(root.join("index.json")) else {
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
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), IpcError> {
    let tmp = path.with_extension("tmp");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| paths::map_io(dir, &e))?;
    }
    std::fs::write(&tmp, bytes).map_err(|e| paths::map_io(&tmp, &e))?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path).map_err(|e| paths::map_io(path, &e))?;
    Ok(())
}

/// 条目规整（读到的旧条目可能缺字段）
fn entry_file(e: &Value) -> String {
    e.get("file").and_then(|f| f.as_str()).unwrap_or("").replace('\\', "/")
}

fn entry_ts(e: &Value) -> u64 {
    e.get("ts").and_then(|t| t.as_u64()).unwrap_or(0)
}

/// 存一份快照并登记 index（持锁；调用方保证 rel 已 fence）。
/// 返回新条目。内容为 None 时只登记不写正文（外部已写好正文的场景不用）。
fn save_snapshot(
    ws: &Workspace,
    rel: &str,
    content: Option<&[u8]>,
    kind: &str,
    message: &str,
) -> Result<Value, IpcError> {
    let root = history_root(ws);
    let key = key_for(rel);
    let bytes = content.unwrap_or_default();
    let ts = now_ms();
    let name = snapshot_name(rel, ts);

    let _guard = INDEX_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut items = load_index(&root);

    // 写正文
    let snap_file = snapshot_path(ws, &key, &name);
    if let Some(b) = content {
        if b.is_empty() {
            return Err(IpcError::new("empty", "内容为空，不存快照"));
        }
        atomic_write(&snap_file, b)?;
    }

    let entry = json!({
        "file": rel,
        "ts": ts,
        "kind": match kind { "ai" | "manual" => kind, _ => "auto" },
        "message": message,
        "bytes": bytes.len(),
        "snapshot": format!("{key}/{name}"),
    });
    items.push(entry.clone());

    // 剪枝：单文件超上限时先淘汰最老的 auto（manual/AI 版本尽量保住）
    prune_per_file(ws, &mut items);

    atomic_write(
        &root.join("index.json"),
        serde_json::to_vec_pretty(&json!({ "version": 1, "items": items })).unwrap_or_default().as_slice(),
    )?;
    Ok(entry)
}

/// 每文件保留 PER_FILE_MAX 份；超限时按「auto 优先、时间靠前优先」淘汰，并删正文文件。
fn prune_per_file(ws: &Workspace, items: &mut Vec<Value>) {
    use std::collections::HashMap;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for e in items.iter() {
        *counts.entry(entry_file(e)).or_default() += 1;
    }
    let mut doomed: Vec<(String, String)> = Vec::new(); // (file, snapshot)
    for (file, n) in counts {
        if n <= PER_FILE_MAX {
            continue;
        }
        let excess = n - PER_FILE_MAX;
        // 该文件的全部条目：auto 在前（更可丢），同 kind 内 ts 小的在前。
        // 注：元组升序排序 false < true，所以首元用 is_manual（auto → false 排前面）。
        let mut mine: Vec<(bool, u64, usize)> = items
            .iter()
            .enumerate()
            .filter(|(_, e)| entry_file(e) == file)
            .map(|(i, e)| {
                (
                    e.get("kind").and_then(|k| k.as_str()) != Some("auto"), // auto=false 排前，先被淘汰
                    entry_ts(e),
                    i,
                )
            })
            .collect();
        mine.sort();
        for k in 0..excess {
            let (_, _, idx) = mine[k];
            if let Some(s) = items[idx].get("snapshot").and_then(|s| s.as_str()) {
                doomed.push((file.clone(), s.to_string()));
            }
            items[idx] = Value::Null; // 标记删除
        }
    }
    if !doomed.is_empty() {
        items.retain(|e| !e.is_null());
        for (_, snap) in doomed {
            let p = history_root(ws).join(&snap);
            if p.is_file() {
                let _ = std::fs::remove_file(p); // 删失败不阻塞（孤儿文件无害）
            }
        }
    }
}

/// 路径参数 → (canonical 绝对路径, workspace rel)。绝不在工作区外快照。
fn resolve_file(ws: &Workspace, raw: &str) -> Result<(PathBuf, String), IpcError> {
    let abs = ws.resolve(raw, false)?;
    let rel = ws
        .rel_of(&abs)
        .ok_or_else(|| IpcError::denied(format!("路径不在工作区内：{raw}")))?;
    if !is_snapshotable(&abs) {
        return Err(IpcError::new(
            "unsupported",
            format!("只快照文本文件（.md/.txt/.markdown）：{raw}"),
        ));
    }
    Ok((abs, rel))
}

/// `snapshots.check { paths: [abs 或 rel, ...] }`
/// 对每个文件：读当前内容，与该文件最新一份快照不同 → 存 auto 快照。
/// 返回 { saved: [{file, ts}], skipped: [{file, reason}] }
pub fn check(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let Some(paths) = args.map.get("paths").and_then(|p| p.as_array()) else {
        return Err(IpcError::new("bad_args", "缺少 paths 数组"));
    };
    let root = history_root(ws);
    let mut saved: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for p in paths.iter().filter_map(|p| p.as_str()) {
        let (abs, rel) = match resolve_file(ws, p) {
            Ok(v) => v,
            Err(e) => {
                skipped.push(json!({ "file": p, "reason": e.message }));
                continue;
            }
        };
        let meta = match std::fs::metadata(&abs) {
            Ok(m) => m,
            Err(_) => {
                skipped.push(json!({ "file": rel, "reason": "读不到（可能已关闭/删除）" }));
                continue;
            }
        };
        if !meta.is_file() || meta.len() > MAX_CONTENT {
            skipped.push(json!({ "file": rel, "reason": "非普通文件或超过 8MB" }));
            continue;
        }
        let cur = match std::fs::read(&abs) {
            Ok(b) => b,
            Err(_) => {
                skipped.push(json!({ "file": rel, "reason": "读取失败" }));
                continue;
            }
        };
        // 找该文件最新快照，内容一致 → 跳过（spec：「如果文件内容有变化 → 存」）
        let _guard = INDEX_LOCK.lock().unwrap_or_else(|g| g.into_inner());
        let items = load_index(&root);
        let latest = items
            .iter()
            .filter(|e| entry_file(e) == rel && e.get("snapshot").is_some())
            .max_by_key(|e| entry_ts(e));
        let same = latest
            .and_then(|e| e.get("snapshot").and_then(|s| s.as_str()))
            .and_then(|s| std::fs::read(history_root(ws).join(s)).ok())
            .is_some_and(|prev| prev == cur);
        drop(_guard);
        if same {
            skipped.push(json!({ "file": rel, "reason": "无变化" }));
            continue;
        }
        match save_snapshot(ws, &rel, Some(&cur), "auto", "") {
            Ok(e) => saved.push(json!({ "file": rel, "ts": e["ts"] })),
            Err(err) => skipped.push(json!({ "file": rel, "reason": err.message })),
        }
    }
    Ok(json!({ "saved": saved, "skipped": skipped }))
}

/// `snapshots.save { path, message?, kind? }` —— 手动 / AI 存版本（阶段 10 MCP 落点）
pub fn save(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.req_str("path")?;
    let message = args.map.get("message").and_then(|m| m.as_str()).unwrap_or("").trim();
    let kind = match args.map.get("kind").and_then(|k| k.as_str()) {
        Some("ai") => "ai",
        Some("manual") => "manual",
        _ => "manual",
    };
    let (abs, rel) = resolve_file(ws, raw)?;
    let cur = std::fs::read(&abs).map_err(|e| paths::map_io(&abs, &e))?;
    if cur.len() as u64 > MAX_CONTENT {
        return Err(IpcError::new("too_large", "内容超过 8MB，不存快照"));
    }
    let e = save_snapshot(ws, &rel, Some(&cur), kind, message)?;
    Ok(json!({ "ok": true, "file": rel, "ts": e["ts"], "snapshot": e["snapshot"] }))
}

/// `snapshots.read { file, snapshot }` → { content }（预览某版本）
pub fn read(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let rel = args.req_str("file")?;
    let snap = args.req_str("snapshot")?;
    // snapshot 必须登记在 index 里且属于该 file（防目录穿越直读任意文件）
    let _guard = INDEX_LOCK.lock().unwrap_or_else(|g| g.into_inner());
    let items = load_index(&history_root(ws));
    let ok = items.iter().any(|e| {
        entry_file(e) == rel.replace('\\', "/")
            && e.get("snapshot").and_then(|s| s.as_str()) == Some(snap)
    });
    if !ok {
        return Err(IpcError::denied("快照不存在或不属于该文件"));
    }
    let p = history_root(ws).join(snap);
    let bytes = std::fs::read(&p).map_err(|e| paths::map_io(&p, &e))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(json!({ "file": rel, "snapshot": snap, "content": text, "bytes": bytes.len() }))
}

/// `snapshots.rollback { file, snapshot }`
/// 安全序：先把「当前内容」存成回滚前备份（可再回滚回来），再覆盖目标文件。
pub fn rollback(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let rel = args.req_str("file")?;
    let snap = args.req_str("snapshot")?;
    let rel = rel.replace('\\', "/");
    let (abs, _) = resolve_file(ws, &rel)?;

    let _guard = INDEX_LOCK.lock().unwrap_or_else(|g| g.into_inner());
    let items = load_index(&history_root(ws));
    let ok = items.iter().any(|e| {
        entry_file(e) == rel && e.get("snapshot").and_then(|s| s.as_str()) == Some(snap)
    });
    if !ok {
        return Err(IpcError::denied("快照不存在或不属于该文件"));
    }
    let src = history_root(ws).join(snap);
    let content = std::fs::read(&src).map_err(|e| paths::map_io(&src, &e))?;

    // 当前内容先备份（存在才备）
    let backup_ts = match std::fs::read(&abs) {
        Ok(cur) if !cur.is_empty() => {
            let e = save_snapshot_locked(ws, &rel, Some(&cur), "auto", "回滚前自动备份")?;
            e["ts"].as_u64().unwrap_or(0)
        }
        _ => 0,
    };
    atomic_write(&abs, &content)?;
    Ok(json!({
        "ok": true,
        "file": rel,
        "snapshot": snap,
        "bytes": content.len(),
        "backupTs": backup_ts,
    }))
}

/// save_snapshot 的已持锁版本（rollback 在锁内复用）
fn save_snapshot_locked(
    ws: &Workspace,
    rel: &str,
    content: Option<&[u8]>,
    kind: &str,
    message: &str,
) -> Result<Value, IpcError> {
    let root = history_root(ws);
    let key = key_for(rel);
    let bytes = content.unwrap_or_default();
    let ts = now_ms();
    let name = snapshot_name(rel, ts);
    let mut items = load_index(&root);
    let snap_file = snapshot_path(ws, &key, &name);
    if let Some(b) = content {
        if b.is_empty() {
            return Err(IpcError::new("empty", "内容为空，不存快照"));
        }
        atomic_write(&snap_file, b)?;
    }
    let entry = json!({
        "file": rel,
        "ts": ts,
        "kind": match kind { "ai" | "manual" => kind, _ => "auto" },
        "message": message,
        "bytes": bytes.len(),
        "snapshot": format!("{key}/{name}"),
    });
    items.push(entry.clone());
    prune_per_file(ws, &mut items);
    atomic_write(
        &root.join("index.json"),
        serde_json::to_vec_pretty(&json!({ "version": 1, "items": items })).unwrap_or_default().as_slice(),
    )?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Args;
    use serde_json::json;

    fn ws() -> (std::path::PathBuf, Workspace) {
        let d = std::env::temp_dir().join(format!(
            "mang-snap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        let w = Workspace::new(&d).unwrap();
        (d, w)
    }

    fn args(m: serde_json::Map<String, Value>) -> Args<'static> {
        let m: &'static serde_json::Map<String, Value> = Box::leak(Box::new(m));
        Args::new(m)
    }

    #[test]
    fn check_saves_on_change_and_skips_when_same() {
        let (d, w) = ws();
        std::fs::write(d.join("a.md"), "v1").unwrap();
        let paths_arg = || args(serde_json::Map::from_iter([("paths".into(), json!(["a.md"]))]));
        let out = check(&w, paths_arg()).unwrap();
        assert_eq!(out["saved"].as_array().unwrap().len(), 1);
        // 无变化 → skip
        let out = check(&w, paths_arg()).unwrap();
        assert_eq!(out["saved"].as_array().unwrap().len(), 0);
        assert_eq!(out["skipped"][0]["reason"], "无变化");
        // 改了 → 再存
        std::fs::write(d.join("a.md"), "v2").unwrap();
        let out = check(&w, paths_arg()).unwrap();
        assert_eq!(out["saved"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn read_and_rollback_roundtrip() {
        let (d, w) = ws();
        std::fs::write(d.join("b.md"), "第一章").unwrap();
        let m0 = args(serde_json::Map::from_iter([("paths".into(), json!(["b.md"]))]));
        check(&w, m0).unwrap();
        std::fs::write(d.join("b.md"), "第二章").unwrap();
        let m1 = args(serde_json::Map::from_iter([("path".into(), json!("b.md")), ("message".into(), json!("AI 存版本"))]));
        let saved = save(&w, m1).unwrap();
        assert!(saved["snapshot"].as_str().is_some());
        // 从 index 拿第一份快照（「第一章」那份）
        let items = load_index(&history_root(&w));
        assert_eq!(items.len(), 2);
        let first = items.iter().find(|e| entry_ts(e) > 0 && e["message"] == "").unwrap();
        let snap = first["snapshot"].as_str().unwrap().to_string();
        // 预览
        let m2 = || args(serde_json::Map::from_iter([("file".into(), json!("b.md")), ("snapshot".into(), json!(snap.clone()))]));
        let r = read(&w, m2()).unwrap();
        assert_eq!(r["content"], "第一章");
        // 回滚 → 文件恢复，且产生「回滚前备份」
        let rb = rollback(&w, m2()).unwrap();
        assert_eq!(rb["ok"], true);
        assert_eq!(std::fs::read_to_string(d.join("b.md")).unwrap(), "第一章");
        let items = load_index(&history_root(&w));
        assert!(items.iter().any(|e| e["message"] == "回滚前自动备份"));
        // 越权读：file 对不上 → 拒绝
        let m3 = args(serde_json::Map::from_iter([("file".into(), json!("别的.md")), ("snapshot".into(), json!(snap))]));
        assert!(read(&w, m3).is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn binary_and_outside_rejected() {
        let (d, w) = ws();
        std::fs::write(d.join("x.xlsx"), "bin").unwrap();
        let m = args(serde_json::Map::from_iter([("paths".into(), json!(["x.xlsx"]))]));
        let out = check(&w, m).unwrap();
        assert_eq!(out["saved"].as_array().unwrap().len(), 0);
        assert!(out["skipped"][0]["reason"].as_str().unwrap().contains("只快照文本"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn key_and_label() {
        assert_eq!(key_for("docs/技术标.md"), key_for("docs/技术标.md"));
        assert_ne!(key_for("a/x.md"), key_for("b/x.md")); // 同名不同目录不撞
        assert!(ts_label(0).starts_with("19700101"));
        assert_eq!(ts_label(0), "19700101-000000");
    }
}
