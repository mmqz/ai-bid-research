# 标书系统 M0→M5 开发计划（PR 分片，逐个可合并）

> 依据：2026-10-26 对 `mmqz/mdcore`（main 3e1f616）与 `mmqz/ai-bid-research`（main e0d6ffc）的实拉实跑审计。
> 原则：**每个 PR 可独立编译、可独立测试、可独立回滚**；不产生"半截合入"。

---

## 0. 审计结论回顾（决定了 PR 顺序）

| 现状 | 事实 |
|---|---|
| 编译 | HEAD 上 `cargo build --workspace` **直接失败**（guard.rs:645，`#[cfg(not(unix))]` 分支从未编译过） |
| 测试 | 修掉编译错后 39 passed / 8 failed。**8 项红全部因本机缺 `pdftotext`/`pdftoppm`/`tesseract`**，非代码缺陷 |
| 入口 | **没有任何产品入口**：无 MCP server、无 HTTP、无 CLI（只有判分器与 M1 流水线两个 bin） |
| 序列化 | HEAD 上 `cargo test --workspace` **不串行可跑**——多个测试共用 `runs/` 目录，并发时互相踩 |
| 与引擎连接 | **零**。全仓对 mdcore 的引用只有 `linkage.rs:10` 一行注释；Cargo 依赖 / wasm / FFI / 子进程全为 0 |
| 语义重复 | 引擎 `mdc-evidence`（53 测试）与标书 `bid-domain/linkage.rs` 是**两份独立实现**，未来必然漂移 |

→ 因此 PR 顺序必须是：**先让它能编译能跑 → 再补入口 → 再打通引擎 → 最后才做功能**。
反过来做（先写起草/审校）会在一个跑不起来的地基上盖楼。

---

## 1. PR 分片总表

| PR | 标题 | 依赖 | 预估 | 验收 |
|---|---|---|---|---|
| **#1** | `fix(bid-parse): Windows 分支编译错误` | — | 已完成 | `cargo build --workspace` 通过 |
| **#2** | `feat(bid-mcp): M0 工具解析与分发骨架` | #1 | 已完成 | 8 项契约测试全绿，未实现工具返回 typed `UNIMPLEMENTED_INTERNAL` |
| **#3** | `fix(test): 测试串行化 + 环境缺件显式跳过` | #1 | 0.5 天 | `cargo test --workspace` 在**任何**机器上要么全绿要么给出明确缺件清单，不再互相踩 |
| **#4** | `feat(env): 环境自举（Dockerfile + setup 脚本）` | #3 | 1 天 | clone → `make setup` → `make test` 一条命令全绿 |
| **#5** | `feat(bid-mcp): MCP server 骨架落地（stdio）` | #2 | 1 天 | 能被 Claude Desktop / 任意 MCP 客户端连上，`tools/list` 返回 9 个工具 |
| **#6** | `feat(bid-kernel): 引擎适配器（对齐 mdcore 契约面）` | #5 + 引擎侧 | 3 天 | 标书侧首次能 `use mdc_evidence::LinkageStatus`，语义漂移变编译错误 |
| **#7** | `feat(bid-domain): 证据模型改用引擎 mdc-evidence` | #6 | 2 天 | 删除 `linkage.rs` 的自造 `LinkageStatus`，测试全绿 |
| **#8** | `feat(bid-tools): import_tender_files 真实现（M1-a）` | #5 | 3 天 | 真实招标文件导入 + 完整性对账 + `parse_failures` 逐件可见 |
| **#9** | `feat(bid-tools): extract_requirements 真实现（M1-b）` | #8 | 4 天 | 合成语料条款召回 ≥95%、**否决项召回 100%（硬门）** |
| **#10** | `feat(bid-kb): search_evidence 双通道（M2-b）` | #9 | 3 天 | 编号/型号/金额区间/日期四类 needle 精确命中率 ≥90% |
| **#11** | `feat(bid-domain): 合规矩阵 + 三道人审门（M2-a）` | #10 | 4 天 | 无 waiver 时矩阵门拒绝放行 |
| **#12** | `feat(bid-tools): draft_response_section（M3）` | #7 + #11 | 5 天 | 无证据断言必须被 `CITATION_MISMATCH` 拦截 |
| **#13** | `feat(bid-rules): 审校规则注册表 ≥10 条（M4）` | #12 | 4 天 | 每规则 ≥1 正 1 反用例，确定性检出率 100% |
| **#14** | `feat(bid-export): 导出适配器 + 暗标模式（M5）` | #6 + #13 + 引擎 G3 | 4 天 | `EXPORTED ⟺ sha256 落库`，中断不留半成品 |

