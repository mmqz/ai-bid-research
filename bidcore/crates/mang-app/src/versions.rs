//! 版本历史：`.mang/history/<文件名>/<时间戳>.md` 快照 + `index.json` 元数据。
//!
//! #42 §八（版本历史 A+B）/ 06:32 补充（git 集成是后续轮——本模块先落
//! 文件快照层，git 引擎在其上叠加）。
//!
//! 设计要点：
//! - **md 层快照**：只有 .md 文件有版本历史（#42 明示「只在 md 层做 diff
//!   ——源头是 md，docx/pdf 是最终产物不做 diff」）；
//! - **自动快照**：30s 节流 + 内容变化门（无变化跳过——索引里不留「无变化」
//!   记录，与 #42 示例的「(无变化跳过)」语义对齐：跳过即不写）；
//! - **AI 主动存**：`kind=ai` + message（MCP `save_version` 的落地面）；
//! - **文件名安全化**：`报价/第2章.md` 这类含分隔符/非法字符的名字 →
//!   保留可读性前提下替换非法字符（Windows 保留字符 + 路径分隔符 + 控制符）；
//! - **索引原子写**：tmp + rename；**快照只追加不覆盖**（时间戳到秒冲突时
//!   加序号后缀——同秒两次 AI 存不互吞）。

use std::path::{Path, PathBuf};

use crate::spec;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VersionEntry {
    /// 相对工作区的 md 路径（正斜杠形）。
    pub file: String,
    /// 快照落盘文件名（`<时间戳>.md`——索引到磁盘文件的映射键）。
    pub snapshot: String,
    /// 时间（Unix 秒）。
    pub ts: u64,
    /// auto = 30s 检查自动存；ai = MCP save_version 主动存。
    #[serde(rename = "type")]
    pub kind: VersionKind,
    /// 版本说明（ai 存必填；auto 为 None）。
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionKind {
    Auto,
    Ai,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct VersionIndex {
    #[serde(default)]
    pub entries: Vec<VersionEntry>,
}

impl VersionIndex {
    fn path(root: &Path) -> PathBuf {
        root.join(spec::META_DIR).join("history").join("index.json")
    }

    pub fn load(root: &Path) -> Self {
        match std::fs::read_to_string(Self::path(root)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => VersionIndex::default(),
        }
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let p = Self::path(root);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("建 history 失败: {e}"))?;
        }
        let tmp = p.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| format!("写 index 失败: {e}"))?;
        std::fs::rename(&tmp, &p).map_err(|e| format!("原子替换 index 失败: {e}"))
    }

    /// 某文件的版本列表（新→旧）。
    pub fn versions_of(&self, file: &str) -> Vec<&VersionEntry> {
        let mut v: Vec<&VersionEntry> = self.entries.iter().filter(|e| e.file == file).collect();
        v.sort_by(|a, b| b.ts.cmp(&a.ts).then(b.snapshot.cmp(&a.snapshot)));
        v
    }

    fn push_entry(&mut self, e: VersionEntry) {
        self.entries.push(e);
        // 全索引软上限（防病态长寿工作区无限膨胀——10 万条后淘汰最旧 auto）
        const CAP: usize = 100_000;
        if self.entries.len() > CAP {
            self.entries.retain(|x| x.kind == VersionKind::Ai);
            if self.entries.len() > CAP / 2 {
                self.entries.drain(CAP / 2..);
            }
        }
    }
}

/// 文件名安全化（跨平台可读优先）。
///
/// Windows 保留字符 `<>:"/\|?*` + 控制字符 → `_`；`.` 首尾保留语义但
/// 全点名（`.`/`..`）整体替换。中文与空格保留（Windows/macOS/Linux 均合法）。
pub fn safe_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            let bad = matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
                || (c as u32) < 0x20;
            if bad {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "_".to_string()
    } else {
        cleaned
    }
}

