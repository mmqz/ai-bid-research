#!/usr/bin/env python3
"""#137 MCP 鍗忚灞傞獙鏀讹細瀹樻柟 Python mcp SDK 绔埌绔€?

鐢ㄥ畼鏂?SDK 鐨?stdio 瀹㈡埛绔媺璧?`bid-mcp-server` 瀛愯繘绋嬶紝璺戝畬鏁存彙鎵?+
宸ュ叿涓変欢濂楋紙issue #137 楠屾敹绗竴鏉★級锛?

  1. initialize        鈥斺€?鐗堟湰鍗忓晢 + serverInfo + capabilities
  2. list_tools        鈥斺€?34 涓伐鍏峰叏閲忓湪搴э紝姣忎釜甯﹀悎娉?inputSchema锛?219 涓夎鐩栧伐鍏?+ #226 set_matrix_override + #243 recover_validating_state + #254 鍒嗘涓夊伐鍏?+ #279 list_tools 鍙戠幇闈級
  3. call_tool         鈥斺€?鎴愬姛璺緞锛坈reate_bid_project锛?
                          宸ュ叿鎵ц閿欒锛坕sError=true + 濂戠害閿欒鐮侊級+
                          鍗忚閿欒锛堟湭鐭ュ伐鍏?鈫?McpError -32602锛?

杩愯鏂瑰紡锛氱敱 Rust 椹卞姩 tests/mcp_sdk_e2e.rs 璋冭捣锛圕ARGO_BIN_EXE_
bid-mcp-server 瀹氫綅浜岃繘鍒讹級锛涗篃鍙墜鍔?`python3 tests/mcp_sdk_e2e.py
/path/to/bid-mcp-server`銆傞€€鍑虹爜 0 = 鍏ㄩ儴閫氳繃銆?

鐜锛欱ID_WS 鏈 鈫?瀛愯繘绋嬬函鍐呭瓨鎬侊紙娴嬭瘯鑷甫娓呯悊锛屼笉钀界洏锛夈€?
"""

from __future__ import annotations

import asyncio
import json
import os
import sys

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

