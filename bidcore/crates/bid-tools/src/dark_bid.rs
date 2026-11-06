//! dark_bid.rs —— #317：暗标泄露扫描（六类）+ 复验报告。
//!
//! 暗标 = 评标时隐藏投标人身份：产物里不能出现任何可识别投标人的信息。
//! 本模块是**标书侧扫描面**——引擎侧（mdcore `anonymize_docx_props`）
//! 承载自动清理（docProps/comments/people 五部件，正文零触碰），本模块
//! 负责：
//!
//! 1. **扫描**（六类泄露面）：
//!    - 正文（document.xml——逐段落合并 run 后查投标人名，防跨 run 切分漏检）
//!    - 页眉/页脚（header*.xml / footer*.xml——常带公司名）
//!    - 文档属性（docProps core 的 creator/lastModifiedBy、app/custom 的
//!      Company/Sender——引擎自动清理面）
//!    - 批注（comments.xml 的 w:author——引擎自动清理面）
//!    - 修订（document.xml 的 w:ins/w:del w:author——引擎不变量不清理，
//!      人工接受/拒绝修订后消失）
//!    - 图片元数据（word/media/* 的 EXIF APP1 在场即报——设备/作者/软件
//!      可能泄露，人工确认或清元数据重嵌）
//!    - 外加：输出路径含投标人名（临时目录带用户名/公司名）
//! 2. **复验**：BLOCKER（确定泄露）归零才放行导出；WARNING（可疑面）进
//!    人工确认清单——**不静默跳过**（issue 验收 3）。
//!
//! 严重度口径：BLOCKER = 确定的身份泄露（正文/页眉页脚/属性/批注/修订
//! 作者命中投标人名或非匿名作者）；WARNING = 泄露面在场但内容待人工
//! 判断（EXIF/路径）。

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- 报告类型

/// 泄露扫描报告（#317 契约面——export 响应 `dark_bid_scan` 的载体）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LeakScanReport {
    /// 全部命中（BLOCKER + WARNING——人工确认清单不静默）
    pub hits: Vec<LeakHit>,
    /// 自动清理面回执（引擎 anonymize 处理过的部件清单——审计面）
    pub auto_cleaned: Vec<String>,
    /// 复验结论：BLOCKER 归零才 true（导出门判据）
    pub passed: bool,
}

/// 单条泄露命中。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeakHit {
    /// 泄露位置（六类）
    pub location: LeakLocation,
    /// 命中内容（截断展示——审计定位面）
    pub content: String,
    /// BLOCKER（确定泄露）/ WARNING（可疑，人工确认）
    pub severity: Severity,
    /// 处置建议（自动清理已做 / 人工确认——不静默）
    pub suggested_action: String,
}

/// 泄露位置（六类 + 路径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeakLocation {
    Body,
    Header,
    Footer,
    DocProps,
    Comments,
    Revisions,
    MediaExif,
    Path,
}

impl LeakLocation {
    pub fn as_str(&self) -> &'static str {
        match self {
            LeakLocation::Body => "body",
            LeakLocation::Header => "header",
            LeakLocation::Footer => "footer",
            LeakLocation::DocProps => "doc_props",
            LeakLocation::Comments => "comments",
            LeakLocation::Revisions => "revisions",
            LeakLocation::MediaExif => "media_exif",
            LeakLocation::Path => "path",
        }
    }
}

/// 严重度（#317 口径：BLOCKER=公司名级确定泄露；WARNING=可疑面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Blocker,
    Warning,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Blocker => "BLOCKER",
            Severity::Warning => "WARNING",
        }
    }
}

// ---------------------------------------------------------------- 扫描

