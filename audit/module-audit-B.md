# 模块审计报告 B —— bid-parse / bid-engine / mdcore 集成边界

审计人：Agent B（解析与引擎集成审计员）。基线：6105beb（只读）。
范围：`bidcore/crates/bid-parse`（src 17 文件 + tests 14 文件 87 测试）、`bidcore/crates/bid-engine`（src 6 文件 + tests 7 文件 49 测试）、mdcore 跨仓调用证据（**未克隆私有仓**——静态审查本仓侧）、`scripts/engine_probe`、`scripts/bid_health_check`（及 veto_gt / veto_measure 口径）。

---

## A. 文件覆盖清单

### A-1 bid-parse/src（17 文件，全覆盖）

| 路径 | 行数 | 状态 | 发现数 |
|---|---|---|---|
| src/lib.rs | 153 | READ | B-13, B-23, B-26 |
| src/engine.rs | 87 | READ | —（接口层干净） |
| src/guard.rs | 529 | READ | B-03, B-14, B-23, B-25 |
| src/extract.rs | 1305 | READ | B-06, B-07, B-08, B-11, B-18, B-26 |
| src/pdf_text.rs | 3283 | READ | （能力边界如实登记，#218/#200/#223 修复链完整） |
| src/clause.rs | 266 | READ | B-22 |
| src/merge.rs | 476 | READ | B-01, B-02, B-19 |
| src/numbering.rs | 409 | READ | B-22 |
| src/nesting.rs | 250 | READ | B-22 |
| src/review_table.rs | 1217 | READ | B-21 |
| src/veto_lexicon.rs | 652 | READ | B-06 |
| src/veto_rule.rs | 636 | READ | —（三轮收紧+28 单测，判据可逐条断言） |
| src/env_preflight.rs | 414 | READ | B-15（SKIP 语义） |
| src/engines/mod.rs | 386 | READ | B-03, B-12, B-23 |
| src/engines/poppler.rs | 139 | READ | B-04, B-24, B-25 |
| src/engines/tesseract.rs | 321 | READ | B-09, B-10, B-27 |
| src/engines/rust_fallback.rs | 76 | READ | —（纯透传） |

### A-2 bid-parse/tests（14 文件 87 测试，全覆盖）

| 文件 | 测试数 | 守卫对象 |
|---|---|---|
| addendum_recovery.rs | 7 | 兜底链/乱码 fail-loud/Type1 英文模板件 |
| corpus_probe.rs | 1 | corpus-A 全链探针 |
| env_preflight_pdfium.rs | 12 | 三态探测/消息面/env 字面量锚定 |
| env_preflight_sim.rs | 3 | SKIP 横幅语义 |
| extract_docx_veto.rs | 12 | DOCX 否决优先级链/#226/#233 对称/幂等 |
| extract_dunhao.rs | 8 | 顿号条款双格式 |
| extract_format_chapter.rs | 9 | #234 格式章守卫 |
| extract_kind_dispatch.rs | 3 | #196/#199 kind 分发矩阵 |
| extract_pdf_l2gap.rs | 10 | PDF L2 残差族（语境传播/程序排除） |
| extract_pdf_para.rs | 9 | 段落重组/词表/目录防御/幂等 |
| nesting_corpus.rs | 2 | 真实语料嵌套配对 |
| numbering_corpus.rs | 2 | 真实语料编号分布 |
| pdf_cid_identity.rs | 7 | CID/GBK-EUC/未用字体/混排页 |
| real_corpus_223.rs | 2 | B-cqyd 局部乱码页/基线条数红线 |

模块内测试另有：pdf_text 38、guard 11（全 `#[cfg(unix)]`）、clause 9、merge 6、numbering 13、nesting 6、review_table 16、veto_lexicon 13、veto_rule 28、env_preflight 6、engines/mod 2、tesseract 2、mapping(bid-engine) 1 —— 合计 ~150。**extract.rs 本体 0 内嵌测试**（1305 行全靠 tests/ 外部守卫）。

### A-3 bid-engine（src 6 + tests 7，全覆盖）

| 路径 | 行数 | 状态 | 发现数 |
|---|---|---|---|
| src/lib.rs | 134 | READ | B-05, B-20 |
| src/reader.rs | 346 | READ | B-12, B-28 |
| src/types.rs | 289 | READ | —（镜像面干净） |
| src/mapping.rs | 531 | READ | B-04, B-11, B-21, B-22 |
| src/pages.rs | 180 | READ | B-20 |
| src/styles.rs | 163 | READ | B-20 |
| tests/{smoke_engine, mapping_docx, mapping_xlsx, mapping_pdf, reader_docx, pages_estimate, styles_template}.rs | 5+7+5+4+10+9+9 | READ | 见 D |

### A-4 跨仓/脚本（静态证据）

