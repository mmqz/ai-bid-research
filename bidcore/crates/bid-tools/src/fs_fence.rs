//! fs_fence.rs —— MCP 文件访问围栏（issue #183 导入面 + #189 导出面，共用单一实现）。
//!
//! 背景（#183/#189 安全审计）：
//! - **导入面**：`import_tender_files` 的 `upload_ref` 曾直接 `PathBuf::from` 进
//!   `sha256_file`/解析管线——MCP 调用者可把任意本地可访问文件喂进证据搜索/
//!   LLM 上下文（数据外泄入口）；
//! - **导出面**：旧 `validate_output_path` 拒 `..` 但绝对路径分支用字符串
//!   `starts_with` 比前缀（`c:/bid/work-evil` 命中 `c:/bid/work` 的前缀陷阱），
//!   且全链无符号链接检查——`out/link/x.docx`（link→外部目录 symlink）可一路
//!   畅通到 `File::create` + `rename`（任意写）。
//!
//! 围栏语义（导入导出同一套，不写两份）：
//! 1. **词法预拒**：用户路径任意分量出现 `..`（`Component::ParentDir`）→ 拒
//!    （先于一切磁盘操作——不存在的路径也不给试探机会）；
//! 2. **锚定**：相对路径以围栏根（`roots[0]`）拼接；绝对路径原样；
//! 3. **canonicalize**（lenient：目标不存在时 canonicalize 最近的存在祖先再拼回
//!    剩余段——`resolve_write` 的「目标文件尚不存在」是常态）；
//! 4. **组件级包含判定** [`is_inside`]：按路径组件逐段比较（`c:/bid/work-evil`
//!    不命中 `c:/bid/work`——修 #189 前缀陷阱）。Windows 大小写不敏感（组件
//!    lowercase 后比较 + 剥 `\\?\` verbatim 前缀，收敛在本模块单一 helper）；
//!    Unix 大小写敏感（文件系统语义如此——lowercase 会把 `/TMP` 误当 `/tmp`，
//!    反而开洞）；
//! 5. **逐段符号链接检查**：用户拼写路径（锚定后）在命中根以下的每一段做
//!    `symlink_metadata`——任何一段是 symlink（Windows reparse 点/junction 同
//!    命中）→ 拒。canonicalize 只能拦「链接指向围栏外」的逃逸；指向围栏内的
//!    symlink 会被解析洗白（`/ws/link/x` → `/ws/real/x`），逐段检查把这类
//!    间接引用也拒掉——围栏内不允许 symlink，策略从严；
//! 6. **限额**（导入面，[crate::service::BidService::import] 施加）：单文件
//!    [`MAX_FILE_BYTES`] / 单批件数 [`MAX_BATCH_FILES`] / 单批累计
//!    [`MAX_TOTAL_BYTES`]。
//!
//! **存在性不是围栏条件**：围栏只判「能不能碰这个路径」；「文件不存在/不可读」
//! 是业务失败（R-01：逐件失败登记、批次不中止，见 service.rs 头注与
//! `unreadable_upload_is_item_failure_without_record` 契约测试）——resolve_read
//! 对围栏内的缺失件返回 Ok（lenient 路径），交给下游 `sha256_file` 产出既有
//! 的 E_PARTIAL 语义。
//!
//! # TOCTOU 残留（登记，v1 = check-then-use）
//!
//! 本实现是 std-only 的「先检查后使用」：检查（canonicalize + symlink_metadata）
//! 与实际打开（`sha256_file` / `File::create`）之间存在竞态窗口——本地攻击者
//! 若能在窗口内把围栏内的目录段替换成 symlink，可绕过围栏。与 mang-app 现役
//! 围栏（`Workspace::resolve`）同语义同残留。升级路径（另行 issue，不在本
//! PR 边界）：Unix `openat` + `O_NOFOLLOW` 逐段打开（零新增依赖但属侵入
//! 重构，需改 bid-parse/engine 的文件打开面）；Windows `CreateFileW`
//! `FILE_FLAG_OPEN_REPARSE_POINT`。**零新增依赖**是本版硬约束。
//!
//! 借鉴来源：mang-app `src/paths.rs` 的 `Workspace{root}` + `resolve()`
//! （词法段处理 + canonicalize_lenient + is_inside 组件级比较）——独立
//! workspace 不能跨 crate 依赖，代码级借鉴进本模块（导入导出共用）。

