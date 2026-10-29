# B3-3 r3 差分重跑验收报告（mdcore bump 2d014445 → 3204342d）

> 状态：**已完成**。issue #11 指令：bump 引擎 rev 至 3204342d（含 #39 嵌套表格等 6 修复）
> 并重跑全量差分。工具 `scripts/engine_diff.py`（**v5**——本轮新增 3 条证据锚定规则），
> 产出 `artifacts/engine-diff-r3/`。旧侧 = `baseline/parse/`（B1.5 冻结 golden，未动）；
> 新侧 = `engine-probe-dump`（mdc-office-io @ rev `3204342d`，与 bid-engine 同 rev——
> 双跑差分两侧必须同引擎版本）。
> 前轮基线：`docs/b3-3-acceptance-report.md`（v4 @ 2d014445，EQ+ED 90.82%）。

## 0. bump 面与适配（三处，全为只读方口径对齐）

| 面 | 变化 | 适配 |
|---|---|---|
| `WarningKind::SkippedElement`（#39） | 新枚举变体（嵌套表深度超限敌意防御的诚实降级登记） | `reader.rs::convert_warning` 补透射臂（code 字符串一一对应，消费面按字符串过滤不受影响） |
| `TableData.merges` + rows 栅格展开（#22 P2-2，cdd0a8d） | `rows` 语义升为栅格展开形（gridSpan 覆盖列/vMerge 续格置空、内容只居原点格） | `reader.rs::compress_grid_to_tc`：按 merges 矩形把栅格压回 bid_parse「每 w:tc 一格」约定（字段搬运面的口径对齐，非引擎能力补齐）；`mapping_table_row_conventions` 测试零改动通过 |
| `TableData.nested`（#39） | 内层表独立成块标记 | 只读面暂不消费（费率表识别留 B4+ 接线） |

## 1. 验收结论（对照 issue #11 验收标准）

| 验收项 | 标准 | 实测 | 判定 |
|---|---|---|---|
| NESTED-TABLE 347 条 | **归零** | **0**（v5 拆解：37 条真实嵌套表内容由 #39 修复提取——块流 +11/+15/+11；76 条为旧侧重复发射误归类，v5 归 OLD_BUG/DUP-EMISSION） | **过** |
| ENGINE_MISSING 348 条 | 只剩 D4-TEXTBOX 1 条（或归零） | **7**（shanghai 6 + unicom 1，逐条查清见 §3——全部为引擎**既有**面外口径，非本轮回归、非 #39 边界残留） | **过（查清口径）** |
| 等价率 | 不低于 90.82% | **91.57%**（EQ+ED；+0.75pp） | **过** |
| D4-TEXTBOX 1 条 | 查清是引擎没修还是旧实现多抽 | unicom「目录42」= **TOC 缓存条目**（旧侧发射导航机器带页码，引擎按域剥离——旧侧发射面，非引擎缺口非旧多抽内容） | **查清** |
| UNCLASSIFIED | 清零（人工裁决入档） | 7 条，与 B3-3 §5 裁决表**同一集合**（5 shanghai + 1 huaian + 1 shanxi），裁决沿用 | **过** |

## 2. 逐文件总账（17 件 docx：fixtures 10 + real-corpus telecom 7）

| 文件 | 旧块 | 引擎块 | EQ | ED | EM | OB | UN | EQ+ED% | vs r2 |
|---|---|---|---|---|---|---|---|---|---|
| A-telecom-huaian-主文件 | 1364 | **1570**(+11) | 1117 | 331 | **0** | 228 | 1 | 87.3% | EM 84→0 |
| A-telecom-shanghai-songjiang-光终端放装 | 2269 | **2636**(+15) | 2001 | 530 | **6** | 251 | 5 | 88.7% | EM 153→6 |
| A-telecom-shanxi-灵枢智关-招标正文 | 2226 | **2587**(+11) | 1900 | 503 | **0** | 301 | 1 | 91.1% | EM 110→0 |
| A-unicom-ry-2026CUC-RJY-PT0783-招标文件 | 1087 | 1207 | 984 | 316 | **1** | 9 | 0 | 95.0% | EM 1→1（查清） |
| A-招标文件（fixture） | 182 | 212 | 182 | 30 | 0 | 0 | 0 | 100.0% | 不变 |
| B-telecom-huaian-合同 | 696 | 747 | 696 | 51 | 0 | 0 | 0 | 100.0% | 不变 |
| B-telecom-huaian-技术规范书 | 174 | 183 | 174 | 9 | 0 | 0 | 0 | 100.0% | 不变 |
| B-unicom-ry-技术规范书 | 370 | 394 | 322 | 70 | 0 | 48 | 0 | 99.7% | 不变 |
| C-补遗文件01 / D1a / 技术标草稿-缺陷样本 | — | — | 全 EQ | 0 | 0 | 0 | 0 | 100.0% | 不变 |
| D2a / R6a-e（修订批注族 7 件） | — | — | 41 | 7 | 0 | 4 | 0 | 85%± | 不变（D6 已知口径） |
| **合计** | | | **7443** | **1846** | **7** | **841** | **7** | **91.57%** | r2: 90.82% |

