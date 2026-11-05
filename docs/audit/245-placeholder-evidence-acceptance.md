# #245 验收报告：占位资质不得冒充已核验的真实证据

> 交付分支：`fix/245-placeholder-evidence`（基于 main `6105beb`，PR 见仓库）
> 日期：2026-11-05 ｜ 标签：bug / P0

## 1. 缺陷复现（事故形态）

2026-11-05 真实客户端运行中，Agent 注册占位资质并绑定到大量矩阵行，阻塞数
归零，被误读为真实资质已满足。代码级根因（main `6105beb`）：

- `derive_matrix_rows`（bid-domain/src/matrix.rs）的响应判定是
  `has_ev = !evs.is_empty()`——**只看绑定存在，不看证据的核验状态**。占位
  登记（甚至不相干文件的 FILE_EXISTS）绑定后，资格条件行从
  NOT_RESPONDED（blocking）翻为 PARTIAL/RESPONDED，blocking 静默清零；
- `EvidenceStatus` 只有有效期语义（ACTIVE/EXPIRING/EXPIRED/SUPERSEDED），
  `VerificationTier`（#217）只有正文链路深度语义（FILE_EXISTS/
  CONTENT_MATCHES），都没有「占位/待核验」的显式声明——占位材料与真实
  证据在矩阵/导出消费面不可区分。

## 2. 方案：占位成为一等公民（可声明、可追踪、不可冒充）

**核心语义**：允许用户在草稿中使用明确占位文案，但占位/未验真的资质**不得**
被当作已验证证据、**不得**清除合规阻塞、**不得**让输出宣称资质已具备。

| 层 | 落地 | 效果 |
|---|---|---|
| 域层（evidence.rs） | `VerificationTier::Placeholder` 五态面 + `is_placeholder()`；`same_payload` 含 tier（异态换版不重放）；换版**绑定迁移** | 占位可持久化；同指纹异态 → 换版（防「重登拿回占位 id」的假成功转正）；占位→真实升级 = 换版重登（旧记录 SUPERSEDED 留审计痕 + 绑定迁移，无原地迁移通道即无非法迁移通道） |
| 矩阵（matrix.rs） | `ResponseStatus::PendingEvidence`（PENDING_EVIDENCE）+ `MatrixRow.pending_evidence_ids`（serde default 旧快照零位移）；`derive_matrix_rows` 绑定三分输入（effective/pending）；blocking 扩为 NOT_RESPONDED \| PENDING_EVIDENCE | 占位绑定 → 待补可见（pending_evidence_ids 追踪面）+ **必选行 blocking 保留**（绝不静默清零）；非必选行待补不阻断（业务允许继续写作）；`split_live_bindings` 单一分流真源（build_matrix 与 matrix_view 共用，幽灵/死绑定两束都不进） |
| 服务层（service.rs） | `EvidenceRegistration.placeholder` → `register_evidence_verified` 裁决 tier=PLACEHOLDER（locator 对账通过也不升格——「文件里有这段文本」≠「资质真实具备」） | 占位声明走服务端裁决；指纹仍系统生成（#185 纪律不变） |
| 就绪（readiness.rs） | `is_usable_evidence` 排除占位；`matrix_responded` 对 PENDING_EVIDENCE 返回 false | 资格章节素材就绪不被占位登记静默满足（无证硬写拦——#110 反编造门 × #245 的纵深防御）；不可用清单标注 PLACEHOLDER（防 ACTIVE 误读） |
| 起草（draft.rs / llm.rs） | `EvBrief.placeholder` 随行；mock/LLM 对占位证据输出【待补：...】专属文案（不引用 [EV-xxxx]、不声称已具备）；收束段措辞收敛 | 占位可进草稿（用户授权形态），但不能成为证据引用、不能生成「已具备/已满足」结论 |
| 验证（validate.rs） | 规则 1/2 扩 PENDING_EVIDENCE（E_QUAL_UNMET/E_VETO_UNRESPONDED 带待补注记）；新规则 5b `E_EVIDENCE_PLACEHOLDER`（BLOCKER，绑定面——非必选行也报）；新规则 5c `E_CITATION_PLACEHOLDER`（BLOCKER，正文引用占位 EV = 引用面冒充） | 占位绑定在验证报告可追踪、显式标记 |
| 导出（service.rs 1d 门） | `placeholder_evidence_findings` 独立复算：活跃占位绑定存在即拒——**即使审核已直接批准**（三门全 Approved 零 RT 也不放行；与 1c 写作验收门同款「不消费记审面」纪律）。差异面：占位有人工豁免通道（matrix_overrides + Human receipt + 偏离说明），篇幅不足没有 | 占位材料不得作为已核验证据导出；修复不是死锁（升级/豁免后放行） |
| MCP 契约（tools.rs / schema.rs） | `register_evidence` 增 `placeholder`（bool，非布尔拒）；**`verification_tier` 参数显式拒**（自报拒，零副作用）；登记/绑定回执带 placeholder 标记与警示 hint；`get_compliance_matrix` 行出参带 pending_evidence_ids（status 过滤器可用 PENDING_EVIDENCE）；evidence 列带 verification_tier | 「MCP 层不能通过参数把占位状态直接改成 verified」——参数面不存在该通道 |

