//! XLSX 解析：calamine 读全工作表行。图纸目录/量价清单的读面（写面属 M3+ 文档内核）。
//!
//! T-03 防护边界（诚实登记）：calamine 是黑盒全量内存加载，实际解压流不可注入计量；
//! 本层入口三道防线——输入字节门 + zip prescan 三门 + 压缩比门；
//! 读后双轴后置门（R3-S3-3，有效行/非空单元格总量，M2 缓解提前落地）：超量
//! 显式拒绝不留部分结果 + 下游 block 向量有界。残余面（声明 0 骗过 prescan 后
//! calamine **内部**解压的内存尖峰——后置门在物化之后才判）仍登记 threat-model T-03，
//! 根治需 xlsx 读面改 zip crate 流式自读（M2+）。

use crate::guard::{
    self, LIMIT_INPUT_BYTES_XLSX, LIMIT_XLSX_FILLED_ROWS, LIMIT_XLSX_TOTAL_CELLS,
};
use crate::{Block, BlockKind, ParsedFile};
use calamine::{open_workbook_auto, Data, Reader};
use std::path::Path;

pub fn parse(path: &Path) -> Result<ParsedFile, String> {
    // T-03：输入字节门 + prescan（条目数/单声明/总声明/压缩比/异常名）。
    // 打开后立即丢弃归档句柄——calamine 自行重开；此处只做拒绝裁决。
    guard::open_guarded(path, LIMIT_INPUT_BYTES_XLSX)
        .map_err(|e| format!("zip 防护拒绝: {e}"))?;
    let mut wb = open_workbook_auto(path).map_err(|e| e.to_string())?;
    let mut blocks: Vec<Block> = Vec::new();
    let sheet_names = wb.sheet_names().to_vec();
    let mut filled_rows = 0usize;
    let mut total_cells = 0usize;

    for sheet in &sheet_names {
        let range = wb.worksheet_range(sheet).map_err(|e| e.to_string())?;
        for (ri, row) in range.rows().enumerate() {
            let cells: Vec<String> = row
                .iter()
                .map(|c| match c {
                    Data::Empty => String::new(),
                    Data::String(s) => s.trim().to_string(),
                    Data::Float(f) => {
                        if f.fract() == 0.0 {
                            format!("{}", *f as i64)
                        } else {
                            format!("{f}")
                        }
                    }
                    Data::Int(i) => format!("{i}"),
                    Data::DateTime(d) => d.to_string(),
                    Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
                    Data::Bool(b) => format!("{b}"),
                    Data::Error(e) => format!("<err {e:?}>"),
                })
                .collect();
            if cells.iter().all(|c| c.is_empty()) {
                continue;
            }
            filled_rows += 1;
            total_cells += cells.iter().filter(|c| !c.is_empty()).count();
            // R3-S3-3 后置门：超量即拒（整体失败，不留可被误认完整的部分 blocks）
            if filled_rows > LIMIT_XLSX_FILLED_ROWS {
                return Err(format!(
                    "xlsx 有效行 {filled_rows} 超上限 {LIMIT_XLSX_FILLED_ROWS}（拒绝不留部分结果）"
                ));
            }
            if total_cells > LIMIT_XLSX_TOTAL_CELLS {
                return Err(format!(
                    "xlsx 非空单元格 {total_cells} 超上限 {LIMIT_XLSX_TOTAL_CELLS}（拒绝不留部分结果）"
                ));
            }
            let text = cells
                .iter()
                .filter(|c| !c.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            blocks.push(Block {
                idx: blocks.len(),
                kind: BlockKind::XlsxRow,
                locator: format!("{sheet}!行{}", ri + 1),
                text,
                cells,
            });
        }
    }

    let pages = sheet_names.len().max(1); // 工作表数作「页」（xlsx 无页概念）
    Ok(ParsedFile {
        name: String::new(),
        sha256: String::new(),
        pages,
        tables: sheet_names.len(),
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary: format!("xlsx: {} 工作表，{filled_rows} 有效行，{total_cells} 非空单元格", sheet_names.len()),
        components: vec![], // SC-02 部件账本属 docx 解析面（xlsx 部件概念不适用）
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    /// 字节级构造最小合法 xlsx（calamine 可开）：单表 + N 行高熵 inlineStr 单元格。
    /// 高熵十六进制后缀防压缩比门误伤——样本设计目标是行数门，不是比例门
    /// （与 guard 测试同纪律：负向样本只应触发它要验证的那道门）。
    fn write_xlsx_rows(path: &Path, rows: usize) {
        let f = std::fs::File::create(path).unwrap();
        let mut w = ZipWriter::new(f);
        let opt = SimpleFileOptions::default();
        w.start_file("[Content_Types].xml", opt).unwrap();
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#).unwrap();
        w.start_file("_rels/.rels", opt).unwrap();
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#).unwrap();
        w.start_file("xl/workbook.xml", opt).unwrap();
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S1" sheetId="1" r:id="rId1"/></sheets></workbook>"#).unwrap();
        w.start_file("xl/_rels/workbook.xml.rels", opt).unwrap();
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#).unwrap();
        w.start_file("xl/worksheets/sheet1.xml", opt).unwrap();
        let mut x: u64 = 0x9E3779B97F4A7C15 | ((std::process::id() as u64) << 1) | 1;
        let mut body = String::with_capacity(rows * 72 + 128);
        body.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>");
        for i in 1..=rows {
            x ^= x >> 12;
            x = x.wrapping_mul(0x2545F4914F6CDD1D);
            x ^= x >> 25;
            body.push_str(&format!(
                "<row r=\"{i}\"><c r=\"A{i}\" t=\"inlineStr\"><is><t>N{i}x{x:016x}</t></is></c></row>"
            ));
        }
        body.push_str("</sheetData></worksheet>");
        w.write_all(body.as_bytes()).unwrap();
        w.finish().unwrap();
    }

    /// 字节级构造最小合法 xlsx（calamine 可开）：单表 + rows×cols 高熵 inlineStr
    /// 单元格。行×列双参版——R5 X3 样本设计直采（格轴负向测试专用）：
    /// 行数 < 行门限值，使触发面**只**落在格轴（列数放大格计数）。
    fn write_xlsx_rows_cols(path: &Path, rows: usize, cols: usize) {
        let f = std::fs::File::create(path).unwrap();
        let mut w = ZipWriter::new(f);
        let opt = SimpleFileOptions::default();
        w.start_file("[Content_Types].xml", opt).unwrap();
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#).unwrap();
        w.start_file("_rels/.rels", opt).unwrap();
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#).unwrap();
        w.start_file("xl/workbook.xml", opt).unwrap();
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S1" sheetId="1" r:id="rId1"/></sheets></workbook>"#).unwrap();
        w.start_file("xl/_rels/workbook.xml.rels", opt).unwrap();
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#).unwrap();
        w.start_file("xl/worksheets/sheet1.xml", opt).unwrap();
        // 列引用 A..（cols ≤ 26 内直接字母）；数值单元格 <c r="A1"><v>n</v></c>
        let letters: Vec<char> = ('A'..).take(cols).collect();
        let mut body = String::with_capacity(rows * cols * 24 + 128);
        body.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>");
        for i in 1..=rows {
            body.push_str("<row>");
            for c in &letters {
                body.push_str(&format!("<c r=\"{c}{i}\"><v>{i}</v></c>"));
            }
            body.push_str("</row>");
        }
        body.push_str("</sheetData></worksheet>");
        w.write_all(body.as_bytes()).unwrap();
        w.finish().unwrap();
    }

    #[test]
    fn xlsx_row_flood_rejected_by_post_gate() {
        // R3-S3-3 负向：LIMIT+1 行真实结构 xlsx（calamine 真解析，非 mock）→
        // 后置行门显式拒绝，无部分结果。
        let d = std::env::temp_dir().join("bidparse_xlsx_test");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("flood_{}.xlsx", std::process::id()));
        write_xlsx_rows(&p, LIMIT_XLSX_FILLED_ROWS + 1);
        let err = parse(&p).unwrap_err();
        assert!(err.contains("有效行") && err.contains("超上限"), "{err}");
    }

    /// 格轴负向（SC-04，R5 X3 直采）：95,239 行 × 21 列 = 2,000,019 非空格
    /// ——行数 95,239 < 行门 200,000（行门静默），格计数在末行破 200 万 →
    /// **格轴**显式拒绝。此前仓库只有行轴负向（R5 P-1 缺口）——本测试
    /// 独立证明格门真实生效且「拒绝不留部分结果」。
    /// 破坏推演：删除 total_cells 门 → parse Ok（blocks≈95,239）→ unwrap_err 红；
    /// 误改判据为行门复用（filled_rows 超限才拒）→ 95,239 行不触发行门 → Ok → 红。
    /// 上游先拒：无——guard 面输入字节/条目数/声明/压缩比均放行（样本设计只触格门）。
    #[test]
    fn xlsx_cell_flood_rejected_by_post_gate_cell_axis() {
        let d = std::env::temp_dir().join("bidparse_xlsx_test");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("cellflood_{}.xlsx", std::process::id()));
        let rows = (LIMIT_XLSX_TOTAL_CELLS / 21) + 1; // 95,239 行 → 恰在末行触发
        assert!(rows < LIMIT_XLSX_FILLED_ROWS, "样本前提：行门不先触发");
        write_xlsx_rows_cols(&p, rows, 21);
        let err = parse(&p).unwrap_err();
        assert!(err.contains("非空单元格") && err.contains("2000019") && err.contains("超上限"),
            "格轴门拒绝（非行门）：{err}");
        assert!(!err.contains("有效行"), "不得误触发行轴：{err}");
    }

    #[test]
    fn xlsx_normal_file_passes_post_gate() {
        // R3-S3-3 正向对照：百行级真实结构 xlsx 正常解析（防“修门即全拒”）
        let d = std::env::temp_dir().join("bidparse_xlsx_test");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("ok_{}.xlsx", std::process::id()));
        write_xlsx_rows(&p, 100);
        let pf = parse(&p).unwrap();
        assert_eq!(pf.blocks.len(), 100);
        assert!(pf.blocks[0].text.starts_with("N1"), "{}", pf.blocks[0].text);
        assert!(pf.parse_summary.contains("100 有效行"));
    }
}
