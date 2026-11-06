# 模块审计报告 B2b：bid-parse 抽取 / 合并 / 否决词链路（逐文件）

> 全仓审计 V2 · Task 4-b2b（模块审计代理 B2b）。
> 冻结基线：main@`6105beb`；工作分支 `audit/v2-full-coverage`（HEAD `a689701`，生产代码与基线一致）。
> 只读审计：未修改任何生产文件/测试/配置/数据；未运行 cargo test/build。
> 模块根：`bidcore/crates/bid-parse/`（任务书所称 `bid-parse/src/` 实际位于此处）。

---

## 1. 覆盖声明

| 面 | 应覆盖 | 实覆盖 | 说明 |
|---|---|---|---|
| src 目标文件 | 8 | 8 | extract.rs(1305) / review_table.rs(1217) / veto_lexicon.rs(652) / veto_rule.rs(636) / merge.rs(476) / numbering.rs(409) / clause.rs(266) / nesting.rs(250)——全部逐行读完 |
| tests 目标文件 | 6 | 6 | extract_docx_veto.rs(304) / extract_format_chapter.rs(232) / extract_dunhao.rs(213) / nesting_corpus.rs(200) / extract_kind_dispatch.rs(111) / real_corpus_223.rs(128)——全部读完 |
| 关联面（只读抽查） | — | — | lib.rs(分发/Block 模型) / bid-mcp tools.rs(生产入口 :650-885、TENDER_FILE_KINDS :83-91、merge 面组装 :724-743、冲突检测 :759-798、缺号检测 :610-645) / bid-tools runner.rs(:225-246 导入 kind 映射) / bid-tools service.rs(ImportSpec) / veto-gt 工件(missed-list 744 条) / real-corpus 与 corpus-A 夹具在位性 |

夹具核对：`real-corpus/water/files/`（3 件水科 PDF）与 `real-corpus/telecom/parse-input/B-cqyd-*.pdf` 在位（real_corpus_223 / nesting_corpus 消费）；extract_docx_veto / extract_format_chapter / extract_dunhao / extract_kind_dispatch 均为测试内联合成块（无外部夹具依赖）；corpus-A 夹具供 bid-tools/bid-mcp 侧测试消费（本模块 6 测试不直接消费）。

## 2. 调用拓扑（谁调谁）

```
MCP 生产入口  bid-mcp/tools.rs:650 extract_requirements
  ├─ :705 extract::extract_requirements_with_kind(parsed, kind_map)   ← kind 来自 TenderFile（导入登记）
  ├─ :724-741 faces = parsed.filter(has_extraction_face(name))        ← ★前缀判据（PARSE2-001）
  ├─ :742 merge::merge_extraction(faces, reqs)                        ← Agent 优先 + 正则兜底
  ├─ :745-757 缺号检测（clause_digits/missing_in_run——警告级）
  └─ :759-798 主/补遗冲突检测（C- 前缀判据——PARSE2-011）
评测/场景入口  bid-tools/runner.rs:255 与 bin/real_eval.rs:144
  └─ extract::extract_requirements(parsed)  ← legacy 入口：kind 恒 None → 前缀兜底分发（PARSE2-004）
extract 内部
  ├─ extract_dispatch → extract_main_pdf / extract_main_docx / extract_b1_lines / extract_b2_ocr
  │    ├─ review_table::{extract_pdf_linearized, extract_docx_tables}（#144 审查表接管）
  │    ├─ veto_lexicon::{is_veto,is_qual,is_list_item,is_placeholder,opens_*}（DOCX 侧词表）
  │    ├─ veto_rule::is_rule_statement（#226 规则说明降级——两侧接线）
  │    └─ clause::clause_boundary（DOCX 续行吸收的边界判定）
  └─ 后置 #52 排除面：veto_lexicon::exclusion_reason → 否决项改判评分项/其他（params._veto_excluded）
clause.rs → numbering.rs（七形态层级单一真源）；nesting.rs 独立纯函数（生产链路未接线，供切分面复用）
```

## 3. 六个重点问题裁定（对应任务书）

### 3.1 extract.rs 条款抽取主链
- **输入块身份稳定性**：Block(idx, kind, locator, text)；merge 索引键 = `(locator, text)`（`or_insert` 首块优先，merge.rs:124-129，与 real_corpus_e2e 块对齐同款惯用法）。DOCX 段落 locator 常为同章名（多段同 locator），身份实际由 (locator, text) 二元组承担；同文重复段（同 locator 同 text）会折叠到首块 idx——合并覆盖判定按首块（文档化取舍）。REQ id `REQ-{seq:04}` 全局顺序号：同输入同输出（确定性达标），但跨重跑/文件集变化时 id 漂移——由「整集替换」语义兜住（tools.rs:800-805）。
- **编号不连续**：抽取层不做连续性推断（编号只进 clause_no）；连续性由 tools.rs:610-645 `clause_digits + missing_in_run` 后置检测（同父路径末位差>1 且 ≤50 → MISSING_SEQUENCE **警告**，非阻断；非纯数字条款号——前附表-N/表-N/形式评审表-N——不参与该检测）。numbering.rs 的 NumberingContext（混排层级修正）**未接线主链**（模块文档自陈，避免高估层级）——层级推断错误无传播面。
- **标题正文混合**：章头行整行作为 chapter 上下文后 `continue`（extract.rs:560-581）——「第X章 标题 + 正文」同行时正文部分不入抽取面（PARSE2-016 关联，低频）。两行章形态由 `chapter_title_ahead` 前瞻补判（#234）。
- **表格内条款**：DOCX TableRow 三通道（#144 审查表接管 → review_map 消费防双抽；前附表否决行；P0-B 表格行否决面）；PDF 表格经 pdftotext 线性化由 review_table 标题驱动装配。
- **跨块合并语义**：PDF 段落重组（行尾无终结标点拼段；列举条目强制断段——#68 G）；DOCX 内联续行吸收（extract.rs:958-983，吸收终止 = 编号/章头/占位/强词/列举——**与 clause::merge_multiline_clauses 的四规则不同**，见 PARSE2-012）。
- **幽灵 REQ 可能性**：① <6 字段落成员行例外（extract.rs:604-613）→ 同一条款产「段落 REQ + 碎片 REQ」两条（PARSE2-007，注释自陈「碎片级条款面，非语义重复」——矩阵视角仍是重复行）；② 非格式章的表格头页：段落 REQ + 多条碎片 REQ 并存（#234 守卫仅覆盖格式章，extract_format_chapter.rs:188-199 锁定「非格式章行为不变」）；③ 无「无主条款」面——所有 REQ 均锚定真实块（B2 OCR 除外，锚为两行拼接的合成文本，PARSE2-013）。
- **kind 分发值域（#199 重核）**：`is_attachment`(附件|图纸|证据) 跳过；`is_b1/is_b2` 的 kind 值（报价文件/扫描件）**不在 TENDER_FILE_KINDS 7 枚举、也不在任何登记路径**（runner 给 B1/B2 登记「附件」）→ 生产 kind 路径下 B1/B2 通道死代码、整文件跳过（PARSE2-004）。反向清单设计（其余 kind 一律主抽取）本身已由 extract_kind_dispatch.rs 值域矩阵锁定 ✅。

