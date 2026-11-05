# 模块审计报告 B2a —— bid-parse PDF/引擎/防护链路（Task 4-b2a）

> 全仓审计 V2 · 模块代理 B2a。冻结基线 main@`6105beb`，分支 `audit/v2-full-coverage`（HEAD `a689701`，生产代码与基线一致）。
> 范围：`bidcore/crates/bid-parse/src/{pdf_text.rs, engines/mod.rs, engines/tesseract.rs, engines/poppler.rs, engines/rust_fallback.rs, engine.rs, lib.rs, guard.rs, env_preflight.rs}`（9 文件全读，pdf_text.rs 3283 行分段读全）+ `tests/{pdf_cid_identity, addendum_recovery, extract_pdf_l2gap, extract_pdf_para, env_preflight_pdfium, env_preflight_sim, corpus_probe, numbering_corpus}.rs`（8 文件全读；顺带读 real_corpus_223.rs 以核 #223 面）。
> 纪律：只读审计，未改动任何生产文件/测试/配置。**运行时佐证不可用**：Lead 的 `/home/z/my-project/logs/03-bidcore-cargo-test.log` 仅含 15 行编译开头（无任何 test result 行，日志截断），本文全部结论基于静态代码证据 + grep 交叉验证。

## 0. 架构事实核对（先钉死链路）

- **bid-parse 内部 PDF 链**（`engines/mod.rs::parse_pdf`）：poppler(`pdftotext -layout`) 文本层① →（空文本/乱码时）纯 Rust 兜底 `pdf_text`② + tesseract OCR 救援③（`try_ocr_recovery`）。扫描件主路径：poppler 空 + 兜底无文本 → `tess.ocr_parsed`（`pdftoppm` 渲染 → `tesseract chi_sim`）。
- **生产链**（`bid-tools/src/service.rs:376-377`，B2b 侧消费方）：`bid_engine::parse_pdf_via_engine`（mdcore mdc-pdf pdfium，git rev `f4328ebf…` 钉住）`.or_else(|_| parse_file(...))` —— 即 pdfium → 旧链（poppler→OCR/兜底）。**引擎侧错误在降级点被 `|_|` 静默丢弃**（见 PARSE-006）。
- `engine.rs` 的 `registry()`（poppler→tesseract→rust-fallback 顺序）**仅被 1 个内联测试消费**，`parse_pdf` 手工路由不走它——R3(CrossValidate)/R4(外部引擎) 是注释里的未来扩展点（见 PARSE-018）。
- mdcore（mdc-office-io/mdc-pdf/mdcore）为外部 git 依赖，**本机无源码 checkout**（`~/.cargo/git/checkouts` 空），引擎侧防护等价性无法在本仓内核对（见 PARSE-017，跨仓移交）。

## 1. 历史问题重核结论（对照 audit/historical-context.md §2 重核清单）

| 历史项 | 当前状态 | 证据 |
|---|---|---|
| R3-S1-1 OCR 200 页死门 | **已修复**：`tesseract.rs:87-92` `probe_pdf_pages`(pdfinfo 真实页数) 过门 + 渲染 `-l real_pages` 双保险；**">200 页拒绝"负向测试在位**（`tesseract.rs:281-320` `oversized_scanned_pdf_rejected_before_render`，经 `parse_pdf` 全链断言"渲染前返回 <30s"）——历史"负向疑似缺失"已补。但 `probe_pdf_pages` 失败 `unwrap_or(1)` 引入新问题（PARSE-001） | 代码 + 测试静态确认；运行时佐证不可用（Lead 日志截断） |
| R3-S2-1 run_limited join 无界 | **已修复**：`guard.rs:214-297` channel 限时接收(1.5s)+补杀进程组；内联负向 `subprocess_timeout_kills_and_classifies` 在位 | CONFIRMED |
| R3-S2-2 TmpGuard 非唯一 | **已修复**：`guard.rs:318-331` 目录名 = tag+pid+全局计数+纳秒（调用级唯一）；`tmpguard_recovers_on_all_paths` 在位 | CONFIRMED |
| R3-S3-1 zip64/EOCD 家门 | **已整体迁出本 crate**：guard.rs(529 行) rg 零命中 zip64/EOCD/压缩比/条目名/XML 深度门；承接面 = bid-engine reader.rs 容器三门→`EngineRejected`（`reader.rs:57-70` PrescanRejected/ContainerCorrupt 映射在位）→ mdcore office-io（外部 rev，**不可本仓核**） | 部分 CONFIRMED（见 PARSE-017） |
| R3-S3-2 PDF 无字节门 | **已修复**：`engines/mod.rs:67` + `guard.rs:35` `LIMIT_INPUT_BYTES_PDF=500MB`（R3 建议 512MB，实现 500MB——差 12MB，语义等价，取值未与契约文档对齐） | CONFIRMED |
| R3-P2-4 stderr drain | **已修复**：`guard.rs:238-256` 持续 drain 到 EOF 保留头 64KB；且注释修正了 R3 对危害机理的误判（关 FD 后是 EPIPE/SIGPIPE 中杀而非"永久阻塞"）——修正描述与 `subprocess_stderr_flood_drained_not_blocked` 负向一致 | CONFIRMED |
| #80 OCR 零块静默导入 | **已修复一半**：`tesseract.rs:170-176` `ocr_fail_pages>0 && ocr_blocks==0` → Err；**白扫（全页成功零文本）与 pdftoppm 成功零 PNG 两条路仍是 Ok+0 块**（PARSE-010） | CONFIRMED |
| 历史体检 Top① PDF 行级碎片否决漏检 | **已修复（有回归锁定）**：`extract_pdf_para.rs`（跨行否决重组/断段/词表/例外从句/语境传播/幂等 9 用例）+ `extract_pdf_l2gap.rs`（10 用例）；续行窗 = 1 行（extract.rs:155）。真实水利件重跑需 Lead 环境——历史 0.386 等数字不引用为当前值 | 静态 CONFIRMED，运行时 NEEDS_RUNTIME_VERIFY |
| 历史体检 Top④ PDF 面表格丢失 | **当前仍存在且根因精确定位**：poppler 通道 `tables` 恒 0（PARSE-005 死指标）；rust_fallback `tables: 0` 硬编码（pdf_text.rs:1690）；OCR `tables: 0` | CONFIRMED |
| 历史体检⑩ pdfium 缺失静默降级无横幅 | **当前仍存在**：`service.rs:377`（PARSE-006）；env_preflight 三态探测本可解决但无生产消费方（PARSE-008） | CONFIRMED |
| 历史体检⑤ OCR 通道抽取面坍塌 | 部分缓解（OCR 行 <2 非空白过滤 tesseract.rs:147 + sanitize_line），但 OCR 后处理（词典纠错/版面复原）仍无——扫描件规则命中面问题属 extract 层，B2b 范围 | SUSPECTED |

