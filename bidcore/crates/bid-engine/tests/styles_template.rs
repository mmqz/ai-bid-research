//! #110 任务 2：样式模板接线测试（`styles.rs` 门面）。
//!
//! 夹具策略（受控构造——零 SKIP 面）：
//! - 测试内用 zip dev-dep 构建**最小 OPC docx**（[Content_Types].xml +
//!   _rels/.rels + word/document.xml + word/_rels/document.xml.rels +
//!   word/styles.xml + word/theme/theme1.xml，Stored 压缩）——styles/
//!   theme 内容全受控，断言 = 手写 XML 的确定值，非真实件的统计面；
//! - 引擎三门（预扫/良构/关系层）全过 = 门面在**真实包结构**上的通路
//!   证明（非「绕过门只测解析」）。
//!
//! 断言锚点 = OOXML 字段值直译（半点→磅 / basedOn 链合并 / 主题槽名），
//! 引擎语义面（fonts **整组替换**非逐槽合并——模块文档登记的边界）在
//! `style_chain_merge_replaces_fonts_wholesale` 显式钉定。

use bid_engine::{EngineError, StyleTemplate};

/// 构建测试 docx（字节）——styles_xml 可定制。
fn build_docx(styles_xml: &str, theme_xml: &str) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        let content_types = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
            r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
            r#"<Default Extension="xml" ContentType="application/xml"/>"#,
            r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
            r#"<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>"#,
            r#"<Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#,
            r#"</Types>"#,
        );
        let root_rels = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#,
            r#"</Relationships>"#,
        );
        let doc_rels = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
            r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>"#,
            r#"</Relationships>"#,
        );
        let document = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
            r#"<w:body><w:p/></w:body></w:document>"#,
        );

        for (name, data) in [
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", root_rels),
            ("word/document.xml", document),
            ("word/_rels/document.xml.rels", doc_rels),
            ("word/styles.xml", styles_xml),
            ("word/theme/theme1.xml", theme_xml),
        ] {
            zw.start_file(name, stored).expect("zip start_file");
            zw.write_all(data.as_bytes()).expect("zip write");
        }
        zw.finish().expect("zip finish");
    }
    buf.into_inner()
}

/// 基准 styles.xml：docDefaults（仿宋小四/两端对齐）+ Normal（无覆盖）+
/// BidTitle（basedOn Normal——居中/方正小标宋/二号/粗体）。
fn base_styles() -> String {
    concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        r#"<w:docDefaults>"#,
        r#"<w:rPrDefault><w:rPr>"#,
        r#"<w:rFonts w:ascii="Times New Roman" w:eastAsia="仿宋_GB2312"/>"#,
        r#"<w:sz w:val="24"/>"#,
        r#"</w:rPr></w:rPrDefault>"#,
        r#"<w:pPrDefault><w:pPr><w:jc w:val="both"/></w:pPr></w:pPrDefault>"#,
        r#"</w:docDefaults>"#,
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal">"#,
        r#"<w:name w:val="Normal"/>"#,
        r#"</w:style>"#,
        r#"<w:style w:type="paragraph" w:styleId="BidTitle">"#,
        r#"<w:name w:val="Bid Title"/>"#,
        r#"<w:basedOn w:val="Normal"/>"#,
        r#"<w:pPr><w:jc w:val="center"/></w:pPr>"#,
        r#"<w:rPr>"#,
        r#"<w:rFonts w:eastAsia="方正小标宋简体" w:ascii="Times New Roman"/>"#,
        r#"<w:b/>"#,
        r#"<w:sz w:val="44"/>"#,
        r#"</w:rPr>"#,
        r#"</w:style>"#,
        r#"</w:styles>"#,
    )
    .to_string()
}

/// 基准 theme1.xml：accent1 = 4472C4、dark1 = 000000（sysClr 形态）。
fn base_theme() -> String {
    concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="测试主题">"#,
        r#"<a:themeElements><a:clrScheme name="测试配色">"#,
        r#"<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>"#,
        r#"<a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>"#,
        r#"<a:accent1><a:srgbClr val="4472C4"/></a:accent1>"#,
        r#"</a:clrScheme></a:themeElements></a:theme>"#,
    )
    .to_string()
}

