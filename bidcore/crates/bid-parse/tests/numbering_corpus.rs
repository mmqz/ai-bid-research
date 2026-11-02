//! G5（#123 任务 2）真实语料验收：3 件水科 PDF 上的编号形态实测分布。
//!
//! 任务书验收面：
//! - 七种形态在**真实招标文件**中的在位证明（5 种高频在位——
//!   AlphaParen/Bullet 在水科语料零命中，属形态面而非语料面缺口，
//!   由单测锁定）；
//! - 层级推断（`1.1` = 2、`1.1.1` = 3）在真实行上成立；
//! - 混排从属：同一文件内 `一、`(1) 与 `1.1`(2) 并存且层级关系正确。
//!
//! 语料：real-corpus/water/files（政府采购公开文件，PROVENANCE.md 登记
//! 来源 URL + sha256）。解析走 [`bid_parse::parse_file`] 公共路径
//! （沙箱无 pdfium → pdftotext 通道；与上轮 artifacts/real-eval 产物同源）。
//!
//! 诚实纪律（issue #9）：语料缺失 → 响亮 SKIP，不伪装通过。
//! 断言取**下限**（形态在位证明），不锁精确计数（pdftotext 版本差异
//! 会引起行切分微差——确定性指同环境同输出，跨引擎版本属组件矩阵）。

use bid_parse::numbering::{detect_numbering, NumberingStyle};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn water_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../real-corpus/water/files")
}

fn counts(pdf: &str) -> BTreeMap<NumberingStyle, usize> {
    let path = water_dir().join(pdf);
    assert!(path.exists(), "语料在位检查应由 require_corpus 把关");
    let parsed = bid_parse::parse_file(&path, pdf).expect("水科 PDF 应可解析");
    let mut m: BTreeMap<NumberingStyle, usize> = BTreeMap::new();
    for b in &parsed.blocks {
        if let Some((style, _)) = detect_numbering(&b.text) {
            *m.entry(style).or_default() += 1;
        }
    }
    m
}

/// 诚实 SKIP：语料不在时不能用「pass」冒充通过。
/// CJK 字符占非空白字符的比例——真实中文招标文件的健康水位约 0.5~0.8。
///
/// 用来判「解析出来的到底是不是中文」。无 pdfium/poppler 的 Windows 环境里
/// 纯 Rust 兜底会把 GBK 双字节按单字节输出，中文全成 `NN`/`eW` 碎片，
/// 该比值会掉到接近 0（详见 issue #169）。用它区分「语料不在」与「解析环境不可用」。
fn cjk_ratio(s: &str) -> f64 {
    let total = s.chars().filter(|c| !c.is_whitespace()).count();
    if total == 0 {
        return 0.0;
    }
    let cjk = s
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    cjk as f64 / total as f64
}

/// 文本可用性下限。低于此值 → 判解析环境不可用，SKIP（不是业务失败）。
const MIN_CJK_RATIO: f64 = 0.25;

fn require_corpus() -> Option<()> {
    let d = water_dir();
    let have = [
        "A-塔里木水库除险加固招标文件.pdf",
        "A-某水利施工标段招标文件.pdf",
        "A-伊犁水库质量检测招标文件.pdf",
    ]
    .iter()
    .filter(|f| d.join(f).exists())
    .count();
    if have < 3 {
        eprintln!(
            "【SKIP】水科语料仅 {have}/3 件在位（{}）——真实分布验收不伪装通过",
            d.display()
        );
        return None;
    }
    // 解析保真度守卫（issue #169）：语料在位 ≠ 文本可用。
    // 无 pdfium/poppler 时纯 Rust 兜底把中文解成碎片，
    // 编号形态/层级分布全对不上——SKIP 而非假红（issue #9 口径）。
    {
        let probe = "A-塔里木水库除险加固招标文件.pdf";
        let parsed = bid_parse::parse_file(&d.join(probe), probe)
            .expect("水科 PDF 应可解析（能解析但文本可能不可用）");
        let joined: String = parsed.blocks.iter().map(|b| b.text.as_str()).collect();
        let ratio = cjk_ratio(&joined);
        if ratio < MIN_CJK_RATIO {
            eprintln!(
                "【SKIP】{probe} 解析文本 CJK 占比 {:.1}% < {:.0}%——无 pdfium/poppler 环境下纯 Rust 兜底的解码不可用（issue #169），真实分布验收不伪装通过",
                ratio * 100.0,
                MIN_CJK_RATIO * 100.0
            );
            return None;
        }
    }
    Some(())
}

