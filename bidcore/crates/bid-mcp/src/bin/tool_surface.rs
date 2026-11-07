//! 工具面注册表——**唯一真源校验与派生面重生成**。
//!
//! ## 为什么需要这个工具
//!
//! 工具面有 5 处必须手工同步：
//!
//! ```text
//! 1. bid-mcp/src/lib.rs            TOOL_NAMES 数组 + 长度常量
//! 2. bid-mcp/src/schema.rs         input_schema() 的 match 分支
//! 3. bid-mcp/src/tools.rs          dispatch 的 match 分支
//! 4. bid-domain/tests/state_machine_matrix.rs
//!                                  ALL_TOOLS + 10 状态位串
//! 5. bid-mcp/tests/dispatch_contract.rs + mcp_sdk_e2e.py  硬编码数字
//! ```
//!
//! 2026-11-07 单日因「加工具只改了一部分面」导致 6 个 PR CI 红
//! （#338/#347/#343/#354/#355/#357），全部是同一类错误。
//!
//! ## 用法
//!
//! ```bash
//! # 检查（CI 用）——不同步就红，退出码 1
//! cargo run -p bid-mcp --bin tool-surface -- --check
//!
//! # 重写派生面（开发用）——自动改 4 和 5 的文件
//! cargo run -p bid-mcp --bin tool-surface -- --write
//! ```
//!
//! 注意：1/2/3 是**手写面**（有注释、有顺序语义），本工具只**校验**不重写；
//! 4/5 是**纯派生面**（位串和数字都能从 1 算出来），本工具负责重写。

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("--check");

    let root = workspace_root();
    let matrix_path = root.join("crates/bid-domain/tests/state_machine_matrix.rs");
    let dispatch_path = root.join("crates/bid-mcp/tests/dispatch_contract.rs");
    let e2e_path = root.join("crates/bid-mcp/tests/mcp_sdk_e2e.py");
    let draft_path = root.join("crates/bid-mcp/tests/draft_tool.rs");

    let report = match mode {
        "--check" => check(&matrix_path, &dispatch_path, &e2e_path, &draft_path),
        "--write" => write(&matrix_path, &dispatch_path, &e2e_path, &draft_path),
        other => {
            eprintln!("未知参数 {other:?}（只接受 --check / --write）");
            std::process::exit(2);
        }
    };

    if let Err(Diagnostic { ref message, ref details }) = report {
        eprintln!("[tool-surface] FAIL: {message}");
        for d in details {
            eprintln!("  {d}");
        }
        eprintln!();
        eprintln!("修法：cargo run -p bid-mcp --bin tool-surface -- --write");
        std::process::exit(1);
    }

    println!("[tool-surface] OK: 工具面 {} 个，派生面全部同步", bid_mcp::TOOL_NAMES.len());
}

#[derive(Debug)]
struct Diagnostic {
    message: String,
    details: Vec<String>,
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = .../bidcore/crates/bid-mcp
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .expect("bidcore 根不可达")
        .to_path_buf()
}

fn check(matrix: &Path, dispatch: &Path, e2e: &Path, draft: &Path) -> Result<(), Diagnostic> {
    let mut details = Vec::new();
    let n = bid_mcp::TOOL_NAMES.len();

    // ---- 1. 位串矩阵：ALL_TOOLS 长度与工具名
    let matrix_src = read(matrix)?;
    let matrix_expected = render_matrix(&matrix_src, n)?;
    if matrix_expected != matrix_src {
        details.push(format!(
            "{}: ALL_TOOLS 或位串与实际 allowed_tools() 不同步",
            display(matrix)
        ));
    }

    // ---- 2. dispatch_contract 的数字
    let dispatch_src = read(dispatch)?;
    let dispatch_expected = render_dispatch(&dispatch_src, n)?;
    if dispatch_expected != dispatch_src {
        details.push(format!("{}: TOOL_NAMES.len() 断言数字不同步", display(dispatch)));
    }

    // ---- 3. python e2e 的数字
    let e2e_src = read(e2e)?;
    let e2e_expected = render_e2e(&e2e_src, n)?;
    if e2e_expected != e2e_src {
        details.push(format!("{}: 工具数断言不同步", display(e2e)));
    }

    // ---- 4. draft_tool.rs 的工具面断言
    let draft_src = read(draft)?;
    let draft_expected = render_draft(&draft_src, n)?;
    if draft_expected != draft_src {
        details.push(format!("{}: 工具面断言数字不同步", display(draft)));
    }

    if details.is_empty() {
        Ok(())
    } else {
        Err(Diagnostic {
            message: format!("工具面 {n} 个，但 {n_details} 处派生面不同步", n_details = details.len()),
            details,
        })
    }
}

