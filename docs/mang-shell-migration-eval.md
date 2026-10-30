# mang-shell 迁移评估（issue #68 阶段 5）

> 评估对象：是否把标书桌面壳 mang-app（bidcore）迁移到 mdcore 的 mang-shell。
> 结论先行：**不换**——存在一个法律级硬阻断（GPL 感染闭源产品）+ 四个工程级
> 阻断（成熟度/平台/维护面/跨仓漂移），收益面无一成立。本文按 Lead 要求只
> 评估不动手（issue 原文「这是大改」）。
>
> 评估基线：bid-research main（mang-app 十阶段完成态）vs mdcore
> `3204342d`（引擎钉定 rev）+ `feat/mang-shell-skeleton` 分支（GPUI 线所在）。
> 评估日期：2026-10-30。

---

## 〇、前提勘误（重要）

issue #68 对 mang-shell 的描述与 mdcore 仓库实际有两处出入，评估以仓库实况为准：

| issue 描述 | 仓库实况 |
|---|---|
| 「mdcore 有 mang-shell（GPUI 原生版），更快」 | GPUI 版**不在钉定 rev `3204342d` 的主线上**——在 `feat/mang-shell-skeleton` 分支（#63/#64，10-29/10-30 提交，骨架态）。主线上的 `crates/shell-native`（mdshell-native）是 **tao+wry+GTK3 栈**（与 mang-app 同栈、更旧版本），Linux-only，蓝图刚实体化 |
| 「更快」 | 无实测数据支撑（骨架尚无文档渲染基准；「快」的理论依据是 GPUI 的 GPU 文本渲染，对标的是超大 markdown，不是标书预览形态） |

## 一、两侧现状盘点

### mang-app（bidcore，现役）

| 面 | 实况 |
|---|---|
| 技术栈 | tao 0.37 + wry 0.57（Windows = WebView2）——**Windows 优先**（DnD 走 Win32 子类化、CI 有 windows-check 门） |
| Rust 壳 | 14 模块 4,736 行：main(911)/chrome(883)/fsapi(578)/ipc(463)/recent(310)/paths(301)/selftest(297)/annotations(204)/dnd(167)/dialogs(159)/gitlog(157)/b64(127)/snapshots(105)/shell(74) |
| Web UI | `web-ui/mang/`：app.js 2,850 行 + styles 2,547 行（VSCode 风格）+ host.js；**三渲染器模块**（docx_view / xlsx_view / pdf_view——懒加载契约） |
| 业务接线 | 直链 bidcore（方案 A，不经 MCP）；mang-snap 版本快照同源；#42 十阶段全完成 + #54 大纲面板 + #53 公式面 + 来源标注浮层 |
| IPC | postMessage + custom protocol `/__ipc/`（大响应读一次即删）——契约有文档（docs/mang/IPC-CONTRACT.md）+ selftest 面 |
| 许可 | **Proprietary**（闭源） |

### mang-shell（mdcore `feat/mang-shell-skeleton`，GPUI 线）

| 面 | 实况 |
|---|---|
| 技术栈 | GPUI（Zed 的 `third_party/zed` 搬运闭包：21 crate / **157K LOC**，零源码修改；`gpui_platform` 增补 8 crate / 62K LOC）；features = **wayland + x11（Linux-only）** |
| 源码规模 | **775 行 / 7 文件**：title_bar(69)/activity_bar(120)/panels(164)/shell(118)/status_bar(42)/settings(54)/assets(124)——**布局骨架**（Lead 布局 7/7 视觉验证），无任何业务功能 |
| 业务功能 | 无 IPC 契约、无文件系统面、无快照/gitlog/对话框/拖放/最近列表、无 docx/xlsx/pdf 渲染（编辑区是空主视图） |
| 工程形态 | 独立 cargo workspace（循 third_party 先例，主仓门禁不触达）；edition 2024；3 条 crates.io patch 钉 git rev（async-task/calloop/khronos-egl） |
| 许可 | **GPL-3.0-or-later**（依赖 Zed `ui`/`theme` 两 crate 均为 GPL-3.0-or-later；gpui 本体是 Apache-2.0，但 UI 组件层在 GPL 面） |
| 验证面 | `--smoke-sec N` Xvfb 无头烟雾（布局冒烟级） |

### mdshell / mdshell-native（mdcore 主线，参照系）

- `mdshell`（480 行）：文档生命周期**契约层**（open/渐进 open/save/document_id + OpenProgress 事件 + Host trait）——headless 可测，这是 mdcore 侧真正成熟的资产；
- `mdshell-native`（447 行）：tao 0.30 + wry 0.46 + GTK3/WebKitGTK，**Linux-only**（其他平台 stub 保编译），GUI 需 webkit2gtk-4.1 系统库——比 mang-app 同栈但**更旧、更少功能、平台相反**。

## 二、迁移成本