## 2. 发现清单（问题记录）

---

### PARSE-001
- **严重程度**: P1
- **模块/文件**: bid-parse / `src/engines/tesseract.rs`
- **代码行号**: 87-99（`ocr_parsed`）、236-253（`probe_pdf_pages`）
- **函数**: `TesseractOcr::ocr_parsed` / `probe_pdf_pages`
- **问题描述**: pdfinfo 探测失败（返回 None）时 `unwrap_or(1)`——①MAX_OCR_PAGES=200 门形同虚设（real_pages=1 恒不过门）；②渲染命令带 `-l 1` 只渲染**第 1 页**。一个 300 页纯扫描件若 pdfinfo 失败而 pdftoppm 成功（损坏 xref、非常规头等容忍度差异），整档只 OCR 第 1 页并以 `Ok(ParsedFile{pages:1, …})` 返回，summary 报"pdf 扫描层 OCR: 1 页，N 行块（chi_sim）"——**无任何"页数探测失败/内容截断"标记**。
- **调用链**: `parse_pdf`(mod.rs:118) → `ocr_parsed`(tesseract.rs:87) → `probe_pdf_pages` 失败 → 渲染 `-l 1` → 单页 OCR → Ok 部分导入
- **触发条件**: 扫描件 + pdfinfo NonZeroExit/解析失败（`is_success()` false 即 None）且 pdftoppm 能渲染
- **实际影响**: 整档内容静默缩水为第 1 页；否决条款/控制价等主体内容全部不进抽取面而导入"成功"（blocker=0 的静默丢失面，与 #103 补遗丢失同型）
- **复现步骤**: 构造 xref 损坏但内容流完好的多页扫描 PDF → `parse_file` → 观察返回 Ok 且 pages=1
- **当前测试**: 无。`oversized_scanned_pdf_rejected_before_render` 只测 pdfinfo **成功**路径的门生效
- **缺失测试**: pdfinfo 失败注入（如 mock/PATH 遮蔽 pdfinfo）+ 多页件断言"必须失败或全页处理，不得单页成功"
- **建议方案**: pdfinfo 失败时不应默认 1——应显式 Err（"页数探测失败，拒绝处理"）或以 pdftoppm 全量渲染 + 事后 pngs.len() 对账；至少在 summary/错误面登记"探测失败，按 N 页处理"
- **证据**:
```rust
let real_pages = probe_pdf_pages(path).unwrap_or(1);   // tesseract.rs:87
if real_pages > MAX_OCR_PAGES { return Err(...) }       // 恒 1 ≤ 200 → 门失效
render_cmd.args(["-l", &real_pages.to_string()])        // :99 只渲染第 1 页
```
- **审计置信度**: CONFIRMED（代码路径确定；端到端触发需构造损坏件，NEEDS_RUNTIME_VERIFY 部分）
- **状态分类**: 当前仍存在

---

### PARSE-002
- **严重程度**: P1
- **模块/文件**: bid-parse / `src/engines/mod.rs`
- **代码行号**: 75-119（`parse_pdf` 文本层路由门）
- **函数**: `parse_pdf`
- **问题描述**: 文本层/扫描层路由门是**文件级**总量判定：poppler 输出全文非空白字符 ≥16 → 全件按文本层导入，**无逐页混合路由**。混合"前 N 页文本 + 后 M 页扫描图"的招标件（历史体检真实形态）：文本页进块流、扫描页 0 内容 0 信号——OCR 永不被触发，也没有"ParsedFile.pages(=非空\f段数 5) vs pdfinfo 真实页数(200)"的异常对账诊断。`engine.rs` 注释自认 R3(CrossValidate 逐页对比)未落地。附带：poppler 通道 `pages` 口径 = 非空分段数（poppler.rs:82-86），混合件页数失真且无真页数对账。
- **调用链**: `parse_file` → `parse_pdf`:75 poppler Ok → `non_ws<16`?否 → 直接 `text_layer_parsed` → Ok
- **触发条件**: 任一"部分页有文本层、部分页纯图像"的 PDF（常见：正文可复制+盖章页/证书页扫描）
- **实际影响**: 扫描页条款（常是否决项所在盖章页）静默缺失；用户看到"pdf 文本层: 5 页 N 行块"无从知道还有 195 页
- **复现步骤**: 文本页 + 扫描页混合 PDF → `parse_file` → blocks 仅文本页
- **当前测试**: 无任何混合路由测试（corpus_probe B1/B2 是纯文本/纯扫描单形态）
- **缺失测试**: 混合件 fixture（1 页文本 + 2 页图像）断言：或逐页 OCR 补齐、或导入失败/显式 partial 状态
- **建议方案**: 短期：poppler 页数 vs pdfinfo 页数不一致时在 parse_summary 显式登记（"文本层仅覆盖 X/Y 页"）；中期：R3 逐页 CrossValidate + 图像页计数触发 OCR
- **证据**:
```rust
let non_ws: usize = pages.iter()  // mod.rs:78-81 文件级总量
    .map(|p| p.text.chars().filter(|c| !c.is_whitespace()).count()).sum();
if non_ws < 16 { …OCR… }           // :82 全有或全无
```
- **审计置信度**: CONFIRMED
- **状态分类**: 当前仍存在（历史体检问题，R3 计划在注释中登记未实施）

---

