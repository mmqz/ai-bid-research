# 业务不变量专项审计（business-chain-audit.md）

> 审计员：整合员。基线 6105beb（只读）。方法：六面不变量逐条**回到源码复核**（每条关键断言均本轮 Read 验证行号，非转抄 agent 报告），守卫测试与变异结果引用 test-quality-audit.md / mutation-report.md。
> 裁定口径：PASS（域层+服务层一致生效）/ SPLIT（双口径分裂）/ DEGRADED（主语义在、传播面缺）/ BROKEN（关键面失效）。

---

## 1. 状态机不变量（10 态线性 + guard 表 + 无回退）

**源码复核**（本轮逐行）：
- 10 态枚举：`ProjectStatus`@bid-domain/src/lib.rs:104-118（Draft→FilesImported→RequirementsExtracted→MatrixReady→EvidenceBound→Drafting→Validating→Reviewing→ReadyToExport→Exported）。
- 线性前向唯一真源：`next_allowed`@lib.rs:122-136——每个状态恰好一条出边（Exported 空 :134）。
- 工具准入裁决表：`allowed_tools`@lib.rs:150-256——**Validating|Reviewing|ReadyToExport**（:224-239）不含 `draft_response_section`；`register_evidence`/`bind_evidence`/`import_tender_files`/`extract_requirements`/`build_outline` 仅出现在早期态（:153-205）。
- 越级防护：`guard_tool`@lib.rs:562-575（不在清单 → E_STATE，零副作用）。
- 推进：`advance_project_status`@lib.rs:601-617——同态幂等 no-op（:603-604）、跳级/回退 E_STATE（:606-611）、推进即 content_version+1（:612-615）。**无任何回退 API**（`force_status` 仅测试用）。

**复核结论（与 P0-1 结构对照）**：域层不变量实现干净——「validate 无条件推进 Drafting→Validating」（service.rs:1372-1389，:1377 报告产出后 :1379-1386 无条件执行 steps）+「Validating 态无 draft」（lib.rs:224-239）+「只进不退」（lib.rs:601-617）三者复合，即构成 P0-1 的**不可恢复结构**：起草期误调全局校验 → 剩余章节永久 E_STATE → 唯一恢复 = 手改 db.json。另复核 A-11：`advance_project_to_exported`@service.rs:1438-1458 从 Drafting 一步连跳四级（:1442），使 Validating/Reviewing/ReadyToExport 三个"人工阶段"在正常流程中从不驻留——状态机图与真实生命周期脱节。

**守卫测试**：state_machine_matrix.rs（9 测试）、status_enum_matrix（12）、lib 内嵌 8；M6 变异（删除 next_allowed 限制）KILLED——但杀伤测试在 bid-tools/tests/draft_section.rs:513（**迟杀**：bid-domain 自己的 21 个相关测试全绿，守卫在 tools 层——结构性脆弱）。
M1 变异（删除推进）KILLED，但**语义缺口**：现有测试只断言「validate 后推进发生」（validate_tool.rs:200/217/242/554），无任何测试断言「校验失败/章节未完成时不得推进」——生产缺陷「无条件推进」本身（#243）无测试约束。

**裁定：域层 PASS / 服务层编排 BROKEN（P0-1、A-11、A-23 三同型死锁）**。

---

## 2. 矩阵派生不变量（幂等、blocking、version bump 时机）

**源码复核**：
- 派生单一真源：`derive_matrix_rows`@matrix.rs:134-198——`ev_by_req` 只按**绑定存在**建索引（:141-144）；行状态 `has_ev/has_sec`（:168-179：(true,true)=Responded、(true,false)=Partial、其余 NotResponded）；override 优先（:171-183）；**blocking = (is_mandatory(类别∈{资格条件,否决项}) ∨ agent_mandatory) ∧ NotResponded**（:186-187；agent_mandatory@matrix.rs:115-118 = marked_by=="agent" ∧ mandatory==true，#229 接线）。
- 幂等：`build_matrix`@matrix.rs:218-252——全量重派生，`rows != self.matrix` 才 bump content_version（:237-243）；首建推进 MATRIX_READY（:246-250）；状态门 Draft/FilesImported 拒（:220-225）；空条款拒（:226-230）。
- **双口径分裂（本轮重点复核）**：`build_matrix` 取**全量** `evidence_bindings`（:232-233，不看证据状态）；`matrix_view`@matrix.rs:347-356 过滤 Active|Expiring 才计入。两者共用 `derive_matrix_rows`，分歧仅在绑定集——**导出门（export_blockers :320-331 读存量 `self.matrix`）消费前者，get_compliance_matrix 工具面消费后者**。
- 重算触发点：仅 `bind_evidence_batch`（service.rs:961-987 循环内）与 `set_matrix_override`→`rebuild_matrix_for_override`（service.rs:731-753:747-750；matrix.rs:263-265 = build_matrix 同语义）。**换版（register_evidence 置 Superseded evidence.rs:506-538）、时间过期、set_requirements/replace_requirements 均不触发重算**。
- 覆写门：`set_matrix_override`@matrix.rs:272-314（非 Human 拒 :278-286；NOT_APPLICABLE 必须给理由 :287-293；REQ 存在性 :294-298；receipt 四重签域 service.rs:602-611）。

