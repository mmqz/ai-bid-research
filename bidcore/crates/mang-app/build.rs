//! 构建元数据注入（#76 设置「关于」面）。
//!
//! - `MANG_GIT_REV`：`git rev-parse --short HEAD`（git 不可用 → unknown——
//!   源码包构建也能过，不因缺 git 而红）；
//! - `MANG_BUILD_EPOCH`：build.rs 运行的 Unix 秒（前端 Date 渲染成本地时区）。
//!
//! 刻意不做：rerun-if-changed 挂 .git/HEAD（每次提交都全量重编不值——
//! rev 落后一个提交无害，CI 里本来就是新 checkout）。

fn main() {
    let rev = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=MANG_GIT_REV={rev}");
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=MANG_BUILD_EPOCH={epoch}");
    println!("cargo:rerun-if-changed=build.rs");
}
