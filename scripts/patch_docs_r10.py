#!/usr/bin/env python3
"""verification-plan / test-plan / README / README 状态回填（M1.7-TEL 第 10 轮）。"""
import io
import sys

# FG-16（#305）：仓外绝对路径参数化——默认仓根（research/ 文档在仓内），DOCS_BASE 可覆盖
import os as _os, pathlib as _pl
BASE = _os.environ.get("DOCS_BASE") or str(_pl.Path(__file__).resolve().parent.parent)

# ---------------- verification-plan ----------------
p = f"{BASE}/research/verification-plan.md"
s = io.open(p, encoding="utf-8").read()

m17tel_row = (
    "| M1.7-TEL | 通信行业真实链（S2 行业专项扩充·txzbqy 工信部法定平台） | 运营商/铁塔真实公告+招标文件：四节点链（含失败→重招）+ 文件级解析 + GT 草案 | §3.5 + real-corpus/telecom + research/s2-scenario-plan.md §五 | **已执行（2026-10-26，T16 全绿）：四节点链 46/46（招标→失败→重招→公示）；文件级 Rust 亲跑 17/17 真实文件 + 2040 条款 + 公告↔文件跨向锚点 14/14；GT 18 条四要素待人工冻结。文件级缺口在通信场景已实质突破（txzbqy 免鉴权文件通道）；ccgp（政采）附件仍 WAF 待人工** |\n"
)
anchor = "| S2 | 采购规则与业务边界"
assert anchor in s, "S2 行锚点缺失"
s = s.replace(anchor, m17tel_row + anchor, 1)

sec35 = """
### 3.5 M1.7-TEL：通信行业真实链（行业专项扩充 · 2026-10-26 执行）

用户指令「招标文件自己找通信行业的」驱动。主平台：**工信部通信工程建设项目招标投标管理信息平台**
（txzbqy.miit.gov.cn，通信行业法定公开媒介），免鉴权五件套 API 亲测打通（门端列表→详情→
文件元数据→令牌→下载，平台 md5 逐字节校验）。覆盖移动/电信/联通/铁塔。

- **语料**：20 节点（招标/变更 10 + 中标公示 10）+ 16 文件（14 md5 校验 OK；2 MISMATCH 同尺寸
  不同内容——指纹双记录纪律实证）+ 8 zip 解压 + 4 LO 旧格式转换（.doc/.xls，sha256 双留痕）。
- **四节点链**（HX-BFZB-2026035，46/46 断言，scripts/telecom_chain.py）：
  招标公告（08-12）→ **招标失败公告（09-08，有效投标人不足3家 + 通信招标办法第三十四条；
  快照证据 SNAPSHOT_ONLY 诚实登记）** → 第二次招标公告（09-18）→ 中标候选人公示（10-26，
  txzbqy 官方）。真实 diff：MODIFY 恰 3（标题轮次/获取窗口/截止 09-03→10-14）+ 不变陷阱 4
  （预算/双档限价/业绩门槛/履约保证金）。输出复验：公示自证时间线与第二次公告截止时间
  **跨文件逐字吻合**；报价双档<限价双档；承诺保修 36≥24（承诺优于要求非冲突）；
  结构化实体为空→公示数据全在正文（正文解析必要性实证）。
- **文件级**（scripts/telecom_file_level.py + bid-parse examples/realfiles.rs，ENGINE_RUN）：
  17 真实文件解析 17/17 + 条款抽取 2040 条（资格/否决/评分非零）+ 公告↔文件跨向锚点 14/14
  （招标编号 2026CUC-RJY-PT0783 / ZJYD 77台·1674.336万·20工作日 / 四项目关键词贯通）。
- **GT 草案**：18 条四要素（17 verbatim/1 normalized，0 定位 FAIL），draft_pending_human。
- **真实数据质量发现（入 GT-T17/T18）**：①平台登记 md5 与分发内容可不一致（同尺寸不同内容）
  ——证据链必须双记录实测哈希；②tenderFileId 字段语义漂移（实为立项批复）——字段名不可单信；
  ③真实电信语料 .doc/.xls/.rar 旧格式占比高——解析器格式面需 LO 桥接（边界留痕）。
- 边界：ccgp 附件通道仍 WAF（政采文件级维持人工事项）；b2b.10086.cn TLS 旧协商、
  联通官网 412、bidcenter/jianyu360 JS 站——失败公告字节级原件须人工从法定媒介获取。

"""
anchor2 = "### 3.4 M1.7：政府采购真实链（真实数据层 · S2 批次一 · 2026-10-26 执行）"
i = s.find(anchor2)
assert i > 0
k = s.find("\n## 4.", i)
assert k > i
s = s[:k] + "\n" + sec35 + s[k:]

old_tail = "**待办**：招标文件本体人工获取后文件级复验；S2 批次二（工程招投标：工程量清单/复杂表格/资质与技术响应）；T-00 存量测试审计；EXP-1/S3（环境）。"
new_tail = ("**第 10 轮（2026-10-26）**：用户指令「通信行业自己找」→ txzbqy 平台五件套免鉴权通道打通，"
            "M1.7-TEL 三批次达成（§3.5）：四节点链 46/46 + 文件级 Rust 亲跑 17/17 + 2040 条款 + "
            "跨向锚点 14/14 + GT 18 条；run_tests 54/54。**待办**：政采招标文件本体人工获取后文件级复验"
            "（GT-M17-24）；电信失败公告字节级原件人工获取；S2 批次二（工程招投标）；T-00 存量测试审计；"
            "EXP-1/S3（环境）。")
assert old_tail in s, "待办锚点缺失"
s = s.replace(old_tail, new_tail, 1)
io.open(p, "w", encoding="utf-8").write(s)
print("verification-plan OK")

# ---------------- test-plan ----------------
p = f"{BASE}/research/test-plan.md"
s = io.open(p, encoding="utf-8").read()
if "T16" not in s:
    old = "| TC-09 |"
    assert old in s
    add = ("| TC-16 | M1.7-TEL 通信行业真实链回归（txzbqy 四节点 46 断言 + 文件级 17/17 + GT 18 条四要素；"
           "run_tests T16a-d；防新平台/新格式/新样本绕过；快照节点 SNAPSHOT_ONLY 诚实口径） |\n")
    i = s.find(old)
    s = s[:i] + add + s[i:]
    io.open(p, "w", encoding="utf-8").write(s)
print("test-plan OK")

# ---------------- README ----------------
p = f"{BASE}/README.md"
s = io.open(p, encoding="utf-8").read()
rep = [
    ("run_tests 50/50", "run_tests 54/54"),
    ("50 项全绿", "54 项全绿"),
]
for a, b in rep:
    s = s.replace(a, b)
if "M1.7-TEL" not in s:
    anchor = "real-corpus/gov-procurement/"
    if anchor in s:
        s = s.replace(anchor,
                      "real-corpus/telecom/（通信行业：txzbqy 20 节点+16 文件+四节点链 46/46+文件级 17/17+GT 18 条）\n  - real-corpus/gov-procurement/", 1)
io.open(p, "w", encoding="utf-8").write(s)
print("README OK")
