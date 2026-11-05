//! scoring_outline.rs —— 评分驱动大纲的域层构建（issue #252 第二步）。
//!
//! ## 定位
//!
//! 消费解析层（bid-parse `scoring_table` 专路）产出的评分表行 REQ
//! （`评分项` + params `{name, score, track, sub_items}`），构建：
//!
//! ```text
//! 评分表（第五章）→ 评审项（6 个）→ L1 章节（含分值）
//!                  → 评分细则（20 条）→ L2 小节（细则可追溯）
//!                  → 每条细则绑定相关 REQ（多对多路由）
//! 业绩（5 分）→ 独立响应位（业绩证明材料）
//! 价格（10 分）→ 独立响应位（报价文件）
//! ```
//!
//! ## 四条硬约束的落点（issue #252）
//!
//! 1. **双轨不是全塞附录**——技术参数经 [`L2_ROUTES`] 路由进评分章节
//!    （上行带宽→整体服务方案；完好率→日常维护/定期巡检；故障抢修→
//!    应急处理预案；15日历天→服务进度保障），**同时保留**其在
//!    技术参数组的绑定（多对多：REQ 可在多处，不丢）。
//! 2. **强制格式优先于评分驱动**——L1 排序把资格/商务/格式组置于评分
//!    主体之前、业绩材料殿后（对应第七章 16 项格式序的响应位次：
//!    一~十四 ≈ 行政组，十五 = 技术主体，十六 = 证明材料）。
//! 3. **多对多映射**——复用 #236 `ScoringMapping` 账本（落账面见
//!    `derive_and_set_outline`，`MappingSource::OutlineDerived`）。
//! 4. **禁止机械分块**——评分 L2 是语义单元（一条细则一节，不按
//!    REQ 数量切）；非评分组的块标题用条款号区间/页码区间
//!    （outline.rs `chunk_title`），不出现「（第N块）」。
//!
//! ## 标题规范化（[`scoring_l2_title`]）——如实登记的标定面
//!
//! 评分表细则原文是长句（「承诺维修服务响应时间：响应时间30分钟内得
//! 2分…」），章节标题需要规范短形。规则 NR1-NR6 里 NR1/NR2/NR5 前缀
//! 是通用文法变换；**NR3/NR4/NR6 是按 #251 人工复核金标准标定的窄域
//! 改写**（在真实政采评分表的常见表述上校准，非常识规则——PR 中明示，
//! 换语料需复核）。目标：L2 标题包含 #251 复核短形（验收 2 的可追溯面）。

use crate::outline::{OutlineNode, SectionStatus};
use crate::{Anchor, Requirement};

/// 评分表行 REQ 的域层视图（解析层 params 的消费面）。
#[derive(Debug, Clone)]
pub struct ScoringRowView {
    pub req_id: String,
    /// 评审项名（存储形——解析层已做窄域部首归一）
    pub name: String,
    pub score: f64,
    /// 技术评审 | 商务评审 | 价格评审
    pub track: String,
    /// 细则文本（①-⑳切分形）
    pub sub_items: Vec<String>,
    /// 首锚（映射 source_anchor——追溯：分数 → 评分项 → 规则原文页/行）
    pub anchor: Option<Anchor>,
}

/// 从 REQ 清单收集评分表行（params 面识别——`name`+`track` 是专路签名；
/// `sub_items` 可缺省：业绩/价格行无圈号细则）。
pub fn collect_scoring_rows(reqs: &[Requirement]) -> Vec<ScoringRowView> {
    reqs.iter()
        .filter(|r| r.category == "评分项")
        .filter(|r| {
            r.params.get("name").and_then(|v| v.as_str()).is_some()
                && r.params.get("track").and_then(|v| v.as_str()).is_some()
        })
        .map(|r| ScoringRowView {
            req_id: r.id.clone(),
            name: r
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            score: r.params.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
            track: r
                .params
                .get("track")
                .and_then(|v| v.as_str())
                .unwrap_or("技术评审")
                .to_string(),
            sub_items: r
                .params
                .get("sub_items")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            anchor: r.anchors.first().cloned(),
        })
        .collect()
}

// ---------------------------------------------------------------- 标题规范化