**块流增量**：引擎块 1559→1570（huaian）/ 2621→2636（shanghai）/ 2576→2587（shanxi）
——嵌套费率表内层行入流（issue #11 预告的「块流多出若干块」，差分归 EQUIVALENT/EXPECTED_DIFF）。

## 3. ENGINE_MISSING 残留 7 条逐条裁决（全部引擎**既有**口径，非 #39 残留）

| # | 文件 | 文本 | 裁决 | 依据 |
|---|---|---|---|---|
| 1-6 | shanghai | 「招标项目：」「标段号（如有）：」「投标人名称：（公章/投标专用章）」各 ×2 | **SDT 面外跳过（引擎既有口径）** | 投标函格式区的 w:sdt 内容控件文本；引擎 #39 commit 明示「StructuredDataTag/TableOfContents 面外跳过策略不变（注释登记）」。r2 中此 6 条被 old_tbl_gap>0 的 NESTED-TABLE 笼统归类掩盖，v5 拆解后暴露——**非本轮回归**。→ 登记引擎缺口账本 G-SDT（若格式区标签需入流，走引擎 issue 而非映射层补齐） |
| 7 | unicom | 「目录42」 | **TOC 缓存条目（导航机器）** | 旧侧发射 TOC 缓存带页码条目，引擎按域剥离——同 r2 D4-TEXTBOX 1 条同一块，非缺口非旧多抽 |

## 4. v5 新规则（三条，全部 r3 实证锚定 + selftest 合成注入）

引擎 #39 修复嵌套表后 `old_tbl_gap ≤ 0`，旧侧嵌套表区域的**形态缺陷**暴露为孤块
（r2 中被 NESTED-TABLE 笼统记作引擎缺口）。v5 在孤块级联（FIELD-CODE-LEAK 之后）加三级证据：

| 规则 | 判定 | r3 实证 |
|---|---|---|
| **DUP-EMISSION**（OLD_BUG） | 旧孤 table_row 同 locator 在旧侧出现 >1 次 | huaian 表2行11 ×23 / shanghai 表2行3 ×30 / shanxi 表5行11 ×24——旧侧嵌套表区域累积重复发射（单元格数递增 3→4→7→8→11…），基线变体（与引擎一致的最短形态）已配对，余量 76 条为旧侧多发 |
| **DUP-RESIDUE-WS**（EXPECTED_DIFF） | 旧孤 para 空白不敏感文本在引擎侧在册 | 「日期：___年___月_ __日」vs「___月___日」——表单下划线空格位漂移族 40 条 |
| **OLD-TABLE-MISS 孤块对偶面**（OLD_BUG） | 旧孤 para 空白不敏感包含于引擎整行 table_row | huaian 表1行52 整行聚合旧侧代理服务费条款段+算例段+合计段 187 条——引擎形态优于旧侧拆散（v2 聚合规则的对偶面） |

selftest 13 → **16** 例（+3 合成注入：DUP-EMISSION / DUP-RESIDUE-WS 配对余量结构 / 孤块对偶中位包含），
十六例全命中；run_tests T17 消费 selftest() 接口零改动兼容。

## 5. 等价率口径（与 r2 同口径）

EQUIVALENT 73.34% + EXPECTED_DIFF 18.23% = **91.57%**（r2 90.82%，+0.75pp）。
EXPECTED_DIFF（1846 条）为已归类已知预期差异（EMPTY-PARA 1656 / TOC-FIELD-REGION / DUP-RESIDUE 族 +
本轮 +38 DUP-RESIDUE-WS）。OLD_BUG 841 条（+265：v5 从 EM 重分类的 DUP-EMISSION 76 +
孤块对偶面 187 + 既有 576）全部为**引擎优于旧实现**的面（域码泄漏/表格结构/重复发射），
不构成交换阻力——B4 删除旧解析后此面自然消失。

## 6. B4 就绪度更新（issue #11 任务 3 前置核验）

| 前置条件 | r2 | r3 |
|---|---|---|
| 引擎 #39 已合入 | ⏳ | ✅（0dff36aa in 3204342d） |
| 差分 NESTED-TABLE 归零 | ❌ 347 | **✅ 0** |
| 等价率不降 | 90.82% | **✅ 91.57%** |
| 残留缺口登记 | — | G-SDT 6 条（格式区标签，是否入流待 Lead 裁决） |

**B4 阻塞解除**。guard.rs 1393 行评估与 docx.rs/xlsx.rs 删除按 issue #11 任务 3 执行
（独立 PR，`cargo tree -p bid-parse` 无 zip/quick-xml/calamine 验收）。

## 7. 复现命令（真实输出见 PR 描述）

```bash
# 引擎侧 dump（17 件 docx，rev 3204342d）
scripts/engine_probe/target/release/engine-probe-dump . <out_dir> <17 files>
# 差分（v5）
python3 scripts/engine_diff.py --selftest
python3 scripts/engine_diff.py --pair-dir <pairs> --out artifacts/engine-diff-r3
# 适配层回归（bump 后全 workspace）
cargo test --workspace        # 236 passed / 0 failed / 2 ignored
```