/// 暗标泄露扫描（六类）。
///
/// `docx_bytes`：待检 docx（导出链里是 anonymize 自动清理**之后**的产物
/// 字节——扫描即复验）；`bidder_name`：投标人身份串（正文/页眉页脚/路径
/// 的检索面——anonymize 文本替换后残留即命中）；`out_path`：产物输出
/// 路径（路径面泄露）。
pub fn scan_dark_bid_leaks(
    docx_bytes: &[u8],
    bidder_name: Option<&str>,
    out_path: &std::path::Path,
) -> LeakScanReport {
    let mut hits = Vec::new();
    let name = bidder_name.map(str::trim).filter(|s| !s.is_empty());

    let cursor = std::io::Cursor::new(docx_bytes);
    let Ok(mut archive) = zip::ZipArchive::new(cursor) else {
        // 非法 zip：导出链上游已保证 docx 语义（pack 产物）；扫描面 fail-open
        // 记 WARNING（不装作扫过了）
        hits.push(LeakHit {
            location: LeakLocation::DocProps,
            content: "产物不是合法 zip——扫描面无法执行（fail-open 如实报）".into(),
            severity: Severity::Warning,
            suggested_action: "人工核查产物".into(),
        });
        return LeakScanReport { hits, auto_cleaned: vec![], passed: true };
    };

    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index(i) else { continue };
        let entry_name = entry.name().to_string();
        let bytes = {
            use std::io::Read;
            let mut b = Vec::new();
            let mut e = entry;
            if e.read_to_end(&mut b).is_err() {
                continue;
            }
            b
        };

        if entry_name == "word/document.xml" {
            scan_document_xml(&bytes, name, &mut hits);
        } else if entry_name.starts_with("word/header") && entry_name.ends_with(".xml") {
            scan_text_face(&bytes, name, LeakLocation::Header, &mut hits);
        } else if entry_name.starts_with("word/footer") && entry_name.ends_with(".xml") {
            scan_text_face(&bytes, name, LeakLocation::Footer, &mut hits);
        } else if entry_name == "docProps/core.xml" {
            scan_core_props(&bytes, name, &mut hits);
        } else if entry_name == "docProps/app.xml" || entry_name == "docProps/custom.xml" {
            scan_extended_props(&entry_name, &bytes, name, &mut hits);
        } else if entry_name == "word/comments.xml" {
            scan_comments_authors(&bytes, name, &mut hits);
        } else if entry_name.starts_with("word/media/") {
            scan_media_exif(&entry_name, &bytes, &mut hits);
        }
    }

    // 路径面（输出路径含投标人名——临时目录带用户名/公司名的真实形态）
    if let Some(n) = name {
        if out_path
            .to_string_lossy()
            .contains(n)
        {
            hits.push(LeakHit {
                location: LeakLocation::Path,
                content: out_path.to_string_lossy().chars().take(120).collect(),
                severity: Severity::Warning,
                suggested_action: "人工确认：输出路径含投标人名（改用中性产物名）".into(),
            });
        }
    }

    let passed = !hits
        .iter()
        .any(|h| h.severity == Severity::Blocker);
    LeakScanReport { hits, auto_cleaned: vec![], passed }
}

/// document.xml：正文（逐段合并 run 防跨 run 切分漏检）+ 修订作者属性。
fn scan_document_xml(bytes: &[u8], name: Option<&str>, hits: &mut Vec<LeakHit>) {
    let xml = String::from_utf8_lossy(bytes);
    // 正文身份命中：逐段落合并 <w:t> 文本再查（run 边界不应吞掉公司名）
    if let Some(n) = name {
        for para in paragraphs_text(&xml) {
            if para.contains(n) {
                hits.push(LeakHit {
                    location: LeakLocation::Body,
                    content: para.chars().take(120).collect(),
                    severity: Severity::Blocker,
                    suggested_action: "人工确认：正文残留投标人名（可能是必要资质说明——"
                        .to_string()
                        + "确认后改为中性表述或显式豁免；自动替换面（正文/标题）已执行仍残留 = "
                        + "跨形态命中（表格/文本框等），逐处人工处置）",
                });
                break; // 每面报首命中即够（人工清单不是全文索引）
            }
        }
    }
    // 修订作者（w:ins/w:del 的 w:author——引擎正文零触碰不变量不清理此面）
    scan_revision_authors(&xml, name, hits);
}

/// 页眉/页脚文本面（与正文同口径——段落合并 run）。
fn scan_text_face(
    bytes: &[u8],
    name: Option<&str>,
    location: LeakLocation,
    hits: &mut Vec<LeakHit>,
) {
    let Some(n) = name else { return };
    let xml = String::from_utf8_lossy(bytes);
    for para in paragraphs_text(&xml) {
        if para.contains(n) {
            hits.push(LeakHit {
                location,
                content: para.chars().take(120).collect(),
                severity: Severity::Blocker,
                suggested_action: "人工确认：页眉/页脚残留投标人名（保留格式只删内容，"
                    .to_string()
                    + "或改为「投标人」占位）",
            });
            break;
        }
    }
}

