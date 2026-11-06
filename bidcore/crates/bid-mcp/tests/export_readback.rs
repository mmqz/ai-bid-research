//! #172 导出物独立回读验证（不信任引擎自报——用独立解析器回读产物）。
//!
//! 差距面：`e2e_artifacts.rs` 的验收靠 `12_export-check.json`——那是**引擎
//! 自己生成、自己断言**的产物（导出器返什么就记什么）。引擎说「导出成功」
//! 不等于文件能打开、不等于内容对。本测试走另一条路：**不看任何工具返回
//! 值**，只把导出落盘的字节解包，用本文件自带的独立解析器重新读一遍，按
//! 「评委打开这份文件会看到什么」的口径断言。
//!
//! 回读面（issue #172 表格逐格落）：
//! | 格式 | 回读方式 | 断言 |
//! |---|---|---|
//! | docx | 解 zip 读 `word/document.xml`（自数 pStyle/tbl/sectPr） | 与 outline 对齐 |
//! | pdf  | pdfium 反向读（有则）／纯 Rust 兜底（无则） | 页数、文本层在位、中文可读 |
//! | xlsx | 解 zip 读 `sharedStrings.xml` | 单元格类型与值正确 |
//!
//! 判据纪律（issue #9）：环境缺件 → 响亮 SKIP 横幅 + return，绝不 pass 冒充
//! 通过；产线 xlsx 通道尚未落地（导出器只放行 docx|pdf），因此 xlsx 面断言
//! 的是「**当前产线诚实拒绝** + **对照件可被独立解析**」——用仓库既有 xlsx
//! 夹具（corpus-A 的 B3-图纸目录.xlsx）证明回读器本身有效，不把「没有 xlsx
//! 导出」粉饰成「xlsx 回读通过」。
//!
//! 保真度守卫（issue #169）：PDF 纯 Rust 兜底在无 pdfium/poppler 环境会解出
//! 中文乱码。回读到乱码时**不判红**——用 `cjk_ratio() >= MIN_CJK_RATIO`
//! （0.25，同 `numbering_corpus.rs` 口径）判「文本可用」，低则 SKIP 横幅。
//! 撞上 #169 的缺陷登记到 issue，不在本任务修解码。

use bid_domain::evidence::{EvidenceKind, EvidenceSpec};
use bid_domain::locator::EngineLocator;
use bid_domain::outline::OutlineSpec;
use bid_domain::{Anchor, ParseStatus, Requirement};
use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::io::Read as _;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 常量与纪律

/// CJK 占非空白字符的比例下限（同 `bid-parse/tests/numbering_corpus.rs`）。
/// 低于此值 → 判「解析环境不可用」，SKIP 而非假红（issue #169）。
const MIN_CJK_RATIO: f64 = 0.25;

/// 残留占位符（未渲染就落盘的模板痕迹）。
const RESIDUAL_PLACEHOLDERS: [&str; 4] = ["{{", "}}", "TODO", "【待填写】"];

/// 「未生成」标记（引擎没生成内容却把标记写进产物）。
const UNGENERATED_MARKS: [&str; 5] = ["未填写", "N/A", "n/a", "null", "undefined"];

/// 中文字体 fallback 的可见痕迹（Word 取不到字形时吐的方框/替换符）。
const FALLBACK_GLYPHS: [char; 3] = ['\u{25a1}', '\u{fffd}', '\u{3013}'];

// ---------------------------------------------------------------- 通用工具面

fn call(svc: &mut BidService, tool: &str, mut args: Value) -> ToolReply {
    // 确定性套件钉 mock（防 dev 机配了 BID_LLM_API_KEY 后 auto 走真实调用）
    if tool == "draft_response_section" && args.get("generator").is_none() {
        args.as_object_mut()
            .unwrap()
            .insert("generator".into(), json!("mock"));
    }
    dispatch_with(
        svc,
        ToolCall {
            tool: tool.to_string(),
            args,
        },
    )
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("回读夹具步骤失败: {error}"),
    }
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-readback-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn mk_req(id: &str, category: &str, excerpt: &str) -> Requirement {
    Requirement {
        id: id.into(),
        category: category.into(),
        clause_no: None,
        anchors: vec![Anchor {
            file: "A-招标文件.docx".into(),
            kind: "para".into(),
            locator: "第二章/3.1".into(),
            excerpt: excerpt.into(),
        }],
        params: Default::default(),
        source_file: "A-招标文件.docx".into(),
    }
}

/// 回读载具：一份**导出落盘**的标书（docx 主 + pdf 副）。
///
/// 全部素材走真实服务面（create → import → extract → outline → matrix →
/// draft → export），与 e2e_artifacts 同族但不读它的 12 号产物——本测试的
/// 真源是**磁盘上的字节**，不是任何 JSON 台账。
struct Fixture {
    svc: BidService,
    pid: String,
    dir: PathBuf,
    /// outline 根章标题（回读对齐的期望面——来自 store，不是导出报告）。
    root_titles: Vec<String>,
    /// 全部章节标题（含子章）。
    all_titles: Vec<String>,
    /// 叶章数（= 导出器应写的章数）。
    leaves: usize,
}

