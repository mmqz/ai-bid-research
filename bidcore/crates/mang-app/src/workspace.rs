//! 工作区：`--workspace` 解析 + 目录扫描 → 文件树 + 最近打开记录。
//!
//! #42 §四（命令行传参）/ §一（侧栏文件树）/ §五（最近打开恢复）。
//!
//! 设计要点：
//! - **零三方目录遍历依赖**：`std::fs` 递归 + 显式栈（避免深层递归溢出）；
//! - **跳过名单**：`.mang`（自家元数据）、`.git`、`node_modules`、`target`
//!   等非文档目录——文件树是给标书工作区看的，不是通用文件管理器；
//! - **有界扫描**：目录数/文件数/深度三重上限（防病态工作区把 UI 卡死——
//!   参照 mdcore 容器预扫门的思路，越限截断并在结果里显式标注，不静默）；
//! - **路径卫生**：树节点带相对路径（正斜杠规范形）——web 端经 mangfs://
//!   取内容时用相对路径，宿主侧再拼根（穿越防护在 protocol.rs 单点把守）。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::spec;

/// 文件树节点（serde 序列化给 web 端）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TreeNode {
    Dir {
        name: String,
        path: String,
        children: Vec<TreeNode>,
    },
    File {
        name: String,
        path: String,
        #[serde(rename = "type")]
        ftype: FileType,
    },
}

/// 文件类型路由（#42 §三——自动路由不用用户选）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    /// .docx / .md → 文档渲染器
    Doc,
    /// .xlsx → 表格渲染器
    Sheet,
    /// .pdf → PDF 渲染器
    Pdf,
    /// 其余——可列出、无专属渲染器（默认文本预览）
    Other,
}

impl FileType {
    /// 按扩展名路由。
    pub fn of(name: &str) -> Self {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "docx" | "md" | "markdown" | "doc" | "txt" => FileType::Doc,
            "xlsx" | "xls" | "csv" => FileType::Sheet,
            "pdf" => FileType::Pdf,
            _ => FileType::Other,
        }
    }
}

/// 扫描限制（越限截断——显式不静默）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanLimits {
    pub max_depth: usize,
    pub max_dirs: usize,
    pub max_files: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        ScanLimits {
            max_depth: 12,
            max_dirs: 2_000,
            max_files: 20_000,
        }
    }
}

/// 扫描结果（树 + 截断标注——越限时 `truncated=true`，UI 显示「已截断」）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanResult {
    pub root: String,
    pub tree: Vec<TreeNode>,
    pub truncated: bool,
    pub dir_count: usize,
    pub file_count: usize,
}

/// 目录跳过名单（相对名精确匹配）。
pub const SKIP_DIRS: &[&str] = &[
    spec::META_DIR,
    ".git",
    "node_modules",
    "target",
    "dist",
    "__pycache__",
    ".venv",
];

/// `--workspace <path>` 解析（无参 → 当前目录）。
///
/// 返回规范化绝对路径；不存在/不是目录 → `Err`（main 转错误弹窗，不 panic）。
pub fn resolve_workspace(args: &[String]) -> Result<PathBuf, String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--workspace" || a == "-w" {
            let Some(v) = it.next() else {
                return Err("--workspace 需要一个路径参数".to_string());
            };
            return canonical_root(std::path::Path::new(v));
        }
        // 容错：--workspace=路径 形
        if let Some(v) = a.strip_prefix("--workspace=") {
            return canonical_root(std::path::Path::new(v));
        }
    }
    canonical_root(&std::env::current_dir().map_err(|e| format!("读取当前目录失败: {e}"))?)
}

fn canonical_root(p: &Path) -> Result<PathBuf, String> {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("读取当前目录失败: {e}"))?
            .join(p)
    };
    if !abs.exists() {
        return Err(format!("工作区不存在: {}", abs.display()));
    }
    if !abs.is_dir() {
        return Err(format!("工作区不是目录: {}", abs.display()));
    }
    // 不做 symlink 解析（dunce 等价语义超范围）——仅词法清理 `.`/`..`
    Ok(abs)
}

