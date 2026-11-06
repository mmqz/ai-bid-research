//! 确定性条款抽取（extract_requirements 的「确定性先行」层——mcp-contract §3）。
//!
//! M1 纪律：零模型调用。类别判定 = 章节上下文映射（章/节标题 → 类别），
//! 参数抽取 = 正则槽位（型号/金额/日期），条款号 = 编号头解析 + 表格行位。
//! 模型补全面（语义归类、陈述规范化）属 M2+——本层输出即 REQ 的稳定基座。
//!
//! 类别映射（章节上下文——政采范本结构惯例，规则钉死可审计）：
//!   评标办法→评分项；技术需求→技术参数；商务要求→商务要求；
//!   须知+资格→资格条件；须知+否决→否决项；须知+编制→格式要求；其余→其他。

use crate::clause;
use crate::{Block, BlockKind, ParsedFile};
use bid_domain::{Anchor, Requirement};
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

/// 型号谱系正则：字母开头（≥1 位）+ 连字符分段（防误配纯数字编号如「附件 7-3」）。
/// 覆盖实测谱系：GW-8000-T4 / S5735-L48T4X-A1 / USG6650E-AC550 / IPC-4T-IR-D2 / NVR-64H-8P / DCS-4000。
fn model_regex() -> Regex {
    Regex::new(r"[A-Z][A-Z0-9]*(?:-[A-Z0-9]+)+").unwrap()
}

/// 纯 NFKC（不剥空白）——**只用于章节/小节识别**，不改任何落库数据。
///
/// 真实政采平台 PDF 的文本层用康熙部首区编码汉字（「第⼀章」的「⼀」
/// 是 U+2F00 不是 U+4E00），导致章名正则匹配不到 → 全部落「其他」。
/// 只在**匹配瞬间**归一化，anchor/text 保持原文——
/// 这样块对齐（locator/text 精确比较）不受影响。
fn nfkc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkc().collect()
}

/// 兼容入口：按【文件名前缀】推断业务类别（既有夹具与外部直接调用沿用）。
///
/// 生产路径应改调 [`extract_requirements_with_kind`] —— 用导入时登记的真实
/// 业务 kind，不依赖文件名。此入口保留是为了不破坏既有测试夹具。
pub fn extract_requirements(files: &[ParsedFile]) -> Vec<Requirement> {
    extract_dispatch(files, &|_name| None)
}

/// 生产入口：接受「文件名 → 业务 kind」查询函数（来自 `TenderFile.kind`）。
///
/// #196：分发必须按业务 kind，不能按文件名前缀——真实用户文件不带 `A-` 前缀，
/// 旧实现会把它们静默丢弃（抽取 0 条而工具仍返回 ok:true）。
///
/// `kind_of` 返回 `None` 时退回前缀推断（兼容夹具）。
pub fn extract_requirements_with_kind<F>(files: &[ParsedFile], kind_of: F) -> Vec<Requirement>
where
    F: Fn(&str) -> Option<String>,
{
    extract_dispatch(files, &kind_of)
}

fn extract_dispatch(
    files: &[ParsedFile],
    kind_of: &dyn Fn(&str) -> Option<String>,
) -> Vec<Requirement> {
    // 【真实文件驱动修复】PDF 文本层的康熙部首污染（2026-11 实测）：
    //
    // 真实招标 PDF（黑龙江政采平台导出）文本层用了 Unicode 康熙部首区：
    //   「第⼀章」的「⼀」是 U+2F00 KANGXI RADICAL ONE，不是 U+4E00「一」
    // 后果：re_chapter 匹配不到 → 章名全空 → 全部落「其他」。
    //
    // 【修法演进】（两次回归的教训）：
    //   ① 首版：对 ParsedFile 全量归一化 → 打挂水科红线（L1 0.22/0.97）。
    //      原因：anchor 来自归一化 blocks、`svc.parsed()` 是原始 blocks，
    //      块对齐 `locator == && text ==` 全对不上。
    //   ② 本版：**不动任何数据**，只在【章节识别】时用归一化副本。
    //      `chapter_of()` 内部先 nfkc 再匹配；anchor/text 全是原文。
    //      → 水科/合成语料零影响（它们本来没有部首污染）。
    let mut reqs: Vec<Requirement> = Vec::new();
    let mut seq = 0usize;
    for f in files {
        // 主文件/补遗按块形态分发：DOCX 块（Para/TableRow）→ docx 版；PDF 块
        //（PdfLine/OcrLine）→ pdf 版（真实招标文件主文件为 PDF 的常态面）
        let is_pdf_blocks = f
            .blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine));
        // 【#196 修复】分发依据：**业务 kind 优先，文件名前缀只作兜底**。
        //
        // 原实现只认文件名前缀（A-/C-/B1/B2），导致任何真实用户文件
        // （如「光纤租赁费招标文件.docx」）落入 `_ => {}` **静默零抽取**，
        // 而工具仍返回 ok:true、状态推进到 REQUIREMENTS_EXTRACTED——
        // 违反 issue #9「缺失不得伪装成功」纪律。
        //
        // 前缀是测试夹具的命名约定，不应泄漏为生产判据。
        //
        // 【#199 审查修复】kind 值域对齐导入面 TENDER_FILE_KINDS
        // （招标文件|附件|图纸|补遗|答疑|评分办法|格式模板）。
        // 初版把 is_main 写成封闭白名单（招标文件|补遗|澄清）——
        // 「答疑/评分办法/格式模板」三个合法 kind 三分支全不命中 →
        // 静默跳过（#196 要修的形态换了马甲回来），且这些 kind 存在时
        // 前缀兜底（kind.is_none() 分支）也不生效。改为**反向清单**：
        // 只有明确无条款面的 kind（附件/图纸/证据）才跳过，其余登记
        // kind（含未来新增）一律主抽取——默认可见（fail-visible），
        // 与工具面 ZERO_REQUIREMENTS 兜底告警配套。
        // 【#271】「格式模板」加入无条款面清单：格式件是**投标人填写的
        // 官方模板**（声明函/偏离表/承诺书——填空表非条款源），条款真源
        // 始终是主件（招标文件格式章 REQ 已携带「按格式N填写」要求）。
        // 模板内容不产 REQ/不进矩阵；解析产物仍进 parsed 累积（块面可经
        // get_document_chunks 读——起草时可参考模板结构）。
        let kind = kind_of(&f.name);
        let is_attachment = matches!(
            kind.as_deref(),
            Some("附件") | Some("图纸") | Some("证据") | Some("格式模板")
        );
        let is_b1 = matches!(kind.as_deref(), Some("报价文件")) || f.name.starts_with("B1");
        let is_b2 = matches!(kind.as_deref(), Some("扫描件")) || f.name.starts_with("B2");
        if is_attachment {
            // 附件/图纸/证据：无条款抽取面（证据入库走 chunk 索引）。
        } else if is_b1 {
            extract_b1_lines(f, &mut reqs, &mut seq);
        } else if is_b2 {
            extract_b2_ocr(f, &mut reqs, &mut seq);
        } else if is_pdf_blocks {
            // 招标文件/补遗/答疑/评分办法/格式模板（及 kind 缺省的
            // 无前缀兜底——真实文件名不再静默丢弃）
            extract_main_pdf(f, &mut reqs, &mut seq);
        } else {
            extract_main_docx(f, &mut reqs, &mut seq);
        }
    }
    // #52 排除面统一复检（引擎排除面——精确率攻坚）：
    // 否决类 REQ 命中形态锁排除面 → 改判「评分项/其他」。
    // - 不删 REQ——L1 抽取面不受影响（召回红线 L1 天然保住）；
    // - 统一后置——覆盖全部分类路径（强词/语境传播/审查表/前附表/
    //   章节上下文映射），两侧（PDF/DOCX）单点接入零遗漏；
    // - 排除原因写入 params["_veto_excluded"]（可审计——下游可追溯
    //   为什么这条不是否决项；issue #52 纪律：每条规则要能解释）。
    // 零误伤已验证：排除面全集对 GT 1752 true 命中 0 条
    // （scripts/tp_collateral_scan.py + fp_attribution.py）。
    for r in reqs.iter_mut() {
        if r.category != "否决项" {
            continue;
        }
        let joined: String = r.anchors.iter().map(|a| a.excerpt.as_str()).collect();
        if let Some(reason) = crate::veto_lexicon::exclusion_reason(&joined) {
            r.category = if reason.starts_with("评分公式") {
                "评分项"
            } else {
                "其他"
            }
            .to_string();
            r.params.insert(
                "_veto_excluded".to_string(),
                serde_json::Value::String(reason.to_string()),
            );
        }
    }
    reqs
}

/// PDF 主文件/补遗：行级解析——章节标题行（第X章/中文序号）更新上下文，
/// 编号条款行 → REQ（分类同 docx 版）。
/// P2 真实文件驱动扩展（风险不对称：漏否决/资格条款 = 废标风险 > 误报可过滤——
/// M1 层宁多勿漏，精化归 M2 语义层）：
/// - 强否决短语行（将被否决/视为无效/取消中标资格/没收…）→ 否决项（无编号也抽）
/// - 资格面行（资质等级/资格后审/资质证书/营业执照…）→ 资格条件
/// - 评分细则行（评分办法章内 "N、报价：65 分"）→ 评分项
/// - 续行窗 1 行：跨行条款还原（同 extract_b2_ocr 纪律）
///
/// P0-A（issue #35）：PDF 行级碎片修复——基线 PDF 组 L1 0.4680/L2 0.1148：
/// - **段落重组**：行尾无终结标点 → 与下一行同段（pdftotext 按物理行切块，
///   一条否决条款被切成两三行后词表跨行失配——「…招标人」/「拒收投标文件。」）
/// - **否决词表扩展**（missed 样例驱动）：拒收/不予受理/不予接收/不接受/
///   否决条件/取消中标资格/按无效标处理 等（原 8 词面漏「A01-GT-016 招标人
///   拒收投标文件」「A01-GT-021 不予受理」类）
/// - **例外从句剥离**：除…外 不参与强词匹配（精确率守护，句法同 DOCX 侧）
/// - **语境传播**：总括句（下列情形之一的…）→ 列举条目（（N）/N、/⑻ 圈号）
///   继承否决语义；新编号条款终止
/// - 段落级 REQ 的 anchors = 构成段落的全行块（excerpt 保真不改写）
/// 裸数字小节头形态判定（#68 阶段 4·D）：
/// 段文本 = 数字头（1-2 位 + 可选 ./．/空格）+ 2-12 个连续 CJK、无尾标点、
/// 无括号/分号——「7．合同授予」「8定标及报告」形态。
/// 以**段**判（短段无尾标点——续行拼进来就不再是头）。
fn pend_head_form(
    i: usize,
    para_text: &dyn Fn(usize, usize) -> String,
    para_end: &[usize],
    blocks: &[Block],
) -> bool {
    if !matches!(blocks[i].kind, BlockKind::PdfLine | BlockKind::OcrLine) {
        return false;
    }
    let para = para_text(i, para_end[i]);
    let re_head = regex::Regex::new(r"^[0-9]{1,2}[.．、]?\s*[一-龥]{2,12}$").unwrap();
    if !re_head.is_match(para.trim()) {
        return false;
    }
    // 尾标点 / 括号 / 分号 → 条目而非标题；
    // #68 阶段 4（M1）：审查条目形排除——「投标范围符合招标文件规定」恰
    // 12 字无尾标点，形态与标题全同（missed 实锤：A-04 块 991 被当小节头
    // 吞掉）。含符合/一致/配备/报价等审查谓词的是条目不是标题。
    let last = para.trim().chars().last().unwrap_or('。');
    let itemish = regex::Regex::new(
        r"符合|一致|配备|应当|应与|不得|未|报价|工期|质量|有效期|信誉|业绩|名称|数量",
    ).unwrap();
    !"。；，：！？".contains(last)
        && !para.contains('（')
        && !para.contains('(')
        && !itemish.is_match(&para)
}

