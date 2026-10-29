//! mang-app —— 标书桌面壳
//!
//! 定位：
//!   - **人用**的只读预览窗口，不是编辑器
//!   - AI 子代理通过 MCP 调 bid-mcp 写数据，本壳只负责显示
//!   - 直链 bidcore（不经 MCP 层，方案 A）
//!
//! 技术栈：tao（窗口）+ wry（WebView2）
//!   - custom protocol 加载 UI（避开 file:// 的 CORS）
//!   - custom protocol /__ipc/<id>.json 取大响应（读一次即删）
//!   - IPC：JS postMessage(json) → __ipcResolve(id, resp, ok) 注入回 JS
//!   - 事件：__ipcEvent({ev:"drop",...})
//!
//! 命令行：
//!   mang-app.exe [DIR] [--workspace DIR] [--ui DIR] [--devtools] [--selftest] [--ls-ui] [--help]
//!
//! 协议细节见 docs/mang/IPC-CONTRACT.md。

mod b64;
mod chrome;
mod dialogs;
mod dnd;
mod fsapi;
mod gitlog;
mod ipc;
mod paths;
mod recent;
mod selftest;
mod shell;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy},
    window::{Window, WindowBuilder},
};
use wry::{WebView, WebViewBuilder};

#[cfg(windows)]
use tao::platform::windows::WindowBuilderExtWindows;

// ================================================================= 命令行

const USAGE: &str = r#"mang-app —— 标书工作台桌面壳

用法：
  mang-app.exe [DIR] [选项]

选项：
  -w, --workspace <DIR>   工作区目录（默认：当前目录）。也可以直接 mang-app.exe DIR
      --ui <DIR>          UI 目录（默认：<exe>/ui → <exe>/../../web-ui/dist → 当前目录 web-ui/dist）
      --devtools          启用 DevTools
      --selftest          不开窗口，跑 IPC 自检并打印 PASS/FAIL（退出码 0=全过）
      --ls-ui             只打印解析出的 UI 目录路径然后退出（脚本用）
      --inject-drop <P1;P2> 启动 1.5s 后模拟一次拖放事件（自测/无鼠标环境验证 DnD 用）
  -h, --help              打印本帮助

示例：
  mang-app.exe --workspace D:\项目A
  mang-app.exe D:\项目A --devtools
"#;

#[derive(Debug, Clone)]
pub struct Cli {
    pub workspace: Option<PathBuf>,
    pub ui: Option<PathBuf>,
    pub devtools: bool,
    pub selftest: bool,
    pub ls_ui: bool,
    /// --inject-drop 的路径列表（分号分隔）；仅用于自测
    pub inject_drop: Option<Vec<PathBuf>>,
}

pub enum CliOutcome {
    Run(Cli),
    Help,
    ArgError(String),
}

pub fn parse_cli<I: IntoIterator<Item = String>>(args: I) -> CliOutcome {
    let mut cli = Cli {
        workspace: None,
        ui: None,
        devtools: false,
        selftest: false,
        ls_ui: false,
        inject_drop: None,
    };
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" | "/?" => return CliOutcome::Help,
            "-w" | "--workspace" => match it.next() {
                Some(v) => cli.workspace = Some(PathBuf::from(v)),
                None => return CliOutcome::ArgError("--workspace 缺少参数".into()),
            },
            "--ui" => match it.next() {
                Some(v) => cli.ui = Some(PathBuf::from(v)),
                None => return CliOutcome::ArgError("--ui 缺少参数".into()),
            },
            "--devtools" | "--dev" => cli.devtools = true,
            "--selftest" => cli.selftest = true,
            "--ls-ui" => cli.ls_ui = true,
            "--inject-drop" => match it.next() {
                Some(v) => {
                    cli.inject_drop = Some(
                        v.split(';')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(PathBuf::from)
                            .collect(),
                    )
                }
                None => return CliOutcome::ArgError("--inject-drop 缺少参数".into()),
            },
            other if other.starts_with("--") => {
                return CliOutcome::ArgError(format!("未知选项：{other}"))
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return CliOutcome::ArgError(format!("未知选项：{other}"))
            }
            other => {
                // 位置参数 = 工作区；--workspace 优先
                if cli.workspace.is_none() {
                    cli.workspace = Some(PathBuf::from(other));
                } else {
                    eprintln!("[mang] 忽略多余参数：{other}");
                }
            }
        }
    }
    CliOutcome::Run(cli)
}

