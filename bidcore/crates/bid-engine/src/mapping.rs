//! EngineX → `bid_parse::ParsedFile` 字段级映射签名（T1：只定义，B3 实现）。
//!
//! 单一真源文档：`docs/engine-bid-mapping.md`（引擎字段 × bid 字段 ×
//! 转换规则 × 缺失处置四列表——本模块签名与该文档一一对应，B3 实现时
//! 以文档为验收对照）。
//!
//! B3 实现验收（issue #2 §B3）：≥90% 文件 blocks 完全等价；不等价每条
//! 有归类结论（等价 / 引擎缺失 / 旧实现 bug / 预期差异）——归类由
//! `scripts/engine_diff.py` 双跑差分工具产出。

use crate::types::{
    EngineBlock, EngineBlockSource, EngineBook, EngineDoc, EngineError, EnginePdf,
};
use bid_parse::{Block, BlockKind, ComponentLedger, ParsedFile};

/// 调用方供给的文件元数据（sha256 由门面层计算——引擎不做文件级指纹）。
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub name: String,
    pub sha256: String,
    pub bytes: usize,
}

/// docx：EngineDoc → ParsedFile（B5 实装——issue #2 §B5 import 接 bid-engine）。
///
/// 映射规则单一真源：`docs/engine-bid-mapping.md` §1。要点：
/// - `EngineBlock::Paragraph/TextBox`（source=Body）→ `BlockKind::Para`；
///   **空段丢弃**（EMPTY-PARA 对齐旧口径）；章节上下文沿旧解析器同款文本模式
///   （本模块 `as_chapter`/`as_subsection`——B4 自 bid-parse/src/docx.rs 迁入），
///   locator = `章` 或 `章 > 节`（` > ` 分隔——extract.rs `split_locator` 口径）；
/// - `EngineBlock::Table`（source=Body）→ 每行一块 `BlockKind::TableRow`
///   （cells 直连；text=非空列单空格连接；全空行跳过但行号照旧消耗——旧实现对齐）；
///   locator = `{表起章节} 表{N} 行{M}`；
/// - Header/Footer source 块**不入流**（页面装置——登记面，映射表 §1 D3 行）；
/// - 页眉/页脚/图片/批注/脚注尾注/修订 → `components` 账本（SC-02 对账：
///   present/extracted/note——零提取必带 note，登记不伪装）；
/// - pages=0 / has_text_layer=true / ocr_blocks=0 / tables=表计数（映射表 §1 末行）。
///
/// 已知引擎缺口（登记不伪装，映射层**禁止自研补齐**——issue #2 约束 3）：
/// - 嵌套表格（mdcore#39）：`w:tbl@w:tc` 内层表引擎不提取——含嵌套表的招标件
///   （费率表族）内容缺失，账本 note 显式登记提示面；
/// - 域剥离边界（mdcore#40）：`""` 邻接正文疑被吞——待引擎侧裁决。
#[allow(clippy::needless_pass_by_value)]
pub fn docx_to_parsed_file(
    doc: EngineDoc,
    meta: &FileMeta,
) -> Result<ParsedFile, EngineError> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut chapter = String::new();
    let mut subsection = String::new();
    let mut table_idx: usize = 0;
    // 部件账本计数面
    let (mut tb_present, mut rev_ins_present, mut fields_dropped_total) = (0usize, 0usize, 0usize);

    let emit_para = |blocks: &mut Vec<Block>, chapter: &str, subsection: &str, text: &str| {
        let locator = if chapter.is_empty() {
            String::new()
        } else if subsection.is_empty() {
            chapter.to_string()
        } else {
            format!("{chapter} > {subsection}")
        };
        blocks.push(Block {
            idx: blocks.len(),
            kind: BlockKind::Para,
            locator,
            text: text.to_string(),
            cells: vec![],
        });
    };

    /// 段落发射器类型（章节上下文 → locator 组装——emit_para 闭包的传递形态）。
    type EmitPara<'a> = &'a dyn Fn(&mut Vec<Block>, &str, &str, &str);

    /// 文本框内层块拍平（映射表 §1：TextBox 与正文同构，块序紧随宿主段）。
    fn flatten_textbox(
        tb_blocks: &[EngineBlock],
        blocks: &mut Vec<Block>,
        chapter: &str,
        subsection: &str,
        table_idx: &mut usize,
        emit_para: EmitPara<'_>,
    ) {
        for inner in tb_blocks {
            match inner {
                EngineBlock::Paragraph(p) => {
                    let t = p.text.trim();
                    if !t.is_empty() {
                        emit_para(blocks, chapter, subsection, t);
                    }
                }
                EngineBlock::Table(t) => {
                    // 文本框内表：并入文档序表计数（旧实现同口径——表号连续）
                    *table_idx += 1;
                    emit_rows(&t.rows, blocks, chapter, *table_idx);
                }
                EngineBlock::TextBox(_) => {} // 嵌套文本框：引擎不产生（镜像保真面）
            }
        }
    }

    /// 表行发射（全空行跳过但行号消耗——旧实现 w:tr 对齐）。
    fn emit_rows(rows: &[Vec<String>], blocks: &mut Vec<Block>, tbl_chapter: &str, table_idx: usize) {
        for (i, row) in rows.iter().enumerate() {
            let row_no = i + 1;
            if row.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            let text = row
                .iter()
                .filter(|c| !c.trim().is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            blocks.push(Block {
                idx: blocks.len(),
                kind: BlockKind::TableRow,
                locator: format!("{tbl_chapter} 表{table_idx} 行{row_no}"),
                text,
                cells: row.clone(),
            });
        }
    }

    for b in &doc.blocks {
        // 页面装置分流（PR #15 review：Header/Footer 不进条款抽取流）
        if b.source() != EngineBlockSource::Body {
            continue;
        }
        match b {
            EngineBlock::Paragraph(p) => {
                let t = p.text.trim().to_string();
                if t.is_empty() {
                    continue; // EMPTY-PARA：引擎保留空段，旧实现丢弃——对齐旧口径
                }
                if !p.revision_inserted.is_empty() {
                    rev_ins_present += 1;
                }
                fields_dropped_total += p.fields_dropped;
                // 章节上下文（旧解析器同款文本模式——extract.rs 类别映射依赖 locator 携带；
                // B4：三助手已自 bid-parse/src/docx.rs 迁入本 crate，本地同口径）
                if let Some(c) = as_chapter(&t) {
                    chapter = c;
                    subsection.clear();
                } else if let Some(s) = as_subsection(&t) {
                    subsection = s;
                }
                emit_para(&mut blocks, &chapter, &subsection, &t);
            }
            EngineBlock::Table(t) => {
                table_idx += 1;
                let tbl_chapter = chapter.clone();
                emit_rows(&t.rows, &mut blocks, &tbl_chapter, table_idx);
            }
            EngineBlock::TextBox(tb) => {
                tb_present += 1;
                flatten_textbox(&tb.blocks, &mut blocks, &chapter, &subsection, &mut table_idx, &emit_para);
            }
        }
    }

    // 修订删除计数（引擎口径：delText 不入正文，warnings 逐段登记）
    let rev_del_present = doc
        .warnings
        .iter()
        .filter(|w| w.code.contains("RevisionDelete") || w.message.contains("delText"))
        .count();

    // ---- SC-02 部件对账账本（映射表 §1：present / extracted / note 三列）----
    let body_paras = blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Para))
        .count();
    let mut components: Vec<ComponentLedger> = Vec::new();
    components.push(ComponentLedger {
        component: "正文".into(),
        present: 1,
        extracted: body_paras,
        note: String::new(),
    });
    components.push(ComponentLedger {
        component: "页眉".into(),
        present: doc.headers.len(),
        extracted: 0,
        note: "引擎读面已接（D3）；页面装置不入块流——M2 定位器变体消费".into(),
    });
    components.push(ComponentLedger {
        component: "页脚".into(),
        present: doc.footers.len(),
        extracted: 0,
        note: "引擎读面已接（D3）；页面装置不入块流——M2 定位器变体消费".into(),
    });
    components.push(ComponentLedger {
        component: "文本框".into(),
        present: tb_present,
        extracted: tb_present,
        note: if tb_present > 0 {
            "#18 已修复：内层段落/表拍平入流（块序紧随宿主段）".into()
        } else {
            String::new()
        },
    });
    components.push(ComponentLedger {
        component: "批注".into(),
        present: doc.comments.len(),
        extracted: 0,
        note: if doc.comments.is_empty() {
            String::new()
        } else {
            "#19 已修复：批注面在位（anchor_block_idx 锚关联——M2 证据链消费设计）".into()
        },
    });
    let fn_en = doc.footnotes + doc.endnotes;
    components.push(ComponentLedger {
        component: "脚注尾注".into(),
        present: fn_en,
        extracted: 0,
        note: if fn_en > 0 {
            "#19 已修复：脚注/尾注部件在位（分隔符已滤）；引用位段落级标注面待 M2 消费".into()
        } else {
            String::new()
        },
    });
    components.push(ComponentLedger {
        component: "修订插入".into(),
        present: rev_ins_present,
        extracted: 0,
        note: if rev_ins_present > 0 {
            "#17 已修复：ins 内 run 已入 text()（账本按独立块计 0——与旧口径一致）".into()
        } else {
            String::new()
        },
    });
    components.push(ComponentLedger {
        component: "修订删除".into(),
        present: rev_del_present,
        extracted: 0,
        note: if rev_del_present > 0 {
            "#17 口径：delText 不入正文（旧实现条件性泄漏为 OLD_BUG，B4 消除）".into()
        } else {
            String::new()
        },
    });
    components.push(ComponentLedger {
        component: "图片".into(),
        present: doc.images.len(),
        extracted: 0,
        note: if doc.images.is_empty() {
            String::new()
        } else {
            "容器级枚举（OCR/扫描消费走 pdf 通道）".into()
        },
    });
    if fields_dropped_total > 0 {
        components.push(ComponentLedger {
            component: "域剥离".into(),
            present: fields_dropped_total,
            extracted: 0,
            note: "A2 已修复：表现型域缓存结果剥离（fields_dropped 逐段计数透传）".into(),
        });
    }
    if !doc.warnings.is_empty() {
        // 引擎警告透传（SC-02 不变式：零提取必带 note）
        for w in &doc.warnings {
            if let Some(last) = components.last_mut() {
                if last.component == "图片" && last.note.is_empty() {
                    last.note = format!("引擎警告 {}: {}", w.code, w.message);
                    continue;
                }
            }
            components.push(ComponentLedger {
                component: format!("引擎警告[{}]", w.code),
                present: 1,
                extracted: 0,
                note: w.message.clone(),
            });
        }
    }

    let parse_summary = format!(
        "engine-docx @ {}｜警告 {} 条｜部件账本 {} 面",
        crate::MDCORE_REV,
        doc.warnings.len(),
        components.len()
    );

    Ok(ParsedFile {
        name: meta.name.clone(),
        sha256: meta.sha256.clone(),
        pages: 0,
        tables: table_idx,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary,
        components,
    })
}