## 3. 验收条件逐条对账（issue 原文）

| 验收条件 | 结果 | 证据（测试） |
|---|---|---|
| 导入来源真实但内容标记为占位/待核验的资质：绑定后矩阵仍显示未满足/待补，导出门仍按既定规则拒绝或标记受限，绝不静默清零阻塞 | ✅ | e2e `placeholder_accident_blocked_then_upgraded_exports`（占位批量绑定 → blocking 未归零 + PENDING_EVIDENCE + 资格章节起草被 readiness 拦）；域测 `placeholder_binding_keeps_mandatory_blocking` |
| 经合法验证的真实证据：正常通过既有证据链，已有正向流程不回归 | ✅ | 域测 `real_evidence_dominates_placeholder_tracking`；服务测 `real_evidence_binding_still_clears_blocking`；e2e 测试 2（真实证据绑必选行 → 1a 过 + blocking=0 零回归）；evidence_content_tool.rs / evidence_source_tool.rs 既有面全绿 |
| 占位可以进入草稿，但在正文/验证/导出报告中可追踪并显式标记，不能伪装成资质证明 | ✅ | e2e 测试 2（评分章节占位 → mock 落【待补：...] 且不引用 [EV] 编号）；服务测 `mock_draft_renders_placeholder_as_pending_not_claim`；validate `E_EVIDENCE_PLACEHOLDER` / `E_CITATION_PLACEHOLDER`；【待补：...】形态与 W_PLACEHOLDER 词表（TODO/待补充/【占位】）不同源不误报（`validate_reports_placeholder_binding_and_citation` 锁定） |
| MCP 层不能通过参数把占位状态直接改成 verified；非法状态迁移拒绝且无副作用 | ✅ | 契约测 `register_rejects_self_reported_tier_and_bad_placeholder_type`（verification_tier 传入即拒 + 台账零入库）；`register_placeholder_reply_carries_tier_and_warning`（同态同指纹才幂等重放） |
| 服务层、矩阵、MCP 工具和端到端导出均有正负测试 | ✅ | 域 5（evidence）+ 5（matrix）；服务 9（placeholder_evidence.rs）；MCP 契约 4（placeholder_evidence_tool.rs）；e2e 2（placeholder_evidence_e2e.rs）——共 25 个新测试 |
| 升级旧数据的兼容策略有测试 | ✅ | 域测 `legacy_matrix_row_snapshot_loads_without_pending_field`（旧 MatrixRow JSON 无 pending_evidence_ids → serde default 空 Vec）；`tier_and_excerpt_passthrough_with_serde_roundtrip` 旧快照面（#217 既有）+ `placeholder_tier_persists_binds_and_roundtrips` 的旧快照非占位断言 |
| 最新 Windows CI 全绿；基于最新 main 独立分支、逐 commit push、PR 交付，不自行合并 | ✅ 分支/推送/PR 见下；CI 以 PR 检查为准 | 分支 `fix/245-placeholder-evidence` 基于 main `6105beb`；本地 Linux 全量回归：bid-domain 189+、bid-tools 33 套件、bid-mcp 43 套件全绿 |

## 4. 门禁语义补明（承接 #242 审核问询的同款问题）

