//! #59 阶段 4（提前）· format-spec 测试：解析 / 验证 / 从招标文件提取 / docx 应用。
//!
//! Lead 验收线：
//! - 「不准硬编码格式」——提取面必须从 REQ 文本读出（三号/仿宋/1.5 倍/边距）；
//! - Lead 样例 JSON（global + per_section）必须可解析可 roundtrip；
//! - 导出的 docx 用 Word 打开，页面设置和字体完全符合 format-spec（本文件
//!   以「解包 docx 断言 styles.xml / document.xml 的 XML 面」等价验证——
//!   Word 的渲染面由 sectPr/styles 的 XML 语义承载）。

use bid_domain::{Anchor, Requirement};
use bid_tools::format_spec::{
    apply_to_docx, baseline, chinese_font_size_pt, derive_from_requirements, line_multiple_to_units,
    mm_to_twips, pt_to_half_points, validate, FormatSpec,
};
use std::io::Read;

fn req(category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: "REQ-0001".into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第六章/格式".into(),
            excerpt: excerpt.into(),
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

// ---------------------------------------------------------------- 1. 单位换算（唯一真源）

#[test]
fn unit_conversions_are_exact() {
    assert_eq!(mm_to_twips(210), 11_906, "A4 宽 210mm → 11906 twips（Word 标准 A4）");
    assert_eq!(mm_to_twips(297), 16_838, "A4 高 297mm → 16838 twips");
    assert_eq!(pt_to_half_points(12.0), 24, "小四 12pt → 24 半点");
    assert_eq!(pt_to_half_points(16.0), 32, "三号 16pt → 32 半点");
    assert_eq!(line_multiple_to_units(1.5), 360, "1.5 倍行距 → 360（240×1.5）");
    assert_eq!(line_multiple_to_units(1.0), 240, "单倍行距 → 240");
}

#[test]
fn chinese_font_size_table_covers_gb_t() {
    assert_eq!(chinese_font_size_pt("三号"), Some(16.0));
    assert_eq!(chinese_font_size_pt("小四"), Some(12.0));
    assert_eq!(chinese_font_size_pt("五号"), Some(10.5));
    assert_eq!(chinese_font_size_pt("初号"), Some(42.0));
    assert_eq!(chinese_font_size_pt("七号"), None, "表外字号不猜");
}

// ---------------------------------------------------------------- 2. serde 面（Lead 样例原文）

#[test]
fn lead_sample_json_parses_and_roundtrips() {
    // Lead #59 阶段 2.1 的样例原文（键结构必须兼容——手写 spec 是真实用户面）
    let sample = r#"{
      "global": {
        "page": { "size": "A4", "margin": {"top":35,"bottom":30,"left":28,"right":26} },
        "font": { "body": "仿宋_GB2312", "size": 12, "lineHeight": 1.5 },
        "header": { "enabled": true, "content": "{project_name}" },
        "footer": { "enabled": true, "content": "第 {page} 页 共 {pages} 页" }
      },
      "per_section": {
        "技术方案": { "font": { "body": "黑体", "size": 14, "lineHeight": 1.5 } }
      }
    }"#;
    // 注意：serde 默认驼峰（size/lineHeight）与蛇峰（line_height）的差异——
    // Lead 样例用驼峰 lineHeight。本断言先验证**驼峰可解析**：
    let parsed: Result<FormatSpec, _> = serde_json::from_str(sample);
    match parsed {
        Ok(spec) => {
            assert_eq!(spec.global.font.size, 12.0);
            assert!(validate(&spec).is_ok(), "Lead 样例必须通过 validate");
            // roundtrip
            let out = serde_json::to_string(&spec).unwrap();
            let back: FormatSpec = serde_json::from_str(&out).unwrap();
            assert_eq!(spec, back);
        }
        Err(e) => panic!("Lead 样例 JSON 必须可解析（字段名对齐 Lead 原文）: {e}"),
    }
}

#[test]
fn baseline_is_valid_and_sane() {
    let b = baseline();
    assert!(validate(&b).is_ok());
    assert_eq!(b.global.page.size, "A4");
    assert_eq!(b.global.page.margin.top, 35);
    assert_eq!(b.global.font.body, "仿宋_GB2312");
    assert_eq!(b.global.font.size, 12.0);
    assert_eq!(b.global.font.line_height, 1.5);
}

