# R7 跨仓集成契约审计：bid-engine ↔ mdcore（@ f4328ebf）

- 审计代理：B3（Task 4-b3）· ai-bid-research 全仓审计 V2
- 标书仓：main@6105beb（audit/v2-full-coverage）；引擎仓：mdcore-audit 克隆 main@6322f86，锁定依赖 SHA **f4328ebf4f936e1321a2254382d59f9ab9f94b7b**（`git show f4328ebf:<path>` 只读取证，工作区未动）
- 取证三源：bidcore/Cargo.lock + logs/01-bidcore-metadata.json + logs/02-bidcore-tree.txt
- 关联文档：audit/modules/module-audit-bid-engine.md（ENG-### 逐文件审计，本文件引用其结论）

---

## 1. 三个 git 依赖锁定核对表（Q1）

| 包 | 声称（Cargo.toml bid-engine:33/36/39） | Cargo.lock 实测（行号） | tree.txt / metadata.json | 判定 |
|---|---|---|---|---|
| mdc-office-io | rev=f4328ebf… | `?rev=f4328ebf…#f4328ebf…`（:1341） | 同 SHA（tree :95 / metadata） | ✅ 一致 |
| mdcore | rev=f4328ebf… | 同（:1365） | 同 SHA（tree :357 / metadata） | ✅ 一致 |
| mdc-pdf | rev=f4328ebf… +features=engine-pdfium | 同（:1355） | 同 SHA（tree :338 / metadata） | ✅ 一致 |
| mdc-formula（office-io 传递引入） | — | 同（:1331） | 同 SHA（tree :265 / metadata） | ✅ 单源纪律成立 |

**结论：三个（实为四个）git 依赖全部解析到同一 SHA f4328ebf，零漂移。** cargo tree 与 metadata 无第二 mdcore checkout。

**但代码内自报 rev 漂移**：`bid-engine/src/lib.rs:78` `MDCORE_REV = "aa0c859d…"`（#110 时点值），bump 提交 85c6074 只改 Cargo.toml 未同步常量——生产 `parse_summary` 溯源指向错误引擎提交（详细证据与修复方案见 ENG-001，此处登记为 INTEG-001）。另 Cargo.toml:20 注释「3204342d → 本 rev 共 87 提交」实测为 **180** 提交（3204342d→aa0c859d=87，aa0c859d→f4328ebf=93）。

## 2. bid-engine → mdcore 真实调用面清单（Q2）

全部调用点经 Read 逐行核实；右侧为 f4328eb 引擎源在场核实（无幽灵 API）：

| API | 调用点 | 用途 | f4328eb 定义处 |
|---|---|---|---|
| `mdc_office_io::docx::read_docx_from_bytes` | reader.rs:231 | docx 读链（门0 52MB→门1 prescan_docx→门2+3 谎言头/良构→rels→docx-rs→联接提取） | docx.rs:1709（gate 链 docx.rs:1749-1760 实证） |
| `docx::{DocxError::Container, Block, ParagraphData, TableData{rows,merges,source,nested}, TextBoxData, CommentData, HeaderFooterData, HeaderFooterVariant, BlockSource, MergeRange, RunData{revision_inserted}, ImageAsset}` | reader.rs:100-250 / types.rs | 类型镜像搬运 | docx.rs:1426-1673 |
| `mdc_office_io::io::read_book_from_bytes` + `ReadError::{PrescanRejected,ContainerCorrupt,InvalidPart,…}` | reader.rs:257（错误分类 :63-74） | xlsx 读链（门0 302MB→prescan→GridScan/calamine 五面+限额） | io.rs:1283（gate 链 :1289-1297 实证） |
| `io::{SheetData::{name,values,formulas,formula_at,merges}, Book, CompatibilityWarning, WarningKind×5}` | reader.rs:262-299 | 值/公式（绝对坐标联接 :276）/合并镜像 | io.rs:1119-1148/1216-1219/275-294 |
| `mdc_pdf::engine_pdfium::PdfiumEngine::{open, open_with_library}` | reader.rs:309-315（env MDC_PDFIUM_LIB→编译期候选→回退） | pdfium 装载（S3-L7 1GiB 输入门 + S3-L4 打开预算内建） | engine_pdfium.rs:96/115 |
| `mdc_pdf::engine::DocumentEngine::{page_count, extract_text}`（trait 静态调用） | reader.rs:321/327 | 页数权威 + 页粒度文本 | engine.rs:25/36 |
| `mdc_pdf::page::{PageSpan::new, PageIndex}` | reader.rs:322-326 | 全文档页跨（0..n-1） | page.rs:26/113/132 |
| `mdcore::markdown::markdown_to_tree` | pages.rs:136 | markdown/纯文本→块树（零自研解析） | markdown.rs:103 |
| `mdcore::pagination::{estimate, PaginationParams, PageEstimate}` | pages.rs:34-35/137 | 分页预估 v2（误差带自报） | pagination.rs:316/104-153 |
| `mdc_office_io::style::{read_docx_styles_from_bytes, StyleBundle{tree,theme,warnings}, StyleTree::{style_ids,effective_format,doc_defaults}, ThemeColors::lookup, EffectiveFormat, DocxStyleError::Container}` | styles.rs:93-149 | 样式模板读取（三门在引擎侧） | style.rs:1002/262/313/244/409/1144 |

