//! 暗标清理（#317）：泄露扫描 → 自动清理 → 复验——暗标导出的端到端防线。
//!
//! 背景：暗标评标隐藏投标人身份，投标文件不得含任何可识别信息（公司名/
//! 作者/设备痕迹）。此前标书侧只有正文文本替换（`ExportOptions::anonymize`
//! ——公司名 → 「投标人」），**无** docProps 清零、无修订/批注作者匿名化、
//! 无产物级复验（audit FG-13：`unknown` 作者烤死在 docx-rs 默认 CoreProps——
//! 其 API 无 creator 设置器；引擎 G3「暗标元数据清零」为 ENGINE_GAP）。
//!
//! 本模块是**过渡实现**（zip 部件级手术，v1_darkbid.py SEMUL 验证逻辑的
//! Rust 移植——audit 修法建议）：引擎导出面产品化（G3/AD-07）后迁移引擎侧，
//! 届时本模块收敛为复验扫描器。
//!
//! 扫描面（issue 六类 + 路径/图片元数据）：
//! | 位置 | 判定 | 严重度 | 处置 |
//! |---|---|---|---|
//! | 正文（document.xml）| bidder_name 残留（拼接 w:t——跨 run 可检）| BLOCKER | 人工修订（资质说明可保留位置由人判断——导出被拦，不静默放行）|
//! | 页眉页脚（header*/footer*.xml）| bidder_name | BLOCKER | 自动替换（保格式删内容）|
//! | 文档属性（docProps/core.xml）| dc:creator / cp:lastModifiedBy 非空 | BLOCKER | 自动清零 |
//! | 文档属性（docProps/app.xml）| Company 非空 | BLOCKER | 自动清零 |
//! | 修订记录（w:ins/w:del 的 w:author）| 作者名非空 | BLOCKER | 自动匿名化 |
//! | 批注（comments*.xml 的 w:author）| 作者名非空 | BLOCKER | 自动匿名化 |
//! | 正文/页眉页脚 | 手机号形态 | WARNING | 人工确认（可能是必要联系方式）|
//! | word/media/* | JPEG EXIF 标记 | WARNING | 人工确认（可能含设备/作者）|
//! | 输出路径 | 含 bidder_name | WARNING | 人工确认 |
//!
//! 纪律：**BLOCKER 复验归零才允许导出**（issue 验收 4）；人工确认项列清单
//! 进导出响应（`dark_bid.manual_review`），不静默跳过（验收 3）。

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// 泄露位置（扫描报告的定位分类——六类 + 补充面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeakLocation {
    /// 正文（word/document.xml）
    Body,
    /// 页眉页脚（word/header*.xml / footer*.xml）
    HeaderFooter,
    /// 文档属性（docProps/core.xml + app.xml）
    DocProps,
    /// 修订记录作者（w:ins / w:del 的 w:author）
    RevisionAuthor,
    /// 批注作者（comments*.xml 的 w:author）
    CommentAuthor,
    /// 图片元数据（EXIF）
    ImageMeta,
    /// 输出文件路径
    OutputPath,
}

impl LeakLocation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Body => "正文",
            Self::HeaderFooter => "页眉页脚",
            Self::DocProps => "文档属性",
            Self::RevisionAuthor => "修订作者",
            Self::CommentAuthor => "批注作者",
            Self::ImageMeta => "图片元数据",
            Self::OutputPath => "输出路径",
        }
    }
}

/// 单条泄露命中。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeakHit {
    pub location: LeakLocation,
    /// 部件名（zip 内路径——人工排查的锚）
    pub part: String,
    /// 命中内容（脱敏截断——报告面不回传全量正文）
    pub content: String,
    /// "BLOCKER"（身份识别点——必须清零）| "WARNING"（可疑——人工确认）
    pub severity: String,
    /// 处置建议（自动清理 / 人工确认）
    pub suggested_action: String,
}

/// 暗标清理报告（进导出响应的 `dark_bid` 面）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DarkBidReport {
    /// 清理前扫描（自动清理项 + 人工确认项全量）
    pub pre_scan: Vec<LeakHit>,
    /// 已执行的自动清理动作（人读清单——清理了什么）
    pub cleaned: Vec<String>,
    /// 清理后复验（BLOCKER 必须为空；WARNING 为人工确认清单）
    pub post_scan: Vec<LeakHit>,
    /// 残余 BLOCKER 数（0 才放行导出）
    pub blockers: usize,
}

