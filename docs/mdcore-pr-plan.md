# mdcore PR 预案 → 验收复测报告（T3）—— issue #2 评论 2026-10-27 03:28 派发

> **状态变更说明**：本任务原为「mdcore#17/#18/#19 归属确认后由标书侧提 PR 的预案」。
> 执行中发现引擎侧已**自行实现三单并合入 main**（#17 修订提取 `c60214f` / #18 文本框
> `977f4c9` / #19 批注脚注 `f7ddd39`，issue 均已关闭）——**标书侧提 PR 的预案作废**
>（§3.2 协议下无 PR 需求，引擎 owner 自研符合归属判定）。
> T3 交付据此转为**标书侧独立验收复测**（亲跑+留证纪律）：以 T2 夹具契约 +
> T1 差分工具为验收面，对引擎修复做双侧对跑实证。**结论：三单全部通过验收。**

---

## 1. 验收方法（与工具链）

| 面 | 工具 | 输入 | 证据 |
|---|---|---|---|
| 引擎探针 | `scripts/engine_probe/`（基线已前移 f7ddd39） | 15 探针（10 基线 + 5 个 D6 变体） | `artifacts/engine-probe/report-after-fixes-f7ddd39.json`（修复前对照 `report.json` @ f8c860a，双基线并存） |
| 双跑差分 | `scripts/engine_diff.py` + `engine-probe-dump` | 8 对真实夹具（旧=baseline/parse 双侧=引擎 dump） | `artifacts/engine-diff-after/`（修复前对照 `artifacts/engine-diff/`） |
| 夹具契约 | `fixtures/corpus-R/D6-revisions/expected-readback.json` | R6a–R6e 五变体期望读回（ECMA-376 口径） | 逐变体断言（T17c/T17d 集成 run_tests） |

## 2. 逐单验收结论（全部通过）

### mdcore#17（P0 · D6 修订提取）✅

- **ins 当前文本入读回**：R6a「开标地点：示例大厦三层第二开标室（插入后生效）」/
  R6b 前缀 / R6c 同段替换「更正后开标时间：2026-11-27 09:30」/ R6e 全部
  修复前缺失 → 修复后在读回文本（差分 ENGINE_MISSING 7→0）；
- **run 级标记面**：`revision_inserted_runs` R6a=2 / R6c=2（段落面）/ R6e=2 / D2a=1 ——逐件可验；
- **delText 不提取 + 逐段登记**：R6b warnings=2（恰=2 处 del）/ R6c warnings=3（恰=3 处）/
  R6e warnings=1（恰=1 处）/ D2a warnings=1——**计数逐件吻合零静默**（f8c860a 时 warnings 全 0，
  呼应引擎侧「静默丢失比丢失本身更危险」发现）；
- **勘误确认**：真 Word COM 侧发现「TR2 word-complex 第一章标题段含 `REV-INSERT-` ins 前缀」
  在新实现下正确保留（读回「REV-INSERT-第一章 概述」= OOXML 语义，ins 前缀是修订插入的本貌）。

### mdcore#18（P1 · D4 文本框）✅

- **文本框入块流**：D2a `text_boxes=1`、条款文本逐字完整、块序紧随宿主段；
- **宿主零截断**：D2a 澄清一至四逐字完整；
- **⚠ 勘误双向确认（重要）**：引擎指出的「bid 基线 2 块命中 = 标书侧双重提取缺陷」
  经标书侧差分**独立确认**（`D4-DOUBLE-EXTRACT`）：bid_parse 对同一文本框条款产出
  两个重复块（`docx.rs` 事件流 txbxContent 嵌套段与宿主段状态污染），引擎恰 1 块为正确口径。
  **登记为 bid_parse 第三项已实证缺陷**（与 delText 泄漏、空段丢弃口径并列），
  B4 删除旧实现时随之消除，不单独修。

### mdcore#19（P2 · D5 批注/脚注）✅

- **批注面**：R6d comments=2（双作者双批注）/ R6e comments=2 / D2a comments=1 /
  TR2 word-complex comments=1（真 Word 件）——`CommentData{anchor_block_idx}` 锚关联可验；
- **脚注/尾注**：引擎侧测试断言已覆盖（分隔符过滤 + `footnote_refs` 引用位）；
  标书语料含脚注部件的电信 2 件（`B-telecom-huaian-合同.docx` / `技术规范书.docx`）
  留作 B3 双跑差分扩展对（未在本轮 8 对内——真实件体量大，留基线全量差分时覆盖）。

## 3. 连带发现：引擎 Phase 1（A1–A4）同步验收 ✅

f8c860a..f7ddd39 区间引擎还合入了 **A1 标题识别**（`a85decd`/mdcore#13）、
**A2 编号去重**（`338acdc`/#14）、**A3 行内 marks**（`fd3f884`/#15）、
**A4 G2 导出链**（`1b8e1ab`/#16）。标书侧探针独立复核：

| 项 | 修复前（f8c860a） | 修复后（f7ddd39） | 判定 |
|---|---|---|---|
| A2 编号叠加 | word-complex 读回「11.1 表格」 | **「1.1 表格」** | ✅ B2.5 判定的 B3 硬前置**已解除** |
| A1 标题识别 | headings=0（真 Word styleId="1" 不识别） | **headings=4**（四链解析：outlineLvl 直接格式/HeadingN 前缀/样式名中英文形/样式级 outlineLvl） | ✅ B2.5 已证 A1 当前形态对标书抽取零影响，修复后 heading_level 可作 B3 冗余加固面 |
| A3 行内 marks | `**粗体**` 星号原样（废标风险） | 引擎侧测试断言五位格式 run 化 | 引擎侧门禁绿；标书面 V1-c 导出链回归待 B3 |
| A4 G2 导出链 | API 在位未消费 | CONTRACT §4 v1.4 设置面 + 导出层 | 标书面 G2/G3 消费走 B3 后导出回归 |

## 4. 修复前后差分总账（B3 验收预演）

| verdict | 修复前（f8c860a） | 修复后（f7ddd39） | 说明 |
|---|---|---|---|
| EQUIVALENT | 54 | **61** | +7 = 引擎修复面 |
| ENGINE_MISSING | 9（D6-INS×7 + D4-TEXTBOX×2） | **0** | **引擎缺口清零** |
| OLD_BUG | 2 | 4 | 修复后 bid 侧缺陷显影：delText 泄漏×3（D2a/R6b/R6e）+ 文本框双重提取×1（D2a）——全部 B4 换 bid-engine 时消除 |
| EXPECTED_DIFF | 4 | 3 | 空段结构性差异（适配层丢空段对齐旧口径；EMPTY-PARA） |
| UNCLASSIFIED | 0 | **0** | B3 验收「待裁清零」在夹具语料上**已达成** |

xlsx 面（B3 图纸目录 + D1b 宽表）双侧 **38/38 全等价**（修复前后不变——该面引擎本就无缺口）。

## 5. B3 就绪度与残留

- **A1–A4 + D4/D5/D6 引擎侧全部在位**（f7ddd39）；**A2 硬前置解除**；
- 探针基线已前移 f7ddd39（f8c860a worktree 保留为历史基线复现通道）；
- B3 开工仍**等 Lead 正式通知**（issue #2 时间线约定——不自行宣布）；
- B4 前置清单新增第三条实证：bid_parse 文本框双重提取（与 delText 泄漏、
  S7 ratio 误报面并列，均在旧实现删除后消失）；
- 修订移动（MoveFrom/MoveTo）引擎 v0 维持不提取（#17 范围外登记）——
  真实澄清件该形态罕见，登记不阻塞。
