# mang-app Windows 构建与运行验证手册

> 目的：让 mang-app.exe（标书桌面壳）在**真实 Windows 机器**上从零构建、启动、
> 通过验收清单。本手册是 issue #68「Windows 实机可用性攻坚」任务 A 的交付物。
>
> **编写环境声明**：本手册在 Linux 开发沙箱编写（无 Windows、无图形栈）——按
> Lead 纪律「不要用 Xvfb 冒充 Windows 验证」，手册本身就是交付物，实测由
> 在 Windows 机器上执行本手册的人完成并把验收清单逐项打勾回帖。
> 手册中每一条命令与预期输出都来自代码与构建脚本的静态事实（引用见各节），
> 不含凭空编造的步骤。
>
> 适用版本：main @ `73e2697` + `fix/ui-resolve-mismatch`（UI 解析修复——见
> §5.4，未合并该修复时按旧文行为会报「找不到 UI」）。

---

## 1. 环境要求

| 组件 | 要求 | 用途 | 验证命令 |
|---|---|---|---|
| Windows | Win10 1909+ / Win11（x64） | WebView2 Evergreen 最低支持面 | `winver` |
| Rust | stable（1.75+ 均可；CI 基线 = stable） | 编译 | `rustc --version` |
| MSVC 工具链 | VS Build Tools 2022：**MSVC v143** + **Windows SDK 10.0.22621+** | link.exe / Windows SDK | 见 §2.2 验证 |
| WebView2 Runtime | Evergreen（常驻版） | mang-app 的渲染层（wry 0.57） | 见 §2.3 验证 |
| Node.js | 18+（含 npm） | 仅构建 Web UI 用（esbuild + pdfjs-dist） | `node --version` |
| Git | 任意近版（含 git-cli fetch 能力） | 克隆 + cargo 的 git 依赖 | `git --version` |
| 磁盘 | ~10 GB 空闲（target 目录 + cargo 缓存） | | |

> 依赖事实来源：mang-app `Cargo.toml`（tao 0.37 / wry 0.57 / rfd 0.17 /
> windows 0.62——Windows 面全部经 MSVC 编译）；`.github/workflows/windows-check.yml`
> （windows-latest + `dtolnay/rust-toolchain@stable` 是已验证绿的组合）；
> `web-ui/package.json`（esbuild ^0.25 / pdfjs-dist ^6.4 / typescript ^5.7）。

### 1.1 为什么需要 PAT（重要）

mang-app 的依赖闭包里有**私有仓**：

```
mang-app → bid-tools → bid-engine → mdc-office-io（git+https://github.com/mmqz/mdcore，rev 3204342d）
```

克隆 ai-bid-research 是匿名的（公开仓），但 cargo 解析上述 git 依赖时
**必须提供 mmqz/mdcore 的读凭据**（PAT 或 SSH key）。CI 用只读 deploy key
（`MD_CORE_DEPLOY_KEY` secret）；本机构建推荐 PAT（§2.4）。

## 2. 依赖安装（每条命令可复制粘贴）

以下命令在 **PowerShell**（管理员不必需，装 VS Build Tools 时会请求提权）执行。

### 2.1 Rust（rustup）

```powershell
winget install Rustlang.Rustup
# 新开一个 PowerShell 窗口（PATH 生效），然后：
rustup default stable
rustc --version    # 预期输出形如：rustc 1.9x.x (...)
```

> 无 winget（LTSC 等）：从 https://rustup.rs 下载 `rustup-init.exe` 运行，
> 全程默认选项（默认 host triple = `x86_64-pc-windows-msvc`，正是需要的）。

### 2.2 VS Build Tools 2022（MSVC v143 + Windows SDK）

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

`--includeRecommended` 自动带上 **MSVC v143** 构建工具与 **Windows 11 SDK**
（≥ 10.0.22621）。装完**新开窗口**验证：

```powershell
# 验证 link.exe 可达（在 x64 本机工具链目录下）
Get-ChildItem "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\*\bin\Hostx64\x64\link.exe"
```

预期：列出 1 个 link.exe 路径。列不出 = 未装上，回头检查工作负载勾选。

> rustup 检测到 MSVC 后，`cargo build` 会自动调用 `link.exe`，无需手动进
> Developer Command Prompt。

### 2.3 WebView2 Runtime（Evergreen）

Win10 1909+ / Win11 消费版**通常已预装**。验证（PowerShell）：

