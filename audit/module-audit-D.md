# 模块审计报告 D —— web-ui / mang-app / m1 / scripts / CI / 语料产物

> 审计员：Agent D（前端·桌面·遗留·测试基础设施）。基线 6105beb（只读）。
> 方法注记：本环境 Bash 工具输出流会剥除 `[hidden]`/`[m.id]` 类方括号序列（曾据此误判"CSS 损坏"，经 Read 工具字节级复核**推翻**）——本报告所有字符串级结论均经 Read 工具二次核验。
> 覆盖：web-ui 全部 TS/HTML/CSS/构建脚本逐文件；mang-app 15 个 src 文件 + 39 内联测试；m1 15 个源文件；scripts 84 文件（重点 12 个全读、其余读头+关键段）；CI/根级配置全读；baseline/artifacts/real-corpus/fixtures/research/raw 全量登记+抽样验证。

---

## A. 文件覆盖清单

### A1. web-ui（186 文件 = 代码 31 + 截图 154 + json 1）

| 文件 | 行数 | 状态 | 发现 |
|---|---|---|---|
| src/main.ts | 3347 | READ | D-02/D-03/D-04/D-08 |
| src/draft.ts | 240 | READ | —（防御完善，边界诚实登记） |
| src/evidence_panel.ts | 983 | READ | D-04/D-05 |
| src/collab_channel.ts | 470 | READ | D-07 |
| src/canvas_view.ts | 1680 | READ | D-04（状态行硬编码英文） |
| src/pdf_view.ts | 2127 | READ | D-04（save/bridge_absent 文案）；innerHTML 仅数值坐标，低风险 |
| src/xlsx_grid.ts | 1277 | READ | —（错误面typed+i18n 完善） |
| src/bench.ts | 491 | READ | 基准页（选型期产物） |
| src/empty_state.ts | 80 | READ | — |
| src/wasm_boot.ts | 44 | READ | 单飞门设计正确（并发崩溃根治） |
| src/i18n/index.ts | 83 | READ | — |
| src/i18n/en.ts | 317 | READ | 268 键 |
| src/i18n/zh-CN.ts | 309 | READ | 268 键，与 en 键集**完全一致**（零缺漏） |
| src/mang/docx_view.ts | 2134 | READ | **正面**：WebView 层自带 ZIP 护栏（EOCD/条目数/64MB/32MB） |
| src/mang/pdf_view.ts | 1381 | READ | —（dispose 纪律完善） |
| src/mang/xlsx_view.ts | 2541 | READ | —（in-flight 异步丢弃 disposed 守卫） |
| mang/app.js | 3224 | READ | **D-01**（href 注入）/D-04/D-09/D-10 |
| mang/host.js | 108 | READ | —（可选 iframe 宿主桥） |
| mang/index.html | 84 | READ | CSP 含 `'unsafe-inline'`（D-01 放大面） |
| mang/styles.css | 2622 | READ | —（真实字节经 Read 复核无损坏） |
| mang/qa-probe.css | 14 | READ | QA 探针样式 |
| index.html | 637 | READ | legacy 宿主页 |
| index.host.html | 37 | READ | — |
| bench.html | 58 | READ | — |
| build.mjs / mang-build.mjs | 83/88 | READ | 构建 3 script：build/build:legacy/serve/typecheck——**无测试框架**（D-06） |
| scripts/serve.mjs | 55 | READ | 遍历防护 resolve+startsWith，OK |
| package.json / tsconfig.json / package-lock.json | 20/13/884 | READ | pdfjs-dist ^6.4.299 运行时依赖 |
| .gitignore | 3 | READ | node_modules/dist |
| tests/results/results_concurrent_web.json | 112 | READ | 历史结果（内含 `E:\Code\ai\md\audit\...` 机器本地路径——D-36 同类） |
| mang-shots/*.png（154 件） | — | 登记 | UI 截图证据（不深读） |

### A2. bidcore/crates/mang-app（18 文件）

| 文件 | 行数 | 状态 | 发现 |
|---|---|---|---|
| src/main.rs | 1013 | READ | 窗口/事件循环/serve()/run_llm_test（直链面见 D-13） |
| src/ipc.rs | 606 | READ | 命令分发/大响应落盘/注入脚本转义（js_json 三防） |
| src/fsapi.rs | 578 | READ | writeBytes 限 .mang/（好）；mkdir 不限（D-14）；preview_write 无清理（D-15） |
| src/paths.rs | 301 | READ | 工作区围栏（组件比较+canonicalize，测试覆盖穿越/兄弟前缀） |
| src/diag.rs | 392 | READ | **D-12**（tools==13 陈旧断言）/D-13（mcp.status 语义） |
| src/recent.rs | 381 | READ | Mutex+tmp/rename；last-workspace 记忆 |
| src/chrome.rs | 883 | SKIM+关键段 READ | WM_NCHITTEST 子类/按钮宽度上报/DPI 注释详实 |
| src/selftest.rs | 414 | READ | --selftest 23+ 断言覆盖 IPC 全命令面 |
| src/gitlog.rs | 157 | READ | **D-20**（git 子进程无超时且在 UI 线程） |
| src/dialogs.rs | 159 | READ | MANG_DIALOG_MOCK 无头门 |
| src/dnd.rs | 167 | READ | WM_DROPFILES 子类；DragFinish 无泄漏 |
| src/snapshots.rs | 105 | READ | mang-snap 薄适配（与 bid-mcp save_version 同一真源） |
| src/annotations.rs | 204 | READ | 边车三态认校验；原子写 |
| src/shell.rs | 74 | READ | explorer /select |
| src/b64.rs | 127 | READ | 手写 base64+测试 |
| build.rs / Cargo.toml / .gitignore | 26/40/2 | READ | **独立 workspace**（CI 缺口根源之一） |
| （tests/ 目录不存在） | — | 核验 | 39 个测试全部为 src 内联 `#[cfg(test)]`（7+6+6+4+3+3+3+2+2+1+1+1=39） |

### A3. m1（16 文件）

| 文件 | 行数 | 状态 | 发现 |
|---|---|---|---|
| README.md / Cargo.toml | 74/17 | READ | 自述"研究验证被测系统"；独立 package |
| src/main.rs / run.rs | 62/319 | READ | TC-01/02/07/11 编排；输出零时间戳 |
| src/state.rs | 144 | READ | 4 态状态机+E_STATE 零副作用 |
| src/import.rs | 203 | READ | pdftotext/pdfimages/tesseract 外部链；sha256_file 用 expect（D-23） |
| src/docx.rs | 210 | READ | **D-22**（zip+quick-xml 无安全门） |
| src/xlsx.rs | 152 | READ | 同上 |
| src/extract.rs | 304 | READ | 规则①~⑨（零 GT 窥视） |
| src/search.rs | 164 | READ | 确定性排序+数值归一化 |
| src/canon.rs / chunk.rs / contract.rs | 89/55/93 | READ | OCR 规整/内容寻址切块/冻结契约 |
| Cargo.lock / tessdata | 747/24212 | 登记 | 钉版纪律 |
| 引用面 | — | 核验 | run_tests.py T10~T13 仍以 m1 为被测系统（D-21） |

### A4. scripts（84 文件）

**重点全读（12）**：run_tests.py(845)、grade.py(751)、fixture_spec.py(722)、v1_corpus_r.py(814)、engine_diff.py(754)、v1_export.py(467)、fixture_gen.py(486)、synth_tender_spec.py(752)、real_eval_score.py(190)、real_eval_gt_enrich.py(165)、v2_impact.py(328)、v3_integration.py(253)。

**结构通读（头+关键段，37）**：v1_pipeline/v1_citation/v1_darkbid、ooxml_util、synth_tender_gen、m17_inventory、m17_real_chain、telecom_{fetch,chain,file_level,gt_draft,prepare_files,refetch}、b5_engine_gt_eval、fp_sampling、baseline_run、patch_docs_r10、xref_check、title_impact_{score,summary}、gh_{audit,deep,search}、fp_attribution/{fp_attribution,tp_collateral_scan}、veto_gt/{s5_final,s5_merge,s5_residual,s6_micro,supplement_merge,supplement_scan}、e2e_bid/{run_e2e,run_e2e_artifacts,run_real_llm_path,run_subagent_path}.sh、e2e_bid/human_llm_relay.py、mang-dev/{mang_dev.ps1,mang_ipc_probe.js,mang_ipc_smoke.mjs,settings_layout_probe.py,settings_ui_probe.py}、workspace-cleanup/{cleanup-audit.ps1,test-cleanup-195.sh}、package-mang-app.ps1、syntax-check.sh、bid_health_check/{main.rs,probe_a02.rs}、engine_probe/{main.rs,bin/dump.rs,bin/p1.rs}、veto_measure/src/bin/measure.rs、veto_gt/src/bin/{baseline,dump}.rs、title_impact/src/main.rs、baseline_harness/src/main.rs、fixtures-requirements.txt。

**登记（锁文件/清单，35）**：Cargo.lock ×6、fp-diag-baseline.json(0 字节空文件)、CLEANUP.md、各 README/.gitignore。逐文件十问结论见 C 节矩阵。

### A5. 根级与 CI（7 文件全读）

.github/workflows/windows-check.yml(140)、.cargo/config.toml、.gitattributes、.gitignore、README.md、CONTRIBUTING.md、DEV-PLAN-M0-M5.md。

---

## B. 发现清单（D-01 ~ D-40）

> 级别定义同共享上下文。CONFIRMED=静态可证；SUSPECTED=需运行验证。

### 前端（web-ui / mang 工作台）

**D-01（P2，CONFIRMED 注入面 / SUSPECTED 利用链）md 预览链接 href 属性注入**
- 位置：web-ui/mang/app.js:2908（mdToHtml inline()）
- 代码：`x = x.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');`
- `$2`（URL）未经 esc() 直接插入 href 属性。正则捕获 `[^)\s]+` 允许 `"` 与任意无空白字符 → 属性逃逸可行（如 `"onpointerover=alert`+模板字面量` `）。标签文字 `$1` 已先经 esc()，仅 href 面裸奔。
- 放大面：mang/index.html:6 CSP `script-src 'self' 'unsafe-inline' blob:` —— `unsafe-inline` 使注入的内联事件处理器获 CSP 放行。
- 触发条件：用户在工作区打开含恶意链接语法的 .md 文件并切到预览态（渲染即触发 pointerover 类事件，无需点击；点击路径有 preventDefault 拦 javascript: URL，但属性注入不走该路径）。
- 影响：WebView 内 JS 执行 → 可调用 window.ipc.postMessage 全 IPC 面（fs.read/fs.readBytes/fs.tree/git.*，读面在工作区围栏内；llm.save 可写 key）。定级 P2（本地单机+围栏缓冲，但属"越权"类）。

**D-02（P2，CONFIRMED）legacy web-ui Ctrl+S 恒报"保存成功"而 web 模式零持久化**
- 位置：web-ui/src/main.ts doSave()（setupStatusBar 内）
- `window.__lastSave = { ok: true, revision, bytes }` 无条件置真；web 独立态（无宿主）下"保存"= 派发 mdc-save 事件 + 状态栏改"已保存 rev N"，**没有任何落盘**。草稿（draft.ts）是崩溃安全网且刻意不改 dirty 位（注释诚实），但状态栏/`__lastSave` 对外语义是"已保存"。宿主形态由壳消费 mdc-save（CONTRACT 路径）——但 UI 未区分两种形态的反馈文案。导出面（docx/xlsx/pdf）均有 typed 失败路径，唯"保存"面无失败语义。

**D-03（P3，CONFIRMED）XLSX 导入的 Grid 联动失败对用户不可见**
- 位置：web-ui/src/main.ts setupXlsxImport（`gridOpenBytesWhenReady` fire-and-forget）
- 主导入成功即显示成功；grid 联动失败仅回填 `__lastXlsxImport.grid`（探针面），状态行无失败后缀（成功才追加 sheet 数）。代码注释自认"增强面非前置条件"——如实登记为部分成功显示语义不完整。

**D-04（P3，CONFIRMED）i18n 三层不齐**
- zh-CN/en 词典键集 268=268 完全一致（好）；但：① mang 工作台（产品面 app.js）**整体无 i18n 机制**（全中文硬编码）；② legacy src/ 内散落硬编码中文（main.ts 设置面板标签/hint/状态、TOC 空态、refreshHint 三态）与硬编码英文（evidence_panel.ts 'calculation registered:'、renderCounts "N items · pending…"、卡片按钮 approve/reject/reverify；canvas_view 全部状态行；pdf_view save/BRIDGE_ABSENT/applySecurityStatus）。

**D-05（P2，CONFIRMED 注释-实现不一致）证据卡 approve 门与注释宣称不符**
- 位置：web-ui/src/evidence_panel.ts buildCard()
- 注释：「reverify is the recovery channel for non-valid linkage (stage-2 gate: broken linkage cannot be approved directly)」；实现：`mkAction('approve', …, !pendingReview)` —— 只看 review==pending，**不检查 linkage 状态**。stale/broken linkage 且 pending 的证据可从 UI 点 approve；是否被 wasm workbench 的 review 门拦下取决于 wasm 面（本仓 pkg 未含，无法静态证实）。UI 层注释宣称的门不存在。

**D-06（P1，基础设施 CONFIRMED）浏览器 E2E 基础设施缺口**
- web-ui 无测试框架（package.json 仅 build/build:legacy/serve/typecheck）；main.ts/pdf_view 等注释引用的 W1-W13/web_smoke.py 冒烟**不在仓库任何位置**（全仓 grep 无 web_smoke）；mang 工作台的验证手段是手工截图（mang-shots 154 张 + artifacts/mang-shots 20 张）与 Playwright 探针（scripts/mang-dev/settings_ui_probe.py——但不在任何 CI）。R15/R16/R17/R19 的性能与等价性护栏（W7/W12/W13）当前**无门禁承载**，纯靠注释声称。

**D-07（P3）collab_channel sync_resp 差量应用失败仅入探针 errors**——无重试、无 UI 面，恢复依赖手动 reconnect（重连会重发 sync_req）。harness 定位可接受，如实登记。

**D-08（P3）设置面板缺 appearance.language 键**——SETTING_VALIDATORS/#43 已加语言键与 localStorage 镜像，但 SETTINGS_FIELDS（设置面板渲染表）无该行：语言切换无 UI 入口（仅宿主注入或手改 localStorage）。

**D-09（P3，已声明的阶段缺口）mang 工作台保存/另存为/查找/撤销重做均为占位 toast**（app.js saveActive/saveActiveAs/findInDocument："将在文档编辑阶段接入"）。文件菜单仍列 Ctrl+S/Ctrl+F 项——用户可触发但无功能。审核语义面（annotations 三态、快照回滚）则已实装。

**D-10（P3）switchWorkspace 未处理拒绝**——app.js 空态最近列表点击工作区条目直接 `ui.switchWorkspace(path)`，`ipc("workspace.open")` 失败（坏路径）成为未处理 promise rejection，无 toast（对比 openFolderDialog 有 catch）。

**正面登记 P-1**：mang/src/mang/docx_view.ts 在 WebView 内实现完整 ZIP 护栏（EOCD 在位、条目数上限、中央目录/本地头边界与签名校验、解压总量 64MB、单条目 32MB、声明-实际守恒）——把 bid-parse guard 纪律复制到了渲染层，错误码中文化 typed。serve.mjs 与 shell serve() 均有目录穿越防护。

### 桌面壳（mang-app）

**D-11（P1，CONFIRMED）mang-app 39 个测试无 CI 执行且已腐烂**
- bidcore/crates/mang-app/Cargo.toml:3 `[workspace]`（空表）→ 独立 workspace；windows-check.yml 只对它跑 `cargo check --all-targets`（编译不执行）。
- src/diag.rs:303-304 `assert_eq!(v["tools"], 13, "工具面 13 个（#68 阶段 2 后）"); assert_eq!(v["implemented"], 11);` —— bid-mcp `TOOL_NAMES: [&str; 25]`（lib.rs:53）→ 该测试在 HEAD 必红。**静态可证**：mang-app `cargo test` 当前是红的，从未被任何门禁发现。
- 影响：mang-app 的 39 测试（含 fs 围栏 write_bytes_guard、paths 穿越、llm 掩码等**安全测试**）形同虚设；后续任何人修好 13→25 也无门禁防再腐。

**D-12（P2→设计澄清，CONFIRMED）「直链 bidcore」的实际边界 + mcp.status「已就绪」语义误导**
- 全 IPC 面（ipc.rs:340-448）枚举：fs/tree/read/readBytes/writeBytes/stat/mkdir、explorer.reveal、recent、history.list、git.{log,show,status}、preview.write、snapshots.{check,save,read,rollback}、annotations.{load,setStatus}、mcp.status、llm.{status,save,test}、fspec.status、file.fromPath、workspace.{current,open}、window.*。
- **直接调用 bid-tools/bid-mcp 的点**：`bid_tools::llm::{LlmConfig::resolve, HttpTransport.post_chat}`（main.rs run_llm_test、diag.rs llm_status/llm_save）；`bid_tools::format_spec::FormatSpec`（fspec_status 只读解析）；`bid_mcp::{tools_schema, dispatch_json}`（仅 E_NOT_FOUND 自检探测）；`mang_snap`（与 bid-mcp save_version 共享单一真源）。
- **结论**：桌面壳**不调用** BidService 的任何业务方法（create/import/extract/validate/export 均不触）——P0-1（validate 卡死）与 P0-2（证据门）**不因桌面壳扩大**；桌面壳不持有 db.json 业务状态（db.json 只作为 workspace 内普通文件被 fs.read 读取）。风险集中在 LLM 配置/HTTP 与 git 子进程（见 D-20）。
- 但 mcp.status 的 `ready: probe_ok && tools>0`（diag.rs:99）语义是「本进程内嵌引擎能应答未知工具」——UI 显示「MCP 服务：已就绪（stdio）」让用户以为 AI 侧 stdio 通道可用；实际 stdio 服务由 MCP 客户端拉起，本壳看不见其生死（注释自认）。显示语义与真实通道状态不符（P3 已含在 D-12）。

**D-13（P3）fs.mkdir 写面纪律不一致**——fs.writeBytes 限定 .mang/（fsapi.rs:253 IPC-CONTRACT §5.2），fs.mkdir 却允许工作区任意路径建目录（resolve(raw,true) 后无 .mang 检查）。WebView 内任意 JS（含 D-01 注入面）可在工作区制造目录污染（git status 噪声），不能写文件内容。

**D-14（P3）preview.write 临时文件无生命周期管理**——写 %TEMP%\mang-preview\<uuid>-<name>（≤48MB/件），退出只清 mang-ipc（main.rs purge_pending）→ mang-preview 无限累积。

**D-15（P3）serve() 响应头 Access-Control-Allow-Origin: \***（main.rs:864）——custom protocol 单页面场景实际跨源面小；/__ipc/<id>.json 一次性读删 + 文件名纯化（无 `../`）防护到位。登记备查。

**D-16（P2，SUSPECTED）git 命令在 UI 线程同步无超时**
- gitlog.rs run_git：`Command::new("git")...output()` 无 timeout；调用链 exec() → handle_ipc（注释自证「跑在 UI 主线程（WebView2 消息循环里）」）。
- 触发：工作区是大仓库/坏 .git（git log 挂起、慢盘）→ WebView 消息循环冻结（整窗无响应，与 llm.test 被移到后台线程的教训同源——那条注释明确说"阻塞它等于冻住整个窗口"，git 却没享受同等待遇）。
- 对比：llm.test 已移后台线程；对话框移主线程队列——git 是唯一遗留的 UI 线程同步外呼。

**D-17（P3）llm key 经 WebView IPC 明文传输**（app.js saveLlmConfig → llm.save）——设计使然（本机单页面），落盘 .mang/llm.json 有 gitignore 防入库、状态面只出掩码（diag.rs mask_key，测试断言明文不外泄）。正面+备查。

### m1（遗留研究线）

**D-18（P2，CONFIRMED 现状）m1 仍在主测试链充当被测系统，与 bidcore 双实现并存**
- run_tests.py T10~T13 以 `m1/target/release/m1` 为被测物（构建/单测/四场景/覆盖/确定性）；bid-grade/tests/e2e_m1.rs 走 bid-tools runner（非 m1 crate）。capability-map.md 自认三套解析实现并存（bid-parse 产品层 / m1 研究层 / ooxml_util.py 仿真）。m1 无 CI（m1/ 无任何 workflow 引用）。生产路径零引用（grep 全仓：仅 run_tests/scripts/docs/research 引用 m1）——**无生产路径调用 m1**，风险限于评测语义漂移（m1 的 extract 规则与 bid-parse extract.rs 已分叉）。

**D-19（P3，CONFIRMED）m1 zip/quick-xml 解析与 bid-parse/guard 安全门差距**
- m1/src/docx.rs:17-21：整包读入内存（`read_to_end`）、`zf.by_name("word/document.xml")` 直读、无 EOCD 守恒校验、无条目数上限、无解压总量上限、quick-xml 无深度/大小限制。
- 对照：bid-parse guard.rs 五门（EOCD 守恒/镜像 zip64/路径穿越/重复条目/zip 炸弹——baseline/security 六敌意件基线）+ 引擎 office-io 门；mang 桌面渲染层（docx_view.ts）也自带护栏。m1 是三套实现中唯一无门的。研究线仅靠"语料全部自产/官方公开件"约束——违反最小权限纪律，若喂入敌意件会 OOM/慢速解析。连带：import.rs sha256_file 用 `expect()` panic（D-19a）。

### scripts（评测与工具链）

**D-20（P2，CONFIRMED）P2 真实文件评测不可从本仓重现——绝对路径指向仓外**
- real_eval_score.py:28 `SAMPLES = Path("/home/z/my-project/repos/sunShineLoveMe__ai_bidding_power_grid/test_samples/water_tender_docs")`——本环境实测不存在（`ls` 失败）。评测输入（04_水利施工标段.pdf）实体在 real-corpus/water/files（同名件在库），但脚本不指向它。PDFTOTEXT 回查与重跑均断。
- 同类：gh_audit.py/gh_deep.py/gh_search.py `OUT = "/home/z/my-project/download/bid-agent-research/research/raw"`（另一工作区）；fp_attribution/fp_attribution.py:9 `open('/home/z/my-project/scripts/fp_diag.json')`（**文件不在仓**，且 scripts/fp_attribution/fp-diag-baseline.json 是 0 字节空文件）；fp_attribution/tp_collateral_scan.py 读 `/home/z/my-project/wt-fp/artifacts/veto-gt/ground-truth.json`（另一 worktree）；veto_gt/s5_*.py+supplement_*.py 依赖 `/home/z/my-project/tmp-conv/…` 与 `/tmp/vgt-dump/`；bid_health_check/src/bin/probe_a02.rs:8 硬编码 `/home/z/my-project/bid-research/…`；patch_docs_r10.py BASE=/home/z/my-project/download/…。**九处机器本地绝对路径，六处当前必然 FileNotFoundError。**

**D-21（P2，CONFIRMED）real-eval GT 的定位核验对象是自家解析产物（部分自证）**
- real_eval_gt_enrich.py:33 `PARSE = artifacts/real-eval/parse/A-…pdf.json`；anchor_verification 三级匹配全部对**解析产物 blocks**（非原始 PDF）。
- VERBATIM_FIXES（:49-54）把 GT-R01/GT-T01 的 excerpt **改写为解析文本层中存在的字符串**（"改为逐字无损段"）——真值流向是「解析器输出 → GT」而非「原文 → GT」；run_tests T15a 还把 `verbatim_fixed == ["GT-R01","GT-T01"]` 锁进断言固化该事实。
- real_eval_score.py 的召回对齐（norm_n2 互含+滑窗）是独立 Python 实现（与生产 extract 无代码共享）——对齐器独立性成立；但 GT 锚定面的同源依赖使其成"半自证"评测。GT 全程 draft_pending_human 未冻结（诚实标注到位）。

**D-22（P2，CONFIRMED 设计性）V1/V2/V3 影响面与定位器为 SEMUL 仿真**
- ooxml_util.py 头注明示「引擎本体在本验证环境不可执行——本模块是其语义级仿真（SEMUL ≠ ENGINE_RUN）」；v2_impact.py「bid-domain §2 语义的 Python 复刻 = SEMUL；Rust 层交叉验证仍待 cargo」；v3_integration.py 同口径。run_tests T8/T9 的全绿因此是**设计语义绿**，不是产品行为绿。T19（bidcore cargo test）提供部分产品面补偿，但 bid-domain 的 impact/export-gate 与 v2_impact.py 之间**没有逐断言交叉验证测试**（未找到将两者输出对比的用例）。

**D-23（P2，CONFIRMED）T17/T14/T16 断言入库历史产物而非当前行为**
- run_tests.py t_engine_diff()：T17b 断言 `artifacts/engine-diff/summary.json`（f8c860a 修复前）特定计数 54/9/2/4；T17c 断言 `engine-diff-after`（f7ddd39）61/0/4/3；T17d 断言 `engine-probe/report-after-fixes-f7ddd39.json` 字段值——**评测不可从当前 HEAD 重现**（不重跑 dump/diff，只读旧 JSON）；断言文案把过期 SHA 当锚。T14a（576=224+174+178）、T16c（17 文件/≥2000 条款/14 锚点）同样只读 stats.json/file-level-report.json 在案数字。
- 该设计叫「证据在案回归」——能防**证据文件被误删/篡改**，不能防**当前代码行为漂移**。与十问第 1/6/7 问直接冲突（评测能否从当前 HEAD 重现：否）。

**D-24（P3）run_tests SKIP 纪律总体诚实，但 cargo 侧 SKIP 计入 passed**
- run_tests.py：`require_tools_or_skip`/`_ENV_MISSING`/汇总单列 skipped + 追加 baseline/ENV_BLOCKED.md——机制正确（异常不静默、不伪装失败）。
- cargo 侧（bid-parse env_preflight）：`skip_if_env_missing` 打横幅后 `return` → `test result: ok`。windows-latest 无 poppler/tesseract → e2e_m1/real_corpus_e2e/corpus_probe 在 CI **恒 SKIP 恒绿**——CI 绿不等于真实链验证过（见 E-1）。real_corpus_e2e.rs:875 还有「解析器与 GT 不同源」的环境性 SKIP（诚实登记）。

**D-25（P3）workspace-cleanup/cleanup-audit.ps1 目标路径 `E:\Code\ai\md\audit`**（另一台机器工作区），带 dry-run/trash 四层防护设计良好但与当前仓无关（历史脚本）。web-ui/tests/results/results_concurrent_web.json 同类（`E:\Code\…` dist 路径入 JSON）。

**正面登记 P-2**：六个 Rust harness（baseline_harness/bid_health_check/engine_probe/title_impact/veto_gt/veto_measure）纪律一致——「不改 crates 任何源码、path 依赖只读、确定性 JSON 输出」头注齐全；baseline_run.py 双跑一致性 digest 设计正确；telecom_fetch.py 逐文件 md5/sha256 双记录 + MISMATCH 双记录（T16a 断言 14 OK + 2 MISMATCH，诚实含坏件）。

**正面登记 P-3**：fixture_spec.py 单一真源 → fixture_gen 生成 + grade.py 判分同源消费；T4h 反标记回声扫描（GT 标记/规则码不入语料正文）；T3a manifest 双向校验（游离文件检出）。合成夹具链路的 GT-语料隔离纪律是同类项目少见的水准。

### CI 与根级

**D-26（P2，CONFIRMED）「Verify SKIP banner」步骤恒绿且可被假绿骗过**
- windows-check.yml:117-127：`$out = cargo test --workspace 2>&1 | Out-String` —— pwsh 管道使步骤退出码取自 Out-String（恒 0），**该步 cargo test 失败也不红**；banner 匹配两分支（有/无横幅）都 echo 后正常结束——无法区分「SKIP 机制坏掉静默全跑全绿」与「工具在位真跑」。作为「机制自证」步骤自身不构成门禁。
- 附带浪费：该步无条件重跑一次全量 cargo test（~20 分钟/轮，且非 --no-fail-fast）。

**D-27（P1，CONFIRMED）run_tests.py 主链不进 CI**——全仓唯一 workflow 不调用 scripts/run_tests.py；T0~T19（含 m1 线、M1.7 真实链、SEMUL 验证、GT 结构门禁）全靠本地手跑。CI 绿与"每轮测试"绿是两个不相干的信号。（与共享上下文已知项一致，此处给出证据锚点。）

**D-28（P3）.cargo/config.toml 仅 windows-msvc 静态链接**——合理（0xC0000135 根治）；.gitattributes 二进制钉死（*.pdf/docx/xlsx/png/zip）有 #169 实证背书。README/CONTRIBUTING/DEV-PLAN 与代码状态的偏差：README 仍自述"研究阶段产出/未实现完整业务"，而 bidcore 已有 8 crate+MCP+桌面壳（文档滞后，P3）。

### 语料与产物（盘点结论）

**D-29（P3，CONFIRMED 盘点）real-corpus 73 文件三层结构**
- water/ 5 件（3 PDF 招标文件 + 1 扫描件修订 + 1 zip 示范文本）——PROVENANCE.md 登记政府原 URL + sha256；**传入路径是第三方 GitHub 仓转载，原站未重访**（自认）；其中 B3 zip 是"容器分发负向测试对象"。
- telecom/parse-input 17 件（A/B/C 前缀命名的真实招标文件族）——txzbqy 免鉴权通道抓取，fetch-report.json 16 文件溯源（md5+sha256）；gitignore 排除 files/extracted/converted/lo-tmp（86MB+83MB 二进制不入库，可按哈希重取——telecom_refetch.py 复现协议在案）。
- gov-procurement/：3 份 cuc-028 链节点公告文本（.txt）+ sources/ 镜像 HTML + inventory/chains/stats；**576 份公告本体不入库**（inventory.json 仅元数据，实体在外部仓 tlyyxjz__BidAgent `_w3/w4/w5_raw`）——T14a 断言的是元数据账本。
- 共 11+ 份真实招标文件（water 4 + telecom 17 + 政采公告文本 3）的来源与用途全部可对上 PROVENANCE/fetch-report（抽样核验：文件名、数量、sha256 记录在位）。

**D-30（P3，CONFIRMED 盘点）baseline 139 与 artifacts 809 对账**
- baseline/：MANIFEST.json（124 语料文件+环境指纹+双跑一致性）+ parse/（逐文件解析）+ security/（6 敌意件当前行为基线 JSON + hostile/ 实体）+ ENV_BLOCKED.md——与 baseline_run.py 产物清单头注逐项对应。
- artifacts/ 809 文件 20 目录：e2e-subagent-path 587（127 章 prompts/manuscripts/drafts 三套——e2e 产物 latest-only 纪律外的例外，gitignore 注释自认"同款只留 latest"仅覆盖 e2e/ 与 e2e-subagent-path/2* 时间戳目录）；e2e 38；engine-diff* 3 组 45；real-eval 10（GT 草案+parse+score-report）；veto-gt 8 / veto-measure 10（GT 1752 true 标注账本）；bid-health-check 22；mang-shots 20；title-impact 2（experiment.json 被 gitignore，summary+gt-recall 入库）。数字与 T17/T15/T16 断言字段抽查一致。

---

## C. scripts 十问矩阵（§3.11）

图例：✓=是/合规 ✗=否/违规 △=部分/豁免有据 N/A。Q1 调当前生产实现？Q2 复制生产算法的仿真？Q3 异常变 SKIP？Q4 SKIP 记通过？Q5 事后清理失败证据？Q6 覆盖仓库真实文件（危险写路径）？Q7 绝对路径/机器本地依赖？Q8 可从当前 HEAD 重现？Q9 硬编码过期 SHA/历史数字？Q10 报告混历史数字？

| 脚本 | Q1 | Q2 | Q3 | Q4 | Q5 | Q6 | Q7 | Q8 | Q9 | Q10 |
|---|---|---|---|---|---|---|---|---|---|---|
| run_tests.py | ✓(m1/bidcore 双线) | △(调 v2/v3 SEMUL) | ✓(显式登记) | △(cargo侧SKIP计ok) | ✓ | ✓只读+临时目录 | ✓ | ✗(T17/T14/T16读旧产物) | ✗(f8c860a/f7ddd39锚) | ✗ |
| grade.py | ✓(独立对齐器) | ✓(对齐器独立于m1) | ✓ | N/A | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| fixture_spec.py / fixture_gen.py | N/A(生成器) | N/A | ✓ | N/A | ✓ | ✓fixtures/ | ✓ | ✓(T6钉版) | ✓ | ✓ |
| synth_tender_spec/gen | N/A | N/A | ✓ | N/A | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| real_eval_score.py | ✓(读parse产物) | △(norm_n2复刻) | ✓ | N/A | ✓ | ✓ | **✗(仓外绝对路径)** | **✗** | ✓ | ✓ |
| real_eval_gt_enrich.py | △ | N/A | ✓ | N/A | ✓(留excerpt_original) | ✓ | ✓ | △(依赖parse产物) | ✓ | ✓ |
| fp_sampling.py | ✓ | ✓ | ✓ | N/A | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| v1_pipeline/citation/export/darkbid | △(ooxml仿真) | **✗(SEMUL自认)** | ✓ | N/A | ✓ | ✓ | ✓(Sarasa字体钉版) | ✓(LO在位时) | ✓ | ✓ |
| v2_impact.py / v3_integration.py | ✗(不复刻调Rust) | **✗(SEMUL自认)** | ✓ | N/A | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| ooxml_util.py | ✗ | ✗(fnv64逐位复刻+公开向量) | ✓ | N/A | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| m17_inventory / m17_real_chain | ✓(真实公告解析) | △(影响面数据驱动实现) | ✓ | N/A | ✓ | ✓real-corpus/ | ✓ | △(公告文本在库) | ✓ | △(GT草案探索性) |
| telecom_fetch/refetch | ✓ | N/A | ✓(MD5_MISMATCH双记录) | N/A | ✓ | ✓ | ✓(URL+fileId) | ✓(重取协议) | ✓ | ✓ |
| telecom_chain / file_level / gt_draft / prepare_files | ✓(Rust亲跑) | N/A | ✓ | N/A | ✓ | ✓ | ✓(libreoffice) | △(二进制gitignore) | ✓ | ✓ |
| b5_engine_gt_eval.py | ✓(引擎+旧路径双跑) | N/A | ✓ | N/A | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| engine_diff.py | ✓(双跑差分本体) | N/A | ✓ | N/A | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| xref_check.py | N/A(文档检查) | N/A | ✓ | N/A | ✓ | ✓docs/ | ✓ | ✓ | ✓ | ✓ |
| baseline_run.py + baseline_harness | ✓(bid_parse真跑) | N/A | ✓ | N/A | ✓(失败件也落盘) | ✓baseline/ | ✓ | ✓(双跑digest) | ✓ | ✓ |
| engine_probe/*(3 bin) | ✓(只读审计mdcore) | N/A | ✓ | N/A | ✓ | ✓ | △(MDC_PDFIUM_LIB $HOME) | △(引擎仓f8c860a钉版——审计快照) | △(rev入档合理) | ✓ |
| bid_health_check(+probe_a02) | ✓ | N/A | ✓ | N/A | ✓ | ✓ | **✗(probe_a02硬编码bid-research路径)** | ✗(probe_a02) | ✓ | △(REPORT含历史指标,自认) |
| title_impact/*(+2 py) | ✓(extract真跑) | △(标题判定镜像) | ✓ | N/A | ✓ | ✓ | ✓ | △(experiment.json被gitignore) | ✓ | ✓ |
| veto_gt/*(7 py+2 rs) | ✓(GT构建) | N/A | ✓ | N/A | ✓(原条目不动只append) | ✓ | **✗(tmp-conv//tmp/vgt-dump/)** | ✗ | ✓ | ✓ |
| veto_measure/measure.rs | ✓(生产路径解析) | N/A | ✓ | N/A | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| gh_audit/gh_deep/gh_search | N/A(取证工具) | N/A | ✓ | N/A | ✓ | ✓research/raw(拷贝入仓) | **✗(/home/z/download/…)** | ✗ | ✓ | ✓ |
| fp_attribution/*(2 py) | ✓ | N/A | ✓ | N/A | ✓ | ✓ | **✗(fp_diag.json不在仓/wt-fp路径)** | ✗ | ✓ | ✓ |
| patch_docs_r10.py | N/A(一次性) | N/A | ✓ | N/A | △(改文档) | ✓docs/ | ✗(/home/z/download) | ✗ | ✓ | ✓ |
| workspace-cleanup/*(2) | N/A | N/A | ✓ | N/A | ✓(dry-run+trash) | ✓(另一机器路径) | ✗(E:\Code\…) | ✗ | ✓ | ✓ |
| e2e_bid/*.sh(4)+relay | ✓(cargo test载体) | N/A | ✓(横幅SKIP) | △(cargo语义) | ✓ | ✓artifacts/ | ✓ | ✓(synth-tender在库) | ✓ | ✓ |
| mang-dev/*(5) | ✓(真实窗口CDP) | N/A | ✓ | N/A | ✓ | ✓ | △(需WebView2/pwsh环境) | △(Windows only) | ✓ | ✓ |
| package-mang-app.ps1 / syntax-check.sh | N/A | N/A | ✓ | N/A | ✓ | ✓dist-pkg/ | ✓ | ✓ | ✓ | ✓ |

**十问总结**：Q1/Q2 的核心风险不在"有没有仿真"（SEMUL 全部头注自认、口径诚实），而在 **SEMUL 与 Rust 实现间无交叉验证测试**；Q7/Q8 的重灾区是真实评测/GT 构建/取证三族脚本（9 处绝对路径、6 处必炸）；Q9/Q10 的唯一违例是 run_tests T17 族（历史产物断言）。

---

## D. 语料/产物分类表

| 目录 | 文件数 | 分类 | 生成/消费脚本 | 抽样验证 |
|---|---|---|---|---|
| fixtures/ | 69 | 合成评测夹具（corpus-A 25 埋点 44 文件 + corpus-R 结构层 5 + TR2 引擎借鉴 5 + synth-tender 13 + queries 2） | fixture_gen.py / v1_corpus_r.py / synth_tender_gen.py 生成；m1/bidcore 测试消费 | T3a manifest 双向+sha256 抽验通过 |
| real-corpus/ | 73 | 真实语料三层（water 5 / telecom parse-input 17+账本 / gov 3 节点+576 元数据账本） | telecom_fetch.py 等抓取；m17_*/telecom_* 消费 | PROVENANCE.md URL+sha256 对账（见 D-29） |
| baseline/ | 139 | B1 冻结基线（124 语料解析 + 6 敌意件行为基线 + env 指纹） | baseline_run.py+baseline_harness | MANIFEST counts/files 与目录对账 |
| artifacts/ | 809 | 20 类运行产物（e2e 两代 625 / engine-diff 三代 45 / real-eval 10 / veto GT 18 / 探针报告等） | 各对应脚本 | T17 断言字段抽查一致（见 D-30） |
| research/raw/ | 56 | 18 仓竞品取证原始证据（README/audit.json/SKILL 抄件） | gh_audit/gh_deep.py（输出至另一路径后拷入） | 与 README 交付物索引对应 |
| web-ui/mang-shots/ | 154 | 桌面 UI 手工截图证据 | mang-dev 探针 | 登记不深读 |

