# 独立交叉复核报告（review-report.md）

- 复核对象：① phase-1 审计（`audit/phase-1` 分支，124+ 发现 / R-01..R-34 / M1-M6 / FG-01..18）② V2 全覆盖审计（本分支 `audit/v2-full-coverage`，PA/PB 试点 + 15 份模块审计 + MUT-01..06）③ 当前 main 上的修复（冻结基线 `6105beb` → `6b6a4d4`，+105 提交）
- 复核执行：Lead（单一复核人，全量回源码裁决 + 真实运行验证）
- 环境：Linux、rustc/cargo 1.99.1（与两轮审计记载一致）、poppler 25.03.0、无 pdfium/chi_sim/GTK
- 复核纪律：只读——两处变异复跑（M1'/M2'）均在验证守卫后即时回滚并复绿，工作区最终纯净（`git status` 零改动提交）
- 报告日期：2026-11-06

---

## 一、phase-1 遗留「接线批第 0 步」执行结果：两个 P0 修复 CONFIRMED-FIXED（变异级验证）

phase-1 最终报告明确遗留："main 上两 P0 的修复未经验证——建议作为接线批第 0 步做针对性复核（修复 commit 范围内重跑 M1/M2 变异与 validate/SDK 用例）"。本次复核执行完毕，结论如下。

### 1.1 R-01 / #243（校验死锁族）——已修复，守卫确凿

| 验收项 | 复核结果 | 证据锚点（当前 main） |
|---|---|---|
| 前置闸：部分章节的全稿校验零副作用拒 | ✅ | `bid-tools/src/service.rs` `validate_document_with_reviewer`：`scope_sections.is_none()` 时 `incomplete_leaf_ids()` 非空 → `E_DRAFT_INCOMPLETE{missing}` 拒（不跑规则、不推进、不落盘） |
| 仅校验通过才推进 | ✅ | 推进段条件 `report.ok && scope_sections.is_none()` → Drafting→Validating + checkpoint；失败留在原态可修订重试 |
| 受支持恢复通道 | ✅ | 域层 `recover_validating_to_drafting`（三重约束：仅 VALIDATING / 仅缺章 / 理由非空白）+ 服务层 checkpoint + MCP 工具 `recover_validating_state`（tools.rs:66/3854-3882）+ 恢复审计账（lib.rs:409 起StateRecoveryRecord 追加式留底） |
| PA-003（测试固化问题行为） | ✅ 已翻转 | `validate_tool.rs:213-232` 断言"缺章全稿校验 → E_DRAFT_INCOMPLETE"；:251 断言"章节级检查不推进"；专项套件 `validate_state_safety_243.rs`（4 测：闸/零副作用/重启守门+恢复） |
| 变异复跑 M1'（删除前置闸） | **KILLED** | 3+ 测试变红：`full_order_rejected_then_draft_validate_export`、`incomplete_draft_guard_is_side_effect_free`、`persistence_restart_holds_guard_and_recovery` + validate_tool 面 |

**结论：#243 的四个死锁根（无条件推进 / 失败即困死 / 无回退边 / 测试特权）全部闭环。PA-001/PA-002/PA-003 均可关闭。**

### 1.2 R-02 / #245（占位证据清零 blocking）——已修复，守卫确凿

