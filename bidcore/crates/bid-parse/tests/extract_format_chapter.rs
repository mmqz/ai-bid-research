//! extract_format_chapter.rs —— #234：格式章表格头/碎片抽取噪声的回归锁定。
//!
//! 基线（A-某水利施工标段，main 实测）：951 REQ / matrix_blocking 131 中，
//! 格式章「第8章 投标文件格式」整页表格行（无句末标点/无条款号）被段落
//! 重组拼成大段，<6 字成员行（「投标人名称」「联系人 电话」「营业执照」）
//! 独立触发 REQ 化，段落中后部的资格词面（「营业执照号/资质证书」）命中
//! re_qual 误判资格条件 → 9 条永久 blocking 死锁（REQ-0943~0951）。
//! 修复后 942 / 122，其余 942 条 dump 逐条不变（GT L1/P 红线不降）。
//!
//! 本文件锁定：
//! - 修法 1：格式章（两行章形态「第8章」/「投标文件格式」+ 同行形态）内
//!   纯名词短语行不抽 REQ；带要求动词/超长度的真条款行不误杀；
//!   强审查表头旁路（review_strong——附表三评标表物理住在格式章）；
//!   非格式章的同类表格头行为不变（章节感知面）。
//! - 修法 2：剥条款号后 <2 字的单行段落不抽 REQ（编号碎片「9、」）；
//!   多行段落的首行碎片（「9」+ 续行——段落本体是真条款）不在此面。
//!
//! 构造纪律：pdftotext 真实形态——章号行/章标题行/正文之间有空行断段
//!（空行块 is_skip_line → 段落重组断开），表格行整页无标点连续拼段。

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

fn line(idx: usize, text: &str) -> Block {
    Block { idx, kind: BlockKind::PdfLine, locator: format!("l{idx}"), text: text.into(), cells: vec![] }
}

/// 空行块（pdftotext 行间空行——段落重组的断段信号）。
fn blank(idx: usize) -> Block {
    line(idx, "")
}

fn all_texts(reqs: &[bid_domain::Requirement]) -> Vec<String> {
    reqs.iter()
        .map(|r| r.anchors.iter().map(|a| a.excerpt.clone()).collect::<String>())
        .collect()
}

fn qual_texts(reqs: &[bid_domain::Requirement]) -> Vec<String> {
    reqs.iter()
        .filter(|r| r.category == "资格条件")
        .map(|r| r.anchors.iter().map(|a| a.excerpt.clone()).collect::<String>())
        .collect()
}

/// A-04 第136页实拍形态（pdftotext 线性化）：章号行与章标题**分立两行**，
/// 表格行全部无句末标点 → 段落重组整页拼段；资格词（企业资质等级/
/// 营业执照号）在段落中后部 → 首行 noun 行误判资格条件。
/// 修复面：这些行不抽（9 条 REQ-0943~0951 形态全灭）。
#[test]
fn two_line_chapter_format_table_headers_not_extracted() {
    let f = pf(vec![
        line(0, "第8章"),
        line(1, "投标文件格式"),
        blank(2),
        line(3, "(一)投标人基本情况表"),
        line(4, "投标人名称"),
        line(5, "联系人 电话"),
        line(6, "联系方式"),
        line(7, "传真 网址"),
        line(8, "组织结构"),
        line(9, "企业资质等级 项目经理"),
        line(10, "营业执照号 高级职称人员"),
        line(11, "备注"),
        line(12, "注 相关材料复印件在“十、原件的复印件”中提供。"),
    ]);
    let reqs = extract_requirements(&[f]);
    let quals = qual_texts(&reqs);
    assert!(
        quals.is_empty(),
        "格式章表格头不得抽为资格条件 REQ（#234 修复面）: {quals:?}"
    );
    let texts = all_texts(&reqs);
    assert!(
        !texts.iter().any(|t| t.contains("投标人名称") || t.contains("联系人 电话")),
        "模板占位行不得出现在任何 REQ 锚面: {texts:?}"
    );
}

/// 同行章形态（「第8章 投标文件格式」一行）——chapter 自含「格式」，
/// 既有 chapter.contains 判定覆盖；前瞻不误取正文行当章标题
///（内联形态下「信用等级」曾被误当标题豁守护卫——回归锁定）。
#[test]
fn same_line_chapter_format_guard_still_works() {
    let f = pf(vec![
        line(0, "第8章 投标文件格式"),
        blank(1),
        line(2, "信用等级"),
        line(3, "营业执照"),
        line(4, "资质证书"),
        line(5, "近 3 年经审计的财务会计报表"),
    ]);
    let reqs = extract_requirements(&[f]);
    let quals = qual_texts(&reqs);
    assert!(quals.is_empty(), "同行章形态守卫同口径: {quals:?}");
}

/// 防误杀面：格式章内带要求动词的真条款行仍抽为资格条件 REQ。
/// 构造：无句末标点的短动词行（「投标人须提供资质证书」——10 字，
/// 过长度门后靠动词面放行）——有效测到动词维（长行/带标点行不经过 (c)）。
#[test]
fn format_chapter_real_clause_with_verb_still_extracted() {
    let f = pf(vec![
        line(0, "第8章"),
        line(1, "投标文件格式"),
        blank(2),
        line(3, "投标人须提供资质证书"),
        line(4, "复印件并加盖单位公章。"),
    ]);
    let reqs = extract_requirements(&[f]);
    let quals = qual_texts(&reqs);
    assert_eq!(quals.len(), 1, "动词条款行不受守卫影响: {reqs:?}");
    assert!(quals[0].contains("须提供资质证书"));
}

