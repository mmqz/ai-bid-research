//! extract_pdf_l2gap.rs —— #68 阶段 4：PDF L2 残差修复的回归锁定。
//!
//! 基线实锤（veto-measure 口径，GT 补标 v2 后）：PDF L2 0.6612 / L1 0.9137；
//! 修复后 PDF L2 0.8067 / L1 0.9537（红线：L1 ≥ 0.93、P ≥ 0.87——全过）。
//! 本文件按修复族逐项锁定（missed 样例驱动的最小复现，全部真实形态）：
//! - F1+G+I：编号审查表头开语境 + 列举条目断段（防吞并）+ 资格词让位传播
//! - F2/M2：裸数字/裸闭括号条目（pdftotext 残影形态）
//! - J：审查语境被编号条款行界定（防百页须知章泛滥——r3 FP 618 教训）
//! - A：总括句处置语义分辨（重新招标/回避不开否决语境）
//! - L1/L3：拒收情形头强词 + 附录评标表穿透格式章守卫（强头旁路面）
//! - F3/F5：投标失效/被拒绝、联合体不接受句式
//! - F4：列举中段引导句不误杀语境
//! - K1/N1：查验证件清单排除；K4：履约主体守卫（利害关系放行面）
//! - D/M1：裸数字小节头识别（审查谓词排除——防条目被当头吞掉）

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

fn veto_texts(reqs: &[bid_domain::Requirement]) -> Vec<String> {
    reqs.iter()
        .filter(|r| r.category == "否决项")
        .map(|r| r.anchors.iter().map(|a| a.excerpt.clone()).collect::<String>())
        .collect()
}

/// F1+G+I：编号审查表头（尾缀 ）非终结标点）→ 开审查语境；⑴ 条目断段
/// 不被吞并；条目含「资质证书」资格词仍走传播（资格性否决在审查表里）。
#[test]
fn numbered_review_header_opens_context_and_items_propagate() {
    let f = pf(vec![
        line(0, "第5章 评标办法"),
        line(1, "初步评审是对投标文件是否符合招标投标有关规定和招标文件实质性要求的评审，从投标"),
        line(2, "文件的形式、资格、响应性和算术错误四个方面进行审查，确认投标文件有效性。"),
        line(3, "5.1.1 形式评审标准：（见附表三：评标表-4）"),
        line(4, "⑴投标人名称应与营业执照、资质证书、安全生产许可证一致；"),
        line(5, "⑵投标人施工项目经理未同时在两个及以上工程中担任项目经理；"),
    ]);
    let reqs = extract_requirements(&[f]);
    let vetos = veto_texts(&reqs);
    assert_eq!(vetos.len(), 2, "两个审查条目都传播否决语义: {reqs:?}");
    assert!(vetos.iter().any(|v| v.contains("投标人名称应与")));
    assert!(vetos.iter().any(|v| v.contains("未同时在两个及以上")));
}

/// F2/M2：裸数字（空格/直连/点号）与裸闭括号条目——pdftotext 残影形态。
#[test]
fn bare_digit_and_bare_paren_items_propagate() {
    let f = pf(vec![
        line(0, "第5章 评标办法"),
        line(1, "5.1.3 响应性评审标准（见附件三：评标表-6）"),
        line(2, "6 只能有一个报价（含修正报价）；"),
        line(3, "7 投标文件的正本、副本数量符合第 2 章 投标人须知第 3.7.4 款规定；"),
        line(4, "8投标文件的印刷与装订符合第2章投标人须知第3.7.5款规定。"),
    ]);
    let reqs = extract_requirements(&[f]);
    let vetos = veto_texts(&reqs);
    assert_eq!(vetos.len(), 3, "三种裸形态条目都传播: {reqs:?}");

    let f2 = pf(vec![
        line(0, "第3章 评标办法"),
        line(1, ""),
        line(2, "有下列情形之一的，视为投标人相互串通投标："),
        line(3, "2）不同投标人委托同一单位或者个人办理投标事宜；"),
        line(4, "3）不同投标人的投标文件载明的项目管理成员之一为同一人；"),
    ]);
    let reqs2 = extract_requirements(&[f2]);
    assert_eq!(veto_texts(&reqs2).len(), 3, "裸闭括号条目 + 开启句自身: {reqs2:?}");
}

