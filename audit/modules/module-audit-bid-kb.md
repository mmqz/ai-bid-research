# 模块审计 A2-2：bid-kb（M1 确定性切块 + 精确通道检索 + S-3 索引分片）

- 审计代理：4-a2（ai-bid-research 全仓审计 V2，Task ID: 4-a2）
- 基线：main@6105beb（分支 audit/v2-full-coverage）
- 纪律：只读审计；本文件与 inventory-a2.csv 为本轮 A2 新增文件之一；未修改任何生产/测试/配置/数据，未运行 cargo test/build。
- 审计方式：声明范围 4 文件逐行全读；调查链扩展定向读取（bid-tools/context.rs、runner.rs、bid-mcp/tools.rs search_evidence 面——KB 生产消费定位必需）。

---

## 0. 覆盖声明

| # | 文件 | 行数 | 覆盖 |
|---|---|---|---|
| 1 | bid-kb/src/lib.rs | 238 | 全读 |
| 2 | bid-kb/src/shard.rs | 293 | 全读 |
| 3 | bid-kb/tests/shard_index.rs | 541 | 全读 |
| 4 | bid-kb/tests/kb_probe.rs | 107 | 全读 |

调查链（非声明范围）：bid-tools/src/context.rs（ChunkTextSource/LedgerTextSource/KbRelevance/anchor_chunk，99-214 面）、bid-tools/src/runner.rs（Kb::ingest/search_exact 消费，250-300 面）、bid-mcp/src/tools.rs search_evidence（1581-1750，确认**不经 bid-kb**）。

---

## 1. 结构与不变量（正面）

- **切块**：chunk_id 内容寻址 = sha256(file_sha:block_idx) 前 16 hex（build_chunk 单一真源，Kb::ingest 与 ShardedKb::upsert_file 共用）；空文本块不入索引（两库同口径——检索等价前提）。
- **分片**（shard.rs）：文件级分片主键 = file label（BTreeMap 确定性迭代）+ 段级子指纹（locator 首路径段分组，FNV-1a 64 over (idx, len 前缀, text)，域标签 0x5A 隔离，borrow mdcore fingerprint 语义）。增量三通道：Unchanged（同 sha 整片零触碰）/ Rebuilt（新片）/ Partial（段级 diff——指纹同段保留原 Chunk（chunk_id 稳定=证据链稳定）、异段重建、消失段随旧片丢弃）；total_chunks 增量记账。
- **检索**：五类 ExactQuery（model/cert_no/date/amount/clause）评分共享核心（score_chunk/hit_of_chunk/sort_hits crate 私有——两检索面单一真源，禁两份漂移）；四级确定性 tie-breaker（匹配分降序→金额单一性升序→文件类别权重升序→chunk_id 升序）；末级 chunk_id 内容寻址唯一 → 合并前遍历序不进输出 → 单库与分片库逐字节等价（tests 断言 score.to_bits() 位级一致）。
- **幂等重建/确定性**：同语料两跑段指纹账本全等（BTreeMap 结构证据）；同查询 5 次重放全等；50 条平局打乱入参序计划不变（budget 侧）。

---

## 2. 调查问题逐项

### 2.1 分片/索引结构、召回与精确查询、排序稳定性、幂等重建、大索引行为

- 分片结构/幂等/等价/移除：shard_index.rs 八测试全绿口径（等价 20 查询逐字段+顺序、重建确定性、Unchanged 值级零变、新文件只建新片且 5 旧片零误触、换版段级 diff（第三章 chunk_id 全等+file_sha 保持旧版的内容寻址语义）、移除片命中面回收、跨片合并+R-15 五次重放、性能验收 60 文件×200 块全量 87ms/新增 30 页补遗 1ms<2s 且防假增量 t_full>t_incr）。**结构面无缺陷。**
- **大索引行为**：search_exact 线性扫全库 chunk（无倒排/无持久化索引）——O(总块数)/查询；60 文件≈12,000 块 debug 87ms 全量、单查询线性扫在该量级毫秒级；**无磁盘持久化**（进程内内存态，重启全量重建——「新增 30 页补遗<2s」的验收面覆盖的是进程内增量）。M1 量级（百万字/500 章级）成立；千万字级未验证（也无此诉求声明）。见 KB-004。
- **双通道融合**：**不存在**——bid-kb 只有 exact 通道；semantic 通道是 M2 接口占位（lib.rs 模块头自认）。MCP `search_evidence` 的 hybrid（exact+token 语义）搜的是**证据台账**（evidence_content(e)=title+scope+excerpt 词面），与 bid-kb 无关（grep：tools.rs 零 bid_kb 引用）。见 KB-005。

