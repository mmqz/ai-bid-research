# mang IPC 契约 v1

> 实现：`bidcore/crates/mang-app/`（tao + wry / WebView2）
> 本文是**唯一权威**：字段名、单位、错误码以这里为准。改了代码要同步改这里。

---

## 0. 一句话

UI 页（`dist/mang/index.html`）通过 `window.ipc.postMessage(JSON.stringify(req))` 发请求；壳把结果以**注入脚本**的形式注入回页面：`window.__ipcResolve(id, resp, ok)`；事件走 `window.__ipcEvent(ev)`。

```
UI 页 (mang://localhost/mang/index.html)
   │  window.ipc.postMessage('{"id":"r1","cmd":"fs.read","path":"a.md"}')
   ▼
wry with_ipc_handler  ──►  ipc::parse → Ctx::dispatch → 响应帧 JSON
   │
   ▼  evaluate_script
try{window.__ipcResolve&&window.__ipcResolve("r1", {…}, true);}catch(e){}
```

---

## 1. 加载与 URL

壳用 `WebViewBuilder::with_url("mang://localhost/mang/index.html")`，custom protocol `mang` 的根是 **UI 目录**（默认 `web-ui/dist`）。

| 请求路径 | 文件 |
|---|---|
| `mang://localhost/mang/index.html` | `<ui>/mang/index.html` |
| `mang://localhost/mang/app.js` | `<ui>/mang/app.js` |
| `mang://localhost/mang/pdf.worker.min.mjs` | `<ui>/mang/pdf.worker.min.mjs` |
| `mang://localhost/__ipc/<id>.json` | 大响应临时文件（见 §4） |

### ⚠ Windows 上的协议改写（**最容易踩的坑**）

wry 在 Windows 上把自定义协议改写成 http 形式（WebView2 限制）：

```
mang://localhost/mang/index.html  →  http://mang.localhost/mang/index.html
```

所以页面里 **`location.origin === "http://mang.localhost"`**。由此两条规则：

1. **页面内 fetch 大响应必须用相对路径**：`fetch('/__ipc/' + file)`。
   `fetch('mang://localhost/__ipc/…')` 会 `TypeError: Failed to fetch`（不会被拦截）。
2. **导航白名单必须同时放行两种形态**，否则主页面会被自己拦掉、WebView 停在 `about:blank`：
   `mang://localhost…`、`mang.localhost…`、`http://mang.localhost…`、`https://mang.localhost…`

壳内部按 `req.uri().path()` 匹配，改写前后都能命中。

### MIME

`.html` → `text/html; charset=utf-8`；`.js/.mjs` → `text/javascript; charset=utf-8`；`.css`；`.json/.map`；`.wasm` → `application/wasm`；`.svg/.png/.jpg/.gif/.webp/.ico`；`.woff/.woff2/.ttf/.otf`；`.pdf`；`.md/.txt`。**其它一律 `application/octet-stream`**（`.bcmap`、`.pfb` 走这条）。

目录穿越（解析后不在 UI 根下）→ `403`；找不到 → 回退 `index.html`（前端路由友好）。

---

## 2. 请求

```jsonc
{ "id": "r1", "cmd": "fs.read", "path": "技术标.md" }
```

- `id`：字符串（建议 `"r" + ++seq`）。缺省为 `null`（仍会回帧，`id:null`）。
- `cmd`：见 §6 命令表。
- 其余字段：命令参数，**平铺在顶层**（不嵌套）。
- 多余字段被忽略（永不因多余参数报错）。
- **壳永不 panic**：坏 JSON → `bad_json`；参数类型不对 → `bad_args`；未知命令 → `unknown_cmd`。

---

## 3. 响应

### 3.1 成功

```js
try{window.__ipcResolve&&window.__ipcResolve("r1", {"id":"r1","ok":true,"result":{…},"data":{…}}, true);}catch(e){}
```

帧体：`{"id", "ok":true, "result":{…}, "data":{…}}`

`result` 与 `data` **内容相同**（兼容两种读法）。`ok` 在函数第三个参数上再给一遍。

### 3.2 失败

