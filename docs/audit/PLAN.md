# ai-bid-research 全仓代码审计与测试重建计划（原文）

> 本文是 issue #203 的执行依据全文。
> 启动前置条件与 6 条修订见 issue #203。
> 除 issue 中的 6 条修订外，本文其余条款全部有效。

---
# ai-bid-research 全仓代码审计与测试重建计划

**任务性质：** 全仓库代码级测试审计，不是单模块补测试。

**核心目标：** 找出实现缺陷、接口断裂、错误状态转换、静默失败、测试假绿、测试与生产实现脱离等问题。

**最高原则：测试必须能够证明代码实际完成了业务行为，而不是证明某段测试代码能够运行。**

---

# 一、仓库范围

## 1. 主仓库：必须完整审计

`mmqz/ai-bid-research`

本轮覆盖所有第一方源码、公开接口、可执行脚本、构建配置和 CI。

| 目录 | 审计范围 | 必须验证 |
|---|---|---|
| bidcore | Rust 全部 crates | 领域逻辑、解析、服务、MCP、引擎集成 |
| sdk | 全部 SDK 源码 | 接口一致性、输入输出、异常处理 |
| web-ui | 全部前端源码 | 交互、数据状态、API 契约 |
| scripts | 全部可执行脚本 | 是否执行真实代码、结果是否可信 |
| .github/workflows | 所有工作流 | 测试是否被执行、是否存在假绿 |
| fixtures | 全部测试夹具 | 是否与实际业务结构一致 |
| real-corpus | 全部真实语料 | 真实文件解析与业务正确性 |
| real-samples | 全部样本 | 输入覆盖和独立预期结果 |
| baseline | 历史基准 | 基线是否过时、是否错误放宽 |
| m1 | 阶段代码 | 是否仍被调用、是否存在重复实现 |
| artifacts | 评测产物 | 是否能从源码和命令重新生成 |
| docs | 业务及接口契约 | 文档是否与代码一致 |
| research | 设计、风险、场景库 | 设计假设是否被真实实现验证 |

注意：

- 不允许因为某个目录没有 `tests` 子目录，就不审计它。
- 不允许只审计已有测试覆盖的代码。
- 历史目录中的可执行代码也必须识别，确认是否仍被产品引用。
- 生成文件、第三方依赖、历史归档可以排除行为测试，但必须登记排除原因。
- 对所有第一方源文件建立清单，不允许出现未归类文件。

## 2. 外部依赖仓库

### mmqz/mdcore

作为实际集成依赖进行只读审计。

重点核验：

- BlockId 和 Locator 契约。
- 文档导入与导出。
- 证据锚点及状态。
- 内容版本和外部修改检测。
- Undo / 重写 / 锚点失效。
- ReviewPolicy。
- PDF、DOCX、XLSX 的能力边界。
- 业务侧与 mdcore 的数据转换是否丢字段。

测试原则：

在 ai-bid-research 中建立真实的跨仓集成测试，使用实际锁定 SHA 的 mdcore。

不能以 Mock 替代 mdcore 实际调用。

发现引擎缺陷时，另行生成 mdcore 回归案例，不擅自修改它的实现。

### mmqz/cleanroom-analysis

作为已有审计证据和基准来源，只读检查。

不重新审计整个项目，主要确认历史审计结论是否与当前代码一致。

---

# 二、R0：全仓源码盘点

这是第一阶段，禁止跳过。

先让 Agent 自己发现全部代码，不能预先只给它指定几个 crate。

执行：

```bash
git ls-files
```

Rust：

```bash
cd bidcore
cargo metadata --no-deps --format-version 1
cargo test --workspace --no-run
cargo tree --workspace
```

检查：

- 全部 workspace members。
- 全部 crate。
- 全部 library。
- 全部 binary。
- 所有 feature flags。
- 所有 integration tests。
- 所有 example。
- 所有 build.rs。
- 所有 proc-macro。
- 所有外部子进程调用。
- 所有可选编译路径。

其他语言：

