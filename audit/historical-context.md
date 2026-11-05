# 历史资料消化报告（audit/historical-context.md）

> 全仓审计 V2 · Task 2-a（历史资料消化代理）产出。
> 冻结基线：main@`6105beb652e88760bdf38f08080af1ed7b1f048a`（= PR #242 合并点）；当前检出分支 `audit/v2-full-coverage`（工作树 HEAD `6ae52f8` = 基线 + R0 审计文档，生产代码与基线一致）。
> 本文用途：为后续 R1~R6 审计轮提供历史上下文基线——**历史结论一律不得照抄，均须按 §2 的"重核清单"在当前 HEAD 重新验证**（历史资料描述的代码基线最早至 2026-10-25 的 `a485a7b`，距今已历经 M1.5/M1.6/M1.7、#200 PDF 多引擎重构（B4）、#219/#227/#237 全块覆盖、PR #236/#241/#242 等多轮合并）。

---

## 1. 历史资料索引表

| # | 路径 | 篇幅 | 性质 | 核心主张 |
|---|---|---|---|---|
| 1 | docs/audit/PLAN.md | 923 行 | 全仓审计执行计划（issue #203 依据原文） | R0~R7 七阶段审计：源码盘点→逐模块审查→测试质量（杀假绿）→八套业务场景→20 项安全专项→Mutation→CI 重建；"测试必须证明代码完成业务行为"；五状态（PASS/FAIL/SKIP/IGNORED/NOT_EXECUTED）不得混同；12 条最终验收门槛 |
| 2 | research/audit-r3.md | 196 行 | R3 独立安全审计报告（2026-10-25，对象 a485a7b/6a392d5） | S1×1（MAX_OCR_PAGES=200 死门——数学上不可触发，250 页纯空白 PDF 实证绕过）+ S2×4（run_limited join 无界晚归 Success；TmpGuard 非唯一目录并发污染；分类召回表 7/18 错类未披露；runner 禁引纪律为死代码）+ S3×5 + M×2 + P2×6 |
| 3 | research/audit-r4.md | 82 行 | R4 主代理自查轮（独立性弱化声明） | R3 剩余项修复属实；zip64 守恒门"7 类攻击变体无绕过"；S3 登记三项（A6/A7/_anchor_hit 拼接窗口）；诚实降级声明：子代理三次启动失败，本轮为自查 |
| 4 | research/audit-r5.md | 247 行 | R5 独立子代理复审（2026-10-26，对象 0e50961） | **推翻 R4**"无绕过"结论：V6 尾部垃圾 ≥65,536B 出窗 / V7 影子 EOCD / V8 zip32 本盘-总盘字段分歧三类实测绕过；34 项负向测试静态审查无一假绿；M1.6 处置闭环（EOCD 镜像重写、乘法判据、格轴负向、setsid 注记）全部落地 |
| 5 | research/verification-plan.md | 277 行 | 验证阶段执行计划（V0~V3、M1.7、S1~S3 状态回填） | 评测夹具三件套 + 三条关键链路（V1 解析→DOCX 导出 / V2 补遗影响面 / V3 文档内核三问）双实现交叉全绿；SIMULATED/IMPLEMENTED/INTEGRATION_TESTED 三级分记；cuc-028 政采链 + txzbqy 通信链真实数据闭环 |
| 6 | research/risk-register.md | 90 行 | 风险登记册 v2（R-01~R-17 + 三层保证模型） | R-01/02/03/04/09/11 六项 BLOCKER；三层模型（L1 输入完整性/L2 规则正确性/L3 语义正确性）——"确定性校验 ≠ 确定性正确"，"100% 拦截"仅对 L2 成立；全量追溯矩阵为跨文档一致性单一真源 |
| 7 | research/test-plan.md | 107 行 | 合成夹具测试计划（TC-01~TC-16） | 25 埋点主夹具 + 六维泛化面（D1~D6 难点层/组件矩阵/支持率人工协议/重跑稳定性/Office 复验）；"目标 ≠ 已证"边界声明；发布门禁用例清单 |
| 8 | docs/threat-model.md | 86 行 | 安全威胁模型 v1.0（T-01~T-15 四组+供应链） | Agent/Skill/模型为不可信执行层，一切防线后端强制；T-03/T-04 建模发现的 2 个 GAP 已实装（guard.rs）；5 动作人工审批双层门禁矩阵；T-07 TOCTOU 等 6 项残余登记 |
| 9 | docs/bid-mcp-contract.md | 757 行 | MCP 工具契约（§0~§19 + 附录 A/SV，工具面 24 个） | 薄壳原则（校验+状态机+转发）；错误码/幂等/大结果分页 fail-closed 游标；四门（matrix/outline/pre_export/addendum_waiver）；#219 二步抽取合并语义；真实语料红线（L1 召回 ≥0.9717、否决精确率 ≥0.9636） |
| 10 | docs/bid-domain-model.md | 100 行 | 领域数据模型（8 实体 + 状态机 + ER） | ID 三命名空间隔离（REQ/EV/SEC vs DOC-*）；溯源锚不可变；诚实状态显式枚举；ProjectStatus 11 态线性 + DIRTY 重跑不整体回退；E_STATE 越级保护 |
| 11 | docs/architecture-decision.md | 120 行 | 架构决策 AD-01~AD-08 | 不用 Dify 编排（状态自持）；路径 A（Skill+MCP+状态机）；KBAdapter 自建默认；AD-05/07 对齐 mdcore 既有契约面（差距 G1~G3 走引擎需求清单）；AD-08 核心全 Rust、serde 契约类型化 |
| 12 | docs/doc-engine-audit.md | 191 行 | mdcore + cleanroom 只读审计（基线 6daeace→c33bcab） | 引擎成熟度高于假设（块锚点/证据三态/审校门禁/原子发布已产品化）；差距矩阵 G1~G11（G1 目录域/G2 文档级样式/G3 暗标元数据清零 = P0）；§5 映射契约（指纹判失效、版本做审计、human: 前缀审批下沉引擎） |
| 13 | artifacts/bid-health-check/REPORT.md | 64 行 | E 组竞品体检报告（15 份真实水利招标文件，基线 main@7fff169） | **否决项召回逐文件 0.226~0.483、去重聚合 0.386——远未达 100% 硬门（头号问题）**；引用支持 317/317 全过；崩溃率 0；Top10 问题见 §5 |
| 14a | audit/evidence/github/issue-203.json + comments | 正文 3.9K 字 + 1 评论 | 全仓审计旧计划 issue（已关闭） | 六条修订（见 §3）；启动前置 8 项 issue；代理 C 预清材料：17 条发现（P1×5——SDK-001~003/CI-001~002） |
| 14b | issue-224.json + comments + pr-236.json | 正文 + 4 评论 + PR（已合并） | P0 ScoringMapping（开放中） | #186 被 PR #209"诚实标记"提前关闭，7 项核心未做；PR #236 已交付第 1 步（ScoringMapping 实体+多对多+UNMAPPED）；第 2/3/4 步遗留 |
| 14c | issue-241.json + comments | 正文 + 3 评论（已关闭=已合并） | Agent 必选标记接线阻断门禁 | agent_mandatory 进 blocking 派生 + 起草门 UncoveredMandatoryItem + 导出门；三决策点已裁定（正文承诺 ≠ 已响应） |
| 14d | issue-242.json + comments | 正文 + 3 评论（已关闭=已合并） | 1.0 写作验收门禁 | E_CHARS_BELOW_MIN BLOCKER + 导出门 1c 独立复算（审核直接批准也禁短稿导出）；真实 LLM 验收未完成（无 API key，如实声明） |
| 14e | issue-243.json | 正文（开放，0 评论） | P0 校验副作用与状态恢复 | 见 §4 精读 |
| 14f | issue-244.json | 正文（开放，0 评论） | P1 全块覆盖闭环 | 见 §4 精读 |
| 14g | issue-245.json | 正文（开放，0 评论） | P0 占位资质 | 见 §4 精读 |
| 14h | open-issues.json | 5 条 | 当前全部开放 issue 清单 | #224（P0 评分映射）、#229（任务面板）、#243、#244、#245 |