**「占位证据在库时，即使有人直接批准审核（record_review 关闭 findings/RT），
是否仍应禁止导出？」——是。** 导出门 1d 与 1c 同为**独立复算门**：不消费
reviews/RT/gate_approved，只复算占位证据 BLOCKER。理由：记审记录是**流程
事实**不是**核验事实**——审核人批准的是「流程走完了」，不能替代「材料真的
核验过」。e2e 测试 2 专门锁定：三门全 Approved、零 findings、零 RT，导出
仍被 1d 拒（错误消息「占位证据门未过：N 处 BLOCKER（E_EVIDENCE_PLACEHOLDER@REQ-*）」）。

**人工通道的边界**（与 1c 的差异面）：占位缺失有人工豁免出口——
`set_matrix_override`（#226/#182：只认 Human receipt + 偏离说明，Agent 不得
自免单）；篇幅不足（1c）没有人工通道，只能补写。被覆写的 REQ 不进 1d 门
（豁免留痕可回溯）。

## 5. 测试清单（25 个新测试）

**bid-domain（evidence.rs ×4 + matrix.rs ×5）**：占位入账/可绑/快照往返；
异 tier 换版；绑定迁移去重；同态重放；占位不清阻塞；真实为准不回归；
占位+章节仍待补；人工覆写豁免通道保留；旧快照兼容。

**bid-tools（placeholder_evidence.rs ×9）**：tier 裁决（文件级/带定位）；
占位绑定保阻塞（batch 回执面）；真实绑定零回归；mock 待补文案；就绪面
不可用；升级换版绑定迁移矩阵翻转；validate 规则面（E_QUAL_UNMET 注记 +
E_EVIDENCE_PLACEHOLDER + E_CITATION_PLACEHOLDER + 【待补】不误报）；
1d 门真源（非必选覆盖/覆写豁免/升级清空）。

**bid-mcp 契约（placeholder_evidence_tool.rs ×4）**：占位回执（tier +
placeholder + hint 警示 + 幂等）；自报拒 + 非布尔拒 + 零副作用；矩阵待补
回显（PENDING_EVIDENCE 过滤器 + pending_evidence_ids + blocking 保留 +
bind 回执警示）；缺省非占位零回归。

**bid-mcp e2e（placeholder_evidence_e2e.rs ×2）**：事故形态全链 containment
（占位批量绑定 → blocking 未归零 → 资格章节起草拦 → 升级修复 → EXPORTED）；
1d 门独立复算（矩阵+审核全过仍拒 → 人工豁免 → EXPORTED）。

## 6. 边界遵守

- **不重复 #212**：文件来源/指纹校验未动（`register_evidence_verified`
  既有流程原样）；占位登记同样必须锚定已导入文件（来源真实性维度不变，
  本 issue 只管「内容核验状态」维度）；
- **不改 #241 Agent 必选阻断语义**：`agent_mandatory` 判定原样；占位绑定
  对 Agent 必选行的效果与类别必选一致（PENDING_EVIDENCE → blocking 保留）；
- **不碰 #234 / 评分模型 / 解析引擎**：无解析面改动（real-corpus-e2e 终态
  件的 394→391 漂移属 #240 解析面遗留——20dce7d 注记「归 #240 后续收口」，
  本 PR 不刷新该批产物）。

## 7. 遗留与说明

1. **真实模型验收**：本 issue 的验收全部走 mock 生成器（确定性）与真实
   corpus 夹具——**未配置真实 LLM 凭据**（`BID_LLM_API_KEY` 等环境变量
   缺席），llm.rs 的占位 prompt 纪律（system_framing 第 6 条规则 +
   user_material 占位素材降格）**未经真实模型端到端验证**，不将其计入
   已验收范围；待凭据就绪后经 `real_llm_path.rs` 两相位补验；
2. **W_PLACEHOLDER 词表**未扩（TODO/待补充/【占位】三词原样）——【待补：...】
   授权占位形态刻意避开词表（不同源不误报），若后续要统一占位残留检测面，
   需先区分「授权占位」与「模板残留」两语义；
3. **中间过程产物纪律**：本轮一次 `git add -A` 曾误扫入全量测试运行再生
   的 real-corpus-e2e 中间明细（+101k 行，19d11ff/20dce7d 惯例不入库）——
   已在 `4fe23bb` 全量恢复 main 状态并加 .gitignore 防复发规则
   （`artifacts/real-corpus-e2e/latest/*/0[2-8]_*`）。
