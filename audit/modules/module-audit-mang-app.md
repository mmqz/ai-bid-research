# 模块审计报告：mang-app 桌面宿主（+ mang-snap）

> 全仓审计 V2 · 模块审计代理 D2（Task 4-d2）。
> 冻结基线：main@6105beb（分支 audit/v2-full-coverage，工作树 HEAD 含审计文档、生产代码与基线一致）。
> 审计方式：只读逐文件全读（git ls-files 全量 21 文件，6144 行）。未运行 cargo build/test/check（纪律禁令；Linux check 环境证据见 §5）。

## 0. 覆盖与结论总览

| 项 | 值 |
|---|---|
| 覆盖文件 | 20/20（mang-app 18：src/×15 + Cargo.toml + build.rs + .gitignore；mang-snap 2：lib.rs + Cargo.toml）——git ls-files 全量，无遗漏 |
| 逐行全读 | 是（含全部内联测试；mang-app src 合计 5561 行 + mang-snap 508 行） |
| P0 | 0 |
| P1 | 1（MANG-002：39 个内联测试 + --selftest 全部从未在任何 CI 执行） |
| P2 | 5（MANG-003/004/005/006/007） |
| P3 | 8（MANG-001/008/009/010/011/012/014/015）+ MANG-013（正面确认：不绕过 MCP）+ O-1/O-2/O-3/MANG-017 四项观察记录见 §3 末 |
| 头号问题裁定 | **桌面壳不绕过 MCP 做业务写**（详见 §1，证据充分）；但**直链 bidcore 的宣称有水分**（bid-domain 是死依赖） |

## 1. 头号问题：桌面端是否绕过 MCP 直调 bid-tools/bid-domain

