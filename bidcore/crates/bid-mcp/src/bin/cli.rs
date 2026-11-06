//! `bid-mcp` —— 启动器/自检 CLI（#314：Skill 缺启动器文档的 CLI 面）。
//!
//! 六场黑盒（#269）的 prompt 都是人工写「用 mcp 启动器连接 …」——真实用户
//! 拿到 Skill 不知道怎么配 MCP、binary 在哪。本 bin 补两个出口：
//!
//! - `bid-mcp --init [--client claude|cursor|continue|json] [--ws <dir>]`
//!   生成 MCP 客户端配置模板（stdout，直接粘贴；command 填本目录下
//!   `bid-mcp-server` 的绝对路径——标准 MCP JSON-RPC 2.0 传输，Claude
//!   Desktop / Cursor / Continue 可直连）。
//! - `bid-mcp --doctor [--ws <dir>]`
//!   自检：server binary 在位 → 工具面真跑（进程内 dispatch list_tools，
//!   非 echo 假绿）→ 工作区可写性 → 导入围栏（BID_FS_ROOT/BID_WS）→
//!   Windows 编码提示。FAIL 给修复指引，退出码 1；WARN 不阻断。
//!
//! 无参数 = 帮助。行协议（`bid-mcp-stdio`）与标准 MCP（`bid-mcp-server`）
//! 两个传输 bin 保持一字不动——本 bin 只做启动器辅助，不参与协议面。
//!
//! 文档面：`sdk/skill/bid-response-pipeline/SKILL.md`「快速开始」节 +
//! `docs/agent-integration.md` §2（单一真源引用，本文档不复制细节）。

use std::path::{Path, PathBuf};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--init") => run_init(&args[1..]),
        Some("--doctor") => run_doctor(&args[1..]),
        Some("--help") | Some("-h") | None => print_help(),
        Some(other) => {
            eprintln!("bid-mcp: 未知参数 {other}（支持 --init / --doctor / --help）");
            exit(2);
        }
    }
}

fn print_help() {
    println!(
        "bid-mcp —— 启动器/自检 CLI（bid-response-pipeline Skill 的连接面，#314）\n\
         \n\
         用法：\n\
         \x20 bid-mcp --init [--client claude|cursor|continue|json] [--ws <dir>] [--bin <server>]\n\
         \x20       生成 MCP 客户端配置模板（stdout，粘贴到客户端配置文件）\n\
         \x20 bid-mcp --doctor [--ws <dir>]\n\
         \x20       自检：binary / 工具面（真跑 list_tools）/ 工作区 / 导入围栏\n\
         \x20 bid-mcp --help\n\
         \n\
         传输 bin（本 CLI 只辅助，不参与协议）：\n\
         \x20 bid-mcp-server   标准 MCP（JSON-RPC 2.0 + initialize/tools/list/tools/call）\n\
         \x20                   ——Claude Desktop / Cursor / Continue 等 MCP 客户端连这个\n\
         \x20 bid-mcp-stdio    行协议（逐行 {{\"tool\",\"args\"}} 裸 JSON）——e2e 与桌面壳兼容面\n\
         \n\
         环境变量：\n\
         \x20 BID_WS=<dir>      工作区（持久化 db.json；不设 = 纯内存态，退出即丢）\n\
         \x20 BID_FS_ROOT=<dir> 导入围栏根（import_tender_files 的 upload_ref 须落在其内）\n\
         \n\
         详细指南：docs/agent-integration.md §2 / Skill「快速开始」节"
    );
}

// ---------------------------------------------------------------- --init

/// 解析 --init 的可选参数。
struct InitArgs {
    client: String,
    ws: Option<PathBuf>,
    server_bin: Option<PathBuf>,
}

fn run_init(rest: &[String]) {
    let mut a = InitArgs {
        client: "claude".to_string(),
        ws: None,
        server_bin: None,
    };
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--client" | "-c" => match it.next() {
                Some(v) => a.client = v.clone(),
                None => {
                    eprintln!("bid-mcp --init: --client 需要值（claude|cursor|continue|json）");
                    exit(2);
                }
            },
            "--ws" => match it.next() {
                Some(v) => a.ws = Some(PathBuf::from(v)),
                None => {
                    eprintln!("bid-mcp --init: --ws 需要目录路径");
                    exit(2);
                }
            },
            "--bin" => match it.next() {
                Some(v) => a.server_bin = Some(PathBuf::from(v)),
                None => {
                    eprintln!("bid-mcp --init: --bin 需要 bid-mcp-server 路径");
                    exit(2);
                }
            },
            other => {
                eprintln!("bid-mcp --init: 未知参数 {other}");
                exit(2);
            }
        }
    }
    let server = a.server_bin.clone().unwrap_or_else(default_server_path);
    if !server.is_file() {
        eprintln!(
            "bid-mcp --init: 找不到 {} —— 先构建：cargo build --release -p bid-mcp \
             （或用 --bin 指定 bid-mcp-server 绝对路径）",
            server.display()
        );
        exit(1);
    }
    let server = absolutize(&server);
    let ws_json = a.ws.as_ref().map(|w| absolutize(w).to_string_lossy().to_string());
    print_client_config(&a.client, &server, ws_json.as_deref());
}

