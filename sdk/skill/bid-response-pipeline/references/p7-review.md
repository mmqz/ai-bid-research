# P7 审校细则（record_review_gate + check_bid_quality 复核面）

> 契约依据：`docs/bid-mcp-contract.md` §12（人工门）/ §9（质量诊断）。
> 本阶段与 P6 互补：P6 是**引擎对文档**的确定性校验（规则表见
> `p6-validation.md`）；P7 是**人对全链**的复审门 + 你按发现面逐章收口。
> 人工门的核心纪律：**AI 操作者不得自审自过**——过门身份由宿主签发的
> receipt 验签保证，不是 Agent 自报。

## 四道门总览

| 门 | gate 值 | 语义 | 状态窗 | 门序前置 |
|---|---|---|---|---|
| 矩阵定稿门 | `matrix_approval` | 条款响应矩阵定稿——先定「响应什么」 | MATRIX_READY / EVIDENCE_BOUND / DRAFTING（VALIDATING+ 不再受理——门要在窗口内过） | 无 |
| 章节树定稿门 | `outline_approval` | 大纲定稿——再定「怎么组织」 | 非 Draft/FilesImported 早期态 | 矩阵门先 Approved |
| 导出前确认门 | `pre_export_confirm` | 付印前最后一道人眼 | 无状态门 | 前两门均 Approved |
| 无补遗确认门 | `addendum_waiver` | 单件招标文件（无补遗件）的人工出口——Approved = 确认本项目无补遗 | 无窗口限制（导入后任意时刻可记） | 无（与三门正交，不入导出门序） |

跳门/乱序记审 → `E_STATE` typed reject（零副作用）——这就是「绕过门」的
裁决面。`addendum_waiver` 只解 MissingAddendum 阻塞（单件无补遗场景）；
补遗件**在场**的不就绪件走 `record_addendum_waiver` 逐件确认（见
`p1-import.md`「补遗分级门」），waiver 不掩蔽解析失败件。

## 调用形（record_review_gate）

```json
{ "project_id": "…", "gate": "matrix_approval", "decision": "APPROVED",
  "receipt": "v1.张工.human.review_gate:matrix_approval.1790000000.6a3f…",
  "findings": [ { "severity": "BLOCKER", "code": "金额冲突",
                  "message": "控制价与限价表述矛盾", "section_id": "SEC-0003" } ],
  "project_version": 41 }
```

- `decision`：`APPROVED`（过门里程碑）/ `REJECTED`（拒审**过程记录**——
  不撤回既有 Approved 里程碑；新问题经新 RT 收紧导出面）；
- `findings` 可选，元素 `{ severity(BLOCKER|WARNING|INFO), code, message,
  section_id? }`——`code`/`message` 必填非空（空发现 = 不可审计的记录）；
  `section_id` 缺省 = 全局级发现（RT 锚根项目章 `PROJECT`）；
- **`REJECTED` 必须携带 ≥1 条 BLOCKER finding**（Warning 不构成拒审依据
  ——诚实状态）；
- 旧 `reviewer` 参数**显式拒**（参数面先于验签面——Agent 不得自述身份）。

## receipt 语义（#182——人工确认的身份载体）

线格式（六段，点分隔）：

```
v1.<actor_id>.<actor_type>.<scope>.<issued_at_unix_secs>.<hex_hmac_sha256>
```

- **宿主签发**：真人在宿主进程确认后，宿主用服务态密钥签发（持久化工作区
  落 `<工作区>/.mang/approval.key`；纯内存态内存生成不落盘）。**签发面
  不进 MCP 工具表——Agent 拿不到密钥**。你的职责是把「请人工确认某门」
  交给用户，由宿主界面完成确认并回填 receipt；
- **scope 门级绑定**：`review_gate:{gate}`——A 门的 receipt 过不了 B 门
  （`record_addendum_waiver` 的签域是 `addendum_waiver:{file_id}`，件级绑定）；
- **时效窗**：`issued_at ∈ [now-86400, now+60]`——24h 有效期防陈旧重放，
  +60s 容差吸收宿主/服务侧时钟微漂；
- **HMAC-SHA256 常量时间比对**——时序侧信道不泄漏签名前缀信息；
- 验签失败的分类码（随 `E_UNPROCESSABLE` 的 `what` 透出，前缀可解析）：
  `MALFORMED_RECEIPT`（结构非法）/ `NOT_HUMAN_ACTOR`（Agent/System 的
  receipt 不用于过审面）/ `EXPIRED_RECEIPT`（超 24h 陈旧重放）/
  `FUTURE_DATED_RECEIPT`（超 60s 容差的未来签发）/ `BAD_SIGNATURE`
  （HMAC 不符**或签域 scope 不匹配**——他域 receipt 与伪造同归）。

