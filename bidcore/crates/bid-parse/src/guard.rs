//! guard.rs —— 输入资源防御层（S1 威胁模型 T-03/T-04 的实装挂点）。
//!
//! B4 收敛（issue #11 任务 3）：docx/xlsx 自研解析删除后，**zip 面**
//! （prescan 声明值门 / 条目名检查 / 实际读取门 / EOCD 守恒门 / XML 深度门 /
//! xlsx 后置限额）无消费方——OOXML 容器安全全链收敛到引擎侧
//! （`bid-engine` reader.rs：docx 容器三门 + .rels → `EngineRejected`；
//! xlsx 门 0/1/2/3 + 提取期限额 → `io::read_book_from_bytes` 全链引擎执行）。
//! 引擎门等价性结论见 issue #11 任务 3 / B4 PR 描述。
//!
//! 保留面（pdf.rs 与 env_preflight.rs 消费）：
//! - **pdf 输入字节门**（`check_input_size` + `LIMIT_INPUT_BYTES_PDF`——读任何
//!   内容之前先查 `metadata`，防「先全量读入内存」型 OOM）；
//! - **流式 sha256**（`sha256_streaming`——lib.rs 文件指纹入口，替换 `fs::read`
//!   全量内存哈希）；
//! - **子进程治理全套**（T-04：`run_limited` / `RunOutcome` / 超时杀进程组 /
//!   stdout 上限 / stderr drain / 并发席 / `TmpGuard`——pdftotext、pdftoppm、
//!   tesseract 与环境预检的外部命令全部受治）。
//!
//! 超限行为契约（T-03）：**显式失败、不留部分结果**——调用方在 guard 拒绝时
//! 不得产出任何可被误认为完整解析的产物（Rust `Err` 天然承载，
//! runner 侧落 `ParseStatus::Failed` + E_UNPROCESSABLE 禁引纪律，R-01）。

use std::fs::File;

/// 子进程类测试的共享串行锁（见 tests 模块注记）。
#[cfg(test)]
pub(crate) static SUBPROC_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use std::io::Read;
use std::path::Path;

// ---------------------------------------------------------------- 限额常量
/// PDF 族输入字节上限（R3-S3-2：docx/xlsx 有门而 PDF 曾无门——任意大小直进 poppler；
/// 500MB 纳入真实大标书 PDF（数百 MB 量级），超限拒绝在读取前）。
/// （docx/xlsx 输入门已随 B4 收敛到引擎侧——office-io LIMIT_INPUT_BYTES 家族。）
pub const LIMIT_INPUT_BYTES_PDF: u64 = 500 * 1024 * 1024;

// ---------------------------------------------------------------- 错误面
/// guard 拒绝原因（显式失败——上层转 E_UNPROCESSABLE 语义，绝不静默降级）。
/// B4 收敛：仅剩输入字节门面（zip 面变体随自研解析删除——引擎侧接管）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardError {
    InputTooLarge { len: u64, limit: u64 },
    Io(String),
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputTooLarge { len, limit } => {
                write!(f, "输入字节超限 {len} > {limit}")
            }
            Self::Io(s) => write!(f, "IO 错误 {s}"),
        }
    }
}

impl std::error::Error for GuardError {}

type GResult<T> = Result<T, GuardError>;

// ---------------------------------------------------------------- 输入字节门
/// 输入文件字节门（读任何内容之前先查 `metadata`，
/// 防"先全量读入内存做哈希/探测"型 OOM）。
pub fn check_input_size(path: &Path, limit: u64) -> GResult<()> {
    let len = std::fs::metadata(path).map_err(|e| GuardError::Io(e.to_string()))?.len();
    if len > limit {
        return Err(GuardError::InputTooLarge { len, limit });
    }
    Ok(())
}

