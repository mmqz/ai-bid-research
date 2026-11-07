//! #424 · 混合 PDF 逐页路由验收（R-26 / B-28）。
//!
//! 缺陷：`parse_pdf` 用**全件非空白字符总和**判路由——混合件（文本页 +
//! 盖章/资质扫描页）整件走文本层，扫描页 0 块 0 文本、内容静默丢失
//! （盖章页上的资质/人员条款全部丢失，无告警）。
//!
//! 修复面：
//! - `bid-parse` 仲裁器逐页判定：0 字符页 = 扫描页（OCR 子集识别），
//!   ≥1 字符稀疏真文本页保持文本层归属（零回归）；
//! - `bid-engine` pdfium 映射门：任一页真空 → MappingGap 落标书侧
//!   （import 主路径 pdfium 优先，不改门混合件仍静默丢页）。
//!
//! 夹具纪律：合成 PDF（python3 + reportlab + PIL——文本页 drawString、
//! 扫描页 drawImage 图像）。环境缺件走 SKIP 门（AGENTS.md §13.1——
//! 横幅 + 原因 + 提前 return，非静默）。

use bid_parse::env_preflight::{
    missing_tools, skip_if_env_missing, FULL_PDF_CHAIN, POPPLER_TEXT_TOOLS,
};
use bid_parse::parse_file;
use std::path::PathBuf;
use std::sync::Mutex;

/// 生成器互斥（python3 生成 + parse 都占子进程并发席——#200 R2 竞态治本）。
static GEN_LOCK: Mutex<()> = Mutex::new(());

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bidparse_mixed_{tag}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 生成器可用性（python3 + reportlab + PIL——缺任一则本文件 e2e 用例 SKIP）。
fn generator_ready(test_id: &str) -> bool {
    let script = r#"
import sys
try:
    from reportlab.pdfgen import canvas
    from reportlab.lib.utils import ImageReader
    from PIL import Image, ImageDraw
except ImportError as e:
    print(f"MISSING:{e}")
    sys.exit(0)
print("OK")
"#;
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .output();
    match out {
        Ok(o) if String::from_utf8_lossy(&o.stdout).trim() == "OK" => true,
        _ => {
            eprintln!(
                "SKIP: {test_id} —— 环境缺 python3+reportlab+PIL（合成夹具生成器缺件，未执行）"
            );
            false
        }
    }
}