> 注：R3 报告提及的 R1（夹具规格审计）/R2（代码审计）未见独立报告文件（research/ 下仅 r3~r5；audit-notes/ 与 raw/ 为 18 个外部竞品仓的审计笔记，非本仓代码审计）。另 research/ 下还有 s2-scenario-plan.md、real-scenario-library.md、repository-matrix.md、workflow-audit.md、cpa-issues-scenario-absorption.md 五份被上述资料引用的文档，本轮未指定精读，需要时由后续轮补读。

---

## 2. 历史已发现问题清单（含当前 HEAD 重核清单）

> 严重度沿用当时口径。**"重核清单"= 在 6105beb 上必须重新验证的事项**；✅=本轮消化时已做只读抽查并得出初步结论（仍需正式审计轮确认），⚠️=尚未核实。

### 2.1 R3 审计（基线 a485a7b/6a392d5，2026-10-25）

| 编号 | 问题主张（当时证据位置） | 严重度 | 当前 HEAD 重核清单（初步抽查结论） |
|---|---|---|---|
| R3-S1-1 | MAX_OCR_PAGES=200 门在唯一适用分支数学上不可触发（死门）：`pdf.rs:23-31` pages 口径=非空分片数，OCR 分支内恒 ≤16<200；250 页纯空白 PDF 实证绕过，pdftoppm 开始批量渲染 | S1 严重 | **已修复，需复核修复完备性**。`pdf.rs` 已不存在（#200 重构为 `src/engines/`）；`engines/tesseract.rs:86-92` 现为 `probe_pdf_pages(path)` 取真实页数过门 + 渲染 `-l real_pages` 双保险。重核：①probe_pdf_pages 实现与失败回退（unwrap_or(1)——探测失败时门形同虚设？）；②">200 页纯扫描件整体拒绝"负向测试是否在位（tesseract.rs 内联测试仅 2 个 #[test]，未见该负向） |
| R3-S2-1 | run_limited 直接子进程退出后 join 无界等待，1s 超时实际挂 60s 且晚归 Success（`guard.rs:449-473`） | S2 应修 | **已修复，需复核**。`guard.rs:214-234` 现为"结果经 channel 发送、主线程限时接收、限时收不到补杀进程组"。重核：宽限时长、Timeout/Success 分类语义、孙进程持管道回归测试在位性 |
| R3-S2-2 | TmpGuard 目录名仅 tag+pid 非调用级唯一：并发交叉污染 + 崩溃残留复用（`guard.rs:493-505`） | S2 应修 | **待重核**。guard.rs:318-333 TmpGuard 仍在；需确认目录名是否已加唯一化成分（R5 报告未见该项修复记录——R3 处置建议序列里 S2-2 排在 S2-1 同批，但 R5 复审清单未提及） |
| R3-S2-3 | real_eval_score.py 分类召回表不做 REQ 类别校验：7/18 命中错类未披露，GT-V03 高风险否决条款落"其他"（`scripts/real_eval_score.py:63-110`） | S2 应修 | **待重核**。R5 复审时"类别命中 12（50.0%）"已作为诚实维度呈现（50% 数字本身即错类披露）；但"分类错类→下游合规矩阵不设防"的业务风险仍然成立。重核：当前 extract/veto_rule 分类对后置否定从句（"否则该业绩无效"）是否仍落"其他"（与 §5 健康检查问题 2 同根） |
| R3-S2-4 | runner.rs "解析失败禁引"为死代码：ParseStatus::Failed 恒假、parse_file `?` 整批中止，一批中一个坏件阻塞全部导入（`bid-tools/src/runner.rs:55-67`） | S2 应修 | **已修复，需复核语义**。`runner.rs:62-64` 现有 `(Some(fid), ParseStatus::Failed, _) => E_UNPROCESSABLE 禁引` 分支（per-file 处理）。重核：import 循环是否 per-file catch + continue（而非整批 `?` 上抛）；`ParseStatus::Failed` 是否真实可达（谁能置 Failed）；E_PARTIAL 语义与契约 §2 一致性 |
| R3-S3-1 | zip64 归档绕过 EOCD 条目守恒门（declared==0xFFFF 跳过守恒比对） | S3 建议 | **代码已整体迁移/移除——历史结论失效**。guard.rs（现 529 行）中 zip64/EOCD/prescan/压缩比/条目名/XML 深度门**全部不存在**（rg 零命中）；R5 时代 guard.rs 有 zip 面 12 测试，现仅 11 个内联测试（子进程面 9 + 输入门 1 + sha256 1）。docs/capability-map.md S9 行显示 docx/xlsx 解析已"随 B4 收敛到引擎侧（office-io LIMIT_INPUT_BYTES 家族）"。重核：①这些防线是否真实活在 mdcore office-io（跨仓核对）；②capability-map S9 记录的"数值不同（2M vs 4M）取严者统一"是否已统一；③threat-model T-03"已实施"描述与代码的错位需修正 |
| R3-S3-2 | PDF 族无输入字节门（docx 52MB/xlsx 302MB 有门，PDF 任意大） | S3 建议 | **已修复**。`engines/mod.rs:67` `check_input_size(path, LIMIT_INPUT_BYTES_PDF)`，guard.rs:35 `LIMIT_INPUT_BYTES_PDF = 500MB`。重核：取值与契约/威胁模型一致性（R3 建议 512MB，实现 500MB） |
| R3-S3-3 | xlsx "声明 0 字节"绕过 prescan 比例/体量门 → calamine 黑盒解压 | S3 建议 | **历史结论失效（同 S3-1）**：bid-parse 已无 xlsx.rs；双轴门（20 万行/200 万格）及负向测试（xlsx_row_flood/xlsx_cell_flood）在 bid-parse 零命中，仅存于文档。重核：引擎侧 office-io 是否承接等价防线（LIMIT_SHEET_COUNT=1000/LIMIT_TOTAL_CELLS=4M——capability-map S9） |
| R3-S3-4 | "引用支持率"判据弱于其名：任意页子串 + page_delta 只算不用 | S3 建议 | R5 修复轮已改严格判据（锚点页 ±1 窗口）+ 页偏差分布 {0:17,-1:1}。重核：当前 HEAD 的 real_eval_score.py 判据是否维持严格口径 |
| R3-S3-5 | extract_main_pdf 编号条款通道不做否决词判定：GT-V03 编号否决条款落"其他"（`extract.rs:112-126`） | S3 建议 | **待重核**。bid-parse 现有 veto_rule.rs / veto_lexicon.rs / extract_docx_veto.rs 测试——编号通道否决词优先判定疑似已实现。重核：extract.rs 现行分类优先级 + 该负向回归测试 |
| R3-M/P2 各项 | 零悬空断言恒真、setter 回环、计数失真、无扩展名误拒等 8 项 | M/P2 | R5 复审确认 M-1/M-2 已修复（four_contracts 断言真化/refresh 真路径）；P2-4 stderr drain 已修（guard.rs:238-256 持续 drain+头 64KB）；P2-3 无扩展名误拒、P2-5 内存上界注记、P2-6 非unix降级——随 guard.rs 缩减需重新逐项定位（部分可能已随迁移消失） |

