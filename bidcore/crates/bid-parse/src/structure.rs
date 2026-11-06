//! #272 跨平台章节结构形态识别（纯函数面——只看编号形态，不看标题字面）。
//!
//! ## 背景（issue #272 实测）
//!
//! 黑龙江政采（batch1×5）与内蒙古政采（batch2×3）骨架用「第X章」
//! （康熙部首经 NFKC 归一后可认）；**询价类文件顶层是「第X部分」**
//! （第一部分 投标邀请 … 第七部分 响应文件格式），`re_chapter` 只认
//! 「章」→ 章上下文恒空 → 全部落「其他」（实测 71%）。另有纯数字顶层
//! 骨架（`1.适用范围`…）的潜在形态——batch2 询价件的供应商须知条款层
//! 即此形态（1..25 连续递增）。
//!
//! ## 形态家族与文档级方案仲裁
//!
//! ```text
//!   Zhang    「第X章」     （含阿拉伯数字；NFKC 归一部首）
//!   Part     「第X部分/篇」（询价/谈判/工程常见顶层）
//!   Numbered 「N.标题」    （裸数字顶层头——最长 +1 连续游程 ≥3）
//!   Uncertain 都没探到     → 响亮告警（不静默失败）
//! ```
//!
//! **仲裁规则（红线安全的构造性保证）**：一份文档只认一种顶层形态；
//! 有「章」骨架的文档，「部分/篇/数字头」行**不设章上下文**（与既有
//! 行为逐字节一致——黑龙江第六章合同内的「第三部分 合同专用条款」
//! 分部行不会冲掉章上下文）。「部分/数字」方案只在「全文无任何章标记」
//! 的文档激活——即今天章上下文恒空的文档，改动面=从无到有，不触碰
//! 既有路径。
//!
//! ## 防过拟合标注
//!
//! 形态家族基于 3 个平台来源（黑龙江政采 5 / 内蒙古政采 3 / 询价 1，
//! 另有吉林 docx 1 走 DOCX 侧），未验证其它省份。「第X部分」与
//! 「N.标题」是全国政采/询价文书的通用惯例形态（非省特异），但游程
//! 阈值（≥3）与长度上限（≤18 字）是本批样本的工程定标——下批语料
//! 若失效，先查这里。

use crate::{Block, BlockKind};
use serde::{Deserialize, Serialize};

/// 顶层骨架方案（文档级仲裁结果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TopScheme {
    /// 未识别到任何顶层骨架（响亮告警——不静默失败）
    #[default]
    Uncertain,
    /// 「第X章」骨架（含阿拉伯数字 + 康熙部首 NFKC 归一）
    Zhang,
    /// 「第X部分 / 第X篇」骨架（询价/谈判类顶层）
    Part,
    /// 「N.标题」裸数字顶层骨架（最长 +1 连续游程）
    Numbered,
}

/// 章标记（方案获胜形态的一个命中行）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterMark {
    /// 块号（extract 主循环按此对齐）
    pub idx: usize,
    /// 章号（1-based；解析失败为 0）
    pub no: u32,
    /// 原文（部首/空格保真——与 anchor 口径一致）
    pub raw: String,
}

/// 文档结构画像（识别面 + 告警面）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StructureProfile {
    pub scheme: TopScheme,
    /// 获胜方案的章标记（块序）。Zhang 方案含全部命中行——与 extract
    /// 既有「每个 ^第X章 行都设章」语义一致（含目录区与正文引用行，
    /// 不做剔除——剔除会改变既有分类面）；Part/Numbered 同语义。
    pub chapters: Vec<ChapterMark>,
    /// 响亮告警（Uncertain / 数字游程截断等识别不确定性）
    pub warnings: Vec<String>,
}

/// 章形态正则（章家族——与 extract.rs `re_chapter` 同口径 + NFKC）。
fn re_zhang() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^第\s*([0-9一二三四五六七八九十百]+)\s*章").unwrap()
    })
}

/// 部分形态正则（部分/篇家族）。
fn re_part() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^第\s*([0-9一二三四五六七八九十百]+)\s*(部分|篇)").unwrap()
    })
}

/// 裸数字顶层头形态：1-2 位数字 + 点/顿 + 2-18 个连续 CJK、无尾标点、
/// 无括号/数字/拉丁（条款句/字段行/列举条目都进不来）。
fn re_numbered_head() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^([0-9]{1,2})[.．、]\s*[一-龥][一-龥\s]{1,17}$").unwrap()
    })
}