**守卫测试**：M3 变异（blocking 恒 false）KILLED 4 红（matrix.rs:425 `agent_mandatory_wires_blocking_outside_category_face` 等）；`matrix_view_filters_dead_evidence_bindings`@matrix.rs:607 只钉**视图**过滤——**无「导出门 vs 视图」一致性测试、无「换版后 export_blockers」回归**（A-01 测试缺口）。

**裁定：SPLIT**——派生幂等/blocking/覆写门 PASS（M3 实证）；build（全量绑定）vs view（活证据）双口径使导出门弱于工具面（A-01），重算触发点缺换版/时间面（A-01/A-02）。

---

## 3. 证据不变量（四态 + 只降不升 + tier + 绑定门）

**源码复核**：
- 登记幂等/换版：`register_evidence`@evidence.rs:485-568——语义四元组 (kind,title,subject,scope) 查重（:488-498）；**same_payload = fingerprint+valid_from+valid_until 三比**（:500-502，**不含 verification_tier/locator/excerpt** → A-12：FILE_EXISTS 无法经重登记升级 CONTENT_MATCHES，同指纹重放被 no-op 吞 :503-504）；换版单向降级旧件 Superseded（:506-538，Expired→Superseded 亦合法）。
- 只降不升：`degrade_to` 状态机 + `apply_validity`@evidence.rs:575-602（幂等：同态迁移被拒；翻转 ≥1 条才 bump 版本 :596-600）。**生产零调用（本轮 grep 复核）**：`apply_validity` 调用方 = impact.rs（孤岛模块内部）+ 6 处 tests —— bid-mcp/bid-tools 生产路径无任何触发（A-02）。唯一真实翻转点 = bind 时刻（evidence.rs:625-634）。
- 绑定门：`bind_evidence`@evidence.rs:608-659——开标日即时裁决（:625-634）→ **`can_bind` 只查 status**（:635-643：Active/Expiring 放行，Expired/Superseded E_UNPROCESSABLE）→ 幂等重放（:644-650）。**不查 verification_tier、不查证据内容与条款的匹配**（P0-2 第二半边）。
- tier 面：登记侧服务端裁决@service.rs:918-931（locator 对账 → CONTENT_MATCHES；无 locator → FILE_EXISTS，fingerprint=sha256:文件 sha :925-930）；四态互斥校验 evidence.rs:402-414。**下游消费（本轮 grep 复核）：matrix.rs 派生（:168 只看绑定）、draft.rs `check_evidence_bindable`（:199-209 只看 status）、readiness.rs `req_has_usable_evidence`（:712-717 只看绑定+status）、validate.rs（无 tier 规则）、export.rs（无检查）——全部零消费（A-14）**。

**守卫测试**：M2 变异（删除 can_bind 门）KILLED 2 红（evidence.rs:845 `gt13_expired_evidence_forbidden_to_bind`、:887 `register_replay_noop_and_renewal_supersedes`）；evidence_content.rs（tier 登记裁决）4 测试——**无「tier 应影响下游门」契约测试（因下游未接线）**。

**裁定：登记面/绑定状态门 PASS（M2）/ 有效性传播 BROKEN**——时间翻转无入口（A-02）+ tier 零消费（A-14）+ bind 不查内容匹配（P0-2）。

---

## 4. 审批不变量（receipt 验签、四道门、审核失效传播）

