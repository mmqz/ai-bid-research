//! export_embed.rs —— #68 阶段 3：图片与表格嵌入验收（issue §3.1/§3.2）。
//!
//! 验收面：
//! - md 表格语法 → docx 表格（`<w:tbl>` 在位 + 表头/单元格文本 + 分隔行
//!   `---` 不进正文）；
//! - md 图片语法 → docx 内嵌图片（zip 内 `word/media/` 在位 + `<w:drawing>`
//!   + 图注「图：说明」）；
//! - 解析面：相对路径（workspace 根）/ EV-id（证据台账 → `files/` 约定）/
//!   绝对路径三分支；
//! - **图片缺失/非图片/未知 EV → 响亮拒绝不静默**（issue §3.1 验收线——
//!   标书缺资质证书图是废标级事故）；
//! - 版面适配：超宽图（2000px）等比缩到内容宽 576px（extent cx = 5486400
//!   EMU——A4 安全面）；
//! - md 标题行 `#`/`##` → 加粗段（不再字面输出 `#`——生成器输出的 md
//!   标题不再以字面井号进文档）。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::assembly::ChapterAssembler;
use bid_tools::export::{DocumentExporter, ExportError};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 夹具

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bid-s3-embed-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 生成 PNG 字节（image 正则依赖面——格式真实，非手拼字节）。
fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    buf.into_inner()
}

/// 登记证据（返回实际签发的 EV id——全局序列不可硬编码）。
fn register_evidence(store: &mut DomainStore, source_file: &str) -> String {
    let (eid, _) = store.register_evidence(EvidenceSpec {
        kind: EvidenceKind::Qualification,
        title: format!("{source_file} 对应证书"),
        source_file: source_file.into(),
        source_sha256: format!("sha-{source_file}"),
        locator: EngineLocator::DocxParagraph {
            paragraph_index: 1,
            text_fingerprint: "fnv64:1".into(),
        },
        fingerprint: "fnv64:1".into(),
        valid_from: None,
        valid_until: None,
        subject_name: "某建设工程有限公司".into(),
        scope: "本标段".into(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    }).unwrap();
    eid
}

/// 单叶基座：1 REQ + 1 叶章（SEC-0001），叶已提交 content 并 DRAFTED。
/// 返回 (store, dir)。
fn committed_workspace(tag: &str, content: &str) -> (DomainStore, PathBuf) {
    let dir = temp_dir(tag);
    let mut store = DomainStore::new();
    store.create_project("嵌入测试", "ZFCG-2026-S3-001", "明标", "s3-key").unwrap();
    store.upsert_file("A-招标文件.docx", "招标文件", "aa", 5, 1, 50, 0,
                      ParseStatus::Ok, "engine-docx @ test").unwrap();
    store.set_requirements(vec![Requirement {
        id: "REQ-0001".into(),
        category: "技术参数".into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第二章/REQ-0001".into(),
            excerpt: "REQ-0001 的唯一原文摘录".into(),
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }]).unwrap();
    store.set_outline(vec![OutlineNode {
        id: "SEC-0001".into(),
        parent_id: None,
        title: "资质与业绩响应".into(),
        level: 1,
        order: 1,
        requirement_ids: vec!["REQ-0001".into()],
        target_chars: 500,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }]).unwrap();
    for to in [SectionStatus::Outlining, SectionStatus::Drafting, SectionStatus::Drafted] {
        store.advance_section_status("SEC-0001", to).unwrap();
    }
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    asm.commit_section("SEC-0001", content, vec!["REQ-0001".into()]).unwrap();
    (store, dir)
}

/// 导出并解包 document.xml（zip dev-dependency——良构与部件面由
/// export_stream.rs 全量覆盖，此处只取文本断言面）。
fn export_and_read_xml(store: &DomainStore, dir: &Path) -> (String, Vec<String>) {
    let out = dir.join("标书.docx");
    DocumentExporter::with_evidences(store.sections(), store.evidences())
        .export(dir, &out)
        .expect("导出应成功");
    let f = std::fs::File::open(&out).unwrap();
    let mut zip = zip::ZipArchive::new(f).unwrap();
    let mut xml = String::new();
    let mut media: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let name = zip.by_index(i).unwrap().name().to_string();
        if name == "word/document.xml" {
            use std::io::Read;
            zip.by_index(i).unwrap().read_to_string(&mut xml).unwrap();
        }
        if name.starts_with("word/media/") && !name.ends_with('/') {
            media.push(name); // 目录条目（word/media/）不计
        }
    }
    assert!(!xml.is_empty(), "document.xml 必须在包内");
    (xml, media)
}

// ---------------------------------------------------------------- 表格

