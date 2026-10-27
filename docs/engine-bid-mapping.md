# 引擎 → 标书字段映射表（engine-bid-mapping）—— issue #2 · T1 产物（B3 预演）

> **用途**：`bidcore/crates/bid-engine`（T1 只定义）在 B3 实装时的**单一真源**——
> 引擎字段 × bid 字段 × 转换规则 × 缺失处置，四列对齐；实现偏差以本文档为验收对照。
> **锚点**：引擎形状 mdcore @ `f8c860a`（只读 worktree：`office-io/src/docx.rs:1468-1529`、
> `office-io/src/io.rs:1112-1222`、`mdc-pdf/src/command.rs:94-148`）；bid 目标类型
> `bid-parse/src/lib.rs:25-60`（BlockKind/Block/ParsedFile）；旧实现口径
> `bid-parse/src/docx.rs:85-215`（事件流提取 + SC-02 部件账本）。
> **纪律**：映射层只做**调用与字段搬运**——解析/安全门/指纹语义一律引擎侧
> （issue #2 §1 约束 1/2）；缺口走「缺失处置」列显式登记，**禁止映射层自研补齐**（约束 3）。

---

## 0. 差异归类词典（双跑差分工具 `scripts/engine_diff.py` 的判定面）

| 归类码 | 含义 | 判定来源 |
|---|---|---|
| `EQUIVALENT` | 归一后文本与 kind 双侧一致 | 基线等价面 |
| `ENGINE_MISSING` | 引擎缺口（旧有新无）——D6 ins/delText、D4 文本框、D5 批注脚注、A1 heading 属性 | `docs/engine-gap-requests-m0.5.md` |
| `OLD_BUG` | 旧实现缺陷（新有旧无/旧多出）——delText 粘性泄漏、文本框宿主段截断 | T2 实测发现 + `docx.rs` 怪癖登记 |
| `EXPECTED_DIFF` | 已知预期差异（A2 编号叠加 / 列对齐空白 / 页码等观感差 / 空段） | A1–A4 清单 + P1 定谳 + EMPTY-PARA（引擎保留空段、旧实现丢弃——适配层丢空段对齐） |
| `UNCLASSIFIED` | 不属上述任何类——**必须人工裁决**，差分报告显式列出 | 兜底（不静默） |

B3 验收口径（issue #2 §B3）：≥90% 文件 blocks EQUIVALENT；其余每条有上表归类，
`UNCLASSIFIED` 清零后才可进 B4。

---

## 1. docx：`EngineDoc` → `ParsedFile`

| 引擎字段（mdcore） | bid 字段（bid-parse） | 转换规则 | 缺失时怎么办 |
|---|---|---|---|
| `DocxDocument.blocks[].Block::Paragraph` | `Block { kind: Para }` | 逐块直连，`idx` 按引擎块序重编；**空段丢弃**（引擎保留空段、旧实现丢弃——T1 实测：del-only 段落引擎为空段块、旧为无块；适配层丢空段保持块流口径，差分归 `EXPECTED_DIFF/EMPTY-PARA`） | — |
| `ParagraphData.text()`（run 直连拼接） | `Block.text` | 直连（引擎侧含 ins 内文本后自动补齐 D6 面） | **D6 修复前**：ins 文本缺失 → `ENGINE_MISSING`；账本登记「修订插入 present>0 / extracted=0 + 引擎缺口 D6」 |
| `ParagraphData.style` | （`Block.kind` 判定辅助，不落字段） | `HeadingN` 样式 → 段落判型参考；**不改变 BlockKind**（标书 kind 面无 Heading 变体） | **A1 修复前**：引擎 headings 恒 0/普通段 → 无影响（B2.5 实测：标书 extract 按文本正则自建章节上下文，A1 当前形态零影响）；不新增 kind 变体 |
| `ParagraphData.numbering`（w:numId） | （消费于 text 归一） | 编号面与可见文本分离后：A2 修复引擎不再把自动编号叠加进 text | **A2 修复前**：引擎 text 含「11.1」型叠加 → 归一规则剥离后判 `EXPECTED_DIFF`（A2）；**A2 是 B3 硬前置**（B2.5：99.9% 条款号污染） |
| `Block::Table(TableData.rows/cells)` | `BlockKind::TableRow` 每行一块 | `rows[i]` → `cells`；`text` = 非空列以单空格连接（旧实现在 `w:tr` END 同口径——`docx.rs:140`） | —；合并结构：X3 定谳引擎扁平（设计登记），D1a 消费口径（行列编址在合并下取读回值）两侧一致 |
| `DocxDocument.headers` | `ParsedFile.components` 账本行「页眉」 | present=挂接数 / extracted=0 / note=引擎警告透传 | D3 定谳：**引擎有**（`HeaderFooterData` 块级读面）——旧实现零提取（`docx.rs:202` 同口径）。B3 起 note 从「零提取（登记不伪装）」升「引擎读面已接、待标书面消费设计（M2 定位器变体）」；**文本不入块流**（与旧行为一致，块流稳定性优先） |
| `DocxDocument.footers` | components「页脚」 | 同上 | 同上 |
| `DocxDocument.images` | components「图片」 | present=`images.len()` / extracted=0 / note=容器级枚举 | 块序内图片位置引擎不入块模型（登记面）；OCR/扫描消费走 pdf 通道 |
| `DocxDocument.warnings` | components 各行 `note` 透传 + `parse_summary` | 警告文本逐条并入（SC-02 不变式：零提取必带 note） | — |
| （引擎无：批注） | components「批注」 | present=comments.xml 在位数 / extracted=0 | **D5/#19 修复前**：双侧同为零提取 → `EQUIVALENT`（行为对齐）；修复后引擎出批注面 → 标书账本升 extracted 并接定位器（M2） |
| （引擎无：脚注尾注） | components「脚注尾注」 | 同上 | 同上（#19） |
| （引擎无：文本框条款块） | `Block.kind: Para`（旧实现在流内） | **D4 修复前**：旧实现 txbxContent 嵌套段成独立块入流、引擎 0/2 → 差分 `ENGINE_MISSING`；旧实现宿主段截断怪癖（`docx.rs:206-208`）同案 `OLD_BUG` | D4（P1）修复后：引擎出 txbxContent 文本 → 直连块流；「宿主段截断」随旧实现删除消失 |
| （引擎无：修订删除 delText） | components「修订删除」 | **修复前**：旧实现「条件性泄漏」（T2 发现：delText 前置 w:t 则入 text）→ 差分判 `OLD_BUG`；引擎侧不泄漏 → 修复前该面引擎**更合规** | #17 修复后：delText 进登记面（不入正文），标书账本 present/extracted 对齐 |
| `ParsedFile.name/sha256/bytes` | 同名 | `FileMeta` 由门面供给（sha256 标书侧算——引擎不做文件级指纹） | — |
| `ParsedFile.pages/tables/has_text_layer/ocr_blocks` | 同名 | docx 面：pages=0 / has_text_layer=true / ocr_blocks=0 / tables=引擎表格数（旧实现 `w:tbl` 计数对齐） | — |
| `ParsedFile.parse_summary` | 状态行 | "engine-docx @ <rev>｜警告 N 条｜部件账本 M 面"（rev 从 `baseline/MANIFEST.json` 的 `BASELINE_MDCORE_REV` 口径取） | — |

