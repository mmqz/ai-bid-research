//! 进程级服务态（M0-d 领域持久化的最小落地，B-1 起写面工具需要跨调用状态）。
//!
//! M0 骨架刻意无持久化——`get_bid_project_state` 由调用方显式带入状态，
//! 「不伪造已存储的项目」。B-1 的 `import_tender_files` 需要幂等键
//! （sha256）、状态机（DRAFT → FILES_IMPORTED）与 content_version 在
//! stdio 长进程的行与行之间保持——本模块给出**进程内**最小承载。
//! 跨进程持久化（重启续跑）仍属后续里程碑，不在此伪装。
//!
//! 测试与嵌入方不经全局：`bid_mcp::dispatch_with(&mut BidService, ..)`
//! 注入独立实例（并行隔离）——全局态只服务默认 stdio 传输。

use bid_tools::service::BidService;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

static GLOBAL: LazyLock<Mutex<BidService>> = LazyLock::new(|| Mutex::new(BidService::new()));

/// 在全局服务态上执行（stdio 默认传输面）。
///
/// 锁中毒自恢复：一次 panic 不终止传输面（服务态可能半更新——内存态的风险
/// 面与进程崩溃等价，由 content_version 乐观并发兜底）。
pub(crate) fn with_global<T>(f: impl FnOnce(&mut BidService) -> T) -> T {
    let mut guard = GLOBAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

static REQUEST_SEQ: AtomicU64 = AtomicU64::new(0);

/// 请求标识（契约包装面的 `request_id`）——进程内单调，重启归零。
/// 跨进程唯一性由调用方前缀承担，不在此伪装持久。
pub(crate) fn next_request_id() -> String {
    let n = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    format!("req-{n:06}")
}
