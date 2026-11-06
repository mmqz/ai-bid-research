# 测试执行与测试质量审计（test-quality-audit.md）

基线 6105beb。本文件 = 实际运行结果（R5 证据）+ 假绿分析。
运行环境：Linux x86_64、rustc 1.99.1 stable、poppler 全套 + tesseract 5（**仅 eng/osd，无 chi_sim**）、pdfium 缺失、Python 3.12.16、mcp SDK 1.27.0。

## 1. 实际执行结果汇总（六类分开统计）

### 1.1 bidcore workspace（cargo test --workspace 等价：逐二进制 105 套件）

| 面 | 套件数 | PASS | FAIL | SKIP | IGNORED | NOT_EXECUTED | BLOCKED |
|---|---|---|---|---|---|---|---|
| bid-domain | 7+lib | 81+180 | 0 | 0 | 0 | 0 | 0 |
| bid-parse | 14+lib | 87+148 | 0 | 0 | 0 | 0 | 0 |
| bid-kb | 2+lib | 9 | 0 | 0 | 0 | 0 | 0 |
| bid-tools | 29+lib | 268+28 | 0 | 0 | 0 | 0 | 0 |
| bid-grade | 2+2bin-lib | 2 | 0 | 0 | 0 | 0 | 0 |
| bid-mcp | 36+lib+3bin-lib | 242+19 | 0 | 0 | 0 | 0 | 0 |
| bid-engine | 7+lib | 49+1 | 0 | 0 | 0 | 0 | 0 |
| mang-snap | lib | 4 | 0 | 0 | 0 | 0 | 0 |
| **合计** | **105** | **1,118** | **0** | **0** | **0** | 0 | 0 |

- 说明：`cargo test --workspace --no-fail-fast` 分三块跑完 + 逐二进制复跑（含 real_corpus_e2e 单独跑 232s 通过）。
- **1,118 通过 / 0 失败**——基线官方 Rust 套件全绿（Linux 环境）。
- 一次环境性红：直跑测试二进制时 `env_preflight::tests::skip_helper_returns_true_only_when_missing` 断言 `cargo` 在 PATH——脱离 cargo 运行即红（P3 测试健壮性：bid-parse/src/env_preflight.rs:367-370 设计假设 cargo test 环境）。
- **SKIP 横幅 0 次**：本机 poppler/tesseract 在位，PDF 链真跑；但 **chi_sim 缺失**时 e2e_m1/real_corpus_e2e 仍通过——OCR 通道是否真被覆盖见 §4。
- 外部引擎面：pdfium 缺失（MDC_PDFIUM_LIB 未设）——mdc-pdf engine-pdfium 特性的 SKIP 语义未被本轮触发任何红/绿差异（未观察到 SKIP 横幅，可能测试全部走 poppler 路径）。

### 1.2 mang-app 独立 workspace（BLOCKED → 静态裁决）

- Linux 无 gdk-3.0/webkit2gtk 且无 root——**cargo test 无法在审计沙箱执行（BLOCKED）**。
- 静态确证（D-11）：`diag.rs:303` 断言 `v["tools"]==13`，而 `mcp_status()` 实际计数 `tools_schema()` 数组=**25**（bid-mcp/src/lib.rs:53 TOOL_NAMES）→ **mang-app cargo test 必红（确定性）**；CI 对 mang-app 只 `cargo check --all-targets`（编译不执行）→ 39 个测试（含 fs 围栏安全测试）**在任何 CI 都从未执行**。
- 结论：NOT_EXECUTED（CI 面）+ BLOCKED（本沙箱）+ CONFIRMED-static-red。

### 1.3 m1（cargo test）

- 5 通过 / 0 失败。

### 1.4 scripts 六个 Rust 工程（cargo test）

- baseline_harness / bid_health_check / engine_probe / title_impact / veto_gt / veto_measure：**全部 0 个测试**（纯二进制工具）。评测链 Rust 侧无单元守卫。