/// 合成混合 PDF：page1 文本层（中文条款），page2 仅图像（扫描页形态），
/// page3 文本层（验证 OCR 页夹在中间时的页序合并）。
fn build_mixed_pdf(path: &std::path::Path) {
    let script = r#"
import sys
from reportlab.pdfgen import canvas
from reportlab.lib.utils import ImageReader
from PIL import Image, ImageDraw

out_pdf = sys.argv[1]
# 扫描页图像：白底黑字大号（OCR 可读形态——内容为英文大字，chi_sim 也能识别拉丁字符）
img = Image.new("RGB", (840, 1188), "white")
d = ImageDraw.Draw(img)
d.text((80, 500), "STAMPED QUALIFICATION PAGE 42", fill="black")
d.rectangle((70, 480, 770, 560), outline="black", width=3)
img_path = out_pdf + ".png"
img.save(img_path)

c = canvas.Canvas(out_pdf)
# page 1: 文本层
c.setFont("Helvetica", 12)
c.drawString(72, 750, "1.1 Tenderer must hold Grade-II or higher electronic contract qualification.")
c.drawString(72, 720, "1.2 The bid document shall respond to all technical requirements item by item.")
c.showPage()
# page 2: 扫描页（仅图像，无文本层）
c.drawImage(ImageReader(img_path), 0, 0, width=595, height=842)
c.showPage()
# page 3: 文本层
c.setFont("Helvetica", 12)
c.drawString(72, 750, "3.1 The bid bond shall be ninety thousand CNY.")
c.showPage()
c.save()
print("MIXED_OK")
"#;
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(path)
        .output()
        .expect("python3 生成混合 PDF");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("MIXED_OK"),
        "生成失败: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// 合成纯文本 PDF（2 页，每页都有文本——尾随 \\f 空段判伪的回归载体）。
fn build_text_pdf(path: &std::path::Path) {
    let script = r#"
import sys
from reportlab.pdfgen import canvas
c = canvas.Canvas(sys.argv[1])
for i in range(2):
    c.setFont("Helvetica", 12)
    c.drawString(72, 750, f"Page {i+1} tender clause with plenty of real text content here.")
    c.showPage()
c.save()
print("TEXT_OK")
"#;
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(path)
        .output()
        .expect("python3 生成纯文本 PDF");
    assert!(String::from_utf8_lossy(&out.stdout).contains("TEXT_OK"));
}

/// 合成纯扫描 PDF（1 页，仅图像）。
fn build_scan_pdf(path: &std::path::Path) {
    let script = r#"
import sys
from reportlab.pdfgen import canvas
from reportlab.lib.utils import ImageReader
from PIL import Image, ImageDraw
img = Image.new("RGB", (840, 1188), "white")
d = ImageDraw.Draw(img)
d.text((80, 500), "SCANNED COVER SHEET ONLY", fill="black")
p = sys.argv[1]
img.save(p + ".png")
c = canvas.Canvas(p)
c.drawImage(ImageReader(p + ".png"), 0, 0, width=595, height=842)
c.showPage()
c.save()
print("SCAN_OK")
"#;
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(path)
        .output()
        .expect("python3 生成纯扫描 PDF");
    assert!(String::from_utf8_lossy(&out.stdout).contains("SCAN_OK"));
}

// ---------------------------------------------------------------- 单元面（无环境依赖）

/// 混合分面纯函数：判据与零回归边界（engines/mod.rs `mixed_partition` 的
/// 镜像断言——pub 面经 `engines::mixed_partition_for_test` 暴露）。
#[test]
fn mixed_partition_predicate_boundaries() {
    use bid_parse::engines::mixed_partition_for_test;
    let pt = |page: usize, text: &str| bid_parse::engine::PageText {
        page,
        text: text.to_string(),
        confidence: 1.0,
        diagnostics: String::new(),
    };
    // 纯文本（含稀疏页 1-15 字符）：非混合——稀疏页保持文本层归属
    let pages = vec![pt(1, "整整十六个字符以上的真实文本页内容在此处"), pt(2, "第 三 章")];
    assert!(mixed_partition_for_test(&pages, Some(2)).is_none(), "稀疏真文本页不得判扫描页");
    // 纯扫描（全 0 字符）：非混合——走旧求和门（sum<16 → 整档 OCR）
    let pages = vec![pt(1, ""), pt(2, " \n ")];
    assert!(mixed_partition_for_test(&pages, Some(2)).is_none());
    // 混合：实文本页 + 0 字符页
    let pages = vec![pt(1, "整整十六个字符以上的真实文本页内容在此处"), pt(2, ""), pt(3, "3.1 文本")];
    let (text_pages, scan) = mixed_partition_for_test(&pages, Some(3)).unwrap();
    assert_eq!(scan, vec![2], "0 字符页判扫描页");
    assert_eq!(text_pages.len(), 2, "≥1 字符页全部保持文本层（含稀疏页）");
    // 尾随 \f 空段判伪：分段数 > 真实页数 → 剔除尾部空段，非混合
    let pages = vec![pt(1, "整整十六个字符以上的真实文本页内容在此处"), pt(2, "")];
    assert!(mixed_partition_for_test(&pages, Some(1)).is_none(), "尾随空段是 pdftotext 伪段");
    // pdfinfo 探测失败（None）：保守不剔——空段计为扫描页（越界 OCR 无产物，无内容丢失）
    let (_, scan) = mixed_partition_for_test(&pages, None).unwrap();
    assert_eq!(scan, vec![2]);
    // 无实文本页的稀疏混合（1-15 字符 + 0 字符，sum<16）：非混合——旧门判整档 OCR
    let pages = vec![pt(1, "十个字符的页"), pt(2, "")];
    assert!(mixed_partition_for_test(&pages, Some(2)).is_none());
}

// ---------------------------------------------------------------- e2e 面（环境门）

/// 验收 1：混合夹具（文本页 + 无文本层图像页 + 文本页）→ 三页内容都在
/// blocks 里；页序统一；summary 带混合路由诊断（页级路由计数）。
#[test]
fn mixed_pdf_routes_scan_pages_to_ocr() {
    let test_id = "mixed_pdf_routes_scan_pages_to_ocr";
    if skip_if_env_missing(test_id, FULL_PDF_CHAIN) || !generator_ready(test_id) {
        return;
    }
    let _g = GEN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tmp_dir("e2e");
    let pdf = dir.join("mixed.pdf");
    build_mixed_pdf(&pdf);

    let pf = parse_file(&pdf, "mixed.pdf").expect("混合件解析必须成功");

    // 文本页内容在场（page1 / page3）
    let text_all = pf
        .blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<String>();
    assert!(text_all.contains("Grade-II"), "page1 文本层内容必须在场：{text_all}");
    assert!(text_all.contains("ninety thousand"), "page3 文本层内容必须在场：{text_all}");
    // 扫描页内容在场（page2 OCR——chi_sim 对拉丁大字可识别；不 assert 逐字，
    // 只 assert 页 2 产出了 OCR 行块且含实质字符）
    let page2_ocr: Vec<&bid_parse::Block> = pf
        .blocks
        .iter()
        .filter(|b| b.locator.starts_with("第2页 OCR"))
        .collect();
    assert!(
        !page2_ocr.is_empty(),
        "扫描页（第2页）必须产出 OCR 行块——修复前整件文本层 0 块静默丢失：{}",
        pf.parse_summary
    );
    assert!(pf.ocr_blocks > 0, "ocr_blocks 计数必须登记：{}", pf.parse_summary);
    assert_eq!(pf.has_text_layer, true);
    assert_eq!(pf.pages, 3, "页数 = 文本页 + 扫描页诚实总数");
    // 页级路由计数进诊断面
    assert!(
        pf.parse_summary.contains("混合路由"),
        "summary 必须带混合路由诊断：{}",
        pf.parse_summary
    );
    assert!(
        pf.parse_summary.contains("扫描页 [2]"),
        "扫描页号清单必须在场：{}",
        pf.parse_summary
    );
    // 页序统一：blocks 按页号递增（locator 第N页）
    let pages_seq: Vec<usize> = pf
        .blocks
        .iter()
        .filter_map(|b| {
            b.locator
                .strip_prefix("第")
                .and_then(|r| r.split_once('页'))
                .and_then(|(n, _)| n.parse().ok())
        })
        .collect();
    let mut sorted = pages_seq.clone();
    sorted.sort();
    assert_eq!(pages_seq, sorted, "blocks 必须按页序排列（交错页合并）");
    // idx 连续重编
    for (i, b) in pf.blocks.iter().enumerate() {
        assert_eq!(b.idx, i, "idx 必须连续重编");
    }
}

/// 验收 3（纯文本件零回归）：每页有文本的 PDF 走旧文本层路径——
/// summary 无混合路由标记、无 OCR 块（尾随 \\f 空段被判伪，不误入混合面）。
#[test]
fn pure_text_pdf_unchanged_text_layer_path() {
    let test_id = "pure_text_pdf_unchanged_text_layer_path";
    if skip_if_env_missing(test_id, POPPLER_TEXT_TOOLS) || !generator_ready(test_id) {
        return;
    }
    let _g = GEN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tmp_dir("pure-text");
    let pdf = dir.join("pure-text.pdf");
    build_text_pdf(&pdf);

    let pf = parse_file(&pdf, "pure-text.pdf").expect("纯文本件解析必须成功");
    assert_eq!(pf.ocr_blocks, 0, "纯文本件不得触发 OCR：{}", pf.parse_summary);
    assert!(pf.has_text_layer);
    assert!(
        !pf.parse_summary.contains("混合路由"),
        "纯文本件（尾随空段判伪后）不得进混合路由：{}",
        pf.parse_summary
    );
    assert!(pf.parse_summary.contains("pdf 文本层"), "走旧文本层路径：{}", pf.parse_summary);
    assert!(
        pf.blocks.iter().all(|b| !b.locator.contains(" OCR ")),
        "不得出现 OCR 行块"
    );
}

/// 验收 3（纯扫描件零回归）：无文本层 PDF 走旧整档 OCR 路径。
#[test]
fn pure_scan_pdf_unchanged_whole_file_ocr_path() {
    let test_id = "pure_scan_pdf_unchanged_whole_file_ocr_path";
    if skip_if_env_missing(test_id, FULL_PDF_CHAIN) || !generator_ready(test_id) {
        return;
    }
    let _g = GEN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tmp_dir("pure-scan");
    let pdf = dir.join("pure-scan.pdf");
    build_scan_pdf(&pdf);

    let pf = parse_file(&pdf, "pure-scan.pdf").expect("纯扫描件解析必须成功");
    assert_eq!(pf.has_text_layer, false, "纯扫描件无文本层");
    assert!(
        !pf.parse_summary.contains("混合路由"),
        "纯扫描件走旧整档 OCR 路径：{}",
        pf.parse_summary
    );
    assert!(
        pf.parse_summary.contains("扫描层 OCR") || pf.parse_summary.contains("OCR"),
        "整档 OCR summary：{}",
        pf.parse_summary
    );
}

/// 验收 2（OCR 缺件环境——反向门）：无 chi_sim 时混合件**显式报扫描页缺件**
/// ——文本页内容保留 + summary 登记扫描页清单与补救指引（不静默丢页）。
/// 与 `mixed_pdf_routes_scan_pages_to_ocr` 互为环境互补门：任一环境恰好
/// 执行其一（chi_sim 在位 → 正路验收；缺件 → 跳过面验收）。
#[test]
fn mixed_pdf_without_ocr_lang_registers_explicit_skip() {
    let test_id = "mixed_pdf_without_ocr_lang_registers_explicit_skip";
    if skip_if_env_missing(test_id, POPPLER_TEXT_TOOLS) || !generator_ready(test_id) {
        return;
    }
    if missing_tools(&[bid_parse::env_preflight::OCR_LANG_CHI_SIM]).is_empty() {
        eprintln!(
            "SKIP: {test_id} —— 本环境 chi_sim 在位（正路由已由 mixed_pdf_routes_scan_pages_to_ocr 验收；本用例专测缺件跳过面，需无 chi_sim 环境执行）"
        );
        return;
    }
    let _g = GEN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tmp_dir("no-ocr");
    let pdf = dir.join("mixed-noocr.pdf");
    build_mixed_pdf(&pdf);

    let pf = parse_file(&pdf, "mixed-noocr.pdf").expect("OCR 缺件时混合件不得整件失败");
    // 文本页内容保留（不陪葬）
    let text_all = pf.blocks.iter().map(|b| b.text.as_str()).collect::<String>();
    assert!(text_all.contains("Grade-II"), "文本页内容必须保留：{text_all}");
    // 扫描页显式登记（不静默）
    assert!(
        pf.parse_summary.contains("扫描页 [2]") && pf.parse_summary.contains("OCR 未解析"),
        "扫描页缺件必须显式登记（清单 + 未解析标记）：{}",
        pf.parse_summary
    );
    assert!(
        pf.parse_summary.contains("非静默丢弃") || pf.parse_summary.contains("重导入"),
        "必须带补救指引：{}",
        pf.parse_summary
    );
    assert_eq!(pf.ocr_blocks, 0);
}
