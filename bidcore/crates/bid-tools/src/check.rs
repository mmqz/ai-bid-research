//! check.rs —— S-5 审核分片 + 全局一致性单列（issue #11 批次 v3 / V3-4）。
//!
//! 两类检查必须分开（S-5 规格的分柱）：
//! - 局部检查（占位符/引用/格式）：单章内自洽，零跨章依赖 → 按章分片可并行；
//! - 全局检查（金额/工期/术语/承诺冲突）：跨章一致性，必须单遍全量——但
//!   「全量扫」≠「全量存」：遍历时只抽「特征元组」(类型, 归一化值, 位置)，
//!   不保留原文。冲突检测的输入面只有特征集合，无任何章正文。
//!
//! 内存纪律（规格硬标准「500 章生成后峰值 < 200MB」的结构保证）：特征抽取
//! 是逐章的纯函数——章正文的扫描副本（全角归一串）在函数返回即弃；run_review
//! 长驻的只有特征集合（500 章量级序列化后 KB 序列，见
//! review_sharding.rs::feature_only_memory_discipline 的 < 100KB 断言）。分片
//! 并行不放大内存：每线程只攒自己片的发现与特征，无原文复制。
//!
//! 确定性纪律（R-15 可重放的分片面）：并行分片结果合并后统一按
//! (section_id, kind, message) 排序——分片认领顺序不进结果面，同输入同发现集，
//! 并行与串行逐字节全等（review_sharding.rs::determinism_parallel_vs_sequential）。
//!
//! 归一化口径（单一真源不复制）：全称/承诺值归一借用 [`bid_parse::norm_n1`]
//! （N1 = NFKC + 去空白——对齐/检索共用的既有规范）；金额归一到元整数字符串
//! （万 ×10000、剥逗号），工期归一到天（个月 ×30——S-5 规格声明的 30 天口径）。
//!
//! T-7 接线面：CheckFinding 的 category 口径对齐 review::Finding（"金额冲突"
//! "占位符残留"等——kind 枚举即六类口径：4 类全局 + 2 类局部，编排者裁决写死）。
//! 正式接线在后续批次：本模块零依赖 bid-domain 新实体；输入面 ChapterInput 与
//! 存储层解耦（章装配器的记录可直接映射进来，本批不引用 assembly）。
//!
//! 扫描器手写（词表 + 窗口状态机）：bid-tools 依赖面虽有 regex，但「全角/逗号/
//! 万·个月单位」混排的归一化抽值本质是逐字符状态机，手写更直白且零编译期正则。

use bid_parse::norm_n1;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

// ---------------------------------------------------------------- 类型面（T-7 接线口径）

/// 检查发现（T-7 接线面：category 口径对齐 review::Finding——"金额冲突"等，
/// 正式接线在后续批次；本结构自持序列化面，可直入审核闭环的发现流）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckFinding {
    /// 发现类别（六类之一：4 全局冲突 + 占位符/引用悬空 2 局部）
    pub kind: CheckKind,
    /// Blocker | Warning（占位符/引用悬空/四类冲突 = Blocker；格式偏差 = Warning）
    pub severity: CheckSeverity,
    /// 定位锚：局部发现 = 所属章；全局发现 = 少数派取值的首现位置（见 check_global）
    pub section_id: String,
    /// 含归一化值与冲突双方位置（可定位可复核——不携带原文摘录）
    pub message: String,
}

/// 发现类别。变体序 = 合并排序的 kind 序（确定性面的一部分，勿随意重排）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    /// 占位符残留（局部：TODO/待补充/【占位】词表命中）
    Placeholder,
    /// 引用悬空（局部：cited REQ/EV id 不在已知集）
    DanglingRef,
    /// 格式偏差（局部：字数与配额偏差 > 50%——Warning 面）
    FormatDeviation,
    /// 金额冲突（全局：同维度归一化金额不一致）
    AmountConflict,
    /// 工期冲突（全局：同维度归一化天数不一致）
    ScheduleConflict,
    /// 术语冲突（全局：同缩写不同全称）
    TermConflict,
    /// 承诺冲突（全局：同维度承诺不一致，如 7×24 vs 5×8）
    CommitmentConflict,
}

