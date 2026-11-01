//! export.rs —— S-6 组装与流式导出（issue #11 批次 v3 / V3-5）。
//!
//! 定位：[`crate::assembly::ChapterAssembler`] 工作区（`chapters/*.json`）→
//! 标书 docx 的**只读消费半边**。组装层保证「每章生成完立即落盘、LLM 生成
//! 的代价永远不重复」；本模块把这条纪律延伸到导出层：**不重新生成任何章
//! 内容**——只读已提交的 ChapterRecord（`ChapterAssembler::open` 装载，零
//! generator 调用），组装失败/未完成时 fail-closed 拒绝导出，绝不静默跳章。
//!
//! ## 流式纪律（500 章不能拼成巨串）
//!
//! Lead 裁决（架构决策，不再发明）：**标书侧自己组装，不给引擎提增量写
//! API**——spike 实测 500 章百万字 docx-rs 内存模型 ~31MB ≪ 500MB。落地：
//! 1. 章内容按文档序**逐章** `add_paragraph` 进 docx-rs 模型（每章自带独立
//!    String，永不先拼一个百万字巨 String 再塞）；
//! 2. [`docx_rs::Docx::pack`] 直接把各 XML 部件流式写进 zip writer（0.4.22
//!    的流式通道——`zipper::zip_docx` 逐部件 `build_to(stream)`，不把整包
//!    XML 物化成 `Vec<u8>`）；
//! 3. 峰值内存 = 工作区记录（~3MB 文本）+ docx-rs 模型（~31MB spike 实测）
//!    ——硬验收 < 1GB 的余量三个数量级（export_stream.rs VmHWM 实测口径）。
//!
//! ## TOC 域（调研结论：docx-rs 0.4.22 原生支持，无需手写 fldSimple 注入）
//!
//! `docx_rs::TableOfContents` + `InstrToC` 是完整 TOC 域类型面：
//! `TableOfContents::new().heading_styles_range(1,3).hyperlink()
//! .use_applied_paragraph_line_level()` 渲染出（vendored 源
//! `documents/elements/table_of_contents.rs::test_toc` 为证）：
//! `w:fldChar begin(dirty=true)` + `w:instrText` = `TOC \o "1-3" \h \u` +
//! `separate` … `end`——正是规格要求的域指令（Word 打开后 F9 更新出目录；
//! `w:dirty="true"` 让 Word 打开时即提示/自动重算该域）。开关面沿用规格
//! 原文 `\o "1-3" \h \u`：标题层级 1-3 进目录（T-5 树最深 4 级——4 级标题
//! 仍是合法 Heading4 段落，只是不进目录，口径与规格字面一致）。各级标题用
//! Heading1..4 段落样式（`w:name` = "Heading 1".."Heading 4"——docx-rs
//! `Name::is_heading` 与 Word 内建样式名同口径，`TOC \o` 才能识别层级）。
//!
//! ## 原子产出（borrow：mdcore `sidecar.rs::SidecarStore::save` 的 tmp+rename
//! ## 纪律，经本仓 assembly.rs / store_io.rs 同款落地）
//!
//! 写 `{out}.docx.tmp` 全量 → `fs::rename` 替换（POSIX 原子；Windows 上
//! std::fs::rename 同为替换式）。**rename 返回即导出安全**——此后进程随时
//! 可死，产物完整；中断残骸只可能是 `*.docx.tmp`（下次导出启动时静默丢弃
//! ——borrow `interrupted_publish_leftover_does_not_break_recovery`）。幂等
//! 导出清单 `export-manifest.json`（与 chapters/ 平级，同款 tmp+rename）。
//!
//! ## 幂等导出清单（「已写入的章不重写」在导出层的落点）
//!
//! `export-manifest.json` 记录 { 源章指纹列表 (section_id + attempt +
//! content sha256，按文档序), 导出时间戳, 输出 sha256, 上次 ExportReport }。
//! 重跑 export：输入指纹全等 **且** 输出存在且 sha 吻合 → **no-op 直返上次
//! 报告**（输出一个字节不重写——产物 sha 对账 borrow `bid-domain::
//! export_gate.rs` 的「EXPORTED 以产物 sha256 落库为界」语义）；输入变了
//! （任一章 attempt/内容指纹漂移、增删章）→ 全量重建。manifest 是**导数**
//! 不是源数据：损坏/缺字段/未来版本 → 当作无 manifest 全量重建（真源
//! chapters/*.json 的 fail-closed 已在 `ChapterAssembler::open` 拦截——
//! 主数据严格、导数宽容，两层面各司其职）。
//!
//! ## fail-closed 导出门（半个文档不猜测——borrow mdcore 哲学）
//!
//! 1. 树为空 / 树不可全序覆盖（孤儿、环、重复 id——DFS 丢节点 = 静默半个
//!    文档，必须显式拒）；
//! 2. 已提交记录不在树中 → `GhostSection` 拒（重开纲后旧工作区不匹配，
//!    语义 borrow [`AssemblerError::GhostSection`]）；
//! 3. 记录挂在**结构节点**（非叶）上 → 拒（assemble_all 只对叶提交——父
//!    节点出现记录即工作区与树不匹配的怪态）；
//! 4. 叶章无 ChapterRecord → `MissingRecord` 拒（缺内容章绝不静默跳过）；
//! 5. 叶章状态未达 DRAFTED → `UndraftedSection` 拒（PENDING/OUTLINING/
//!    DRAFTING 都算「半个文档」；DRAFTED 及其后 VALIDATING/REVISING/
//!    ACCEPTED 均为已定稿面，放行）。「未 DRAFTED」只查**叶**——父节点是
//!    结构容器，assemble_all 不推进其状态（查全树会与 S-2 语义矛盾）。
//!
//! crate 纪律不变：时间戳用 SystemTime→UNIX_EPOCH 秒（u64，no-chrono）。

use crate::assembly::{AssemblerError, ChapterAssembler};
use crate::tables::{
    BidTable, EquipmentRow, PerformanceRow, PersonnelRow, PriceRow, TechDeviationRow, PENDING_FILL,
};
use bid_domain::evidence::{Evidence, EvidenceKind, EvidenceStatus};
use bid_domain::forms::{Blank, StandardForm};
use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::param::{Deviation, ParamResponse};
use bid_domain::party::BidderProfile;
use bid_domain::{DomainStore, Requirement};
use docx_rs::{
    DocumentChild, Docx, Paragraph, Pic, Run, Style, StyleType, Table, TableCell, TableOfContents,
    TableRow,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// 幂等导出清单文件名（与 `chapters/` 平级：`{dir}/export-manifest.json`）。
pub const MANIFEST_FILE: &str = "export-manifest.json";

/// 导出清单 schema 版本（结构性变更时 +1——当前仅 v1）。
const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// TOC 域开关（规格原文）：`TOC \o "1-3" \h \u`——标题层级 1-3 + 超链接 +
/// 应用段落大纲级别。构造常量函数，唯一真源。
fn toc_field() -> TableOfContents {
    // with_instr_text 直接吃规格原文（docx-rs 的 FromStr 解析 \o/\h/\u 开关
    // —— vendored `instr_toc.rs::with_instr_text` 单元测试锚定此行为：
    // `TOC \o "1-3" \h \z \u` 解析出 heading_styles_range(1,3)+hyperlink+
    // use_applied_paragraph_line_level）。相比链式 builder（TableOfContents 只
    // 暴露 heading_styles_range/hyperlink 两个开关，缺 \u），字符串面是唯一
    // 能落齐规格三开关的入口。
    TableOfContents::with_instr_text(r#"TOC \o "1-3" \h \u"#)
}

// ---------------------------------------------------------------- #117：表格/格式件接线（任务 1）

/// 导出时的**投标决策输入**（报价面——调用方显式给出，不入库、不猜测）。
///
/// 为什么不进 store：报价是**投标决策**（score_price 的试算产物/人工定盘），
/// 不是招标事实——真源在决策方手上；导出层只消费显式传入的值，缺了就落
/// 【待填】+ 响亮告警，绝不替投标人选一个数。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ExportDecision {
    /// 投标总报价（元；None = 未定——概况表/投标函落【待填】）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bid_amount: Option<f64>,
    /// 分项报价（报价明细表数据面；空 = 无报价决策 → 该表不生成 + 告警）。
    #[serde(default)]
    pub price_breakdown: Vec<PriceItem>,
    // ---- #117 任务 2：格式件决策面（投标承诺——同样不入库、缺省【待填】+ 告警）
    /// 项目官方名称（招标文件口径；缺省回落 store 项目名 + 告警——内部名≠官方名）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    /// 投标工期（日历天；None = 未定 → 投标函工期栏【待填】）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_days: Option<u32>,
    /// 投标有效期（日历天；None = 未定 → 投标函有效期栏【待填】）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity_days: Option<u32>,
    /// 投标保证金金额（元；None = 未定 → 退还申请金额栏【待填】）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deposit_amount: Option<f64>,
    /// 法定代表人性别（「男」/「女」——身份证明书要求写出；空 → 待填位）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legal_rep_gender: Option<String>,
}

/// 分项报价的一行（调用方口径：item/unit/quantity/unit_price 必填，note 可空）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PriceItem {
    /// 项目名称/费用构成（如「设备采购」「安装调试」）。
    #[serde(default)]
    pub item: String,
    /// 单位（批/项/台/套…）。
    #[serde(default)]
    pub unit: String,
    /// 数量。
    #[serde(default)]
    pub quantity: f64,
    /// 单价（元）。
    #[serde(default)]
    pub unit_price: f64,
    /// 备注。
    #[serde(default)]
    pub note: String,
}

/// 表格落点：锚定章节（标题关键词命中）或文末附表。
#[derive(Debug, Clone, PartialEq)]
pub enum TableAnchor {
    /// 章标题链（自身 + 祖先）任一命中关键词 → 注入该子树**最后一个叶章**
    /// 正文之后（表在章内容后——评委读完响应看表）。
    Section(&'static [&'static str]),
    /// 文末「附表」部分（关键词无命中的诚实回落——**不丢弃**：报价/业绩表
    /// 在大纲无对应章时仍要进标书，评委看得到、行数对得上）。
    Appendix,
}

/// 一张待导出的表格 + 它的落点。
#[derive(Debug, Clone, PartialEq)]
pub struct LocatedBidTable {
    pub table: BidTable,
    pub anchor: TableAnchor,
}

/// 导出附加件（#117 任务 1 表格 + 任务 2 格式件的容器）。
///
/// 由 [`build_export_extras`] 从三个真源派生：store（REQ/参数账/证据台账）、
/// [`BidderProfile`]（工作区 bidder-profile.json）、[`ExportDecision`]（调用方
/// 报价决策）。**零编造**：任一真源缺数据 → 该表不生成 + 告警，或生成带
/// 【待填】的模板行——绝不填 0/「无」这类假值。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportExtras {
    /// 六种结构化表格（按固定序：概况 → 偏离 → 报价 → 业绩 → 人员 → 设备）。
    pub tables: Vec<LocatedBidTable>,
    /// 七件格式件（任务 2 填充；任务 1 恒空——分线交付，接口先行）。
    pub forms: Vec<StandardForm>,
}

