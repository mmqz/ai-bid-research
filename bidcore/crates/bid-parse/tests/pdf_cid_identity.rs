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
    v.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");
    v
}

/// 「本项目为市政基础设施建设项目，主要建设内」的逐字 Tj 形态（形态 C：
/// 每串 2 字节 + Tm 逐字定位——修复前全落 Latin-1 分支的失败形态）。
fn per_char_tj_content() -> Vec<u8> {
    let line1: &[u16] = &[
        0x672C, 0x9879, 0x76EE, 0x4E3A, 0x5E02, 0x653F, 0x57FA, 0x7840, 0x8BBE, 0x65BD,
        0x5EFA, 0x8BBE, 0x9879, 0x76EE, 0xFF0C, 0x4E3B, 0x8981, 0x5EFA, 0x8BBE, 0x5185,
    ];
    let line2: &[u16] = &[
        0x5BB9, 0x5305, 0x62EC, 0x7BA1, 0x9053, 0x6577, 0x8BBE, 0x3001, 0x9600, 0x95E8,
        0x5B89, 0x88C5, 0x4E0E, 0x6CF5, 0x7AD9, 0x6539, 0x9020, 0x4E09, 0x90E8, 0x5206,
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
    assert!(text.contains("本项目为市政基础设施建设项目"), "恒等解码应还原中文，实际：\n{text}");
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

/// 坏文件仍被拦住（#169 防线零退化）：同一 UniGB-UCS2 字体声明，但码值
/// 是 Latin Extended-A（合法 Unicode、零 CJK）——恒等解码照走、输出无
/// CJK → 探测 Garbled → fail-loud 拒收。任何环境（有无 poppler）同判。
#[test]
fn garbled_cid_text_still_fails_loud() {
    // 0x0122 起的 Latin Extended-A 码位 ×40：恒等解码输出「ĢĢĳ...)」类
    // 零 CJK 文本——这就是「真解码失败」形态（对照：好文件解码出中文）。
    let mut content = Vec::new();
    content.extend_from_slice(b"BT /F2 12 Tf 72 720 Td (");
    for i in 0..40u16 {
        let u = 0x0122 + (i % 20);
        content.push((u >> 8) as u8);
        content.push((u & 0xFF) as u8);
    }
    content.extend_from_slice(b") Tj ET\n");
    let pdf = uc2_pdf(&content);
    let p = temp_pdf(&pdf, "garbled");

    let err = bid_parse::parse_file(&p, "garbled.pdf")
        .expect_err("声明 CJK 字体但输出零 CJK 的文件必须 fail-loud 拒收");
    assert!(
        err.contains("疑似 CJK 乱码"),
        "拒收原因必须指明 CJK 乱码防线，实际：{err}"
    );
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}