fn write(matrix: &Path, dispatch: &Path, e2e: &Path, draft: &Path) -> Result<(), Diagnostic> {
    let n = bid_mcp::TOOL_NAMES.len();

    let matrix_src = read(matrix)?;
    let matrix_new = render_matrix(&matrix_src, n)?;
    if matrix_new != matrix_src {
        fs::write(matrix, &matrix_new).map_err(|e| Diagnostic {
            message: format!("写 {} 失败: {e}", display(matrix)),
            details: vec![],
        })?;
        println!("[tool-surface] 重写 {}", display(matrix));
    }

    let dispatch_src = read(dispatch)?;
    let dispatch_new = render_dispatch(&dispatch_src, n)?;
    if dispatch_new != dispatch_src {
        fs::write(dispatch, &dispatch_new).map_err(|e| Diagnostic {
            message: format!("写 {} 失败: {e}", display(dispatch)),
            details: vec![],
        })?;
        println!("[tool-surface] 重写 {}", display(dispatch));
    }

    let e2e_src = read(e2e)?;
    let e2e_new = render_e2e(&e2e_src, n)?;
    if e2e_new != e2e_src {
        fs::write(e2e, &e2e_new).map_err(|e| Diagnostic {
            message: format!("写 {} 失败: {e}", display(e2e)),
            details: vec![],
        })?;
        println!("[tool-surface] 重写 {}", display(e2e));
    }

    Ok(())
}

fn read(p: &Path) -> Result<String, Diagnostic> {
    fs::read_to_string(p).map_err(|e| Diagnostic {
        message: format!("读 {} 失败: {e}", display(p)),
        details: vec![],
    })
}

