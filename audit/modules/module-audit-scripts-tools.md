# 模块审计 E3：scripts 工具面 + veto_gt GT 生成器（module-audit-scripts-tools）

- 审计代理：4-e3（ai-bid-research 全仓审计 V2 · Task ID 4-e3）
- 冻结基线：main@6105beb（分支 audit/v2-full-coverage；工作树含 R1 审计提交，生产代码与基线一致——`git log -1` 为审计文档提交）
- 范围：scripts/e2e_bid/（5）、scripts/mang-dev/（5）、scripts/fp_attribution/（3）、scripts/workspace-cleanup/（3）、scripts/package-mang-app.ps1（1）、scripts/veto_gt/（8 源 + Cargo.toml）、6 个 Rust 工程（baseline_harness / bid_health_check / engine_probe / title_impact / veto_measure，8 个 .rs + README + 5 个 Cargo.toml + Cargo.lock）——逐文件全读
- 纪律：只读审计；未修改任何生产文件/测试/配置/数据；未运行任何脚本/工具；本文与 inventory-e3.csv、module-audit-sdk-skill.md 为本轮唯一新增产出
- 交叉参照：`audit/modules/module-audit-bid-mcp.md`（25 工具清单）、`bidcore/crates/bid-parse/src/veto_lexicon.rs`（生产判定词表）、`artifacts/veto-gt/ground-truth.json`（GT 本体）、git 历史 4 个 GT 相关提交

---

## 1. 头号问题：veto_gt GT 生成器的 Oracle 独立性判定

**结论（一行）：GT 与生产 veto_lexicon.rs/veto_rule.rs 非逐字同源（词表非复制、终裁走语义规则），但标注者自述为 AI（非人工）、增补轮 S3/S5/S6 以引擎自身 FP 输出为工作清单回改 GT、且 #52 排除规则以「对 GT 1752 true 命中=0」为红线与 GT 共同演化——构成结构性的「标注↔实现」反馈环，real_corpus_e2e 的 L1/P 红线（0.9717/0.9636）只能解释为池内相对指标，不构成独立 Oracle 下的绝对召回/精确率（P1，方法学发现，GT 头部已诚实声明部分局限）。**

三层证据：

1. **标注主体 = AI**。`artifacts/veto-gt/ground-truth.json` method.annotator 原文：`"annotator": "AI 标注员（executor-agent，GLM）——如实声明：非人类人工标注"`。S1 轮（1212 true）由宽模式候选池 + AI 逐条裁决产生；增补轮 method 自述「三子代理并行裁决 + 主裁全量 true 复核改判 5 条」——主裁同为代理体系。
2. **引擎 FP 反馈环（核心）**：
   - `scripts/veto_gt/supplement_scan.py:27`（`MEASURE_PATH = '/tmp/main-verify.json'`，S3 家族=「FP 清单：引擎已抽但 GT 无对齐 true 的条目」）；
   - `scripts/veto_gt/s5_residual.py:17`（`MEAS = 'artifacts/veto-measure/after-gt-supplement.json'`）与 `s6_micro.py:6` 同——S5/S6 直接以 veto-measure（跑生产 `bid_parse::extract::extract_requirements`）的 FP 清单为输入，用正则规则机自动改判/补 true 后并入 GT；
   - 反向：生产 `veto_lexicon.rs:206-215`（#52 排除面）自述方法论为「493 条 FP 全量归因 + TP 误伤面扫描——每条规则对 GT 1752 true 的命中数必须为 0」；
   - git 提交自证：`860ddfd feat(#48): GT 增补轮1 修尺子——候选池盲区补标 +540 true，P 0.5057→0.7519 达标`、`1635773 fix(#68): 阶段 4 PDF L2 残差修复——语境传播工程化，0.6960→0.8067（红线全过）`——「修尺子」即 GT 随引擎指标目标共同调整的直接记录。
3. **候选池词面与生产判定面部分同源**。dump.rs:21-24 的 20 词取自 LingYan `veto_lexicon.json`（外部 MIT 竞品仓，与 bid-health-check 体检 truth 面同源——注释自认）；生产 `veto_lexicon.rs` 的 `re_veto_strong`/`re_negative_list` 是另一套更大的正则短语集，但与池词共享词面（否决/无效/废标/拒收/不予受理/取消/没收/列入 等 8/20 重叠）。池用 `contains` 匹配，凡不含池词也不含 37 句式的真否决条款**从进不了候选池**→ GT known_limits 第一条自认「候选池外的否决条款存在漏标风险」——即召回分母与生产盲区同向受限，召回红线无法证明池外召回。S4 抽样（40/件、种子 20261029）只量化残余盲区，未消除。

