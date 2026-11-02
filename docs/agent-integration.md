# Agent 接入指南（agent-integration）

> 给使用方 Agent 框架的接入文档。你的 Agent 有自己的模型（免费额度），
> **正文由你的 Agent 写**；本系统提供确定性引擎——引用回查、CAS 落盘、
> 校验、导出。你的 Agent 通过 MCP 调 bid-mcp 工具，按 SKILL.md 阶段表驱动。
>
> 配套件：`sdk/skill/bid-response-pipeline/`（SKILL.md + 阶段 references +
> 质检脚本）；可运行示例 `sdk/examples/minimal_flow.py`。

## 1. 架构

```text
┌─────────────────────────┐        ┌──────────────────────────────┐
│  你的 Agent 框架         │        │  ai-bid-research（本系统）    │
│                         │        │                              │
│  · 加载 SKILL.md        │  MCP   │  bid-mcp（20 个工具的薄壳）   │
│    （阶段表+纪律+反模式） ├───────►│    ├ 参数校验/状态机裁决      │
│  · 按阶段表编排          │ stdio  │    ├ 引用回查（幻觉拦截）     │
│  · 写各章正文            │ JSON   │    ├ CAS 落盘（乐观并发）     │
│    draft_response_      │        │    ├ 确定性校验（BLOCKER 门） │
│    section(             │        │    └ 导出（docx/pdf + sha256）│
│      generator=external,│        │                              │
│      content=正文)      │        │  状态机/证据台账/矩阵/版本快照  │
└─────────────────────────┘        └──────────────────────────────┘
```

职责分界（谁做什么）：

| 事项 | 你的 Agent | 本系统引擎 |
|---|---|---|
| 阶段推进编排、向用户复述数字 | ✅ | |
| 章节正文撰写 | ✅（external 模式提交） | |
| 文件解析/切块/条款抽取 | | ✅ 确定性管线 |
| 矩阵/大纲推导、状态机裁决 | | ✅ |
| 引用回查（正文里的 REQ/EV 键是否真实） | | ✅ |
| CAS 落盘、幂等、断点恢复 | | ✅ |
| 校验（确定性规则+模型检查）与导出 | | ✅ |

## 2. 启动与连接

```bash
# 构建并启动 MCP server（stdio 传输）
cd ai-bid-research/bidcore
BID_WS=/path/to/workspace cargo run -p bid-mcp --bin bid-mcp-stdio
```

- 协议形态：**逐行 JSON** ——每行发 `{"tool": "<工具名>", "args": {…}}`，
  收一行应答 `{ok, tool, data, request_id, project_version?, warnings?}`；
  空行忽略；EOF 退出。
- `BID_WS`（工作区目录）：设置后进程级服务态**持久化**（db.json 恢复 + 每次写
  面 checkpoint 落盘）；不设 = 纯内存态（进程退出即丢——只适合冒烟）。
  工作区损坏/不可写 → 启动即 panic（不静默降级）。
- 工具名单一真源 `TOOL_NAMES`（20 个）与契约：`docs/bid-mcp-contract.md`；
  未接线工具（如 `search_evidence`）返回 typed `UNIMPLEMENTED_INTERNAL`，
  **绝不静默成功**。

> 协议层演进：当前是 stdio 行协议（最简接线验证形态）；标准 MCP JSON-RPC
> 传输层（initialize / tools/list 握手）由协议层任务接线后本节更新——工具
> 语义面不变。

## 3. 配置

### 3.1 LLM（仅 auto/real 生成器需要——external 模式不需要）

你的 Agent 用 external 模式提交正文时，**引擎不生成任何文本，无需配置
LLM**。仅当宿主想让引擎自己生成（auto/real）才配置：

`<workspace>/.mang/llm.json`（env `BID_LLM_API_BASE` / `BID_LLM_API_KEY` /
`BID_LLM_MODEL` 覆盖文件）：

```json
{
  "api_base": "https://api.openai.com/v1",
  "api_key": "sk-…",
  "model": "…",
  "timeout_secs": 60,
  "temperature": 0.3,
  "max_tokens_cap": 8192
}
```

全字段可选；`api_key` 在场时 `model` 必填；api_key 绝不落日志/台账（用量落
`.mang/llm-usage.json`）。

### 3.2 断点恢复与版本

- 会话开始（或用户说「继续」）先调 `get_bid_project_state`，按 SKILL.md
  恢复协议对齐断点；
- 每个写工具响应携带 `project_version`（调用完成后的最新值）——把它传给
  下一个写工具（乐观并发）；收到 `E_CONFLICT` → 重读状态对齐重试；
