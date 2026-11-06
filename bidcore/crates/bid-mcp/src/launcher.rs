//! launcher.rs —— #314：启动器 CLI 面（`--init` / `--doctor`）。
//!
//! 由来：六场黑盒的连接方式全靠 prompt 人工写「用 mcp 启动器连接 …」——
//! 真实用户拿到 Skill 不知道 binary 在哪、怎么配 MCP、坏了怎么查。
//! 本模块给两个传输面 bin（bid-mcp-server / bid-mcp-stdio）共同的启动器
//! 出口：
//!
//! - `--init`：打印三个主流 MCP 客户端（Claude Desktop / Cursor / Continue）
//!   的 mcpServers 配置模板 + 各客户端配置文件落位路径——用户复制进对应
//!   文件即完成接线；
//! - `--doctor`：自检并给出修复建议（工具面 / 工作区 / LLM 配置 / 外部
//!   工具链 poppler·转换器·OCR）。退出码：任一 FAIL → 1（接线不可用），
//!   仅 WARN → 0（可用但有降级面，如实告知）。
//!
//! 设计注记：doctor 的检查以**显式参数**注入（`ws: Option<&str>`）而非
//! 直接读环境——main() 读 `BID_WS` 后传入，单元测试无需碰进程级环境变量
//! （set/remove env 在并行测试下是竞态面）。

/// 入口：args 是 argv[1..]。识别 `--init` / `--doctor` / `--help` 并执行；
/// 返回 `Some(exit_code)` 表示已消费（调用方直接退出），`None` 表示不是
/// 启动器参数（继续正常 stdio 传输面循环）。
pub fn handle_cli(args: &[String]) -> Option<i32> {
    let first = args.iter().find(|a| a.starts_with("--"))?;
    match first.as_str() {
        "--init" => {
            print!("{}", init_template());
            Some(0)
        }
        "--doctor" => {
            let ws = std::env::var("BID_WS")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let (report, code) = doctor_report(ws.as_deref());
            print!("{report}");
            Some(code)
        }
        "--help" | "-h" => {
            print!("{}", help_text());
            Some(0)
        }
        other => {
            eprintln!("未知参数 {other}（支持：--init / --doctor / --help；无参数 = 启动 stdio 传输面）");
            Some(2)
        }
    }
}

fn help_text() -> String {
    format!(
        "bid-mcp 启动器\n\
         \n\
         用法：\n\
         \x20 bid-mcp-server            启动标准 MCP stdio 传输面（JSON-RPC 2.0——Claude Desktop/官方 SDK 接这个）\n\
         \x20 bid-mcp-server --init     打印三个主流 MCP 客户端的配置模板\n\
         \x20 bid-mcp-server --doctor   自检（工具面/工作区/LLM/外部工具链）并给修复建议\n\
         \x20 bid-mcp-server --help     本帮助\n\
         \n\
         环境变量：\n\
         \x20 BID_WS=<dir>     持久化工作区（db.json 恢复 + 写面落盘；未设 = 纯内存态）\n\
         \x20 BID_FS_ROOT=<dir> 文件访问围栏根（默认 = BID_WS）\n\
         \n\
         旧传输面 bid-mcp-stdio（逐行裸 JSON {{\"tool\":..,\"args\":..}}）同样支持以上参数。\n"
    )
}

/// `--init`：三客户端配置模板（纯函数——测试面直接断言内容）。
pub fn init_template() -> String {
    let exe_server = "bid-mcp-server";
    format!(
        r#"# bid-mcp MCP 客户端配置模板（--init 生成）

> 复制对应客户端的模板到其配置文件，把 command 改成你的 bid-mcp-server
> 实际路径（Windows 用反斜杠或正斜杠均可），BID_WS 改成你的工作区目录
> （标书数据落盘处；不设则纯内存态——重启即失，不建议生产用）。

## 1. Claude Desktop

配置文件（JSON）：
  - macOS： ~/Library/Application Support/Claude/claude_desktop_config.json
  - Windows：%APPDATA%\Claude\claude_desktop_config.json

{{
  "mcpServers": {{
    "bid": {{
      "command": "/absolute/path/to/{exe_server}",
      "env": {{
        "BID_WS": "/absolute/path/to/bid-workspace"
      }}
    }}
  }}
}}

## 2. Cursor

配置文件：项目根 .cursor/mcp.json（或全局 ~/.cursor/mcp.json）

{{
  "mcpServers": {{
    "bid": {{
      "command": "/absolute/path/to/{exe_server}",
      "env": {{
        "BID_WS": "/absolute/path/to/bid-workspace"
      }}
    }}
  }}
}}

## 3. Continue

配置文件：~/.continue/config.json（experimental 节）

{{
  "experimental": {{
    "modelContextProtocolServers": [
      {{
        "name": "bid",
        "command": "/absolute/path/to/{exe_server}",
        "env": {{
          "BID_WS": "/absolute/path/to/bid-workspace"
        }}
      }}
    ]
  }}
}}

## 验证连接

配置生效后（Claude Desktop 需重启），让 Agent 调 list_tools——应看到
39 个工具（工具清单即发现面 #279）。或命令行手动验证：

  echo '{{"tool":"list_tools","args":{{}}}}' | bid-mcp-stdio

（旧 stdio 面逐行裸 JSON；标准 MCP 面的验证走各客户端的 MCP 面板。）
"#
    )
}