**对照问题「是否同源→自证」的裁定**：非「复制生产词表当 GT」的直接自证（生产 `is_veto` 的短语集与池词集不同构，GT true 中大量条目（成组列举子项、评审表行）根本不含触发词、靠组传播/表级扫描入池）；但 S3/S5/S6 把引擎 FP 系统性转写为 GT 条目（双生补锚/误标改判），加上 #52 规则以 GT 为零误伤红线——精确率指标存在明显的拟合面。**定级 P1（非 P0）理由**：GT 头部 method/known_limits 与脚本纪律声明均如实披露（「只按领域语义标注，不看实现结果」的纪律写明、S5 未命中规则项报人工、增补轮判例口径延续），且 veto-measure 的三通道块对齐以解析确定性为前提独立于标注；但披露≠独立，指标解读面（real_corpus_e2e.rs:11-12 红线「逐位不退化」）未标注「池内口径」限定语。

### 1.1 veto_gt 工程结构（8 文件全读）

| 文件 | 角色 | 关键行为 |
|---|---|---|
| src/bin/dump.rs | 候选池生成（标注工作底稿） | 生产路径解析（镜像 bid-tools service.rs 路由：pdf→引擎 Err 回退 legacy；docx→引擎）；20 词 + 37 句式宽模式标记 candidate；输出 `<file>.blocks.json` |
| src/bin/baseline.rs | 基线测量 | 生产 `bid_parse::extract::extract_requirements` 全量 REQ dump（id/category/clause_no/anchors）——**调用当前生产实现**，非内置复制 |
| supplement_scan.py | 增补轮工作清单 | S1 组传播/S2 表行/S3 引擎 FP/S4 随机抽样（种子 20261029）四家族 worklist |
| supplement_merge.py | 增补并入 GT | 幂等（剔旧 `-S1-` id）；assert len==1624 / 原 true==1212 硬编码自检；写 supplement 元数据 |
| s5_residual.py | 残余 FP 清扫 | 读 after-gt-supplement.json 的 fp；双生文本补锚 + 11 条误标改判正则 |
| s5_final.py | 终裁规则机 | 10 TRUE_RULES + 28 FALSE_RULES + 5 组 SPECIAL_TRUES；未命中报人工 |
| s5_merge.py | S5 并入 | 幂等（剔 `-S5-`）；更新 supplement/stats |
| s6_micro.py | 微清扫 | 复用 s5_final 规则机（exec 源码截取）+ 4 条 TRUE_EXTRA；直接写 GT |

veto_measure/src/bin/measure.rs（测量器）：调生产 `bid_parse`/`bid_engine`（path 依赖当前 HEAD），L1 三通道覆盖（块对齐优先/拼接包含/≥6 字锚子串）、L2 分类召回、否决类精确率 + FP 归因诊断（回放 `veto_lexicon::is_veto` 等 5 个判定原语）。测量器本身只读、确定性、无模型调用 ✔。

---

## 2. 问题记录（SCR-200 起）