| 验收项 | 复核结果 | 证据锚点（当前 main） |
|---|---|---|
| 占位/待核验持久化状态（PB-002） | ✅ | `VerificationTier::Placeholder`（evidence.rs:224）+ `is_placeholder()`；登记面 MCP 参数 `placeholder:true`（从低不从高——自报未核验安全，自报核验被显式拒，tools.rs:4128） |
| 矩阵消费 tier（PB-001 核心） | ✅ | `split_live_bindings`（matrix.rs:518）单一分流真源：Active/Expiring 且占位 → pending 束；`derive_matrix_rows` 派生 `PENDING_EVIDENCE` 态；**mandatory 行占位绑定保持 blocking**（matrix.rs:330-339） |
| 双口径分裂（PB-005/A-01 面） | ✅ 同源修复 | `build_matrix` 与 `matrix_view` 共用 `split_live_bindings`——"两入口口径永不分叉"（注释自述，代码证实） |
| 起草面措辞分档（PB-003） | ✅ | `EV_PENDING_TEMPLATE`（draft.rs:305）：占位证据生成【待补：…真实证明材料尚未核验…不作已具备声明】，替代"可核验证据支撑"断言；结语段降级措辞（draft.rs:382） |
| 占位导出门（PB-004） | ✅ | 导出门新增 **1d**：`placeholder_evidence_findings` 导出时点独立复算 fail-closed（service.rs:1273 起）——不消费记审面（审核批准也不放行）；E_EVIDENCE_PLACEHOLDER / E_CITATION_PLACEHOLDER 均为 BLOCKER（validate.rs:350/744） |
| 人工豁免纪律（PR #247 收紧） | ✅ | 仅 `NOT_APPLICABLE + 非空偏离说明 + 可信 Human receipt`（`is_legitimate_placeholder_exemption`）构成豁免；普通状态覆写（Responded 等）不豁免——"覆写在场 ≠ 豁免成立" |
| 变异复跑 M2'（删除占位分流，占位并入 effective） | **KILLED** | bid-domain matrix 内联 4 红（`placeholder_binding_keeps_mandatory_blocking` / `placeholder_keeps_pending_blocking_and_gap_with_body` / `placeholder_with_section_still_pending` / `real_evidence_dominates_placeholder_tracking`）+ bid-mcp 2 红（`placeholder_accident_blocked_then_upgraded_exports` / **`placeholder_gate_1d_fires_even_when_matrix_and_reviews_pass`**——导出门 1d 独立复算的专项守卫） |

**结论：#245 全链（登记→绑定→矩阵→起草→校验→导出）六道消费面接通。PB-001/PB-002/PB-003/PB-004 均可关闭。**

### 1.3 随 P0 批一并核验的已修复项

| 登记项 | 状态 | 证据 |
|---|---|---|
| R-10 / B-02（合并面前缀过滤） | ✅ 已修复 | `has_extraction_face` 按业务 kind 判定（#250——前缀仅夹具兜底，与 extract 分发同源；merge.rs:90 起）；#271 再补 kind=格式模板 |
| R-16 / C-05（guard 窗口与契约不符） | ✅ 已修复 | Validating/Reviewing 态 allowed_tools 扩窗：register_evidence / bind_evidence / set_matrix_override / record_addendum_waiver / recover_validating_state 全部入窗（lib.rs:246-276） |
| PB-006（upsert_scoring_mapping 零生产调用） | ✅ 已接通 | outline.rs:1223：评分驱动大纲派生时落账（#252 第二步）——多对多映射生产入口存在 |
| R-29（契约缺 set_matrix_override + 工具数漂移） | ✅ 已修整 | 契约文档收录（演进史 ：19 行"工具面 30"）；#278 缺陷3 落实"文档不硬编码数量、指向 TOOL_NAMES 单一真源" |
| PA-005 附带（is_usable_evidence 不查 tier） | ✅ 随 #245 修复 | readiness 面 submission_readiness/evidence_missing 分账（#241），占位件不再计入"可用证据" |

---

## 二、复核新发现（当前 main 6b6a4d4）

### REV-01（P1）：main 存在真实测试红灯——quota_scoring_share 基线失修，红灯存活 26+ 提交无人察觉

**现象**：`bid-tools/tests/songling_scoring_outline.rs::pdf_engine_outline_quota_distribution` FAILED：
```
[songling-252] 配额分布: total=844000 scoring=691700(0.820) admin=84300(0.100)
[quota_scoring_share] 评分章节配额占比 回归：实测 0.820 < 固化基线 0.824（新回归——不允许比现状更差）
```

**归因（同环境二分，非环境差异）**：

| 提交 | 结果 |
|---|---|
| 43451fd（#253 基线固化点，0.824 写入） | ✅ 绿（实测=基线） |
| 2791685（#271，2e98319 的父提交） | ✅ 绿（实测 0.824） |
| **2e98319（feat #266：PDF 布局面语义表接管——商务要求 0→13 + 五题★结构化标记）** | ❌ **红（实测 0.820）——回归引入点** |
| 42640c3（#267）/ e27cadc / ceae70c（#293） | ❌ 红（0.820 恒定） |
| 6b6a4d4（当前 main） | ❌ 红 |

