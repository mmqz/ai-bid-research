//! #272 跨平台章节结构验收（issue 五条验收的机证面）。
//!
//! | 验收 | 测试 |
//! |---|---|
//! | 1. HLJ 5 份识别结果不变 | `hlj_recognition_unchanged`（结构方案 + 类别分布锁定）|
//! | 2. NM 3 份识别 7 章结构 | `nm_seven_chapters_recognized` |
//! | 3. 询价识别数字编号结构 | `inquiry_part_scheme_and_recovery`（含类别恢复）|
//! | 4. 分类与章名解耦 | `classification_survives_chapter_renaming`（合成——无需 poppler）|
//! | 5. 识别不确定响亮告警 | `inquiry_part_scheme_and_recovery`（parse_summary 断言）+
//! |   | structure.rs 单测（Uncertain → warnings 非空）|
//!
//! GT 红线（L1 ≥309/318、否决 ≥291/302）由 `real_corpus_e2e` 承载——
//! 本文件锁定**结构识别面**与**类别分布面**。batch1/NM 的逐条 REQ 指纹
//! 由 `structure_probe_272` 采集 diff（开发侧基线对照已零 diff——见 PR 描述）。
//!
//! 防过拟合标注：形态家族与阈值基于 3 个平台来源（黑龙江政采 5 /
//! 内蒙古政采 3 / 询价 1），未验证其它省份。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bid_parse::extract::extract_requirements;
use bid_parse::structure::{detect_structure, TopScheme};
use bid_parse::{Block, BlockKind, ParsedFile};

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while p.join("fixtures").read_dir().is_err() {
        p = p
            .parent()
            .map(|x| x.to_path_buf())
            .expect("repo root not found");
    }
    p
}

/// 主件 PDF（跳过附件/ 目录）。
fn main_pdf(dir: &Path) -> Option<PathBuf> {
    dir.read_dir()
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.extension().and_then(|x| x.to_str()) == Some("pdf")
                && p.is_file()
        })
}

fn parsed_of(rel: &str) -> ParsedFile {
    let d = repo_root().join(rel);
    let pdf = main_pdf(&d).unwrap_or_else(|| panic!("语料缺件: {rel}"));
    let name = pdf.file_name().unwrap().to_string_lossy().to_string();
    bid_parse::parse_file(&pdf, &name).expect("解析失败")
}

fn cat_counts(reqs: &[bid_domain::Requirement]) -> BTreeMap<&str, usize> {
    let mut m: BTreeMap<&str, usize> = BTreeMap::new();
    for r in reqs {
        *m.entry(r.category.as_str()).or_default() += 1;
    }
    m
}

fn poppler_ok() -> bool {
    bid_parse::env_preflight::missing_tools(bid_parse::env_preflight::POPPLER_TEXT_TOOLS).is_empty()
}

// ---------------------------------------------------------------- 验收 1：HLJ 识别不变

/// 黑龙江 5 份：结构方案 = Zhang（章骨架），类别分布与既有口径一致
/// （基线来自 main 实测——structure_probe_272 采集，逐条 digest 零 diff）。
#[test]
fn hlj_recognition_unchanged() {
    if !poppler_ok() {
        eprintln!(
            "【SKIP】hlj_recognition_unchanged：无 poppler（#9 口径——环境差异不伪装业务结果）"
        );
        return;
    }
    // (目录, 类别基线：main 实测)
    let expected: &[(&str, &[(&str, usize)])] = &[
        (
            "fixtures/real-tender-batch1/college-ai-230001",
            &[("其他", 115), ("否决项", 51), ("商务要求", 14), ("技术参数", 5),
              ("格式要求", 96), ("评分项", 15), ("资格条件", 21)],
        ),
        (
            "fixtures/real-tender-batch1/danqing-reservoir-230001",
            &[("其他", 113), ("否决项", 51), ("商务要求", 12), ("技术参数", 3),
              ("格式要求", 96), ("评分项", 17), ("资格条件", 21)],
        ),
        (
            "fixtures/real-tender-batch1/hc-cluster-230001",
            &[("其他", 97), ("否决项", 49), ("商务要求", 11), ("技术参数", 15),
              ("格式要求", 96), ("评分项", 15), ("资格条件", 20)],
        ),
        (
            "fixtures/real-tender-batch1/hq-dev-procure-230001",
            &[("其他", 99), ("否决项", 95), ("商务要求", 26), ("技术参数", 77),
              ("格式要求", 96), ("评分项", 19), ("资格条件", 32)],
        ),
        (
            "fixtures/real-tender-batch1/wenming-longjiang-230001",
            &[("其他", 113), ("否决项", 51), ("商务要求", 11), ("技术参数", 2),
              ("格式要求", 96), ("评分项", 18), ("资格条件", 21)],
        ),
    ];
    for (rel, base) in expected {
        let pf = parsed_of(rel);
        // 结构方案：章骨架
        let st = detect_structure(&pf.blocks);
        assert_eq!(st.scheme, TopScheme::Zhang, "{rel} 应为章骨架");
        // parse_summary 自报（#272 响亮面）
        assert!(
            pf.parse_summary.contains("结构形态[第X章]"),
            "{rel} parse_summary 应含结构形态自报：{}",
            pf.parse_summary
        );
        // 类别分布逐项锁定（识别结果不变）
        let reqs = extract_requirements(std::slice::from_ref(&pf));
        let got = cat_counts(&reqs);
        for (cat, n) in *base {
            assert_eq!(
                got.get(*cat).copied().unwrap_or(0),
                *n,
                "{rel} 类别「{cat}」计数漂移（识别结果不变红线）"
            );
        }
    }
}