fn extract_main_pdf(f: &ParsedFile, reqs: &mut Vec<Requirement>, seq: &mut usize) {
    // #218 任务 3：条款号双形态——多级点分（4.2/2.3.4）+ 中文顿号（1、）。
    // 顿号是 level-1 列举编号（numbering.rs ArabicDot——「1、提供上行带宽30M/s
    // 光纤链路295条。」式技术/商务列举，实机测试漏抽面）；1-3 位对齐
    // numbering 面（「2026、」年份形不入）。
    let re_clause = Regex::new(r"^(\d+(?:\.\d+)+|\d{1,3}、)\s*").unwrap();
    let re_dunhao_list = Regex::new(r"^\d{1,3}、").unwrap();
    // 真实文件驱动修正（P2）：真实水利招标文件用阿拉伯数字章节（"第2章 投标人
    // 须知"）——与合成夹具/政采范本的中文数字（"第三章"）并存，双通道容忍空格
    let re_chapter = Regex::new(r"^第\s*([0-9一二三四五六七八九十百]+)\s*章").unwrap();
    let re_subsection = Regex::new(r"^[一二三四五六七八九十]+、").unwrap();
    // #272 跨平台形态：「第X部分/篇」顶层（询价/谈判类）——只在全文
    // 无「第X章」标记的文档激活（文档级方案仲裁，见 structure.rs）。
    // 黑龙江第六章合同内的「第三部分 合同专用条款」分部行不会冲掉章上下文
    let re_part_head = Regex::new(r"^第\s*([0-9一二三四五六七八九十百]+)\s*(部分|篇)").unwrap();
    let re_model = model_regex();
    let re_amount = Regex::new(r"(\d+(?:\.\d+)?)\s*万\s*元").unwrap();
    let re_date = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
    // 强否决短语（判定动词面——正文提及"否决"的弱引用不算）。
    // P0-A 扩展（#35 missed 样例驱动）：拒收/不予受理/不予接收/不接受/
    // 否决条件/取消中标资格/按无效标处理/视为串通 等
    let re_veto = Regex::new(concat!(
        "将被否决|否决投标处理|否决所有投标|否决条件|否决其投标|否决该投标|",
        "视为无效|无效投标|按无效标处理|作无效标处理|认定为无效|",
        "取消其中标|取消中标资格|取消投标资格|取消其投标资格|没收其投标保证金|废标|",
        "拒收|不予受理|不予接收|不予接受|不予开标|有权拒收|有权否决|",
        "不接受[^，。；]{0,12}(投标|报价|备选|联合体)|不接受联合体|",
        "弄虚作假|串通投标|相互串通|骗取中标|行贿|",
        "不良行为|失信|黑名单|行政处罚|刑事处罚|严重违约|重大工程质量问题|",
        "允许他人以本单位名义|将被拒绝|不允许偏离|实质性要求和条件|",
        "利害关系[^，。；]{0,32}不得参加投标|不接受[^，。；]{0,20}(文件|格式)|",
        // #68 阶段 4（PDF L2 残差 176 条解剖——B/A/D 族 missed 样例驱动）：
        // 「投标人拒绝延长的，其投标失效」——失效是处置结果；
        // 「导致其投标被拒绝且投标文件被退回」——被拒绝被动形态（非「将被拒绝」）；
        // 「1.4.2是否接受联合体投标☑不接受」——前附表联合体不接受（GT 资格性否决）。
        "投标失效|投标被拒绝|",
        "联合体投标[^，。；]{0,10}不接受|是否接受联合体[^，。；]{0,10}不接受|",
        "备选[^，。；]{0,8}不允许|",
        // 「24投标文件的拒收情形」——前附表行号的拒决情形头（其列举即否决清单）；
        // 「9.2.1下列行为均属以他人名义投标」——冒名投标语义（其 GT 条款本体）
        "拒收情形|否决情形|无效情形|废标情形|以他人名义投标",
    ))
    .unwrap();
    // 例外从句（排除面——「除不可抗力外」不算否决信号，句法同 DOCX 侧 P0-B）
    let re_exception = Regex::new(r"除[^，。；]{0,24}外(，|以外)?").unwrap();
    // 资格面（要求/证书/审查语境——弱提及不算）
    let re_qual = Regex::new(concat!(
        "资质等级|资格审查|资格后审|资格预审|资质证书|营业执照|安全生产许可证|类似工程|",
        "级资质|资格条件|注册建造师|注册造价工程师",
    ))
    .unwrap();
    // #266-C 量化参数形态（数值 + 单位/%）——技术章子项内容行的判据
    // （「完好率≥95%」「平均接头损耗应不大于0.2dB/个」类——单位集保守：
    // 条/个/人/年等泛用量词不进面，防止技术章整段泛化）
    let re_quantified = Regex::new(
        r"\d+(?:\.\d+)?\s*(?:%|％|分钟|⼩时|小时|日历天|工作日|dB|dBm|M/s|Mbps|Gbps|万元|元|℃|km|GB|TB|MB|⽶|米)",
    )
    .unwrap();
    // 评分细则行：数字开头 + 短语 + "N 分"（评分办法章内）
    let re_score_line = Regex::new(r"^\d+[、.．]\s*\S{2,30}[：:，,]?\s*\S{0,16}\d+(?:\.\d+)?\s*分").unwrap();
    // 评分算法行：评标基准价/下浮率（强算法词免章门——基准价算法常在须知章）
    let re_score_algo = Regex::new(r"评标基准价|下浮率\s*K|基准价.{0,12}确定").unwrap();
    // 否决语境开启句（总括句——其后列举条目继承）。
    // #68 阶段 4（FP 收紧面）：「下列情形之一的」是歧义句式——处置结果
    // 才是判定关键（否决/无效 vs 重新招标/流标/终止/回避）。开启前先过
    // re_ctx_no_veto 排除面（非否决处置的总括句不开否决语境）。
    let re_ctx_open = Regex::new(
        r"下列情形之一|下列行为之一|下列情况之一|下列情形|下列行为|以下情形|其投标将被否决|否决其投标|视为允许他人|视为串通|不得参加投标|不得参与投标|拒收情形|否决情形|无效情形|废标情形",
    )
    .unwrap();
    // 总括句的非否决处置（重新招标/流标/终止/回避——列举继承的是程序语义）
    let re_ctx_no_veto = Regex::new(
        r"重新招标|重新采购|流标|终止招标|终止评标|终止投标|暂停|主动回避|应当回避|予以回避",
    )
    .unwrap();
    // 审查表语境开启（评审标准/资格审查标准行——其后圈号条目为隐含否决面）。
    // #68 阶段 4 扩面（missed 样例驱动）：
    //   「5.1.1 形式评审标准」「5.1.3 响应性评审标准」——四类评审段名；
    //   附表「评标表-4 / 序号 评审项目」——评标办法附表表头（pdftotext 线性化）；
    //   「初步评审是对投标文件…进行审查」——须知章的评审总述段。
    let re_review_open = Regex::new(
        r"评审标准|评审因素|初步审查标准|资格审查标准|审查标准|符合性审查|\
         初步评审|形式评审|资格评审|响应性评审|评审项目|评标表|检查项目|查验以下内容",
    )
    .unwrap();
    // 符合性句式（审查语境内行级过滤——参数行排除）。
    // #68 阶段 4（K2）：裸「符合」（工程质量：符合标准）是技术参数行——
    // 收紧为指向招标文件/条款/证件的一致性形态（missed 样例全是
    // 「符合招标文件规定」「与营业执照…一致」形）。
    let re_conformance = Regex::new(
        r"符合招标文件|符合第|符合规定|相一致|应当与|应与|是否完好|是否唯一|不存在|应为|须为|须具备|具备有效|数量符合|只能有一个报价|未同时在",
    )
    .unwrap();
    // 分值形态特征（评分结构行「①施工组织设计20分②…」——多处「N分」，
    // 圈号开头不匹配 re_score_line 的数字头，需独立排除——评分项语义非否决）
    let re_score_shape = Regex::new(r"\d+(?:\.\d+)?分").unwrap();
    // 表头词聚集（格式模板表头行：姓名/年龄/学历/职称/证号 ≥3 同现）
    let header_words = ["姓名", "年龄", "学历", "职称", "证号", "职务", "专业", "毕业学校"];
    let is_header_row = |t: &str| -> bool { header_words.iter().filter(|w| t.contains(**w)).count() >= 3 };
    // 澄清程序形态（澄清+说明同现且无强否决处置词——询标/补正程序非否决）
    let is_clarification = |t: &str| -> bool {
        t.contains("澄清") && t.contains("说明")
            && !["否决", "拒收", "无效", "废标", "取消"].iter().any(|k| t.contains(k))
    };
    // 投标人侧关系禁止信号（K4 的放行面——利害关系/隶属/控股/串通）
    let re_bidder_rel = Regex::new(r"利害关系|隶属|控股|串通|弄虚作假").unwrap();
    // 强审查表头（L3）：评标表/评审标准/检查项目类**显式表头**——它开的
    // 审查语境可穿透格式章守卫（附表三评标表物理住在投标文件格式章区域，
    // 但它是审查表不是格式模板；弱开启（初步评审等总述词）不穿透）。
    let re_review_strong = Regex::new(
        r"评标表|评审标准|资格审查标准|符合性审查|检查项目|查验以下内容|序号\s*评审项目",
    )
    .unwrap();
    // #234 要求动词面（格式章表格头判定的反面）：真条款行必有动词
    //（应/须/提供/递交/加盖/拟投入…），纯名词短语（「投标人名称」
    // 「传真 网址」）是模板占位。表求宽——防误杀真条款（漏抽比多留
    // 代价大，与 #226 死锁面权衡后宁可放行）。
    let re_req_verb = Regex::new(concat!(
        "应|须|需|请|不得|不予|提供|提交|递交|出具|附|加盖|填写|注明|标明|",
        "载明|写明|具备|具有|包括|包含|承诺|接受|拒绝|采用|使用|按照|依据|",
        "遵照|执行|响应|满足|符合|一致|密封|装订|签署|签字|盖章|编制|持有|",
        "取得|授予|委托|授权|拟|担任|负责|参与|发生|完成|承担"
    ))
    .unwrap();
    // #68 阶段 4（K1）：开标查验证件清单形态——「是否携带身份证/保证金收据/
    // 复印件/开户许可证/电汇单」是到场核查列举（GT 跨 7 件一致未标否决——
    // 标注口径：携带义务是形式要求，处置词未出现不推定否决）。传播排除面。
    let is_checklist_doc = |t: &str| -> bool {
        // #68 阶段 4（N1 扩面）：「⑵项目经理是否持本人居民身份证到开标现场」
        // 无原件/复印件字样（现场形态）——身份证/收据本身即清单信号；
        // 「4拟投入…注册建造师资格证及注册证加盖公章的复印件」——复印件
        // + 证件词共现
        (t.contains("身份证") || t.contains("收据") || t.contains("开户许可证")
            || t.contains("电汇单") || t.contains("网银转账凭证"))
            || (t.contains("复印件") && (t.contains("资质证书") || t.contains("许可证")
                || t.contains("资格证") || t.contains("建造师") || t.contains("营业执照")
                || t.contains("代码证")))
            || (t.contains("携带") && (t.contains("原件") || t.contains("授权委托书")))
    };
    // 模板聚集排除（分值/万元/表头——格式模板与评分结构面）
    let is_template_shape = |t: &str| -> bool {
        re_score_shape.find_iter(t).count() >= 2
            || (t.matches("万元").count() >= 3)
            || is_header_row(t)
            || is_clarification(t)
    };
    // 流程步骤句式（语境继承排除面——评标程序/开标议程/分工职责的列举
    // 非否决语义：「负责/宣布/介绍/组建/复核/汇总/推荐…」）
    let re_procedure = Regex::new(concat!(
        "评审阶段|负责|宣布|介绍|组建|议程|复核、|汇总|推荐中标候选|提交评标报告|澄清、说明|",
        "以大写金额为准|以单价金额为准|以开标报价为准|来往函件|合同约定|履约|",
        "汇报|审议|赋分|评分标准|详评表|见评标办法前附表|是对投标文件|是对招标|",
        "为准|修正总价|修正投标报价|修正总报价|",
        "我承诺|我方承诺|承诺书|组织询标|询问核实|评标预备会|",
        // #68 阶段 4（FP 收紧面——missed 样例反面的误报样例驱动）：
        // 评委会职责/推选/详细评审/公示/定标议程——评标程序列举非否决；
        // 评标委员会面（组建/分工/回避）是 RECUSE/程序语义（GT false 族）
        "推选|详细评审|职责分工|评标结果公示|组织评委|定标|",
        // #68 阶段 4（K3——r4 FP 样例驱动）：评审定性/报价修改程序/费用行/
        // 开标议程/技术运维列举（数据库检查、软件维护——技术章非否决面）
        "定性评审|总评为|修改报价|修改投标报价|招标代理费|唱标|开标结束|",
        "软件维护|补丁升级|性能检查|数据库|测站|运行情况|",
        // N2：运维巡检/质保费用/技术标编制结构（r8 FP 样例驱动）
        "巡检|软件版本|功能检查|质保期|编制水平|内容完整性"
    ))
    .unwrap();
    // #144：名词「负责人」（单位负责人/技术负责人/项目负责人——身份描述）
    // 不是程序动词「负责」——**仅在评审表/定义组语义场内剥离名词**后再匹配
    // （保守面：非语义场的「技术负责人应具有…职称」类资格条款仍走原守卫）。
    // 实测误杀面：A-01/A-05 形式评审表「单位负责人为同一人…不得参加同一
    // 标段…」与 9.2.2 本单位人员定义组（「技术负责人、财务负责人…不是本
    // 单位人员」——语境被杀后续 ⑴聘任/⑵工资全部失语境——GT round-2 增补
    // 目标）。
    let procedure_hit = |t: &str| -> bool {
        let stripped = if t.contains("单位负责人为同一人")
            || t.contains("本单位人员")
            || t.contains("聘任合同")
            || t.contains("工资关系")
        {
            t.replace("负责人", "")
        } else {
            t.to_string()
        };
        re_procedure.is_match(&stripped)
    };
    // 列举条目（（N）/N、/⑴⑵⑶ 圈号——语境传播对象；「3.5.2」条款号排除）。
    // #68 阶段 4 扩面（missed 样例驱动——审查表条目的 pdftotext 残影形态）：
    //   「1.投标范围符合…」「7 投标文件的正本…」——点号/空格分隔的裸数字条目
    //   （附表线性化丢分隔符；「5.1.3」条款号由 re_clause 先行分支排除，不冲突）；
    //   「8投标文件的密封…」——数字直接连排。
    let re_list_cn = Regex::new(
        r"^[（(][0-9一二三四五六七八九十]{1,3}[）)]|^[⑴⑵⑶⑷⑸⑹⑺⑻⑼⑽⑾⑿]|^[⒈⒉⒊⒋⒌⒍⒎⒏⒐]|^[0-9]{1,2}[、.．]|^[0-9]{1,2}\s*[一-龥]|^[0-9]{1,3}[）)]",
    )
    .unwrap();
    let mut chapter = String::new();
    // #272：当前章起点块号（内容证据的查询键——章分支重置）
    let mut chapter_start: usize = 0;
    let mut subsection = String::new();
    // 否决语境（总括句开启；新编号条款/非列举普通行终止）
    let mut veto_ctx = false;
    // 审查表语境（评审标准/资格审查标准行开启；章节头终止）
    let mut review_ctx = false;
    // 强审查头已开标志（L3——格式章守卫的旁路面）
    let mut review_strong = false;
    // #234 格式章标志：章号行自身含「格式」（同行形态）或章标题行
    //（两行形态——「第8章」/「投标文件格式」分立）含「格式」。
    // chapter 变量在两行形态下停在章号行（不含「格式」），既有
    // chapter.contains("格式") 守卫全部漏判——本标志补面，chapter
    // 字符串本身不动（category_for 行为面不变）。
    let mut format_chapter = false;
    // #234 章标题行文本（两行形态前瞻所得）：章标题是章节结构非正文，
    // 拼进段落时不得作为段首 noun 判定拦段（标题行段的既有行为保持）。
    let mut chapter_title: Option<String> = None;

    // #234 格式章表格头形态判定（纯文本面——格式章上下文由调用侧组合）：
    // 剥条款号/列举序号后 (a) 无句末标点 (b) 非空白 ≤12 字 (c) 无要求
    // 动词 → 纯名词短语占位行（「投标人名称」「联系人 电话」——投标人
    // 要填的位置标注，非响应要求）。
    let is_noun_phrase_row = |t: &str| -> bool {
        let no_clause = re_clause.replace(t, "").into_owned();
        let no_list = re_list_cn.replace(&no_clause, "").into_owned();
        let stripped = re_subsection.replace(&no_list, "");
        let s = stripped.trim();
        if s.is_empty() {
            return false;
        }
        // (a) 无句末标点（。；！？：——模板头无完整句）
        if s.chars().last().map(|c| "。；！？：；".contains(c)).unwrap_or(false) {
            return false;
        }
        // (b) 短行：≤12 字（长行是表内容/正文段，不在此面）
        if s.chars().filter(|c| !c.is_whitespace()).count() > 12 {
            return false;
        }
        // (c) 无要求动词
        !re_req_verb.is_match(s)
    };

    // 强词判定（例外从句剥离后匹配；履约语境排除——「承包人不得…拒收」
    // 类合同期条款非投标否决：主体为承包人/发包人且无投标词时不判）
    let veto_hit = |text: &str| -> bool {
        let stripped = re_exception.replace_all(text, "");
        if !re_veto.is_match(&stripped) {
            return false;
        }
        // N3：提疑/答疑程序的「不予受理、答复」非否决（逾期提问不受理是
        // 答疑程序规则——r8 FP：「逾期提出的，招标人及代理机构不予受理、答复」）
        if stripped.contains("提疑") || stripped.contains("提出疑问") {
            return false;
        }
        let perf_ctx = stripped.contains("承包人") || stripped.contains("发包人") || stripped.contains("监理人现场服务") || stripped.contains("监理人对所有来往文件");
        let bid_ctx = stripped.contains("投标");
        !(perf_ctx && !bid_ctx)
    };

    let blocks = &f.blocks;
    // #272 跨平台结构形态：文档级方案仲裁（章 > 部分/篇 > 数字头 > 告警）。
    // 仲裁保证：有「章」骨架的文档（batch1×5 + NM×3）新增面完全不激活——
    // 既有行为逐字节不变；「部分/数字」方案只在今天章上下文恒空的文档生效。
    // part_starts 已过枚举簇抑制（「询价文件构成」节里逐行列举的七部分不是边界）。
    let structure = crate::structure::detect_structure(blocks);
    let part_starts: std::collections::HashSet<usize> =
        if structure.scheme == crate::structure::TopScheme::Part {
            structure.chapters.iter().map(|c| c.idx).collect()
        } else {
            Default::default()
        };
    let numbered_starts: std::collections::HashSet<usize> =
        if structure.scheme == crate::structure::TopScheme::Numbered {
            structure.chapters.iter().map(|c| c.idx).collect()
        } else {
            Default::default()
        };
    // 识别不确定性响亮告警（不静默失败）——测试面 eprintln 横幅 +
    // parse_file 侧追加 parse_summary（import 回执透出）
    for w in &structure.warnings {
        eprintln!("\n====================================================================\n【结构告警 #272】{w}\n====================================================================\n");
    }
    // ---- 段落重组（P0-A 核心）：行尾无终结标点 → 与下一行同段
    // 预计算每行的「段落文本」（本行 + 后续续行拼接）与「段落块范围」。
    // 断段条件（优先级从上到下）：
    //   目录行 / 章节头 / 中文序号头 → 独立段（不与后行拼）
    //   行尾有终结标点（。；！？：）→ 自身成段
    //   下一行是新条款标志（编号/章节头/中文序号）→ 当前行成段
    //   其余 → 拼接（行尾截断检测——pdftotext 物理换行切断句子）
    let ends_sentence = |s: &str| -> bool {
        s.chars().last().map(|c| "。；！？：；".contains(c)).unwrap_or(false)
    };
    let is_clause_start = |idx: usize, s: &str| -> bool {
        re_clause.is_match(s) || re_chapter.is_match(&nfkc(s)) || re_subsection.is_match(s)
            // #272：部分/数字顶层头是章同构的段边界——只在对应方案激活
            //（章骨架文档里它们是普通正文行，语义不变）。
            // 按块号判（非正则）——枚举簇成员已剔除，正则形态不可区分
            || part_starts.contains(&idx)
            || numbered_starts.contains(&idx)
    };
    let is_skip_line = |s: &str| -> bool {
        s.is_empty() || s.matches('.').count() >= 6 || s.contains("……")
    };

    let n = blocks.len();
    // para_end[i]：行 i 所在段落的末行（含自身）；para_start[i]：首行。
    // 段落成员行（para_start[i] < i）由首行统一处理（防段落尾行双抽）。
    let mut para_end = vec![0usize; n];
    let mut para_start = vec![0usize; n];
    let mut i = 0usize;
    while i < n {
        if !matches!(blocks[i].kind, BlockKind::PdfLine | BlockKind::OcrLine) || is_skip_line(blocks[i].text.trim()) {
            para_end[i] = i;
            para_start[i] = i;
            i += 1;
            continue;
        }
        let mut j = i;
        // 向后扩展：当前行未终结 && 下一行可拼接（非跳过行/非条款头）
        loop {
            let cur = blocks[j].text.trim();
            if ends_sentence(cur) {
                break;
            }
            let Some(next) = blocks.get(j + 1) else { break };
            if !matches!(next.kind, BlockKind::PdfLine | BlockKind::OcrLine)
                || is_skip_line(next.text.trim())
                || is_clause_start(j + 1, next.text.trim())
                // #68 阶段 4（G——根因修复）：列举条目行强制断段。missed 样例：
                // 「5.1.1 形式评审标准：（见附表三：评标表-4）」尾缀 ）非终结
                // 标点 → ⑴⑵⑶ 审查条目被拼进编号段 → 段落去重吞掉条目、
                // 整段按 re_qual 落「资格条件」——D 族 L2miss 的主因。
                // 条目（（N）/N、/⑴/裸数字）是新单元，永不作为续行。
                || re_list_cn.is_match(next.text.trim())
            {
                break;
            }
            j += 1;
        }
        for k in i..=j {
            para_end[k] = j;
            para_start[k] = i;
        }
        i = j + 1;
    }
    // 段落拼接文本（行序）
    let para_text = |start: usize, end: usize| -> String {
        let mut s = String::new();
        for k in start..=end {
            if matches!(blocks[k].kind, BlockKind::PdfLine | BlockKind::OcrLine) {
                s.push_str(blocks[k].text.trim());
            }
        }
        s
    };
    // 段落 anchors（每行一个，excerpt 保真）
    let para_anchors = |start: usize, end: usize| -> Vec<Anchor> {
        (start..=end).filter_map(|k| {
            if matches!(blocks[k].kind, BlockKind::PdfLine | BlockKind::OcrLine) {
                Some(anchor(f, &blocks[k]))
            } else {
                None
            }
        }).collect()
    };

    // #144 审查表接管（PDF 线性化附表——形式评审表/初评表等）：
    // review_table 装配消费的行（含孤立序号行）扩展到整段后从主循环剔除
    // （防段落重组双抽）；结构化条目在主循环后统一 REQ 化（表名行不剔除——
    // 留给旧链开审查语境，与既有行为一致）。
    let (review_items, review_used) = crate::review_table::extract_pdf_linearized(f);
    // #252 综合评分表专路：评分表行（评审项/分值/细则）装配 + 区域接管
    // （表名行计入消费——评分表名行在旧链无语境价值，留着会被评分细则
    // 行误抽。review_table 在评分表表名行收表，两路区域互不重叠）。
    let (scoring_rows, scoring_used) = crate::scoring_table::extract_scoring_table_pdf(f);
    let review_consumed: std::collections::HashSet<usize> = review_used
        .iter()
        .chain(scoring_used.iter())
        .filter(|&&rb| rb < blocks.len())
        .flat_map(|&rb| para_start[rb]..=para_end[rb])
        .collect();

    // #266 语义表接管（PDF 布局面）：优先级 审查表 > 语义表 > 值行 > 旧链
    // （区域接管防双抽——「1、期次1…」编号单元格旧路由装配行整体取代）。
    let (pdf_sem_items, pdf_sem_used) =
        crate::table_semantics::extract_pdf_semantic_tables(f, &review_consumed);
    let sem_consumed: std::collections::HashSet<usize> = pdf_sem_used
        .iter()
        .filter(|&&rb| rb < blocks.len())
        .flat_map(|&rb| para_start[rb]..=para_end[rb])
        .collect();
    let pdf_value_items = {
        let mut skip = review_consumed.clone();
        skip.extend(sem_consumed.iter().copied());
        crate::table_semantics::extract_pdf_value_rows(f, &skip)
    };
    let mut sem_consumed = sem_consumed;
    for vi in &pdf_value_items {
        sem_consumed.insert(vi.block_idx);
    }

    // ---- #272 验收 4：章区间内容特征证据（表格锚定——分类与章名解耦）----
    // 三源（全部复用既有专路的识别产物，零新字面）：
    //   参数语义表（section 非商务）→ 技术参数
    //   综合评分表 + 详细评审表 → 评分项
    //   资格性审查表 → 资格条件
    // 唯一强信号才判（多信号冲突/无证据 → 不判）——证据是「其他」的
    // 兜底而非主判：字面命中照旧优先（既有行为不变），章名改形后字面
    // 失配时内容特征接管（验收 4：改章名不影响分类结果）。
    let chapter_bounds: Vec<usize> = {
        let mut v: Vec<usize> = if structure.scheme == crate::structure::TopScheme::Zhang {
            structure.chapters.iter().map(|c| c.idx).collect()
        } else {
            part_starts
                .iter()
                .chain(numbered_starts.iter())
                .copied()
                .collect()
        };
        v.sort_unstable();
        v.dedup();
        v
    };
    let param_blocks: std::collections::HashSet<usize> = pdf_sem_items
        .iter()
        .filter(|it| !it.section.contains("商务"))
        .flat_map(|it| it.block_idxs.iter().copied())
        .collect();
    let qual_blocks: std::collections::HashSet<usize> = review_items
        .iter()
        .filter(|it| matches!(it.kind, crate::review_table::ReviewTableKind::Qualification))
        .flat_map(|it| it.block_idxs.iter().copied())
        .collect();
    let evidence: std::collections::HashMap<usize, &'static str> = chapter_bounds
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let e = chapter_bounds.get(i + 1).copied().unwrap_or(usize::MAX);
            let (mut p, mut sc, mut q) = (0usize, 0usize, 0usize);
            for b in blocks.iter().skip(s).take_while(|b| b.idx < e) {
                if param_blocks.contains(&b.idx) {
                    p += 1;
                } else if scoring_used.contains(&b.idx) {
                    sc += 1;
                } else if qual_blocks.contains(&b.idx) {
                    q += 1;
                }
            }
            let cat = if p > 0 && sc == 0 && q == 0 {
                "技术参数"
            } else if sc > 0 && p == 0 && q == 0 {
                "评分项"
            } else if q > 0 && p == 0 && sc == 0 {
                "资格条件"
            } else {
                "其他"
            };
            (s, cat)
        })
        .collect();
    // 解析后的分类（字面优先 → 内容证据兜底）——主循环各 category_for
    // 调用点统一改走此面（chapter_start 是当前章的起点块号）。
    // #272：字面判定前剥掉章名括号内的补充说明——「第六部分 政府采购
    // 合同（参考格式）」的「（参考格式）」不是格式章语义（实测 62 条
    // 合同条款误落格式要求）。括号外才是章的本题。
    let category_resolved =
        |chapter: &str, subsection: &str, chapter_start: usize| -> &'static str {
            let core = strip_paren_notes(chapter);
            let lit = category_for(&core, subsection);
            if lit != "其他" {
                return lit;
            }
            evidence.get(&chapter_start).copied().unwrap_or("其他")
        };

    let mut seen_para = std::collections::HashSet::new();
    for (i, b) in blocks.iter().enumerate() {
        if !matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine) {
            continue;
        }
        // #144 审查表接管：被装配行（含其段落成员）跳过既有逐行分支
        if review_consumed.contains(&i) {
            continue;
        }
        // #266 语义表/值行接管：同款区域跳过
        if sem_consumed.contains(&i) {
            continue;
        }
        let t = b.text.trim();
        // 目录行防御：带页码引导点（……/....）的目录条目不得建立章节上下文
        if t.matches('.').count() >= 6 || t.contains("……") {
            continue;
        }
        // #234 修法 2：碎片过滤——剥条款号后 <2 字的**单行**段落不抽 REQ
        //（表格单元格残片/纯数字残渣；veto_rule 闸 5 同思路在抽取层落地，
        // 不依赖否决信号）。多行段落的首行碎片（「9」+ 跨页续行——段落
        // 本体是真条款文本）不在此面：拦了就丢条款。
        if para_end[i] == i {
            let frag_n = re_clause
                .replace(t, "")
                .chars()
                .filter(|c| !c.is_whitespace())
                .count();
            if frag_n < 2 {
                continue;
            }
        }
        if re_chapter.is_match(&nfkc(t)) {
            chapter = t.to_string();
            chapter_start = b.idx;
            subsection.clear();
            veto_ctx = false;
            review_ctx = false;
            review_strong = false;
            // #234：两行章形态（「第8章」/「投标文件格式」分立行）——
            // 章号行不含标题，向前看首个内容行补判格式章。同行形态
            //（「第8章 投标文件格式」标题内联）不前瞻——窗口内首个
            // 内容行是正文行，误记为章标题会放走正文 noun 行（same_line
            // 实测：内联形态下「信用等级」被误当标题豁守护卫）。
            let inline_title = re_chapter.replace(&nfkc(t), "").trim().to_string();
            chapter_title = if inline_title.is_empty() {
                chapter_title_ahead(blocks, i)
            } else {
                None
            };
            format_chapter = chapter.contains("格式")
                || chapter_title
                    .as_deref()
                    .map_or(false, |title| title.contains("格式"));
            continue;
        }
        // #272：「第X部分/篇」顶层骨架——仅当全文无「第X章」（文档级
        // 方案仲裁）。语义镜像章分支：重置语境、两行形态前瞻、格式章
        // 判定同款。询价类文件（第一部分 投标邀请 … 第七部分 响应文件
        // 格式）由此获得章上下文（此前恒空 → 全落「其他」）。
        // 按块号判（part_starts 已过枚举簇抑制）。
        if part_starts.contains(&b.idx) {
            chapter = t.to_string();
            chapter_start = b.idx;
            subsection.clear();
            veto_ctx = false;
            review_ctx = false;
            review_strong = false;
            let inline_title = re_part_head.replace(&nfkc(t), "").trim().to_string();
            chapter_title = if inline_title.is_empty() {
                chapter_title_ahead(blocks, i)
            } else {
                None
            };
            // #272：格式章判定用「括号外含格式」——括号内补充说明不算。
            // 实测：询价「第六部分 政府采购合同（参考格式）」被裸 contains
            // 误判格式章 → 62 条合同条款误落格式要求。括号内容剥离后再判
            //（Zhang 分支保持原 contains 语义——既有行为红线不动）。
            format_chapter = fmt_chapter_of(&chapter)
                || chapter_title
                    .as_deref()
                    .map_or(false, |title| fmt_chapter_of(title));
            continue;
        }
        // #272：「N.标题」数字顶层骨架——仅获胜游程成员（同上仲裁）。
        // 行本身就是标题（数字+题名同段），无两行形态。
        if numbered_starts.contains(&b.idx) {
            chapter = t.to_string();
            chapter_start = b.idx;
            subsection.clear();
            veto_ctx = false;
            review_ctx = false;
            review_strong = false;
            chapter_title = None;
            format_chapter = fmt_chapter_of(&chapter);
            continue;
        }
        if re_subsection.is_match(t) && t.chars().count() <= 40 {
            subsection = t.to_string();
            continue;
        }
        // #68 阶段 4（D）：裸数字短标题行 = 小节头语义——pdftotext 把
        // 「7．合同授予」「8定标及报告」「9．纪律和监督」类小节头线性化为
        // 数字+标题形态（无尾标点、≤12 字）。按条目传播会产 FP（评审程序/
        // 公示/定标议程段的列举非否决）：按**段**判（短段、无尾标点），
        // 是评审头则开审查语境，否则清语境并 continue。
        // 「7 投标人是否」类表条目断头不在此面——其段落与续行拼接后长
        // 且带尾标点（；），不匹配头形态。
        if pend_head_form(i, &para_text, &para_end, &blocks) {
            if re_review_open.is_match(&para_text(i, para_end[i])) {
                review_ctx = true;
            } else {
                veto_ctx = false;
                review_ctx = false;
            }
            subsection = para_text(i, para_end[i]);
            continue;
        }
        // 段落去重：非首行成员行跳过（防段落尾行独立双抽）——
        // **超短行例外**（<6 字）：pdftotext 碎片行（「予以拒收。」）是
        // GT 单行条款的锚定面（段落 REQ 的 anchor 因 <6 字不参与 L2
        // 文本匹配），保留其独立 REQ（碎片级条款面，非语义重复）
        if para_start[i] < i {
            let norm_len = t.chars().filter(|c| !c.is_whitespace()).count();
            if norm_len >= 6 {
                continue;
            }
        }
        let pend = para_end[i];
        if pend > i {
            if seen_para.contains(&i) {
                continue;
            }
            seen_para.insert(i);
        }
        let para = para_text(i, pend);
        // 编号行：条款号取首行；判定文本用整段（跨行词还原）。
        // 顿号列举（N、）双守卫（#218 任务 3）——命中则让本行落入下方既有
        // 分支链（等价于改前行为），不进编号分支：
        // ① 开放传播语境（veto_ctx/review_ctx）：总括句/审查表的列举条目按
        //    传播语义继承否决面（顿号编号化会丢语境继承——P 红线面）；
        // ② 格式章模板（投标函「1、我方承诺…」是投标人侧范本非要求面）
        //    ——与传播分支的格式章节守卫同口径。
        // 点分条款号（4.2）不在守卫面：编号面窄、模板密度低，保持既有编号
        // 捕获（#68 J：编号条款终结语境的语义不变）。
        let dunhao_defer = re_dunhao_list.is_match(t)
            && ((veto_ctx || review_ctx)
                || (chapter.contains("格式") && !review_strong));
        if let Some(m) = re_clause.captures(t).filter(|_| !dunhao_defer) {
            // #68 阶段 4（J）：编号条款行同时终结审查语境——原实现 review_ctx
            // 只被章节头终结，「1.5 …见评标表」类提及会在百页须知章内长开，
            // 其后全部列举条目误传播（r3 实测 FP 618 的根因）。审查表头
            // 自身（F1）在其后重新开启——语境被界定在审查表区域内。
            veto_ctx = false;
            review_ctx = false;
            let mut params = HashMap::new();
            fill_params(&para, &re_model, &re_amount, &re_date, &re_date, &mut params);
            // 编号行归类优先级（R3-S3-5）：强否决短语 > 资格面 > 章节上下文——
            // 编号否决条款（如"3.4.2 …将被否决"）不得落"其他"（漏否决=废标风险）
            // #226 层 2：否决信号若其实是「招标人规则陈述」（本次招标/招标人/
            // 逾期不予受理…），不判否决项——它没有可请求的响应正文，判了就是
            // 永久 blocking 的死锁。安全网在 veto_rule::is_rule_statement 内部
            // （投标人义务词/资格材料诉求一律不算规则说明）。
            let veto_signal = veto_hit(&para) && !is_template_shape(&para);
            let is_rule = veto_signal && crate::veto_rule::is_rule_statement(&para);
            let cat = if veto_signal && !is_rule {
                "否决项"
            } else if re_qual.is_match(&para) {
                "资格条件"
            } else {
                category_resolved(&chapter, &subsection, chapter_start)
            };
            // 顿号形态的条款号取纯数字（「1、」→「1」——顿号是分隔符，
            // 与点分形态「4.2」同语义：编号本体入 clause_no）
            let cno = m[1].trim_end_matches('、').to_string();
            push_req(reqs, seq, cat, Some(cno),
                     para_anchors(i, pend), params, &f.name);
            if re_ctx_open.is_match(&para) && !re_ctx_no_veto.is_match(&para) {
                veto_ctx = true;
            }
            // #68 阶段 4（F1）：编号行是审查表头（「5.1.1 形式评审标准：（见
            // 附表三：评标表-4）」）→ 开审查语境——原实现里语境开启检查在
            // 逐条 else-if 链的尾部，编号行先行分支吃掉后永远走不到，
            // 其后的 ⑴⑵⑶ 审查条目全部失语境（L2miss D 族主因之一）
            if re_review_open.is_match(&para) {
                review_ctx = true;
                review_strong = re_review_strong.is_match(&para);
            }
        } else if veto_hit(&para) && !is_template_shape(&para)
        {
            // #226 层 2：规则说明段（「招标人不予受理…」类招标人规则陈述）
            // 不落「否决项」分类——它没有可请求的响应正文，判了就是永久
            // blocking 的死锁。**条款本体不丢**：第一版把规则说明形整段跳过
            // 本分支，段落直落兜底 else（不 push REQ）——rebase 到含 #223
            // 修复的 main 后实测重庆西永条款数 -1（316 < 317，real_corpus_223
            // 基线红线抓出）：条款丢了比错分类更严重。现在改为分类降级
            // （资格面/章节上下文），REQ 照常落账。
            // 强否决段落（无编号面——前附表/正文流条款；段落重组后跨行
            // 条款「…招标人」+「拒收投标文件。」还原为整段判定；
            // 分值结构行排除——「N分」≥2 处的评分语义不判否决）
            let is_rule = crate::veto_rule::is_rule_statement(&para);
            let cat = if is_rule {
                if re_qual.is_match(&para) && !para.starts_with("见") {
                    "资格条件"
                } else {
                    category_resolved(&chapter, &subsection, chapter_start)
                }
            } else {
                "否决项"
            };
            veto_ctx = false;
            push_req(reqs, seq, cat, None, para_anchors(i, pend), HashMap::new(), &f.name);
            if re_ctx_open.is_match(&para) && !re_ctx_no_veto.is_match(&para) {
                veto_ctx = true;
            }
        } else if re_qual.is_match(&para) && !para.starts_with("见")
            // #68 阶段 4（I）：语境中的列举条目优先走传播分支——审查表条目
            // 常含资格词（「⑴投标人名称应与营业执照、资质证书…一致」），
            // 资格分支抢先会把它落「资格条件」而丢否决语义（GT 的资格性
            // 否决恰在审查表条目里）。条目 + 开放语境 → 传播优先。
            && !((veto_ctx || review_ctx) && re_list_cn.is_match(t))
            // #234 修法 1：格式章表格头守卫——格式章（第X章 投标文件格式）
            // 内的纯名词短语行是模板占位（「投标人名称」「传真 网址」——
            // 投标人要填的位置标注），不是响应要求：整页表格行无句末标点
            // 被段落重组拼成大段，<6 字成员行独立触发 REQ 化，段落中后部
            // 的资格词面（「营业执照号/资质证书」）误判资格条件 → 永久
            // blocking 死锁（A-04 实测 9 条 REQ-0943~0951）。强审查表头
            // 旁路（review_strong——附表三评标表物理住在格式章区域的既有
            // 口径）；带要求动词/句末标点的真条款行不拦；章标题行拼段时
            // 是章节结构非段首 noun，排除（标题行段既有行为保持）。
            && !(format_chapter
                && !review_strong
                && is_noun_phrase_row(t)
                && Some(t) != chapter_title.as_deref())
        {
            // 资格面行："见投标人须知前附表"类引用行排除
            push_req(reqs, seq, "资格条件", None, para_anchors(i, pend), HashMap::new(), &f.name);
            veto_ctx = false;
        } else if (veto_ctx || review_ctx) && re_list_cn.is_match(t)
            && t.chars().filter(|c| !c.is_whitespace()).count() >= 5
            && !(chapter.contains("格式") && !review_strong)
            && !is_checklist_doc(&para)
            && !procedure_hit(&para)
            && !re_score_line.is_match(t)
            && !is_template_shape(&para)
            // K4：履约主体守卫（与 veto_hit 的 perf_ctx 同口径）——
            // 「承包人需配合…验收」「质保期…费用由中标单位承担」类合同期
            // 条款列举不是投标否决（GT CONTRACT 族 false）。利害关系/
            // 隶属/控股/串通信号在场时是投标人侧禁止条件（missed 样例：
            // 「⑼与本招标项目的施工承包人…有隶属关系或者其他利害关系」）
            // ——放行。
            && !((para.contains("承包人") || para.contains("发包人"))
                && !para.contains("投标")
                && !re_bidder_rel.is_match(&para))
        {
            // 语境传播：总括句/审查表头后的列举条目（（N）/N、/⑻ 圈号）
            // 继承否决语义（审查表条目 = 隐含否决面）；
            // 流程步骤句式排除（评标程序/开标议程/分工职责列举非否决）。
            // #68 阶段 4：最短长度门 ≥5 字——裸数字扩面后「6 银行」类
            // 前附表碎片行（支付方式行的断头）不传播（精度护栏）；
            // 格式章节守卫——投标函/格式模板的编号列举（「4.如我方中标：」
            // 「6.（其他补充说明）。」）是范本结构非否决语义
            push_req(reqs, seq, "否决项", None, para_anchors(i, pend), HashMap::new(), &f.name);
        } else if review_ctx && re_conformance.is_match(t) && t.chars().count() >= 6
            && !is_checklist_doc(&para)
            && !procedure_hit(&para)
            && !is_template_shape(&para)
        {
            // 审查语境内符合性行（「投标人名称应与…一致」——非圈号形态）
            push_req(reqs, seq, "否决项", None, para_anchors(i, pend), HashMap::new(), &f.name);
        } else if re_review_open.is_match(&para) {
            // 审查表语境开启（评审标准/资格审查标准行）
            review_ctx = true;
            review_strong = re_review_strong.is_match(&para);
        } else if (chapter.contains("评标办法") && re_score_line.is_match(t))
            || re_score_algo.is_match(&para)
        {
            // 评分细则行（中文序号/数字序号分值行）+ 评分算法行（评标基准价/下浮率——
            // P2：基准价算法与下浮率 K 常在须知章 10.x 而非评标办法章，强算法词免章门）
            push_req(reqs, seq, "评分项", None, para_anchors(i, pend), HashMap::new(), &f.name);
            veto_ctx = false;
        } else if re_list_cn.is_match(t)
            && !format_chapter
            && !is_checklist_doc(&para)
            && !procedure_hit(&para)
            && !is_template_shape(&para)
            && category_resolved(&chapter, &subsection, chapter_start) == "技术参数"
            && re_quantified.is_match(&para)
        {
            // #266-C：技术章子项内容行——（N）/N、 列举里的量化参数形态
            // 单独成 REQ（songling 层1 实测：技术章「（1）完好率≥95%」「（5）
            // 接头损耗不大于0.2dB」类子项碎行是 12/16 锚点 MISS 的主体面——
            // 标题行 REQ「1、接入速率」不含内容值）。上下文窄守卫：仅
            // 技术面章节（category_for 裁决）+ 量化形态——防泛化噪声。
            let mut params = HashMap::new();
            params.insert("statement".to_string(), serde_json::json!(para));
            push_req(reqs, seq, "技术参数", None, para_anchors(i, pend), params, &f.name);
        } else {
            // 非列举普通行终止否决语境（防语境流污染）——**长散文才杀**。
            // #68 阶段 4（F4）：原无条件终止在两种真实形态上误杀语境
            // （missed 样例：A04 p25「投标人本单位人员，必须同时满足以下
            // 条件：」是列举中段的引导句，其后 (1)(3) 条目全部失语境）；
            // 短行/引导句（尾随冒号）保持语境，≥40 字的连续散文段才终止。
            // 编号新条款/章节头/资格/评分分支的既有终止面不变。
            // #68 阶段 4（F4b）：裸总括句（自身无强词、非任何先行分支——
            // 「有下列情形之一的，不得参加投标：」形）在此开启语境（原实现
            // 只在编号/强词两分支内开——裸形态漏开）。
            if re_ctx_open.is_match(&para) && !re_ctx_no_veto.is_match(&para) {
                veto_ctx = true;
            } else if para.chars().count() >= 40 {
                veto_ctx = false;
            }
        }
    }

    // #144 审查表条目 REQ 化：表7/表8 面（形式评审表/初评表等附表形态）→
    // 否决项（表级脚注「任何一项未通过的，评标委员会应当否决其投标」——
    // round-1 GT 判例口径：评审表行 = true）；详细评审面 → 评分项。
    // 锚 = 装配行文本（孤立序号行不入锚——GT 对齐口径；locator 取首块定位）。
    for it in &review_items {
        let cat = match it.kind {
            crate::review_table::ReviewTableKind::Detailed => "评分项",
            _ => "否决项",
        };
        let locator = it
            .block_idxs
            .first()
            .and_then(|&bi| f.blocks.get(bi).map(|b| b.locator.clone()))
            .unwrap_or_else(|| it.locator.clone());
        // #170 方案 A（抽取侧输入）：评审表行文本的「第X章」引用
        // （详见/按/响应 第X章…——真实件实测形态）登记为
        // params.section_title，联结侧（bid-mcp build_scorer）按章号
        // 归一匹配大纲节点——「评分项按章节自评」由此恢复。无引用不
        // 登记（映射缺省与现状一致，不虚构）。
        let joined_ref = format!(
            "{} {} {}",
            it.item,
            it.criteria,
            it.score_rules.as_deref().unwrap_or("")
        );
        let mut params = HashMap::new();
        if let Some(title) = crate::review_table::chapter_ref_of(&joined_ref) {
            params.insert("section_title".into(), serde_json::json!(title));
        }
        push_req(
            reqs, seq, cat,
            Some(format!("{}-{}", it.table_name, it.row)),
            vec![Anchor {
                file: f.name.clone(),
                kind: "table".into(),
                locator,
                excerpt: it.item.clone(),
            }],
            params, &f.name,
        );
    }
    // ---- #252 综合评分表行 REQ 化（评分驱动大纲输入面）


    // #252 综合评分表行 REQ 化：每行 → 评分项（评分驱动大纲的输入面）。
    // params 携带 name/score/track/objective/sub_items——域层（scoring_outline）
    // 据此建「评审项 → L1 章节 / 细则 → L2 小节」。锚 excerpt = 规范名前缀
    // + 行文本（NFKC 重建——康熙部首归一，检索面可用；构造见 scoring_table
    // 模块文档）。clause_no 沿用审查表行约定「表名-行号」。
    for sr in &scoring_rows {
        let mut params = HashMap::new();
        params.insert("name".into(), serde_json::json!(sr.name));
        params.insert("score".into(), serde_json::json!(sr.score));
        params.insert("track".into(), serde_json::json!(sr.track));
        params.insert("objective".into(), serde_json::json!(sr.objective));
        if !sr.sub_items.is_empty() {
            params.insert("sub_items".into(), serde_json::json!(sr.sub_items));
        }
        push_req(
            reqs, seq, "评分项",
            Some(format!("综合评分表-{}", sr.row)),
            vec![Anchor {
                file: f.name.clone(),
                kind: "table".into(),
                locator: sr.locator.clone(),
                excerpt: sr.criteria.clone(),
            }],
            params, &f.name,
        );
    }

    // ---- #266 PDF 语义表条目 REQ 化（布局面，商务/技术参数）

    // #266 语义表条目 REQ 化（PDF 布局面）：类别=小节上下文优先（含「商务」
    // →商务要求——小节级显式命名优先于章级推断），否则技术参数；★入
    // params.star（#276 对齐面）；锚=行+续行块原文（数值与单位可检索——
    // #250 验收口径的 PDF 面）。
    for it in &pdf_sem_items {
        let sec = crate::norm_n1(&it.section);
        let cat = if sec.contains("商务") {
            "商务要求"
        } else {
            "技术参数"
        };
        let mut params = HashMap::new();
        params.insert("statement".to_string(), serde_json::json!(it.statement));
        if it.star {
            params.insert("star".to_string(), serde_json::json!(true));
        }
        let anchors: Vec<Anchor> = it
            .block_idxs
            .iter()
            .filter_map(|&bi| f.blocks.get(bi).map(|b| Anchor {
                file: f.name.clone(),
                kind: "table".into(),
                locator: b.locator.clone(),
                excerpt: b.text.clone(),
            }))
            .collect();
        push_req(
            reqs,
            seq,
            cat,
            Some(format!("表{}-行{}", it.table_no, it.row)),
            anchors,
            params,
            &f.name,
        );
    }
    // #266 值行 REQ 化（前附表/公告区关键值——商务要求值面，废标关键值
    // GR-010~014 的锚面）。
    for vi in &pdf_value_items {
        let mut params = HashMap::new();
        params.insert("statement".to_string(), serde_json::json!(vi.statement));
        push_req(
            reqs,
            seq,
            "商务要求",
            Some(format!("值-{}", vi.label)),
            vec![Anchor {
                file: f.name.clone(),
                kind: "table".into(),
                locator: vi.locator.clone(),
                excerpt: vi.statement.clone(),
            }],
            params,
            &f.name,
        );
    }
}


