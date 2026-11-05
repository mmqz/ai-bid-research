//! outline.rs —— T-5 OutlineNode 章节树（issue #11 批次 v3 / V3-1，百万字分片的骨架）。
//!
//! 定位：500 章级标书的调度根——没有章节树，「每章 2,000 字 × 500 次 per-section
//! 调度」无从谈起（S-1/S-2/S-6/S-8 全部依赖本实体）。
//!
//! 状态词表（**xref_check C7 强制口径**，domain-model §1 BidSection）：
//! `PENDING → OUTLINING → DRAFTING → DRAFTED → VALIDATING → REVISING → ACCEPTED`；
//! `dirty` 是**可叠加脏标记**（非状态——上游 content_version 变化时打标，R-05）。
//! （注：issue #11 批次 v3 评论的 5 态草图与 domain-model 词表不一致，本实现
//! 取 domain-model——它是 xref 的机器检查真源。）
//!
//! 树不变量（[`DomainStore::set_outline`] 的验收面，违反即 E_UNPROCESSABLE）：
//! 1. id 唯一；2. parent 引用存在（无孤儿）；3. 无环（父链有限）；4. 根 level=1；
//! 5. 子 level = 父 level + 1（无跳级）；6. level ∈ 1..=4；7. order 全局唯一；
//! 8. 每节点绑定 ≥1 条 REQ（T-5 验收：分片的最小调度单元必须有活干）；
//! 9. 父节点 requirement_ids == 子节点并集（反范式聚合的一致性——查询面免递归，
//!    重建面由 set_outline 把关）。

use crate::{DomainStore, ProjectStatus, Requirement, ToolError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 章节最大层级（1..=4——T-5 规格「1-3（可到 4）」）。
pub const MAX_LEVEL: u8 = 4;

// ---------------------------------------------------------------- 状态机
/// 章节状态（domain-model §1 词表——xref C7 真源；ACCEPTED 后需显式 reopen，R-06）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SectionStatus {
    Pending,
    Outlining,
    Drafting,
    /// 生成失败（重试耗尽）——#68 阶段 2 的标记态：批量组装对单章的
    /// 「已知失败」显式落域（不阻塞其他章）；可经 Drafting 重试（续跑/
    /// 人工修复后重新起草）。从 Drafting 进入；不与 Drafted 直通。
    Failed,
    Drafted,
    Validating,
    Revising,
    Accepted,
}

impl SectionStatus {
    /// 合法前向链（domain-model §1 的线性推进）。
    pub fn next_allowed(&self) -> &'static [SectionStatus] {
        use SectionStatus::*;
        match self {
            Pending => &[Outlining],
            Outlining => &[Drafting],
            // 生成成功 → Drafted；重试耗尽 → Failed（#68 阶段 2——失败是
            // 一等结果，显式建模优于「停在 DRAFTING 靠调用方猜」）
            Drafting => &[Drafted, Failed],
            // 失败章的重试入口（人工修复/续跑重起草——不回退到 Pending）
            Failed => &[Drafting],
            Drafted => &[Validating],
            // 校验零发现直通验收（免空转 REVISING）；有发现走修订
            Validating => &[Revising, Accepted],
            Revising => &[Accepted],
            Accepted => &[], // 终态——修改需显式 reopen（R-06 幂等保护）
        }
    }

    pub fn as_str(&self) -> &'static str {
        use SectionStatus::*;
        match self {
            Pending => "PENDING",
            Outlining => "OUTLINING",
            Drafting => "DRAFTING",
            Failed => "FAILED",
            Drafted => "DRAFTED",
            Validating => "VALIDATING",
            Revising => "REVISING",
            Accepted => "ACCEPTED",
        }
    }
}

// ---------------------------------------------------------------- 实体
/// 章节树节点（T-5 规格 struct 的 Rust 落地——字段对齐 issue #11 批次 v3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineNode {
    pub id: String,                  // SEC-xxxx（store 单序列签发）
    pub parent_id: Option<String>,   // None = 根
    pub title: String,
    pub level: u8,                   // 1..=4
    pub order: u32,                  // 文档序（全局唯一）
    pub requirement_ids: Vec<String>, // 本章响应的 REQ（叶 = 分片块；父 = 子并集聚合）
    pub target_chars: u32,           // 字数配额（父 = 子配额和）
    pub status: SectionStatus,
    /// 脏标记（可叠加——上游变更打标，非状态机成员；R-05 影响面分析的输入）
    #[serde(default)]
    pub dirty: bool,
    /// #59 阶段 1（draft_response_section）：章正文（**仅叶节点**——结构节点
    /// 是调度单元不承载内容；`None` = 未起草）。与 S-2 ChapterAssembler 的
    /// `chapters/{id}.json` 双真源：内存面供 MCP 即时读，磁盘面供断点续跑
    /// 与导出（两处由同一提交路径同步写——见 bid-tools service::draft_section）。
    #[serde(default)]
    pub content: Option<String>,
    /// 正文版本（1 起；重提交不同内容 +1——CAS 写入的 attempt 账面，同内容
    /// no-op 不推进）。与 ChapterRecord.attempt 同源同值。
    #[serde(default)]
    pub content_attempt: u32,
}

// ---------------------------------------------------------------- 纯逻辑面
/// 建纲目标总字数的**单一真源默认值**（issue #253 第一段：三层默认值统一）。
///
/// 推导：实测产物密度 844 字符/页（97 页 PDF / 81,850 字符反推），用户硬指标
/// 「总篇幅 ≥ 1000 页」→ 目标总字数下限 = 1000 × 844 = 844,000。
/// 取 **850,000**（≈ 1007 页）：落在 issue 给出的 844,000～866,000 论证区间内，
/// 对下限留 ~0.7% 余量（密度为单次实测，存在样本误差），且为整五万好记的数。
/// 取代旧值 1,000,000（超区间上限 15%，会连带抬高每章配额，与 #253 配额
/// 按评分权重重构的方向冲突）。
pub const DEFAULT_TARGET_TOTAL_CHARS: u32 = 850_000;

/// 派生参数：`max_reqs_per_leaf` 决定分片粒度（验收口径：12,000 REQ / 500 章 ≈ 24）。
#[derive(Debug, Clone, Copy)]
pub struct OutlineSpec {
    pub target_total_chars: u32,
    pub max_reqs_per_leaf: usize,
}

impl Default for OutlineSpec {
    fn default() -> Self {
        // 850,000 字 ≈ 1007 页（844 字符/页）——满足 1000 页硬指标；单一真源见
        // [`DEFAULT_TARGET_TOTAL_CHARS`]（issue #253：MCP/schema 层引用而非重写）
        OutlineSpec {
            target_total_chars: DEFAULT_TARGET_TOTAL_CHARS,
            max_reqs_per_leaf: 24,
        }
    }
}

/// 叶节点配额下限（防零配额退化——低于此值的章节失去独立调度意义）。
const MIN_LEAF_CHARS: u32 = 500;

/// 兜底组名（章节与主题都推不出的「其他」条款——叶标题退化为条款号区间，
/// 不再裸「（第N块）」）。
const OTHER_FALLBACK_GROUP: &str = "其他条款";

// ------------------------------------------------ 其他类语义分组（issue #100 任务 1）

/// 「其他」类条款的主题规则表（关键词 → 主题；命中数最多者优先，并列取
/// 表序——**具体规则在前，通用规则殿后**）。
///
/// 依据（真实标书语料口径，非主观编造）：
/// - `fixtures/synth-tender` 31 条「其他」条款实测分布（e2e 03_requirements.json）：
///   项目概况（1.1/1.3）/资格声明（1.4.7）/获取（1.5）/开标（1.6/1.7）/
///   须知总则（3.1.x）/报价（3.4.1）/保证金（3.5.2）/合同授予（3.8.x）/
///   纪律（3.9.2）/合同条款（6.1-6.4）/格式（7.x）/附件（8.x）/补遗（4 条）；
/// - `real-corpus` 政采 GT 类目词：补遗变更/项目标识/时间参数/商务参数；
/// - issue #100 任务 1 starter 表：工期/质量/资质/报价/技术/服务/安全/基本情况。
const THEME_RULES: &[(&[&str], &str)] = &[
    (&["项目名称", "项目编号", "采购人", "招标人", "招标代理", "联系方式", "质疑"], "项目基本情况"),
    (&["工期", "进度", "交付", "完成时间", "日历天", "竣工"], "项目工期要求"),
    (&["质量标准", "验收", "合格", "质量保证"], "质量标准要求"),
    (&["资质", "资格", "证书", "许可", "违法记录"], "投标人资格要求"),
    (&["获取", "购买", "下载", "发售"], "招标文件获取"),
    (&["投标截止", "开标时间", "递交", "截标", "顺延"], "投标与开标安排"),
    (&["报价", "价格", "费用", "金额", "预算", "包干", "总价"], "报价要求"),
    (&["保证金", "保函"], "投标保证金"),
    (&["适用范围", "资金来源", "总则"], "须知总则"),
    (&["签订", "签约", "中标通知书", "履约"], "合同授予与签订"),
    (&["违约", "争议", "付款", "预付款", "结算", "合同价格"], "合同条款响应"),
    (&["格式", "样例", "签署", "盖章", "装订", "份数"], "投标文件格式"),
    (&["附件", "图纸", "明细表", "工程量清单"], "附件与图纸说明"),
    (&["变更", "澄清", "补遗", "答疑", "踏勘", "调整", "更正"], "补遗与澄清响应"),
    (&["纪律", "监督", "违纪", "投诉"], "纪律与监督"),
    // —— 通用主题（issue starter 表原文；具体规则未命中时才轮到它们）——
    (&["技术", "方案", "参数", "规格", "性能"], "技术方案要求"),
    (&["服务", "售后", "培训", "质保", "响应时间"], "服务与售后要求"),
    (&["安全", "保密", "防护", "应急"], "安全与保密要求"),
];

