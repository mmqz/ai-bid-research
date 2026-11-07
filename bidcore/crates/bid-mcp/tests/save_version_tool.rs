//! save_version（issue #42 阶段 10 + #374 围栏）：AI 主动存版本。
//!
//! 验收面：
//!   * 正常路径：绝对路径 + message → ok=true，index 里 kind=ai、message 落盘，
//!     与桌面壳（mang-snap 单一真源）写同一份 `.mang/history`
//!   * 工作区根发现：向上找 `.mang`；找不到用文件所在目录（两者都在围栏内）
//!   * 拒绝面：相对路径 / 空 message / 非文本 / 不存在的文件 → 类型化错误（不空成功）
//!   * 断点幂等：同一内容再存 → 第二份（kind=ai 不去重，AI 明确要求存版本就存）
//!   * #374 围栏（R-27/C-06）：file_path 经 fs_fence（BID_WS/BID_FS_ROOT 锚定），
//!     无根响亮拒 / 围栏外拒（先于存在性——不泄露 oracle）/ 祖先 .mang 越出
//!     围栏拒 / symlink 段拒。
//!
//! 测试经 `dispatch_with` 注入独立服务（测试隔离面——不用全局 LazyLock，
//! 围栏根随测试目录注入 `with_fs_root`）。

use bid_mcp::schema::ToolCall;
use bid_mcp::{dispatch_with, ToolReply};
use bid_tools::service::BidService;
use serde_json::json;
use std::path::{Path, PathBuf};

/// 带围栏根的独立服务（#374：save_version 需要 fs_roots——纯内存态响亮拒）。
fn fenced_service(root: &Path) -> BidService {
    BidService::new().with_fs_root(root)
}

fn call_save(svc: &mut BidService, file_path: &str, message: &str) -> ToolReply {
    dispatch_with(
        svc,
        ToolCall {
            tool: "save_version".to_string(),
            args: json!({ "file_path": file_path, "message": message }),
        },
    )
}

fn temp_ws(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "mcp-sv-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn read_index(ws: &std::path::Path) -> Vec<serde_json::Value> {
    let p = ws.join(".mang/history/index.json");
    let bytes = std::fs::read(p).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["items"].as_array().unwrap().clone()
}

#[test]
fn save_version_writes_ai_entry_into_mang_history() {
    let ws = temp_ws("ok");
    std::fs::create_dir_all(ws.join("docs")).unwrap();
    std::fs::create_dir_all(ws.join(".mang")).unwrap(); // 桌面壳建过的根标记
    let file = ws.join("docs/技术标.md");
    std::fs::write(&file, "# 技术标\n\n第1章 概述完成").unwrap();
    let mut svc = fenced_service(&ws);

    match call_save(&mut svc, file.to_str().unwrap(), "第1章 概述完成") {
        ToolReply::Ok(o) => {
            assert_eq!(o.result["ok"], json!(true));
            assert_eq!(o.result["kind"], json!("ai"));
            assert_eq!(o.result["file"], json!("docs/技术标.md"));
            assert!(o.result["snapshot"].as_str().unwrap().contains('-'));
        }
        ToolReply::Err { error, .. } => panic!("正常路径不得失败: {error:?}"),
    }

    // index：kind=ai + message 落盘
    let items = read_index(&ws);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], json!("ai"));
    assert_eq!(items[0]["message"], json!("第1章 概述完成"));
    assert_eq!(items[0]["file"], json!("docs/技术标.md"));

    // 与桌面壳同一真源：mang-snap 的读端能拿到同一份快照
    let snap = items[0]["snapshot"].as_str().unwrap();
    let content = mang_snap::read(&ws, "docs/技术标.md", snap).unwrap();
    assert!(content.contains("第1章 概述完成"));

    // 再存一版（AI 每章调用）→ 两份
    std::fs::write(&file, "# 技术标\n\n第1章 概述完成\n\n第2章 技术方案完成").unwrap();
    match call_save(&mut svc, file.to_str().unwrap(), "第2章 技术方案完成") {
        ToolReply::Ok(_) => {}
        ToolReply::Err { error, .. } => panic!("第二版不得失败: {error:?}"),
    }
    let items = read_index(&ws);
    assert_eq!(items.len(), 2);

    let _ = std::fs::remove_dir_all(ws);
}

#[test]
fn save_version_rejections_are_typed() {
    let ws = temp_ws("rej");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("a.md"), "x").unwrap();
    std::fs::write(ws.join("b.xlsx"), "bin").unwrap();
    let mut svc = fenced_service(&ws);

    // 相对路径
    match call_save(&mut svc, "a.md", "msg") {
        ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_UNPROCESSABLE"),
        ToolReply::Ok(_) => panic!("相对路径不得成功"),
    }
    // 空 message
    match call_save(&mut svc, ws.join("a.md").to_str().unwrap(), "  ") {
        ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_UNPROCESSABLE"),
        ToolReply::Ok(_) => panic!("空 message 不得成功"),
    }
    // 非文本
    match call_save(&mut svc, ws.join("b.xlsx").to_str().unwrap(), "msg") {
        ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_UNPROCESSABLE"),
        ToolReply::Ok(_) => panic!("二进制不得成功"),
    }
    // 不存在（围栏内缺失件——E_NOTFOUND 语义保留，围栏层不抢答）
    match call_save(&mut svc, ws.join("nope.md").to_str().unwrap(), "msg") {
        ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_NOTFOUND"),
        ToolReply::Ok(_) => panic!("不存在的文件不得成功"),
    }
    let _ = std::fs::remove_dir_all(ws);
}

