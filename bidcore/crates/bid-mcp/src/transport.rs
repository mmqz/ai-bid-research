//! 传输面抽象——业务分发与传输实现解耦。
//!
//! M0 只提供 stdio 一种实现（见 `bin/stdio.rs`）。未来 HTTP / wasm 只需实现同一
//! trait，`dispatch` 语义零变化。

use crate::schema::ToolReply;

/// 一条入站请求的抽象（原始文本，由 `dispatch_json` 解析）。
pub trait Transport {
    /// 收下一条请求；EOF 返回 `None`。
    fn recv(&mut self) -> Option<String>;
    /// 回一条应答。
    fn send(&mut self, reply: &ToolReply);
}