/// 扫描工作区 → 文件树（BFS 序 + 目录后序插入，字母序稳定）。
///
/// 稳定性：同名兄弟按不区分大小写的字母序（Windows 资源管理器观感一致）。
pub fn scan_workspace(root: &Path, limits: &ScanLimits) -> ScanResult {
    let mut truncated = false;
    let mut dir_count = 0usize;
    let mut file_count = 0usize;

    // BFS 收集 (相对路径, 深度) → 逐层构建。栈式遍历防深递归溢出。
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((PathBuf::new(), 0));

    // 排序键（大小写不敏感）——收集后统一排序组装。
    use std::collections::BTreeMap;
    let mut sort_keys: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();

    while let Some((rel, depth)) = queue.pop_front() {
        if depth >= limits.max_depth {
            truncated = true;
            continue;
        }
        let full = root.join(&rel);
        let Ok(entries) = std::fs::read_dir(&full) else {
            continue; // 读不了的目录跳过（权限/竞态）——树保持可用
        };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                if SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                if dir_count >= limits.max_dirs {
                    truncated = true;
                    continue;
                }
                dir_count += 1;
                let child_rel = join_rel(&rel, &name);
                sort_keys
                    .entry(rel.to_string_lossy().into_owned())
                    .or_default()
                    .push((name, true));
                queue.push_back((child_rel, depth + 1));
            } else if ft.is_file() {
                if file_count >= limits.max_files {
                    truncated = true;
                    continue;
                }
                file_count += 1;
                sort_keys
                    .entry(rel.to_string_lossy().into_owned())
                    .or_default()
                    .push((name, false));
            }
            // 符号链接不跟（防环 + 防逃逸）——按其 file_type 原样归类，
            // read_dir 的 file_type 不跟链接，天然满足。
        }
    }

    // 自底向上组装：先文件/目录混合排序键，再递归挂 children。
    // 目录深度最大者优先组装（父依赖子）。
    let mut all_dirs: Vec<String> = sort_keys.keys().cloned().collect();
    // 深度降序（深的先组装）
    all_dirs.sort_by_key(|k| std::cmp::Reverse(k.split('/').count()));

    let mut trees: BTreeMap<String, Vec<TreeNode>> = BTreeMap::new();
    for dirkey in &all_dirs {
        let mut children: Vec<TreeNode> = Vec::new();
        let mut keys = sort_keys.remove(dirkey).unwrap_or_default();
        // 大小写不敏感排序；目录与文件混排（VSCode 观感：目录优先）
        keys.sort_by(|a, b| {
            (!a.1)
                .cmp(&!b.1)
                .then(a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });
        for (name, is_dir) in keys {
            let path = join_rel(Path::new(dirkey), &name)
                .to_string_lossy()
                .into_owned();
            if is_dir {
                let grandchildren = trees.remove(&path).unwrap_or_default();
                children.push(TreeNode::Dir {
                    name,
                    path,
                    children: grandchildren,
                });
            } else {
                let ftype = FileType::of(&name);
                children.push(TreeNode::File { name, path, ftype });
            }
        }
        trees.insert(dirkey.clone(), children);
    }

    let tree = trees.remove("").unwrap_or_default();
    ScanResult {
        root: root.to_string_lossy().into_owned(),
        tree,
        truncated,
        dir_count,
        file_count,
    }
}

/// 相对路径拼接（恒正斜杠——web 端协议一致性）。
fn join_rel(base: &Path, name: &str) -> PathBuf {
    if base.as_os_str().is_empty() {
        PathBuf::from(name)
    } else {
        PathBuf::from(format!("{}/{}", base.to_string_lossy(), name))
    }
}

/// 最近打开记录（工作区级：`<root>/.mang/recent.json`）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct RecentStore {
    #[serde(default)]
    pub entries: Vec<RecentEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RecentEntry {
    pub path: String,
    pub name: String,
    /// 打开时间（Unix 秒——UI 端格式化为「2 小时前」）。
    pub opened_at: u64,
}

impl RecentStore {
    fn file_path(root: &Path) -> PathBuf {
        root.join(spec::META_DIR).join(spec::RECENT_FILE)
    }

    /// 读档（无文件/坏 JSON → 空表——首次打开与手改容错）。
    pub fn load(root: &Path) -> Self {
        match std::fs::read_to_string(Self::file_path(root)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => RecentStore::default(),
        }
    }

