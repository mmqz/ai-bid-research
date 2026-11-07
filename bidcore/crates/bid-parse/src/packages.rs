//! #316 多标段/多包识别与 REQ 归属（三形态——文件名 / 章节结构 / 分包清单表格）。
//!
//! 真实形态实证（batch2）：
//! - 文件名：`…-深部微波治疗仪等医疗设备采购项目公开招标文件-第2包.doc`
//!   （单文件即单包——整件归属该包）；
//! - 章节结构：`第一章 第1包 技术要求 / 第二章 第2包 技术要求`（一文件多包
//!   ——块按所属章归属，章外为共用）；
//! - 表格：分包/标段一览表（TableRow 单行携带包标记——该行条款归属该包）。
//!
//! 归属算法（块级——PDF 锚 locator 是页/行形态无章信息，必须按块序扫描）：
//! 1. 文件名带单一包标记 → **文件级归属**（该件全部 REQ 归该包，最权威）；
//! 2. 否则逐块维护「当前包上下文」：
//!    - 标题形块（章头/短行）含**唯一**包标记 → 切换上下文；
//!    - 章级标题（第X章/第X部分）无标记 → 复位为共用（后面的格式章不误吞）；
//!    - 非标题块继承上下文；表格行自带标记则行级覆盖；
//!    - 单文本提及 ≥2 个不同包号 → 共用（「可投第1包或第2包」类总括句）；
//! 3. REQ 按锚回填：anchor.block_idx → 块归属；无块位退 locator/excerpt 文本
//!    匹配；仍无 → 共用（None——选任何包都进纲）。
//!
//! 纪律：
//! - **匹配瞬间 NFKC 归一化**（康熙部首污染——`第⼆章` 的 `⼆` 是 U+2F08，
//!   与 extract.rs 章节识别同款防御），不改任何落库数据；
//! - REQ 产物零位移：只写 `params["package_id"]`（#271 form_file_id 同款
//!   加性模式），不动 id/category/anchors——requirement_hash 不含 params，
//!   幂等合并不受影响；
//! - 无分包件 → 空注册表 + 全部 REQ 不标（调用面行为与 #316 前逐位一致）；
//! - 形态学不绑字面：`包`/`标段` 双词面、中文/阿拉伯数字、全/半角括号。
//!
//! 已知边界（如实登记）：
//! - 罗马数字标段（Ⅰ/Ⅱ 标段）不识别——真实语料未见，宁缺勿错；
//! - 「N 标段」无「第」前缀的独立形态只认标题/表格行（正文误配风险）。

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use bid_domain::packages::{PackageInfo, REQ_PARAM_PACKAGE_ID};
use bid_domain::Requirement;

use crate::{Block, BlockKind, ParsedFile};

/// 纯 NFKC（不剥空白）——只在匹配瞬间归一化，落库数据保持原文
/// （与 extract.rs `nfkc` 同纪律；不复用是为保持两模块独立演进）。
fn nfkc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkc().collect()
}

/// 中文数字 → u32（一~九十九；阿拉伯数字串直接解析）。
fn cn_number(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    let digits: Vec<Option<u32>> = s
        .chars()
        .map(|c| match c {
            '一' | '壹' => Some(1),
            '二' | '贰' => Some(2),
            '三' | '叁' => Some(3),
            '四' | '肆' => Some(4),
            '五' | '伍' => Some(5),
            '六' | '陆' => Some(6),
            '七' | '柒' => Some(7),
            '八' | '捌' => Some(8),
            '九' | '玖' => Some(9),
            '十' | '拾' => Some(10),
            _ => None,
        })
        .collect();
    if digits.is_empty() || digits.iter().any(|d| d.is_none()) {
        return None;
    }
    let ds: Vec<u32> = digits.into_iter().map(|d| d.unwrap()).collect();
    match ds.as_slice() {
        [n] if *n < 10 => Some(*n),
        [10] => Some(10),
        [10, u] if *u < 10 => Some(10 + u), // 十一..十九
        [1, 10] => Some(10),                // 「一十」异写
        [t, 10] if (2..10).contains(t) => Some(t * 10), // 二十..九十
        [t, 10, u] if (2..10).contains(t) && *u < 10 => Some(t * 10 + u), // 二十一..
        _ => None,
    }
}

/// `第N包` / `第N标段`（N = 1-3 位阿拉伯或中文数字；NFKC 后匹配）。
fn marker_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"第\s*([0-9]{1,3}|[一二三四五六七八九十]{1,3})\s*(包|标段)").unwrap()
    })
}