**镜像类型零泄漏纪律**：`PaginationParams`/`PageEstimate`/`EffectiveFormat`/`StyleBundle` 均经 `BidPageEstimate`/`EngineStyleFormat`/私有 bundle 承载封死在 bid-engine 边界（pages.rs:37-41、styles.rs:59-67）——引擎类型不出接触面，承诺兑现。

**本地重复实现核查（Q2 后半）**：docx/xlsx 无（B4 commit 50e6332 删自研解析，cargo tree 无 zip/quick-xml/calamine 于 bid-parse 产品面）；`compress_grid_to_tc`/章节三助手/`cell_to_string` 为口径对齐与字段搬运（有注释依据，非引擎能力平行实现）；**pdf 面例外**：poppler 旧路径 + tesseract OCR 为登记的保留面（映射表 §3），`bid-parse/src/pdf_text.rs` 纯 Rust 最小 PDF 提取器是**第三条本地解析实现**（双引擎全断兜底，理由成文：补遗不静默丢失——见 INTEG-005 登记口径）。

## 3. 类型转换对照表（Q3：丢字段/语义漂移）

### 3.1 docx/xlsx/pdf 读面（EngineX 镜像）

| 引擎类型（f4328eb 字段全集） | 镜像承载 | 丢字段 | 语义漂移 | 严重度 |
|---|---|---|---|---|
| `TableData{rows, merges, source, nested}` | `EngineTable{rows(压回 w:tc 形), source}` | **merges、nested** | 栅格→w:tc 压缩是有意口径对齐（P2-2，注释完备）；压缩后合并结构不可逆 | P2（ENG-002） |
| `ParagraphData{style, heading_level, fields_dropped, runs, numbering{num_id,ilvl,kind,format}, indent{left,right,special}, footnote_refs, source}` | `EngineParagraph{…, has_numbering:bool, —, —, …}` | numbering 明细、indent | 无 | P3（ENG-003） |
| `RunData{text,bold,italic,hyperlink,revision_inserted}` | text + revision_inserted 文本收集 | bold/italic/hyperlink | 无 | P3 |
| `CommentData{id,author,date,anchor_block_idx,blocks}` | 同名字段 + texts（仅段落） | 批注内 Table 块 | 无 | P3（ENG-004） |
| `FootnoteData{id,blocks}` / `endnotes` | `footnotes: usize` / `endnotes: usize` | 内容块 | 无（账本登记设计） | P3 |
| `ImageAsset{part_name,content_type,bytes,references,body_uses}` | `{name, bytes:len, content_type}` | 字节本体/references/body_uses | content_type `unwrap_or_default()`→空串（引擎 None=无声明） | P3 |
| `SheetData{name,values,formulas,merges,hyperlinks,typed}` | `{name,values,formulas(对齐 values 行列),merges(1基)}` | hyperlinks、typed | merges 0基→1基换算注明；formula_at 绝对坐标联接正确（reader.rs:263-282 注释澄清引擎注释「同包围盒」不成立之误） | P3 |
| `DocxDocument{blocks,headers,footers,images,comments,footnotes,endnotes,warnings}` | 全字段镜像 | — | — | ✅ |
| 错误面 `DocxError`/`ReadError`/`DocxStyleError` | `EngineError{EngineRejected,EngineFailed,MappingGap}` | — | 分类对应正确（Container/三安全变体→Rejected；其余→Failed）；`RevisionDelete` 警告 code 字符串过滤消费不受 #39 新增 SkippedElement 影响（reader.rs:199-205 预判注释兑现） | ✅ |

