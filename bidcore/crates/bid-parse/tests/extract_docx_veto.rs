//! P0-B（issue #35）：DOCX 否决分类信号源 + 后置否定从句 + 表格行否决面 集成测试。
//!
//! 基线实锤（veto-measure 块对齐口径）：DOCX 组否决分类召回 0.0000——
//! `extract_main_docx` 原只走章节上下文映射，R3-S3-5 优先级链（强否决 >
//! 资格面 > 章节上下文）在 DOCX 侧整体缺失。本文件锁定修复行为：
//! - 编号段落优先级链（否决短语不得落「其他」）
//! - 后置否定从句（处置动词在句尾）
//! - 例外从句排除（除…外 不是否决——精确率守护）
//! - 无编号普通段抽取面（资格负面清单/审查表导语）
//! - 语境传播（总括句 → 列举条目继承；条款号「3.5.2」不误判为列举）
//! - TableRow 否决面扩展（不限前附表）+ 审查表语境
//! - 幂等（同输入两遍全等）

use bid_parse::extract::extract_requirements;
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
    Block { idx, kind: BlockKind::Para, locator: locator.into(), text: text.into(), cells: vec![] }
}

fn trow(idx: usize, locator: &str, cells: &[&str]) -> Block {
    Block {
        idx,
        kind: BlockKind::TableRow,
        locator: locator.into(),
        text: cells.join(" "),
        cells: cells.iter().map(|s| s.to_string()).collect(),
    }
}

fn cats(reqs: &[bid_domain::Requirement]) -> Vec<String> {
    reqs.iter().map(|r| r.category.clone()).collect()
}

/// 编号段落优先级链：强否决短语 > 资格面 > 章节上下文。
#[test]
fn numbered_clause_priority_chain() {
    let f = pf(
        "A-test.docx",
        vec![
            para(0, "第二章 投标人须知 > 3.资格", "3.4.2 投标人未提供有效营业执照的，其投标将被否决。"),
            para(1, "第二章 投标人须知 > 3.资格", "3.4.3 投标人须具备水利水电工程施工总承包贰级及以上资质。"),
        ],
    );
    let reqs = extract_requirements(&[f]);
    assert_eq!(cats(&reqs), vec!["否决项".to_string(), "资格条件".to_string()]);
    // 章节上下文兜底仍在（非否决/资格面条款走 category_for）
    let f2 = pf(
        "A-test.docx",
        vec![para(0, "第二章 投标人须知 > 3.资格审查", "3.1.1 投标人应按招标文件要求编制投标文件。")],
    );
    let reqs2 = extract_requirements(&[f2]);
    assert_eq!(cats(&reqs2), vec!["资格条件".to_string()]);
}

/// 后置否定从句：处置动词在句尾（「…，招标人有权拒收。」）。
#[test]
fn postpositive_negation_clause() {
    let f = pf(
        "A-test.docx",
        vec![
            para(0, "第二章 投标人须知", "未按要求密封的投标文件，招标人有权拒收。"),
            para(1, "第二章 投标人须知", "不包含运输费用的报价将被否决。"),
        ],
    );
    let reqs = extract_requirements(&[f]);
    assert_eq!(cats(&reqs).len(), 2);
    assert!(cats(&reqs).iter().all(|c| c == "否决项"));
}

/// 例外从句排除：除…外 不是否决（精确率守护）。
#[test]
fn exception_clause_excluded() {
    let f = pf(
        "A-test.docx",
        vec![para(0, "第四章 合同条款", "除不可抗力外，投标人应按期完成全部工程内容。")],
    );
    let reqs = extract_requirements(&[f]);
    // 不产生否决项 REQ（其他面也不抽——无编号非资格非否决）
    assert!(cats(&reqs).iter().all(|c| c != "否决项"));
}

/// 无编号普通段：资格负面清单（负面情形词面）抽为否决项。
#[test]
fn unnumbered_negative_list_para() {
    let f = pf(
        "A-test.docx",
        vec![
            para(0, "第二章 投标人须知", "（12）在最近三年内有骗取中标或严重违约或重大工程质量问题的。"),
            para(1, "第二章 投标人须知", "（4）曾因在招标、评标以及其他与招标投标有关活动中从事违法行为而受过行政处罚或刑事处罚的；"),
        ],
    );
    let reqs = extract_requirements(&[f]);
    assert_eq!(cats(&reqs).len(), 2);
    assert!(cats(&reqs).iter().all(|c| c == "否决项"));
}

/// 语境传播：总括句开启 → 列举条目继承；条款号「3.5.2」不误判为列举。
#[test]
fn context_propagation_and_clause_no_guard() {
    let f = pf(
        "A-test.docx",
        vec![
            para(0, "第二章 投标人须知", "9.2.2 下列行为，视为允许他人以本单位名义承揽工程："),
            para(1, "第二章 投标人须知", "（1）投标人的法定代表人的委托代理人不是投标人本单位人员；"),
            para(2, "第二章 投标人须知", "（2）投标人拟在施工现场所设项目管理机构的项目经理不是本单位人员。"),
            // 非列举普通段终止语境
            para(3, "第二章 投标人须知", "本章说明由招标人负责解释。"),
            // 条款号形态（3.5.2）不是列举条目——不得继承语境
            para(4, "第二章 投标人须知", "3.5.2 近年财务状况的年份要求详见前附表。"),
        ],
    );
    let reqs = extract_requirements(&[f]);
    let veto_excerpts: Vec<&str> =
        reqs.iter().filter(|r| r.category == "否决项").map(|r| r.anchors[0].excerpt.as_str()).collect();
    assert_eq!(veto_excerpts.len(), 3); // 总括句 + 2 列举条目
    assert!(veto_excerpts.iter().any(|e| e.contains("委托代理人")));
    assert!(veto_excerpts.iter().any(|e| e.contains("项目经理")));
    // 「3.5.2」编号条款正常抽取（章节映射兜底分类），但**不落否决项**
    // （语境已终止 + 条款号形态不误判为列举条目）
    let fin = reqs.iter().find(|r| r.anchors[0].excerpt.contains("财务状况"));
    assert!(fin.is_some(), "编号条款应被抽取");
    assert_ne!(fin.unwrap().category, "否决项");
}

