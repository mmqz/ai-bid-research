//! star_scan.rs —— #285 ① ★漏标检测内核（纯函数，零 IO）。
//!
//! 问题（issue #285）：★实质性条款的抽取/标记质量没人看得见——六场黑盒
//! 实测捕获率 1/5 ~ 5/5 波动，danqing 丢 4/5 直接威胁不废标（★一条不响应
//! 即响应无效）。
//!
//! 思路：**★ 是可机器检测的硬信号**——不需要模型判断，纯字符串 + 集合运算：
//!
//! ```text
//! ★ 字面在文件里的出现（排除样板句声明）  → 真值集合（可精确计算）
//! 已抽取 REQ 的 anchors/params.star      → 实际集合
//! 差集                                   → 漏标候选（可精确列举）
//! ```
//!
//! ## 口径（三级恒等式——测试固化）
//!
//! ```text
//! star_total（字面★总数，含样板句——与 fixtures manifest star_count 同口径）
//!   = declaration_stars（样板句★——规则 c 排除，不是真值）
//!   + truth_stars（真值★）
//! truth_stars = captured_stars + missing_stars（不漏不重）
//! ```
//!
//! ## 样板句排除（规则 c——防假阳性）
//!
//! 行内含「打★号」「标★」「★号条款」「非★号」→ 是声明句，不算真值。
//! 语料实测形态（引号变体 + 跨行碎片）：
//! - `打“★”号条款为实质性条款，若有一条不响应或不满足则导致响应无效。`
//! - `★号条款必须满足，否则报价无效。技术参` / `数中非★号的条款，供应商完全满足招文件`
//!   （同一句被 pdftotext 撕成两行——两半都按声明排除）。
//! 判定前先 NFKC 归一（康熙部首 ⾮→非）+ 剥引号（“ ” " 等）。
//!
//! ## REQ 匹配（双腿）
//!
//! 1. **locator 腿**：`REQ.anchors[].locator == 块 locator`（抽取面/标记面
//!    锚都在块上创建——excerpt 即块原文，天然携带★信息，#276 信号③兜底）；
//! 2. **★前缀腿**：anchor.excerpt 包含该行的★内容前缀（`2 ★ 服务地点`
//!    → `"2 ★ 服务地点"`——前缀带★后的第一个内容 token 才够独特；裸
//!    `3 ★`（参数性质列独行）子串会在多表间撞车，**只走 locator 腿**），
//!    且两腿都解析出页号时要求同页（防跨表同号假命中）。
//!
//! REQ 侧★信号（① params.star ② params.substantive——#276 标记面 +
//! #280 抽取面在 `params.star` 汇流）用于 captured 明细的**优先展示**
//! （star-marked REQ 优先列出），不改变命中判定——命中由位置决定，
//! 信号决定「标没标」。

use crate::{Block, ParsedFile};
use bid_domain::Requirement;
use regex::Regex;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

/// 样板句判定（issue #285 规则 c）。
///
/// 输入原文；内部 NFKC 归一 + 剥引号后按四模式匹配。
/// `pub` 供 MCP 工具层复用（声明样本回显共用同一判定）。
pub fn is_star_declaration(text: &str) -> bool {
    let mut s: String = text.nfkc().collect();
    for q in [
        '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '"', '\'', '「', '」', '『', '』', '【',
        '】',
    ] {
        s = s.replace(q, "");
    }
    s.contains("打★号") || s.contains("标★") || s.contains("★号条款") || s.contains("非★号")
}

