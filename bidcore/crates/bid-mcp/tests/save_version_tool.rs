//! save_version（issue #42 阶段 10）：AI 主动存版本。
//!
//! 验收面：
//!   * 正常路径：绝对路径 + message → ok=true，index 里 kind=ai、message 落盘，
//!     与桌面壳（mang-snap 单一真源）写同一份 `.mang/history`
//!   * 工作区根发现：向上找 `.mang`；找不到用文件所在目录
//!   * 拒绝面：相对路径 / 空 message / 非文本 / 不存在的文件 → 类型化错误（不空成功）
//!   * 断点幂等：同一内容再存 → 第二份（kind=ai 不去重，AI 明确要求存版本就存）

use bid_mcp::{dispatch, schema::ToolCall};
use serde_json::json;

fn call_save(file_path: &str, message: &str) -> bid_mcp::ToolReply {
    dispatch(ToolCall {
        tool: "save_version".to_string(),
        args: json!({ "file_path": file_path, "message": message }),
    })
}

fn read_index(ws: &std::path::Path) -> Vec<serde_json::Value> {
    let p = ws.join(".mang/history/index.json");
    let bytes = std::fs::read(p).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["items"].as_array().unwrap().clone()
}

#[test]
fn save_version_writes_ai_entry_into_mang_history() {
    let ws = std::env::temp_dir().join(format!(
        "mcp-sv-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(ws.join("docs")).unwrap();
    std::fs::create_dir_all(ws.join(".mang")).unwrap(); // 桌面壳建过的根标记
    let file = ws.join("docs/技术标.md");
    std::fs::write(&file, "# 技术标\n\n第1章 概述完成").unwrap();

    match call_save(file.to_str().unwrap(), "第1章 概述完成") {
        bid_mcp::ToolReply::Ok(o) => {
            assert_eq!(o.result["ok"], json!(true));
            assert_eq!(o.result["kind"], json!("ai"));
            assert_eq!(o.result["file"], json!("docs/技术标.md"));
            assert!(o.result["snapshot"].as_str().unwrap().contains('-'));
        }
        bid_mcp::ToolReply::Err { error, .. } => panic!("正常路径不得失败: {error:?}"),
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
    match call_save(file.to_str().unwrap(), "第2章 技术方案完成") {
        bid_mcp::ToolReply::Ok(_) => {}
        bid_mcp::ToolReply::Err { error, .. } => panic!("第二版不得失败: {error:?}"),
    }
    let items = read_index(&ws);
    assert_eq!(items.len(), 2);

    let _ = std::fs::remove_dir_all(ws);
}

#[test]
fn save_version_rejections_are_typed() {
    let ws = std::env::temp_dir().join(format!(
        "mcp-sv-rej-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("a.md"), "x").unwrap();
    std::fs::write(ws.join("b.xlsx"), "bin").unwrap();

    // 相对路径
    match call_save("a.md", "msg") {
        bid_mcp::ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_UNPROCESSABLE"),
        bid_mcp::ToolReply::Ok(_) => panic!("相对路径不得成功"),
    }
    // 空 message
    match call_save(ws.join("a.md").to_str().unwrap(), "  ") {
        bid_mcp::ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_UNPROCESSABLE"),
        bid_mcp::ToolReply::Ok(_) => panic!("空 message 不得成功"),
    }
    // 非文本
    match call_save(ws.join("b.xlsx").to_str().unwrap(), "msg") {
        bid_mcp::ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_UNPROCESSABLE"),
        bid_mcp::ToolReply::Ok(_) => panic!("二进制不得成功"),
    }
    // 不存在
    match call_save(ws.join("nope.md").to_str().unwrap(), "msg") {
        bid_mcp::ToolReply::Err { error, .. } => assert_eq!(error.code(), "E_NOTFOUND"),
        bid_mcp::ToolReply::Ok(_) => panic!("不存在的文件不得成功"),
    }
    let _ = std::fs::remove_dir_all(ws);
}

#[test]
fn save_version_discovers_root_without_mang_marker() {
    // 没有 .mang 标记：用文件所在目录做根（快照仍可用，只是不向上归并）
    let ws = std::env::temp_dir().join(format!(
        "mcp-sv-root-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(ws.join("sub")).unwrap();
    let file = ws.join("sub/c.md");
    std::fs::write(&file, "内容").unwrap();
    match call_save(file.to_str().unwrap(), "单章完成") {
        bid_mcp::ToolReply::Ok(o) => {
            assert_eq!(o.result["file"], json!("c.md"));
        }
        bid_mcp::ToolReply::Err { error, .. } => panic!("无标记根也应成功: {error:?}"),
    }
    assert!(ws.join("sub/.mang/history/index.json").is_file());
    let _ = std::fs::remove_dir_all(ws);
}
