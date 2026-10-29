# 任务：修复 mang 桌面壳的自绘标题栏（拖拽 / 双击 / 上边缘缩放全部失效）

## 工作区
- 仓库：`E:\Code\ai\md\audit\ai-bid-research`
- UI：`web-ui/`（esbuild 打包到 `web-ui/dist/`）
- 桌面壳：`bidcore/crates/mang-app/`（Rust，tao 0.37.2 + wry 0.57.1 + WebView2）
- **mang-app 是独立 crate**，有自己的 `[workspace]` 空表，不在 bidcore workspace members 里。
  构建：`cd bidcore\crates\mang-app && cargo build --release`
  产物：`bidcore\crates\mang-app\target\release\mang-app.exe`
  启动（必须显式给 --ui，因为 target 目录变了推导不到）：
  ```
  mang-app.exe --workspace E:\Code\ai\md\audit\ai-bid-research --ui E:\Code\ai\md\audit\ai-bid-research\web-ui\dist
  ```
- UI 构建：`cd web-ui && npm run build`

## 技术架构
- 窗口用 `with_decorations(false)`（无系统标题栏），自己用 WebView 里的 HTML 菜单栏当标题栏
- `chrome.rs` 通过 `SetWindowSubclass` 挂 `WM_NCCALCSIZE` / `WM_NCHITTEST` / `WM_NCLBUTTONDBLCLK`
- `WM_NCCALCSIZE` 在 wparam==TRUE 时返回 0（把整窗当客户区）
- `WM_NCHITTEST` 对标题带返回 `HTCAPTION`，左右下三边 5px 返回 `HTLEFT/HTRIGHT/HTBOTTOM` 系列
- 窗口按钮（最小化/最大化/关闭）由 UI 走 IPC `window.action` → `chrome::window_action` → `SetWindowPos`
- 最大化状态自己维护 `IS_MAXED` 原子量（不用 `IsZoomed`，因为不走 `SW_MAXIMIZE`）

## 已经确认的事实（不要再重复验证）

**1. `hit_test()` 逻辑是对的。** 用 `SendMessage(WM_NCHITTEST)` 外部探测主窗口：
```
rel 0..4   -> 12 (HTTOP)      顶部缩放边
rel 8..32  -> 2  (HTCAPTION)  标题带
rel 36+    -> 1  (HTCLIENT)
```

**2. 但真实鼠标点击标题带时，主窗口一次 `WM_NCHITTEST` 都收不到。**
在 `chrome.rs` 的 `WM_NCHITTEST` 分支里加了写文件日志（环境变量 `MANG_HT_LOG=1` 开启），
用真实鼠标点击标题带 32px 区域 → 日志文件 0 行；
同样的机制用外部 `SendMessage` 触发 → 日志正常输出。
**结论：鼠标消息根本没到主窗口。**

**3. 窗口层级（`EnumChildWindows` 实测）：**
```
MAIN                           1243x682  at 102,102
└ WRY_WEBVIEW         1229x674  at 110,103    ← 从主窗口第 1 像素就开始铺
  └ Chrome_WidgetWin_0
    └ Chrome_WidgetWin_1
      ├ Chrome_RenderWidgetHostHWND  1230x674
      └ Intermediate D3D Window      1434x816
```
`WindowFromPoint` 在标题带 y=2/10/16/26 处命中的是 `Chrome_RenderWidgetHostHWND`，
另有一次命中的是 `MozillaWindowClass`。

**4. 用户的关键观察：左右下三边能缩放，只有上边不能。**
这与「WebView 盖住了顶部 32px」完全吻合——下左右三边露在 WebView 矩形之外，系统能问到主窗口。

## 核心问题定位

`WM_NCCALCSIZE` 返回 0 让整窗成为客户区（尺寸 1243x682），但 **wry 创建 `WRY_WEBVIEW` 子窗口时
按「有 32px 标题栏」的客户区来定位**（y=103，只比主窗口顶部低 1px，实际应该低 32px）。
于是 WebView 覆盖了标题带，Chromium 渲染窗口吃掉了所有鼠标消息。

## 候选修复方案（未实施，需自己判断选哪个）

**方案 A**：在 `WM_NCCALCSIZE` / `WM_SIZE` 后，对 `WRY_WEBVIEW` 子窗口做 `SetWindowPos`，
y 下移 32px，高度减 32px。注意要处理 DPI 缩放（`GetDpiForWindow`，本机 120dpi = 1.25 倍）。

