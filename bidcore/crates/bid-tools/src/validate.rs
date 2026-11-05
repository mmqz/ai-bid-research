//! validate.rs —— #59 阶段 3：`validate_bid_document` 的业务半边。
//!
//! **定位：防废标的最后一道关**（Lead 原文）。双层校验（契约 §7 / R-07
//! 规则分界）：
//! - **第一层（确定性，先行必跑）**——代码断言零模型：
//!   1. `E_VETO_UNRESPONDED`（BLOCKER）：否决项类 REQ 矩阵行未响应；
//!   2. `E_QUAL_UNMET`（BLOCKER）：资格条件类 REQ 矩阵行未响应；
//!   3. `E_SECTION_EMPTY`（BLOCKER）：叶章节无正文 / 未达 DRAFTED；
//!   4. `E_CITATION_GHOST`（BLOCKER）：章节引用了不存在的 REQ/EV；
//!   5. `E_EVIDENCE_EXPIRED`（BLOCKER）：绑定证据已失效（EXPIRED/
//!      SUPERSEDED——bind 时拒了，但 `apply_validity` 可后置翻转，真实风险）；
//!   6. `W_PLACEHOLDER`（MAJOR）：占位符残留（borrow check.rs 的
//!      PLACEHOLDER_MARKERS 口径——TODO/待补充/【占位】）；
//!   7. `W_CHARS_OFF_TARGET`（MINOR）：字数**超出**配额 +20% 之外（与 draft
//!      生成带对称——同口径可对账）；
//!   7b. `E_CHARS_BELOW_MIN`（BLOCKER）：正文低于配额 −20% 容差下限
//!      （1.0 写作验收：最低篇幅不达标 → 整份标书不得报告达标/导出——
//!      下限与 draft `pad_to_target` 的 80% 同口径，见规则 4 内注）；
//!   8. `W_FORMAT_BASELINE`（MINOR）：格式条款零命中按工程基线（Lead
//!      「不准硬编码」的降级告知面）。
//! - **第二层（模型复核，可选）**——[`ModelReviewer`] trait 注入（表述质量/
//!   逻辑连贯/偏离合理化/疑似过度承诺）；默认 [`NoModel`] 占位
//!   （counts.model = 0——无 API key 的 e2e 面，与 draft 的 MockGenerator
//!   同族设计）。
//!
//! **#187 分类返回（产品契约准确性）**：`ok: true` 只意味着「已执行的检查中
//! BLOCKER == 0」，不等于「整本标书已通过审核」。报告按四桶分类返回
//! （structural / semantic / evidence / compliance，各自独立的
//! [`CheckResult`]）——**「没做」必须显式报 [`CheckResult::NotPerformed`]**，
//! 不能因「没发现错误」伪装成 `Passed`（#9 口径：没做 ≠ 做对了）。当前
//! NoModel 未配置模型 → semantic 恒 `NotPerformed { reason: "no model
//! configured" }`。`NotPerformed` **不把 `ok` 翻 false**（它不是缺陷，是能力
//! 缺席——否则无模型的 e2e/现网全部误报不通过）。
//!
//! **纯只读诊断**：validate 不改稿不落库——BLOCKER 的解除通道是既有面
//! （`set_matrix_override` 响应覆写 / 证据补绑 / 章节 draft），export 的
//! 双前置门消费同一真源。报告是导数不是源数据。
//!
//! **导出时点复算面（1.0 写作验收）**：[`writing_acceptance_findings`] 把
//! 写作验收 BLOCKER（规则 3 空章节 + 规则 4 下界）单独成面，供
//! `export_document` 前置门 1c 消费——篇幅不足时**即使审核已直接批准**
//! （零 findings 零 RT）也不得导出：记审记录是流程事实不是质量事实，
//! 导出门不消费记审面、独立复算（fail-closed）。
//!
//! scope：`"all"` | 叶 section_id 清单（局部复审面——过滤只作用于**章节级**
//! 规则 3/4/6/7；全局级规则 1/2/5/8 恒全量——资格与否决没有「局部」语义）。

use std::collections::HashSet;

use bid_domain::evidence::EvidenceStatus;
use bid_domain::matrix::ResponseStatus;
use bid_domain::outline::SectionStatus;
use bid_domain::{DomainStore, ToolError};