impl DarkBidReport {
    /// 人工确认清单（复验后的 WARNING 面）。
    pub fn manual_review(&self) -> Vec<&LeakHit> {
        self.post_scan.iter().filter(|h| h.severity == "WARNING").collect()
    }
}

/// 手机号形态（WARNING 面——联系方式可能是必要的，人工判断）。
fn phone_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"1[3-9]\d{9}").unwrap())
}

/// XML 文本面提取：拼接全部 `<w:t…>…</w:t>` 内容（跨 run 可检——与渲染面
/// 一致的身份泄漏探测口径），基础实体还原。
fn wt_text(xml: &str) -> String {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"<w:t[^>]*>([^<]*)</w:t>").unwrap());
    re.captures_iter(xml)
        .map(|c| c[1].to_string())
        .collect::<Vec<_>>()
        .join("")
}

/// docx 部件读出（zip 级——过渡实现，见模块头）。
fn read_parts(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("zip 读失败: {e}"))?;
    let mut parts = BTreeMap::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| format!("zip 条目读失败: {e}"))?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf).map_err(|e| format!("zip 内容读失败: {e}"))?;
        parts.insert(name, buf);
    }
    Ok(parts)
}

/// 部件写回（保序重打包——docx-rs pack 的部件集原样保留，仅替换被清理件）。
fn write_parts(parts: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in parts {
            writer.start_file(name.as_str(), opts).map_err(|e| format!("zip 写失败: {e}"))?;
            std::io::Write::write_all(&mut writer, data).map_err(|e| format!("zip 内容写失败: {e}"))?;
        }
        writer.finish().map_err(|e| format!("zip 收尾失败: {e}"))?;
    }
    Ok(out)
}

fn xml_of(parts: &BTreeMap<String, Vec<u8>>, name: &str) -> Option<String> {
    parts.get(name).map(|b| String::from_utf8_lossy(b).to_string())
}

/// 主件/页眉页脚部件名集合（document + header*/footer*——不含 rels）。
fn text_parts(parts: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    parts
        .keys()
        .filter(|n| {
            (n.starts_with("word/document") || n.starts_with("word/header") || n.starts_with("word/footer"))
                && n.ends_with(".xml")
                && !n.contains("_rels")
        })
        .cloned()
        .collect()
}

