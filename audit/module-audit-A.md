# 模块审计报告 A（Agent A —— 业务核心审计）

范围：bid-domain / bid-tools / bid-kb / bid-grade（/home/z/my-project/ai-bid-audit/bidcore/crates/）
基线：6105beb（只读，未修改任何仓库文件）
方法：47 个 src 文件逐文件全读（生产代码全读、测试模块扫读用于测试映射）；关键结论用 rg 交叉验证消费方（apply_validity / run_review / negative_deviations / ExportGate / LinkageLedger / revision_task 生命周期等的全仓引用 grep）。
共享上下文：audit-work/context-for-agents.md（Lead 已确认 P0-1 #243、P0-2 #245，本报告不重复立项，只登记相邻面）。

---

## A. 文件覆盖清单

状态说明：READ=生产代码逐行读完；READ-MAIN=生产代码读完、纯渲染/测试段扫读。发现数为该文件贡献的发现条目（见 B 节编号）。

### bid-domain/src（19 文件，~16,050 行）

| 路径 | 行数 | 状态 | 发现数 |
|---|---|---|---|
| lib.rs | 1721 | READ | A-04, A-05, A-11(相邻), A-14, A-17 |
| matrix.rs | 639 | READ | A-01, A-14 |
| evidence.rs | 1268 | READ | A-01, A-02, A-12, A-14 |
| scoring.rs | 2387 | READ | A-10, A-18, A-22 |
| review.rs | 1318 | READ | A-03, A-10, A-23 |
| outline.rs | 1385 | READ | A-09 |
| principal.rs | 447 | READ | A-03 |
| export_gate.rs | 169 | READ | A-08 |
| linkage.rs | 181 | READ | A-08 |
| impact.rs | 523 | READ | A-02, A-09 |
| coverage.rs | 404 | READ | — |
| dates.rs | 114 | READ | A-16 |
| locator.rs | 372 | READ | — |
| pricing.rs | 1887 | READ-MAIN | — |
| budget.rs | 529 | READ | — |
| assignment.rs | 485 | READ | A-23（无 MCP 接线） |
| party.rs | 537 | READ | A-15 |
| forms.rs | 886 | READ-MAIN | — |
| param.rs | 799 | READ | A-07, A-19, A-23（无 MCP 接线） |

### bid-tools/src（17 文件 + bin/real_eval.rs，~13,120 行）

| 路径 | 行数 | 状态 | 发现数 |
|---|---|---|---|
| service.rs | 1833 | READ | A-01, A-02, A-11(相邻), A-23, A-24 |
| export.rs | 2103 | READ-MAIN | A-07, A-13(相邻) |
| draft.rs | 1264 | READ-MAIN | A-14 |
| readiness.rs | 1113 | READ | A-04 |
| quality.rs | 921 | READ-MAIN | A-14 |
| tables.rs | 868 | READ-MAIN | A-19 |
| assembly.rs | 656 | READ | A-24(相邻) |
| check.rs | 615 | READ | A-06 |
| llm.rs | 604 | READ-MAIN | A-21 |
| format_spec.rs | 554 | READ-MAIN | — |
| validate.rs | 536 | READ | A-01, A-14 |
| context.rs | 494 | READ | — |
| runner.rs | 362 | READ | — |
| fs_fence.rs | 294 | READ | —（TOCTOU 自登记） |
| store_io.rs | 282 | READ | A-13 |
| placeholder.rs | 261 | READ | — |
| schema.rs | 158 | READ | — |
| lib.rs | 26 | READ | — |
| bin/real_eval.rs | 174 | READ | — |

### bid-kb/src（2 文件 531 行）

| 路径 | 行数 | 状态 | 发现数 |
|---|---|---|---|
| lib.rs | 238 | READ | —（金额查询元/万元换算边界，P3 未立项） |
| shard.rs | 293 | READ | — |

### bid-grade/src（6 文件 1233 行）

| 路径 | 行数 | 状态 | 发现数 |
|---|---|---|---|
| grade.rs | 537 | READ | A-20 |
| selftest.rs | 349 | READ-MAIN | A-20 |
| align.rs | 148 | READ | — |
| main.rs | 74 | READ | A-20(相邻) |
| fixtures.rs | 118 | READ | A-20 |
| lib.rs | 7 | READ | — |

unwrap/expect/panic 非测试分布（rg 统计+逐点抽查）：bid-domain 14 处（多为 rfind 后 position().unwrap()、常量正则、水印断言——抽查全部有前置不变量保护）；bid-tools 30 处（生产面 12 处均为门后 expect / 常量正则，其余在 runner/real_eval 评测壳）；bid-kb 3 处（OnceLock 常量正则）；bid-grade 14 处（CLI/自测壳，含 main.rs `expect("夹具装载失败")`）。**未发现无守卫的生产 panic 路径**。并发面：check.rs std::thread::scope + mpsc 有界 8 线程、结果按片号归位排序（确定性有测试钉死）；BidService 单线程假设（MCP stdio 串行）；无锁原语。