### 2.2 R4/R5 审计（基线 0e50961/e5a86fd，2026-10-26）

| 编号 | 问题主张 | 严重度 | 当前 HEAD 重核清单 |
|---|---|---|---|
| R5-S3 新增 | EOCD 守恒门三类绕过（V6 尾部垃圾出窗/V7 影子 EOCD/V8 字段分歧）——R4"7 类变体无绕过"被证伪 | S3 | **同 2.1-S3-1：该门整体已不在 bid-parse**。M1.6 的"EOCD 镜像重写"（EocdCountFieldsDiverge/EocdUnverifiable/三档差分负向）在本仓 rg 零命中——需确认是随 B4 迁入引擎还是被删除（若删除，threat-model T-03 的"已封"叙事失效，需在引擎侧或本仓重新核验等效防线） |
| R5-P1 | xlsx 单元格轴负向测试缺口 | P | 已随 M1.6 入仓（R5 §9 记录）——但当前 bid-parse 已无该测试文件（xlsx.rs 移除）。重核同上：防线归属 |
| R5-P2 | 压缩比门整数除法截断（实效拒点=商≥101） | P | 同上：guard.rs 已无压缩比门；乘法判据去哪了需追 |
| R5-P3 | run_limited 杀组假设边界：setsid 脱组孙进程逃过 kill(-pgid) | P | guard.rs 仍在用 process_group(0)。重核：threat-model §4.2 setsid 注记是否仍在 + 当前三个外部命令（pdftotext/pdftoppm/tesseract）派生面 |
| R5-环境契约 | e2e_m1 缺 `~/.tessdata/chi_sim.traineddata` 时响亮红（设计如此） | — | m1/tessdata/chi_sim.traineddata 已入仓；重核：当前 bid-parse OCR 测试的环境依赖声明（env_preflight.rs 存在——预检机制疑似新增，需读） |
| R4-A6/A7/_anchor_hit | 三项 S3 登记（畸形归档接受/前置垃圾/相邻块拼接窗口） | S3 | _anchor_hit 在 run_tests.py T12——重核该脚本现状；A6/A7 同 S3-1 归属问题 |

### 2.3 Issue #203 代理 C 预清发现（2026-11-03，基线约 e6f0192）

| 编号 | 问题主张 | 严重度 | 当前 HEAD 重核清单（初步抽查结论） |
|---|---|---|---|
| AUDIT-SDK-001 | 人工门参数文档滞后于 #182 receipt 验签（SKILL.md/p1-import.md 教已废除的 human: 前缀） | P1 | 评论称 PR #235 已修。重核：sdk/skill/bid-response-pipeline/references/p1-import.md 等是否已改为 receipt 口径 |
| AUDIT-SDK-002 | minimal_flow.py 示例断裂：四门必死 + 乐观并发静默失效 | P1 | **疑似已修复**：minimal_flow.py 已重写为 external 起草链路（头注"四门（matrix/outline/pre_export + addendum_waiver）→ external 起草首叶"）。重核：全链可运行性 |
| AUDIT-SDK-003 | checklist.py P6 判据与实现信封漂移（契约 §0 五键 vs 实现 3 键） | P1 | **疑似已修复**：checklist.py:14 现按 `{ok, tool, data, request_id, project_version?}` 校验；tools.rs:298/318 确有顶层 request_id 注入。重核：request_id/project_version 顶层键在全部写工具响应中的覆盖面 |
| AUDIT-CI-001 | CI 覆盖缺口：单 workflow/单平台（Windows cargo check），SDK pytest/run_tests/Web-UI/Mutation 零接线 | P1 | **待重核**（.github/workflows 现状）；web-ui/package.json 仍只有 build/serve/typecheck（无 playwright/vitest）——web-ui E2E 前置工程（修订 2）仍未启动 |
| AUDIT-CI-002 | SKIP 计为 passed 的假绿设计（run_e2e.sh exit 0 + mcp_sdk_e2e 默认跳过） | P1 | 待重核（scripts/run_e2e.sh、sdk/tests/test_mcp_e2e.py 的跳过条件） |

### 2.4 已合并 PR 相关（#236/#241/#242——历史结论必须重核，不能照抄）

| PR/Issue | 宣称交付 | 当前 HEAD 重核清单（初步抽查结论） |
|---|---|---|
| PR #236（#224 第 1 步） | ScoringMapping 实体（SM-xxxx）+ 多对多 weight_share 摊分 + UNMAPPED 结构化 + 映射报告 + 快照加性迁移；bid-domain 230 测试 | **符号已在位**（scoring.rs 4 处 + tests/scoring_mapping.rs + lib.rs）。重核：①摊分语义正确性（跨 2 章 0.6/0.4 → 12/8）；②单映射 share=1.0 与历史逐位一致；③UNMAPPED 告警链路；④**第 2 步未做**——scoring.rs:141 注释明言"重复计分检测见 validate_sums 扩展——第 2 步"（share 之和 >1.0 检测缺口仍在）；⑤第 3/4 步（score_rules 消费/评分变更→审核失效）未做 |
| issue #241（Agent 必选阻断） | agent_mandatory 单一真源进 blocking 派生 + 起草门 UncoveredMandatoryItem + 导出门 + 警告键改名 AGENT_MANDATORY_OFFCATEGORY + corpus-A 全链门禁测试 | **符号已在位**（matrix.rs 7 处/merge.rs 6 处/readiness.rs 9 处/tests/agent_mandatory_gate.rs）。重核：①blocking 派生 `(is_mandatory ∪ agent_mandatory) && NOT_RESPONDED`；②"正文承诺 ≠ 已响应"裁定落测（readiness_check.rs 三面）；③三门（矩阵/起草/导出）拦截-放行矩阵 |
| issue #242（1.0 写作验收） | E_CHARS_BELOW_MIN BLOCKER + E_SECTION_EMPTY + 导出门三道（1a 矩阵/1b 审核/1c 写作验收独立复算）+ MCP 全流程验收 + 短稿闭环 | **符号已在位**（validate.rs 6 处/tests/writing_acceptance_e2e.rs 13 处）。重核：①1c 门不消费记审面（直接 Approved 也拒短稿）；②分治语义（下限阻断/上限 MINOR 提示）；③**真实 LLM 验收仍未完成**（环境无 BID_LLM_API_KEY——issue 明示"不算完成"）；④artifacts/e2e/latest 基线漂移问题（#241 评论：干净 HEAD 重跑否决项 52→50，疑似 #238/#233 合并后产物未刷新——**基线新鲜度重核**） |

