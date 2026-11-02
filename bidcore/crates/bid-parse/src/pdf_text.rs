//! pdf_text.rs —— 纯 Rust 最小 PDF 文本提取器（补遗降级路）。
//!
//! 背景（实测根因）：导入 pdf 的现有链路是双路回退——
//! pdfium 引擎 → pdftotext（poppler）。但在未装 pdfium 且未装 poppler 的
//! 环境里两条路全断：首选路 EngineFailed，回退路 program not found。
//! 后果不是「读不出来」那么简单——补遗文件（招标文件的修正案，
//! 法律上优先于原件）会静默丢失，后续抽取/建纲/起草全部基于旧文件，
//! 而校验仍然 blocker=0（因为系统不知道补遗存在）。
//!
//! 本模块是第三条降级路：不依赖任何外部二进制的最小文本提取。
//!
//! 甜区（不夸大，如实登记）：覆盖 ASCII85/ASCIIHex/RunLength/FlateDecode 四种滤链；
//! **ToUnicode CMap**（bfchar/bfrange，Identity-H 类字体——国内招标文件最普遍形态）；
//! **UniXX-UCS2 预定义 CMap 恒等解码**（#200：字体声明 /UniGB-UCS2-H 等
//! 预定义 CMap 时 2 字节码即 UCS-2 值，无 ToUnicode 也能精确解码——按 Tf
//! 字体上下文生效，逐字 Tj 的单字串不再落 Latin-1 分支）；
//! Tm/CTM 定位行合成（逐字 Tj 能拼回成行）；文本层字面串
//! 与十六进制串；Tj / TJ / ''' / " 四种显示运算符；字符串编码
//! UTF-16BE（带 BOM 或高字节 0x00 特征）与 WinAnsi/PDFDocEncoding（字节直取）。
//! #200 剩余能力边界（如实登记——遇到时错误信息会点名原因，见
//! [`cjk_garble_reason_hint`]；不硬解，因为硬解会产出垃圾文本，比失败更危险）：
//!
//! | 边界形态 | 根因 | 为什么不硬解 | 绕过 |
//! |---|---|---|---|
//! | GBK-EUC-H/V、GB-EUC、GBpc-EUC（简体） | 预定义 CMap 的 2 字节码是 GBK/GB2312 码值，**不是 Unicode 码位** | 恒等解码（2 字节码=UCS-2 值）只对 UniXX-UCS2 系列成立；正确解码需内置 GBK↔Unicode 映射大表（数万条目） | ① 装 pdfium ② 装 poppler+poppler-data ③ Word/WPS 另存为 DOCX 导入 |
//! | B5pc-H/V、ETen-B5、CNS-EUC（繁体） | 同上，Big5 码值非 Unicode | 同上（需 Big5↔Unicode 大表） | 同上 |
//! | 真 Adobe-GB1/CNS1/Japan1/Korea1 CID 序号（如 Identity-H + CIDSystemInfo (Adobe)(GB1)，无 ToUnicode） | 码值是 Adobe 注册表**序号**（字形集内编号），与 Unicode 无恒等关系 | 需 Adobe-GB1↔Unicode 映射大表（分 Supplement 版本，数万条目） | 同上 |
//! | PDF 1.5+ 对象流（/ObjStm） | 字体字典被压缩进对象流，本模块的字典层字节扫描**扫不到**编码声明 → UniXX-UCS2 恒等解码资格判定失效，可解形态也退回启发式 | 解压 ObjStm 需完整 xref 流解析（超出最小提取器定位） | 同上（首选路 pdfium/poppler 不受影响） |
//!
//! 不覆盖（其他）：加密 PDF、LZW 滤链、编码表自定义的 CID 字体、跨页复杂
//! 排版重组。这些情形一律明说失败（返回 Err / 触发 CJK 乱码探测 fail-loud），
//! 绝不输出半截乱码——因为静默的错误文本比明说读不出来危险得多。
//!
//! 定位：这是输入层的兜底——只负责把字节变成行文本，不做条款分类
//! （那是 extract.rs 的职责）。上游一旦有 pdfium 或 poppler，就不应走到这里。

use flate2::read::ZlibDecoder;
use std::io::Read;

/// 最小 PDF 文本提取失败原因（全部是「明说失败」，无静默降级通道）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfTextError {
    /// 不是合法 PDF（缺 %PDF- 头）。
    NotPdf,
    /// 加密 PDF——本模块不解密。
    Encrypted,
    /// 读了所有流，但一个可显示字串都没抽到（典型：扫描件无文本层）。
    NoTextLayer,
    /// 流解压失败且无任何可用流（非支持的滤链）。
    Unsupported(String),
}

impl std::fmt::Display for PdfTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdfTextError::NotPdf => write!(f, "不是合法 PDF（缺 %PDF- 头）"),
            PdfTextError::Encrypted => {
                write!(f, "加密 PDF（纯 Rust 兜底不解密，需 pdfium/poppler）")
            }
            PdfTextError::NoTextLayer => {
                write!(f, "PDF 无文本层（疑扫描件——需 OCR 链）")
            }
            PdfTextError::Unsupported(why) => write!(f, "PDF 流解压不支持：{why}"),
        }
    }
}

impl std::error::Error for PdfTextError {}

/// 提取结果（按页分组的文本行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfText {
    /// 逐页的文本行（顺序：流出现序 → 行出现序）。
    pub pages: Vec<Vec<String>>,
    /// 成功解压并抽取到字串的流数（审计用）。
    pub streams_decoded: usize,
    /// 尝试过但解压/抽取失败的流数（诚实登记）。
    pub streams_skipped: usize,
}

impl PdfText {
    /// 全部文本行（跨页拉平）。
    pub fn lines(&self) -> Vec<String> {
        self.pages.iter().flat_map(|p| p.iter().cloned()).collect()
    }

    /// 是否抽到了非空文本。
    pub fn has_text(&self) -> bool {
        self.pages
            .iter()
            .any(|p| p.iter().any(|l| !l.trim().is_empty()))
    }

    /// 可供人读的摘要（进 parse_summary）。
    pub fn summary(&self) -> String {
        format!(
            "纯 Rust PDF 文本兜底：{} 页、{} 行、{} 流解码成功、{} 流跳过",
            self.pages.len(),
            self.pages.iter().map(|p| p.len()).sum::<usize>(),
            self.streams_decoded,
            self.streams_skipped
        )
    }
}

// ---------------------------------------------------------------- 主入口

/// 从 PDF 字节提取文本行。
///
/// 纯函数、零 IO、不依赖外部二进制；同输入同输出（可重放）。
pub fn extract(bytes: &[u8]) -> Result<PdfText, PdfTextError> {
    if !looks_like_pdf(bytes) {
        return Err(PdfTextError::NotPdf);
    }
    // 加密不解（宁可失败也不输出乱码）。
    if contains(bytes, b"/Encrypt") {
        return Err(PdfTextError::Encrypted);
    }

    // 先合并全文件里所有 ToUnicode CMap。
    //
    // 合并而不是按字体分别查：实务里同一文档的 CID 编码在各字体间
    // 通常不冲突（都从 0 开始编），合并后命中率更高。真正冲突的个例
    // 宁可少映射几个字（下游看得出缺字），也不把读不到的字编成错字。
    let cmap = collect_cmaps(bytes);
    // #200：UniXX-UCS2 字体上下文——按字体决定恒等解码资格（见 FontEncodings）。
    let fonts = collect_font_encodings(bytes);
    // #218：按字体路由的专属 CMap（多字体 CID 重叠时全局合并会错字；
    // 未能路由的字体回退 merged——行为与旧版一致）。
    let fcmaps = FontCMaps {
        maps: collect_font_cmaps(bytes),
        merged: cmap,
    };

    let mut pages: Vec<Vec<String>> = Vec::new();
    let mut streams_decoded = 0usize;
    let mut streams_skipped = 0usize;
    let mut saw_unsupported: Option<String> = None;

    for (data, dict_filters) in iter_streams(bytes) {
        let decoded = match decode_stream(data, &dict_filters) {
            Some(d) => d,
            None => {
                streams_skipped += 1;
                if saw_unsupported.is_none() {
                    saw_unsupported = Some(format!("Filter {:?}", dict_filters));
                }
                continue;
            }
        };
        let lines = extract_text_lines(&decoded, &fcmaps, &fonts);
        if lines.is_empty() {
            // 解压成功但无字串（比如字体/图形流——正常）：不计跳过。
            streams_decoded += 1;
            continue;
        }
        streams_decoded += 1;
        pages.push(lines);
    }

    let out = PdfText {
        pages,
        streams_decoded,
        streams_skipped,
    };
    if out.has_text() {
        return Ok(out);
    }
    if let Some(why) = saw_unsupported {
        return Err(PdfTextError::Unsupported(why));
    }
    Err(PdfTextError::NoTextLayer)
}

/// 收集全文件里所有 `/ToUnicode N 0 R` 指向的 CMap 并合并。
///
/// 只认「`/ToUnicode` 引用 → 对象号 → 对象流解压」这一条短路径，不做完整 xref 解析——
/// 对本模块的定位（短文本 PDF 兜底）正合适，也不会因为 xref 坏了就整体失败。
fn collect_cmaps(bytes: &[u8]) -> CMap {
    let mut merged = CMap::default();
    for cm in collect_cmaps_inner(bytes) {
        for (k, v) in cm.single {
            merged.single.entry(k).or_insert(v);
        }
        for (k, v) in cm.range_map {
            merged.range_map.entry(k).or_insert(v);
        }
    }
    merged
}

fn collect_cmaps_inner(bytes: &[u8]) -> Vec<CMap> {
    let key: &[u8] = b"/ToUnicode";
    let mut obj_nums: Vec<u64> = Vec::new();
    let mut k = 0usize;
    while let Some(p) = find_from(bytes, k, key) {
        let mut j = p + key.len();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut num = String::new();
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            num.push(bytes[j] as char);
            j += 1;
        }
        if let Ok(n) = num.parse::<u64>() {
            obj_nums.push(n);
        }
        k = p + key.len();
    }

    let mut out = Vec::new();
    for n in obj_nums {
        if let Some(body) = object_stream_bytes(bytes, n) {
            let text = String::from_utf8_lossy(&body).to_string();
            let cm = CMap::parse(&text);
            if !cm.is_empty() {
                out.push(cm);
            }
        }
    }
    out
}

// ---------------------------------------------------------------- UniXX-UCS2 字体上下文（issue #200）

/// UniXX-UCS2 预定义 CMap 名前缀（H/V 变体语义相同：2 字节码 = UCS-2 值）。
///
/// 依据 PDF 规范：`UniGB-UCS2-H` 等「Unicode UCS-2」系列预定义 CMap 把
/// 2 字节码**恒等**映射为同值 Unicode——这正是 ReportLab 等生成器
/// 「非嵌入 CID 字体（STSong-Light）」的编码声明方式，因此**没有
/// ToUnicode 也能精确解码**，不靠启发式猜。
const UCS2_CMAP_MARKERS: &[&[u8]] = &[b"UniGB-UCS2", b"UniCNS-UCS2", b"UniJIS-UCS2", b"UniKS-UCS2"];

/// 字体资源名（内容流 `Tf` 运算符的操作数，如 `F2`）→ 是否声明了
/// UniXX-UCS2 预定义 CMap。
///
/// 安全前提（为什么必须按字体而非按文档判定）：恒等解码只能作用于
/// 声明了该编码的字体。拉丁字体（Helvetica 等）的 2 字节 ASCII 串
/// （如 `kV` = 0x6B56 恰落 CJK 区）若被误按 UCS-2 恒等解码会变成
/// 错字「敖」——字体上下文把这扇门只对声明了 UniXX-UCS2 的字体打开。
#[derive(Debug, Default)]
struct FontEncodings {
    uc2_fonts: std::collections::HashSet<String>,
}

impl FontEncodings {
    fn is_uc2(&self, name: &str) -> bool {
        self.uc2_fonts.contains(name)
    }
}

/// 全部 `N 0 obj` 对象头（" 0 obj" 匹配位置, 对象号），位置有序。
///
/// 共享扫描器（#200 R1 重构：原 collect_font_encodings 内联的扫描逻辑抽出，
/// CID 在用探测与 uc2 收集共用同一套对象头/引用扫描口径）。
fn scan_obj_heads(bytes: &[u8]) -> Vec<(usize, u64)> {
    let mut heads: Vec<(usize, u64)> = Vec::new();
    let mut k = 0usize;
    while let Some(p) = find_from(bytes, k, b" 0 obj") {
        let mut j = p;
        while j > 0 && bytes[j - 1].is_ascii_digit() {
            j -= 1;
        }
        if j < p {
            let num = std::str::from_utf8(&bytes[j..p]).unwrap_or("");
            if let Ok(n) = num.parse::<u64>() {
                heads.push((p, n));
            }
        }
        k = p + 6;
    }
    heads
}

/// 全部 `/资源名 N 0 R` 间接引用（资源名, 对象号）。
fn scan_name_refs(bytes: &[u8]) -> Vec<(String, u64)> {
    let mut refs: Vec<(String, u64)> = Vec::new();
    let mut k = 0usize;
    while let Some(p) = find_from(bytes, k, b" 0 R") {
        let mut j = p;
        while j > 0 && bytes[j - 1].is_ascii_digit() {
            j -= 1;
        }
        if j < p {
            let num = std::str::from_utf8(&bytes[j..p]).unwrap_or("");
            if let Ok(n) = num.parse::<u64>() {
                let mut m = j;
                while m > 0 && bytes[m - 1].is_ascii_whitespace() {
                    m -= 1;
                }
                let mut s = m;
                while s > 0
                    && (bytes[s - 1].is_ascii_alphanumeric()
                        || bytes[s - 1] == b'-'
                        || bytes[s - 1] == b'+'
                        || bytes[s - 1] == b'_')
                {
                    s -= 1;
                }
                if s < m && s > 0 && bytes[s - 1] == b'/' {
                    let name = std::str::from_utf8(&bytes[s..m]).unwrap_or("");
                    if !name.is_empty() {
                        refs.push((name.to_string(), n));
                    }
                }
            }
        }
        k = p + 4;
    }
    refs
}

/// 收集「资源名 → UniXX-UCS2 字体」映射（#200 恒等解码的字体资格面）。
///
/// 能力边界（如实登记）：PDF 1.5+ 对象流（ObjStm 压缩存放的字体字典）
/// 扫不到 → 映射为空 → 退回既有启发式行为（fail-safe，不劣化）。
fn collect_font_encodings(bytes: &[u8]) -> FontEncodings {
    let heads = scan_obj_heads(bytes);
    let mut uc2_objs: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for marker in UCS2_CMAP_MARKERS {
        let mut k = 0usize;
        while let Some(p) = find_from(bytes, k, marker) {
            if let Some(&(_, n)) = heads.iter().rev().find(|(pos, _)| *pos < p) {
                uc2_objs.insert(n);
            }
            k = p + marker.len();
        }
    }
    let mut fonts = FontEncodings::default();
    for (name, n) in scan_name_refs(bytes) {
        if uc2_objs.contains(&n) {
            fonts.uc2_fonts.insert(name);
        }
    }
    fonts
}

// ---------------------------------------------------------------- 按字体路由的 ToUnicode CMap（#218）

/// 资源名（Tf 操作数）→ 字体专属 ToUnicode CMap + 全局合并表（回退）。
///
/// #218 背景：全局合并所有字体的 CMap 在 **CID 空间重叠**时错字——实测
/// 水利施工标段（9 个 Identity-H 字体）：/ToUnicode 出现序 Calibri 先于
/// SimSun，合并表 or_insert 先到先得 → Calibri 的 CID 0x0011→'B' 抢占，
/// SimSun 的 '.'（同 CID 0x0011）全部错成 'B'——'4.2' 变 '4B2'、目录
/// 点线成 'BBB…'。Identity-H 的 CID 是**字体各自的字形索引**，同号
/// 不同字是常态——解码必须按 Tf 当前字体查各自的表。
#[derive(Debug, Default)]
struct FontCMaps {
    /// 资源名（如 FT14）→ 该字体的 CMap。
    maps: std::collections::HashMap<String, CMap>,
    /// 全局合并表：字体未登记（无 ToUnicode / ObjStm 扫不到 / 资源名
    /// 缺失）时的回退——保持 #218 之前的行为。
    merged: CMap,
}

impl FontCMaps {
    /// 当前字体的解码表（未登记 → merged 回退）。
    fn of(&self, font: Option<&str>) -> &CMap {
        match font.and_then(|f| self.maps.get(f)) {
            Some(cm) => cm,
            None => &self.merged,
        }
    }
}