### 3.2 review_table.rs 评分表结构化抽取
- 表头识别：`recognize_header` 纯函数，评审内容+合格条件（表7/8 同形，靠表名区分）/评审项目+标准分（表9）；同格粘连防呆（评审项目+标准分同格 → None，即**合并单元格表头显式不支持**）。
- 跨页：DOCX 侧表键连续即连续装配（引擎块流无页概念）；PDF 线性化侧页面装饰（招标文件/裸页码/评标表-N）按噪声跳过，跨页不打断。
- 嵌套表格/合并单元格：无嵌套表处理面（docx 侧按行块平铺）；合并单元格表头 → 识别失败。
- 权重列/档位映射：无权重列概念（#224 ScoringMapping 在 bid-domain 侧另建）；score 解析 = 纯数字/「N分」，**解析失败时 find_map 任意格兜底**可捞异物（PARSE2-015）。
- **抽取失败状态**：识别失败与「无审查表」同为一个空 Vec 返回——**无「表在位但未结构化」的显式状态/账面**（ParsedFile.components 部件账本不覆盖此面）（PARSE2-006，与健康检查问题④「engine_pdf 通道 tables=0」同族）。
- PDF/DOCX 产物不对称：PDF 线性化条目 `criteria/score/score_rules` 恒空（flush_pending），仅 section_title 进 params；DOCX 侧 score+score_rules 入 params（PARSE2-005）。

### 3.3 veto_lexicon + veto_rule 否决规则引擎
- **后置否定从句覆盖（健康检查 P0-② 重核）**：复合短语面已覆盖「否则其投标将被否决 / 否则各相关投标均无效 / 否则按无效标处理」类（veto_rule.rs:534-542 测试、re_veto_strong）；**宾语为非投标对象的后置否定（「否则该业绩无效」——veto-gt missed-list A01-GT-024，GT 资格性否决）在两份词表中均零覆盖**（PARSE2-002）。词表命中 ≠ 真实否决语义的边界：代码以三层声明（模块文档「位置权重」启发式、#52 句级放行/排除算法、#226 规则说明安全网），**但正通道命中不落任何 provenance 参数**（仅降级面有 _veto_excluded）——命中理由不可追溯（PARSE2-016）。
- **「其他」兜底比例**：category_for 缺省「其他」+ 规则说明降级也常落「其他」（extract_docx_veto.rs:89 断言「招标人有权拒收」→其他）。无「其他占比过高」观测告警（仅 ZERO_REQUIREMENTS 总门）。
- **规则优先级**：编号条款 = 强否决（经 veto_rule 规则说明闸）> 资格面 > 章节上下文；分值形态锁 + 否决旁路（#110 G4）；#52 后置排除面统一复检（单点接入两侧）。优先级链自洽且测试锁定 ✅。
- **误报面**：exclusion_reason 句级算法 + 保护词机制，模块内 10 组族测试 + tp_collateral_scan 声明零误伤；PDF/DOCX **两份否决词表各自维护、覆盖面已分叉**（PARSE2-003）。
- **#226 规则说明层与 GT 的系统性张力**：GT 标 true 的程序性否决（「逾期送达…招标人不予受理」「招标人有权拒收」——missed-list 程序性否决 19 条）被 is_rule_statement 判规则说明 → 降级其他（防永久 blocking 死锁，#230/#233 三轮收紧+dump 复核驱动）——有意取舍，但该损失面未在任何召回口径中单列披露（PARSE2-010）。

### 3.4 merge.rs Agent 标注与正则兜底合并
- 冲突解决：块级裁决（任一 anchor 命中覆盖块 → 整条正则 REQ 丢弃，verdict 不区分）；Agent REQ 追加 `REQ-A{seq}`，正则项原 id 原序。无标记严格 no-op（测试锁定）✅。
- 重跑幂等：纯函数，同输入同输出；REQ-A 编号为 face 序+block_idx 稳定序——submitted 集变化时 id 会移位（整集替换语义兜住，域层 fail-loud id 唯一性在位）。
- 噪声与待审区分：MergeStats 有 uncovered_blocks/coverage_complete，但 verdict 粒度不进统计（noise 与 requirement 覆盖同权）——#244 需要的三分处置在 stats 面不存在（coverage_tools 另有 per-file 报告）。
- **★核心缺陷：合并面文件过滤用前缀判据 `has_extraction_face(name)`（A-/C-/B1/B2）**（merge.rs:71-76，tools.rs:727）——真实文件名（无前缀）不进 faces：Agent marks/submitted **静默不生效**（正则 REQ 不丢弃、Agent REQ 不追加）、uncovered 统计不含该文件；而 get_coverage_report（coverage_tools.rs:233-275）按 sha 全文件口径报告——两工具口径分裂（PARSE2-001，P1）。
- 多锚 REQ 部分覆盖即整条丢弃的召回面（PARSE2-009）。

