//! forms_wire.rs —— issue #117 任务 2：格式件接线单元验收。
//!
//! 与 bid-domain forms.rs 测试（本地团队的「格式件能渲染」面）的分工：
//! 那边证「七件 markdown 对不对」，这边证「七件真的从 BidderProfile +
//! ExportDecision **自动构造**并**接进导出链**」——issue 验收线逐条落测试：
//!
//! ```text
//! [x] 七件格式件全部进 docx（forms_written == 7，Heading2 进 TOC 面）
//! [x] 字段从 BidderProfile 填充（不是占位符）：地址/开户行/税号/联系人逐格验
//! [x] 缺决策 → 【待填】显式标注（工期 0 → 【工期（日历天）】——不是「0 日历天」）
//! [x] 报价大写 = 数字的确定性换写（壹仟伍佰捌拾万元整）
//! [x] 锚定「投标文件格式」章尾；无该章 → 文末独立部分
//! [x] 暗标脱敏覆盖格式件（公司名 → 投标人）
//! ```

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::party::{BidderProfile, EquipmentFact, PersonnelFact};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::assembly::ChapterAssembler;
use bid_tools::export::{
    amount_to_cn_upper, build_export_extras, DocumentExporter, ExportDecision, ExportOptions,
    PriceItem,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 夹具

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bid-117-forms-{tag}-{}-{}",
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

/// 基座：两章（技术参数 / 第七章 投标文件格式）+ 满配 profile。
/// 叶已 DRAFTED + 已提交（带定位标记——断言注入位置用）。
fn forms_store(tag: &str) -> (DomainStore, PathBuf) {
    let dir = temp_dir(tag);
    let mut store = DomainStore::new();
    store
        .create_project("格式件接线验收", "ZFCG-2026-117F", "明标", "t117f-key")
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
    let mut r1 = Requirement {
        id: "REQ-0001".into(),
        category: "其他".into(),
        clause_no: Some("1.1".into()),
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "1.1 段".into(),
            excerpt: "1.1 项目名称：智慧病房物联网平台；项目编号：ZFCG-2026-117F；采购人：某医院。"
                .into(),
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    };
    r1.params.insert("model".into(), "ZFCG-2026-117F".into());
    // REQ-0002（技术参数章绑定用）——requirements 必须先于 outline（绑定校验）
    let r2 = Requirement {
        id: "REQ-0002".into(),
        category: "技术参数".into(),
        clause_no: Some("5.1".into()),
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "5.1 段".into(),
            excerpt: "平台须支持不少于 5000 个终端接入。".into(),
                block_idx: None,
                page: None,
                line: None
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    };
    store.set_requirements(vec![r1, r2]).unwrap();
    store
        .set_outline(vec![
            node("SEC-0001", None, "技术参数", 1, &["REQ-0002"], 300),
            node(
                "SEC-0002",
                Some("SEC-0001"),
                "技术参数（第1块）",
                2,
                &["REQ-0002"],
                300,
            ),
            node(
                "SEC-0003",
                None,
                "第七章 投标文件格式",
                3,
                &["REQ-0001"],
                300,
            ),
            node(
                "SEC-0004",
                Some("SEC-0003"),
                "第七章 投标文件格式",
                4,
                &["REQ-0001"],
                300,
            ),
        ])
        .unwrap();
    store.build_matrix().unwrap();
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    for (sid, marker) in [
        ("SEC-0002", "【技术参数章正文】"),
        ("SEC-0004", "【第七章格式正文】"),
    ] {
        for to in [
            SectionStatus::Outlining,
            SectionStatus::Drafting,
            SectionStatus::Drafted,
        ] {
            store.advance_section_status(sid, to).unwrap();
        }
        asm.commit_section(sid, &format!("{marker}\n\n本章内容。"), vec![])
            .unwrap();
    }
    (store, dir)
}

fn full_profile() -> BidderProfile {
    BidderProfile {
        company_name: "安徽示例智能科技有限公司".into(),
        legal_rep: "汪建国".into(),
        legal_rep_id: "340104197508011237".into(),
        registered_capital: 5000.0,
        address: "安徽省合肥市高新区示例大道 66 号".into(),
        bank_name: "中国建设银行股份有限公司合肥分行".into(),
        bank_account: "34001468608059999999".into(),
        tax_no: "91340100MA2RTXKJ0E".into(),
        contact_person: "李敏".into(),
        contact_phone: "13800000000".into(),
        legal_rep_position: "董事长".into(),
        agent: "王锐".into(),
        agent_id: "34010419920202000X".into(),
        postcode: "230000".into(),
        established_on: "2008-06-18".into(),
        qualification: "电子与智能化工程专业承包二级".into(),
        personnel: vec![PersonnelFact {
            position: "项目经理".into(),
            name: "陈志强".into(),
            title: "高级工程师".into(),
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
            spec: "LoRaWAN".into(),
            arrival: "合同签订后 7 日内".into(),
        }],
    }
}

fn full_decision() -> ExportDecision {
    ExportDecision {
        bid_amount: Some(15_800_000.0),
        price_breakdown: vec![PriceItem {
            item: "设备采购".into(),
            unit: "批".into(),
            quantity: 1.0,
            unit_price: 15_800_000.0,
            note: String::new(),
        }],
        project_name: Some("智慧病房物联网平台建设项目".into()),
        duration_days: Some(180),
        validity_days: Some(90),
        deposit_amount: Some(300_000.0),
        legal_rep_gender: Some("男".into()),
    }
}

fn export_xml(
    store: &DomainStore,
    dir: &Path,
    extras: &bid_tools::export::ExportExtras,
    anonymize: Option<&str>,
) -> String {
    let out = dir.join("标书.docx");
    let opts = ExportOptions {
        extras: Some(extras),
        anonymize,
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

// ---------------------------------------------------------------- ① 七件构造 + docx 落位

#[test]
fn seven_forms_built_from_profile_and_anchored_to_format_chapter() {
    let (store, dir) = forms_store("full");
    let (extras, warnings) = build_export_extras(&store, &full_profile(), &full_decision());

    // ---- 构造面：七件齐、标题带附件编号
    assert_eq!(extras.forms.len(), 7, "七件格式件缺一不可");
    let titles: Vec<String> = extras.forms.iter().map(|f| f.title()).collect();
    for (i, t) in titles.iter().enumerate() {
        assert!(
            t.contains(&format!(
                "附件{}",
                ["一", "二", "三", "四", "五", "六", "七"][i]
            )),
            "序号对: {t}"
        );
    }
    // 决策齐备 → 相关告警不该在（工期/有效期/保证金/项目名四类「未给出」告警）
    for w in &warnings {
        assert!(
            !w.contains("未随导出给出") && !w.contains("未显式给出"),
            "决策齐备时无未给出类告警: {w}"
        );
    }

    // ---- docx 落位：锚定第七章（投标文件格式）子树最后一个叶章后
    let xml = export_xml(&store, &dir, &extras, None);
    let idx_chapter_body = xml.find("【第七章格式正文】").expect("章正文在位");
    let idx_bid_letter = xml.find("附件一、投标函").expect("投标函标题在位");
    let idx_integrity = xml.find("附件七、廉洁承诺书").expect("廉洁承诺书在位");
    assert!(idx_chapter_body < idx_bid_letter, "格式件注入在章正文之后");
    assert!(idx_bid_letter < idx_integrity, "七件按附件编号连续展开");
    // 第七章之后没有别的章——格式件连续展开到文档尾（无「投标文件格式件」尾注部分）
    assert!(
        xml.find("投标文件格式件").is_none(),
        "锚定命中时不该有文末格式件部分"
    );

    // ---- 字段从 BidderProfile 填充（不是占位符）——逐件抽验
    // 投标函：大写金额（确定性换写）+ 工期 + 有效期 + 项目官方名 + 编号
    let bid_letter_md = extras.forms[0].to_markdown();
    assert!(
        bid_letter_md.contains("壹仟伍佰捌拾万元整"),
        "报价大写: {bid_letter_md}"
    );
    assert!(
        bid_letter_md.contains("15800000.00 元"),
        "报价小写: {bid_letter_md}"
    );
    assert!(
        bid_letter_md.contains("180 日历天"),
        "工期: {bid_letter_md}"
    );
    assert!(
        bid_letter_md.contains("90 日历天"),
        "有效期: {bid_letter_md}"
    );
    assert!(
        bid_letter_md.contains("智慧病房物联网平台建设项目"),
        "项目官方名: {bid_letter_md}"
    );
    assert!(
        bid_letter_md.contains("ZFCG-2026-117F"),
        "项目编号: {bid_letter_md}"
    );
    assert!(
        bid_letter_md.contains("安徽示例智能科技有限公司"),
        "投标人: {bid_letter_md}"
    );
    // 身份证明书：地址 + 性别（从 profile/决策来，不是【注册地址】占位）
    let cert_md = extras.forms[1].to_markdown();
    assert!(
        cert_md.contains("安徽省合肥市高新区示例大道 66 号"),
        "地址铺管: {cert_md}"
    );
    assert!(cert_md.contains("男"), "性别: {cert_md}");
    assert!(!cert_md.contains("【注册地址】"), "不留占位: {cert_md}");
    // 退还申请：开户行 + 联系人（#117 铺管面）
    let refund_md = extras.forms[3].to_markdown();
    assert!(
        refund_md.contains("中国建设银行股份有限公司合肥分行"),
        "开户行铺管: {refund_md}"
    );
    assert!(refund_md.contains("李敏"), "联系人铺管: {refund_md}");
    assert!(
        refund_md.contains("13800000000"),
        "联系电话铺管: {refund_md}"
    );
    assert!(
        refund_md.contains("300000.00 元"),
        "保证金金额: {refund_md}"
    );
    // 资格声明：税号 + 地址（#117 铺管面）
    let qual_md = extras.forms[4].to_markdown();
    assert!(
        qual_md.contains("91340100MA2RTXKJ0E"),
        "税号铺管: {qual_md}"
    );
    assert!(
        !qual_md.contains("【统一社会信用代码】"),
        "不留税号占位: {qual_md}"
    );
    assert!(!qual_md.contains("【注册地址】"), "不留地址占位: {qual_md}");

    // ---- Heading2 进 TOC 面（pStyle Heading2 + 附件标题文本）
    assert!(xml.contains("附件一、投标函"), "标题进正文");
    let heading2_count = xml.matches("w:val=\"Heading2\"").count();
    assert!(
        heading2_count >= 7,
        "七件至少 7 个 Heading2 段（实际 {heading2_count}）"
    );
    // 签字落款占位是格式件固有面（签字无法预填）——在场但不含业务字段占位
    assert!(
        xml.contains("【签字人姓名】") || xml.contains("【签署日期】"),
        "落款悬空位在（签字不预填）"
    );
}

// ---------------------------------------------------------------- ② 缺决策 → 待填 + 告警

#[test]
fn missing_decision_renders_pending_fill_not_fake_commitment() {
    let (store, dir) = forms_store("bare");
    let empty_decision = ExportDecision::default();
    let (extras, warnings) = build_export_extras(&store, &full_profile(), &empty_decision);
    assert_eq!(
        extras.forms.len(),
        7,
        "缺决策不缺件——七件全出（缺栏【待填】）"
    );

    let bid_letter_md = extras.forms[0].to_markdown();
    assert!(
        bid_letter_md.contains("【工期（日历天）】"),
        "工期未定 → 待填: {bid_letter_md}"
    );
    assert!(
        bid_letter_md.contains("【投标有效期（日历天）】"),
        "有效期未定 → 待填: {bid_letter_md}"
    );
    assert!(
        bid_letter_md.contains("【投标总报价（大写）】"),
        "大写随报价缺 → 待填: {bid_letter_md}"
    );
    assert!(
        !bid_letter_md.contains("0 日历天"),
        "绝不落「0 日历天」假承诺: {bid_letter_md}"
    );
    // 项目名回落 store 内部名 + 告警
    assert!(
        bid_letter_md.contains("格式件接线验收"),
        "回落内部项目名: {bid_letter_md}"
    );
    let joined = warnings.join("；");
    assert!(joined.contains("工期未随导出给出"), "工期告警: {joined}");
    assert!(
        joined.contains("有效期未随导出给出"),
        "有效期告警: {joined}"
    );
    assert!(
        joined.contains("保证金金额未随导出给出"),
        "保证金告警: {joined}"
    );
    assert!(
        joined.contains("项目名称未显式给出"),
        "项目名回落告警: {joined}"
    );
    assert!(
        joined.contains("查询区间未给出"),
        "无违法声明区间告警: {joined}"
    );

    let refund_md = extras.forms[3].to_markdown();
    assert!(
        refund_md.contains("【退还金额】"),
        "保证金未定 → 退还金额待填: {refund_md}"
    );

    // docx：锚定仍在（第七章在场）——缺决策不改变落位
    let xml = export_xml(&store, &dir, &extras, None);
    assert!(xml.find("附件一、投标函").is_some(), "投标函仍进 docx");
    assert!(xml.contains("【工期（日历天）】"), "待填进 docx");
}

// ---------------------------------------------------------------- ③ 无格式章 → 文末独立部分

#[test]
fn no_format_chapter_falls_back_to_tail_part() {
    let (mut store, _dir) = forms_store("noanchor");
    // 砍掉第七章（只留技术参数）—— outline 重设
    store
        .set_outline(vec![
            node("SEC-0001", None, "技术参数", 1, &["REQ-0002"], 300),
            node(
                "SEC-0002",
                Some("SEC-0001"),
                "技术参数（第1块）",
                2,
                &["REQ-0002"],
                300,
            ),
        ])
        .unwrap();
    // 第七章的叶提交记录成了幽灵——ChapterAssembler 侧需清掉：换新工作区更省
    let dir2 = temp_dir("noanchor-ws");
    let mut asm = ChapterAssembler::open(&dir2).unwrap();
    for to in [
        SectionStatus::Outlining,
        SectionStatus::Drafting,
        SectionStatus::Drafted,
    ] {
        store.advance_section_status("SEC-0002", to).unwrap();
    }
    asm.commit_section("SEC-0002", "【技术参数章正文】", vec![])
        .unwrap();

    let (extras, _w) = build_export_extras(&store, &full_profile(), &full_decision());
    let xml = export_xml(&store, &dir2, &extras, None);
    let idx_part = xml.find("投标文件格式件").expect("文末格式件部分在位");
    let idx_letter = xml.find("附件一、投标函").unwrap();
    assert!(idx_part < idx_letter, "尾注部分标题在前");
    // 附表（报价/业绩/人员/设备回落表）也在文末——顺序：附表在前、格式件在后
    let idx_appendix = xml.rfind("附表").unwrap();
    assert!(
        idx_appendix < idx_part,
        "附表先于格式件（表格是正文数据面，格式件是签署面）"
    );
}

// ---------------------------------------------------------------- ④ 暗标脱敏

#[test]
fn dark_bid_scrubs_company_name_in_forms() {
    let (store, dir) = forms_store("dark");
    let (extras, _w) = build_export_extras(&store, &full_profile(), &full_decision());
    let xml = export_xml(&store, &dir, &extras, Some("安徽示例智能科技有限公司"));
    assert!(
        !xml.contains("安徽示例智能科技有限公司"),
        "格式件不留公司名（暗标）"
    );
    // 脱敏后仍可读（抬头变「投标人」）
    let idx = xml.find("附件一、投标函").unwrap();
    let seg = &xml[idx..idx + 3000];
    assert!(
        seg.contains("投标人"),
        "脱敏为「投标人」: {}",
        &seg[..200.min(seg.len())]
    );
}

// ---------------------------------------------------------------- ⑤ 中文大写换写

#[test]
/// #117 任务 3 回归锚：格式件字段行的行内粗体——星号零进 docx，
/// 字段名成粗体 run（`<w:b/>`）。
fn inline_bold_field_lines_render_without_literal_asterisks() {
    let (store, dir) = forms_store("inline-bold");
    let (extras, _w) = build_export_extras(&store, &full_profile(), &full_decision());
    let xml = export_xml(&store, &dir, &extras, None);
    // (a) 渲染面零星号（#117 任务 3 验收线——不是 markdown 源，是标书正本）
    let asterisk_leak = xml.matches("**").count();
    assert_eq!(
        asterisk_leak, 0,
        "document.xml 含 {asterisk_leak} 处 `**` 字面量（行内粗体未拆段）"
    );
    // (b) 授权委托书的字段值在场（粗体定界剥除后文本完整）
    assert!(xml.contains("王锐"), "受托人姓名在场: 截取自 {} 字节 XML", xml.len());
    assert!(
        xml.contains("34010419920202000X"),
        "受托人身份证号在场（字段值不被粗体拆段误伤）"
    );
    // (c) 字段名成粗体 run：`<w:r><w:rPr>…<w:b/>` 且含「姓名」的 run 带 <w:b/>
    let idx = xml.find("附件三、授权委托书").unwrap();
    let seg = &xml[idx..];
    let name_run = seg.find("姓名").unwrap();
    let run_start = seg[..name_run].rfind("<w:r>").unwrap();
    let run_end = seg[name_run..].find("</w:r>").unwrap() + name_run;
    let run = &seg[run_start..run_end];
    assert!(
        run.contains("<w:b/>") || run.contains("<w:b "),
        "「姓名」run 必须带粗体标记: {run}"
    );
}

#[test]
fn cn_upper_amount_conversion() {
    assert_eq!(amount_to_cn_upper(Some(15_800_000.0)), "壹仟伍佰捌拾万元整");
    assert_eq!(amount_to_cn_upper(Some(100_000_005.0)), "壹亿零伍元整");
    assert_eq!(amount_to_cn_upper(Some(120.05)), "壹佰贰拾元零伍分");
    assert_eq!(amount_to_cn_upper(Some(1_580.0)), "壹仟伍佰捌拾元整");
    assert_eq!(amount_to_cn_upper(Some(0.5)), "伍角");
    assert_eq!(amount_to_cn_upper(Some(1_005.0)), "壹仟零伍元整");
    // 缺失/非正 → 空（渲染层落【待填】——大写不能凭空造）
    assert_eq!(amount_to_cn_upper(None), "");
    assert_eq!(amount_to_cn_upper(Some(0.0)), "");
    assert_eq!(amount_to_cn_upper(Some(-5.0)), "");
}

// ---------------------------------------------------------------- ⑥ 证据面不受任务 2 影响（回归锚）

#[test]
fn forms_do_not_disturb_table_pipeline() {
    // 七件在场时表格管线照常（六表 + 七件同导出互不干扰）
    let (store, dir) = forms_store("mixed");
    let (extras, _w) = build_export_extras(&store, &full_profile(), &full_decision());
    let xml = export_xml(&store, &dir, &extras, None);
    // 概况 + 偏离 + 报价 + 业绩(无证据→缺) + 人员 + 设备 = 5 表（业绩证据未登记）
    assert_eq!(
        xml.matches("<w:tbl>").count(),
        5,
        "表格面不受格式件影响（业绩表无证据不生成 → 5）"
    );
    assert!(xml.contains("附件一、投标函"), "格式件同导出");
    let _ = EvidenceKind::Qualification; // 引用锚（防止未用 import 告警）
    let _ = EvidenceSpec {
        kind: EvidenceKind::Certificate,
        title: String::new(),
        source_file: String::new(),
        source_sha256: String::new(),
        locator: EngineLocator::PdfPageText {
            page: 1,
            text_fingerprint: String::new(),
        },
        fingerprint: String::new(),
        valid_from: None,
        valid_until: None,
        subject_name: String::new(),
        scope: String::new(),
        client: None,
        amount_wan: None,
        completed_on: None,
        fields: Default::default(),
        excerpt: None,
        excerpt_locator: None,
        verification_tier: None,
    };
}
