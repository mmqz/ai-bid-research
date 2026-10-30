//! 最近打开（%APPDATA%\\mang\\recent.json）+ 版本历史（<workspace>/.mang/history/index.json）
//!
//! 两者都是"多进程/多线程可能同时写"的小 JSON，所以：
//!   * recent：一把全局 Mutex 串行读写
//!   * 写盘一律 先写 .tmp 再 rename

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::ipc::{Args, IpcError};
use crate::paths::{self, Workspace};

pub const RECENT_MAX: usize = 50;

static RECENT_LOCK: Mutex<()> = Mutex::new(());

/// %APPDATA%\\mang\\recent.json（APPDATA 缺失时退回 %LOCALAPPDATA% → %TEMP%）
pub fn recent_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);
    base.join("mang").join("recent.json")
}

fn read_items(path: &std::path::Path) -> Vec<Value> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
        eprintln!("[mang] recent.json 损坏，已忽略：{}", path.display());
        return Vec::new();
    };
    match v {
        Value::Array(a) => a,
        Value::Object(o) => o
            .get("items")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn clean_item(v: &Value) -> Option<Value> {
    let o = v.as_object()?;
    let path = o.get("path")?.as_str()?.trim();
    if path.is_empty() {
        return None;
    }
    let name = o
        .get("name")
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            std::path::Path::new(path)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string())
        });
    let kind = match o.get("kind").and_then(|k| k.as_str()) {
        Some("dir") => "dir",
        Some("file") => "file",
        _ => "file",
    };
    let opened_at = o
        .get("openedAt")
        .and_then(|t| t.as_u64().or_else(|| t.as_f64().map(|f| f as u64)))
        .unwrap_or_else(crate::ipc::now_ms);
    let exists = std::path::Path::new(path).exists();
    Some(json!({
        "path": path,
        "name": name,
        "kind": kind,
        "openedAt": opened_at,
        "exists": exists,
    }))
}

/// recent.load → {items:[{path,name,kind,openedAt,exists}], file}
pub fn load() -> Result<Value, IpcError> {
    let _g = RECENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = recent_path();
    let items: Vec<Value> = read_items(&path)
        .iter()
        .filter_map(clean_item)
        .take(RECENT_MAX)
        .collect();
    Ok(json!({ "items": items, "file": paths::display(&path) }))
}

/// recent.save {items:[...]} → {ok:true, count, file}
pub fn save(args: Args<'_>) -> Result<Value, IpcError> {
    let arr = match args.get("items") {
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(IpcError::bad_args("items 必须是数组")),
        None => return Err(IpcError::bad_args("缺少参数 items")),
    };
    let mut items: Vec<Value> = Vec::new();
    for v in arr.iter() {
        if let Some(c) = clean_item(v) {
            // 同一个 path 只留最新一条
            if let Some(pos) = items.iter().position(|x| x["path"] == c["path"]) {
                items[pos] = c;
            } else {
                items.push(c);
            }
        }
    }
    items.sort_by_key(|v| std::cmp::Reverse(v["openedAt"].as_u64().unwrap_or(0)));
    items.truncate(RECENT_MAX);

    let _g = RECENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = recent_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| paths::map_io(dir, &e))?;
    }
    let bytes = serde_json::to_vec_pretty(&json!(items)).map_err(|e| IpcError::io(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| paths::map_io(&tmp, &e))?;
    std::fs::rename(&tmp, &path).map_err(|e| paths::map_io(&path, &e))?;
    Ok(json!({ "ok": true, "count": items.len(), "file": paths::display(&path) }))
}

