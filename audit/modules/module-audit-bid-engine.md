# 模块审计 B3：bid-engine 逐文件审计（含 mdcore 跨仓核对）

- 审计代理：B3（Task 4-b3）· ai-bid-research 全仓审计 V2
- 冻结基线：main@6105beb（分支 audit/v2-full-coverage，工作树 HEAD a689701——audit 提交，业务代码与 6105beb 一致）
- 引擎侧参照：mdcore @ f4328ebf4f936e1321a2254382d59f9ab9f94b7b（Cargo.lock 实测锁定 SHA；mdcore-audit 克隆 main@6322f86 仅作漂移对照，锁定内容一律 `git show f4328ebf:<path>` 读取）
- 审计方式：只读。逐文件 Read + mdcore f4328eb 源比对 + Cargo.lock/依赖树/metadata 三源核对 + git 历史（-S pickaxe）溯源。
- 审计日期：2026-11-05（沙箱会话）

## 0. 覆盖率

| 项 | 数 |
|---|---|
| 应覆盖（git ls-files bidcore/crates/bid-engine） | 15 文件（6 src + 7 tests + 1 example + Cargo.toml） |
| 实覆盖 | 15/15（100%） |
| mdcore f4328eb 侧核对文件 | office-io/docx.rs、io.rs、style.rs；mdcore/pagination.rs、markdown.rs、anchor.rs、revision.rs、lib.rs；pdf/engine_pdfium.rs、engine.rs、page.rs、security.rs；evidence/locator.rs；office-io/Cargo.toml |
| 逐文件行数核对 | lib.rs 134 / mapping.rs 531 / reader.rs 346 / types.rs 289 / pages.rs 180 / styles.rs 163（与任务书一致） |

## 1. 模块定位（事实）

bid-engine 是标书仓**唯一** mdcore 引擎接触面（issue #2 §1 约束 1）。三层结构：

- `reader.rs`：`EngineReader` trait + `MdcoreReader` 唯一实装——直连 `mdc_office_io::docx::read_docx_from_bytes` / `mdc_office_io::io::read_book_from_bytes` / `mdc_pdf::engine_pdfium::PdfiumEngine` + `DocumentEngine`；
- `mapping.rs`：EngineX → `bid_parse::ParsedFile` 字段级映射（章节上下文/表行发射/部件账本 SC-02）；
- `pages.rs`/`styles.rs`：#110 任务 2 接线的 `mdcore::pagination` / `mdc_office_io::style` 门面（**零生产消费方**，见 ENG-007）；
- `lib.rs`：三通道门面 `parse_docx_via_engine` / `parse_xlsx_via_engine` / `parse_pdf_via_engine` + `MDCORE_REV` 溯源常量。

**真实调用方**（全仓 grep `bid_engine::`）：`bid-tools/src/service.rs:369/372/376`（import 路由，pdf 引擎优先+旧路径回退）、`bid-tools/src/bin/real_eval.rs:33-37`、`bid-parse/src/lib.rs`（仅文档提及，非代码依赖）、`bid-parse/examples/realfiles.rs`、`bid-parse/tests/corpus_probe.rs`、`bid-kb/tests/kb_probe.rs`（后三者 docx/xlsx 走引擎、pdf 走旧路径——与 service.rs 路由不完全一致，见 ENG-008）。`pages.rs`/`styles.rs` 公开符号全仓零消费。

## 2. bid-engine → mdcore 真实调用面（代码证据）

