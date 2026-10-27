# 引擎缺口清单（engine-gap-requests-m0.5）—— issue #2 · B2 产物

> **审计基线**：mdcore @ `f8c860a`（2026-10-27 09:33 +0800 main；B1 记录的 `b317e95e`
> 已被引擎 TR2 轮超越——按 issue #2 评论 2026-10-27 01:29「B2 写缺口清单时按引擎侧编号对齐」
> 以当前 main 为锚）。ai-bid-research @ `e57cac2` + B1.5 语料（118 件基线）。
> **证据方法**：`scripts/engine_probe/`（只读 path 依赖引擎 worktree，10 探针真跑，
> 报告 `artifacts/engine-probe/report.json`）+ `artifacts/engine-probe/p1-layout-parity.json`
> （P1 双侧对跑）+ `baseline/`（118 件基线）+ `artifacts/title-impact/`（B2.5 实验）。
> **条目计数**：B1 汇总「无 3 / 部分 8 / 待验证 7 = 18」中 P1、X3 在「部分」与
> 「待验证」各计一次——本清单去重后 **16 个独立条目**（§1~§16），另加引擎侧
> 已排期对齐条目 **A1~A4**（§17，issue #2 评论 2026-10-27 01:29 指定）。
> **每条按 issue #2 §B2 格式**：标书用例 / 引擎现状 / 期望 API / 归属判定 / 验收测试。
> **判定词典**：`引擎缺口`（提 mdcore issue）/ `标书保留`（业务增量或加固层，
> 引 AD-07）/ `B3 接入点`（引擎 API 在位，标书面未接）/ `判据差异`（双侧均有、
> 判据不同，登记不缺口）/ `无缺口`（实测消除）。

---

## §1 · D4 文本框文本提取（txbxContent）【✅ 已修复并验收：mdcore#18 @ 977f4c9 → f7ddd39】

> **状态（2026-10-27 T3 验收复测）**：`Block::TextBox` 双形态（VML/DrawingML）已合入；
> 标书侧探针+差分独立验收通过（D2a text_boxes=1、宿主零截断、差分 D4 面缺口 2→0）；
> **连带勘误**：引擎发现的「bid 基线 2 块命中 = 标书侧双重提取缺陷」经差分独立确认
>（D4-DOUBLE-EXTRACT）——登记为 bid_parse 第三项实证缺陷，B4 删除旧实现时消除。见 `docs/mdcore-pr-plan.md`。

**标书用例**：`fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx`（zip 注入文本框，
条款「文本框条款：本文件所附技术参数表以最新澄清版本为准」）。bid-parse 现行为：
嵌套 `w:p` 成独立块入流（2 块，`baseline/parse/.../D2a-*.json` 实证；宿主段落后
文本被截断——缺陷登记在案）。期望输出：文本框内段落文本全部进入块流且不截断宿主段。
**引擎现状**：mdcore @ `f8c860a` `crates/office-io/src/docx.rs:150-152`
「图形/图片/形状（媒体面已暴露……块序内图片位置不入块模型）」——文本框属形状面，
**整体跳过并登记**。探针实测（`d456_corpus_r_d2a_engine`）：6 段输出中
**文本框条款 0 命中**（bid 基线 2 命中）。
**期望 API**：`DocxDocument.blocks` 增 `Block::TextBox(TextBoxData { blocks: Vec<Block> })`
或等价面（文本框内段落块入流/独立承载，宿主段落不截断）。
**归属判定**：**引擎缺口**。理由（AD-07）：文档结构解析是通用能力，引擎为唯一
事实来源；「澄清/补遗条款载于文本框」是真实招标文件形态（V1-a 已实证对 naive
遍历不可见），标书不得自建第二套 docx 解析（issue #2 约束 #1/#2）。
**验收测试**：引擎侧读 D2a 同族样本，断言文本框条款文本出现在读回块流，且
宿主段落文本完整（无截断）。

## §2 · D6 修订痕迹当前文本提取（w:ins）【✅ 已修复并验收：mdcore#17 @ c60214f → f7ddd39】