```jsonc
{ "id": "r2", "ok": false,
  "code": "outside_workspace", "message": "路径越界：…",
  "error": { "code": "outside_workspace", "message": "路径越界：…" } }
```

错误帧**同时**给顶层 `code`/`message` 和嵌套 `error:{code,message}`。

### 3.3 错误码

| code | 含义 | 典型触发 |
|---|---|---|
| `bad_json` | 请求不是合法 JSON / 不是对象 | 手抖拼错 |
| `bad_args` | 参数缺失或类型不对 | `fs.read` 无 `path` |
| `unknown_cmd` | 没有这个命令 | `nope.cmd` |
| `not_found` | 路径不存在 | 文件被删/改名 |
| `denied` | 存在但越权（写工作区非 `.mang/`、拒绝访问） | `fs.writeBytes` 写正文 |
| `outside_workspace` | 解析后不在工作区内（`../` 穿越、盘符外绝对路径） | 安全拦截 |
| `too_large` | 超过该命令的体积上限 | 50MB 的 pdf 用 `fs.read` |
| `io` | 其它系统 IO 失败 | 磁盘满、权限 |

---

## 4. 大响应（> 1.5 MB）

阈值 `INLINE_LIMIT = 1_500_000` 字节（按响应体 JSON 序列化后计）。

1. 壳把**完整响应帧**写到 `%TEMP%\mang-ipc\<safe_id>.json`（UTF-8，就是那个响应帧本身）。
2. 内联帧变成：

```json
{ "id": "r7", "ok": true, "result": { "file": "r7.json", "bytes": 2796354, "inline": false }, "data": { …同上… } }
```

> `result.bytes` = 整个响应帧 JSON 序列化后的字节数（**不是**原文件大小）。判断是否走了落盘请认 `result.inline === false`
> 且 `result.file` 是**纯文件名**（不含 `:`、`/`）。`recent.load` 的 `file` 是磁盘路径，**不要**拿去 fetch。

3. UI 用**相对路径**取回（同一 origin，只服务一次）：

```js
const url = '/__ipc/' + resp.result.file;      // → http://mang.localhost/__ipc/r7.json
const full = await (await fetch(url)).json();  // 与内联时那个帧完全一致：{id, ok, result, data}
```

4. 壳**读一次即删**；第二次取回 `404`，body 为 `gone`。
5. 启动时清理上一轮遗留的 `%TEMP%\mang-ipc\*.json`；退出前也清。
6. `safe_id` 只保留 `[A-Za-z0-9_-]`（其它字符换 `_`），最长 120；服务端只接受纯文件名，`../` 与分隔符一律拒绝。

> 判定依据是**实际写盘的字节数**，不是预估：小响应永远内联。

实测：2 MB 文件 → 响应 2,796,354 B → 落盘 → fetch#1 200 解码 2,097,153 B → fetch#2 404 `gone`。

---
## 5. 路径与安全围栏

解析顺序（`paths::Workspace::resolve`）：