/// xlsx：EngineBook → ParsedFile（B3-4 实装——映射表 §2）。
///
/// 口径与旧实现（`bid-parse/src/xlsx.rs`）逐项对齐（块流等价验收锚）：
/// - 逐 sheet 逐行 → `BlockKind::XlsxRow`；**全空行跳过**；
/// - locator = `{sheet名}!行{N}`（N = 矩阵行位 1 基——旧实现口径）；
/// - text = 非空列单空格连接；cells 直连（值面——公式格 text=缓存值）；
/// - 公式双通道（D1b 口径）：公式文本矩阵非空计数入 components「公式」账本行
///   （present=非空公式格数 / extracted=0 / note=公式文本不占 text 通道）；
/// - pages = tables = sheet 数（xlsx 无页概念——工作表数作「页」）；
/// - merges 透传消费面（D1a/X3——账本 note 登记）。
#[allow(clippy::needless_pass_by_value)]
pub fn xlsx_to_parsed_file(
    book: EngineBook,
    meta: &FileMeta,
) -> Result<ParsedFile, EngineError> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut filled_rows = 0usize;
    let mut total_cells = 0usize;
    let mut formula_cells = 0usize;
    let mut merge_count = 0usize;

    for sheet in &book.sheets {
        for (ri, row) in sheet.values.iter().enumerate() {
            if row.iter().all(|c| c.is_empty()) {
                continue;
            }
            filled_rows += 1;
            total_cells += row.iter().filter(|c| !c.is_empty()).count();
            let text = row
                .iter()
                .filter(|c| !c.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            blocks.push(Block {
                idx: blocks.len(),
                kind: BlockKind::XlsxRow,
                locator: format!("{}!行{}", sheet.name, ri + 1),
                text,
                cells: row.clone(),
            });
        }
        formula_cells += sheet
            .formulas
            .iter()
            .flatten()
            .filter(|f| !f.is_empty())
            .count();
        merge_count += sheet.merges.len();
    }

    let mut components: Vec<ComponentLedger> = Vec::new();
    if formula_cells > 0 {
        components.push(ComponentLedger {
            component: "公式".into(),
            present: formula_cells,
            extracted: 0,
            note: "公式文本独立通道（值面 text=缓存值——D1b 双通道口径）".into(),
        });
    }
    if merge_count > 0 {
        components.push(ComponentLedger {
            component: "合并区".into(),
            present: merge_count,
            extracted: 0,
            note: "merges 透传（D1a/X3 消费面——扁平读回口径）".into(),
        });
    }

    let parse_summary = format!(
        "engine-xlsx @ {}｜{} 工作表，{filled_rows} 有效行，{total_cells} 非空单元格",
        crate::MDCORE_REV,
        book.sheets.len()
    );

    Ok(ParsedFile {
        name: meta.name.clone(),
        sha256: meta.sha256.clone(),
        pages: book.sheets.len().max(1),
        tables: book.sheets.len(),
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary,
        components,
    })
}

