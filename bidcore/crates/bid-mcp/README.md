# bid-mcp —— M0 工具解析与分发骨架

> 状态：**骨架**（M0 里程碑）。实现 (`dispatch`) 中已落地的工具直接可调用，
> 未落地的工具返回 **typed `UNIMPLEMENTED_INTERNAL`**，绝不静默成功。

## 为什么先有这一层

`docs/mvp-implementation-plan.md` 的 M0 要求「MCP server 骨架：12 工具，状态校验与
错误包先行，业务可返回 `UNIMPLEMENTED_INTERNAL`」。本 crate 就是那层骨架。

它解决三个实际问题：

1. **工具名单一真源** —— 8+1 个工具名此前只存在于 `docs/bid-mcp-contract.md` 与
   `bid-tools/runner.rs` 的字符串字面量里，没有可被编译器检查的载体；
2. **错误包统一** —— `E_STATE` / `E_NOTFOUND` / `UNIMPLEMENTED_INTERNAL` 走同一条
   `ToolError` 通道，调用方拿到的永远是 typed 错误而非字符串；
3. **传输与业务解耦** —— `Transport` trait 让 stdio JSON-RPC 骨架与未来 HTTP 各自
   独立接入，`dispatch` 只做解析与转发。

## 工具面（9 个）

| 工具 | 状态 |
|---|---|
| `get_bid_project_state` | **已实现**（只读，走 `bid-domain` 状态机） |
| `create_bid_project` | 骨架（`UNIMPLEMENTED_INTERNAL`） |
| `import_tender_files` | 骨架 |
| `extract_requirements` | 骨架 |
| `get_compliance_matrix` | 骨架 |
| `search_evidence` | 骨架 |
| `draft_response_section` | 骨架 |
| `validate_bid_document` | 骨架 |
| `export_bid_document` | 骨架 |

## 运行

```bash
cd bidcore
cargo run -p bid-mcp --bin bid-mcp-stdio
# 逐行喂 JSON：{"tool":"get_bid_project_state","args":{}}
```

## 设计约束（来自 docs/bid-mcp-contract.md）

- 工具只做**参数校验 + 状态机裁决 + 转发**，业务逻辑留在 domain/parse/kb；
- 状态机裁决一律走 `bid-domain` 的 `guard_tool`，本层**不得自行判断状态**；
- 未实现 ≠ 成功：必须返回 `UNIMPLEMENTED_INTERNAL` 并在 note 里写明待接里程碑。
