//! Windows 外壳交互：explorer /select（在资源管理器中显示）

use serde_json::{json, Value};

use crate::ipc::{Args, IpcError};
use crate::paths::{self, Workspace};

/// explorer.reveal {path} → {ok:true, path}
pub fn reveal(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.path("path")?;
    let p = ws.resolve(raw, true)?;
    reveal_path(&p)?;
    Ok(json!({ "ok": true, "path": paths::display(&p) }))
}

#[cfg(windows)]
pub fn reveal_path(p: &std::path::Path) -> Result<(), IpcError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exists = p.exists();
    let mut cmd = std::process::Command::new("explorer.exe");
    if exists && p.is_file() {
        // explorer /select,"<path>" —— 选中该文件
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(p.as_os_str());
        cmd.arg(arg);
    } else if exists {
        // 目录：直接打开该目录
        cmd.arg(p.as_os_str());
    } else {
        // 不存在：打开最近的存在的父目录
        let mut parent = p.parent();
        while let Some(d) = parent {
            if d.exists() {
                cmd.arg(d.as_os_str());
                break;
            }
            parent = d.parent();
        }
    }
    cmd.creation_flags(CREATE_NO_WINDOW);
    match cmd.spawn() {
        Ok(_child) => Ok(()),
        Err(e) => Err(IpcError::io(format!("explorer.exe 启动失败：{e}"))),
    }
}

#[cfg(not(windows))]
pub fn reveal_path(p: &std::path::Path) -> Result<(), IpcError> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    let dir = if p.is_dir() { p.to_path_buf() } else { p.parent().unwrap_or(p).to_path_buf() };
    cmd.arg(dir);
    cmd.spawn().map(|_| ()).map_err(|e| IpcError::io(e.to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_crash_on_missing() {
        // 只验证路径解析分支，不真的拉起 explorer
        let d = std::env::temp_dir().join(format!("mang-reveal-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let ws = crate::paths::Workspace::new(&d).unwrap();
        let p = ws.resolve("nope/x.md", true).unwrap();
        assert!(p.to_string_lossy().contains("mang-reveal-"));
        let _ = std::fs::remove_dir_all(d);
    }
}
