# 模块审计 E3：SDK / Skill 面审计（module-audit-sdk-skill）

- 审计代理：4-e3（ai-bid-research 全仓审计 V2 · Task ID 4-e3）
- 冻结基线：main@6105beb（分支 audit/v2-full-coverage）
- 范围：sdk/examples/minimal_flow.py（298 行全读）、sdk/skill/bid-response-pipeline/SKILL.md（239 行全读）、references/p1~p8（8 文件 1317 行——全读关键面 + 参数级 grep 对账）、scripts/checklist.py（199 行全读）、tests/test_mcp_e2e.py（1663 行——头部/关键段读 + mint_receipt/gate 面 grep 定位）、tests/evidence/*.json（11 文件——6 个深查 + 5 个结构抽查）
- 纪律：只读审计；未修改任何生产文件/测试/配置/数据；未运行任何脚本/工具
- 对照真源：`audit/modules/module-audit-bid-mcp.md`（实现 25 工具清单）+ `bidcore/crates/bid-mcp/src/tools.rs`（record_review_gate reviewer 显式拒）、`src/lib.rs:53 TOOL_NAMES`

---

## 1. 总判定

Skill 的阶段编排（P0→P8）与 25 工具面的**主干一致**（阶段表 22 个工具名全部真实存在、状态门叙述与实现相符、external 起草/receipt 验签/补遗分级门等新特性已同步写入）；但存在**一处 P1 级示例断裂**（minimal_flow.py 用已废除的 reviewer 参数，四门步骤必然被实现显式拒绝）与**两处 P2 级参考文档与实现矛盾**（p3-matrix.md 的 set_matrix_override「未接线」声明、p2-extract.md 的 file_ids[] 参数）。SKILL.md 头部「24 个 MCP 工具」计数过期（实现 25，#226 后未更新），且 3 个工具（list_tasks/set_matrix_override/get_coverage_report）在整个 Skill 面零指引。证据 JSON 为 2026-11-03 快照（n_tools=24、二进制来自另一工作树路径），不可原地重放，但可由 test_mcp_e2e.py 重跑再生（需 mcp Python SDK + cargo 环境）。

## 2. 工具面对账（SKILL.md ↔ 25 工具真源）

| 对账项 | SKILL.md 面 | 实现面（C1 审计） | 结论 |
|---|---|---|---|
| 工具总数声明 | 「24 个 MCP 工具」（SKILL.md:12） | TOOL_NAMES 25（#226 set_matrix_override） | **过期（SDK-002）** |
| 阶段表引用工具 | create/import/get_document_chunks/submit_block_marks/extract/build_outline/get_compliance_matrix/register/bind/list/search_evidence/draft/save_version/validate/export + 辅助 get_bid_project_state/check_bid_quality/score_price/score_section/score_document/record_review_gate/record_addendum_waiver = 22 | 均存在、参数叙述与实现对得上（含四门枚举、补遗 not_ready 锚、external content 语义、quota 字段族） | 一致 ✔ |
| 零指引工具 | — | list_tasks、set_matrix_override、get_coverage_report | 3 工具无任何 Skill 文档（SDK-002） |
| receipt 验签叙述 | 「旧 reviewer 参数显式拒——AI 操作者不得自审自过」+ v1.<actor>.human.<scope> 线格式 | tools.rs:4016-4020 显式拒 + req_str("receipt") | 一致 ✔（**但 minimal_flow.py 违反此纪律——见 SDK-001**） |
| search_evidence 模式 | hybrid/semantic/exact、top_k 1..=1000 | #145 实现一致 | 一致 ✔ |
| score_* 诊断口径 | diagnostic_only/rule_heuristic 纪律（#186） | C1 审计确认 | 一致 ✔ |

## 3. 问题记录（SDK-001 起）

### SDK-001
- ID: SDK-001
- 严重程度: P1
- 模块/文件路径/代码行号: sdk/examples/minimal_flow.py:222-228（四门循环）
- 函数或脚本: main() → record_review_gate 调用
- 问题描述: 示例用**已废除的 `reviewer` 参数**调 record_review_gate 且不带 `receipt`：`mcp.must("record_review_gate", {"project_id": pid, "gate": gate, "decision": "APPROVED", "reviewer": "human:minimal-flow-操作者"})`——#182 后实现显式拒绝 reviewer 在场（tools.rs:4016-4020「reviewer 参数已废除」→ E_UNPROCESSABLE）且 receipt 必填（4024）——示例运行到该步**必然 die**
- 调用链: 用户照 SKILL.md/agent-integration.md 跑 minimal_flow.py → P4 证据绑定成功 → 四门步骤 E_UNPROCESSABLE → 示例失败
- 触发条件: 任何环境跑 `python3 sdk/examples/minimal_flow.py`（当前 HEAD）
- 实际影响: 官方接入示例不可运行；外部 Agent 按示例学到的过门姿势（自报 reviewer）恰是被实现显式禁止的——**教了反模式**
- 复现步骤: 读 minimal_flow.py L223-228 + tools.rs L4011-4024 对比（静态可证，无需运行）
- 当前测试: sdk/tests/test_mcp_e2e.py（正确的 mint_receipt 流程 ✔）——但**不覆盖 examples/**；minimal_flow.py 无任何测试引用
- 缺失测试: examples 冒烟测试（或 CI 跑一次 minimal_flow）
- 建议方案: 示例改为 mint_receipt 同款流程（读 ws/.mang/approval.key 签 v1 receipt——test_mcp_e2e.py:137-150 已有可复刻实现）；或示例跳过四门并注明「人工门需宿主签发，示例不演示」
- 证据:
  ```python
  for gate in ("matrix_approval", "outline_approval", "pre_export_confirm", "addendum_waiver"):
      mcp.must("record_review_gate", {
          "project_id": pid, "gate": gate, "decision": "APPROVED",
          "reviewer": "human:minimal-flow-操作者",
      })
  ```
  对照 tools.rs:4017-4020：`if args.get("reviewer").is_some() { … "reviewer 参数已废除（#182）——人工确认由宿主签发 receipt 提供，Agent 不得伪造" }`
- 审计置信度: CONFIRMED
- 状态分类: 当前仍存在

### SDK-002
- ID: SDK-002
- 严重程度: P2
- 模块/文件路径/代码行号: sdk/skill/bid-response-pipeline/SKILL.md:12（「24 个 MCP 工具」）
- 问题描述: 工具总数声明过期（实现 25——#226 set_matrix_override 已接线）；且 list_tasks / set_matrix_override / get_coverage_report 三工具在 SKILL.md 与全部 references 中零出现——外部 Agent 无从得知其存在与用法（get_coverage_report 尤其关键：P2 submit_block_marks 的覆盖核对天然应查它）
- 触发条件: Agent 按 SKILL.md 建立工具心智模型
- 实际影响: 工具面认知缺 3/25（12%）；「24」计数误导对契约漂移的感知
- 当前测试: test_mcp_e2e 断言 n_tools ≥20（宽松）；evidence 01 记录 24（快照时点值）
- 缺失测试: SKILL/契约/实现三方工具清单一致性对账测试
- 建议方案: SKILL.md 计数改 25；补 set_matrix_override（含 receipt 纪律）与 get_coverage_report 的辅助工具条目（list_tasks 可标「内部/宿主面」）
- 证据: `> 确定性后端（bid-mcp，24 个 MCP 工具）` vs `pub const TOOL_NAMES: [&str; 25]`（lib.rs:53）
- 审计置信度: CONFIRMED
- 状态分类: 文档与实现不一致

### SDK-003
- ID: SDK-003
- 严重程度: P2
- 模块/文件路径/代码行号: sdk/skill/bid-response-pipeline/references/p3-matrix.md:122、:145
- 问题描述: p3-matrix.md 断言「NOT_APPLICABLE override …… **截至本版 main 无该 MCP 工具**（域层 set_matrix_override 存在、MCP 面未接线）：如实告知用户该行无法覆写」——与实现（#226 已接线为 MCP 工具 19，tools.rs:4120+，含 receipt/显式拒 principal）**直接矛盾**；同时与 p2-extract.md:43（「人工 receipt 覆写（set_matrix_override）」——暗示存在）**文档互相矛盾**。Agent 按 p3 指引会放弃一条已存在的合规路径
- 触发条件: 用户标书存在确不适用条款（联合体单方要求等）时按 p3 排查表操作
- 实际影响: 功能性误导（该走的覆写路径被文档劝退，改走「按缺证据处理」绕路）
- 复现步骤: 对读 p3-matrix.md L122 与 tools.rs set_matrix_override 实现段
- 建议方案: p3-matrix.md 改写为 set_matrix_override 用法（status 四枚举 + NOT_APPLICABLE 必带 deviation + receipt）
- 证据: `需要 NOT_APPLICABLE override（必须给偏离说明）——**截至本版 main 无该 MCP 工具**（域层 \`set_matrix_override\` 存在、MCP 面未接线）`
- 审计置信度: CONFIRMED
- 状态分类: 文档与实现不一致

### SDK-004
- ID: SDK-004
- 严重程度: P2
- 模块/文件路径/代码行号: sdk/skill/bid-response-pipeline/references/p2-extract.md:25
- 问题描述: 「`scope` 可传 `file_ids[]` 只抽指定件（增量补遗面）」——实现的 scope 仅支持 `"all"`（C1 审计 MCP-015：file_ids[] 未实现，非 all 值被拒/忽略）——Agent 按文档做增量抽取会得到 E_UNPROCESSABLE 或静默全量
- 触发条件: 补遗增量抽取场景（P2 恢复协议明确引用增量面）
- 实际影响: 增量流程指引失效（实际可行路径=整集替换重算合并，SKILL.md P2 恢复点描述的另一句倒是正确）
- 建议方案: 删除 file_ids[] 句或标注「预留位，当前仅 all」
- 证据: `` `scope` 可传 `file_ids[]` 只抽指定件（增量补遗面）``
- 审计置信度: CONFIRMED（以 C1 审计实现结论为真源）
- 状态分类: 文档与实现不一致

### SDK-005
- ID: SDK-005
- 严重程度: P2
- 模块/文件路径: sdk/tests/evidence/（11 个 JSON）
- 问题描述: 证据快照过期 + 环境锚定——① 01-initialize-and-tools.json 记录 `n_tools: 24`（当前 HEAD 25）；② 全部文件 `server_bin` 指向 `/home/z/my-project/bid-research-rw/bidcore/target/debug/bid-mcp-server`（另一工作树，非本仓路径——二进制 sha256 33d84ab… 无法在本仓原地验证）；③ ws 路径 /tmp/bid-mcp-e2e/* 已失效。**可重放性判定：可重跑再生、不可原地重放**——test_mcp_e2e.py 完整在仓（mint_receipt 正确复刻 #182 宿主签发面 L137-150 ✔、断言纪律「不为通过而改断言」✔、linejson/mcp 双传输 ✔），重跑需 pip install mcp pytest + cargo build + fixtures/corpus-A（仓内 ✔）+ BID_MCP_TARGET_DIR（默认 /tmp/shared-target，环境变量可调）
- 实际影响: docs/mcp-e2e-verification.md 引用的证据数据与当前工具面差 1 工具；审计追溯需信任快照（server_sha256 提供弱锚——但对应二进制不在本仓）
- 建议方案: #226 类工具面变更提交时同步重跑 e2e 证据；或证据头部加「生成时 HEAD」字段
- 证据: `"n_tools": 24` + `"server_bin": "/home/z/my-project/bid-research-rw/…"`
- 审计置信度: CONFIRMED
- 状态分类: 历史已修复但缺乏当前证据（指「与 HEAD 同步」这一面——快照本身完整且 189/189 checks pass）

### SDK-006
- ID: SDK-006
- 严重程度: P3
- 模块/文件路径/代码行号: sdk/skill/bid-response-pipeline/scripts/checklist.py:58-131（STAGES 表）
- 问题描述: ① 阶段判据缺 P7（审校/RevisionTask 闭环）——STAGE_TITLES 仅 P0-P6/P8，SKILL.md P7 的「每个 MAJOR finding 有 RT 且 DONE」无机器可执行版；② 门禁为**纯被动核对**（问题面答复：checklist.py 只读应答 JSON，不调用任何工具、不会触发最终校验/导出——项目不满足条件时它仅报 FAIL 退出码 1，无强制执行力）；③ P1 特例逻辑正确（E_PARTIAL ok=false 时判据落 totals 面 ✔）
- 实际影响: P7 收口靠 Agent 自觉；质检脚本覆盖 8/9 阶段
- 建议方案: 补 P7 判据（validate findings 的 severity 分布 + RT 状态需 list_tasks/查询口——若工具面不足则记缺口）
- 证据: `STAGES: dict[str, Stage] = {"P0": …, "P6": …, "P8": …}`（无 P7 键）
- 审计置信度: CONFIRMED
- 状态分类: 当前仍存在（P7 判据缺失面）

### SDK-007
- ID: SDK-007
- 严重程度: P3
- 模块/文件路径: sdk/tests/test_mcp_e2e.py:44（`BID_MCP_TARGET_DIR` 默认 `/tmp/shared-target`）、:197-199
- 问题描述: 默认 target 目录为环境约定路径（/tmp/shared-target）而非仓内 bidcore/target——新环境首跑需显式设置或依赖历史约定；auto 模式找不到 server 时静默降级 linejson（协议层用例显式 skip 并注明——诚实降级 ✔，但「mcp 传输」证据的再生依赖该约定）
- 建议方案: 缺省探测 bidcore/target 后回退 /tmp/shared-target
- 审计置信度: CONFIRMED；状态分类: 当前仍存在（设计取舍，影响低）

## 4. 逐文件审计记录

- **SKILL.md（239 行）**：阶段表顺序锁死（P0→P8 与实现状态机推进一致 ✔）；「禁止在材料只有招标文件而无企业证据时直接进入起草」前置禁令 ✔；补遗分级门（#184/#182 签域 addendum_waiver:{file_id}）与实现叙述一致 ✔；篇幅纪律（4000 字/批、quota 字段族）与 draft 返回体字段名对得上（quota.target_chars/written_chars/remaining/min_segments/remaining_segments/progress_pct）✔；external 两硬纪律（引用落本章节绑定集/承诺句同句带引用）与引擎实测面一致 ✔。问题：SDK-002。p5-skeleton.md（444 行）为内容面参考（真实标书章节骨架/字数分配），无工具参数——无漂移。
- **references p1-p8（1317 行）**：p1（导入分类/OCR 档位/失败降级）、p4（证据三板斧/有效期/补料清单）、p6（确定性规则清单/WAIVED）、p7（四门+receipt 验签五类失败码+RevisionTask）——与实现对账一致 ✔（p7:150 的「reviewer 在场→E_UNPROCESSABLE」排障表与实现显式拒一致）。漂移集中：p2:25（SDK-004）、p3:122/145（SDK-003）。
- **checklist.py（199 行）**：纯标准库 ✔（SKILL.md「无依赖质检脚本」承诺兑现）；点路径断链 fail-closed ✔；--dump-version 并发账提取 ✔。见 SDK-006。
- **minimal_flow.py（298 行）**：链路编排（create→import→extract→build_outline→矩阵翻页收集→register→bind→四门→check_bid_quality→external 首叶→save_version（先落盘再存版本——正确处理 §SV 文件须存在约束 ✔）→mock 补齐→validate→export）与契约一致；P4 矩阵游标翻页 ✔；**除四门外全部步骤与当前实现参数兼容**。见 SDK-001。
- **test_mcp_e2e.py（1663 行）**：双传输（mcp/linejson）+ 7 用例（00 环境留档/01 握手与工具清单≥20/02 全链 38 阶段 189 断言含重启续跑/03 错误信封/10-12 external 三负例（幽灵 EV/覆盖不全/无锚承诺）/13 CAS+幂等/14 search_evidence 全链 16 阶段）；mint_receipt 正确（approval.key + HMAC-SHA256 + scope 门级绑定）✔；旧 reviewer 参数负例探针（L704 expect_reject）✔；伪造 receipt 拒（L708-710）✔——测试面质量高，是 Skill 正确姿势的真源（与 examples 形成反差）。
- **evidence/*.json（11 文件）**：结构统一（case/transport/server_bin+sha256/python/mcp_sdk/os/started_at/stages[stage,tool,args,result,checks,elapsed]/payload_shapes/ws/finished_at）；10-14 各 34-38 checks 全 pass；00 环境自认「rustc 不在 PATH」（诚实）。见 SDK-005。

## 5. 风险汇总
P1×1（SDK-001——官方示例教反模式且必然失败）；P2×4（SDK-002/003/004/005）；P3×2（SDK-006/007）。
Skill 与工具面漂移要点：计数 24≠25、3 工具零指引、p3「未接线」矛盾、p2 file_ids[] 幻参数、示例 reviewer 废参——全部为**文档/示例滞后于 #182/#226 两轮实现演进**的单向漂移（实现正确、文档过期），未发现反向（文档承诺实现缺失）的新增项（file_ids[] 属契约遗留，已在 C1 记 MCP-015）。
