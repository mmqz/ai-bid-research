//! #200：CID 字体（UniGB-UCS2-H，无 ToUnicode）恒等解码的整链路双向测试。
//!
//! 背景：真实用户 65 页中文标书被 `E_PARTIAL: pdf 两条路输出均为疑似 CJK 乱码`
//! 拒收——文件本身是好的（pdfplumber 完整提取），根因是纯 Rust 兜底对
//! 「逐字 Tj 单字串」形态（每串仅 2 字节，len<4 过不了 looks_utf16be 门槛）
//! 全部落 Latin-1 分支。修复：按 Tf 字体上下文对声明了 UniXX-UCS2 预定义
//! CMap 的字体做恒等解码（2 字节码 = UCS-2 值）。
//!
//! 双向铁律（issue #200）：
//!   - **好文件不被误杀**：形态 C（逐字 Tj）现在经兜底正确解码，探测 Clean；
//!   - **坏文件仍被拦住**：声明 CJK 字体但码值真不是中文（Latin Extended-A），
//!     恒等解码照走、输出无 CJK → 探测 Garbled → fail-loud 拒收——
//!     #169「宁可失败不产出垃圾」防线零退化。
//!
//! 环境可移植性：测试不预设 pdftotext/pdfium 在位与否——有外部工具时走
//! 首选路，没有时走兜底，两个方向的断言（Ok 含关键词 / Err 含乱码拒收）
//! 在任何环境都成立。

use bid_parse::pdf_text::{self, CjkGarbleProbe};
use std::path::PathBuf;

/// Type0 CID 字体字典（UniGB-UCS2-H 预定义 CMap，无 ToUnicode——#200 形态）。
const UC2_FONT_DICT: &str = "<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light \
     /Encoding /UniGB-UCS2-H /DescendantFonts [6 0 R] >>";

