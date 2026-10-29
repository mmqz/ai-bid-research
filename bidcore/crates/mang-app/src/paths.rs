//! 路径解析 + 工作区围栏（防 ../ 穿越）。
//!
//! 规则（IPC-CONTRACT §5）：
//!   1. 相对路径 → 以工作区根为基准
//!   2. 绝对路径 → 原样使用
//!   3. canonicalize（文件不存在时 canonicalize 最近的存在祖先，再拼回剩余段）
//!   4. 结果必须位于 canonical 工作区根之下，否则 `outside_workspace`
//!
//! 刻意用「路径组件」比较而不是字符串前缀，避免 `C:\ws2` 被 `C:\ws` 误判为子目录。

use std::path::{Component, Path, PathBuf};

use crate::ipc::IpcError;

/// 拖拽/对话框返回的路径统一成 `\\?\C:\...` → `C:\...`（UI 里更好看）。
pub fn display(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        // UNC：\\?\UNC\server\share → \\server\share
        if let Some(unc) = rest.strip_prefix("UNC\\") {
            return format!("\\\\{unc}");
        }
        return rest.to_string();
    }
    s.into_owned()
}

fn io_err(p: &Path, e: &std::io::Error) -> IpcError {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => "not_found",
        std::io::ErrorKind::PermissionDenied => "denied",
        _ => "io",
    };
    IpcError::new(code, format!("{}: {e}", display(p)))
}

/// canonicalize；不存在时 canonicalize 最近的存在祖先再拼回剩余组件。
/// `missing_ok` 决定"整条路径不存在"时是否报 not_found。
pub fn canonicalize_lenient(p: &Path, missing_ok: bool) -> Result<PathBuf, IpcError> {
    if let Ok(c) = std::fs::canonicalize(p) {
        return Ok(c);
    }
    // 找最近的存在祖先
    let mut missing: Vec<&std::ffi::OsStr> = Vec::new();
    let mut cur = p;
    loop {
        match cur.parent() {
            Some(parent) => {
                if let Some(name) = cur.file_name() {
                    missing.push(name);
                }
                if let Ok(c) = std::fs::canonicalize(parent) {
                    let mut out = c;
                    for seg in missing.iter().rev() {
                        out.push(seg);
                    }
                    return Ok(out);
                }
                cur = parent;
            }
            None => {
                if missing_ok {
                    // 没有任何存在祖先（Windows 下几乎不会发生）——做一次纯词法规范化
                    return Ok(lexical_normalize(p));
                }
                return Err(IpcError::new(
                    "not_found",
                    format!("{}: 路径不存在", display(p)),
                ));
            }
        }
    }
}

/// 纯词法规范化（不碰磁盘）：消掉 `.`、`..`。
fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 工作区围栏
#[derive(Debug, Clone)]
pub struct Workspace {
    /// canonical 绝对路径（不带 \\?\ 前缀的显示形态）
    root: PathBuf,
}

impl Workspace {
    pub fn new(p: &Path) -> Result<Self, IpcError> {
        let c = std::fs::canonicalize(p)
            .map_err(|e| IpcError::new("not_found", format!("工作区不存在：{}: {e}", display(p))))?;
        if !c.is_dir() {
            return Err(IpcError::new(
                "bad_args",
                format!("工作区不是目录：{}", display(&c)),
            ));
        }
        Ok(Self { root: c })
    }

    /// canonical 根（可能是 \\?\C:\... 形态，仅用于比较）
    pub fn root_canonical(&self) -> &Path {
        &self.root
    }

    /// 给 UI 看的根路径
    pub fn root_display(&self) -> String {
        display(&self.root)
    }