| # | API（mdcore @ f4328eb） | 调用点 | 用途 | f4328eb 在场核实 |
|---|---|---|---|---|
| 1 | `mdc_office_io::docx::read_docx_from_bytes` | reader.rs:231 | docx 全链读取（三门→rels→模型→联接） | docx.rs:1709 ✅ |
| 2 | `docx::{Block, ParagraphData, TableData, TextBoxData, CommentData, HeaderFooterData, HeaderFooterVariant, BlockSource, MergeRange, ImageAsset, RunData, DocxError, FootnoteData}` | reader.rs:100-250 / types.rs 镜像 | 类型搬运 | docx.rs:1426-1673 ✅（字段逐一比对，见 §4） |
| 3 | `mdc_office_io::io::read_book_from_bytes` + `ReadError`（PrescanRejected/ContainerCorrupt/InvalidPart）+ `SheetData::{values, formulas, formula_at, merges, name}` + `Book` + `CompatibilityWarning`/`WarningKind`（5 变体） | reader.rs:63-74/257-301 | xlsx 读取+错误分类 | io.rs:1283/139/1119/275 ✅（WarningKind 5 变体逐一核对） |
| 4 | `mdc_pdf::engine_pdfium::PdfiumEngine::{open, open_with_library}` | reader.rs:309-315 | pdfium 引擎打开（env → 编译期候选） | engine_pdfium.rs:96/115 ✅（含 S3-L7 输入字节门 1GiB + S3-L4 打开预算） |
| 5 | `mdc_pdf::engine::DocumentEngine::{page_count, extract_text}` + `mdc_pdf::page::{PageSpan, PageIndex}` | reader.rs:320-328 | 页粒度文本抽取 | engine.rs:17-36 / page.rs:26/113/132 ✅ |
| 6 | `mdcore::markdown::markdown_to_tree` + `mdcore::pagination::{estimate, PaginationParams, PageEstimate}` | pages.rs:34-35/135-137 | 分页预估 | markdown.rs:103 / pagination.rs:104-153/316 ✅ |
| 7 | `mdc_office_io::style::{read_docx_styles_from_bytes, StyleBundle, StyleTree::{style_ids, effective_format, doc_defaults}, ThemeColors::lookup, EffectiveFormat, DocxStyleError}` | styles.rs:33/93-149 | 样式模板读取 | style.rs:1002/262/274/313/409/244/1144 ✅ |

**镜像不消费的引擎面**（lib.rs:38-53 能力账本登记 + 本审计核实 f4328eb 在场）：`mdcore::{fingerprint, word_count, anchor, revision, chunk, history, op, snapshot, id, inline}`、`office-io::{docx_edit, numbering, rebuild, write, xlsx_chart, xlsx_condfmt}`、`mdc-evidence` 包、`mdc-formula` 包、`mdc_pdf::{organize, annotation/annotate, links, layers, ocr, forms, stamp, signature, security_scan}`——各有登记理由（工作范围/口径变更成本/无消费用例），账本与代码一致，未见「登记未接实则偷接」或反向。

## 3. 三 git 依赖锁定核对（Q1）

| 包 | Cargo.toml 声明 rev | Cargo.lock 实测 source（行号） | metadata.json | 依赖树 02-tree.txt |
|---|---|---|---|---|
| mdc-office-io | f4328ebf… | `git+…?rev=f4328ebf…#f4328ebf…`（Cargo.lock:1341） | 同 SHA | :95 同 SHA |
| mdcore | f4328ebf… | 同（Cargo.lock:1365） | 同 SHA | :357 同 SHA |
| mdc-pdf | f4328ebf…（features=engine-pdfium） | 同（Cargo.lock:1355） | 同 SHA | :338 同 SHA |
| mdc-formula（传递） | — | 同（Cargo.lock:1331） | 同 SHA | :265 同 SHA |

**结论：四个 mdcore 家族条目全部钉在同一 SHA f4328ebf，无漂移。** calamine 0.36.1（与 office-io 同版，Cargo.toml:42 注释属实）。注意工作区双 zip 版并存：zip 4.6.1（bid-engine dev-dep——styles 测试自构夹具）与 zip 8.6.0（mdc-office-io 引擎侧）——dev/prod 面分离，非缺陷，登记备查。

## 4. 类型转换丢字段核对（Q3——镜像面逐字段比对）

引擎 f4328eb 类型 → bid-engine 镜像，**真实丢弃字段清单**（代码证据）：

