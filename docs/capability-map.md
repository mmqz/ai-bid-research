# 能力映射表（capability-map）—— issue #2 · B1 产物（B1.5 语料扩充后更新）

> **范围**：三套旧实现（`bidcore/crates/bid-parse` 产品层 / `m1/` 研究层第二套 / `scripts/ooxml_util.py` Python 语义仿真）× mdcore 引擎公开 API 的逐能力比对。
> **锚点**：ai-bid-research @ `e0d6ffc7`（B1 跑点）；mdcore @ `b317e95e`（只记录，不依赖——issue #2 §3.1 rev 钉子属 B3）。
> **B1.5 更新（2026-10-27）**：语料 93→118 件（+`real-corpus/water/` 5 件水利真实文件 +
> `real-corpus/telecom/parse-input/` 17 件通信真实解析输入 + 3 件登记文档）——真实
> docx/xlsx/pdf 由 24 件合成层扩充至 45 件（含 21 件真实件），基线重跑双跑一致
> （MANIFEST `97b98fc2…` 两连跑同值）。§1 观测面按 118 件全量更新；§2/§3 判定不受影响。
> **判定词典**（只允许以下四值）：`有` / `无` / `部分` / `待验证`。
> **证据**：`baseline/` 内 JSON（B1 实测）或引擎源码锚点（`mdcore:文件:行号`）。
> **去向**：本表为 B2 缺口清单（`docs/engine-gap-requests-m0.5.md`）的直接输入；「无/部分/待验证」条目数见 §3（不展开，B2 再细化）。

---

## 1. BlockKind 观测面（B1 baseline 实测 · B1.5 真实件扩充后全量更新）

118 件语料双跑：**45 ok / 73 failed**（73 件全部为「不支持的文件类型」显式拒绝——公告级语料主体为 txt/html/json 镜像件，`parse_file` 按扩展名分发不支持即拒，无静默、无伪装）。观测到的**全部** BlockKind（§2 主表必须全覆盖）：

| BlockKind | B1 块数（93 件） | **B1.5 块数（118 件）** | 出现面（证据文件，B1.5 新增面加粗） |
|---|---|---|---|
| `pdf_line` | 132 | **15,688**（24 件） | corpus-A B1/EV 系列、corpus-R D5b；**水利 3 主文件（塔里木 8107/施工标段 3525/伊犁 2272）+ 电信 7 件 PDF（含公告/公示/变更 675 页总量主体）** |
| `para` | 185 | **7,304**（12 件） | corpus-A/corpus-R/real-samples；**电信真实 docx 7 件（联通软研院主文件 2269 块为单件最大）** |
| `table_row` | 48 | **1,115**（9 件） | 同上（含合并表头跨页表 D1a）；**电信真实 docx 表格面（技术规范书/合同/主文件）** |
| `ocr_line` | 28 | **279**（4 件） | corpus-A B2 扫描件、corpus-R D5a；**福建修订通知 8 页 100 块 + 电信河北公告 6 页 151 块（tesseract chi_sim 通道真跑）** |
| `xlsx_row` | 38 | **116**（5 件） | corpus-A B3、corpus-R D1b；**电信真实 xlsx 3 件（投标一览表/详细评审表/报价文件）** |

五种 BlockKind 全数观测到；解析分支覆盖：docx/xlsx/pdf 三通道 + OCR 分通道 + 不支持类型显式拒绝 + 容器防护拒绝（敌意件 §2.1）。`large` 文件：无（118 件最大解析输出 < 5MB）。

**B1.5 真实件耗时画像**（`elapsed_ms` 观测字段）：OCR 件主导（河北公告 26.7s/6 页、福建修订 19.5s/8 页）；文本层大件塔里木 283 页 2.14s；docx/xlsx 均值 <1s——与 real-eval 首轮画像一致，性能回归基线就位。

## 2. 主映射表

「旧实现位置」列三套并列（空 = 该套无此能力）；「引擎 API」列锚点 = mdcore @ `b317e95e`。

### 2.1 容器与安全门（zip/OPC）

