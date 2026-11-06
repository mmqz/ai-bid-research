# 模块审计 D1：web-ui 前端逐文件审计（module-audit-web-ui）

- 审计代理：4-d1（ai-bid-research 全仓审计 V2 · Task ID 4-d1）
- 冻结基线：main@6105beb（分支 audit/v2-full-coverage；工作树仅新增本审计文档，生产代码零触碰）
- 范围：`git ls-files web-ui/` 全集（156 个跟踪文件）——代码文件逐文件全读（src/ 11 TS + i18n 3 + src/mang/ 3 TS + mang/ 5 + 根 7 + 构建 3）；mang-shots/ 149 张 PNG 与 package-lock.json 做覆盖性检查；styles.css/qa-probe.css 覆盖性检查
- 纪律：只读审计；未修改任何生产文件/测试/配置/数据；未 npm install/typecheck（Lead 统一执行）；本文与 inventory-d1.csv 为本轮唯一新增产出
- 方法：逐文件通读 + 与 C1 工具清单（module-audit-bid-mcp.md 25 工具表）与 D-MANG（module-audit-mang-app.md ipc.rs dispatch 表 + main.rs window/llm.test 特例）做前后端契约对账 + i18n 键集脚本对账 + lock/manifest 一致性脚本对账

---

## 0. 架构级结论（先于逐文件细节——决定全部"前后端契约"问题的判读框架）

**web-ui 与 bid-mcp（C1 的 25 工具面）之间不存在任何调用契约。**