## 2. xlsx：`EngineBook` → `ParsedFile`

| 引擎字段 | bid 字段 | 转换规则 | 缺失时怎么办 |
|---|---|---|---|
| `SheetData.name` | `Block.locator` 前缀 | `"<sheet名> 行<N>"`（旧实现口径对齐） | — |
| `SheetData.values` | `BlockKind::XlsxRow` 每行一块；`cells` | 行序直连；空格=空串 | — |
| `SheetData.formulas` | `Block.text` 取**值**；公式入 components「公式」note（双通道，D1b 口径） | `formulas.used_cells()` 计数入账本；公式文本进 note（旧实现公式双通道指纹对齐） | — |
| `SheetData.merges` | （消费面：D1a/X3） | merges 透传——X3 探针实证引擎 API 生效（B3 图纸目录 A1:F1 读出） | — |
| `Book.warnings` | parse_summary + 账本 note | 同 docx | — |

## 3. pdf：`EnginePdf` → `ParsedFile`

| 引擎字段 | bid 字段 | 转换规则 | 缺失时怎么办 |
|---|---|---|---|
| `Outcome::ExtractedText.pages` | `BlockKind::PdfLine` | 页文本按行切分（`\n`）；`locator`="第N页 行M"；**行内空白归一**后与 `pdftotext -layout` 对照——P1 定谳：非空白字符比 1.000，差异全为列对齐空白 → 归一后 `EQUIVALENT` | — |
| （引擎无：布局元数据） | — | 引擎不产 layout 列位信息；标书不新增消费（条款抽取按文本正则，不依赖列位） | — |
| `any_text_layer == false`（全页零文本） | `has_text_layer=false` | OCR 分支路由到标书 tesseract 补遗通道（**映射层只判型不跑 OCR**——OCR 不在引擎面，`docs/engine-gap-requests-m0.5.md` §P3） | — |
| `ParsedFile.ocr_blocks` | 同名 | 引擎面恒 0；OCR 件由补遗通道单列（D5a/D5b 基线口径不变） | — |

## 4. 拒绝面与安全门（旧 guard → 引擎三门）

| 旧实现（guard.rs） | 引擎对应 | B3/B4 处置 |
|---|---|---|
| 64MB 实读门 / 压缩比 100 / 条目数守恒 / 路径穿越拒 / 实体炸弹拒 / EOCD | office-io 容器三门（lie-header 诚实判据 + 绝对上限） | B4 删自研 guard 前逐项差分：S7 定谳 ratio 门引擎**无**（合法 149:1 引擎放行、旧实现误拒→`OLD_BUG` 面）；S8 判据差异（未声明嵌套拒面 vs 引擎 content-type 面）**标书 deny-by-default 保留**，B4 决策显式记录 |
| 敌意件 6 类基线 | 引擎三门 | `baseline/security/` 双侧对跑差分全 `EQUIVALENT` 才可删 guard（B4 前置条件） |

## 5. 双跑差分工具协议（`scripts/engine_diff.py`）

```
输入: --old baseline/parse/<file>.json（旧实现）
      --new <engine 侧适配 JSON>（B3 适配器输出；T1 阶段可用 engine_probe --full-dump 产出的引擎原生 JSON）
输出: 差分报告 JSON + 人读表
      逐块: {align, kind_old/new, text_old/new, verdict, rule}
      verdict ∈ §0 词典；rule = 命中的已知差异条目编号（A1/A2/D4/D5/D6/S7…）
对齐: ① locator 完全匹配优先；② 前缀窗口滑配（bid 判分器同源的锚滑窗思想）；③ 未配对块单列
归一: 全角/半角空白折叠、行首行尾空白、连续空白压一（列对齐差异吸收面）；
      A2 编号叠加规则单独登记（不并入通用归一——保留差分可见性）
自测: 合成注入（四类 verdict 各 ≥1 例 + UNCLASSIFIED 兜底）+ 真实对照
      （D2a/R6a–R6e 双侧真跑，T1 证据 artifacts/engine-diff/）
```
