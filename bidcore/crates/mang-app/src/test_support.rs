//! 跨模块共享的测试基建（#[cfg(test)] 专用，不进产物）。
//!
//! #383：`%APPDATA%\mang\last-workspace.json` 与 `recent.json` 是进程级
//! 全局单文件——cargo test 默认多线程并行，任何「碰这两个文件」的测试
//! 之间都可能互踩：A 的落盘被 B 的直写覆盖，或 A 断言前文件已被 B 改写。
//! #303/FG-01 只在 ipc::tests 内部加了一把 `LAST_WS_TEST_LOCK`，没发现
//! recent::tests 也直写 last-workspace——两把锁互不相识，CI 并行即红
//! （`workspace_open_persists_last_workspace`，串行全绿）。
//!
//! 规则：**所有会读写 last-workspace.json / recent.json 的测试，必须在
//! 测试体开头取 [`workspace_files_guard`]（全 crate 唯一的测试级锁）**。
//! 生产代码的 `recent::RECENT_LOCK` 只串行化单次调用，保不住「open →
//! 断言」这类跨调用序列不被别的测试插队——测试级互斥必须由测试自己
//! 持有，且只能有这一把：再冒出第二把（哪怕藏在另一个模块里）就会
//! 重演 #383。
//!
//! 不用「每测试独立临时目录」：路径由全局函数从 APPDATA 派生，不可
//! 注入（见 #383 方案 B 的成本评估）。
//!
//! 不用 `--test-threads=1`：让全 crate 测试串行换绿 = 用串行掩盖
//! 并行缺陷（#383 验收第 2 条明令禁止）。

use std::sync::{Mutex, MutexGuard};

/// 所有会碰 last-workspace.json / recent.json 的测试共用这一把锁。
///
/// 放在 crate 根的 test-only 模块里（而不是某个业务模块内），是要让
/// 「新增触碰者」在 import 时就看到它属于测试基建而非任何业务面；
/// 离得远，反而谁都不会误以为自己的模块私有锁就够用。
pub static WORKSPACE_FILE_LOCK: Mutex<()> = Mutex::new(());

/// 取锁入口。毒化不复位——与 `ipc::Ctx::ws` / `recent::RECENT_LOCK`
/// 同口径：持锁者 panic 后锁已不可救，下一个取锁者直接复用继续跑
/// （测试进程里一次 panic 就整体退出，无所谓「脏状态」）。
pub fn workspace_files_guard() -> MutexGuard<'static, ()> {
    WORKSPACE_FILE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}
