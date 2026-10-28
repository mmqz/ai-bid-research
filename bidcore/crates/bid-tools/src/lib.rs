//! bid-tools —— MCP 工具薄壳 + 场景运行器（mcp-contract §1~§5 的 M1 子集）。
//! 薄壳纪律：工具只做参数校验、状态机裁决与转发；业务逻辑在 domain/parse/kb。

pub mod runner;
pub mod schema;
pub mod service;
pub mod store_io;

pub use runner::{run_scenario, write_run, RunOutput};
pub use service::{import_files, BidService, ImportReport, ImportSpec};
pub use store_io::{StoreFile, StoreIoError};