fn fixture(tag: &str, target_chars: u32) -> Fixture {
    let dir = tmp_dir(tag);
    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(
        &mut svc,
        "create_bid_project",
        json!({
            "name": "回读验证项目——智慧病房物联网平台",
            "bid_deadline": "2026-12-31", "mode": "明标",
            "external_key": format!("readback-{tag}"),
        }),
    ))["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    svc.store_mut()
        .upsert_file(
            "A-招标文件.docx",
            "招标文件",
            &format!("sha-rb-{tag}"),
            60,
            6,
            400,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
    // #131 适配：补遗件（否则 L3 MissingAddendum 闸拦下全叶 draft）
    svc.store_mut()
        .upsert_file(
            "C-补遗文件01.docx",
            "补遗",
            &format!("sha-rb-add-{tag}"),
            20,
            0,
            100,
            0,
            ParseStatus::Ok,
            "",
        )
        .unwrap();
    let cats = ["资格条件", "否决项", "评分项", "技术参数", "商务要求", "格式要求"];
    let reqs: Vec<Requirement> = (0..36)
        .map(|i| {
            mk_req(
                &format!("REQ-{:04}", i + 1),
                cats[i % cats.len()],
                &format!("第 {} 条：智慧病房物联网平台的建设与验收要求", i + 1),
            )
        })
        .collect();
    svc.set_requirements(reqs).unwrap();
    let (n_sec, _, _) = svc
        .prepare_outline_and_matrix(OutlineSpec {
            target_total_chars: target_chars,
            max_reqs_per_leaf: 6,
        })
        .unwrap();
    assert!(n_sec >= 6, "夹具章节树规模: {n_sec}");

    // 素材面（readiness 缺失回报闸：资格/否决/技术参数须证据佐证）
    let (eid, _) = svc
        .store_mut()
        .register_evidence(EvidenceSpec {
            kind: EvidenceKind::Qualification,
            title: "电子与智能化工程专业承包二级资质证书（回读夹具）".into(),
            source_file: "EV-readback.pdf".into(),
            source_sha256: format!("sha-rb-ev-{tag}"),
            locator: EngineLocator::DocxParagraph {
                paragraph_index: 0,
                text_fingerprint: "fnv64:rb".into(),
            },
            fingerprint: "fnv64:rb".into(),
            valid_from: None,
            valid_until: None,
            subject_name: "投标人".into(),
            scope: "电子与智能化工程".into(),
            client: None,
            amount_wan: None,
            completed_on: None,
            fields: Default::default(),
            excerpt: None,
            excerpt_locator: None,
            verification_tier: None,
        })
        .unwrap();
    // #117 任务 1：业绩证据台账（3 条——MIN_PERFORMANCE_COUNT 门槛；
    //     业绩一览表的真源，也是「六表全部进产物」回读面的素材）
    for (i, (title, client, amount)) in [
        ("某市第一人民医院智慧护理单元改造项目", "某市第一人民医院", 820.0),
        ("某省中医院物联网监测平台建设项目", "某省中医院", 660.0),
        ("某市中心医院手术室智能环境监控项目", "某市中心医院", 540.0),
    ]
    .iter()
    .enumerate()
    {
        let (_, _) = svc
            .store_mut()
            .register_evidence(EvidenceSpec {
                kind: EvidenceKind::Performance,
                title: title.to_string(),
                source_file: format!("EV-readback-业绩-{i}.pdf"),
                source_sha256: format!("sha-rb-perf-{tag}-{i}"),
                locator: EngineLocator::PdfPageText {
                    page: 1,
                    text_fingerprint: format!("fnv64:rbperf{i}"),
                },
                fingerprint: format!("fnv64:rbperf{i}"),
                valid_from: Some("2024-01-08".into()),
                valid_until: None,
                subject_name: "投标人".into(),
                scope: "智慧病房/物联网平台".into(),
                client: Some(client.to_string()),
                amount_wan: Some(*amount),
                completed_on: Some("2025-02-28".into()),
                fields: Default::default(),
                excerpt: None,
                excerpt_locator: None,
                verification_tier: None,
            })
            .unwrap();
    }
    let must: Vec<String> = svc
        .store()
        .requirements()
        .iter()
        .filter(|r| matches!(r.category.as_str(), "资格条件" | "否决项" | "技术参数"))
        .map(|r| r.id.clone())
        .collect();
    for rid in &must {
        svc.store_mut()
            .bind_evidence(&eid, rid, "2026-12-31")
            .unwrap();
    }
    let _ = svc.store_mut().build_matrix().unwrap();
    // #117：主体信息建档（人员/设备表的真源——缺档时格式件落【待填】，
    // 回读面要的是「待填位显式在位」而不是「字段为空」）
    let profile = bid_domain::party::BidderProfile {
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
        personnel: vec![
            bid_domain::party::PersonnelFact {
                position: "项目经理".into(),
                name: "陈志强".into(),
                title: "高级工程师 / 一级建造师（机电）".into(),
                major: "机电工程".into(),
                years: Some(15),
                note: "在岗，社保在本单位".into(),
            },
            bid_domain::party::PersonnelFact {
                position: "技术负责人".into(),
                name: "刘雅芳".into(),
                title: "高级工程师".into(),
                major: "物联网工程".into(),
                years: Some(12),
                note: String::new(),
            },
        ],
        equipment: vec![bid_domain::party::EquipmentFact {
            name: "物联网关".into(),
            model: "IoT-GW-3000".into(),
            quantity: Some(48),
            unit: "台".into(),
            origin: "国产".into(),
            spec: "LoRaWAN/RS485/蓝牙多模，双链路上行".into(),
            arrival: "合同签订后 7 日内".into(),
        }],
    };
    profile.save(&dir).expect("bidder-profile.json 落盘");

    use bid_domain::review::{Decision, Gate};
    // #180 审核内容绑定：AddendumWaiver/矩阵/大纲门批的是补遗/响应面/结构面，
    // 保持起草前原位；PreExportConfirm 绑定章节正文摘要，挑至全部叶起草
    // 完成后记审（见下方 draft 循环后）
    for gate in [
        Gate::AddendumWaiver,
        Gate::MatrixApproval,
        Gate::OutlineApproval,
    ] {
        svc.store_mut()
            .record_review(gate, &bid_domain::TrustedPrincipal::human("readback-lead"), Decision::Approved, vec![], None)
            .unwrap();
    }
    let parents: std::collections::HashSet<String> = svc
        .store()
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.clone())
        .collect();
    let leaf_ids: Vec<String> = svc
        .store()
        .sections()
        .iter()
        .filter(|n| !parents.contains(&n.id))
        .map(|n| n.id.clone())
        .collect();
    assert!(!leaf_ids.is_empty(), "夹具必须有叶章");
    for sid in &leaf_ids {
        svc.draft_section(sid, None, None, None, &mut bid_tools::draft::MockGenerator)
            .unwrap();
    }
    // #180：定稿内容过门（夹具链到此正文已定——Fixture 交出后再改正文的
    // 用例须重过 PreExportConfirm，如 readback_docx_dark_bid_scrubbed）
    svc.store_mut()
        .record_review(Gate::PreExportConfirm, &bid_domain::TrustedPrincipal::human("readback-lead"), Decision::Approved, vec![], None)
        .unwrap();
    let root_titles: Vec<String> = svc
        .store()
        .sections()
        .iter()
        .filter(|n| n.parent_id.is_none())
        .map(|n| n.title.clone())
        .collect();
    let all_titles: Vec<String> = svc
        .store()
        .sections()
        .iter()
        .map(|n| n.title.clone())
        .collect();
    Fixture {
        svc,
        pid,
        dir,
        root_titles,
        all_titles,
        leaves: leaf_ids.len(),
    }
}

