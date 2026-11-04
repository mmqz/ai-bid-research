# P8 导出细则（export_bid_document）

> 契约依据：`docs/bid-mcp-contract.md` §8。导出是全链的收口闸——引擎在
> 此做**双前置门**裁决（矩阵 blocking 清零 + 审核门全过），任一不过
> `E_UNPROCESSABLE` 响亮拒（人话理由）。你的职责：把报价决策/主体信息
> 给足、路径给对、产物 sha256 与对账数字向用户复述。

## 调用形

```json
{ "project_id": "…", "format": "docx",
  "output_path": "out/投标文件-技术标.docx",
  "profile": { "bid_amount": 15800000, "project_name": "…招标官方项目名",
               "duration_days": 180, "validity_days": 90,
               "deposit_amount": 300000, "legal_rep_gender": "男",
               "price_breakdown": [ { "item": "…", "unit": "项",
                                      "quantity": 1, "unit_price": 15800000 } ] },
  "project_version": 30 }
```

- `format`：`docx` | `pdf`（pdf 走 docx 链 + Word/LibreOffice headless 转换
  ——两者均未检测到 → `E_UNPROCESSABLE` 响亮拒，不静默降级 docx）；
- `output_path`：**工作区相对路径**（不得绝对路径/含 `..`——违反 →
  `E_UNPROCESSABLE`）。产物落 `<工作区>/<output_path>`，父目录自动创建；
  返回的 `artifact.url` 是解析后的绝对路径；
- `profile` 是**投标决策面**（报价/工期/有效期/保证金/分项报价/招标官方
  项目名）——进幂等键，**换报价重导 = 新导出**（不吃旧缓存）；
- `options.tracked_changes` / `options.watermark`：暂不支持——传 truthy 会
  告警后按普通模式导出（不静默吞）；
- 状态窗：Drafting+ 可调（含 EXPORTED——重复导出/换参数重导）。

## 前置检查（引擎双门，fail-closed）

1. **矩阵门**（`check_export_ready`）：blocking 行（必选未响应）未清零 →
   `E_UNPROCESSABLE` 逐行列出「`REQ-x（CMP-x）: 必选未响应——R-03/R-04
   闸门`」；矩阵未构建同样拒（无矩阵 = 未对账）；
2. **审核门**（`export_allowed`，单一真源）：
   - 三门均须有 Approved 记录（缺哪门点名哪门）；
   - **#180 审核内容绑定**：最后一条 `pre_export_confirm` Approved 的
     `reviewed_content_digest`（章节树摘要）≠ 当前 → `E_UNPROCESSABLE`
     「审核针对的内容已变更，请重新审核」——**过门后改章节正文/结构 =
     旧审核失效**，重过门重新绑定（详见 `p7-review.md`）。绑摘要不绑
     全局 content_version——过门后导入无关件、不改正文重复导出不受影响；
   - 全部 RevisionTask DONE（未关单逐个点名 RT-id）。

成功响应里的 `precheck: { blockers: 0, waived: [] }` 是通过后的回显——
**失败路径不进 data 面，直接 Err**（读 `error.what` 人话理由）。

## 六表七件对账（产物完整性）

导出自动注入**六张表 + 七件格式件**（#117/#122），返回逐张点名：

```
tables_written: 6   table_kinds: ["项目概况表", "技术参数偏离表", "报价明细表",
                                   "业绩一览表", "人员配置表", "设备清单表"]
forms_written:  7   form_titles: ["附件一、投标函", "附件二、法定代表人身份证明书",
                                   "附件三、授权委托书", "附件四、投标保证金退还申请",
                                   "附件五、资格声明", "附件六、无重大违法记录声明",
                                   "附件七、廉洁承诺书"]
```

| 表 | 生成条件（缺输入 → 缺席 + 响亮告警，不静默） |
|---|---|
| ① 项目概况表 | **恒生成**（REQ 参数 + 项目面，缺栏落【待填】） |
| ② 技术参数偏离表 | 有「技术参数」类 REQ（负偏离★整行标红；无技术参数 REQ → 缺席 + 告警） |
| ③ 报价明细表 | `profile.price_breakdown` 在场（分项合计 ≠ 总价 → 响亮告警——评标否决点） |
| ④ 业绩一览表 | 台账有可用（Active/Expiring）业绩证据 |
| ⑤ 人员配置表 / ⑥ 设备清单表 | 工作区 `bidder-profile.json` 的 personnel / equipment 非空 |

七件恒生成（字段从 BidderProfile 铺管，缺 → 【待填】+ 告警；标题 Heading2
样式段进 TOC——评委在目录里看得到七件在哪）。**对账纪律**：`tables_written`/
`forms_written` 与输入面互证——配置齐时应为 6/7；缺席项必伴有 warnings，
逐条向用户转述（缺 bidder-profile.json 时人员/设备表与格式件字段全部
【待填】——主体信息建档是投标方输入面，不编造）。