/// ★内容前缀（★前缀腿的匹配键）。
///
/// 定义：★之前的最后一个 token + `★` + ★之后的第一个 token，内部空白折叠
/// 为单空格（块文本与锚文 excerpt 同经 collapse_spaces，单空格形态可互含）。
/// 三种形态：
/// - `"2 ★ 服务地点 模型试验…"` → `"2 ★ 服务地点"`（带内容——可走前缀腿）
/// - `"★网络直连功能：…"`（★后无空格）→ `"★网络直连功能：…"`（连续形态）
/// - `"3 ★"`（参数性质列独行——裸★）→ `""`（**不可走前缀腿**：`"3 ★"`
///   子串在任何同号表格行都会撞车，只允许 locator 腿精确命中）
pub fn content_star_prefix(text: &str) -> String {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let Some(pos) = t.find('★') else {
        return String::new();
    };
    let before: Vec<&str> = t[..pos].split(' ').filter(|s| !s.is_empty()).collect();
    let after: Vec<&str> = t[pos + '★'.len_utf8()..]
        .split(' ')
        .filter(|s| !s.is_empty())
        .collect();
    let Some(first_after) = after.first() else {
        // ★后无内容 token（行尾裸★）——前缀腿禁用
        return String::new();
    };
    match before.last() {
        Some(b) => format!("{b} ★ {first_after}"),
        None => {
            if t[pos + '★'.len_utf8()..].starts_with(' ') {
                format!("★ {first_after}")
            } else {
                // ★紧贴内容（"★网络直连…"）：前缀 = ★ + 连续 token
                format!("★{first_after}")
            }
        }
    }
}

/// locator → (页, 行)。三在产格式：
/// `第1页 行2`（poppler 首选路）/ `第1 页 > 第2 行`（纯 Rust 兜底路）/
/// `第1页 OCR 行2`（OCR）。解析不出 → (None, None)（DOCX 章节路径等）。
fn page_line_of(locator: &str) -> (Option<u32>, Option<u32>) {
    static RE_PAGE: OnceLock<Regex> = OnceLock::new();
    static RE_LINE_CN: OnceLock<Regex> = OnceLock::new(); // 第2 行
    static RE_LINE_BARE: OnceLock<Regex> = OnceLock::new(); // 行2
    let page = RE_PAGE
        .get_or_init(|| Regex::new(r"第\s*(\d+)\s*页").unwrap())
        .captures(locator)
        .and_then(|c| c[1].parse().ok());
    let line = RE_LINE_CN
        .get_or_init(|| Regex::new(r"第\s*(\d+)\s*行").unwrap())
        .captures(locator)
        .or_else(|| {
            RE_LINE_BARE
                .get_or_init(|| Regex::new(r"行\s*(\d+)").unwrap())
                .captures(locator)
        })
        .and_then(|c| c[1].parse().ok());
    (page, line)
}

/// 一处★命中（块级——一个块可能含多个★字符，`stars` 计数）。
#[derive(Debug, Clone, PartialEq)]
pub struct StarHit {
    pub block_idx: usize,
    pub locator: String,
    pub page: Option<u32>,
    pub line: Option<u32>,
    /// 块内★字符数（多★行如实计数——hq-dev 实测「31 ★ ★13.主机…」双★形态）。
    pub stars: usize,
    pub text: String,
    /// 样板句声明（规则 c 排除——不是真值）。
    pub declaration: bool,
    /// 表格行（cells 非空——DOCX TableRow；装配问题告警面）。
    pub table_row: bool,
}

/// 已捕获明细（块级；`req_id` 优先取 star-marked REQ——「标没标」可见）。
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedStar {
    pub block_idx: usize,
    pub locator: String,
    pub page: Option<u32>,
    pub line: Option<u32>,
    pub stars: usize,
    pub text: String,
    /// 命中的 REQ（star-marked 优先）。
    pub req_id: String,
    /// 命中 REQ 是否带★信号（params.star/substantive == true）。
    pub star: bool,
}

/// 漏标明细（块级）。
#[derive(Debug, Clone, PartialEq)]
pub struct MissingStar {
    pub block_idx: usize,
    pub locator: String,
    pub page: Option<u32>,
    pub line: Option<u32>,
    pub stars: usize,
    pub text: String,
    /// 漏标原因（默认「该块未产生任何 REQ」；表格行追加装配面提示）。
    pub reason: String,
}