**源码复核**：
- 验签：`verify_approval_receipt`@principal.rs:223-289——六段线格式（:229-246）、Human-only（:248-251）、24h 时效窗+60s 未来容差（:253-259）、scope 进 MAC 输入（:261-281，他域 receipt → BadSignature）、HMAC-SHA256 常量时间比对（:274-281）。**消息构成 :274-277 = `RECEIPT_VERSION|actor_id|actor_type_s|scope|issued_at`——无内容摘要**（A-03：宿主为门 X 签发 receipt 后，Agent 持 receipt 在 24h 内改章节内容再过门，record_review 落账绑**当时**的 digest，与宿主签发时人所见无关）。
- 四道门与门序：`record_review`@review.rs:274-469——矩阵门状态集合 MatrixReady/EvidenceBound/Drafting（:303-315，VALIDATING+ 拒）；大纲门需矩阵门先 Approved（:318-332）；导出确认门需前两门（:335-348）；AddendumWaiver 正交无门序（:349-352）。拒审依据门（:364-371，Rejected 必带 Blocker）。幂等分支先比 digest（:384-414：内容变更不走幂等、落新 REV；既有 RT 一律不复位 :395-397）。
- 导出裁决：`export_allowed`@review.rs:601-643——三门遍历（:604-611）+ **最后一条 PreExportConfirm Approved 的 reviewed_content_digest ≠ 当前 sections_digest → 拒**（:612-630）+ RT 全 DONE（:631-641）。

**特别复核一：评分条件变更后相关审核是否真的失效？**
**结论：不失效（零失效面）。** 证据链（本轮源码）：
1. `upsert_scoring_mapping`@scoring.rs:1347-1457 变更时 bump 的是 `content_version`（:1435-1439 更新路径、:1452-1455 新增路径——#224 修复面）；
2. `sections_digest`@outline.rs:621-640 的字段清单**显式排除** requirement_ids/target_chars/status/dirty/content_attempt（:604-610 注释自述），更不含 scoring_mappings/matrix_overrides——评分面/覆写面任何变更**不改 digest**；
3. `export_allowed` 的失效判据**只有 digest 比对**（review.rs:617-619）——content_version 不参与（:604-610 注释明确解释了为什么不绑全局版本：防误报）；
4. 章节面：upsert_scoring_mapping 不碰 sections（无 dirty 传播——dirty 唯一置位点 outline.rs:809 reopen_section）；
5. 矩阵面：upsert_scoring_mapping 不触发 build_matrix。
→ **ScoringMapping 变更 invalidate 的面 = 空集**（不使三门任何一道失效、不打脏章节、不重算矩阵；唯一影响 = 乐观并发版本号 + score_* 派生输出）。

**特别复核二：ScoringMapping 变更 invalidate 哪些？set_matrix_override 变更呢？**
- ScoringMapping：如上，**零 invalidate**（A-10 实证）。
- set_matrix_override：service.rs:731-753 验签→写覆写→`rebuild_matrix_for_override`（矩阵真实重算，#226 出口有效）→checkpoint；但 **sections_digest 不变**（覆写不属于六字段）→ PreExportConfirm 审核不失效 → export_allowed 照旧放行（A-10：人工所批的「整体标书」与实际响应面不一致无需重审；缓解 = 覆写自身的 receipt 留痕）。
- 对照（真实失效通道只有两类）：① 章节树六字段变更（set_section_content/set_outline/draft 重写）→ digest 变 → 审核失效；② 上游 replace_requirements → **digest 不变 → 审核不失效**（补遗链断点，见 callgraph 链 4）。

**守卫测试**：M5 变异（验签旁路）KILLED（set_matrix_override_tool.rs:491 `forged_receipt_is_rejected`）；review.rs `pre_export_digest_binds_section_content`（:1133）、`reapproval_after_content_change_creates_new_record`（:1197）；receipt 攻击样本 set_matrix_override_tool.rs:513/558/627（跨行挪用/Agent 身份/错状态——#239）。**无「receipt 与内容绑定」攻击样本测试（A-03）、无「评分面变更后审核应失效」测试（A-10——语义本身缺失）**。

**裁定：验签面 PASS（M5）/ 失效传播 DEGRADED**——digest 绑定真实（#180）但覆盖面过窄（六字段，A-10）；receipt 不绑内容（A-03）；RT 无产品出口（A-23：拒审即锁导出，review.rs:494-583 域方法完备、MCP 零工具——本轮 grep 复核确认）。

---

## 5. 导出门不变量（1a 存量矩阵 vs 过期证据脱钩）