---

## E. CI 门禁缺口清单（windows-check.yml 单工作流细目）

1. **E-1（P1）cargo 测试在 Windows runner 上系统性空转**：pdftotext/pdftoppm/tesseract/soffice 均不在 windows-latest → e2e_m1、real_corpus_e2e、corpus_probe/kb_probe、e2e_pipeline 真件链、pdf_export（软门 soffice）全部 SKIP 计 ok——**CI 从未真正执行过任何真实文件全链测试**（Detect external toolchain 步骤把这一事实写进 summary，但门禁不因此黄）。
2. **E-2（P1）mang-app 无 cargo test**：独立 workspace + 仅 check（D-11 的直接后果；39 测试含安全围栏测试）。
3. **E-3（P1）run_tests.py 不进任何 CI**（D-27）：m1 线/M1.7 真实链/T15-T18 全部门禁仅存在于本地惯例。
4. **E-4（P2）无 Linux job**：guard.rs `#[cfg(not(unix))]` E0596 的教训只修了 Windows 侧——`#[cfg(unix)]` 分支与 Linux-only 依赖（rustls/native-tls 差异、路径语义）无 CI 覆盖。
5. **E-5（P2）无 clippy/fmt/rustfmt 门禁**；syntax-check.sh（本地秒级语法门）未入 CI。
6. **E-6（P2）web-ui 零 CI**：typecheck（tsc --noEmit）、build（esbuild）、renderer 打包、CSP/HTML 结构无任何自动化；唯一验证是入库截图。
7. **E-7（P2）「Verify SKIP banner」步骤自身可假绿**（D-26）：pwsh 管道吞退出码 + 双分支恒过 + 重复全量跑 ~20min。
8. **E-8（P3）scripts/m1/SDK/Skill 无 CI**（共享上下文已知项，确认）：Python 语法（py_compile）都无门禁——本审计发现的 6 处"必炸路径"脚本若有一道 py_compile+干跑冒烟即可早发现。
9. **E-9（P3）缓存 key 只含 bidcore/Cargo.lock**：mang-app 独立 lock 变化不刷新缓存（小成本问题）。