### 2.5 威胁模型/风险册登记的遗留缺口（历史已识别未闭环）

| 项 | 历史主张 | 重核清单 |
|---|---|---|
| T-06 跨项目越权 | M2 权限用例 DESIGN_ASSUMPTION | 当前是否有 ID 命名空间隔离测试 |
| T-07 TOCTOU | fs_fence check-then-use v1 残余（#183 实装时登记） | fs_fence.rs 现状与 openat 方案排期 |
| T-13 日志脱敏 | 全链路脱敏清单 M5 未做 | trace/draft_note 中的敏感面 |
| T-15 供应链闭包 | NOTICES 式闭包审查 M0 待执行 | 是否仍无 |
| R-16 ccgp WAF | 文件级招标本体获取受阻转人工（GT-M17-24） | real-corpus/water 是否已有文件级补验（water 目录已有 A-01/A-04/A-05 等真实 PDF——**历史"文件级未验"结论已过时**，water 语料后来已入库并被 real_corpus_e2e 使用） |
| R-17 GT 冻结 | 24 条政采 + 18 条通信 GT 草案 draft_pending_human | 冻结状态是否变化 |
| G1/G2/G3 引擎缺口 | doc-engine-audit P0（目录域/文档级样式/暗标元数据）——引擎侧待排期 | mdcore 已前进（#211 已合入、mdcore 2d014445 被健康检查用作生产口径）；G1~G3 三锚点是否仍有效需跨仓复核 |

---

## 3. Issue #203 六条修订摘要（逐条原文要点）

1. **修订 1（增量修复与审计的关系）**：审计基线 = 启动时刻的 main（含所有已合并修复）；initial-risk-register.md 登记"审计前已修"项（附 issue/PR 号）；**已修项仍要审计——验证修复是否真的解决问题（有没有被测试绕过）**。
2. **修订 2（web-ui E2E 是前置工程，不是审计任务）**：实测 web-ui/package.json 无任何测试框架（无 playwright/puppeteer/vitest/jest）；搭建 E2E 基础设施拆为独立前置任务（预计 2-3 天）；本批次只覆盖 typecheck + 手工验证清单 + API 契约对账；浏览器 E2E 单独一批。
3. **修订 3（Mutation Testing 明确范围和触发方式）**：工具 cargo-mutants；范围只覆盖 P0/P1 业务不变量所在函数（白名单，非全仓）；触发 nightly 定时 + 手动，不进 PR CI；判据 = 高危变异（删状态转换检查/删权限校验/删审核版本绑定）必须被捕获；不追求行覆盖率。
4. **修订 4（Oracle 来源必须分级）**：PDF/DOCX/XLSX 文本提取=双路交叉验证（自动）；条款抽取正确性=人工标注+冻结基线；评分映射正确性=业务规则文档+人工抽样；状态机转换=领域规则直接编码；导出保真度=回读对账；安全边界=攻击输入+断言拒绝。test-to-production-map.csv 的 oracle 字段必须标来源类型，"人工标注"须登记标注人/日期/覆盖范围。
5. **修订 5（工作量估算与分工）**：R0~R7 合计 46-69 人天；四代理分工——A=bid-domain+bid-tools+bid-mcp（核心业务链）、B=bid-parse+bid-engine+跨仓集成、C=sdk+scripts+CI、Lead=R0 汇总+报告整合+最终复审；web-ui E2E 单独排期。
6. **修订 6（先试点再全仓铺开）**：第 1 步只审 bid-parse 一个模块（R0+R1+R2）——产出该模块 inventory+test-quality-audit，验收"能否真的发现之前没发现的问题"；试点通过后其余模块并行；不通过先改方法论。

另：issue #203 正文"启动前置条件"列 8 项 issue（#182/#183/#189/#199/#200/#185/#186/#188）须全部关闭后开工——**当前 #186 被 #224 判定"过早关闭"，前置条件的满足度本身需重核**。#203 五条"特别认同"的坑位（自证测试/双实现同错假设/SKIP 遮蔽/失败转 SKIP/降阈值造绿）应作为本轮假绿猎杀的检查基准。

---

## 4. #243/#244/#245 开放问题定义精读（本轮审计的调查目标定义）

### 4.1 issue #243（P0：起草中校验无副作用并支持状态恢复）

- **复现场景（历史日志，需在当前 main 重现）**：全稿尚未完成时调用全局 `validate_bid_document` → 项目进入 `VALIDATING` → 后续章节起草/修订无法继续；恢复方式是杀 MCP 子进程 + 手改持久化 `db.json` 状态字段（绕过域层状态机）。issue 明示：**不要假定历史日志中的状态行为仍与当前 main 一致**——先判断问题归属（工具/API 状态迁移缺保护 / Agent 编排调用顺序错误 / 两者兼有）。
- **目标**：起草期间误触发校验无副作用；最终校验失败后可修订重试；提供受支持恢复路径；**禁止要求用户手工编辑 db.json**。
- **范围**：①复现定位 validate 在章节未完成时的行为与状态迁移；②区分"起草中/章节级检查"与"全稿最终校验"——起草未完成时全局校验应返回明确缺章信息且**不得改变项目状态或版本**；③最终校验失败后项目必须保持可修订可重试（明确状态迁移与并发/重复调用语义）；④对已处于不完整 VALIDATING 的项目提供受支持且有审计记录的恢复办法（不得无约束任意状态写入）；⑤更新 MCP 契约与 Skill/流程说明；⑥**保持 PR #242 的篇幅验收与导出门禁语义，不得通过放宽质量门解决**。
- **验收标准（原样提取）**：部分章节状态下调用全局校验→返回可操作的 `DRAFT_INCOMPLETE`（或等价错误码）+ 缺失章节清单，且项目状态/版本/审核记录均不变；全稿校验失败→仍可修订再次校验，修复后可正常进入导出；三处（服务层/MCP 工具层/持久化重启后）均有回归测试；重复请求、超时重试及并发调用不会把项目留在无法恢复的状态；历史异常 VALIDATING 状态恢复方案带状态不变量与审计记录测试；端到端测试覆盖"未完成校验被拒→继续起草→完成全稿→校验→导出"完整顺序；最新 Windows CI 全绿；独立分支 PR、代理不得自行合并。
- **不在范围**：Agent 标注合并/证据来源可信度/Agent 必选阻断/最低篇幅验收（近期 PR 已覆盖，勿重复实现）；MCP 客户端自动重连（客户端仓库职责）。
- **本轮初步核对**：`DRAFT_INCOMPLETE` 在 bidcore 全部 Rust 源 0 命中——错误码尚未存在，待实现面从零开始。

### 4.2 issue #244（P1：建纲前闭环处置全块覆盖盲区）