- `bid-engine/Cargo.toml:33-39`：mdc-office-io / mdcore / mdc-pdf 三包同钉 rev `f4328ebf…`（与共享上下文一致）。
- `bid-tools/src/service.rs:366-379`：import 路由 docx/xlsx→引擎、pdf→引擎优先失败回退 `parse_file`；`.xls/.ods`→显式拒绝。
- 全仓 grep：`mdc_office_io|mdc_pdf|mdcore::` 在 bidcore 内**只出现在 bid-engine src/tests**（+bid-kb 测试探针）——OOXML 读面唯一接触面纪律成立；calamine 只在 bid-engine reader（类型镜像 + cell_to_string，与引擎同版 0.36）；zip/quick-xml 在 bid-tools/bid-mcp 仅 dev-dep（导出件校验/测试夹具），docx-rs 为写面。
- `scripts/engine_probe/Cargo.toml`：钉 **rev 3204342d（旧）**；`scripts/bid_health_check/src/main.rs:36-47`：STRONG_VETO_PHRASES 8 词镜像；`scripts/veto_gt`（人工标注 GT）与 `scripts/veto_measure`（生产代码同源测量）为 0.83 口径来源。

---

## B. 发现清单（B-01 … B-28）

> 严重程度定义同共享上下文。每条含位置/证据/触发条件/影响/置信度。

### P1

**B-01｜Agent「requirement 标记未提交」条款黑洞（合并语义单向校验）**
- 位置：`bid-parse/src/merge.rs:133-148`（正则 REQ 丢弃）+ `merge.rs:151-189`（REQ-A 只来自 submitted）+ `bid-domain/src/coverage.rs:192-218`（validate_submitted）。
- 证据：`regex_reqs.retain(|r| !r.anchors.iter().any(|a| … covered.contains_key(&idx)))` —— 丢弃与 verdict 无关；REQ-A 仅由 `f.submitted` 生成。`validate_submitted` 只校验「submitted 必须落在 requirement 区间」，**不校验反向**（requirement 区间必须有 submitted）。
- 触发：Agent 调 `submit_block_marks` 提交 `verdict=requirement` 区间但未（或部分未）随附 requirements（两参数可独立缺失，coverage_tools.rs:139-216）。
- 影响：该块正则 REQ 被丢、无 REQ-A 生成 → 条款**无任何告警地从矩阵消失**（blocking 面直接缩水）。merge.rs 测试 `covered_block_regex_dropped_any_verdict` 把该端态**钉为预期**（"全覆盖 + 无提交 → 空"），说明是有意设计，但「Agent 标 requirement」语义上即主张"此处有要求"，语义与实现相悖。
- 置信度：CONFIRMED（代码路径静态成立）；典型 Agent 行为频率 SUSPECTED。

**B-02｜merge 面按文件名前缀过滤——真实文件名的 Agent 标注整体失效**
- 位置：`bid-mcp/src/tools.rs:724-741`（faces 组装 `.filter(|p| has_extraction_face(&p.name))`）+ `bid-parse/src/merge.rs:71-76`（`has_extraction_face` = `A-|C-|B1|B2` 前缀）。
- 对照：抽取分发（`extract.rs:100-116`）已按业务 kind（#196/#199 修复），真实文件名「光纤租赁费招标文件.pdf」正常主抽取。
- 影响：真实文件名文件上的 `block_marks`/`submitted_requirements` **永不进合并**——REQ-A 不生成、正则产物不被 Agent 裁决覆盖、`coverage_complete/uncovered_blocks` 统计不含该文件。#196 修了抽取分发却没修合并面：Agent 对真实件做完覆盖标注再 extract，会得到"标注被无视"的结果且 `UNCOVERED_BLOCKS` 告警完全失真（tools.rs:839 的提示会误导 Agent 去补标无效的块）。
- 置信度：CONFIRMED。

**B-03｜历史 ZIP 炸弹门链（V6/V7/V8 修复面）已随 B4 整体删除，本仓侧无等价可验证物；对账脚本钉旧 rev**
- 位置：`bid-parse/src/guard.rs`（现内容仅剩：PDF 输入字节门 64-70、流式 sha256 73-86、子进程治理 88-336——**无任何 zip/EOCD/XML 门**）；`docs/engine-bid-mapping.md §4`（"S7 定谳 ratio 门引擎**无**（合法 149:1 引擎放行）"）；`scripts/engine_probe/Cargo.toml`（rev `3204342d` ≠ 生产 `f4328ebf`）。
- 影响：①解压比 100:1 门、EOCD 守恒门（V6 尾垃圾/V7 影子 EOCD/V8 字段分歧三绕过的封堵）、zip64 前置垃圾门、XML 深度/实体门——这些**修复不在本仓产品链上**（删除而非保留），当前防线=引擎三门（prescan 声明值 + 尺寸诚实 + 部件良构）+ xlsx 读后双轴门（引擎侧）。②engine-bid-mapping 自认 ratio 门引擎侧不存在——即压缩比炸弹防线在当前链上**没有等价物**（仅剩声明值预扫与读后计量）。③engine_probe 是唯一曾实测 S7/S8 的对账工具，但其钉的引擎 rev 落后生产 87+ 提交，README 声称的"双跑差分两侧必须同引擎版本"已被自身违反——**当前 rev 的引擎门行为零对账证据**。
- 置信度：CONFIRMED（删除事实与脚本 rev 差异静态可证）；引擎侧实际防御力 NEEDS_ENV（私有仓不可克隆）。
- 备注：threat-model.md T-03 仍把整套门写在 "bid-parse/guard.rs 挂门"名下（见 E-1）——按审计纪律"文档与代码不一致也是发现"。

