//! DOCX 解析：zip + quick-xml 走 word/document.xml。
//! 输出与文档顺序一致的块序列（段落 / 表格行），并维护章节上下文（章 → 节 → 条款号前缀）
//! 作为 locator —— 下游条款抽取与影响面分析的位置真源。

use crate::guard::{self, LIMIT_ACTUAL_READ_DOCX, LIMIT_INPUT_BYTES_DOCX, LIMIT_XML_DEPTH};
use crate::{Block, BlockKind, ComponentLedger, ParsedFile};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::path::Path;

pub fn parse(path: &Path) -> Result<ParsedFile, String> {
    // T-03 防御链：输入字节门 → prescan 三门 → 受限读取（实际累计计量）→ XML 深度门
    let mut zf = guard::open_guarded(path, LIMIT_INPUT_BYTES_DOCX)
        .map_err(|e| format!("zip 防护拒绝: {e}"))?;
    let doc_xml = guard::read_entry_limited(&mut zf, "word/document.xml", LIMIT_ACTUAL_READ_DOCX)
        .map_err(|e| format!("读取防护拒绝: {e}"))?;

    let mut reader = Reader::from_str(&doc_xml);
    reader.config_mut().trim_text(true);

    let mut blocks: Vec<Block> = Vec::new();
    let mut chapter = String::new();
    let mut subsection = String::new();

    let mut in_table = false;
    let mut table_idx: usize = 0;
    let mut in_cell = false;
    let mut cell_text = String::new();
    let mut row_cells: Vec<String> = Vec::new();
    let mut row_no: usize = 0;
    let mut para_text = String::new();
    let mut in_para = false;
    let mut capture_text = false;

    // 表格挂在哪个章节上下文（表格开始时冻结当前章节——locator 直接携带）
    let mut tbl_chapter = String::new();

    fn emit_para(blocks: &mut Vec<Block>, idx: usize, chapter: &str, subsection: &str, text: &str) {
        let locator = if chapter.is_empty() {
            String::new()
        } else if subsection.is_empty() {
            chapter.to_string()
        } else {
            format!("{chapter} > {subsection}")
        };
        blocks.push(Block { idx, kind: BlockKind::Para, locator, text: text.to_string(), cells: vec![] });
    }

    // T-03：XML 嵌套深度门（引擎 LIMIT_XML_DEPTH 同构；深嵌套耗内存/CPU 面）
    let mut depth: usize = 0;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                depth += 1;
                if depth > LIMIT_XML_DEPTH {
                    return Err(format!(
                        "XML 嵌套深度超限 {depth} > {LIMIT_XML_DEPTH}（防护拒绝）"));
                }
                match e.name().as_ref() {
                b"w:tbl" => {
                    in_table = true;
                    table_idx += 1;
                    row_no = 0;
                    tbl_chapter = chapter.clone();
                }
                b"w:tr" if in_table => {
                    row_no += 1;
                    row_cells.clear();
                }
                b"w:tc" if in_table => {
                    in_cell = true;
                    cell_text.clear();
                }
                b"w:p" => {
                    in_para = true;
                    para_text.clear();
                }
                b"w:t" | b"w:tab" => {
                    capture_text = true;
                }
                _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if capture_text {
                    let txt = t.unescape().map_err(|e| e.to_string())?;
                    if in_cell {
                        cell_text.push_str(&txt);
                    } else if in_para {
                        para_text.push_str(&txt);
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                // 空元素（如 <w:tab/>）：对 tab 补一个空格分隔符
                if e.name().as_ref() == b"w:tab" && in_para && !in_cell {
                    para_text.push(' ');
                }
            }
            Ok(Event::End(e)) => {
                // 递减先于 match——`continue` 跳段也不会漏计（saturating 防负）
                depth = depth.saturating_sub(1);
                match e.name().as_ref() {
                b"w:p" => {
                    in_para = false;
                    capture_text = false;
                    if in_cell {
                        cell_text.push(' '); // 单元格内多段以空格连接
                    } else {
                        let t = para_text.trim().to_string();
                        if t.is_empty() {
                            continue;
                        }
                        // 章节上下文更新（按文本模式——fixture 与政采范本惯例）
                        if let Some(c) = as_chapter(&t) {
                            chapter = c;
                            subsection.clear();
                        } else if let Some(s) = as_subsection(&t) {
                            subsection = s;
                        }
                        let idx = blocks.len();
                        emit_para(&mut blocks, idx, &chapter, &subsection, &t);
                    }
                }
                b"w:tc" => {
                    in_cell = false;
                    capture_text = false;
                    row_cells.push(cell_text.trim().to_string());
                }
                b"w:tr" => {
                    if !row_cells.iter().all(|c| c.trim().is_empty()) {
                        let locator = format!("{tbl_chapter} 表{table_idx} 行{row_no}");
                        let text = row_cells
                            .iter()
                            .filter(|c| !c.trim().is_empty())
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(" ");
                        blocks.push(Block {
                            idx: blocks.len(),
                            kind: BlockKind::TableRow,
                            locator,
                            text,
                            cells: row_cells.clone(),
                        });
                    }
                }
                b"w:tbl" => {
                    in_table = false;
                }
                _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("XML 事件错误: {e}")),
        }
    }

    // 页数估算：DOCX 无真实分页（渲染口径页数属 M4 导出面）；以结构页（章）计
    let pages = 1 + blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Para && as_chapter(&b.text).is_some())
        .count();
    let tables = table_idx;

    // ---- SC-02 部件对账账本（登记不伪装面扩展到部件级，issue #38 吸收）：
    // 在位部件 vs 进入输出流的提取数；零提取部件必须显式登记（note 非空）——
    // G7 面从文档级登记升级为逐文件可见。语义口径：
    //   extracted = 进入**独立输出块**的部件数（页眉/页脚/批注/脚注等独立部件
    //   文本从不进入块流 → 恒 0 + 显式登记；文本框/修订插入文本会**内联**并入宿主
    //   段落流——文本未丢失但非独立块，note 如实区分「内联」与「未提取」）。
    let names: Vec<String> = zf.file_names().map(|n| n.to_string()).collect();
    let headers = names.iter()
        .filter(|n| n.starts_with("word/header") && n.ends_with(".xml"))
        .count();
    let footers = names.iter()
        .filter(|n| n.starts_with("word/footer") && n.ends_with(".xml"))
        .count();
    let comments = names.iter()
        .filter(|n| n.starts_with("word/comments") && n.ends_with(".xml"))
        .count();
    let fn_en: usize = names.iter()
        .filter(|n| (n.starts_with("word/footnotes") || n.starts_with("word/endnotes"))
            && n.ends_with(".xml"))
        .count();
    // txbxContent 开闭成对 → 除 2；w:ins/w:delText 按开标签计次
    let txbx = doc_xml.matches("txbxContent").count() / 2;
    let ins = doc_xml.matches("<w:ins ").count() + doc_xml.matches("<w:ins>").count();
    let del_text = doc_xml.matches("<w:delText").count();
    let mut components = vec![ComponentLedger {
        component: "正文".into(), present: 1, extracted: 1, note: String::new(),
    }];
    let mut push_ledger = |component: &str, present: usize, extracted: usize, note: &str| {
        if present > 0 {
            components.push(ComponentLedger {
                component: component.into(), present, extracted, note: note.into(),
            });
        }
    };
    push_ledger("页眉", headers, 0, "G7：页眉部件在位、零提取（引擎非正文定位器变体待接入，登记不伪装）");
    push_ledger("页脚", footers, 0, "G7：页脚部件在位、零提取（同上）");
    push_ledger("批注", comments, 0, "G7：批注部件在位、零提取（批注读面待 M2+）");
    push_ledger("脚注尾注", fn_en, 0, "G7：脚注/尾注部件在位、零提取（同上）");
    // 文本框：嵌套 w:p 语义下其内段落以**独立块**进入流；宿主段落位于文本框
    // 之后的文本因 in_para 状态被截断（本层真实怪癖——如实登记，引擎面根治）
    push_ledger("文本框", txbx, txbx, "嵌套段落成独立块入流；宿主段落在框后文本被截断（定位器变体与读面归引擎 M2+）");
    push_ledger("修订插入", ins, 0, "插入文本内联并入宿主段落（非独立块）；修订语义读面待 M2+");
    push_ledger("修订删除", del_text, 0, "删除文本 w:delText 未提取（不在捕获面——真实丢失面，登记在案）");
    let zero_unnoted = components.iter()
        .any(|c| c.present > 0 && c.extracted == 0 && c.note.is_empty());
    debug_assert!(!zero_unnoted, "SC-02 不变式：零提取部件必须显式登记");
    let ledger_note = if components.len() > 1 {
        format!("｜部件对账 {} 面（零提取件显式登记）", components.len() - 1)
    } else {
        String::new()
    };

    Ok(ParsedFile {
        name: String::new(),
        sha256: String::new(),
        pages,
        tables,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary: String::new(),
        components,
    })
    .map(|mut p| {
        let cnt = p.blocks.len();
        p.parse_summary = format!("docx: {} 表格，{cnt} 块{}", p.tables, ledger_note);
        p
    })
}

