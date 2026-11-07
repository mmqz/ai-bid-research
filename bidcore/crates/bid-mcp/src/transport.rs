//! 传输面抽象——业务分发与传输实现解耦。
//!
//! M0 只提供 stdio 一种实现（见 `bin/stdio.rs`）。未来 HTTP / wasm 只需实现同一
//! trait，`dispatch` 语义零变化。
//!
//! #377（C-18）：本模块另提供 [`BoundedLineReader`]——std io 分帧的**单行长度
//! 有界**读取器（两个 stdio bin 共用的单一真源）。治理纪律对称：出站面
//! （guard.rs 子进程 stdout 上限/收割）有界，入站面同样有界——对端一条超大行
//! 不再把整行读进内存。

use std::io::BufRead;

use serde_json::json;

use crate::protocol::INVALID_REQUEST;
use crate::schema::ToolReply;

/// 一条入站请求的抽象（原始文本，由 `dispatch_json` 解析）。
pub trait Transport {
    /// 收下一条请求；EOF 返回 `None`。
    fn recv(&mut self) -> Option<String>;
    /// 回一条应答。
    fn send(&mut self, reply: &ToolReply);
}

/// 单行字节上限（#377/C-18）：8 MiB（**含行尾换行计入长度**）。
///
/// 选值理由：现有合法报文面 = 工具调用参数（证据摘录/批量条款行/长文本），
/// 实测远小于 1 MiB；8 MiB 留 8 倍以上余量不误伤，同时把失控/误用/恶意
/// 超大行挡在内存外（超限行流式读弃，不驻留）。
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// [`BoundedLineReader::next_line`] 的结果。
#[derive(Debug)]
pub enum LineOutcome {
    /// 一行（已去行尾 `\n`/`\r\n`——与 `BufRead::lines` 同语义）。
    Line(String),
    /// 行长超上限：**整行已流式读弃**（直奔 `\n`/EOF，未进内存缓冲），
    /// 后续行不受污染、可继续读。调用方应响亮告警 + 回一条错误应答。
    TooLong,
    /// EOF（对端关闭）。
    Eof,
    /// IO 错误（含非法 UTF-8——与 `BufRead::lines` 同语义报 `InvalidData`，
    /// 不做静默替换）。
    Io(std::io::Error),
}

impl PartialEq for LineOutcome {
    /// 测试等值语义：Io 只比「是 Io」不比错误内容（io::Error 无等值；
    /// 错误种类由测试侧单独断言 kind）。
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LineOutcome::Line(a), LineOutcome::Line(b)) => a == b,
            (LineOutcome::TooLong, LineOutcome::TooLong)
            | (LineOutcome::Eof, LineOutcome::Eof)
            | (LineOutcome::Io(_), LineOutcome::Io(_)) => true,
            _ => false,
        }
    }
}
impl Eq for LineOutcome {}

/// 有界行读取器：任意 [`BufRead`] 上的「单行不超上限」分帧（#377/C-18）。
///
/// 与裸 `lines()` 的唯一行为差异：超限行返回 [`LineOutcome::TooLong`]
/// 而不是把整行读进内存。正常行语义（含 `\r\n` 剥离、EOF 末行无换行、
/// 空 UTF-8 校验）逐项对齐 `BufRead::lines`。
pub struct BoundedLineReader<R: BufRead> {
    inner: R,
    limit: usize,
    buf: Vec<u8>,
}

impl<R: BufRead> BoundedLineReader<R> {
    /// 生产构造（上限 = [`MAX_LINE_BYTES`]）。
    pub fn new(inner: R) -> Self {
        Self::with_limit(inner, MAX_LINE_BYTES)
    }

    /// 自定义上限构造（单测钉边界用；生产走 [`new`]）。
    pub fn with_limit(inner: R, limit: usize) -> Self {
        Self {
            inner,
            limit,
            buf: Vec::new(),
        }
    }

    /// 读下一行。超限即切换「读弃模式」直奔行尾——超限内容不进缓冲。
    pub fn next_line(&mut self) -> LineOutcome {
        self.buf.clear();
        let mut too_long = false;
        loop {
            let window = match self.inner.fill_buf() {
                Ok(w) => w,
                Err(e) => return LineOutcome::Io(e),
            };
            if window.is_empty() {
                // EOF：三种终态——超限行 / 末段无换行内容 / 干净收尾（空缓冲 = EOF）
                return if too_long {
                    LineOutcome::TooLong
                } else if self.buf.is_empty() {
                    LineOutcome::Eof
                } else {
                    take_string(&mut self.buf)
                };
            }
            let take = match window.iter().position(|&b| b == b'\n') {
                Some(nl) => nl + 1,
                None => window.len(),
            };
            let had_newline = window[take - 1] == b'\n';
            if !too_long {
                if self.buf.len() + take > self.limit {
                    too_long = true;
                    self.buf.clear(); // 已缓冲前半段放弃（超限内容不驻留内存）
                } else {
                    self.buf.extend_from_slice(&window[..take]);
                }
            }
            self.inner.consume(take);
            if had_newline {
                return if too_long {
                    LineOutcome::TooLong
                } else {
                    take_string(&mut self.buf)
                };
            }
            // 窗口内无换行 → 继续拉下一窗口（缓冲区已按上限封顶）
        }
    }
}

