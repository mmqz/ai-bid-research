#!/usr/bin/env python3
"""bid-response-pipeline 阶段质检脚本（无三方依赖——纯标准库）。

核对工具返回 JSON 与阶段完成判据（SKILL.md 阶段表的机器可执行版）。
每条判据来自 docs/bid-mcp-contract.md 的出参形状——形状不符即 FAIL
（fail-closed，不猜）。

用法：
  python3 checklist.py --stage P1 import.json      # 单文件
  cat reply.json | python3 checklist.py --stage P5 -   # stdin
  python3 checklist.py --dump-version reply.json   # 提取 project_version（并发账）
  python3 checklist.py --list                     # 阶段判据速查

输入 = MCP 应答原文：{ ok, tool, data, request_id, project_version?, warnings? }
退出码：0 全过 / 1 任一判据 FAIL / 2 用法错误。
"""

from __future__ import annotations

import json
import sys
from typing import Any, Callable

# ---------------------------------------------------------------- 判据表


def _dig(doc: dict, path: str) -> Any:
    """点路径取值：_dig(d, "data.totals.parse_failed")。缺路径抛 KeyError。"""
    cur: Any = doc
    for key in path.split("."):
        if not isinstance(cur, dict) or key not in cur:
            raise KeyError(f"应答缺路径 {path!r}（在 {key!r} 断链）")
        cur = cur[key]
    return cur


Stage = tuple[tuple[str, Callable[[dict], tuple[bool, str]]], ...]


def check(path: str, test: Callable[[Any], bool], human: str):
    """构造一条判据：取 path 值 → test → 人读说明。"""

    def _run(doc: dict) -> tuple[bool, str]:
        try:
            val = _dig(doc, path)
        except KeyError as e:
            return False, f"{path}: 缺失（{e}）"
        return (test(val), f"{path} = {val!r}（{human}）")

    return _run


def _is_ok(doc: dict) -> tuple[bool, str]:
    """P1 之外的通用底座：ok=true（E_PARTIAL 是 P1 的合法中间态，单独处理）。"""
    return doc.get("ok") is True, f"ok = {doc.get('ok')!r}（工具成功）"


STAGES: dict[str, Stage] = {
    "P0": (
        (_is_ok,),
        (check("data.project_id", lambda v: isinstance(v, str) and v != "",
               "project_id 已签发（非空串）"),),
    ),
    "P1": (
        # E_PARTIAL 特例：ok=false 但 data 携带成功面——判据落在 totals 与
        # 用户确认流程，脚本只做机器面：parse_failed 必须为 0（严格口径；
        # 有确认降级的人工流程不在机器质检范围）。
        (check("data.totals.parse_failed", lambda v: v == 0,
               "解析失败件 = 0（否则须重试或经用户确认降级）"),),
        (check("data.totals.files", lambda v: isinstance(v, int) and v > 0,
               "导入件数 > 0（复述给用户）"),),
        (check("data.totals.chunks", lambda v: isinstance(v, int) and v > 0,
               "切块数 > 0（复述给用户）"),),
    ),
    "P2": (
        (_is_ok,),
        (check("data.counts.total", lambda v: isinstance(v, int) and v > 0,
               "条款总数 > 0"),),
        (check("data.missing_sequence", lambda v: v == [],
               "缺号为空（非空必须逐号解释）"),),
    ),
    "P3": (
        (_is_ok,),
        (check("data.status", lambda v: v == "MATRIX_READY",
               "建纲后状态 = MATRIX_READY"),),
        (check("data.counts.leaves", lambda v: isinstance(v, int) and v > 0,
               "可起草叶 > 0"),),
    ),
    "P4": (
        (_is_ok,),
        (check("data.matrix.blocking", lambda v: v == 0,
               "绑定后 blocking = 0（资格/否决/技术参数行证据齐）"),),
    ),
    "P5": (
        (_is_ok,),
        (check("data.section_status", lambda v: v == "DRAFTED",
               "章节状态 = DRAFTED"),),
        (check("data.stats.unanchored_claims", lambda v: v == 0,
               "无锚断言 = 0（承诺句必须带 EV 引用）"),),
        (check("data.stats.generator", lambda v: v in ("external", "real", "mock"),
               "实际生成器回显（external 模式应为 external）"),),
    ),
    "P6": (
        (_is_ok,),
        (check("data.counts.blocker", lambda v: v == 0,
               "BLOCKER = 0（非零必须修复或登记 WAIVED）"),),
        # 注意（#243）：校验通过且推进（DRAFTING→VALIDATING）时 project_version
        # +1——下一步工具必须用本响应新值；失败/前置拒零副作用（版本不变）。
        (check("project_version", lambda v: isinstance(v, int),
               "响应携带 project_version（发生推进时为新值——传给下一个工具）"),),
    ),
    "P8": (
        (_is_ok,),
        (check("data.precheck.blockers", lambda v: v == 0,
               "导出预检 BLOCKER = 0"),),
        (check("data.artifact.sha256", lambda v: isinstance(v, str) and v != "",
               "产物 sha256 在场（复述给用户）"),),
        (check("data.artifact.size", lambda v: isinstance(v, int) and v > 0,
               "产物尺寸 > 0"),),
    ),
}