/// Unix 秒 → `yyyyMMdd-HHmmss` 快照文件名（UTC——索引里另有 ts 原值，
/// 展示层用 ts 格式化本地时间，文件名仅作稳定排序键）。
pub fn snapshot_name(ts: u64, seq: usize) -> String {
    // 手写 civil-from-days（Howard Hinnant 算法）——零依赖，避免拖 chrono
    let days = (ts / 86400) as i64;
    let secs = ts % 86400;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if seq == 0 {
        format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
    } else {
        format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}-{seq:02}")
    }
}

/// days since 1970-01-01 → (y, m, d)。Howard Hinnant civil_from_days。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 自动快照判定（纯函数——#42 §八「每 30 秒检查：内容有变化才存」）。
///
/// `now` 为本次检查时间；返回 `Some(ts)` 表示应存快照（ts = now）。
pub fn should_autosnap(
    content_now: &str,
    content_at_last_snap: Option<&str>,
    last_snap_ts: Option<u64>,
    now: u64,
) -> bool {
    // 内容门：与上次快照比较有变化（无历史 = 首次见到该文件，有内容才存）
    let changed = match content_at_last_snap {
        Some(prev) => prev != content_now,
        None => !content_now.is_empty(),
    };
    if !changed {
        return false;
    }
    // 节流门：距上次快照 ≥ 间隔（首快照不节流——编辑者打开就改了，尽快有底）
    match last_snap_ts {
        Some(t) => now.saturating_sub(t) >= spec::AUTOSNAP_INTERVAL_SECS,
        None => true,
    }
}

/// 存一份快照（auto 或 ai）——写快照文件 + 更新索引（两步，索引最后写）。
///
/// 快照目录：`<root>/.mang/history/<safe_stem>/<snapshot>.md`。
/// 同秒冲突自动加序号（`-01`、`-02`…）。
pub fn save_snapshot(
    root: &Path,
    file: &str,
    content: &str,
    kind: VersionKind,
    message: Option<&str>,
    now: u64,
    seq_start: usize,
) -> Result<String, String> {
    let stem = safe_stem(
        Path::new(file)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .as_deref()
            .unwrap_or("_"),
    );
    let dir = root
        .join(spec::META_DIR)
        .join(spec::HISTORY_DIR)
        .join(&stem);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建快照目录失败: {e}"))?;

    // 同秒冲突探测（seq_start 让调用方在并发场景传起始序号；常态 0）
    let mut seq = seq_start;
    let mut snap_file = format!("{}.md", snapshot_name(now, 0));
    while dir.join(&snap_file).exists() {
        seq += 1;
        if seq > 99 {
            return Err("同秒快照超 99 份——异常频繁".to_string());
        }
        snap_file = format!("{}.md", snapshot_name(now, seq));
    }
    // 写快照文件；索引记录同名（含 .md——读回拼路径直接用，读写一致）
    std::fs::write(dir.join(&snap_file), content).map_err(|e| format!("写快照失败: {e}"))?;

    let mut idx = VersionIndex::load(root);
    idx.push_entry(VersionEntry {
        file: file.to_string(),
        snapshot: snap_file.clone(),
        ts: now,
        kind,
        message: message.map(|s| s.to_string()),
    });
    idx.save(root)?;
    Ok(snap_file)
}

