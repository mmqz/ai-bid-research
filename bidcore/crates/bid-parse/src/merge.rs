//! merge.rs —— #219 第二步：抽取通路合并（Agent 优先 + 正则兜底）。
//!
//! 两条通路并存的现状（PR #227 后）：
//! - A 通路（正则确定性）：`extract::extract_requirements`——字符形态匹配，
//!   覆盖不了无编号段落/表格语义，且把「规则说明」误判为要求（#226）；
//! - B 通路（Agent 判断）：`submit_block_marks`——区间标记 + 关键点提交。
//!
//! 本模块把两通路合并成一个最终条款集，规则（issue #219）：
//! - **Agent 优先**：被任何区间标记覆盖的块上，正则产物一律丢弃
//!   （无论 verdict 是 requirement / noise / rule / review_table——
//!   Agent 已看过该块并给出结论，正则不再插嘴）；
//! - **正则兜底**：未被任何标记覆盖的块（uncovered），正则产物保留；
//! - **冲突以 Agent 为准**：Agent 标了 requirement 的块，其产出以
//!   [`SubmittedRequirement`] 转换的 REQ 为准（`REQ-Axxxx` 编号）。
//!
//! **无标记时严格 no-op**（GT 红线保护）：空 marks + 空 submitted →
//! 输出与输入逐位一致、编号不动——`real_corpus_e2e`（只走正则全链）
//! 的 L1/否决红线零位移。
//!
//! 本模块是纯函数（零 IO）：输入 parsed blocks + marks + submitted +
//! 正则产物，输出最终集与统计。语义判断全在上游（Agent/正则）。

use std::collections::HashMap;

use bid_domain::coverage::{BlockMark, SubmittedRequirement};
use bid_domain::{Anchor, Requirement};

use crate::{Block, BlockKind};

/// 每文件的合并输入面（调用方组装——tools.rs 按 TenderFile sha256 join parsed）。
pub struct MergeFace<'a> {
    /// 文件 ID（TF-xxxx——marks/submitted 的存储键）。
    pub file_id: &'a str,
    /// 解析产物名（anchor.file / source_file 的对齐键）。
    pub name: &'a str,
    /// 块面（索引与定位的唯一真源）。
    pub blocks: &'a [Block],
    /// Agent 区间标记（可能为空）。
    pub marks: &'a [BlockMark],
    /// Agent 提交的关键点（可能为空——须已通过 validate_submitted）。
    pub submitted: &'a [SubmittedRequirement],
}

/// 合并统计（响应面——extract_requirements 的 `merge` 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MergeStats {
    /// 正则抽取总数（合并过滤前）。
    pub regex_total: usize,
    /// 因块被 Agent 标记覆盖而丢弃的正则 REQ 数。
    pub regex_dropped_covered: usize,
    /// Agent 提交转化入库的 REQ 数（REQ-Axxxx）。
    pub agent_requirements: usize,
    /// 有抽取面文件中未被标记覆盖的块数。
    pub uncovered_blocks: usize,
    /// 有抽取面文件是否全部覆盖。
    pub coverage_complete: bool,
    /// Agent 标 mandatory=true 且类别不在类别阻断面（资格条件/否决项）的条数。
    ///
    /// 历史名 `agent_mandatory_not_blocking`（AGENT_MANDATORY_NOT_BLOCKING
    /// 警告时代——那批条款确实不拦）。Agent 必选标记接线进矩阵 blocking 面
    /// 后这些条款**会拦**（matrix.rs `agent_mandatory`），改名 offcategory
    /// 防语义漂移：本统计只计「靠 Agent 标记而非类别进入阻断面」的条数，
    /// 供 extract 响应面的信息性提示（AGENT_MANDATORY_OFFCATEGORY warning）。
    pub agent_mandatory_offcategory: usize,
}

