# P2 抽取细则（extract_requirements）

> 契约依据：`docs/bid-mcp-contract.md` §3。执行 = 确定性先行（条款编号序列
> 解析、表格结构化槽位抽取）→ 模型补全语义归类。按 dedupe_hash 幂等合并
> 重跑结果——补遗后增量重抽安全。

## 调用形

```json
{
  "project_id": "…",
  "scope": "all",
  "options": { "include_scoring_table": true }
}
```

`scope` 可传 `file_ids[]` 只抽指定件（增量补遗面）。`granularity` 是保留位
**当前未实现**（传入被忽略）。

## 返回核对

```
counts:  { total, by_category{ qualification, veto, scoring, tech_param,
                               commercial, format, other } }
conflicts:        [ { a: "REQ-*", b: "REQ-*", reason } ]   ← 补遗与正文冲突双向登记
missing_sequence: [ { file_id, expected_clause_nos, found } ] ← 条款编号缺号
warnings:         [ … ]
```

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
