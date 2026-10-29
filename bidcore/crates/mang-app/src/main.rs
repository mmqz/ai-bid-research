//! mang-app GUI 入口（`--features gui` 才编译）。
//!
//! tao + wry 胶水：窗口（≥1024×640）+ mangapp://（UI bundle）+ mangfs://
//! （工作区只读）+ mangipc:// 命令分发 + 30s 自动快照节拍。
//!
//! 逻辑全部下沉 lib 纯层（workspace/versions/ipc/protocol）——本文件只做
//! 接线，Windows CI（ci/windows-check）验证编译与冒烟。
//!
//! 环解耦：navigation_handler 闭包不能持有 webview（与 build 互相引用环），
//! 命令经 mpsc channel 入队，事件循环侧 drain + dispatch + evaluate_script
//! 回注响应。

#![cfg(feature = "gui")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mang_app::ipc::{self, SCHEME};
use mang_app::protocol::{self, RawResponse};
use mang_app::spec;
use mang_app::versions::{self, VersionKind};
use mang_app::workspace::{self, FileType, RecentStore, ScanLimits};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ws_root = match workspace::resolve_workspace(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("mang: {e}");
            std::process::exit(2);
        }
    };

    // UI bundle 定位：--dist > MANG_APP_DIST > cwd/web-ui/dist > exe 同级。
    let dist_root = args
        .iter()
        .position(|a| a == "--dist")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| std::env::var("MANG_APP_DIST").ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            let cwd = std::env::current_dir().unwrap_or_default();
            let cand = cwd.join("web-ui").join("dist");
            if cand.exists() {
                return cand;
            }
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|d| d.join("web-ui").join("dist")))
                .unwrap_or(cand)
        });

    let scan = workspace::scan_workspace(&ws_root, &ScanLimits::default());
    println!(
        "mang: workspace={} ({} files / {} dirs, truncated={})",
        ws_root.display(),
        scan.file_count,
        scan.dir_count,
        scan.truncated
    );

    // ---- 共享状态 ----
    let ws = Arc::new(ws_root);
    let recent = Arc::new(Mutex::new(RecentStore::load(&ws_root)));
    // 自动快照面：path → (上次快照内容, 上次快照时间)
    let last_snap = Arc::new(Mutex::new(
        std::collections::HashMap::<String, (String, u64)>::new(),
    ));
    // 命令通道（navigation_handler → 事件循环）
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<String>();
    let cmd_rx = Arc::new(Mutex::new(cmd_rx));

    let event_loop = tao::event_loop::EventLoop::new();
    let window = tao::window::WindowBuilder::new()
        .with_title("mang — 标书工作台")
        .with_inner_size(tao::dpi::LogicalSize::new(1440.0, 900.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(
            spec::MIN_WIDTH as f64,
            spec::MIN_HEIGHT as f64,
        ))
        .build(&event_loop)
        .expect("窗口创建失败");

    // 启动注入：页面脚本运行前可见（wry initialization script 语义）。
    let bootstrap = serde_json::json!({
        "workspace": scan,
        "recent": *recent.lock().unwrap(),
        "autosnapInterval": spec::AUTOSNAP_INTERVAL_SECS,
    });
    let init_js = format!(
        "window.__mangBootstrap = {{ data: {}, onMessage: null }};",
        serde_json::to_string(&bootstrap).unwrap()
    );

    let ws_fs = Arc::clone(&ws);
    let tx = cmd_tx.clone();

    let builder = wry::WebViewBuilder::new()
        .with_custom_protocol("mangapp".to_string(), {
            let dist = dist_root.clone();
            move |_uri, request| {
                let r = protocol::serve_app(&dist, request.uri().path());
                raw_to_wry(r)
            }
        })
        .with_custom_protocol("mangfs".to_string(), move |_uri, request| {
            let r = protocol::serve_fs(&ws_fs, request.uri().path());
            raw_to_wry(r)
        })
        .with_initialization_script(&init_js)
        .with_url("mangapp://localhost/")
        // mangipc:// 导航拦截通道（wry 0.46 IPC handler 的 webkitgtk file://
        // panic 规避——mdcore mdshell-native s5a-env 实证方案）：只入队 +
        // 取消导航，处理在事件循环侧（持 webview 引用）。
        .with_navigation_handler(move |url: String| {
            if url.starts_with(&format!("{SCHEME}://")) {
                let _ = tx.send(url);
                return false;
            }
            true
        });

    let webview = builder.build(&window).expect("WebView 创建失败");

    // ---- 事件循环 ----
    let next_tick = Arc::new(Mutex::new(
        Instant::now() + Duration::from_secs(spec::AUTOSNAP_INTERVAL_SECS),
    ));
    let running = Arc::new(AtomicBool::new(true));

    event_loop.run(move |event, _, control_flow| {
        use tao::event::{Event, StartCause, WindowEvent};
        use tao::event_loop::ControlFlow;

        match event {
            Event::NewEvents(StartCause::Init) => {
                println!("mang: shell init; dist={}", dist_root.display());
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                running.store(false, Ordering::SeqCst);
                *control_flow = ControlFlow::Exit;
            }
            Event::MainEventsCleared => {
                // 1) drain 命令队列 → dispatch → 回注
                let drained: Vec<String> = {
                    let Ok(rx) = cmd_rx.lock() else { Vec::new() };
                    let mut v = Vec::new();
                    while let Ok(url) = rx.try_recv() {
                        v.push(url);
                    }
                    v
                };
                for url in drained {
                    let resp = dispatch(&url, &ws, &recent, &last_snap);
                    let js = format!(
                        "window.__mang && __mang.onMessage && __mang.onMessage({});",
                        serde_json::to_string(&resp).unwrap()
                    );
                    let _ = webview.evaluate_script(&js);
                }
                // 2) 30s 自动快照节拍：通知页面送回脏文件内容（save 通道）
                let fire = {
                    let mut nt = next_tick.lock().unwrap();
                    if Instant::now() >= *nt {
                        *nt = Instant::now() + Duration::from_secs(spec::AUTOSNAP_INTERVAL_SECS);
                        true
                    } else {
                        false
                    }
                };
                if fire {
                    let _ = webview.evaluate_script(
                        "window.__mang && __mang.onAutosnapTick && __mang.onAutosnapTick()",
                    );
                }
            }
            Event::LoopDestroyed => {
                // 退出前持久化最近打开（异常退出也尽量保住）
                let _ = recent.lock().unwrap().save(&ws);
            }
            _ => {}
        }
        if running.load(Ordering::SeqCst) {
            *control_flow = ControlFlow::WaitUntil(*next_tick.lock().unwrap());
        }
    });
}