// ---------------------------------------------------------------------------
// 读取通路（引擎三门全过的最小真实包结构）
// ---------------------------------------------------------------------------

/// 最小包结构过引擎三门（预扫/良构/关系层）——门面通路证明。
#[test]
fn style_template_reads_minimal_opc_package() {
    let bytes = build_docx(&base_styles(), &base_theme());
    let tpl = StyleTemplate::from_bytes(&bytes).expect("最小 OPC 包读取成功");
    assert_eq!(tpl.style_ids(), vec!["Normal", "BidTitle"], "出现序保留");
    assert!(tpl.warnings().is_empty(), "良构件零警告");
}

/// 非 docx 字节 → 容器拒绝（`EngineRejected` 分类——与 reader 面
/// 同款安全语义，不伪装成解析失败）。
#[test]
fn style_template_rejects_non_docx_bytes() {
    let junk = b"not a zip at all".to_vec();
    match StyleTemplate::from_bytes(&junk) {
        Err(EngineError::EngineRejected(_)) => {}
        Err(other) => panic!("期望 EngineRejected，实得 EngineFailed: {other:?}"),
        Ok(_) => panic!("垃圾字节竟然读取成功——容器门失效"),
    }
}

// ---------------------------------------------------------------------------
// 有效格式（docDefaults / basedOn 链合并）
// ---------------------------------------------------------------------------

/// docDefaults 投影：无 pStyle 段落的生效面（固定格式件正文的格式基线）。
#[test]
fn style_template_doc_default_projection() {
    let bytes = build_docx(&base_styles(), &base_theme());
    let tpl = StyleTemplate::from_bytes(&bytes).expect("读取成功");
    let def = tpl.doc_default();
    assert_eq!(def.east_asia_font.as_deref(), Some("仿宋_GB2312"));
    assert_eq!(def.ascii_font.as_deref(), Some("Times New Roman"));
    assert_eq!(def.size_pt, Some(12.0), "w:sz 24 半点 = 12pt");
    assert_eq!(def.alignment.as_deref(), Some("both"));
    assert_eq!(def.bold, None, "docDefaults 未设粗体——诚实 None");
}

/// basedOn 链合并：BidTitle 覆盖字体/字号/粗体/对齐，其余继承 docDefaults。
#[test]
fn style_template_effective_merges_based_on_chain() {
    let bytes = build_docx(&base_styles(), &base_theme());
    let tpl = StyleTemplate::from_bytes(&bytes).expect("读取成功");
    let title = tpl.effective("BidTitle").expect("BidTitle 在场");
    assert_eq!(
        title.east_asia_font.as_deref(),
        Some("方正小标宋简体"),
        "派生覆盖"
    );
    assert_eq!(
        title.ascii_font.as_deref(),
        Some("Times New Roman"),
        "自声明（见整组替换测试）"
    );
    assert_eq!(title.size_pt, Some(22.0), "w:sz 44 半点 = 22pt（二号）");
    assert_eq!(title.bold, Some(true), "w:b 在场");
    assert_eq!(title.italic, None, "未设——继承面诚实 None");
    assert_eq!(
        title.alignment.as_deref(),
        Some("center"),
        "段落对齐覆盖 both"
    );
}

/// Normal（链头，无覆盖）= docDefaults 直通。
#[test]
fn style_template_normal_equals_defaults() {
    let bytes = build_docx(&base_styles(), &base_theme());
    let tpl = StyleTemplate::from_bytes(&bytes).expect("读取成功");
    let normal = tpl.effective("Normal").expect("Normal 在场");
    assert_eq!(normal.size_pt, Some(12.0));
    assert_eq!(normal.alignment.as_deref(), Some("both"));
    assert_eq!(normal.bold, None);
}