/// UI 目录候选（按优先级）
fn ui_candidates(explicit: Option<&Path>) -> Vec<PathBuf> {
    if let Some(p) = explicit {
        return vec![p.to_path_buf()];
    }
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("ui"));
            // <repo>/bidcore/target/release/mang-app.exe → <repo>/web-ui/dist
            out.push(
                dir.join("..")
                    .join("..")
                    .join("..")
                    .join("web-ui")
                    .join("dist"),
            );
            out.push(dir.join("..").join("..").join("web-ui").join("dist"));
        }
    }
    out.push(PathBuf::from("web-ui/dist"));
    out.push(PathBuf::from("dist"));
    out
}

fn resolve_ui(explicit: Option<&Path>) -> Option<PathBuf> {
    for c in ui_candidates(explicit) {
        if c.join("index.html").exists() {
            return Some(std::fs::canonicalize(&c).unwrap_or(c));
        }
    }
    None
}

// ================================================================= 事件

enum UserEvent {
    /// 有新活要干（对话框出队 + 把注入队列写进 WebView）
    Wake,
}

/// 需要主线程执行的请求（对话框）
struct MainJob {
    id: Value,
    cmd: String,
    body: Arc<serde_json::Map<String, Value>>,
}

/// IPC 线程 → 主线程的注入脚本投递箱
fn outbox() -> &'static Mutex<Vec<String>> {
    static OUTBOX: Mutex<Vec<String>> = Mutex::new(Vec::new());
    &OUTBOX
}

fn outbox_push(script: String) {
    let mut q = outbox().lock().unwrap_or_else(|e| e.into_inner());
    if q.len() > 4096 {
        eprintln!("[mang-ipc] 注入队列过长，丢弃最旧的 {}", q.len() - 4096);
        let drop_n = q.len() - 4096;
        q.drain(..drop_n);
    }
    q.push(script);
}

fn outbox_drain() -> Vec<String> {
    let mut q = outbox().lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *q)
}

static DROP_Q: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// 拖放事件入队（UI 线程写、UI 线程读）
fn drop_queue_push(ev: Value) {
    let mut q = DROP_Q.lock().unwrap_or_else(|e| e.into_inner());
    if q.len() < 32 {
        q.push(ev);
    }
}

fn drop_queue_drain() -> Vec<Value> {
    let mut q = DROP_Q.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *q)
}

// ================================================================= 主程序

fn main() {
    let cli = match parse_cli(std::env::args().skip(1)) {
        CliOutcome::Help => {
            print!("{USAGE}");
            std::process::exit(0);
        }
        CliOutcome::ArgError(e) => {
            eprintln!("[mang] 参数错误：{e}\n");
            eprint!("{USAGE}");
            std::process::exit(2);
        }
        CliOutcome::Run(c) => c,
    };

    // 工作区
    let ws_path = cli
        .workspace
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let ws_path = if ws_path.is_absolute() {
        ws_path
    } else {
        std::env::current_dir()
            .map(|d| d.join(&ws_path))
            .unwrap_or(ws_path)
    };

    if cli.ls_ui {
        match resolve_ui(cli.ui.as_deref()) {
            Some(p) => println!("{}", paths::display(&p)),
            None => {
                eprintln!(
                    "[mang] 找不到 UI 目录（候选：{:?}）",
                    ui_candidates(cli.ui.as_deref())
                );
                std::process::exit(1);
            }
        }
        return;
    }

    if cli.selftest {
        let ui = resolve_ui(cli.ui.as_deref());
        let code = selftest::run(ui.as_deref());
        std::process::exit(code);
    }

    let workspace = match paths::Workspace::new(&ws_path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[mang] 工作区无效：{e}");
            eprintln!("[mang] 路径：{}", paths::display(&ws_path));
            std::process::exit(1);
        }
    };

    let Some(ui_dir) = resolve_ui(cli.ui.as_deref()) else {
        eprintln!("[mang] 找不到 UI：尝试过");
        for c in ui_candidates(cli.ui.as_deref()) {
            eprintln!("        {}", c.display());
        }
        eprintln!("[mang] 请先 cd web-ui && node mang-build.mjs，或用 --ui 指定目录");
        std::process::exit(1);
    };

    println!(
        "[mang] version {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );
    println!("[mang] workspace = {}", workspace.root_display());
    println!("[mang] ui        = {}", paths::display(&ui_dir));
    // 注意：**不**在启动时创建 <ws>/.mang —— 工作区常常是 git 跟踪的 fixture 目录，
    // 冒出一个新目录很脏。.mang/history 由第一次真正写历史时按需创建（见 fs.writeBytes）。
    // 清理上次运行遗留的 IPC 临时文件
    ipc::purge_pending();

    if let Err(e) = run_window(workspace, ui_dir, cli.devtools, cli.inject_drop.clone()) {
        eprintln!("[mang] 启动失败：{e}");
        std::process::exit(1);
    }
}

