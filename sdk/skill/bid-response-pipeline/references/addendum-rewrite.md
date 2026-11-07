# 补遗后的增量重写（#319 两工具——起草后补遗到达的局部改写面）

> 契约依据：仓库根 `docs/bid-mcp-contract.md` §23/§24。**为什么存在**：
> 补遗/更正公告常在起草中途甚至导出后到达。V2 影响面计算只做了「标记
> DIRTY + 级联失效」，但 DIRTY 章节怎么改没有出口——只能整章重写
> （浪费已写好且已通过的内容）或人工判断改哪里。本组工具把「补遗改了
> 什么」翻译成「哪些**段**要改」，只动受影响段，其余内容字节级不动。

## 何时走这条路

- 起草已开始（MATRIX_READY+，含 EXPORTED）+ 收到补遗/更正/澄清；
- 起草**前**到达的补遗不走这里——P1 导入 → P2 抽取，REQ 天然进矩阵。

## 第一步：`register_addendum_changes`（登记 + 生成指令）

```json
{ "project_id": "PRJ-0001",
  "changes": [
    { "req_id": "REQ-0018", "change_type": "value_changed",
      "old_text": "设备内存 32GB",
      "new_requirement": "补遗第 2 号：设备内存提升至 64GB，部署形态不限",
      "source_file_id": "TF-0003" },
    { "change_type": "requirement_added",
      "new_requirement": "提供原厂授权书及三年免费上门维保",
      "category": "商务要求", "clause_no": "6.2", "section_id": "SEC-0006" },
    { "req_id": "REQ-0021", "change_type": "requirement_removed",
      "old_text": "质保期三年", "new_requirement": "" }
  ],
  "project_version": 42 }
```

要点：

- `old_text` 是**受影响段的定位线索**：段含此文本即入影响面（与「段引用
  该 REQ」双腿并集——`matched_by` 回显命中腿）。写得太短会漏段、太长会
  匹配不上——取补遗原文中「旧要求的特征短语」；
- 批量**原子**：任一条校验失败（REQ 不存在/类别非法/叶清空）整批拒，
  零副作用；错误消息带 `changes[i]` 定位；
- 登记即落账：R-mod 的最新要求进 `params.addendum_requirement`；R-add
  的新 REQ 立即可被段引用（已挂叶）；R-del 连证据绑定一起解除、矩阵行
  消失——**不要在正文里继续口头响应已删除的条款**；
- 返回 `directives`（RD-xxxx）+ `dirty_sections`；无段账本/未匹配到段
  的指令带 `directive_warnings`（如「章节未起草——可 dismiss」）。

## 第二步：`rewrite_section_partial`（逐指令执行）

先预览（dry_run——「将修改 X 段，Y 段不动」，零落笔）：

```json
{ "project_id": "PRJ-0001", "directive_id": "RD-0001", "dry_run": true,
  "rewrites": [
    { "segment_id": "SEG-0003", "content": "…该段重写后全文（引用 [REQ-0018]）…" }
  ] }
```

真跑（去掉 dry_run）：

- `rewrites` 必须 ⊆ 指令影响面（越界段 E_UNPROCESSABLE——防借指令改
  无关段）；`expected_version` 是段级 CAS（list_section_segments 回显）；
- 返回 `modified_segments` / `untouched_segment_ids` / 局部复验
  （`revalidation`：只验受影响章节——blocker/major/minor + 「M 条 REQ
  重新响应」）/ `dirty_cleared`；
- 章节全部指令处置完自动清 DIRTY；进度面在 `get_bid_project_state` 的
  `dirty_sections` + `pending_rewrite_directives`。

三种特殊处置：

| 场景 | 做法 |
|---|---|
| 新增要求（影响面恒空） | `append_section_segment` 写响应段（引用新 REQ id）→ 回本工具**空 rewrites** 处置 |
| 人工判定无需改写 | `dismiss: true`（与 rewrites 互斥）——同样参与清 DIRTY |
| 影响面覆盖绝大多数段 | 那是真重构——走 `draft_response_section` 整稿替换（段账本重置单段） |

## 反模式（禁止）

- ❌ 收到补遗就把 DIRTY 章节整章重写（浪费已通过的修订——本工具组存在
  的理由）；
- ❌ 跳过 dry_run 直接真跑（改动前预告是纪律——影响面核对后再落笔）；
- ❌ 在 rewrites 里夹带影响面之外的段（会被拒——别试）;
- ❌ R-del 后正文仍保留旧条款的响应段不处置（指令影响面会点名它——
  重写去掉引用或 dismiss 并向用户说明）。