/// 评审项名 → 章节标题成分：「，」→「及」（标题内并列连词）。
///
/// 松岭实例：「项目重点、难点分析，应对措施及相关的合理化建议」→
/// 「项目重点、难点分析及应对措施及相关的合理化建议」（含 #251 复核
/// 短形「项目重点、难点分析及应对措施」）。
pub fn canonical_item_name(name: &str) -> String {
    name.replace('，', "及")
}

/// 细则文本 → L2 章节标题（规范短形）。
///
/// 规则（按序应用；NR3/NR4/NR6 是标定面——见模块文档）：
/// - **NR1**（通用）：「承诺X：Y」→「X承诺」——动词前缀名词化（标题
///   惯例：承诺作定语后置）；
/// - **NR2**（通用）：冒号截断——标题不带评分细则参数（Y 是打分说明）；
/// - **NR3**（标定）：「在项目实施所在地设立相应的」→「本地」；
/// - **NR4**（标定）：「对X、设备发生故障后的补救措施」→「X与故障补救
///   措施」（故障补救是响应面惯用词）；
/// - **NR5**（通用）：「有完整的」量化前缀剥离 + 「，」→「与」（标题内
///   并列改连词）；
/// - **NR6**（标定）：「A、B」共享语素并列（B 首二字出现在 A 尾四字中
///   ——「固定联系人、联系方式」）→「A及B」。
pub fn scoring_l2_title(clause: &str) -> String {
    let mut t: String = clause.chars().filter(|c| !c.is_whitespace()).collect();
    // NR1 / NR2：冒号处理
    if t.starts_with("承诺") {
        if let Some(colon) = t.find('：').or_else(|| t.find(':')) {
            let head: String = t["承诺".len()..colon].to_string();
            if !head.is_empty() {
                t = format!("{head}承诺");
            }
        }
    } else if let Some(colon) = t.find('：').or_else(|| t.find(':')) {
        t.truncate(colon);
    }
    // NR3：所在地设立 → 本地
    t = t.replace("在项目实施所在地设立相应的", "本地");
    // NR4：对X、设备发生故障后的补救措施 → X与故障补救措施
    if let Some(rest) = t.strip_prefix('对') {
        if let Some(a) = rest.strip_suffix("、设备发生故障后的补救措施") {
            if !a.is_empty() {
                t = format!("{a}与故障补救措施");
            }
        }
    }
    // NR5：量化前缀剥离 + 逗号并列改连词
    if let Some(rest) = t.strip_prefix("有完整的") {
        t = rest.to_string();
    }
    t = t.replace('，', "与");
    // NR6：共享语素并列 、→ 及（只动最后一个「、」，且要求 B 首二字
    // 出现在 A 尾四字中——「固定联系人、联系方式」；「项目重点、难点
    // 分析」不触发）
    if let Some(pos) = t.rfind('、') {
        let (a, b) = (&t[..pos], &t[pos + '、'.len_utf8()..]);
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        if a_chars.len() >= 2 && b_chars.len() >= 2 {
            let b_head: String = b_chars[..2].iter().collect();
            let a_tail: String = a_chars[a_chars.len().saturating_sub(4)..]
                .iter()
                .collect();
            if a_tail.contains(&b_head) {
                t = format!("{a}及{b}");
            }
        }
    }
    t
}

// ---------------------------------------------------------------- 双轨路由

/// REQ 锚面关键词 → 目标小节关键词（多对多）。
///
/// 依据：expected/outline.expected.json `dual_track_requirements`
/// （#251 复核的技术参数去向登记）+ 响应位惯例。路由是**加绑定不是搬
/// 家**——REQ 在原组的绑定保留（覆盖轨），评分章节获得同一 REQ（评分
/// 轨），双轨不丢面。
const L2_ROUTES: &[(&[&str], &[&str])] = &[
    // 链路与带宽（GR-001/002）→ 整体服务方案
    (
        &["上行带宽", "30M/s", "50M/s", "光纤链路", "平滑升级", "以太网", "汇聚设备", "双电源"],
        &["整体服务方案"],
    ),
    // 15日历天调测（GR-005）→ 服务进度保障方案
    (&["日历天", "调测"], &["服务进度保障方案"]),
    // 完好率（GR-003）→ 日常维护 + 定期巡检（双向路由）
    (&["完好率"], &["日常维护方案", "定期巡检方案"]),
    // 故障抢修/历时（GR-004）→ 应急处理预案
    (&["故障抢修", "故障历时", "360分钟"], &["应急处理预案"]),
    // 保密 → 保密措施
    (&["保密"], &["保密措施"]),
    // 量化承诺 → 服务承诺的两个 L2
    (&["响应时间"], &["维修服务响应时间承诺"]),
    (&["恢复时间"], &["故障恢复时间承诺"]),
    // 业绩 → 业绩证明材料位
    (&["业绩", "同类项目"], &["业绩"]),
    // 报价 → 报价文件位（价格评审响应：开标一览表/分项报价）
    (&["开标一览表", "报价"], &["报价文件"]),
];

