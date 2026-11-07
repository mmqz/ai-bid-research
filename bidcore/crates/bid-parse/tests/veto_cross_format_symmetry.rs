//! veto_cross_format_symmetry.rs —— #428（B-06）：否决词表单一真源的
//! 跨格式对称锁定。
//!
//! 审计 module-audit-B.md B-06：DOCX/PDF 否决词表两套不同源（零共享、
//! 差集两向 ~26 词面）——同一条款跨格式分类不同，GT/评测按格式分桶时
//! 数字不可比。修复后（词面并入 veto_lexicon 共享集，PDF 侧消费
//! `re_veto_union`）本文件锁定四件事：
//!
//! 1. **单源锁**：union 词面 == strong ∪ negative 的显式派生
//!    （防 union 私自加词——词面只能改 strong_patterns /
//!    negative_patterns 两处声明面）+ 空分支防护（alternation 出现
//!    匹配空串的分支 → is_veto 恒真——#428 实测踩过：concat 尾竖线）；
//! 2. **词面层**：差集探针（两向 × strong/negative 两层）逐条命中
//!    `is_veto`——审计触发例「本次招标不接受联合体投标」原 DOCX 侧
//!    miss（B-06 差集向 ①），现两侧词面同判 true；
//! 3. **分类层对称（处置形）**：差集探针中文本为处置形（非规则说明）
//!    的，在 docx / pdf 两格式下 `extract_requirements` 分类**全等**且
//!    落「否决项」——修复前 PDF 侧落否决项 / DOCX 侧落章兜底类，
//!    即审计「同一条款跨格式分类不同」的本体；
//! 4. **降级不误伤**：规则说明形（#226 层 2 有意降级——判否决项会造成
//!    永久 blocking 死锁）与非否决文本（资格正面/例外从句/词面边界）
//!    两侧均**不落否决项**——合并变宽不制造跨格式假阳性。
//!
//! 探针均为「编号头 + 章上下文」形态（两侧落账的最小共同形态——
//! PDF 侧无编号头的裸行会被段落重组吞掉，见 #68 形态解剖）。避开
//! PDF 侧判定链的格式特有排除面（提疑 N3 / 履约语境 K4）。
//!
//! **如实登记（相邻面，不在本单）**：降级组两侧**落类**不同
//! （docx=资格条件[locator 子节串切出的「资格审查」章] vs pdf=其他
//! [章行兜底]）——is_veto 词面层两侧同判（对称修复本体 ✓），分类层
//! 降级后的落类差异源自两侧章上下文构造形态不同源，属 #226/#233
//! 既有行为，B-06 词面单不碰。

use bid_parse::extract::extract_requirements;
use bid_parse::veto_lexicon;
use bid_parse::{Block, BlockKind, ParsedFile};

fn docx_pf(text: &str) -> ParsedFile {
    ParsedFile {
        name: "A-test.docx".to_string(),
        sha256: "test".into(),
        pages: 1,
        tables: 0,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks: vec![Block {
            idx: 0,
            kind: BlockKind::Para,
            locator: "第二章 投标人须知 > 3.资格审查".into(),
            text: format!("2.3 {text}"),
            cells: vec![],
        }],
        parse_summary: "test".into(),
        components: vec![],
    }
}

fn pdf_pf(text: &str) -> ParsedFile {
    ParsedFile {
        name: "A-test.pdf".to_string(),
        sha256: "test".into(),
        pages: 1,
        tables: 0,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks: vec![
            Block {
                idx: 0,
                kind: BlockKind::PdfLine,
                locator: "l0".into(),
                text: "第二章 投标人须知".into(),
                cells: vec![],
            },
            Block {
                idx: 1,
                kind: BlockKind::PdfLine,
                locator: "l1".into(),
                text: format!("2.3 {text}"),
                cells: vec![],
            },
        ],
        parse_summary: "test".into(),
        components: vec![],
    }
}

fn cats(reqs: &[bid_domain::Requirement]) -> Vec<String> {
    reqs.iter().map(|r| r.category.clone()).collect()
}

