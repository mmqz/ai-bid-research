# P2 抽取细则（Agent 标记优先 + 正则兜底）

> 契约依据：仓库根 `docs/bid-mcp-contract.md`（路径约定见 SKILL.md 头部——`docs/` 指仓库根，非本目录相对） §3 / §17 / §18 / §19。#219 二步起
> 抽取是**两通路合并**：Agent 块标记优先（被标记块的正则产物让位），
> 正则兜底（未覆盖块保留），冲突以 Agent 为准。无标记时与纯正则逐位一致
> （GT 红线零位移）。重跑 extract 按整集替换重算——补标后重跑即生效。

## 编排（#219 二步起）

```
1. get_document_chunks(project_id, file_id, from, size≤200)
   → 逐窗口看块（block_idx/locator/text/cells + 已标记 verdict 回显）
2. submit_block_marks(project_id, file_id,
     marks:[{from,to,verdict,reason}],          ← 整集替换（非增量！翻页标注须带全量）
     requirements:[{block_idx,statement,category?,mandatory?,substantive?}])
   → verdict 四态：requirement（需响应）/ noise（无关）/
     rule（招标人规则说明——#226 防死锁核心）/ review_table（审查表）；
     requirements[].substantive=true 标★实质性条款（#276——见下方
     「表格行条款」专节，废标级标记面）
3. extract_requirements(project_id)  → 合并落库（响应带 merge 统计）
```

Agent 关键点转 `REQ-Axxxx`（params 携 statement/mandatory/block_idx/
marked_by=agent；溯源锚=块原文）。 REQ-A 后缀非纯数字 → CMP 派生 id 走
`CMP-0000-REQ-Axxxx` 回退分支，与正则 `CMP-000N` 零碰撞。

`scope` 可传 `file_ids[]` 只抽指定件（增量补遗面）。`granularity` 是保留位
**当前未实现**（传入被忽略）。

## 返回核对

```
counts:  { total, added(可为负——整集替换语义), by_category{…七类} }
merge:   { regex_total, regex_dropped_covered, agent_requirements,
           uncovered_blocks, coverage_complete }        ← #219 二步合并面
conflicts:        [ { a: "REQ-*", b: "REQ-*", reason } ]   ← 补遗与正文冲突双向登记
missing_sequence: [ { file_id, expected_clause_nos, found } ] ← 条款编号缺号
warnings:         [ …含 UNCOVERED_BLOCKS / AGENT_MANDATORY_OFFCATEGORY ]
```

**合并守恒**：counts.total = regex_total − regex_dropped_covered + agent_requirements。
`UNCOVERED_BLOCKS`：未覆盖块正则已兜底——Agent 补标后重跑更准。
`AGENT_MANDATORY_OFFCATEGORY`：Agent 标必选且类别不在资格/否决阻断面——
**已接线**：这些条款进入矩阵 blocking 面，未响应将拦起草与导出（重跑
build_matrix 后生效）；响应（绑证据）或人工 receipt 覆写（set_matrix_override）
后放行。向用户提示的口径：必选漏响应 = 废标风险，请逐条确认响应状态。

**必须向用户复述**：`共抽取 N 条（资格 q / 否决 v / 评分 s / 技术参数 t /
商务 c / 格式 f / 其他 o）`。

## 七类产物定义（by_category 语义）

| 类别 | 判定边界 | 去向 |
|---|---|---|
| `qualification` 资格条件 | 「投标人须具备/具有」句式 + 资质证书/注册资本/人员配置要求 | P3 资格三态判断；不进正文（合并进【资格与响应】章节组） |
| `veto` 否决项 | 「否则按无效标处理/视为废标」及评分办法里的★/实质性条款 | P3 逐条响应；**不写规则原文** |
| `scoring` 评分项 | 评分办法表内的分值项 | **完全不进正文**（只进自评面 score_*） |
| `tech_param` 技术参数 | 参数表/「须满足以下技术要求」 | P4 必须证据佐证（无佐证 draft 拒生成）；进技术偏离表 |
| `commercial` 商务条款 | 报价/工期/付款/质保 | 确定性校验主对象（P6 报价 vs 控制价、工期比对） |
| `format` 格式项 | 投标文件组成与格式要求 | 七格式件对齐；P6 必填格式项校验 |
| `other` 其他 | 以上不覆盖 | 大纲二次分组（章节→主题→兜底条款号区间，不裸「（第N块）」） |