### 3.2 Locator/BlockId/Revision/Anchor（证据定位面）

| 面 | 标书侧 | 引擎侧（f4328eb） | 契约状态 |
|---|---|---|---|
| Locator 序列化 | `bid_domain::locator::EngineLocator`（7 变体镜像） | `mdc-evidence/src/locator.rs::Locator` | **逐字节一致 CONFIRMED**（7 变体/字段名/serde tag=kind+snake_case/CellCoord 裸串——本审计逐行比对 + bid-domain 字面 JSON 钉死测试 :298-357）。声明锚定 rev 2d01444，f4328eb 复核仍一致。 |
| Locator 解析行为 | `CellCoord::parse` 不 trim | 引擎先 `trim().to_ascii_uppercase()` | 微漂移（仅解析，序列化不受影响）→ INTEG-004 |
| BlockId 稳定锚 | **未消费**（无转换点——locator.rs:11-12 自记「后续批次接线」） | `mdcore::anchor::{Anchor(BlockId,offset), Selection, TextTarget::ByAnchor, AnchorError}`（编辑/结构变化后可解析 + expect 并发保护） | 能力在库未接 → INTEG-006 |
| Revision | 未消费 | `mdcore::revision::RevisionLog`（op 日志/jsonl） | 账本登记（mdcore::op 等） |
| 证据定位验证 | `service.rs::locate_content/verify_locator_against`：对 `ParsedFile` 复算（DocxParagraph{paragraph_index,fnv64 指纹}、PdfPageText{page,指纹}；其余 5 变体 fail-closed 显式报错） | 引擎 sidecar 三态（定位器/复算/审核）未接 | 双台账归并语义登记待定（lib.rs:46 自记） |

**跨仓往返结论**：EngineLocator 序列化语义与 mdcore 一致（jsonl/serde roundtrip 成立）；但 bid-engine 目前**没有任何**引擎 Locator 实例 ↔ 镜像的运行时转换点——「逐字节一致」只是数据形态契约，互认通道尚未建（设计登记，非缺陷）。

## 4. f4328eb → main(6322f86) 漂移与未消费能力清单（Q4）

**提交距离：89 commits**（`git log f4328ebf..6322f86 --oneline | wc -l`）。

### 4.1 被调用 API 漂移核查（逐文件 diff 实证）

| 引擎文件 | f4328eb→6322f86 diff | 对 bid-engine 消费面影响 |
|---|---|---|
| office-io/src/io.rs | **零 diff** | 无 |
| mdcore/src/pagination.rs | 零 diff | 无 |
| mdcore/src/markdown.rs | 零 diff | 无 |
| pdf/src/engine.rs | 零 diff | 无 |
| pdf/src/page.rs | 零 diff | 无 |
| office-io/src/style.rs | +21/−8：写面预置字符样式 Strong/Emphasis（PRESET_CHAR_STYLE_IDS）、define_style 重名拒扩、**测试期望变更** | 读面（style_ids/effective_format/lookup）签名与语义零变更；`style_ids()` 输出序在引擎**自产**文档上多 2 预置——只影响读引擎新版写出的模板，无破坏 |
| pdf/src/engine_pdfium.rs | +31/−1：库缺失错误消息加可操作指引（#211） | 签名零变更；EngineFailed 文本更长 |
| office-io/src/docx.rs | +809/−26：**全部为写面加性**（TocEntry、RunSpec.char_style、field_pageref/char_styled/heading_bookmarked/toc_entries、#196 样式视觉面、#190 页眉页脚/图片导出）——读面结构体（ParagraphData/DocxDocument/Block/TableData/CommentData…）与 read_docx_from_bytes 零改动（diff 删除侧仅测试与写面注释） | 无（85c6074 提交所述「纯加性零破坏」与本次 diff 复核一致） |

**结论：被调用的 10 组 API 在 f4328eb→main 之间零破坏、零语义变化。升级到 main 的技术风险低（读面冻结）；按纪律须独立 bump PR + 基线差分重跑（尚未发生）。**

