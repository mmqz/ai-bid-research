# P5b 章节分段写入（#254 三工具——长章节分批写作的引擎对偶面）

> 契约依据：仓库根 `docs/bid-mcp-contract.md`（路径约定见 SKILL.md 头部——`docs/` 指仓库根，非本目录相对） §6-A（分段写入）。**为什么存在**：
> `draft_response_section` 的 `content` 是整节全文，提交即**整稿替换**——
> 旧指引「写一批→提交→写下一批」会静默丢弃前批（真实事故：SEC-0006
> 九次重提，5628→12201→…→5868 字越写越短）。本组工具给出**真增量**语义：
> 每次只提交新段，引擎按序追加并合稿，前批永不丢。

## 核心语义（先读这段再动手）

- **段（SEG-xxxx）**：章节正文的组成单元，有稳定 ID + 顺序（seq，1 起）
  + 幂等键 + 版本（version）；
- **合稿**：该章节全部段按 seq 升序以 `\n\n` 拼接 = 章节正文
  （`OutlineNode.content` 单一真源——validate/export/引用回查/字符统计/
  `E_CHARS_BELOW_MIN` 全部按**合稿**计算，单段不判门）；
- **账本与正文同进退**：append/rewrite 先改段账本，再同步合稿正文——
  两者永不对不上；
- **整稿 ↔ 分段互斥**：`draft_response_section` 提交**不同内容**会把该章
  段账本重置为单段（整稿全文）；同内容重提不动账本（幂等）。

## 三工具

### 1. `append_section_segment` —— 追加一批

```json
{ "project_id": "PRJ-0001", "section_id": "SEC-0006",
  "content": "…本批 ≤4000 字正文（引用语法与整稿一致 [REQ-0001]/[EV-0002]）…",
  "idem_key": "SEC-0006/batch-1", "project_version": 14 }
```

| 参数 | 必填 | 说明 |
|---|---|---|
| `section_id` | ✅ | 目标叶章节（结构节点响亮拒） |
| `content` | ✅ | **本批段文**（不是章节全文！） |
| `idem_key` | — | 幂等键；缺省 = 内容指纹 → **同段同内容重提自动 no-op**；网络重试场景可显式指定（如 `SEC-0006/batch-1`） |

响应关键面：`segment.{id,seq,version,chars}` / `changed`（false = 幂等重放）
/ `assembled_chars`（合稿字数）/ `remaining_chars`（剩余配额，可负）/
`over_target`（合稿超 +20% 带）/ `segment_count` / `citations` /
`unanchored_claims` / `warnings`。

### 2. `rewrite_section_segment` —— 局部修订（改一批，不动其他批）

```json
{ "project_id": "PRJ-0001", "segment_id": "SEG-0007",
  "content": "…该段修订后的全文…", "expected_version": 2 }
```

- 定位用 `segment_id`（`list_section_segments` 回显——不要猜 ID）；
- `expected_version` = 该段当前 version（CAS）：不匹配 → `E_CONFLICT`
  （并发/过期写入显式拒，**无部分写入**——段内容/合稿/版本全不动）；
  缺省跳过检查（与 `project_version` 乐观并发同模式）；
- 同内容重写 = no-op（version 不推进）。

### 3. `list_section_segments` —— 对账与恢复（只读）

```json
{ "project_id": "PRJ-0001", "section_id": "SEC-0006" }
```

响应：`segments[]`（逐段 `id/seq/version/chars/idem_key/preview`——
preview 为段首 40 字，人侧辨识用）/ `assembled_chars` / `target_chars` /
`remaining_chars` / `over_target`。

## 错误面（全部零副作用）

| 错误 | 触发 | 处置 |
|---|---|---|
| `E_NOTFOUND` | 章节/段不存在 | 先 `list` 对账（重启后 SEC/SEG 编号不变——持久化保真） |
| `E_UNPROCESSABLE` | 空白段；结构节点；**引用回查失败**（段内 `[REQ-*]/[EV-*]` 不在本章绑定集） | 分段路径**不自动剔除**幻觉引用（保「合稿==段拼接」不变量）——修正引用后重提 |
| `E_CONFLICT` | `expected_version` 过期 | 有并发写作——`list` 拿新 version 再改 |
| `E_STATE` | 项目态不在窗口（append/rewrite = MATRIX_READY+；list 全窗口只读） | 按阶段表推进 |

## 恢复流程（断点/重启——段账本随快照持久化）

```
重启 → list_section_segments(章节)
     → 看 segments[].seq（已写到第几批）+ remaining_chars（还差多少）
     → append_section_segment 接着写（seq 自动接续，id 水位防碰撞）
```

## 端到端示例（32,271 字/节 = 9 批）

```jsonc
// 批 1-9：同构循环（每批 ≤4000 字，idem_key 用「章节/批号」）
append_section_segment { "section_id": "SEC-0006", "content": "…批1…",
                         "idem_key": "SEC-0006/1" } → { "segment": { "seq": 1 }, … }
append_section_segment { … "idem_key": "SEC-0006/2" } → { "segment": { "seq": 2 }, … }
// …（超时重发同批：同 idem_key/同内容 → deduplicated:true，不重复追加）

// 中途改批 3（其他批不动）
list_section_segments { "section_id": "SEC-0006" }   // → SEG-0009, version 1
rewrite_section_segment { "segment_id": "SEG-0009", "content": "…批3修订…",
                          "expected_version": 1 }     // → version 2

// 服务重启后（段全在）
list_section_segments { "section_id": "SEC-0006" }
→ { "segments": [9 段], "assembled_chars": 32271, "remaining_chars": 0 }

// 收尾：validate/export 与整稿完全同路（门按合稿）
validate_bid_document { "project_id": "…", "scope": "sections", "section_ids": ["SEC-0006"] }
export_bid_document   { "project_id": "…" }
```

## 与既有面的关系

- **`draft_response_section`（整稿）**：≤4000 字小章节直接用；已分段章节
  重提**不同内容** = 整章重写（账本重置单段）——旧段修订历史不保留，
  慎用；
- **`save_version`**：每批写完仍然要存（message 写「第 N/M 批」）——分段
  幂等保的是**引擎账本**，版本快照保的是**过程稿**，两层各管各的；
- **`check_bid_quality`**：跨章查重/覆盖度看合稿，与分段无关（透明）；
- **quota 的 `min_segments`/`remaining_segments`**：分批节奏规划数字
  （4000 字/批折算）——分段工具落地后这两个数字有了真实的执行载体。
