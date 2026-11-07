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

/// 读边车（缺失 → 空数组；坏 JSON → 空数组 + error 提示，不炸 UI）。
///
/// #430 · IO 错误分流：`fs::read` 的**非 NotFound 失败**（Windows 文件锁 /
/// Defender 实时扫描窗口的 sharing violation、权限面等瞬态）此前与「文件
/// 不存在」同路（空 + 无错）——set_status 紧随 load 之后会误报
/// not_found「标注不存在」，UI 显示「无标注」而非报错（静默丢数据面）。
/// 现口径：NotFound 维持现状（「无标注」是正常态）；其他 IO 错误先做
/// **有界重试**（3 次 × 50ms——吸收写→读序列撞上的扫描窗口），仍失败则
/// 如实报 error（读失败 ≠ 不存在）。
fn load_items(path: &std::path::Path) -> (Vec<Value>, Option<String>) {
    let bytes = match read_with_retry(path) {
        Ok(b) => b,
        Err(None) => return (Vec::new(), None), // NotFound——无标注是正常态
        Err(Some(e)) => {
            return (
                Vec::new(),
                Some(format!("标注边车读取失败（可能被占用/权限问题）：{e}")),
            )
        }
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

/// 有界重试读（#430）：NotFound 立即返回 `Err(None)`；其他 IO 错误重试
/// 3 次 × 50ms（瞬时文件锁窗口量级），仍失败返回 `Err(Some(错误描述))`。
/// 收在模块内小 helper——不引依赖、不改调用方签名。
fn read_with_retry(path: &std::path::Path) -> Result<Vec<u8>, Option<String>> {
    const ATTEMPTS: usize = 3;
    const BACKOFF_MS: u64 = 50;
    let mut last_err: Option<String> = None;
    for attempt in 0..ATTEMPTS {
        match std::fs::read(path) {
            Ok(b) => return Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(None),
            Err(e) => {
                last_err = Some(format!("{}（第 {} 次尝试）", e, attempt + 1));
                if attempt + 1 < ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(BACKOFF_MS));
                }
            }
        }
    }
    Err(last_err)
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
    // #474 · 瞬态空确认：load 刚读到的边车，set_status 重读时可瞬时
    // NotFound（NTFS rename 元数据窗口 / 并行测试收尾的 remove 竞态——
    // Windows CI 实录：load 断言通过、紧邻的 set_status 误报 not_found）。
    // 空列表是「判 not_found」的依据，判定前做一次短等待复核，把
    // 「瞬态不可见」与「真无标注」分开；复核中的读错误如实上抛。
    if items.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(25));
        let (retry, retry_err) = load_items(&p);
        if let Some(e) = retry_err {
            return Err(IpcError::new("io", e));
        }
        items = retry;
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

    /// #474 · 目录唯一性双保险：pid + 原子序列号 + nanos。Windows 时钟
    /// 粒度下两个并行测试线程可能拿到相同 nanos（GetSystemTimePrecise
    /// 的实际更新粒度在 CI 虚拟机上更粗）——撞名即共享 temp 目录，另一
    /// 测试收尾的 remove_dir_all 会把本测试的边车删掉，set_status 读
    /// 到瞬态 NotFound。序列号保证同进程内永不撞名。
    static DIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn ws() -> (std::path::PathBuf, Workspace) {
        let d = std::env::temp_dir().join(format!(
            "mang-anno-{}-{}-{}",
            std::process::id(),
            DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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

    /// #430 · IO 错误分流三分支（load_items 直测）：
    /// NotFound → (空, None)（无标注是正常态）；
    /// 读失败（目录当读取目标——跨平台确定性失败，免权限模拟）→ (空, Some)；
    /// 正常读 → 内容解析照旧。
    #[test]
    fn load_items_routes_io_errors_not_silent() {
        // ① NotFound：正常态，零错误提示
        let (d, w) = ws();
        let missing = sidecar_path(&w, "none.md");
        let (items, err) = load_items(&missing);
        assert!(items.is_empty());
        assert!(err.is_none(), "NotFound 不得报错（无标注是正常态）");

        // ② 读失败 ≠ 不存在：把边车路径本身占成目录——fs::read 必失败
        //（Unix: IsADirectory / Windows: PermissionDenied——均非 NotFound），
        // 有界重试耗尽后必须如实报错（修复前：静默空 + 无错 → set_status
        // 误报 not_found、UI 显示「无标注」——静默丢数据面）。
        let blocked = sidecar_path(&w, "blocked.md");
        std::fs::create_dir_all(&blocked).unwrap();
        let (items, err) = load_items(&blocked);
        assert!(items.is_empty());
        let err = err.expect("读失败必须报 error（不静默）");
        assert!(err.contains("读取失败"), "报错须指明读失败：{err}");
        let _ = std::fs::remove_dir_all(d);
    }

    /// #474 · 瞬态不可见确认面：边车在 load 之后、set_status 第一次读
    /// 之前被并行活动移走、确认窗口内回来 → 必须命中而非误报 not_found
    /// （Windows CI 实录的 flake 形态：load 断言通过、set_status 报
    /// 「标注 1 不存在」——40 passed; 1 failed 偶发）。
    #[test]
    fn set_status_survives_transient_sidecar_invisibility() {
        let (d, w) = ws();
        std::fs::write(d.join("t.md"), "内容").unwrap();
        let sidecar = sidecar_path(&w, "t.md");
        atomic_write(
            &sidecar,
            r#"{"version":1,"file":"t.md","items":[{"id":"1","block":"b2","quote":"q","source":{"file":"f.pdf","page":1,"section":"1","quote":"sq"},"status":"pending","note":""}]}"#.as_bytes(),
        )
        .unwrap();

        // load 读到（CI 实录里的真实前提）
        let m = args(serde_json::Map::from_iter([("path".into(), json!("t.md"))]));
        let out = load(&w, m).unwrap();
        assert_eq!(out["present"], json!(true));

        // 模拟并行收尾的删除 + 10ms 后恢复（瞬态不可见窗口，确定性时序：
        // set_status 第一次读在微秒级、恢复在 10ms、确认复核在 25ms）
        std::fs::remove_file(&sidecar).unwrap();
        let sc = sidecar.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(10));
            std::fs::write(&sc, r#"{"version":1,"file":"t.md","items":[{"id":"1","block":"b2","quote":"q","source":{"file":"f.pdf","page":1,"section":"1","quote":"sq"},"status":"pending","note":""}]}"#).unwrap();
        });

        let m = args(serde_json::Map::from_iter([
            ("path".into(), json!("t.md")),
            ("id".into(), json!("1")),
            ("status".into(), json!("confirmed")),
        ]));
        let out = set_status(&w, m).unwrap();
        assert_eq!(out["ok"], json!(true), "瞬态不可见必须被确认复核救回");
        let _ = std::fs::remove_dir_all(d);
    }

    /// #474 · 行为兼容面：真无标注（边车从未存在）的 setStatus 语义
    /// 不变——确认复核后仍报 not_found（只多 25ms，不改变判定结果）。
    #[test]
    fn set_status_on_missing_sidecar_still_not_found() {
        let (d, w) = ws();
        std::fs::write(d.join("m.md"), "内容").unwrap();
        let m = args(serde_json::Map::from_iter([
            ("path".into(), json!("m.md")),
            ("id".into(), json!("1")),
            ("status".into(), json!("confirmed")),
        ]));
        let err = set_status(&w, m).expect_err("无边车的 setStatus 必须报 not_found");
        assert_eq!(err.code, "not_found", "真无标注 ≠ 瞬态不可见（{err:?}）");
        let _ = std::fs::remove_dir_all(d);
    }

    /// #474 · 目录唯一性回归钉：同进程内两次 ws() 永不撞名（序列号保证）。
    #[test]
    fn ws_dirs_never_collide() {
        let (d1, _w1) = ws();
        let (d2, _w2) = ws();
        assert_ne!(d1, d2, "temp 目录必须唯一（pid+seq+nanos 双保险）");
        let _ = std::fs::remove_dir_all(d1);
        let _ = std::fs::remove_dir_all(d2);
    }

    /// #430 · set_status 接线面：边车不可读时报 **io 错误**而非误导性的
    /// not_found「标注不存在」（#413 CI 假红的失败形态——load 刚读到、
    /// set_status 却说标注不存在）。
    #[test]
    fn set_status_reports_io_error_when_sidecar_unreadable() {
        let (d, w) = ws();
        std::fs::write(d.join("u.md"), "内容").unwrap();
        // 边车路径占成目录 → load_items 读失败 → set_status 必须走 io 路径
        let blocked = sidecar_path(&w, "u.md");
        std::fs::create_dir_all(&blocked).unwrap();
        let m = args(serde_json::Map::from_iter([
            ("path".into(), json!("u.md")),
            ("id".into(), json!("1")),
            ("status".into(), json!("confirmed")),
        ]));
        let err = set_status(&w, m).expect_err("边车不可读必须报错");
        assert_eq!(
            err.code,
            "io",
            "不可读 ≠ 不存在——报 io 而非 not_found（{err:?}）"
        );
        let _ = std::fs::remove_dir_all(d);
    }
}
