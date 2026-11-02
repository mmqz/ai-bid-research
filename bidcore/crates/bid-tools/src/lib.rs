//! bid-tools —— MCP 工具薄壳 + 场景运行器（mcp-contract §1~§5 的 M1 子集）。
//! 薄壳纪律：工具只做参数校验、状态机裁决与转发；业务逻辑在 domain/parse/kb。

pub mod assembly;
pub mod check;
pub mod context;
pub mod draft;
pub mod export;
pub mod format_spec;
// #183+#189：MCP 文件访问围栏（导入 upload_ref / 导出 output_path 共用单一实现）
pub mod fs_fence;
pub mod llm;
pub mod placeholder;
pub mod quality;
pub mod readiness;
pub mod runner;
pub mod schema;
pub mod service;
pub mod store_io;
// #103 子任务 B：六种标书表格（struct → markdown → 现有导出链 → docx 真表格）
pub mod tables;
pub mod validate;

pub use runner::{run_scenario, write_run, RunOutput};
pub use service::{import_files, BidService, ImportReport, ImportSpec};
pub use store_io::{StoreFile, StoreIoError};
