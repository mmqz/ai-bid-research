//! env_preflight —— 环境预检（issue #9）：外部依赖缺失 → 测试 **SKIP**，绝不伪装成业务 FAIL。
//!
//! 背景（issue #9 实录）：缺 `tesseract`/`pdftoppm`/`pdftotext`/`pdfinfo` 时，
//! `e2e_m1_all_scenarios_pass` 报「TC-01 判分未过 verdict=FAIL」——看起来像抽取
//! 逻辑坏了，实际是环境缺件。任何拿到这个失败的人第一反应都会去查
//! `extract.rs`，浪费大量时间。
//!
//! 语义（issue #9 判定标准）：
//! - 有依赖缺失时，测试输出必须是 `SKIP: <test_id>` + `reason: ENV_MISSING: <工具清单>`；
//! - 「依赖缺失」不得判成「业务 FAIL」；
//! - stable Rust 的 libtest 无运行时 ignore 面——本模块以「响亮横幅 + 提前 return」
//!   实现（测试计数显示 passed，但输出首屏即见 SKIP 与原因，`--nocapture` 下不可错过）。
//!
//! 可测性设计：`BID_PREFLIGHT_PATH` 环境变量覆盖查找路径——CI/单测可模拟
//! 「缺件机器」（如无 poppler 的 Windows）而不必真改系统 PATH（改 PATH 会
//! 干扰同进程并行测试与子进程 cargo）。

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// PDF 全链（文本层 + 扫描件 OCR + 页数探测）所需的外部工具四件套。
pub const PDF_CHAIN_TOOLS: &[&str] = &["pdftotext", "pdftoppm", "pdfinfo", "tesseract"];

/// OCR 语言数据预检名（`missing_tools` 的特殊成员）：检查 `chi_sim.traineddata`
/// 在 `pdf.rs` 同款解析路径（`TESSDATA_PREFIX` 或 `~/.tessdata`）在位——
/// 本环境实测踩坑：tesseract 二进制在位而语言数据缺失，OCR 链路照断，
/// 错误面仍是「业务 FAIL」伪装（issue #9 的一个隐蔽变体）。
pub const OCR_LANG_CHI_SIM: &str = "chi_sim.traineddata";

/// 全链预检清单：四件套 + chi_sim 语言数据（消费方一站式常量）。
pub const FULL_PDF_CHAIN: &[&str] = &[
    "pdftotext",
    "pdftoppm",
    "pdfinfo",
    "tesseract",
    OCR_LANG_CHI_SIM,
];

fn tessdata_dir() -> PathBuf {
    std::path::PathBuf::from(std::env::var("TESSDATA_PREFIX").unwrap_or_else(|_| {
        format!("{}/.tessdata", std::env::var("HOME").unwrap_or_default())
    }))
}

