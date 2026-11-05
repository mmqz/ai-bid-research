# 模块审计 C3：bid-mcp 测试质量 B（其余全部测试文件 + bin 入口 + 测试→生产映射）

- 审计代理：4-c3（ai-bid-research 全仓审计 V2 · Task ID 4-c3）
- 冻结基线：main@6105beb（分支 audit/v2-full-coverage；生产/测试代码与基线一致）
- 范围：`bidcore/crates/bid-mcp/tests/` **全部 37 个测试文件**（36 .rs + 1 .py，19,596 行——含任务书点名 12 文件逐行读全 + 其余文件以 git ls-files 为准全数覆盖：头部+测试清单+关键断言/跳过/副作用面定向精读）＋ `src/bin/` 3 个生产入口（stdio.rs 34 / server.rs 49 / a8.rs 124 全读）＋ `examples/` 2 个验收探针（头部精读）
- 纪律：只读审计；未修改任何生产文件/测试/配置；未运行 cargo test/build（禁令遵守）；本文与 inventory-c3.csv 为本轮唯一新增产出
- 方法：逐文件通读 + 与 C1（module-audit-bid-mcp.md：MCP-013/005/003 等 19 项）交叉复核 + 与 B2b（PARSE2-001 merge 前缀判据）做 R5 测试→生产映射 + issue #241/#242/#185/#219/#144 原文对账（audit/evidence/github/）
- 测试总量：**约 229 个 #[test] 函数**（另有 mcp_sdk_e2e.py 1 套官方 SDK 验收脚本）

---

## 1. 覆盖矩阵（37 测试文件 + 3 bin + 2 examples）

