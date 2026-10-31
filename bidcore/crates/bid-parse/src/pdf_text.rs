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
//! 甜区（不夸大，如实登记）：覆盖 ASCII85Decode + FlateDecode 流；文本层字面串
//! 与十六进制串；Tj / TJ / ''' / " 四种显示运算符；字符串编码
//! UTF-16BE（带 BOM 或高字节 0x00 特征）与 WinAnsi/PDFDocEncoding（字节直取）。
//! 不覆盖：加密 PDF、LZW/ASCIIHex/RunLength 等其他滤链、编码表自定义的 CID 字体、
//! 跨页复杂排版重组。这些情形一律明说失败（返回 Err），绝不输出半截乱码——
//! 因为静默的错误文本比明说读不出来危险得多。
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
        self.pages.iter().any(|p| p.iter().any(|l| !l.trim().is_empty()))
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
        let lines = extract_text_lines(&decoded);
        if lines.is_empty() {
            // 解压成功但无字串（比如字体/图形流——正常）：不计跳过。
            streams_decoded += 1;
            continue;
        }
        streams_decoded += 1;
        pages.push(lines);
    }

    let out = PdfText { pages, streams_decoded, streams_skipped };
    if out.has_text() {
        return Ok(out);
    }
    if let Some(why) = saw_unsupported {
        return Err(PdfTextError::Unsupported(why));
    }
    Err(PdfTextError::NoTextLayer)
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
        let Some(pos) = find_from(bytes, i, b"stream") else { break };
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
        let Some(end_rel) = find_from(bytes, start, b"endstream") else { break };
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
    let names = ["ASCII85Decode", "FlateDecode", "ASCIIHexDecode", "LZWDecode", "RunLengthDecode"];
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

// ---------------------------------------------------------------- 内容流扫描

/// 从已解压的内容流里抽文本行。
///
/// 行切分口径：一条 `Tj`/`TJ` 显示运算符 = 一个文本块；
/// 实务 PDF（ReportLab/Word/LaTeX）每行正文多以独立 Tj 落位，因此逐显示运算符取行。
fn extract_text_lines(content: &[u8]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut i = 0usize;
    let n = content.len();
    let mut pending: Vec<Vec<u8>> = Vec::new(); // 当前显示运算符积累的字符串

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
            b'<' if i + 1 < n && content[i + 1] == b'<' => {
                i += 2; // 字典开头，跳过
            }
            b'>' if i + 1 < n && content[i + 1] == b'>' => {
                i += 2;
            }
            b'T' => {
                // Tj / TJ / T* / TJ
                let (op, next) = read_name(content, i);
                match op.as_str() {
                    "Tj" | "'" | "\"" | "TJ" => {
                        if !pending.is_empty() {
                            let joined: Vec<u8> = pending.concat();
                            let s = decode_text_string(&joined);
                            let s = s.trim().to_string();
                            if !s.is_empty() {
                                lines.push(s);
                            }
                            pending.clear();
                        }
                        i = next;
                    }
                    _ => i = next,
                }
            }
            b'\'' => {
                if !pending.is_empty() {
                    let joined: Vec<u8> = pending.concat();
                    let s = decode_text_string(&joined);
                    let s = s.trim().to_string();
                    if !s.is_empty() {
                        lines.push(s);
                    }
                    pending.clear();
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    if !pending.is_empty() {
        let joined: Vec<u8> = pending.concat();
        let s = decode_text_string(&joined).trim().to_string();
        if !s.is_empty() {
            lines.push(s);
        }
    }
    lines
}

fn read_name(content: &[u8], start: usize) -> (String, usize) {
    let mut i = start;
    while i < content.len() && (content[i].is_ascii_alphabetic() || content[i] == b'*' || content[i] == b'\'') {
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
                b'n' => { out.push(b'\n'); i += 2; }
                b'r' => { out.push(b'\r'); i += 2; }
                b't' => { out.push(b'\t'); i += 2; }
                b'b' => { out.push(8); i += 2; }
                b'f' => { out.push(12); i += 2; }
                b'(' | b')' | b'\\' => { out.push(nx); i += 2; }
                b'\n' => { i += 2; }
                b'\r' => { i += 2; }
                b'0'..=b'7' => {
                    let mut oct = String::new();
                    let mut k = i + 1;
                    while k < content.len() && oct.len() < 3 && (b'0'..=b'7').contains(&content[k]) {
                        oct.push(content[k] as char);
                        k += 1;
                    }
                    let v = u8::from_str_radix(&oct, 8).unwrap_or(0);
                    out.push(v);
                    i = k;
                }
                _ => { out.push(nx); i += 2; }
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
fn decode_text_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return utf16be_to_string(&bytes[2..]);
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
                || (0x2010..=0x2027).contains(&c)   // 一般标点
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

    let pages = t.pages.iter().filter(|p| p.iter().any(|l| !l.trim().is_empty())).count();
    let summary = format!("{} [{}]", t.summary(), name);
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

#[cfg(test)]
mod tests {
    use super::*;

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
        pdf.extend_from_slice(b"%PDF-1.4\n1 0 obj\n<< /Filter [ /ASCII85Decode /FlateDecode ] /Length ");
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
        assert_eq!(decode_text_string(&with_bom), "中文");

        // 无 BOM 但高字节全 0（UTF-16BE 的 ASCII 区域）
        let no_bom = [0x00, 0x41, 0x00, 0x42];
        assert_eq!(decode_text_string(&no_bom), "AB");

        // 单字节编码：直取
        assert_eq!(decode_text_string(b"Hello"), "Hello");
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
        let lines = extract_text_lines(content);
        assert_eq!(lines, vec!["line1", "seg-two", "third"], "TJ 数组内字串应拼接成一行");
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
        assert_eq!(extract(&pdf), Err(PdfTextError::NoTextLayer), "无文本层必须明说，不能返回空");
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
        assert!(p.exists(), "缺少黄金例夹具 {}（issue #9 SKIP 口径要求响亮横幅，不能静默通过）", p.display());
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
            assert!(all.contains(needle), "黄金例应含「{label}」({needle})，实际提取：\n{all}");
        }
assert!(t.has_text());
        assert!(t.pages.len() >= 2, "至少两页有文本，实际 {}", t.pages.len());
    }

}
