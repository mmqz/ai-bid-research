//! P1 · PDF 文本抽取布局保真双侧对跑：mdc-pdf（pdfium）vs pdftotext -layout。
//! issue #2 §3.3 已列「真实招标 PDF 对比召回率」——本探针以 B1.5 入库真实件实测。

use std::collections::BTreeMap;

fn main() {
    let repo = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let lib = std::env::var("MDC_PDFIUM_LIB").unwrap_or_else(|_| format!("{}/.pdfium/libpdfium.so", std::env::var("HOME").unwrap()));
    let files = [
        ("cqyd_32p", format!("{repo}/real-corpus/telecom/parse-input/A-cqyd-20260500728-重庆西永水冷空调-招标文件.pdf")),
        ("shigong_142p", format!("{repo}/real-corpus/water/files/A-某水利施工标段招标文件.pdf")),
        ("fujian_ocr_8p", format!("{repo}/real-corpus/water/files/C-福建修订通知-扫描件.pdf")),
    ];
    let probes: Vec<String> = vec![
        "第一章".into(), "第二章".into(), "投标人须知".into(), "前附表".into(),
        "1.1 项目概况".into(), "评标办法".into(), "合同条款".into(), "投标文件".into(),
        "资格".into(), "否决".into(),
    ];
    let mut out = BTreeMap::new();
    for (tag, path) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => { out.insert(tag.clone(), serde_json::json!({"result": format!("FILE_MISSING: {e}")})); continue; }
        };
        let eng = match mdc_pdf::engine_pdfium::PdfiumEngine::open_with_library(std::path::Path::new(&lib), bytes.clone())
            .or_else(|_| mdc_pdf::engine_pdfium::PdfiumEngine::open(bytes.clone())) {
            Ok(d) => d,
            Err(e) => { out.insert(tag.clone(), serde_json::json!({"result": "ENGINE_OPEN_FAILED", "error": format!("{e:?}").chars().take(200).collect::<String>().to_string()})); continue; }
        };
        let n = mdc_pdf::engine::DocumentEngine::page_count(&eng);
        let span = mdc_pdf::page::PageSpan::new(mdc_pdf::page::PageIndex(0), mdc_pdf::page::PageIndex(n.saturating_sub(1))).expect("span");
        let pages = match mdc_pdf::engine::DocumentEngine::extract_text(&eng, span) {
            Ok(p) => p,
            Err(e) => { out.insert(tag.clone(), serde_json::json!({"result": "EXTRACT_FAILED", "error": format!("{e:?}").chars().take(200).collect::<String>().to_string()})); continue; }
        };
        let engine_text: String = pages.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n");
        let engine_chars = engine_text.chars().count();
        let engine_chars_nonws = engine_text.chars().filter(|c| !c.is_whitespace()).count();
        // 关键结构行命中（引擎侧）
        let hit_engine: Vec<&String> = probes.iter().filter(|p| engine_text.contains(p.as_str())).collect();
        // pdftotext -layout 对照
        let pdft = std::process::Command::new("pdftotext")
            .arg("-layout").arg(path).arg("-")
            .output().expect("pdftotext");
        let pdft_text = String::from_utf8_lossy(&pdft.stdout).to_string();
        let pdft_chars = pdft_text.chars().count();
        let pdft_chars_nonws = pdft_text.chars().filter(|c| !c.is_whitespace()).count();
        let hit_pdft: Vec<&String> = probes.iter().filter(|p| pdft_text.contains(p.as_str())).collect();
        // 空页统计（扫描件：文本层空 → 引擎字符数应趋零）
        let empty_pages_engine = pages.iter().filter(|(_, t)| t.trim().is_empty()).count();
        out.insert(tag.clone(), serde_json::json!({
            "result": "ok",
            "pages": n,
            "engine_chars": engine_chars,
            "engine_chars_per_page": engine_chars / n.max(1) as usize,
            "pdftotext_chars": pdft_chars,
            "pdftotext_chars_per_page": pdft_chars / n.max(1) as usize,
            "char_ratio_engine_over_pdftotext": (engine_chars as f64) / (pdft_chars.max(1) as f64),
            "engine_chars_nonws": engine_chars_nonws,
            "pdftotext_chars_nonws": pdft_chars_nonws,
            "char_ratio_nonws_engine_over_pdftotext": (engine_chars_nonws as f64) / (pdft_chars_nonws.max(1) as f64),
            "probe_hits_engine": hit_engine,
            "probe_hits_pdftotext": hit_pdft,
            "engine_empty_pages": empty_pages_engine,
        }));
    }
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