---

## B. 发现清单

> 级别定义见共享上下文。已知 P0-1（#243 validate 无条件推进）/ P0-2（#245 FILE_EXISTS 绑资格条件）不重复立项；A-01/A-11/A-14 是其二阶相邻面。

### A-01【P1｜CONFIRMED】导出门消费存量矩阵（不过滤失效证据），且换版/过期后无重算入口——失效证据响应可过导出闸门

- 位置：bid-domain/src/matrix.rs:218-252（build_matrix 用全量绑定）、:347-356（matrix_view 过滤 Active/Expiring）；bid-domain/src/evidence.rs:485-568（register_evidence 换版将旧记录置 Superseded，不迁移绑定）；bid-tools/src/service.rs:1178-1181（export 门 1a=check_export_ready→存量矩阵）
- 代码摘录：
  ```rust
  // matrix.rs build_matrix —— 与 matrix_view 的唯一分歧：
  let bindings: Vec<(String, String)> = self.evidence_bindings.iter()... // 全量，不看证据状态
  // matrix.rs matrix_view：
  let live_bindings = ...filter(|b| ... matches!(e.status, Active | Expiring))...
  // service.rs export_document 门 1a/1b/1c —— 无任何证据状态检查
  self.store.check_export_ready()?;
  self.store.export_allowed()?;
  let wa = crate::validate::writing_acceptance_findings(self.store());
  ```
- 调用链：register_evidence（换版，MCP register_evidence 工具，MatrixReady+ 合法）→ 旧 EV Superseded、绑定仍在 → bind/set_override 之外的任何路径都不重算 matrix → export_document 门 1a 读 `self.matrix`（构建时点的绑定快照）→ blocking 行已清零 → 放行。
- 触发条件：资格条件/否决项绑定证据 EV-A →（换版或时间过期）→ EV-A 失效 → 不重新 bind（或重 bind 新 EV 后 build_matrix 仍把旧 EV-A 的死绑定计入 has_ev）→ export。
- 影响：**导出门比 get_compliance_matrix 工具面更宽松**——用户在矩阵视图看到 NOT_RESPONDED+blocking，导出却放行；资格条件以失效证据响应仍可出标（废标级风险面，P0-2 在导出面的残留）。validate 规则 8（E_EVIDENCE_EXPIRED）能拦 Superseded（状态即时翻转）但拦不住「时间上已过期而 status 仍 Active」（见 A-02），且导出三道门均不复查规则 8。
- 现有测试：matrix.rs `matrix_view_filters_dead_evidence_bindings`（638）只钉了**视图**过滤，没有测试钉「导出门 vs 视图」一致性，也没有换版后导出的回归测试。
- 建议：导出门 1a 改用 matrix_view 同源派生（或 build_matrix 时同样过滤死证据）；register_evidence 换版路径触发矩阵重算；为「换版后 export_blockers」补契约测试。

### A-02【P1｜CONFIRMED】证据有效期时间翻转在生产链路零入口——apply_validity 全仓仅测试调用

- 位置：bid-domain/src/evidence.rs:575-602（apply_validity）；grep 全仓：bid-mcp/src、bid-tools/src 生产代码零调用（命中全部在 tests/：store_roundtrip.rs:191、readiness_check.rs:198、draft_section.rs:352、validate_tool.rs:466 等手工 `svc.store_mut().apply_validity(...)`）。
- 代码摘录（evidence.rs:606-634）：bind_evidence 携带开标日做**绑定时刻**即时翻转——是唯一翻转点。
- 调用链：MCP 面无任何工具触发 apply_validity；导出/校验/readiness 的「可用证据 = status.can_bind()」全部依赖绑定时刻的陈旧 status。
- 触发条件：证据登记后时间流逝越过 valid_until（或补遗改开标日），期间不再 bind。
- 影响：时间性过期证据长期被当 Active 消费：readiness 判「有证据」放行起草、quality/export 业绩表收录、context.rs 装配进 prompt；R-date 语义（domain-model §2「评估基准日变更→全项目重检」）只实现在孤岛 impact.rs（A-09）。与 A-01 叠加后：时间过期 + 存量矩阵 = 双重放行。
- 现有测试：大量（手工翻转后验证各门），恰好掩盖「无人调用翻转」的缺口——测试自己充当了生产缺失的调用方。
- 建议：validate/export 前置一步 `apply_validity(开标日)`（开标日来自 REQ 参数或 ExportDecision）；或把「按基准日的活算过滤」并入 matrix_view 同源口径。

### A-03【P1｜CONFIRMED】审批 receipt 只绑门名不绑内容——24h 窗口内可对未审内容过门