### PARSE-003
- **严重程度**: P1
- **模块/文件**: bid-parse / `src/pdf_text.rs`
- **代码行号**: 696-701（`inflate`）、671-694（`decode_stream`）、563-603（`iter_streams`）、1863-1873（`fonts_used_by_tf`）
- **函数**: `inflate` / `decode_stream` / `iter_streams`
- **问题描述**: 纯 Rust 兜底的 FlateDecode 解压**无输出字节上限**（`read_to_end` 无界）。guard 只限输入 500MB，不限解压后体积——500KB 恶意 PDF 的单个流可膨胀至数十 GB → 进程 OOM。对照：office-io 引擎侧对 OOXML 有"声明值门/实际读取门"家族，PDF 纯 Rust 路无等价物。放大因素：①`object_stream_bytes`(CMap)、`fonts_used_by_tf`(乱码探测) 也各自全量解压；②`cid_cjk_font_in_use` 在 `probe_cjk_garble_pages` 内被反复调用（primary_probe / fb_probe / append_page_diagnostics 至少 3 次/件），每次都重扫重解压全部流——大文件 happy path 也承受多次 O(file) 扫描。
- **调用链**: `parse_pdf` → `probe_cjk_garble_pages`(mod.rs:123) → `cid_cjk_font_in_use` → `fonts_used_by_tf` → `decode_stream` → `inflate`（无界）；`fallback.extract_parsed` → `pdf_text::extract` → 同路
- **触发条件**: 恶意/损坏 PDF（高压缩比流）；或正常大件（性能面）
- **实际影响**: DoS（内存耗尽）/大件解析延迟数倍
- **复现步骤**: 构造 `%PDF-` + 单 Flate 流（zip-bomb 式 1000:1 压缩比）→ 无 poppler 环境走兜底 → 观察 RSS
- **当前测试**: 无（guard 的输入门测试只测 4MB>2MB 输入面，无解压输出面测试）
- **缺失测试**: 高压缩比 PDF 负向（断言 Err/内存有界而非 OOM）
- **建议方案**: `inflate` 加解压上限（如 256MB/流），超限计 `streams_skipped` 或 Err(Unsupported)；`cid_cjk_font_in_use` 结果按文件 sha/字节缓存或提取一次复用
- **证据**:
```rust
fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut dec = ZlibDecoder::new(data);
    dec.read_to_end(&mut out).ok()?;   // pdf_text.rs:699 无任何 take/上限
    Some(out)
}
```
- **审计置信度**: CONFIRMED（无上限是确定的；实际 OOM 需运行时验证）
- **状态分类**: 当前仍存在

---

### PARSE-004
- **严重程度**: P1
- **模块/文件**: bid-parse / `src/env_preflight.rs`（+ 消费它的全部测试）
- **代码行号**: 298-311（`skip_if_env_missing`）、330-348（`skip_if_fixture_missing`）、模块文档 11-12
- **函数**: `skip_if_env_missing` / `skip_if_fixture_missing`
- **问题描述**: SKIP 机制 = eprintln 横幅 + 提前 return → **cargo test 计数显示 passed**（模块文档自认）。三个叠加风险：①libtest 默认捕获输出，`cargo test`（无 `--nocapture`）下横幅**不可见**——SKIP 与真通过在 CLI 观感上零差别；②无任何聚合计数/汇总行/退出码信号——环境整体退化（CI 容器丢 poppler/tesseract/chi_sim/夹具）时，PDF 测试面（corpus_probe、real_corpus_223、numbering/nesting_corpus、addendum 的部分面、bid-mcp/bid-tools/bid-grade 大片消费方）**全部静默变绿**，无一处会红；③消费面极广（grep：bid-mcp 20+ 测试文件、bid-tools 7、bid-grade 2、bid-parse 5）。
- **调用链**: 各测试入口 → `skip_if_env_missing(test_id, FULL_PDF_CHAIN)` → eprintln → return → libtest 记 passed
- **触发条件**: CI/开发环境缺任一外部工具或夹具
- **实际影响**: 假绿——PDF 链路的回归防护在环境退化时整体蒸发且不可见；与"测试必须证明代码完成业务行为"的审计纲领冲突（issue #9 是有意取舍，但缺聚合披露面）
- **复现步骤**: `BID_PREFLIGHT_PATH=/tmp/empty cargo test -p bid-parse --test corpus_probe` → 显示 passed（横幅仅在 --nocapture 或失败时可见）
- **当前测试**: env_preflight_sim.rs 验证 SKIP 判定一致性（横幅数据源），但**不验证可见性/聚合面**
- **缺失测试**: 无（机制本身缺的是运行面不是测试面）
- **建议方案**: 测试二进制结束时输出 SKIP 总计并写入 sidecar 文件（CI 门禁断言 SKIP 数=0 或阈值）；或迁移到 `#[ignore]` + env 探测自动反 ignore 的 nightly 特性一旦可用
- **证据**:
```rust
// env_preflight.rs:11-12 stable Rust 的 libtest 无运行时 ignore 面——本模块以
// 「响亮横幅 + 提前 return」实现（测试计数显示 passed，但输出首屏即见 SKIP…
eprintln!("SKIP: {test_id}");  // :305 捕获模式下默认不可见
```
- **审计置信度**: CONFIRMED（设计性，文档自认）
- **状态分类**: 当前仍存在（文档与实现一致，但与测试纪律目标冲突）

---