/// `N标段` 无「第」前缀的独立形态（前界防误配「2026年第1标段」外的连数字）。
fn standalone_section_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:^|[^0-9一二三四五六七八九十])([0-9]{1,2}|[一二三四五六七八九十]{1,3})\s*标段",
        )
        .unwrap()
    })
}

/// 章级标题形态（上下文复位的判定面——「第X章」「第X部分」开头）。
fn chapter_heading_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*第\s*[0-9一二三四五六七八九十]{1,3}\s*[章部]").unwrap())
}

/// 文本中的全部包标记 → 归一化 (number, kind) 集合（去重保序）。
fn markers_in(text: &str) -> Vec<(u32, &'static str)> {
    let norm = nfkc(text);
    let mut out: Vec<(u32, &'static str)> = Vec::new();
    let mut push = |n: u32, k: &'static str| {
        if !out.contains(&(n, k)) {
            out.push((n, k));
        }
    };
    for cap in marker_regex().captures_iter(&norm) {
        if let (Some(num), Some(kind)) = (cap.get(1), cap.get(2)) {
            if let Some(n) = cn_number(num.as_str()) {
                push(n, kind_str(kind.as_str()));
            }
        }
    }
    for cap in standalone_section_regex().captures_iter(&norm) {
        if let Some(num) = cap.get(1) {
            if let Some(n) = cn_number(num.as_str()) {
                push(n, "标段");
            }
        }
    }
    out
}

fn kind_str(k: &str) -> &'static str {
    if k.contains("标段") {
        "标段"
    } else {
        "包"
    }
}

/// 标题形判定（PDF 上下文切换面）：章头 或 短行含标记。
/// TableRow 不走此面（行级覆盖另算）。
fn is_context_heading(b: &Block, norm_text: &str) -> bool {
    if matches!(b.kind, BlockKind::TableRow | BlockKind::XlsxRow) {
        return false;
    }
    if chapter_heading_regex().is_match(norm_text) {
        return true;
    }
    norm_text.chars().count() <= 50
        && marker_regex().is_match(norm_text)
        && !norm_text.contains("。")
        && !norm_text.contains("；")
}