/// 校验发现（契约 §7 findings 元素——rule_code 是稳定契约字面量，调用方
/// 按码分支不解析 message）。
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationFinding {
    pub rule_code: String,
    /// BLOCKER / MAJOR / MINOR（errors vs warnings 的分界：BLOCKER+MAJOR =
    /// errors 面，MINOR = warnings 面——Lead 输出形状的映射）。
    pub severity: &'static str,
    pub section_id: Option<String>,
    pub requirement_id: Option<String>,
    pub message: String,
}

/// 单类校验结果（#187 分类返回——四桶各自独立的状态面）。
///
/// 三态语义（#9 口径：没做 ≠ 做对了）：
/// - [`CheckResult::Passed`]：该类检查**已执行**且零发现；
/// - [`CheckResult::Failed`]：该类检查**已执行**且有发现项（含 MINOR——
///   「failed」指该类有发现，废标级与否仍看 ok/counts.blocker 全局面）；
/// - [`CheckResult::NotPerformed`]：该类检查**未执行**（如未配置模型）——
///   必须显式返回并带 reason，不得伪装成 `Passed`。
#[derive(Debug, Clone, PartialEq)]
pub enum CheckResult {
    Passed,
    Failed(Vec<ValidationFinding>),
    NotPerformed { reason: String },
}