/// 防误杀面：超过 noun 长度门（>12 字）的资格词行不在守卫面——
/// 表格内容行（非表头）照旧抽（A-04 第142页「企业资质等级：…」形态）。
#[test]
fn format_chapter_long_qual_row_not_guarded() {
    let f = pf(vec![
        line(0, "第8章"),
        line(1, "投标文件格式"),
        blank(2),
        line(3, "企业资质等级：水利水电工程施工总承包叁级及以上"),
    ]);
    let reqs = extract_requirements(&[f]);
    let quals = qual_texts(&reqs);
    assert_eq!(quals.len(), 1, "长行不在守卫面: {reqs:?}");
    assert!(quals[0].contains("企业资质等级"));
}

/// 章标题行拼段面：章标题（「投标文件格式」）是章节结构非段首 noun——
/// 章号行断段（后随空行）、标题行尾无标点拼进正文段时，不得作为段首
/// noun 判定拦段（标题行段的既有行为保持——锚面含标题行，与改前一致）。
#[test]
fn chapter_title_row_in_paragraph_not_guarded() {
    let f = pf(vec![
        line(0, "第8章"),
        blank(1),
        line(2, "投标文件格式"),
        line(3, "企业资质等级：水利水电工程施工总承包叁级"),
    ]);
    let reqs = extract_requirements(&[f]);
    let quals = qual_texts(&reqs);
    // 段首「投标文件格式」被章标题豁免 → 资格词段照旧抽（含标题行锚）
    assert_eq!(quals.len(), 1, "章标题行拼段不拦: {reqs:?}");
}

/// 强审查表头旁路（L3 口径，extract_pdf_l2gap 同形态）：附表评标表
/// 物理住在投标文件格式章区域，但它是审查表不是格式模板——强头开后
///（review_strong）格式章 noun 表头行照旧走资格面（防「评审表区域被
/// 格式守卫吞掉」的回归）。
#[test]
fn strong_review_header_bypasses_format_guard() {
    let f = pf(vec![
        line(0, "第8章"),
        line(1, "投标文件格式"),
        blank(2),
        line(3, "评标表-6"),
        line(4, "序号 评审项目"),
        line(5, "投标人 备注"),
        line(6, "A B C ……"),
        line(7, "营业执照"),
        line(8, "1 投标范围符合招标文件规定"),
    ]);
    let reqs = extract_requirements(&[f]);
    let quals = qual_texts(&reqs);
    assert!(!quals.is_empty(), "强审查头开启后格式章 noun 行不被守卫拦（L3 旁路面）: {reqs:?}");
}

/// 章节感知面：非格式章（投标人须知）的同类表格头行保持既有行为
///（照旧抽——只有格式章的模板占位才拦）。
#[test]
fn non_format_chapter_table_header_unchanged() {
    let f = pf(vec![
        line(0, "第2章 投标人须知"),
        line(1, "投标人名称"),
        line(2, "联系人 电话"),
        line(3, "企业资质等级 项目经理"),
        line(4, "营业执照号 高级职称人员"),
    ]);
    let reqs = extract_requirements(&[f]);
    let quals = qual_texts(&reqs);
    assert!(!quals.is_empty(), "非格式章行为不变（资格词面照抽）: {reqs:?}");
}

/// 修法 2：剥条款号后 <2 字的单行段落（编号碎片「9、」——顿号列举的
/// 断头，下一行新条款头断段）不抽 REQ。
#[test]
fn single_line_numeric_fragment_not_extracted() {
    let f = pf(vec![
        line(0, "第2章 投标人须知"),
        line(1, "9、"),
        line(2, "4.1 本次招标要求投标人须具备水利水电工程施工总承包叁级以上资质等级。"),
    ]);
    let reqs = extract_requirements(&[f]);
    let texts = all_texts(&reqs);
    assert!(
        !texts.iter().any(|t| t.trim() == "9、" || t.trim() == "9"),
        "单行编号碎片不得成 REQ: {texts:?}"
    );
}

/// 修法 2 防误杀：多行段落的首行碎片（「9」+ 跨页续行——A-04 REQ-0026
/// 形态：前附表行号断头 + 真条款正文续行拼段）**保留**——段落本体是
/// 真条款，拦了就丢条款。
#[test]
fn multi_line_fragment_head_kept() {
    let f = pf(vec![
        line(0, "第2章 投标人须知"),
        line(1, "9"),
        line(2, "业绩要求：投标人在近三年内至少承担过一项类似工程施工的业绩。"),
    ]);
    let reqs = extract_requirements(&[f]);
    let quals = qual_texts(&reqs);
    assert_eq!(quals.len(), 1, "多行段落首行碎片不拦（段落是真条款）: {reqs:?}");
    assert!(quals[0].contains("类似工程"));
}
