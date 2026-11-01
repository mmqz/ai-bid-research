//! 补遗恢复回归（issue #103 跟进 · pdf_text 纯 Rust 兜底）。
//!
//! 实测背景（本回归用例锁定的情形）：在未装 pdfium 且未装 poppler 的环境里，
//! 补遗文件（招标文件的修正案，法律上优先于原件）会整个导入失败。
//! 后果不是「读不出来」那么简单：
//!
//! ```text
//! μ   补遗将开标时间从 2026-11-18 改为 2026-11-30
//! μ   补遗将控制价从 1650 万改为 1580 万 ← 按 1650 报价 = 超控制价 = 否决投标
//! μ   补遗新增「未提交回执确认函 → 招标人有权拒收」否决条款
//! …… 而系统不知道补遗存在，校验仍然 blocker=0
//! ```
//!
//! 本用例验收：全链路必须把补遗里的五个事实面都抽到。

use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/synth-tender/招标文件")
}

fn addendum() -> PathBuf {
    fixture_dir().join("C-补遗文件.pdf")
}

/// 诚实 SKIP：夹具不在时不能用「pass」冒充通过（issue #9 口径）。
fn require_fixture() -> Option<PathBuf> {
    let p = addendum();
    if !p.exists() {
        eprintln!("【SKIP】缺少补遗夹具{}（本回归不能验证，但不伪装通过）", p.display());
        return None;
    }
    Some(p)
}

/// 通过公开 API 路径解析（走的就是生产代码那条路，不是测试侧另开小址）。
#[test]
fn addendum_pdf_yields_text_through_public_parse_path() {
    let Some(p) = require_fixture() else { return };
    let parsed = bid_parse::parse_file(&p, "C-补遗文件.pdf")
        .expect("C-补遗.pdf 应能解析（纯 Rust 兜底）");

    assert!(parsed.has_text_layer, "补遗有文本层");
    assert!(!parsed.blocks.is_empty(), "应抽到非空文本块");
    let all: String = parsed.blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");

    // 五个事实面——少任何一条，补遗就等于没读到
    for (needle, label) in [
        ("2026-11-30", "延期后的开标/截止时间"),
        ("1580 万元", "调整后的控制价"),
        ("回执确认函", "新增的否决条款"),
        ("最多 4 分", "商务分修正"),
        ("踏勘", "踏勘与答疑安排"),
    ] {
        assert!(all.contains(needle), "补遗应含「{label}」({needle})，实际：\n{all}");
    }

    // 断言改为环境可移植：公共路径走「环境里可用的最好一条路」——
    // 有 poppler → 「pdf 文本层」（pdf.rs 首选路）；pdfium+poppler 全缺 →
    // 纯 Rust 兜底（自报「兜底」）。两条都是**诚实自报**：断言 summary 命中
    // 任一已知标记即可（原断言硬要「兜底」，在有 poppler 的机器上把
    // 走了首选路的成功解析误判为失败——环境敏感缺陷，issue #9 纪律：
    // 环境差异不得伪装成业务回归）。兜底行为本身由下方
    // addendum_fallback_recovers_and_self_reports 直接钉死（任何环境可复现）。
    assert!(
        parsed.parse_summary.contains("兜底") || parsed.parse_summary.contains("pdf 文本层"),
        "parse_summary 必须诚实自报所走的路（「兜底」或「pdf 文本层」），实际：{}",
        parsed.parse_summary
    );
}

/// 兜底路径行为钉死：直接调 pdf_text::parse_via_fallback（不依赖外部工具
/// 在位与否——任何环境都能验证兜底恢复 + 诚实自报）。
#[test]
fn addendum_fallback_recovers_and_self_reports() {
    let Some(p) = require_fixture() else { return };
    let parsed = bid_parse::pdf_text::parse_via_fallback(&p, "C-补遗文件.pdf")
        .expect("C-补遗.pdf 应能走纯 Rust 兜底恢复");
    assert!(parsed.has_text_layer, "兜底从文本层恢复");
    let all: String = parsed
        .blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for (needle, label) in [
        ("2026-11-30", "延期后的开标/截止时间"),
        ("1580 万元", "调整后的控制价"),
        ("回执确认函", "新增的否决条款"),
        ("最多 4 分", "商务分修正"),
    ] {
        assert!(all.contains(needle), "兜底应含「{label}」({needle})，实际：\n{all}");
    }
    assert!(
        parsed.parse_summary.contains("兜底"),
        "兜底路径必须在 summary 里自报（对账能区分正常链路/兜底），实际：{}",
        parsed.parse_summary
    );
}

/// 突出岛控件�：兜底对「不是 PDF」要明说失败，不能吐出乱码当文本。
#[test]
fn fallback_reports_failure_instead_of_garbage() {
    let dir = std::env::temp_dir().join(format!("bidparse-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("not-really.pdf");
    std::fs::write(&fake, b"this is plain text, not a PDF at all").unwrap();

    let r = bid_parse::parse_file(&fake, "not-really.pdf");
    assert!(r.is_err(), "非 PDF 应明说失败，实际：{r:?}");
    let msg = format!("{}", r.unwrap_err());
    assert!(
        msg.contains("不是合法 PDF") || msg.contains("%PDF"),
        "失败原因要能读懂：{msg}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