/// 该文件是否有条款抽取面（#250：按业务 kind 判定，与
/// `extract::extract_requirements_with_kind` 的 #196/#199 分发口径同源——
/// **反向清单**：仅附件/图纸/证据无条款面，其余登记 kind 一律有面）。
///
/// - `kind = Some(...)`（生产路径——导入时登记，`TenderFile.kind: String`
///   恒有值）：按业务事实判定。真实用户文件（如「光纤租赁费招标文件.docx」）
///   与夹具文件（「A-招标文件.docx」）走**完全相同**的合并路径；
/// - `kind = None`（夹具直调/未登记）：文件名前缀兜底（A-/C-/B1/B2）——
///   前缀是测试夹具的命名约定，**不是生产判据**（#196 修 extract.rs 时
///   已判定为错，本处是同一根因的第二次出现——issue #250）。
///   B3（图纸目录/证据件）无抽取面 → 不进合并统计（uncovered 语义 =
///   「抽取面的未覆盖」，B3 无兜底可漏）。
///
/// 与 extract 分发的保守性差异：extract 对「None 且无前缀」仍主抽取
/// （fail-visible——宁多勿漏），本门禁判无面（正则产物不经覆盖过滤、
/// 不进 uncovered 统计——no-op 保守方向）。生产路径 kind 恒登记，该
/// 差异仅存在于直调测试面。
pub fn has_extraction_face(kind: Option<&str>, name: &str) -> bool {
    match kind {
        // #271：「格式模板」同入无面清单（与 extract.rs is_attachment 同口径）
        // ——格式件是填空模板非条款源，不进覆盖/矩阵面。
        Some("附件") | Some("图纸") | Some("证据") | Some("格式模板") => false,
        Some(_) => true,
        None => name.starts_with("A-")
            || name.starts_with("C-")
            || name.starts_with("B1")
            || name.starts_with("B2"),
    }
}

/// 阻断面类别（is_mandatory 同源口径——matrix.rs）。
const BLOCKING_CATEGORIES: [&str; 2] = ["资格条件", "否决项"];

fn block_kind_str(k: BlockKind) -> &'static str {
    match k {
        BlockKind::Para => "para",
        BlockKind::TableRow => "table",
        BlockKind::PdfLine => "pdf_line",
        BlockKind::OcrLine => "ocr",
        BlockKind::XlsxRow => "xlsx_row",
    }
}