// ---------------------------------------------------------------- 验收 2：NM 7 章结构

/// 内蒙古 3 份：章骨架识别在位——正文章号覆盖 1..=7（康熙部首经 NFKC
/// 归一后可认；TOC 区与正文区双份标记，去重后 7 章）。
#[test]
fn nm_seven_chapters_recognized() {
    if !poppler_ok() {
        eprintln!("【SKIP】nm_seven_chapters_recognized：无 poppler（#9 口径）");
        return;
    }
    for rel in [
        "fixtures/real-tender-batch2/nm-lib-mongolian",
        "fixtures/real-tender-batch2/nm-health-terminal",
        "fixtures/real-tender-batch2/nm-property-mgmt",
    ] {
        let pf = parsed_of(rel);
        let st = detect_structure(&pf.blocks);
        assert_eq!(st.scheme, TopScheme::Zhang, "{rel} 应为章骨架");
        let nos: std::collections::BTreeSet<u32> =
            st.chapters.iter().map(|c| c.no).collect();
        for no in 1..=7u32 {
            assert!(
                nos.contains(&no),
                "{rel} 缺第{no}章（识别 7 章结构验收；实得 {nos:?}）"
            );
        }
        eprintln!("[#272] {rel}: 章号集合 {nos:?}（7 章结构在位）");
    }
}

// ---------------------------------------------------------------- 验收 3+5：询价结构与恢复

/// 询价件：顶层为「第X部分」骨架（TOC 实证：第一部分 投标邀请 …
/// 第七部分 响应文件格式；issue 表中的 `3.供应商资格要求` 等数字头是
/// 第二部分须知内的条款层——本测试同时断言该层可见）。类别面恢复：
/// 基线 其他 71% → 分类健康；parse_summary 含结构自报（不静默失败）。
#[test]
fn inquiry_part_scheme_and_recovery() {
    if !poppler_ok() {
        eprintln!("【SKIP】inquiry_part_scheme_and_recovery：无 poppler（#9 口径）");
        return;
    }
    let pf = parsed_of("fixtures/real-tender-batch2/hlj-inquiry");
    let st = detect_structure(&pf.blocks);

    // 顶层方案：部分骨架（此前 re_chapter 只认「章」→ 章上下文恒空 = 全灭根因）
    assert_eq!(st.scheme, TopScheme::Part, "询价件应为部分骨架");
    // 枚举簇抑制：「询价文件构成」节里逐行列举的七部分不是边界——
    // 真部分恰好 7 个（第一..第七）
    let nos: std::collections::BTreeSet<u32> = st.chapters.iter().map(|c| c.no).collect();
    assert_eq!(
        nos,
        (1..=7u32).collect::<std::collections::BTreeSet<_>>(),
        "枚举簇抑制后应恰有 7 个真部分"
    );

    // 数字编号层可见（验收 3：能识别数字编号结构——第二部分须知内的
    // 1..25 条款头 + 6.x 点分子层，由 pend_head_form/点分条款面承载）
    let numbered: usize = pf
        .blocks
        .iter()
        .filter(|b| bid_parse::structure::is_numbered_head_form(&b.text))
        .count();
    assert!(
        numbered >= 20,
        "数字头候选应可见（实得 {numbered}——issue 骨架面）"
    );

    // 响亮自报（验收 5 面）：parse_summary 含结构形态，不静默
    assert!(
        pf.parse_summary.contains("结构形态[第X部分/篇]"),
        "parse_summary 应含部分骨架自报：{}",
        pf.parse_summary
    );

    // 类别恢复（基线：其他 243/344 = 71%，技术参数/格式要求双零）
    let reqs = extract_requirements(std::slice::from_ref(&pf));
    let got = cat_counts(&reqs);
    let n = |c: &str| got.get(c).copied().unwrap_or(0);
    assert!(n("技术参数") >= 10, "技术参数应恢复（基线 0）：{}", n("技术参数"));
    assert!(n("评分项") >= 30, "评分项应恢复（基线 1）：{}", n("评分项"));
    assert!(n("格式要求") >= 1, "格式要求应恢复（基线 0）：{}", n("格式要求"));
    assert!(
        n("其他") <= 180,
        "其他类应大幅回落（基线 243）：{}",
        n("其他")
    );
    eprintln!("[#272] 询价类别恢复: {got:?}（总 {}）", reqs.len());
}

