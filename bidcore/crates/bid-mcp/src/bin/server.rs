//! 标准 MCP stdio 传输（issue #137）：JSON-RPC 2.0 换行分帧 + initialize /
//! tools/list / tools/call。
//!
//! 与旧 `bid-mcp-stdio` 的分工：
//! - **bid-mcp-server（本 bin）**：标准 MCP 信封——官方 Python/TypeScript SDK、
//!   Claude Desktop 等 MCP 客户端可直接连接（验收见 tests/mcp_sdk_e2e.rs：
//! 官方 `mcp` SDK 拉起本进程跑 initialize + list_tools + call_tool）；
//! - **bid-mcp-stdio（旧 bin，一字未动）**：逐行裸 JSON `{"tool":..,"args":..}`
//!   直进 `dispatch_json`——既有 e2e 与桌面壳的接线面，保持向后兼容。
//!
//! 工作区（与旧 bin 同语义）：设 `BID_WS=<dir>` 后进程级服务态按该目录
//! 【持久化】打开（db.json 恢复 + 写面 checkpoint 落盘；`.mang/llm.json`
//! 亦从该目录解析）。未设时纯内存态。工作区损坏/不可写 → 启动即 panic。
//!
//! 协议层语义（版本协商 / 错误映射 / 通知处理）全在 `bid_mcp::protocol`
//! ——本文件只做分帧循环，不含任何协议判断。

use std::io::{self, BufRead, Write};

use bid_mcp::protocol::McpSession;

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut session = McpSession::new();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("bid-mcp-server: 读取失败: {e}");
                break;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 通知（无 id）返回 None——不产生应答行；坏行只污染它自己的应答。
        match session.handle_line(trimmed, bid_mcp::dispatch) {
            Some(reply) => {
                if writeln!(stdout, "{reply}").is_err() {
                    break;
                }
                let _ = stdout.flush();
            }
            None => continue,
        }
    }
}