/// J：审查语境被编号条款行界定——r3 教训（FP 618：语境百页长开）。
#[test]
fn review_context_bounded_by_numbered_clause() {
    let f = pf(vec![
        line(0, "第5章 评标办法"),
        line(1, "5.1.1 形式评审标准：（见附表三：评标表-4）"),
        line(2, "⑴投标人名称应与营业执照一致；"),
        line(3, "5.2 评标委员会职责分工"), // 非审查头的编号条款 → 语境终结
        line(4, "⑴评标委员会分为技术组和商务组，进行分组评标。"),
    ]);
    let reqs = extract_requirements(&[f]);
    let vetos = veto_texts(&reqs);
    assert_eq!(vetos.len(), 1, "编号条款后的列举不继承审查语境: {reqs:?}");
    assert!(vetos[0].contains("投标人名称"));
}

/// A：总括句处置语义分辨——重新招标/评委回避的总括句不开否决语境。
#[test]
fn non_veto_disposal_openers_do_not_open_context() {
    let f = pf(vec![
        line(0, "第3章 评标办法"),
        line(1, "有下列情形之一的，招标人将重新招标："),
        line(2, "⑴投标截止时间止，投标人少于3个的；"),
        line(3, "⑵同意延长投标有效期的投标人少于3个的；"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert!(veto_texts(&reqs).is_empty(), "流标情形列举非否决: {reqs:?}");

    let f2 = pf(vec![
        line(0, "第3章 评标办法"),
        line(1, "评标委员会成员有下列情形之一的，应当主动回避："),
        line(2, "⑴项目主管部门或者行政监督部门的人员；"),
        line(3, "⑵与投标人有经济利益关系，可能影响对投标公正评审的；"),
    ]);
    let reqs2 = extract_requirements(&[f2]);
    assert!(veto_texts(&reqs2).is_empty(), "评委回避列举非否决（RECUSE 族）: {reqs2:?}");
}

/// L1/L3：拒收情形头（前附表行号形态）自身判否决 + 开语境；附录评标表
/// （强审查头）穿透格式章守卫。
#[test]
fn rejection_header_and_appendix_table_in_format_chapter() {
    let f = pf(vec![
        line(0, "第2章 投标人须知"),
        line(1, "24投标文件的拒收情形一、未密封的投标文件。"),
        line(2, "(2)未按招标文件要求加盖投标人公章，或未经法定代表人或者其授权代表签字；"),
        line(3, "(14)投标人提供的证书原件不在有效期内的；"),
    ]);
    let reqs = extract_requirements(&[f]);
    let vetos = veto_texts(&reqs);
    assert!(vetos.len() >= 2, "拒收情形头 + 条目: {reqs:?}");
    assert!(vetos.iter().any(|v| v.contains("拒收情形")), "头自身判否决");
    assert!(vetos.iter().any(|v| v.contains("加盖投标人公章")), "条目传播");

    // 附录评标表物理住在格式章区域——强头（评标表/序号 评审项目）旁路
    let f2 = pf(vec![
        line(0, "第6章 投标文件格式"),
        line(1, "评标表-6"),
        line(2, "序号 评审项目"),
        line(3, "投标人 备注"),
        line(4, "A B C ……"),
        line(5, "1 投标范围符合招标文件规定"),
        line(6, "2 计划工期符合招标文件规定"),
    ]);
    let reqs2 = extract_requirements(&[f2]);
    assert_eq!(veto_texts(&reqs2).len(), 2, "强审查头穿透格式章: {reqs2:?}");
}

/// F3/F5：投标失效/投标被拒绝处置句 + 前附表联合体不接受。
#[test]
fn validity_loss_and_consortium_rejection_strong_phrases() {
    let f = pf(vec![
        line(0, "第2章 投标人须知"),
        line(1, "3.3.2 出现特殊情况需要延长投标有效期的，招标人以书面形式通知所有投标人延长"),
        line(2, "投标有效期。投标人拒绝延长的，其投标失效，但投标人有权收回投标保证金。"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert!(veto_texts(&reqs).iter().any(|v| v.contains("其投标失效")),
        "投标失效是处置结果: {reqs:?}");

    let f2 = pf(vec![
        line(0, "第2章 投标人须知"),
        line(1, "1.4.2是否接受联合体投标☑不接受"),
    ]);
    let reqs2 = extract_requirements(&[f2]);
    assert!(veto_texts(&reqs2).iter().any(|v| v.contains("联合体")),
        "联合体不接受是资格性否决: {reqs2:?}");
}

/// F4：列举中段的引导句（「投标人本单位人员，必须同时满足以下条件：」）
/// 不误杀语境——其后条目继续传播。
#[test]
fn midlist_leadin_keeps_context() {
    let f = pf(vec![
        line(0, "第2章 投标人须知"),
        line(1, "9.2.2 下列行为，视为允许他人以本单位名义承揽工程："),
        line(2, "(1)投标人的法定代表人的委托代理人不是投标人本单位人员；"),
        line(3, "质量管理人员、安全管理人员(专职安全生产管理人员)不是本单位人员。"),
        line(4, "投标人本单位人员，必须同时满足以下条件："),
        line(5, "(3)投标人单位为其办理社会保险关系，或具有其它有效证明。"),
    ]);
    let reqs = extract_requirements(&[f]);
    let vetos = veto_texts(&reqs);
    assert!(vetos.iter().any(|v| v.contains("社会保险关系")),
        "引导句后的 (3) 条目不失语境: {reqs:?}");
}

/// K1/N1：开标查验证件清单不传播（GT 跨 7 件一致未标——标注口径）。
#[test]
fn checklist_documents_do_not_propagate() {
    let f = pf(vec![
        line(0, "第4章 开标与评标"),
        line(1, "6 由监督人依次查验以下内容，宣布查验结果："),
        line(2, "⑵项目经理是否持本人居民身份证到开标现场；"),
        line(3, "⑶投标保证金收据原件；"),
        line(4, "⑷组织机构代码证副本加盖公章的复印件、营业执照副本加盖公章的复印件。"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert!(veto_texts(&reqs).is_empty(), "查验证件清单非否决: {reqs:?}");
}

/// K4：履约主体守卫——合同期条款列举非投标否决；利害关系信号放行。
#[test]
fn performance_subject_guard_with_relation_bypass() {
    let f = pf(vec![
        line(0, "第7章 合同条款"),
        line(1, "有下列情形之一的，按违约处理："),
        line(2, "3承包人需配合发包人进行到货验收、项目单元工程验收。"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert!(veto_texts(&reqs).is_empty(), "履约验收条款非投标否决: {reqs:?}");

    let f2 = pf(vec![
        line(0, "第2章 投标人须知"),
        line(1, ""),
        line(2, "有下列情形之一的，不得参加投标："),
        line(3, "⑼与本招标项目的施工承包人以及建筑材料供应商有隶属关系或者其他利害关系；"),
    ]);
    let reqs2 = extract_requirements(&[f2]);
    assert!(veto_texts(&reqs2).iter().any(|v| v.contains("隶属关系")),
        "投标人侧利害关系放行（承包人是第三方对象）: {reqs2:?}");
}

/// D/M1：裸数字小节头 vs 审查条目——「7．合同授予」是头（清语境无 REQ），
/// 「1.投标范围符合招标文件规定」是条目（含审查谓词不被当头吞掉）。
#[test]
fn bare_digit_head_vs_review_item_disambiguation() {
    let f = pf(vec![
        line(0, "第3章 评标办法"),
        line(1, "2.1.3 响应性评审"),
        line(2, "1.投标范围符合招标文件规定"),
        line(3, "2.计划工期符合招标文件规定"),
    ]);
    let reqs = extract_requirements(&[f]);
    assert_eq!(veto_texts(&reqs).len(), 2, "审查条目不被头形态吞掉: {reqs:?}");

    let f2 = pf(vec![
        line(0, "第2章 投标人须知"),
        line(1, ""),
        line(2, "有下列情形之一的，其投标将被否决："),
        line(3, "⑴投标文件未按要求密封的；"),
        line(4, "7．合同授予"), // 小节头形态——清语境
        line(5, "⑴定标原则见前附表；"),
    ]);
    let reqs2 = extract_requirements(&[f2]);
    let vetos2 = veto_texts(&reqs2);
    // 2 = 总括句自身（将被否决——GT 对该形态标 true）+ ⑴密封条目；
    // 小节头「7．合同授予」后的 ⑴定标条目**不**在位（语境被头终结）
    assert_eq!(vetos2.len(), 2, "总括句 + 密封条目: {reqs2:?}");
    assert!(vetos2.iter().any(|v| v.contains("未按要求密封")));
    assert!(!vetos2.iter().any(|v| v.contains("定标原则")), "小节头后的列举不继承语境");
}