- 每完成一个章节调 `save_version`（file_path 绝对路径 + message 写明章节与
  完成内容）。

## 4. external 模式（外部 Agent 写正文——本文重点）

`draft_response_section` 的第四生成器：**正文来自 `content` 参数**（你的
Agent 写好直注，免中继）；引擎不生成只提交——引用回查 / CAS / 落盘 / 校验
与 mock/real 完全同路径。

```json
{
  "project_id": "PRJ-…",
  "section_id": "SEC-0007",
  "instructions": "按招标文件逐项响应，突出同类业绩与实施方案",
  "generator": "external",
  "content": "## 售后服务承诺\n\n我方针对 [REQ-0001] 建立三级售后保障体系：7×24 小时热线响应，2 小时内远程响应，8 小时内到场处置。质保期内免费提供系统升级与巡检服务。",
  "project_version": 42
}
```

语义细则（实测行为，勿猜）：

| 规则 | 行为 |
|---|---|
| `content` 在场即 external | `generator` 可省（省略 + content = external 语义） |
| 双生成源互斥 | `content` + `generator="mock"/"real"` → `E_UNPROCESSABLE` 响亮拒 |
| 无手稿的 external | `generator="external"` 但无 `content` → `E_UNPROCESSABLE` |
| 空白稿 | `content` 全空白 → `E_UNPROCESSABLE`（空稿无提交意义） |
| 幂等指纹 | 幂等键 = (section_id, instructions_hash, content_version) + **content 指纹**——同章节同指令不同稿 = 不同请求（改稿重提不误重放）；同稿重提 → `deduplicated: true` 直返首次结果 |
| 正文逐字落位 | 引擎不加工（无模板痕迹）；`stats.generator` 回显 `"external"` |
| 引用回查看跑 | 正文里的 `[REQ-…]`/`[EV-…]` 引用被登记——但仅限**本章节绑定集内**（known = depends_on √绑定 EV；引用他章 REQ 被当幻觉剔除）；承诺句无同句引用 → `unanchored_claims > 0` + warning |
| 感知告警 | 返回 warnings 携带 external 模式说明（不生成只提交） |

**external 正文写作纪律**（SKILL.md 全局纪律在正文的落点；①②为实测执行面——
引擎引用回查算法决定，进 agent-integration 验证链路）：

1. **引用必须落在本章节绑定集内**：known 集 = 章节 depends_on 的 REQ √
   绑定的 EV——引用其他章节的 REQ 会被当**幻觉剔除**（citations 归零 +
   warning，不拒稿）。章节绑定集的正式查询口 = `check_bid_quality` 的
   `sections[].requirement_ids`；
2. **承诺句同句带引用**：含「承诺/保证/确保/完全满足」的句子若无同句
   REQ/EV 引用 → `unanchored_claims +1`（标题也别用承诺词——标题句无引用
   同样计数）；
3. 逐槽位响应 `source_requirements` 对应 REQ（技术参数值不得编造）；
4. md 块语法可用：连续 `|` 行 = 表格（首行表头 + `|---|` 分隔行）、整行
  `![说明](路径或EV-id)` = 内嵌图、`#`/`##` = 标题行（导出时剥号加粗）；
5. 不残留 TODO/待确认/占位公司名（W_PLACEHOLDER）；暗标项目正文禁公司名。

## 5. 完整对话示例（外部 Agent 纯 MCP 驱动全链）

> 素材 = `bidcore/crates/bid-mcp/tests/mcp_scaffold_tools.rs`（真实测试的
> 调用序列）。输入 = 仓内夹具 `fixtures/synth-tender/招标文件/`（A 招标
> 文件 + B 技术规范书）。`→` 为 MCP 应答关键字段（节选）。

**P0 建项**

```json
{"tool": "create_bid_project", "args": {
  "name": "智慧病房物联网平台投标", "bid_deadline": "2026-11-30",
  "mode": "明标", "external_key": "my-agent-run-001" }}
```
```json
→ {"ok": true, "data": {"project_id": "PRJ-…", "status": "DRAFT"},
   "project_version": 1}
```

（Agent 复述用户：项目已建，截止 2026-11-30 未过，明标。）

**P1 导入**

```json
{"tool": "import_tender_files", "args": {"project_id": "PRJ-…", "files": [
  {"upload_ref": "fixtures/synth-tender/招标文件/A-招标文件.docx",
   "filename": "A-招标文件.docx", "kind": "招标文件"},
  {"upload_ref": "fixtures/synth-tender/招标文件/B-技术规范书.docx",
   "filename": "B-技术规范书.docx", "kind": "招标文件"}]}}
```
```json
→ {"ok": true, "data": {"file_ids": ["TF-…", "TF-…"],
   "totals": {"files": 2, "chunks": 214, "parse_failed": 0, "partial": 0}},
   "project_version": 5}
```