- **复现场景**：#219/#227/#237 已建立全块覆盖报告、Agent 标注优先与正则兜底，但**历史真实语料运行仍报告有未覆盖块**；仅生成 coverage_report 或一条 warning 不足以证明所有可能含要求的内容已审阅；反过来也不能要求所有 PDF 块都变 REQ（目录/页眉/噪声应能被明确排除并留依据）。
- **目标**：构建正式大纲前，对所有输入块形成可审计的覆盖处置闭环——**每块要么有最终条款，要么被明确归类为非要求/噪声/不适用；仍可能含要求但未覆盖的块必须触发补标/复查，不得静默进入起草**。
- **范围**：①在当前 main 重跑现有真实语料记录最新 coverage 基线（**不沿用过期数字作为当前事实**）；②未覆盖项三分（尚未判定/已判定为要求生成 REQ/已判定非要求含原因），记录块 ID、文件/页/区间和判定来源；③可重复补漏流程（Agent 补标+重跑合并，幂等、留审计、不制造重复/幽灵 REQ）；④`build_outline` 前接入覆盖处置检查（尚有未判定或高风险候选→可操作阻断+清单；已明确排除的块放行且报告可查）；⑤更新 Skill 编排（覆盖未闭环时先补漏/分类而非继续建纲起草）。
- **验收标准（原样提取）**：真实语料生成覆盖处置报告，所有未覆盖块有稳定 ID 与处置状态，明确区分"已排除"与"遗漏待审"；人为保留一个疑似要求但未判定块→建纲被阻止并指明该块，补标后重跑 REQ 入最终集合、建纲继续；人为保留明确页眉/目录噪声排除项→不误阻断且排除理由可追溯；重启后处置状态持久化、重复补标/重跑幂等；Agent 标记优先与正则兜底既有语义、GT 红线不回归；产物只提交摘要/报告不提交大体积中间文件；最新 Windows CI 全绿、真实语料验证如实报告运行环境与 SKIP 项；独立分支逐 commit push、不自行合并。
- **边界**：不重做 #219/#227/#237 标注合并架构、不重做 #216 输入完整性门、不改 #241 必选阻断策略——**本 issue 补的是覆盖报告到建纲之间的审阅/补漏闭环**。
- **本轮初步核对**：`bid-mcp/src/tools.rs:837-840` 的 UNCOVERED_BLOCKS 目前只是 warning（"正则已兜底"），`build_outline`（tools.rs:3609+）尚无覆盖处置阻断——与 issue 判断一致，待做面真实存在。coverage.rs（BlockVerdict 模型）与 coverage_tools.rs（17/18/19 三工具）是现有基础。

### 4.3 issue #245（P0：占位资质不得冒充已核验的合规证据）

- **复现场景（2026-11-05 真实客户端运行）**：Agent 注册了**占位资质**并绑定到大量矩阵行→阻塞数因此归零→被误读为真实资质已满足。根因：#212 已验证证据来源指向已导入文件且 SHA/指纹服务端生成，但**来源真实 ≠ 证据内容已核实**；当前 `EvidenceStatus` 只有 ACTIVE/EXPIRING/EXPIRED/SUPERSEDED（有效期/换版语义），没有"占位/未验真"语义。
- **目标**：允许草稿使用明确占位文案，但**占位/未验真的资质不得被当作已验证证据、不得清除合规阻塞、不得让输出宣称资质已具备**。
- **范围**：①先在当前 main 检查证据注册/绑定/矩阵派生/起草/导出路径，复现占位资质能否被计为有效证据（区分"来源存在"与"内容/资格已核实"）；②为占位/待核验材料建立明确可持久化的状态或类型（**不得由 MCP 调用方自报为已验证**）；③矩阵和导出门必须三分：真实有效证据/占位待核验/缺失证据——占位项不得让资格或否决阻塞变为已满足；业务允许继续写作时呈现待补状态并保留阻塞/受限标记；④起草可按用户授权输出 `【待补：...】` 占位，但该占位不能成为证据引用、不能生成"已具备/已满足"结论；⑤验证状态改变必须走受控可审计的服务路径（保留 #182 人工 receipt 边界与 #212 来源校验，不引入 Agent 自证）；⑥更新 MCP 契约、Skill 与验收报告。
- **验收标准（原样提取）**：导入来源真实但内容标记占位/待核验的资质→绑定后矩阵仍显示未满足/待补，导出门仍按既定规则拒绝或标记受限，**绝不静默清零阻塞**；经合法验证的真实证据→正常通过既有证据链不回归；占位可进草稿但在正文/验证/导出报告中可追踪显式标记、不能伪装成资质证明；MCP 层不能通过参数把占位状态直接改成 verified（非法状态迁移拒绝且无副作用）；服务层/矩阵/MCP 工具/端到端导出均有正负测试；旧数据升级兼容策略有测试；最新 Windows CI 全绿；独立分支逐 commit push、不自行合并。
- **边界**：不重复实现 #212 文件来源/指纹验证、不改 #241 Agent 必选阻断语义——**关注证据内容的核验状态与占位材料不能冒充真实资质**。
- **本轮初步核对**：`bid-domain/src/evidence.rs:67-80` EvidenceStatus 确为四态（无占位态，迁移单向 Active→Expiring→Expired、任何→Superseded、任何→Active 拒）；`bid-tools/src/placeholder.rs` 是正文占位语法（【待补充：…】/【待确认：…】，issue #103 面，"不能留到导出"）——正文占位面已存在但**证据状态面无占位语义**，与 issue 判断一致。

---

## 5. 历史健康检查（bid-health-check REPORT.md）关键数据

> **口径警示：历史数据（基线 main@7fff169、mdcore 2d014445），需在冻结版本 6105beb 重跑后才能作为当前结论。** 语料：竞品仓 power_grid（MIT）真实招标文件 15 份去重（9 PDF + 7 官方示范文本 .doc→docx），全部为**水利行业**（行业单一性如实施报）；引擎路径生产口径 engine_docx 7 / engine_pdf 8 / engine_pdf_err→legacy 1。