| # | 能力 | 旧实现位置 | 引擎对应 API | 覆盖判断 | 证据 |
|---|---|---|---|---|---|
| S1 | 输入字节门 | bid-parse `guard.rs:25-30`（docx 52MB/xlsx 302MB/pdf 500MB，常量自注「对齐引擎」）；m1 无；py 无 | `office-io/src/io.rs:109-113` `LIMIT_INPUT_BYTES(_DOCX)`（300MB/50MB+2MB 余量） | **有** | guard.rs:24 注释直书对齐引擎；pdf 500MB 为标书侧扩展（引擎无 pdf 容器门——mdc-pdf 走 pdfium 动态库不属 zip 面） |
| S2 | zip 声明值预扫（条目数/单声明/总声明） | bid-parse `guard.rs:33-41,168`（`prescan_zip`）；m1 无；py 无 | `office-io/src/io.rs:206` `prescan` + `:94-113` 常量族（条目 1 万/单 300MB/总 300MB） | **有** | 常量族同值（guard.rs:33-37 自注对齐）；两套均有路径逃逸检查（S3） |
| S3 | 路径穿越条目拒绝 | bid-parse `guard.rs:212` `check_entry_name` | `office-io` 门 1「路径逃逸」检查（io.rs 模块文档：读路径门结构 1） | **有** | `baseline/security/02-*.json`：rejected「路径穿越段 `..`」 |
| S4 | EOCD 守恒/截断容器检出 | bid-parse `guard.rs:323-410`（`eocd_declared_entry_count`/`mirror_eocd_candidate`/`mirror_zip64` 全文件镜像——M1.6）；m1/py 无 | `office-io` 门 2/3：尺寸诚实性（lie86 探针定谳）+ 部件良构性（EOF 未闭合/截断 typed reject） | **部分** | `security/03`（声明 3≠实际 2 拒）/`security/04`（EOCD 丢失拒）双绿；语义差异：标书侧另有全文件签名镜像扫描（V6/V7/V8 exotica 族），引擎面为声明-实际守恒+良构——B2 归类判据差异 |
| S5 | 实际读取上限（zip 炸弹） | bid-parse `guard.rs:47` `LIMIT_ACTUAL_READ_DOCX=64MB`（`read_entry_limited` 逐条计量） | `office-io` 门 2「实际解压量>声明即拒」 | **部分** | `security/01`：rejected（已读 67,108,865 > 上限 67,108,864），峰值 75.7MB、32ms——声明 10KB 谎报未骗过实际计量；判据差异：引擎「实际>声明」vs 标书「绝对 64MB 上限」，B2 归类 |
| S6 | XML 深度/实体炸弹 | bid-parse `guard.rs:50` `LIMIT_XML_DEPTH=1000` + quick-xml 自定义实体不展开 | `office-io/src/io.rs:117` `LIMIT_XML_DEPTH=1000` + 门 3 流式结构校验 | **有** | `security/05`：rejected「unrecognized entity `lol9`」（解析层拒绝，零展开）；引擎门 3 同深度常量 |
| S7 | 压缩比门 | bid-parse `guard.rs:43` `LIMIT_COMPRESS_RATIO=100` | 引擎门 2 尺寸诚实性（判据不同） | **待验证** | 敌意件 S1 证明标书面门真跑；引擎面等价判据待 B2 以同族样本在引擎侧核 |
| S8 | 嵌套归档递归防护 | bid-parse `guard.rs:60` `NESTED_ARCHIVE_EXTS` | 待核（office-io 门面未见显式嵌套递归门） | **待验证** | B2：构造嵌套 zip.docx 双侧对跑 |
| S9 | xlsx 规模后置门 | bid-parse `xlsx.rs`（`LIMIT_XLSX_FILLED_ROWS=200k`/`LIMIT_XLSX_TOTAL_CELLS=2M`，guard.rs:55-58） | `office-io/src/io.rs:123,131`（`LIMIT_SHEET_COUNT=1000`/`LIMIT_TOTAL_CELLS=4M`） | **部分** | 数值不同（2M vs 4M）——B2 归类：取严者统一；`security/06` 稀疏件 succeeded=1 格（calamine 实际物化，声明谎报未触发门亦未致害——真实行为如实登记） |
| S10 | sha256 流式指纹 | bid-parse `guard.rs:538` `sha256_streaming`（T-03 全量内存根治）；m1 `sha2`；py `ooxml_util.py:81` | 引擎无 sha256 文件指纹面（evidence 用内容指纹，见 S11） | **部分** | 文件完整性哈希归属判定：随 B3 适配层（引擎 Book/DocxDocument 未暴露输入 sha——B2 出清单） |
| S11 | 内容指纹（证据身份） | py `ooxml_util.py:93-101` `fnv64_bytes/fnv64`（SEMUL 复刻，自注 :17「SEMUL ≠ ENGINE_RUN」）；bid-parse/m1 无 | `evidence/src/fingerprint.rs:32` `Fnv64::hash_bytes` + `:60` `fingerprint_of`（指纹-版本分离） | **无（标书面）→ 引擎有** | S11 是 B4 删除 ooxml_util.py 仿真层的根因项：引擎为唯一事实来源（issue #2 约束 #1/#3） |