/// 单页 PDF：/F2 → UniGB-UCS2-H 字体；内容流未压缩。
fn uc2_pdf(content: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    v.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    v.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F2 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
    );
    v.extend_from_slice(format!("4 0 obj\n{UC2_FONT_DICT}\nendobj\n").as_bytes());
    v.extend_from_slice(
        b"6 0 obj\n<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light \
          /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> >>\nendobj\n",
    );
    v.extend_from_slice(format!("5 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes());
    v.extend_from_slice(content);
    v.extend_from_slice(b"\nendstream\nendobj\n");
    // xref 桩（pdftotext 容错重建；pdf_text 兜底不依赖 xref）
    v.extend_from_slice(b"xref\n0 7\ntrailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n0\n%%EOF\n");
    v
}

/// Type1 拉丁字体字典（无 UniXX 声明——恒等解码资格必须为 false）。
const LATIN_FONT_DICT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";

/// 双字体页（F1 + F2 各自字体字典，内容流自行决定用哪个）。
fn mixed_font_pdf(f1_dict: &str, f2_dict: &str, content: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    v.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    v.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 7 0 R >>\nendobj\n",
    );
    v.extend_from_slice(format!("4 0 obj\n{f1_dict}\nendobj\n").as_bytes());
    v.extend_from_slice(format!("5 0 obj\n{f2_dict}\nendobj\n").as_bytes());
    v.extend_from_slice(
        b"6 0 obj\n<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light \
          /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> >>\nendobj\n",
    );
    v.extend_from_slice(format!("7 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes());
    v.extend_from_slice(content);
    v.extend_from_slice(b"\nendstream\nendobj\n");
    v.extend_from_slice(b"xref\n0 8\ntrailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n0\n%%EOF\n");
    v
}

/// 「本项目为市政基础设施建设项目，主要建设内」的逐字 Tj 形态（形态 C：
/// 每串 2 字节 + Tm 逐字定位——修复前全落 Latin-1 分支的失败形态）。
fn per_char_tj_content() -> Vec<u8> {
    let line1: &[u16] = &[
        0x672C, 0x9879, 0x76EE, 0x4E3A, 0x5E02, 0x653F, 0x57FA, 0x7840, 0x8BBE, 0x65BD, 0x5EFA,
        0x8BBE, 0x9879, 0x76EE, 0xFF0C, 0x4E3B, 0x8981, 0x5EFA, 0x8BBE, 0x5185,
    ];
    let line2: &[u16] = &[
        0x5BB9, 0x5305, 0x62EC, 0x7BA1, 0x9053, 0x6577, 0x8BBE, 0x3001, 0x9600, 0x95E8, 0x5B89,
        0x88C5, 0x4E0E, 0x6CF5, 0x7AD9, 0x6539, 0x9020, 0x4E09, 0x90E8, 0x5206,
    ];
    let mut content = Vec::new();
    content.extend_from_slice(b"BT /F2 12 Tf\n");
    for (y, line) in [(800u32, line1), (780u32, line2)] {
        let mut x = 50u32;
        for &u in line {
            content.extend_from_slice(format!("1 0 0 1 {x} {y} Tm (").as_bytes());
            content.push((u >> 8) as u8);
            content.push((u & 0xFF) as u8);
            content.extend_from_slice(b") Tj\n");
            x += 12;
        }
    }
    content.extend_from_slice(b"ET\n");
    content
}

fn temp_pdf(bytes: &[u8], tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bidparse-200-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{tag}.pdf"));
    std::fs::write(&p, bytes).unwrap();
    p
}

/// 好文件不被误杀（#200 主断言）：形态 C 经纯 Rust 兜底正确解码、
/// 探测 Clean——修复前这个文件正是用户遇到的 E_PARTIAL 拒收件。
#[test]
fn form_c_cid_file_parses_clean_via_fallback() {
    let pdf = uc2_pdf(&per_char_tj_content());
    let p = temp_pdf(&pdf, "formc");

    // 兜底路单独钉死（不依赖外部工具在位与否——任何环境可复现）
    let parsed = pdf_text::parse_via_fallback(&p, "form-c.pdf")
        .expect("形态 C（逐字 Tj + UniGB-UCS2-H）应经恒等解码恢复");
    let text = pdf_text::parsed_text_of(&parsed);
    assert!(
        text.contains("本项目为市政基础设施建设项目"),
        "恒等解码应还原中文，实际：\n{text}"
    );
    assert!(text.contains("管道敷设"), "第二行同样还原，实际：\n{text}");
    assert!(
        parsed.parse_summary.contains("兜底"),
        "兜底路径必须诚实自报，实际：{}",
        parsed.parse_summary
    );
    // 文本量 40 字 > 32 门槛：探测应给 Clean（修复前同一文件为 Garbled）
    assert_eq!(
        pdf_text::probe_cjk_garble(&pdf, &text),
        CjkGarbleProbe::Clean,
        "正确解码的中文不得再被判乱码（#200 误杀）"
    );

    // 整链路（有 poppler 走首选路 / 无则走兜底——两环境都应 Ok 且含关键词）
    let parsed_full = bid_parse::parse_file(&p, "form-c.pdf")
        .expect("好文件在任何环境都不得被拒收（#200 主诉求）");
    let text_full = pdf_text::parsed_text_of(&parsed_full);
    assert!(
        text_full.contains("本项目为市政基础设施建设项目"),
        "整链路输出应含中文正文，summary：{}",
        parsed_full.parse_summary
    );
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

/// #200 R1 fail-loud 面钉死（确定性，任何环境同判）：CID 字体在用、码值为
/// Latin Extended-A（合法 Unicode、零 CJK、非 ASCII 占比 100%）——
/// **无 xref 桩**让 pdftotext 必败（缺 trailer），首选路不可用是确定性的；
/// 兜底单路输出高残渣（≥0.3）→ 无第二路互证 → fail-loud 拒收。
/// 对照：mixed_low_cjk_ratio_page_not_misjudged 覆盖「低残渣单路采信」与
/// 「双路互证采信」两个 Ok 面——好/坏两个方向都有测试钉住。
#[test]
fn fallback_only_garbled_high_residue_fails_loud() {
    let mut content = Vec::new();
    content.extend_from_slice(b"BT /F2 12 Tf 72 720 Td (");
    for i in 0..40u16 {
        let u = 0x0122 + (i % 20);
        content.push((u >> 8) as u8);
        content.push((u & 0xFF) as u8);
    }
    content.extend_from_slice(b") Tj ET\n");
    let mut pdf = uc2_pdf(&content);
    // 去掉 xref 桩（截到 endobj 后即止）——pdftotext 缺 trailer 必败，
    // pdf_text 兜底不依赖 xref 照常提取（两侧能力差是确定性的）
    let cut = pdf
        .windows(7)
        .rposition(|w| w == b"endobj\n")
        .map(|i| i + 7)
        .unwrap();
    pdf.truncate(cut);
    let p = temp_pdf(&pdf, "latext");

    let err = bid_parse::parse_file(&p, "latext.pdf")
        .expect_err("单路高残渣（非 ASCII 占比 1.0）必须 fail-loud");
    assert!(
        err.contains("[pdf-fallback-only-garbled]"),
        "失败必须带稳定标签（实际：{err}）"
    );
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

// ---------------------------------------------------------------- #200 剩余能力边界（原因定位面）

/// hint 纯函数：非 UCS2 CMap / ObjStm / CID 注册序号 / 无标记四态。
#[test]
fn reason_hint_names_boundary_families() {
    // GBK-EUC-H 声明 → 点名该族 + 给出绕过
    let h = pdf_text::cjk_garble_reason_hint(b"/Encoding /GBK-EUC-H").expect("GBK-EUC 必须点名");
    assert!(h.contains("GBK-EUC-H/V"), "族名点名：{h}");
    assert!(h.contains("非 Unicode 码位"), "根因说明：{h}");
    assert!(h.contains("绕过"), "绕过指引：{h}");

    // ObjStm → 点名对象流边界
    let h =
        pdf_text::cjk_garble_reason_hint(b"5 0 obj << /Type /ObjStm >>").expect("ObjStm 必须点名");
    assert!(h.contains("/ObjStm"), "对象流点名：{h}");
    assert!(h.contains("pdfium"), "绕过首选路：{h}");

    // 双边界并存 → 两段都在（疑因线索全列）
    let both = b"/Encoding /GBK-EUC-H /Ordering (GB1) + /ObjStm";
    let h = pdf_text::cjk_garble_reason_hint(both).expect("双边界必须点名");
    assert!(
        h.contains("GBK-EUC-H/V") && h.contains("/ObjStm"),
        "两族并列：{h}"
    );

    // B5pc 繁体族
    let h = pdf_text::cjk_garble_reason_hint(b"/Encoding /B5pc-H").expect("B5pc 必须点名");
    assert!(h.contains("B5pc-H/V"), "繁体族点名：{h}");

    // 无任何边界标记 → None（泛化乱码保持原报错文案）
    assert!(pdf_text::cjk_garble_reason_hint(b"%PDF-1.4 plain english").is_none());
}

/// GBK-EUC-H 整链（真 GBK 码值）：环境可移植双态断言——
/// 有完整 CMap 资源的 poppler → 首选路 Ok 解出中文；否则 fail-loud 拒收
/// 且错误信息**点名 GBK-EUC 边界 + 绕过路径**（#200 验收：遇到边界文件
/// 时用户能看到「这是什么原因、有没有办法绕过」）。
#[test]
fn gbk_euc_h_pdf_rejection_names_reason_and_workaround() {
    // 36 字句子的 GBK 码值（逐字 Tj——每串 2 字节；≥32 非空白字符，
    // 过 CJK 探测的样本门槛——短文本 NoSignal 是 #169 防线既有的
    // 短样本权衡，这里测的是完整防线路径）
    let gbk_words: [(&str, [u8; 2]); 36] = [
        ("市", [0xCA, 0xD0]),
        ("政", [0xD5, 0xFE]),
        ("基", [0xBB, 0xF9]),
        ("础", [0xB4, 0xA1]),
        ("设", [0xC9, 0xE8]),
        ("施", [0xCA, 0xA9]),
        ("建", [0xBD, 0xA8]),
        ("设", [0xC9, 0xE8]),
        ("项", [0xCF, 0xEE]),
        ("目", [0xC4, 0xBF]),
        ("主", [0xD6, 0xF7]),
        ("要", [0xD2, 0xAA]),
        ("建", [0xBD, 0xA8]),
        ("设", [0xC9, 0xE8]),
        ("内", [0xC4, 0xDA]),
        ("容", [0xC8, 0xDD]),
        ("包", [0xB0, 0xFC]),
        ("括", [0xC0, 0xA8]),
        ("道", [0xB5, 0xC0]),
        ("路", [0xC2, 0xB7]),
        ("敷", [0xB7, 0xF3]),
        ("设", [0xC9, 0xE8]),
        ("闸", [0xD5, 0xA2]),
        ("门", [0xC3, 0xC5]),
        ("安", [0xB0, 0xB2]),
        ("装", [0xD7, 0xB0]),
        ("与", [0xD3, 0xEB]),
        ("泵", [0xB1, 0xC3]),
        ("站", [0xD5, 0xBE]),
        ("改", [0xB8, 0xC4]),
        ("造", [0xD4, 0xEC]),
        ("三", [0xC8, 0xFD]),
        ("部", [0xB2, 0xBF]),
        ("分", [0xB7, 0xD6]),
        ("工", [0xB9, 0xA4]),
        ("程", [0xB3, 0xCC]),
    ];
    let mut content = Vec::new();
    content.extend_from_slice(b"BT /F2 12 Tf\n");
    let mut x = 20u32;
    let mut y = 800u32;
    for (i, (_, gbk)) in gbk_words.iter().enumerate() {
        if i > 0 && i % 18 == 0 {
            y -= 20; // 两行排版（18 字/行）
            x = 20;
        }
        content.extend_from_slice(format!("1 0 0 1 {x} {y} Tm (").as_bytes());
        content.extend_from_slice(gbk);
        content.extend_from_slice(b") Tj\n");
        x += 14;
    }
    content.extend_from_slice(b"ET\n");

    // GBK-EUC-H 字体字典：在 UC2 夹具的字体字典上替换编码声明
    // （/Encoding /UniGB-UCS2-H → /GBK-EUC-H——长度一致，xref 偏移不漂移）
    let pdf = uc2_pdf(&content);
    let from = b"/Encoding /UniGB-UCS2-H";
    let to = b"/Encoding /GBK-EUC-H  ";
    let mut out = Vec::with_capacity(pdf.len());
    let mut i = 0usize;
    while i < pdf.len() {
        if pdf[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(pdf[i]);
            i += 1;
        }
    }
    assert!(
        out.windows(9).any(|w| w == b"GBK-EUC-H"),
        "夹具必须声明 GBK-EUC-H"
    );
    let p = temp_pdf(&out, "gbk-euc");

    match bid_parse::parse_file(&p, "gbk.pdf") {
        Ok(pf) => {
            // 环境有带 CMap 资源包的外部工具——首选路正确解码
            let text = pdf_text::parsed_text_of(&pf);
            assert!(text.contains("市"), "完整环境下应解出中文：{text:?}");
        }
        Err(err) => {
            // 无外部工具 / 缺 CMap——fail-loud 拒收且点名原因与绕过
            assert!(
                err.contains("GBK-EUC"),
                "错误信息必须点名 GBK-EUC 边界（#200 验收），实际：{err}"
            );
            assert!(
                err.contains("绕过"),
                "错误信息必须给出绕过路径（#200 验收），实际：{err}"
            );
            assert!(
                err.contains("疑似 CJK 乱码"),
                "fail-loud 主防线文案不退化，实际：{err}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

/// #200 R1 边界①：多字体页——F2 是 CID-CJK 字体但**只声明未使用**（内容
/// 全走 F1 拉丁字体）→ 探测不激活（判「未用」的三前提齐备：流解出、找到
/// Tf、Tf 选用集无标记字体）→ 任何环境都正常解析。
#[test]
fn declared_but_unused_cid_font_not_misjudged() {
    // 内容只用 F1；F2（UniGB-UCS2-H）在资源表里声明但从无 Tf 选用
    let content = b"BT /F1 12 Tf 72 720 Td (Tender summary in English with enough words to be a real page) Tj ET";
    let pdf = mixed_font_pdf(LATIN_FONT_DICT, UC2_FONT_DICT, content);
    let p = temp_pdf(&pdf, "unused-cid");
    let parsed = bid_parse::parse_file(&p, "unused-cid.pdf")
        .expect("#200 R1：声明未使用的 CID 字体不得触发乱码判定");
    let text = pdf_text::parsed_text_of(&parsed);
    assert!(
        text.contains("Tender summary"),
        "英文内容原样（实际：{text}）"
    );
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

/// #200 R1 边界②：中英混排低 CJK 占比页（1 个汉字 + 大段英文，占比
/// ≈0.017 < 0.02 阈值）→ 不得因比例误杀：
/// · 有首选路：主路与兜底输出一致 → 互证采信；
/// · 无首选路：非 ASCII 占比 ≈0.017 < 0.3 → 残渣不成立 → 单路采信。
#[test]
fn mixed_low_cjk_ratio_page_not_misjudged() {
    // 本 + 58 个英文单词的整串 Tj（两路引擎输出同一字符序列）
    // UniGB-UCS2-H 的串必须是 UCS-2 编码：本=0x672C，ASCII 'T'=0x0054……
    // （单字节 ASCII 直写属畸形件——恒等解码会把 ASCII 对拼成汉字，边界登记）
    let mixed = "\u{672c} Tender document summary covering scope and period, all technical requirements listed here for the record and review";
    let mut content = Vec::new();
    content.extend_from_slice(b"BT /F2 12 Tf 72 720 Td (");
    let units: Vec<u8> = mixed.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    content.extend_from_slice(&units);
    content.extend_from_slice(b") Tj ET\n");
    let pdf = uc2_pdf(&content);
    let p = temp_pdf(&pdf, "mixed");
    let parsed = bid_parse::parse_file(&p, "mixed.pdf")
        .expect("#200 R1：混排低占比页不得因 CJK 比例误杀（任何环境）");
    let text = pdf_text::parsed_text_of(&parsed);
    assert!(
        text.contains('本') && text.contains("Tender"),
        "中英都应保留（实际：{text}）"
    );
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

/// #200 R1 判据单元面：互证归一化 + 残渣占比 + CID 在用探测三件套。
#[test]
fn r1_verdict_primitives() {
    // 互证：同内容不同空白/标点 → 互证；分叉乱码 → 不互证
    assert!(pdf_text::texts_corroborate(
        "This is a tender, summary!",
        "This is a tender summary"
    ));
    assert!(!pdf_text::texts_corroborate("SbheNLRQ", "g,yv\u{ee}N"));

    // 残渣占比：纯英文 0；GBK 被逐字节 Latin-1 化 ≈ 1.0
    assert!(pdf_text::non_ascii_ratio("Plain English text only") < 0.01);
    let gbk_residue: String = [0xC4u16, 0xB3, 0xCA, 0xD0]
        .iter()
        .map(|&b| char::from_u32(b as u32).unwrap())
        .collect();
    assert!(pdf_text::non_ascii_ratio(&gbk_residue) >= 0.99);

    // CID 在用探测四态
    let english = b"BT /F1 12 Tf 72 720 Td (hello) Tj ET";
    // ① Type1-SimSun：无 CMap 标记 → 不激活
    let t1_simsum = mixed_font_pdf(
        "<< /Type /Font /Subtype /Type1 /BaseFont /SimSun >>",
        UC2_FONT_DICT,
        english,
    );
    assert!(
        !pdf_text::cid_cjk_font_in_use(&t1_simsum),
        "Type1-SimSun 不激活"
    );
    // ② CID 声明未使用 → 不激活（内容只用 F1）
    let unused = mixed_font_pdf(LATIN_FONT_DICT, UC2_FONT_DICT, english);
    assert!(!pdf_text::cid_cjk_font_in_use(&unused), "声明未使用不激活");
    // ③ CID 实际在用 → 激活
    let used = mixed_font_pdf(
        LATIN_FONT_DICT,
        UC2_FONT_DICT,
        b"BT /F2 12 Tf 72 720 Td (x) Tj ET",
    );
    assert!(
        pdf_text::cid_cjk_font_in_use(&used),
        "Tf 选用 CID 字体应激活"
    );
    // ④ 内容流解不开但标记在 → 保守激活（无法排除）
    let broken = uc2_pdf(b"broken not a stream"); // 无 stream 结构 → any_stream=false
    assert!(pdf_text::cid_cjk_font_in_use(&broken), "流解不开时保守激活");
}
