# 增量复核销账清单 —— R8「未修复项确认」对最新 HEAD `431dca83`

- 复核执行：Agent-qkm4（审计复核会话，只登记不修复）
- 日期：2026-11-06（T0b 增量轮）
- 核验方式：`git grep` 提交对象级只读核验（不动工作区、不改任何生产代码/测试）
- 对象：R8 独立交叉复核（`audit/review/review-report.md` 第三节）确认的未修复项，基线 6b6a4d4 → 最新 main `431dca83a8c323e1427ce488d52ce20451f3b6e9`

---

## 一、销账矩阵（14 行 = 13 项未修复 + 1 项部分改善）

| R8 登记项 | 6b6a4d4 | 431dca83 | 证据锚点（HEAD 431dca83） | issue 登记 |
|---|---|---|---|---|
| R-03 / A-23（RT 生命周期 MCP 零出口） | 未修复 | ❌ 仍存在 | `assign/advance/complete/batch_task/reopen_section/set_param_responses` 于 bid-mcp src **零命中**（32 工具无一覆盖） | **#321（本次新建）** |
| R-04 / A-01（apply_validity 后矩阵不重算） | 未修复 | ❌ 仍存在 | `evidence.rs:646-671`：翻转后仅 bump content_version，无 build_matrix；1a 仍读持久化旧矩阵 | **#322（本次新建）** |
| R-05 / A-02（apply_validity 无生产接线） | 未修复 | ❌ 仍存在 | 全仓生产调用零（调用点全在 tests/ 与 doc 注释） | **#322（本次新建）** |
| R-06 / A-03（receipt 不绑内容摘要） | 未修复 | ❌ 仍存在 | `principal.rs:17/262` 线格式仍六段，scope 只绑门名 | **#323（本次新建）** |
| R-09 / B-01（Agent 标 requirement 未 submit） | 部分改善 | ⚠️ 维持部分 | 未深查（R8 已裁定部分状态） | （维持 R8 结论） |
| R-12 / B-04（PDF tables 计数恒 0） | 未修复 | ❌ 仍存在 **+ 新副本** | ①`bid-parse/src/engines/poppler.rs:102-107`（原址移位仍在）②**`bid-engine/src/mapping.rs:429`（本次新发现的同型第二副本）** | **#324（本次新建，含双副本）** |
| R-13 / C-01（E_NOTFOUND 线格式漂移） | 未修复 | ❌ 仍存在 | `error.rs:52` `E_NOTFOUND` vs `sdk/tests/test_mcp_e2e.py:1037`/evidence JSON `E_NOT_FOUND` | **#325（本次新建）** |
| R-14 / C-02（minimal_flow.py 废弃参数） | 未修复 | ❌ 仍存在 | `minimal_flow.py:228` 仍传 `"reviewer"` | **#325（本次新建）** |
| R-19 / FG-01（mang-app 39 测试零执行） | 未修复 | ❌ 仍存在 | CI 单一 `windows-check.yml`，mang-app 仅 `cargo check`（无 test step） | #303（已有） |
| R-21 / FG-02（windows-only CI，真件链全 SKIP） | 未修复 | ❌ 仍存在 | `.github/workflows/` 仅 windows-check.yml 一件；无 Linux job | #295（已有，PR #300 在途） |
| R-22（暗标 docProps 清零 + 禁字扫描） | 部分存在 | ❌ 仍存在 | Rust 面 `docProps` **零命中**；export_readback 等价物未见 | #317（部分覆盖——产品面登记，代码锚点须并入其处置范围） |
| R-23 / B-05（MDCORE_REV 常量错报） | 未修复 | ❌ 仍存在 | `bid-engine/src/lib.rs:78` `aa0c859d…` vs Cargo.toml `f4328ebf…` | **#327（本次新建）** |
| R-30 / D-01（web-ui href 属性注入 XSS） | 未修复 | ❌ 仍存在 | `app.js:2908` 与冻结基线 6105beb 逐字相同 | **#326（本次新建）** |
| #244 / PA-004 / PB-012（build_outline 覆盖门） | 未修复 | ❌ 仍存在 | main 上 `E_COVERAGE_INCOMPLETE` **零命中**（修复仅在 PR #292 分支） | #244（已有，PR #292 在途） |
| FG-08（engine_probe 钉旧 rev） | 未修复 | ❌ 仍存在 | `scripts/engine_probe/Cargo.toml:16/21` rev=`3204342d…`（注释却称与 bid-engine 同 rev） | #304（已有，本复核独立再证实） |

**汇总**：13 项未修复中，9 项此前无任何 issue 登记 → 本次建立 **#321–#327**（按仓库惯例归并为 7 个：R-04+R-05 合并、R-13+R-14 合并）；4 项已有登记（#303/#295/#304/#244），1 项部分覆盖（#317）。全部 14 个核验点在 431dca83 **无一被新提交修复**（#285 星标漏检合入未触及任何登记面）。

## 二、REV-01 更新（重要状态变化）

R8 记录的红灯（songling `quota_scoring_share` 实测 0.820 < 固化基线 0.824，回归引入点 2e98319）——**基线已被 #287 刷新**：

- `fixtures/songling-232762/manifest.json:211`：`quota_scoring_share: 0.873`（含根因注记：默认档 1,200,000 下实测——评分细则 20 叶底价 200,000 + 分值权重让渡合计 1,047,700；行政轨按条计价封顶 84,300）。
- 该刷新符合项目「基线更新须写清根因」纪律（AGENTS.md 规则 9），红灯针对的旧基线已不存在。
- **结论：疑似已解决，未独立复跑**——本环境未执行 Linux 实跑确认；PR #300（CI 装 poppler）合入后该门将在 CI 首次真跑，届时即有权威结论。

## 三、本次复核净新增发现

1. **R-12 死指标第二副本**：`bid-engine/src/mapping.rs:429` 与 `bid-parse/src/engines/poppler.rs:107` 同型（`contains("  ")` 判表格行），R8 只登记后者——已并入 #324，验收要求两处同修。
2. **main 持续漂移**：a5d7ec9 → 431dca83（10 提交：#285 星标漏检全套 + 工具面 30→32 + README 重写）。T0 快照（1 小时前）中的部分数字已过时，见 T0b 补记。
3. **issue 面变化**：open 18 → 26（#312–#320 黑盒九连发：篇幅产能墙/web-ui 接线/Skill 启动器/业绩材料库/多标段/暗标清理/评标逐条响应/补遗增量重写/多轮改标对比）→ **33**（本次 #321–#327）；在途 PR 3 → 2（#291 已合入，#285 已关闭 2026-11-06T10:44）。

## 四、复核方法与边界

- 全部核验为 `git grep <pattern> <commit> -- <path>` 与 `git show <commit>:<file>` 提交对象级只读检查，工作区零改动（`git status` 干净）。
- 未复核面（如实报告）：REV-01 的测试实跑（需 Linux + poppler + 私仓依赖编译，本环境未执行）；R-09 的断言级深查；mang-app/OCR/pdfium 族（维持 R8/两轮审计原结论）。
- 登记的 7 个 issue 仅为问题登记（含验收建议），**未做任何修复**——严格遵守「发现新问题开 issue，不要顺手修」（AGENTS.md 规则 17）。
