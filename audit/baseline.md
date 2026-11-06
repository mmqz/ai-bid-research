# 审计基线（baseline.md）

- 审计轮次：全仓代码覆盖审计 V2（用户 2026-11-06 指令）
- 审计员：Lead（自动化审计代理）
- 仓库：https://github.com/mmqz/ai-bid-research
- 审计分支：`audit/phase-1`（本分支，只新增审计产物，不改生产代码）

## 冻结基线

| 项 | 值 |
|---|---|
| HEAD SHA | `6105beb652e88760bdf38f08080af1ed7b1f048a` |
| 基线含义 | Merge pull request #242 from mmqz/feat/1.0-writing-gate |
| 基线时间 | 2026-11-04 21:25:30 +0800 |
| 工作区状态 | 干净（git status --short 为空） |
| 子模块 | 无 |
| 审计期间 main | 已推进 79 提交（至 2791685，2026-11-06 快照）——按纪律不混入 |

## 与远程 main 的关系（重要审计语义）

基线 `6105beb` 之后 main 又合入 79 个提交（含 #243、#245 的修复与 #252/#253/#269/#270/#271/#276 等）。
**本审计冻结在 6105beb**：#243/#245 的修复不在基线内，须按"当前仍存在"对待并复现验证；
其修复提交（main 侧）仅作为线索参考。#244 在 main 上仍为 open，基线上必然存在。

## 工具链（审计沙箱）

| 工具 | 版本 |
|---|---|
| Python | 3.12.16 |
| Node | v24.22.2 |
| Rust | rustup stable（沙箱安装，版本见测试记录） |
| 外部工具 | tesseract / poppler（pdftoppm/pdftotext/pdfinfo）按沙箱可用性登记，缺失时记 ENV_MISSING |

CI 侧（远端）：仓库唯一工作流 `.github/workflows/windows-check.yml`（Windows 单跑：
cargo check --workspace --all-targets；mang-app cargo check --all-targets；
cargo test --workspace --no-fail-fast；SKIP 横幅验证；外部工具链探测）。
无 Linux CI、无 clippy/fmt 门、scripts 六个 Rust 工程 / m1 / web-ui / SDK Python 无 CI 覆盖。

## 基线提交史（近 30，git log --oneline --decorate，冻结点）

```
6105beb (HEAD -> audit/phase-1) Merge pull request #242 from mmqz/feat/1.0-writing-gate
20dce7d test(e2e): 产物定向刷新——W_CHARS_OFF_TARGET 文案变更 + 导出门 1c 落地后的当前输出
d4c5470 feat(1.0 写作验收): 导出门独立复算——篇幅不足即使审核直接批准也禁导出
b9e6a8a fix(1.0 写作验收): 正路径断言按分治语义收窄 + 超带 MINOR 回归锁
76533d7 test(1.0 写作验收): MCP 全流程验收 + 篇幅不足拒导出回归
17fb5f3 feat(1.0 写作验收): E_CHARS_BELOW_MIN 最低篇幅 BLOCKER + 回归测试
fdb8db1 Merge pull request #241 from mmqz/feat/agent-mandatory-blocking-gate
a35c0b1 test(#241 决策点2 落测): 商务必选「正文已响应、无外部证据」不算已响应——承诺型出口=人工覆写 RESPONDED+说明
46b7c0e feat(#229): 起草门 UncoveredMandatoryItem + 门禁测试三面——Agent 必选未响应拦起草
34607c6 feat(#229): Agent 必选标记接线矩阵 blocking 面——AGENT_MANDATORY_NOT_BLOCKING 缺口收口
4a09940 Merge pull request #240 from mmqz/fix/format-chapter-noise
315f1de Merge pull request #236 from mmqz/feat/224-scoring-mapping-step1
601abc0 fix(#224 审查追修): 多映射结果正确性——by_item 逐章聚合 + 映射报告只认生效来源
7433eb5 fix(#234): 格式章表格头/碎片抽取噪声清理——951→942、blocking 131→122
0c407de fix(#224 二次阻塞审查): restore 聚合不变量——同一评分项联结面有效份额合计 ≤ 1.0
46ba2aa fix(#224 review blocker 3): 映射写面 content_version 语义——真变更 bump、真 no-op 不 bump
84db056 fix(#224 review blocker 2): 映射引用完整性——重建三入口守卫 + restore 行约束
637cecb fix(#224 review blocker 1): upsert 聚合份额校验——联结面有效份额合计 ≤ 1.0
1a14a7e feat(#224 第1步): ScoringMapping 多对多映射 + UNMAPPED 结构化——评分项↔章节映射的核心补课
e6f0192 Merge pull request #239 from mmqz/fix/226-receipt-scope-e2e
8bcea39 Merge pull request #238 from mmqz/fix/docx-rule-statement
7a49db3 test(#239 审核追修): wrong_status_receipt 单维隔离——RESPONDED 调用携带同 deviation
a13d7d6 Merge pull request #204 from mmqz/feat/pdf-engine-abstraction
f9ef123 Merge pull request #199 from mmqz/fix/extract-by-kind
07c3d4f Fix Identity-H CID font detection
bb78871 test(e2e): 产物更新——#199 kind 分发后 B-技术规范书.docx 入抽
2fcd488 fix(#199 审查): 分发值域对齐 TENDER_FILE_KINDS + 建纲配额死循环修复
0c26058 fix(#233): DOCX 路径接入层 2 判据——规则说明分类降级，跨格式对称
eac447b feat(#226/#182): receipt 签域四重绑定 + 直起草路径测试——Lead 合并门收口
55d8986 Merge pull request #230 from mmqz/feat/226-veto-unlock
```