/// 纯 NFKC（不剥空白）——PDF 部首归一化专用。
///
/// `crate::norm_n1` 是「NFKC + 去全部空白」，用于对齐/检索面；
/// 这里要保留原文空白（locator 的「第N 页 > 第M 行」形态、excerpt 的原样）。

/// #272：剥掉章名括号内的补充说明（（参考格式）（二次）类）——括号外
/// 才是章的本题。深度剥离圆/方/书名括号；括号外为空时回退原文。
fn strip_paren_notes(s: &str) -> String {
    let mut core = String::with_capacity(s.len());
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '（' | '《' | '[' | '【' | '(' => depth += 1,
            '）' | '》' | ']' | '】' | ')' => depth -= 1,
            _ if depth <= 0 => core.push(c),
            _ => {}
        }
    }
    if core.trim().is_empty() {
        s.to_string()
    } else {
        core
    }
}

/// #272：格式章判定（部分/数字方案面专用）——括号外含「格式」才算。
///
/// 括号内的补充说明（实测：询价「第六部分 政府采购合同（参考格式）」）
/// 不是章的格式语义——裸 `contains("格式")` 会把整章合同条款误落格式
/// 要求（45-51 页 62 条实测误伤）。圆/方/书名括号内容逐字符深度剥离后
/// 再判。Zhang 分支不使用本函数（既有 contains 语义——行为红线）。
fn fmt_chapter_of(s: &str) -> bool {
    let mut core = String::with_capacity(s.len());
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '（' | '《' | '[' | '【' | '(' => depth += 1,
            '）' | '》' | ']' | '】' | ')' => depth -= 1,
            _ if depth <= 0 => core.push(c),
            _ => {}
        }
    }
    core.contains("格式")
}