### PARSE-005
- **严重程度**: P2
- **模块/文件**: bid-parse / `src/engines/poppler.rs`
- **代码行号**: 100-108（`text_layer_parsed`）、124-139（`collapse_spaces`）
- **函数**: `text_layer_parsed`
- **问题描述**: PDF 文本层 `tables` 计数是**结构性恒 0 的死指标**：块文本先过 `collapse_spaces`（把任意空白游程折叠为单空格，输出不可能含 "  " 双空格），之后才 `filter(|b| b.text.contains("  "))` 计数——条件永假。rust_fallback（pdf_text.rs:1690）与 OCR（tesseract.rs:181）更是硬编码 `tables: 0`。三通道 PDF `tables` 一律 0，且该值上行到用户可见面（bid-mcp tools.rs:470 import 响应 `"tables": o.tables`、bid-tools service.rs:347 落库）。
- **调用链**: `text_layer_parsed`(poppler.rs:96-102 collapse 先) → :105-108 计数（永 0）→ `ParsedFile.tables` → tools.rs:470 用户可见
- **触发条件**: 一切走 bid-parse 链的 PDF
- **实际影响**: 历史体检 Top④"PDF 面表格结构丢失"在 legacy 通道的精确根因之一——评分办法表格整面不可结构化的可见信号失真（用户看到 tables:0 无法区分"无表格"与"计数器坏死"）
- **复现步骤**: 解析任何含多列空白对齐表格的 PDF（如 B1-设备清单.pdf）→ `tables == 0` 恒成立
- **当前测试**: corpus_probe 只断言 DOCX 的 `a.tables>=5`，PDF 的 tables 无断言（死指标因此漏网）
- **缺失测试**: PDF 件 tables>0 的正断言（会立即红，暴露死指标）
- **建议方案**: 若 -layout 双空格判表格语义仍要保留，须在 collapse **前**计数；或删掉该启发式改为 summary 明示"PDF 通道不做表格结构化"（诚实登记优于死数据）
- **证据**:
```rust
text: collapse_spaces(&cleaned),           // poppler.rs:100 先折叠
...
let tables = blocks.iter()
    .filter(|b| b.kind == BlockKind::PdfLine && b.text.contains("  "))  // :107 永假
    .count();
```
- **审计置信度**: CONFIRMED
- **状态分类**: 当前仍存在

---

### PARSE-006
- **严重程度**: P2
- **模块/文件**: bid-tools / `src/service.rs`（B2a 侧登记：engines/mod.rs 的直接上层调用方）
- **代码行号**: 376-377（`real_eval.rs:38` 同款）
- **函数**: import 路由
- **问题描述**: pdfium 引擎通道错误在降级点被 `.or_else(|_| parse_file(...))` **整体静默丢弃**——无日志、无 summary 标记、无统计。pdfium 缺失/加载失败/特定件 EngineFailed 时直接落旧链，用户与运维均不知道首选路从未工作（历史体检工具缺陷⑩"#9 登记不伪装纪律冲突"，至今未修）。EnvPreflight 三态（PARSE-008）本可提供该披露但未接线。
- **调用链**: service.rs:376 `parse_pdf_via_engine(...).or_else(|_| parse_file(...))`
- **触发条件**: 无 pdfium / pdfium 打开失败 / MappingGap
- **实际影响**: 环境性静默降级不可观测；同文件跨环境走不同通道 → 块身份漂移（PARSE-007 联动）
- **复现步骤**: 无 MDC_PDFIUM_LIB 环境导入 PDF → 成功，无任何引擎失败痕迹
- **当前测试**: 无（无任何断言降级可观测性）
- **缺失测试**: 引擎失败注入 → 断言 summary/日志含降级标记
- **建议方案**: `.or_else(|e| { log/记入 parse_summary("引擎通道失败: {e}，已降级旧链"); parse_file(...) })`
- **证据**:
```rust
"pdf" => bid_engine::parse_pdf_via_engine(&spec.upload_ref)
    .or_else(|_| parse_file(&spec.upload_ref, &spec.filename)),  // service.rs:376-377
```
- **审计置信度**: CONFIRMED
- **状态分类**: 当前仍存在（历史已登记项，B2b 应正式收编）

---

### PARSE-007
- **严重程度**: P2
- **模块/文件**: bid-parse / `src/pdf_text.rs`（locator 口径）、`src/engines/poppler.rs`、`src/engines/tesseract.rs`、`src/lib.rs`
- **代码行号**: pdf_text.rs:1608-1631（`parse_via_fallback` 块/locator 构造）、2191-2203（`page_no_of_locator` 三格式自认）；poppler.rs:99；tesseract.rs:155；lib.rs:29-30；bid-kb/src/lib.rs:96-109（chunk_id=sha256(file_sha:idx)）
- **函数**: 各通道块装配
- **问题描述**: **同一文件走不同解析通道产生不同块身份/定位**：①locator 三格式（"第N页 行M" / "第N 页 > 第M 行" / "第N页 OCR 行M"）；②页语义不一致——poppler=\f 分段（真页）、兜底=**文本流组序（伪页**，pdf_text.rs:1617 直接 enumerate streams，无页树解析）、OCR=渲染张序；③块数/顺序/切行口径不同（历史体检：pdfium 8984 vs pdftotext 8107）；④下游 `chunk_id = sha256(文件哈希:块序)`（bid-kb）随通道漂移——同一 sha256 文件在不同环境/引擎可用性下生成完全不同的 KB 分片与证据锚点。lib.rs:29 的确定性契约（"同输入文件→同输出 blocks"）**未声明"仅在同环境同通道"前提**。
- **调用链**: parse_file → 通道分支 → Block{idx, locator} → bid-kb chunk_id → KB 分片/证据锚/影响面分析
- **触发条件**: 环境差异（pdfium 有无）或引擎通道退化（乱码降级/OCR 救援）
- **实际影响**: 跨环境跑 KB/评测不可比；补遗影响面/覆盖对账跨环境漂移（历史"covered 跨跑 2 块漂移"疑似同根）
- **复现步骤**: 同一 PDF 分别在有/无 poppler 环境解析 → 对比 blocks.len()、locator 前缀、chunk_id
- **当前测试**: pdf_cid_identity.rs 有"双态断言"（任何环境 Ok 含关键词）但**不锁块数/locator 一致性**（有意不锁——版本差异入组件矩阵）
- **缺失测试**: 跨通道块身份差异的显式登记测试（或文档化放弃声明）
- **建议方案**: 短期在 lib.rs 确定性契约补"通道前提"声明 + parse_summary 已有自报（良好）；中期锚点用内容哈希而非 (file_sha, idx)
- **证据**:
```rust
locator: format!("第{} 页 > 第{} 行", pno + 1, lno + 1),  // pdf_text.rs:1625 伪页=流组序
// page_no_of_locator 注释:2190 兼容三种在产格式：`第1页 行2`…`第1 页 > 第2 行`…`第2页 OCR 行5`
```
- **审计置信度**: CONFIRMED
- **状态分类**: 当前仍存在（部分为内容寻址的固有属性，登记为设计偏好）