/// 引擎合并语义钉定（模块文档登记的边界）：`w:rFonts` 是**整组替换**——
/// 派生样式声明 rFonts 时，未声明的槽**不**从下层补（与 OOXML 逐属性
/// 继承不同——引擎 v0 语义，消费方须知：模板样式须全量声明字体组）。
#[test]
fn style_chain_merge_replaces_fonts_wholesale() {
    // 派生样式只声明 eastAsia（不含 ascii）——docDefaults 的 ascii 丢失。
    let styles = base_styles().replace(
        r#"<w:rFonts w:eastAsia="方正小标宋简体" w:ascii="Times New Roman"/>"#,
        r#"<w:rFonts w:eastAsia="方正小标宋简体"/>"#,
    );
    let bytes = build_docx(&styles, &base_theme());
    let tpl = StyleTemplate::from_bytes(&bytes).expect("读取成功");
    let title = tpl.effective("BidTitle").expect("BidTitle 在场");
    assert_eq!(title.east_asia_font.as_deref(), Some("方正小标宋简体"));
    assert_eq!(
        title.ascii_font, None,
        "引擎语义：rFonts 整组替换——docDefaults 的 ascii 不从下层补"
    );
    // 对照：未声明 rFonts 的样式（Normal）不受影响。
    let normal = tpl.effective("Normal").expect("Normal 在场");
    assert_eq!(normal.ascii_font.as_deref(), Some("Times New Roman"));
}

/// 未知样式 id → None（诚实缺失——消费方决定回退，不伪造空格式）。
#[test]
fn style_template_unknown_id_is_none() {
    let bytes = build_docx(&base_styles(), &base_theme());
    let tpl = StyleTemplate::from_bytes(&bytes).expect("读取成功");
    assert!(tpl.effective("NoSuchStyle").is_none());
}

// ---------------------------------------------------------------------------
// 主题色（themeColor token → RRGGBB）
// ---------------------------------------------------------------------------

/// 主题色三面：srgbClr 直值 / sysClr lastClr / token 别名（dark1→dk1）。
#[test]
fn style_template_theme_color_lookup() {
    let bytes = build_docx(&base_styles(), &base_theme());
    let tpl = StyleTemplate::from_bytes(&bytes).expect("读取成功");
    assert_eq!(
        tpl.theme_color("accent1").as_deref(),
        Some("4472C4"),
        "srgbClr 直值（大写归一）"
    );
    assert_eq!(
        tpl.theme_color("dark1").as_deref(),
        Some("000000"),
        "sysClr lastClr + dark1→dk1 映射"
    );
    assert_eq!(
        tpl.theme_color("light1").as_deref(),
        Some("FFFFFF"),
        "light1→lt1 别名"
    );
    assert_eq!(tpl.theme_color("noSuchToken"), None, "未命中诚实 None");
}

/// theme 部件缺席（无主题关系）→ 空主题表 + 零警告（引擎文档语义：
/// 关系缺席 = 无该部件，非 Word 产线常态但非错误）。
#[test]
fn style_template_without_theme_part() {
    let no_theme_rels = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
        r#"</Relationships>"#,
    );
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let content_types = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
            r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
            r#"<Default Extension="xml" ContentType="application/xml"/>"#,
            r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
            r#"<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>"#,
            r#"</Types>"#,
        );
        let root_rels = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#,
            r#"</Relationships>"#,
        );
        let document = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
            r#"<w:body><w:p/></w:body></w:document>"#,
        );
        for (name, data) in [
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", root_rels),
            ("word/document.xml", document),
            ("word/_rels/document.xml.rels", no_theme_rels),
            ("word/styles.xml", &base_styles()),
        ] {
            zw.start_file(name, stored).expect("zip start_file");
            zw.write_all(data.as_bytes()).expect("zip write");
        }
        zw.finish().expect("zip finish");
    }
    let tpl = StyleTemplate::from_bytes(&buf.into_inner()).expect("无 theme 包读取成功");
    assert_eq!(tpl.theme_color("accent1"), None, "空主题表");
    assert_eq!(
        tpl.effective("Normal").map(|f| f.size_pt),
        Some(Some(12.0)),
        "styles 读取不受 theme 缺席影响"
    );
}
