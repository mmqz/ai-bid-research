//! 文件系统类命令：fs.tree / fs.read / fs.readBytes / fs.writeBytes / fs.stat / fs.mkdir / preview.write

use std::path::Path;

use serde_json::{json, Map, Value};

use crate::b64;
use crate::ipc::{mtime_ms, Args, IpcError};
use crate::paths::{self, Workspace};

/// fs.read 的文本上限
pub const TEXT_LIMIT: u64 = 8 * 1024 * 1024;
/// fs.readBytes 的上限（base64 后约 64MB，再大就交给预览/流式方案）
pub const BYTES_LIMIT: u64 = 48 * 1024 * 1024;
/// fs.tree 节点上限
pub const TREE_NODE_CAP: usize = 4000;
/// fs.tree 递归深度上限（防深度炸弹）
pub const TREE_DEPTH_CAP: usize = 32;
/// fs.writeBytes 上限
pub const WRITE_LIMIT: u64 = 64 * 1024 * 1024;

/// 树里永远跳过的目录名（小写比较）
// 这些目录永远不进树（与 includeHidden 无关）：版本历史/依赖/构建产物/回收站
const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", ".mang", "$recycle.bin"];

/// 自然排序：把数字段当数字比（"2" < "10"，"第2章" < "第10章"）
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut ia, mut ib) = (0usize, 0usize);
    let ab: Vec<char> = a.chars().flat_map(|c| c.to_lowercase()).collect();
    let bb: Vec<char> = b.chars().flat_map(|c| c.to_lowercase()).collect();
    while ia < ab.len() && ib < bb.len() {
        let ca = ab[ia];
        let cb = bb[ib];
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let sa = ia;
            while ia < ab.len() && ab[ia].is_ascii_digit() {
                ia += 1;
            }
            let sb = ib;
            while ib < bb.len() && bb[ib].is_ascii_digit() {
                ib += 1;
            }
            let na: String = ab[sa..ia].iter().collect();
            let nb: String = bb[sb..ib].iter().collect();
            let ta = na.trim_start_matches('0');
            let tb = nb.trim_start_matches('0');
            let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
            if ord != Ordering::Equal {
                return ord;
            }
            // 数字完全相同就继续比后面的字符
        } else {
            if ca != cb {
                return ca.cmp(&cb);
            }
            ia += 1;
            ib += 1;
        }
    }
    ia.cmp(&ib).then_with(|| ab.len().cmp(&bb.len())).then_with(|| a.cmp(b))
}

struct TreeNode {
    name: String,
    rel: String,
    kind: &'static str,
    size: u64,
    mtime_ms: u64,
    children: Vec<TreeNode>,
}

impl TreeNode {
    fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("name".into(), json!(self.name));
        o.insert("rel".into(), json!(self.rel));
        o.insert("kind".into(), json!(self.kind));
        if self.kind == "file" {
            o.insert("size".into(), json!(self.size));
        }
        o.insert("mtimeMs".into(), json!(self.mtime_ms));
        if self.kind == "dir" {
            o.insert("children".into(), json!(self.children.len()));
            let arr: Vec<Value> = self.children.iter().map(|c| c.to_json()).collect();
            o.insert("nodes".into(), Value::Array(arr));
        }
        Value::Object(o)
    }
}

/// fs.tree {path?, depth?, includeHidden?}
/// → {root, path, nodes:[{name, rel, kind, size, mtimeMs, nodes?}], truncated, nodeCount}
pub fn tree(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let sub = args.opt_path("path")?.unwrap_or("");
    let depth = args.u64("depth")?.unwrap_or(8) as usize;
    let depth = depth.clamp(1, TREE_DEPTH_CAP);
    let include_hidden = args.bool("includeHidden")?.unwrap_or(false);

    let start = ws.resolve(sub, false)?;
    if !start.is_dir() {
        return Err(IpcError::bad_args(format!("不是目录：{sub}")));
    }
    let mut budget = TREE_NODE_CAP;
    let mut truncated = false;
    let mut count = 0usize;

    let nodes = walk(ws, &start, depth, include_hidden, &mut budget, &mut truncated, &mut count)?;

    Ok(json!({
        "root": ws.root_display(),
        "path": if sub.is_empty() { String::new() } else { sub.to_string() },
        "nodes": nodes.iter().map(|n| n.to_json()).collect::<Vec<_>>(),
        "nodeCount": count,
        "nodeCap": TREE_NODE_CAP,
        "truncated": truncated,
    }))
}