/// 单文件扫描产物。
struct FileScan {
    /// 文件级归属（文件名单一包标记——整件权威归属）
    file_package: Option<(u32, &'static str)>,
    /// 块级归属（block idx → 归一化包号）
    block_packages: HashMap<usize, u32>,
    /// 识别到的标记（证据收集用）
    markers: Vec<(u32, &'static str, String)>,
}

fn scan_file(f: &ParsedFile) -> FileScan {
    // ---- 形态① 文件名（stem 去扩展名——防「第2包.docx」的扩展名干扰）
    let stem = f.name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&f.name);
    let name_markers = markers_in(stem);
    let file_package = match name_markers.as_slice() {
        [(n, k)] => Some((*n, *k)),
        _ => None, // 无标记或多个标记（怪名）→ 不做文件级归属
    };

    let mut block_packages: HashMap<usize, u32> = HashMap::new();
    let mut markers: Vec<(u32, &'static str, String)> = Vec::new();
    for (n, k) in &name_markers {
        markers.push((*n, *k, format!("filename:{}", f.name)));
    }

    // ---- 形态②/③ 块扫描（章节上下文 + 表格行覆盖）
    let mut current: Option<u32> = None;
    for b in &f.blocks {
        let norm = nfkc(&b.text);
        let loc_norm = nfkc(&b.locator);
        // 该块自身证据：locator（DOCX 章节路径——形态②）+ 文本（表格行/正文）
        let loc_markers = markers_in(&loc_norm);
        let text_markers = markers_in(&norm);
        let mut own: Vec<(u32, &'static str)> = loc_markers.clone();
        for m in &text_markers {
            if !own.contains(m) {
                own.push(*m);
            }
        }
        let is_row = matches!(b.kind, BlockKind::TableRow | BlockKind::XlsxRow);
        let mut record = |n: u32, k: &'static str, evidence: String| {
            if !markers
                .iter()
                .any(|(n2, k2, e2)| *n2 == n && *k2 == k && *e2 == evidence)
            {
                markers.push((n, k, evidence));
            }
        };
        for (n, k) in &loc_markers {
            record(*n, *k, format!("chapter:{}", truncate(&b.locator, 40)));
        }
        for (n, k) in &text_markers {
            let evidence = if is_row {
                format!("table:{}", truncate(&b.text, 40))
            } else {
                format!("text:{}", truncate(&b.text, 40))
            };
            record(*n, *k, evidence);
        }

        if is_row {
            // 表格行：自带标记 → 行级归属（不改上下文——表后正文仍随章）
            if let [(n, _)] = own.as_slice() {
                block_packages.insert(b.idx, *n);
            }
            continue;
        }
        let heading = is_context_heading(b, &norm);
        if heading {
            // 标题形：唯一标记 → 切换；章头无标记 → 复位共用
            current = match own.as_slice() {
                [(n, _)] => Some(*n),
                [] => None,
                _ => current, // 多标记总括标题（「第1包第2包一览」）→ 不切换
            };
        }
        let effective = match (own.as_slice(), heading) {
            ([(n, _)], _) => Some(*n), // 自带唯一标记（标题或正文单提）
            (_, true) => current,      // 标题无标记 → 复位后的共用
            (_, false) => current,     // 正文继承上下文
        };
        if let Some(n) = effective {
            block_packages.insert(b.idx, n);
        }
    }

    FileScan {
        file_package,
        block_packages,
        markers,
    }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// 识别 + 归属一步完成（extract_requirements 合并面之后调用）。
///
/// 返回分包注册表（按 number 升序、PKG-xx 确定性 id）；REQ 就地写
/// `params["package_id"]`。无分包 → 空注册表，REQ 不标（零位移）。
pub fn detect_and_attribute(files: &[ParsedFile], reqs: &mut [Requirement]) -> Vec<PackageInfo> {
    let scans: Vec<(usize, FileScan)> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (i, scan_file(f)))
        .collect();

    // ---- 注册表（跨文件合并证据；label 用归一化形态——确定性，重跑幂等）
    let mut labels: HashMap<u32, (String, &'static str)> = HashMap::new();
    let mut sources: HashMap<u32, Vec<String>> = HashMap::new();
    let mut push_marker = |n: u32, k: &'static str, label: &str, evidence: String| {
        labels.entry(n).or_insert_with(|| (label.to_string(), k));
        sources.entry(n).or_default().push(evidence);
    };
    for (_, s) in &scans {
        for (n, k, evidence) in &s.markers {
            let label = marker_label(*n, *k);
            push_marker(*n, *k, &label, evidence.clone());
        }
    }
    let mut packages: Vec<PackageInfo> = labels
        .into_iter()
        .map(|(n, (label, kind))| {
            PackageInfo::new(
                n,
                label,
                kind.to_string(),
                sources.remove(&n).unwrap_or_default(),
            )
        })
        .collect();
    packages.sort_by_key(|p| p.number);
    if packages.is_empty() {
        return packages; // 无分包——零位移
    }
    let id_of = |n: u32| format!("PKG-{n:02}");

    // ---- REQ 归属（锚驱动）
    for r in reqs.iter_mut() {
        // 每候选包号计票（锚多数决——多锚 REQ 如双锚点形态）
        let mut votes: HashMap<u32, usize> = HashMap::new();
        for a in &r.anchors {
            let Some((_, scan)) = scans.iter().find(|(i, _)| files[*i].name == a.file) else {
                continue;
            };
            if let Some((n, _)) = scan.file_package {
                // 文件级归属最权威——直接定案
                *votes.entry(n).or_insert(0) += 10;
                continue;
            }
            if let Some(bi) = a.block_idx {
                if let Some(n) = scan.block_packages.get(&bi) {
                    *votes.entry(*n).or_insert(0) += 2;
                    continue;
                }
            }
            // 兜底：锚文本/locator 单一标记（旧路径无块位）
            let ms = markers_in(&format!("{} {}", a.locator, a.excerpt));
            if let [(n, _)] = ms.as_slice() {
                *votes.entry(*n).or_insert(0) += 1;
            }
        }
        if let Some((n, _)) = votes
            .iter()
            .max_by_key(|(_, v)| **v)
            .filter(|(_, v)| **v > 0)
        {
            r.params.insert(
                REQ_PARAM_PACKAGE_ID.to_string(),
                serde_json::Value::String(id_of(*n)),
            );
        }
    }
    packages
}

fn marker_label(n: u32, kind: &str) -> String {
    format!("第{n}{kind}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pf(name: &str, blocks: Vec<Block>) -> ParsedFile {
        ParsedFile {
            name: name.into(),
            sha256: format!("sha-{name}"),
            pages: 1,
            tables: 0,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks,
            parse_summary: String::new(),
            components: vec![],
        }
    }

    fn para(idx: usize, locator: &str, text: &str) -> Block {
        Block {
            idx,
            kind: BlockKind::Para,
            locator: locator.into(),
            text: text.into(),
            cells: vec![],
        }
    }

    fn pdf_line(idx: usize, locator: &str, text: &str) -> Block {
        Block {
            idx,
            kind: BlockKind::PdfLine,
            locator: locator.into(),
            text: text.into(),
            cells: vec![],
        }
    }

    fn table_row(idx: usize, locator: &str, cells: &[&str]) -> Block {
        Block {
            idx,
            kind: BlockKind::TableRow,
            locator: locator.into(),
            text: cells.join(" "),
            cells: cells.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn req(
        id: &str,
        file: &str,
        block_idx: Option<usize>,
        locator: &str,
        excerpt: &str,
    ) -> Requirement {
        Requirement {
            id: id.into(),
            category: "技术参数".into(),
            clause_no: None,
            anchors: vec![bid_domain::Anchor {
                file: file.into(),
                kind: "para".into(),
                locator: locator.into(),
                excerpt: excerpt.into(),
                block_idx,
                page: None,
                line: None,
            }],
            params: HashMap::new(),
            source_file: file.into(),
        }
    }

    // ---------------------------------------------------------------- 形态① 文件名

    #[test]
    fn filename_single_package_attributes_whole_file() {
        let files = vec![pf(
            "国家区域医疗中心-医疗设备采购-第2包.docx",
            vec![
                para(0, "第一章 招标公告", "招标公告正文"),
                para(1, "第二章 技术要求", "设备参数要求"),
            ],
        )];
        let mut reqs = vec![
            req(
                "REQ-0001",
                "国家区域医疗中心-医疗设备采购-第2包.docx",
                Some(0),
                "第一章 招标公告",
                "招标公告正文",
            ),
            req(
                "REQ-0002",
                "国家区域医疗中心-医疗设备采购-第2包.docx",
                Some(1),
                "第二章 技术要求",
                "设备参数要求",
            ),
        ];
        let pkgs = detect_and_attribute(&files, &mut reqs);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].id, "PKG-02");
        assert_eq!(pkgs[0].number, 2);
        assert_eq!(pkgs[0].label, "第2包");
        assert!(pkgs[0].sources.iter().any(|s| s.starts_with("filename:")));
        for r in &reqs {
            assert_eq!(
                r.params.get(REQ_PARAM_PACKAGE_ID),
                Some(&json!("PKG-02")),
                "文件级归属：{} 应归第2包",
                r.id
            );
        }
    }

    // ---------------------------------------------------------------- 形态② 章节结构（DOCX locator）

    #[test]
    fn docx_chapter_structure_splits_packages() {
        let files = vec![pf(
            "A-招标文件.docx",
            vec![
                para(0, "第一章 招标公告", "第一章 招标公告"),
                para(1, "第一章 招标公告", "投标人须具备相应资质。"),
                para(2, "第二章 第1包 技术要求", "第二章 第1包 技术要求"),
                para(3, "第二章 第1包 技术要求", "第1包设备数量为10台。"),
                para(4, "第三章 第2包 技术要求", "第三章 第2包 技术要求"),
                para(5, "第三章 第2包 技术要求", "第2包设备为微波治疗仪。"),
                para(6, "第四章 投标文件格式", "第四章 投标文件格式"),
                para(7, "第四章 投标文件格式", "投标函应按格式填写。"),
            ],
        )];
        let mut reqs = vec![
            req(
                "REQ-0001",
                "A-招标文件.docx",
                Some(1),
                "第一章 招标公告",
                "投标人须具备相应资质。",
            ),
            req(
                "REQ-0002",
                "A-招标文件.docx",
                Some(3),
                "第二章 第1包 技术要求",
                "第1包设备数量为10台。",
            ),
            req(
                "REQ-0003",
                "A-招标文件.docx",
                Some(5),
                "第三章 第2包 技术要求",
                "第2包设备为微波治疗仪。",
            ),
            req(
                "REQ-0004",
                "A-招标文件.docx",
                Some(7),
                "第四章 投标文件格式",
                "投标函应按格式填写。",
            ),
        ];
        let pkgs = detect_and_attribute(&files, &mut reqs);
        assert_eq!(pkgs.len(), 2, "两包：{pkgs:?}");
        assert_eq!(
            (pkgs[0].id.as_str(), pkgs[1].id.as_str()),
            ("PKG-01", "PKG-02")
        );
        assert_eq!(
            reqs[0].params.get(REQ_PARAM_PACKAGE_ID),
            None,
            "第一章资格条款为共用"
        );
        assert_eq!(
            reqs[1].params.get(REQ_PARAM_PACKAGE_ID),
            Some(&json!("PKG-01"))
        );
        assert_eq!(
            reqs[2].params.get(REQ_PARAM_PACKAGE_ID),
            Some(&json!("PKG-02"))
        );
        assert_eq!(
            reqs[3].params.get(REQ_PARAM_PACKAGE_ID),
            None,
            "第四章格式条款为共用（章头复位）"
        );
    }

    // ---------------------------------------------------------------- 形态③ 分包清单表格

    #[test]
    fn table_rows_carry_row_level_package() {
        let files = vec![pf(
            "A-招标文件.docx",
            vec![
                para(0, "第一章 招标公告", "第一章 招标公告"),
                table_row(
                    1,
                    "第一章 招标公告 表1 行1",
                    &["包号", "包名称", "技术要求"],
                ),
                table_row(
                    2,
                    "第一章 招标公告 表1 行2",
                    &["第1包", "监护设备", "参数A"],
                ),
                table_row(
                    3,
                    "第一章 招标公告 表1 行3",
                    &["第2包", "治疗设备", "参数B"],
                ),
                para(4, "第一章 招标公告", "投标人可投第1包、第2包或全部两包。"),
            ],
        )];
        let mut reqs = vec![
            req(
                "REQ-0001",
                "A-招标文件.docx",
                Some(2),
                "第一章 招标公告 表1 行2",
                "第1包 监护设备 参数A",
            ),
            req(
                "REQ-0002",
                "A-招标文件.docx",
                Some(3),
                "第一章 招标公告 表1 行3",
                "第2包 治疗设备 参数B",
            ),
            req(
                "REQ-0003",
                "A-招标文件.docx",
                Some(4),
                "第一章 招标公告",
                "投标人可投第1包、第2包或全部两包。",
            ),
        ];
        let pkgs = detect_and_attribute(&files, &mut reqs);
        assert_eq!(pkgs.len(), 2);
        assert!(
            pkgs.iter()
                .any(|p| p.sources.iter().any(|s| s.starts_with("table:"))),
            "表格形态证据在案"
        );
        assert_eq!(
            reqs[0].params.get(REQ_PARAM_PACKAGE_ID),
            Some(&json!("PKG-01"))
        );
        assert_eq!(
            reqs[1].params.get(REQ_PARAM_PACKAGE_ID),
            Some(&json!("PKG-02"))
        );
        assert_eq!(
            reqs[2].params.get(REQ_PARAM_PACKAGE_ID),
            None,
            "多包并提的总括句为共用"
        );
    }

    // ---------------------------------------------------------------- PDF 章上下文（无 locator 章信息）

    #[test]
    fn pdf_context_switch_and_reset() {
        let files = vec![pf(
            "A-招标文件.pdf",
            vec![
                pdf_line(0, "第1页 行1", "第一章 招标公告"),
                pdf_line(1, "第1页 行2", "投标人须具备资质。"),
                pdf_line(2, "第2页 行1", "第二章 第1包 技术要求"),
                pdf_line(3, "第2页 行2", "设备数量为10台，含安装。"),
                pdf_line(4, "第3页 行1", "第三章 第2包 技术要求"),
                pdf_line(5, "第3页 行2", "微波治疗仪一批。"),
                pdf_line(6, "第4页 行1", "第四章 投标文件格式"),
                pdf_line(7, "第4页 行2", "投标函按格式填写。"),
            ],
        )];
        let mut reqs = vec![
            req(
                "REQ-0001",
                "A-招标文件.pdf",
                Some(1),
                "第1页 行2",
                "投标人须具备资质。",
            ),
            req(
                "REQ-0002",
                "A-招标文件.pdf",
                Some(3),
                "第2页 行2",
                "设备数量为10台，含安装。",
            ),
            req(
                "REQ-0003",
                "A-招标文件.pdf",
                Some(5),
                "第3页 行2",
                "微波治疗仪一批。",
            ),
            req(
                "REQ-0004",
                "A-招标文件.pdf",
                Some(7),
                "第4页 行2",
                "投标函按格式填写。",
            ),
        ];
        let pkgs = detect_and_attribute(&files, &mut reqs);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(reqs[0].params.get(REQ_PARAM_PACKAGE_ID), None, "公告章共用");
        assert_eq!(
            reqs[1].params.get(REQ_PARAM_PACKAGE_ID),
            Some(&json!("PKG-01"))
        );
        assert_eq!(
            reqs[2].params.get(REQ_PARAM_PACKAGE_ID),
            Some(&json!("PKG-02"))
        );
        assert_eq!(
            reqs[3].params.get(REQ_PARAM_PACKAGE_ID),
            None,
            "格式章复位共用"
        );
    }

    // ---------------------------------------------------------------- 无分包 / 标段 / 中文数字

    #[test]
    fn no_package_structure_is_zero_displacement() {
        let files = vec![pf(
            "A-招标文件.docx",
            vec![
                para(0, "第一章 招标公告", "第一章 招标公告"),
                para(1, "第一章 招标公告", "设备采购一批。"),
            ],
        )];
        let mut reqs = vec![req(
            "REQ-0001",
            "A-招标文件.docx",
            Some(1),
            "第一章 招标公告",
            "设备采购一批。",
        )];
        let pkgs = detect_and_attribute(&files, &mut reqs);
        assert!(pkgs.is_empty());
        assert!(reqs[0].params.is_empty(), "无分包 → REQ 不标（零位移）");
    }

    #[test]
    fn bid_section_and_chinese_numerals() {
        let files = vec![pf(
            "A-施工招标.docx",
            vec![
                para(0, "第一章 招标公告", "第一章 招标公告"),
                para(
                    1,
                    "第二章 第一标段 工程量清单",
                    "第二章 第一标段 工程量清单",
                ),
                para(2, "第二章 第一标段 工程量清单", "土方开挖若干。"),
                para(
                    3,
                    "第三章 第二标段 工程量清单",
                    "第三章 第二标段 工程量清单",
                ),
                para(4, "第三章 第二标段 工程量清单", "混凝土浇筑若干。"),
            ],
        )];
        let mut reqs = vec![
            req(
                "REQ-0001",
                "A-施工招标.docx",
                Some(2),
                "第二章 第一标段 工程量清单",
                "土方开挖若干。",
            ),
            req(
                "REQ-0002",
                "A-施工招标.docx",
                Some(4),
                "第三章 第二标段 工程量清单",
                "混凝土浇筑若干。",
            ),
        ];
        let pkgs = detect_and_attribute(&files, &mut reqs);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].kind, "标段");
        assert_eq!((pkgs[0].number, pkgs[1].number), (1, 2), "中文数字归一化");
        assert_eq!(
            reqs[0].params.get(REQ_PARAM_PACKAGE_ID),
            Some(&json!("PKG-01"))
        );
        assert_eq!(
            reqs[1].params.get(REQ_PARAM_PACKAGE_ID),
            Some(&json!("PKG-02"))
        );
    }

