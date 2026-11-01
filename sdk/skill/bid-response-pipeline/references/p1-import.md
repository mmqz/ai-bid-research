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

## 文件分类表（kind 合法域 7 种）

| kind | 用途 | 备注 |
|---|---|---|
| 招标文件 | 主文件 + 技术规范书 | 条款抽取的主源 |
| 附件 | 资格证明类附属件 | |
| 图纸 | 图纸说明等技术附件 | |
| 补遗 | 补遗/澄清文件 | **改变条款面**——导入后必须增量重抽（P2） |
| 答疑 | 答疑纪要 | 同上 |
| 评分办法 | 评分细则 | 只进自评，不进正文 |
| 格式模板 | 投标文件格式件 | 七格式件的对齐参考 |

## 返回核对（数字核对纪律的落点）

```
per_file[]: file_id / sha256 / pages / tables / chunks / ocr_tier / parse_status
totals:     { files, chunks, parse_failed, partial }
```

**必须向用户复述**：`共导入 N 件 / 解析块 M / 失败 F 件`，并与预期比对。
`parse_failed > 0` → 返回 `E_PARTIAL`（**错误码不吞成功面**——data 里成功件
仍然有效）；失败件重试或经用户确认后放弃，放弃件内容全程禁引。

## OCR 档位含义（ocr_tier，确定性推导）

| 档位 | 含义 | Agent 动作 |
|---|---|---|
| `none` | 文本层在位，无需 OCR | 正常 |
| `ocr` | 无文本层但 OCR 已产出 | 提醒用户：扫描件内容来自 OCR，抽查关键页 |
| `pending` | 无文本层且 OCR 未产出 | 登记缺口；该件当前不可引，等 OCR 通道补齐 |
| `n/a` | 解析失败件，无内容面 | 走失败件处理（重试/确认放弃） |

## 失败降级话术（向用户）

> 「`<filename>` 解析失败（原因：<errors>）。已成功导入其余 N 件。你可以：
> ① 给我修复后的文件重试（sha256 幂等——同文件重传不重复计费）；
> ② 确认放弃该件——放弃后该件内容我不会引用，涉及条款将标为无源。」

## 幂等与补遗

- 同 sha256 重传 → no-op 返回原 file_id（`deduplicated: true`）——放心重试；
- 项目已过 IMPORTED 阶段追加新文件：允许，但会标 dirty 波及下游——补遗件
  走此路径，随后必须 `extract_requirements` 增量重抽。