> **状态（2026-10-27 T3 验收复测）**：ins 入读回 + run 级 `revision_inserted` 标记 +
> delText 逐段 `WarningKind::RevisionDelete` 登记已合入；标书侧独立验收通过（R6a/R6c/R6e
> ins 全入读回、warnings 计数与 del 数逐件吻合 2/3/1/1、差分 D6-INS 缺口 7→0）。验收详情
> `docs/mdcore-pr-plan.md`；前后双基线证据 `artifacts/engine-probe/report{,-after-fixes-f7ddd39}.json`。

**标书用例**：同 D2a（修订插入「响应截止时间顺延至 2026-12-01 09:30」）。bid-parse
现行为：`w:ins` 内联并入宿主段（**当前生效文本**被提取），`w:delText` 不提取
（`docx.rs:188-210`）。真实语义：带修订的招标文件，`w:ins` 内容=当前生效口径——
**丢失即漏当前生效条款**（澄清截止时间是废标级条款）。
**引擎现状**：`docx.rs:161`「修订追踪（Insert/Delete/MoveFrom/MoveTo 容器——
**内容不入提取**）」。探针实测：引擎 D2a 读回「澄清四：响应截止时间口径如下——」
**修订插入内容整体缺失**（bid 基线完整）。
**期望 API**：`ParagraphData.runs` 保留 `w:ins` 内 run 文本（附 `revision_inserted: bool`
标记面）；`w:delText` 维持不提取但入 `warnings` 计数登记。
**归属判定**：**引擎缺口（P0 级）**。理由（AD-07）：同 §1；且语义上「ins=当前文本」
是 OOXML 规范语义，跳过整容器比 bid 的并入门更损内容；真实带修订采购文件导入
会静默丢当前生效条款。
**验收测试**：引擎侧读含 `w:ins` 样本，断言 ins 内文本出现在读回文本且带标记；
delText 不出现但 warning 计数≥1。

**T2 批增补（2026-10-27，专项夹具已入库）**：
- **验收夹具**：`fixtures/corpus-R/D6-revisions/`（5 变体 + `expected-readback.json` 期望读回契约，
  ECMA-376 口径：ins=当前生效文本 / delText=历史文本）——R6a 纯插入 / R6b 纯删除 /
  R6c 同段替换+表格内修订 / R6d 双批注 / R6e 组合（澄清实务形态）；
- **引擎侧「前」证据**：`report.json` d6rev_* 5 探针（2026-10-27）——ins 缺失与批注 0
  在新夹具全量复现（R6a「开标地点：」后空、R6c 同段空壳、R6e 双缺口叠加）；
- **bid 侧勘误（本条「w:delText 不提取」表述修正）**：bid_parse 对 delText 并非一律不提取——
  `capture_text` 为**粘性标志**（`w:t` 置位后至段落/单元格结束才复位，`docx.rs:85-97`），
  delText 前若有 w:t 则历史文本**泄漏进当前文本**：R6b「原资格条款：具备建筑工程施工
  总承包三级及以上资质」、R6e「澄清一：原响应截止时间…更正后…」实测泄漏；delText 居段首
  （R6c 形态）或居单元格首（无前置 w:t）不泄漏；del-only 段落整段无块。`docx.rs:210` ledger
  「未提取（不在捕获面）」应读作「**条件性泄漏**」；既有单测（`docx.rs:345`）delText 样本前置
  无 w:t，断言盲区被 R6b/R6e 实件暴露——**被废止的历史条款会被当作现行条款读出**（合规误读风险）；
- **处置**：bid_parse 不在本轮修（B3/B4 由 bid-engine 替换，引擎 #17 修复后继承正确语义）；
  现状如实登记为回归基线（`baseline/parse/fixtures/corpus-R/D6-revisions/`）。

## §3 · D5 批注/脚注/尾注读面【✅ 已修复并验收：mdcore#19 @ f7ddd39】