### 4.2 未消费能力清单（引擎已交付、标书侧未接）

**f4328eb 已在场未接**（lib.rs:38-53 账本登记，本审计核实模块在场）：`mdcore::{fingerprint, word_count, anchor, revision, chunk, history, op, snapshot, id, inline, html}`；`office-io::{docx_edit（#155 批注/脚注/域编辑、#150 set_field_value）, numbering（read_docx_numbering）, write（StyleDef 写面）, rebuild, xlsx_chart, xlsx_condfmt}`；`mdc-evidence`（sidecar 三态台账）；`mdc-formula`；`mdc_pdf::{organize, annotation/annotate, links, layers, ocr(ocr_page 中英混识), forms, stamp, signature, security_scan}`。

**main 新增未消费**（89 commits 摘录）：#177 xlsx 原位编辑引擎（单元格级读改写+部件透传保真）；#190 Markdown 图片行接活内嵌 PNG + 页眉页脚六槽位导出；#196 Heading 样式视觉面 + rStyle 公开面；#203 合并格注释勘误（「左上单点显示」语义钉定）；#204 带图 docx 黄金夹具 + #170 图片正向对账缺口守卫；#205 逐键 O(doc) 增量 fence 缓存；#217 keymap（与标书无关）。**对标书最有价值的三项**：xlsx 原位编辑（报价表在位改写）、导出图片/页眉页脚面（补 docx-rs 导出链短板——标书导出链至今不嵌图）、图片对账守卫。

**消费方冻结现状**：pages/styles 门面交付即冻结（零生产消费方，ENG-007）；f4328eb bump 动机「标书仓用的引擎编辑不了自己生成的 docx（#159 sdt）」解决的是 **mang-shell**（mdcore 仓内应用）编辑链，标书仓自身不消费 docx_edit。

## 5. 引擎侧防护归属核查（Q8：zip/EOCD/xlsx——threat-model T-03 重核）

**历史宣称**（docs/threat-model.md:32 T-03 行）：「已实施：bid-parse/guard.rs 挂门——输入字节/prescan 三门/压缩比（乘法判据）/实际读取累计计量/EOCD 守恒门（V6 尾垃圾/V7 影子 EOCD/V8 字段分歧三绕过已封）/条目名拒绝/XML 深度/xlsx 读后行/单元格双轴门；负向测试 18 项全绿」。

**当前现实**（B4 commit 50e6332 收敛后）：bid-parse/src/guard.rs:4-18 自述「zip 面（prescan 声明值门/条目名检查/实际读取门/EOCD 守恒门/XML 深度门/xlsx 后置限额）**无消费方**——OOXML 容器安全全链收敛到引擎侧」；guard.rs 现仅剩：pdf 输入字节门（500MB）、流式 sha256、子进程治理（T-04）。**历史 T-03 所述那套 zip-exotica 防线在标书仓已不存在。**

**引擎侧 f4328eb 实测防线**（office-io/src/io.rs + docx.rs 逐行取证）：

| 防 | 引擎实装（f4328eb） | 位置 |
|---|---|---|
| 门 0 输入字节 | xlsx 302MB（LIMIT_INPUT_BYTES）/ docx 52MB（LIMIT_INPUT_BYTES_DOCX）/ pdf 1GiB（pdf security.rs Limits） | io.rs:109-111 / docx.rs:1694-1709 / pdf security.rs:229 |
| 门 1 prescan | 条目数 ≤10,000、单声明 ≤300MB（xlsx）/50MB（docx）、路径逃逸拒（enclosed_name）、总声明限 | io.rs:206-262（prescan_family） |
| 门 2 尺寸诚实性 | `CountingReader` 逐字节累计，实际>声明即 lied（zip 8.6 读界无内建强制的策略层补面——谎言头闭环） | io.rs:322-360 |
| 门 3 良构 | quick-xml 标签配对/根唯一/截断/**深度 ≤1000**/控制字节登记 | io.rs:362-420（validate_xml_structure） |
| 稀疏炸弹 | GridScan 坐标流式扫描（稠密包围盒 ≤4,000,000 格，先于 calamine；1588 字节→550GB 分配实证闭项）+ 表数 ≤1,000 | io.rs 模块注 :67-80 |
| EOCD | zip 8.6 `ZipArchive::new` 失败→`ContainerCorrupt`（「EOCD/中央目录不可读」）；测试 eocd_corrupt_is_container_error + corpus_harness mal6e | io.rs:145-146/1522 |
| docx 全链 | 门0→prescan_docx→verify_gates→rels→content-types→模型（含 run 级图片引用计数） | docx.rs:1749-1760 |

