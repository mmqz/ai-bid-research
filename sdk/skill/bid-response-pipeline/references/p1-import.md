# P1 导入细则（import_tender_files）

> 契约依据：`docs/bid-mcp-contract.md` §2。Agent 不做解析与切块，只核对返回
> 统计并重试失败件（R-01）。

## 调用形

```json
{
  "project_id": "…",
  "files": [
    { "upload_ref": "<本地文件路径>", "filename": "A-招标文件.docx", "kind": "招标文件" },
    { "upload_ref": "<路径>", "filename": "E-盖章页-扫描件.pdf", "kind": "附件", "expected_pages": 3 }
  ]
}
```

`upload_ref` 是本机文件路径（服务端直读——MCP 宿主与 bid-mcp 同机时直接给
绝对路径）。`kind` 可省（服务层自动判），给了走壳校验。

> **路径围栏（#183）**：`upload_ref` 必须落在受信根内——启动 server 时
> `BID_FS_ROOT` 声明的导入根，或 `BID_WS` 工作区。词法 `..` 分量会被
> 预拒（给 canonical 绝对路径最稳）；文件在别处时先拷入工作区。越界/
> symlink 逃逸/超限（单件 500MB·单批 64 件·累计 2GB）→ `E_UNPROCESSABLE`
> 整批拒绝（零副作用——修正后整批重传即可）。

## 文件分类表（kind 合法域 7 种）

| kind | 用途 | 备注 |
|---|---|---|
| 招标文件 | 主文件 + 技术规范书 | 条款抽取的主源 |
| 附件 | 资格证明类附属件 | 无条款面 |
| 图纸 | 图纸说明等技术附件 | 无条款面 |
| 补遗 | 补遗/澄清文件 | **改变条款面**——导入后必须增量重抽（P2） |
| 答疑 | 答疑纪要 | 同上 |
| 评分办法 | 评分细则 | 只进自评，不进正文 |
| 格式模板 | 投标文件格式件（含招标包 `附件/` 目录下的官方模板——声明函/偏离表/承诺书等，#271） | 无条款面（不产 REQ/不进矩阵/不阻塞起草）；解析后在 `forms_manifest` 清单可见；起草时格式类 REQ 带 `params.form_file_id`，可经 get_document_chunks 读模板结构 |

## 返回核对（数字核对纪律的落点）

```
per_file[]: file_id / kind(#271 回显) / sha256 / pages / tables / chunks / ocr_tier / parse_status
totals:     { files, chunks, parse_failed, partial }
forms_manifest?: { count, items:[{file_id, name, parse_status, pages, tables}] }
            ← #271 格式件清单（kind=格式模板；无格式件时键省略）
addendum_readiness: { total, ready, not_ready:[{file_id, name, readiness, waived, reason}] }
            ← #184 补遗分级就绪报告（仅补遗/澄清类件）；非 READY 件另发
              ADDENDUM_NOT_READY 告警点名 + 指路 record_addendum_waiver
```

**必须向用户复述**：`共导入 N 件 / 解析块 M / 失败 F 件`，并与预期比对。
`parse_failed > 0` → 返回 `E_PARTIAL`（**错误码不吞成功面**——data 里成功件
仍然有效）；失败件重试或经用户确认后放弃，放弃件内容全程禁引。

`.doc` 件（Word 97-2003 旧格式）的返回面（#270）：本机有转换器（Word COM /
LibreOffice 任一）时引擎**自动转 .docx 解析**——`per_file[].warnings` 会带
`CONVERTED_FROM_DOC` 标注（sha256 记原件，用户无感）；`.doc` 改名 `.docx`
的撒谎件报「OLE2 容器/旧格式」引导（不是「容器损坏」）。见下节处理指引。

## 遇到「不支持 .doc 且无转换器」时（#270）

导入 `.doc` 报错含「不支持直接解析 .doc（Word 97-2003 旧格式），且本机
未检测到可用的转换器」+ 两项检测结果（Word COM / LibreOffice）时：

1. **先确认自己的环境**——检查本机是否有 Word 或 LibreOffice；
   有 → 自己执行转换命令，然后导入转换后的 .docx（新 sha256 即新档）。
2. **自行转换**（任选其一）：
   ```powershell
   # Windows + Office（PowerShell 调 Word COM）
   $w = New-Object -ComObject Word.Application
   $d = $w.Documents.Open("C:\path\file.doc")
   $d.SaveAs2("C:\path\file.docx", 16)
   $w.Quit()
   ```
   ```bash
   # LibreOffice（Windows/macOS/Linux 通用）
   soffice --headless --convert-to docx --outdir /tmp "file.doc"
   ```
3. **无法转换时，如实告知用户**：
   - 明确说「这个文件是 .doc 旧格式，当前环境无法转换」；
   - 给出用户自己转换的方法（Word 打开 → 另存为 → .docx，或安装
     LibreOffice 后重新导入——引擎将自动转换）；
   - **不要**：跳过该文件继续做标、编造该件内容、假装已处理。

## OCR 档位含义（ocr_tier，确定性推导）

| 档位 | 含义 | Agent 动作 |
|---|---|---|
| `none` | 文本层在位，无需 OCR | 正常 |
| `ocr` | 无文本层但 OCR 已产出 | 提醒用户：扫描件内容来自 OCR，抽查关键页 |
| `pending` | 无文本层且 OCR 未产出 | 登记缺口；该件当前不可引，等 OCR 通道补齐。**补遗/澄清类件同时使 addendum_readiness 报 OCR_PENDING（#184）——阻塞 P5 起草，见下** |
| `n/a` | 解析失败件，无内容面 | 走失败件处理（重试/确认放弃）。**补遗/澄清类件同时阻塞 P5 起草（#184），见下** |

## 补遗分级门（#184）

任一补遗/澄清件不处 READY（readiness=FAILED 解析失败 / OCR_PENDING 零文本）
→ `draft_response_section` 被 MissingAddendum 拒，拒稿理由逐件点名文件名与
原因——**部分失败也算**（3 份坏 1 份也拦：坏的那份可能正是改控制价/截止
时间的，按旧条款做标是废标）。出口两条：

1. **修复重传**可解析版本（新 sha256 即新档，阻塞自然消失）；
2. **用户逐件确认后放行**：`record_addendum_waiver`（file_id + reason +
   宿主签发 receipt——签域 `addendum_waiver:{file_id}` 件级绑定，旧
   `principal` 参数显式拒（#182）；逐件留底审计，非全局布尔；已确认件在
   not_ready 清单里 waived=true，不再阻塞但仍可查）。

## 失败降级话术（向用户）

> 「`<filename>` 解析失败（原因：<errors>）。已成功导入其余 N 件。你可以：
> ① 给我修复后的文件重试（sha256 幂等——同文件重传不重复计费）；
> ② 确认放弃该件——放弃后该件内容我不会引用，涉及条款将标为无源。」

**补遗/澄清类件不适用话术 ②**（#184）：坏补遗不能「确认放弃」了事——
它可能改了控制价/截止时间。要么修复重传，要么用户逐件确认
`record_addendum_waiver` 留底放行（话术：「这份补遗我无法读取。若你已
人工核对原件且确认条款无实质变化，我可以记录你的逐件确认后继续；否则
请提供可解析版本。」）。

## 幂等与补遗

- 同 sha256 重传 → no-op 返回原 file_id（`deduplicated: true`）——放心重试；
- 项目已过 IMPORTED 阶段追加新文件：允许，但会标 dirty 波及下游——补遗件
  走此路径，随后必须 `extract_requirements` 增量重抽。