### 3.5 numbering / nesting / clause
- numbering.rs：无状态七形态 + 段点数自描述层级；语境修正未接线（文档自陈）；**无连续性推断**（由 tools.rs 警告级补）。错误传播面：不存在（层级不入 REQ）。
- nesting.rs：纯函数配对/保护切分，未配对优雅降级；真实语料冒烟（nesting_corpus）零 panic + join-back 全等不变量 ✅。生产主链未消费（供切分层复用）。
- clause.rs：clause_boundary（薄封装 numbering）被 extract DOCX 吸收消费；`merge_multiline_clauses` **无生产调用方**（仅自测）——「跨行合并单一真源」声明与实际不符（PARSE2-012）。

### 3.6 测试 Oracle 独立性（详见 §5）
- 6 测试中 5 个为手写语义判例（独立期望值，oracle=业务规则直接编码）——无自证；
- **real_corpus_223 的 923/317/409 是抽取器自身历史产出的下限基线（自证回归锁）**，且只断言 `>=`（膨胀型回归不可检）；生产路由（引擎路）的 951→942 数字仅存于注释，无测试锁定；
- 全部 6 测试走 **legacy 入口 `extract_requirements`（kind=None → 前缀分发）**，生产入口 `extract_requirements_with_kind` 仅 extract_kind_dispatch 覆盖分发值域——否决/段落/表格行为面在 kind 通道下无测试（当前内部分发后同路径，风险低但属测试-生产入口分叉）；
- 真正的独立真值（人工标注 GT）在 artifacts/veto-gt + bid-mcp real_corpus_e2e 红线——不在本模块测试内。

## 4. 问题记录

---

**ID: PARSE2-001**
严重程度: P1
模块/文件路径/代码行号/函数或结构体: bid-parse/src/merge.rs:71-76 `has_extraction_face`；bid-mcp/src/tools.rs:724-743 `extract_requirements`（faces 组装）
问题描述: 合并面文件过滤用**文件名前缀**（A-/C-/B1/B2）判据，与 extract.rs 已修复的 #196/#199「kind 优先、前缀仅兜底」分发不对齐。真实用户文件（无前缀、kind=招标文件）不进 MergeFace。
调用链: tools.rs:727 `.filter(|p| has_extraction_face(&p.name))` → merge_extraction(faces, reqs)
触发条件: 任意真实文件名（如「光纤租赁费招标文件.docx」）导入并执行 extract_requirements（Agent 已 submit_block_marks / submit 关键点）
实际影响: ① 该文件的 Agent marks/submitted **静默失效**（覆盖块正则 REQ 不丢弃、Agent REQ-A 不入库——#219「Agent 优先」语义对真实文件整体旁路）；② uncovered_blocks/coverage_complete 统计不含该文件 → UNCOVERED_BLOCKS 告警失真；③ 与 get_coverage_report（按 sha 全文件口径，coverage_tools.rs:233-275）**口径分裂**——覆盖报告说有未覆盖块、合并却忽略同文件；④ #244 覆盖处置闭环在建纲前的阻断面会漏掉此类文件。
复现步骤: ① MCP 链导入 `招标文件v1.docx`（kind=招标文件）；② submit_block_marks 标记若干块为 noise；③ 重跑 extract_requirements；④ 观察 merge.regex_dropped_covered=0、agent_requirements=0、uncovered_blocks=0，而 get_coverage_report 仍列出未覆盖区间。
当前测试: merge.rs 内联 6 测试全部用「A-主文件/C-补遗」夹具名——前缀面永远为真，缺陷不可见。
缺失测试: 无前缀文件名 + Agent marks 的合并语义测试（正是 #196 的测试盲区在 merge 面重演）。
建议方案: has_extraction_face 改为接受 kind（与 extract_dispatch 同源判定：附件/图纸/证据无面、其余有面），tools.rs 组装 faces 时传入 kind_map；或干脆以「parsed 文件是否产出正则 REQ」为口径。补无前缀名 + marks 的回归测试。
证据:
```rust
// merge.rs:67-76
/// 该文件是否有条款抽取面（与 `extract::extract_requirements` 的分发前缀对齐）。
pub fn has_extraction_face(name: &str) -> bool {
    name.starts_with("A-") || name.starts_with("C-")
        || name.starts_with("B1") || name.starts_with("B2")
}
// tools.rs:727
.filter(|p| bid_parse::merge::has_extraction_face(&p.name))
```
（注：注释宣称「与 extract 分发前缀对齐」——extract.rs:82-116 现为 kind 优先，注释已失实。）
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

**ID: PARSE2-002**
严重程度: P1
模块/文件路径/代码行号/函数或结构体: bid-parse/src/veto_lexicon.rs:22-40 `re_veto_strong`、:44-57 `re_negative_list`；bid-parse/src/extract.rs:216-237 `re_veto`（PDF 面）
问题描述: **后置否定从句的宾语为非投标对象时零覆盖**。两份词表只收复合处置短语（视为无效/投标无效/按无效标处理/投标失效/投标被拒绝…）；「否则该业绩无效」「否则其证明文件无效」类（处置词=裸「无效」，宾语=业绩/文件/材料）无任何模式命中。
调用链: extract_main_pdf veto_hit → re_veto 不命中 → re_qual/章节上下文 → 资格条件/其他；DOCX 侧 vl::is_veto 同样不命中。
触发条件: PDF/DOCX 含「…需另行提供相关的业主证明文件，否则该业绩无效。」类条款（veto-gt missed-list A01-GT-024 为实锤样本，GT 类别=资格性否决）。
实际影响: 否决召回损失（健康检查 P0-②「后置否定从句落其他」的残余根因之一）——该族条款今天仍落资格条件/其他，不进否决项/阻断面。
复现步骤: 静态可复现：`is_veto("…否则该业绩无效")` → false（无「视为/投标/按…处理」前缀词面）。
当前测试: extract_docx_veto.rs `postpositive_negation_clause` 只测「将被否决」形——非投标宾语形态无测试。
缺失测试: 「否则+非投标宾语+无效/作废/不予认可」形态的否决判定测试；veto-gt missed-list 该族的回归夹具。
建议方案: 增补后置否定从句模式（`否则[^。；]{0,16}(无效|作废|不予认可|不予接受)`）并过 #52 排除面零误伤验证；以 veto-gt A01-GT-024 固化为回归样例。
证据:
```rust
// veto_lexicon.rs:26-31（DOCX 面强词——无裸「无效」宾语形）
"将被否决|否决其投标|…|视为无效|无效投标|按无效标处理|作无效标处理|认定为无效|投标无效|投标均无效|…"
// veto-gt/missed-list.json A01-GT-024（GT 资格性否决，missed）：
// "相关特性的，需另行提供相关的业主证明文件，否则该业绩无效。"
```
审计置信度: 词表缺口 CONFIRMED（静态）；对当前召回数字的影响 NEEDS_RUNTIME_VERIFY（需重跑 veto-measure/健康检查）
状态分类: 当前仍存在