/// 校验报告（契约 §7 out 素材）。
///
/// #187 分类返回：`structural` / `semantic` / `evidence` / `compliance` 四桶
/// 各自独立状态；`findings` = 四桶串接（桶序：structural → semantic →
/// evidence → compliance，桶内保持规则触发序——确定性稳定）。旧字段
/// （ok/findings/counts/sections_checked）原样保留（纯加法，消费方零破坏）。
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationReport {
    /// 已执行桶中 BLOCKER == 0（导出闸门的本地预演——真闸在 export 双前置门）。
    /// **NotPerformed 不把 ok 翻 false**（能力缺席不是缺陷，见模块文档 #187）。
    pub ok: bool,
    /// 四桶发现的串接全量清单（调用方按 rule_code 分支）。
    pub findings: Vec<ValidationFinding>,
    pub counts: ValidationCounts,
    /// scope 命中的章节数（空树/空 scope 的诊断面）
    pub sections_checked: usize,
    /// 结构面：E_SECTION_EMPTY / E_CITATION_GHOST / W_PLACEHOLDER /
    /// W_CHARS_OFF_TARGET / E_SCOPE_UNKNOWN（已实现——确定性结构检查）。
    pub structural: CheckResult,
    /// 语义面：M_ 前缀模型发现（未实现——NoModel 时 NotPerformed）。
    pub semantic: CheckResult,
    /// 证据面：E_EVIDENCE_EXPIRED（部分实现——效期面，深度复核后续 PR）。
    pub evidence: CheckResult,
    /// 合规面：E_VETO_UNRESPONDED / E_QUAL_UNMET / W_FORMAT_BASELINE
    /// （部分实现——矩阵响应 + 格式基线，暗标禁字等后续阶段）。
    pub compliance: CheckResult,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ValidationCounts {
    pub blocker: usize,
    pub major: usize,
    pub minor: usize,
    pub deterministic: usize,
    pub model: usize,
}

// ---------------------------------------------------------------- 模型复核层（注入位）

/// 模型复核 trait（第二层——仅产 finding 不改稿）。
///
/// #187 返回值语义（三态，实现方必须全调用面统一——不得对某些章 Some、
/// 另一些章 None 的混合）：
/// - `None`：未配置模型 → semantic 桶 = `NotPerformed { reason: "no model
///   configured" }`；
/// - `Some(v)`：已复核（`v` 为发现清单，空 vec = 该章通过）。
///
/// 调用面：scope 内**全部**叶章节都会被询问（含无正文的空章——`content`
/// 传空串；结构性缺陷已由 E_SECTION_EMPTY 报，空章返回 `Some(vec![])` 即可）。
pub trait ModelReviewer {
    /// 对单章内容做模型复核（表述质量/逻辑连贯/偏离合理化/疑似过度承诺）。
    fn review_section(&mut self, section_id: &str, content: &str)
        -> Option<Vec<ValidationFinding>>;
}

/// 无模型占位（e2e 面——counts.model 恒 0；真实 LLM 面后续 PR 接）。
/// #187：返回 `None`（未配置）——semantic 桶显式 NotPerformed，不伪装 Passed。
pub struct NoModel;

impl ModelReviewer for NoModel {
    fn review_section(&mut self, _section_id: &str, _content: &str)
        -> Option<Vec<ValidationFinding>> {
        None
    }
}

// ---------------------------------------------------------------- 主入口

/// 全量/局部校验（确定性先行 + 模型复核）。
///
/// `scope_sections`：`None` = all（全部叶）；`Some(ids)` = 局部（过滤章节级
/// 规则——不存在的 id 计入 findings 的 `E_SCOPE_UNKNOWN` MINOR 告警，
/// 不静默吞）。
pub fn validate_document(
    store: &DomainStore,
    scope_sections: Option<&[String]>,
    model: &mut dyn ModelReviewer,
) -> Result<ValidationReport, ToolError> {
    // ---- #187 四桶分类收集（桶内保持规则触发序；findings = 桶串接）
    let mut structural_f: Vec<ValidationFinding> = Vec::new();
    let mut semantic_f: Vec<ValidationFinding> = Vec::new();
    let mut evidence_f: Vec<ValidationFinding> = Vec::new();
    let mut compliance_f: Vec<ValidationFinding> = Vec::new();
    // 语义面是否未配置（任一 review_section 返回 None 即判定——实现方契约：
    // 全调用面统一，见 ModelReviewer 文档）
    let mut semantic_not_configured = false;

    // ---- 叶集合与 scope 解析
    let parents: HashSet<&str> = store.sections().iter()
        .filter_map(|n| n.parent_id.as_deref()).collect();
    let leaves: Vec<&bid_domain::outline::OutlineNode> = store.sections().iter()
        .filter(|n| !parents.contains(&n.id[..])).collect();
    let (scoped, unknown_scope): (Vec<_>, Vec<String>) = match scope_sections {
        None => (leaves.clone(), vec![]),
        Some(ids) => {
            let mut hit = Vec::new();
            let mut miss = Vec::new();
            for id in ids {
                match leaves.iter().find(|n| n.id == *id) {
                    Some(n) => hit.push(*n),
                    None => miss.push(id.clone()),
                }
            }
            (hit, miss)
        }
    };
    for id in &unknown_scope {
        structural_f.push(ValidationFinding {
            rule_code: "E_SCOPE_UNKNOWN".into(),
            severity: "MINOR",
            section_id: Some(id.clone()),
            requirement_id: None,
            message: format!("scope 指定的 {id} 不是叶章节或不存在——已从校验面剔除"),
        });
    }

    // ---- 规则 1/2：否决项/资格条件矩阵行（全局级——恒全量）
    for row in store.matrix() {
        let req = store.requirements().iter().find(|r| r.id == row.requirement_id);
        let Some(req) = req else { continue };
        let unresponded = matches!(
            row.status,
            ResponseStatus::NotResponded | ResponseStatus::Partial
        );
        if !unresponded {
            continue;
        }
        let (code, cat) = match req.category.as_str() {
            "否决项" => ("E_VETO_UNRESPONDED", "否决项"),
            "资格条件" => ("E_QUAL_UNMET", "资格条件"),
            _ => continue, // 其他类别的未响应是导出门（blocking）面，不是 validate 的 errors 面
        };
        compliance_f.push(ValidationFinding {
            rule_code: code.into(),
            severity: "BLOCKER",
            section_id: None,
            requirement_id: Some(row.requirement_id.clone()),
            message: format!(
                "{cat}「{}」未响应（矩阵状态 {}）——未响应的否决项是废标级风险",
                excerpt_of(req), row.status.as_str(),
            ),
        });
    }

    // ---- 规则 3/4/6/7：章节级（scope 过滤面）
    for node in &scoped {
        // 规则 3：无正文 / 未 DRAFTED
        let has_content = node.content.as_deref().map(|c| !c.trim().is_empty()).unwrap_or(false);
        if !has_content {
            structural_f.push(section_empty_no_content(node));
            // #187：模型复核面覆盖 scope 内**全部**章节——空章也询问
            // reviewer（内容传空串）。否则唯一章节为空时零次调用，semantic
            // 桶无法区分「未配置」与「已复核零章」（NoModel 会伪装成
            // Passed——#9 口径反面）。结构性缺陷已由 E_SECTION_EMPTY 报，
            // 实现方对空内容可返回 Some(vec![])。
            match model.review_section(&node.id, "") {
                None => semantic_not_configured = true,
                Some(v) => semantic_f.extend(v),
            }
            continue; // 无正文的章节不再跑字数/占位符/引用规则（无米之炊）
        }
        if !is_drafted(node.status) {
            structural_f.push(section_empty_not_drafted(node));
        }
        let content = node.content.as_deref().unwrap_or("");

        // 规则 4：篇幅验收（与 draft 生成带对称的验收口径——带 = target ×
        //      [80%, 120%]，Lead 验收线 ±20%）。
        //
        // 【1.0 写作验收】下界与上界分治：
        // - **下界 BLOCKER**（`E_CHARS_BELOW_MIN`）：篇幅不足 = 偷工减料——
        //   响应覆盖度不足以支撑评审，不得把整份标书报告为达标（ok 翻
        //   false）；经 record_review 派生 RT 后导出门（export_allowed
        //   的「全 RT DONE」面）拒导出——防废标的最后一关。
        // - **上界 MINOR**（`W_CHARS_OFF_TARGET` 原样）：超出只是提示（页数
        //   超限风险由人裁），不阻断——与旧口径兼容，避免过度收紧。
        // - `target_chars == 0`（无配额的叶）不判：没有目标就没有带外；
        //   空正文已由规则 3 E_SECTION_EMPTY 报 BLOCKER（continue 跳过），
        //   不双报。
        if node.target_chars > 0 {
            let chars = content.chars().count();
            let target = f64::from(node.target_chars);
            if (chars as f64) < target * 0.80 {
                structural_f.push(chars_below_min(node, chars));
            } else if (chars as f64) > target * 1.20 {
                structural_f.push(ValidationFinding {
                    rule_code: "W_CHARS_OFF_TARGET".into(),
                    severity: "MINOR",
                    section_id: Some(node.id.clone()),
                    requirement_id: None,
                    message: format!(
                        "章节「{}」字数 {chars} 超出配额 {}（+20% 带外：上限 {:.0}）",
                        node.title, node.target_chars, target * 1.20,
                    ),
                });
            }
        }

        // 规则 6：占位符残留（borrow check.rs 词表口径）
        for marker in crate::check::PLACEHOLDER_MARKERS {
            if content.contains(marker) {
                structural_f.push(ValidationFinding {
                    rule_code: "W_PLACEHOLDER".into(),
                    severity: "MAJOR",
                    section_id: Some(node.id.clone()),
                    requirement_id: None,
                    message: format!(
                        "章节「{}」含占位符残留 {marker:?}——模板未填完整",
                        node.title,
                    ),
                });
            }
        }

        // 规则 7：引用回查（章节 citations——ChapterRecord 的 citations 面
        // 与 OutlineNode.requirement_ids 双源；此处用 REQ 绑定面 + 章内引用）
        let known_req: HashSet<&str> = store.requirements().iter().map(|r| &r.id[..]).collect();
        let known_ev: HashSet<&str> = store.evidences().iter().map(|e| &e.id[..]).collect();
        for id in crate::draft::scan_citations(content) {
            let known = known_req.contains(&id[..]) || known_ev.contains(&id[..]);
            if !known {
                structural_f.push(ValidationFinding {
                    rule_code: "E_CITATION_GHOST".into(),
                    severity: "BLOCKER",
                    section_id: Some(node.id.clone()),
                    requirement_id: None,
                    message: format!(
                        "章节「{}」引用了不存在的 {id}（幻觉引用——废标级：评标查证无此件）",
                        node.title,
                    ),
                });
            }
        }

        // ---- 第二层：模型复核（注入位——仅产 finding 不改稿；rule_code 的
        //      M_ 前缀由实现方遵守，counts.model 据此分账）。#187 三态：
        //      None = 未配置 → semantic 桶 NotPerformed；Some(v) = 已复核
        //      （空 = 通过）。
        match model.review_section(&node.id, content) {
            None => semantic_not_configured = true,
            Some(v) => semantic_f.extend(v),
        }
    }

    // ---- 规则 8：绑定证据失效（全局级——apply_validity 后置翻转的真实风险）
    for b in store.evidence_bindings() {
        if let Some(ev) = store.evidences().iter().find(|e| e.id == b.evidence_id) {
            if matches!(ev.status, EvidenceStatus::Expired | EvidenceStatus::Superseded) {
                evidence_f.push(ValidationFinding {
                    rule_code: "E_EVIDENCE_EXPIRED".into(),
                    severity: "BLOCKER",
                    section_id: None,
                    requirement_id: Some(b.requirement_id.clone()),
                    message: format!(
                        "证据 {}（{}）状态 {} 仍绑定在 {} 上——失效证据禁绑（R-03），先换版或解绑",
                        ev.id, ev.title, ev.status.as_str(), b.requirement_id,
                    ),
                });
            }
        }
    }

    // ---- 规则 5：格式基线告知（Lead「不准硬编码」的降级面——只读复算；
    //      #86 摩擦 #13：区分三种形态——①零候选（真没格式条款）②有候选零
    //      命中（条款在但参数没提出来）③部分命中（提取到参数，不告警——
    //      旧实现按「含基线」字样匹配会把 ③ 也误报成「未识别到格式条款」，
    //      文案与事实打架）
    let (spec, fw) = crate::format_spec::derive_from_requirements(store.requirements());
    let zero_candidates = fw.iter().any(|w| w.contains("未发现格式条款"));
    let candidates_no_params = fw.iter().any(|w| w.contains("可识别"));
    if zero_candidates || candidates_no_params {
        let lead_msg = if zero_candidates {
            "招标文件未发现格式条款（格式要求类 0 条、格式字眼兜底 0 条）"
        } else {
            "招标文件存在格式候选条款但未提取到字号/字体/行距/边距参数"
        };
        compliance_f.push(ValidationFinding {
            rule_code: "W_FORMAT_BASELINE".into(),
            severity: "MINOR",
            section_id: None,
            requirement_id: None,
            message: format!(
                "{lead_msg}——导出将按工程基线（{}/{:.0}pt/{} 倍行距）执行，请人工确认形式评审要求",
                spec.global.font.body, spec.global.font.size, spec.global.font.line_height,
            ),
        });
    }

    // ---- #187 四桶归位（Passed / Failed；semantic 未配置 → NotPerformed）
    let into_result =
        |v: Vec<ValidationFinding>| if v.is_empty() { CheckResult::Passed } else { CheckResult::Failed(v) };
    let structural = into_result(structural_f);
    let semantic = if semantic_not_configured {
        CheckResult::NotPerformed { reason: "no model configured".into() }
    } else if scoped.is_empty() {
        // scope 内零章节（如 scope=sections 全部 id 未知——E_SCOPE_UNKNOWN 已
        // 告警）：零章被复核，不声称「已执行且通过」
        CheckResult::NotPerformed { reason: "no sections in scope".into() }
    } else {
        into_result(semantic_f)
    };
    let evidence = into_result(evidence_f);
    let compliance = into_result(compliance_f);

    // ---- 汇总（确定性/模型分账：rule_code 以 M_ 开头 = 模型面——注入实现约定）
    // findings = 四桶串接（桶序：structural → semantic → evidence → compliance）
    let mut findings: Vec<ValidationFinding> = Vec::new();
    for cr in [&structural, &semantic, &evidence, &compliance] {
        if let CheckResult::Failed(fs) = cr {
            findings.extend(fs.iter().cloned());
        }
    }
    let mut counts = ValidationCounts::default();
    for f in &findings {
        match f.severity {
            "BLOCKER" => counts.blocker += 1,
            "MAJOR" => counts.major += 1,
            _ => counts.minor += 1,
        }
        if f.rule_code.starts_with("M_") {
            counts.model += 1;
        } else {
            counts.deterministic += 1;
        }
    }
    // NotPerformed 不把 ok 翻 false：它贡献零 finding，counts.blocker 只数
    // 已执行桶的 BLOCKER（#187 红线——否则无模型面全体误报不通过）。
    let ok = counts.blocker == 0;
    Ok(ValidationReport {
        ok,
        findings,
        counts,
        sections_checked: scoped.len(),
        structural,
        semantic,
        evidence,
        compliance,
    })
}

// is_drafted 的单一真源在域层（#243：`SectionStatus::is_drafted`——本模块
// 规则 3 与域层 incomplete_leaf_ids 同源消费，防两面口径漂移）。
fn is_drafted(s: SectionStatus) -> bool {
    s.is_drafted()
}

// ---------------------------------------------------------------- 1.0 写作验收 · 导出门复算面

/// 规则 3 无正文变体的 finding 构造（validate 与导出复算面共用——单一
/// 真源，防两面文案漂移）。
fn section_empty_no_content(node: &bid_domain::outline::OutlineNode) -> ValidationFinding {
    ValidationFinding {
        rule_code: "E_SECTION_EMPTY".into(),
        severity: "BLOCKER",
        section_id: Some(node.id.clone()),
        requirement_id: None,
        message: format!(
            "章节「{}」（{}）无正文（状态 {}）——半个文档不能过审",
            node.title, node.id, node.status.as_str(),
        ),
    }
}

/// 规则 3 未定稿变体的 finding 构造（共用，同上）。
fn section_empty_not_drafted(node: &bid_domain::outline::OutlineNode) -> ValidationFinding {
    ValidationFinding {
        rule_code: "E_SECTION_EMPTY".into(),
        severity: "BLOCKER",
        section_id: Some(node.id.clone()),
        requirement_id: None,
        message: format!(
            "章节「{}」（{}）状态 {} 未达 DRAFTED——内容未定稿",
            node.title, node.id, node.status.as_str(),
        ),
    }
}

/// 规则 4 下界 BLOCKER 的 finding 构造（共用，同上）。
fn chars_below_min(
    node: &bid_domain::outline::OutlineNode,
    chars: usize,
) -> ValidationFinding {
    let target = f64::from(node.target_chars);
    ValidationFinding {
        rule_code: "E_CHARS_BELOW_MIN".into(),
        severity: "BLOCKER",
        section_id: Some(node.id.clone()),
        requirement_id: None,
        message: format!(
            "章节「{}」正文 {} 字低于最低篇幅 {:.0}（目标 {} 的 −20% 容差下限）——篇幅不足不得达标/导出，补写后重校",
            node.title, chars, target * 0.80, node.target_chars,
        ),
    }
}

/// 1.0 写作验收的导出时点复算（`export_document` 前置门 1c 消费）。
///
/// **门禁语义（Lead 问询补明 2026-11）**：篇幅不足时，即使有人直接批准
/// 审核（PreExportConfirm Approved、零 findings、零 RT），仍然禁止导出。
/// 理由：记审记录是**流程事实**不是**质量事实**——审核员漏看/绕过
/// validate 的发现不构成篇幅豁免；validate 本身是纯只读诊断（报告是
/// 导数不是源数据，过审与导出之间内容可再被改短），所以导出门必须
/// **独立复算**写作验收硬线，不消费记审面（reviews / RT / gate_approved
/// 均不看）——fail-closed。
///
/// 复算范围 = 写作验收 BLOCKER 两条（与 validate 同规则同构造器，无
/// scope 恒全量叶）：
/// - 规则 3 `E_SECTION_EMPTY`：必写叶无正文 / 未达 DRAFTED；
/// - 规则 4 下界 `E_CHARS_BELOW_MIN`：正文 < target×80%。
///
/// **不在**此面的规则及其理由：上界 `W_CHARS_OFF_TARGET` 是 MINOR 提示
/// 不阻断导出；矩阵/否决/资格面有关 1a 矩阵门；证据效期/引用幽灵走
/// 审核记审面（findings→RT）——写作验收门只管篇幅与空章，不越界。
///
/// 返回空 vec = 过门；非空 = BLOCKER 清单（导出面逐条点名拒）。
pub fn writing_acceptance_findings(store: &DomainStore) -> Vec<ValidationFinding> {
    let parents: HashSet<&str> = store
        .sections()
        .iter()
        .filter_map(|n| n.parent_id.as_deref())
        .collect();
    let mut out = Vec::new();
    for node in store.sections().iter().filter(|n| !parents.contains(&n.id[..])) {
        let has_content = node
            .content
            .as_deref()
            .map(|c| !c.trim().is_empty())
            .unwrap_or(false);
        if !has_content {
            out.push(section_empty_no_content(node));
            continue; // 无正文的叶不再跑字数规则（与 validate 同口径，不双报）
        }
        if !is_drafted(node.status) {
            out.push(section_empty_not_drafted(node));
        }
        if node.target_chars > 0 {
            let chars = node.content.as_deref().unwrap_or("").chars().count();
            if (chars as f64) < f64::from(node.target_chars) * 0.80 {
                out.push(chars_below_min(node, chars));
            }
        }
    }
    out
}

fn excerpt_of(req: &bid_domain::Requirement) -> String {
    let ex = req.anchors.first().map(|a| a.excerpt.as_str()).unwrap_or("");
    let cut: String = ex.chars().take(24).collect();
    if cut.is_empty() { req.id.clone() } else { cut }
}