/// 表格的默认锚定关键词（章标题链任一命中即锚定；无命中 → 附表）。
fn anchor_keywords(table: &BidTable) -> TableAnchor {
    match table {
        BidTable::ProjectOverview { .. } => {
            TableAnchor::Section(&["项目概况", "项目概述", "招标公告"])
        }
        BidTable::TechDeviation { .. } => {
            TableAnchor::Section(&["技术参数", "技术方案", "技术响应", "技术需求"])
        }
        BidTable::Personnel { .. } => {
            TableAnchor::Section(&["人员配置", "项目团队", "组织机构", "人员", "团队"])
        }
        BidTable::Equipment { .. } => TableAnchor::Section(&["设备配置", "机具", "设备"]),
        BidTable::Performance { .. } => TableAnchor::Section(&["业绩", "类似项目"]),
        BidTable::PriceBreakdown { .. } => {
            TableAnchor::Section(&["报价", "开标一览", "商务部分", "价格"])
        }
    }
}

/// 从三个真源派生导出附加件（纯只读——不写 store、不推状态）。
///
/// 生成规则（诚实口径，逐表）：
/// - **项目概况表**：恒生成——项目名（store）、项目编号/最高限价（REQ 参数
///   ×摘要关键词定位）、投标总报价（决策，缺 → 【待填】）、投标人（profile）；
/// - **技术参数偏离表**：有「技术参数」类 REQ 即生成——参数响应账本
///   （`set_param_responses`）在场的 REQ 逐参数列**真响应 + 真偏离**；
///   账本缺失的 REQ 列原文要求 + 响应【待填】+ 偏离「不可比」；
/// - **报价明细表**：仅当决策带分项报价（合计行自动求和；与 bid_amount
///   不一致 → 响亮告警——评标否决点）；
/// - **业绩一览表**：仅当台账有可用（Active/Expiring）业绩证据；
/// - **人员/设备表**：仅当主体信息提供事实（空 → 不生成 + 告警）。
///
/// 返回 (附加件, 告警清单)——告警由服务层并入导出 warnings。
pub fn build_export_extras(
    store: &DomainStore,
    profile: &BidderProfile,
    decision: &ExportDecision,
) -> (ExportExtras, Vec<String>) {
    let mut warnings: Vec<String> = Vec::new();
    let mut tables: Vec<LocatedBidTable> = Vec::new();

    // ---- ① 项目概况表（恒生成——缺的栏位落【待填】，不缺表）
    tables.push(LocatedBidTable {
        table: build_overview_table(store, profile, decision),
        anchor: TableAnchor::Appendix, // 占位——统一在收尾处换成 anchor_keywords
    });
    if decision.bid_amount.is_none() {
        warnings.push(
            "投标总报价未随导出给出（profile.bid_amount）——概况表/投标函报价栏将落【待填】".into(),
        );
    }

    // ---- ② 技术参数偏离表（有技术参数类 REQ 即生成——账本缺行 → 待填模板行）
    let (tech, tech_warn) = build_tech_deviation_table(store);
    if let Some(t) = tech {
        tables.push(LocatedBidTable {
            table: t,
            anchor: TableAnchor::Appendix,
        });
    }
    warnings.extend(tech_warn);

    // ---- ③ 报价明细表（仅当分项报价在场——合计与总价对不上要响亮说）
    if !decision.price_breakdown.is_empty() {
        let rows = decision
            .price_breakdown
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut r = PriceRow::new(
                    i + 1,
                    p.item.clone(),
                    p.unit.clone(),
                    p.quantity,
                    p.unit_price,
                );
                r.note = p.note.clone();
                r
            })
            .collect();
        let t = BidTable::PriceBreakdown { rows };
        if let Some(bid) = decision.bid_amount {
            let total: f64 = decision
                .price_breakdown
                .iter()
                .map(|p| p.quantity * p.unit_price)
                .sum();
            let total = (total * 100.0).round() / 100.0;
            if (total - bid).abs() > 0.01 {
                warnings.push(format!(
                    "分项报价合计 {total:.2} 元 ≠ 投标总报价 {bid:.2} 元——两者必须一致（评标否决点：报价不唯一/开标一览表与投标函不一致）"
                ));
            }
        }
        tables.push(LocatedBidTable {
            table: t,
            anchor: TableAnchor::Appendix,
        });
    } else {
        warnings.push(
            "无分项报价决策（profile.price_breakdown）——报价明细表未生成（招标文件 7.2/7.5 要求分项报价不得缺漏项）".into(),
        );
    }

    // ---- ④ 业绩一览表（台账里可用的业绩证据——过期/被替代不算有）
    let perf_evs: Vec<&Evidence> = store
        .evidences()
        .iter()
        .filter(|e| e.kind == EvidenceKind::Performance && can_use(e))
        .collect();
    if !perf_evs.is_empty() {
        let rows = perf_evs
            .iter()
            .map(|e| PerformanceRow {
                project: e.title.clone(),
                client: Blank::or(&e.client.clone().unwrap_or_default(), "发包人"),
                amount_wan: e.amount_wan,
                signed: Blank::or(&e.valid_from.clone().unwrap_or_default(), "签订日期"),
                completed: Blank::or(&e.completed_on.clone().unwrap_or_default(), "完成日期"),
                evidence: e.id.clone(),
                note: String::new(),
            })
            .collect();
        tables.push(LocatedBidTable {
            table: BidTable::Performance { rows },
            anchor: TableAnchor::Appendix,
        });
    } else {
        warnings.push(
            "证据台账无可用业绩证据——业绩一览表未生成（readiness 闸会拦业绩章起草，导出面在此如实回报）".into(),
        );
    }

    // ---- ⑤ 人员配置表（主体信息事实）
    if !profile.personnel.is_empty() {
        let rows = profile
            .personnel
            .iter()
            .map(|f| PersonnelRow {
                position: f.position.clone(),
                name: f.name.clone(),
                title: f.title.clone(),
                major: f.major.clone(),
                years: f.years,
                note: f.note.clone(),
            })
            .collect();
        tables.push(LocatedBidTable {
            table: BidTable::Personnel { rows },
            anchor: TableAnchor::Appendix,
        });
    } else {
        warnings.push(
            "主体信息未提供拟派人员（bidder-profile.json 的 personnel）——人员配置表未生成".into(),
        );
    }

    // ---- ⑥ 设备清单表（主体信息事实）
    if !profile.equipment.is_empty() {
        let rows = profile
            .equipment
            .iter()
            .map(|f| EquipmentRow {
                name: f.name.clone(),
                model: f.model.clone(),
                quantity: f.quantity,
                unit: f.unit.clone(),
                origin: f.origin.clone(),
                spec: f.spec.clone(),
                arrival: f.arrival.clone(),
            })
            .collect();
        tables.push(LocatedBidTable {
            table: BidTable::Equipment { rows },
            anchor: TableAnchor::Appendix,
        });
    } else {
        warnings.push(
            "主体信息未提供拟投入设备（bidder-profile.json 的 equipment）——设备清单表未生成".into(),
        );
    }

    // 锚定统一换算（按表种类取默认关键词——上文 Appendix 只是构造占位）
    for lt in &mut tables {
        lt.anchor = anchor_keywords(&lt.table);
    }

    // ---- #117 任务 2：七件格式件（字段从 BidderProfile 铺管，缺 → 【待填】）
    let forms = build_export_forms(store, profile, decision, &mut warnings);

    (ExportExtras { tables, forms }, warnings)
}

/// 七件格式件构造（#117 任务 2——字段从 BidderProfile 铺管，缺 → 【待填】）。
///
/// 口径：
/// - 项目名：决策显式 > store 项目名（回落必告警——内部项目名与招标官方名
///   常不一致，投标函抬头错了就是形式审查瑕疵）；
/// - 报价/工期/有效期/保证金：决策面，缺 → 各自【待填】+ 告警；
/// - 报价大写：数字的确定性换写（不是新事实），None → 待填；
/// - 资格声明两项承诺按「是」出具（递交投标文件即该等承诺的作出动作）
///   + 响亮告警提醒调用方显式确认；
/// - 无违法声明的查询区间取决于开标日——**不替调用方定**，落【待填】+ 告警。
fn build_export_forms(
    store: &DomainStore,
    profile: &BidderProfile,
    decision: &ExportDecision,
    warnings: &mut Vec<String>,
) -> Vec<StandardForm> {
    let project_name = match &decision.project_name {
        Some(n) => n.clone(),
        None => {
            let n = store.project().map(|p| p.name.clone()).unwrap_or_default();
            warnings.push(format!(
                "项目名称未显式给出（profile.project_name）——格式件使用内部项目名「{n}」（应核对招标文件官方项目名）"
            ));
            n
        }
    };
    let project_no = req_param_by_excerpt(store, "项目编号", "model").unwrap_or_default();
    if decision.duration_days.is_none() {
        warnings
            .push("投标工期未随导出给出（profile.duration_days）——投标函工期栏将落【待填】".into());
    }
    if decision.validity_days.is_none() {
        warnings.push(
            "投标有效期未随导出给出（profile.validity_days）——投标函有效期栏将落【待填】".into(),
        );
    }
    if decision.deposit_amount.is_none() {
        warnings.push(
            "投标保证金金额未随导出给出（profile.deposit_amount）——退还申请金额栏将落【待填】"
                .into(),
        );
    }
    warnings.push(
        "资格声明的偿付能力/纳税社保两项按「是」出具（递交投标文件即承诺的作出动作——调用方应显式确认）".into(),
    );
    warnings.push(
        "无重大违法记录声明的查询区间未给出——按【待填】出具（区间取决于开标日，不由导出层替定）"
            .into(),
    );
    let gender = decision.legal_rep_gender.clone().unwrap_or_default();
    let mut forms = vec![
        StandardForm::bid_letter(
            profile,
            &project_name,
            &project_no,
            decision.bid_amount.unwrap_or(0.0),
            decision.duration_days.unwrap_or(0),
            decision.validity_days.unwrap_or(0),
        ),
        StandardForm::legal_rep_certificate(profile, &gender),
        StandardForm::power_of_attorney(profile, &project_name, "参加本项目投标并签署投标文件"),
        StandardForm::deposit_refund(
            profile,
            &project_name,
            decision.deposit_amount.unwrap_or(0.0),
        ),
        StandardForm::qualification_statement(profile, &project_name, true, true),
        // 查询区间空串 → 渲染层【待填】（开标日口径归调用方）
        StandardForm::no_violation_statement(profile, &project_name, "", "", false),
        StandardForm::integrity_commitment(profile, &project_name, vec![]),
    ];
    // 投标函大写金额回填（数字的确定性换写——壹仟伍佰捌拾万元整一类口径）
    if let Some(StandardForm::BidLetter {
        bid_amount_upper, ..
    }) = forms.first_mut()
    {
        *bid_amount_upper = amount_to_cn_upper(decision.bid_amount);
    }
    forms
}

