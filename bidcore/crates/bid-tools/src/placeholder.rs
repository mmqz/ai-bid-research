//! placeholder.rs —— 缺失回报的「显式占位」面（issue #103 子任务 C）。
//!
//! ## 为什么需要占位而不是「硬写」
//!
//! Lead 原话：「缺失回报，如果缺失这个内容却还硬做只会做错」。当前的
//! draft 通路在没有素材时照样生成——没有业绩证据 → LLM 编造业绩 → 废标；
//! 缺资质证书 → 虚假响应 → 资格性审查否决。占位是**诚实的缺省**：正文里
//! 留下「【待补充：请提供 3 个类似项目业绩】」，谁看都知道这里缺什么、
//! 要补什么，而不是一段读起来通顺但不可核验的编造文字。
//!
//! 纪律：
//! - 占位是**机器可扫的**（[`scan_placeholders`] 反向解析），校验面因此能
//!   数出「这份稿还剩几处待补充」，导出闸门才有依据；
//! - 占位是**人可读的**（[`Placeholder::describe`]），不泄漏枚举名；
//! - 本模块**零 store 依赖**（纯字符串面）——判定逻辑在 readiness.rs。
//!
//! ## 语法
//!
//! ```text
//! 【待补充：<what>】            ← 缺外部素材（要材料）
//! 【待确认：<what>】            ← 需人工裁决（要人）
//! ```
//!
//! 「待补充」= 我们知道缺什么、缺的是外部素材；「待确认」= 我们不知道
//! 该怎么填、需要人裁决。两者都不能留到导出（导出前清零或显式豁免），
//! 但语义不同：前者去要材料，后者去问人——混在一起会让「去要材料」的
//! 清单里混进「去问人」的事项，所以按 [`PlaceholderKind`] 分开。

use serde::{Deserialize, Serialize};

/// 占位左括号（全角——中文正文里的显式标记，不会与正常行文撞车）。
pub const OPEN: char = '【';
/// 占位右括号。
pub const CLOSE: char = '】';
/// 分隔符（全角冒号：`【待补充：…】`）。
pub const SEP: char = '：';

/// 占位种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceholderKind {
    /// 缺外部素材（要材料——业绩/资质/证书/参数证据）。
    Missing,
    /// 需人工裁决（要人——口径不明、责任不明、参数待确认）。
    Confirm,
}

impl PlaceholderKind {
    /// 标签词（占位里冒号前的那一段）。
    pub fn label(&self) -> &'static str {
        match self {
            PlaceholderKind::Missing => "待补充",
            PlaceholderKind::Confirm => "待确认",
        }
    }

    /// 反查（扫描面：标签词 → 种类；未知标签返回 None）。
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "待补充" => Some(PlaceholderKind::Missing),
            "待确认" => Some(PlaceholderKind::Confirm),
            _ => None,
        }
    }

    /// 占位种类的人类可读说明（报告用，不泄漏枚举名）。
    pub fn as_str(&self) -> &'static str {
        match self {
            PlaceholderKind::Missing => "缺素材",
            PlaceholderKind::Confirm => "待裁决",
        }
    }
}

/// 正文中的一处占位。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placeholder {
    pub kind: PlaceholderKind,
    /// 缺什么 / 要确认什么（人类可读，如「3 个类似项目业绩」）。
    pub what: String,
    /// 正文行号（**1 起**——给人看的坐标，直接对上编辑器行号）。
    pub line: usize,
}

impl Placeholder {
    /// 重建占位文本（与 [`make_placeholder`] 同构——往返一致）。
    pub fn render(&self) -> String {
        make_placeholder(self.kind, &self.what)
    }

    /// 人类可读描述（报告/告警面）：「第 12 行【缺素材】3 个类似项目业绩」。
    pub fn describe(&self) -> String {
        format!("第 {} 行【{}】{}", self.line, self.kind.as_str(), self.what)
    }
}

impl std::fmt::Display for Placeholder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

/// 构造占位文本：「【待补充：请提供 3 个类似项目业绩】」。
///
/// `what` 里的开/闭方括号与换行会被清洗（否则占位会提前闭合或被扫描面
/// 截断——产物必须是可反解的自描述标记；空 `what` 也补兜底文案，
/// **占位绝不产出空壳**）。
pub fn make_placeholder(kind: PlaceholderKind, what: &str) -> String {
    let cleaned: String = what
        .chars()
        .map(|c| match c {
            CLOSE => '）',
            OPEN => '（',
            '\r' | '\n' => ' ',
            c => c,
        })
        .collect();
    let what = cleaned.trim();
    let what = if what.is_empty() { "具体内容待说明" } else { what };
    format!("{OPEN}{}{SEP}{what}{CLOSE}", kind.label())
}

