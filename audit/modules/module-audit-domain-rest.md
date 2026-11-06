# 模块审计 A2-1：bid-domain 剩余模型（scoring/pricing/outline/forms/param/matrix/party/budget/impact/assignment）

- 审计代理：4-a2（ai-bid-research 全仓审计 V2，Task ID: 4-a2）
- 基线：main@6105beb（分支 audit/v2-full-coverage）
- 纪律：只读审计。本文件与 inventory-a2.csv 是本轮 A2 新增文件之一；未修改任何生产/测试/配置/数据文件，未运行 cargo test/build。
- 审计方式：声明范围 10 个生产源码 + 4 个测试文件逐文件通读（长文件分段读全）；调查链扩展定向读取（见 §0，未逐行的不在覆盖声明内）。
- 试点交叉引用：PB-001/005/006/007/008/011（audit/pilot/pilot-b-domain-evidence.md）——已登记项不重复登记，仅引用。

---

## 0. 覆盖声明与调查链

### 0.1 逐文件全覆盖（14/14）

| # | 文件 | 行数 | 覆盖 |
|---|---|---|---|
| 1 | bid-domain/src/scoring.rs | 2387 | 全读（生产面 1-1296 逐行；1297-1468 DomainStore 扩面逐行；1469-2387 内联测试全读） |
| 2 | bid-domain/src/pricing.rs | 1887 | 生产面 1-720 逐行；测试 722-1887 全读（8 套真实规则对照/网格验证/退化等价锁定） |
| 3 | bid-domain/src/outline.rs | 1385 | 全读 |
| 4 | bid-domain/src/forms.rs | 886 | 全读 |
| 5 | bid-domain/src/param.rs | 799 | 生产面 1-281 逐行；测试 282-799 全读 |
| 6 | bid-domain/src/matrix.rs | 639 | 全读 |
| 7 | bid-domain/src/party.rs | 537 | 全读 |
| 8 | bid-domain/src/budget.rs | 529 | 全读 |
| 9 | bid-domain/src/impact.rs | 523 | 全读 |
| 10 | bid-domain/src/assignment.rs | 485 | 生产面 1-216 逐行；测试 218-485 全读 |
| 11 | bid-domain/tests/scoring_mapping.rs | 891 | 全读 |
| 12 | bid-domain/tests/budget.rs | 322 | 全读 |
| 13 | bid-domain/tests/scoring_b2_self_eval.rs | 246 | 全读 |
| 14 | bid-domain/tests/outline_invariants.rs | 199 | 全读 |

### 0.2 调查链扩展读取（#224 第 2/3/4 步推演必需，定向读关键面）

- bid-mcp/src/tools.rs（4311 行）：build_scorer（2852-2982）、score_section（2403-2458）、score_document（2473-2741）、chapter_no_of/first_leaf_under、search_evidence（1581-1750）、build_outline（3609-3688）、parse_price_spec。
- bid-tools/src/service.rs（1834 行）：prepare_outline_and_matrix（822-830）、register/replace_requirements、build_matrix 触发点（728/750/827/984）。
- bid-tools/src/context.rs（495 行）：ChunkTextSource/LedgerTextSource/KbRelevance/anchor_chunk（KB 报告引用）。
- bid-tools/src/draft.rs（1265 行）：build_section_input/EvBrief（112-231）。
- bid-domain/src/lib.rs（1721 行）：restore 聚合不变量（1293-1362）、set/replace_requirements（846-975）、scoring_mappings 持久化面。
- bid-parse/src/extract.rs：params.section_title 产出面（784-825）。

---

## 1. #224 第 2/3/4 步现状核查（本轮核心问题）

### 1.1 第 2 步（重复计分检测 + 双向追溯）

**重复计分检测（Σ weight_share ≤ 1.0 聚合不变量）——域层已实现但检查窗口与消费窗口分裂：**