/// pdf：EnginePdf → ParsedFile（B3-5 实装——映射表 §3）。
///
/// 口径与旧实现（`bid-parse/src/pdf.rs` 文本层分支）对齐：
/// - 行切分：页文本 `lines()` 逐行 trim，空行跳过（行号只计非空行）；
/// - locator = `第{页号}页 行{行号}`（页号 1 基）；
/// - text = collapse_spaces（空白折叠为单空格——块文本不含双空格）；
/// - tables = 折叠前含双空格的行计数（-layout 列对齐多列表征，粗粒度——D1 面；
///   #324：判定在折叠前，折叠后判恒假是死指标）；
/// - has_text_layer：任一页非空白字符 ≥16（旧实现同阈值——空白/仅页码层判无）；
/// - pages = 引擎实际页数（pdfium 页计数——优于旧 pdftotext 非空分页启发式）；
/// - **扫描件（any_text_layer=false）不在本映射**：OCR 补遗链（pdftoppm +
///   tesseract）是标书侧保留面（映射表 §3 处置列）——调用方回退旧路径。
#[allow(clippy::needless_pass_by_value)]
pub fn pdf_to_parsed_file(
    pdf: EnginePdf,
    meta: &FileMeta,
) -> Result<ParsedFile, EngineError> {
    if !pdf.any_text_layer {
        return Err(EngineError::MappingGap(
            "pdf 无文本层（扫描件）——OCR 补遗链在标书侧旧路径（映射表 §3 处置列）".to_string(),
        ));
    }
    let mut blocks: Vec<Block> = Vec::new();
    // #324（R-12/B-04）：表格行计数在折叠前判定——原实现先 collapse_spaces
    // 再 filter(text.contains("  "))，折叠后双空格恒假 → tables 恒 0（死指标，
    // 与 bid-parse poppler.rs 同型双副本，两处同修）。
    let mut tables = 0usize;
    for (pi, page) in pdf.pages.iter().enumerate() {
        let mut line_no = 0usize;
        for line in page.lines() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            line_no += 1;
            // 折叠前判定（本函数内块全为 PdfLine，kind 过滤等价省略）
            if l.contains("  ") {
                tables += 1;
            }
            blocks.push(Block {
                idx: blocks.len(),
                kind: BlockKind::PdfLine,
                locator: format!("第{}页 行{}", pi + 1, line_no),
                text: collapse_spaces(l),
                cells: vec![],
            });
        }
    }
    let pages = pdf.pages.len().max(1);
    let parse_summary = format!(
        "engine-pdf @ {}｜文本层: {pages} 页 {} 行块（pdfium）",
        crate::MDCORE_REV,
        blocks.len()
    );
    Ok(ParsedFile {
        name: meta.name.clone(),
        sha256: meta.sha256.clone(),
        pages,
        tables,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary,
        components: vec![],
    })
}