证据（CONFIRMED）：
1. `package.json:5` 自述：`"mang 标书工作台前端：VSCode 风格文档工作台（web-ui/mang）+ 旧 mdcore web 资产（src/ 等，仅供组件参考搬运）"`——src/ 全部 TS（main/draft/evidence_panel/canvas_view/pdf_view/xlsx_grid/collab_channel/bench/empty_state/wasm_boot/i18n）是历史 mdcore 项目的 web 演示资产整体搬入。
2. grep 全 web-ui（src/ + mang/ + src/mang/）：零命中 25 工具名（create_bid_project / import_tender_files / list_evidence / register_evidence / record_review_gate / export_bid_document / get_bid_project_state …）；零命中 `E_` 错误码族解析；零 JSON-RPC/MCP 客户端代码；零对 bid-mcp stdio/server 端口的 fetch。
3. mang-app 审计（§1.1）已钉定：壳与 MCP 的关系是"两进程各写各的文件"（AI 客户端拉起 bid-mcp-stdio 写 db.json；壳读工作区文件 + 写 .mang/**）。**web-ui/mang 是这个壳的 UI，它消费的是壳 IPC（fs/git/snapshots/annotations/llm/mcp.status 诊断面），不是 MCP 工具面。**

由此推论（对应任务重点 1/2/4）：
- "用了废弃参数/解析了不存在的响应结构/E_* 错误码覆盖"——**N/A（无调用面）**。唯一沾边的"MCP 面"是设置面板诊断区 `ipc("mcp.status")`——消费壳的 mcp.status（diag.rs:69 `bid_mcp::tools_schema()` 只读探针），仅显示 `channel/tools/ready` 三字段，不解析任何工具契约。
- §3.10 的项目状态一致性（部分成功/审核失效/证据 tier/导出失败误显示）——**在 web-ui 中无实现物**。前端不渲染 bid 域项目状态/矩阵/证据 tier，故"UI 误显示"类风险不存在于当前代码；真正的风险是**缺失**（见 UI-002）。
- draft.ts 的"乐观并发/project_version 冲突 UI 行为"——**N/A**：draft.ts 是 localStorage 崩溃草稿（mdc-draft 键、1MiB 上限、四触发落稿、beforeunload 拦截），与 bid 域 project_version 完全无关。results_concurrent_web.json 里的 `doc_expect_guard_rejects_stale_write` 用例验证的是 mdcore wasm 的 `Anchor::ExpectMismatch`（历史项目），不是本仓乐观并发。

**web-ui 真实的前后端契约是 mang/app.js ↔ mang-app ipc.rs/main.rs 的 IPC 消息集**——该契约经逐调用点对账（见 §1），**完全对齐、零断裂**。

---

## 1. IPC 契约对账（mang 面 ↔ mang-app；任务重点 5）

app.js（3224 行）全部 `ipc()` 调用点（27 处）vs 壳 dispatch 表（ipc.rs:348-454 + main.rs:540/549/567 特例）：

| UI 发出 | 壳处理 | 参数/响应解析 | 结论 |
|---|---|---|---|
| app.info | ipc.rs:368 | workspace/version/protocol | ✔ |
| window.info / window.buttons / window.action(drag/toggleMaximize/close/minimize…) | main.rs:540-545（chrome.rs HWND 面） | maximized/captionHeight | ✔（window.action close→Alt+F4 路径，浏览器预览时 toast 明示不关） |
| fs.tree / fs.read / fs.readBytes | ipc.rs:382-386 | tree{nodes}/read{text,bytes,mtimeMs}/readBytes{data(b64),bytes} | ✔（大响应 spill→`/__ipc/<id>.json` 由 app.js:400-409 materializeBigResult 消费，404→typed ipc_file 错误） |
| recent.load / recent.save | ipc.rs:396-398 | items[{path,name,kind,openedAt}] | ✔（#194 kind:"dir" 双侧一致） |
| history.list | ipc.rs:400 | items[{ts,kind,message,file,snapshot}] | ✔ |
| git.status / git.log | ipc.rs:403-415 | available/items/branch | ✔（app.js:2717 注释自认历史 bug：loadGit 曾 3 处调用未定义，#76 修复——现已接线） |
| snapshots.check/save/read/rollback | ipc.rs:423-429 | saved[]/skipped[]、content/bytes | ✔（30s tick→check 有变才 save，幂等；失败静默下轮重试——app.js:1219 注释明示） |
| annotations.load / annotations.setStatus | ipc.rs:432-434 | items[{id,block,quote,source,status,note}] | ✔（认校验三态 confirmed/pending/mismatch，状态保存 .catch→toast error 双面，app.js:1183） |
| mcp.status / llm.status / llm.save / fspec.status | ipc.rs:437-440 | 诊断三面 | ✔（api_key 只出掩码——UI 不触明文） |
| llm.test | main.rs:549-565 | **立即回帧 `{started:true}` + 后台事件 `{"ev":"llm.test",...}`** | ✔（app.js:1747 ipc() 5s 等回帧、onBridgeEvent:457 消费事件、DIAG.llmTestBusy 状态机正确——契约双面消费是难点，此处实现正确） |
| dialog.openFile / dialog.openFolder | main.rs:567（主线程模态队列） | path | ✔（12 万 ms 超时与模态时长匹配） |
| workspace.open | ipc.rs:359 | root | ✔（切换后全面板刷新） |
| explorer.reveal | ipc.rs:394 | ok | ✔ |

- UI 未消费的壳命令（preview.write、file.fromPath、fs.stat、fs.mkdir、fs.writeBytes、git.show、workspace.current）——壳多 UI 少，无害。
- **无 UI 发出而壳不认识的命令**（逐条核对 dispatch 表）。**契约零断裂。**
- postMessage 来源校验（任务重点 5）：壳侧无来源/令牌校验 = MANG-006（已由 D-MANG 登记，本审计不重复立项）；UI 侧补充观察见 UI-009（host.js 的 message 监听与 CSP 面）。

---

## 2. 状态一致性（§3.10）与任务重点 2 的逐面裁决

| 审计问题 | web-ui 事实 | 裁决 |
|---|---|---|
| 项目状态变化后 UI 是否更新 | 无 bid 项目状态 UI（N/A 面）；mang 面有 workspace.open 后全面板刷新（app.js:2489）、快照保存后 changes 面板条件刷新（:1217）、git 启动 800ms 后加载（:3218） | ✔（mang 自有状态面完备） |
| 部分成功误显示为全部成功 | 唯一批量面 = snapshots.check 多路径（saved[] + skipped[] 分开回）；UI 只 toast saved、skipped 静默 | 观察项：skipped 文件用户不可见（P3，UI-011 附注） |
| 审核失效后 UI 仍显示已审核 | mang 标注三态（confirmed/pending/mismatch）由 annotations.setStatus 落 .mang/annotations 边车；重开文件 annotations.load 重读——**边车为真源，UI 无本地缓存** | ✔ |
| 证据待核验显示为有效证据（PB-001/002 VerificationTier） | web-ui **零** bid 域证据 UI；src/ 的 evidence_panel 是 mdcore WasmWorkbench（八步证据环）演示面：卡片同时渲染 linkage/calc/review 三徽章 + stale 计数 + `refresh()` 每操作重读 `evidence_json()`（evidence_panel.ts:764-777）——无"已核验"误显示；但它与 bid-mcp register_evidence 的 VerificationTier **无任何数据通路** | UI 无误显示；缺口在"无消费面"（UI-002） |
| 导出失败误显示成功 | src/ 面 __exportDocx/__exportXlsx/__gridExportEdits：catch→typed code/message→状态行 error + __last*.ok=false（main.ts:2388-2404、xlsx_grid.ts:1130-1134）；mang 面 saveActive 为只读存根（toast 明示"保存将在文档编辑阶段接入"，app.js:2763） | ✔ 双面诚实 |
| xlsx #42 联动失败回滚 | gridOpenBytesWhenReady fire-and-forget，失败仅回填 __lastXlsxImport.grid + 不显示后缀；主编入不回滚（注释明示"增强面非前置条件"） | ✔ 语义诚实（部分失败不伪装成功——后缀缺席） |

**结论行：状态误显示风险——当前代码未发现任何"失败显示为成功/失效显示为有效"实例；真实风险是 bid 域状态面在前端整体缺席（UI-002，P1 架构登记）。**

---

## 3. 逐文件审计记录（覆盖核对：git ls-files 156 文件全数过目）

### 3.1 src/（旧 mdcore web 资产——逐文件全读）

| 文件 | 行数 | 职责 | 关键发现 |
|---|---|---|---|
| main.ts | 3347 | CM6↔wasm 桥 + 导入导出 + 设置面板 + 状态栏/TOC/查找/字号/帮助 | i18n 硬编码缺口集中地（设置面板/TOC 空态/导出提示，UI-005）；import @codemirror/* 与 ../pkg/mdcore_wasm.js（UI-001/008 的受害面）；设置键封闭集+校验器纪律好 |
| draft.ts | 240 | localStorage 崩溃草稿 + beforeunload | 与 MCP project_version 无关（重点 4 裁决 N/A）；三重激活门/坏信封清稿/超限清稿不误恢复——实现诚实 |
| evidence_panel.ts | 983 | WasmWorkbench 八步证据环 UI | review 门（human: 前缀）/atMs 单调/restore 后 sourceSeq 重置与 clockMs 对齐（Review S5/N2 注释）——质量高；少量硬编码英文（UI-005） |
| canvas_view.ts | 1680 | SVG 保留模式 + WasmCanvas 双实例协同 | slot 平移防 client_id 撞车（7_200_001+slot×10）；rAF 泵 + PUMP_MAX_ROUNDS 防回声 |
| pdf_view.ts | 2127 | pdf.js 显示面 + mdc-pdf 会话 | teardownDocument 全清（renderTask.cancel/loadingTask.destroy/bitmap.close）；LRU 64MB；**8k×64M px 渲染上限自注释（1398 行）**；innerHTML 仅注入数值坐标/受控 hex（无污点） |
| xlsx_grid.ts | 1277 | WasmBook 虚拟网格 + 编辑台账导出 | MAX_WINDOW_CELLS 窗口钳制；exportEdits 鸭子探测 bridge_absent typed 降级；openBytes 换簿无显式旧 book 释放（UI-010） |
| collab_channel.ts | 470 | BroadcastChannel 跨页 CRDT harness | UI-003/004（断线丢失 + 空 sv 歧义） |
| empty_state.ts | 80 | 三表面空态引导卡 | 无问题（pointer-events:none 不拦拖放；textContent-only） |
| bench.ts | 491 | 宿主选型基准页 | import @codemirror + fetch ./bench-data/*.json（bench-data 不在 git——历史遗留，仅 build:legacy 面） |
| wasm_boot.ts | 44 | wasm 单飞 init 门 | **正面样板**：WeakMap 单飞 + 失败重置（可重试）；文件头详载双实例化 SEGV 根因（2026-10-23 页面崩溃根治）——任务重点 7 的降级链见 §4 |
| i18n/index.ts | 83 | 语言态 + t()/tCount() + data-i18n 三属性面 | t() 未知键回退 en→键名（开发期可见漏译）；语言切换=location.reload（诚实简单） |

### 3.2 src/i18n/（键集对账——脚本双向比对）

- en.ts 268 键 = zh-CN.ts 268 键，**双向零缺失**；zh-CN 以 `Dict` 类型（`import type { Dict } from './en.js'`）锁完备性——漏键=编译错。✔ 正面
- 缺口不在词典而在**绕过 t() 的调用点**（UI-005 清单见问题记录）。

### 3.3 mang/（桌面宿主页——逐文件全读）

| 文件 | 行数 | 关键发现 |
|---|---|---|
| app.js | 3224 | esc()（:4-6）全量转义纪律 ✔（57 处 innerHTML 动态插值逐一抽查，仅 :3214 boot 失败盒 String(err.message) 未过 esc——UI-012）；demo bridge（无壳降级，含完整模拟响应集与 unsupported typed 拒绝）；loadTab 失败→error 态 + 重试按钮（状态栏同步 error）——加载失败不伪装成功 ✔ |
| host.js | 108 | iframe 形态桥转发（默认不用）；message 监听无 origin 校验（UI-009） |
| index.html | 84 | CSP meta（script-src 'unsafe-inline'——UI-009）；无脚本逻辑 |
| styles.css | 2622 | 覆盖性检查：变量系统/浅深双主题/溢出布局；无 @import 外链、无 expression 类风险面 ✔ |
| qa-probe.css | 14 | QA 截图探针轮廓样式（手工 QA 遗产，佐证 UI-007） |

### 3.4 src/mang/（桌面渲染器——逐文件全读）

| 文件 | 行数 | 关键发现 |
|---|---|---|
| docx_view.ts | 2134 | 自写 ZIP 解析（EOCD→中央目录→本地头，仅 stored/deflate）+ MAX_ENTRY_COUNT 反炸弹第一道门；dispose 幂等 + blob: URL 登记表统一 revoke + 异步构建竞态（disposed 标志链在构建任务后）✔ 正面 |
| xlsx_view.ts | 2541 | 无 wasm 自写解析；防炸弹上限矩阵（4096 条目/32MB 单条/64MB 总量/30 万格）；EAGER_SHEETS=6 同步预解析 + 其余 lazy；disposed+loadSeq 双守卫 ✔ 正面 |
| pdf_view.ts | 1381 | legacy 构建 + useWorkerFetch 三资源齐全门；worker 失败退主线程（注释明示"更慢但能显示"）；v6 destroy 挂 loadingTask（:436 注释）✔ |

### 3.5 根文件与构建（逐文件全读）

- **tsconfig.json**：strict:true ✔、noEmit、target ES2022、include 含 `pkg/mdcore_wasm.d.ts`——pkg/ 不在 git（从未提交，.gitignore 亦未列）→ 干净 clone 上 typecheck 必挂（UI-008 证据链）。
- **package.json**：仅 build/build:legacy/serve/typecheck 四 script——**无任何测试框架**（dependencies: pdfjs-dist；devDeps: esbuild/typescript）。E2E 工程缺口登记（UI-007）。
- **package-lock.json**：lockfileVersion 3；**根段 name="mdcore-web" 且 dependencies 含 @codemirror/commands/state/view**——与 package.json（"mang-web-ui"，无 codemirror）漂移（UI-001，P1）。
- **build.mjs**（legacy）：入口 src/main.ts + src/bench.ts（都 import @codemirror → 干净 install 后 esbuild 解析失败）；报错文案引用 `./build-wasm.sh`（全仓不存在）与 `npm run build:wasm:pdf` / `build:full`（package.json 无此 script）——UI-008。
- **mang-build.mjs**（主构建）：入口 mang/app.js + stdin 包装 src/mang/*.ts——**不触 codemirror/pkg**，当前可用 ✔（node_modules 缺失时 pdf worker/静态资源拷贝静默跳过——:70/:82 条件存在性检查，行为是"少拷资源"而非失败，登记于 UI-008 附注）。
- **scripts/serve.mjs**：开发服务器；`resolve(ROOT,'.'+file)` + startsWith(ROOT) 逃逸防护 ✔（但 decodeURIComponent 先行——路径含 ../ 时 resolve 后仍被 startsWith 拦，OK）。
- **index.html**（637 行）：防闪白内联脚本 + data-i18n 62 处；`<script type="module" src="./main.js">`（依赖 build:legacy 产物）。
- **bench.html / index.host.html**：覆盖性检查完成（宿主页 boot veil + 6s 慢加载提示）。
- **tests/results/results_concurrent_web.json**：见 UI-006。
- **mang-shots/**（149 PNG + qa-probe.css）：手工截图 QA 存档——qa-before/qa-after 1440x900 系列双拍对照 + real-* 真实文件系列 + host-* 壳内验证；**证明视觉回归曾以人工流程执行，非自动化**。

---

## 4. wasm 加载失败降级行为（任务重点 7——专项结论）

**分层降级矩阵（全部 CONFIRMED by reading）：**

| wasm 面 | 缺失时的行为 | 用户可见性 |
|---|---|---|
| pkg/mdcore_wasm（主核） | `main().catch` → #debug 行 `boot failed: <e>`（main.ts:3113-3117）+ throw（控制台） | 弱：仅一行 debug 文本，编辑器空白，无重试按钮 |
| WasmCollabSession | typed `{ok:false,code:'bridge_absent'}`，通道照常（BroadcastChannel 不依赖 wasm） | 探针面诚实；UI 无专门提示（harness 面） |
| WasmWorkbench | 同上 bridge_absent；面板 inert-but-alive | evidence 面按钮可用但全拒（e2e SKIP 门） |
| WasmCanvas | 同上；工具条照常构建 | canvas 面显示但模型操作全拒 |
| WasmBook（grid） | 同上；`gridOpenBytesWhenReady` 5s 有界等待→not_ready typed | 状态行 grid.openFailed/未联动后缀 |
| pkg_pdf（mdc-pdf） | `catch → pdfWasm=null` → display-only 模式 + `t('pdf.displayOnlyPending')` warn（pdf_view.ts:2104-2106/919） | **最佳实践**：明示降级 + 标注/搜索面 typed 拒绝 |
| pdfjs worker | import 失败 → setupPdfView 外层 catch（:2111）→ 面板 boot 失败文本 | 面板内错误文本 |
| pdf.worker 路径错误 | pdf.js 退主线程解析（mang/pdf_view.ts:142 注释） | 更慢但能显示 ✔ |

**裁决**：降级链路设计完整（单飞门 + 失败重置可重试 + typed bridge_absent 全家桶）；唯一弱面是主核 boot 失败的用户反馈仅一行 debug 文本（P3 归入 UI-005 附注）。

---

## 5. 问题记录

---

ID: UI-001
严重程度: P1
模块/文件路径: web-ui/package.json + web-ui/package-lock.json（受害面 src/main.ts、src/bench.ts）
代码行号: package.json:2-19（name "mang-web-ui"、dependencies 无 codemirror）；package-lock.json:2-20（根段 name "mdcore-web"、dependencies 含 @codemirror/commands ^6.11.1 / state ^6.5.2 / view ^6.36.5）；main.ts:46-59（import @codemirror/state|commands|view）；bench.ts:14-15
函数或接口: npm 依赖解析 / build:legacy / typecheck
问题描述: package.json 改名并删除 @codemirror 三依赖后，package-lock.json 未重新生成——lock 根段仍是旧 manifest（mdcore-web + codemirror 三件）。① `npm ci` 因 manifest 与 lock 不一致直接 EUSAGE 拒绝（可复现的安装路径断裂）；② 干净环境 `npm install` 会按新 manifest 装出无 codemirror 的 node_modules → `npm run build:legacy`（esbuild 打 main.ts/bench.ts）解析不到 @codemirror 立即失败；③ `tsc --noEmit` 同样因模块缺失报错（叠加 UI-008 的 pkg/mdcore_wasm.d.ts 缺失）。
调用链: npm ci/install → node_modules 解析 → build.mjs esbuild bundle(src/main.ts) / tsc include src/**/* → @codemirror/* 无法解析 → 构建/类型检查失败
触发条件: 任何干净环境（新 clone / CI / 新机器）安装或构建 legacy 面
实际影响: 主构建（mang-build.mjs，不引 codemirror）不受影响，但 legacy 演示面与 typecheck 在干净环境必断；npm ci 完全不可用；依赖面真源二义（以 lock 还是 manifest 为准）。
复现步骤: 删 node_modules → npm ci（观察 EUSAGE）；或 npm install 后 node build.mjs。
当前测试: 无（无 CI）
缺失测试: 干净环境构建冒烟（Lead 统一 typecheck 时将实测命中）
建议方案: 二选一并落档：A) 恢复 package.json 的 @codemirror 三依赖（保留 legacy 可构建）；B) 正式废弃 legacy 面（移除 build:legacy/bench.html/index.html 主页或移入 archive/）并重生成 lock。
证据:
```
package.json:  "name": "mang-web-ui", "dependencies": { "pdfjs-dist": "^6.4.299" }
package-lock.json: "": { "name": "mdcore-web", "dependencies": {
  "@codemirror/commands": "^6.11.1", "@codemirror/state": "^6.5.2", "@codemirror/view": "^6.36.5", "pdfjs-dist": "^6.4.299" } }
main.ts:46: import { EditorSelection, EditorState, RangeSetBuilder, StateField, Text, Transaction } from '@codemirror/state';
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: UI-002
严重程度: P1
模块/文件路径: web-ui/ 全域（结构性，非单文件缺陷点）
代码行号: package.json:5（"旧 mdcore web 资产（src/ 等，仅供组件参考搬运）"）；mang/app.js 全文（唯一后端接口= 壳 IPC，无 MCP 客户端）
函数或接口: 前后端契约（任务重点 1 的对象本身）
问题描述: web-ui 对 bid-mcp 25 工具面零调用、零响应解析、零 E_* 错误码处理——标书业务（项目状态机/合规矩阵/证据 tier/审核门/评分/导出门）在前端没有任何消费 UI。§3.10 的全部状态一致性问题的"误显示"分支因无显示面而不可触发，但"正确显示"分支同样不存在。
调用链: （无——这正是问题：应存在 UI→bid-mcp（经宿主或直连）的调用链而不存在）
触发条件: 用户在 mang 工作台查看/操作标书项目时
实际影响: bid-mcp 的产出（db.json/矩阵/证据/审核记录）只能由 AI 客户端驱动，桌面壳侧无任何业务可见面——工作台目前仅是通用文件浏览器 + .mang 快照/标注管理。这不是缺陷而是当前建设阶段（issue #42 路线：壳先行、业务 UI 后续），但必须在审计中作为契约断裂点 Top1 登记。
复现步骤: grep web-ui 无任何 25 工具名（本审计已执行，零命中）。
当前测试: N/A
缺失测试: 未来接入时的契约对账测试（工具名/参数/响应结构 vs docs/bid-mcp-contract.md）
建议方案: 在后续迭代规划中明确"业务 UI 接入通道"（经壳扩展 IPC 转发 MCP，或 AI 客户端渲染）；接入时以 C1 工具表 + MCP-001~019 已知偏差为对账基线（尤其 project_version 可选语义、E_PARTIAL 带 data 成功面、isError 双通道）。
证据:
```
package.json: "description": "mang 标书工作台前端：VSCode 风格文档工作台（web-ui/mang）+ 旧 mdcore web 资产（src/ 等，仅供组件参考搬运）"
# rg 'create_bid_project|import_tender_files|list_evidence|register_evidence|record_review_gate|export_bid_document' web-ui/ → 0 hits
```
审计置信度: CONFIRMED
状态分类: 文档与实现不一致（路线图宣称的"标书工作台"当前无标书业务面）

---

ID: UI-003
严重程度: P2
模块/文件路径: web-ui/src/collab_channel.ts
代码行号: pumpDoc():149-171（take_events_json 排空后逐条 post）；post():141-146（`if (!channel || !connected) return false`——字节丢弃）；__collabReconnect():395-406（重连只发自方 sync_req）
函数或接口: pumpDoc / post / __collabReconnect / __collabDisconnect
问题描述: 断线期间本地操作外发丢失。40ms 泵与 __collabExec 懒排空在 connected=false 时仍执行 take_events_json()（取出并清空 wasm 事件缓冲），随后 post() 因连接门直接返回 false——这些 update 字节被永久丢弃（不再进任何重发队列）。重连 catch-up 只单向：重连方发 sync_req 收别人的差量（使自己被补偿），但对端不会（重新）发 sync_req（hello 不触发对端请求，注释 :183-185 明示"既有成员不主动推"）→ 断线期间产生的本地操作对对端不可见，直到某成员再次加入/重连才经 docDiffFor 传播。
调用链: 用户编辑/__collabExec → pumpDoc → take_events_json（缓冲清空）→ post（connected=false → 丢弃）→ 重连 → sync_req（仅自方收差量）→ 对端永不收该操作
触发条件: __collabDisconnect 后执行任何 doc/canvas 操作，再 __collabReconnect
实际影响: harness/演示面（双 tab CRDT 协同）的最终一致窗口无界；对生产无影响（该模块属旧资产参考面）。
复现步骤: p1 `__collabDisconnect()` → `__collabExec('ai','...插入...')`（懒排空即丢弃）→ `__collabReconnect()` → p2 `__collabDoc()` 不含该插入。
当前测试: results_concurrent_web.json 有 disconnect/reconnect 相关面（collab_state_consistency 显示 connected/sent/recv）但无"断线期间操作传播"用例
缺失测试: disconnect→local-edit→reconnect→对端收敛 用例
建议方案: pumpDoc 在 !connected 时跳过 take（保留缓冲）；或断线期收集 update 至重发队列，重连时与 sync_req 一并发出（CRDT 幂等保证重发安全）。
证据:
```
function post(msg: Record<string, unknown>): boolean {
  if (!channel || !connected) return false;   // ← 字节在此丢弃
  channel.postMessage(msg); ...
}
function pumpDoc(): void {
  ...
  raw = docSession.take_events_json();        // ← 缓冲已被排空
  for (const b64 of ev.updates) { post({ k: 'doc_update', from: mySlot, b64 }); }
```
审计置信度: CONFIRMED
状态分类: 当前仍存在（旧资产参考面，生产不消费）

---

ID: UI-004
严重程度: P2
模块/文件路径: web-ui/src/collab_channel.ts
代码行号: joinerCatchUp():243-248（`docSv = docSession ? bytesToBase64(docSession.state_vector()) : ''`）；docDiffFor():251-260（`if (!docSession || svB64.length === 0) return ''`）
函数或接口: joinerCatchUp / docDiffFor / canvasDiffFor（:263-269 同构）
问题描述: 空状态向量与"无会话"共用空串表示——新会话（刚 new、零操作）的 state_vector() 若序列化为空字节，joinerCatchUp 发出的 docSv='' 被对端 docDiffFor 判为"请求方无会话"而返回空差量 → 刷新页面/新加入者可能拿不到任何 catch-up 内容。:241 注释自认 p6-t6 修正"必须待会话面就绪后带真实 sv"，但空会话的"真实 sv"就是空。
调用链: 刷新/新开 ?cc= 页 → docSession = new ctor(...)（零操作）→ joinerCatchUp → docSv='' → 对端 sync_resp.docDiff='' → 本页内容为空
触发条件: 新建会话 state_vector() 为空字节时加入已有房间（取决于 wasm 序列化是否对空会话产生非空 sv，如含 client_id 条目则不触发）
实际影响: 依赖 wasm 面行为，无法纯读码定论；若触发则新成员静默拿不到全量。
复现步骤: 双 tab 建立内容 → 第三 tab 开同 room → 观察第三 tab 是否收到内容。
当前测试: results_concurrent_web.json 的 dual_tab_doc_bidirectional_convergence（已含内容的两 tab）不覆盖"空会话后加入"路径
缺失测试: 空 sv 加入者的全量 catch-up 断言
建议方案: 协议中空 sv 用哨兵（如 "-"）或 docSv 字段缺席语义区分"全量请求"；或对端对空 sv 调 encode_diff(空数组)（= 全量快照）而非短路返回 ''。
证据:
```
const docSv = docSession ? bytesToBase64(docSession.state_vector()) : '';
...
function docDiffFor(svB64: string): string {
  if (!docSession || svB64.length === 0) return '';   // 空 sv 与无会话混同
```
审计置信度: NEEDS_RUNTIME_VERIFY
状态分类: 尚无法复现

---

ID: UI-005
严重程度: P2
模块/文件路径: web-ui/src/main.ts + src/evidence_panel.ts + src/pdf_view.ts + src/xlsx_grid.ts + src/canvas_view.ts（i18n 缺口清单）
代码行号: main.ts:2762（TOC 空态 `listEl.innerHTML = '<div class="toc-empty">暂无标题——…</div>'`——硬编码中文且走 innerHTML）；main.ts:2769（`(空标题)`）；main.ts:2430-2434（导出选项 refreshHint 三条中文模板）；main.ts:3196-3204（SETTINGS_FIELDS 全部 label/hint 硬编码中文：'撤销合并窗口 (ms)' 等 9 行）；main.ts:3223-3316（openSettingsPanel 的 '设置'/'关闭 (Esc)'/`✗ …需数字`/`✓ …已保存`/`✗ …值域外`）；main.ts:3115（`boot failed: ${e}`）；evidence_panel.ts:549（`calculation registered: ${value}`）；:792-794（`${n} items · ${p} pending · …` 纯英文拼串）；:831（title 'jump to claim paragraph'）；:837（`value ${value}`）；:855-857（按钮字面 'approve'/'reject'/'reverify'）；:980（boot failed）；pdf_view.ts:2065（`evidence jump: paragraph ${idx}…`）；xlsx_grid.ts:1274 / canvas_view.ts:1677 / evidence_panel.ts:980（`grid/canvas/evidence panel boot failed: …`）
函数或接口: t()/tCount() 的绕过点（硬编码用户可见文案）
问题描述: 双语词典（en/zh-CN 各 268 键、Dict 类型锁）完备，但源码存在两族绕过：① 整个设置面板（含 9 项 label/hint 与保存/拒绝反馈）零 i18n；② 各面板零散英文/中文硬编码状态文案。zh 用户在 en 界面会看到中文设置面板；en 用户在 zh 界面会看到英文 evidence 卡片计数行——语言切换（location.reload 重建）也无法消除。TOC 空态还以 innerHTML 注入固定中文串（非污点，但违反本项目 textContent-only 纪律）。mang/ 宿主页为中文单语（设计选择，无 i18n 机制——若未来要双语需整体建系统，登记为架构性差异而非缺陷）。
调用链: initI18n 后 t() 可用 → 上述调用点直接拼字面量 → 与 current 语言态无关
触发条件: 任意语言下打开设置面板/TOC 空态/证据面板/导出选项面板
实际影响: 双语完整性破损（产品宣称 #43 双语面）；跨语言用户看到混合文案。
复现步骤: 设置 language=en → 打开设置面板（中文）；打开大纲面板无标题文档（中文空态）。
当前测试: en-locale 测试锚只锁 t() 面（状态行等），未覆盖设置面板
缺失测试: 设置面板/TOC 空态的 locale 断言
建议方案: SETTINGS_FIELDS/面板反馈/散点文案补键入词典（约 +40 键）；TOC 空态改 textContent。
证据:
```
main.ts:2762: listEl.innerHTML = '<div class="toc-empty">暂无标题——用 H1-H3 或 # 前缀创建。</div>';
main.ts:3225: title.textContent = '设置';
main.ts:3308: status.textContent = `✓ ${f.label} 已保存`;
evidence_panel.ts:793: `${this.evidence.length} items · ${c.pending} pending · ${c.approved} approved · ${c.rejected} rejected · ${c.stale} stale`
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: UI-006
严重程度: P2
模块/文件路径: web-ui/tests/results/results_concurrent_web.json（目录下唯一文件）
代码行号: meta 段 :2-8（`"dist": "E:\\Code\\ai\\md\\audit\\mdcore\\web\\dist"`、`"gate": "stage 6 ¡ì¾Å.5 concurrent web composition e2e"`）；summary :107-112
函数或接口: （测试产物文件——审计其来源与可信度，任务重点 10）
问题描述: ① 该文件是**外部历史项目（mdcore）的人工执行结果快照**：meta.dist 为 Windows 绝对路径且不属于本仓（E:\Code\ai\md\audit\mdcore）；git 历史显示它在 c1cbaf5（feat #42 搬入 mang 壳时）随大批资产一次性入库，非本仓任何脚本产物。② gate 字段 "¡ì¾Å" 是 "§九" 的 GBK/UTF-8 mojibake——文件经历过编码损坏或来自控制台重定向。③ 驱动脚本（e2e runner）不在 web-ui/（scripts/ 仅 serve.mjs）——本仓无法复现。④ 16 用例内容为 collab_channel/web harness 的双 tab CRDT 协同、undo 传播、expect guard 等（消费 __collab*/__mdcoreEvents 探针），全部属于旧 mdcore 资产面。
调用链: （历史）mdcore 仓 e2e runner → 手工执行 → JSON 落盘 → 整体拷贝入库
触发条件: N/A（不可再触发）
实际影响: 作为"web 测试通过"的引用时可信度=存档证据（人工快照、环境已消失、编码受损）；不能作为回归基线；未覆盖 mang 面（当前真正的前端）任何路径。
复现步骤: 不可复现（无 runner、无 dist、无 wasm pkg）。
当前测试: 本文件即"当前测试"的全部
缺失测试: 可在本仓执行的 web/mang 前端自动化测试
建议方案: 移至 audit/archive/ 或在文件头/README 标注"历史 mdcore 项目产物，非本仓可复现测试"；防止后续被误当 CI 证据引用。
证据:
```
"meta": { "gate": "stage 6 ¡ì¾Å.5 concurrent web composition e2e",
  "dist": "E:\\Code\\ai\\md\\audit\\mdcore\\web\\dist", "port": 8943, "total": 16, "failed": 0 }
# git log --follow: c1cbaf5 feat(#42): mang 桌面壳……（一次性搬入）
```
审计置信度: CONFIRMED
状态分类: 文档与实现不一致（作为"测试"的存在性与仓库现实不一致）