/// 差集探针·处置形（审计 B-06 差集原文逐条，is_rule_statement=false 的
/// 选段）——修复前 PDF 侧命中词面（落否决项）而 DOCX 侧 miss（落章
/// 兜底类）：跨格式分类不同的本体。修复后两侧分类全等且落「否决项」。
const DIFFSET_PROBES: &[(&str, &str)] = &[
    // 差集向 ①（PDF 有 / DOCX 无）——strong 层；修复前 DOCX 侧词面 miss
    ("投标失效（#68 F3）", "投标人拒绝延长的，其投标失效，投标文件被退回。"),
    ("拒收情形头（#68 L1）", "投标文件的拒收情形（前附表行号头）。"),
    ("以他人名义投标（#68）", "下列行为均属以他人名义投标。"),
    ("利害关系不得参加（词面形态）", "与招标人存在利害关系的单位不得参加投标。"),
    // 差集向 ②（DOCX 有 / PDF 无）——strong 层；修复前 PDF 侧词面 miss。
    // 词面层命中由 diffset_probes_hit_is_veto 覆盖；分类层对称由
    // SYMMETRY_ONLY（不予评审）与 DOWNGRADE（投标无效）两组各按其
    // 分派路径锁定。
    // 差集向 ②——negative 层；修复前 PDF 侧词面 miss（DOCX 原有）
    ("重大安全事故", "近三年内发生重大安全事故的企业，不得参加本次投标。"),
    ("私下接触（#233）", "不私下接触招标人、招标代理机构或评标委员会成员。"),
    ("被责令停业", "被责令停业且在停业期内的投标人，其投标文件将被拒收。"),
];

/// 差集探针·仅对称（处置形但走两侧共有的降级/兜底路径——「招标人有权
/// 取消其中标资格」是 #226 复核登记过的处置条款，两侧同判「其他」：
/// 分类全等成立，但不在否决项组——断言只锁对称）。
const SYMMETRY_ONLY_PROBES: &[(&str, &str)] = &[
    ("有权取消（处置条款）", "中标人无正当理由拒签合同的，招标人有权取消其中标资格。"),
    ("不予评审（程序条款）", "开标时发现未按要求提交原件的，不予评审。"),
];

/// 规则说明形（is_veto=true 但 #226 层 2 有意降级——判否决项=永久
/// blocking 死锁）+ 词面边界（`[^，。；]{0,32}` 跨不过逗号——两侧一致
/// miss，真实语料该形态走列举语境传播路径，不经词面）。
/// 断言：词面层命中（is_veto）/分类层两侧均不落否决项（降级不误伤）。
const DOWNGRADE_PROBES: &[(&str, &str)] = &[
    ("不接受联合体（审计触发例·规则说明）", "本次招标不接受联合体投标。"),
    ("投标无效（规则说明用法）", "逾期送达或者未按招标文件要求密封的投标文件，其投标无效。"),
];

/// 负例——非否决文本（资格正面/例外从句/无词面规则说明/词面边界逗号
/// 形态）。合并变宽不得把它们误伤成否决；is_veto=false + 分类非否决项。
const NEGATIVE_PROBES: &[(&str, &str)] = &[
    ("资格正面（无否决词）", "投标人须具备水利水电工程施工总承包贰级及以上资质。"),
    ("例外从句（除…外）", "除不可抗力因素外，投标人应按时递交投标文件。"),
    ("规则说明（无词面）", "本次招标仅接受公开招标方式。符合本项规定。"),
    ("利害关系·逗号形态（词面边界）", "与招标人存在利害关系可能影响招标公正性的单位，不得参加投标。"),
];

