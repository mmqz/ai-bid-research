//! tables_export.rs —— #103 子任务 B：六种表格 → markdown → docx 真表格 验收。
//!
//! 验收面对齐 issue §子任务 B：
//! - 6 种表格都能生成 markdown（结构可断言：表头 / 分隔行 / 列数）；
//! - 导出 docx 后是真表格（`<w:tbl>` / `<w:tr>` 在位、单元格文本齐全）；
//! - **技术参数偏离表：负偏离标红**（`<w:color w:val="FF0000"/>`），
//!   正偏离 / 无偏离不染红；
//! - 用真实招标技术要求自动生成偏离表草案（`TechDeviationRow::from_param`
//!   —— 偏离判定复用 `bid_domain::param::derive_deviation` 单一真源）；
//! - 单元格内的 `|` 不会把表格切成错位列；
//! - 暗标模式下表格单元格同步脱敏。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::param::Deviation;
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::assembly::ChapterAssembler;
use bid_tools::export::{DocumentExporter, ExportOptions};
use bid_tools::tables::{
    BidTable, EquipmentRow, PerformanceRow, PersonnelRow, PriceRow, TechDeviationRow,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 夹具

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bid-103b-tables-{tag}-{}-{}",
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

/// 单叶工作区：1 REQ + 1 叶章（内容 = 传入的 markdown），叶已 DRAFTED。
fn committed_workspace(tag: &str, content: &str) -> (DomainStore, PathBuf) {
    let dir = temp_dir(tag);
    let mut store = DomainStore::new();
    store
        .create_project("表格验收", "ZFCG-2026-103B", "明标", "t103b-key")
        .unwrap();
    store
        .upsert_file(
            "A-招标文件.docx",
            "招标文件",
            "aa",
            5,
            1,
            50,
            0,
            ParseStatus::Ok,
            "engine-docx @ test",
        )
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
    for to in [
        SectionStatus::Outlining,
        SectionStatus::Drafting,
        SectionStatus::Drafted,
    ] {
        store.advance_section_status("SEC-0001", to).unwrap();
    }
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    asm.commit_section("SEC-0001", content, vec!["REQ-0001".into()])
        .unwrap();
    (store, dir)
}

/// 读取 docx 包内的 document.xml。
fn read_document_xml(out: &Path) -> String {
    let f = std::fs::File::open(out).unwrap();
    let mut zip = zip::ZipArchive::new(f).unwrap();
    let mut xml = String::new();
    for i in 0..zip.len() {
        if zip.by_index(i).unwrap().name() == "word/document.xml" {
            use std::io::Read;
            zip.by_index(i).unwrap().read_to_string(&mut xml).unwrap();
        }
    }
    assert!(!xml.is_empty(), "document.xml 必须在包内");
    xml
}

/// 导出并解包 document.xml。
fn export_and_read_xml(store: &DomainStore, dir: &Path) -> String {
    let out = dir.join("标书.docx");
    DocumentExporter::with_evidences(store.sections(), store.evidences())
        .export(dir, &out)
        .expect("导出应成功");
    read_document_xml(&out)
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// docx-rs 序列化形态：`<w:color w:val="FF0000" />`（`/>` 前有空格）。
const RED_XML: &str = "<w:color w:val=\"FF0000\" />";

/// 一份覆盖六种表的完整章正文（模拟生成器把表格写进正文）。
fn all_six_tables_markdown() -> String {
    let tables = vec![
        BidTable::ProjectOverview {
            rows: vec![
                ("项目名称".into(), "某水库除险加固工程".into()),
                ("招标编号".into(), "ZFCG-2026-103B".into()),
                ("计划工期".into(), "180 日历天".into()),
            ],
        },
        BidTable::TechDeviation {
            rows: vec![
                TechDeviationRow::new(1, "存储容量 >=100GB", "120GB", Deviation::Positive),
                TechDeviationRow::new(2, "质保年限 不低于 3 年", "2年", Deviation::Negative),
                TechDeviationRow::new(3, "并发用户数 不低于 1000 人", "1,000 人", Deviation::None),
            ],
        },
        BidTable::Personnel {
            rows: vec![
                PersonnelRow::new("项目经理", "张三", "高级工程师/一级建造师", "水利水电", 12),
                PersonnelRow::new("技术负责人", "李四", "高级工程师", "水工结构", 9),
            ],
        },
        BidTable::Equipment {
            rows: vec![
                EquipmentRow::new("挖掘机", "PC200", 2, "台"),
                EquipmentRow::new("闸阀", "DN100|PN16", 8, "套"),
            ],
        },
        BidTable::Performance {
            rows: vec![PerformanceRow::new(
                "甲水库除险加固",
                "某市水务局",
                3200.0,
                "2023-03",
                "2024-06",
            )],
        },
        BidTable::PriceBreakdown {
            rows: vec![
                PriceRow::new(1, "建筑工程", "项", 1.0, 8_000_000.0),
                PriceRow::new(2, "安装调试", "项", 1.0, 250_000.0),
            ],
        },
    ];
    let mut md = String::new();
    for t in &tables {
        md.push_str(&t.to_markdown_titled());
        md.push_str("\\n\\n");
    }
    md
}
// ---------------------------------------------------------------- 测试

/// 验收 1：六种表格 → markdown 结构正确（每种都有标题 + 表头 + 分隔行）。
#[test]
fn all_six_tables_produce_well_formed_markdown() {
    let md = all_six_tables_markdown();
    for kind in [
        "项目概况表",
        "技术参数偏离表",
        "人员配置表",
        "设备清单表",
        "业绩一览表",
        "报价明细表",
    ] {
        assert!(md.contains(&format!("**{kind}**")), "标题在位: {kind}");
    }
    // 6 张表 = 6 条分隔行（GFM 表头分隔行的特征形态）
    let separators = md.lines().filter(|l| l.starts_with("| --- |")).count();
    assert_eq!(separators, 6, "六张表各一条分隔行");
    // 行数账：6 个表头 + 概览 3 + 偏离 3 + 人员 2 + 设备 2 + 业绩 1 + 报价(2+合计 1)
    let data_rows = md
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| --- |"))
        .count();
    assert_eq!(data_rows, 20, "表头 6 + 数据 13 + 报价合计 1 = 20，实得 {data_rows}");
}

/// 验收 2：导出 docx 后是**真表格**（`<w:tbl>` / `<w:tr>`，不是一堆段落）。
#[test]
fn exported_docx_contains_real_tables_not_paragraphs() {
    let content = all_six_tables_markdown();
    let (store, dir) = committed_workspace("realtables", &content);
    let xml = export_and_read_xml(&store, &dir);
    assert_eq!(count(&xml, "<w:tbl>"), 6, "恰好 6 张真表");
    // 表行数 = 6 表头 + (3+3+2+2+1+2) 数据行 + 1 报价合计行 = 20
    assert_eq!(count(&xml, "<w:tr>"), 20, "表行数（含报价合计行）");
    assert!(xml.contains("某水库除险加固工程"), "项目概况数据在位");
    assert!(xml.contains("PC200"), "设备数据在位");
    assert!(xml.contains("甲水库除险加固"), "业绩数据在位");
    assert!(xml.contains("8,000,000.00"), "报价千分位在位");
    assert!(xml.contains("8,250,000.00"), "报价合计行在位");
    assert!(!xml.contains("| --- |"), "分隔行是排版指令不进正文");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 验收 3（★核心）：技术参数偏离表——负偏离标红，正/无偏离不染红。
#[test]
fn negative_deviation_cell_is_red_in_docx() {
    let content = BidTable::TechDeviation {
        rows: vec![
            TechDeviationRow::new(1, "存储容量 >=100GB", "120GB", Deviation::Positive),
            TechDeviationRow::new(2, "质保年限 不低于 3 年", "2年", Deviation::Negative),
            TechDeviationRow::new(3, "并发用户数 不低于 1000 人", "1,000 人", Deviation::None),
        ],
    }
    .to_markdown();
    let (store, dir) = committed_workspace("redneg", &content);
    let xml = export_and_read_xml(&store, &dir);
    assert_eq!(count(&xml, "<w:tbl>"), 1, "一张偏离表");
    let red = count(&xml, RED_XML);
    assert_eq!(red, 5, "负偏离整行 5 个单元格全部标红，实得 {red}");
    // 负偏离的响应值确实落在红色 run 里
    let idx_neg = xml.find(">2年<").expect("负偏离响应值在文档里");
    let before = &xml[idx_neg.saturating_sub(300)..idx_neg];
    assert!(before.contains("FF0000"), "负偏离单元格是红的: ...{before}");
    // 无偏离的响应值不带颜色
    let idx_none = xml.find(">1,000 人<").expect("无偏离响应在位");
    let around = &xml[idx_none.saturating_sub(300)..idx_none];
    assert!(!around.contains("FF0000"), "无偏离不得染红: ...{around}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 验收 4：真实招标技术要求 → 偏离表草案（偏离判定复用 domain 单一真源）。
#[test]
fn deviation_draft_from_real_technical_requirements() {
    // 与 fixtures/synth-tender 同款句式（「>=」「不低于」「不大于」「满足」）
    let required: [(&str, &str); 5] = [
        ("存储容量", ">=100GB"),
        ("并发用户数", "不低于 1000 人"),
        ("质保年限", "不低于 3 年"),
        ("响应时间", "不大于 2 秒"),
        ("兼容性", "兼容主流操作系统"),
    ];
    let responses = ["120GB", "800 人", "5年", "1 秒", "满足"];
    let rows: Vec<TechDeviationRow> = required
        .iter()
        .zip(responses.iter())
        .enumerate()
        .map(|(i, ((name, req), resp))| TechDeviationRow::from_param(i + 1, name, req, resp))
        .collect();
    let table = BidTable::TechDeviation { rows };
    // 两条负偏离，都是 domain 单一真源（derive_deviation）判出来的：
    //   1. 800 人 < 1000 人 —— 正确（确实不满足）；
    //   2. 1 秒 vs 「不大于 2 秒」—— **数值主判只比大小、不看方向词**，
    //      把「1 < 2」判成负偏离。这是 param.rs 已登记的已知限制（模块头
    //      「方向敏感参数留待 T-7 加 direction 覆写面」）——本测试如实
    //      断言 2 条，不为了好看去改上游判定（改了就是掩盖真实行为）。
    assert_eq!(table.negative_count(), 2, "两条负偏离（含方向词已知限制那一条）");
    let md = table.to_markdown();
    assert!(md.contains("负偏离"), "负偏离标签在位");
    assert!(md.contains("{{r:"), "负偏离标红标记在位");
    let (store, dir) = committed_workspace("draftdev", &md);
    let xml = export_and_read_xml(&store, &dir);
    assert_eq!(count(&xml, "<w:tbl>"), 1);
    assert_eq!(count(&xml, RED_XML), 10, "两条负偏离 × 5 格 = 10 个红 run");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 验收 5：单元格内的 `|` 不破坏表格结构（不切成错位列）。
#[test]
fn cell_pipe_does_not_break_table_columns() {
    let content = BidTable::Equipment {
        rows: vec![
            EquipmentRow::new("闸阀", "DN100|PN16", 8, "套"),
            EquipmentRow::new("球阀", "DN50|PN10", 4, "套"),
        ],
    }
    .to_markdown();
    let (store, dir) = committed_workspace("pipes", &content);
    let xml = export_and_read_xml(&store, &dir);
    assert_eq!(count(&xml, "<w:tbl>"), 1, "仍是一张表");
    assert_eq!(count(&xml, "<w:tr>"), 3, "3 行（表头 + 2 数据）");
    assert!(xml.contains("DN100|PN16"), "竖线内容原样保留（不丢字）");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 验收 6：暗标模式下表格单元格同步脱敏（公司名 → 投标人）。
#[test]
fn anonymized_table_cells_scrub_company_name() {
    let content = BidTable::Performance {
        rows: vec![PerformanceRow::new(
            "甲水库除险加固（某建设工程有限公司承建）",
            "某市水务局",
            3200.0,
            "2023-03",
            "2024-06",
        )],
    }
    .to_markdown();
    let (store, dir) = committed_workspace("anon", &content);
    let out = dir.join("标书暗标.docx");
    DocumentExporter::with_evidences(store.sections(), store.evidences())
        .export_with(
            &dir,
            &out,
            &ExportOptions {
                anonymize: Some("某建设工程有限公司"),
                project_name: "ZFCG-2026-103B".into(),
                ..Default::default()
            },
        )
        .expect("暗标导出应成功");
    let xml = read_document_xml(&out);
    assert!(!xml.contains("某建设工程有限公司"), "公司名不得出现在暗标里");
    assert!(xml.contains("投标人"), "替换为「投标人」");
    assert_eq!(count(&xml, "<w:tbl>"), 1, "脱敏后仍是真表格");
    let _ = std::fs::remove_dir_all(&dir);
}
