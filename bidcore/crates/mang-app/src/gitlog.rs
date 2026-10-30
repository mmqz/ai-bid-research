//! git 记录（只读）：给「改动 / 历史」面板提供提交列表。
//!
//! 为什么不在 UI 里跑 git：WebView 没有文件系统权限，且 git 需要一个干净的子进程。
//! 这里用 std::process 调系统 git（不引 libgit2，避免再加一个重依赖），
//! 输出用固定分隔符解析，永不 panic；git 不在 PATH / 不是仓库时返回空列表 + 提示。

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::ipc::IpcError;
use crate::paths;

/// 一次最多返回多少条提交（面板只需最近这些）
const LIMIT: usize = 200;

/// 记录分隔符：RS(0x1e) 分条目，US(0x1f) 分字段 —— 提交标题里出现这两个字符的概率≈0
const RS: char = '\u{1e}';
const US: char = '\u{1f}';

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-c")
        .arg("core.quotepath=off")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("无法执行 git：{e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            "git 命令执行失败".to_string()
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// git.log { path?: 相对路径, limit?: n }
/// 返回 { available, branch, items:[{hash, short, author, ts, subject, files?}], error? }
pub fn log(ws: &paths::Workspace, rel: Option<&str>, limit: usize) -> Result<Value, IpcError> {
    let root = ws.root_canonical();
    let limit = limit.clamp(1, LIMIT);
    // 1) 是不是仓库
    let inside = run_git(root, &["rev-parse", "--is-inside-work-tree"]);
    match inside {
        Ok(s) if s.trim() == "true" => {}
        Ok(_) => return Ok(json!({ "available": false, "items": [], "error": "当前工作区不是 git 仓库" })),
        Err(e) => {
            return Ok(json!({
                "available": false,
                "items": [],
                "error": if e.contains("not found") || e.contains("系统找不到") { "未找到 git（请先安装 Git for Windows）" } else { &e }
            }))
        }
    }

    let branch = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // 2) 提交列表：hash / 作者 / 时间 / 标题
    let fmt = format!("--pretty=format:%H{us}%h{us}%an{us}%at{us}%s{rs}", us = US, rs = RS);
    let limit_s = limit.to_string();
    let mut args: Vec<&str> = vec!["log", "--no-color", &fmt, "-n", &limit_s];
    // 只看某个文件的提交（「查看历史」）
    let file_arg;
    if let Some(r) = rel {
        let safe = ws.rel(r)?; // 路径围栏：必须在工作区内
        file_arg = safe;
        args.push("--");
        args.push(&file_arg);
    }
    let raw = match run_git(root, &args) {
        Ok(s) => s,
        Err(e) => {
            // 空仓库（还没有提交）
            return Ok(json!({ "available": true, "branch": branch, "items": [], "error": e }));
        }
    };

    let mut items = Vec::new();
    for chunk in raw.split(RS) {
        let c = chunk.trim_start_matches('\n').trim();
        if c.is_empty() {
            continue;
        }
        let f: Vec<&str> = c.split(US).collect();
        if f.len() < 5 {
            continue;
        }
        items.push(json!({
            "hash": f[0],
            "short": f[1],
            "author": f[2],
            "ts": f[3].parse::<i64>().unwrap_or(0) * 1000,
            "subject": f[4],
        }));
    }

    Ok(json!({ "available": true, "branch": branch, "items": items }))
}

/// git.show { hash, path? } —— 单个提交的 diff（用于「查看这次改了什么」）
pub fn show(ws: &paths::Workspace, hash: &str, rel: Option<&str>) -> Result<Value, IpcError> {
    let root = ws.root_canonical();
    if hash.len() < 4 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(IpcError::bad_args("hash 不合法"));
    }
    let mut args: Vec<&str> = vec!["show", "--no-color", "--stat", "--format=%H%n%an%n%at%n%s", hash];
    let file_arg;
    if let Some(r) = rel {
        file_arg = ws.rel(r)?;
        args.push("--");
        args.push(&file_arg);
    }
    match run_git(root, &args) {
        Ok(text) => Ok(json!({ "hash": hash, "patch": text })),
        Err(e) => Err(IpcError::io(e)),
    }
}
/// git.status { path? } —— 工作区未提交改动（VSCode「源代码管理」的核心数据）
pub fn status(ws: &paths::Workspace, rel: Option<&str>) -> Result<Value, IpcError> {
    let root = ws.root_canonical();
    let mut args: Vec<&str> = vec!["status", "--porcelain=v1", "--untracked-files=normal"];
    let file_arg;
    if let Some(r) = rel {
        file_arg = ws.rel(r)?;
        args.push("--");
        args.push(&file_arg);
    }
    let raw = match run_git(root, &args) {
        Ok(s) => s,
        Err(e) => return Ok(json!({ "available": false, "items": [], "error": e })),
    };
    let mut items = Vec::new();
    for line in raw.lines() {
        if line.len() < 4 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].trim().to_string();
        let state = match code.trim() {
            "M" => "modified",
            "A" => "added",
            "D" => "deleted",
            "R" => "renamed",
            "??" => "untracked",
            "UU" => "conflict",
            _ => "changed",
        };
        items.push(json!({ "code": code.trim(), "state": state, "path": path }));
    }
    Ok(json!({ "available": true, "items": items }))
}