#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""#269 黑盒考验 · MCP 调用留痕器——子代理对引擎的唯一调用面。"""
import json
import os
import subprocess
import sys
import time

BIN = "/tmp/cargo-target-86j6/debug/bid-mcp-stdio"
WS = os.environ.get("BID_WS", "/tmp/269-wenming/ws")
LOG = os.environ.get("BID_CALL_LOG", os.path.join(os.path.dirname(os.path.abspath(__file__)), "mcp-calls.log"))
CALL_TIMEOUT = int(os.environ.get("BID_CALL_SECS", "300"))


def main() -> int:
    if len(sys.argv) < 2:
        print(json.dumps({"ok": False, "error": {"code": "E_USAGE",
              "what": "用法: mcp_call.py <工具名> '<args JSON>'"}}, ensure_ascii=False))
        return 2
    tool = sys.argv[1]
    args_raw = sys.argv[2] if len(sys.argv) > 2 else "{}"
    try:
        args = json.loads(args_raw)
    except json.JSONDecodeError as e:
        print(json.dumps({"ok": False, "error": {"code": "E_ARGS_JSON",
              "what": f"args 不是合法 JSON: {e}"}}, ensure_ascii=False))
        return 2

    env = dict(os.environ)
    env["BID_WS"] = WS
    env.setdefault("BID_FS_ROOT", "/home/z/my-project/ai-bid-research")

    line = json.dumps({"tool": tool, "args": args}, ensure_ascii=False)
    t0 = time.monotonic()
    try:
        r = subprocess.run([BIN], input=line + "\n", capture_output=True,
                           text=True, timeout=CALL_TIMEOUT, env=env)
        elapsed = round((time.monotonic() - t0) * 1000)
        out_lines = [l for l in r.stdout.splitlines() if l.strip()]
        reply = out_lines[0] if out_lines else json.dumps(
            {"ok": False, "error": {"code": "E_NO_REPLY",
             "what": f"无应答（stderr: {r.stderr[:200]}）"}}, ensure_ascii=False)
    except subprocess.TimeoutExpired:
        elapsed = round((time.monotonic() - t0) * 1000)
        reply = json.dumps({"ok": False, "error": {"code": "E_TIMEOUT",
             "what": f"调用超时（>{CALL_TIMEOUT}s）"}}, ensure_ascii=False)

    seq = 1
    if os.path.exists(LOG):
        with open(LOG, "r", encoding="utf-8") as f:
            seq = sum(1 for _ in f) + 1
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(json.dumps({
            "seq": seq,
            "ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "tool": tool,
            "args": args,
            "elapsed_ms": elapsed,
            "reply": json.loads(reply) if reply.strip().startswith("{") else reply,
        }, ensure_ascii=False) + "\n")

    print(reply)
    return 0


if __name__ == "__main__":
    sys.exit(main())
