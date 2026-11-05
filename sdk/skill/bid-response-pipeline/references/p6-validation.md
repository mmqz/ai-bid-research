# P6 校验细则（validate_bid_document）

> 契约依据：`docs/bid-mcp-contract.md` §7。规则分层：**确定性规则代码断言、
> 先行必跑、BLOCKER 必须清零**；模型规则只产 finding 不改稿。

## 何时可调（#243 校验状态安全）

| 时点 | 用法 | 行为 |
|---|---|---|
| 起草中（仍有叶未 DRAFTED） | `scope="sections"` + `section_ids`（章节级检查） | 纯读诊断：矩阵未响应面仍全量报告；不推进状态 |
| 起草中误调 `scope="all"` | — | `E_DRAFT_INCOMPLETE`（missing 清单）**零副作用**拒：状态/版本/审核记录均不变，起草面照常——按 missing 清单回 P5 逐叶补起草 |
| 全稿完成 | `scope="all"`（全稿最终校验） | 跑全部规则；**仅 ok=true（BLOCKER==0）才推进 VALIDATING** |
| 校验失败（ok=false） | 修复后重调 | 项目留在 DRAFTING，可修订可重试（draft_response_section / set_matrix_override / 证据补绑）；无状态变迁不落版本 |
| 项目滞留 VALIDATING 且缺章（历史异常态） | `recover_validating_state` | 回 DRAFTING 解锁起草面（见下「历史异常态恢复」） |

## 调用形

```json
{ "project_id": "…", "scope": "all", "rules_profile": "standard" }
```

- `scope`：`"all"`（全稿最终校验——要求全部叶达 DRAFTED）或 `"sections"` +
  `section_ids[]`（章节级检查——起草期诊断面；修订后只重跑受影响章）；
- `rules_profile`：`standard`（缺省）/ `dark_bid`（暗标——附加禁字清单扫描）。

**注意**：校验通过且发生状态推进（DRAFTING → VALIDATING）时，响应携带的
`project_version` 是**推进后的新值**，后续工具必须用新值，否则 E_CONFLICT；
前置拒（`E_DRAFT_INCOMPLETE`）与校验失败（ok=false）零副作用——版本不变。

## 返回核对

```
findings: [ { vf_id, kind: DETERMINISTIC|MODEL, rule_code, severity,
              section_id?, requirement_id?, message, evidence } ]
counts:   { blocker, major, minor, deterministic, model }
categories: {                      # #187 分类返回（四桶各自独立状态）
  structural: { status: "passed" | "failed", findings? },
  semantic:   { status: "passed" | "failed", findings? }
            | { status: "not_performed", reason: "no model configured" },
  evidence:   { status: "passed" | "failed", findings? },
  compliance: { status: "passed" | "failed", findings? } }
```

**四桶语义（#187）**：`ok: true` 只意味着「**已执行的检查**中 BLOCKER == 0」，
**不等于整本标书已通过审核**。四桶各自独立：

| 桶 | 覆盖规则 | 实现状态 |
|---|---|---|
| `structural` | E_SECTION_EMPTY / E_CITATION_GHOST / W_PLACEHOLDER / W_CHARS_OFF_TARGET / E_SCOPE_UNKNOWN | 已实现 |
| `semantic` | M_ 前缀模型发现（表述质量等） | **未实现**——现网恒 `not_performed` |
| `evidence` | E_EVIDENCE_EXPIRED | 部分实现 |
| `compliance` | E_VETO_UNRESPONDED / E_QUAL_UNMET / W_FORMAT_BASELINE | 部分实现 |

- `status: "not_performed"` = 该类检查**没做**（如未配置模型）——**没做 ≠ 做对了**
  （issue #9 口径），不得当成通过；
- `status: "failed"` = 该类**有发现**（含 MINOR 告警——废标级与否看 ok/counts）；
- `not_performed` 不把 `ok` 翻 false（能力缺席不是缺陷）。

**必须向用户复述**：`BLOCKER b / MAJOR j / MINOR k`，逐条 BLOCKER 给出修复
方案；**同时如实转述四桶状态**——semantic 为 not_performed 时必须明说
「语义质量复核未执行（未配置模型）」，不得宣称「全部检查已通过」。

