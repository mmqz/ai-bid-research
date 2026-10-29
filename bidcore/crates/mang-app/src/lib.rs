//! mang-app — mang 标书工作台桌面壳（bid #42 完整规格）。
//!
//! # 结构（纯逻辑 / GUI 胶水分离）
//!
//! ```text
//! ┌─ 纯逻辑层（默认编译，全环境可测）───────────────────────┐
//! │ workspace.rs  --workspace 解析 + 目录扫描 → 文件树 JSON  │
//! │                + 最近打开记录（.mang/recent.json）      │
//! │ versions.rs   版本快照（.mang/history/）+ index.json    │
//! │                + 自动保存判定（30s 节流 + 内容变化门）  │
//! │ ipc.rs        mangipc:// 导航通道协议编解码             │
//! │ protocol.rs   mangapp://（UI bundle）/ mangfs://（工作区 │
//! │                文件）静态服务 + 穿越防护 + mime        │
//! └──────────────────────────────────────────────────────┘
//!          ↓ 被 src/main.rs（gui feature）消费
//! ┌─ GUI 胶水（--features gui，Windows/WebView2 主目标）────┐
//! │ tao 事件循环（窗口 ≥1024×640）+ wry WebView             │
//! │ mangapp:// serve web-ui/dist；mangfs:// serve 工作区     │
//! │ 启动注入 bootstrap（工作区树+配置）；mangipc:// 命令分发 │
//! └──────────────────────────────────────────────────────┘
//! ```
//!
//! # wry 0.46 已知坑（mdcore mdshell-native s5a-env 实测，此处全部规避）
//!
//! 1. `with_url` 无返回值（不落盘不失败）；
//! 2. `evaluate_script` 无返回值——JS→Rust 回传走导航拦截通道
//!    （`location.assign("mangipc://…")` + navigation_handler 截获取消）；
//! 3. webkitgtk 的 `with_ipc_handler` 回调里 `webview.uri()` 对 file://
//!    空 authority 形 panic——不使用 IPC handler，统一走导航通道；
//! 4. WebKit file:// 下 wasm fetch/ESM 被安全策略拦——本地 bundle 必须
//!    走自定义协议（mangapp://），不用 file://。

pub mod ipc;
pub mod protocol;
pub mod versions;
pub mod workspace;

/// 桌面壳常量（#42 规格硬性要求）。
pub mod spec {
    /// 窗口最小宽度（px）。
    pub const MIN_WIDTH: u32 = 1024;
    /// 窗口最小高度（px）。
    pub const MIN_HEIGHT: u32 = 640;
    /// 工作区元数据目录名（历史/最近打开都放这里）。
    pub const META_DIR: &str = ".mang";
    /// 版本历史子目录。
    pub const HISTORY_DIR: &str = "history";
    /// 最近打开记录文件。
    pub const RECENT_FILE: &str = "recent.json";
    /// 自动快照检查间隔（秒）——#42 §八「每 30 秒检查一次」。
    pub const AUTOSNAP_INTERVAL_SECS: u64 = 30;
    /// 最近打开列表上限（循环覆盖淘汰最旧）。
    pub const RECENT_MAX: usize = 20;
}