/// 收集「资源名 → 字体专属 ToUnicode CMap」映射。
///
/// 三步扫描（与 collect_font_encodings 同款「字典层字节扫描」；ObjStm
/// 压缩字体字典扫不到 → 该字体不登记 → 运行时回退全局合并表，不劣化）：
///   1. 全部 `N 0 obj` 对象头（位置有序）；
///   2. 每个 `/ToUnicode N 0 R` 向前找最近对象头 M（= 所在字典的对象号，
///      即字体对象）→ 解压对象 N → CMap::parse 得字体 M 的表；
///   3. 扫全部 `/资源名 M 0 R` 间接引用，M 在字体集合 → 记资源名 → 表。
fn collect_font_cmaps(bytes: &[u8]) -> std::collections::HashMap<String, CMap> {
    // 步骤 1：对象头（collect_font_encodings 同款——" 0 obj" 前扫完整数字串）。
    let mut heads: Vec<(usize, u64)> = Vec::new();
    let mut k = 0usize;
    while let Some(p) = find_from(bytes, k, b" 0 obj") {
        let mut j = p;
        while j > 0 && bytes[j - 1].is_ascii_digit() {
            j -= 1;
        }
        if j < p {
            let num = std::str::from_utf8(&bytes[j..p]).unwrap_or("");
            if let Ok(n) = num.parse::<u64>() {
                heads.push((p, n));
            }
        }
        k = p + 6;
    }

    // 步骤 2：/ToUnicode → 就近对象头 = 字体对象；ToUnicode 对象解压成表。
    let mut obj_cmap: std::collections::HashMap<u64, CMap> = std::collections::HashMap::new();
    let mut k = 0usize;
    while let Some(p) = find_from(bytes, k, b"/ToUnicode") {
        let mut j = p + b"/ToUnicode".len();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut num = String::new();
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            num.push(bytes[j] as char);
            j += 1;
        }
        if let (Ok(tu_num), Some(&(_, font_obj))) = (
            num.parse::<u64>(),
            heads.iter().rev().find(|(pos, _)| *pos < p),
        ) {
            // 同一字体对象的重复 /ToUnicode 只认首个。
            if !obj_cmap.contains_key(&font_obj) {
                if let Some(body) = object_stream_bytes(bytes, tu_num) {
                    let text = String::from_utf8_lossy(&body).to_string();
                    let cm = CMap::parse(&text);
                    if !cm.is_empty() {
                        obj_cmap.insert(font_obj, cm);
                    }
                }
            }
        }
        k = p + b"/ToUnicode".len();
    }
    if obj_cmap.is_empty() {
        return std::collections::HashMap::new();
    }

    // 步骤 3：`/资源名 M 0 R` 引用 → M 有表 → 记资源名（首个赢）。
    let mut maps: std::collections::HashMap<String, CMap> = std::collections::HashMap::new();
    let mut k = 0usize;
    while let Some(p) = find_from(bytes, k, b" 0 R") {
        let mut j = p;
        while j > 0 && bytes[j - 1].is_ascii_digit() {
            j -= 1;
        }
        if j < p {
            let num = std::str::from_utf8(&bytes[j..p]).unwrap_or("");
            if let Ok(n) = num.parse::<u64>() {
                if obj_cmap.contains_key(&n) {
                    // 再往前跳空白，应见到 `/资源名`（同款词法）。
                    let mut m = j;
                    while m > 0 && bytes[m - 1].is_ascii_whitespace() {
                        m -= 1;
                    }
                    let mut s = m;
                    while s > 0
                        && (bytes[s - 1].is_ascii_alphanumeric()
                            || bytes[s - 1] == b'-'
                            || bytes[s - 1] == b'+'
                            || bytes[s - 1] == b'_')
                    {
                        s -= 1;
                    }
                    if s < m && s > 0 && bytes[s - 1] == b'/' {
                        let name = std::str::from_utf8(&bytes[s..m]).unwrap_or("");
                        if !name.is_empty() && !maps.contains_key(name) {
                            if let Some(cm) = obj_cmap.get(&n) {
                                maps.insert(name.to_string(), cm.clone());
                            }
                        }
                    }
                }
            }
        }
        k = p + 4;
    }
    maps
}
/// 取 `N 0 obj` 对象流的字节（若有 Filter 则先解压）。
///
/// #218 修复——对象定位三重防护（此前「单匹配 + 无界 stream 搜索」两处踩坑）：
/// 1. **对象头边界**：`N 0 obj` 前一字节不能是数字——否则 "9 0 obj" 会命中
///    "79 0 obj" 的尾部拿错对象。实测事故（水利施工标段 PDF）：ToUnicode
///    引用的对象 9/15/25/30/400 全部被替换成文档前部的同尾号对象（Link
///    注释/页字典），FlateDecode 解压出错误数据 → CMap 解析为空 →
///    Identity-H 码值直出 → 全文乱码、抽取 0 条；
/// 2. **本对象定界**：stream 关键字必须在本对象的 endobj 之前——对象 N 若
///    是非流对象（字典/注释），不能跨对象去拿别人的流；
/// 3. **stream 词边界**：匹配处前一字符不能是字母——"endstream" 的尾部
///    子串不是 stream 关键字（iter_streams 同款防护）。
/// 同号多处匹配（增量更新）按序尝试，取第一个能解开流数据的。
fn object_stream_bytes(bytes: &[u8], num: u64) -> Option<Vec<u8>> {
    let head = format!("{num} 0 obj");
    let mut k = 0usize;
    while let Some(p) = find_from(bytes, k, head.as_bytes()) {
        // 防护 1：对象头前一字节非数字（"9 0 obj" ≠ "79 0 obj" 的尾部）。
        if p > 0 && bytes[p - 1].is_ascii_digit() {
            k = p + head.len();
            continue;
        }
        // 防护 2：本对象的 endobj（找不到时按文件尾处理——容忍损坏文件）。
        let endobj = find_from(bytes, p, b"endobj").unwrap_or(bytes.len());
        // 防护 2+3：本对象范围内找 stream 关键字（前一字符非字母——排除
        // "endstream" 尾部子串）。
        let st = {
            let mut j = p + head.len();
            let mut hit = None;
            while let Some(q) = find_from(bytes, j, b"stream") {
                if q >= endobj {
                    break; // 本对象内无 stream → 非流对象
                }
                let prev = if q > 0 { bytes[q - 1] } else { b'\n' };
                if !prev.is_ascii_alphabetic() {
                    hit = Some(q);
                    break;
                }
                j = q + 6;
            }
            hit
        };
        let Some(st) = st else {
            // 该匹配不是流对象（Link 注释/页字典等）→ 试下一个同号匹配。
            k = p + head.len();
            continue;
        };
        // 数据段：stream 后 EOL 到 endstream 前 EOL。
        let mut data_start = st + 6;
        if data_start < bytes.len() && bytes[data_start] == b'\r' {
            data_start += 1;
        }
        if data_start < bytes.len() && bytes[data_start] == b'\n' {
            data_start += 1;
        }
        let Some(end) = find_from(bytes, data_start, b"endstream") else {
            k = p + head.len();
            continue;
        };
        let mut data_end = end;
        while data_end > data_start
            && (bytes[data_end - 1] == b'\n' || bytes[data_end - 1] == b'\r')
        {
            data_end -= 1;
        }
        let dict_lo = find_dict_start(bytes, st);
        let filt = parse_filters(&bytes[dict_lo..st]);
        if let Some(d) = decode_stream(&bytes[data_start..data_end], &filt) {
            return Some(d);
        }
        // 解压失败（Filter 不认识/数据损坏）→ 试下一个同号匹配（增量更新
        // 场景：旧版对象坏了，新版对象可能是好的）。
        k = p + head.len();
    }
    None
}

// ---------------------------------------------------------------- 字节层小工具

fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes.windows(5).take(1024).any(|w| w == b"%PDF-")
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    hay.windows(needle.len()).any(|w| w == needle)
}

/// 找到每个 stream 的数据段，并抽出它的 /Filter 链。
///
/// 返回 (stream 数据, 过滤器名列表)。过滤器名从 stream 前面的
/// 字典里读（只认识常见的四个名字）。
fn iter_streams(bytes: &[u8]) -> Vec<(&[u8], Vec<String>)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 6 <= bytes.len() {
        // 定位 "stream"
        let Some(pos) = find_from(bytes, i, b"stream") else {
            break;
        };
        // 前一个字符不能是字母（避开 endstream 里的 stream）
        let prev = if pos > 0 { bytes[pos - 1] } else { b'\n' };
        if prev.is_ascii_alphabetic() {
            // 「endstream」里的 stream 子串——跳过它继续找下一个真 stream。
            // 这里必须 +6 而不能跳到 endstream 之后：两者结果相同，
            // 但 +6 不依赖 endstream 存在。
            i = pos + 6;
            continue;
        }
        // 跳过 "stream" 后的 EOL（\r\n 或 \n）
        let mut start = pos + 6;
        if start < bytes.len() && bytes[start] == b'\r' {
            start += 1;
        }
        if start < bytes.len() && bytes[start] == b'\n' {
            start += 1;
        }
        let Some(end_rel) = find_from(bytes, start, b"endstream") else {
            break;
        };
        let mut data_end = end_rel;
        // 剎掉末尾 EOL
        while data_end > start && (bytes[data_end - 1] == b'\n' || bytes[data_end - 1] == b'\r') {
            data_end -= 1;
        }
        // 从 stream 之前的字典里抽 /Filter。
        let dict_lo = find_dict_start(bytes, pos);
        let filt = parse_filters(&bytes[dict_lo..pos]);
        out.push((&bytes[start..data_end], filt));
        i = end_rel + 9;
    }
    out
}

fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= hay.len() || hay.len() - from < needle.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// 回退到 "<</" 或 开头，作为字典区起点（只用于读 /Filter）。
fn find_dict_start(bytes: &[u8], stream_pos: usize) -> usize {
    // 取**自己这个对象**的字典：从 stream 位置往回掃找最后一个「<<」，
    // 但切断于最近一次 endobj / endstream——再往前就是别人的对象了。
    //
    // 实测两个坑都在这里：
    // 1. 不切断 → 把上一个流的 /Filter 也括进来（读成四个过滤器，解压必失败）；
    // 2. 只取最后一个 << 不切断 → 拿到下一个对象的字典。
    let lo = stream_pos.saturating_sub(4096);
    let window = &bytes[lo..stream_pos];

    // 先定义可用的起点：最近一次 endobj/endstream 之后。
    let mut floor = 0usize;
    if let Some(p) = rfind(window, b"endobj") {
        floor = floor.max(p + 6);
    }
    if let Some(p) = rfind(window, b"endstream") {
        floor = floor.max(p + 9);
    }

    match rfind(&window[floor..], b"<<") {
        Some(p) => lo + floor + p,
        None => lo + floor,
    }
}

/// 从后往前找子串（rfind）。
fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).rposition(|w| w == needle)
}

fn parse_filters(dict: &[u8]) -> Vec<String> {
    let names = [
        "ASCII85Decode",
        "FlateDecode",
        "ASCIIHexDecode",
        "LZWDecode",
        "RunLengthDecode",
    ];
    let mut found: Vec<(usize, String)> = Vec::new();
    for name in names {
        let nb = name.as_bytes();
        let mut k = 0usize;
        while let Some(p) = find_from(dict, k, nb) {
            found.push((p, name.to_string()));
            k = p + nb.len();
        }
    }
    found.sort_by_key(|(p, _)| *p);
    found.into_iter().map(|(_, n)| n).collect()
}

/// 按 /Filter 链逐层解码；任一层不支持 → None。
fn decode_stream(data: &[u8], filters: &[String]) -> Option<Vec<u8>> {
    // 无 Filter 声明时探测：%s头就是 Flate（实务里最常见）。
    let eff: Vec<&str> = if filters.is_empty() {
        if data.starts_with(&[0x78]) {
            vec!["FlateDecode"]
        } else {
            vec![]
        }
    } else {
        filters.iter().map(|s| s.as_str()).collect()
    };

    let mut cur = data.to_vec();
    for f in eff {
        cur = match f {
            "ASCII85Decode" => ascii85_decode(&cur)?,
            "FlateDecode" => inflate(&cur)?,
            "ASCIIHexDecode" => ascii_hex_decode(&cur)?,
            "RunLengthDecode" => run_length_decode(&cur)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut dec = ZlibDecoder::new(data);
    dec.read_to_end(&mut out).ok()?;
    Some(out)
}

/// ASCIIHexDecode：两位十六进制 → 一字节；`>` 为结束符，非十六进制字符忽略。
///
/// 奇数个十六进制位时**末位补 0**（PDF 7.3.4.3 规定）。
pub fn ascii_hex_decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut hex: Vec<u8> = Vec::with_capacity(data.len());
    for &b in data {
        if b == b'>' {
            break;
        }
        if b.is_ascii_hexdigit() {
            hex.push(b);
        }
    }
    let mut out = Vec::with_capacity(hex.len() / 2 + 1);
    let mut k = 0usize;
    while k + 1 < hex.len() {
        let hi = hex_val(hex[k])?;
        let lo = hex_val(hex[k + 1])?;
        out.push((hi << 4) | lo);
        k += 2;
    }
    if k < hex.len() {
        out.push(hex_val(hex[k])? << 4);
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// RunLengthDecode（PDF 7.4.5）。
///
/// 首字节 n：0..=127 表示接下来 n+1 个字节原样拷贝；
/// 129..=255 表示接下来的 1 个字节重复 257-n 次；128 为结束。
pub fn run_length_decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut k = 0usize;
    while k < data.len() {
        let n = data[k];
        k += 1;
        if n == 128 {
            break;
        }
        if n < 128 {
            let len = n as usize + 1;
            if k + len > data.len() {
                out.extend_from_slice(&data[k..]);
                break;
            }
            out.extend_from_slice(&data[k..k + len]);
            k += len;
        } else {
            if k >= data.len() {
                break;
            }
            let byte = data[k];
            k += 1;
            let reps = 257 - n as usize;
            out.extend(std::iter::repeat(byte).take(reps));
        }
    }
    Some(out)
}

/// ASCII85 解码（Adobe 变体：结尾 ~>，全零组用 z）。
///
/// 遇到非法字符就 **跳过而不报错**——PDF 实务里 ASCII85 换行不定长，
/// 且有时嵌在 XML/ObjStm 里带额外空白。结果不对的情形由上游 inflate 失败兜住
/// → 流计入 skipped（不会冒充成文本）。
pub fn ascii85_decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    let mut group: Vec<u8> = Vec::with_capacity(5);
    let mut seen_eod = false;

    for &b in data {
        if b == b'~' {
            seen_eod = true;
            break;
        }
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'z' && group.is_empty() {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        if !(b'!'..=b'u').contains(&b) {
            continue;
        }
        group.push(b - 33);
        if group.len() == 5 {
            let mut v: u32 = 0;
            for &g in &group {
                v = v.wrapping_mul(85).wrapping_add(g as u32);
            }
            out.extend_from_slice(&v.to_be_bytes());
            group.clear();
        }
    }
    // 尾组：n 个字符 → n-1 字节
    if group.len() > 1 {
        let n = group.len();
        while group.len() < 5 {
            group.push(84);
        }
        let mut v: u32 = 0;
        for &g in &group {
            v = v.wrapping_mul(85).wrapping_add(g as u32);
        }
        let bytes = v.to_be_bytes();
        out.extend_from_slice(&bytes[..n - 1]);
    }
    let _ = seen_eod;
    Some(out)
}

// ---------------------------------------------------------------- ToUnicode CMap

/// ToUnicode CMap：字体字符码 → Unicode 的映射表。
///
/// 为什么需要它：国内招标文件里非常普遍的形态是
/// `/Subtype /Type0 + /Encoding /Identity-H + /ToUnicode N 0 R`——
/// 此时内容流里的字串字节 **不是任何标准编码**，而是字体私有的
/// 字形索引（CID）。不读 CMap 就只能得到乱码。
///
/// 幸好这类文档几乎总会带 ToUnicode（否则 PDF 自己也没法复制文本）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CMap {
    /// 单码 → Unicode。
    single: std::collections::BTreeMap<u32, String>,
    /// 码区间（起, 止，其中只是佋位——本层仅支持长度 1 的直映）。
    ranges: Vec<(u32, u32)>,
    /// 是否已从 cidrange 建出矩阵（一次性展开）。
    range_map: std::collections::BTreeMap<u32, String>,
}

impl CMap {
    /// 空 CMap？
    pub fn is_empty(&self) -> bool {
        self.single.is_empty() && self.range_map.is_empty()
    }

    /// 查一个码点的 Unicode（全部来源合并后的统一查询面）。
    pub fn get(&self, code: u32) -> Option<&str> {
        self.range_map
            .get(&code)
            .or_else(|| self.single.get(&code))
            .map(|s| s.as_str())
    }

    /// 解析 ToUnicode CMap 流的文本。
    ///
    /// 支持 `beginbfchar`…`endbfchar` 与 `beginbfrange`…`endbfrange`。
    /// 不支持的字段（如 `beginbfrange` 的数组形式处理不全）会被**安静地跳过**，
    /// 因为 CMap 本身是“字体辅助信息”——拿不全楼不会造成静默错误，
    /// 而看不明白的字会在下游被识别为缺字。
    pub fn parse(src: &str) -> Self {
        let mut cm = CMap::default();
        for chunk in between_all(src, "beginbfchar", "endbfchar") {
            for (code, dst) in parse_hex_pairs(chunk) {
                cm.single.insert(code, dst);
            }
        }
        for chunk in between_all(src, "beginbfrange", "endbfrange") {
            for (lo, hi, base) in parse_hex_triples(chunk) {
                let n = hi.saturating_sub(lo).min(65_535) + 1;
                let mut cur = base_codepoint(&base);
                for code in lo..=lo + n - 1 {
                    cm.range_map.insert(code, codepoint_to_string(cur));
                    cur = cur.saturating_add(1);
                }
            }
        }
        cm
    }
}

/// 取出 hay 里所有 `start…end` 之间的片段（不含顺序外的子片）。
fn between_all<'a>(hay: &'a str, start: &str, end: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut k = 0usize;
    while let Some(a) = hay[k..].find(start) {
        let from = k + a + start.len();
        match hay[from..].find(end) {
            Some(b) => {
                out.push(&hay[from..from + b]);
                k = from + b + end.len();
            }
            None => break,
        }
    }
    out
}