/// 金额 → 人民币中文大写（#117 任务 2：投标函「大写」栏——纯换写，非新事实）。
///
/// 规则：万进制四位分段（万/亿/万亿）；段内零折叠（"壹仟零伍"）；跨段缺位
/// 补一个零（1 亿 0 万 5 → 壹亿零伍）；角分全零 → 「整」；金额缺失/非正 →
/// 空串（渲染层落【待填】——大写不能凭空造）。
pub fn amount_to_cn_upper(amount: Option<f64>) -> String {
    const D: [&str; 10] = ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"];
    const SCALES: [&str; 4] = ["", "万", "亿", "万亿"];
    let a = match amount {
        Some(v) if v.is_finite() && v > 0.0 => v,
        _ => return String::new(),
    };
    let cents = (a * 100.0).round() as u128;
    let yuan = cents / 100;
    let jiao = ((cents % 100) / 10) as u8;
    let fen = (cents % 10) as u8;

    /// 四位段 → 中文（1005 → 壹仟零伍；1580 → 壹仟伍佰捌拾）。
    fn section(n: u32) -> String {
        const D: [&str; 10] = ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"];
        const U: [&str; 4] = ["", "拾", "佰", "仟"];
        let digits = [
            (n / 1000) as usize,
            (n / 100 % 10) as usize,
            (n / 10 % 10) as usize,
            (n % 10) as usize,
        ];
        let mut out = String::new();
        let mut started = false;
        let mut zero_pending = false;
        for (i, d) in digits.iter().enumerate() {
            if *d == 0 {
                if started {
                    zero_pending = true;
                }
            } else {
                if zero_pending {
                    out.push('零');
                    zero_pending = false;
                }
                out.push_str(D[*d]);
                out.push_str(U[3 - i]);
                started = true;
            }
        }
        out
    }

    let mut parts = String::new();
    if yuan > 0 {
        let mut groups: Vec<(u32, usize)> = Vec::new();
        let mut rest = yuan;
        let mut si = 0usize;
        while rest > 0 {
            let g = (rest % 10000) as u32;
            if g > 0 {
                groups.push((g, si));
            }
            rest /= 10000;
            si += 1;
        }
        groups.reverse(); // 高段 → 低段
        for (idx, (g, si)) in groups.iter().enumerate() {
            if idx > 0 {
                let prev_si = groups[idx - 1].1;
                if prev_si - si > 1 || *g < 1000 {
                    parts.push('零'); // 跨段缺位（壹亿零伍）/低段高位为零（壹万零伍）
                }
            }
            parts.push_str(&section(*g));
            parts.push_str(SCALES[*si]);
        }
        parts.push('元');
    }
    // 角分（yuan>0 且 jiao==0 且 fen==0 → 整；jiao==0 但 fen>0 → 零X分）
    match (jiao, fen) {
        (0, 0) => {
            if yuan > 0 {
                parts.push_str("整");
            } else {
                return String::new();
            }
        }
        (j, 0) => {
            parts.push_str(D[j as usize]);
            parts.push_str("角");
        }
        (0, f) => {
            if yuan > 0 {
                parts.push('零');
            }
            parts.push_str(D[f as usize]);
            parts.push_str("分");
        }
        (j, f) => {
            parts.push_str(D[j as usize]);
            parts.push_str("角");
            parts.push_str(D[f as usize]);
            parts.push_str("分");
        }
    }
    parts
}

/// 证据可用判定（与 readiness.rs / evidence.rs `can_bind` 同口径——
/// Active / Expiring；过期件不能算「有业绩」）。
fn can_use(e: &Evidence) -> bool {
    matches!(e.status, EvidenceStatus::Active | EvidenceStatus::Expiring)
}

/// 项目概况表（项/值两列——评委第一眼直读面）。
///
/// 取值口径（全部确定性——同 store 同输出）：
/// - 项目名称 ← store.project().name；
/// - 项目编号 ← 首个「摘要含『项目编号』且 params.model 在场」的 REQ；
/// - 最高限价 ← 首个「摘要含 最高限价/招标控制价/预算金额 且 params.amount_wan
///   在场」的 REQ（万元口径原样落表）；
/// - 投标总报价 ← 决策（缺 → 【待填】）；
/// - 投标人 ← profile（缺 → 【待填】）。
fn build_overview_table(
    store: &DomainStore,
    profile: &BidderProfile,
    decision: &ExportDecision,
) -> BidTable {
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Ok(p) = store.project() {
        rows.push(("项目名称".into(), p.name.clone()));
    }
    if let Some(no) = req_param_by_excerpt(store, "项目编号", "model") {
        rows.push(("项目编号".into(), no));
    }
    if let Some(cap) = store
        .requirements()
        .iter()
        .filter(|r| {
            r.anchors
                .first()
                .map(|a| {
                    a.excerpt.contains("最高限价")
                        || a.excerpt.contains("招标控制价")
                        || a.excerpt.contains("预算金额")
                })
                .unwrap_or(false)
        })
        .find_map(|r| {
            r.params
                .get("amount_wan")
                .and_then(serde_json::Value::as_f64)
        })
    {
        rows.push(("招标控制价（最高限价）".into(), format!("{cap:.2} 万元")));
    }
    rows.push((
        "投标总报价".into(),
        decision
            .bid_amount
            .map(|a| format!("{a:.2} 元"))
            .unwrap_or_else(|| Blank::of("投标总报价")),
    ));
    rows.push((
        "投标人".into(),
        Blank::or(&profile.company_name, "投标人名称"),
    ));
    BidTable::ProjectOverview { rows }
}

/// 摘要含 `keyword` 且 `params[param_key]` 在场的首个 REQ → 参数字符串值。
fn req_param_by_excerpt(store: &DomainStore, keyword: &str, param_key: &str) -> Option<String> {
    store
        .requirements()
        .iter()
        .filter(|r| {
            r.anchors
                .first()
                .map(|a| a.excerpt.contains(keyword))
                .unwrap_or(false)
        })
        .find_map(|r| {
            r.params
                .get(param_key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
}

/// 技术参数偏离表（★评委打分主要依据——负偏离整行标红）。
///
/// 行口径：**每条技术参数 REQ 至少一行**（issue 任务 3 验收：行数 ≥ 技术参数
/// REQ 数）——参数响应账本在场的按账本逐参数列行（真响应 + 真偏离）；
/// 账本缺的 REQ 列要求原文 + 响应【待填】+ 偏离不可比（人工裁决面）。
fn build_tech_deviation_table(store: &DomainStore) -> (Option<BidTable>, Vec<String>) {
    let tech: Vec<&Requirement> = store
        .requirements()
        .iter()
        .filter(|r| r.category == "技术参数")
        .collect();
    if tech.is_empty() {
        return (None, vec!["无技术参数类 REQ——技术参数偏离表未生成".into()]);
    }
    // 参数账按 REQ 分组（同 REQ 多参数 = 多行——账本粒度就是比对粒度）
    let mut by_req: HashMap<&str, Vec<&ParamResponse>> = HashMap::new();
    for pr in store.param_responses() {
        by_req
            .entry(pr.requirement_id.as_str())
            .or_default()
            .push(pr);
    }
    let mut rows: Vec<TechDeviationRow> = Vec::new();
    let mut seq = 1usize;
    let mut unledgered = 0usize;
    for req in &tech {
        let excerpt = req
            .anchors
            .first()
            .map(|a| a.excerpt.clone())
            .unwrap_or_else(|| req.clause_no.clone().unwrap_or_else(|| req.id.clone()));
        match by_req.get(req.id.as_str()) {
            Some(prs) if !prs.is_empty() => {
                for pr in prs {
                    let mut row = TechDeviationRow::new(
                        seq,
                        format!("{}：{}", pr.param_name, pr.required_value),
                        pr.response_value.clone(),
                        pr.deviation,
                    );
                    if let Some(ev) = &pr.evidence_id {
                        row.note = format!("佐证：{ev}");
                    }
                    rows.push(row);
                    seq += 1;
                }
            }
            _ => {
                let mut row = TechDeviationRow::new(
                    seq,
                    excerpt,
                    PENDING_FILL.to_string(),
                    Deviation::Incomparable,
                );
                row.note = "响应未录入（set_param_responses 补录后自动判定偏离）".into();
                rows.push(row);
                seq += 1;
                unledgered += 1;
            }
        }
    }
    let mut warnings = Vec::new();
    if unledgered > 0 {
        warnings.push(format!(
            "技术参数偏离表有 {unledgered}/{} 条 REQ 无参数响应账——响应列落【待填】、偏离列「不可比」（补录 set_param_responses 后自动判定）",
            tech.len()
        ));
    }
    let t = BidTable::TechDeviation { rows };
    let neg = t.negative_count();
    if neg > 0 {
        warnings.push(format!(
            "技术参数偏离表含 {neg} 条负偏离——按当前响应不满足招标要求（废标级：技术参数不达标是高频否决原因）"
        ));
    }
    (Some(t), warnings)
}

// ---------------------------------------------------------------- 导出报告与清单

/// 导出报告（对账面——一轮 export 的产物账）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    /// 写入 docx 的章数（= 叶章数——每叶一标题 + 正文段落组）
    pub chapters_written: usize,
    /// 全部已写章正文字数和（char_count 口径，与 S-2 账面同源）
    pub total_chars: u64,
    /// 本次构建耗时（毫秒；no-op 直返上次值——报告全等性的一部分）。
    /// no-op 路径不重新计时（重计会让「输入未变 → 报告相等」永远失败）。
    pub duration_ms: Option<u64>,
    /// 输出 docx 的 sha256（十六进制小写）——产物指纹（export_gate 语义）
    pub output_sha256: String,
    /// TOC 域是否进正文（构造面断言：本导出器恒插 TOC 域，见 toc_field；
    /// 文件级验证在 export_stream.rs 以「document.xml 含 `TOC \o` 字样」
    /// 为准——报告位与测试断言双面同源）。
    pub toc_field_present: bool,

    // ---- #117 任务 1：表格/格式件对账面（serde default——旧清单兼容读入）
    /// 写入 docx 的结构化表格数（注入章内 + 附表部分）。
    #[serde(default)]
    pub tables_written: usize,
    /// 逐表种类名（「技术参数偏离表」…——e2e 断言/人工对账面）。
    #[serde(default)]
    pub table_kinds: Vec<String>,
    /// 写入 docx 的格式件数（任务 2 面；任务 1 恒 0）。
    #[serde(default)]
    pub forms_written: usize,
    /// 逐格式件标题（任务 2 面）。
    #[serde(default)]
    pub form_titles: Vec<String>,
}

/// 源章指纹（幂等对账的最小充分量：同章同版本同内容 → 三元组全等）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterFingerprint {
    pub section_id: String,
    /// ChapterRecord 的 attempt（内容修订 → +1 → 指纹变 → 全量重建）
    pub attempt: u32,
    /// content 的 sha256（十六进制小写）
    pub content_sha256: String,
}

/// 幂等导出清单（`{dir}/export-manifest.json`，tmp+rename 原子落盘）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportManifest {
    pub schema_version: u32,
    /// 按文档序的源章指纹列表（顺序确定——Vec 全等比较含序）
    pub fingerprints: Vec<ChapterFingerprint>,
    /// UNIX_EPOCH 秒（no-chrono 纪律）
    pub exported_at: u64,
    /// 上次导出报告（no-op 直返面——「一个字节不重写」的对价是报告可复现）
    pub report: ExportReport,
}

