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

/// git 子进程超时（毫秒）。#476（D-16）：大仓库/坏 .git 上 git 可能极慢
/// 或挂起——UI 线程同步等待无上限 = WebView 消息循环冻结（整窗无响应且
/// 不可恢复）。常态 git log/status <1s；10s 覆盖慢盘，超出即判异常仓库。
const GIT_TIMEOUT_MS: u64 = 10_000;

/// 记录分隔符：RS(0x1e) 分条目，US(0x1f) 分字段 —— 提交标题里出现这两个字符的概率≈0
const RS: char = '\u{1e}';
const US: char = '\u{1f}';

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run_git_timed(dir, args, GIT_TIMEOUT_MS)
}

/// 带超时的 git 调用（#476 可测核心——`run_git` 的固定 10s 包装）。
///
/// 错误语义与旧实现一致：stderr 优先，空则「git 命令执行失败」。
fn run_git_timed(dir: &Path, args: &[&str], timeout_ms: u64) -> Result<String, String> {
    let mut child = Command::new("git")
        .arg("-c")
        .arg("core.quotepath=off")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行 git：{e}"))?;
    let (status, out, err) = wait_output_with_deadline(&mut child, timeout_ms)?;
    if !status.success() {
        let err = err.trim();
        return Err(if err.is_empty() {
            "git 命令执行失败".to_string()
        } else {
            err.to_string()
        });
    }
    Ok(out)
}

/// 子进程带截止时间的输出收集（#476 核心机制——gitlog 生产面 + 单测共用）。
///
/// 结构（为什么不是简单 `output()` + join）：
/// - **读线程消费 stdout/stderr**：轮询 `try_wait` 期间若无人读管道，
///   输出超过管道缓冲区（Linux 64KB；LIMIT=200 条 commit 的 log 输出
///   可超）会让子进程阻塞在 write——「进程活着但永不退出」的死锁面。
///   读线程持续 drain，子进程正常退出；
/// - **try_wait 轮询 + deadline**：deadline 只在「进程仍在跑」时检查
///   （已退出则正常收集，不误伤快命令）。超时 `kill()` + `wait()`
///   （收尸防僵尸）——kill 后管道 EOF，读线程自然退出，join 不悬空。
fn wait_output_with_deadline(
    child: &mut std::process::Child,
    timeout_ms: u64,
) -> Result<(std::process::ExitStatus, String, String), String> {
    let mut stdout_pipe = child.stdout.take().expect("调用方须 piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("调用方须 piped stderr");
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut out = String::new();
        let mut err = String::new();
        let _ = stdout_pipe.read_to_string(&mut out);
        let _ = stderr_pipe.read_to_string(&mut err);
        (out, err)
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // 收尸（防僵尸进程挂在面板会话下）
                    let _ = reader.join(); // kill → 管道 EOF → 读线程退出
                    return Err(format!(
                        "git 命令超时（>{timeout_ms}ms）已终止——大仓库或损坏 .git 可能极慢，请检查仓库健康度"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            Err(e) => {
                let _ = reader.join();
                return Err(format!("git 等待失败：{e}"));
            }
        }
    };
    let (out, err) = reader.join().map_err(|_| "git 输出读取线程崩溃".to_string())?;
    Ok((status, out, err))
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
#[cfg(test)]
mod tests {
    use super::*;

    /// 稳定慢命令（~2s）：跨平台可用的「进程在跑」构造器。
    /// Linux/macOS：sleep；Windows：ping 本机 2 包（~2s）。
    fn slow_command() -> Command {
        if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "3", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("2");
            c
        }
    }

    fn spawn_piped(cmd: &Command) -> std::process::Child {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("慢命令应可 spawn（CI/开发机 sleep/ping 在 PATH）")
    }

    /// #476（D-16）主断言：进程仍在跑 + 预算耗尽 → kill + 超时 Err
    ///（防回归锁：超时路径必须「可恢复地失败」，不是永久挂起）。
    /// 慢命令 ~2s、预算 50ms → 稳定走超时分支（非时序敏感）。
    #[test]
    fn deadline_kills_slow_process_with_timeout_error() {
        let mut child = spawn_piped(&slow_command());
        let r = wait_output_with_deadline(&mut child, 50);
        let msg = r.expect_err("2s 慢命令配 50ms 预算必超时");
        assert!(
            msg.contains("超时"),
            "超时路径的错误文案须引导用户——实际: {msg}"
        );
        // kill 已收尸：后续 wait 立即返回（不再挂起）
        let st = child.wait().expect("kill 后 wait 应立即成功");
        assert!(!st.success(), "被 kill 的进程退出码非成功");
    }

    /// 充裕预算：慢命令（~2s）正常完成 → Ok + 输出收集无损
    ///（timeout 机制不误伤慢但合法的命令；读线程的 stdout 传递面）。
    #[test]
    fn generous_budget_lets_slow_process_finish() {
        let mut child = spawn_piped(&slow_command());
        let (status, _out, _err) =
            wait_output_with_deadline(&mut child, 15_000).expect("15s 预算足够 2s 命令完成");
        assert!(status.success(), "sleep/ping 正常退出应 success");
    }

    /// 既有语义回归锁：git 命令失败（非仓库目录跑 log）→ run_git_timed
    /// 的 Err 带 stderr 或「git 命令执行失败」——错误分流不因超时重构漂移。
    #[test]
    fn run_git_failed_command_returns_err_with_stderr_semantics() {
        let dir = std::env::temp_dir();
        let r = run_git_timed(&dir, &["log", "--oneline", "-n", "1"], 10_000);
        match r {
            Err(msg) => assert!(
                !msg.contains("超时"),
                "非仓库目录的 log 是立即失败（fatal: not a git repository），不是超时——实际: {msg}"
            ),
            Ok(out) => panic!("非仓库目录的 git log 应失败——实际 Ok: {out:?}"),
        }
    }

    /// 快命令不误伤：git --version 在默认 10s 预算下完成
    ///（deadline 只在「仍在跑」时检查——已退出则正常收集）。
    #[test]
    fn run_git_fast_command_completes_within_default_budget() {
        let dir = std::env::temp_dir();
        let out =
            run_git(&dir, &["--version"]).expect("git 在 PATH 时 --version 应成功");
        assert!(out.contains("git version"), "stdout 传递面——实际: {out:?}");
    }
}