/// 三件主文件的编号形态分布（下限断言 = 在位证明；数值实测于
/// pdftotext -layout 通道，见 PR 描述的分布表）。
#[test]
fn real_water_corpus_numbering_distribution() {
    if require_corpus().is_none() {
        return;
    }
    // 塔里木（284 页）——五形态全量在位，圈数字大本营（⑸ 系 570+ 行）
    let t = counts("A-塔里木水库除险加固招标文件.pdf");
    assert!(
        t.get(&NumberingStyle::ArabicMultiLevel)
            .copied()
            .unwrap_or(0)
            >= 1000,
        "塔里木多级点分应 ≥1000 行，实际 {t:?}"
    );
    assert!(
        t.get(&NumberingStyle::ChineseParen).copied().unwrap_or(0) >= 600,
        "塔里木括号数字应 ≥600 行，实际 {t:?}"
    );
    assert!(
        t.get(&NumberingStyle::Circled).copied().unwrap_or(0) >= 400,
        "塔里木圈数字应 ≥400 行，实际 {t:?}"
    );
    assert!(
        t.get(&NumberingStyle::ArabicDot).copied().unwrap_or(0) >= 80,
        "塔里木阿拉伯点顿应 ≥80 行，实际 {t:?}"
    );
    assert!(
        t.get(&NumberingStyle::ChineseNumeral).copied().unwrap_or(0) >= 40,
        "塔里木中文顿号应 ≥40 行，实际 {t:?}"
    );

    // 施工标段（142 页，GT 主体）
    let s = counts("A-某水利施工标段招标文件.pdf");
    assert!(
        s.get(&NumberingStyle::ArabicMultiLevel)
            .copied()
            .unwrap_or(0)
            >= 700,
        "施工标段多级点分应 ≥700 行，实际 {s:?}"
    );
    assert!(
        s.get(&NumberingStyle::ChineseParen).copied().unwrap_or(0) >= 250,
        "施工标段括号数字应 ≥250 行，实际 {s:?}"
    );
    assert!(
        s.get(&NumberingStyle::ArabicDot).copied().unwrap_or(0) >= 180,
        "施工标段阿拉伯点顿应 ≥180 行，实际 {s:?}"
    );
    assert!(
        s.get(&NumberingStyle::ChineseNumeral).copied().unwrap_or(0) >= 40,
        "施工标段中文顿号应 ≥40 行，实际 {s:?}"
    );

    // 伊犁（76 页）
    let y = counts("A-伊犁水库质量检测招标文件.pdf");
    assert!(
        y.get(&NumberingStyle::ArabicMultiLevel)
            .copied()
            .unwrap_or(0)
            >= 200,
        "伊犁多级点分应 ≥200 行，实际 {y:?}"
    );
    assert!(
        y.get(&NumberingStyle::ChineseParen).copied().unwrap_or(0) >= 150,
        "伊犁括号数字应 ≥150 行，实际 {y:?}"
    );
    assert!(
        y.get(&NumberingStyle::ArabicDot).copied().unwrap_or(0) >= 120,
        "伊犁阿拉伯点顿应 ≥120 行，实际 {y:?}"
    );
}

/// 层级语义在真实行上成立：多级点分行覆盖 2/3/4 级；混排从属
/// （同文件 `一、`=1 与 `1.1`=2 并存，从属方向正确——任务书验收 3）。
#[test]
fn real_corpus_levels_and_mixed_subordination() {
    if require_corpus().is_none() {
        return;
    }
    let path = water_dir().join("A-塔里木水库除险加固招标文件.pdf");
    let parsed = bid_parse::parse_file(&path, "A-塔里木水库除险加固招标文件.pdf").unwrap();
    let mut levels_of_dotted: Vec<u8> = Vec::new();
    let mut has_cn_l1 = false;
    let mut has_dotted_l2 = false;
    for b in &parsed.blocks {
        if let Some((style, level)) = detect_numbering(&b.text) {
            match style {
                NumberingStyle::ArabicMultiLevel => {
                    levels_of_dotted.push(level);
                    if level == 2 {
                        has_dotted_l2 = true;
                    }
                }
                NumberingStyle::ChineseNumeral if level == 1 => has_cn_l1 = true,
                _ => {}
            }
        }
    }
    assert!(levels_of_dotted.contains(&2), "应有二级点分行（1.1）");
    assert!(levels_of_dotted.contains(&3), "应有三级点分行（1.1.1）");
    assert!(
        has_cn_l1 && has_dotted_l2,
        "混排从属：`一、`(1) 与 `1.1`(2) 应并存（一、下跟 1.1 层级更深）"
    );
}