/// 严重度：Blocker（废标级——硬冲突/占位/悬空）| Warning（偏差级——格式面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckSeverity {
    Blocker,
    Warning,
}

/// 全局特征元组（S-5 规格原文形态：(类型, 归一化值, 位置)——本结构把
/// 「归一化值」细分为维度键 + 归一化值，让分组比对可按 (类型, 键) 落桶）。
/// 不保留原文是硬纪律：500 章的特征集合只有 KB 级（内存峰值的结构保证）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureTuple {
    /// Amount | Schedule | Term | Commitment
    pub feature_type: FeatureType,
    /// 归一化维度键（如 "投标保证金" / "工期" / 缩写本身 / "服务窗口"）
    pub key: String,
    /// 归一化值（金额统一到元整数字符串；天数统一整数；术语/承诺 N1 归一文本）
    pub value: String,
    /// 位置（章 section_id——S-5 规格的 (金额, 3500000, SEC-2.1) 第三元）
    pub section_id: String,
}

/// 特征类型。变体序即分组遍历序（BTreeMap 键——确定性面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureType {
    Amount,
    Schedule,
    Term,
    Commitment,
}

/// 单章输入（局部检查面——与存储层解耦：章装配器的记录可直接映射进来，
/// 本批不引用 assembly；借用式字段让调用方零拷贝喂入）。
#[derive(Debug, Clone, Copy)]
pub struct ChapterInput<'a> {
    pub section_id: &'a str,
    pub content: &'a str,
    /// 本章声称引用的 REQ-xxxx / EV-xxxx id（悬空判定面）
    pub citations: &'a [String],
    /// 字数配额；None = 不做格式偏差检查（配额未定的章）
    pub target_chars: Option<u32>,
}

/// 审核总出口：发现集 + 特征规模证据（内存纪律的可观测面）。
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewOutcome {
    pub findings: Vec<CheckFinding>,
    /// 抽取的特征元组总数（去重后——500 章量级应为数百，非正文量级）
    pub feature_count: usize,
    /// 实际分片数（chunks(25) 产出；线程并发上限 8，见 run_review）
    pub peak_shards: usize,
}

// ---------------------------------------------------------------- 词表（显式清单，非模式）

/// 占位符词表：大小写敏感的 TODO（代码/模板残留）+ 中文占位词。
/// 显式清单口径——防误报优先：宁可漏一个新词，不可把正文当占位符；
/// 词表扩展（英文占位变体等）归 T-7 接线批次，不在本批自由发挥。
const PLACEHOLDER_MARKERS: [&str; 3] = ["TODO", "待补充", "【占位】"];

/// 金额维度词（S-5 规格列举：投标保证金|招标控制价|项目预算|服务费——「等」
/// 的扩展面归 T-7 词表批次，本批收窄到列举四词，误报面可控）。
const AMOUNT_DIMS: [&str; 4] = ["投标保证金", "招标控制价", "项目预算", "服务费"];

/// 工期维度词（同上——规格列举四词收窄）。
const SCHEDULE_DIMS: [&str; 4] = ["工期", "交付周期", "实施周期", "服务期"];

/// 承诺服务窗口词表（×/x 同形归一后匹配；全天候是独立归一值——与 7x24 的
/// 语义等价归并涉及猜测，不猜：口径留给 T-7 词表扩展）。
const WINDOW_PATTERNS: [&str; 3] = ["7x24", "5x8", "全天候"];

/// 内部特征集合形态：(类型, 键, 归一化值, 位置) 四元组——BTreeSet 去重 +
/// 全序（extract_features 输出天然有序确定）。
type FeatSet = BTreeSet<(FeatureType, String, String, String)>;

// ---------------------------------------------------------------- 局部检查（可并行的原因：零跨章依赖）

