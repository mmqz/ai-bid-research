//! env_preflight_pdfium.rs —— L1：PDF 引擎三态探测（pdfium + poppler）。
//!
//! 背景（Lead 点名）：只探 poppler 不探 pdfium，报错「PDF 链缺失」有误导性。
//! 真相：本仓 pdf 文本层首选路是 pdfium（bid-engine reader.rs 直连）；
//! 「pdfium 在位、poppler 不在」的机器上文本层解析正常，却被误报缺件。
//!
//! 验收线：三态可区分 + 报错信息准确。

use bid_parse::env_preflight::{
    missing_tools, pdfium_available, pdfium_available_impl, pdfium_library_candidates,
    pdfium_library_filename, PdfEnvStatus, PDFIUM_LIB_ENV, POPPLER_TEXT_TOOLS,
};

// ---------------------------------------------------------------- 三态判定

#[test]
fn three_states_are_distinguishable() {
    let all = [PdfEnvStatus::Full, PdfEnvStatus::PdfiumOnly, PdfEnvStatus::None];
    for (i, a) in all.iter().enumerate() {
        for (j, b) in all.iter().enumerate() {
            assert_eq!(i == j, a == b, "{a:?} 与 {b:?} 的相等性判定错");
        }
    }
}

#[test]
fn capabilities_match_the_three_states() {
    assert!(PdfEnvStatus::Full.can_parse_text_pdf());
    assert!(PdfEnvStatus::PdfiumOnly.can_parse_text_pdf(), "pdfium 在位即能解析文本层 PDF");
    assert!(!PdfEnvStatus::None.can_parse_text_pdf());
    assert!(PdfEnvStatus::Full.can_ocr());
    assert!(!PdfEnvStatus::PdfiumOnly.can_ocr());
    assert!(!PdfEnvStatus::None.can_ocr());
}

#[test]
fn machine_codes_are_stable() {
    assert_eq!(PdfEnvStatus::Full.as_str(), "FULL");
    assert_eq!(PdfEnvStatus::PdfiumOnly.as_str(), "PDFIUM_ONLY");
    assert_eq!(PdfEnvStatus::None.as_str(), "NONE");
}

// ---------------------------------------------------------------- 报错信息准确

#[test]
fn pdfium_only_message_says_text_layer_still_works() {
    let msg = PdfEnvStatus::PdfiumOnly.to_message();
    assert!(msg.contains("文本层"), "PdfiumOnly 必须说明文本层可用：{msg}");
    assert!(msg.contains("pdfium"), "要点名 pdfium：{msg}");
    assert!(msg.contains("扫描件"), "影响面应限于扫描件：{msg}");
}

#[test]
fn none_message_names_the_real_consequence() {
    let msg = PdfEnvStatus::None.to_message();
    assert!(msg.contains("pdfium") && msg.contains("poppler"), "None 要说清两边都缺：{msg}");
    assert!(msg.contains("无法解析"), "要说清后果：{msg}");
}

#[test]
fn full_message_has_no_warning_tone() {
    let msg = PdfEnvStatus::Full.to_message();
    assert!(msg.contains("全链可用"), "Full 应正面陈述：{msg}");
}

// ---------------------------------------------------------------- pdfium 探测面

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bid-l1-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn pdfium_detected_from_explicit_env_path() {
    let dir = tmp_dir("env");
    let fake = dir.join(pdfium_library_filename());
    std::fs::write(&fake, b"not-a-real-so").unwrap();
    assert!(
        pdfium_available_impl(Some(fake.to_str().unwrap()), None),
        "env 指向存在的文件 → 探测面认 pdfium"
    );
    let ghost = dir.join("nope.dll");
    assert!(!pdfium_available_impl(Some(ghost.to_str().unwrap()), None));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pdfium_absent_when_nothing_is_installed() {
    let empty = tmp_dir("empty");
    let candidates_have = pdfium_library_candidates()
        .iter()
        .any(|d| d.join(pdfium_library_filename()).is_file());
    if candidates_have {
        eprintln!("SKIP: 候选目录里已有 pdfium，跳过「全缺」用例");
        return;
    }
    assert!(
        !pdfium_available_impl(None, Some(empty.to_str().unwrap())),
        "空 PATH + 无 env + 候选目录无库 → pdfium 判缺"
    );
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn pdfium_detected_from_path_entry() {
    let dir = tmp_dir("path");
    std::fs::write(dir.join(pdfium_library_filename()), b"x").unwrap();
    let candidates_have = pdfium_library_candidates()
        .iter()
        .any(|d| d.join(pdfium_library_filename()).is_file());
    if !candidates_have {
        assert!(
            pdfium_available_impl(None, Some(dir.to_str().unwrap())),
            "PATH 里有 pdfium 库 → 探测面认"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 常量一致性

#[test]
fn pdfium_env_var_matches_mdcore_literal() {
    // bid-parse 不依赖 mdc-pdf（依赖方向 bid-engine → mdc-pdf），字面量复制
    // 一份——此处锚定，改动时两处必须同步（mdcore engine_pdfium.rs:73）
    assert_eq!(PDFIUM_LIB_ENV, "MDC_PDFIUM_LIB");
}

#[test]
fn poppler_text_tools_exclude_ocr_binary() {
    assert!(!POPPLER_TEXT_TOOLS.contains(&"tesseract"), "文本层判定不该被 tesseract 拖累");
    assert!(POPPLER_TEXT_TOOLS.contains(&"pdftotext"));
}

// ---------------------------------------------------------------- 组合一致性

#[test]
fn detect_matches_the_two_boolean_combination() {
    let actual = bid_parse::env_preflight::detect_pdf_env_status();
    let poppler_ok = missing_tools(POPPLER_TEXT_TOOLS).is_empty();
    let pdfium_ok = pdfium_available();
    let expected = match (pdfium_ok, poppler_ok) {
        (true, true) => PdfEnvStatus::Full,
        (true, false) => PdfEnvStatus::PdfiumOnly,
        (false, _) => PdfEnvStatus::None,
    };
    assert_eq!(actual, expected, "detect 结果必须与两布尔组合一致");
    assert_eq!(bid_parse::env_preflight::pdf_env_status(poppler_ok), expected);
}