/// 流式 sha256（替换 `fs::read` 全量内存哈希——大文件面顺带根治）。
pub fn sha256_streaming(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

// ---------------------------------------------------------------- T-04 子进程治理
/// 外部命令子进程治理（S1 威胁模型 T-04 实装）：
/// - 超时 → 杀**进程组**（`process_group(0)` + `kill(-pgid, SIGKILL)`——
///   pdftoppm/tesseract 可能派生孙进程，只杀直接子进程留孤儿）；
/// - 输出上限 → stdout/stderr 各自累计计量，stdout 超限即杀即失败
///   （截断输出继续解析 = 产出损坏结果，违反「不留部分结果」）；
/// - 结果五分类：成功 / 超时 / 输出超限 / 非零退出 / 繁忙（并发满）；
///   取消、超时、崩溃**永不**返回 Success——上层据此不落 `Ok`/`EXPORTED`；
/// - 并发上限：全局原子计数，满载显式拒绝（调用方可重试），不静默排队。
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// 同时在跑的解析子进程上限（OCR 管道单文件即占 1 席；防并发洪泛）。
pub const MAX_CONCURRENT_SUBPROCS: usize = 4;
static LIVE_SUBPROCS: AtomicUsize = AtomicUsize::new(0);

/// 各外部命令的治理参数（部署清单钉住；调整需过组件版本矩阵）。
pub const TIMEOUT_PDFTOTEXT: Duration = Duration::from_secs(60);
pub const TIMEOUT_PDFTOPPM: Duration = Duration::from_secs(180);
pub const TIMEOUT_TESSERACT: Duration = Duration::from_secs(30);
pub const LIMIT_PDFTOTEXT_STDOUT: u64 = 64 * 1024 * 1024;
pub const LIMIT_TESSERACT_STDOUT: u64 = 1024 * 1024;
pub const LIMIT_SUBPROC_STDERR: u64 = 64 * 1024;

/// OCR 渲染页数上限：扫描件超页 → 整体显式拒绝（不产部分 OCR 结果）。
pub const MAX_OCR_PAGES: usize = 200;

/// 治理结果五分类（+ Spawn 失败）。Timeout/Overflow/NonZero/Busy 均为失败面，
/// 调用方**必须**向上传播——不得映射为空输出成功。
#[derive(Debug)]
pub enum RunOutcome {
    Success { stdout: String },
    Timeout { program: String, secs: u64 },
    OutputOverflow { program: String, limit: u64 },
    NonZeroExit { program: String, code: Option<i32>, stderr_head: String },
    Busy { program: String },
    Spawn { program: String, why: String },
}

impl RunOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }
    /// 统一转错误字符串（pdf.rs 语义保留 program 上下文）。
    pub fn into_result(self) -> Result<String, String> {
        match self {
            Self::Success { stdout } => Ok(stdout),
            Self::Timeout { program, secs } => Err(format!("子进程超时被杀（{program} {secs}s）")),
            Self::OutputOverflow { program, limit } => {
                Err(format!("子进程输出超限（{program} > {limit} 字节）"))
            }
            Self::NonZeroExit { program, code, stderr_head } => {
                Err(format!("{program} 退出码 {code:?}: {stderr_head}"))
            }
            Self::Busy { program } => Err(format!("解析子进程并发满载（{program}），请重试")),
            Self::Spawn { program, why } => Err(format!("{program} 无法启动: {why}")),
        }
    }
}

/// 并发席 RAII：离开作用域必减计（即使 Err 提前返回）。
struct SubprocSlot;
impl SubprocSlot {
    fn acquire(program: &str) -> Result<Self, RunOutcome> {
        let n = LIVE_SUBPROCS.fetch_add(1, Ordering::SeqCst);
        if n >= MAX_CONCURRENT_SUBPROCS {
            LIVE_SUBPROCS.fetch_sub(1, Ordering::SeqCst);
            return Err(RunOutcome::Busy { program: program.to_string() });
        }
        Ok(Self)
    }
}
impl Drop for SubprocSlot {
    fn drop(&mut self) {
        LIVE_SUBPROCS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 杀整个进程组（Unix；`pgid == child_pid` 因 spawn 时 `process_group(0)`）。
fn kill_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        // negative pid = 进程组；errno 检查省略（组可能已空——kill 返回 ESRCH 属正常竞态）
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        // P2-6①：非 unix 降级为只杀直接子进程（Windows 下孙进程漏杀）；
        // 本仓部署面为 Linux，降级路径仅保编译——Windows 部署需 taskkill /T 等价物
        let _ = child.kill();
    }
}

