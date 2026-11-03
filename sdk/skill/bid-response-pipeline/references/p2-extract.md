# P2 抽取细则（Agent 标记优先 + 正则兜底）

> 契约依据：`docs/bid-mcp-contract.md` §3 / §17 / §18 / §19。#219 二步起
> 抽取是**两通路合并**：Agent 块标记优先（被标记块的正则产物让位），
> 正则兜底（未覆盖块保留），冲突以 Agent 为准。无标记时与纯正则逐位一致
> （GT 红线零位移）。重跑 extract 按整集替换重算——补标后重跑即生效。

## 编排（#219 二步起）

```
1. get_document_chunks(project_id, file_id, from, size≤200)
   → 逐窗口看块（block_idx/locator/text/cells + 已标记 verdict 回显）
2. submit_block_marks(project_id, file_id,
     marks:[{from,to,verdict,reason}],          ← 整集替换（非增量！翻页标注须带全量）
     requirements:[{block_idx,statement,category?,mandatory?}])
   → verdict 四态：requirement（需响应）/ noise（无关）/
     rule（招标人规则说明——#226 防死锁核心）/ review_table（审查表）
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
warnings:         [ …含 UNCOVERED_BLOCKS / AGENT_MANDATORY_NOT_BLOCKING ]
```

**合并守恒**：counts.total = regex_total − regex_dropped_covered + agent_requirements。
`UNCOVERED_BLOCKS`：未覆盖块正则已兜底——Agent 补标后重跑更准。
`AGENT_MANDATORY_NOT_BLOCKING`：Agent 标必选但类别不在阻断面（资格条件/
否决项）——导出闸门暂不拦（is_mandatory 接线属后续 issue），必须向用户提示。

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

## 参数槽位表

技术参数类 REQ 在矩阵行的 `requirement.params` 里携带结构化槽位（参数名/
要求值/单位——引擎从参数表格抽取）。Agent 在 P5 起草技术响应时：

1. 逐槽位响应（**参数值不得编造**——有佐证给佐证，无佐证写「满足」并引用
   EV，或如实报缺口）；
2. 响应面值与槽位要求值逐项对应，P6 会做确定性比对；
3. 负偏离如实声明（技术偏离表★整行标红是引擎行为，不因回避而隐匿）。
