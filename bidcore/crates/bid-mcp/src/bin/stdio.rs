//! stdio 传输：逐行读 `{"tool":..,"args":..}`，逐行回 JSON 应答。
//!
//! 用途：给 Claude Desktop / 其它 MCP 客户端做最简接线验证；也是 M0 骨架的
//! 可执行证据（`cargo run -p bid-mcp --bin bid-mcp-stdio`）。
//!
//! 工作区（#80 第二批）：设 `BID_WS=<dir>` 环境变量后，进程级服务态按该目录
//! 【持久化】打开（db.json 恢复 + 每次写面 checkpoint 落盘；`.mang/llm.json`
//! 亦从该目录解析——真实 LLM 通路的配置面）。未设时保持 M0 纯内存态不变。
//! 工作区损坏/不可写 → 启动即 panic（不静默降级内存态——见 state.rs）。

use std::io::{self, BufRead, Write};

fn main() {
    // #314：启动器 CLI 面（--init / --doctor / --help）——先于传输面消费
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() && args[0].starts_with("--") {
        std::process::exit(bid_mcp::launcher::handle_cli(&args).unwrap_or(0));
    }
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