| 引擎字段（f4328eb） | 镜像承载 | 丢弃面 | 严重度 |
|---|---|---|---|
| `TableData::nested: bool`（docx.rs:1621，#39 内嵌表标记） | **无**（reader.rs:118-129 只搬 rows+source） | 嵌套表（费率表族）与直系表不可区分；账本/消费方无法分流 | **ENG-002 P2** |
| `TableData::merges: Vec<MergeRange>` | 仅在 `compress_grid_to_tc`（reader.rs:154-178）内部消费后**弃**；`EngineTable` 无 merges 字段 | 镜像面合并结构不可及（types.rs:110 注释仍写「X3 定谳：扁平无合并结构」——已过时） | ENG-002 P2 |
| `ParagraphData::numbering: Option<ParagraphNumbering>`（num_id/ilvl/kind/format 四字段） | `has_numbering: bool`（reader.rs:106） | 编号明细丢失（ilvl/numFmt 语义面） | ENG-003 P3 |
| `ParagraphData::indent: Option<ParagraphIndent>` | **无** | 缩进全丢 | ENG-003 P3 |
| `RunData::{bold, italic, hyperlink}`（docx.rs:1426-1437） | **无**（只取 text + revision_inserted） | 行内格式/超链目标丢 | ENG-003 P3 |
| `CommentData::blocks` 内 `Block::Table` | `EngineComment.texts` 只收段落（reader.rs:218-225） | 批注内表格内容丢 | ENG-004 P3 |
| `FootnoteData::blocks`（docx.rs 脚注/尾注内容） | 仅计数 `footnotes/endnotes: usize`（reader.rs:248-249） | 脚注内容丢（账本登记设计） | ENG-004 P3 |
| `ImageAsset::{bytes 内容, references, body_uses}`（docx.rs:973-991） | `bytes: usize`（长度）+name+content_type（reader.rs:240-244） | 图片字节/引用关系/正文使用计数丢 | ENG-004 P3 |
| `SheetData::{hyperlinks: Vec<Hyperlink>, typed: CellGrid}`（io.rs:1124-1127） | **无** | xlsx 超链/类型化值丢 | ENG-004 P3 |

**Locator/BlockId/Revision/Anchor 跨仓转换**：bid-engine **不存在**与 mdcore `evidence::Locator` / `mdcore::anchor::{Anchor(BlockId,offset), Selection, TextTarget}` / `mdcore::revision::RevisionLog` 的任何转换点——标书侧证据锚是 `bid_domain::Anchor{file,kind,locator:String,excerpt}`（字符串呈现层）+ `bid_domain::locator::EngineLocator`（serde 镜像）。EngineLocator 与引擎 `Locator` 在 f4328eb **逐字段一致**（7 变体/字段名/`#[serde(tag="kind", rename_all="snake_case")]`/CellCoord 裸串——本审计逐行比对引擎源 + bid-domain 字面 JSON 钉死测试 locator.rs:298-357），序列化语义跨仓往返成立；两处微小漂移：引擎 `CellCoord::parse` 先 `trim().to_ascii_uppercase()`（evidence/locator.rs:24-25），镜像不 trim（locator.rs:63-91）——仅解析行为差，不影响序列化；`LocatorError` 镜像只保留 2/4 变体（已注释声明）。引擎 `anchor.rs` 的 BlockId 稳定寻址（编辑/合并后仍可解析 + expect 乐观并发）**未被消费**——标书锚在编辑后靠指纹 fail-closed 拒绝（见 INTEG-006）。

## 5. 问题记录

---