> **状态（2026-10-27 T3 验收复测）**：`CommentData{anchor_block_idx}` + `FootnoteData` +
> `ParagraphData.footnote_refs` 已合入；标书侧验收通过（R6d/R6e comments=2、D2a=1、
> TR2 真 Word 件=1）。电信脚注部件 2 件留作 B3 全量差分扩展对。见 `docs/mdcore-pr-plan.md`。

**标书用例**：D2a（zip 注入批注条款通道）——「批注条款」是澄清文件真实形态
（评审专家补遗常载于批注）。bid-parse 现行为：仅计数零提取（ComponentLedger
八面登记，`docx.rs:181-185`）。期望输出：批注文本与锚定段落可关联读出。
**引擎现状**：`docx.rs:162-166`「书签、批注……面外跳过」+「脚注/尾注（读面引擎
受限：docx-rs 从不读 footnotes.xml 且正文 w:footnoteReference 不入模型——
CAPABILITY_NOTES §5；写面已暴露 RunSpec::footnote）」。探针实测：TR2
word-complex.docx（真 Word 批注件）读回 0 批注文本。
**期望 API**：`DocxDocument.comments: Vec<CommentData { anchor_para_idx, blocks }>`；
`footnotes: Vec<FootnoteData { id, blocks }>`（正文引用位以 run 标记面承接）。
**归属判定**：**引擎缺口**（P2——真实频率低于 §1/§2，但 D2a 批注条款通道证明
业务可达）。理由（AD-07）：同 §1。
**验收测试**：读含批注样本断言批注文本+锚段落关联读出；含脚注样本断言
footnotes.xml 文本读回。

**T2 批增补（2026-10-27）**：专项夹具 `fixtures/corpus-R/D6-revisions/R6d-带批注澄清.docx`
（双作者双批注、commentRange 锚定、期望读回见 `expected-readback.json`）；引擎侧「前」
证据 `report.json` `d6rev_r6d_comments_engine`（批注 0 提取，正文不受批注污染）。

## §4 · S7 压缩比门【判据差异 · 实测定谳】

**标书用例**：`baseline/security/hostile/01-zip-bomb-declared-10kb-actual-1gb.docx`
（声明 10KB/实解 1GB）+ 合法高压缩比探针件（`engine_probe` 运行时构造：诚实声明
1,080,113B / 压缩 7,244B / 比率 149:1——真实招标文件重复样板段落的极端化）。
**引擎现状**：`io.rs:682,722`「actual size exceeds declared (lie header)」——
**诚实性判据**。探针实测：炸弹 rejected（`prescan rejected: actual size exceeds
declared`）；**合法 149:1 件 accepted**（6000 段全解析）。bid 侧（`guard.rs:43,197`
ratio>100 拒）：炸弹 rejected（64MB 实读门，`security/01` 基线）；**合法 149:1
件会误拒**。
**期望 API**：无（引擎判据成立，不建议引擎加 ratio 门——诚实声明+绝对上限已覆盖
炸弹面，ratio 门是启发式且实测有误报面）。
**归属判定**：**判据差异 + 标书保留**。标书 `LIMIT_COMPRESS_RATIO=100` 为
deny-by-default 加固层，保留但登记误报面（极端重复样板 docx 真实可达 >100:1）；
B4 删除自研 guard 时若复用引擎三门，此差异须显式决策（建议：保留标书面
业务加固常量，非引擎缺口）。
**验收测试**：引擎侧已有 lie-header 测试；建议补「合法高压缩比放行」回归测试
（构造比率 >100 诚实件断言 ok——`engine_probe` S7b 构造代码可直接转引擎测试）。

## §5 · S8 嵌套归档递归【判据差异 · 实测定谳】