/// 命令分发（纯逻辑消费 + 响应 JSON 构造）。
///
/// 响应形：`{ "verb": …, "ok": bool, …载荷 }`——web 端按 verb 路由回调。
fn dispatch(
    url: &str,
    ws: &std::path::Path,
    recent: &Mutex<RecentStore>,
    last_snap: &Mutex<std::collections::HashMap<String, (String, u64)>>,
) -> serde_json::Value {
    use serde_json::json;

    let (verb, payload) = match ipc::decode(url) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": e }),
    };

    // 工作区内路径校验（穿越/内部目录——与 protocol::serve_fs 同口径）
    let safe_path = |p: &str| -> Result<PathBuf, String> {
        if p.contains("..") || p.starts_with('/') {
            return Err("非法路径".to_string());
        }
        let first = p.split('/').next().unwrap_or("");
        if [".mang", ".git"].contains(&first) {
            return Err("内部目录不可访问".to_string());
        }
        Ok(ws.join(p))
    };

    match verb.as_str() {
        "tree" => {
            let scan = workspace::scan_workspace(ws, &ScanLimits::default());
            json!({ "verb": "tree", "ok": true, "scan": scan })
        }
        "open" => {
            let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match safe_path(path) {
                Ok(full) => match std::fs::read(&full) {
                    Ok(bytes) => {
                        // 文本类直读；二进制类（docx/xlsx/pdf）回 URL——web 端
                        // 经 mangfs:// fetch + wasm 导入管线处理
                        let name = full
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let ftype = FileType::of(&name);
                        let is_text = matches!(ftype, FileType::Doc)
                            && !matches!(
                                full.extension().and_then(|e| e.to_str()),
                                Some("docx" | "doc")
                            );
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let text = if is_text {
                            Some(String::from_utf8_lossy(&bytes).into_owned())
                        } else {
                            None
                        };
                        recent.lock().unwrap().record_open(path, &name, now);
                        json!({
                            "verb": "open", "ok": true, "path": path, "name": name,
                            "ftype": ftype, "text": text,
                            "url": format!("mangfs://localhost/{}", ipc::percent_encode(path)),
                        })
                    }
                    Err(e) => json!({ "verb": "open", "ok": false, "error": e.to_string() }),
                },
                Err(e) => json!({ "verb": "open", "ok": false, "error": e }),
            }
        }
        "save" => {
            let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let content = payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match safe_path(path) {
                Ok(full) => {
                    // 原子写（tmp + rename）
                    let tmp = full.with_extension("md.tmp");
                    let r =
                        std::fs::write(&tmp, content).and_then(|_| std::fs::rename(&tmp, &full));
                    match r {
                        Ok(()) => {
                            // 自动快照判定（30s 节流 + 内容门——versions.rs 纯逻辑）
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let mut snaps = last_snap.lock().unwrap();
                            let (prev, prev_ts) = snaps
                                .get(path)
                                .map(|(c, t)| (Some(c.clone()), Some(*t)))
                                .unwrap_or((None, None));
                            if versions::should_autosnap(content, prev.as_deref(), prev_ts, now) {
                                let _ = versions::save_snapshot(
                                    ws,
                                    path,
                                    content,
                                    VersionKind::Auto,
                                    None,
                                    now,
                                    0,
                                );
                                snaps.insert(path.to_string(), (content.to_string(), now));
                            }
                            json!({ "verb": "save", "ok": true })
                        }
                        Err(e) => json!({ "verb": "save", "ok": false, "error": e.to_string() }),
                    }
                }
                Err(e) => json!({ "verb": "save", "ok": false, "error": e }),
            }
        }
        "snapshot" => {
            let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let message = payload.get("message").and_then(|v| v.as_str());
            let content = payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if content.is_empty() {
                return json!({ "verb": "snapshot", "ok": false, "error": "空内容不入版本" });
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            match versions::save_snapshot(ws, path, content, VersionKind::Ai, message, now, 0) {
                Ok(snap) => {
                    last_snap
                        .lock()
                        .unwrap()
                        .insert(path.to_string(), (content.to_string(), now));
                    json!({ "verb": "snapshot", "ok": true, "snapshot": snap })
                }
                Err(e) => json!({ "verb": "snapshot", "ok": false, "error": e }),
            }
        }
        "versions" => {
            let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let idx = versions::VersionIndex::load(ws);
            let v: Vec<&versions::VersionEntry> = idx.versions_of(path);
            json!({ "verb": "versions", "ok": true, "versions": v })
        }
        "read-version" => {
            let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let snap = payload
                .get("snapshot")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match versions::read_snapshot(ws, path, snap) {
                Ok(c) => json!({ "verb": "read-version", "ok": true, "content": c }),
                Err(e) => json!({ "verb": "read-version", "ok": false, "error": e }),
            }
        }
        "rollback" => {
            let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let snap = payload
                .get("snapshot")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match (versions::read_snapshot(ws, path, snap), safe_path(path)) {
                (Ok(content), Ok(full)) => {
                    // 回滚前先存当前内容为 auto 快照（回滚可再回滚）
                    if let Ok(cur) = std::fs::read_to_string(&full) {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let _ = versions::save_snapshot(
                            ws,
                            path,
                            &cur,
                            VersionKind::Auto,
                            Some("回滚前自动保存"),
                            now,
                            0,
                        );
                    }
                    let tmp = full.with_extension("md.tmp");
                    let r =
                        std::fs::write(&tmp, &content).and_then(|_| std::fs::rename(&tmp, &full));
                    match r {
                        Ok(()) => json!({
                            "verb": "rollback", "ok": true,
                            "content": content, "snapshot": snap,
                        }),
                        Err(e) => {
                            json!({ "verb": "rollback", "ok": false, "error": e.to_string() })
                        }
                    }
                }
                (Err(e), _) => json!({ "verb": "rollback", "ok": false, "error": e }),
                (_, Err(e)) => json!({ "verb": "rollback", "ok": false, "error": e }),
            }
        }
        "recent" => {
            let r = recent.lock().unwrap();
            json!({ "verb": "recent", "ok": true, "entries": r.entries })
        }
        _ => json!({ "ok": false, "error": format!("未实现命令: {verb}") }),
    }
}

/// 平台中立响应 → wry Response 适配。
fn raw_to_wry(r: RawResponse) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let mut b = wry::http::Response::builder().status(r.status);
    if let Some(h) = b.headers_mut() {
        h.insert("Content-Type", r.mime.parse().unwrap());
    }
    b.body(std::borrow::Cow::Owned(r.body)).unwrap()
}