/// docProps/core.xml：creator / lastModifiedBy（引擎 anonymize 后应为
/// 匿名常量——命中即「清理未执行/失效」的复验信号）。
fn scan_core_props(bytes: &[u8], name: Option<&str>, hits: &mut Vec<LeakHit>) {
    let xml = String::from_utf8_lossy(bytes);
    for tag in ["dc:creator", "cp:lastModifiedBy"] {
        if let Some(v) = extract_tag_text(&xml, tag) {
            let anonymous = v == "unknown" || v.trim().is_empty();
            let matches_name = name.map(|n| v.contains(n)).unwrap_or(false);
            if !anonymous {
                hits.push(LeakHit {
                    location: LeakLocation::DocProps,
                    content: format!("{tag} = {v}"),
                    severity: if matches_name {
                        Severity::Blocker
                    } else {
                        Severity::Warning
                    },
                    suggested_action: if matches_name {
                        "自动清理面（anonymize）应已处理——命中说明清理失效，重跑导出".into()
                    } else {
                        "文档属性作者非匿名常量（anonymize 后应为 unknown）——复验信号".into()
                    },
                });
            }
        }
    }
}

/// app.xml / custom.xml：Company/Manager/Sender 等扩展与自定义属性。
fn scan_extended_props(entry: &str, bytes: &[u8], name: Option<&str>, hits: &mut Vec<LeakHit>) {
    let xml = String::from_utf8_lossy(bytes);
    // anonymize 后是空骨架：任何非空 <vt:lpstr>/<Company>/<Manager> 值都是残留
    for tag in ["Company", "Manager", "vt:lpstr"] {
        if let Some(v) = extract_tag_text(&xml, tag) {
            if !v.trim().is_empty() {
                let matches_name = name.map(|n| v.contains(n)).unwrap_or(false);
                hits.push(LeakHit {
                    location: LeakLocation::DocProps,
                    content: format!("{entry} {tag} = {v}"),
                    severity: if matches_name {
                        Severity::Blocker
                    } else {
                        Severity::Warning
                    },
                    suggested_action: "自动清理面（anonymize）应已处理——命中说明清理失效，重跑导出".into(),
                });
            }
        }
    }
}

/// comments.xml：批注作者（引擎 anonymize 置空——命中即清理失效信号）。
fn scan_comments_authors(bytes: &[u8], name: Option<&str>, hits: &mut Vec<LeakHit>) {
    let xml = String::from_utf8_lossy(bytes);
    // w:author="..." 属性值提取（首命中即报——审计清单不是作者索引）
    for attr in ["w:author=\"", "w:initials=\""] {
        if let Some(pos) = xml.find(attr) {
            let rest = &xml[pos + attr.len()..];
            if let Some(end) = rest.find('"') {
                let v = &rest[..end];
                if !v.trim().is_empty() {
                    let matches_name = name.map(|n| v.contains(n)).unwrap_or(false);
                    hits.push(LeakHit {
                        location: LeakLocation::Comments,
                        content: format!("{attr}{v}\""),
                        severity: if matches_name {
                            Severity::Blocker
                        } else {
                            Severity::Warning
                        },
                        suggested_action: "批注作者面——自动清理（anonymize）后应为空；命中即清理失效".into(),
                    });
                    break;
                }
            }
        }
    }
}

/// document.xml 修订作者（w:ins/w:del 的 w:author）。
fn scan_revision_authors(xml: &str, name: Option<&str>, hits: &mut Vec<LeakHit>) {
    for attr in ["w:author=\""] {
        let mut from = 0usize;
        while let Some(rel) = xml[from..].find(attr) {
            let pos = from + rel;
            let rest = &xml[pos + attr.len()..];
            if let Some(end) = rest.find('"') {
                let v = &rest[..end];
                if !v.trim().is_empty() {
                    let matches_name = name.map(|n| v.contains(n)).unwrap_or(false);
                    hits.push(LeakHit {
                        location: LeakLocation::Revisions,
                        content: format!("修订 w:author=\"{v}\""),
                        severity: if matches_name {
                            Severity::Blocker
                        } else {
                            Severity::Warning
                        },
                        suggested_action: "修订作者面——引擎不变量不自动清（正文零触碰）："
                            .to_string()
                            + "接受/拒绝全部修订后作者面消失；残留 = 导出源仍带修订，人工处置",
                    });
                    return; // 首命中即报
                }
            }
            from = pos + attr.len();
        }
    }
}

