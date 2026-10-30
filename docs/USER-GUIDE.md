# 用户手册——ai-bid-research 标书系统

> 面向使用者：怎么从一份招标文件，走到一份能递交的投标文件。
> 开发者文档见 `docs/`（架构决策 / 契约 / 模型）；本手册只讲操作。

---

## 一、系统做什么

```
招标文件（docx / pdf / xlsx）          你的证据（资质 / 业绩 / 证书）
        │                                       │
        ▼                                       ▼
   ① 导入 + 解析                        证据台账登记
        │                                       │
        ▼                                       │
   ② 条款抽取（资格/否决/评分/参数/商务/格式）    │
        │                                       │
        ▼                                       ▼
   ③ 响应矩阵（要求 × 证据绑定 × 响应状态）◄──────┘
        │
        ▼
   ④ 章节树 + 字数配额（大纲）
        │
        ▼
   ⑤ 逐章起草（AI 生成 / mock；引用回查防幻觉）
        │
        ▼
   ⑥ 校验（8 条确定性规则——防废标最后一道关）
        │
        ▼
   ⑦ 导出（docx / pdf；格式从招标文件提取——不硬编码；暗标脱敏）
```

**三个安全闸门**（缺一不可，全部通过才能导出）：

1. **矩阵门**：每条必选条款（资格/否决）都有响应（证据绑定或人工覆写）；
2. **审核门**：三道人工审核门过审（矩阵定稿 → 大纲定稿 → 导出前确认——
   审核人必须 `human:` 前缀，AI 不得自审自过）；
3. **校验门**：`validate_bid_document` 的 BLOCKER 清零（未响应否决项 /
   空章节 / 幻觉引用 / 失效证据——任何一条都是废标级风险）。

---

## 二、启动桌面壳（Windows）

```bat
:: 编译需要先跑 vcvars（否则报 link.exe not found）
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
cd /d E:\Code\ai\md\audit\ai-bid-research\bidcore
cargo run -p mang-app
```

桌面壳是 VSCode 风格的标书工作台：左侧文件树 / 中间编辑区 / 右侧面板；
带版本历史（自动快照 + AI 存版徽章）、来源标注浮层（点角标看条款原文）、
文档大纲面板（1330 页标书的导航刚需）。

---

## 三、跑一次完整流程（命令行 e2e）

```bash
# 一条命令：招标文件进 → 成品标书出（corpus-A 真件演示）
./scripts/e2e_bid/run_e2e.sh
```

跑的是什么（8 步，每步失败会定位到步号）：

| 步 | 做什么 | 产物在哪 |
|---|---|---|
| ① | 建项目 | 工作区 `db.json` |
| ② | 导入招标文件 + 证据 | 解析统计（页/块/表） |
| ③ | 抽取条款 | REQ 台账（六类分类） |
| ④ | 建大纲 + 矩阵 | 章节树（字数配额） |
| ⑤ | 绑证据 + 三道审核门 | 矩阵响应状态 |
| ⑥ | 逐章起草 | `chapters/*.json`（断点续跑） |
| ⑦ | 校验 | BLOCKER 清零报告 |
| ⑧ | 导出 | `out/投标文件-技术标.docx` |

**中途崩了怎么办**：直接重跑。断点续跑是结构保证——已完成的章一个字节
不重写、不重复生成（token 不浪费）。

---

## 四、MCP 工具面（AI Agent 调用）

12 个工具（`bid-mcp-stdio` 的 stdio 协议），按项目状态机有序开放：

| 工具 | 什么时候能调 | 干什么 |
|---|---|---|
| `create_bid_project` | 起步 | 建项目（external_key 幂等——重复调不重复建） |
| `import_tender_files` | 建项后 | 导入文件（sha256 幂等——同文件重导去重） |
| `extract_requirements` | 导入后 | 抽条款（重抽零新增——幂等合并） |
| `get_compliance_matrix` | 随时 | 看矩阵（要求×响应状态，游标分页） |
| `list_evidence` / `list_tasks` | 随时 | 证据台账 / 任务分工 |
| `draft_response_section` | 矩阵就绪后 | 单章起草（引用回查防幻觉；字数按配额 ±20%） |
| `validate_bid_document` | 起草后 | 校验（8 规则——BLOCKER 清零才可导出） |
| `export_bid_document` | 起草后 | 导出（docx/pdf；暗标模式公司名→「投标人」） |
| `save_version` | 随时 | 存版本快照（桌面壳「改动」面板可见） |
| `get_bid_project_state` | 随时 | 看状态 + 下一步允许哪些工具 |