**缺口（引擎侧无等价物，原 bid 侧已删）**：① EOCD 守恒镜像族（V6 尾部垃圾出窗/V7 影子 EOCD/V8 双计数字段分歧/zip64 重复条目）——引擎依赖 zip crate 8.6 自身行为，**未对这些绕过族做策略层封堵**（corpus_harness 无对应负向样本）；② 压缩比门（>100:1 拒）——引擎无（声明限+谎言头门以不同机制覆盖常规炸弹向量）；③ 重复条目吸收防御（zip crate 同名条目语义）——引擎无显式检查；④ 嵌套归档载具拒——无；⑤ xlsx 读后行/单元格双轴门——被声明面 GridScan 前置门替代（机制不同：防稀疏炸弹 vs 防真实巨表，后者 4M 格/1000 表上限覆盖）。

**核查结论**：威胁面基本盘（输入限/声明限/谎言头/深度/稀疏炸弹/良构）在引擎侧真实存在且强于原 bid 侧（GridScan 稀疏炸弹闭环是 bid 侧从未有的）；**但 T-03 历史宣称的 zip-exotica 三绕过封堵与压缩比门在两仓当前代码中均不存在**——其防御前提（zip 4.6.1 语义）已随引擎改用 zip 8.6 而变化，zip 8.6 下这些绕过族是否可复现**未经任何测试验证**。登记 INTEG-002（P2，NEEDS_RUNTIME_VERIFY）。

## 6. 集成问题记录

---

ID: INTEG-001
严重程度: P2
模块/文件路径/代码行号: bidcore/crates/bid-engine/src/lib.rs:78（MDCORE_REV）；Cargo.toml:19-20 注释
函数或结构体: `MDCORE_REV` 常量
问题描述: 三依赖锁 SHA 一致（f4328ebf×4，§1 表）但**代码内自报 rev 漂移**：常量停留 aa0c859d，bump 提交 85c6074 未同步；Cargo.toml 注释提交数 87 实为 180。
调用链: mapping.rs 三映射函数把 MDCORE_REV 写入 parse_summary → service.rs 落库 → 对外溯源口径。
触发条件: 全部引擎路径解析。
实际影响: 跨仓归因/差分复现指令指向错误引擎提交；「parse_summary 溯源 rev」承诺（mapping_docx.rs:87-91 断言）因断言同源而失效。
复现步骤: 对比 lib.rs:78 与 Cargo.lock:1341/1355/1365。
当前测试: contains(MDCORE_REV)（同源自证，抓不到）。
缺失测试: lock rev ↔ 常量一致性断言。
建议方案: 改常量 + 补一致性测试 + 修注释。
证据:
```rust
pub const MDCORE_REV: &str = "aa0c859d956597992c2d93e9055e2f0beca017c3";  // lib.rs:78
mdc-office-io = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", … }  // Cargo.toml:33
```
审计置信度: CONFIRMED
状态分类: 当前仍存在（= ENG-001，R7 视角登记）

---