```powershell
(Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}").pv
```

预期：输出版本号（如 `120.0.2210.61`）。**无输出/报错** = 未装，下载安装：

```powershell
# Evergreen Standalone（离线包，适合 LTSC/Server/无网环境预部署）
# 下载地址：https://developer.microsoft.com/microsoft-edge/webview2/
#   → "Evergreen Standalone Installer" x64
```

> wry 0.57 的 Windows 后端加载 WebView2 的方式是固定加载器 + 系统 Evergreen
> 运行时；**不需要**把 WebView2Loader.dll 放在 exe 旁边。

### 2.4 Node.js（仅构建 UI 需要）

```powershell
winget install OpenJS.NodeJS.LTS
node --version    # 预期 ≥ v18
```

### 2.5 mdcore 私有仓凭据（PAT 通道）

```powershell
# <你的PAT> 换成实际 token（需要 mmqz/mdcore 的读权限；只存 git 配置，勿写进任何文件）
git config --global url."https://<你的PAT>@github.com/".insteadOf "https://github.com/"
```

> 这是 CI 同款思路的本地版（CI 用 deploy key + insteadOf 改道，见
> windows-check.yml 注释）。PAT 只进 git config，不进脚本、不进环境文件——
> 沙箱重置会脱敏脚本内 token 的前车之鉴。

## 3. 构建命令

```powershell
# 3.1 克隆（任意目录；示例用 C:\dev）
cd C:\dev
git clone https://github.com/mmqz/ai-bid-research.git
cd ai-bid-research

# 3.2 构建 Web UI（产物 web-ui/dist/mang/）
cd web-ui
npm install        # 首次 ~1-2 分钟
node mang-build.mjs
cd ..

# 3.3 构建 mang-app（release）
cargo build --release --manifest-path bidcore/crates/mang-app/Cargo.toml
```

### 预期输出

| 步骤 | 预期 |
|---|---|
| `npm install` | `added N packages...`（N ≈ 100+，含 esbuild/pdfjs-dist 二进制包） |
| `node mang-build.mjs` | 末行 **`[build] ok -> dist/  (11 entries in dist/mang)`**——11 = index.html + styles.css + app.js + 3 个渲染器 js + pdf.worker.min.mjs + cmaps/ + standard_fonts/ + wasm/ + host.js。少于 11 说明缺件（§5.5） |
| `cargo build` | 首次 5-15 分钟（tao/wry/windows crate 编译量大）；末行 `Finished 'release' profile` |

### 产物位置

```
bidcore\crates\mang-app\target\release\mang-app.exe    ← 可执行文件
web-ui\dist\mang\                                      ← UI 资产（运行时读取）
```

> mang-app 是**独立 workspace**（Cargo.toml 自带空 `[workspace]` 节），所以
> target 不在 `bidcore\target\` 而在 `bidcore\crates\mang-app\target\`。
> 依赖闭包与 bidcore 共享 `~\.cargo` 全局缓存，二次构建显著变快。

## 4. 运行命令

```powershell
cd C:\dev\ai-bid-research

# 姿势一（推荐）：exe 相对 UI 候选自动定位（需 fix/ui-resolve-mismatch 已合并）
bidcore\crates\mang-app\target\release\mang-app.exe fixtures\corpus-A\files

# 姿势二：显式指定两个目录（任何版本都可用，最稳）
bidcore\crates\mang-app\target\release\mang-app.exe --workspace fixtures\corpus-A\files --ui web-ui\dist

# 姿势三：绿色版布局（见 §6 打包脚本）：exe 同目录放 ui\ 子目录
mang-app.exe D:\项目A
```

启动后控制台预期输出（main.rs 294-300 行）：

```
[mang] version 0.1.0 (windows)
[mang] workspace = C:\dev\ai-bid-research\fixtures\corpus-A\files
[mang] ui        = C:\dev\ai-bid-research\web-ui\dist
```

窗口预期：主屏工作区 80% 尺寸（≥1024x640），左侧文件树列出
`A-招标文件.docx` / `B1-设备清单.pdf` / `B2-投标函格式页-扫描件.pdf` /
`B3-图纸目录.xlsx` / `C-补遗文件01.docx` 与 `evidence\` 子目录。

### 命令行参数全集（main.rs USAGE 原文）

```
mang-app.exe [DIR] [--workspace DIR] [--ui DIR] [--devtools] [--selftest] [--ls-ui] [--help]

  -w, --workspace <DIR>   工作区目录（默认：当前目录）。也可以直接 mang-app.exe DIR
      --ui <DIR>          UI 目录（默认：<exe>/ui → 相对推导 web-ui/dist → 当前目录 web-ui/dist）
      --devtools          启用 DevTools
      --selftest          不开窗口，跑 IPC 自检并打印 PASS/FAIL（退出码 0=全过）
      --ls-ui             只打印解析出的 UI 目录路径然后退出（脚本用）
      --inject-drop <P1;P2> 启动 1.5s 后模拟一次拖放事件（自测/无鼠标环境验证 DnD 用）
  -h, --help              打印本帮助
