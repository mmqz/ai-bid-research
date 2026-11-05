#!/usr/bin/env python3
"""#137 MCP 协议层验收：官方 Python mcp SDK 端到端。

用官方 SDK 的 stdio 客户端拉起 `bid-mcp-server` 子进程，跑完整握手 +
工具三件套（issue #137 验收第一条）：

  1. initialize        —— 版本协商 + serverInfo + capabilities
  2. list_tools        —— 26 个工具全量在座，每个带合法 inputSchema（#219 三覆盖工具 + #226 set_matrix_override + #243 recover_validating_state）
  3. call_tool         —— 成功路径（create_bid_project）+
                          工具执行错误（isError=true + 契约错误码）+
                          协议错误（未知工具 → McpError -32602）

运行方式：由 Rust 驱动 tests/mcp_sdk_e2e.rs 调起（CARGO_BIN_EXE_
bid-mcp-server 定位二进制）；也可手动 `python3 tests/mcp_sdk_e2e.py
/path/to/bid-mcp-server`。退出码 0 = 全部通过。

环境：BID_WS 未设 → 子进程纯内存态（测试自带清理，不落盘）。
"""

from __future__ import annotations

import asyncio
import json
import os
import sys

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

SUPPORTED = {"2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"}
# #219 全块覆盖三工具后工具面 24（与 bid-mcp/src/lib.rs TOOL_NAMES 逐一对齐）
EXPECTED_TOOLS = {
    "create_bid_project", "import_tender_files", "extract_requirements",
    "get_compliance_matrix", "list_evidence", "list_tasks", "search_evidence",
    "draft_response_section", "validate_bid_document", "export_bid_document",
    "get_bid_project_state", "save_version", "check_bid_quality",
    "build_outline", "register_evidence", "bind_evidence", "record_review_gate",
    "record_addendum_waiver",
    "score_price", "score_section", "score_document",
    # #219 全块覆盖三工具（#237 已入 main；#227 曾漏登 e2e 清单——
    # CI 无 Python SDK 抓不到，本分支合流时与 TOOL_NAMES 26 对齐）
    "get_document_chunks", "submit_block_marks", "get_coverage_report",
    "set_matrix_override",  # #226 层 1：否决项死锁的人工出口
    "recover_validating_state",  # #243：历史异常态（VALIDATING+缺章）恢复出口
}

server_bin = sys.argv[1] if len(sys.argv) > 1 else "bid-mcp-server"


def check(cond: bool, msg: str) -> None:
    if not cond:
        print(f"FAIL: {msg}", file=sys.stderr)
        sys.exit(1)
    print(f"  ok: {msg}")


async def main() -> None:
    params = StdioServerParameters(
        command=server_bin,
        args=[],
        env={**os.environ, "BID_WS": ""},  # 纯内存态：不落盘、不污染宿主工作区
    )
    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            # ---- ① initialize（版本协商 + 身份 + 能力面）----
            init = await session.initialize()
            check(init.protocolVersion in SUPPORTED,
                  f"initialize: protocolVersion={init.protocolVersion} 在支持集内")
            check(init.serverInfo.name == "bid-mcp",
                  f"initialize: serverInfo.name={init.serverInfo.name}")
            check(init.serverInfo.version is not None and init.serverInfo.version != "",
                  "initialize: serverInfo.version 非空")
            check(init.capabilities.tools is not None,
                  "initialize: capabilities.tools 已声明")

            # ---- ② list_tools（全量工具面 + inputSchema 形态）----
            tools = (await session.list_tools()).tools
            names = {t.name for t in tools}
            check(len(tools) == 26, f"list_tools: 工具数 = {len(tools)}（26）")
            check(names == EXPECTED_TOOLS, "list_tools: 与 TOOL_NAMES 单一真源一致")
            for t in tools:
                check(isinstance(t.inputSchema, dict) and t.inputSchema.get("type") == "object",
                      f"list_tools: {t.name}.inputSchema 是合法 JSON Schema（type=object）")
                if t.name == "search_evidence":
                    # #145 任务 1 已接线：断言语义随接线状态翻转——
                    # 「描述诚实」的本意不变：描述不得与实装状态相悖。
                    desc = t.description or ""
                    check("未接线" not in desc and "UNIMPLEMENTED" not in desc
                          and "朴素实现" in desc,
                          "list_tools: search_evidence 描述与接线状态一致（#145 朴素实现）")

            # ---- ③ call_tool 成功路径 ----
            created = await session.call_tool("create_bid_project", {
                "name": "SDK e2e 验收项目",
                "bid_deadline": "2099-12-31",
                "mode": "明标",
                "external_key": "mcp-sdk-e2e-137",
            })
            check(created.isError is False, "call_tool: create_bid_project 成功")
            pid = (created.structuredContent or {}).get("project_id")
            check(isinstance(pid, str) and pid.startswith("PRJ-"),
                  f"call_tool: structuredContent.project_id = {pid}")
            text = created.content[0].text if created.content else ""
            wire = json.loads(text)
            check(wire.get("ok") is True and wire.get("tool") == "create_bid_project",
                  "call_tool: content[0].text 是契约 §0 包装 JSON")
            check(wire.get("data", {}).get("project_id") == pid,
                  "call_tool: content 与 structuredContent 同一真源")

            # 同 external_key 幂等重放
            replay = await session.call_tool("create_bid_project", {
                "name": "SDK e2e 验收项目", "bid_deadline": "2099-12-31",
                "mode": "明标", "external_key": "mcp-sdk-e2e-137",
            })
            check(replay.isError is False
                  and (replay.structuredContent or {}).get("deduplicated") is True,
                  "call_tool: 同 external_key 幂等重放（deduplicated=true）")

            # 状态查询（断点恢复入口）
            state = await session.call_tool("get_bid_project_state", {"project_id": pid})
            check(state.isError is False
                  and (state.structuredContent or {}).get("status") == "DRAFT",
                  "call_tool: get_bid_project_state → DRAFT")

            # ---- ④ 工具执行错误 → isError:true + 契约错误码 ----
            err = await session.call_tool("extract_requirements", {"project_id": pid})
            check(err.isError is True, "call_tool: 未导入先抽取 → isError=true")
            code = (err.structuredContent or {}).get("code")
            check(code == "E_STATE",
                  f"call_tool: 契约错误码透传（E_STATE，实际 {code}）")

            bad = await session.call_tool("create_bid_project", {"mode": "明标"})
            check(bad.isError is True
                  and (bad.structuredContent or {}).get("code") == "E_UNPROCESSABLE",
                  "call_tool: 参数缺失 → isError=true + E_UNPROCESSABLE")

            # ---- ⑤ 协议错误：未知工具 → JSON-RPC -32602（SDK 抛 McpError）----
            from mcp.shared.exceptions import McpError
            try:
                await session.call_tool("no_such_tool", {})
                check(False, "未知工具必须抛 McpError（-32602）")
            except McpError as e:
                check("-32602" in str(e.error.code) or int(e.error.code) == -32602,
                      f"未知工具 → JSON-RPC error -32602（实际 {e.error.code}）")

    print("SDK e2e 全部通过：initialize + list_tools + call_tool + 错误映射")


if __name__ == "__main__":
    asyncio.run(main())
