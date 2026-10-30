# 接手须知——#11 的 2026-10-30 大版本更新（#59 主线收尾后）

> 本文件是 issue #11「新会话启动接手须知」的仓库面沉淀。#59 六个阶段
> （draft/format-spec/export/validate/e2e/规模/PDF）落地后的新现状、
> 新架构决策、新踩坑。**先读这个，再读 #11 原文（历史背景）。**

---

## 一、当前状态快照（2026-10-30）

```
引擎核心能力（已达标）：
  L1 抽取召回 0.9338 / L2 否决分类召回 0.8413 / P 精确率 0.8762

12 个 MCP 工具（全部接线，search_evidence 降级可选除外）：
  ✅ create / import / extract / matrix / list×2 / state / save_version
  ✅ draft_response_section（#60）     ← 单章起草（mock + 引用回查）
  ✅ export_bid_document（#62/#66）    ← docx + PDF + 暗标
  ✅ validate_bid_document（#63）      ← 8 规则防废标
  ⬜ search_evidence                   ← 降级可选（查表面够用）

端到端（#64）：corpus-A 真件 8 步全链 0.83s——招标文件进成品 docx 出
规模（#65）：50 万字 252 章全链 19s；导出 0.3s；内存峰值 44MB
PDF（#66）：298 页 2.7s（LibreOffice headless）
收官测验（#80 第一批/PR #81）：synth-tender 合成夹具 7 坑 + 95→98 条答案
收官测验（#80 第二批）：真实生成通路（人肉 LLM 中继）+ e2e 十件套产物落盘
桌面壳：十阶段全完成（#42）+ 大纲面板（#54）+ 公式面（#53）
```

**能力边界（诚实面）**：

- AI 生成的 **mock 是默认面**（确定性模板——e2e 能跑但不出真内容）；
  **真实 LLM 通路已验证跑通**（#80 第二批，2026-10-30）：`RealGenerator`
  （LlmConfig → HttpTransport/ureq → envelope 容错解析 → 用量台账）经
  「人肉 LLM 中继」全链实测——开发者本人作为补全作者（无 API key），
  `scripts/e2e_bid/run_real_llm_path.sh` 两相位（capture 抓 prompt → 人写
  正文 → serve 回灌全链）。接真 API 只需 `.mang/llm.json` 配 api_base/
  api_key/model（#80 第二批实测详见 artifacts/e2e-real-path/latest/）；
- 图片/表格嵌入导出面未做（PDF 管线承载，无夹具验证）；
- `search_evidence` 未做（Lead 裁决：标书场景是查表不是检索——素材库
  上来再启）。

---

## 二、新架构决策（#59 批次的）

### D1 · 导出的硬闸门在工具内部，不在状态机面预拦

`export_bid_document` 的状态机面是 Drafting+（宽），真正的门是工具内部的
**双前置检查**（矩阵 blocking 行清零 + 审核 BLOCKER findings 清零）。

**为什么**：阶段 3 的 validate 工具落地前，项目没有走
Validating→Reviewing→ReadyToExport 的推进通道——状态机预拦会把 e2e 锁死。
闸门语义由域数据（矩阵/review）承载，状态机只管流程序。

### D2 · validate 是纯只读诊断，不是门

校验产报告不改稿不落库；BLOCKER 的解除走既有面（矩阵覆写/证据补绑/
章节 draft）。export 消费同一真源——**报告是导数不是源数据**。

### D3 · OutlineNode.content 与 ChapterAssembler 双真源

draft 写两处（内存面 `OutlineNode.content` 供 MCP 即时读 + 磁盘面
`chapters/{sec}.json` 供断点续跑/导出），**单一提交路径同步写**不分叉。
CAS 语义：同内容 no-op（attempt 不推进）。

### D4 · format-spec 三级优先级（不硬编码铁律）

手写 spec（`.mang/format-spec.json`）> 条款提取（中文字号 GB/T 表 +
字体族 + 行距 + 边距；补遗后到优先）> 工程基线（A4/仿宋小四/1.5 倍 +
**响亮告警**）。serde 面兼容 Lead 样例原文（驼峰 lineHeight 别名 +
per_section 嵌套 font 包裹）。

### D5 · PDF 的幂等由跳过路径承载

soffice 的 PDF 二进制不确定（CreationDate/ID 每次变）——「产物一个字节
不重写」不能靠 sha 对账，靠 staging docx 的 sha 标记跳过重转。

### D6 · 项目状态推进通道补全

`advance_project_status`（next_allowed 裁决 + 幂等 no-op）；
`build_matrix` 首建推进 MATRIX_READY。此前 MatrixReady 起靠测试
force_status 绕过状态机——现在闭环。

---

## 三、新踩的坑（按发现顺序，都有测试锚定）

### K1 · soffice 的 profile URL 两斜杠

`file://{path}` 是 host 形式（host=path）——soffice **静默忽略**该参数
→ 并发实例共享默认 profile → 锁冲突（四个并发测试三个「成功但无产物」）。
正确：三斜杠 `file:///{abs}`。**教训：URL 参数不报错不代表生效。**

### K2 · conv_out 住在 profile 里，先删 profile = 删产物

`remove_dir_all(profile)` 删掉了转换产物目录——soffice 退出码 0、stdout
有 convert 消息、产物就是不在。**教训：清理顺序=产物搬移之后。**

### K3 · 测试夹具的状态门（三次踩）

`set_requirements` 只在 FilesImported/RequirementsExtracted 态可调——
夹具忘 `upsert_file` 或二次 set 都会 E_STATE。**教训：夹具要从
`BidService::open` + 完整管线走，别抄单步。**

### K4 · 幂等键含 content_version 的层错位

draft 成功推进 content_version → 幂等键变 → 第二次不命中键层但命中
manifest 层（no-op 直返）。**两层幂等是有意的**：键层（稳定态快路径）+
manifest 层（状态推进后的等价判定）。测试要分步断言。

### K5 · mock 生成器的填充句不能有裸承诺词

「确保…」出现在无锚填充句 → unanchored_claims 误报。mock 要结构性
零无锚（该指标留给真 AI 面才有信号）。

---

## 四、必读文档（更新序）

| 文档 | 为什么读 |
|---|---|
| `docs/USER-GUIDE.md`（新） | 用户视角全流程 + 三闸门 + 常见问题 |
| `docs/bid-mcp-contract.md` | 12 工具契约（§6/§7/§8 已全部落地） |
| `docs/bid-domain-model.md` | 状态机 + 词表（xref 真源） |
| `docs/architecture-decision.md` | AD-07 责任边界（引擎管通用、标书管业务） |
| issue #59 + 六 PR（#60-#66） | 本批次的验收证据与设计决策 |

## 五、五条纪律（不变，加一条）

1. 走 PR 不直推 main；开 PR 确认 `baseRefName == main`
2. commit 后验证推送：`git rev-parse HEAD` == `origin/<branch>`
3. 不许「读了就信」——说"接上了"先跑对账命令
4. 不许新增引擎语义仿真
5. 盘点/分析类任务不改 Rust 源码
6. **（新）stacked PR 的冲突按并集语义解**——同块多 PR 修改时，取语义
   并集（如 allowed_tools 表），不留任何一方的删减