/// 单源锁：union 的构建串必须等于 strong | negative 的显式派生。
/// union 是 PDF 侧消费面——若有人在 union 里私加词面（绕过两层声明），
/// 此测试红。
#[test]
fn union_is_derived_from_two_layers() {
    let derived = format!(
        "{}|{}",
        veto_lexicon::strong_patterns(),
        veto_lexicon::negative_patterns()
    );
    assert_eq!(
        veto_lexicon::re_veto_union().as_str(),
        derived,
        "union 必须由 strong|negative 派生——单一真源不旁路（#428/B-06）"
    );
    // 空分支防护：任一层或拼接处多出的 `|`（`||`）会让 alternation 匹配
    // 空串 → is_veto 恒真（#428 实测教训：concat 尾竖线 + union 拼接竖线）。
    // 空串与纯标点文本都必须不命中。
    for t in ["", "。", "，；", "除不可抗力外，投标人应按期完成"] {
        assert!(
            !veto_lexicon::is_veto(t),
            "空分支回归：'{t}' 不应命中否决词面（alternation 出现空分支）"
        );
    }
    assert!(
        !veto_lexicon::re_veto_union().is_match(""),
        "union 正则不得匹配空串（空 alternation 分支）"
    );
}

/// 词面层快照：差集探针逐条命中 is_veto（差集修复的最小面——不依赖
/// extract 主链，纯 veto_lexicon 判定）。降级组同样命中（词面层两侧
/// 同判是 B-06 的修复本体；分类层降级见下）。负例不命中。
#[test]
fn diffset_probes_hit_is_veto() {
    for (label, text) in DIFFSET_PROBES
        .iter()
        .chain(SYMMETRY_ONLY_PROBES)
        .chain(DOWNGRADE_PROBES)
    {
        assert!(
            veto_lexicon::is_veto(text),
            "差集探针未命中词面（#428 修复失效？）：{label}：{text}"
        );
    }
    for (label, text) in NEGATIVE_PROBES {
        assert!(
            !veto_lexicon::is_veto(text),
            "负例被误伤为否决（合并变宽的假阳性——违反对称锁前提）：{label}：{text}"
        );
    }
}

/// 分类层对称（处置形）：差集探针在 docx / pdf 两格式下分类**全等**且
/// 落「否决项」——修复前 PDF 侧落否决项 / DOCX 侧落章兜底类（跨格式
/// 分类不同的本体，审计 B-06 原文）。
#[test]
fn cross_format_symmetry_on_diffset_probes() {
    for (label, text) in DIFFSET_PROBES {
        let d = cats(&extract_requirements(&[docx_pf(text)]));
        let p = cats(&extract_requirements(&[pdf_pf(text)]));
        assert_eq!(
            d,
            p,
            "跨格式分类不一致（#428 修复失效）：{label} docx={d:?} pdf={p:?}"
        );
        assert!(
            d.iter().any(|c| c == "否决项"),
            "处置形差集探词应同落否决项：{label} 类别={d:?}"
        );
    }
}

/// 分类层对称（仅对称组）：两侧分类全等（「有权取消」「不予评审」走
/// 两侧共有的兜底路径——同判「其他」/「否决项」由各自分支决定，
/// 锁的是**跨格式一致**）。
#[test]
fn cross_format_symmetry_symmetry_only_probes() {
    for (label, text) in SYMMETRY_ONLY_PROBES {
        let d = cats(&extract_requirements(&[docx_pf(text)]));
        let p = cats(&extract_requirements(&[pdf_pf(text)]));
        assert_eq!(
            d, p,
            "跨格式分类不一致（对称锁失效）：{label} docx={d:?} pdf={p:?}"
        );
    }
}

/// 降级不误伤：规则说明形（#226 层 2 有意降级）与非否决文本两侧均不落
/// 「否决项」。降级组的两侧**落类**可能不同（docx=资格条件[locator
/// 子节章] vs pdf=其他[章行兜底]）——is_veto 词面层两侧同判（本单修复
/// ✓），分类层降级落类差异源自两侧章上下文构造形态不同源（相邻面，
/// 见文件头注释）——故本组断言两侧非否决项，不断言落类全等。
#[test]
fn downgrade_and_negative_probes_stay_off_veto() {
    for (label, text) in DOWNGRADE_PROBES.iter().chain(NEGATIVE_PROBES) {
        let d = cats(&extract_requirements(&[docx_pf(text)]));
        let p = cats(&extract_requirements(&[pdf_pf(text)]));
        for (side, c) in [("docx", &d), ("pdf", &p)] {
            assert!(
                !c.iter().any(|x| x == "否决项"),
                "降级/负例误落否决项（{side} 侧假阳性）：{label} 类别={c:?}"
            );
        }
    }
}
