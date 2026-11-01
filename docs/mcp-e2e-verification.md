# MCP 端到端验证报告（issue #139）

> 验证目标：证明使用方按「**加载 SKILL.md → MCP 连 bid-mcp → Agent 按阶段表驱动**」
> 的方式能真跑通——正文由使用方 Agent 写（`generator=external`），
> 引擎做引用回查 / CAS / 落盘 / 校验 / 导出。
>
> 验证脚本：`sdk/tests/test_mcp_e2e.py`（pytest 与独立运行双入口）
> 证据留档：`sdk/tests/evidence/*.json`（每阶段的工具原始返回 + 检查项）

---

## 0. 结论（TL;DR）

| 验证面 | 结果 | 说明 |
|---|---|---|
| P0-P8 全链（工具层，linejson 传输） | ✅ 通过 | 23 章全起草 → 校验 BLOCKER=0 → docx 169,965 字节 |
| external 模式四用例 | ✅ 全过 | ①幽灵 EV 剔除不拒稿 ②覆盖缺口回报 ③无锚承诺计数 ④CAS 版本递增 |
| 错误信封（E_* 归类） | ✅ 通过 | 7 种失败面全部正确分类，无静默成功 |
| **MCP 协议层（initialize / tools/list / tools/call）** | ⏸ **SKIP（如实）** | **待 #137 交付 `bid-mcp-server`**——脚本已就绪，交付后 `BID_MCP_TRANSPORT=mcp` 即真跑 |

**当前验证了什么**：真实进程边界（spawn + stdio）上的**工具层全链**——
20 个工具的参数面、状态机、幂等、CAS、落盘、导出、重启续跑全部实测通过。
linejson 与 mcp 两种传输走的是**同一个分发内核**（`dispatch_json → tools::invoke`），
协议层只差 JSON-RPC 帧那一层（#137 的范围）。
**没有验证什么**：JSON-RPC 2.0 帧本身的握手与编码（待 #137），已在
`test_01_initialize_and_tools` 中以 SKIP 显式标记，不假装通过。

---

## 1. 验证环境

| 项 | 值 |
|---|---|
| OS | Linux x86_64（glibc 2.41） |
| Python | 3.12.16 |
| MCP Python SDK | `mcp==1.27.0`（官方 `pip install mcp`） |
| pytest | 9.0.2 |
| Rust 工具链（构建 server） | cargo/rustc 1.99.1 stable |
| server 二进制 | `bid-mcp-stdio`（main = `2a0e311`） |
| server sha256 | `134e9939d46742d229bba97ead748a2a2878c620af44258bac1968e7cd1029cd` |
| 语料 | `fixtures/corpus-A/files`（GT 25 埋点真件：A 主文件 docx + C 补遗 + 2 件证据 PDF） |
| 工作区 | `BID_WS` 环境变量指向临时目录（每用例独立，进程级持久化） |
| 全链耗时 | 约 2 秒/项目（P0→P8，23 章） |

---

## 2. 传输模式（本报告「真跑」的确切含义）

脚本支持两种传输，`BID_MCP_TRANSPORT` 选择：

- **`mcp`**（目标形态）：官方 SDK `stdio_client(StdioServerParameters)` +
  `ClientSession`，走标准 initialize / tools/list / tools/call。
  **需要 #137 的 `bid-mcp-server`**。脚本自动发现二进制并断言
  `serverInfo.name == "bid-mcp"`、`tools ≥ 20`。
- **`linejson`**（本轮实际使用）：现网 `bid-mcp-stdio` 的逐行 JSON 协议
  （`{"tool","args"}` 一行进、`{"ok","tool","data"|"error"}` 一行出）。
  真实子进程 + 真实 stdio 管道 + `BID_WS` 持久化工作区，
  与 mcp 模式共享同一分发内核与同一场景体。

`auto` 模式优先找 `bid-mcp-server`，找不到回落 `bid-mcp-stdio`——
#137 交付后**无需改脚本**，重新编译即自动切换为真 MCP 握手。

---