### SCR-200
- ID: SCR-200
- 严重程度: P1
- 模块/文件路径: scripts/veto_gt/（supplement_scan.py、s5_residual.py、s6_micro.py、s5_final.py、supplement_merge.py）× artifacts/veto-gt/ground-truth.json × bidcore/crates/bid-parse/src/veto_lexicon.rs
- 代码行号/函数或脚本: supplement_scan.py:27（MEASURE_PATH）、s5_residual.py:17（MEAS）、s6_micro.py:6（MEAS）、s5_final.py:15-56（TRUE/FALSE_RULES）、ground-truth.json method.annotator；veto_lexicon.rs:202-215（#52 方法论注释）；git 860ddfd/1635773
- 问题描述: GT 增补轮以引擎自身 FP 输出为工作清单回改 GT；生产排除规则又以 GT true 零命中为设计红线；标注者为 AI（GLM）非人工——Oracle 独立性为「弱」，指标链存在标注↔实现反馈环
- 调用链: veto-measure（生产 extract_requirements）→ after-gt-supplement.json(fp) → s5_residual/s6_micro 规则机 → ground-truth.json → real_corpus_e2e.rs:42 红线对照 →（修复）→ veto_lexicon.rs #52 排除面（以 GT 1752 true 零误伤验收）→ 再 measure → …
- 触发条件: 任何引用「L1 ≥0.9717 / P ≥0.9636」作为独立质量声明的场景（readme/报告/对外口径）
- 实际影响: 否决召回/精确率红线只能证明「候选池内 + 引擎反馈校准后」的相对性能；池外真否决（不含 20 词与 37 句式者）召回不可证；精确率含 GT 拟合成分（S3 家族直接把引擎 FP 转为 GT true）
- 复现步骤: 读 ground-truth.json method/supplement.method + supplement_scan.py 头注 S3 定义 + git show 860ddfd --stat
- 当前测试: real_corpus_e2e.rs（红线）、veto-measure 三跑口径；均以同一 GT 为真值
- 缺失测试: 独立第二标注者/人工抽检交叉一致率（κ）；池外全量盲扫（非抽样）估计漏标率
- 建议方案: ① GT 头部与 real_corpus_e2e 断言处补「池内相对口径」限定声明；② 增补轮条目已带 `s5_kind/family` 标记——红线指标可分「原 S1 面」与「引擎反馈面」两组分别报告；③ 对 5088 条已裁决块做人工抽检（如 5%）测标注一致率
- 证据: `"annotator": "AI 标注员（executor-agent，GLM）——如实声明：非人类人工标注"`；`MEAS = 'artifacts/veto-measure/after-gt-supplement.json'`（s5_residual.py:17）；`860ddfd … GT 增补轮1 修尺子——候选池盲区补标 +540 true，P 0.5057→0.7519 达标`
- 审计置信度: CONFIRMED
- 状态分类: 当前仍存在（披露存在、独立性缺口存在）

### SCR-201
- ID: SCR-201
- 严重程度: P2
- 模块/文件路径: artifacts/veto-gt/ground-truth.json（stats 块）——由 scripts/veto_gt/s6_micro.py:122-125 之后的 945e0b8 直接编辑引入
- 代码行号/函数或脚本: gt['stats'] vs annotations 实测：annotations=5138 条 / true=1802；stats.veto_true=1752 / candidates_adjudicated=5088（差 50）
- 问题描述: 945e0b8（#52,#68 Lead 裁决①——RELATION/RECUSE 口径统一 true）向 annotations 追加 50 条（35 SUP + 12 S2 + 3 REC，全部 is_veto=true，id 前缀 `S68-*`），**未更新 stats 块、且 scripts/veto_gt 无对应生成脚本**（不可复现、无幂等通道）
- 调用链: ground-truth.json ←（945e0b8 手工/未入库脚本编辑）；下游消费者：veto_lexicon.rs:209 注释「GT 1752 true」、tp_collateral_scan.py 头注「1752 条 true」
- 触发条件: 任何信任 stats 块或「1752」口径的消费（文档引用、红线叙述）
- 实际影响: 元数据与本体漂移 50 条（318 红线分母的 A-01/A-04/A-05 三件中 31 条来自该批）；测量器/红线测试按 annotations 全量计数（不受影响），但人工口径叙述失真
- 复现步骤: `python3 -c "import json;g=json.load(open('artifacts/veto-gt/ground-truth.json'));print(len(g['annotations']),sum(1 for a in g['annotations'] if a['is_veto']),g['stats']['veto_true'],g['stats']['candidates_adjudicated'])"` → 5138 1802 1752 5088
- 当前测试: 无（GT 无自检测试；supplement_merge.py 的 assert 只在其自身运行时有效）
- 缺失测试: GT 一致性校验（stats==实际计数、id 前缀白名单、files 清单覆盖）
- 建议方案: 补一个只读校验脚本（或 veto-measure 启动时校验 stats 一致性）；945e0b8 类直改入库前过校验
- 证据: `Counter({'GT': 3279, 'S1': 1624, 'S5': 134, 'S6': 51, 'SUP': 35, 'S2': 12, 'REC': 3})`；`"veto_true": 1752` vs 实测 1802
- 审计置信度: CONFIRMED
- 状态分类: 当前仍存在

