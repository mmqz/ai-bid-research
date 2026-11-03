//! #218 任务 3：中文顿号编号条款（`1、提供上行带宽30M/s光纤链路295条。`式）
//! 集成测试。
//!
//! 改前面（实机测试漏抽）：`re_clause = ^(\d+(?:\.\d+)+)` 只认多级点分
//! （1.1/2.3.4），顿号 level-1 列举编号（numbering.rs ArabicDot——`1、`）
//! 不进编号分支，语境未开时整条漏抽。本文件锁定：
//! - 顿号条款进编号分支：clause_no 取纯数字（「1、」→「1」）
//! - **双守卫**（顿号编号化的两条误伤面）：
//!   ① 开放传播语境（veto_ctx/review_ctx）内的顿号列举走既有传播链
//!      （否决语境继承不丢——P 红线面；条目无条款号）
//!   ② 格式章模板（投标函「1、我方承诺…」投标人侧范本）不抽——
//!      与传播分支的格式章节守卫同口径
//! - 点分条款号（4.2）不受守卫影响：语境终结语义（#68 J）不变
//! - 1-3 位数字界（`2026、` 年份形不是条款号）
//! - PDF / DOCX 两条主路同口径

use bid_parse::extract::extract_requirements;
use bid_parse::{Block, BlockKind, ParsedFile};

