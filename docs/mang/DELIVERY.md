# mang 桌面壳 —— 交付说明（VSCode 风格文档工作台）

> 交付日期：2026-10-29 · 分支 feat/mang-app
> 规格依据：docs/mang/SPEC-issue42.md（issue #42）· 接手文档 docs/MANG-APP-HANDOFF.md · IPC 契约 docs/mang/IPC-CONTRACT.md

---

## 一、这一版做了什么

把 web-ui 从「五个 Tab 平铺」重写成 VSCode 风格的文档工作台，并且已经跑在真实桌面壳里：

| 区域 | 尺寸 | 状态 |
|---|---|---|
| 菜单栏（文件/编辑/查看/帮助 + 工作区 + 主题） | 32px | 完成 |
| 图标条（文件/历史/改动/收藏/设置 = 切侧栏） | 48px | 完成 |
| 侧栏（可折叠 Ctrl+B、可拖宽、宽度记忆） | 260px | 完成 |
| Tab 栏（打开的文件，非文件类型） | 34px | 完成 |
| 编辑区 | flex + min-height:0 | 完成 |
| 状态栏（行号:列 / 文件名 / 大小 / 更新时间 / 主题 / 连接状态） | 26px | 完成 |

**硬性要求全部满足**：整页 100vh、页面永不出现滚动条（html/body overflow:hidden；1024x640 ~ 2560x1440 实测 scrollHeight === clientHeight），滚动只发生在编辑区/侧栏内部。

---

## 二、架构（两层）

```
mang-app.exe (tao+wry)
  +-- mang://localhost/mang/index.html     <- 壳直接加载这一页（不是 iframe）
        app.js                              UI（无框架，纯 DOM）
        ^ window.ipc.postMessage(JSON)
        v 壳注入 window.__ipcResolve(id, resp, ok) / window.__ipcEvent(ev)
        渲染器（按需 script 注入，不打进主包）：
          docx_view.js   自研 ZIP + DOCX -> HTML 纸面渲染
          xlsx_view.js   自研 ZIP + XLSX 解析 + 虚拟滚动表格
          pdf_view.js    pdf.js v6（worker + cmaps/standard_fonts/wasm）
```

- **不用 iframe**：wry 的注入脚本只在顶层文档的隔离世界里可用；UI 页放在顶层，自己持有 window.ipc，dist/index.html 退化为可选宿主页。
- **自绘标题栏**：壳用 with_decorations(false) 去掉系统标题栏，菜单栏即标题栏（左上角只有 11px 的 mark，符合「不要多余标识」）；拖动 / 双击最大化 / 贴边吸附 / 四边缩放交给 WM_NCHITTEST（返回 HTCAPTION、HTLEFT 等）由系统完成，UI 只在 mousedown 时报一次「这一条是空白标题区」，菜单项和按钮不会被当成拖动区；右侧三个窗口按钮走 window.action。
- **渲染器不用 ESM 动态 import**：用普通 script 注入 IIFE bundle，每个 bundle 尾部把模块命名空间挂到 window.__mangRenderers[name]（动态 import 在部分 webview / CSP / 自定义协议组合下会被拦）。
- **构建**：cd web-ui && npm run build（esbuild，web-ui/mang-build.mjs），产物 web-ui/dist/。

---

## 三、功能（对照 issue #42）

已实现：

- 文件类型自动路由：.docx/.doc -> 文档；.xlsx/.xls -> 表格；.pdf -> PDF；.md -> Markdown 预览（可切源码）；其它 -> 文本（行号/换行）
- Tab 管理：打开/切换/关闭（× / 中键 / Ctrl+W）、关闭其他/全部、未保存标记位、Tab 右键菜单、Ctrl+Tab / Ctrl+1…9、溢出横向滚动
- 空状态：欢迎页 + 最近打开（图标、相对时间、路径），点击即打开
- 侧栏五个视图：资源管理器（目录树，目录在前、子项计数、展开状态）、历史（最近打开）、改动（占位，等版本历史接入）、收藏（占位）、设置（主题/状态栏/换行/行号/工作区/运行模式）
- 菜单栏：文件/编辑/查看/帮助，含快捷键、勾选态、禁用态；键盘上下/Enter/Esc 可用
- 快捷键：Ctrl+O、Ctrl+K Ctrl+O（和弦）、Ctrl+S、Ctrl+W、Ctrl+B、Ctrl+Tab、Ctrl+1…9、Ctrl+Shift+E/H/G、Ctrl+,、F5、Ctrl+R
- 状态栏：随渲染器上报（xlsx 显示「工作表 行:列 单元格内容」、pdf 显示「第 N/M 页 · 缩放%」、文本显示「行/列」）；切换 Tab 时整份跟随，不会串上一个 Tab 的光标位置
- 主题：跟随系统 / 浅色 / 深色（渲染器按 theme 渲染）
- 工作区：mang-app.exe --workspace D:\项目（或 mang-app.exe D:\项目），目录树来自壳的 fs.tree
- 大文件传输：>1.5MB 的 IPC 响应落临时文件，UI 用相对 '/__ipc/<file>' 一次性取回（仅当 result.inline === false 且是纯文件名）
- 拖放：壳侧 WM_DROPFILES -> {ev:'drop', paths}，UI 直接开 Tab