检查 `sdk`、`web-ui`、`scripts` 中实际存在的：
- package.json / lockfile。
- pyproject.toml / requirements 文件。
- tsconfig。
- 构建配置。
- 可执行入口。
- 测试入口。

最终生成：

`docs/audit/full-source-inventory.csv`

字段：

```text
source_file
language
module
public_entrypoints
dependencies
current_tests
integration_tests
risk_level
audit_status
known_gaps
```

**验收要求：**

所有第一方源文件必须有记录。

不允许出现：
- 未审计文件。
- 不知道调用方的公开函数。
- 没有确认用途的历史代码。
- 无人负责的业务模块。

---

# 三、R1：逐模块代码审查

审查不是只看测试文件，而是先读生产源码，再检查测试能否保护对应行为。

## A. bid-domain：领域模型和状态机

检查：

- 所有业务 ID。
- 数据结构和序列化。
- 状态转换。
- 业务不变量。
- 版本控制。
- 幂等性。
- 错误传播。
- 领域对象间引用关系。

专项：

1. Project 状态机。
2. TenderFile 身份和角色。
3. Requirement 生命周期。
4. Evidence 三态。
5. Linkage 只降不升。
6. Addendum 影响面。
7. Outline 构建和更新。
8. ScoringItem 及评分映射。
9. ReviewRecord。
10. ExportGate。
11. 状态恢复和事务一致性。

每个状态转换必须测试：

- 正常路径。
- 非法路径。
- 重复操作。
- 并发操作。
- 中途失败。
- 重启恢复。
- 旧版本操作。

特别检查：

审核后修改正文是否使审核失效；补遗是否正确使相关章节失效；Undo 是否错误恢复已经失效的证据。

## B. bid-parse：所有文件解析器

不只测试 PDF。

覆盖：

| 输入类型 | 必测内容 |
|---|---|
| PDF 文本层 | 字体、编码、CMap、页码 |
| PDF 扫描件 | OCR、页数、空页 |
| 混合 PDF | 文本页与扫描页混合 |
| PDF 表格 | 表格结构、金额、跨页 |
| DOCX | 段落、表格、图片、页眉页脚 |
| XLSX | 工作表、单元格、合并单元格、公式、坐标 |
| XML | 实体、深度、编码、非法结构 |
| ZIP | 路径穿越、重复条目、压缩炸弹 |
| 图片 | 图片解码、尺寸和容量限制 |

同时检查格式兼容性：

- Office / WPS 生成的文件。
- 不同 LibreOffice 版本生成的文件。
- 扩展名与实际文件格式不一致。
- 文件损坏。
- 加密文件。
- 空文件。
- 超大文件。
- 恶意构造文件。

PDF #181/#200 的完整专项测试纳入这里，不另行缩小全仓范围。

## C. bid-tools：业务服务层

这是高优先级审查区域。

检查每个公开 Service 方法：

- 输入是否验证。
- 状态是否检查。
- 数据是否持久化。
- 失败是否回滚。
- 是否有部分成功。
- 是否存在静默跳过。
- 是否存在重复写入。
- 是否错误修改其他业务对象。

重点业务链：

```text
项目创建
  ↓
招标文件导入
  ↓
补遗导入
  ↓
条款提取
  ↓
需求整理
  ↓
证据登记
  ↓
证据匹配
  ↓
大纲生成
  ↓
评分映射
  ↓
章节生成
  ↓
质量校验
  ↓
人工审批
  ↓
DOCX 导出
  ↓
最终回读
```

每个阶段都必须验证输入、输出和持久化状态。

## D. bid-engine：文档引擎适配

检查：

- mdcore 调用是否使用真实实现。
- 文档块 ID 是否正确保存。
- 块 ID 和业务 ID 是否混用。
- Locator 是否失效。
- 文档修改后是否正确标记 Stale。
- 外部文件修改是否被检测到。
- 导出后引用是否仍然有效。
- Undo 是否破坏关联。
- 导出失败是否造成虚假 EXPORTED。

必须做跨仓库真实集成测试。

## E. bid-mcp：全部 MCP 工具