```

**无参数启动** = 工作区取当前目录（没有目录选择对话框——首次体验改进见
issue #68 任务 C，当前版本请务必带目录参数或从目标目录启动）。

## 5. 常见错误对照表（按错误信息索引）

### 5.1 `error: linker 'link.exe' not found`

| | |
|---|---|
| 现象 | `cargo build` 报 `error: linker "link.exe" not found` |
| 原因 | VS Build Tools 未安装，或装了但没选 C++ 工作负载 |
| 解决 | 重做 §2.2（关键是 `--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended`），装完**新开** PowerShell 窗口 |

### 5.2 `LNK1104: cannot open file '...\.lib'`（kernel32.lib / msvcrt.lib 等）

| | |
|---|---|
| 现象 | 链接期报 LNK1104，找不到 Windows SDK 或 CRT 库 |
| 原因 | Windows SDK 未装（只装了 MSVC 编译器没装 SDK） |
| 解决 | 打开 "Visual Studio Installer" → 修改 Build Tools → 勾选 **Windows 11 SDK (10.0.22621.0)** → 重试 |

### 5.3 `revision 3204342d... not found` / `Authentication failed`（cargo fetch mdcore）

| | |
|---|---|
| 现象 | `error: failed to get ... mdc-office-io ... unable to update https://github.com/mmqz/mdcore` |
| 原因 | mdcore 是私有仓，匿名拉不到（无凭据，或 PAT 无该仓读权限） |
| 解决 | ① 按 §2.5 配 PAT；② 验证凭据：`git ls-remote https://github.com/mmqz/mdcore`（能列出 refs 即通）；③ 仍失败检查 PAT 的 repo 读权限范围 |

### 5.4 `[mang] 找不到 UI：尝试过 ...`