fn walk(
    ws: &Workspace,
    dir: &Path,
    depth_left: usize,
    include_hidden: bool,
    budget: &mut usize,
    truncated: &mut bool,
    count: &mut usize,
) -> Result<Vec<TreeNode>, IpcError> {
    if depth_left == 0 || *budget == 0 {
        return Ok(Vec::new());
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        // 单个目录读不到不影响整棵树
        Err(_) => return Ok(Vec::new()),
    };
    let mut dirs: Vec<TreeNode> = Vec::new();
    let mut files: Vec<TreeNode> = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        let name = ent.file_name().to_string_lossy().into_owned();
        if !include_hidden && name.starts_with('.') {
            continue;
        }
        let meta = match ent.metadata() {
            Ok(m) => m,
            Err(_) => continue, // 断链/权限
        };
        let is_dir = meta.is_dir();
        if is_dir {
            let lname = name.to_lowercase();
            if SKIP_DIRS.iter().any(|s| *s == lname) {
                continue;
            }
        } else if !meta.is_file() {
            continue; // 设备/符号链接等
        }
        if *budget == 0 {
            *truncated = true;
            break;
        }
        *budget -= 1;
        *count += 1;
        let rel = ws
            .rel_of(&path)
            .unwrap_or_else(|| paths::rel_join(&path));
        if is_dir {
            let children = walk(ws, &path, depth_left - 1, include_hidden, budget, truncated, count)?;
            dirs.push(TreeNode {
                name,
                rel,
                kind: "dir",
                size: 0,
                mtime_ms: mtime_ms(&meta),
                children,
            });
        } else {
            files.push(TreeNode {
                name,
                rel,
                kind: "file",
                size: meta.len(),
                mtime_ms: mtime_ms(&meta),
                children: Vec::new(),
            });
        }
    }
    dirs.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    files.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    dirs.extend(files);
    Ok(dirs)
}

/// fs.read {path} → {path, rel, bytes, text, mtimeMs}
pub fn read(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.path("path")?;
    let p = ws.resolve(raw, false)?;
    let meta = std::fs::metadata(&p).map_err(|e| paths::map_io(&p, &e))?;
    if meta.is_dir() {
        return Err(IpcError::bad_args(format!("是目录，不能 fs.read：{raw}")));
    }
    if meta.len() > TEXT_LIMIT {
        return Err(IpcError::too_large(format!(
            "{raw} 有 {} 字节，超过文本读取上限 {} 字节",
            meta.len(),
            TEXT_LIMIT
        )));
    }
    let bytes = std::fs::read(&p).map_err(|e| paths::map_io(&p, &e))?;
    let n = bytes.len();
    let text = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    Ok(json!({
        "path": paths::display(&p),
        "rel": ws.rel_of(&p).unwrap_or_else(|| raw.to_string()),
        "bytes": n,
        "text": text,
        "mtimeMs": mtime_ms(&meta),
    }))
}

/// fs.readBytes {path} → {path, rel, bytes, enc:"b64", data, mtimeMs}
pub fn read_bytes(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.path("path")?;
    let p = ws.resolve(raw, false)?;
    let meta = std::fs::metadata(&p).map_err(|e| paths::map_io(&p, &e))?;
    if meta.is_dir() {
        return Err(IpcError::bad_args(format!("是目录，不能 fs.readBytes：{raw}")));
    }
    if meta.len() > BYTES_LIMIT {
        return Err(IpcError::too_large(format!(
            "{raw} 有 {} 字节，超过二进制读取上限 {} 字节",
            meta.len(),
            BYTES_LIMIT
        )));
    }
    let bytes = std::fs::read(&p).map_err(|e| paths::map_io(&p, &e))?;
    let n = bytes.len();
    Ok(json!({
        "path": paths::display(&p),
        "rel": ws.rel_of(&p).unwrap_or_else(|| raw.to_string()),
        "bytes": n,
        "enc": "b64",
        "data": b64::encode(&bytes),
        "mtimeMs": mtime_ms(&meta),
    }))
}