域层另有纵深：即使壳层漏验，`record_review` 也拦 `actor_type != Human`。

## 返回核对

```
receipt: { review_id: "REV-…", gate, decision,
           reviewer: "human:<actor_id>"（legacy 回显——派生值，非身份判据）,
           actor: { actor_id, actor_type: "human",
                    authentication_method: "hmac_receipt_v1" },
           finding_ids: ["FND-…"], spawned_rt_ids: ["RT-…"],
           reviewed_content_digest: "<sha256-hex>"（#180 内容绑定摘要） }
gates_approved: ["matrix_approval", …]（域层 gate_approved 同一真源——
                 含 addendum_waiver 门）
status / project_version
```

**必须向用户复述**：过门结果与 `gates_approved` 进度（三门缺一不可导出）。
记审是写操作——`project_version` 步进，传给下一个工具。

## 过门 findings 与 RevisionTask 闭环

- **BLOCKER 强制派生 RT**（结构保证非约定保证）：每条 Blocker finding
  自动创建修订工单（owner 空、status=Open、锚 `section_id` 或 `PROJECT`）
  ——「Blocker 数 ≡ RT 数」是数据结构不变量；WARNING/INFO 不派生；
- RT 生命周期（后端面——MCP 工具面无 RT 推进工具）：
  `Open → InProgress → Done` 逐级；`Done` 必须带非空 resolution（空说明
  不算修完）；**Done 是终态不可重开**——重走门 = 新 finding 新 RT；
- **导出门要求全部 RT DONE**（`open_blockers` 空）——未关单清单会逐个
  点名 RT-id 拒绝导出；
- 同门重复 Approved：幂等放行**不复位既有 RT**（重新过门 ≠ 已修完——修
  没修完只看 RT 状态）。#180 起幂等判定先比内容摘要：内容已变则落新 REV
  重新绑定（否则改稿后重过门永远返回旧记录 = 死锁）。

与 validate 的关系（诚实桥接）：`validate_bid_document` 的 MAJOR finding
**不自动生成 RT**——它的修订闭环 = 回 P5 重提该章正文 + `scope` 重跑
validate；引擎的 RT 只由过门 findings 里的 BLOCKER 派生。阶段表「每个
MAJOR finding 有 RT 且 DONE」的编排纪律按此对应执行。

## #180 审核内容绑定（过门后改稿 → 旧审核失效）

每条记审落账时快照**章节树内容摘要**（`sections_digest`：按 (order,id)
排序逐节点喂 id/parent_id/title/level/order/content 六字段，sha256-hex——
账面字段不进摘要）。`export_bid_document` 前置裁决会比对**最后一条**
`pre_export_confirm` Approved 的摘要：

- 过门后改了章节正文/结构 → 摘要不符 → `E_UNPROCESSABLE`「审核针对的
  内容已变更，请重新审核」（报文带双方摘要前 8 位）——重过门重新绑定；
- 绑摘要**不绑**全局 content_version（后者被导入无关件/记审自身/导出推进
  一律 bump——绑它必误报）：过门后导入无关件、状态推进、不改正文重复
  导出均不受影响；
- 收尾纪律：**全部叶定稿、dup_pairs=0、篇幅带内之后才过 pre_export_confirm**
  ——过门后再改稿就得重新人工审。

## check_bid_quality 复核面（P7 收口清单）

只读诊断（MATRIX_READY+，不推状态），返回 `summary` + 逐叶 `sections[]`：

| 发现桶（sections[] 字段） | 含义 | 收口动作 |
|---|---|---|
| `uncovered_reqs` | 响应面内未被正文回应的 REQ | 回 P5 补写（`progress_pct < 60% 且已无内容可写`时看这里） |
| `ghost_reqs` | 章节绑定面外的 REQ（抽取漂移） | 如实报，不猜 |
| `unknown_citations` | 幻觉引用残留（>0 即异常） | 修正引用键 |
| `unanchored_claims` | 承诺句无同句引用 | 补 REQ/EV 引用 |
| `within_band: false` | 字数带外（target ±20%） | 增删正文 |
| `dup_with` / `summary.dup_pairs` | 跨章重复对（句级 Jaccard/包含率；≥0.8 严重） | **P7 收尾前 dup_pairs 必须为 0** |
| `summary.char_deviation_pct` | 全局篇幅偏离 | **±10% 内**（超配删冗余、不足补缺口） |
| `needs_human` | FAILED / 有未覆盖 REQ / 幻觉引用 / 带外字数 / 严重跨章重复 | 逐章清零或向用户说明 |