- 位置：bid-domain/src/principal.rs:223-289（verify：scope=`review_gate:{gate}`+时效+HMAC，消息不含内容摘要）；bid-domain/src/review.rs:373-415（record_review 收到验签过的 principal 后才快照当前 sections_digest——**审核对象由本次调用时的库状态决定，与宿主签发时人看到的内容无关**）。
- 代码摘录（principal.rs 消息构成）：
  ```rust
  format!("{RECEIPT_VERSION}|{actor_id}|{actor_type_s}|{scope}|{now}") // 无内容摘要
  ```
- 对比：同 PR 系的 `matrix_override_scope`（service.rs:602-611）做了四重绑定（project/requirement/status/reason-sha256）——review_gate 门级 receipt 明显未达同一标准。
- 触发条件：宿主为门 X 签发 receipt（人确认了内容 v1）→ Agent 持 receipt 在 24h 内修改章节内容为 v2 → 调 record_review_gate(v2) → 落账 REV 绑 v2 摘要，export_allowed 对 v2 放行。幂等分支的摘要比对（review.rs:391）只防「旧记录复用」，不防「receipt 复用于新内容」。
- 影响：人工审批可被挪用到从未展示给人的内容（错误审批面）；`reviewed_content_digest` 只保证「过门时点有快照」，不保证「人批的就是这份」。
- 现有测试：principal.rs 测了 scope 错配/过期/伪造；review.rs 测了摘要变更→重审；无「receipt 与内容绑定」攻击样本测试。
- 建议：签发面把目标内容摘要并入 scope（如 `review_gate:{gate}:{sha256(sections_digest)}`），或过门时校验 receipt 签发时刻 ≤ 当前 digest 首次出现时刻。

### A-04【P1｜CONFIRMED】逐件 waiver 与「本项目无补遗」全局门共用同一 Gate——无关件确认静默解除补遗缺失阻塞

- 位置：bid-domain/src/lib.rs:734-762（record_addendum_waiver 无条件追加 `ReviewRecord{gate: AddendumWaiver, decision: Approved}`）；bid-tools/src/readiness.rs:1062-1076（missing_addendum ②：无补遗件时 `gate_approved(AddendumWaiver)` 即放行）
- 代码摘录（lib.rs:735-747 注释自认两账同写）：
  ```rust
  // 两道记录职责不同，不能只写一处：
  //   · addendum_waivers = 逐件免责台账
  //   · reviews(AddendumWaiver, Approved) = 门禁消费的放行事实
  ```
- 触发条件：项目只有主件+一份坏「图纸」（非补遗）→ 用户按 UnreadyInput 提示对图纸做 record_addendum_waiver（这是文档化出口）→ `gate_approved(AddendumWaiver)=true` → 「未发现补遗/澄清文件」阻塞（NotProvided）被解除，尽管用户从未确认过「本项目无补遗」。
- 影响：MissingAddendum(NotProvided)（#131/#184 定义的「不知道≠没有」防线）可被语义上无关的输入件 waiver 顺手关掉——带病放行面扩大；审计回看也会把两件事混为一谈。
- 现有测试：addendum_waiver 面测试覆盖了逐件确认与 Gate 放行各自路径，但没有「waiver 非补遗件→NotProvided 门被隐式打开」的反例测试。
- 建议：全局「无补遗确认」与逐件 waiver 拆成两个 Gate（或 record_addendum_waiver 仅当目标件 is_addendum_file 时写 AddendumWaiver 门）。

### A-05【P1｜CONFIRMED】restore 快照校验漏 evidence_bindings / matrix / matrix_overrides / block_marks——孤儿绑定可制造幽灵「已响应」

- 位置：bid-domain/src/lib.rs:1100-1401（restore 校验清单）
- 代码摘录：校验了 9 集合 id 唯一、assignments target_id、水位（TF/EV/SEC/PR/FND/REV/RT/SM/PRJ）、RT→FND、REV→FND、waiver→file、role→file、mapping→req/sec/ev+份额。**未校验**：
  - `evidence_bindings`：(ev_id, req_id) 复合键查重、ev_id∈evidences、req_id∈requirements（与 scoring_mappings 的同款三查形成对照——#224 给映射补了，绑定没补）；
  - `matrix`（CMP 行 id 查重、requirement_id 存在性）；
  - `matrix_overrides` 键→requirements 存在性；
  - `block_marks`/`submitted_requirements` 的 file_id 存在性。
- 触发条件：手改/半截 db.json（restore 注释自己列的威胁模型：「手改/旧版本落盘」）。
- 影响：孤儿绑定直接进入 `derive_matrix_rows` 的 `ev_by_req` → 该 REQ 恒 has_ev（PARTIAL 以上，资格条件不再 blocking）→ 快照卡死前先经历一段「幽灵已响应」状态；与全文件 fail-closed 口径不一致。
- 现有测试：restore 拒载面测试覆盖孤儿 RT/waiver/role/mapping，无孤儿绑定样本。
- 建议：restore 补三查（复合键唯一、双向引用存在）；matrix/matrix_overrides 至少查 requirement 引用。