---

ID: UI-007
严重程度: P2
模块/文件路径: web-ui/package.json（scripts）+ web-ui/tests/（空壳）+ mang-shots/ + mang/qa-probe.css
代码行号: package.json:7-12（scripts 仅 build/build:legacy/serve/typecheck）；qa-probe.css 全文（14 条 .qa-* 探针轮廓）；mang-shots/ 149 张 PNG（qa-before-/qa-after-1440 系列、real-* 系列、host-* 系列）
函数或接口: 测试工程面（任务背景指令的如实登记）
问题描述: web-ui 无浏览器 E2E 框架、无单测框架、无 CI 接线：devDependencies 无 playwright/puppeteer/vitest/web-test-runner 任何一项；tests/ 目录只有一个外部结果 JSON；package.json 无 test script。历史质量流程 = 人工截图对照（qa-before/qa-after 成对 + qa-probe.css 高亮目标区域 + 3 种分辨率 + 深浅双主题 + 真实文件系列）——流程认真但完全手工，不可回归。
调用链: N/A
触发条件: 任何前端改动
实际影响: mang/app.js 3224 行、渲染器 6056 行、app.js 57 处 innerHTML 的 esc 纪律全部无自动化护栏——回归只能靠人工再截一轮图。
复现步骤: npm run（无 test 可选）。
当前测试: 手工截图 QA（证据在库）
缺失测试: 最低限自动化建议：① esbuild 构建冒烟（干净 install 后 mang-build.mjs 成功）；② app.js esc 纪律静态断言（innerHTML 动态插值必须经 esc()——可写 grep 级检查脚本）；③ 关键 IPC 对账快照（ipc cmd 集合 vs 壳 dispatch 表）；后续再上 Playwright。
建议方案: 如上三级递进；Lead 统一执行 typecheck 时一并落 build 冒烟。
证据:
```
package.json scripts: "build" | "build:legacy" | "serve" | "typecheck"   （无 test）
devDependencies: esbuild ^0.25.0, typescript ^5.7.0                    （无测试框架）
web-ui/tests/results/ 仅 results_concurrent_web.json 一个文件
```
审计置信度: CONFIRMED
状态分类: 当前仍存在（工程缺口如实登记——本审计未做也未声称做 E2E）

