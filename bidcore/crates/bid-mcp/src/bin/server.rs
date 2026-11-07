//! 标准 MCP stdio 传输（issue #137）：JSON-RPC 2.0 换行分帧 + initialize /
//! tools/list / tools/call。
//!
//! 与旧 `bid-mcp-stdio` 的分工：
//! - **bid-mcp-server（本 bin）**：标准 MCP 信封——官方 Python/TypeScript SDK、
//!   Claude Desktop 等 MCP 客户端可直接连接（验收见 tests/mcp_sdk_e2e.rs：
//!   官方 `mcp` SDK 拉起本进程跑 initialize + list_tools + call_tool）；
//! - **bid-mcp-stdio（旧 bin）**：逐行裸 JSON `{"tool":..,"args":..}`
//!   直进 `dispatch_json`——既有 e2e 与桌面壳的接线面，信封语义保持向后兼容
//!   （#377 后两 bin 分帧同为有界读取）。
//!
//! 工作区（与旧 bin 同语义）：设 `BID_WS=<dir>` 后进程级服务态按该目录
//! 【持久化】打开（db.json 恢复 + 写面 checkpoint 落盘；`.mang/llm.json`
//! 亦从该目录解析）。未设时纯内存态。工作区损坏/不可写 → 启动即 panic。
//!
//! 协议层语义（版本协商 / 错误映射 / 通知处理）全在 `bid_mcp::protocol`
//! ——本文件只做分帧循环，不含任何协议判断。
//!
//! #377（C-18）：分帧改 [`BoundedLineReader`]（单行 ≤ 8 MiB）。超限行流式
//! 读弃 + stderr 响亮横幅 + 一条 JSON-RPC 错误应答（id null）后**继续服务**
//! ——超大行不再全量进内存。

use bid_mcp::protocol::McpSession;
use bid_mcp::transport::{line_too_long_jsonrpc_reply, BoundedLineReader, LineOutcome, MAX_LINE_BYTES};
use std::io::{self, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut session = McpSession::new();
    let mut reader = BoundedLineReader::new(stdin.lock());
    loop {
        match reader.next_line() {
            LineOutcome::Line(line) => {
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
            LineOutcome::TooLong => {
                eprintln!(
                    "bid-mcp-server: LINE_TOO_LONG: 行超过 {MAX_LINE_BYTES} 字节上限（含换行），本行已读弃，后续行继续处理"
                );
                let reply = line_too_long_jsonrpc_reply(MAX_LINE_BYTES);
                if writeln!(stdout, "{reply}").is_err() {
                    break;
                }
                let _ = stdout.flush();
            }
            LineOutcome::Eof => break,
            LineOutcome::Io(e) => {
                eprintln!("bid-mcp-server: 读取失败: {e}");
                break;
            }
        }
    }
}
