# 模块审计 C1：bid-mcp 源码完整工具面审计（module-audit-bid-mcp）

- 审计代理：4-c1（ai-bid-research 全仓审计 V2 · Task ID 4-c1）
- 冻结基线：main@6105beb（分支 audit/v2-full-coverage；工作树 HEAD 6ae52f8 仅含审计文档，生产代码与基线一致）
- 范围：`bidcore/crates/bid-mcp/src/` 9 个源文件（tools.rs 4310 行分段读全 + schema.rs/protocol.rs/cursor.rs/coverage_tools.rs/lib.rs/error.rs/state.rs/transport.rs 全读）＋ 3 个 bin（stdio.rs/server.rs/a8.rs）交叉读取
- 纪律：只读审计；未修改任何生产文件/测试/配置；未运行 cargo test/build；本文与 inventory-c1.csv 为本轮唯一新增产出
- 方法：逐文件通读 + 与 docs/bid-mcp-contract.md（758 行全读）逐工具对账 + 与 audit/pilot/（PA/PB）交叉核实 + bid-domain `allowed_tools`/`guard_tool`、bid-tools service.rs/draft.rs 关键面定向复核

---

## 1. 工具面总清单（25 工具——实现真源 `lib.rs:53-81 TOOL_NAMES`）

契约 §0 宣称工具面 **24**；实现与 `tests/dispatch_contract.rs:30` 断言均为 **25**（#226 `set_matrix_override` 未写入契约）。逐工具对账（"门"= guard_tool 裁决；"无门"= 仅 route_project/无校验，任意状态可调）：

| # | 工具 | 关键参数（* = 必填） | 状态门（域 allowed_tools 实口径） | 幂等/版本 | 契约对账结论 |
|---|------|----------------------|-----------------------------------|-----------|----------------|
| 1 | create_bid_project | name*, bid_deadline*, mode*(明标\|暗标), external_key*, tender_notice_ref? | 入口边：无项目免 guard；同 key 重放先于 guard；异 key 非 DRAFT → E_STATE | external_key 幂等（重放回 deduplicated:true） | §1 基本一致；重放多回 `deduplicated` 字段（未记档） |
| 2 | import_tender_files | project_id*, files*[{upload_ref*, filename*, kind?(7 枚举), expected_pages?}], project_version? | Draft\|FilesImported\|RequirementsExtracted；**围栏先于路由/状态机**（零副作用拒） | sha256 去重 + #188 FileRole；project_version 可选 | §2 一致（E_PARTIAL 带 data 成功面 ✔）；响应多 `input_integrity`/`parse_summary` 块（未记档，加法） |
| 3 | extract_requirements | project_id*, scope?("all" 仅此值), options?(忽略), project_version? | FilesImported\|RequirementsExtracted + parsed 非空 | replace_requirements 整集替换 | §3 大体一致；**契约 scope:"all"\|file_ids[] 的 file_ids 未实现**（MCP-015）；ZERO_REQUIREMENTS/ENGINE_PATH 告警未记档（好行为） |
| 4 | get_compliance_matrix | project_id*, filters{category?, status?}, cursor?, limit?, page? | **无门**（任意状态，只读） | 只读零步进；游标信封 | §4 信封/游标一致；**category/status 过滤值不校验**——拼错静默空页（MCP-012，与 §4-A/B 的 fail-closed 不一致） |
| 5 | list_evidence | project_id*, filters{kind?, status?}, cursor?, limit? | 无门（只读） | 只读 | §4-A 一致（kind/status 非法显式拒 ✔；id 字典序 ✔） |
| 6 | list_tasks | project_id*, filters{owner?, status?}, cursor?, limit? | 无门（只读） | 只读 | §4-B 一致 |
| 7 | search_evidence | project_id*, query*, mode?(hybrid\|semantic\|exact), exact?{field(5 枚举), value}, filters{source_type?, tags?(显式拒), valid_at?, min_valid_until?}, top_k?(1..=1000, 缺省 20), threshold?(0..=1) | 无门（只读） | 只读；SUPERSEDED 不入池 | §5 朴素实现一致（降权×0.5/告警/tie-breaker ✔）；**E_STATE（资料库绑定）未实现**（代码注释自认，M2-b）；游标信封留 M2-b（契约已记） |
| 8 | draft_response_section | project_id*, section_id*, instructions?(**可选**), source_requirements?, evidence_ids?, generator?(auto\|mock\|real\|external), content?, project_version? | **MatrixReady+（Validating 起拒 → PA-002 死锁点，tools.rs:1895）** + 章节树非空 + 服务层 readiness 七 Blocker | ikey=(section_id, instructions_hash[, ext content_hash])；**登记在业务 checkpoint 之后**（PA-008）；同键重放 deduplicated | §6 主体一致（RESTRICTED/draft_note/quota ✔）；**instructions 实现可选（契约必填）、spec:{title,order,…} 建章路径与 style_profile 完全未实现**（MCP-016） |
| 9 | validate_bid_document | project_id*, scope?(all\|sections), section_ids?(scope=sections 必填), rules_profile?(standard\|dark_bid——dark_bid 仅告警), project_version? | Drafting+（guard） | 重复校验幂等（已 Validating no-op） | §7 一致（#187 四桶/ok 语义/版本步进回传 ✔）；**无条件推进 Drafting→Validating = #243/PA-001 在 MCP 面的确认**（MCP-004，P0） |
| 10 | export_bid_document | project_id*, format*(docx\|pdf), output_path*(工作区相对，拒绝对/..)， profile{dark_bid?, bidder_name?, project_name?, bid_amount?, price_breakdown?, duration_days?, validity_days?, deposit_amount?, legal_rep_gender?}, options?(未支持→告警), project_version? | Drafting+\|Exported（guard）+ 服务层三前置门 + #180 摘要绑定 | ikey=(project, content_version, format, dark_bid, output_path, decision_fp)；重放零 IO | §8 一致（决策指纹入键 ✔）；**out.precheck 硬编码 {blockers:0, waived:[]}——waived VF 清单永不回填**（MCP-017） |
| 11 | get_bid_project_state | project_status?（**调试注入参数——契约未记载**） | 无门 | 只读 | **§0 承诺的"各阶段完成度/DIRTY 章节/BLOCKER 计数"全部未实现**——只回 {status, allowed_next_tools, terminal}（MCP-007，P2） |
| 12 | save_version | file_path*(绝对路径), message*(非空) | 无门（不走项目状态机，§SV ✔） | mang-snap 单一真源 | §SV 一致；**唯一无围栏的文件面写工具**（任意绝对路径 → 向上找 .mang，找不到用文件所在目录建 .mang/history）（MCP-008，P3） |
| 13 | check_bid_quality | project_id* | MatrixReady+（guard）+ 章节树非空 | 只读诊断 + 报告落盘 | §9 一致（叶清单/B3 评分/needs_human ✔）；summary 扩展 dup_pairs/char_deviation_pct 未记档（加法） |
| 14 | build_outline | project_id*, target_total_chars?(1000..=5000000), max_reqs_per_leaf?(1..=50), project_version? | **仅 RequirementsExtracted**（重复调用 E_STATE ✔ §10） | 推导确定性 + 状态机不重入 | §10 参数/状态一致；**无任何覆盖处置门禁或告警 = #244/PA-004 MCP 面确认**（MCP-005，P1） |
| 15 | register_evidence | project_id*, kind*(7 枚举或中文), title*, source_file_id*, subject_name*, scope?, valid_from?, valid_until?, locator?(docx_paragraph\|pdf_page_text), client?, amount_wan?, completed_on?, fields?, project_version? | **仅 MatrixReady\|EvidenceBound**（Drafting 起 E_STATE） | 语义位查重（同 key 同指纹幂等重放）；tier 服务端裁决 | §11-A 主体一致（#185/#217 ✔，旧参数显式拒 ✔）；**契约"状态: MATRIX_READY+"过诺——实现窗口窄**（MCP-003，P2） |
| 16 | bind_evidence | project_id*, ev_id*, requirement_ids*(非空), bid_open_date*, project_version? | **仅 MatrixReady\|EvidenceBound** | 重复绑定幂等重放；E_PARTIAL 带 data | §11-B 一致（R-03 禁绑 ✔）；同 MCP-003 窗口问题 |
| 17 | record_review_gate | project_id*, gate*(4 枚举), decision*(2), receipt*, findings?[{severity(3), code*, message*, section_id?}], project_version? | MatrixReady+（全状态；域层按门细分窗口） | 同门重复 Approved 先比摘要（#180） | §12 一致（reviewer 显式拒 ✔、receipt 验签在服务层 ✔、四门 ✔） |
| 18 | record_addendum_waiver | project_id*, file_id*, reason*(非空白), receipt*, project_version? | FilesImported+ 全部状态（Draft 拒 E_STATE） | 追加式审计日志 | §16 一致（principal 显式拒 ✔、件级 receipt ✔） |
| 19 | set_matrix_override | project_id*, requirement_id*, status*(RESPONDED\|NOT_APPLICABLE\|NOT_RESPONDED\|PARTIAL), deviation?(NOT_APPLICABLE 必填), receipt*, project_version? | MatrixReady+（全状态） | 覆写后立即重算矩阵 | **契约完全没有本工具**（§0 说 24；无 §20；唯一资料是 schema.rs 描述与 851 行测试）（MCP-001，P2） |
| 20 | score_price | method?(benchmark\|lowest\|average\|对象), coefficient?, max_score?, budget?, deviation?(ratio\|price\|rate\|对象)+deduct 族, bid_price?, others? | **无路由无状态门**（纯计算器 ✔ §13） | 纯函数 | §13 一致 |
| 21 | score_section | project_id*, section_id*, project_version? | MatrixReady+（guard）+ 叶+已起草 | 只读 | §14 一致（#186 降级标记/#224 摊分/unmapped ✔） |
| 22 | score_document | project_id*, bid_price?, others?, 价格口径参数?, project_version? | MatrixReady+（guard）+ 章节树非空 | 只读 | §15 一致（mappings 报告/contributions/notes ✔） |
| 23 | get_document_chunks | project_id*, file_id*, from?(缺省 0), size?(1..=200, 缺省 50) | **无门**（route_project 而已——契约 §17 承诺的 E_STATE 窗口外未实现）（MCP-006，P3） | 只读 | §17 参数/分页/越界拒一致；E_STATE 门缺 |
| 24 | submit_block_marks | project_id*, file_id*, marks*[{from,to,verdict(4 枚举),reason?}]（非空，整集替换）, requirements?[{block_idx, statement(域层拒空), category?(7 枚举), mandatory?}], project_version? | **仅 FilesImported\|RequirementsExtracted**（建纲后永久锁标） | 校验全过后原子落库（set_block_coverage + checkpoint） | §18 一致（区间/越界/重叠/越区间校验 ✔、拒绝零副作用 ✔） |
| 25 | get_coverage_report | project_id* | **无门**（Draft 也可调——契约 §19 承诺的 E_STATE 窗口外未实现） | 只读 | §19 聚合口径一致（前 50 段/合并区间 ✔）；**无解析产物文件静默跳过（FAILED 件在聚合面不可见）**（MCP-019，P3） |