**B-04｜PDF `tables` 计数恒 0（两处实现同源死代码）——健康检查"PDF 表格 tables=0"的代码侧根因**
- 位置：`bid-parse/src/engines/poppler.rs:105-108` 与 `bid-engine/src/mapping.rs:427-430`：`filter(|b| … b.text.contains("  ")).count()`；而两处的 `collapse_spaces`（poppler.rs:124-139 / mapping.rs:452-467）把**任意连续空白压成单空格**——计数前提永不成立。
- 证据：mapping.rs:450 注释宣称"空白折叠（**保留双空格**——tables 计数锚）"，与实现（`if !prev_space` 才推单空格）**直接相反**。
- 影响：所有 PDF 件 `ParsedFile.tables=0`。与 `artifacts/bid-health-check/REPORT.md` 的"PDF 表格 tables=0"实测互证——该指标在当前基线是**死指标**，任何以 tables>0 为判据的消费方（readiness/统计）对 PDF 恒假阴性。
- 置信度：CONFIRMED。

### P2

**B-05｜MDCORE_REV 常量与实钉 rev 不符——parse_summary 溯源口径失真，且测试自引用无法报警**
- 位置：`bid-engine/src/lib.rs:78`（`MDCORE_REV = "aa0c859d…"`）vs `bid-engine/Cargo.toml:33/36/39`（rev = `f4328ebf…`，注释明言 #110 任务 2 指令 bump）。
- 证据：`mapping.rs:280-285/369-373/432-436` 把该常量写进每个 ParsedFile.parse_summary；tests（mapping_docx.rs:88、mapping_xlsx.rs:52/75）断言 `contains(MDCORE_REV)` —— **同一常量自证**，rev 漂移永不红。
- 影响：所有引擎路径解析产物的 rev 溯源错报（差至少一个 bump 周期）；对账/差分回放（"升级走独立 PR 并重跑基线差分"纪律）失去可信锚。
- 置信度：CONFIRMED。

**B-06｜DOCX/PDF 否决词表两套不同源——同一条款跨格式分类不同（#226/#233 层 2 的判据面不对称）**
- 位置：PDF 侧 `extract.rs:216-237`（re_veto，~48 形态，含 F3/F5 扩展）；DOCX 侧 `veto_lexicon.rs:22-40`（re_veto_strong）+ `44-57`（re_negative_list）。
- 差集（PDF 有 / DOCX 无）：`不接受…(投标|报价|备选|联合体)`、`不接受联合体`、`投标失效`、`投标被拒绝`、`联合体投标…不接受`、`是否接受联合体…不接受`、`备选…不允许`、`拒收情形`、`否决情形`、`无效情形`、`废标情形`、`以他人名义投标`、`利害关系…不得参加投标`、`行贿`（DOCX 有，仅列差向）。
- 差集（DOCX 有 / PDF 无）：`投标无效`、`投标均无效`、`不予评审`、`有权取消`、`否决投标条件`、`骗取`(裸词)、`重大安全事故`、`受贿`、`私下接触`、`不良记录`、`不是本单位人员`、`非本单位人员`、`被责令停业`、`被逐`、`合同被解除`、`违约或被逐`。
- 触发例：DOCX 条款「本次招标不接受联合体投标」在 DOCX 侧 `vl::is_veto=false`（extract_docx_veto.rs:122 的"对称测试"实际经非否决路径通过——未钉住否决信号本身）；同一文本 PDF 侧 veto_hit=true 走 #226 规则说明降级。「以他人名义投标」「其投标无效」同理单侧命中。
- 影响：L2 否决召回在 docx/pdf 两格式间系统性不一致；GT/评测按格式分桶时数字不可比。
- 置信度：CONFIRMED（词表差集静态比对）。

**B-07｜B1/B2 分支的 kind 判据用了导入枚举外的值——生产只能靠文件名前缀触达（#196"前缀不得作生产判据"在 B1/B2 面回归）；B1 仅匹配 PdfLine**
- 位置：`extract.rs:102-103`（`kind=="报价文件"` / `kind=="扫描件"`）；`bid-mcp/src/tools.rs:83-91`（TENDER_FILE_KINDS 七值不含这两者）。
- 影响：①真实命名的报价清单/扫描附件（kind 只能登记为 附件/图纸 等）走 B1/B2 分支的唯一途径是文件名以 `B1`/`B2` 开头；②`extract_b1_lines`（extract.rs:1152-1166）只匹配 `BlockKind::PdfLine`——xlsx 报价清单（XlsxRow）静默 0 抽取；③B2 OCR 关键词仅 3 词（否决/签署/盖章， extract.rs:1170）——扫描附件的资格/技术条款全部不抽。extract_kind_dispatch.rs:103-110 的 B1 测试自注"可能为 0，只验证不 panic"。
- 置信度：CONFIRMED。

**B-08｜XlsxRow 块落入 extract_main_docx 的 `_ => {}` 黑洞——xlsx 主文件静默零抽取**
- 位置：`extract.rs:78-116`（dispatch：非附件/非 B1/非 B2/无 PdfLine 块 → `extract_main_docx`）+ `extract.rs:932-1146`（match 仅 Para/TableRow，`_ => {}`）。
- 触发：kind=招标文件/补遗的 `.xlsx`（清单型招标文件真实存在）→ blocks 全为 XlsxRow → 0 REQ，无 per-file 信号（ZERO_REQUIREMENTS 告警只在**全部**文件 0 条时触发，tools.rs:860；同批 docx 有产出时完全静默）。
- 置信度：CONFIRMED。