ID: INTEG-002
严重程度: P2
模块/文件路径/代码行号: docs/threat-model.md:32（T-03 行历史宣称）↔ bidcore/crates/bid-parse/src/guard.rs:4-18（现状）↔ mdcore f4328ebf crates/office-io/src/io.rs:206-262/322-420
函数或结构体: T-03 zip 防线归属
问题描述: B4 收敛后，T-03「IMPLEMENTED」宣称的挂点（bid-parse/guard.rs zip 面 18 负向测试、EOCD 守恒镜像 V6/V7/V8、压缩比乘法判据、重复条目吸收防御、xlsx 读后双轴门）整体不存在；引擎侧等价类防线在场（§5 表）但 zip-exotica 封堵与压缩比门**两仓皆无**。
调用链: import（docx/xlsx 引擎路径）→ office-io 门 0/1/2+3 → zip 8.6 crate。
触发条件: 构造尾部垃圾/影子 EOCD/字段分歧/高压缩比声明谎报的敌意 OOXML。
实际影响: 若 zip 8.6 保留 4.x 家族的可绕过语义，敌意件可绕过 prescan 声明值（但门 2 谎言头/门 3 深度/GridScan 仍构成二道防线，实际可达内存/CPU 面待验证）。
复现步骤: 以 baseline/security/hostile 族（01-zip-bomb/03-duplicate-entries 等）+ V6/V7/V8 构造件跑 `parse_docx_via_engine`/`parse_xlsx_via_engine` 观察分类与资源。
当前测试: 引擎侧 eocd_corrupt_is_container_error、corpus_harness mal6e、稀疏炸弹 spike；**无 V6/V7/V8 族负向**。标书侧 baseline/security/hostile 夹具在库但 B3 面测试只跑截断容器（reader_docx.rs:134-148）。
缺失测试: zip 8.6 语义下 exotica 族负向回归（引擎侧或集成侧）。
建议方案: ① 把 V6/V7/V8/重复条目/压缩比五个负向样本移植到 bid-engine 集成测试（读分拒/资源界）；② 或引擎侧 io.rs 补 EOCD 守恒策略层；③ 更新 threat-model T-03 行为「引擎侧等价类 + 残余登记」如实状态。
证据:
```rust
// guard.rs:4-7（现状自述）
//! B4 收敛（issue #11 任务 3）：docx/xlsx 自研解析删除后，**zip 面**
//! （prescan 声明值门 / 条目名检查 / 实际读取门 / EOCD 守恒门 / XML 深度门 /
//! xlsx 后置限额）无消费方——OOXML 容器安全全链收敛到引擎侧
// threat-model.md:32（历史宣称）
//! **EOCD 守恒门 crate 4.6.1 选择语义全文件镜像**（V6 尾垃圾出窗/V7 影子
//! EOCD/V8 字段分歧三绕过已封 + …）… | 已实施
```
审计置信度: CONFIRMED（防线缺位事实）/ NEEDS_RUNTIME_VERIFY（zip 8.6 残余可绕过性）
状态分类: 历史已修复但缺乏当前证据（T-03 宣称坐标失效；等价类防护在引擎、exotica 族防护无处）

---

ID: INTEG-003
严重程度: P3
模块/文件路径/代码行号: bidcore/crates/bid-kb/tests/kb_probe.rs:13-25（同款路由注释）↔ bid-tools/src/service.rs:366-379
问题描述: kb/corpus 探针 pdf 臂恒走旧 poppler 路径，service.rs 是引擎优先+回退——注释宣称「同款路由」与实现不符（详 ENG-008）。
实际影响: 检索索引校准源与生产解析源理论可分（P1 比率 1.000 缓解）。
建议方案: 注释改口径或真对齐。
审计置信度: CONFIRMED
状态分类: 文档与实现不一致

---

ID: INTEG-004
严重程度: P3
模块/文件路径/代码行号: bidcore/crates/bid-domain/src/locator.rs:62-91 ↔ mdcore f4328ebf crates/evidence/src/locator.rs:24-49
函数或结构体: `CellCoord::parse`
问题描述: EngineLocator serde 形态与引擎逐字节一致（CONFIRMED，§3.2），但镜像 parse 不 trim 输入（引擎 `s.trim().to_ascii_uppercase()`）：`" C7 "` 引擎 Ok("C7")、镜像 Err。LocatorError 镜像 2/4 变体（已声明）。另：bid-engine 无引擎 Locator↔镜像运行时转换点（locator.rs:11-12 登记「后续批次接线」），「逐字节一致」仅为形态契约；service.rs 证据定位对账只支持 DocxParagraph/PdfPageText 两变体，其余 5 变体 fail-closed。
实际影响: 低（形态一致已满足互认地基；行为漂移仅影响宽松输入接受度）。
建议方案: 镜像 parse 补 trim 对齐；转换点接线时一并处理。
审计置信度: CONFIRMED
状态分类: 当前仍存在（登记型）

---