/// 导出决策（与 e2e_artifacts 同口径的报价决策——六表接线素材）。
fn export_profile() -> Value {
    json!({
        "bid_amount": 15_800_000.0,
        "price_breakdown": [
            { "item": "感知层设备", "unit": "批", "quantity": 1.0, "unit_price": 4_200_000.0 },
            { "item": "网络层设备", "unit": "批", "quantity": 1.0, "unit_price": 2_600_000.0 },
            { "item": "平台层软硬件", "unit": "批", "quantity": 1.0, "unit_price": 5_100_000.0 },
            { "item": "终端层设备", "unit": "批", "quantity": 1.0, "unit_price": 2_900_000.0 },
            { "item": "安装调试与集成实施", "unit": "项", "quantity": 1.0, "unit_price": 700_000.0 },
            { "item": "售后服务（三年质保）", "unit": "项", "quantity": 1.0, "unit_price": 300_000.0 }
        ],
        "project_name": "某市第一人民医院智慧病房物联网平台建设项目",
        "duration_days": 180,
        "validity_days": 90,
        "deposit_amount": 300_000.0,
        "legal_rep_gender": "男",
    })
}

// ---------------------------------------------------------------- 独立解析器（docx）

/// 一个 docx 包的**独立回读**（不借 docx-rs，不借导出器）。
struct DocxRead {
    /// `word/document.xml` 全文。
    document: String,
    /// 包内全部部件名（完整性面：`[Content_Types].xml` / `word/styles.xml`…）。
    parts: Vec<String>,
}

/// 段落（独立解析出的结构面——不是 docx-rs 的模型）。
struct Para {
    /// `w:pStyle` 的 `w:val`（无样式段 = None）。
    style: Option<String>,
    /// 段内全部 `<w:t>` 文本拼接（标题可能被多 run 切分）。
    text: String,
}

fn read_docx(path: &Path) -> DocxRead {
    let f = std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("回读打开 docx 失败（{e}）: {}", path.display()));
    let mut zip = zip::ZipArchive::new(f).unwrap_or_else(|e| {
        panic!(
            "导出物不是合法 zip（{e}）——评委打不开这份文件: {}",
            path.display()
        )
    });
    let mut parts = Vec::new();
    let mut document = String::new();
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).unwrap();
        parts.push(e.name().to_string());
        if e.name() == "word/document.xml" {
            e.read_to_string(&mut document).unwrap();
        }
    }
    assert!(
        !document.is_empty(),
        "word/document.xml 不在包内（{}）: {parts:?}",
        path.display()
    );
    DocxRead { document, parts }
}

impl DocxRead {
    /// OOXML 最小完备包（缺一件 Word 就报「文件已损坏」）。
    fn required_parts() -> [&'static str; 4] {
        [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/styles.xml",
        ]
    }