**B-09｜OCR 逐页循环 NonZeroExit 即 break——部分成功无显式状态、截断范围不可见**
- 位置：`tesseract.rs:137-141`（`ocr_fail_pages += 1; summary = …; break;`）。
- 触发：第 N 页 tesseract 报错（如某页字体特异/内存不足）→ 其后所有页**不再 OCR**；`pages` 字段=全部渲染张数（114 行 `pages = pngs.len()`）、`ocr_blocks>0` → 返回 `Ok`。
- 影响：100 页扫描件第 5 页失败 → 95 页内容静默丢失，parse_summary 仅写"OCR 失败（stderr）"，不写丢页范围；导入状态 Ok/ocr_required 照常推进。注释宣称"部分页成功仍走降级（扫描页残缺属数据面，语义不变）"——但"跳过其后所有页"远超"该页残缺"。
- 置信度：CONFIRMED。

**B-10｜pdfinfo 探测失败回退 real_pages=1 → 多页扫描件只 OCR 第 1 页**
- 位置：`tesseract.rs:87`（`probe_pdf_pages(path).unwrap_or(1)`）+ `:99`（`.args(["-l", &real_pages.to_string()])` 渲染上限=1）。
- 触发：pdfinfo 退出失败/解析失败（部分损坏但 pdftoppm 可渲染的件、或 pdfinfo 缺失而 pdftoppm 在位的畸形部署）。
- 影响：整件只渲染+OCR 1 页，返回 Ok（ocr_blocks>0）——**大面积静默内容丢失**且无任何"页数探测失败"提示（探测失败被 `.unwrap_or(1)` 吞掉，连 summary 都不记）。
- 置信度：SUSPECTED（需构造 pdfinfo 失败环境验证触发频率；代码路径 CONFIRMED）。

**B-11｜docx 引擎映射的章头识别只认中文数字——「第2章」不建章节上下文（与 PDF 侧不对称）**
- 位置：`bid-engine/src/mapping.rs:481-491`（`as_chapter`：`cn_num_ok` 仅 `一二三四五六七八九十百`）+ `:476-478`。
- 对照：PDF 侧 `extract.rs:208`（`re_chapter = ^第\s*([0-9一二三四五六七八九十百]+)\s*章`——阿拉伯章号可命中，且注释明言真实水利招标文件用「第2章 投标人须知」）。
- 影响：阿拉伯章号的 docx 主文件全部段落 locator 章上下文为空 → `category_for` 大面积落「其他」（真实语料 13/26 落其他的已知面之一），评分/技术/资格章节映射失效。
- 置信度：CONFIRMED。

**B-12｜pdf 引擎通道（pdfium）绕过全套乱码仲裁与行清洗——同一文件两通道不同文本口径/页数语义**
- 位置：`bid-tools/src/service.rs:376-377`（pdf 引擎优先，成功即用）；`mapping.rs:400-448`（`pdf_to_parsed_file` 无 `probe_cjk_garble_pages`/`sanitize_line`/OCR 救援调用）；对照 `engines/mod.rs:65-207`（旧路径全套 #169/#200/#223 仲裁）。
- 差异点：①控制字符：poppler 路每行过 `sanitize_line`（C1 剔除），引擎路不过；②页数：引擎=pdfium 真实页数（mapping.rs:431），poppler=非空分段数（poppler.rs:82-86）；③行切分与 locator 行号两通道不同 → 同一 sha 的文件在有无 pdfium 的环境间块数/idx/页数均不同——`submit_block_marks` 的 block_idx 与 chunk 对齐只在同通道内稳定；④乱码防线（含 OCR 救援、局部页诊断 #232）对引擎路完全不生效。
- 影响：环境差异=身份/定位漂移；pdfium 环境部署后历史 marks 的 block_idx 可能错位（Agent 若跨会话持旧索引）。
- 置信度：CONFIRMED（口径差异）；乱码实际风险 SUSPECTED（pdfium 自带 CMap，通常解码正确）。

**B-13｜OCR「白扫」诚实空 → 导入 Ok(0 chunks) 无 per-file 告警——内容丢失与空文件不可区分**
- 位置：`tesseract.rs:161-176`（全页成功但 0 行块=Ok，注释自认"那是诚实空"）；`service.rs:380-431`（Ok → OcrRequired，`parsed.push(p)` 进抽取面）；tools.rs 仅全局 ZERO_REQUIREMENTS（:860）。
- 影响：扫描主文件 OCR 全空时，只要同批其他文件有 REQ，该文件零贡献**零告警**；`ocr_required` 状态位不产生任何 warning（readiness 侧有 input_integrity_gate 测试兜底主件场景，但抽取响应面无信号）。
- 置信度：CONFIRMED（部分缓解：bid-tools readiness 有"主件零文本=不就绪"测试）。

