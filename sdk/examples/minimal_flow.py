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
import hashlib
import hmac
import json
import os
import queue
import shutil
import subprocess
import threading
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURES = REPO_ROOT / "fixtures" / "synth-tender" / "招标文件"
# 证据来源件（#185：register_evidence 的 source_file_id 须锚定已导入文件）
EVIDENCE_PDF = REPO_ROOT / "fixtures" / "corpus-A" / "files" / "evidence" / "EV-02-电子与智能化工程专业承包二级资质证书.pdf"
BIN_NAME = "bid-mcp-stdio"

# external 正文模板——{req} 占位符替换为本章节绑定集内的 REQ（首叶绑定
# 5 条，演示引用其一；同一 REQ 多处引用合法，citations 按去重集计数）。
# 两条实测写作纪律（引擎引用回查的执行面，契约 §6）：
#   ① 引用的 REQ/EV 必须在本章节绑定集内（known = depends_on √绑定 EV）——
#      引用其他章节的 REQ 会被当幻觉剔除（citations 归零 + warning）；
#   ② 承诺句（含「承诺/保证/确保/完全满足」）必须**同句**带引用——
#      标题也别用承诺词（无引用的标题句会计 unanchored_claims）。
# 篇幅说明（#325 顺带）：演示稿写足 ~4k 字——行政章节有 1500 字下限
# （W_CHARS_BELOW_MIN），且全稿总量是档位硬指标（E_TOTAL_CHARS_BELOW_MIN），
# 103 字的极简稿会把两条门都踩红（即 #312 篇幅产能墙的示例面）。
EXTERNAL_MANUSCRIPT = (
    "## 售后服务方案\n\n"
    "### 1. 总体响应体系\n\n"
    "我方针对 [{req}] 承诺建立三级售后保障体系：7×24 小时热线响应，"
    "2 小时内远程响应，8 小时内到场处置。质保期内免费提供系统升级与"
    "巡检服务，重要节点提前一周书面报备。\n\n"
    "### 2. 服务组织与驻场安排\n\n"
    "针对 [{req}] 我方组建专职售后团队：项目经理 1 名（持 PMP 资质，"
    "统筹服务计划与月度回访）、系统工程师 2 名（负责平台与终端的日常"
    "维护）、网络工程师 1 名（负责链路巡检与故障定位）。工作日实行"
    "驻场服务（9:00-18:00），非工作时段保持电话与即时通讯双通道待命，"
    "确保 [{req}] 响应路径全程可达。\n\n"
    "驻场团队每月提交《运维月报》，内容包括：系统可用率（目标 ≥99.9%）、"
    "故障清单与处置时长、备件消耗与库存、下月巡检计划。月报经采购人"
    "签收后归档，作为服务考核的原始凭证。\n\n"
    "### 3. 故障分级与响应时限\n\n"
    "我方针对 [{req}] 承诺按四级故障分级响应：\n\n"
    "- **一级（致命）**：平台整体不可用或数据面临丢失风险——15 分钟内"
    "远程接入、2 小时内到场，4 小时内恢复或启用应急回退方案；\n"
    "- **二级（严重）**：核心功能不可用（如终端离线超 30%）——30 分钟内"
    "远程接入、4 小时内到场，8 小时内恢复；\n"
    "- **三级（一般）**：非核心功能异常或性能下降——2 小时内响应，"
    "1 个工作日内修复；\n"
    "- **四级（咨询）**：使用咨询与配置调整——当日响应，2 个工作日内"
    "闭环。\n\n"
    "每起故障均开具《故障处置单》，记录报障时间、定级依据、处置过程、"
    "恢复时间与根因分析，确保 [{req}] 的处置过程可追溯、可复盘。\n\n"
    "### 4. 备品备件保障\n\n"
    "针对 [{req}] 我方在项目现场配置备件柜：常用终端备机不低于在用"
    "设备数的 5%，线材与光模块按 10% 冗余备存；关键部件（主机板卡、"
    "电源模块）与厂商签订 48 小时供货协议。备件出入库实行双人签收，"
    "每季度盘点并向采购人提交《备件库存报告》。\n\n"
    "### 5. 巡检与预防性维护\n\n"
    "我方承诺 [{req}] 实行月度例行巡检：硬件面（终端运行状态、存储"
    "健康度、链路衰耗）、软件面（版本一致性、日志异常扫描、容量趋势）、"
    "环境面（机房温湿度、供电冗余）三张清单逐项核对。巡检发现的隐患"
    "按二级故障时限处置，确保 [{req}] 的隐患不过夜。\n\n"
    "### 6. 培训与知识转移\n\n"
    "质保期内针对 [{req}] 提供三轮培训：①管理员培训（平台配置、账号"
    "与权限管理、日志审计，1 天）；②使用方培训（业务流程操作、常见"
    "问题自助排查，0.5 天）；③应急处置演练（模拟一级故障的全流程"
    "协同，0.5 天）。培训教材与录屏随交付文档一并移交，确保 [{req}] "
    "的操作知识完整沉淀在采购人侧。\n\n"
    "### 7. 服务考核与持续改进\n\n"
    "我方接受采购人按 [{req}] 对服务质量的季度考核：响应及时率"
    "（权重 30%）、恢复及时率（30%）、巡检完成率（20%）、满意度"
    "（20%）。考核低于 85 分的季度，我方提交整改方案并在下一考核周期"
    "复验；连续两个季度不达标，采购人有权按合同约定扣减当季服务费。\n\n"
    "### 8. 质保期满衔接\n\n"
    "质保期届满前 30 天，我方针对 [{req}] 提交《服务总结与移交清单》，"
    "并给出续保方案（含可选服务档位与报价）供采购人决策，确保 [{req}] "
    "的服务衔接不留空窗。"
)