/// 受治运行一个外部命令：超时杀组、stdout 上限、stderr 持续 drain（保留头 64KB）、五分类。
/// 溢出即时杀：读线程置共享原子标志，轮询循环立即杀组（不等超时——
/// `yes` 类无限输出进程会在管道写阻塞，必须主动收割）。
///
/// 内存上界全档化（P2-5）：Success 路径 `from_utf8_lossy(≤64MB stdout)` 瞬时可达
/// 3×（192MB）× 并发门 4 席 ≈ 0.75GB；stderr drain 头 64KB 有界。
/// 部署内存规划按此上界留余量。
pub fn run_limited(mut cmd: Command, timeout: Duration, stdout_limit: u64) -> RunOutcome {
    use std::sync::atomic::AtomicBool;
    let program = format!("{:?}", cmd.get_program());
    let slot = match SubprocSlot::acquire(&program) {
        Ok(s) => s,
        Err(b) => return b,
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组：孙进程可整组收割（否则孤儿进程滞留占资源——T-04 回收面）
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            drop(slot);
            return RunOutcome::Spawn { program, why: e.to_string() };
        }
    };
    let started = Instant::now();

    // 双读线程：防管道写满死锁；stdout 计量超限置溢出标志（轮询循环即时杀组）。
    // R3-S2-1：结果经 channel 发送，主线程限时接收——直接子进程退出但孙进程继承
    // 持有管道时，读线程读不到 EOF，无界 join 会把 1s 超时拖成实际 60s+。
    // 限时收不到 → 补杀进程组（孙进程被杀后管道关闭、线程自然退出释放）。
    use std::sync::mpsc;
    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let overflow = std::sync::Arc::new(AtomicBool::new(false));
    let ov_flag = overflow.clone();
    let out_limit = stdout_limit;
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
    // 读线程 detached：结果经 channel 回收，JoinHandle 不持有（杀组后管道关闭自然退出）
    let _ = std::thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::new();
        let mut limited = (&mut stdout_pipe).take(out_limit + 1);
        // 超限 take 最多读 limit+1 字节即返回——超限即置溢出标志
        let _ = std::io::Read::read_to_end(&mut limited, &mut buf);
        if buf.len() as u64 > out_limit {
            ov_flag.store(true, Ordering::SeqCst);
        }
        let _ = out_tx.send(buf);
    });
    let (err_tx, err_rx) = mpsc::channel::<String>();
    let _ = std::thread::spawn(move || {
        // R3-P2-4 修正版：**持续 drain 到 EOF**，只保留头部 64KB——
        // 原实现 take(64KB+1) 读满即停：子进程 stderr 超量（管道 64KB + 已读 64KB）后
        // 写管道永久阻塞 → 活进程被拖到超时误判 Timeout。drain 保证 stderr 无论
        // 多少都不阻塞子进程（stderr 是诊断面，超量丢弃不计溢出——与 stdout 分类语义不同）。
        let mut head: Vec<u8> = Vec::new();
        let mut sink = [0u8; 8192];
        loop {
            match std::io::Read::read(&mut stderr_pipe, &mut sink) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if head.len() < LIMIT_SUBPROC_STDERR as usize {
                        let room = LIMIT_SUBPROC_STDERR as usize - head.len();
                        head.extend_from_slice(&sink[..n.min(room)]);
                    }
                    // 超过 64KB 的部分：继续读并丢弃（drain，不退出不阻塞）
                }
            }
        }
        let _ = err_tx.send(String::from_utf8_lossy(&head).to_string());
    });

    // 轮询等待：超时杀组 / 溢出即时杀组（截断输出不得被当成功消费）
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if overflow.load(Ordering::SeqCst) {
                    kill_process_tree(&mut child);
                    let _ = child.wait();
                    return RunOutcome::OutputOverflow { program, limit: stdout_limit };
                }
                if started.elapsed() >= timeout {
                    kill_process_tree(&mut child);
                    let _ = child.wait();
                    return RunOutcome::Timeout { program, secs: timeout.as_secs() };
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                kill_process_tree(&mut child);
                let _ = child.wait();
                return RunOutcome::Spawn { program, why: format!("wait 失败: {e}") };
            }
        }
    }

    // 限时收流（R3-S2-1）：1.5s 内读不到 = 孙进程持有管道——补杀组 + 归类 Timeout
    // （线程留 detached，进程组被杀后管道关闭自然退出，无泄漏累积）。
    let stdout_bytes = match out_rx.recv_timeout(Duration::from_millis(1500)) {
        Ok(b) => b,
        Err(_) => {
            kill_process_tree(&mut child);
            let _ = child.wait();
            return RunOutcome::Timeout { program, secs: timeout.as_secs() };
        }
    };
    let stderr_head: String = err_rx
        .recv_timeout(Duration::from_millis(1500))
        .map(|s| s.chars().take(160).collect())
        .unwrap_or_default();
    drop(slot);

    // 竞态双检：子进程先退出但输出已溢出的场景（溢出优先——截断输出不算成功）
    if overflow.load(Ordering::SeqCst) {
        return RunOutcome::OutputOverflow { program, limit: stdout_limit };
    }
    let status = child.wait().ok();
    match status {
        Some(st) if st.success() => {
            RunOutcome::Success { stdout: String::from_utf8_lossy(&stdout_bytes).to_string() }
        }
        Some(st) => RunOutcome::NonZeroExit { program, code: st.code(), stderr_head },
        None => RunOutcome::Spawn { program, why: "无退出状态".into() },
    }
}

