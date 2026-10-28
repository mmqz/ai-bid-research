//! locator.rs —— 引擎定位器的标书侧镜像（T-1 Evidence 台账的锚点面，issue #11 V3-1）。
//!
//! 形态契约：与 mdcore `crates/evidence/src/locator.rs`（rev 2d01444）**逐字节一致**——
//! 内部标签 `kind` + snake_case 变体名 + `CellCoord` 裸字符串坐标。这是证据记录
//! 可与引擎侧互认的序列化地基，由 [`tests::locator_serde_shapes_pinned_by_literal_json`]
//! 用字面 JSON 钉死（借鉴 mdcore `evidence.rs:274-296` legacy JSON 钉死模式——
//! mdcore 侧缺 Locator 专属 roundtrip 测试，本仓补齐）。
//!
//! 纪律（issue #2 §1 约束 1/2）：
//! - 本模块是**数据形态镜像**，不是算法仿真——不含任何解析/指纹计算逻辑；
//! - bid-domain 不依赖 mdcore；引擎实例 ↔ 本镜像的转换只发生在 bid-engine
//!   （唯一引擎接触面），后续批次接线；
//! - 字段增删 = 引擎契约变更，必须双向审查后同步（与引擎 Cargo.lock rev 一起走
//!   独立 bump PR）。
//!
//! 七变体语义（对齐引擎源注释）：
//! - `XlsxCell`：单元格（值指纹 + 公式指纹双通道——公式变更即使缓存值相同也应失效）；
//! - `XlsxRegion`：闭区间区域（start ≤ end，行优先拼接序的区域指纹）；
//! - `DocxParagraph`：段落序（0 起，仅计段落块）+ 文本指纹（正文改写主判定面）；
//! - `DocxTableCell`：表序 + 行 + 列 + 文本指纹；
//! - `MarkdownBlock`：块 ID 寻址；
//! - `PdfPageText`：1 起页号 + 页文本指纹；
//! - `CanvasObject`：画布对象 ID + 对象语义指纹。

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- 引擎面
/// 定位器所属的文档引擎面（serde 小写——对齐引擎 `refs.rs` EngineKind）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EngineKind {
    #[serde(rename = "docx")]
    Docx,
    #[serde(rename = "xlsx")]
    Xlsx,
    #[serde(rename = "pdf")]
    Pdf,
    #[serde(rename = "md")]
    Markdown,
    #[serde(rename = "canvas")]
    Canvas,
}

// ---------------------------------------------------------------- A1 坐标域
/// A1 记法单元格坐标（newtype——serde 为裸字符串，如 `"C7"`；与引擎 `CellCoord` 同形）。
///
/// 域：列字母 1..=3 个（列值 ≤ 16384/XFD），行号 1..=1_048_576；
/// 大小写规范化为大写存储（`"c7"` → `"C7"`）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellCoord(pub String);

/// 坐标域校验错误（对齐引擎 `LocatorError::BadCellCoord` 语义，仅保留本仓消费面）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocatorError {
    /// A1 记法不合法（空/格式错/越域）
    BadCellCoord,
    /// 区域端点逆序（start 必须先于等于 end：先行后列）
    BadRegion,
}

impl CellCoord {
    /// 解析并校验 A1 记法（borrow：mdcore `locator.rs` `cell_coord_domain` 测试口径）。
    pub fn parse(s: &str) -> Result<Self, LocatorError> {
        let bytes = s.as_bytes();
        if bytes.is_empty() {
            return Err(LocatorError::BadCellCoord);
        }
        // 列字母段：1..=3 个 ASCII 字母（大小写归一大写）
        let mut col: u32 = 0;
        let mut i = 0;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            let c = bytes[i].to_ascii_uppercase();
            col = col * 26 + u32::from(c - b'A' + 1);
            i += 1;
            if i > 3 {
                return Err(LocatorError::BadCellCoord); // >3 列字母（如 AAAA1）
            }
        }
        if i == 0 {
            return Err(LocatorError::BadCellCoord); // 无列字母（如 "1A" / "7"）
        }
        // 行号段：非空纯数字
        let row_part = &s[i..];
        if row_part.is_empty() || !row_part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(LocatorError::BadCellCoord);
        }
        let row: u64 = row_part.parse().map_err(|_| LocatorError::BadCellCoord)?;
        // 域上界：列 ≤ 16384（XFD），行 ≤ 1_048_576
        if col == 0 || col > 16384 || row == 0 || row > 1_048_576 {
            return Err(LocatorError::BadCellCoord);
        }
        Ok(CellCoord(format!("{}{}", col_letters(col), row)))
    }
}