// ---------------------------------------------------------------- 验收 4：分类与章名解耦

fn mk_pdf_file(texts: &[&str]) -> ParsedFile {
    let blocks: Vec<Block> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| Block {
            idx: i,
            kind: BlockKind::PdfLine,
            locator: format!("第{}页 行{}", i / 40 + 1, i % 40 + 1),
            text: (*t).to_string(),
            cells: vec![],
        })
        .collect();
    ParsedFile {
        name: "A-test.pdf".into(),
        sha256: String::new(),
        pages: 1,
        tables: 1,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary: String::new(),
        components: vec![],
    }
}

/// 改章名不影响分类结果：同一份「第X部分」文档，把「第四部分 采购需求」
/// 改成无语义名「第四部分 香蕉」——章名字面失配后，参数语义表的内容
/// 证据接管分类，参数表条目仍落「技术参数」（issue：章节含「参数表/
/// 技术要求」表格 → 技术参数）。纯合成（无 poppler 依赖——CI 可跑）。
#[test]
fn classification_survives_chapter_renaming() {
    let doc = |title4: &'static str| -> Vec<&'static str> {
        vec![
            "第一部分 投标邀请",
            "邀请正文第一行，内容若干。",
            "邀请正文第二行，内容若干。",
            "邀请正文第三行，内容若干。",
            "第二部分 供应商须知",
            "须知正文第一行，内容若干。",
            "须知正文第二行，内容若干。",
            "须知正文第三行，内容若干。",
            title4,
            "一、技术标内容",
            "参数名称 参数要求",
            "1 整体尺寸 标准段长度3000mm",
            "2 护栏高度 中央隔离1250mm人行道隔离1100mm",
            "3 立柱间距 3000mm",
        ]
    };
    let original = mk_pdf_file(&doc("第四部分 采购需求"));
    let renamed = mk_pdf_file(&doc("第四部分 香蕉"));

    for pf in [&original, &renamed] {
        let st = detect_structure(&pf.blocks);
        assert_eq!(st.scheme, TopScheme::Part, "应为部分骨架");
    }

    let reqs_of = |pf: &ParsedFile| -> Vec<String> {
        extract_requirements(std::slice::from_ref(pf))
            .into_iter()
            .filter(|r| r.category == "技术参数")
            .map(|r| {
                r.anchors
                    .first()
                    .map(|a| a.excerpt.clone())
                    .unwrap_or_default()
            })
            .collect()
    };
    let from_original = reqs_of(&original);
    let from_renamed = reqs_of(&renamed);

    // 字面命中版：语义表条目 ≥3（参数表装配在位——否则测试本身失效）
    assert!(
        from_original.len() >= 3,
        "合成参数表应产出技术参数条目（实得 {}——夹具失效）",
        from_original.len()
    );
    // 改名版：分类结果不变（内容证据接管）
    assert_eq!(
        from_original, from_renamed,
        "改章名（采购需求→香蕉）不得改变参数表条目的分类结果"
    );
}