/// 把 CMap 片段切成「十六进制字符串」与「其它 token」两类・保序的流。
///
/// 关键细节：**目标侧的两个 `<...>` 经常直接相连而中间没有空格**（如 `<0001><0031>`）。
/// 按空白切 token 会把它们连成一个串，所以必须按 `<`/`>` 自己切。
fn cmap_tokens(chunk: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = chunk.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] == b'<' {
            if let Some(rel) = chunk[i..].find('>') {
                out.push(chunk[i..i + rel + 1].to_string());
                i += rel + 1;
                continue;
            }
            break;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'<' {
            i += 1;
        }
        out.push(chunk[start..i].to_string());
    }
    out
}

/// 解析 `beginbfchar`…`endbfchar` 片段：每两个十六进制字符串算一对。
fn parse_hex_pairs(chunk: &str) -> Vec<(u32, String)> {
    let toks = cmap_tokens(chunk);
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < toks.len() {
        if let (Some(code), Some(s)) = (hex_to_u32(&toks[i]), hex_to_unicode(&toks[i + 1])) {
            out.push((code, s));
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

/// 解析 `beginbfrange`…`endbfrange` 片段：每三个十六进制字符串算一组。
fn parse_hex_triples(chunk: &str) -> Vec<(u32, u32, String)> {
    let toks = cmap_tokens(chunk);
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 2 < toks.len() {
        if let (Some(lo), Some(hi), Some(base)) = (
            hex_to_u32(&toks[i]),
            hex_to_u32(&toks[i + 1]),
            hex_to_unicode(&toks[i + 2]),
        ) {
            if hi >= lo {
                out.push((lo, hi, base));
                i += 3;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// `<XXXX>` → 数值。
fn hex_to_u32(tok: &str) -> Option<u32> {
    let t = tok.trim();
    if !t.starts_with('<') || !t.ends_with('>') {
        return None;
    }
    let hex = &t[1..t.len() - 1];
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// `<XXXX>`（UTF-16BE）→ 字符串。
fn hex_to_unicode(tok: &str) -> Option<String> {
    let t = tok.trim();
    if !t.starts_with('<') || !t.ends_with('>') {
        return None;
    }
    let hex = &t[1..t.len() - 1];
    if hex.len() % 4 != 0 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut units = Vec::with_capacity(hex.len() / 4);
    let mut k = 0;
    while k + 4 <= hex.len() {
        units.push(u16::from_str_radix(&hex[k..k + 4], 16).ok()?);
        k += 4;
    }
    Some(
        char::decode_utf16(units)
            .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
    )
}

fn base_codepoint(s: &str) -> u32 {
    s.chars().next().map(|c| c as u32).unwrap_or(0)
}

fn codepoint_to_string(cp: u32) -> String {
    char::from_u32(cp)
        .map(|c| c.to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- 内容流扫描

/// 从已解压的内容流里抽文本行。
///
/// 行切分口径：一条 `Tj`/`TJ` 显示运算符 = 一个文本块；
/// 实务 PDF（ReportLab/Word/LaTeX）每行正文多以独立 Tj 落位，因此逐显示运算符取行。
/// 从已解压的内容流里抽文本行。
///
/// 行切分口径（两种文档生成器只中的一种）：
///
/// 1. **连续 Tj 模式**（ReportLab 类）：一个 Tj 就是一整行，Tj 之间就是行与行之间；
/// 2. **逐字 Tj + Tm 定位模式**（Word/Spire 类，也是国内招标文件常见形态）：
///    每个 Tj 只装 1-2 个字符，真正的行由 `Tm` 的平移部分决定。
///
/// 第二种情形下如果还按 Tj 切行，得到的就是一个字一行的碎片——
/// 所以必须跟踪当前文本矩阵（Tm）的 y，对 y 相同、x 递增的 Tj 合成一行。
///
/// 两种模式共存：合成时如果当前行已经很长且新字符序列又出现大块\u7a7a白，
/// 说明是连续 Tj 模式的新行，也要断开。
fn extract_text_lines(content: &[u8], fcmaps: &FontCMaps, fonts: &FontEncodings) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut parts: Vec<(f64, f64, String)> = Vec::new();
    let mut cur_y: Option<f64> = None;
    let mut cur_x = 0.0f64;
    // #218：Tm 矩阵缩放（文本空间→页面空间）。WPS 等生成器把大字号放进 Tf
    // （如 319），再用 0.05 缩放的 Tm 落位——Td/TD 的推进量是文本空间单位，
    // 必须乘缩放系数才是页面坐标（实测水利标段文件：不乘则行内 x 序错乱，
    // 中西文段按错误 x 排序 → 一字一码交错混排）。默认 1.0 = 无缩放的常规
    // 文档，行为与旧版一致。
    let mut tm_scale_x = 1.0f64;
    let mut tm_scale_y = 1.0f64;
    let mut seq = 0.0f64;
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut nums: Vec<f64> = Vec::new();
    // CTM（当前变换矩阵的平移量）+ q/Q 栈。
    let mut ctm: (f64, f64) = (0.0, 0.0);
    let mut ctm_stack: Vec<(f64, f64)> = Vec::new();
    // #200：字体上下文——`/F2 12 Tf` 的资源名 + 当前字体（决定恒等解码资格）。
    let mut last_name: Option<String> = None;
    let mut cur_font: Option<String> = None;
    // #218 后续：文本窗口（PDF 规范：显示运算符 Tj/TJ/'/" 必须在 BT…ET 内）。
    // 窗口外的字串一律不产出——实测缺口：ToUnicode CMap 流解压后全是
    // `<hex> <hex>` 映射对，无任何运算符上下文，全部堆进 pending、在流尾
    // 被 flush 成巨型垃圾行（目标件 9 个 CMap 共 ~90 万字符，把 CJK 占比
    // 从 0.69 稀释到 0.08——乱码探测的判据被垃圾污染）。字体文件/图像流
    // 里的偶发 `Tj`/`BT` 字节序列同理被窗口挡住。
    let mut in_text = false;
    let mut i = 0usize;
    let n = content.len();

    while i < n {
        match content[i] {
            b'(' => {
                let (s, next) = read_literal_string(content, i);
                pending.push(s);
                i = next;
            }
            b'<' if i + 1 < n && content[i + 1] != b'<' => {
                let (s, next) = read_hex_string(content, i);
                pending.push(s);
                i = next;
            }
            b'<' if i + 1 < n && content[i + 1] == b'<' => i += 2,
            b'>' if i + 1 < n && content[i + 1] == b'>' => i += 2,
            // 资源名 `/F2` 等：记最近的名字——Tf 的字体操作数（#200）。
            b'/' => {
                let mut j = i + 1;
                while j < n
                    && (content[j].is_ascii_alphanumeric()
                        || content[j] == b'-'
                        || content[j] == b'+'
                        || content[j] == b'_')
                {
                    j += 1;
                }
                if j > i + 1 {
                    last_name = Some(String::from_utf8_lossy(&content[i + 1..j]).to_string());
                }
                i = j;
            }
            // BT：进入文本窗口。窗口开启前堆积的 pending 是垃圾
            // （CMap 映射对/二进制噪声）——丢弃，绝不冒充正文。
            b'B' => {
                let (op, next) = read_name(content, i);
                if op == "BT" {
                    in_text = true;
                    pending.clear();
                }
                i = next;
            }
            // ET：退出文本窗口（parts 保留——同一视觉行常被拆成多个
            // BT/ET 块，逐字 Tj 形态每字一块，断行仍靠 Tm 的 y 跳变）。
            b'E' => {
                let (op, next) = read_name(content, i);
                if op == "ET" {
                    in_text = false;
                }
                i = next;
            }
            // q：保存 CTM
            b'q' => {
                ctm_stack.push(ctm);
                i += 1;
            }
            // Q：恢复 CTM（若栈已空则回零——宁可回零也不沿用错的偏移）
            b'Q' => {
                ctm = ctm_stack.pop().unwrap_or((0.0, 0.0));
                i += 1;
            }
            b'c' => {
                let (op, next) = read_name(content, i);
                if op == "cm" && nums.len() >= 6 {
                    let e = nums[nums.len() - 2];
                    let f = nums[nums.len() - 1];
                    ctm = (ctm.0 + e, ctm.1 + f);
                }
                nums.clear();
                i = next;
            }
            b'T' => {
                let (op, next) = read_name(content, i);
                match op.as_str() {
                    "Tj" | "TJ" | "'" | "\"" => {
                        if in_text && !pending.is_empty() {
                            let joined: Vec<u8> = pending.concat();
                            let txt = decode_text_string(
                                &joined,
                                fcmaps,
                                cur_font.as_deref(),
                                cur_font.as_deref().is_some_and(|f| fonts.is_uc2(f)),
                            );
                            if !txt.trim().is_empty() {
                                seq += 1.0;
                                append_at(&mut parts, &txt, cur_x, seq);
                            }
                            pending.clear();
                        }
                        i = next;
                    }
                    "Tf" => {
                        // #200：字体切换——后续文本串的解码上下文换成该字体。
                        cur_font = last_name.clone();
                        nums.clear();
                        i = next;
                    }
                    "Tm" => {
                        if nums.len() >= 6 {
                            let e = nums[nums.len() - 2];
                            let f = nums[nums.len() - 1];
                            // 绝对坐标 = Tm 平移 加上当前 CTM 平移
                            // #218：记录 Tm 的水平/垂直缩放分量（a/d 绝对值——d 常为负的
                            // y 翻转矩阵）。退化矩阵（0）保持原值，防除零式异常。
                            if nums.len() >= 6 {
                                let a = nums[nums.len() - 6].abs();
                                let d = nums[nums.len() - 3].abs();
                                if a > f64::EPSILON {
                                    tm_scale_x = a;
                                }
                                if d > f64::EPSILON {
                                    tm_scale_y = d;
                                }
                            }
                            on_tm2(
                                &mut parts,
                                &mut cur_y,
                                &mut cur_x,
                                ctm.0 + e,
                                ctm.1 + f,
                                &mut lines,
                            );
                        }
                        nums.clear();
                        i = next;
                    }
                    "Td" | "TD" => {
                        if nums.len() >= 2 {
                            let tx = nums[nums.len() - 2];
                            let ty = nums[nums.len() - 1];
                            cur_x += tx * tm_scale_x; // #218：文本空间→页面空间
                            let new_y = cur_y.map(|y| y + ty * tm_scale_y); // #218：同上（垂直分量）
                            if let Some(ny) = new_y {
                                if let Some(prev) = cur_y {
                                    if (ny - prev).abs() > 1.5 {
                                        flush_parts(&mut parts, &mut lines);
                                    }
                                }
                                cur_y = Some(ny);
                            }
                        }
                        nums.clear();
                        i = next;
                    }
                    _ => {
                        nums.clear();
                        i = next;
                    }
                }
            }
            b'0'..=b'9' | b'-' | b'.' => {
                let mut j = i;
                let mut txt = String::new();
                while j < n
                    && (content[j].is_ascii_digit() || content[j] == b'-' || content[j] == b'.')
                {
                    txt.push(content[j] as char);
                    j += 1;
                }
                if let Ok(v) = txt.parse::<f64>() {
                    nums.push(v);
                    if nums.len() > 16 {
                        nums.remove(0);
                    }
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    if in_text && !pending.is_empty() {
        let joined: Vec<u8> = pending.concat();
        let txt = decode_text_string(
            &joined,
            fcmaps,
            cur_font.as_deref(),
            cur_font.as_deref().is_some_and(|f| fonts.is_uc2(f)),
        );
        if !txt.trim().is_empty() {
            seq += 1.0;
            append_at(&mut parts, &txt, cur_x, seq);
        }
    }
    flush_parts(&mut parts, &mut lines);
    lines
}

/// 遇到 `Tm`：若 y 跳变就断行，并记下新行的起点 x。
fn on_tm2(
    parts: &mut Vec<(f64, f64, String)>,
    cur_y: &mut Option<f64>,
    cur_x: &mut f64,
    x: f64,
    y: f64,
    lines: &mut Vec<String>,
) {
    const Y_EPS: f64 = 1.5;
    if let Some(prev) = *cur_y {
        if (y - prev).abs() > Y_EPS {
            flush_parts(parts, lines);
        }
    }
    *cur_y = Some(y);
    *cur_x = x;
}

/// #223 修 A：行级控制字符清洗。
///
/// 剔除 `\u{00}-\u{08}` / `\u{0B}-\u{0C}` / `\u{0E}-\u{1F}` / `\u{7F}` /
/// `\u{80}-\u{9F}`（C1 控制符），保留 `\t \n \r`（合法排版控制）。
///
/// 来源登记：[`read_literal_string`] 的 `\nnn` 八进制转义可解出任意字节、
/// 非 UTF-16BE 分支的 `decode_byte = b as char`（Latin-1 直映）会把
/// 0x80-0x9F 变成 U+0080-U+009F 原样进正文——B-cqyd-招标公告.pdf 实测
/// 157,029 个控制符把全局 cjk_ratio 拉到 0.0086，整份好文件被误判乱码
/// （issue #223）。行级（而非全文级）清理：行边界不变，locator 行号与
/// 文本天然对齐。
pub fn sanitize_line(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{00}'..='\u{08}'
                    | '\u{0B}'..='\u{0C}'
                    | '\u{0E}'..='\u{1F}'
                    | '\u{7F}'
                    | '\u{80}'..='\u{9F}'
            )
        })
        .collect()
}

/// 把当前行落盘（#223：落盘前过 [`sanitize_line`]——所有行的唯一出口，
/// Tm 断行 / Td 断行 / 流尾 flush 全部途经此处，单点接线不漏行）。
fn flush_parts(parts: &mut Vec<(f64, f64, String)>, lines: &mut Vec<String>) {
    if parts.is_empty() {
        return;
    }
    let t = sanitize_line(&join_line(std::mem::take(parts)))
        .trim()
        .to_string();
    if !t.is_empty() {
        lines.push(t);
    }
}

/// 遇到 `Tm`：若 y 变了就断行；否则继续同一行。
fn on_tm(
    cur: &mut String,
    cur_y: &mut Option<f64>,
    cur_x_end: &mut Option<f64>,
    x: f64,
    y: f64,
    lines: &mut Vec<String>,
) {
    const Y_EPS: f64 = 1.5;
    let line_changed = match *cur_y {
        Some(prev) => (y - prev).abs() > Y_EPS,
        None => false,
    };
    if line_changed && !cur.trim().is_empty() {
        lines.push(cur.trim().to_string());
        cur.clear();
    }
    *cur_y = Some(y);
    *cur_x_end = Some(x);
}

/// 追加一段文本到当前行。
///
/// 逐字 Tj 模式下，同一行的字符会按 x 从小到大依次出现，但也有生成器
/// 先写右半再回头写左半——所以不能只靠追加，要按 x 定位插入。
///
/// 实现：行内按 (x, 当前序) 排列存在一个小型带定位的串列表里，落盘时拼接。
fn append_at(cur: &mut Vec<(f64, f64, String)>, txt: &str, x: f64, seq: f64) {
    if !txt.is_empty() {
        cur.push((x, seq, txt.to_string()));
    }
}

/// 把带定位的字符串列表拼成一行（按 x 升序，同 x 保持写入顺序）。
fn join_line(mut parts: Vec<(f64, f64, String)>) -> String {
    parts.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    // 不插空格：同一个 Tj 里的字共享同一个 x，而我们没有字宽信息，
    // 插空格只会把「中文」变成「中 文」。保持紧挨；词间空格属于排版层，
    // 不是本模块的职责范围。
    let mut out = String::new();
    for (_, _, t) in parts {
        out.push_str(&t);
    }
    out
}

fn read_name(content: &[u8], start: usize) -> (String, usize) {
    let mut i = start;
    while i < content.len()
        && (content[i].is_ascii_alphabetic() || content[i] == b'*' || content[i] == b'\'')
    {
        i += 1;
    }
    (String::from_utf8_lossy(&content[start..i]).to_string(), i)
}

/// 读字面串 `(...)`（含嵌套圆括号与转义）。
fn read_literal_string(content: &[u8], start: usize) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut depth = 1i32;
    let mut i = start + 1;
    while i < content.len() {
        let c = content[i];
        if c == b'\\' {
            if i + 1 >= content.len() {
                break;
            }
            let nx = content[i + 1];
            match nx {
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'b' => {
                    out.push(8);
                    i += 2;
                }
                b'f' => {
                    out.push(12);
                    i += 2;
                }
                b'(' | b')' | b'\\' => {
                    out.push(nx);
                    i += 2;
                }
                b'\n' => {
                    i += 2;
                }
                b'\r' => {
                    i += 2;
                }
                b'0'..=b'7' => {
                    let mut oct = String::new();
                    let mut k = i + 1;
                    while k < content.len() && oct.len() < 3 && (b'0'..=b'7').contains(&content[k])
                    {
                        oct.push(content[k] as char);
                        k += 1;
                    }
                    let v = u8::from_str_radix(&oct, 8).unwrap_or(0);
                    out.push(v);
                    i = k;
                }
                _ => {
                    out.push(nx);
                    i += 2;
                }
            }
            continue;
        }
        if c == b'(' {
            depth += 1;
            out.push(c);
            i += 1;
            continue;
        }
        if c == b')' {
            depth -= 1;
            if depth == 0 {
                return (out, i + 1);
            }
            out.push(c);
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    (out, i)
}

/// 读十六进制串 `<...>`。
fn read_hex_string(content: &[u8], start: usize) -> (Vec<u8>, usize) {
    let mut hex = Vec::new();
    let mut i = start + 1;
    while i < content.len() && content[i] != b'>' {
        let c = content[i];
        if c.is_ascii_hexdigit() {
            hex.push(c);
        }
        i += 1;
    }
    let next = if i < content.len() { i + 1 } else { i };
    let mut out = Vec::with_capacity(hex.len() / 2);
    let mut k = 0;
    while k + 1 < hex.len() {
        let s = std::str::from_utf8(&hex[k..k + 2]).unwrap_or("00");
        out.push(u8::from_str_radix(s, 16).unwrap_or(0));
        k += 2;
    }
    if k < hex.len() {
        // 奇数位：末位补 0（PDF 规范）
        let mut s = String::from(std::str::from_utf8(&hex[k..k + 1]).unwrap_or("0"));
        s.push('0');
        out.push(u8::from_str_radix(&s, 16).unwrap_or(0));
    }
    (out, next)
}

/// 字符串编码判定与转码。
///
/// 三种情形（按优先序）：
/// 1. **UTF-16BE**：开头 BOM `FE FF`，或偶数长且过半数高字节为 0x00
///    （拉丁补充/中日韩区域字符的典型形态）；
/// 2. 否则按 **单字节编码**（WinAnsi / PDFDocEncoding 共用 ASCII 面）逐字节取字符。
/// 按 2 字节 CID 逐个查 ToUnicode。
///
/// 全部码都能在 CMap 里命中才采用（返回 Some）；有任何一个查不到就返回 None，
/// 让上层回退到常规编码判定——**不做部分映射**，否则会产出半截乱码。
fn decode_via_cmap(bytes: &[u8], cmap: &CMap) -> Option<String> {
    let mut out = String::new();
    let mut k = 0usize;
    while k + 2 <= bytes.len() {
        let code = u16::from_be_bytes([bytes[k], bytes[k + 1]]) as u32;
        match cmap.get(code) {
            Some(s) => out.push_str(s),
            None => return None,
        }
        k += 2;
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn decode_text_string(
    bytes: &[u8],
    fcmaps: &FontCMaps,
    font: Option<&str>,
    uc2_identity: bool,
) -> String {
    // Identity-H 类字体：字节是 2 字节的 CID，必须走 CMap 查 Unicode。
    // #218 两级查表：先当前字体的**专属表**（多字体 CID 重叠——全局
    // 合并会错字，见 FontCMaps）；未全命中再降级**全局合并表**（小表
    // 字体的跨字体借字——#218 之前全局合并路径的既有行为面，保留
    // 以不劣化：实测 cqyd 文件 6 字体中 3 个表只有 4~13 条，专属表
    // 不全命中的串靠并集表碰巧命中）；仍未命中回退常规编码判定。
    if !bytes.is_empty() && bytes.len() % 2 == 0 {
        if let Some(own) = font.and_then(|f| fcmaps.maps.get(f)) {
            if let Some(mapped) = decode_via_cmap(bytes, own) {
                return mapped;
            }
        }
        if !fcmaps.merged.is_empty() {
            if let Some(mapped) = decode_via_cmap(bytes, &fcmaps.merged) {
                return mapped;
            }
        }
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return utf16be_to_string(&bytes[2..]);
    }
    // #200：UniXX-UCS2 预定义 CMap 恒等映射——当前字体声明了该编码时，
    // 2 字节码就是 UCS-2 值（无 ToUnicode 也能精确解码）。放 BOM 之后、
    // 启发式之前；字体上下文（uc2_identity 只对声明字体为 true）保证
    // 拉丁字体的 2 字节 ASCII 串（如 "kV" = 0x6B56 落 CJK 区）绝不误入此路。
    if uc2_identity && !bytes.is_empty() && bytes.len() % 2 == 0 {
        return utf16be_to_string(bytes);
    }
    if looks_utf16be(bytes) {
        return utf16be_to_string(bytes);
    }
    bytes.iter().map(|&b| decode_byte(b)).collect()
}

fn looks_utf16be(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return false;
    }
    // 将每两字节当一个 UTF-16 码元，看它们是否落在「真实会用到的码位区」。
    //
    // 实测经验：C-补遗.pdf 的环境字串是「某市第一…」，高字节并不是 0（某=U+67D0）——
    // 旧判据「过半高字节为 0」会把整段中文判残。改用码位域判定：
    // 逆向判据是「这批码元里有多少是合法的 CJK/拉丁码位」。
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    if units.is_empty() {
        return false;
    }
    let plausible = units
        .iter()
        .filter(|&&u| {
            // 拉丁基本面可打印字符 / CJK 统一表意区 / 常见中文标点与全角字符 / 空格
            let c = u;
            c == 0x20
                || (0x21..=0x7E).contains(&c)
                || (0x3000..=0x303F).contains(&c)   // CJK 标点
                || (0x4E00..=0x9FFF).contains(&c)   // CJK 统一表意
                || (0xFF00..=0xFFEF).contains(&c)   // 全角形式
                || (0x2010..=0x2027).contains(&c) // 一般标点
        })
        .count();
    // 七成以上码元合理 → 当 UTF-16BE 读（单字节编码下这个比例不可能达到）。
    plausible * 10 >= units.len() * 7
}

fn utf16be_to_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// WinAnsi / PDFDocEncoding 的 ASCII 面直取；高位字节保留为 Latin-1 同码位。
///
/// 如实登记：真正的 WinAnsi 高位区（0x80..0xFF）与 Latin-1 在 0x80-0x9F
/// 不同（WinAnsi 在那里放的是欧文标点与特殊字符）。中文业务里这个差异
/// 影响面极小（中文覆盖靠 UTF-16BE 路），故不做完整映射表。
fn decode_byte(b: u8) -> char {
    b as char
}

/// 将兜底提取结果装成标准 [`ParsedFile`]（与 poppler 路径同形态——
/// 下游抽取/建索引不需要区分文本来自哪条路）。
///
/// 诚实记载：parse_summary 里明写「纯 Rust PDF 文本兜底」与流解码/跳过数，
/// 后期对账时能区分「走了正常链路」和「走了兜底」。
pub fn parse_via_fallback(path: &std::path::Path, name: &str) -> Result<crate::ParsedFile, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取失败：{e}"))?;
    let t = extract(&bytes).map_err(|e| e.to_string())?;
    if !t.has_text() {
        return Err("PDF 无文本层（兜底不做 OCR）".to_string());
    }

    let mut blocks = Vec::new();
    let mut idx = 0usize;
    for (pno, page) in t.pages.iter().enumerate() {
        for (lno, line) in page.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            blocks.push(crate::Block {
                idx,
                kind: crate::BlockKind::PdfLine,
                locator: format!("第{} 页 > 第{} 行", pno + 1, lno + 1),
                text: line.clone(),
                cells: Vec::new(),
            });
            idx += 1;
        }
    }

    let pages = t
        .pages
        .iter()
        .filter(|p| p.iter().any(|l| !l.trim().is_empty()))
        .count();
    // #218 乱码可见性：兜底产物自报 CJK 占比；声明中文字体而占比异常低
    //（<2%，与 probe_cjk_garble 的 Garbled 同阈）→ summary 显式标注
    // 「疑似乱码」——Agent 在导入回执即知文本层可能不可用，不用等
    // 抽取 0 条再回头排查（半乱码形态 probe 阈值会漏，summary 兜底可见；
    // probe 的拒收面在 lib.rs fail-loud——两条互补）。
    let all_text: String = blocks.iter().map(|b| b.text.as_str()).collect();
    let ratio = cjk_ratio(&all_text);
    let mut summary = format!("{} [{}]", t.summary(), name);
    // #223：summary 同步页级判定——兜底「页」是文本流组（语义与 summary
    // 的「N 页」口径一致，如实自报）。多数页正常而个别页垃圾 → 点名异常页
    // 而非整份误标（B-cqyd 实测：修 A 后全局 ratio≈1.1% 仍 <2%，拉平判定
    // 会继续误标整份——页级判定后 Clean + 点名第 5 流组）。
    let page_texts: Vec<String> = t.pages.iter().map(|p| p.concat()).collect();
    let page_probe = probe_cjk_garble_pages(&bytes, &page_texts);
    let fmt_pages = |v: &[usize]| -> String {
        v.iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join("、")
    };
    match page_probe.verdict {
        CjkGarbleProbe::Garbled => {
            summary.push_str(&format!(
                "；【疑似乱码】声明中文字体而有效页 CJK 占比异常（乱码页：{}）——文本层可能不可用（条款抽取可能为 0），建议提供可解析版本（DOCX）或安装 pdfium/poppler 后重新导入",
                fmt_pages(&page_probe.garbled_pages)
            ));
        }
        CjkGarbleProbe::Clean if !page_probe.garbled_pages.is_empty() => {
            summary.push_str(&format!(
                "；CJK 占比 {:.1}%（第{}页疑似乱码，其余页正常——异常页文本可能不可用，不影响整份可用性）",
                ratio * 100.0,
                fmt_pages(&page_probe.garbled_pages)
            ));
        }
        // #218 可见性兜底：无有效页（全部页样本不足，退化拉平判 NoSignal）
        // 时保持拉平可见性——声明 CJK 而占比异常低仍标注（Agent 可见性）。
        // 不影响 probe 的拒收面（那是 lib.rs fail-loud 的事），也不影响
        // B-cqyd 形态（它有有效页，走上面的页级分支）。
        CjkGarbleProbe::NoSignal if bytes_declare_cjk_font(&bytes) && ratio < 0.02 => {
            summary.push_str(&format!(
                "；【疑似乱码】声明中文字体而文本 CJK 占比仅 {:.1}%——文本层可能不可用（条款抽取可能为 0），建议提供可解析版本（DOCX）或安装 pdfium/poppler 后重新导入",
                ratio * 100.0
            ));
        }
        _ => {
            summary.push_str(&format!("；CJK 占比 {:.1}%", ratio * 100.0));
        }
    }
    Ok(crate::ParsedFile {
        name: name.to_string(),
        sha256: String::new(), // 由 parse_file 统一回填
        pages,
        tables: 0,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary: summary,
        components: Vec::new(),
    })
}

// ---------------------------------------------------------------- CJK 乱码探测（issue #169）
//
// 实测根因链（Windows CI 稳定复现）：
//   pdftotext 在缺 CMap 资源包（poppler-data）的 poppler 构建上，对
//   「非嵌入 CID 字体 + UniGB-UCS2-H」形态的中文 PDF 输出乱码——
//   双字节字符码被逐字节当 Latin-1（实测形态：`原招标文件` → `SbheN`，
//   即 U+539F→'S'、U+62DB→'b'、U+6807→'h'、U+6587→'e'、U+4EF6→'N'，
//   不可打印字节被丢弃后剩下的可打印残渣）。
//   关键危害：`pdf::parse` 对此**返回 Ok**（has_text_layer=true、blocks 非空），
//   parse_file 拿到「成功」就不再走兜底——静默产出垃圾文本（issue #169 影响面）。
//
// 判据设计（语义判据，零误伤）：
//   PDF 字典明文层声明了 CJK 字体/CMap 痕迹，而解析出的文本 CJK 占比≈0
//   → 必然是解码失败（真中文件正常解码 CJK 占比 >0.5；纯英文件不声明
//     CJK 字体，本判据永不触发）。

/// CJK 字体 / CMap 名痕迹。PDF 对象字典不经压缩（流才压缩），字体名与
/// 编码名可在原始字节上直接搜索。覆盖国内政采 PDF 的主流形态：
/// ReportLab（STSong-Light + Adobe-GB1/UniGB-UCS2-H）、Word/WPS 导出
/// （SimSun/SimHei/KaiTi/FangSong 等）、macOS（PingFang）、开源
/// （NotoSansCJK/SourceHanSans/WenQuanYi）与繁体/日韩（B5pc/MSung/UniJIS）。
const CJK_FONT_MARKERS: &[&[u8]] = &[
    // CMap / CIDSystemInfo 注册名
    b"Adobe-GB1",
    b"Adobe-CNS1",
    b"Adobe-Japan1",
    b"Adobe-Korea1",
    b"UniGB-UCS",
    b"UniCNS-UCS",
    b"UniJIS-UCS",
    b"UniKS-UCS",
    b"GBK-EUC",
    b"GB-EUC",
    b"GBpc-EUC",
    b"B5pc-",
    b"ETen-B5",
    b"CNS-EUC",
    // 常见中文/日韩字体名（非嵌入时字典里仍留名）
    b"STSong",
    b"STKaiti",
    b"STXihei",
    b"STFangsong",
    b"STZhongsong",
    b"SimSun",
    b"SimHei",
    b"FangSong",
    b"KaiTi",
    b"YouYuan",
    b"HuaWenKaiTi",
    b"Microsoft YaHei",
    b"DengXian",
    b"PingFang",
    b"Hiragino",
    b"Yu Gothic",
    b"MSung",
    b"MHei",
    b"MKai",
    b"SourceHanSans",
    b"SourceHanSerif",
    b"NotoSansCJK",
    b"NotoSerifCJK",
    b"WenQuanYi",
];

/// PDF 原始字节是否声明了 CJK 字体/CMap。
pub fn bytes_declare_cjk_font(bytes: &[u8]) -> bool {
    CJK_FONT_MARKERS.iter().any(|m| contains(bytes, m))
}

// ------------------------------------------------ CID-CJK 在用探测（#200 R1）
//
// 背景：#169 判据「PDF 声明 CJK 字体而输出无 CJK → 乱码」的字面口径把
// 「声明 SimSun 的纯英文模板件」「声明未使用的 CID 字体」也判成乱码
// （owner #200 R1 裁决：误伤）。乱码的真实成因是 **CID 字体的 CMap 映射
// 环节失败**——所以激活条件收窄为：字典里有 CMap/CIDSystemInfo 注册名
// **且** 该字体被内容流的 Tf 实际选用（或内容流解不开、无法排除）。

/// CID CMap 注册名标记（能「解码失败」的编码形态）。
///
/// 与 [`CJK_FONT_MARKERS`] 的区别：**不含纯字体名**（SimSun/STSong 等）——
/// 简单字体的文本不经 CMap，无从映射失败；SimSun 既可能是 Type1 简单字体
/// （英文模板件常用）也可能是 Type0 的 BaseFont，判定必须看编码标记而非名字。
const CID_CMAP_MARKERS: &[&[u8]] = &[
    b"Adobe-GB1",
    b"Adobe-CNS1",
    b"Adobe-Japan1",
    b"Adobe-Korea1",
    b"UniGB-UCS",
    b"UniCNS-UCS",
    b"UniJIS-UCS",
    b"UniKS-UCS",
    b"GBK-EUC",
    b"GB-EUC",
    b"GBpc-EUC",
    b"B5pc-",
    b"ETen-B5",
    b"CNS-EUC",
];

/// 对象号 → 对象体内是否含 CID CMap 标记（对象体 = 本对象头到下一头/文件尾）。
fn obj_declares_cid_cmap(bytes: &[u8], heads: &[(usize, u64)], num: u64) -> bool {
    for (i, &(pos, n)) in heads.iter().enumerate() {
        if n == num {
            let end = heads.get(i + 1).map(|&(p, _)| p).unwrap_or(bytes.len());
            let body = &bytes[pos..end];
            return CID_CMAP_MARKERS.iter().any(|m| contains(body, m));
        }
    }
    false
}

/// 内容流里 Tf 实际选用的字体资源名（跨全部可解码流）。
///
/// 返回 (找到的 Tf 名字集合, 是否解出过至少一个流)。**保守原则**：
/// Tf 扫描只用于「排除」——解不出流或一个 Tf 都没找到时，调用方应按
/// 「无法排除」继续激活探测（防 #169 防线被扫描盲区静默关掉）。
fn fonts_used_by_tf(bytes: &[u8]) -> (std::collections::HashSet<String>, bool) {
    let mut used = std::collections::HashSet::new();
    let mut any_stream = false;
    for (data, filters) in iter_streams(bytes) {
        if let Some(decoded) = decode_stream(&data, &filters) {
            any_stream = true;
            collect_tf_names(&decoded, &mut used);
        }
    }
    (used, any_stream)
}

/// 从单个已解码内容流收集 `/NAME <size> Tf` 的资源名。
fn collect_tf_names(content: &[u8], out: &mut std::collections::HashSet<String>) {
    let mut k = 0usize;
    while let Some(p) = find_from(content, k, b"Tf") {
        // 向前：跳空白 → 跳字号数字（含负号/小数点）→ 跳空白 → 读 `/NAME`。
        let mut j = p;
        while j > 0 && content[j - 1].is_ascii_whitespace() {
            j -= 1;
        }
        while j > 0
            && (content[j - 1].is_ascii_digit() || content[j - 1] == b'-' || content[j - 1] == b'.')
        {
            j -= 1;
        }
        while j > 0 && content[j - 1].is_ascii_whitespace() {
            j -= 1;
        }
        let mut s = j;
        while s > 0
            && (content[s - 1].is_ascii_alphanumeric()
                || content[s - 1] == b'-'
                || content[s - 1] == b'+'
                || content[s - 1] == b'_')
        {
            s -= 1;
        }
        if s < j && s > 0 && content[s - 1] == b'/' {
            if let Ok(name) = std::str::from_utf8(&content[s..j]) {
                if !name.is_empty() {
                    out.insert(name.to_string());
                }
            }
        }
        k = p + 2;
    }
}

/// 乱码探测的激活条件（#200 R1）：存在**被内容流实际选用**（或无法排除）的
/// CID-CJK 编码字体。
///
/// 保守方向：判「未用」必须同时满足——解出过流、找到过 Tf、且没有任何
/// Tf 选用的字体带 CID CMap 标记。任一环节失明 → 按标记存在即激活
/// （宁可多激活走互证/fail-loud，不可把 #169 防线静默关掉）。
pub fn cid_cjk_font_in_use(bytes: &[u8]) -> bool {
    let heads = scan_obj_heads(bytes);
    let refs = scan_name_refs(bytes);
    let marker_fonts: Vec<u64> = {
        let mut seen = std::collections::HashSet::new();
        let mut v = Vec::new();
        for (_, n) in refs.iter() {
            if seen.insert(*n) && obj_declares_cid_cmap(bytes, &heads, *n) {
                v.push(*n);
            }
        }
        v
    };
    let any_marker_in_doc =
        !marker_fonts.is_empty() || CID_CMAP_MARKERS.iter().any(|m| contains(bytes, m));
    if !any_marker_in_doc {
        return false; // 全文档无 CID CMap 痕迹（Type1 简单字体件）——永不激活
    }
    let (used, any_stream) = fonts_used_by_tf(bytes);
    if !any_stream || used.is_empty() {
        return true; // 流解不开 / 没找到任何 Tf：无法排除 → 保守激活
    }
    refs.iter()
        .any(|(name, n)| used.contains(name) && marker_fonts.contains(n))
}

/// 非空白字符中非 ASCII 的占比（#200 R1 残渣特征）。
///
/// 用途：首选路不可用、兜底单路输出无 CJK 时——占比高（如 GBK 双字节被
/// 逐字节 Latin-1 化后 ≈ 一半字符 ≥ U+0080）说明输出像「多字节误判残渣」，
/// 无第二路互证时 fail-loud；占比低（真英文/拉丁模板件）说明内容本就
/// 单字节，按「内容确无 CJK」采信。
pub fn non_ascii_ratio(text: &str) -> f64 {
    let mut total = 0usize;
    let mut non_ascii = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        total += 1;
        if !c.is_ascii() {
            non_ascii += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        non_ascii as f64 / total as f64
    }
}

/// 双路输出是否互证（#200 R1）：NFKC + 去空白 + 去标点后逐字符相等。
///
/// 严格相等才互证——两路独立引擎解出同一内容，说明内容确实如此（字体
/// 声明与内容语言不符的模板件）；任何差异都不互证（宁可 fail-loud 也不
/// 把分叉的乱码当一致）。
pub fn texts_corroborate(a: &str, b: &str) -> bool {
    norm_for_corroborate(a) == norm_for_corroborate(b)
}

fn norm_for_corroborate(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let nfkc: String = s.nfkc().collect();
    nfkc.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !c.is_ascii_punctuation()
                && !matches!(
                    c,
                    '，' | '。'
                        | '；'
                        | '、'
                        | '：'
                        | '（'
                        | '）'
                        | '！'
                        | '？'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '【'
                        | '】'
                        | '《'
                        | '》'
                        | '—'
                        | '…'
                )
        })
        .collect()
}

/// 文本的 CJK 占比：CJK 表意区（含扩展A/兼容）+ 中文标点/全角 / 非空白字符数。
/// 与 [`looks_utf16be`] 的码位合理区同源——探测与兜底解码用同一套「中文长什么样」。
pub fn cjk_ratio(text: &str) -> f64 {
    let mut total = 0usize;
    let mut cjk = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        total += 1;
        if matches!(c,
            '\u{4E00}'..='\u{9FFF}'     // CJK 统一表意
            | '\u{3400}'..='\u{4DBF}'   // 扩展 A
            | '\u{F900}'..='\u{FAFF}'   // 兼容表意
            | '\u{3000}'..='\u{303F}'   // CJK 标点
            | '\u{FF00}'..='\u{FFEF}'   // 全角形式
            | '\u{2010}'..='\u{2027}'   // 一般标点（–—‘’“”…）
        ) {
            cjk += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        cjk as f64 / total as f64
    }
}

/// CJK 乱码探测结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CjkGarbleProbe {
    /// PDF 声明 CJK 字体且输出 CJK 占比正常——解码可信，不干预。
    Clean,
    /// PDF 声明 CJK 字体但输出 CJK 占比≈0——解码失败（乱码），应降级/报错。
    Garbled,
    /// 判据不适用：无 CJK 字体声明（英文件）或文本样本不足（<32 非空白字符）。
    NoSignal,
}

/// 探测一条解析路的文本是否为 CJK 乱码。
///
/// 阈值 0.02：真中文件正常解码占比 >0.5、乱码残渣 ≈0（实测 CI 乱码串 0 个
/// CJK 字符）——0.02 距两者都有数量级余量。位于中间的合法形态（声明了中文
/// 字体但正文以英文为主的模板件）被降级也无损失：降级只是换纯 Rust 兜底
/// 重解同一文件，summary 诚实自报。
pub fn probe_cjk_garble(pdf_bytes: &[u8], text: &str) -> CjkGarbleProbe {
    probe_cjk_garble_flat(pdf_bytes, text)
}

/// 全文拉平判据（#223 前的唯一判定；保留为分页判据的退化面）。
fn probe_cjk_garble_flat(pdf_bytes: &[u8], text: &str) -> CjkGarbleProbe {
    // #200 R1 收窄：仅 CID-CJK 字体实际在用时才启用乱码探测。
    if !cid_cjk_font_in_use(pdf_bytes) {
        return CjkGarbleProbe::NoSignal;
    }
    let non_ws = text.chars().filter(|c| !c.is_whitespace()).count();
    if non_ws < 32 {
        return CjkGarbleProbe::NoSignal;
    }
    if cjk_ratio(text) < 0.02 {
        CjkGarbleProbe::Garbled
    } else {
        CjkGarbleProbe::Clean
    }
}

/// 页级乱码判定结论（#223）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageGarbleVerdict {
    /// 整体结论。
    pub verdict: CjkGarbleProbe,
    /// 疑似乱码页号（1-based，按传入 pages 序；Clean 且非空 = 局部异常页）。
    pub garbled_pages: Vec<usize>,
}

/// #223 修 B：分页多数决 CJK 乱码探测——一页垃圾不再否决整份文件。
///
/// B-cqyd-招标公告.pdf 实测：第 1-4 页正常（汉字 537/906/1161/579）而
/// 第 5 文本流组 3209 字符全是控制符——全文拉平后 cjk_ratio=0.0086 被误判
/// Garbled 整份拒收。分页后：有效页中「CJK 正常」的占严格多数 → Clean，
/// 异常页单独点名（进 parse_summary），不否决整份。
///
/// 阈值与拉平判据同源：有效页 = 非空白字符 ≥ 32（样本门）；页正常 =
/// cjk_ratio ≥ 0.02（Garbled 阈）——不引入第二套魔法数。
///
/// **不放松的红线**：全部有效页皆乱（clean==0）→ 仍判 [`CjkGarbleProbe::Garbled`]
/// （fail-loud：全乱就是全乱，多数决不救）；平票（clean*2 == valid）保守判
/// Garbled；有效页为 0（全部页样本不足）→ 退化为拉平判据（拼全部页，
/// 行为与 #223 之前逐位一致）。
pub fn probe_cjk_garble_pages(pdf_bytes: &[u8], pages: &[String]) -> PageGarbleVerdict {
    // 与全文判据一致：声明但未被内容流使用的 CID 字体不触发误判。
    if !cid_cjk_font_in_use(pdf_bytes) {
        return PageGarbleVerdict {
            verdict: CjkGarbleProbe::NoSignal,
            garbled_pages: Vec::new(),
        };
    }
    let valid: Vec<(usize, &str)> = pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.chars().filter(|c| !c.is_whitespace()).count() >= 32)
        .map(|(i, p)| (i, p.as_str()))
        .collect();
    if valid.is_empty() {
        // 退化面：拼全部页跑拉平判据（小文档不放松——单页文件行为不变）
        let flat: String = pages.concat();
        return PageGarbleVerdict {
            verdict: probe_cjk_garble_flat(pdf_bytes, &flat),
            garbled_pages: Vec::new(),
        };
    }
    let mut garbled_pages = Vec::new();
    let mut clean = 0usize;
    for (i, p) in &valid {
        if cjk_ratio(p) < 0.02 {
            garbled_pages.push(i + 1);
        } else {
            clean += 1;
        }
    }
    let verdict = if clean == 0 {
        CjkGarbleProbe::Garbled
    } else if clean * 2 > valid.len() {
        CjkGarbleProbe::Clean
    } else {
        // 少数正常（含平票）：多数已乱，整份不可信——保守 Garbled
        CjkGarbleProbe::Garbled
    };
    PageGarbleVerdict {
        verdict,
        garbled_pages,
    }
}

/// 拼一条路的全部文本（探测输入）。
pub fn parsed_text_of(pf: &crate::ParsedFile) -> String {
    pf.blocks.iter().map(|b| b.text.as_str()).collect()
}

/// 从块 locator 解析页号（#223 分桶用）。
///
/// 兼容三种在产格式：`第1页 行2`（pdftotext 首选路 pdf.rs）、
/// `第1 页 > 第2 行`（纯 Rust 兜底路 parse_via_fallback）、
/// `第1页 OCR 行2`（OcrLine）。无法解析 → None（调用方兜底进尾桶）。
fn page_no_of_locator(loc: &str) -> Option<usize> {
    let t = loc.trim_start().strip_prefix('第')?;
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = &t[digits.len()..];
    if rest.starts_with("页") || rest.starts_with(" 页") {
        digits.parse().ok()
    } else {
        None
    }
}

/// 按块 locator 的「第N页」前缀把一条解析路的文本分桶回页（#223 探测输入）。
///
/// locator 语义本就是「页内序号」的确定性编号（见 [`page_no_of_locator`] 的
/// 三种在产格式），分桶即还原页结构；无法解析的块兜底进当前最后一桶——
/// 文本绝不从探测中丢失（丢文本会把 Garbled 误判成 NoSignal）。
pub fn parsed_pages_of(pf: &crate::ParsedFile) -> Vec<String> {
    let mut pages: Vec<String> = Vec::new();
    for b in &pf.blocks {
        match page_no_of_locator(&b.locator) {
            Some(no) if no >= 1 => {
                while pages.len() < no {
                    pages.push(String::new());
                }
                pages[no - 1].push_str(&b.text);
            }
            _ => {
                if pages.is_empty() {
                    pages.push(String::new());
                }
                let last = pages.len() - 1;
                pages[last].push_str(&b.text);
            }
        }
    }
    pages
}

// ---------------------------------------------------------------- CJK 乱码拒收的原因定位（issue #200 剩余能力边界登记面）

/// 非 UCS2 预定义 CMap 痕迹（#200 剩余边界的核心族）。
///
/// 这些 CMap 的 2 字节码是 GBK/Big5 等本地码值——**不是 Unicode 码位**，
/// UniXX-UCS2 恒等解码对它们不成立（恒等会把 GBK 码值读成无关汉字，
/// 即「硬解产出垃圾文本」）。正确解码需内置映射大表，本模块不做
/// （fail-loud 纪律）。字节可在字体字典层直接搜到（字典不进压缩流）。
const NON_UCS2_CMAP_MARKERS: &[(&[u8], &str)] = &[
    (
        b"GBK-EUC",
        "GBK-EUC-H/V（简体中文，GBK 码值——非 Unicode 码位）",
    ),
    (
        b"GB-EUC",
        "GB-EUC-H/V（简体中文 EUC 码值——非 Unicode 码位）",
    ),
    (b"GBpc-EUC", "GBpc-EUC（简体中文——非 Unicode 码位）"),
    (b"B5pc-", "B5pc-H/V（繁体中文 Big5 码值——非 Unicode 码位）"),
    (b"ETen-B5", "ETen-B5（繁体中文——非 Unicode 码位）"),
    (b"CNS-EUC", "CNS-EUC（繁体中文——非 Unicode 码位）"),
];

/// Adobe CID 注册序号痕迹——码值是注册表内的**字形编号**（分 Supplement
/// 版本），与 Unicode 无恒等关系，需 Adobe-GB1↔Unicode 映射大表。
///
/// 两种书写形态都探：连字注册名（`/Adobe-GB1-0`、CMap 名等）与
/// CIDSystemInfo 分离书写（`/Registry (Adobe) /Ordering (GB1)`）。
/// 注：UniGB-UCS2-H 可解形态的 CIDSystemInfo 同样含 `(GB1)`——但可解
/// 文件探测为 Clean 不会走到本 hint，命中即「确实解不出且存在该族字体」。
const CID_REGISTRY_MARKERS: &[(&[u8], &str)] = &[
    (b"Adobe-GB1", "Adobe-GB1"),
    (b"Adobe-CNS1", "Adobe-CNS1"),
    (b"Adobe-Japan1", "Adobe-Japan1"),
    (b"Adobe-Korea1", "Adobe-Korea1"),
    (b"Ordering (GB1)", "Adobe-GB1"),
    (b"Ordering (CNS1)", "Adobe-CNS1"),
    (b"Ordering (Japan1)", "Adobe-Japan1"),
    (b"Ordering (Korea1)", "Adobe-Korea1"),
];

/// CJK 乱码拒收时的「原因 + 绕过」定位文案（#200 剩余边界的用户可见面）。
///
/// 消费方：`lib.rs` 两处 fail-loud 报错（首选路乱码兜底失败 / 双路皆乱）。
/// 纪律：**只在探测已判 Garbled（确认解不出）后调用**——这是原因线索
/// 不是唯一断言（一个 PDF 可同时声明多种字体）。返回 `None` = 无可点名
/// 边界（泛化乱码，调用方保持原报错文案）。
pub fn cjk_garble_reason_hint(bytes: &[u8]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    // 族 1：非 UCS2 预定义 CMap（GBK/Big5 码值族）
    let non_ucs2: Vec<&str> = NON_UCS2_CMAP_MARKERS
        .iter()
        .filter(|(m, _)| contains(bytes, m))
        .map(|(_, name)| *name)
        .collect();
    if !non_ucs2.is_empty() {
        parts.push(format!(
            "声明了 {}——这些预定义 CMap 的码值不是 Unicode 码位，恒等解码不可用，正确解码需内置映射大表（本兜底不硬解）",
            non_ucs2.join("、")
        ));
    }

    // 族 2：Adobe CID 注册序号（同族多形态去重）
    let mut cid: Vec<&str> = CID_REGISTRY_MARKERS
        .iter()
        .filter(|(m, _)| contains(bytes, m))
        .map(|(_, name)| *name)
        .collect();
    cid.sort_unstable();
    cid.dedup();
    if !cid.is_empty() {
        parts.push(format!(
            "存在 {} CID 注册序号字体（无 ToUnicode）——码值是 Adobe 注册表字形编号，与 Unicode 无恒等关系，需映射大表（本兜底不硬解）",
            cid.join("/")
        ));
    }

    // 族 3：PDF 1.5+ 对象流——字体字典可能在压缩对象流内（扫描不到）
    if contains(bytes, b"/ObjStm") {
        parts.push(
            "检测到 /ObjStm（PDF 1.5+ 对象流）——字体字典可能被压缩在对象流内，\
             字典层扫描拿不到编码声明（UniXX-UCS2 恒等解码资格判定失效）"
                .to_string(),
        );
    }

    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "原因定位（#200 能力边界——疑因线索，非唯一断言）：{}。绕过：① 安装 pdfium（首选路自带 CMap）② 安装带 poppler-data（CMap 资源包）的 poppler ③ 用 Word/WPS 打开该 PDF 另存为 DOCX 后导入。",
        parts.join("；")
    ))
}

#[cfg(test)]
mod tests {
    /// #218 测试辅助：merged=EMPTY 的 FontCMaps（保持旧测试语义）。
    fn decode_text_string_fc(bytes: &[u8], _cm: &CMap, uc2: bool) -> String {
        let fc = FontCMaps::default();
        decode_text_string(bytes, &fc, None, uc2)
    }

    use super::*;

    /// 空 CMap —— 不做任何字体映射时的基准。
    const EMPTY: CMap = CMap {
        single: std::collections::BTreeMap::new(),
        ranges: Vec::new(),
        range_map: std::collections::BTreeMap::new(),
    };

    // ---------------------------------------------------------------- 工具

    fn min_pdf(content: &[u8]) -> Vec<u8> {
        // 最小可读 PDF：一页，一个未压缩内容流。
        let mut v = Vec::new();
        v.extend_from_slice(b"%PDF-1.4\n1 0 obj\n<< /Length ");
        v.extend_from_slice(content.len().to_string().as_bytes());
        v.extend_from_slice(b" >>\nstream\n");
        v.extend_from_slice(content);
        v.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");
        v
    }

    // ---------------------------------------------------------------- ASCII85

    #[test]
    fn ascii85_matches_reference_vectors() {
        // "Man " 的 ASCII85 是 "9jqo^"（Adobe 官方示例）
        let got = ascii85_decode(b"9jqo^~>").unwrap();
        assert_eq!(&got[..4], b"Man ", "官方向量：Man + 空格");

        // z = 4 个 0 字节
        let z = ascii85_decode(b"z~>").unwrap();
        assert_eq!(z, vec![0, 0, 0, 0], "z 展开为四个零");

        // 空白与换行必须被忽略（PDF 实务里 ASCII85 换行不定长）
        let wrapped = ascii85_decode(b"9j\nqo\r\n^~>").unwrap();
        assert_eq!(&wrapped[..4], b"Man ");

        // 尾组：2 字符 → 1 字节
        assert_eq!(ascii85_decode(b"!!~>").unwrap(), vec![0]);
    }

    // ---------------------------------------------------------------- 流解码

    #[test]
    fn flate_stream_is_decoded() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let payload = b"BT /F1 12 Tf (Hello) Tj ET";
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(payload).unwrap();
        let compressed = enc.finish().unwrap();

        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n1 0 obj\n<< /Filter /FlateDecode /Length ");
        pdf.extend_from_slice(compressed.len().to_string().as_bytes());
        pdf.extend_from_slice(b" >>\nstream\n");
        pdf.extend_from_slice(&compressed);
        pdf.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");

        let t = extract(&pdf).expect("FlateDecode 流应能提取");
        assert_eq!(t.lines(), vec!["Hello".to_string()]);
        assert_eq!(t.streams_decoded, 1);
    }

    #[test]
    fn ascii85_then_flate_chain_is_decoded() {
        // 实务里的 C-补遗 就是 [ /ASCII85Decode /FlateDecode ] 两层
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        fn a85_encode(data: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            for chunk in data.chunks(4) {
                let mut buf = [0u8; 4];
                buf[..chunk.len()].copy_from_slice(chunk);
                let v = u32::from_be_bytes(buf);
                if v == 0 && chunk.len() == 4 {
                    out.push(b'z');
                    continue;
                }
                let mut tmp = [0u8; 5];
                let mut vv = v;
                for k in (0..5).rev() {
                    tmp[k] = (vv % 85) as u8 + 33;
                    vv /= 85;
                }
                let keep = chunk.len() + 1;
                out.extend_from_slice(&tmp[..keep]);
            }
            out.extend_from_slice(b"~>");
            out
        }

        // 中文在 PDF 里走 UTF-16BE（带 BOM）：中(4E2D) 文(6587) 测(6D4B) 试(8BD5)
        let payload: &[u8] = b"BT /F1 12 Tf <FEFF4E2D65876D4B8BD5> Tj ET";
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(payload).unwrap();
        let compressed = enc.finish().unwrap();
        let encoded = a85_encode(&compressed);

        let mut pdf = Vec::new();
        pdf.extend_from_slice(
            b"%PDF-1.4\n1 0 obj\n<< /Filter [ /ASCII85Decode /FlateDecode ] /Length ",
        );
        pdf.extend_from_slice(encoded.len().to_string().as_bytes());
        pdf.extend_from_slice(b" >>\nstream\n");
        pdf.extend_from_slice(&encoded);
        pdf.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");

        let t = extract(&pdf).expect("两层链应能提取");
        assert_eq!(t.lines(), vec!["中文测试".to_string()]);
    }

    // ---------------------------------------------------------------- 字符串编码

    #[test]
    fn utf16be_strings_are_decoded() {
        // 带 BOM
        let with_bom = [0xFE, 0xFF, 0x4E, 0x2D, 0x65, 0x87];
        assert_eq!(decode_text_string_fc(&with_bom, &EMPTY, false), "中文");

        // 无 BOM 但高字节全 0（UTF-16BE 的 ASCII 区域）
        let no_bom = [0x00, 0x41, 0x00, 0x42];
        assert_eq!(decode_text_string_fc(&no_bom, &EMPTY, false), "AB");

        // 单字节编码：直取
        assert_eq!(decode_text_string_fc(b"Hello", &EMPTY, false), "Hello");
    }

    #[test]
    fn literal_string_handles_escapes_and_nesting() {
        let raw: &[u8] = b"(a\\(b\\)c\\n)";
        let (s, next) = read_literal_string(raw, 0);
        assert_eq!(s, b"a(b)c\n".to_vec(), "转义序列应还原：括号、反斜杠、换行");
        assert_eq!(next, raw.len(), "读完整个字面串才停");

        // 嵌套圆括号不提前结束
        let (s2, _) = read_literal_string(b"(x(y)z)", 0);
        assert_eq!(s2, b"x(y)z".to_vec());
    }

    #[test]
    fn hex_string_decodes_including_odd_length() {
        let (s, _) = read_hex_string(b"<4E2D>", 0);
        assert_eq!(s, vec![0x4E, 0x2D]);
        // 奇数位补 0
        let (s2, _) = read_hex_string(b"<414>", 0);
        assert_eq!(s2, vec![0x41, 0x40]);
    }

    // ---------------------------------------------------------------- 显示运算符

    #[test]
    fn tj_tj_and_quote_operators_produce_lines() {
        let content = b"BT /F1 12 Tf 72 700 Td (line1) Tj ET \
                       BT /F1 12 Tf 72 680 Td [(seg-) 20 (two)] TJ ET \
                       BT /F1 12 Tf 72 660 Td (third) Tj T* ET";
        let lines = extract_text_lines(content, &FontCMaps::default(), &FontEncodings::default());
        // 连续 Tj 且中间没有 Tm 定位 → 它们就是同一行的延续（这是正确行为）。
        // 要断行必须靠 Tm 的 y 变化，见 per_char_tm_positioning_joins_into_one_line。
        assert_eq!(
            lines,
            vec!["line1seg-twothird"],
            "TJ 数组与同行的连续 Tj 应拼接"
        );
    }

    // ---------------------------------------------------------------- 黑盒与诚实失败

    #[test]
    fn non_pdf_and_encrypted_fail_loudly() {
        assert_eq!(extract(b"not a pdf at all"), Err(PdfTextError::NotPdf));

        let enc = min_pdf(b"BT (x) Tj ET");
        let mut with_encrypt = enc.clone();
        with_encrypt.extend_from_slice(b"/Encrypt 9 0 R\n");
        assert_eq!(extract(&with_encrypt), Err(PdfTextError::Encrypted));
    }

    #[test]
    fn empty_text_layer_is_reported_not_silently_empty() {
        // 一个合法 PDF，流里没有任何显示运算符（扫描件特征）
        let pdf = min_pdf(b"BT /F1 12 Tf ET");
        assert_eq!(
            extract(&pdf),
            Err(PdfTextError::NoTextLayer),
            "无文本层必须明说，不能返回空"
        );
    }

    #[test]
    fn extraction_is_deterministic() {
        let content = b"BT /F1 12 Tf (A) Tj ET BT /F1 12 Tf (B) Tj ET";
        let pdf = min_pdf(content);
        let a = extract(&pdf).unwrap();
        let b = extract(&pdf).unwrap();
        assert_eq!(a, b, "纯函数：同输入同输出");
    }
    // ---------------------------------------------------------------- 真件黄金例

    /// 黄金例：仓内自带的 C-补遗文件.pdf（ReportLab 生成的双重编码中文 PDF）。
    ///
    /// 这正是本模块要救的那个文件：在没有 pdfium 也没有 poppler 的环境里，
    /// 它之前被整个丢弃，导致补遗里的新控制价/新截止时间全部没进抽取面。
    #[test]
    fn golden_addendum_pdf_is_fully_recovered() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/synth-tender/招标文件/C-补遗文件.pdf");
        // 缺夹具时**不能用「pass」冒充通过**——黄金例是这套兜底逻辑唯一的真值来源。
        assert!(
            p.exists(),
            "缺少黄金例夹具 {}（issue #9 SKIP 口径要求响亮横幅，不能静默通过）",
            p.display()
        );
        let bytes = std::fs::read(&p).expect("读黄金例");
        let t = extract(&bytes).expect("补遗 PDF 应能靠纯 Rust 提取");
        let all = t.lines().join("\n");

        // 五个核心事实面（与 2026-10-31 的人工核对一致）
        let must = [
            ("补遗文件（01 号）", "标题"),
            ("2026-11-30 09:30", "新开标时间"),
            ("1580 万元", "调整后的控制价"),
            ("回执确认函", "新增的否决条款"),
            ("最多 4 分", "商务分修正"),
        ];
        for (needle, label) in must {
            assert!(
                all.contains(needle),
                "黄金例应含「{label}」({needle})，实际提取：\n{all}"
            );
        }
        assert!(t.has_text());
        assert!(t.pages.len() >= 2, "至少两页有文本，实际 {}", t.pages.len());
    }

    /// 真件测试：仓内自带的真实招标文件 PDF（重庆西永水冷空调）。
    ///
    /// 这个文件是 `/Type0 + /Identity-H + /ToUnicode` 形态——国内招标文件最普遍的
    /// 编码方式。实测结论（不夸大）：
    /// - CMap 命中 860 条，**中文字符全部正确还原**；
    /// - 行切分靠 Tm/CTM 定位，能把逐字 Tj 拼回成行；
    /// - **词间空格不保留**（没有字宽信息，强插空格会把「中文」变「中 文」）；
    ///   这在业务上是可接受的退化——抽取层用的是字词不是排版；
    /// - 表格内容会按单元格拆成多段（本模块不做表格重构——那是引擎/导出层的事）。
    #[test]
    fn real_tender_pdf_with_identity_h_cmap_is_decoded() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../real-corpus/telecom/parse-input/A-cqyd-20260500728-重庆西永水冷空调-招标文件.pdf");
        assert!(p.exists(), "缺少真件夹具 {}（不能静默通过）", p.display());
        let bytes = std::fs::read(&p).expect("读黄金例");

        // CMap 必须被读到（否则后面的中文判断都是假的）
        let cm = collect_cmaps(&bytes);
        assert!(
            cm.single.len() > 100,
            "应读到大量 bfchar 映射，实际 {}",
            cm.single.len()
        );

        let t = extract(&bytes).expect("真件应能提取");
        assert!(t.has_text(), "真件应有文本层");

        // 把所有行拼起来找业务词：这些词在原文里是连续中文，
        // 所以只要 CMap 生效就必然出现。
        let all: String = t.lines().join("");
        for kw in ["重庆", "水冷", "空调", "招标人", "中国移动通信"] {
            assert!(
                all.contains(kw),
                "真件应含「{kw}」（前 400 字符：{}）",
                all.chars().take(400).collect::<String>()
            );
        }
        assert!(
            t.pages.len() >= 20,
            "真件应有多页文本，实际 {}",
            t.pages.len()
        );
    }
    #[test]
    fn per_char_tm_positioning_joins_into_one_line() {
        let content = b"BT /F1 12 Tf 1 0 0 -1 100 50 Tm (B) Tj ET \
                       BT /F1 12 Tf 1 0 0 -1 90 50 Tm (A) Tj ET \
                       BT /F1 12 Tf 1 0 0 -1 110 50 Tm (C) Tj ET \
                       BT /F1 12 Tf 1 0 0 -1 80 70 Tm (N) Tj ET";
        let lines = extract_text_lines(content, &FontCMaps::default(), &FontEncodings::default());
        assert_eq!(
            lines,
            vec!["ABC", "N"],
            "同 y 应按 x 拼行；不同 y 应断行；实际 {lines:?}"
        );
    }
    // ---------------------------------------------------------------- #218：ToUnicode / 字体路由 / Tm 缩放

    /// #218 事故形态（水利施工标段实测）：对象头尾号撞车——字体字典声明
    /// `/ToUnicode 9 0 R`，而文件前部存在 `79 0 obj`（其字节含 "9 0 obj"
    /// 尾部子串）。旧实现的 find 无数字边界 → 命中 79 的尾部 → 拿错对象
    /// → CMap 空 → Identity-H 码值直出乱码。新实现三重防护必须拿对 9 号。
    #[test]
    fn tounicode_object_head_digit_boundary() {
        // CMap：CID 0x0001→'中'、0x0002→'文'（未压缩流——零 Filter 依赖）。
        let cmap_stream =
            b"begincmap\n2 beginbfchar\n<0001> <4E2D>\n<0002> <6587>\nendbfchar\nendcmap";
        let content = b"BT /F1 12 Tf 1 0 0 -1 100 50 Tm <00010002> Tj ET";
        let mut v = Vec::new();
        v.extend_from_slice(b"%PDF-1.4\n");
        // 干扰对象 79（在 9 之前）：无 ToUnicode 语义的垃圾流——
        // 旧实现会把它当 9 号的流（"79 0 obj" 尾部子串命中 "9 0 obj"）。
        v.extend_from_slice(b"79 0 obj\n<< /Length 8 >>\nstream\nJUNKJUNK\nendstream\nendobj\n");
        // 字体 4：Type0 + Identity-H + ToUnicode 9。
        v.extend_from_slice(
            b"4 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /SimSun /Encoding /Identity-H \
              /DescendantFonts [6 0 R] /ToUnicode 9 0 R >>\nendobj\n",
        );
        v.extend_from_slice(
            b"6 0 obj\n<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SimSun \
              /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> >>\nendobj\n",
        );
        // 真 CMap 对象 9（在 79 之后——旧实现的首次搜索必然先撞上 79）。
        v.extend_from_slice(
            format!("9 0 obj\n<< /Length {} >>\nstream\n", cmap_stream.len()).as_bytes(),
        );
        v.extend_from_slice(cmap_stream);
        v.extend_from_slice(b"\nendstream\nendobj\n");
        v.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
              /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        );
        v.extend_from_slice(
            format!("5 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        v.extend_from_slice(content);
        v.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");

        let t = extract(&v).expect("应能提取");
        let all: String = t.lines().join("");
        assert!(
            all.contains("中文"),
            "CMap 必须从 9 号对象读到（对象头数字边界）——实际：{all:?}"
        );
    }

    /// #218：非流对象不跨对象取流——`/ToUnicode N 0 R` 指向的对象 N 若是
    /// 字典/注释（无自己的 stream），旧实现会拿 N 之后**别人的**流当 N 的。
    #[test]
    fn tounicode_non_stream_object_not_borrowing_foreign_stream() {
        let cmap_stream = b"begincmap\n1 beginbfchar\n<0001> <4E2D>\nendbfchar\nendcmap";
        // #223 修 A：混入可打印 CID 0x0041（'A'）——纯 <0001> 件清洗后无文本层，
        // 改后不借流剩 "A"（可打印），借流仍解出「中」被断言抓到，强度不变。
        let content = b"BT /F1 12 Tf 1 0 0 -1 100 50 Tm <00410001> Tj ET";
        let mut v = Vec::new();
        v.extend_from_slice(b"%PDF-1.4\n");
        // 对象 9：**非流**字典对象（只有 endobj，无 stream）——但后面 12 号有流。
        v.extend_from_slice(b"9 0 obj\n<< /Type /XObj /Subtype /PSXObject >>\nendobj\n");
        v.extend_from_slice(b"12 0 obj\n<< /Length 4 >>\nstream\nJUNK\nendstream\nendobj\n");
        v.extend_from_slice(
            b"4 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /SimSun /Encoding /Identity-H \
              /DescendantFonts [6 0 R] /ToUnicode 9 0 R >>\nendobj\n",
        );
        v.extend_from_slice(b"6 0 obj\n<< /Type /Font /Subtype /CIDFontType2 >>\nendobj\n");
        v.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
              /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        );
        v.extend_from_slice(
            format!("5 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        v.extend_from_slice(content);
        v.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");

        // 9 号是字典对象：绝不能把 12 号的 JUNK 流当它的 CMap。
        // 期望行为：读不到 CMap → 不解出「中」（宁可缺字不错字）。
        let t = extract(&v).expect("应能提取");
        let all: String = t.lines().join("");
        assert!(
            !all.contains("中"),
            "非流对象不得借别人的流当 CMap——实际：{all:?}"
        );
    }

    /// #218：按字体路由 CMap——多字体 CID 空间重叠时，全局合并 or_insert
    /// 先到先得会错字（实测：Calibri 的 CID 0x0011→'B' 先占，SimSun 的
    /// '.'（同 CID）全变 'B'，'4.2'→'4B2'）。各字体必须查各自的表。
    #[test]
    fn per_font_cmap_routing_no_cross_font_override() {
        // 字体 FT1（SimSun 形态）：CID 0x0011→'.'；字体 FT2（Calibri 形态）：
        // CID 0x0011→'B'。/ToUnicode 出现序 FT2 在前（模拟事故的先占序）。
        let cm1 = b"begincmap\n1 beginbfchar\n<0011> <002E>\nendbfchar\nendcmap";
        let cm2 = b"begincmap\n1 beginbfchar\n<0011> <0042>\nendbfchar\nendcmap";
        // 内容流：FT1 在左（x=100）出 '.'，FT2 在右（x=200）出 'B'。
        let content = b"BT /FT1 12 Tf 1 0 0 -1 100 50 Tm <0011> Tj ET \
                        BT /FT2 12 Tf 1 0 0 -1 200 50 Tm <0011> Tj ET";
        let mut v = Vec::new();
        v.extend_from_slice(b"%PDF-1.4\n");
        // FT2 的字体字典 + CMap 先出现（对象 21/22）——先占序。
        v.extend_from_slice(
            b"21 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /Calibri /Encoding /Identity-H \
              /DescendantFonts [26 0 R] /ToUnicode 22 0 R >>\nendobj\n",
        );
        v.extend_from_slice(format!("22 0 obj\n<< /Length {} >>\nstream\n", cm2.len()).as_bytes());
        v.extend_from_slice(cm2);
        v.extend_from_slice(b"\nendstream\nendobj\n");
        // FT1 的字体字典 + CMap（对象 11/12）。
        v.extend_from_slice(
            b"11 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /SimSun /Encoding /Identity-H \
              /DescendantFonts [16 0 R] /ToUnicode 12 0 R >>\nendobj\n",
        );
        v.extend_from_slice(format!("12 0 obj\n<< /Length {} >>\nstream\n", cm1.len()).as_bytes());
        v.extend_from_slice(cm1);
        v.extend_from_slice(b"\nendstream\nendobj\n");
        v.extend_from_slice(b"16 0 obj\n<< /Type /Font /Subtype /CIDFontType2 >>\nendobj\n");
        v.extend_from_slice(b"26 0 obj\n<< /Type /Font /Subtype /CIDFontType2 >>\nendobj\n");
        v.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
              /Resources << /Font << /FT1 11 0 R /FT2 21 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        );
        v.extend_from_slice(
            format!("5 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        v.extend_from_slice(content);
        v.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");

        let t = extract(&v).expect("应能提取");
        let all: String = t.lines().join("");
        assert!(
            all.contains('.') && all.contains('B'),
            "按字体路由：FT1 出 '.'、FT2 出 'B'（同 CID 0x0011 不得互相覆盖）——实际：{all:?}"
        );
    }

    /// #218：Tm 缩放系数——WPS 形态把大字号放进 Tf（如 319）、用 0.05 的
    /// Tm 落位，Td/TD 推进量是文本空间单位。不乘缩放系数则行内 x 序错乱
    /// （实测：'标段编号：E6532…' 被排成 '标E6段53编20号03…' 交错混排）。
    #[test]
    fn tm_scaled_td_advances_keep_line_order() {
        // FT1（中文）：Tm 起点 x=110，逐字 TD 319.188（×0.05 = 15.96pt 全角步进）。
        // FT2（西文）：Tm 起点 x=190，逐字 TD 160.797（×0.05 = 8.04pt 半角步进）。
        // 旧实现不乘系数：FT1 第二字 x=110+319.19=429 > FT2 两字（190/350.8）
        //   → 行序「甲 丙 丁 乙」（错——视觉序应为 甲 乙 丙 丁）。
        // 新实现乘系数：110, 125.96 | 190, 198.04 → 「甲 乙 丙 丁」✓。
        let content = b"BT /F1 319 Tf 0.05 0 0 -0.05 110 100 Tm <0001> Tj 319.188 -0 TD <0002> Tj ET \
                        BT /F2 319 Tf 0.05 0 0 -0.05 190 100 Tm <0001> Tj 160.797 -0 TD <0002> Tj ET";
        let cm1 = b"begincmap\n2 beginbfchar\n<0001> <7532>\n<0002> <4E59>\nendbfchar\nendcmap"; // 甲乙
        let cm2 = b"begincmap\n2 beginbfchar\n<0001> <4E19>\n<0002> <4E01>\nendbfchar\nendcmap"; // 丙丁
        let mut v = Vec::new();
        v.extend_from_slice(b"%PDF-1.4\n");
        v.extend_from_slice(
            b"11 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /SimSun /Encoding /Identity-H \
              /DescendantFonts [16 0 R] /ToUnicode 12 0 R >>\nendobj\n",
        );
        v.extend_from_slice(format!("12 0 obj\n<< /Length {} >>\nstream\n", cm1.len()).as_bytes());
        v.extend_from_slice(cm1);
        v.extend_from_slice(b"\nendstream\nendobj\n");
        v.extend_from_slice(b"16 0 obj\n<< /Type /Font /Subtype /CIDFontType2 >>\nendobj\n");
        v.extend_from_slice(
            b"21 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /Calibri /Encoding /Identity-H \
              /DescendantFonts [26 0 R] /ToUnicode 22 0 R >>\nendobj\n",
        );
        v.extend_from_slice(format!("22 0 obj\n<< /Length {} >>\nstream\n", cm2.len()).as_bytes());
        v.extend_from_slice(cm2);
        v.extend_from_slice(b"\nendstream\nendobj\n");
        v.extend_from_slice(b"26 0 obj\n<< /Type /Font /Subtype /CIDFontType2 >>\nendobj\n");
        v.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
              /Resources << /Font << /F1 11 0 R /F2 21 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        );
        v.extend_from_slice(
            format!("5 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        v.extend_from_slice(content);
        v.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");

        let t = extract(&v).expect("应能提取");
        let lines = t.lines();
        assert!(
            lines.iter().any(|l| l.contains("甲乙丙丁")),
            "Tm 缩放系数必须作用于 TD 推进（视觉序 甲乙丙丁）——实际：{lines:?}"
        );
    }

    /// #218 真件回归：仓内水利施工标段 PDF（事故文件）——ToUnicode CMap
    /// 三重修复后首页关键行必须精确还原（旧实现：全文乱码、条款 0 条）。
    #[test]
    fn real_water_tender_tounicode_decodes_header_lines() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../real-corpus/water/files/A-某水利施工标段招标文件.pdf");
        assert!(p.exists(), "缺少真件夹具 {}（不能静默通过）", p.display());
        let pf = parse_via_fallback(&p, "A-某水利施工标段招标文件.pdf")
            .expect("水利真件兜底应成功（ToUnicode 修复后）");
        let all = parsed_text_of(&pf);
        assert!(
            all.contains("标段编号：E6532003923001526001003"),
            "封面关键行必须精确还原——前 200 字符：{}",
            all.chars().take(200).collect::<String>()
        );
        assert!(all.contains("标段名称"), "标段名称行应可读");
        assert!(
            !all.contains("4B2") || !all.contains("BBBB"),
            "目录点线/条款号不得再被跨字体覆盖成 B 串"
        );
    }

    /// #218 后续：BT…ET 文本窗口——CMap 流（无显示运算符上下文）的字串
    /// 不许被当正文产出。
    ///
    /// 实测缺口形态：ToUnicode CMap 流解压后全是 `<hex> <hex>` 映射对，没有
    /// 任何 BT/ET——旧逻辑把映射对逐个 push 进 pending，在流尾 flush 成巨型
    /// 垃圾行。本用例同时验证「窗口前 pending 丢弃」（BT 开启时清仓）与
    /// 「窗口外不产出」（无 BT 的流零行）。
    #[test]
    fn bt_et_window_blocks_cmap_stream_garbage() {
        let content = b"BT /F1 12 Tf (Hi) Tj ET";
        let mut v = font_pdf("F1", LATIN_FONT_DICT, content);
        // 追加一个不被任何字典引用的裸 CMap 流对象——纯垃圾源。
        // 旧逻辑：流尾 flush 产出 `<0003>` 的解码残渣行；新逻辑：无 BT 窗口 → 零行。
        v.extend_from_slice(
            b"99 0 obj\n<< /Length 40 >>\nstream\nbeginbfchar\n<0003> <4E2D>\n<0004> <6587>\nendbfchar\nendstream\nendobj\n",
        );
        let t = extract(&v).expect("应能提取");
        assert_eq!(
            t.lines(),
            vec!["Hi".to_string()],
            "CMap 流的映射对不许混进正文：{:?}",
            t.lines()
        );
    }

    /// #218 后续真件断言：BT…ET 窗口后，目标件 CJK 占比必须回到真实水平。
    ///
    /// main 基线（#220 修复后仍缺窗口）：9 个 CMap 流的垃圾产出把 CJK 占比
    /// 稀释到 0.067（真实值 0.690，poppler 同口径 0.689）——垃圾不仅污染
    /// chunk/检索面，还**污染乱码探测判据**（见下一个 C-zjyd 用例）。
    #[test]
    fn bt_et_window_restores_true_cjk_ratio() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../real-corpus/water/files/A-某水利施工标段招标文件.pdf");
        assert!(p.exists(), "缺少真件夹具 {}（不能静默通过）", p.display());
        let pf = parse_via_fallback(&p, "A-某水利施工标段招标文件.pdf").expect("真件应能提取");
        let all = parsed_text_of(&pf);
        assert!(
            cjk_ratio(&all) > 0.5,
            "CJK 占比应回到真实水平（poppler 同口径 0.689；被 CMap 垃圾流稀释时为 0.067），实际 {:.3}",
            cjk_ratio(&all)
        );
    }

    /// #218 后续真件断言：垃圾稀释曾把**正确解码**的文件误判成乱码——
    /// C-zjyd 变更公告在 main 基线上 probe=Garbled（cjk 0.014 < 0.02 阈值，
    /// 被垃圾字符稀释），窗口后回到 0.809/Clean。乱码探测的灵敏度
    /// 依赖文本面干净。
    #[test]
    fn bt_et_window_unpoisons_garble_probe() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../../real-corpus/telecom/parse-input/C-zjyd-20260500552-数据库服务器-变更公告.pdf",
        );
        assert!(p.exists(), "缺少真件夹具 {}（不能静默通过）", p.display());
        let bytes = std::fs::read(&p).expect("读真件");
        let pf = parse_via_fallback(&p, "C-zjyd-20260500552-数据库服务器-变更公告.pdf")
            .expect("变更公告应能提取（文本层完整）");
        let all = parsed_text_of(&pf);
        assert!(
            cjk_ratio(&all) > 0.5,
            "正确解码的中文公告 CJK 占比应过半（垃圾稀释时 0.014 会被误判乱码），实际 {:.3}",
            cjk_ratio(&all)
        );
        assert_ne!(
            probe_cjk_garble(&bytes, &all),
            CjkGarbleProbe::Garbled,
            "正确解码的文件不得再被误判乱码"
        );
    }

    /// #218 乱码可见性：声明中文字体而兜底文本 CJK 占比 <2% → parse_summary
    /// 必须显式标注「疑似乱码」（Agent 在导入回执即知文本层不可用）。
    #[test]
    fn fallback_summary_flags_suspected_garble() {
        // 声明 SimSun（CJK 字体 marker 命中）+ 无 ToUnicode + CID 直出
        // （0x0043/0x0032/0x0030/0x0031 按 UTF-16BE → 可打印 "C201"，非 CJK 区）
        // → 占比 0 → 必须标注。#223 修 A 后改用可打印 CID：纯控制符件会被
        // 行级清洗为空 → 诚实报无文本层（见 control_only_text_is_empty），
        // 而 #218 的真实乱码形态（Latin 残渣）是可打印的，不受修 A 影响。
        let content = b"BT /F1 12 Tf 1 0 0 -1 100 50 Tm <0043003200300031> Tj ET";
        let v = font_pdf(
            "F1",
            "<< /Type /Font /Subtype /Type0 /BaseFont /SimSun /Encoding /Identity-H \
               /DescendantFonts [6 0 R] >>",
            content,
        );
        let dir = std::env::temp_dir().join("bid_parse_218_garble_probe.pdf");
        std::fs::write(&dir, &v).expect("写临时 PDF");
        let pf = parse_via_fallback(&dir, "garble-probe.pdf").expect("应能提取（乱码不拒收）");
        assert!(
            pf.parse_summary.contains("疑似乱码"),
            "summary 必须标注疑似乱码——实际：{}",
            pf.parse_summary
        );
        let all = parsed_text_of(&pf);
        assert!(all.contains("C201"), "可打印 CID 应直出：{all:?}");
        let _ = std::fs::remove_file(&dir);
    }

    /// #223 修 A 新语义：纯控制符「文本」被行级清洗为空 → 诚实报无文本层。
    ///
    /// 修前：U+0001-U+0004 算 has_text → 兜底产出 4 个控制符的空壳「成功」
    /// （假成功面）；修后：清洗后零行 → NoTextLayer（与「只抽到空白」同判）。
    #[test]
    fn control_only_text_is_empty_not_text_layer() {
        let content = b"BT /F1 12 Tf 1 0 0 -1 100 50 Tm <0001000200030004> Tj ET";
        let v = font_pdf(
            "F1",
            "<< /Type /Font /Subtype /Type0 /BaseFont /SimSun /Encoding /Identity-H /DescendantFonts [6 0 R] >>",
            content,
        );
        let dir = std::env::temp_dir().join("bid_parse_223_control_only.pdf");
        std::fs::write(&dir, &v).expect("写临时 PDF");
        let err = parse_via_fallback(&dir, "control-only.pdf")
            .expect_err("纯控制符件应诚实报无文本层（#223 修 A）");
        assert!(err.contains("无文本层"), "报错应点名无文本层：{err}");
        let _ = std::fs::remove_file(&dir);
    }

    // ---------------------------------------------------------------- #200：UniXX-UCS2 恒等解码

    /// Type0 CID 字体字典（UniGB-UCS2-H 预定义 CMap，无 ToUnicode——#200 形态）。
    const UC2_FONT_DICT: &str = "<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light \
         /Encoding /UniGB-UCS2-H /DescendantFonts [6 0 R] >>";
    /// Type1 拉丁字体字典（无 UniXX 声明——恒等解码资格必须为 false）。
    const LATIN_FONT_DICT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";

    /// 完整单页 PDF：/Font << /<res> 4 0 R >>，内容流未压缩，附占位 CID 后代字体。
    fn font_pdf(res: &str, font_dict: &str, content: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        v.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        v.extend_from_slice(
            format!(
                "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
                 /Resources << /Font << /{res} 4 0 R >> >> /Contents 5 0 R >>\nendobj\n"
            )
            .as_bytes(),
        );
        v.extend_from_slice(format!("4 0 obj\n{font_dict}\nendobj\n").as_bytes());
        v.extend_from_slice(
            b"6 0 obj\n<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light \
              /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> >>\nendobj\n",
        );
        v.extend_from_slice(
            format!("5 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        v.extend_from_slice(content);
        v.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");
        v
    }

    /// 双字体页（F1 拉丁 + F2 UniGB-UCS2）——混排安全测试用。
    fn mixed_font_pdf(f1_dict: &str, f2_dict: &str, content: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        v.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        v.extend_from_slice(
            format!(
                "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
                 /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 7 0 R >>\nendobj\n"
            )
            .as_bytes(),
        );
        v.extend_from_slice(format!("4 0 obj\n{f1_dict}\nendobj\n").as_bytes());
        v.extend_from_slice(format!("5 0 obj\n{f2_dict}\nendobj\n").as_bytes());
        v.extend_from_slice(
            b"6 0 obj\n<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light \
              /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> >>\nendobj\n",
        );
        v.extend_from_slice(
            format!("7 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        v.extend_from_slice(content);
        v.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");
        v
    }

    fn all_text(pdf: &[u8]) -> String {
        let t = extract(pdf).expect("提取应成功");
        t.pages
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// #200 核心：UniGB-UCS2-H 字体的**逐字 Tj 单字串**（2 字节）此前全落
    /// Latin-1 分支（len<4 过不了 looks_utf16be 门槛）——字体上下文恒等
    /// 解码后精确还原。
    #[test]
    fn uc2_single_char_tj_decodes_via_identity() {
        // 本=U+672C 项=U+9879 目=U+76EE：每字一个 Tj、Tm 逐字定位（同 y 拼行）
        let content = b"BT /F2 12 Tf 1 0 0 1 50 800 Tm (\x67\x2c) Tj \
                        1 0 0 1 62 800 Tm (\x98\x79) Tj \
                        1 0 0 1 74 800 Tm (\x76\xee) Tj ET";
        let pdf = font_pdf("F2", UC2_FONT_DICT, content);
        let text = all_text(&pdf);
        assert!(
            text.contains("本项目"),
            "恒等解码应还原「本项目」，实际：{text:?}"
        );
        // probe Clean 的整链路断言在 tests/pdf_cid_identity.rs（文本量过 32 字门槛）
    }

    /// 安全前提：恒等解码资格只给声明了 UniXX-UCS2 的字体——拉丁字体的
    /// 2 字节 ASCII 串 "kV"（0x6B56 恰落 CJK 区，若误入恒等路会变「敖」）
    /// 必须原样保留。
    #[test]
    fn latin_font_short_pairs_stay_latin() {
        let content = b"BT /F1 12 Tf 72 720 Td (kV 12mm) Tj ET";
        let pdf = font_pdf("F1", LATIN_FONT_DICT, content);
        let text = all_text(&pdf);
        assert!(text.contains("kV"), "拉丁串应原样保留，实际：{text:?}");
        assert!(
            !text.contains('\u{6556}'),
            "拉丁串绝不能被恒等解码成 CJK 错字"
        );
    }

    /// 混排：同一页 F1 拉丁 + F2 UniGB——两套字体各按各的编码解码，互不串扰。
    #[test]
    fn mixed_fonts_uc2_and_latin_independent() {
        let content = b"BT /F1 12 Tf 72 720 Td (kV) Tj ET \
                        BT /F2 12 Tf 1 0 0 1 50 700 Tm (\x67\x2c) Tj \
                        1 0 0 1 62 700 Tm (\x98\x79) Tj ET";
        let pdf = mixed_font_pdf(LATIN_FONT_DICT, UC2_FONT_DICT, content);
        let text = all_text(&pdf);
        assert!(text.contains("kV"), "拉丁串原样：{text:?}");
        assert!(text.contains("本项"), "CID 串恒等解码：{text:?}");
        assert!(!text.contains('\u{6556}'), "不得串扰：{text:?}");
    }

    /// 十六进制串形态的逐字显示（Word/WPS 常见）走同一条恒等路。
    #[test]
    fn uc2_hex_string_single_char_decodes() {
        let content = b"BT /F2 12 Tf 1 0 0 1 50 800 Tm <672C> Tj \
                        1 0 0 1 62 800 Tm <9879> Tj ET";
        let pdf = font_pdf("F2", UC2_FONT_DICT, content);
        let text = all_text(&pdf);
        assert!(text.contains("本项"), "hex 单字应恒等解码，实际：{text:?}");
    }

    /// 收集器单元面：资源名 → uc2 判定准确（F2 是、F1 不是）。
    #[test]
    fn collect_font_encodings_resolves_resource_names() {
        let content = b"BT /F2 12 Tf 72 720 Td (x) Tj ET";
        let pdf = mixed_font_pdf(LATIN_FONT_DICT, UC2_FONT_DICT, content);
        let fonts = collect_font_encodings(&pdf);
        assert!(fonts.is_uc2("F2"), "F2（UniGB-UCS2-H）应被识别");
        assert!(!fonts.is_uc2("F1"), "F1（Helvetica）不是 uc2 字体");
    }

    /// decode_text_string 三态：同一 2 字节串，字体上下文决定解码路。
    #[test]
    fn decode_text_string_identity_gated_by_font_context() {
        // 本 = U+672C：uc2 字体串 → 恒等还原；非 uc2 字体串 → 旧行为（Latin-1）
        assert_eq!(decode_text_string_fc(b"\x67\x2c", &EMPTY, true), "本");
        assert_eq!(decode_text_string_fc(b"\x67\x2c", &EMPTY, false), "g,");
        // 奇数长度不进恒等路（fail-safe）
        assert_eq!(
            decode_text_string_fc(b"\x67\x2c\x01", &EMPTY, true),
            "g,\u{1}"
        );
    }

    // ---------------------------------------------------------------- #223 修 A：控制字符清洗

    /// sanitize_line：剔除 C0（除 \t\n\r）/DEL/C1，正文与合法控制零损伤。
    #[test]
    fn sanitize_line_strips_control_chars_keeps_text() {
        assert_eq!(sanitize_line("投标\u{00}文件\u{1f}合格"), "投标文件合格");
        assert_eq!(sanitize_line("中文\u{80}混\u{9f}排"), "中文混排");
        assert_eq!(sanitize_line("a\u{7f}b"), "ab");
        // 合法排版控制保留
        assert_eq!(sanitize_line("a\tb\nc\rd"), "a\tb\nc\rd");
        // 正文零损伤
        assert_eq!(
            sanitize_line("第3.2条 不接受联合体"),
            "第3.2条 不接受联合体"
        );
        assert_eq!(sanitize_line(""), "");
    }

    /// 事故回归：Latin-1 直映（b as char）产出的 C1 残渣必须被行级清洗
    /// （B-cqyd 157,029 个控制符把 cjk_ratio 拉到 0.0086 的根因面）。
    #[test]
    fn flush_path_strips_c1_residue_from_latin1_bytes() {
        // 非 uc2 字体 + 含 0x80-0x9F 裸字节的字面串 → Latin-1 直映 → C1 进正文
        let content = b"BT /F1 12 Tf 72 720 Td (Re\x87sultat \x93\x94garbage\x95\x94) Tj ET";
        let pdf = font_pdf("F1", LATIN_FONT_DICT, content);
        let lines = extract(&pdf).expect("提取成功").lines();
        let joined = lines.concat();
        assert!(
            !joined
                .chars()
                .any(|c| matches!(c, '\u{00}'..='\u{08}' | '\u{0B}'..='\u{0C}' | '\u{0E}'..='\u{1F}' | '\u{7F}' | '\u{80}'..='\u{9F}')),
            "行面不得再有 C0/C1 残渣（#223 修 A）：{lines:?}"
        );
    }

    // ---------------------------------------------------------------- #223 修 B：分页多数决

    /// 混合形态：2 正常中文页 + 1 垃圾页 → Clean + 垃圾页单独点名
    /// （B-cqyd 实测形态：前 4 页正常、第 5 流组全控制符——修前整份误杀）。
    #[test]
    fn paged_probe_mixed_pages_majority_clean() {
        let pdf = font_pdf(
            "F2",
            UC2_FONT_DICT,
            b"BT /F2 12 Tf 72 720 Td (\xb1\xbe) Tj ET",
        );
        let good = "本项目为2026年度数据中心水冷空调群控系统采购项目招标公告，本项目为2026年度数据中心水冷空调群控系统采购项目";
        let garbled: String = "\u{93}".repeat(40);
        let pages = vec![good.to_string(), good.to_string(), garbled];
        let v = probe_cjk_garble_pages(&pdf, &pages);
        assert_eq!(
            v.verdict,
            CjkGarbleProbe::Clean,
            "多数页正常应 Clean（修前拉平判据误杀）：{:?}",
            v.garbled_pages
        );
        assert_eq!(v.garbled_pages, vec![3], "垃圾页应单独点名");
    }

    /// 全部有效页皆乱 → 仍 Garbled（fail-loud 红线：多数决不救全乱）。
    #[test]
    fn paged_probe_all_garbled_still_fails_loud() {
        let pdf = font_pdf(
            "F2",
            UC2_FONT_DICT,
            b"BT /F2 12 Tf 72 720 Td (\xb1\xbe) Tj ET",
        );
        let garbled: String = "SbheN\u{93}\u{94}".repeat(10);
        let pages = vec![garbled.clone(), garbled.clone(), garbled];
        let v = probe_cjk_garble_pages(&pdf, &pages);
        assert_eq!(v.verdict, CjkGarbleProbe::Garbled, "全乱必须继续判乱");
        assert_eq!(v.garbled_pages, vec![1, 2, 3]);
    }

    /// 平票（2 正 2 乱）保守判 Garbled——不放松。
    #[test]
    fn paged_probe_tie_is_conservative_garbled() {
        let pdf = font_pdf(
            "F2",
            UC2_FONT_DICT,
            b"BT /F2 12 Tf 72 720 Td (\xb1\xbe) Tj ET",
        );
        let good = "本项目为数据中心水冷空调群控系统采购项目招标公告内容，本项目为数据中心水冷空调群控系统采购项目";
        let garbled: String = "\u{93}\u{94}".repeat(40);
        let pages = vec![good.to_string(), good.to_string(), garbled.clone(), garbled];
        let v = probe_cjk_garble_pages(&pdf, &pages);
        assert_eq!(v.verdict, CjkGarbleProbe::Garbled, "平票保守判乱（不放松）");
        assert_eq!(v.garbled_pages, vec![3, 4]);
    }

    /// 无有效页（全部样本不足）→ 退化拉平判据，与 #223 之前行为逐位一致。
    #[test]
    fn paged_probe_degenerates_to_flat_when_no_valid_page() {
        let pdf = font_pdf(
            "F2",
            UC2_FONT_DICT,
            b"BT /F2 12 Tf 72 720 Td (\xb1\xbe) Tj ET",
        );
        // 各页 <32 非空白且合计也不足 → NoSignal
        let v = probe_cjk_garble_pages(&pdf, &["短".to_string(), "页".to_string()]);
        assert_eq!(v.verdict, CjkGarbleProbe::NoSignal);
        // 各页 <32 但合计足且 CJK 正常 → Clean（退化面 == 拉平面）
        let half: String = "本".repeat(20);
        let v2 = probe_cjk_garble_pages(&pdf, &[half.clone(), half]);
        assert_eq!(
            v2.verdict,
            CjkGarbleProbe::Clean,
            "退化面与拉平判据逐位一致"
        );
    }

    /// 无 CJK 字体声明 → NoSignal（判据不适用——英文件永不触发）。
    #[test]
    fn paged_probe_no_cjk_declaration_is_no_signal() {
        let pdf = b"%PDF-1.4\n1 0 obj\nendobj\n%%EOF\n".to_vec();
        let good =
            "本项目为数据中心水冷空调群控系统采购项目招标公告内容重复若干次以满足样本门槛要求";
        let v = probe_cjk_garble_pages(&pdf, &[good.to_string(), good.to_string()]);
        assert_eq!(v.verdict, CjkGarbleProbe::NoSignal);
    }

    // ---------------------------------------------------------------- #223：locator 分桶

    /// page_no_of_locator：三种在产格式 + 无法解析返回 None。
    #[test]
    fn page_no_of_locator_accepts_three_production_formats() {
        assert_eq!(page_no_of_locator("第1页 行2"), Some(1));
        assert_eq!(page_no_of_locator("第12 页 > 第3 行"), Some(12));
        assert_eq!(page_no_of_locator("第2页 OCR 行5"), Some(2));
        assert_eq!(page_no_of_locator("表格 3-1"), None);
        assert_eq!(page_no_of_locator("第页 行1"), None);
    }

    /// parsed_pages_of：按页分桶、空页保留空桶、无法解析兜底尾桶（文本不丢）。
    #[test]
    fn parsed_pages_buckets_by_locator_page_prefix() {
        let mk = |idx: usize, loc: &str, text: &str| crate::Block {
            idx,
            kind: crate::BlockKind::PdfLine,
            locator: loc.to_string(),
            text: text.to_string(),
            cells: Vec::new(),
        };
        let pf = crate::ParsedFile {
            name: "t.pdf".into(),
            sha256: String::new(),
            pages: 3,
            tables: 0,
            has_text_layer: true,
            ocr_blocks: 0,
            blocks: vec![
                mk(0, "第1页 行1", "首页内容"),
                mk(1, "第1 页 > 第2 行", "首页第二行"),
                mk(2, "第3页 行1", "第三页"),
                mk(3, "无页号格式", "尾桶兜底"),
            ],
            parse_summary: String::new(),
            components: Vec::new(),
        };
        let pages = parsed_pages_of(&pf);
        assert_eq!(pages.len(), 3, "应有 3 个页桶：{pages:?}");
        assert!(
            pages[0].contains("首页内容") && pages[0].contains("首页第二行"),
            "第 1 页两行都该在桶内：{:?}",
            pages[0]
        );
        assert!(pages[1].is_empty(), "第 2 页无块应为空桶（不参与多数决）");
        assert!(
            pages[2].contains("第三页") && pages[2].contains("尾桶兜底"),
            "无法解析 locator 的块兜底进尾桶，文本绝不丢失：{:?}",
            pages[2]
        );
    }
}