### A-06【P2｜CONFIRMED】S-5 全局一致性检查（金额/工期/术语/承诺冲突）已实现未接线——投标函 vs 报价表金额冲突无防线

- 位置：bid-tools/src/check.rs:509-553（check_global Blocker 级冲突检测）、:568-610（run_review 分片编排）
- 证据：grep 全仓 `run_review|check_local|extract_features|check_global` → 生产代码零消费，唯一调用方 tests/review_sharding.rs。check.rs 头注自认「正式接线在后续批次」。
- 影响：金额冲突（投标函总价≠分项合计≠开标一览表——评标现场最高频否决点之一）在导出链无自动防线；export.rs 只在**分项报价合计 vs bid_amount** 一处做了 0.01 容差告警（export.rs:284，warning 不阻断），跨章正文金额一致性（如章节里写「控制价 350 万」与表格 480 万）无人查。
- 现有测试：review_sharding.rs 对算法本身覆盖良好（确定性/内存纪律/归一化等价）——测试的是孤岛。
- 建议：validate 第一层并入 run_review 的全局发现（输入即 ChapterInput，store 直供）；或将 AmountConflict 至少接入 export 前置告警。

### A-07【P2｜CONFIRMED】负偏离（废标级）查询面零消费——技术参数不达标可一路导出

- 位置：bid-domain/src/param.rs:264-280（negative_deviations/incomparable_deviations）；grep 全仓 → bid-tools 生产代码仅 export.rs 读 `param_responses()` 渲染偏离表（:660-731，负偏离行红标+warning 文案），无任何门消费 `negative_deviations()`。
- 影响：param.rs 头注自述「负偏离=废标级……漏检一行就是放一行废标风险过闸」，但导出门（1a/1b/1c）与 readiness 均不查；偏离表只是「评委看得见」——自动化防线为零。叠加 A-23（set_param_responses 无 MCP 入口），现实中参数账本身都恒为空。
- 建议：writing_acceptance_findings 同款「导出时点独立复算」模式加 negative_deviations 门（或至少 readiness 阻塞）。

### A-08【P2｜CONFIRMED】ExportGate（两阶段导出门）与 LinkageLedger（证据链只降不升）为无接线孤岛——文档宣称 IMPLEMENTED 的语义不在产品路径

- 位置：bid-domain/src/export_gate.rs:1-169、linkage.rs:1-181；grep 全仓 → 仅 bid-domain/tests/four_contracts.rs 引用。
- 对比：生产导出的原子性由 export.rs manifest+tmp+rename 与 service.rs advance_project_to_exported 承载——**功能近似但非同一实现**：ExportGate 的 EXPORTING 中间态/query_view（SC-01「进度面必须显式 EXPORTING」）在产品路径不存在；中断后用户可查询的状态只有 manifest 的 no-op 判定。
- 影响：文档（export_gate.rs 头注「EXPORT_GATE_SEMANTICS = IMPLEMENTED」、domain-model §引用）与产品行为漂移；「断链检测/显式重验」同样只在测试里活着（linkage 与 A-09 同族）。
- 建议：要么接线（export_document 用 query_view 口径回报），要么把模块标注 EXPERIMENTAL 收编文档。

### A-09【P2｜CONFIRMED】影响面引擎（R-mod/R-date/DIRTY 级联）未接线；生产 store 无 dirty 传播——条款/证据变更后 ACCEPTED 章节无失效信号

- 位置：bid-domain/src/impact.rs（全模块孤岛，grep 无生产消费者）；outline.rs:794-814（dirty 唯一置位点=reopen_section）；lib.rs set_requirements/replace_requirements、evidence.rs register_evidence/bind_evidence 均不碰 sections[].dirty。
- 影响：domain-model §2 的「上游变更打脏相关章节（R-05）」在生产为零——补遗重导入、证据换版后，已 ACCEPTED 章节（以及 pre_export_confirm 审核快照，见 A-10）均无失效信号；check.rs 的 dirty 断言口径（S-5）因此也接不上。
- 建议：set_requirements/register_evidence（换版）后按 requirement_ids∩章节 打 dirty（impact.rs 的 related 语义可直接搬）。

### A-10【P2｜CONFIRMED】PreExportConfirm 内容绑定摘要只覆盖章节树六字段——矩阵覆写/评分映射变更不使审核失效

- 位置：bid-domain/src/outline.rs:601-640（sections_digest 字段清单：id/parent/title/level/order/content）；review.rs:612-630（export_allowed 只比该摘要）；scoring.rs:1418-1456（upsert_scoring_mapping bump content_version 但 digest 不含评分面）；matrix.rs:272-314（set_matrix_override 改变响应结构，digest 不变）。
- 触发条件：三门全过（digest=D）→ set_matrix_override（合法，有 Human receipt）或改评分映射 → export_allowed 仍按 D 放行。
- 影响：导出物的响应面/计分面与人工所批版本不一致而无需重审（覆写自身的 receipt 提供了部分缓解，但「人批的是整体标书」这层语义丢了）。
- 建议：digest 并入 matrix_overrides 摘要与 scoring_mappings 计数（或 export_allowed 加「过门后 override/mapping 变更」检查）。