ID: ENG-001
严重程度: P2
模块/文件路径: bidcore/crates/bid-engine/src/lib.rs
代码行号: 74-78（`MDCORE_REV` 常量及其文档注释）
函数或结构体: `pub const MDCORE_REV: &str`
问题描述: 溯源常量停留在 `aa0c859d956597992c2d93e9055e2f0beca017c3`，而 Cargo.toml/Cargo.lock 实际 rev 已是 `f4328ebf4f936e1321a2254382d59f9ab9f94b7b`。bump 提交 85c6074（"chore(dep): bump mdcore to f4328ebf"）只改 Cargo.toml 三处 rev 字符串，未同步 lib.rs（`git show 85c6074 -- …/src/lib.rs` 为空；`git log -S 'aa0c859d' -- …/lib.rs` 仅命中引入提交 5264940）。Cargo.toml:20 注释「旧 rev 3204342d → 本 rev 共 87 提交」同样失实——实测 `git log 3204342d..f4328ebf` = **180** 提交（87 属于 3204342d→aa0c859d 段，aa0c859d→f4328ebf 为 93）。
调用链: `docx_to_parsed_file`/`xlsx_to_parsed_file`/`pdf_to_parsed_file`（mapping.rs:282/371/433）把 `MDCORE_REV` 写进 `ParsedFile.parse_summary` → 落库 TenderFile（service.rs:389）→ 成为「哪版引擎解析」的对外溯源口径。
触发条件: 任何一次引擎路径解析（全量生产路径）。
实际影响: 生产 parse_summary 自报引擎 rev=aa0c859d 而实际跑 f4328eb——跨仓问题归因/差分审计/复现指令全部指向错误提交；lib.rs:74「与 Cargo.toml git 依赖一致」的文档承诺被打破。
复现步骤: `rg aa0c859d bidcore/crates/bid-engine/src/lib.rs` vs `rg 'rev = ' bidcore/crates/bid-engine/Cargo.toml`；或任取引擎路径导入产物看 parse_summary。
当前测试: mapping_docx.rs:87-91、mapping_xlsx.rs:74-78 断言 `parse_summary.contains(MDCORE_REV)`——**断言与被验值同源（都用过时常量），结构上不可能抓到此漂移**（bump 后测试照常绿正是证明）。
缺失测试: 锁文件 rev ↔ `MDCORE_REV` 常量一致性断言（如 build.rs 读 lock 或测试解析 Cargo.lock）。
建议方案: ① `MDCORE_REV` 改为 `f4328ebf…`；② 补一条测试：编译期/测试期从 Cargo.lock 提取实际 git rev 与 `MDCORE_REV` 全等断言（防再犯）；③ 修正 Cargo.toml 注释的提交数叙述。
证据:
```rust
// lib.rs:76-78
/// #110 任务 2：3204342d → aa0c859d（87 提交——消费面公开 API 零破坏，
/// 差分验证见 PR 描述）。
pub const MDCORE_REV: &str = "aa0c859d956597992c2d93e9055e2f0beca017c3";
// Cargo.toml:33
mdc-office-io = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", package = "mdc-office-io" }
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: ENG-002
严重程度: P2
模块/文件路径: bidcore/crates/bid-engine/src/reader.rs · types.rs
代码行号: reader.rs:118-129（convert_block Table 臂）、154-178（compress_grid_to_tc）；types.rs:110-117（EngineTable 及其注释）
函数或结构体: `convert_block` / `EngineTable`
问题描述: 引擎 f4328eb 的 `TableData` 已是 `{rows(栅格展开), merges(矩形清单), source, nested(#39 内嵌表标记)}` 四字段（docx.rs:1608-1626），镜像只搬 `rows`（经栅格压回 w:tc 形）与 `source`——`nested` 与 `merges` 在镜像面**永久丢弃**。嵌套表（引擎按文档序在含它的外层表之后以独立 `Block::Table(nested=true)` 发射，docx.rs:3293-3342/3433）与直系表在镜像/映射层不可区分：mapping.rs:150-154 对两者一视同仁递增 `table_idx`（表N 连号）。
调用链: `read_docx` → `convert_block` → `docx_to_parsed_file` → `BlockKind::TableRow`（locator「表N 行M」）→ extract/检索/证据锚。
触发条件: 任何含嵌套表的 docx（招标件费率表族——B3-3 r3 验收明确其为实质条款区）。
实际影响: ① 消费方无法把嵌套表从表序中分流（账本/定位/类别映射粒度受损）；② 外层表中被嵌套表占据的单元格为空，若整行为空则该行被 emit_rows 跳过（mapping.rs:106）——行位与「表N 行M」语义在嵌套场景下与 Word 视图存在结构性偏差（内容不丢——在内层表块中）；③ types.rs:110 注释「X3 定谳：扁平无合并结构」与现实（merges+nested 双字段）不符，误导后续维护者。
复现步骤: 构造含 w:tbl@w:tc 嵌套表 docx → `parse_docx_via_engine` → 检查 `EngineBlock::Table` 无任何途径得知 nested/merges。
当前测试: mapping_docx.rs D1a 锚点覆盖合并表头栅格压回；**无嵌套表镜像/映射测试**（引擎侧 corpus_harness 有，标书侧 B3-3 r3 验收报告 #39 归零靠差分脚本，非单测）。
缺失测试: 嵌套表件的镜像断言（nested 透传后）+ 表序/行位口径锚。
建议方案: `EngineTable` 增 `nested: bool` 与 `merges: Vec<(usize,usize,usize,usize)>`（加性扩面，零破坏）；mapping 层先在账本登记「嵌套表」面（present/extracted），分流语义留给消费方决议；同步修正 types.rs 过时注释。
证据:
```rust
// reader.rs:118-129（引擎四字段只搬两字段）
docx::Block::Table(t) => EngineBlock::Table(EngineTable {
    rows: compress_grid_to_tc(&t.rows, &t.merges),   // merges 用后即弃
    source: map_source(t.source),                     // nested 未搬运
}),
// 引擎 f4328eb docx.rs:1613-1621
pub merges: Vec<MergeRange>,
pub source: BlockSource,
pub nested: bool,
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: ENG-003
严重程度: P3
模块/文件路径: bidcore/crates/bid-engine/src/reader.rs
代码行号: 102-117（Paragraph 臂）
函数或结构体: `convert_block`
问题描述: 段落/run 级字段有损镜像：`ParagraphNumbering`（num_id/ilvl/kind/format）塌缩为 `has_numbering: bool`（:106）；`ParagraphIndent`（left/right/special）整字段丢；`RunData::{bold, italic, hyperlink}` 丢（镜像只收 text 与 revision_inserted）。属「按用例收窄镜像」的设计取舍（条款抽取只用文本面），但 numbering 的 ilvl/numFmt 是引擎 r4-t3 语义解析成果，镜像面不可及意味着未来条款编号消费须再扩面。
调用链: 同 ENG-002。
触发条件: 含多级编号/缩进/超链的 docx。
实际影响: 当前抽取面无消费点（B6 numbering.rs 在 bid-parse 用文本形态识别，不消费此面）——潜在能力损失，非现症。
当前测试: reader_docx.rs 扩面字段测试覆盖 heading_level/fields_dropped/footnote_refs/has_numbering 布尔面；无 numbering 细节断言。
缺失测试: 无（按需）。
建议方案: 若 B6 条款编号链后续需要结构证据，扩 `EngineParagraph.numbering: Option<EngineNumbering>` 镜像；否则保持并在 types.rs 注明「有损点」。
证据: reader.rs:106 `has_numbering: p.numbering.is_some(),`；引擎 docx.rs:1443-1474 `ParagraphNumbering { num_id, ilvl, kind, format }`。
审计置信度: CONFIRMED
状态分类: 当前仍存在（设计取舍登记）

---

ID: ENG-004
严重程度: P3
模块/文件路径: bidcore/crates/bid-engine/src/reader.rs
代码行号: 212-227（convert_comment）、237-249（images/footnotes/endnotes）、253-301（read_xlsx）
函数或结构体: `convert_comment` / `read_docx` / `read_xlsx`
问题描述: 内容级丢弃四处：① 批注内容只收段落文本，批注内表格丢；② 脚注/尾注只计数，`FootnoteData::blocks` 内容丢；③ `ImageAsset` 丢字节本体/references/body_uses（图片映射只余登记面）；④ `SheetData` 丢 hyperlinks 与 typed CellGrid。均为账本「登记不提取」设计（SC-02 note 在位），但 excerpt/证据链若要引用批注表格或图片 OCR，需回引擎。
实际影响: 低（当前消费面均不消费）；登记为镜像收窄清单。
当前测试: mapping_emits_component_ledger 断言批注 present 面；无内容面断言。
建议方案: 保持；在 types.rs 各镜像字段注释集中列「引擎另有 X 未镜像」清单（当前散落）。
审计置信度: CONFIRMED
状态分类: 当前仍存在（设计取舍登记）

---

ID: ENG-005
严重程度: P3
模块/文件路径: bidcore/crates/bid-engine/src/lib.rs
代码行号: 112-134（parse_office_via_engine）
函数或结构体: `parse_office_via_engine`
问题描述: 门面 `std::fs::read(path)` **无前置大小检查**即全量入内存，引擎字节门（docx 52MB/xlsx 302MB/pdf 1GiB）在 bytes 已在内存后才触发（office-io `check_input_size` 于 `read_*_from_bytes` 内部；引擎路径入口的 metadata 前置检查 r4-t2 只覆盖 path 形态入口）。这正是 bid-parse guard.rs T-03 曾修的「先全量读入内存型 OOM」类（guard.rs:11-13 现仅剩 pdf 面前置门+流式 sha256）。缓解：生产 import 链在 service.rs:1106-1126 有 fs_fence 前置门（单件 500MB/批 2GB/64 件，先 metadata 后读）；**未设防的是直连调用方**——real_eval.rs、docx_categories.rs example、corpus_probe/kb_probe 测试传任意路径。
触发条件: 直连门面对超大/敌意文件（>内存量）。
实际影响: 受限（生产有围栏；CLI/example 是开发者面）。
当前测试: 无。
缺失测试: 门面级超限输入拒绝测试。
建议方案: 门面读文件前 `fs::metadata` 前置门（对齐引擎 52/302MB 常量语义或 500MB 围栏口径），或改流式 sha256+有界读。
证据:
```rust
// lib.rs:116-117
let bytes = std::fs::read(path)
    .map_err(|e| EngineError::EngineFailed(format!("读取 {} 失败: {e}", path.display())))?;
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: ENG-006
严重程度: P3
模块/文件路径: bidcore/crates/bid-engine/src/mapping.rs
代码行号: 262-278（警告入账循环）
函数或结构体: `docx_to_parsed_file`（部件账本组装）
问题描述: 引擎警告入账不对称：循环第一条警告若遇末位「图片」行 note 为空，被**折叠进图片行的 note**（`continue`），第二条起才各自成行「引擎警告[code]」。后果：① 警告 #1 的归因被错挂在「图片」部件下；② 1 条 vs N 条警告的账本形态不一致；③ 图片 present=0 时也占用其 note 槽（图片行无条件 push，mapping.rs:244-253）。
触发条件: docx 有兼容警告（如 RevisionDelete/SkippedElement）且图片 note 为空。
实际影响: 账本可读性/对账口径（SC-02 面额统计）轻微失真；语义不丢（警告文本在账本内可见）。
当前测试: mapping_emits_component_ledger 只断言 9 面名存在与不变式（present>0&&extracted==0→note 非空）——本形态恰好满足不变式，测试不红。
缺失测试: 多警告件的账本行形态断言。
建议方案: 删除折叠分支，警告一律独立成行（`引擎警告[code]`）；或折叠进专门的「引擎警告汇总」行。
证据:
```rust
if let Some(last) = components.last_mut() {
    if last.component == "图片" && last.note.is_empty() {
        last.note = format!("引擎警告 {}: {}", w.code, w.message);
        continue;
    }
}
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: ENG-007
严重程度: P3
模块/文件路径: bidcore/crates/bid-engine/src/pages.rs · styles.rs · lib.rs
代码行号: pages.rs:29-33（消费方登记）、styles.rs:24-26；lib.rs:38-53（能力账本）
函数或结构体: `estimate_pages`/`check_page_limit`/`StyleTemplate`
问题描述: #110 任务 2 交付的两门面**零生产消费方**（全仓 grep：`StyleTemplate|estimate_pages|check_page_limit|bid_baseline_params` 在 bid-tools/bid-kb/bid-parse 无任何命中）。导出链（export.rs）仍用 `format_spec::baseline` 硬编码参数 + docx-rs 直写；页数限制软预警未接导出前门；样式模板读取无匹配/应用消费。pages.rs:83-90 自认「镜像关系是显式契约……跨 crate 等值断言登记在 PR 跟进项」——等值断言至今未做（本审计人工核对 210×297/35-30-28-26/12pt/1.5 与 format_spec.rs:117-124 当前一致，纯手工同步无护栏）。
触发条件: 任何导出（无页数预警）/任何固定格式件导出（格式跟硬编码 spec 不跟模板）。
实际影响: 用例价值未兑现（「超页=形式评审扣分甚至废标」的预警未上线）；lib.rs:53 自评「接了没消费点才是负债」——现状即此。属**如实登记的冻结面**（bid-tables 冻结），非隐瞒。
当前测试: pages_estimate.rs 13 测 + styles_template.rs 8 测（真引擎调用，质量良好）——只测门面自身。
缺失测试: bid_baseline_params ↔ format_spec::baseline 跨 crate 等值断言（两处手工镜像漂移无红灯）。
建议方案: ① 解冻后接 export 前预警（check_page_limit 三态→warnings/errors）；② 先补跨 crate 等值测试（bid-tools dev-dep 已含 bid-engine 方向可行）。
审计置信度: CONFIRMED
状态分类: 当前仍存在（登记在案的待接线）

---

ID: ENG-008
严重程度: P3
模块/文件路径: bidcore/crates/bid-kb/tests/kb_probe.rs · bid-parse/tests/corpus_probe.rs · bid-parse/examples/realfiles.rs
代码行号: kb_probe.rs:13-25
函数或结构体: `parse_routed`
问题描述: 注释「与 service.rs 同款路由」不实：service.rs pdf 臂是 `parse_pdf_via_engine().or_else(|_| parse_file())`（引擎优先+回退），探针是 `_ => parse_file()`（pdf 恒走旧 poppler 路径）。kb 检索索引的 pdf 文本源（poppler）与生产导入（pdfium，可得时）不同源——P1 定谳两者非空白字符比 1.000，故实际风险极低，但「同款路由」表述与代码不符，且探针的期望值校准面与生产面存在理论漂移。
实际影响: 低（1.000 比率 + 引擎失败自动回退收敛）。
建议方案: 探针注释改为「docx/xlsx 同款；pdf 固定旧路径（校准口径）」，或真对齐 service 路由。
审计置信度: CONFIRMED
状态分类: 文档与实现不一致（测试侧）

---

### 无问题核验（正面结论）

- **Q7 Mock 检查：PASS（CONFIRMED）**——bid-engine src/tests/examples 全量 grep `mock|Mock|仿真|fake|stub`：零命中。smoke_engine.rs **直调** `mdc_office_io::docx::read_docx_from_bytes` 证明 git 依赖非空壳；styles_template.rs 自构最小 OPC 夹具但引擎三门真跑；mapping_pdf.rs 引擎 vs 旧路径**双跑对照**（非替身）。SKIP 机制（fixture 缺失/pdfium 缺失）打 ENV_MISSING 横幅提前 return，符合 issue #9「不伪装失败」口径。
- 错误分类透传：`EngineRejected`/`EngineFailed` 边界与引擎 typed 错误（DocxError::Container / ReadError 三安全变体 / DocxStyleError::Container）一一对应（reader.rs:55-74、styles.rs:72-89），敌意面差分语义未改写。
- `compress_grid_to_tc`（栅格→w:tc 口径对齐）与章节三助手（as_chapter/as_subsection/is_numbered_clause）均有充分注释依据（旧口径对齐/呈现层语义），**不构成引擎能力自研平行实现**；`cell_to_string` 为 calamine Data 字段搬运（口径真源迁移自旧 xlsx.rs，B4 后唯一真源）——Q2「本地重复实现」结论：docx/xlsx 面无重复实现（B4 已删）；pdf 面保留 poppler/OCR 旧路径 + `pdf_text.rs` 纯 Rust 第三兜底（属 bid-parse 范围，有登记理由：双引擎全断时补遗不静默丢失——见 INTEG-005）。
- B3-3 r3 验收口径（EQ+ED 91.57%、A 件 100% EQ、NESTED-TABLE 归零）与 mapping_docx/mapping_xlsx 黄金锚点数值一致（182/154/28/6、15/13/6、23/24 等），锚点纪律（数值漂移必须来自显式 bump PR）在测试头注释成文。

## 6. 测试资产清单（Q7 证据面）

| 文件 | 测试数（数 fn） | 类型 | 预言源（oracle） |
|---|---|---|---|
| tests/smoke_engine.rs | 5 | 引擎直调探针 | 引擎侧已验收口径（gen-log/T17d/D2a/R6 系） |
| tests/reader_docx.rs | 9 | 镜像保真 | gen-log.txt 统计行 + expected-readback.json（引擎仓借鉴件）+ 自研结构断言 |
| tests/mapping_docx.rs | 7 | 映射黄金锚 | B4 固化常量（3204342d 时点差分验收值）+ 真实件锚（北京 2025：2741/52/2302） |
| tests/mapping_xlsx.rs | 5 | 映射黄金锚 | 同上（D1b/B3/TR2 锚） |
| tests/mapping_pdf.rs | 4 | 双跑对照 | 旧路径产物 + P1 比率容限（<5% 行差/2% 字符守恒）+ SKIP 语义 |
| tests/pages_estimate.rs | 9 | 手算锚 | mdcore pagination 模型 v2 公式手算（不抄实现输出） |
| tests/styles_template.rs | 8 | 受控夹具 | 手写 OOXML 字段直译（含引擎「rFonts 整组替换」语义钉定） |
| examples/docx_categories.rs | — | 对照工具 | #233 改前后基线（real-corpus） |

oracle 来源三分：借鉴 mdcore（TR2/R6/expected-readback）、旧路径差分固化（B4 黄金锚）、手算/手写字面值——无一处以实现输出自证（除 mapping_pdf 对照容限面，其对照对象是独立实现）。测试纪律（issue #11 双轨）执行到位。

## 7. 结论

bid-engine 实现 discipline 良好：单一接触面边界清晰、错误语义零改写、测试真引擎零 Mock、能力账本诚实。**P0/P1：无。** 两项 P2（ENG-001 溯源常量漂移、ENG-002 嵌套表/合并结构镜像丢弃）均为低修复成本的确定性问题；其余 P3 为登记型。跨仓集成核对（R7）另见 `audit/integration-contract-audit.md`。