未做（下一阶段，规格里属于 P1/P2）：

- 编辑与保存（当前只读预览；保存动作给出明确提示）
- 版本历史 / diff / 来源标注浮层（.mang/history 数据面由壳提供，UI 侧「改动」面板留位）
- 右键菜单里的「生成 docx / 生成 PDF」流水线
- 文档内查找

---

## 四、验证证据

真实桌面壳（mang-app.exe --workspace fixtures/corpus-A，CDP 驱动真实 WebView2）：

| 文件 | 结果 |
|---|---|
| files/A-招标文件.docx | 31 页纸面渲染，标题层级/表格正确，状态栏 41 KB |
| files/B3-图纸目录.xlsx | 24x6 网格、合并单元格、冻结表头、公式栏、选区统计 |
| files/B1-设备清单.pdf | 2 页 pdf.js 渲染，工具栏（页码/缩放/适应宽度/整页/旋转） |
| .md / .json | Markdown 预览（可切源码）/ 带行号文本 |

截图：web-ui/mang-shots/real-window.png、real-A-招标文件-docx.png、real-B3-图纸目录-xlsx.png、real-B1-设备清单-pdf.png

布局矩阵（headless，每档都断言无页面滚动条）：1024x640 / 1440x900 / 1920x1080 / 2560x1440 -> 菜单 32 / 图标 48 / 侧栏 260 / Tab 34 / 状态栏 26，全部命中。

壳侧自检：cargo test 26/26、--selftest 23/23、mang_ipc_smoke.mjs 13/13（真实 WebView2 窗口）。

---

## 五、目录

```
web-ui/
  mang/index.html      工作台页面（壳直接加载）
  mang/styles.css      全部样式（主题变量 + 布局铁律）
  mang/app.js          UI 逻辑（壳桥 / 状态 / Tab / 侧栏 / 菜单 / 渲染器路由）
  mang/host.js         可选宿主页脚本（iframe 形态时才用）
  index.host.html      可选宿主页（构建到 dist/index.host.html，默认不用）
  mang-build.mjs       构建（esbuild）
  scripts/serve.mjs    开发用静态服务（node scripts/serve.mjs 8130）
  src/mang/docx_view.ts   DOCX 渲染器
  src/mang/xlsx_view.ts   表格渲染器
  src/mang/pdf_view.ts    PDF 渲染器（pdf.js）
  src/*（旧的 pdf_view.ts / xlsx_grid.ts / evidence_panel.ts 仅作参考，未参与构建）
```

---

## 六、已知限制

- docx：按分页符切页（不按内容高度真实排版），页眉页脚/脚注/批注/修订标记不渲染，EMF/WMF 图片不显示
- xlsx：不计算公式（显示缓存值），不支持 .xls / 加密 / 图表 / 图片
- pdf：不支持链接注解与表单域；字体资源缺失时非嵌入字体会降级
- 所有文档只读：保存/另存为给出明确提示，等编辑阶段接入
## 七、QA 复核（外部 QA 代理，Q3 轮）

- 布局硬指标：1024x640 / 1280x800 / 1440x900 / 1920x1080 / 2560x1440 / 800x600，以及 1.25x 缩放下 —— 全部无整页滚动条，六个区域尺寸全部命中（32/48/260/34/flex/26）。
- 深色主题文档区：修掉「黑底黑字」—— md 的 th/pre/code/blockquote、视图工具条改用独立的纸张 token，深色下仍是白纸灰阶。
- 侧栏拖拽手柄：从 3px 可见区改成 6px 命中区，真机可抓（260 → 332 实拖验证）。
- 菜单弹层：无图标菜单收起空图标列、勾选列加宽、深浅色 hover 一致、键盘焦点不被边框裁切。
- 无障碍：activitybar 补 role=tablist、侧栏标题改 h2、树行/列表项补 focus-visible 环、关闭按钮聚焦时可见。
- 已修复：空状态按钮被 panes 遮挡点不到（z-index）、树点击相对路径打不开、菜单键盘导航失效、侧栏收起后 activitybar 仍标 aria-selected。
- 文件树右键补齐规格 §七 的四项：生成 docx / 生成 PDF / 保存版本快照 / 查看历史（后三项为占位，数据面已就绪）。
