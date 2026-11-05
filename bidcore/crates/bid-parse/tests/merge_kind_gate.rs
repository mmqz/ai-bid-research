//! #250 回归：合并面门禁按业务 kind + 来源字段完整化 + 表格抽取专路。
//!
//! 背景（issue #250 实测）：`has_extraction_face` 按文件名前缀判定，真实
//! 用户文件（「光纤租赁费招标文件.docx」）无 A- 前缀 → 被 filter 掉 →
//! `merge_extraction` faces 为空 → 67 条 Agent 提交与 76 个区间标记永不
//! 消费。与 #196（extract.rs 按 kind 分发）同根因的第二次出现。
//!
//! 修复面断言：
//! - 门禁值域矩阵（kind 优先 / 前缀仅夹具兜底）；
//! - `marked_by` 来源字段完整化（正则产物全量 regex——与 Agent 项 agent
//!   对称，merge.rs 既有测试覆盖）；
//! - 表格专路（table_semantics.rs）经 extract 主链的端到端形态：技术
//!   参数表/商务要求表语义装配、前附表/审查表/格式表不接管（防双抽）。

use bid_parse::extract::{extract_requirements, extract_requirements_with_kind};
use bid_parse::merge::has_extraction_face;
use bid_parse::{Block, BlockKind, ParsedFile};