---

**ID: PARSE2-003**
严重程度: P2
模块/文件路径/代码行号/函数或结构体: bid-parse/src/extract.rs:216-237 `re_veto`（PDF）vs veto_lexicon.rs:22-57 `re_veto_strong`+`re_negative_list`（DOCX）
问题描述: PDF 与 DOCX 各持一份否决词表，注释宣称「对齐并扩展」，实际覆盖面已分叉：PDF 独有（投标失效/投标被拒绝/利害关系…不得参加投标/拒收情形/否决情形/无效情形/废标情形/以他人名义投标/弄虚作假/串通投标/失信/黑名单/不良行为/行政处罚/刑事处罚/严重违约/重大工程质量问题…）；DOCX 独有（受贿/私下接触/不是本单位人员/非本单位人员/被责令停业/被逐/合同被解除/违约或被逐/重大安全事故/不良记录/不予评审/有权取消/资格审查不合格/评审不合格的）。
调用链: 同一条款文本经 PDF 通道与 DOCX 通道会得到不同否决判定 → 同项目混装 docx+pdf 主件时分类不一致。
触发条件: 任一单侧独有词的条款（例：「投标人存在受贿行为的…」DOCX 判否决、PDF 不判；「其投标失效」PDF 判否决、DOCX 不判——DOCX 侧靠 veto_rule 闸 0 兜住不豁免但 is_veto 本身 false）。
实际影响: 跨格式分类漂移；词表演进只能单侧修（历史上 #68 各阶段扩面只写了 PDF 侧注释即为例证）。
当前测试: 两侧各自测各自词面；无「同文本双通道同判」对称性测试（#233 做过 3 条对称，未覆盖词表全集）。
缺失测试: 词表同源性测试（提取两侧词面求差集断言为空，或合并为单一真源）。
建议方案: 否决词表收敛到 veto_lexicon 单一真源，extract.rs PDF 侧改调 vl::is_veto（保留 PDF 特有的行级语境守卫在调用侧）。
证据:
```rust
// extract.rs:216-235（PDF 面独有词示例）
"投标失效|投标被拒绝|…|不良行为|失信|黑名单|行政处罚|刑事处罚|严重违约|…|拒收情形|否决情形|无效情形|废标情形|以他人名义投标"
// veto_lexicon.rs:47-54（DOCX 负面清单独有词示例）
"骗取中标|骗取|严重违约|…|行贿|受贿|失信|黑名单|不良行为|不良记录|…|私下接触|不是本单位人员|非本单位人员|…|被责令停业|被逐|合同被解除|违约或被逐"
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

**ID: PARSE2-004**
严重程度: P2
模块/文件路径/代码行号/函数或结构体: bid-parse/src/extract.rs:100-116 `extract_dispatch`（is_b1/is_b2 kind 值）；bid-mcp/src/tools.rs:83-91 `TENDER_FILE_KINDS`；bid-tools/src/runner.rs:233-237（导入 kind 映射）
问题描述: B1/B2 专用抽取通道（extract_b1_lines/extract_b2_ocr）的 kind 值（「报价文件」「扫描件」）不在 TENDER_FILE_KINDS 7 枚举、不在任何登记路径（runner 给 B1/B2 登记 kind=「附件」）。生产 kind 路径下 is_attachment 先命中 → B1/B2 整文件跳过（零 REQ）；runner/real_eval 走 legacy 入口（kind=None → 前缀兜底）仍抽取——**评测口径与生产口径分叉**。
调用链: MCP 生产：import(kind=附件) → with_kind → is_attachment → skip；评测：runner import(kind=附件) → extract_requirements(legacy) → 前缀 B1/B2 → 抽取。
触发条件: 真实用户把设备清单/扫描盖章页按「附件」导入（与 runner 同款语义）——MCP 链零抽取且无 per-file 提示（只有全局 ZERO_REQUIREMENTS 总门）。
实际影响: B1 型号/日期条款（corpus-A GT 技术参数 5 条）与 B2 扫描件否决关键词（否决/签署/盖章）在生产 MCP 口径下丢失；红线基线（923/317/409、corpus-A 25 REQ）全部以 legacy 口径测得——**红线测的不是生产路径**。
复现步骤: extract_requirements_with_kind(&[B1 文件], |_| Some("附件")) → 空。
当前测试: extract_kind_dispatch.rs:103-110 对 B1 仅 `let _ = reqs_b1;`（零值断言——只验证不 panic）；attachment_kinds_skip_even_with_main_prefix 锁定了「附件跳过」行为（即把该分叉钉成了规范）。
缺失测试: 「评测入口=生产入口」的一致性测试（或至少登记该分叉为已知口径差）。
建议方案: 二选一：① 若「附件无条款面」是规范 → runner/real_eval 改用 with_kind 并把 B1/B2 的期望值改为 kind 口径；② 若 B1/B2 面要保留 → is_b1/is_b2 的 kind 值入 TENDER_FILE_KINDS 或以 FileRole 特化。同时 extract_kind_dispatch 的 B1 分支补真实断言。
证据:
```rust
// extract.rs:101-103
let is_attachment = matches!(kind.as_deref(), Some("附件") | Some("图纸") | Some("证据"));
let is_b1 = matches!(kind.as_deref(), Some("报价文件")) || f.name.starts_with("B1");
let is_b2 = matches!(kind.as_deref(), Some("扫描件")) || f.name.starts_with("B2");
// runner.rs:235-236（B1/B2 登记为「附件」——报价文件/扫描件 kind 无处登记）
("B1-设备清单.pdf", "附件"), ("B2-投标函格式页-扫描件.pdf", "附件"),
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