/// 局部检查（单章内——可并行的原因：只读本章与全局只读的 known_refs，
/// 零跨章依赖、零共享可变态）。输出序确定：(kind, message)——与 run_review
/// 的合并排序口径一致，单章直查与批查结果全等可被逐字节断言。
pub fn check_local(ch: &ChapterInput, known_refs: &HashSet<String>) -> Vec<CheckFinding> {
    let mut out = Vec::new();
    // 占位符残留：词表命中即 Blocker（生成未完成的硬证据）
    for marker in PLACEHOLDER_MARKERS {
        if ch.content.contains(marker) {
            out.push(CheckFinding {
                kind: CheckKind::Placeholder,
                severity: CheckSeverity::Blocker,
                section_id: ch.section_id.to_string(),
                message: format!("占位符残留：{marker}（词表口径：TODO/待补充/【占位】）"),
            });
        }
    }
    // 引用悬空：REQ-/EV- 前缀 id 必须在 known_refs；前缀外的自由文本引用
    // 不在本面（词表外不猜）。BTreeSet 去重排序——同 id 多次引用只报一条。
    let dangling: BTreeSet<&str> = ch
        .citations
        .iter()
        .filter(|c| (c.starts_with("REQ-") || c.starts_with("EV-")) && !known_refs.contains(c.as_str()))
        .map(|c| c.as_str())
        .collect();
    for id in dangling {
        out.push(CheckFinding {
            kind: CheckKind::DanglingRef,
            severity: CheckSeverity::Blocker,
            section_id: ch.section_id.to_string(),
            message: format!("引用悬空：{id} 不在已知需求/证据集（known_refs）"),
        });
    }
    // 格式偏差：字数与配额偏差 > 50% → Warning（整数算术无浮点；target=0
    // 视为配额未定——不查，避免除零面）
    if let Some(target) = ch.target_chars {
        if target > 0 {
            let actual = ch.content.chars().count();
            let dev = actual.abs_diff(target as usize);
            if dev * 100 > target as usize * 50 {
                out.push(CheckFinding {
                    kind: CheckKind::FormatDeviation,
                    severity: CheckSeverity::Warning,
                    section_id: ch.section_id.to_string(),
                    message: format!(
                        "格式偏差：正文字数 {actual} 偏离配额 {target}（偏差 {}%，阈值 50%）",
                        dev * 100 / target as usize
                    ),
                });
            }
        }
    }
    out.sort_by_key(|f| (f.kind, f.message.clone()));
    out
}

// ---------------------------------------------------------------- 特征抽取（全局第一遍：只抽特征不留原文）

/// 特征抽取（全局检查的第一遍——只抽特征不留原文：正文的扫描副本是本函数
/// 的局部量，返回的只有特征元组）。输出经 BTreeSet 去重 + 全序——同章同维度
/// 同值的重复提及只记一条（特征集合的最小形态）。
pub fn extract_features(ch: &ChapterInput) -> Vec<FeatureTuple> {
    let scan: Vec<char> = ch.content.chars().map(hw).collect();
    let mut set: FeatSet = BTreeSet::new();
    scan_amount(ch.section_id, &scan, &mut set);
    scan_schedule(ch.section_id, &scan, &mut set);
    scan_terms(ch.section_id, &scan, &mut set);
    scan_commitments(ch.section_id, &scan, &mut set);
    set.into_iter()
        .map(|(feature_type, key, value, section_id)| FeatureTuple { feature_type, key, value, section_id })
        .collect()
}

/// 全角→半角映射（扫描面）：全角 ASCII 块（U+FF01..=U+FF5E）平移 0xFEE0，
/// 乘号 × 归一为 x（7×24 与 7x24 同形）。中文标点（。等）保留原样作句界。
fn hw(c: char) -> char {
    let u = c as u32;
    let mapped = match u {
        0xFF01..=0xFF5E => u - 0xFEE0, // 全角 ASCII → 半角（含全角数字/逗号/括号）
        0x00D7 => 0x78,                // × → x（服务承诺同形归一）
        _ => u,
    };
    char::from_u32(mapped).unwrap_or(c) // 块内映射必落合法码点——unwrap_or 仅作边界保险
}