---

ID: UI-008
严重程度: P3
模块/文件路径: web-ui/build.mjs + web-ui/tsconfig.json + web-ui/pkg/（缺席）
代码行号: build.mjs:23-25（`missing pkg/mdcore_wasm_bg.wasm — run ./build-wasm.sh first`——build-wasm.sh 全仓不存在）；build.mjs:69（`run npm run build:wasm:pdf (or npm run build:full)`——package.json 无此二 script）；tsconfig.json:12（include `pkg/mdcore_wasm.d.ts`——pkg/ 不在 git，从未提交）
函数或接口: build:legacy 依赖链 / typecheck include
问题描述: ① legacy 构建的 wasm 前置（build-wasm.sh/build-pdf-wasm.sh）不在本仓——错误提示指向不存在的可执行路径；② 提示语引用的 npm script（build:wasm:pdf/build:full）未在 package.json 定义；③ tsconfig include 的类型声明文件（pkg/mdcore_wasm.d.ts）不在版本控制——干净 clone 上 `npm run typecheck` 在 codemirror（UI-001）之外还叠加 pkg 缺失错误；④ mang-build.mjs 对 node_modules 缺失（pdf worker/cmaps/standard_fonts/wasm）是静默跳过（:70/:82 条件判断）——产物缺资源时无 fail-fast，运行时才以渲染降级暴露。
调用链: 干净 clone → npm install → npm run typecheck（pkg/*.d.ts 缺失报错）/ node build.mjs（pkg/mdcore_wasm_bg.wasm 缺失 exit 1）
触发条件: 干净环境构建/类型检查
实际影响: 与 UI-001 叠加使"干净环境可构建"仅剩主构建 mang-build.mjs 一条路（且其资源拷贝为静默降级）。
复现步骤: 见调用链。
当前测试: 无
缺失测试: 干净环境构建冒烟
建议方案: 把 wasm 构建脚本入库或改提示指向真实入口（如根目录 Makefile/justfile）；npm script 名对齐；tsconfig 用 pkg 遗憾性 include 可改为 `pkg/**/*.d.ts` + 文档说明。
证据:
```
build.mjs:24: console.error('missing pkg/mdcore_wasm_bg.wasm — run `./build-wasm.sh` first');
build.mjs:69: '... run `npm run build:wasm:pdf` (or `npm run build:full`) to enable the model face'
tsconfig.json:12: "include": ["src/**/*.ts", "pkg/mdcore_wasm.d.ts"]
# git ls-files web-ui/pkg/ → 空
```
审计置信度: CONFIRMED
状态分类: 文档与实现不一致

---

ID: UI-009
严重程度: P3
模块/文件路径: web-ui/mang/index.html + web-ui/mang/host.js + web-ui/mang/app.js
代码行号: mang/index.html:6（CSP `script-src 'self' 'unsafe-inline' blob:`）；host.js:95-103（`window.addEventListener('message', …)` 无 e.origin/e.source 校验，仅处理 `d.ev==='ready'` 隐藏 boot veil）；app.js:2908（mdToHtml 链接 `<a href="$2" target="_blank" rel="noreferrer">` —— href 无 scheme 白名单）；app.js:3031-3037（链接点击 preventDefault + toast，拦截直接导航）
函数或接口: CSP meta / message 监听 / mdToHtml inline()
问题描述: 三层纵深弱化点（均被下游防线缓解，故 P3 观察而非漏洞）：① 页面自带 CSP 是 MANG-006（壳 serve 无 CSP）的唯一 UI 侧缓解，但 script-src 'unsafe-inline' 允许任意内联脚本——一旦发生注入即失去 CSP 这一层；② host.js 的 message 监听不校验来源，任意嵌入页/弹窗可 postMessage {ev:'ready'} 提前撤掉启动遮罩（影响=遮罩提前消失，无数据面）；③ 手写 markdown 渲染器输出 `<a href>` 不滤 javascript:/data: scheme——esc() 只转义实体不滤 scheme；但点击被 preventDefault 拦截，target=_blank 的新窗口路径落入壳的导航白名单（main.rs:484-490 blob:/data:/about:），链条长且有兜底。
调用链: 恶意 .md 文件 → mdToHtml → javascript: href →（点击被拦 / 新窗口走壳白名单）
触发条件: 打开含 `[x](javascript:...)` 的 md 文件并设法绕过点击拦截
实际影响: 当前无可达路径；作为 MANG-006 的纵深补强清单项登记。
复现步骤: 构造恶意 md 打开 → 点击链接（toast 拦截）→ 中键/右键新窗口（壳白名单裁决）。
当前测试: 无
缺失测试: mdToHtml 输出的 scheme 过滤单测（若未来放开点击拦截则升 P1）
建议方案: mdToHtml 链接仅允许 http(s)/mang: scheme（其余降级纯文本）；host.js message 校验 e.source === frame.contentWindow；CSP 收紧（渲染器体系无内联脚本需求——qa-probe/样式不受影响）。
证据:
```
mang/index.html:6: content="default-src 'self' blob: data: mang:; script-src 'self' 'unsafe-inline' blob: ..."
app.js:2908: x = x.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');
host.js:95: window.addEventListener("message", function (e) { var d = e.data; if (!d || typeof d !== "object") return; if (d.ev === "ready") { if (veil) veil.hidden = true; ...
```
审计置信度: CONFIRMED（缓解链）/ 攻击可达性 SUSPECTED
状态分类: 需要真实环境验证

---

ID: UI-010
严重程度: P3
模块/文件路径: web-ui/src/xlsx_grid.ts
代码行号: openBytes():1040-1075（`const book = new this.mod.WasmBook(bytes); this.book = book;`——旧 this.book 直接被覆盖，无 free()/drop 调用；全文 grep 无释放面）
函数或接口: XlsxGridUI.openBytes / WasmBook 生命周期
问题描述: 换开工作簿时旧 WasmBook（wasm 侧含解析后的工作表模型）无显式释放——依赖 wasm-bindgen 的 GC/FinalizationRegistry 兜底；多次大簿开合时 wasm 线性内存峰值回收时机不确定。对照 pdf_view 的 teardownDocument（renderTask.cancel + loadingTask.destroy + bitmap.close 全清）与 mang 渲染器的 dispose 幂等纪律，这是四视图里唯一无释放路径的面。同族：collab_channel 的 docSession 在 __collabRestoreDoc 中直接换新实例（:378），旧会话同样靠 GC。
调用链: 用户连续开多个 xlsx → openBytes → 旧 book 失引用 → GC 不确定延迟 → wasm 内存峰值抬升
触发条件: 反复打开大工作簿（grid 面自身有 64MB 解压上限兜底——单簿规模受控）
实际影响: 旧资产演示面；内存峰值型风险，无功能错误。
复现步骤: 连续 openBytes 多个大 xlsx 观察 performance.memory/wasm memory 增长（需 --enable-precise-memory-info）。
当前测试: 无
缺失测试: 换簿内存曲线
建议方案: 若 pkg 侧 WasmBook 暴露 free() 则调用；否则文档登记 GC 兜底语义。
证据:
```
openBytes(bytes: Uint8Array) { try {
    const book = new this.mod.WasmBook(bytes);
    this.book = book;                    // ← 旧实例无释放
```
审计置信度: CONFIRMED（代码面）/ 影响面 NEEDS_RUNTIME_VERIFY
状态分类: 当前仍存在

---

ID: UI-011
严重程度: P3
模块/文件路径: web-ui/mang/app.js
代码行号: snapshotTick():1213-1221（`const saved = (r && r.saved) || []; if (saved.length) toast(...)`——r.skipped 分支无任何用户可见反馈）；loadTab():2571-2575（savedText = fmtBytes(tab.meta.bytes)——"已保存"槽位显示字节数）；saveActive():2763-2768（存根 toast）
函数或接口: snapshotTick / loadTab 状态栏 / saveActive
问题描述: ① snapshots.check 的 skipped[]（如"演示模式"/未变化跳过/异常原因）静默——用户不知道某些打开文件本轮没存上快照（与"部分成功显式化"的诚实边界纪律不一致，但失败会下轮重试且壳侧 skipped 的 reason 主要是正当跳过）；② 状态栏"已保存/字节数"语义混用：加载成功后 savedText 显示文件字节数（label 是"已保存"系文案），实际语义是"已读取"——只读浏览阶段无保存动作，字节数当保存态显示属口径混淆（无功能危害）；③ Ctrl+S 是明示的存根（诚实）。
调用链: 30s tick → snapshots.check → saved[] toast / skipped[] 静默
触发条件: 多文件打开 + 部分文件快照跳过
实际影响: 低——快照 skipped 主因是"无变化"；字节数口径只影响文案语义。
复现步骤: 开两个 tab（一个 .md 一个 .docx）→ 等待 30s → 只有 saved 文件有 toast。
当前测试: 无
缺失测试: 快照部分失败的用户可见性
建议方案: skipped 含非"无变化" reason 时（如演示模式/错误）降级 toast 或状态栏计数；savedText 改"已读取 N KB"文案。
证据:
```
ipc("snapshots.check", { paths }, 3e4).then((r) => {
  const saved = (r && r.saved) || [];
  if (saved.length) { ui.toast("info", "已存版本快照：" + ...); ... }
}).catch(() => { /* 快照失败不打扰（下一轮再试） */ });
savedText: tab.meta.bytes ? fmtBytes(tab.meta.bytes) : "已保存"
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: UI-012
严重程度: P3
模块/文件路径: web-ui/mang/app.js
代码行号: :3214（`box.innerHTML = '<div class="view-state" ...><div>界面启动失败</div><div>' + String(err && err.message ? err.message : err) + "</div></div>"`——唯一未过 esc() 的动态 innerHTML sink）
函数或接口: boot() 的 catch 兜底
问题描述: 启动失败盒直接拼 String(err.message) 进 innerHTML。err 来自 boot() 同步段的内部异常（DOM/bridge 构造），当前无用户可控内容进入 message 的路径；但按 app.js 自身"动态插值必过 esc()"的纪律（本审计逐条抽查 57 处 innerHTML 仅此一处例外），这是纪律破口——若未来 boot 路径异常消息携带文件名（如 workspace.open 失败重抛），文件名可含 HTML 元字符。
调用链: boot() throw → catch → innerHTML 拼接
触发条件: 启动异常（当前基本不可达）
实际影响: 防御性缺口，非现行漏洞。
复现步骤: 需注入异常消息（当前无路径）。
当前测试: 无
缺失测试: N/A（一行修复）
建议方案: `esc(String(err && err.message ? err.message : err))`。
证据:
```
box.innerHTML = '<div class="view-state" data-kind="error" ...><div>界面启动失败</div><div>' + String(err && err.message ? err.message : err) + "</div></div>";
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

## 6. 正面确认清单（防回归资产——后续审计可直接引用）

1. **IPC 契约零断裂**（§1 表）：27 调用点全对齐壳 dispatch；llm.test 的"立即回帧 + 后台事件"双面契约消费正确（这是最容易做错的面）。
2. **esc() 纪律**：app.js 57 处 innerHTML 动态插值仅 1 处例外（UI-012）；mdToHtml 先 esc 后做格式替换；渲染器全部 textContent/createElement 路径。
3. **dispose 纪律**（src/mang/ 三渲染器）：幂等、blob: URL 登记表统一 revoke、disposed 标志 + loadSeq 序号双守卫防异步竞态、样式表随根移除。
4. **pdf_view.ts teardownDocument**：IO 断开/renderTask cancel/textLayer cancel/缓存清空/loadingTask.destroy()——内存释放样板。
5. **wasm_boot.ts 单飞门**：WeakMap 按 glue 模块序列化全部 default() 调用、失败重置门可重试——双实例化 SEGV 根治（文件头有完整根因档案）。
6. **i18n 词典完备**：en/zh-CN 268 键双向零缺失 + Dict 类型编译锁；t() 未知键回退可见。
7. **tsconfig strict:true**。
8. **渲染防炸弹**：xlsx_view 上限矩阵（4096 条目/32MB/64MB/30 万格/250K tag reach）；docx_view ZIP 反炸弹第一道门；pdf_view 渲染像素上限注释 + LRU 64MB + bitmap.close() 驱逐；xlsx_grid MAX_WINDOW_CELLS 窗口钳制。
9. **serve.mjs 路径逃逸防护**（resolve + startsWith）。
10. **诚实的存根与降级**：saveActive 只读存根 toast、pdf display-only warn、bridge_absent typed 失败全家桶、grid 联动失败不回滚主编入且后缀缺席。

---

## 7. 任务重点问题 → 结论索引

| # | 重点问题 | 结论 |
|---|---|---|
| 1 | MCP 契约对账 | **零调用面**（§0/UI-002）——无废弃参数/响应漂移/E_* 处理可言；mcp.status 诊断面三字段对齐 |
| 2 | 状态一致性 §3.10 | 无 bid 域 UI（UI-002）；mang 自有面诚实（§2 表）；唯一观察项 UI-011 |
| 3 | collab_channel 幂等/重连/刷新 | 断线丢失（UI-003 CONFIRMED）+ 空 sv 歧义（UI-004 NEEDS_RUNTIME_VERIFY）；重连幂等（:396 已连接直接 return）✔；刷新=新会话+catch-up（受 UI-004 影响） |
| 4 | draft.ts 乐观并发 | N/A——localStorage 草稿与 project_version 无关（§0 推论） |
| 5 | 桌面宿主页 IPC 对齐 + postMessage 来源 | IPC 全对齐（§1）；来源校验三层弱化点（UI-009 + MANG-006 归 D-MANG） |
| 6 | i18n 键完备性 | 词典 268=268 零缺失 ✔；绕过 t() 的硬编码清单（UI-005）；mang 面单语（架构差异） |
| 7 | wasm 失败降级 | §4 降级矩阵——pkg_pdf display-only 为最佳实践；主核失败反馈弱（UI-005 附注） |
| 8 | 大文件渲染/内存释放 | 防炸弹上限矩阵 + 虚拟窗口钳制 ✔（正面清单 #8）；xlsx_grid 换簿无释放（UI-010）；pdf_view 样板（#4） |
| 9 | 构建配置 | tsconfig strict ✔；package.json↔lock 漂移 P1（UI-001）；build 脚本文档漂移（UI-008） |
| 10 | results_concurrent_web.json | 外部历史项目人工快照、不可复现、编码受损（UI-006）；E2E 工程缺口如实登记（UI-007） |

## 8. 覆盖统计

- git ls-files web-ui/ 共 156 文件：代码/配置 29 文件逐文件全读（~24,500 行）；i18n 键集脚本对账；package-lock 根段脚本对账；styles.css/qa-probe.css 覆盖性检查；mang-shots/ 149 PNG 覆盖性检查（截图 QA 存档证据）；.gitignore 覆盖性检查。
- 未执行（纪律遵守）：npm install / typecheck / build / 浏览器运行——UI-004 与 UI-010 的运行时面留待 Lead 统一执行或后续 NEEDS_RUNTIME_VERIFY 复核。