- 登记面：`upsert_scoring_mapping`（scoring.rs:1387-1417）按「联结后有效份额」校验——口径写明计入派生 params.section 单值（share=1.0）、排除被替换行、同节去重、1e-9 容差。restore 面（lib.rs:1319-1362）同源双侧执行。域测试 `aggregate_share_rejects_overshoot_on_insert/update`、`restore_rejects_oversized_aggregate_shares`、`derived_section_mapping_counts_into_sum` 数值断言充分。
- **缺陷（DOM-001）**：两侧校验读取的都是**原始** `req.params.get("section")`（scoring.rs:1395、lib.rs:1338）。而真实消费面 MCP `build_scorer`（tools.rs:2919-2931）在派生前对 `params.section` 为 None 的评分项做 #170 联结——`params.section_title`（extract.rs:812 对含「第X章」引用的评审表行登记）→ `chapter_leaf` → 补出一条 share=1.0 的派生映射。于是：REQ 原始 params.section=None、经 #170 联结生效派生映射（1.0）+ 账本异节行（≤1.0）→ upsert 校验只见 0+share ≤ 1.0 放行；score_document 消费面有效份额合计 2.0 → **同一评分项跨章双计**——恰是该不变量要防的形态。生产可达性当前受 PB-006（账本无生产写入口）压制，但不变量本身（#224 第 2 步交付物）在唯一活跃消费面不成立。
- **weight_share 消费真实性**：score_section 按 share 摊分（scoring.rs:1056-1059：max=max_full×share、estimated 同比，rationale 带「摊分N%」）；score_document 逐章求和；MCP by_item.estimated=Σ逐章贡献（tools.rs:2663-2664）+ contributions 逐章明细 + coverage 份额加权均值——消费链真实（在 share 数据真实可达的前提下）。

**双向追溯**：MCP score_document 出参 mappings 报告（tools.rs:2536-2615：item→sections[]（section_id/title/weight_share/source/source_anchor/evidence_refs），账本映射归因只认生效键集 ledger_effective、RuleParsed 锚回 REQ 首锚）；by_item 带 mapping_ids + 旧键 section_id=首映射。域层账本 SM-id/复合键/快照水位/引用完整性守卫齐备。**结论：第 2 步在域层+MCP 读面成立；唯一缺口是 DOM-001 的检查窗口分裂与 PB-006 的写入口缺失。**

### 1.2 第 3 步（score_rules 消费）——未做（如实自认）

- `ScoringItem.score_rules`（scoring.rs:73-76）与 `ItemScore.score_rules`（781-784）只透传；`assess()`（986-1005）只做关键词覆盖/字数/证据/具体性四维，**不读 score_rules**；score_document notes 显式登记 `SCORE_RULES_RESERVED`（1179-1183）。内联测试 `score_rules_reserved_surface_wired_but_not_consumed`（scoring_b2_self_eval.rs:204-246）用「同文同分」强验证判档不消费——自认口径有测试钉死。
- **附带发现（DOM-006）**：`ScoringItem.tiers` / `parse_tiers` 同样**零生产消费**（grep 全仓：.tiers 无 bid-mcp/bid-tools 读者）。估分档位比例是硬编码四档中点（GradeTier::ratio 0.95/0.825/0.65/0.275，scoring.rs:719-722），不读招标文件自己的档位表（如「优 9-10」中点 0.95 恰合、「良 7-9」中点 0.8≠0.825）。
- 规则启发式 vs 真实评标的边界**诚实且外显**：MCP score_* 出参带 `diagnostic_only: true, result_kind: "rule_heuristic"`（tools.rs:2443/2721），#209 的语义降级标记保留。

### 1.3 第 4 步（评分项变更 → 审核失效）——未做

- `upsert_scoring_mapping` 真变更 bump content_version（scoring.rs:1435-1439/1452-1455，测试 mapping_mutation_bumps_content_version_but_noop_does_not 钉死）——但 content_version 是 CAS 面；#180 的 MatrixApproval/PreExportConfirm 门绑 `sections_digest`（outline.rs:621-640，六字段编码不含评分映射/评分项），映射变更不改 digest → 过门不失效。与 issue #224 分步计划、issue 状态一致（未做，非回归）。
- 域层另有 set/replace_requirements 与 set_outline 对映射账本的引用完整性 fail-loud 守卫（lib.rs:893-908/953-968、outline.rs:700-715）——防孤儿化快照卡死，测试齐备（scoring_mapping.rs:571-652）。这是第 4 步的「防炸库」半边，「审核失效」半边未做。

### 1.4 评分项解析/总和/不适用/重复计分（调查问题清单逐项）