**根因链**：
1. 2e98319 把商务表 13 条 REQ 接入配额体系 → 行政/商务轨配额增加（0.095→0.100）、评分轨被挤占（0.824→0.820）——与该提交说明"商务要求 0→13"完全吻合；
2. 该提交对 songling manifest 的改动**仅添加空行与 note**（"本表数值待合并后实跑复核"）——自认待复核却从未刷新 `quota_scoring_share` 基线值（仍 0.824）；
3. 其后 26+ 提交（含 42640c3 的 manifest 基线刷新、ceae70c 的 #293 配额算法改动）均未触发该测试或未发现红灯；
4. **CI 缺口是红灯存活的直接原因**：`.github/workflows/windows-check.yml` 仍为 windows-latest 单一工作流，Windows runner 无 poppler/tesseract → 真件链测试全 SKIP 恒绿——**这正是 phase-1 审计 R-21/FG-02 预警的现实兑现**（审计预警在先、回归发生在后，因果闭环）。

**处置建议**（按项目自身"IMPROVED 协议"惯例）：
- 若商务 REQ 化是有意变更：实跑刷新 manifest `quota_scoring_share` 0.824→0.820 并在 PR 说明原因（同步复核 admin share 与 l2_floor）；
- 评分轨被商务轨挤占是否符合 #252 评分驱动大纲的意图，请 Lead 裁定；
- **优先补 Linux 真件 CI job**（phase-1 P1 CI 重建批的第 1 项）——否则同类基线失修还会反复发生。

### REV-02（P3）：测试副作用（非封闭性）仍在

复核期间多次运行测试后 `git status` 出现 `artifacts/e2e/latest/*`、`artifacts/real-corpus-e2e/latest/*`、`artifacts/synth-tender/run/latest/*` 的 M/?? 变更——测试运行改写入库产物。phase-1 test-quality-audit §3 已登记该面，当前 main 未修（评测产物与测试执行未隔离）。

### REV-03（登记，供 R6/R8 后续批次）：本环境复核边界

- mang-app 39 测试：BLOCKED（GTK `gdk-3.0` 系统库缺失，无法编译）——R-19 的"零执行"状态无法在本环境推翻，但结构事实未变：mang-app 仍是独立 workspace（其 Cargo.toml 自述"独立于 bidcore workspace"），CI 仍只 `cargo check`（windows-check.yml:93-94），**test step 未加**；
- OCR（chi_sim）/pdfium/敌意 ZIP 语料（R-11/R-24/R-25/R-26 NEEDS_ENV 族）：本环境无对应组件，未复核，状态维持两轮审计原结论。

---

## 三、未修复项复核确认（当前 main 仍存在——抽样式回源码验证）