**ID: PARSE2-005**
严重程度: P2
模块/文件路径/代码行号/函数或结构体: bid-parse/src/extract.rs:785-825（PDF review_items REQ 化）vs :1074-1100（DOCX TableRow REQ 化）
问题描述: PDF 线性化评审表条目不写 score/score_rules params（flush_pending 恒 None；REQ 化只插 section_title）；DOCX 侧 score+score_rules 均入 params。PDF 详细评审表条目（cat=评分项）仅靠 push_req 的 score_of(锚文本) 兜底——item 文本无「N分」时评分项无分值。
调用链: extract_pdf_linearized → ReviewItem{score:None, score_rules:None} → push_req → score_of(it.item)（多数表格线性化行不含分值文本时失败）
触发条件: PDF 主件的详细评审表（标题行 ≤18 字「详细评审表」可识别即触发装配）。
实际影响: #224 ScoringMapping / T-3 评分矩阵在 PDF 件上的结构化输入不完整（无 score/score_rules）——PDF 评分项权重面弱于 DOCX（健康检查问题④的下游表现）。
当前测试: review_table.rs 内联测试覆盖 PDF 装配条数/行号/拼接，未断言 score 承载；extract_pdf_l2gap/pdf_para 不测 params。
缺失测试: PDF 评审表条目 params.score 的正负测试。
建议方案: flush_pending 尝试从行文本解析 `(\d+(?:\.\d+)?)\s*分`（复用 parse_score/score_of）；或 REQ 化时对 Detailed 类条目补 score_rules=行文本。
证据:
```rust
// review_table.rs:648-658（PDF 侧 flush——score/score_rules 恒空）
items.push(ReviewItem { kind, item: t.to_string(), criteria: String::new(),
    score: None, score_rules: None, locator, row, table_name: name.to_string(), block_idxs: idxs });
// extract.rs:810-813（PDF 侧 REQ 化仅 section_title）
let mut params = HashMap::new();
if let Some(title) = crate::review_table::chapter_ref_of(&joined_ref) {
    params.insert("section_title".into(), serde_json::json!(title)); }
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

**ID: PARSE2-006**
严重程度: P2
模块/文件路径/代码行号/函数或结构体: bid-parse/src/review_table.rs:101-138 `recognize_header`、:418-445 `linearized_table_title_kind`、:193-203 `extract_review_tables`
问题描述: 评分表/审查表「识别失败」与「文档无此表」不可区分——返回同一个空 Vec，无「表在位但表头不合形态/表名行未识别」的显式状态或账面。表头合并单元格（「评审项目 标准分」同格）被防呆规则显式拒绝；PDF 侧表名行 >18 字/含「详见」/含冒号/含工程名称装饰即不识别。
调用链: extract → extract_review_tables 空 → 旧链逐行分支接管（退化为词面抽取）→ 结构化评分面静默缺失。
触发条件: 真实件表头异形（如「评审因素与标准」「分值范围」列名变体；表名带项目名超长）。
实际影响: 评分矩阵/T-3 无结构化数据源时无任何 warning（unlike ZERO_REQUIREMENTS 总门）；#224 unmapped_scoring_items 的上游输入面静默收缩。
当前测试: recognize_non_review_headers_none 测「非审查表头 → None」——把失败面钉为规范，无「在位表识别失败」的告警测试。
缺失测试: 「ParsedFile.tables>0 且含评审章上下文 但 extract_review_tables 空」→ 应有可见信号的测试。
建议方案: 复用 ComponentLedger 思想加 ReviewTableLedger（表在位数 vs 结构化数），或在 extract 响应 warnings 加「评审表存在但未结构化」条件告警（章名含评标办法/评审 + tables>0 + 结构化 0）。
证据:
```rust
// review_table.rs:107-118（同格粘连即 None——合并单元格表头不支持）
if item_col != score_col { return Some(HeaderShape::Detailed { … }) }
// :422（PDF 表名行长度门——超长表名静默不识别）
if n.chars().count() > 18 || n.is_empty() || n.contains("详见") { return None; }
```
审计置信度: CONFIRMED（设计缺口）
状态分类: 当前仍存在

---

**ID: PARSE2-007**
严重程度: P2
模块/文件路径/代码行号/函数或结构体: bid-parse/src/extract.rs:604-620（<6 字成员行例外 + seen_para）
问题描述: 段落成员行（非首行）归一化长度 <6 时绕过去重独立 REQ 化——同一条款产两条 REQ：① 段落 REQ（首行编号、锚=全段行）；② 碎片 REQ（锚=尾部短行，如「予以拒收。」）。注释自陈为 GT L2 对齐的有意设计（「碎片级条款面，非语义重复」），但矩阵/合规视角是重复行（双 blocking 面双计）。同机制使**非格式章**的表格头页产「整段 REQ + 多条碎片 REQ」（#234 守卫仅格式章）。
调用链: 段落重组 → para_start[i]<i 且 norm_len<6 → 不 continue → 独立走分支链 → push_req。
触发条件: pdftotext 把一句条款切成「长行 + 短行(<6字)」；或非格式章的整页无标点表格行。
实际影响: REQ 计数虚增（#234 后非格式章残余面）；同一否决条款在矩阵中两行（其一无条款号）——响应面重复劳动；「其他」类稀释。
当前测试: extract_format_chapter.rs `multi_line_fragment_head_kept` 锁定保留行为（防误杀真条款）；无「碎片 REQ 与段落 REQ 重复」的计数/去重测试。
缺失测试: 碎片双抽的显式登记测试（断言产物对数与预期重复面）或下游去重不变量。
建议方案: 短期：碎片 REQ 加 params.fragment=true 供下游识别；长期：GT 对齐层用「锚页+文本包含」替代「短行独立锚」后删除该例外。
证据:
```rust
// extract.rs:604-613
if para_start[i] < i {
    let norm_len = t.chars().filter(|c| !c.is_whitespace()).count();
    if norm_len >= 6 { continue; }   // <6 字成员行不跳过 → 独立 REQ
}
```
审计置信度: CONFIRMED（行为）；有意设计（注释在位）
状态分类: 当前仍存在

---

**ID: PARSE2-008**
严重程度: P2
模块/文件路径/代码行号/函数或结构体: bid-parse/src/extract.rs:583-585（re_subsection 分支）
问题描述: PDF 侧中文序号小节头（「一、投标报价」≤40 字）`continue` 时**不重置 veto_ctx/review_ctx**（仅章头 :560-565 与 pend_head_form :594-602 清语境）。否决语境可跨越小节边界流入下一小节的列举条目。
调用链: 总括句开 veto_ctx → 小节头不清 → 新小节 re_list_cn 条目 → 传播分支误判否决项。
触发条件: 「有下列情形之一的，其投标将被否决：」后紧跟「一、投标报价」小节头 + 其下 (1)(2) 报价条目。
实际影响: 否决项误报（FP）——报价条目继承否决语义进阻断面。DOCX 侧无此问题（普通段无条件终止 :1067-1069）。
当前测试: extract_dunhao.rs 守卫①只测条目连续语境；无跨小节头语境测试。
缺失测试: 「总括句 → 小节头 → 异域列举」不传播的负向测试。
建议方案: re_subsection 分支加 `veto_ctx = false; review_ctx = false;`（与 DOCX 普通段终止对齐）。
证据:
```rust
// extract.rs:583-585
if re_subsection.is_match(t) && t.chars().count() <= 40 {
    subsection = t.to_string();
    continue;              // ← veto_ctx/review_ctx 未清
}
```
审计置信度: SUSPECTED（代码路径确认，真实语料触发面未量化）
状态分类: 尚无法复现（需构造语料验证）

---

**ID: PARSE2-009**
严重程度: P2
模块/文件路径/代码行号/函数或结构体: bid-parse/src/merge.rs:133-148（retain 过滤）
问题描述: 多锚 REQ（段落重组后锚=全段行）任一 anchor 命中 Agent 覆盖块 → 整条丢弃（「保守——Agent 优先」）。Agent 只标了段落部分行（如首行 noise）时，未覆盖行的正则兜底一并失去，且 Agent 未提交 statement 时无任何补偿 REQ——该段内容静默退出最终集。
调用链: retain(|r| !r.anchors.iter().any(命中覆盖)) → 整条 drop。
触发条件: Agent 对多行段落做部分区间标记（marks 区间粒度=块=物理行）。
实际影响: #219 兜底语义的召回洞：块级裁决 vs 段落级 REQ 的粒度错配。anchor 匹配不到块的防御分支（:140-147 None→保留）反而宁多勿漏——两分支语义相反。
当前测试: merge.rs 测试均单锚 REQ；无「多锚部分覆盖」用例。
缺失测试: 多锚 REQ 部分锚覆盖时的语义测试（丢弃 or 保留未覆盖行）。
建议方案: 多锚 REQ 改为「全部锚命中才丢弃」或「丢弃后若 Agent 无对应 submitted 则回退保留」；至少在 MergeStats 登记 partial_covered_dropped 计数。
证据:
```rust
// merge.rs:133-136
regex_reqs.retain(|r| {
    !r.anchors.iter().any(|a| …covered.contains_key(&idx))   // 任一命中即弃
});
```
审计置信度: SUSPECTED（语义确认；实际 Agent 标注行为是否产生部分覆盖未验证）
状态分类: 当前仍存在

---

**ID: PARSE2-010**
严重程度: P2
模块/文件路径/代码行号/函数或结构体: bid-parse/src/veto_rule.rs:355-387 `has_rule_pattern`（PROCEDURE 全串搜索）+ :131-146 `has_self_veto_consequence`（词表不含「不予受理/拒收/取消中标」主语形态）
问题描述: #226 规则说明层把「招标人主语 + 程序后果（不予受理/不予接收/不予开标/逾期送达）」判为规则说明 → 降级「其他/资格条件」。veto-gt 真值把这类条款标为**程序性否决 true**（missed-list 程序性否决 19 条；A01-GT-016「招标人拒收投标文件」/A01-GT-021「招标人不予受理」均为实锤）。防死锁取舍（#230 三轮收紧、dump 复核）与 GT 召回口径系统性冲突，且该损失面未在任何召回指标中单列披露。
调用链: veto_hit=true → is_rule_statement=true（PROCEDURE 命中/主语窗口命中）→ 分类降级。
触发条件: 「未按要求密封的投标文件，招标人有权拒收。」「逾期送达…招标人不予受理。」
实际影响: 程序性否决族召回缺口（有意换取「无永久 blocking」）；GT 对齐口径下 veto recall 上限被压低。
当前测试: extract_docx_veto.rs:89 断言该族落「其他」；extract_pdf_para.rs:82-90 同——把取舍钉成规范 ✅（测试-设计一致，但与 GT 口径的差未披露）。
缺失测试: 无（设计取舍面）——需要的是**口径披露**：veto-measure/健康检查报告应单列「规则说明降级致 GT-true 损失 N 条」。
建议方案: 短期在召回评测里分桶披露；长期给规则说明降级类 REQ 加 params.rule_statement=true + 类别「程序性要求」而非「其他」，保留可响应面（响应=按时送达/密封承诺）以解除死锁担忧。
证据:
```rust
// veto_rule.rs:378-386
const PROCEDURE: [&str; 6] = ["逾期送达","逾期提交","逾期未","不予受理","不予接收","不予开标"];
PROCEDURE.iter().any(|s| t.contains(s))
// veto-gt/missed-list.json：程序性否决 19 条（A01-GT-021「…招标人不予受理。」GT true）
```
审计置信度: CONFIRMED（代码+GT 工件双证；取舍自陈）
状态分类: 当前仍存在（有意取舍，未披露损失面）

---

**ID: PARSE2-011**
严重程度: P3
模块/文件路径/代码行号/函数或结构体: bid-mcp/src/tools.rs:763-767（addenda_names 前缀过滤）
问题描述: 主/补遗冲突检测的补遗件集合按 `starts_with("C-")` 过滤——真实补遗（kind=补遗、无前缀）不进冲突面。与 #196/PARSE2-001 同型前缀泄漏。
触发条件: 真实文件名补遗 + 与主文件同条款号异文。
实际影响: CONFLICTS 检测静默漏报（缺冲突对账 → 补遗覆盖语义丢失）。
当前测试: 冲突面测试（e2e/contract 类）均用 C- 前缀夹具名。
缺失测试: 无前缀补遗冲突测试。
建议方案: addenda_names 改按 kind_map（kind=补遗）过滤。
证据:
```rust
// tools.rs:763-767
let addenda_names: Vec<String> = svc.parsed().iter()
    .filter(|p| p.name.starts_with("C-"))
    .map(|p| p.name.clone()).collect();
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