/// history.list {path?} → {items:[{ts,kind,message,file,snapshot?}], file, present}
///
/// 读 <workspace>/.mang/history/index.json。容忍三种形态：
///   * 数组 [ {ts,kind,message,file,...} ]
///   * 对象 {items:[...]}
///   * 对象 { "<relpath>": [ {...} ] }（按文件分组）
/// `path` 给了就只返回该文件的记录（相对工作区，或绝对路径）。
pub fn history_list(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let filter = args.opt_path("path")?;
    let limit = args.u64("limit")?.unwrap_or(200) as usize;
    let index = ws.sub(".mang").join("history").join("index.json");
    let exists = index.exists();
    if !exists {
        return Ok(json!({
            "items": [],
            "file": paths::display(&index),
            "present": false,
        }));
    }
    let bytes = std::fs::read(&index).map_err(|e| paths::map_io(&index, &e))?;
    let v: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            // 历史索引坏了也不能让 UI 崩；返回空 + 提示
            return Ok(json!({
                "items": [],
                "file": paths::display(&index),
                "present": true,
                "error": format!("index.json 解析失败：{e}"),
            }));
        }
    };

    let filter_norm = filter.map(|f| {
        let s = f.replace('\\', "/");
        s.trim_start_matches("./").to_string()
    });

    let mut items: Vec<Value> = Vec::new();
    let mut push = |entry: &Value, owner: Option<&str>| {
        let Some(o) = entry.as_object() else { return };
        let ts = o
            .get("ts")
            .or_else(|| o.get("time"))
            .or_else(|| o.get("mtimeMs"))
            .and_then(|t| t.as_u64().or_else(|| t.as_f64().map(|f| f as u64)))
            .unwrap_or(0);
        let kind = match o.get("kind").and_then(|k| k.as_str()) {
            Some("manual") | Some("ai") => "manual",
            _ => "auto",
        };
        let message = o
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let file = o
            .get("file")
            .or_else(|| o.get("path"))
            .and_then(|f| f.as_str())
            .map(|s| s.replace('\\', "/"))
            .or_else(|| owner.map(|s| s.to_string()))
            .unwrap_or_default();
        let snapshot = o
            .get("snapshot")
            .and_then(|s| s.as_str())
            .map(|s| s.replace('\\', "/"));
        items.push(json!({
            "ts": ts,
            "kind": kind,
            "message": message,
            "file": file,
            "snapshot": snapshot,
        }));
    };

    match v {
        Value::Array(a) => {
            for e in &a {
                push(e, None);
            }
        }
        Value::Object(mut o) => {
            if let Some(Value::Array(a)) = o.remove("items") {
                for e in &a {
                    push(e, None);
                }
            } else {
                for (k, val) in o.iter() {
                    let owner = k.replace('\\', "/");
                    match val {
                        Value::Array(a) => {
                            for e in a {
                                push(e, Some(&owner));
                            }
                        }
                        Value::Object(_) => push(val, Some(&owner)),
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }

    if let Some(f) = filter_norm {
        items.retain(|i| {
            let file = i["file"].as_str().unwrap_or("");
            file == f || file.ends_with(&format!("/{f}")) || f.ends_with(file)
        });
    }
    // 新的在前
    items.sort_by_key(|v| std::cmp::Reverse(v["ts"].as_u64().unwrap_or(0)));
    items.truncate(limit);

    Ok(json!({
        "items": items,
        "file": paths::display(&index),
        "present": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Args;

    #[test]
    fn history_tolerates_missing_and_grouped() {
        let d = std::env::temp_dir().join(format!(
            "mang-hist-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(d.join(".mang/history")).unwrap();
        let ws = Workspace::new(&d).unwrap();
        // 缺失
        let out = history_list(&ws, Args::new(&serde_json::Map::new())).unwrap();
        assert_eq!(out["present"], json!(false));
        assert_eq!(out["items"].as_array().unwrap().len(), 0);
        // 分组形态
        std::fs::write(
            d.join(".mang/history/index.json"),
            r#"{"技术标.md":[{"ts":100,"kind":"manual","message":"第1章","snapshot":".mang/history/技术标.md/100.md"},{"ts":300,"kind":"auto","message":""}],"报价.xlsx":[{"ts":200,"kind":"manual","message":"x"}]}"#,
        )
        .unwrap();
        let out = history_list(&ws, Args::new(&serde_json::Map::new())).unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), 3);
        assert_eq!(out["items"][0]["ts"], json!(300)); // 新的在前
        // 过滤
        let mut m = serde_json::Map::new();
        m.insert("path".into(), json!("技术标.md"));
        let out = history_list(&ws, Args::new(&m)).unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), 2);
        assert_eq!(out["items"][0]["kind"], json!("auto"));
        // 坏 JSON 不炸
        std::fs::write(d.join(".mang/history/index.json"), "{oops").unwrap();
        let out = history_list(&ws, Args::new(&serde_json::Map::new())).unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), 0);
        assert!(out["error"].is_string());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn recent_roundtrip() {
        let mut m = serde_json::Map::new();
        m.insert(
            "items".into(),
            json!([
                {"path":"C:\\a\\b.md","name":"b.md","kind":"file","openedAt":10},
                {"path":"C:\\a\\b.md","openedAt":99},
                {"path":"  ","openedAt":1},
            ]),
        );
        let out = save(Args::new(&m)).unwrap();
        assert_eq!(out["count"], json!(1));
        let loaded = load().unwrap();
        assert!(loaded["items"].is_array());
    }
}