#[test]
fn save_version_discovers_root_without_mang_marker() {
    // 没有 .mang 标记：用文件所在目录做根（快照仍可用，只是不向上归并）
    // ——root 仍在围栏内（#374 复验面）
    let ws = temp_ws("root");
    std::fs::create_dir_all(ws.join("sub")).unwrap();
    let file = ws.join("sub/c.md");
    std::fs::write(&file, "内容").unwrap();
    let mut svc = fenced_service(&ws);
    match call_save(&mut svc, file.to_str().unwrap(), "单章完成") {
        ToolReply::Ok(o) => {
            assert_eq!(o.result["file"], json!("c.md"));
        }
        ToolReply::Err { error, .. } => panic!("无标记根也应成功: {error:?}"),
    }
    assert!(ws.join("sub/.mang/history/index.json").is_file());
    let _ = std::fs::remove_dir_all(ws);
}

// ---------------------------------------------------------------- #374 围栏

/// 无围栏根（纯内存服务）→ 响亮拒（指路 BID_WS/BID_FS_ROOT——与 import
/// 的 #183 壳层围栏同款文案纪律）。
#[test]
fn save_version_requires_fence_root_loudly() {
    let ws = temp_ws("noroot");
    std::fs::create_dir_all(&ws).unwrap();
    let file = ws.join("a.md");
    std::fs::write(&file, "x").unwrap();
    let mut svc = BidService::new(); // 纯内存态：无 BID_WS/无 fs_root
    match call_save(&mut svc, file.to_str().unwrap(), "msg") {
        ToolReply::Err { error, .. } => {
            assert_eq!(error.code(), "E_UNPROCESSABLE");
            let msg = format!("{error:?}");
            assert!(msg.contains("BID_WS"), "报错须指路环境变量：{msg}");
        }
        ToolReply::Ok(_) => panic!("无围栏根不得成功（R-27：无根写盘口已收口）"),
    }
    let _ = std::fs::remove_dir_all(ws);
}

/// 围栏外路径 → 拒，且**不泄露存在性**（文件存在与否都是围栏错误——
/// 关闭 C-06 的文件存在性 oracle：E_NOTFOUND vs 成功可探测任意路径）。
#[test]
fn save_version_rejects_outside_fence_without_existence_oracle() {
    let ws = temp_ws("inside");
    let outside = temp_ws("outside");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    // 围栏外两个文件：一个存在、一个不存在——错误必须同码同面（无 oracle）
    std::fs::write(outside.join("secret.md"), "围栏外机密").unwrap();
    let mut svc = fenced_service(&ws);

    for f in ["secret.md", "nope.md"] {
        let p = outside.join(f);
        match call_save(&mut svc, p.to_str().unwrap(), "msg") {
            ToolReply::Err { error, .. } => {
                assert_eq!(
                    error.code(),
                    "E_UNPROCESSABLE",
                    "围栏外必须围栏错误（不是 E_NOTFOUND——存在性不泄露）"
                );
                let msg = format!("{error:?}");
                assert!(msg.contains("越界") || msg.contains("围栏"), "报错须含围栏语义：{msg}");
            }
            ToolReply::Ok(_) => panic!("围栏外路径不得成功（{f}）"),
        }
    }
    // 围栏外目录家族不得出现 .mang（未落任何快照）
    assert!(!outside.join(".mang").exists());
    let _ = std::fs::remove_dir_all(ws);
    let _ = std::fs::remove_dir_all(outside);
}

/// 祖先 `.mang` 越出围栏：围栏根 = ws/proj，文件在 proj 内，但 proj 的
/// 父目录（围栏外）有 `.mang` → find_root 会向上找到父级 → root 越界拒。
#[test]
fn save_version_rejects_root_escape_via_ancestor_mang() {
    let ws = temp_ws("esc");
    let proj = ws.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::create_dir_all(ws.join(".mang")).unwrap(); // 围栏外的祖先标记
    let file = proj.join("a.md");
    std::fs::write(&file, "内容").unwrap();
    let mut svc = fenced_service(&proj); // 围栏根 = ws/proj（BID_FS_ROOT 语义）
    match call_save(&mut svc, file.to_str().unwrap(), "msg") {
        ToolReply::Err { error, .. } => {
            assert_eq!(error.code(), "E_UNPROCESSABLE");
            let msg = format!("{error:?}");
            assert!(msg.contains("越界"), "报错须说明根发现越界：{msg}");
        }
        ToolReply::Ok(_) => panic!("祖先 .mang 不得牵引快照落到围栏外"),
    }
    // 围栏外祖先的 .mang/history 不得有新落盘
    assert!(!ws.join(".mang/history").exists());
    let _ = std::fs::remove_dir_all(ws);
}

/// 围栏内 symlink → 拒（fs_fence 逐段 symlink 检查——围栏内不允许
/// symlink，策略从严，与 import 同一真源）。
#[test]
fn save_version_rejects_symlink_inside_fence() {
    let ws = temp_ws("sym");
    let real_dir = ws.join("real");
    std::fs::create_dir_all(&real_dir).unwrap();
    std::fs::write(real_dir.join("a.md"), "真身").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_dir, ws.join("link")).unwrap();
    let mut svc = fenced_service(&ws);
    let linked = ws.join("link/a.md");
    #[cfg(unix)]
    {
        match call_save(&mut svc, linked.to_str().unwrap(), "msg") {
            ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_UNPROCESSABLE"),
            ToolReply::Ok(_) => panic!("symlink 段不得成功（围栏内不允许 symlink）"),
        }
    }
    #[cfg(not(unix))]
    let _ = linked;
    let _ = std::fs::remove_dir_all(ws);
}