/// 用子串猜（「评审」不会误配「评审结果公告」因为那是段落不是章名）。
fn category_for(chapter: &str, subsection: &str) -> &'static str {
    // ── 评分面（评标办法/评审/评标）：详细评审表 ↔ 评分项 ──
    if chapter.contains("评标办法")
        || chapter.contains("评审")
        || chapter.contains("评标")
    {
        return "评分项";
    }
    // ── #266 小节级商务命名优先于章级推断：「三、商务要求」住在采购需求/
    //    技术要求章内，章级映射会把商务条款行误归技术参数（songling
    //    REQ-0159/0160 实测——语义表接管后本修法兜住残留编号行）──
    if subsection.contains("商务要求") || subsection.contains("商务条款") {
        return "商务要求";
    }
    // ── 技术面（技术需求/技术规范/采购内容/采购需求/技术要求/服务要求）──
    if chapter.contains("技术需求")
        || chapter.contains("技术规范")
        || chapter.contains("采购内容")
        || chapter.contains("采购需求")
        || chapter.contains("技术要求")
        || chapter.contains("服务要求")
        || chapter.contains("货物需求")
    {
        return "技术参数";
    }
    // ── 商务面（商务要求/合同与验收/合同条款/商务条款）──
    if chapter.contains("商务要求")
        || chapter.contains("商务条款")
        || chapter.contains("合同与验收")
        || chapter.contains("合同条款")
        || chapter.contains("合同要求")
    {
        return "商务要求";
    }
    // ── 资格面（章名本身含资格/资信——真实文件「投标人应当提交的资格、
    //    资信证明文件」不经过「须知」章，原实现的 `须知 && 资格` 双条件挡死）──
    if chapter.contains("资格") || chapter.contains("资信") || subsection.contains("资格")
    {
        return "资格条件";
    }
    // ── 否决面（否决/无效投标/废标）──
    if chapter.contains("否决") || chapter.contains("无效投标") || chapter.contains("废标")
        || subsection.contains("否决")
    {
        return "否决项";
    }
    // ── 格式面（投标文件格式/格式要求/编制要求）──
    if chapter.contains("格式") || subsection.contains("编制") {
        return "格式要求";
    }
    "其他"
}