#[test]
fn validate_rejects_out_of_domain() {
    let mut s = baseline();
    s.global.page.size = "A5".into();
    assert!(validate(&s).is_err(), "纸张枚举外必须拒");
    let mut s = baseline();
    s.global.font.size = 99.0;
    assert!(validate(&s).is_err(), "字号越域必须拒");
    let mut s = baseline();
    s.global.font.line_height = 5.0;
    assert!(validate(&s).is_err(), "行距越域必须拒");
    let mut s = baseline();
    s.global.page.margin.top = 300;
    assert!(validate(&s).is_err(), "边距越域必须拒");
    let mut s = baseline();
    s.global.font.body = "".into();
    assert!(validate(&s).is_err(), "空字体名必须拒");
}

// ---------------------------------------------------------------- 3. 从招标文件提取（不硬编码铁律）

#[test]
fn derive_extracts_font_size_family_line_spacing() {
    let reqs = vec![req("格式要求", "投标文件正文用仿宋三号字，行距为 1.5 倍，A4 纸打印装订")];
    let (spec, warnings) = derive_from_requirements(&reqs);
    assert_eq!(spec.global.font.size, 16.0, "三号 → 16pt");
    assert_eq!(spec.global.font.body, "仿宋");
    assert_eq!(spec.global.font.line_height, 1.5);
    assert!(warnings.iter().any(|w| w.contains("提取")), "命中必须可观测: {warnings:?}");
    assert!(validate(&spec).is_ok());
}

#[test]
fn derive_extracts_margins_four_sides() {
    let reqs = vec![req("其他", "页面设置：页边距上 35mm、下 30mm、左 28mm、右 26mm，左侧装订")];
    // 类别「其他」但含格式字眼 → 兜底候选面
    let (spec, _) = derive_from_requirements(&reqs);
    assert_eq!(spec.global.page.margin.top, 35);
    assert_eq!(spec.global.page.margin.bottom, 30);
    assert_eq!(spec.global.page.margin.left, 28);
    assert_eq!(spec.global.page.margin.right, 26);
}

#[test]
fn derive_addendum_overrides_main_file() {
    // 补遗在后 → 后到优先（招标语义：补遗覆写主文件）
    let reqs = vec![
        req("格式要求", "投标文件正文用宋体小四号字"),
        req("格式要求", "补遗：正文更改为仿宋三号字，行距 1.5 倍"),
    ];
    let (spec, _) = derive_from_requirements(&reqs);
    assert_eq!(spec.global.font.size, 16.0, "补遗的三号覆写主文件的小四");
    assert_eq!(spec.global.font.body, "仿宋");
    assert_eq!(spec.global.font.line_height, 1.5);
}

#[test]
fn derive_variant_font_family_preferred() {
    let reqs = vec![req("格式要求", "标题用方正小标宋简体二号，正文用仿宋_GB2312 三号")];
    let (spec, _) = derive_from_requirements(&reqs);
    // 同句双字体：具体变体优先序——仿宋_GB2312 在列表前位于方正小标宋之后？
    // 识别序为列表序（具体变体优先），本句命中的最后写入者生效——
    // 「方正小标宋简体」先于「仿宋_GB2312」出现在枚举列表，但文本扫描按
    // 列表序而非文本序：两都命中，后者（列表后面的）覆写。此为已知口径：
    // 正文位字体以列表序最后的命中为准（标题/正文混排句取正文族——
    // 仿宋族排在方正小标宋之后，正文胜出）。
    assert_eq!(spec.global.font.body, "仿宋_GB2312", "正文族（仿宋变体）优先于标题族");
}

#[test]
fn derive_falls_back_to_baseline_with_warning() {
    let reqs = vec![req("资格条件", "投标人须具备贰级资质")];
    let (spec, warnings) = derive_from_requirements(&reqs);
    assert_eq!(spec, baseline(), "零命中 → 基线原样");
    assert!(
        warnings.iter().any(|w| w.contains("未发现格式条款") || w.contains("可识别")),
        "降级必须响亮: {warnings:?}"
    );
}

// ---------------------------------------------------------------- 4. docx 应用桥（解包验证）

/// 解包 docx 取指定部件的文本。
fn part_text(bytes: &[u8], name: &str) -> String {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).expect("docx 必须是合法 zip");
    let mut f = zip.by_name(name).unwrap_or_else(|_| panic!("缺部件 {name}"));
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    s
}