`cited_evs` / `uncited_evs` 是信息面（绑定集内援引情况——非全部证据都
必须引用）；`requirement_ids` 是章节绑定 REQ 全集（章节→条款归属的正式
查询口）。报告落盘 `{workspace}/artifacts/quality-report.json`（纯内存态
artifact=null + 告警）。

**四桶复核语义（与 p6 互补）**：validate 的 `categories`
（structural/semantic/evidence/compliance）是**校验规则**的分桶——
`ok: true` 只意味着「已执行的检查中 BLOCKER == 0」，semantic 恒
`not_performed`（未配置模型）时不得宣称「全部检查已通过」；check_bid_quality
的上述字段是**成稿质量**的分桶。P7 收口 = 两者都清 + 三门 Approved + 全部
RT DONE（BLOCKER 清零路径见 `p6-validation.md`——修复或经用户确认 WAIVED，
**不自作主张豁免**）。

## 常见错误

| 错误 | 触发 | 动作 |
|---|---|---|
| `E_UNPROCESSABLE`（reviewer 在场） | 传了旧 `reviewer` 参数 | 移除——身份由 receipt 提供 |
| `E_UNPROCESSABLE`（receipt 分类码） | 验签失败五类 | 看 `what` 前缀：结构/时效问题让宿主重签；签域不符 = 用错门的 receipt |
| `E_STATE` | 跳门/乱序/窗口外（如 VALIDATING 后补矩阵门） | 按门序重排；矩阵门窗口已过属既成事实——与用户确认处理路径 |
| `E_UNPROCESSABLE`（导出时「审核针对的内容已变更」） | 过门后改稿 | 重过 pre_export_confirm 门 |

## 来源锚点

- `docs/bid-mcp-contract.md` §12（record_review_gate 契约，L466-506）、
  §9（check_bid_quality，L277-309）、§8 门禁段（#180 内容绑定，L263-272）
- `bidcore/crates/bid-mcp/src/tools.rs` L3716-3776（record_review_gate
  壳层：reviewer 显式拒/gates_approved 组装/reviewed_content_digest 回显）、
  L3642-3700 附近（parse_gate/parse_decision/parse_findings）、
  L1995-2167（check_bid_quality：summary 含 dup_pairs/dup_max_similarity/
  char_total/char_target/char_deviation_pct 的组装在 L2146-2167）
- `bidcore/crates/bid-mcp/src/schema.rs` L395-418（record_review_gate
  入参 schema——receipt 描述、findings 元素形状）
- `bidcore/crates/bid-domain/src/principal.rs` L16-27（receipt 线格式/
  scope 绑定/时效窗/常量时间比对）、L39-49（RECEIPT_VERSION/TTL 86400s/
  未来容差 60s）、L122-162（ReceiptError 五类及分类码）、L223-289（验签
  裁决序：解析 → Human → 时效窗 → 签域 → HMAC）
- `bidcore/crates/bid-tools/src/service.rs` L896-981（record_review_gate
  业务半边：scope=`review_gate:{gate}`、verify_approval_receipt 映射
  E_UNPROCESSABLE 带分类码、issue_approval_receipt 宿主签发面不进
  MCP 工具表）、L521-560 附近（approval_key：持久化 `<dir>/.mang/
  approval.key`）
- `bidcore/crates/bid-domain/src/review.rs` L66-98（Gate 四门）、L100-117
  （Decision 语义）、L119-137（Severity——BLOCKER 派生面）、L139-168
  （RTStatus 前向链）、L274-468（record_review 裁决序/状态窗口/门序/
  幂等分支）、L364-371（Rejected 须带 BLOCKER）、L417-444（BLOCKER→RT
  强制派生）、L491-583（RT 指派/推进/关单——resolution 必填）、L585-643
  （export_allowed：三门 + 摘要比对 + 全 RT DONE）
- `bidcore/crates/bid-tools/src/quality.rs` L43-93（SectionQuality 各
  发现桶字段）、L107-135（QualitySummary）、L289/L402-405（严重重复纳入
  needs_human）
- `sdk/skill/bid-response-pipeline/references/p6-validation.md`（四桶语义/
  BLOCKER 清零路径——本文互补面）
- `bidcore/crates/bid-mcp/tests/mcp_scaffold_tools.rs` L212-324（三门记审
  → pre_export → export 的全链时序与断言）、L555-580（伪造 receipt 拒）