**标书用例**：`engine_probe` 三变体（有效容器内 `word/media/attached.zip`）。
**引擎现状**：无嵌套归档门。探针实测：①未声明 content-type → rejected
（`invalid part: media part has no content type declaration`——OPC 部件校验面，
**非嵌套检测**）；②补 `Default Extension="zip"` 声明 → **accepted**（落入
`images` 媒体枚举 1 项——**字节仅枚举不递归解压**，无放大路径）；③内容级伪装
（document.xml 字节=zip）→ rejected（`no root element`——XML 良构面）。bid 侧：
`NESTED_ARCHIVE_EXTS` 按扩展名拒①②，③走 XML 解析错。
**期望 API**：无强制缺口。可选加固：`images` 枚举对嵌套归档字节加
`is_archive: bool` 标记（下游消费可见）。
**归属判定**：**判据差异 + 标书保留**。引擎无递归路径（实测），无放大风险；
标书 deny-by-default 保留。暗标扫描件 G3 面（媒体枚举消费）建议引擎补
is_archive 标记（低优先级）。
**验收测试**：引擎侧「声明齐全嵌套 zip 落 images 且零递归」登记测试
（`engine_probe` S8a2 构造可直接转）。

## §6 · P1 PDF 文本抽取布局保真【无缺口 · 实测定谳】

**标书用例**：三件真实招标 PDF 双侧对跑（`artifacts/engine-probe/p1-layout-parity.json`）：
重庆西永 32 页（文本层）/ 某水利施工标段 142 页（文本层）/ 福建修订通知 8 页（纯扫描）。
**引擎现状**：`pdf/src/command.rs:96` `Command::ExtractText`（pdfium 原生）。
实测：两件文本层件**非空白内容字符比 1.000**（27,733/38,190 与 27,970 级别全同
——0.726 总字符比差异**全部为 -layout 列对齐空白**）；10 项结构探针
（第一章/第二章/投标人须知/前附表/1.1 项目概况/评标办法/合同条款/投标文件/资格/否决）
**双侧命中集完全一致**；扫描件双侧皆零文本（引擎 empty_pages=8 = pdftotext 0）。
**期望 API**：无缺口。B3 接 `mdc-pdf` 文本层抽取替换 `pdftotext -layout` 子进程
（内容等价已证；消一个外部工具依赖）。
**归属判定**：**无缺口**。布局列对齐信息差异（-layout 空白）对标书条款抽取
无影响（抽取按行文本非视觉列）；M2+ 表格视觉重建若需要列对齐再立新条目。
**验收测试**：本探针即双侧对照验收（引擎侧可转为「文本层 PDF 非空白字符集
== 参照抽取」回归）。

## §7 · X3 合并单元格【无缺口 · 低消费必要性】

**标书用例**：`fixtures/corpus-A/files/B3-图纸目录.xlsx`（真实合并 A1:F1 标题行）
+ TR2 word-merged.docx（真 Word 纵横合并 4×3 表）+ excel-complex.xlsx。
**引擎现状**：xlsx `SheetData.merges: Vec<Dimensions>`（`io.rs:1116`）——实测
B3 件 merges=1（r1-1c1-6）**正确读出**；TR2 excel-complex 实测 merges=0，经
openpyxl 独立核证**该件本无合并**（TR-2D batch2 特性表描述不精确，勘误登记）。
docx `TableData { rows: Vec<Vec<String>> }` 扁平文本面**无合并结构**（实测
4×3 合并表读为 4×2 扁平行——设计登记）。
**期望 API**：无缺口（xlsx merges API 在位且实测生效）。
**归属判定**：**无缺口 + B3 接入点（xlsx merges 可选消费）**。图纸目录场景
消费必要性低：bid xlsx_row 扁平行块抽取不受合并影响（24 行全抽出，基线在档）；
M2+ 表格理解（合并区语义）再消费 `merges`。docx 合并结构缺失与 bid 现行为
（列文本空格连接）等价，非回退面。
**验收测试**：B3 接入时对 B3-图纸目录断言 merges 读出非空（探针已有）。

## §8 · P3 OCR 通道【标书保留】