（复述：共导入 2 件 / 214 块 / 零失败——数字与预期比对。）

**P2 抽条款**

```json
{"tool": "extract_requirements", "args": {"project_id": "PRJ-…"}}
```
```json
→ {"ok": true, "data": {"counts": {"total": 108,
   "by_category": {"qualification": 9, "veto": 7, "scoring": 25,
                   "tech_param": 31, "commercial": 12, "format": 8, "other": 16}},
   "conflicts": [], "missing_sequence": []}, "project_version": 12}
```

（复述七类分布；conflicts/missing_sequence 空即过，非空走确认流程。）

**P3 建纲+矩阵**

```json
{"tool": "build_outline", "args": {"project_id": "PRJ-…",
  "target_total_chars": 20000, "max_reqs_per_leaf": 8}}
```
```json
→ {"ok": true, "data": {"counts": {"sections": 18, "leaves": 9,
   "requirements": 108, "matrix_rows": 108, "matrix_blocking": 47},
   "status": "MATRIX_READY"}, "project_version": 14}
```

（matrix_blocking=47：资格+否决+技术参数行还没绑证据——正是 P4 的活。
矩阵资格三态经 `get_compliance_matrix` 逐页读 filters.category=
"qualification" 判断并向用户报告。）

**P4 登记+绑定证据**

```json
{"tool": "register_evidence", "args": {"project_id": "PRJ-…",
  "kind": "qualification",
  "title": "电子与智能化工程专业承包二级资质证书",
  "source_file": "SYN-EV-资质证书.pdf", "source_sha256": "sha-…-001",
  "subject_name": "投标人", "scope": "电子与智能化工程"}}
```
```json
→ {"ok": true, "data": {"ev_id": "EV-0007", "deduplicated": false,
   "status": "ACTIVE"}, "project_version": 15}
```
```json
{"tool": "bind_evidence", "args": {"project_id": "PRJ-…", "ev_id": "EV-0007",
  "requirement_ids": ["REQ-0001", "…（资格+否决+技术参数行全集）"],
  "bid_open_date": "2026-11-30", "project_version": 15}}
```
```json
→ {"ok": true, "data": {"ev_id": "EV-0007", "bound_new": 47,
   "matrix": {"rows": 108, "blocking": 0}}, "project_version": 16}
```

（blocking 清零 ✓。EXPIRED 证据禁绑——先换证再补料，见 references/p4。）

**人工门（四门）**

```json
{"tool": "record_review_gate", "args": {"project_id": "PRJ-…",
  "gate": "matrix_approval", "decision": "APPROVED",
  "receipt": "v1.张工.human.review_gate:matrix_approval.1790000000.6a3f…"}}
```
```json
→ {"ok": true, "data": {"gates_approved": ["matrix_approval"],
  "actor": {"actor_id": "张工", "actor_type": "human",
            "authentication_method": "hmac_receipt_v1"}}, …}
```

（#182：`receipt` 是宿主签发的审批回执——真人在宿主界面确认后由宿主用
本地密钥签发（`<工作区>/.mang/approval.key`），Agent 拿不到密钥、传旧
`reviewer` 参数会被显式拒绝——AI 操作者不得自审自过。同法记
`outline_approval` → `pre_export_confirm`；单件招标无补遗时另记
`addendum_waiver`，否则 draft 会被 MissingAddendum 阻塞。）

**章节发现（P5 前置）**

```json
{"tool": "check_bid_quality", "args": {"project_id": "PRJ-…"}}
```
```json
→ {"ok": true, "data": {"sections": [
   {"section_id": "SEC-0003", "title": "售后服务承诺", "status": "PENDING", …},
   {"section_id": "SEC-0004", …}, …]}, …}
```

（**sections 只列可起草叶**——这是外部 Agent 拿章节清单的正式查询口；
`get_bid_project_state` 不回章节清单。）

**P5 起草（external 模式——你的 Agent 写正文）**

（首叶绑定集先查：`check_bid_quality` 的 `sections[0].requirement_ids` ——
本例首叶绑定 6 条 REQ，正文引用其中的 REQ-0031。）