**通用面结论**：① 所有 `project_version` 入参均为可选（opt_u64，None=跳过）——契约 §0/各工具 in 无 `?` 标记暗示必填（MCP-009，P2，与 PA-009 跨进程无锁叠加）；② 契约 §0 的通用 `idempotency_key` 参数**任何工具都不接受**（幂等全靠内部键：external_key/sha256/ikey）（MCP-010）；③ 参数校验全部在 tools.rs 手写（req_str/opt_*），schema.rs 的 inputSchema 是纯文档面（无运行时 JSON-Schema 校验）——未见 schema 与手写校验漂移实例，但 schema.rs:105-109 引用的 `tests/mcp_protocol.rs` 漂移护栏测试**不存在**（MCP-018）。

---

## 2. 逐文件审计记录

### 2.1 src/tools.rs（4310 行，头号目标——9 段读全）

- **职责**：25 工具的参数解包 + 路由 + guard_tool + check_version + 转发 BidService + 契约 out 组装；#219 二步抽取合并的 MCP 半边；评分联结器 build_scorer。
- **公开符号**：`invoke`（crate 内分派表）、`req_str`/`opt_str`/`opt_u64`/`opt_str_array`/`route_project`/`route_guard`（coverage_tools 复用）；其余 25 个工具函数私有。
- **结构**：分派表 30-77 → 参数助手 79-214 → 逐工具实现。create/import/extract/get_matrix/list_*/search/draft/check_quality/score_*/export/validate/build_outline/register/bind/review/addendum_waiver/matrix_override/save_version。
- **正面确认**：
  - import 围栏顺序正确：词法 `..`/越界预拒先于项目路由与状态机（382-424），失败件 E_PARTIAL 不吞成功面（581-590）✔；
  - 旧参数显式拒纪律一致（register_evidence 3787-3795 拒 source_file/source_sha256/fingerprint；record_review_gate 4017-4023 拒 reviewer；record_addendum_waiver 4240-4046 拒 principal；set_matrix_override 4131-4137 拒 principal/operator）——**Agent 无法经参数自报身份/tier**；
  - draft external 互斥裁决前置到路由之前（1859-1883，参数面错误零副作用）；
  - export 幂等键含 decision_fp（3331-3350）✔ #117；
  - score_document 的 #224 多对多聚合与 inert 账本行归因（2536-2615）实现细致。
- **问题**：见 MCP-001~019 中 tools.rs 相关项；关键行号：draft 幂等登记 2074-2076（PA-008）、export 幂等登记 3416（同族）、validate 转发 3526-3528（PA-001）、build_outline 唯一门禁 3642-3653（PA-004）。

### 2.2 src/schema.rs（616 行，全读）

- **职责**：ToolCall/ToolOutcome/ToolReply serde 契约（手写 Serialize 保 `ok` 布尔）+ tools/list 的静态描述 + 逐工具 inputSchema（draft-07 子集）。
- **正面**：ToolReply 手写 serde 保 ok 布尔与 data 键名（58-101）；inputSchema 逐工具与 tools.rs 手写校验对读一致（required 集合抽查 create/import/draft/bind/review 均吻合）。
- **问题**：MCP-018（注释声称的 `tests/mcp_protocol.rs` required⊆properties 漂移护栏不存在——仅 protocol.rs 内联测试查 inputSchema 在场）；schema 对 `get_coverage_report` 的 describe 写「**全覆盖才允许建纲**」（579 行）——与 PA-004（build_outline 无门禁）直接矛盾，tools/list 对外部 Agent 虚假宣传一道不存在的门（并入 MCP-005 证据）。

### 2.3 src/protocol.rs（592 行，全读）

- **职责**：MCP 协议层（#137）：JSON-RPC 2.0 + initialize/tools/list/tools/call；双通道错误映射（工具错 → isError:true + 契约 JSON；协议错 → -32700/-32600/-32601/-32602/-32603/-32002）。
- **正面**：握手序（未 initialize → -32002）、通知静默、id 回显合法性、batch 拒、版本协商回显/回退、structuredContent 增益面、content[0].text 与 dispatch_json 同一序列化真源——协议层实现质量高，内联单测覆盖 12 个面。
- **问题**：**538-546 行测试注释自认线格式错误码不一致**——wire 序列化（serde tag SCREAMING_SNAKE_CASE）把 `ENotFound` 发成 `"E_NOT_FOUND"`，而契约 §0 与 `ToolError::code()` 都是 `"E_NOTFOUND"`（MCP-002，P2，CONFIRMED）；`serde_json::to_string(reply).unwrap_or_default()`（291/296）——序列化理论失败时空文本 + isError 语义漂移（P3 信息性，并入 MCP-013）。