    /// 按 `<w:p ` 切段逐段解析（paraId 由导出层统一重排——全部带属性，
    /// 无裸 `<w:p>` 形态；带空格查找不误中 `<w:pPr>`）。
    fn paragraphs(&self) -> Vec<Para> {
        let mut out = Vec::new();
        for seg in self.document.split("<w:p ").skip(1) {
            let end = seg.find("</w:p>").unwrap_or(seg.len());
            let body = &seg[..end];
            let style = body
                .split_once("<w:pStyle ")
                .and_then(|(_, rest)| rest.split_once("w:val=\""))
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(v, _)| v.to_string());
            let mut text = String::new();
            let mut rest = body;
            while let Some(t) = rest.find("<w:t") {
                let after = &rest[t..];
                let open_end = after.find('>').unwrap_or(after.len());
                let close = after[open_end..].find("</w:t>").unwrap_or(after.len());
                text.push_str(&after[open_end + 1..open_end + close]);
                rest = &rest[t + open_end + 1 + close..];
            }
            out.push(Para { style, text });
        }
        out
    }

    /// 指定样式的段落文本（按文档序）。
    fn texts_with_style(&self, style: &str) -> Vec<String> {
        self.paragraphs()
            .into_iter()
            .filter(|p| p.style.as_deref() == Some(style))
            .map(|p| p.text)
            .collect()
    }

    /// 真表格数（自数 `<w:tbl>`——不读导出报告的 tables_written）。
    fn table_count(&self) -> usize {
        self.document.matches("<w:tbl>").count()
    }

    /// 节属性（sectPr）——无它 Word 不知页面尺寸/边距，打开即版面错乱。
    fn sect_pr(&self) -> Option<&str> {
        let i = self.document.find("<w:sectPr")?;
        let rest = &self.document[i..];
        let end = rest.find("</w:sectPr>")?;
        Some(&rest[..end + "</w:sectPr>".len()])
    }

    /// TOC 域指令（目录——评委翻页的第一站）。
    fn toc_instr(&self) -> Option<String> {
        let i = self.document.find("<w:instrText")?;
        let rest = &self.document[i..];
        let open = rest.find('>')?;
        let close = rest[open..].find("</w:instrText>")?;
        Some(rest[open + 1..open + close].to_string())
    }

    /// 全部正文文本（`<w:t>` 拼接——占位符/乱码扫描的输入面）。
    fn body_text(&self) -> String {
        self.paragraphs()
            .into_iter()
            .map(|p| p.text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// CJK 占非空白字符的比例（同 numbering_corpus 口径）。
fn cjk_ratio(s: &str) -> f64 {
    let total = s.chars().filter(|c| !c.is_whitespace()).count();
    if total == 0 {
        return 0.0;
    }
    let cjk = s
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    cjk as f64 / total as f64
}

/// 通用断言①：无残留占位符（模板没渲染就落盘）。
fn assert_no_residual_placeholder(what: &str, s: &str) {
    for p in RESIDUAL_PLACEHOLDERS {
        assert!(
            !s.contains(p),
            "回读 {what}：残留占位符「{p}」（模板未渲染就落盘——评委看到的是模板不是标书）"
        );
    }
}

/// 通用断言②：无「未生成」标记（引擎没生成却把标记写进产物）。
fn assert_no_ungenerated_mark(what: &str, s: &str) {
    for m in UNGENERATED_MARKS {
        assert!(
            !s.contains(m),
            "回读 {what}：未生成标记「{m}」（内容没生成却写进了交付文件）"
        );
    }
}

/// 通用断言③：中文字体不是 fallback（不是 □ / 替换符）。
fn assert_no_fallback_glyph(what: &str, s: &str) {
    for g in FALLBACK_GLYPHS {
        assert!(
            !s.contains(g),
            "回读 {what}：中文字体 fallback 痕迹「{g}」（字形缺失——评委看到方框）"
        );
    }
}

/// 通用断言全套（三种格式共用——issue #172 的「通用断言」清单）。
fn assert_common(what: &str, s: &str) {
    assert_no_residual_placeholder(what, s);
    assert_no_ungenerated_mark(what, s);
    assert_no_fallback_glyph(what, s);
}

// ---------------------------------------------------------------- 1. docx 回读（主面）

#[test]
fn readback_docx_structure_matches_outline() {
    let mut fx = fixture("docx-main", 12_000);
    let r = ok_result(call(
        &mut fx.svc,
        "export_bid_document",
        json!({
            "project_id": fx.pid, "format": "docx",
            "output_path": "out/回读-投标文件.docx",
            "profile": export_profile(),
        }),
    ));
    let url = r["artifact"]["url"].as_str().unwrap().to_string();
    let path = PathBuf::from(&url);
    assert!(path.is_file(), "产物落盘: {url}");

    // ① 包完整性（Word 打开的第一道门——缺件即「文件已损坏」）
    let doc = read_docx(&path);
    for req in DocxRead::required_parts() {
        assert!(
            doc.parts.iter().any(|p| p == req),
            "回读 docx：缺必需部件 {req}（包内: {:?}）",
            doc.parts
        );
    }

    // ② 结构完整：sectPr 在位（页面/边距/栏——无它版面无定义）
    let sect =
        doc.sect_pr()
            .unwrap_or_else(|| panic!("回读 docx：无 <w:sectPr>（页面属性缺失）"));
    assert!(
        sect.contains("<w:pgSz") && sect.contains("<w:pgMar"),
        "回读 docx：sectPr 缺 pgSz/pgMar（页面尺寸或边距未落）: {sect}"
    );

    // ③ 与 outline 对齐（标题集对账——比计数强：防错位重排）
    let h1 = doc.texts_with_style("Heading1");
    assert!(
        !fx.root_titles.is_empty(),
        "回读 docx：outline 根章为空（空大纲何谈对账）"
    );
    for title in &fx.root_titles {
        assert!(
            h1.iter().any(|t| t == title),
            "回读 docx：outline 根章「{title}」未以 Heading1 渲染（docx H1 集: {h1:?}）"
        );
    }
    // 全部章节标题（含子章）在各级 Heading 里的覆盖度
    let headings: Vec<String> = (1..=4u8)
        .flat_map(|l| doc.texts_with_style(&format!("Heading{l}")))
        .collect();
    for title in &fx.all_titles {
        assert!(
            headings.iter().any(|t| t == title),
            "回读 docx：章节「{title}」未以 Heading 样式渲染（文档序错位或章节丢失）"
        );
    }

    // ④ 真表格数（自数 <w:tbl>——不读导出报告的 tables_written）
    let tables = doc.table_count();
    assert!(
        tables >= 6,
        "回读 docx：真表格数 {tables} < 6（六表应全部成 <w:tbl>，不是 markdown 残文本）"
    );

    // ⑤ 正文段落在位（叶章数 → 正文段数下界）
    let paras = doc.paragraphs();
    assert!(
        paras.len() > fx.leaves,
        "回读 docx：段落数 {} ≤ 叶章数 {}（正文未展开）",
        paras.len(),
        fx.leaves
    );

    // ⑥ 通用断言（占位符/未生成标记/字体 fallback）
    let body = doc.body_text();
    assert_common("docx", &body);

    // ⑦ 中文可读（CJK 占比——解出来的是不是中文）
    let ratio = cjk_ratio(&body);
    assert!(
        ratio >= MIN_CJK_RATIO,
        "回读 docx：正文 CJK 占比 {:.1}% < {:.0}%（产物不是中文标书）",
        ratio * 100.0,
        MIN_CJK_RATIO * 100.0
    );

    // ⑧ 目录域在位（评委翻页第一站——页码连贯的前提）
    let instr = doc
        .toc_instr()
        .unwrap_or_else(|| panic!("回读 docx：无 TOC 域指令（目录不生成=评委无法定位）"));
    assert!(
        instr.contains("TOC"),
        "回读 docx：域指令不是 TOC（实际 {instr:?}）"
    );
    // 目录页码连贯面：域带 hyperlink/层级开关（\\h \\u）——页码由 Word 展开，
    // 回读面只能验域形态（开关缺失 = 目录无页码/无跳转）
    assert!(
        instr.contains("\\h") && instr.contains("\\u"),
        "回读 docx：TOC 域缺 \\h/\\u 开关（目录页码/跳转不可用）: {instr}"
    );

    // ⑨ 待填位显式在位（#117 诚实面：查不到的事实落【…】，不落空/0）
    //     夹具给了完整主体信息 —— 人员/设备表不该有待填；格式件的查询区间
    //     是调用方口径（导出层不替定），因此【查询起始日期】必须显式在位。
    assert!(
        body.contains("【查询起始日期】"),
        "回读 docx：查询区间未落显式待填位（导出层不得替调用方定开标日口径）"
    );

    println!(
        "[回读 docx] 部件 {}｜段落 {}｜表 {tables}｜H1 {}｜CJK {:.1}%｜TOC {instr}",
        doc.parts.len(),
        paras.len(),
        h1.len(),
        ratio * 100.0
    );
}

#[test]
fn readback_docx_no_residual_placeholders_and_cjk_fidelity() {
    let mut fx = fixture("docx-scan", 8_000);
    let r = ok_result(call(
        &mut fx.svc,
        "export_bid_document",
        json!({
            "project_id": fx.pid, "format": "docx",
            "output_path": "out/回读-扫描.docx",
            "profile": export_profile(),
        }),
    ));
    let doc = read_docx(Path::new(r["artifact"]["url"].as_str().unwrap()));

    // 扫的是 **XML 全文**（不只 <w:t> 拼接）——模板痕迹可能藏在属性里
    // （未生成标记只扫正文：XML 命名空间/属性里出现 "null" 这类字面量
    //  不是「内容未生成」，那面由 body_text 承载）
    let xml = &doc.document;
    assert_no_residual_placeholder("docx XML", xml);
    assert_no_fallback_glyph("docx XML", xml);
    assert_no_ungenerated_mark("docx 正文", &doc.body_text());

    // markdown 星号字面量（渲染面干净——评委看到星号比看到加粗更糟）
    assert!(
        !doc.body_text().contains("**"),
        "回读 docx：markdown 星号字面量进产物（粗体未渲染）"
    );
    // 单元格在座（每表至少一行一列——空表 = 表格渲染失败）
    let rows = xml.matches("<w:tr>").count();
    let cells = xml.matches("<w:tc>").count();
    assert!(
        rows > 0 && cells >= rows,
        "回读 docx：表格行列异常（行 {rows} / 格 {cells}——空表或渲染失败）"
    );
}

#[test]
fn readback_docx_dark_bid_scrubbed_and_structure_intact() {
    let mut fx = fixture("docx-dark", 6_000);
    // 植入公司名稿（暗标评审禁公司名）
    let sid = {
        let parents: std::collections::HashSet<String> = fx
            .svc
            .store()
            .sections()
            .iter()
            .filter_map(|n| n.parent_id.clone())
            .collect();
        fx.svc
            .store()
            .sections()
            .iter()
            .find(|n| !parents.contains(&n.id))
            .unwrap()
            .id
            .clone()
    };
    struct CompanyGen;
    impl bid_tools::draft::SectionGenerator for CompanyGen {
        fn generate(
            &mut self,
            input: &bid_tools::draft::SectionDraftInput,
        ) -> Result<bid_tools::draft::GeneratedDraft, String> {
            let base = bid_tools::draft::generate_mock(input);
            Ok(bid_tools::draft::GeneratedDraft {
                content: format!("{base}\n\n某某建设工程有限公司承建本项目。"),
            })
        }
    }
    fx.svc
        .draft_section(&sid, None, None, Some("公司名稿"), &mut CompanyGen)
        .unwrap();
    // #180：正文最后一次写入（公司名稿）后重过导出确认门
    use bid_domain::review::{Decision, Gate};
    fx.svc
        .store_mut()
        .record_review(Gate::PreExportConfirm, &bid_domain::TrustedPrincipal::human("readback-lead"), Decision::Approved, vec![], None)
        .unwrap();
    let r = ok_result(call(
        &mut fx.svc,
        "export_bid_document",
        json!({
            "project_id": fx.pid, "format": "docx",
            "output_path": "out/回读-暗标.docx",
            "profile": { "dark_bid": true, "bidder_name": "某某建设工程有限公司" },
        }),
    ));
    let doc = read_docx(Path::new(r["artifact"]["url"].as_str().unwrap()));
    let body = doc.body_text();
    // 回读面（不信任工具返回）——公司名零残留 + 替换词在位
    assert!(
        !body.contains("某某建设工程有限公司"),
        "回读 docx（暗标）：公司名残留（暗标识别点——直接废标）"
    );
    assert!(
        body.contains("投标人"),
        "回读 docx（暗标）：替换词「投标人」不在位"
    );
    // 脱敏不破坏结构（换词后面仍在——sectPr/表格/H1 全套）
    assert!(doc.sect_pr().is_some(), "回读 docx（暗标）：sectPr 缺失");
    assert!(doc.table_count() >= 1, "回读 docx（暗标）：表格丢失");
    assert!(
        !doc.texts_with_style("Heading1").is_empty(),
        "回读 docx（暗标）：Heading1 丢失"
    );
    assert_common("docx（暗标）", &body);
}

// ---------------------------------------------------------------- 2. pdf 回读（副面）

/// PDF 回读通道（引擎侧公共路径 / 纯 Rust 兜底）。
///
/// 纪律：**有首选路就用首选路**（引擎侧也是这个口径——`parse_file` 的
/// pdf 面先 pdfium/pdftotext、最后纯 Rust 兜底）。通道全断 → SKIP
/// 而不是假绿（issue #9）。
enum PdfChannel {
    /// 引擎侧解析（pdfium / pdftotext，由 `parse_file` 裁决）。
    Engine(bid_parse::ParsedFile),
    /// 纯 Rust 兜底（bid_parse::pdf_text 直读字节——零外部依赖）。
    Fallback(bid_parse::pdf_text::PdfText),
}

fn read_pdf(path: &Path, name: &str) -> Option<PdfChannel> {
    // 统一走 `parse_file`：它在 pdfium/pdftotext 全缺时本就落到纯 Rust 兜底
    // （bid-parse/src/lib.rs 模块头：「parse_file B4 后仅剩 pdf 面」）。
    //
    // 不再直调 `PdfText::extract` —— 那条路径返回 `PdfTextError`，该类型未从
    // `pdf_text` 模块再导出，测试侧够不着（E0599 根因），且语义与 parse_file 重复。
    let pf = bid_parse::parse_file(path, name).ok()?;
    if pf.has_text_layer && pf.blocks.iter().any(|b| !b.text.trim().is_empty()) {
        Some(PdfChannel::Engine(pf))
    } else {
        None
    }
}

impl PdfChannel {
    fn label(&self) -> &'static str {
        match self {
            PdfChannel::Engine(_) => "引擎通道（pdfium/pdftotext）",
            PdfChannel::Fallback(_) => "纯 Rust 兜底",
        }
    }
    fn pages(&self) -> usize {
        match self {
            PdfChannel::Engine(pf) => pf.pages,
            PdfChannel::Fallback(t) => t.pages.len(),
        }
    }
    fn text(&self) -> String {
        match self {
            PdfChannel::Engine(pf) => pf
                .blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            PdfChannel::Fallback(t) => t.lines().join("\n"),
        }
    }
}

#[test]
fn readback_pdf_text_layer_and_chinese_readable() {
    // 软门（issue #9）：soffice 不在 → PDF 产物根本不会生成 → SKIP 横幅
    if !bid_tools::service::soffice_available() {
        eprintln!(
            "【SKIP】readback_pdf_text_layer_and_chinese_readable——环境缺失: soffice\
             （无 LibreOffice 则无 PDF 产物可回读；安装后重跑，不伪装通过）"
        );
        return;
    }
    let mut fx = fixture("pdf-main", 12_000);
    let r = ok_result(call(
        &mut fx.svc,
        "export_bid_document",
        json!({
            "project_id": fx.pid, "format": "pdf",
            "output_path": "out/回读-投标文件.pdf",
            "profile": export_profile(),
        }),
    ));
    let url = r["artifact"]["url"].as_str().unwrap().to_string();
    let path = PathBuf::from(&url);

    // ① 魔数（回读的是真 PDF，不是改名文件）
    let head = std::fs::read(&path).unwrap();
    assert!(
        head.starts_with(b"%PDF-"),
        "回读 pdf：魔数不是 %PDF-（产物不是 PDF 字节流）"
    );

    let ch = match read_pdf(&path, "回读-投标文件.pdf") {
        Some(c) => c,
        None => {
            eprintln!(
                "【SKIP】readback_pdf_text_layer_and_chinese_readable——PDF 文本层读不出来\
                 （无 pdfium/poppler 且纯 Rust 兜底无文本层）；不伪装通过"
            );
            return;
        }
    };

    // ② 页数（12,000 字标书 ≥3 页——页数异常 = 转换管线只转了首页）
    let pages = ch.pages();
    assert!(pages >= 3, "回读 pdf：页数 {pages} < 3（正文未进 PDF）");

    // ③ 文本层在位（扫描化 PDF = 评委无法检索/复制）
    let text = ch.text();
    assert!(
        !text.trim().is_empty(),
        "回读 pdf（{}）：文本层为空（PDF 被栅格化——评审无法检索）",
        ch.label()
    );

    // ④ 中文可读（issue #169 守卫：解出来的到底是不是中文）
    let ratio = cjk_ratio(&text);
    if ratio < MIN_CJK_RATIO {
        eprintln!(
            "【SKIP】readback_pdf_text_layer_and_chinese_readable（{}）——文本层 CJK 占比 \
             {:.1}% < {:.0}%：中文解码不可用（issue #169 的缺陷面），不判红",
            ch.label(),
            ratio * 100.0,
            MIN_CJK_RATIO * 100.0
        );
        return;
    }

    // ⑤ 通用断言（占位符/未生成标记/字体 fallback）
    assert_common(&format!("pdf（{}）", ch.label()), &text);

    // ⑥ 正文在位（章标题/响应面进 PDF——转换不是空壳）
    assert!(
        fx.root_titles.iter().any(|t| text.contains(t.as_str())),
        "回读 pdf（{}）：outline 根章标题一个都不在文本层（转换丢失正文）: {:?}",
        ch.label(),
        fx.root_titles
    );

    // ⑦ 页脚页码域展开（目录页码连贯的 PDF 面证据）
    assert!(
        text.contains("第") && text.contains("页"),
        "回读 pdf（{}）：页脚页码文字不在文本层（页码域未由 Writer 展开）",
        ch.label()
    );

    println!(
        "[回读 pdf] {}｜{pages} 页｜文本 {} 字｜CJK {:.1}%",
        ch.label(),
        text.chars().count(),
        ratio * 100.0
    );
}

// ---------------------------------------------------------------- 3. xlsx 回读（对照面）

/// 一个单元格（独立解析出的值面——不是 calamine 的模型）。
#[derive(Debug, Clone, PartialEq)]
struct Cell {
    /// 单元格引用（A1/F24…）。
    reference: String,
    /// 单元格类型（t 属性；缺省 = n 数值）。
    kind: String,
    /// 解析出的值（s 型已按 sharedStrings 解出文本）。
    value: String,
}

/// xlsx 包的独立回读（解 zip 读 workbook/worksheets/sharedStrings）。
struct XlsxRead {
    /// 工作表名（workbook.xml 的 sheet name）。
    sheets: Vec<String>,
    /// 全部单元格（跨 sheet，按解析序）。
    cells: Vec<Cell>,
    /// 共享字符串表（s 型单元格的取值索引面——可能为空：inlineStr 型
    /// 工作簿不写 sharedStrings.xml，那是合法形态不是缺陷）。
    shared: Vec<String>,
    /// 包内部件名。
    parts: Vec<String>,
}

impl XlsxRead {
    /// 全部单元格文本（值面扫描的输入）。
    fn values_text(&self) -> String {
        self.cells
            .iter()
            .map(|c| c.value.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 单元格类型集合（去重保序——类型面断言：不许出现未知类型）。
    fn kinds(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in &self.cells {
            if !out.contains(&c.kind) {
                out.push(c.kind.clone());
            }
        }
        out
    }
}

/// 取 XML 标签的属性值（name="v" 形态；缺属性 → None）。
fn attr_of(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    tag.split_once(&needle)
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(v, _)| v.to_string())
}

/// 取 XML 元素的文本体（<t …>文本</t>——含属性形态也认）。
fn element_text(xml: &str, tag: &str) -> Option<String> {
    let open = xml.find(tag)?;
    let after = &xml[open..];
    let gt = after.find('>')?;
    let close_tag = format!("</{}>", &tag[1..]);
    let close = after[gt..].find(&close_tag)?;
    Some(decode_entities(&after[gt + 1..gt + close]))
}

/// XML 实体解码（数值引用 + 五个预定义实体）。
///
/// 为什么必须有：xlsx 的 inlineStr 常常把中文写成 `&#38468;` 形态（夹具
/// B3-图纸目录.xlsx 实测——整张表全是数值引用）。不解码的话「值面扫描」
/// 读到的是实体而不是中文，**回读面会把「解析器没解码」误判成「产线值
/// 丢了」**——假红，与 issue #9 的纪律相悖。
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let Some(i) = rest.find('&') else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(end) = after.find(';') else {
            out.push_str(after);
            return out;
        };
        let body = &after[1..end];
        let decoded = match body {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            hex if hex.starts_with("#x") || hex.starts_with("#X") => u32::from_str_radix(&hex[2..], 16)
                .ok()
                .and_then(char::from_u32),
            dec if dec.starts_with('#') => dec[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => out.push(c),
            // 未识别实体（自定义实体/截断）：原样保留——宁可留字面也不吞内容
            None => out.push_str(&after[..end + 1]),
        }
        rest = &after[end + 1..];
    }
}

fn read_xlsx(path: &Path) -> XlsxRead {
    let f = std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("回读打开 xlsx 失败（{e}）: {}", path.display()));
    let mut zip = zip::ZipArchive::new(f)
        .unwrap_or_else(|e| panic!("xlsx 不是合法 zip（{e}）: {}", path.display()));
    let mut parts = Vec::new();
    let mut workbook_src = String::new();
    let mut shared_src = String::new();
    let mut sheet_srcs: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).unwrap();
        parts.push(e.name().to_string());
        match e.name() {
            "xl/workbook.xml" => {
                e.read_to_string(&mut workbook_src).unwrap();
            }
            "xl/sharedStrings.xml" => {
                e.read_to_string(&mut shared_src).unwrap();
            }
            n if n.starts_with("xl/worksheets/") && n.ends_with(".xml") => {
                let mut s = String::new();
                e.read_to_string(&mut s).unwrap();
                sheet_srcs.push(s);
            }
            _ => {}
        }
    }
    // 工作表名（workbook.xml 的 <sheet name="…">）
    let mut sheets = Vec::new();
    let mut rest = workbook_src.as_str();
    while let Some(p) = rest.find("<sheet ") {
        if let Some(v) = attr_of(&rest[p..], "name") {
            sheets.push(v);
        }
        rest = &rest[p + 7..];
    }
    // 共享字符串（按 <si> 分块，块内全部 <t> 拼接——富文本会被多 run 切分）
    let mut shared = Vec::new();
    for si in shared_src.split("<si>").skip(1) {
        let end = si.find("</si>").unwrap_or(si.len());
        let mut s = String::new();
        let mut r = &si[..end];
        while let Some(t) = r.find("<t") {
            let after = &r[t..];
            let gt = after.find('>').unwrap_or(after.len());
            let close = after[gt..].find("</t>").unwrap_or(after.len());
            s.push_str(&after[gt + 1..gt + close]);
            r = &r[t + gt + 1 + close..];
        }
        shared.push(s);
    }
    // 单元格（逐 <c> 解引用/类型/值）
    let mut cells = Vec::new();
    for src in &sheet_srcs {
        let mut r = src.as_str();
        while let Some(p) = r.find("<c ") {
            let seg = &r[p..];
            let end = seg
                .find("</c>")
                .unwrap_or_else(|| seg.find("/>").unwrap_or(seg.len()));
            let body = &seg[..end];
            let reference = attr_of(body, "r").unwrap_or_default();
            // 类型面：缺省 = n（数值）；inlineStr/s/str/b/e/d 显式声明
            let kind = attr_of(body, "t").unwrap_or_else(|| "n".to_string());
            let value = match kind.as_str() {
                "inlineStr" => element_text(body, "<t").unwrap_or_default(),
                // s 型 = 共享字符串索引（取不到索引或越界 → 空串，由断言面报）
                "s" => element_text(body, "<v")
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .and_then(|i| shared.get(i).cloned())
                    .unwrap_or_default(),
                // 其余（n/str/b/e/d）：值在 <v>
                _ => element_text(body, "<v").unwrap_or_default(),
            };
            cells.push(Cell {
                reference,
                kind,
                value,
            });
            r = &r[p + 3..];
        }
    }
    XlsxRead {
        sheets,
        cells,
        shared,
        parts,
    }
}