- **权重**：params.score → weight=max_score（缺失→None 入 scoring_gaps，不伪装）。
- **档位**：parse_tiers 手写扫描（分号/逗号分隔 × 全角/波浪线 × 无「分」字形态）确定性解析，测试覆盖真实形态；但见 DOM-006（不消费）。
- **得分估算**：estimated = max_full × share × 档位中点，两位小数舍入（评标惯例）；rationale 带可复算算式（「10.00×0.950=9.50」）。
- **多章节映射贡献**：见 1.1（真实，含 MCP contributions）。
- **评分总和**：technical（主观叶）+ price（B1）+ commercial（客观叶，无客观叶→None 不伪装 0）；target（MCP）= 非硬门可达叶满分合计（诚实分母）；树病（子和失配/根合计≠100）如实呈现不截断（synth_tender 自评 103.17>100 有测试注释自认）。
- **不适用项**：价格类叶跳过内容自评（is_price_item 词面：价格分/投标报价/下浮率/基准价），rationale 登记「价格分=f(报价)」；未报价→NO_PRICE_INPUT note、price null（MCP）；UNMAPPED 价格叶不入（价格分不依赖章节映射）。矩阵面 NOT_APPLICABLE 走人工覆写（matrix.rs:287-293 必须给偏离说明）。
- **重复计分**：见 1.1/DOM-001；树内重复 id（DUPLICATE_ID）、环（CYCLE 零贡献停振）、孤儿不计根合计——validate_sums 四病+重复全检出有测试。
- **视图分裂**（PB-007，引用不重登）：`scoring_tree()`（scoring.rs:1322-1324）= 纯 params.section；`scoring_items()`（1303-1319）= params.section ∪ 账本；MCP build_scorer = （params.section ∪ #170 联结）∪ 账本——**三种联结语义并存**，其中 #170 联结差异是 PB-007 描述未覆盖的加深度（DOM-001 即源于此）。

---

## 2. pricing.rs（1887 行）——B1 价格分引擎

**正确性（正面）**：五方法（BenchmarkPrice/LowestPrice/AveragePrice/Composite/DiscountRate）× 三偏离规则（Price 不对称扣分/Ratio 87号令/Rate 百分点）的适配矩阵 validate 全枚举拒非法组合；check_price 拒 0/负/NaN/±inf/超控制价（87号令第六十三条兜底）；clamp [0,max]（亳州「负分按 0」同构）；「去尾」不存在——返回全精度 f64，舍入归展示层（doc 注释钉死口径）；最低价法「基准含己方、不得去掉最高最低」与 87号令§55 原文一致；optimal_price 各方法闭式解/候选集精确解，网格暴力搜索上界性测试双例（1000 点/280 点）。

**真实锚点 Oracle 独立性**：35 个内联测试，8 套真实规则（87号令/东南大学/玉带河/康居嘉苑/亳州导则）手工算式独立写死逐位对照（assert_close 1e-9）；退化等价锁定（budget_weight=1≡BenchmarkPrice、=0≡AveragePrice）；唯一性/浮点（同输入双算全等）。**未发现计算正确性缺陷。**

**边界缺口（DOM-003，P3）**：求和溢出面无防护——others 含两个 ≥1e308 的合法有限正数时 Σ=inf → benchmark=inf → deviation=(bid−inf)/inf=NaN → deduction/score=NaN，`clamp`（f64::clamp 对 NaN 返回 NaN）穿透 → `score()` 返回 Ok(NaN)，违反模块头「禁止用 NaN 伪装计算失败」纪律。触发需荒谬量级输入（万元口径报价 1e308），实害趋零；无测试覆盖。

---

## 3. outline.rs（1385 行）——章节树

- **身份**：SEC-xxxx store 单序列签发；validate_outline 拒重复 id；重开纲（补遗重推导）走同门。
- **九不变量**：空树/id 唯一/level 域 1..=4/每节点 ≥1 REQ/根 level=1/子=父+1/父链无环（hops>nodes.len() 拦）/order 全局唯一/父聚合（requirement_ids==子并集 && target_chars==子和）——outline_invariants.rs 每条一组正反验证 + 边界值（单子配额相等/恰 4 层/单根无子/20 亿配额不溢出）。**不变量实现与测试均扎实。**
- **建纲配额**：最大余数法 + MIN_LEAF_CHARS=500 下限 + 二次均衡（整轮零进展退出——#199 死循环修复有回归锁 quota_rebalance_no_infinite_loop_on_partial_floor_stall，构造 78 REQ/25 叶/下限和超 target 的停滞路径）；12,000 REQ/320 REQ 验收带测试；语义分组（「其他」类 chapter_of→infer_theme→兜底条款号区间三级归组）确定性双推导全等测试。
- **脏章节计算（补遗影响）**：OutlineNode.dirty 是「可叠加脏标记」（R-05），但 **DomainStore 内唯一写者是 reopen_section**（outline.rs:796-814，grep 验证）；补遗→条款变更→related 章打脏的传播链只存在于 impact.rs 平行引擎（见 §8/DOM-005）。set_outline 状态门 REQUIREMENTS_EXTRACTED/MATRIX_READY/EVIDENCE_BOUND；MCP build_outline 在 MATRIX_READY+ 被 guard_tool 拒（tools.rs:3615-3617 自认「重建面属后续需求」）。
- **#180 sections_digest**：编码即契约（六字段进导出产物面，显式排除调度/账面字段），存储序无关（按 (order,id) 排序）、None 与空串同喂入、status/dirty/attempt 变更不改摘要——四组对照测试钉死。