### 2.2 证据摘录能否回到真实文件原文（Loc 定位有效性）——「命中标题≠读过正文」

**结论：分三层，生产主流路径确实只到「标题/台账」层：**

1. **bid-kb 命中层（真实正文）**：SearchHit.hit_text = 解析产物块文本（Chunk.text 原文）——kb 检索命中给的是真实文件正文块，不是标题。但这层**生产不可达**：Kb/ShardedKb 的消费方只有 runner.rs 离线管线（TC-02 报告/25 查询重放）与 bid-tools context.rs 的 ChunkTextSource/KbRelevance——后者 `assemble_section_input_with`（注入 KB 的唯一入口）**全仓只有测试调用**（context_budget.rs），生产起草路径 `assemble_section_input` 默认 LedgerTextSource（grep 验证）。
2. **ChunkTextSource 锚定层（受限于锚型）**：anchor_chunk（context.rs:206-214）只支持 `EngineLocator::DocxParagraph`（block_idx 反查）；PdfPageText/XlsxCell/MarkdownBlock/DocxTableCell/CanvasObject 锚一律 `None → LedgerTextSource 降级`——扫描件证据（PdfPageText 锚）走 KB 通道也拿不到正文。
3. **台账层（标题≠正文）**：LedgerTextSource.full_text = title+kind+scope+source_file+locator_brief+subject（**台账元数据，非文件正文**）；EvBrief.excerpt = ev.excerpt（#217 登记）——**只有登记时带可对账 locator（docx_paragraph/pdf_page_text）才有真实原文摘录**（tier=CONTENT_MATCHES；evidence_content.rs 测试扎实，pilot PB-001 已确认 tier 数据面真实）。无 locator 登记（FILE_EXISTS）→ excerpt 空 → 起草素材只有标题/范围。

即：**「命中标题≠读过正文」在生产成立**——MCP search_evidence 的语义通道对 FILE_EXISTS 证据只在 title/scope 上命中；真实回原文的链路（#217 locator 对账摘录 / kb chunk 定位）一条受 tier 不被消费压制（PB-001）、一条生产未接线（KB-001）。

---

## 3. 问题记录

```
ID: KB-001
严重程度: P2
模块: bid-kb + bid-tools + bid-mcp
文件路径: bidcore/crates/bid-kb/src/lib.rs；bidcore/crates/bid-tools/src/context.rs；bidcore/crates/bid-mcp/src/tools.rs
代码行号: context.rs:323-333（assemble_section_input 默认 LedgerTextSource）、206-214（anchor_chunk 仅 DocxParagraph）；tools.rs:1581-1750（search_evidence 搜台账不经 kb）
函数/结构体/接口: assemble_section_input / assemble_section_input_with / ChunkTextSource / search_evidence
问题描述: KB→正文的回源链路在生产断开：①生产起草路径（build_section_input→EvBrief / assemble_section_input）不经 bid-kb——证据「全文」是台账元数据（标题/类别/范围/来源/主体），非文件正文；②ChunkTextSource（唯一 chunk→正文通道）只支持 DocxParagraph 锚且只被测试调用（assemble_section_input_with 无生产调用方，grep 验证）；③MCP search_evidence 的 hybrid 搜证据台账（title/scope/excerpt 词面），与 kb 零关联。「检索命中真实正文块」的能力只在 runner.rs 离线评估管线可达。
调用链: 生产：register_evidence(FILE_EXISTS) → search_evidence（台账词面命中）/ draft（EvBrief.title+空 excerpt）；离线：runner TC-02 → Kb::ingest → search_exact（hit_text=真实正文）
触发条件: 无需触发——接线缺失本身即问题（与 PB-006 同族：域层有、生产不可达）
实际影响: 「证据摘录回到真实文件原文」仅当登记带可对账 locator（#217 链，CONTENT_MATCHES+excerpt）时成立；kb 侧真实正文检索对 MCP Agent 不可见；起草素材对无 locator 证据只有标题（PB-003 的模板断言在标题级素材上生成「可核验」措辞的输入面）
当前测试: context_budget.rs（测试内注入 ChunkTextSource——测试绿≠生产接线）；kb_probe/shard_index 测 kb 自身
缺失测试: 不适用（缺接线非缺测试）
建议方案: M2-b 知识库接线时：service 持有 Kb/ShardedKb 实例（ingest 随 import_files 增量入片），search_evidence 混入 kb exact 通道命中（带 chunk_id/locator 溯源），draft 的 EvBrief 对 DocxParagraph 锚经 anchor_chunk 回正文；PdfPageText 锚的 chunk 对应面（页文本块序）补齐
证据: context.rs:323-333
  pub fn assemble_section_input(store: &DomainStore, section_id: &str, budget: &ContextBudget)
      -> Result<SectionInput, ContextAssembleError> {
      assemble_section_input_with(store, section_id, budget,
          &BuiltinConstraintSource, &LedgerTextSource, &BindingOrderRelevance)
审计置信度: CONFIRMED
状态分类: 当前仍存在（M1/M2 分期既知——lib.rs 模块头「semantic 通道 M2 双通道合并后补」，但正文回源的 exact 侧断链未被任何 issue 显式登记为缺口面）
```