**源码复核**（三道门逐行，service.rs:1161-1348）：
- 门 1a@service.rs:1178-1181 → `check_export_ready`@matrix.rs:334-343 → `export_blockers`@matrix.rs:320-331：读**存量** `self.matrix`（构建时点快照）；矩阵未构建同样拒（:322-325，诚实状态）。
- 门 1b@service.rs:1181 → `export_allowed`@review.rs:601-643（见 §4）。
- 门 1c@service.rs:1182-1207 → `writing_acceptance_findings`@validate.rs:502-530：E_SECTION_EMPTY（必写叶无正文/未 DRAFTED）+ E_CHARS_BELOW_MIN（正文 < target×80%）——**导出时点独立复算、不消费 reviews/RT/gate_approved**（validate.rs:482-501 门禁语义注记自述，代码实现一致：函数只读 store.sections() :503-527）。

**过期证据脱钩链（A-01/A-02 复核，导出门最薄弱面）**：
1. 换版：`register_evidence`（MCP 工具合法路径，MatrixReady+）→ 旧 EV 置 Superseded（evidence.rs:511），**绑定不迁移不清除**；
2. 重算缺口：换版不触发 build_matrix（触发点仅 bind/override）——存量矩阵中该 REQ 的 has_ev 仍真；
3. 时间过期：`apply_validity` 生产零调用（A-02）→ valid_until 已过的 EV status 仍 Active；
4. 门 1a 消费存量矩阵 + 三道门**均不复查证据状态**（service.rs:1178-1207 无任何证据面检查；validate 规则 8 只在校验工具面，导出门不复算）；
5. 对照面：`matrix_view`@matrix.rs:347-356 过滤 Active|Expiring——用户在 get_compliance_matrix 看到 NOT_RESPONDED+blocking，**导出却放行**。
→ 资格条件/否决项以失效证据响应仍可出标（P0-2 在导出面的残留）；M4 证明门 1c 独立复算模式可行——**门 1a 应同源化（matrix_view 口径）**。

**守卫测试**：M4 KILLED（writing_acceptance_e2e.rs:486——门 1c 有独立复算回归）；validate_tool.rs:445-466 测了 validate 拦截过期证据（手工 apply_validity 翻转），**无 export 拦截用例**；无换版后 export_blockers 回归（A-01 缺口）。

**裁定：门 1c PASS / 门 1a BROKEN（存量+死绑定双口径）/ 门 1b DEGRADED（§4 三缺口）**。

---

## 6. 持久化不变量（store_io 原子性、checkpoint、恢复路径）

**源码复核**：
- 原子性：`StoreFile::save`@store_io.rs:109-121——单文件 `db.json`（域快照+parsed 全量）→ 写 tmp（:117-118）→ `rename` 原子提交（:119）。**无 fsync、无父目录 fsync**（A-13：掉电可失最近若干次写；进程级告警「内存已变更、磁盘未跟上」service.rs:700-704 只在同进程内有效）；**全量写放大**（parsed 含全部块文本，每个写工具调用重写整库，A-13）。
- 版本门/损坏语义：`load`@store_io.rs:96-106 + `migrate`@:125-137——schema_version 高于支持 → SchemaUnsupported 响亮拒；坏 JSON → Corrupt 拒载原文件保留。
- checkpoint：`auto_checkpoint`@service.rs:700-704（每个写操作后调用——本轮在 import/register/bind/review/override/validate/export 各服务方法复核确认）+ 显式 `checkpoint`@:693-698。
- 恢复路径：`BidService::open`@service.rs:630-656 → `StoreFile::load` → `DomainStore::restore`@lib.rs:1100-1401。restore 校验面（本轮全读复核）：9 集合 id 唯一（:1116-1162）、assignments target_id（:1165-1174）、水位 8 前缀（:1189-1212）、RT→FND/REV→FND（:1213-1236）、waiver→file（:1237-1250）、role→file（:1251-1262）、**#224 三查**：mapping→req/sec/ev + 复合键 + 份额域 + 聚合 ≤1.0（:1263-1361）、孤儿集合（:1363-1378）。**未校验（A-05 实证）**：`evidence_bindings`（ev_id∈evidences / req_id∈requirements / 复合键唯一——:1384 原样装入）、`matrix` 行引用、`matrix_overrides`→requirements、`block_marks`/`submitted_requirements` 的 file_id——手改/半截 db.json 的孤儿绑定直接进入 `derive_matrix_rows` 的 ev_by_req → 幽灵「已响应」。
- batch 半态：`bind_evidence_batch`@service.rs:961-987 循环内绑定先落库，build_matrix/checkpoint 失败即整批 Err 而绑定已生效（A-24——错误返回后内存与磁盘不一致窗口）。