### 2.4 src/cursor.rs（408 行，全读）

- **职责**：S-4 游标分页核心：FNV-1a 指纹（工具名+project_id+过滤参数，0x1f 分隔/0x00 缺席标记）、base64(JSON{v,offset,fp}) 编解码、limit 1..=1000 显式拒、page 映射、offset>total fail-closed。
- **正面**：fail-closed 纪律完备（坏 base64/坏 JSON/坏版本/错指纹/负 offset/缺字段/截断全拒）；offset==total 合法空尾页；指纹防拼接歧义有测试。
- **问题**：MCP-011——**一致性只覆盖"缩水"方向**：两页之间底层列表**增长**（如 MatrixReady 态并发 register/bind 使 list_evidence/矩阵行前插）时 offset 仍合法 → 静默滑窗（跳行/重行），契约 §0"偏移越界（底层列表已变化）→ E_UNPROCESSABLE"只兑现一半（游标不内嵌 total/列表版本）。P3。

### 2.5 src/coverage_tools.rs（284 行，全读）

- **职责**：#219 三工具：get_document_chunks（块窗口+verdict 回显）、submit_block_marks（整集替换+域校验+原子落库）、get_coverage_report（全文件聚合）。
- **正面**：find_parsed 按 sha256 join（块身份 = (file_id, block_idx)，与 #217 locate_content 同口径）；submit 校验全部前置（validate_marks/validate_submitted——域层再拦 statement 空/category 七值/越区间）→ 拒绝零副作用；size 上限 200 防上下文吃爆。
- **问题**：get_document_chunks/get_coverage_report 用 `route_project` **不走 guard_tool**（MCP-006——契约 §17/§19 的 E_STATE 窗口承诺落空；allowed_tools 表中对两工具的登记是死条目）；get_coverage_report 无解析产物文件 `continue` 静默跳过（MCP-019）；get_document_chunks `from>=total && total>0` 才拒——total==0 时任意 from 返回空集（微瑕，并入 MCP-006 备注）。

### 2.6 src/lib.rs（134 行，全读）

- **职责**：模块声明 + TOOL_NAMES 单一真源（**25**）+ dispatch/dispatch_with/dispatch_json/tools_schema。
- **问题**：MCP-001——TOOL_NAMES 注释自述"= 25"（52 行），而契约 §0 说 24 且无 set_matrix_override 章节；dispatch 未知工具返回 E_NOTFOUND（与 protocol 层 -32602 分工已有注释说明，行为一致）。

### 2.7 src/error.rs（104 行，全读）

- **职责**：与 bid-domain 共用错误词典；`EState/EConflict/ENotFound/EUnprocessable/EPartial{what,item_errors,hint,data}/UnimplementedInternal`；From<domain ToolError> 单向转换。
- **问题**：MCP-002（serde tag 使 ENotFound 线格式 = "E_NOT_FOUND" ≠ 契约 "E_NOTFOUND"）；MCP-014（契约 §0 的 E_AUTH/E_RATE/E_INTERNAL 三个码不存在——save_version 的注释自认"契约枚举无 E_INTERNAL，统一 E_UNPROCESSABLE"）；UNIMPLEMENTED_INTERNAL 变体现在无发射点（25 工具全实装，保留为骨架纪律，非问题）。

### 2.8 src/state.rs（70 行，全读）

- **职责**：进程级 `LazyLock<Mutex<BidService>>` 全局态（BID_WS 持久化打开 / BID_FS_ROOT 围栏根 / 纯内存）；锁中毒 `into_inner` 自恢复；request_id 进程内单调（重启归零）。
- **问题**：54-55 行注释「一次 panic 不终止传输面」**误导**——bin/stdio.rs 与 bin/server.rs 均无 catch_unwind（全仓 grep 无命中），panic 直接穿透 main 杀死进程；中毒恢复分支在生产不可达（只为测试服务）（MCP-013）；BID_WS 打开失败 panic = 宿主连接崩溃（PA-012 已登记，MCP 面确认）。

### 2.9 src/transport.rs（14 行，全读）

- **职责**：仅 `Transport` trait（recv/send）抽象。
- **结论（问题 9 答案）**：**子进程退出/异常恢复的真实处理不在本文件**——实际 stdio 生命周期在 `src/bin/stdio.rs`（逐行裸 JSON）与 `src/bin/server.rs`（McpSession JSON-RPC）：stdin 读错 → eprintln+break（退出码 0，无错误面）；stdout 写错 → break；**无 panic 捕获、无超时、无重启**——工具内 panic/长时间 LLM 生成（60s×2 重试）直接表现为客户端连接挂起或断开；state.rs 的 open 失败 panic 只在启动期。`src/bin/a8.rs` 为 CLI 验收壳（非 MCP 面，服务层直连+本地围栏）。

---

## 3. 契约 vs 实现差异表

### 3.1 契约多承诺（实现缺失/过诺）

| # | 契约条目 | 实现现状 | 影响条目 |
|---|----------|----------|----------|
| 1 | §0 工具面 24 | 实现 25（set_matrix_override 未入契约） | MCP-001（反向：实现多一个） |
| 2 | §0 错误码 `E_NOTFOUND` | wire 发 `E_NOT_FOUND` | MCP-002 |
| 3 | §0 `E_AUTH`/`E_RATE`/`E_INTERNAL` | 枚举中不存在 | MCP-014 |
| 4 | §0 写操作接受 `idempotency_key` | 无任何工具接受该参数 | MCP-010 |
| 5 | §0 get_bid_project_state 读"各阶段完成度/DIRTY 章节/BLOCKER 计数" | 只回 status/allowed_next_tools/terminal | MCP-007 |
| 6 | §3 scope:"all"\|file_ids[] | 仅 "all"（file_ids 属 M2 未实现且契约未标注） | MCP-015 |
| 7 | §5 E_STATE（项目无企业资料库绑定） | 无绑定概念（代码注释自认 M2-b 补） | 低危（已在 §5 注中半承认） |
| 8 | §6 instructions 必填；`section_id? \| spec:{title,order,parent_id?,toc_ref}`；`style_profile?` | instructions 可选；spec 建章路径、style_profile 完全未实现 | MCP-016 |
| 9 | §8 out.precheck.waived:[VF-*] | 硬编码 `{"blockers":0,"waived":[]}` | MCP-017 |
| 10 | §11-A/§11-B 状态"MATRIX_READY+" | 实际仅 MatrixReady\|EvidenceBound（Drafting 起 E_STATE） | MCP-003 |
| 11 | §17 get_document_chunks E_STATE（窗口外） | 无状态门（任意状态可调） | MCP-006 |
| 12 | §19 get_coverage_report E_STATE（窗口外） | 无状态门（DRAFT 也可调） | MCP-006 |
| 13 | §0/各工具 in 的 project_version（无 `?` → 暗示必填） | 全部可选（None=跳过检查） | MCP-009 |
| 14 | §0 游标"偏移越界（底层列表已变化）→ E_UNPROCESSABLE" | 只拦 offset>total（缩水）；增长滑窗不拒 | MCP-011 |

### 3.2 实现有、契约未记录的行为（加法面为主）