## 确定性规则清单（代码断言——引擎实测面）

| rule_code | 触发 | severity | Agent 修复姿势 |
|---|---|---|---|
| `E_VETO_UNRESPONDED` | 否决项未响应 | BLOCKER | 回 P5 对应章节补响应（引用 REQ+EV） |
| `E_QUAL_UNMET` | 资质行不满足 | BLOCKER | 如实声明偏离或走用户决策（不可伪装满足） |
| `E_SECTION_EMPTY` | 章节空内容 | BLOCKER | 回 P5 补写正文 |
| `E_CITATION_GHOST` | 幻觉引用（EV 不存在/未绑定） | BLOCKER | 正文引用键改为已绑定 EV——纪律 3 的拦截点 |
| `E_EVIDENCE_EXPIRED` | 引用了过期证据 | BLOCKER | 回 P4 换绑 ACTIVE 证据 |
| `E_SCOPE_UNKNOWN` | scope 引用了不存在的章节 | 错误（参数面） | 改 scope 重调 |
| `W_PLACEHOLDER` | 残留 TODO/待确认/占位符 | WARNING | 草稿痕迹清零（纪律 7）——替换为正式表述或引用其他章节 |
| `W_CHARS_OFF_TARGET` | 字数带外（target ±20%） | WARNING | 增删正文至带宽内（check_bid_quality 的 within_band 可预检） |
| `W_FORMAT_BASELINE` | 格式基线偏离 | WARNING | 对齐格式模板件 |

另（引擎规则族持续增补，以 find 出的 rule_code 原样转述为准——不要按本表
猜测未列出的规则含义，message 是人读正文）。

## 模型检查范围（kind=MODEL，rule_code 以 `M_` 前缀）

表述质量 / 逻辑连贯 / 偏离合理化 / 疑似过度承诺——**只产 finding 不改稿**：
你读 finding 后回 P5 修订该章（external 模式重提 content），再重跑 validate。

## BLOCKER 清零路径（导出前置）

`校验通过（ok=true）才推进 VALIDATING；BLOCKER 清零（或登记 WAIVED+理由）才可 EXPORT`

两条路：

1. **修复**（推荐）：逐条 BLOCKER 按上表修复 → `scope` 重跑受影响章 → 全量
   `scope=all` 复核；
2. **豁免**：确属误报或业务上可接受 → 登记 WAIVED + 理由（记录在案，导出
   precheck 的 waived 清单可见）。

### WAIVED 审批话术（向用户）

> 「BLOCKER `<vf_id>`（`<rule_code>`：`<message>`）经核对属于
> <误报/业务可接受>，理由：<…>。是否豁免？豁免将记录在导出预检的 waived
> 清单中。」

用户确认后才豁免——**不要自作主张豁免**（AI 操作者不得自审自过的门禁
精神）。

## P7 审校闭环（validate 的 MAJOR 面）

每个 MAJOR finding → 修订任务（RT）→ 修订 → `DONE`。只重跑 DIRTY 章节
（上游 content_version 变化波及面）；未受影响的 ACCEPTED 章节不动。

## 历史异常态恢复（#243 · recover_validating_state）

旧版引擎的校验会在草稿未完成/校验失败时误推进 VALIDATING，而 VALIDATING
态禁起草 → 项目死锁（用户曾被迫终止进程并手改 db.json 才能续跑——那不是
受支持路径）。若 `get_bid_project_state` 显示项目在 `VALIDATING` 且仍有
未起草叶：

1. 调 `recover_validating_state`（`reason` 必填非空白，如实写明原因——审计
   留底：何时/为何/从哪到哪/缺哪些章）→ 项目回 `DRAFTING`，起草面解锁；
2. 按恢复响应的 `missing_sections`（或重查矩阵）逐叶补起草；
3. 全稿完成后 `scope=all` 校验 → 通过后进导出流程。

约束：仅 VALIDATING 态、仅缺章可恢复（草稿完整的 VALIDATING 是健康态，拒）；
重复恢复被状态门拒（E_STATE，非死锁态）。**禁止手改 db.json**。
