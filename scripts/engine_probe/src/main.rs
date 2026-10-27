//! engine_probe —— issue #2 · B2 引擎侧探针（只读审计 mdcore @ f8c860a）
//!
//! 重点核实 capability-map 7 条「待验证」中的引擎侧行为：
//!   S7 压缩比门（zip 炸弹 + 合法高压缩比两方向）
//!   S8 嵌套归档递归（条目级 word/media/*.zip + 内容级 document.xml=zip 两变体）
//!   D4/D5/D6 文本框/批注脚注/修订痕迹读面（TR2 真 Word 件 + corpus-R D2a）
//!   X3 合并单元格（xlsx merges API 面 + docx 表格扁平面 + 图纸目录真实件）
//!
//! 纪律：不改引擎任何源码（path 依赖只读 worktree）；输出确定性 JSON
//!（sort_keys 语义由调用方 Python 归一，Rust 侧保字段稳定）。

use std::collections::BTreeMap;
use std::io::Write;

fn read_file(p: &str) -> Option<Vec<u8>> {
    std::fs::read(p).ok()
}

/// 结果对象（确定性字段序——BTreeMap）。
type Obj = BTreeMap<String, String>;

fn obj(pairs: &[(&str, String)]) -> Obj {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn fmt_docx_result(r: Result<mdc_office_io::docx::DocxDocument, mdc_office_io::docx::DocxError>) -> Obj {
    match r {
        Ok(d) => {
            let mut paras = 0usize;
            let mut tables = 0usize;
            let mut texts: Vec<String> = Vec::new();
            for b in &d.blocks {
                match b {
                    mdc_office_io::docx::Block::Paragraph(p) => {
                        paras += 1;
                        texts.push(p.text());
                    }
                    mdc_office_io::docx::Block::Table(t) => {
                        tables += 1;
                        texts.push(format!("<TABLE {}x{}>", t.rows.len(), t.rows.first().map(|r| r.len()).unwrap_or(0)));
                    }
                }
            }
            // PR 精简（issue #2 评论 2026-10-27 03:28 要求）：block_texts 仅保留
            // 前 20 条样例——全量计数由 paragraphs/tables 字段承载，超限件置
            // block_texts_truncated=true。此前 s7b（合法高压缩比件 6000 段）的
            // 全文 dump 达 312K 字符；结论（result/计数/比率）保留、过程数据不入库。
            let truncated = texts.len() > 20;
            let texts_capped: Vec<String> = texts.iter().take(20).cloned().collect();
            let mut o = obj(&[
                ("result", "ok".into()),
                ("paragraphs", paras.to_string()),
                ("tables", tables.to_string()),
                ("headers_n", d.headers.len().to_string()),
                ("footers_n", d.footers.len().to_string()),
                ("images_n", d.images.len().to_string()),
                ("warnings_n", d.warnings.len().to_string()),
                ("block_texts", serde_json::to_string(&texts_capped).unwrap_or_default()),
                ("header_texts", serde_json::to_string(
                    &d.headers.iter().map(|h| h.blocks.iter().map(|b| match b {
                        mdc_office_io::docx::Block::Paragraph(p) => p.text(),
                        mdc_office_io::docx::Block::Table(_) => "<TABLE>".into(),
                    }).collect::<Vec<_>>()).collect::<Vec<_>>()
                ).unwrap_or_default()),
                ("footer_texts", serde_json::to_string(
                    &d.footers.iter().map(|h| h.blocks.iter().map(|b| match b {
                        mdc_office_io::docx::Block::Paragraph(p) => p.text(),
                        mdc_office_io::docx::Block::Table(_) => "<TABLE>".into(),
                    }).collect::<Vec<_>>()).collect::<Vec<_>>()
                ).unwrap_or_default()),
            ]);
            if truncated {
                o.insert("block_texts_truncated".into(), "true".into());
            }
            o
        }
        Err(e) => obj(&[
            ("result", "rejected".into()),
            ("error_text", format!("{e}")[..format!("{e}").len().min(400)].to_string()),
        ]),
    }
}

fn fmt_book_result(r: Result<mdc_office_io::io::Book, mdc_office_io::io::ReadError>) -> Obj {
    match r {
        Ok(b) => {
            let sheets: Vec<Obj> = b.sheets.iter().map(|s| obj(&[
                ("sheet", s.name.clone()),
                ("merges_n", s.merges.len().to_string()),
                ("merges", serde_json::to_string(&s.merges.iter().map(|d| format!("r{}-{}c{}-{}", d.start.0+1, d.end.0+1, d.start.1+1, d.end.1+1)).collect::<Vec<_>>()).unwrap_or_default()),
                ("formulas_n", s.formulas.used_cells().count().to_string()),
                ("hyperlinks_n", s.hyperlinks.len().to_string()),
                ("rows_n", s.values.height().to_string()),
                ("cols_n", s.values.width().to_string()),
            ])).collect();
            obj(&[
                ("result", "ok".into()),
                ("sheets", serde_json::to_string(&sheets).unwrap_or_default()),
                ("warnings_n", b.warnings.len().to_string()),
            ])
        }
        Err(e) => obj(&[
            ("result", "rejected".into()),
            ("error_text", format!("{e}")[..format!("{e}").len().min(400)].to_string()),
        ]),
    }
}

/// 构造合法高压缩比 docx：以有效基础 docx（D2a）为底，仅替换 word/document.xml
/// 为高重复正文（诚实声明尺寸——真实招标文件大量重复样板段落的极端化，非炸弹）。
fn rebuild_with_body(base: &[u8], new_doc_xml: &[u8]) -> Vec<u8> {
    let mut reader = zip::ZipArchive::new(std::io::Cursor::new(base.to_vec())).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..reader.len() {
        let mut f = reader.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).unwrap();
        if name == "word/document.xml" {
            out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(new_doc_xml).unwrap();
        } else {
            out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(&buf).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}

fn build_legit_high_ratio(base: &[u8]) -> (Vec<u8>, u64, u64) {
    let para = "<w:p><w:r><w:t>本招标文件章节内容为重复样板文本，用于压缩比探针；投标人须知前附表格式条款重复出现属真实文档常态。</w:t></w:r></w:p>";
    let body = para.repeat(6000);
    let doc = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let xml = doc.as_bytes();
    let declared = xml.len() as u64;
    // 预压测压缩比（与落盘同 deflate）
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.start_file("probe.bin", zip::write::SimpleFileOptions::default()).unwrap();
    z.write_all(xml).unwrap();
    let compressed = z.finish().unwrap().into_inner().len() as u64;
    (rebuild_with_body(base, xml), declared, compressed)
}

/// S8a：条目级嵌套归档（在有效容器内追加 word/media/attached.zip——bid 侧 NESTED_ARCHIVE_EXTS 拒绝面）。
fn build_nested_entry_docx(base: &[u8]) -> Vec<u8> {
    let mut reader = zip::ZipArchive::new(std::io::Cursor::new(base.to_vec())).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..reader.len() {
        let mut f = reader.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).unwrap();
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(&buf).unwrap();
    }
    let mut inner = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    inner.start_file("inner.txt", zip::write::SimpleFileOptions::default()).unwrap();
    inner.write_all(b"nested archive payload").unwrap();
    let inner = inner.finish().unwrap().into_inner();
    out.start_file("word/media/attached.zip", zip::write::SimpleFileOptions::default()).unwrap();
    out.write_all(&inner).unwrap();
    out.finish().unwrap().into_inner()
}

/// S8a2：嵌套 zip 且补全 content-type 声明（Default zip）——引擎是否仍放行。
fn build_nested_declared_docx(base: &[u8]) -> Vec<u8> {
    let nested = build_nested_entry_docx(base);
    let mut reader = zip::ZipArchive::new(std::io::Cursor::new(nested)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..reader.len() {
        let mut f = reader.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).unwrap();
        if name == "[Content_Types].xml" {
            let xml = String::from_utf8_lossy(&buf).replace(
                "</Types>",
                "<Default Extension=\"zip\" ContentType=\"application/zip\"/></Types>",
            );
            out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(xml.as_bytes()).unwrap();
        } else {
            out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(&buf).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}

/// S8b：内容级伪装（word/document.xml 字节实为 zip——扩展名不暴露）。
fn build_masqueraded_docx(base: &[u8]) -> Vec<u8> {
    let mut inner = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    inner.start_file("evil.txt", zip::write::SimpleFileOptions::default()).unwrap();
    inner.write_all(b"masqueraded zip content").unwrap();
    let inner = inner.finish().unwrap().into_inner();
    rebuild_with_body(base, &inner)
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let fixtures = if args.len() > 1 { args[1].clone() } else { ".".into() };
    let hostile = format!("{fixtures}/baseline/security/hostile");
    let tr2 = "/home/z/my-project/repos/mdcore-audit-f8c860a/test-artifacts/TR2/fixtures";
    let corpus_r = format!("{fixtures}/fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx");
    let corpus_a_b3 = format!("{fixtures}/fixtures/corpus-A/files/B3-图纸目录.xlsx");
    let word_merged = format!("{tr2}/word-merged.docx");
    let excel_complex = format!("{tr2}/excel-complex.xlsx");
    let word_complex = format!("{tr2}/word-complex.docx");

    let mut probes: BTreeMap<String, Obj> = BTreeMap::new();

    // ---- S7a · zip 炸弹（声明 10KB / 实解 1GB）引擎侧行为
    if let Some(bomb) = read_file(&format!("{hostile}/01-zip-bomb-declared-10kb-actual-1gb.docx")) {
        let mut o = fmt_docx_result(mdc_office_io::docx::read_docx_from_bytes(&bomb));
        o.insert("file_bytes".into(), bomb.len().to_string());
        probes.insert("s7a_zip_bomb_engine".into(), o);
    }

    // ---- S7b · 合法高压缩比（诚实声明、非炸弹）
    let d2a_bytes = read_file(&corpus_r).unwrap_or_default();
    let (hi, declared, compressed) = build_legit_high_ratio(&d2a_bytes);
    {
        let ratio = if compressed > 0 { declared / compressed } else { 0 };
        let mut o = fmt_docx_result(mdc_office_io::docx::read_docx_from_bytes(&hi));
        o.insert("declared_bytes".into(), declared.to_string());
        o.insert("compressed_bytes".into(), compressed.to_string());
        o.insert("ratio_declared_per_compressed".into(), ratio.to_string());
        probes.insert("s7b_legit_high_ratio_engine".into(), o);
    }

    // ---- S8a/S8b · 嵌套归档两变体
    {
        let d2a_bytes2 = read_file(&corpus_r).unwrap_or_default();
        let nested = build_nested_entry_docx(&d2a_bytes2);
        let mut o = fmt_docx_result(mdc_office_io::docx::read_docx_from_bytes(&nested));
        o.insert("variant".into(), "entry-level word/media/attached.zip".into());
        probes.insert("s8a_nested_entry_engine".into(), o);
    }
    {
        let d2a_bytes3 = read_file(&corpus_r).unwrap_or_default();
        let masq = build_masqueraded_docx(&d2a_bytes3);
        let mut o = fmt_docx_result(mdc_office_io::docx::read_docx_from_bytes(&masq));
        o.insert("variant".into(), "content-level document.xml=zip".into());
        probes.insert("s8b_masqueraded_engine".into(), o);
    }
    {
        let d2a_bytes4 = read_file(&corpus_r).unwrap_or_default();
        let decl = build_nested_declared_docx(&d2a_bytes4);
        let mut o = fmt_docx_result(mdc_office_io::docx::read_docx_from_bytes(&decl));
        o.insert("variant".into(), "entry-level nested zip + content-type declared".into());
        probes.insert("s8a2_nested_declared_engine".into(), o);
    }

    // ---- D4/D5/D6 · 读面对比（TR2 真 Word 件 + corpus-R D2a）
    if let Some(b) = read_file(&word_complex) {
        probes.insert("d456_tr2_word_complex_engine".into(), fmt_docx_result(mdc_office_io::docx::read_docx_from_bytes(&b)));
    } else {
        probes.insert("d456_tr2_word_complex_engine".into(), obj(&[("result", "FILE_MISSING".into())]));
    }
    if let Some(b) = read_file(&corpus_r) {
        probes.insert("d456_corpus_r_d2a_engine".into(), fmt_docx_result(mdc_office_io::docx::read_docx_from_bytes(&b)));
    } else {
        probes.insert("d456_corpus_r_d2a_engine".into(), obj(&[("result", "FILE_MISSING".into())]));
    }

    // ---- X3 · 合并单元格（docx 扁平面 + xlsx merges 面 + 真实图纸目录）
    if let Some(b) = read_file(&word_merged) {
        probes.insert("x3_tr2_word_merged_engine".into(), fmt_docx_result(mdc_office_io::docx::read_docx_from_bytes(&b)));
    } else {
        probes.insert("x3_tr2_word_merged_engine".into(), obj(&[("result", "FILE_MISSING".into())]));
    }
    if let Some(b) = read_file(&excel_complex) {
        probes.insert("x3_tr2_excel_complex_engine".into(), fmt_book_result(mdc_office_io::io::read_book_from_bytes(&b)));
    } else {
        probes.insert("x3_tr2_excel_complex_engine".into(), obj(&[("result", "FILE_MISSING".into())]));
    }
    if let Some(b) = read_file(&corpus_a_b3) {
        probes.insert("x3_corpus_a_b3_tuzhi_engine".into(), fmt_book_result(mdc_office_io::io::read_book_from_bytes(&b)));
    } else {
        probes.insert("x3_corpus_a_b3_tuzhi_engine".into(), obj(&[("result", "FILE_MISSING".into())]));
    }

    let json = serde_json::to_string_pretty(&probes).unwrap();
    println!("{json}");
}