// ---------------------------------------------------------------- 错误面（typed）

/// 导出错误（typed——调用方按变体分支，不解析 message；变体风格与
/// [`AssemblerError`] / StoreIoError 同族）。
#[derive(Debug)]
pub enum ExportError {
    Io(std::io::Error),
    Serialize(serde_json::Error),
    /// docx 打包失败（docx-rs `Docx::pack` 的 ZipError 透传——终端 IO 面，
    /// 文本承载即可：zip 错误类型不在本 crate 的依赖面）
    Pack(String),
    /// 章节树为空（无内容可导出——不产出空文档假装成功）
    EmptyTree,
    /// 树中存在 DFS 全序不可达的节点（孤儿父引用/环/重复 id——静默丢节点
    /// 等于导出半个文档，必须显式拒）
    UnreachableSection {
        section_id: String,
    },
    /// 叶章状态未达 DRAFTED（PENDING/OUTLINING/DRAFTING——半个文档不猜测）
    UndraftedSection {
        section_id: String,
        status: String,
    },
    /// 叶章无 ChapterRecord（未提交——内容缺失，绝不静默跳章）
    MissingRecord {
        section_id: String,
    },
    /// 已提交记录不在章节树中（幽灵章——语义 borrow AssemblerError::GhostSection）
    GhostSection {
        section_id: String,
    },
    /// 记录挂在结构节点（非叶）上——assemble_all 只对叶提交，父节点出现
    /// 记录即工作区与树不匹配的怪态，fail-closed 拒
    RecordOnStructuralNode {
        section_id: String,
    },
    /// 组装工作区打开/校验失败（S-2 fail-closed 面透传——Corrupt 等原样上抛）
    Assembler(AssemblerError),
    /// #68 阶段 3：图片不可解析/不可读/非图片格式——**响亮拒绝不静默跳过**
    /// （issue §3.1 验收线；标书缺资质证书图是废标级事故）。
    ImageUnresolvable {
        target: String,
        why: String,
    },
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io(e) => write!(f, "导出 IO 失败: {e}"),
            ExportError::Serialize(e) => write!(f, "导出清单序列化失败: {e}"),
            ExportError::Pack(why) => write!(f, "docx 打包失败（zip 写入）: {why}"),
            ExportError::EmptyTree => write!(f, "章节树为空——无内容可导出（半个文档不猜测）"),
            ExportError::UnreachableSection { section_id } => write!(
                f,
                "章节 {section_id} 从根不可达（孤儿父引用/环/重复 id）——静默丢节点等于半个文档，显式拒绝"
            ),
            ExportError::UndraftedSection { section_id, status } => write!(
                f,
                "章节 {section_id} 状态为 {status}（未 DRAFTED）——半个文档不猜测，完成该章后重导"
            ),
            ExportError::MissingRecord { section_id } => write!(
                f,
                "章节 {section_id} 无已提交内容（ChapterRecord 缺失）——绝不静默跳章，续跑 assemble 后重导"
            ),
            ExportError::GhostSection { section_id } => write!(
                f,
                "已提交章 {section_id} 不在当前章节树中（幽灵章——重开纲后旧工作区不匹配，显式报错不猜测）"
            ),
            ExportError::RecordOnStructuralNode { section_id } => write!(
                f,
                "章节 {section_id} 是结构节点（非叶）却有已提交记录——工作区与树不匹配，显式报错不猜测"
            ),
            ExportError::Assembler(e) => write!(f, "组装工作区校验失败: {e}"),
            ExportError::ImageUnresolvable { target, why } => write!(
                f,
                "图片 {target} 无法嵌入: {why}——缺图不静默跳过（EV 引用查 workspace/files/，路径引用相对工作区根）"
            ),
        }
    }
}

impl std::error::Error for ExportError {}

// ---------------------------------------------------------------- 导出器

/// 组装产物导出器：章节树（文档序由内部 DFS 全序化）+ ChapterAssembler
/// 工作区 → 标书 docx。
///
/// 用法：`DocumentExporter::new(store.sections()).export(dir, out_path)`。
/// 树是**借用输入**（不可变——导出绝不改调度状态）；`export` 可反复调用
/// （幂等面由 manifest 承载）。
pub struct DocumentExporter<'a> {
    /// 章节树全集（任意序传入，内部按「根起 DFS + 兄弟按 order 升序」全序化）
    nodes: &'a [OutlineNode],
    /// #68 阶段 3：EV-id 图片解析面（`![说明](EV-0003)` → 证据台账 →
    /// source_file → workspace `files/` 下取件）。空切片 = 无 EV 解析
    /// （纯路径模式——既有调用方零变化）。
    evidences: &'a [Evidence],
}

/// 导出选项（#59 阶段 2：format-spec 应用 + 暗标模式）。
///
/// - `format_spec`：`None` = 纯默认面（无页眉页脚、docx-rs 默认 A4——
///   既有测试的零变化通道）；`Some` = 应用规格（页面/边距/Normal 字体行距/
///   页眉页脚，见 [`crate::format_spec::apply_to_docx`]）。
/// - `project_name`：页眉占位符替换素材。
/// - `anonymize`：暗标模式（Some(bidder_name) = 公司名 → 「投标人」——
///   正文与章标题同步替换；None = 明标）。
#[derive(Debug, Clone, Default)]
pub struct ExportOptions<'a> {
    pub format_spec: Option<&'a crate::format_spec::FormatSpec>,
    pub project_name: String,
    pub anonymize: Option<&'a str>,
    /// #117 任务 1：表格/格式件附加件（None = 纯章节面——既有调用方零变化；
    /// Some = 六表注入 + 附表 + 格式件，见 [`build_export_extras`]）。
    pub extras: Option<&'a ExportExtras>,
}

impl<'a> ExportOptions<'a> {
    /// 暗标替换（公司名 → 「投标人」；bidder 为空则原文不动——无名字可替
    /// 不是错误，但导出报告需告警，见服务层）。
    fn scrub(&self, text: &str) -> String {
        match self.anonymize {
            Some(bidder) if !bidder.is_empty() => text.replace(bidder, "投标人"),
            _ => text.to_string(),
        }
    }
}

impl<'a> DocumentExporter<'a> {
    /// 以章节树构造导出器。`nodes` 为空 → export 时 [`ExportError::EmptyTree`]。
    pub fn new(nodes: &'a [OutlineNode]) -> Self {
        Self {
            nodes,
            evidences: &[],
        }
    }

    /// 带 evidence 台账的构造（#68 阶段 3——`![说明](EV-xxxx)` 引用的
    /// 解析面：EV → source_file → workspace `files/{source_file}`）。
    pub fn with_evidences(nodes: &'a [OutlineNode], evidences: &'a [Evidence]) -> Self {
        Self { nodes, evidences }
    }

    /// 图片目标解析 + 读取（#68 阶段 3——EV-id / 相对路径 / 绝对路径）。
    fn resolve_image(&self, ws: &Path, target: &str) -> Result<Vec<u8>, ExportError> {
        let path = resolve_image_target(self.evidences, ws, target);
        std::fs::read(&path).map_err(|e| ExportError::ImageUnresolvable {
            target: target.to_string(),
            why: format!("{} 读取失败: {e}", path.display()),
        })
    }

