//! 静态服务层：mangapp://（UI bundle）+ mangfs://（工作区文件只读）。
//!
//! 平台中立设计：本模块只产出 `(status, mime, body)` 三元组——不依赖
//! wry/http 类型（纯层全环境可测）；main.rs 的 gui 面负责把三元组适配成
//! wry::http::Response。
//!
//! 安全面（两协议共用一个把关口）：
//! - **目录穿越**：任何 `..` 段（含 percent 编码展开后的）→ 404；
//! - **绝对锚定**：所有路径拼在各自 root 下（mangfs 的 root = 工作区）；
//! - **读失败/不存在** → 404（不泄露存在性差异）；
//! - **mangfs 白名单扩展**：只服务文档类扩展（docx/md/xlsx/pdf 等）——
//!   工作区里的 .exe/.dll/.mang 内部文件不经 web 面（攻击面收敛）。

use std::path::{Path, PathBuf};

/// 平台中立响应。
#[derive(Debug, Clone, PartialEq)]
pub struct RawResponse {
    pub status: u16,
    pub mime: &'static str,
    pub body: Vec<u8>,
}

impl RawResponse {
    fn not_found() -> Self {
        RawResponse {
            status: 404,
            mime: "text/plain; charset=utf-8",
            body: b"not found".to_vec(),
        }
    }
}

/// mangfs:// 服务扩展白名单（文档工作面）。
pub const FS_ALLOWED_EXTS: &[&str] = &[
    "md", "markdown", "txt", "docx", "doc", "xlsx", "xls", "csv", "pdf", "png", "jpg", "jpeg",
    "gif", "svg", "json",
];

/// mangfs 面不服务的内部目录（自家元数据 + scm）。
const FS_HIDDEN_ROOTS: &[&str] = &[".mang", ".git"];

/// 路径卫生检查：拒绝任何 `..` 段（URL 路径已 percent 解码后判定）。
fn is_safe_rel(raw_path: &str) -> Option<String> {
    let decoded = crate::ipc::percent_decode(raw_path);
    let rel = decoded.trim_start_matches('/');
    if rel.is_empty() {
        return Some(String::new()); // 根——由调用方决定默认页
    }
    let segs: Vec<&str> = rel.split('/').collect();
    if segs.contains(&"..") {
        return None;
    }
    // 空段（`a//b`）无害（join 归一），首段内部目录挡在 serve_fs 白名单外
    Some(rel.to_string())
}

/// mangapp:// —— UI bundle 静态服务（root = web-ui/dist）。
///
/// `/` 或空 → index.html；其余按文件直读。无缓存（每请求读盘——桌面
/// 本地盘，简单优先）。
pub fn serve_app(root: &Path, raw_path: &str) -> RawResponse {
    let Some(rel) = is_safe_rel(raw_path) else {
        return RawResponse::not_found();
    };
    let rel = if rel.is_empty() {
        "index.html".to_string()
    } else {
        rel
    };
    serve_file(&root.join(rel))
}

/// mangfs:// —— 工作区文件只读服务（root = 工作区目录）。
///
/// 扩展白名单门 + 穿越门；`/` → 404（工作区没有「首页」概念）。
pub fn serve_fs(root: &Path, raw_path: &str) -> RawResponse {
    let Some(rel) = is_safe_rel(raw_path) else {
        return RawResponse::not_found();
    };
    if rel.is_empty() {
        return RawResponse::not_found();
    }
    // 内部目录门（.mang/.git 首段——元数据不经 web 面）
    let first = rel.split('/').next().unwrap_or("");
    if FS_HIDDEN_ROOTS.contains(&first) {
        return RawResponse::not_found();
    }
    // 白名单门（扩展小写化判定）
    let ext = rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if !FS_ALLOWED_EXTS.contains(&ext.as_str()) {
        return RawResponse::not_found();
    }
    serve_file(&root.join(&rel))
}

fn serve_file(full: &PathBuf) -> RawResponse {
    match std::fs::read(full) {
        Ok(bytes) => RawResponse {
            status: 200,
            mime: mime_of(&full.to_string_lossy()),
            body: bytes,
        },
        Err(_) => RawResponse::not_found(),
    }
}