**B-14｜guard.rs 测试模块整体 `#[cfg(unix)]`——T-04 子进程治理 11 项负向测试在仅 Windows 的 CI 上不编译不执行**
- 位置：`guard.rs:339-341`（`#[cfg(test)] #[cfg(unix)] mod tests`）；CI 仅 `windows-check.yml`（无 Linux/clippy job）。
- 影响：超时杀进程组、溢出即时收割、stderr drain、Busy 门、TmpGuard 回收等治理纪律**零 CI 守卫**——回归只会在 Linux 开发机上偶然暴露（guard.rs 头注释记载的 E0596 潜伏事故同款模式）。
- 置信度：CONFIRMED。

**B-15｜SKIP 横幅在 CI 上成"绿色静默"——CI 绿 ≠ 解析链被验证**
- 位置：`env_preflight.rs:298-311`（SKIP 打横幅+return，自认"测试计数显示 passed"）；CI=windows-latest（无 poppler/tesseract/chi_sim/pdfium/python3-reportlab/中文字体）。
- 受影响面（CI 上实际不跑）：poppler 文本层链、OCR 主链与页数门（tesseract.rs:281-320）、OCR 救援（engines/mod.rs:339-385）、引擎 PDF 映射 4 测试（mapping_pdf.rs:41-54 SKIP 门）、guard 全模块（B-14）。跑得动的只有纯 Rust 兜底、pdf_text 38 测试、extract/merge 纯函数面。
- 影响：**主解析通道（外部工具依赖面）的全部回归防线在 CI 上为零**；"ENV_MISSING 时测试真的没跑"这一点是诚实的（横幅+提前 return，非假断言），但组织面上 CI 结论被系统性高估。
- 置信度：CONFIRMED。

**B-16｜否决召回"两套口径"均非生产算法同源——0.386 与 0.83 分母不同且都偏离生产词表**
- 位置：`bid_health_check/src/main.rs:36-47`（STRONG_VETO_PHRASES **8 词**镜像，注释自认"两处若漂移以 extract.rs 为准"——现已漂移：生产 re_veto ~48 形态）；:106-127（LingYan 20 词 GT=0.386 口径）；`scripts/veto_gt`（候选池=20 词+37 宽模式，人工裁决 GT）+ `scripts/veto_measure`（生产代码直跑=0.83 口径）。
- 分析：①0.386（unique_face）的"真值面"是外部 LingYan 词表块级命中，测量面是 8 词镜像——**分子分母都不是生产算法**，对生产 L2 只能作下界参考；②0.83 的 GT 候选池由 20 词+宽模式生成——GT 召回受候选池上界约束（池外否决语义进不了 GT），veto_measure 的"生产同源"仅指解析/抽取代码同源、GT 非算法自产（这点做得对，避免自证）；③生产词表比两套口径都宽，真实语料上的实际召回无任何一套数字代表。
- 影响：以 0.386 驱动的改进决策（如继续扩词表）与以 0.83 驱动的"够用"结论互相不可比；health_check 镜像已落后生产一个演进周期（F3/F5 家族未镜像）。
- 置信度：CONFIRMED（口径事实）。

**B-17｜对账脚本族钉旧 rev / 无 CI——引擎对账证据链过期**
- 位置：`engine_probe/Cargo.toml`（mdc-office-io/mdc-pdf rev `3204342d`）；veto_gt/veto_measure/bid_health_check 为 path 依赖（跟随本地 checkout，无 rev 钉扎、无 CI）。
- 影响：①engine_probe 的 S7/S8/D4-D6/X3 结论只对 3204342d 负责，生产 f4328ebf（87+ 提交差）无对账；②path 依赖脚本跑在任意本地基线上，"与 service.rs import 路由同款"的说法依赖运行者自觉 checkout 正确 rev。
- 置信度：CONFIRMED。

**B-18｜PDF 段落重组无页界守卫——跨页条款拼接吞页码/页眉噪声**
- 位置：`extract.rs:442-499`（段落扩展循环：仅以行尾标点/条款头/列举头/skip 行断段——**无页边界判定**）。
- 触发：条款跨页（pdftotext 页间插入页码行「12」、页眉「XX 水库除险加固工程」）→ 页码/页眉行不是 skip 行（<6 个点）、不是条款头 → 被拼进条款 `para_text`，进 excerpt 与 veto/qual 判定文本。
- 影响：跨页条款 excerpt 污染（证据引用回查噪声）、词表匹配跨噪声行失配（否决词被页眉截断的 missed 面）；#68 的列举断段只缓解部分。跨页条款测试（extract_pdf_para）未覆盖页间噪声形态。
- 置信度：SUSPECTED（构造跨页 PDF 验证；机制 CONFIRMED）。

**B-19｜REQ-A 编号按运行序重排——Agent 补标后重跑 extract，REQ-A id 漂移**
- 位置：`merge.rs:151-189`（`seq` 每次调用从 0 递增，按 faces 顺序+block_idx 排序）；`tools.rs:801-805`（`replace_requirements` 全量替换）。
- 触发：Agent 先 extract（得 REQ-A0001..N）→ 补 submit → 再 extract → 覆盖面变化使正则保留集/REQ-A 序变化 → 同一条款 id 变化。
- 影响：以 req id 为外部键的下游对象（矩阵 override、绑定、人工记录）悬空；域层是否级联清理属 Agent A 范围（此处登记交界风险）。替换语义保证不累积重复（重跑幂等在 store 层成立）。
- 置信度：CONFIRMED（编号机制）；下游悬空面 SUSPECTED。

