//! #314：`bid-mcp` 启动器/自检 CLI 集成测试——真拉起进程验证（非单测桩）。
//!
//! 覆盖面：
//! - `--doctor`：无 BID_WS 环境下健康（WARN 不阻断）+ 工具面真跑断言
//!   （list_tools 穿透真实分发层，防 echo 假绿）；
//! - `--doctor --ws`：存在且可写 → OK；不存在 → FAIL + 退出码 1（修复指引在案）；
//! - `--init`：stdout 是**可解析 JSON**（parse 失败即红——模板必须能直接粘贴），
//!   command 指向真实 bid-mcp-server 绝对路径；--ws 注入 env 块；
//!   未知客户端 → 退出码 2；
//! - `--help`：三种模式 + 两个传输 bin 的说明在案。

use std::path::PathBuf;
use std::process::Command;

fn cli() -> Command {
    let exe = env!("CARGO_BIN_EXE_bid-mcp");
    let mut c = Command::new(exe);
    // doctor 的环境断言须确定性——剥离宿主环境里的工作区/围栏变量
    c.env_remove("BID_WS").env_remove("BID_FS_ROOT");
    c
}

fn server_bin_path() -> PathBuf {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_bid-mcp"));
    let name = if cfg!(windows) {
        "bid-mcp-server.exe"
    } else {
        "bid-mcp-server"
    };
    exe.parent().unwrap().join(name)
}

#[test]
fn doctor_健康_无工作区时提醒不阻断() {
    let out = cli().arg("--doctor").output().expect("拉起 bid-mcp --doctor");
    assert!(out.status.success(), "退出码应为 0（WARN 不阻断）");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[OK] bid-mcp-server"), "server 在位检查：{stdout}");
    assert!(
        stdout.contains("list_tools 应答 ok=true"),
        "工具面真跑断言（穿透分发层，非 echo）：{stdout}"
    );
    assert!(stdout.contains("[WARN] 工作区"), "未设 BID_WS 应 WARN：{stdout}");
    assert!(stdout.contains("导入围栏"), "围栏检查在案：{stdout}");
}

#[test]
fn doctor_工作区存在且可写_ok_不存在_fail() {
    let tmp = std::env::temp_dir().join(format!(
        "bid-mcp-doctor-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&tmp).expect("建临时工作区");

    // 存在且可写 → OK，退出码 0
    let out = cli()
        .arg("--doctor")
        .arg("--ws")
        .arg(&tmp)
        .output()
        .expect("拉起 doctor --ws <存在目录>");
    assert!(out.status.success(), "可写工作区不应失败");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[OK] 工作区"), "存在且可写应 OK：{stdout}");

    // 不存在 → FAIL + 退出码 1 + 修复指引
    let missing = tmp.join("no-such-dir");
    let out = cli()
        .arg("--doctor")
        .arg("--ws")
        .arg(&missing)
        .output()
        .expect("拉起 doctor --ws <不存在目录>");
    assert!(!out.status.success(), "工作区缺失应退出码 1");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[FAIL] 工作区"), "缺失应 FAIL：{stdout}");
    assert!(stdout.contains("修复指引"), "FAIL 必须带修复指引：{stdout}");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn init_模板是可解析json且指向真实server() {
    let out = cli()
        .arg("--init")
        .arg("--client")
        .arg("json")
        .output()
        .expect("拉起 bid-mcp --init");
    assert!(out.status.success(), "--init json 应成功（server 已随包构建）");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let cfg: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout 必须是可直接粘贴的合法 JSON");
    let command = cfg
        .pointer("/mcpServers/bid/command")
        .and_then(|c| c.as_str())
        .expect("mcpServers.bid.command 在案");
    let expected = server_bin_path();
    assert!(
        PathBuf::from(command).ends_with(expected.file_name().unwrap()),
        "command 应指向 bid-mcp-server（实际 {command}）"
    );
    assert!(PathBuf::from(command).is_absolute(), "command 应为绝对路径（客户端配置要求）");
    // stderr 带落点与验证指引（人读说明走 stderr，stdout 保持纯 JSON）
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("验证"), "stderr 应有验证指引：{stderr}");
}

#[test]
fn init_带ws注入env块() {
    let tmp = std::env::temp_dir().join(format!("bid-mcp-init-ws-{}", std::process::id()));
    let out = cli()
        .arg("--init")
        .arg("--client")
        .arg("cursor")
        .arg("--ws")
        .arg(&tmp)
        .output()
        .expect("拉起 --init --ws");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let cfg: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("带 ws 的模板仍是合法 JSON");
    assert!(
        cfg.pointer("/mcpServers/bid/env/BID_WS").is_some(),
        "--ws 应注入 env.BID_WS：{stdout}"
    );
}

#[test]
fn init_未知客户端退出码2() {
    let out = cli()
        .arg("--init")
        .arg("--client")
        .arg("not-a-client")
        .output()
        .expect("拉起 --init 未知客户端");
    assert_eq!(out.status.code(), Some(2), "用法错误退出码 2");
}

#[test]
fn help_三种模式与传输bin在案() {
    let out = cli().arg("--help").output().expect("拉起 --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--init"), "--init 在案");
    assert!(stdout.contains("--doctor"), "--doctor 在案");
    assert!(stdout.contains("bid-mcp-server"), "标准 MCP 入口在案");
    assert!(stdout.contains("bid-mcp-stdio"), "行协议兼容面在案");
    assert!(stdout.contains("BID_WS"), "工作区变量在案");
}

#[test]
fn 未知参数退出码2() {
    let out = cli().arg("--nonsense").output().expect("拉起未知参数");
    assert_eq!(out.status.code(), Some(2), "未知参数退出码 2（非 panic）");
}