**方案 B**：不再让 `WM_NCCALCSIZE` 返回 0，改成保留真正的非客户区（32px），
让系统自己把 WebView 放在 32px 以下。更接近 Electron/VSCode 的标准做法。

**方案 C**：标题栏做成 WebView 之外的原生子窗口。改动最大。

建议先评估 B（系统托管客户区计算，最稳），再考虑 A。

## 已排除的错误方向（不要再走）

1. **不要改 `hit_test()` 的边框/标题带计算** —— 外部探测证明它是对的
2. **不要加 `mpsc` 队列 + `recv_timeout` 等待** —— `handle_ipc` 跑在 UI 线程，
   等事件循环 = 自己堵自己，曾导致点击延迟 1.5 秒
3. **不要用 JS 上报 `window.caption` 来开关命中测试** —— 已全部删除，
   它在「按下 → 系统进拖动循环」之间改状态，导致完全拖不动
4. **不要用 tao 的 `set_maximized`** —— 在去边框窗口上只改内部状态不动窗口
5. **不要加 DPI 夹紧/兜底** —— 本机实测 DPI=120、scale=1.25、border=6px，数值正常，
   加夹紧只会掩盖问题

## 环境坑（非常重要）

- **`read` 工具在这台机器上会截断到约 1000 行**，且不报错。曾两次因此把 `app.js` 写坏。
  **处理大文件一律用 `node -e` 脚本读写，不要依赖 read 的行号。**
- PowerShell 命令必须是**单行**、不能含双引号；复杂脚本写成 `.ps1` 文件再
  `powershell -NoProfile -ExecutionPolicy Bypass -File` 执行
- 合成鼠标输入（`SetCursorPos` + `mouse_event`）在这台机器上**拖不动任何窗口**，
  包括纯原生 Win32 窗口。所以「模拟拖动」无法用来验证，只能验证消息流
- 真机自动化：
  ```
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9401'
  ```
  然后 playwright-core 的 `chromium.connectOverCDP('http://127.0.0.1:9401')`
  （playwright-core 在 `C:/temp/mang-dev/node_modules/playwright-core`）
- 杀进程：`Get-Process mang-app | Stop-Process -Force`

## 当前状态

- 代码可编译，app 能正常启动，UI 功能（文件树 32 行、git 面板、PDF、docx、xlsx）都正常
- **唯一问题：标题栏拖拽、双击最大化、上边缘缩放全部失效**
- 窗口按钮（最小化/最大化/关闭）**正常且无延迟**（约 50-110ms），这部分不要动
- `chrome.rs` 里当前有一个诊断日志开关（`MANG_HT_LOG=1` 时把 `WM_NCHITTEST` 坐标写进
  `%TEMP%\mang-ht.log`），定位完成后记得删掉

## 验收标准

1. 在标题栏空白处按住左键能拖动窗口
2. 双击标题栏能最大化/还原
3. 上边缘能拖拽缩放（和左右下三边一致）
4. 菜单栏按钮、右上角三个窗口按钮仍然可点
5. 上面这些都验证完，把诊断开关删掉

## 代码规范

- UI 文案和代码注释用中文
- 不新增 npm 依赖
- 不要改动 `mdcore` 仓（另一个仓库）
- 不要跑任何 git 写操作

## 先做这一步

先读 `bidcore/crates/mang-app/src/chrome.rs` 和 `main.rs` 里窗口创建的部分，
确认 `WM_NCCALCSIZE` 的处理和 `WRY_WEBVIEW` 的位置关系，再决定用 A 还是 B。
拿到真实证据（消息日志 / 窗口矩形）再改代码，不要凭猜测修改。

---

# 修复记录（2026-10-29）

原诊断「WebView 按有标题栏的客户区定位、盖住标题带」**方向对、原因错**。真正的根因在 tao 那边：

## 根因（已用活体对照实验证实）

1. **tao 的 `decoration_shadow` 默认是 `true`。** 无装饰 + 有阴影时，tao 的 `WM_NCCALCSIZE`
   走 `util::calculate_window_insets()`，把客户区按阴影内缩（实测 1243x682 的窗口算出
   1228x673 的客户区）。tao 处理完这条消息**之后**才轮到我们的子类，所以以前在这里
   返回 0 只是把结果又改了一次，客户区早已算错 —— WebView 于是从窗口第 1 像素铺开。
   对照实验：临时把 `WS_THICKFRAME` 摘掉再 `SetWindowPos`，客户区立刻变成整窗。
   **修法：`WindowBuilder::with_undecorated_shadow(false)`。**