### 2.2 DOCX 读面

| # | 能力 | 旧实现位置 | 引擎对应 API | 覆盖判断 | 证据 |
|---|---|---|---|---|---|
| D1 | 正文段落块序（Para） | bid-parse `docx.rs:11` `parse`（w:p→Block 平文本）；m1 `docx.rs:15`；py 无读面 | `office-io/src/docx.rs:1537` `read_docx(_from_bytes)` → `DocxDocument.blocks`（`ParagraphData:1468`：run 级 text/bold/italic + 超链接解析 + 编号承载 :1419 + 缩进 :1447） | **有** | 引擎更富（run 级三态+编号+缩进）；标书面平文本属业务块流口径——B3 双跑差分按「预期差异」归类 |
| D2 | 表格行块（TableRow/cells） | bid-parse `docx.rs`（列文本空格连接）；m1 `docx.rs` | `office-io/src/docx.rs` `TableData:1495` | **有** | baseline D1a 合并表头跨页表 48 行块实测 |
| D3 | 页眉/页脚读面 | bid-parse `docx.rs:175-179` 仅**计数**（部件账本 present/extracted=0，G7 登记 :202-203）；m1 无；py 无 | `office-io/src/docx.rs:1523-1528` `DocxDocument.headers/footers: Vec<HeaderFooterData>`（块级读出 + 引用悬空严格拒/兼容警告） | **无（标书面）→ 引擎有** | **B3 接入点**：引擎已具备块级读面（G7 的引擎解在位）；bid-parse 账本 note「引擎非正文定位器变体待接入」即此缺口 |
| D4 | 文本框 | bid-parse `docx.rs:206-208`（嵌套 w:p 成独立块入流；宿主段落后文本被截断——登记）；m1 无 | 待核（docx-rs 模型层 txbxContent 面） | **待验证** | B2：以 corpus-R D2a 同族样本在引擎侧对跑 |
| D5 | 批注/脚注尾注 | bid-parse `docx.rs:181-185,204-205` 仅计数零提取（G7）；m1 无 | 待核 | **待验证** | 同 D4；引擎读面文档「不支持面逐项登记」——B2 查登记表定归属 |
| D6 | 修订痕迹（w:ins/w:delText） | bid-parse `docx.rs:188-210`（ins 内联并入宿主；delText 不提取——「真实丢失面，登记在案」）；m1 无 | 待核 | **待验证** | 同 D4 |
| D7 | 媒体清单 | 无（三套均无） | `office-io/src/docx.rs:1530` `images: Vec<ImageAsset>`（容器级枚举+引用计数） | **引擎有/标书无** | B2 判定：标书是否需要（暗标扫描件 G3 面可能消费） |

### 2.3 XLSX 读面