| # | 行为 | 位置 |
|---|------|------|
| 1 | **set_matrix_override 整工具**（四重签域 receipt + 覆写即重算 + 审计面） | tools.rs:4102-4215（无契约章节） |
| 2 | get_bid_project_state 的 `project_status` 注入参数（dry-run 探测任意状态的 allowed_tools） | tools.rs:226-244 |
| 3 | import 响应携带 `input_integrity` 块与 `parse_summary`（#218 乱码可见性） | tools.rs:540-574 |
| 4 | create 重放回 `deduplicated:true` | tools.rs:291-301 |
| 5 | extract 的 ZERO_REQUIREMENTS/ENGINE_PATH/UNCOVERED_BLOCKS/AGENT_MANDATORY_OFFCATEGORY 告警（好行为，部分在 §3 有记） | tools.rs:827-864 |
| 6 | draft 响应的 `quota` 块（target/written/remaining/min_segments/progress_pct） | tools.rs:2000-2035 |
| 7 | check_bid_quality summary 的 dup_pairs/dup_max_similarity/char_total/char_target/char_deviation_pct | tools.rs:2243-2258 |
| 8 | search_evidence hits 附带 kind/title/status/valid_until 台账直通字段 + trace.engine | tools.rs:1748-1753 |
| 9 | get_compliance_matrix 行附带 `blocking`/`category_key` 字段 | tools.rs:977-1001 |
| 10 | search_evidence filters 未知键显式拒（fail-closed 好行为） | tools.rs:1509-1520 |

---

## 4. 问题记录

```
ID: MCP-001
严重程度: P2
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/lib.rs + docs/bid-mcp-contract.md
代码行号: lib.rs:53-81（TOOL_NAMES=25，注释 52 行"= 25"）；契约 §0:17"工具面 **24**"
函数/结构体/接口: TOOL_NAMES / tools_schema
问题描述: 契约与实现的工具面数量错位：#226 set_matrix_override（否决项死锁人工出口，receipt 四重绑定 matrix_override:{pid}:{req}:{status}:{理由sha}、覆写即重算矩阵）完全未写入契约——无 §20、无参数/错误面记载；契约仍宣称 24 工具。tests/dispatch_contract.rs:30 断言 25（与实现一致、与契约矛盾）。
调用链: 外部 Agent ← tools/list（25 条）↔ docs/bid-mcp-contract.md（24 + 无该工具语义）
触发条件: 外部 Agent 以契约为唯一参考编程（按 code 分支、按 §0 工具面枚举）
实际影响: 契约基准失真：Agent 不知道存在该人工出口（否决项死锁的解法）、不知道其 receipt 签域格式；审计对账基线漂移。
复现步骤: 对比 docs/bid-mcp-contract.md 全文（grep set_matrix_override 零命中）与 bid_mcp::tools_schema()["tools"] 长度 25。
当前测试: set_matrix_override_tool.rs（851 行，含 receipt/门/审计面）；dispatch_contract.rs:30 断言 25。
缺失测试: 契约-实现工具面数量一致性护栏（断言契约章节数 == TOOL_NAMES.len()）。
建议方案: 契约补 §20（set_matrix_override 全语义：参数/receipt 签域/E_STATE 窗口/覆写重算/审计回显），§0 工具面改为 25。
证据: lib.rs:79-81:
    // #226 层 1：否决项死锁的人工出口
    "set_matrix_override",
];（TOOL_NAMES: [&str; 25]）
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
```

```
ID: MCP-002
严重程度: P2
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/error.rs
代码行号: error.rs:12-14（#[serde(tag="code", rename_all="SCREAMING_SNAKE_CASE")] + ENotFound 变体）；佐证 protocol.rs:538-546 测试注释与断言
函数/结构体/接口: ToolError::ENotFound（线格式）/ ToolError::code()
问题描述: E_NOTFOUND 的线格式与契约字面量不一致：serde SCREAMING_SNAKE_CASE 把 ENotFound 发成 "E_NOT_FOUND"，而契约 §0 与 ToolError::code() 均为 "E_NOTFOUND"。E_STATE/E_CONFLICT/E_UNPROCESSABLE/E_PARTIAL 四码恰好两种口径重合，唯独 NotFound 分叉。协议层测试自认"预先存在的不一致，#137 只透传不改"。
调用链: 任意工具 E_NOTFOUND → ToolReply::error → serde 序列化 → dispatch_json/MCP content[0].text + structuredContent 的 error.code = "E_NOT_FOUND"
触发条件: 调用方按契约 §0 以 error.code=="E_NOTFOUND" 分支处理"目标不存在"
实际影响: 客户端按契约分支会漏接全部 NotFound 错误（误入默认/错误分支）；契约稳定字面量承诺被打破。
复现步骤: MCP 调 get_compliance_matrix{project_id:"P-404"} → 解析 content[0].text 的 error.code → 得 "E_NOT_FOUND" ≠ 契约 "E_NOTFOUND"。
当前测试: protocol.rs:542/546 断言 "E_NOT_FOUND"（把不一致钉死为现状）；无契约字面量断言。
缺失测试: 断言线格式 code ∈ 契约 §0 八码集合的护栏测试。
建议方案: ToolError::ENotFound 加 #[serde(rename="E_NOTFOUND")]（或全变体显式 rename 对齐契约），同步更新 protocol.rs 测试；契约 §0 附线格式表。
证据: protocol.rs:538-546（测试注释）:
    // 注意：wire 序列化是 SCREAMING_SNAKE_CASE 变体名 → "E_NOT_FOUND"，
    // 而 ToolError::code() 返回契约字面量 "E_NOTFOUND"（预先存在的不
    // 一致，#137 只透传不改——旧路径线格式不动是验收项）
    assert_eq!(resp["result"]["structuredContent"]["code"], "E_NOT_FOUND");
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCP-003
严重程度: P2
模块: bid-mcp × bid-domain
文件路径: bidcore/crates/bid-domain/src/lib.rs（allowed_tools）+ bidcore/crates/bid-mcp/src/tools.rs
代码行号: lib.rs:187-205（MatrixReady|EvidenceBound 含 register/bind）、206-223（Drafting 清单无 register/bind）、224-239（Validating+ 同）；draft.rs:786-801（首次起草 MatrixReady→EvidenceBound→Drafting 连跳两态）
函数/结构体/接口: ProjectStatus::allowed_tools / register_evidence / bind_evidence
问题描述: 契约 §11-A/§11-B 承诺登记/绑定窗口为"MATRIX_READY+"（对照 §12 record_review_gate 同措辞=全后续状态可用），实现仅 MatrixReady|EvidenceBound——首次 draft_response_section 即把项目推到 DRAFTING（连跳 EvidenceBound→Drafting），此后 register_evidence/bind_evidence 全部 E_STATE，且状态机无回退边 → 起草后发现证据缺口（如 validate 报 E_QUAL_UNMET 需补登记）在 MCP 面无补救通道（唯一出口是 set_matrix_override 逐行覆写）。
调用链: build_outline → register/bind（窗口内）→ draft（advance_project_to_drafting 两步）→ 再 register/bind → guard_tool E_STATE
触发条件: Agent 在起草后（或任何进入 DRAFTING 的时点）需要补登记/补绑证据
实际影响: 证据补录通道在 DRAFTING 起永久关闭；外部 Agent 全链（mcp_scaffold_tools 顺序）不受影响，但任何回补工作流失效——与 PA-002 死锁族同型（窗口收窄+无回退边）。
复现步骤: ① create→import→extract→build_outline→draft 一章（项目 DRAFTING）② 调 register_evidence → E_STATE「当前状态 DRAFTING 不允许调用 register_evidence」③ 枚举 DRAFTING 的 allowed_tools：无任何证据面工具。
当前测试: mcp_scaffold_tools.rs 按正确顺序走全链（register 先于 draft）——绿≠覆盖回补场景；无"起草后补绑"负测试。
缺失测试: DRAFTING 态 register/bind 的 E_STATE 断言 + 契约窗口对账测试。
建议方案: 二选一：域层把 register/bind 扩窗到 Drafting（证据补录与矩阵重算本就幂等）或契约改口径为"MatrixReady|EvidenceBound"并写明起草即封窗。
证据: lib.rs:206-211（Drafting 清单）:
    Drafting => &[
        "get_coverage_report", "get_compliance_matrix", "search_evidence",
        "get_bid_project_state", "draft_response_section", "validate_bid_document",
        "export_bid_document", "check_bid_quality",
        // G3：PreExportConfirm 门在 Drafting+ 窗口内
        "record_review_gate", ...（无 register_evidence/bind_evidence）
审计置信度: CONFIRMED
状态分类: 当前仍存在（契约与实现不一致 + 工作流刚性）
```