## 3. 任务 1：P0-P8 全链实测（test_02_full_pipeline）

场景：corpus-A 真件导入 → 抽取 → 建纲 → 绑证据 → 三门过审 →
**23 章全部 external 起草**（模拟使用方 Agent 写正文）→ 校验 → 导出 docx。

| 阶段 | 工具 | 实测结果（JSON 摘要） |
|---|---|---|
| P0 建项 | `create_bid_project` | `project_id: "PRJ-0001"`, `status: "DRAFT"`；同 `external_key` 重放 → 原 id + `deduplicated: true` ✅ |
| P1 导入 | `import_tender_files` | `totals: {files: 4, chunks: 208, parse_failed: 0, partial: 0}`；A 主文件 182 chunks / 6 tables / parse ok ✅ |
| P2 抽取 | `extract_requirements` | `counts: {total: 121, by_category: {tech_param: 82, veto: 13, qualification: 6, scoring: 4, commercial: 3, format: 2, other: 11}}` ✅ |
| P3 建纲 | `build_outline` | `counts: {sections: 30, leaves: 23, matrix_rows: 121, matrix_blocking: 19, total_target_chars: 11500}`，`status: "MATRIX_READY"` ✅ |
| P3 矩阵 | `get_compliance_matrix` | `total: 121`（一页 limit=1000 取全），`draft_section_id` 分组得 23 个可起草章节 ✅ |
| P4 登记 | `register_evidence` | `ev_id: "EV-0036"`, `status: "ACTIVE"`, `deduplicated: false` ✅ |
| P4 绑定 | `bind_evidence` | 必绑类目（资格 6+否决 13+技术参数 82）101 条全绑：`bound_new: 101`, `matrix.blocking: 19 → 0` ✅ |
| G3 过审 | `record_review_gate ×3` | matrix/outline/pre_export 三门 APPROVED（reviewer `human:mcp-e2e`）✅ |
| **P5 起草** | `draft_response_section ×23` | 全部 `generator: "external"`、`section_status: "DRAFTED"`；样例 SEC-0007：`words: 344, citations: 7, uncovered: [], unanchored: 0` ✅ |
| P6 质检 | `check_bid_quality` | `summary: {total: 23, drafted: 23, failed: 0, uncovered_req_total: 0, unanchored_total: 0}`；报告落盘 43,434 字节 ✅ |
| P7 校验 | `validate_bid_document` | `ok: true, counts: {blocker: 0, major: 0, minor: 24, deterministic: 24}` ✅ |
| P8 导出 | `export_bid_document` | `chapters_written: 23`, `size: 169,965`, `tables_written: 2`（技术参数偏离表/项目概况表），`forms_written: 7`, `toc_field_present: true`, `status: "EXPORTED"` ✅ |

**产物深检**（解 zip 读 `word/document.xml`，等价于「Word 打开」）：
TOC 域在位 ✅ · 正文探针句进文档 ✅ · **无字面 `**` 泄漏**（external 正文里的
行内粗体标记被正确转成粗体 run——#117 的接线在 external 路径同样生效）✅ ·
无 md 井号泄漏 ✅ · `styles.xml` 含中文字体面（`w:eastAsia`）✅

**工作区落盘**：`chapters/*.json` 23 件（attempt/citations/draft_note 全在位），
`db.json` 状态机持久化在场 ✅

**重启续跑**（新进程读同一 `BID_WS`）：`status: "EXPORTED"` 完整恢复，
`terminal: true` ✅

---

## 4. 任务 2：external 模式四用例（使用方核心路径）

四个用例各自独立建项目跑完整前置（P0→P4→三门），对同一章 SEC-0007
（资格与响应组，6 REQ + 1 绑定 EV）投递不同正文：

### ① 幽灵 EV- 编号 → 剔除 + warning，不拒稿 ✅

正文同时引用真证据 `[EV-0036]` 与不存在的 `[EV-9999]`：

```json
"stats": {"generator": "external", "words": 367, "citations": 7, "unanchored_claims": 0}
"warnings": ["引用回查：EV-9999 不在已知 REQ/EV 集合（生成器幻觉）——已从正文剔除"]
```