/// fs.writeBytes {path, enc:"b64", data} → {path, bytes}
///
/// 安全门槛（IPC-CONTRACT §5.2）：**只允许写工作区内的 `.mang/`**（历史快照、导出中间件）。
/// 草稿/正文的保存以后另开命令并带显式授权，别顺手放宽这里。
pub fn write_bytes(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.path("path")?;
    let enc = args.str("enc")?.unwrap_or("b64");
    let data = args.req_str("data")?;
    let p = ws.resolve(raw, true)?;
    let mang = ws.sub(".mang");
    let mang_canon = std::fs::canonicalize(&mang).unwrap_or(mang.clone());
    if !paths::is_inside(&p, &mang_canon) {
        return Err(IpcError::denied(format!(
            "fs.writeBytes 只允许写工作区内的 .mang/：{raw}"
        )));
    }
    if enc != "b64" {
        return Err(IpcError::bad_args(format!("不支持的 enc：{enc}（只支持 b64）")));
    }
    let bytes = b64::decode(data).map_err(IpcError::bad_args)?;
    if bytes.len() as u64 > WRITE_LIMIT {
        return Err(IpcError::too_large(format!(
            "写入 {} 字节超过上限 {}",
            bytes.len(),
            WRITE_LIMIT
        )));
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| paths::map_io(parent, &e))?;
    }
    write_atomic(&p, &bytes)?;
    Ok(json!({ "path": paths::display(&p), "rel": ws.rel_of(&p), "bytes": bytes.len() }))
}

/// 先写 .tmp 再 rename，避免半截文件
fn write_atomic(p: &Path, bytes: &[u8]) -> Result<(), IpcError> {
    let tmp = p.with_extension(format!(
        "{}.tmp",
        p.extension().and_then(|e| e.to_str()).unwrap_or("bin")
    ));
    std::fs::write(&tmp, bytes).map_err(|e| paths::map_io(&tmp, &e))?;
    match std::fs::rename(&tmp, p) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(paths::map_io(p, &e))
        }
    }
}

/// fs.stat {path} → {path, rel, size, mtimeMs, isDir, exists}
pub fn stat(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.path("path")?;
    let p = ws.resolve(raw, false)?;
    match std::fs::metadata(&p) {
        Ok(m) => Ok(json!({
            "path": paths::display(&p),
            "rel": ws.rel_of(&p).unwrap_or_else(|| raw.to_string()),
            "size": if m.is_dir() { 0 } else { m.len() },
            "mtimeMs": mtime_ms(&m),
            "isDir": m.is_dir(),
            "exists": true,
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({
            "path": paths::display(&p),
            "rel": raw,
            "size": 0,
            "mtimeMs": 0,
            "isDir": false,
            "exists": false,
        })),
        Err(e) => Err(paths::map_io(&p, &e)),
    }
}

/// fs.mkdir {path} → {ok:true, path}
pub fn mkdir(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.path("path")?;
    let p = ws.resolve(raw, true)?;
    std::fs::create_dir_all(&p).map_err(|e| paths::map_io(&p, &e))?;
    Ok(json!({ "ok": true, "path": paths::display(&p), "rel": ws.rel_of(&p) }))
}

/// file.fromPath {path} → {path, rel, kind:"file"|"dir", size, mtimeMs, insideWorkspace}
///
/// 入参可以是工作区外的绝对路径（拖拽/对话框来的），此时 rel=null、insideWorkspace=false；
/// 环境变量与 `~` 会被展开。
pub fn from_path(ws: &Workspace, args: Args<'_>) -> Result<Value, IpcError> {
    let raw = args.req_str("path")?.trim();
    if raw.is_empty() {
        return Err(IpcError::bad_args("path 不能为空"));
    }
    let expanded = expand_env(raw);
    let p = std::path::PathBuf::from(&expanded);
    let p = if p.is_absolute() {
        p
    } else {
        // 相对路径按工作区解析
        return description(ws, &ws.resolve(&expanded, false)?);
    };
    description(ws, &p)
}

fn description(ws: &Workspace, p: &Path) -> Result<Value, IpcError> {
    let meta = std::fs::metadata(p).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => IpcError::not_found(format!("不存在：{}", paths::display(p))),
        _ => paths::map_io(p, &e),
    })?;
    Ok(json!({
        "path": paths::display(p),
        "rel": ws.rel_of(p),
        "kind": if meta.is_dir() { "dir" } else { "file" },
        "size": if meta.is_dir() { 0 } else { meta.len() },
        "mtimeMs": mtime_ms(&meta),
        "insideWorkspace": ws.rel_of(p).is_some(),
    }))
}

