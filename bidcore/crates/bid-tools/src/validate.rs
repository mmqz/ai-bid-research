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
//!   7. `W_CHARS_OFF_TARGET`（MINOR）：字数偏离配额 ±20% 之外（与 draft
//!      生成带对称——同口径可对账）；
//!   8. `W_FORMAT_BASELINE`（MINOR）：格式条款零命中按工程基线（Lead
//!      「不准硬编码」的降级告知面）。
//! - **第二层（模型复核，可选）**——[`ModelReviewer`] trait 注入（表述质量/
//!   逻辑连贯/偏离合理化/疑似过度承诺）；默认 [`NoModel`] 占位
//!   （counts.model = 0——无 API key 的 e2e 面，与 draft 的 MockGenerator
//!   同族设计）。
//!
//! **纯只读诊断**：validate 不改稿不落库——BLOCKER 的解除通道是既有面
//! （`set_matrix_override` 响应覆写 / 证据补绑 / 章节 draft），export 的
//! 双前置门消费同一真源。报告是导数不是源数据。
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

/// 校验报告（契约 §7 out 素材）。
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationReport {
    /// BLOCKER == 0（导出闸门的本地预演——真闸在 export 双前置门）
    pub ok: bool,
    pub findings: Vec<ValidationFinding>,
    pub counts: ValidationCounts,
    /// scope 命中的章节数（空树/空 scope 的诊断面）
    pub sections_checked: usize,
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
pub trait ModelReviewer {
    /// 对单章内容做模型复核（表述质量/逻辑连贯/偏离合理化/疑似过度承诺）。
    fn review_section(&mut self, section_id: &str, content: &str)
        -> Vec<ValidationFinding>;
}

/// 无模型占位（e2e 面——counts.model 恒 0；真实 LLM 面后续 PR 接）。
pub struct NoModel;

impl ModelReviewer for NoModel {
    fn review_section(&mut self, _section_id: &str, _content: &str) -> Vec<ValidationFinding> {
        Vec::new()
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
    let mut findings: Vec<ValidationFinding> = Vec::new();

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
        findings.push(ValidationFinding {
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
        findings.push(ValidationFinding {
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
            findings.push(ValidationFinding {
                rule_code: "E_SECTION_EMPTY".into(),
                severity: "BLOCKER",
                section_id: Some(node.id.clone()),
                requirement_id: None,
                message: format!(
                    "章节「{}」（{}）无正文（状态 {}）——半个文档不能过审",
                    node.title, node.id, node.status.as_str(),
                ),
            });
            continue; // 无正文的章节不再跑字数/占位符/引用规则（无米之炊）
        }
        if !is_drafted(node.status) {
            findings.push(ValidationFinding {
                rule_code: "E_SECTION_EMPTY".into(),
                severity: "BLOCKER",
                section_id: Some(node.id.clone()),
                requirement_id: None,
                message: format!(
                    "章节「{}」（{}）状态 {} 未达 DRAFTED——内容未定稿",
                    node.title, node.id, node.status.as_str(),
                ),
            });
        }
        let content = node.content.as_deref().unwrap_or("");

        // 规则 4：字数偏离 ±20%（与 draft 生成带对称的验收口径）
        if node.target_chars > 0 {
            let chars = content.chars().count();
            let target = f64::from(node.target_chars);
            if (chars as f64) < target * 0.80 || (chars as f64) > target * 1.20 {
                findings.push(ValidationFinding {
                    rule_code: "W_CHARS_OFF_TARGET".into(),
                    severity: "MINOR",
                    section_id: Some(node.id.clone()),
                    requirement_id: None,
                    message: format!(
                        "章节「{}」字数 {chars} 偏离配额 {}（±20% 带外：[{:.0}, {:.0}]）",
                        node.title, node.target_chars, target * 0.80, target * 1.20,
                    ),
                });
            }
        }

        // 规则 6：占位符残留（borrow check.rs 词表口径）
        for marker in crate::check::PLACEHOLDER_MARKERS {
            if content.contains(marker) {
                findings.push(ValidationFinding {
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
                findings.push(ValidationFinding {
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
        //      M_ 前缀由实现方遵守，counts.model 据此分账）
        findings.append(&mut model.review_section(&node.id, content));
    }

    // ---- 规则 8：绑定证据失效（全局级——apply_validity 后置翻转的真实风险）
    for b in store.evidence_bindings() {
        if let Some(ev) = store.evidences().iter().find(|e| e.id == b.evidence_id) {
            if matches!(ev.status, EvidenceStatus::Expired | EvidenceStatus::Superseded) {
                findings.push(ValidationFinding {
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
        findings.push(ValidationFinding {
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

    // ---- 汇总（确定性/模型分账：rule_code 以 M_ 开头 = 模型面——注入实现约定）
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
    let ok = counts.blocker == 0;
    Ok(ValidationReport { ok, findings, counts, sections_checked: scoped.len() })
}

fn is_drafted(s: SectionStatus) -> bool {
    matches!(
        s,
        SectionStatus::Drafted
            | SectionStatus::Validating
            | SectionStatus::Revising
            | SectionStatus::Accepted
    )
}

fn excerpt_of(req: &bid_domain::Requirement) -> String {
    let ex = req.anchors.first().map(|a| a.excerpt.as_str()).unwrap_or("");
    let cut: String = ex.chars().take(24).collect();
    if cut.is_empty() { req.id.clone() } else { cut }
}