fn cn_num_ok(s: &str) -> bool {
    s.chars().all(|c| "一二三四五六七八九十百".contains(c))
}

fn as_chapter(t: &str) -> Option<String> {
    // 「第X章 标题」——章号用中文数字；携带完整标题（类别映射与 GT locator 口径需要全文）
    if t.len() >= 4 && t.starts_with("第") {
        if let Some(pos) = t.find("章") {
            let num = &t[3..pos]; // "第" 占 3 字节
            if !num.is_empty() && cn_num_ok(num) && pos + 3 <= t.len() {
                return Some(t.trim().to_string());
            }
        }
    }
    None
}

fn as_subsection(t: &str) -> Option<String> {
    // 「一、标题」——中文序号节标题（长度短，避免把正文当标题）；携带完整标题
    if t.chars().count() <= 30 {
        if let Some(idx) = t.find('、') {
            let head = &t[..idx];
            if !head.is_empty() && cn_num_ok(head) {
                return Some(t.trim().to_string());
            }
        }
    }
    None
}

pub fn is_numbered_clause(t: &str) -> bool {
    // 「3.2.1 正文…」——层级条款号开头（数字开头且含点）
    let head: String = t.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    head.contains('.') && head.len() >= 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    #[test]
    fn chapter_subsection_patterns() {
        assert!(as_chapter("第三章 投标人须知").is_some());
        assert!(as_chapter("第十二章 合同").is_some());
        assert!(as_chapter("3.1.1 正文不是章").is_none());
        assert!(as_subsection("七、开标、评标与否决条款").is_some());
        assert!(as_subsection("二、投标人资格要求").is_some());
        assert!(as_subsection("评标办法（综合评分法）").is_none());
        assert!(is_numbered_clause("3.2.1 投标人须具有独立承担民事责任能力的法人营业执照"));
        assert!(is_numbered_clause("4.1 价格分（30 分）"));
        assert!(!is_numbered_clause("投标文件格式"));
    }

    fn real_fixture() -> std::path::PathBuf {
        // 真实公开文件样本（real-samples/README.md 溯源：北京市水务局
        // 《水利工程施工电子招标文件示范文本（2025 版）》官方公开示范文本，
        // LibreOffice .doc→.docx 转换工件——LO 生成器真实产物）
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../real-samples/北京水利施工示范文本2025-LO.docx")
    }

    /// SC-02 部件对账（真实锚点）：真实示范文本 130 部件（58 页眉 + 58 页脚 +
    /// 52 表格 + 6698 段落）——账本必须显式登记 116 个零提取部件（G7 面逐文件
    /// 可见化）。破坏推演：删账本（或 extracted 误计 >0）→ 58/0 断言红；
    /// 零提取部件无 note → 不变式断言红。
    #[test]
    fn real_sample_component_ledger_beijing_2025() {
        let p = real_fixture();
        assert!(p.exists(), "真实样本缺失：{p:?}");
        let pf = parse(&p).expect("真实示范文本必须可解析");
        assert_eq!(pf.tables, 52, "真实表格数");
        assert!(pf.blocks.len() > 1000, "真实段落量级：{}", pf.blocks.len());
        let row = |name: &str| pf.components.iter().find(|c| c.component == name);
        let body = row("正文").expect("正文行在位");
        assert_eq!((body.present, body.extracted), (1, 1));
        let h = row("页眉").expect("页眉行在位");
        assert_eq!((h.present, h.extracted), (58, 0), "58 页眉件零提取且显式登记");
        assert!(!h.note.is_empty(), "零提取部件必须带登记注记");
        let f = row("页脚").expect("页脚行在位");
        assert_eq!((f.present, f.extracted), (58, 0));
        assert!(row("批注").is_none(), "真实件无批注部件——不在位不入账（不虚记）");
        assert!(row("文本框").is_none(), "真实件无文本框");
        // SC-02 不变式：零提取部件必须显式登记（note 非空）
        assert!(pf.components.iter()
            .filter(|c| c.present > 0 && c.extracted == 0)
            .all(|c| !c.note.is_empty()));
        assert!(pf.parse_summary.contains("部件对账"), "summary 须携带账本口径：{}", pf.parse_summary);
    }

    /// SC-02 合成补面：真实件缺的部件（批注/脚注/修订/文本框）字节级构造——
    /// 补齐「零提取显式登记」的覆盖面（真实锚点 + 定向合成互补，非替代）。
    /// 破坏推演：批注/脚注行 missing → 红；w:delText 行 extracted≠0 → 红。
    #[test]
    fn component_ledger_synthetic_complement() {
        let d = std::env::temp_dir().join("bidparse_docx_test");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("ledger_{}.docx", std::process::id()));
        let f = std::fs::File::create(&p).unwrap();
        let mut w = ZipWriter::new(f);
        let opt = SimpleFileOptions::default();
        let doc = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
            r#"<w:p><w:r><w:t>第一章 总则</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:delText w:id="d1">被删除的修订文本</w:delText></w:r></w:p>"#,
            r#"<w:p><w:ins w:id="i1" w:author="审计"><w:r><w:t>插入的修订文本</w:t></w:r></w:ins></w:p>"#,
            r#"<w:p><w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml"><v:textbox><w:txbxContent><w:p><w:r><w:t>文本框内文本</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>"#,
            r#"</w:body></w:document>"#,
        );
        w.start_file("word/document.xml", opt).unwrap();
        w.write_all(doc.as_bytes()).unwrap();
        w.start_file("word/header1.xml", opt).unwrap();
        w.write_all(br#"<w:hdr/>"#).unwrap();
        w.start_file("word/footer1.xml", opt).unwrap();
        w.write_all(br#"<w:ftr/>"#).unwrap();
        w.start_file("word/comments.xml", opt).unwrap();
        w.write_all(br#"<w:comments/>"#).unwrap();
        w.start_file("word/footnotes.xml", opt).unwrap();
        w.write_all(br#"<w:footnotes/>"#).unwrap();
        w.finish().unwrap();

        let pf = parse(&p).expect("合成补面件必须可解析");
        let row = |name: &str| pf.components.iter().find(|c| c.component == name);
        assert_eq!(row("页眉").map(|c| (c.present, c.extracted)), Some((1, 0)));
        assert_eq!(row("页脚").map(|c| (c.present, c.extracted)), Some((1, 0)));
        assert_eq!(row("批注").map(|c| (c.present, c.extracted)), Some((1, 0)));
        assert_eq!(row("脚注尾注").map(|c| (c.present, c.extracted)), Some((1, 0)));
        // 文本框：嵌套 w:p 成独立块（extracted==present==1）
        assert_eq!(row("文本框").map(|c| (c.present, c.extracted)), Some((1, 1)));
        // 修订插入内联（0 独立块）；修订删除真实未提取（0）
        assert_eq!(row("修订插入").map(|c| (c.present, c.extracted)), Some((1, 0)));
        assert_eq!(row("修订删除").map(|c| (c.present, c.extracted)), Some((1, 0)));
        // 全部零提取行 note 非空（SC-02 不变式）
        assert!(pf.components.iter()
            .filter(|c| c.present > 0 && c.extracted == 0)
            .all(|c| !c.note.is_empty()));
        // 提取流真实性：文本框文本成块；插入文本内联进宿主段；删除文本不出现
        let texts: Vec<&str> = pf.blocks.iter().map(|b| b.text.as_str()).collect();
        assert!(texts.contains(&"文本框内文本"), "嵌套段落独立块：{texts:?}");
        assert!(texts.contains(&"插入的修订文本"), "w:ins 内联并入段落：{texts:?}");
        assert!(!texts.iter().any(|t| t.contains("被删除")), "w:delText 不得入流：{texts:?}");
    }

    /// SC-08 生成器差异对（真实面）：同一批真实段落文本，经两种生成器——
    /// ①LibreOffice 转换工件（real-samples 真实 LO 产物）
    /// ②字节级直构 docx（raw zip + 最小 XML——与 LO 生成器差异最大化）
    /// ——提取的段落流必须逐字一致（指纹判定不因生成器差异漂移 → 不误降级）。
    /// 破坏推演：提取流对生成器形状敏感（XML 转义/空白处理漂移）→ 流不等 → 红。
    #[test]
    fn generator_pair_stream_invariance() {
        let lo = parse(&real_fixture()).expect("LO 生成器工件解析");
        // 取前 200 个真实段落文本作为对照集（确定性子集）
        let paras: Vec<String> = lo.blocks.iter()
            .filter(|b| b.kind == BlockKind::Para)
            .take(200)
            .map(|b| b.text.clone())
            .collect();
        assert!(paras.len() == 200, "真实件段落数不足：{}", paras.len());

        // 生成器 B：直构最小 docx（单 run 单段；XML 转义保持文本逐字）
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let mut body = String::with_capacity(paras.iter().map(|p| p.len() + 40).sum());
        body.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        body.push_str(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#);
        for t in &paras {
            body.push_str(&format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", esc(t)));
        }
        body.push_str("</w:body></w:document>");

        let d = std::env::temp_dir().join("bidparse_docx_test");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("twin_{}.docx", std::process::id()));
        let f = std::fs::File::create(&p).unwrap();
        let mut w = ZipWriter::new(f);
        w.start_file("word/document.xml", SimpleFileOptions::default()).unwrap();
        w.write_all(body.as_bytes()).unwrap();
        w.finish().unwrap();

        let twin = parse(&p).expect("直构孪生件解析");
        let lo_texts: Vec<&str> = lo.blocks.iter()
            .filter(|b| b.kind == BlockKind::Para).take(200)
            .map(|b| b.text.as_str()).collect();
        let twin_texts: Vec<&str> = twin.blocks.iter()
            .filter(|b| b.kind == BlockKind::Para).take(200)
            .map(|b| b.text.as_str()).collect();
        assert_eq!(lo_texts, twin_texts, "段落流必须生成器无关（提取不变性）");

        // 指纹稳定性（SC-08 oracle 主断言）：同文本 → 同指纹 → 不误降级
        use bid_domain::linkage::text_fingerprint;
        let fp_lo: Vec<String> = lo_texts.iter().map(|t| text_fingerprint(t)).collect();
        let fp_twin: Vec<String> = twin_texts.iter().map(|t| text_fingerprint(t)).collect();
        assert_eq!(fp_lo, fp_twin, "指纹判定不得因生成器差异漂移");
    }
}