| 登记项 | 复核确认 | 当前 main 证据 |
|---|---|---|
| R-03 / A-23（RevisionTask 生命周期 MCP 零出口） | ❌ 仍存在 | tools.rs 无 assign/advance/complete/batch_task/reopen_section/set_param_responses 任何工具（rg 零命中）；拒审派生 Open RT → export_allowed 恒拒的死锁面仍在 |
| R-04 / A-01（apply_validity 后矩阵不重算，1a 消费旧矩阵） | ❌ 仍存在（部分缓解） | `apply_validity` 仍只 bump content_version 不触发 build_matrix（evidence.rs:646）；缓解面：导出门新增 1c/1d 均为独立复算，但 **1a（check_export_ready）仍读持久化矩阵**——翻转 Expired 后非占位证据的 RESPONDED 旧行仍可过 1a |
| R-05 / A-02（apply_validity 无生产接线） | ❌ 仍存在 | rg 全仓生产调用方仅测试（impact.rs 的是另一同名 trait 方法）；时间性过期证据长期被当 Active 消费 |
| R-06 / A-03（receipt 不绑内容摘要） | ❌ 仍存在 | 线格式仍 `v1.<actor_id>.<actor_type>.<scope>.<issued_at>.<hmac>` 六段（principal.rs:17）——scope 只绑门名，24h 窗口内改章节内容后旧 receipt 仍可过门 |
| R-09 / B-01（Agent 标 requirement 未 submit 黑洞） | ⚠️ 部分改善 | merge.rs 有 `regex_dropped_covered` 统计与 "#219 二步：未覆盖块提醒"；但"Agent 标 requirement 且未随附 submitted → 正则 REQ 丢弃"的专属告警未见断言级证据（维持部分状态，待 B 组后续核） |
| R-12 / B-04（PDF tables 计数恒 0） | ❌ 仍存在 | poppler.rs:100 `text: collapse_spaces(&cleaned)`（空白折叠为单空格）→ :105-108 `tables = blocks.filter(\|b\| b.text.contains("  "))`（找双空格）——折叠后恒假，死指标依旧 |
| R-13 / C-01（E_NOTFOUND 线格式漂移） | ❌ 仍存在 | 工具层统一 `E_NOTFOUND`（error.rs:52）；SDK `test_mcp_e2e.py:1105-1106` 仍断言 `E_NOT_FOUND`——两侧未对齐（SDK 02 面红/固化漂移状态待 SDK 环境复核） |
| R-14 / C-02（minimal_flow.py 废弃参数） | ❌ 仍存在 | sdk/examples/minimal_flow.py:228 仍传 `"reviewer": "human:minimal-flow-操作者"`（基线确认 + 当前 main 确认） |
| R-19 / FG-01（mang-app 39 测试零执行） | ❌ 仍存在 | 见 REV-03：独立 workspace + CI 只 check |
| R-21 / FG-02（windows-only CI，真件链全 SKIP） | ❌ 仍存在 | 单一 windows-check.yml；REV-01 即其现实后果 |
| R-22（暗标 docProps 清零 + 全部件禁字扫描） | ❌ 仍存在（部分） | export.rs 有 anonymize 正文/章标题/表格/图注统一脱敏规则（改善）；但全仓 Rust 面 rg `docProps` 零命中——文档属性部件无清零、无回读禁字扫描面（export_readback.rs 已不在 src/，R-22 的回读三缺口未见等价物） |
| R-23 / B-05（MDCORE_REV 常量错报） | ❌ 仍存在 | `bid-engine/src/lib.rs:78` `MDCORE_REV = "aa0c859d…"` vs Cargo.toml 实钉 `f4328ebf…`——产物 rev 溯源错报依旧 |
| R-30 / D-01（web-ui md 预览 href 属性注入 + unsafe-inline CSP） | ❌ 仍存在 | app.js:2908 与基线逐字相同（`<a href="$2">` 未做属性级转义，`javascript:` 协议可注入） |
| #244 / PA-004 / PB-012（build_outline 覆盖处置门） | ❌ 仍存在 | tools.rs `build_outline` 前置仍仅"条款集非空"（E_STATE on empty requirements）——无 coverage 消费、无未判定块阻断 |

---

## 四、V2 审计结论抽查（R8 交叉复核——回基线源码验证）

| 抽查项 | 基线证据 | 结论 |
|---|---|---|
| D-01（web-ui XSS）：声称 app.js:2908 md 链接 href 未转义 | `git show 6105beb:web-ui/mang/app.js` 2900-2916 行：`inline()` 内 `esc(s)` 后 `x.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<a href="$2"…')` ——URL 分组 `$2` 未经属性级转义直接进 href | ✅ 属实（且当前 main 未修） |
| SDK-001（minimal_flow.py 废弃参数）：声称示例用已废除 reviewer 参数 | `git show 6105beb:sdk/examples/minimal_flow.py` 223-228 行：`"reviewer": "human:minimal-flow-操作者"` | ✅ 属实（且当前 main 未修） |
| PA-001（#243 无条件推进）：声称 service.rs:1379-1387 无条件推进 | 基线源码与本分支 pilot-a §3 表一致（本复核在 M1' 变异中反向验证了推进语义） | ✅ 属实（已修复） |

抽查 3/3 证实——V2 审计的行号级结论可信。未逐条重验全部 124+ 发现（抽样 + 已修复项全量验证 + 未修复项针对性验证的组合策略）。