/// 缓冲区 → 行字符串（剥行尾；非法 UTF-8 与 `lines()` 同语义报
/// `InvalidData`——不做静默替换）。
fn take_string(buf: &mut Vec<u8>) -> LineOutcome {
    match String::from_utf8(std::mem::take(buf)) {
        Ok(mut s) => {
            if s.ends_with('\n') {
                s.pop();
                if s.ends_with('\r') {
                    s.pop();
                }
            }
            LineOutcome::Line(s)
        }
        Err(e) => LineOutcome::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e.utf8_error(),
        )),
    }
}

/// 超限行的 MCP JSON-RPC 错误应答（`bin/server.rs` 用——id 按规范置 null：
/// 无法从被读弃的行中检测合法 id）。
pub fn line_too_long_jsonrpc_reply(limit: usize) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": null,
        "error": {
            "code": INVALID_REQUEST,
            "message": format!(
                "LINE_TOO_LONG: 行超过传输上限 {limit} 字节（含换行），本行已读弃；请缩小单条报文或分批发送"
            ),
        },
    })
    .to_string()
}

/// 超限行的裸 JSON 错误应答（`bin/stdio.rs` 用——与 `dispatch_json` 既有
/// 错误信封同形状：`{"ok":false,"error":{"code":..,"what":..}}`）。
pub fn line_too_long_tool_reply(limit: usize) -> String {
    json!({
        "ok": false,
        "error": {
            "code": "E_UNPROCESSABLE",
            "what": format!(
                "LINE_TOO_LONG: 行超过传输上限 {limit} 字节（含换行），本行已读弃；请缩小单条报文或分批发送"
            ),
        },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    /// 强制小窗口读取器：fill_buf 每次最多吐 chunk 字节——把「窗口内无换行」
    /// 多窗口路径压出来（Cursor 一次全吐，测不到该路径）。
    struct Chunky<const CHUNK: usize>(Cursor<Vec<u8>>);

    impl<const CHUNK: usize> std::io::Read for Chunky<CHUNK> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(out)
        }
    }

    impl<const CHUNK: usize> BufRead for Chunky<CHUNK> {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            let cap = self.0.position() as usize;
            let end = (cap + CHUNK).min(self.0.get_ref().len());
            Ok(&self.0.get_ref()[cap..end])
        }
        fn consume(&mut self, amt: usize) {
            self.0.set_position(self.0.position() + amt as u64);
        }
    }

    fn drain<R: BufRead>(r: &mut BoundedLineReader<R>) -> Vec<LineOutcome> {
        let mut out = Vec::new();
        loop {
            match r.next_line() {
                LineOutcome::Eof => return out,
                other => out.push(other),
            }
        }
    }

    #[test]
    fn normal_lines_preserve_order_and_content() {
        let mut r = BoundedLineReader::with_limit(Cursor::new(b"OK\nsecond line\n\nlast".to_vec()), 1024);
        let got = drain(&mut r);
        assert_eq!(
            got,
            vec![
                LineOutcome::Line("OK".into()),
                LineOutcome::Line("second line".into()),
                LineOutcome::Line("".into()),
                LineOutcome::Line("last".into()), // EOF 末行无换行（lines() 同语义）
            ]
        );
    }

    #[test]
    fn crlf_stripped_like_lines() {
        let mut r = BoundedLineReader::with_limit(Cursor::new(b"a\r\nb\r\n".to_vec()), 1024);
        assert_eq!(r.next_line(), LineOutcome::Line("a".into()));
        assert_eq!(r.next_line(), LineOutcome::Line("b".into()));
        assert_eq!(r.next_line(), LineOutcome::Eof);
    }

    /// 边界钉死：恰好 = 上限（含换行）收；上限 +1 拒。
    #[test]
    fn boundary_exact_limit_passes_limit_plus_one_rejects() {
        // limit=8："1234567\n" 恰 8 字节 → 收
        let mut r = BoundedLineReader::with_limit(Cursor::new(b"1234567\n".to_vec()), 8);
        assert_eq!(r.next_line(), LineOutcome::Line("1234567".into()));
        assert_eq!(r.next_line(), LineOutcome::Eof);
        // "12345678\n" 9 字节 → 拒
        let mut r = BoundedLineReader::with_limit(Cursor::new(b"12345678\n".to_vec()), 8);
        assert_eq!(r.next_line(), LineOutcome::TooLong);
        assert_eq!(r.next_line(), LineOutcome::Eof);
    }

    /// 核心安全语义：超限行读弃后，**后续行不受污染**；多窗口超限同理。
    #[test]
    fn oversized_line_discarded_then_next_line_intact() {
        let mut input = Vec::new();
        input.extend_from_slice(b"OK\n");
        input.extend_from_slice(&vec![b'x'; 20]); // 超 8 上限
        input.extend_from_slice(b"\nAFTER\n");
        let mut r = BoundedLineReader::with_limit(Cursor::new(input), 8);
        assert_eq!(r.next_line(), LineOutcome::Line("OK".into()));
        assert_eq!(r.next_line(), LineOutcome::TooLong);
        assert_eq!(r.next_line(), LineOutcome::Line("AFTER".into()));
        assert_eq!(r.next_line(), LineOutcome::Eof);
    }

    /// 超限且 EOF 无换行（最后一行超限直奔 EOF）。
    #[test]
    fn oversized_without_newline_ends_at_eof() {
        let mut r = BoundedLineReader::with_limit(Cursor::new(vec![b'x'; 20]), 8);
        assert_eq!(r.next_line(), LineOutcome::TooLong);
        assert_eq!(r.next_line(), LineOutcome::Eof);
    }

    /// 多窗口路径：行被切成 4 字节窗口逐段拼装（合法行跨窗口仍完整）。
    #[test]
    fn multi_window_assembles_long_line_across_chunks() {
        let data = b"0123456789ABCDE\nNEXT\n".to_vec(); // 首行恰 16 字节
        let mut r = BoundedLineReader::with_limit(Chunky::<4>(Cursor::new(data)), 16);
        assert_eq!(
            r.next_line(),
            LineOutcome::Line("0123456789ABCDE".into()),
            "跨 4 个 4 字节窗口拼装，恰等上限"
        );
        assert_eq!(r.next_line(), LineOutcome::Line("NEXT".into()));
        assert_eq!(r.next_line(), LineOutcome::Eof);
    }

    /// 多窗口 + 超限：前半段已进缓冲后才发现超限 → 放弃缓冲继续读弃，
    /// 后续行完整。
    #[test]
    fn multi_window_oversized_discards_partial_then_continues() {
        let mut data = Vec::new();
        data.extend_from_slice(&vec![b'y'; 18]); // 超 16 上限，跨 ≥5 个窗口
        data.extend_from_slice(b"\nTAIL\n");
        let mut r = BoundedLineReader::with_limit(Chunky::<4>(Cursor::new(data)), 16);
        assert_eq!(r.next_line(), LineOutcome::TooLong);
        assert_eq!(r.next_line(), LineOutcome::Line("TAIL".into()));
        assert_eq!(r.next_line(), LineOutcome::Eof);
    }

    /// 非法 UTF-8：与 `BufRead::lines` 同语义报 Io(InvalidData)，不静默替换。
    #[test]
    fn invalid_utf8_reports_io_error() {
        let mut r = BoundedLineReader::with_limit(Cursor::new(vec![0xff, b'\n']), 8);
        match r.next_line() {
            LineOutcome::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidData),
            other => panic!("预期 Io(InvalidData)，实际 {other:?}"),
        }
    }

    /// 应答形状钉死：两信封各自与既有协议面同形（防手改漂移）。
    #[test]
    fn too_long_reply_shapes_match_existing_envelopes() {
        let rpc: serde_json::Value =
            serde_json::from_str(&line_too_long_jsonrpc_reply(8)).unwrap();
        assert_eq!(rpc["jsonrpc"], "2.0");
        assert_eq!(rpc["id"], serde_json::Value::Null);
        assert_eq!(rpc["error"]["code"], INVALID_REQUEST);
        assert!(rpc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("LINE_TOO_LONG"));

        let tool: serde_json::Value = serde_json::from_str(&line_too_long_tool_reply(8)).unwrap();
        assert_eq!(tool["ok"], false);
        assert_eq!(tool["error"]["code"], "E_UNPROCESSABLE");
        assert!(tool["error"]["what"]
            .as_str()
            .unwrap()
            .contains("LINE_TOO_LONG"));
    }
}
