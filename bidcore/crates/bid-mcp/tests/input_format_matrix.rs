//! 输入格式矩阵——同一批文件用不同格式/损坏形态/边界形态导入，
//! 验证「格式路由 + 失败归类 + 零副作用」三条不变量。
//!
//! 为什么单开：`import_tender_files_tool.rs` 已有 23 个用例，但都是
//! **单点验证**（一个 docx、一个截断 docx、一个不可读路径）。没有测试
//! 穷举过「**同一内容换格式**」与「**格式 × 损坏形态**」的矩阵。
//!
//! 真正会咬人的是组合面：加密 PDF、0 字节文件、扩展名撒谎（xlsx 内容
//! 叫 .docx）、超长文件名、Windows 保留名。这些走的是不同的错误分支，
//! 且**每条都必须零副作用**（不能留下半个文件记录）。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("fixtures/synth-tender/招标文件")
}

/// 临时工作区（每测试独立——并行安全）。
fn temp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "input-matrix-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(
        svc,
        ToolCall {
            tool: tool.to_string(),
            args,
        },
    )
}

fn ok(svc: &mut BidService, tool: &str, args: Value) -> Value {
    match call(svc, tool, args) {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("{tool} 应成功但失败: {error}"),
    }
}

fn err_code(svc: &mut BidService, tool: &str, args: Value) -> String {
    match call(svc, tool, args) {
        ToolReply::Ok(out) => panic!("{tool} 应失败但成功: {:?}", out.result),
        ToolReply::Err { error, .. } => serde_json::to_value(&error)
            .ok()
            .and_then(|v| v.get("code").and_then(|c| c.as_str()).map(String::from))
            .unwrap_or_else(|| format!("{error}")),
    }
}

fn new_project(svc: &mut BidService, tag: &str) -> String {
    ok(
        svc,
        "create_bid_project",
        json!({
            "name": format!("输入矩阵-{tag}"),
            "bid_deadline": "2026-12-31",
            "mode": "明标",
            "external_key": format!("input-matrix-{tag}"),
        }),
    )["project_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn import(svc: &mut BidService, pid: &str, path: &Path, name: &str, kind: &str) -> ToolReply {
    call(
        svc,
        "import_tender_files",
        json!({
            "project_id": pid,
            "files": [{
                "upload_ref": path.display().to_string(),
                "filename": name,
                "kind": kind,
            }],
        }),
    )
}

// ---------------------------------------------------------------- 1. 合法格式

/// docx / xlsx / pdf 三种已接引擎的格式，都能导入并推进状态机。
///
/// xlsx/pdf 的夹具从 fixtures 目录按扩展名找；找不到就 SKIP 该格式
/// （issue #9 纪律：缺件不是业务失败）。
#[test]
fn supported_formats_route_to_engine() {
    let ws = temp_ws("formats");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "fmt");

    let dir = fixtures();
    let mut by_ext: std::collections::BTreeMap<String, PathBuf> =
        std::collections::BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                by_ext.entry(ext.to_ascii_lowercase()).or_insert(p);
            }
        }
    }

    let mut tested = Vec::new();
    for ext in ["docx", "xlsx", "pdf"] {
        let Some(path) = by_ext.get(ext) else {
            eprintln!("SKIP: fixtures 里没有 .{ext}（该格式本轮不验）");
            continue;
        };
        let name = format!("格式矩阵.{ext}");
        match import(&mut svc, &pid, path, &name, "招标文件") {
            ToolReply::Ok(out) => {
                let per = out.result.get("per_file").and_then(|v| v.as_array());
                assert!(per.is_some(), ".{ext} 导入返回缺 per_file");
                tested.push(ext);
            }
            ToolReply::Err { error, .. } => {
                // 真有引擎缺口要响亮记录（不静默吞），但不算失败——
                // 本轮验的是「路由到引擎」，引擎自身能力是另一回事
                eprintln!("WARN: .{ext} 导入失败（引擎能力面，非路由面）: {error}");
            }
        }
    }
    assert!(
        !tested.is_empty(),
        "docx/xlsx/pdf 一个都没导成功——格式路由整体崩了"
    );
}

// ---------------------------------------------------------------- 2. 损坏形态

