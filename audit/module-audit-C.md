# 模块审计报告 C —— bid-mcp / SDK / Skill / MCP 契约对账

> 审计人：Agent C（对外接口审计员）。基线：6105beb（工作树 8ee4799 audit(R0)）。
> 范围：`bidcore/crates/bid-mcp/`（src 13 文件 + bin 3 + tests 36 文件 243 测试 + mcp_sdk_e2e.py）、`sdk/`（21 文件）、`docs/bid-mcp-contract.md`（对账基准）、`docs/bid-skill-design.md`、`docs/agent-integration.md`。只读，未改任何生产文件。
> 交叉引用：P0-1（#243 validate 无条件推进状态机）、P0-2（#245 矩阵行派生只看绑定存在）——Lead 已确认，本报告只评估其**对外接口面的放大面**。

---

## A. 文件覆盖清单

### A-1 bid-mcp src（13 文件，6,740 行级）

| 路径 | 行数 | 状态 | 发现数 |
|---|---|---|---|
| src/lib.rs | 134 | READ | C-01/C-22/C-25 |
| src/tools.rs | 4310 | REVIEWED（全文分段） | C-04/C-05/C-07/C-09/C-12/C-15/C-16/C-23/C-27 |
| src/schema.rs | 616 | REVIEWED | C-05/C-24（对账表 C 节逐工具） |
| src/state.rs | 70 | READ | C-10/C-18 |
| src/transport.rs | 14 | READ | C-20 |
| src/protocol.rs | 592 | REVIEWED（含内嵌单测） | C-01/C-18/C-28 |
| src/cursor.rs | 408 | REVIEWED（含内嵌单测） | C-14 |
| src/error.rs | 104 | READ | C-01/C-13 |
| src/coverage_tools.rs | 284 | REVIEWED | C-16 |
| src/bin/stdio.rs | 34 | READ | C-18 |
| src/bin/server.rs | 49 | READ | C-18 |
| src/bin/a8.rs | 124 | READ | C-21 |
| （examples/blocking_probe.rs、req_dump.rs——探针非工具面 | 86+~80 | SKIM | —） |

### A-2 bid-mcp tests（36 文件 243 测试）

全部文件经测试函数名级扫描（`rg "fn ..."` 逐文件），重点文件（dispatch_contract / global_invariants / cursor_pagination / coverage_tools / set_matrix_override_tool / import_tender_files_tool / mcp_scaffold_tools）通读结构。逐文件覆盖见 E 节映射表。mcp_sdk_e2e.rs（65 行，Rust 驱动）与 mcp_sdk_e2e.py（149 行）全文 REVIEWED。

### A-3 sdk（21 文件）

