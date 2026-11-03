//! P0-A（issue #35）：PDF 行级碎片修复 集成测试。
//!
//! 基线实锤（veto-measure 口径）：PDF 组 L1 0.4680 / L2 0.1148——
//! pdftotext 按物理行切块，否决条款被切成两三行后词表跨行失配；
//! 无编号普通条款不抽；词表缺「拒收/不予受理」面。本文件锁定：
//! - 段落重组（行尾无标点续行 / 行尾截断 / 短行碎片）
//! - 断段（编号新条款/章节头前断段）
//! - 词表扩展（拒收/不予受理/否决条件…）
//! - 例外从句排除（除…外）
//! - 语境传播（总括句 → 圈号列举继承；流程步骤排除）
//! - 评分结构行排除（分值聚集非否决）
//! - 幂等

use bid_parse::extract::extract_requirements;
use bid_parse::{Block, BlockKind, ParsedFile};

fn pf(blocks: Vec<Block>) -> ParsedFile {
    ParsedFile {
        name: "A-test.pdf".to_string(),
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

fn line(idx: usize, locator: &str, text: &str) -> Block {
    Block { idx, kind: BlockKind::PdfLine, locator: locator.into(), text: text.into(), cells: vec![] }
}

fn cats(reqs: &[bid_domain::Requirement]) -> Vec<String> {
    reqs.iter().map(|r| r.category.clone()).collect()
}

fn veto_excerpts(reqs: &[bid_domain::Requirement]) -> Vec<String> {
    reqs.iter()
        .filter(|r| r.category == "否决项")
        .map(|r| r.anchors.iter().map(|a| a.excerpt.clone()).collect::<String>())
        .collect()
}

/// 段落重组：否决词被行尾切断（「…招标人」+「拒收投标文件。」）→ 整段判定命中。
#[test]
fn para_reassembly_cross_line_veto() {
    let f = pf(vec![
        line(0, "第6页 行1", "1.4.3 未提交有效凭证的，招标人"),
        line(1, "第6页 行2", "拒收投标文件。"),
    ]);
    let reqs = extract_requirements(&[f]);
    let vetos = veto_excerpts(&reqs);
    assert_eq!(vetos.len(), 1, "跨行否决条款应整段命中: {reqs:?}");
    // anchors 覆盖两行（excerpt 保真）
    assert!(vetos[0].contains("招标人"));
    assert!(vetos[0].contains("拒收投标文件"));
}

/// 段落重组：短行碎片（「拒收。」独立成行）——段落 REQ 覆盖全行
/// anchors；超短尾行（<6 字）保留独立碎片 REQ（GT 单行条款锚定面）。
///
/// #226 层 2 后文本改版：原文本「逾期送达…招标人将拒收」是规则说明形
/// （issue REQ-0007 同款），段落 REQ 不再落否决项——重组行为面改用非
/// 规则语境的否决句验证（意图不变：跨行否决词还原为整段判定）。
#[test]
fn para_reassembly_short_fragment() {
    let f = pf(vec![
        line(0, "第10页 行9", "未按要求密封的投标文件，评标委员会将"),
        line(1, "第10页 行10", "拒收。"),
    ]);
    let reqs = extract_requirements(&[f]);
    // 段落 REQ：两行 anchors，分类否决
    let para_req = reqs.iter().find(|r| r.anchors.len() == 2);
    assert!(para_req.is_some(), "段落 REQ 应覆盖两行: {reqs:?}");
    assert_eq!(para_req.unwrap().category, "否决项");
    // 超短尾行独立碎片 REQ（「拒收。」）
    assert!(reqs.iter().any(|r| r.anchors.len() == 1
        && r.anchors[0].excerpt == "拒收。"));

    // #226 集成面登记：规则说明形（逾期送达 + 招标人拒收）不再产否决 REQ
    let f2 = pf(vec![
        line(0, "第10页 行9", "逾期送达的投标文件的，招标人将"),
        line(1, "第10页 行10", "拒收。"),
    ]);
    let reqs2 = extract_requirements(&[f2]);
    assert!(
        reqs2.iter().all(|r| r.anchors.len() != 2 || r.category != "否决项"),
        "#226 规则说明形段落不落否决项: {reqs2:?}"
    );
}

/// 断段：行尾有终结标点 + 下一行是编号新条款 → 各自成段。
#[test]
fn para_break_on_new_clause() {
    let f = pf(vec![
        line(0, "第6页 行1", "3.4.2 投标人未提供有效营业执照的，其投标将被否决。"),
        line(1, "第6页 行2", "3.4.3 投标人须具备水利水电工程施工总承包贰级资质。"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert_eq!(cats(&reqs), vec!["否决项".to_string(), "资格条件".to_string()]);
}

/// 词表扩展：拒收/否决条件面（#226 层 2 后，「招标人不予受理」形程序
/// 后果条款不再落否决项——见 veto_rule.rs；这里验证词表在非规则语境下
/// 仍命中，并集成面登记规则说明形的降级）。
#[test]
fn expanded_veto_lexicon() {
    // 评委处置形（非「招标人/逾期」规则语境）——词面命中且保留否决
    let f = pf(vec![
        line(0, "第14页 行37", "未按本章第 4.1.1 项要求密封的，评标委员会将予以拒收。"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert_eq!(veto_excerpts(&reqs).len(), 1, "评委拒收形仍否决: {reqs:?}");

    // #226 集成面登记：程序后果条款（招标人不予受理）不产否决 REQ
    let f2 = pf(vec![
        line(0, "第14页 行38", "未按本章第 4.1.1 项要求密封的投标文件，招标人不予受理。"),
    ]);
    let reqs2 = extract_requirements(&[f2]);
    assert!(veto_excerpts(&reqs2).is_empty(), "#226 规则说明形不落否决: {reqs2:?}");
}

/// 例外从句排除：除…外 不算否决。
#[test]
fn exception_clause_excluded() {
    let f = pf(vec![
        line(0, "第8章 行5", "除不可抗力外，投标人应按期完成全部工程内容，"),
        line(1, "第8章 行6", "并保证工程质量达到合同约定标准。"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert!(veto_excerpts(&reqs).is_empty(), "例外从句不应判否决: {reqs:?}");
}

/// 语境传播：总括句 → 圈号列举继承；流程步骤排除。
#[test]
fn context_propagation_and_procedure_guard() {
    let f = pf(vec![
        line(0, "第3页 行1", "9.2.2 下列行为，视为串通投标："),
        line(1, "第3页 行2", "⑴投标人之间相互约定抬高或压低投标报价；"),
        line(2, "第3页 行3", "⑵投标人之间约定，在招标项目中分别以高、中、低价位报价；"),
        // 流程步骤（评标职责——程序词排除）
        line(3, "第3页 行4", "⑴初步评审阶段负责资格评审和响应性评审；"),
    ]);
    let reqs = extract_requirements(&[f]);
    let vetos = veto_excerpts(&reqs);
    assert_eq!(vetos.len(), 3, "总括句 + 2 串通列举（流程职责行排除）: {reqs:?}");
    assert!(vetos.iter().all(|v| !v.contains("负责")));
}

/// 评分结构行排除：分值聚集（≥2 处「N分」）不判否决。
#[test]
fn score_shape_excluded() {
    let f = pf(vec![
        line(0, "第9页 行1", "评审标准："),
        line(1, "第9页 行2", "⑴工程特点及施工重点和难点分析准确、全面的，得1分，否则得0～1分；"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert!(veto_excerpts(&reqs).is_empty(), "分值结构行不应判否决: {reqs:?}");
}

/// 目录行防御：带引导点的目录条目不抽取不建上下文。
#[test]
fn toc_line_defense() {
    let f = pf(vec![
        line(0, "第2页 行1", "第一章 投标公告 ............................... 5"),
        line(1, "第2页 行2", "第二章 投标人须知 ........................... 12"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert!(reqs.is_empty());
}

/// 幂等：同输入两遍全等。
#[test]
fn idempotent_extraction() {
    let blocks = vec![
        line(0, "第6页 行1", "1.4.3 未提交有效凭证的，招标人"),
        line(1, "第6页 行2", "拒收投标文件。"),
        line(2, "第6页 行3", "9.2.2 下列行为，视为串通投标："),
        line(3, "第6页 行4", "⑴投标人之间相互约定抬高或压低投标报价；"),
    ];
    let r1 = extract_requirements(&[pf(blocks.clone())]);
    let r2 = extract_requirements(&[pf(blocks)]);
    assert_eq!(r1, r2);
}