### A-11【P2｜CONFIRMED（P0-1 相邻面）】validate 无条件推进 + export 跨态跳级——Validating/Reviewing/ReadyToExport 三个「人工阶段」可全程不驻留

- 位置：bid-tools/src/service.rs:1372-1389（validate_document_with_reviewer 无论报告结果如何都 Drafting→Validating——即 Lead P0-1）；service.rs:1438-1458（advance_project_to_exported：Drafting 一步跳 Validating→Reviewing→ReadyToExport→Exported）。
- 补充观察：guard 允许 Drafting 态调 export_bid_document（lib.rs:206-223），配合跳级函数，后三个状态在正常流程中只是导出的副作用，从未承载任何独立裁决——状态机图与真实生命周期脱节（「Reviewing 态禁 draft」的杀伤力见 P0-1）。
- 建议：随 #243 修复一并考虑——validate 结果含 BLOCKER 时禁推进；export 只从 ReadyToExport 起步（由 review 链推进）。

### A-12【P2｜CONFIRMED】证据幂等重放键不含 verification_tier/excerpt——内容核验态无法升级，文件级锚永久锁死 FILE_EXISTS

- 位置：bid-domain/src/evidence.rs:495-504（same_payload = fingerprint+valid_from+valid_until 三比）；service.rs:866-954（register_evidence_verified：无 locator 时 fingerprint=`sha256:<库内文件sha>`——同一文件的文件级锚重登记永远三全等）。
- 触发条件：先无 locator 登记（FILE_EXISTS）→ 后带 locator 重新登记（期望升 CONTENT_MATCHES）→ 语义位命中、payload 全等 → `deduplicated:true` no-op，新摘录/新 locator 丢弃。
- 影响：#217 四态在登记面的升级通道事实缺失（唯一路径是先换内容改指纹，语义上等于「重新换证」）；同时同语义位的 title/scope 变化也被 no-op 吞掉。
- 建议：same_payload 并入 excerpt/verification_tier/locator；或 tier 不同时视为换版。

### A-13【P2｜CONFIRMED】auto_checkpoint 全量快照写放大 + 无 fsync

- 位置：bid-tools/src/store_io.rs:109-121（save 序列化 domain+parsed 全量 → tmp → rename）；service.rs:700-704（每个写操作后调用）。
- 影响：parsed 含全部解析块文本（真实语料 1330 页约数十 MB）——每个 bind/draft/review/override 工具调用全量重写 db.json（对比：章内容已按章分片 O(1)，域快照没有）；rename 原子但无 fsync/无父目录 fsync，掉电可失最近若干次写（进程级「内存已变更、磁盘未跟上」的告警只在同进程内有效）。
- 建议：parsed 与 domain 分文件；快照分片或增量；rename 前 fsync tmp。

### A-14【P2｜CONFIRMED（P0-2 相邻面）】verification_tier 生产消费方为零——「找到证据≠读过证据」四态未进任何门

- 位置：grep 全仓 `verification_tier` → 生产消费仅 service.rs 登记时写入与 MCP 出参透出；**下游全不消费**：matrix.rs 派生（已知 P0-2）、draft.rs bound_evidence_ids/check_evidence_bindable（:199-222 只看 status）、readiness.rs req_has_usable_evidence（:712-717 只看绑定+status）、validate.rs（无 tier 规则）、export.rs（无检查）。
- 影响：FILE_EXISTS 占位证据（甚至指向招标文件本身）不仅在矩阵面清零 blocking（P0-2），也放行起草门（MissingTechEvidence）、进入生成素材（EvBrief.excerpt 空串只是文案诚实）、通过校验与导出——#217 的防线全部停在「入账时防伪」半边。
- 现有测试：evidence_content.rs / evidence_source.rs（bid-tools）测登记裁决面；无「tier 应影响下游门」的契约测试（因为下游本来就没接）。
- 建议：与 #245 修复联动——req_has_usable_evidence / matrix 派生至少要求 content_read()。

### A-15【P3｜CONFIRMED】bid-domain「零 IO」纪律被 party.rs 打破

- 位置：bid-domain/src/party.rs:204-234（load/save 直用 std::fs；save 无 tmp+rename，与 store_io/assembly 原子纪律不一致）。
- 影响：crate 头注「本 crate 零 IO（纯逻辑+内存态），持久化由 bid-tools 承接」名实不符；半写的 bidder-profile.json 可落盘。
- 建议：save 挪 bid-tools 或补 tmp+rename。

### A-16【P3｜CONFIRMED】dates::parse_ymd 只查月日域不查月长——2026-02-30 合法入库