| 路径 | 行数 | 状态 | 发现数 |
|---|---|---|---|
| examples/minimal_flow.py | 298 | REVIEWED | C-02/C-09 |
| tests/test_mcp_e2e.py | 1663 | REVIEWED | C-01/C-10（验收强度评估见 E） |
| tests/evidence/*.json（9 件） | ~204KB | SKIM（留档证据） | — |
| skill/bid-response-pipeline/SKILL.md | 239 | REVIEWED | C-03/C-04/C-29 |
| skill/.../references/p1-import.md | 91 | REVIEWED | — |
| skill/.../references/p2-extract.md | 78 | REVIEWED | C-12 |
| skill/.../references/p3-matrix.md | 153 | REVIEWED | C-03 |
| skill/.../references/p4-evidence.md | 119 | REVIEWED | — |
| skill/.../references/p5-skeleton.md | 444 | SKIM（业务骨架/篇幅纪律） | — |
| skill/.../references/p6-validation.md | 96 | REVIEWED | C-11/C-26 |
| skill/.../references/p7-review.md | 187 | REVIEWED | C-29 |
| skill/.../references/p8-export.md | 149 | REVIEWED | C-07 |
| skill/.../scripts/checklist.py | 199 | REVIEWED | C-08 |

### A-4 docs

| 路径 | 行数 | 状态 | 发现数 |
|---|---|---|---|
| docs/bid-mcp-contract.md | 757 | REVIEWED（全文） | C-05/C-13/C-30（对账表 C 节） |
| docs/agent-integration.md | 376 | REVIEWED | C-02/C-25 |
| docs/bid-skill-design.md | 77 | REVIEWED | （设计稿，历史层） |

---

## B. 发现清单（按严重程度排序）

### C-01【P1 · CONFIRMED】错误码线格式漂移：契约 `E_NOTFOUND` vs 线格式 `E_NOT_FOUND`
- 位置：`bidcore/crates/bid-mcp/src/error.rs:12-14`；`protocol.rs:538-546`（测试注释自认）；对照 `docs/bid-mcp-contract.md:11,120,131,390`。
- 证据：`#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]` 使 `ENotFound` 序列化为 `"E_NOT_FOUND"`；而 `ToolError::code()`（error.rs:44）与契约 §0/§4-A/§4-B/SV 全部使用 `"E_NOTFOUND"`。其余五码（E_STATE/E_CONFLICT/E_UNPROCESSABLE/E_PARTIAL/UNIMPLEMENTED_INTERNAL）两口径一致，唯独 NotFound 不一致。
- protocol.rs:538-546 注释原文承认："wire 序列化是 SCREAMING_SNAKE_CASE 变体名 → "E_NOT_FOUND"，而 ToolError::code() 返回契约字面量 "E_NOTFOUND"（预先存在的不一致，#137 只透传不改）"。
- 影响：契约 §0 承诺「`code` 是稳定契约字面量，调用方按 code 分支」。外部 Agent 按契约写 `if code == "E_NOTFOUND"` 将 miss 全部路由失败分支。**且 sdk/tests/test_mcp_e2e.py:1099,1105 已把 `E_NOT_FOUND` 固化为断言**（"实测：无项目查矩阵 = E_NOT_FOUND"）——测试适应了漂移而非纠正漂移，缺陷被测试面洗白。
- 触发条件：任何项目路由失败（project_id 不匹配）的调用方分支处理。
- 同根表现：C-28（协议层双通道两写法）。

### C-02【P1 · CONFIRMED】官方示例 minimal_flow.py 使用已废除的 `reviewer` 参数——示例链路必死，且方向性错误
- 位置：`sdk/examples/minimal_flow.py:222-228`；对照 `bidcore/crates/bid-mcp/src/tools.rs:4016-4023`。
- 证据：示例四门调用 `record_review_gate({... "reviewer": "human:minimal-flow-操作者"})`；tools.rs 对 `args.get("reviewer").is_some()` 显式返回 E_UNPROCESSABLE「reviewer 参数已废除（#182）」。
- 三重断裂：① 参数被显式拒——照抄必死在第一个门；② 示例注释「reviewer 必须 human: 前缀」是 #182 之前的旧信任模型，**方向与现行纪律（Agent 不得自述身份）完全相反**——在教 Agent 伪造人工身份（幸而 fail-closed 拦住）；③ 即便换成 receipt：示例在**起草之前**过 pre_export_confirm 门（P4 后立即四门），#180 审核内容绑定（reviewed_content_digest）会因后续起草改变章节树摘要而在 export 时拒绝（「审核针对的内容已变更」）——test_mcp_e2e.py:924-946 的 P7.5 顺序（全部叶起草后过门）才是正确时序，示例与之矛盾。
- 对照：`docs/agent-integration.md:369-371` §7 仍声称「实测全链通过：P0→P8…」——该声明对基线代码不成立（#182 后示例从未重跑/更新）。
- 影响：外部 Agent 按官方接入文档 §7 的指引运行示例即失败；信任模型的错误教学。

### C-03【P1 · CONFIRMED】Skill 对 set_matrix_override 的指引自相矛盾且与实现不符——否决项死锁出口在编排面分裂
- 位置：`sdk/skill/bid-response-pipeline/references/p3-matrix.md:122`；对照 `lib.rs:53-81`（TOOL_NAMES 含 set_matrix_override）、`tools.rs:4122-4215`（实现）、`tests/set_matrix_override_tool.rs`（20 测试）。
- 证据（三方矛盾）：
  1. **契约文档**：`docs/bid-mcp-contract.md` 全文 **0 次出现 set_matrix_override**（rg 验证）——对账基准缺第 25 个工具的正式条目（§17/§18/§19 之后无 §20）；
  2. **p3-matrix.md:122**：「需要 NOT_APPLICABLE override……**截至本版 main 无该 MCP 工具**（域层 set_matrix_override 存在、MCP 面未接线）：如实告知用户该行无法覆写」——错误断言，且其「来源锚点」还引用「lib.rs L50-72（TOOL_NAMES 21 个——无矩阵覆写工具）」的**三个版本前**代码；
  3. **p2-extract.md:43**：同一 Skill 包内又写「人工 receipt 覆写（set_matrix_override）后放行」——承认工具存在。
- 影响：Agent 遇否决项死锁（规则说明被判 mandatory 无正文可响应，#226 的场景）时，按 p3-matrix 的排查表会走「缺证据路径处理或等待工具面补齐」——正确出口（set_matrix_override + receipt）在主排查表中被否定。SKILL.md 主文说「24 个 MCP 工具」（实际 25）且全文未提 set_matrix_override。
- 修复方向：契约补 §20；p3-matrix 排查表翻案；SKILL.md 工具数与辅助工具清单更新。

### C-04【P1 · CONFIRMED】Skill 恢复协议在 VALIDATING 态引导 Agent 跳过未完成章节（P0-1 的编排放大面）
- 位置：`sdk/skill/bid-response-pipeline/SKILL.md:82-91`（恢复协议）；对照 P0-1（validate 无条件推进 Drafting→Validating；Validating 态 allowed_tools 不含 draft_response_section）。
- 调用链：P5 起草中途（任一叶未写完）→ Agent/用户误调 `validate_bid_document`（guard 表 Drafting 态**允许**——lib.rs:212）→ 状态推进 VALIDATING → 剩余叶 `draft_response_section` 永久 E_STATE。
- Skill 侧放大：恢复协议规则 1「status 已达阶段 N → 从 N 继续，禁止重跑已完成阶段」——Agent 读到 VALIDATING 会判定"已到 P6"而**放弃剩余章节直接进校验收口**，空章节 BLOCKER（E_SECTION_EMPTY）→ 无回写路径 → 整单报废（唯一恢复=手改 db.json，与 P0-1 同）。
- Skill 全文（SKILL.md + p6-validation.md）**没有任何「起草期禁调 validate / validate 不可逆」的警告**；p6 只说「validate 会推进 project_version」（版本面警告，掩盖了状态面不可逆性）。阶段表把 P6 完全放在 P5 之后给了虚假安全感——guard 允许 Drafting 态调 validate 是诱因，Skill 未设防是放大器。
- 严重程度：P1（编排引导与 P0-1 叠加成不可恢复流程事故；根因在 P0-1，本条记录接口面放大）。

### C-05【P1 · CONFIRMED】register/bind_evidence 的 guard 窗口与契约「MATRIX_READY+」不符——起草后补证据/补遗恢复链路全断
- 位置：`bidcore/crates/bid-domain/src/lib.rs:187-205`（guard 表）；对照 `docs/bid-mcp-contract.md:451,482`（「状态: MATRIX_READY+」）、`schema.rs:570,573`（describe 同口径）。
- 证据：guard 表中 `register_evidence`/`bind_evidence`/`import_tender_files`/`extract_requirements`/`build_outline` 仅出现在 MATRIX_READY / EVIDENCE_BOUND（及更早）——**Drafting/Validating/Reviewing/ReadyToExport/Exported 全部缺席**；契约与 schema 描述的「MATRIX_READY+」（'+' 惯例=含后续态）承诺了更宽的窗口。
- 触发场景（两个真实业务路径被断）：
  1. **起草中发现证据缺失**：DRAFTING 态调 register/bind → E_STATE；唯一出路是把证据内容写进 external 手稿（绕过证据台账）；
  2. **Skill 恢复协议第 3 条**（SKILL.md:89）「新增补遗文件 → P1 幂等导入 → P2 增量抽取 → … → 从 P4 起补」——一旦项目已过 EVIDENCE_BOUND，P1（import）、P2（extract）全被 guard 拒（Drafting 态 allowed_tools 无 import/extract）——**Skill 教的补遗恢复链路在 DRAFTING 之后不可执行**。
- 置信：CONFIRMED（guard 表静态可证；契约文本语义明确）。

### C-06【P2 · CONFIRMED】save_version 无围栏——任意绝对路径文件快照写盘，与 import/export 围栏纪律不对称
- 位置：`bidcore/crates/bid-mcp/src/tools.rs:1781-1825`。
- 证据：`file_path` 仅要求「绝对路径 + .md/.txt/.markdown + 文件存在」，随后 `mang_snap::find_root(&path)`（向上找最近 `.mang`，找不到用文件所在目录）并写入 `<root>/.mang/history/`。**无 BID_WS/BID_FS_ROOT 围栏检查**——Agent 可对围栏外任意可读文本文件（如 `/home/user/notes/secret.md`、`/etc` 下 md）创建快照并在其目录家族落 `.mang/history` 文件。
- 对照：import（fs_fence resolve_read 拒越界/symlink）、export（工作区相对路径 + resolve_write）双层围栏；save_version 是工具面唯一「绝对路径 + 无根校验」的写盘口。契约 §SV 如实写了「绝对路径」规则，但威胁模型未对齐（Agent 任意文件系统枚举/写面）。
- 影响：受限环境信息探测（文件存在性 oracle：E_NOTFOUND vs 成功可探测任意路径的 .md/.txt 是否存在）+ 围栏外目录写入。P2（写入内容受控——快照副本+message 元数据，非任意内容）。

### C-07【P2 · CONFIRMED】export 幂等键漏 bidder_name——暗标场景换投标人名重导返回旧脱敏产物
- 位置：`bidcore/crates/bid-mcp/src/tools.rs:3346-3350`；`parse_export_decision`（3133-3222，ExportDecision 不含 bidder_name/dark_bid）。
- 证据：ikey = `export:{project_id}:{content_version}:{format}:{dark_bid}:{output_path}:{decision_fp}`。`bidder_name` 既不在 ikey 明文段也不进 decision 指纹（ExportDecision 字段 = bid_amount/project_name/duration/validity/deposit/legal_rep/price_breakdown）。而 anonymize = dark_bid ? bidder_name : None 直接影响导出产物（公司名脱敏替换）。
- 触发条件：dark_bid=true，导出后更正 bidder_name（如公司名笔误）重导，content_version 未变（改 profile 不 bump 版本）→ 幂等命中 → 返回**旧 bidder_name 的缓存 artifact**（sha 亦旧）。
- 对照：契约 §8「幂等: (project_id, content_version, format, **profile**, output_path, decision_fp)」——profile 应整体入键；p8-export.md:80 同口径承诺「换报价重导 = 新导出」。bidder_name 属 profile 却被漏。违反 #117「换决策重导是新导出」语义。P2（需用户主动改名的二次导出场景）。

### C-08【P2 · CONFIRMED】checklist.py 的 P6 判据与 --dump-version 取位错误——project_version 永远取不到
- 位置：`sdk/skill/bid-response-pipeline/scripts/checklist.py:108-109,173-179`；对照 `schema.rs:58-76`（ToolReply 序列化只输出 `ok`/`tool`/`data` 三键）。
- 证据：`check("project_version", ...)` 与 `doc.get("project_version")` 都在**信封顶层**取值；实现中 project_version 恒在 `data` 内部。docstring 自述「输入 = MCP 应答原文：{ ok, tool, data, request_id, project_version?, warnings? }」——该信封形状与实现不符。
- 影响：① P6 阶段质检对**每一份合格应答**报 FAIL（"应答缺路径 'project_version'"）→ 退出码 1——质检脚本不可用；② `--dump-version`（Skill 并发纪律「带上最近一次响应里的最新值」的机器执行面）恒输出 `<应答未携带 project_version>`——乐观并发账在 Skill 工具链上断链。
- 与 C-09 同根（SDK 侧同错），独立表现。

### C-09【P2 · CONFIRMED】minimal_flow.py 客户端三缺陷：版本号取位错（并发失效）+ stderr 管道死锁风险 + 无超时
- 位置：`sdk/examples/minimal_flow.py:95-123`（McpStdio 类）。
- 证据：
  1. `reply.get("project_version")`（:108）顶层取值——与 C-08 同根因，`self.project_version` 恒 None，**所有写调用从不携带 project_version**（走服务端「缺省跳过检查」路径——乐观并发在示例里从未生效，与 SKILL.md:104-106 并发纪律直接矛盾）；
  2. `stderr=subprocess.PIPE`（:89）且循环内从不读取——Rust 侧 stderr 输出超管道缓冲（典型 64KB）即阻塞写 → 服务进程 hang → `readline()` 永久阻塞（真实 LLM 路径 generator=real 的日志面可能触发）。SUSPECTED（需运行验证 stderr 量）；
  3. `readline()` 无超时——服务进程 panic 恢复路径（state.rs 锁中毒自恢复之外的真实 panic 会终止循环）后管道关闭返回空才 die，但 hang 场景无守卫。
- 对照：test_mcp_e2e.py 的 LineJsonClient 已修复 ②（stderr 落文件）与 ③（asyncio.wait_for CALL_TIMEOUT）——examples 未同步测试侧的成熟实现。

### C-10【P2 · CONFIRMED（机制）/ 部署假设缺口】receipt 信任边界 = approval.key 文件可读性——同 uid 进程可伪造人工授权
- 位置：`bidcore/crates/bid-tools/src/service.rs:517-573`（密钥管理）；`bid-domain/src/principal.rs:223-289`（验签）；`sdk/tests/test_mcp_e2e.py:137-150`（mint_receipt）。
- 机制：#182 的信任根是 `<BID_WS>/.mang/approval.key`（32B，unix 0600）。验签只保证「持钥者签发」，**不绑定进程身份**。sdk/tests/test_mcp_e2e.py 的 mint_receipt 直接 `read_bytes()` 该文件并以「测试进程即宿主」身份签出合法 Human receipt（e2e 全绿证明此路径有效）——即：**任何与 bid-mcp 服务进程同 uid（或 root）的进程都能读 key 并伪造任意 actor 的人工授权**（record_review_gate 四门 / set_matrix_override 豁免 / record_addendum_waiver 全部放行）。
- 评估：#182 声称「宿主持钥，Agent 拿不到（这正是 #182 的信任根）」在威胁模型上只对「Agent 仅经 MCP 工具面访问」成立。stdio 部署形态（agent-integration.md §2：Agent 框架与 bid-mcp 同机同用户起子进程）下，Agent 框架进程具备直接 `open(BID_WS/.mang/approval.key)` 的能力。MCP 工具面本身没有读 key 内容的通道（import 需可解析文本、approval.key 是 32B 二进制必 parse FAILED），但同机进程级访问无防线。0600 只防跨用户。
- 结论：信任边界声明与部署现实存在缺口，威胁模型文档未把「宿主与 Agent 进程隔离」列为前置条件。P2（需进程级越权才可利用；但对「人工审批不可伪造」这一核心安全承诺是实质缺口）。

### C-11【P2 · CONFIRMED】WAIVED 豁免通道无实现——Skill 引导用户走不存在的 BLOCKER 豁免路径
- 位置：`sdk/skill/bid-response-pipeline/references/p6-validation.md:81-88`（豁免话术）；对照全仓 grep：无 waive finding 的工具/域方法；`tools.rs:3412`（export 响应 `precheck: { blockers: 0, waived: [] }` 硬编码空数组）。
- 证据：p6 教 Agent「确属误报或业务上可接受 → 登记 WAIVED + 理由（记录在案，导出 precheck 的 waived 清单可见）」并给了完整用户话术；契约 §7 也写「BLOCKER 清零（**或登记 WAIVED+理由**）才可 EXPORT」。但工具面 25 个工具无 waive 能力（record_review_gate 的 findings 是人发现问题不是豁免机器 finding；export 的 waived 恒 []）。
- 影响：Agent 按文档对用户说「是否豁免？豁免将记录在导出预检的 waived 清单中」，用户确认后**没有任何工具可以执行登记**——编排卡死或 Agent 自创记录（写进对话/正文，审计面无痕）。

### C-12【P2 · CONFIRMED】p2-extract.md 教 `scope: file_ids[]` 增量抽取——实现显式拒绝非 "all"
- 位置：`sdk/skill/bid-response-pipeline/references/p2-extract.md:25`；对照 `tools.rs:653-657`（`scope != "all"` → E_UNPROCESSABLE「B5 最小面：all——文件子集过滤属 M2 扩展」）、契约 §3 in 形状。
- 证据：p2 写「`scope` 可传 `file_ids[]` 只抽指定件（增量补遗面）」；契约 §3 in 也写 `scope:"all"|file_ids[]`（契约与 Skill 一致地超前于实现）。schema.rs:170 enum 只有 `["all"]`（schema 反而诚实）。
- 影响：与 C-05 场景 2 叠加——Skill 的补遗增量链路（P1 导入→P2 增量抽取→P4 补绑）三步中 P1/P2 被 guard 拒、P2 的 scope 参数被工具拒——**整条恢复路径在基线上不可走**。Agent 只能整集替换重抽（行为可行但与文档承诺不符）。

### C-13【P2 · CONFIRMED】契约 §0 三处虚承诺：E_AUTH/E_RATE/E_INTERNAL 无实现、idempotency_key 通用参数无实现
- 位置：`docs/bid-mcp-contract.md:11-12`；对照 `error.rs:14-38`（词典仅 6 码，且 UNIMPLEMENTED_INTERNAL 不在契约清单）、25 工具无任何 idempotency_key 入参（幂等靠 external_key/sha256/内部键）。
- agent-integration.md:360-361 错误速查表同样教 Agent 对 E_AUTH/E_RATE「退避重试」——Agent 永远等不到这两个码。P2（契约面承诺与词典失配；调用方错误分支面写空）。

### C-14【P2 · CONFIRMED】游标 fail-closed 只防「列表缩水」不防「列表增长位移」——分页漂移静默给错页
- 位置：`bidcore/crates/bid-mcp/src/cursor.rs:245-267`（slice 只在 `offset > total` 拒）。
- 证据：游标=偏移语义（cursor.rs:12-14 自述），指纹只绑「工具名+project_id+过滤器」，**不绑数据版本/总数**。两页之间底层列表**增长**（如 extract 整集替换新增 REQ 插在前部、或提交 marks 后重跑 extract）时 offset 仍 ≤ total → 静默返回移位后的窗口——翻页跳行/重行，无任何告警。契约 §0 承诺「偏移越界（底层列表已变化）→ E_UNPROCESSABLE」，只覆盖缩水方向。
- 触发面：#219 工作流鼓励「翻页标注→重跑 extract→再看矩阵」——正是矩阵行集变化场景。get_compliance_matrix 行序=条款位置序，REQ-Axxxx 追加与整集替换都会移位。P2（分页正确性；单次只读快照场景安全，连续写间翻页不安全）。

### C-15【P2 · CONFIRMED】get_compliance_matrix 过滤值不校验——非法 category/status 静默 0 行
- 位置：`bidcore/crates/bid-mcp/src/tools.rs:938-955`；对照 `list_evidence`（parse_evidence_kind/parse_evidence_status 显式拒）、`list_tasks`（parse_task_status 拒）、`search_evidence`（parse_search_filters 未知键显式拒）。
- 证据：category/status 过滤只做字符串比较，`category="资格"`（拼错/半角）或 `status="RESPOND"` → matched 空 → 返回 `total: 0` + ok。错误信息面零提示。
- 影响：与全仓 fail-closed 纪律（filters_of 的收紧注释 tools.rs:169-180 自述「静默吞掉是 B5 期宽松遗留，fail-closed 纪律下显式拒」）不一致；Agent 拼错过滤值会得到「矩阵为空」的危险误读（P3 资格判断按"qualification"逐页读——拼错即误报无资格条款）。

### C-16【P2 · CONFIRMED】get_document_chunks / get_coverage_report 无状态门——契约 §17/§19 的 E_STATE 承诺未实现
- 位置：`bidcore/crates/bid-mcp/src/coverage_tools.rs:67,236`（两者走 route_project——无 guard）；对照契约 §17「错误: E_STATE（状态机窗口外）状态: FILES_IMPORTED / REQUIREMENTS_EXTRACTED」、§19「错误: E_STATE（窗口外）」。
- 证据：guard 表（lib.rs:158-254）两工具列在 FilesImported/RequirementsExtracted，但实现不调 guard_tool → **任何状态（含 DRAFT、EXPORTED）可调**。submit_block_marks 走 route_guard ✓ 与契约一致——同族三工具两种门语义。
- 影响：只读无副作用，实害低；但契约对账基准失真（Agent 依赖 E_STATE 做窗口判断会误判状态机位置）。P2（契约漂移类）。

### C-17【P2 · CONFIRMED】search_evidence 的 E_STATE（项目无企业资料库绑定）承诺未实现
- 位置：契约 §5「错误: E_STATE（项目无企业资料库绑定）」；tools.rs:1636-1638 注释自认「E_STATE 资料库绑定面在朴素实现下不存在独立绑定——项目路由即总闸，M2-b 接知识库时补」。
- 同 C-16 类：契约未随实现降级修订。P2→P3（文档失真）。

### C-18【P2 · CONFIRMED】两个 stdio bin 的行读取无长度上限——超大消息全量进内存
- 位置：`src/bin/stdio.rs:16-33`、`src/bin/server.rs:26-48`（`stdin.lock().lines()` + `serde_json::from_str`，无行长限制）；MCP 官方规范有 maxMessageSize 概念，本实现未设。
- 触发：客户端发一行 1GB JSON → 进程内存全吞（ BufRead lines 逐行累积）。stdio 宿主（单客户端受信面）下风险有限，但「子进程退出异常恢复」审计点：读失败 break 退出（✓ 干净）；panic 面：state.rs 锁中毒自恢复 ✓（unwrap_or_else poisoned.into_inner）；服务层 panic 会终止传输循环（with_global 内 f panic 传播出 dispatch → stdio 循环 unwind → 进程死）——**工具执行中途 panic 会杀掉整个 stdio 服务进程**（无 catch_unwind）。
- P2（资源防护缺失）+ P3（panic 传播面）。

### C-19【P3 · CONFIRMED】dispatch_json 解析失败信封缺 `tool` 键、错误码口径与契约包装形状不符
- 位置：`lib.rs:112-127`。Parse 失败时返回 `{"ok":false,"error":{code:"E_UNPROCESSABLE",...}}`——无 `tool`（未知）与 request_id；契约 §0 包装 `{ok:false, tool, error:{...}}`。SDK 解析器对缺 tool 字段的容错未见约定。P3。

### C-20【P3 · CONFIRMED】Transport trait 零实现（死代码）
- 位置：`src/transport.rs:9-14`。全仓 grep 无 `impl Transport`；stdio.rs bin 直接用 io 循环不经 trait。注释声称「未来 HTTP/wasm 只需实现同一 trait」——当前是纯声明。P3（维护面：两条 bin 各自为政，抽象未兑现）。

### C-21【P3 · CONFIRMED】bin/a8.rs 验收链与生产工具链语义分叉
- 位置：`src/bin/a8.rs:27-49,75-78`。三处分叉：① `mode="公开招标"`（壳层枚举仅 明标|暗标，domain 透传不校验）；② `kind="主文件"`（TENDER_FILE_KINDS 七枚举外，#196 按 kind 分发抽取——a8 链的抽取行为与 MCP 路径 kind="招标文件" 不同）；③ `svc.set_requirements`（旧面，MCP 工具走 `replace_requirements` 整集替换）；④ 输出 status 字面量 `"unanswered"`（工具面是 NOT_RESPONDED）。CLI 直连服务层绕过全部壳校验。P3（验收数字与生产路径不可比）。

### C-22【P3 · CONFIRMED】allowed_next_tools 回显缺 4 个无门工具——Agent 误判可调面
- 位置：`bid-domain/src/lib.rs:150-256`（guard/回显表）；对照 tools.rs 分发（list_evidence/list_tasks 走 route_project、save_version/score_price 无路由）。
- 证据：get_bid_project_state 的 `allowed_next_tools` 是 allowed_tools() 直出——不含 list_evidence / list_tasks / save_version / score_price（这四个刻意无门，契约如实）。Agent 按「allowed_next_tools=现在能调什么」的理解会漏调两个核心只读列表工具与版本快照纪律工具。P3（信息面不全；e2e test_mcp_e2e.py:1008-1024 的注释已自我提醒「allowed_next_tools 与 terminal 口径不同」，同类口径差再添一层）。

### C-23【P2 · CONFIRMED（放大面评估）】register_evidence 语义自报面：P0-2 伪造链的对外入口
- 位置：`tools.rs:3784-3880`；`bid-tools/src/service.rs`（register_evidence_verified）。
- 评估（任务点 3 的「工具参数能否冒充证据」）：#185 已收来源身份（source_file_id 必须指向已导入文件，sha/指纹服务端生成）、#217 已收内容定位（locator 对账→CONTENT_MATCHES+excerpt 服务端截取）——**但证据的语义属性全部调用方自报**：kind（类别）、title、subject_name、scope、valid_from/valid_until、client/amount_wan/completed_on、fields（证书号/人员/金额结构化面）。
- 攻击链（纯 MCP 工具面，无需任何 receipt）：把招标文件本身（或任一已导入附件）的一段无关文本（如项目概述段）以 `locator` 锚定 → CONTENT_MATCHES 四态（「读过正文」的技术标记全绿）→ `kind="qualification"` + `title="ISO9001 质量管理体系认证证书"` + 任意 `valid_until` → bind_evidence 绑资格/否决行（P0-2：只查 can_bind，FILE_EXISTS/CONTENT_MATCHES 无差别，指向招标文件自身即可）→ **blocking 清零**。CONTENT_MATCHES 的语义是「该文本存在于该文件」，不是「该文本是所称的证书」——验证四态防位置伪造，不防语义伪造。
- 对比人工授权面：record_review_gate / set_matrix_override / record_addendum_waiver 三处有 receipt 验签（Agent 不可伪造），**证据真实性面零人工闸**（register 无 receipt 路径；契约 §11-A 的 SEMANTIC_SUPPORT/HUMAN_CONFIRMED 是「预留态」——#217 自己登记了这个洞）。
- 放大面结论：P0-2 的根在 domain 派生，但 MCP 面把「伪造一条证据」的成本降到一次工具调用（自报 title + 一次绑定），且全程无审计告警。与 P0-2 合并看是完整 P0 链；单独看接口面为 P2（入口无语义校验）。

### C-24【P3 · CONFIRMED】文档/注释工具数三版本并存（21/24/25）+ mcp_sdk_e2e.py 注释笔误
- 证据：agent-integration.md:16,60（「21 个工具」「全部 21 个已接线」）；SKILL.md:12（「24 个 MCP 工具」）；contract.md:17（「工具面 24」）；代码 TOOL_NAMES=25；mcp_sdk_e2e.py:31 注释「工具面 24」但 :78 断言 `len(tools) == 25`（断言正确、注释滞后）。P3（计数漂移——契约对账的第一道噪声）。

### C-25【P2 · CONFIRMED】agent-integration.md 接入文档整体滞后两个版本
- 证据：① 工具数 21（实际 25）；② §2 只教 `bid-mcp-stdio` 行协议（未提 `bid-mcp-server` 的标准 MCP JSON-RPC 通道——**工具发现面（tools/list）在接入文档层缺失**，外部 Agent 被引导走无握手/无发现的旧协议；#137 已交付但「本节更新」承诺未兑现）；③ §6 错误速查含未实现 E_AUTH/E_RATE（同 C-13）；④ §7「实测全链通过」声明失真（同 C-02）。基线上 tools/list 在 bid-mcp-server 真实存在（protocol.rs:180 + mcp_sdk_e2e.py 断言 25 工具）——任务点 4 的验证结论：**工具发现面在 server bin 在位、在文档与示例接线面缺位**。P2。

### C-26【P3 · CONFIRMED】p6-validation.md 的 scope 写法漂移
- 证据：p6:12 写「`scope`: "all" 或 `section_ids[]`」——实现是 `scope: "all"|"sections"` 字符串 + 独立 `section_ids` 数组参数（tools.rs:3473-3501）。Agent 若按文档传 `scope: ["SEC-0001"]` → E_UNPROCESSABLE。P3。

### C-27【P3 · CONFIRMED】错误信息内部状态泄漏评估（低危，如实记录）
- 证据：① route_project/import 等错误 what 携带「当前服务态项目：PRJ-xxxx」（tools.rs:197-204 等 7 处）——单项目单租户无害，多租户部署泄漏项目存在性与 ID；② E_CONFLICT 携带期望/实际版本号（error.rs:91-93）——可接受（并发对账需要）；③ save_version/export 错误带绝对路径（工作区内）——可接受；④ cursor 越界带过滤后总数（cursor.rs:250-256）——可接受。⑤ API Key 面：llm 配置错误只提示路径不含 key 值（tools.rs:1936-1941）；评审内容不入 MCP 错误面。**未见密钥级泄漏**。P3（多租户前瞻）。

### C-28【P3 · CONFIRMED】协议层双通道错误码两写法（C-01 的独立表现）
- 证据：`protocol.rs:218-225` 未知工具 → JSON-RPC error `data.code="E_NOTFOUND"`（硬编码契约字面量）；同一服务器工具执行错误 → isError 通道 `structuredContent.code="E_NOT_FOUND"`（serde）。两通道对「找不到」概念用两个不同字面量——按 code 分支的调用方要写两套。P3（与 C-01 合并修复）。

### C-29【P1 · CONFIRMED】p7-review.md 引导的 MAJOR 修复路径「回 P5 重提」在基线上不可行（P0-1 相邻面最直接证据）
- 位置：`sdk/skill/bid-response-pipeline/references/p7-review.md:98-101`「validate_bid_document 的 MAJOR finding 不自动生成 RT——它的修订闭环 = **回 P5 重提该章正文** + scope 重跑 validate」；SKILL.md 阶段表 P7（validate + RT 闭环）。
- 调用链：P6 validate（→ VALIDATING，P0-1 无条件推进）→ P7 读到 MAJOR finding → 按文档回 P5 `draft_response_section` → **E_STATE（Validating 态无 draft）** → 修复路径死 → 唯一出路手改 db.json。
- 与 C-04 合看：Skill 的 P6/P7 设计**假定 validate 可反复进入且起草可回写**——基线状态机不满足该假定。这是 Skill 流程×状态机窗口矩阵（F 节）中最致命的格子。CONFIRMED（guard 表 + service 推进逻辑静态可证）。

### C-30【P2 · CONFIRMED】契约文档缺 set_matrix_override 正式条目（对账基准不完整）
- 证据：`rg -c "set_matrix_override" docs/bid-mcp-contract.md` = **0**。25 工具中 24 个有契约章节（§1-§19 + §SV + §0 补充），唯 set_matrix_override（#226）只有 schema.rs:481-493 的 inputSchema 与 describe、lib.rs 注释、20 个专项测试——**对账基准文档本身漏收**。后果：以契约为单一真源的 Skill 生成物（如 p3-matrix）自然判其不存在（C-03 的根源之一）；外部 Agent 纯按契约驱动永远不知道有死锁出口。P2（基准完整性）。

---

## C. 工具面对账大表（25 工具）

四处一致性 = lib.rs TOOL_NAMES / tools.rs invoke 分发 / schema.rs inputSchema / guard 表（allowed_tools）。契约列 = docs/bid-mcp-contract.md 章节。

| # | 工具名 | 契约章节 | 四处一致? | Schema vs 实现 | guard vs 契约 | 实现特殊行为未写入契约 |
|---|---|---|---|---|---|---|
| 1 | create_bid_project | §1 | ✓（25/25 分发齐） | ✓（required/enum 对齐） | ✓ Draft+幂等前置 | 幂等重放响应含 deduplicated:true（契约 §1 只写「返回已有 project_id」） |
| 2 | import_tender_files | §2 | ✓ | ✓（files[].kind 七枚举/minItems） | ✓ Draft/FilesImported/ReqExtracted | **input_integrity 块（主件/附件就绪+waived）**——契约 §2 只有 addendum_readiness；parse_summary 透出（#218）未入契约 |
| 3 | extract_requirements | §3 | ✓ | ✗ 契约 in 写 `scope:"all"|file_ids[]`，schema/实现仅 all | ✓ | 契约承诺 E_PARTIAL（某文件抽取失败）**无实现路径**（纯函数全量成功） |
| 4 | get_compliance_matrix | §4 | ✓ | ✓（cursor/limit/page/filters） | ✓ 无门 | filter 值域不校验（C-15）——契约 §4 错误行未承诺拒非法值（纪律缺口） |
| 5 | list_evidence | §4-A | ✓ | ✓ | ✓ 无门 | — |
| 6 | list_tasks | §4-B | ✓ | ✓ | ✓ 无门 | — |
| 7 | search_evidence | §5 | ✓ | ✓ | ✗ 契约 E_STATE 承诺未实现（C-17） | tags 显式拒 / top_k≤1000 / naive engine 自标记（契约已如实标注 M2-b） |
| 8 | draft_response_section | §6 | ✓ | ✓（generator 四态/content 互斥） | ✓ MatrixReady/EvidenceBound/Drafting | **quota 块**（target/written/remaining/min_segments——SKILL 篇幅纪律的核心数据面）契约 §6 out 未收录 |
| 9 | validate_bid_document | §7 | ✓ | ✓（scope all|sections + section_ids） | ✓ Drafting+ | **无条件推进状态**（P0-1）；categories 四桶已入契约 ✓ |
| 10 | export_bid_document | §8 | ✓ | ✓（format/output_path/profile 全家） | ✓ Drafting+ 与 Exported | 幂等键漏 bidder_name（C-07）；precheck.waived 硬编码空数组（C-11） |
| 11 | get_bid_project_state | §0 补充建议 | ✓ | ✓（project_status 调试参数） | ✓ 无门 | allowed_next_tools 缺 4 无门工具（C-22） |
| 12 | save_version | §SV | ✓ | ✓ | ✓（契约明示不走状态机） | **无围栏**（C-06）——契约 §SV 如实但威胁面未对齐 |
| 13 | check_bid_quality | §9 | ✓ | ✓ | ✓ MatrixReady+ | dup_pairs/dup_max_similarity/char_deviation_pct 已入契约 ✓；「只读诊断」落盘 artifacts（契约已写） |
| 14 | build_outline | §10 | ✓ | ✓（值域 1000..5000000/1..50 与实现一致） | ✓ 仅 REQUIREMENTS_EXTRACTED | — |
| 15 | register_evidence | §11-A | ✓ | ✓（旧参数显式拒一致） | ✗ **契约「MATRIX_READY+」vs guard 仅 MatrixReady/EvidenceBound（C-05）** | title/subject/kind/validity/fields 全自报（C-23）；SEMANTIC_SUPPORT/HUMAN_CONFIRMED 预留态（契约已自登记） |
| 16 | bind_evidence | §11-B | ✓ | ✓ | ✗ 同 C-05 | — |
| 17 | record_review_gate | §12 | ✓ | ✓（receipt/findings 形状） | ✓ guard MatrixReady+，域层另按门细分窗口（矩阵门 VALIDATING+ 拒——两层门，契约已写） | — |
| 18 | record_addendum_waiver | §16 | ✓ | ✓ | ✓ FilesImported+ 全态（DRAFT 拒） | 契约 §16 错误行残留「principal 缺 human: 前缀」旧口径（#182 后应写「principal 参数在场即拒」） |
| 19 | score_price | §13 | ✓ | ✓（required 空——纯计算器） | ✓ 无门（契约已写「无项目也能算」） | — |
| 20 | score_section | §14 | ✓ | ✓ | ✓ MatrixReady+ | — |
| 21 | score_document | §15 | ✓ | ✓ | ✓ MatrixReady+ | — |
| 22 | get_document_chunks | §17 | ✓ | ✓（from/size≤200） | ✗ **契约 E_STATE 窗口承诺 vs 实现无门（C-16）** | kind 字段 = `format!("{:?}")` Debug 输出（契约枚举字面量靠变体名巧合对齐） |
| 23 | submit_block_marks | §18 | ✓ | ✓ | ✓ FilesImported/ReqExtracted（route_guard） | — |
| 24 | get_coverage_report | §19 | ✓ | ✓ | ✗ 契约 E_STATE vs 实现无门（C-16） | — |
| 25 | set_matrix_override | **无章节（C-30）** | ✓（TOOL_NAMES/invoke/schema/guard 四处在位） | ✓ | ✓ MatrixReady+ | 契约文档整体缺条目——描述真源只在 schema.rs |

**行协议层对账（任务点 4）**：
- JSON-RPC 框架（protocol.rs）：initialize（版本协商 4 版回显+兜底）/ ping / tools/list / tools/call 完整；jsonrpc:"2.0" 硬校验、通知静默、batch 拒（-32600）、id 形状校验、未握手 -32002——**框架完整且带 12 个内嵌单测**。
- **tools/list 在基线真实存在**（protocol.rs:180→schema::tools_schema；mcp_sdk_e2e.py 断言 25 工具+inputSchema）——#279 所指的工具发现面在 server bin 已开；缺的是**旧 stdio bin 与接入文档**（C-25）。
- 错误信封：双通道映射（isError:true + content[0].text 契约包装 + structuredContent）设计合理；唯 E_NOT_FOUND 双写法（C-28）。
- EOF/半包：逐行分帧，半包行 parse error 应答不崩（✓）；读 IO 错误 break 退出（✓）；**超大行无上限**（C-18）。
- 并发：dispatch 全局 Mutex 串行 + 单线程行循环——请求天然串行化（无并发正确性问题，亦无并发能力；state.rs 锁中毒自恢复 ✓）。

---

## D. 安全边界矩阵

| 面 | 机制 | 评级 | 发现 |
|---|---|---|---|
| **绕过 service 层直改持久化** | 全部 25 工具经 `tools::invoke → BidService`（写面全走服务层 checkpoint）；无工具直达 DomainStore/store_io。bin/a8.rs 是 CLI 直连服务层（同进程，非绕过）。**结论：无旁路** | ✅ | — |
| 人工授权伪造（receipt） | HMAC-SHA256 + scope 绑定 + 24h 窗 + Human-only + 常量时间比对；密钥 0600 文件。**MCP 参数面不可伪造**；同 uid 进程读 key 可伪造（C-10） | ⚠️ | C-10 |
| human: 前缀谁可声明 | `reviewer`/`principal`/`operator` 参数全部显式拒（三工具）；human: 前缀只出现在**回显**（`format!("human:{}", actor_id)`，tools.rs:4053）——派生值非判据，验签是唯一入口 | ✅（参数面） | C-02（示例反教） |
| 证据参数冒充（P0-2 放大面） | 来源身份/内容定位已服务端化（#185/#217）；**语义属性（kind/title/validity/fields）全自报、无人工闸**——CONTENT_MATCHES 只证「文本在文件里」不证「是所称证据」 | ❌ | C-23 |
| 文件系统写面围栏 | import：fs_fence 双层 ✓；export：相对路径+resolve_write ✓；**save_version：任意绝对路径无围栏** | ⚠️ | C-06 |
| 输出路径逃逸 | export 拒绝对路径/`..`（词法）+ 服务层组件级+symlink 复验（#189 契约已写） | ✅ | — |
| 错误信息泄漏 | 项目 ID 回显（多租户前瞻）/版本号/路径——无密钥级泄漏；LLM api_key 不入错误与日志面 | ✅（单租户口径） | C-27 |
| 日志面泄漏 | MCP 层无日志；stdio bin 仅 IO 错误 eprintln；测试侧 stderr 落文件；**api_key 全链未见输出** | ✅ | — |
| DoS 面 | stdio/server 行无上限（C-18）；get_document_chunks size≤200、limit≤1000、top_k≤1000、导入 500MB/64 件/2GB 限额 ✓ | ⚠️ | C-18 |
| 幂等/重复调用 | create(external_key)/import(sha256)/draft(ikey+content 指纹)/export(ikey——漏 bidder_name)/register(语义位+指纹)/bind(重复重放)/override(同参幂等，20 测试含)——**总体强**；唯 export 键缺一参 | ⚠️ | C-07 |
| 并发调用语义 | 全局 Mutex 串行（单客户端）；乐观并发 check_version 缺省跳过——**SDK 侧从未传递版本号**（C-08/C-09），并发账在实践上空转 | ⚠️ | C-08/C-09 |
| 子进程退出恢复 | EOF 干净退出 ✓；panic 传播杀进程（无 catch_unwind）——服务态由 db.json 重启恢复（BID_WS）✓；锁中毒自恢复 ✓ | ⚠️ | C-18 |

---

## E. 测试守卫缺口

### E-1 工具×测试覆盖映射（243 测试全量归位）

| 工具 | 测试文件（数量） | 覆盖的分支 |
|---|---|---|
| create_bid_project | import_tender_files_tool(21 中约 6)、bad_input_matrix、dispatch_contract、e2e_pipeline、protocol 内嵌 | 幂等重放/deadline 警告/参数校验/状态门 |
| import_tender_files | import_tender_files_tool(21)、partial_failure_gate(6)、input_format_matrix(9)、bad_input_matrix(6)、stdio e2e(1) | **最强**：E_PARTIAL/脏追加/围栏/幂等/版本冲突/坏件重传 |
| extract_requirements | extract_matrix_tools(8)、coverage_tools(13 中 4)、real_corpus_e2e、scale_1330、synth_tender_eval | 合并守恒/无标记 no-op/REQ-A/零条警告 |
| get_compliance_matrix | cursor_pagination(8 中 4)、extract_matrix_tools、review_table_matrix(3) | 12k 分页/跨过滤游标拒/坏游标 fail-closed |
| list_evidence / list_tasks | cursor_pagination(8 中 2+1) | 过滤分页/游标互斥/确定性 |
| search_evidence | search_evidence_tool(18)、sdk test 14 | exact 五字段/semantic 分词/hybrid 去重/降权/排序/错误面 8 例 |
| draft_response_section | draft_tool(10)、draft_generator_switch(10)、writing_acceptance_e2e(3)、subagent/real_llm_path | 四态生成器/CAS/幂等/引用回查/配额 |
| validate_bid_document | validate_tool(20)、quality_summary | 四桶/BLOCKER/规则族 |
| export_bid_document | export_tool(13)、export_readback(6)、pdf_export(5)、e2e_artifacts | 双前置门/六表七件/回读深断言/PDF 通道 |
| get_bid_project_state | dispatch_contract(8 中 5)、import 工具内 | 状态回显/terminal/伪造 status 拒 |
| save_version | save_version_tool(3) | 基本三例（**无围栏负例**——C-06 无守卫） |
| check_bid_quality | check_quality_tool(6)、quality_summary(3) | 诊断面/落盘 |
| build_outline | extract_matrix_tools、mcp_scaffold_tools | 窗口/空条款拒 |
| register_evidence | evidence_source_tool(8)、evidence_content_tool(4) | #185 来源验证/#217 四态 |
| bind_evidence | evidence_source_tool、agent_mandatory_gate(4) | 禁绑/重算 |
| record_review_gate | mcp_scaffold_tools(2 中 1)、sdk e2e G3 | 门序/receipt 正反例（伪造/旧参） |
| record_addendum_waiver | addendum_waiver_tool(4) | 件级确认/审计 |
| set_matrix_override | set_matrix_override_tool(20) | **最密**：四重绑定 receipt 正反 9 例/幂等/状态门 |
| score_* | score_tools(11)、e2e_artifacts(11 件套) | 三工具/多映射摊分 |
| coverage 三工具 | coverage_tools(13) | 窗口分页/区间校验/合并/重启持久化 |

### E-2 缺口清单

1. **E_NOT_FOUND 漂移无守卫**（C-01）：sdk e2e 把漂移断言为正确行为；无「线格式 code == ToolError::code()」一致性测试。
2. **minimal_flow.py 无 CI 无测试**（C-02）：官方示例断裂不被任何自动化面捕获（CI 仅 windows-check 的 cargo 面；scripts/SDK 无 CI——上下文已知，此处为断链现场）。
3. **save_version 无围栏负例**（C-06）：save_version_tool 3 例不测围栏外路径。
4. **export 幂等键 bidder_name 维度零测试**（C-07）：export_tool 13 例无「换 bidder_name 重导」用例。
5. **协议层健壮性**（C-18）：无超大行/半包/行溢出用例；protocol 单测全是正常帧。
6. **get_compliance_matrix 非法过滤值**（C-15）：无 category="foo" 应拒（当前实际不拒）或应告警的用例。
7. **Skill/文档一致性零守卫**：契约工具数/工具存在性/Skill references 与 TOOL_NAMES 的对账无任何测试（C-03/C-24/C-30 的根因——文档漂移只能靠人肉发现）。
8. **checklist.py 无自测**（C-08）：P6 判据取位 bug 若有最小自测（对一份真实应答跑一遍）即暴露。

### E-3 Python E2E 验收强度结论（任务点 7）

**sdk/tests/test_mcp_e2e.py 与 tests/mcp_sdk_e2e.py 均为真进程**（spawn bid-mcp-stdio/bid-mcp-server 子进程 + stdio 管道 + BID_WS 真实工作区 + 重启续跑用例 + receipt 真签验 + docx 解 zip 深检）——非 Mock。BID_MCP_CALL_SECS 超时（300s）+ TRANSPORT_DEAD 错误分类 + evidence 落盘留档，工程纪律高。**注意**：mcp 传输依赖 `pip install mcp`（缺席→linejson 或 skip）；linejson 模式下协议层用例显式 skip（不假绿 ✓）。

---

## F. Skill 流程 × 状态机窗口矩阵

行 = Skill 阶段（SKILL.md 阶段表）；列 = 项目状态（guard 实际放行面）。✓=可调；✗=E_STATE；△=域层二次窗口拒；○=无门恒可调。

| Skill 阶段（工具） | DRAFT | FILES_IMP | REQ_EXT | MATRIX_READY | EVID_BOUND | DRAFTING | VALIDATING+ | 评估 |
|---|---|---|---|---|---|---|---|---|
| P0 create | ✓(重放幂等) | 幂等重放○ | 同 | 同 | 同 | 同 | 同 | ✓ 一致 |
| P1 import | ✓ | ✓(dirty) | ✓(dirty) | ✗ | ✗ | ✗ | ✗ | **C-05**：Skill 恢复协议「补遗从 P1 补」在 MATRIX_READY+ 死 |
| P2 chunks/marks | ✗ | ✓ | ✓ | ✗ | ✗ | ✗ | ✗ | ✓（与契约 §17/18 一致；extract 同窗口） |
| P2 extract | ✗ | ✓ | ✓ | ✗ | ✗ | ✗ | ✗ | **C-05** 同上；scope=file_ids 增量路径 C-12 |
| P3 build_outline | ✗ | ✗ | ✓ | ✗(重复拒) | ✗ | ✗ | ✗ | ✓ 契约一致 |
| P3/P4 matrix 查询 | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ✓ |
| P4 register/bind | ✗ | ✗ | ✗ | ✓ | ✓ | **✗** | **✗** | **C-05 核心**：契约「MATRIX_READY+」假承诺；起草后补证死 |
| P4 list/search | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ✓ |
| P5 draft | ✗ | ✗ | ✗ | ✓ | ✓ | ✓ | **✗** | **C-04/C-29（P0-1）**：validate 误调后回 P5 死 |
| P5 save_version | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ✓（无门，契约如实；围栏 C-06） |
| P6 validate | ✗ | ✗ | ✗ | ✗ | ✗ | ✓ | ✓ | ✓ guard 放行——**但推进不可逆是 P0-1，Skill 无警告（C-04）** |
| P7 record_review_gate | ✗ | ✗ | ✗ | ✓ | ✓ | ✓ | ✓(仅 pre_export；矩阵/大纲门 △ VALIDATING+ 拒) | ✓（p7 门窗口表与 review.rs 一致——Skill 此处**正确且最新**） |
| P7 set_matrix_override | ✗ | ✗ | ✗ | ✓ | ✓ | ✓ | ✓ | ✓（但 **SKILL/p3-matrix 不教这个工具**——C-03/C-30） |
| P8 export | ✗ | ✗ | ✗ | ✗ | ✗ | ✓ | ✓ | ✓ |
| 补遗确认 record_addendum_waiver | ✗(DRAFT 拒) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓（契约 §16 一致） |
| 自评 score_* / check_quality | ✗ | ✗ | ✗ | ✓+ | ✓+ | ✓+ | ✓+ | ✓ |

**矩阵判读（Skill 会不会引导踩雷）**：
1. **主链顺序（P0→P8 线性走）与状态机完全兼容**——Skill 阶段表「顺序锁死」设计正确避开了所有 ✗ 格子；
2. **三个雷区全部在「恢复/回环」语义**：① P6 误触后回 P5（C-29，P0-1 放大）；② DRAFTING 后补证据/补遗（C-05——Skill 恢复协议第 3 条明确引导走进此雷）；③ VALIDATING+ 补矩阵门（p7 已正确文档化 ✓）；
3. **set_matrix_override 死锁出口在 Skill 主文缺失**（C-03/C-30）——唯一能解「否决项规则说明死锁」的工具没有任何阶段引用；
4. SKILL P4 完成判据「bind 后 matrix.blocking=0」依赖的绑定语义正是 P0-2 的弱派生——**Skill 把 blocking=0 当作「证据齐」的停止条件，而该数字可被伪造证据清零**（C-23 + P0-2 交叉：编排骨架建立在可污染的信号上）。

---

## 结语（对 Lead 的三个行动建议优先级）

1. **接口契约止血**（C-01/C-30/C-03）：修 E_NOT_FOUND 序列化（或契约改口径并在 SDK 同步）；契约补 set_matrix_override 条目；p3-matrix 排查表翻案——三处一起改才能对齐「契约=单一真源」的纪律声明。
2. **外部接入面修复**（C-02/C-08/C-09）：minimal_flow.py 换 receipt+修正四门时序；checklist.py/客户端版本号取位（data.project_version）；这两个是外部 Agent 的第一触点，当前状态是「文档说能跑、跑了必死」。
3. **Skill 恢复协议加护栏**（C-04/C-29/C-05）：在 SKILL.md 显式登记三个不可逆点（validate 不可逆、起草后无补证窗口、矩阵门窗口），恢复协议从「status 已达阶段 N → 从 N 继续」改为「按 allowed_next_tools 与内容完成度双判断」——在 P0-1/#243 修复落地前，这是编排侧唯一能做的防呆。

（本报告未改动任何生产文件；全部发现基于静态阅读，标注 CONFIRMED 者均有文件:行号级证据链。）
