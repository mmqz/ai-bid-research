//! #327：MDCORE_REV 单一真源防回归测试。
//!
//! 三层守卫（编译期 + 测试期）：
//! 1. 编译期：build.rs 注入 `env!("MDCORE_REV")`——Cargo.toml 无 rev 钉 / 系内
//!    分裂直接编不过（漂移即编译失败，验收主断言）；
//! 2. 本测试：解析本体（`rev_source.rs` 与 build.rs 同一源码 include）对
//!    真实 Cargo.toml 的提取断言——负例（注释 rev 误收 / 分裂 / branch 钉）；
//! 3. 本测试：`MDCORE_REV` 常量 == 真实 Cargo.toml 提取值——防有人把 lib.rs
//!    改回手写常量后再次漂移（build.rs 照常注入但 env! 不再被消费的假绿面）。

// 与 build.rs 共用同一解析源码（单一真源，非副本）
#[path = "../rev_source.rs"]
mod rev_source;

use rev_source::extract_mdcore_rev;

/// 真实 Cargo.toml 的 rev 钉必须可提取且为 40 位 hex。
#[test]
fn 真实manifest可提取40位hex_rev() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).expect("读 Cargo.toml");
    let rev = extract_mdcore_rev(&text).expect("mdcore 系 rev 提取");
    assert_eq!(rev.len(), 40, "rev 须为完整 40 位: {rev}");
    assert!(
        rev.chars().all(|c| c.is_ascii_hexdigit()),
        "rev 须为 hex: {rev}"
    );
}

/// 常量与 manifest 单一真源一致（防手写常量回归——#327 本体）。
#[test]
fn 常量与manifest单一真源一致() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).expect("读 Cargo.toml");
    let rev = extract_mdcore_rev(&text).expect("mdcore 系 rev 提取");
    assert_eq!(
        bid_engine::MDCORE_REV, rev,
        "MDCORE_REV 与 Cargo.toml 钉住的 rev 漂移——若 lib.rs 被改回手写常量即红"
    );
}

/// 常量线格式：40 位 hex（parse_summary 溯源面的消费约定）。
#[test]
fn 常量为40位hex() {
    let rev = bid_engine::MDCORE_REV;
    assert_eq!(rev.len(), 40, "MDCORE_REV 须为完整 40 位: {rev}");
    assert!(rev.chars().all(|c| c.is_ascii_hexdigit()));
}

/// 负例：注释行里的历史 rev 字样不得误收（现实形态——bid-engine
/// Cargo.toml 注释带「旧 rev → 新 rev」升级记录）。
#[test]
fn 负例_注释行历史rev不误收() {
    let toml = r#"
# rev = mdcore main @ f4328ebf4f936e1321a2254382d59f9ab9f94b7b（bump 记录，旧 3204342d）
[dependencies]
mdc-office-io = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", package = "mdc-office-io" }
"#;
    assert_eq!(
        extract_mdcore_rev(toml).unwrap(),
        "f4328ebf4f936e1321a2254382d59f9ab9f94b7b"
    );
}

/// 负例：mdcore 系内 rev 分裂必须报错（单源纪律）。
#[test]
fn 负例_系内rev分裂报错() {
    let toml = r#"
[dependencies]
mdc-office-io = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", package = "mdc-office-io" }
mdc-pdf = { git = "https://github.com/mmqz/mdcore", rev = "aa0c859d956597992c2d93e9055e2f0beca017c3", package = "mdc-pdf" }
"#;
    let err = extract_mdcore_rev(toml).unwrap_err();
    assert!(err.contains("分裂"), "实际: {err}");
}

/// 负例：branch 浮动钉法（issue #2 §3.1 禁止）提取不到 rev 必须报错。
#[test]
fn 负例_branch浮动钉报错() {
    let toml = r#"
[dependencies]
mdcore = { git = "https://github.com/mmqz/mdcore", branch = "main" }
"#;
    assert!(extract_mdcore_rev(toml).is_err());
}

/// 负例：非 mdcore 系 git 依赖的 rev 不属于本注入面。
#[test]
fn 负例_非mdcore系rev不收() {
    let toml = r#"
[dependencies]
other = { git = "https://github.com/mmqz/some-other", rev = "1111111111111111111111111111111111111111" }
"#;
    assert!(extract_mdcore_rev(toml).is_err());
}