    /// md 块序列追加进 docx 模型（#117 从章循环里提出的复用面——章正文、
    /// 注入表格、附表、格式件同走这一条渲染路径：段落/图片/表格 + 暗标
    /// 脱敏 + 红标渲染的规则只有一份）。
    fn append_blocks(
        &self,
        docx: Docx,
        blocks: Vec<Block>,
        opts: &ExportOptions<'_>,
        dir: &Path,
    ) -> Result<Docx, ExportError> {
        let mut docx = docx;
        for block in blocks {
            match block {
                Block::Para { text, bold } => {
                    let text = opts.scrub(&text);
                    // #117 任务 3：行内粗体在 run 发射层拆段（不是 parse_blocks
                    // 层——Block::Para 的文本契约不变，`**字段**：值` 这类
                    // 格式件字段行在这里变粗体 run，星号字面量零进 docx）。
                    let mut para = Paragraph::new();
                    let mut any_run = false;
                    for (seg, seg_bold) in split_bold_marks(&text) {
                        let mut run = Run::new().add_text(seg.as_str());
                        if bold || seg_bold {
                            run = run.bold();
                        }
                        para = para.add_run(run);
                        any_run = true;
                    }
                    if !any_run {
                        // 退化行（整行只有 `**`）：保底空 run——段落仍在，
                        // 星号不进 docx
                        para = para.add_run(Run::new().add_text(""));
                    }
                    docx = docx.add_paragraph(para);
                }
                Block::Image { alt, target } => {
                    let bytes = self.resolve_image(dir, &target)?;
                    let pic = build_pic(&bytes, &target)?;
                    docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_image(pic)));
                    let alt = opts.scrub(&alt);
                    if !alt.trim().is_empty() {
                        docx = docx.add_paragraph(
                            Paragraph::new()
                                .add_run(Run::new().add_text(format!("图：{alt}").as_str())),
                        );
                    }
                }
                Block::Table { rows } => {
                    let table = build_table(rows, opts);
                    docx = docx.add_table(table);
                }
            }
        }
        Ok(docx)
    }

    /// 一件格式件追加进 docx（#117 任务 2）。
    ///
    /// 渲染口径：格式件 markdown 的首行 `# 附件N、名` 提升为 **Heading2**
    /// （进 TOC \o 1-3——评委在目录里看得到七件在哪）；其余正文走通用
    /// [`Self::append_blocks`]（暗标脱敏/粗体/表格规则只有一份）。
    fn append_form(
        &self,
        docx: Docx,
        form: &StandardForm,
        opts: &ExportOptions<'_>,
        dir: &Path,
    ) -> Result<Docx, ExportError> {
        let md = form.to_markdown();
        let (first, rest) = match md.split_once('\n') {
            Some((a, b)) => (a, b),
            None => (md.as_str(), ""),
        };
        let title = first.strip_prefix("# ").unwrap_or(first).trim();
        let heading = Paragraph::new()
            .add_run(Run::new().add_text(opts.scrub(title)))
            .style("Heading2");
        let docx = docx.add_paragraph(heading);
        self.append_blocks(docx, parse_blocks(rest), opts, dir)
    }

    /// 导出标书 docx（唯一入口）。
    ///
    /// 步骤（顺序即注释里的序号——fail-closed 门全部在写盘**之前**）：
    /// ① tmp 残骸清理 → ② 工作区装载（S-2 fail-closed 透传）→ ③ 文档序
    /// 全序 + 树完整性 → ④ 导出门五查 → ⑤ 幂等 no-op 判定 → ⑥ 逐章构建
    /// docx 模型 → ⑦ tmp+rename 原子落盘 → ⑧ sha256 + manifest 原子落盘。
    pub fn export(
        &self,
        dir: impl AsRef<Path>,
        out_path: impl AsRef<Path>,
    ) -> Result<ExportReport, ExportError> {
        self.export_with(dir, out_path, &ExportOptions::default())
    }

    /// 带选项导出（#59 阶段 2：format-spec + 暗标）。
    ///
    /// 与 [`Self::export`] 同门同幂等面（manifest 指纹含 attempt + content
    /// sha——同指纹重导 no-op 直返）；差异仅在构建面：
    /// - format_spec 在场 → 页面/边距/Normal 字体行距/页眉页脚叠加；
    /// - anonymize 在场 → 正文与标题的公司名 → 「投标人」（暗标评审禁
    ///   公司名/logo——文本面替换；图片水印面属阶段 7 PDF 通道）。
    pub fn export_with(
        &self,
        dir: impl AsRef<Path>,
        out_path: impl AsRef<Path>,
        opts: &ExportOptions<'_>,
    ) -> Result<ExportReport, ExportError> {
        let started = Instant::now();
        let dir = dir.as_ref();
        let out_path = out_path.as_ref();

        // ① 中断残骸静默丢弃：上一次 export 死在 rename 前只可能留下 tmp。
        // （borrow：mdcore interrupted_publish_leftover_does_not_break_recovery
        // ——残骸不影响主文件，下次写入原子顶替；本导出器在入口先行清扫，
        // 让「no-op 直返」路径同样不留垃圾。）
        let tmp_path = sibling_tmp(out_path);
        match std::fs::remove_file(&tmp_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ExportError::Io(e)),
        }

        // ② 工作区装载：损坏/截断/char_count 失配 → S-2 Corrupt 透传（主数据
        // fail-closed——导出层不重复实现，单一真源）。
        let asm = ChapterAssembler::open(dir).map_err(ExportError::Assembler)?;

        // ③ 文档序全序化 + 树完整性（孤儿/环/重复 → 显式拒，见 order_tree）。
        let (ordered, children) = order_tree(self.nodes)?;

        // ④ 导出门（fail-closed——半页纸都不猜测）：
        let tree_ids: HashSet<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();
        let leaf_ids: HashSet<&str> = ordered
            .iter()
            .filter(|n| is_leaf(n, &children))
            .map(|n| n.id.as_str())
            .collect();
        for id in asm.committed_ids() {
            if !tree_ids.contains(id) {
                return Err(ExportError::GhostSection {
                    section_id: id.to_string(),
                });
            }
        }
        for id in asm.committed_ids() {
            if !leaf_ids.contains(id) {
                return Err(ExportError::RecordOnStructuralNode {
                    section_id: id.to_string(),
                });
            }
        }
        for node in &ordered {
            if !is_leaf(node, &children) {
                continue; // 结构节点：标题面，不查内容/状态（见模块头 fail-closed 第 5 条）
            }
            if !asm.has_committed(&node.id) {
                return Err(ExportError::MissingRecord {
                    section_id: node.id.clone(),
                });
            }
            if !is_drafted(node.status) {
                return Err(ExportError::UndraftedSection {
                    section_id: node.id.clone(),
                    status: node.status.as_str().to_string(),
                });
            }
        }

        // ⑤ 幂等 no-op：指纹全等 + 输出在位且 sha 吻合 → 上次报告直返
        // （输出零重写——「已写入的章不重写」的导出层落点）。
        let fingerprints: Vec<ChapterFingerprint> = ordered
            .iter()
            .filter(|n| is_leaf(n, &children))
            .map(|n| {
                // ④ 已保证叶有记录——unwrap 由前置门构造保证（不可达面）
                let rec = asm.record(&n.id).expect("叶记录已在导出门验证");
                ChapterFingerprint {
                    section_id: n.id.clone(),
                    attempt: rec.attempt,
                    content_sha256: sha256_hex(rec.content.as_bytes()),
                }
            })
            .collect();
        let manifest_path = dir.join(MANIFEST_FILE);
        if let Some(manifest) = read_manifest(&manifest_path)? {
            if manifest.fingerprints == fingerprints
                && out_path.is_file()
                && sha256_file(out_path)? == manifest.report.output_sha256
            {
                return Ok(manifest.report);
            }
        }

        // ⑥ 逐章构建 docx 模型（流式纪律：每章独立 add，永不拼巨串）。
        let mut docx = Docx::new();
        // Heading1..4 段落样式（w:name = "Heading 1".."Heading 4"——Word 内建
        // 样式名 + docx-rs Name::is_heading 同口径，TOC \o 识别层级的前提）。
        for lvl in 1..=4u8 {
            docx = docx.add_style(
                Style::new(format!("Heading{lvl}"), StyleType::Paragraph)
                    .name(format!("Heading {lvl}")),
            );
        }
        // #59 阶段 2：format-spec 应用（页面/边距/Normal 字体行距/页眉页脚
        // ——加性桥，不触碰下文的段落构建序）。None = 纯默认面（既有行为
        // 零变化——测试与旧调用方的兼容通道）。
        if let Some(spec) = opts.format_spec {
            docx = crate::format_spec::apply_to_docx(docx, spec, &opts.project_name);
        }
        // TOC 域插正文最前（Word F9 更新出目录；开关面见 toc_field 注释）。
        docx = docx.add_table_of_contents(toc_field());
        let mut chapters_written = 0usize;
        let mut total_chars = 0u64;
        // #117 任务 1：表格注入位解析（锚定章 → 该子树最后一个叶章；
        // 无命中 → 文末附表——不丢弃，见 TableAnchor 注释）。
        let (injections, appendix_tables) =
            resolve_table_injections(&ordered, &children, opts.extras);
        // #117 任务 2：格式件锚定（「投标文件格式」章尾——招标文件第七章的
        // 语义位；无该章 → 文末独立部分，见 ⑥c）
        let forms_anchor: Option<&str> = match opts.extras {
            Some(e) if !e.forms.is_empty() => {
                last_leaf_by_keyword(&ordered, &children, FORMS_ANCHOR_KEYWORDS)
                    .map(|n| n.id.as_str())
            }
            _ => None,
        };
        let mut tables_written = 0usize;
        let mut table_kinds: Vec<String> = Vec::new();
        let mut forms_written = 0usize;
        let mut form_titles: Vec<String> = Vec::new();
        for node in &ordered {
            // 标题段：Heading{level}（level 由 set_outline 保证 1..=4；防御性
            // clamp——手工构造的树不因此产出悬空 pStyle）。暗标：标题同步
            // 脱敏（章标题含公司名同样是暗标度标识别点）。
            let lvl = node.level.clamp(1, 4);
            let title = opts.scrub(node.title.as_str());
            let mut heading = Paragraph::new()
                .add_run(Run::new().add_text(title.as_str()))
                .style(&format!("Heading{lvl}"));
            if lvl == 1 {
                heading = heading.page_break_before(true); // 一级部件换页起（标书惯例）
            }
            docx = docx.add_paragraph(heading);
            if is_leaf(node, &children) {
                let rec = asm.record(&node.id).expect("叶记录已在导出门验证");
                // #68 阶段 3：正文按 md 块渲染（段落 / 图片 / 表格——issue
                // §3.1/§3.2）。段落行为与既有版全兼容（无 md 语法的内容
                // 字节级同输出）；图片缺失**响亮拒绝**（不静默跳过——fail-
                // closed 纪律与导出门同族）。暗标：正文/单元格/图注同步脱敏。
                docx = self.append_blocks(docx, parse_blocks(&rec.content), opts, dir)?;
                chapters_written += 1;
                total_chars += u64::from(rec.char_count);
                // #117：锚定本叶的表格紧随章内容（表在章尾——评委读完响应看表；
                // 暗标脱敏/红标渲染走同一 append_blocks 面，与章正文同规则）。
                if let Some(lts) = injections.get(node.id.as_str()) {
                    for lt in lts {
                        docx = self.append_blocks(
                            docx,
                            parse_blocks(&lt.table.to_markdown_titled()),
                            opts,
                            dir,
                        )?;
                        tables_written += 1;
                        table_kinds.push(lt.table.kind().to_string());
                    }
                }
                // #117 任务 2：七件格式件锚定在本章（「投标文件格式」子树
                // 最后一个叶章后）——按附件编号从一到七，Heading2 进 TOC。
                if forms_anchor == Some(node.id.as_str()) {
                    if let Some(extras) = opts.extras {
                        for form in &extras.forms {
                            docx = self.append_form(docx, form, opts, dir)?;
                            forms_written += 1;
                            form_titles.push(form.title());
                        }
                    }
                }
            }
        }

        // ⑥b #117：文末「附表」部分（无锚定命中的表格——报价/业绩/人员/设备
        //      在大纲无对应章时的诚实落点；一级部件换页起，与章部分同序）。
        if !appendix_tables.is_empty() {
            let heading = Paragraph::new()
                .add_run(Run::new().add_text(opts.scrub("附表")))
                .style("Heading1")
                .page_break_before(true);
            docx = docx.add_paragraph(heading);
            for lt in &appendix_tables {
                docx = self.append_blocks(
                    docx,
                    parse_blocks(&lt.table.to_markdown_titled()),
                    opts,
                    dir,
                )?;
                tables_written += 1;
                table_kinds.push(lt.table.kind().to_string());
            }
        }

        // ⑥c #117 任务 2：文末「投标文件格式件」部分（大纲无「投标文件格式」
        //      章时的诚实落点——七件按附件编号一至七，Heading2 进 TOC）。
        if forms_anchor.is_none() {
            if let Some(extras) = opts.extras.filter(|e| !e.forms.is_empty()) {
                let heading = Paragraph::new()
                    .add_run(Run::new().add_text(opts.scrub("投标文件格式件")))
                    .style("Heading1")
                    .page_break_before(true);
                docx = docx.add_paragraph(heading);
                for form in &extras.forms {
                    docx = self.append_form(docx, form, opts, dir)?;
                    forms_written += 1;
                    form_titles.push(form.title());
                }
            }
        }

        // ⑦ 段落 id 确定化：docx-rs 的 paraId 在 `Paragraph::new()` 时由**线程
        // 局部自增计数器**分配（vendored `documents/paragraph_id.rs`）——同进程
        // 内第二次构建从上次的计数值继续，`pack` 内部的 `reset_para_id` 只影响
        // 其后**新建**的段落（auto-TOC 扩展面），不回收已建段——document.xml
        // 因此逐字节不可复现（实测两次导出 paraId 面 `00000001..` vs
        // `000005A7..`）。导出层在 pack 前按**文档序**统一重排 id：
        // `00000001` 起单调递增——同输入恒同输出（S-6 确定性硬验收的前提；
        // id 唯一性由单调保证，`refresh_duplicate_para_ids` 零重派——vendored
        // 逐源核读，`auto=false` 的域式 TOC 不走 TocKey/bookmark 计数器面）。
        let mut next_para_id: u32 = 1;
        for child in docx.document.children.iter_mut() {
            if let DocumentChild::Paragraph(p) = child {
                p.id = format!("{next_para_id:08x}");
                next_para_id += 1;
            }
        }

        // ⑧ 原子落盘：tmp 写全量 → rename 替换（borrow：mdcore sidecar
        // tmp+rename 纪律）。pack 把 XML 部件流式写进 zip——rename 返回即
        // 导出安全，此后进程随时可死。
        let file = std::fs::File::create(&tmp_path).map_err(ExportError::Io)?;
        docx.pack(file)
            .map_err(|e| ExportError::Pack(e.to_string()))?;
        std::fs::rename(&tmp_path, out_path).map_err(ExportError::Io)?;

        // ⑨ 产物指纹 + 幂等清单落盘（导出后产物不可变——sha 是纯读）。
        let output_sha256 = sha256_file(out_path)?;
        let report = ExportReport {
            chapters_written,
            total_chars,
            duration_ms: Some(started.elapsed().as_millis() as u64),
            output_sha256: output_sha256.clone(),
            toc_field_present: true, // 构造面：toc_field() 恒插（见字段注释）
            tables_written,
            table_kinds,
            forms_written,
            form_titles,
        };
        let manifest = ExportManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            fingerprints,
            exported_at: now_secs(),
            report: report.clone(),
        };
        write_manifest_atomic(&manifest_path, &manifest)?;

        Ok(report)
    }
}