/// 从条款内容推断主题（规则版——issue #100 路线 A，不依赖 LLM）。
///
/// 输入面：全部锚点摘录 + 条款号（[`Requirement`] 无正文字段——锚点摘录
/// 即条款原文）。命中关键词最多的主题胜出；并列取 [`THEME_RULES`] 表序
/// （具体在前）；零命中 → `None`（调用方走兜底）。
pub fn infer_theme(reqs: &[&Requirement]) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for (keywords, theme) in THEME_RULES {
        let mut hits = 0usize;
        for r in reqs {
            let hay = req_haystack(r);
            for kw in *keywords {
                if hay.contains(kw) {
                    hits += 1;
                }
            }
        }
        if hits > 0 && best.map(|(h, _)| hits > h).unwrap_or(true) {
            best = Some((hits, theme));
        }
    }
    best.map(|(_, t)| t.to_string())
}

/// 主题匹配的文本面（锚点摘录全量 + 条款号拼接——一次构建多次复用）。
fn req_haystack(r: &Requirement) -> String {
    let mut s = String::new();
    for a in &r.anchors {
        s.push_str(&a.excerpt);
        s.push(' ');
    }
    if let Some(c) = &r.clause_no {
        s.push_str(c);
    }
    s
}

/// 锚点定位里的招标文件章节（issue #100 路线 B——「第X章 …」段）。
///
/// locator 是解析层固化的章节路径（如「第一章 招标公告 > 一、项目概况」），
/// 按「 > 」分段扫描，首个含「第X章」的段即组名——**招标文件自己的章节名**
/// 比关键词主题更忠实（验收线「标题与招标文件章节对应」）。补遗等无章节
/// 结构的锚（「第1页 行6」）返回 `None` → 走 [`infer_theme`]。
fn chapter_of(r: &Requirement) -> Option<String> {
    for a in &r.anchors {
        for seg in a.locator.split(['>', '＞']) {
            if let Some(title) = chapter_heading(seg.trim()) {
                return Some(title);
            }
        }
    }
    None
}

/// 「第X章」标题段提取（手写扫描——bid-domain 不引 regex，依赖面不加宽）。
///
/// 形态：`第` + 章序数（中文/阿拉伯数字，1..=8 位）+ `章` + 章名余文；
/// 余文截至首个 `/`／`／`／`。`（夹具的「第二章/1.1」形态 → 「第二章」）。
/// 「第1页」「第2行」等非章序数不误触（后随字符非「章」）。
fn chapter_heading(seg: &str) -> Option<String> {
    const CN_NUMERALS: &[char] = &[
        '一', '二', '三', '四', '五', '六', '七', '八', '九', '十',
        '百', '千', '零', '〇', '两', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
    ];
    let cs: Vec<char> = seg.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] != '第' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < cs.len() && CN_NUMERALS.contains(&cs[j]) && j - i <= 8 {
            j += 1;
        }
        if j > i + 1 && j < cs.len() && cs[j] == '章' {
            let mut end = cs.len();
            for (k, &c) in cs[j + 1..].iter().enumerate() {
                if c == '/' || c == '／' || c == '。' {
                    end = j + 1 + k;
                    break;
                }
            }
            let title: String = cs[i..end].iter().collect();
            return Some(title.trim().to_string());
        }
        i += 1;
    }
    None
}