---

### PARSE-008
- **严重程度**: P2
- **模块/文件**: bid-parse / `src/env_preflight.rs`
- **代码行号**: 255-284（`pdf_env_status`/`detect_pdf_env_status`/`pdf_env_report`）、224-253（`pdfium_available_impl`）
- **函数**: 上述 L1 三态探测族
- **问题描述**: ①三态探测（Full/PdfiumOnly/None）与人类可读回报**无任何生产消费方**——grep 全仓仅 env_preflight_pdfium.rs 测试与自身内联测试引用；import 链（service.rs）不咨询三态而是静默降级（PARSE-006）——"报错信息准确"的价值承诺未接线，属死代码面。②pdfium 探测口径 = `is_file()` 文件在位，实际使用 = dlopen 加载初始化（reader.rs:310-318 `PdfiumEngine::open_with_library`）——损坏/架构不符的 libpdfium.so **过探测**但使用必败 → 探测面与使用面口径分叉（TOCTOU 变体）。
- **调用链**: （无生产调用链——这正是问题）
- **触发条件**: 损坏 so 文件 / 探测与使用间环境变化
- **实际影响**: L1 预检给运营的"环境健康"结论可能与实际解析能力不符；三态文案承诺（Lead 点名的"用户照报错装 poppler 是白费功夫"）实际用户根本看不到
- **复现步骤**: `touch bin/lib/libpdfium.so`（空文件）→ `pdfium_available()==true` → 引擎 open 失败 → 静默降级
- **当前测试**: env_preflight_pdfium.rs 12 用例全绿但全部测**函数自身**，无一断言生产接线
- **缺失测试**: import 链路在 PdfiumOnly/None 下的用户可见回报测试
- **建议方案**: 在 service import 失败路径或 MCP initialize 报告接入 `pdf_env_report()`；pdfium 探测改为"尝试 dlopen 探测"或注明仅文件面
- **证据**:
```rust
if !p.trim().is_empty() && std::path::Path::new(p).is_file() { return true; }  // :227 只查存在
// rg detect_pdf_env_status 全仓命中：env_preflight_pdfium.rs（测试）+ env_preflight.rs（自身）——零生产调用
```
- **审计置信度**: CONFIRMED
- **状态分类**: 文档与实现不一致（能力在、接线无）

---

### PARSE-009
- **严重程度**: P2
- **模块/文件**: bid-parse / `src/engines/tesseract.rs`
- **代码行号**: 137-141（OCR 页循环）
- **函数**: `TesseractOcr::ocr_parsed`
- **问题描述**: 单页 tesseract `NonZeroExit` → `break`——**失败页之后的所有页不再尝试**。部分成功降级本身是登记过的设计（"扫描页残缺属数据面"），但 summary 只报"pdf 扫描层: N 页，OCR 失败（stderr 头）"，**不披露失败页号与被放弃的页范围**——10 页件第 3 页坏 → 4-10 页静默丢失，仅剩含糊一行。
- **调用链**: `ocr_parsed` 页循环 → 第 k 页 NonZeroExit → break → Ok(部分 blocks)
- **触发条件**: 任一页 OCR 崩溃（坏图/内存/语言数据边界）
- **实际影响**: 部分导入的内容边界不可审计（哪页丢了不知道）；续页可能本来能成功
- **复现步骤**: 多页扫描件中间页为异常图像 → 观察 blocks 只到中间页
- **当前测试**: 无
- **缺失测试**: 中间页失败注入 → 断言 summary 披露失败页号/被放弃页数
- **建议方案**: continue 而非 break（逐页尝试+计数）；至少 summary 记 "第 k 页失败，k+1..N 页未处理"
- **证据**:
```rust
RunOutcome::NonZeroExit { stderr_head, .. } => {
    ocr_fail_pages += 1;
    summary = format!("pdf 扫描层: {pages} 页，OCR 失败（{stderr_head}）");
    break;                                   // tesseract.rs:140 后续页全弃
}
```
- **审计置信度**: CONFIRMED
- **状态分类**: 当前仍存在

---

### PARSE-010
- **严重程度**: P2
- **模块/文件**: bid-parse / `src/engines/tesseract.rs`
- **代码行号**: 106-115（pngs 空判）、161-187（返回 Ok）、164-176（#80 堵漏）
- **函数**: `TesseractOcr::ocr_parsed`
- **问题描述**: 两条"零产物 Ok"路径未被 #80 堵漏覆盖：①**白扫**——全部页 OCR Success 但输出零有效行（注释自认"诚实空"不在此列）；②pdftoppm 退出码 0 但产出 **0 个 PNG**（`pngs.is_empty()` → pages 保持 1，循环不进，blocks=[]）。两路都返回 `Ok(ParsedFile{blocks:[], ocr_blocks:0})`——导入成功零内容。下游 status=OcrRequired（has_text_layer=false）是唯一兜底信号，但零块导入本身无独立失败面/告警。
- **调用链**: parse_pdf → ocr_parsed → Ok(空) → service.rs:382 OcrRequired 落库
- **触发条件**: 全白/全噪扫描件；pdftoppm 容错渲染出 0 张图
- **实际影响**: "导入成功但 0 内容"与"导入失败"在用户面难以区分（依赖下游 readiness 分级是否到位——#184 属 B2b 面）
- **复现步骤**: reportlab 生成全空白扫描 PDF（无字）→ OCR 全页 Success 零行 → Ok
- **当前测试**: `oversized_scanned_pdf_rejected_before_render` 的 201 页空白件在门处被拒——**恰好绕过了白扫路径**（201>200），白扫负向（≤200 页全白件）无测试
- **缺失测试**: ≤200 页全白扫描件 → 断言 Err 或显式 OcrRequired+告警事件
- **建议方案**: `blocks.is_empty() && ocr_fail_pages==0 && 全页 Success` → 返回 Err("OCR 零产出（疑空白扫描件）")或强制 OcrRequired+事件
- **证据**:
```rust
if ocr_fail_pages > 0 && ocr_blocks == 0 { return Err(...) }  // :170 只堵 fail_pages 面
Ok(ParsedFile { pages, tables: 0, has_text_layer: false, ocr_blocks, blocks, … })  // :177 零块照过
```
- **审计置信度**: CONFIRMED
- **状态分类**: 当前仍存在（设计取舍"诚实空"，但与 #80 精神边界模糊）