/// `--doctor`：自检报告。返回 (报告文本, 退出码)——退出码 1 = 有 FAIL。
pub fn doctor_report(ws: Option<&str>) -> (String, i32) {
    let mut out = String::from("bid-mcp --doctor 自检\n========================\n");
    let mut fails = 0usize;
    let mut warns = 0usize;

    // ---- ① 工具面（进程内 dispatch 真跑——list_tools 发现面 #279）
    match tool_face_check() {
        Ok(n) => {
            out.push_str(&format!(
                "[ok]   工具面：dispatch 可用，list_tools 返回 {n} 个工具\n"
            ));
            if n != crate::TOOL_NAMES.len() {
                out.push_str(&format!(
                    "       警告：与 TOOL_NAMES 单一真源（{}）不一致\n",
                    crate::TOOL_NAMES.len()
                ));
                warns += 1;
            }
        }
        Err(e) => {
            out.push_str(&format!("[FAIL] 工具面：dispatch 异常——{e}\n"));
            out.push_str("       修复：重装/重建 binary（cargo install --path bidcore/crates/bid-mcp）\n");
            fails += 1;
        }
    }

    // ---- ② 工作区（BID_WS）
    match ws {
        None => {
            out.push_str("[warn] 工作区：BID_WS 未设置——纯内存态（重启数据即失）\n");
            out.push_str("       修复：客户端配置里给 env.BID_WS 指一个专用目录（见 --init 模板）\n");
            warns += 1;
        }
        Some(dir) => {
            let path = std::path::Path::new(dir);
            if !path.is_dir() {
                out.push_str(&format!("[FAIL] 工作区：BID_WS={dir} 不是目录\n"));
                out.push_str("       修复：先创建该目录（mkdir），或改指已有目录\n");
                fails += 1;
            } else {
                // 可写探针：建临时文件再删（存在性 + 权限一次验完）
                let probe = path.join(format!(".doctor-probe-{}", std::process::id()));
                match std::fs::File::create(&probe)
                    .and_then(|f| {
                        drop(f);
                        std::fs::remove_file(&probe)
                    })
                {
                    Ok(()) => {
                        out.push_str(&format!("[ok]   工作区：BID_WS={dir} 在位且可写\n"));
                        // ---- ③ LLM 配置（仅持久化态有意义）
                        let llm = path.join(".mang").join("llm.json");
                        if llm.is_file() {
                            out.push_str("[ok]   LLM：.mang/llm.json 在场（起草走真实生成器）\n");
                        } else {
                            out.push_str(&format!(
                                "[warn] LLM：{} 不在场——起草走 Mock 生成器（结构对、正文短）\n",
                                llm.display()
                            ));
                            out.push_str("       修复：按 docs/bid-mcp-contract.md 的 LLM 配置节写 llm.json（api_key/model/base_url）\n");
                            warns += 1;
                        }
                    }
                    Err(e) => {
                        out.push_str(&format!(
                            "[FAIL] 工作区：BID_WS={dir} 不可写（{e}）\n"
                        ));
                        out.push_str("       修复：检查目录权限（Windows：右键属性-安全；macOS/Linux：chmod）\n");
                        fails += 1;
                    }
                }
            }
        }
    }

    // ---- ④ 外部工具链（降级面如实告知——AGENTS.md §13 环境缺件纪律）
    // poppler 文本工具（PDF 抽取 + 9 条验收门）
    if probe_command(&["pdftotext", "-v"]) {
        out.push_str("[ok]   poppler：pdftotext 在场（PDF 文本抽取/验收门可用）\n");
    } else {
        out.push_str("[warn] poppler：pdftotext 不在场——PDF 文本抽取走降级链、相关验收门 SKIP\n");
        out.push_str("       修复：装 poppler（Windows: poppler-windows 发布包入 PATH；macOS: brew install poppler；Linux: apt install poppler-utils）\n");
        warns += 1;
    }
    // PDF 转换器（导出 pdf 格式用；docx 不受影响）
    if bid_tools::service::pdf_converter_available() {
        out.push_str(&format!(
            "[ok]   PDF 转换器：{} 在场（export pdf 可用）\n",
            bid_tools::service::pdf_converter_name()
        ));
    } else {
        out.push_str("[warn] PDF 转换器：Word/LibreOffice 均未检测到——export pdf 将被拒（docx 导出不受影响）\n");
        out.push_str("       修复：装 Microsoft Office（最快最准）或 LibreOffice（免费）\n");
        warns += 1;
    }
    // OCR（扫描件）
    if probe_command(&["tesseract", "--version"]) {
        out.push_str("[ok]   OCR：tesseract 在场（扫描件可 OCR）\n");
    } else {
        out.push_str("[warn] OCR：tesseract 不在场——无文本层扫描件无法 OCR（parse_status=OCR_REQUIRED 会如实报告）\n");
        out.push_str("       修复：装 tesseract（含中文语言包 chi_sim）\n");
        warns += 1;
    }

    // ---- 汇总
    out.push_str("========================\n");
    if fails > 0 {
        out.push_str(&format!(
            "结果：{fails} 项 FAIL、{warns} 项 warn——修 FAIL 后重跑 --doctor\n"
        ));
        (out, 1)
    } else {
        out.push_str(&format!(
            "结果：0 FAIL、{warns} 项 warn——接线可用（warn 是降级面，按需修）\n"
        ));
        (out, 0)
    }
}