```
ID: KB-002
严重程度: P3
模块: bid-kb
文件路径: bidcore/crates/bid-kb/src/lib.rs
代码行号: 38-46（SearchHit 结构）、183-193（hit_of_chunk）
函数/结构体/接口: SearchHit / hit_of_chunk
问题描述: SearchHit 不携带 locator/block_idx——命中只回传 chunk_id/file/hit_text/score/matched_tokens/amount_count；消费方拿不到引擎定位器（Kb.chunks 虽 pub 可线性反查，但无 by-chunk-id 公开查询面）。审计链（命中→原文位置）需二次回查，离线 runner 只用 file|hit_text（TC-02 报告无定位信息）。
触发条件: 任何需要从命中回到文件位置的消费方
实际影响: 溯源弱化（chunk_id 内容寻址可反查但无 API 面）；对未来 MCP 接线（KB-001 建议）多一次反查设计负担
当前测试: 等价性/重放测试断言字段全等——不含 locator（字段不存在）
缺失测试: 不适用
建议方案: SearchHit 增加 locator/block_idx/kind_rank 透传（Chunk 字段直拷，零成本）
证据: lib.rs:39-46
  pub struct SearchHit {
      pub chunk_id: String,
      pub file: String,
      pub hit_text: String,
      pub score: f64,
      pub matched_tokens: Vec<String>,
      pub amount_count: usize,
  }
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: KB-003
严重程度: P3
模块: bid-kb
文件路径: bidcore/crates/bid-kb/src/lib.rs
代码行号: 142-154（amount 面：re_plain ≥10000 纯数字串换算入金额）、215-237（parse_amount_query）
函数/结构体/接口: score_chunk（amount 分支）/ parse_amount_query
问题描述: 金额面把任意 ≥10000 的纯数字串当金额（v/10000 万口径）——合同编号/批号/日期数字串（如 20260315 → 2026.0315 万）误入块侧金额集合：①amount_count 虚高（排序第二级「金额单一性」被污染——多数字段被当多金额块降位）；②数值恰与查询万口径相等时假阳性命中（如查询「350 万元」时块内编号 3500000 命中）。块侧无「紧邻万元字样」约束（查询侧有 trim_end_matches("万元")），两侧口径不对称。
触发条件: 块文本含 ≥10000 纯数字序列（编号/日期压缩格式/页码串）且 ①参与排序平局或 ②数值换算后恰等于查询金额
实际影响: 排序漂移（低危）+ 罕见假阳性命中（amount 查询精确性）
当前测试: shard_index queries a1-a4 均为真实金额面；无编号串污染用例
缺失测试: 「编号 3500000 vs 查询 350 万元」的负测试；amount_count 对编号串的计数断言
建议方案: re_plain 换算侧加词面约束（数字串前后任一侧近邻「元/万/价/额」类字符才算金额），或把纯数字串只入 amount_count 不入 amounts 匹配集
证据: lib.rs:148-153
  for m in re_plain().captures_iter(text) {
      if let Ok(v) = m[1].replace(',', "").parse::<f64>() {
          if v >= 10000.0 {
              amounts.push(v / 10000.0);
          }
      }
  }
审计置信度: CONFIRMED（行为由代码确定；实害依赖语料形态，标 NEEDS_RUNTIME_VERIFY 亦可——取保守 CONFIRMED for 行为/影响 P3）
状态分类: 当前仍存在
```