/// 拒绝软链接逃逸（mangfs 面：工作区里的链接指到外面 → 当作不存在）。
///
/// 保守近似：逐段 canonicalize 已存在前缀，验证仍在 root 下。Windows 同样
/// 适用（canonicalize 是跨平台 API；`\??\` 前缀差异不影响 starts_with 判定
/// ——两侧同经 canonicalize）。
pub fn resolves_within(root: &Path, rel: &str) -> bool {
    let mut cur = root.to_path_buf();
    for seg in rel.split('/') {
        if seg.is_empty() {
            continue;
        }
        cur = cur.join(seg);
        if let Ok(c) = cur.canonicalize() {
            let Ok(r) = root.canonicalize() else {
                return false;
            };
            if !c.starts_with(&r) {
                return false;
            }
        }
    }
    true
}

/// mime 推断（与 mdcore mdshell-native serve_dist 同表——成熟面复用）。
pub fn mime_of(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "md" | "markdown" | "txt" => "text/plain; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "pdf" => "application/pdf",
        "docx" | "doc" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" | "xls" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn bundle() -> (PathBuf, PathBuf) {
        let d = std::env::temp_dir().join(format!("mang_proto_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("pkg")).unwrap();
        fs::write(d.join("index.html"), b"<html>ok</html>").unwrap();
        fs::write(d.join("main.js"), b"console.log(1)").unwrap();
        fs::write(d.join("pkg/a.wasm"), b"\0asm").unwrap();
        (d.clone(), d)
    }

    #[test]
    fn serve_app_default_and_files() {
        let (root, _) = bundle();
        let r = serve_app(&root, "/");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html; charset=utf-8");
        assert_eq!(r.body, b"<html>ok</html>");
        let r = serve_app(&root, "/main.js");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/javascript; charset=utf-8");
        let r = serve_app(&root, "/pkg/a.wasm");
        assert_eq!(r.mime, "application/wasm");
        assert_eq!(r.body, b"\0asm");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn serve_app_traversal_rejected() {
        let (root, _) = bundle();
        for p in [
            "/../etc/passwd",
            "/..%2F..%2Fetc/passwd",
            "/pkg/../../x",
            "/a/..",
        ] {
            assert_eq!(serve_app(&root, p).status, 404, "path {p} 必须 404");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn serve_fs_whitelist_and_root() {
        let (ws, tmp) = bundle();
        fs::write(ws.join("招标文件.docx"), b"docx-bytes").unwrap();
        fs::write(ws.join("技术标.md"), b"# md").unwrap();
        fs::write(ws.join("evil.exe"), b"MZ").unwrap();
        fs::create_dir_all(ws.join(".mang")).unwrap();
        fs::write(ws.join(".mang/recent.json"), b"{}").unwrap();

        let r = serve_fs(&ws, "/招标文件.docx");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"docx-bytes");
        // percent 编码形（fetch 端 encodeURIComponent 产物）
        let enc = crate::ipc::percent_encode("招标文件.docx");
        let r = serve_fs(&ws, &format!("/{enc}"));
        assert_eq!(r.status, 200, "编码形等价");

        let r = serve_fs(&ws, "/技术标.md");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/plain; charset=utf-8");

        // 白名单外（exe）→ 404；.mang 内部文件 → 404（json 虽在白名单，
        // 但 .mang 是内部目录——显式挡在 web 面外）
        assert_eq!(serve_fs(&ws, "/evil.exe").status, 404, "白名单外 404");
        assert_eq!(
            serve_fs(&ws, "/.mang/recent.json").status,
            404,
            ".mang 内部 404"
        );
        assert_eq!(serve_fs(&ws, "/.git/config").status, 404, ".git 内部 404");
        assert_eq!(serve_fs(&ws, "/").status, 404, "根无默认页");
        assert_eq!(serve_fs(&ws, "/no/such/file.md").status, 404);
        let _ = fs::remove_dir_all(&tmp);
    }
}
