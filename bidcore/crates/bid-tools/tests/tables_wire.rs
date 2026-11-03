//! tables_wire.rs —— issue #117 任务 1：表格接线单元验收。
//!
//! 与 tables_export.rs（本地团队的「表格 → markdown → docx」能力面）的分工：
//! 那边证「表格能渲染」，这边证「表格真的从三个真源（store / BidderProfile /
//! ExportDecision）**自动派生**并**接进导出链**」——issue 验收线逐条落测试：
//!
//! ```text
//! [x] 六表从真源派生（概况/偏离/报价/业绩/人员/设备——缺真源 → 不生成 + 告警）
//! [x] 技术参数偏离表行数 ≥ 技术参数 REQ 数（账本缺行 → 【待填】模板行）
//! [x] 负偏离整行标红 + 导出告警在场（废标风险不静默）
//! [x] 表格落点：锚定章 = 命中子树最后一个叶章之后；无命中 → 文末附表
//! [x] 导出 docx 里 <w:tbl> == 6（真表格，不是 markdown 残文本）
//! [x] 报价合计与 bid_amount 一致性告警
//! [x] 缺真源时零编造：待填显式标【待填】，绝不落 0/「无」
//! ```

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::param::ParamInput;
use bid_domain::party::{BidderProfile, EquipmentFact, PersonnelFact};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::assembly::ChapterAssembler;
use bid_tools::export::{
    build_export_extras, DocumentExporter, ExportDecision, ExportOptions, PriceItem,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 夹具

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bid-117-wire-{tag}-{}-{}",
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

fn req(id: &str, category: &str, clause: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: Some(clause.into()),
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: format!("{clause} 段"),
            excerpt: excerpt.into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

fn node(
    id: &str,
    parent: Option<&str>,
    title: &str,
    order: u32,
    reqs: &[&str],
    target_chars: u32,
) -> OutlineNode {
    OutlineNode {
        id: id.into(),
        parent_id: parent.map(str::to_string),
        title: title.into(),
        level: if parent.is_some() { 2 } else { 1 },
        order,
        requirement_ids: reqs.iter().map(|s| s.to_string()).collect(),
        target_chars,
        status: SectionStatus::Pending,
        dirty: false,
        content: None,
        content_attempt: 0,
    }
}

/// 满配基座：两章（招标公告 / 技术参数——各一子叶）+ 五 REQ + 业绩证据 +
/// 参数账（正/负/未录三形态）。返回 (store, dir)——叶已 DRAFTED + 已提交。
fn full_store(tag: &str) -> (DomainStore, PathBuf) {
    let dir = temp_dir(tag);
    let mut store = DomainStore::new();
    store
        .create_project("表格接线验收", "ZFCG-2026-117", "明标", "t117-key")
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
    let mut r1 = req(
        "REQ-0001",
        "其他",
        "1.1",
        "1.1 项目名称：智慧病房物联网平台；项目编号：ZFCG-2026-117；采购人：某医院。",
    );
    r1.params.insert("model".into(), "ZFCG-2026-117".into());
    let mut r2 = req(
        "REQ-0002",
        "否决项",
        "1.2",
        "1.2 本项目预算金额为 1650 万元（含税），最高限价：人民币 1650 万元。",
    );
    r2.params.insert("amount_wan".into(), 1650.0.into());
    let r3 = req(
        "REQ-0003",
        "技术参数",
        "5.1.1",
        "平台须支持不少于 5000 个感知终端并发接入。",
    );
    let r4 = req(
        "REQ-0004",
        "技术参数",
        "5.1.2",
        "温湿度传感器电池续航≥3 年。",
    );
    let r5 = req(
        "REQ-0005",
        "技术参数",
        "5.3.1",
        "物联网平台须提供设备管理等五类核心能力。",
    );
    store.set_requirements(vec![r1, r2, r3, r4, r5]).unwrap();
    store
        .set_outline(vec![
            node(
                "SEC-0001",
                None,
                "第一章 招标公告",
                1,
                &["REQ-0001", "REQ-0002"],
                300,
            ),
            node(
                "SEC-0002",
                Some("SEC-0001"),
                "第一章 招标公告",
                2,
                &["REQ-0001", "REQ-0002"],
                300,
            ),
            node(
                "SEC-0003",
                None,
                "技术参数",
                3,
                &["REQ-0003", "REQ-0004", "REQ-0005"],
                600,
            ),
            node(
                "SEC-0004",
                Some("SEC-0003"),
                "技术参数（第1块）",
                4,
                &["REQ-0003"],
                300,
            ),
            node(
                "SEC-0005",
                Some("SEC-0003"),
                "技术参数（第2块）",
                5,
                &["REQ-0004", "REQ-0005"],
                300,
            ),
        ])
        .unwrap();
    store.build_matrix().unwrap();
    // 业绩证据（业务面齐——发包人/金额/完成日期）
    store
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Performance,
            title: "某医院智慧护理单元改造项目".into(),
            source_file: "SYN-EV-业绩合同.pdf".into(),
            source_sha256: "sha-117-perf".into(),
            locator: EngineLocator::PdfPageText {
                page: 1,
                text_fingerprint: "fnv64:p1".into(),
            },
            fingerprint: "fnv64:p1".into(),
            valid_from: Some("2023-06-10".into()),
            valid_until: None,
            subject_name: "安徽示例智能科技有限公司".into(),
            scope: "智慧病房".into(),
            client: Some("某市第一人民医院".into()),
            amount_wan: Some(820.0),
            completed_on: Some("2024-09-30".into()),
            fields: Default::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        })
        .unwrap();
    // 参数账：正偏离（5000→8000）/ 负偏离（3年→2年）/ REQ-0005 不录（【待填】面）
    store
        .set_param_responses(vec![
            ParamInput {
                requirement_id: "REQ-0003".into(),
                param_name: "感知终端并发接入".into(),
                required_value: "不少于 5000 个".into(),
                response_value: "8000 个".into(),
                evidence_id: None,
            },
            ParamInput {
                requirement_id: "REQ-0004".into(),
                param_name: "温湿度传感器电池续航".into(),
                required_value: "≥3 年".into(),
                response_value: "2 年".into(),
                evidence_id: None,
            },
        ])
        .unwrap();
    // 叶推进 DRAFTED + 提交章内容（带定位标记——断言注入位置用）
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    for (sid, marker) in [
        ("SEC-0002", "【招标公告章正文】"),
        ("SEC-0004", "【技术参数章第1块正文】"),
        ("SEC-0005", "【技术参数章第2块正文】"),
    ] {
        for to in [
            SectionStatus::Outlining,
            SectionStatus::Drafting,
            SectionStatus::Drafted,
        ] {
            store.advance_section_status(sid, to).unwrap();
        }
        asm.commit_section(sid, &format!("{marker}\n\n本章响应内容。"), vec![])
            .unwrap();
    }
    (store, dir)
}