/// 裸数字顶层头形态判定（段边界/章标记两用——extract 主循环的公开而）。
/// 与 [`detect_structure`] 的候选形态同源；只判形态，不判游程 membership。
pub fn is_numbered_head_form(s: &str) -> bool {
    re_numbered_head().is_match(s.trim())
}

/// 中文数字 → 数值（至百）。解析失败返回 None。
/// 输入字符域由调用方正则保证（[0-9一二三四五六七八九十百]）——filter_map
/// 丢弃域外字符，空串返回 None。
fn cn_num(s: &str) -> Option<u32> {
    let digits: Vec<u32> = s
        .chars()
        .filter_map(|c| match c {
            '一' | '1' => Some(1),
            '二' | '2' => Some(2),
            '三' | '3' => Some(3),
            '四' | '4' => Some(4),
            '五' | '5' => Some(5),
            '六' | '6' => Some(6),
            '七' | '7' => Some(7),
            '八' | '8' => Some(8),
            '九' | '9' => Some(9),
            '零' | '0' => Some(0),
            '十' => Some(10),
            '百' => Some(100),
            _ => None,
        })
        .collect();
    if digits.is_empty() {
        return None;
    }
    if digits.iter().any(|&d| d >= 10) {
        // 含十/百的复合形式（「二十三」「一百零五」）——按位权展开
        let mut total = 0u32;
        let mut unit = 1u32;
        for &d in digits.iter().rev() {
            match d {
                10 => unit = 10,
                100 => unit = 100,
                _ => total += d * unit,
            }
        }
        // 「十」开头省「一」：十=10、十二=12
        if digits[0] == 10 {
            total += 10 * unit_max(&digits);
        }
        return Some(total);
    }
    // 纯数字串（政采编号形「123」）
    let s2: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if !s2.is_empty() {
        return s2.parse().ok();
    }
    Some(digits.iter().sum())
}

fn unit_max(digits: &[u32]) -> u32 {
    // 「十」开头时补的位权 = 其后最大位权（十二 → 1；一百二十 → 10）
    let mut m = 1u32;
    for &d in digits.iter().skip(1) {
        if d == 10 {
            m = m.max(10);
        } else if d == 100 {
            m = m.max(100);
        }
    }
    m
}

fn nfkc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkc().collect()
}

/// 与 extract 主循环同口径的行可用性（TOC 行防御 + 块型过滤）。
fn usable_line(b: &Block) -> Option<&str> {
    if !matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine) {
        return None;
    }
    let t = b.text.trim();
    if t.is_empty() || t.matches('.').count() >= 6 || t.contains("……") {
        return None;
    }
    Some(t)
}

/// 收集某形态家族的全部命中（块序）。
fn collect_marks(blocks: &[Block], re: &regex::Regex) -> Vec<ChapterMark> {
    blocks
        .iter()
        .filter_map(|b| {
            let t = usable_line(b)?;
            let n = nfkc(t);
            let caps = re.captures(&n)?;
            Some(ChapterMark {
                idx: b.idx,
                no: cn_num(&caps[1]).unwrap_or(0),
                raw: t.to_string(),
            })
        })
        .collect()
}

/// 数字头候选里找最长 +1 连续游程（≥3 才算骨架；任意起点兼容——
/// 前置字段行可能打断前段，截断骨架从中间起号）。
fn longest_numbered_run(cands: &[ChapterMark]) -> Vec<ChapterMark> {
    let mut best: Vec<ChapterMark> = Vec::new();
    let mut cur: Vec<ChapterMark> = Vec::new();
    for c in cands {
        let continues = cur
            .last()
            .map(|last| c.no == last.no + 1 && c.idx > last.idx)
            .unwrap_or(false);
        if continues {
            cur.push(c.clone());
        } else {
            if cur.len() > best.len() {
                best = cur.clone();
            }
            cur.clear();
            cur.push(c.clone()); // 新种子（任意起点）
        }
    }
    if cur.len() > best.len() {
        best = cur;
    }
    if best.len() >= 3 {
        best
    } else {
        Vec::new()
    }
}