### SCR-202
- ID: SCR-202
- 严重程度: P2
- 模块/文件路径: scripts/fp_attribution/fp_attribution.py、scripts/fp_attribution/tp_collateral_scan.py
- 代码行号/函数或脚本: fp_attribution.py:10（`FP = json.load(open('/home/z/my-project/scripts/fp_diag.json'))`）、:108-109（写 `/home/z/my-project/scripts/fp_buckets.json`）；tp_collateral_scan.py:8（`/home/z/my-project/wt-fp/artifacts/veto-gt/ground-truth.json`）
- 问题描述: 两脚本输入/输出均硬编码仓外绝对路径，且该路径当前不存在（实测 `/home/z/my-project/scripts/fp_diag.json`、`/home/z/my-project/wt-fp` 均不存在）——#52 归因流程不可重跑；仓内 `fp-diag-baseline.json`（493 条 FP 快照）不是脚本的输入
- 调用链: 手工运行（issue #52 流程）；产物 fp_buckets.json 未入库
- 触发条件: 重跑归因/误伤扫描
- 实际影响: #52 的 493 条 FP 归因与 1752 true 误伤验证过程不可复现，只能信仓内快照与 veto_lexicon.rs 注释
- 复现步骤: `python3 scripts/fp_attribution/fp_attribution.py` → FileNotFoundError
- 当前测试: 无
- 缺失测试: 无（一次性分析脚本）；缺「可复现说明」
- 建议方案: 脚本参数化（argparse 输入路径），或头注写明快照再生命令（veto-measure --out fp_diag.json）
- 证据: `FP = json.load(open('/home/z/my-project/scripts/fp_diag.json'))`
- 审计置信度: CONFIRMED
- 状态分类: 当前仍存在

### SCR-203
- ID: SCR-203
- 严重程度: P2
- 模块/文件路径: scripts/engine_probe/Cargo.toml
- 代码行号/函数或脚本: `mdc-office-io/mdc-pdf git rev = "3204342d9a5e…" `（与 bidcore/crates/bid-engine/Cargo.toml 的 `f4328ebf…`（#110 任务 2 bump，相差 87 提交）不一致）
- 问题描述: engine_probe 钉死的 mdcore rev 落后生产 bid-engine 87+ 提交；README 还停留在 f8c860a/f7ddd39 worktree 时代叙述——B2 引擎探针面（S7/S8/D456/X3/d6rev 15 探针 + p1 布局保真）测的是旧引擎，B4/后续 mdcore 演进后探针结论不再代表当前引擎
- 调用链: cargo build scripts/engine_probe → mdc-office-io@3204342d（≠ 生产 f4328ebf）
- 触发条件: 以 engine_probe 结论佐证当前引擎行为（如 zip 炸弹门、嵌套归档、修订/批注读面）
- 实际影响: 探针证据与生产引擎版本脱钩；若 mdcore 在 87 提交中改变读面行为，仓内 report.json 结论过期且无漂移告警
- 复现步骤: 对比两处 Cargo.toml rev 字段
- 当前测试: 无 rev 一致性校验
- 缺失测试: bid-engine 与 engine_probe 的 mdcore rev 一致性断言（或探针报告内嵌 rev 与生产 MDCORE_REV 对账）
- 建议方案: 探针 rev 跟随 bid-engine 同步 bump，或在 report.json 头部记录探针 rev 并与 bid_engine::MDCORE_REV diff
- 证据: engine_probe: `rev = "3204342d9a5ee33400aba2b17dad29696e2f0bb8"`；bid-engine: `rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b"`
- 审计置信度: CONFIRMED
- 状态分类: 当前仍存在

### SCR-204
- ID: SCR-204
- 严重程度: P3
- 模块/文件路径: scripts/engine_probe/src/main.rs:237、src/bin/p1.rs:10-12
- 问题描述: 环境锚定绝对路径——main.rs 硬编码 `/home/z/my-project/repos/mdcore-audit-f8c860a/test-artifacts/TR2/fixtures`；p1.rs 依赖 `real-corpus/{telecom,water}` 本地目录（不入 git）。缺失时诚实降级（FILE_MISSING/结果标记），不伪造通过 ✔
- 实际影响: 换环境探针静默缩面（d456_tr2_* 三探针 FILE_MISSING）
- 建议方案: 路径参数化或 README 复现清单中显式列出前置目录（README 已部分列出 worktree 命令 ✔）
- 证据: `let tr2 = "/home/z/my-project/repos/mdcore-audit-f8c860a/test-artifacts/TR2/fixtures";`
- 审计置信度: CONFIRMED；状态分类: 当前仍存在