**总预估：约 6~7 周**（不含引擎侧 G1/G2/G3 的等待时间）。

---

## 2. 关键设计决策（PR 前必须定）

### D1. MCP 传输形态

**建议：stdio 先行，HTTP 后置。**
理由：`bid-skill-design.md` 的九阶段路径是**封闭 Agent 编排**，本地 stdio 足够；
HTTP 会过早引入鉴权/并发/部署面，而 M0 阶段没有任何并发需求。

### D2. 引擎集成形态

两种：

| 形态 | 做法 | 优点 | 缺点 |
|---|---|---|---|
| **A（建议）** | 标书侧 crate 直接 `dep = path` 引引擎 `mdc-evidence`/`mdc-office-io` | 同进程、类型安全、编译期抓漂移 | 两仓必须同时 checkout 到固定相对路径 |
| B | 通过引擎 wasm + 进程间调用 | 松耦合 | 跨语言序列化、性能损失、调试困难 |

**建议 A**：两边都是 Rust，同语言还绕开 FFI 是没道理的自伤。

### D3. 两仓关系

**建议：不合并 git 仓，但用 `path = "../../mdcore/crates/xxx"` 建立代码依赖。**

理由见 `AUDIT-engine-vs-bid.md` §4：`zip`/`quick-xml`/`calamine` 版本差异（`zip` 8.6 vs 4.6）
会让 M1.6 的 EOCD 守恒门语义基准不一致；且引擎仓的合规面（`compliance/scan.sh` denylist）
会被标书侧的 `research/raw/` 第三方源码摘录污染。

---

## 3. 每类 PR 的验收纪律

1. **不假绿**：未落地能力返回 typed 错误，**不返回空成功**（#2 的 8 项测试就是这条的守卫）
2. **不静默跳过**：缺件必须显式 `skip + 原因`，而不是"测试通过"（#3）
3. **不提前锁架构**：每个 PR 只做当期里程碑，不顺手把 M5 的接口先定死
4. **跨仓改动走对侧仓 PR**：本仓不修改 mdcore 任何文件，需要引擎能力就开 issue（见 mdcore#6）

---

## 4. 当前已提交的 PR

- **PR `feat/m0-skeleton`**
  包含 `#1`（编译修复）+ `#2`（bid-mcp crate）。
  这是"让标书系统第一次有一个可运行的、说人话的入口"的一步。

**为什么先做这个**：M0 里程碑原文是"领域模型持久化 + 状态机 + 事件存储 + 幂等键 +
**MCP server 骨架（12 工具）** + 内核适配器 stub + CI"，本仓已完成状态机，但骨架为零。
先补骨架，后续每个功能 PR 都只是往已存在的分发表里填一个 `invoke` 分支——
这是唯一能让后续 12 个 PR 互不阻塞的结构。

---

## 5. 不在本计划内的

- **UI / 前端**：标书系统当前是研究仓，UI 是产品化阶段的事，需先有服务端（#5）
- **引擎 G1/G2/G3**：由引擎侧完成（mdcore#6），标书侧不碰
- **模型 / 检索效果评测（S3）**：需要模型 provider 配置，本机不具备