use bid_domain::ToolError;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// 单文件导入上限（500MB——对齐 bid-parse guard.rs 的 PDF 面口径，统一到
/// 全格式：docx/xlsx 原先零大小门）。
pub const MAX_FILE_BYTES: u64 = 500 * 1024 * 1024;
/// 单批导入件数上限（64——最大真实批 33 件留余量）。
pub const MAX_BATCH_FILES: usize = 64;
/// 单批累计字节上限（2GB——资源耗尽面：解析产物全内存累积）。
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 围栏解析结果：命中根 + 根下相对段（校验期类型——不落盘、不进快照）。
#[derive(Debug, Clone)]
pub struct WorkspaceFileRef {
    /// 命中的围栏根（canonical；可能带 Windows `\\?\` verbatim 前缀，仅内部用）。
    root: PathBuf,
    /// canonical 候选路径（存在段已解析 symlink；缺失尾段保留用户拼写）。
    candidate: PathBuf,
}

impl WorkspaceFileRef {
    /// 围栏内绝对路径（`\\?\` 前缀剥除的显示形态——下游 sha256/解析/File::create
    /// 直接可用；Windows 对两种形态同语义）。
    #[must_use]
    pub fn absolute(&self) -> PathBuf {
        display_path(&self.candidate)
    }

    /// 命中的围栏根（显示形态）。
    #[must_use]
    pub fn root(&self) -> PathBuf {
        display_path(&self.root)
    }
}

fn unprocessable(reason: String) -> ToolError {
    ToolError::EUnprocessable { reason }
}

/// `\\?\C:\...` → `C:\...`（Windows canonicalize 返回 verbatim 前缀；比较与
/// 返回统一走本 helper，收敛前缀处理——Unix 上恒等）。
fn display_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
    PathBuf::from(stripped)
}

/// a 是否在 b 之内（**组件级**逐段比较——`c:/ws2` 不命中 `c:/ws`）。
///
/// Windows：组件 lowercase 后比较（大小写不敏感语义）；Unix：精确比较
/// （文件系统大小写敏感，lowercase 反而会把 `/TMP` 误当 `/tmp` 开洞）。
#[must_use]
pub fn is_inside(a: &Path, b: &Path) -> bool {
    strip_root(a, b).is_some()
}

/// 从 `a` 里剥掉根 `b`，返回 b 之下的相对段（组件级；None = a 不在 b 内）。
/// 大小写语义与 [`is_inside`] 同轨（Windows 不敏感 / Unix 敏感）。
fn strip_root(a: &Path, b: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let fold = |s: &OsStr| s.to_string_lossy().to_lowercase();
    #[cfg(not(windows))]
    let fold = |s: &OsStr| s.to_string_lossy().into_owned();
    let a_disp = display_path(a);
    let b_disp = display_path(b);
    let av: Vec<_> = a_disp.components().collect();
    let bv: Vec<_> = b_disp.components().collect();
    if bv.len() > av.len() || bv.is_empty() {
        return None;
    }
    // 前缀盘符/根分量不同（如 C: vs D:、/ vs C:\）→ 不在内
    for (x, y) in av.iter().zip(bv.iter()) {
        let (xs, ys) = (x.as_os_str(), y.as_os_str());
        if fold(xs) != fold(ys) {
            return None;
        }
    }
    Some(av[bv.len()..]
        .iter()
        .map(|c| c.as_os_str())
        .collect())
}

/// canonicalize；不存在时 canonicalize 最近的存在祖先再拼回剩余段（borrow：
/// mang-app `canonicalize_lenient`）。整个路径不存在时返回 None（词法归一化
/// 会丢掉 symlink 信息，围栏不猜测——交由调用方按「围栏外/不可判」拒）。
fn canonicalize_lenient(p: &Path) -> Option<PathBuf> {
    if let Ok(c) = std::fs::canonicalize(p) {
        return Some(c);
    }
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        let parent = cur.parent()?.to_path_buf();
        if let Some(name) = cur.file_name() {
            missing.push(name.to_os_string());
        }
        if let Ok(c) = std::fs::canonicalize(&parent) {
            let mut out = c;
            for seg in missing.iter().rev() {
                out.push(seg);
            }
            return Some(out);
        }
        if parent == cur {
            return None; // 走到根都没有存在祖先（挂载点缺失等）——不猜
        }
        cur = parent;
    }
}

/// 用户路径是否含 `..` 分量（词法预拒——先于一切磁盘操作）。
fn has_parent_dir(p: &Path) -> bool {
    p.components()
        .any(|c| matches!(c, Component::ParentDir))
}

/// 逐段符号链接检查：`segments` 是用户拼写路径在命中根 `root` 之下的相对段，
/// 逐前缀 `symlink_metadata`（不跟随链接——symlink/reparse 点/junction 都命中）。
/// 段不存在（write 的未建目标叶）→ 跳过；只有**确认是 symlink** 才拒。
fn ensure_no_symlink_segs(root: &Path, segments: &Path, display: &str) -> Result<(), ToolError> {
    let mut acc = root.to_path_buf();
    for seg in segments.components() {
        match seg {
            Component::Normal(s) => {
                acc.push(s);
                match std::fs::symlink_metadata(&acc) {
                    Ok(md) if md.file_type().is_symlink() => {
                        return Err(unprocessable(format!(
                            "路径含符号链接段（围栏内不允许 symlink——Windows reparse 点同拒）: {display} → {}",
                            display_path(&acc).display()
                        )));
                    }
                    _ => {}
                }
            }
            Component::CurDir => {}
            _ => {} // 相对段内不会出现（调用方已过滤），防御面静默
        }
    }
    Ok(())
}