fn pf(name: &str, blocks: Vec<Block>) -> ParsedFile {
    ParsedFile {
        name: name.to_string(),
        sha256: "test".into(),
        pages: 1,
        tables: 0,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary: "test".into(),
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

fn tr(idx: usize, locator: &str, cells: &[&str]) -> Block {
    Block {
        idx,
        kind: BlockKind::TableRow,
        locator: locator.to_string(),
        text: cells
            .iter()
            .filter(|c| !c.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
        cells: cells.iter().map(|s| s.to_string()).collect(),
    }
}

// ================================================================ 门禁值域矩阵

/// #250 核心：kind 优先——登记 kind 的文件无论文件名带不带前缀，
/// 门禁判定完全相同（真实文件名与夹具文件名走同一合并路径）。
/// #271：「格式模板」移入无面组（官方填空模板非条款源）。
#[test]
fn kind_gate_takes_precedence_over_prefix() {
    let main_kinds = ["招标文件", "补遗", "答疑", "评分办法"];
    for kind in main_kinds {
        for name in [
            "光纤租赁费招标文件-文本修复版v5w.docx", // 真实用户文件名（无前缀）
            "A-招标文件.docx",                       // 夹具名（有前缀）
        ] {
            assert!(
                has_extraction_face(Some(kind), name),
                "kind={kind} name={name} 应有合并面"
            );
        }
    }
}

/// 附件/图纸/证据/格式模板无条款面：即使文件名带主文件前缀也不进合并统计
/// （kind 是登记的业务事实，优先于夹具前缀——#196 同口径；#271 格式模板
/// 同入——官方填空模板非条款源）。
#[test]
fn attachment_kinds_have_no_face_even_with_main_prefix() {
    for kind in ["附件", "图纸", "证据", "格式模板"] {
        assert!(!has_extraction_face(Some(kind), "A-招标文件.docx"), "kind={kind}");
        assert!(!has_extraction_face(Some(kind), "图纸目录.pdf"));
    }
}

/// kind 缺省（夹具直调/未登记）：前缀兜底保持既有夹具行为——
/// A-/C-/B1/B2 进面，B3（图纸目录/证据件）不进。
#[test]
fn none_kind_falls_back_to_fixture_prefix() {
    for name in ["A-主文件.docx", "C-补遗.pdf", "B1-报价文件.xlsx", "B2-扫描件.pdf"] {
        assert!(has_extraction_face(None, name), "{name} 夹具兜底进面");
    }
    assert!(!has_extraction_face(None, "B3-图纸目录.pdf"), "B3 无面");
    assert!(!has_extraction_face(None, "任意未登记名.docx"), "None 无前缀保守无面");
}

// ================================================================ 来源字段完整化

/// marked_by 完整化：正则产物全量带 `marked_by: "regex"`（单一插入点
/// push_req——段落/表格/前附表全路径），与 Agent 项（merge.rs
/// `marked_by: "agent"`）来源面对称。
#[test]
fn regex_requirements_all_marked_by_regex() {
    let blocks = vec![
        para(0, "第二章 投标人须知 > 3.资格",
             "3.4.2 投标人未提供有效营业执照的，其投标将被否决。"),
        tr(1, "第三章 采购需求 > 二、技术要求 表1 行1",
           &["序号", "参数名称", "参数要求"]),
        tr(2, "第三章 采购需求 > 二、技术要求 表1 行2",
           &["1", "光缆链路完好率", "≥95%/月"]),
    ];
    let reqs = extract_requirements(&[pf("A-招标文件.docx", blocks)]);
    assert!(!reqs.is_empty());
    for r in &reqs {
        assert_eq!(
            r.params.get("marked_by").and_then(|v| v.as_str()),
            Some("regex"),
            "REQ {} 缺 marked_by=regex", r.id
        );
    }
}

// ================================================================ 表格专路（extract 主链端到端）

/// #250 验收核心形态：参数表（数量+单位形态——re_model 型号守卫抽不到）
/// 经主链抽为「技术参数」，语义组装完整（数值单位不丢）、可检索。
#[test]
fn param_table_extracted_with_full_semantics() {
    let blocks = vec![
        tr(0, "第三章 采购需求 > 二、技术要求 表1 行1",
           &["序号", "参数名称", "参数要求"]),
        tr(1, "第三章 采购需求 > 二、技术要求 表1 行2",
           &["1", "光纤链路数量", "295 条 30M/s"]),
        tr(2, "第三章 采购需求 > 二、技术要求 表1 行3",
           &["2", "光缆链路完好率", "≥95%/月"]),
        tr(3, "第三章 采购需求 > 二、技术要求 表1 行4",
           &["3", "单条链路平均历时", "≤360 分钟"]),
        tr(4, "第三章 采购需求 > 二、技术要求 表1 行5",
           &["4", "调测时限", "签订合同之日起 15 日历天内调测完毕"]),
        tr(5, "第三章 采购需求 > 二、技术要求 表1 行6",
           &["★", "服务地点", "黑龙江省大兴安岭地区松岭区"]),
    ];
    let reqs = extract_requirements(&[pf("光纤租赁费招标文件-文本修复版v5w.docx", blocks)]);
    let tech: Vec<&bid_domain::Requirement> =
        reqs.iter().filter(|r| r.category == "技术参数").collect();
    assert_eq!(tech.len(), 5, "5 条参数行全抽（表头不产）: {:?}",
        reqs.iter().map(|r| (&r.category, &r.id)).collect::<Vec<_>>());
    // 可检索面：statement（语义组装）与 excerpt（块原文）双含关键词
    let stmts: Vec<&str> = tech.iter()
        .filter_map(|r| r.params.get("statement").and_then(|v| v.as_str()))
        .collect();
    for kw in ["295 条 30M/s", "≥95%/月", "≤360 分钟", "15 日历天", "松岭区"] {
        assert!(stmts.iter().any(|s| s.contains(kw)),
            "关键词「{kw}」在 statement 面不可检索: {stmts:?}");
    }
    for r in &tech {
        // 溯源面：excerpt 锚保真（块原文——cells 空格连接）+ locator 带
        // 表号行号（DOCX 表格位置——issue #250 验收 4 的定位面）
        assert!(r.anchors[0].excerpt.contains("295") || r.anchors[0].excerpt.contains("完好率")
            || r.anchors[0].excerpt.contains("360") || r.anchors[0].excerpt.contains("15 日历天")
            || r.anchors[0].excerpt.contains("松岭区"),
            "excerpt 锚保真（块原文）: {}", r.anchors[0].excerpt);
        assert!(r.anchors[0].locator.contains("技术要求") && r.anchors[0].locator.contains("表1"),
            "locator 带章节与表位: {}", r.anchors[0].locator);
        assert!(r.params.get("marked_by").is_some(), "来源字段在位");
        assert!(r.clause_no.as_deref().unwrap_or("").starts_with("表1-"),
            "条款号表位形态: {:?}", r.clause_no);
    }
    // ★实质性条款入 params.star（服务地点行）
    let star = tech.iter().find(|r| r.params.get("statement")
        .and_then(|v| v.as_str()).unwrap_or("").contains("服务地点"));
    assert_eq!(star.and_then(|r| r.params.get("star")).and_then(|v| v.as_bool()),
        Some(true), "★实质性条款入 params.star");
}

/// 商务要求表：此前无任何商务抽取分支（实测 0 条——保证金/有效期/履约
/// 全丢），专路补齐。
#[test]
fn commercial_table_extracted_as_commercial_category() {
    let blocks = vec![
        tr(0, "第三章 采购需求 > 三、商务要求 表2 行1",
           &["序号", "条款名称", "内容"]),
        tr(1, "第三章 采购需求 > 三、商务要求 表2 行2",
           &["1", "投标保证金", "17,000.00 元（A级 8,500）"]),
        tr(2, "第三章 采购需求 > 三、商务要求 表2 行3",
           &["2", "采购代理服务费", "20,000 元"]),
        tr(3, "第三章 采购需求 > 三、商务要求 表2 行4",
           &["3", "预算/最高限价", "1,195,800.00 元"]),
        tr(4, "第三章 采购需求 > 三、商务要求 表2 行5",
           &["4", "履约保证金", "不缴纳"]),
        tr(5, "第三章 采购需求 > 三、商务要求 表2 行6",
           &["5", "投标有效期", "90 日"]),
        tr(6, "第三章 采购需求 > 三、商务要求 表2 行7",
           &["6", "服务期限", "365 天"]),
    ];
    let reqs = extract_requirements(&[pf("光纤租赁费招标文件-文本修复版v5w.docx", blocks)]);
    let comm: Vec<&bid_domain::Requirement> =
        reqs.iter().filter(|r| r.category == "商务要求").collect();
    assert_eq!(comm.len(), 6, "6 条商务行全抽: {:?}",
        reqs.iter().map(|r| (&r.category, &r.id)).collect::<Vec<_>>());
    let joined: String = comm.iter()
        .filter_map(|r| r.params.get("statement").and_then(|v| v.as_str()))
        .collect::<Vec<_>>().join(" | ");
    for kw in ["17,000.00", "20,000", "1,195,800.00", "不缴纳", "90 日", "365 天"] {
        assert!(joined.contains(kw), "商务关键词「{kw}」不可检索: {joined}");
    }
}

/// 防双抽：被专路接管的参数表数据行不再走旧 re_model 碎片链——
/// 同一块只产一条 REQ（区域接管纪律）。
#[test]
fn semantic_table_region_takeover_no_double_extraction() {
    let blocks = vec![
        tr(0, "第五章 技术需求 表1 行1",
           &["序号", "参数名称", "参数要求"]),
        // 同一行同时含型号谱系（re_model 命中）与参数语义——只准出一条
        tr(1, "第五章 技术需求 表1 行2",
           &["1", "设备型号", "GW-8000-T4"]),
    ];
    let reqs = extract_requirements(&[pf("A-招标文件.docx", blocks)]);
    let hits: Vec<&bid_domain::Requirement> = reqs
        .iter()
        .filter(|r| r.anchors[0].excerpt.contains("GW-8000-T4"))
        .collect();
    assert_eq!(hits.len(), 1, "同一块只产一条 REQ（区域接管防双抽）: {}",
        hits.len());
    assert_eq!(hits[0].category, "技术参数");
    assert_eq!(
        hits[0].params.get("statement").and_then(|v| v.as_str()),
        Some("设备型号：GW-8000-T4"),
        "专路语句面优先（型号不再裸抽为碎片）"
    );
}

/// 前附表/审查表/格式表不接管：既有链语义保留（前附表否决行 / review_table
/// #144 / #234 格式章纪律）。
#[test]
fn legacy_tables_untouched_by_semantic_path() {
    let blocks = vec![
        // 前附表（条款号列）：否决行走既有链（前附表-N 条款号）
        tr(0, "第二章 投标人须知 前附表 表1 行1",
           &["条款号", "条款名称", "内容"]),
        tr(1, "第二章 投标人须知 前附表 表1 行2",
           &["1", "否决条款", "未按要求密封的投标文件将被否决"]),
        // 审查表（#144 review_table 形状）
        tr(2, "第六章 评审 表2 行1",
           &["序号", "评审内容", "合格条件"]),
        tr(3, "第六章 评审 表2 行2",
           &["1", "营业执照", "具备有效营业执照"]),
        // 格式章表格（#234 纪律：模板表不进抽取面）
        tr(4, "第六章 投标文件格式 表3 行1",
           &["序号", "参数名称", "参数要求"]),
        tr(5, "第六章 投标文件格式 表3 行2",
           &["1", "格式项", "按格式填写"]),
    ];
    let reqs = extract_requirements(&[pf("A-招标文件.docx", blocks.clone())]);
    // 前附表否决行：既有链抽取（否决项 + 前附表-1 条款号）
    let veto = reqs.iter().find(|r| r.category == "否决项");
    assert!(veto.is_some(), "前附表否决行仍走既有链: {:?}",
        reqs.iter().map(|r| &r.category).collect::<Vec<_>>());
    assert!(veto.unwrap().clause_no.as_deref().unwrap_or("").contains("前附表"));
    // 审查表：review_table 接管（资格条件）
    let qual = reqs.iter().find(|r| r.category == "资格条件");
    assert!(qual.is_some(), "审查表仍走 #144 review_table 接管");
    // 格式章表格：零抽取
    let fmt = reqs.iter().find(|r| r.anchors[0].excerpt.contains("按格式填写"));
    assert!(fmt.is_none(), "格式章表格不进抽取面（#234 纪律）");
    // 无技术参数/商务要求误产
    assert!(reqs.iter().all(|r| r.category != "技术参数" && r.category != "商务要求"),
        "三张非语义表不得误产技术参数/商务要求: {:?}",
        reqs.iter().map(|r| &r.category).collect::<Vec<_>>());
}

/// 真实文件名 + 无 kind（extract 兜底面）仍可抽取（#196 回归锁不回退），
/// 与门禁的 None 保守分支对照——抽取面宁多勿漏、门禁面保守。
#[test]
fn real_name_without_kind_still_extracts_via_fallback() {
    let blocks = vec![para(
        0,
        "第二章 投标人须知 > 3.资格",
        "3.4.2 投标人未提供有效营业执照的，其投标将被否决。",
    )];
    let reqs = extract_requirements_with_kind(
        &[pf("光纤租赁费招标文件.docx", blocks)],
        |_| None,
    );
    assert_eq!(reqs.len(), 1, "真实文件名（无 kind）不得静默零抽取（#196）");
}