**结论：业务读写不经壳内直调；壳是「显示 + .mang/** 本地文件管理」层。但 Cargo 直链 bidcore 的"方案 A"宣称与实际使用面不符（bid-domain 零使用）。**

### 1.1 证据链（全部 CONFIRMED）

1. **依赖声明**：`mang-app/Cargo.toml:14-20` — `bid-domain`、`bid-tools`、`bid-mcp` 均 path 直链，注释"直链 bidcore（方案 A：不经 MCP 层）"。
2. **实际使用面**（grep `bid_tools::|bid_domain::|bid_mcp::|use bid_` 全量）：
   - `main.rs:652` `use bid_tools::llm::{HttpTransport, LlmConfig, LlmTransport}` — 仅 llm.test 连接测试（HTTP POST，20s 上限）；
   - `diag.rs:69` `bid_mcp::tools_schema()`、`diag.rs:91` `bid_mcp::dispatch_json(r#"{"tool":"__mang_selftest__","args":{}}"#)` — **入参是写死的探针串**，仅验证"E_NOT_FOUND 应答=就绪"，WebView 无法借道执行任意 MCP 工具；
   - `diag.rs:118/124/215/226` `bid_tools::llm::LlmConfig::resolve` — 配置校验；
   - `diag.rs:250` `bid_tools::format_spec::FormatSpec` — 只读反序列化摘要。
   - **`bid_domain::` 全仓零命中** —— bid-domain 是纯死依赖（MANG-001）。
3. **无子进程桥接**：全模块无 `Command::new("bid-mcp-stdio")`；`diag.rs:4-6` 注释明示 stdio 服务由 AI 客户端拉起，"本壳不监听端口"。**IPC 与 MCP 的关系不是壳桥接，而是"两进程各写各的文件"**：AI 客户端 → bid-mcp-stdio 子进程写 db.json/chapters/…；壳 → 读工作区任意文件 + 写 `.mang/**`。
4. **壳的写面枚举**（ipc.rs:348-454 dispatch 全表）：`fs.writeBytes`（fsapi.rs:255-283，**仅 .mang/**，IPC-CONTRACT §5.2 同款纪律）、`fs.mkdir`（fsapi.rs:327-332，仅建目录）、`snapshots.save/check/rollback`（mang-snap，仅 .md/.txt/.markdown）、`annotations.setStatus`（仅 .mang/annotations 边车）、`llm.save`（仅 .mang/llm.json）、`preview.write`（仅 %TEMP%）、`recent.save`（仅 %APPDATA%）。**db.json 位于工作区根（bid-tools/src/store_io.rs:26 `DB_FILE="db.json"`、:81 "存储目录=工作区锚点"），不在 .mang/ 下 → 壳的任何写命令都够不到它**（`.mang/../db.json` 形态被 `paths::is_inside` 组件级比较拦截，见 fsapi.rs:259-266；但缺该形态负向测试 → MANG-008）。
5. **rollback 是唯一"越过 MCP 改正文"的通道**：`mang-snap/src/lib.rs:388-413` rollback 直接 `atomic_write(abs, content)` 覆盖工作区正文 .md（带回滚前备份）——这是 issue #42 设计内的文件级版本回滚，**不是**域状态机旁路（不触 db.json/状态/审核版本）。

### 1.2 对照 PA-002（issue #243：VALIDATING 死锁与恢复通道）

- **桌面端没有特权恢复通道——事实上它连普通恢复通道都没有**：壳不调 bid-domain 状态机、不能写 db.json、不重启/管理 bid-mcp-stdio 子进程。#243 描述的"杀 MCP 子进程 + 手改 db.json"恢复路径完全发生在壳外。
- 对 #243 的影响：桌面壳侧无需改动状态机语义；但若 #243 落地"受支持恢复路径"，**壳是天然的恢复 UI 候选位**（目前零接线——dispatch 表无任何 project/state/recovery 命令）。
- 附带确认：壳的 `fs.read` 可读 db.json（工作区内任意文件）→ UI 只读展示状态不破坏状态机，属设计内。

### 1.3 "直链 bidcore"宣称的真实含义（文档与实现不一致，P3）

`main.rs:5-6` 与 Cargo.toml 注释宣称"直链 bidcore（不经 MCP 层，方案 A）"；实际直链面= bid-tools 的 LLM 配置/传输 + FormatSpec serde + bid-mcp 的 schema/探针。**业务数据面（bid-domain 状态机/矩阵/证据）零直调**。这层"直链"只用于设置面板的自检与配置，不是业务旁路。审计口径应记：宣称 > 实际（死依赖 + 误导性注释）。

## 2. 专项审查结论（对应任务重点 2-8）

### 2.1 IPC（ipc.rs, 606 行）
- **消息边界**：parse（ipc.rs:127-141）拒非对象/缺 cmd；id 容忍 null；多余字段忽略（契约 §2 一致）。`dispatch_never_panics_on_garbage`（fsapi.rs:552-577）7 类畸形帧全回帧，不 panic。✅
- **注入安全**：`js_json`（ipc.rs:178-183）转义 `</`、U+2028/2029；resolve_script/event_script 全带守卫+try/catch（有测试 scripts_are_guarded）。✅
- **大响应**：INLINE_LIMIT 1.5MB → `%TEMP%\mang-ipc\<safe_id>.json`，safe_file_id 白名单+截断 120（ipc.rs:220-235）；take_pending 拒 `/`、`\`、`..`、非 .json（ipc.rs:263-275，有测试）。✅
- **超时**：壳侧无逐命令超时；`llm.test` 是唯一显式卸载到后台线程的命令（main.rs:549-565，注释明确"阻塞 IPC handler=冻住整个窗口"的教训）。**git.log/show/status、fs.tree、snapshots.check 在 UI 线程同步执行且 git 子进程无超时**（MANG-017，观察项 P3）。
- **类型转换**：mtime/ts/openedAt 统一 u64 毫秒（< 2^53，JS 安全）；`Args::u64` 拒浮点；ok_frame/err_frame 双键冗余（result/data、code+error）为兼容性设计。未发现丢字段转换。
- **丢帧**：outbox 上限 4096 丢最旧（main.rs:197-205）→ 洪泛下 JS promise 永不 resolve（壳侧无超时兜底，UI 侧 app.js 有 15-30s 超时）（MANG-010，P3）。

### 2.2 文件系统（fsapi.rs/paths.rs）
- **围栏**：`Workspace::resolve`（paths.rs:128-168）相对路径逐段词法回退（`..` 越根即拒）+ canonicalize_lenient（不存在时规范最近存在祖先再拼回）+ 组件级大小写不敏感 is_inside（防 `C:\ws2` 前缀陷阱，有测试 sibling_prefix_not_inside）。绝对路径同样过 canonical 后围栏。✅
- **符号链接**：读路径 canonicalize 后围栏 → 链接逃逸被拦（walk 里 symlink 条目直接跳过，fsapi.rs:155-157）；写路径 `.mang` 若为外指 symlink，canonical 解析后不在 root 内 → outside_workspace。**残余：resolve→write 间隙的 TOCTOU**（MANG-012，P3，与历史 T-07 同主题的 v1 残余）。
- **写纪律**：write_atomic tmp+rename（fsapi.rs:286-299）；`.mang/` 双围栏（root fence + mang fence）。唯一缺口=缺 `.mang/../db.json` 负向测试（MANG-008）。
- **上限**：TEXT 8MB / BYTES 48MB / WRITE 64MB / TREE 4000 节点+32 深。**tree 的 truncated 标志有一个精确耗尽边界 bug**：budget 恰在进入最后一个子目录时归零 → walk 顶部 `return Ok(Vec::new())` 不置 truncated（fsapi.rs:129-131 vs 158-160），若该目录又是父级最后条目，则静默少报且 truncated=false（观察项 O-1，P3，CONFIRMED by reading）。
- **recent.rs 损坏恢复**：recent.json 坏 JSON → eprintln+空表（recent.rs:32-35）；last-workspace 坏/空/失效目录 → None 回退 cwd（recent.rs:106-120，**有测试** last_workspace_roundtrip_and_staleness 覆盖三态）。✅（recent.json 自身损坏路径无测试，O-2）。
- **file.fromPath 展开 %VAR% 与 ~**（fsapi.rs:370-397）：环境变量展开只用于显示归一，不放大权限（后续仍走 metadata/描述）。✅

### 2.3 窗口生命周期（chrome.rs/main.rs）
- **退出落盘**：CloseRequested → ExitWithCode(0)；LoopDestroyed flush_outbox + purge_pending（main.rs:786-790）。壳自身无未保存状态（编辑保存由宿主/管线负责；30s 自动快照由 UI 驱动 snapshots.check）。✅
- **崩溃恢复**：所有落盘原子写（tmp+rename）；无崩溃日志/日志面。可接受。
- **多实例**：**无单实例锁**（全模块无 CreateMutex/命名互斥/锁文件）→ MANG-004（跨进程 index.json 丢更新）+ MANG-005（第二实例启动 purge_pending 删掉第一实例未取的大响应 → UI 404 gone）。
- **chrome.rs（883 行，Windows-only）**：SetWindowSubclass 双子类（主窗口+WebView 层）、WM_NCHITTEST/WM_NCCALCSIZE、自管最大化状态位 IS_MAXED（不用 IsZoomed）。带一条宝贵的回归测试（`_ => false` 通配符曾吃掉 drag/toggleMaximize 两分支——chrome.rs:847-880 注释+测试双证据）。unsafe 面大但均为标准 Win32 消息处理；`try_dispatch` 里 SendMessage(WM_NCLBUTTONDOWN) 自拖动模式正确。

### 2.4 快照/拖放/对话框/设置/日志/诊断
- **敏感信息**：`llm.status` api_key 只出掩码（diag.rs:44-50 mask_key；**有测试** llm_status_masks_key_when_present 断言"明文绝不外泄"）；llm_save 成功不回显 key。日志（eprintln/println）只打路径与错误摘要，不打 token。诊断数据（mcp.status/llm.status/fspec.status）**纯本地，无外发面**。✅
- **但**：`.mang/llm.json` 明文存 key（bid-tools llm.rs:54）且 `fs.read` 可读全文 + serve 无 CSP + IPC 无来源校验 → 单点 XSS 即可读明文 key（详见 MANG-006）。当前 UI 侧 esc() 纪律到位（app.js:4-6 全量转义 + main.ts:1817 "导入文件字节永不进 innerHTML" 纪律注记，grep 无污染 sink），故为纵深缺口。
- **拖放**（dnd.rs）：WM_DROPFILES 子类 + DragFinish（防泄漏）；事件只含路径，UI 侧自判 insideWorkspace。✅
- **对话框**（dialogs.rs）：主线程模态 + MANG_DIALOG_MOCK 无头门（有测试 mock_mode_never_blocks）；rfd default-features=false → Linux 无后端（见 §5 平台盲区）。
- **快照**：mang-snap 单一真源声明属实——bid-mcp save_version（tools.rs:1796-1813）与桌面 IPC 确实共用同一 `mang_snap::save`。剪枝（200/文件，auto 优先）、同毫秒序号防互覆、8MB 上限、文本白名单。**跨进程锁缺口见 MANG-004**。

### 2.5 平台分支盲区（重点 6）
- `#[cfg(windows)]` 整块：chrome.rs `mod imp`（~800 行）、dnd.rs `mod imp`（~90 行）、main.rs screen_work_area/open_external(windows 分支)/DnD 挂载块、shell.rs reveal_path(windows)。**这些代码在 Linux check 下根本不编译**（stub 恒 false/空）。
- `#[cfg(not(windows))]`：initial_size 1440x900、chrome/dnd 四个 stub、shell xdg-open 分支——同样无编译验证（见 §5）。
- 平台中立可编译面：ipc/paths/fsapi/recent/gitlog/diag/snapshots/annotations/b64/selftest/dialogs(编译但 rfd 无后端)。
- chrome.rs 的 1 条回归测试带 `#[cfg(windows)]` 内层断言 + 非 Windows 恒真断言——**在 CI（windows-latest）上其实会跑**——但 CI 只 `cargo check --all-targets` 不 `cargo test`，所以也没跑（MANG-002）。

### 2.6 mang-snap（重点 7）
- 职责：`.mang/history` **写端单一真源**（save/check/read/rollback + file_key + find_root）；消费方=mang-app/src/snapshots.rs、annotations.rs（file_key）+ bid-mcp/src/tools.rs save_version。bidcore workspace 成员（bidcore/Cargo.toml:11）→ **其 4 个内联测试在 CI cargo test 全量 workspace 中会跑**（与 mang-app 相反）。
- 风险：进程内 INDEX_LOCK（lib.rs:33）不跨进程（MANG-004）；atomic_write 的 remove-then-rename（lib.rs:214-217）在 Rust std rename（Windows MoveFileEx REPLACE_EXISTING）语义下 remove 是多余且**加宽了崩溃窗口**（观察项 O-3，P3；annotations.rs:49-60 同款）。read/rollback 的"snapshot 必须登记在 index 且属于该 file"防穿越校验在位（lib.rs:373-385/398-405，有测试 ai_kind_survives_roundtrip 的越权读断言）。

### 2.7 selftest.rs（重点 8）：能否替代 CI 测试
- 覆盖（约 24 个检查）：ping/app.info/fs.tree×2/fs.read/fs.readBytes/大响应落盘+取回+删除/fs.writeBytes×3（ok+denied+坏b64）/fs.stat×2/fs.mkdir/路径围栏 5 例/recent 往返/history.list×2/unknown_cmd/bad_json/purge_pending/mcp.status/llm.status→save→status×5/fspec.status×2/workspace.open×6（切换/幂等/记忆/失败不换根）。
- **不覆盖**：snapshots.×4、annotations.×2、git.×3、preview.write、file.fromPath、explorer.reveal、dialog.×2、llm.test、window.×3——**issue #42 新增的快照/标注 IPC 面与 #76 的 git 面在 selftest 和内联测试双双为零**（MANG-003）。
- 裁定：selftest 是**运行时冒烟**（真实进程、真实临时盘），与 #[test] 单元测试互补，**不能替代**：① 它不进 CI（MANG-002 的另一半）；② 它不跑 Windows-only 模块（chrome/dnd/dialogs 真窗）；③ 它对负向安全面的覆盖（围栏 5 例）好于多数单元测试，但快照/标注/git 的负向面完全空白。
- 手工冒烟 `scripts/mang-dev/mang_ipc_smoke.mjs`（真实 WebView2 + CDP）存在但纯手工。

## 3. 问题记录

---

ID: MANG-001
严重程度: P3
模块/文件路径: bidcore/crates/mang-app/Cargo.toml
代码行号: 14-16
函数或结构体: [dependencies]
问题描述: bid-domain 为死依赖：全模块 `bid_domain::`/`use bid_domain` 零命中；"直链 bidcore（方案 A）"的宣称实际只覆盖 bid-tools(llm/format_spec) 与 bid-mcp(schema/探针)。
调用链: Cargo.toml 声明 → 无任何 use。
触发条件: 静态事实。
实际影响: 无运行时影响；编译面浪费 + 审计/文档误导（"不经 MCP 层"易被误读为业务旁路）。
复现步骤: `rg bid_domain bidcore/crates/mang-app/src` → 0 命中。
当前测试: 无。
缺失测试: N/A（依赖清理项）。
建议方案: 删除 bid-domain 依赖或改注释为"设置面板自检直链 bid-tools/bid-mcp"。
证据:
```toml
# 直链 bidcore（方案 A：不经 MCP 层）
bid-domain = { path = "../bid-domain" }
bid-tools = { path = "../bid-tools" }
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: MANG-002
严重程度: P1
模块/文件路径: .github/workflows/windows-check.yml + bidcore/crates/mang-app/src/*.rs
代码行号: workflow:93-95（`cargo check (mang-app 独立 workspace)`，仅 check）；各模块 #[cfg(test)] 合计 39 个 #[test]
函数或结构体: CI 门禁面
问题描述: mang-app 的 39 个内联单元测试与 --selftest 运行时冒烟**从未在任何 CI 执行**：CI 只有 `cargo check --all-targets`（编译测试但不运行）；bidcore 根的 `cargo test --workspace` 不含 mang-app（独立 [workspace]）；--selftest 不进任何 workflow。gitlog.rs/selftest.rs/snapshots.rs 三文件 0 测试。
调用链: CI → cargo check --all-targets（编译过即绿）。
触发条件: 任何测试断言失效都不会被发现（假绿结构性风险——与 AUDIT-CI-001 同类）。
实际影响: 桌面壳全部行为验证靠 Lead 手验（workflow 注释自认"只靠 Lead 手验"）；路径围栏、b64、llm 掩码等安全断言形同虚设。
复现步骤: 读 windows-check.yml 93-95 行；rg -c '#\[test\]' mang-app/src/*.rs。
当前测试: 存在但零执行。
缺失测试: CI 接线（cargo test -p mang-app + mang-app --selftest 步骤）。
建议方案: workflow 增加两步：`cargo test --workspace`（mang-app 目录内）与 `target/release/mang-app --selftest`（MANG_DIALOG_MOCK=1）。
证据:
```yaml
      - name: cargo check (mang-app 独立 workspace)
        working-directory: bidcore/crates/mang-app
        run: cargo check --all-targets
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: MANG-003
严重程度: P2
模块/文件路径: bidcore/crates/mang-app/src/selftest.rs + src/snapshots.rs
代码行号: selftest.rs:1-8（头注覆盖清单）；snapshots.rs 全文件无 #[cfg(test)]
函数或结构体: selftest::run / snapshots::{check,save,read,rollback}
问题描述: issue #42 新增的 IPC 面（snapshots.×4、annotations.×2）与 git 面（git.log/show/status）在 selftest 覆盖清单与内联测试**双双为零**；preview.write/file.fromPath/explorer.reveal/dialog.*/llm.test/window.* 同样不在 selftest。snapshots.rs 作为 IPC 适配层（含回滚安全序：先备份后覆盖）无任何自动化测试。
调用链: UI(app.js:1213/1536/2687 调 snapshots.check/save/rollback) → ipc.rs:423-429 → snapshots.rs → mang-snap。
触发条件: 任何快照 IPC 适配回归（参数解包/围栏/rel 规范化）无门禁拦截。
实际影响: 版本历史/回滚（用户数据保护面）回归风险无自动化防线；mang-snap 核心有 4 测试在 bidcore CI 跑，但适配层语义（如 rollback 的 resolve_file 白名单）无覆盖。
复现步骤: rg 'snapshots\.|annotations\.|git\.' selftest.rs → 0。
当前测试: mang-snap 核心测试 4 个（跑）；适配层 0。
缺失测试: selftest 增加快照存/读/回滚/越权读四检查 + annotations 三态检查；或 snapshots.rs 补内联测试。
建议方案: 扩 selftest 清单（成本最低，且 --selftest 已能无头跑）。
证据:
```rust
//! 覆盖：ping / app.info / fs.tree / fs.read / fs.readBytes / fs.writeBytes(happy+denied)
//!       / fs.stat / fs.mkdir / path-guard(../ 与绝对越界) / 大响应落盘与取回 / recent 往返
//!       / history.list / unknown_cmd / bad_json
```
（清单无 snapshots/annotations/git/preview/file.fromPath）
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: MANG-004
严重程度: P2
模块/文件路径: bidcore/crates/mang-snap/src/lib.rs（+ mang-app/src/recent.rs 同型）
代码行号: lib.rs:33 `static INDEX_LOCK: Mutex<()> = Mutex::new(());`；save_locked lib.rs:248-280（load_index→push→atomic_write 全在进程内锁下）
函数或结构体: INDEX_LOCK / save_locked / check
问题描述: `.mang/history/index.json` 的读-改-写只被**进程内** Mutex 串行化；mang-app（30s 自动快照）与 bid-mcp-stdio（AI save_version，**独立进程**）以及可能的双开实例之间无文件锁/锁文件 → 跨进程丢更新：A 读 N 条 → B 写 N+1 → A 写 N+1' 覆盖 B 的登记（B 的快照正文成孤儿，版本条目丢失）。
调用链: mang-app UI 30s tick → ipc snapshots.check → mang_snap::check（持进程内锁 RMW）‖ AI 客户端 → bid-mcp-stdio → save_version → mang_snap::save（另一进程的同一把"锁"失效）。
触发条件: 桌面壳开着的同时 AI 管线存版本（设计内的常态并发）。
实际影响: 版本历史条目丢失（不撕裂文件——原子写在位；孤儿 .md 不清理但 prune 按 index 走，孤儿无害滞留）。
复现步骤: 两进程循环调用 mang_snap::save 同一 root（如 mang-app 开 + 手动 bid-mcp save_version 脚本）→ 对比 index.json 条目数与写入次数。
当前测试: 单进程测试（check_saves_on_change…）无跨进程面。
缺失测试: 跨进程并发写 index 一致性测试。
建议方案: index.json 写入改为追加式 JSONL 或引入锁文件（fs2/LockFileEx）；至少把 INDEX_LOCK 升级为跨进程文件锁。
证据:
```rust
static INDEX_LOCK: Mutex<()> = Mutex::new(());
...
let _guard = INDEX_LOCK.lock().unwrap_or_else(|g| g.into_inner());
let mut items = load_index(root);   // 读
items.push(json!({ ... }));          // 改
atomic_write(&hroot.join("index.json"), ...)?;  // 写——锁只在本进程有效
```
审计置信度: CONFIRMED（竞态窗口为静态推演，丢失概率属 NEEDS_RUNTIME_VERIFY，但锁的类型学错误是确定的）
状态分类: 当前仍存在