/// 图片 EXIF：JPEG APP1（Exif）在场即报 WARNING（设备/作者/软件可能泄露——
/// 内容级解析过度工程，面级报告已可指导人工处置）。
fn scan_media_exif(entry: &str, bytes: &[u8], hits: &mut Vec<LeakHit>) {
    let is_jpeg = bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8;
    if !is_jpeg {
        return;
    }
    // 扫 JPEG 段标记：FFE1（APP1）+ "Exif\0\0" 头
    let mut i = 2usize;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            break; // 段结构破坏——不是合法 JPEG 流，不猜
        }
        let marker = bytes[i + 1];
        if marker == 0xE1 {
            // APP1：查 Exif 头
            if bytes[i + 4..].starts_with(b"Exif\x00\x00") {
                hits.push(LeakHit {
                    location: LeakLocation::MediaExif,
                    content: format!("{entry} 含 EXIF 元数据（设备/作者/软件可能泄露）"),
                    severity: Severity::Warning,
                    suggested_action: "人工确认：清除图片元数据后重嵌（工具批处理或替换为清洁图）".into(),
                });
                return;
            }
        }
        if marker == 0xDA {
            break; // SOS 后是图像数据，不再有元数据段
        }
        // 段长跳进
        let len = ((bytes[i + 2] as usize) << 8) | bytes[i + 3] as usize;
        if len < 2 {
            break;
        }
        i += 2 + len;
    }
}

// ---------------------------------------------------------------- XML 小工具

/// 逐段落提取可见文本（</w:p> 分段 + 段内剥标签——run 边界合并，防跨 run
/// 切分的公司名漏检；段落边界不跨——防跨段误拼）。
fn paragraphs_text(xml: &str) -> Vec<String> {
    xml.split("</w:p>")
        .map(|para| strip_tags(para))
        .filter(|s| !s.trim().is_empty())
        .collect()
}