---

## 五、mdcore 依赖锁定核验（R7 跨仓一致性）

| 核验点 | 结果 |
|---|---|
| bid-engine Cargo.toml 三件声明 | ✅ `mdcore` / `mdc-office-io` / `mdc-pdf` 均统一 `rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b"`（含 pdfium feature） |
| Cargo.lock 实锁 | ✅ 四个包（含传递的 mdc-formula）全部 `git+…?rev=f4328ebf…#f4328ebf…` |
| 冻结基线 6105beb 一致性 | ✅ 基线 Cargo.toml 同 rev——审计期间零漂移，**未自动升级**（纪律遵守） |
| mdcore 远端 HEAD | `6b9c95a8…`（高于锁定点；R-34 的"main 演进未消费"状态延续，属既知登记，非新风险） |

---

## 六、当前 main 测试实跑统计（Linux，六态口径）

| 包/面 | 结果 |
|---|---|
| bid-domain + bid-tools + bid-kb + bid-grade + bid-engine | 708 passed / **1 failed**（REV-01 songling quota） |
| bid-parse | 265 passed / 0 failed |
| bid-mcp | 304 passed / 0 failed |
| **合计（可执行面）** | **1,277 passed / 1 failed / 0 skipped-mandatory** |
| mang-app | BLOCKED（GTK gdk-3.0 缺失——NOT_EXECUTED，环境限制） |
| 对照基线（V2 审计：1,119 passed / 0 failed） | +158 测试；唯一红灯为 REV-01（基线失修，非环境） |

编译警告：bid-domain 1 处 unused_mut、bid-tools 3 处（dead_code×2 + unused×1）——与 V2 compiler-warnings.md 登记面同族，无新增错误级警告。

---

## 七、复核总裁决

1. **两个 P0（#243 校验死锁、#245 占位证据）在当前 main 已真实修复且有变异级守卫**——phase-1"接线批第 0 步"完成，phase-1 执行摘要中"先验证 main 修复再决定"的前置条件现已满足，接线批其余项（#244 覆盖门、RT 出口、1a 复算、暗标产品面、waiver 分门）可以排队。
2. **审计结论可信度**：抽查 3/3 证实 + 已修复项全量闭环验证 + 未修复项 13 项针对性确认——两轮审计的登记册与源码事实高度一致，无误报发现。
3. **当前 main 的真实风险排序更新**：
   - P1：REV-01（基线失修红灯 + CI 缺口的现实兑现）——建议立即处置并以此为 Linux CI job 的立项证据；
   - P1 维持：R-03（RT 死锁）、R-04/R-05（证据时效面）、R-06（receipt 内容绑定）、#244（覆盖门）、R-22（暗标 docProps/禁字）、R-14/R-13（SDK 示例与错误码漂移）、R-30（XSS）；
   - P2 维持：R-12（tables 死指标）、R-23（rev 错报）等。
4. **测试体系**：1,277 实跑全绿面（除 REV-01）证实两轮审计"Rust 测试真实执行生产代码"的裁定在 +105 提交后依然成立；新增强守卫（validate_state_safety_243 / placeholder_evidence_*）质量与既有纪律同水位。
5. **mdcore 锁定纪律**：全程遵守，无漂移。

## 八、复核证据清单（可复现）

- 变异 M1'：删除 service.rs #243 前置闸 → `cargo test -p bid-mcp --test validate_state_safety_243 --test validate_tool` → 3+ 红 → 回滚复绿
- 变异 M2'：删除 matrix.rs split_live_bindings 占位分流 → `cargo test -p bid-domain --lib matrix` 4 红 + `-p bid-mcp --test placeholder_evidence_e2e` 2 红 → 回滚复绿（21 passed 复验）
- REV-01 二分序列：43451fd ✅ → 2791685 ✅ → 2e98319 ❌ → 42640c3 ❌ → e27cadc ❌ → 6b6a4d4 ❌（同环境、同 27.2-27.9s 稳定复现；worktree 已清理）
- 全量实跑命令：`cd bidcore && CARGO_PROFILE_DEV_DEBUG=0 cargo test --workspace --no-fail-fast`（分包补齐统计）