/// 仓库既有 xlsx 夹具（corpus-A 的图纸目录——PROVENANCE 登记件）。
fn xlsx_corpus() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/corpus-A/files/B3-图纸目录.xlsx");
    p.is_file().then_some(p)
}

#[test]
fn readback_xlsx_cells_types_and_values() {
    // 语料在位门（issue #9）：夹具不在 → SKIP，绝不 pass 冒充通过
    let Some(path) = xlsx_corpus() else {
        eprintln!(
            "【SKIP】readback_xlsx_cells_types_and_values——xlsx 夹具不在\
             （fixtures/corpus-A/files/B3-图纸目录.xlsx）；不伪装通过"
        );
        return;
    };
    let x = read_xlsx(&path);

    // ① 结构完整：容器部件 + 至少一张 sheet + 工作表部件（无 sheet = 空工作簿）
    for req in ["[Content_Types].xml", "xl/workbook.xml", "xl/styles.xml"] {
        assert!(
            x.parts.iter().any(|p| p == req),
            "回读 xlsx：缺必需部件 {req}（包内: {:?}）",
            x.parts
        );
    }
    assert!(
        !x.sheets.is_empty(),
        "回读 xlsx：workbook.xml 内无 sheet（空工作簿——打开即空白）"
    );
    assert!(
        x.parts.iter().any(|p| p.starts_with("xl/worksheets/")),
        "回读 xlsx：无 worksheets 部件（表不存在）: {:?}",
        x.parts
    );

    // ② 单元格在位（图纸目录 A1:F24——值不能全丢）
    assert!(
        x.cells.len() >= 20,
        "回读 xlsx：单元格仅 {} 个（值面丢失——打开是空表）",
        x.cells.len()
    );

    // ③ 单元格类型在合法域（ECMA-376：b/e/d/n/str/inlineStr/s——
    //     出现域外类型 = 解析错位或产线写了非法类型）
    for k in x.kinds() {
        assert!(
            ["b", "e", "d", "n", "str", "inlineStr", "s"].contains(&k.as_str()),
            "回读 xlsx：单元格类型 {k:?} 不在合法域（类型面错位）: {:?}",
            x.kinds()
        );
    }

    // ④ 值面：图纸目录的表头与首行数据在座（单元格值真的解出来了）
    let vals = x.values_text();
    for expect in ["序号", "图纸编号", "图纸名称", "图幅", "张数", "版本"] {
        assert!(
            vals.contains(expect),
            "回读 xlsx：表头值「{expect}」不在单元格值面: {:?}",
            x.cells
                .iter()
                .map(|c| &c.value)
                .take(12)
                .collect::<Vec<_>>()
        );
    }
    assert!(
        vals.contains("GC-01") && vals.contains("弱电系统总图"),
        "回读 xlsx：首行数据（GC-01 / 弱电系统总图）不在值面——单元格值解析错"
    );

    // ⑤ s 型索引面自洽（有 s 型就必须能按 sharedStrings 解出非空值）
    for c in x.cells.iter().filter(|c| c.kind == "s") {
        assert!(
            !c.value.is_empty(),
            "回读 xlsx：s 型单元格 {} 解出空值（sharedStrings 索引面 {} 条——越界或解析错）",
            c.reference,
            x.shared.len()
        );
    }

    // ⑥ 通用断言（占位符/未生成标记/字体 fallback）
    assert_common("xlsx", &vals);

    // ⑦ 中文可读（图纸目录是中文件——解出来不是中文就是解码错）
    let ratio = cjk_ratio(&vals);
    assert!(
        ratio > 0.0,
        "回读 xlsx：单元格值零 CJK（中文内容丢失——值面不对）: {:?}",
        x.cells
            .iter()
            .map(|c| &c.value)
            .take(12)
            .collect::<Vec<_>>()
    );

    println!(
        "[回读 xlsx] {}｜sheet {:?}｜单元格 {}｜类型 {:?}｜共享串 {}｜CJK {:.1}%",
        path.file_name().unwrap().to_string_lossy(),
        x.sheets,
        x.cells.len(),
        x.kinds(),
        x.shared.len(),
        ratio * 100.0
    );
}

