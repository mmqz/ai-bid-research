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

static GLOBAL: LazyLock<Mutex<BidService>> = LazyLock::new(|| Mutex::new(open_global_service()));

/// 全局服务态的构造（stdio 默认传输面）。
///
/// `BID_WS` 环境变量在场 → 按该目录打开【持久化】工作区（db.json 恢复/
/// checkpoint 落盘——长进程 stdio 宿主如 Claude Desktop / e2e 驱动需要跨调用
/// 持久）；未设 → 纯内存态（M0 骨架默认，行为不变）。打开失败要响亮 panic
/// （配置错误不能降级成静默内存态——那会让宿主误以为在持久区工作）。
///
/// `BID_FS_ROOT`（#183/#189 可选）：文件访问围栏根覆盖（默认 = BID_WS 工作区
/// ——导入导出都锁在工作区内）。显式声明更广的受信导入根（如共享夹具
/// 目录）时设此变量；工作区（若在）始终隐式受信（服务自有存储）。未设
/// BID_WS 时也可单设 BID_FS_ROOT（纯内存态 + 围栏导入根——便捷接线面，
/// 无持久化）。
fn open_global_service() -> BidService {
    let fs_root = std::env::var("BID_FS_ROOT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match std::env::var("BID_WS") {
        Ok(dir) if !dir.trim().is_empty() => {
            let mut svc = BidService::open(dir.trim()).unwrap_or_else(|e| {
                panic!("bid-mcp 全局服务态：BID_WS={dir} 打开失败（工作区损坏或不可写）: {e}")
            });
            if let Some(root) = fs_root {
                svc = svc.with_fs_root(root);
            }
            svc
        }
        _ => match fs_root {
            Some(root) => BidService::new().with_fs_root(root),
            None => BidService::new(),
        },
    }
}

/// 在全局服务态上执行（stdio 默认传输面）。
///
/// 锁中毒自恢复：一次 panic 不终止传输面（服务态可能半更新——内存态的风险
/// 面与进程崩溃等价，由 content_version 乐观并发兜底）。
pub(crate) fn with_global<T>(f: impl FnOnce(&mut BidService) -> T) -> T {
    let mut guard = GLOBAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

static REQUEST_SEQ: AtomicU64 = AtomicU64::new(0);

/// 请求标识（契约包装面的 `request_id`）——进程内单调，重启归零。
/// 跨进程唯一性由调用方前缀承担，不在此伪装持久。
pub(crate) fn next_request_id() -> String {
    let n = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    format!("req-{n:06}")
}