/// 读回某版本内容。
pub fn read_snapshot(root: &Path, file: &str, snapshot: &str) -> Result<String, String> {
    // snapshot 必须是安全形（无路径分隔/穿越段）——防御纵深
    if snapshot.contains('/') || snapshot.contains('\\') || snapshot.contains("..") {
        return Err("非法快照名".to_string());
    }
    let stem = safe_stem(
        Path::new(file)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .as_deref()
            .unwrap_or("_"),
    );
    let p = root
        .join(spec::META_DIR)
        .join(spec::HISTORY_DIR)
        .join(&stem)
        .join(snapshot);
    std::fs::read_to_string(&p).map_err(|e| format!("读快照失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mang_ver_test_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn safe_stem_cross_platform() {
        assert_eq!(safe_stem("第2章 技术方案.md"), "第2章 技术方案.md");
        assert_eq!(safe_stem("a<b>:c\"d/e|f?g*.md"), "a_b__c_d_e_f_g_.md");
        assert_eq!(safe_stem(".."), "_");
        assert_eq!(safe_stem("  ."), "_");
        assert_eq!(safe_stem("报价"), "报价");
    }

    #[test]
    fn snapshot_name_format_and_seq() {
        // 2026-10-29 06:30:05 UTC = 1792000205? 用已知锚：1970-01-02 03:04:05
        let ts = 86400 + 3 * 3600 + 4 * 60 + 5;
        assert_eq!(snapshot_name(ts, 0), "19700102-030405");
        assert_eq!(snapshot_name(ts, 3), "19700102-030405-03");
        // 闰年锚：2024-02-29 23:59:59 UTC = 1709251199
        assert_eq!(snapshot_name(1_709_251_199, 0), "20240229-235959");
    }

    #[test]
    fn autosnap_gates() {
        let now = 1_000_000;
        // 首见有内容 → 存
        assert!(should_autosnap("hello", None, None, now));
        // 首见空内容 → 不存
        assert!(!should_autosnap("", None, None, now));
        // 内容没变 → 不存
        assert!(!should_autosnap("same", Some("same"), Some(now - 100), now));
        // 内容变了但 30s 未到 → 不存
        assert!(!should_autosnap("new", Some("old"), Some(now - 10), now));
        // 内容变了且 30s 已到 → 存
        assert!(should_autosnap("new", Some("old"), Some(now - 30), now));
        // 无上次快照时间但已有内容记录（首快照）→ 存
        assert!(should_autosnap("new", Some("old"), None, now));
    }

    #[test]
    fn save_read_and_index_roundtrip() {
        let root = tmpdir("round");
        let s1 = save_snapshot(
            &root,
            "chapters/ch02.md",
            "v1 内容",
            VersionKind::Ai,
            Some("第2章第1块完成"),
            1_000,
            0,
        )
        .unwrap();
        let s2 = save_snapshot(
            &root,
            "chapters/ch02.md",
            "v2 内容",
            VersionKind::Auto,
            None,
            1_050,
            0,
        )
        .unwrap();

        let idx = VersionIndex::load(&root);
        let v = idx.versions_of("chapters/ch02.md");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].snapshot, s2, "新→旧序");
        assert_eq!(v[0].kind, VersionKind::Auto);
        assert_eq!(v[1].kind, VersionKind::Ai);
        assert_eq!(v[1].message.as_deref(), Some("第2章第1块完成"));

        assert_eq!(
            read_snapshot(&root, "chapters/ch02.md", &s1).unwrap(),
            "v1 内容"
        );
        assert_eq!(
            read_snapshot(&root, "chapters/ch02.md", &s2).unwrap(),
            "v2 内容"
        );

        // 穿越防御
        assert!(read_snapshot(&root, "chapters/ch02.md", "../../etc/passwd").is_err());
        assert!(read_snapshot(&root, "chapters/ch02.md", "a/b").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn same_second_conflict_gets_sequence() {
        let root = tmpdir("conflict");
        let a = save_snapshot(&root, "x.md", "a", VersionKind::Ai, None, 5_000, 0).unwrap();
        let b = save_snapshot(&root, "x.md", "b", VersionKind::Ai, None, 5_000, 0).unwrap();
        assert_ne!(a, b, "同秒两份快照不互吞");
        assert!(b.ends_with("-01.md"), "第二份带序号，实际 {b}");
        let idx = VersionIndex::load(&root);
        assert_eq!(idx.versions_of("x.md").len(), 2);
        let _ = fs::remove_dir_all(&root);
    }
}