type SharedWebview = Arc<Mutex<WebView>>;

static PROXY: std::sync::OnceLock<EventLoopProxy<UserEvent>> = std::sync::OnceLock::new();

/// 初始窗口尺寸：主屏工作区的 80%，至少 1024x640（规格里的最小尺寸）。
/// 去边框窗口的 inner size 会被当成 outer size 用，所以必须按屏幕算而不是写死。
#[cfg(windows)]
fn screen_work_area() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    let w = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(1024);
    let h = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(640);
    (w, h)
}
fn initial_size() -> tao::dpi::PhysicalSize<u32> {
    #[cfg(windows)]
    {
        let (sw, sh) = screen_work_area();
        // 用物理像素：LogicalSize 会被 1.25/1.5 倍缩放放大，导致窗口比屏幕还大、
        // 最大化前后看起来"没变化"。
        let w = ((sw as f64 * 0.8) as u32).clamp(1024, (sw as u32).saturating_sub(60));
        let h = ((sh as f64 * 0.78) as u32).clamp(640, (sh as u32).saturating_sub(120));
        return tao::dpi::PhysicalSize::new(w, h);
    }
    #[cfg(not(windows))]
    {
        tao::dpi::PhysicalSize::new(1440, 900)
    }
}
fn run_window(
    workspace: paths::Workspace,
    ui_dir: PathBuf,
    devtools: bool,
    inject_drop: Option<Vec<PathBuf>>,
) -> wry::Result<()> {
    // 自绘标题栏里「双击最大化」是系统窗口循环处理的，壳拿不到 IPC 回调；
    // 这里给它一个回调，把当前最大化状态推回 UI（右上角按钮图标要跟着换）。
    #[cfg(windows)]
    chrome::set_state_emit(Box::new(|maximized| {
        push_event(serde_json::json!({ "ev": "window", "maximized": maximized }));
        if let Some(p) = PROXY.get() {
            let _ = p.send_event(UserEvent::Wake);
        }
    }));

    let event_loop: EventLoop<UserEvent> = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let _ = PROXY.set(proxy.clone());

    let window = WindowBuilder::new()
        .with_title("mang — 标书工作台")
        // 精确尺寸由 chrome::apply_workarea 在窗口建好后钉一次（见那里的说明）：
        // tao 的 set_inner_size 在「无边框无阴影」时会把尺寸当外框算，这里先给初值，
        // 建好后按客户区尺寸校正。
        .with_inner_size(initial_size())
        .with_min_inner_size(LogicalSize::new(1024.0, 640.0))
        // 自绘标题栏：去掉系统标题栏/边框，改由 UI 的菜单栏当标题栏
        // （拖动/缩放/双击最大化/贴边吸附由 chrome.rs 的 WM_NCHITTEST 交还系统处理）
        .with_decorations(false)
        // ⚠ 关键：必须关掉「无边框阴影」。tao 0.37 的默认值是 true，此时它的
        //   WM_NCCALCSIZE 会按 util::calculate_window_insets() 把客户区按阴影内缩
        //   （本机 1243x682 的窗口算出 1228x673 的客户区），WebView 于是顶在窗口
        //   第 1 像素、把 32px 标题带整条盖住 —— 拖动/双击/上边缩放全部失效的根因。
        //   关掉之后 tao 原样保留 rgrc：整窗 = 客户区，WebView 从 0,0 铺满。
        .with_undecorated_shadow(false)
        // 先隐藏，等 WebView 建好再显示，避免白屏闪一下
        .with_visible(false)
        .build(&event_loop)
        .unwrap_or_else(|e| {
            eprintln!("[mang] 窗口创建失败：{e}");
            std::process::exit(1);
        });

    let ctx = Arc::new(ipc::Ctx::new(workspace.clone(), ui_dir.clone()));
    // IPC 线程 → 主线程 的对话框任务队列
    let dialog_queue: Arc<Mutex<Vec<MainJob>>> = Arc::new(Mutex::new(Vec::new()));
    let webview = build_webview(
        &window,
        &ui_dir,
        Arc::clone(&ctx),
        Arc::clone(&dialog_queue),
        proxy,
        devtools,
    )?;
    let shared: SharedWebview = Arc::new(Mutex::new(webview));

    // DnD：HWND 子类化（WM_DROPFILES）
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowExtWindows;
        let hwnd = window.hwnd();
        let ws_root = workspace.root_canonical().to_path_buf();
        let drop_proxy = {
            // WM_DROPFILES 回调在 UI 线程里，只需把"落下了哪些文件"排进队列，
            // 由事件循环统一注入 JS（避免在消息处理中重入 WebView）。
            let outer = event_loop.create_proxy();
            move |paths: Vec<String>| {
                let ev = dnd::drop_event(paths, Some(&ws_root));
                drop_queue_push(ev);
                let _ = outer.send_event(UserEvent::Wake);
            }
        };
        set_main_hwnd(hwnd);
        // 自绘标题栏：WM_NCHITTEST / WM_NCCALCSIZE 子类（拖动、缩放、双击最大化）
        if chrome::install(hwnd) {
            println!("[mang] chrome    = 自绘标题栏已挂载（WM_NCHITTEST）");
        } else {
            eprintln!("[mang] chrome    = 自绘标题栏挂载失败（退回系统标题栏）");
        }

        let ok = dnd::install(hwnd, Box::new(drop_proxy));
        if ok {
            println!("[mang] DnD       = WM_DROPFILES 已挂载（SetWindowSubclass）");
        } else {
            eprintln!("[mang] DnD       = SetWindowSubclass 失败（拖放不可用）");
        }
    }

    window.set_visible(true);
    println!("[mang] 就绪");

    // --inject-drop：启动 1.5s 后把给定的文件当成一次"拖放"推给页面（自测用，绕开真实鼠标）
    if let Some(files) = inject_drop {
        let paths: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let ws_root = workspace.root_canonical().to_path_buf();
        let tx = event_loop.create_proxy();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let ev = dnd::drop_event(paths, Some(&ws_root));
            println!("[mang] inject-drop → {}", ev);
            drop_queue_push(ev);
            let _ = tx.send_event(UserEvent::Wake);
        });
    }

    run_loop(event_loop, shared, dialog_queue)
}

