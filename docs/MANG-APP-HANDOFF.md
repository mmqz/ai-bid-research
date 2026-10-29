# mang 桌面壳 —— 接手文档

> 工作区：`E:\Code\ai\md\audit\ai-bid-research`
> 分支：`feat/mang-app`
> 目标：下周二前做出标书可用桌面工具

---

## 一、现状（已经有的）

```
✅ bidcore/crates/mang-app/        Rust 壳（tao + wry），已编译 1.8 MB
✅ web-ui/                          UI（从 mdcore/web 复制，20 个文件）
✅ bidcore/crates/bid-domain/       标书领域模型
✅ bidcore/crates/bid-tools/        store_io 读写 db.json
```

### 运行的 exe

```
bidcore\target\release\mang-app.exe
```

**当前 main.rs 已实现**：
- tao 窗口（1280×860，最小 800×600）
- wry WebView
- custom protocol `mang://localhost/` → 指向 UI 目录
- IPC handler（`with_ipc_handler`）

---

## 二、架构（重要）

```
┌──────────────────────────────────────────┐
│  mang-app.exe (Rust)                     │
│    ├─ tao 窗口                            │
│    ├─ wry WebView2                        │
│    │    └─ 加载 mang://localhost/index.html│
│    └─ IPC handler (JS → Rust)             │
│         ├─ 读 bidcore 数据                │
│         └─ 文件系统操作                    │
└──────────────────────────────────────────┘
         ↑
         │ 人的预览窗口（只读）
         │
    AI 子代理通过 MCP 调 bid-mcp 写数据
         ↓
┌──────────────────────────────────────────┐
│  bidcore（标书引擎）                       │
│    bid-domain  领域模型                   │
│    bid-tools   持久化（db.json）           │
│    bid-mcp     11 个 MCP 工具              │
└──────────────────────────────────────────┘
```

**关键：UI 是给人看的，AI 不经过 UI。**

---

## 三、要做什么（布局规格）

### 目标布局

```
┌─────────────────────────────────────────────────────────────────────┐
│ 文件  编辑  查看  生成  帮助              ─  □  ×                  │ 32px
├──┬───────────────┬─────────────────────────────────────────────────┤
│📁│ 最近打开       │ [招标文件.docx ×] [技术标.md ●]                  │ 34px
│🕐│  招标文件.docx │ ┌─────────────────────────────────────────────┐ │
│📝│  技术标.md     │ │                                             │ │
│★ │               │ │         文档内容（主视图，占满）             │ │
│  │ 文件夹         │ │                                             │ │
│  │  ▾ 项目A       │ │                                             │ │
│  │   ├ a.docx    │ └─────────────────────────────────────────────┘ │
│  │   └ b.xlsx    │                                                 │
│⚙ │               │                                                 │
├──┴───────────────┴─────────────────────────────────────────────────┤
│ 行 12, 列 3 │ 招标文件.docx │ 已保存 │ 2 秒前更新                   │ 26px
└─────────────────────────────────────────────────────────────────────┘
```

### 硬性要求

```
1. 整页面 100vh，编辑区 flex_1 + min_h_0
2. 绝对不出现整页滚动条
3. 只有编辑区内可以有滚动
4. 窗口最小 1024×640
5. 标题栏左上角不要多余标识
```

### 尺寸

```
菜单栏   32px
图标条   48px（📁 文件 / 🕐 历史 / 📝 改动 / ★ 收藏 / ⚙ 设置）
侧栏     260px（可折叠）
Tab 栏   34px
编辑区   flex（占满）
状态栏   26px
```

---

## 四、功能清单（分优先级）

### P0（下周二必须）

| 功能 | 说明 |
|---|---|
| 三栏布局 + 无滚动条 | CSS flex |
| 菜单栏 | 文件/编辑/查看/生成/帮助 |
| 左侧图标条切换侧栏 | 5 个按钮 |
| 文件树 | 需要 Rust 提供列目录 API |
| Tab 管理 | 打开/关闭/切换 |
| 空状态 | 最近打开列表 |
| 状态栏 | 行:列 / 文件名 / 保存状态 |
| 快捷键 | Ctrl+O/S/W/B/Tab |
| 工作区命令行 | `mang-app.exe --workspace D:\项目` |