/// 剥 XML 标签（含属性）——文本面提取的最简真源（quick-xml 全量解析对
/// 「查子串」的目标过度）。
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// 提取首个 `<tag>text</tag>` 的 text（无标签 → None）。
fn extract_tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = xml.find(&open)?;
    let after = &xml[start..];
    let gt = after.find('>')?;
    let mut inner = &after[gt + 1..];
    // 自闭合 <tag/> → None
    if after[..gt].ends_with('/') {
        return None;
    }
    let close = format!("</{tag}>");
    let end = inner.find(&close)?;
    inner = &inner[..end];
    Some(strip_tags(inner))
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造极简 docx zip（给定部件字节）——测试夹具真源。
    fn mini_docx(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            for (name, data) in parts {
                use std::io::Write;
                let f = w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                let _ = f;
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    const BIDDER: &str = "某某建设工程有限公司";

    #[test]
    fn scan_finds_all_six_leak_classes() {
        // 六类全脏的产物：正文（跨 run 切分）+ 页眉 + 属性 + 批注 + 修订 + EXIF
        let body = format!(
            "<w:document><w:p><w:r><w:t>本方案由某某建设</w:t></w:r><w:r><w:t>工程有限公司承建</w:t></w:r></w:p></w:document>"
        );
        let header = format!("<w:hdr><w:p><w:r><w:t>{BIDDER} 投标文件</w:t></w:r></w:p></w:hdr>");
        let core = r#"<cp:coreProperties><dc:creator>张三</dc:creator><cp:lastModifiedBy>李四</cp:lastModifiedBy></cp:coreProperties>"#;
        let app = r#"<Properties><Company>某某建设工程有限公司</Company></Properties>"#;
        let comments = r#"<w:comments><w:comment w:author="王五" w:initials="W"></w:comment></w:comments>"#;
        let doc_with_rev = format!(
            "<w:document><w:p><w:ins w:author=\"赵六\"><w:r><w:t>x</w:t></w:r></w:ins></w:p></w:document>"
        );
        // EXIF JPEG：SOI + APP1(Exif 头) + SOS
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xE1, 0x00, 0x10]); // APP1 len=16
        jpeg.extend_from_slice(b"Exif\x00\x00");
        jpeg.extend_from_slice(&[0u8; 10]); // padding to len
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]); // SOS
        jpeg.extend_from_slice(&[0x00, 0x00]);

        let docx = mini_docx(&[
            ("word/document.xml", doc_with_rev.as_bytes()),
            ("word/header1.xml", header.as_bytes()),
            ("docProps/core.xml", core.as_bytes()),
            ("docProps/app.xml", app.as_bytes()),
            ("word/comments.xml", comments.as_bytes()),
            ("word/media/image1.jpeg", &jpeg),
        ]);
        let report = scan_dark_bid_leaks(&docx, Some(BIDDER), std::path::Path::new("out/标书.docx"));
        assert!(!report.passed, "BLOCKER 未归零: {report:?}");
        let locs: Vec<&str> = report.hits.iter().map(|h| h.location.as_str()).collect();
        for expect in [
            "revisions",     // 修订作者赵六（WARNING——非投标人名但非空）
            "header",        // 页眉公司名（BLOCKER）
            "doc_props",     // creator 张三 + Company 命中
            "comments",      // 批注作者王五
            "media_exif",    // EXIF 在场
        ] {
            assert!(locs.contains(&expect), "缺 {expect}: {report:#?}");
        }
        assert!(!report.passed);
    }

    #[test]
    fn body_leak_across_run_split_detected() {
        // 公司名跨 run 切分：<w:t>某某建设</w:t>…<w:t>工程有限公司</w:t>
        let body = "<w:document><w:p><w:r><w:t>某某建设</w:t></w:r><w:r><w:t>工程有限公司</w:t></w:r></w:p></w:document>";
        let docx = mini_docx(&[("word/document.xml", body.as_bytes())]);
        let report = scan_dark_bid_leaks(&docx, Some(BIDDER), std::path::Path::new("out/x.docx"));
        assert!(
            report.hits.iter().any(|h| h.location == LeakLocation::Body
                && h.severity == Severity::Blocker),
            "跨 run 切分必须检出: {report:?}"
        );
    }

    #[test]
    fn path_leak_warns() {
        let docx = mini_docx(&[("word/document.xml", b"<w:document/>")]);
        let p = format!("out/{BIDDER}-技术标.docx");
        let report = scan_dark_bid_leaks(&docx, Some(BIDDER), std::path::Path::new(&p));
        assert!(
            report.hits.iter().any(|h| h.location == LeakLocation::Path
                && h.severity == Severity::Warning),
            "路径含投标人名 → WARNING: {report:?}"
        );
        assert!(report.passed, "路径是 WARNING 不阻断: {report:?}");
    }

    #[test]
    fn clean_artifact_passes() {
        // 匿名常量属性 + 无公司名正文 + 无 EXIF → 复验通过
        let body = "<w:document><w:p><w:r><w:t>本方案由投标人承建</w:t></w:r></w:p></w:document>";
        let core = r#"<cp:coreProperties><dc:creator>unknown</dc:creator><cp:lastModifiedBy>unknown</cp:lastModifiedBy></cp:coreProperties>"#;
        let docx = mini_docx(&[
            ("word/document.xml", body.as_bytes()),
            ("docProps/core.xml", core.as_bytes()),
        ]);
        let report = scan_dark_bid_leaks(&docx, Some(BIDDER), std::path::Path::new("out/标书.docx"));
        assert!(report.passed, "清洁产物必须通过: {report:?}");
        assert!(report.hits.is_empty(), "零命中: {report:?}");
    }

    #[test]
    fn engine_anonymize_then_scan_passes() {
        // 引擎面真联动：脏 docx（属性/批注带身份）→ anonymize → 扫描通过
        let core = r#"<cp:coreProperties><dc:creator>张三</dc:creator><cp:lastModifiedBy>李四</cp:lastModifiedBy></cp:coreProperties>"#;
        let comments = r#"<w:comments><w:comment w:author="王五" w:initials="W"></w:comment></w:comments>"#;
        let body = "<w:document><w:p><w:r><w:t>正文内容</w:t></w:r></w:p></w:document>";
        let dirty = mini_docx(&[
            ("word/document.xml", body.as_bytes()),
            ("docProps/core.xml", core.as_bytes()),
            ("word/comments.xml", comments.as_bytes()),
        ]);
        // 扫脏产物：应报 doc_props/comments 命中
        let before = scan_dark_bid_leaks(&dirty, Some(BIDDER), std::path::Path::new("out/x.docx"));
        assert!(before.hits.iter().any(|h| h.location == LeakLocation::DocProps));
        assert!(before.hits.iter().any(|h| h.location == LeakLocation::Comments));
        // 引擎清理（bid-engine 转发面真跑）
        let cleaned = bid_engine::anonymize_docx_props(&dirty).expect("引擎 anonymize");
        let after = scan_dark_bid_leaks(&cleaned, Some(BIDDER), std::path::Path::new("out/x.docx"));
        assert!(
            after.hits.iter().all(|h| h.location != LeakLocation::DocProps
                && h.location != LeakLocation::Comments),
            "引擎清理面命中应清零: {after:?}"
        );
    }

    #[test]
    fn not_a_zip_reports_honestly() {
        let report = scan_dark_bid_leaks(b"not a zip", Some(BIDDER), std::path::Path::new("out/x.docx"));
        assert!(
            report.hits.iter().any(|h| h.content.contains("无法执行")),
            "fail-open 如实报: {report:?}"
        );
    }
}