### SCR-205
- ID: SCR-205
- 严重程度: P3
- 模块/文件路径: scripts/bid_health_check/src/bin/probe_a02.rs:8
- 问题描述: 硬编码旧仓路径 `/home/z/my-project/bid-research/artifacts/…`（现仓为 `ai-bid-research`）——探针在当前仓环境必然 panic（expect("parse")）
- 实际影响: A-02 OCR 对账探针不可复现（历史诊断件，影响有限）
- 建议方案: 参数化或标注「历史诊断，一次性」
- 证据: `let path = std::path::Path::new("/home/z/my-project/bid-research/artifacts/bid-health-check/corpus/A-02_福建修订通知_扫描件.pdf");`
- 审计置信度: CONFIRMED；状态分类: 当前仍存在

### SCR-206
- ID: SCR-206
- 严重程度: P3
- 模块/文件路径: scripts/veto_gt/src/bin/dump.rs:21-24
- 问题描述: 注释称「LingYan 20 词（veto_lexicon.json 字面口径）」，实测第 12 词 dump.rs 用「不接受」而 veto_lexicon.json 为「不予接受」——字面口径漂移（方向为更宽池，标注安全向，但「同口径」声明不成立）
- 实际影响: 候选池比声明宽一词；GT method.pipeline 描述与实际池有微小出入（溯源口径）
- 建议方案: 从 veto_lexicon.json 运行时载入词表（health-check 同款 load_lexicon），消除复制
- 证据: dump.rs `("不接受", …)` vs veto_lexicon.json `['…','不予接受',…]`
- 审计置信度: CONFIRMED；状态分类: 当前仍存在

### SCR-207
- ID: SCR-207
- 严重程度: P3
- 模块/文件路径: scripts/veto_gt/s5_final.py、s5_merge.py、s5_residual.py、s6_micro.py、supplement_merge.py
- 问题描述: ① 中间产物依赖 `/tmp/vgt-dump/`、`/home/z/my-project/tmp-conv/vgt-supplement/`（现已不存在的临时区）；② supplement_merge.py:103 `assert len(supp)==len(verdicts)==1624`、:106 `assert len(orig_trues)==1212` 硬编码一次性断言（重放即挂）；③ s5_final.py:96 `SKIP = {('A-18…pdf', 276)}` 特例硬编码；④ 各脚本须在仓根 cwd 运行（相对路径 `artifacts/…`）且无头注完整复现命令（supplement_scan 有、其余部分有）
- 实际影响: 增补轮链路只可按 git 历史理解，不可机械重放——GT 的「可复现性」停留在 veto-measure 重测口径（测量可重放、标注过程不可重放）
- 建议方案: 归档 tmp-conv 中间产物或写 REPRODUCE.md（输入→输出→断言的全链命令）
- 证据: `OPEN = '/home/z/my-project/tmp-conv/vgt-supplement/s5-open-items.json'`；`assert len(supp) == len(verdicts) == 1624`
- 审计置信度: CONFIRMED；状态分类: 当前仍存在（历史一次性脚本，未声明退役）

### SCR-208
- ID: SCR-208
- 严重程度: P3
- 模块/文件路径: scripts/package-mang-app.ps1
- 问题描述: 校验门为 12 项**存在性**检查（exe/ui 三件套/渲染器/pdf.js cmaps/fonts/wasm），产出的 zip 无 SHA256 清单/签名——绿色包分发给第三方后完整性不可验证（对比：cleanup 产物有 manifest；mang IPC 大响应有一次性 URL）
- 实际影响: 无法检测传输损坏或被替换的分发包
- 建议方案: Compress-Archive 后生成 SHA256 与文件清单写入 zip 旁 `.sha256`/`MANIFEST.json`，并打印校验命令
- 证据: `$checks = @(@{ Path = $exe; …}` … `Compress-Archive -Path … -DestinationPath $zip`（无哈希步骤）
- 审计置信度: CONFIRMED；状态分类: 当前仍存在