不能只验证 MCP Server 能启动。

对每个工具逐一检查：

- 参数 Schema。
- 必填字段。
- 类型边界。
- 非法参数。
- 文件路径权限。
- 工具调用权限。
- 状态门禁。
- 返回结构。
- 错误码。
- 超时。
- 幂等性。
- 重复调用。
- 并发调用。
- 超大输出。
- 部分失败。

必须对照 MCP 契约文档检查实际实现。

重点检查 Agent 是否能够：
- 越权调用。
- 伪造人工审批。
- 直接访问任意本地文件。
- 绕过状态机。
- 重复执行不可重复操作。

## F. SDK

先建立 SDK 完整文件清单，然后验证：

- SDK 与 MCP 参数一致。
- 类型定义与真实响应一致。
- 错误处理没有丢失。
- 超时和取消行为正确。
- 空值和可选字段处理正确。
- 版本兼容。
- 重试是否造成重复操作。

SDK 每个公开接口必须至少有一次真实服务集成测试。

## G. web-ui

必须把前端纳入此次审计。

检查：

- 每个页面和路由。
- 所有组件。
- 所有 API 调用。
- 数据状态管理。
- 错误显示。
- 加载状态。
- 表单校验。
- 文件上传。
- 大纲编辑。
- 章节编辑。
- 评分展示。
- 证据展示。
- 人工审核。
- 导出。

重点场景：

1. 后端操作成功，前端是否正确更新。
2. 后端操作失败，前端是否错误显示成功。
3. 页面刷新后状态是否丢失。
4. 多标签页修改是否出现旧数据。
5. 修改内容后审核状态是否立即失效。
6. 证据失效是否展示给用户。
7. 导出过程中断是否显示虚假完成。
8. 后端返回部分成功时，UI 是否错误显示全部完成。

要求使用浏览器真实 E2E，而不是只测试 React 组件能够渲染。

## H. scripts / research / CI

这是本轮重点审查的测试基础设施。

逐个检查：

- `scripts/v1_*`
- `scripts/v2_*`
- `scripts/v3_*`
- 真实文件评测脚本。
- SEMUL 脚本。
- 负向测试。
- Mutation 测试。
- GitHub Actions。
- 环境预检。
- 测试报告生成。

每个脚本都必须回答：

1. 它调用的是生产代码还是复制实现？
2. 它使用真实文件还是手工构造对象？
3. 它的预期值从哪里来？
4. 它是否与被测代码共享同一套算法？
5. 它是否可能在异常情况下输出成功？
6. 它的产物能否独立复现？
7. 它是否测试了当前 main，而不是历史 commit？

所有历史验证产物必须确认仍然可重现。

---

# 四、R2：测试体系质量审计

本轮最重要的目标：杀死假测试。

对全仓测试逐一分类：

| 分类 | 定义 | 能否作为功能验收 |
|---|---|---|
| Unit | 单函数真实逻辑 | 可以，但仅证明局部行为 |
| Contract | 模块输入输出契约 | 可以 |
| Integration | 多模块真实调用 | 可以 |
| E2E | 用户真实业务路径 | 可以 |
| SEMUL | 独立仿真 | 不可当作实装通过 |
| Mock | 模拟依赖 | 只用于边界测试 |
| Snapshot | 快照对比 | 不能单独证明正确 |
| Fixture | 固定夹具 | 必须有独立 Oracle |
| Negative | 故障与恶意输入 | 必须真正触发拒绝 |
| Mutation | 主动破坏代码 | 必须能被测试捕获 |

逐项检查这些反模式：

- 仅检查返回值不是 null。
- 只检查字符串包含关键词。
- 只检查数组非空。
- 只检查状态等于成功。
- 用手写的结果对象替代生产调用。
- 测试函数和实现函数共享同一错误逻辑。
- 捕获所有错误并判为预期成功。
- 失败转 SKIP。
- 只测试 mock，不测试真实依赖。
- 只测 happy path。
- `#[ignore]` 长期挂起而没有负责人。
- 测试实际没有进入目标代码分支。
- 通过降低阈值制造绿色结果。