/// 主文件（A）与补遗（C）：编号条款段落 + 前附表否决行 + 技术需求参数表。
/// DOCX 主文件（合成夹具 corpus-A 形态）；PDF 主文件（真实招标文件形态——P2
/// 评测驱动新增，语义与 DOCX 版一致：章节上下文 → 编号条款抽取）。
///
/// P0-B（issue #35）：DOCX 侧补齐 R3-S3-5 优先级链（原仅 PDF 侧实现——
/// DOCX 组否决分类召回 0.0000，分类信号源整体缺失）：
/// - 编号段落：veto_lexicon 强否决判定 > 资格面 > 章节上下文映射
/// - 无编号普通段：命中否决/资格词表 → 抽为 REQ（真实示范文本大量
///   无编号条款：资格负面清单/审查表导语）
/// - 语境传播：「下列情形之一的，…否决：」类总括句开启否决语境，
///   其后列举条目（（N）/N、）继承否决语义，至新编号条款终止
/// - TableRow：不限前附表——拼接文本命中强否决 → 否决项 REQ
///   （BJ 示范文本否决条款大量在须知/审查表格内，L1 miss 主体）
fn extract_main_docx(f: &ParsedFile, reqs: &mut Vec<Requirement>, seq: &mut usize) {
    use crate::veto_lexicon as vl;
    let mut last_numbered_head: Option<String> = None; // 表格行收养最近的编号段（5.x 参数表上下文）
    // #218 任务 3：与 PDF 侧同口径——多级点分 + 中文顿号（1、）双形态
    let re_clause = Regex::new(r"^(\d+(?:\.\d+)+|\d{1,3}、)\s*").unwrap();
    let re_dunhao_list = Regex::new(r"^\d{1,3}、").unwrap();
    let re_model = model_regex();
    let re_amount = Regex::new(r"(\d+(?:\.\d+)?)\s*万\s*元").unwrap();
    let re_date = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
    let re_date_cn = Regex::new(r"[〇一二三四五六七八九零]{4}年[〇一二三四五六七八九零]{1,2}月[〇一二三四五六七八九零]{1,3}日").unwrap();
    // 分值形态（评分细则行「N分」——容忍空格；评标办法章内分值锚定的
    // 判定面，P0-B 修订 2）
    let re_score_shape = Regex::new(r"\d+(?:\.\d+)?\s*分").unwrap();
    // 否决语境（父条款总括句开启；新编号条款/非列举普通段终止）
    let mut veto_ctx = false;
    // 表格审查语境（评审标准/资格审查表内行；表切换重置）
    let mut review_ctx = false;
    let mut cur_table: Option<String> = None;
    // 章节头形态（G1 续行吸收的硬断点——与 PDF 侧 re_chapter 同口径；
    // 编号头之外，「第X章/篇/部分」标题也开新语义单元，不得当续行吸收）
    let re_chapter_line = Regex::new(r"^第\s*[0-9一二三四五六七八九十百]+\s*(章|篇|部分)").unwrap();

    let blocks = &f.blocks;
    // #144 审查表接管（DOCX 表格侧）：块位 → 结构化条目（区域接管防双抽）。
    // 表7/表8 → 资格条件（必选废标面——矩阵 blocking）；表9 → 评分项 +
    // params.score 承载标准分（整数形态——矩阵 as_i64 消费口径）。
    let review_map: HashMap<usize, crate::review_table::ReviewItem> =
        crate::review_table::extract_docx_tables(f)
            .into_iter()
            .flat_map(|it| {
                let idxs = it.block_idxs.clone();
                idxs.into_iter().map(move |bi| (bi, it.clone()))
            })
            .collect();
    // #250 表格专路接管：参数表/商务表按「表头+数据行」结构化装配（一行
    // 一条参数/要求，数值单位完整）。区域接管防双抽——被接管的块位不走
    // 旧碎片链；优先级：审查表（#144 红线）> 语义表 > 旧链。
    let semantic = crate::table_semantics::extract_docx_semantic_tables(f);
    // 语义表条款号前缀：表号（locator「表N」提取——「参数表」类无名表用
    // 「表N-行M」形态，与前附表-N / 表-N 既有条款号族同构）
    let sem_clause_no = |b: &Block| -> String {
        let key = table_key_of(&b.locator);
        match key.rfind(" 表") {
            Some(i) => format!("{}-{}", &key[i + 1..], row_no_of(&b.locator)),
            None => format!("表-{}", row_no_of(&b.locator)),
        }
    };
    let mut bi = 0usize;
    while bi < blocks.len() {
    let b = &blocks[bi];
        match b.kind {
            BlockKind::Para => {
                // 表格流结束：审查语境随表终止
                review_ctx = false;
                cur_table = None;
                let t = b.text.trim();
                // 顿号列举双守卫（与 PDF 侧同口径，见 extract_main_pdf）：
                // veto_ctx 开放 → 传播分支（「否决总括句后的列举条目」语义）
                // 继承否决面；格式章（locator 派生章名）模板不进编号分支。
                let (loc_chapter, _) = split_locator(&b.locator);
                let dunhao_defer = re_dunhao_list.is_match(t)
                    && (veto_ctx || loc_chapter.contains("格式"));
                if let Some(m) = re_clause.captures(t).filter(|_| !dunhao_defer) {
                    // 新编号条款：语境终止（本条自身若再开启则重启）
                    veto_ctx = false;
                    // 顿号形态条款号取纯数字（「1、」→「1」——与 PDF 侧同口径）
                    let cno = m[1].trim_end_matches('、').to_string();
                    last_numbered_head = Some(cno.clone());
                    let (chapter, subsection) = split_locator(&b.locator);
                    // B6·G1 跨行合并（#123 任务 1）：吸收后续无编号续行——
                    // docx 解析按段产出，分页符/软换行把一条条款切成多段
                    // （synth T4 实锤：3.7.2(8) docx 拆 2 条 REQ），前半句
                    // 判定**内容不全**。续行仅在无独立编号/强信号时吸收
                    // （子项（N）/强词段保持独立 REQ——任务书规则 1：编号
                    // 行开新条）；excerpt 逐行保真（与 PDF 侧 para_anchors
                    // 同纪律）。
                    let mut grp_end = bi;
                    let mut grp_text = t.to_string();
                    let mut grp_anchors = vec![anchor(f, b)];
                    for j in (bi + 1)..blocks.len() {
                        let nb = &blocks[j];
                        if !matches!(nb.kind, BlockKind::Para) {
                            break;
                        }
                        let nt = nb.text.trim();
                        // 任务书规则 1：任何编号形态（4.2/（一）/①/一、/1./a)）
                        // 都开新条款——续行吸收到「首个编号形态」为止
                        // （clause_boundary 为 G1 的边界判定单一真源）。
                        if nt.is_empty()
                            || clause::clause_boundary(nt).is_some()
                            || re_chapter_line.is_match(nt)
                            || vl::is_placeholder(nt)
                            || vl::is_veto(nt)
                            || vl::is_qual(nt)
                            || vl::is_list_item(nt)
                        {
                            break;
                        }
                        grp_text.push_str(nt);
                        grp_anchors.push(anchor(f, nb));
                        grp_end = j;
                    }
                    let mut params = HashMap::new();
                    fill_params(&grp_text, &re_model, &re_amount, &re_date, &re_date_cn, &mut params);
                    // R3-S3-5 优先级链（DOCX 版，对齐 extract_main_pdf）：
                    // 强否决短语 > 资格面 > 章节上下文——编号否决条款
                    // （如"3.4.2 …将被否决"）不得落"其他"（漏否决=废标风险）
                    // P0-B 修订 2（分值形态锁定）：评标办法章内**含分值形态**
                    // （"N分"）的行才锁定评分项。修订 1 的整章 blanket 优先
                    // 误伤评审否决条款（3.1.2 下列情形之一的将被否决 /
                    // 3.2.x 低于成本价应当否决——实测 37 条、DOCX L2 -4.9pp）。
                    // 实测面（探针）：corpus-A 4 条评分项全部含 N分形态；
                    // BJ 语料 37 条被误伤否决条款全部无分值形态——两全无歧义面。
                    // #110 任务 3（G4 修复）：分值形态锁加**否决旁路**——
                    // 「形态+否决处置并存」时否决优先（synth-tender T2b：
                    // 「4.6 …每项扣 1 分（最多扣 5 分）；出现实质性负偏离的，
                    // 作否决投标处理」被形态锁吞为评分项）。GT 实证口径一致：
                    // 真实语料唯一并存条目（「每有一项错误扣1分，最多扣5分，
                    // 修正后的报价与投标报价偏差大于 5%，按废标处理」）标
                    // veto true。与 PDF 路径对齐（469 行 veto_hit 分支先于
                    // 评分分支——PDF 侧本就否决优先，本修复补齐 DOCX 侧）。
                    // 零扰动面：corpus-A/BJ 实测「形态+否决并存」除 GT 那
                    // 1 条 true 外为 0——纯增益修复。
                    let ctx_cat = category_for(chapter, subsection);
                    // #226/#233 层 2（DOCX 编号分支，对齐 PDF :530 模式）：
                    // 强否决短语若其实是「招标人规则陈述」（本次招标/逾期
                    // 不予受理…），不判否决项——它没有可请求的响应正文，
                    // 判了就是永久 blocking 死锁。分类降级到资格面/章节
                    // 上下文，条款本体照常 push（编号分支本就无条件落账）。
                    // 分值形态锁的否决旁路（#110 G4）只对**非规则说明**的
                    // 真否决生效——规则说明形不参与否决旁路。
                    let cat = if ctx_cat == "评分项" && re_score_shape.is_match(&grp_text) && !vl::is_veto(&grp_text) {
                        "评分项"
                    } else if vl::is_veto(&grp_text)
                        && !crate::veto_rule::is_rule_statement(&grp_text)
                    {
                        "否决项"
                    } else if ctx_cat == "评分项" {
                        "评分项"
                    } else if vl::is_qual(&grp_text) {
                        "资格条件"
                    } else {
                        ctx_cat
                    };
                    push_req(reqs, seq, cat, Some(cno),
                             grp_anchors, params, &f.name);
                    if vl::opens_veto_context(&grp_text) {
                        veto_ctx = true;
                    }
                    // 吸收的续行块已并入本条（防止下方循环重复消费）
                    bi = grp_end;
                } else if vl::is_placeholder(t) {
                    // 待填占位（□复选/下划线留白）不抽
                } else if vl::is_veto(t) {
                    // 无编号普通段：强否决信号 → 否决项 REQ（真实示范文本
                    // 大量无编号条款：资格负面清单/审查表导语——L1 修复面）。
                    // #226/#233 层 2（对齐 PDF :553 修复版）：规则说明段
                    // （「招标人不予受理…」类）分类降级（资格面/章节上下文）
                    // ——**条款本体不丢**（整段跳过会让 REQ 掉进兜底 else
                    // 不落账，#230 已在 PDF 侧踩过，DOCX 同款防线）。
                    if crate::veto_rule::is_rule_statement(t) {
                        // 章节上下文从 locator 重拆（编号分支 :816 同款——
                        // 无编号段没有现成变量）
                        let (chapter, subsection) = split_locator(&b.locator);
                        let cat = if vl::is_qual(t) {
                            "资格条件"
                        } else {
                            category_for(chapter, subsection)
                        };
                        push_req(reqs, seq, cat, None, vec![anchor(f, b)], HashMap::new(), &f.name);
                        veto_ctx = false;
                    } else {
                        push_req(reqs, seq, "否决项", None, vec![anchor(f, b)], HashMap::new(), &f.name);
                        if vl::opens_veto_context(t) {
                            veto_ctx = true;
                        }
                    }
                } else if vl::is_qual(t) {
                    push_req(reqs, seq, "资格条件", None, vec![anchor(f, b)], HashMap::new(), &f.name);
                    veto_ctx = false;
                } else if veto_ctx && vl::is_list_item(t) {
                    // 语境传播：否决总括句后的列举条目（（N）/N、/①）继承否决语义。
                    // 仅列举标志继承——非列举普通段一律终止（防语境流污染
                    // 到后续无关条款，修正规则/澄清通知类模板误报面）
                    push_req(reqs, seq, "否决项", None, vec![anchor(f, b)], HashMap::new(), &f.name);
                } else {
                    veto_ctx = false;
                }
            }
            BlockKind::TableRow => {
                // #144 审查表接管：数据行 → 结构化 REQ（旧链不走——防双抽）；
                // 表头形状行只消费（防旧链误开语境/误产 REQ）。
                if let Some(ri) = review_map.get(&b.idx) {
                    let mut params = HashMap::new();
                    if let Some(sc) = ri.score {
                        params.insert(
                            "score".into(),
                            if sc.fract() == 0.0 {
                                serde_json::json!(sc as i64)
                            } else {
                                serde_json::json!(sc)
                            },
                        );
                    }
                    if let Some(rules) = &ri.score_rules {
                        params.insert(
                            "score_rules".into(),
                            serde_json::Value::String(rules.clone()),
                        );
                    }
                    let cat = match ri.kind {
                        crate::review_table::ReviewTableKind::Detailed => "评分项",
                        _ => "资格条件",
                    };
                    push_req(
                        reqs, seq, cat,
                        Some(format!("{}-{}", ri.table_name, ri.row)),
                        vec![anchor(f, b)], params, &f.name,
                    );
                } else if let Some(ri) = semantic.params.get(&b.idx) {
                    // #250 表格专路：参数表数据行（区域接管——不走旧
                    // re_model 碎片链）。params.statement 承载语义组装
                    //（数值单位完整），excerpt 锚保真（块原文）。
                    let mut params = HashMap::new();
                    params.insert(
                        "statement".into(),
                        serde_json::Value::String(ri.statement.clone()),
                    );
                    if ri.star {
                        params.insert("star".into(), serde_json::json!(true));
                    }
                    push_req(reqs, seq, "技术参数", Some(sem_clause_no(b)),
                             vec![anchor(f, b)], params, &f.name);
                } else if let Some(ri) = semantic.commercial.get(&b.idx) {
                    // #250 表格专路：商务表数据行（此前无任何商务抽取
                    // 分支——实测商务要求 0 条，保证金/有效期/履约全丢）。
                    let mut params = HashMap::new();
                    params.insert(
                        "statement".into(),
                        serde_json::Value::String(ri.statement.clone()),
                    );
                    if ri.star {
                        params.insert("star".into(), serde_json::json!(true));
                    }
                    push_req(reqs, seq, "商务要求", Some(sem_clause_no(b)),
                             vec![anchor(f, b)], params, &f.name);
                } else if crate::review_table::recognize_header(&b.cells).is_some() {
                    // 审查表表头行——只消费不产出
                } else {
                // 表切换检测（locator 表号变化）
                let tbl_key = table_key_of(&b.locator);
                if cur_table.as_deref() != Some(tbl_key.as_str()) {
                    cur_table = Some(tbl_key);
                    review_ctx = false;
                }
                let joined = b.cells.join("");
                if b.locator.contains("前附表") && (joined.contains("否决") || b.text.contains("否决")) {
                    // 前附表否决行：条款号 = 前附表-{序号}（序号列为准；缺序号回退 XML 行号）
                    let item = b.cells.iter()
                        .find(|c| c.chars().all(|x| x.is_ascii_digit()) && !c.is_empty())
                        .and_then(|c| c.parse::<usize>().ok())
                        .unwrap_or_else(|| row_no_of(&b.locator));
                    push_req(reqs, seq, "否决项", Some(format!("前附表-{item}")),
                             vec![anchor(f, b)], HashMap::new(), &f.name);
                } else if (vl::is_veto(&joined) || (veto_ctx && vl::is_list_item(&joined)))
                    && !vl::is_placeholder(&joined) {
                    // P0-B：表格行否决面扩展（不限前附表）——BJ 示范文本
                    // 否决条款大量在须知表格/审查表内。条款号 = 行号定位
                    push_req(reqs, seq, "否决项", Some(format!("表-{}", row_no_of(&b.locator))),
                             vec![anchor(f, b)], HashMap::new(), &f.name);
                } else if review_ctx && !joined.trim().is_empty() && joined.chars().count() >= 4
                    && !vl::is_placeholder(&joined) && vl::is_review_standard(&joined) {
                    // 审查表条目：评审标准/资格审查表内的行（无否决词的
                    // 隐含否决面——「不符合评审标准的否决其投标」）
                    push_req(reqs, seq, "否决项", Some(format!("表-{}", row_no_of(&b.locator))),
                             vec![anchor(f, b)], HashMap::new(), &f.name);
                } else if vl::opens_review_context(&joined) || vl::opens_review_context(&b.text) {
                    // 表格审查语境开启（评审标准表头/资格审查行）
                    review_ctx = true;
                } else if (b.locator.contains("技术需求") || b.locator.contains("第五章"))
                    && re_model.is_match(&b.text)
                {
                    // 参数表行：收养最近的 5.x 编号头作条款号
                    let mut params = HashMap::new();
                    fill_params(&b.text, &re_model, &re_amount, &re_date, &re_date_cn, &mut params);
                    push_req(reqs, seq, "技术参数", last_numbered_head.clone(),
                             vec![anchor(f, b)], params, &f.name);
                }
                } // #144：审查表接管 else 块闭合
            }
            _ => {}
        }
        bi += 1;
    }
}

