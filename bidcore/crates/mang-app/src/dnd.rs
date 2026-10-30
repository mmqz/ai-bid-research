//! Windows 拖放：给窗口 HWND 装 WM_DROPFILES 子类过程。
//!
//! 为什么不用 wry 的 with_drag_drop_handler：
//!   wry 0.57 的实现是 RevokeDragDrop + RegisterDragDrop 接管 WebView 窗口的 IDropTarget，
//!   只在部分平台路径下可用，且和我们"整窗都能接文件"的需求不完全吻合；
//!   这里按需求用 SetWindowSubclass + DragAcceptFiles，拖进来推 IPC 事件。
//!
//! 事件格式：{ev:"drop", paths:[abs...], within:[{path,rel}], ts}
//!
//! 线程模型：WM_DROPFILES 回调在 UI 线程；闭包只做"入队 + 唤醒事件循环"，
//! 真正的 evaluate_script 由事件循环统一做（避免在消息处理里重入 WebView）。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::paths;

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::sync::Mutex;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Shell::{
        DefSubclassProc, DragAcceptFiles, DragFinish, DragQueryFileW, SetWindowSubclass, HDROP,
    };
    use windows::Win32::UI::WindowsAndMessaging::WM_DROPFILES;

    /// 拖放事件出口：由 main.rs 装进一个"把事件排进队列 + 唤醒"的闭包。
    /// Ex 只在 UI 线程用（不跨线程），Mutex 只是为了让 static 满足 Sync。
    pub type Emit = Box<dyn Fn(Vec<String>)>;
    /// SAFETY: 只在创建它的 UI 线程上被调用（WM_DROPFILES 走窗口消息），
    /// 所以 Box<dyn Fn> 不要求 Send+Sync；Mutex 仅为了拿到 Sync。
    struct EmitCell(Mutex<Emit>);
    unsafe impl Sync for EmitCell {}

    thread_local! {
        static EMIT: RefCell<Option<EmitCell>> = const { RefCell::new(None) };
    }

    const SUBCLASS_ID: usize = 0x4D41_4E47; // 'MANG'

    pub fn install(hwnd_raw: isize, emit: Emit) -> bool {
        EMIT.with(|slot| *slot.borrow_mut() = Some(EmitCell(Mutex::new(emit))));
        let hwnd = HWND(hwnd_raw as _);
        unsafe {
            DragAcceptFiles(hwnd, true);
            SetWindowSubclass(hwnd, Some(proc_), SUBCLASS_ID, 0).as_bool()
        }
    }

    unsafe extern "system" fn proc_(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_DROPFILES {
            let hdrop = HDROP(wparam.0 as _);
            let paths = unsafe { query_paths(hdrop) };
            // 必须 DragFinish，否则系统资源泄漏
            unsafe { DragFinish(hdrop) };
            if !paths.is_empty() {
                EMIT.with(|slot| {
                    if let Some(cell) = slot.borrow().as_ref() {
                        if let Ok(f) = cell.0.lock() {
                            f(paths);
                        }
                    }
                });
            }
            return LRESULT(0);
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    unsafe fn query_paths(hdrop: HDROP) -> Vec<String> {
        let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let len = unsafe { DragQueryFileW(hdrop, i, None) };
            if len == 0 {
                continue;
            }
            let mut buf = vec![0u16; len as usize + 1];
            let n = unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) };
            if n == 0 {
                continue;
            }
            buf.truncate(n as usize);
            if let Ok(s) = String::from_utf16(&buf) {
                out.push(s);
            }
        }
        out
    }
}

#[cfg(windows)]
pub use imp::install;

#[cfg(not(windows))]
pub fn install(_hwnd: isize, _emit: Box<dyn Fn(Vec<String>)>) -> bool {
    false
}

/// 文件列表 → drop 事件体
pub fn drop_event(paths_in: Vec<String>, ws_root: Option<&Path>) -> Value {
    let mut paths_out = Vec::new();
    let mut rels = Vec::new();
    for p in paths_in {
        let pb = PathBuf::from(&p);
        let disp = paths::display(&pb);
        if let Some(root) = ws_root {
            if let Some(rel) = rel_of_loose(&pb, root) {
                rels.push(json!({ "path": disp.clone(), "rel": rel }));
            }
        }
        paths_out.push(disp);
    }
    json!({
        "ev": "drop",
        "paths": paths_out,
        "within": rels,
        "ts": crate::ipc::now_ms(),
    })
}

/// 拖入文件若在工作区内，给出相对路径；在外面的就不给（UI 会把它当"外部文件"处理）
fn rel_of_loose(p: &Path, ws: &Path) -> Option<String> {
    let pc = std::fs::canonicalize(p).ok()?;
    let wc = std::fs::canonicalize(ws).ok()?;
    if paths::is_inside(&pc, &wc) {
        let rel = pc.strip_prefix(&wc).ok()?;
        Some(paths::rel_join(rel))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_event_shape() {
        let d = std::env::temp_dir().join(format!("mang-drop-{}", std::process::id()));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/x.md"), "x").unwrap();
        let f = d.join("sub/x.md");
        let ev = drop_event(vec![f.to_string_lossy().into_owned()], Some(&d));
        assert_eq!(ev["ev"], json!("drop"));
        assert_eq!(ev["within"][0]["rel"], json!("sub/x.md"));
        assert_eq!(ev["paths"].as_array().unwrap().len(), 1);
        // 越界文件：仍在 paths 里，但没有 rel
        let outside = std::env::temp_dir().join("mang-drop-outside.txt");
        std::fs::write(&outside, "o").unwrap();
        let ev = drop_event(vec![outside.to_string_lossy().into_owned()], Some(&d));
        assert_eq!(ev["paths"].as_array().unwrap().len(), 1);
        assert_eq!(ev["within"].as_array().unwrap().len(), 0);
        let _ = std::fs::remove_file(outside);
        let _ = std::fs::remove_dir_all(d);
    }
}
