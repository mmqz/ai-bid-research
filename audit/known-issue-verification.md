# R3 已知问题复核总表（known-issue-verification.md）

> 审计员：整合员。基线 6105beb（只读，未改仓库）。issue 清单来源：audit/baseline.md（2026-11-06 API 快照）+ docs/audit/PLAN.md。
> 每节口径：**基线状态**（CONFIRMED 存在 / 已修复验证通过 / 已修复但缺证据）+ 证据（文件:行 + 测试/变异结果，均本轮源码复核）+ main 后续修复说明（**仅注明，不作为基线结论**——基线后 main 推进 79 提交，按纪律不混入）。

---

## #203 全仓代码审计与测试重建计划

**基线状态：已执行（本审计补位）**。
- 证据（基线上"未执行"的确认）：docs/audit/PLAN.md 为 #203 计划全文（917 行）；docs/audit/ 下无任何执行产物（audit/baseline.md:93 登记）。基线快照状态 "计划已入库，**未执行**"（baseline.md:78）。
- 本审计补位证据：audit-work/ 七份产物（context + 模块审计 A/B/C/D 共 ~122 条发现 + test-quality-audit（bidcore 1,118 测试真实执行 0 失败 + run_tests 51/3/3 + SDK e2e 9/10 + mang-app 静态必红裁决）+ mutation-report（6 变异 5 直接杀）+ corpus-verification（否决召回 0.386→0.6998 重跑）+ 本轮三份整合文档）。
- main 后续：closed (11-03)，无代码修复面。

## #224 ScoringMapping 多对多映射（step1）

**基线状态：已修复（step1）验证通过；MCP 写面后续**。
- step1 三 blocker 逐项源码验证：
  1. 聚合不变量：`upsert_scoring_mapping`@bid-domain/src/scoring.rs:1387-1417（联结后有效份额 ≤1.0+ε；派生 params.section 计入 1.0、同节去重、更新排除被替换行）；
  2. restore 行约束：lib.rs:1263-1361（req/sec/ev 引用 + 复合键唯一 + 份额域 + **聚合 ≤1.0** :1326-1361）；
  3. content_version 语义：scoring.rs:1435-1439（真变更 bump）/ :1452-1455（真 no-op 不 bump）。
- 测试：bid-domain/tests/scoring_mapping.rs 九项红绿锚（:437 `aggregate_share_rejects_overshoot_on_insert`、:486 update 面、:519 派生计入、:571/:630 重建守卫、:657/:686/:716 restore 三面、:794 version 语义）——test-quality-audit §1.1 全绿（bid-domain 81+180 PASS）。
- UNMAPPED 结构化：scoring.rs:1232-1242 + score_document :1145-1156（`unmapped_scoring_items` + notes 响亮告警；测试 :235 `unmapped_items_structured_and_loud`）。
- **缺口（如实登记）**：`upsert_scoring_mapping` 的 MCP 写面不存在——全仓调用方仅测试（本轮 grep：scoring_mapping.rs / score_tools.rs / global_invariants.rs）；产品路径映射来源 = 派生 params.section（build_scorer@tools.rs:2919-2931 章引用联结回填）+ 账本（仅嵌入方/测试可写）。另登记 A-22（无撤销面）、A-18（is_price_item 词面误伤 scoring.rs:971-974）。
- main 后续：closed (11-05)；MCP 写面属 step2 后续工作。

## #226 否决项死锁（规则说明误判）

**基线状态：已修复（层 1 + 层 2）验证通过**。
- 层 1（人工出口 set_matrix_override）：工具面 tools.rs:4122-4233（TOOL_NAMES 含 25 工具 lib.rs:53-81）；服务面 service.rs:731-753（验签→覆写→`rebuild_matrix_for_override` 立即重算——死锁真解）；域面 matrix.rs:272-314（非 Human 拒/NOT_APPLICABLE 必须给理由/REQ 存在性）；guard 全态接线 lib.rs:203/221/237/252。
- 层 2（规则说明分类降级）：extract.rs:113-146 **统一后置**——对全部否决项 REQ 跑 `veto_lexicon::exclusion_reason`，命中即降级（评分公式→评分项，其余→其他），原因写 `params["_veto_excluded"]`（可审计）；注释声明覆盖 PDF/DOCX 两侧全部分类路径零遗漏。
- 测试：set_matrix_override_tool.rs 20 项（含 :137 `not_applicable_unblocks_a_blocking_row`、:491/:513/:558/:627 receipt 四反例）；blocking_probe.rs（#226 层 2 验收探针）；#226 语义并入 merge.rs:57-64（AGENT_MANDATORY_OFFCATEGORY 告警）与 coverage.rs:32（RuleVerdict 语义）。
- 残余关联（非 #226 本体）：B-06（DOCX/PDF 否决词表两套不同源——跨格式不对称，见 #233 节）。
- main 后续：closed (11-05)。