尤其要检查 `scripts` 的仿真与 Rust 实现是否重复实现了同一业务算法。

两个实现都通过相同的错误假设，不构成双路验证。

---

# 五、R3：全业务链路真实测试

至少建立以下八套场景。

### S1：普通招标文件

真实文件导入、提取需求、生成大纲、起草、导出。

验证内容正确，而不仅是流程完成。

### S2：多份补遗

测试新增、删除、修改、冲突、日期变更、评分变更。

验证影响面准确，不漏更新、不误伤。

### S3：证据不足

要求必须提供资质或业绩，但不存在对应材料。

验证系统明确报告缺失，而不是编造证明材料。

### S4：复杂评分

包括：
- 一对一映射。
- 一对多映射。
- 多对一映射。
- 未映射。
- 不适用。
- 跨章节评分。
- 评分规则不完整。

检查 #170 的完整修复。

### S5：复杂文档

PDF、DOCX、XLSX 混合输入，复杂表格、跨页表格、扫描件。

### S6：重复和并发

重复上传、重复调用、并发修改、并发导出、请求超时后重试。

### S7：人工审核

审批、版本修改、撤销审核、重新审核、导出授权、审批人身份验证。

### S8：导出与恢复

生成 DOCX、独立回读、PDF 转换、中断恢复、重复导出、暗标检查。

全部从真实公开入口执行。

---

# 六、R4：安全与可靠性专项

威胁模型中的每一项威胁必须对应一个可执行测试。

重点：

1. 路径穿越。
2. 符号链接逃逸。
3. 压缩炸弹。
4. 超大文件。
5. 恶意 XML。
6. PDF 对象资源耗尽。
7. 子进程挂死。
8. 输出无限增长。
9. 并发资源耗尽。
10. 文件伪造 SHA。
11. 证据来源伪造。
12. 审批身份伪造。
13. 审批后内容修改。
14. Prompt Injection。
15. 恶意招标文件诱导 Agent 越权。
16. 日志泄漏敏感信息。
17. 导出残留元数据。
18. 外部依赖不可用。
19. 网络/API 超时。
20. 持久化中断。

每个威胁测试必须包含：

- 攻击输入。
- 真实执行入口。
- 预期拒绝状态。
- 实际副作用检查。
- 日志与审计记录。
- 回归用例。

---

# 七、R5：测试有效性证明

不仅要让测试通过，还要证明测试能够发现错误。

建立全仓 Mutation Testing。

按模块进行针对性变异：

| 模块 | 变异 |
|---|---|
| Domain | 删除状态转换限制 |
| Parse | 删除编码映射、丢弃文本页 |
| Guard | 删除一个安全校验 |
| Tools | 跳过持久化、忽略错误 |
| Evidence | 取消来源校验 |
| Outline | 删除需求映射 |
| Scoring | 取消评分项检查 |
| Review | 删除审核版本校验 |
| Export | 跳过最终验证 |
| MCP | 放宽参数和权限检查 |
| Engine | 忽略锚点失效 |
| SDK | 忽略错误响应 |
| UI | 操作失败仍显示成功 |

要求每个高危变异都被对应测试捕获。

不追求无意义地让每一行都达到 100% 覆盖率。

追求关键业务不变量有测试保护。

---

# 八、R6：CI 全面重建

至少分成以下工作流：

| Job | 覆盖 |
|---|---|
| Rust Core | 全部 Rust crates |
| Rust Integration | 真实模块集成 |
| Parser Matrix | PDF / DOCX / XLSX / OCR |
| Security | 负向测试和故障注入 |
| SDK | 全部 SDK |
| Web UI | 类型检查、单测、浏览器 E2E |
| Real Corpus | 真实文件黄金测试 |
| Cross Repo | 固定 SHA 的 mdcore 集成 |
| Mutation | 核心业务不变量破坏测试 |
| Reproducibility | 评测脚本和报告重建 |

必须单独统计：

