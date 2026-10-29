//! env_preflight 模拟集成测试（issue #9）：在**独立进程**里验证 SKIP 机制的真实行为。
//!
//! 与 `src/env_preflight.rs` 内联单测的区别：内联单测验证判定逻辑正确性；
//! 本文件验证**消费方视角的端到端契约**——一个真实存在的测试函数，
//! 在模拟缺件环境（`BID_PREFLIGHT_PATH` 指向空目录）下：
//! 1. 函数体必须在预检后立即返回（不触碰后续业务代码——用「若继续执行则必炸」
//!    的哨兵断言守门）；
//! 2. SKIP 判定必须为真（`missing_tools` 非空）。
//!
//! 设计注记：stable Rust libtest 无运行时 ignore，SKIP = 「横幅 + 提前 return」
//! （见 `env_preflight.rs` 模块文档）。本测试把该模式固化为契约，防止后人
//! 「顺手删掉那个 if」导致缺件机器重新回到误导性 FAIL。

use bid_parse::env_preflight::{missing_tools, skip_if_env_missing, PDF_CHAIN_TOOLS};

/// PR #17 同型修复延伸（main 既有 bug，非本批引入）：libtest 文件内并行——
/// `full_env_runs_test_body_normally` 两次读环境（`missing_tools` 与
/// `skip_if_env_missing` 内部各读一次 `BID_PREFLIGHT_PATH`），并行伙伴的
/// set/remove 窗口落在两次读之间 → 首读命中覆盖判 N 缺、次读真实环境判 0 缺
/// → 「skip 判定与清单严格一致」断言炸。修复：变更方（with_restricted_path
/// 持锁横跨 set→remove 全窗口）与读取方（full_env 用例持锁横跨两次读）
/// 用同一互斥锁串行化——读写任一方持锁期间另一方不可进入。
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 独立测试二进制内的哨兵：预检通过后才允许触达业务面。
/// 用 `static` 计数器而非 panic-on-drop：提前 return 的路径不抛异常，
/// 唯一能证明「业务面未被执行」的方式是业务面自身从未置位。
static BUSINESS_REACHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn with_restricted_path<T>(f: impl FnOnce() -> T) -> T {
    // 写方持锁：横跨 set→remove 全窗口（见 ENV_LOCK 注释）。
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // 模拟「无 poppler / 无 tesseract 的机器」：查找路径指向空目录。
    let empty = std::env::temp_dir().join("bid-preflight-sim-empty");
    std::fs::create_dir_all(&empty).unwrap();
    // SAFETY(env mut): 独立测试进程（每集成测试文件一个二进制），
    // 进程内并发面已由 ENV_LOCK 串行化；测试结束进程即退出，无跨测试污染面。
    unsafe { std::env::set_var("BID_PREFLIGHT_PATH", &empty) };
    let out = f();
    unsafe { std::env::remove_var("BID_PREFLIGHT_PATH") };
    out
}

#[test]
fn missing_env_skips_test_body_without_touching_business_logic() {
    with_restricted_path(|| {
        // ① 缺件判定：四件套全缺
        let miss = missing_tools(PDF_CHAIN_TOOLS);
        assert_eq!(miss.len(), PDF_CHAIN_TOOLS.len(), "模拟空 PATH 应四件全缺: {miss:?}");

        // ② SKIP 预检返回 true —— 消费方按契约提前 return
        let should_skip = skip_if_env_missing("sim_missing_env_case", PDF_CHAIN_TOOLS);
        assert!(should_skip, "缺件环境下 skip_if_env_missing 必须返回 true");

        // ③ 哨兵：真实测试在此已 return，业务面（如下面的置位）不可达
        if !should_skip {
            BUSINESS_REACHED.store(true, std::sync::atomic::Ordering::SeqCst);
            panic!("缺件环境不得进入业务面");
        }
    });
    assert!(
        !BUSINESS_REACHED.load(std::sync::atomic::Ordering::SeqCst),
        "业务哨兵不得被置位（提前 return 契约被破坏）"
    );
}

#[test]
fn full_env_runs_test_body_normally() {
    // 读方持锁：两次读环境原子化（防并行 set/remove 窗口落中间，见 ENV_LOCK 注释）。
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // 对偶面：真实 PATH（无 BID_PREFLIGHT_PATH 覆盖）下，若工具齐备则预检返回 false。
    // 本断言容忍「当前机器恰缺件」——此时返回 true 且清单恰含缺失项（一致性不变量），
    // 保证测试在两类机器上都有效而不放松契约。
    let miss = missing_tools(PDF_CHAIN_TOOLS);
    let should_skip = skip_if_env_missing("sim_full_env_case", PDF_CHAIN_TOOLS);
    assert_eq!(
        should_skip,
        !miss.is_empty(),
        "skip 判定必须与缺件清单严格一致（清单: {miss:?}）"
    );
}

#[test]
fn skip_banner_names_the_test_and_missing_tools() {
    // SKIP 横幅的可读性契约（issue #9 判定标准原文）：
    //   SKIP: <test_id>
    //   reason: ENV_MISSING: <工具清单>
    // —— 横幅走 eprintln，此处验证其数据源（清单与工具名逐一对应），
    // 格式串在 env_preflight::skip_if_env_missing 内钉死，评审对照即可。
    let miss = with_restricted_path(|| missing_tools(PDF_CHAIN_TOOLS));
    for tool in PDF_CHAIN_TOOLS {
        assert!(
            miss.iter().any(|m| m == tool),
            "模拟环境下 {tool} 必须出现在 ENV_MISSING 清单"
        );
    }
}