| | |
|---|---|
| 现象 | 启动即退出，打印一串候选路径 |
| 原因（甲） | §3.2 的 `node mang-build.mjs` 没跑（或 npm install 失败没注意） |
| 原因（乙） | 布局不符：UI 解析要求候选目录下有 `mang/index.html`（即 dist 根）。手工拷贝时拷错层级（把 dist/mang/* 直接撒到某目录根下）会不命中 |
| 解决 | ① 补跑 §3.2；② 用 `--ls-ui` 自查：`mang-app.exe --ls-ui` 打印实际解析到的目录（脚本化排查利器）；③ 显式 `--ui <repo>\web-ui\dist`；④ 若是 `fix/ui-resolve-mismatch` 之前的版本，见该 PR 说明——旧检查条件本身有 bug，必须先合并该修复 |

### 5.5 窗口打开但白屏

| | |
|---|---|
| 现象 | 窗口出来了，内容区全白 |
| 原因（甲） | UI 资产缺件：`dist/mang/` 下文件不全（构建日志若显示跳过 pdf.worker/cmaps 等复制行，说明 npm install 后 node_modules 缺 pdfjs-dist） |
| 原因（乙） | WebView2 Runtime 异常（§2.3 的注册表验证失败） |
| 诊断 | 加 `--devtools` 启动，看 Console 报错（404 = 资产缺件；WebView 初始化错误 = 运行时问题） |
| 解决 | 甲：删 `web-ui\node_modules` 重装重构建；乙：装 Evergreen Standalone（§2.3） |

### 5.6 控制台中文乱码

| | |
|---|---|
| 现象 | `[mang] 工作区无效：...` 等中文提示显示成乱码 |
| 原因 | 旧式控制台代码页（GBK 终端遇到 UTF-8 输出）——**只影响控制台文本，不影响窗口内容** |
| 解决 | `chcp 65001` 后重跑，或用 Windows Terminal（默认 UTF-8） |

### 5.7 Windows 保护了你的电脑（SmartScreen 蓝框）

| | |
|---|---|
| 现象 | 双击运行被蓝框拦截 |
| 原因 | 未签名 exe 的默认策略 |
| 解决 | 「更多信息」→「仍要运行」。正式分发需代码签名（后续议题，本手册不覆盖） |

### 5.8 杀毒软件删除/隔离 mang-app.exe

| | |
|---|---|
| 现象 | 构建成功后 exe 消失或被隔离 |
| 原因 | 未签名 + Rust 静态编译 exe 的启发式误报（常见于 Defender/360） |
| 解决 | 加白名单（排除 `C:\dev\ai-bid-research` 目录）；这是自建未签名软件的常态，非 mang-app 特有 |

### 5.9 拖拽文件进窗口没反应

| | |
|---|---|
| 现象 | 从资源管理器拖 docx 进窗口，无任何反馈 |
| 原因（甲） | mang-app 以**管理员**身份运行时，Windows UIPI 会拦截来自非提权进程（桌面/资源管理器）的拖放消息（WM_DROPFILES 跨完整性级别不投递）——操作系统机制 |
| 原因（乙） | 拖的不是文件而是文本选区 |
| 解决 | 甲：普通权限运行（不要右键管理员）；乙：从资源管理器拖文件图标。命令行自测通道：`--inject-drop "C:\path\A.docx"`（绕开鼠标验证 DnD 处理链路） |

### 5.10 `--selftest` 退出码非 0

| | |
|---|---|
| 现象 | `mang-app.exe --selftest` 打印 FAIL 项 |
| 原因 | IPC 面（fs 读写 / 版本快照 / gitlog 等子命令）有失败——自检就是干这个的 |
| 解决 | 按打印的 FAIL 项定位（都是确定性子命令，失败信息含子命令名与错误码）；把完整输出发回 issue #68 |

## 6. 绿色版打包（可选，分发形态）

构建脚本 `scripts/package-mang-app.ps1`（随任务 B 落地）产出 zip：

```
mang-app-portable-<version>.zip
├── mang-app.exe        ← release 构建
└── ui\                 ← web-ui\dist\ 的完整内容（含 mang\ 子目录）
```

解压到任意目录双击即用（UI 候选链第一项 `<exe>\ui` 命中——**ui\ 下必须是
dist 的整体结构，含 mang\ 子目录**，不是把页面文件直接撒在 ui\ 根下）。

打包脚本自带校验门（缺件即失败不产半包）：exe / 入口页 / 渲染器三件套 /
pdf.js 静态资源（cmaps、standard_fonts、wasm——PDF 渲染缺一即白屏）。

### 6.1 产物依赖检查（任务 B，Windows 实机执行）

```powershell
# dumpbin 需在 Developer PowerShell for VS 2022 里跑（或自行补 PATH）
dumpbin /dependents bidcore\crates\mang-app\target\release\mang-app.exe
```

预期形态（任务 B 验证目标）：依赖列表只有 **Windows 系统 DLL**（如
`KERNEL32.dll`、`USER32.dll`、`GDI32.dll`、`ADVAPI32.dll`、`ole32.dll`、
`WS2_32.dll` 之类）——tao/wry 的 Windows 面不引入第三方 DLL；WebView2 走
系统 Evergreen 运行时（非 DLL 同发）。**出现任何非系统 DLL** = 异常，把
清单发回 issue #68。

再补两项体积与静态面记录：

```powershell
Get-Item ...\release\mang-app.exe | Select-Object Length   # 体积（回帖记录）
dumpbin /headers ...\release\mang-app.exe | Select-String "machine|subsystem"
```

### 6.2 发行档打包（#248——正式分发形态）

开发档（上文缺省行为）产出可调试的 zip；**发行档**加 `-Release` 开关
（或环境变量 `MANG_RELEASE=1`——CI 用后者）：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\package-mang-app.ps1 -Release
```

发行档相对开发档的差异（全部经发行校验门，任一失败即拒发——exit 1）：

| 面 | 开发档 | 发行档 |
|---|---|---|
| UI 构建（mang-build.mjs） | minify=false（可读） | **MANG_RELEASE=1**：minify + 显式无 sourcemap |
| exe profile | workspace `[profile.release]`（strip=symbols / lto=thin / codegen-units=1——#248 保守档，`debug=false` 下无 PDB） | 同左（profile 对两档同一，差异在 UI 与校验门） |
| 版本一致性 | 不检查 | **fail-closed**：mang-app Cargo.toml ↔ web-ui package.json 不一致即拒 |
| 开发产物 | 不检查 | **零容忍**：zip 逐条目扫描 `.map`/`.pdb`/`.ts`/`node_modules`/`.git`，出现即拒 |
| SHA-256 | 无 | `SHA256SUMS.txt` 入包 + 独立 `SHA256SUMS-<version>.txt`（逐文件可独立复核） |
| 构建元数据 | 无 | `build-manifest-<version>.json`（版本/commit/工具链/zip 哈希/签名状态） |

签名：打包脚本不签（`signed=false` 显式可见）——Authenticode 由发行
工作流的可选步骤负责（证书经 GitHub secrets 提供，不入库不入日志）。

### 6.3 发行工作流（#248——`.github/workflows/windows-release.yml`）

**手动触发**（Actions 页 → windows-release → Run workflow）或**正式版本
tag**（`v*`）触发；PR/日常 push 走 windows-check（编译+测试），不因发行
面变慢。流程（15 步，失败零半成品——上传排在全部校验门后）：

```
checkout → mdcore deploy key（条件布置，与 windows-check 同款）
→ Rust + Node 工具链 → release 缓存（与 CI 缓存分 key）
→ UI 发行构建（minify；断言 dist 零 .map）
→ cargo build --release -p mang-app → 断言无 PDB
→ 打包（-Release 全部发行门）
→ Smoke check（解包后 --selftest / --ls-ui / ui\mang\index.html 在位）
→ Authenticode 签名（可选：secrets 缺席跳过+状态可见；
   tag 触发或 require_signed=true 时 fail-closed）
→ 上传 artifacts（zip + SHA256SUMS + build-manifest）
→ tag 触发时创建 GitHub Release → 构建摘要（审计面）
```

> **编写环境声明**（与手册头同款纪律）：工作流与打包脚本在 Linux 沙箱
> 编写——已本地实测的：UI 发行构建真跑（minify + 零 .map）、打包脚本
> 全部发行门（含 fail-closed 负例：违禁注入拒发 / 版本失配拒发）、
> YAML 结构解析、PowerShell 语法解析（pwsh 7.4）。**Windows runner 实跑
> 面**（真实 exe 构建 + WebView2 加载 + smoke check）由 Lead 触发
> workflow_dispatch 验证——本节命令与步骤引用的均为静态事实（脚本/
> 工作流文件内容），不含未验证的断言。

## 7. 验收清单（在真实 Windows 上逐项勾选）

> 执行环境记录：`winver` 版本 ____ ／ rustc ____ ／ 构建耗时 ____ 分钟

- [ ] **构建**：§3 全链零报错，mang-app.exe 产出
- [ ] **窗口能打开**：双击/命令行启动，2 秒内出现主窗口
- [ ] **能看到文件树**：左侧树列出 corpus-A 的 5 个文件 + evidence 目录
- [ ] **能打开 .md 并看到内容**：任选 evidence 下 markdown 文件
- [ ] **能打开 .docx 并看到内容**：点 `A-招标文件.docx`，正文渲染（含表格）
- [ ] **能打开 .xlsx 并看到表格**：点 `B3-图纸目录.xlsx`
- [ ] **能打开 .pdf 并看到页面**：点 `B1-设备清单.pdf`（多页翻页）
- [ ] **标题栏能拖动**：自定义标题栏（chrome.rs）拖拽移动窗口
- [ ] **双击标题栏能最大化**：双击切换最大化/还原
- [ ] **滚轮能滚动**：内容区滚动文档
- [ ] **状态栏显示正确**：底部状态栏路径/统计信息
- [ ] **拖放**：从资源管理器拖一个文件进窗口有响应
- [ ] **控制台输出**：启动三行 `[mang] version/workspace/ui` 路径正确
- [ ] **`--selftest`**：退出码 0

## 8. 实测结果回帖格式（issue #68）

```
### Windows 实机验证结果
- 机器：Win__ ____（物理机/VM）
- 版本：rustc ____ / WebView2 ____
- 构建：__ 分钟（首次/增量）
- 验收清单：__ / 14 项通过
- 未过项及现象：<逐条列，含 --devtools console 截图>
- 其他观察：<启动耗时/内存/交互手感>
```