/// ★漏标检测结果（文件级）。
#[derive(Debug, Clone, PartialEq)]
pub struct StarCoverage {
    /// ★字面总数（**含样板句**——与 manifest star_count 同口径，±2 对账面）。
    pub star_total: usize,
    /// 样板句★数（规则 c 排除——不是真值）。
    pub declaration_stars: usize,
    /// 样板句样本（≤5 条——引号原样，供人工核对判定）。
    pub declaration_samples: Vec<String>,
    /// 真值★数 = star_total - declaration_stars。
    pub truth_stars: usize,
    /// 已捕获（位置对应 REQ 在场）。
    pub captured_stars: usize,
    /// 漏标（真值 - 已捕获）。
    pub missing_stars: usize,
    /// 已捕获明细（块级，文档序）。
    pub captured: Vec<CapturedStar>,
    /// 漏标明细（块级，文档序）。
    pub missing: Vec<MissingStar>,
}

impl StarCoverage {
    /// 三级恒等式自检（不漏不重）——工具层与测试共用。
    pub fn invariant_holds(&self) -> bool {
        self.star_total == self.declaration_stars + self.truth_stars
            && self.truth_stars == self.captured_stars + self.missing_stars
            && self.captured_stars == self.captured.iter().map(|c| c.stars).sum::<usize>()
            && self.missing_stars == self.missing.iter().map(|m| m.stars).sum::<usize>()
    }
}

/// 扫描一个文件的全部★命中（字面真值面——不含 REQ 匹配）。
pub fn scan_stars(pf: &ParsedFile) -> Vec<StarHit> {
    pf.blocks
        .iter()
        .filter(|b| b.text.contains('★'))
        .map(|b| {
            let (page, line) = page_line_of(&b.locator);
            StarHit {
                block_idx: b.idx,
                locator: b.locator.clone(),
                page,
                line,
                stars: b.text.matches('★').count(),
                text: b.text.clone(),
                declaration: is_star_declaration(&b.text),
                table_row: !b.cells.is_empty(),
            }
        })
        .collect()
}

/// REQ 是否位置命中该块（双腿：locator 精确 + ★前缀带页守卫）。
fn req_hits_block<'a>(b: &Block, file_name: &str, reqs: &'a [Requirement]) -> Vec<&'a Requirement> {
    let prefix = content_star_prefix(&b.text);
    let block_page = page_line_of(&b.locator).0;
    reqs.iter()
        .filter(|r| {
            r.anchors.iter().any(|a| {
                if a.file != file_name {
                    return false;
                }
                if a.locator == b.locator {
                    return true;
                }
                if prefix.is_empty() || !a.excerpt.contains(&prefix) {
                    return false;
                }
                // 页守卫：两腿都解析出页号时要求同页（防「2 ★ 服务期限」类
                // 前缀在同文异页撞车——页号是第二道闸）
                match (page_line_of(&a.locator).0, block_page) {
                    (Some(ap), Some(bp)) => ap == bp,
                    _ => true,
                }
            })
        })
        .collect()
}