fn build_webview(
    window: &Window,
    ui_root: &Path,
    ctx: Arc<ipc::Ctx>,
    dialog_queue: Arc<Mutex<Vec<MainJob>>>,
    proxy: EventLoopProxy<UserEvent>,
    devtools: bool,
) -> wry::Result<WebView> {
    let root = ui_root.to_path_buf();
    let webview = WebViewBuilder::new()
        .with_devtools(devtools || cfg!(debug_assertions))
        .with_custom_protocol("mang".into(), move |_id, req| serve(&root, req))
        .with_navigation_handler(|url| {
            // ⚠ 注意：wry 在 Windows 上会把自定义协议改写成 http(s)://<协议>.<host>/…
            //   mang://localhost/index.html → http://mang.localhost/index.html
            // 所以这里必须同时放行 mang:// 与 mang.localhost 两种形态，否则主页面
            // 会被自己拦掉（表现为 WebView 一直 about:blank）。
            let self_hosted = url.starts_with("mang://localhost")
                || url.starts_with("mang.localhost")
                || url.starts_with("http://mang.localhost")
                || url.starts_with("https://mang.localhost")
                || url.starts_with("about:")
                || url.starts_with("blob:")
                || url.starts_with("data:");
            if self_hosted {
                return true;
            }
            // 其余 http(s) 交给系统浏览器；其它协议一律拒绝
            if url.starts_with("http://") || url.starts_with("https://") {
                let _ = open_external(&url);
            }
            false
        })
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            handle_ipc(&ctx, &dialog_queue, &proxy, req.body());
        })
        // 直接加载 UI 页（dist/mang/index.html）——它自带 window.__ipcResolve/__ipcEvent，
        // 不再需要 dist/index.html 那层 iframe 宿主转发。
        .with_url("mang://localhost/mang/index.html")
        // 给 UI 一个「我跑在桌面壳里」的标记。必须用 initialization script：
        // 用 evaluate_script 发得太晚，UI 的 boot() 早就跑完了，
        // 结果窗口按钮不渲染、标题栏按浏览器模式处理。
        .with_initialization_script(
            "try{window.__mangShell={version:1,protocol:1};}catch(e){}",
        )
        .build(window)?;
    Ok(webview)
}