fn full_profile() -> BidderProfile {
    BidderProfile {
        company_name: "安徽示例智能科技有限公司".into(),
        personnel: vec![PersonnelFact {
            position: "项目经理".into(),
            name: "陈志强".into(),
            title: "高级工程师/一级建造师".into(),
            major: "机电工程".into(),
            years: Some(15),
            note: String::new(),
        }],
        equipment: vec![EquipmentFact {
            name: "物联网关".into(),
            model: "IoT-GW-3000".into(),
            quantity: Some(48),
            unit: "台".into(),
            origin: "国产".into(),
            spec: "LoRaWAN 多模".into(),
            arrival: "合同签订后 7 日内".into(),
        }],
        ..Default::default()
    }
}

fn full_decision() -> ExportDecision {
    ExportDecision {
        bid_amount: Some(15_800_000.0),
        duration_days: Some(180),
        validity_days: Some(90),
        deposit_amount: Some(300_000.0),
        legal_rep_gender: Some("男".into()),
        project_name: Some("智慧病房物联网平台建设项目".into()),
        price_breakdown: vec![
            PriceItem {
                item: "设备采购".into(),
                unit: "批".into(),
                quantity: 1.0,
                unit_price: 12_000_000.0,
                note: String::new(),
            },
            PriceItem {
                item: "安装调试".into(),
                unit: "项".into(),
                quantity: 1.0,
                unit_price: 3_800_000.0,
                note: String::new(),
            },
        ],
    }
}

