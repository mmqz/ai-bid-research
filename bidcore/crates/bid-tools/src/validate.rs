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
//!   5b. `E_EVIDENCE_PLACEHOLDER`（BLOCKER，#245）：占位/待核验证据仍绑定
//!      在条款上——占位材料不得冒充已核验证据（规则 1/2 的 PENDING_EVIDENCE
//!      态覆盖矩阵面，本规则覆盖绑定面：非必选行的占位绑定也在此报）；
//!   5c. `E_CITATION_PLACEHOLDER`（BLOCKER，#245）：章节正文引用了占位/
//!      待核验证据——占位不得作为资质证明引用（正文层面的冒充面）；
//!   6. `W_PLACEHOLDER`（MAJOR）：占位符残留（borrow check.rs 的
//!      PLACEHOLDER_MARKERS 口径——TODO/待补充/【占位】；与【待补：...】
//!      授权占位形态不同源——后者是 #245 起草面的合规形态，不误报）；
//!   7. `W_CHARS_OFF_TARGET`（MINOR）：字数**超出**配额 +20% 之外（与 draft
//!      生成带对称——同口径可对账）；
//!   7b. `E_CHARS_BELOW_MIN`（BLOCKER，**仅评分章节**）：正文低于配额 −20%
//!      容差下限（#253 门禁分级：评分主体写不够 = 直接丢分；下限与 draft
//!      `pad_to_target` 的 80% 同口径）；行政/实质章节 → `W_CHARS_BELOW_MIN`
//!      （MAJOR——issue #253 的 WARNING 语义：不翻 ok、不拦导出）；
//!   7c. `E_TOTAL_CHARS_BELOW_MIN`（BLOCKER，文档级）：全稿实写总量 <
//!      `outline_total_chars()`（选中档位硬指标——产品级硬比较无容差；
//!      #287 起随篇幅档位动态：默认 Conservative 1,200,000 ≈ 1422 页）；
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

    // ---- 规则 1/2：否决项/资格条件矩阵行（全局级——恒全量）。
    //      #245：PENDING_EVIDENCE（仅占位证据在绑）计入未响应面——占位
    //      资质不满足资格/否决响应（消息携带待补语义，人工可辨）。
    for row in store.matrix() {
        let req = store.requirements().iter().find(|r| r.id == row.requirement_id);
        let Some(req) = req else { continue };
        let unresponded = matches!(
            row.status,
            ResponseStatus::NotResponded | ResponseStatus::Partial | ResponseStatus::PendingEvidence
        );
        if !unresponded {
            continue;
        }
        let (code, cat) = match req.category.as_str() {
            "否决项" => ("E_VETO_UNRESPONDED", "否决项"),
            "资格条件" => ("E_QUAL_UNMET", "资格条件"),
            _ => continue, // 其他类别的未响应是导出门（blocking）面，不是 validate 的 errors 面
        };
        let pending_note = if row.status == ResponseStatus::PendingEvidence {
            "（占位/待核验证据不满足响应——待补真实材料）"
        } else {
            ""
        };
        compliance_f.push(ValidationFinding {
            rule_code: code.into(),
            severity: "BLOCKER",
            section_id: None,
            requirement_id: Some(row.requirement_id.clone()),
            message: format!(
                "{cat}「{}」未响应（矩阵状态 {}）{pending_note}——未响应的否决项是废标级风险",
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
        // - **下界分轨**（#253 第二段门禁分级）：**评分章节** BLOCKER
        //   （`E_CHARS_BELOW_MIN`——评分主体写不够 = 直接丢分，不得把整份
        //   标书报告为达标）；**行政/实质章节** `W_CHARS_BELOW_MIN` MAJOR
        //   （issue #253 的 WARNING 语义——不翻 ok、不拦导出，内容完整度
        //   由规则 3 / 矩阵 / 绑定面把关）；
        // - **上界 MINOR**（`W_CHARS_OFF_TARGET` 原样）：超出只是提示（页数
        //   超限风险由人裁），不阻断——与旧口径兼容，避免过度收紧。
        // - `target_chars == 0`（无配额的叶）不判：没有目标就没有带外；
        //   空正文已由规则 3 E_SECTION_EMPTY 报 BLOCKER（continue 跳过），
        //   不双报。
        if node.target_chars > 0 {
            let chars = content.chars().count();
            let target = f64::from(node.target_chars);
            if (chars as f64) < target * 0.80 {
                if is_scoring_leaf(node, store) {
                    structural_f.push(chars_below_min(node, chars));
                } else {
                    structural_f.push(chars_below_min_advisory(node, chars));
                }
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
        // #245：占位/待核验证据 id 集（活跃面——Superseded/Expired 的占位件
        // 由规则 8 的失效面报，不双报）
        let placeholder_ev: HashSet<&str> = store
            .evidences()
            .iter()
            .filter(|e| {
                e.is_placeholder()
                    && matches!(e.status, EvidenceStatus::Active | EvidenceStatus::Expiring)
            })
            .map(|e| &e.id[..])
            .collect();
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
            } else if placeholder_ev.contains(&id[..]) {
                // 规则 5c（#245）：占位证据被正文引用 = 冒充已核验——即使
                // id 存在（非 ghost），占位材料也不得作为资质证明援引。
                evidence_f.push(ValidationFinding {
                    rule_code: "E_CITATION_PLACEHOLDER".into(),
                    severity: "BLOCKER",
                    section_id: Some(node.id.clone()),
                    requirement_id: None,
                    message: format!(
                        "章节「{}」引用了占位/待核验证据 {id}——占位材料不得作为资质证明援引（#245：不得声称已具备）；改为【待补：...】占位说明，真实材料登记换版后再引用",
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

    // ---- 规则 5b（#245）：占位/待核验证据仍绑定——占位不得冒充已核验证据。
    //      与规则 1/2 的分工：规则 1/2 只覆盖资格/否决类的矩阵态（全局面）；
    //      本规则覆盖**绑定面**（任何类别的 REQ）——非必选行的占位绑定
    //      （矩阵 PENDING_EVIDENCE 不阻断写作）也在此报 BLOCKER：导出前
    //      必须解决（补真实材料换版，或合法人工豁免——只认 NOT_APPLICABLE
    //      + 偏离说明 + 可信 Human receipt，见 is_legitimate_placeholder_
    //      exemption：普通状态覆写不豁免）。
    for f in placeholder_evidence_findings(store) {
        evidence_f.push(f);
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

    // ---- 规则 4 文档级（#253 验收 7）：总篇幅门（全局级——恒全量，
    //      scope 局部复审不缩小它的视野：全稿总量是产品级硬指标）
    if let Some(f) = total_chars_below_min(store) {
        structural_f.push(f);
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

/// 规则 4 下界 BLOCKER 的 finding 构造（共用，同上）——**仅评分章节**
/// （#253 第二段门禁分级：评分主体写不够 = 直接丢分，BLOCKER 不得达标/导出）。
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
            "评分章节「{}」正文 {} 字低于最低篇幅 {:.0}（目标 {} 的 −20% 容差下限）——评分主体篇幅不足不得达标/导出，补写后重校",
            node.title, chars, target * 0.80, node.target_chars,
        ),
    }
}

/// 规则 4 下界的行政/实质面（#253 第二段门禁分级）。
///
/// issue #253 的「WARNING」在本词表的映射 = `W_CHARS_BELOW_MIN` **MAJOR**：
/// 进 errors 面可见，但不翻 `ok`（仅 BLOCKER 翻）、不拦导出——行政/实质
/// 章节的内容完整度由 E_SECTION_EMPTY / 矩阵 / 章节绑定面把关，篇幅只作
/// 质量提示（「内容完整即可，不强制篇幅」）。
fn chars_below_min_advisory(
    node: &bid_domain::outline::OutlineNode,
    chars: usize,
) -> ValidationFinding {
    let target = f64::from(node.target_chars);
    ValidationFinding {
        rule_code: "W_CHARS_BELOW_MIN".into(),
        severity: "MAJOR",
        section_id: Some(node.id.clone()),
        requirement_id: None,
        message: format!(
            "行政/实质章节「{}」正文 {} 字低于目标 {} 的 −20% 容差下限（#253：篇幅不强制，内容完整即可——补写可提升评审阅读体验）",
            node.title, chars, node.target_chars,
        ),
    }
}

/// 叶是否评分承载章节（#253 门禁分级的分类面——与配额三轨的
/// [`bid_domain::outline`] 评分轨同源判定：绑定评分项 REQ，或被
/// ScoringMapping 映射为被评章节）。
///
/// 双信号容错：#252 评分驱动大纲落地前 REQ 绑定是主信号；落地后若结构
/// 改用映射面（ScoringMapping.section_id），账本信号兜底——两信号任一
/// 命中即按评分章节的 BLOCKER 口径把关（fail-strict：分值主体不得漏管）。
fn is_scoring_leaf(
    node: &bid_domain::outline::OutlineNode,
    store: &DomainStore,
) -> bool {
    let scoring_req: bool = node.requirement_ids.iter().any(|rid| {
        store.requirements().iter().any(|r| r.id == *rid && r.category == "评分项")
    });
    let mapped: bool = store.scoring_mappings().iter().any(|m| m.section_id == node.id);
    scoring_req || mapped
}

/// 文档级总篇幅门（#253 验收 7：总篇幅 < 1000 页 → BLOCKER——用户硬指标）。
///
/// 阈值 = `outline_total_chars()`（配额总和，单一真源；#287 起随选中档位
/// 动态：默认档 Conservative 1,200,000 ≈ 1422 页，Practical 400,000 ≈
/// 474 页……——非固定 844,000）：**硬比较无容差**——章节级 ±20% 容差是起草
/// 容差（迭代中可短写），产品级门是硬指标（导出的标书必须达到目标总量；
/// 若容忍 −20%，全带下限起草的标书只有 800 页，违背用户硬要求）。
fn total_chars_below_min(store: &DomainStore) -> Option<ValidationFinding> {
    let target = store.outline_total_chars();
    if target == 0 {
        return None; // 无配额不判（与规则 4 的 target_chars == 0 同口径）
    }
    // 正文只住叶（set_section_content 拒结构节点）——非空 content 即叶实写面
    let actual: u64 = store
        .sections()
        .iter()
        .filter(|n| n.content.as_deref().map(|c| !c.trim().is_empty()).unwrap_or(false))
        .map(|n| n.content.as_deref().map(|c| c.chars().count() as u64).unwrap_or(0))
        .sum();
    if actual < target {
        Some(ValidationFinding {
            rule_code: "E_TOTAL_CHARS_BELOW_MIN".into(),
            severity: "BLOCKER",
            section_id: None,
            requirement_id: None,
            message: format!(
                "全稿正文字数 {actual} 低于总篇幅目标 {target}（选中档位硬指标，现状约 {} 页）——总篇幅不足不得达标/导出，逐章补写后重校",
                actual / u64::from(bid_domain::outline::CHARS_PER_PAGE),
            ),
        })
    } else {
        None
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
/// 复算范围 = 写作验收 BLOCKER 面（无 scope 恒全量叶；#253 第二段后）：
/// - 规则 3 `E_SECTION_EMPTY`：必写叶无正文 / 未达 DRAFTED；
/// - 规则 4 下界 `E_CHARS_BELOW_MIN`：**评分章节**正文 < target×80%
///   （行政/实质章节降为 W_CHARS_BELOW_MIN MAJOR——不拦导出，
///   #253 门禁分级：内容完整即可，不强制篇幅）；
/// - 规则 4 文档级 `E_TOTAL_CHARS_BELOW_MIN`：全稿实写总量 <
///   `outline_total_chars()`（1000 页硬指标——产品级硬比较无容差）。
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
            if (chars as f64) < f64::from(node.target_chars) * 0.80
                && is_scoring_leaf(node, store)
            {
                out.push(chars_below_min(node, chars));
            }
        }
    }
    if let Some(f) = total_chars_below_min(store) {
        out.push(f);
    }
    out
}

fn excerpt_of(req: &bid_domain::Requirement) -> String {
    let ex = req.anchors.first().map(|a| a.excerpt.as_str()).unwrap_or("");
    let cut: String = ex.chars().take(24).collect();
    if cut.is_empty() { req.id.clone() } else { cut }
}

/// #245 占位门人工豁免裁决（单一真源——validate 规则 5b 与导出门 1d 共用；
/// PR #247 审核收紧：**覆写在场 ≠ 豁免成立**）。
///
/// 只有**明确的合法人工豁免**才放行仍活跃的占位绑定，三要件缺一不可：
/// 1. **NOT_APPLICABLE**——人裁决「本行不适用」。其他覆写状态
///    （Responded/Partial/NotResponded/PendingEvidence）是「已响应」类
///    声明——正是占位材料不得冒充的结论；`set_matrix_override` 对它们
///    不强制偏离说明，普通人工状态覆写不得因此绕过占位门；
/// 2. **非空偏离说明**（trim 后非空）——不适用必须给理由（诚实状态；
///    域层写入面已强制，此处独立复算：手改/旧快照不带说明不放行）；
/// 3. **可信 Human receipt**——`MatrixOverride.trusted`（#182：服务层
///    HMAC 验签后注入的 TrustedPrincipal，写入面单一真源）。**只认
///    receipt 背书面，不认 `principal` 回显串**（"human:xxx" 是可伪造的
///    普通字段；legacy 快照 trusted 缺席 → 不豁免，fail-closed）。
pub fn is_legitimate_placeholder_exemption(ov: &bid_domain::matrix::MatrixOverride) -> bool {
    ov.status == bid_domain::matrix::ResponseStatus::NotApplicable
        && ov
            .deviation
            .as_deref()
            .map(|d| !d.trim().is_empty())
            .unwrap_or(false)
        && ov
            .trusted
            .as_ref()
            .map(|t| t.actor_type == bid_domain::ActorType::Human)
            .unwrap_or(false)
}

/// #245 占位证据独立复算（导出门 1d 的真源——validate 规则 5b 同款）。
///
/// 语义（与 1c 写作验收门对称的 fail-closed 门禁）：
/// - 活跃占位绑定（PLACEHOLDER 且 Active/Expiring）存在 → 占位材料不得
///   冒充已核验证据导出——**即使审核已直接批准**（三门全 Approved、零
///   findings 零 RT）也仍拒绝：记审面是流程事实不是核验事实，本门不消费
///   reviews/RT（PR #242 1c 门同款纪律）；
/// - **人工裁决通道**（与 1c 的差异面——占位缺失有人工豁免出口，篇幅不足
///   没有）：只认 [`is_legitimate_placeholder_exemption`]（NOT_APPLICABLE +
///   非空偏离说明 + 可信 Human receipt）——覆写在场 ≠ 豁免成立，
///   豁免留痕可回溯；
/// - 解决通道：①补真实材料重新登记（换版：占位记录 Superseded + 绑定迁移
///   → 本门自然清空）；②合法人工豁免（NOT_APPLICABLE + 偏离说明 +
///   Human receipt）。
///
/// 返回空 vec = 过门；非空 = BLOCKER 清单（导出面逐条点名拒）。
pub fn placeholder_evidence_findings(store: &DomainStore) -> Vec<ValidationFinding> {
    let mut out = Vec::new();
    for b in store.evidence_bindings() {
        // 人工豁免裁决（PR #247 审核收紧）：只有「NOT_APPLICABLE + 非空
        // 偏离说明 + 可信 Human receipt」的明确豁免才跳过占位门——
        // 覆写在场 ≠ 豁免成立：Responded 等状态覆写（set_matrix_override
        // 对其不强制偏离说明）即使由 Human 发起 receipt 签发，也不放行
        // 仍活跃的占位绑定（fail-closed）。
        if let Some(ov) = store.matrix_overrides().get(&b.requirement_id) {
            if is_legitimate_placeholder_exemption(ov) {
                continue;
            }
        }
        let Some(ev) = store.evidences().iter().find(|e| e.id == b.evidence_id) else {
            continue;
        };
        // 只看活跃占位件：Superseded（已换版升级——绑定已迁移，理论上不会
        // 走到）与 Expired（失效面由规则 8 报）不在本门。
        if ev.is_placeholder() && matches!(ev.status, EvidenceStatus::Active | EvidenceStatus::Expiring) {
            out.push(ValidationFinding {
                rule_code: "E_EVIDENCE_PLACEHOLDER".into(),
                severity: "BLOCKER",
                section_id: None,
                requirement_id: Some(b.requirement_id.clone()),
                message: format!(
                    "占位/待核验证据 {}（{}）仍绑定在 {} 上——占位材料不得作为已核验证据导出（#245）；解决：导入真实材料重新登记（换版升级），或合法人工豁免（NOT_APPLICABLE + 偏离说明 + Human receipt——普通状态覆写不豁免）",
                    ev.id, ev.title, b.requirement_id,
                ),
            });
        }
    }
    out
}

// ---------------------------------------------------------------- #241 裁定：提交就绪面

/// 提交就绪清单项（机器可读的逐条补件面——供 Agent/Human 逐项消除）。
#[derive(Debug, Clone, PartialEq)]
pub struct ReadinessItem {
    pub requirement_id: String,
    /// 矩阵行 id（CMP-xxxx——与 get_compliance_matrix 行对账）
    pub row_id: String,
    pub category: String,
    /// 原文摘录（excerpt_of 同款 24 字口径——人可辨条款指代）
    pub excerpt: String,
    /// 人读理由（含解决出口指引）
    pub message: String,
}

/// #241 裁定：提交就绪面（submission readiness）。
///
/// **Export ≠ Submission Readiness**（两分账的门禁分工）：
/// - **导出**（草稿可导出）：矩阵 blocking 门（必选未响应/待补）+ 审核门 +
///   写作验收门 + 占位证据门——证据缺口（正文已响应、材料未绑）**不拦**
///   （#241 裁定：导出：不阻断，草稿可导出，见 #253 分离面）；
/// - **提交就绪**（可正式提交）：上述门全过 **且** 证据缺口清零——
///   「没证明材料就是废标风险」，缺口行不得进入可提交状态。
///
/// 阻断两清单：
/// 1. `unresponded`：必选条款未响应（矩阵 blocking 面——否决项/资格条件/
///    Agent 必选的 NOT_RESPONDED/PENDING_EVIDENCE，外加 PARTIAL——
///    材料齐正文未落也不可提交，与 validate 规则 1/2 同口径，#253 验收 8
///    「否决项未响应仍阻断 submission readiness」的正门）；
/// 2. `evidence_missing`：必选条款证据缺口（`evidence_missing=true` 且无
///    合法人工豁免——#241 裁定的合规事实面）。
///
/// 必选口径 = 类别阻断面（资格条件/否决项）∪ Agent 必选标记——与
/// matrix.rs blocking 派生同一判定面。人工豁免通道 =
/// [`is_legitimate_placeholder_exemption`]（NOT_APPLICABLE + 偏离说明 +
/// 可信 Human receipt——单一真源，与占位门共用）；**普通 RESPONDED 覆写
/// 不豁免**（PR #247 纪律同源：覆写在场 ≠ 豁免成立——人工确认响应成立
/// 不等于材料已提交）。
///
/// 纯只读派生（D2 同语义）：消费 `matrix_view()` 新鲜视图（content 变化
/// 即时反映，不依赖调用方记得重建矩阵），不落库不 bump。
#[derive(Debug, Clone, PartialEq)]
pub struct SubmissionReadiness {
    pub ready: bool,
    /// 矩阵未构建（未对账 = 流程面未完成，不可判就绪——诚实状态；清单仍
    /// 从新鲜视图派生供参考）
    pub not_built: bool,
    /// 必选条款未响应清单（空 = 无）
    pub unresponded: Vec<ReadinessItem>,
    /// 必选条款证据缺口清单（空 = 无）
    pub evidence_missing: Vec<ReadinessItem>,
}

impl SubmissionReadiness {
    /// 阻断总数（两清单逐条计——同一条款可同时未响应且缺证据，两条各计
    /// 一次：未响应是响应问题，缺证据是材料问题，解决出口不同）。
    pub fn blockers(&self) -> usize {
        self.unresponded.len() + self.evidence_missing.len()
    }
}

/// 提交就绪裁决（#241 裁定——check_bid_quality / validate 响应面的素材）。
///
/// 返回值是导数不是源数据；解除通道是既有面（补绑证据 / 补写正文 /
/// set_matrix_override 人工豁免）。
pub fn submission_readiness(store: &DomainStore) -> SubmissionReadiness {
    let not_built = store.matrix().is_empty();
    // 新鲜派生视图（与 get_compliance_matrix 同源——12k 行量级 O(N+B+S)）
    let view = store.matrix_view();
    let req_by_id: std::collections::HashMap<&str, &bid_domain::Requirement> =
        store.requirements().iter().map(|r| (r.id.as_str(), r)).collect();
    let mut unresponded = Vec::new();
    let mut evidence_missing = Vec::new();
    for row in &view {
        let Some(req) = req_by_id.get(row.requirement_id.as_str()) else {
            continue; // 幽灵行（REQ 已删）——视图与条款集不一致的防御面
        };
        let mandatory = bid_domain::matrix::is_mandatory(&req.category)
            || bid_domain::matrix::agent_mandatory(req);
        if !mandatory {
            continue; // 非必选的缺口/未响应是信息面（矩阵行标记可见），不阻断提交就绪
        }
        // 清单 1：未响应（NOT_RESPONDED/PENDING_EVIDENCE/PARTIAL——与
        // validate 规则 1/2 的 unresponded 集合同口径）
        if matches!(
            row.status,
            bid_domain::matrix::ResponseStatus::NotResponded
                | bid_domain::matrix::ResponseStatus::Partial
                | bid_domain::matrix::ResponseStatus::PendingEvidence
        ) {
            let st = row.status.as_str();
            let hint = match row.status {
                bid_domain::matrix::ResponseStatus::PendingEvidence =>
                    "（占位/待核验材料不算有效证据——导入真实材料换版升级）",
                bid_domain::matrix::ResponseStatus::Partial =>
                    "（材料已绑但正文未落——补写响应章节）",
                _ => "（补绑证据 / 落正文响应 / 人工覆写）",
            };
            unresponded.push(ReadinessItem {
                requirement_id: row.requirement_id.clone(),
                row_id: row.id.clone(),
                category: req.category.clone(),
                excerpt: excerpt_of(req),
                message: format!(
                    "必选条款未响应（矩阵 {st}）{hint}——未响应的否决项/资格条件是废标级风险"
                ),
            });
        }
        // 清单 2：证据缺口（#241 裁定主面）。合法人工豁免（NOT_APPLICABLE +
        // 偏离说明 + 可信 Human receipt）清除阻断——人工裁决优先于机器判缺；
        // 普通 RESPONDED 覆写不豁免（材料要求未满足，覆写只确认响应事实）。
        if row.evidence_missing {
            let exempt = store
                .matrix_overrides()
                .get(&row.requirement_id)
                .map(is_legitimate_placeholder_exemption)
                .unwrap_or(false);
            if !exempt {
                evidence_missing.push(ReadinessItem {
                    requirement_id: row.requirement_id.clone(),
                    row_id: row.id.clone(),
                    category: req.category.clone(),
                    excerpt: excerpt_of(req),
                    message: format!(
                        "必选条款要求证明材料但无有效证据在绑（正文已响应不影响缺口——文本事实与合规事实分账）；解决：补绑证据（bind_evidence）或合法人工豁免（NOT_APPLICABLE + 偏离说明 + Human receipt）"
                    ),
                });
            }
        }
    }
    let ready = !not_built && unresponded.is_empty() && evidence_missing.is_empty();
    SubmissionReadiness { ready, not_built, unresponded, evidence_missing }
}