2. **鼠标永远先问 WebView。** WebView 铺满整个客户区，`WindowFromPoint` 永远命中
   `Chrome_RenderWidgetHostHWND`；系统先给 WebView 派发 `WM_NCHITTEST`，再问父链。
   WebView 答 `HTCLIENT`（Chromium 默认），于是主窗口的派生命中测试（拖动 / 双击 /
   顶边缩放）一个都触发不了 —— 而左右下三边「能用」，是因为它们落在 WebView 矩形之外
   的几个像素上，系统直接问了主窗口。**这才是「只有上边不能缩放」的真原因。**

3. **命中测试的坐标系是 `GetWindowRect`。** 本机 `DWMWA_EXTENDED_FRAME_BOUNDS` 与
   `GetWindowRect` 相差 45px（DWM 位图缩放）。用 DWM 矩形做基准，整条标题带都会探测落空。
   已用两套基准对照验证过，必须用 `GetWindowRect`。

## 改动清单

- `chrome.rs`
  - `hit_rect()`：显式统一命中测试基准（= `GetWindowRect`），写明为什么不能用 DWM 矩形
  - `hit_test()`：左右下三边 → 标题带（HTCAPTION）→ 顶边 / 左上角 / 右上角
  - `WM_NCHITTEST` 在子类里直接给答案，不再往下透传
  - 新增 `set_buttons_width()`：标题带右端给 UI 的窗口按钮让位
  - 新增 `set_state_emit()`：双击标题栏改最大化状态后回调 UI
  - 删除交接时留下的 `MANG_HT_LOG` 诊断开关
- `main.rs`
  - `.with_undecorated_shadow(false)`（根因 1）
  - `window.buttons` IPC：UI 上报窗口按钮占位宽度
  - `window.__mangShell` 改用 `with_initialization_script`（原来 `evaluate_script` 发得太晚，
    UI 的 `boot()` 已经跑完 → 窗口按钮不渲染、标题栏按浏览器模式处理）
- `web-ui/mang/app.js`
  - 用 `ResizeObserver` 盯按钮组宽度并上报（桌面壳里窗口缩放不触发 window resize）
  - 监听 `mang:windowstate`（双击标题栏后同步按钮图标）

## 验收（已实测）

系统命中测试矩阵（`SendMessage(WM_NCHITTEST)`，屏幕坐标，窗口 1229x674 @ (26,26)）：

```
标题带 y=顶部+8/+16/+31  -> 2  (HTCAPTION)   拖动 / 双击最大化
正文内容区                -> 1  (HTCLIENT)
左边                      -> 10 (HTLEFT)
右边                      -> 11 (HTRIGHT)
下边                      -> 15 (HTBOTTOM)
左上角 / 右上角           -> 10 / 11
右上角窗口按钮区          -> 1  (HTCLIENT，不会被当成拖动)
```

另外：`cargo build --release` 零警告；`mang-app.exe --selftest` 23 passed / 0 failed；
CDP 实测 `window.action toggle` 最大化 / 还原往返正常，UI 的 `data-maximized` 与按钮文案同步。

## 这台机器上验证不到的部分（重要）

- **合成鼠标注入在这台机器上完全无效**（SendInput 绝对坐标、SetCursorPos+mouse_event 都试过）：
  光标会动，但目标进程输入队列里一条鼠标消息都没有（`PeekMessage` 实测为空），打原生 Win32
  窗口也一样。所以「真实按下 → 系统派发」这一段没法在本机自动化验证，只能靠上面的命中测试
  矩阵 + 代码路径推理。**请在有真实鼠标的会话里手工过一遍验收标准 1-4。**
- Firefox 会反复抢前台，干扰前台点击测试。

## 留下的临时工具

`E:\Code\ai\md` 下的 `wm2.ps1`（鼠标注入）、`htp.ps1` / `matrix.ps1` / `scanb.ps1`
（命中测试探测）、`dwm.ps1`（窗口矩形对比）是这次排查的临时工具，不在仓库里，可随时删除。