**ID: PARSE2-012**
严重程度: P3
模块/文件路径/代码行号/函数或结构体: bid-parse/src/clause.rs:72-114 `merge_multiline_clauses`（生产零调用方）；extract.rs:958-983（DOCX 内联续行吸收）
问题描述: G1「条款边界与跨行合并单一真源」声明失实——生产 DOCX 续行吸收在 extract.rs 内联实现（终止条件=编号/章头/占位/强词/列举五类，比 merge_multiline_clauses 的四规则多三类信号），clause.rs 的合并函数仅被自测消费。双实现漂移：改其一不改其二时行为分叉。
触发条件: 无直接触发（死代码面）；风险在维护演进。
实际影响: 单一真源纪律破损（文档-实现不一致族）。
当前测试: clause.rs 自测完备（对自身规则）；无「extract 内联吸收与 clause::merge_multiline_clauses 同判」的一致性测试。
建议方案: extract 内联循环改调 merge_multiline_clauses（扩展其断点参数），或删除该公开函数并在文档改口径。
证据:
```rust
// extract.rs:970-977（内联吸收终止条件——与 clause.rs 四规则不同源）
if nt.is_empty() || clause::clause_boundary(nt).is_some()
    || re_chapter_line.is_match(nt) || vl::is_placeholder(nt)
    || vl::is_veto(nt) || vl::is_qual(nt) || vl::is_list_item(nt) { break; }
```
审计置信度: CONFIRMED
状态分类: 文档与实现不一致