/// IPC handler：跑在 UI 主线程（WebView2 消息循环里）
fn handle_ipc(
    ctx: &Arc<ipc::Ctx>,
    dialog_queue: &Arc<Mutex<Vec<MainJob>>>,
    proxy: &EventLoopProxy<UserEvent>,
    body: &str,
) {
    let req = match ipc::parse(body) {
        Ok(r) => r,
        Err(e) => {
            // 没有 id 也要尽量回一条错误帧
            let id = serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|v| v.get("id").cloned())
                .unwrap_or(Value::Null);
            eprintln!("[mang-ipc] 请求解析失败：{e}");
            outbox_push(ipc::resolve_script(&id, &ipc::err_frame(&id, &e), false));
            let _ = proxy.send_event(UserEvent::Wake);
            return;
        }
    };

    match req.cmd.as_str() {
        // 窗口控制（自绘标题栏）：直接操作 HWND，不需要回事件循环
        "window.info" | "window.action" | "window.buttons" => {
            let resp = window_cmd(&req);
            let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            outbox_push(ipc::resolve_script(&req.id, &resp, ok));
            let _ = proxy.send_event(UserEvent::Wake);
        }
        // 对话框要在主线程跑（Win32 模态），顺手给事件循环发个唤醒
        "dialog.openFile" | "dialog.openFolder" => {
            {
                let mut q = dialog_queue.lock().unwrap_or_else(|e| e.into_inner());
                q.push(MainJob {
                    id: req.id,
                    cmd: req.cmd,
                    body: Arc::new(req.body),
                });
            }
            let _ = proxy.send_event(UserEvent::Wake);
        }
        _ => {
            let resp = ipc::spill_if_large(&req.id, ctx.dispatch(&req));
            let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            if !ok {
                eprintln!(
                    "[mang-ipc] {} → 失败 {}",
                    req.cmd,
                    resp.get("message").and_then(|m| m.as_str()).unwrap_or("")
                );
            }
            outbox_push(ipc::resolve_script(&req.id, &resp, ok));
            let _ = proxy.send_event(UserEvent::Wake);
        }
    }
}

/// window.info / window.action / window.caption —— 自绘标题栏需要的三个小命令。
///
/// 为什么放在 main.rs 而不是 ipc.rs：它们直接碰 HWND，属于"壳"而不是"数据面"，
/// 和对话框走同一层（主线程持有的窗口句柄）。
fn window_cmd(req: &ipc::Request) -> Value {
    let args = Value::Object(req.body.clone());
    let Some(hwnd) = current_hwnd() else {
        return ipc::err_frame(&req.id, &ipc::IpcError::io("窗口句柄不可用"));
    };
    match req.cmd.as_str() {
        "window.info" => ipc::ok_frame(
            &req.id,
            serde_json::json!({
                "maximized": chrome::is_maximized(hwnd),
                "captionHeight": 32,
                "native": true,
            }),
        ),
        "window.action" => {
            // 直接在当前线程操作 HWND。
            //
            // 历史教训：这里曾经把动作投到事件循环再 recv_timeout 等它完成，
            // 但 handle_ipc 本身就跑在同一个 UI 线程上 —— 等 = 自己堵住自己，
            // 每次点击固定卡 1.5 秒才生效。窗口动作不需要 tao 的状态机，
            // 自己算矩形用 SetWindowPos 就够了（见 chrome::window_action）。
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !matches!(
                action.as_str(),
                "minimize" | "maximize" | "restore" | "toggle" | "close"
                    | "drag" | "toggleMaximize"
            ) {
                return ipc::err_frame(&req.id, &ipc::IpcError::bad_args("未知的窗口动作"));
            }
            let applied = chrome::window_action(hwnd, &action);
            // 状态立即回推，UI 图标随之切换（不读 window.info，避免旧值）
            let now = chrome::is_maximized(hwnd);
            ipc::ok_frame(
                &req.id,
                serde_json::json!({ "action": action, "applied": applied, "maximized": now }),
            )
        }
        // UI 上报「右上角三个窗口按钮占多宽」：标题带右端这一段要留给按钮，
        // 不能返回 HTCAPTION，否则点最大化会变成拖窗口。
        "window.buttons" => {
            let w = args.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
            chrome::set_buttons_width(w.round() as i32);
            ipc::ok_frame(&req.id, serde_json::json!({ "width": w.round() as i32 }))
        }
        _ => ipc::err_frame(&req.id, &ipc::IpcError::bad_args("未知窗口命令")),
    }
}