/// 导出（带附加件）并解包 document.xml。
fn export_xml(store: &DomainStore, dir: &Path, extras: &bid_tools::export::ExportExtras) -> String {
    let out = dir.join("标书.docx");
    let opts = ExportOptions {
        extras: Some(extras),
        ..Default::default()
    };
    DocumentExporter::with_evidences(store.sections(), store.evidences())
        .export_with(dir, &out, &opts)
        .expect("导出应成功");
    let f = std::fs::File::open(&out).unwrap();
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

/// docx-rs 序列化形态：`<w:color w:val="FF0000" />`（`/>` 前有空格——与
/// tables_export.rs 同口径锚定）。
const RED_XML: &str = "<w:color w:val=\"FF0000\" />";

// ---------------------------------------------------------------- ① 六表派生 + docx 落位

#[test]
fn six_tables_derived_from_real_sources_and_written_into_docx() {
    let (store, dir) = full_store("full");
    let (extras, warnings) = build_export_extras(&store, &full_profile(), &full_decision());

    // ---- 派生面：六表齐、序固定
    let kinds: Vec<&str> = extras.tables.iter().map(|lt| lt.table.kind()).collect();
    assert_eq!(
        kinds,
        vec![
            "项目概况表",
            "技术参数偏离表",
            "报价明细表",
            "业绩一览表",
            "人员配置表",
            "设备清单表",
        ],
        "六表按固定序派生"
    );

    // ---- 偏离表：行数 ≥ 技术参数 REQ 数（3 REQ → 3 行：正/负/待填）
    let tech = extras
        .tables
        .iter()
        .find(|lt| lt.table.kind() == "技术参数偏离表")
        .unwrap();
    assert!(
        tech.table.row_count() >= 3,
        "偏离表行数 {} ≥ 技术参数 REQ 数 3",
        tech.table.row_count()
    );
    let md = tech.table.to_markdown();
    assert!(md.contains("8000 个"), "账本行带真响应: {md}");
    assert!(md.contains("负偏离"), "账本行带真判定: {md}");
    assert!(md.contains("【待填】"), "未录 REQ 落待填行: {md}");
    assert!(md.contains("不可比"), "未录 REQ 偏离诚实态: {md}");
    assert_eq!(tech.table.negative_count(), 1, "负偏离恰 1 行");
    // 负偏离告警在场（废标风险不静默）
    assert!(
        warnings.iter().any(|w| w.contains("负偏离")),
        "负偏离告警: {warnings:?}"
    );

    // ---- 概况表：REQ 参数定位 + 决策报价
    let overview = extras
        .tables
        .iter()
        .find(|lt| lt.table.kind() == "项目概况表")
        .unwrap();
    let ov_md = overview.table.to_markdown();
    assert!(
        ov_md.contains("ZFCG-2026-117"),
        "项目编号从 REQ 参数来: {ov_md}"
    );
    assert!(
        ov_md.contains("1650.00 万元"),
        "最高限价从 REQ 参数来: {ov_md}"
    );
    assert!(
        ov_md.contains("15800000.00 元"),
        "投标总报价从决策来: {ov_md}"
    );
    assert!(
        ov_md.contains("安徽示例智能科技有限公司"),
        "投标人从 profile 来: {ov_md}"
    );

    // ---- 业绩表：证据业务面直落
    let perf = extras
        .tables
        .iter()
        .find(|lt| lt.table.kind() == "业绩一览表")
        .unwrap();
    let perf_md = perf.table.to_markdown();
    assert!(
        perf_md.contains("某医院智慧护理单元改造项目"),
        "项目名=证据标题: {perf_md}"
    );
    assert!(
        perf_md.contains("某市第一人民医院"),
        "发包人=证据业务面: {perf_md}"
    );
    assert!(perf_md.contains("820.00"), "金额=证据业务面: {perf_md}");

    // ---- 报价表：合计行 = bid_amount（一致性——评标否决点）
    let price = extras
        .tables
        .iter()
        .find(|lt| lt.table.kind() == "报价明细表")
        .unwrap();
    let price_md = price.table.to_markdown();
    assert!(
        price_md.contains("15,800,000.00"),
        "合计行自动求和且与 bid_amount 一致: {price_md}"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("≠")),
        "分项合计与总价一致 → 无不一致告警: {warnings:?}"
    );

    // ---- docx 落位：真表格 6 张 + 注入位置正确 + 标红在位
    let xml = export_xml(&store, &dir, &extras);
    assert_eq!(xml.matches("<w:tbl>").count(), 6, "docx 里真表格恰 6 张");
    // 注入位置：技术参数偏离表在技术参数章最后一块正文之后、附表之前
    let idx_tech_body = xml.find("【技术参数章第2块正文】").expect("章正文在位");
    let idx_tech_table = xml.find("技术参数偏离表").expect("偏离表标题在位");
    let idx_appendix = xml.find("附表").expect("附表部分在位");
    assert!(idx_tech_body < idx_tech_table, "偏离表注入在章正文之后");
    assert!(
        idx_tech_table < idx_appendix,
        "偏离表在附表之前（锚定命中）"
    );
    // 概况表锚定第一章（招标公告）
    let idx_announce_body = xml.find("【招标公告章正文】").expect("公告章正文在位");
    let idx_overview = xml.find("项目概况表").expect("概况表标题在位");
    assert!(
        idx_announce_body < idx_overview,
        "概况表注入在公告章正文之后"
    );
    assert!(
        idx_overview < idx_tech_body,
        "概况表在技术参数章之前（文档序）"
    );
    // 负偏离整行标红：5 格 × 1 行 = 5 个红 run
    assert_eq!(xml.matches(RED_XML).count(), 5, "负偏离行 5 格全标红");
    // 附表承接无锚定的四表（报价/业绩/人员/设备）
    for kind in ["报价明细表", "业绩一览表", "人员配置表", "设备清单表"] {
        let idx = xml.find(kind).unwrap_or_else(|| panic!("{kind} 应在 docx"));
        assert!(idx > idx_appendix, "{kind} 落在附表部分");
    }
    // 【待填】面（未录 REQ 的响应列）
    assert!(xml.contains("【待填】"), "待填占位在 docx（不是 0/空白）");
}