```
ID: MCP-004
严重程度: P0
模块: bid-mcp × bid-tools（继承 #243/PA-001，MCP 面确认）
文件路径: bidcore/crates/bid-mcp/src/tools.rs + bidcore/crates/bid-tools/src/service.rs
代码行号: tools.rs:3469-3528（guard 允许 Drafting+ validate；3519-3523 唯一前置=章节树非空）→ service.rs:1372-1389（validate_document_with_reviewer 无条件推进）
函数/结构体/接口: validate_bid_document / BidService::validate_document
问题描述: MCP 面确认 PA-001 全链：validate 工具在部分章节未完成时仍可成功调用并**无条件推进 DRAFTING→VALIDATING**（+content_version、checkpoint 落盘）；工具返回的是数据面 ok:false（E_SECTION_EMPTY findings）而非错误码——无 DRAFT_INCOMPLETE 等价物，调用方极易当作普通失败查询继续编排；随后 draft_response_section 被 guard E_STATE 拒（PA-002 死锁），无 MCP 恢复工具。
调用链: MCP validate_bid_document → guard_tool（Drafting 允许）→ check_version → svc.validate_document → 纯读报告 → advance_project_status(Validating) → auto_checkpoint → db.json
触发条件: DRAFTING 态存在未起草/空章节时调用全局校验（#243 真实客户端日志场景）
实际影响: 项目持久化锁进 VALIDATING；修订-重校验-导出闭环在 MCP 面断裂；唯一恢复=手改 db.json。
复现步骤: 见 PA-001（S1-S3 场景表）；MCP 面入口即本条调用链。
当前测试: validate_tool.rs:217 断言"DRAFTING → VALIDATING 推进"（把问题行为锁死为预期，PA-003）。
缺失测试: 部分章节校验后断言 status/version 不变；DRAFT_INCOMPLETE 错误码。
建议方案: 见 PA-001（完成度门禁/显式推进语义分治）；MCP 契约 §7 需同步写明推进条件。
证据: service.rs:1379-1386:
    let steps: &[bid_domain::ProjectStatus] = match cur {
        bid_domain::ProjectStatus::Drafting => &[bid_domain::ProjectStatus::Validating],
        _ => &[],
    };
    for to in steps { self.store_mut().advance_project_status(*to)?; }
    self.auto_checkpoint()?;
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCP-005
严重程度: P1
模块: bid-mcp（继承 #244/PA-004，MCP 面确认 + 补充）
文件路径: bidcore/crates/bid-mcp/src/tools.rs + bidcore/crates/bid-mcp/src/schema.rs
代码行号: tools.rs:3618-3653（build_outline 唯一门禁=requirements 非空，无 coverage 检查/无告警）；schema.rs:579（describe("get_coverage_report")="……全覆盖才允许建纲"）；coverage_tools.rs:224-228/277-283（next 仅人话指引）
函数/结构体/接口: build_outline / tools_schema describe
问题描述: PA-004 在 MCP 面确认：建纲入口对块覆盖状态零消费、零告警（get_coverage_report 是独立只读工具，submit 的 next 与 extract 的 UNCOVERED_BLOCKS 告警都只是文本提示）。**补充发现**：tools/list 的 get_coverage_report 描述文本宣称「全覆盖才允许建纲」——把不存在的门当能力广告给外部 Agent（Agent 可能据此跳过自查覆盖，以为系统会拦）。
调用链: MCP build_outline → route_guard → requirements.len()>0 → prepare_outline_and_matrix → MATRIX_READY（未判定块静默进入起草面）
触发条件: 导入+抽取后存在未判定块即建纲
实际影响: #244 要堵的盲区（未判定块静默进入起草）在 MCP 面完全敞开，且工具元数据反向误导。
复现步骤: ① import→extract（不 submit_block_marks）② get_coverage_report → complete:false ③ build_outline → 成功 MATRIX_READY（无告警）④ tools/list 里 get_coverage_report 描述声称建纲被门禁。
当前测试: coverage_tools.rs/agent_mandatory_gate.rs 均无"未覆盖时 build_outline 被拦/告警"断言。
缺失测试: 未判定块 → build_outline 阻断或至少 warning 的正反测试；tools/list 描述与实现行为一致性断言。
建议方案: ① build_outline 前置消费 coverage（未判定→阻断+点名，已排除→放行）；② 在修门前先改 schema.rs:579 描述为真实语义（「建议全覆盖后建纲——当前不强制」）。
证据: tools.rs:3642-3652:
    let n_reqs = svc.store().requirements().len();
    if n_reqs == 0 { return Err(...E_STATE...) }   // ← 唯一门禁
schema.rs:579:
    "get_coverage_report" => "只读：覆盖报告（#219——哪些块没归属 + 未覆盖区间 + 各类 verdict 块数；全覆盖才允许建纲）",
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCP-006
严重程度: P3
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/coverage_tools.rs
代码行号: coverage_tools.rs:67（get_document_chunks 仅 route_project）、236（get_coverage_report 仅 route_project）；对照 bid-domain lib.rs:167-186（两工具在 FilesImported/RequirementsExtracted 的 allowed_tools 登记为死条目）
函数/结构体/接口: get_document_chunks / get_coverage_report / route_project
问题描述: 契约 §17/§19 承诺两工具在状态机窗口外返回 E_STATE；实现只做项目路由（无 guard_tool），任意状态可调（含 DRAFT——get_coverage_report 在 DRAFT 返回「还没导入文件」ok 响应而非 E_STATE；get_document_chunks 在 MATRIX_READY+/EXPORTED 均可取块）。allowed_tools 表中相应登记不起作用（工具不查表）。
触发条件: 在 FILES_IMPORTED/REQUIREMENTS_EXTRACTED 之外的状态调用两工具
实际影响: 契约错误面过诺（宽松方向，只读工具，无状态副作用）；与 submit_block_marks（真有 guard）窗口口径不一致——Agent 在建纲后不能再补标但还能读块/看报告（只读可看不可改，语义尚自洽）。
复现步骤: EXPORTED 态调 get_document_chunks{project_id,file_id} → 200 正常返回块窗口（契约预期 E_STATE）。
当前测试: coverage_tools.rs 测试均在合法窗口内。
缺失测试: 窗口外调用断言（按契约应 E_STATE 或按新口径应放行——先对齐口径）。
建议方案: 契约删两工具的 E_STATE 行（改为"任何阶段可调（只读）"）或补 guard——鉴于只读性质建议改契约。
证据: coverage_tools.rs:67:
    route_project(svc, &project_id, TOOL)?;
    let file = find_parsed(svc, &file_id, TOOL)?;   // 无 guard_tool
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
```