/// 探测工具是否可调用（spawn 成功即在位——退出码无关：参数不兼容也算「有这个程序」，
/// 与原始失败面 `"pdftotext" 无法启动: program not found` 的判定口径一致）。
fn tool_available(tool: &str) -> bool {
    if tool == OCR_LANG_CHI_SIM {
        return tessdata_dir().join(tool).is_file();
    }
    Command::new(tool)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// 受控查找路径：`BID_PREFLIGHT_PATH` 优先（模拟缺件环境用），否则继承当前 PATH。
fn search_dirs() -> Vec<PathBuf> {
    if let Ok(over) = std::env::var("BID_PREFLIGHT_PATH") {
        return std::env::split_paths(&over).collect();
    }
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
}

/// 在给定目录集合里查工具（POSIX 直名；Windows 追加 .exe——与 Command 的
/// 查找口径对齐，覆盖本仓两类开发环境）。
fn tool_in_dirs(tool: &str, dirs: &[PathBuf]) -> bool {
    let mut names = vec![tool.to_string()];
    if cfg!(windows) {
        names.push(format!("{tool}.exe"));
    }
    dirs.iter().any(|d| names.iter().any(|n| d.join(n).is_file()))
}

/// 返回缺失的外部工具清单（空 = 环境完备）。
///
/// 覆盖面语义：`BID_PREFLIGHT_PATH` 未设时走真实探测（spawn 试调 + 语言数据在位）；
/// 设置后模拟「裸机」（目录存在性检查 + 语言数据判缺——模拟面必然全缺，
/// 与真实缺件机器的观测面一致）。
pub fn missing_tools(tools: &[&str]) -> Vec<String> {
    if let Ok(over) = std::env::var("BID_PREFLIGHT_PATH") {
        let dirs: Vec<PathBuf> = std::env::split_paths(&over).collect();
        return tools
            .iter()
            .filter(|t| {
                if **t == OCR_LANG_CHI_SIM {
                    true // 模拟裸机：语言数据一并判缺
                } else {
                    !tool_in_dirs(t, &dirs)
                }
            })
            .map(|t| t.to_string())
            .collect();
    }
    tools
        .iter()
        .filter(|t| !tool_available(t))
        .map(|t| t.to_string())
        .collect()
}

/// 测试入口预检：缺件则打印 SKIP 横幅并返回 `true`（调用方应立即 `return`）。
///
/// 用法（issue #9 要求的形态）：
/// ```ignore
/// #[test]
/// fn e2e_m1_all_scenarios_pass() {
///     if bid_parse::env_preflight::skip_if_env_missing(
///         "e2e_m1_all_scenarios_pass", bid_parse::env_preflight::PDF_CHAIN_TOOLS) {
///         return;
///     }
///     // ……真实测试体
/// }
/// ```
pub fn skip_if_env_missing(test_id: &str, tools: &[&str]) -> bool {
    let missing = missing_tools(tools);
    if missing.is_empty() {
        return false;
    }
    eprintln!();
    eprintln!("====================================================================");
    eprintln!("SKIP: {test_id}");
    eprintln!("reason: ENV_MISSING: {}", missing.join(", "));
    eprintln!("（环境缺件，非业务失败——issue #9：依赖缺失不得伪装成业务 FAIL）");
    eprintln!("补装指引: poppler-utils(pdftotext/pdftoppm/pdfinfo) + tesseract-ocr");
    eprintln!("====================================================================");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_tool_reported_missing() {
        // 真实 PATH 下，不存在的工具必然被报告（探测逻辑的底线正确性）
        let miss = missing_tools(&["definitely-not-a-real-tool-xyz-12345"]);
        assert_eq!(miss, vec!["definitely-not-a-real-tool-xyz-12345".to_string()]);
    }

    #[test]
    fn skip_helper_returns_true_only_when_missing() {
        // 一致性不变量：返回值 ⟺ 缺件清单非空（横幅与判定不得脱钩）
        let tools = ["definitely-not-a-real-tool-xyz-12345"];
        assert!(skip_if_env_missing("unit_probe_fake", &tools));
        let present = ["cargo"]; // 测试进程内 cargo 必在 PATH（cargo test 自身依赖它）
        assert!(!skip_if_env_missing("unit_probe_cargo", &present));
    }

    #[test]
    fn restricted_path_simulates_missing_env() {
        // BID_PREFLIGHT_PATH 指向空目录 → 外部工具 + 语言数据全部判缺
        // （模拟无 poppler / 无 tessdata 的机器）
        let empty = std::env::temp_dir().join("bid-preflight-empty-dirs");
        std::fs::create_dir_all(&empty).unwrap();
        // SAFETY(env mut): 同 `skip_helper_returns_true_only_when_missing` 注记
        unsafe { std::env::set_var("BID_PREFLIGHT_PATH", &empty) };
        let full_chain: Vec<&str> = PDF_CHAIN_TOOLS
            .iter()
            .copied()
            .chain(std::iter::once(OCR_LANG_CHI_SIM))
            .collect();
        let miss = missing_tools(&full_chain);
        unsafe { std::env::remove_var("BID_PREFLIGHT_PATH") };
        assert_eq!(
            miss.len(),
            full_chain.len(),
            "空 PATH 模拟下全链工具应全部判缺，实际: {miss:?}"
        );
    }

    #[test]
    fn chi_sim_check_mirrors_pdf_rs_resolution() {
        // 语言数据判定与 pdf.rs 的解析路径同源：TESSDATA_PREFIX 优先、
        // 否则 ~/.tessdata。真实 PATH 下（无覆盖变量），二进制在位时
        // chi_sim 判定必须与文件在位性严格一致（防「探测面与消费面分叉」）。
        let expect = tessdata_dir().join(OCR_LANG_CHI_SIM).is_file();
        let miss = missing_tools(&[OCR_LANG_CHI_SIM]);
        assert_eq!(
            miss.is_empty(),
            expect,
            "chi_sim 探测({})与 pdf.rs 消费面({expect})不一致",
            if miss.is_empty() { "在位" } else { "缺失" }
        );
    }
}