/// 扫描正文里的全部占位（行号 1 起，文档序）。
///
/// - 同一行多处占位 → 逐处列出；
/// - 未闭合 / 跨行 / 标签词不认识 → **跳过**（不猜——不是占位就当正文）；
/// - `what` 两端空白裁掉；空 `what` 仍算占位（占位空壳本身就是要报的缺陷）。
///
/// 复杂度 O(len)：单遍字符扫，不建正则（12,000 章正文量级也要快）。
pub fn scan_placeholders(content: &str) -> Vec<Placeholder> {
    let mut out = Vec::new();
    let mut line = 1usize;
    // 当前占位起点（字节下标）
    let mut open_at: Option<usize> = None;
    for (idx, ch) in content.char_indices() {
        match ch {
            '\n' => {
                // 换行打断未闭合占位（跨行不是占位——不当占位吞正文）
                open_at = None;
                line += 1;
            }
            OPEN => open_at = Some(idx),
            CLOSE => {
                if let Some(start) = open_at.take() {
                    let body_start = start + OPEN.len_utf8();
                    let body = &content[body_start..idx];
                    if let Some((label, what)) = body.split_once(SEP) {
                        if let Some(kind) = PlaceholderKind::from_label(label.trim()) {
                            out.push(Placeholder { kind, what: what.trim().to_string(), line });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// 正文里残留占位数（校验/闸门用：0 = 无待补充）。
pub fn count_placeholders(content: &str) -> usize {
    scan_placeholders(content).len()
}

/// 正文含占位 → 人类可读清单（None = 干净）。
///
/// 报告口径：先说总数再逐条列，上限 10 条（长稿不至于淹掉报告）。
pub fn placeholder_report(content: &str) -> Option<String> {
    let ps = scan_placeholders(content);
    if ps.is_empty() {
        return None;
    }
    let shown: Vec<String> = ps.iter().take(10).map(Placeholder::describe).collect();
    let more = if ps.len() > shown.len() {
        format!("……等共 {} 处", ps.len())
    } else {
        String::new()
    };
    Some(format!(
        "正文残留 {} 处占位（导出前必须补齐或显式豁免）：{}{}",
        ps.len(),
        shown.join("；"),
        more
    ))
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_and_scan_roundtrip() {
        let p = make_placeholder(PlaceholderKind::Missing, "请提供 3 个类似项目业绩");
        assert_eq!(p, "【待补充：请提供 3 个类似项目业绩】");
        let found = scan_placeholders(&p);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, PlaceholderKind::Missing);
        assert_eq!(found[0].what, "请提供 3 个类似项目业绩");
        assert_eq!(found[0].line, 1);
        assert_eq!(found[0].render(), p, "往返一致");
    }

    #[test]
    fn scan_reports_line_numbers_and_multiple_per_line() {
        let content = "第一段正文。\n\n第二段【待补充：资质证书】与【待确认：工期口径】并列。\n尾段。";
        let found = scan_placeholders(content);
        assert_eq!(found.len(), 2, "同一行两处都要列出：{found:?}");
        assert_eq!(found[0].line, 3);
        assert_eq!(found[1].line, 3);
        assert_eq!(found[0].kind, PlaceholderKind::Missing);
        assert_eq!(found[1].kind, PlaceholderKind::Confirm);
    }

    #[test]
    fn scan_ignores_non_placeholder_and_malformed() {
        // 普通强调括号、未闭合、跨行 —— 都不算占位
        let content = "【注：这是正文里的强调】\n【待补充：闭合正常】\n【待补充：没闭合\n待补充：无括号】";
        let found = scan_placeholders(content);
        assert_eq!(found.len(), 1, "只应命中闭合且标签合法的一处：{found:?}");
        assert_eq!(found[0].what, "闭合正常");
        assert_eq!(found[0].line, 2);
    }

    #[test]
    fn make_placeholder_sanitizes_what() {
        let p = make_placeholder(PlaceholderKind::Confirm, "第一行\n第二行】混入");
        assert_eq!(scan_placeholders(&p).len(), 1, "净化后仍须可反解：{p}");
        assert!(!p.contains('\n'));
        assert_eq!(p.matches(CLOSE).count(), 1, "只允许一个闭合括号：{p}");
    }

    #[test]
    fn empty_what_still_yields_legal_placeholder() {
        let p = make_placeholder(PlaceholderKind::Missing, "   ");
        let found = scan_placeholders(&p);
        assert_eq!(found.len(), 1);
        assert!(!found[0].what.is_empty(), "占位不得为空壳：{p}");
    }

    #[test]
    fn report_counts_and_truncates() {
        let clean = "全是正常正文，没有占位。";
        assert_eq!(placeholder_report(clean), None);
        assert_eq!(count_placeholders(clean), 0);

        let many: String = (0..12).map(|i| format!("【待补充：条目 {i}】\n")).collect();
        assert_eq!(count_placeholders(&many), 12);
        let report = placeholder_report(&many).unwrap();
        assert!(report.contains("12 处"), "{report}");
        assert!(report.contains("等共 12 处"), "{report}");
    }

    #[test]
    fn describe_is_human_readable_not_enum_names() {
        let p = Placeholder { kind: PlaceholderKind::Missing, what: "3 个类似项目业绩".into(), line: 7 };
        let d = p.describe();
        assert_eq!(d, "第 7 行【缺素材】3 个类似项目业绩");
        assert!(!d.contains("Missing"), "不得泄漏枚举名：{d}");
    }
}