/// 临时目录 RAII：OCR 渲染的临时 PNG 区——超时/失败/正常路径统一回收（T-04 资源回收面）。
/// R3-S2-2：目录名 = tag + pid + 全局计数 + 纳秒——调用级唯一（并发 4 席下两个
/// 扫描件曾共享 tag+pid 目录交叉覆盖 page-*.png，静默污染 OCR 结果；崩溃残留的
/// 同名复用同样被纳秒后缀消除）。
pub struct TmpGuard(pub std::path::PathBuf);
impl TmpGuard {
    pub fn new(tag: &str) -> std::io::Result<Self> {
        static GUARD_SEQ: AtomicUsize = AtomicUsize::new(0);
        let seq = GUARD_SEQ.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("bidparse_{tag}_{}_{seq}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&p)?;
        Ok(Self(p))
    }
}
impl Drop for TmpGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------- 测试（负向为主——T-03/T-04 纪律）
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    fn tmp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join("bidparse_guard_test");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("{name}_{}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// 高熵不可压缩字节流——**限制在 ASCII 可见字符区间**（合法 UTF-8，熵高）。
    fn incompressible(len: usize) -> Vec<u8> {
        let mut x: u64 = 0x9E3779B97F4A7C15 | ((std::process::id() as u64) << 1) | 1;
        let mut v = Vec::with_capacity(len);
        for _ in 0..(len / 8 + 1) {
            x ^= x >> 12;
            x = x.wrapping_mul(0x2545F4914F6CDD1D);
            x ^= x >> 25;
            for b in x.to_le_bytes() {
                v.push(0x21 + (b % 0x5E)); // 0x21..0x7E 可见 ASCII
            }
        }
        v.truncate(len);
        v
    }

    /// 输入字节门在读任何内容之前拒绝（B4 后保留面的核心负向）：
    /// 4MB 真实文件 > 2MB 上限 → `InputTooLarge`（不读文件体）。
    /// 破坏推演：门改为「读后判」或删除 metadata 预检 → OOM 面回归 / 断言红。
    /// B4 注：docx/xlsx 输入门已收敛引擎侧（office-io 门 0）；本门护 pdf 面。
    #[test]
    fn input_size_gate_rejects_before_read() {
        let p = tmp_file("big", &incompressible(4 * 1024 * 1024));
        let err = check_input_size(&p, 2 * 1024 * 1024).unwrap_err();
        assert!(matches!(err, GuardError::InputTooLarge { .. }), "{err}");
        // 限内放行（正常路径不受门误伤）
        check_input_size(&p, 8 * 1024 * 1024).expect("限内文件必须放行");
    }

    #[test]
    fn sha256_streaming_matches_full_read() {
        let p = tmp_file("hash", b"hello world");
        let h1 = sha256_streaming(&p).unwrap();
        // 与 lib.rs 原 fs::read 全量口径逐字节等价
        use sha2::{Digest, Sha256};
        let data = std::fs::read(&p).unwrap();
        let mut h = Sha256::new();
        h.update(&data);
        assert_eq!(h1, hex::encode(h.finalize()));
    }

    // ================= T-04 子进程治理 =================
    // 并行测试共享全局并发席（LIVE_SUBPROCS）——凡起子进程的测试统一持
    // crate 级锁（guard::SUBPROC_TEST_LOCK）互斥串行，避免席位数竞态
    // （#200 R2 实锤：env_preflight/engines 的测试也起子进程，仅 guard
    // 内部互斥不够——锁提升到模块级 pub(crate) 共享）。

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        c
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_timeout_kills_and_classifies() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // sh → sleep 进程组；1.2s 超时必须杀组并分类 Timeout（不是 Success/挂死）
        let started = std::time::Instant::now();
        let out = run_limited(sh("sleep 30"), Duration::from_millis(1200), 1024 * 1024);
        assert!(matches!(out, RunOutcome::Timeout { .. }), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(5),
            "超时后 5s 内必须返回（杀组生效），实际 {:?}",
            started.elapsed());
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_output_overflow_kills_fast() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // yes 无限输出：64KB 上限 → 溢出即时杀（不等 60s 超时——快速失败纪律）
        let started = std::time::Instant::now();
        let out = run_limited(sh("yes | head -c 100M"), Duration::from_secs(60), 64 * 1024);
        assert!(matches!(out, RunOutcome::OutputOverflow { .. }), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(10),
            "溢出必须即时收割，实际 {:?}", started.elapsed());
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_nonzero_exit_classified() {
        // P2-6②：全 T-04 测试纳入互斥锁（均触全局并发席，非仅四项）
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let out = run_limited(sh("exit 3"), Duration::from_secs(10), 4096);
        match out {
            RunOutcome::NonZeroExit { code: Some(3), .. } => {}
            other => panic!("期望 NonZeroExit code=3，实际 {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_success_roundtrip() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let out = run_limited(sh("printf 'hello bid'"), Duration::from_secs(10), 4096);
        match out {
            RunOutcome::Success { stdout } => assert_eq!(stdout, "hello bid"),
            other => panic!("期望 Success，实际 {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_spawn_failure_classified() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let out = run_limited(Command::new("/nonexistent/binary/xyz"), Duration::from_secs(5), 4096);
        assert!(matches!(out, RunOutcome::Spawn { .. }), "{out:?}");
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_busy_gate_rejects_over_concurrency() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // 手动占满 4 席（同文件内可访问私有 RAII）→ 第 5 个 run 必须 Busy
        let slots: Vec<SubprocSlot> = (0..MAX_CONCURRENT_SUBPROCS)
            .map(|_| SubprocSlot::acquire("test").unwrap())
            .collect();
        let out = run_limited(sh("true"), Duration::from_secs(5), 4096);
        assert!(matches!(out, RunOutcome::Busy { .. }), "{out:?}");
        drop(slots);
        // 释放后恢复
        let out2 = run_limited(sh("true"), Duration::from_secs(5), 4096);
        assert!(out2.is_success(), "释放后应恢复，实际 {out2:?}");
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_stderr_flood_drained_not_blocked() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // R3-P2-4 负向（实测修正版）：256KB stderr。原 take(64KB+1) 读满即停后读线程退出
        // **关闭管道读端** → 子进程后续 stderr 写收到 EPIPE/SIGPIPE 被杀（实测：非审计
        // 原描述的“永久阻塞→Timeout”——关 FD 后阻塞不成立；真实危害 = verbose 合法
        // 工具（如长渲染的 pdftoppm 告警流）被 SIGPIPE 中杀 → 误判 NonZeroExit）。
        // && 使 sh 退出码依赖 dd 存活：旧行为 dd 被杀 → NonZeroExit；drain 后 → Success。
        let started = std::time::Instant::now();
        let out = run_limited(
            sh("dd if=/dev/zero bs=1024 count=256 >&2 && echo done"),
            Duration::from_secs(8),
            4096,
        );
        match out {
            RunOutcome::Success { stdout } => {
                assert_eq!(stdout.trim(), "done", "stdout 应为载荷非 stderr 残留");
            }
            other => panic!("期望 Success（stderr 洪泛不得 SIGPIPE 中杀/误判），实际 {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(5),
            "drain 生效应秒回，实际 {:?}", started.elapsed());
    }

    #[test]
    fn tmpguard_recovers_on_all_paths() {
        // 正常 drop 与提前返回路径统一回收
        let g = TmpGuard::new("t1").unwrap();
        let p = g.0.clone();
        assert!(p.exists());
        drop(g);
        assert!(!p.exists());
        // 带文件的目录
        let g2 = TmpGuard::new("t2").unwrap();
        let p2 = g2.0.clone();
        std::fs::write(g2.0.join("page-1.png"), b"png").unwrap();
        drop(g2);
        assert!(!p2.exists());
    }

    #[test]
    fn timeout_never_lies_as_success() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // T-04 核心纪律：超时/溢出/非零 → into_result 全部 Err（不误标成功）
        let t = run_limited(sh("sleep 5"), Duration::from_millis(300), 4096);
        assert!(t.into_result().is_err());
        let n = run_limited(sh("exit 2"), Duration::from_secs(5), 4096);
        assert!(n.into_result().is_err());
        let o = run_limited(sh("yes"), Duration::from_secs(3), 1024);
        assert!(o.into_result().is_err());
    }
}