| 项 | 量级 | 说明 |
|---|---|---|
| Rust 壳业务重建 | **~4,700 行全部重写** | mang-shell 无 IPC/fsapi/snapshots/gitlog/dialogs/dnd/recent 任何对应物；GPUI 的窗口/事件模型与 tao 完全不同（非搬运是重写） |
| Web UI 处置 | **~5,600 行废弃或双轨** | GPUI 是原生 UI——app.js/styles/三渲染器全部无法复用；docx/xlsx/pdf 渲染需在 GPUI 生态重建（现状 GPUI 生态无现成 Office 渲染器） |
| 平台面 | **Windows 支持从零开始** | mang-shell 仅 wayland/x11；标书用户在 Windows（mang-app.exe 是交付形态）。GPUI 的 Windows 后端在 Zed 上游存在，但搬运闭包未启用且未经 mdcore 验证 |
| 系统面 | Linux 侧需 wgpu/EGL/wayland 全家桶；Windows 侧未知 | mdshell-native 的 GTK 自愈经验不可迁移（GPUI 不走 GTK） |
| 跨仓依赖 | 钉 rev + 分支 | mang-shell **在分支上不在主线**——bidcore 要依赖它得等 mdcore 合主线并出 rev；157K LOC Zed 闭包 + 3 条 patch 的漂移全量继承 |
| 团队知识 | GPUI/edition 2024 新面 | mang-app 的 tao/wry 经验（含两轮踩坑：#42 十阶段）不可平移 |

**合计：等于把桌面壳推倒重做一遍，且目标平台（Windows）从零开始。**

## 三、收益

| 假想收益 | 评估 |
|---|---|
| 「更快」 | ❌ 无证据。GPUI 的优势场景是超大文本 GPU 渲染（Zed 编辑百万行代码）；标书壳是**只读预览**（main.rs 自述），瓶颈在渲染器（docx/xlsx/pdf 的解析与排版——WebView 走浏览器原生排版，恰好是强项）。渲染器重建后 GPUI 版大概率**更慢**（自绘排版 vs 浏览器二十年优化） |
| 原生观感 | ➖ 标书用户要的是「打开标书文件看内容」，VSCode 风格 WebView 已达标（mang-shots 截图为证）；无用户反馈要求原生感 |
| Linux 支持 | ➖ 用户在 Windows；mang-app 若要 Linux 支持，同栈的 mdshell-native 反而可参考 |
| 引擎一致性 | ❌ 标书业务直链 bidcore（方案 A），不走 mdcore 文档模型；mdshell 契约层（markdown 文档生命周期）与标书管线无交集 |
| 复用 mdcore 壳成果 | ❌ mang-shell 是 775 行布局骨架——没有可复用的业务件 |

## 四、风险

1. **GPL 感染（法律级，硬阻断）**：mang-shell 及其依赖闭包（Zed `ui`/`theme`）为 GPL-3.0-or-later；bidcore 全仓 **Proprietary** 闭源。链接 GPL 代码 = 标书系统整体负 GPL 义务（开源/发布合规）。mdcore 自己的 ADR-C1 对核心也是「GPL 零容忍」（mang-shell 以独立 workspace 隔离才得以存在）——闭源商用产品不能走这条路，除非整个 ai-bid-research 决定开源。
2. **平台倒挂**：迁到一个 Linux-only 骨架，把现役 Windows 用户面丢掉。
3. **成熟度悬崖**：从十阶段完成态（截图/CI/夹具测试齐备）退到布局骨架。
4. **维护面膨胀**：157K LOC Zed 搬运闭包 + 3 条 git-pinned patch + edition 2024 + 独立 workspace——每次 Zed 上游安全/兼容更新都是跨仓手术。
5. **主线未合**：mang-shell 在分支上，mdcore 主线随时可能调整方向（#63/#64 进行中）——现在钉 rev 依赖的是移动靶。

## 五、建议

**不换。** mang-app 继续作为标书桌面壳的唯一主线；GPUI 线保持观察。

### 不迁移但可吸收的三个点（低成本借鉴）

| 借鉴点 | 来源 | 落法 |
|---|---|---|
| Host trait 契约测试模式（原生壳与 headless 测试实现同一契约） | mdshell | mang-app 的 selftest.rs 可升级为显式 Host trait 面——IPC 契约零窗口可测 |
| `--smoke-sec N` 无头烟雾通道 | mang-shell / mdshell-native 的 xvfb_smoke | mang-app 加 `--smoke-sec`（开窗 N 秒自退、stdout 打标）——CI 里可做窗口级冒烟（Linux 跑 xvfb，Windows 跑真窗） |
| OpenProgress 渐进打开事件 | mdshell | 若标书预览大 PDF（数百页）出现首屏等待，可借鉴分段加载 + 进度事件面 |

### 重新评估的触发条件

1. mdcore mang-shell 合入主线**且**出 Windows 支持**且** GPL 问题有解（Zed ui/theme 出 Apache 替代、或 mdcore 用 Apache 面 gpui 自绘组件重写 UI 层）；
2. 标书预览出现**实测的**窗口层性能瓶颈（当前瓶颈证据为零）；
3. 产品决定开源（GPL 不再是阻断）。

三者同时成立再启动评估 v2。