// ---------------------------------------------------------------- 私有辅助

/// #117 任务 2：格式件组的锚定关键词（招标文件「投标文件格式」章——
/// forms.rs 模块头锚定的同一语义位；不含裸「格式」——避免误中
/// 「格式要求」类条款章）。
const FORMS_ANCHOR_KEYWORDS: &[&str] = &["投标文件格式"];

/// 文档序最后一个「标题链（自身 + 祖先）含任一关键词」的叶章。
fn last_leaf_by_keyword<'a>(
    ordered: &[&'a OutlineNode],
    children: &BTreeMap<&'a str, Vec<&'a OutlineNode>>,
    keywords: &[&str],
) -> Option<&'a OutlineNode> {
    let by_id: HashMap<&str, &OutlineNode> = ordered.iter().map(|n| (n.id.as_str(), *n)).collect();
    ordered
        .iter()
        .rev()
        .filter(|n| is_leaf(n, children))
        .find(|n| {
            let mut cur: &OutlineNode = n;
            loop {
                if keywords.iter().any(|k| cur.title.contains(k)) {
                    return true;
                }
                match cur.parent_id.as_deref().and_then(|p| by_id.get(p)) {
                    Some(p) => cur = p,
                    None => return false,
                }
            }
        })
        .copied()
}

/// #117：解析表格落点 → (叶章 id → 注入表列表, 附表列表)。
///
/// 锚定规则（确定性）：`Section(keywords)` 的目标 = **文档序最后一个**标题链
/// （自身 + 全部祖先标题）含任一关键词的叶章——即整个命中子树的最后一个
/// 叶章，表格注入在子树全部内容之后（评委读完响应再看表）。无命中的表进
/// 附表（`Appendix` 或关键词全落空——不丢弃）。
fn resolve_table_injections<'a>(
    ordered: &[&'a OutlineNode],
    children: &BTreeMap<&'a str, Vec<&'a OutlineNode>>,
    extras: Option<&'a ExportExtras>,
) -> (
    HashMap<&'a str, Vec<&'a LocatedBidTable>>,
    Vec<&'a LocatedBidTable>,
) {
    let mut injections: HashMap<&str, Vec<&LocatedBidTable>> = HashMap::new();
    let mut appendix: Vec<&LocatedBidTable> = Vec::new();
    let extras = match extras {
        Some(e) if !e.tables.is_empty() => e,
        _ => return (injections, appendix), // 纯章节面（None / 空表——零开销直返）
    };
    // id → 节点索引（标题链回溯用）
    let by_id: HashMap<&str, &OutlineNode> = ordered.iter().map(|n| (n.id.as_str(), *n)).collect();
    for lt in &extras.tables {
        let keywords = match &lt.anchor {
            TableAnchor::Section(kws) => kws,
            TableAnchor::Appendix => &[] as &[&str],
        };
        let target = if keywords.is_empty() {
            None
        } else {
            ordered
                .iter()
                .rev()
                .filter(|n| is_leaf(n, children))
                .find(|n| {
                    // 标题链：自身 + 祖先（L1「技术参数」的子叶都命中）
                    let mut cur: &OutlineNode = n;
                    loop {
                        if keywords.iter().any(|k| cur.title.contains(k)) {
                            return true;
                        }
                        match cur.parent_id.as_deref().and_then(|p| by_id.get(p)) {
                            Some(p) => cur = p,
                            None => return false,
                        }
                    }
                })
        };
        match target {
            Some(node) => injections.entry(node.id.as_str()).or_default().push(lt),
            None => appendix.push(lt),
        }
    }
    (injections, appendix)
}

/// 叶判定（与 assemble_all 同口径：无子节点者）——O(1) 查 children 索引。
fn is_leaf(node: &OutlineNode, children: &BTreeMap<&str, Vec<&OutlineNode>>) -> bool {
    !children.contains_key(&node.id[..])
}

/// 状态是否已达 DRAFTED（含 VALIDATING/REVISING/ACCEPTED——均为已定稿面；
/// 线性链位次 ≥ Drafted）。
fn is_drafted(s: SectionStatus) -> bool {
    matches!(
        s,
        SectionStatus::Drafted
            | SectionStatus::Validating
            | SectionStatus::Revising
            | SectionStatus::Accepted
    )
}

/// 文档序全序化：根集合 + 兄弟按 order 升序 → DFS 前序遍历。
///
/// 返回 (全序节点列表, children 索引)。父节点必先于子节点出现（DFS 前序
/// 保证——plain order 排序在「父 order > 子 order」的树上会输出子先于父的
/// 坏文档，DFS 无此风险）。**完整性对账**：遍历输出数 ≠ 输入数 → 有节点
/// 从根不可达（孤儿父引用 / 环 / 重复 id）→ [`ExportError::UnreachableSection`]
/// 定位到第一个失踪者（静默丢节点 = 导出半个文档，fail-closed）。
fn order_tree<'a>(
    nodes: &'a [OutlineNode],
) -> Result<
    (
        Vec<&'a OutlineNode>,
        BTreeMap<&'a str, Vec<&'a OutlineNode>>,
    ),
    ExportError,
> {
    if nodes.is_empty() {
        return Err(ExportError::EmptyTree);
    }
    // 重复 id 防御（set_outline 已拦——手工构造面的兜底；重复会让 DFS 双计
    // 或错挂，完整性对账只能发现数量对不上，先在此显式拒）。
    let mut seen: HashSet<&str> = HashSet::new();
    for n in nodes {
        if !seen.insert(&n.id[..]) {
            return Err(ExportError::UnreachableSection {
                section_id: n.id.clone(),
            });
        }
    }
    let mut children: BTreeMap<&str, Vec<&OutlineNode>> = BTreeMap::new();
    let mut roots: Vec<&OutlineNode> = Vec::new();
    for n in nodes {
        match n.parent_id.as_deref() {
            None => roots.push(n),
            Some(p) => children.entry(p).or_default().push(n),
        }
    }
    roots.sort_by_key(|n| n.order);
    for v in children.values_mut() {
        v.sort_by_key(|n| n.order); // 兄弟序 = order 升序（order 全局唯一——稳定）
    }
    // DFS 前序（迭代式——显式栈，树深 ≤4 但迭代写法与数据规模解耦）。
    let mut ordered: Vec<&OutlineNode> = Vec::with_capacity(nodes.len());
    let mut stack: Vec<&OutlineNode> = roots.into_iter().rev().collect();
    while let Some(n) = stack.pop() {
        ordered.push(n);
        if let Some(kids) = children.get(&n.id[..]) {
            stack.extend(kids.iter().rev()); // 逆序入栈 → 弹出即文档序
        }
    }
    if ordered.len() != nodes.len() {
        // 定位首个失踪者：输入里第一个不在全序中的 id（确定性诊断面）
        let in_order: HashSet<&str> = ordered.iter().map(|n| &n.id[..]).collect();
        let missing = nodes
            .iter()
            .find(|n| !in_order.contains(&n.id[..]))
            .expect("数量不等必有失踪者");
        return Err(ExportError::UnreachableSection {
            section_id: missing.id.clone(),
        });
    }
    Ok((ordered, children))
}

/// 章正文切段（\n 为界，剥 \r，跳过空段）——迭代器借用面，零复制。
fn content_paragraphs(content: &str) -> impl Iterator<Item = &str> {
    content
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.is_empty())
}

// ---------------------------------------------------------------- #68 阶段 3：md 块解析与嵌入

/// 章正文的 md 块（issue §3.1/§3.2 的最小语法面）。
#[derive(Debug, Clone, PartialEq)]
enum Block {
    /// 普通段（bold = md 标题行 `#`/`##`… 剥号后的加粗段——章标题已由
    /// 节点标题段承载，内容层标题不重复占 TOC 层级）。
    Para { text: String, bold: bool },
    /// 图片：`![说明](路径或EV-id)`（整行独立成块）。
    Image { alt: String, target: String },
    /// 表格：连续 `|` 起始行（首行表头 + 分隔行 `|---|` + 数据行）。
    Table { rows: Vec<Vec<String>> },
}

