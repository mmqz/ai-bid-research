//! stdio 传输：逐行读 `{"tool":..,"args":..}`，逐行回 JSON 应答。
//!
//! 用途：给 Claude Desktop / 其它 MCP 客户端做最简接线验证；也是 M0 骨架的
//! 可执行证据（`cargo run -p bid-mcp --bin bid-mcp-stdio`）。
//!
//! 工作区（#80 第二批）：设 `BID_WS=<dir>` 环境变量后，进程级服务态按该目录
//! 【持久化】打开（db.json 恢复 + 每次写面 checkpoint 落盘；`.mang/llm.json`
//! 亦从该目录解析——真实 LLM 通路的配置面）。未设时保持 M0 纯内存态不变。
//! 工作区损坏/不可写 → 启动即 panic（不静默降级内存态——见 state.rs）。
//!
//! #377（C-18）：分帧改 [`BoundedLineReader`]（单行 ≤ 8 MiB）。超限行流式
//! 读弃 + stderr 响亮横幅 + 一条 `E_UNPROCESSABLE` 错误应答后**继续服务**
//! （坏行只污染它自己的应答）——超大行不再全量进内存。

use bid_mcp::transport::{line_too_long_tool_reply, BoundedLineReader, LineOutcome, MAX_LINE_BYTES};
use std::io::{self, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = BoundedLineReader::new(stdin.lock());
    loop {
        match reader.next_line() {
            LineOutcome::Line(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let reply = bid_mcp::dispatch_json(trimmed);
                if writeln!(stdout, "{reply}").is_err() {
                    break;
                }
                let _ = stdout.flush();
            }
            LineOutcome::TooLong => {
                eprintln!(
                    "bid-mcp-stdio: LINE_TOO_LONG: 行超过 {MAX_LINE_BYTES} 字节上限（含换行），本行已读弃，后续行继续处理"
                );
                let reply = line_too_long_tool_reply(MAX_LINE_BYTES);
                if writeln!(stdout, "{reply}").is_err() {
                    break;
                }
                let _ = stdout.flush();
            }
            LineOutcome::Eof => break,
            LineOutcome::Io(e) => {
                eprintln!("bid-mcp-stdio: 读取失败: {e}");
                break;
            }
        }
    }
}
