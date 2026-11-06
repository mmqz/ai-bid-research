# 模块审计 C2：bid-mcp 测试质量审计 A（12 个最大集成测试文件 + 测试→生产映射 R5 前哨）

- 审计代理：4-c2（ai-bid-research 全仓审计 V2 · Task ID 4-c2）
- 冻结基线：main@6105beb（分支 audit/v2-full-coverage；`git diff 6105beb HEAD -- bid-mcp/tests/` = 0 行——测试面与基线一致）
- 范围：`bidcore/crates/bid-mcp/tests/` 按行数排序的最大 12 个文件，**逐文件全读**（合计 10,825 行 / 110 个 `#[test]`）：
  export_readback(1285/6 例)、e2e_artifacts(1225/1)、real_corpus_e2e(1067/1)、search_evidence_tool(955/18)、subagent_llm_path(906/1)、set_matrix_override_tool(851/20)、global_invariants(826/2)、cursor_pagination(793/8)、score_tools(787/11)、import_tender_files_tool(770/21)、validate_tool(738/20)、real_llm_path(622/1)
- 纪律：只读审计；未修改任何生产文件/测试/配置/数据；未运行 cargo test/build；未 git commit/push。本文与 inventory-c2.csv 为本轮唯一新增产出
- 方法：逐文件通读 + 与 C1（module-audit-bid-mcp.md，MCP-001~019）逐条对账 + PA-003/PA-001（validate_tool.rs:217 锁定问题行为）定向复核 + 关键生产锚点抽查（service.rs:1372-1389 无条件推进、env_preflight.rs:298-348 SKIP 机制、param.rs:179-210 幽灵行校验、tools.rs:3469/3643）

---

## 1. 总览（R5 分类 + 关键判定）