- 调用**成功**（`section_status: "DRAFTED"`——不拒稿）✅
- `cited_ids` 含 `EV-0036` 不含 `EV-9999` ✅
- 白盒复查 `chapters/SEC-0007.json`：落盘正文已**物理剔除** `EV-9999` ✅

### ② 覆盖不全 → uncovered_requirements ✅

正文只引用 6 条 REQ 中的第一条：

```json
"stats": {"cited_ids": ["REQ-0001"],
          "uncovered_requirements": ["REQ-0002","REQ-0005","REQ-0006","REQ-0007","REQ-0008"]}
```

`uncovered_requirements` 与「本章 REQ − 已引用」精确相等；warnings 逐条告知 ✅

### ③ 承诺句无引用 → count_unanchored_claims > 0 ✅

正文含 2 句带「承诺/确保」但无引用的句子（其余句全部带锚）：

```json
"stats": {"unanchored_claims": 2, "uncovered_requirements": []}
"warnings": ["无锚承诺 2 句（含承诺词但同句无 [REQ-*]/[EV-*] 引用）——建议补引用或改写；是否阻断由 validate 裁决"]
```

计数**恰为 2**（句级判定准确）；覆盖面不受影响 ✅

### ④ 正常正文 → CAS 写入成功，版本递增 ✅

| 调用 | content | version | content_ref | deduplicated | project_version |
|---|---|---|---|---|---|
| #1 | 内容 A | 1 | `DOC-SEC-0007-v1` | — | 120 |
| #2 | 内容 B（≠A） | **2** | `DOC-SEC-0007-v2` | — | **121** |
| #3 | 内容 B（重放） | 2 | `DOC-SEC-0007-v2` | **true** | 121（零副作用） |

白盒复查：`chapters/SEC-0007.json` 的 `attempt == 2`，落盘内容为最新稿 B ✅

---

## 5. 错误信封（附加验证，test_03_error_envelope）

Agent 的分支处理依赖 `E_*` 错误码。七种失败面实测归类：

| 失败面 | 错误码 |
|---|---|
| 无项目时查矩阵 | `E_NOT_FOUND` |
| 调用未知工具 | `E_NOT_FOUND` |
| 建项缺 `name` | `E_UNPROCESSABLE` |
| `mode` 非法（非 明标/暗标） | `E_UNPROCESSABLE` |
| `generator=external` 缺 `content` | `E_UNPROCESSABLE` |
| `content` × `generator=mock` 互斥 | `E_UNPROCESSABLE` |
| 未导入先抽取（状态机越级） | `E_STATE` |

信封形状统一为 `{"ok": false, "error": {"code", "what"}}`，`what` 人类可读 ✅
全部被拒，无静默成功 ✅

---

## 6. 工具返回格式观察（如实报告——给 #137/#138 的输入）

验证过程中记录到的形态细节，**均不构成阻断**，但接入文档（#138）应写明：

1. **`create_bid_project` 首建响应无 `deduplicated` 字段**（缺席 ≠ false）。
   契约 §1 的 out 只承诺 `{project_id, status}`；只有同 key 重放才带
   `deduplicated: true`。客户端判定首建应以「字段缺席或假值」为准。
2. **EXPORTED 终态的 `allowed_next_tools` 非空**（8 个只读/幂等面工具：
   查矩阵、质检、重导出等），与 `terminal: true` 并存。两字段口径不同：
   判终态用 `terminal`，`allowed_next_tools` 是「该态还能调什么」。
3. **`register_evidence` 签发的 EV-id 从后台账计数后继**（本轮为 `EV-0036`，
   因 import 阶段切块已先入 EV 台账 35 条）。契约 §2 已注明
   （#80 摩擦 #7），Agent 不应假设新登记从 `EV-0001` 起。
4. **矩阵行不携带章节标题**：`get_compliance_matrix` 的行只有
   `draft_section_id`，无章名。Agent 能知道「哪章管哪些 REQ」，但拿不到
   章节标题文本（导出时标题由引擎从大纲树注入，不影响产物）。