---

## F. 测试基础设施可靠性结论

1. **双轨门禁实质单轨**：产品真实门禁 = 本地手跑 `run_tests.py`（T0-T19）+ 本地 `cargo test`；GitHub CI 提供的只是「Windows 编译 + 单元层绿」（其中真件链全部 SKIP——E-1）。任何"CI 绿"的 PR 都可能携带：m1 语义漂移、SEMUL 与 Rust 实现分叉、mang-app 测试腐烂（D-11 已实际发生）、web-ui 全量回归（无任何防线）。**测试基础设施的可靠性瓶颈不在 SKIP 纪律（该纪律是这套仓库最扎实的部分——run_tests 的 ENV_MISSING 三态账本 + cargo 横幅 + ENV_BLOCKED.md 落盘均属上乘），而在"哪些测试从未被谁跑过"没有账本。**
2. **评测可重现性分层清晰但断层明确**：合成线（fixture→m1/bidcore→grade）可从 HEAD 完整重现且 T6/T13 双跑字节一致；引擎差分线（T17）与真实链线（T14/T16 部分）依赖**入库历史产物 + 仓外路径**——一旦换机器/换 HEAD 即不可重跑（D-20/D-23）。GT 侧：corpus-A GT 严格（反标记回声+对齐器独立）；real-eval GT 是半自证草案（D-21）；veto GT 构建管线不可复现但成品入库可测（veto_measure 可重跑）。
3. **mang 桌面线的验证纵深**：Rust 侧 selftest（23+ 断言）+ mang-dev CDP 冒烟 + Playwright 设置面板探针 + 154 截图——手段齐全但**全部不入 CI**（E-2/E-6）；39 个内联测试中至少 1 个在 HEAD 是红的（D-11），直接证明该层门禁失效。
4. **与两个已知 P0 的交叉**：mang-app 不调 BidService 业务方法（D-12），故 P0-1/P0-2 不经桌面壳放大；但 mcp.status「已就绪」文案把"引擎同源自检"包装成"MCP 通道就绪"，用户可能据此认为 AI 侧管线可用——建议改文案或探测真实子进程。
5. **最优先修复建议**（按杠杆排序）：① mang-app 撤 `[workspace]` 空表或入 CI cargo test（一次改动同时修 D-11 与 E-2）；② run_tests.py 加 GitHub Linux job（poppler/tesseract action 装齐即得真件链门禁）；③ real_eval_score.py 的 SAMPLES 改指 real-corpus/water/files（仓内已有同 sha 文件）；④ mdToHtml href 转义 + CSP 去 unsafe-inline（D-01）；⑤ T17 增加一个"从当前 HEAD 重跑 dump+diff"的活体档（历史档保留为证据）。

---

### 附：发现级别汇总

| 级别 | 数量 | 编号 |
|---|---|---|
| P0 | 0 | —（Agent D 面内无数据丢失/越权级；D-01 为 P2 越权候选，需运行验证升/降级） |
| P1 | 4 | D-06（浏览器 E2E 缺口）、D-11（mang-app 测试腐烂+无门禁）、D-27（run_tests 不进 CI）、E-1（CI 真件链空转——并入 D-26 语境） |
| P2 | 11 | D-01、D-02、D-05、D-12、D-16、D-18、D-20、D-21、D-22、D-23、D-26 |
| P3 | 25 | D-03、D-04、D-07~D-10、D-13~D-15、D-17、D-19、D-24、D-25、D-28~D-30 及附注项 |

（全文发现 40 项 + 正面登记 3 项；每项含位置/证据/置信度，SUSPECTED 项已标注所需运行验证。）