**标书用例**：`C-福建修订通知-扫描件.pdf`（8 页 100 ocr_line）、
`B-telecom-hebei-无线网勘察设计-招标公告.pdf`（6 页 151 ocr_line，26.7s）、
corpus-A B2/corpus-R D5a——OCR 是扫描补遗/公告的唯一文本通道。
**引擎现状**：引擎无 OCR（pdfium 文本层 only，§6 实测扫描件 empty_pages=8）；
`pdf` crate render（pdfium 渲染）在位。
**期望 API**：无引擎缺口。可选后续：`render` 面输出图像流供标书 OCR 消费
（替换 pdftoppm 子进程——B2 依赖渲染-OCR 兼容性验证后再定）。
**归属判定**：**标书保留**（业务面：OCR 质量/治理/200 页门属标书纪律，
`pdf.rs:75-113` + guard 567）。
**验收测试**：N/A（标书侧已有 279 ocr_line 基线回归）。

## §9 · D3 页眉页脚条款读面【B3 接入点（引擎有 / 标书面缺口）】

**标书用例**：D2a 页眉「本页眉所示内容与正文具有同等效力」/ 页脚
「受控文件编号：CL-2026-SY-001」——页眉页脚载条款是采购文件真实形态。
**引擎现状**：`docx.rs:1523-1528` `DocxDocument.headers/footers`（块级读面）。
探针实测：D2a 读出 headers=2（默认+首页两变体文本全量）、footers=1（受控编号
条款）——**引擎面完整**。bid 侧仅计数零提取（ComponentLedger）。
**期望 API**：无缺口。B3 适配层把 headers/footers 块接入业务块流（带
`HeaderFooterVariant` 标记——first/even 槽位激活语义归消费方，引擎已透传）。
**归属判定**：**B3 接入点**（issue #2 评论 2026-10-27 00:36 已裁决改判）。
**验收测试**：B3 后对 D2a 断言页眉/页脚条款进入标书块流（探针输出即预期基线）。

## §10 · S11 内容指纹（Fnv64）【标书侧 B4 删除对象】

**标书用例**：证据身份链（`ooxml_util.py:93-101` SEMUL 复刻引擎 Fnv64——
唯一剩余的 Python 引擎语义仿真，`grep -r SEMUL` 的主命中）。
**引擎现状**：`evidence/src/fingerprint.rs:32` `Fnv64::hash_bytes` + `:60`
`fingerprint_of`（指纹-版本分离）——引擎为唯一事实来源。
**期望 API**：无缺口。B3 后标书经 `bid-engine` 调引擎指纹；B4 删
`ooxml_util.py` 仿真层（A4 验收 `grep -r SEMUL` 清空的根因项）。
**归属判定**：**B4 删除对象**（非缺口）。
**验收测试**：A4 验收即本条（`grep -r "SEMUL"` 为空）。

## §11 · S4 EOCD 守恒/截断容器【判据差异 · 标书加固层保留】

**标书用例**：`baseline/security/03/04`（重复条目/截断容器）双向拒 +
M1.6 全文件镜像 V6/V7/V8 exotica 族（`guard.rs:323-410`，EOCD 0xFFFF 不豁免）。
**引擎现状**：门 2/3（尺寸诚实性 lie86 + 部件良构性 EOF typed reject）——
`io.rs` 门面；引擎侧 87 用例敌意测试零 crash（mdcore #8 完成，B1 已记录）。
**期望 API**：可选吸收：引擎 prescan 增「EOCD 声明计数 vs 中央目录实际计数守恒」
显式判据（引擎现为良构面覆盖；标书全文件镜像更严）。
**归属判定**：**判据差异 + 标书保留加固层**。B4 删自研 guard 后若仅靠引擎三门，
exotica 族（声明欺骗变体）覆盖依赖引擎良构面——建议 mdcore issue 吸收全文件
镜像扫描（P2 优先级，非阻塞 B3）。
**验收测试**：引擎侧对 V6/V7/V8 族样本断言 typed reject（标书可提供样本生成器）。

## §12 · S5 实际读取上限（zip 炸弹）【判据差异 · 实测双侧全拒】