---

### PARSE-011
- **严重程度**: P2
- **模块/文件**: bid-parse / `src/engines/poppler.rs`（+ extract 层联动）
- **代码行号**: poppler.rs:87-104（`text_layer_parsed` 行装配）；tesseract.rs:147（对照：OCR 通道有 <2 非空白过滤）
- **函数**: `text_layer_parsed`
- **问题描述**: PDF 文本层通道**无页眉/页脚/页码噪声过滤**——所有非空行（含每页重复的页眉、页码孤行"- 3 -"、"3"）原样成块；且与 OCR 通道的噪声门不一致（OCR 滤 <2 非空白字符行，poppler 通道不滤）。页码短行可被 extract.rs 续行窗（1 行）吸收进上一条款尾部污染文本；每页重复页眉持续重置章节语境面。
- **调用链**: text_layer_parsed → Block 流 → extract.rs:443 段落预计算（续行窗 1 行）→ 条款文本污染
- **触发条件**: 带页眉页脚的真实 PDF（几乎所有招标件）
- **实际影响**: 条款抽取噪声/误并；跨通道块数差异的来源之一（PARSE-007 联动）
- **复现步骤**: 解析带页码的 PDF → 块流含页码孤行
- **当前测试**: extract_pdf_para.rs `toc_line_defense` 只防目录行（引导点形态），无页眉页脚用例
- **缺失测试**: 页眉页脚 fixture 断言被过滤或至少不入条款
- **建议方案**: 至少把 OCR 的 <2 非空白过滤对齐到 poppler 通道；页眉（页间重复行）去重属 extract 层，建议登记 B2b
- **证据**:
```rust
for line in pt.text.lines() {
    let l = line.trim();
    let cleaned = sanitize_line(l);
    if cleaned.is_empty() { continue; }   // poppler.rs:92 只滤空——"3" 这类页码行照过
```
- **审计置信度**: CONFIRMED（污染程度需语料验证）
- **状态分类**: 当前仍存在

---

### PARSE-012
- **严重程度**: P3
- **模块/文件**: bid-parse / `src/engines/poppler.rs`
- **代码行号**: 22-34（`probe`）、57-76（`map_run_err`）
- **函数**: `PopplerText::probe` / `map_run_err`
- **问题描述**: probe 判定：除 Spawn 外**一律 Ready**（含 Timeout/OutputOverflow——挂死的 pdftotext 也算"在位可用"）；`map_run_err` 用中文字符串 `contains("无法启动")`/`contains("退出码")` 做错误分类——脆弱的文本协议（run_limited 文案改动即静默错分类为 Governed）。当前实际影响有限（probe 仅 OCR 救援路径用）。
- **审计置信度**: CONFIRMED
- **状态分类**: 当前仍存在

---

### PARSE-013
- **严重程度**: P3
- **模块/文件**: bid-parse / `src/pdf_text.rs`
- **代码行号**: 548-550（`looks_like_pdf`）、115-118（加密检测）
- **函数**: `looks_like_pdf` / `extract`
- **问题描述**: ①`%PDF-` 头窗仅前 ~1028 字节——超长垃圾前缀件误判 NotPdf（规范建议 1024 内，可接受但无登记）；②`contains(bytes, b"/Encrypt")` 字节级检测：内容流里出现字面 "/Encrypt" 文本（如 PDF 教学文档）→ 假阳性 Encrypted 拒收；/Encrypt 压进 ObjStm → 假阴性漏检（后续靠 pdftotext 失败兜住，语义仍可达）。
- **审计置信度**: CONFIRMED（边界形态）
- **状态分类**: 当前仍存在

---

### PARSE-014
- **严重程度**: P3
- **模块/文件**: bid-parse / `src/guard.rs`
- **代码行号**: 284-297（1.5s 收流宽限）、167-182（`kill_process_tree`）
- **函数**: `run_limited`
- **问题描述**: ①子进程退出后 stdout 收流宽限 1.5s——64MB 输出在高负载/慢管道下排空可能超时 → 合法成功被误判 Timeout（竞态窗口小）；②setsid 脱组的孙进程逃过 `kill(-pgid)`（R5-P3 残余在位，threat-model §4.2 有注记）且持管道读线程滞留（detached 有界泄漏）。均为已知边界、有文档登记。
- **审计置信度**: ①SUSPECTED ②CONFIRMED（历史已登记残余）
- **状态分类**: 当前仍存在（低危登记项）

---

### PARSE-015
- **严重程度**: P3
- **模块/文件**: bid-parse / `src/engines/mod.rs` + `src/lib.rs`
- **代码行号**: mod.rs:65-69（check_input_size→fs::read）、lib.rs:29-30
- **函数**: `parse_pdf`
- **问题描述**: ①输入门(metadata)与 fs::read 之间文件可增长（TOCTOU 窗口）且全量读入内存（500MB 上限 × 并发 4 席 + poppler 输出 64MB，guard.rs:187-190 自注内存上界 ~0.75GB）——上界有登记但 parse_pdf 里 raw 与 fallback 再读一次共 2 次全量；②lib.rs:8-10 对 PDF 首选路描述含混（"pdfium 引擎优先，映射表 §3"指 service 层，本 crate 链是 poppler①）——读者易误解；`engine.rs registry()` 的 FirstSuccess 顺序仅测试消费（装饰性抽象，R3/R4 扩展点）。
- **审计置信度**: CONFIRMED
- **状态分类**: 当前仍存在（文档表述类）

---