- 位置：bid-domain/src/dates.rs:12-24（`!(1..=31).contains(&d)` 放行 2-31）；测试 `parse_ymd_strict_domain`（:80）钉死该口径为既定行为。
- 影响：valid_until/deadline 等域可收「2 月 30 日」，字典序比较行为怪但不致错判方向；add_days 会把它归一到 3-2 造成对账漂移。
- 建议：补 Hinnant 反查（civil_from_days 往返校验）。

### A-17【P3｜CONFIRMED】BidService::store_mut() 公开全量可变域面——服务层不变量可整体绕过

- 位置：bid-tools/src/service.rs:710-712；TrustedPrincipal::human（principal.rs:112-120）公开构造不验签。
- 证据：当前 MCP 生产路径仅 `record_idempotent`/`advance_project_status` 两个良性用途（grep），测试大量使用；但任何嵌入方经 store_mut() 可直接 record_review（human 伪造）、set_requirements、跳过 auto_checkpoint——receipt 验签「MCP 面唯一入口」的保证只覆盖 MCP。
- 建议：store_mut 收窄为私有三件套（或 #[doc(hidden)] + 门面方法）。

### A-18【P3｜CONFIRMED】is_price_item 词面误伤——含「投标报价」字样的主观评审项被跳过自评

- 位置：bid-domain/src/scoring.rs:971-974（`c.contains("价格分")||c.contains("投标报价")||c.contains("下浮率")||c.contains("基准价")`）。
- 触发条件：评分项「投标报价合理性分析 5 分」（评委主观打分）→ 被当价格项跳过 → 该项不计入自评 max/estimated（notes 只记「价格类跳过」，用户难分辨）。
- 影响：自评总分结构性偏低且无 UNMAPPED 级别的响亮提示。
- 建议：价格项判定改 params 面显式标记或加「疑似价格项」warning。

### A-19【P2｜CONFIRMED】derive_deviation「数值大=优」无方向覆写——响应时间/故障率类参数系统性误判偏离方向

- 位置：bid-domain/src/param.rs:94-108（数值主判恒 resp>req=Positive）；注释自认「方向敏感参数留待 T-7 审核闭环加 direction 覆写面——本批不猜测方向」，但 set_param_responses（:179-262）派生覆写**无条件**采用该口径，且无任何人工覆写通道（T-7 面见 A-23——也没有接线）。
- 影响：「响应时间 ≤2 小时（要求 4 小时）」判为负偏离并红标进导出偏离表——把优势写成废标级缺陷，评委按表读直接误解。
- 建议：Incomparable 化处理低者优启发（「响应|时间|率|噪声」词面），或至少负偏离前加人工确认位。

### A-20【P3｜CONFIRMED】bid-grade 判分器夹具耦合与恒真断言——「全绿」可信度打折（eval 面非产品面）

- 位置：grade.rs:454（`"metric" => pass(true)`——恒真）；grade.rs:73-75（dedupe_ok 硬编码 GT-03/GT-14）；grade.rs:330-334（fixture_open_bid_after 硬编码 "2026-11-27"）；main.rs:18/63（`expect("夹具装载失败")` panic 而非错误码）。
- 影响：metric 类断言永不失败（自注「由 TC-10 专项报告承载」）；判分口径与特定 GT 绑定，换语料即失效。
- 建议：metric 断言缺失即 Fail（对齐「留空字段不能中性化断言」的自我纪律）；硬编码下沉到夹具。

### A-21【P3｜CONFIRMED】LLM API key 明文落盘且无权限收紧

- 位置：bid-tools/src/llm.rs:81-88（.mang/llm.json 读取）；对比 approval.key 有 0600（service.rs:562-568）。凭证纪律「不写日志/不进生成内容/不落 usage」经 grep 证实属实。
- 建议：llm.json 写入面同样 chmod 0600（当前只读不写，风险有限）。

### A-22【P2｜CONFIRMED】评分映射无删除/撤销面——错误映射永久在账，restore 拒孤儿使撤销只能靠删 REQ

- 位置：bid-domain/src/scoring.rs:1347-1457（仅 upsert，无 remove）；lib.rs:893-908/953-968（set/replace_requirements 拒绝孤儿化映射）；A-10（审核不因映射变更失效）。
- 影响：错建的 (req, section) 映射只能降 share 无法清零行；配合「评分条件修改后审核不失效」（A-10），错误计分面可长期存活。#224 的三 blocker（聚合校验/restore 行约束/content_version 语义）本身修复完整（upsert 聚合口径 :1395-1417 与 restore :1326-1362 逐项对上，scoring_mapping.rs 891 行测试覆盖）——本条只针对缺失的撤销语义。

### A-23【P1｜CONFIRMED】RevisionTask 生命周期在服务层/MCP 面零接线——BLOCKER findings 造成永久导出死锁（P0-1 同型）

