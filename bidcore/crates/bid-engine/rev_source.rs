// #327（R-23/B-05）：mdcore rev 提取——单一源码，build.rs 与集成测试共用。
//
// `include!` 进 build.rs（编译期注入 `MDCORE_REV`），`#[path]` 进
// `tests/mdcore_rev_single_source.rs`（解析逻辑真测试，非副本断言）。
//
// 解析口径与 CI 门禁 `scripts/check_mdcore_rev_sync.py` 的 TOML 侧一致：
// 只认「含 `github.com/mmqz/mdcore` 的**非注释**依赖行内的 `rev = "<40位hex>"`」。
// Cargo.toml 注释里带历史 rev 字样（升级记录），注释行必须跳过。

/// 从 Cargo.toml 文本提取 mdcore 系 git 依赖的唯一 rev。
///
/// Ok(rev)：恰好一个唯一 rev（≥1 个 mdcore 系依赖钉住且全部一致）；
/// Err(reason)：找不到 / 系内 rev 分裂，reason 面向编译错误信息。
pub fn extract_mdcore_rev(text: &str) -> Result<String, String> {
    let mut revs: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue; // 注释行跳过（注释含历史 rev 字样——#327 复核确认过）
        }
        if !line.contains("github.com/mmqz/mdcore") {
            continue; // 只认 mdcore 系依赖行（其他 git/注册表依赖不收）
        }
        if let Some(rev) = extract_rev_inline(line) {
            revs.push(rev);
        }
    }
    let mut unique: Vec<String> = revs.clone();
    unique.sort();
    unique.dedup();
    match unique.len() {
        1 => Ok(unique.remove(0)),
        0 => Err(
            "Cargo.toml 中未找到 mdcore 系 git 依赖的 rev 钉（mdc-office-io/mdcore/mdc-pdf）"
                .to_string(),
        ),
        _ => Err(format!(
            "mdcore 系依赖 rev 分裂（单源纪律要求同 rev）：{unique:?}"
        )),
    }
}

/// 提取单行 inline table 依赖声明里的 `rev = "<40位hex>"`。
/// 非 40 位 hex（branch/tag 钉法或占位）返回 None——由上层按「未找到」报错。
fn extract_rev_inline(line: &str) -> Option<String> {
    let key = "rev = \"";
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    let rev = &rest[..end];
    if rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(rev.to_string())
    } else {
        None
    }
}