**标书用例**：`security/01` 炸弹件：bid rejected（已读 67,108,865 > 64MB 上限，
峰值 75.7MB/32ms）。
**引擎现状**：门 2「实际解压量>声明即拒」+ 总量 300MB。探针实测：同一炸弹件
引擎 rejected（lie header 判据）——**双侧全拒，无行为缺口**。
**期望 API**：无。
**归属判定**：**判据差异（覆盖等价）**。标书 64MB 绝对上限为业务收紧
（真实正文 XML < 30MB 注释在案），保留。
**验收测试**：双侧已有（`security/01` + 引擎 lie-header 测试）。

## §13 · S9 xlsx 规模后置门【判据差异 · 取严者已定】

**标书用例**：`security/06` 稀疏件（声明 A1:XFD1048576/实际 1 格）succeeded=1 格
如实登记（声明谎报未触发门亦未致害——calamine 按实际物化）。
**引擎现状**：`io.rs:123,131` `LIMIT_SHEET_COUNT=1000` / `LIMIT_TOTAL_CELLS=4M`；
标书 `LIMIT_XLSX_FILLED_ROWS=200k` / `LIMIT_XLSX_TOTAL_CELLS=2M`（R3-S3-3）。
**期望 API**：无。
**归属判定**：**判据差异（标书更严，双轴 vs 引擎单轴）**。标书保留后置门
（下游边界）；B3 复用引擎读面时后置门仍在标书侧执行（顺序：引擎三门 → 标书
业务门），无回退。
**验收测试**：标书侧 `security/06` 回归已有；稀疏件行为双侧登记一致。

## §14 · S10 文件级 sha256 指纹【标书保留】

**标书用例**：全语料完整性账本（`baseline/MANIFEST.json` 逐文件 sha256、
`fetch-report.json`/`refetch-report.json` 哈希复核协议、V1-b 补遗链同 sha 幂等）。
**引擎现状**：引擎无输入文件 sha256 面（evidence 指纹为内容指纹 Fnv64，
语义不同：文件身份 vs 内容身份）。
**期望 API**：可选低优先级：`read_docx_from_bytes` 返回值附 `input_sha256`
（省标书侧一次流式哈希）；非必需。
**归属判定**：**标书保留**（文件完整性是业务账本面；sha256_streaming 在
bid-parse T-03 已全量内存根治）。
**验收测试**：标书侧 MANIFEST 二跑一致即验收（已有，`97b98fc2…`）。

## §15 · D7 媒体清单【B3 接入点】

**标书用例**：暗标扫描件面（G3 禁字双面扫描需枚举媒体；扫描件 OCR 需发现
图片部件）——corpus-R D5a/D5b、B2 扫描件通道。
**引擎现状**：`docx.rs:1530` `images: Vec<ImageAsset>`（字节/内容类型/引用计数）；
探针实测 S8a2 嵌套件 images 枚举 1 项。
**期望 API**：无缺口。B3 接入：暗标扫描与媒体审计消费 images 清单。
**归属判定**：**B3 接入点**。
**验收测试**：B3 后对 D5 系样本断言 images 非空消费（扫描页发现）。

## §16 · L1 结构化定位器【B3 映射面（引擎更强）】

**标书用例**：`Block.locator: String`（自由文本「章节/页/行」）→ 引用回查
（real-eval 引用支持率 100% 的锚面）。
**引擎现状**：`evidence/src/locator.rs:83` 七变体 `Locator` + `:152`
`validate_region`——结构化、可校验。
**期望 API**：无缺口。B3 适配层做 `Locator → 标书锚描述` 映射（`emit_*` 家族
Python 仿真是 B4 删除对象——同 §10）。
**归属判定**：**B3 映射面**。
**验收测试**：B3 后对 corpus-A 25 埋点断言锚回查等价（V1-b R1 基线对照）。

---

## §17 · 引擎侧已排期条目对齐（A1–A4，issue #2 评论 2026-10-27 01:29）

引擎 TR-2D（`test-artifacts/TR2/`，Windows 真 Word 互操作）与本清单的对接编号：

