# P3 矩阵细则（build_outline → get_compliance_matrix）

> 契约依据：`docs/bid-mcp-contract.md` §10（建纲）/ §4（矩阵只读）。本阶段
> 两个动作：`build_outline` 一次性完成「大纲树推导 + 矩阵首建」（推进
> MATRIX_READY——draft 的硬前置）；之后 `get_compliance_matrix` 只读查询、
> 随时重查。Agent 不算矩阵——矩阵行是引擎从（条款 × 绑定 × 章节 × 覆写）
> 派生的权威账本，你只读它、复述它、按 blocking 行驱动 P4。

## 调用形

```json
{ "project_id": "…", "target_total_chars": 844000, "max_reqs_per_leaf": 8,
  "project_version": 12 }
```

- `target_total_chars`：总目标字数（1,000..=5,000,000，缺省 844,000 ≈
  1000 页 × 844 字符/页——与引擎单一真源 `DEFAULT_TARGET_TOTAL_CHARS`
  同源，#253）——长标书按实测篇幅给大值（见主 Skill「篇幅纪律」）；
- `max_reqs_per_leaf`：每叶挂载条款上限（1..=50，缺省 8）；
- 状态窗口：**REQUIREMENTS_EXTRACTED 态可调**（FilesImported/Draft →
  E_STATE；MATRIX_READY+ 重复调用 → E_STATE——重建面属「补遗后增量重抽」
  的后续需求，不隐式放行）；
- 前置：条款面非空（空纲无意义——先 P2）。

```json
→ { "counts": { "sections": 18, "leaves": 9, "requirements": 108,
     "total_target_chars": 20000, "matrix_rows": 108, "matrix_blocking": 47 },
   "status": "MATRIX_READY", "project_version": 14 }
```

**必须向用户复述**：`建纲 N 节 / M 叶 / 矩阵 R 行 / 阻断 b 行`。
`matrix_blocking` 是 P4 的工作量预告——资格/否决（以及未绑证据的技术参数）
行还没绑证据时全部计入。

## 矩阵行结构（get_compliance_matrix 返回）

```json
{ "project_id": "…", "filters": { "category": "qualification" },
  "cursor": "<opaque>", "limit": 100 }
```

```
rows: [ { cmp_id: "CMP-0001",
          requirement: { id, clause_no, category, category_key, statement,
                         mandatory, source_anchor{file,kind,locator,excerpt}, params },
          status: "NOT_RESPONDED|RESPONDED|PARTIAL|NOT_APPLICABLE",
          evidence: [ { ev_id, title, status, valid_until } ],
          blocking: true|false, draft_section_id: "SEC-…|null",
          response_policy: "default", gap: null } ]
total / limit / cursor / next_cursor / counts{total, returned, page, page_size}
```

- 行序 = 条款位置序（CMP-0001..N，确定性稳定）；先过滤后分页（`total`
  恒为过滤后总数）；
- 游标纪律：`limit` 1..=1000、缺省 100，越界/非整数**显式拒**；游标与
  filters+project_id 绑定，换过滤器必须换游标（复用 → E_UNPROCESSABLE）；
  `next_cursor: null` 即取完；`page` 是兼容旧调用的信息性回显，新调用走
  cursor；
- 只读零副作用：任何阶段可调，不推状态不 bump 版本；
- **EXPIRED/SUPERSEDED 证据不计入响应面**（派生时即被过滤）——行显示的
  evidence 列全是活证据，别再拿过期证据安慰自己「绑过了」。

## REQ 类别（category 七类，双形态口径）

| 契约键 | 中文 | mandatory | 矩阵面关注点 |
|---|---|---|---|
| `qualification` | 资格条件 | ✅ | 资格判断主对象；不写规则原文（合并进【资格与响应】章节组） |
| `veto` | 否决项 | ✅ | 逐条必须响应；缺响应 = blocking |
| `scoring` | 评分项 | — | 完全不进正文（只进自评面 score_*） |
| `tech_param` | 技术参数 | — | P4 必须证据佐证；进技术参数偏离表 |
| `commercial` | 商务要求 | — | P6 确定性校验主对象（报价/工期/付款/质保） |
| `format` | 格式要求 | — | 七格式件对齐参考 |
| `other` | 其他 | — | 大纲二次分组 |

`mandatory` 与 blocking 判定同源：`category ∈ {资格条件, 否决项}`。
过滤值给契约键或中文类别名均可（双形态）。

## 响应四态与 blocking 判定

| status | 派生条件（引擎，单一真源） | 含义 |
|---|---|---|
| `RESPONDED` | 显式 override 标记，或「证据在绑 + 响应章节在位」 | 已响应 |
| `PARTIAL` | 有证据绑定但响应章节缺位 | 材料齐、正文未落（P5 的活） |
| `NOT_RESPONDED` | 无证据无章节 | 未响应——**必选类即 blocking** |
| `NOT_APPLICABLE` | 显式 override（必须带偏离说明——不适用不给理由 = 不可审计的豁免） | 不适用 |