/// B1 设备清单：含型号或中文大写日期的行 → 技术参数 REQ（到货/规格要求行）。
fn extract_b1_lines(f: &ParsedFile, reqs: &mut Vec<Requirement>, seq: &mut usize) {
    let re_model = model_regex();
    let re_date_cn = Regex::new(r"[〇一二三四五六七八九零]{4}年[〇一二三四五六七八九零]{1,2}月[〇一二三四五六七八九零]{1,3}日").unwrap();
    let re_amount = Regex::new(r"(\d+(?:\.\d+)?)\s*万\s*元").unwrap();
    let re_date = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
    for b in &f.blocks {
        if matches!(b.kind, BlockKind::PdfLine)
            && (re_model.is_match(&b.text) || re_date_cn.is_match(&b.text))
        {
            let mut params = HashMap::new();
            fill_params(&b.text, &re_model, &re_amount, &re_date, &re_date_cn, &mut params);
            push_req(reqs, seq, "技术参数", None, vec![anchor(f, b)], params, &f.name);
        }
    }
}

/// B2 扫描件：否决关键词 OCR 行 + 1 行续接窗 → 否决项 REQ（扫描件内条款不丢的抽取面）。
fn extract_b2_ocr(f: &ParsedFile, reqs: &mut Vec<Requirement>, seq: &mut usize) {
    let keywords = ["否决", "签署", "盖章"];
    let blocks = &f.blocks;
    for (i, b) in blocks.iter().enumerate() {
        if !matches!(b.kind, BlockKind::OcrLine) {
            continue;
        }
        if !keywords.iter().any(|k| b.text.contains(k)) {
            continue;
        }
        // 种子行 + 下一行拼接（断行条款还原）；excerpt 同时携带两行原文
        let mut excerpt = b.text.clone();
        let mut locator = b.locator.clone();
        if let Some(next) = blocks.get(i + 1) {
            if matches!(next.kind, BlockKind::OcrLine) && next.text.chars().count() >= 2 {
                excerpt.push_str(&next.text);
                locator.push_str(" + 行接");
            }
        }
        push_req(reqs, seq, "否决项", None,
                 vec![Anchor { file: f.name.clone(), kind: "ocr".into(), locator, excerpt }],
                 HashMap::new(), &f.name);
    }
}