---

## 4. matrix.rs（639 行）——需求响应矩阵

- **派生 vs 持久**：matrix 是**持久化派生态**（build_matrix 全量重算落库，幂等无变化不 bump）；行 id=CMP-{REQ 数字后缀} 镜像派生（重建幂等零序列污染，无数字后缀回退 CMP-0000-{id} 保唯一）。
- **重建触发点（生产）**：prepare_outline_and_matrix（service.rs:827，MCP build_outline 转发）、bind_evidence_batch（service.rs:984）、rebuild_matrix_for_override（service.rs:750，#226 覆写后立刻重算）。**漂移窗口**：apply_validity 翻转证据为 Expired 后不触发重建 → 持久化行保持 RESPONDED → check_export_ready 放行（**PB-005，试点已登记，不重登**）；同库两口径（matrix_view 过滤死证据 vs build_matrix 全量绑定）矛盾面同属 PB-005。set_outline 重挂后矩阵不自动重算（response_section 陈旧）——但生产面 build_outline E_STATE 门 + replace_requirements 状态门（lib.rs:935-942 只允许 FilesImported/RequirementsExtracted，彼时矩阵尚不存在）结构性关闭该窗口，无独立缺陷。
- **blocking 语义**：`(is_mandatory(类别) ∪ agent_mandatory(params 标记)) && NOT_RESPONDED`；Agent 必选接线正反测试（含半标记/正则形状零位移）；matrix_view 只读零序列污染有测试。
- **PB-001（tier 不进 has_ev）引用不重登**。

---

## 5. forms.rs（886 行）+ party.rs（537 行）——格式件与主体信息

- 七件格式件纯函数渲染（同输入逐字节同输出）；Blank 全角方括号待填位纪律（「绝不用无/-// 占位」）；format_amount 对 ≤0/NaN/∞落待填位；0 工期/0 有效期落【待填】不落「0 日历天」假承诺——四处验收测试。生产接线：bid-tools/export.rs build_export_forms（404-468）消费七件 + amount_to_cn_upper 大写回填（数字确定性换写）。
- party.rs 校验：信用代码/身份证/银行账号硬格式 + 逐字段人话缺陷清单（含 JSON 字段键）+ 半填可存盘（完整性硬门在渲染侧）+ roundtrip/旧 JSON 加性兼容测试。signer_name 统一口径防「投标函签委托人、授权书写法定代表人」自相矛盾。
- **DOM-004（P3）**：`QualificationStatement.solvency` 字段文档倒置——doc 写「是否处于被责令停业、财产被接管/冻结/破产状态」，实现却是 `yes_no(!*solvency)`（forms.rs:463）即 solvency=true → 渲染「我方【否】处于被责令停业…」（正常经营声明，**输出正确**）。export.rs:431 警告文本「偿付能力/纳税社保两项按『是』出具」与 solvency 项实际渲染（否）不符。当前唯一调用方传 true 输出正确；按文档字面传值（true=处于停业）会产出**相反**的资格声明——潜伏误用面。无该句断言测试。

---

## 6. param.rs（799 行）+ budget.rs（529 行）+ assignment.rs（485 行）

- **param.rs**：偏离派生单一真源 derive_deviation（数值主判[千分位/紧邻「万」×10000/前后缀容忍] → 语义标记词表 → Incomparable 诚实态）；ParamInput 不含 deviation——写入面派生覆写防劫持；幂等键 (req_id, param_name) 原位覆盖保 id 稳定；幽灵 REQ/幽灵证据整批拒绝零残留；「数值大=优」默认口径在模块头与测试注释中诚实自认（方向敏感参数留 T-7 direction 覆写面——未做，测试只造相等行回避反例）。100 行/3 植入负偏离全检出验收。
- **budget.rs**：四级 tie-breaker（score 降序 total_cmp→金额单一性→文件类别→chunk_id）确定性；token 记账一致性校验（谎报字段拒）+ 约束区超红线硬错误 + overflow 裁决位（非失败）+ schema 版本门（未来版本拒载/缺省按当前版）；estimate_tokens 口径单测边界值齐。tests/budget.rs 黑盒验收四阶梯精确分界/确定性（50 条平局打乱入参序不变）/fail-closed/版本门全绿。**无新发现。**
- **assignment.rs**：target_id 自然键 upsert（同 owner 重批=改期不清进度；换 owner=状态复位 ASSIGNED——「换人=新进度轴」语义有测试）；TaskStatus 线性链越级拒零副作用；ghost SEC/REQ 整批拒；list_tasks 双过滤字典序确定性；500 节点批量验收。**无新发现。**