/// TableRow 否决面扩展（不限前附表）：审查表行 + 评审标准表语境。
#[test]
fn table_row_veto_expansion() {
    let f = pf(
        "A-test.docx",
        vec![
            // 评审标准表：表头行开启审查语境 → 后续行继承（隐含否决面）
            trow(0, "第三章 评标办法 表3 行1", &["2.1.1", "形式评审标准", "评审标准"]),
            trow(1, "第三章 评标办法 表3 行2", &["投标人名称", "投标人名称应与营业执照、资质证书、安全生产许可证一致"]),
            // 普通表格行：强否决短语直接命中
            trow(2, "第二章 投标人须知 表1 行5", &["3.4", "未按要求密封的投标文件，招标人有权拒收"]),
            // 模板占位行（□）不抽
            trow(3, "第二章 投标人须知 表1 行6", &["1.10", "□不接受 □接受"]),
        ],
    );
    let reqs = extract_requirements(&[f]);
    let veto: Vec<&str> =
        reqs.iter().filter(|r| r.category == "否决项").map(|r| r.anchors[0].excerpt.as_str()).collect();
    assert!(veto.iter().any(|e| e.contains("投标人名称应与营业执照")));
    assert!(veto.iter().any(|e| e.contains("拒收")));
    assert!(veto.iter().all(|e| !e.contains("□"))); // 占位排除
}

/// 前附表否决行既有行为回归（前附表-{item} 条款号）。
#[test]
fn frontmatter_veto_row_regression() {
    let f = pf(
        "A-test.docx",
        vec![trow(0, "第二章 投标人须知 前附表 行3", &["10", "否决投标的情形：逾期送达的投标文件将被否决"])],
    );
    let reqs = extract_requirements(&[f]);
    assert_eq!(cats(&reqs), vec!["否决项".to_string()]);
    assert_eq!(reqs[0].clause_no.as_deref(), Some("前附表-10"));
}

/// 幂等：同输入两遍输出全等。
#[test]
fn idempotent_extraction() {
    let blocks = vec![
        para(0, "第二章 投标人须知", "3.4.2 投标人未提供有效营业执照的，其投标将被否决。"),
        para(1, "第二章 投标人须知", "下列行为，视为串通投标："),
        para(2, "第二章 投标人须知", "（1）相互串通投标报价的；"),
        trow(3, "第三章 评标办法 表3 行1", &["形式评审标准"]),
        trow(4, "第三章 评标办法 表3 行2", &["签字盖章", "投标文件签字盖章符合第二章规定"]),
    ];
    let f1 = pf("A-test.docx", blocks.clone());
    let f2 = pf("A-test.docx", blocks);
    let r1 = extract_requirements(&[f1]);
    let r2 = extract_requirements(&[f2]);
    assert_eq!(r1, r2);
}

/// P0-B 修订 2（分值形态锁定）：评标办法章内——
/// - 含「N分」形态的评分细则行 → 评分项（分值锚定优先，即便含资格词）
/// - 无分值形态的评审否决条款 → 否决项（修订 1 整章 blanket 误伤面的回收）
/// - 无形态无否决词 → 保持评分项（术语行最小扰动）
/// 实测依据：corpus-A 4 条评分项全部含 N分；BJ 37 条被误伤否决条款全部无形态。
#[test]
fn scoring_shape_lock_in_evaluation_chapter() {
    let f = pf(
        "A-test.docx",
        vec![
            // 含分值形态 + 资格词（安全生产许可证）——形态锁定评分项
            para(0, "第三章 评标办法 > 4.详细评审", "4.2 技术分（55 分）：售后服务方案 8 分；持有效安全生产许可证且有效期覆盖工期的得 2 分。"),
            // 评审否决条款（无分值形态）——修订 1 会误判评分项，必须判否决
            para(1, "第三章 评标办法 > 3.初步评审", "3.1.2 投标人有以下情形之一的，其投标文件将被否决："),
            // 低于成本价否决（无分值形态）
            para(2, "第三章 评标办法 > 3.初步评审", "3.2.4 投标人不能证明其报价合理性的，评标委员会应当否决其投标。"),
            // 无形态无否决词的术语行——保持章节上下文（评分项）
            para(3, "第三章 评标办法 > 1.总则", "3.1 评标委员会依法组建，负责评标活动。"),
        ],
    );
    let reqs = extract_requirements(&[f]);
    assert_eq!(
        cats(&reqs),
        vec![
            "评分项".to_string(),
            "否决项".to_string(),
            "否决项".to_string(),
            "评分项".to_string(),
        ],
        "分值形态锁定：N分行锁评分项；无形态否决句判否决；术语行保持上下文"
    );
}
