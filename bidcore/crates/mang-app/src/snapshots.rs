//! 版本快照（issue #42 阶段 7）：`.mang/history/` 的 IPC 适配层。
//!
//! 核心逻辑（格式、剪枝、原子写、安全回滚）在 **mang-snap crate**（单一真源，
//! bid-mcp 的 save_version 工具共用同一实现）。本模块只做：
//! 参数解包 → 工作区围栏（`ws.resolve`）→ 转发 → 响应组装。
//!
//! IPC 面：
//!   * `snapshots.check`   —— UI 每 30s 调一次：打开的文本文件内容有变 → 存快照
//!   * `snapshots.save`    —— 手动/AI 存版本
//!   * `snapshots.read`    —— 预览某份快照内容
//!   * `snapshots.rollback`—— 回滚（先把当前内容存成「回滚前备份」，回滚本身可撤销）
//!
//! 读端（`history.list`）在 `recent::history_list`。

use serde_json::{json, Value};

use crate::ipc::{Args, IpcError};
use crate::paths::Workspace;
use mang_snap::{self, Kind};

/// SnapError → IpcError（码位一一对应）
fn map_err(e: mang_snap::SnapError) -> IpcError {
    IpcError::new(e.code, e.message)
}

/// 路径参数 → canonical 绝对路径（工作区围栏）+ rel。
fn resolve_file(ws: &Workspace, raw: &str) -> Result<(std::path::PathBuf, String), IpcError> {
    let abs = ws.resolve(raw, false)?;
    let rel = ws
        .rel_of(&abs)
        .ok_or_else(|| IpcError::denied(format!("路径不在工作区内：{raw}")))?;
    if !mang_snap::is_snapshotable(&abs) {
        return Err(IpcError::new(
            "unsupported",
            format!("只快照文本文件（.md/.txt/.markdown）：{raw}"),
        ));
    }
    Ok((abs, rel))
}

/// `snapshots.check { paths: [abs 或 rel, ...] }`
/// 返回 { saved: [{file, ts}], skipped: [{file, reason}] }
pub fn check(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let Some(paths) = args.map.get("paths").and_then(|p| p.as_array()) else {
        return Err(IpcError::new("bad_args", "缺少 paths 数组"));
    };
    let mut abs_list = Vec::with_capacity(paths.len());
    for p in paths.iter().filter_map(|p| p.as_str()) {
        // 围栏失败不炸批次（记 skipped，reason 带 err 信息）
        match ws.resolve(p, false) {
            Ok(abs) => abs_list.push(abs),
            Err(e) => return Err(e), // check 的调用方（JS）对整批失败有 catch，逐条软失败在核心层
        }
    }
    let root = ws.root_canonical();
    let outcomes = mang_snap::check(root, &abs_list);
    let mut saved = Vec::new();
    let mut skipped = Vec::new();
    for o in outcomes {
        match o {
            mang_snap::FileOutcome::Saved(e) => saved.push(json!({ "file": e.file, "ts": e.ts })),
            mang_snap::FileOutcome::Skipped { file, reason } => skipped.push(json!({ "file": file, "reason": reason })),
        }
    }
    Ok(json!({ "saved": saved, "skipped": skipped }))
}

/// `snapshots.save { path, message?, kind? }` —— 手动 / AI 存版本（bid-mcp save_version 同一真源）
pub fn save(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.req_str("path")?;
    let message = args.map.get("message").and_then(|m| m.as_str()).unwrap_or("").trim();
    let kind = match args.map.get("kind").and_then(|k| k.as_str()) {
        Some("ai") => Kind::Ai,
        Some("manual") => Kind::Manual,
        _ => Kind::Manual,
    };
    let (abs, rel) = resolve_file(ws, raw)?;
    let e = mang_snap::save(ws.root_canonical(), &abs, kind, message).map_err(map_err)?;
    Ok(json!({ "ok": true, "file": rel, "ts": e.ts, "snapshot": e.snapshot }))
}

/// `snapshots.read { file, snapshot }` → { content }（预览某版本）
pub fn read(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let rel = args.req_str("file")?;
    let snap = args.req_str("snapshot")?;
    let content = mang_snap::read(ws.root_canonical(), rel, snap).map_err(map_err)?;
    Ok(json!({ "file": rel, "snapshot": snap, "content": content, "bytes": content.len() }))
}

/// `snapshots.rollback { file, snapshot }`
/// 安全序：先把「当前内容」存成回滚前备份（可再回滚回来），再覆盖目标文件。
pub fn rollback(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let rel = args.req_str("file")?;
    let snap = args.req_str("snapshot")?;
    let rel_norm = rel.replace('\\', "/");
    let (abs, _) = resolve_file(ws, &rel_norm)?;
    let out = mang_snap::rollback(ws.root_canonical(), &abs, &rel_norm, snap).map_err(map_err)?;
    Ok(json!({
        "ok": true,
        "file": out.file,
        "snapshot": snap,
        "bytes": out.bytes,
        "backupTs": out.backup_ts,
    }))
}