#[test]
fn md_table_renders_to_docx_table() {
    let content = "业绩一览如下：\n\n| 项目名称 | 合同额（万元） | 完成年份 |\n| --- | --- | --- |\n| 甲水库除险加固 | 3200 | 2024 |\n| 乙灌区信息化 | 1800 | 2025 |\n\n以上业绩均可在全国建筑市场监管平台查验。";
    let (store, dir) = committed_workspace("table", content);
    let (xml, media) = export_and_read_xml(&store, &dir);
    assert!(xml.contains("<w:tbl>"), "docx 表格元素在位");
    assert!(xml.contains("项目名称"), "表头文本在位");
    assert!(xml.contains("甲水库除险加固"), "数据行文本在位");
    assert!(xml.contains("乙灌区信息化"), "数据行文本在位");
    assert!(!xml.contains("| --- |"), "分隔行是排版指令不进正文");
    assert!(!xml.contains("|"), "竖线字符不残留");
    assert!(media.is_empty(), "纯表格无媒体件");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn table_without_separator_falls_back_to_paragraphs() {
    // 无分隔行的「表格」不是表格——原样回退段落（不吞内容）
    let content = "| 项目 | 金额 |\n| 甲项目 | 100 |";
    let (store, dir) = committed_workspace("notable", content);
    let (xml, _) = export_and_read_xml(&store, &dir);
    assert!(!xml.contains("<w:tbl>"), "无分隔行不识别为表格");
    assert!(xml.contains("项目"), "内容不丢");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 图片

#[test]
fn image_from_relative_path_embedded_with_caption() {
    let dir0 = temp_dir("img-rel-prep");
    let content = "资质证书扫描件如下：\n\n![施工总承包贰级资质证书](evidence/证书.png)\n\n特此响应。";
    let (store, dir) = committed_workspace("img-rel", content);
    // 图片放 workspace 相对路径 evidence/证书.png
    std::fs::create_dir_all(dir.join("evidence")).unwrap();
    std::fs::write(dir.join("evidence/证书.png"), png_bytes(120, 80)).unwrap();
    let _ = dir0; // （同夹具独立 tag——避免跨测试同名）
    let (xml, media) = export_and_read_xml(&store, &dir);
    assert_eq!(media.len(), 1, "恰好一个媒体件: {media:?}");
    assert!(media[0].ends_with(".png"), "PNG 通道: {media:?}");
    assert!(xml.contains("<w:drawing>"), "drawing 元素在位");
    assert!(xml.contains("图：施工总承包贰级资质证书"), "图注（alt）在位");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn image_from_ev_id_resolves_via_files_convention() {
    let content_pref = "证书原件见下图：\n\n![资质证书](";
    // 先建 store 拿 EV id（内容须含真实 id——两段式）
    let (mut store, dir) = committed_workspace(
        "img-ev",
        "占位（将重建内容）",
    );
    let ev = register_evidence(&mut store, "资质证书扫描件.png");
    let content = format!("{content_pref}{ev})", );
    // 重提交内容（attempt+1——commit_section 的换版通道）
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        asm.commit_section("SEC-0001", &content, vec!["REQ-0001".into()]).unwrap();
    }
    // EV → files/{source_file} 约定位
    std::fs::create_dir_all(dir.join("files")).unwrap();
    std::fs::write(dir.join("files/资质证书扫描件.png"), png_bytes(90, 60)).unwrap();
    let (xml, media) = export_and_read_xml(&store, &dir);
    assert_eq!(media.len(), 1, "EV-id 解析出媒体件: {media:?}");
    assert!(xml.contains("<w:drawing>"), "drawing 在位");
    assert!(xml.contains("图：资质证书"), "图注在位");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wide_image_scaled_to_content_width() {
    let content = "![宽图](evidence/wide.png)";
    let (store, dir) = committed_workspace("img-wide", content);
    std::fs::create_dir_all(dir.join("evidence")).unwrap();
    std::fs::write(dir.join("evidence/wide.png"), png_bytes(2000, 1000)).unwrap();
    let (xml, _) = export_and_read_xml(&store, &dir);
    // 576px × 9525 EMU/px = 5486400（A4 内容宽安全面）
    assert!(xml.contains("cx=\"5486400\""), "宽图缩到内容宽（576px 面）: 需含 cx=5486400");
    assert!(xml.contains("cy=\"2743200\""), "高等比（288px 面）");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- fail-closed

#[test]
fn missing_image_is_loud_not_silent() {
    let content = "![不存在的图](evidence/缺失.png)";
    let (store, dir) = committed_workspace("img-missing", content);
    let out = dir.join("标书.docx");
    let err = DocumentExporter::with_evidences(store.sections(), store.evidences())
        .export(&dir, &out)
        .err()
        .expect("缺图必须报错");
    match &err {
        ExportError::ImageUnresolvable { target, why } => {
            assert_eq!(target, "evidence/缺失.png");
            assert!(why.contains("缺失.png"), "错误含路径: {why}");
        }
        other => panic!("期望 ImageUnresolvable，实际: {other}"),
    }
    assert!(err.to_string().contains("不静默"), "错误文案明示纪律面");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_ev_id_is_loud() {
    let content = "![幻灯](EV-9999)";
    let (store, dir) = committed_workspace("img-ghost", content);
    let out = dir.join("标书.docx");
    let err = DocumentExporter::with_evidences(store.sections(), store.evidences())
        .export(&dir, &out)
        .err()
        .expect("未知 EV 必须报错");
    assert!(matches!(err, ExportError::ImageUnresolvable { .. }), "{err}");
    assert!(err.to_string().contains("EV-9999"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_image_file_is_loud() {
    let content = "![文本伪装图](evidence/说明.txt)";
    let (store, dir) = committed_workspace("img-txt", content);
    std::fs::create_dir_all(dir.join("evidence")).unwrap();
    std::fs::write(dir.join("evidence/说明.txt"), "这不是图片".repeat(100)).unwrap();
    let out = dir.join("标书.docx");
    let err = DocumentExporter::with_evidences(store.sections(), store.evidences())
        .export(&dir, &out)
        .err()
        .expect("非图片必须报错");
    assert!(matches!(err, ExportError::ImageUnresolvable { .. }), "{err}");
    assert!(err.to_string().contains("不是可识别的图片"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 整册综合

#[test]
fn full_bid_with_images_and_tables_exports() {
    // issue §3 验收线原文：「实测：带图带表的标书能正常导出」——图 + 表 +
    // 混排段落同册，产物良构且要素齐备。
    let (mut store, dir) = committed_workspace(
        "full",
        "## 资质响应

我方具备施工总承包贰级资质，证书扫描件如下：

![资质证书](evidence/cert.png)

## 业绩一览

| 项目名称 | 合同额（万元） | 完成年份 |\n| --- | --- | --- |\n| 甲水库除险加固 | 3200 | 2024 |\n| 乙灌区信息化 | 1800 | 2025 |\n\n以上业绩真实可查。".replace("\\n", "\n").as_str(),
    );
    let ev = register_evidence(&mut store, "获奖证书.png");
    let content = format!(
        "## 资质响应\n\n我方具备施工总承包贰级资质，证书扫描件如下：\n\n![资质证书](evidence/cert.png)\n\n近年获奖情况：\n\n![获奖证书]({ev})\n\n## 业绩一览\n\n| 项目名称 | 合同额（万元） | 完成年份 |\n| --- | --- | --- |\n| 甲水库除险加固 | 3200 | 2024 |\n| 乙灌区信息化 | 1800 | 2025 |\n\n以上业绩真实可查。"
    );
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        asm.commit_section("SEC-0001", &content, vec!["REQ-0001".into()]).unwrap();
    }
    std::fs::create_dir_all(dir.join("evidence")).unwrap();
    std::fs::write(dir.join("evidence/cert.png"), png_bytes(300, 200)).unwrap();
    std::fs::create_dir_all(dir.join("files")).unwrap();
    std::fs::write(dir.join("files/获奖证书.png"), png_bytes(160, 120)).unwrap();
    let out = dir.join("成品标书.docx");
    let report = DocumentExporter::with_evidences(store.sections(), store.evidences())
        .export(&dir, &out)
        .expect("带图带表整册导出");
    assert_eq!(report.chapters_written, 1);
    assert!(out.is_file() && std::fs::metadata(&out).unwrap().len() > 0);
    // 结构面：2 图 + 1 表 + 2 图注齐备
    let f = std::fs::File::open(&out).unwrap();
    let mut zip = zip::ZipArchive::new(f).unwrap();
    let mut xml = String::new();
    let mut media = 0usize;
    for i in 0..zip.len() {
        let mut z = zip.by_index(i).unwrap();
        if z.name() == "word/document.xml" {
            use std::io::Read;
            z.read_to_string(&mut xml).unwrap();
        }
        if z.name().starts_with("word/media/") && !z.name().ends_with('/') {
            media += 1;
        }
    }
    assert_eq!(media, 2, "两张图（路径 + EV-id 各一）");
    assert!(xml.contains("<w:tbl>"));
    assert_eq!(xml.matches("<w:drawing>").count(), 2, "两个 drawing");
    assert!(xml.contains("图：资质证书") && xml.contains("图：获奖证书"), "图注齐备");
    assert!(xml.contains("甲水库除险加固") && xml.contains("乙灌区信息化"), "表格数据齐备");
    // 300px/160px 均在 576px 界内——不缩放（原尺寸面）
    assert!(!xml.contains("5486400"), "界内图不触发缩放");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 标题行

#[test]
fn heading_lines_render_bold_without_literal_hash() {
    let content = "# 资质响应\n\n## 营业执照\n\n正文段。";
    let (store, dir) = committed_workspace("heading", content);
    let (xml, _) = export_and_read_xml(&store, &dir);
    assert!(!xml.contains("# 资质响应"), "md 标题不再字面输出井号");
    assert!(!xml.contains("## 营业执照"), "二级标题同");
    assert!(xml.contains("资质响应"), "标题文本在位");
    assert!(xml.contains("营业执照"), "二级标题文本在位");
    assert!(xml.contains("<w:b/>") || xml.contains("<w:b "), "加粗标记在位");
    let _ = std::fs::remove_dir_all(&dir);
}