/// char 级朴素子串查找（from 起始）——扫描器专用，确定性直给。
fn find_sub(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    let mut i = from;
    while i <= last {
        if hay[i..].starts_with(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 句内窗口：从 from 起到句界（。；;！!？?换行）或末尾，另设 48 字符护栏——
/// 真实标书里「维度词 → 数字」几乎同句紧邻，护栏外视为不相关句段。
fn sentence_window(scan: &[char], from: usize) -> usize {
    let limit = (from + 48).min(scan.len());
    let mut i = from;
    while i < limit {
        if matches!(scan[i], '。' | '；' | ';' | '！' | '!' | '？' | '?' | '\n' | '\r') {
            break;
        }
        i += 1;
    }
    i
}

/// 跳过空白（数字与单位之间允许空白：如「50 万元」「60 天」）。
fn skip_ws(scan: &[char], mut i: usize, end: usize) -> usize {
    while i < end && scan[i].is_whitespace() {
        i += 1;
    }
    i
}

/// 数字记号前的首个非空白字符（日期/序数守卫用：紧邻「月/年/第」的数字
/// 视为日期或序数成分——「3月1日」「第12个月」不当金额/工期值）。
fn prev_char_nonspace(scan: &[char], before: usize) -> Option<char> {
    let mut i = before;
    while i > 0 {
        i -= 1;
        if !scan[i].is_whitespace() {
            return Some(scan[i]);
        }
    }
    None
}

/// 数字记号解析：`[0-9]+（,[0-9]+)*（.[0-9]{1,8})?`——返回（剥逗号数字串,
/// 结束下标）。小数截 8 位（金额/工期面足够；超长小数视为噪声面跳过不解析）。
fn parse_number(scan: &[char], start: usize, end: usize) -> Option<(String, usize)> {
    let mut s = String::new();
    let mut i = start;
    while i < end && scan[i].is_ascii_digit() {
        s.push(scan[i]);
        i += 1;
    }
    if s.is_empty() {
        return None;
    }
    while i + 1 < end && scan[i] == ',' && scan[i + 1].is_ascii_digit() {
        i += 1;
        while i < end && scan[i].is_ascii_digit() {
            s.push(scan[i]);
            i += 1;
        }
    }
    if i + 1 < end && scan[i] == '.' && scan[i + 1].is_ascii_digit() {
        s.push('.');
        i += 1;
        let mut taken = 0;
        while i < end && scan[i].is_ascii_digit() && taken < 8 {
            s.push(scan[i]);
            i += 1;
            taken += 1;
        }
        while i < end && scan[i].is_ascii_digit() {
            i += 1; // 超 8 位的小数尾部：跳过（不进值面）
        }
    }
    Some((s, i))
}

/// 「数字串 × 倍率」→ 整数（纯整数算术，无浮点——确定性面；溢出/坏串 → None）。
/// mult：万 = 10000，个月 = 30（S-5 规格的 30 天口径），个 = 1。
fn scale_to_int(num: &str, mult: u128) -> Option<i64> {
    let (ip, fp) = match num.split_once('.') {
        Some((a, b)) => (a, b),
        None => (num, ""),
    };
    let int_v: u128 = ip.parse().ok()?;
    let scale = 10u128.pow(fp.len() as u32);
    let frac_v: u128 = if fp.is_empty() { 0 } else { fp.parse().ok()? };
    // 定点算术：units = num × 10^小数位，再乘倍率后除回——全程 u128 checked
    let total = int_v.checked_mul(scale)?.checked_add(frac_v)?.checked_mul(mult)?;
    let v = total / scale;
    if v <= i64::MAX as u128 {
        Some(v as i64)
    } else {
        None
    }
}

/// 金额抽取：维度词 → 句内窗口首个合规数字 → 归一化到元整数字符串。
/// 口径：万 ×10000、剥逗号、全角数字/逗号先归半角；「万/元/圆」是单位锚，
/// 裸数字（无锚）需整数部分 ≥2 位（防「附件2.3」「详见第3章」类误报）；
/// 数字后紧跟 % 视为比率面跳过；紧邻「月/年/第」的数字视为日期/序数跳过。
/// 每处维度词只取首个合规数字（后续数字属别的语义面）。
fn scan_amount(section: &str, scan: &[char], out: &mut FeatSet) {
    for dim in AMOUNT_DIMS {
        let dim_cs: Vec<char> = dim.chars().collect();
        let mut from = 0;
        while let Some(pos) = find_sub(scan, &dim_cs, from) {
            let word_end = pos + dim_cs.len();
            let win_end = sentence_window(scan, word_end);
            let mut k = word_end;
            while k < win_end {
                if !scan[k].is_ascii_digit() {
                    k += 1;
                    continue;
                }
                let Some((num, next)) = parse_number(scan, k, win_end) else {
                    break;
                };
                // 日期/序数守卫：数字前紧邻「月/年/第」→ 非金额面
                if matches!(prev_char_nonspace(scan, k), Some('月') | Some('年') | Some('第')) {
                    k = next;
                    continue;
                }
                let u = skip_ws(scan, next, win_end);
                let pct = u < win_end && scan[u] == '%';
                let has_wan = !pct && u < win_end && scan[u] == '万';
                let after_wan = if has_wan { skip_ws(scan, u + 1, win_end) } else { u };
                let has_yuan = after_wan < win_end && matches!(scan[after_wan], '元' | '圆');
                let int_digits = num.find('.').unwrap_or(num.len());
                if !pct && (has_wan || has_yuan || int_digits >= 2) {
                    if let Some(yuan) = scale_to_int(&num, if has_wan { 10_000 } else { 1 }) {
                        out.insert((FeatureType::Amount, dim.to_string(), yuan.to_string(), section.to_string()));
                    }
                    break;
                }
                k = next; // 该数字无单位锚——跳过，继续找窗口内下一个数字
            }
            from = word_end;
        }
    }
}

/// 工期抽取：维度词 → 句内窗口首个带时间单位的数字 → 归一化到天
/// （个月 ×30——S-5 规格声明的 30 天口径；天/日 ×1）。无单位数字跳过
/// （「工期为2026年」的年份不是工期值）；日期/序数守卫同金额面。
fn scan_schedule(section: &str, scan: &[char], out: &mut FeatSet) {
    for dim in SCHEDULE_DIMS {
        let dim_cs: Vec<char> = dim.chars().collect();
        let mut from = 0;
        while let Some(pos) = find_sub(scan, &dim_cs, from) {
            let word_end = pos + dim_cs.len();
            let win_end = sentence_window(scan, word_end);
            let mut k = word_end;
            while k < win_end {
                if !scan[k].is_ascii_digit() {
                    k += 1;
                    continue;
                }
                let Some((num, next)) = parse_number(scan, k, win_end) else {
                    break;
                };
                if matches!(prev_char_nonspace(scan, k), Some('月') | Some('年') | Some('第')) {
                    k = next;
                    continue;
                }
                let u = skip_ws(scan, next, win_end);
                let days = if u + 1 < win_end && scan[u] == '个' && scan[u + 1] == '月' {
                    scale_to_int(&num, 30) // 个月 → 天（30 天口径）
                } else if u < win_end && (scan[u] == '天' || scan[u] == '日') {
                    scale_to_int(&num, 1)
                } else {
                    None // 无时间单位（年份/小节号等）——跳过继续找
                };
                if let Some(d) = days {
                    out.insert((FeatureType::Schedule, dim.to_string(), d.to_string(), section.to_string()));
                    break;
                }
                k = next;
            }
            from = word_end;
        }
    }
}

/// 术语抽取：「缩写（全称）」模式——2-10 个大写字母/数字紧邻开括号（全角/半角
/// 均归半角后匹配），闭括号内的全称经 N1 归一（borrow bid-parse::norm_n1——
/// 单一真源不复制口径）。前界守卫：紧邻 ASCII 字母数字或「-./」的记号视为
/// 更长标记的一部分（REQ-0001 的数字段不是缩写）。
fn scan_terms(section: &str, scan: &[char], out: &mut FeatSet) {
    let n = scan.len();
    let mut i = 0;
    while i < n {
        if !(scan[i].is_ascii_uppercase() || scan[i].is_ascii_digit()) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && (scan[i].is_ascii_uppercase() || scan[i].is_ascii_digit()) {
            i += 1;
        }
        let run_len = i - start;
        let prev_ok = start == 0
            || !(scan[start - 1].is_ascii_alphanumeric() || matches!(scan[start - 1], '-' | '/' | '.'));
        if run_len >= 2 && run_len <= 10 && prev_ok && i < n && scan[i] == '(' {
            if let Some(close) = scan[i + 1..].iter().position(|&c| c == ')') {
                let inner: String = scan[i + 1..i + 1 + close].iter().collect();
                let norm = norm_n1(&inner); // N1：NFKC + 去空白（单一真源）
                if !norm.is_empty() {
                    let abbr: String = scan[start..i].iter().collect();
                    out.insert((FeatureType::Term, abbr, norm, section.to_string()));
                }
                i += close + 2; // 跳过闭括号
                continue;
            }
        }
        // 不成术语的记号：i 已越过 run，继续扫描
    }
}

/// 承诺抽取：服务窗口（7x24/5x8/全天候——× 与 x 同形归一）+ 响应时间
/// （「响应时间 N 小时」→ 归一值 "N小时"；分钟面不在本批口径，跳过不猜）。
fn scan_commitments(section: &str, scan: &[char], out: &mut FeatSet) {
    for pat in WINDOW_PATTERNS {
        let pcs: Vec<char> = pat.chars().collect();
        if find_sub(scan, &pcs, 0).is_some() {
            out.insert((FeatureType::Commitment, "服务窗口".to_string(), pat.to_string(), section.to_string()));
        }
    }
    let rcs: Vec<char> = "响应时间".chars().collect();
    let mut from = 0;
    while let Some(pos) = find_sub(scan, &rcs, from) {
        let word_end = pos + rcs.len();
        let win_end = sentence_window(scan, word_end);
        let mut k = word_end;
        while k < win_end {
            if !scan[k].is_ascii_digit() {
                k += 1;
                continue;
            }
            let Some((num, next)) = parse_number(scan, k, win_end) else {
                break;
            };
            let u = skip_ws(scan, next, win_end);
            if u + 1 < win_end && scan[u] == '小' && scan[u + 1] == '时' {
                if let Some(h) = scale_to_int(&num, 1) {
                    out.insert((FeatureType::Commitment, "响应时间".to_string(), format!("{h}小时"), section.to_string()));
                }
                break;
            }
            k = next; // 非小时单位（分钟等）——跳过继续找
        }
        from = word_end;
    }
}

// ---------------------------------------------------------------- 全局冲突检测（第二遍：特征集合上的分组比对）

/// 全局冲突检测（第二遍——特征集合上的分组比对，输入只有特征无原文：这就是
/// 全局检查不炸内存的结构保证）。按 (feature_type, key) 分组；组内归一化值
/// 去重后 >1 种 → 冲突 Blocker。message 列出全部取值 + 各自首现位置
/// （字典序最小），如「金额冲突：投标保证金 在 SEC-2.1=3500000 与
/// SEC-5.2=3800000」；finding 锚点 = 少数派取值的首现位置（出现位置数最少
/// 的取值——500 章一致 + 1 章偏离的场景下锚点即偏离章，复核直指异常面；
/// 平局取 (位置, 值) 字典序，确定性闭合）。
pub fn check_global(features: &[FeatureTuple]) -> Vec<CheckFinding> {
    // 分组面：BTreeMap——(类型, 维度键) → (归一化值 → 位置集合)。借用键聚合，
    // 不克隆特征本体（比对面零复制）。
    let mut groups: BTreeMap<(FeatureType, &str), BTreeMap<&str, BTreeSet<&str>>> = BTreeMap::new();
    for f in features {
        groups
            .entry((f.feature_type, f.key.as_str()))
            .or_default()
            .entry(f.value.as_str())
            .or_default()
            .insert(f.section_id.as_str());
    }
    let mut out = Vec::new();
    for ((ft, key), by_value) in groups {
        if by_value.len() < 2 {
            continue; // 单一取值 = 一致——不报（只出现一次的值不是冲突）
        }
        // 每个取值取首现位置（字典序最小），按 (首现位置, 值) 排序列举
        let mut entries: Vec<(&str, &str, usize)> = by_value
            .iter()
            .map(|(v, secs)| (secs.iter().next().copied().unwrap_or(""), *v, secs.len()))
            .collect();
        entries.sort();
        let listed = entries.iter().map(|(pos, v, _)| format!("{pos}={v}")).collect::<Vec<_>>().join(" 与 ");
        // 锚点 = 少数派：(位置数, 首现位置, 值) 全序最小
        let anchor = entries
            .iter()
            .min_by_key(|&&(pos, v, n)| (n, pos, v))
            .map(|(pos, _, _)| *pos)
            .unwrap_or("");
        let (label, kind) = match ft {
            FeatureType::Amount => ("金额冲突", CheckKind::AmountConflict),
            FeatureType::Schedule => ("工期冲突", CheckKind::ScheduleConflict),
            FeatureType::Term => ("术语冲突", CheckKind::TermConflict),
            FeatureType::Commitment => ("承诺冲突", CheckKind::CommitmentConflict),
        };
        out.push(CheckFinding {
            kind,
            severity: CheckSeverity::Blocker,
            section_id: anchor.to_string(),
            message: format!("{label}：{key} 在 {listed}"),
        });
    }
    out
}

// ---------------------------------------------------------------- 分片编排

/// 分片编排（S-5 的两柱合流）：
/// - 局部检查 + 特征抽取按章分片并行（chunks(25) 一片，std::thread::scope
///   借用安全 + join 语义；线程并发上限 8——分片多于 8 时线程从原子计数器
///   认领下一片，**不 spawn 无界线程**：线程数与章数解耦，OS 调度压力有上界，
///   500 章与 5000 章的线程面恒定）；
/// - 全局检查单遍两阶段：并行遍历中全量抽特征（不留原文）→ 特征集合上
///   check_global 分组比对。
///
/// 确定性纪律：分片结果按片号归位合并（mpsc 收集 + 片号排序——认领顺序不进
/// 结果面），最终统一按 (section_id, kind, message) 排序：同输入同发现集，
/// 并行与串行逐字节全等（R-15 可重放）。
pub fn run_review(chapters: &[ChapterInput], known_refs: &HashSet<String>) -> ReviewOutcome {
    const SHARD_SIZE: usize = 25;
    const MAX_THREADS: usize = 8;
    let shards: Vec<&[ChapterInput]> = chapters.chunks(SHARD_SIZE).collect();
    let shard_count = shards.len();
    let thread_count = shard_count.min(MAX_THREADS);
    let next = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(usize, Vec<CheckFinding>, Vec<FeatureTuple>)>();
    std::thread::scope(|s| {
        for _ in 0..thread_count {
            let tx = tx.clone();
            let next = &next;
            let shards = &shards;
            s.spawn(move || loop {
                // Relaxed 即可：计数器只做任务分发，无其他共享内存依赖它
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= shard_count {
                    break;
                }
                let mut fs = Vec::new();
                let mut feats = Vec::new();
                for ch in shards[i] {
                    fs.extend(check_local(ch, known_refs));
                    feats.extend(extract_features(ch));
                }
                if tx.send((i, fs, feats)).is_err() {
                    break; // 接收端已弃——唯一失败面，无善后动作
                }
            });
        }
    });
    drop(tx); // 释放主句柄，into_iter 才能自然终止
    let mut collected: Vec<(usize, Vec<CheckFinding>, Vec<FeatureTuple>)> = rx.into_iter().collect();
    collected.sort_by_key(|(i, _, _)| *i); // 片号归位——与串行章序一致
    let mut findings = Vec::new();
    let mut features = Vec::new();
    let mut feature_count = 0usize;
    for (_, fs, feats) in collected {
        findings.extend(fs);
        feature_count += feats.len();
        features.extend(feats);
    }
    findings.extend(check_global(&features));
    findings.sort_by(|a, b| {
        (a.section_id.as_str(), a.kind, a.message.as_str()).cmp(&(b.section_id.as_str(), b.kind, b.message.as_str()))
    });
    ReviewOutcome { findings, feature_count, peak_shards: shard_count }
}