**B-27｜TESSDATA_PREFIX 强制覆盖 `~/.tessdata`——系统安装的 chi_sim 不被识别**
- 位置：`tesseract.rs:118-127`（每页 OCR：`env("TESSDATA_PREFIX", $HOME/.tessdata)` **显式注入**给子进程）与 `:194-212`（chi_sim_available 同口径）。
- 触发：发行版包管理器安装 tesseract-ocr-chi（数据在 /usr/share/tesseract-ocr/*/tessdata），未设 env、无 ~/.tessdata。
- 影响：probe 判 Missing（OCR 救援不启动）；主路径逐页 NonZeroExit → 整文件失败。行为"诚实失败"但部署面极易踩（与 env_preflight 的 chi_sim 检查口径一致地苛刻）。幂等缓解：文档 hint 提示设 TESSDATA_PREFIX。
- 置信度：CONFIRMED（注入逻辑）；实际环境失败形态 NEEDS_ENV。

**B-28｜混合 PDF（文本页+扫描页）无逐页路由——扫描页内容两通道都静默丢失**
- 位置：`engines/mod.rs:77-119`（旧路径：poppler 全文非空白 ≥16 即整件走文本层，其余空页不 OCR、不点名）；`reader.rs:338-340` + `mapping.rs:404-408`（引擎路径：any_text_layer=任一页 ≥16 → 整件文本层，纯扫描件才 MappingGap）。
- 触发：前 2 页有文字说明、后 50 页扫描图纸的混合件。
- 影响：两通道都以"任一页有文本"判整件——扫描页 0 块且无 per-page 诊断（#232 的局部乱码页点名只覆盖乱码页，不覆盖空页）。B1 图纸目录类文件的典型形态。
- 置信度：CONFIRMED（判定逻辑）；真实混合件占比 NEEDS_ENV。

### P3

**B-20｜#110「已接」能力（pagination/styles）实为门面级——全仓零生产消费方**
- 位置：`bid-engine/src/lib.rs:29-53`（能力账本"已接（本 PR）"）；grep 全仓：`check_page_limit|estimate_pages|StyleTemplate|bid_baseline_params` 只出现在 bid-engine 自身 tests。pages.rs:29-32 自认"export.rs 接线在下一轮"。
- 影响：账本"已接"与业务消费为零的差距——登记诚实但表格口径易误读为"已生效"。属 DESIGN_PREF（登记纪律本身合格）。

**B-21｜review_table 死表达式与 mapping 死代码**
- 位置：`review_table.rs:630`（`re_row.is_match(n) == false && false` 恒 false——"防误伤"注释的意图未实现）；`review_table.rs:680`（chapter_ref_of 字符类重复 '（''）'）；`mapping.rs:507-510`（`is_numbered_clause` 零生产调用）。
- 置信度：CONFIRMED。

**B-22｜「单一真源」声明的接线落差：merge_multiline_clauses / nesting 全模块 / NumberingContext 零生产消费**
- 位置：`lib.rs:14-23`（模块头宣称 G1/G5/G6 为"单一真源"）；grep：`merge_multiline_clauses`、`split_respecting_nesting`、`find_matching`、`NumberingContext` 仅测试消费（extract_main_docx 用自己的续行吸收循环 + `clause_boundary`，未用 merge_multiline_clauses 的四规则）。
- 影响：clause_boundary 确已接线（extract.rs:971 ✓），但 G1 的合并主函数与 G6 全模块是"已交付未接线"面——文档语义（"docx 续行吸收的边界判定单一真源"）与实际（抽取层内联实现，仅边界判定同源）有差距。DESIGN_PREF/P3。

**B-23｜大文件 IO/内存面：parse_pdf 全量读 ≤500MB + sha256 前置双读**
- 位置：`engines/mod.rs:69`（`std::fs::read(path)` 全量入内存）；`lib.rs:121`（parse_file 先 sha256_streaming 全文件，再进 parse_pdf 的 check_input_size——字节门在哈希读之后，与 guard.rs:61-63"读任何内容之前先查 metadata"的声明顺序不符）；import 侧 service.rs:288 又算一遍 sha。
- 影响：500MB 件单次导入 3 遍读 + 1 份全量内存（×并发 4 席）；门序声明与实现不一致。
- 置信度：CONFIRMED。

**B-24｜poppler probe/错误映射的字符串协议脆弱**
- 位置：`poppler.rs:22-34`（probe：非 Spawn 结果一律 Ready——pdftotext -v 超时也算在位）；`:57-76`（`map_run_err` 以 `contains("无法启动")`/`contains("退出码")` 字符串分类 EngineError——RunOutcome 文本改动即错分类）。
- 置信度：CONFIRMED。

**B-25｜外部工具参数注入面（收窄但存在）**
- 位置：`poppler.rs:37-38`、`tesseract.rs:96-101/122-127`、`pdfinfo`（tesseract.rs:237-239）——path 以 `as_os_str()` 直传，无 `--` 分隔或 `-` 前缀转义。
- 触发：导入文件名/路径以 `-` 开头且以相对路径调用时可能被解析为选项。fs_fence（Agent A 面）保证落盘绝对路径时面收敛为理论面。
- 置信度：CONFIRMED（无转义代码事实）；可利用性 NEEDS_ENV。

**B-26｜附件/图纸/证据整文件跳过抽取——SC-02「登记不伪装」未延伸到文件级跳过面**
- 位置：`extract.rs:104-105`（`if is_attachment { /* 无条款抽取面 */ }`——无 note/ledger 写入）；对照 ComponentLedger 纪律（lib.rs:79-95"present>0 而 extracted==0 必须带 note"）。
- 影响：附件类文件照常落库进 chunk/检索面，抽取面 0 无任何 per-file 标记——"跳过"与"抽不出"在 ParsedFile 上不可区分。
- 置信度：CONFIRMED。

---

## C. 解析通道 × 失败语义矩阵

| 通道 | 入口 | 失败/降级语义 | 状态可见性 | 结论 |
|---|---|---|---|---|
| docx → mdc-office-io | service.rs:369 | 容器门拒→EngineRejected→import Failed（显式）；模型错→EngineFailed→Failed | per_file errors ✓ | 诚实 |
| xlsx → mdc-office-io | service.rs:372 | 同上；.xls/.ods→parse_file→「不支持的文件类型」显式拒绝 | ✓ | 诚实 |
| pdf → pdfium 引擎 | service.rs:376 | 无 pdfium→EngineFailed→回退旧路径；纯扫描→MappingGap→回退；文本层→**直接采信（无乱码仲裁/无行清洗，B-12）**；混合件→扫描页丢（B-28） | parse_summary 自报 rev（**错报**，B-05） | 部分成功无状态 |
| pdf → poppler 文本层 | engines/mod.rs:75 | ≥16 非空白→采信；乱码→兜底重解→互证/OCR 救援→fail-loud（带 [pdf-*] 标签与原因 hint） | ✓（全套仲裁，本仓最完整失败语义链） | 诚实 |
| pdf → rust 兜底 | mod.rs:86/125/172 | NotPdf/Encrypted/NoTextLayer/Unsupported 显式 Err；[pdf-both-failed] 双因都报 | ✓ | 诚实 |
| pdf → OCR 主链 | mod.rs:118 | >200 页拒（渲染前）；pdftoppm 失败→Err；**逐页 NonZeroExit→break 部分成功（B-09）**；全败+0 块→Err；全成 0 块→Ok 诚实空（B-13）；pdfinfo 失败→只 OCR 1 页（B-10） | 部分 | 部分成功无状态 |
| OCR 救援 | mod.rs:267-289 | 缺件诚实跳过（None→落回 fail-loud）；产物须过乱码探测才采信 | ✓ | 诚实 |
| 导入聚合 | service.rs:276-459 | 单件失败显式登记+继续（R-01）；失败件禁引（不进 parsed）✓；**Ok-0-块件进 parsed 无告警（B-13）** | per_file + 状态机 | 基本诚实 |

**同一文件不同通道身份/定位**：sha256 恒同 ✓；blocks/idx/locator/pages/parse_summary 在「pdfium ↔ poppler ↔ 兜底」三通道间互不相同（行切分、页数口径、清洗差异，B-12）——channel 决定身份面，环境变更=漂移面。block_idx 稳定性仅在「同文件+同通道+同引擎版本」内成立（组件版本矩阵纪律覆盖引擎版本，未覆盖通道选择）。

---

## D. 测试守卫缺口

已覆盖（点名）：乱码三态/双路 fail-loud（addendum_recovery、pdf_cid_identity 7 测试）；CID/无 ToUnicode/GBK-EUC/混排页（pdf_cid_identity——但全走纯 Rust 路，poppler 在位行为靠环境）；顿号/格式章/L2 残差族/规则说明对称（extract_* 系列，纯函数面扎实）；kind 分发矩阵（extract_kind_dispatch）；OCR 页数门（tesseract.rs:281，需 python3+reportlab，CI SKIP）；幂等（extract_docx_veto/extract_pdf_para 各 1）；引擎映射黄金锚（mapping_docx/xlsx/pdf）；styles/pages 手算锚（零 SKIP 面，bid-engine 内最可靠测试组）；部件账本/镜像纯度（reader_docx 10 项）。

**缺口（按提问清单）**：
1. **扫描件**：OCR 主链逐页语义零单测（不依赖环境即可测的装配逻辑——如 NonZeroExit break、白扫——无负向测试）；OCR 救援只有 1 项且 CI SKIP。
2. **混合 PDF**（文本页+扫描页）：无任何测试（B-28）。
3. **CID/无 ToUnicode**：fallback 面覆盖好；poppler/pdfium 通道的 CID 行为零测试（依赖环境，CI 全 SKIP）。
4. **中文断字/逐字 Tj**：pdf_text 38 测试覆盖（Tm 合成行/WPS 缩放/按字体路由 CMap）✓——此项是强项。
5. **跨页条款**：无页间噪声（页码/页眉插入）测试（B-18）。
6. **嵌套表格**：无测试；且 mapping.rs:39-41 注释（引擎不提取）与 service.rs:365 注释（#39 已修复）**互相矛盾**——哪个对应当前 rev 无从判定（NEEDS_ENV）。
7. **损坏文件**：docx/xlsx/pdf 截断拒各有 1-2 项 ✓；PDF 乱码 EOCD/xref 损坏对 poppler/pdfium 的行为差零测试。
8. **加密文件**：pdf_text Encrypted 单测 ✓（纯 Rust 路显式拒）；poppler/pdfium 对加密件行为零测试。
9. **超大文件**：LIMIT_INPUT_BYTES_PDF=500MB 无端到端负向测试（guard 函数级测试用的是 2MB 临时上限）；MAX_OCR_PAGES 有负向 ✓（环境 SKIP）。
10. **表格抽取（真表格结构）**：PDF 侧无（tables 恒 0 正因无消费无测试而未被发现，B-04）；docx 审查表（review_table 16 测试）✓。
11. **B-01 黑洞 / B-02 merge 面前缀过滤 / B-08 XlsxRow 黑洞 / B-09/B-10**：零测试守卫（B-01 端态反被钉为预期）。
12. **CI 面**：windows-only + SKIP 语义 = 主解析链回归零 CI（B-14/B-15）；scripts 对账脚本族无 CI（B-17）。
13. **REQ-A id 稳定性**（B-19）：无"补标→重跑→id 不变/漂移"断言。

---

## E. 与 docs 的差异

| # | 文档声明 | 代码事实 | 判定 |
|---|---|---|---|
| E-1 | threat-model.md T-03：解压比/EOCD 守恒（V6/V7/V8）/zip64/XML 深度门「挂 bid-parse/guard.rs」 | guard.rs 已无任何 zip/XML 门（B4 收敛删除） | 文档严重过时，安全审计结论不可引用 |
| E-2 | capability-map.md S6/S9/X1：锚点指向 `bid-parse guard.rs:50 LIMIT_XML_DEPTH`、`bid-parse xlsx.rs:17` | 两文件/常量已不存在（xlsx.rs 已删） | 同上 |
| E-3 | engine-bid-mapping.md §1 末行：parse_summary rev「从 baseline/MANIFEST.json 的 BASELINE_MDCORE_REV 口径取」 | 代码硬编码 `MDCORE_REV` 常量且与 Cargo.toml rev 不符（B-05） | 双重不符 |
| E-4 | engine-bid-mapping.md §1/§3 与 mapping.rs:450 注释：collapse_spaces「保留双空格——tables 计数锚」 | 实现把连续空白压成单空格，tables 恒 0（B-04） | 注释与实现相反 |
| E-5 | mapping.rs:39-41「嵌套表格（mdcore#39）引擎不提取——账本登记」vs service.rs:365「嵌套表 mdcore#39 已修复」 | 两注释互斥；代码无嵌套表测试可裁决 | NEEDS_ENV（引擎侧行为），文档至少一处错 |
| E-6 | engine-bid-mapping.md §2：xlsx locator「`<sheet名> 行<N>`」 | 代码 `format!("{}!行{}", …)`（多 `!`） | 小偏差 |
| E-7 | doc-engine-audit.md G7/V1-a：「文本框/批注/修订对 naive 正文遍历不可见」（基线 6daeace） | 当前 rev 面（f4328ebf）：TextBox 变体/comments/RevisionDelete warnings 均已入镜像与块流（reader/tests 实证） | 文档时点旧——能力账本（lib.rs）为准，非缺陷但引用需注意 |
| E-8 | lib.rs 能力账本「pagination/style 已接」 | 门面级接线，业务零消费（B-20） | 口径易误读（登记本身诚实） |
| E-9 | bid-mcp-contract/共享上下文：parse_summary 溯源可对账 | rev 错报（B-05）+ 引擎/旧路径两通道 summary 语义不同（B-12） | 溯源链不可信 |

---

## 结论摘要（供 Lead 汇总）

1. **失败语义总体纪律良好**（fail-loud 标签、双因并报、诚实空、SKIP 横幅），四个真缺口：OCR 逐页 break 部分成功（B-09）、pdfinfo 回退 1 页（B-10）、混合 PDF 无逐页路由（B-28）、Ok-0-块无 per-file 告警（B-13）。
2. **两个 P1 合并语义洞**：requirement-未-submit 黑洞（B-01）与 merge 面前缀过滤使真实文件名 Agent 标注整体失效（B-02）——#219 的"Agent 优先"设计对 #196 修复后的真实文件名世界不成立。
3. **安全门链的历史修复（V6/V7/V8）不在当前产品链上**（B4 删除+引擎侧 ratio 门自认缺位+对账脚本钉旧 rev，B-03/B-17）——需要 Lead 裁决：引擎三门等价性验证列入 NEEDS_ENV 或恢复镜像负向测试。
4. **跨格式不对称三件套**（否决词表两套 B-06、章头中文数字限定 B-11、tables 恒 0 B-04）——直接对应真实语料"类别落其他 13/26、PDF 表格 0、召回两口径"的健康检查遗留问题。
5. CI（windows-only + SKIP=passed + guard 测试 unix-only）对主解析链的回归守卫为零（B-14/B-15）。