fn pdf(blocks: Vec<Block>) -> ParsedFile {
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

fn docx(blocks: Vec<Block>) -> ParsedFile {
    ParsedFile {
        name: "A-test.docx".to_string(),
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

fn para(idx: usize, locator: &str, text: &str) -> Block {
    Block { idx, kind: BlockKind::Para, locator: locator.into(), text: text.into(), cells: vec![] }
}

fn excerpts(reqs: &[bid_domain::Requirement]) -> Vec<String> {
    reqs.iter().map(|r| r.anchors.iter().map(|a| a.excerpt.clone()).collect::<String>()).collect()
}

/// 任务书原型：技术需求章的顿号列举条款——编号分支捕获，clause_no 纯数字。
#[test]
fn pdf_dunhao_clause_extracted_with_bare_number() {
    let f = pdf(vec![
        line(0, "第98页 行1", "第5章 技术需求"),
        line(1, "第98页 行2", "1、提供上行带宽30M/s光纤链路295条。"),
        line(2, "第98页 行3", "2、提供下行带宽30M/s光纤链路295条。"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert_eq!(reqs.len(), 2, "两条顿号条款应各成一条 REQ: {reqs:?}");
    assert_eq!(reqs[0].clause_no.as_deref(), Some("1"), "「1、」→ 条款号「1」: {reqs:?}");
    assert_eq!(reqs[0].category, "技术参数", "技术需求章上下文归类");
    assert_eq!(reqs[1].clause_no.as_deref(), Some("2"));
    assert!(
        reqs.iter().all(|r| r.anchors.iter().any(|a| a.excerpt.contains("光纤链路"))),
        "anchor 保真（原文含顿号形态）: {reqs:?}"
    );
}

/// 守卫 ①：否决总括句后的顿号列举走既有传播链——条目继承否决面且
/// **不带条款号**（顿号编号化会丢语境继承——P 红线面）。水利真件形态：
/// 「视为串通投标：」+「1、不同投标人的投标文件由同一单位或者个人编制；」
#[test]
fn pdf_dunhao_in_veto_context_keeps_propagation() {
    let f = pdf(vec![
        line(0, "第52页 行1", "9.2.2 下列行为，视为串通投标："),
        line(1, "第52页 行2", "1、不同投标人的投标文件由同一单位或者个人编制；"),
        line(2, "第52页 行3", "2、不同投标人的投标文件异常一致或者投标报价呈规律性差异；"),
    ]);
    let reqs = extract_requirements(&[f]);
    let vetos: Vec<&bid_domain::Requirement> =
        reqs.iter().filter(|r| r.category == "否决项").collect();
    assert_eq!(vetos.len(), 3, "总括句 + 2 传播条目: {reqs:?}");
    // 顿号条目（anchor 以「N、」开头）走传播分支 → 无条款号；
    // 伞句（点分 9.2.2）照常带条款号
    let dunhao_items: Vec<&bid_domain::Requirement> = vetos
        .iter()
        .copied()
        .filter(|r| {
            r.anchors
                .first()
                .map(|a| {
                    let t = a.excerpt.trim_start();
                    let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
                    n > 0 && n <= 3 && t.chars().nth(n) == Some('、')
                })
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(dunhao_items.len(), 2, "两条顿号条目应被判为列举形: {reqs:?}");
    assert!(
        dunhao_items.iter().all(|r| r.clause_no.is_none()),
        "语境内顿号条目走传播分支（无条款号）——编号化丢语境继承是 P 红线回归: {reqs:?}"
    );
    assert!(
        excerpts(&reqs).iter().any(|e| e.contains("同一单位或者个人编制")),
        "条目 anchor 在场: {reqs:?}"
    );
}

/// 守卫 ②：格式章（投标文件格式）的顿号模板列举不抽——伊犁真件形态
/// （第五章 > 一、投标函 > 「1、我方承诺在投标有效期…」投标人侧范本）。
/// 与传播分支的格式章节守卫同口径。
#[test]
fn pdf_dunhao_in_format_chapter_not_captured() {
    let f = pdf(vec![
        line(0, "第120页 行1", "第五章 投标文件格式"),
        line(1, "第120页 行2", "一、投标函及投标函附录"),
        line(2, "第120页 行3", "1、我方承诺在投标有效期日历天内不修改、撤销投标文件。"),
        line(3, "第120页 行4", "2、随同本投标函提交投标保证金一份，金额为人民币（大写）元。"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert!(
        reqs.is_empty(),
        "格式章模板的顿号列举不抽（范本结构非要求面）: {reqs:?}"
    );
}

/// 守卫不外溢：点分条款号（4.2 形）在否决语境内仍走编号分支并终结语境
/// （#68 J 语义——「1.5 …详见附表」类提及不得长开语境）。
#[test]
fn pdf_dotted_clause_still_terminates_veto_context() {
    let f = pdf(vec![
        line(0, "第3页 行1", "9.2.2 下列行为，视为串通投标："),
        line(1, "第3页 行2", "1.5 评标办法详见附表。"),
        line(2, "第3页 行3", "⑴投标人之间相互约定抬高或压低投标报价；"),
    ]);
    let reqs = extract_requirements(&[f]);
    let veto_n = reqs.iter().filter(|r| r.category == "否决项").count();
    assert_eq!(veto_n, 1, "总括句自身否决；点分条款与语境终结后的圈号不传播: {reqs:?}");
    assert!(
        reqs.iter().any(|r| r.clause_no.as_deref() == Some("1.5")),
        "点分条款号照常进编号分支: {reqs:?}"
    );
    assert!(
        !excerpts(&reqs).iter().any(|e| e.contains("抬高或压低")),
        "语境被 1.5 终结——其后的圈号条目不得继承否决: {reqs:?}"
    );
}

/// 数字界：`2026、`（4 位年份形）不是条款号——1-3 位对齐 numbering 面。
#[test]
fn pdf_year_like_dunhao_not_clause_number() {
    let f = pdf(vec![line(0, "第1页 行1", "2026、年度框架计划安排")]);
    let reqs = extract_requirements(&[f]);
    assert!(
        !reqs.iter().any(|r| r.clause_no.as_deref() == Some("2026")),
        "4 位顿号头不是条款号: {reqs:?}"
    );
}

/// DOCX 同口径：顿号条款进编号分支，clause_no 纯数字。
#[test]
fn docx_dunhao_clause_extracted_with_bare_number() {
    let f = docx(vec![
        para(0, "第5章 技术需求 > 5.1", "1、提供上行带宽30M/s光纤链路295条。"),
        para(1, "第5章 技术需求 > 5.1", "2、提供下行带宽30M/s光纤链路295条。"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert_eq!(reqs.len(), 2, "DOCX 顿号条款各成一条: {reqs:?}");
    assert_eq!(reqs[0].clause_no.as_deref(), Some("1"));
    assert_eq!(reqs[0].category, "技术参数");
    assert_eq!(reqs[1].clause_no.as_deref(), Some("2"));
}

/// DOCX 守卫 ①：否决总括句后的顿号列举走传播分支（无条款号）。
#[test]
fn docx_dunhao_in_veto_context_keeps_propagation() {
    let f = docx(vec![
        para(0, "第二章 投标人须知", "有下列情形之一的，视为串通投标："),
        para(1, "第二章 投标人须知", "1、不同投标人的投标文件由同一单位或者个人编制；"),
        para(2, "第二章 投标人须知", "2、不同投标人的投标文件相互混装；"),
    ]);
    let reqs = extract_requirements(&[f]);
    let vetos: Vec<&bid_domain::Requirement> =
        reqs.iter().filter(|r| r.category == "否决项").collect();
    assert_eq!(vetos.len(), 3, "总括句 + 2 传播条目: {reqs:?}");
    assert!(
        vetos.iter().all(|r| r.clause_no.is_none()),
        "DOCX 侧同口径——语境内顿号条目不带条款号: {reqs:?}"
    );
}

/// DOCX 守卫 ②：格式章（locator 派生章名）的顿号模板不抽。
#[test]
fn docx_dunhao_in_format_chapter_not_captured() {
    let f = docx(vec![para(
        0,
        "第五章 投标文件格式 > 一、投标函",
        "1、我方承诺在投标有效期日历天内不修改、撤销投标文件。",
    )]);
    let reqs = extract_requirements(&[f]);
    assert!(reqs.is_empty(), "格式章模板顿号列举不抽: {reqs:?}");
}