/// 空白折叠（**保留双空格**——pdftotext -layout 列对齐表征是 tables 计数锚；
/// 与旧实现 `bid-parse/src/pdf.rs::collapse_spaces` 同款）。
fn collapse_spaces(s: &str) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim_end().to_string()
}

// ---------------------------------------------------------------- 章节模式三助手
// B4 自 `bid-parse/src/docx.rs` 迁入（引擎单一真源化——旧自研解析删除）。
// 既定设计（docx.rs 243-244 行原文）：locator 语义属于**标书面呈现**，
// 非引擎能力；提取层 extract.rs 只消费 locator 字符串。三助手是映射层
// 构建章节上下文的唯一口径（「第X章」/「一、」/「3.2.1」），保持 pub 供
// 测试与潜在消费方复用；迁移后语义零变化（逐字搬运，验收锚：A 件 100% EQ）。

fn cn_num_ok(s: &str) -> bool {
    s.chars().all(|c| "一二三四五六七八九十百".contains(c))
}

/// 「第X章 标题」——章号用中文数字；携带完整标题（类别映射与 GT locator 口径需要全文）。
pub fn as_chapter(t: &str) -> Option<String> {
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

/// 「一、标题」——中文序号节标题（长度短，避免把正文当标题）；携带完整标题。
pub fn as_subsection(t: &str) -> Option<String> {
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

/// 「3.2.1 正文…」——层级条款号开头（数字开头且含点）。
pub fn is_numbered_clause(t: &str) -> bool {
    let head: String = t.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    head.contains('.') && head.len() >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 章节模式三助手（B4 自 bid-parse/src/docx.rs `chapter_subsection_patterns`
    /// 随迁——逐字同断言）。破坏推演：模式误判（如把「3.1.1 正文」当章）→
    /// locator 上下文污染 → 下游类别映射错位 → 红。
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
}
