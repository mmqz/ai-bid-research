#!/usr/bin/env python3
"""minimal_flow —— 外部 Agent 纯 MCP 驱动最小全链（可运行示例）。

链路（docs/agent-integration.md §5 的可执行版）：
  create → import(A+B) → extract → build_outline → register/bind
  → 四门（matrix/outline/pre_export + addendum_waiver）
  → check_bid_quality（章节发现）→ **external 起草首叶**（本文重点）
  → mock 补齐余叶 → validate → export(docx)

external 模式：正文由本脚本撰写并经 generator="external" + content 直注——
引擎不生成只提交（引用回查/CAS/落盘/校验与其他生成器同路径）。

用法（仓根执行）：
  python3 sdk/examples/minimal_flow.py                     # 自动 cargo build
  python3 sdk/examples/minimal_flow.py --bin <bid-mcp-stdio 路径>
  python3 sdk/examples/minimal_flow.py --keep              # 保留工作区现场

依赖：纯标准库。需要 rustc/cargo（构建 bidcore）。夹具 =
fixtures/synth-tender/招标文件/（仓内自带）。产物留在临时工作区
（--keep 时打印路径不清理）。

协议说明：当前 MCP 面是 stdio 行协议（逐行 {"tool","args"} JSON → 逐行应答，
见 bidcore/crates/bid-mcp/src/bin/stdio.rs）。标准 MCP JSON-RPC 传输层接线
后本脚本的 transport 函数是唯一需要适配的点（工具语义面不变）。
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURES = REPO_ROOT / "fixtures" / "synth-tender" / "招标文件"
# 证据来源件（#185：register_evidence 的 source_file_id 须锚定已导入文件）
EVIDENCE_PDF = REPO_ROOT / "fixtures" / "corpus-A" / "files" / "evidence" / "EV-02-电子与智能化工程专业承包二级资质证书.pdf"
BIN_NAME = "bid-mcp-stdio"

# external 正文模板——{req} 占位符替换为**本章节绑定集内**的 REQ。
# 两条实测写作纪律（引擎引用回查的执行面，契约 §6）：
#   ① 引用的 REQ/EV 必须在本章节绑定集内（known = depends_on √绑定 EV）——
#      引用其他章节的 REQ 会被当幻觉剔除（citations 归零 + warning）；
#   ② 承诺句（含「承诺/保证/确保/完全满足」）必须**同句**带引用——
#      标题也别用承诺词（无引用的标题句会计 unanchored_claims）。
EXTERNAL_MANUSCRIPT = (
    "## 售后服务方案\n\n"
    "我方针对 [{req}] 承诺建立三级售后保障体系：7×24 小时热线响应，"
    "2 小时内远程响应，8 小时内到场处置。质保期内免费提供系统升级与"
    "巡检服务，重要节点提前一周书面报备。"
)


def die(msg: str) -> None:
    print(f"[FAIL] {msg}", file=sys.stderr)
    sys.exit(1)


def build_binary() -> Path:
    cargo = shutil.which("cargo")
    if cargo is None:
        die("找不到 cargo——请先安装 Rust 工具链（docs/agent-integration.md §2）")
    print("[1/9] 构建 bid-mcp-stdio（首次约数分钟）…")
    r = subprocess.run(
        [cargo, "build", "--manifest-path", str(REPO_ROOT / "bidcore" / "Cargo.toml"),
         "-p", "bid-mcp", "--bin", BIN_NAME],
        cwd=REPO_ROOT,
    )
    if r.returncode != 0:
        die("cargo build 失败——见上方编译输出")
    target_dir = REPO_ROOT / "bidcore" / "target" / "debug" / BIN_NAME
    if not target_dir.exists():
        die(f"构建成功但未找到二进制：{target_dir}")
    return target_dir


class McpStdio:
    """stdio 行协议最小客户端（协议层演进后仅本类需适配）。"""

    def __init__(self, binary: Path, workspace: Path):
        self.proc = subprocess.Popen(
            [str(binary)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env={**os.environ, "BID_WS": str(workspace)},
        )
        self.project_version: int | None = None

    def call(self, tool: str, args: dict) -> dict:
        payload = dict(args)
        # 乐观并发：带上最近一次应答的 project_version（写面；只读工具不带）
        if self.project_version is not None:
            payload.setdefault("project_version", self.project_version)
        assert self.proc.stdin and self.proc.stdout
        self.proc.stdin.write(json.dumps({"tool": tool, "args": payload}) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            err = self.proc.stderr.read() if self.proc.stderr else ""
            die(f"{tool}: 应答为空（进程可能已退出）stderr: {err[:500]}")
        reply = json.loads(line)
        if reply.get("project_version") is not None:
            self.project_version = reply["project_version"]
        return reply

    def must(self, tool: str, args: dict) -> dict:
        reply = self.call(tool, args)
        if not reply.get("ok"):
            err = reply.get("error", {})
            die(f"{tool} 失败 [{err.get('code')}] {err.get('what')}")
        return reply["data"]

    def close(self) -> None:
        if self.proc.stdin:
            self.proc.stdin.close()
        self.proc.wait(timeout=30)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bin", type=Path, help=f"{BIN_NAME} 二进制路径（缺省自动构建）")
    ap.add_argument("--keep", action="store_true", help="保留工作区现场（不清理）")
    opt = ap.parse_args()

    binary = opt.bin if opt.bin else build_binary()
    ws = Path(tempfile.mkdtemp(prefix="minimal-flow-"))
    print(f"[2/9] 工作区：{ws}（BID_WS 持久化——db.json + checkpoint）")

    mcp = McpStdio(binary, ws)
    try:
        # ---- P0 建项
        print("[3/9] P0 create_bid_project …")
        proj = mcp.must("create_bid_project", {
            "name": "minimal-flow 演示项目——智慧病房物联网平台",
            "bid_deadline": "2026-11-30", "mode": "明标",
            "external_key": f"minimal-flow-{os.getpid()}",
        })
        pid = proj["project_id"]
        print(f"      project_id = {pid}（status={proj['status']}）")

        # ---- P1 导入（A 招标文件 + B 技术规范书 + EV-02 证据件[附件]）
        print("[4/9] P1 import_tender_files …")
        imp = mcp.must("import_tender_files", {"project_id": pid, "files": [
            {"upload_ref": str(FIXTURES / "A-招标文件.docx"),
             "filename": "A-招标文件.docx", "kind": "招标文件"},
            {"upload_ref": str(FIXTURES / "B-技术规范书.docx"),
             "filename": "B-技术规范书.docx", "kind": "招标文件"},
            # #185 来源验证：证据件同批导入（附件）——登记时以 file_id 引用
            {"upload_ref": str(EVIDENCE_PDF),
             "filename": EVIDENCE_PDF.name, "kind": "附件"},
        ]})
        totals = imp["totals"]
        ev_fid = next(
            (p["file_id"] for p in imp["per_file"] if p["filename"] == EVIDENCE_PDF.name),
            "",
        )
        if not ev_fid:
            die("导入回执缺证据件 file_id（#185 来源锚）")
        print(f"      导入 {totals['files']} 件 / {totals['chunks']} 块 / "
              f"失败 {totals['parse_failed']}（复述并比对预期：零失败 ✓）")
        if totals["parse_failed"] != 0:
            die("parse_failed != 0——先处理失败件再继续")

        # ---- P2 抽条款
        ext = mcp.must("extract_requirements", {"project_id": pid})
        by_cat = ext["counts"]["by_category"]
        print(f"[5/9] P2 extract_requirements：共 {ext['counts']['total']} 条"
              f"（资格 {by_cat['qualification']} / 否决 {by_cat['veto']} / "
              f"技术参数 {by_cat['tech_param']}）")
        if ext["missing_sequence"]:
            die(f"缺号未解释：{ext['missing_sequence'][:3]}…")

        # ---- P3 建纲 + 矩阵
        bo = mcp.must("build_outline", {
            "project_id": pid, "target_total_chars": 20_000,
            "max_reqs_per_leaf": 8,
        })
        print(f"[6/9] P3 build_outline：{bo['counts']['leaves']} 叶 / "
              f"matrix_blocking={bo['counts']['matrix_blocking']}"
              f"（状态 {bo['status']}）")

        # ---- P4 证据三板斧：矩阵翻页收资格+否决+技术参数 REQ → 登记一条 EV → 全绑
        bind_ids: list[str] = []
        for category in ("qualification", "veto", "tech_param"):
            cursor = None
            while True:
                args = {"project_id": pid, "filters": {"category": category},
                        "limit": 100}
                if cursor:
                    args["cursor"] = cursor
                rows = mcp.must("get_compliance_matrix", args)
                bind_ids += [r["requirement"]["id"] for r in rows["rows"]]
                cursor = rows.get("next_cursor")
                if not cursor:
                    break
        if not bind_ids:
            die("待绑 REQ 为空——夹具异常")
        reg = mcp.must("register_evidence", {
            "project_id": pid, "kind": "qualification",
            "title": "电子与智能化工程专业承包二级资质证书（minimal-flow 演示）",
            # #185：来源锚已导入的 EV-02 件（sha/指纹由服务端生成）
            "source_file_id": ev_fid,
            "subject_name": "投标人", "scope": "电子与智能化工程",
        })
        ev_id = reg["ev_id"]
        print(f"[7/9] P4 证据：登记 {ev_id}（deduplicated={reg['deduplicated']}）"
              f" → 绑 {len(bind_ids)} 条 REQ …")
        bind = mcp.must("bind_evidence", {
            "project_id": pid, "ev_id": ev_id,
            "requirement_ids": bind_ids, "bid_open_date": "2026-11-30",
        })
        if bind["matrix"]["blocking"] != 0:
            die(f"绑定后 blocking={bind['matrix']['blocking']}（应为 0）")
        print(f"      bound_new={bind['bound_new']}，blocking=0 ✓")

        # ---- 人工门（四门；reviewer 必须 human: 前缀）
        for gate in ("matrix_approval", "outline_approval",
                     "pre_export_confirm", "addendum_waiver"):
            mcp.must("record_review_gate", {
                "project_id": pid, "gate": gate, "decision": "APPROVED",
                "reviewer": "human:minimal-flow-操作者",
            })
        print("      四门已记（matrix/outline/pre_export + addendum_waiver）")

        # ---- 章节发现（叶清单的正式查询口；requirement_ids = 章节→REQ
        #      归属的正式查询口——external 正文引用必须落在该集合内）
        q = mcp.must("check_bid_quality", {"project_id": pid})
        secs = [s["section_id"] for s in q["sections"]]
        first_reqs = q["sections"][0].get("requirement_ids", []) if secs else []
        if not secs:
            die("章节清单为空")
        if not first_reqs:
            die("首叶绑定 REQ 为空——external 正文无锚可引，先回 P4 绑定")
        print(f"[8/9] P5 章节发现：{len(secs)} 个可起草叶（首叶绑定 "
              f"{len(first_reqs)} 条 REQ）")

        # ---- external 起草首叶（本文重点）+ mock 补齐余叶
        first, rest = secs[0], secs[1:]
        manuscript = EXTERNAL_MANUSCRIPT.format(req=first_reqs[0])
        d = mcp.must("draft_response_section", {
            "project_id": pid, "section_id": first,
            "instructions": "按招标文件逐项响应，突出同类业绩与实施方案",
            "generator": "external", "content": manuscript,
        })
        stats = d["stats"]
        print(f"      external 起草 {first}：generator={stats['generator']} / "
              f"citations={stats['citations']} / "
              f"unanchored_claims={stats['unanchored_claims']} ✓")
        if stats["generator"] != "external" or stats["unanchored_claims"] != 0:
            die("external 起草语义异常")
        # save_version 的 file_path 必须指向【已存在】的文本文件（契约 §SV：
        # 文件不存在 → E_NOTFOUND）——先把 external 稿落盘，再存版本快照。
        draft_dir = ws / "drafts"
        draft_dir.mkdir(parents=True, exist_ok=True)
        draft_path = draft_dir / f"{first}.md"
        draft_path.write_text(manuscript, encoding="utf-8")
        mcp.must("save_version", {
            "file_path": str(draft_path),
            "message": f"{first}：external 稿（引用 {first_reqs[0]}，三级售后体系）",
        })
        for sid in rest:  # 演示链只 external 首叶；余叶 mock 补齐（空章节是 BLOCKER）
            mcp.must("draft_response_section", {
                "project_id": pid, "section_id": sid, "generator": "mock",
            })
        print(f"      mock 补齐余 {len(rest)} 叶（空章节是 P6 BLOCKER——"
              f"真实链路中每叶都由你的 Agent external 撰写）")

        # ---- P6 校验 + P8 导出
        v = mcp.must("validate_bid_document", {"project_id": pid, "scope": "all"})
        if v["counts"]["blocker"] != 0:
            die(f"BLOCKER={v['counts']['blocker']}：{v['findings'][:2]}")
        e = mcp.must("export_bid_document", {
            "project_id": pid, "format": "docx",
            "output_path": "out/minimal-flow-投标文件.docx",
        })
        art = e["artifact"]
        print(f"[9/9] P6 校验 BLOCKER=0 ✓ → P8 导出：{art['url']} "
              f"（{art['size']} bytes）\n      sha256={art['sha256']}"
              f"（复述给用户）\n      六表 {e['tables_written']} 张 / "
              f"七件 {e['forms_written']} 份在册")
    finally:
        mcp.close()
        if opt.keep:
            print(f"\n工作区保留：{ws}（db.json / drafts / out）")
        else:
            shutil.rmtree(ws, ignore_errors=True)

    print("\nminimal-flow 全链通过 ✅  external 模式语义与 agent-integration.md §4 一致")


if __name__ == "__main__":
    main()
