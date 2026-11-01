//! #137 MCP 协议层 · 官方 Python mcp SDK 端到端验收。
//!
//! 进程级真实链路：`cargo test` 先构建 `bid-mcp-server` bin
//! （`CARGO_BIN_EXE_bid-mcp-server` 由 cargo 自动注入），本驱动用 python3
//! 拉起 `tests/mcp_sdk_e2e.py`——官方 SDK 的 `stdio_client` 再 spawn
//! `bid-mcp-server` 子进程，跑 initialize（版本协商）+ list_tools（20 工具
//! + inputSchema）+ call_tool（成功 / isError:true 契约错误码 / 未知工具
//! -32602）。这是 issue #137 验收第一条（官方客户端能连上）的最终证据。
//!
//! SKIP 纪律（issue #9 同款）：环境无 python3 或未装 `mcp` 包时响亮横幅 +
//! return（CI 无 Python 面不红）；`BID_MCP_SDK_E2E=1` 强制跑（缺环境 =
//! 测试失败——防「静默跳过骗绿」）；`=0` 强制跳过。

use std::path::PathBuf;
use std::process::Command;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn mcp_official_sdk_e2e() {
    let script = crate_dir().join("tests/mcp_sdk_e2e.py");
    // cargo 为被测 crate 的每个 bin 注入 CARGO_BIN_EXE_<name>
    let server = option_env!("CARGO_BIN_EXE_bid-mcp-server")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bid-mcp-server"));

    match std::env::var("BID_MCP_SDK_E2E").as_deref() {
        Ok("0") => {
            eprintln!("[SKIP] BID_MCP_SDK_E2E=0 —— 官方 SDK e2e 被显式跳过");
            return;
        }
        _ => {}
    }

    // 环境探测：python3 + mcp 包（任一缺席 → 默认跳过；强制模式 → 失败）
    let probe = Command::new("python3")
        .args(["-c", "import mcp"])
        .output();
    let available = matches!(&probe, Ok(o) if o.status.success());
    if !available {
        if std::env::var("BID_MCP_SDK_E2E").as_deref() == Ok("1") {
            panic!(
                "BID_MCP_SDK_E2E=1 强制运行，但 python3/mcp 包不可用：{:?}",
                probe.err().map(|e| e.to_string())
            );
        }
        eprintln!("[SKIP] 官方 SDK e2e 需要 python3 + pip 包 mcp（当前环境缺席）——\
                   本地装法：pip install mcp；强制跑：BID_MCP_SDK_E2E=1 cargo test");
        return;
    }

    let out = Command::new("python3")
        .arg(&script)
        .arg(&server)
        .output()
        .expect("拉起 mcp_sdk_e2e.py");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    print!("{stdout}");
    if !out.status.success() {
        panic!("官方 SDK e2e 失败（exit {:?}）\n--- stderr ---\n{stderr}", out.status.code());
    }
}