---

**ID: PARSE2-013**
严重程度: P3
模块/文件路径/代码行号/函数或结构体: bid-parse/src/extract.rs:1169-1192 `extract_b2_ocr`
问题描述: B2 OCR 锚为合成物：excerpt=行1+行2 无分隔符直接 push_str、locator 追加「 + 行接」——锚文本不逐字节等于任何真实块文本（违反「excerpt=块原文、创建时固化」纪律）。下游按 (locator,text) 精确对齐的面（merge 索引、GT 块对齐、证据支持率回查 fail_text_mismatch 判据）会失配。
触发条件: B2 扫描件含「否决/签署/盖章」关键词行且下一行 ≥2 字符（几乎必触发拼接）。
实际影响: 该族 REQ 的锚无法回查到块（支持率/溯源面弱化）；双抽面（种子行 + 拼接锚重复计否决关键词行）。
当前测试: 无 B2 OCR 锚形态测试（本模块测试集不含 b2 用例）。
缺失测试: B2 锚=真实块文本（分锚或保真）的断言。
建议方案: 锚拆两个（种子行锚 + 续行锚），与 PDF 段落锚同纪律。
证据:
```rust
// extract.rs:1180-1186
let mut excerpt = b.text.clone();
if let Some(next) = blocks.get(i + 1) { …
    excerpt.push_str(&next.text);            // 无分隔拼接
    locator.push_str(" + 行接");             // 合成 locator
}
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

**ID: PARSE2-014**
严重程度: P3
模块/文件路径/代码行号/函数或结构体: bid-parse/src/review_table.rs:629-630 `is_linearized_header` 尾行
问题描述: `re_row.is_match(n) == false && false` 恒为 false——注释宣称的「编号数据行防误伤（re_row 兜底）」逻辑实际不存在（死表达式）。当前无害（编号行先被上游分支消费），但注释与代码不符，误导维护者以为有兜底。
当前测试: 无（死代码不可测）。
建议方案: 删除该行或改为真实判定 `!re_row.is_match(n)` 并补测试。
证据:
```rust
// review_table.rs:629-630
// 防误伤：形如「1 投标人名称…」的编号数据行不是表头（re_row 兜底）
re_row.is_match(n) == false && false
```
审计置信度: CONFIRMED
状态分类: 当前仍存在（无害死码）

---

**ID: PARSE2-015**
严重程度: P3
模块/文件路径/代码行号/函数或结构体: bid-parse/src/review_table.rs:287-291 `assemble_docx_group`（Detailed score 解析）
问题描述: 标准分列解析失败时 `.or_else(|| cells.iter().find_map(|c| parse_score(c)))` 全行任意格兜底——item/序号列含纯数字（如年份「2026」、数量「3」）会被采作 score。
触发条件: score_col 单元格非纯数字（「30分（四舍五入）」类）且行内他格有裸数字。
实际影响: 评分项 params.score 错值（#224 映射/validate_sums 输入污染）。
当前测试: parse_score_forms 只测单元格级；无兜底捞异物负向测试。
建议方案: 兜底范围限「分值列右侧相邻格」或去掉兜底改报缺（配合 PARSE2-006 账面）。
证据:
```rust
// review_table.rs:288-291
let score = cells.get(*score_col).and_then(|v| parse_score(v))
    .or_else(|| cells.iter().find_map(|c| parse_score(c)));