### P1（之后）

| 功能 | 说明 |
|---|---|
| 文档渲染 | md 先做，docx 后做 |
| 类型路由 | .xlsx → 表格，.pdf → PDF |
| 来源标注浮层 | 复用 evidence_panel.ts |
| 版本历史 | git2 + 侧栏列表 |
| 右键菜单 | 打开/保存/复制路径/在文件系统显示/查看历史 |
| 文件树刷新 | notify + F5 |

### P2（更后）

| 功能 | 说明 |
|---|---|
| diff 对比 | md 层逐行对比（similar crate） |
| 表格渲染 | 复用 xlsx_grid.ts |
| PDF 渲染 | 复用 pdf_view.ts |

---

## 五、可复用的旧代码

| 文件 | 行数 | 怎么用 |
|---|---|---|
| `web-ui/src/pdf_view.ts` | 2128 | **整体能用**（PDF 渲染完整） |
| `web-ui/src/xlsx_grid.ts` | 1278 | **整体能用**（表格） |
| `web-ui/src/evidence_panel.ts` | 984 | **整体能用**（来源面板） |
| `web-ui/src/empty_state.ts` | 81 | 参考 |
| `web-ui/src/draft.ts` | 241 | 参考（草稿） |
| `web-ui/src/main.ts` | 3348 | **只参考，不要整个搬**（3348 行太杂） |
| `web-ui/src/canvas_view.ts` | 1681 | 暂时不用 |
| `web-ui/src/collab_channel.ts` | 471 | 暂时不用 |

**建议：新写骨架，按需搬组件。**

---

## 六、关键技术细节

### 1. custom protocol（已实现）

```rust
wry::WebViewBuilder::new()
    .with_custom_protocol("mang".into(), move |_id, req| serve_file(&root, req))
    .with_url("mang://localhost/index.html")
```

**为什么不用 file://**：file:// 下 fetch wasm 会被 CORS 拦截。

### 2. IPC 通道（已实现）

**JS 侧**：
```javascript
window.ipc.postMessage(JSON.stringify({ cmd: "load_state" }));
```

**Rust 侧**：
```rust
.with_ipc_handler(move |req: wry::http::Request<String>| {
    handle_ipc(&project, req.body())
})
```

### 3. 读 bidcore 数据

```rust
use bid_tools::store_io::StoreFile;

let store = StoreFile::new(project_dir);
match store.load() {
    Ok(Some((snapshot, parsed))) => { /* 有数据 */ }
    Ok(None) => { /* 首启空态 */ }
    Err(e) => { /* 加载失败 */ }
}
```

### 4. 构建 UI

```bash
cd web-ui
npm install
npm run build      # 产出 dist/
```

**然后 mang-app 读 `web-ui/dist`。**

### 5. 编译 mang-app

```bash
cd bidcore
cargo build --release -p mang-app
```

---

## 七、坑（我踩过的）

| 坑 | 说明 |
|---|---|
| **file:// 加载 wasm 失败** | 必须用 custom protocol |
| **GPUI 无法嵌 WebView** | WS_EX_NOREDIRECTIONBITMAP 图层冲突，所以用 tao/wry 不用 GPUI |
| **cargo 编译被回收** | 长时间编译用计划任务跑，或者前台 -j 4 |
| **exe 被占用无法覆盖** | 编译前先 Stop-Process |

---

## 八、契约（要和 Lead 对齐）

```
1. 不动 mdcore 仓
2. 独立 crate：bidcore/crates/mang-app/
3. UI 目录：web-ui/
4. 每阶段能跑起来再进下一阶段
5. 报完成贴真实截图
```

---

## 九、开工第一件事

```bash
# 1. 确认环境
cd E:\Code\ai\md\audit\ai-bid-research
git branch --show-current     # 应该是 feat/mang-app

# 2. 试跑现有的
BIDCORE\target\release\mang-app.exe

# 3. 如果窗口打开且显示旧 UI（五个 Tab 平铺），说明基础是通的
# 4. 开始改造布局
```
