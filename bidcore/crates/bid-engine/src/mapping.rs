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
///   （`bid_parse::docx::{as_chapter, as_subsection}`——B4 删旧实现时迁入本 crate），
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

    /// 文本框内层块拍平（映射表 §1：TextBox 与正文同构，块序紧随宿主段）。
    fn flatten_textbox(
        tb_blocks: &[EngineBlock],
        blocks: &mut Vec<Block>,
        chapter: &str,
        subsection: &str,
        table_idx: &mut usize,
        emit_para: &dyn Fn(&mut Vec<Block>, &str, &str, &str),
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
                // 章节上下文（旧解析器同款文本模式——extract.rs 类别映射依赖 locator 携带）
                if let Some(c) = bid_parse::docx::as_chapter(&t) {
                    chapter = c;
                    subsection.clear();
                } else if let Some(s) = bid_parse::docx::as_subsection(&t) {
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

/// xlsx：EngineBook → ParsedFile。
///
/// 映射要点：逐 sheet 逐行 → `BlockKind::XlsxRow`（locator=`sheet名 行N`）；
/// 值/公式双通道（公式格 text=值，公式入账本 note——与旧实现 D1b 口径对齐）。
#[allow(clippy::needless_pass_by_value)]
pub fn xlsx_to_parsed_file(
    _book: EngineBook,
    _meta: &FileMeta,
) -> Result<bid_parse::ParsedFile, EngineError> {
    Err(EngineError::MappingGap(
        "xlsx_to_parsed_file: B3 未实装（T1 只定义）".to_string(),
    ))
}

/// pdf：EnginePdf → ParsedFile。
///
/// 映射要点：页文本 → 行块（`BlockKind::PdfLine`——**行切分规则**在 B3 对齐
/// `pdftotext -layout` 口径：P1 定谳非空白字符比 1.000，仅列对齐空白差异，
/// 归一规则入映射表）；`any_text_layer=false` → `has_text_layer=false` +
/// OCR 分支路由（扫描件补遗通道不在引擎面，映射层只判型不跑 OCR）。
#[allow(clippy::needless_pass_by_value)]
pub fn pdf_to_parsed_file(
    _pdf: EnginePdf,
    _meta: &FileMeta,
) -> Result<bid_parse::ParsedFile, EngineError> {
    Err(EngineError::MappingGap(
        "pdf_to_parsed_file: B3 未实装（T1 只定义）".to_string(),
    ))
}
