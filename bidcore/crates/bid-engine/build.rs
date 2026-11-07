//! #327（R-23/B-05）：MDCORE_REV 单一真源——Cargo.toml 实钉 rev 编译期注入。
//!
//! ## 为什么需要这个脚本
//!
//! `MDCORE_REV` 曾是 lib.rs 手写字面量，两次 bump（aa0c859d → f4328ebf）只改了
//! Cargo.toml、漏改常量——产物 parse_summary / 诊断面全部 rev 溯源错报，且测试
//! `contains(MDCORE_REV)` 自引用断言结构上抓不到（FG-12 登记形态）。
//!
//! ## 本脚本把纪律变成编译期硬约束
//!
//! - lib.rs 常量改 `env!("BID_MDCORE_REV")` 取值——**手改字面量的漂移面从此不存在**；
//! - 解析本 crate Cargo.toml 的 mdcore git 依赖（`mmqz/mdcore`）：
//!   · 任一 mdcore 依赖缺 `rev`（浮动）→ `panic!`（编译失败）；
//!   · 多包 rev 不一致 → `panic!`（issue #2 §3.1 单源纪律：mdc-office-io /
//!     mdcore / mdc-pdf 三包必须同 rev）；
//!   · rev 非完整 40 位 hex → `panic!`（禁止 branch/tag 浮动）；
//! - Cargo.toml 变更触发重跑（`rerun-if-changed`）——bump rev 即常量自动跟随。
//!
//! bump 纪律不变：升级走独立 PR（`chore(dep): bump mdcore`）并重跑基线差分
//! （issue #2 §3.1）。

use std::collections::BTreeSet;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest = std::path::Path::new(&manifest_dir).join("Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let text = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("#327 build.rs: 读 Cargo.toml 失败: {} ({e})", manifest.display()));

    // 行级解析：mdcore git 依赖当前为单行 TOML 表格式（本仓既有风格）。
    // 注释行跳过（Cargo.toml 里有含 "mdcore" 的历史注释——不参与钉扎判定）。
    let mut pinned: BTreeSet<String> = BTreeSet::new();
    let mut floating: Vec<usize> = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with('#') || !line.contains("mmqz/mdcore") {
            continue;
        }
        match extract_rev(line) {
            Some(rev) => {
                pinned.insert(rev);
            }
            None => floating.push(idx + 1),
        }
    }

    if !floating.is_empty() {
        panic!(
            "#327 build.rs: Cargo.toml 第 {floating:?} 行的 mdcore git 依赖缺 rev \
             （浮动钉扎）。修法：补 `rev = \"<完整 40 位>\"`（issue #2 §3.1 禁止 \
             branch/tag 浮动）。若是多行 TOML 写法导致的误报，请改回单行依赖格式。"
        );
    }
    if pinned.is_empty() {
        panic!(
            "#327 build.rs: Cargo.toml 中找不到 mmqz/mdcore git 依赖——MDCORE_REV \
             的单一真源消失（本 crate 不再接引擎？）。若为架构变更，请同步处理 \
             lib.rs 的 MDCORE_REV 与本脚本。"
        );
    }
    if pinned.len() > 1 {
        panic!(
            "#327 build.rs: mdcore 多包 rev 不一致（单源纪律被破坏）: \
             {pinned:?}。修法：mdc-office-io / mdcore / mdc-pdf 统一同一 rev \
             （issue #2 §3.1——两包不同 rev = 跨包语义漂移面）。"
        );
    }

    let rev = pinned.iter().next().unwrap();
    if rev.len() != 40 || !rev.chars().all(|c| c.is_ascii_hexdigit()) {
        panic!(
            "#327 build.rs: mdcore rev `{rev}` 非完整 40 位 hex。修法：钉完整 \
             commit hash（issue #2 §3.1 禁止短 rev / branch / tag 浮动）。"
        );
    }

    println!("cargo:rustc-env=BID_MDCORE_REV={rev}");
}

/// 从单行依赖里抠 `rev = "..."` 的值。
fn extract_rev(line: &str) -> Option<String> {
    let i = line.find("rev = \"")?;
    let rest = &line[i + "rev = \"".len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}