| 引擎编号 | 内容 | 标书面消费面 | 本清单对应 | 状态 |
|---|---|---|---|---|
| **A1** 标题识别 | 导入 `headings: 0`（TR-2D P1-2）——标题样式段落全按普通段 | **B2.5 实测**：bid extract 按文本正则自建章节上下文（`extract.rs:57-60`），标题**文本**在即免疫；但引擎若丢标题**段落本身**则评分通道死（量化：GT 召回 17→16、评分项 438→221、类别翻转 169 条——`docs/title-impact-analysis.md`） | §17-A1 + B2.5 | 引擎 Phase 1 修复中；标书侧 B3 后以引擎 heading 面冗余加固 |
| **A2** 编号去重 | Word 自动编号叠加文本编号（"1.1"→"11.1"，TR-2D P1-1）——**本探针在引擎读面复现**（word-complex 读回「11.1 表格」） | **B2.5 实测**：docx 条款号污染 1809/1810=99.9%——合规矩阵行号/补遗条款配对/引用锚全错（PDF 面免疫：文本层编号不经引擎导入链） | §17-A2 | 引擎 Phase 1 修复中；**B3 前置**：编号叠加不修，docx 主文件解析不可用 |
| **A3** 行内 marks | `**粗体**` 星号原样输出（TR-2D W4，P0——废标风险：金额强调失效） | 标书导出面（V1-c 技术标交付样本依赖 bold/italic 语义） | §17-A3 | 引擎 Phase 1 修复中；标书 V1-c 导出链 B3 后回归 |
| **A4** G2 导出链 | `default_typography`/`page_geometry` 引擎 API 在位，**产品导出链未消费**（TR-2D W1/W5：全文 10pt Times New Roman、边距写死） | 标书导出面 G2（doc-engine-audit P0 缺口第三项） | §17-A4 | 引擎产品链接线中；标书侧不重复实现（issue #2 约束 #2） |

> A1–A4 全部归属**引擎侧**（TR2 排期在案），标书侧不立平行实现——B3 双跑差分
> 将以「预期差异」归类这四条的残留，并以本清单与 B2.5 量化数据作为引擎修复的
> 验收对照基线。

---

## 附 · mdcore issue 转化清单（按 §3.2 规则可直接开单的条目）

| 优先级 | 条目 | mdcore issue 标题草案 | 标书侧可复现样例 | 验收测试 |
|---|---|---|---|---|
| **P0** | §2 D6 修订 | `feat(office-io): w:ins 当前文本提取 + delText 登记（for ai-bid-research#2）` | corpus-R D2a（澄清四修订插入条款丢失对照：引擎 6 段 vs bid 7 块）+ **T2 批验收夹具 `D6-revisions/` 5 变体 + `expected-readback.json` 期望读回契约** | ins 文本入读回+标记；delText warning 计数；**期望读回逐变体断言（expected-readback.json）** |
| **P1** | §1 D4 文本框 | `feat(office-io): txbxContent 文本块提取（for ai-bid-research#2）` | corpus-R D2a（文本框条款 2 块对照） | 文本框文本入块流且宿主段不截断 |
| P2 | §3 D5 批注/脚注 | `feat(office-io): comments/footnotes 读面（for ai-bid-research#2）` | corpus-R D2a + TR2 word-complex（0 批注对照） | 批注文本+锚段关联；footnotes.xml 读回 |
| P2 | §11 S4 可选吸收 | `harden(office-io): EOCD 声明计数守恒显式判据 + 全文件镜像扫描（for ai-bid-research#2）` | V6/V7/V8 exotica 族生成器（`guard.rs:323-410` 可供样例） | exotica 族 typed reject |
| P3 | §5 S8 可选标记 | `feat(office-io): ImageAsset 增 is_archive 标记（for ai-bid-research#2）` | engine_probe S8a2 构造（声明齐全嵌套 zip） | 嵌套件落 images 且标记 |
| P3 | §14 S10 可选 | `feat(office-io): 读面返回附 input_sha256（for ai-bid-research#2）` | baseline MANIFEST 哈希协议 | 返回 sha256 == 外部流式计算 |

> P3（OCR）不立引擎 issue（§8 标书保留）；D3/D7/L1/X3/P1/S7/S12/S13 为
> B3 接入/判据差异/无缺口，不占引擎排期。