def die(msg: str) -> None:
    print(f"[FAIL] {msg}", file=sys.stderr)
    sys.exit(1)


def mint_receipt(ws: Path, actor_id: str, scope: str) -> str:
    """#182 宿主签发面：读工作区密钥，按域层线格式签审批 receipt。

    示例进程即宿主（spawn server 的同一进程树/同一文件系统权限）——
    持钥签发是宿主特权；MCP 工具面上不存在等价能力（这正是 #182 的
    信任根）。线格式与 bid-domain/src/principal.rs 一字不差（与
    sdk/tests/test_mcp_e2e.py 的 mint_receipt 同源）：
      v1.<actor>.human.<scope>.<issued_at>.<hex_hmac_sha256>
      HMAC 消息 v1|<actor>|human|<scope>|<issued_at>
    """
    key = (ws / ".mang" / "approval.key").read_bytes()
    now = int(time.time())
    msg = f"v1|{actor_id}|human|{scope}|{now}".encode()
    mac = hmac.new(key, msg, hashlib.sha256).hexdigest()
    return f"v1.{actor_id}.human.{scope}.{now}.{mac}"


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
    """stdio 行协议最小客户端（协议层演进后仅本类需适配）。

    #396（C-09）三面修复（对齐 sdk/tests/test_mcp_e2e.py 的成熟实现）：
    - stderr 落文件（server.stderr.log）——不再 PIPE（PIPE 不读，服务端
      stderr 超管道缓冲 64KB 即阻塞写 → hang；真实 LLM 日志面可触发）；
    - readline 超时（后台线程 + queue，env BID_MCP_CALL_SECS 可调，缺省
      300s——与 e2e 同名变量）；
    - project_version 从 data 内取（信封三键 {ok, tool, data}——顶层取值
      恒 None，乐观并发从未生效）。
    """

    CALL_TIMEOUT = float(os.environ.get("BID_MCP_CALL_SECS", "300"))

    def __init__(self, binary: Path, workspace: Path):
        # stderr 落文件：进程退出后仍可回看；die 时回显尾部（诊断面）
        self.stderr_path = workspace.parent / "server.stderr.log"
        self._err = open(self.stderr_path, "wb")
        self.proc = subprocess.Popen(
            [str(binary)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self._err,
            text=True,
            env={
                **os.environ,
                "BID_WS": str(workspace),
                # #183 围栏：受信导入根——示例夹具（fixtures/synth-tender）在
                # 工作区外，服务端按根白名单放行（与 sdk/tests/test_mcp_e2e.py
                # 的 BID_FS_ROOT 同款）。不设则 import_tender_files 报
                # E_UNPROCESSABLE「导入路径越界」。
                "BID_FS_ROOT": str(REPO_ROOT / "fixtures"),
            },
        )
        self.project_version: int | None = None
        # 超时读的队列通道（readline 在后台线程——主线程 join 带超时）
        self._lines: "queue.Queue[str]" = queue.Queue()

        def _pump() -> None:
            assert self.proc.stdout
            for line in self.proc.stdout:
                self._lines.put(line)
            self._lines.put("")  # EOF 哨兵

        self._reader = threading.Thread(target=_pump, daemon=True)
        self._reader.start()

    def _stderr_tail(self, n: int = 2000) -> str:
        try:
            return self.stderr_path.read_bytes()[-n:].decode("utf-8", "replace")
        except OSError:
            return ""

    def call(self, tool: str, args: dict) -> dict:
        payload = dict(args)
        # 乐观并发：带上最近一次应答的 project_version（写面；只读工具不带）
        if self.project_version is not None:
            payload.setdefault("project_version", self.project_version)
        assert self.proc.stdin
        self.proc.stdin.write(json.dumps({"tool": tool, "args": payload}) + "\n")
        self.proc.stdin.flush()
        try:
            line = self._lines.get(timeout=self.CALL_TIMEOUT)
        except queue.Empty:
            self.proc.kill()
            die(f"{tool}: 应答超时（>{self.CALL_TIMEOUT}s，env BID_MCP_CALL_SECS 可调）；"
                f"stderr 尾部: {self._stderr_tail()!r}")
        if not line:
            die(f"{tool}: 应答为空（进程可能已退出）stderr: {self._stderr_tail(500)!r}")
        reply = json.loads(line)
        # #396（C-09）：信封三键——project_version 在 data 内部（顶层取值
        # 恒 None，所有写调用从不带版本——乐观并发在示例里从未生效）
        pv = (reply.get("data") or {}).get("project_version")
        if isinstance(pv, int):
            self.project_version = pv
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
        try:
            self.proc.wait(timeout=30)
        finally:
            self._err.close()  # stderr 落文件句柄（#396）


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
        # 篇幅目标取 Standard 档 800,000（#287 四档：400k/800k/1.2M/2.5M）。
        # 本示例 mock 补齐面 ≈843k 字符：贴 844k 阈值会被 E_TOTAL_CHARS_BELOW_MIN
        # 卡死（实测 843,014 < 844,000，差 0.1%——校准随上游计数口径漂移即翻车，
        # 即 #312 篇幅产能墙的示例面）。演示链路的意义在「跑通」不在「达标」，
        # 选 800k 档留 ~5% 余量；真实项目按 recommend_length_profile 推荐 + 用户
        # 确认选档（#287 流程，见 #288 Skill 面）。
        bo = mcp.must("build_outline", {
            "project_id": pid, "target_total_chars": 800_000,
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

        # ---- 人工门（三门前置：matrix/outline + addendum_waiver；#325/R-14：
        #      旧 reviewer 参数已废除并被参数面显式拒——契约 §12「旧 reviewer
        #      参数显式拒——Agent 不得自述身份」。现行 #182 receipt 验签：
        #      宿主持 approval.key 签发 receipt，本示例进程即宿主（与 e2e
        #      测试同款 mint_receipt）。
        #      pre_export_confirm 不在此处记：#180 审核内容绑定——该门绑定
        #      全部章节正文摘要，必须在起草完成且校验通过后记（见下方
        #      P6 之后），先过门后起草会被导出前置裁决拒（摘要失配））
        for gate in ("matrix_approval", "outline_approval",
                     "addendum_waiver"):
            mcp.must("record_review_gate", {
                "project_id": pid, "gate": gate, "decision": "APPROVED",
                "receipt": mint_receipt(ws, "minimal-flow-操作者",
                                        f"review_gate:{gate}"),
            })
        print("      三门已记（matrix/outline + addendum_waiver；receipt 验签）")

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

        # ---- P6 校验 + P7.5 终审门 + P8 导出
        v = mcp.must("validate_bid_document", {"project_id": pid, "scope": "all"})
        if v["counts"]["blocker"] != 0:
            die(f"BLOCKER={v['counts']['blocker']}：{v['findings'][:2]}")
        # P7.5 pre_export_confirm（#180 审核内容绑定：绑定全部章节正文摘要，
        # 起草完成且校验通过后才记——与人类流程同序，e2e 的 _case_full_pipeline
        # 同款；先记门后改稿会被导出前置裁决拒「审核针对的内容已变更」）
        mcp.must("record_review_gate", {
            "project_id": pid, "gate": "pre_export_confirm",
            "decision": "APPROVED",
            "receipt": mint_receipt(ws, "minimal-flow-操作者",
                                    "review_gate:pre_export_confirm"),
        })
        print("      P6 校验 BLOCKER=0 ✓ → P7.5 pre_export_confirm 已记（receipt 验签）")
        e = mcp.must("export_bid_document", {
            "project_id": pid, "format": "docx",
            "output_path": "out/minimal-flow-投标文件.docx",
        })
        art = e["artifact"]
        print(f"[9/9] P8 导出：{art['url']}"
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