1. 空串 → 工作区根。
2. 相对路径 → 以工作区根为基准（`/` 与 `\` 都当分隔符；`.` 忽略，`..` 逐段回退）。
3. 绝对路径 → 原样使用（对话框/拖拽/最近打开来的）。
4. `canonicalize`；**目标不存在时** canonicalize 最近的存在祖先，再拼回剩余段（支持“新建文件”路径）。
5. 结果必须位于 canonical 工作区根之下 → 否则 `outside_workspace`。

**比较按路径组件、大小写不敏感**（Windows 语义）：`C:\ws2` 不会被当成 `C:\ws` 的子目录；`C:\WS\x` 在 `C:\ws` 内。

写权限：`fs.writeBytes` **只允许写工作区内的 `.mang/`**（历史快照/导出中间件）。正文保存要另开命令并带显式授权——不要顺手放宽这条。

**`.mang/` 懒创建**：启动时**不**建 `<ws>/.mang`；只有真正写历史时才创建目录（工作区常是 git 跟踪的 fixture 目录）。

---

## 6. 命令表

单位约定：`size`/`bytes` = 字节；`mtimeMs`/`ts`/`openedAt` = **Unix 毫秒**（UTC）；`rel` = 相对工作区、**`/` 分隔**；绝对路径用 `\`。

### ping
```jsonc
→ { "pong": true, "t": 1793260341382 }
```

### app.info
```jsonc
→ {
  "version": "0.1.0",
  "workspace": "E:\\Code\\ai\\md\\audit\\ai-bid-research\\fixtures\\corpus-A",
  "ui_dir": "E:\\Code\\ai\\md\\audit\\ai-bid-research\\web-ui\\dist",
  "exe": "E:\\…\\bidcore\\target\\release\\mang-app.exe",
  "pid": 4236,
  "platform": "windows",
  "protocol": 1
}
```

### fs.tree  `{ path?, depth?=8, includeHidden?=false }`

- `depth`：1~32（默认 8）。
- 排序：**目录在前**，同类**自然排序**（`第2章` 在 `第10章` 前、`a2` 在 `a10` 前）。
- 永远跳过：`.git` / `node_modules` / `target` / `.mang` / `$recycle.bin`。
- `includeHidden=false`（默认）时跳过所有以 `.` 开头的条目。
- 单目录读失败（权限）只跳过该目录，不失败整棵树。
- 节点上限 4000：超出置 `truncated:true`（先序遍历，先满足目录）。

```jsonc
→ {
  "root": "E:\\…\\fixtures\\corpus-A",
  "path": "",                       // 入参 path 原样回显，根目录为 ""
  "nodes": [
    { "name": "files", "rel": "files", "kind": "dir", "mtimeMs": 1793015364734,
      "children": 1,                 // = 子节点个数（number）
      "nodes": [                     // = 子节点数组（真正的 children）
        { "name": "B3-图纸目录.xlsx", "rel": "files/B3-图纸目录.xlsx", "kind": "file",
          "size": 6168, "mtimeMs": 1793015364738 }
      ] },
    { "name": "manifest.json", "rel": "manifest.json", "kind": "file", "size": 10794, "mtimeMs": 1793015364734 }
  ],
  "nodeCount": 29,
  "nodeCap": 4000,
  "truncated": false
}
```

> 目录节点：`children`=个数（number）、`nodes`=子数组；文件节点没有这两个键。

### fs.read  `{ path }` — 文本
上限 **8 MB**（超了 `too_large`）。UTF-8 **lossy** 解码（坏字节换成 U+FFFD，不报错）。

```jsonc
→ { "path": "E:\\…\\技术标.md", "rel": "技术标.md", "bytes": 1620,
    "text": "# 第一章 概述\n\n正文。\n", "mtimeMs": 1793015364726 }
```

### fs.readBytes  `{ path }` — 二进制
上限 **48 MB**。`data` 为标准 base64（带 `=` 填充）。

```jsonc
→ { "path": "E:\\…\\B3-图纸目录.xlsx", "rel": "files/B3-图纸目录.xlsx",
    "bytes": 6168, "enc": "b64", "data": "UEsDBBQ…", "mtimeMs": 1793015364738 }
```

> >1.5 MB 时走 §4：`result = {file, bytes, inline:false}`。

### fs.writeBytes  `{ path, enc:"b64", data }` — 只写 `.mang/`
```jsonc
→ { "path": "E:\\…\\.mang\\history\\技术标.md\\1.md", "rel": ".mang/history/技术标.md/1.md", "bytes": 13 }
```
非 `.mang/` → `denied`；越界 → `outside_workspace`；坏 b64 → `bad_args`；上限 64 MB。落盘**先写 `.tmp` 再 rename**。

### fs.stat  `{ path }`
不存在**不报错**，返回 `exists:false`。
```jsonc
→ { "path": "E:\\…\\files", "rel": "files", "size": 0, "mtimeMs": 1793015364734, "isDir": true, "exists": true }
```

### fs.mkdir  `{ path }`
`create_dir_all`，父目录一起建。→ `{ "ok": true, "path": "…", "rel": "新目录/子" }`

### file.fromPath  `{ path }` — 绝对路径归一
拖拽 / 对话框 / 最近打开给的绝对路径 → 统一描述。会展开 `%VAR%` 与开头 `~`。不在工作区时 `rel:null、insideWorkspace:false`（**不报错**）。
```jsonc
→ { "path": "D:\\项目A\\技术标.md", "rel": "技术标.md", "kind": "file",
    "size": 1620, "mtimeMs": 1793015364726, "insideWorkspace": true }
