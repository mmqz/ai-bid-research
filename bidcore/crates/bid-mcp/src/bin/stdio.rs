//! stdio 传输：逐行读 `{"tool":..,"args":..}`，逐行回 JSON 应答。
//!
//! 用途：给 Claude Desktop / 其它 MCP 客户端做最简接线验证；也是 M0 骨架的
//! 可执行证据（`cargo run -p bid-mcp --bin bid-mcp-stdio`）。

use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("bid-mcp-stdio: 读取失败: {e}");
                break;
            }
        };
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
}