```json
{"tool": "draft_response_section", "args": {
  "project_id": "PRJ-…", "section_id": "SEC-0005",
  "instructions": "按招标文件逐项响应，突出同类业绩与实施方案",
  "generator": "external",
  "content": "## 售后服务方案\n\n我方针对 [REQ-0031] 承诺建立三级售后保障体系：7×24 小时热线响应，2 小时内远程响应，8 小时内到场处置。质保期内免费提供系统升级与巡检服务，重要节点提前一周书面报备。",
  "project_version": 20}}
```
```json
→ {"ok": true, "data": {"section_id": "SEC-0005", "content_ref": "DOC-…",
   "stats": {"words": 78, "citations": 1, "unanchored_claims": 0,
             "generator": "external", "uncovered_requirements": ["REQ-0032", …]},
   "section_status": "DRAFTED", "warnings": ["…external 模式…（不生成只提交）…",
   "覆盖度：REQ-0032 未被本章正文回应——人工补写或重起草", …]},
   "project_version": 21}
```

（`citations=1` ✓（REQ-0031 在本叶绑定集内）；`unanchored_claims=0` ✓（承诺句
同句带引用）；`uncovered_requirements` 列出本叶未被正文回应的 REQ——真实
链路中逐条补写后重提，或接受 warning；逐叶重复至全部叶起草完。**每完成
一章**先把稿落盘再调 `save_version`（file_path 必须指向已存在的文件，
文件不存在 → E_NOTFOUND）：）

```json
{"tool": "save_version", "args": {
  "file_path": "/path/to/workspace/drafts/SEC-0005.md",
  "message": "SEC-0005 售后服务方案：external 稿，引用 REQ-0031，三级售后体系"}}
```

**P6 校验**

（余叶全部起草完后——空章节是 BLOCKER（E_SECTION_EMPTY）；演示链可 mock
补齐，真实链路每叶 external。）
```json
{"tool": "validate_bid_document", "args": {"project_id": "PRJ-…",
  "scope": "all"}}
```
```json
→ {"ok": true, "data": {"findings": [], "counts": {"blocker": 0, "major": 0,
   "minor": 0, "deterministic": 0, "model": 0}}, "project_version": 30}
```

（BLOCKER>0 时逐条修复（回 P5 重提 content）或经用户确认 WAIVED；
**validate 推进 project_version——后续工具必须用新值**。）

**P8 导出**

```json
{"tool": "export_bid_document", "args": {"project_id": "PRJ-…",
  "format": "docx", "output_path": "out/投标文件-技术标.docx",
  "project_version": 30}}
```
```json
→ {"ok": true, "data": {"artifact": {"url": "out/投标文件-技术标.docx",
   "sha256": "…", "size": 48213},
   "precheck": {"blockers": 0, "waived": []},
   "tables_written": 6, "forms_written": 7}, "project_version": 33}
```

（`output_path` 必须**工作区相对路径**（不得绝对路径/含 `..`）；复述
sha256 与六表/七件对账数字给用户。）

## 6. 错误速查

| 错误码 | 含义 | Agent 动作 |
|---|---|---|
| `E_STATE` | 状态机拒绝越级/非法转换 | 别跳步；按阶段表走（反模式 1） |
| `E_CONFLICT` | project_version 过期 | 重读状态拿新值，重试本工具 |
| `E_PARTIAL` | 部分条目失败（附 item_errors；data 携带成功面） | 逐件看 item_errors，失败件重试/确认放弃 |
| `E_NOTFOUND` | 项目/条目不存在 | 检查 ID；重查 get_bid_project_state / list_* |
| `E_UNPROCESSABLE` | 参数形态非法/前置不满足（含 external 互斥、EXPIRED 禁绑、无补遗阻塞） | 读 `error.what`（人读正文）修正后重试 |
| `E_AUTH` / `E_RATE` | 鉴权/限流 | 退避重试 |
| `E_INTERNAL` | 引擎内部错 | 带 request_id 报告维护方 |
| `UNIMPLEMENTED_INTERNAL` | 工具未接线（如 search_evidence） | 换替代路径（P4 三板斧），勿重试 |

错误应答形：`{ok: false, tool, error: {code, what, hint?, item_errors?}}`——
`what` 是人读正文（**不要对 what 做字符串匹配分支**，用 code）。

## 7. 联调与验收

- 最小可跑链：`python3 sdk/examples/minimal_flow.py`（stdio 直驱全链，
  external 模式起草——实测全链通过：P0→P8、citations≥1、unanchored=0、
  BLOCKER=0、docx 导出）；见该文件头部说明；
- 逐阶段质检：`python3 sdk/skill/bid-response-pipeline/scripts/checklist.py
  --stage P1 <应答.json>`（判据速查 `--list`）；
- 引擎侧回归：`bidcore` 全链测试 `tests/mcp_scaffold_tools.rs`（外部 Agent
  纯 MCP 驱动闭环 + 错误路径）与 `tests/draft_generator_switch.rs`
  （external 四态语义）。