/// 围栏解析核心（read/write 共用——#189 指令：导入导出同一套实现）。
///
/// 步骤：词法拒 `..` → 锚定（相对 = roots[0] 拼 / 绝对原样）→ lenient
/// canonicalize → 命中根判定（组件级）→ 用户拼写段的逐段 symlink 检查。
fn resolve(
    roots: &[PathBuf],
    user_path: &Path,
    verb: &str,
) -> Result<WorkspaceFileRef, ToolError> {
    if roots.is_empty() {
        return Err(unprocessable(format!(
            "{verb}需要工作区（设 BID_WS 环境变量，或用 BidService::open 打开持久化工作区，或 with_fs_root 声明受信根）"
        )));
    }
    if has_parent_dir(user_path) {
        return Err(unprocessable(format!(
            "{verb}路径不得包含父级逃逸 `..`（实际：{}）",
            user_path.display()
        )));
    }
    let joined: PathBuf = if user_path.is_absolute() {
        user_path.to_path_buf()
    } else {
        roots[0].join(user_path)
    };
    let Some(candidate) = canonicalize_lenient(&joined) else {
        return Err(unprocessable(format!(
            "{verb}路径不可解析（无存在祖先，围栏无法判定）：{}",
            user_path.display()
        )));
    };
    // 命中根：候选（canonical）落在任一受信根内
    let Some(root) = roots
        .iter()
        .find(|r| is_inside(&candidate, r))
        .cloned()
    else {
        return Err(unprocessable(format!(
            "{verb}路径越界（不在任何受信工作区内：{}；实际：{}）",
            roots
                .iter()
                .map(|r| display_path(r).display().to_string())
                .collect::<Vec<_>>()
                .join(" | "),
            display_path(&candidate).display()
        )));
    };
    // 逐段 symlink 检查：优先查「用户拼写的段」（canonicalize 会把指向围栏内
    // 的 symlink 洗白——/ws/link/x → /ws/real/x，只有用户拼写面能看到 link）；
    // 用户拼写经围栏外 symlink 间接进来（strip 不中）时退回查 canonical 段
    // （存在段已无 symlink，缺失尾段照查——防「叶是链接」的写面）。
    let user_rel = strip_root(&joined, &root)
        .or_else(|| strip_root(&candidate, &root))
        .unwrap_or_default();
    ensure_no_symlink_segs(&root, &user_rel, &user_path.display().to_string())?;
    Ok(WorkspaceFileRef { root, candidate })
}

/// 读面解析（导入 upload_ref）：围栏内即可——文件不存在/不可读不是围栏条件
/// （R-01 逐件失败语义在下游 sha256_file，见模块头）。
pub fn resolve_read(roots: &[PathBuf], user_path: &Path) -> Result<WorkspaceFileRef, ToolError> {
    resolve(roots, user_path, "导入")
}

/// 写面解析（导出 output_path）：目标文件可以尚不存在（lenient canonicalize
/// 最近存在祖先 + 拼回剩余段）；越界/`..`/symlink 段 → 拒（#189：`out/link/x.docx`
/// 的 link→外部目录在此拦下，到不了 export.rs 的 File::create+rename）。
pub fn resolve_write(roots: &[PathBuf], user_path: &Path) -> Result<WorkspaceFileRef, ToolError> {
    resolve(roots, user_path, "导出")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_inside_is_component_wise_not_string_prefix() {
        // 前缀陷阱（#189）：c:/bid/work-evil 不得命中 c:/bid/work
        assert!(!is_inside(
            Path::new("c:/bid/work-evil/x.docx"),
            Path::new("c:/bid/work")
        ));
        assert!(is_inside(
            Path::new("c:/bid/work/out/x.docx"),
            Path::new("c:/bid/work")
        ));
        assert!(is_inside(Path::new("/a/b"), Path::new("/a")));
        assert!(!is_inside(Path::new("/a"), Path::new("/a/b")));
        assert!(is_inside(Path::new("/a"), Path::new("/a")));
        // Windows verbatim 前缀不干扰组件比较（仅 Windows 有该形态——
        // canonicalize 返回 `\\?\C:\...`；Unix 上反斜杠是普通文件名字符，
        // 该断言无意义）
        #[cfg(windows)]
        assert!(is_inside(
            Path::new(r"\\?\C:\ws\x.docx"),
            Path::new("C:\\ws")
        ));
    }
}