/// 合并两通路（纯函数）。
///
/// 输出的编号纪律：
/// - 正则保留项**原 id 原序不动**（REQ-0001..N 的子序列——无标记时与
///   旧产物逐位一致）；
/// - Agent 项追加在后，id = `REQ-A{seq:04}`（seq 按 face 顺序 + block_idx
///   稳定序全局递增）。`A` 前缀保证与正则数字编号零碰撞；CMP 派生 id
///   走非数字后缀回退分支（`CMP-0000-REQ-A0001`），与正则 `CMP-000N`
///   零碰撞（matrix.rs `derived_cmp_id`）。
pub fn merge_extraction(
    faces: &[MergeFace<'_>],
    mut regex_reqs: Vec<Requirement>,
) -> (Vec<Requirement>, MergeStats) {
    let regex_total = regex_reqs.len();

    // ---- ① 每文件两个索引：
    //      块定位（locator+text → idx）：GT 块对齐同款惯用法——首个命中
    //      （real_corpus_e2e.rs 的 anchor→块匹配用 `find`，这里用
    //      entry().or_insert() 保持「首块优先」语义）；
    //      覆盖集（idx → ()）：被任何区间标记覆盖即算（verdict 不区分——
    //      Agent 优先是块级裁决，正则不再插嘴）。
    struct FaceIdx<'a> {
        covered: HashMap<usize, ()>,
        index: HashMap<(&'a str, &'a str), usize>,
    }
    let mut by_name: HashMap<&str, FaceIdx<'_>> = HashMap::with_capacity(faces.len());
    for f in faces {
        let mut covered: HashMap<usize, ()> = HashMap::new();
        for m in f.marks {
            for i in m.from..=m.to {
                covered.insert(i, ());
            }
        }
        let mut index: HashMap<(&str, &str), usize> = HashMap::with_capacity(f.blocks.len());
        for b in f.blocks {
            index
                .entry((b.locator.as_str(), b.text.as_str()))
                .or_insert(b.idx);
        }
        by_name.insert(f.name, FaceIdx { covered, index });
    }

    // ---- ② 过滤：正则 REQ 的任一 anchor 命中已覆盖块 → 丢弃。
    //      anchor 匹配不到块（跨版本解析差异等防御分支）→ 保留
    //      （宁多勿漏——L1 召回纪律）。多锚点 REQ：任一命中即弃
    //      （保守——Agent 优先）。
    let before = regex_reqs.len();
    regex_reqs.retain(|r| {
        !r.anchors
            .iter()
            .any(|a| match by_name.get(a.file.as_str()) {
                Some(fi) => fi
                    .index
                    .get(&(a.locator.as_str(), a.excerpt.as_str()))
                    .is_some_and(|&idx| fi.covered.contains_key(&idx)),
                None => false,
            })
    });
    let regex_dropped_covered = before - regex_reqs.len();

    // ---- ③ 转换：submitted → REQ-Axxxx（按 face 顺序 + block_idx 稳定序）。
    let mut agent_reqs: Vec<Requirement> = Vec::new();
    let mut seq = 0usize;
    let mut offcategory = 0usize; // Agent 必选且类别不在类别阻断面（见 MergeStats 字段注）
    for f in faces {
        let mut subs: Vec<&SubmittedRequirement> = f.submitted.iter().collect();
        // 稳定排序：同 block_idx 保持提交序（sort_by_key 是稳定排序）
        subs.sort_by_key(|s| s.block_idx);
        for s in subs {
            // 块必在（validate_submitted 已挡越区间，此处防御性跳过）
            let Some(block) = f.blocks.iter().find(|b| b.idx == s.block_idx) else {
                continue;
            };
            seq += 1;
            let category = s.category.clone().unwrap_or_else(|| "其他".to_string());
            if s.mandatory && !BLOCKING_CATEGORIES.contains(&category.as_str()) {
                offcategory += 1;
            }
            let mut params: HashMap<String, serde_json::Value> = HashMap::new();
            params.insert("statement".into(), serde_json::json!(s.statement));
            params.insert("mandatory".into(), serde_json::json!(s.mandatory));
            params.insert("block_idx".into(), serde_json::json!(s.block_idx));
            params.insert("marked_by".into(), serde_json::json!("agent"));
            // #276：★实质性条款显式落库（bool 都写——false = Agent 看过并判非★，
            // 区别于抽取面的「仅在检测到★时写 true」；派生面统一按 == true 判）
            params.insert("star".into(), serde_json::json!(s.substantive));
            agent_reqs.push(Requirement {
                id: format!("REQ-A{seq:04}"),
                category,
                clause_no: None,
                anchors: vec![{
                    let (page, line) = crate::page_line_of(&block.locator);
                    Anchor {
                        file: f.name.to_string(),
                        kind: block_kind_str(block.kind).to_string(),
                        locator: block.locator.clone(),
                        // 溯源锚 = 块原文（与正则 anchor 同纪律：创建时固化）
                        excerpt: block.text.clone(),
                        // #285 ② 块级定位回填（Agent 项锚定块——s.block_idx 已
                        // 校验落在 requirement 区间内，此处必中）
                        block_idx: Some(block.idx),
                        page,
                        line,
                    }
                }],
                params,
                source_file: f.name.to_string(),
            });
        }
    }
    let agent_requirements = agent_reqs.len();

    // ---- ④ 覆盖统计（有抽取面文件的口径）。
    let mut uncovered_blocks = 0usize;
    let mut coverage_complete = true;
    for f in faces {
        let total = f.blocks.len();
        if let Some(fi) = by_name.get(f.name) {
            let covered_n = fi.covered.len().min(total);
            uncovered_blocks += total - covered_n;
            if covered_n < total {
                coverage_complete = false;
            }
        }
    }

    // ---- ⑤ 合并：正则保留项（原 id 原序）在前，Agent 项追加。
    regex_reqs.extend(agent_reqs);

    (
        regex_reqs,
        MergeStats {
            regex_total,
            regex_dropped_covered,
            agent_requirements,
            uncovered_blocks,
            coverage_complete,
            agent_mandatory_offcategory: offcategory,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(idx: usize, locator: &str, text: &str) -> Block {
        Block {
            idx,
            kind: BlockKind::PdfLine,
            locator: locator.to_string(),
            text: text.to_string(),
            cells: vec![],
        }
    }

    fn mark(from: usize, to: usize) -> BlockMark {
        BlockMark {
            from,
            to,
            verdict: bid_domain::coverage::BlockVerdict::Noise,
            reason: String::new(),
        }
    }

    fn submitted(idx: usize, statement: &str) -> SubmittedRequirement {
        SubmittedRequirement {
            block_idx: idx,
            statement: statement.to_string(),
            category: None,
            mandatory: false,
            substantive: false,
        }
    }

    /// #276：★实质性条款标记面落库——substantive → params.star
    /// （bool 都写：true=Agent 判★，false=看过判非★；与 #280 抽取面的
    /// 「仅★时写 true」在派生面 == true 判定下统一）。
    #[test]
    fn agent_substantive_marks_params_star() {
        let blocks = vec![block(0, "p1", "★接到通知2小时内响应"), block(1, "p1", "普通条款")];
        let marks = vec![BlockMark {
            from: 0,
            to: 1,
            verdict: bid_domain::coverage::BlockVerdict::Requirement,
            reason: String::new(),
        }];
        let subs = vec![
            SubmittedRequirement {
                block_idx: 0,
                statement: "★接到通知2小时内响应".into(),
                category: Some("技术参数".into()),
                mandatory: false,
                substantive: true,
            },
            SubmittedRequirement {
                block_idx: 1,
                statement: "普通条款（非★）".into(),
                category: Some("技术参数".into()),
                mandatory: false,
                substantive: false,
            },
        ];
        let (out, _) = merge_extraction(&[face("A-主文件", &blocks, &marks, &subs)], vec![]);
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].params.get("star").and_then(|v| v.as_bool()),
            Some(true),
            "Agent 标★ → params.star = true"
        );
        assert_eq!(
            out[1].params.get("star").and_then(|v| v.as_bool()),
            Some(false),
            "Agent 显式判非★ → params.star = false（看过与没看过的审计区分）"
        );
        // ★与 mandatory 正交：★条款可以非必选（★技术参数不属类别阻断面）
        assert_eq!(
            out[0].params.get("mandatory").and_then(|v| v.as_bool()),
            Some(false),
            "substantive=true 不改变 mandatory=false（两条轴独立）"
        );
    }

    fn regex_req(id: &str, locator: &str, text: &str, name: &str) -> Requirement {
        Requirement {
            id: id.to_string(),
            category: "其他".to_string(),
            clause_no: None,
            anchors: vec![Anchor {
                file: name.to_string(),
                kind: "pdf_line".to_string(),
                locator: locator.to_string(),
                excerpt: text.to_string(),
                    block_idx: None,
                    page: None,
                    line: None
            }],
            params: HashMap::new(),
            source_file: name.to_string(),
        }
    }

    fn face<'a>(
        name: &'a str,
        blocks: &'a [Block],
        marks: &'a [BlockMark],
        submitted: &'a [SubmittedRequirement],
    ) -> MergeFace<'a> {
        MergeFace {
            file_id: "TF-0001",
            name,
            blocks,
            marks,
            submitted,
        }
    }

    #[test]
    fn no_marks_is_strict_noop() {
        // GT 红线保护：空 marks + 空 submitted → 逐位一致
        let blocks = vec![block(0, "p1", "1.1 要求A"), block(1, "p1", "1.2 要求B")];
        let reqs = vec![
            regex_req("REQ-0001", "p1", "1.1 要求A", "A-主文件"),
            regex_req("REQ-0002", "p1", "1.2 要求B", "A-主文件"),
        ];
        let f = face("A-主文件", &blocks, &[], &[]);
        let (out, stats) = merge_extraction(&[f], reqs.clone());
        assert_eq!(out, reqs);
        assert_eq!(stats.regex_total, 2);
        assert_eq!(stats.regex_dropped_covered, 0);
        assert_eq!(stats.agent_requirements, 0);
        assert_eq!(stats.uncovered_blocks, 2);
        assert!(!stats.coverage_complete);
    }

    #[test]
    fn covered_block_regex_dropped_any_verdict() {
        // Agent 优先是块级裁决：noise/rule/review_table/requirement 全丢
        let blocks = vec![
            block(0, "p1", "1.1 要求A"),
            block(1, "p1", "1.2 要求B"),
            block(2, "p1", "1.3 要求C"),
            block(3, "p1", "1.4 要求D"),
        ];
        let marks = vec![
            BlockMark {
                from: 0,
                to: 0,
                verdict: bid_domain::coverage::BlockVerdict::Noise,
                reason: String::new(),
            },
            BlockMark {
                from: 1,
                to: 1,
                verdict: bid_domain::coverage::BlockVerdict::Rule,
                reason: String::new(),
            },
            BlockMark {
                from: 2,
                to: 2,
                verdict: bid_domain::coverage::BlockVerdict::ReviewTable,
                reason: String::new(),
            },
            BlockMark {
                from: 3,
                to: 3,
                verdict: bid_domain::coverage::BlockVerdict::Requirement,
                reason: String::new(),
            },
        ];
        let reqs = vec![
            regex_req("REQ-0001", "p1", "1.1 要求A", "A-主文件"),
            regex_req("REQ-0002", "p1", "1.2 要求B", "A-主文件"),
            regex_req("REQ-0003", "p1", "1.3 要求C", "A-主文件"),
            regex_req("REQ-0004", "p1", "1.4 要求D", "A-主文件"),
        ];
        let f = face("A-主文件", &blocks, &marks, &[]);
        let (out, stats) = merge_extraction(&[f], reqs);
        assert!(out.is_empty(), "全覆盖 + 无提交 → 空");
        assert_eq!(stats.regex_dropped_covered, 4);
        assert_eq!(stats.coverage_complete, true);
    }

    #[test]
    fn uncovered_regex_kept_and_agent_appended() {
        // 正则兜底：块0被标 noise 丢弃；块1未覆盖保留；块2 requirement+提交→REQ-A
        let blocks = vec![
            block(0, "p1", "第一章 招标公告"),
            block(1, "p2", "2.1 未覆盖块"),
            block(2, "p2", "投标人须具备资质"),
        ];
        let marks = vec![
            mark(0, 0),
            BlockMark {
                from: 2,
                to: 2,
                verdict: bid_domain::coverage::BlockVerdict::Requirement,
                reason: String::new(),
            },
        ];
        let subs = vec![submitted(2, "须具备水利水电二级资质")];
        let reqs = vec![
            regex_req("REQ-0001", "p1", "第一章 招标公告", "A-主文件"),
            regex_req("REQ-0002", "p2", "2.1 未覆盖块", "A-主文件"),
            regex_req("REQ-0003", "p2", "投标人须具备资质", "A-主文件"),
        ];
        let f = face("A-主文件", &blocks, &marks, &subs);
        let (out, stats) = merge_extraction(&[f], reqs);
        assert_eq!(out.len(), 2, "块1 正则兜底 + 块2 Agent");
        assert_eq!(out[0].id, "REQ-0002", "正则保留项原 id 原序");
        assert_eq!(out[1].id, "REQ-A0001", "Agent 项追加 REQ-A 编号");
        assert_eq!(
            out[1].params.get("statement").unwrap(),
            "须具备水利水电二级资质"
        );
        assert_eq!(out[1].params.get("marked_by").unwrap(), "agent");
        assert_eq!(
            out[1].anchors[0].excerpt, "投标人须具备资质",
            "溯源锚=块原文"
        );
        assert_eq!(out[1].source_file, "A-主文件");
        assert_eq!(stats.regex_dropped_covered, 2);
        assert_eq!(stats.agent_requirements, 1);
        assert_eq!(stats.uncovered_blocks, 1, "块1 未覆盖");
        assert!(!stats.coverage_complete);
    }

    #[test]
    fn anchor_not_matching_block_is_kept() {
        // 防御分支：anchor 匹配不到块（解析版本差异）→ 保留（宁多勿漏）
        let blocks = vec![block(0, "p1", "1.1 要求A")];
        let marks = vec![mark(0, 0)];
        let reqs = vec![regex_req("REQ-0001", "p9", "完全不同的文本", "A-主文件")];
        let f = face("A-主文件", &blocks, &marks, &[]);
        let (out, stats) = merge_extraction(&[f], reqs.clone());
        assert_eq!(out, reqs);
        assert_eq!(stats.regex_dropped_covered, 0);
    }

    #[test]
    fn multi_face_seq_and_mandatory_stat() {
        // 多文件：编号跨 face 全局递增；mandatory 非 blocking 计数
        let blocks_a = vec![block(0, "p1", "块A")];
        let blocks_c = vec![block(0, "p1", "块C")];
        let marks_a = vec![BlockMark {
            from: 0,
            to: 0,
            verdict: bid_domain::coverage::BlockVerdict::Requirement,
            reason: String::new(),
        }];
        let marks_c = vec![BlockMark {
            from: 0,
            to: 0,
            verdict: bid_domain::coverage::BlockVerdict::Requirement,
            reason: String::new(),
        }];
        let subs_a = vec![SubmittedRequirement {
            block_idx: 0,
            statement: "A件要求".into(),
            category: Some("资格条件".into()),
            mandatory: true,
            substantive: true,
        }];
        let subs_c = vec![
            SubmittedRequirement {
                block_idx: 0,
                statement: "C件必选".into(),
                category: Some("商务要求".into()),
                mandatory: true,
                substantive: true,
            },
            SubmittedRequirement {
                block_idx: 0,
                statement: "C件普通".into(),
                category: Some("其他".into()),
                mandatory: false,
                substantive: false,
            },
        ];
        let fa = face("A-主文件", &blocks_a, &marks_a, &subs_a);
        let fc = face("C-补遗", &blocks_c, &marks_c, &subs_c);
        let (out, stats) = merge_extraction(&[fa, fc], vec![]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, "REQ-A0001");
        assert_eq!(out[1].id, "REQ-A0002");
        assert_eq!(out[2].id, "REQ-A0003");
        assert_eq!(out[0].category, "资格条件");
        assert_eq!(out[1].category, "商务要求");
        assert_eq!(
            stats.agent_mandatory_offcategory, 1,
            "C件必选（商务要求）计入"
        );
        assert!(stats.coverage_complete);
    }

    #[test]
    fn same_block_two_statements_both_kept() {
        // 同块两条 statement → 两条 REQ（replace 语义不去重——见域层）
        let blocks = vec![block(0, "p1", "复合要求块")];
        let marks = vec![BlockMark {
            from: 0,
            to: 0,
            verdict: bid_domain::coverage::BlockVerdict::Requirement,
            reason: String::new(),
        }];
        let subs = vec![submitted(0, "要点一"), submitted(0, "要点二")];
        let f = face("A-主文件", &blocks, &marks, &subs);
        let (out, stats) = merge_extraction(&[f], vec![]);
        assert_eq!(out.len(), 2, "同块两条都保留");
        assert_eq!(stats.agent_requirements, 2);
    }
}