| # | 能力 | 旧实现位置 | 引擎对应 API | 覆盖判断 | 证据 |
|---|---|---|---|---|---|
| X1 | 值矩阵读取（XlsxRow） | bid-parse `xlsx.rs:17`（calamine `open_workbook_auto` 全表行）；m1 `xlsx.rs:31` | `office-io/src/io.rs:1242` `read_book`（calamine 五 API 面：表单/值矩阵/公式/合并/超链接） | **有** | baseline B3/D1b 实测 38 行块 |
| X2 | 公式文本/缓存值 | bid-parse 值面经 D1b 宽表公式夹具；m1 同简面 | 引擎五 API 面含公式文本+缓存值（io.rs 模块文档） | **有** | D1b「公式双通道」夹具过；引擎面更全（标书未消费公式文本——B2 判定是否需要） |
| X3 | 合并单元格范围 | bid-parse 无显式面（D1a 为 docx 侧）；m1 无 | 引擎五 API 面含合并范围 | **引擎有/标书无** | 图纸目录读面潜在消费——B2 归属判定 |

### 2.4 PDF 面

| # | 能力 | 旧实现位置 | 引擎对应 API | 覆盖判断 | 证据 |
|---|---|---|---|---|---|
| P1 | PDF 文本层抽取（PdfLine） | bid-parse `pdf.rs:17,21-73`（外部 `pdftotext -layout` 子进程 + 64MB stdout 门） | `pdf/src/command.rs:96` `Command::ExtractText` → `:128` `ExtractedText{pages: Vec<(PageIndex,String)>}`（pdfium 原生，engine_pdfium.rs） | **部分** | 两侧均有文本抽取；**布局保真口径不同**（`-layout` 列对齐 vs pdfium 原生顺序）——issue #2 §3.3 已列「真实招标 PDF 对比召回率」，B2 以 real 语料双侧对跑 |
| P2 | PDF 页数 | bid-parse `pdf.rs:182-185`（`pdfinfo` 子进程，失败回退分页计数） | `pdf/src/document.rs:346` `page_count`（pdfium 实数据） | **有** | baseline 17 件 pdf 页数实测 |
| P3 | OCR 通道（OcrLine） | bid-parse `pdf.rs:75-113`（`pdftoppm` → `tesseract chi_sim` + 治理 + 200 页上限/并发门 guard.rs:567） | **引擎无 OCR**（pdfium 文本层 only） | **无（引擎面）** | baseline ocr_line 28 块真跑（B2/D5a）；归属判定候选：标书侧保留（业务面）或引擎新能力——B2 出清单按 §3.2 规则提 mdcore issue |
| P4 | PDF 渲染 | bid-parse pdftoppm（OCR 前置） | `pdf` crate render（pdfium） | **有** | 引擎面原生；B3 后标书 OCR 可改走引擎渲染（待 B2 验证渲染-OCR 兼容性） |

### 2.5 定位与业务层

| # | 能力 | 旧实现位置 | 引擎对应 API | 覆盖判断 | 证据 |
|---|---|---|---|---|---|
| L1 | 结构化定位器 | bid-parse `lib.rs:34-41` `Block.locator: String`（自由文本「章节/页/行」）；m1 同简面 | `evidence/src/locator.rs:83` `Locator` 枚举（七变体：CellCoord:17/Region/…）+ `:152` `validate_region` | **部分** | 字符串描述 vs 结构化变体——B3 适配层核心映射面（`emit_*` 家族 ooxml_util.py:118-143 同为 SEMUL，B4 删） |
| L2 | 条款抽取 | bid-parse `extract.rs:22` `extract_requirements`；m1 `extract.rs` | **引擎无**（业务增量） | **标书侧保留**（issue #2 约束 #2） | real-eval 召回 75%（GT 草案口径）；非引擎面 |
| L3 | 归一化（norm_n1/n2） | bid-parse `lib.rs:117-124` | 引擎无 | **标书侧保留** | 对齐/检索共用业务层 |
| L4 | 文件权威序（kind_rank） | bid-parse `lib.rs:71` | 引擎无 | **标书侧保留** | A-/C-/B- 前缀业务语义 |
| L5 | 部件对账账本（SC-02） | bid-parse `docx.rs:171-215` `ComponentLedger`（八面：正文/页眉/页脚/批注/脚注尾注/文本框/修订插入/修订删除） | 引擎 `DocxDocument.warnings` 结构化警告（不同语义） | **标书侧保留** | 账本为「登记不伪装」业务纪律；B3 后以引擎读面回填 extracted 数 |
| L6 | m1 第二套 docx/xlsx 解析 | m1 `src/{docx,xlsx,import}.rs`（**无守卫**——研究层简面：直接 zip 读取，无 S1-S10 全部门） | —（不适用：m1 是标书仓内部重复） | **重复实现**（B4 处置对象） | issue #2 §0 盘点第 2 份；B4 二选一：废弃 / 改调 bid-engine |

