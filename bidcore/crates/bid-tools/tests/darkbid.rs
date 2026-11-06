//! darkbid.rs —— #317 暗标清理验收：扫描 6 类泄露 → 自动清理 → 复验归零。
//!
//! 验收对照（issue）：
//! 1. 扫描能找出 6 类泄露（正文/页眉页脚/属性/图片/修订/批注）——金丝雀纪律
//!    （v1_darkbid D4 同款：泄漏件必须被检出，否则扫描器是摆设）；
//! 2. 自动清理覆盖 docProps + 修订 + 批注；
//! 3. 人工确认项（联系方式/EXIF/路径）列清单不静默跳过；
//! 4. 复验 BLOCKER 归零才放行（残余 BLOCKER → Err——导出面据此拒）；
//! 5. 真导出集成：暗标导出的产物 docProps 清零 + 正文脱敏 + 报告在场
//!    （修前实况：docx-rs 默认 core.xml 是 `unknown` 作者——清零前必命中）。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::assembly::ChapterAssembler;
use bid_tools::darkbid::{self, LeakLocation};
use bid_tools::export::{DocumentExporter, ExportOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

const COMPANY: &str = "某智慧建设工程有限公司";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bid-317-darkbid-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 合成泄漏 docx：六类泄露全注入（金丝雀——扫描器有效性证明，D4 纪律）。
fn leaky_docx() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let document_xml = format!(
            r#"<w:document><w:body><w:p><w:r><w:t>本章由{COMPANY}编制，联系电话 13800001234。</w:t></w:r></w:p>\
<w:p><w:ins w:id="1" w:author="张三" w:date="2026-01-01T00:00:00Z"><w:r><w:t>修订插入内容</w:t></w:r></w:ins></w:p></w:body></w:document>"#
        );
        let header_xml = format!(r#"<w:hdr><w:p><w:r><w:t>{COMPANY}投标文件</w:t></w:r></w:p></w:hdr>"#);
        let core_xml = r#"<cp:coreProperties xmlns:cp="…" xmlns:dc="…"><dc:creator>李四</dc:creator><cp:lastModifiedBy>王五</cp:lastModifiedBy></cp:coreProperties>"#;
        let app_xml = r#"<Properties xmlns="…"><Company>某智慧建设工程有限公司</Company></Properties>"#;
        let comments_xml = r#"<w:comments><w:comment w:id="1" w:author="赵六"><w:p/></w:comment></w:comments>"#;
        let logo_jpg: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x1C, b'E', b'x', b'i', b'f', 0x00, 0x00, 0x00, 0x00];
        let parts: Vec<(&str, &[u8])> = vec![
            ("[Content_Types].xml", br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#),
            ("word/document.xml", document_xml.as_bytes()),
            ("word/header1.xml", header_xml.as_bytes()),
            ("docProps/core.xml", core_xml.as_bytes()),
            ("docProps/app.xml", app_xml.as_bytes()),
            ("word/comments.xml", comments_xml.as_bytes()),
            ("word/media/logo.jpg", logo_jpg),
        ];
        for (name, data) in parts {
            w.start_file(name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }
    buf
}

fn part_of(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut f = zip.by_name(name).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut f, &mut s).unwrap();
    s
}

/// 验收 1+3：六类泄露全检出（BLOCKER 五面 + WARNING 联系方式/EXIF 面）。
#[test]
fn scan_finds_all_six_leak_types() {
    let hits = darkbid::scan(&leaky_docx(), Some(COMPANY), Path::new("/tmp/out/投标文件.docx"));
    let loc = |l: LeakLocation| hits.iter().filter(|h| h.location == l).count();
    assert!(loc(LeakLocation::Body) >= 1, "正文公司名必须命中");
    assert!(loc(LeakLocation::HeaderFooter) >= 1, "页眉公司名必须命中");
    assert!(loc(LeakLocation::DocProps) >= 3, "docProps（creator+lastModifiedBy+Company）必须命中，实际 {}",
        hits.iter().map(|h| format!("{:?}", h.location)).collect::<Vec<_>>().join(","));
    assert!(loc(LeakLocation::RevisionAuthor) >= 1, "修订作者必须命中");
    assert!(loc(LeakLocation::CommentAuthor) >= 1, "批注作者必须命中");
    assert!(loc(LeakLocation::ImageMeta) >= 1, "EXIF 面必须命中");
    // 联系方式 = WARNING（人工确认面，不拦导出）
    assert!(hits.iter().any(|h| h.severity == "WARNING" && h.content.contains("13800001234")),
        "手机号进人工确认清单");
    // 输出路径不含公司名 → 不命中（负面防误报）
    assert!(!hits.iter().any(|h| h.location == LeakLocation::OutputPath));
}

/// 验收 2+4：自动清理后复验 BLOCKER 归零；人工确认清单不静默。
#[test]
fn process_auto_cleans_and_zeroes_blockers() {
    let (cleaned, report) = darkbid::process(&leaky_docx(), Some(COMPANY), Path::new("/tmp/out/投标文件.docx"))
        .expect("六类全注入件经自动清理后 BLOCKER 应归零");
    assert_eq!(report.blockers, 0, "复验 BLOCKER 必须归零");
    // 自动清理动作清单（人读——清了什么）
    assert!(report.cleaned.iter().any(|a| a.contains("core.xml")), "docProps 清零动作在案");
    assert!(report.cleaned.iter().any(|a| a.contains("作者属性匿名化")), "修订/批注作者匿名化在案");
    assert!(report.cleaned.iter().any(|a| a.contains("公司名替换")), "页眉/正文公司名替换在案");
    // 产物核验：core.xml 作者字段为空 / Company 消失 / w:author 清空 / 名字换「投标人」
    let core = part_of(&cleaned, "docProps/core.xml");
    assert!(!core.contains("李四") && !core.contains("王五"), "作者字段清零：{core}");
    assert!(!part_of(&cleaned, "docProps/app.xml").contains("Company>某"), "Company 清零");
    let doc = part_of(&cleaned, "word/document.xml");
    assert!(doc.contains(r#"w:author="""#), "修订作者匿名化");
    assert!(!doc.contains(COMPANY), "正文公司名已替换");
    assert!(doc.contains("投标人"), "替换目标词在案");
    assert!(!part_of(&cleaned, "word/comments.xml").contains("赵六"), "批注作者匿名化");
    // 人工确认清单（WARNING 面——联系方式 + EXIF）不静默跳过
    let manual = report.manual_review();
    assert!(manual.iter().any(|h| h.content.contains("13800001234")), "联系方式进人工清单");
    assert!(manual.iter().any(|h| h.location == LeakLocation::ImageMeta), "EXIF 进人工清单");
}

/// 验收 4 负例：跨 run 切分的正文公司名（单点替换清不掉）→ 复验 Err（导出面据此拒）。
#[test]
fn process_rejects_split_run_residue() {
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        let doc = format!(
            r#"<w:document><w:body><w:p><w:r><w:t>本节由{}</w:t></w:r><w:r><w:t>{}编制</w:t></w:r></w:p></w:body></w:document>"#,
            &COMPANY[..6],
            &COMPANY[6..]
        );
        w.start_file("word/document.xml", opts).unwrap();
        w.write_all(doc.as_bytes()).unwrap();
        w.finish().unwrap();
    }
    let err = darkbid::process(&buf, Some(COMPANY), Path::new("/tmp/o.docx"))
        .expect_err("跨 run 残留必须复验拦截");
    assert!(
        err.iter().any(|h| h.location == LeakLocation::Body && h.severity == "BLOCKER"),
        "残留正文公司名 = BLOCKER（实际：{:?}）",
        err.iter().map(|h| h.location.as_str()).collect::<Vec<_>>()
    );
}

/// 验收 5：真导出集成——暗标导出的产物 docProps 清零 + 正文脱敏 + 报告在场。
/// 修前实况：docx-rs 默认 core.xml 写 `unknown` 作者（清零防线缺失即泄漏）。
#[test]
fn dark_export_artifact_is_clean_and_reported() {
    let dir = temp_dir("e2e");
    let mut store = DomainStore::new();
    store.create_project("暗标验收", "ZFCG-2026-317", "暗标", "t317-key").unwrap();
    store
        .upsert_file("A-招标文件.docx", "招标文件", "aa", 5, 1, 50, 0, ParseStatus::Ok, "engine-docx @ test")
        .unwrap();
    store
        .set_requirements(vec![Requirement {
            id: "REQ-0001".into(),
            category: "技术参数".into(),
            clause_no: Some("5.1".into()),
            anchors: vec![Anchor {
                file: "A-招标文件.docx".into(),
                kind: "table".into(),
                locator: "技术参数表 > 第 1 行".into(),
                excerpt: "存储容量 >=100GB".into(),
                block_idx: None,
                page: None,
                line: None,
            }],
            params: Default::default(),
            source_file: "A-招标文件.docx".into(),
        }])
        .unwrap();
    store
        .set_outline(vec![OutlineNode {
            id: "SEC-0001".into(),
            parent_id: None,
            title: "技术参数响应".into(),
            level: 1,
            order: 1,
            requirement_ids: vec!["REQ-0001".into()],
            target_chars: 500,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        }])
        .unwrap();
    for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
        store.advance_section_status("SEC-0001", to).unwrap();
    }
    let content = format!("本节由{COMPANY}针对所辖条款逐项响应，实施安排与保障措施齐备，内容完整。");
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    asm.commit_section("SEC-0001", &content, vec!["REQ-0001".into()]).unwrap();

    let out = dir.join("暗标-技术标.docx");
    let report = DocumentExporter::new(&store.sections())
        .export_with(
            &dir,
            &out,
            &ExportOptions {
                format_spec: None,
                project_name: "暗标验收".into(),
                anonymize: Some(COMPANY), // 暗标模式（#317：anonymize 在场 = dark）
                extras: None,
                review_banner: None,
            },
        )
        .expect("暗标导出成功（自动清理面覆盖 docProps/正文）");

    // 报告在场且 BLOCKER 归零
    let dark = report.dark_bid.as_ref().expect("暗标导出必须携带 dark_bid 报告");
    assert_eq!(dark.blockers, 0);
    assert!(dark.cleaned.iter().any(|a| a.contains("core.xml")),
        "docx-rs 默认 unknown 作者必须被清零（清理动作在案）");

    // 产物核验：core.xml 作者字段空（修前是 unknown）+ 正文脱敏
    let bytes = std::fs::read(&out).unwrap();
    let core = part_of(&bytes, "docProps/core.xml");
    assert!(!core.contains("unknown"), "docx-rs 默认作者必须被清零：{core}");
    let doc = part_of(&bytes, "word/document.xml");
    assert!(!doc.contains(COMPANY), "正文公司名必须脱敏");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 明标导出不动产物（dark 面不激活——兼容性：既有行为零变化）。
#[test]
fn bright_export_has_no_dark_report() {
    let dir = temp_dir("bright");
    let mut store = DomainStore::new();
    store.create_project("明标验收", "ZFCG-2026-317B", "明标", "t317b-key").unwrap();
    store
        .upsert_file("A.docx", "招标文件", "aa", 5, 1, 50, 0, ParseStatus::Ok, "engine-docx @ test")
        .unwrap();
    store
        .set_requirements(vec![Requirement {
            id: "REQ-0001".into(),
            category: "技术参数".into(),
            clause_no: Some("5.1".into()),
            anchors: vec![Anchor {
                file: "A.docx".into(),
                kind: "para".into(),
                locator: "第1页 行1".into(),
                excerpt: "要求".into(),
                block_idx: None,
                page: None,
                line: None,
            }],
            params: Default::default(),
            source_file: "A.docx".into(),
        }])
        .unwrap();
    store
        .set_outline(vec![OutlineNode {
            id: "SEC-0001".into(),
            parent_id: None,
            title: "响应".into(),
            level: 1,
            order: 1,
            requirement_ids: vec!["REQ-0001".into()],
            target_chars: 500,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        }])
        .unwrap();
    for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
        store.advance_section_status("SEC-0001", to).unwrap();
    }
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    asm.commit_section("SEC-0001", "明标内容：正常响应正文。", vec!["REQ-0001".into()]).unwrap();

    let out = dir.join("明标.docx");
    let report = DocumentExporter::new(&store.sections())
        .export_with(
            &dir,
            &out,
            &ExportOptions {
                format_spec: None,
                project_name: "明标验收".into(),
                anonymize: None, // 明标
                extras: None,
                review_banner: None,
            },
        )
        .expect("明标导出成功");
    assert!(report.dark_bid.is_none(), "明标不激活暗标面");
    // 明标产物保留 docx-rs 默认（unknown）——不越权清理
    let bytes = std::fs::read(&out).unwrap();
    assert!(part_of(&bytes, "docProps/core.xml").contains("unknown"));
    let _ = std::fs::remove_dir_all(&dir);
}