---

ID: MANG-005
严重程度: P2
模块/文件路径: bidcore/crates/mang-app/src/main.rs + src/ipc.rs
代码行号: main.rs:317 `ipc::purge_pending()`（启动即清）；main.rs 无单实例锁；ipc.rs:278-287 purge_pending 删除目录下全部 .json
函数或结构体: main() / purge_pending()
问题描述: 无单实例机制（无命名互斥体/锁文件/现有实例激活）；双开同一工作区时：① 第二实例启动 purge_pending 会删除**第一实例尚未 fetch 的大响应临时文件**（%TEMP%\mang-ipc 共享）→ 第一实例 UI 取 /__ipc/<id>.json 得 404 "gone"；② 放大 MANG-004 跨进程竞态。
调用链: 用户双开 → 第二实例 main() → purge_pending() → 第一实例 pending 文件被删。
触发条件: 双开（无任何拦截）。
实际影响: 大响应丢失（UI 报错）；index.json 丢更新概率放大。
复现步骤: 实例 A 打开工作区 → 触发 >1.5MB fs.readBytes → A fetch 前启动实例 B → A 的 fetch 404。
当前测试: spill_and_take（单进程一次即删语义）有；多实例无。
缺失测试: N/A（需运行时）。
建议方案: 启动时创建 %TEMP%\mang-ipc\<pid> 分桶；或命名互斥体单实例 + 二次激活转发。
证据:
```rust
    // 清理上次运行遗留的 IPC 临时文件
    ipc::purge_pending();
```
审计置信度: CONFIRMED（purge 全删行为）；触发链 NEEDS_RUNTIME_VERIFY
状态分类: 当前仍存在