/// 兜底组叶标题的条款号区间（chunk 内全部 REQ 都有条款号才可用——
/// 「其他条款 1.1-1.7」；单条 chunk →「其他条款 4.1」；任一条款号缺失
/// → `None`（调用方退化到「（第N块）」）。
fn clause_range_title(chunk: &[&Requirement]) -> Option<String> {
    let nos: Vec<&str> = chunk
        .iter()
        .map(|r| r.clause_no.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .collect::<Option<Vec<_>>>()?;
    let (first, last) = (nos.first()?, nos.last()?);
    Some(if first == last {
        format!("{OTHER_FALLBACK_GROUP} {first}")
    } else {
        format!("{OTHER_FALLBACK_GROUP} {first}-{last}")
    })
}

/// 从 REQ 清单推导章节树（纯函数——确定性：按组首现序 + id 序，无隐藏随机性）。
/// 该 REQ 类目是否属于【标书正文】（#121 P0-1）。
///
/// 三类【评标规则】不进大纲——它们是评标委员会评判投标人的标准，不是
/// 投标人要写的内容；进正文 = 把「投标人不满足 XX 的予以否决」抄进投标
/// 文件（评审视为响应性瑕疵）。数据仍完整保留在 store 与合规矩阵里：
///
/// | 类目 | 归属 | 去处 |
/// |---|---|---|
/// | 否决项 | 评标规则 | 合规矩阵 blocking 判定 |
/// | 评分项 | 评标规则 | 自评打分依据 |
/// | 资格条件 | 评标规则 | 合规矩阵 blocking 判定 |
///
/// 其余（技术参数 / 商务要求 / 格式要求 / 其他）都是投标人要正面响应的
pub fn is_body_category(category: &str) -> bool {
    !matches!(category, "否决项" | "评分项" | "资格条件")
}

/// 该 REQ 类目是否需要在**正文里有响应章节**（#121 P0-1）。
///
/// 与 [`is_body_category`] 的区别是关键：
///
/// ```text
/// 否决项/资格条件  → 不写【规则原文】进正文（那是评标委员会的判断标准）
///                  → 但必须写【响应内容】（「我方满足 1.4.1 的要求，附营业执照…」）
///
/// 评分项          → 不写正文（自评表里体现）
/// ```
///
/// 所以大纲里要有一章「资格与响应」，把否决项/资格条件作为**响应提纲**列出
/// （标题用条款号+摘要，正文由 draft 写响应话术），而不是把招标文件原文抄进
/// 正文。
pub fn needs_response_section(category: &str) -> bool {
    matches!(category, "否决项" | "资格条件")
}

/// 结构：每个非空类目 → 1 个 level-1 节点；**「其他」类二次分组**（issue #100
/// 任务 1：锚点章节 [`chapter_of`] → 关键词主题 [`infer_theme`] → 兜底
/// 「其他条款」——catch-all 不再平铺成「其他（第N块）」×31）；组内 REQ 按
/// id 序切块（每块 ≤ max_reqs_per_leaf）→ level-2 叶节点。父节点
/// requirement_ids = 子并集、target_chars = 子和（聚合一致性由构造保证，
/// set_outline 复核）。
///
/// 配额：最大余数法（largest remainder）——先按 REQ 数占比取整分配，余量补给
/// 小数部分最大的叶（并列按文档序），总和恰等于 target（下限保护除外）。
/// **语义分组只改归组与标题，配额/切块算法不动**（issue #100 验收线）。
///
/// `next_id` 签名器由调用方注入（store 传 `self.next_id`；测试传计数闭包）——
/// 派生逻辑与 id 序列解耦。
pub fn derive_outline(
    reqs: &[Requirement],
    spec: OutlineSpec,
    next_id: &mut dyn FnMut(&str) -> String,
) -> Vec<OutlineNode> {
    // 【正文类目的两条边界】（#121 P0-1，实测 e2e 暴露的真问题）：
    //
    //   ① 评分项 —— 完全不进正文。评分方法是评标委员会的算分规则，
    //      投标人只需要在「自评表」里逐项打分，正文里写「价格分 30 分」
    //      是把自己的报价策略和评标办法混为一谈。
    //
    //   ② 否决项 / 资格条件 —— 【不写规则原文，但必须有响应章节】。
    //      这是最容易做错的一条（第一版按「规则类全过滤」做，result:
    //      合规矩阵立刻报 30+ 条 BLOCKER「否决项未响应」——因为规则要求
    //      每个否决项都得有响应章节）。
    //
    //      正确做法：合并成【一个】「资格与响应」章节组，叶标题用
    //      「条款号 + 原文摘要」（让投标人知道要响应哪一条），正文由
    //      draft 写【响应话术】（「我方满足 1.4.1 的要求，附件为营业执照
    //      复印件」），而不是抄招标文件原文。
    //
    //      这样：矩阵有章节 → 不报 BLOCKER；正文是响应而非规则 → 不出现
    //      「投标人不满足 XX 的予以否决」这种评标口径。
    const RESPONSE_GROUP: &str = "资格与响应";
    let body_reqs: Vec<(&Requirement, String)> = reqs
        .iter()
        .filter_map(|r| {
            if is_body_category(&r.category) {
                Some((r, r.category.clone()))
            } else if needs_response_section(&r.category) {
                // 否决项 + 资格条件 合并进同一个响应章节组
                Some((r, RESPONSE_GROUP.to_string()))
            } else {
                // 评分项：不进正文
                None
            }
        })
        .collect();
    let total_reqs: usize = reqs.len();
    if total_reqs == 0 {
        return vec![];
    }
    if body_reqs.is_empty() {
        // 全是纯规则类目（只有评分项）：大纲为空（调用方据 store 空否提示）。
        return vec![];
    }
    // 类目分桶（首现序——输入已按 id 序时全确定）
    let mut buckets: Vec<(String, Vec<&Requirement>)> = Vec::new();
    for (r, group) in &body_reqs {
        match buckets.iter_mut().find(|(c, _)| c == group) {
            Some((_, v)) => v.push(r),
            None => buckets.push((group.clone(), vec![r])),
        }
    }
    // 「其他」类二次分组（issue #100 任务 1）：catch-all 类平铺成
    // 「其他（第N块）」×31 是目录不可读的根因（用户实测）。归组键优先级：
    //   ① 招标文件章节（锚点 locator 的「第X章」段——路线 B，最忠实）；
    //   ② 关键词主题（infer_theme——路线 A，补遗等无章节结构者）；
    //   ③ 兜底「其他条款」（叶标题退化为条款号区间，不再裸「（第N块）」）。
    // 非其他类目原样通过（本任务只治 catch-all，不动既有类目结构）。
    let mut groups: Vec<(String, Vec<&Requirement>)> = Vec::new();
    for (cat, rs) in buckets {
        if cat != "其他" {
            groups.push((cat, rs));
            continue;
        }
        for r in rs {
            let key = chapter_of(r)
                .or_else(|| infer_theme(std::slice::from_ref(&r)))
                .unwrap_or_else(|| OTHER_FALLBACK_GROUP.to_string());
            match groups.iter_mut().find(|(c, _)| *c == key) {
                Some((_, v)) => v.push(r),
                None => groups.push((key, vec![r])),
            }
        }
    }
    // 叶块清单（先收集再配额——需要全局余数视角；标题在此一并定型）
    struct LeafDraft { group: String, title: String, req_ids: Vec<String> }
    let mut leaves: Vec<LeafDraft> = Vec::new();
    for (g, rs) in &groups {
        let chunk_size = spec.max_reqs_per_leaf.max(1);
        let n_chunks = rs.chunks(chunk_size).count();
        for (ci, chunk) in rs.chunks(chunk_size).enumerate() {
            let req_ids: Vec<String> = chunk.iter().map(|r| r.id.clone()).collect();
            let title = if g == OTHER_FALLBACK_GROUP {
                // 兜底组：条款号区间（「其他条款 1.1-1.7」）；缺条款号 → （第N块）
                clause_range_title(chunk)
                    .unwrap_or_else(|| format!("{g}（第{}块）", ci + 1))
            } else if n_chunks > 1 {
                format!("{}（第{}块）", g, ci + 1)
            } else {
                // 单叶组：组名即标题（「第一章 招标公告」——无块后缀）
                g.clone()
            };
            leaves.push(LeafDraft { group: g.clone(), title, req_ids });
        }
    }
    // 最大余数法配额
    let leaf_counts: Vec<u64> = leaves.iter().map(|l| l.req_ids.len() as u64).collect();
    let sum_counts: u64 = leaf_counts.iter().sum();
    let mut quotas: Vec<u32> = leaf_counts.iter()
        .map(|&c| ((u64::from(spec.target_total_chars) * c / sum_counts) as u32).max(MIN_LEAF_CHARS))
        .collect();
    // 下限保护后的偏差：差额按 REQ 占比回补/扣减（简单二次均衡，保总和 ≈ target）
    let quota_sum: u64 = quotas.iter().map(|q| u64::from(*q)).sum();
    let target = u64::from(spec.target_total_chars);
    if quota_sum != target {
        let diff = target.abs_diff(quota_sum);
        let step: u32 = if quota_sum < target { 1 } else { 0 }; // 1 = 加，0 = 减
        let mut remaining = diff;
        // 【#199 审查修复】触底判定改为「整轮零进展」，废弃旧判据
        // `i > len*4 && remaining == diff`：旧判据只在【从未扣减】时成立——
        // 一旦部分扣减后全部触底，remaining < diff 且不再变化，守卫永假 →
        // 死循环（100% CPU 空转）。实测触发：叶片增多使下限和超过
        // target、且大叶片头寸 < 差额时（e2e_artifacts 在 #199 分发扩大
        // 抽取面后必现）。加法模式每轮必有进展，不从此口退出。
        loop {
            let before = remaining;
            for q in quotas.iter_mut() {
                if remaining == 0 {
                    break;
                }
                if step == 1 {
                    *q += 1;
                    remaining -= 1;
                } else if *q > MIN_LEAF_CHARS {
                    *q -= 1;
                    remaining -= 1;
                }
            }
            // 一轮零进展 = 能减的已全部触底（target 过小）——接受偏差
            // （验收容差 ±10% 兜底）。空叶表亦安全（首轮即零进展退出）。
            if remaining == 0 || remaining == before {
                break;
            }
        }
    }
    // 组树：组（类目或语义子组）→ level-1，叶 → level-2
    let mut nodes: Vec<OutlineNode> = Vec::new();
    let mut order: u32 = 0;
    for (g, _) in &groups {
        let group_leaves: Vec<usize> = leaves.iter()
            .enumerate()
            .filter(|(_, l)| &l.group == g)
            .map(|(i, _)| i)
            .collect();
        let parent_id = next_id("SEC");
        order += 1;
        // 聚合面（子并集 / 配额和）
        let mut agg_reqs: Vec<String> = Vec::new();
        let mut agg_chars: u32 = 0;
        for &li in &group_leaves {
            for rid in &leaves[li].req_ids {
                if !agg_reqs.contains(rid) {
                    agg_reqs.push(rid.clone());
                }
            }
            agg_chars += quotas[li];
        }
        nodes.push(OutlineNode {
            id: parent_id.clone(),
            parent_id: None,
            title: g.clone(),
            level: 1,
            order,
            requirement_ids: agg_reqs,
            target_chars: agg_chars,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        });
        for &li in &group_leaves {
            order += 1;
            nodes.push(OutlineNode {
                id: next_id("SEC"),
                parent_id: Some(parent_id.clone()),
                title: leaves[li].title.clone(),
                level: 2,
                order,
                requirement_ids: leaves[li].req_ids.clone(),
                target_chars: quotas[li],
                status: SectionStatus::Pending,
                dirty: false,
                content: None,
                content_attempt: 0,
            });
        }
    }
    nodes
}

/// 树不变量校验（纯函数——set_outline 的裁决面，错误即 E_UNPROCESSABLE reason）。
pub fn validate_outline(nodes: &[OutlineNode]) -> Result<(), String> {
    if nodes.is_empty() {
        return Err("空章节树（REQ 为空时无从推导——先 extract_requirements）".into());
    }
    // 1. id 唯一
    let mut seen = std::collections::HashSet::new();
    for n in nodes {
        if !seen.insert(&n.id) {
            return Err(format!("章节 id 重复: {}", n.id));
        }
    }
    // 2-6. 结构面
    for n in nodes {
        if n.level == 0 || n.level > MAX_LEVEL {
            return Err(format!("{} level={} 越域（1..={MAX_LEVEL}）", n.id, n.level));
        }
        if n.requirement_ids.is_empty() {
            return Err(format!("{} 未绑定任何 REQ（每节点 ≥1——分片最小调度单元必须有活干）", n.id));
        }
        match &n.parent_id {
            None => {
                if n.level != 1 {
                    return Err(format!("{} 无父但 level={}（根必须 level=1）", n.id, n.level));
                }
            }
            Some(pid) => {
                let parent = nodes.iter().find(|x| &x.id == pid).ok_or_else(|| {
                    format!("{} 的父 {} 不存在（孤儿节点）", n.id, pid)
                })?;
                if n.level != parent.level + 1 {
                    return Err(format!(
                        "{} level={} 与父 {} level={} 跳级（子 = 父 + 1）",
                        n.id, n.level, pid, parent.level
                    ));
                }
                // 3. 环检测：父链必须在有限步内到根
                let mut cursor = parent.clone();
                let mut hops = 0;
                while let Some(ref pp) = cursor.parent_id {
                    cursor = nodes.iter().find(|x| x.id == *pp).ok_or_else(|| {
                        format!("{} 的祖先 {} 不存在", cursor.id, pp)
                    })?.clone();
                    hops += 1;
                    if hops > nodes.len() {
                        return Err(format!("{} 的父链成环", n.id));
                    }
                }
            }
        }
    }
    // 7. order 全局唯一
    let mut orders = std::collections::HashSet::new();
    for n in nodes {
        if !orders.insert(n.order) {
            return Err(format!("order={} 重复（文档序必须全局唯一）", n.order));
        }
    }
    // 9. 父聚合一致性（requirement_ids == 子并集 && target_chars == 子和）
    for n in nodes {
        let children: Vec<&OutlineNode> = nodes.iter().filter(|x| x.parent_id.as_deref() == Some(&n.id[..])).collect();
        if !children.is_empty() {
            let mut union: Vec<String> = Vec::new();
            let mut sum = 0u64;
            for c in &children {
                for rid in &c.requirement_ids {
                    if !union.contains(rid) {
                        union.push(rid.clone());
                    }
                }
                sum += u64::from(c.target_chars);
            }
            let mut stored = n.requirement_ids.clone();
            stored.sort();
            let mut expect = union;
            expect.sort();
            if stored != expect {
                return Err(format!(
                    "{} 的 requirement_ids 与子并集不一致（聚合面失真——重建章节树）", n.id
                ));
            }
            if u64::from(n.target_chars) != sum {
                return Err(format!(
                    "{} target_chars={} ≠ 子配额和 {}（配额账不平）", n.id, n.target_chars, sum
                ));
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------ #180 章节内容摘要
/// 章节树内容摘要（#180 审核内容绑定——PreExportConfirm 门绑定的判据面）。
///
/// **编码即契约**（字段清单与排序理由——改动即令所有历史摘要作废）：
/// - **字段清单**：每节点喂 `(id, parent_id, title, level, order, content)`
///   六字段——这正是**进出导出产物**的章节面（结构定位 + 叶正文）。
///   显式排除 `requirement_ids` / `target_chars` / `status` / `dirty` /
///   `content_attempt`（调度/账面字段，不进产物——绑它们会把「无关变更」
///   误报成「内容变更」；绑全局 `content_version` 更必然误报：导入无关件/
///   记审自身/导出推进都 bump 它，连「重新过门」都会在 bump 后立刻失效）；
/// - **排序**：按 `(order, id)` 升序逐节点喂入——`order` 是文档序（全局
///   唯一），`id` 仅作防御性并列锚（合法树 order 不并列；手搓/损坏数据
///   不依赖 Vec 存储序——同一棵树的不同插入序必须同摘要）；
/// - **分隔符**：字段间与每节点尾均 `\0`（NUL 不出现在正常章标题/正文
///   文本中，是比空格/换行更强的字段界）；`content` 为 `None`（未起草/
///   结构节点）与 `parent_id` 为 `None`（根）均喂空串；
/// - **数值面**：`level`/`order` 取十进制无填充（定界靠 `\0` 不靠宽度）。
///
/// 返回 sha256 十六进制小写（64 字符）；空树 = 空输入的 sha256（常量，
/// 「无章节」与「无章节」仍相等——不误伤无大纲项目）。
pub fn sections_digest(sections: &[OutlineNode]) -> String {
    let mut sorted: Vec<&OutlineNode> = sections.iter().collect();
    sorted.sort_by(|a, b| (a.order, &a.id).cmp(&(b.order, &b.id)));
    let mut hasher = Sha256::new();
    for n in sorted {
        hasher.update(n.id.as_bytes());
        hasher.update(b"\0");
        hasher.update(n.parent_id.as_deref().unwrap_or("").as_bytes());
        hasher.update(b"\0");
        hasher.update(n.title.as_bytes());
        hasher.update(b"\0");
        hasher.update(n.level.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(n.order.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(n.content.as_deref().unwrap_or("").as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------- DomainStore 扩面（T-5 章节树操作）
impl DomainStore {
    /// 章节树只读面。
    pub fn sections(&self) -> &[OutlineNode] {
        &self.sections
    }

    /// 当前章节树内容摘要（#180——`record_review` 落账与 `export_allowed`
    /// 判定共用的单一真源；编码契约见自由函数 [`sections_digest`]）。
    pub fn sections_digest(&self) -> String {
        sections_digest(&self.sections)
    }

    /// 章节树总字数配额（**根节点求和**——父配额 = 子配额和，全树求和会双计）。
    pub fn outline_total_chars(&self) -> u64 {
        self.sections.iter()
            .filter(|n| n.parent_id.is_none())
            .map(|n| u64::from(n.target_chars)).sum()
    }

    /// 整树替换（重开纲——补遗后的重推导走同门）。
    ///
    /// 状态门：REQUIREMENTS_EXTRACTED / MATRIX_READY / EVIDENCE_BOUND 可重开纲；
    /// DRAFT 起草前置面未抽条款、DRAFTING+ 起草中重开纲需 S-5 影响面机制（后续批次）。
    pub fn set_outline(&mut self, nodes: Vec<OutlineNode>) -> Result<(usize, u64), ToolError> {
        let p = self.project()?;
        let allowed = matches!(
            p.status,
            ProjectStatus::RequirementsExtracted | ProjectStatus::MatrixReady | ProjectStatus::EvidenceBound
        );
        if !allowed {
            return Err(ToolError::EState {
                current: p.status,
                attempted: "set_outline",
            });
        }
        if let Err(reason) = validate_outline(&nodes) {
            return Err(ToolError::EUnprocessable { reason });
        }
        // REQ 存在性（引用完整性——挂空 REQ 的章节是幽灵调度单元）
        let known: std::collections::HashSet<&str> = self.requirements.iter().map(|r| r.id.as_str()).collect();
        for n in &nodes {
            for rid in &n.requirement_ids {
                if !known.contains(&rid[..]) {
                    return Err(ToolError::EUnprocessable {
                        reason: format!("{} 引用了不存在的条款 {rid}（幽灵调度单元）", n.id),
                    });
                }
            }
        }
        let total = nodes.iter()
            .filter(|n| n.parent_id.is_none())
            .map(|n| u64::from(n.target_chars)).sum::<u64>();
        let count = nodes.len();
        // #224 引用完整性（Lead 2026-11-04 review blocker 2）：评分映射引用的
        // 章节不得随重挂大纲消失——SEC id 随树重建重签，旧 SEC 失踪 = 孤儿
        // 映射；孤儿态一经 checkpoint 落盘，下一次 restore 拒载（快照卡死）。
        // 拒破坏性重挂（fail-loud）：映射账本是审计面，不得静默蒸发。
        if !self.scoring_mappings.is_empty() {
            let known: std::collections::HashSet<&str> =
                nodes.iter().map(|n| n.id.as_str()).collect();
            if let Some(m) = self
                .scoring_mappings
                .iter()
                .find(|m| !known.contains(m.section_id.as_str()))
            {
                return Err(ToolError::EUnprocessable {
                    reason: format!(
                        "评分映射 {} 引用的章节 {} 将在重挂大纲后不存在（#224 引用完整性：大纲重建不得孤儿化映射账本——先处理该映射或保留该章节）",
                        m.id, m.section_id
                    ),
                });
            }
        }
        self.sections = nodes;
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok((count, total))
    }

    /// 从已抽条款推导并落树（验收主入口——「从 REQ 清单推导章节树」）。
    pub fn derive_and_set_outline(&mut self, spec: OutlineSpec) -> Result<(usize, u64), ToolError> {
        let reqs = self.requirements.clone();
        let nodes = derive_outline(&reqs, spec, &mut |prefix| self.next_id(prefix));
        self.set_outline(nodes)
    }

    /// 写入章正文（#59 阶段 1 / draft_response_section 的 CAS 面）。
    ///
    /// - 章节不存在 → E_NOTFOUND；
    /// - **结构节点（非叶）拒绝正文**——父节点是调度聚合单元，内容只住叶
    ///   （与 S-2 assemble_all「只对叶提交」同口径，导出面 ChapterRecord
    ///   挂非叶即 fail-closed，见 export.rs `RecordOnStructuralNode`）；
    /// - 同内容重提交 → no-op（attempt 不推进——「前 N 章字节不变」的账面
    ///   前提）；不同内容 → attempt + 1（版本单调）；
    /// - content_version + 1（仅实际变更时）。
    ///
    /// 返回 (attempt, char_count)——char_count 用 `chars().count()`（中文
    /// 字数口径，与 ChapterRecord.char_count / budget.rs N-3 同源）。
    pub fn set_section_content(&mut self, id: &str, content: String)
        -> Result<(u32, u32), ToolError> {
        let idx = self.sections.iter().position(|n| n.id == id).ok_or_else(|| {
            ToolError::ENotFound { what: format!("section {id} 不存在") }
        })?;
        // 叶判定：无子者（与 assemble_all 的 parents 集合同口径）
        let has_child = self.sections.iter().any(|n| n.parent_id.as_deref() == Some(id));
        if has_child {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "{id} 是结构节点（有子节点）——正文只住叶节点，聚合单元不承载内容"
                ),
            });
        }
        let node = &mut self.sections[idx];
        if node.content.as_deref() == Some(content.as_str()) {
            // no-op：同内容重提交（幂等面——attempt 与 content_version 均不动）
            let chars = node.content.as_ref().map(|c| c.chars().count()).unwrap_or(0) as u32;
            return Ok((node.content_attempt.max(1), chars));
        }
        node.content_attempt = node.content_attempt.saturating_add(1).max(1);
        let chars = content.chars().count() as u32;
        node.content = Some(content);
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok((node.content_attempt, chars))
    }

    /// 章节状态推进（domain-model 线性链 + Validating 直通 Accepted 捷径）。
    /// 拒绝面零副作用（E_UNPROCESSABLE——T-7 审核闭环落地时升格为 typed 状态错）。
    pub fn advance_section_status(&mut self, id: &str, to: SectionStatus) -> Result<(), ToolError> {
        let idx = self.sections.iter().position(|n| n.id == id).ok_or_else(|| {
            ToolError::ENotFound { what: format!("section {id} 不存在") }
        })?;
        let cur = self.sections[idx].status;
        if !cur.next_allowed().contains(&to) {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "章节状态 {} 不允许推进到 {}（合法前向: {:?}）",
                    cur.as_str(), to.as_str(),
                    cur.next_allowed().iter().map(|s| s.as_str()).collect::<Vec<_>>()
                ),
            });
        }
        self.sections[idx].status = to;
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }

    /// 显式 reopen（R-06）：ACCEPTED → REVISING + 打脏标记。
    /// 唯一离开终态的通道——「验收后修改必须显式」，防静默回退。
    pub fn reopen_section(&mut self, id: &str) -> Result<(), ToolError> {
        let idx = self.sections.iter().position(|n| n.id == id).ok_or_else(|| {
            ToolError::ENotFound { what: format!("section {id} 不存在") }
        })?;
        if self.sections[idx].status != SectionStatus::Accepted {
            return Err(ToolError::EUnprocessable {
                reason: format!(
                    "reopen 仅适用于 ACCEPTED（当前 {}——非终态用 advance）",
                    self.sections[idx].status.as_str()
                ),
            });
        }
        self.sections[idx].status = SectionStatus::Revising;
        self.sections[idx].dirty = true;
        if let Some(p) = self.project.as_mut() {
            p.content_version += 1;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParseStatus;

    /// 章节树基座：项目 + 文件 + N 条 REQ。
    fn store_with_reqs(n_reqs: usize) -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("测试项目", "ZFCG-2026-GK-0311", "明标", "ext-key-sec").unwrap();
        s.upsert_file("A-招标文件.docx", "招标文件", "aa", 31, 12, 120, 0,
                      ParseStatus::Ok, "").unwrap();
        let categories = ["资格条件", "技术参数", "评分项", "商务要求", "格式要求", "其他"];
        let reqs: Vec<Requirement> = (0..n_reqs).map(|i| Requirement {
            id: format!("REQ-{:04}", i + 3),
            category: categories[i % categories.len()].into(),
            clause_no: Some(format!("3.{}", i + 1)),
            anchors: vec![crate::Anchor {
                file: "A".into(), kind: "para".into(),
                locator: "第三章".into(),
                excerpt: format!("条款-{i} 的原文摘录"),
            }],
            params: std::collections::HashMap::new(),
            source_file: "A".into(),
        }).collect();
        s.set_requirements(reqs).unwrap();
        s
    }

    /// GT sections.jsonl 六章节语义面（人工树——验证 set_outline 与聚合一致性）。
    fn gt_six_sections() -> Vec<OutlineNode> {
        let gt_sections: &[(&str, &[&str])] = &[
            ("资格响应", &["REQ-0003", "REQ-0004", "REQ-0005"]),
            ("技术方案-网络与安全", &["REQ-0006", "REQ-0007", "REQ-0008"]),
            ("商务部分-报价", &["REQ-0009", "REQ-0010", "REQ-0011", "REQ-0012"]),
            ("售后服务方案", &["REQ-0013"]),
            ("项目团队", &["REQ-0014"]),
            ("格式与签署", &["REQ-0015", "REQ-0016", "REQ-0017"]),
        ];
        gt_sections.iter().enumerate().map(|(i, (title, gts))| {
            let reqs: Vec<String> = gts.iter().map(|g| g.to_string()).collect();
            let chars = (reqs.len() as u32) * 2000;
            OutlineNode {
                id: format!("SEC-{:04}", i + 1),
                parent_id: None,
                title: (*title).into(),
                level: 1,
                order: (i + 1) as u32,
                requirement_ids: reqs,
                target_chars: chars,
                status: SectionStatus::Pending,
                dirty: false,
                content: None,
                content_attempt: 0,
            }
        }).collect()
    }

    // ============================ 自研测试 ============================

    /// T-5 验收主测：320 条 REQ → 300-500 节点、每节点 ≥1 REQ、配额和 ≈ 目标 ±10%。
    #[test]
    fn derive_acceptance_scale_320_reqs() {
        let mut s = store_with_reqs(320);
        let (count, total) = s.derive_and_set_outline(OutlineSpec {
            target_total_chars: 640_000,
            max_reqs_per_leaf: 1, // 每叶 1 REQ → 320 叶 + 类目父
        }).unwrap();
        // #121 P0-1 后：资格条件/评分项被过滤（不进正文），320 条里剩「技术参数/商务要求/
        // 格式要求/其他」4 类 ≈ 213 条 → 213 叶 + 4 类目父 ≈ 217 节点。
        // 验收带相应下移（原 300..=500 是六类全进正文时的口径）。
        assert!((200..=350).contains(&count), "节点数 {count} 必须在 200..=350（#121 P0-1 后的验收带）");
        // 每节点 ≥1 REQ（validate_outline 已把关——这里复核总量守恒）
        let bound: std::collections::HashSet<&str> = s.sections().iter()
            .flat_map(|n| n.requirement_ids.iter())
            .map(|r| &r[..])
            .collect();
        // #121 P0-1：评分项(53) 不进正文 → 剩 267 条（资格条件54 并入「资格与响应」）。
        let leaves: Vec<&OutlineNode> = s.sections().iter().filter(|n| n.level == 2).collect();
        assert_eq!(leaves.len(), 267, "每叶恰 1 REQ（267 = 320 - 53 评分项）");
        assert_eq!(leaves.iter().map(|l| l.requirement_ids.len()).sum::<usize>(), 267);
        assert_eq!(bound.len(), 267, "正文类目 REQ 全覆盖无遗漏");
        // 配额和 ≈ 目标 ±10%
        let dev = (total as f64 - 640_000.0).abs() / 640_000.0;
        assert!(dev <= 0.10, "配额和 {total} 偏离目标 ±10%（实际偏离 {:.1}%）", dev * 100.0);
        assert_eq!(s.outline_total_chars(), total, "账面一致");
        // 默认参数（24/叶）：267 REQ ÷ 5 组 → 资格与响应54/技术参数54/商务53/格式53/其他53
        // → 每组 ceil(n/24)=3 叶 → 5 父 + 15 叶 = 20 节点
        let mut s2 = store_with_reqs(320);
        let (c2, _) = s2.derive_and_set_outline(OutlineSpec::default()).unwrap();
        assert_eq!(c2, 5 + 15, "默认粒度分片数（5 组 × 3 叶 + 5 父）");
    }

    /// 确定性：同输入双推导全等（分片调度的可重放前提——S-2 断点续跑的地基）。
    #[test]
    fn derive_is_deterministic() {
        let reqs_of = |s: &DomainStore| s.requirements.clone();
        let mut s1 = store_with_reqs(50);
        let mut s2 = store_with_reqs(50);
        s1.derive_and_set_outline(OutlineSpec { target_total_chars: 100_000, max_reqs_per_leaf: 7 }).unwrap();
        s2.derive_and_set_outline(OutlineSpec { target_total_chars: 100_000, max_reqs_per_leaf: 7 }).unwrap();
        assert_eq!(s1.sections(), s2.sections(), "同输入双推导必须全等");
        assert_eq!(reqs_of(&s1), reqs_of(&s2));
        // 配额和恰等于目标（最大余数法无下限触发时）
        assert_eq!(s1.outline_total_chars(), 100_000, "最大余数法：总和恰等于目标");
    }

    /// #253 第一段：建纲默认总字数的**单一真源**——常量满足 1000 页硬指标、
    /// `Default` 引用常量（不另写数字）、默认规格建纲后配额总和恰等于常量
    /// （最大余数法不缩水）。MCP 缺省与 schema 描述的对偶断言分别在
    /// bid-mcp tests（mcp_scaffold_tools / dispatch_contract）——三层一致。
    #[test]
    fn default_target_total_chars_is_single_source_of_truth() {
        // 硬指标下限：1000 页 × 实测密度 844 字符/页 = 844,000
        assert!(
            DEFAULT_TARGET_TOTAL_CHARS >= 844_000,
            "默认总字数 {} 必须满足 1000 页硬指标（844 字/页 → ≥844,000）",
            DEFAULT_TARGET_TOTAL_CHARS
        );
        // 单一真源：Default 与常量一致（MCP/schema 层引用同一常量）
        assert_eq!(
            OutlineSpec::default().target_total_chars,
            DEFAULT_TARGET_TOTAL_CHARS
        );
        // 最大余数法：默认规格建纲后，叶配额总和恰等于默认目标
        let mut s = store_with_reqs(50);
        s.derive_and_set_outline(OutlineSpec::default()).unwrap();
        assert_eq!(
            s.outline_total_chars(),
            DEFAULT_TARGET_TOTAL_CHARS as u64,
            "默认规格建纲后配额总和必须恰等于 DEFAULT_TARGET_TOTAL_CHARS"
        );
    }

    // ==================== #100 任务 1：其他类语义分组 ====================

    /// 语义分组夹具：带锚点（locator 章节路径 + 摘录）与条款号的 REQ。
    fn req_with_anchor(
        id: &str, category: &str, clause_no: Option<&str>, locator: &str, excerpt: &str,
    ) -> Requirement {
        Requirement {
            id: id.into(),
            category: category.into(),
            clause_no: clause_no.map(String::from),
            anchors: vec![crate::Anchor {
                file: "A".into(),
                kind: "para".into(),
                locator: locator.into(),
                excerpt: excerpt.into(),
            }],
            params: std::collections::HashMap::new(),
            source_file: "A".into(),
        }
    }

    /// 计数签名器（derive_outline 的 next_id 注入面）。
    fn counting_ids() -> impl FnMut(&str) -> String {
        let mut n = 0u32;
        move |p| {
            n += 1;
            format!("{p}-{n:04}")
        }
    }

    /// 章节优先分组（路线 B）：「其他」类按锚点 locator 的「第X章」段归并——
    /// level-1 标题 = 招标文件自己的章节名（验收线「标题与招标文件章节对应」），
    /// 目录不再有「其他（第N块）」；配额/覆盖/树不变量全保持。
    #[test]
    fn other_category_regroups_by_chapter() {
        let reqs = vec![
            req_with_anchor("REQ-0001", "资格条件", Some("1.4.1"),
                "第一章 招标公告 > 二、投标人资格要求",
                "1.4.1 投标人须具备独立法人资格"),
            req_with_anchor("REQ-0002", "其他", Some("1.1"),
                "第一章 招标公告 > 一、项目概况",
                "1.1 项目名称：某市第一人民医院智慧病房物联网平台建设项目；项目编号：ZHYY-2026-GK-018"),
            req_with_anchor("REQ-0003", "其他", Some("1.3"),
                "第一章 招标公告 > 一、项目概况",
                "1.3 建设地点：院区内；工期：180 日历天；质量标准：符合验收合格标准"),
            req_with_anchor("REQ-0004", "其他", Some("3.4.1"),
                "第三章 投标人须知 > 四、投标报价",
                "3.4.1 投标报价为一次包干总价，包含货物、安装、调试等全部费用"),
            req_with_anchor("REQ-0005", "其他", Some("6.1"),
                "第六章 合同条款及格式（摘要）",
                "6.1 合同价格形式：固定总价合同，除约定情形外不做调整"),
        ];
        let nodes = derive_outline(
            &reqs,
            OutlineSpec { target_total_chars: 10_000, max_reqs_per_leaf: 1 },
            &mut counting_ids(),
        );
        // level-1 = 首现序：资格条件（原类目）+ 三个招标文件章节组
        let l1: Vec<&str> = nodes.iter().filter(|n| n.level == 1).map(|n| n.title.as_str()).collect();
        assert_eq!(
            l1,
            vec!["资格与响应", "第一章 招标公告", "第三章 投标人须知", "第六章 合同条款及格式（摘要）"],
            "其他类按招标文件章节归并；资格条件并入「资格与响应」组（#121 P0-1）"
        );
        // 核心验收：不再有裸「其他（第N块）」
        assert!(
            nodes.iter().all(|n| !n.title.starts_with("其他（")),
            "目录不得再出现「其他（第N块）」——实测 31 块平铺的根因"
        );
        // 多叶组带块序，单叶组用组名（无块后缀）
        let ch1: Vec<&OutlineNode> = nodes.iter().filter(|n| n.title.starts_with("第一章")).collect();
        assert_eq!(ch1.len(), 3, "1 父 + 2 叶");
        assert_eq!(ch1[1].title, "第一章 招标公告（第1块）");
        assert_eq!(ch1[2].title, "第一章 招标公告（第2块）");
        let ch6_leaves: Vec<&OutlineNode> = nodes.iter()
            .filter(|n| n.parent_id.is_some() && n.title.starts_with("第六章"))
            .collect();
        assert_eq!(ch6_leaves.len(), 1);
        assert_eq!(ch6_leaves[0].title, "第六章 合同条款及格式（摘要）", "单叶组：组名即标题");
        // 守恒面：REQ 全覆盖 + 配额和恰等于目标 + 树不变量
        let bound: std::collections::HashSet<&str> = nodes.iter()
            .flat_map(|n| n.requirement_ids.iter()).map(|s| &s[..]).collect();
        // #121 P0-1：资格条件并入「资格与响应」组，仍覆盖全部 5 条。
        assert_eq!(bound.len(), 5, "REQ 全覆盖无遗漏（资格条件在「资格与响应」组）");
        let total: u64 = nodes.iter().filter(|n| n.parent_id.is_none())
            .map(|n| u64::from(n.target_chars)).sum();
        assert_eq!(total, 10_000, "配额算法不动（最大余数法总和恰等于目标）");
        assert!(validate_outline(&nodes).is_ok(), "树不变量（聚合一致/无孤儿/无环）");
    }

    /// 主题兜底（路线 A）与条款号区间叶标题：无章节锚的补遗条款按关键词归
    /// 主题组；章节与主题都推不出 → 兜底「其他条款」，叶标题用条款号区间
    /// （issue #100「其他条款 1.1-1.7」形态）；条款号缺失才退化到（第N块）。
    #[test]
    fn other_category_theme_fallback_and_clause_ranges() {
        let reqs = vec![
            // 补遗（无章节锚「第1页 行6」形态）：关键词命中主题
            req_with_anchor("REQ-0001", "其他", Some("1.1"), "第1页 行6",
                "原招标文件第四章商务分评分办法调整，奖项分值上限修改"),
            req_with_anchor("REQ-0002", "其他", Some("5.1"), "第2页 行8",
                "现场踏勘与答疑安排：踏勘集合时间地点详见补遗说明"),
            // 开标顺延（主题规则命中「投标与开标安排」——具体规则先于补遗规则）
            req_with_anchor("REQ-0003", "其他", Some("1.2"), "第1页 行7",
                "投标截止时间及开标时间相应顺延为 2026-11-30 09:30"),
            // 无任何信号 → 兜底组（有条款号 → 区间标题）
            req_with_anchor("REQ-0004", "其他", Some("9.9"), "附录 说明",
                "本条款无主题信号，仅作兜底归组验证"),
            req_with_anchor("REQ-0005", "其他", Some("9.10"), "附录 说明",
                "同上，另一条无信号条款"),
            // 无信号且无条款号 → 兜底组退化到（第N块）
            req_with_anchor("REQ-0006", "其他", None, "附录 说明",
                "无信号且无条款号的兜底条款"),
        ];
        let nodes = derive_outline(
            &reqs,
            OutlineSpec { target_total_chars: 12_000, max_reqs_per_leaf: 8 },
            &mut counting_ids(),
        );
        let l1: Vec<&str> = nodes.iter().filter(|n| n.level == 1).map(|n| n.title.as_str()).collect();
        assert_eq!(
            l1,
            vec!["补遗与澄清响应", "投标与开标安排", "其他条款"],
            "无章节锚：主题归组 + 兜底组（首现序）"
        );
        // 主题组单叶 → 组名即标题
        let byd: Vec<&OutlineNode> = nodes.iter()
            .filter(|n| n.parent_id.is_some() && n.title.starts_with("补遗")).collect();
        assert_eq!(byd.len(), 1);
        assert_eq!(byd[0].title, "补遗与澄清响应");
        assert_eq!(byd[0].requirement_ids.len(), 2, "两条补遗条款归同一主题组");
        // 兜底组单叶（3 条 ≤ 8/叶）：有条款号的区间形态失效（REQ-0006 缺号）→ （第1块）
        let fb: Vec<&OutlineNode> = nodes.iter()
            .filter(|n| n.parent_id.is_some() && n.title.starts_with("其他条款")).collect();
        assert_eq!(fb.len(), 1);
        assert_eq!(fb[0].title, "其他条款（第1块）", "混合缺号 chunk 退化为（第N块）");
        assert_eq!(fb[0].requirement_ids.len(), 3);
        // 纯条款号 chunk 的区间标题（去掉无号条款再推导）
        let reqs2: Vec<Requirement> = reqs[..5].to_vec();
        let nodes2 = derive_outline(
            &reqs2,
            OutlineSpec { target_total_chars: 10_000, max_reqs_per_leaf: 8 },
            &mut counting_ids(),
        );
        let fb2: Vec<&OutlineNode> = nodes2.iter()
            .filter(|n| n.parent_id.is_some() && n.title.starts_with("其他条款")).collect();
        assert_eq!(fb2.len(), 1);
        assert_eq!(fb2[0].title, "其他条款 9.9-9.10", "issue #100 兜底形态：条款号区间");
        // 守恒面
        let bound: std::collections::HashSet<&str> = nodes.iter()
            .flat_map(|n| n.requirement_ids.iter()).map(|s| &s[..]).collect();
        assert_eq!(bound.len(), 6);
        let total: u64 = nodes.iter().filter(|n| n.parent_id.is_none())
            .map(|n| u64::from(n.target_chars)).sum();
        assert_eq!(total, 12_000);
        assert!(validate_outline(&nodes).is_ok());
    }

    /// 用户实测形态回归（synth-tender 31 条「其他」）：混合章节/补遗/无信号
    /// 的大样本——任何节点不得以「其他（」开头；分组确定性可重放。
    #[test]
    fn other_category_no_bare_block_titles_at_scale() {
        let mut reqs: Vec<Requirement> = Vec::new();
        let mut n = 0;
        // 24 条带章节锚（3 章 × 8 条——synth 一/三/六章分布的放大）
        for (chap, cnt) in [("第一章 招标公告", 8), ("第三章 投标人须知", 8), ("第六章 合同条款及格式（摘要）", 8)] {
            for i in 0..cnt {
                n += 1;
                reqs.push(req_with_anchor(
                    &format!("REQ-{n:04}"), "其他", Some(&format!("{n}.1")),
                    &format!("{chap} > 一、子节{i}"),
                    &format!("{n}.1 章节内条款：本条按招标文件该章要求响应"),
                ));
            }
        }
        // 4 条补遗（无章节锚）
        for i in 0..4 {
            n += 1;
            reqs.push(req_with_anchor(
                &format!("REQ-{n:04}"), "其他", Some(&format!("1.{i}")), "第1页 行6",
                &format!("1.{i} 原招标文件相关内容调整，以此补遗为准"),
            ));
        }
        // 3 条无信号（兜底）
        for i in 0..3 {
            n += 1;
            reqs.push(req_with_anchor(
                &format!("REQ-{n:04}"), "其他", Some(&format!("9.{i}")), "附录 说明",
                "无主题信号条款",
            ));
        }
        let spec = OutlineSpec { target_total_chars: 62_000, max_reqs_per_leaf: 8 };
        let nodes = derive_outline(&reqs, spec, &mut counting_ids());
        // 核心断言：全树无裸「其他（第N块）」
        assert!(
            nodes.iter().all(|x| !x.title.starts_with("其他（")),
            "31 块实测形态：任何节点不得再以「其他（」开头"
        );
        let l1: Vec<&str> = nodes.iter().filter(|x| x.level == 1).map(|x| x.title.as_str()).collect();
        assert_eq!(
            l1,
            vec!["第一章 招标公告", "第三章 投标人须知", "第六章 合同条款及格式（摘要）",
                 "补遗与澄清响应", "其他条款"],
            "5 个语义组（3 章节 + 1 主题 + 1 兜底）"
        );
        // 每章 8 条 = 恰 1 叶 → 组名即标题；兜底 3 条 → 1 叶（区间 9.0-9.2）
        let leaves: Vec<&str> = nodes.iter().filter(|x| x.parent_id.is_some())
            .map(|x| x.title.as_str()).collect();
        assert!(leaves.contains(&"第一章 招标公告"));
        assert!(leaves.contains(&"其他条款 9.0-9.2"));
        // 守恒面：31 条全覆盖 + 配额 ±10% + 树不变量 + 确定性
        let bound: std::collections::HashSet<&str> = nodes.iter()
            .flat_map(|x| x.requirement_ids.iter()).map(|s| &s[..]).collect();
        assert_eq!(bound.len(), n as usize, "REQ 全覆盖");
        let total: u64 = nodes.iter().filter(|x| x.parent_id.is_none())
            .map(|x| u64::from(x.target_chars)).sum();
        let dev = (total as f64 - 62_000.0).abs() / 62_000.0;
        assert!(dev <= 0.10, "配额和 {total} 偏离目标 ±10% 以内");
        assert!(validate_outline(&nodes).is_ok());
        let again = derive_outline(&reqs, spec, &mut counting_ids());
        assert_eq!(nodes, again, "语义分组确定性：同输入双推导全等");
    }

    /// GT sections.jsonl 六章节（人工树走 set_outline——全 level-1 平铺也合法）。
    #[test]
    fn gt_six_section_outline_lands() {
        let mut s = store_with_reqs(15);
        let (count, total) = s.set_outline(gt_six_sections()).unwrap();
        assert_eq!(count, 6, "六章节（sections.jsonl 口径）");
        assert_eq!(total, (3 + 3 + 4 + 1 + 1 + 3) * 2000, "配额账 = REQ 数 × 2000（全 level-1 平铺——根求和口径）");
        // 幂等重设（重开纲）：同树替换成功（补遗后重推导同门）
        let v = s.content_version();
        let (count2, _) = s.set_outline(gt_six_sections()).unwrap();
        assert_eq!(count2, 6);
        assert_eq!(s.content_version(), v + 1, "重开纲 bump（真实结构变化语义）");
    }

    /// 树不变量拒绝面（九类破坏逐项注入——校验器的回归闸门）。
    #[test]
    fn set_outline_rejects_broken_trees() {
        let mut s = store_with_reqs(15);
        let base = gt_six_sections();
        let assert_reject = |s: &mut DomainStore, nodes: Vec<OutlineNode>, why: &str| {
            let n_before = s.sections().len();
            let err = s.set_outline(nodes).unwrap_err();
            assert_eq!(err.code(), "E_UNPROCESSABLE", "{why}");
            assert_eq!(s.sections().len(), n_before, "{why}: 拒绝路径零残留");
        };
        // 孤儿父
        let mut t1 = base.clone();
        t1[1].parent_id = Some("SEC-9999".into());
        t1[1].level = 2;
        assert_reject(&mut s, t1, "孤儿父");
        // 环（两节点互为父子——先造合法两代树再互指）
        let mut t2 = vec![
            OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 1, order: 1,
                requirement_ids: vec!["REQ-0003".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false,
                content: None, content_attempt: 0 },
            OutlineNode { id: "SEC-B".into(), parent_id: Some("SEC-A".into()), title: "B".into(), level: 2, order: 2,
                requirement_ids: vec!["REQ-0004".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false,
                content: None, content_attempt: 0 },
        ];
        t2[0].parent_id = Some("SEC-B".into()); // A 的父是 B，B 的父是 A
        assert_reject(&mut s, t2, "父链成环");
        // 跳级
        let t3 = vec![
            OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 1, order: 1,
                requirement_ids: vec!["REQ-0003".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false,
                content: None, content_attempt: 0 },
            OutlineNode { id: "SEC-B".into(), parent_id: Some("SEC-A".into()), title: "B".into(), level: 3, order: 2,
                requirement_ids: vec!["REQ-0004".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false,
                content: None, content_attempt: 0 },
        ];
        assert_reject(&mut s, t3, "level 跳级");
        // level 越域（5 层）
        let t4 = vec![
            OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 5, order: 1,
                requirement_ids: vec!["REQ-0003".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false,
                content: None, content_attempt: 0 },
        ];
        assert_reject(&mut s, t4, "level 越域");
        // 根非 level 1
        let t5 = vec![OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 2, order: 1,
            requirement_ids: vec!["REQ-0003".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false,
                content: None, content_attempt: 0 }];
        assert_reject(&mut s, t5, "根 level≠1");
        // order 重复
        let mut t6 = gt_six_sections();
        t6[1].order = t6[0].order;
        assert_reject(&mut s, t6, "order 重复");
        // 空绑定
        let mut t7 = gt_six_sections();
        t7[3].requirement_ids = vec![];
        assert_reject(&mut s, t7, "节点零 REQ 绑定");
        // 幽灵 REQ
        let mut t8 = gt_six_sections();
        t8[4].requirement_ids = vec!["REQ-9999".into()];
        assert_reject(&mut s, t8, "引用不存在的 REQ");
        // 聚合失真（父聚合 ≠ 子并集）
        let t9 = vec![
            OutlineNode { id: "SEC-A".into(), parent_id: None, title: "A".into(), level: 1, order: 1,
                requirement_ids: vec!["REQ-0003".into(), "REQ-0004".into()], target_chars: 200, status: SectionStatus::Pending, dirty: false,
                content: None, content_attempt: 0 },
            OutlineNode { id: "SEC-B".into(), parent_id: Some("SEC-A".into()), title: "B".into(), level: 2, order: 2,
                requirement_ids: vec!["REQ-0003".into(), "REQ-0005".into()], target_chars: 100, status: SectionStatus::Pending, dirty: false,
                content: None, content_attempt: 0 },
        ];
        assert_reject(&mut s, t9, "父聚合与子并集不一致");
        // 空树
        assert_reject(&mut s, vec![], "空章节树");
    }

    /// 状态门：未抽条款（FILES_IMPORTED）不可开纲；DRAFTING+ 不可重开纲（S-5 面）。
    #[test]
    fn set_outline_state_gates() {
        let mut s = DomainStore::new();
        s.create_project("p", "c", "明标", "k-sec").unwrap();
        s.upsert_file("f", "招标文件", "sha-x", 1, 0, 5, 0, ParseStatus::Ok, "").unwrap();
        let err = s.set_outline(gt_six_sections()).unwrap_err();
        assert_eq!(err.code(), "E_STATE", "FILES_IMPORTED 无 REQ 不可开纲");
        // 推到 REQUIREMENTS_EXTRACTED 后放行
        s.set_requirements(vec![Requirement {
            id: "REQ-0003".into(), category: "资格条件".into(), clause_no: None,
            anchors: vec![], params: std::collections::HashMap::new(), source_file: "f".into(),
        }]).unwrap();
        let mut tree = gt_six_sections();
        tree.truncate(1);
        tree[0].requirement_ids = vec!["REQ-0003".into()];
        assert!(s.set_outline(tree).is_ok(), "REQUIREMENTS_EXTRACTED 可开纲");
    }

    /// 章节状态机：线性链 + 直通捷径 + 拒绝面零副作用 + reopen 唯一出口。
    #[test]
    fn section_status_chain_gates_and_reopen() {
        use SectionStatus::*;
        // 链形态
        assert_eq!(Pending.next_allowed(), &[Outlining]);
        assert_eq!(Validating.next_allowed(), &[Revising, Accepted], "零发现直通 + 有发现修订");
        assert_eq!(Accepted.next_allowed(), &[], "终态");
        // 推进 + 越级拒绝
        let mut s = store_with_reqs(15);
        s.set_outline(gt_six_sections()).unwrap();
        let v = s.content_version();
        let err = s.advance_section_status("SEC-0001", Drafted).unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "PENDING→DRAFTED 越级拒");
        assert_eq!(s.sections()[0].status, Pending, "拒绝零副作用");
        assert_eq!(s.content_version(), v, "拒绝不 bump");
        s.advance_section_status("SEC-0001", Outlining).unwrap();
        s.advance_section_status("SEC-0001", Drafting).unwrap();
        s.advance_section_status("SEC-0001", Drafted).unwrap();
        s.advance_section_status("SEC-0001", Validating).unwrap();
        s.advance_section_status("SEC-0001", Accepted).unwrap(); // 直通捷径
        assert_eq!(s.sections()[0].status, Accepted);
        // reopen 唯一出口（R-06）
        s.reopen_section("SEC-0001").unwrap();
        assert_eq!(s.sections()[0].status, Revising);
        assert!(s.sections()[0].dirty, " reopen 必打脏标记");
        // 非 ACCEPTED 的 reopen 拒
        let err = s.reopen_section("SEC-0002").unwrap_err();
        assert_eq!(err.code(), "E_UNPROCESSABLE", "reopen 仅适用终态");
        // 不存在
        assert!(matches!(
            s.advance_section_status("SEC-9999", Outlining),
            Err(ToolError::ENotFound { .. })
        ));
    }

    /// #180 摘要编码契约：存储序无关（按 (order, id) 排序——同一棵树不同
    /// Vec 插入序同摘要）；进出导出产物的字段变 → 摘要变；调度/账面字段变
    /// （status/dirty/content_attempt）→ 摘要不变（这正是「绑正文不绑账面」
    /// 的判据面）。
    #[test]
    fn sections_digest_encoding_contract() {
        let mk = |leaf_first: bool, content: Option<&str>, status: SectionStatus| {
            let root = OutlineNode { id: "SEC-0001".into(), parent_id: None, title: "根".into(),
                level: 1, order: 1, requirement_ids: vec!["REQ-0001".into()],
                target_chars: 100, status, dirty: true, content: None, content_attempt: 0 };
            let leaf = OutlineNode { id: "SEC-0002".into(), parent_id: Some("SEC-0001".into()),
                    title: "叶".into(), level: 2, order: 2,
                    requirement_ids: vec!["REQ-0001".into()], target_chars: 100,
                    status, dirty: true, content: content.map(str::to_string), content_attempt: 7 };
            if leaf_first { vec![leaf, root] } else { vec![root, leaf] }
        };
        // Vec 存储序无关：同一棵树（order 值不变）不同插入序 → 同摘要
        assert_eq!(sections_digest(&mk(false, Some("正文"), SectionStatus::Pending)),
                   sections_digest(&mk(true, Some("正文"), SectionStatus::Pending)));
        // 叶正文变 → 摘要变
        assert_ne!(sections_digest(&mk(false, Some("正文 v1"), SectionStatus::Pending)),
                   sections_digest(&mk(false, Some("正文 v2"), SectionStatus::Pending)));
        // 未起草（None）与空串是同一喂入面 → 同摘要（unwrap_or 空串契约）
        assert_eq!(sections_digest(&mk(false, None, SectionStatus::Pending)),
                   sections_digest(&mk(false, Some(""), SectionStatus::Pending)));
        // 调度/账面字段变（status/dirty/attempt）→ 摘要不变
        assert_eq!(sections_digest(&mk(false, Some("正文"), SectionStatus::Pending)),
                   sections_digest(&mk(false, Some("正文"), SectionStatus::Accepted)));
        // 稳定形态：sha256 hex 小写 64 字符；空树 = 空输入 sha256（常量）
        let d = sections_digest(&mk(false, None, SectionStatus::Pending));
        assert_eq!(d.len(), 64);
        assert!(d.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(sections_digest(&[]), hex::encode(Sha256::new().chain_update(b"").finalize()));
    }

    /// #199 审查回归锁：配额均衡「部分扣减 → 全部触底」的停滞路径。
    ///
    /// 旧守卫 `i > len*4 && remaining == diff` 只在【从未扣减】时成立；
    /// 一旦先扣减了一部分（remaining < diff），随后全部叶触底
    /// MIN_LEAF_CHARS，remaining 不再变化但守卫永假 → 100% CPU 死循环。
    /// 实测触发面：#199 按 kind 分发扩大抽取面后，e2e_artifacts /
    /// synth_tender_eval 在建纲步必现（叶片增多使下限和超过 target，
    /// 且大叶片头寸 < 差额）。
    ///
    /// 构造（target=12000，max_reqs_per_leaf=6，共 78 REQ / 25 叶）：
    /// · 技术参数 ×6 → 1 叶，配额 12000×6/78 ≈ 923（头寸 423）
    /// · 其他 ×72，分属 24 个「第X章」组，各 3 条 → 各 1 叶，
    ///   配额 12000×3/78 ≈ 461 → 抬到下限 500
    /// → quota_sum = 923 + 24×500 = 12923 > 12000（减法，diff=923 > 头寸 423）
    /// → 扣完 423 后全触底停滞：旧代码死循环，新代码整轮零进展退出。
    #[test]
    fn quota_rebalance_no_infinite_loop_on_partial_floor_stall() {
        let mk_req = |i: usize, cat: &str, locator: String| Requirement {
            id: format!("REQ-Q{i:03}"),
            category: cat.into(),
            clause_no: Some(format!("3.{i}")),
            anchors: vec![crate::Anchor {
                file: "A".into(),
                kind: "para".into(),
                locator,
                excerpt: format!("条款-{i} 摘录"),
            }],
            params: std::collections::HashMap::new(),
            source_file: "A".into(),
        };
        let mut reqs: Vec<Requirement> = (0..6)
            .map(|i| mk_req(i, "技术参数", "第三章 采购需求".into()))
            .collect();
        for ch in 1..=24u32 {
            for i in 0..3u32 {
                reqs.push(mk_req(
                    (ch * 10 + i) as usize,
                    "其他",
                    format!("第{ch}章 专用技术要求"),
                ));
            }
        }
        assert_eq!(reqs.len(), 78);
        let spec = OutlineSpec { target_total_chars: 12_000, max_reqs_per_leaf: 6 };
        let mut seq = 0;
        // 旧代码在本构造下死循环（测试超时即红）；新代码必须返回。
        let nodes = derive_outline(&reqs, spec, &mut |p| {
            seq += 1;
            format!("{p}-{seq}")
        });
        let leaves: Vec<&OutlineNode> = nodes.iter().filter(|n| n.level == 2).collect();
        assert_eq!(leaves.len(), 25, "1 技术参数叶 + 24 章叶");
        for l in &leaves {
            assert!(
                l.target_chars >= MIN_LEAF_CHARS,
                "叶配额不得低于下限：{}",
                l.target_chars
            );
        }
    }
}