### 1.5 Python 主链（scripts/run_tests.py，T0-T19）

**51 通过 / 3 跳过 / 3 失败**：

| 项 | 状态 | 定性 |
|---|---|---|
| T3a manifest 双向校验（55 文件） | FAIL | 真实夹具卫生问题：游离文件未入清单（corpus-A/files/D-技术标-第一章.md、synth-tender/README.md、expected-format-spec.json）——P3 |
| T18a 引擎路径召回≥旧路径 | FAIL | **评测腐化**：b5_engine_gt_eval.py 生成 example 调 `bid_parse::parse_file(path, "A-招标文件.docx")`——B4 后该函数仅支持 PDF（bid-parse/src/lib.rs:120-139 显式拒绝 docx）→ panic「不支持的文件类型 .docx」。自 B4 合入起红，无 CI 监视——P2（同时使「引擎路径抽取质量≥旧路径」声明失去可复现证据链） |
| T19a bidcore 测试全绿 | FAIL | **环境失败**（沙箱磁盘满，ironcalc_base 编译失败）——非基线问题；本审计 §1.1 已单独复跑全绿 |
| T0c chi_sim / T4c-OCR / T10-T13 m1 线 | SKIP | ENV_MISSING（chi_sim 不在 ~/.tessdata）——SKIP 语义诚实（横幅+单列计数），未伪装通过 ✅ |

### 1.6 SDK E2E（sdk/tests/test_mcp_e2e.py，mcp 传输，真进程）

**9/10 通过，1 失败（TRUE_RC=1）**：

| 用例 | 结果 |
|---|---|
| 00-environment / 01-initialize-and-tools / 03-error-envelope | PASS |
| 10-external-ghost-ev / 11-uncovered / 12-unanchored / 13-cas-version / 14-search-evidence（6 子组） | PASS |
| **02-full-pipeline** | **FAIL** |

02-full-pipeline 失败根因（三层）：
1. **产品面（正向验证）**：#242 E_CHARS_BELOW_MIN 门真实生效——external 模式夹具正文 344/119/256/166 字 < 最低 400 字（目标 500×80%），validate 返回 22 个 BLOCKER、compliance/structural 两桶 failed——**门本身按设计工作**（这是 #242 的独立运行证据）。
2. **测试面（腐化）**：SDK E2E 夹具内容长度写死于 #242 之前，合入 #242 时无人重跑该 E2E（不在 CI）→ 旗舰全链验收在基线即红——**P1 测试基建发现**。
3. **可诊断性**：失败以「unhandled errors in a TaskGroup (1 sub-exception)」呈现，子异常不进汇总、evidence JSON 的 error 字段为 null——断言细节丢失，排障成本高——P3。

附带：默认二进制定位 `/tmp/shared-target/debug`（BID_MCP_TARGET_DIR 可覆写）——机器本地路径默认值（D 类发现）。

### 1.7 MCP 官方 SDK 握手（bidcore/crates/bid-mcp/tests/mcp_sdk_e2e.py）

- 1 套件通过（5s）：initialize 协商 + **25 工具全量在座**（与 TOOL_NAMES 对齐，含 set_matrix_override）+ call_tool 成功/工具错误/协议错误三面。
- 文档漂移再证：该文件头注写「工具面 24」而 EXPECTED_TOOLS=25——21/24/25 三版本并存（C 类发现互证）。

## 2. 测试类型分类（按 R5 分类法映射）