/// 解析章正文为块序列（纯函数——同输入同输出，可单测）。
///
/// - 无 md 语法的内容 → 与旧逐行渲染**全兼容**（每非空行一个 Para）；
/// - 图片行/表格块之外的空行是段落边界，跳过（与旧行为一致）；
/// - 表格识别：≥2 行连续 `|` 行且第 2 行是分隔行（`|---|:--|` 形态）；
///   分隔行从输出行集中剔除（它是排版指令不是内容）。
fn parse_blocks(content: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut table_lines: Vec<&str> = Vec::new();

    let flush_table = |table_lines: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if table_lines.is_empty() {
            return;
        }
        let rows: Vec<Vec<String>> = table_lines.iter().map(|l| split_table_row(l)).collect();
        // 分隔行剔除：第 2 行是分隔形态 → 表头+数据；否则整个按普通段落回退。
        //
        // #103 B：判定必须**只认分隔行字符**（`| - : 空白`）。早先的全行
        // 字符白名单会被红标单元格里的 `{`/`r`/`:` 破坏——单元格文本若
        // 恰好含冒号（如「时间：7:30」）或红标标记，整张表就退化成段落。
        // 逐格判定（去掉竖线与空白后只允许 `-`/`:`）语义更准：真分隔行
        // 为「---」「:---:」形态，任何含正文的格都过不去。
        let is_separator_row = |cells: &[String]| {
            !cells.is_empty()
                && cells.iter().all(|c| {
                    let core: String = c.chars().filter(|ch| *ch != ' ' && *ch != '|').collect();
                    !core.is_empty()
                        && core.chars().all(|ch| ch == '-' || ch == ':')
                        && core.contains('-')
                })
        };
        if rows.len() >= 2
            && table_lines.iter().all(|l| l.starts_with('|'))
            && is_separator_row(&rows[1])
        {
            blocks.push(Block::Table {
                rows: rows
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| *i != 1)
                    .map(|(_, r)| r)
                    .collect(),
            });
        } else {
            // 非法表格形态（无分隔行）——原样回退段落（不吞内容）
            for l in table_lines.drain(..) {
                blocks.push(Block::Para {
                    text: l.to_string(),
                    bold: false,
                });
            }
            return;
        }
        table_lines.clear();
    };

    for raw in content.split('\n') {
        let line = raw.trim_end_matches('\r');
        let t = line.trim();
        if t.starts_with('|') {
            table_lines.push(t);
            continue;
        }
        flush_table(&mut table_lines, &mut blocks);
        if t.is_empty() {
            continue;
        }
        if let Some((alt, target)) = parse_image_line(t) {
            blocks.push(Block::Image { alt, target });
            continue;
        }
        if let Some(text) = strip_heading(t) {
            blocks.push(Block::Para { text, bold: true });
            continue;
        }
        // #117：全行粗体 `**文本**` → 加粗段（[crate::tables::BidTable::
        // to_markdown_titled] 的标题面走这里——不解析会把 `**` 字面量带进
        // docx，评标现场看到星号比看到加粗更糟）。整行一对定界在此提前
        // 剥除；行内混合粗体（`**a** 与 **b**`）保留在 Para 文本里，由
        // run 发射层的 [split_bold_marks] 拆段渲染（#117 任务 3——同样
        // 零星号进 docx）。
        if let Some(inner) = t.strip_prefix("**").and_then(|r| r.strip_suffix("**")) {
            if !inner.is_empty() && !inner.contains("**") {
                blocks.push(Block::Para {
                    text: inner.trim().to_string(),
                    bold: true,
                });
                continue;
            }
        }
        blocks.push(Block::Para {
            text: t.to_string(),
            bold: false,
        });
    }
    flush_table(&mut table_lines, &mut blocks);
    blocks
}

/// 图片行解析：`![说明](target)`（整行恰好一个图片语法——行内混合文本
/// 不识别，防误吞；说明可为空）。
fn parse_image_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("![")?;
    let close = rest.find("](")?;
    let alt = &rest[..close];
    let tail = &rest[close + 2..];
    let end = tail.rfind(')')?;
    if !tail.ends_with(')') || tail[..end].contains(')') {
        return None; // ) 后还有内容或目标内含 )——不识别（保守面）
    }
    let target = &tail[..end];
    if target.trim().is_empty() {
        return None;
    }
    Some((alt.trim().to_string(), target.trim().to_string()))
}

/// md 标题行剥号（`#`/`##`/`###`… → 文本；非标题行 None）。
fn strip_heading(line: &str) -> Option<String> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let text = line[hashes..].trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}

/// 表格行切单元格（剥首尾竖线 → `|` 切分 → 逐格 trim）。
///
/// #103 子任务 B：支持 GFM 的 `\|` 转义——被反斜杠转义的竖线是**单元格
/// 内容**不是分隔符，切分后去掉转义反斜杠还原原文。不处理转义会怎样：
/// 设备型号「DN100|PN16」会被切成两格（整行错位），而 [crate::tables]
/// 的 `BidTable::to_markdown` 正是显式转义后再交给本函数。
///
/// 红标标记（`{{r:文本}}`）不需要任何特殊处理——它的定界符是 `:`，与
/// 列分隔符 `|` 正交（这正是不用 `{{r|…}}` 的原因，见 [crate::tables]。
fn split_table_row(line: &str) -> Vec<String> {
    let inner = line.strip_prefix('|').unwrap_or(line);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some(next) if next == '|' => cur.push('|'),
                Some(next) => {
                    cur.push('\\');
                    cur.push(next);
                }
                None => cur.push('\\'),
            },
            '|' => {
                cells.push(cur.trim().to_string());
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}
/// 图片目标解析（EV-id → 证据源文件；路径 → 相对 workspace 或绝对）。
fn resolve_image_target(evidences: &[Evidence], ws: &Path, target: &str) -> PathBuf {
    if let Some(ev) = evidences.iter().find(|e| e.id == target) {
        return ws.join("files").join(&ev.source_file);
    }
    let p = Path::new(target);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        ws.join(p)
    }
}

/// 版面宽上限（内容区 6 英寸 @96dpi——A4 默认边距的安全面；超出等比缩放）。
const MAX_CONTENT_WIDTH_PX: u32 = 576;

/// 图片字节 → docx Pic（PNG 直传 / 其他格式转 PNG；宽超限等比缩放）。
///
/// 错误面**不 panic**（docx-rs `Pic::new` 内部 expect 的 IO 面——本函数
/// 预验证 + 显式 Err，与模块 fail-closed 纪律一致）。
fn build_pic(bytes: &[u8], target: &str) -> Result<Pic, ExportError> {
    use image::GenericImageView;
    let unresolvable = |why: String| ExportError::ImageUnresolvable {
        target: target.to_string(),
        why,
    };
    let img = image::load_from_memory(bytes)
        .map_err(|e| unresolvable(format!("不是可识别的图片格式: {e}")))?;
    let (w, h) = img.dimensions();
    let png: Vec<u8> = if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        bytes.to_vec() // PNG 直传（免重编码——与 docx-rs Pic::new 的短路同路）
    } else {
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|e| unresolvable(format!("图片转 PNG 失败: {e}")))?;
        buf
    };
    // 版面适配：宽超 6 英寸等比缩到界内（Word 页宽硬约束；高同比——
    // 特长图（如扫描长页）高分屏不炸版）
    let (mw, mh) = if w > MAX_CONTENT_WIDTH_PX {
        let scale = MAX_CONTENT_WIDTH_PX as f64 / w as f64;
        let mh = (h as f64 * scale).round().max(1.0) as u32;
        (MAX_CONTENT_WIDTH_PX, mh)
    } else {
        (w, h)
    };
    Ok(Pic::new_with_dimensions(png, mw, mh))
}

/// 红标 run 的颜色（纯红——负偏离/废标级风险项的唯一色彩语义）。
///
/// #103 子任务 B：标书的「颜色」只有一种用法——把**不满足招标要求**的
/// 单元格戳出来。多一种颜色就多一种解释，评标现场不需要配色方案。
const RED_RUN_COLOR: &str = "FF0000";

/// md 表格行集 → docx Table（表头加粗；暗标脱敏逐格应用；红标标记渲染）。
///
/// #103 子任务 B：单元格内的 `{{r|文本}}` 行内标记（[crate::tables] 的
/// [`RED_MARK_OPEN`] 语法）渲染为**红色 run**——技术参数偏离表用它把负
/// 偏离整行标红。标记之外的文本原样渲染（无标记的单元格与既有版同输出）。
///
/// 为什么走行内标记而不是给 build_table 加一个「第 N 行标红」参数：表格
/// 数据从 markdown 来（[crate::tables::BidTable::to_markdown]），导出层
/// 只认识 md 文本；把表达力放进文本里，markdown 就仍然自足（纯文本阅读
/// 时可读、可 diff、可诊断），导出层只负责渲染。
///
/// [`RED_MARK_OPEN`]: crate::tables::RED_MARK_OPEN
fn build_table(rows: Vec<Vec<String>>, opts: &ExportOptions<'_>) -> Table {
    let doc_rows: Vec<TableRow> = rows
        .into_iter()
        .enumerate()
        .map(|(i, cells)| {
            let doc_cells: Vec<TableCell> = cells
                .into_iter()
                .map(|c| {
                    let mut para = Paragraph::new();
                    for (text, is_red) in split_red_marks(&c) {
                        let text = opts.scrub(&text);
                        // #117 任务 3：单元格同享行内粗体拆段（红标与粗体
                        // 正交组合——红段内的 `**…**` 也渲染为粗体）
                        let mut any_run = false;
                        for (seg, seg_bold) in split_bold_marks(&text) {
                            let mut run = Run::new().add_text(seg.as_str());
                            if i == 0 || seg_bold {
                                run = run.bold(); // 表头行加粗（无样式表依赖的稳妥面）
                            }
                            if is_red {
                                run = run.color(RED_RUN_COLOR);
                            }
                            para = para.add_run(run);
                            any_run = true;
                        }
                        // 空红段（`{{r:}}`——负偏离行的空备注格）：保底空 run
                        // 带色——整行标红的「整行」语义不因空格破缺
                        if !any_run {
                            let mut run = Run::new().add_text("");
                            if i == 0 {
                                run = run.bold();
                            }
                            if is_red {
                                run = run.color(RED_RUN_COLOR);
                            }
                            para = para.add_run(run);
                        }
                    }
                    TableCell::new().add_paragraph(para)
                })
                .collect();
            TableRow::new(doc_cells)
        })
        .collect();
    Table::new(doc_rows)
}

