# Workspace 与构建入口盘点（workspace-inventory.md）

基线：6105beb。数据源：`git ls-files` + 各 manifest 实读。

## 1. Cargo 工程（共 17 个 Cargo.toml）

| # | 工程 | 类型 | CI 覆盖 | 说明 |
|---|---|---|---|---|
| 1 | `bidcore/` | workspace 根（resolver=2，8 members） | ✅ windows-check | 核心业务 workspace |
| 2 | `bidcore/crates/bid-domain` | lib | ✅（随 workspace） | 领域模型 |
| 3 | `bidcore/crates/bid-parse` | lib | ✅ | 解析引擎 |
| 4 | `bidcore/crates/bid-kb` | lib | ✅ | 知识库 |
| 5 | `bidcore/crates/bid-tools` | lib | ✅ | 业务服务 |
| 6 | `bidcore/crates/bid-grade` | lib | ✅ | 判分器 |
| 7 | `bidcore/crates/bid-mcp` | lib | ✅ | MCP 工具面 |
| 8 | `bidcore/crates/bid-engine` | lib | ✅ | mdcore 引擎适配（唯一接触面） |
| 9 | `bidcore/crates/mang-snap` | lib | ✅（随 workspace） | 快照（成员） |
| 10 | `bidcore/crates/mang-app` | **独立 workspace** | ✅（CI 单独 step） | 桌面壳 tao+wry，直链 bid-domain/bid-tools/bid-mcp/mang-snap（绕过 MCP 层！） |
| 11 | `m1/` | 独立 package | ❌ 无 CI | 研究验证最小流水线（zip+quick-xml 直用——早于引擎收敛） |
| 12 | `scripts/baseline_harness/` | 独立 | ❌ | 基线对比 harness |
| 13 | `scripts/bid_health_check/` | 独立 | ❌ | 真实语料健康检查（REPORT.md 生成器） |
| 14 | `scripts/engine_probe/` | 独立 | ❌ | 引擎探测 |
| 15 | `scripts/title_impact/` | 独立 | ❌ | 标题影响测量 |
| 16 | `scripts/veto_gt/` | 独立 | ❌ | 否决 GT 生成 |
| 17 | `scripts/veto_measure/` | 独立 | ❌ | 否决召回测量 |

## 2. mdcore 跨仓依赖（R7 预检）

`bidcore/crates/bid-engine/Cargo.toml` 三个 git 依赖**同 rev 钉住**（符合单源纪律）：

```toml
mdc-office-io = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", ... }
mdcore         = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", ... }
mdc-pdf        = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", features = ["engine-pdfium"] }
```

- rev = mdcore main @ f4328ebf（#110 任务 2 bump，自 3204342d 起 87 提交）。
- mdc-pdf 走 engine-pdfium 特性：pdfium 动态库 env `MDC_PDFIUM_LIB`，无 pdfium 环境走 SKIP 语义。
- mdcore 为私有仓：CI 用只读 deploy key；本审计沙箱用 PAT 拉取（不入库不入日志）。

## 3. 前端工程

| 工程 | 入口 | scripts | CI |
|---|---|---|---|
| `web-ui/` | package.json | build / serve / typecheck（**无 test script、无浏览器 E2E 框架**） | ❌ 无 CI |

待核：package.json 实读、依赖树、构建产物。

## 4. Python 面

| 面 | 文件 | CI |
|---|---|---|
| `scripts/*.py`（~44 个顶层脚本） | run_tests.py 主链 16 T 组、v1/v2/v3 评测、telecom_*、real_eval_*、gh_*、ooxml_util 等 | ❌ |
| `scripts/fp_attribution/`、`scripts/e2e_bid/`、`scripts/mang-dev/`、`scripts/veto_gt/` | 子包脚本 | ❌ |
| `sdk/` | skill/bid-response-pipeline（checklist.py）、examples/minimal_flow.py、tests/test_mcp_e2e.py | ❌ |
| `bidcore/crates/bid-mcp/tests/mcp_sdk_e2e.py` | MCP↔SDK 端到端 | ❌（cargo 集成测试伴生） |

## 5. 构建配置

- `.cargo/config.toml`：仅 Windows MSVC 静态链接（+crt-static）。无其他构建配置。
- `.github/workflows/windows-check.yml`：唯一 CI（详见 baseline.md）。
- 无 rust-toolchain.toml（CI 用 dtolnay/rust-toolchain@stable 浮动 stable）。
- Cargo.lock：bidcore/ 一份（8+1 members 锁）；mang-app 共享 bidcore 锁？待 cargo metadata 核实（mang-app 独立 workspace 有无自己的 lock）；scripts 六工程各有独立 lock 与否待核实。

## 6. 工程覆盖结论（初步）

- **CI 只覆盖 bidcore workspace（Windows）+ mang-app check**；scripts 六个 Rust 工程、m1、web-ui、全部 Python 无 CI——评测链（含 run_tests.py 主链与 real_eval_*）完全依赖本地手跑。
- mang-app 直链 bid-tools/bid-domain（方案 A，不经 MCP）——R1 §3.8 必审：桌面端是否绕过 MCP 权限/状态门禁。
- m1 为遗留研究实现（zip+quick-xml 自解析），需追踪是否仍被调用。
