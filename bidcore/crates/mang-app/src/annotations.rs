//! 来源标注边车（issue #42 阶段 8）：`.mang/annotations/<key>.json`。
//!
//! 定位：**浮层是覆盖层，绝对不修改文件内容**——标注数据全部住在边车里。
//! 谁写标注（AI 管线 / MCP 后续接线）与本模块解耦：这里只提供读 + 认校验
//! 三态落盘。key 复用 mang-snap 的 `file_key`（与快照目录同一消毒规则）。
//!
//! IPC 面：
//!   * `annotations.load { path }`          → { file, items, present }
//!   * `annotations.setStatus { path, id, status }` → { ok }（原子读改写）
//!
//! 条目形状（对齐 issue #42 §六的浮层样例）：
//! ```json
//! { "id": "1", "block": "b3", "quote": "本次方案采用分布式架构",
//!   "source": { "file": "招标文件.docx", "page": 12, "section": "3.2",
//!               "quote": "投标人须提供分布式架构方案" },
//!   "status": "confirmed" | "pending" | "rejected", "note": "" }
//! ```

use serde_json::{json, Value};

use crate::ipc::{now_ms, Args, IpcError};
use crate::paths::{self, Workspace};

/// 标注状态三态（认校验面）
const STATUSES: [&str; 3] = ["confirmed", "pending", "rejected"];

fn sidecar_path(ws: &Workspace, rel: &str) -> std::path::PathBuf {
    ws.sub(".mang").join("annotations").join(mang_snap::file_key(rel) + ".json")
}

/// 读边车（缺失 → 空数组；坏 JSON → 空数组 + error 提示，不炸 UI）
fn load_items(path: &std::path::Path) -> (Vec<Value>, Option<String>) {
    let Ok(bytes) = std::fs::read(path) else {
        return (Vec::new(), None);
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(o)) => {
            let items = o.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
            (items, None)
        }
        _ => (
            Vec::new(),
            Some(format!("标注边车损坏，已忽略：{}", paths::display(path))),
        ),
    }
}

/// 原子写：.tmp + rename（与 mang-snap 同款纪律）
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), IpcError> {
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

/// `annotations.load { path }` → { file, items, present, error? }
pub fn load(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.req_str("path")?;
    let abs = ws.resolve(raw, false)?;
    let rel = ws
        .rel_of(&abs)
        .ok_or_else(|| IpcError::denied(format!("路径不在工作区内：{raw}")))?;
    let p = sidecar_path(ws, &rel);
    let (items, err) = load_items(&p);
    Ok(json!({
        "file": rel,
        "items": items,
        "present": p.exists(),
        "error": err,
    }))
}

/// `annotations.setStatus { path, id, status }` → { ok }
/// 读改写整份边车（单用户桌面场景，无并发窗口问题；原子提交防撕裂）。
pub fn set_status(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.req_str("path")?;
    let id = args.req_str("id")?;
    let status = args.req_str("status")?;
    if !STATUSES.contains(&status) {
        return Err(IpcError::new(
            "bad_args",
            format!("status 必须是 confirmed/pending/rejected 之一（收到 {status}）"),
        ));
    }
    let abs = ws.resolve(raw, false)?;
    let rel = ws
        .rel_of(&abs)
        .ok_or_else(|| IpcError::denied(format!("路径不在工作区内：{raw}")))?;
    let p = sidecar_path(ws, &rel);

    let (mut items, parse_err) = load_items(&p);
    if let Some(e) = parse_err {
        return Err(IpcError::new("io", e));
    }
    let mut hit = false;
    for it in items.iter_mut() {
        if it.get("id").and_then(|v| v.as_str()) == Some(id) {
            if let Some(o) = it.as_object_mut() {
                o.insert("status".into(), json!(status));
                hit = true;
            }
        }
    }
    if !hit {
        return Err(IpcError::new("not_found", format!("标注 {id} 不存在（边车里没有这一条）")));
    }
    let doc = json!({
        "version": 1,
        "file": rel,
        "updated": now_ms(),
        "items": items,
    });
    atomic_write(
        &p,
        serde_json::to_vec_pretty(&doc).unwrap_or_default().as_slice(),
    )?;
    Ok(json!({ "ok": true, "file": rel, "id": id, "status": status }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Args;
    use serde_json::json;

    fn ws() -> (std::path::PathBuf, Workspace) {
        let d = std::env::temp_dir().join(format!(
            "mang-anno-{}-{}",
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
    fn load_missing_is_empty_not_error() {
        let (d, w) = ws();
        std::fs::write(d.join("a.md"), "内容").unwrap();
        let m = args(serde_json::Map::from_iter([("path".into(), json!("a.md"))]));
        let out = load(&w, m).unwrap();
        assert_eq!(out["present"], json!(false));
        assert_eq!(out["items"].as_array().unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn set_status_roundtrip_and_validations() {
        let (d, w) = ws();
        std::fs::write(d.join("b.md"), "内容").unwrap();
        let sidecar = sidecar_path(&w, "b.md");
        atomic_write(
            &sidecar,
            r#"{"version":1,"file":"b.md","items":[{"id":"1","block":"b2","quote":"q","source":{"file":"招标文件.docx","page":12,"section":"3.2","quote":"sq"},"status":"pending","note":""}]}"#.as_bytes(),
        )
        .unwrap();

        // load 能读
        let m = args(serde_json::Map::from_iter([("path".into(), json!("b.md"))]));
        let out = load(&w, m).unwrap();
        assert_eq!(out["present"], json!(true));
        assert_eq!(out["items"][0]["status"], json!("pending"));

        // 改状态
        let m = args(serde_json::Map::from_iter([
            ("path".into(), json!("b.md")),
            ("id".into(), json!("1")),
            ("status".into(), json!("confirmed")),
        ]));
        let out = set_status(&w, m).unwrap();
        assert_eq!(out["ok"], json!(true));
        let (items, err) = load_items(&sidecar);
        assert!(err.is_none());
        assert_eq!(items[0]["status"], json!("confirmed"));

        // 非法状态
        let m = args(serde_json::Map::from_iter([
            ("path".into(), json!("b.md")),
            ("id".into(), json!("1")),
            ("status".into(), json!("hacked")),
        ]));
        assert!(set_status(&w, m).is_err());
        // 不存在的标注
        let m = args(serde_json::Map::from_iter([
            ("path".into(), json!("b.md")),
            ("id".into(), json!("99")),
            ("status".into(), json!("pending")),
        ]));
        assert!(set_status(&w, m).is_err());
        let _ = std::fs::remove_dir_all(d);
    }
}