**状态机**（越级调用会被拒——`E_STATE`）：

```
DRAFT → FILES_IMPORTED → REQUIREMENTS_EXTRACTED → MATRIX_READY
      → EVIDENCE_BOUND → DRAFTING → VALIDATING → REVIEWING
      → READY_TO_EXPORT → EXPORTED
```

**出错码**（按码分支，不解析消息文本）：

| 码 | 含义 | 该怎么办 |
|---|---|---|
| `E_STATE` | 状态机越级 | 先看 `get_bid_project_state`，按序推进 |
| `E_CONFLICT` | project_version 过期 | 重取状态后再调 |
| `E_NOTFOUND` | 对象不存在 | 检查 project_id / section_id |
| `E_UNPROCESSABLE` | 参数/前置不满足 | 看消息里的具体指引 |
| `E_PARTIAL` | 部分失败 | 逐件核对返回清单 |

---

## 五、格式规格（.mang/format-spec.json）

**格式从招标文件读，不硬编码**——每份标书要求不同。

```json
{
  "global": {
    "page": { "size": "A4", "margin": { "top": 35, "bottom": 30, "left": 28, "right": 26 } },
    "font": { "body": "仿宋_GB2312", "size": 12, "lineHeight": 1.5 },
    "header": { "enabled": true, "content": "{project_name}" },
    "footer": { "enabled": true, "content": "第 {page} 页 共 {pages} 页" }
  },
  "per_section": {
    "技术方案": { "font": { "body": "黑体", "size": 14, "lineHeight": 1.5 } }
  }
}
```

三级优先级：

1. **手写 spec**：工作区放 `.mang/format-spec.json` → 直接用（人工核过的最准）；
2. **条款提取**：招标文件里的格式条款（「正文用仿宋三号，行距 1.5 倍」）
   自动识别（中文字号表/字体族/行距/四边边距；补遗覆写主文件）；
3. **工程基线**：都没命中 → A4 / 仿宋_GB2312 小四 / 1.5 倍行距 / 35-30-28-26mm
   （水利标书主流口径）+ **响亮告警**（不静默降级——形式评审风险要人工确认）。

占位符：`{project_name}`（页眉）、`{page}` / `{pages}`（页脚页码域）。

---

## 六、暗标模式

导出时传 `profile`：

```json
{ "dark_bid": true, "bidder_name": "某某建设工程有限公司" }
```

效果：正文与章标题里的公司名 → 「投标人」（评审禁公司名的暗标规矩）。
**漏配 bidder_name 会响亮告警**（无名字可替不是错误，但不静默裸导）。

---

## 七、常见问题

**Q：跑 `cargo test` 有的测试显示 SKIP？**
环境缺依赖（soffice/pdftotext/真件）——SKIP 横幅会写明缺什么。缺件不是
业务失败；装上依赖重跑即测。

**Q：导出被拒「N 条必选未响应」？**
防废标门在工作。两条路：给必选条款绑证据（`bind_evidence`），或人工覆写
响应状态（`set_matrix_override`——不适用必须给理由）。

**Q：AI 生成的引用是编的？**
两层防线：draft 时引用回查（未知 REQ/EV 剔除 + 告警）；validate 时
`E_CITATION_GHOST` 规则（BLOCKER——评标查证无此件是废标级）。

**Q：想看每一步耗时？**
`cargo test -p bid-mcp --test scale_1330 -- --nocapture`——50 万字 252 章
全链逐步计时（实测全链 19s：起草 18s + 导出 0.3s）。

**Q：PDF 和 docx 哪个先出？**
docx 是真源（PDF 由 docx 转换）。PDF 需要 LibreOffice（`soffice`）——
缺失时 docx 通道不受影响。

**Q：重启后进度还在吗？**
在。工作区三件套：`db.json`（状态机）+ `chapters/*.json`（章内容）+
`export-manifest.json`(导出幂等)——重启全量恢复，已完成章不重跑。