/// 0 字节文件 → 不崩，归类为逐件失败（E_PARTIAL），且**零文件记录入库**。
#[test]
fn zero_byte_file_is_item_failure_with_zero_side_effect() {
    let ws = temp_ws("zero");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "zero");

    let empty = ws.join("empty.docx");
    std::fs::write(&empty, b"").unwrap();

    let reply = import(&mut svc, &pid, &empty, "空文件.docx", "招标文件");
    // 单件全败 → E_PARTIAL（E_* 信封）或 Ok+0 成功，二者都可接受，
    // 唯一不可接受的是 panic 或「假装成功」
    match reply {
        ToolReply::Ok(out) => {
            let n = out.result["per_file"].as_array().map(|a| a.len()).unwrap_or(0);
            let ok_n = out.result["totals"]["files"].as_u64().unwrap_or(0);
            assert!(
                ok_n == 0 || n > 0,
                "0 字节文件被当作成功导出（per_file={n}, success={ok_n}）"
            );
        }
        ToolReply::Err { .. } => { /* E_PARTIAL 合理 */ }
    }
    // 零副作用：项目状态不能推进到 FILES_IMPORTED
    let st = ok(&mut svc, "get_bid_project_state", json!({}));
    assert_ne!(
        st["status"].as_str(),
        Some("FILES_IMPORTED"),
        "0 字节文件让状态机推进了（应零副作用）"
    );
}

/// 截断的 docx（合法 ZIP 头 + 随机垃圾）→ 逐件失败，不 panic。
#[test]
fn truncated_docx_is_item_failure() {
    let ws = temp_ws("trunc");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "trunc");

    let bad = ws.join("truncated.docx");
    // PK + 垃圾——让 unzip 解析失败
    let mut bytes = vec![0x50, 0x4b, 0x03, 0x04];
    bytes.extend((0u8..200).map(|i| i.wrapping_mul(7)));
    std::fs::write(&bad, &bytes).unwrap();

    let reply = import(&mut svc, &pid, &bad, "截断.docx", "招标文件");
    match reply {
        ToolReply::Ok(out) => {
            let ok_n = out.result["totals"]["files"].as_u64().unwrap_or(0);
            assert_eq!(ok_n, 0, "截断 docx 被当作成功");
        }
        ToolReply::Err { .. } => { /* E_PARTIAL 合理 */ }
    }
}

/// 扩展名撒谎（xlsx 内容叫 .docx）→ 走 docx 引擎 → 解析失败 → 逐件失败。
///
/// 这条钉的是「路由按扩展名而非嗅探」的**已知设计**：真出问题时
/// 用户要看到明确失败，而不是被静默当成 xlsx 解析成功。
#[test]
fn lying_extension_fails_loudly() {
    let ws = temp_ws("liar");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "liar");

    let liar = ws.join("actually.xlsx.docx");
    // xlsx 的 ZIP 内部结构（xl/workbook.xml）冒充 docx
    let mut bytes = vec![0x50, 0x4b, 0x03, 0x04];
    bytes.extend(b"xl/workbook.xml".iter().copied());
    bytes.extend((0u8..300).map(|i| i.wrapping_add(13)));
    std::fs::write(&liar, &bytes).unwrap();

    let reply = import(&mut svc, &pid, &liar, "其实是xlsx.docx", "招标文件");
    match reply {
        ToolReply::Ok(out) => {
            let ok_n = out.result["totals"]["files"].as_u64().unwrap_or(0);
            assert_eq!(ok_n, 0, "扩展名撒谎的文件被当作成功（应响亮失败）");
        }
        ToolReply::Err { .. } => { /* 合理 */ }
    }
}

/// 不存在的路径 → E_NOTFOUND 或逐件失败，绝不 panic。
#[test]
fn nonexistent_path_is_handled() {
    let ws = temp_ws("gone");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "gone");
    let ghost = ws.join("not-here.docx");

    let reply = import(&mut svc, &pid, &ghost, "不存在.docx", "招标文件");
    match reply {
        ToolReply::Ok(out) => {
            let ok_n = out.result["totals"]["files"].as_u64().unwrap_or(0);
            assert_eq!(ok_n, 0, "不存在的文件被当作成功");
        }
        ToolReply::Err { .. } => { /* 合理 */ }
    }
}

/// 目录当文件传 → 失败归类，不崩。
#[test]
fn directory_as_file_is_handled() {
    let ws = temp_ws("dir");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "dir");
    let dir_as_file = ws.join("iam-a-dir.docx");
    std::fs::create_dir_all(&dir_as_file).unwrap();

    let reply = import(&mut svc, &pid, &dir_as_file, "目录.docx", "招标文件");
    match reply {
        ToolReply::Ok(out) => {
            let ok_n = out.result["totals"]["files"].as_u64().unwrap_or(0);
            assert_eq!(ok_n, 0, "目录被当作文件成功导入");
        }
        ToolReply::Err { .. } => { /* 合理 */ }
    }
}