| 文件 | 行数 | #test | R5 类型 | 被测生产面 | Oracle 来源 | 跳过机制 | 副作用（仓内路径） |
|---|---|---|---|---|---|---|---|
| synth_tender_eval.rs | 606 | 1 | REAL_CORPUS 评测 | 全链 25 工具经 dispatch_with | 人工参考答案 expected-requirements.json（ideal 面）+ engine_expect（自证面） | skip_if_fixture_missing（横幅） | **写 artifacts/synth-tender/run/latest/**（仓内） |
| mcp_scaffold_tools.rs | 593 | 2 | E2E+NEGATIVE | 纯 MCP 工具面全链（G1/G2/G3） | issue #80 验收文 + 契约 | **无跳过（缺件 panic）** | temp ws 不清理 |
| review_table_matrix.rs | 588 | 3 | CONTRACT+自检+合成基线 | #144 审查表抽取器（经全链） | issue #145 逐行转录 RowExpect（独立判例）+ 测试本地验证器（突变自检） | 无需（合成件自造） | temp（自清） |
| writing_acceptance_e2e.rs | 585 | 3 | E2E（#242 验收） | validate 幅幅门 + 导出门 1a/1b/1c + RT 闸 | issue #242 三面原文 | corpus_ready 横幅 | temp ws |
| export_tool.rs | 517 | 12 | INTEGRATION+NEGATIVE | export_bid_document 全语义 | 契约 §8 + docx 解包 | 无需（合成 REQ） | temp |
| evidence_source_tool.rs | 512 | 8 | INTEGRATION（#185） | register_evidence 来源验证 | issue #185 A/B/C 原文 | **无跳过（缺件 panic）** | 内存态零落盘 |
| bad_input_matrix.rs | 473 | 6 | NEGATIVE（红线收紧） | import_tender_files 坏件面 | 「坏件绝不静默成功」不变式（单一判定点 classify） | 程序生成坏件；超大件=外部件 SKIP | temp |
| input_format_matrix.rs | 383 | 8 | NEGATIVE（软口径） | 格式路由/损坏/边界 | 不变量（软断言） | fixtures 目录扫描缺格式 SKIP | temp |
| coverage_tools.rs | 375 | 13 | INTEGRATION（#219/#244） | 覆盖三工具 + merge 接线 | #219 契约 + 合并守恒式 | 仅 1/13 用 corpus_ready；**3 处静默 early-return** | temp ws 不清理 |
| evidence_content_tool.rs | 373 | 4 | INTEGRATION（#217） | search_evidence 正文面/register 出参 | issue #185 E/F 原文 + 夹具语义防线 | **无跳过（缺件 panic）** | 内存态 |
| addendum_waiver_tool.rs | 370 | 4 | CONTRACT+NEGATIVE（#184） | record_addendum_waiver + readiness 接线 | issue #184 验收 | skip_if_fixture_missing | temp 上传区 |
| agent_mandatory_gate.rs | 349 | 4 | E2E（#241 验收） | agent_mandatory 接线（矩阵/起草/导出三门） | issue #241 裁定原文 | corpus_ready 横幅 | temp ws 不清理 |
| import_tender_files_tool.rs | 770 | 19 | CONTRACT+E2E+stdio | import 全语义 + #196 真名回归 | 借鉴轨 gen-log（tables=1）+ issue 判例 | skip_if_fixture/env ×15 处 | temp 上传区 |
| search_evidence_tool.rs | 955 | 19 | INTEGRATION | search/register 双面 | issue #145 判例 | 无需（合成台账） | 内存态 |
| set_matrix_override_tool.rs | 851 | 21 | CONTRACT+NEGATIVE（#226） | set_matrix_override + #182 签域 | issue #226 + 签域契约 | corpus_a_ready | corpus-A 真件 |
| validate_tool.rs | 738 | 22 | UNIT/规则矩阵 | validate 8 规则 | 规则判例（手写期望） | 无需（合成） | temp |
| score_tools.rs | 787 | 12 | INTEGRATION | score_price/section/document | issue #100 判例 + B1 双跑对账 | 无需 | temp |
| cursor_pagination.rs | 793 | 8 | INTEGRATION+确定性 | 游标分页三工具 | issue #11 验收 + 逐字节相等 | 无需（循环生成 12k） | temp |
| global_invariants.rs | 826 | 2 | E2E 不变式+突变自检 | 跨模块五接缝 | #171 不变式判例（check_* 纯函数+注缺陷必红） | fixtures_ready 横幅 | temp |
| e2e_pipeline.rs | 287 | 2 | E2E（corpus-A） | 全链 | Lead 验收线 | corpus_ready 横幅 | temp |
| e2e_artifacts.rs | 1225 | 1 | E2E+产物落盘 | 全链 + pdf 软门 | issue #80 §7 产物清单 | pdf 链软门 | **写 artifacts/e2e/<run-id> + latest**（仓内） |
| real_corpus_e2e.rs | 1067 | 1 | REAL_CORPUS+GT 红线 | 全链（3 真件） | 人工 GT ground-truth.json + veto-measure 同口径算法 | 缺件 SKIP + **REQ>1500 解析器不同源 SKIP** + #169 乱码 SKIP | **写 artifacts/real-corpus-e2e/latest/**（仓内，先删后写） |
| export_readback.rs | 1285 | 6 | 独立回读（#172） | 导出产物（不信自报） | 独立解析器回读（评委视角） | pdfium/CJK 软门 | temp |
| real_llm_path.rs | 622 | 1 | MANUAL 两相位 | RealGenerator HTTP 链 + bin/stdio 真传输 | 人肉中继 + 503 错误语义 | **默认 SKIP（BID_REAL_PATH_WS/PHASE）** | 指定 ws |
| subagent_llm_path.rs | 906 | 1 | MANUAL 两相位 | 子代理多章 + MCP 全贯通 | skill 偏移评估 | 默认 SKIP（BID_SUBAGENT_*） | 指定 ws |
| mcp_sdk_e2e.rs/.py | 65/149 | 1 | E2E（官方 SDK 进程级） | bin/server 协议层 | MCP 规范 + 25 工具清单 | python3+mcp 缺席 SKIP（=1 强制） | 纯内存子进程 |
| dispatch_contract.rs | 161 | 8 | CONTRACT（骨架纪律） | TOOL_NAMES/dispatch/dispatch_json | 契约 §0 | 无需 | 全局内存态 |
| save_version_tool.rs | 126 | 3 | INTEGRATION | save_version（经全局 dispatch） | issue #42 | 无需 | temp（自清） |
| draft_tool.rs / draft_generator_switch.rs | 301/291 | 9/9 | CONTRACT+NEGATIVE | draft 契约/生成器四态 | 契约 §6 + issue #68/#100 | real/auto 无 key 面：BID_LLM_API_KEY 在场→SKIP | 内存态 |
| check_quality_tool.rs / quality_summary.rs | 288/259 | 6/3 | CONTRACT | check_bid_quality | issue #68 §2.1/2.2 + #145 §1 | 无需 | temp（自清） |
| extract_matrix_tools.rs | 255 | 6 | CONTRACT+真件 | extract/matrix | 契约 §3/§4 + GT 埋点定性 | corpus_a_ready 横幅 | 内存态 |
| partial_failure_gate.rs | 314 | 6 | NEGATIVE（P0-1） | 起草输入完整性闸 | 地图 P0-1 场景判例 | skip_if_fixture（1/6） | temp |
| scale_1330.rs | 250 | 1 | 规模红线 | 调度/落盘/导出 | Lead 4.2 红线（120s/2GB） | 非 Linux SKIP 横幅 | temp |
| pdf_export.rs | 301 | 5 | E2E（soffice 通道） | export pdf | Lead 验收线 | soffice/pdftotext/pdfinfo 缺→SKIP | temp |
| bad_input/…（上行已列） | — | — | — | — | — | — | — |
| **bin/stdio.rs** | 34 | — | 生产入口 | dispatch_json 逐行裸 JSON | — | — | — |
| **bin/server.rs** | 49 | — | 生产入口 | McpSession JSON-RPC 分帧 | — | — | — |
| **bin/a8.rs** | 124 | — | 生产入口（CLI 验收壳） | 服务层直连非 MCP | — | — | — |
| examples/blocking_probe.rs / req_dump.rs | — | — | MANUAL 探针 | #226 层2/#234 抽取对照 | — | — | — |

---

## 2. 特别任务裁定

### 2.1 #241/#242 门禁验收真实性（writing_acceptance_e2e.rs + agent_mandatory_gate.rs）——**CONFIRMED 真实（mock 面）**

- **「必选项未响应拦起草」**：`agent_mandatory_draft_gate_blocks_until_receipt_override`（agent_mandatory_gate.rs:192-252）走 MCP `draft_response_section` → 断言 Err 消息**同时点名**「必选条款尚未响应」与「REQ-A0001」；覆写后反向断言 REQ-A0001 退出阻塞消息。对照面 `non_mandatory_agent_marks_never_block_any_gate`（mandatory=false → blocking=0 + export_blockers 空 + check_export_ready ok）——**唯一差异法**夹具设计，因果干净。
- **「篇幅不足拒达标/导出」**：`writing_acceptance_short_draft_blocked_from_export`（:348-467）断言链完整：validate ok=false + E_CHARS_BELOW_MIN=BLOCKER 锚定短稿章节 → PreExportConfirm Rejected 派生 1 RT → 导出面 1（E_STATE）→ 面 2（同门补 Approved 后 RT 未关单仍 E_UNPROCESSABLE，拒因点名 RT）→ **零半成品断言**（拒导出时产物不得在盘）→ 修复闭环（补写→关 RT→重审→EXPORTED）。
- **d4c5470「即使审核直接批准也禁导出」**：`writing_acceptance_export_independent_of_review_approval`（:486-585）**已覆盖且断言强度高**——面A 在拒导出前先断言 `gate_approved(PreExportConfirm)==true` **且** `open_blockers().is_empty()`（证明记审面全绿、RT 闸为空），拒力只能来自门 1c 独立复算；断言拒因点名「写作验收门」+「E_CHARS_BELOW_MIN」；面C 空叶 + 直接批准 → E_SECTION_EMPTY 同门拒且**不误报**已起草叶篇幅。与 issue #242 body 描述的三面逐字对上。✔ 覆盖成立。
- **弱面（如实登记）**：① 三测试全部 corpus_ready 门控——corpus-A 缺件时静默 return（横幅已打），CI 无件则零验证；② 起草/短稿注入走服务面 `svc.draft_section`（Tiny 生成器——测试特权，文件头已显式标注为 e2e_pipeline 同款纪律）；③ 覆写后的起草放行断言接受「Err 但不含 REQ-A0001 **或** Ok」两态（其余阻塞如 MissingAddendum 与本面无关——弱化有据）。
- **「真实 LLM 验收未完成」属实且诚实**：issue #242 body 原文自认「本环境无 BID_LLM_API_KEY/BID_LLM_MODEL/.mang/llm.json——真实 LLM 全链未执行（不伪造通过）」；对应代码面 real_llm_path.rs 是**人肉中继两相位 MANUAL 测试**（默认 SKIP），subagent_llm_path.rs 同型（子代理作者）。writing_acceptance_e2e 头注明确「本套件钉 mock（确定性），真实 LLM 全链走 real_llm_path 两相位——无凭据时不伪造通过」。**结论：历史资料所述未完成状态与当前代码一致，且非假绿（诚实降级登记）。**

### 2.2 synth_tender_eval.rs——Oracle 双面性：ideal 面独立、engine_expect 面自证

- **ideal_category/excerpt_key/trap**：fixtures/synth-tender/参考答案/expected-requirements.json 的人工核对键（源文本子串）——独立于生产抽取规则，作差距报告分母。**但**：
- **engine_expect（will_extract/category/occurrences）与生产抽取器同源风险**：该字段是「预判引擎行为」的登记，作者与抽取规则开发者同源 → `n_pred_ok`（推演准确度）指标是**自证回归锁**（测的是「引擎是否符合作者对引擎的预判」，非独立真值）；B2b 在 real_corpus_223 上同型结论（923/317/409 为抽取器自身历史产出下限基线）。
- **红线仅为坑位最小集**（T1/T1b/T2/T3/T6×9 硬编码 excerpt_key + 类目下限 否决≥40/资格≥6/评分≥4/技术≥12/格式≥1 + n_reqs≥60）——**n_miss（命中率）不进断言**（G1-G7 已知缺口「登记在案的行为」）：抽取命中率退化不红灯，只有坑位条款丢失或类目规模塌方才红。这是**有意的差距评测设计**，但意味着「命中 {n_hit}/{expected.len()}」只在 report.md 里人读。
- 历史基线数字：n_reqs≥60（预测 81）、C 件 pages==2、E 件 chunks≥5——夹具耦合锁（换夹具即红）。
- 环境软门：PDF 链缺 poppler/tesseract → C/E 面 ENV_MISSING 不判红；但 pdf_text 纯 Rust 兜底后 C 件**必须**成功（逐件判比整批判更严——正面）。D 件 E_PARTIAL fail-closed + waive_unready_inputs 受限起草闭环。

### 2.3 coverage_tools.rs（tests）× #244 × PARSE2-001——**闭环测试在位，但全部跑在「前缀永远为真」的夹具名上**

- #244 全块覆盖闭环现状：13 测试覆盖三工具契约（分页/verdict 回显/size 上限/越界/重叠/越区间/空 statement/非法 category 零副作用）+ 合并三语义（Agent 优先压制正则、无标记严格 no-op + CMP-0001 序不动、整集替换 dropped==t1 + added<0）+ **PR #227 checkpoint 缺口修复验证**（submitted_persists_across_restart——drop 后重开工作区 agent_requirements==1）。闭环本身质量高。
- **R5 映射核心发现（MCT-101）**：全部合并面测试的导入文件名是 **「A-招标文件.docx」**（corpus-A 夹具名，带 A- 前缀）→ 生产 `merge.rs:71-76 has_extraction_face(name)` 的前缀判据（A-/C-/B1/B2）**恒为真** → PARSE2-001（P1，B2b CONFIRMED：真实文件名静默绕过 Agent 优先合并、与 get_coverage_report 口径分裂）**在测试面不可见**。对照：#196 在 extract 分发面有专门的真名回归锁 `import_tender_files_tool.rs:695 real_filename_with_registered_kind_extracts_via_tool`（word-complex.docx 无前缀 + kind=招标文件 → total>0）——**merge 面没有对应用例**（真名 + submit_block_marks + extract → 断言 merge.regex_dropped_covered/agent_requirements——该测试今天会红，即缺口实锤）。测试→生产映射结论：**#244 验收测试没有在真实文件名场景下验证合并判据，前缀判据缺陷被夹具名系统性遮蔽**。
- 另：3 处静默 early-return（`if total < 4 { return; }` ×2、`if total < 10 { return; }`）无横幅——违反本仓 issue #9 自定 SKIP 纪律（同文件 submitted_persists_across_restart 却用了 corpus_ready 横幅——同文件双纪律）。

### 2.4 bin/stdio.rs + bin/server.rs——MCP-013 深挖（生命周期逐行复核）

两文件合计 83 行，逐行确认 C1 结论并细化：

| 生命周期面 | stdio.rs | server.rs | 证据 |
|---|---|---|---|
| **panic 隔离** | 无。任何工具 panic 直接穿透 main → 进程死 | 无（`session.handle_line` 无 catch_unwind；全仓 grep catch_unwind 零命中） | stdio.rs:13-34 / server.rs:39-47；state.rs:54-55 注释「一次 panic 不终止传输面」**误导**（锁中毒恢复分支生产不可达） |
| **stdin 读错** | `eprintln + break` → **退出码 0**（静默成功退出——宿主无法区分正常 EOF 与管道错误） | 同 | stdio.rs:17-23 / server.rs:27-33 |
| **stdout 写错** | `break`（无 stderr 报告） | 同 | stdio.rs:29-31 / server.rs:41-43 |
| **请求超时** | 无。dispatch_json 同步执行；真实 LLM 起草（服务层 60s×2 重试口径）期间**单线程循环 + lib.rs::dispatch 的全局 Mutex**（state.rs with_global）→ 后续请求（含只读）全部排队 = 宿主观感假死 | 同（handle_line 同步） | transport.rs 仅 14 行 trait；两 bin 均无超时/读侧 deadline |
| **超大输出** | `writeln!("{reply}")` 无大小护栏——超大 rows（limit≤1000×行宽）单行整发；无截断/分帧上限（客户端侧缓冲压力未测） | 同 | stdio.rs:28-32 |
| **僵尸子进程** | 两 bin 自身不 spawn 子进程；但 export pdf 面经服务层 spawn soffice——若 soffice 挂起，无超时杀死机制 → 整个 server 挂起（soffice 超时是否存在需运行时验证） | 同 | NEEDS_RUNTIME_VERIFY |
| **信号处理（Ctrl+C/SIGTERM）** | 无 handler。数据落盘依赖服务层「每次写面 checkpoint」（stdio.rs:6-8 头注承诺）→ 大部分状态已落盘；但**进行中请求无错误应答**（客户端只见连接断开），导出面 staging→final 的中途 kill 是否留半成品未验证 | 同 | 需运行时验证 |
| **BID_WS 损坏** | 启动即 panic（state.rs fail-closed，PA-012）——宿主表现为「连接立即断」 | 同 | 头注 9 行自认 |

**测试覆盖现状**：mcp_sdk_e2e.rs（python 缺席默认 SKIP）仅覆盖 server bin 的握手+list+call happy path；import_tender_files_tool.rs:595 `stdio_end_to_end_real_docx_import` 覆盖 stdio bin 两行请求 happy path；real_llm_path/subagent_llm_path 用 StdioProc 走真实传输面但**默认 SKIP（MANUAL）**。**零测试覆盖**：panic 注入→宿主可观察错误、长请求阻塞行为、stdin 中途 IO 错误、SIGTERM 落盘一致性、超大单行应答。a8.rs 为 CLI 验收壳（非 MCP 面，#183 围栏根=文件所在目录，有注释）——无自动化测试（MANUAL cargo run）。

### 2.5 bad_input_matrix.rs + input_format_matrix.rs——负向覆盖广度

- **bad_input_matrix（收紧层，质量高）**：单一判定点 `classify()` 把「坏件 → ① E_* 信封 或 ② Ok+per_file 显式登记失败」二选一定为红线，`silent_success` 直接红——覆盖 0 字节（docx+pdf 双臂 + **#174/#175 状态不推进 DRAFT 断言**）、截断 docx（PK+垃圾）、非 PDF 头（LCG 伪随机 + **造件自检断言无 %PDF- 魔数**——防测错分支）、合法 ZIP 非 OOXML（zip crate 现造 + 造件自检是合法 ZIP + 诱骗部件名 word/document.xml）、混合批次（E_PARTIAL + 好件 chunks>0 非空壳 + parse_failed==1 + partial≥1）、超大页数件（**外部件 MANUAL**：默认路径残缺 + BID_BAD_INPUT_DIR 启用——CI 恒 SKIP，红线从未在 CI 跑过）。
- **input_format_matrix（软层，已被收编但未删）**：0 字节/截断/撒谎扩展名/不存在路径/目录当文件五测试全部 `Ok(...) => 宽断言 / Err => 合理` 双接受（0 字节 Ok 臂断言 `ok_n==0 || n>0` 近乎恒真）——bad_input_matrix 头注明言「软口径——坏件被判成功也未必红」并自述为其存在理由。**当前仍并列在跑**：软层绿不能证明什么（收紧层才是真闸），但也不锁坏行为（负资产=维护噪音+虚假安全感）。文件名边界测试（超长/全角/../CON/NUL）为**纯 no-panic 断言**（`let _ = import(...)`——能返回就算过，无错误码校验，fuzz-lite 性质）。
- **广度结论**：坏件「静默成功」面（最高危）被 bad_input_matrix 单点收紧且造件自检纪律好；但「加密 PDF」「超大真件」「多文件组合坏件」「编码攻击类文件名」为 0 覆盖（部分依赖外部件）。

---

## 3. 问题记录

```
ID: MCT-100
严重程度: P1
模块: bid-mcp 生产入口生命周期（MCP-013 测试面深挖）
文件路径: bidcore/crates/bid-mcp/src/bin/stdio.rs + src/bin/server.rs
代码行号: stdio.rs:16-33 / server.rs:26-48
函数: main（两 bin）
问题描述: bin 生命周期四大面零测试：① 工具 panic 无隔离（穿透杀进程、无 -32603 应答）；② stdin 读错 break 退出码 0（宿主无法区分 EOF 与管道断）；③ 无请求超时——真实 LLM 60s×2 期间单线程+全局 Mutex 使全部后续请求（含只读）排队假死；④ 无信号处理（Ctrl+C/SIGTERM 进行中请求无错误应答；导出 staging 中途 kill 是否留半成品未验证）。
调用链: 宿主 → stdin line → dispatch_json/handle_line → 服务层（panic/长调用） → 进程死/挂起
触发条件: 任意工具 panic；长 LLM 生成期间并发请求；管道错误；SIGTERM
实际影响: MCP 会话整体崩溃无错误面；宿主表现为「工具无响应」；state.rs:54-55「panic 不终止传输面」注释与实现相反
当前测试: mcp_sdk_e2e.rs（python 缺席默认 SKIP，happy path）；import_tender_files_tool.rs:595 stdio happy path；real_llm_path/subagent_llm_path（MANUAL 默认 SKIP）
缺失测试: panic 注入→单请求 -32603 且传输面存活；读错/写错路径；超时行为；SIGTERM 落盘一致性；超大单行应答
建议方案: handle_line 包 catch_unwind（panic→-32603 应答）；读侧 recv 超时或请求级超时；读错改非零退出码+stderr；修正 state.rs 注释
证据: server.rs:39-47 match session.handle_line(trimmed, bid_mcp::dispatch) { Some(reply) => { if writeln!(...).is_err() { break; } ... } None => continue }（无 panic 捕获/超时/错误应答通道）
审计置信度: CONFIRMED（①②③静态可证；soffice 挂起传导与 SIGTERM 半成品面 NEEDS_RUNTIME_VERIFY）
状态分类: 当前仍存在
```

```
ID: MCT-101
严重程度: P1
模块: bid-mcp tests × 生产 merge 前缀判据（R5 映射主发现）
文件路径: bidcore/crates/bid-mcp/tests/coverage_tools.rs:209-318（三个 merge 测试）+ tests/agent_mandatory_gate.rs:100-107（导入 A-招标文件.docx）
代码行号: 见上；生产缺陷本体 = bid-parse/src/merge.rs:71-76 has_extraction_face（PARSE2-001，B2b CONFIRMED P1）
函数: extract_merge_agent_priority_with_regex_fallback / extract_no_marks_is_strict_noop / extract_remarks_drop_covered_regex / gated_project
问题描述: #244 覆盖闭环与 #241 门禁验收的全部合并面测试都用带 A- 前缀的夹具文件名——has_extraction_face 前缀判据恒真，真实文件名（无前缀）下 Agent marks/submitted 静默失效的生产缺陷被夹具名系统性遮蔽。#196 在 extract 分发面有真名回归锁（import_tender_files_tool.rs:695），merge 面无对应用例。
触发条件: 真实用户文件名 + Agent 已 submit_block_marks → merge.regex_dropped_covered=0 / agent_requirements=0 / uncovered 统计失真（与 get_coverage_report 口径分裂）
实际影响: #219「Agent 优先」语义对真实文件整体旁路——覆盖闭环验收（#244）在真实场景不成立
当前测试: 有（但全在前缀为真场景）；缺真实文件名场景
缺失测试: 无前缀文件名 + submit_block_marks + extract → 断言 merge 统计与覆盖语义（今天跑会红——正是需要的回归锁）
建议方案: 修 PARSE2-001（has_extraction_face 改 kind 判据）同时补真名 merge 回归测试；短期先落测试标红登记
证据: coverage_tools.rs:66-73 导入 "filename": "A-招标文件.docx"（所有 merge 用例同一夹具名）
审计置信度: CONFIRMED
状态分类: 当前仍存在（测试缺口）；生产缺陷已由 B2b PARSE2-001 登记
```

```
ID: MCT-102
严重程度: P2
模块: bid-mcp tests SKIP 面积
文件路径: writing_acceptance_e2e.rs / agent_mandatory_gate.rs / coverage_tools.rs(1/13) / e2e_pipeline.rs / extract_matrix_tools.rs / set_matrix_override_tool.rs / global_invariants.rs / synth_tender_eval.rs / partial_failure_gate.rs / bad_input_matrix.rs(部分)
问题描述: 十余个核心验收套件以 corpus_ready/skip_if_fixture_missing 门控：夹具缺席时测试函数 return（横幅打印）→ cargo test 报 passed——「SKIP=绿」在测试摘要层面与真绿不可区分（PLAN.md 五状态纪律要求不得混同）。corpus-A/synth-tender 均已入 git（本仓检出必在场），风险实体化为浅克隆/LFS 失效/单测过滤运行的场景。
实际影响: 验收测试（#241/#242 门禁、GT 红线）可在无件环境零断言通过
当前测试: 即本问题本体
缺失测试: SKIP 应显式计数/汇总（如 env_preflight 全局登记表 + 测试末尾打印 SKIP 清单），CI 区分 pass/skip
建议方案: cargo test 层面把 SKIP 变成显式可统计面（如 --format json 汇总 or 自定义 harness 标记）；或至少 CI 加 fixtures 在位预检 job
证据: writing_acceptance_e2e.rs:48-52 corpus_ready → skip_if_fixture_missing(横幅) → return
审计置信度: CONFIRMED（机制）；影响概率 SUSPECTED（需特定环境）
状态分类: 当前仍存在（设计取舍，issue #9 有意为之——但摘要面假绿未收口）
```

```
ID: MCT-103
严重程度: P2
模块: real_corpus_e2e GT 红线
文件路径: bidcore/crates/bid-mcp/tests/real_corpus_e2e.rs:871-884
函数: real_corpus_full_chain
问题描述: L1/P GT 红线（#123 基线 309/318=0.9717、291/302=0.9636 硬编码）带环境条件跳过：total_reqs > REQ_PLAUSIBLE_MAX(1500) → 打印「解析器与 GT 标注不同源」横幅后 return——poppler/纯 Rust 兜底环境下（CI 常见）红线整体不执行；#169 乱码环境再跳一次。GT 缺失才显式 assert 失败，其余两跳都是绿退化为 SKIP。
触发条件: 环境解析器非 pdfium（块粒度不同 → REQ 数膨胀）
实际影响: 召回/精确率红线只在 pdfium 路径环境成立——「GT 红线进 CI」的承诺（issue #131）在多数 CI 环境实际是 SKIP
当前测试: 本体；红线数字本身是好基线（历史产出锁——veto-measure @51874cd 逐位一致）
缺失测试: 无（环境问题非测试缺口）；建议 CI 至少一个 pdfium 在位 job 跑红线
建议方案: CI 矩阵中显式加 pdfium 通道 job；SKIP 时把 redline 字段写进 summary 标记 skipped=true（当前直接 return 连 summary 都不写）
审计置信度: CONFIRMED（代码路径）；CI 实际跳过频率 NEEDS_RUNTIME_VERIFY
状态分类: 当前仍存在
```

```
ID: MCT-104
严重程度: P2
模块: synth_tender_eval Oracle 独立性
文件路径: bidcore/crates/bid-mcp/tests/synth_tender_eval.rs:100-126/291-328
函数: load_expected / synth_tender_eval（④ 对照段）
问题描述: 参考答案双面性：ideal_category/excerpt_key 为人工核对真值（独立）；engine_expect.{will_extract,category,occurrences} 与生产抽取规则同作者——「推演准确度 n_pred_ok」是自证回归锁；且 n_miss（命中率）不进断言（G1-G7 已知缺口设计性放行）——抽取命中率退化只有坑位最小集（T1/T1b/T2/T3/T6×9）与类目下限兜底。
实际影响: 非坑位条款的召回退化静默；推演准确度指标不能当独立质量证据引用
当前测试: 本体
缺失测试: 独立第二人复核的 expected-requirements 增量；或把 n_miss 相对上轮报告的回归比较断言
建议方案: report.md 已如实输出 n_hit/n_miss——补一条「n_hit 不得低于上次落盘 report 的值」的软回归（产物已在 artifacts/synth-tender/run/latest）
审计置信度: CONFIRMED
状态分类: 当前仍存在（部分为登记在案的有意设计）
```

```
ID: MCT-105
严重程度: P2
模块: 测试副作用写仓内路径（工作树污染）
文件路径: synth_tender_eval.rs:41-43（artifacts/synth-tender/run/latest）/ e2e_artifacts.rs:155,1131（artifacts/e2e/<run-id>+latest）/ real_corpus_e2e.rs:46-48,301-303（artifacts/real-corpus-e2e/latest——先 remove_dir_all 再写）
问题描述: 三个 E2E 测试直接写仓库工作树内 artifacts/（git check-ignore 验证：NOT-IGNORED）——cargo test 后工作树出现新增/修改文件。**历史事故已发生一次**：issue #242 body 自认 f3fe702 误把 real-corpus-e2e 中间明细（每件 2-3 万行 03_requirements.json、571-839KB docx，合计约 10.5 万行）提交入库，后 force-push 重写剔除。
触发条件: 任何人在仓库根跑 cargo test -p bid-mcp 后 git add -A / git commit -a
实际影响: 工作树脏 + 高概率再次误提交大产物（纪律靠人肉「只 add 明确路径」维持）
当前测试: 无测试问题——是测试自身的副作用问题
建议方案: 产物根改写到 env 可配目录或 target/ 下（Cargo 惯例）；或给 artifacts/{synth-tender/run,e2e,real-corpus-e2e} 加 .gitignore（现仅 bid-health-check 子目录有 .gitignore 先例）
审计置信度: CONFIRMED
状态分类: 当前仍存在（PR #242 已部分收敛入库纪律，但机制未变）
```

```
ID: MCT-106
严重程度: P3
模块: review_table 真件覆盖
文件路径: bidcore/crates/bid-mcp/tests/review_table_matrix.rs:316-411（build_review_docx 合成件）vs RowExpect（真实呼中件 #145 转录）
问题描述: #144 审查表抽取器的验收在【测试内合成的 docx 复刻件】上跑（docx-rs 现造三表+两列同名怪癖+序号跳6）——真实招标文件第 6 章三表从未作为输入跑过 #144 抽取器（corpus-A 有真件但无此断言面）；合成复刻保真度依赖人工转录。
当前测试: verifier_selfcheck（突变×5 自检——test-of-test 纪律好）+ synthetic_review_docx_parses_today（基线只打印不断言三表覆盖——有意留档）+ review_matrix_semantics_full（原 #[ignore] 已移除、当前激活 ✔）
缺失测试: corpus-A/呼中真件的审查表抽取对账（RowExpect 直接对真实件产物）
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-107
严重程度: P3
模块: 官方 SDK E2E 默认跳过
文件路径: bidcore/crates/bid-mcp/tests/mcp_sdk_e2e.rs:37-52 + tests/mcp_sdk_e2e.py
问题描述: 唯一进程级协议验收（initialize 协商/tools/list 25 个/inputSchema/call_tool 错误映射/-32602）在无 python3+mcp 包环境默认 SKIP（BID_MCP_SDK_E2E=1 可强制——设计诚实）；另注释漂移两处：.rs:7 说「20 工具」、.py:31 说「24」（实际断言 25——断言正确、注释滞后）。
实际影响: bin/server 协议层在无 Python CI 上零进程级验证
审计置信度: CONFIRMED
状态分类: 当前仍存在（环境门有意设计；注释漂移另计）
```

```
ID: MCT-108
严重程度: P3
模块: oversized_pdf MANUAL 用例默认路径残缺
文件路径: bidcore/crates/bid-mcp/tests/bad_input_matrix.rs:424
函数: oversized_pdf_never_silently_empty
问题描述: DEFAULT_DIR = r"C:Usersyezi6Downloadswork标书"——反斜杠在源码里已丢失（raw string 字面无 \），Windows 上也永不解析；只有设 BID_BAD_INPUT_DIR 才能启用 → 该红线（超大件不得 0 页 0 块空成功）在任何默认环境（含原用户机）都不会跑。
证据: const DEFAULT_DIR: &str = r"C:Usersyezi6Downloadswork标书";
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-109
严重程度: P3
模块: coverage_tools 静默 early-return
文件路径: bidcore/crates/bid-mcp/tests/coverage_tools.rs:126,151,175
问题描述: `if total < 4 { return; }` ×2、`if total < 10 { return; }`——无横幅静默跳过（违反本仓 issue #9 自定纪律；同文件 submitted_persists_across_restart 用了 corpus_ready 横幅——同文件双纪律）。corpus-A 已入仓且块面 >10，实际触发概率≈0。
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-110
严重程度: P3
模块: 夹具缺席处理纪律不一致（fail-loud vs SKIP）
文件路径: evidence_source_tool.rs:153-155 / evidence_content_tool.rs:81-85（panic!("夹具缺失")）/ mcp_scaffold_tools.rs（无任何预检——缺件时 ok_result panic）
问题描述: 同仓两种纪律并存：验收套件多数走 issue #9 SKIP 横幅；#185/#217/mcp_scaffold 走 panic 硬红。浅克隆下后者红得诚实（更好），但行为不统一使「哪些测试会在缺件环境炸」不可预期。
审计置信度: CONFIRMED
状态分类: 当前仍存在（文档与实现不一致类——issue #9 纪律描述未覆盖这批文件）
```

```
ID: MCT-111
严重程度: P3
模块: draft_generator_switch 环境条件跳过
文件路径: bidcore/crates/bid-mcp/tests/draft_generator_switch.rs:261-263,277-279
问题描述: real 无 key 响亮拒 / auto 降级 mock 两面在 BID_LLM_API_KEY 在场的开发机上 SKIP（横幅）——降级/拒路径在配 key 环境零验证（设计取舍：key 在场时 auto 走真实调用是 feature）。
审计置信度: CONFIRMED
状态分类: 当前仍存在（有意设计）
```

```
ID: MCT-112
严重程度: P3
模块: PDF 导出通道 CI 不钉
文件路径: bidcore/crates/bid-mcp/tests/export_tool.rs:222-241（双接受 Ok|E_UNPROCESSABLE+soffice 指路）+ tests/pdf_export.rs:40-53（soffice/pdftotext/pdfinfo 缺→SKIP）
问题描述: 无 LibreOffice 的 CI 上 pdf 转换正路径从未验证（pdf_export 五测试全 SKIP；export_tool 的 pdf 用例接受「转换成功」或「环境缺失诚实拒」两态——无法区分）。红线上仅 100 页 <30s 在有 soffice 环境成立。
审计置信度: CONFIRMED
状态分类: 当前仍存在（issue #9 软门设计）
```

```
ID: MCT-113
严重程度: P3
模块: 全局单例测试态
文件路径: bidcore/crates/bid-mcp/tests/dispatch_contract.rs + tests/save_version_tool.rs
问题描述: 两文件经 lib.rs::dispatch 使用 LazyLock 全局 BidService（纯内存）——同二进制内并行测试共享单例（无 reset 机制）。当前用例面（get_bid_project_state 纯函数探测 / save_version 文件面）实际不碰撞；未来在全局面加写操作测试将引入顺序耦合。dispatch_contract.rs:45 `unimplemented_tools_return_typed_unimplemented_not_ok` 当前遍历空数组——恒绿空转（骨架纪律保留声明，但清单收缩后该测试零断言，须人工回填才有效）。
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-114
严重程度: P2
模块: #241/#242 验收残留弱面（总体结论：验收真实）
文件路径: bidcore/crates/bid-mcp/tests/writing_acceptance_e2e.rs:213-216 等 corpus_ready 门 + :364-382/:496-510 服务面直调注入 + agent_mandatory_gate.rs:242-251 两态放行断言
问题描述: ① 三套验收全部依赖 corpus-A 在位（缺件即零验证，见 MCT-102）；② 短稿/公司名注入走 svc.draft_section 测试特权（MCP 面被绕——VALIDATING 态本就无法经 MCP 重起草，特权合法但意味着「MCP 契约面无法表达重写」这一产品事实未被测试记录）；③ 覆写后放行断言接受 Err-不含-目标条款 或 Ok 两态（弱化有据：MissingAddendum 等无关阻塞可能在场）。
审计置信度: CONFIRMED
状态分类: 当前仍存在（权衡登记，非缺陷）
```

```
ID: MCT-115
严重程度: P3
模块: input_format_matrix 软断言层残留
文件路径: bidcore/crates/bid-mcp/tests/input_format_matrix.rs:161-171（0 字节 Ok 臂 ok_n==0||n>0 近恒真）/ :273-302（文件名边界纯 no-panic 断言）
问题描述: 8 个用例中 5 个双接受软断言 + 1 个纯不崩断言——被 bad_input_matrix 明言收编收紧后，软层继续在跑：绿不代表任何坏件语义成立（维护噪音+虚假安全感）；不锁坏行为（红线在收紧层）。
建议方案: 软层用例收缩为「不 panic + 状态零副作用」两断言的显式分层命名，或直接标注 superseded-by=bad_input_matrix
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-116
严重程度: P3
模块: 历史基线数字盘点（行为锁——非缺陷，登记耦合面）
文件路径/数字: real_corpus_e2e.rs:68-69（309/318、291/302）、:352-354（pages≥50、blocks≥1000）、:905-909（每件≥115）；synth_tender_eval.rs:280-289（≥60/否决≥40/资格≥6/评分≥4/技术≥12/格式≥1、C pages==2、E chunks≥5）；mcp_scaffold_tools.rs:130（≥60 实测108）；global_invariants.rs:430-437（≥30）；scale_1330.rs:137,196（≥200 叶、≥350k 字）；import_tender_files_tool.rs:650（tables==1 gen-log 锚）、:682（≥30）；pdf_export.rs:213（pages≥80）；export_tool.rs:192-195,458,487（w:sz 32/21/24 半点、仿宋_GB2312）；examples/blocking_probe.rs 头注（935 条→blocking 121——注释基线）
问题描述: 夹具耦合行为锁集中且均有出处注释（好纪律）；换夹具/换解析器即全红——真值性依赖夹件永不重制。锁定的是【期望行为】非【问题行为】（未发现明知故锁的坏行为面——唯一近似例外见 MCT-115 软层）。
审计置信度: CONFIRMED
状态分类: 当前仍存在（设计特性）
```

```
ID: MCT-117
严重程度: P3
模块: examples 验收探针无自动化
文件路径: bidcore/crates/bid-mcp/examples/blocking_probe.rs + req_dump.rs
问题描述: #226 层2/#234 的人工对照探针（cargo run --example，输出供人判「真否决 vs 规则说明」/ REQ 逐条 diff）——纯 MANUAL，无断言；其对照数字（935→121）只存注释。属工作流工具而非测试，登记以免 R6 漏数。
审计置信度: CONFIRMED
状态分类: 当前仍存在（有意 MANUAL）
```

---

## 4. 假绿面 Top 3（按风险排序）

1. **SKIP=绿摘要面**（MCT-102/MCT-107/MCT-111/MCT-112）：验收级套件（#241/#242 门禁、GT 红线、官方 SDK 协议、pdf 通道）在夹具/环境缺席时全部退化为 return-绿——cargo test 摘要与真绿不可区分。单文件是诚实设计（横幅+强制开关），**系统性风险在汇总层**：没有任何机制回答「这次跑到底验证了多少」。
2. **merge 前缀判据被夹具名遮蔽**（MCT-101）：覆盖闭环验收测试在「前缀恒真」的输入上全绿——#219/#244 的 Agent 优先语义对真实文件名整体旁路（PARSE2-001 P1 生产缺陷）而 CI 无从发现。这是本批测试最实质的「绿但没测到」。
3. **红线条件跳过 + MANUAL 双相位**（MCT-103 + real_llm_path/subagent_llm_path）：real_corpus GT 红线在非 pdfium 环境 REQ>1500 即跳（连 summary 都不落）；真实 LLM 全链与真实 stdio 传输面的唯一测试是默认 SKIP 的两相位人肉流程——「真实 LLM 验收未完成」在测试结构上**制度化**（诚实，但意味着 CI 永远只验 mock 面）。

（次级：oversized_pdf 残缺默认路径恒 SKIP MCT-108；input_format 软层近恒真 MCT-115；unimplemented 空清单恒绿 MCT-113。）

---

## 5. 逐文件 R5 十问速览（点名 12 文件摘要，其余见 §1 矩阵 + inventory-c3.csv）

- **synth_tender_eval.rs**：REAL_CORPUS 评测；Oracle 双面（ideal 独立/engine_expect 自证，MCT-104）；错误路径=导入 E_PARTIAL 容忍链；断言强（坑位+类目+守恒）；SKIP=缺件横幅；副作用=仓内 artifacts（MCT-105）；基线数字多处；PDF 链软门；不锁坏行为。
- **mcp_scaffold_tools.rs**：E2E+负例；Oracle=issue 验收文+契约；零服务面直调（唯一例外终态 store 断言）；E_STATE×4/E_UNPROCESSABLE×8/E_PARTIAL×2+伪造 receipt 验签拒——错误路径密度全仓最高之一；**无 skip（缺件 panic）**；temp ws 不清理（微瑕）。
- **review_table_matrix.rs**：验证器=测试本地独立实现+**突变自检×5**（test-of-test 典范）；合成件基线不断言三表覆盖（有意留档）；semantics_full 已激活。
- **writing_acceptance_e2e.rs / agent_mandatory_gate.rs**：见 §2.1——验收真实（mock 面），CONFIRMED。
- **export_tool.rs**：断言深（zip 解包 styles.xml 字号半点/暗标解包断言/零半成品）；pdf 双接受（MCT-112）；服务面夹具显式标注。
- **evidence_source_tool.rs / evidence_content_tool.rs**：#185/#217 验收——错误路径可达性好（幽灵 file/越界/指纹不符/旧参数/失败件禁引/不支持 locator）；「测试侧独立复算指纹核对系统生成值」是真独立 Oracle；缺件 panic（MCT-110）。
- **bad_input_matrix.rs / input_format_matrix.rs**：见 §2.5。
- **coverage_tools.rs**：见 §2.3（MCT-101/MCT-109）。
- **addendum_waiver_tool.rs**：负例矩阵密（8 错误码判例含错域 receipt/伪造 MAC/旧 principal）；零副作用断言逐例在场。
- **scale_1330 / pdf_export / mcp_sdk_e2e / real_llm_path / subagent_llm_path / dispatch_contract / save_version_tool / cursor_pagination / global_invariants / export_readback / real_corpus_e2e / e2e_artifacts / e2e_pipeline / 其余工具面测试**：见 §1 矩阵与问题记录；其中 global_invariants（check_* 纯函数+注缺陷必红）与 export_readback（不信自报、独立解析器回读）是全仓 Oracle 独立性最佳实践样板。

## 6. 结论速览

- 覆盖：37/37 测试文件 + 3/3 bin + 2/2 examples（含任务书点名 12 文件逐行读全）。
- **#241/#242 验收真实性**：mock 面 CONFIRMED 真实且强（含 d4c5470 直接批准绕过面的三面覆盖——先证记审面全绿再断言拒力来自独立复算门）；真实 LLM 面诚实未完成（issue #242 自认 + 测试结构默认 SKIP）。
- **bin 生命周期**：MCP-013 四面（panic 隔离/读错退出码/超时/信号）全部 CONFIRMED 且零测试覆盖（MCT-100，P1）。
- **假绿 Top3**：SKIP=绿汇总面 / merge 前缀判据被夹具名遮蔽（PARSEE2-001 测试盲区）/ GT 红线环境条件跳过 + 真实通路 MANUAL 化。
- P1×2（MCT-100 bin 生命周期、MCT-101 merge 真名盲区）、P2×5（MCT-102/103/104/105/114）、P3×10。
- 测试基建设计面（多数文件）纪律显著优于一般工程：单一判定点、突变自检、造件自检、独立回读、诚实降级登记——问题集中在**汇总层（SKIP 不可见）与夹具名选择造成的判据盲区**。

---
*本文件由模块审计代理 C3（Task ID 4-c3）生成；只读审计，未修改任何生产文件/测试/配置。所有行号基于 audit/v2-full-coverage @ 1e0bc9d（生产/测试代码与冻结基线 6105beb 一致）。*