**必复述**：`产物 sha256 / 尺寸 / 六表 N 张 / 七件 M 件`，与预期比对。

## 幂等与缓存

幂等键 = `(project_id, content_version, format, dark_bid, output_path,
决策指纹)`——同键重调直返首次 artifact（`deduplicated: true`，零 IO）。
换报价/换路径/内容版本推进后 = 新导出。pdf 面另有转换幂等（staging docx
未变 → 不重转，产物字节不重写）。

## 暗标模式（profile.dark_bid）

`dark_bid: true` + `bidder_name` → 正文与章标题的公司名替换为「投标人」。
`bidder_name` 缺失/空白 → **响亮 warning：未做脱敏**（无名字可替——用户
漏配），补齐后重导。P6 已做暗标禁字扫描，导出面是第二道。

## 产物验证（引擎侧回读——你要知道它存在）

e2e 不信任工具返回值：`tests/export_readback.rs`（#172）把落盘 docx **解
包**用独立解析器回读——自数 `<w:tbl>` 表数（六表应全部成真表格，不是
markdown 残文本）、七件关键词、表行数下限、负偏离红 run、报价合计一致、
零 markdown 星号/占位符残留（#129 深断言）。你侧的对应动作：复述
sha256 + 对账数字；用户拿到产物后抽查【待填】残留（缺决策的字段引擎会
如实留白并告警——**留给人工补，不由 AI 编**）。

## 状态推进与恢复

- 单次 export 内部经 `Drafting→Validating→Reviewing→ReadyToExport→
  Exported` 逐态补链（REVIEWING 是瞬态，不出现在两次调用之间）；终态
  `EXPORTED`；
- 导出需要**持久化工作区**（纯内存态 → `E_UNPROCESSABLE`「无章内容可导」
  ——`BID_WS` 建服务态）；
- 恢复点：同参幂等直返；改稿/换报价 → 重走对应上游（改稿需重过
  pre_export_confirm 门）。

## 常见错误

| 错误 | 触发 | 动作 |
|---|---|---|
| `E_UNPROCESSABLE`（必选未响应） | 矩阵 blocking 未清零 | 回 P4 补绑定（见 `p3-matrix.md` 排查表） |
| `E_UNPROCESSABLE`（缺门/RT 未关/内容已变更） | 审核门三面任一不过 | 补过门 / 关 RT / 重过 pre_export_confirm |
| `E_UNPROCESSABLE`（output_path） | 绝对路径/含 `..` | 改工作区相对路径 |
| `E_UNPROCESSABLE`（PDF 环境） | Word/LibreOffice 均未装 | 装 Office/LO，或导 docx |
| `E_CONFLICT` | project_version 过期 | 重读状态拿新值重试 |

## 来源锚点

- `docs/bid-mcp-contract.md` §8（export 契约：入参/output_path/六表七件/
  #180 门禁/幂等键/状态，L225-273）
- `bidcore/crates/bid-mcp/src/tools.rs` L2945-3123（export_bid_document
  壳层：format 校验 L2989-3005、路径形状校验 L3006-3016、幂等键
  L3048-3062、出参组装 L3098-3120——precheck 注释「失败路径在 Err 面」
  在 L3117）
- `bidcore/crates/bid-mcp/src/schema.rs` L263-320 附近（export 入参
  schema：profile 决策字段/options 告警位）
- `bidcore/crates/bid-tools/src/service.rs` L1046-1218（export_document
  业务半边：双前置门 L1083-1086、bidder-profile 装载 L1102-1126、
  #189 resolve_write 围栏 L1145-1151、pdf staging 与转换 L1158-1213、
  推进 EXPORTED L1215-1217）
- `bidcore/crates/bid-tools/src/export.rs` L229-387（build_export_extras
  ——六表生成条件与告警）、L389-468（七件构造：决策缺项告警/大写金额
  换写）、L755-764（ExportReport：tables_written/table_kinds/
  forms_written/form_titles）、L1169-1257（渲染计数与七件锚定）
- `bidcore/crates/bid-tools/src/tables.rs` L338-346（BidTable::kind 六表名）
- `bidcore/crates/bid-domain/src/forms.rs` L61-179（StandardForm 七件）、
  L209-239（kind/appendix_no 附件一~七/title 带编号标题）
- `bidcore/crates/bid-domain/src/matrix.rs` L239-266（export_blockers/
  check_export_ready）、`bidcore/crates/bid-domain/src/review.rs`
  L585-643（export_allowed：三门 + #180 摘要 + 全 RT DONE）
- `bidcore/crates/bid-domain/src/lib.rs` L194-224（allowed_tools：export
  的状态窗 Drafting+）
- `bidcore/crates/bid-mcp/tests/export_readback.rs` L1-50（#172 独立回读
  纪律）、L618-622（真表格数 ≥6 断言）；
  `bidcore/crates/bid-mcp/tests/mcp_scaffold_tools.rs` L304-320
  （纯 MCP 全链 export → EXPORTED 终态）