fn display(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

// ---------------------------------------------------------------- 派生面渲染

/// 重写 state_machine_matrix.rs 的 ALL_TOOLS 数组与 10 行位串。
///
/// **行尾保真**（#363 CI 假红教训）：Windows runner 的 autocrlf 会把工作区
/// 文本转成 CRLF；渲染若硬编码 `\n`，则「已同步」的文件在比较时也必不等
/// （恒红假阳性）。这里探测源文件行尾并以其原样渲染——比较与 `--write`
/// 都平台无关，且 Windows 开发者跑 `--write` 不会产生全文件行尾 diff。
fn render_matrix(src: &str, n: usize) -> Result<String, Diagnostic> {
    use bid_domain::ProjectStatus;

    let names = bid_mcp::TOOL_NAMES;
    let eol = if src.contains("\r\n") { "\r\n" } else { "\n" };

    // ---- ALL_TOOLS 数组
    let arr_start = src.find("const ALL_TOOLS: [&str;").ok_or_else(|| Diagnostic {
        message: "找不到 ALL_TOOLS 定义".into(),
        details: vec![],
    })?;
    let arr_end = src[arr_start..]
        .find("];")
        .map(|i| arr_start + i + 2)
        .ok_or_else(|| Diagnostic {
            message: "找不到 ALL_TOOLS 结束".into(),
            details: vec![],
        })?;

    let mut arr = String::new();
    arr.push_str(&format!("const ALL_TOOLS: [&str; {n}] = [{eol}"));
    for t in names {
        arr.push_str(&format!("    \"{t}\",{eol}"));
    }
    arr.push_str("];");

    let mut out = String::with_capacity(src.len() + 256);
    out.push_str(&src[..arr_start]);
    out.push_str(&arr);
    out.push_str(&src[arr_end..]);

    // ---- 10 行位串
    let states = [
        ("Draft", ProjectStatus::Draft),
        ("FilesImported", ProjectStatus::FilesImported),
        ("RequirementsExtracted", ProjectStatus::RequirementsExtracted),
        ("MatrixReady", ProjectStatus::MatrixReady),
        ("EvidenceBound", ProjectStatus::EvidenceBound),
        ("Drafting", ProjectStatus::Drafting),
        ("Validating", ProjectStatus::Validating),
        ("Reviewing", ProjectStatus::Reviewing),
        ("ReadyToExport", ProjectStatus::ReadyToExport),
        ("Exported", ProjectStatus::Exported),
    ];

    for (name, st) in states {
        let allowed = st.allowed_tools();
        let bits: String = names
            .iter()
            .map(|t| if allowed.contains(t) { '1' } else { '0' })
            .collect();

        // 定位 (ProjectStatus::<Name>, "...") 并替换位串
        let needle = format!("(ProjectStatus::{name},");
        let pos = out.find(&needle).ok_or_else(|| Diagnostic {
            message: format!("找不到状态 {name} 的位串行"),
            details: vec![],
        })?;
        let rest = &out[pos..];
        let q1 = rest.find('"').ok_or_else(|| Diagnostic {
            message: format!("{name} 位串行缺少引号"),
            details: vec![],
        })?;
        let after_q1 = &rest[q1 + 1..];
        let q2 = after_q1.find('"').ok_or_else(|| Diagnostic {
            message: format!("{name} 位串行引号不配对"),
            details: vec![],
        })?;
        let bits_start = pos + q1 + 1;
        let bits_end = bits_start + q2;
        out.replace_range(bits_start..bits_end, &bits);
    }

    Ok(out)
}

/// 重写 dispatch_contract.rs 里 TOOL_NAMES.len() 的断言数字。
fn render_dispatch(src: &str, n: usize) -> Result<String, Diagnostic> {
    let needle = "assert_eq!(TOOL_NAMES.len(),";
    let pos = src.find(needle).ok_or_else(|| Diagnostic {
        message: "找不到 TOOL_NAMES.len() 断言".into(),
        details: vec![],
    })?;
    let after = &src[pos + needle.len()..];
    let comma = after.find(',').ok_or_else(|| Diagnostic {
        message: "断言格式异常".into(),
        details: vec![],
    })?;
    let old_num = after[..comma].trim();
    if old_num == n.to_string() {
        return Ok(src.to_string());
    }
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..pos + needle.len()]);
    out.push_str(&format!(" {n},"));
    out.push_str(&after[comma + 1..]);
    Ok(out)
}

/// 重写 mcp_sdk_e2e.py 里工具数的断言数字。
fn render_e2e(src: &str, n: usize) -> Result<String, Diagnostic> {
    let needle = "check(len(tools) ==";
    let pos = src.find(needle).ok_or_else(|| Diagnostic {
        message: "找不到 list_tools 工具数断言".into(),
        details: vec![],
    })?;
    let after = &src[pos + needle.len()..];
    let comma = after.find(',').ok_or_else(|| Diagnostic {
        message: "断言格式异常".into(),
        details: vec![],
    })?;
    let old_num = after[..comma].trim();
    if old_num == n.to_string() {
        return Ok(src.to_string());
    }
    // 同时更新后面的括号说明数字
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..pos + needle.len()]);
    out.push_str(&format!(" {n},"));
    let tail = &after[comma + 1..];
    // 找 "（旧数字）" 并替换
    let paren_open = tail.find('（');
    let paren_close = tail.find('）');
    let tail_new = match (paren_open, paren_close) {
        (Some(o), Some(c)) if o < c && c < 80 => {
            let mut t = String::new();
            t.push_str(&tail[..o + 3]); // '（' is 3 bytes in UTF-8
            t.push_str(&n.to_string());
            t.push_str(&tail[c..]);
            t
        }
        _ => tail.to_string(),
    };
    out.push_str(&tail_new);
    Ok(out)
}