### SCR-209
- ID: SCR-209
- 严重程度: P3
- 模块/文件路径: scripts/mang-dev/settings_ui_probe.py:82-88（STUB mcp.status）
- 问题描述: 桩返回 `tools: 13, implemented: 11`——当前真实面为 25 工具（TOOL_NAMES；diag.rs 动态统计 tools_schema）。断言 `"13" in mcp_text` 只验证 UI 能渲染桩值，不覆盖真实 mcp.status 路径（mang_ipc_probe.js 不含 mcp.status 调用）
- 实际影响: UI 探针与真实 MCP 面脱节（#76 时代口径）；「已就绪 + 工具数」渲染逻辑对新数字未经测试（风险低：纯字符串插值）
- 建议方案: 桩值改为 25/25，或标注「stub 值不追踪真实工具数」
- 证据: `case 'mcp.status': return { … tools: 13, implemented: 11, … }` + `check("MCP 分组显示 stdio 就绪 + 工具数", ("stdio" in mcp_text and "13" in mcp_text …))`
- 审计置信度: CONFIRMED；状态分类: 当前仍存在

### SCR-210
- ID: SCR-210
- 严重程度: P3
- 模块/文件路径: scripts/e2e_bid/run_real_llm_path.sh:150-152、run_subagent_path.sh:231-233
- 问题描述: phase2 归档采用 `rm -rf artifacts/e2e-real-path/latest` → `mkdir` → `cp -r "$ART"/. latest/`——cp 中途失败（磁盘满/权限）会留下**半成品 latest** 且旧 latest 已被删；失败清理仅 trap stop_relay（杀中继），cargo test 失败时产物目录不回滚（`set -e` 直接退出）
- 触发条件: phase2 测试失败或归档 cp 中断
- 实际影响: latest 符号性快照可能指向不完整运行（低概率、有人工复核 latest 习惯时可察觉）
- 建议方案: 先 cp 到 `latest.new` 再原子 `mv`（或先备份旧 latest）
- 证据: `rm -rf artifacts/e2e-real-path/latest; mkdir -p artifacts/e2e-real-path/latest; cp -r "$ART"/. artifacts/e2e-real-path/latest/`
- 审计置信度: CONFIRMED；状态分类: 当前仍存在

### SCR-211
- ID: SCR-211
- 严重程度: P3
- 模块/文件路径: scripts/e2e_bid/human_llm_relay.py:146-152
- 问题描述: usage 口径 `prompt_tokens = len(prompt_text)`（JSON 字符串字符数当 token）——llm-usage.json 台账在 real-path 链路中为近似值（已注释声明「中文 1 字 ≈ 1 token 口径，台账可读即可」）
- 实际影响: 用量统计不精确（跨工具对比时口径差异）；评测语义不受影响
- 建议方案: 台账字段名标注 `approx` 或按字符/2 估计
- 审计置信度: CONFIRMED（口径已声明）；状态分类: 文档与实现不一致（轻微）→ 归 DESIGN_PREFERENCE

### SCR-212
- ID: SCR-212
- 严重程度: P3
- 模块/文件路径: scripts/veto_measure/src/bin/measure.rs:220-224
- 问题描述: L1 覆盖通道 (c)：任一 anchor 规范化 ≥6 字是 GT 全文**子串**即算覆盖——短锚（如单行「废标。」在 GT 跨行长条款内）即可记覆盖，L1 召回存在高估方向（代码注释已自认「行级碎片文本面兜底」）
- 审计置信度: CONFIRMED（口径声明存在）；状态分类: DESIGN_PREFERENCE

### SCR-213
- ID: SCR-213
- 严重程度: P3
- 模块/文件路径: scripts/bid_health_check/src/main.rs:36-47
- 问题描述: `STRONG_VETO_PHRASES` 是 extract.rs 私有 `re_veto` 的手工镜像（8 短语），注释自认「两处若漂移以 extract.rs 为准」——无自动对账，extract.rs 演进后体检的「类别命中」维度静默过期
- 建议方案: 抽检面改为调用 `veto_lexicon::is_veto`（已是 pub）或加镜像一致性测试
- 证据: `/// 强否决短语（**镜像** extract.rs 私有 re_veto 的判定动词面…两处若漂移以 extract.rs 为准）`
- 审计置信度: CONFIRMED；状态分类: 当前仍存在

---

## 3. 各目录审计结论（无问题面也记录）