// ---------------------------------------------------------------- ② 缺真源 → 不生成 + 告警（零编造）

#[test]
fn missing_sources_skip_tables_with_loud_warnings_not_fake_values() {
    let (store, dir) = full_store("bare");
    // 空档：无业绩消费面之外——业绩证据在 full_store 里登记过且 Active，
    // 这里用「无 profile 事实 + 无报价决策」验证回落面（业绩表仍在——证据在账）。
    let empty_profile = BidderProfile::default();
    let (extras, warnings) =
        build_export_extras(&store, &empty_profile, &ExportDecision::default());

    let kinds: Vec<&str> = extras.tables.iter().map(|lt| lt.table.kind()).collect();
    assert_eq!(
        kinds,
        vec!["项目概况表", "技术参数偏离表", "业绩一览表"],
        "缺决策/缺主体事实 → 报价/人员/设备三表不生成（不造空表）: {kinds:?}"
    );
    let joined = warnings.join("；");
    assert!(
        joined.contains("投标总报价未随导出给出"),
        "缺总价告警: {joined}"
    );
    assert!(joined.contains("无分项报价决策"), "缺分项告警: {joined}");
    assert!(joined.contains("未提供拟派人员"), "缺人员告警: {joined}");
    assert!(joined.contains("未提供拟投入设备"), "缺设备告警: {joined}");
    assert!(
        joined.contains("参数响应账"),
        "参数账缺行告警（REQ-0005 未录）: {joined}"
    );

    // 概况表：报价/投标人两栏落【待填】——不是 0 元、不是空白
    let overview = extras
        .tables
        .iter()
        .find(|lt| lt.table.kind() == "项目概况表")
        .unwrap();
    let ov_md = overview.table.to_markdown();
    assert!(ov_md.contains("【投标总报价】"), "缺报价 → 待填: {ov_md}");
    assert!(ov_md.contains("【投标人名称】"), "缺主体 → 待填: {ov_md}");

    // docx：只落 3 张表（生成几张写几张——不虚报）
    let xml = export_xml(&store, &dir, &extras);
    assert_eq!(
        xml.matches("<w:tbl>").count(),
        3,
        "docx 真表格数 = 派生表数"
    );
}

// ---------------------------------------------------------------- ③ 报价不一致 → 响亮告警

#[test]
fn breakdown_total_mismatch_with_bid_amount_warns_loudly() {
    let (store, _dir) = full_store("mismatch");
    let mut d = full_decision();
    d.bid_amount = Some(9_999_999.0); // 分项合计 1580 万 ≠ 总报价
    let (_extras, warnings) = build_export_extras(&store, &full_profile(), &d);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("≠") && w.contains("9999999.00") && w.contains("15800000.00")),
        "不一致告警要带两个数字（实际 vs 期望）: {warnings:?}"
    );
}