## #229 任务面板（Agent 必选接线 + 起草门）

**基线状态：已修复验证通过（+M3 变异杀伤）**。
- 矩阵接线：`agent_mandatory`@matrix.rs:115-118（marked_by=="agent" ∧ mandatory）→ blocking 面 matrix.rs:186-187。
- 起草门：`Blocker::UncoveredMandatoryItem`@readiness.rs:209（+机器码 :400、占位建议 :438-441）；消费面 readiness.rs:639-643（非否决类的 Agent 必选、矩阵未响应 → 拦 draft_response_section）。
- 测试：bid-mcp/tests/agent_mandatory_gate.rs:158 `agent_mandatory_offcategory_flows_into_matrix_and_export_gate`、:192 `agent_mandatory_draft_gate_blocks_until_receipt_override`、:257 `non_mandatory_agent_marks_never_block_any_gate`；域层 matrix.rs:425/:454 正反两面。
- 变异：**M3 KILLED（4 红）**——blocking 恒 false 被 `derive_covers_four_states_and_blocking` / `agent_mandatory_wires_blocking_outside_category_face` / 负面形态 / 12k 规模四测试击杀。
- main 后续：open（元 issue）。

## #241 Agent 必选阻断门（决策点 2）

**基线状态：已修复验证通过**。
- 语义：商务必选「正文已响应、无外部证据」**不算已响应**——判定走矩阵（`matrix_responded`@readiness.rs:730-744：命中行 Responded 才算；blocking/not_responded/partial 均 false），不看「章正文写过承诺」；承诺型出口 = 人工 receipt 覆写 RESPONDED+说明。
- 证据：readiness.rs:632 注释点名 `drafted_body_without_evidence_is_not_response`；测试 bid-tools/tests/readiness_check.rs:430（同名）+ agent_mandatory_gate.rs:287 `promise_form_mandatory_exits_via_responded_override_receipt`（决策点 2 裁定落测，:275-280 注释）。
- main 后续：closed (11-04，基线内合并 fdb8db1）。

## #233 DOCX 判据对称（规则说明跨格式）

**基线状态：已修复验证通过**。
- 证据：extract.rs:113-146 统一后置（**两侧单点接入**——PDF 强词/语境传播/审查表/前附表/章节映射全部分类路径后统一跑 DOCX 同款 `veto_lexicon::exclusion_reason`）；对照工具 bid-engine/examples/docx_categories.rs（改前/改后基线）。
- 测试：bid-parse/tests/extract_docx_veto.rs 12 测试（含 #226/#233 对称与幂等）——test-quality-audit §1.1 全绿。
- 残余登记（B-06，静态复核成立）：PDF 侧 re_veto（extract.rs:216-237，~48 形态）与 DOCX 侧 veto_lexicon.rs:22-57 词表**差集非空**（如「以他人名义投标/拒收情形/否决情形」PDF 有 DOCX 无；「投标无效/不予评审/被责令停业」DOCX 有 PDF 无）——同条款跨格式分类可不同。属 #233 修复后的下一层遗留，非修复缺口。
- main 后续：closed。

## #234 格式章表格头/碎片噪声

**基线状态：已修复验证通过**。
- 证据：extract.rs:390-400（format_chapter 两行/同行章形态判定）+ :707-716（格式章表格头守卫：格式章内 noun 表头行不抽为资格 REQ，强审查头 review_strong 旁路保留）+ :301（#234 要求动词面——真条款行必有动词，防误杀）。
- 测试：bid-parse/tests/extract_format_chapter.rs 9 测试（:85 格式章表格头不抽、:112 防误杀、:164 强头旁路、:185 非格式章行为不变）——全绿。
- main 后续：closed。

## #239 receipt 签域 e2e（#226/#182 追修）

**基线状态：已修复验证通过（+M5 变异杀伤）**。
- 签域四重绑定：`matrix_override_scope`@service.rs:602-611（project/requirement/status/reason-sha256 四段——与门级 receipt `review_gate:{gate}` 单段形成对照，后者为 A-03 的对象）。
- e2e 反例族：set_matrix_override_tool.rs:491 `forged_receipt_is_rejected`（**M5 KILLED**——验签旁路变异被此测试击杀）、:513 `cross_requirement_receipt_is_rejected`（跨行挪用，:543 断言 E_UNPROCESSABLE）、:558 `agent_identity_receipt_is_rejected`、:627 `wrong_status_receipt_is_rejected`（7a49db3 追修的单维隔离——同 deviation 只变 status）；mcp_scaffold_tools.rs 三门正例 + gate_receipt 工装。
- main 后续：closed。

## #242 最低篇幅 BLOCKER + 导出门独立复算（1.0 写作验收）

**基线状态：已修复验证通过（基线 6105beb 即其合并点；M4 变异杀伤 + SDK 红反向证实）**。
- 规则：`E_CHARS_BELOW_MIN`@validate.rs:465-480（构造器）/:522-527（复算面：正文 < target×80%）；`E_SECTION_EMPTY` :437-462。
- 导出门 1c：service.rs:1182-1207 → `writing_acceptance_findings`@validate.rs:502-530——**导出时点独立复算、显式不消费 reviews/RT/gate_approved**（:482-501 门禁语义注记；实现一致：只读 sections）。
- 验证三面：
  1. 变异：**M4 KILLED**——删除 service.rs:1188-1208 复算被 bid-mcp/tests/writing_acceptance_e2e.rs:486 `writing_acceptance_export_independent_of_review_approval` 击杀；
  2. 反向运行证实：SDK e2e 02-full-pipeline 红（test-quality-audit §1.6——external 夹具正文 344/119/256/166 字 < 400 下限 → validate 22 BLOCKER；门按设计工作，红的是夹具腐化本身，P1 测试基建项）；
  3. e2e 产物：20dce7d/76533d7（基线内）MCP 全流程验收 + 篇幅不足拒导出回归。
- main 后续：closed (11-04)。

## #243 P0 起草中校验无副作用并支持状态恢复（validate 无条件推进）

**基线状态：CONFIRMED 存在（静态四证据 + 无恢复路径）**。
- 证据（四件，本轮逐行复核）：
  1. **无条件推进**：bid-tools/src/service.rs:1372-1389 `validate_document_with_reviewer`——:1377 `validate::validate_document` 产出 report 后，:1379-1386 无条件执行 `Drafting → Validating`（steps 匹配不读 report；`validate_document` 是纯只读诊断的声明见 :1350-1357 注释，但状态面不服从该声明）；
  2. **Validating 态禁起草**：lib.rs:224-239（Validating|Reviewing|ReadyToExport 的 allowed_tools 不含 draft_response_section）；
  3. **只进不退**：lib.rs:601-617（advance_project_status 拒绝一切回退；`force_status` 仅 cfg(test)）——无任何 MCP 工具/服务方法可回退；
  4. **无恢复路径**：全仓无「状态恢复」工具（A-23 同族：reopen_section/advance_section_status 亦无 MCP 面）——唯一恢复 = 手改 db.json。
- 变异学佐证：M1 KILLED 但只杀「删除推进」方向——无任何测试断言「校验失败/章节未完成时不得推进或不得困死」（mutation-report §3 语义缺口=假绿盲区）。
- 编排放大面：C-04（SKILL.md:82-91 恢复协议引导 VALIDATING 态跳过未完成章节）、C-29（p7-review.md:98-101 的 MAJOR「回 P5 重提」路径在基线不可行——E_STATE）。
- main 后续：修复在基线之后（closed 11-05）——**仅注明，不作基线结论**。

## #244 P1 建纲前闭环处置全块覆盖盲区

**基线状态：CONFIRMED 存在（覆盖闭环未接——本轮自采代码证据五件）**。
- 证据（**按指令从 bid-parse/src/merge.rs 与 bid-tools 建纲入口自采**，均本轮 Read 复核）：
  1. **merge.rs:192-204（④ 覆盖统计）**：`uncovered_blocks`/`coverage_complete` 只是 `MergeStats` 返回值里的**纯统计数字**——`merge_extraction` 对未覆盖块的处理是"正则兜底保留"（:133-148 只丢已覆盖块），对「Agent 标 requirement 但没提交 submitted」的块则**既丢正则 REQ 又不生成 REQ-A**（见第 5 件）；
  2. **tools.rs:836-842（UNCOVERED_BLOCKS 仅告警）**：`merge_stats.uncovered_blocks > 0` → 只 push 一条 warning（"正规已兜底补漏——用 get_coverage_report 查看后 submit_block_marks 补标"）——随后 :805 `replace_requirements` 照常整集替换落库、状态照常推进 REQUIREMENTS_EXTRACTED；
  3. **tools.rs:3642-3653（build_outline 入口唯一前置）**：`n_reqs == 0` → E_STATE——**全函数不消费 `coverage_complete`/`uncovered_blocks`/`merge_stats`**（:3655-3661 直接进 `prepare_outline_and_matrix`）；
  4. **lib.rs:182 注释-实现背离**：guard 表注释承诺「#219 全块覆盖（建纲前最后机会补标——**覆盖不足时 build_outline 应告警**）」——build_outline 实现（tools.rs:3618-3688）中**不存在任何覆盖告警**；
  5. **B-01 黑洞叠加（merge.rs:133-148 + coverage.rs:192-218）**：正则 REQ 丢弃与 verdict 无关（merge.rs:138-148），REQ-A 仅由 submitted 生成（:151-189）；`validate_submitted` 只校验「submitted 必须落在 requirement 区间」（coverage.rs:208-215），**不校验反向**（requirement 区间必须有 submitted）——Agent 标 requirement 未随附提交时，该块条款**无任何告警地从矩阵消失**（端态被 merge.rs:304-349 钉为预期设计）。
- 次级证据：coverage_tools.rs:224-228（submit 出参 next 文案「全覆盖——可以继续 build_outline」纯建议性）；get_coverage_report（coverage_tools.rs:233-285）为只读查询无门。
- 影响：#219 的「Agent 优先」架构只有**度量闭环**（块→标记→统计→报告），没有**裁决闭环**（覆盖不足不阻断建纲/矩阵/导出）；叠加 B-02（merge faces 按文件名前缀过滤 tools.rs:727→merge.rs:71-76——真实文件名标注整体失效）后，全块覆盖机制在真实文件名世界基本旁路。
- main 后续：**open**（基线上必然存在——baseline.md:86）。

## #245 P0 占位资质不得冒充已核验合规证据

**基线状态：CONFIRMED 存在（矩阵不消费 tier + bind 不查内容匹配——三组静态证据）**。
- 证据（本轮逐行复核）：
  1. **矩阵派生只看绑定存在**：matrix.rs:168（`evs = ev_by_req.get(...)`——按 (ev_id, req_id) 绑定对建索引 :141-144）+ :174-179（`has_ev = !evs.is_empty()` 直接决定 Partial/Responded）——**不消费 verification_tier、不消费证据内容与条款的匹配**；blocking 清零只看 status != NotResponded（:186-187）；
  2. **bind 门只查状态**：evidence.rs:608-659 `bind_evidence`——:625-634 开标日即时翻转后，:635-643 **唯一闸门 `ev.status.can_bind()`**（Active/Expiring 放行）；无 tier 检查、无「证据内容是否支撑该条款」检查；
  3. **FILE_EXISTS 门槛可指任意已导入文件**：service.rs:918-931（`register_evidence_verified`）——无 locator 时 fingerprint = `sha256:<库内文件 sha>`（:925-930），tier=FILE_EXISTS：**指向招标文件本身（或任一附件）的文件级锚**即可登记为 kind=qualification 的"资质证据"并绑定资格条件 → blocking 清零。
- 下游放大（A-14，本轮 grep 复核）：verification_tier 生产消费方为零——draft.rs:199-209（check_evidence_bindable 只看 status）、readiness.rs:712-717（req_has_usable_evidence 只看绑定+status）、validate.rs（无 tier 规则）、export.rs（无检查）——FILE_EXISTS 占位证据放行起草门（MissingTechEvidence）、进生成素材、过校验与导出。
- 入口放大（C-23）：语义属性（kind/title/subject/validity/fields）全调用方自报——CONTENT_MATCHES 只证「该文本存在于该文件」，不证「该文本是所称证书」；纯 MCP 工具面一次调用即可伪造一条证据并清零 blocking，全程无人工闸（register 无 receipt 路径）。
- 守卫缺口：无「tier 应影响下游门」契约测试（下游未接线）；matrix_view 的活证据过滤（matrix.rs:347-356）只覆盖 get_compliance_matrix 工具面，不覆盖导出门（A-01——本 P0 在导出面的残留）。
- main 后续：修复在基线之后（closed 11-05）——**仅注明，不作基线结论**。

---

## 总表

| Issue | 基线状态 | 关键证据锚 | 验证强度 |
|---|---|---|---|
| #203 | 计划未执行 → **本审计已补** | docs/audit/PLAN.md（917 行）+ audit-work/ 全套产物 | 全量执行（1118 测试 + 6 变异 + 语料重跑） |
| #224 | **已修复（step1）验证通过**；MCP 写面后续 | scoring.rs:1387-1417/:1435-1455；lib.rs:1263-1361；scoring_mapping.rs 九锚 | 源码+测试+（M3 部分覆盖） |
| #226 | **已修复（层1+层2）验证通过** | tools.rs:4122-4233；service.rs:731-753；matrix.rs:272-314；extract.rs:113-146 | 源码+20 工具测试 |
| #229 | **已修复验证通过** | matrix.rs:115-118/:186-187；readiness.rs:209/:639-643；agent_mandatory_gate.rs | 源码+测试+**M3 KILLED** |
| #241 | **已修复验证通过** | readiness.rs:730-744；readiness_check.rs:430；agent_mandatory_gate.rs:287 | 源码+测试 |
| #233 | **已修复验证通过**（B-06 词表差集残留） | extract.rs:113-146；extract_docx_veto.rs 12 测试 | 源码+测试 |
| #234 | **已修复验证通过** | extract.rs:301/:390-400/:707-716；extract_format_chapter.rs 9 测试 | 源码+测试 |
| #239 | **已修复验证通过** | service.rs:602-611；set_matrix_override_tool.rs:491/:513/:558/:627 | 源码+测试+**M5 KILLED** |
| #242 | **已修复验证通过**（基线即合并点） | validate.rs:465-480/:502-530；service.rs:1188-1207 | 源码+测试+**M4 KILLED**+SDK 红反向证实 |
| #243 | **CONFIRMED 存在（P0）** | service.rs:1377-1386；lib.rs:224-239/:601-617；无恢复工具 | 静态四证据 + M1 语义缺口佐证 |
| #244 | **CONFIRMED 存在（P1）** | merge.rs:133-148/:151-189/:192-204；tools.rs:836-842/:3642-3653；lib.rs:182 注释背离；coverage.rs:192-218 | 五件自采源码证据 |
| #245 | **CONFIRMED 存在（P0）** | matrix.rs:141-144/:168/:174-187；evidence.rs:635-643；service.rs:918-931 | 三组静态证据 + A-14/C-23 放大面 |

**基线结论**：8 项已修复验证通过（#224 step1/#226/#229/#241/#233/#234/#239/#242——其中三项有变异级击杀证据）；1 项计划类已补执行（#203）；**3 项在基线成立（#243 P0、#244 P1、#245 P0）**——三者均属「域层/规则层无缺陷、接线层缺线」型：#243 缺「推进前的结果裁决线」，#244 缺「覆盖统计→建纲门的裁决线」，#245 缺「tier/内容→bind 与派生的消费线」。