/// 枚举簇抑制：相邻同族标记间距 ≤2 块 = 行内列举（实测：询价件
/// 「7.询价文件构成」节里逐行列出第一部分..第七部分——末行会把后文
/// 章上下文钉死在枚举末项上），非章节边界。真实部分之间隔着正文
///（实测间距 ≥80 块）——密集邻判定不会误伤。仅用于 Part 族：
/// Zhang 族不动（红线）；Numbered 族的骨架节天然短，游程判定已够。
fn suppress_dense_cluster(marks: &[ChapterMark]) -> Vec<ChapterMark> {
    marks
        .iter()
        .enumerate()
        .filter(|(i, m)| {
            let prev_close = *i > 0 && m.idx.saturating_sub(marks[i - 1].idx) <= 2;
            let next_close = i + 1 < marks.len() && marks[i + 1].idx.saturating_sub(m.idx) <= 2;
            !(prev_close || next_close)
        })
        .map(|(_, m)| m.clone())
        .collect()
}

/// 结构形态识别（纯函数：只依赖块序列，无字面判定）。
pub fn detect_structure(blocks: &[Block]) -> StructureProfile {
    let mut warnings = Vec::new();

    // ── 方案仲裁：章 > 部分/篇 > 数字（一份文档只认一种顶层形态）──
    let zhang = collect_marks(blocks, re_zhang());
    if !zhang.is_empty() {
        return StructureProfile {
            scheme: TopScheme::Zhang,
            chapters: zhang,
            warnings,
        };
    }

    let part = suppress_dense_cluster(&collect_marks(blocks, re_part()));
    if !part.is_empty() {
        return StructureProfile {
            scheme: TopScheme::Part,
            chapters: part,
            warnings,
        };
    }

    let numbered_cands = collect_marks(blocks, re_numbered_head());
    if !numbered_cands.is_empty() {
        let run = longest_numbered_run(&numbered_cands);
        if !run.is_empty() {
            if run[0].no != 1 {
                warnings.push(format!(
                    "数字顶层骨架从 {} 开始而非 1（截断形态——前段可能被前置字段行打断，基于 3 个平台来源定标）",
                    run[0].no
                ));
            }
            return StructureProfile {
                scheme: TopScheme::Numbered,
                chapters: run,
                warnings,
            };
        }
        warnings.push(format!(
            "探到 {} 个裸数字头但未构成 ≥3 连续递增游程——不判数字骨架",
            numbered_cands.len()
        ));
    }

    warnings.push(
        "未识别到顶层章节骨架（无「第X章/第X部分/N.标题」形态）——\
         章节上下文为空，分类将依赖词面/表格/否决词典兜底（issue #272：不静默失败，响亮登记）"
            .to_string(),
    );
    StructureProfile {
        scheme: TopScheme::Uncertain,
        chapters: Vec::new(),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blk(idx: usize, text: &str) -> Block {
        Block {
            idx,
            kind: BlockKind::PdfLine,
            locator: format!("第1页 行{}", idx + 1),
            text: text.to_string(),
            cells: vec![],
        }
    }

    /// 黑龙江形态：章骨架（含部首 + 目录区 + 正文引用 FP）→ Zhang，全量命中。
    #[test]
    fn zhang_scheme_with_radicals_toc_and_fp() {
        let blocks = vec![
            blk(0, "⿊政采计划"),
            blk(1, "第⼀章 投标邀请"),       // 部首 ⼀
            blk(2, "第⼆章 投标⼈须知"),
            blk(3, "第三章 采购需求 .........9"), // TOC 行（点引导）→ 防御跳过
            blk(4, "正文条款"),
            blk(5, "第三章 采购需求"),
            blk(6, "第七章标准格式《联合体协议》并按要求"), // 正文引用 FP——语义同 extract
            blk(7, "第三部分 合同专⽤条款"),  // 章骨架文档里的部分行——不设上下文（Zhang 获胜）
        ];
        let p = detect_structure(&blocks);
        assert_eq!(p.scheme, TopScheme::Zhang);
        assert_eq!(p.chapters.len(), 4); // idx 1,2,5,6（FP 含；目录行被点防御剔除）
        assert!(p.chapters.iter().all(|c| c.raw.contains("章")));
        assert_eq!(p.chapters[0].no, 1); // ⼀ → 1（部首归一后解析）
        assert!(p.warnings.is_empty());
    }

    /// 询价形态：无任何「章」标记，「第X部分」骨架获胜。
    ///（部分之间隔正文——密集邻抑制要求 Δ>2，真实部分恒满足）
    #[test]
    fn part_scheme_wins_when_no_zhang() {
        let blocks = vec![
            blk(0, "询价通知书"),
            blk(1, "1. 项目编号：JL-2026-001"), // 数字头但含拉丁 → 非候选
            blk(2, "第一部分 投标邀请"),
            blk(3, "邀请正文"),
            blk(4, "更多邀请正文"),
            blk(5, "更多邀请正文"),
            blk(10, "第二部分 供应商须知"),
            blk(12, "1.适用范围"), // 部分骨架文档里的数字头——不设章上下文
            blk(13, "正文内容"),
            blk(20, "第三部分 资格审查"),
        ];
        let p = detect_structure(&blocks);
        assert_eq!(p.scheme, TopScheme::Part);
        assert_eq!(p.chapters.len(), 3);
        assert_eq!(p.chapters[2].no, 3);
    }

    /// 纯数字骨架：前置字段行/条款句干扰下，最长 +1 游程获胜。
    #[test]
    fn numbered_scheme_longest_run_wins() {
        let blocks = vec![
            blk(0, "1. 项目编号：X-1"),  // 拉丁 → 非候选
            blk(1, "1.满足规定；"),       // 尾标点 → 非候选
            blk(2, "2.供应商提供的资质和业绩等所有资料均须附在响应文件中"), // 超长 → 非候选
            blk(3, "1.适用范围"),
            blk(4, "正文"),
            blk(5, "2.说明"),
            blk(6, "3.供应商资格要求"),
            blk(7, "条款"),
            blk(8, "4.政府采购政策"),
        ];
        let p = detect_structure(&blocks);
        assert_eq!(p.scheme, TopScheme::Numbered);
        assert_eq!(p.chapters.len(), 4);
        assert_eq!(p.chapters[0].no, 1);
        assert_eq!(p.chapters[0].idx, 3);
        assert_eq!(p.chapters[3].no, 4);
    }

    /// 数字头不足以成骨架（<3 连游程）→ Uncertain + 响亮告警。
    #[test]
    fn uncertain_scheme_warns_loudly() {
        let blocks = vec![
            blk(0, "散乱文本"),
            blk(1, "1.适用范围"),
            blk(2, "正文"),
            blk(3, "3.供应商资格要求"), // 跳过 2 → 游程断
        ];
        let p = detect_structure(&blocks);
        assert_eq!(p.scheme, TopScheme::Uncertain);
        assert!(!p.warnings.is_empty());
        assert!(p
            .warnings
            .iter()
            .any(|w| w.contains("未识别到顶层章节骨架")));
    }

    /// 枚举簇抑制：正文里逐行列举「第一部分..第七部分」（询价文件构成
    /// 节实测形态）不是章节边界——密集邻（Δ≤2）剔除；真实部分保留。
    #[test]
    fn part_enumeration_cluster_suppressed() {
        let mut blocks = vec![
            blk(0, "询价文件正文"),
            blk(5, "第一部分 投标邀请"),
            blk(8, "邀请内容若干行"),
            blk(300, "第二部分 供应商须知"),
            // 「7.询价文件构成」节里的枚举簇（连续 7 行）
            blk(465, "第一部分 投标邀请"),
            blk(466, "第二部分 供应商须知"),
            blk(467, "第三部分 资格审查"),
            blk(468, "第四部分 采购需求"),
            blk(469, "第五部分 评标标准和评标方法"),
            blk(470, "第六部分 政府采购合同"),
            blk(471, "第七部分 响应文件格式"),
            blk(810, "第三部分 资格审查"),
        ];
        blocks.push(blk(900, "第四部分 采购需求"));
        let p = detect_structure(&blocks);
        assert_eq!(p.scheme, TopScheme::Part);
        let idxs: Vec<usize> = p.chapters.iter().map(|c| c.idx).collect();
        assert_eq!(idxs, vec![5, 300, 810, 900], "枚举簇（465-471）必须被剔除");
    }

    /// 中文数字解析：二十三 / 一百零五 / 十二。
    #[test]
    fn cn_num_forms() {
        assert_eq!(cn_num("一"), Some(1));
        assert_eq!(cn_num("十"), Some(10));
        assert_eq!(cn_num("十二"), Some(12));
        assert_eq!(cn_num("二十三"), Some(23));
        assert_eq!(cn_num("一百零五"), Some(105));
        assert_eq!(cn_num("7"), Some(7));
        assert_eq!(cn_num("x"), None);
    }
}