fn trunc(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// 泄露扫描（不修改产物——纯读）。
///
/// `bidder`：投标人公司名（Some 时做名字面扫描；None 时名字面无从扫——
/// 调用方已有「暗标但缺 bidder_name」告警，此处扫与名字无关的面：
/// docProps/修订/批注作者/联系方式/EXIF/路径）。
pub fn scan(bytes: &[u8], bidder: Option<&str>, out_path: &Path) -> Vec<LeakHit> {
    let mut hits = Vec::new();
    let Ok(parts) = read_parts(bytes) else {
        return hits; // zip 读失败由导出主链报（这里不重复报）——扫描面静默空
    };

    // ---- ① 正文/页眉页脚：名字面 + 联系方式面（拼接 w:t——跨 run 可检）
    for part in text_parts(&parts) {
        let Some(xml) = xml_of(&parts, &part) else { continue };
        let text = wt_text(&xml);
        let is_hf = part.contains("header") || part.contains("footer");
        if let Some(name) = bidder.filter(|n| !n.trim().is_empty()) {
            if text.contains(name.trim()) {
                hits.push(LeakHit {
                    location: if is_hf { LeakLocation::HeaderFooter } else { LeakLocation::Body },
                    part: part.clone(),
                    content: trunc(name.trim(), 60),
                    severity: "BLOCKER".into(),
                    suggested_action: if is_hf {
                        "自动清理（保格式替换为「投标人」）".into()
                    } else {
                        "人工修订正文后重导（资质说明可保留位置由人工判断——暗标场景通常整句删除）".into()
                    },
                });
            }
        }
        for m in phone_re().find_iter(&text) {
            hits.push(LeakHit {
                location: if is_hf { LeakLocation::HeaderFooter } else { LeakLocation::Body },
                part: part.clone(),
                content: m.as_str().to_string(),
                severity: "WARNING".into(),
                suggested_action: "人工确认（可能是必要的联系方式——暗标要求下通常应删除）".into(),
            });
        }
    }

    // ---- ② docProps：core.xml 作者字段 + app.xml Company
    if let Some(core) = xml_of(&parts, "docProps/core.xml") {
        static CREATOR: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = CREATOR.get_or_init(|| {
            Regex::new(r"<dc:creator>([^<]+)</dc:creator>|<cp:lastModifiedBy>([^<]+)</cp:lastModifiedBy>").unwrap()
        });
        for c in re.captures_iter(&core) {
            let content = c.get(1).or_else(|| c.get(2)).map(|m| m.as_str()).unwrap_or_default();
            hits.push(LeakHit {
                location: LeakLocation::DocProps,
                part: "docProps/core.xml".into(),
                content: trunc(content, 60),
                severity: "BLOCKER".into(),
                suggested_action: "自动清理（作者字段清零）".into(),
            });
        }
    }
    if let Some(app) = xml_of(&parts, "docProps/app.xml") {
        static COMPANY: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = COMPANY.get_or_init(|| Regex::new(r"<Company>([^<]+)</Company>").unwrap());
        for c in re.captures_iter(&app) {
            hits.push(LeakHit {
                location: LeakLocation::DocProps,
                part: "docProps/app.xml".into(),
                content: trunc(&c[1], 60),
                severity: "BLOCKER".into(),
                suggested_action: "自动清理（Company 字段清零）".into(),
            });
        }
    }

    // ---- ③ 修订作者（w:ins / w:del 的 w:author 属性）
    if let Some(doc) = xml_of(&parts, "word/document.xml") {
        static AUTHOR: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = AUTHOR.get_or_init(|| Regex::new(r#"<w:(?:ins|del)\b[^>]*\bw:author="([^"]+)""#).unwrap());
        for c in re.captures_iter(&doc) {
            hits.push(LeakHit {
                location: LeakLocation::RevisionAuthor,
                part: "word/document.xml".into(),
                content: trunc(&c[1], 60),
                severity: "BLOCKER".into(),
                suggested_action: "自动清理（修订作者匿名化）".into(),
            });
        }
    }

    // ---- ④ 批注作者（comments*.xml 的 w:author）
    for name in parts.keys().filter(|n| n.starts_with("word/comments") && n.ends_with(".xml")) {
        let Some(xml) = xml_of(&parts, name) else { continue };
        static CA: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = CA.get_or_init(|| Regex::new(r#"\bw:author="([^"]+)""#).unwrap());
        for c in re.captures_iter(&xml) {
            hits.push(LeakHit {
                location: LeakLocation::CommentAuthor,
                part: name.clone(),
                content: trunc(&c[1], 60),
                severity: "BLOCKER".into(),
                suggested_action: "自动清理（批注作者匿名化）".into(),
            });
        }
    }

    // ---- ⑤ 图片元数据（JPEG EXIF 标记——设备/作者可能入 EXIF）
    for (name, data) in parts.iter().filter(|(n, _)| n.starts_with("word/media/")) {
        // JPEG APP1 EXIF：FF D8 ... "Exif\0\0"
        let is_jpeg = data.len() > 4 && data[0] == 0xFF && data[1] == 0xD8;
        let has_exif = data.windows(6).take(64).any(|w| w == b"Exif\0\0");
        if is_jpeg && has_exif {
            hits.push(LeakHit {
                location: LeakLocation::ImageMeta,
                part: name.clone(),
                content: "JPEG 含 EXIF 块".into(),
                severity: "WARNING".into(),
                suggested_action: "人工确认（EXIF 可能含设备/作者——建议导出前转存去元数据）".into(),
            });
        }
    }

    // ---- ⑥ 输出路径（临时目录/工作区路径可能含用户名或公司名）
    if let Some(name) = bidder.filter(|n| !n.trim().is_empty()) {
        let p = out_path.to_string_lossy();
        if p.contains(name.trim()) {
            hits.push(LeakHit {
                location: LeakLocation::OutputPath,
                part: out_path.display().to_string(),
                content: trunc(name.trim(), 60),
                severity: "WARNING".into(),
                suggested_action: "人工确认（输出路径含公司名——重命名输出文件）".into(),
            });
        }
    }

    hits
}

/// 自动清理：docProps 清零 + 修订/批注作者匿名化 + 页眉页脚名字替换。
///
/// 不自动清理（人工确认面，见 scan 的 suggested_action）：正文公司名
/// （可能是必要的资质说明）、联系方式、EXIF、路径。
/// 返回（清理后字节, 动作清单）。
pub fn clean(bytes: &[u8], bidder: Option<&str>) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut parts = read_parts(bytes)?;
    let mut actions = Vec::new();

    // ① docProps/core.xml：作者字段清零
    if let Some(core) = xml_of(&parts, "docProps/core.xml") {
        static CREATOR: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = CREATOR.get_or_init(|| {
            Regex::new(r"<dc:creator>[^<]+</dc:creator>|<cp:lastModifiedBy>[^<]+</cp:lastModifiedBy>").unwrap()
        });
        let n = re.find_iter(&core).count();
        if n > 0 {
            let cleaned = re.replace_all(&core, "").to_string();
            parts.insert("docProps/core.xml".into(), cleaned.into_bytes());
            actions.push(format!("docProps/core.xml：作者字段清零（{n} 处）"));
        }
    }
    // ② docProps/app.xml：Company 清零
    if let Some(app) = xml_of(&parts, "docProps/app.xml") {
        static COMPANY: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = COMPANY.get_or_init(|| Regex::new(r"<Company>[^<]+</Company>").unwrap());
        if re.is_match(&app) {
            let cleaned = re.replace_all(&app, "").to_string();
            parts.insert("docProps/app.xml".into(), cleaned.into_bytes());
            actions.push("docProps/app.xml：Company 字段清零".into());
        }
    }
    // ③ 修订/批注作者匿名化（w:author 属性值清空——保留修订/批注内容本体）
    let author_parts: Vec<String> = parts
        .keys()
        .filter(|n| {
            (n.as_str() == "word/document.xml" || n.starts_with("word/comments"))
                && n.ends_with(".xml")
        })
        .cloned()
        .collect();
    static AUTHOR_ATTR: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = AUTHOR_ATTR.get_or_init(|| Regex::new(r#"\bw:author="[^"]*""#).unwrap());
    for name in author_parts {
        let Some(xml) = xml_of(&parts, &name) else { continue };
        if re.is_match(&xml) {
            let cleaned = re.replace_all(&xml, "w:author=\"\"").to_string();
            parts.insert(name.clone(), cleaned.into_bytes());
            actions.push(format!("{name}：作者属性匿名化"));
        }
    }
    // ④ 页眉页脚 + 正文名字替换（保格式——w:t 文本内替换；跨 run 切分名
    // 残留由复验拦截为 BLOCKER，人工处置）
    if let Some(name) = bidder.map(str::trim).filter(|n| !n.is_empty()) {
        for part in text_parts(&parts) {
            let Some(xml) = xml_of(&parts, &part) else { continue };
            if xml.contains(name) {
                let cleaned = xml.replace(name, "投标人");
                parts.insert(part.clone(), cleaned.into_bytes());
                actions.push(format!("{part}：公司名替换为「投标人」"));
            }
        }
    }

    Ok((write_parts(&parts)?, actions))
}

/// 端到端：扫描 → 自动清理 → 复验。
///
/// Ok((清理后字节, 报告))：BLOCKER 归零（残余 WARNING 进人工确认清单）；
/// Err(残余 BLOCKER 清单)：复验不通过——调用方必须拒绝导出（issue 验收 4）。
pub fn process(
    bytes: &[u8],
    bidder: Option<&str>,
    out_path: &Path,
) -> Result<(Vec<u8>, DarkBidReport), Vec<LeakHit>> {
    let pre_scan = scan(bytes, bidder, out_path);
    let (cleaned, actions) = match clean(bytes, bidder) {
        Ok(x) => x,
        Err(e) => {
            // 清理失败（zip 手术失败）——fail-loud：当 BLOCKER 处理，不放行
            let hit = LeakHit {
                location: LeakLocation::DocProps,
                part: "（自动清理）".into(),
                content: e,
                severity: "BLOCKER".into(),
                suggested_action: "自动清理失败——报告维护方（zip 手术面）".into(),
            };
            return Err(vec![hit]);
        }
    };
    let post_scan = scan(&cleaned, bidder, out_path);
    let blockers = post_scan.iter().filter(|h| h.severity == "BLOCKER").count();
    let report = DarkBidReport { pre_scan, cleaned: actions, blockers, post_scan };
    if blockers > 0 {
        return Err(report.post_scan);
    }
    Ok((cleaned, report))
}