---

ID: MANG-006
严重程度: P2
模块/文件路径: bidcore/crates/mang-app/src/main.rs + src/fsapi.rs + src/diag.rs
代码行号: main.rs:860-868 respond()（ACAO `*`、无 CSP 头）；main.rs:500-502 ipc handler 无来源/令牌校验；fsapi.rs:195-222 fs.read 无 .mang 排除；diag.rs:148-236 llm_save 合并语义；main.rs:651-708 run_llm_test
函数或结构体: respond / handle_ipc / read / llm_save / run_llm_test
问题描述: 三层纵深缺失叠加：① custom protocol 响应无 Content-Security-Policy（仅 ACAO * + no-store）；② IPC 桥（window.ipc.postMessage）无来源/能力校验；③ `.mang/llm.json` 明文存 api_key 且 fs.read 可整读。一旦 UI 出现任何 XSS（当前 app.js esc() 纪律完好、无 innerHTML 污点 sink，故非现行漏洞），单条脚本即可：fs.read `.mang/llm.json` 直接拿明文 key；或 llm.save 只改 apiBase（合并语义保留原 key）→ llm.test 把真实 Bearer key POST 到任意外部 URL（二通道 exfil + 200 字响应回显）。blob:/data:/about: 均在导航白名单（main.rs:484-490），wry initialization script 是否注入到跨源/file: iframe 及其 ipc 可达性未验证。
调用链: UI XSS → window.ipc.postMessage({cmd:"fs.read",path:".mang/llm.json"}) → 明文 key → fetch 外发（WebView 有网络栈）。
触发条件: UI 渲染层出现单一 HTML 注入点（当前未发现，但 web-ui 无 CSP meta 兜底）。
实际影响: LLM api_key 泄露 + .mang/** 任意写（覆盖 llm.json/history/标注边车）。
复现步骤: DevTools 里对页面执行 `window.ipc.postMessage(JSON.stringify({id:"x",cmd:"fs.read",path:".mang/llm.json"}))` 观察回帧含明文 key。
当前测试: llm.status 掩码有测试（防的是 status 回显，不防 fs.read 直读）。
缺失测试: 无 CSP 存在性断言；无"fs.read 不回 api_key"类约束（设计上 fs.read 就该读文件——问题在叠加面）。
建议方案: serve() 加 `Content-Security-Policy: default-src 'self'; connect-src 'self'`（注意 pdf.js worker/wasm 需求）；llm.json 的 api_key 考虑 DPAPI 或至少将 fs.read 对 .mang/llm.json 做脱敏豁免；ipc handler 校验 frame origin。
证据:
```rust
fn respond(status: u16, mime: &str, bytes: Vec<u8>) -> ... {
    wry::http::Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-store")
        // ← 无 Content-Security-Policy
```
审计置信度: 缺陷组合 CONFIRMED（静态）；可利用性 NEEDS_RUNTIME_VERIFY（依赖 wry iframe 行为 + UI 未来 XSS）
状态分类: 当前仍存在

---

ID: MANG-007
严重程度: P2
模块/文件路径: bidcore/crates/mang-app/src/gitlog.rs
代码行号: gitlog.rs:22-40 run_git（仅 `-c core.quotepath=off`）；:66-76 git log 参数；:113 git show
函数或结构体: run_git / log / show / status
问题描述: 对不可信工作区（招标项目常以 zip 分发、自带 .git）执行 git 子命令未做 untrusted-repo 加固：repo 内 .git/config 可设 `log.showsignature=true` + `gpg.program=<path>`（git log 会执行签名验证程序）、`.gitattributes` + `diff.<driver>.textconv` 在 `git show`（默认带 patch 渲染）路径可执行过滤程序；且无 GIT_CONFIG_NOSYSTEM、无子进程超时（UI 线程同步等待）。git 官方文档明示不要对不受信仓库运行 git（此为已知攻击面类别，VSCode 等以 `-c core.fsmonitor=false -c core.hooksPath=` 等缓解）。
调用链: UI 改动/历史面板自动刷新 → ipc git.log/show → run_git（cwd=工作区根，读工作区 .git/config）。
触发条件: 用户打开带恶意 .git 的投标材料目录（壳的 workspace 概念就是任意目录）。
实际影响: 潜在任意命令执行（RCE 级），或病态仓库把 git log 拖到分钟级冻结 UI（无超时）。
复现步骤: 工作区 .git/config 写 `[log] showsignature = true` + `[gpg] program = calc.exe` → 壳打开 git 面板。
当前测试: gitlog.rs 0 测试。
缺失测试: 子进程超时/加固参数回归。
建议方案: run_git 统一追加 `-c log.showsignature=false -c diff.external= -c core.fsmonitor=false -c core.hooksPath=`（或 GIT_CONFIG 参数注入），并给子进程加超时。
证据:
```rust
fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-c")
        .arg("core.quotepath=off")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()   // ← 同步无超时；repo config 全部生效
```
审计置信度: NEEDS_RUNTIME_VERIFY（攻击向量属 git 已知行为，具体触发需实机验证）
状态分类: 当前仍存在

---

ID: MANG-008
严重程度: P3
模块/文件路径: bidcore/crates/mang-app/src/fsapi.rs
代码行号: fsapi.rs:255-266（双围栏实现）；tests:512-539 write_bytes_guard
函数或结构体: write_bytes / write_bytes_guard(测试)
问题描述: 缺 `.mang/../db.json`、`.mang/x/../../db.json` 形态的负向测试——现有测试只覆盖根围栏（../evil.txt → outside_workspace）与非 .mang（denied）；"借 .mang 前缀再回退到根"的攻击形态未被测试锁定（实现当前安全：resolve 词法回退后 is_inside(组件级) 对 .mang 前缀判否）。
调用链: —
触发条件: 未来重构 resolve/write_bytes 时回归。
实际影响: 测试缺口（防回归锁缺失），当前无漏洞。
复现步骤: dispatch `{"cmd":"fs.writeBytes","path":".mang/../db.json",...}` → 期望 denied/outside_workspace（无测试断言）。
当前测试: write_bytes_guard（3 例）。
缺失测试: .mang 前缀回退形态负向。
建议方案: 补 2 条断言。
证据:
```rust
    if !paths::is_inside(&p, &mang_canon) {
        return Err(IpcError::denied(format!(
            "fs.writeBytes 只允许写工作区内的 .mang/：{raw}"
        )));
    }
```
审计置信度: CONFIRMED（缺口）；实现安全性 CONFIRMED
状态分类: 当前仍存在

---

ID: MANG-009
严重程度: P3
模块/文件路径: docs/mang/IPC-CONTRACT.md
代码行号: §6 命令表（159-363）；§3.3 错误码表（96-108）
函数或结构体: —
问题描述: 自称"唯一权威"的契约文档落后实现 4 个命令族：git.log/show/status（ipc.rs:403-418）、snapshots.check/save/read/rollback（:423-429）、annotations.load/setStatus（:432-434）、window.info/action/buttons（main.rs:540-545）；错误码表缺 mang-snap 透传的 `unsupported`/`empty`（ipc.rs:447-450 的 dialog 占位错误也用 io 码——与契约"对话框必须在主线程"无码位对应）。llm.test 命令本身有文档（§6 有 llm.test 段）。
触发条件: 文档对账（R1 轮）。
实际影响: 契约-实现漂移；外部对接方按文档开发会漏 4 个命令族。
建议方案: 回填 §6（四族 + 两错误码）。
证据: ipc.rs dispatch 表 vs 契约 §6 逐条比对（本文 §2.1）。
审计置信度: CONFIRMED
状态分类: 文档与实现不一致

---

ID: MANG-010
严重程度: P3
模块/文件路径: bidcore/crates/mang-app/src/main.rs
代码行号: main.rs:197-205 outbox_push（4096 上限丢最旧）；:215-220 drop_queue_push（32 静默丢）
函数或结构体: outbox_push / drop_queue_push
问题描述: 注入队列超限时丢最旧帧且只 eprintln——被丢的若是 resolve 帧，JS 侧 promise 无壳侧超时兜底（UI app.js 侧各命令有 15-30s 超时，可自愈为报错）；拖放事件 >32 静默丢弃。
触发条件: 极端洪泛（大量并发 IPC + 大响应）。
实际影响: 低概率下 UI 卡 promise 至 UI 超时。
建议方案: 丢帧时向 JS 注入一条 error 帧（id 可从被丢脚本解析）。
证据:
```rust
    if q.len() > 4096 {
        eprintln!("[mang-ipc] 注入队列过长，丢弃最旧的 {}", q.len() - 4096);
        let drop_n = q.len() - 4096;
        q.drain(..drop_n);
    }
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: MANG-011
严重程度: P3
模块/文件路径: bidcore/crates/mang-app/src/fsapi.rs + src/ipc.rs
代码行号: fsapi.rs:399-437 preview_write；ipc.rs:278-287 purge_pending（只清 mang-ipc 的 .json）
函数或结构体: preview_write / purge_pending
问题描述: %TEMP%\mang-preview 每次预览新建 `<uuid>-<name>`（上限 48MB/次）**无任何清理**（启动/退出都不清，只清 mang-ipc）→ 临时目录无界增长，依赖 OS 清理策略。
触发条件: 长期使用 + 大文件预览。
实际影响: 磁盘占用；预览文件含工作区内容副本（同用户可读，本地残留敏感面）。
建议方案: purge_pending 扩展到 mang-preview（按 mtime 龄期清）。
证据:
```rust
pub fn purge_pending() {
    let dir = pending_dir();   // 只有 mang-ipc
    ...
}
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: MANG-012
严重程度: P3
模块/文件路径: bidcore/crates/mang-app/src/fsapi.rs + src/paths.rs
代码行号: fsapi.rs:259-281（resolve → create_dir_all → write_atomic 的间隙）
函数或结构体: write_bytes
问题描述: 写路径为 check-then-use：resolve 校验（含 symlink 解析）与实际 write 之间存在 TOCTOU 窗口（.mang 或其父段被换为符号链接）。Windows 同用户创建符号链接需开发者模式/特权，攻击前置条件强；与历史 T-07（fs_fence check-then-use v1 残余）同主题。
触发条件: 本地同用户主动竞态。
实际影响: 理论上可把写入导向 .mang 围栏外（root 围栏仍第二次拦截 root 级逃逸——不，resolve 已完成，write 直接写 p——若 p 被换链则写出 root 之外）。
建议方案: openat 风格（Windows 用 NtCreateFile 相对打开）或写后 verify（canonicalize(p)==p）。登记为 v1 残余即可。
审计置信度: SUSPECTED（需真实环境验证）
状态分类: 尚无法复现

---

ID: MANG-014
严重程度: P3
模块/文件路径: bidcore/crates/mang-app/src/main.rs
代码行号: main.rs:836-847（不存在路径回退 index.html）；:828-834 /__ipc 段 + respond ACAO *
函数或结构体: serve
问题描述: ① 资产 404 被吞成 index.html 200（SPA 友好但调试不友好，契约 §1 有记录——按契约实现，属设计权衡）；② `Access-Control-Allow-Origin: *` 使**任何内嵌于 WebView 的远程 http(s) iframe**可跨源读 `http://mang.localhost/__ipc/<id>.json` 大响应（id 为 JS 侧 "r"+seq 可预测序列）——当前 UI 不嵌远程 iframe（预览 iframe 走 blob:/file:），属条件性窃读窗口。
建议方案: ACAO 收敛为同源（`http://mang.localhost`）或去掉 ACAO（同源 fetch 不需要）。
证据:
```rust
        .header("Access-Control-Allow-Origin", "*")
```
审计置信度: 静态 CONFIRMED；利用链 NEEDS_RUNTIME_VERIFY
状态分类: 当前仍存在

---

ID: MANG-015
严重程度: P3
模块/文件路径: bidcore/crates/mang-app/Cargo.toml + src/dialogs.rs + src/chrome.rs + src/dnd.rs
代码行号: Cargo.toml:27（rfd default-features=false）；chrome.rs:829-846 / dnd.rs:105-108（not(windows) stub）
函数或结构体: 平台分支面
问题描述: 非 Windows 分支从未被编译验证：① rfd 关默认 feature 后 Linux 无对话框后端（pick_files 恒 Err→空，功能性缺失但可编译）；② chrome/dnd 非 Windows stub、initial_size 1440x900、xdg-open 分支零编译证据（Lead 的 Linux check 因环境问题未达编译阶段，见 §5）；③ chrome.rs 的 window_action 回归测试在非 Windows 上退化为恒真断言（测试注释自认）。
实际影响: 若未来交付 Linux/macOS 构建，当前为零验证状态；Windows-only 逻辑（~900 行）只能靠 Windows CI 编译 + 手验。
建议方案: 接受"Windows 单一交付面"现状并把此记录为正式风险登记（移交 Lead）。
审计置信度: CONFIRMED
状态分类: 当前仍存在（平台盲区）

---

ID: MANG-013
严重程度: P3（信息确认，非缺陷）
模块/文件路径: bidcore/crates/mang-app/src/{main.rs,ipc.rs,diag.rs,fsapi.rs} + bidcore/crates/bid-tools/src/store_io.rs
代码行号: 见 §1.1 证据链（main.rs:5-6/500-502/652；diag.rs:69/91/118/124/215/226/250；fsapi.rs:255-266；store_io.rs:26/81）
函数或结构体: 全模块直调面核查
问题描述: （正面确认，登记为可引用结论）桌面壳**不绕过 MCP 做业务写**：无 bid-domain 调用（死依赖）、无 bid-mcp-stdio 子进程、dispatch_json 仅固定探针串；壳写面=.mang/**（fs.writeBytes/snapshots/annotations/llm.save）+ 文本级 rollback；db.json 在工作区根而非 .mang → 双围栏成立。
调用链: 见 §1.1。
触发条件: N/A。
实际影响: PA-002/#243 视角——桌面端无任何 VALIDATING 恢复通道（特权与否都没有），#243 的"受支持恢复路径"在壳侧零接线；壳亦未提供绕过域层手改 db.json 的捷径。
复现步骤: rg 'bid_domain::|bid_engine::' mang-app/src → 0；rg 'Command::new' mang-app/src → 仅 gitlog/git、explorer、rundll32、xdg-open。
当前测试: selftest 的 mcp.status 探针 + fs.writeBytes denied/outside 三例。
缺失测试: 无诉求。
建议方案: #243 落地时把壳作为恢复 UI 候选位接线。
证据: 见 §1.1 五条证据链。
审计置信度: CONFIRMED
状态分类: 文档与实现不一致（"直链 bidcore 方案 A"宣称大于实际使用面——业务面零直调，详见 MANG-001）

---

### 观察项（不占 MANG 编号，无整改诉求或纯注记）

- **O-1（P3）**：fsapi.rs walk() 的 truncated 标志精确耗尽边界——budget 归零恰好发生在进入"父级最后条目"的子目录时，walk 顶部早退不置 truncated → 树静默少报且 truncated=false（fsapi.rs:129-131 与 158-160 的不对称）。CONFIRMED by reading。
- **O-2（P3）**：recent.json 自身损坏 → 空表恢复路径无测试（last-workspace 的损坏三态有测试；同类面不对齐）。
- **O-3（P3）**：mang-snap/annotations 的 atomic_write 做 remove-then-rename（lib.rs:214-217 注释称"Windows rename 目标存在时失败"）——Rust std rename 在 Windows 用 MOVEFILE_REPLACE_EXISTING，可覆盖已存在目标，remove 步骤多余且加宽崩溃窗口（丢整文件 vs 丢旧版本）。CONFIRMED by std 语义。
- **MANG-017（观察 P3）**：git 子进程无超时 + git.log/show 在 UI 线程同步 dispatch（病态仓库可长时间冻窗）。与 MANG-007 同文件，建议一并修（超时杀进程）。
- **正面确认**：llm.test 后台线程化（main.rs:549-565，有教训注释）；workspace.open 写锁前置于读锁防死锁（ipc.rs:349-364）；chrome.rs 通配符回归测试（教训入测试）；b64 手写实现 0..1000 全长度往返 + 已知向量 + 拒垃圾测试；outbox/drop_q 锁毒化 `into_inner` 一致口径；navigation handler 拒绝非白名单协议并把 http(s) 交系统浏览器（main.rs:479-499）。

## 4. 与历史/全局审计线索的交叉引用

| 线索 | 本轮结论 |
|---|---|
| PA-002 / issue #243（VALIDATING 死锁） | 桌面壳无任何状态机触点、无 db.json 写通道、无恢复命令——#243 的"受支持恢复路径"在壳侧零存在；壳亦未提供"绕过域层手改 db.json"的捷径（fence 成立）。见 §1.2。 |
| AUDIT-CI-001（CI 覆盖缺口） | 在桌面壳的具体化即 MANG-002：check-only、测试零执行。 |
| T-07（TOCTOU 残余） | MANG-012 为同类 v1 残余（桌面侧）。 |
| T-13（日志脱敏） | 壳侧达标（掩码+测试），但明文落盘 + fs.read 叠加面见 MANG-006。 |
| mang-snap 单一真源宣称 | 属实（bid-mcp tools.rs:1796-1813 共用）；跨进程锁缺口 MANG-004。 |
| #194 工作区切换 | 实现与 5 个测试 + selftest 6 检查对齐（切换/幂等/记忆/失败不换根），质量良好。 |
| #76 设置面板 | mcp/llm/fspec 三面实现与契约一致；llm 掩码有测试。 |

## 5. Lead Linux check 结果（平台差异证据）

`/home/z/my-project/logs/07-mang-app-check.log`（CHECK-EXIT:101）：**未达到编译阶段**——失败于依赖解析：`failed to get rfd ... unable to update registry crates-io ... error in environment variable CARGO_NET_GIT_FETCH_WITH_CLI: provided string was not true or false`。即：失败原因是隔离副本环境的 `CARGO_NET_GIT_FETCH_WITH_CLI` 值非法（应为 true/false），属**环境配置差异**而非代码/平台编译错误；该日志不能作为"Linux 编译失败=平台差异"的代码级证据（未编译任何 .rs）。结合任务背景（Linux 缺 gtk/webkit 系统依赖），mang-app 在 Linux 的真实可编译性**当前无任何有效证据**（见 MANG-015）。

## 6. 审计方法与限制

- 逐文件全读：21/21；行号引用以当前工作树为准（与冻结基线生产代码一致）。
- 未运行任何构建/测试（纪律）；所有"CONFIRMED"均为静态代码证据；运行时行为类（wry iframe ipc 可达性、git config RCE 链、TOCTOU、多实例竞态实测）标 NEEDS_RUNTIME_VERIFY/SUSPECTED。
- 交叉仓只读核对：bid-tools/src/{llm.rs,store_io.rs}、bid-mcp/src/{lib.rs,tools.rs,protocol.rs}、bidcore/Cargo.toml、.github/workflows/windows-check.yml、web-ui/src/main.ts 与 web-ui/mang/app.js（仅为 CSP/XSS 面取证，未深审 web-ui——属其他模块代理范围）。

## 7. 修复优先级建议（供 Lead 汇总）

1. **P1 MANG-002**：CI 接 `cargo test`（mang-app 目录）+ `--selftest`——半天工作量，立即让 39+24 个既有断言复活。
2. **P2 MANG-003**：selftest 补 snapshots/annotations/git/preview 检查（复用既有 scratch_dir 夹具）。
3. **P2 MANG-004**：mang-snap 跨进程锁（或 index 追加式）——数据丢失面。
4. **P2 MANG-005/006/007**：单实例锁 + CSP/ACAO 收敛 + git 加固与超时——三个独立小改动。
5. P3 各项随批次清理；MANG-009 文档回填随 R1 轮。
