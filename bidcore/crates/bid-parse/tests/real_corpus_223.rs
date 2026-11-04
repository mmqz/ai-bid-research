//! #223 局部垃圾拉垮全局判定——真实语料回归面。
//!
//! 事故形态（Lead 实测）：B-cqyd-招标公告.pdf 第 1-4 页正常（汉字
//! 537/906/1161/579、控制符 0），第 5 文本流组 3209 字符全是控制符——
//! 全文拉平后 cjk_ratio=0.0086 <0.02，整份好文件被判「疑似乱码」拒收。
//!
//! 三组断言：
//! 1. B-cqyd 钉兜底路（任何环境可复现事故路）：Ok、关键词在场、分页
//!    多数决 Clean、垃圾流组单独点名、输出零 C1 控制符、summary 不再
//!    整份误标；
//! 2. 三个基线件条款数不回归（935/317/431，#228 后兜底路口径）——修 A
//!    可能让条款数上升（控制符不再打断正则），断言 >= 基线，上升则把新
//!    精确值登进 PR 对照表；
//! 3. 全乱负件（合成）仍 fail-loud——红线不放松（合成面在内联单测）。
//!
//! 口径纪律：935/317/431 是**兜底路**口径（任何环境可复现），不是
//! pdftotext 首选路口径（两者文本内容不同，条款数天然有差）——本文件
//! 全部钉 `parse_via_fallback`，与 `pdf_cid_identity.rs` 同款纪律。

use bid_parse::env_preflight;
use bid_parse::extract::extract_requirements;
use std::path::PathBuf;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../real-corpus")
}

/// B-cqyd：修前被全局 cjk_ratio=0.0086 整份拒收，修后 Clean + 异常页点名。
#[test]
fn b_cqyd_announcement_survives_mixed_garbage_stream() {
    let pdf = corpus_dir().join("telecom/parse-input/B-cqyd-招标公告.pdf");
    assert!(
        pdf.exists(),
        "缺少真件夹具 {}（不能静默通过）",
        pdf.display()
    );
    let raw = std::fs::read(&pdf).expect("读取 B-cqyd");

    // 钉兜底路（#223 事故路：iText 5.5.13 输出在无 poppler 环境走兜底）
    let pf = pdf_text_fallback(&pdf, "B-cqyd-招标公告.pdf")
        .expect("B-cqyd 不再被一页垃圾否决整份（issue #223 主诉求）");

    // 关键内容在场（Lead 实测的正确正文）
    let text = bid_parse::pdf_text::parsed_text_of(&pf);
    assert!(
        text.contains("招标公告"),
        "正文关键词应还原，前 200 字符：{}",
        text.chars().take(200).collect::<String>()
    );

    // 真实件使用 Type0 Identity-H + Adobe/Identity CIDSystemInfo；必须识别为
    // 实际在用的 CJK CID 字体，不能因没有 Adobe-GB1 等显式 CMap 名而退成 NoSignal。
    assert!(
        bid_parse::pdf_text::cid_cjk_font_in_use(&raw),
        "B-cqyd 的 /F1 Type0 Identity-H 字体应被识别为在用 CID-CJK"
    );

    // 分页多数决：多数页正常 → Clean；垃圾流组单独点名
    let pages = bid_parse::pdf_text::parsed_pages_of(&pf);
    let v = bid_parse::pdf_text::probe_cjk_garble_pages(&raw, &pages);
    assert_eq!(
        v.verdict,
        bid_parse::pdf_text::CjkGarbleProbe::Clean,
        "分页多数决应判 Clean（修前整份 Garbled），garbled={:?}",
        v.garbled_pages
    );
    // 注：修 A 后第 5 流组的控制符被行级清洗为空行——该流组不再产出可判
    // 乱的文本（garbled_pages 为空是正确行为）；「异常页单独点名」形态由
    // 内联合成夹具 paged_probe_mixed_pages_majority_clean 覆盖。

    // 修 A：输出零 C1 控制符（修前 157,029 个）
    let c1 = text
        .chars()
        .filter(|c| matches!(c, '\u{80}'..='\u{9F}'))
        .count();
    assert_eq!(c1, 0, "兜底输出不得再有 C1 控制符（修 A），实际 {c1} 个");

    // summary：整份误标消失（修 A 后垃圾流组被清洗为空，无乱码页可点名——
    // summary 回到正常「CJK 占比」面；「Clean + 点名」形态由合成夹具覆盖）
    assert!(
        !pf.parse_summary.contains("【疑似乱码】"),
        "整份误标应消失（页级判定 + 修 A 后）：{}",
        pf.parse_summary
    );
}

/// 基线件条款数不回归（#231/#228 口径：935/317/431，兜底路）。
#[test]
fn baseline_files_requirement_counts_hold() {
    let dir = corpus_dir();
    let cases: [(&str, usize); 3] = [
        ("water/files/A-某水利施工标段招标文件.pdf", 935),
        (
            "telecom/parse-input/A-cqyd-20260500728-重庆西永水冷空调-招标文件.pdf",
            317,
        ),
        ("water/files/A-伊犁水库质量检测招标文件.pdf", 431),
    ];
    for (rel, floor) in cases {
        let p = dir.join(rel);
        if env_preflight::skip_if_fixture_missing("baseline_223", &[&p]) {
            return;
        }
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let pf = bid_parse::pdf_text::parse_via_fallback(&p, &name)
            .unwrap_or_else(|e| panic!("{rel} 兜底路应解析成功: {e}"));
        let reqs = extract_requirements(std::slice::from_ref(&pf));
        assert!(
            reqs.len() >= floor,
            "{rel} 条款数 {} < 基线 {floor}（#223 修复不得掉基线）",
            reqs.len()
        );
    }
}

fn pdf_text_fallback(p: &std::path::Path, name: &str) -> Result<bid_parse::ParsedFile, String> {
    bid_parse::pdf_text::parse_via_fallback(p, name)
}