```
ID: MCP-007
严重程度: P2
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/tools.rs
代码行号: tools.rs:226-244（get_bid_project_state 实现）；对照契约 §0:17
函数/结构体/接口: get_bid_project_state
问题描述: 契约 §0 把本工具定位为断点恢复入口并承诺「读项目状态、各阶段完成度、DIRTY 章节、BLOCKER 计数」——实现只返回 {status, allowed_next_tools, terminal}：无各阶段完成度、无 DIRTY 章节清单、无 BLOCKER 计数（DIRTY/BLOCKER 需另调 get_compliance_matrix/check_bid_quality 拼装，且矩阵过滤后才能看到 blocking 计数）。另：未写入契约的 project_status 注入参数允许调用方带入任意假想状态探测 allowed_tools（只读 dry-run，无持久化影响，但契约零记载）。
触发条件: 外部 Agent 断点恢复时按契约向本工具要 DIRTY 章节/BLOCKER 计数
实际影响: 恢复入口信息量不足，Agent 须多工具拼装；project_status 参数为隐性行为面（若宿主把用户输入直传 args，可被用来探测状态机全貌——信息泄露面极低）。
当前测试: dispatch_contract.rs:66-92（status/allowed/terminal + 非法注入拒）；无完成度/DIRTY 断言（因不存在）。
缺失测试: 不适用（缺的是实现/契约对齐决策）。
建议方案: 契约 §0 修订为实口径（status+allowed_next_tools+terminal）并把 project_status 参数记档（标注调试/dry-run 语义）；或扩实现补 DIRTY/BLOCKER 摘要（对断点恢复价值高）。
证据: tools.rs:239-243:
    Ok(json!({
        "status": status.as_str(),
        "allowed_next_tools": status.allowed_tools(),
        "terminal": status.next_allowed().is_empty(),
    }))
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
```

```
ID: MCP-008
严重程度: P3
模块: bid-mcp × mang-snap
文件路径: bidcore/crates/bid-mcp/src/tools.rs + bidcore/crates/mang-snap/src/lib.rs
代码行号: tools.rs:1781-1825（save_version）；mang-snap lib.rs:103-114（find_root：从任意绝对路径向上找 .mang，找不到用文件父目录）
函数/结构体/接口: save_version / mang_snap::find_root / mang_snap::save
问题描述: save_version 是 25 工具中唯一无围栏的文件面工具：file_path 接受任意绝对路径（仅限 .md/.txt/.markdown）；快照根=向上最近的 .mang，找不到则用**文件所在目录**——即可以在工作区之外的任意可写目录创建 .mang/history 并把任意可读文本文件内容复制进去（读+写双越界），与 #183/#189 导入导出围栏纪律形成反差。契约 §SV 如实记载了"绝对路径+向上找根"行为，故属"按文档实现但纪律不一致"。
触发条件: Agent 对工作区外的文本文件调 save_version（如 /home/user/notes/secret.md）
实际影响: 单用户桌面语境下危害有限（进程本有用户权限）；但构成：① 工作区外落盘面（.mang/history 目录树污染）；② 任意可读文本→持久副本的信息搬运面。多租户/服务化部署下升级为 P2。
当前测试: save_version_tool.rs（126 行，含绝对路径/空 message/非文本拒）。
缺失测试: 工作区外路径行为断言（当前行为未被钉死）。
建议方案: save_version 增加 BID_WS 前缀校验（file_path 须在工作区内，越界 E_UNPROCESSABLE 指路）；契约 §SV 同步收紧。
证据: mang-snap lib.rs:103-113:
    pub fn find_root(file: &Path) -> PathBuf {
        let mut dir = file.parent()...;
        loop {
            if dir.join(".mang").is_dir() { return dir; }
            match dir.parent() { Some(p) => dir = p.to_path_buf(),
                None => return file.parent()... }   // ← 找不到 .mang → 文件所在目录为根
审计置信度: CONFIRMED
状态分类: 当前仍存在（契约如实记载，纪律缺口）
```

```
ID: MCP-009
严重程度: P2
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/tools.rs（全部写工具）
代码行号: 各工具 opt_u64("project_version", ...)（如 381/664/1851/2476/3246/3621/3804/3903/4026/4128/4248/143）；bid-domain lib.rs:579-591（check_version Option=跳过）
函数/结构体/接口: opt_u64 / DomainStore::check_version
问题描述: 契约 §0 及各写工具 in 形（project_version 无 ? 标记）暗示乐观并发为强制入参；实现全部可选（缺省=None=跳过检查）。MCP 客户端可完全不带版本号串行写——结合 PA-009（无跨进程文件锁、双宿主 last-writer-wins），MCP 面没有任何强制并发防护；进程内靠全局 Mutex 串行化兜底。
触发条件: 客户端省略 project_version 连续调用写工具（最常见的外部 Agent 行为——契约未把"必须回传"写成硬约束）
实际影响: 并发/重试场景丢更新无检测（响应虽携带最新版本，但不传就不校验）；#243 类"报错但状态已变"（PA-007 半提交）叠加时调用方无从感知版本分叉。
当前测试: 各工具测试带版本冲突面（E_CONFLICT 断言在场）但均显式传版本——"不传即跳过"行为有测试（cursor 面缺省断言）而无线程交错场景。
缺失测试: 双 dispatch_with 并发（同进程双 &mut 不可行——至少跨进程双宿主场景标注）。
建议方案: 契约把写工具 project_version 升为必填（破坏性变更需排期）或实现层对"同工具高频跳版本写"发 warning；跨进程锁见 PA-009 建议。
证据: lib.rs:578-579:
    /// 乐观并发检查（契约 §0：project_version 过期 → E_CONFLICT）。
    /// M1 工具调用方传 `None` 表示跳过检查（后端内部步骤）；外部调用必须传。
（"外部调用必须传"仅是注释纪律，tools.rs 全部 opt_u64 不强制）
审计置信度: CONFIRMED
状态分类: 文档与实现不一致（契约暗示必填 vs 实现可选）
```

```
ID: MCP-010
严重程度: P3
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/tools.rs
代码行号: 全文件无 idempotency_key 参数（grep 零命中）；幂等实现点：create external_key（291-305）、import sha256（服务层）、draft ikey（1914-1928）、export ikey（3346-3357）
函数/结构体/接口: 全部写工具
问题描述: 契约 §0 通用约定「写操作接受 idempotency_key；同一 key 重复调用返回首次结果」——实现不存在该通用参数；幂等全部走内部键（external_key/sha256/(section,instructions_hash)/(project,version,format,path,decision_fp)）。语义上等价覆盖了主要写路径，但通用承诺面（任意写工具可带 key）未兑现，且 draft/export 的幂等登记不落盘（PA-008）使"重复调用返回首次结果"在崩溃后不成立。
触发条件: 客户端按契约给写工具传 idempotency_key 字段
实际影响: 参数被静默忽略（未知参数不拒——工具只挑已知键读）；崩溃后重试非幂等（PA-008）。
建议方案: 契约 §0 改写为逐工具幂等键表（如实），或实现通用 key 通道；未知参数是否显式拒可一并定纪律（当前 search filters 拒未知键、其余工具静默忽略——口径不一）。
证据: 契约 §0:12「写操作接受 `idempotency_key`」 vs tools.rs 任何 req_str/opt_str 清单中无该键。
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
```