/// 默认 server 路径 = 本 bin 同目录下的 bid-mcp-server（cargo build 产物布局）。
fn default_server_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("bid-mcp --init: 无法定位自身路径: {e}");
        exit(1);
    });
    let name = if cfg!(windows) {
        "bid-mcp-server.exe"
    } else {
        "bid-mcp-server"
    };
    exe.parent().map(|p| p.join(name)).unwrap_or_else(|| PathBuf::from(name))
}

fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

/// 按客户端打印配置模板（三种主流客户端 + 通用 JSON）。
///
/// 三客户端配置形状同源（mcpServers.<name>.command/args/env）；差异在
/// 配置文件的落点——模板头部注释写明各家路径。
fn print_client_config(client: &str, server: &Path, ws: Option<&str>) {
    let dest = match client {
        "claude" => {
            // Claude Desktop: claude_desktop_config.json（路径见注释，跨平台不同）
            #[cfg(target_os = "macos")]
            let d = "~/Library/Application Support/Claude/claude_desktop_config.json";
            #[cfg(target_os = "windows")]
            let d = "%APPDATA%\\Claude\\claude_desktop_config.json";
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let d = "~/.config/claude/claude_desktop_config.json（Linux 社区客户端路径，以所用客户端文档为准）";
            d
        }
        "cursor" => "<项目根>/.cursor/mcp.json（或 ~/.cursor/mcp.json 全局）",
        "continue" => "~/.continue/config.json（mcpServers 节）",
        "json" => "（通用 JSON——任何支持 mcpServers 形状的客户端）",
        other => {
            eprintln!("bid-mcp --init: 未知客户端 {other}（支持 claude|cursor|continue|json）");
            exit(2);
        }
    };
    let env_block = match ws {
        Some(w) => format!(
            ",\n      \"env\": {{\n        \"BID_WS\": {}\n      }}",
            serde_json::json!(w)
        ),
        None => String::new(),
    };
    println!(
        "{{\n  \"mcpServers\": {{\n    \"bid\": {{\n      \"command\": {},\n      \"args\": []{}\n    }}\n  }}\n}}",
        serde_json::json!(server.to_string_lossy()),
        env_block
    );
    eprintln!(
        "\n# 配置落点（{client}）：{dest}\n\
         # 建议设 BID_WS（工作区持久化；不设 = 纯内存态，退出即丢）——用 --ws <dir> 重新生成\n\
         # 验证：客户端连上后调 tools/list（或行协议 list_tools），应答 ok=true 且 count>0\n\
         # 下一步：把 SKILL.md 交给 Agent，说「用 bid 这个 MCP 帮我做投标文件」"
    );
}

// ---------------------------------------------------------------- --doctor

struct CheckResult {
    ok: usize,
    fail: usize,
    warn: usize,
}

impl CheckResult {
    fn record(&mut self, level: &str, name: &str, detail: &str, fix: Option<&str>) {
        println!("[{level}] {name}: {detail}");
        if let Some(f) = fix {
            println!("       修复指引: {f}");
        }
        match level {
            "OK" => self.ok += 1,
            "FAIL" => self.fail += 1,
            _ => self.warn += 1,
        }
    }
}