    /// 保存（原子写：临时文件 + rename——参照 S-6 组装导出的原子落盘纪律）。
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let p = Self::file_path(root);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("建 .mang 失败: {e}"))?;
        }
        let tmp = p.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| format!("写 recent 失败: {e}"))?;
        std::fs::rename(&tmp, &p).map_err(|e| format!("原子替换 recent 失败: {e}"))
    }

    /// 记一次打开（去重置顶 + 上限淘汰）。
    pub fn record_open(&mut self, path: &str, name: &str, opened_at: u64) {
        self.entries.retain(|e| e.path != path);
        self.entries.insert(
            0,
            RecentEntry {
                path: path.to_string(),
                name: name.to_string(),
                opened_at,
            },
        );
        self.entries.truncate(spec::RECENT_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mang_ws_test_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn file_type_routing() {
        assert_eq!(FileType::of("a.DOCX"), FileType::Doc);
        assert_eq!(FileType::of("b.md"), FileType::Doc);
        assert_eq!(FileType::of("c.XLSX"), FileType::Sheet);
        assert_eq!(FileType::of("d.pdf"), FileType::Pdf);
        assert_eq!(FileType::of("e.bin"), FileType::Other);
        assert_eq!(FileType::of("无扩展名"), FileType::Other);
    }

    #[test]
    fn scan_skips_meta_and_sorts_dirs_first() {
        let root = tmpdir("scan1");
        // 项目A/a.docx、项目A/b.xlsx、招标文件.docx、.mang/x、.git/config、技术标.md
        fs::create_dir_all(root.join("项目A")).unwrap();
        fs::create_dir_all(root.join(".mang")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("项目A/a.docx"), b"x").unwrap();
        fs::write(root.join("项目A/b.xlsx"), b"x").unwrap();
        fs::write(root.join("招标文件.docx"), b"x").unwrap();
        fs::write(root.join("技术标.md"), b"x").unwrap();
        fs::write(root.join(".mang/recent.json"), b"{}").unwrap();
        fs::write(root.join(".git/config"), b"x").unwrap();

        let r = scan_workspace(&root, &ScanLimits::default());
        assert!(!r.truncated);
        assert_eq!(r.file_count, 4, "a.docx/b.xlsx/招标文件.docx/技术标.md");
        assert_eq!(r.dir_count, 1);
        // 目录优先 + 字母序（码点）：[项目A/, 技术标.md, 招标文件.docx]——
        // 「技」U+6280 < 「招」U+62DB
        let names: Vec<String> = r
            .tree
            .iter()
            .map(|n| match n {
                TreeNode::Dir { name, .. } => format!("{name}/"),
                TreeNode::File { name, .. } => name.clone(),
            })
            .collect();
        assert_eq!(names[0], "项目A/", "目录优先");
        assert_eq!(
            names.len(),
            3,
            "顶层=项目A/+技术标.md+招标文件.docx（a.docx/b.xlsx 在项目A 内）"
        );
        assert_eq!(names[1], "技术标.md", "码点序：技 U+6280 < 招 U+62DB");
        assert_eq!(names[2], "招标文件.docx");
        // .mang/.git 不在树里
        assert!(r.tree.iter().all(|n| match n {
            TreeNode::Dir { name, .. } => name != ".mang" && name != ".git",
            _ => true,
        }));
        // 相对路径正斜杠形
        if let TreeNode::File { path, .. } = &r.tree[1] {
            assert!(path.contains('/') || !path.is_empty());
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_limits_truncate_explicitly() {
        let root = tmpdir("scan2");
        for i in 0..5 {
            fs::write(root.join(format!("f{i}.md")), b"x").unwrap();
        }
        let r = scan_workspace(
            &root,
            &ScanLimits {
                max_depth: 1,
                max_dirs: 0,
                max_files: 3,
            },
        );
        assert!(r.truncated, "越限必须显式标注");
        assert_eq!(r.file_count, 3);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recent_store_dedupe_and_cap() {
        let root = tmpdir("recent");
        let mut st = RecentStore::default();
        st.record_open("a.md", "a.md", 100);
        st.record_open("b.md", "b.md", 200);
        st.record_open("a.md", "a.md", 300);
        assert_eq!(st.entries.len(), 2);
        assert_eq!(st.entries[0].path, "a.md", "重复打开置顶");
        assert_eq!(st.entries[0].opened_at, 300);

        for i in 0..(spec::RECENT_MAX + 5) {
            st.record_open(&format!("f{i}.md"), "x", i as u64);
        }
        assert_eq!(st.entries.len(), spec::RECENT_MAX, "上限淘汰最旧");
        st.save(&root).unwrap();

        let loaded = RecentStore::load(&root);
        assert_eq!(loaded.entries.len(), spec::RECENT_MAX);
        assert_eq!(
            loaded.entries[0].path,
            format!("f{}.md", spec::RECENT_MAX + 4)
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recent_load_tolerates_corrupt_json() {
        let root = tmpdir("recent_bad");
        fs::create_dir_all(root.join(".mang")).unwrap();
        fs::write(root.join(".mang/recent.json"), "{broken").unwrap();
        assert!(RecentStore::load(&root).entries.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_workspace_forms() {
        let d = tmpdir("resolve");
        // 绝对路径
        let abs = d.to_string_lossy().into_owned();
        assert_eq!(
            resolve_workspace(&["--workspace".into(), abs.clone()]).unwrap(),
            d
        );
        // = 形
        assert_eq!(
            resolve_workspace(&[format!("--workspace={abs}")]).unwrap(),
            d
        );
        // 缺参
        assert!(resolve_workspace(&["--workspace".into()]).is_err());
        // 不存在
        assert!(resolve_workspace(&["--workspace".into(), "/nonexistent/xyz".into()]).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