**守卫测试**：store_roundtrip.rs（6 测试：往返全等/原子性/版本门/损坏拒载/legacy 加性迁移/双保存字节一致，store_io.rs:139-282 内嵌）；restore 拒载面（review.rs:1289 孤儿 RT 等）。**变异：store_io 未入本轮白名单（mutation-report §4 登记——未变异验证）**；无孤儿绑定 restore 样本（A-05 缺口）。

**裁定：进程级 PASS（原子提交+版本门+拒载语义均有测试）/ 掉电级与快照校验 DEGRADED（无 fsync A-13、restore 漏 bindings/matrix/overrides A-05、写放大）**。

---

## 不变量矩阵总表

| # | 不变量 | 实现位置（本轮复核） | 守卫测试 | 变异结果（mutation-report） | 裁定 |
|---|---|---|---|---|---|
| 1.1 | 10 态线性枚举 + next_allowed 单出边 | lib.rs:104-118 / :122-136 | state_machine_matrix.rs（9）、status_enum_matrix（12） | M6 KILLED（**迟杀**——杀伤在 bid-tools/draft_section.rs:513，域层无自守卫） | 域层 PASS |
| 1.2 | guard_tool 越级零副作用 | lib.rs:562-575 + allowed_tools :150-256 | dispatch_contract.rs（25 工具位串断言） | —（M6 覆盖相邻面） | PASS |
| 1.3 | 状态只进不退（无回退 API） | lib.rs:601-617（force_status 仅测试） | draft_section.rs:513 | M6 KILLED | PASS（域）/ **编排面 BROKEN：P0-1（service.rs:1372-1389 无条件推进）+ A-11（:1438-1458 跳级）** |
| 1.4 | validate 结果参与状态裁决 | service.rs:1377-1386 —— **不存在**（报告与推进脱钩） | 无（M1 语义缺口：只测推进发生，无失败不推进断言） | M1 KILLED（只杀「永不推进」方向） | **BROKEN（P0-1/#243）** |
| 2.1 | 矩阵派生幂等（重跑等价，真变更才 bump） | matrix.rs:218-252（:237-243） | matrix.rs 派生族 + scale_1330 | M3 KILLED（4 红） | PASS |
| 2.2 | blocking = (类别必选 ∨ Agent 必选) ∧ 未响应 | matrix.rs:186-187 + agent_mandatory :115-118 | matrix.rs:425（agent_mandatory_wires_blocking…） | M3 KILLED | PASS（#229/#241 在位） |
| 2.3 | 导出门 1a 与工具面同源口径 | **分裂**：build_matrix :232-233（全量绑定）vs matrix_view :347-356（活证据）；export_blockers :320-331 读存量 | matrix_view_filters_dead_evidence_bindings（:607，只钉视图） | —（无导出门 vs 视图一致性测试） | **SPLIT（A-01）** |
| 2.4 | 矩阵重算时机完备（换版/过期触发） | 触发点仅 bind（service.rs:961-987）/ override（:731-753） | 无换版后 export_blockers 回归 | — | **BROKEN（A-01/A-02 触发缺口）** |
| 3.1 | 证据四态只降不升 + 换版单向 | evidence.rs:506-538 / degrade_to / apply_validity :575-602 | evidence.rs:845/:887 | M2 KILLED | PASS（域层） |
| 3.2 | 时间翻转有生产入口 | **无**——apply_validity 生产零调用（grep 实证）；唯一翻转=bind 时刻 :625-634 | 测试手工充当调用方（store_roundtrip.rs:191 等——掩盖缺口） | — | **BROKEN（A-02）** |
| 3.3 | 绑定门查 tier/内容匹配 | **无**——bind_evidence :635-643 只查 can_bind(status)；派生 :168 只看绑定存在 | 无（下游未接线故无契约测试） | — | **BROKEN（P0-2/#245）** |
| 3.4 | tier 登记裁决 + 幂等键含 tier | 登记 service.rs:918-931 ✓；幂等键 :500-502 **不含 tier/excerpt/locator** | evidence_content.rs 4 测试（登记面） | — | 登记面 PASS / 升级通道 BROKEN（A-12）+ 下游零消费（A-14） |
| 4.1 | receipt 验签（HMAC/scope/时效/Human/常时比对） | principal.rs:223-289 | principal.rs 测 + set_matrix_override_tool.rs:491/513/558/627 | M5 KILLED | PASS |
| 4.2 | receipt 绑定被审内容 | **不绑**——消息 :274-277 无内容摘要 | 无攻击样本测试 | — | **DEGRADED（A-03）** |
| 4.3 | 四道门 + 门序 + 拒审依据 | review.rs:300-371 | review.rs:823（bypassing_gates_typed_reject）/928（rejected_without_blocker） | — | PASS |
| 4.4 | 审核失效传播覆盖条款/评分/覆写面 | digest 六字段 outline.rs:604-640；export_allowed 只比 digest review.rs:612-630 —— 评分映射/覆写/整集替换 REQ **均不失效审核** | pre_export_digest_binds_section_content（:1133，仅章节面） | — | **DEGRADED（A-10——ScoringMapping 变更 invalidate=∅，本轮实证）** |
| 4.5 | BLOCKER→RT 生命周期有产品出口 | 域层完备 review.rs:494-583；**MCP 零工具**（grep 实证） | 域单测有、MCP 贯通无 | — | **BROKEN（A-23——拒审即锁导出）** |
| 5.1 | 门 1a：blocking 清零才可导出（活口径） | check_export_ready matrix.rs:334-343（存量口径） | M3 杀伤面部分覆盖 | — | **门在/口径 BROKEN（A-01+A-02：失效证据脱钩）** |
| 5.2 | 门 1b：三门+digest+RT DONE | review.rs:601-643 | review.rs:766（acceptance_blocker_rt_lifecycle…） | — | PASS（面在）/ 缺口见 4.2/4.4/4.5 |
| 5.3 | 门 1c：写作验收导出时点独立复算 | service.rs:1188-1207 → validate.rs:502-530 | writing_acceptance_e2e.rs:486 + SDK 02-full-pipeline 红（反向证实） | M4 KILLED | **PASS（#242 基线即合并点，验证最强）** |
| 6.1 | 单文件 tmp+rename 原子提交 | store_io.rs:109-121 | store_roundtrip.rs（6 测试） | 未变异（白名单未执行项） | PASS（进程级） |
| 6.2 | rename 前 fsync / 增量写 | **无**（全量写放大） | 无 | 未变异 | DEGRADED（A-13，掉电位） |
| 6.3 | 版本门+损坏拒载+加性迁移 | store_io.rs:96-106/:125-137 | store_io.rs:180/:200/:219 | 未变异 | PASS |
| 6.4 | restore 引用完整性覆盖全部集合 | lib.rs:1100-1401 —— **漏 evidence_bindings/matrix/matrix_overrides/block_marks** | 无孤儿绑定样本 | — | **DEGRADED（A-05）** |
| 6.5 | 写后 checkpoint 全覆盖 | service.rs:700-704（各写方法逐一复核✓）；例外=bind batch 半态 :961-987 | assembly_checkpoint 等 | 未变异 | PASS（A-24 半态窗口除外） |

## 总结论

1. **有真实变异级守卫的不变量**（5/6 直接击杀 + M4）：状态推进存在性、过期证据禁绑、blocking 派生、#242 导出门、receipt 防伪、跳级拒绝——P0/P1 面「测试会红」总体成立。
2. **三处「实现即缺席」的 BROKEN 面**：validate 结果不参与推进（P0-1）、bind/matrix 不查 tier 与内容（P0-2）、覆盖闭环不接（#244，见 callgraph 链 2）——三者共同特征是**域层/规则层无缺陷，接线层缺一根线**。
3. **审批失效传播是本轮最重要的否定性发现**：评分条件（ScoringMapping）变更的 invalidate 面 = **空集**（upsert 只 bump content_version；sections_digest 显式排除评分/覆写/条款面 outline.rs:604-610；export_allowed 唯一判据是 digest review.rs:617-619）——矩阵覆写有自身 receipt 缓解，评分映射则完全无失效信号（A-10 实证）。
4. 导出门薄弱面的准确刻画：**1a 是三道门中唯一消费「构建时点快照」而非「导出时点复算」的门**（1b 消费持久账本、1c 独立复算）——A-01/A-02 的脱钩全部汇聚于此。