- 位置：bid-domain/src/review.rs:494-583（assign/advance/complete_revision_task 域方法完备）；grep 证实 bid-tools/src（service.rs 等）**无任何包装**、bid-mcp/src/tools.rs **无任何工具**（零命中）。
- 调用链：record_review_gate（MCP，合法过审面）带 ≥1 条 Blocker finding → record_review 派生 RT(Open) → open_blockers 非空 → export_allowed 恒 E_UNPROCESSABLE → **无工具可 assign/advance/complete RT**；record_review 幂等分支明确「既有 RT 一律不复位」（review.rs:396-397）；Rejected 也不撤回 Approved 里程碑（:100-102）。唯一出路=手改 db.json（与 P0-1 的恢复形态一致）。
- 同族缺失（grep 同证）：set_param_responses、batch_assign/advance_task、reopen_section、advance_section_status 的 MCP 面全部不存在——参数偏离表（A-07）、任务分工账、ACCEPTED 章节重开在产品面不可达。
- 触发条件：任何一次 record_review_gate 携带 Blocker（拒审依据门甚至**要求**必须带 Blocker——review.rs:365-371，Rejected 必须带）。
- 影响：正常使用「拒审」语义即永久锁死导出；修单/认领/关单的域层设计完整但产品不可用。
- 现有测试：review.rs 单元测试覆盖 RT 生命周期（assign→advance→complete 全链）——又是「孤岛测试」：测的是域方法，无 MCP 层贯通测试。
- 建议：补 `manage_revision_task` 类 MCP 工具（或 record_review_gate 出参明示「此门一开即锁导出」）；短期至少给 e2e 加「Blocker 后无出口」的契约钉子。

### A-24【P3｜CONFIRMED】bind_evidence_batch 部分失败面：绑定先落库、build_matrix 失败即整批错返而绑定已生效

- 位置：bid-tools/src/service.rs:961-987（循环 bind → build_matrix → auto_checkpoint）；绑定在循环内已 push 进 store（evidence.rs:651-657），build_matrix 或 checkpoint 失败 → Err 返回时内存已含新绑定。重启（或调用方重试同参）会经幂等重放恢复一致，但错误返回后的窗口内「内存与磁盘不一致」与「错误返回后残留副作用」成立（对照：其余方法均为单写点+checkpoint）。
- 建议：batch 收集后统一提交；或错误路径回滚绑定。

---

## C. 关键不变量分析（五面）

**1. 状态机面**：ProjectStatus next_allowed 线性前向、guard_tool 越级零副作用、advance 幂等 no-op——域层实现干净。真实风险全部在**服务层编排**：P0-1（validate 无条件推进）、A-11（export 跳级使中段状态形同虚设）、A-23（RT 生命周期无出口）。状态只进不退 + 缺工具 = 三个同型死锁面（P0-1、A-23、#226 之前的否决项死锁已被 override 解掉）。

**2. 矩阵面**：derive_matrix_rows 是好的单一真源设计，但**双口径分裂**（build=全量绑定 vs view=活证据）使「导出门读存量、工具面读派生」（A-01）；重算触发点只有 bind/override 两条（换版/过期/时间推移均不触发，A-01/A-02）；restore 不验矩阵与绑定（A-05）。

**3. 评分面**：份额守恒（单行 (0,1] + 聚合 ≤1.0 + epsilon）在 upsert 与 restore 两侧同源闭合，#224 修复完整；缺撤销语义（A-22）；判档=关键词覆盖启发（rule_heuristic）有 SCORE_RULES_RESERVED 诚实告警，未冒充评标（bid-grade 是 benchmark 判分器不涉及评分——「关键词覆盖率冒充评分」的风险面在 scoring.rs 自评出口，已由 notes/is_objective 标注缓解，属可接受设计）；is_price_item 词面误伤（A-18）。

**4. 证据面**：四态只降不升/禁绑/换版语义域层完备；三个断点：时间翻转无生产入口（A-02）、幂等键吞 tier 升级（A-12）、tier 无下游消费者（A-14，P0-2 的泛化）。bind 时刻 R-03 是唯一真实闸门。

**5. 审批面**：三门序+BLOCKER→RT+digest 绑定的域设计是五面中最完整的；三个缺口：receipt 不绑内容（A-03）、digest 不覆盖 override/mapping（A-10）、RT 无产品出口（A-23）。principal.rs 的 HMAC/scope/时效/常量时间比对实现质量高（矩阵覆写四重签域是标杆）。

---

## D. 测试守卫缺口清单（按 crate）

