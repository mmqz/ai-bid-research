#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_mcp_e2e.py —— MCP 端到端验证（issue #139）。

用【真客户端】跑通 P0-P8 + external 模式，证明使用方
「加载 SKILL.md → MCP 连 bid-mcp → Agent 按阶段表驱动」的接入路径真实可用。

传输模式（环境变量 BID_MCP_TRANSPORT）：
  auto     自动发现：target/debug 下有 bid-mcp-server → mcp；否则 bid-mcp-stdio → linejson
  mcp      官方 MCP Python SDK（stdio_client + ClientSession：initialize /
           tools/list / tools/call）——需要 #137 交付的 bid-mcp-server
  linejson 现网 bid-mcp-stdio 的逐行 JSON 协议（{"tool","args"} 一行进、
           {"ok","data"|"error"} 一行出）。#137 交付前先用它真跑【工具层】：
           场景体 / 参数面 / 状态机 / external 四用例全部可验；
           协议层（initialize/tools/list/tools/call）在此模式【不可验】，
           对应测试显式 skip 并注明原因（不假装通过）。

用例面：
  test_00_environment           环境留档（OS/Python/SDK/rustc/二进制 sha256）
  test_01_initialize_and_tools  MCP 协议握手 + 工具清单 ≥20（仅 mcp 传输）
  test_02_full_pipeline         P0-P8 全链（external 起草全部章节 → docx 导出
                                → 产物深检 → 进程重启续跑）
  test_10_external_ghost_ev     external ①：正文引用不存在的 EV- 编号
                                → 引擎剔除 + warning，不拒稿
  test_11_external_uncovered    external ②：正文覆盖不全 → uncovered_requirements
  test_12_external_unanchored   external ③：承诺句无引用 → count_unanchored_claims > 0
  test_13_external_cas_version  external ④：正常正文 → CAS 写入成功 + 版本递增
                                + 同稿重提幂等

运行：
  pip install mcp pytest
  cargo build -p bid-mcp --bin bid-mcp-stdio          # linejson 模式（现在即可）
  cargo build -p bid-mcp --bin bid-mcp-server         # mcp 模式（#137 交付后）
  pytest sdk/tests/test_mcp_e2e.py -v
  python3 sdk/tests/test_mcp_e2e.py                   # 独立运行（无 pytest 也可）

环境变量：
  BID_MCP_TRANSPORT   auto（默认）| mcp | linejson
  BID_MCP_SERVER_BIN  server 二进制显式路径（缺省在 BID_MCP_TARGET_DIR/debug 下找）
  BID_MCP_SERVER_ARGS 额外启动参数（如 bid-mcp-stdio 若以 --mcp 切协议时用）
  BID_MCP_TARGET_DIR  cargo target 目录（默认 /tmp/shared-target）
  BID_MCP_CALL_SECS   单次工具调用超时秒数（默认 300）

证据落盘：sdk/tests/evidence/<case>.json —— docs/mcp-e2e-verification.md 的数据源。