---

## 7. impact.rs（523 行）——影响面平行引擎

SEMUL（Python v2_impact.py）的 Rust 双实现：R-mod/R-date/R-conf/R-idem + R2 全项目重检精确翻转 + ACCEPTED 门禁（Dirty/未裁决冲突关联章拒）+ adjudicate 补遗优先终裁。**全仓唯一消费方是 bid-domain/tests/four_contracts.rs（契约测试）——grep 验证：bid-tools/bid-mcp 零引用。** 它与 DomainStore 是两套平行状态语义（BindingStatus vs EvidenceStatus、Section{status,dirty,related} vs OutlineNode），无互操作。试点已注明此架构形态；本轮从「脏章节计算（补遗影响）」问题角度登记 DOM-005。

---

## 8. 问题记录

```
ID: DOM-001
严重程度: P2
模块: bid-domain + bid-mcp
文件路径: bidcore/crates/bid-domain/src/scoring.rs；bidcore/crates/bid-domain/src/lib.rs；bidcore/crates/bid-mcp/src/tools.rs
代码行号: scoring.rs:1395-1408（upsert 聚合校验读 raw params.section）；lib.rs:1338（restore 同源）；tools.rs:2919-2931（build_scorer 的 #170 联结补派生映射）
函数/结构体/接口: DomainStore::upsert_scoring_mapping / DomainStore::restore / build_scorer
问题描述: #224 第 2 步「重复计分检测」聚合不变量的检查窗口与消费窗口分裂：校验侧只认原始 params.section（None 则计 0），而消费侧 build_scorer 对 params.section=None 的评分项按 params.section_title（extract.rs:812 对评审表行「第X章」引用登记）→chapter_leaf 联结补出 share=1.0 派生映射。带 section_title 的评分项 + 账本异节行（≤1.0）→ upsert 校验放行，消费面有效份额合计 2.0，score_document 对同一评分项跨章双计——恰是该不变量设计要防的形态。
调用链: extract（params.section_title）→ upsert_scoring_mapping（raw section=None → 校验 0+share≤1.0 放行）→ build_scorer（#170 联结派生 1.0 ∪ 账本 share）→ score_section 逐条摊分 ×2 → score_document 跨章求和双计
触发条件: ①账本写入口接通（当前 PB-006 压制）；②评分项带 params.section_title 且章号可归一到正文叶；③对该项异节入账
实际影响: #224 聚合不变量在其唯一活跃消费面（MCP build_scorer）不成立；接通后立即出现单项贡献超满分的双计。
复现步骤: 域层构造：REQ(评分项, params={section_title:"第X章…"}) + outline 含该章正文叶 → upsert_scoring_mapping(REQ, 异节SEC-Y, 1.0)（通过）→ MCP score_document → by_item.estimated ≈ 2×max
当前测试: scoring_mapping.rs 全部用 params.section 形态（无 section_title 用例）——测试绿≠不变量在消费面成立
缺失测试: 「section_title 联结派生映射 + 账本异节行」组合的 upsert 拒绝/放行语义测试；build_scorer 联结面与 upsert 校验面的等价性测试
建议方案: 聚合校验复用 build_scorer 同款联结（含 #170 join 与去重规则）计算「联结后有效份额」——或把联结下沉到域层单一构造点（scoring_items/scoring_tree/build_scorer 同源），校验与消费共享同一视图（兼治 PB-007）
证据: scoring.rs:1395-1396
  let derived_sec: Option<&str> = req.params.get("section").and_then(|v| v.as_str());
  let mut resulting = if derived_sec.is_some() { 1.0 } else { 0.0 };
tools.rs:2924-2928（消费侧联结——校验侧未见）
  } else if let Some(title) = r.params.get("section_title").and_then(|v| v.as_str()) {
      if let Some(no) = chapter_no_of(title) {
          if let Some(sec) = chapter_leaf.get(&no) {
              r.params.insert("section".into(), serde_json::json!(sec));
审计置信度: CONFIRMED
状态分类: 当前仍存在（生产影响条件性于 PB-006 接通）
```