| 维度 | 历史数字 | 判定 |
|---|---|---|
| **否决项召回（硬门 100%）** | 逐文件 0.226~0.483；去重聚合 **0.386**（unique_face 真值块 1006、覆盖 388） | ❌ 远未达硬门——**头号问题** |
| 最差五词召回 | 违约金 0.178 / 失效 0.200 / 列入 0.288 / 取消 0.340 / 否决 0.342；最好：撤回 0.625 | — |
| 条款抽取面 | 8,688 REQ / 122.9 万字 / 7.07 条每千字 | ✅ 量级正常 |
| 类别命中 | A-01 词表命中 26 条抽检：否决 7/评分 5/资格 1/**其他 13**——命中否决词的条款 50% 落"其他" | ⚠️ |
| 引用支持（locator 回查） | **317/317 全过**（fail_no_block=0 / fail_text_mismatch=0） | ✅ 100% 可溯源 |
| 崩溃率 | panic 0 / error 0 / ok 16 | ✅ 零崩溃 |
| 吞吐 | docx 1.93ms/千字；pdf 1.87ms/页；OCR 件 2.3s/页 | ✅（OCR 慢已知） |

**Top10 问题（合成夹具测不出来的）**：① 否决语义在 PDF 行级碎片上系统性漏检（P0——pdfium/pdftotext 行切分，句子被切短行/行尾截断，最小复现 A-01 第10页「拒收。」独立成行未抽取；合成夹具 docx 段落级天然完整、此面零覆盖）；② 命中否决词的条款 50% 分类落"其他"（P0——后置否定从句不识别，「否则该业绩无效」→其他）；③ 「其他」类占比 82%（7140/8688，P1——召回分母被稀释）；④ PDF 面表格结构丢失（P1——engine_pdf 通道 tables=0，评分办法表格整面不可结构化，T-3 评分矩阵在 PDF 件上无数据源）；⑤ 扫描件 OCR 通道抽取面坍塌（P1——A-02 福建 8 页仅抽 1 条，OCR 行噪声使规则全线不命中）；⑥ 引擎与旧路块数差 +877（P2——pdfium 行切分更细 8984 vs 8107；veto_truth 两路同为 108，真值面未损）；⑦ 「违约金」召回 0.18 全场最低（P2——条款尾部/表格附注与主句分离）；⑧ 缺跨行拼接预处理（P2——块流无短行归并，PDF 件通用缺失）；⑨ A-02 路由 engine_pdf_err→legacy 回退正常但 OCR 18.6s/8 页待优化（P3）；⑩ 工具自身两发现（P3——pdfium 缺失静默降级 legacy 无响亮横幅，与 #9"登记不伪装"纪律冲突；去重聚合 covered 跨跑 2 块漂移 390→388 根因待查）。

**方法学局限（原文声明）**：否决"真值"是词表近似（LingYan 20 词反查表——命中块 ≠ 人工标注否决条款，召回为下界近似口径）；类别命中为单文件 26 条抽检；三跑验证块数/reqs/逐文件召回全同、仅聚合 covered 有 2 块漂移；语料全为水利行业，外推需谨慎。**建议登记（当时）**：否决召回双 P0 入 M2 抽取面任务池（块流短行归并预处理 + 否决词命中→分类强制覆核），需以该 corpus 固化为回归夹具（corpus/ 已 .gitignore、来源 provenance.json 可再取）。

**重核要点**：冻结版本重跑该体检（复跑命令在 REPORT 头部：`MDC_PDFIUM_LIB=... cargo run --manifest-path scripts/bid_health_check/Cargo.toml --release --bin bid-health-check`）；关注 ①②号 P0 是否已被后续（veto_rule.rs/veto_lexicon.rs 入 bid-parse、#226/#219 轮次）实质改善——**历史 0.386 数字本身不能引用为当前性能**；pdfium 静默降级横幅与 covered 漂移两个工具缺陷是否已修。

---

## 6. 历史资料与当前代码可能不一致的风险点（需 grep 核实清单）

> 消化过程中已做的只读抽查结论用 ✅（已确认不一致/已确认现状）标注；其余为**尚未核实的符号漂移嫌疑**，后续审计轮（尤其 R1 模块审查与 R2 测试质量轮）必须先对表再引用历史结论。

**A. 已确认的结构性重构（历史资料的文件/函数定位大面积失效）**

1. ✅ `bid-parse/src/pdf.rs` 不存在 → 重构为 `src/engines/{mod,poppler,tesseract,rust_fallback}.rs` + `pdf_text.rs` + `engine.rs`（#200 PDF 多引擎可插拔）。R3/R4/R5 所有引用 `pdf.rs:行号` 的结论定位失效。
2. ✅ `bid-parse/src/docx.rs`、`xlsx.rs` 不存在 → docx/xlsx 解析"随 B4 收敛到引擎侧（office-io）"（guard.rs:34 注释 + docs/capability-map.md S9 行）。由此：**guard.rs 从 R5 时代的 21+ 测试缩至 11 个内联测试（529 行），zip 全家门（prescan 三门/压缩比乘法判据/EOCD 守恒门/EocdCountFieldsDiverge/EocdUnverifiable/条目名五类拒绝/XML 深度门/xlsx 行/格双轴门及其全部负向测试）在本仓 bid-parse 已不存在**。需核实：这些防线是 (a) 迁入 mdcore office-io（跨仓核对 io.rs LIMIT_SHEET_COUNT/LIMIT_TOTAL_CELLS 等）、(b) 被引擎等价能力替代、还是 (c) 纯粹丢失——**这是本轮审计最高优先级的重核项**，直接影响 threat-model T-03"IMPLEMENTED"叙事的有效性。
3. ✅ `run_tests.py` / `grade.py` / `xref_check.py` / `real_eval_score.py` / `v2_impact.py` / `v3_integration.py` 等 scripts 的现状未逐个核实（verification-plan 大量引用 T7/T8/T9/T14/T16 断言计数 37→46→51→54）——需重核脚本与断言计数现状。
4. ✅ m1/ 并行线仍在仓（m1/src/*.rs）——PLAN.md 要求审计 m1 是否仍被调用/重复实现。

**B. 已确认的"已修复"历史问题（引用时必须改口径）**

5. ✅ S1-1 OCR 死门 → engines/tesseract.rs:86-99 已修（probe_pdf_pages 真实页数 + `-l` 双保险）；**但 ">200 页拒绝"的负向测试疑似缺失（tesseract.rs 仅 2 个 #[test]）——审计应作为发现登记**。
6. ✅ S2-1 join 无界 → guard.rs:214-234 已修（channel 限时接收 + 补杀）。
7. ✅ S2-4 runner 禁引死代码 → runner.rs:62-64 已有 Failed 分支（per-file 禁引）。
8. ✅ S3-2 PDF 字节门 → engines/mod.rs:67 已挂（500MB）。
9. ✅ P2-4 stderr drain → guard.rs:238-256 已修。
10. ✅ #224 第 1 步 ScoringMapping → scoring.rs/scoring_mapping.rs 在位；**第 2 步（share 和>1.0）明确未做（scoring.rs:141 注释）**。
11. ✅ #241 agent_mandatory 门禁三面、#242 写作验收门 1c 在位。
12. ✅ AUDIT-SDK-002/003 疑似已修（minimal_flow.py 重写为 external 链路；checklist.py 按 §0 五键信封校验、tools.rs 顶层注入 request_id）。

**C. 尚未核实的符号漂移嫌疑（需 grep/读码确认）**

13. ⚠️ TmpGuard 目录唯一化（R3-S2-2）是否修复——guard.rs:320 TmpGuard::new 的目录名构成需读。
14. ⚠️ extract.rs 编号通道否决词优先（R3-S3-5）——现 extract.rs + veto_rule.rs 的分类优先级。
15. ⚠️ `four_contracts.rs` / `corpus_probe.rs` / `e2e_m1` / `selftest_gate` / `kb_probe` 等 R5 时代 53 测试清单与当前测试清单的对应关系（cargo test --list 不可跑，静态数 tests/ 目录与 #[test]）。
16. ⚠️ health-check 双 P0（行级碎片/后置否定从句）在当前 extract/veto_rule 的改善程度。
17. ⚠️ `fs_fence.rs`（T-05/T-07）、`principal.rs`（#182 receipt）、`store_io.rs`、`assembly.rs`、`draft.rs` 等新模块为 issue #80/#100/#131/#180/#183/#184/#185/#188/#217 轮次新增——PLAN.md 的模块清单（A~H 八块）需按现状重画。
18. ⚠️ `docs/agent-integration.md`、`docs/subagent-llm-assessment.md`、`docs/capability-map.md`、`docs/engine-gap-requests-m0.5.md` 等新文档未在本轮指定清单内，但被契约/SDK 引用——R1 文档对账轮需纳入。
19. ⚠️ `real-corpus/water/`（health-check 语料 + real_corpus_e2e 红线语料）与 R-16"文件级待人工"结论的关系——water 语料已入库说明政采文件级缺口已被水利语料部分填补，历史"文件级未验"表述过时。
20. ⚠️ 契约宣称"工具面 24 个"（§0：21+3）与 tools.rs 当前 TOOL_NAMES 实际数——R1 工具面对账的基准数字需重数。
21. ⚠️ `artifacts/e2e/latest` 基线漂移（#241 评论：否决项 52→50 未刷新）——审计应先判定产物基线新鲜度再引用任何 artifacts 数字。
22. ⚠️ threat-model.md T-03 行（"guard.rs 18 测试 + xlsx 3 测试"）与 T-04 行（"9 测试"）的计数与现状不符（guard.rs 现 11 测试）——文档-代码一致性需回填修正（属审计发现，不得由本审计批直接修改）。

---

## 7. 威胁模型与 MCP 契约的关键承诺清单（R6 安全审计与 R1 工具面对账基准）

### 7.1 威胁模型关键承诺（docs/threat-model.md，R6 对账基准）

| # | 承诺 | 对账要点（当前 HEAD 验证什么） |
|---|---|---|
| W1 | 上传件=不可信输入，内容一律数据化永不执行；上传类型白名单仅 docx/xlsx/pdf（T-01/T-02） | import 面类型检查；宏载体拒收 |
| W2 | T-03 压缩包炸弹：输入字节门 + prescan 三门 + 压缩比乘法判据 + 实际读取累计计量 + EOCD 守恒门 + 条目名五类拒 + XML 深度门 + xlsx 双轴门（宣称 IMPLEMENTED+，负向 18+3 测试） | **高优先级重核**：§6-A2 已证实这些门不在 bid-parse——逐门追查引擎侧归属或登记丢失 |
| W3 | T-04 子进程治理：超时杀进程组/溢出即时杀/五分类永不误标成功/并发 4 席/TmpGuard 全路径回收/OCR 页数门 200/stderr drain（宣称 IMPLEMENTED，9 测试） | guard.rs 现存 11 测试中子进程面 9 个仍在；五分类 into_result 全 Err 断言在位 |
| W4 | T-05 超大文件 fs_fence：单件 500MB/单批 64 件/累计 2GB，超限 E_UNPROCESSABLE 逐件报因先于读取（#183） | fs_fence.rs 限额面 10 测试 |
| W5 | T-07 任意路径读写：upload_ref 文件围栏（受信根 BID_FS_ROOT∪BID_WS、组件级包含判定防 work vs work-evil 前缀陷阱、词法 `..` 预拒、逐段 symlink 检查）+ 导出侧 resolve_write 同款；TOCTOU v1 残余登记 | fs_fence + export 围栏；TOCTOU 注记仍在 |
| W6 | T-08 审批绕过双层门禁：引擎 ReviewPolicy `human:` 前缀（AI 结构性不可写审核态、失效不过审）+ MCP §8 precheck；#182 receipt 验签（HMAC-SHA256、24h 时效、scope 门级绑定、常量时间比对、旧 reviewer 参数显式拒） | principal.rs / record_review_gate / record_addendum_waiver 的 receipt 验签面 |
| W7 | T-10 注入：确定性解析层结构性免疫（无模型消费文档原文）；起草层 L3 残余靠人工审校门禁 | 解析→抽取链路确无模型；draft 面 LLM 消费的输入标记 |
| W8 | T-12 证据库投毒：来源身份服务端验证（#185 source_file_id 必填、sha256/指纹系统生成不接受自报）+ #217 四态（FILE_EXISTS/CONTENT_MATCHES/SEMANTIC_SUPPORT 预留/HUMAN_CONFIRMED 预留）+ CONTENT_MATCHES 必带非空 excerpt | register_evidence 的旧参数拒绝（source_file/source_sha256 传入即拒） |
| W9 | T-13 日志不泄敏感：trace.jsonl 仅工具调用摘要；api_key 绝不落日志/台账（llm.rs 用量累计 .mang/llm-usage.json） | trace 产物抽查 |
| W10 | T-14 暗标：docProps 清零 + 禁字双面扫描（DOCX 全部件 XML + 渲染 PDF 逐页）+ 金丝雀防空洞 + 注入式金丝雀（SC-06 十标识符） | T7d 断言现状 |
| W11 | 人工审批 5 动作矩阵：矩阵 policy 确认/BLOCKER WAIVE/冲突终裁/最终导出放行/提交（人工动作系统只准备不代交）——引擎层 human: 前缀 + 业务层 MCP §8 双层，任一层独立成立 | 导出门 1a/1b/1c + 四门语义 |
| W12 | 幂等铁律：文件 sha256 幂等、idempotency_key 首次结果重放、同门 Approved 幂等但内容变更落新 REV（#180 摘要重绑） | store 幂等面 |

### 7.2 MCP 契约关键承诺（docs/bid-mcp-contract.md，R1 工具面对账基准）

| # | 承诺 | 对账要点 |
|---|---|---|
| C1 | 通用信封 `{ok, tool, data, request_id, project_version?, warnings?}`；错误 `{ok:false, tool, error:{code, what, hint?, item_errors?}}`（what 非 message——#80 摩擦 #12 修订） | 全工具响应包装一致性（AUDIT-SDK-003 疑已修，需全量核对） |
| C2 | 错误码八枚：E_AUTH/E_NOTFOUND/E_STATE/E_CONFLICT/E_PARTIAL/E_UNPROCESSABLE/E_RATE/E_INTERNAL | 错误码穷举与路由 |
| C3 | 大结果分页 fail-closed 游标（§0）：opaque base64、跨过滤器复用→E_UNPROCESSABLE、limit 1..=1000 越界显式拒不静默 clamp、行序确定性（矩阵=条款位置序/证据=id 字典序/任务=target_id 字典序） | cursor.rs（bid-mcp）+ get_compliance_matrix/list_evidence/list_tasks |
| C4 | 工具面 24 个（§0 口径：8 原始 + get_bid_project_state + list_evidence/list_tasks + save_version + #80 后 21 + #219 三工具 24）；外部 Agent 纯 MCP 全链闭环（create→import→extract→build_outline→register/bind→（waiver 门）→三门→check_bid_quality→draft→validate→export） | tools.rs TOOL_NAMES 清点 + mcp_scaffold_tools.rs 闭环测试 |
| C5 | §2 import：sha256 幂等去重 + #188 同 SHA 异身份 FileRole 追加不覆盖 + METADATA_CONFLICT 告警；#183/#189 围栏；#184 addendum_readiness 分级（READY/FAILED/OCR_PENDING）；E_PARTIAL 不吞成功面 | import 响应结构逐字段 |
| C6 | §3 extract：#219 二步合并（Agent 块标记优先+正则兜底、覆盖块正则丢弃、整集替换语义、无标记严格 no-op GT 红线零位移）；AGENT_MANDATORY_OFFCATEGORY 已接线 blocking；合并守恒 counts.total = regex_total - dropped + agent | merge.rs + tools.rs:837+ |
| C7 | §5 search_evidence 双通道 + 确定性 tie-breaker（R-15）+ #217 正文摘录面（excerpt 上取 500 字）；M2-b 落地必须走 cursor.rs 统一分页 | 检索面 |
| C8 | §6 draft：四态生成器（auto/mock/real/external 互斥）；readiness 七种 Blocker（无关联 REQ/缺资质/业绩不足/技术参数无佐证/否决项未响应/MissingAddendum/UnreadyInput）；input_integrity RESTRICTED；draft_note 账面与正文严格分离（正文零标记）；幂等键 (section_id, instructions_hash, inputs_version) + external 追加 content 指纹 | readiness.rs 七 Blocker 枚举 |
| C9 | §7 validate：categories 四桶（structural/semantic not_performed 诚实/evidence/compliance）；ok=true 仅指"已执行桶中 BLOCKER==0"；project_version 步进语义（每次内容面落库 +1，一次调用可 +N，只读零步进）；REVIEWING 为导出时瞬态 | **与 #243 直接相关**：validate 推进状态的行为正是 #243 要治理的对象——契约 §7 与 issue #243 的目标（起草中校验无状态副作用）存在待协调张力，审计时应一并裁决 |
| C10 | §8 export：output_path 必填工作区相对路径；md 块语法（表格/图片 EV-id 解析/缺图响亮拒绝 E_UNPROCESSABLE）；六表七件对账（tables_written==6、forms_written==7、e2e 解包 docx 深断言不信任工具返回值）；幂等键含 decision_fp；#180 审核内容绑定（reviewed_content_digest ≠ 当前 → 拒）；三门 Approved + 全 RT DONE | export.rs + 12_export-check.json |
| C11 | §9 check_bid_quality：只读诊断（报告是导数不是源数据）；sections 只列叶节点（章节发现面）；B3 三维评分由真实评分项 REQ 驱动、无评分项不伪装分 | quality.rs |
| C12 | §11-A register_evidence：来源身份服务端验证（旧参数废除传入即拒）；#217 四态；语义位查重（同指纹幂等回原 id、异指纹旧记录 SUPERSEDED）；locator 缺省诚实标记 mcp-registered | **与 #245 直接相关**：四态无占位态即 issue 目标缺口 |
| C13 | §12 record_review_gate：receipt 验签五分类错误码（Malformed/NotHuman/Expired/FutureDated/BadSignature）；门序（矩阵先大纲、pre_export 要求前两门 Approved）；#180 每条落 sections_digest；addendum_waiver 第四门与三门正交 | review.rs + principal.rs |
| C14 | §13~15 评分三工具：score_price 纯计算器（五方法闭式最优、无项目也能算）；score_section/score_document diagnostic_only=true result_kind=rule_heuristic（**不是评标模拟**）；#224 多对多 weight_share 摊分 + unmapped_scoring_items + mappings 报告 + contributions 聚合；#170 章引用联结（chapter_ref_of→params.section_title→build_scorer 两路首现序）；价格项不入 unmapped | scoring 面（R1 对账重点——#224 第 2/3/4 步遗留） |
| C15 | §16 record_addendum_waiver：件级 receipt（scope=addendum_waiver:{file_id}，TF-0003 的 receipt 免不了 TF-0004）；追加式审计日志不覆盖；写账 content_version+1；分级取重（有 FAILED 报 parse_failed 否则 ocr_pending） | 与 #184 联动 |
| C16 | §17~19 #219 三工具：get_document_chunks（size≤200 防上下文吃爆）；submit_block_marks（整集替换非增量、原子落库持久化、四 verdict）；get_coverage_report（uncovered_ranges 合并区间）；**#244 要补的正是覆盖报告→建纲之间的处置闭环** | coverage_tools.rs 13 测试 |
| C17 | 真实语料红线（#131）：real_corpus_e2e 三件水科 PDF 独立项目全链，L1 召回 ≥309/318=0.9717、否决类精确率 ≥291/302=0.9636（#123 基线逐位不退化）；合成面（synth-tender 115 REQ + e2e 十项深断言）双轨把关——"合成通过 ≠ 真实通过" | tests/real_corpus_e2e.rs 现状与红线数字 |
| C18 | 附录 A KBAdapter：知识库不是 MCP 工具而是 import/search 的可替换实现层；AdapterA 自建默认；附录否决记录（Dify Cloud 出域一票否决/Cherry AGPL） | 架构约束非代码 |

### 7.3 领域模型/架构决策关键承诺（R1 域模型对账基准）

- ID 三命名空间永不混用（REQ/EV/SEC vs DOC-*）；content_ref 只存引用；溯源锚原文片段创建时固化不可篡改；诚实状态显式枚举禁空值默认。
- ProjectStatus 11 态线性（DRAFT→…→EXPORTED）+ DIRTY 章节级重跑不整体回退 + E_STATE 越级保护。
- 矩阵四态（NOT_RESPONDED/PARTIAL/RESPONDED/NOT_APPLICABLE）从绑定+章节+覆写三真源派生（derive_matrix_rows 单一真源）；blocking=必选类（资格/否决）∪ agent_mandatory 未响应；id=CMP-{REQ 后缀} 派生稳定。
- EvidenceStatus 迁移单向（任何→Active 拒——静默复活禁令）；valid_until<开标日→Expired 禁绑；Expiring=开标日前 90 天窗口可绑+告警。
- 审核门四道：matrix_approval/outline_approval/pre_export_confirm 门序链 + addendum_waiver 正交门（无窗口限制、不入 export_allowed 门序）。
- AD-08：运行产物契约 = bid-tools/src/schema.rs serde 单一真源，schema 漂移编译期捕获；Python 收敛为存档用途（fixture_gen.py 数据出处 + xref_check.py 研究检查器）。

---

## 8. 后续轮衔接建议（给主控的消化结论）

1. **R1 模块审查前必须先做"防线归属追查"**（§6-A2）：zip/EOCD/xlsx 双轴门族在 B4 重构后的真实归属（引擎 office-io or 丢失）——这决定 threat-model T-03 是否需要重写、R5 全部 zip 面历史结论是否作废。
2. **R2 测试质量轮的基线**：R5 §4 的 34 项假绿猎杀模板（测什么面/攻击样本/破坏推演红证/是否被上游先拒）+ #203 五条"特别认同"坑位（自证测试/双实现同错/SKIP 遮蔽/失败转 SKIP/降阈值造绿）+ 修订 3 的 cargo-mutants 白名单口径。
3. **#243/#244/#245 三个开放 issue 的验收标准已原样提取于 §4**——调查目标定义以 issue 正文为准，注意三个 issue 都强调"在当前 main 复现/重跑基线，不沿用历史数字"。
4. **健康检查需冻结版本重跑**（§5）：0.386 否决召回是 7fff169 基线数字，且语料 .gitignore（provenance 可再取）。
5. **产物基线新鲜度**：artifacts/e2e/latest 存在 52→50 漂移嫌疑（#241 评论），引用任何 artifacts 数字前先验证其对应 HEAD。
6. web-ui E2E 前置工程（修订 2）仍未启动（package.json 无测试框架）——CI 重建轮（R6）的第一批料。

---
*本文件由历史资料消化代理（Task 2-a）生成于 audit/v2-full-coverage 分支；只读审计，未修改任何生产文件。文中所有"当前 HEAD 初步核对"均为只读 grep/read 抽查结论，不构成正式审计发现，正式结论以各专项审计轮报告为准。*