```

### dialog.openFile  `{ filters?, title?, multiple?=true, directory? }`
`filters: [{"name":"Word","extensions":["docx","md"]}]`（扩展名带不带点都行，`*` 忽略）。**主线程同步弹模态**（Win32 `IFileOpenDialog`，rfd）。
```jsonc
→ { "paths": ["D:\\项目A\\技术标.md"], "canceled": false }
```

### dialog.openFolder  `{ title?, directory? }`
```jsonc
→ { "paths": ["D:\\项目A"], "canceled": false }
```
取消时 `{"paths":[],"canceled":true}`（不是错误）。

> **阻塞语义**：这两个命令在主线程同步弹模态，会阻塞 UI 线程直到用户选完/取消（浏览器模态同理）。
> IPC 线程永不阻塞；对话框任务通过主线程队列串行执行。
> 无头环境设 `MANG_DIALOG_MOCK=1` 可让它们立即返回取消。

### explorer.reveal  `{ path }`
`explorer /select,"<path>"`（`CREATE_NO_WINDOW`）；目录则直接打开；路径不存在则打开最近的存在的父目录。→ `{ "ok": true, "path": "…" }`

### recent.load / recent.save `{ items }`
文件 `%APPDATA%\mang\recent.json`（缺失退回 LOCALAPPDATA → TEMP），最多 50 条，按 `openedAt` 倒序。同一 `path` 只留最新一条。写盘先 `.tmp` 再 rename。
```jsonc
recent.load → { "items": [ { "path":"D:\\项目A\\技术标.md", "name":"技术标.md",
                            "kind":"file", "openedAt":1699999999000, "exists":true } ],
                "file": "C:\\Users\\…\\AppData\\Roaming\\mang\\recent.json" }
recent.save → { "ok": true, "count": 1, "file": "…\\recent.json" }
```

### history.list  `{ path?, limit?=200 }`
读 `<workspace>/.mang/history/index.json`，容忍三种形态：数组 `[entry]`、`{"items":[entry]}`、**按文件分组** `{"技术标.md":[entry]}`。`kind`：`"manual"`（AI 主动存，也接受 `"ai"`）或 `"auto"`。
```jsonc
→ { "items": [ { "ts": 1699999999000, "kind": "manual", "message": "第1章 概述完成",
                 "file": "技术标.md", "snapshot": ".mang/history/技术标.md/1699999999000.md" } ],
    "file": "E:\\…\\.mang\\history\\index.json", "present": true }
```
文件不存在：`{items:[], present:false}`；坏 JSON：`{items:[], present:true, error:"…"}`（都不报错）。

### preview.write  `{ name, enc:"b64", data }`
写 `%TEMP%\mang-preview\<uuid>-<name>`，给大文件在页面里预览（不碰工作区）。文件名清洗掉 `/ \ : * ? " < > |` 与控制字符。上限 48 MB。
```jsonc
→ { "path": "C:\\temp\\mang-preview\\3f2a…-报价单.xlsx", "bytes": 18320, "name": "报价单.xlsx" }
```

---

## 7. 事件（无 id）

```js
try{window.__ipcEvent&&window.__ipcEvent({…});}catch(e){}
```

### drop —— Windows 拖放（`WM_DROPFILES` + `SetWindowSubclass`）
```jsonc
{ "ev": "drop",
  "paths": ["D:\\项目A\\技术标.md", "C:\\Users\\x\\桌面\\a.pdf"],      // 绝对路径，已去掉 \\?\ 前缀
  "within": [ { "path": "D:\\项目A\\技术标.md", "rel": "技术标.md" } ],  // 仅工作区内的
  "ts": 1793260341382 }
```
- 工作区内的文件才出现在 `within`；外部的只进 `paths`（UI 用 `file.fromPath` 归一）。
- 必须 `DragFinish`（已做），否则系统资源泄漏。
- 回调在 UI 线程，只做“入队 + 唤醒”，真正注入由事件循环统一做。
- 自测：`mang-app.exe --inject-drop "D:\a.docx;D:\b.xlsx"` 会在启动 1.5s 后推一条同样的 drop 事件（已验证能直接开出两个 Tab）。