STAGE_TITLES = {
    "P0": "建项 create_bid_project",
    "P1": "导入 import_tender_files",
    "P2": "抽取 extract_requirements",
    "P3": "矩阵 build_outline",
    "P4": "绑定 bind_evidence",
    "P5": "起草 draft_response_section",
    "P6": "校验 validate_bid_document",
    "P8": "导出 export_bid_document",
}

# ---------------------------------------------------------------- 主流程


def load_doc(source: str) -> dict:
    if source == "-":
        raw = sys.stdin.read()
    else:
        with open(source, "r", encoding="utf-8") as fh:
            raw = fh.read()
    doc = json.loads(raw)
    if not isinstance(doc, dict):
        raise ValueError("应答必须是 JSON 对象（MCP 包装 {ok,tool,data,…}）")
    return doc


def run_stage(stage: str, doc: dict) -> int:
    groups = STAGES[stage]
    failed = 0
    print(f"[{stage}] {STAGE_TITLES[stage]}")
    for group in groups:
        for predicate in group:
            ok, detail = predicate(doc)
            mark = "PASS" if ok else "FAIL"
            print(f"  {mark}  {detail}")
            failed += 0 if ok else 1
    verdict = "通过" if failed == 0 else f"{failed} 条判据未过"
    print(f"  → {stage} {verdict}")
    return 1 if failed else 0


def main(argv: list[str]) -> int:
    if "--list" in argv:
        print("阶段判据速查（详情 SKILL.md 阶段表）：")
        for s in sorted(STAGES):
            print(f"  {s}  {STAGE_TITLES[s]}")
        return 0

    dump_version = "--dump-version" in argv
    args = [a for a in argv if a != "--dump-version"]

    if dump_version:
        if len(args) != 1:
            print("用法: checklist.py --dump-version <reply.json|->", file=sys.stderr)
            return 2
        doc = load_doc(args[0])
        print(doc.get("project_version", "<应答未携带 project_version>"))
        return 0

    if len(args) != 3 or args[0] != "--stage":
        print(__doc__, file=sys.stderr)
        return 2
    _, stage, source = args
    if stage not in STAGES:
        print(f"未知阶段 {stage!r}（合法：{', '.join(sorted(STAGES))}）",
              file=sys.stderr)
        return 2

    try:
        doc = load_doc(source)
    except (OSError, json.JSONDecodeError, ValueError) as e:
        print(f"FAIL  载入应答失败：{e}", file=sys.stderr)
        return 1
    return run_stage(stage, doc)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