```text
PASS
FAIL
SKIP
IGNORED
NOT_EXECUTED
```

五种状态不能混为一谈。

任何 P0/P1 测试未执行或 SKIP，CI 不得宣称全套验证通过。

环境缺件应该单独报告，不允许用普通绿色覆盖。

---

# 九、R7：最终全仓交付

必须输出：

```text
docs/audit/
  full-source-inventory.csv
  module-audit.md
  test-quality-audit.md
  false-green-register.md
  production-path-coverage.md
  risk-register.md
  integration-contract-audit.md
  full-test-matrix.md
  mutation-report.md
  ci-audit.md
  final-verification-report.md
```

每项问题记录：

```text
ID
源文件路径
函数或接口
问题描述
风险等级
复现条件
失败测试
修复建议
回归测试
状态
```

还需要提供：

- 所有实际执行命令。
- Cargo / Rust 版本。
- Node / Python 等运行时版本。
- 外部依赖版本。
- 测试开始和结束 commit SHA。
- 测试原始日志。
- 真实语料 SHA256。
- 失败案例原始输入。
- 修复前后对比。
- CI 对应执行记录。

---

# 十、执行阶段与权限纪律

## 第一阶段：只读全面审计

R0 + R1 + R2。

禁止修改生产代码。

目标是得到完整源代码清单、测试质量报告和缺陷清单。

## 第二阶段：补测试，不修生产代码

根据审计结果，建立能够真实复现缺陷的失败测试。

要求至少包含：
- 生产公开入口。
- 独立 Oracle。
- 真实数据。
- 负向输入。
- 故障注入。

测试必须先证明当前代码存在问题。

## 第三阶段：逐模块修复

按 P0 → P1 → P2 修复。

不允许同时无边界重构多个模块。

每批修改必须包含对应回归测试。

## 第四阶段：全链路回归

重新执行全部核心测试。

检查修复是否造成其他模块回归。

## 第五阶段：独立复审

由不参与主实现的审查者重新检查：

- 缺陷是否真正修复。
- 测试是否真实有效。
- 是否存在测试绕过。
- 是否出现新的静默失败。
- 是否存在只修改测试、不修改实际功能的假修复。

---

# 十一、最终验收门槛

不使用新增测试数量、Cargo 总测试数量或单纯代码覆盖率作为项目质量指标。

必须满足：

1. 100% 第一方源码文件完成归类。
2. 100% 对外公开接口完成契约检查。
3. 所有 P0/P1 业务不变量有真实测试保护。
4. 所有核心状态转换有正向和负向测试。
5. 所有关键业务链至少有一条生产代码 E2E。
6. 所有支持文件格式有独立黄金样本。
7. 所有安全威胁有明确测试锚点。
8. 所有历史评测脚本完成真实性审查。
9. 所有核心测试的 SKIP、IGNORED、NOT_EXECUTED 可以被单独发现。
10. 核心高危变异不能存活。
11. 跨仓库集成测试使用实际依赖版本。
12. 不允许以文档修改替代功能修复。

**最终目标不是证明仓库有很多测试，而是证明仓库里的关键功能、关键状态和关键安全边界真的能经受破坏性测试。**# 全仓审计执行补充：本地 Agent 操作规程

本文件补充 `Full Repository Test Audit & Reliability Reset`。

## 1. 第一条命令：冻结审计基线

必须在本地真实仓库根目录执行：

```powershell
git status --short
git branch --show-current
git rev-parse HEAD
git ls-files
```

创建审计目录：

```powershell
New-Item -ItemType Directory -Force docs/audit | Out-Null
git ls-files | Set-Content -Encoding utf8 docs/audit/tracked-files.txt
```

记录运行环境：

```powershell
rustc --version
cargo --version
node --version
npm --version
python --version
```

不存在的运行时必须如实记录，不能自行安装后覆盖初始环境记录。

检查 Cargo：

```powershell
cargo metadata `
  --manifest-path bidcore/Cargo.toml `
  --format-version 1 `
  --no-deps `
  > docs/audit/cargo-metadata.json
```