---

## 8. 页面侧需要提供的两个函数

```js
window.__ipcResolve = function (id, resp, ok) { /* resp 是完整帧；ok 是布尔副本 */ };
window.__ipcEvent   = function (ev) { /* drop / theme … */ };
```
壳的注入**永远**带 `window.__ipcResolve&&` 守卫 + `try/catch`：函数没定义就静默跳过，抛异常也只吞掉，绝不影响页面。

壳另外在文档创建时设置 `window.__mangShell = {version:1, protocol:1}`（仅用于“我跑在壳里”检测）。

---

## 9. 命令行

```
mang-app.exe [DIR] [选项]

  -w, --workspace <DIR>   工作区目录（默认当前目录）。也可直接 mang-app.exe DIR
      --ui <DIR>          UI 目录（默认：<exe>/ui → <exe>/../../../web-ui/dist → ./web-ui/dist → ./dist）
      --devtools          启用 DevTools
      --selftest          不开窗口跑 IPC 自检，打印 PASS/FAIL，退出码 0=全过
      --ls-ui             只打印解析出的 UI 目录然后退出
      --inject-drop <P1;P2> 启动 1.5s 后模拟一次拖放（无鼠标环境验证 DnD）
  -h, --help              帮助
```

环境变量：
- `MANG_DIALOG_MOCK=1`：不弹真实系统对话框，`dialog.*` 直接返回 `{paths:[], canceled:true, mock:true}`（无头自检/CI）。
- `MANG_DEVTOOLS=1`：等价于 `--devtools`。
- `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<N>`：打开 WebView2 CDP（冒烟脚本用）。

- 位置参数与 `--workspace` 同给时 **`--workspace` 优先**。
- 未知选项 → 打印用法 + 退出码 **2**；工作区不存在 → 退出码 **1**。
- 窗口：标题 `mang — 标书工作台`，初始 1440×900，**最小 1024×640**。

---

## 10. 自检

```
bidcore\target\release\mang-app.exe --selftest
```

无窗口跑：ping / app.info / fs.tree / fs.read / fs.readBytes / 大响应落盘取回删除 / fs.writeBytes(允许+拒绝) / fs.stat / fs.mkdir / 路径围栏 5 例 / recent 往返 / history.list / unknown_cmd / bad_json / 启动清理。输出 `PASS <name>：<detail>` / `FAIL …`，最后 `== N passed, M failed ==`。

窗口内 IPC 冒烟（真实 WebView2，走 CDP）：
```
node scripts/mang-dev/mang_ipc_smoke.mjs
```

---

## 11. 开发脚本

| 脚本 | 作用 |
|---|---|
| `scripts/mang-dev/mang_dev.ps1 build` | 结束占用进程 + `cargo build --release -p mang-app` |
| `scripts/mang-dev/mang_dev.ps1 selftest` | 跑 `--selftest` |
| `scripts/mang-dev/mang_dev.ps1 ui` | 打印解析出的 UI 目录 |
| `scripts/mang-dev/mang_dev.ps1 run [工作区]` | 启动 |
| `scripts/mang-dev/mang_ipc_smoke.mjs` | 真实窗口 IPC 冒烟（造临时工作区 + 探针 UI，CDP 驱动） |
| `scripts/mang-dev/mang_ipc_probe.js` | 冒烟用页面探针（被 smoke.mjs 拷进临时 UI） |

退出码：`0` 全过 / `1` 有失败 / `2` 环境问题（找不到 exe 或页面）。

用法：`node scripts/mang-dev/mang_ipc_smoke.mjs [--ui DIR] [--workspace DIR] [--port N] [--keep]`