ID: INTEG-005
严重程度: P3
模块/文件路径/代码行号: bidcore/crates/bid-parse/src/lib.rs:8-16（pdf 保留面+pdf_text.rs 兜底）↔ bid-tools/src/export.rs:12-35（docx-rs 直写）↔ service.rs:1301-1325（soffice 转 pdf）
问题描述: 导入/导出/回读链不对称（设计登记）：**读**=mdcore（docx/xlsx）+pdfium/poppler/纯 Rust 三降级（pdf）；**写**=docx-rs 0.4.22（引擎 write.rs/docx_edit 未消费）+soffice headless（mdc-pdf generate/organize 未消费）。pdf_text.rs 是引擎外的第三条本地 PDF 文本解析实现（双引擎全断兜底——理由成文：补遗新控制价/截止时间不静默丢失）。往返保真依赖三条独立实现的一致性（e2e 12_export-check 十项断言 + engine-diff 差分族兜底），无引擎级往返保证。
实际影响: 中低（e2e 锚点在位；docx-rs 写→mdcore 读的往返已被 mang-shell #159 sdt 修复案例证明存在历史盲区，f4328eb 已含修复）。
建议方案: 保持（有登记）；导出链如需图片/页眉页脚六槽位/样式模板，引擎 main 已交付对应写面能力（§4.2）。
审计置信度: CONFIRMED
状态分类: 当前仍存在（设计取舍登记）

---

ID: INTEG-006
严重程度: P3
模块/文件路径/代码行号: bid-tools/src/service.rs:133-184（locate_content/verify_locator_against）↔ mdcore f4328ebf crates/mdcore/src/anchor.rs:1-60
问题描述: 证据锚（paragraph_index/page + fnv64 指纹）在源文档编辑/合并/重导后**fail-closed 失效**（索引漂移或指纹不符即拒），无重锚定/模糊复绑；引擎 `mdcore::anchor` 的 BlockId 稳定寻址（结构变化后仍可解析 + expect 乐观并发 + AnchorMissing 显式拒）未被标书侧消费。mdcore locator 源注自认「§四.5：润色后允许失效——人工重绑定路径在状态机」——失效是两端共识语义，但标书侧的人工重绑面未见实现。
触发条件: 招标文件补遗/修订后再导入 + 既有证据登记复用。
实际影响: 证据链在文档演化后需人工重登记（诚实拒绝，无静默错引——安全性成立，可用性受限）。
建议方案: 若该场景高频，评估消费引擎 anchor/BlockId（须先定双台账归并语义——lib.rs:46 登记的同一前提）。
审计置信度: CONFIRMED
状态分类: 当前仍存在（设计边界登记）

---

ID: INTEG-007
严重程度: P3
模块/文件路径/代码行号: 引擎仓整体（f4328ebf..6322f86 = 89 commits）
问题描述: 引擎落后 main 89 提交；被调 API 零破坏（§4.1 逐文件 diff 实证），升级技术风险低；但按 issue #2 §3.1 纪律须独立 bump PR + 基线差分重跑，且 main 含对标书高价值未消费能力（xlsx 原位编辑 #177、导出图片/页眉页脚 #190、图片对账守卫 #204）。
建议方案: 计划性 bump（先跑 mapping_* 黄金锚 + engine_diff 差分脚本确认零漂移）；图片对账守卫与标书导出链短板（不嵌图）同批评估。
审计置信度: CONFIRMED
状态分类: 当前仍存在（流程登记）

---

## 7. 汇总

| 项 | 结论 |
|---|---|
| 三依赖同 SHA | ✅ f4328ebf ×4（含传递 mdc-formula），lock/tree/metadata 三源一致；❌ 代码内 MDCORE_REV 常量漂移（INTEG-001/ENG-001 P2） |
| 真实调用面 | 10 组 API 全部实证在场且被真实调用；无 Mock/无空壳依赖；无 docx/xlsx 平行实现（pdf 三降级链为登记保留面） |
| 类型转换 | docx Table{merges,nested} 丢弃（ENG-002 P2）+ 若干 P3 有损点；EngineLocator serde 与引擎逐字节一致（CONFIRMED），无运行时转换点 |
| 引擎漂移 | f4328eb→main 89 commits，被调 API 零变更（diff 实证）；升级风险低、bump 未做（INTEG-007） |
| 防护归属 | 引擎侧等价类防线真实在场（门0/1/2+3+GridScan）；T-03 历史宣称的 zip-exotica 封堵/压缩比门两仓皆无（INTEG-002 P2，NEEDS_RUNTIME_VERIFY） |
| P0/P1 | 无 |