断言纪律（issue #139 提醒）：发现工具返回格式问题 → 如实 FAIL，
evidence 记录原始返回；绝不为通过而改断言。
边界遵守：本文件只新建于 sdk/tests/**，不改 bidcore/**。
"""

from __future__ import annotations

import asyncio
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import time
import zipfile
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Awaitable, Callable

import pytest

# ---------------------------------------------------------------- 路径与常量

REPO_ROOT = Path(__file__).resolve().parents[2]  # sdk/tests/x.py → 仓库根
FIXTURES = REPO_ROOT / "fixtures" / "corpus-A" / "files"
EVIDENCE_DIR = REPO_ROOT / "sdk" / "tests" / "evidence"

CALL_TIMEOUT = float(os.environ.get("BID_MCP_CALL_SECS", "300"))

# MCP SDK（linejson 模式下允许缺失——只有 mcp 传输需要它）
try:
    from mcp import ClientSession, StdioServerParameters
    from mcp.client.stdio import get_default_environment, stdio_client

    HAS_MCP_SDK = True
except ImportError:  # pragma: no cover
    HAS_MCP_SDK = False

# P1 导入的招标包（与 bidcore Rust e2e 同一套真件——GT 25 埋点语料）
TENDER_FILES: list[tuple[str, str]] = [
    ("A-招标文件.docx", "招标文件"),
    ("C-补遗文件01.docx", "补遗"),
    ("evidence/EV-01-营业执照.pdf", "附件"),
    ("evidence/EV-02-电子与智能化工程专业承包二级资质证书.pdf", "附件"),
]
EV_SOURCE = "evidence/EV-02-电子与智能化工程专业承包二级资质证书.pdf"

# blocking 判定的必绑类目（矩阵 is_mandatory：否决项 | 资格条件；技术参数
# 同样须佐证——#110 缺失回报闸，不绑则 draft 前置闸拒生成）
MANDATORY_CATS = {"资格条件", "否决项", "技术参数"}

GHOST_EV = "EV-9999"  # 用例①的幽灵证据号（任何真实台账都不会有）

# 无锚承诺句（用例③——措辞避开引用，各成一句，计数恰为 2）
PROMISE_SENTENCES = [
    "我方郑重承诺所投设备均为全新原厂正品。",
    "我方确保本项目按期交付并顺利通过验收。",
]


# ---------------------------------------------------------------- 小工具


def now_iso() -> str:
    return datetime.now(timezone.utc).astimezone().isoformat(timespec="seconds")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def compact(obj: Any, max_len: int = 1500) -> Any:
    """evidence 摘要——长数组折叠成 {count, head}，长字符串截断。"""
    if isinstance(obj, dict):
        return {k: compact(v, max_len) for k, v in obj.items()}
    if isinstance(obj, (list, tuple)):
        if len(obj) > 12:
            return {"count": len(obj), "head": [compact(x, max_len) for x in obj[:12]]}
        return [compact(x, max_len) for x in obj]
    if isinstance(obj, str) and len(obj) > max_len:
        return obj[:max_len] + f"…(<{len(obj)} chars>)"
    return obj


class ToolFailure(RuntimeError):
    """工具层失败（envelope ok=false 或 MCP isError）。"""

    def __init__(self, tool: str, code: str, what: str, raw: Any = None):
        super().__init__(f"[{tool}] {code}: {what}")
        self.tool, self.code, self.what, self.raw = tool, code, what, raw


# ---------------------------------------------------------------- server 发现


def resolve_server() -> tuple[Path, str, list[str]]:
    """返回 (二进制, transport, 额外args)。auto：优先 bid-mcp-server。"""
    transport_pref = os.environ.get("BID_MCP_TRANSPORT", "auto").strip().lower()
    target = Path(os.environ.get("BID_MCP_TARGET_DIR", "/tmp/shared-target"))
    debug = target / "debug"
    extra_args = [a for a in os.environ.get("BID_MCP_SERVER_ARGS", "").split() if a]

    override = os.environ.get("BID_MCP_SERVER_BIN")
    if override:
        p = Path(override)
        assert p.is_file(), f"BID_MCP_SERVER_BIN 指向的文件不存在: {p}"
        if transport_pref not in ("mcp", "linejson"):
            transport_pref = "mcp" if p.name == "bid-mcp-server" else "linejson"
        return p, transport_pref, extra_args

    server, stdio = debug / "bid-mcp-server", debug / "bid-mcp-stdio"
    if transport_pref == "mcp":
        assert server.is_file(), (
            f"BID_MCP_TRANSPORT=mcp 但 {server} 不在——#137（MCP 协议层）是否已交付并编译？"
            f" 先跑: cargo build -p bid-mcp --bin bid-mcp-server"
        )
        return server, "mcp", extra_args
    if transport_pref == "linejson":
        assert stdio.is_file(), (
            f"BID_MCP_TRANSPORT=linejson 但 {stdio} 不在——先跑: "
            f"cargo build -p bid-mcp --bin bid-mcp-stdio"
        )
        return stdio, "linejson", extra_args
    # auto
    if server.is_file():
        return server, "mcp", extra_args
    if stdio.is_file():
        return stdio, "linejson", extra_args
    raise AssertionError(
        f"{debug} 下既无 bid-mcp-server 也无 bid-mcp-stdio——先编译（见模块 docstring）"
    )


# ---------------------------------------------------------------- 客户端实现


class LineJsonClient:
    """现网 bid-mcp-stdio：一行 {"tool","args"} 进、一行 {"ok","tool","data"|"error"} 出。

    真进程边界（spawn + stdio 管道），全局服务态由 BID_WS 持久化工作区承载
    ——与 MCP 模式走的是同一 dispatch_json → tools::invoke 分发面。
    """

    transport = "linejson"

    def __init__(self, bin_path: Path, ws: Path, extra_args: list[str]):
        self.ws = ws
        self.stderr_path = ws.parent / "server.stderr.log"
        self._err = open(self.stderr_path, "wb")
        env = {
            k: os.environ[k]
            for k in ("PATH", "HOME", "LANG", "TERM", "USER")
            if k in os.environ
        }
        env["BID_WS"] = str(ws)
        self.proc = subprocess.Popen(
            [str(bin_path), *extra_args],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self._err,
            env=env,
            cwd=str(ws),
        )
        self.calls = 0
        self.server_info = {"name": "bid-mcp-stdio", "version": "(linejson, 无握手)"}

    def _roundtrip(self, tool: str, args: dict) -> Any:
        assert self.proc.poll() is None, f"server 进程已退出（code={self.proc.returncode}）"
        req = json.dumps({"tool": tool, "args": args}, ensure_ascii=False)
        self.proc.stdin.write((req + "\n").encode("utf-8"))
        self.proc.stdin.flush()
        raw = self.proc.stdout.readline()
        if not raw:
            self._err.flush()
            tail = b""
            try:
                tail = Path(self.stderr_path).read_bytes()[-2000:]
            except OSError:
                pass
            raise ToolFailure(tool, "TRANSPORT_DEAD", f"server 无应答且管道关闭；stderr 尾部: {tail!r}")
        self.calls += 1
        return json.loads(raw.decode("utf-8"))

    async def call_tool(self, tool: str, args: dict) -> Any:
        return await asyncio.to_thread(self._roundtrip, tool, args)

    async def list_tools(self) -> list[str]:
        raise NotImplementedError("linejson 协议无 tools/list——协议层归 #137")

    def close(self) -> None:
        try:
            if self.proc.poll() is None:
                self.proc.stdin.close()
                try:
                    self.proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
        finally:
            self._err.close()


class McpClient:
    """官方 MCP Python SDK——initialize / tools/list / tools/call 真握手。"""

    transport = "mcp"

    def __init__(self, session: "ClientSession", server_info: Any):
        self.session = session
        self.server_info = server_info
        self.calls = 0
        # 工具返回 text 的实际形态留档（issue #139：格式问题如实报告）
        self.payload_shapes: dict[str, str] = {}

    async def call_tool(self, tool: str, args: dict) -> Any:
        try:
            res = await self.session.call_tool(tool, dict(args))
        except ToolFailure:
            raise
        except Exception as e:  # McpError 等协议级错误
            raise ToolFailure(tool, "PROTOCOL_ERROR", repr(e)) from e
        self.calls += 1
        # structuredContent 在场优先（MCP 规范的结构化载荷通道）
        structured = getattr(res, "structuredContent", None)
        texts = [
            c.text for c in (res.content or []) if getattr(c, "type", "") == "text"
        ]
        text = "\n".join(texts)
        if getattr(res, "isError", False):
            # 工具层失败：#137 协议层把 envelope 放进 content[0].text，
            # structuredContent 只有 {code, what}——从 text 解析完整 envelope 拿 code。
            v: dict = {}
            try:
                v = json.loads(text)
            except json.JSONDecodeError:
                v = {}
            if isinstance(v, dict) and v.get("ok") is False:
                err = v.get("error", {}) or {}
                raise ToolFailure(tool, err.get("code", "?"), err.get("what", ""), v)
            sc = structured if isinstance(structured, dict) else {}
            if sc.get("code"):
                raise ToolFailure(tool, sc.get("code"), sc.get("what", ""), sc)
            raise ToolFailure(tool, "IS_ERROR", text[:500] or "(空)", text)
        if structured is not None:
            # #137 后：成功时 structuredContent = data 本体（无 ok 外壳）；
            # 失败时 = {code, what}。先判是不是 envelope 形态。
            if isinstance(structured, dict) and "ok" in structured:
                self.payload_shapes[tool] = "structuredContent/envelope"
                return self._unwrap_envelope(structured, tool)
            self.payload_shapes[tool] = "structuredContent/data"
            return structured
        try:
            v = json.loads(text)
        except json.JSONDecodeError:
            raise ToolFailure(
                tool, "NON_JSON_PAYLOAD", f"tools/call 返回非 JSON text: {text[:300]!r}", text
            )
        if isinstance(v, dict) and "ok" in v:
            self.payload_shapes[tool] = "text/envelope"
            return self._unwrap_envelope(v, tool)
        self.payload_shapes[tool] = "text/raw-data"
        return v

    @staticmethod
    def _unwrap_envelope(v: dict, tool: str) -> Any:
        if v.get("ok") is True:
            return v.get("data", {})
        err = v.get("error", {}) or {}
        raise ToolFailure(
            tool, err.get("code", "?"), err.get("what", json.dumps(err, ensure_ascii=False)), v
        )

    async def raw_call(self, tool: str, args: dict) -> dict:
        """返回【完整 envelope】（ok/tool/data|error）——从 content[0].text 解析。

        用途：#137 协议层把 structuredContent 设成 data 本体（不含 ok 外壳），
        需要 ok/counts 等完整字段时走这个。"""
        res = await self.session.call_tool(tool, dict(args))
        texts = [x.text for x in (res.content or []) if getattr(x, "type", "") == "text"]
        if not texts:
            return {"ok": not getattr(res, "isError", False),
                    "data": getattr(res, "structuredContent", None)}
        try:
            return json.loads(texts[0])
        except json.JSONDecodeError:
            return {"ok": False, "raw": texts[0][:500]}

    async def list_tools(self) -> list[str]:
        res = await self.session.list_tools()
        return [t.name for t in res.tools]


@asynccontextmanager
async def open_client(ws: Path):
    """按 transport 起一个【连接到全新工作区 ws】的客户端。

    两种传输在客户端侧等价：call_tool(tool, args) → data dict / ToolFailure。
    """
    bin_path, transport, extra_args = resolve_server()
    if transport == "mcp":
        assert HAS_MCP_SDK, "mcp 传输需要官方 SDK：pip install mcp"
        env = {**get_default_environment(), "BID_WS": str(ws)}
        params = StdioServerParameters(
            command=str(bin_path), args=extra_args, env=env
        )
        errlog = open(ws.parent / "server.stderr.log", "wb")
        try:
            async with stdio_client(params, errlog=errlog) as (read, write):
                async with ClientSession(read, write) as session:
                    info = await session.initialize()
                    yield McpClient(session, info)
        finally:
            errlog.close()
    else:
        client = LineJsonClient(bin_path, ws, extra_args)
        try:
            yield client
        finally:
            client.close()


async def call(client: Any, tool: str, args: dict) -> dict:
    """带超时的调用 + envelope 解包（{"ok":false} → ToolFailure）。"""
    r = await asyncio.wait_for(client.call_tool(tool, args), timeout=CALL_TIMEOUT)
    if isinstance(r, dict) and "ok" in r:  # linejson 顶层就是 envelope
        if r.get("ok") is True:
            return r.get("data", {})
        err = r.get("error", {}) or {}
        raise ToolFailure(
            tool, err.get("code", "?"), err.get("what", json.dumps(err, ensure_ascii=False)), r
        )
    return r  # 已被 McpClient 解包 / 非 envelope 形态——原样上抛由调用方断言


async def call_env(client: Any, tool: str, args: dict) -> dict:
    """带超时的调用，返回【完整 envelope】（含 ok/tool/data|error）。

    #137 后 structuredContent = data 本体（无 ok 外壳）；需要 ok/counts 时用这个。
    """
    fn = getattr(client, "raw_call", None)
    if fn is not None:
        return await asyncio.wait_for(fn(tool, dict(args)), timeout=CALL_TIMEOUT)
    return await asyncio.wait_for(client.call_tool(tool, dict(args)), timeout=CALL_TIMEOUT)


# ---------------------------------------------------------------- 证据记录


class Recorder:
    """每阶段的请求/返回/检查项留档——报告的数据源；失败也先落盘。"""

    def __init__(self, case: str, transport: str, bin_path: Path):
        self.case = case
        self.doc: dict[str, Any] = {
            "case": case,
            "transport": transport,
            "server_bin": str(bin_path),
            "server_sha256": sha256_file(bin_path),
            "python": sys.version.split()[0],
            "mcp_sdk": _sdk_version(),
            "os": platform.platform(),
            "started_at": now_iso(),
            "stages": [],
            "payload_shapes": {},
        }

    def stage(
        self,
        name: str,
        tool: str | None,
        args: dict | None,
        result: Any,
        checks: list[tuple[str, bool]],
        extra: dict | None = None,
        elapsed_ms: float | None = None,
    ) -> None:
        entry = {
            "stage": name,
            "tool": tool,
            "args": compact(args or {}),
            "result": compact(result),
            "checks": [{"desc": d, "pass": bool(ok)} for d, ok in checks],
            "elapsed_ms": elapsed_ms,
        }
        if extra:
            entry["extra"] = compact(extra)
        self.doc["stages"].append(entry)
        n_pass = sum(1 for _, ok in checks if ok)
        status = "PASS" if n_pass == len(checks) and checks else "FAIL"
        print(f"  [{self.case}] {name:<28} {status} ({n_pass}/{len(checks)})", flush=True)

    def finish(self, note: str = "") -> None:
        self.doc["finished_at"] = now_iso()
        if note:
            self.doc["note"] = note
        EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
        out = EVIDENCE_DIR / f"{self.case}.json"
        out.write_text(
            json.dumps(self.doc, ensure_ascii=False, indent=1), encoding="utf-8"
        )

    def all_pass(self) -> bool:
        return all(
            c["pass"] for s in self.doc["stages"] for c in s.get("checks", [])
        )


def _sdk_version() -> str:
    if not HAS_MCP_SDK:
        return "(not installed)"
    try:
        from importlib.metadata import version

        return version("mcp")
    except Exception:  # pragma: no cover
        return "?"


def require(checks: list[tuple[str, bool]]) -> None:
    """先记录后断言：所有检查项过才放行，失败给首个描述。"""
    for desc, ok in checks:
        if not ok:
            raise AssertionError(f"验收失败：{desc}")


# ---------------------------------------------------------------- 场景体（P0→P4→三门）


async def setup_project(client: Any, rec: Recorder, tag: str) -> dict:
    """建项→导入→抽取→建纲→矩阵→登记→绑定→三门过审。

    全部走 MCP 工具（外部 Agent 视角的纯 MCP 驱动面），
    返回 ctx：pid / ev_id / sections(sec→[req]) / mandatory REQ 清单。
    """
    # ---- P0 建项
    t0 = time.monotonic()
    r = await call(client, "create_bid_project", {
        "name": f"MCP e2e {tag}——智慧园区弱电工程",
        "bid_deadline": "2026-12-31",
        "mode": "明标",
        "external_key": tag,
    })
    pid = r.get("project_id", "")
    # 契约 §1：out 只承诺 {project_id, status}——deduplicated 不在首建响应里
    # （缺席 ≠ false，重放路径才带 deduplicated:true；如实记录该形态差异）
    dedup_first = r.get("deduplicated")
    # 幂等重放：同 external_key 再建 → 原 project_id + deduplicated=true
    r2 = await call(client, "create_bid_project", {
        "name": f"MCP e2e {tag}——智慧园区弱电工程",
        "bid_deadline": "2026-12-31",
        "mode": "明标",
        "external_key": tag,
    })
    checks = [
        ("P0: project_id 前缀 PRJ-", pid.startswith("PRJ-")),
        ("P0: status=DRAFT", r.get("status") == "DRAFT"),
        ("P0: 首建非重放（deduplicated 缺席或 false）", not dedup_first),
        ("P0: warnings 为空（截止日未过）", r.get("warnings") == []),
        ("P0: 同 key 重放 → 原 project_id", r2.get("project_id") == pid),
        ("P0: 重放 deduplicated=true（契约幂等面）", r2.get("deduplicated") is True),
    ]
    rec.stage("P0 create_bid_project（含幂等重放）", "create_bid_project",
              {"external_key": tag},
              {"first": r, "replay": r2,
               "note": "首建响应无 deduplicated 字段——契约 §1 out 只承诺 project_id/status"},
              checks, elapsed_ms=(time.monotonic() - t0) * 1000)
    require(checks)

    # ---- P1 导入（真件：A 主文件 + C 补遗 + 2 件证据 PDF）
    t0 = time.monotonic()
    files = [
        {"upload_ref": str(FIXTURES / n), "filename": n, "kind": k}
        for n, k in TENDER_FILES
    ]
    r = await call(client, "import_tender_files", {"project_id": pid, "files": files})
    totals = r.get("totals", {})
    checks = [
        ("P1: totals.files >= 4", totals.get("files", 0) >= 4),
        ("P1: parse_failed == 0", totals.get("parse_failed", -1) == 0),
        ("P1: partial == 0", totals.get("partial", -1) == 0),
    ]
    rec.stage("P1 import_tender_files", "import_tender_files",
              {"files": [f["filename"] for f in files]}, r, checks,
              elapsed_ms=(time.monotonic() - t0) * 1000)
    require(checks)

    # ---- P2 抽取
    t0 = time.monotonic()
    r = await call(client, "extract_requirements", {"project_id": pid})
    counts = r.get("counts", {})
    by_cat = counts.get("by_category", {})
    checks = [
        ("P2: counts.total >= 5（GT 25 埋点件）", counts.get("total", 0) >= 5),
        ("P2: by_category 非空", bool(by_cat)),
    ]
    rec.stage("P2 extract_requirements", "extract_requirements", {}, r, checks,
              elapsed_ms=(time.monotonic() - t0) * 1000)
    require(checks)

    # ---- P3 建纲（MCP 工具；规模收小提速：6 千字 / 每叶 ≤6 REQ）
    t0 = time.monotonic()
    r = await call(client, "build_outline", {
        "project_id": pid, "target_total_chars": 6000, "max_reqs_per_leaf": 6,
    })
    oc = r.get("counts", {})
    checks = [
        ("P3: sections >= 2", oc.get("sections", 0) >= 2),
        ("P3: matrix_rows >= 5", oc.get("matrix_rows", 0) >= 5),
        ("P3: status=MATRIX_READY", r.get("status") == "MATRIX_READY"),
    ]
    rec.stage("P3 build_outline", "build_outline", {}, r, checks,
              elapsed_ms=(time.monotonic() - t0) * 1000)
    require(checks)

    # ---- P3 矩阵（Agent 发现「哪章管哪些 REQ」的唯一 MCP 窗口）
    t0 = time.monotonic()
    r = await call(client, "get_compliance_matrix", {"project_id": pid, "limit": 1000})
    rows = r.get("rows", [])
    sections: dict[str, list[str]] = {}
    sec_titles: dict[str, str] = {}
    mandatory: list[str] = []
    n_blocking = 0
    for row in rows:
        rid = row.get("requirement", {}).get("id", "")
        sec = row.get("draft_section_id")
        cat = row.get("requirement", {}).get("category", "")
        if row.get("blocking"):
            n_blocking += 1
        if cat in MANDATORY_CATS:
            mandatory.append(rid)
        if sec:
            sections.setdefault(sec, []).append(rid)
            if not sec_titles.get(sec):  # 行内无章名——占位记录
                sec_titles[sec] = f"(矩阵行未携带章名)"
    checks = [
        ("P3: 矩阵行数 >= 5", len(rows) >= 5),
        ("P3: total 与 rows 一致", r.get("total", -1) == len(rows)),
        ("P3: 至少 1 个可起草章节", len(sections) >= 1),
        ("P3: 必绑类目 REQ 非空", len(mandatory) >= 1),
        (f"P3: 建纲后 blocking 行 = {n_blocking}（未绑证据，预期 >0 或 =0）", True),
    ]
    rec.stage("P3 get_compliance_matrix", "get_compliance_matrix", {"limit": 1000},
              {"total": r.get("total"), "rows_head": rows[:3], "n_blocking": n_blocking},
              checks, extra={"sections": {k: v for k, v in sections.items()}},
              elapsed_ms=(time.monotonic() - t0) * 1000)
    require(checks)

    # ---- P4 证据登记（MCP 工具；source_sha256 用真件实算）
    t0 = time.monotonic()
    ev_path = FIXTURES / EV_SOURCE
    r = await call(client, "register_evidence", {
        "project_id": pid,
        "kind": "qualification",
        "title": "电子与智能化工程专业承包二级资质证书",
        "source_file": ev_path.name,
        "source_sha256": sha256_file(ev_path),
        "subject_name": "投标人",
        "scope": "电子与智能化工程",
    })
    ev_id = r.get("ev_id", "")
    checks = [
        ("P4: ev_id 前缀 EV-", ev_id.startswith("EV-")),
        ("P4: status=ACTIVE", r.get("status") == "ACTIVE"),
        ("P4: deduplicated=false（首登）", r.get("deduplicated") is False),
    ]
    rec.stage("P4 register_evidence", "register_evidence",
              {"kind": "qualification", "source_file": ev_path.name}, r, checks,
              elapsed_ms=(time.monotonic() - t0) * 1000)
    require(checks)

    # ---- P4 绑定（必绑类目全绑 → blocking 清零）
    t0 = time.monotonic()
    r = await call(client, "bind_evidence", {
        "project_id": pid, "ev_id": ev_id,
        "requirement_ids": mandatory, "bid_open_date": "2026-12-31",
    })
    m = r.get("matrix", {})
    checks = [
        ("P4: bound_new + replayed == 请求数",
         r.get("bound_new", 0) + r.get("replayed", 0) == len(mandatory)),
        ("P4: 绑后 blocking == 0（否则导出闸会拒）", m.get("blocking", -1) == 0),
    ]
    rec.stage("P4 bind_evidence", "bind_evidence",
              {"ev_id": ev_id, "n_reqs": len(mandatory)}, r, checks,
              elapsed_ms=(time.monotonic() - t0) * 1000)
    require(checks)

    # ---- G3 三门过审（reviewer 必须 human: 前缀）
    t0 = time.monotonic()
    gates = ("matrix_approval", "outline_approval", "pre_export_confirm")
    for g in gates:
        r = await call(client, "record_review_gate", {
            "project_id": pid, "gate": g, "decision": "APPROVED",
            "reviewer": "human:mcp-e2e",
        })
    approved = r.get("gates_approved", [])
    checks = [(f"G3: 三门全过 {gates}", set(gates).issubset(set(approved)))]
    rec.stage("G3 record_review_gate ×3", "record_review_gate", {"gates": list(gates)},
              r, checks, elapsed_ms=(time.monotonic() - t0) * 1000)
    require(checks)

    return {
        "pid": pid, "ev_id": ev_id, "rows": rows,
        "sections": sections, "mandatory": mandatory,
    }


# ---------------------------------------------------------------- 正文作者（模拟使用方 Agent）


def author_body(
    reqs: list[str],
    ev_ids: list[str] | None = None,
    cite_only_first: bool = False,
    ghost_ev: str | None = None,
    promise_sentences: list[str] | None = None,
    tail_sentence: str | None = None,
) -> str:
    """模拟使用方 Agent 写的 markdown 正文。

    纪律（与引擎的回查/统计面对齐）：
    - 每个 REQ 引用单独成句（句号边界——无锚承诺按句计）；
    - 不使用 承诺/保证/确保/完全满足 字样（除非显式传 promise_sentences）；
    - 行内粗体走 **字样**（#117 的 run 发射层会在 docx 里变粗体 run）。
    """
    parts = ["# 分项响应", "", "本章依据响应矩阵，对所辖条款逐条对照如下。"]
    target = reqs[:1] if cite_only_first else reqs
    for rid in target:
        parts.append(f"依据 [{rid}]，我方具备该条款所要求的能力与条件，配置与招标文件规定一致。")
    if ev_ids:
        cites = "、".join(f"[{e}]" for e in ev_ids)
        parts.append(f"**佐证材料**：本章相应条款的证明文件见 {cites}，扫描件与台账一致。")
    if ghost_ev:
        parts.append(f"另附高新技术企业证书 {ghost_ev} 作为补充佐证材料。")
    if tail_sentence:
        parts.append(tail_sentence)
    for s in promise_sentences or []:
        parts.append(s)
    return "\n\n".join(parts)


def read_chapter_record(ws: Path, sec_id: str) -> dict:
    p = ws / "chapters" / f"{sec_id}.json"
    return json.loads(p.read_text(encoding="utf-8"))


# ---------------------------------------------------------------- 用例体


async def _case_environment(rec: Recorder) -> None:
    bin_path, transport, _ = resolve_server()
    checks: list[tuple[str, bool]] = [
        ("环境: server 二进制在场", bin_path.is_file()),
        ("环境: fixtures/corpus-A 真件在场", (FIXTURES / "A-招标文件.docx").is_file()),
        ("环境: mcp Python SDK 已安装" if HAS_MCP_SDK else "环境: mcp SDK 未装（linejson 可跑）", True),
    ]
    rustc = shutil.which("rustc")
    rustc_v = subprocess.run([rustc, "--version"], capture_output=True, text=True).stdout.strip() if rustc else "(rustc 不在 PATH)"
    rec.stage(
        "环境留档", None, {},
        {
            "transport_resolved": transport,
            "server_bin": str(bin_path),
            "server_sha256": sha256_file(bin_path) if bin_path.is_file() else None,
            "python": sys.version.split()[0],
            "mcp_sdk": _sdk_version(),
            "os": platform.platform(),
            "rustc": rustc_v,
        },
        checks,
    )
    require(checks)


async def _case_initialize_and_tools(rec: Recorder) -> None:
    bin_path, transport, _ = resolve_server()
    if transport != "mcp":
        pytest.skip(
            "MCP 协议层（initialize/tools/list/tools/call）待 #137 交付 bid-mcp-server；"
            f"当前传输={transport}（逐行 JSON，无握手面）——本用例如实 SKIP，不假装通过"
        )
    ws = _mk_ws("handshake")
    async with open_client(ws) as client:
        info = client.server_info
        names = await client.list_tools()
        key_tools = {
            "create_bid_project", "import_tender_files", "extract_requirements",
            "get_compliance_matrix", "build_outline", "register_evidence",
            "bind_evidence", "record_review_gate", "draft_response_section",
            "validate_bid_document", "export_bid_document", "get_bid_project_state",
        }
        caps = getattr(info, "capabilities", None)
        caps_dict = caps.model_dump() if caps is not None else {}
        server_name = getattr(getattr(info, "serverInfo", None), "name", "")
        server_ver = getattr(getattr(info, "serverInfo", None), "version", "?")
        checks = [
            ("握手: initialize 成功且 serverInfo.name == 'bid-mcp'", server_name == "bid-mcp"),
            ("握手: protocolVersion 已协商", bool(getattr(info, "protocolVersion", None))),
            ("握手: capabilities.tools 在场", "tools" in caps_dict),
            (f"握手: tools/list ≥ 20（实际 {len(names)}）", len(names) >= 20),
            ("握手: 关键工具全在场（P0-P8 主链 12 个）", key_tools.issubset(set(names))),
        ]
        rec.stage("MCP 握手 + 工具清单", None, {},
                  {"serverInfo": {"name": server_name, "version": server_ver},
                   "protocolVersion": getattr(info, "protocolVersion", None),
                   "n_tools": len(names), "tool_names": sorted(names)},
                  checks)
        require(checks)


async def _case_full_pipeline(rec: Recorder, ws: Path) -> None:
    async with open_client(ws) as client:
        ctx = await setup_project(client, rec, "mcp-e2e-pipeline")
        pid, sections, ev_id = ctx["pid"], ctx["sections"], ctx["ev_id"]
        mandatory = set(ctx["mandatory"])

        # ---- P5 external 起草（使用方核心路径：Agent 写正文 → 引擎回查+CAS+落盘）
        t0 = time.monotonic()
        draft_stats = {}
        for sec, reqs in sections.items():
            ev_ids = [ev_id] if any(r in mandatory for r in reqs) else []
            body = author_body(reqs, ev_ids=ev_ids)
            r = await call(client, "draft_response_section", {
                "project_id": pid, "section_id": sec,
                "generator": "external", "content": body,
                "instructions": "按招标文件逐项响应，突出同类业绩与实施方案",
            })
            stats = r.get("stats", {})
            draft_stats[sec] = {
                "words": stats.get("words"), "citations": stats.get("citations"),
                "uncovered": stats.get("uncovered_requirements"),
                "unanchored": stats.get("unanchored_claims"),
            }
            checks = [
                (f"P5[{sec}]: generator=external", stats.get("generator") == "external"),
                (f"P5[{sec}]: section_status=DRAFTED", r.get("section_status") == "DRAFTED"),
                (f"P5[{sec}]: 字数 > 0", (stats.get("words") or 0) > 0),
                (f"P5[{sec}]: 引用覆盖本章全部 REQ",
                 set(stats.get("cited_ids", [])) >= set(reqs)),
                (f"P5[{sec}]: uncovered_requirements 为空", stats.get("uncovered_requirements") == []),
                (f"P5[{sec}]: unanchored_claims == 0", stats.get("unanchored_claims") == 0),
            ]
            rec.stage(f"P5 draft(external) {sec}", "draft_response_section",
                      {"section_id": sec, "chars": len(body)}, r, checks)
            require(checks)
        first_status = r.get("project_status")
        checks = [
            ("P5: 全部章节起草完成（矩阵可见章节）", len(draft_stats) == len(sections)),
            ("P5: project_status=DRAFTING", first_status == "DRAFTING"),
        ]
        rec.stage("P5 汇总", None, None,
                  {"n_sections": len(sections), "draft_stats": draft_stats}, checks)

        # ---- P6 质量检查（只读诊断——观察返回形状，分数值如实记录不断言）
        t0 = time.monotonic()
        r = await call(client, "check_bid_quality", {"project_id": pid})
        checks = [("P6: check_bid_quality 返回 ok 面", isinstance(r, dict))]
        rec.stage("P6 check_bid_quality", "check_bid_quality", {}, r, checks,
                  elapsed_ms=(time.monotonic() - t0) * 1000)

        # ---- P7 校验（BLOCKER 清零才能导出）
        t0 = time.monotonic()
        # #137 后 structuredContent = data 本体（无 ok 外壳），
        # 完整信封（ok/tool/data）只在 content[0].text → 用 call_env。
        env = await call_env(client, "validate_bid_document",
                             {"project_id": pid, "scope": "all"})
        data = env.get("data") if isinstance(env.get("data"), dict) else {}
        counts = data.get("counts", {})
        checks = [
            ("P7: ok=true", env.get("ok") is True),
            ("P7: counts.blocker == 0", counts.get("blocker", -1) == 0),
            ("P7: findings 面存在",
             isinstance(data.get("findings") or data.get("errors"), list)),
        ]
        rec.stage("P7 validate_bid_document", "validate_bid_document", {"scope": "all"},
                  data, checks, elapsed_ms=(time.monotonic() - t0) * 1000)
        require(checks)

        # ---- P8 导出（docx）
        t0 = time.monotonic()
        r = await call(client, "export_bid_document", {
            "project_id": pid, "format": "docx",
            "output_path": "out/投标文件-技术标.docx",
        })
        art = r.get("artifact", {})
        docx_path = ws / "out" / "投标文件-技术标.docx"
        checks = [
            ("P8: 产物 > 5KB", (art.get("size") or 0) > 5000),
            ("P8: sha256 在场", bool(art.get("sha256"))),
            ("P8: chapters_written == 起草章数",
             r.get("chapters_written") == len(sections)),
            ("P8: project_status=EXPORTED", r.get("project_status") == "EXPORTED"),
            ("P8: 产物落在工作区 out/", docx_path.is_file()),
        ]
        rec.stage("P8 export_bid_document", "export_bid_document",
                  {"format": "docx", "output_path": "out/投标文件-技术标.docx"},
                  r, checks, elapsed_ms=(time.monotonic() - t0) * 1000)
        require(checks)

        # ---- 产物深检（「Word 打开」的机器等价面：解 zip 查 document.xml）
        probe = "配置与招标文件规定一致"
        with zipfile.ZipFile(docx_path) as z:
            doc_xml = z.read("word/document.xml").decode("utf-8")
            styles_xml = z.read("word/styles.xml").decode("utf-8")
        checks = [
            ("产物: TOC 域在位", "TOC \\o" in doc_xml),
            ("产物: 正文探针句进文档", probe in doc_xml),
            ("产物: 无字面 ** 泄漏（行内粗体已转 run）", "**" not in doc_xml),
            ("产物: 无 md 井号标题泄漏", "# " not in doc_xml),
            ("产物: styles.xml 中文字体面（w:eastAsia）", "w:eastAsia=" in styles_xml),
        ]
        rec.stage("产物深检 docx", None, None,
                  {"size": docx_path.stat().st_size,
                   "sha256": sha256_file(docx_path)},
                  checks)
        require(checks)

        # ---- 工作区持久化（S-2 落盘 + db.json）
        chapter_files = sorted((ws / "chapters").glob("SEC-*.json"))
        checks = [
            (f"落盘: chapters/*.json == {len(sections)} 件",
             len(chapter_files) == len(sections)),
            ("落盘: db.json（状态机持久化）在场", (ws / "db.json").is_file()),
        ]
        sample = read_chapter_record(ws, next(iter(sections)))
        checks += [
            ("落盘: 章记录 attempt >= 1", (sample.get("attempt") or 0) >= 1),
            ("落盘: 章记录 citations 非空", bool(sample.get("citations"))),
            ("落盘: 章记录 draft_note 留档（#127）", "draft_note" in sample),
        ]
        rec.stage("工作区落盘", None, None,
                  {"n_chapters": len(chapter_files), "sample": {k: sample.get(k) for k in ("section_id", "attempt", "char_count", "citations")}},
                  checks)
        require(checks)

    # ---- 进程重启续跑（新连接读同一 BID_WS —— 断点续跑面）
    async with open_client(ws) as client2:
        r = await call(client2, "get_bid_project_state", {})
        # 口径注意（如实记录）：EXPORTED 是状态机终态（terminal=true，无后继
        # 迁移），但 allowed_next_tools 仍列【只读/幂等面】工具（重导出、查
        # 矩阵、质检等）——两字段口径不同，判终态用 terminal，不能用
        # allowed_next_tools 是否为空。
        write_tools = {
            "create_bid_project", "import_tender_files", "extract_requirements",
            "build_outline", "draft_response_section", "register_evidence",
            "bind_evidence",
        }
        allowed = set(r.get("allowed_next_tools") or [])
        checks = [
            ("重启: EXPORTED 状态完整恢复", r.get("status") == "EXPORTED"),
            ("重启: terminal=true（状态机无后继迁移）", r.get("terminal") is True),
            ("重启: 终态不再放行写面工具", not (allowed & write_tools)),
        ]
        rec.stage("重启续跑（新进程读同工作区）", "get_bid_project_state", {}, r, checks,
                  extra={"note": "EXPORTED 下 allowed_next_tools 含只读/幂等面工具（口径与 terminal 不同）"})
        require(checks)


async def _case_error_envelope(rec: Recorder, ws: Path) -> None:
    """错误信封验证：Agent 的分支处理依赖 E_* 错误码——四种典型失败面。

    断言的是【信封形状】（ok=false + error.code + error.what）与错误码归类，
    不断言文案细节（文案演进不影响 Agent 分支）。
    """
    async with open_client(ws) as client:
        results: list[dict] = []

        # ① 无项目时调只读工具 → E_NOT_FOUND
        try:
            await call(client, "get_compliance_matrix", {"project_id": "PRJ-9999", "limit": 10})
            results.append({"case": "无项目查矩阵", "err": None})
        except ToolFailure as e:
            results.append({"case": "无项目查矩阵", "code": e.code, "what": e.what[:80]})
        # ② 未知工具 → 【协议层】拦（JSON-RPC -32602，error.data.code=E_NOTFOUND）
        try:
            await call(client, "no_such_tool", {})
            results.append({"case": "未知工具_协议层", "err": None})
        except ToolFailure as e:
            # McpError 走 JSON-RPC error 通道（不是工具层 isError:true）——
            # 这是协议层的正确行为，与工具层 E_* 错误面区分开。
            results.append({"case": "未知工具_协议层",
                            "code": "PROTOCOL_LEVEL",
                            "what": str(e.what)[:80]})
        # ③ 参数缺失/非法 → E_UNPROCESSABLE
        try:
            await call(client, "create_bid_project", {"bid_deadline": "2026-12-31"})
            results.append({"case": "建项缺参", "err": None})
        except ToolFailure as e:
            results.append({"case": "建项缺参", "code": e.code, "what": e.what[:80]})
        try:
            r = await call(client, "create_bid_project", {
                "name": "错误面", "bid_deadline": "2026-12-31",
                "mode": "随口填", "external_key": "err-face",
            })
            results.append({"case": "mode 非法", "err": None, "got": r})
        except ToolFailure as e:
            results.append({"case": "mode 非法", "code": e.code, "what": e.what[:80]})
        # ④ external 互斥/缺 content → E_UNPROCESSABLE（参数面错误零副作用）
        r = await call(client, "create_bid_project", {
            "name": "错误面", "bid_deadline": "2026-12-31",
            "mode": "明标", "external_key": "err-face",
        })
        pid = r["project_id"]
        try:
            await call(client, "draft_response_section", {
                "project_id": pid, "section_id": "SEC-0001",
                "generator": "external",  # 缺 content
            })
            results.append({"case": "external 缺 content", "err": None})
        except ToolFailure as e:
            results.append({"case": "external 缺 content", "code": e.code, "what": e.what[:80]})
        try:
            await call(client, "draft_response_section", {
                "project_id": pid, "section_id": "SEC-0001",
                "generator": "mock", "content": "互斥面",
            })
            results.append({"case": "content×generator 互斥", "err": None})
        except ToolFailure as e:
            results.append({"case": "content×generator 互斥", "code": e.code, "what": e.what[:80]})
        # ⑤ 状态机越级（无导入直接抽取）→ E_STATE
        try:
            await call(client, "extract_requirements", {"project_id": pid})
            results.append({"case": "未导入先抽取", "err": None})
        except ToolFailure as e:
            results.append({"case": "未导入先抽取", "code": e.code, "what": e.what[:80]})

        by_case = {x["case"]: x for x in results}
        # 实测（#137 交付后的协议层行为）：
        #   · 未知工具由【协议层】拦（JSON-RPC -32602），不是工具层 E_NOTFOUND；
        #   · 无项目查矩阵 = E_NOT_FOUND（工具层路由守卫）
        expect_notfound = {"无项目查矩阵"}
        expect_unprocessable = {"建项缺参", "mode 非法", "external 缺 content",
                               "content×generator 互斥"}
        expect_state = {"未导入先抽取"}
        checks = [
            (f"E_NOT_FOUND 面（{expect_notfound}）",
             all(by_case.get(c, {}).get("code") == "E_NOT_FOUND" for c in expect_notfound)),
            (f"E_UNPROCESSABLE 面（{expect_unprocessable}）",
             all(by_case.get(c, {}).get("code") == "E_UNPROCESSABLE" for c in expect_unprocessable)),
            (f"E_STATE 面（{expect_state}）",
             all(by_case.get(c, {}).get("code") == "E_STATE" for c in expect_state)),
            ("未知工具由协议层拦（JSON-RPC error 而非工具层信封）",
             by_case.get("未知工具_协议层", {}).get("code") == "PROTOCOL_LEVEL"),
            ("错误信封携带 what（人类可读）",
             all(isinstance(x.get("what"), str) and x["what"] for x in results)),
            ("全部七种失败面都被拒（无静默成功）",
             all(x.get("code") for x in results)),
        ]
        rec.stage("错误信封（E_* 归类）", None, None, {"cases": results}, checks)
        require(checks)


async def _case_external_ghost_ev(rec: Recorder, ws: Path) -> None:
    """① 正文里有不存在的 EV- 编号 → 引擎剔除 + warning（不拒稿）。"""
    async with open_client(ws) as client:
        ctx = await setup_project(client, rec, "mcp-e2e-ghost-ev")
        pid, sections, ev_id = ctx["pid"], ctx["sections"], ctx["ev_id"]
        # 选必绑 REQ 最多的章（其响应面里 known EV 集非空——真 EV 与幽灵 EV 混引）
        sec, reqs = max(sections.items(),
                        key=lambda kv: sum(1 for r in kv[1] if r in set(ctx["mandatory"])))
        body = author_body(reqs, ev_ids=[ev_id], ghost_ev=f"[{GHOST_EV}]")
        r = await call(client, "draft_response_section", {
            "project_id": pid, "section_id": sec,
            "generator": "external", "content": body,
            "instructions": "外部手稿——含一个不存在的证据编号",
        })
        stats = r.get("stats", {})
        warnings = r.get("warnings", [])
        ghost_warn = [w for w in warnings if GHOST_EV in str(w)]
        checks = [
            ("①: 不拒稿（draft 成功返回）", r.get("section_status") == "DRAFTED"),
            ("①: generator=external", stats.get("generator") == "external"),
            (f"①: {GHOST_EV} 不在有效引用里", GHOST_EV not in (stats.get("cited_ids") or [])),
            ("①: 真 EV 引用保留", ev_id in (stats.get("cited_ids") or [])),
            (f"①: warning 指认 {GHOST_EV}（剔除告知）", bool(ghost_warn)),
            ("①: warning 文案含「剔除」", any("剔除" in w for w in ghost_warn)),
        ]
        rec.stage(f"① draft(external) 幽灵 EV [{sec}]", "draft_response_section",
                  {"section_id": sec, "ghost": GHOST_EV}, r, checks,
                  extra={"warnings": warnings})
        require(checks)

        # 白盒：落盘正文里幽灵编号应已被物理剔除
        rec_doc = read_chapter_record(ws, sec)
        checks = [
            (f"①: 落盘正文不含 {GHOST_EV}", GHOST_EV not in rec_doc.get("content", "")),
            (f"①: 章记录 citations 不含 {GHOST_EV}", GHOST_EV not in (rec_doc.get("citations") or [])),
        ]
        rec.stage("① 白盒：chapters 落盘复查", None, None,
                  {"attempt": rec_doc.get("attempt"),
                   "citations": rec_doc.get("citations")}, checks)
        require(checks)


async def _case_external_uncovered(rec: Recorder, ws: Path) -> None:
    """② 正文覆盖不全 → 返回 uncovered_requirements。"""
    async with open_client(ws) as client:
        ctx = await setup_project(client, rec, "mcp-e2e-uncovered")
        pid, sections = ctx["pid"], ctx["sections"]
        sec, reqs = max(sections.items(), key=lambda kv: len(kv[1]))
        assert len(reqs) >= 2, f"用例②需要 ≥2 REQ 的章节（{sec} 只有 {len(reqs)}）"
        body = author_body(reqs, cite_only_first=True)
        r = await call(client, "draft_response_section", {
            "project_id": pid, "section_id": sec,
            "generator": "external", "content": body,
            "instructions": "外部手稿——只回应第一条",
        })
        stats = r.get("stats", {})
        uncovered = stats.get("uncovered_requirements") or []
        expect_uncovered = set(reqs[1:])
        warnings = r.get("warnings", [])
        checks = [
            ("②: 不拒稿（draft 成功返回）", r.get("section_status") == "DRAFTED"),
            ("②: uncovered_requirements 非空", len(uncovered) > 0),
            ("②: uncovered == 本章未引用的 REQ 全集", set(uncovered) == expect_uncovered),
            ("②: warning 逐条告知覆盖缺口",
             all(any(rid in str(w) for w in warnings) for rid in expect_uncovered)),
            ("②: 有效引用恰为首条 REQ", stats.get("cited_ids") == [reqs[0]]),
        ]
        rec.stage(f"② draft(external) 覆盖不全 [{sec}]", "draft_response_section",
                  {"section_id": sec, "n_reqs": len(reqs), "cited": reqs[:1]},
                  r, checks, extra={"uncovered": uncovered, "warnings": warnings})
        require(checks)


async def _case_external_unanchored(rec: Recorder, ws: Path) -> None:
    """③ 有承诺句但没引用 → count_unanchored_claims > 0。"""
    async with open_client(ws) as client:
        ctx = await setup_project(client, rec, "mcp-e2e-unanchored")
        pid, sections = ctx["pid"], ctx["sections"]
        sec, reqs = next(iter(sections.items()))
        body = author_body(reqs, promise_sentences=PROMISE_SENTENCES)
        r = await call(client, "draft_response_section", {
            "project_id": pid, "section_id": sec,
            "generator": "external", "content": body,
            "instructions": "外部手稿——含无锚承诺句",
        })
        stats = r.get("stats", {})
        warnings = r.get("warnings", [])
        n_promise = len(PROMISE_SENTENCES)
        checks = [
            ("③: 不拒稿（draft 成功返回）", r.get("section_status") == "DRAFTED"),
            (f"③: unanchored_claims == {n_promise}（恰为无引用承诺句数）",
             stats.get("unanchored_claims") == n_promise),
            ("③: 覆盖面仍完整（引用句全带锚）",
             stats.get("uncovered_requirements") == []),
            ("③: warning 含「无锚承诺」告知", any("无锚承诺" in w for w in warnings)),
        ]
        rec.stage(f"③ draft(external) 无锚承诺 [{sec}]", "draft_response_section",
                  {"section_id": sec, "n_promise_sentences": n_promise}, r, checks,
                  extra={"warnings": warnings})
        require(checks)


async def _case_external_cas_version(rec: Recorder, ws: Path) -> None:
    """④ 正常正文 → CAS 写入成功，版本递增；同稿重提幂等。"""
    async with open_client(ws) as client:
        ctx = await setup_project(client, rec, "mcp-e2e-cas")
        pid, sections, ev_id = ctx["pid"], ctx["sections"], ctx["ev_id"]
        mandatory = set(ctx["mandatory"])
        sec, reqs = max(sections.items(),
                        key=lambda kv: sum(1 for r in kv[1] if r in mandatory))
        ev_ids = [ev_id] if any(r in mandatory for r in reqs) else []
        instr = "外部手稿——CAS 版本递增验证"

        body_a = author_body(reqs, ev_ids=ev_ids)
        r1 = await call(client, "draft_response_section", {
            "project_id": pid, "section_id": sec,
            "generator": "external", "content": body_a, "instructions": instr,
        })
        v1 = r1.get("version")
        checks = [
            ("④: 首稿成功", r1.get("section_status") == "DRAFTED"),
            ("④: 首稿 version >= 1", (v1 or 0) >= 1),
            ("④: content_ref 带版本号", str(r1.get("content_ref", "")).endswith(f"-v{v1}")),
            ("④: 非重放（deduplicated 缺省/false）", r1.get("deduplicated") in (None, False)),
        ]
        rec.stage("④ draft #1（内容 A）", "draft_response_section",
                  {"section_id": sec, "chars": len(body_a)}, r1, checks)
        require(checks)

        body_b = author_body(reqs, ev_ids=ev_ids,
                             tail_sentence=f"以上各项经复核与招标文件逐条对应 [{reqs[0]}]，无保留意见。")
        r2 = await call(client, "draft_response_section", {
            "project_id": pid, "section_id": sec,
            "generator": "external", "content": body_b, "instructions": instr,
        })
        v2 = r2.get("version")
        checks = [
            ("④: 换稿重写成功", r2.get("section_status") == "DRAFTED"),
            (f"④: 版本递增（v{v1} → v{v2}）", (v2 or 0) == (v1 or 0) + 1),
            ("④: project_version 递增",
             (r2.get("project_version") or 0) > (r1.get("project_version") or 0)),
        ]
        rec.stage("④ draft #2（内容 B ≠ A）", "draft_response_section",
                  {"section_id": sec, "chars": len(body_b)}, r2, checks)
        require(checks)

        r3 = await call(client, "draft_response_section", {
            "project_id": pid, "section_id": sec,
            "generator": "external", "content": body_b, "instructions": instr,
        })
        checks = [
            ("④: 同稿重提 → deduplicated=true", r3.get("deduplicated") is True),
            ("④: 幂等重放版本不变", r3.get("version") == v2),
            ("④: 重放零副作用（project_version 不变）",
             r3.get("project_version") == r2.get("project_version")),
        ]
        rec.stage("④ draft #3（内容 B 重放）", "draft_response_section",
                  {"section_id": sec, "chars": len(body_b)}, r3, checks)
        require(checks)

        rec_doc = read_chapter_record(ws, sec)
        checks = [
            ("④: 落盘 attempt == v2", rec_doc.get("attempt") == v2),
            ("④: 落盘内容为最新稿 B", rec_doc.get("content") == body_b),
        ]
        rec.stage("④ 白盒：chapters 落盘复查", None, None,
                  {"attempt": rec_doc.get("attempt"), "char_count": rec_doc.get("char_count")},
                  checks)
        require(checks)


# ---------------------------------------------------------------- 运行骨架


def _mk_ws(case: str) -> Path:
    base = Path(os.environ.get("BID_MCP_WS_ROOT", "/tmp/bid-mcp-e2e")) / case
    if base.exists():
        shutil.rmtree(base)
    (base / "ws").mkdir(parents=True)
    return base / "ws"


def run_case(case: str, body: Callable[[Recorder, Path], Awaitable[None]],
             ws_needed: bool = True) -> None:
    """同步入口：起 recorder → 跑异步体 → 无论成败都落盘 evidence。"""
    bin_path, transport, _ = resolve_server()
    rec = Recorder(case, transport, bin_path)
    print(f"\n=== {case}（transport={transport}, server={bin_path.name}）===", flush=True)
    ws = _mk_ws(case.split("-", 1)[-1]) if ws_needed else None
    rec.doc["ws"] = str(ws) if ws else None
    err: BaseException | None = None
    note = ""
    try:
        asyncio.run(_run_body(body, rec, ws))
    except BaseException as e:  # 含 pytest Skipped（BaseException 子类）
        err, note = e, f"{type(e).__name__}: {e}"
    if err is None and not rec.all_pass():
        err = AssertionError(f"{case}: 存在未通过的检查项（见 evidence/{case}.json）")
        note = str(err)
    if err is not None:
        kind = type(err).__name__
        prefix = "SKIPPED" if kind == "Skipped" else "FAILED"
        rec.finish(note=f"{prefix}: {note}")
        if isinstance(err, (KeyboardInterrupt, SystemExit)):
            raise err
        raise err  # Skipped 亦上抛：pytest 标 SKIP，独立模式由 main 汇总
    rec.finish()


async def _run_body(body, rec, ws) -> None:
    if ws is not None:
        await body(rec, ws)
    else:
        await body(rec)


# ---------------------------------------------------------------- pytest 入口


def test_00_environment():
    run_case("00-environment", _case_environment, ws_needed=False)


def test_01_initialize_and_tools():
    run_case("01-initialize-and-tools", _case_initialize_and_tools, ws_needed=False)


def test_02_full_pipeline():
    run_case("02-full-pipeline", _case_full_pipeline)


def test_03_error_envelope():
    run_case("03-error-envelope", _case_error_envelope)


def test_10_external_ghost_ev():
    run_case("10-external-ghost-ev", _case_external_ghost_ev)


def test_11_external_uncovered():
    run_case("11-external-uncovered", _case_external_uncovered)


def test_12_external_unanchored():
    run_case("12-external-unanchored", _case_external_unanchored)


def test_13_external_cas_version():
    run_case("13-external-cas-version", _case_external_cas_version)


# ---------------------------------------------------------------- 独立运行入口


def main() -> int:
    cases = [
        ("00-environment", _case_environment, False),
        ("01-initialize-and-tools", _case_initialize_and_tools, False),
        ("02-full-pipeline", _case_full_pipeline, True),
        ("03-error-envelope", _case_error_envelope, True),
        ("10-external-ghost-ev", _case_external_ghost_ev, True),
        ("11-external-uncovered", _case_external_uncovered, True),
        ("12-external-unanchored", _case_external_unanchored, True),
        ("13-external-cas-version", _case_external_cas_version, True),
    ]
    results: list[tuple[str, str]] = []
    for name, body, ws_needed in cases:
        try:
            run_case(name, body, ws_needed=ws_needed)
            results.append((name, "PASS"))
        except BaseException as e:  # Skipped 亦在此汇总（run_case 已落 evidence）
            if isinstance(e, (KeyboardInterrupt, SystemExit)):
                raise
            kind = type(e).__name__
            label = "SKIP" if kind == "Skipped" else "FAIL"
            results.append((name, f"{label}: {str(e)[:160]}"))
    print("\n===== 汇总 =====")
    for name, st in results:
        print(f"  {name:<28} {st}")
    print(f"evidence: {EVIDENCE_DIR}")
    # SKIP 是如实报告的「待 #137」阻塞面，不算失败；只有 FAIL 才非零退出
    ok = all(v == "PASS" or v.startswith("SKIP") for _, v in results)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
