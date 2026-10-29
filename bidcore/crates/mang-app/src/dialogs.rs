//! dialog.openFile / dialog.openFolder —— rfd 同步 Win32 对话框。
//!
//! **必须在 UI 主线程调用**（WebView2 的 IPC handler 跑在消息循环里，与 JS 同线程；
//! 模态对话框会内嵌消息泵，HTML 模态框因此正常工作）。
//! 参数：{filters?:[{"name":"Word","extensions":["docx","md"]}], title?, multiple?:bool, directory?}
//! 返回：{paths:[abspath...], canceled:bool}
//!
//! **风险边界**：模态对话框会阻塞整个 UI 线程（这是 Win32 的既有事实，浏览器模态也是同样表现）。
//! 为了不让"对话框卡住"把壳拖死，做了两件事：
//!   1. 只有主线程的对话框队列会调用本模块；IPC 线程永远不会阻塞。
//!   2. 环境变量 `MANG_DIALOG_MOCK=1` 时不弹窗、直接回 canceled —— 无头自检/CI 用。
//! 另外 rfd 在 Win32 上走 IFileOpenDialog，用户按 Esc/取消都会正常返回 None。

use rfd::FileDialog;
use serde_json::{json, Map, Value};

use crate::ipc::IpcError;
use crate::paths;

/// 无头模式：MANG_DIALOG_MOCK=1 时不弹真窗，直接返回取消
fn mock_mode() -> bool {
    matches!(
        std::env::var("MANG_DIALOG_MOCK").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn parse_filters(a: &Map<String, Value>) -> Result<Vec<(String, Vec<String>)>, IpcError> {
    let Some(v) = a.get("filters") else {
        return Ok(Vec::new());
    };
    let Some(arr) = v.as_array() else {
        return Err(IpcError::bad_args("filters 必须是数组"));
    };
    let mut out = Vec::new();
    for f in arr {
        let Some(o) = f.as_object() else {
            return Err(IpcError::bad_args("filters[] 必须是对象"));
        };
        let name = o
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("文件")
            .to_string();
        let ext: Vec<String> = match o.get("extensions") {
            Some(Value::Array(e)) => e
                .iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim_start_matches('.').to_string())
                .filter(|s| !s.is_empty() && s != "*")
                .collect(),
            Some(Value::String(s)) => s
                .split([',', ' '])
                .map(|x| x.trim().trim_start_matches('.').to_string())
                .filter(|x| !x.is_empty() && x != "*")
                .collect(),
            _ => Vec::new(),
        };
        if !ext.is_empty() {
            out.push((name, ext));
        }
    }
    Ok(out)
}

/// dialog.openFile → {paths, canceled}
pub fn open_file(args: &Map<String, Value>) -> Result<Value, IpcError> {
    let filters = parse_filters(args)?;
    if mock_mode() {
        return Ok(json!({ "paths": [], "canceled": true, "mock": true }));
    }
    let multiple = args
        .get("multiple")
        .and_then(|m| m.as_bool())
        .unwrap_or(true);
    let title = args
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("打开文件");
    let mut dlg = FileDialog::new().set_title(title);
    for (name, ext) in &filters {
        dlg = dlg.add_filter(name.as_str(), ext.as_slice());
    }
    if args.get("directory").and_then(|d| d.as_str()).is_some() {
        if let Some(dir) = args.get("directory").and_then(|d| d.as_str()) {
            if std::path::Path::new(dir).is_dir() {
                dlg = dlg.set_directory(dir);
            }
        }
    }
    let picked: Vec<std::path::PathBuf> = if multiple {
        dlg.pick_files().unwrap_or_default()
    } else {
        dlg.pick_file().into_iter().collect()
    };
    Ok(paths_value(picked))
}

/// dialog.openFolder → {paths, canceled}
pub fn open_folder(args: &Map<String, Value>) -> Result<Value, IpcError> {
    if mock_mode() {
        return Ok(json!({ "paths": [], "canceled": true, "mock": true }));
    }
    let title = args
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("打开文件夹");
    let mut dlg = FileDialog::new().set_title(title);
    if let Some(dir) = args.get("directory").and_then(|d| d.as_str()) {
        if std::path::Path::new(dir).is_dir() {
            dlg = dlg.set_directory(dir);
        }
    }
    let picked: Vec<std::path::PathBuf> = dlg.pick_folder().into_iter().collect();
    Ok(paths_value(picked))
}

fn paths_value(picked: Vec<std::path::PathBuf>) -> Value {
    let paths: Vec<String> = picked.iter().map(|p| paths::display(p)).collect();
    let canceled = paths.is_empty();
    // 相对路径由 JS 侧调用方按 app.info.workspace 自己算（壳不知道谁是工作区时也别瞎猜）
    json!({ "paths": paths, "canceled": canceled })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_mode_never_blocks() {
        // 该单测会在 MANG_DIALOG_MOCK 下跑：不弹窗、立即返回 canceled
        std::env::set_var("MANG_DIALOG_MOCK", "1");
        let m = serde_json::Map::new();
        let f = open_file(&m).unwrap();
        assert_eq!(f["canceled"], serde_json::json!(true));
        assert_eq!(f["paths"].as_array().unwrap().len(), 0);
        let d = open_folder(&m).unwrap();
        assert_eq!(d["canceled"], serde_json::json!(true));
        std::env::remove_var("MANG_DIALOG_MOCK");
    }

    #[test]
    fn filter_parsing() {
        let m = json!({
            "filters": [
                {"name":"Word","extensions":["docx",".md"]},
                {"name":"All","extensions":["*"]},
                {"name":"X","extensions":"txt,csv"}
            ]
        });
        let f = parse_filters(m.as_object().unwrap()).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].0, "Word");
        assert_eq!(f[0].1, vec!["docx", "md"]);
        assert_eq!(f[1].1, vec!["txt", "csv"]);
        let e = parse_filters(json!({"filters":"nope"}).as_object().unwrap()).unwrap_err();
        assert_eq!(e.code, "bad_args");
    }
}