/// 主窗口 HWND（启动时登记一次；IPC handler 跑在 UI 线程，可安全读取）
static MAIN_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

fn set_main_hwnd(h: isize) {
    MAIN_HWND.store(h, std::sync::atomic::Ordering::Relaxed);
}

fn current_hwnd() -> Option<isize> {
    let h = MAIN_HWND.load(std::sync::atomic::Ordering::Relaxed);
    if h == 0 {
        None
    } else {
        Some(h)
    }
}

/// 事件推送（拖放等）：统一经 outbox，下一轮事件循环注入
fn push_event(ev: Value) {
    outbox_push(ipc::event_script(&ev));
}

/// 把攒下的脚本注入 WebView
fn flush_outbox(webview: &SharedWebview) {
    for script in outbox_drain() {
        match webview.lock() {
            Ok(g) => {
                if let Err(e) = g.evaluate_script(&script) {
                    eprintln!("[mang-ipc] 注入失败：{e}");
                }
            }
            Err(_) => return,
        }
    }
}

fn run_loop(
    event_loop: EventLoop<UserEvent>,
    webview: SharedWebview,
    dialog_queue: Arc<Mutex<Vec<MainJob>>>,
) -> wry::Result<()> {
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::ExitWithCode(0);
            }
            Event::UserEvent(UserEvent::Wake) => {
                // 主线程跑对话框（阻塞用户的模态，跑完继续）
                let jobs: Vec<MainJob> = {
                    let mut q = dialog_queue.lock().unwrap_or_else(|e| e.into_inner());
                    std::mem::take(&mut *q)
                };
                for job in jobs {
                    let result = match job.cmd.as_str() {
                        "dialog.openFile" => dialogs::open_file(&job.body),
                        "dialog.openFolder" => dialogs::open_folder(&job.body),
                        other => {
                            Err(ipc::IpcError::unknown_cmd(format!("未知命令：{other}")))
                        }
                    };
                    let resp = match result {
                        Ok(v) => ipc::ok_frame(&job.id, v),
                        Err(e) => ipc::err_frame(&job.id, &e),
                    };
                    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                    outbox_push(ipc::resolve_script(&job.id, &resp, ok));
                }
            }
            Event::LoopDestroyed => {
                // 退出前把队列里的响应尽量发完
                flush_outbox(&webview);
                ipc::purge_pending();
            }
            _ => {}
        }
        // 拖放事件 → 事件推送 → 与响应一起注入
        for ev in drop_queue_drain() {
            push_event(ev);
        }
        // 每轮事件后把攒下的响应注入进 WebView
        flush_outbox(&webview);
    });
}

fn open_external(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("rundll32.exe");
        cmd.arg("url.dll,FileProtocolHandler").arg(url);
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        cmd.spawn().map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ())
    }
}

// ================================================================= custom protocol

fn serve(root: &Path, req: wry::http::Request<Vec<u8>>) -> wry::http::Response<Cow<'static, [u8]>> {
    let raw_path = req.uri().path().to_string();
    let path = raw_path.trim_start_matches('/');

    // 大响应：/__ipc/<id>.json —— 读一次即删
    if let Some(name) = path.strip_prefix("__ipc/") {
        let name = name.split(['?', '#']).next().unwrap_or(name);
        return match ipc::take_pending(name) {
            Some(bytes) => respond(200, "application/json; charset=utf-8", bytes),
            None => respond(404, "text/plain; charset=utf-8", b"gone".to_vec()),
        };
    }

    let path = if path.is_empty() { "index.html" } else { path };
    // 防目录穿越：解析后必须仍在 UI 根下
    let full = root.join(path);
    let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let Ok(canon) = std::fs::canonicalize(&full) else {
        // 不是文件：回 index.html（前端自己做路由）
        let idx = root.join("index.html");
        return match std::fs::read(&idx) {
            Ok(b) => respond(200, "text/html; charset=utf-8", b),
            Err(_) => respond(404, "text/plain", b"404".to_vec()),
        };
    };
    if !paths::is_inside(&canon, &canon_root) {
        return respond(403, "text/plain", b"403".to_vec());
    }
    match std::fs::read(&canon) {
        Ok(bytes) => {
            let mime = mime_of(&canon);
            respond(200, mime, bytes)
        }
        Err(_) => respond(404, "text/plain", b"404".to_vec()),
    }
}

