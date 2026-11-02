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
/// #169 修复后此测试在任何环境都应通过：
///   - poppler 解码正常（装了 poppler-data）→ 首选路直出中文；
///   - poppler 缺 CMap 资源包（Windows runner 实测形态：首选路「成功」
///     吐 Latin-1 化乱码残渣）→ 被 probe_cjk_garble 拦截（PDF 声明 CJK
///     字体而文本无 CJK）→ 降级纯 Rust 兜底重解，summary 诚实自报；
///   - 无 poppler → 直接走兜底（原有第三条降级路）。
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

// ================================================================ #169：CJK 乱码防线

/// #169 判据单元面：同一份 PDF 字节（声明 CJK 字体），三种输出三态判定。
/// CI 实测乱码残渣串（Latin-1 化 + 不可打印字节被清洗后的形态）作 Garbled 样本。
#[test]
fn cjk_garble_probe_three_states() {
    let Some(p) = require_fixture() else { return };
    let raw = std::fs::read(&p).unwrap();
    assert!(bid_parse::pdf_text::bytes_declare_cjk_font(&raw),
        "补遗 PDF 字典应声明 CJK 字体/CMap（STSong-Light + Adobe-GB1）");

    // 正常解码的中文 → Clean
    let good = "某市第一人民医院智慧病房物联网平台建设项目补遗文件（01 号）\n\
        1.1 原招标文件规定的开标时间变更为 2026-11-30 09:30；调整后的控制价 1580 万元。";
    assert_eq!(bid_parse::pdf_text::probe_cjk_garble(&raw, good),
        bid_parse::pdf_text::CjkGarbleProbe::Clean);

    // CI 实测乱码残渣（Windows poppler 缺 CMap 资源的输出形态）→ Garbled
    let garbled = "g--^{,NN\"lS;bfzagu?b?riTQ^sS^oeyveWeN01 S\n\
        s[?bheNLRQ[,,O\\YNonNOe90g,eWeNN:bheNv~~bLRNbheNQwgT{Il_eHR\n\
        NNNKYNg,eWeNN:Q0bhN\"e6R0g,eWT 24 \\eQNfbVbgxn/0\n\
        1.1 SbheN~[v_he\"2026-11-18 09:30\"SfN:\"2026-11-30 09:30\"";
    assert_eq!(bid_parse::pdf_text::probe_cjk_garble(&raw, garbled),
        bid_parse::pdf_text::CjkGarbleProbe::Garbled);

    // 无 CJK 字体声明的字节 → 永不触发（纯英文件防误伤）
    let english_pdf = b"%PDF-1.4\n1 0 obj\n<< /BaseFont /Helvetica /Type /Font /Subtype /Type1 >>\nendobj\n%%EOF";
    assert_eq!(bid_parse::pdf_text::probe_cjk_garble(english_pdf, garbled),
        bid_parse::pdf_text::CjkGarbleProbe::NoSignal);

    // 样本不足（<32 非空白）→ NoSignal（空文本层由上层既有判定处理）
    assert_eq!(bid_parse::pdf_text::probe_cjk_garble(&raw, "short text"),
        bid_parse::pdf_text::CjkGarbleProbe::NoSignal);
}

/// 最小 PDF：字典声明 SimSun，内容流是不压缩的英文 literal string。
/// pdftotext 对它输出正常英文（cjk_ratio=0）→ 命中 Garbled 判据 →
/// 降级兜底 → 兜底输出同样无中文 → **双路皆乱 fail-loud**。
/// 这是 #169 影响面的最终防线：宁可失败，不静默产出垃圾。
#[test]
fn both_paths_garbled_fails_loudly() {
    let content = b"BT /F1 12 Tf 72 720 Td (This PDF declares a CJK font but contains only English text) Tj ET";
    let pdf = format!(
        "%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
         2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
         3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n\
         4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /SimSun >>\nendobj\n\
         5 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n\
         xref\n0 6\ntrailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
        content.len(),
        String::from_utf8_lossy(content),
        0 // xref 偏移占位（pdf_text 兜底不依赖 xref；pdftotext 容错）
    );
    let dir = std::env::temp_dir().join(format!("bidparse-garble-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("cjk-marker-english.pdf");
    std::fs::write(&p, pdf.as_bytes()).unwrap();

    let r = bid_parse::parse_file(&p, "cjk-marker-english.pdf");
    match r {
        Ok(pf) => panic!(
            "声明 CJK 字体而双路文本均无 CJK 应显式失败（fail-loud），实际 Ok：{}",
            pf.parse_summary
        ),
        Err(e) => assert!(
            e.contains("乱码") && e.contains("#169"),
            "失败原因要点明乱码防线（实际：{e}）"
        ),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 防误伤正面用例：纯英文 PDF（无 CJK 字体声明）任何环境下都正常解析，
/// 乱码防线永不介入。
#[test]
fn english_pdf_is_never_flagged() {
    let content = b"BT /F1 12 Tf 72 720 Td (Plain English tender document with sufficient text to pass sample gate) Tj ET";
    let pdf = format!(
        "%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
         2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
         3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n\
         4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n\
         5 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n\
         xref\n0 6\ntrailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
        content.len(),
        String::from_utf8_lossy(content),
        0
    );
    let dir = std::env::temp_dir().join(format!("bidparse-english-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("english.pdf");
    std::fs::write(&p, pdf.as_bytes()).unwrap();

    let r = bid_parse::parse_file(&p, "english.pdf");
    // 有 poppler：首选路 Ok（English 文本，NoSignal 不干预）；
    // 无 poppler：兜底解出同一英文（NoSignal 不干预）——两种环境都应 Ok。
    let pf = r.unwrap_or_else(|e| panic!("纯英文 PDF 不应被乱码防线误伤：{e}"));
    assert!(!pf.blocks.is_empty(), "英文 PDF 应解出文本块");
    let _ = std::fs::remove_dir_all(&dir);
}