注意：命令失败必须记录，不能通过删除相关 crate 或修改配置让盘点继续显示成功。

## 2. 找出所有真实测试入口

Rust：

```powershell
Get-ChildItem bidcore -Recurse -File -Filter Cargo.toml

git grep -n -E '#\[test\]|#\[tokio::test\]|#\[ignore' -- bidcore
```

Python：

```powershell
git grep -n -E 'pytest|unittest|assert |skipif|skip\\(' -- scripts
```

JavaScript / TypeScript：

```powershell
git grep -n -E 'describe\\(|it\\(|test\\(|skip\\(|todo\\(' -- sdk web-ui
```

CI：

```powershell
git ls-files ".github/workflows/*"
git grep -n -E 'continue-on-error|if:|pytest|cargo test|npm test||| true' -- .github
```

以上只是定位命令，不代表所有测试已经被识别。还要检查动态加载、脚本间接调用和构建生成的测试入口。

## 3. 建立真正有用的测试追踪表

文件：

`docs/audit/test-to-production-map.csv`

每条测试必须填写：

| 字段 | 内容 |
|---|---|
| test_id | 测试唯一编号 |
| test_file | 测试文件 |
| production_file | 实际生产源码 |
| production_symbol | 实际被测函数 |
| entry_path | 公开调用入口 |
| test_level | Unit / Integration / E2E |
| oracle | 预期结果来源 |
| fixture_sha256 | 测试文件指纹 |
| failure_assertion | 失败如何被检测 |
| side_effect_check | 是否检查真实副作用 |
| mutation_test | 能否捕获对应故障变异 |
| skip_status | 是否可能跳过 |

没有 `production_file` 和 `production_symbol` 的业务测试，不能计入生产代码覆盖证据。

## 4. 测试可信等级

统一使用以下等级：

- T0：源码静态检查。
- T1：局部单元测试。
- T2：真实模块集成测试。
- T3：真实外部依赖集成测试。
- T4：生产公开入口 E2E。
- T5：独立 Oracle 对照和故障变异验证。

例如：

```text
纯函数测试通过          = T1
Rust Service 实际调用   = T2
真实 Poppler 执行       = T3
MCP 导入到 DOCX 导出    = T4
真实文件对照 + Mutation = T5
```

不能把五种等级折算成一个测试通过数量。

## 5. 缺陷编号和证据要求

统一使用：

```text
AUDIT-<MODULE>-<NNN>
```

例如：

```text
AUDIT-PARSE-001
AUDIT-DOMAIN-001
AUDIT-EVIDENCE-001
AUDIT-REVIEW-001
AUDIT-EXPORT-001
AUDIT-MCP-001
AUDIT-UI-001
AUDIT-CI-001
```

每个缺陷必须提供：

1. 实际源码位置。
2. 具体触发条件。
3. 独立复现输入。
4. 实际运行命令。
5. 原始失败日志。
6. 影响的业务结果。
7. 最小修复建议。
8. 修复后的回归测试。

不能只写「可能存在」「建议加强」。

## 6. 停止条件

第一阶段完成前：

- 不修改生产代码。
- 不调整测试阈值。
- 不删减失败测试。
- 不增加全局 SKIP。
- 不更改业务状态机。
- 不升级依赖来绕开环境问题。

第一阶段交付：

```text
docs/audit/
  tracked-files.txt
  cargo-metadata.json
  full-source-inventory.csv
  test-to-production-map.csv
  test-quality-audit.md
  false-green-register.md
  initial-risk-register.md
```

主 Agent 必须核对全部报告后，才允许进入第二阶段。

## 7. 审计完成条件

必须回答六个问题：

1. 仓库到底有哪些生产代码？
2. 每个业务入口实际调用哪些代码？
3. 现有测试究竟测试了哪些生产函数？
4. 哪些测试只是 Mock、仿真或固定输入输出？
5. 哪些故障虽然存在，却不会导致测试失败？
6. 哪些关键业务能力实际上还没有被真实验证？

不能回答这六个问题，就不能宣布全仓审计完成。