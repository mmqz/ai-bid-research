# 审计基线（baseline）

> 本文件为 ai-bid-research 全仓审计 V2 的冻结基线记录。审计纪律：只读审计（不改生产代码/测试/配置/数据），所有发现以证据（文件:行号）为准，交付物仅新增于本审计分支。

## 1. 冻结基线

| 项 | 值 |
|---|---|
| 仓库 | https://github.com/mmqz/ai-bid-research（私有） |
| 基线分支 | main |
| 基线 SHA（冻结） | `6105beb652e88760bdf38f08080af1ed7b1f048a` |
| 本地克隆 HEAD | `6105beb652e88760bdf38f08080af1ed7b1f048a`（与冻结一致，克隆时 main 未前移） |
| 工作区状态 | 干净（`git status --short` 为空） |
| 子模块 | 无（`git submodule status` 为空） |
| 跟踪文件数 | 1,737（`git ls-files`） |
| 审计分支 | `audit/v2-full-coverage`（自冻结 SHA 创建） |

## 2. 审计环境（工具链）

| 工具 | 版本 |
|---|---|
| git | 2.47.3 |
| rustc | 1.99.0 (b940084d7 2026-09-28) |
| cargo | 1.99.0 (5f94df478 2026-08-27) |
| clippy | 0.1.99 (b940084d7e 2026-09-28) |
| Python | 3.12.16 |
| Node | v24.22.2 |
| npm | 11.19.1 |
| tesseract | 5.5.0 |
| poppler（pdftoppm/pdftotext/pdfinfo） | 已安装（版本探测方式不兼容 --version，二进制存在） |
| LibreOffice | 25.2.3.2 |
| qpdf | 12.2.0 |
| mutool | 缺失（本机） |

平台注意：审计在 Linux x86_64 进行；仓库唯一 CI 门禁为 Windows（见 §4）。Linux 侧测试结果是**新增证据面**，Windows 专属行为（如 `.cargo/config.toml` 的 MSVC 静态链接、PowerShell 脚本）在 Linux 无法直接执行，将如实登记为 BLOCKED/platform-diff，不得臆断。

## 3. 依赖锁

| 锁文件 | 版本 |
|---|---|
| bidcore/Cargo.lock | lock 格式 version = 4 |
| web-ui/package-lock.json | lockfileVersion = 3 |

## 4. 工程盘点（构建入口）

### 4.1 Cargo 工程（共 18 个 Cargo.toml）

- **bidcore workspace**（`bidcore/Cargo.toml`，resolver=2）：成员 8 个 —— bid-domain、bid-parse、bid-kb、bid-tools、bid-grade、bid-mcp、bid-engine、mang-snap。
- **mang-app**（`bidcore/crates/mang-app/Cargo.toml`）：独立 workspace（有自己的 [workspace] 段，bidcore `--workspace` 不含它；CI 有单独 check 步骤，注释明确「此前零 CI 覆盖」）。
- **m1**（`m1/Cargo.toml`）：独立 Rust 工程。
- **scripts 下 6 个独立 Rust 工程**：baseline_harness、bid_health_check、engine_probe、title_impact、veto_gt、veto_measure。
- workspace.package：version 0.1.0 / edition 2021 / license Proprietary。
- 关键依赖面：serde/serde_json/sha2/hex/regex/unicode-normalization/libc/ureq(2)/image(0.25)；OOXML 解析栈（zip/quick-xml/calamine）已收敛至 bid-engine → mdc-office-io（git 依赖，mdcore 私有仓）。
- **mdcore 系 git 依赖锁定 SHA：`f4328ebf4f936e1321a2254382d59f9ab9f94b7b`**（已在 mdcore 仓验证存在：`fix(#157): brackets whole_lang 围栏推进语义——代理 B 方案 + 围栏优先级保留`）。mdcore main 当前 HEAD 为 `6322f869aa062a8948896169f6847161ec107e3c`（R7 需核对主干漂移与未消费能力）。

### 4.2 CI（唯一 workflow）

`.github/workflows/windows-check.yml`（触发：push/PR → main + 手动）：

1. mdcore 私有仓凭据布置（只读 deploy key，条件执行，secret 未配置则跳过）。
2. `cargo check --workspace --all-targets`（bidcore）。
3. `cargo check --all-targets`（mang-app 独立 workspace）。
4. `cargo test --workspace --no-fail-fast`（bidcore）。`--no-fail-fast` 来自 #168/#169 教训：此前第一个失败目标后的测试二进制从未在 CI 跑过。
5. SKIP 横幅自证（issue #9 SKIP 机制：外部工具缺件时 PDF 全链测试打 SKIP 横幅并提前 return，**计数为 passed 而非 failed**——R5 假绿审计重点对象）。
6. 外部工具链探测（tesseract/pdftoppm/pdftotext/pdfinfo）。

**CI 覆盖缺口（初判，待 R5 复核）**：无 Linux CI；m1 与 scripts 下 6 个 Rust 工程无 CI 门禁；mang-app 仅 check 无 test；Python（scripts/sdk）与 web-ui（typecheck/build）无 CI 门禁。

### 4.3 其他构建/测试入口

- `scripts/run_tests.py`：本地测试总入口（待逐文件审计）。
- `web-ui/package.json`：build / serve / typecheck（无浏览器 E2E 框架——按计划如实登记缺口，不得声称已做 E2E）。
- `.cargo/config.toml`：仅 Windows MSVC 静态链接配置（crt-static）。
- `scripts/fixtures-requirements.txt`：Python 依赖清单。