/// 路由源池：可进评分章节的 REQ 类别。
///
/// 排除面（如实登记）：否决项/资格条件不路由（合规响应归「资格与响应」，
/// 评分章节不承载合规话术）；格式要求不路由（模板归格式组）；
/// 评分表行自身是 L2 的锚（已绑定，无需路由）。
fn route_pool(r: &Requirement) -> bool {
    match r.category.as_str() {
        "技术参数" | "其他" | "商务要求" => true,
        "评分项" => r.params.get("name").and_then(|v| v.as_str()).is_none(),
        _ => false,
    }
}

/// REQ 的路由检索面（锚 excerpt+locator + 条款号）。
fn route_haystack(r: &Requirement) -> String {
    let mut s = String::new();
    for a in &r.anchors {
        s.push_str(&a.excerpt);
        s.push_str(&a.locator);
    }
    if let Some(c) = &r.clause_no {
        s.push_str(c);
    }
    s
}

/// 把 REQ 按 [`L2_ROUTES`] 路由到目标小节（返回 命中的目标关键词集）。
fn route_targets(r: &Requirement) -> Vec<String> {
    if !route_pool(r) {
        return Vec::new();
    }
    let hay = route_haystack(r);
    let mut out: Vec<String> = Vec::new();
    for (keywords, targets) in L2_ROUTES {
        if keywords.iter().any(|k| hay.contains(k)) {
            for t in *targets {
                if !out.iter().any(|x| x == t) {
                    out.push((*t).to_string());
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- 构建产物

/// 评分章节（技术评审行 → L1 + L2）。
#[derive(Debug, Clone)]
pub struct ChapterBuild {
    /// L1 标题（规范名 + 分值：「服务方案（25分）」）
    pub title: String,
    /// L2 标题序列（细则规范短形）
    pub l2_titles: Vec<String>,
    /// 行 REQ id（L2 的锚绑定 + 映射的 scoring_req_id）
    pub req_id: String,
    pub score: f64,
}

/// 独立响应位（商务/价格评审行 → L1 叶）。
#[derive(Debug, Clone)]
pub struct SlotBuild {
    /// L1 标题：「业绩证明材料（5分）」/「报价文件（价格评审 10分）」
    pub title: String,
    pub req_id: String,
    pub track: String,
}

/// 建纲映射意图（derive_and_set_outline 落账面）。
#[derive(Debug, Clone)]
pub struct MappingIntent {
    pub scoring_req_id: String,
    pub section_id: String,
    pub anchor: Option<Anchor>,
}

/// 评分驱动构建产物（outline.rs 组树消费）。
#[derive(Debug, Clone, Default)]
pub struct ScoringBuild {
    pub chapters: Vec<ChapterBuild>,
    pub slots: Vec<SlotBuild>,
    pub mappings: Vec<MappingIntent>,
}

/// 从评分表行构建章节/槽位/映射意图（纯函数——确定性：按 REQ id 序）。
///
/// 返回 `None` = 无技术评审行（非评分表输入——outline.rs 走既有类目
/// 分组路径，行为与 #252 之前逐位一致）。
pub fn build_scoring_sections(reqs: &[Requirement]) -> Option<ScoringBuild> {
    let rows = collect_scoring_rows(reqs);
    let mut chapters: Vec<ChapterBuild> = Vec::new();
    let mut slots: Vec<SlotBuild> = Vec::new();
    for r in &rows {
        match r.track.as_str() {
            "技术评审" => chapters.push(ChapterBuild {
                title: format!("{}（{}分）", canonical_item_name(&r.name), fmt_score(r.score)),
                l2_titles: r.sub_items.iter().map(|s| scoring_l2_title(s)).collect(),
                req_id: r.req_id.clone(),
                score: r.score,
            }),
            "商务评审" => slots.push(SlotBuild {
                // 业绩的响应位是证明材料章节（合同扫描件）——评分表
                // criteria「投标文件中需附有业绩证明材料」
                title: format!("{}证明材料（{}分）", canonical_item_name(&r.name), fmt_score(r.score)),
                req_id: r.req_id.clone(),
                track: r.track.clone(),
            }),
            "价格评审" => slots.push(SlotBuild {
                // 价格分的响应位是报价文件（开标一览表/分项报价明细表），
                // 不是技术正文——独立响应位硬约束
                title: format!("报价文件（{} {}分）", canonical_item_name(&r.name), fmt_score(r.score)),
                req_id: r.req_id.clone(),
                track: r.track.clone(),
            }),
            _ => {}
        }
    }
    if chapters.is_empty() {
        return None;
    }
    Some(ScoringBuild { chapters, slots, mappings: Vec::new() })
}

/// 分值格式（整数不带小数点：25.0 → 25）。
fn fmt_score(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}

/// 把 REQ 路由到评分小节/槽位（加绑定，多对多）。
///
/// 返回 (l2_index → 追加 REQ ids, slot_index → 追加 REQ ids)：
/// `l2_reqs[(chapter_idx, l2_idx)]` / `slot_reqs[slot_idx]`。
pub fn route_requirements<'a>(
    reqs: &'a [Requirement],
    build: &ScoringBuild,
) -> (Vec<Vec<Vec<String>>>, Vec<Vec<String>>) {
    let mut l2_reqs: Vec<Vec<Vec<String>>> = build
        .chapters
        .iter()
        .map(|c| vec![Vec::new(); c.l2_titles.len()])
        .collect();
    let mut slot_reqs: Vec<Vec<String>> = vec![Vec::new(); build.slots.len()];
    for r in reqs {
        for target in route_targets(r) {
            // 目标关键词匹配 L2 标题（contains 双向——目标短语 ⊆ 标题）
            let mut hit = false;
            for (ci, ch) in build.chapters.iter().enumerate() {
                for (li, lt) in ch.l2_titles.iter().enumerate() {
                    if lt.contains(target.as_str()) || target.contains(lt.as_str()) {
                        l2_reqs[ci][li].push(r.id.clone());
                        hit = true;
                    }
                }
            }
            for (si, s) in build.slots.iter().enumerate() {
                if s.title.contains(target.as_str()) {
                    slot_reqs[si].push(r.id.clone());
                    hit = true;
                }
            }
            let _ = hit; // 未命中目标（规则表与构建面脱节）——静默跳过，
                         // 路由是加绑定不是覆盖，不命中不破坏任何面
    }
    }
    (l2_reqs, slot_reqs)
}

// ---------------------------------------------------------------- 组分类与排序

/// L1 组的配额/序类（#252 双轨 + 格式优先的落点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupClass {
    /// 评分主体（6 章节——格式位十五：项目实施方案、质量保证及售后服务承诺）
    Scoring,
    /// 独立响应位（报价文件=格式位四 / 业绩证明材料=格式位十六）
    Slot,
    /// 技术支撑（技术参数=偏离表位 + 技术类主题组——评分主体的伴随面）
    TechSupport,
    /// 行政（资格/格式/合同/程序——格式位一~十四的响应位）
    Admin,
    /// 其余（无语义分类的残余组）
    Residual,
}

/// 行政组关键词（与验收测试 ADMIN_L1_KEYWORDS 同源口径 + 保证金）。
const ADMIN_KEYWORDS: &[&str] = &[
    "资格", "格式", "合同", "须知", "公告", "商务", "其他条款", "补遗", "澄清",
    "纪律", "附件", "获取", "开标", "报价", "业绩", "投标文件", "保证金",
];

/// 技术支撑组关键词（技术参数偏离面 + 技术类主题组）。
const TECH_SUPPORT_KEYWORDS: &[&str] =
    &["技术参数", "技术方案要求", "服务与售后要求", "质量标准要求"];

/// 非评分组的类判（评分章节/槽位在构建面直接标类）。
pub fn classify_group(title: &str) -> GroupClass {
    if TECH_SUPPORT_KEYWORDS.iter().any(|k| title.contains(k)) {
        return GroupClass::TechSupport;
    }
    if ADMIN_KEYWORDS.iter().any(|k| title.contains(k)) {
        return GroupClass::Admin;
    }
    GroupClass::Residual
}

/// L1 组的文档序（小者在前；类内保持构建序——确定性）。
///
/// 序的依据（硬约束②：强制格式优先）：第七章 16 项格式序的响应位次——
/// 资格与响应（一~三：承诺书/资格承诺/授权委托）→ 报价文件（四）→
/// 行政/格式组（五~十四：商务承诺/偏离表/各类声明）→ 技术支撑
/// （偏离与参数）→ 评分主体（十五：技术正文）→ 业绩证明材料（十六）→
/// 残余组殿后。
pub fn group_order_key(class: GroupClass, title: &str) -> u8 {
    match class {
        GroupClass::Admin => {
            if title.contains("资格与响应") {
                0 // 格式位一~三 + 资格响应——居首
            } else {
                2 // 其余行政组（格式五~十四面）
            }
        }
        GroupClass::Slot => {
            if title.contains("报价文件") {
                1 // 格式位四：分项报价明细表
            } else {
                6 // 业绩证明材料——格式位十六殿后于技术主体
            }
        }
        GroupClass::TechSupport => 3,
        GroupClass::Scoring => 4, // 格式位十五：技术主体
        GroupClass::Residual => 7,
    }
}

// ---------------------------------------------------------------- 配额

/// 配额政策（评分输入在场时的分类预算——占总量的比例）。
///
/// 论证（issue #252「配额按分值分配」+ PR 面要求自证比例）：
/// - **评分主体 65%**：85 分主观技术分是评委裁量空间，「每有一项表述
///   存在缺陷的扣1分」的扣分制要求每个细则充分展开；20 个 L2 平均
///   ~27,400 字（最小章服务承诺 4 细则各 ~12,900 字 ≥ 10,000 下限）。
///   不取 85%（分值占比）的原因：业绩/价格 15 分是客观分（证明材料/
///   报价表，不需长文），按分值线性会给行政面挤兑空间。
/// - **技术支撑 10%**：技术参数逐条响应（偏离表 + 参数说明），表格化
///   内容篇幅中等。
/// - **行政 20%**：资格/格式/合同是模板化填空（每行政条款 ≈ 千字级），
///   松岭实测 162 条行政类 REQ。
/// - **其余 5%**：程序性条款（保证金/纪律等）简短响应。
pub struct QuotaPolicy {
    pub scoring_share: f64,
    pub tech_support_share: f64,
    pub admin_share: f64,
    pub residual_share: f64,
    /// 独立响应位固定配额（从行政预算内扣）——证明材料/报价表是表格位
    pub slot_fixed_chars: u64,
}

impl Default for QuotaPolicy {
    fn default() -> Self {
        QuotaPolicy {
            scoring_share: 0.65,
            tech_support_share: 0.10,
            admin_share: 0.20,
            residual_share: 0.05,
            slot_fixed_chars: 2_000,
        }
    }
}

// ---------------------------------------------------------------- 节点构建

/// 评分章节/槽位 → 节点子树（含路由绑定）。返回 (nodes, mappings)。
///
/// `next_id` 由调用方注入（store 的 id 序列器——派生与 id 解耦）。
/// L2 绑定 = 行 REQ（锚） + 路由 REQ（多对多）；槽位绑定 = 行 REQ +
/// 路由 REQ。父聚合面（requirement_ids 并集）由本函数保证。
pub fn scoring_nodes(
    build: &ScoringBuild,
    routed_l2: &[Vec<Vec<String>>],
    routed_slots: &[Vec<String>],
    next_id: &mut dyn FnMut(&str) -> String,
) -> (Vec<OutlineNode>, Vec<MappingIntent>) {
    let mut nodes: Vec<OutlineNode> = Vec::new();
    let mut mappings: Vec<MappingIntent> = Vec::new();
    let mut order: u32 = 0;
    // —— 评分章节（L1 + L2）——
    for (ci, ch) in build.chapters.iter().enumerate() {
        let parent_id = next_id("SEC");
        order += 1;
        let mut l2_nodes: Vec<OutlineNode> = Vec::new();
        for (li, l2_title) in ch.l2_titles.iter().enumerate() {
            let mut req_ids = vec![ch.req_id.clone()];
            for rid in &routed_l2[ci][li] {
                if !req_ids.contains(rid) {
                    req_ids.push(rid.clone());
                }
            }
            l2_nodes.push(OutlineNode {
                id: next_id("SEC"),
                parent_id: Some(parent_id.clone()),
                title: l2_title.clone(),
                level: 2,
                order: 0, // 组树后统一编序（outline.rs 按最终 L1 序重排）
                requirement_ids: req_ids,
                target_chars: 0, // 配额在 outline.rs 分类预算面分配
                status: SectionStatus::Pending,
                dirty: false,
                content: None,
                content_attempt: 0,
            });
        }
        let agg_reqs: Vec<String> = l2_nodes
            .iter()
            .flat_map(|n| n.requirement_ids.iter().cloned())
            .fold(Vec::new(), |mut acc, rid| {
                if !acc.contains(&rid) {
                    acc.push(rid);
                }
                acc
            });
        nodes.push(OutlineNode {
            id: parent_id.clone(),
            parent_id: None,
            title: ch.title.clone(),
            level: 1,
            order,
            requirement_ids: agg_reqs,
            target_chars: 0,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        });
        nodes.append(&mut l2_nodes);
        mappings.push(MappingIntent {
            scoring_req_id: ch.req_id.clone(),
            section_id: parent_id,
            anchor: None, // 行 REQ 锚在落账面回填（derive_and_set_outline）
        });
    }
    // —— 独立响应位（L1 叶——无子节点，正文直住）——
    for (si, slot) in build.slots.iter().enumerate() {
        let mut req_ids = vec![slot.req_id.clone()];
        for rid in &routed_slots[si] {
            if !req_ids.contains(rid) {
                req_ids.push(rid.clone());
            }
        }
        let id = next_id("SEC");
        order += 1;
        nodes.push(OutlineNode {
            id: id.clone(),
            parent_id: None,
            title: slot.title.clone(),
            level: 1,
            order,
            requirement_ids: req_ids,
            target_chars: 0,
            status: SectionStatus::Pending,
            dirty: false,
            content: None,
            content_attempt: 0,
        });
        mappings.push(MappingIntent {
            scoring_req_id: slot.req_id.clone(),
            section_id: id,
            anchor: None,
        });
    }
    (nodes, mappings)
}

// ---------------------------------------------------------------- 测试
#[cfg(test)]
mod tests {
    use super::*;

    fn scoring_req(id: &str, name: &str, score: f64, track: &str, subs: &[&str]) -> Requirement {
        Requirement {
            id: id.to_string(),
            category: "评分项".to_string(),
            clause_no: Some(format!("综合评分表-{id}")),
            anchors: vec![Anchor {
                file: "A".into(),
                kind: "table".into(),
                locator: "第25页 行8".into(),
                excerpt: format!("{name}（{score}分 {track}）：…"),
            }],
            params: [
                ("name".to_string(), serde_json::json!(name)),
                ("score".to_string(), serde_json::json!(score)),
                ("track".to_string(), serde_json::json!(track)),
                ("sub_items".to_string(), serde_json::json!(subs)),
            ]
            .into_iter()
            .collect(),
            source_file: "A".into(),
        }
    }

    fn body_req(id: &str, cat: &str, excerpt: &str) -> Requirement {
        Requirement {
            id: id.to_string(),
            category: cat.to_string(),
            clause_no: None,
            anchors: vec![Anchor {
                file: "A".into(),
                kind: "pdf_line".into(),
                locator: "第17页 行3".into(),
                excerpt: excerpt.to_string(),
            }],
            params: std::collections::HashMap::new(),
            source_file: "A".into(),
        }
    }

    /// 松岭评分表 8 行 → 6 章节 + 2 槽位 + 20 L2（结构面验收 1-3）。
    #[test]
    fn build_six_chapters_two_slots() {
        let reqs = vec![
            scoring_req("R1", "服务方案", 25.0, "技术评审", &[
                "整体服务方案（对本项目建设、运行维护服务方案）",
                "服务人员管理规范",
                "服务阶段投入本项目的人力安排计划",
                "服务进度保障方案",
                "服务时间节点安排",
            ]),
            scoring_req("R2", "项目重点、难点分析，应对措施及相关的合理化建议", 10.0, "技术评审", &[
                "项目重点、难点分析",
                "项目实施过程重点、难点应对措施、解决方案及合理化建议",
            ]),
            scoring_req("R3", "保密工作", 10.0, "技术评审", &["保密措施", "保密承诺"]),
            scoring_req("R4", "服务承诺", 8.0, "技术评审", &[
                "在项目实施所在地设立相应的技术支持及售后服务机构（提供有效的办公场所证明材料）",
                "对备品配件、设备发生故障后的补救措施",
                "承诺维修服务响应时间：响应时间30分钟内得2分，1小时内得1分，超过1小时不得分",
                "承诺故障恢复时间：接到故障报修后，派出工程师到现场并在2小时内解决问题故障，得2分，超过2小时不得分",
            ]),
            scoring_req("R5", "质量保证", 20.0, "技术评审", &[
                "服务质量保证体系及保证措施",
                "有完整的服务质量自检措施，整改保障措施",
                "日常维护方案",
                "定期巡检方案",
            ]),
            scoring_req("R6", "应急方案", 12.0, "技术评审", &[
                "人员安排",
                "应急处理预案",
                "固定联系人、联系方式",
            ]),
            scoring_req("R7", "业绩", 5.0, "商务评审", &[]),
            scoring_req("R8", "价格评审", 10.0, "价格评审", &[]),
        ];
        let build = build_scoring_sections(&reqs).expect("技术评审在场");
        assert_eq!(build.chapters.len(), 6);
        assert_eq!(build.slots.len(), 2);
        // 章节标题（验收 1 的规范形——含 #251 复核短形）
        assert_eq!(build.chapters[0].title, "服务方案（25分）");
        assert!(build.chapters[1].title.contains("项目重点、难点分析及应对措施"));
        assert_eq!(build.chapters[2].title, "保密工作（10分）");
        assert_eq!(build.chapters[3].title, "服务承诺（8分）");
        assert_eq!(build.chapters[4].title, "质量保证（20分）");
        assert_eq!(build.chapters[5].title, "应急方案（12分）");
        // L2 标题（验收 2：细则可追溯——含复核短形）
        let l2_all: Vec<String> = build.chapters.iter().flat_map(|c| c.l2_titles.clone()).collect();
        assert_eq!(l2_all.len(), 20);
        assert!(l2_all[0].contains("整体服务方案"));
        // NR1：承诺X：→X承诺
        assert!(build.chapters[3].l2_titles[2].contains("维修服务响应时间承诺"));
        assert!(build.chapters[3].l2_titles[3].contains("故障恢复时间承诺"));
        // NR3/NR4/NR5/NR6
        assert!(build.chapters[3].l2_titles[0].contains("本地技术支持及售后服务机构"));
        assert_eq!(build.chapters[3].l2_titles[1], "备品配件与故障补救措施");
        assert_eq!(build.chapters[4].l2_titles[1], "服务质量自检措施与整改保障措施");
        assert_eq!(build.chapters[5].l2_titles[2], "固定联系人及联系方式");
        // 槽位（验收 3：独立响应位）
        assert_eq!(build.slots[0].title, "业绩证明材料（5分）");
        assert_eq!(build.slots[1].title, "报价文件（价格评审 10分）");
    }

    /// 双轨路由：技术参数 REQ 多对多进评分章节（验收 8 的绑定面）。
    #[test]
    fn routes_tech_params_into_scoring_l2s() {
        let reqs = vec![
            scoring_req("R1", "服务方案", 25.0, "技术评审", &[
                "整体服务方案", "服务人员管理规范", "服务阶段投入本项目的人力安排计划", "服务进度保障方案", "服务时间节点安排",
            ]),
            scoring_req("R5", "质量保证", 20.0, "技术评审", &[
                "服务质量保证体系及保证措施", "服务质量自检措施与整改保障措施", "日常维护方案", "定期巡检方案",
            ]),
            scoring_req("R6", "应急方案", 12.0, "技术评审", &["人员安排", "应急处理预案", "固定联系人及联系方式"]),
            scoring_req("R7", "业绩", 5.0, "商务评审", &[]),
            scoring_req("R8", "价格评审", 10.0, "价格评审", &[]),
            // GR-001/002：链路带宽 → 整体服务方案
            body_req("T1", "技术参数", "提供上行带宽30M/s光纤链路295条。"),
            // GR-003：完好率 → 日常维护 + 定期巡检（多对多双向）
            body_req("T2", "技术参数", "光缆线路完好率（芯公里）≥95%"),
            // GR-004：故障抢修 → 应急处理预案
            body_req("T3", "技术参数", "故障抢修的原则：故障历时≤360分钟"),
            // GR-005：15日历天 → 服务进度保障方案
            body_req("T4", "技术参数", "从合同签订之日起15日历天内调测完毕"),
            // 否决项不路由（合规面归资格与响应）
            body_req("V1", "否决项", "串通投标情形的投标将被否决，涉及保密要求"),
        ];
        let build = build_scoring_sections(&reqs).expect("构建");
        let (l2_reqs, slot_reqs) = route_requirements(&reqs, &build);
        // T1 → 服务方案.1.1 整体服务方案
        assert!(l2_reqs[0][0].contains(&"T1".to_string()), "30M/s → 整体服务方案");
        // T2 → 质量保证.5.3 日常维护 + 5.4 定期巡检
        assert!(l2_reqs[1][2].contains(&"T2".to_string()), "完好率 → 日常维护");
        assert!(l2_reqs[1][3].contains(&"T2".to_string()), "完好率 → 定期巡检");
        // T3 → 应急方案.6.2 应急处理预案
        assert!(l2_reqs[2][1].contains(&"T3".to_string()), "故障抢修 → 应急处理预案");
        // T4 → 服务方案.1.4 服务进度保障方案
        assert!(l2_reqs[0][3].contains(&"T4".to_string()), "15日历天 → 服务进度保障");
        // V1 不路由
        for ch in &l2_reqs {
            for l2 in ch {
                assert!(!l2.contains(&"V1".to_string()), "否决项不进评分章节");
            }
        }
        let _ = slot_reqs;
    }

    /// 组分类与排序（硬约束②：格式位次）。
    #[test]
    fn classify_and_order() {
        assert_eq!(classify_group("资格与响应"), GroupClass::Admin);
        assert_eq!(classify_group("格式要求"), GroupClass::Admin);
        assert_eq!(classify_group("合同条款响应"), GroupClass::Admin);
        assert_eq!(classify_group("技术参数"), GroupClass::TechSupport);
        assert_eq!(classify_group("服务与售后要求"), GroupClass::TechSupport);
        assert_eq!(classify_group("质量标准要求"), GroupClass::TechSupport);
        assert_eq!(classify_group("某残余组"), GroupClass::Residual);
        // 序：资格与响应 < 报价文件slot < 行政 < 技术支撑 < 评分 < 业绩slot < 残余
        let keys = [
            group_order_key(GroupClass::Admin, "资格与响应"),
            group_order_key(GroupClass::Slot, "报价文件（价格评审 10分）"),
            group_order_key(GroupClass::Admin, "格式要求"),
            group_order_key(GroupClass::TechSupport, "技术参数"),
            group_order_key(GroupClass::Scoring, "服务方案（25分）"),
            group_order_key(GroupClass::Slot, "业绩证明材料（5分）"),
            group_order_key(GroupClass::Residual, "其他条款"),
        ];
        assert_eq!(keys, [0, 1, 2, 3, 4, 6, 7]);
        assert!(keys.is_sorted(), "文档序单调（格式位次）");
    }

    /// 非评分表输入（corpus-A docx / synth 路径）→ None（回退既有路径）。
    #[test]
    fn non_scoring_input_falls_back() {
        let reqs = vec![
            body_req("A1", "技术参数", "带宽 30M/s"),
            body_req("A2", "否决项", "否决情形"),
        ];
        assert!(build_scoring_sections(&reqs).is_none());
    }
}