| # | 文件 | 测试类型 | 被测生产面（主） | Oracle 来源 | SKIP 机制 | 仓内副作用 | 风险 |
|---|------|---------|-----------------|------------|-----------|-----------|------|
| 1 | validate_tool.rs | INTEGRATION+CONTRACT（+SEMUL：StubReviewer 注入缝） | dispatch_with→validate_bid_document（tools.rs:3469）；服务层夹具 setup | 固定夹具+规则语义（#59 八规则） | 无 | /tmp/bid-validate-* | HIGH（MCT-001） |
| 2 | e2e_artifacts.rs | E2E（synth 夹具全链） | create/import/extract/draft/validate/export/score_document/score_price 经 dispatch_with；④建纲⑤绑定⑧'记审走服务面 | synth 固定夹具+docx XML 独立解包+历史产量下限（>= 锁） | 夹具缺失 SKIP；soffice 软门 | **artifacts/e2e/&lt;run-id&gt;/（14 件）+ latest/ 刷新** | HIGH（MCT-005） |
| 3 | real_corpus_e2e.rs | REAL_CORPUS E2E + GT 对照 | 同上 + check_bid_quality；AddendumWaiver 走 store 直调 | 人工标注 GT（veto-gt/ground-truth.json）+ 历史基线数字 + 复制 veto_measure 匹配算法 | **三重 SKIP**（夹具/乱码/解析器粒度） | **artifacts/real-corpus-e2e/latest/**（3 件×11 文件+summary） | HIGH（MCT-002/010） |
| 4 | search_evidence_tool.rs | CONTRACT（dispatch 全链） | search_evidence / register_evidence（G2 全链含 MATRIX_READY 前置） | #145 验收语义→精确值断言（分数 1.0/0.5、trace、排序） | 无 | 无（纯内存+tmp） | LOW |
| 5 | subagent_llm_path.rs | MANUAL（子代理中继；CI 恒 SKIP） | **MCP 工具面全贯通**（build_outline/register/bind/record_review_gate/save_version）+ 真实 stdio 子进程 | 子代理产物+结构断言（引用/覆盖/±20% 字数） | env 门控（BID_SUBAGENT_PATH_*） | 外部 WS（.mang/manuscripts/band-report） | MED（MCT-003/007） |
| 6 | set_matrix_override_tool.rs | CONTRACT+INTEGRATION+NEGATIVE（receipt 攻击面 20 例） | set_matrix_override 全语义（四重签域/幂等/可逆） | 契约+HMAC 语义+corpus-A 真件 blocking 面 | 夹具缺失 SKIP；**数据条件静默 return（1 条）** | 无（内存） | LOW |
| 7 | global_invariants.rs | INTEGRATION（接缝不变量）+NEGATIVE（反例证明判定面） | 全链（MCP 写操作走工具面）+ 五纯函数判定面 | 跨模块账本对账（设计意图）；④部分同源 | 夹具缺失 SKIP（纯 docx，CI 友好） | 无（tmp，收尾删除） | MED（MCT-004） |
| 8 | cursor_pagination.rs | CONTRACT（12k 分页硬验收）+ fail-closed NEGATIVE | get_compliance_matrix/list_evidence/list_tasks（S-4 游标全链） | 静态推导 oracle+domain 台账直读镜像 | 无 | 无（内存） | LOW（MCT-012） |
| 9 | score_tools.rs | CONTRACT/INTEGRATION | score_price/section/document（含 build_scorer 联结） | issue #100 手算示例+B1 引擎双跑对账 | 无 | 无（内存） | MED（MCT-008） |
| 10 | import_tender_files_tool.rs | CONTRACT+INTEGRATION+stdio E2E | import_tender_files/create/extract + dispatch_json + stdio bin | 借鉴 gen-log.txt 统计锚（tables=1/chunks=9）+契约形状 | 夹具 SKIP；pdftotext env SKIP | /tmp 上传根 | LOW |
| 11 | real_llm_path.rs | MANUAL（人肉 LLM 两相位） | draft generator=real 真实 HttpTransport+stdio 子进程；record_review_gate 经 MCP | 开发者正文（人工）+结构断言+零副作用断言 | env 门控（BID_REAL_PATH_*） | 外部 WS | MED（MCT-003） |
| 12 | export_readback.rs | INTEGRATION+独立回读 | export_bid_document（docx/pdf/xlsx 拒绝面） | **产物字节独立解析**（自含 zip/XML 解析器，不借 docx-rs） | soffice/PDF 文本层/CJK<0.25/夹具 四类 SKIP | /tmp/bid-readback-* | MED（MCT-003 部分） |

**通用面结论**（问题 6/9 总答）：全部 SKIP 走 `bid_parse::env_preflight`（rs:298-348）或 env 门控 early-return——**cargo 计数一律 passed**（env_preflight.rs:315 注释自认「提前 return（计数 passed）」）。CI 无 poppler/pdfium/soffice/real-corpus/BID_*_PATH_* 的环境下，本批 12 文件中真实断言强度显著缩水（详见 MCT-002/003）。

---

## 2. 逐文件审计记录（10 问逐答）

### 2.1 validate_tool.rs（738 行 / 20 例）——PA-001/PA-003 对齐专项

1. **类型**：INTEGRATION+CONTRACT。SUT=validate_bid_document 经 `dispatch_with`（tools.rs:3469 全链：guard→check_version→svc.validate_document）；夹具 setup 全部服务层直调（`svc.store_mut().upsert_file`/`set_requirements`/`prepare_outline_and_matrix`/`draft_section`/`bind_evidence`——**测试特权面**）；`semantic_failed_with_stub_reviewer` 用 SEMUL（StubReviewer 注入缝）+ 服务面直调 `validate_document_with_reviewer`。
2. **被测生产面**：validate_bid_document（tools.rs:3469-3528）→ BidService::validate_document_with_reviewer（service.rs:1372-1389，**已复核：无条件推进段在位**）；`draft_ready_leaves` 间接锁定 #110 缺失回报闸（draft 拒生成语义）。
3. **Oracle**：固定夹具+规则语义（rule_code 精确断言：E_VETO_UNRESPONDED/E_QUAL_UNMET/E_SECTION_EMPTY/E_CHARS_BELOW_MIN/W_CHARS_OFF_TARGET/W_PLACEHOLDER/E_CITATION_GHOST/E_EVIDENCE_EXPIRED）——来自 #59/#187 规则定义，独立性好；无历史数字。
4. **错误路径可达性**：**两处注入为测试特权**——E_SECTION_EMPTY 用 `set_section_content(&sids[0], String::new())` 直改 store（289 行注释自认「绕过 CAS 直接改 store 是测试特权——真实面 draft 失败即空」）；E_CITATION_GHOST 用 store 直写投毒（422-426 行注释「模拟 AI 面直改库的损坏态」）。生产面经 MCP 工具序列构造这两个态的通路存疑（external 手稿 draft 是最近通路但会经回查过滤）。其余错误面（E_STATE/E_UNPROCESSABLE/E_NOTFOUND/scope）真可达。
5. **断言强度**：强——精确 rule_code、counts、REQ id 定位、消息文案含关键词；少量 `>=` 下界（findings>=3）。
6. **SKIP**：无。
7. **副作用**：仅 /tmp/bid-validate-{tag}-{pid}-{nanos}（含 auto_checkpoint 的 db.json）；仓内零写入。
8. **硬编码**：无仓库 SHA/历史基线数字（sha-v-* 为夹具数据）。
9. **环境依赖**：零外部依赖（无 LLM/无外部工具）。
10. **锁定问题行为**：**是——MCT-001（P0）**：217/242/554 三处 `assert_eq!(r["project_status"], "VALIDATING", "DRAFTING → VALIDATING 推进")`。`validate_contract_shape_and_promotes_validating`（199-221）在 **ok=false 且 blocker>=3** 的夹具上断言推进到 VALIDATING + 复跑幂等「推进 no-op」——把 #243 的完整问题形态（部分章节未起草+阻塞在场+状态仍推进+无回退）逐条钉死为预期。与 PA-003 完全对齐；修 #243 时此三断言必反向阻挡。缺失测试：部分章节校验后 status/version 不变；DRAFT_INCOMPLETE 错误码；VALIDATING 态 draft 的 E_STATE 死锁面（全套件零覆盖——起草全部用服务面 `svc.draft_section`，其对 Drafting+ no-op（PA-001 已证「测试特权根」），MCP 面死锁从不被踩到）。

### 2.2 e2e_artifacts.rs（1225 行 / 1 例）——副作用落盘专项

1. **类型**：E2E（synth-tender 真夹具全链，#80 任务 B）。
2. **被测生产面**：MCP §1/§2/§3/§6/§7/§8 + score_document/score_price（dispatch_with 全链）；**④建纲建矩阵、⑤证据登记绑定、③门记审（MatrixApproval/OutlineApproval/PreExportConfirm）、⑧.5 param_responses、⑦ format_spec 为服务面/域面直调**（248 行注释自认「服务面——无 MCP 工具，显式标注」）；waiver 走 MCP record_addendum_waiver + `svc.issue_approval_receipt`（宿主签发缝，非 MCP 工具）。
3. **Oracle**：混合——最强面是 ⑨.5 docx XML 深断言（解 zip 数字节，不信任工具返回值：`<w:tbl>`==6、红 run==5、H1 集=outline 根章+附表+格式件、报价合计字面量、大写金额换写）；弱面是**历史产量下限**（n_reqs>=60、n_sec>=12、total_words>500——数字来自既往运行输出，产出即真值）。
4. **错误路径可达性**：好——D 件 fail-closed E_PARTIAL 真断言；`waive_unready_inputs` 走真实受限起草面。
5. **断言强度**：强（XML 逐格/段落样式/标题集对账）。
6. **SKIP**：`skip_if_fixture_missing`（132 行）；PDF 软门：soffice 缺失 → 08 号产物记 ENV_MISSING 不判红（942-948）——**PDF 导出链断言在无 soffice 环境为零**。
7. **副作用（仓内，精确清单）**：`artifacts/e2e/e2e-<YYYYMMDD-HHMMSS>/`（每次运行新增，不清理）：01_project.json、02_imported.json、03_requirements.json、04_outline.json、05_drafts/（每章一个 .md + index.json）、06_format-spec.json、07_export.docx、08_export.pdf 或 08_export.pdf.json、09_validation.json、10_report.md、11_self-score.json、12_export-check.json；外加 `artifacts/e2e/latest/` **remove_dir_all + 全量重刷**（1131-1142——并发运行竞态面）。/tmp/e2e-artifacts-<nanos> 工作区（含 out/*.docx、bidder-profile.json、.mang）。
8. **硬编码 REQ id**：param_rows 绑 REQ-0082/0083/0084/0085/0092（373-416 行）——**id 序来自当前抽取引擎输出**；param.rs:204-210 幽灵行校验只拦「不存在的 id」，不拦「错位但存在的 id」：抽取重排后负偏离可静默漂移到错误条款行（red_runs==5 仍绿）——产出即真值耦合（MCT-005）。
9. **环境依赖**：soffice（PDF 面）、PDF 链工具（探测记录不判红）、synth 夹具。
10. **锁定问题行为**：无直接锁定；但 validate ok=true 路径隐含「全素材就绪才绿」——与 PA-005 呼应（见 MCT-007：单条合成资质证据绑全部必选 REQ → blocking==0 是绿灯前提）。

### 2.3 real_corpus_e2e.rs（1067 行 / 1 例）——GT 红线与自证风险专项

1. **类型**：REAL_CORPUS E2E（3 件真实水科 PDF，每件独立项目全链）+ GT 对照（#131）。
2. **被测生产面**：同 2.2；AddendumWaiver 门用 `svc.store_mut().record_review` 直调（508-515——比 e2e_artifacts 更深的服务面旁路）；⑥.0 先断言 MissingAddendum fail-closed 面（真实 MCP 可达）。
3. **Oracle**：人工标注 GT（artifacts/veto-gt/ground-truth.json，assert 在场性）+ **测试内嵌 veto_measure 三通道匹配算法**（135-238 行，注释自认「匹配口径与 scripts/veto_measure 完全一致——测试内嵌同一算法」——测量面同构复制，非独立实现）+ **历史基线数字**：L1_BASELINE=309/318、P_BASELINE=291/302（66-69 行注释「veto-measure 于 main 51874cd 实测，逐位一致」——历史 commit SHA + 既往运行产出当真值的 >= 锁，PARSE2-005 同型）。
4. **错误路径可达性**：好（MissingAddendum、技术参数表缺席告警双向断言）。
5. **断言强度**：红线为 `>= baseline - 1e-9`（浮点容差）；表格数精确==expected（5+tech>0）；forms 7/7；星号==0；**但 08_validation.json 只落盘不断言**（ok/blocker 零检查——导出门间接兜底）。
6. **SKIP（三重，MCT-002 核心）**：① real-corpus 缺件 SKIP（274）；② #169 乱码 fail-loud SKIP（289-299）；③ **解析器粒度 SKIP：total_reqs > 1500 → 873-884 打横幅 return**——注释自证「CI 实测 REQ 2624 vs 本地 ~900（poppler 粒度）」。第三重在 poppler/纯 Rust 兜底环境**静默跳过全部红线断言（886-900）、抽取规模、大纲结构、起草产出、导出表格断言（903-1010）与 summary 落盘**——L1/P 红线只在 pdfium 同粒度环境真执行。
7. **副作用（仓内）**：`artifacts/real-corpus-e2e/latest/`（remove_dir_all+重刷）：3 个 `latest/<件名>/`（01~11 号产物 + report.md）+ latest/summary.json + latest/report.md。/tmp 工作区循环内删除（851）。
8. **硬编码**：51874cd（历史 SHA，注释）+ 309/318、291/302（历史基线）+ 页数域 76-284/块 2272-8107（历史观测域，仅用于 >= 下限）。
9. **环境依赖**：real-corpus PDF（克隆不含）、GT（仓内）、解析器粒度（pdfium vs poppler）。
10. **锁定问题行为**：无；但三件水利件 tech_req==0 的「确定性型号正则场景局限」被如实登记为设计边界（非锁定缺陷）。

### 2.4 search_evidence_tool.rs（955 行 / 18 例）

1. **类型**：CONTRACT（#145 验收面，全部 dispatch_with 全链）。
2. **被测生产面**：search_evidence（tools.rs 双通道/降权/排序/截断/信封）+ tools_schema（len==25/implemented==25——与 dispatch_contract 同口径锁定工具面）+ register_evidence（含 fields 校验的 **MCP 全链前置**：create→import→extract→build_outline 才到 MATRIX_READY，744-781 行注释如实登记「不靠直调 store 绕状态机」——正确做法的反例补位）。
3. **Oracle**：#145 验收语义精确值（score 1.0/0.5、match_type、trace 计数、有界短语不误报前缀、SUPERSEDED 排除、降权×0.5、EVIDENCE_EXPIRING 窗口 90 天）——独立性好。
4. **错误路径可达性**：真可达（error_faces 14 例 E_UNPROCESSABLE + E_NOTFOUND；拒绝零入库断言 811 行）。
5. **断言强度**：强（精确计数/分值/排序不变式 windows(2) 全序校验）。
6. **SKIP**：无。
7. **副作用**：纯内存（BidService::new）；matrix_ready_store 用 /tmp/search-fields-{pid}（固定名，无 nanos——pid 复用+陈旧 /tmp 残留时二次运行可能撞 E_STATE 假红，低概率）。
8. **硬编码**：无。
9. **环境依赖**：synth 夹具（仅 fields 校验例）。
10. **锁定问题行为**：无。person_name 不在 exact 枚举被显式拒并断言（938-955）——如实登记字段面未扩，非锁定。

### 2.5 subagent_llm_path.rs（906 行 / 1 例）

1. **类型**：MANUAL（#80 第三批 A'——子代理作者多章实测；两相位 capture/serve）。
2. **被测生产面**：**本批 12 文件中唯一 MCP 工具面全贯通**：build_outline（G1）、register_evidence/bind_evidence（G2，含 get_compliance_matrix 游标分页发现待绑 REQ——外部 Agent 同款视角 271-290）、record_review_gate ×3（G3，宿主签发 receipt 经 MCP 提交）、draft/validate/export/save_version/check_bid_quality 走**真实 stdio 子进程**。
3. **Oracle**：子代理产物结构断言（citations>0、uncovered==∅、±20% 字数带或容差汇总）+ usage 台账 calls>=N + 重启恢复。
4. **错误路径可达性**：capture 相位断言 503 错误语义 + **零副作用**（失败不推进章节状态）——真实传输面。
5. **断言强度**：强。
6. **SKIP**：`BID_SUBAGENT_PATH_WS/PHASE` 未设 → 横幅 return（540-554）——**CI 恒 SKIP**（MCT-003）。
7. **副作用**：外部 WS（.mang/subagent-targets.json、llm.json、llm-usage.json、approval.key 读取、manuscripts/&lt;SEC&gt;.md、.mang/history 快照、band-report.json、out/*.docx）；仓内零写入。
8. **硬编码**：无（规模旋钮全 env 化）。
9. **环境依赖**：驱动脚本 scripts/e2e_bid/run_subagent_path.sh + human_llm_relay.py（均在仓）+ stdio bin。
10. **锁定问题行为**：间接——`bind["matrix"]["blocking"]==0`（449-453）在**单条合成资质证据绑定全部 qualification+veto+tech_param REQ** 后成立：即 #245/PA-005 的「占位证据→阻塞归零」场景本身是绿灯路径（MCT-007）。另：serve 相位直读 .mang/approval.key 签发 receipt（831-844）——测试进程即宿主，属设计信任边界（与 C1 残余信任面结论一致）。

### 2.6 set_matrix_override_tool.rs（851 行 / 20 例）

1. **类型**：CONTRACT+INTEGRATION+NEGATIVE（#226 层 1；receipt 验签攻击面全家福）。
2. **被测生产面**：set_matrix_override 全语义（tools.rs:4102-4215）：四重签域（项目/行/状态/理由摘要——receipt scope 经 `bid_mcp::matrix_override_scope` re-export 单一真源，测试面不复制格式串，61-77 行注释明确防漂移）；export_blockers 为 blocking 权威对照面（146 行注释——主动规避「矩阵响应无 counts.blocking 的真空断言」）。
3. **Oracle**：契约语义+HMAC 攻击分类码（BAD_SIGNATURE/NOT_HUMAN_ACTOR/MALFORMED_RECEIPT）+corpus-A 真件 blocking 前提。
4. **错误路径可达性**：全真可达（缺 deviation/空白 deviation/REQ-999999/非法 status/DRAFT 态 E_STATE/错 project_id/无 receipt/伪造 receipt/跨行/跨项目/Agent 身份/裸身份参数——12 类拒绝，每类附零副作用复检）。
5. **断言强度**：强（blocking 恰减 1、幂等重放、可逆回 blocking、审计面 actor/at 回显）。
6. **SKIP**：corpus_a_ready 门控 18/20 例；**cross_requirement_receipt_is_rejected 在语料仅一条 blocking 行时静默 return（529-531，无横幅）**——数据条件 SKIP（当前 corpus-A 有多条 blocking，实际执行）。
7. **副作用**：无（BidService::new 内存态）。
8. **硬编码**：无。
9. **环境依赖**：corpus-A A-招标文件.docx。
10. **锁定问题行为**：无——唯一覆盖 MCP-001（工具未入契约）的语义真源测试；死锁出口闭环（direct_draft_path 763-851：blocking 未解直起草 E_UNPROCESSABLE → 覆写 → 同章起草成功）是 #226 验收线的可执行化，质量高。

### 2.7 global_invariants.rs（826 行 / 2 例）——守卫缺口专项（详 §3.3）

1. **类型**：INTEGRATION（#171 接缝不变量）+NEGATIVE（反例注入缺陷证明判定面非空转——680-826，18 个反例断言）。
2. **被测生产面**：主链 MCP 写操作全走工具面（70 行注释「写操作全部走工具面，不旁路」——call 助手 dispatch_with；建纲/绑定/记审仍服务面）；五判定面为纯数据函数（154-383）。
3. **Oracle**：跨模块账本对账不变量（设计意图独立推导）；**④部分同源**：期望集 parents 经 `bid_domain::scoring::derive_scoring_items`（生产函数）推导（311 行）——派生语义若变，不变量随之漂（部分重言式）；②③与 is_mandatory/ResponseStatus 同源（注释自认「不另立一套」——有意决策）。
4. **错误路径可达性**：主链全绿面；反例面纯数据注入（正当——判定面防空转纪律）。
5. **断言强度**：**本套件最强**——①多重集逐字全等+恰一次；②③逐条对账+空转护栏（零绑定/零必选行即红）；⑤双向逐字（标题截断/层级错位/来历不明标题全红）；反例逐条证明可红。
6. **SKIP**：fixtures_ready（纯 docx 两件——刻意不依赖 PDF 链，CI 友好）。
7. **副作用**：/tmp/bid-gi-*（收尾 remove_dir_all）；仓内零写入。
8. **硬编码**：无。
9. **环境依赖**：synth 夹具。
10. **锁定问题行为**：无——但五不变量全部是「账本↔产物」维度，状态机/契约字面量维度零覆盖（MCT-004）。

### 2.8 cursor_pagination.rs（793 行 / 8 例）

1. **类型**：CONTRACT（S-4 分页硬验收，issue #11 v3/V3-5）。
2. **被测生产面**：get_compliance_matrix/list_evidence/list_tasks（cursor.rs FNV 指纹+base64 编解码+slice 越界门全链）。
3. **Oracle**：静态推导（12k 循环生成夹具的 cmp_id 序列精确相等）+ evidence/task 用「直读 domain 台账镜像过滤」当独立 oracle（509-519/648-656——同数据源不同代码路径，部分独立）。
4. **错误路径可达性**：真可达——坏游标 fail-closed 10 例（坏 base64/坏 JSON/截断/空串/坏版本/负 offset/溢出/fp 缺失/fp 错配）+ 缩水面（同构小 store 复用大 store 游标 → offset 越界拒，399-404）。
5. **断言强度**：强（12k 页取完无重无漏+页数恰 120+行序精确+确定性逐字节相等跨重建）。
6. **SKIP**：无。
7. **副作用**：无（内存）。
8. **硬编码**：无。
9. **环境依赖**：无。
10. **锁定问题行为**：无——但**无「列表增长滑窗」测试**（MCP-011 回声，MCT-012：翻页中途插行 → offset 仍合法 → 静默跳/重行，契约「列表已变化→E_UNPROCESSABLE」只兑现缩水半边，与 C1 判定一致）。

### 2.9 score_tools.rs（787 行 / 11 例）

1. **类型**：CONTRACT/INTEGRATION（#100 任务 3 接线验收）。
2. **被测生产面**：score_price（纯计算器无门）/score_section/score_document（guard+build_scorer 联结：leaf_of 反查、#170 章引用挂载、#224 多对多摊分、inert 账本去重归因）。
3. **Oracle**：issue #100 §3.3 手算示例（1650×0.95=1567.5 万、满分 40）+ **B1 引擎双跑对账**（bid_domain::pricing::PriceScoreSpec 当 oracle 比对工具壳输出，185-206——壳层等价性验证的正确形态；引擎自身正确性另册）。
4. **错误路径可达性**：真可达（gpt4 method/benchmark 缺 budget/price 缺 deduct_high 响亮拒；E_STATE 两态门）。
5. **断言强度**：强（share 和==1.0、contributions 之和==estimated、≤max 不虚高、inert 归因 source=RULE_PARSED/share=1.0/锚回 REQ 首锚）。
6. **SKIP**：无。
7. **副作用**：无（内存）。
8. **硬编码**：无。
9. **环境依赖**：无。
10. **锁定问题行为**：无——**PB-006 确认**（MCT-008）：452-455 注释自认「账本多对多映射（域层直写——MCP 写面后续 issue）」——upsert_scoring_mapping 仅测试特权可达，映射账本态对 MCP Agent 不可达。

### 2.10 import_tender_files_tool.rs（770 行 / 21 例）

1. **类型**：CONTRACT+INTEGRATION+stdio E2E（B-1）。
2. **被测生产面**：import_tender_files/create_bid_project/get_bid_project_state/extract_requirements/dispatch_json + stdio bin 子进程（BID_FS_ROOT 注入，595-652）。
3. **Oracle**：借鉴轨 gen-log.txt 统计锚（word-complex tables=1、chunks=9=7 非空段+2 表行——**第三方（mdcore 借鉴件）独立统计对照**，独立性好）+ 契约形状（TF-0002/project_version=2/totals）。
4. **错误路径可达性**：**全真可达且断言零副作用**——参数校验 9 例（含 project_version 非整数）、E_NOTFOUND 路由、E_STATE 越级（FILES_IMPORTED 态建异 key 项目——经同一 guard_tool 通道构造可达非法转换，334 行注释如实说明 import 自身越级面 M2 前不可达）、E_PARTIAL（截断 docx/混合批次/不可读 ref）、E_CONFLICT。
5. **断言强度**：强（每类拒绝附 store 状态复检）。
6. **SKIP**：夹具 SKIP（word-complex/corpus-R）+ pdftotext env SKIP（pdf 路由例）。
7. **副作用**：/tmp/b1_mcp_uploads_*；无仓内写入。
8. **硬编码**：无（TF-0002 等 id 序列为确定性推导并注释口径）。
9. **环境依赖**：fixtures/engine-borrowed、corpus-R、corpus-A B1 pdf、pdftotext。
10. **锁定问题行为**：get_state 的 `project_status` 注入参数被当 dry-run 纯函数面断言（576-577 EXPORTED probe terminal==true）——MCP-007 的未记档调试面在测试中被钉为预期（低危，随契约修订）。

### 2.11 real_llm_path.rs（622 行 / 1 例）

1. **类型**：MANUAL（#80 第二批 A——人肉 LLM 中继两相位）。
2. **被测生产面**：RealGenerator 真实代码路径（LlmConfig::resolve→HttpTransport/ureq→Bearer→JSON 容错→失控检查→用量台账）+ **真实 stdio 子进程**（BID_WS 持久化——Claude Desktop 同款接线）+ record_review_gate 经 MCP（receipt 宿主签发）。
3. **Oracle**：开发者人工正文+结构断言（citations>0/uncovered==∅/±20% 字数）+ 503 错误语义+零副作用+重启恢复。
4. **错误路径可达性**：真实（capture=503 错误面；serve=成功面）。
5. **断言强度**：强。
6. **SKIP**：`BID_REAL_PATH_WS/PHASE` 未设 → 横幅 return（400-414）——**CI 恒 SKIP**（MCT-003）。
7. **副作用**：外部 WS（.mang/real-path-target.json、llm.json、llm-usage.json、out/*.docx）；仓内零写入。
8. **硬编码**：无。
9. **环境依赖**：run_real_llm_path.sh、human_llm_relay.py、stdio bin、llm.json。无 BID_LLM_API_KEY 依赖（本地中继配置）。
10. **锁定问题行为**：无。

### 2.12 export_readback.rs（1285 行 / 6 例）

1. **类型**：INTEGRATION+**独立回读**（#172——不看任何工具返回值，产物字节用自含解析器重读）。
2. **被测生产面**：export_bid_document（docx 结构/暗标脱敏/xlsx 诚实拒/PDF 文本层）+ fixture 链（服务面 setup + export 经 dispatch_with）。
3. **Oracle**：**本套件独立性最强**——自含 docx/xlsx 解析器（zip+字符串解析，不借 docx-rs/calamine；xlsx 实体解码含数值引用防假红，999-1039）+ outline 对照（期望面来自 store 非导出报告）；PDF 回读经 bid_parse::parse_file（同仓解析器，半独立）。
4. **错误路径可达性**：xlsx 拒绝面真可达（E_UNPROCESSABLE+指路+拒后无半个 xlsx 落盘断言 1272-1283）；暗标公司名零残留+替换词在位（真生产脱敏路径；公司名稿经服务面 draft 注入——近似通路为 external 手稿）。
5. **断言强度**：强（OOXML 必需部件/sectPr/TOC 域开关 \h\u/待填位显式在位/占位符与未生成标记与 fallback 字形三套扫描）。
6. **SKIP**：soffice 缺失→PDF 例 SKIP（844-850）；PDF 文本层读不出→SKIP；CJK<0.25→SKIP（#169 纪律：解码环境不可用不判红）；xlsx 对照夹具缺失→SKIP。
7. **副作用**：/tmp/bid-readback-*；仓内零写入。
8. **硬编码**：无（RESIDUAL_PLACEHOLDERS/UNGENERATED_MARKS 为设计清单）。
9. **环境依赖**：soffice、pdfium/poppler（PDF 面）、fixtures/corpus-A xlsx。
10. **锁定问题行为**：无——xlsx 「当前拒绝+回读器有效性分离」是诚实处理「未上产线通道」的正面范式。

---

## 3. 专项任务结论

### 3.1 validate_tool.rs × PA-001/PA-003（对齐复核）

- **断言固化面 CONFIRMED 且为三处**：217（shape 例）、242（after-draft 例）、554（green 例）。`validate_contract_shape_and_promotes_validating` 的夹具正是 #243 症状形态：grouped 夹具下含未响应否决项的叶保持 Pending（draft 拒生成）+ 至少一叶未起草 + findings>=3 blocker + **ok=false 时仍断言推进 VALIDATING**，并断言复跑「推进 no-op」（219-220）——把「数据面报错但状态机已持久化推进」的双面语义逐条锁死。
- 与 PA-003 记录完全一致（含「修 #243 时测试反向阻挡」预判）；与 MCP-004（P0）互为源码/测试两面。
- **本套件对该死锁族的覆盖为零**：所有起草用 `svc.draft_section`（服务面，对 Drafting+ no-op——PA-001 已定性为「测试特权根」）；无任何测试在 VALIDATING 态经 MCP 调 draft_response_section 断言 E_STATE（死锁关闭点 tools.rs:1895 无人踩）。唯一触及「死锁出口」的是 set_matrix_override_tool 的 direct_draft_path（#226 覆写出口——那是另一条出口，非 #243 修复回归）。

### 3.2 e2e_artifacts / real_corpus_e2e 副作用与自证风险

**Lead 隔离运行副作用预测（仓内精确路径清单）**：
1. `artifacts/e2e/e2e-<UTC+8 时间戳>/`——新增 14 文件/目录（01~12 号 + 05_drafts/&lt;SEC&gt;.md×N + 05_drafts/index.json + 08_export.pdf 或 08_export.pdf.json）；**每跑一次新增一个 run-id 目录，无清理**（当前仓内仅 latest/——历史 run-id 未入库或已清）。
2. `artifacts/e2e/latest/`——remove_dir_all + 全量重刷（**与其他 e2e 并发跑存在竞态**：cargo test 默认并行线程下，本文件仅 1 例，但与 real_corpus_e2e 分属不同目录互不冲突；同文件重入才会撞）。
3. `artifacts/real-corpus-e2e/latest/`——remove_dir_all + 重刷：`A-塔里木…/`、`A-某水利…/`、`A-伊犁…/` 各 11 件（01_project/02_imported/03_requirements/04_outline/05_drafts(3 样例 md+index)/06_export.docx/07_quality/08_validation/09_export-meta/10_gt-check/11_export-check/report.md）+ `latest/summary.json` + `latest/report.md`。real_corpus 缺件时整个测试 SKIP → **latest/ 不会被刷新**（隔离副本若缺 real-corpus，该目录保持原样）。
4. /tmp 工作区：bid-validate-*、e2e-artifacts-&lt;nanos&gt;、real-corpus-e2e-*（循环尾删）、bid-gi-*（尾删）、b1_mcp_uploads_*、search-fields-&lt;pid&gt;（**不删**）、bid-readback-*（不删）——/tmp 垃圾面。
5. 外部 WS（仅 MANUAL 测试，CI 不会触发）：.mang/{real-path-target,subagent-targets,llm,llm-usage}.json、manuscripts/、band-report.json。

**artifacts 基线自证风险（产出即真值）**：
- real_corpus L1/P 红线 = **main@51874cd 一次实测的历史数字**（309/318、291/302）做 `>=` 下限——引擎输出当真值锁：抽取出垃圾也会因 GT 命中率恰不降而绿（红线不测「多抽的碎屑是否进入正文/矩阵」，仅测 GT 召回与否决类 TP 率）；基线数字从未向 GT 独立重推导。同 PARSE2-005（real_corpus_223 923 floor）模式。
- e2e_artifacts 的 12_export-check.json 半自证：`expected` 侧多数来自当前 outline/reqs 快照（如 dev_rows>=tech_req_count、H1 数=根章数+1+格式件位）——结构一致性对账（artifact vs 内存账本）而非语义 GT；工具 JSON 与产物 XML 双真源交叉只在 docx 面成立（这部分是真独立）。
- e2e_artifacts 硬编码 REQ-0082..0092 绑参数行：幽灵校验（param.rs:204-210）拦「不存在」不拦「错位」——抽取重排时负偏离语义可静默漂移（红 run==5 不变），测试仍绿。
- real_corpus 的 08_validation.json 落盘不断言（无 ok/blocker 检查）——e2e 的「校验绿」仅由导出门间接背书。

### 3.3 global_invariants 守卫缺口（专项核心结论）

五条不变量（①叶↔Heading2 多重集逐字；②必选 REQ 有响应节且有正文落点；③绑定↔RESPONDED 逐条且 evidence_ids 携带对账；④by_item↔全部评分项（-父项-硬门）；⑤标题逐字双向）质量高（反例注入证明可红 + 空转护栏），**但已知 PA/PB/MCP 问题中本应被跨模块不变量抓住而未设防的清单**：

| 已知问题 | 本应的不变量 | 现状 |
|---|---|---|
| PA-001/MCP-004（validate 无条件推进） | 「validate ok=false（存在 BLOCKER/未起草叶）⇒ project_status 与 content_version 不变」 | 无——主链只走全绿面（val.ok==true）；未起草面由 validate_tool 锁死**反向语义** |
| PA-004/MCP-005（build_outline 零覆盖门禁） | 「存在未判定块 ⇒ build_outline 被拒或告警」/「建纲时 coverage.complete==true」 | 无——主链不经 submit_block_marks，覆盖面零消费（与 schema.rs:579 虚假广告互证） |
| MCP-003（register/bind 起草后封窗） | 「DRAFTING+ 态 register/bind ⇒ E_STATE 且契约口径一致」 | 无 |
| PB-005（matrix_view vs 持久化 build_matrix 双口径） | ③ 的对账若改用 **MCP get_compliance_matrix 输出**与 store().matrix() 双向对账即可抓住 | ③ 只对 store().matrix()（持久化全量面）——过滤视图（过期绑定滤除）从未与台账对账 |
| MCP-017（out.precheck.waived 恒空） | 「导出回执 waived[] == 已登记 WAIVED 的 VF findings」 | 无 |
| MCP-011（游标增长滑窗） | 「翻页期间列表增长 ⇒ 拒或告警」 | 无（cursor_pagination 亦无） |
| MCP-002（E_NOT_FOUND 线格式分叉） | 「线格式 code ∈ 契约 §0 八码」 | 无（protocol.rs 内联测试反向锁死 "E_NOT_FOUND"） |
| PA-005（tier 不进裁决面） | 「FILE_EXISTS 级证据绑定 ⇒ 矩阵仍阻塞/待补」 | 无——主链恰用单条合成资质绑全部必选 REQ 断言 blocking==0（反方向固化） |

另：④期望集的 parents 由生产函数 `derive_scoring_items` 推导（部分重言式：派生语义变更时不变量跟随漂移，只有「评分项类目全集」一半是独立口径）。

**结论一句话**：守卫缺口集中在**状态机语义/契约字面量/裁决门**三个维度——现有不变量全部是「数据账本↔产物」维度；P0 级 PA-001 恰好落在无守卫区，且被 validate_tool 反向锁定，双因素叠加。

### 3.4 real_llm_path / subagent_llm_path 真实性与缺 key 行为

- **真实性**：两者是唯一驱动「真实 stdio 子进程 + 真实 HttpTransport（ureq→本地中继→503/正文）+ MCP 工具面全贯通（subagent）」的测试；capture 相位的「503 错误语义 + 零副作用」断言是全套件唯一的 LLM 失败面覆盖。subagent 的 G1/G2/G3 全 MCP 化（build_outline/register/bind/record_review_gate + 矩阵分页发现）是**生产可达性最忠实**的编排范本。
- **缺 key/缺环境行为**：两者不依赖 BID_LLM_API_KEY——依赖本地中继（llm.json 由驱动脚本写入）+ env 门控（BID_*_PATH_WS/PHASE）。**未设 env → 横幅 + return = cargo 记 passed**：CI 上真实 LLM 通路断言面恒为零（MCT-003）；全局 `call` 助手在其余 8 个使用 dispatch_with 的测试文件里统一钉 `generator:"mock"`（防 dev 机 BID_LLM_API_KEY 使 auto 走真调用——注释自认），即 **auto→real 的回退路径在任何 CI 测试中都不执行**。
- Receipt 签发：real_llm 用 `svc.issue_approval_receipt`（宿主缝），subagent serve 相位直读 `.mang/approval.key` 原钥签发（831-844）——设计信任边界（宿主持钥），与 C1 残余信任面结论一致，非新缺口。

---

## 4. 问题记录

```
ID: MCT-001
严重程度: P0
模块: bid-mcp（测试有效性——PA-003/MCP-004 测试面）
文件路径: bidcore/crates/bid-mcp/tests/validate_tool.rs
代码行号: 217 / 242 / 554（推进断言）；199-221（shape 例完整固化链）
函数或测试名: validate_contract_shape_and_promotes_validating / validate_after_draft_runs_and_reports / green_path_ok_true_when_no_blockers
问题描述: 三处断言把「validate 无条件推进 DRAFTING→VALIDATING」锁死为预期：shape 例在 ok=false、findings>=3、含未起草叶的 #243 症状形态上断言 project_status=="VALIDATING" 并断言复跑「推进 no-op」。修复 #243（部分章节不推进/完成度门禁）时这三个断言反向阻挡；且全套件无「VALIDATING 态经 MCP draft → E_STATE 死锁面」测试（起草全用服务面 draft_section，其对 Drafting+ no-op=测试特权根，PA-001 已证）。
调用链: 测试 → dispatch_with(validate_bid_document) → guard_tool(Drafting 允许) → service.rs:1379-1387 无条件 advance → 断言固化
触发条件: 任何回归测试运行（绿=问题行为在场）
实际影响: P0 缺陷（#243 死锁）被测试体系钉死；修复 PR 必须同步改测试口径，否则 CI 红→开发者可能回退修复。
复现步骤: rg 'VALIDATING", "DRAFTING' validate_tool.rs → 三处命中；对照 service.rs:1380-1383 steps 表（Drafting=>[Validating] 无前置）。
当前测试: 即本条（问题=测试本身）。
缺失测试: ①部分章节未起草时 validate 后断言 status/content_version 不变；②VALIDATING 态 MCP draft 的 E_STATE 断言；③DRAFT_INCOMPLETE 错误码面。
建议方案: 断言改挂新契约（ok=false ⇒ 不推进+可修订重试）；补②③；参照 set_matrix_override_tool direct_draft_path 的三段式写法。
证据: validate_tool.rs:211-220:
    assert_eq!(r["ok"], json!(false), "有 BLOCKER 时 ok=false（未响应×3 + 未起草叶）");
    assert!(r["findings"].as_array().unwrap().len() >= 3, "未响应 3 条");
    ...
    assert_eq!(r["project_status"], "VALIDATING", "DRAFTING → VALIDATING 推进");
    // 复跑幂等（VALIDATING 态再校验——guard 允许 + 推进 no-op）
    let r2 = validate(&mut svc, &pid);
    assert_eq!(r2["counts"], r["counts"], "同输入复校结果全等（纯读诊断）");
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-002
严重程度: P1
模块: bid-mcp（测试有效性——红线断言环境依赖）
文件路径: bidcore/crates/bid-mcp/tests/real_corpus_e2e.rs
代码行号: 871-884（REQ_PLAUSIBLE_MAX=1500 SKIP）；佐证 860-870 注释（CI 实测 REQ 2624 vs 本地 ~900）、66-69（基线数字）
函数或测试名: real_corpus_full_chain
问题描述: GT 红线断言（L1>=309/318、P>=291/302）与全部汇总断言（大纲结构/起草产出/表格 7/7/星号 0）位于「解析器粒度守卫」之后：total_reqs>1500（poppler/纯 Rust 兜底粒度）→ 打横幅 return → cargo 记 passed。CI（有 poppler 无 pdfium 粒度）上 L1/P 红线从不执行——真实语料验收面退化为「导入成功+页数下限+MissingAddendum 面」。
调用链: parse_file（poppler 通道 → REQ 2624）→ REQ_PLAUSIBLE_MAX 判定 → return（跳过 886-1010 全部断言与 summary 落盘）
触发条件: 无 pdfium 粒度解析器的环境跑本测试
实际影响: 「红线进 CI」的设计目标（模块头注释）在主流 CI 环境落空；SKIP 与 pass 在测试报告中不可区分（假绿面 Top1）。
复现步骤: 在仅 poppler 环境跑 cargo test -p bid-mcp --test real_corpus_e2e -- --nocapture：stderr 出现「SKIP: …解析器与 GT 标注不同源」横幅，测试 exit 0。
当前测试: 即本条。
缺失测试: 粒度差异环境下的降级断言（如按 poppler 粒度独立基线）或显式 #[ignore] 化（SKIP 不计 passed）。
建议方案: ①把粒度守卫上移为测试分拣（pdfium 环境跑红线版 / 其他环境跑无 GT 版且命名不同）；②SKIP 改用 --ignored 门控或在横幅中要求 CI 以 FAIL-fast 方式显式登记。
证据: real_corpus_e2e.rs:871-884:
    let total_reqs: u64 = per_file.iter().map(|f| f["reqs"].as_u64().unwrap_or(0)).sum();
    const REQ_PLAUSIBLE_MAX: u64 = 1500;
    if total_reqs > REQ_PLAUSIBLE_MAX {
        eprintln!("SKIP: real_corpus_full_chain —— 解析器与 GT 标注不同源\n…");
        return;
    }
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-003
严重程度: P1
模块: bid-mcp（测试有效性——CI 断言面空洞）
文件路径: bidcore/crates/bid-mcp/tests/real_llm_path.rs + tests/subagent_llm_path.rs + tests/e2e_artifacts.rs + tests/export_readback.rs
代码行号: real_llm_path.rs:400-414 / subagent_llm_path.rs:540-554（env 门控 SKIP）；e2e_artifacts.rs:917-948（soffice 软门）；export_readback.rs:844-850+871-880+896-905（PDF 三重 SKIP）；佐证 env_preflight.rs:315（「提前 return（计数 passed）」）
函数或测试名: real_llm_human_relay_path / subagent_llm_multi_chapter_path / e2e_artifacts_synth_tender / readback_pdf_text_layer_and_chinese_readable
问题描述: 四类 CI 不可达断言面叠加：①真实 LLM 传输面（HttpTransport/503 语义/零副作用/真实生成链）仅在 env 门控下执行——CI 恒 SKIP；②PDF 导出链（soffice）在 e2e 与回读两处双双软 SKIP——无 LibreOffice 环境 PDF 产物断言为零；③回读 PDF 再叠「文本层读不出」「CJK<0.25」两重 SKIP；④其余 8 文件统一钉 generator=mock，auto→real 回退分支零覆盖。SKIP 全部计 passed。
调用链: cargo test → env 缺件 → eprintln 横幅 + return → passed
触发条件: 标准 CI 环境（无 BID_*_PATH_* / soffice / pdfium）
实际影响: 「真实通路已验收」的绿灯证据只存在于人工运行记录；CI 绿不能外推到 LLM/PDF 面。与 #9 纪律（缺件不伪装失败）形成张力：不伪装 FAIL 的代价是伪装 PASS（计数面）。
复现步骤: CI 跑 12 文件全绿 → 本机设 BID_REAL_PATH_* 跑 phase1 观察差异。
当前测试: 即本条。
缺失测试: SKIP 面的显式化（#[ignore]+显式 job）或最低限度 smoke（如 HttpTransport 对不存在端点的错误路径单测，不依赖中继）。
建议方案: ①env 门控 SKIP 改 #[ignore]（cargo 记 ignored 而非 passed）；②PDF 链加「soffice 缺失时至少断言 export_bid_document pdf 面 E_UNPROCESSABLE/ENV_MISSING 错误码」的 CI 分支；③HttpTransport 错误面抽独立 CONTRACT 测试。
证据: real_llm_path.rs:400-413:
    let (Some(ws_s), Some(phase)) = (…) else {
        println!("================ SKIP real_llm_path ================\n…");
        return;
    };
env_preflight.rs:315: 提前 return（计数 passed），**绝不** panic 红脸
审计置信度: CONFIRMED
状态分类: 当前仍存在（设计取舍，需 Lead 裁定口径）
```

```
ID: MCT-004
严重程度: P1
模块: bid-mcp（测试有效性——全局不变量守卫缺口）
文件路径: bidcore/crates/bid-mcp/tests/global_invariants.rs
代码行号: 149-383（五判定面）；311（④ oracle 同源点 derive_scoring_items）；对照 module-audit-bid-mcp.md MCP-002/003/004/005/011/017、PA-001/PA-004/PA-005、PB-005
函数或测试名: gi_five_global_invariants / gi_invariant_checks_reject_injected_defects
问题描述: 五不变量全部为「账本↔产物」维度且实现质量高，但对已知 P0/P1 问题零守卫：PA-001（validate 推进）、PA-004（建纲覆盖门）、MCP-003（register/bind 封窗）、MCP-017（precheck.waived 恒空）、MCP-011（游标增长滑窗）、MCP-002（E_NOT_FOUND 线格式）、PB-005（matrix_view 双口径——③若对 MCP get_compliance_matrix 输出与 store().matrix() 双向对账即可抓住）、PA-005（tier 不进裁决——主链反方向固化）。④期望集 parents 用生产函数 derive_scoring_items 推导，部分重言式。
调用链: 不适用（缺失守卫面）
触发条件: 上述任一问题回归时 global_invariants 仍全绿
实际影响: 「模块间对不上」类问题中只有数据对账维度有网；状态机/契约字面量/裁决门维度的回归只能靠单册工具测试（而其中 validate 面已被 MCT-001 反向锁定）。
复现步骤: 思想实验：删掉 service.rs:1380-1383 推进段或把 ENotFound serde 改名 → 全仓现有测试（含 global_invariants）无一红（前者只有 validate_tool 三断言红——它们断言的是问题行为本身，删除推进段反而会红在「应推进」上，属同一枚硬币）。
当前测试: 即本条。
缺失测试: 八条新不变量（§3.3 表）。
建议方案: 按表逐条补：validate 前后 status/version 快照不变量（挂新契约）、build_outline 覆盖前置不变量、register/bind 窗口不变量、导出回执 waived==登记 WAIVED、游标遍历完整性、线格式 code∈契约八码、MCP 矩阵视图↔持久矩阵双向、FILE_EXISTS 绑定仍阻塞。
证据: global_invariants.rs:310-317:
    let items = bid_domain::scoring::derive_scoring_items(reqs);   // ← 生产函数当 oracle 的一半
    let parents: HashSet<&str> = items.iter().filter_map(|i| i.parent_id.as_deref()).collect();
    let expected: Vec<&str> = reqs.iter().filter(|r| r.category == "评分项")…
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-005
严重程度: P2
模块: bid-mcp（测试副作用 + 自证耦合）
文件路径: bidcore/crates/bid-mcp/tests/e2e_artifacts.rs
代码行号: 373-416（硬编码 REQ-0082..0092 参数行）；149-156+1131-1142（run-id 目录 + latest 重刷）；232-233/256（>= 产量下限）
函数或测试名: e2e_artifacts_synth_tender
问题描述: ①仓内副作用：每次运行新增 artifacts/e2e/e2e-<stamp>/（14 文件+每章 md）且无上限清理；latest/ 用 remove_dir_all+重刷（同测试并发重入竞态）；②硬编码 REQ id 系当前抽取引擎产出：param.rs:204-210 幽灵校验只拦不存在 id，不拦错位 id——抽取重排后「电池续航 3 年→2 年」的负偏离语义可静默漂移到别的条款（red_runs==5 仍绿）；③n_reqs>=60/n_sec>=12/words>500 为历史产量下限（产出即真值）。
调用链: extract（引擎输出定 REQ 序）→ 测试硬编码 REQ-008x 绑参数行 → set_param_responses（幽灵校验不过拦错位）→ 导出偏离表/红 run 断言
触发条件: 抽取引擎改动使 REQ 编号重排（id 存在但语义换位）
实际影响: 语义漂移静默通过；仓内 artifacts 无界增长；latest 刷新在隔离副本/并发场景竞态。
复现步骤: 思想实验：交换 synth 夹具两个参数条款位置 → 幽灵校验通过、断言全绿但负偏离已挂错条款。
当前测试: 即本条。
缺失测试: 参数行按「条款摘要文本」锚定而非 id 锚定的断言（如按 param_name 反查 REQ）。
建议方案: ①REQ 锚改为语义锚（param_name+required_value 匹配）；②run-id 目录滚动清理（保留最近 N）；③latest 刷新加原子 rename。
证据: e2e_artifacts.rs:382-388:
    // 负偏离（★废标级——偏离表整行标红）：电池续航 3 年 → 2 年
    ParamInput {
        requirement_id: "REQ-0084".into(),
        param_name: "温湿度传感器电池续航".into(),
        required_value: "≥3 年".into(),
        response_value: "2 年".into(),
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-006
严重程度: P2
模块: bid-mcp（测试真实性——人审门链的 MCP 可达性）
文件路径: 跨文件：validate_tool.rs / e2e_artifacts.rs / real_corpus_e2e.rs / global_invariants.rs / export_readback.rs / score_tools.rs（服务面 record_review/set_requirements/draft_section/upsert_file 直调）；对照 subagent_llm_path.rs（全 MCP 面）
代码行号: 代表点：real_corpus_e2e.rs:508-523（record_review 三门直调）；global_invariants.rs:488-496；validate_tool.rs:181-190（svc.draft_section）；export_readback.rs:311-342；score_tools.rs:129-131
函数或测试名: 各 e2e/integration 主链
问题描述: 除 subagent_llm_path（CI 恒 SKIP，MCT-003）外，**没有任何常跑测试经 MCP 工具面走完人审门链**（record_review_gate 需宿主签发 receipt——issue_approval_receipt 非 MCP 工具，设计使然；测试普遍用 svc.store_mut().record_review 直调绕过整条链）。绿灯 e2e 演示的「三门全过→导出」流程对外部 Agent 的真实形态（持 receipt 调 record_review_gate）只在 CI-SKIP 的 MANUAL 测试里出现。
调用链: 测试 → svc.store_mut().record_review（域面直写）→ readiness 门放行（MCP 面的 receipt 验签/门参数解析全跳过）
触发条件: 常规 CI 回归
实际影响: record_review_gate 工具的 MCP 壳层（gate 枚举/decision/receipt 参数面、#180 摘要绑定）在常跑测试中零执行——壳层回归只能靠 addendum_waiver/review 相关小册子（不在本批）。
复现步骤: rg 'store_mut\(\)\s*\.record_review' bid-mcp/tests/ → 5 文件命中；rg '"record_review_gate"' → 仅 real_llm_path/subagent_llm_path（均 CI SKIP）。
当前测试: 即本条。
缺失测试: 一条 CI 常跑的「宿主签发 receipt → MCP record_review_gate 三门 → 导出」链测试（可用 issue_approval_receipt 服务缝签发+MCP 提交，同 real_llm_path 563-572 的写法，无需 LLM）。
建议方案: 把 real_llm_path 的 G3 段（svc.issue_approval_receipt + proc.call("record_review_gate")）抽成无 LLM 依赖的常跑 CONTRACT 测试。
证据: real_corpus_e2e.rs:508-515:
    svc.store_mut()
        .record_review(Gate::AddendumWaiver, &bid_domain::TrustedPrincipal::human("real-corpus-lead"), …)
        .unwrap();
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-007
严重程度: P2
模块: bid-mcp × bid-tools（PA-005/#245 测试面回声）
文件路径: bidcore/crates/bid-mcp/tests/e2e_artifacts.rs + tests/real_corpus_e2e.rs + tests/global_invariants.rs + tests/subagent_llm_path.rs
代码行号: e2e_artifacts.rs:275-313（单证据绑 qual_ids 全体）；real_corpus_e2e.rs:396-432；global_invariants.rs:442-480（⑤ 绑定后 blocking==0 断言 497-501）；subagent_llm_path.rs:441-453（bind["matrix"]["blocking"]==0）
函数或测试名: 各主链夹具的「绑定后 blocking 清零」断言
问题描述: 四个全链夹具都用**一条合成资质证据（无 locator 内容核验、verification_tier=None）绑定全部必选 REQ** 并把 blocking==0 当作链路前置断言——#245/PA-005 的「占位资质绑定→阻塞归零」场景即当前所有绿灯 e2e 的标准夹具形态；无任何「FILE_EXISTS 级证据绑定后矩阵仍阻塞/待补」反例（PA-005 缺失测试在 MCP e2e 面同样成立）。
调用链: register_evidence（tier 服务端裁决但不被消费）→ bind_evidence 全体必选 → build_matrix（has_ev=!evs.is_empty()）→ blocking==0 断言 → 导出
触发条件: 常规 CI（绿=占位放行面在场）
实际影响: e2e 绿灯为「来源存在=资格满足」的当前语义背书；tier 语义化改造时四个夹具的 blocking==0 前置全部需要重写（回归阻挡面）。
复现步骤: rg 'blocking.*0|blocking_rows.*0' 四文件 → 均为链路前置断言。
当前测试: 即本条。
缺失测试: tier=FILE_EXISTS（或无 locator）证据绑定后 blocking 不清零/矩阵呈现待补的反例断言。
建议方案: 在 PA-005 修复排期时同步加「占位证据不清零」夹具；e2e 主链改用带 excerpt/locator 的证据（更接近真实核验通过形态）。
证据: subagent_llm_path.rs:449-453:
    assert_eq!(
        bind["matrix"]["blocking"],
        json!(0),
        "绑定后 blocking 清零（G2 工具面）: {bind:?}"
    );
审计置信度: CONFIRMED
状态分类: 当前仍存在（夹具形态与 PA-005 缺口互证）
```

```
ID: MCT-008
严重程度: P2
模块: bid-mcp（PB-006 测试面确认）
文件路径: bidcore/crates/bid-mcp/tests/score_tools.rs
代码行号: 452-493 + 687-701（svc.store_mut().upsert_scoring_mapping 直写）；455 注释「（域层直写——MCP 写面后续 issue）」
函数或测试名: score_document_reports_mappings_unmapped_and_multi_mapping / score_document_mapping_report_attributes_effective_source_only
问题描述: #224 映射账本（upsert_scoring_mapping）仅能经测试特权直写构造——MCP 工具面无任何写入口（C1 PB-006 CONFIRMED）。score_section/score_document 的 mappings/contributions/inert 归因等读面测试（实现质量很高）演示的是**生产不可达的账本态**：外部 Agent 无法把评分项映射到章节，只能依赖 params.section_title 派生路径。
调用链: 测试 → store_mut().upsert_scoring_mapping → build_scorer 读账本 → score_* 出参
触发条件: 常规 CI（绿≠Agent 可达）
实际影响: 映射账本功能的验证完整性依赖测试特权；若未来加 MCP 写工具，这些测试无法回归其壳层。
复现步骤: rg 'upsert_scoring_mapping' bid-mcp/src/ → 零命中（仅 tests 与 bid-domain）。
当前测试: 即本条。
缺失测试: 不适用（缺的是生产写面——PB-006 范畴）；测试侧可补「MCP 面确无该工具」的负断言防越权。
建议方案: 随 PB-006 排期；测试注释已如实登记。
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-009
严重程度: P3
模块: bid-mcp（错误路径可达性——注入特权）
文件路径: bidcore/crates/bid-mcp/tests/validate_tool.rs
代码行号: 288-289（set_section_content 清空——注释自认测试特权）；422-426（幻觉引用 store 直写投毒）
函数或测试名: rule_section_empty_is_blocker / rule_citation_ghost_is_blocker
问题描述: E_SECTION_EMPTY 与 E_CITATION_GHOST 两错误分支的触达态由 store 直改构造（绕过 CAS/绕过 draft 回查）。生产面最接近的通路是 draft external 手稿（content 参数）——external 正文可注入幻觉引用，但 draft 回查会剔除「不在本章响应面」的引用，测试选择的投毒形态（引用在响应面内但台账不存在）是否可经工具序列到达未验证。
调用链: store 直写 → validate 读 store → 规则命中
触发条件: 仅测试
实际影响: 低——规则本身逻辑被测到；风险是「上游防线（回查）漏掉的形态」与「测试构造的形态」不一致时测试给出虚假覆盖感。
复现步骤: 读 288/422 行注释（均自认测试特权）。
当前测试: 即本条。
缺失测试: 经 draft external content 构造同态的可达性验证。
建议方案: 补一条 external 手稿注入 [EV-ghost] 的 MCP 全链用例（若被回查剔除则说明该分支生产不可达，应改防御层测试定位）。
证据: validate_tool.rs:288-289:
    // 清空第一章（绕过 CAS 直接改 store 是测试特权——真实面 draft 失败即空）
    svc.store_mut().set_section_content(&sids[0], String::new()).unwrap();
审计置信度: CONFIRMED（注入特权）；NEEDS_RUNTIME_VERIFY（生产可达形态）
状态分类: 当前仍存在
```

```
ID: MCT-010
严重程度: P3
模块: bid-mcp（Oracle 独立性——历史基线与算法复制）
文件路径: bidcore/crates/bid-mcp/tests/real_corpus_e2e.rs
代码行号: 66-69（L1_BASELINE/P_BASELINE=main@51874cd 实测）；93-238（veto_measure 三通道匹配算法内嵌，注释自认「完全一致」）
函数或测试名: L1_BASELINE / P_BASELINE / measure_one
问题描述: ①红线基线数字是一次历史运行（main@51874cd）的产出当真值下限（>= 锁）——未从 GT 独立重推导，改善可过、恰等可过；基线注释含过期 commit SHA 引用。②匹配口径（块对齐/拼接包含/≥6 字子串+去空白规范化）整段复制自 scripts/veto_measure——测量算法与离线工具同构：若测量口径本身有偏（如子串通道过宽），测试与工具一起错，无第三方独立实现。
调用链: GT(人工) × 引擎输出 × 同构匹配算法 → 指标 → >= 历史基线
触发条件: 匹配算法或基线口径变更
实际影响: 红线的「不退化」语义绑定在特定测量实现+历史快照上；换测量口径需人工重标基线（当前无护栏提示）。
复现步骤: 读 66-69 注释（「veto-measure 于 main 51874cd 实测，逐位一致」）。
当前测试: 即本条。
缺失测试: 基线数字与 GT 重推导的一致性护栏（或把基线写入可追溯的测量产物文件而非源码常量）。
建议方案: 基线常量改由生成脚本+落盘 JSON（含 SHA/日期/口径版本），测试读文件并在口径变更时强制重生成。
证据: real_corpus_e2e.rs:66-69:
    /// #123 红线基线（veto-measure 于 main 51874cd 实测，逐位一致）：
    /// covered 309/318、veto_tp 291/302。
    const L1_BASELINE: f64 = 309.0 / 318.0;
    const P_BASELINE: f64 = 291.0 / 302.0;
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-011
严重程度: P3
模块: bid-mcp（SKIP 卫生 + 跨运行污染面）
文件路径: bidcore/crates/bid-mcp/tests/set_matrix_override_tool.rs + tests/search_evidence_tool.rs
代码行号: set_matrix_override_tool.rs:529-531（语料仅一条 blocking 行 → 无横幅静默 return）；search_evidence_tool.rs:745（tmp 路径 search-fields-{pid} 固定名、无 nanos、不清理）
函数或测试名: cross_requirement_receipt_is_rejected / register_evidence_rejects_malformed_fields（matrix_ready_store）
问题描述: ①数据条件 SKIP 无横幅（与其他 SKIP 的「响亮横幅」纪律不一致）——当前 corpus-A 有多条 blocking 实际执行，属潜伏形态；②search fields 校验夹具的 /tmp 工作区按 pid 命名且持久残留：同 pid 复用（重启后）+ 陈旧目录 → 二次运行读到 MATRIX_READY 旧态 → extract E_STATE → 假红（与假绿相反的跨运行污染面）。
调用链: ①语料形态判定 → return；②/tmp/search-fields-{pid} 残留 → BidService::open 复用旧库 → 状态机前置错位
触发条件: ①语料退化为单 blocking；②pid 复用+陈旧 /tmp
实际影响: ①SKIP 无观测面；②偶发假红（概率低）。
复现步骤: ①构造单 blocking 语料跑 cross_requirement（无横幅直过）；②保留 /tmp/search-fields-<pid> 重跑同 pid。
当前测试: 即本条。
缺失测试: 不适用（测试卫生问题）。
建议方案: ①该 return 也打横幅；②tmp 路径加 nanos 或收尾 remove_dir_all（同 global_invariants ws_dir 惯例）。
证据: set_matrix_override_tool.rs:529-531:
    let Some(rid_b) = rid_b else {
        return; // 只有一条 blocking 的语料形态——跳过（不适用本测试）
    };
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: MCT-012
严重程度: P3
模块: bid-mcp（MCP-011 缺口回声 + oracle 同源度）
文件路径: bidcore/crates/bid-mcp/tests/cursor_pagination.rs
代码行号: 362-405（缩水面唯一覆盖：同构小 store offset 越界拒）；509-519/648-656（evidence/task oracle=domain 直读镜像）
函数或测试名: matrix_bad_cursors_fail_closed_no_panic / evidence_list_filter_paging_and_cursor_exclusivity / tasks_list_filter_paging_and_cursor_exclusivity
问题描述: ①「底层列表增长→静默滑窗」无测试（契约 §0 增长方向承诺落空无护栏——与 C1 MCP-011 判定一致：当前测试只有缩水拒）；②evidence/task 的「独立 oracle」实为同数据源 domain API 直读（svc.store().evidences() 过滤）——对分页/过滤逻辑独立性成立，对台账数据本身不独立（若 register/apply_validity 有 bug，oracle 与 SUT 同错）。
调用链: ①翻页中途 register/bind → offset 合法 → 滑窗；②oracle 与 SUT 共享 DomainStore
触发条件: ①MatrixReady/EvidenceBound 态分页遍历中插行；②台账层 bug
实际影响: ①遍历完整性静默受损无护栏；②低（domain 另有单测）。
复现步骤: ①思想实验：两页间 register_evidence 一条 → 断言无红；②读 509-519。
当前测试: 即本条（缩水面是好测试）。
缺失测试: 翻页中途插行 → 拒或告警的行为断言（MCP-011 建议配套）。
建议方案: 补一条「页 1 取得后插 3 条 → 页 2 断言当前行为（至少钉死现状）」的行为锁。
证据: cursor_pagination.rs:399-404:
    let (mut small, small_pid) = matrix_store(5);
    …
    assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "底层列表缩水 → offset 越界拒");
（增长方向无对应分支）
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

---

## 5. 结论摘要

1. **覆盖**：12/12 文件全读，110 个测试函数逐一定性（R5）。整体测试文化高于仓均水准：fail-closed 纪律、零副作用复检、反例证明判定面（global_invariants）、独立回读（export_readback）、receipt 攻击面全家福（set_matrix_override）都是正面范式。
2. **P0 一项（MCT-001）**：validate_tool.rs:217/242/554 把 #243 问题行为（validate 无条件推进 VALIDATING）三重锁死为预期；同时全套件无 VALIDATING 态 MCP draft 死锁面测试——修复 #243 将被测试反向阻挡（与 PA-003 完全对齐）。
3. **假绿面 Top3**：①real_corpus_e2e.rs:871-884 解析器粒度 SKIP——L1/P GT 红线在 poppler/CI 环境从不执行且计 passed；②real_llm_path.rs:400-414 + subagent_llm_path.rs:540-554 env 门控——真实 LLM 传输面 CI 恒 SKIP（env_preflight.rs:315 自认「计数 passed」）；③e2e_artifacts.rs:917-948 + export_readback.rs:844-850 PDF 软门双 SKIP——无 soffice 环境 PDF 导出链断言为零（auto→real 生成器回退分支亦零覆盖）。
4. **global_invariants 守卫缺口一句话**：五不变量全部是「账本↔产物」数据对账维度且质量高（反例可红），但 PA-001/PA-004/MCP-003/MCP-017/MCP-011/MCP-002/PB-005/PA-005 八项已知问题无一设有不变量——状态机语义/契约字面量/裁决门三维度无网，P0 恰落无网区且被 MCT-001 反锁。
5. **副作用清单（Lead 隔离运行预测）**：仓内新增 `artifacts/e2e/e2e-<stamp>/`（14 文件，无界累积）+ `artifacts/e2e/latest/` 重刷 + `artifacts/real-corpus-e2e/latest/`（3 件×11 文件+summary，缺 real-corpus 时不刷新）；/tmp 十类前缀工作区（search-fields-* 不清理）；MANUAL 测试副作用全在外部 WS（CI 不触发）。
6. **测试→生产映射（R5 前哨）**：search/cursor/score/import/set_matrix_override 五册为「dispatch 全链真生产」；validate/export_readback/e2e_artifacts/global_invariants/real_corpus 为「MCP SUT+服务面夹具」混合（人审门链仅 CI-SKIP 的 subagent 走全 MCP 面——MCT-006）；score_tools 确认 PB-006（映射写面仅测试特权可达）。