## 5. 冻结基线最近提交（git log -30 --oneline --decorate）

```
6105beb (HEAD -> main, origin/main, origin/HEAD) Merge pull request #242 from mmqz/feat/1.0-writing-gate
20dce7d (origin/feat/1.0-writing-gate) test(e2e): 产物定向刷新——W_CHARS_OFF_TARGET 文案变更 + 导出门 1c 落地后的当前输出
d4c5470 feat(1.0 写作验收): 导出门独立复算——篇幅不足即使审核直接批准也禁导出
b9e6a8a fix(1.0 写作验收): 正路径断言按分策语义收窄 + 超带 MINOR 回归锁
76533d7 test(1.0 写作验收): MCP 全流程验收 + 篇幅不足拒导出回归
17fb5f3 feat(1.0 写作验收): E_CHARS_BELOW_MIN 最低篇幅 BLOCKER + 回归测试
fdb8db1 Merge pull request #241 from mmqz/feat/agent-mandatory-blocking-gate
a35c0b1 (origin/feat/agent-mandatory-blocking-gate) test(#241 决策点2 落测): 商务必选「正文已响应、无外部证据」不算已响应——承诺型出口=人工覆写 RESPONDED+说明
46b7c0e feat(#229): 起草门 UncoveredMandatoryItem + 门禁测试三面——Agent 必选未响应拦起草
34607c6 feat(#229): Agent 必选标记接线矩阵 blocking 面——AGENT_MANDATORY_NOT_BLOCKING 缺口收口
4a09940 Merge pull request #240 from mmqz/fix/format-chapter-noise
315f1de Merge pull request #236 from mmqz/feat/224-scoring-mapping-step1
601abc0 (origin/feat/224-scoring-mapping-step1) fix(#224 审查追修): 多映射结果正确性——by_item 逐章聚合 + 映射报告只认生效来源
7433eb5 (origin/fix/format-chapter-noise) fix(#234): 格式章表格头/碎片抽取噪声清理——951→942、blocking 131→122
0c407de fix(#224 二次阻塞审查): restore 聚合不变量——同一评分项联结面有效份额合计 ≤ 1.0
46ba2aa fix(#224 review blocker 3): 映射写面 content_version 语义——真变更 bump、真 no-op 不 bump
84db056 fix(#224 review blocker 2): 映射引用完整性——重建三入口守卫 + restore 行约束
637cecb fix(#224 review blocker 1): upsert 聚合份额校验——联结面有效份额合计 ≤ 1.0
1a14a7e feat(#224 第1步): ScoringMapping 多对多映射 + UNMAPPED 结构化——评分项↔章节映射的核心补课
e6f0192 Merge pull request #239 from mmqz/fix/226-receipt-scope-e2e
8bcea39 Merge pull request #238 from mmqz/fix/docx-rule-statement
7a49db3 (origin/fix/226-receipt-scope-e2e) test(#239 审核追修): wrong_status_receipt 单维隔离——RESPONDED 调用携带同 deviation
a13d7d6 Merge pull request #204 from mmqz/feat/pdf-engine-abstraction
f9ef123 Merge pull request #199 from mmqz/fix/extract-by-kind
07c3d4f (origin/feat/pdf-engine-abstraction) Fix Identity-H CID font detection
bb78871 (origin/fix/extract-by-kind) test(e2e): 产物更新——#199 kind 分发后 B-技术规范书.docx 入抽
2fcd488 fix(#199 审查): 分发值域对齐 TENDER_FILE_KINDS + 建纲配额死循环修复
0c26058 (origin/fix/docx-rule-statement) fix(#233): DOCX 路径接入层 2 判据——规则说明分类降级，跨格式对称
eac447b feat(#226/#182): receipt 签域四重绑定 + 直起草路径测试——Lead 合并门收口
55d8986 Merge pull request #230 from mmqz/feat/226-veto-unlock
```

## 6. 顶层目录规模（git ls-files 按首段统计）

| 目录 | 文件数（含带引号转义路径） | 说明 |
|---|---|---|
| artifacts | 719 + 90（转义名） | 评测输出/历史产物 |
| bidcore | 217 | Rust 主 workspace |
| web-ui | 169 + 17 | 前端 |
| baseline | 88 + 51 | 历史评测基线 |
| scripts | 84 | Python/Rust/Shell 工具 |
| research | 83 + 1 | 历史研究报告 |
| real-corpus | 51 + 22 | 真实语料 |
| fixtures | 40 + 29 | 测试夹具 |
| docs | 29 | 文档 |
| sdk | 21 | Python SDK + Skill |
| m1 | 16 | 独立 Rust 实现 |
| real-samples | 2 + 1 | 真实样本 |
| 根级 | README.md、DEV-PLAN-M0-M5.md、CONTRIBUTING.md、.github、.cargo、.gitignore、.gitattributes | |

（转义名 = 路径含非 ASCII 字符被 git quotepath 转义的条目，清单脚本中将关闭 quotepath 取原始名。）

## 7. 审计范围声明

- 双仓：ai-bid-research（本仓，全量）+ mdcore（f4328eb 锁定基线 + 主干 6322f86 漂移核对，R7）。
- 历史资料（docs/audit/PLAN.md、Issue #203 六条修订、research/audit-r3/r4/r5、verification-plan、risk-register、test-plan、threat-model、bid-mcp-contract、bid-domain-model、architecture-decision、doc-engine-audit）仅作调查线索，一切以当前冻结 HEAD 重新核对为准。
- 已知开放问题（#243/#245/#244 等）登记为调查对象，不作为审计阻塞条件。