5. **每章字数配额不可经 MCP 查询**：P7 的 24 条 MINOR 警告全部是
   「字数 344 偏离配额 500（±20% 带外）」，但 `build_outline` 只返回
   `total_target_chars`（总量），**没有每叶配额字段**——Agent 无从对齐。
   建议后续在矩阵行或状态工具中暴露 `target_chars`（API 面缺口，非缺陷）。
6. **issue #139 示例参数与实际契约参数不一致**（以实装为准）：
   `create_bid_project` 实际是 `{name, bid_deadline, mode(明标|暗标),
   external_key, tender_notice_ref?}`，不是示例中的
   `project_name/project_code/procurement_type`。
   **#138 的 SKILL.md 必须按契约参数写**，否则使用方第一步就会撞 E_UNPROCESSABLE。

---

## 7. 已知限制（如实）

1. **MCP 协议层未验证**（本轮最大限制）：initialize / tools/list / tools/call
   的 JSON-RPC 2.0 帧、`notifications/initialized`、错误码映射
   （ToolError → JSON-RPC error 或 `isError: true`）都待 #137 交付
   `bid-mcp-server` 后由 `test_01_initialize_and_tools` 真跑。
   当前该用例如实 SKIP。
2. **工具数口径**：`TOOL_NAMES` 共 **20** 个（不是 #137 提到的 26），
   其中 `search_evidence` 返回 `UNIMPLEMENTED_INTERNAL`（M2-b 待启）。
   `tools/list ≥ 20` 的验收线以「声明数」满足；实装 19 个。
3. **单项目约束**：每个 server 进程（每个 `BID_WS`）承载一个项目；
   异 `external_key` 再建项会被状态机拒（E_STATE）。多项目需多工作区多进程。
4. **linejson 模式无握手面**：`initialize`/`tools/list` 在该传输下不存在，
   脚本以显式 SKIP 表达，不以工具层通过冒充协议层通过。
5. **语料为合成 GT 件**（corpus-A）：真实水科 PDF 的全链验证属 #131 范围，
   上一轮已完成（3 件真实件红线逐位复现）；本轮聚焦接入面而非语料真实性。
6. **PDF 导出通道未测**：P8 只测 docx（pdf 通道依赖 soffice，与本轮验证
   目标正交）。

---

## 8. 复现步骤

```bash
# 1) 依赖
pip install mcp pytest

# 2) 编译 server（当前 main 即可——linejson 模式）
cd bidcore
cargo build -p bid-mcp --bin bid-mcp-stdio   # 产物在 target/debug/（脚本默认找 /tmp/shared-target/debug，可用 BID_MCP_TARGET_DIR 指）

# 3) 跑（linejson——现状即可复现本报告全部结果）
pytest sdk/tests/test_mcp_e2e.py -v
#   预期：7 passed, 1 skipped（SKIP = MCP 协议握手，待 #137）

# 4) #137 交付后（mcp 模式——无需改脚本）
cargo build -p bid-mcp --bin bid-mcp-server
BID_MCP_TRANSPORT=mcp pytest sdk/tests/test_mcp_e2e.py -v
#   预期：8 passed, 0 skipped（握手 + 工具清单 + 全链都走真 MCP）
```

证据文件（本轮实测原始留档）：

```
sdk/tests/evidence/00-environment.json          环境（二进制 sha256 / SDK 版本）
sdk/tests/evidence/01-initialize-and-tools.json SKIP 留档（待 #137）
sdk/tests/evidence/02-full-pipeline.json        P0-P8 全链（38 个阶段记录）
sdk/tests/evidence/03-error-envelope.json       七种失败面归类
sdk/tests/evidence/10-external-ghost-ev.json    用例①
sdk/tests/evidence/11-external-uncovered.json   用例②
sdk/tests/evidence/12-external-unanchored.json  用例③
sdk/tests/evidence/13-external-cas-version.json 用例④
```

---

*验证人：代理 C（issue #139）· 2026-11-01 · 分支 `test/mcp-e2e`*