## 条款类型判定边界（Agent 核对面）

- 一条 REQ 只归一类（引擎裁决，你复核边界件：如「★参数」归 veto 还是
  tech_param——以招标文件评分办法的★标注语义为准，向用户确认口径）；
- `conflicts` 非空 → **逐条向用户确认口径**（以补遗/最新澄清为准），确认
  结果记入对话，不改引擎数据（引擎已双向登记）；
- `missing_sequence` 非空 → 缺号必须解释（格式缺号常见于附件编号跳段；
  真缺号 = 解析遗漏，重试该文件）。

## 表格行条款（★商务要求表——废标级标记面，#278）

**为什么单列一节**：五份真实考题（#269）的★实质性条款全部住在**同一形态**
的商务要求表——表头 `序号｜参数性质(★)｜类型｜要求`（政府采购货物服务类
高频形态）。此前标记质量全靠代理自己悟，六场实测方差：5/5、3/5、4/5、
**1/5**、3/5——danqing 丢 4/5 直接威胁不废标（★一条不响应即响应无效）。
本节是专项教学：见到此形态，按下面的映射逐行标记，不许跳过。

### 列语义映射（一行 = 一条 REQ）

| 列 | 标记用法 |
|---|---|
| `参数性质` 带★ | `substantive: true`（★实质性条款——一条不响应即响应无效） |
| `类型` | 条款主题（如「服务地点」「验收要求」）——statement 命名参考，不单独成 REQ |
| `要求` | statement 正文（关键点陈述照抄要求列内容，不得改写语义） |
| `序号` | 行定位锚（跨页续行拼接依据） |

标记动作：该行块 verdict 标 `requirement`，`requirements[]` 对应提交
`{block_idx, statement: <要求列内容>, substantive: true}`。`category` 按
七类正常判（★商务行多为 `商务要求` / `技术参数`）——**不要因为带★就归
`否决项`**：★是实质性标记（substantive），与类别、必选性（mandatory）
两两正交，各按原文声明独立标记，不互相推断。

### 跨页续行的装配

商务表常跨页：某行「要求」单元格在第 N 页截断、第 N+1 页续完。拼接依据
= `序号` 列与列位——续行块首列为空或重复上一序号时，**归属上一行条款**
（拼成一条），不要拆成两条。`get_document_chunks` 的 cells 按列序回显，
跨页块先拼再判。

### ★样板句的边界（rule 不是 requirement）

表尾常见声明：「打"★"号条款为实质性条款，若有一条不响应或不满足则导致
响应无效」——这是**★语义声明（招标人规则说明）**：verdict 标 `rule`，
不产 REQ。两个方向的用法：

- 看到样板句 = 本表★行都是 substantive 的**信号**——回头逐行核标记；
- **版面归属**：声明管它紧贴的那张表（songling 实测：表尾声明紧跟商务表，
  同文档的技术要求表无★行——勿跨表扩大化，也不因技术表无声明漏标商务表）。

### 与引擎★检测的对账（P3 必做）

★标记有两条写入路径，在 `params.star` 汇流（#276 数据模型）：**标记面**
（你，`submit_block_marks.requirements[].substantive`）与**抽取面**（引擎
语义表★字符检测，#280 起接线）。两路取并集不冲突，但**不要假设引擎一定
检出**——非语义表形态、切块截断都可能漏，标记责任在你。

**验收动作**：`get_compliance_matrix` 的 `star_summary`——`total` 必须
与招标文件★行数一致（对着表尾声明的表逐行数）；`total=0` 而原文确有
★表 = 标记漏了，回 P2 补标重跑 `extract_requirements`。起草完成后
`not_responded > 0` = 废标风险在场（引擎 note 原文：一条不响应即响应
无效），逐条补响应后重查。

## 参数槽位表

技术参数类 REQ 在矩阵行的 `requirement.params` 里携带结构化槽位（参数名/
要求值/单位——引擎从参数表格抽取）。Agent 在 P5 起草技术响应时：

1. 逐槽位响应（**参数值不得编造**——有佐证给佐证，无佐证写「满足」并引用
   EV，或如实报缺口）；
2. 响应面值与槽位要求值逐项对应，P6 会做确定性比对；
3. 负偏离如实声明（技术偏离表★整行标红是引擎行为，不因回避而隐匿）。