#[test]
fn readback_xlsx_channel_currently_refused_honestly() {
    // 产线现状登记（不上产线修，只把事实钉住）：导出器只放行 docx|pdf，
    // xlsx 必须**诚实拒**（E_UNPROCESSABLE + 指路），不得静默产出假 xlsx。
    // 回读面的 xlsx 能力由上一用例（对照夹具）证明——本用例钉「拒得诚实」。
    let mut fx = fixture("xlsx-refuse", 4_000);
    let reply = call(
        &mut fx.svc,
        "export_bid_document",
        json!({
            "project_id": fx.pid, "format": "xlsx",
            "output_path": "out/回读.xlsx",
        }),
    );
    match &reply {
        ToolReply::Err { error, .. } => {
            assert_eq!(
                error.code(),
                "E_UNPROCESSABLE",
                "xlsx 导出必须诚实拒（实际 {}）",
                error.code()
            );
            let msg = format!("{error}");
            assert!(
                msg.contains("docx") || msg.contains("pdf"),
                "拒绝理由须指路合法格式（实际: {msg}）"
            );
        }
        ToolReply::Ok(out) => panic!(
            "xlsx 通道当前不该产出——若已落地请补 xlsx 回读断言（产物: {}）",
            out.result
        ),
    }
    // 拒后不得留下半个 xlsx 文件（回读面：目录里出现 xlsx 就是假产物）
    let out_dir = fx.dir.join("out");
    if out_dir.is_dir() {
        for entry in std::fs::read_dir(&out_dir).unwrap() {
            let p = entry.unwrap().path();
            assert!(
                p.extension().and_then(|e| e.to_str()) != Some("xlsx"),
                "回读：拒绝后仍落盘 xlsx（假产物）: {}",
                p.display()
            );
        }
    }
}