/// 展开 %VAR% 与开头的 ~
fn expand_env(raw: &str) -> String {
    let mut s = raw.to_string();
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            s = format!("{}{}{}", std::path::PathBuf::from(home).display(), std::path::MAIN_SEPARATOR, rest);
        }
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == '%') {
                let name: String = chars[i + 1..i + 1 + end].iter().collect();
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    if let Some(v) = std::env::var_os(&name) {
                        out.push_str(&v.to_string_lossy());
                        i = i + 1 + end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// preview.write {name, enc:"b64", data} → {path, bytes}
/// 写到 %TEMP%\mang-preview\<uuid>-<name>（不进工作区，供 iframe 内预览大文件）
pub fn preview_write(args: Args<'_>) -> Result<Value, IpcError> {
    let name = args.req_str("name")?;
    let enc = args.str("enc")?.unwrap_or("b64");
    let data = args.req_str("data")?;
    if enc != "b64" {
        return Err(IpcError::bad_args(format!("不支持的 enc：{enc}（只支持 b64）")));
    }
    // 文件名清洗：去掉路径分隔符与控制字符
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = if clean.trim().is_empty() { "preview.bin".to_string() } else { clean };
    let bytes = b64::decode(data).map_err(IpcError::bad_args)?;
    if bytes.len() as u64 > BYTES_LIMIT {
        return Err(IpcError::too_large(format!(
            "预览文件 {} 字节超过上限 {}",
            bytes.len(),
            BYTES_LIMIT
        )));
    }
    let uuid = crate::uuid_v4();
    let dir = crate::ipc::preview_dir();
    let path = dir.join(format!("{uuid}-{clean}"));
    std::fs::write(&path, &bytes).map_err(|e| paths::map_io(&path, &e))?;
    Ok(json!({
        "path": paths::display(&path),
        "bytes": bytes.len(),
        "name": clean,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::parse;

    #[test]
    fn natural_sort() {
        let mut v = vec!["第10章.md", "第2章.md", "a10.txt", "a2.txt", "b.md"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["a2.txt", "a10.txt", "b.md", "第2章.md", "第10章.md"]);
    }

    fn fixture() -> (std::path::PathBuf, Workspace) {
        let d = std::env::temp_dir().join(format!(
            "mang-fs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(d.join("sub/deep")).unwrap();
        std::fs::create_dir_all(d.join("node_modules/x")).unwrap();
        std::fs::write(d.join("a.md"), "# hi").unwrap();
        std::fs::write(d.join("sub/b.json"), "{}").unwrap();
        std::fs::write(d.join("sub/deep/c.txt"), "c").unwrap();
        std::fs::create_dir_all(d.join(".mang/history")).unwrap();
        std::fs::write(d.join(".mang/history/index.json"), "[]").unwrap();
        let ws = Workspace::new(&d).unwrap();
        (d, ws)
    }

    fn args(v: serde_json::Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn tree_shape() {
        let (d, ws) = fixture();
        let out = tree(&ws, Args::new(&args(json!({})))).unwrap();
        assert_eq!(out["root"], json!(paths::display(ws.root_canonical())));
        let names: Vec<String> = out["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["name"].as_str().unwrap().to_string())
            .collect();
        // 目录在前；.mang / node_modules 被跳过；自然排序
        assert_eq!(names, vec!["sub", "a.md"]);
        assert_eq!(out["nodes"][0]["kind"], json!("dir"));
        assert_eq!(out["nodes"][0]["rel"], json!("sub"));
        assert_eq!(out["nodes"][1]["kind"], json!("file"));
        assert_eq!(out["nodes"][1]["size"], json!(4));
        assert_eq!(out["truncated"], json!(false));
        // 子目录带 nodes
        let sub = &out["nodes"][0]["nodes"];
        assert_eq!(sub.as_array().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn read_and_bytes() {
        let (d, ws) = fixture();
        let r = read(&ws, Args::new(&args(json!({"path":"a.md"})))).unwrap();
        assert_eq!(r["text"], json!("# hi"));
        assert_eq!(r["bytes"], json!(4));
        assert_eq!(r["rel"], json!("a.md"));
        let b = read_bytes(&ws, Args::new(&args(json!({"path":"a.md"})))).unwrap();
        assert_eq!(b["enc"], json!("b64"));
        assert_eq!(b64::decode(b["data"].as_str().unwrap()).unwrap(), b"# hi");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn write_bytes_guard() {
        let (d, ws) = fixture();
        let payload = b64::encode(b"snapshot!");
        // 工作区内非 .mang 路径 → denied
        let e = write_bytes(
            &ws,
            Args::new(&args(json!({"path":"a.md","enc":"b64","data":payload}))),
        )
        .unwrap_err();
        assert_eq!(e.code, "denied");
        // .mang 下 → ok
        let out = write_bytes(
            &ws,
            Args::new(&args(json!({"path":".mang/history/x/1.md","enc":"b64","data":payload}))),
        )
        .unwrap();
        assert_eq!(out["bytes"], json!(9));
        assert!(d.join(".mang/history/x/1.md").exists());
        // 越界 → outside_workspace
        let e = write_bytes(
            &ws,
            Args::new(&args(json!({"path":"../evil.txt","enc":"b64","data":"aGk="}))),
        )
        .unwrap_err();
        assert_eq!(e.code, "outside_workspace");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn stat_missing_is_exists_false() {
        let (d, ws) = fixture();
        let s = stat(&ws, Args::new(&args(json!({"path":"nope.md"})))).unwrap();
        assert_eq!(s["exists"], json!(false));
        let s = stat(&ws, Args::new(&args(json!({"path":"sub"})))).unwrap();
        assert_eq!(s["isDir"], json!(true));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn dispatch_never_panics_on_garbage() {
        let (d, ws) = fixture();
        let ctx = crate::ipc::Ctx::new(ws, d.clone());
        for body in [
            r#"{"id":1,"cmd":"fs.tree","depth":"deep"}"#,
            r#"{"id":2,"cmd":"fs.read"}"#,
            r#"{"id":3,"cmd":"nope"}"#,
            r#"{"id":4,"cmd":"fs.read","path":123}"#,
            r#"{"id":5,"cmd":"fs.readBytes","path":"../outside"}"#,
            r#"[]"#,
            r#"{"id":6,"cmd":"ping"}"#,
        ] {
            let req = match parse(body) {
                Ok(r) => r,
                Err(e) => {
                    let f = crate::ipc::err_frame(&serde_json::Value::Null, &e);
                    assert_eq!(f["ok"], json!(false));
                    continue;
                }
            };
            let resp = ctx.dispatch(&req);
            assert!(resp.get("id").is_some());
            assert!(resp.get("ok").is_some());
        }
        let _ = std::fs::remove_dir_all(d);
    }
}