/// 评分项分值提取（T-3 评分矩阵输入面——issue #11 批次 v3）：
/// 评分细则行「N、报价：65 分」/ 表格行「…30 分」→ `params.score`。
/// 纯确定性正则（零模型）；「满分 100 分」类总分层可能误配为单行分值——
/// 领域层 scoring sanity（总分 >100 告警）兑底，不在本层猜测语义。
fn score_of(text: &str) -> Option<f64> {
    static RE_SCORE: OnceLock<Regex> = OnceLock::new();
    RE_SCORE
        .get_or_init(|| Regex::new(r"(\d+(?:\.\d+)?)\s*分").unwrap())
        .captures(text)
        .and_then(|c| c[1].parse().ok())
}

fn push_req(reqs: &mut Vec<Requirement>, seq: &mut usize, category: &str,
            clause_no: Option<String>, anchors: Vec<Anchor>,
            mut params: HashMap<String, serde_json::Value>, source: &str) {
    // T-3：评分项统一补分值槽位（所有推入路径单一插入点——段落/表格/前附表）
    if category == "评分项" && !params.contains_key("score") {
        let joined: String = anchors.iter().map(|a| a.excerpt.as_str()).collect();
        if let Some(s) = score_of(&joined) {
            params.insert("score".into(), serde_json::json!(s));
        }
    }
    // #250：来源字段完整化——正则产物统一 `marked_by: "regex"`（Agent 项
    // 由 merge.rs 写 "agent"）。全量 REQ 的来源面对称：regex / agent /
    // human（human 面当前无录入通路——工具面落地时同键扩展）。
    params.insert("marked_by".into(), serde_json::json!("regex"));
    *seq += 1;
    reqs.push(Requirement {
        id: format!("REQ-{seq:04}"),
        category: category.to_string(),
        clause_no,
        anchors,
        params,
        source_file: source.to_string(),
    });
}