```
ID: DOM-002
严重程度: P3
模块: bid-domain
文件路径: bidcore/crates/bid-domain/src/scoring.rs
代码行号: 1390-1417（聚合校验含派生 1.0 恒计）；407-453（mappings_from_params share 恒 1.0）
函数/结构体/接口: upsert_scoring_mapping / mappings_from_params
问题描述: 派生 params.section 映射不可降级/覆写：带派生映射的评分项，任何异节账本行恒被拒（1.0+share>1.0），报错提示「调整既有占比请先降再增」——但派生映射来自抽取层 params，无任何 API 可移除/降权（账本同节行被去重为 inert 不生效）。多对多（#224 第 3 项/#170 根因）对 RuleParsed 已映射项结构性不可达；派生映射份额永远 1.0。
触发条件: 对带 params.section 的评分项尝试建立第二章节映射
实际影响: #224 多对多只能覆盖「无派生映射」的评分项；AgentSubmitted/Manual 无法纠正抽取层单映射的份额
当前测试: scoring_mapping.rs:519-565 derived_section_mapping_counts_into_sum 把该行为钉死为预期（拒+零副作用）
缺失测试: 不适用（缺的是覆写面不是测试）
建议方案: 接 MCP 写面时提供「派生映射降权/失效」显式操作（如账本同节行 share 生效、派生行让位的反转语义——现联结去重注释已预留「同章重合以账本 share 为准的语义留给后续显式覆写面」）
证据: scoring.rs:1410-1415
  if resulting > 1.0 + 1e-9 {
      return Err(bad(format!(
          "{scoring_req_id} 映射份额合计将达 {resulting:.3}（合法 ≤ 1.0）——… 调整既有占比请先降再增"
审计置信度: CONFIRMED
状态分类: 当前仍存在（代码注释自认的既知设计边界）
```

```
ID: DOM-003
严重程度: P3
模块: bid-domain
文件路径: bidcore/crates/bid-domain/src/pricing.rs
代码行号: 436-439（AveragePrice 求和）、289-291（clamp_score）、538-546（deviation/deduction）
函数/结构体/接口: PriceScoreSpec::benchmark / clamp_score / score_detail
问题描述: others 求和溢出无防护：两条 ≥1e308 的合法有限正报价 Σ=inf → benchmark=inf → deviation=(bid−inf)/inf=NaN → deduction/score=NaN；f64::clamp 对 NaN 返回 NaN → score() 返回 Ok(NaN)——违反模块头「禁止用 NaN/0 伪装计算失败」的诚实错误纪律（应为 E_UNPROCESSABLE）。
触发条件: others 或（Composite）被保留报价集合求和溢出 f64（万元口径需 1e308 量级——荒谬输入；check_price 只拦非有限单体，不拦求和溢出）
实际影响: 极低（输入量级不现实）；纪律面破损——NaN 可流向 score_document 的 price/total 并穿透两位小数舍入
当前测试: validation_rejects_bad_specs_and_prices 只测单体 NaN/0/负/超控制价，无求和溢出用例
缺失测试: 溢出输入 → score() 应 Err 的负测试
建议方案: benchmark/求和结果 is_finite() 校验，非有限 → E_UNPROCESSABLE（PRICE_OVERFLOW）
证据: pricing.rs:437-439
  let n = (others.len() + 1) as f64;
  let sum: f64 = others.iter().sum::<f64>() + bid_price;
  Ok(sum / n * coefficient)
审计置信度: CONFIRMED（IEEE-754 语义推演；未运行）
状态分类: 尚无法复现（需荒谬量级输入；逻辑确定）
```