fn respond(status: u16, mime: &str, bytes: Vec<u8>) -> wry::http::Response<Cow<'static, [u8]>> {
    wry::http::Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-store")
        .body(Cow::Owned(bytes))
        .unwrap_or_else(|_| wry::http::Response::new(Cow::Owned(Vec::new())))
}

fn mime_of(p: &Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") => "application/json; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("pdf") => "application/pdf",
        Some("md") | Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// 生成一个 v4 UUID（不引 crate：splitmix64 展开成 128 位）
pub fn uuid_v4() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() as u64;
    let addr = &COUNTER as *const _ as u64;
    let mut h: u64 = now ^ c.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (pid << 32) ^ addr.rotate_left(17);
    let mut next = || {
        h = h.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = h;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let all: Vec<u8> = [next(), next()]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let hexs: String = all.iter().map(|b| format!("{b:02x}")).collect();
    // 版本位 + 变体位
    let mut buf: Vec<char> = hexs.chars().collect();
    buf[12] = '4';
    let variant = (all[8] & 0x3f) | 0x80;
    let vh = format!("{variant:02x}");
    buf[16] = vh.chars().next().unwrap_or('8');
    buf[17] = vh.chars().nth(1).unwrap_or('0');
    let s: String = buf.into_iter().collect();
    format!(
        "{}-{}-{}-{}-{}",
        &s[0..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> CliOutcome {
        parse_cli(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn cli_forms() {
        match parse(&["--workspace", "D:\\proj", "--devtools"]) {
            CliOutcome::Run(c) => {
                assert_eq!(c.workspace.as_deref(), Some(Path::new("D:\\proj")));
                assert!(c.devtools);
            }
            _ => panic!("应当解析成功"),
        }
        match parse(&["D:\\proj"]) {
            CliOutcome::Run(c) => assert_eq!(c.workspace.as_deref(), Some(Path::new("D:\\proj"))),
            _ => panic!("位置参数应当被当作工作区"),
        }
        // --workspace 优先于位置参数
        match parse(&["D:\\a", "--workspace", "D:\\b"]) {
            CliOutcome::Run(c) => assert_eq!(c.workspace.as_deref(), Some(Path::new("D:\\b"))),
            _ => panic!("--workspace 应当优先"),
        }
        match parse(&["--nope"]) {
            CliOutcome::ArgError(_) => {}
            _ => panic!("未知选项应报错"),
        }
        match parse(&["-h"]) {
            CliOutcome::Help => {}
            _ => panic!("-h 应打帮助"),
        }
        match parse(&["--ui"]) {
            CliOutcome::ArgError(_) => {}
            _ => panic!("--ui 缺参数应报错"),
        }
        match parse(&["--selftest", "--ui", "x"]) {
            CliOutcome::Run(c) => {
                assert!(c.selftest);
                assert_eq!(c.ui.as_deref(), Some(Path::new("x")));
            }
            _ => panic!(),
        }
        match parse(&["--inject-drop", "a.md;b.pdf"]) {
            CliOutcome::Run(c) => {
                let d = c.inject_drop.unwrap();
                assert_eq!(d.len(), 2);
                assert_eq!(d[1], Path::new("b.pdf"));
            }
            _ => panic!("--inject-drop 应解析成两个路径"),
        }
    }

    #[test]
    fn mime_guess() {
        assert!(mime_of(Path::new("a.html")).starts_with("text/html"));
        assert_eq!(mime_of(Path::new("x.wasm")), "application/wasm");
        assert_eq!(mime_of(Path::new("n.OUT")), "application/octet-stream");
    }

    #[test]
    fn uuid_shape() {
        let a = uuid_v4();
        let b = uuid_v4();
        assert_eq!(a.len(), 36);
        assert_ne!(a, b);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[2].len(), 4);
        assert!(parts[2].starts_with('4'));
        assert!(['8', '9', 'a', 'b'].contains(&parts[3].chars().next().unwrap()));
    }
}