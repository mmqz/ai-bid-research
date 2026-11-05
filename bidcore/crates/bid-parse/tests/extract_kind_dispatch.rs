//! #196 / #199 审查回归：按业务 kind 分发的值域矩阵。
//!
//! 背景：`extract_requirements_with_kind` 的分发必须与导入面
//! `TENDER_FILE_KINDS`（招标文件|附件|图纸|补遗|答疑|评分办法|格式模板）
//! 对齐。初版（#199 首commit）把主抽取写成封闭白名单
//! （招标文件|补遗|澄清），「答疑/评分办法/格式模板」三分支全不命中 →
//! 静默跳过——#196 要修的「真实文件名静默零抽取」换了马甲回来。
//! 现行为反向清单：仅附件/图纸/证据跳过，其余登记 kind 一律主抽取
//!（fail-visible，配合工具面 ZERO_REQUIREMENTS 告警）。

use bid_parse::extract::{extract_requirements, extract_requirements_with_kind};
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

/// 一份含两条可抽取条款的主文件内容（编号段落 + 章节定位）。
fn main_content() -> Vec<Block> {
    vec![
        para(
            0,
            "第二章 投标人须知 > 3.资格",
            "3.4.2 投标人未提供有效营业执照的，其投标将被否决。",
        ),
        para(
            1,
            "第二章 投标人须知 > 3.资格",
            "3.4.3 投标人须具备水利水电工程施工总承包贰级及以上资质。",
        ),
    ]
}

// #271：「格式模板」从主抽取面移入无条款面——格式件是官方填空模板
// （声明函/偏离表），条款真源是主件格式章 REQ；与 附件/图纸/证据 同口径。
const KINDS_MAIN: &[&str] = &["招标文件", "补遗", "答疑", "评分办法"];
const KINDS_SKIP: &[&str] = &["附件", "图纸", "证据", "格式模板"];

/// 主抽取面：TENDER_FILE_KINDS 中除附件/图纸/证据外的全部登记 kind，
/// 无论文件名带不带夹具前缀，都必须产出条款（#196 核心 + 审查修复面）。
#[test]
fn registered_kinds_dispatch_to_main_extraction() {
    for kind in KINDS_MAIN {
        for name in ["光纤租赁费招标文件.docx", "A-招标文件.docx"] {
            let f = pf(name, main_content());
            let reqs =
                extract_requirements_with_kind(&[f], |n| if n == name { Some(kind.to_string()) } else { None });
            assert_eq!(
                reqs.len(),
                2,
                "kind={kind} name={name} 应主抽取 2 条（实际 {}）",
                reqs.len()
            );
        }
    }
}

/// 附件面：附件/图纸/证据无条款抽取面——即使文件名带 A- 前缀也跳过
/// （kind 是登记的业务事实，优先于夹具前缀约定）。
#[test]
fn attachment_kinds_skip_even_with_main_prefix() {
    for kind in KINDS_SKIP {
        let f = pf("A-招标文件.docx", main_content());
        let reqs = extract_requirements_with_kind(&[f], |_| Some(kind.to_string()));
        assert!(
            reqs.is_empty(),
            "kind={kind} 应跳过抽取（实际 {} 条）",
            reqs.len()
        );
    }
}

/// 兼容面：kind 缺省（None）时前缀兜底——真实文件名（无前缀）不再
/// 静默零抽取（#196 原始缺陷的回归锁），B1/B2 前缀行为不变。
#[test]
fn none_kind_falls_back_to_prefix_and_real_names_extract() {
    // 真实文件名 + 无 kind → 主抽取（#196 原始缺陷）
    let f = pf("测试文件.docx", main_content());
    let reqs = extract_requirements_with_kind(&[f], |_| None);
    assert_eq!(reqs.len(), 2, "真实文件名（无 kind）不得静默零抽取");
    // 旧兼容入口 = kind 全 None 的等价面
    let reqs_old = extract_requirements(&[pf("测试文件.docx", main_content())]);
    assert_eq!(reqs_old.len(), 2, "兼容入口与 None-kind 语义一致");
    // B1/B2 前缀兜底保持（夹具约定路径不受 kind 缺省影响）
    let b1 = pf(
        "B1-设备清单.docx",
        vec![para(0, "附表1", "设备清单表内容——型号与数量")],
    );
    let reqs_b1 = extract_requirements_with_kind(&[b1], |_| None);
    // B1 路径产出形状由 b1 抽取器决定（可能为 0），关键是【不走主抽取
    // 抛错】——此处仅验证分发不 panic 且有定义行为。
    let _ = reqs_b1;
}