fn run_doctor(rest: &[String]) {
    let mut ws_arg: Option<PathBuf> = None;
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--ws" => match it.next() {
                Some(v) => ws_arg = Some(PathBuf::from(v)),
                None => {
                    eprintln!("bid-mcp --doctor: --ws 需要目录路径");
                    exit(2);
                }
            },
            other => {
                eprintln!("bid-mcp --doctor: 未知参数 {other}");
                exit(2);
            }
        }
    }

    println!("bid-mcp doctor —— 自检（#314）\n");
    let mut r = CheckResult { ok: 0, fail: 0, warn: 0 };

    // ① 传输 binary 在位（server = 标准 MCP 客户端入口；stdio = 行协议兼容面）
    let server = default_server_path();
    if server.is_file() {
        r.record("OK", "bid-mcp-server", &format!("{}（在位）", server.display()), None);
    } else {
        r.record(
            "FAIL",
            "bid-mcp-server",
            &format!("找不到 {}", server.display()),
            Some("cargo build --release -p bid-mcp（在 bidcore/ 下执行；产物在 target/release/）"),
        );
    }
    let stdio = server.parent().map(|p| {
        p.join(if cfg!(windows) { "bid-mcp-stdio.exe" } else { "bid-mcp-stdio" })
    });
    match stdio {
        Some(p) if p.is_file() => {
            r.record("OK", "bid-mcp-stdio", &format!("{}（在位）", p.display()), None);
        }
        _ => {
            r.record(
                "WARN",
                "bid-mcp-stdio",
                "行协议 bin 不在本目录（仅影响 e2e/桌面壳兼容面，不影响 MCP 客户端）",
                None,
            );
        }
    }

    // ② 工具面真跑：进程内 dispatch list_tools（不是 echo——真实穿透分发层）
    let reply = bid_mcp::dispatch_json(r#"{"tool":"list_tools","args":{}}"#);
    match serde_json::from_str::<serde_json::Value>(&reply) {
        Ok(v) if v.get("ok").and_then(|o| o.as_bool()) == Some(true) => {
            let count = v
                .pointer("/data/count")
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            if count > 0 {
                r.record(
                    "OK",
                    "工具面",
                    &format!("list_tools 应答 ok=true，{count} 个工具（单一真源 TOOL_NAMES）"),
                    None,
                );
            } else {
                r.record("FAIL", "工具面", "list_tools count=0（TOOL_NAMES 为空？）", Some("报告维护方——TOOL_NAMES 不应为空"));
            }
        }
        Ok(v) => {
            r.record(
                "FAIL",
                "工具面",
                &format!("list_tools 应答 ok=false：{v}"),
                Some("带应答原文报告维护方"),
            );
        }
        Err(e) => {
            r.record("FAIL", "工具面", &format!("应答非 JSON（{e}）：{reply}"), Some("报告维护方——分发层损坏"));
        }
    }

    // ③ 工作区（--ws > BID_WS；未设 = 纯内存态提醒）
    let ws_env_set = ws_arg.is_some() || std::env::var_os("BID_WS").is_some();
    let ws: Option<PathBuf> = ws_arg
        .clone()
        .or_else(|| std::env::var_os("BID_WS").map(PathBuf::from));
    match ws {
        Some(w) => {
            if !w.is_dir() {
                r.record(
                    "FAIL",
                    "工作区",
                    &format!("{} 不存在", w.display()),
                    Some("mkdir -p <dir>（工作区须预先存在且可写）"),
                );
            } else {
                match probe_writable(&w) {
                    Ok(()) => r.record("OK", "工作区", &format!("{} 存在且可写", w.display()), None),
                    Err(e) => r.record(
                        "FAIL",
                        "工作区",
                        &format!("{} 不可写（{e}）", w.display()),
                        Some("检查目录权限（chown/chmod）；工作区不可写时 server 启动即 panic（不静默降级）"),
                    ),
                }
            }
        }
        None => {
            r.record(
                "WARN",
                "工作区",
                "未设置 BID_WS——纯内存态，进程退出数据即丢（冒烟可用，正式使用必须设）",
                Some("export BID_WS=<dir>（或客户端配置 env 块里加 BID_WS）"),
            );
        }
    }

    // ④ 导入围栏（BID_FS_ROOT ∪ BID_WS；两者皆无 → import 响亮拒）
    let fs_root_set = std::env::var_os("BID_FS_ROOT").is_some();
    if !fs_root_set && !ws_env_set {
        r.record(
            "WARN",
            "导入围栏",
            "未设置 BID_FS_ROOT 且未设置 BID_WS——import_tender_files 将响亮拒",
            Some("export BID_FS_ROOT=<招标文件素材目录>（或设 BID_WS 后把素材放进工作区）"),
        );
    } else {
        r.record("OK", "导入围栏", "已设置（upload_ref 须落在受信根内）", None);
    }

    // ⑤ Windows 编码（中文乱码历史坑——主动提示而非等炸）
    #[cfg(windows)]
    r.record(
        "WARN",
        "终端编码",
        "Windows 控制台默认 GBK——中文输出可能乱码",
        Some("chcp 65001（或用 Windows Terminal / 设 UTF-8 终端）"),
    );
    #[cfg(not(windows))]
    let _ = ();

    println!(
        "\n{} 项通过，{} 项失败，{} 项提醒 —— {}",
        r.ok,
        r.fail,
        r.warn,
        if r.fail == 0 { "健康（提醒不阻断）" } else { "存在失败项（按修复指引处理后重跑）" }
    );
    if r.fail > 0 {
        exit(1);
    }
}

/// 工作区可写探测：建临时探测文件再删（立即清理，不留垃圾）。
fn probe_writable(dir: &Path) -> std::io::Result<()> {
    let probe = dir.join(".bid-mcp-doctor-probe");
    std::fs::write(&probe, b"probe")?;
    std::fs::remove_file(&probe)
}