### 3.1 scripts/e2e_bid/（5 文件全读）
- **run_e2e.sh**：入口为 `cargo test -p bid-mcp --test e2e_pipeline`（**构建并跑当前 HEAD** ✔）；预检 fixtures/corpus-A 真件缺件 → 响亮 SKIP exit 0（issue #9 纪律）；PATH 注入 `$HOME/.cargo/bin`（Linux CI/本地 rustup 约定）；无 LLM key 依赖（链路 mock 生成）✔。
- **run_e2e_artifacts.sh**：`--test e2e_artifacts`，载体 fixtures/synth-tender（仓内自带 → CI 可跑）；产物 artifacts/e2e/<run-id>/ 十一件套 + latest；无清理/回滚逻辑（产物追加式，风险低）。
- **run_real_llm_path.sh / run_subagent_path.sh**：两相位（capture→人工/子代理撰文→serve）；stdio 二进制先 `cargo build` 当前 HEAD 再以 `BID_REAL_PATH_STDIO_BIN/BID_SUBAGENT_STDIO_BIN` 显式注入 ✔（target 目录用 cargo metadata 权威解析 ✔）；llm.json 指向 127.0.0.1 本地中继（key 本地占位、不出网 ✔）；trap EXIT 杀中继 ✔；见 SCR-210（latest 半成品）。**人工/模型边界声明充分**：脚本头注与 relay 文档串三处强调「serve 正文必须开发者/子代理撰写，不是引擎输出回灌——那是循环论证」，且 prompt 全量落盘可审计（relay-prompts/ 入库归档）。
- **human_llm_relay.py**：OpenAI 兼容 chat/completions 最小端点（ThreadingHTTPServer）；Bearer 校验、capture 落盘 prompt-<n>.json + 503（真实错误语义）、serve 单章 --content / 多章 --content-map 按「章节标题：」路由（缺失即 500 响亮拒，无静默回退 ✔）；见 SCR-211。

### 3.2 scripts/mang-dev/（5 文件全读）
- **mang_dev.ps1**：build/selftest/ui/run/smoke 快捷面；构建 release mang-app；Stop-MangApp 杀占用进程（dev 工具可接受）。
- **mang_ipc_probe.js + mang_ipc_smoke.mjs**：WebView2 真实窗口 CDP 冒烟——探针消息集 `ping/app.info/fs.tree/fs.read/fs.readBytes/fs.writeBytes/fs.stat/recent.load/history.list/unknown_cmd` **全部存在于 ipc.rs:351-447 分派表**（无过期消息名 ✔）；`__ipcResolve/__ipcEvent` 注入契约与 main.rs:11-12/503 一致 ✔；覆盖路径围栏（outside_workspace）、writeBytes 拒正文/允 .mang、大响应落盘一次性 URL（404 二次）——与 fsapi 行为对齐 ✔。mcp.status/llm.*/fspec.* 等新消息不在探针面（覆盖缺口，非漂移）。
- **settings_ui_probe.py / settings_layout_probe.py**：Playwright + 桥接桩（window.ipc 桩模拟壳）；验证 7 分组/llm.save 参数收集/llm.test 异步事件回执/拖拽 userSelect/深浅主题；见 SCR-209（桩 mcp.status 工具数 13/11 过期）。探针依赖 web-ui/dist 在场（npm 构建前置）。

### 3.3 scripts/fp_attribution/（3 文件全读）
fp_attribution.py（493 FP → 20 桶，GT 侧/引擎侧/未归类三分类，UNATTR 残余打印）；tp_collateral_scan.py（排除规则候选对 GT true 全量命中扫描，红线=0）；fp-diag-baseline.json（493 条 FP 入库快照，含 veto_lexicon 判定面回放 diag 字段——veto-measure measure.rs:270-283 的输出形状 ✔）。见 SCR-202。**注意**：#52 排除规则最终入生产 veto_lexicon.rs，而「对 GT true 零命中」红线是规则设计约束——见 SCR-200 第 2 层证据。