    /// 工作区下的子目录（用于 .mang、.mang/history 等）
    pub fn sub(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// 把请求里的 path 解析成**已 fence 的 canonical 绝对路径**。
    ///
    /// `allow_missing`：目标不必存在（新建文件场景）。
    pub fn resolve(&self, raw: &str, allow_missing: bool) -> Result<PathBuf, IpcError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(self.root.clone());
        }
        // 允许前端的 "a/b/c" 带斜杠；Windows 也接受 '/'
        let joined: PathBuf = if raw.starts_with("\\\\") || Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            let mut acc = self.root.clone();
            for seg in raw.split(['/', '\\']) {
                if seg.is_empty() || seg == "." {
                    continue;
                }
                if seg == ".." {
                    if !acc.pop() {
                        return Err(IpcError::new(
                            "outside_workspace",
                            format!("路径越界：{raw}"),
                        ));
                    }
                } else {
                    acc.push(seg);
                }
            }
            acc
        };

        let canon = canonicalize_lenient(&joined, allow_missing)?;
        if !is_inside(&canon, &self.root) {
            return Err(IpcError::new(
                "outside_workspace",
                format!(
                    "路径越界：{raw} → {} 不在工作区 {} 内",
                    display(&canon),
                    display(&self.root)
                ),
            ));
        }
        Ok(canon)
    }

    /// 把任意绝对路径（对话框/拖拽来的）转成"相对工作区的 rel 路径"；不在工作区内返回 None。
    pub fn rel_of(&self, p: &Path) -> Option<String> {
        let c = std::fs::canonicalize(p).ok()?;
        if !is_inside(&c, &self.root) {
            return None;
        }
        let rel = c.strip_prefix(&self.root).ok()?;
        Some(rel_join(rel))
    }

    /// 绝对路径 → 相对工作区的 rel（允许文件已不存在；越界返回 Err）。
    pub fn rel(&self, raw: &str) -> Result<String, IpcError> {
        let p = std::path::Path::new(raw);
        let cand = if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) };
        let target = std::fs::canonicalize(&cand).or_else(|_| {
            let parent = cand.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| cand.clone());
            let name = cand.file_name().map(|s| s.to_os_string());
            match (std::fs::canonicalize(parent), name) {
                (Ok(d), Some(n)) => Ok(d.join(n)),
                _ => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "not found")),
            }
        });
        let c = target.map_err(|e| IpcError::io(format!("路径不存在：{raw} ({e})")))?;
        if !is_inside(&c, &self.root) {
            return Err(IpcError::denied(format!("路径越界：{raw}")));
        }
        let stripped = c.strip_prefix(&self.root).map_err(|_| IpcError::denied("路径越界"))?;
        Ok(rel_join(stripped))
    }

    /// 传 rel 是否在工作区围栏内（只做词法判断，用于早筛）
    #[allow(dead_code)]
    pub fn rel_is_sane(&self, raw: &str) -> bool {
        !raw.split(['/', '\\']).any(|s| s == "..")
    }
}

/// rel 路径统一用 '/' 分隔（Windows 上也这样给前端）
pub fn rel_join(p: &Path) -> String {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// a 是否在 b 之内（按路径组件比较，大小写不敏感——Windows 语义）
pub fn is_inside(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    let av = norm(a);
    let bv = norm(b);
    if bv.len() > av.len() {
        return false;
    }
    av[..bv.len()] == bv[..]
}

/// 读路径时的 IO 错误 → typed error
pub fn map_io(p: &Path, e: &std::io::Error) -> IpcError {
    io_err(p, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpws(tag: &str) -> (PathBuf, Workspace) {
        let dir = std::env::temp_dir().join(format!(
            "mang-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.md"), "hi").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        let ws = Workspace::new(&dir).unwrap();
        (dir, ws)
    }

    #[test]
    fn resolves_inside() {
        let (dir, ws) = tmpws("inside");
        let p = ws.resolve("sub/a.md", false).unwrap();
        assert!(p.ends_with("a.md"));
        let p2 = ws.resolve("sub\\a.md", false).unwrap();
        assert_eq!(p, p2);
        let p3 = ws.resolve("b.txt", false).unwrap();
        assert!(p3.ends_with("b.txt"));
        assert_eq!(ws.rel_of(&p3).as_deref(), Some("b.txt"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn blocks_traversal() {
        let (dir, ws) = tmpws("escape");
        let e = ws.resolve("../x.txt", false).unwrap_err();
        assert_eq!(e.code, "outside_workspace");
        let e = ws.resolve("..\\..\\Windows\\win.ini", false).unwrap_err();
        assert_eq!(e.code, "outside_workspace");
        // 绝对路径越界同样拦
        let outside = std::env::temp_dir().join("mang-guard-outside.txt");
        let e = ws.resolve(&outside.to_string_lossy(), true).unwrap_err();
        assert_eq!(e.code, "outside_workspace");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sibling_prefix_not_inside() {
        // C:\ws2 不能被当成 C:\ws 的子目录
        assert!(!is_inside(
            Path::new("C:\\ws2\\x"),
            Path::new("C:\\ws")
        ));
        assert!(is_inside(Path::new("C:\\WS\\x"), Path::new("C:\\ws")));
    }

    #[test]
    fn allows_missing_leaf() {
        let (dir, ws) = tmpws("missing");
        let p = ws.resolve("new/dir/file.md", true).unwrap();
        assert!(p.ends_with("file.md"));
        let _ = std::fs::remove_dir_all(dir);
    }
}