## 基线时 GitHub Issue 状态（2026-11-06 API 快照，含基线后关闭项标注）

| Issue | 主题 | 基线 6105beb 上 | 2026-11-06 状态 |
|---|---|---|---|
| #203 | 全仓代码审计与测试重建计划 | 计划已入库，**未执行** | closed (11-03) |
| #224 | ScoringMapping 多对多映射 | 第 1 步已合入（PR #236） | closed (11-05) |
| #226 | 否决项死锁（规则说明误判） | 修复已合入 | closed (11-05) |
| #229 | 任务面板（元 issue） | — | open |
| #233/#234/#239 | DOCX 判据对称 / 格式章噪声 / receipt e2e | 修复已合入 | closed |
| #241 | Agent 必选阻断门 | 修复已合入（基线内） | closed (11-04) |
| #242 | 最低篇幅 BLOCKER + 导出门复算 | **基线即其合并点** | closed (11-04) |
| #243 | P0 起草中校验无副作用并支持状态恢复 | **修复在基线之后——视为未修复，须复现** | closed (11-05) |
| #244 | P1 建纲前闭环处置全块覆盖盲区 | **存在** | open |
| #245 | P0 占位资质不得冒充已核验合规证据 | **修复在基线之后——视为未修复，须复现** | closed (11-05) |
| #248/#252/#253 | 发行构建 / 评分驱动大纲 / 配额算法 | 未开始（基线后工作） | open |
| #266/#267/#269/#272/#277/#278/#279 | PDF 装配 / 检索归一 / 黑盒考验 / 骨架差异 / 无宿主形态 / Skill 缺陷 / 行协议 | 未开始（基线后暴露） | open |

## 历史审计资料（已读取，结论仅作线索）

- `docs/audit/PLAN.md`：#203 计划全文（917 行），docs/audit/ 下无任何执行产物。
- `research/audit-r3/r4/r5.md`：三轮历史审计（S1×1、S2×4、S3×5 起步；R5 证伪 R4 的 zip64 结论——V6/V7/V8 三类绕过后以 EOCD 全文件镜像收口）。
- `research/verification-plan.md` / `test-plan.md` / `risk-register.md`：V0~V3 + M1.7 场景矩阵；R-01~R-17；GT 冻结状态 PENDING。
- `docs/threat-model.md`：15 威胁 + 五级信任边界。
- `artifacts/bid-health-check/REPORT.md`（7fff169，2026-10-28）：真实水利语料 15 件去重 16 份——否决项召回聚合 **0.386**（硬门 100%，❌）；引用支持 317/317 ✅；崩溃 0；类别抽检 26 条中 13 落"其他" ⚠️。后续 veto-measure 将 L2 否决召回提至 ~0.83（另一套 GT 口径 1787 条），两套口径并存未回填。
- 关键不一致：契约工具数 24 vs 代码断言 25；历史测试计数（cargo 66 等）全面过时，当前静态 #[test] ≈ 1150+。