    #[test]
    fn kangxi_radicals_are_normalized_for_matching() {
        // 康熙部首 U+2F08（⼈形「二」）——NFKC 后成「二」
        let files = vec![pf(
            "A-招标文件.docx",
            vec![
                para(0, "第一章 招标公告", "第一章 招标公告"),
                para(1, "第二章 第⼆包 技术要求", "第二章 第⼆包 技术要求"),
                para(2, "第二章 第⼆包 技术要求", "设备参数若干。"),
            ],
        )];
        let mut reqs = vec![req(
            "REQ-0001",
            "A-招标文件.docx",
            Some(2),
            "第二章 第⼆包 技术要求",
            "设备参数若干。",
        )];
        let pkgs = detect_and_attribute(&files, &mut reqs);
        assert_eq!(pkgs.len(), 1, "康熙部首不致漏识别");
        assert_eq!(pkgs[0].number, 2);
        assert_eq!(
            reqs[0].params.get(REQ_PARAM_PACKAGE_ID),
            Some(&json!("PKG-02"))
        );
    }

    // ---------------------------------------------------------------- 数字解析单测

    #[test]
    fn cn_number_forms() {
        assert_eq!(cn_number("1"), Some(1));
        assert_eq!(cn_number("02"), Some(2));
        assert_eq!(cn_number("一"), Some(1));
        assert_eq!(cn_number("十"), Some(10));
        assert_eq!(cn_number("十一"), Some(11));
        assert_eq!(cn_number("二十"), Some(20));
        assert_eq!(cn_number("二十三"), Some(23));
        assert_eq!(cn_number("百"), None, "超界不猜");
        assert_eq!(cn_number(""), None);
    }
}