```
ID: DOM-004
严重程度: P3
模块: bid-domain + bid-tools
文件路径: bidcore/crates/bid-domain/src/forms.rs；bidcore/crates/bid-tools/src/export.rs
代码行号: forms.rs:151-153（solvency 字段 doc）、463（渲染 yes_no(!*solvency)）；export.rs:431-433（警告文本）
函数/结构体/接口: StandardForm::QualificationStatement / build_export_forms
问题描述: solvency 字段文档倒置：doc 写「是否处于被责令停业、财产被接管/冻结/破产状态」，实现是 yes_no(!solvency)——solvency=true 渲染「我方【否】处于被责令停业…」（正常经营声明，输出正确）。export.rs 警告「偿付能力/纳税社保两项按『是』出具」与 solvency 项实际渲染（否）不符。按文档字面传值（true=处于停业）会产出相反的资格声明——潜伏误用陷阱。
触发条件: 未来调用方按字段 doc 字面语义传 solvency=true 表示「处于停业」
实际影响: 当前唯一调用链（export.rs:455 传 true）输出正确；文档/警告文本误导后续调用方
当前测试: key_clauses_and_format_requirements_are_present 断言注册资本/资质，未断言 solvency 句文本
缺失测试: solvency 两态的渲染文本断言（「否」处于停业/「是」处于停业）
建议方案: 字段改名（solvency_ok / normal_operation）或修 doc；export 警告改为「偿付能力项按正常经营出具（渲染『否』处于停业状态）」
证据: forms.rs:151-153 vs 463
  /// 是否处于被责令停业、财产被接管/冻结/破产状态。
  pub solvency: bool,
  …
  solvency_text = yes_no(!*solvency),
审计置信度: CONFIRMED
状态分类: 文档与实现不一致（输出本身正确）
```

```
ID: DOM-005
严重程度: P2
模块: bid-domain
文件路径: bidcore/crates/bid-domain/src/outline.rs；bidcore/crates/bid-domain/src/impact.rs
代码行号: outline.rs:91-93（dirty 字段 doc「上游 content_version 变化时打标，R-05」）、796-814（唯一写者 reopen_section）；impact.rs:423-435（related∩受影响 REQ 打脏——平行引擎内）
函数/结构体/接口: OutlineNode.dirty / ImpactEngine::import_events / DomainStore::reopen_section
问题描述: R-05 脏章节影响面在 DomainStore 生产链路不存在：dirty 字段在（含持久化 serde default），但域内唯一写者是人工 reopen_section；「补遗导入→条款修改→related 章节打脏」的传播只实现在 impact.rs 平行引擎（BindingStatus/Section 独立结构），其全仓唯一消费方是 four_contracts.rs 契约测试。生产补遗链（import→re-extract→replace_requirements）在 MATRIX_READY+ 被 E_STATE 拦，MCP build_outline 同拦——「重开纲需 S-5 影响面机制（后续批次）」注释自认未做。
调用链: （无生产调用链——能力缺失本身即发现）
触发条件: 补遗到达时项目已 DRAFTING+（当前被状态机整体拦死，连带正常重开纲也被拦）
实际影响: 补遗增量响应（DIRTY 重检/related 章失效/ACCEPTED 门禁）在生产不可达；DomainStore.dirty 对 readiness/导出/审核门零消费（grep 验证）
当前测试: four_contracts.rs（SEMUL 交叉验证）测的是平行引擎；outline.rs 只测 reopen 打脏
缺失测试: 不适用（缺接线非缺测试；与 PB-006 同族）
建议方案: 把 impact.rs 的 related-传播语义并入 DomainStore（条款 params 合并/有效性翻转时按章节 requirement_ids 反查打脏），或接 S-5 批次时明确退役平行引擎
证据: outline.rs:91-93
  /// 脏标记（可叠加——上游变更打标，非状态机成员；R-05 影响面分析的输入）
  #[serde(default)]
  pub dirty: bool,
impact.rs:430-433（平行引擎内的同类逻辑——生产 store 无）
  for (sec, v) in &mut self.sections {
      if v.related.iter().any(|g| affected.contains(g)) {
          v.dirty = true;
审计置信度: CONFIRMED
状态分类: 当前仍存在（issue 既知未做——set_outline 注释明示 S-5 后续批次）
```

```
ID: DOM-006
严重程度: P3
模块: bid-domain
文件路径: bidcore/crates/bid-domain/src/scoring.rs
代码行号: 29-41（ScoreTier 结构）、163-241（parse_tiers 手写解析）、703-734（GradeTier 硬编码中点）
函数/结构体/接口: ScoreTier / parse_tiers / GradeTier::ratio
问题描述: ScoringItem.tiers 与 parse_tiers 零生产消费（grep 全仓：bid-mcp/bid-tools 无 .tiers 读者）：评分细则的档位表被解析入库但不进判档——估分档位比例用硬编码四档中点（0.95/0.825/0.65/0.275），不读招标文件自身档位区间（如「良 7-9 分」中点 0.8≠0.825）。属 #224 第 3 步（score_rules/细则消费）未做的同族数据面闲置。
触发条件: 无需触发——档位数据已在树/出参中透出但无裁决面消费
实际影响: 自评档位与真实评标档位口径脱钩（已有 diagnostic_only 降级标记兜底，不伪装评标）
当前测试: parse_tiers_real_forms 只测解析正确性，无消费面测试
缺失测试: （若接入）tiers 区间中点 vs GradeTier.ratio 的一致性或映射测试
建议方案: #224 第 3 步接入时 assess 优先读 item.tiers 派生档位区间中点，缺省回退硬编码四档
证据: scoring.rs:719-722
  GradeTier::Excellent => 0.95,
  GradeTier::Good => 0.825,
  GradeTier::Fair => 0.65,
  GradeTier::Poor => 0.275,
审计置信度: CONFIRMED
状态分类: 当前仍存在（issue 既知未做，#186/#224 第 3 步范围）
```

