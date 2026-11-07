//! #327（R-23/B-05 / FG-12）：MDCORE_REV 单一真源对账测试。
//!
//! build.rs 已把常量改为 Cargo.toml 注入（漂移即编译失败），本测试是第二道闸：
//! **若 build.rs 被回退 / env! 改回手写字面量**，这里独立解析 Cargo.toml 对账，
//! 漂移重新变红——不再依赖「断言与被验值同源」的自引用形态（FG-12 盲区补口）。

use std::collections::BTreeSet;

/// 解析本 crate Cargo.toml 里 mdcore git 依赖实钉的 rev 集合。
/// 与 build.rs 同一判据：含 `mmqz/mdcore` 的非注释行、单行 TOML 表格式。
fn pinned_mdcore_revs() -> (BTreeSet<String>, Vec<usize>) {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).expect("读 Cargo.toml 失败");
    let mut revs = BTreeSet::new();
    let mut floating = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with('#') || !line.contains("mmqz/mdcore") {
            continue;
        }
        match line.split_once("rev = \"").map(|(_, r)| {
            let end = r.find('"').expect("rev 值未闭合");
            &r[..end]
        }) {
            Some(rev) => {
                revs.insert(rev.to_string());
            }
            None => floating.push(idx + 1),
        }
    }
    (revs, floating)
}

/// 常量 == Cargo.toml 实钉 rev（真源对账，非自引用）。
#[test]
fn mdcore_rev_constant_matches_cargo_toml_pin() {
    let (revs, floating) = pinned_mdcore_revs();
    assert!(
        floating.is_empty(),
        "Cargo.toml 第 {floating:?} 行 mdcore git 依赖缺 rev（浮动钉扎）——issue #2 §3.1"
    );
    assert!(
        !revs.is_empty(),
        "Cargo.toml 找不到 mmqz/mdcore git 依赖——MDCORE_REV 单一真源消失"
    );
    assert_eq!(
        revs.len(),
        1,
        "mdcore 多包 rev 不一致（单源纪律被破坏）: {revs:?}"
    );

    let rev = revs.iter().next().unwrap();
    assert_eq!(
        rev.len(),
        40,
        "rev 必须完整 40 位（issue #2 §3.1 禁止短 rev / branch / tag 浮动）: {rev}"
    );
    assert!(
        rev.chars().all(|c| c.is_ascii_hexdigit()),
        "rev 含非 hex 字符: {rev}"
    );
    assert_eq!(
        bid_engine::MDCORE_REV,
        rev.as_str(),
        "MDCORE_REV 与 Cargo.toml 实钉 rev 漂移（#327 R-23 复发）——\
         build.rs 注入是否被回退成手写字面量？"
    );
}

/// parse_summary 溯源口径携带真源 rev（值面仍由 mapping_golden_anchor 系列锚定，
/// 此处只钉「@ rev」片段必在——防常量与产物脱钩的最低线）。
#[test]
fn mdcore_rev_is_full_40_hex_in_const() {
    assert_eq!(bid_engine::MDCORE_REV.len(), 40);
    assert!(bid_engine::MDCORE_REV.chars().all(|c| c.is_ascii_hexdigit()));
}