/// 重写 draft_tool.rs 里 `assert_eq!(arr.len(), N, ...)` 的工具面断言。
fn render_draft(src: &str, n: usize) -> Result<String, Diagnostic> {
    let needle = "assert_eq!(arr.len(),";
    let pos = src.find(needle).ok_or_else(|| Diagnostic {
        message: "找不到 draft_tool 的工具面断言".into(),
        details: vec![],
    })?;
    let after = &src[pos + needle.len()..];
    let comma = after.find(',').ok_or_else(|| Diagnostic {
        message: "断言格式异常".into(),
        details: vec![],
    })?;
    let old_num = after[..comma].trim();
    if old_num == n.to_string() {
        return Ok(src.to_string());
    }
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..pos + needle.len()]);
    out.push_str(&format!(" {n},"));
    out.push_str(&after[comma + 1..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 覆盖全部 10 状态needle 的最小骨架（与真实 state_machine_matrix.rs 同构）。
    fn fixture_src(eol: &str) -> String {
        let mut s = String::from("// fixture\n");
        s.push_str(&format!("const ALL_TOOLS: [&str; 1] = [{eol}    \"stale_tool\",{eol}];{eol}"));
        s.push_str(&format!("const CASES: &[(ProjectStatus, &str)] = &[{eol}"));
        for st in [
            "Draft", "FilesImported", "RequirementsExtracted", "MatrixReady",
            "EvidenceBound", "Drafting", "Validating", "Reviewing",
            "ReadyToExport", "Exported",
        ] {
            s.push_str(&format!("    (ProjectStatus::{st}, \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"),{eol}"));
        }
        s.push_str(&format!("];{eol}"));
        s
    }

    /// LF 源 → 渲染产物数组段仍是 LF（不引入 \r）。
    #[test]
    fn lf_source_renders_lf_array() {
        let n = bid_mcp::TOOL_NAMES.len();
        let out = render_matrix(&fixture_src("\n"), n).unwrap();
        assert!(out.contains(&format!("const ALL_TOOLS: [&str; {n}] = [\n")));
        assert!(!out.contains('\r'), "LF 源不应被渲染出 CR");
        for t in bid_mcp::TOOL_NAMES {
            assert!(out.contains(&format!("    \"{t}\",\n")), "缺工具 {t}");
        }
    }

    /// CRLF 源（Windows autocrlf 工作区形态）→ 数组段以 CRLF 渲染。
    #[test]
    fn crlf_source_renders_crlf_array() {
        let n = bid_mcp::TOOL_NAMES.len();
        let out = render_matrix(&fixture_src("\r\n"), n).unwrap();
        assert!(out.contains(&format!("const ALL_TOOLS: [&str; {n}] = [\r\n")));
        for t in bid_mcp::TOOL_NAMES {
            assert!(out.contains(&format!("    \"{t}\",\r\n")), "缺工具 {t}");
        }
    }

    /// **CI 假红回归测试**：已同步的 CRLF 源（= Windows 工作区的真实状态）
    /// render 必须幂等——修复前恒不等（LF 渲染 vs CRLF 文件），即 #363 的红。
    #[test]
    fn synced_crlf_source_render_is_idempotent() {
        let n = bid_mcp::TOOL_NAMES.len();
        let synced_lf = render_matrix(&fixture_src("\n"), n).unwrap();
        let synced_crlf = synced_lf.replace('\n', "\r\n");
        let rendered = render_matrix(&synced_crlf, n).unwrap();
        assert_eq!(rendered, synced_crlf, "已同步 CRLF 源不得因行尾被判不同步");
    }

    /// 位串长度恒等于工具数（防未来改坏位串替换面）。
    #[test]
    fn bitstrings_match_tool_count() {
        let n = bid_mcp::TOOL_NAMES.len();
        let out = render_matrix(&fixture_src("\n"), n).unwrap();
        for st in [
            "Draft", "FilesImported", "RequirementsExtracted", "MatrixReady",
            "EvidenceBound", "Drafting", "Validating", "Reviewing",
            "ReadyToExport", "Exported",
        ] {
            let needle = format!("(ProjectStatus::{st}, \"");
            let pos = out.find(&needle).unwrap_or_else(|| panic!("缺 {st} 位串行"));
            let rest = &out[pos + needle.len()..];
            let bits = &rest[..rest.find('"').unwrap()];
            assert_eq!(bits.len(), n, "{st} 位串长度应为 {n}");
            assert!(bits.chars().all(|c| c == '0' || c == '1'));
        }
    }
}
