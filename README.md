# AI 标书系统 · mang bid

> 基于 [mmqz/mdcore](https://github.com/mmqz/mdcore) 引擎的投标文件生成系统。
> **状态：1.0 开发中**——链路已全线打通（P0 建项 → P8 导出），真实语料可跑通全流程。

## 快速上手

```bash
git clone https://github.com/mmqz/ai-bid-research.git
cd ai-bid-research/bidcore
cargo build --release
```

环境要求见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 能力面

### 八阶段链路

```
P0 建项  → create_bid_project
P1 导入  → import_tender_files（PDF / DOCX / XLSX / .doc 两层转换）
P2 抽取  → extract_requirements（语义表 + 评分表 + ★条款结构化）
P3 建纲  → build_outline（评分驱动 + 篇幅档位 + 覆盖闭环门）
P4 证据  → register_evidence / bind_evidence（台账 + 只降不升）
P5 起草  → draft_response_section / append_section_segment（增量写入）
P6 校验  → validate_bid_document（门禁 + BLOCKER 汇总）
P7 复核  → record_review_gate（签域四重绑定 + 提交就绪面）
P8 导出  → export_bid_document（篇幅门 + 矩阵 blocking 清零）
```

### 33 个 MCP 工具

```
建项导入（2）：create_bid_project / import_tender_files
抽取矩阵（3）：extract_requirements / get_compliance_matrix / set_matrix_override
建纲篇幅（2）：build_outline / recommend_length_profile
证据（4）：list_evidence / register_evidence / bind_evidence / search_evidence
起草校验（5）：draft_response_section / append_section_segment /
            rewrite_section_segment / list_section_segments / validate_bid_document
复核评分（7）：record_review_gate / record_addendum_waiver / check_bid_quality
              / score_price / score_section / score_document
              / check_scoring_coverage（#318 细则逐条响应覆盖检查）
全块覆盖（4）：get_document_chunks / submit_block_marks / get_coverage_report
              / get_star_coverage（★漏标检测）
导出恢复（4）：export_bid_document / get_bid_project_state / save_version
              / recover_validating_state
其他（3）：list_tasks / list_tools（#279 行协议发现面）
```

### 篇幅档位（四档）

| 档位 | 目标字数 | 约页数 | 适用 |
|---|---|---|---|
| `practical` | 400,000 | 474 | 小型采购、细则 < 10 |
| `standard` | 800,000 | 948 | 中型服务、细则 10-19 |
| **`conservative`（默认）** | **1,200,000** | **1422** | 大项目、细则 20-39 |
| `maximum` | 2,500,000 | 2962 | 超大型工程、细则 ≥ 40 |

页面密度实测 844 字符/页（97 页 PDF / 81,850 字符）。

`recommend_length_profile` 按细则数/★条款数/预算/采购类型/明写篇幅给出建议档位。

## 代码结构

```
bidcore/crates/
  bid-domain     领域模型（PRJ/TF/REQ/EV/CMP/SEC/VF/RT + 状态机）
  bid-parse      解析层（PDF/DOCX/XLSX 抽取 + 结构识别 + ★扫描）
  bid-engine     引擎桥（mdcore 集成 + 格式转换）
  bid-tools      MCP 工具实现 + 持久化
  bid-mcp        MCP 协议层（stdio 行协议 + JSON-RPC）
  bid-kb         知识库适配器
  bid-grade      评分/自评
  mang-app       桌面壳（注入 MCP 用的独立 workspace）
  mang-snap      快照
```

## 真实语料

```
fixtures/
  real-tender-batch1/   黑龙江政采 5 份（统一导出 PDF，「第X章」结构）
  real-tender-batch2/   跨平台 6 份（.doc/.docx/询价数字编号）
  songling-232762/      松岭项目 65 页真实招标文件 + 金标准
  corpus-A/             39 件合成语料
  corpus-R/             D1/D2/D5 确定性合成
  synth-tender/         25 埋点合成件
```

**语料纪律**：文件名不许改——文件名带项目名称和编号，是 Agent 的合法信息来源。

## 关键设计

### 三态覆盖分账

```
excluded_blocks   已判定与投标无关（Agent 显式排除）
unjudged_blocks   未判定（Agent 漏标）
unjudged_ranges   未判定区间

三者之和必须覆盖全部块——缺失即 E_COVERAGE_INCOMPLETE
```

### 只降不升

```
证据状态：Active → Expiring → Expired（单向）
任何 → Active 被拒绝（borrow mdcore degrade_linkage 纪律）
```

### 门禁分级

```
BLOCKER  阻断导出（必选项未响应、篇幅不足、矩阵未对账）
WARNING  告警但放行（覆盖不足、★漏标、配额偏离）
INFO     仅记录
```

## 文档索引

| 文档 | 内容 |
|---|---|
| [AGENTS.md](AGENTS.md) | **代理协作规则**——开工前必读 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 环境搭建 + 开发流程 |
| [docs/bid-domain-model.md](docs/bid-domain-model.md) | 领域对象 + 状态机 + ID 命名空间 |
| [docs/bid-mcp-contract.md](docs/bid-mcp-contract.md) | MCP 工具契约（33 个工具） |
| [docs/bid-skill-design.md](docs/bid-skill-design.md) | Skill 九阶段路径 + 停止条件 |
| [docs/architecture-decision.md](docs/architecture-decision.md) | AD-01~AD-07 决策记录 |
| [docs/threat-model.md](docs/threat-model.md) | 安全威胁模型 |
| [docs/doc-engine-audit.md](docs/doc-engine-audit.md) | 引擎能力审计 + 差距矩阵 |
| [audit/README.md](audit/README.md) | **架构审计报告（含假绿登记 FG-01~18）** |
| [research/](research/) | 研究阶段产出（18 仓库对比、验证计划） |

## 验证状态词典

```
SIMULATED          Python 仿真通过（≠ 实装）
IMPLEMENTED        Rust 实现完成且有测试
INTEGRATION_TESTED 真实语料跑通
VERIFIED_SOURCE   源码级核验
PENDING           待验证
```

**纪律**：仿真通过不计入实装通过，分记不折抵。

## 测试

```bash
cd bidcore
cargo test --workspace              # 全量
cargo test -p bid-domain            # 领域层
cargo test -p bid-tools             # 工具层 + 真实语料
```

### 环境门（重要）

涉及真实语料 / 外部工具（poppler / tesseract / Word COM）的测试必须加门：

```rust
use bid_parse::env_preflight::{skip_if_env_missing, POPPLER_TEXT_TOOLS};

#[test]
fn my_real_corpus_test() {
    if skip_if_env_missing("my_real_corpus_test", POPPLER_TEXT_TOOLS) { return; }
    // ...
}
```

**理由**：CI 上没有这些工具；不加门 → CI 红；静默 return → 假绿。
详见 [AGENTS.md](AGENTS.md) 的「三之二、测试环境门」。

## 已知缺口

见 [audit/false-green-register.md](audit/false-green-register.md)——18 条「假绿」登记：

```
FG-01  mang-app 39 测试零执行
FG-02  真件链在 CI 上恒 SKIP 恒绿
FG-03  Verify SKIP banner 步骤吞退出码
FG-04  T18a 引擎路径评测断裂
...
```

以及 [docs/doc-engine-audit.md](docs/doc-engine-audit.md) 的引擎差距矩阵 G1~G11。

## 许可证

见 [LICENSE](LICENSE)（待补）。

---

**最后更新**：2026-11-06