### PARSE-016
- **严重程度**: P3
- **模块/文件**: bid-parse / `src/engines/tesseract.rs`
- **代码行号**: 96-101（渲染参数）、122-127（OCR 参数）
- **函数**: `ocr_parsed`
- **问题描述**: OCR 治理细节登记：①`-r 200 -gray -png` 200 页渲染 PNG 峰值磁盘无配额（约数百 MB，TmpGuard 只保证事后回收）——MAX_OCR_PAGES 是间接门；②语言参数仅 `chi_sim`（无 eng 组合，纯数字/字母密集页召回依赖 chi_sim 的 ASCII 面）；③`--psm 6` 固定单栏假设（双栏扫描件行序错乱——corpus-R D5a"全扫描双栏"形态）；④每页 30s 超时 × 200 页无整件总预算（最坏 ~100 分钟/件）。
- **审计置信度**: CONFIRMED（参数事实；召回影响 NEEDS_RUNTIME_VERIFY）
- **状态分类**: 当前仍存在

---

### PARSE-017
- **严重程度**: P2
- **模块/文件**: bid-parse / `src/guard.rs`（防护真空核对项）
- **代码行号**: 1-22（模块文档：B4 收敛声明）、31-35（保留面常量）
- **函数**: （架构面）
- **问题描述**: 防护迁移核对结论：guard.rs 现存 = PDF 输入字节门(500MB) + 流式 sha256 + 子进程治理全套 + TmpGuard（11 内联测试：输入门 1 + sha256 1 + 子进程 9）。zip/EOCD/压缩比/条目名/XML 深度/xlsx 双轴门在本 crate **确认不存在**（rg 零命中）——承接方：①bid-engine reader.rs 容器三门→EngineRejected 映射**在位**（reader.rs:57-70）；②mdcore office-io（`mdc-office-io` git rev `f4328ebf…`）**源码不在本机**——门数值（LIMIT_INPUT_BYTES 2M vs 4M 取严者、行/格双轴等）与 R5 时代等价性**无法在本仓验证**。threat-model.md T-03/T-04 行的测试计数（"18+3/9 测试"）与现状（guard 11）不符——历史已登记的文档失真仍在。
- **触发条件**: 审计验收"防护无真空"论断时
- **实际影响**: 本仓静态结论只能到"移交在位、引擎侧映射在位"，**等价性存证缺失**——若 mdcore rev 回退或门数值变动，本仓无任何测试会红
- **建议方案**: 移交 B3/B4 侧代理：核 mdcore@f4328ebf office-io 门族数值与 R5 负向测试等价性；回填 threat-model T-03 计数
- **审计置信度**: 部分 CONFIRMED（本侧缺失确定）+ NEEDS_RUNTIME_VERIFY（跨仓）
- **状态分类**: 当前仍存在（跨仓核对项）

---

### PARSE-018
- **严重程度**: P3
- **模块/文件**: bid-parse / 测试佐证面
- **问题描述**: Lead 运行时佐证不可用——`/home/z/my-project/logs/03-bidcore-cargo-test.log` 仅 15 行编译开头（无 test result、无 SKIP 横幅），本模块所有"运行时验证"级结论降级为静态 CONFIRMED / NEEDS_RUNTIME_VERIFY。建议 Lead 重跑 `cargo test -p bid-parse -- --nocapture` 并保留完整日志（SKIP 规模同时也是 PARSE-004 的直接证据）。
- **审计置信度**: CONFIRMED（日志事实）
- **状态分类**: 尚无法复现（佐证通道故障）

---

## 3. 重点问题逐条回答（任务书六问）