**blocking = `mandatory && NOT_RESPONDED`**（资格条件/否决项未响应）。
评分/技术参数/商务等非必选类未完成**不阻断导出**——但 P6 的确定性规则
（E_VETO_UNRESPONDED / E_QUAL_UNMET 等）会在校验面另算总账，别把「矩阵
不拦」当「没风险」。

矩阵行是**派生态**：requirements / bindings / sections 任一变化后重算
（幂等）。P4 绑定后即时重算，`bind_evidence` 返回的 `matrix.blocking`
与下次 `get_compliance_matrix` 查询同一真源。

## 资格结论（Agent 侧判断面——引擎无此字段）

阶段表要求「资格三态结论（eligible/conditional/not_eligible）向用户报告；
not_eligible → 停止流程，只产审查报告」。**如实口径**：引擎矩阵没有
policy/资格三态字段——`get_compliance_matrix` 行内 `response_policy` 恒为
`"default"`，三态是**编排层从 qualification 行推导的判断纪律**（设计来源
`docs/bid-skill-design.md`；`docs/threat-model.md` 将引擎侧 policy 终判标注
为 DESIGN_ASSUMPTION（M2））。落地做法：

1. 逐页读 `filters.category="qualification"` 的行（游标翻完，别只看首页）；
2. 逐行核对 `evidence[]`（在绑活证据的数量与 `valid_until`）与 `status`；
3. 汇总向用户报告：哪些资格条件已满足（RESPONDED + 证据有效）、哪些缺料
   （NOT_RESPONDED/PARTIAL——进 P4 补料清单）、哪些**确定无法满足**
   （如证据已 EXPIRED 且无替代来源、硬性门槛不达标）；
4. 存在确定无法满足的资格条件 → 按 SKILL.md 纪律**停止流程、只产审查
   报告**——不要伪装满足（P6 的 E_QUAL_UNMET 会拦，但废标风险在提交面，
   不在校验面）。

「已查未发现 / 尚未检查 / 用户补充 / 原文证据 / 模型归纳」五类陈述纪律
在这里同样适用：资格结论必须能指回矩阵行与 EV-id。

## blocking 行排查路径（Agent 决策表）

| 行情 | 动作 |
|---|---|
| 资格/否决行无证据（`evidence: []`） | 进 P4：register_evidence + bind_evidence——绑后重算，blocking 自然清零 |
| 证据已 EXPIRED / SUPERSEDED | 走补料流程（换证重传）——**禁绑过期待换证证据**；EXPIRING 可绑但提示换证风险 |
| 条款确不适用（如仅对联合体一方的要求） | 需要 NOT_APPLICABLE override（必须给偏离说明）——**截至本版 main 无该 MCP 工具**（域层 `set_matrix_override` 存在、MCP 面未接线）：如实告知用户该行无法覆写，按缺证据路径处理或等待工具面补齐 |
| 技术参数行未佐证 | 不 blocking，但 P5 起草会被 readiness 闸拦（技术参数无佐证拒生成）——P4 一并绑 |

## 常见错误

| 错误 | 触发 | 动作 |
|---|---|---|
| `E_STATE` | 非 REQUIREMENTS_EXTRACTED 态调 build_outline | 回阶段表核对当前状态（get_bid_project_state） |
| `E_UNPROCESSABLE` | limit 越界 / 游标与过滤器不匹配 / page=0 | 按报文修正参数重试（游标 fail-closed，不静默 clamp） |
| `E_UNPROCESSABLE` | build_outline 条款集为空 | 先 P2 extract_requirements |

## 来源锚点

- `docs/bid-mcp-contract.md` §10（build_outline 入出参/状态窗，L375-393）、
  §4（get_compliance_matrix 行结构/游标，L74-88）、§0（大结果分页约定，L13-16）
- `bidcore/crates/bid-mcp/src/lib.rs` L50-72（TOOL_NAMES 21 个——无矩阵
  覆写工具）
- `bidcore/crates/bid-mcp/src/tools.rs` L3323-3393（build_outline 壳层）、
  L816-931（get_compliance_matrix：双形态过滤/游标/行组装，
  `response_policy: "default"` 在 L901）、L590-600（category_key 七类）
- `bidcore/crates/bid-mcp/src/schema.rs` L176-192（矩阵入参 schema）
- `bidcore/crates/bid-domain/src/matrix.rs` L6-14（四态派生 + blocking 语义）、
  L30-46（ResponseStatus）、L77-79（is_mandatory）、L128-155（行派生）、
  L212-237（set_matrix_override——域层面，未接 MCP）、L243-266
  （check_export_ready）、L268-279（matrix_view 活证据过滤）
- `bidcore/crates/bid-tools/src/service.rs` L729-737
  （prepare_outline_and_matrix：建纲 + 矩阵首建 + 落盘）
- `docs/agent-integration.md` §5 P3 段（资格结论逐页读 qualification 行）
- `docs/bid-skill-design.md` 阶段表 P3 行（资格三态——设计层概念）；
  `docs/threat-model.md` §3（矩阵 policy 终判 = DESIGN_ASSUMPTION（M2））
- `sdk/skill/bid-response-pipeline/scripts/checklist.py` L82-88
  （P3 机器判据：status=MATRIX_READY、leaves>0）