```
审计置信度: CONFIRMED（路径）；触发罕见
状态分类: 当前仍存在

---

**ID: PARSE2-016**
严重程度: P3
模块/文件路径/代码行号/函数或结构体: bid-parse/src/extract.rs 全链（push_req）；veto_lexicon 正通道
问题描述: 否决命中的 REQ 不携带任何 provenance（命中短语/规则 id/分支来源）；仅 #52 降级面有 params._veto_excluded。章头行内联正文（「第X章 标题+正文」同行）continue 时正文部分不入抽取面（同文件 :560-581）。
实际影响: ① 否决判定不可追溯（审计/复核需重放推断）；② 混合章头行正文丢失（低频）。
当前测试: 无。
建议方案: push_req 加 params._veto_hit（首个命中短语）——成本一行；章头行残文（剥「第X章」后 ≥N 字）转普通段处理。
审计置信度: CONFIRMED（观测性缺口）；正文丢失面 SUSPECTED
状态分类: 当前仍存在

---

## 5. 测试 Oracle 独立性评估（逐测试）

| 测试 | Oracle 来源 | 独立性判定 | 关键缺口 |
|---|---|---|---|
| extract_docx_veto.rs | 手写语义判例（issue 原文 + 真实语料判例回填，veto_rule 三轮收紧的 dump 复核样本入测） | **独立**（业务规则直接编码） | 全走 legacy 入口；非投标宾语后置否定无形（PARSE2-002） |
| extract_format_chapter.rs | A-04 实拍形态复刻 + 手写期望 | **独立** | 仅格式章守卫面；非格式章噪声保留面被钉为规范 |
| extract_dunhao.rs | 实机漏抽样例回填 + 手写期望（PDF/DOCX 双口径对称断言） | **独立** | — |
| extract_kind_dispatch.rs | #199 值域矩阵手写期望 | **独立** | B1 分支 `let _ = reqs_b1;` 零值断言（PARSE2-004）；with_kind 通道的否决/段落行为无测试 |
| nesting_corpus.rs | 真实语料不变量（join-back 全等、配对率 ≥0.85 下限、配对位置正确性） | **独立**（不变量式） | 诚实 SKIP（环境不可信分流按错误码前缀——#204 口径，非中文文案）✅ |
| real_corpus_223.rs | **抽取器自身历史产出基线**（923/317/409，#234 后兜底路口径） | **自证**（回归锁非独立真值）；且只断 `>=` 下限——膨胀型回归（如双抽 bug）不可检 | 生产路由 951→942 数字仅存注释、无测试锁定；GT 红线在 bid-mcp real_corpus_e2e（非本模块） |

**Oracle 自证风险结论**：本模块 6 测试中 5 个为手写独立判例（总体健康）；唯一的真实语料数值断言（real_corpus_223 基线）是真值来自**与生产同源算法的历史输出**的自证回归锁，且为下限断言；全部测试走 legacy `extract_requirements` 入口而生产走 `extract_requirements_with_kind`——测试-生产入口分叉（当前分发后内部同路径，风险为间接）。真正的独立真值（人工标注 GT 1752 true）在 artifacts/veto-gt 与 bid-mcp 红线测试，不覆盖本模块 B1/B2 通道与 kind 分发面。

## 6. 历史问题重核对账

| 历史问题 | 重核结论 |
|---|---|
| 健康检查 P0-①（PDF 行级碎片漏检） | **已修**：P0-A 段落重组在位（extract.rs:442-519），#35/#68 扩面注释完整；extract_pdf_para/format_chapter 锁定。CONFIRMED |
| 健康检查 P0-②（后置否定从句→其他） | **部分修复**：投标宾语形已覆盖（视为无效/投标无效/否则其投标将被否决）；非投标宾语形（否则该业绩无效）零覆盖 → PARSE2-002（P1）；另有 #226 规则说明降级损失面未披露 → PARSE2-010 |
| 健康检查 P1-③（其他类 82%） | 无专门治理：category_for 兜底「其他」+ 规则说明降级落其他；无占比告警（观测性缺口） |
| 健康检查 P1-④（PDF 面表格结构丢失） | review_table PDF 线性化面已建（#144），但识别失败静默空（PARSE2-006）+ 条目缺 score（PARSE2-005） |
| R3-S3-5（编号通道否决词优先） | 已修：两侧优先级链在位且有测试（numbered_clause_priority_chain / extract_pdf_para）。CONFIRMED |
| #233（DOCX 规则说明分类） | **已修且验证通过**：veto_rule.rs 三轮收紧 + 12 组判例测试 + PDF/DOCX 对称测试（rule_statement_para_symmetric_with_pdf / numbered_rule_statement_downgrades_not_veto）。CONFIRMED |
| #234（格式章表格头噪声，951→942） | **已修，合成面验证通过**（extract_format_chapter 7 测试锁定两行/同行章形态、动词放行、强审查旁路、碎片过滤）；**真实面仅兜底路下限锁定**（real_corpus_223: 923 floor，>= 断言）；生产路由 942 数字无测试锁定；非格式章残余噪声面仍在（PARSE2-007）。部分 CONFIRMED |
| #240 | 代码零引用（rg 全仓 #240 无命中）——951→942 数字与 #234 记录重合（extract_format_chapter.rs 头注）；如 #240 另有所指（可能是 matrix_blocking 131→122），同注记录，无独立代码锚点可重核 |
| #196/#199（kind 分发） | 分发面已修且有值域矩阵测试；但 merge 面（PARSE2-001）、冲突面（PARSE2-011）、B1/B2 通道（PARSE2-004）三处前缀判据残留——修复不完整 |

## 7. 结论速览

- **否决召回不足根因（一行）**：词表只收复合处置短语（非投标宾语后置否定从句零覆盖）+ #226 规则说明层把招标人主语程序性否决系统性降级「其他」（死锁规避与召回红线未解耦、损失面未披露）+ PDF/DOCX 双词表漂移。
- **评分表结构化输入现状（一行）**：DOCX 侧三表装配可用（score/score_rules 入 params）；PDF 线性化侧标题驱动装配可用但条目无 score/score_rules，识别失败静默空、合并单元格表头显式不支持——PDF 件评分面仍是弱输入。
- **P1 两项**：PARSE2-001（合并面前缀判据——Agent 优先语义对真实文件名失效+覆盖口径分裂）、PARSE2-002（非投标宾语后置否定零覆盖）。
- **测试 Oracle**：5/6 独立手写判例；唯一真实语料数值断言为自证下限基线；测试统一走 legacy 入口与生产 kind 入口分叉。

---
*本文件由模块审计代理 B2b（Task 4-b2b）生成；只读审计，未修改任何生产文件。所有行号基于 audit/v2-full-coverage @ a689701（生产代码与冻结基线 6105beb 一致）。*