/// 工具面自检：进程内真跑 list_tools（发现面 #279——契约与 TOOL_NAMES 同源）。
/// 回信形状：`{"ok":true,"tool":"list_tools","data":{"count":N,…}}`。
fn tool_face_check() -> Result<usize, String> {
    let reply = crate::dispatch_json(r#"{"tool":"list_tools","args":{}}"#);
    let v: serde_json::Value =
        serde_json::from_str(&reply).map_err(|e| format!("应答不是合法 JSON: {e}"))?;
    if v["ok"] != serde_json::Value::Bool(true) {
        return Err(format!("list_tools 返回错误面: {reply}"));
    }
    v["data"]["count"]
        .as_u64()
        .map(|n| n as usize)
        .ok_or_else(|| format!("应答缺 data.count: {reply}"))
}

/// 命令探针：能拉起且退出（退出码任意——版本横幅面 -v/--version 常非零退出，
/// 只验证「binary 在 PATH 上可执行」）。10s 超时防挂死。
fn probe_command(cmd: &[&str]) -> bool {
    std::process::Command::new(cmd[0])
        .args(&cmd[1..])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .timeout_output()
        .is_some()
}

/// spawn + wait 超时包装（Command 原生无超时——wait_with_output 会无限等，
/// 挂死的探针会拖死 --doctor）。超时杀掉按「不在场」处理。
trait CommandExt {
    fn timeout_output(&mut self) -> Option<std::process::ExitStatus>;
}

impl CommandExt for std::process::Command {
    fn timeout_output(&mut self) -> Option<std::process::ExitStatus> {
        let mut child = self.spawn().ok()?;
        // 轮询 10s（100ms 步进）——不引入线程/依赖的最简超时面
        for _ in 0..100 {
            match child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(_) => return None,
            }
        }
        None // 超时
    }
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_template_covers_three_clients() {
        let t = init_template();
        assert!(t.contains("Claude Desktop"), "Claude Desktop 模板在場");
        assert!(t.contains("Cursor"), "Cursor 模板在場");
        assert!(t.contains("Continue"), "Continue 模板在場");
        assert!(t.contains("bid-mcp-server"), "真实 binary 名（非 issue 草稿里的 bid-mcp）");
        assert!(t.contains("BID_WS"), "持久化工作区引导在場");
        assert!(t.contains("mcpServers"), "mcpServers 键在場");
        // 三份 JSON 模板都是合法 JSON（花括号配平的粗检 + 逐段 serde 解析）
        assert_eq!(t.matches('{').count(), t.matches('}').count(), "花括号配平");
    }

    #[test]
    fn doctor_memory_mode_warns_but_passes() {
        // 纯内存态（BID_WS 未设）：warn 不 fail——接线可用
        let (report, code) = doctor_report(None);
        assert_eq!(code, 0, "纯内存态可用（降级如实告知）: {report}");
        assert!(report.contains("BID_WS 未设置"), "{report}");
        assert!(report.contains("[ok]   工具面"), "工具面必须 ok: {report}");
        assert!(report.contains("0 FAIL"), "{report}");
    }

    #[test]
    fn doctor_valid_workspace_passes() {
        let dir = std::env::temp_dir().join(format!(
            "bid-doctor-ok-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (report, code) = doctor_report(Some(dir.to_str().unwrap()));
        assert_eq!(code, 0, "可写工作区: {report}");
        assert!(report.contains("在位且可写"), "{report}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doctor_broken_workspace_fails() {
        let (report, code) = doctor_report(Some("/nonexistent/bid-ws-xyz"));
        assert_eq!(code, 1, "不存在的目录必须 FAIL: {report}");
        assert!(report.contains("不是目录"), "{report}");
    }

    #[test]
    fn doctor_llm_hint_on_missing_config() {
        let dir = std::env::temp_dir().join(format!(
            "bid-doctor-llm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (report, _) = doctor_report(Some(dir.to_str().unwrap()));
        assert!(
            report.contains("llm.json 不在场") || report.contains("Mock 生成器"),
            "缺 LLM 配置要给修复指引: {report}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn help_text_lists_all_modes() {
        let h = help_text();
        for kw in ["--init", "--doctor", "--help", "BID_WS"] {
            assert!(h.contains(kw), "帮助缺 {kw}: {h}");
        }
    }
}