```
ID: KB-004
严重程度: P3
模块: bid-kb
文件路径: bidcore/crates/bid-kb/src/lib.rs；bidcore/crates/bid-kb/src/shard.rs
代码行号: lib.rs:76-85（search_exact 线性全扫）；shard.rs:53-57（ShardedKb 内存态）
函数/结构体/接口: Kb::search_exact / ShardedKb::search_exact
问题描述: 大索引行为边界：检索无倒排/无前缀索引——每查询线性扫全库 chunk；无磁盘持久化索引——进程重启全量重建（分片增量只在进程生命周期内生效）。60 文件/12,000 块量级实测（注释）全量建库 87ms、查询毫秒级——M1 百万字诉求成立；未测 10× 以上语料（无此声明）。
触发条件: 语料规模数量级增长或跨进程检索诉求
实际影响: 规模上限未探明（当前量级无实害）；「新增 30 页补遗<2s」验收只覆盖进程内增量，不含重启重建
当前测试: acceptance_addendum_update_under_2s（含防假增量 t_full>t_incr）
缺失测试: 大语料（如 600 文件）线性扫查询时延基准——非必需
建议方案: M2 接线时按需评估（倒排或按 query_type 的稀疏索引）；持久化随 M2-b 落地
审计置信度: CONFIRMED（结构与实测注释确定；规模外推未验证）
状态分类: 当前仍存在（M1 范围内声明的一致设计）
```

```
ID: KB-005
严重程度: P3
模块: bid-kb + bid-mcp
文件路径: bidcore/crates/bid-kb/src/lib.rs；bidcore/crates/bid-mcp/src/tools.rs
代码行号: lib.rs:6-8（模块头「semantic 通道是接口占位（M2 双通道合并后补）」）；tools.rs:1659-1695（search_evidence 自有 hybrid——台账 token 匹配，非 kb）
函数/结构体/接口: Kb / search_evidence
问题描述: 「双通道融合」在 bid-kb 层不存在（exact-only，semantic 为 M2 占位且无接口形态——ExactQuery 之外无 semantic 查询类型）；MCP search_evidence 的 hybrid/exact/semantic 三模式全部作用于证据台账（evidence_content=title+scope+excerpt 的 token contains），与 bid-kb 的五类精确通道是两套独立检索面，命名易混（search_evidence(mode=exact) ≠ kb exact 通道）。
触发条件: 读者把 MCP search_evidence 的「exact/semantic」理解为 kb 通道
实际影响: 概念面混淆——两套 exact 语义（台账词面 vs kb 五类查询）共存；M2 接线时的合并设计负担
当前测试: 各测各面（kb_probe 测 kb；search 工具测台账）
缺失测试: 不适用
建议方案: M2-b 合并时统一通道命名或在 MCP 出参标注检索面来源（ledger/kb）
审计置信度: CONFIRMED
状态分类: 文档与实现一致（M1 范围声明明确——登记为口径债非缺陷）
```

---

## 4. 测试有效性评估

| 测试文件 | 类型 | 独立 Oracle | 错误路径可达 | 评注 |
|---|---|---|---|---|
| tests/shard_index.rs（541 行） | UNIT/CONTRACT（全合成夹具，手造 ParsedFile 控制变量） | ✅ 等价性=双实现对账（单库 vs 分片逐字段+score 位级）；增量三通道=结构性证据（chunks_built 只含新片块数、旧片 5 片逐字段零误触、保留段 chunk_id 全等） | ✅ 无命中面查询（空结果对账也是等价面）；性能验收含防假增量断言 | 高质量；borrow 方法论（B3 双跑差分/contract_freeze/跨进程确定性）显式注明 |
| tests/kb_probe.rs（107 行） | REAL_CORPUS（corpus-A 真文件 + 引擎解析路由 docx/xlsx→bid-engine、pdf→parse_file） | ✅ 25 查询 expect（hit_text+hit_file）独立写死在 fixtures/queries/exact-25.jsonl（fixture_spec.py 冻结真源） | ✅ ENV 缺件 SKIP（ENV_MISSING 不伪装业务 FAIL）；misses 逐条打印 | 真实语料锚定；B4 路由与 service 同款——解析面代表性好 |

生产接线缺口（KB-001）不被任何测试暴露——测试绿≠生产可达（与 PB-006 同型盲区）。

---

## 5. 结论速览

- **分片/幂等/确定性/等价**：全部成立且有强测试（含 <2s 性能与防假增量）；无缺陷。
- **证据回原文能力（核心问题）**：分三层——kb 命中层是真实正文但生产不可达（KB-001，P2）；ChunkTextSource 只锚 DocxParagraph；台账层（生产实际形态）对 FILE_EXISTS 证据=标题/范围元数据，「命中标题≠读过正文」成立；真实回原文仅 #217 locator 对账摘录一条链（tier 不被消费见 PB-001）。
- **双通道融合**：bid-kb 无 semantic 通道（M2 占位）；MCP hybrid 是台账面 token 匹配，与 kb 无关（KB-005 口径债）。
- 其余：SearchHit 无定位面（KB-002）、金额面纯数字串误判（KB-003）、线性扫/无持久化规模边界（KB-004）均 P3。