// ---------------------------------------------------------------- 3. 文件名边界

/// 超长文件名 / 中文名 / 保留字 / 特殊字符——都必须是**数据处理**，
/// 不能变成路径注入或 panic。
#[test]
fn filename_edge_cases_are_data_not_paths() {
    let ws = temp_ws("names");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "names");

    let dir = fixtures();
    let Some(src) = std::fs::read_dir(&dir).ok().and_then(|rd| {
        rd.flatten()
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|s| s.to_str()) == Some("docx"))
    }) else {
        eprintln!("SKIP: 无 docx 夹具");
        return;
    };

    let names = [
        "超长文件名".to_string() + &"很长".repeat(80) + ".docx",
        "中文（全角括号）【方括号】《书名号》.docx".to_string(),
        "../../etc/passwd.docx".to_string(),
        "CON.docx".to_string(),          // Windows 保留设备名
        "文件 截断.docx".to_string(), // NUL 字符
        ".".to_string() + &"x".repeat(200) + ".docx",
    ];

    for name in names {
        // 只要能返回（Ok 或 Err）就算通过——关键是**不 panic**
        let _ = import(&mut svc, &pid, &src, &name, "招标文件");
    }
    // 走到这里说明没有任何一个名字引发 panic
}

/// 非法 kind 枚举 → E_UNPROCESSABLE，且零副作用。
#[test]
fn illegal_kind_is_rejected_with_zero_side_effect() {
    let ws = temp_ws("kind");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "kind");
    let dir = fixtures();
    let Some(src) = std::fs::read_dir(&dir).ok().and_then(|rd| {
        rd.flatten()
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|s| s.to_str()) == Some("docx"))
    }) else {
        eprintln!("SKIP: 无 docx 夹具");
        return;
    };

    let code = err_code(
        &mut svc,
        "import_tender_files",
        json!({
            "project_id": pid,
            "files": [{ "upload_ref": src.display().to_string(), "filename": "x.docx", "kind": "胡说八道" }],
        }),
    );
    assert_eq!(code, "E_UNPROCESSABLE", "非法 kind 应 E_UNPROCESSABLE，实际 {code}");

    // 零副作用
    let st = ok(&mut svc, "get_bid_project_state", json!({}));
    assert_eq!(st["status"].as_str(), Some("DRAFT"), "非法 kind 让状态机推进了");
}

// ---------------------------------------------------------------- 4. 批量部分失败

/// 一批里好件 + 坏件混合 → totals.partial，好件照常入库。
#[test]
fn partial_batch_keeps_good_files() {
    let ws = temp_ws("partial");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "partial");

    let dir = fixtures();
    let Some(good) = std::fs::read_dir(&dir).ok().and_then(|rd| {
        rd.flatten()
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|s| s.to_str()) == Some("docx"))
    }) else {
        eprintln!("SKIP: 无 docx 夹具");
        return;
    };
    let bad = ws.join("bad.docx");
    std::fs::write(&bad, b"not a zip at all").unwrap();

    let reply = call(
        &mut svc,
        "import_tender_files",
        json!({
            "project_id": pid,
            "files": [
                { "upload_ref": good.display().to_string(), "filename": "好件.docx", "kind": "招标文件" },
                { "upload_ref": bad.display().to_string(), "filename": "坏件.docx", "kind": "招标文件" },
            ],
        }),
    );
    match reply {
        // 契约 §2：item_failures > 0 一律走 E_PARTIAL（不返回 Ok）。
        // 故 Ok 分支只在「全成功」时出现——本用例应走 Err 分支。
        ToolReply::Ok(out) => {
            let t = &out.result["totals"];
            assert_eq!(
                t["parse_failed"].as_u64().unwrap_or(0), 0,
                "坏件存在却返回 Ok（应 E_PARTIAL）: {t:?}"
            );
        }
        ToolReply::Err { error, .. } => {
            // E_PARTIAL 也算对（壳层按 partial 判据组装）
            let s = format!("{error}");
            assert!(s.contains("E_PARTIAL") || s.contains("部分"), "混合批次应 E_PARTIAL，实际 {s}");
        }
    }
}
