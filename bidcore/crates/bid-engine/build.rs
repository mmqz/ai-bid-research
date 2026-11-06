//! #327（R-23/B-05）：MDCORE_REV 单一真源注入——常量不再手写。
//!
//! 问题（审计登记）：`lib.rs` 手写常量曾停在旧 rev（aa0c859d），而 Cargo.toml
//! 实钉 f4328ebf——产物/诊断面（parse_summary 溯源、bid_health_check）报告的
//! 引擎 rev 与真实构建不符，rev 溯源失真。
//!
//! 修法（issue #327 验收建议）：常量从 Cargo.toml 单一真源生成——编译期提取
//! mdcore 系 git 依赖（mdc-office-io / mdcore / mdc-pdf）的 rev，经
//! `cargo:rustc-env` 注入 `lib.rs` 的 `env!("MDCORE_REV")`。
//!
//! 漂移即编译失败（不再依赖人记得改常量）：
//! · Cargo.toml 里找不到 mdcore 系 rev（含改用 branch 浮动钉法）→ panic；
//! · mdcore 系三包 rev 不一致（单源纪律：同 rev 钉住）→ panic。
//!
//! 解析逻辑在 `rev_source.rs`（与集成测试共用同一源码——测试真跑解析本体，
//! 不是副本断言）；`cargo test` 不会编译 build.rs 的测试目标，故单测放
//! `tests/mdcore_rev_single_source.rs`。

use std::env;
use std::fs;
use std::path::PathBuf;

// 单一源码 include（集成测试经 #[path] 引同一文件）
include!("rev_source.rs");

fn main() {
    // rev 钉变化（Cargo.toml bump）必须重跑注入
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=rev_source.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let manifest = manifest_dir.join("Cargo.toml");
    let text = fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("#327 读 {} 失败: {e}", manifest.display()));

    match extract_mdcore_rev(&text) {
        Ok(rev) => {
            println!("cargo:rustc-env=MDCORE_REV={rev}");
        }
        Err(reason) => panic!(
            "#327 MDCORE_REV 单一真源注入失败：{reason}\n\
             修复指引：mdcore 系依赖（mdc-office-io/mdcore/mdc-pdf）必须以\n\
             完整 40 位 rev 钉住（issue #2 §3.1 禁止 branch 浮动），且三包同 rev。"
        ),
    }
}