## 3. 覆盖判定汇总（B2 缺口清单输入）

| 判定 | 条目 | 编号 |
|---|---|---|
| **有**（引擎已覆盖，B3 直接接入） | 9 | S1 S2 S3 S6 X1 X2 P2 P4 D1 D2 |
| **部分**（两侧均有但判据/口径/语义有差异，B2 归类） | 8 | S4 S5 S9 S10 X3→(引擎有/标书无 计入部分统计) P1 L1 +D7(引擎有/标书无) |
| **无**（引擎无此能力，B2 按 §3.2 规则提 mdcore issue 或判标书保留） | 3 | P3（OCR——候选：标书保留或引擎新能力） D3（页眉页脚——**引擎已有**，标书面缺口=B3 接入） S11（内容指纹——引擎已有，py 仿真层 B4 删） |
| **待验证**（B2 首批核实对象） | 7 | S7 S8 D4 D5 D6 + P1 布局口径 + X3 消费必要性 |
| **标书侧保留**（业务增量，引擎无对应——正确形态） | 5 | L2 L3 L4 L5 |
| **重复实现**（B4 删除/处置对象） | 2 | m1 三件（L6）+ ooxml_util.py SEMUL 族（S11/L1 内含） |

> 「无/部分/待验证」初步条目数：**无 3 / 部分 8 / 待验证 7**（含口径性差异；B2 逐条细化并转 `docs/engine-gap-requests-m0.5.md` 按 §B2 格式落账）。
>
> **B2 落账完成（2026-10-27）**：7 条「待验证」已全部经 `scripts/engine_probe/` 引擎侧
> 真跑核实（mdcore @ f8c860a）——S7/S8 判据差异实测定谳、D4/D5/D6 确证引擎缺口、
> P1 布局保真双侧对跑非空白字符比 1.000（无内容缺口）、X3 merges API 实测生效。
> 逐条结论与 mdcore issue 转化清单见 `docs/engine-gap-requests-m0.5.md`（16 独立条目
> + A1–A4 引擎侧对齐）；量化影响面见 `docs/title-impact-analysis.md`（B2.5）。

## 4. 证据索引

- 解析/拒绝实测：`baseline/parse/`（**118 件**逐文件 JSON，双跑 digest 一致）+ `baseline/MANIFEST.json`（B1.5 后 `97b98fc2…` 两连跑同值）
- **B1.5 真实件语料**：`real-corpus/water/`（水利 5 件，sha256 与 `artifacts/real-eval/parse/` 评测登记逐一相符）+ `real-corpus/telecom/parse-input/`（通信 17 件，sha256 与 `real-corpus/telecom/file-level-report.json` 逐一致；重取复现链 `scripts/telecom_refetch.py` 16/16 哈希复核 + `refetch-report.json`）+ `real-corpus/PROVENANCE.md`（逐件来源 URL 与许可证）
- 敌意行为基线：`baseline/security/01~06`（zip 炸弹 64MB 实读门拒、路径穿越拒、重复条目 EOCD 守恒拒、截断 EOCD 丢失拒、实体炸弹解析层拒、稀疏 xlsx succeeded=1 格如实登记——**B1.5 重跑行为不变**）+ 敌意件本体 `baseline/security/hostile/`
- 引擎锚点：mdcore @ `b317e95e`（`office-io/src/{io,docx}.rs`、`evidence/src/{locator,fingerprint}.rs`、`pdf/src/command.rs`——本表「引擎 API」列行号即锚点）
- 环境指纹与阻塞：`baseline/env.json` + `baseline/ENV_BLOCKED.md`