**引用试点（不重复登记）**：PB-001（matrix has_ev 不查 verification_tier）、PB-005（apply_validity 后矩阵陈旧/双口径）、PB-006（upsert_scoring_mapping 无生产调用方——DOM-001 的前置条件）、PB-007（scoring_tree/draft_section_iterative vs build_scorer 视图分裂）、PB-008（set_requirements id 唯一性）、PB-011（dates parse_ymd 口径债）、PB-013（evidence 语义位）。

---

## 9. 测试有效性评估（4 个声明范围测试文件）

| 测试文件 | 类型 | 独立 Oracle | 错误路径可达 | 评注 |
|---|---|---|---|---|
| tests/scoring_mapping.rs（891 行） | UNIT+CONTRACT | ✅ 摊分期望 12/8、聚合 1.4 拒、restore 双侧拒——数值断言非形态断言 | ✅ 越界/孤儿/幽灵 EV/复合键重复/更新路径排除被替行/水位 | #224 第 1 步域层充分；**全部直调域 API**——PB-006/DOM-001 的消费面缺口不被暴露（无 section_title 组合用例） |
| tests/budget.rs（322 行） | UNIT（黑盒 pub API） | ✅ 四阶梯分界的手工 token 账（155/672/827/739…）独立推算 | ✅ 重复 id/空 section id/谎报 token/未来版本全拒不 panic | 高质量：50 条平局打乱入参序不变 + 版本门 roundtrip |
| tests/scoring_b2_self_eval.rs（246 行） | UNIT/INTEGRATION | ✅ 手工总分 40.75/79.7/38.95 与引擎 <10% 对照；「同文同分」强验证判档不消费细则 | ✅ UNMAPPED/SECTION_MISSING 不误报、无细则不登记 | 用 scoring_tree() 直建——**复现 PB-007 视图**（不走 build_scorer 联结），测试绿掩盖联结面差异 |
| tests/outline_invariants.rs（199 行） | UNIT/NEGATIVE 矩阵 | ✅ 每条不变量注入一处违规断言 Err+关键锚 | ✅ 九规则+边界值（20 亿配额不溢出） | 与 outline.rs 内联 set_outline_rejects_broken_trees 互补（后者走 store 含状态门/幽灵 REQ） |

内联测试（scoring.rs/pricing.rs/outline.rs/forms.rs/param.rs/party.rs/budget.rs/matrix.rs/assignment.rs）：均为行为断言（数值/错误码/零副作用/幂等/确定性双跑）；pricing 真实规则手工对照 8 套独立写死；synth-tender 树病（子和 8≠10、根合计 108≠100）如实断言不硬套。

---

## 10. 结论速览

- **#224 第 2/3/4 步**：第 2 步域层+MCP 读面真实（摊分/贡献/追溯/双侧完整性校验），但聚合不变量的检查窗口与消费窗口分裂（DOM-001）+ 写入口无生产调用方（PB-006）；第 3 步未做（score_rules/tiers 均零消费，DOM-006 同族）；第 4 步未做（content_version bump 只是 CAS 面，sections_digest 不含评分面）。
- **pricing**：计算正确性未发现缺陷（35 测试/8 真实规则/网格上界性），唯一缺口是求和溢出 NaN 穿透（DOM-003，P3）。
- **outline**：九不变量+配额（#199 回归锁）扎实；R-05 脏章节生产链路缺失（DOM-005）。
- **matrix**：持久化派生态漂移窗口＝apply_validity 后不重建（PB-005）；其余窗口被状态门结构性关闭。
- 其余（forms/party/param/budget/assignment）：不变量/身份/序列化对称/非法态拒绝全部成立；唯一文档倒置陷阱在 forms.solvency（DOM-004）。
