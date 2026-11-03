# bid-mcp —— MCP 工具解析与分发骨架

> 状态：**骨架**（M0 里程碑）+ **MCP 协议层**（#137）。实现 (`dispatch`) 中已落地的
> 工具直接可调用，未落地的工具返回 **typed `UNIMPLEMENTED_INTERNAL`**，绝不静默成功。

## 为什么先有这一层

`docs/mvp-implementation-plan.md` 的 M0 要求「MCP server 骨架：12 工具，状态校验与
错误包先行，业务可返回 `UNIMPLEMENTED_INTERNAL`」。本 crate 就是那层骨架。

它解决三个实际问题：

1. **工具名单一真源** —— 24 个工具名（`TOOL_NAMES`）可被编译器检查，schema 声明
   与分发都从这里取；
2. **错误包统一** —— `E_STATE` / `E_NOTFOUND` / `E_UNPROCESSABLE` / `UNIMPLEMENTED_INTERNAL`
   走同一条 `ToolError` 通道，调用方拿到的永远是 typed 错误而非字符串；
3. **传输与业务解耦** —— `Transport` trait 与 `protocol::McpSession` 让 stdio /
   JSON-RPC / 未来 HTTP 各自独立接入，`dispatch` 只做解析与转发。

## MCP 协议层（issue #137）

标准 MCP 客户端（官方 Python/TypeScript SDK、Claude Desktop）此前连不上——旧
`bid-mcp-stdio` 只认逐行裸 JSON。#137 补齐 **JSON-RPC 2.0 + 三个必需方法**：

| 方法 | 行为 |
|---|---|
| `initialize` | 版本协商（支持 `2024-11-05` / `2025-03-26` / `2025-06-18` / `2025-11-25`，集内回显、集外回最新）+ `capabilities`（只声明 tools）+ `serverInfo` |
| `tools/list` | 24 个工具全量返回，逐工具 `inputSchema`（合法 JSON Schema，与 tools.rs 实现逐位核对） |
| `tools/call` | 参数形状校验后包装既有 `dispatch`，工具语义零变化 |

错误映射（双通道）：

- **工具执行错误**（ToolError：`E_STATE`/`E_NOTFOUND`/`E_UNPROCESSABLE`/`E_CONFLICT`/
  `E_PARTIAL`/`UNIMPLEMENTED_INTERNAL`）→ result 通道 `isError:true`，`content[0].text`
  携带契约 §0 包装 JSON（`error.code` 字面量透传）；
- **协议层错误** → JSON-RPC error：解析失败 `-32700`、非法请求 `-32600`、未知方法
  `-32601`、参数形状非法/未知工具 `-32602`、未握手先调用 `-32002`。

验收：`tests/mcp_sdk_e2e.rs`（官方 Python `mcp` SDK 拉起 `bid-mcp-server` 子进程跑
initialize + list_tools + call_tool + 错误映射）；协议层传输无关单测在
`src/protocol.rs`（`tests/mcp_protocol` 系，注入独立 `BidService`）。

## 工具面（24 个）

| 工具 | 状态 |
|---|---|
| `get_bid_project_state` | **已实现**（只读，走 `bid-domain` 状态机） |
| `create_bid_project` | **已实现**（B-1） |
| `import_tender_files` | **已实现**（B-1） |
| `extract_requirements` | **已实现**（B5） |
| `get_compliance_matrix` | **已实现**（只读，S-4 游标分页） |
| `list_evidence` / `list_tasks` | **已实现**（只读，S-4 游标分页） |
| `save_version` | **已实现**（#42 阶段 10：AI 存版本快照，mang-snap 单一真源） |
| `draft_response_section` | **已实现**（#59 阶段 1：mock/real/external 生成器） |
| `validate_bid_document` | **已实现**（#59 阶段 3：双层校验） |
| `export_bid_document` | **已实现**（#59 阶段 2：docx/PDF + #117 报价决策面） |
| `check_bid_quality` | **已实现**（#68 阶段 2：只读诊断） |
| `build_outline` / `register_evidence` / `bind_evidence` / `record_review_gate` | **已实现**（#80 第三批 G1/G2/G3） |
| `score_price` / `score_section` / `score_document` | **已实现**（#100 任务 3：B1/B2 评分） |
| `search_evidence` | 骨架（M2-b 落地时必须走 cursor.rs——调用返回 `UNIMPLEMENTED_INTERNAL`） |

## 运行

```bash
cd bidcore

# ---- 标准 MCP 客户端（#137）：JSON-RPC 2.0 + initialize/tools/list/tools/call
cargo run -p bid-mcp --bin bid-mcp-server
# 喂标准 MCP 帧：{"jsonrpc":"2.0","id":1,"method":"initialize","params":{...}}

# ---- 旧逐行裸 JSON（既有 e2e / 桌面壳接线面，一字未动）
cargo run -p bid-mcp --bin bid-mcp-stdio
# 逐行喂 JSON：{"tool":"get_bid_project_state","args":{}}
```

两个 bin 都支持 `BID_WS=<dir>` 持久化工作区（db.json 恢复 + 写面 checkpoint 落盘；
`.mang/llm.json` 真实 LLM 配置面；未设 = 纯内存态）。

## 设计约束（来自 docs/bid-mcp-contract.md）

- 工具只做**参数校验 + 状态机裁决 + 转发**，业务逻辑留在 domain/parse/kb；
- 状态机裁决一律走 `bid-domain` 的 `guard_tool`，本层**不得自行判断状态**；
- 未实现 ≠ 成功：必须返回 `UNIMPLEMENTED_INTERNAL` 并在 note 里写明待接里程碑。