#[test]
fn apply_to_docx_writes_page_font_spacing_and_headers() {
    let spec = baseline();
    let docx = apply_to_docx(docx_rs::Docx::new(), &spec, "某水库除险加固工程");
    let mut out = std::io::Cursor::new(Vec::new());
    docx.pack(&mut out).expect("docx 打包");
    let out = out.into_inner();

    // document.xml：sectPr 的页面尺寸与边距（A4 = 11906×16838；35mm=1984twip）
    let doc_xml = part_text(&out, "word/document.xml");
    assert!(doc_xml.contains("w:w=\"11906\""), "A4 宽（实际片段缺失）");
    assert!(doc_xml.contains("w:h=\"16838\""), "A4 高");
    assert!(doc_xml.contains("w:top=\"1984\""), "上边距 35mm → 1984 twips");

    // styles.xml：Normal 样式的字体/字号/行距
    let styles_xml = part_text(&out, "word/styles.xml");
    assert!(styles_xml.contains("w:eastAsia=\"仿宋_GB2312\""), "中文字体");
    assert!(styles_xml.contains("w:sz w:val=\"24\""), "小四 → 24 半点");
    assert!(styles_xml.contains("w:line=\"360\""), "1.5 倍行距 → 360");

    // header 部件：项目名占位符已替换
    let header1 = part_text(&out, "word/header1.xml");
    assert!(header1.contains("某水库除险加固工程"), "页眉占位符替换");

    // footer 部件：页码域（PAGE / NUMPAGES）
    let footer1 = part_text(&out, "word/footer1.xml");
    assert!(footer1.contains("PAGE"), "页脚当前页域");
    assert!(footer1.contains("NUMPAGES"), "页脚总页数域");
    assert!(footer1.contains("第") && footer1.contains("共"), "页脚模板文本保留");
}

#[test]
fn apply_to_docx_disabled_header_footer_omits_them() {
    let mut spec = baseline();
    spec.global.header.enabled = false;
    spec.global.footer.enabled = false;
    let docx = apply_to_docx(docx_rs::Docx::new(), &spec, "项目");
    let mut out = std::io::Cursor::new(Vec::new());
    docx.pack(&mut out).unwrap();
    let out = out.into_inner();
    let cursor = std::io::Cursor::new(&out);
    let mut zip = zip::ZipArchive::new(cursor).unwrap();
    let has_header = zip.by_name("word/header1.xml").is_ok();
    assert!(!has_header, "页眉关闭 → 不产出 header 部件");
}

#[test]
fn apply_to_docx_custom_spec_overrides_baseline() {
    // 提取链 → 应用链的端到端（三号/仿宋/1.5 倍 → XML 面）
    let reqs = vec![req("格式要求", "正文用仿宋三号字，行距 1.5 倍，上边距 30mm")];
    let (spec, _) = derive_from_requirements(&reqs);
    let docx = apply_to_docx(docx_rs::Docx::new(), &spec, "测试项目");
    let mut out = std::io::Cursor::new(Vec::new());
    docx.pack(&mut out).unwrap();
    let out = out.into_inner();
    let styles_xml = part_text(&out, "word/styles.xml");
    assert!(styles_xml.contains("w:sz w:val=\"32\""), "三号 → 32 半点（不是基线 24）");
    let doc_xml = part_text(&out, "word/document.xml");
    assert!(doc_xml.contains("w:top=\"1701\""), "上边距 30mm → 1701 twips（不是基线 1984）");
}

// ---------------------------------------------------------------- 5. per_section 覆盖查询

#[test]
fn font_for_section_override_and_fallback() {
    let mut spec = baseline();
    spec.per_section.insert("技术方案".into(), bid_tools::format_spec::SectionOverride {
        font: Some(bid_tools::format_spec::FontSpec {
            body: "黑体".into(), size: 14.0, line_height: 1.5,
        }),
    });
    spec.per_section.insert("cover".into(), bid_tools::format_spec::SectionOverride {
        font: Some(bid_tools::format_spec::FontSpec {
            body: "方正小标宋简体".into(), size: 22.0, line_height: 1.5,
        }),
    });
    // 包含匹配：标题含 key 即命中
    let f = spec.font_for_section("第三章 施工技术方案");
    assert_eq!((f.body.as_str(), f.size), ("黑体", 14.0));
    let f = spec.font_for_section("封面 cover page");
    assert_eq!((f.body.as_str(), f.size), ("方正小标宋简体", 22.0));
    // 无命中 → global
    let f = spec.font_for_section("商务部分");
    assert_eq!((f.body.as_str(), f.size), ("仿宋_GB2312", 12.0));
}