```
ID: MCP-011
严重程度: P3
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/cursor.rs
代码行号: cursor.rs:249-267（slice 仅拦 offset>total）；101-109（CursorPayload 无 total/列表版本字段）
函数/结构体/接口: cursor::slice / CursorPayload
问题描述: 游标分页一致性只 fail-close"列表缩水"（offset>total→E_UNPROCESSABLE）；两页之间列表**增长**（如 MatrixReady 态 register/bind 使证据或矩阵行在当前 offset 之前插入）时 offset 仍合法→静默滑窗（跳过新插入行或重复已读行），调用方无感知。契约 §0「偏移越界（底层列表已变化）→ E_UNPROCESSABLE」只兑现缩水半边。
触发条件: 分页遍历期间同状态内发生插入类写操作（register_evidence/bind 在 MatrixReady/EvidenceBound 与 get_compliance_matrix/list_evidence 并发可用）
实际影响: 遍历完整性静默受损（12k 行矩阵翻页中 bind 一批证据→行序偏移）；确定性行序保证"同快照可重放"，不保证"跨变更不漏"。
当前测试: cursor_pagination.rs 有"缩水拒"（limit 超 total 面）无"增长滑窗"行为测试。
缺失测试: 翻页中途插行的行为断言（拒或至少告警）。
建议方案: CursorPayload 内嵌签发时的 total（或 content_version），解码时 total 变化即拒（代价：正常穿插写操作会强制重取——需产品权衡）；或契约如实记"偏移分页仅防缩水，遍历期间勿写"。
证据: cursor.rs:250-256:
    if window.offset > total as u64 {
        return Err(ToolError::EUnprocessable { what: "cursor offset=… 越界（当前过滤后总数 …——底层列表已变化…" }) }
（仅 >，增长方向的列表变化不触发任何分支）
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCP-012
严重程度: P3
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/tools.rs
代码行号: tools.rs:913-914 + 939-955（filter_str 取值后直接字符串比较，无枚举校验）；对照 list_evidence 1089-1096（parse_evidence_kind/parse_evidence_status 显式拒）
函数/结构体/接口: get_compliance_matrix 的 filters.category/status
问题描述: get_compliance_matrix 的 category/status 过滤值不校验合法域：status 拼错（"RESPONDED "尾空格/"responded"小写/"FOO"）→ 静默 0 行空页（total=0、next_cursor=null），与 cursor.rs 的 fail-closed 纪律和 §4-A/§4-B（非法 kind/status 显式拒）不一致。category 同理（任意字符串→0 行）。
触发条件: 客户端传非法 category/status 过滤值
实际影响: "空结果"与"过滤无命中"不可区分——Agent 可能误判为"该类目无条款"；对账面静默出错。
当前测试: cursor_pagination.rs 过滤面用合法值；bad_input_matrix.rs 不覆盖过滤值枚举。
缺失测试: 非法 status/category → E_UNPROCESSABLE 断言。
建议方案: 复用 parse_evidence_status 同款 serde 枚举校验（status 四值）+ category 双形态（契约键∪中文七类）白名单。
证据: tools.rs:949-952:
    if let Some(s) = &filter_status {
        if status != s { continue; }   // 无枚举校验——任意串直接比
    }
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCP-013
严重程度: P3
模块: bid-mcp（transport/protocol/state 生命周期）
文件路径: bidcore/crates/bid-mcp/src/bin/stdio.rs + src/bin/server.rs + src/state.rs + src/protocol.rs
代码行号: stdio.rs:16-33 / server.rs:26-48（读错 eprintln+break、写错 break、无 catch_unwind）；state.rs:54-55（「一次 panic 不终止传输面」注释）；protocol.rs:291/296（to_string unwrap_or_default）；全仓 grep catch_unwind 零命中
函数/结构体/接口: main（两 bin）/ with_global / call_result
问题描述: 问题 9 的正面回答：① 工具执行期 panic 无任何隔离——直接穿透 main 终止 stdio 子进程（客户端只见连接断开，无 JSON-RPC 错误应答）；state.rs 的锁中毒恢复分支生产不可达（无捕获者），其注释误导；② stdin/stdout IO 错误一律 break 静默退出（退出码 0）；③ 无请求超时：真实 LLM 起草（60s×2 重试，服务层口径）期间单线程分派循环 + 全局 Mutex 使后续请求（含只读）全部排队——宿主表现为假死；④ protocol 层 reply 序列化理论失败 → content[0].text 空串（isError 语义漂移，理论不可达）。
触发条件: 服务层/工具任何 panic（或索引越界类 bug）；长 LLM 生成期间的并发请求
实际影响: MCP 会话整体崩溃无错误面；单请求阻塞全线；对 Claude Desktop 类宿主表现为"工具无响应"。
当前测试: mcp_sdk_e2e.rs（65 行，官方 SDK 拉起子进程 happy path）；无 panic 注入/超时行为测试。
缺失测试: 工具 panic → 宿主可观察错误（需 catch_unwind + -32603 应答）的行为测试。
建议方案: bin 层把 session.handle_line 包 catch_unwind（panic→单请求 -32603 + 保留传输面）；考虑读侧 recv 超时或请求级超时线程；修正 state.rs 注释。
证据: server.rs:39-45:
    match session.handle_line(trimmed, bid_mcp::dispatch) {
        Some(reply) => {
            if writeln!(stdout, "{reply}").is_err() { break; }
            let _ = stdout.flush();
        }
        None => continue,
    }（无任何 panic 捕获/超时/错误应答通道）
审计置信度: CONFIRMED
状态分类: 当前仍存在（state.rs 注释与实现不一致）
```

```
ID: MCP-014
严重程度: P3
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/error.rs + docs/bid-mcp-contract.md §0
代码行号: error.rs:14-38（枚举六变体）；tools.rs:1807-1812（save_version 注释自认）
函数/结构体/接口: ToolError
问题描述: 契约 §0 承诺错误码体系含 E_AUTH / E_RATE / E_INTERNAL——ToolError 枚举均不存在（无鉴权面/无限流面/内部错误统一进 E_UNPROCESSABLE）。save_version 的注释如实自认「契约枚举无 E_INTERNAL，统一 E_UNPROCESSABLE 并在 what 里带核心层码位」。
实际影响: 契约错误码八分之三是空承诺；按 code 分支的客户端不会收到这三个码（当前无功能依赖它们，属文档债）。
建议方案: 契约 §0 标注三码为预留（未实现）或删去；未来加鉴权/限流时同步。
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
```

```
ID: MCP-015
严重程度: P3
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/tools.rs
代码行号: tools.rs:653-658（scope != "all" → E_UNPROCESSABLE "暂不支持……M2 扩展"）；schema.rs:170（enum ["all"]）
函数/结构体/接口: extract_requirements 的 scope
问题描述: 契约 §3 in 写 `scope:"all"|file_ids[]` 且只在 granularity 上标注"保留位未实现"——file_ids 子集过滤同样未实现但契约未标注（实现显式拒并指路 M2）。
实际影响: 按契约传 file_ids 的调用收到 E_UNPROCESSABLE（报错文案诚实），属可恢复摩擦；契约对账面失真。
建议方案: 契约 §3 给 file_ids 加与 granularity 同款"保留位未实现"标注。
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
```

```
ID: MCP-016
严重程度: P3
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/tools.rs + schema.rs
代码行号: tools.rs:1850（instructions = opt_str）；schema.rs:295-308（required 仅 project_id+section_id；无 spec/style_profile 位）
函数/结构体/接口: draft_response_section
问题描述: 契约 §6 的三处形状偏差：① instructions 实现为可选（缺省 None 直传服务层；幂等键按空串哈希）而契约列为必填；② `section_id? | spec:{title, order, parent_id?, toc_ref}` 的 spec 建章路径完全未实现（无参数位、无代码路径——章节只能来自 build_outline）；③ style_profile? 未实现。schema 与实现一致地偏离契约（两处一起漂）。
实际影响: 形状级契约失真；Agent 传 spec/style_profile 被静默忽略（未知参数不拒）。
建议方案: 契约 §6 修订为实形状（instructions 可选、删 spec/style_profile 或标预留）。
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
```

