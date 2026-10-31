//! tables_sample.rs —— 生成一份含六种表格的标书 docx 样张（人工验收用）。
//!
//! 用法：`cargo test -p bid-tools --test tables_sample -- --nocapture`
//! 产物：`%TEMP%\bid-tables-sample\标书样张.docx`（可直接用 Word/LibreOffice 打开）

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::param::Deviation;
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::assembly::ChapterAssembler;
use bid_tools::export::DocumentExporter;
use bid_tools::tables::{
    BidTable, EquipmentRow, PerformanceRow, PersonnelRow, PriceRow, TechDeviationRow,
};
use std::path::PathBuf;

fn out_dir() -> PathBuf {
    std::env::temp_dir().join("bid-tables-sample")
}

#[test]
fn generate_sample_with_all_six_tables() {
    let dir = out_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let tables = vec![
        BidTable::ProjectOverview {
            rows: vec![
                ("项目名称".into(), "某水库除险加固工程".into()),
                ("招标编号".into(), "ZFCG-2026-103B".into()),
                ("招标人".into(), "某市水务局".into()),
                ("计划工期".into(), "180 日历天".into()),
                ("质量目标".into(), "合格".into()),
            ],
        },
        BidTable::TechDeviation {
            rows: vec![
                TechDeviationRow::new(1, "存储容量 ≥100GB", "120GB", Deviation::Positive),
                TechDeviationRow::new(2, "并发用户数 不低于 1000 人", "1,000 人", Deviation::None),
                TechDeviationRow::with_note(
                    3,
                    "质保年限 不低于 3 年",
                    "2年",
                    Deviation::Negative,
                    "拟增补两年延保，见偏差说明",
                ),
                TechDeviationRow::new(4, "兼容性 兼容主流操作系统", "满足", Deviation::None),
                TechDeviationRow::new(5, "响应时间 不大于 2 秒", "1.5 秒", Deviation::Incomparable),
            ],
        },
        BidTable::Personnel {
            rows: vec![
                PersonnelRow::new("项目经理", "张三", "高级工程师/一级建造师", "水利水电", 12),
                PersonnelRow::new("技术负责人", "李四", "高级工程师", "水工结构", 9),
                PersonnelRow::new("安全员", "王五", "工程师/安全员证", "水利水电", 6),
            ],
        },
        BidTable::Equipment {
            rows: vec![
                EquipmentRow::new("挖掘机", "PC200", 2, "台"),
                EquipmentRow::new("闸阀", "DN100|PN16", 8, "套"),
                EquipmentRow::new("混凝土搅拌站", "HZS60", 1, "座"),
            ],
        },
        BidTable::Performance {
            rows: vec![
                PerformanceRow::new("甲水库除险加固", "某市水务局", 3200.0, "2023-03", "2024-06"),
                PerformanceRow::new("乙灌区信息化", "某县水利局", 1800.0, "2024-01", "2025-02"),
            ],
        },
        BidTable::PriceBreakdown {
            rows: vec![
                PriceRow::new(1, "建筑工程", "项", 1.0, 8_000_000.0),
                PriceRow::new(2, "安装工程", "项", 1.0, 2_150_000.0),
                PriceRow::new(3, "设备购置", "批", 1.0, 3_600_000.5),
                PriceRow::new(4, "其他费用", "项", 1.0, 250_000.0),
            ],
        },
    ];

    let mut md = String::new();
    md.push_str("# 第三章 商务与技术响应\n\n");
    for t in &tables {
        md.push_str(&t.to_markdown_titled());
        md.push_str("\n\n");
    }
    md.push_str("以上表格内容均真实有效。\n");

    let mut store = DomainStore::new();
    store.create_project("样张", "ZFCG-2026-103B", "明标", "sample-key").unwrap();
    store.upsert_file("A-招标文件.docx", "招标文件", "aa", 5, 1, 50, 0, ParseStatus::Ok, "").unwrap();
    store.set_requirements(vec![Requirement {
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
    }]).unwrap();
    store.set_outline(vec![OutlineNode {
        id: "SEC-0001".into(),
        parent_id: None,
        title: "商务与技术响应".into(),
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
    asm.commit_section("SEC-0001", &md, vec!["REQ-0001".into()]).unwrap();

    let out = dir.join("标书样张-六表.docx");
    DocumentExporter::with_evidences(store.sections(), store.evidences())
        .export(&dir, &out)
        .expect("导出应成功");
    assert!(out.is_file(), "样张已生成");
    eprintln!("SAMPLE_DOCX = {}", out.display());
    eprintln!("SAMPLE_MD = {}", dir.join("sample.md").display());
    std::fs::write(dir.join("sample.md"), &md).unwrap();
}