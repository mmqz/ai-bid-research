# 部署与依赖说明

## 推荐包内容

```
mang-bid/
  bin/
    bid-mcp-server.exe    标准 MCP（JSON-RPC 2.0）
    bid-mcp-stdio.exe     裸 JSON 行协议
  skill/
    bid-response-pipeline/   可直接导入的 Skill
  docs/
    bid-mcp-contract.md      工具契约单一真源
    agent-integration.md     外部 Agent 集成说明
    DEPLOYMENT.md            本文件
  mcp-config/
    claude-desktop.json
    generic.json
  README.md
  KNOWN-ISSUES.md
```

**体积**：约 30 MB（两个 exe 静态链接运行时，无需安装 .NET / Rust / Python）

---

## 目标机器要求

### 必需

| 项 | 说明 |
|---|---|
| Windows 10 及以上 x64 | — |
| 约 100 MB 磁盘空间 | 含工作区数据 |

**不需要**：Rust、.NET、Python、Node.js

### 可选（按功能需要）

| 功能 | 依赖 | 没有时的表现 |
|---|---|---|
| **导出 PDF** | Microsoft Word **或** LibreOffice | 只能导出 DOCX |
| **PDF 导入（文本层）** | 内置纯 Rust 兜底（pdfium/poppler 缺席时的第三条路） | 复杂 CID 编码解不出时显式拒收并点名原因（见下表），不产出乱码 |
| **PDF 导入（扫描件）** | poppler（`pdftoppm`）+ tesseract + chi_sim 语言包 | 扫描件导入失败 |
| **DOCX/XLSX 导入** | 内置 | 无需依赖 |

**PDF 纯 Rust 兜底的编码能力边界**（#200 登记——遇到时错误信息会点名
具体原因并给出绕过路径，fail-loud 不硬解）：

| PDF 编码形态 | 根因 | 绕过 |
|---|---|---|
| GBK-EUC-H/V、GB-EUC、GBpc-EUC（简体） | 码值是 GBK 码值非 Unicode，需内置映射大表 | 装 pdfium / poppler+poppler-data / 另存 DOCX 导入 |
| B5pc-H/V、ETen-B5、CNS-EUC（繁体） | 码值是 Big5 码值非 Unicode，同上 | 同上 |
| Adobe-GB1/CNS1/Japan1/Korea1 CID 序号（无 ToUnicode） | 码值是 Adobe 注册表字形编号，需 CIDToUnicode 映射大表 | 同上 |
| PDF 1.5+ 对象流（/ObjStm）里的字体字典 | 兜底的字典层扫描拿不到编码声明 | 同上（pdfium/poppler 不受影响） |

---

## 各依赖的安装方法

### 1. Word 或 LibreOffice（导出 PDF 用）

**推荐：Microsoft Office**（投标用户通常已有）
```
· 转换速度 2-5 秒
· 保真度最高（Word 自己渲染）
· 无需额外安装
```

**备选：LibreOffice**（免费开源）
```
· 下载：https://www.libreoffice.org/download/
· 安装后确保 soffice.exe 在 PATH 中
· 转换速度 5-15 秒
```

**都没有**：导出 DOCX 后，用 Word/WPS 手动「另存为 PDF」。

### 2. poppler + tesseract（PDF 扫描件 OCR）

**poppler for Windows**：
```
· 下载：https://github.com/oschwartz10612/poppler-windows/releases
· 解压后把 bin 目录加入 PATH
· 提供 pdftotext / pdftoppm / pdfinfo
```

**tesseract**：
```
· 下载：https://github.com/UB-Mannheim/tesseract/wiki
· 安装时勾选 Chinese (Simplified) 语言包
· 或手动下载 chi_sim.traineddata 放到 tessdata 目录
```

验证：
```powershell
pdftotext -v
tesseract --list-langs    # 应包含 chi_sim
```

---

## 关于 VC++ 运行库

exe 已**静态链接** MSVC 运行时（`-C target-feature=+crt-static`），
因此**不需要**目标机器安装 Visual C++ 可再发行组件。

```
验证方法：
  在没有装 VC++ 运行库的干净 Windows 上直接双击 exe
  应该能正常启动（不再报 0xC0000135 找不到 VCRUNTIME140.dll）
```

---

## 快速验证（30 秒）

```powershell
# 1. 解压后进入 bin 目录
cd mang-bid\bin

# 2. 直接喂一条 JSON
echo '{"tool":"get_bid_project_state","args":{}}' | .\bid-mcp-stdio.exe
```

**期望输出**：
```json
{"ok":false,"tool":"get_bid_project_state","error":{"code":"E_NOTFOUND","what":"project 未创建"}}
```

能回 JSON 就说明 exe 正常（`E_NOTFOUND` 是对的——还没建项目；#325：契约字面量）。

---

## 环境自检

检查各依赖是否到位：

```powershell
# PDF 转换
$word = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\WINWORD.EXE' -EA SilentlyContinue
if ($word) { "Word: 已安装" } else { "Word: 未安装" }
$lo = Get-Command soffice -EA SilentlyContinue
if ($lo) { "LibreOffice: 已安装" } else { "LibreOffice: 未安装" }

# PDF 解析
if (Get-Command pdftotext -EA SilentlyContinue) { "poppler: 已安装" } else { "poppler: 未安装" }
if (Get-Command tesseract -EA SilentlyContinue) { "tesseract: 已安装" } else { "tesseract: 未安装" }
```

---

## 常见问题

### Q: 导出 PDF 报「需要 Microsoft Word 或 LibreOffice」

说明两个都没检测到。检查：
```
1. Word 是否真的装了（打开 Word 确认能启动）
2. LibreOffice 的 soffice.exe 是否在 PATH 里
```

### Q: Word 转换超时

```
原因：Word 被其他文档占用，或有弹窗未关闭
处理：关闭所有 Word 窗口后重试；转换会在 30 秒后超时放弃
```

### Q: PDF 导入报「疑似 CJK 乱码」

```
说明：PDF 用了特殊 CID 字体，内置解析器解不出
处理：安装 poppler 后重试；或先用其他工具转成 DOCX
```

### Q: 扫描件 PDF 导入失败

```
处理：安装 tesseract + chi_sim 语言包
注意：OCR 较慢，100 页约 2-5 分钟
```