/// 单元格文本按红标标记切段 → [(文本, 是否红色)]。
///
/// 语法：`前缀{{r:红色段}}后缀` → [("前缀", false), ("红色段", true),
/// ("后缀", false)]。无标记 → 单段 [("原文", false)]；空串 → 空 vec
/// （调用方补空 run——docx 单元格必须有段落）。
///
/// 容错纪律：未闭合的 `{{r:` **按普通文本处理**——半截标记是数据错误，
/// 静默吞掉后半格文本比原样显示标记更糟。
fn split_red_marks(cell: &str) -> Vec<(String, bool)> {
    let open = crate::tables::RED_MARK_OPEN;
    let close = crate::tables::RED_MARK_CLOSE;
    let mut segments: Vec<(String, bool)> = Vec::new();
    let mut rest = cell;
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(close) else { break }; // 未闭合：整体按普通文本
        let (head, red_text) = (&rest[..start], &after[..end]);
        if !head.is_empty() {
            segments.push((head.to_string(), false));
        }
        segments.push((red_text.to_string(), true));
        rest = &after[end + close.len()..];
    }
    if !rest.is_empty() {
        segments.push((rest.to_string(), false));
    }
    if segments.is_empty() && !cell.is_empty() {
        segments.push((cell.to_string(), false));
    }
    segments
}
/// `out` + ".tmp" 的同目录兄弟路径（纯后缀追加——不猜调用方的扩展名形态）。
fn sibling_tmp(out: &Path) -> PathBuf {
    let mut s = out.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

/// 行内粗体切段（#117 任务 3）：`**a**b` → [("a", true), ("b", false)]。
///
/// 语义：`**` 定界符按 markdown 交替开/关（toggle）。配对完好时与
/// CommonMark 一致；出现落单 `**`（LLM 生成文本的常见碎屑）时其后
/// 文本按粗体渲染——**不是**把星号字面量带进 docx（渲染面零星号是
/// #117 任务 3 的验收线：半截标记宁可视作强调，不可漏成垃圾字符）。
/// 纯文本 → 单段；空串/纯定界符 → 空 vec（调用方保底空 run）。
fn split_bold_marks(text: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut bold = false;
    for seg in text.split("**") {
        if !seg.is_empty() {
            out.push((seg.to_string(), bold));
        }
        bold = !bold;
    }
    out
}

/// 当前 UNIX_EPOCH 秒（no-chrono 纪律；时钟异常 → 0——单调性由 attempt 承载）。
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// sha256（十六进制小写）——字节串一次性口径（章内容 ≤ 数 KB）。
fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// 文件流式 sha256（64KiB 块——产物 MB 序，不整读进内存）。
fn sha256_file(path: &Path) -> Result<String, ExportError> {
    let mut f = std::fs::File::open(path).map_err(ExportError::Io)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf).map_err(ExportError::Io)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// 读 manifest（**导数宽容**面：缺失/损坏/版本不识别 → None 全量重建；
/// 主数据的 fail-closed 在 ChapterAssembler::open——两层面各司其职，见
/// 模块头「幂等导出清单」节）。
fn read_manifest(path: &Path) -> Result<Option<ExportManifest>, ExportError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ExportError::Io(e)),
    };
    match serde_json::from_str::<ExportManifest>(&raw) {
        Ok(m) if m.schema_version == MANIFEST_SCHEMA_VERSION => Ok(Some(m)),
        _ => Ok(None), // 损坏/未来版本 → 无清单重建（导数不挡路）
    }
}

/// manifest 原子落盘（tmp+rename——borrow：mdcore sidecar save 同款纪律）。
fn write_manifest_atomic(path: &Path, manifest: &ExportManifest) -> Result<(), ExportError> {
    let json = serde_json::to_string_pretty(manifest).map_err(ExportError::Serialize)?;
    let tmp = sibling_tmp(path);
    std::fs::write(&tmp, json.as_bytes()).map_err(ExportError::Io)?;
    std::fs::rename(&tmp, path).map_err(ExportError::Io) // 原子提交点
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TOC 域开关渲染面（vendored docx-rs 单元测试同款断言——`TOC \o "1-3"
    /// \h \u` 的 XML 字面）。锚定「构造的域指令 == 规格原文」。
    #[test]
    fn toc_field_renders_spec_instruction() {
        use docx_rs::BuildXML as _;
        let xml = toc_field().instr.build();
        let s = String::from_utf8(xml).unwrap();
        assert!(
            s.contains(r#"TOC \o &quot;1-3&quot; \h \u"#),
            "TOC 域指令必须是规格原文（TOC \\o \"1-3\" \\h \\u，XML 引号转义形态）: {s}"
        );
    }

    /// #103 B：红标标记切段（markdown 层 → run 段）——标红渲染的前置面。
    #[test]
    fn split_red_marks_segments_cell() {
        assert_eq!(
            split_red_marks("{{r:2年}}"),
            vec![("2年".to_string(), true)],
            "整格标红 → 单段红色"
        );
        assert_eq!(
            split_red_marks("前缀{{r:红}}后缀"),
            vec![
                ("前缀".to_string(), false),
                ("红".to_string(), true),
                ("后缀".to_string(), false)
            ],
            "混合格 → 三段"
        );
        assert_eq!(
            split_red_marks("纯文本"),
            vec![("纯文本".to_string(), false)]
        );
        assert_eq!(split_red_marks(""), Vec::new());
        // 未闭合：按普通文本（不吞内容）
        assert_eq!(
            split_red_marks("{{r:半截"),
            vec![("{{r:半截".to_string(), false)]
        );
    }

    /// #117：全行粗体 `**文本**` → 加粗段（表格标题面——`**` 字面量
    /// 不许进 docx）；行内混合粗体在解析层保留原文本，由 run 发射层
    /// 的 [split_bold_marks] 渲染为粗体段（#117 任务 3）。
    #[test]
    fn full_line_bold_parses_as_bold_para_not_literal_asterisks() {
        let blocks =
            parse_blocks("**技术参数偏离表**\n\n| 序号 | 要求 |\n| --- | --- |\n| 1 | ≥100GB |");
        assert_eq!(
            blocks[0],
            Block::Para {
                text: "技术参数偏离表".to_string(),
                bold: true
            },
            "全行粗体 → 加粗段（不带星号）: {blocks:?}"
        );
        assert!(matches!(blocks[1], Block::Table { .. }), "表格行照常成表");
        // 行内混合粗体（两对定界）→ 解析层保留原文本（run 发射层拆段）
        let mixed = parse_blocks("**a** 与 **b**");
        assert_eq!(
            mixed[0],
            Block::Para {
                text: "**a** 与 **b**".to_string(),
                bold: false
            },
            "混合粗体解析层不猜，发射层拆段: {mixed:?}"
        );
        // 空粗体 `****` → 普通段（不产空加粗段）
        let empty = parse_blocks("****");
        assert_eq!(
            empty[0],
            Block::Para {
                text: "****".to_string(),
                bold: false
            }
        );
    }

    /// #117 任务 3：行内粗体切段（run 发射层）——格式件字段行
    /// `**姓名**：王锐` 的星号零进 docx，字段名成粗体 run。
    #[test]
    fn split_bold_marks_segments_inline() {
        // 格式件字段行（授权委托书）：前缀无 → 字段名粗体 + 值普通
        assert_eq!(
            split_bold_marks("**姓名**：王锐"),
            vec![("姓名".to_string(), true), ("：王锐".to_string(), false)],
            "字段行 → 粗体字段名 + 普通值"
        );
        // 行内多对定界
        assert_eq!(
            split_bold_marks("**a** 与 **b**"),
            vec![
                ("a".to_string(), true),
                (" 与 ".to_string(), false),
                ("b".to_string(), true)
            ]
        );
        // 纯文本 → 单段（与旧单 run 行为字节级一致）
        assert_eq!(
            split_bold_marks("工期 180 日历天"),
            vec![("工期 180 日历天".to_string(), false)]
        );
        // 落单定界（LLM 碎屑）：toggle 语义——后续按粗体，星号不进 docx
        assert_eq!(
            split_bold_marks("落单**后半"),
            vec![("落单".to_string(), false), ("后半".to_string(), true)]
        );
        // 空串 / 纯定界符 → 空 vec（调用方保底空 run）
        assert_eq!(split_bold_marks(""), Vec::new());
        assert_eq!(split_bold_marks("**"), Vec::new());
        assert_eq!(split_bold_marks("****"), Vec::new());
    }

    /// #103 B：md 表格行 → 单元格切分（红标标记不被列分隔符误伤）。
    #[test]
    fn table_row_split_keeps_red_marks_whole() {
        let cells = split_table_row("| {{r:2}} | {{r:负偏离}} |");
        assert_eq!(
            cells,
            vec!["{{r:2}}".to_string(), "{{r:负偏离}}".to_string()]
        );
        // GFM 转义竖线仍是内容
        assert_eq!(
            split_table_row("| a\\|b | c |"),
            vec!["a|b".to_string(), "c".to_string()]
        );
    }

    /// 文档序全序化：兄弟按 order、父先于子；孤儿（父引用不存在）→ 显式拒。
    #[test]
    fn order_tree_orders_and_rejects_orphan() {
        let mk = |id: &str, parent: Option<&str>, level: u8, order: u32| OutlineNode {
            id: id.into(),
            parent_id: parent.map(str::to_string),
            title: format!("{id} 标题"),
            level,
            order,
            requirement_ids: vec!["REQ-0001".into()],
            target_chars: 100,
            status: SectionStatus::Drafted,
            dirty: false,
            content: None,
            content_attempt: 0,
        };
        // 正常态：根 A(order 1) → 子 A1(order 3)、A2(order 2)；根 B(order 4)。
        // A2 的 order < A1 但同为 A 的子——兄弟序按 order：A1 前还是后？
        // A1 order=3、A2 order=2 → 兄弟序 [A2, A1]（order 升序，非插入序）。
        let nodes = vec![
            mk("A", None, 1, 1),
            mk("A1", Some("A"), 2, 3),
            mk("A2", Some("A"), 2, 2),
            mk("B", None, 1, 4),
        ];
        let (ordered, children) = order_tree(&nodes).unwrap();
        let ids: Vec<&str> = ordered.iter().map(|n| &n.id[..]).collect();
        assert_eq!(
            ids,
            vec!["A", "A2", "A1", "B"],
            "父先于子 + 兄弟 order 升序"
        );
        assert!(!is_leaf(&nodes[0], &children), "A 是结构节点");
        assert!(is_leaf(&nodes[1], &children), "A1 是叶");
        assert!(is_leaf(&nodes[3], &children), "B 是叶");

        // 孤儿：父引用不存在 → DFS 不可达 → 显式拒（不静默丢）
        let orphan = vec![mk("A", None, 1, 1), mk("X", Some("NOPE"), 2, 2)];
        match order_tree(&orphan) {
            Err(ExportError::UnreachableSection { section_id }) => {
                assert_eq!(section_id, "X", "报错定位到孤儿节点");
            }
            other => panic!("孤儿必须 UnreachableSection 拒，实际: {:?}", other.err()),
        }

        // 重复 id → 显式拒（手工构造面兜底）
        let dup = vec![mk("A", None, 1, 1), mk("A", None, 1, 2)];
        assert!(
            matches!(
                order_tree(&dup),
                Err(ExportError::UnreachableSection { .. })
            ),
            "重复 id 必须拒"
        );

        // 空树 → EmptyTree
        assert!(matches!(order_tree(&[]), Err(ExportError::EmptyTree)));
    }
}

// ---------------------------------------------------------------- #59 阶段 2：服务面产物

/// `export_bid_document` 的服务层产物（MCP 壳组装契约 §8 out 的素材面）。
#[derive(Debug, Clone)]
pub struct ExportOutcome {
    pub report: ExportReport,
    /// 服务层告警（format-spec 降级 / 暗标漏配 bidder_name 等）
    pub warnings: Vec<String>,
    /// 产物绝对路径（壳层补 url/size 面）
    pub output_path: PathBuf,
}

/// `ExportError` → `bid_domain::ToolError`（服务层转发面——导出失败统一
/// E_UNPROCESSABLE 带原文；导出门（blocking/幽灵/半个文档）都是「不可处理
/// 的状态」，不是「对象不存在」或「状态机越级」）。
pub fn outcome_error(e: ExportError) -> bid_domain::ToolError {
    bid_domain::ToolError::EUnprocessable {
        reason: e.to_string(),
    }
}