**bid-domain**（81 测试，tests/ 7 文件 + src 内嵌）：
- 无测试执行：impact.rs 全部（仅 four_contracts.rs 走 corpus 夹具）、export_gate.rs / linkage.rs（four_contracts.rs 有测但产品无消费——测试通过≠语义生效）、party.rs save/load 的 IO 面（forms_wire.rs 在 bid-tools 侧覆盖 profile 读写）、param.rs 的方向敏感样例（src 内测全为「大者优」形态）。
- 关键无守卫路径：**换版后 export_blockers**（A-01）、**apply_validity 的生产触发**（A-02）、**restore 孤儿绑定**（A-05）、**非补遗件 waiver→AddendumWaiver 门**（A-04）、**同一 receipt 审不同内容**（A-03）。
- 有守卫：状态机矩阵（state_machine_matrix/status_enum_matrix）、scoring_mapping 聚合/restore（scoring_mapping.rs 891 行）、outline 九不变量、budget 四阶梯。

**bid-tools**（270 测试，tests/ 29 文件）：
- 无测试执行的生产函数：check.rs run_review/check_global（A-06 孤岛）、param negative_deviations 消费链（A-07）、service.rs 的 advance_project_to_exported 跳级语义、load_or_derive_format_spec 的坏 JSON 分支（部分在 export_tool.rs 覆盖）。
- 关键无守卫路径：**RT 关单全链（MCP 面无工具也无测试）**（A-23）、**导出门 vs matrix_view 口径一致性**（A-01）、**导出前证据失效**（validate_tool.rs:445 测了 validate 拦截、没测 export 拦截）、auto_checkpoint 失败半态（A-24）。
- 守卫良好面：store_roundtrip（快照往返/损坏/版本门）、assembly_checkpoint（tmp/rename/幂等）、fs_fence（355 行含前缀陷阱/symlink）、export_stream/embed、readiness 六阻塞源（readiness_check/l3_addendum/wire 三文件）、draft 门（draft_section/iterative）、quality 四面、input_integrity_gate（P0-1 受限起草）。

**bid-kb**（9 测试）：lib/shard 双面覆盖良好（kb_probe 25 真实查询、shard_index 等价性/增量性/确定性）；缺口：amount 查询元/万元跨界（如查「5000元」对不上「0.5万」）无样本——P3 未立项。

**bid-grade**（2 测试文件）：selftest_gate/e2e_m1 覆盖完美库+破坏注入；缺口即 A-20（metric 恒真、硬编码）。

---

## E. 与 docs/bid-domain-model.md、docs/bid-mcp-contract.md 的差异

1. **契约工具数**：文档 24 vs 代码 25（共享上下文已登记；本审计补充：域层还有大量「已实现未暴露」面——A-23 列表说明 tools 数差异的同时存在反向缺口：RT/param/assignment/reopen 无工具）。
2. **domain-model §1 Finding**：文档声明 Finding 含 `status（OPEN/RESOLVED/WAIVED + 理由）`；代码 `review::Finding` 无 status 字段（状态由 RT 承载）——文档口径未更新。
3. **domain-model §2 R-date/DIRTY**：文档声明补遗/基准日变更触发全项目重检与章节打脏；代码仅孤岛 impact.rs（A-02/A-09）。
4. **export_gate「EXPORTED 以 sha256 落库为界 / EXPORTING 显式中间态」**：文档与模块头注宣称 IMPLEMENTED；产品路径无 ExportGate（A-08），等价能力由 manifest 部分承载。
5. **contract §8 precheck「审核 BLOCKER findings 清零」**：代码实现为「全部 RT DONE」（结构等价但语义更强——还需 resolution 非空）；文档未区分 findings 清零与 RT 关单。
6. **domain-model「零 IO crate」**（bid-domain 头注）：party.rs 直碰 std::fs（A-15）。
7. **contract §4/#217 verification_tier 四态可见**：可见（出参透出）但无消费语义（A-14）——契约描述了字段、没描述门，代码与契约字面一致、与「找到证据≠读过证据」的意图不一致。
8. **bid-mcp-contract record_review_gate**：receipt 线格式与验签分类文档化准确（Malformed/NotHuman/Expired/FutureDated/BadSignature 均实现）；未文档化「receipt 不绑内容」限制（A-03）。

---

## F. 结论与建议优先级

1. **立即修（P1 死锁/错误放行族）**：A-23（RT 无出口——record_review 带 Blocker 即锁死导出）、A-01+A-02（导出门活证据复算 + 有效性翻转入口）、A-04（waiver 混同）、A-03（receipt 绑内容）、A-05（restore 绑定校验）。
2. **短期（P2 接线族）**：A-06（金额冲突进 validate）、A-07（负偏离进导出门）、A-14（tier 进 readiness/matrix——随 #245）、A-11（随 #243）、A-12/A-19/A-22/A-13。
3. **卫生（P3）**：A-15/16/17/18/20/21。

整体评价：域层（bid-domain）不变量设计与测试密度是四个 crate 中最好的（fail-closed/幂等/水位/份额守恒都可圈可点）；主要系统性风险不在域规则本身，而在**「域规则已实现但生产链路未接线/接错线」**——共 6 个孤岛模块（check、param 消费、ExportGate、LinkageLedger、ImpactEngine、RT 生命周期）与 2 处双口径分裂（矩阵 build/view、validate 报告/状态推进）。