/// 列值 → 字母串（1 → A，26 → Z，27 → AA，16384 → XFD）。
fn col_letters(mut col: u32) -> String {
    let mut out = Vec::new();
    while col > 0 {
        let rem = (col - 1) % 26;
        out.push(b'A' + rem as u8);
        col = (col - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// 区域端点序校验：start ≤ end（先行后列；borrow：mdcore `region_ordering` 口径）。
pub fn validate_region(start: &CellCoord, end: &CellCoord) -> Result<(), LocatorError> {
    match (split_coord(start), split_coord(end)) {
        (Some((sc, sr)), Some((ec, er))) => {
            if (sr, sc) <= (er, ec) {
                Ok(())
            } else {
                Err(LocatorError::BadRegion)
            }
        }
        _ => Err(LocatorError::BadCellCoord),
    }
}

/// 坐标 → (列值, 行值)（已过域校验的串再拆；None = 形态不合法）。
fn split_coord(c: &CellCoord) -> Option<(u32, u64)> {
    let s = c.0.as_str();
    let mut col: u32 = 0;
    let mut i = 0;
    for b in s.bytes() {
        if b.is_ascii_alphabetic() {
            col = col * 26 + u32::from(b.to_ascii_uppercase() - b'A' + 1);
            i += 1;
        } else {
            break;
        }
    }
    let row: u64 = s[i..].parse().ok()?;
    Some((col, row))
}

// ---------------------------------------------------------------- 七变体定位器
/// 引擎定位器（标书侧镜像——serde 形态与引擎 `Locator` 逐字节一致，见模块注释）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineLocator {
    /// XLSX 单元格：值指纹 + 公式指纹（非公式格 = None——区分「无公式」与「公式被删除」）。
    XlsxCell {
        sheet: String,
        cell: CellCoord,
        value_fingerprint: String,
        formula_fingerprint: Option<String>,
    },
    /// XLSX 区域：闭区间（start ≤ end）。
    XlsxRegion {
        sheet: String,
        start: CellCoord,
        end: CellCoord,
        region_fingerprint: String,
    },
    /// DOCX 段落：0 起段落序（仅计段落块，表格块不占号）+ 文本指纹。
    DocxParagraph {
        paragraph_index: u32,
        text_fingerprint: String,
    },
    /// DOCX 表格单元格：表序 + 行 + 列 + 文本指纹。
    DocxTableCell {
        table_index: u32,
        row: u32,
        col: u32,
        text_fingerprint: String,
    },
    /// Markdown 块（块 ID 寻址）。
    MarkdownBlock {
        block_id: String,
        text_fingerprint: String,
    },
    /// PDF 页文本（1 起页号）。
    PdfPageText {
        page: u32,
        text_fingerprint: String,
    },
    /// Canvas 画布对象。
    CanvasObject {
        object_id: String,
        object_fingerprint: String,
    },
}

impl EngineLocator {
    /// 引擎分派面（borrow：mdcore `locator_engine_dispatch` 口径）。
    pub fn engine(&self) -> EngineKind {
        match self {
            EngineLocator::XlsxCell { .. } | EngineLocator::XlsxRegion { .. } => EngineKind::Xlsx,
            EngineLocator::DocxParagraph { .. } | EngineLocator::DocxTableCell { .. } => EngineKind::Docx,
            EngineLocator::MarkdownBlock { .. } => EngineKind::Markdown,
            EngineLocator::PdfPageText { .. } => EngineKind::Pdf,
            EngineLocator::CanvasObject { .. } => EngineKind::Canvas,
        }
    }

    /// 定位器的定位时刻内容指纹（各变体的指纹字段投影——判内容变更的对照面）。
    pub fn fingerprint(&self) -> &str {
        match self {
            EngineLocator::XlsxCell { value_fingerprint, .. } => value_fingerprint,
            EngineLocator::XlsxRegion { region_fingerprint, .. } => region_fingerprint,
            EngineLocator::DocxParagraph { text_fingerprint, .. }
            | EngineLocator::DocxTableCell { text_fingerprint, .. }
            | EngineLocator::MarkdownBlock { text_fingerprint, .. }
            | EngineLocator::PdfPageText { text_fingerprint, .. } => text_fingerprint,
            EngineLocator::CanvasObject { object_fingerprint, .. } => object_fingerprint,
        }
    }
}

// ---------------------------------------------------------------- 指纹格式纪律
/// 内容指纹格式校验：必须带算法前缀（`algo:hex`，borrow：mdcore `refs.rs`
/// `fingerprint_requires_algorithm_prefix` 口径——裸值不可辨识算法，拒绝）。
///
/// 跨算法不可比：`fnv64:aa` 与 `sha256:aa` 是**不同内容指纹**（内容匹配判定
/// 要求算法前缀相同——防算法切换期的误判变更，见 mdcore `content_matches`）。
pub fn fingerprint_algorithm(fp: &str) -> Result<&str, String> {
    let (alg, value) = fp
        .split_once(':')
        .ok_or_else(|| format!("指纹缺算法前缀（裸值不可辨识算法）: {fp:?}"))?;
    if alg.is_empty() || alg.len() > 16 {
        return Err(format!("指纹算法前缀域非法（1..=16 字符）: {alg:?}"));
    }
    if value.is_empty() {
        return Err(format!("指纹值为空: {fp:?}"));
    }
    Ok(alg)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================ 借鉴测试（mdcore evidence crate） ============================

    /// borrow：mdcore `locator.rs::cell_coord_domain`（187-198）——A1 坐标域恰过恰拒 + 大写归一。
    #[test]
    fn cell_coord_domain() {
        // 合法面
        assert_eq!(CellCoord::parse("A1").unwrap().0, "A1");
        assert_eq!(CellCoord::parse("c7").unwrap().0, "C7", "小写输入规范化为大写存储");
        // 域上界恰过：XFD = 16384 列，行 1_048_576
        assert!(CellCoord::parse("XFD1048576").is_ok(), "XFD1048576 是域上界，必须恰过");
        // 域外拒绝
        assert!(CellCoord::parse("XFE1").is_err(), "列超 XFD 拒");
        assert!(CellCoord::parse("A1048577").is_err(), "行超 1048576 拒");
        assert!(CellCoord::parse("A0").is_err(), "0 行拒");
        // 形态拒绝
        assert!(CellCoord::parse("1A").is_err(), "数字开头拒");
        assert!(CellCoord::parse("").is_err(), "空串拒");
        assert!(CellCoord::parse("AAAA1").is_err(), "4 列字母拒（>3）");
    }

    /// borrow：mdcore `locator.rs::region_ordering`（200-209）——区域端点序先行后列。
    #[test]
    fn region_ordering() {
        let a1 = CellCoord::parse("A1").unwrap();
        let b2 = CellCoord::parse("B2").unwrap();
        let a3 = CellCoord::parse("A3").unwrap();
        assert!(validate_region(&a1, &b2).is_ok(), "A1..B2 合法");
        assert!(validate_region(&a1, &a3).is_ok(), "同列跨行合法");
        assert!(validate_region(&b2, &a1).is_err(), "B2..A1 逆序拒（先行后列）");
        assert!(validate_region(&a3, &a1).is_err(), "A3..A1 同列逆序拒");
    }

    /// borrow：mdcore `locator.rs::locator_engine_dispatch`（211-225）——七变体引擎分派。
    #[test]
    fn locator_engine_dispatch() {
        let x = EngineLocator::XlsxCell {
            sheet: "报价表".into(),
            cell: CellCoord::parse("C7").unwrap(),
            value_fingerprint: "fnv64:dd".into(),
            formula_fingerprint: None,
        };
        assert_eq!(x.engine(), EngineKind::Xlsx);
        let p = EngineLocator::DocxParagraph { paragraph_index: 3, text_fingerprint: "fnv64:cc".into() };
        assert_eq!(p.engine(), EngineKind::Docx);
        assert_eq!(
            EngineLocator::PdfPageText { page: 1, text_fingerprint: "fnv64:ff".into() }.engine(),
            EngineKind::Pdf
        );
        assert_eq!(
            EngineLocator::MarkdownBlock { block_id: "b1".into(), text_fingerprint: "fnv64:11".into() }.engine(),
            EngineKind::Markdown
        );
        assert_eq!(
            EngineLocator::CanvasObject { object_id: "o1".into(), object_fingerprint: "fnv64:22".into() }.engine(),
            EngineKind::Canvas
        );
    }

    /// 新增（mdcore 缺失面，研究结论建议补齐）：七变体 serde 形态用**字面 JSON** 钉死——
    /// 内部标签 `kind` + snake_case + CellCoord 裸字符串。形态漂移 = 引擎互认断裂。
    /// 钉死素材：前两形态为 mdcore `evidence.rs:280-289` 测试实证，后五形态由 serde 属性
    /// 确定性推出（与引擎同属性 ⇒ 同形态）。
    #[test]
    fn locator_serde_shapes_pinned_by_literal_json() {
        // XlsxCell（含 formula_fingerprint: null 分支——「无公式」显式化）
        let x = EngineLocator::XlsxCell {
            sheet: "报价表".into(),
            cell: CellCoord::parse("C7").unwrap(),
            value_fingerprint: "fnv64:dd".into(),
            formula_fingerprint: None,
        };
        assert_eq!(
            serde_json::to_string(&x).unwrap(),
            r#"{"kind":"xlsx_cell","sheet":"报价表","cell":"C7","value_fingerprint":"fnv64:dd","formula_fingerprint":null}"#
        );
        // XlsxRegion
        let r = EngineLocator::XlsxRegion {
            sheet: "Sheet1".into(),
            start: CellCoord::parse("A1").unwrap(),
            end: CellCoord::parse("B2").unwrap(),
            region_fingerprint: "fnv64:ab".into(),
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"kind":"xlsx_region","sheet":"Sheet1","start":"A1","end":"B2","region_fingerprint":"fnv64:ab"}"#
        );
        // DocxParagraph（mdcore 实证形态）
        let p = EngineLocator::DocxParagraph { paragraph_index: 3, text_fingerprint: "fnv64:cc".into() };
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"kind":"docx_paragraph","paragraph_index":3,"text_fingerprint":"fnv64:cc"}"#
        );
        // DocxTableCell
        let t = EngineLocator::DocxTableCell { table_index: 0, row: 1, col: 2, text_fingerprint: "fnv64:ee".into() };
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"kind":"docx_table_cell","table_index":0,"row":1,"col":2,"text_fingerprint":"fnv64:ee"}"#
        );
        // MarkdownBlock
        let m = EngineLocator::MarkdownBlock { block_id: "blk-9".into(), text_fingerprint: "fnv64:12".into() };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"kind":"markdown_block","block_id":"blk-9","text_fingerprint":"fnv64:12"}"#
        );
        // PdfPageText
        let f = EngineLocator::PdfPageText { page: 12, text_fingerprint: "fnv64:34".into() };
        assert_eq!(
            serde_json::to_string(&f).unwrap(),
            r#"{"kind":"pdf_page_text","page":12,"text_fingerprint":"fnv64:34"}"#
        );
        // CanvasObject
        let c = EngineLocator::CanvasObject { object_id: "obj-3".into(), object_fingerprint: "fnv64:56".into() };
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"kind":"canvas_object","object_id":"obj-3","object_fingerprint":"fnv64:56"}"#
        );
        // 全变体 roundtrip（序列化 → 反序列化 → 全等）
        for loc in [x, r, p, t, m, f, c] {
            let s = serde_json::to_string(&loc).unwrap();
            let back: EngineLocator = serde_json::from_str(&s).unwrap();
            assert_eq!(back, loc, "roundtrip 必须全等: {s}");
        }
    }

    /// borrow：mdcore `refs.rs::fingerprint_requires_algorithm_prefix`（144-156）+
    /// `content_matches_requires_same_algorithm`（158-182）——算法前缀纪律。
    #[test]
    fn fingerprint_requires_algorithm_prefix() {
        // 无冒号裸值拒（不可辨识算法）
        assert!(fingerprint_algorithm("9ae16a3b2f90ac64").is_err());
        // 合法前缀
        assert_eq!(fingerprint_algorithm("fnv64:9ae16").unwrap(), "fnv64");
        assert_eq!(fingerprint_algorithm("sha256:abcd").unwrap(), "sha256");
        // 空值 / 空算法拒
        assert!(fingerprint_algorithm("fnv64:").is_err(), "空值拒");
        assert!(fingerprint_algorithm(":abcd").is_err(), "空算法拒");
    }
}