### 3.4 scripts/workspace-cleanup/（3 文件全读）
- **cleanup-audit.ps1**（806 行全读）：危险面清点——目标是 audit 根（默认 E:\Code\ai\md\audit）顶层散落文件与子目录；判定四值 TRASH（→`_work/.trash/<ts>/` 7 天缓冲）/ARCHIVE（→`_archive/` 只进不删）/KEEP/LEAVE；保护名单 ai-bid-research/mdcore/_pack/_archive/_work/docs 绝不触碰；三重预检（audit 非 git 仓/不在上层仓内/保护仓 .git 在）+ 每批（500 项）git status 基线逐字比对护栏，异常即中止；PurgeTrash 需 -Yes；隐藏文件（.env）只归档。**危险面评估：低**（四层防护成立，唯一不可逆点 = 7 天后周清理/PurgeTrash，均有确认门）。
- **test-cleanup-195.sh**：T1-T8 模拟工作区测试（Linux pwsh 7.4.6 实测 68/68 全绿，CLEANUP.md 第十节诚实声明 Windows 原生未实测）——测试对象即本脚本的行为面（干跑零副作用/分类/幂等/周清理/脏仓库/三种拒绝路径/清单 CSV 抽检）✔。
- **CLEANUP.md**：政策全文 + 验收对照 + 已知边界（诚实口径）✔。

### 3.5 scripts/package-mang-app.ps1
见 SCR-208（无哈希/签名）。正面：缺件即失败不产半包；版本号从 mang-app Cargo.toml 提取；`--ls-ui/--selftest` 验证指引打印。

### 3.6 Rust 工程五件（逐文件读）
| 工程 | 调用面 | 与生产关系 | 产物 |
|---|---|---|---|
| baseline_harness | `bid_parse::parse_file`（path 依赖当前 HEAD） | **无内置复制算法**（头注纪律「不含任何解析业务逻辑」✔）；panic→101/Err→3 退出码约定 | baseline/parse/*.json（B1 基线，由 scripts/baseline_run.py 编排） |
| bid_health_check | `bid_engine::parse_{docx,pdf}_via_engine` + `extract_requirements`（镜像 service.rs import 路由）+ LingYan 20 词表（--lexicon） | 生产实现直调 ✔；STRONG_VETO_PHRASES 镜像（SCR-213）；「否决召回」为词表近似真值面（REPORT.md 自述方法学局限）| artifacts/bid-health-check/results/*.json + BASELINE.json + REPORT.md（**REPORT.md 为人工撰写的体检报告，BASELINE/results 为本 harness 生成器输出**——报告头「基线：main @ 7fff169」与环境预检 mdcore_rev 字段构成溯源链 ✔） |
| engine_probe | mdc-office-io/mdc-pdf @ git rev 3204342d | **钉旧 rev**（SCR-203）；探针本体为引擎 API 直调 + 运行时构造样例（zip 炸弹/高压缩比/嵌套/伪装）——非复制算法 | stdout JSON → artifacts/engine-probe/report*.json |
| title_impact | `bid_parse::extract_requirements` 真跑 ×3 变体（V0/V1 标题剔除/V2 A2 污染） | 数据准备层镜像 extract.rs 章节正则（注释自认）；判分交 title_impact_score.py | 报告 JSON |
| veto_measure | `bid_parse`/`bid-engine` path 依赖（当前 HEAD） | 测量器（生产实现直调）；见 SCR-212 | veto-measure/*.json（红线数据源） |

**bid_health_check 与 REPORT.md 关系结论**：REPORT.md 是「报告层」（人写+引用 harness 输出），BASELINE.json/results 是「数据层」（harness 生成，schema 稳定可 diff）；两者分离清晰，无生成器冒充报告的问题。

---

## 4. 测试覆盖与结论
- scripts/ 下 12 个历史 Python 分析脚本（telecom_*、v1_*、gh_* 等）不在本轮 E3 范围（属其他批次）；范围内工具的「测试」现状：workspace-cleanup 有完整模拟测试（68 断言）✔；engine_probe/bid_health_check/veto_gt 无自动化测试（一次性审计 harness，靠确定性 JSON + 入库快照）；e2e_bid 四 shell 的内核是 bid-mcp Rust 测试（e2e_pipeline/e2e_artifacts/real_llm_path/subagent_llm_path——默认 SKIP 面见各测试文件）。
- 风险分级汇总：P1×1（SCR-200），P2×4（SCR-201/202/203 + 无），P3×9（SCR-204~213）。
- 全部 P0 检查完毕：**未发现 P0**（veto_gt 独立性定 P1 的理由见 §1——非直接自证、有诚实披露、测量器独立于标注可重放）。
