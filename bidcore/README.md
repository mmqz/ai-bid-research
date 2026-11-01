# bidcore —— 标书系统 Rust 核心（M1 里程碑实现）

> 语言决策（AD-08）：**核心全部 Rust，不留 Python 双语言契约**。文档引擎（mdcore）同为 Rust，
> 同语言消除 FFI/IPC 边界；运行产物契约由 serde 类型共享——**后端与判分器共用同一组编译期类型，
> schema 漂移在编译期即被捕获**（这是对「双语言 docstring 冻结契约」的根治）。
> Python 仅保留两件存档用途：夹具生成器（`scripts/fixture_gen.py`，44 文件已 sha256 冻结（corpus-A 39 + corpus-R 结构层 5），
> 生成器是数据出处）与研究文档一致性检查器（`scripts/xref_check.py`）——均不进产品链路。

## 工作区结构

| crate | 职责 | 对应设计文档 |
|---|---|---|
| `bid-domain` | 领域对象 + 权威状态机（E_STATE 越级防护 / 幂等 / content_version）| docs/bid-domain-model.md |
| `bid-parse` | 确定性解析：DOCX(zip+quick-xml) / XLSX(calamine) / PDF(poppler 适配) / OCR(tesseract 适配) + 条款抽取 | mcp-contract §3「确定性先行」层 |
| `bid-kb` | AdapterA：结构化切块（内容寻址 chunk_id）+ 精确通道检索（四级确定性 tie-breaker）| mcp-contract 附录 A |
| `bid-tools` | 工具薄壳 + 场景运行器 + **db.json/trace.jsonl serde 契约**（`schema.rs`）+ **导出链**（六表/七格式件/负偏离红标/大写金额/行内粗体——#117/#122）+ 起草 readiness 闸（#110/L3）+ 质量报告（B3 三维度，#131 接线） | mcp-contract §1~§9 |
| `bid-grade` | 判分器：七判分项 + 里程碑范围分级 + 破坏注入自测 | verification-plan §2.2 |

## 每轮测试

```bash
cd bidcore
cargo test --workspace          # 单测 + corpus-A 探针 + 25 查询探针 + 四场景端到端 + 判分器自测门禁
cargo run -p bid-grade -- --self-test        # 判分器自测（完美库 + 11 类破坏注入 + 类型化 schema 拒绝）
cargo run -p bid-grade -- --all-runs ../runs # 判分 runs/ 下全部场景产物
```

M1 端到端场景：**TC-01**（导入+抽取，G1 召回 100%）/ **TC-02**（25 精确查询×5 重放一致）/
**TC-07**（sha256 幂等 no-op）/ **TC-11**（状态机越级 E_STATE）——全部在真实 corpus-A 夹具上运行，
产物写 `runs/<scenario>/{db.json, trace.jsonl, report.json}`（确定性，可重放比对）。

双轨 e2e（#131 起）：合成面 `bid-mcp/tests/e2e_artifacts.rs`（synth-tender 115 REQ +
十项深断言）+ 真实面 `bid-mcp/tests/real_corpus_e2e.rs`（3 件水科 PDF 全链 + GT 三通道
红线 L1 0.9717 / P 0.9636，产物 artifacts/real-corpus-e2e/）——两轨都绿才算绿。

## 判分纪律（从 grade.py 完整移植并强化）

- 反标记回声：召回只认对齐器（条款号双因子 / 锚点互含含 3× 长度上限 / 唯一参数值），
  自报 gt_id 无法对齐的计 unverified；语料零 GT 标记。
- OCR 噪声容忍：锚点对齐在 N2 归一空间（NFKC + 去空白 + 去标点）进行。
- 核心断言不可跳过：路径不可解析即失败；未执行 ≠ 通过（幂等/精确块缺失判 FAIL）。
- 里程碑范围分级：M1 场景不判 G3/G4/G6（M2+ 交付面）——OUT_OF_SCOPE 显式留痕，
  verdict 只由被判定维度决定（范围外不假绿也不误红）。
- 类型化 schema 拒绝：伪装类型字段（如 unanchored_claims=false）在反序列化期即被拦截。

## 外部引擎适配器（组件版本矩阵钉住点，test-plan §5.2）

- **PDF 文本层**：poppler `pdftotext -layout`（外部进程适配器）
- **PDF 扫描层**：poppler `pdftoppm` 渲染 → tesseract `chi_sim`（OCR 分档）
- 引擎版本变化属组件矩阵差异，不属本层自由度；解析失败/无文本层绝不静默（诚实状态）。

## M2+ 路线（按 test-plan MVP 顺序）

矩阵 + 双通道 search_evidence（semantic 通道）+ 证据有效期断言（M2）→
draft_response_section 经文档内核稳定接口（M3）→ validate 确定性规则（M4）→
主 Skill 全阶段 + 恢复协议（M5）。引擎 P0 缺口（G1 目录域 / G2 文档级样式 / G3 暗标元数据清零）
见 docs/doc-engine-audit.md，待引擎团队排期。
