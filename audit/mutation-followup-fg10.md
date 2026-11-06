# Mutation Follow-up：FG-10 复核（mutation-followup-fg10.md）

> 增补于 2026-11-06（issue #304，代理E / Agent-E）。**不修改原
> `mutation-report.md` 的任何内容**——本文件是对其 M1「语义缺口」论断的
> 复核证据，方法与原报告同源（原地变异 → 运行目标测试 → 回滚）。

## 结论（一句话）

**FG-10「无条件推进无测试约束」的论断已过时**：变异基线 6105beb（11-04
21:25）之后 35 分钟，#243 修复（`4d4d8e6`，11-05 04:35）落地并携带了
完整的负向约束测试（`bid-mcp/tests/validate_state_safety_243.rs`）。本
复核把原报告**没跑**的「无条件推进」族三个变异在当前 main 跑了一遍：
**3/3 KILLED**。

## 时间线（为什么审计会误判）

| 时点 | 事件 |
|---|---|
| 11-04 21:25 | `6105beb`（#242 合并）——**变异基线**。此时推进逻辑无条件（无 ok 门、无 scope 门、无前置闸），`validate_state_safety_243.rs` 不存在 |
| 11-05 04:35 | `4d4d8e6` feat(#243)：前置闸 + `if report.ok && scope==all` 才推进 + `recover_validating_state` + 4 个状态安全测试 |
| 11-05 10:20 | `6f05f1e` test(#241)：全序测试失败源同步改写（未映射资格条款） |
| 11-06 16:56 | `af639d3` 审计归档——FG-10 登记「没有任何测试断言不得推进」系引用变异基线时点的结论，归档时未对当前树复核 |

原报告 M1 跑的变异是「**删除**推进」（→ 被 validate_tool.rs 21 个
happy-path 测试杀）；「无条件推进」这一生产缺陷方向（#243 本体）当时
**没有跑**——因为缺陷本身当时还在树上，跑了也没意义（变异=原样）。

## 复核变异（当前 main @ 431dca8，Rust 1.99.1，Linux）

变异点统一为 `bid-tools/src/service.rs` 的推进条件
（`validate_document_with_reviewer`）：

| ID | 变异内容 | 预期守卫 | 实际结果 | 杀伤测试 |
|---|---|---|---|---|
| F-Mut-A | 丢 scope 门（`if report.ok`——章节级检查也推进） | 章节级不推进 | **KILLED**（1 红） | validate_state_safety_243.rs:236（`章节级检查不推进`） |
| F-Mut-B | 丢 ok 门（`if scope==all`——校验失败也推进，**#243 缺陷精确形态**） | 失败不推进可修订 | **KILLED**（1 红） | validate_state_safety_243.rs:271（`③ 失败不推进（可修订可重试）`） |
| F-Mut-C | 删除 E_DRAFT_INCOMPLETE 前置闸（未完成全稿也跑规则） | 零副作用拒绝 | **KILLED**（3 红） | validate_state_safety_243.rs:217/:225/:249（`E_DRAFT_INCOMPLETE` 断言，3 个测试齐红） |

复现命令（每轮：变异 → 测试 → `git checkout` 回滚）：

```
# 变异示例（F-Mut-B）：
#   if report.ok && scope_sections.is_none() {
# → if scope_sections.is_none() {
CARGO_TARGET_DIR=/tmp/cargo-target-agente cargo test -p bid-mcp --test validate_state_safety_243
#   → test result: FAILED. 3 passed; 1 failed（full_order_rejected_then_draft_validate_export 红）
```

## 现状覆盖清单（「不得推进/不得困死」方向，全在 validate_state_safety_243.rs）

| 约束 | 断言位置 |
|---|---|
| 章节未完成 + scope=all → E_DRAFT_INCOMPLETE + 零副作用（不跑规则不落版本） | :204-:236（`incomplete_draft_guard_is_side_effect_free`） |
| 章节级检查（scope=sections）不推进 | :236 |
| 全稿失败（BLOCKER：未映射资格条款）→ 留 DRAFTING 可修订可重试 | :271 |
| 修订后通过 → 才推进 VALIDATING | :291 |
| 重验幂等（无变迁不落版本） | :297 |
| 历史异常态（VALIDATING+缺章）→ 受支持恢复 + 审计 + 起草面解锁 | :319-:339 |
| 持久化重启后守卫与恢复行为一致 | `persistence_restart_holds_guard_and_recovery` |

## 对 #304 三问的回答

1. **「无条件推进」指什么**：`validate_document_with_reviewer` 的
   Drafting→Validating 状态推进（ProjectStatus 面）——不是 mdcore 的
   BlockId/身份层（P0-1 的引擎半边，mdcore PR #249，与本仓无关）。
2. **现状有没有测试覆盖**：有——#243 修复自带 4 测试（见上表），
   加上 validate_tool.rs 21 个 happy-path（推进存在性）。
3. **「无约束」是完全没测试还是有测试不约束**：变异基线时点（11-04
   21:25）完全没测试；当前树已有完整约束（本文件三变异 KILLED 为证）。
   审计登记时未复核时间线。

## 遗留

- 原 M1「删除推进」方向在当前树的杀伤面未复跑（happy-path 测试仍在，
  原报告结论可延续；非本 issue 范围）。
- 审计登记册（false-green-register.md）FG-10 条目建议由 Lead 按
  「已解决（#243 + 本复核）」口径标注——登记册是归档文件，不由本 PR
  直接改写。