| 类别 | 载体 | 规模 | 备注 |
|---|---|---|---|
| UNIT | 各 crate lib 内 #[cfg(test)] | ~630 | bid-domain 180/bid-parse 148/bid-tools 28/bid-mcp 19 等 |
| CONTRACT | bid-mcp tests/*（dispatch_schema、各工具 *_tool.rs） | ~243 | 工具参数/错误码/状态门契约 |
| INTEGRATION | bid-tools tests（service_import、store_roundtrip、export_* 等） | ~270 | 服务层+持久化 |
| E2E | e2e_m1、real_corpus_e2e、writing_acceptance_e2e、mcp_sdk_e2e.py、sdk 02-full-pipeline | ~6 | 真 import→export 全链 |
| REAL_CORPUS | real_corpus_223（232s）、T14/T16 真实链、scoring_real_corpus | 数十断言 | 政采公告级 + 电信文件级 17/17 |
| SEMUL | Python v1_*/v2_*/v3_*（T7-T9）、fp_*、m17_* | 16 T 组 | **头注自认仿真实现，与 Rust 生产无交叉验证**（D-22） |
| SNAPSHOT | （未见快照测试框架） | 0 | — |
| NEGATIVE | guard/zip/xml 负向族、破坏注入 T5（13 类）、xref --inject | ~100+ | 历史上最强面 |
| MOCK | draft mock（e2e 用 mock LLM）、llm.json mock 降级 | 少量 | 明确标注 |
| MANUAL | docs/mcp-e2e-verification.md（#139 人工验收记录） | 1 | 已入档 |

## 3. 测试副作用（非封闭性）——实测证据

跑测试会污染工作区（2026-11-06 实测，已恢复）：

1. **改写已提交产物**：artifacts/e2e/latest/08_export.pdf、10_report.md（run-id 时间戳漂移 e2e-20261104→e2e-20261106）、real-corpus-e2e/latest/A-*/09_export-meta.json——Rust e2e 测试直接覆写库内产物。
2. **重写 Cargo.lock**：scripts/{bid_health_check,title_impact,veto_gt,veto_measure}/Cargo.lock 因本地 cargo 1.99 解析变化被改（602/509/334/211 行 diff）。
3. **产生游离中间明细**：artifacts/real-corpus-e2e/latest/*/02_imported.json…08_validation.json（未 gitignore）+ 杂项目录 bidcore/2/。
4. 影响：「评测产物=特定一次运行的快照，重跑必 diff」——**产物不可作为确定性基线**（回答审计问 5 的关键证据）；CI 无感（临时 runner），本地开发者每次跑测试都弄脏树。

## 4. 测试可信度裁定

**可信的部分**：
- Rust 侧 1,118 测试真实执行生产代码（无 Mock 假绿迹象；NEGATIVE 族质量高）。
- SKIP 纪律（issue #9）诚实：横幅+单列+ENV_BLOCKED.md 账本，T0 预检先登记。
- MCP 官方 SDK 握手 25 工具真实验证。

**不可信/盲区（按危害排序）**：
1. **mang-app 39 测试零执行**（CI 只 check）+ 确定性红（tools==13 断言过期）。
2. **Python 主链完全无 CI**：T18a 评测腐化（B4 起红无人知）、SDK 02-full-pipeline 红（#242 起红无人知）——两个「旗舰验收」都处于失守状态。
3. **SEMUL 仿真链无交叉验证**：T7-T9 的「V1/V2/V3 验证」跑的是 Python 仿真实现，不进 Rust 生产代码（D-22）——历史文档大量「全绿」结论实为仿真绿。
4. **OCR 链覆盖缺口**：chi_sim 缺失时 e2e_m1/real_corpus_e2e 不红不 SKIP（本轮实测）——真实语料的 OCR 件路径覆盖情况存疑，需 chi_sim 环境复跑（NEEDS_ENV）。
5. **引擎路径评测断裂**（T18a）使「引擎抽取质量」声明失去当前证据。
6. run_tests.py T19 在沙箱磁盘满时把环境失败计为 FAIL（诚实），但注意其内嵌 `cargo test` 重复编译浪费且受磁盘约束。

## 5. Mutation Testing（P0/P1 不变量白名单——抽样执行）

见 mutation-report.md（对 validate 状态推进、bind can_bind、导出门、receipt 验签等做变异并验证对应测试变红）。