fn anchor(f: &ParsedFile, b: &Block) -> Anchor {
    Anchor {
        file: f.name.clone(),
        kind: match b.kind {
            BlockKind::Para => "para",
            BlockKind::TableRow => "table",
            BlockKind::PdfLine => "pdf_line",
            BlockKind::OcrLine => "ocr",
            BlockKind::XlsxRow => "xlsx_row",
        }
        .to_string(),
        locator: b.locator.clone(),
        excerpt: b.text.clone(), // 溯源锚原文（创建时固化）
    }
}

fn fill_params(text: &str, re_model: &Regex, re_amount: &Regex, re_date: &Regex,
               re_date_cn: &Regex, params: &mut HashMap<String, serde_json::Value>) {
    if let Some(m) = re_model.find(text) {
        params.insert("model".into(), serde_json::Value::String(m.as_str().to_string()));
    }
    if let Some(m) = re_amount.captures(text) {
        let v: f64 = m[1].parse().unwrap_or(0.0);
        params.insert("amount_wan".into(), serde_json::json!(v));
    }
    if let Some(m) = re_date.find(text) {
        params.insert("date".into(), serde_json::Value::String(m.as_str().to_string()));
    }
    if let Some(m) = re_date_cn.find(text) {
        params.insert("date_cn".into(), serde_json::Value::String(m.as_str().to_string()));
    }
}

fn split_locator(locator: &str) -> (&str, &str) {
    match locator.split_once(" > ") {
        Some((c, s)) => (c, s),
        None => (locator, ""),
    }
}

/// #234：章号行向前看章标题行（真实件两行章形态——pdftotext 把
/// 「第8章」与「投标文件格式」切成两行，章号行自身不含标题，
/// `chapter` 字符串判不出格式章）。窗口 3 块：跳过空行/纯页码行，
/// 首个内容行满足标题形态（≤20 字、无句末标点）即返回；长行/带
/// 标点（正文段非标题）返回 None。
fn chapter_title_ahead(blocks: &[Block], idx: usize) -> Option<String> {
    for b in blocks.iter().skip(idx + 1).take(3) {
        if !matches!(b.kind, BlockKind::PdfLine | BlockKind::OcrLine) {
            return None;
        }
        let s = b.text.trim();
        if s.is_empty() || s.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let ends_sentence = s
            .chars()
            .last()
            .map(|c| "。；！？：；".contains(c))
            .unwrap_or(false);
        if s.chars().count() <= 20 && !ends_sentence {
            return Some(s.to_string());
        }
        return None;
    }
    None
}

fn row_no_of(locator: &str) -> usize {
    locator.rsplit("行").next().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

/// 表格键（locator 的 表N 序号——表切换检测用，P0-B 审查语境隔离）。
fn table_key_of(locator: &str) -> String {
    // 「第二章 投标人须知 表1 行41」→「第二章 投标人须知 表1」
    match locator.find(" 行") {
        Some(i) => locator[..i].to_string(),
        None => locator.to_string(),
    }
}