**Q1 pdf_text.rs 输入面与失败语义**：输入面=四滤链(ASCII85/Hex/RunLength/Flate)+ToUnicode CMap(bfchar/bfrange，按字体路由#218)+UniXX-UCS2 恒等解码(#200)+Tm/CTM 定位行合成(含 Tm 缩放#218)+BT/ET 窗口+UTF-16BE/WinAnsi 判定+行级控制符清洗(#223A)+分页多数决乱码探测(#223B)。失败语义**全部明说**（NotPdf/Encrypted/NoTextLayer/Unsupported + 零 CJK=Garbled fail-loud + summary 自报兜底/乱码/流组计数）——无静默空输出通道；纯控制符件清洗后诚实报无文本层（有测试）。**硬缺口**：解压无上限（PARSE-003）、GBK-EUC/Adobe-CID 序号族不硬解（登记为能力边界+hint，正确的取舍）、ObjStm 字典扫不到（登记）、页=流组序的伪页定位（PARSE-007）、无页眉页脚过滤（PARSE-011）、混合扫描+文本无逐页分流（PARSE-002，此为仲裁器 mod.rs 的门）。

**Q2 engines/tesseract**：页数门已修（probe_pdf_pages 真页数+`-l` 双保险），">200 页拒绝"负向测试**在位**（tesseract.rs:281，依赖 python3+reportlab，缺件 eprintln skip 计 passed——SKIP 掩盖面之一）；**但 pdfinfo 失败 unwrap_or(1) 使门失效且只渲染第 1 页（PARSE-001，P1）**。语言=chi_sim 单语言；超时=渲染 180s/页 OCR 30s/整件无总预算；子进程=进程组杀+wait 收割（无僵尸面）+setsid 残余（PARSE-014②）；stderr=drain 修复在位。降级链判定顺序：service 层 pdfium→(静默丢错)→bid-parse{poppler→兜底/OCR}——**降级丢失错误 CONFIRMED（PARSE-006）**，bid-parse 内部降级均有 summary 自报/稳定错误标签（[pdf-*] 8 种），内部链合格、外层链断点。

**Q3 guard.rs 防护面**：现存=500MB PDF 输入门（读前 metadata 预检）+流式 sha256+子进程治理（超时杀组/输出上限/stderr drain/五分类/并发 4 席/TmpGuard 唯一名）。已移走=zip 全家门（确认 rg 零命中；reader.rs 三门映射在位；mdcore office-io 不可本仓核→**防护真空结论：本仓内无真空证据，跨仓存证缺失，移交 B3x**）。本侧剩余覆盖评估：PDF 魔数/扩展名一致性无显式门但失败语义可达（pdftotext 失败→NotPdf 兜底，addendum_recovery.rs 有负向）；加密无预检门（/Encrypt 字节检测假阳/假阴 PARSE-013）；**解压输出无门是本侧最大真空（PARSE-003）**。

**Q4 env_preflight SKIP**：假绿风险 **CONFIRMED（P1，PARSE-004）**——横幅+return+计数 passed、无聚合/退出码、默认 capture 下横幅不可见、消费面横跨 5 crate 数十测试；TOCTOU：探测(--version/is_file) vs 使用(实际参数/dlopen) 口径分叉（PARSE-008②），chi_sim 探测与消费同口径（有锚定测试）。

**Q5 测试质量**（逐文件见 §4 表格）：pdf_cid_identity.rs 是**唯一环境可移植强断言面**（双态断言+fail-loud 标签绑定+边界族 hint），质量最高；addendum_recovery/real_corpus_223 双路钉死+真件黄金例+诚实 SKIP；extract_pdf_para/l2gap=合成行块锁定历史 P0 修复（oracle=人工核对基线+GT 词表）；corpus_probe/numbering/nesting=真语料但 SKIP 面大（环境/夹具/CJK 占比三重 skip）；env_preflight_sim=独立进程契约验证（亮点：哨兵原子计数证明业务面不可达）；env_preflight_pdfium=12 用例但零生产接线断言（PARSE-008）。错误路径可达性整体良好（8 个稳定 [pdf-*] 标签均有测试绑定），SKIP 掩盖面=corpus_probe 全部断言、numbering/nesting 的真分布、tesseract 门负向（无 reportlab 时）、OCR 救援头牌（无 chi_sim 时）。

**Q6 块身份稳定性**：**不稳定（CONFIRMED，PARSE-007）**——locator 三格式/页语义三口径（\f 真页 vs 流组伪页 vs 渲染张序）/块数通道差（历史 8984 vs 8107）/chunk_id=sha256(file_sha:idx) 随通道漂移；lib.rs 确定性契约未声明通道前提；parse_summary 自报是唯一可对账信号（在位且良好）。

## 4. 测试文件逐一评估

| 测试文件 | 用例数 | 类型 | Oracle 独立性 | 真实语料 | 错误路径 | SKIP 面 | 评价 |
|---|---|---|---|---|---|---|---|
| pdf_cid_identity.rs | 7 | 集成（合成 PDF 字节级夹具，双向） | 高——断言绑定稳定错误标签+探测三态原语，非镜像实现 | 无（合成形态来自真实事故） | ✅ 4 条 fail-loud 路径全绑定 | **零 SKIP**（环境可移植双态断言——设计典范） | 最强 |
| addendum_recovery.rs | 7 | 集成（真件黄金例+合成） | 高——五事实面人工核对（2026-10-31 登记） | ✅ C-补遗文件.pdf | ✅ 非 PDF/双路败/乱码三态 | 夹具缺→横幅 skip（2 用例受影响） | 强 |
| real_corpus_223.rs | 2 | 真语料回归 | 中高——923/317/409 基线+dump 对照记录（#234 下调有清单） | ✅ 水利/电信 4 件 | 白扫负件在合成面 | 夹具 skip | 强（兜底路钉死规避通道漂移） |
| extract_pdf_para.rs | 9 | 单元/集成（行块合成） | 高——GT 词表+段落重组行为断言 | 无 | 部分（例外/误伤负向） | 无 | 强 |
| extract_pdf_l2gap.rs | 10 | 单元/集成（missed 样例最小复现） | 高——veto-measure 实测基线驱动 | 形态取自真件 | 部分 | 无 | 强 |
| corpus_probe.rs | 1 | 全链冒烟（真夹具 4 件） | 中——断言为规格下限（结构面） | ✅ corpus-A | 无错误路径 | **双重 skip（环境+夹具）——全部断言可被 SKIP 蒸发** | 中 |
| numbering_corpus.rs | 2 | 真语料分布 | 中——下限断言+实测分布表 | ✅ 水科 3 件 | 无 | 语料+ENV_INCAPABLE+CJK 占比三重 skip | 中 |
| env_preflight_sim.rs | 3 | 契约（独立进程） | 高——哨兵原子计数证明业务面不可达 | 无 | N/A | 无 | 强（方法论亮点） |
| env_preflight_pdfium.rs | 12 | 单元 | 中——纯函数注入 | 无 | 无 | 候选目录已有库时 2 用例 skip | 中（**零生产接线断言**） |

## 5. 风险分层汇总

- **P0**：无（未发现直接资损/安全绕过级；PARSE-001/002/003 为内容静默丢失与 DoS 面，按本仓分级列 P1）。
- **P1×4**：PARSE-001（pdfinfo 失败→单页 OCR 静默部分导入）、PARSE-002（混合扫描+文本无逐页路由）、PARSE-003（兜底解压无上限）、PARSE-004（SKIP 假绿无聚合面）。
- **P2×7**：PARSE-005（PDF tables 死指标）、006（引擎降级丢错）、007（块身份跨通道漂移）、008（L1 探测无接线）、009（OCR break 弃后续页）、010（白扫/零 PNG Ok 导入）、011（页眉页脚噪声无过滤）、017（跨仓防护存证缺失——8 项，含 017）。
- **P3×6**：012-016、018。

## 6. 移交与后续

1. **B3/B4 侧**：mdcore@`f4328ebf` office-io 门族数值核对（zip/EOCD 家门等价性）+ threat-model T-03/T-04 计数回填（PARSE-017）。
2. **B2b 侧**：service.rs:377 降级丢错（PARSE-006）与 #184 OcrRequired→readiness 分级对 PARSE-010 的兜底是否闭环；extract 层页眉页脚/续行污染（PARSE-011 后半）。
3. **Lead**：重跑 `cargo test -p bid-parse -- --nocapture`（修复 03 日志截断；SKIP 规模=PARSE-004 直接证据）；真实水利件重跑健康检查以更新历史召回数字（口径警示见 historical-context.md）。

> 审计覆盖：17/17 范围文件全读（src 9 + tests 8，另加读 real_corpus_223.rs 佐证 #223 面）。pdf_text.rs 3283 行分 6 段读全。所有 rg 结论均在本仓 bidcore 工作树复验。