```
ID: MCP-017
严重程度: P3
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/tools.rs
代码行号: tools.rs:3412（"precheck": { "blockers": 0, "waived": [] } 硬编码——注释自认"双前置门已过（失败路径在 Err 面）"）
函数/结构体/接口: export_bid_document out.precheck
问题描述: 契约 §8 out 承诺 precheck:{blockers:0, waived:[VF-*]}——waived（已登记 WAIVED 的 VF 清单）在成功路径永不回填（恒空数组）；调用方无法从导出回执看到"哪些 BLOCKER 是靠豁免放行的"。
实际影响: 豁免可见性缺口（审计面弱化）；功能影响低（validate 可查 findings）。
建议方案: 从审核记录取 WAIVED findings 填充 waived 数组，或在契约 §8 删该字段。
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
```

```
ID: MCP-018
严重程度: P3
模块: bid-mcp（测试有效性）
文件路径: bidcore/crates/bid-mcp/src/schema.rs
代码行号: schema.rs:105-109（注释声称"schema 与实现的漂移由 tests/mcp_protocol.rs 的 required ⊆ properties 一致性断言兜底"）
函数/结构体/接口: tools_schema / input_schema
问题描述: 注释引用的 tests/mcp_protocol.rs **不存在**（tests/ 目录 38 个文件无此名）；全 tests 无 required⊆properties 断言——实际护栏仅 protocol.rs 内联测试的 inputSchema 在场性 + dispatch_contract.rs 的 len==25。inputSchema 本身无运行时校验（纯文档面——参数强制全在 tools.rs 手写）。
实际影响: schema-实现漂移无真实护栏（当前抽查未见漂移实例：required 集合与 req_str 清单吻合）；注释误导审计与维护者。
建议方案: 补一个 schema 一致性测试（每工具 inputSchema.required 逐个用"缺该参数"的负调用验证确实 E_UNPROCESSABLE——与 bad_input_matrix.rs 合并）；修正注释。
审计置信度: CONFIRMED
状态分类: 当前仍存在（注释引用不存在的测试）
```

```
ID: MCP-019
严重程度: P3
模块: bid-mcp
文件路径: bidcore/crates/bid-mcp/src/coverage_tools.rs
代码行号: coverage_tools.rs:251-252（`let Some(f) = parsed_by_sha.get(...) else { continue };`——无解析产物文件静默跳过）；269（complete = total_blocks>0 && covered==total）
函数/结构体/接口: get_coverage_report
问题描述: 聚合报告对无解析产物的文件（parse FAILED 件、或纯内存态丢 parsed 的场景）静默 continue——失败件不进 files 清单、不进 total_blocks；全库只有失败件时 complete=false（total=0 分支输出"还没导入文件"误导文案——实际是导入了但全失败）。import 的 E_PARTIAL/INPUT_FILE_NOT_READY 是第一现场，但覆盖报告作为"#219 建纲前检查入口"对坏件不可见。
触发条件: 导入含 parse FAILED 件后查覆盖报告
实际影响: Agent 在覆盖报告里看不到坏件（以为块覆盖是完整口径）；与 PA-004 叠加（建纲又不查覆盖）时双重盲区。
当前测试: coverage_tools.rs 无失败件聚合场景。
缺失测试: 含 FAILED 件的覆盖报告断言（文件应列出并标注不可覆盖/失败）。
建议方案: 无解析产物文件在 files[] 中以显式条目呈现（total_blocks:0 + parse_failed 标记），next 指引提及修复重传。
证据: coverage_tools.rs:251-252:
    for (fid, sha) in tender_files.iter() {
        let Some(f) = parsed_by_sha.get(sha.as_str()) else { continue };
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

**PA/PB 交叉核实结论**（问题 3/7/8 的专项回答）：
- **PA-001/PA-002（#243 死锁）在 MCP 面 CONFIRMED 并补充**：入口 tools.rs:3469-3528（guard 只查章节树非空）→ service.rs:1379-1387 无条件推进；死锁关闭点 tools.rs:1895（draft guard，Validating 不在 allowed_tools lib.rs:224-239）；MCP 面无任何回退/恢复工具（grep force_status 仅测试）——**补充：MCP-003 显示死锁族比 PA 报告更宽——register/bind 在 DRAFTING 起即封窗（契约"MATRIX_READY+"过诺），第一次起草后就进入"证据不可补录"半死锁**。
- **PA-004（#244）MCP 面 CONFIRMED + 加重**：build_outline 零覆盖消费（3642-3653）；加重项=schema.rs:579 tools/list 虚假广告"全覆盖才允许建纲"（MCP-005）。
- **PB-005（陈旧矩阵）MCP 面佐证**：get_compliance_matrix 走 matrix_view（过滤失效绑定）而导出门/validate 走持久化 build_matrix 全量——同库两口径在 MCP 只读面即可观察（tools.rs:931/969-976 注释自认"过期证据绑定已被 matrix_view 过滤"）。
- **PB-006（映射无 MCP 写面）CONFIRMED**：TOOL_NAMES 25 个工具无任何评分映射写工具；score_section/score_document 经 build_scorer（tools.rs:2853-2982）只**读** scoring_mappings 账本——upsert_scoring_mapping 在 MCP 面不可达。
- **Agent 越权面（问题 8）结论**：**MCP 面已封死自报通道**——register_evidence 无 tier/指纹参数（旧参数显式拒，tools.rs:3787-3795）；三人工门工具（record_review_gate/record_addendum_waiver/set_matrix_override）全部 receipt 必填 + 裸身份参数显式拒；issue_approval_receipt 不在任何 MCP 工具面（grep 确认）。残余信任面（设计使然）：① submit_block_marks 的 verdict/mandatory 判定（Agent 可把真实条款标 noise 使其在重跑 extract 时被丢弃——#219 架构的固有信任面）；② draft content=external 手稿直注；③ save_version 无围栏路径（MCP-008）；④ PB-001/PB-005 所述 tier/效期不进裁决面（MCP 面无 verified 消费入口，与其说越权不如说门禁缺席）。
- **PA-008 幂等登记时序在 MCP 面 CONFIRMED**：tools.rs:2074-2076（draft）/3416（export）——record_idempotent 经 store_mut 不触发 checkpoint，且幂等键不含 content_version（`let _ = version;` 死变量）。

---

## 5. 结论摘要

1. **工具面真源 = 25**（TOOL_NAMES），契约宣称 24：set_matrix_override（#226，含 receipt 四重绑定语义）整体未入契约（MCP-001）；另 get_bid_project_state 的 project_status 注入参数、import 的 input_integrity 块等未记档行为见 §3.2。
2. 契约过诺 14 项（§3.1）：其中影响外部 Agent 编程的实义项——E_NOTFOUND 线格式分叉（MCP-002）、get_bid_project_state 断点恢复信息缺口（MCP-007）、register/bind 窗口收窄（MCP-003）、§17/§19 E_STATE 门缺（MCP-006）、idempotency_key 通用参数不存在（MCP-010）。
3. 试点结论交叉核实：PA-001/PA-002（P0 死锁）在 MCP 面 CONFIRMED 且死锁族**加宽**（register/bind 起草即封窗）；PA-004 CONFIRMED 且 tools/list 描述虚假广告加重；PB-005/PB-006 在 MCP 只读面得到佐证；PA-008 幂等登记时序 CONFIRMED。
4. Agent 越权面：MCP 面身份/tier/receipt 伪造通道全部封死（显式拒 + 服务端裁决 + HMAC）；残余信任面为 submit_block_marks 语义判定、external 手稿、save_version 无围栏路径。
5. 协议/传输层质量高（JSON-RPC 双通道错误映射、握手序、游标 fail-closed 纪律完备），但 stdio 生命周期无 panic 隔离/无超时（MCP-013），transport.rs 仅是 14 行 trait、真实生命周期在两个 bin。