SUPPORTED = {"2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"}
# #219 鍏ㄥ潡瑕嗙洊涓夊伐鍏峰悗宸ュ叿闈?24锛堜笌 bid-mcp/src/lib.rs TOOL_NAMES 閫愪竴瀵归綈锛?
EXPECTED_TOOLS = {
    "create_bid_project", "import_tender_files", "extract_requirements",
    "get_compliance_matrix", "list_evidence", "list_tasks", "search_evidence",
    "draft_response_section", "validate_bid_document", "export_bid_document",
    "get_bid_project_state", "save_version", "check_bid_quality",
    "build_outline", "register_evidence", "bind_evidence", "record_review_gate",
    "record_addendum_waiver",
    "score_price", "score_section", "score_document",
    # #219 鍏ㄥ潡瑕嗙洊涓夊伐鍏凤紙#237 宸插叆 main锛?227 鏇炬紡鐧?e2e 娓呭崟鈥斺€?
    # CI 鏃?Python SDK 鎶撲笉鍒帮紝鏈垎鏀悎娴佹椂涓?TOOL_NAMES 26 瀵归綈锛?
    "get_document_chunks", "submit_block_marks", "get_coverage_report",
    "get_star_coverage",  # #285 鈶?鈽呮紡鏍囨娴嬶紙涓?TOOL_NAMES 30 瀵归綈锛?
    "set_matrix_override",  # #226 灞?1锛氬惁鍐抽」姝婚攣鐨勪汉宸ュ嚭鍙?
    "recover_validating_state",  # #243锛氬巻鍙插紓甯告€侊紙VALIDATING+缂虹珷锛夋仮澶嶅嚭鍙?
    # #254 绔犺妭绾у閲忓啓鍏ヤ笁宸ュ叿 + #279 list_tools锛堜笌 TOOL_NAMES 30 瀵归綈锛?
    "append_section_segment",
    "rewrite_section_segment",
    "list_section_segments",
    # #279 琛屽崗璁彂鐜伴潰锛圝SON-RPC tools/list 鍚屾牱鍙锛?
    "list_tools",
    "recommend_length_profile",  # #287 绡囧箙妗ｄ綅鎺ㄨ崘
    "get_requirement_provenance",  # #285鈶?鍧楃骇鍙屽悜绱㈠紩涓夊伐鍏凤紙涓?TOOL_NAMES 34 瀵归綈锛?
    "get_block_requirements",
    "get_annotation_overlay",
    # #319 琛ラ仐鍚庡閲忛噸鍐欎袱宸ュ叿锛堜笌 TOOL_NAMES 37 瀵归綈锛?
    "register_addendum_changes",
    "rewrite_section_partial",
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
        env={**os.environ, "BID_WS": ""},  # 绾唴瀛樻€侊細涓嶈惤鐩樸€佷笉姹℃煋瀹夸富宸ヤ綔鍖?
    )
    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            # ---- 鈶?initialize锛堢増鏈崗鍟?+ 韬唤 + 鑳藉姏闈級----
            init = await session.initialize()
            check(init.protocolVersion in SUPPORTED,
                  f"initialize: protocolVersion={init.protocolVersion} 鍦ㄦ敮鎸侀泦鍐?)
            check(init.serverInfo.name == "bid-mcp",
                  f"initialize: serverInfo.name={init.serverInfo.name}")
            check(init.serverInfo.version is not None and init.serverInfo.version != "",
                  "initialize: serverInfo.version 闈炵┖")
            check(init.capabilities.tools is not None,
                  "initialize: capabilities.tools 宸插０鏄?)

            # ---- 鈶?list_tools锛堝叏閲忓伐鍏烽潰 + inputSchema 褰㈡€侊級----
            tools = (await session.list_tools()).tools
            names = {t.name for t in tools}
 $args[0].Groups[1].Value.TrimEnd() 
            check(names == EXPECTED_TOOLS, "list_tools: 涓?TOOL_NAMES 鍗曚竴鐪熸簮涓€鑷?)
            for t in tools:
                check(isinstance(t.inputSchema, dict) and t.inputSchema.get("type") == "object",
                      f"list_tools: {t.name}.inputSchema 鏄悎娉?JSON Schema锛坱ype=object锛?)
                if t.name == "search_evidence":
                    # #145 浠诲姟 1 宸叉帴绾匡細鏂█璇箟闅忔帴绾跨姸鎬佺炕杞€斺€?
                    # 銆屾弿杩拌瘹瀹炪€嶇殑鏈剰涓嶅彉锛氭弿杩颁笉寰椾笌瀹炶鐘舵€佺浉鎮栥€?
                    desc = t.description or ""
                    check("鏈帴绾? not in desc and "UNIMPLEMENTED" not in desc
                          and "鏈寸礌瀹炵幇" in desc,
                          "list_tools: search_evidence 鎻忚堪涓庢帴绾跨姸鎬佷竴鑷达紙#145 鏈寸礌瀹炵幇锛?)

            # ---- 鈶?call_tool 鎴愬姛璺緞 ----
            created = await session.call_tool("create_bid_project", {
                "name": "SDK e2e 楠屾敹椤圭洰",
                "bid_deadline": "2099-12-31",
                "mode": "鏄庢爣",
                "external_key": "mcp-sdk-e2e-137",
            })
            check(created.isError is False, "call_tool: create_bid_project 鎴愬姛")
            pid = (created.structuredContent or {}).get("project_id")
            check(isinstance(pid, str) and pid.startswith("PRJ-"),
                  f"call_tool: structuredContent.project_id = {pid}")
            text = created.content[0].text if created.content else ""
            wire = json.loads(text)
            check(wire.get("ok") is True and wire.get("tool") == "create_bid_project",
                  "call_tool: content[0].text 鏄绾?搂0 鍖呰 JSON")
            check(wire.get("data", {}).get("project_id") == pid,
                  "call_tool: content 涓?structuredContent 鍚屼竴鐪熸簮")

            # 鍚?external_key 骞傜瓑閲嶆斁
            replay = await session.call_tool("create_bid_project", {
                "name": "SDK e2e 楠屾敹椤圭洰", "bid_deadline": "2099-12-31",
                "mode": "鏄庢爣", "external_key": "mcp-sdk-e2e-137",
            })
            check(replay.isError is False
                  and (replay.structuredContent or {}).get("deduplicated") is True,
                  "call_tool: 鍚?external_key 骞傜瓑閲嶆斁锛坉eduplicated=true锛?)

            # 鐘舵€佹煡璇紙鏂偣鎭㈠鍏ュ彛锛?
            state = await session.call_tool("get_bid_project_state", {"project_id": pid})
            check(state.isError is False
                  and (state.structuredContent or {}).get("status") == "DRAFT",
                  "call_tool: get_bid_project_state 鈫?DRAFT")

            # ---- 鈶?宸ュ叿鎵ц閿欒 鈫?isError:true + 濂戠害閿欒鐮?----
            err = await session.call_tool("extract_requirements", {"project_id": pid})
            check(err.isError is True, "call_tool: 鏈鍏ュ厛鎶藉彇 鈫?isError=true")
            code = (err.structuredContent or {}).get("code")
            check(code == "E_STATE",
                  f"call_tool: 濂戠害閿欒鐮侀€忎紶锛圗_STATE锛屽疄闄?{code}锛?)

            bad = await session.call_tool("create_bid_project", {"mode": "鏄庢爣"})
            check(bad.isError is True
                  and (bad.structuredContent or {}).get("code") == "E_UNPROCESSABLE",
                  "call_tool: 鍙傛暟缂哄け 鈫?isError=true + E_UNPROCESSABLE")

            # ---- 鈶?鍗忚閿欒锛氭湭鐭ュ伐鍏?鈫?JSON-RPC -32602锛圫DK 鎶?McpError锛?---
            from mcp.shared.exceptions import McpError
            try:
                await session.call_tool("no_such_tool", {})
                check(False, "鏈煡宸ュ叿蹇呴』鎶?McpError锛?32602锛?)
            except McpError as e:
                check("-32602" in str(e.error.code) or int(e.error.code) == -32602,
                      f"鏈煡宸ュ叿 鈫?JSON-RPC error -32602锛堝疄闄?{e.error.code}锛?)

    print("SDK e2e 鍏ㄩ儴閫氳繃锛歩nitialize + list_tools + call_tool + 閿欒鏄犲皠")


if __name__ == "__main__":
    asyncio.run(main())