/// ★漏标检测主入口：字面真值 vs 已抽 REQ 的差集。
///
/// `reqs` 传全量 REQ（多文件混合亦可）——按 `anchors[].file == pf.name`
/// 过滤本文件锚；未抽取时传空切片（全部真值计为漏标——诚实口径）。
pub fn star_coverage(pf: &ParsedFile, reqs: &[Requirement]) -> StarCoverage {
    let mut cov = StarCoverage {
        star_total: 0,
        declaration_stars: 0,
        declaration_samples: Vec::new(),
        truth_stars: 0,
        captured_stars: 0,
        missing_stars: 0,
        captured: Vec::new(),
        missing: Vec::new(),
    };
    for h in scan_stars(pf) {
        cov.star_total += h.stars;
        if h.declaration {
            cov.declaration_stars += h.stars;
            if cov.declaration_samples.len() < 5 {
                cov.declaration_samples.push(h.text.clone());
            }
            continue;
        }
        cov.truth_stars += h.stars;
        // 块本体（双腿匹配以块为键）
        let Some(block) = pf.blocks.iter().find(|b| b.idx == h.block_idx) else {
            continue; // 防御：scan_stars 产自 pf.blocks，不可达
        };
        let matched = req_hits_block(block, &pf.name, reqs);
        if matched.is_empty() {
            cov.missing_stars += h.stars;
            let reason = if h.table_row {
                "★在表格行内（cells 非空）——可能因表格装配问题漏抽（该块未产生任何 REQ）"
                    .to_string()
            } else {
                "该块未产生任何 REQ".to_string()
            };
            cov.missing.push(MissingStar {
                block_idx: h.block_idx,
                locator: h.locator.clone(),
                page: h.page,
                line: h.line,
                stars: h.stars,
                text: h.text.clone(),
                reason,
            });
        } else {
            cov.captured_stars += h.stars;
            // star-marked REQ 优先展示（★信号①②——「标没标」一眼可见）
            let best = matched
                .iter()
                .find(|r| {
                    r.params.get("star").and_then(|v| v.as_bool()) == Some(true)
                        || r.params.get("substantive").and_then(|v| v.as_bool()) == Some(true)
                })
                .copied()
                .unwrap_or(matched[0]);
            let star = best.params.get("star").and_then(|v| v.as_bool()) == Some(true)
                || best.params.get("substantive").and_then(|v| v.as_bool()) == Some(true);
            cov.captured.push(CapturedStar {
                block_idx: h.block_idx,
                locator: h.locator.clone(),
                page: h.page,
                line: h.line,
                stars: h.stars,
                text: h.text.clone(),
                req_id: best.id.clone(),
                star,
            });
        }
    }
    cov
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, BlockKind, ParsedFile};
    use bid_domain::{Anchor, Requirement};
    use std::collections::HashMap;

    fn block(idx: usize, locator: &str, text: &str) -> Block {
        Block {
            idx,
            kind: BlockKind::PdfLine,
            locator: locator.to_string(),
            text: text.to_string(),
            cells: vec![],
        }
    }

    fn parsed(blocks: Vec<Block>) -> ParsedFile {
        ParsedFile {
            name: "A-测试.pdf".into(),
            sha256: "x".into(),
            pages: 1,
            tables: 0,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks,
            parse_summary: "test".into(),
            components: vec![],
        }
    }

    fn req(id: &str, locator: &str, excerpt: &str, star: Option<bool>) -> Requirement {
        let mut params = HashMap::new();
        if let Some(s) = star {
            params.insert("star".to_string(), serde_json::json!(s));
        }
        Requirement {
            id: id.to_string(),
            category: "商务要求".into(),
            clause_no: None,
            anchors: vec![Anchor {
                file: "A-测试.pdf".into(),
                kind: "pdf_line".into(),
                locator: locator.to_string(),
                excerpt: excerpt.to_string(),
                // 旧快照形态（无块级定位）——star_scan 双腿匹配只消费
                // locator/excerpt（#285② 块位面不参与，兼容旧锚正是设计面）
                block_idx: None,
                page: None,
                line: None,
            }],
            params,
            source_file: "A-测试.pdf".into(),
        }
    }

    /// 规则 c 四模式 + 引号变体 + 跨行碎片。
    #[test]
    fn declaration_patterns() {
        for s in [
            "打“★”号条款为实质性条款，若有⼀条不响应或不满⾜则导致响应⽆效。",
            "打“★\"号条款为实质性条款，若有任何⼀条负偏离或不满⾜则导致响应⽆效。",
            "★号条款必须满⾜，否则报价⽆效。技术参",
            "数中⾮★号的条款，供应商完全满⾜招⽂件", // NFKC ⾮→非
            "标★的条款为实质性条款",
        ] {
            assert!(is_star_declaration(s), "应为声明句: {s}");
        }
        // 真值不得被误伤
        for s in [
            "1 ★ 服务期限",
            "★服务地点：松岭区",
            "★⽹络直连功能：可通过⽹线传输",
            "31 ★ ★13.主机：CPU 13代I3及以上",
        ] {
            assert!(!is_star_declaration(s), "应为真值: {s}");
        }
    }

    /// ★内容前缀三形态 + 裸★禁用前缀腿。
    #[test]
    fn prefix_forms() {
        assert_eq!(
            content_star_prefix("2 ★ 服务地点 模型试验在供应商场地"),
            "2 ★ 服务地点"
        );
        assert_eq!(content_star_prefix("★ 1.1"), "★ 1.1");
        assert_eq!(
            content_star_prefix("★⽹络直连功能：可通过⽹线"),
            "★⽹络直连功能：可通过⽹线"
        );
        assert_eq!(
            content_star_prefix("★服务地点：松岭区"),
            "★服务地点：松岭区"
        );
        // 裸★（参数性质列独行）——前缀腿禁用
        assert_eq!(content_star_prefix("3 ★"), "");
        assert_eq!(content_star_prefix("★"), "");
    }

    /// locator 三格式页行解析。
    #[test]
    fn page_line_parsing() {
        assert_eq!(page_line_of("第19页 行6"), (Some(19), Some(6)));
        assert_eq!(page_line_of("第1 页 > 第2 行"), (Some(1), Some(2)));
        assert_eq!(page_line_of("第1页 OCR 行2"), (Some(1), Some(2)));
        assert_eq!(page_line_of("第三章 采购需求"), (None, None));
    }

    /// 核心口径：三级恒等式 + 样板句排除 + 双腿匹配。
    #[test]
    fn coverage_partition_and_matching() {
        let pf = parsed(vec![
            block(0, "第18页 行9", "1 ★"),  // 裸★——locator 腿
            block(1, "第18页 行25", "2 ★"), // 裸★ 无 REQ → 漏
            block(
                2,
                "第19页 行1",
                "打“★”号条款为实质性条款，若有⼀条不响应则响应⽆效。",
            ), // 声明排除
            block(3, "第19页 行6", "1 ★ 服务期限"), // locator 腿 + star
            block(4, "第19页 行8", "2 ★ 服务地点 松岭区"), // 前缀腿（同页）
        ]);
        let reqs = vec![
            req("REQ-0001", "第18页 行9", "1 ★", None), // 命中块0（无 star 标记）
            req("REQ-0002", "第19页 行6", "1 ★ 服务期限", Some(true)), // 命中块3
            req(
                "REQ-0003",
                "第19页 行30",
                "服务期限要求：合同签订后30日内",
                None,
            ), // 不含★前缀
            // 块4 的前缀腿命中方：锚在表头行（异 locator），excerpt 含★前缀且同页
            req(
                "REQ-0004",
                "第19页 行2",
                "商务要求表：2 ★ 服务地点 松岭区",
                None,
            ),
        ];
        let cov = star_coverage(&pf, &reqs);
        assert_eq!(cov.star_total, 5, "字面★总数（含样板句）");
        assert_eq!(cov.declaration_stars, 1, "样板句排除");
        assert_eq!(cov.truth_stars, 4);
        assert_eq!(cov.captured_stars, 3, "块0/3/4 捕获");
        assert_eq!(cov.missing_stars, 1, "块1 漏标");
        assert!(cov.invariant_holds(), "不漏不重恒等式");
        // captured 明细：star-marked 优先展示
        let c3 = cov.captured.iter().find(|c| c.block_idx == 3).unwrap();
        assert_eq!(c3.req_id, "REQ-0002");
        assert!(c3.star);
        let c0 = cov.captured.iter().find(|c| c.block_idx == 0).unwrap();
        assert_eq!(c0.req_id, "REQ-0001");
        assert!(!c0.star, "命中但未标★——「标没标」可见");
        // missing 明细
        assert_eq!(cov.missing.len(), 1);
        assert_eq!(cov.missing[0].block_idx, 1);
        assert_eq!(cov.missing[0].reason, "该块未产生任何 REQ");
    }

    /// 裸★前缀腿禁用的原因：跨表同号撞车（danqing 实测形态）。
    ///
    /// p18 技术表「2 ★」与 p19 商务表 REQ（excerpt「2 ★ 服务地点…」）
    /// 若按子串匹配会假命中——前缀腿必须拒绝裸★。
    #[test]
    fn bare_star_rejects_substring_collision() {
        let pf = parsed(vec![
            block(0, "第18页 行25", "2 ★"),
            block(1, "第19页 行8", "2 ★ 服务地点 松岭区"),
        ]);
        let reqs = vec![req(
            "REQ-0001",
            "第19页 行8",
            "2 ★ 服务地点 松岭区",
            Some(true),
        )];
        let cov = star_coverage(&pf, &reqs);
        assert_eq!(cov.missing_stars, 1, "p18「2 ★」不得因子串撞车假捕获");
        assert_eq!(cov.captured_stars, 1);
        assert!(cov.invariant_holds());
    }

    /// 前缀腿的页守卫：同文异页同前缀 → 页号第二道闸。
    #[test]
    fn prefix_leg_page_guard() {
        let pf = parsed(vec![
            block(0, "第18页 行25", "2 ★ 服务期限"),
            block(1, "第25页 行8", "2 ★ 服务期限"),
        ]);
        // REQ 只锚在 p25 的表头行（excerpt 含 p18 行前缀文本——跨页装配形态）
        let reqs = vec![req("REQ-0001", "第25页 行1", "表头 2 ★ 服务期限", None)];
        let cov = star_coverage(&pf, &reqs);
        // p25 块：前缀腿同页命中；p18 块：页守卫拦截
        assert_eq!(cov.captured_stars, 1, "仅同页命中");
        assert_eq!(cov.missing_stars, 1, "异页被页守卫拦截");
    }

    /// 多★行如实计数（hq-dev「31 ★ ★13.主机…」双★形态）。
    #[test]
    fn multi_star_block_counts_all() {
        let pf = parsed(vec![block(
            0,
            "第20页 行65",
            "31 ★ ★13.主机：CPU：13代I3及以上",
        )]);
        let cov = star_coverage(&pf, &[]);
        assert_eq!(cov.star_total, 2, "一个块两个★字符");
        assert_eq!(cov.missing_stars, 2);
        assert_eq!(cov.missing.len(), 1, "明细块级一条");
        assert_eq!(cov.missing[0].stars, 2);
        assert!(cov.invariant_holds());
    }

    /// 未抽取（空 REQ）诚实口径：全部真值计漏标。
    #[test]
    fn no_reqs_all_missing() {
        let pf = parsed(vec![
            block(0, "第1页 行1", "1 ★ 服务期限"),
            block(1, "第2页 行1", "打“★”号条款为实质性条款。"),
        ]);
        let cov = star_coverage(&pf, &[]);
        assert_eq!(cov.star_total, 2);
        assert_eq!(cov.truth_stars, 1);
        assert_eq!(cov.missing_stars, 1);
        assert!(cov.invariant_holds());
    }

    /// DOCX 表格行（cells 非空）漏标原因带装配面提示。
    #[test]
    fn table_row_missing_reason() {
        let b = Block {
            idx: 0,
            kind: BlockKind::TableRow,
            locator: "表1 行2".into(),
            text: "2 ★ 服务地点 松岭区".into(),
            cells: vec!["2".into(), "★".into(), "服务地点".into(), "松岭区".into()],
        };
        let pf = parsed(vec![b]);
        let cov = star_coverage(&pf, &[]);
        assert_eq!(
            cov.missing[0].reason,
            "★在表格行内（cells 非空）——可能因表格装配问题漏抽（该块未产生任何 REQ）"
        );
    }

    /// 其他文件的锚不参与本文件匹配（多文件混合 REQ 面）。
    #[test]
    fn other_file_anchors_ignored() {
        let pf = parsed(vec![block(0, "第1页 行1", "1 ★ 服务期限")]);
        let mut r = req("REQ-0001", "第1页 行1", "1 ★ 服务期限", Some(true));
        r.anchors[0].file = "B-其他文件.pdf".into();
        let cov = star_coverage(&pf, &[r]);
        assert_eq!(cov.missing_stars, 1, "他文件锚不算本文件命中");
    }
}
