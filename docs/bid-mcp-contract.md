# 标书业务 MCP 工具契约（Bid MCP Contract）

> 定位：MCP Server 是**薄壳**——工具只做参数校验、状态机裁决与转发，业务逻辑在后端服务层（参照 BidMaster services/mcp/server.py 的形态，但规避其 AGPL 实现）。每个工具均要求幂等；响应统一携带 `project_version`（乐观并发）。
> 通用响应包装：`{ ok, tool(回显), data, request_id, project_version?, warnings? }`；
> 错误：`{ ok:false, tool, error:{ code, what, hint?, item_errors? } }`（`what` 是人读错误
> 正文——#80 第三批外部 Agent 实测摩擦 #12：历史契约写 `message`，实现面是 `what`，
> 以实现为准修订本文；`tool` 是调用回显字段）。

## 0. 通用约定

- **错误码**：`E_AUTH` / `E_NOTFOUND` / `E_STATE`（状态机拒绝越级或非法转换）/ `E_CONFLICT`（project_version 过期）/ `E_PARTIAL`（部分条目失败，附 item_errors）/ `E_UNPROCESSABLE`（解析不可恢复）/ `E_DRAFT_INCOMPLETE`（#243——全稿最终校验前置拒：起草未完成，附 `missing` 叶清单，零副作用）/ `E_RATE` / `E_INTERNAL`。
- **行协议信封（#279）**：stdio 裸 JSON 路径（`bid-mcp-stdio` / `dispatch_json`）每行一个 `{"tool":"...","args":{...}}`。参数放**顶层**（`{"tool":..,"name":..}`）时历史行为是 serde 静默吞掉 + 工具层报误导性的「字段缺失」（#269 六场 11 次失败调用）；#279 起顶层出现该工具已知参数名 → 直接报**信封结构错误**：`ToolCall 解析失败：参数须嵌套在 "args" 下（收到顶层字段 name、bid_deadline，未见 args）`。未知顶层字段不触发（不越权猜测，维持工具层原语义）。
- **project_version 乐观并发（#279 第 3 项）**：写工具接受三种形态——① 非负整数：强一致 CAS，过期 `E_CONFLICT`（报错带对齐建议，不变）；② 字符串 `"latest"`：显式声明「我确认无并发」，跳过检查（单代理顺序作业不必追踪版本号）；③ 缺省/null：内部步骤语义，跳过检查（现状不变）。非法字符串 → `E_UNPROCESSABLE`（文案指明合法形态）。
- **agent 声明过门（#277 方案 A——用户裁定「ai可以操控这个过去但是留痕」）**：审核门（record_review_gate / record_addendum_waiver）双路验签——人类签域（#182 现状）+ agent 签域（scope 带 `:agent` 后缀 + actor_type=agent，agent 持钥自签声明）。通道开关 = 工作区配置 `<ws>/.mang/config.json` 的 `require_human_receipt`（缺省 **true** 严格模式——Q1 裁定：保守，自动化跑批显式写 false）。放行面：GatePassRecord 审计账（GP-xxxx，append-only——谁/什么身份/什么方法/何时/为什么，reason 必填非空）+ 导出警告横幅（Q3 措辞）+ 响应面 `agent_waivers`/`human_review_complete` + 事后人工追认（重过门追加 human 条，最新条定警告，两账可追溯）。防线不弱化：混用签域双向拒（NOT_AGENT_ACTOR / NOT_HUMAN_ACTOR——scope 进 MAC）；System 恒拒。
- **幂等策略**：写操作接受 `idempotency_key`；同一 key 重复调用返回首次结果（不重复执行）。文件按 `sha256` 天然幂等；条款抽取按 `dedupe_hash` 幂等合并。
- **大结果分页（S-4，V3-5）**：列表类工具（get_compliance_matrix / list_evidence / list_tasks，及 M2-b 的 search_evidence）不得一次返回全量（12,000 行矩阵 / 10,000 条证据量级）——统一游标信封：
  `in : { project_id, filters:{...}, cursor?, limit?, page?(仅矩阵兼容) }`；
  `out: { rows|items, total(过滤后总数), limit(生效页宽), cursor(本次生效游标), next_cursor: string|null }`，`next_cursor=null` 即取完，原样回传取下一页。
  游标规则（fail-closed）：opaque base64（内嵌版本字节/偏移/过滤器指纹）；跨过滤器或跨工具复用游标 → `E_UNPROCESSABLE`；limit 合法域 1..=1000、缺省 100，越界/非整数**显式拒**（不静默 clamp）；游标损坏/版本不认识/偏移越界（底层列表已变化）→ `E_UNPROCESSABLE`。行序确定性（矩阵=条款位置序 CMP-0001..N；证据=id 字典序；任务=target_id 字典序）是分页安全前提，不得破坏。
- **补充建议（第 9 个只读工具）**：`get_bid_project_state`——断点恢复的入口（读项目状态、各阶段完成度、DIRTY 章节、BLOCKER 计数），仿 LingYan `check_pipeline_state`。原 8 工具契约不受影响。V3-5（S-4）另增两个只读列表工具 `list_evidence` / `list_tasks`（§4-A/§4-B，工具面共 11）。issue #42 阶段 10 增第 12 个工具 `save_version`（AI 主动存版本快照，见文末 §SV）。#80 第三批（G1-G3）后工具面 21；#219 全块覆盖三工具（§17/§18/§19）后工具面 24（§3 同步抽取通路合并语义）；#226 增 `set_matrix_override` → 25；#243 增 `recover_validating_state`（§20，校验状态安全与恢复）后工具面 26；#254 增分段三工具 `append_section_segment` / `rewrite_section_segment` / `list_section_segments`（§21，章节级增量写入）后工具面 29；#285 ① 增 `get_star_coverage`（§22，★漏标检测——只读）后工具面 **30**。

- **补充建议（第 9 个只读工具）**：`get_bid_project_state`——断点恢复的入口（读项目状态、各阶段完成度、DIRTY 章节、BLOCKER 计数），仿 LingYan `check_pipeline_state`。原 8 工具契约不受影响。V3-5（S-4）另增两个只读列表工具 `list_evidence` / `list_tasks`（§4-A/§4-B，工具面共 11）。issue #42 阶段 10 增第 12 个工具 `save_version`（AI 主动存版本快照，见文末 §SV）。#80 第三批（G1-G3）后工具面 21；#219 全块覆盖三工具（§17/§18/§19）后工具面 24（§3 同步抽取通路合并语义）；#226 增 `set_matrix_override` → 25；#243 增 `recover_validating_state`（§20，校验状态安全与恢复）后工具面 26；#254 增分段三工具 `append_section_segment` / `rewrite_section_segment` / `list_section_segments`（§21，章节级增量写入）后工具面 29；#279 增 `list_tools`（§LT，行协议发现面）后工具面 30；#287 增 `recommend_length_profile`（§22，篇幅档位推荐）后工具面 **31**。

## 1. create_bid_project
```
in : { name, tender_notice_ref?, bid_deadline, mode(明标|暗标), external_key }
out: { project_id, status: "DRAFT" }
幂等: external_key 相同 → 返回已有 project_id（E_PARTIAL 不适用，静默幂等）
警告: bid_deadline 已过 → **不报错**，warnings[] 携带文案
      「BID_DEADLINE_PAST: 投标截止 … 已过（契约 §1：创建即警告，不阻断）」
      （#86 摩擦 #14 清偿：旧版把 BID_DEADLINE_PAST 写在「错误: E_CONFLICT」
      条目下，与实现不符——实现一直是 warning 不阻断，契约以实现为准修订）
状态: → DRAFT
```

## 2. import_tender_files（内含知识库入库）
```
in : { project_id, files:[{upload_ref, filename, kind(招标文件|附件|图纸|补遗|答疑|评分办法|格式模板)?, expected_pages?}], project_version }
      upload_ref（#183/#189 文件围栏）: 必须落在受信根内——BID_FS_ROOT 声明的
      导入根，或 BID_WS 工作区（二者的并集）；相对路径按受信根锚定（不再按
      进程 CWD 解析）；词法 `..` 分量预拒（先于一切磁盘操作，零副作用）。
out: { file_ids:[TF-*], per_file:[{file_id, kind(#271 回显), sha256, pages, tables, chunks, ocr_tier, parse_status, deduplicated, errors?, warnings?(#188 METADATA_CONFLICT | #270 CONVERTED_FROM_DOC)}],
       totals:{files, chunks, parse_failed, partial},
       forms_manifest?:{ count, items:[{file_id, name, parse_status, pages, tables}] }
         ← #271 格式件清单（kind=格式模板 的库内登记逐件可见；无格式件时键省略）。
           格式件（附件/ 下官方填空模板）不产 REQ/不进矩阵/不阻塞起草
           （readiness 不在面）；解析产物进 parsed 累积，可经
           get_document_chunks 读模板结构；extract_requirements 时格式类
           REQ 尽力关联 params.form_file_id/form_name。
       addendum_readiness:{ total, ready, not_ready:[{file_id, name,
         readiness(READY|FAILED|OCR_PENDING), waived, reason}] } }   ← #184 分级就绪报告
幂等: sha256 已存在 → no-op 返回原 file_id（deduplicated:true）。
      #188 业务角色不随去重丢失：同 SHA **异身份**（kind/文件名不同）导入
      → 登记 FileRole（角色追加不覆盖，判定面看任一身份——补遗/正本识别
      从此看得到「它也是补遗」）；完全相同的重放（同 SHA 同 kind 同名）纯
      no-op。原记录 kind/name 不覆盖，import 响应 per_file.warnings 携带
      「METADATA_CONFLICT: 同 SHA 文件以不同身份再次导入（原 X「a」，本次
      Y「b」）…」显式报告（告警不失败）。
语义: 后端确定性管线——解析（表格单元格级、扫描件 OCR 分档）→ 结构化切块（chunk_id 稳定、登记 文件→位置 映射）→ 证据注册（切块即入 EV 台账——后续 register_evidence 签发的 EV-id 从已有计数后继，不是从 EV-0001 起——#80 第三批外部 Agent 实测摩擦 #7）→ 向量+精确双索引。**Agent 不做解析与切块，只核对返回统计并重试失败件（R-01）。**
限额: 单件 500MB / 单批 64 件 / 累计 2GB（#183——超限 E_UNPROCESSABLE 逐件报因）
警告: 补遗件非 READY 且未逐件确认 → warnings[] 携带
      「ADDENDUM_NOT_READY: 补遗 <名> 未就绪（<原因>）——补遗可能改控制价/
      截止时间/评分办法，请修复重传或人工确认后放行（record_addendum_waiver）」
      （#184——import 是 Agent 看到坏补遗的第一现场；已确认件不重复告警）
错误: E_PARTIAL（部分文件 parse FAILED；失败件内容全程禁引；data 携带成功面含
      addendum_readiness 块）；E_STATE（project 已过 IMPORTED 阶段追加新文件 → 允许，但标 dirty 波及）；
      E_UNPROCESSABLE（#183 围栏：路径越界/词法 `..`/symlink 段逃逸/超限额——
      先于项目路由与状态机，整批零副作用；无受信根时响亮拒并指路 BID_WS）
状态: DRAFT → FILES_IMPORTED
```

## 3. extract_requirements
```
in : { project_id, scope:"all"|file_ids[], options:{granularity, include_scoring_table:true}, project_version }
     options.granularity：**保留位，当前未实现**（传入被忽略——#86 摩擦 #6 清偿：
     契约曾提到但无实现，如实标注；未来实现时枚举合法值并在此补记）
out: { counts:{total, added(可为负——整集替换语义下的诚实增量), by_category{qualification,veto,scoring,tech_param,commercial,format,other}},
       merge:{regex_total, regex_dropped_covered, agent_requirements, uncovered_blocks, coverage_complete}（#219 二步合并面），
       star_summary:{total, captured, missing, declarations}（#285 ① ★漏标检测汇总——
         抽取面文件口径；total 为真值★（样板句已排除，captured+missing==total
         恒成立）；missing>0 时 warnings 追加 STAR_MISSING；逐文件明细走 §22
         get_star_coverage），
       conflicts:[{a:REQ-*,b:REQ-*,reason}], missing_sequence:[{file_id,expected_clause_nos,found}], warnings[] }
执行: 确定性先行（条款编号序列解析、表格结构化槽位抽取）→ #219 二步抽取通路合并
      （Agent 块标记优先 + 正则兜底）：被任何区间标记覆盖的块，正则产物丢弃
      （Agent 已看过该块——无论 noise/rule/review_table/requirement）；未覆盖块
      正则保留（兜底）；Agent 提交的关键点转 REQ-Axxxx 追加（冲突以 Agent 为准；
      params 携 statement/mandatory/block_idx/marked_by=agent，溯源锚=块原文）。
      落库改整集替换（replace）——「先 extract → 补标 → 重跑」时被覆盖块上的旧
      正则 REQ 消失（只增不删的合并语义做不到）；REQ-A 后缀非纯数字，CMP 派生
      id 走 CMP-0000-REQ-Axxxx 回退分支，与正则 CMP-000N 零碰撞。
      **无标记时严格 no-op**：与旧正则产物逐位一致（GT 红线零位移）。
      B3（图纸目录/证据件，无抽取面）不进合并统计。
      warnings 新增：UNCOVERED_BLOCKS（未覆盖块数——正则已兜底，Agent 补标后
      重跑更准）；AGENT_MANDATORY_OFFCATEGORY（Agent 标必选且类别不在类别
      阻断面资格条件/否决项——**已接线**：经 matrix.rs `agent_mandatory`
      进入矩阵 blocking 面，未响应即拦起草/导出；响应或人工 receipt 覆写后
      放行。历史名 AGENT_MANDATORY_NOT_BLOCKING 时代只亮警告不拦）；STAR_MISSING
      （#285 ① ★漏标警告——N 处★未见对应 REQ，不是 BLOCKER（可能误报），
      逐文件核实面在 get_star_coverage）。
硬规则: 条款编号缺号 → missing_sequence 必须报告（完整性校验）；补遗与正文冲突 → 双向登记 conflicts；
      合并守恒：counts.total = regex_total - regex_dropped_covered + agent_requirements
错误: E_PARTIAL（某文件抽取失败，其余成功）；E_STATE（未导入文件）
状态: FILES_IMPORTED → REQUIREMENTS_EXTRACTED
```

## 4. get_compliance_matrix（只读）
```
in : { project_id, filters:{category?, status?}, cursor?, limit?, page? }
     category: 契约键（qualification|veto|scoring|tech_param|commercial|format|other）或中文类别名；
     status  : NOT_RESPONDED|RESPONDED|PARTIAL|NOT_APPLICABLE|PENDING_EVIDENCE
               （T-2+#245 五态派生——PENDING_EVIDENCE = 仅占位/待核验证据
               在绑：待补可见、必选行仍 blocking）；
     cursor  : opaque 游标（与 filters+project_id 绑定，优先于 page）；limit: 1..=1000（缺省 100）；
     page    : 兼容旧调用（映射 offset=(page-1)*limit；page≥1，0 → E_UNPROCESSABLE）
out: { rows:[{cmp_id, requirement:{id, clause_no, category, statement, mandatory, source_anchor, params},
              evidence:[{ev_id, title, status, valid_until, verification_tier}],
              pending_evidence_ids:[EV-*]（#245 占位/待核验证据待补追踪面）,
              evidence_required: bool, evidence_missing: bool（#241 裁定正交标记——
                与 status 分账：正文实写即 RESPONDED（文本事实），材料要求与
                缺口由这两列承载（合规事实）；evidence_missing=true 的必选
                行阻断提交就绪但不拦导出）,
              substantive: bool（#276 ★实质性条款——识别列，数据源 params.star
                （标记面 submit_block_marks.requirements[].substantive ∪ 抽取面★
                检测两条路径统一派生，matrix.rs `substantive`）；一条不响应即
                响应无效。**不参与 blocking 派生**（★响应门的分级是 Lead 裁决面）——
                验收看 star_summary 统计口）,
              gap, response_policy, draft_section_id, blocking}],
       total(过滤后总数), limit, cursor(本次生效), next_cursor: string|null,
       counts:{total, returned, page(信息性回显), page_size(=limit)},
       star_summary:{total, responded, partial, not_responded, not_applicable,
         responded_share: f64|null}（#276 ★实质性条款统计口——**全量口径**，
         不受 filters 影响：验收面「★全部识别并响应（对照 manifest.star_count）」
         的可编程基础；NOT_APPLICABLE 单列不计入 responded——不适用是免除响应
         而非满足响应，验收单独对账；无★条款时 responded_share=null（非 0/0 假象）） }
无状态副作用；任何阶段可调。行序 = 条款位置序（cmp_id CMP-0001..N，确定性稳定）；
先过滤后分页（total 恒为过滤后总数）。
`rows[].requirement.mandatory` = 类别阻断面（资格条件/否决项）∪ Agent 必选标记
（params.marked_by=agent && params.mandatory=true）——与矩阵 blocking 派生同一真源
（matrix.rs `is_mandatory`/`agent_mandatory`），工具面 mandatory 即门禁输入。
状态派生（#241 裁定，2026-11-05）：正文有实质响应（归属章节 content 非空）且
无有效证据无占位 → **RESPONDED**（写了就是写了——文本事实）；证据缺口由
evidence_missing 正交承载，由提交就绪面（validate/check_bid_quality 的
submission_readiness）拦截，不压进状态面。
错误: E_UNPROCESSABLE（游标与过滤器不匹配/游标损坏/limit 或 page 越界——显式拒不 clamp）
```

## 4-A. list_evidence（只读，S-4 新增）
```
in : { project_id, filters:{kind?, status?}, cursor?, limit? }
     kind    : qualification|performance|personnel|certificate|product_doc|solution_module|contract
               （或对应中文展示名——与矩阵 category 双形态口径一致）；
     status  : ACTIVE|EXPIRING|EXPIRED|SUPERSEDED（T-1 台账状态）
out: { items:[{id:EV-*, kind(snake_case 机器形态), title, status, subject_name, valid_until,
            verification_tier(PLACEHOLDER|FILE_EXISTS|CONTENT_MATCHES|null——
            #217+#245 五态可见：null = 旧记录登记于四态面之前；
            PLACEHOLDER = 占位/待核验——不得当作已验证证据)}],
       total(过滤后总数), limit, cursor(本次生效), next_cursor: string|null }
无状态副作用；任何阶段可调。行序 = id 字典序（确定全序）；先过滤后分页。
错误: E_UNPROCESSABLE（非法 kind/status、游标与过滤器不匹配、limit 越界）；E_NOTFOUND（项目路由）
```

## 4-B. list_tasks（只读，S-4 新增）
```
in : { project_id, filters:{owner?, status?}, cursor?, limit? }
     owner  : 执行人精确匹配；status: ASSIGNED|IN_PROGRESS|DONE（T-6 人侧进度）
out: { items:[{target_id:SEC-*|REQ-*, owner, deadline, status}],
       total(过滤后总数), limit, cursor(本次生效), next_cursor: string|null }
无状态副作用；任何阶段可调。行序 = target_id 字典序（domain BTree 语义——R-15 可重放）；
先过滤后分页（过滤下沉 DomainStore::list_tasks 双过滤，工具层只切页）。
错误: E_UNPROCESSABLE（非法 status、游标与过滤器不匹配、limit 越界）；E_NOTFOUND（项目路由）
```

## 5. search_evidence（知识库检索——双通道；#217 正文摘录面）
```
in : { project_id, query, mode:"hybrid"|"semantic"|"exact", exact?:{field:"model|amount|date|clause_no|cert", value},
       filters:{source_type?, tags?, valid_at?:开标日, min_valid_until?}, top_k, threshold? }
out: { hits:[{ev_id, chunk_id, content, source:{file, position}, score, match_type:"semantic|exact|both",
            excerpt(正文摘录——#217；null = 文件级锚未定位),
            excerpt_locator(《文件》第 N 页 / 《文件》正文段 N),
            verification_tier(FILE_EXISTS|CONTENT_MATCHES|null)}],
       trace:{semantic_k, exact_k, merged_by, content_face:"title+scope+subject+client+excerpt"} }
硬规则: 资格类查询默认 exact 优先；valid_until < 开标日 的证据自动降权并附 warning=EVIDENCE_EXPIRING/EXPIRED；
      排序带确定性 tie-breaker（score, evidence_id）保证可重放（R-15）；
      **semantic 通道检索面含正文摘录（#217）——正文含词但标题不含的词
      （如合同页文本里的「水利工程」）可命中；命中质量受摘录上取 500 字
      限制（页级截取——全文检索/分块是 M2-b 面）**；
      **M2-b 落地时必须走 bid-mcp cursor.rs 游标模块（S-4）——hits 面同属大结果列表，
      分页信封/游标绑定/fail-closed 语义与 §0 大结果分页约定统一，不得自带第二套分页口径**。
错误: E_STATE（项目无企业资料库绑定）
```

## 6. draft_response_section
```
in : { project_id, section_id? | spec:{title, order, parent_id?, toc_ref}, instructions,
       source_requirements:[REQ-*], evidence_ids:[EV-*], style_profile?, project_version,
       generator?:"auto"|"mock"|"real"|"external",     ← #68 阶段 1 三态 + #100 任务 2 external
       content?:"外部手稿全文" }                        ← #100 任务 2（external 的正文来源）
out: { section_id, content_ref:"DOC-*", version, stats:{words, citations, unanchored_claims:0,
       generator:"real"|"mock"|"external", uncovered_requirements:[REQ-*]}, warnings[],
       input_integrity:{ status:"COMPLETE"|"RESTRICTED",       ← 地图 P0-1：输入完整性面
         waived_files:[{file_id, name, readiness, reason}] },
       draft_note }                                        ← #128（#127 P0-2）：指令与缺失
       回报的【留档账面】——随 attempt 落盘 ChapterRecord::draft_note，
       人类可读；与正文严格分离（渲染永不碰）：正文零「起草指令：」「缺失
       回报：」标记（原缺陷：缺失回报整段嵌在开篇括号里把句子劈断）。
       生成器 prompt 面的注入（with_missing_report）与账面同源不同层。
     ← stats.generator 标记实际生成器；uncovered_requirements = 覆盖度自动检查
       （#68 阶段 2 §2.1——响应面内未被正文回应的 REQ，逐条另发 warning）
幂等: (section_id, instructions_hash, inputs_version) 相同 → no-op 返回原稿；
     external 稿幂等键追加 content 指纹（同章节同指令不同稿 = 不同请求；同稿重提 → deduplicated）
前置: 矩阵 READY（资格/否决行 policy 已填）；EV 状态 OK（EXPIRED 拒绝绑定→E_UNPROCESSABLE）；
     引用的 REQ 必须在 depends_on 中
生成: #68 阶段 1 三态 + #100 任务 2 第四态——auto：配置了 api_key（.mang/llm.json 或 BID_LLM_API_KEY 等 env，
     env 覆盖文件）走真实 LLM（OpenAI 兼容 chat/completions），未配置降级 mock 并 warning；
     real：未配置 key → E_UNPROCESSABLE（显式要求要响亮拒绝）；mock：强制确定性生成（e2e 面）；
     external：正文来自 content 参数（外部作者手稿——子代理/人工写好直注，免中继），引擎不生成
     只提交（引用回查/CAS/落盘与 mock/real 同路径）；content 在场即 external 语义，与
     generator=mock/real 显式互斥（E_UNPROCESSABLE），external 无 content / content 空白 → E_UNPROCESSABLE。
     真实通路：60s 超时重试 1 次；max_tokens 按 target_chars 推导（防失控，completion 超
     硬上限 2 倍 → 整章废弃报错）；用量累计落 .mang/llm-usage.json（api_key 绝不落日志/台账）。
执行: 后端经**文档内核稳定接口**创建/更新 DOC 块（本系统不触碰内核内部结构）；引用覆盖率校验——承诺句无证据引用计为 unanchored_claims>0 时 warning（阻断留给 validate）
阻塞: draft 前置 readiness 闸（#110 任务 1 + L3；#184 升级为分级；地图 P0-1 扩面）：无关联 REQ/缺
      资质/业绩不足/技术参数无佐证/否决项未响应/补遗不就绪（MissingAddendum
      ——任一件非 READY 且无逐件确认即阻塞，**部分失败/零文本扫描件也算**，
      拒稿理由逐件点名文件名与原因）/其余输入件不就绪（UnreadyInput——主件/
      附件/图纸 FAILED 或 OCR 零内容；「已提取出一部分需求」不构成放行理由，
      门禁以文件级状态为准）七种 Blocker → E_UNPROCESSABLE（人话
      理由，不泄漏枚举名）。人工出口分两支：无补遗件 = addendum_waiver 门
      （§12）；任何不就绪件（补遗或主件/附件）= record_addendum_waiver 逐件
      确认（§16）。确认放行后可起草，但 out.input_integrity.status =
      RESTRICTED（受限起草——放行 ≠ 没问题，响应/账面（draft_note）/机器面
      三处显式标注，不得静默当成完整输入；waived_files 逐件点名确认来源）。
大纲: 否决项/资格条件不写规则原文但合并进【资格与响应】响应章节组（#121
      P0-1/#126：评标规则不进正文）；评分项完全不进正文（只进自评表）；
      「其他」类二次分组（章节→主题→兜底条款号区间，不裸「（第N块）」）。
状态: SEC → DRAFTED；项目 → DRAFTING
```

## 7. validate_bid_document
```
in : { project_id, scope:"all"|section_ids[], rules_profile:"standard|dark_bid", project_version }
out: { findings:[{vf_id, kind:DETERMINISTIC|MODEL, rule_code, severity, section_id?, requirement_id?, message, evidence}],
       counts:{blocker, major, minor, deterministic, model},
       categories:{ #187 分类返回（纯加法新增；ok/findings/counts 原样保留）
         structural: {status:"passed"} | {status:"failed", findings:[…]},
         semantic:   {status:"passed"} | {status:"failed", findings:[…]}
                   | {status:"not_performed", reason:"no model configured"},
         evidence:   {status:"passed"} | {status:"failed", findings:[…]},
         compliance: {status:"passed"} | {status:"failed", findings:[…]} },
       submission_readiness:{ #241 裁定（2026-11-05）——提交就绪面（纯加法；与 ok 分账）
         ready: bool, not_built: bool, blockers: usize,
         unresponded_count, evidence_missing_count,
         unresponded:[{requirement_id, row_id, category, excerpt, message}],
         evidence_missing:[{requirement_id, row_id, category, excerpt, message}] } }
提交就绪语义（#241 裁定——Export ≠ Submission Readiness）：
  · 正文已响应但无外部证据 → 矩阵 RESPONDED（文本事实——写了就是写了，
    不再报 E_QUAL_UNMET/E_VETO_UNRESPONDED 冤枉真响应）；证据缺口由
    evidence_missing 清单承载（合规事实——没证明材料就是废标风险），
    ready=false（不得正式提交）但**不拦导出**（草稿可导出，见 §8）；
  · ok=true 只表示已执行检查零 BLOCKER，不等于可提交——就绪裁决看
    submission_readiness.ready；
  · 阻断两清单：unresponded（必选未响应/PARTIAL/待补——含 #253 验收 8
    否决项面）+ evidence_missing（必选条款证据缺口）；
  · 解决出口：补绑证据（register_evidence/bind_evidence——状态窗口
    MATRIX_READY..READY_TO_EXPORT，#241 后 DRAFTING 起亦可用）或合法
    人工豁免（set_matrix_override NOT_APPLICABLE + 偏离说明 + Human
    receipt；普通 RESPONDED 覆写不豁免缺口）。
规则分层（R-07 规则分界；原引审计问题 Q7）:
  确定性（代码断言，先行、必跑）: 报价 vs 控制价/预算、工期 vs 招标要求、资质有效期 vs 开标日、
      页数/字数上限（Word 渲染口径）、必填格式项、签章位存在性、暗标禁字清单扫描、条款引用完整性
  模型（仅产 finding 不改稿）: 表述质量、逻辑连贯、偏离合理化、疑似过度承诺
categories 四桶（#187——「没做 ≠ 做对了」，issue #9 口径）:
  structural（已实现）: E_SECTION_EMPTY / E_CITATION_GHOST / W_PLACEHOLDER /
      W_CHARS_OFF_TARGET / E_SCOPE_UNKNOWN / E_CHARS_BELOW_MIN（#253：仅
      评分章节 BLOCKER）/ W_CHARS_BELOW_MIN（#253：行政/实质章节 MAJOR）/
      E_TOTAL_CHARS_BELOW_MIN（#253：全稿总量硬门 BLOCKER）
  semantic（未实现）: M_ 前缀模型发现——现网未配置模型，恒
      not_performed{reason:"no model configured"}，不得因「没发现错误」伪装
      passed；接入真实 LLM 后经服务层注入缝（validate_document_with_reviewer）
      变为 passed/failed
  evidence（部分实现）: E_EVIDENCE_EXPIRED（效期面）+ E_EVIDENCE_PLACEHOLDER
      （#245：占位/待核验证据仍绑定——BLOCKER，占位不得冒充已核验证据）
      + E_CITATION_PLACEHOLDER（#245：正文引用占位 EV——BLOCKER，引用面冒充）
  compliance（部分实现）: E_VETO_UNRESPONDED / E_QUAL_UNMET
      （#245：PENDING_EVIDENCE 待补行计入未响应面，消息携带待补注记） /
      W_FORMAT_BASELINE（暗标禁字等后续阶段）
  ok 语义（#187 厘清）: ok=true 只意味着「已执行桶中 BLOCKER == 0」，不是
      「整本标书已通过审核」；not_performed 不把 ok 翻 false（能力缺席不是
      缺陷）。桶内 findings 与平面 findings 串接一致（桶序：structural →
      semantic → evidence → compliance，桶内规则触发序）。
状态（#243 收紧——校验状态安全）: 分面 + 条件推进——
      ① 前置闸：scope=all（全稿最终校验）要求全部叶章节达 DRAFTED。存在
      未完成叶 → `E_DRAFT_INCOMPLETE`（missing = 未完成叶清单）零副作用拒
      （不跑规则/不推进状态/不落版本/不动审核记录）——起草期误触发无后果，
      起草面照常可用；起草中的章节级检查走 scope=sections（不受此闸，恒纯读
      ——矩阵级规则 1/2 仍全量报告未响应面）。
      ② 仅「scope=all 且 ok=true（BLOCKER==0）」才推进 DRAFTING → VALIDATING
      （复审语义）+ checkpoint；校验失败（ok=false）留在原态——项目保持可
      修订、可重试（修订通道：draft_response_section / set_matrix_override /
      证据补绑），无状态变迁则不落版本。重复校验幂等（无变迁不推进）。
      ③ 历史异常态（VALIDATING + 缺章——旧版无条件推进所致）的恢复走 §20
      recover_validating_state，禁止手改 db.json。
      BLOCKER 清零（或登记 WAIVED+理由）才可 EXPORT
      （#86 摩擦 #10 清偿：REVIEWING 是导出时自动补齐的瞬态——单次 export 内部
      经 Drafting→Validating→Reviewing→ReadyToExport→Exported 逐态补链，
      不会作为可观察的停泊状态出现在两次调用之间；人工审面是三门
      record_review_gate（§12），不是项目状态 REVIEWING。若未来需要人工终审
      停泊位，再启用为可观察状态。）
出参补充（#80 第三批外部 Agent 实测摩擦 #2；#243 收紧）：校验通过且发生状态
      推进时，响应携带 project_version = 推进后的新值——后续工具调用必须用
      新值，否则 E_CONFLICT；前置拒（E_DRAFT_INCOMPLETE）与校验失败（ok=false）
      零副作用——版本不变，响应携带当前值。
project_version 步进源（#86 摩擦 #9 清偿）：project_version = 内容版本计数器，
      每一次【内容面落库】+1（set_requirements / set_section_content / 绑定 /
      状态推进 / 幂等登记等）；一次工具调用内含多步落库 → 步进 +N（N>1 常见，
      实测 +1~+6）；只读调用零步进。响应携带的 project_version 恒为本次调用
      完成后的最新值——以此为准传给下一个工具。
```

## 8. export_bid_document
```
in : { project_id, format:"docx"|"pdf", output_path(必填——产物路径，须为【工作区相对路径】，
       不得绝对路径/含 ..；如 "out/投标文件-技术标.docx"——#80 第三批外部 Agent 实测摩擦
       #1：历史契约漏写此参数，靠报错文案才能发现),
       （#189 导出围栏纵深：工具层形状校验之外，服务层 resolve_write 按受信根
       组件级判定 + 逐段 symlink 检查——字符串前缀陷阱与 symlink 逃逸在
       两层任一被拦），
       bid_open_date?(#322/R-05：开标日时效裁决——ISO 日期 YYYY-MM-DD。提供则
       在导出前按开标日对全部证据做单向翻转（Active/Expiring→Expired，
       只降不升；翻转后矩阵同步重算 R-04，矩阵门读到的是翻转后状态；
       翻转非空进 warnings 审计面）。未提供但存在带 valid_until 的证据 →
       响亮告警「时效未裁决」，不硬拒（契约兼容）；非法日期/非字符串 →
       E_UNPROCESSABLE 响亮拒),
       profile:{dark_bid? , template?}, options:{tracked_changes?, watermark?}, project_version }
嵌入: #68 阶段 3——章正文支持 md 块语法（无语法内容与旧版字节级同输出）：
     - 表格：连续 | 行（首行表头 + |---| 分隔行 + 数据行）→ docx 表格
       （表头加粗；分隔行是排版指令不进正文；无分隔行的 | 行回退普通段落）
     - 图片：整行 ![说明](路径或EV-id) → docx 内嵌图 + 图注「图：说明」
       · 相对路径 → workspace 根解析；绝对路径直取
       · EV-id → 证据台账 source_file → workspace files/{source_file}
       · 缺图/非图片格式/未知 EV → E_UNPROCESSABLE 响亮拒绝（不静默跳过
         ——标书缺资质证书图是废标级事故）
       · 版面适配：宽超内容区 6 英寸等比缩放（A4 安全面）
     - 标题行：# / ## … → 剥号加粗段（不再字面输出井号）
out: { artifact:{url, sha256, size}, content_version_snapshot, precheck:{blockers:0, waived:[VF-*]},
       tables_written, table_kinds:[...], forms_written, form_titles:[...],
       agent_waivers:[{ gate, actor, actor_type, method, human_confirmed, at, reason }](#277),
       human_review_complete(bool)(#277), warnings[] }
     ← #117/#122 导出面对账：六表/七件全部进 docx，逐张点名（tables_written
       == 6 且 table_kinds 逐类在座；forms_written == 7）。e2e 不信任工具
       返回值——解包 docx 数节验：`<w:tbl>`==6、七件关键词、表行数下限、
       红run 数、合计一致、零 markdown 星号（#129 深断言，12_export-check.json）。
横幅: （#277 方案 A）存在 agent 声明过门（每门最新一条 human_confirmed=false）
       → 产物正文首页（TOC 后）插红色粗体警告段：「【警告】本标书的部分
       审核门由 AI 声明通过，未经人工复核（门：…）」（Q3 措辞）+ warnings
       携带同一事实 + 响应面 machine-readable 清单。真人补 human receipt
       重过对应门后（追认，导出**前**——EXPORTED 是审核门终态）最新一条翻
       human → 横幅消失、human_review_complete=true，审计账两条并存可追溯。
六表: （#117 三真源 + #122 接线）①项目概况表（恒生成——REQ 参数+项目面，
      缺栏落【待填】）；②技术参数偏离表（有「技术参数」类 REQ 才生成——
      参数响应账 set_param_responses；负偏离★整行标红 5 格；无技术参数
      REQ → 缺席+响亮告警（不静默）；③报价明细表（profile.bid_amount +
      price_breakdown——分项合计=总价一致性断言；投标函大写金额换写）；
      ④业绩一览表（可用业绩证据——Performance 类带发包人/金额/完成日期）；
      ⑤人员配置表/⑥设备清单表（bidder-profile.json 的 personnel/equipment
      ——投标方输入面，主体信息档落盘）。
七件: 附件一投标函/二法定代表人身份证明书/三授权委托书/四投标保证金退还
      申请/五资格声明/六无重大违法记录声明/七廉洁承诺书（BidderProfile
      真源；缺字段标【待填】；标题 Heading2 样式段——防关键词假阳性）。
幂等: (project_id, content_version, format, profile, output_path, decision_fp) 相同
     → 返回缓存 artifact（#117：报价决策指纹入键——换报价重导是新导出）
门禁: 三门 Approved + 全 RT DONE 之外，#180 起另校验**审核内容绑定**：
     pre_export_confirm 最后一条 Approved 的 reviewed_content_digest
     （章节树摘要——结构定位 + 叶正文）≠ 当前 → E_UNPROCESSABLE
     「导出前置裁决未过：审核针对的内容已变更，请重新审核（REV-x 审核摘要
     aaaa…，当前 bbbb…）」——过门后改章节正文/结构 = 旧审核失效，重过门
     重新绑定。绑摘要**不绑**全局 content_version（后者被导入无关件/记审
     自身/导出推进一律 bump——必误报）：过门后导入无关件、状态推进、
     不改正文重复导出均不受影响；#180 前旧快照的空摘要记录跳过判定
     （宽容载入，旧数据的洞保留——重新过门即补上绑定）。
     **四前置门全量清单（#245 补齐 1d）**：
     · 1a 矩阵门：blocking 行清零（必选未响应**或 PENDING_EVIDENCE 待补**
       ——占位绑定不清阻塞。#241 裁定（2026-11-05）：正文有实质响应
       （归属章节 content 非空）且无有效证据无占位 → RESPONDED 不阻断
       ——证据缺口不拦导出，由提交就绪面（§7 submission_readiness）
       承载，草稿可导出）；
     · 1b 审核门：三门 Approved + 全 RT DONE + #180 内容摘要绑定；
     · 1c 写作验收门（1.0；#253 门禁分级）：**评分章节**篇幅不足
       （E_CHARS_BELOW_MIN——叶绑定评分项 REQ 或被 ScoringMapping 映射）/
       **全稿总量**不足（E_TOTAL_CHARS_BELOW_MIN——实写总量 < 配额总和，
       1000 页硬指标硬比较无容差）/ 空章节，即使审核已直接批准也拒
       （独立复算，不消费记审面）；行政/实质章节篇幅不足不拦导出
       （W_CHARS_BELOW_MIN MAJOR——WARNING 语义，内容完整即可）；
     · 1d 占位证据门（#245）：活跃占位绑定存在即拒——占位/待核验材料
       不得作为已核验证据导出（与 1c 同为独立复算门、不消费记审面；
       差异面：占位有合法人工豁免通道 = NOT_APPLICABLE 覆写 + 非空偏离
       说明 + 可信 Human receipt（#247 审核收紧：三要件缺一不可，
       Responded 等普通状态覆写即使由 Human 发起也不豁免），篇幅不足
       没有——只能补写）。解决通道：①导入真实材料重新登记（换版升级，
       绑定自动迁移）；②合法人工豁免。
状态: → READY_TO_EXPORT → EXPORTED
```

---

## 9. check_bid_quality（#68 阶段 2 新增——只读诊断）
```
in : { project_id }
out: { project_id, summary:{total, drafted, failed, pending, uncovered_req_total, out_of_band,
       unanchored_total}, needs_human:[SEC-*],
       submission_readiness:{ #241 裁定（2026-11-05）——提交就绪面（与 §7 同源同构：
         ready/not_built/blockers/unresponded_count/evidence_missing_count/
         unresponded[]/evidence_missing[]——单一出口，两工具面永不分叉）},
       sections:[{section_id, title, status, chars,
       target_chars, within_band, requirement_ids, covered_reqs, uncovered_reqs, ghost_reqs,
       cited_evs, uncited_evs, unknown_citations, unanchored_claims, error, needs_human}],
       artifact:{path, size}|null, warnings[] }
前置: MATRIX_READY+（起草中即可查进度）；章节树已建
语义: 只读诊断（D2 同款——报告是导数不是源数据）：
     - 覆盖度：响应面 REQ 是否被正文引用回应（未起草章不计——pending 计数单列）；
     - EV 核对：幻觉引用单列（unknown_citations）；绑定集未援引单列（信息面）；
     - 带宽：target_chars ±20%（target=0 或未起草不评——within_band:null）；
     - needs_human 判定：FAILED / 有未覆盖 REQ / 幻觉引用 / 带外字数；
     - FAILED 章错误明细从 S-2 的 failed.json 边车并入（assembler 在场时）；
     - B3 三维度评分（#131 接线——此前 ctx=None 评分器从未在生产路径评过）：
       逐章 specificity/substance/score（已起草才评，未起草 None 不伪装 0）
       + 报告级 duplication（跨章重复，Jaccard/包含率双口径）+ 严重重复对
       needs_human；评分器由真实评分项 REQ 驱动（SectionScorer+
       derive_scoring_tree；无评分项空树 → rationale 诚实说明，不伪装分）；
     - 落盘：{workspace}/artifacts/quality-report.json（workspace 在场；纯内存态
       artifact:null + 告警）。
     - requirement_ids（#86 摩擦 #4 清偿）：章节绑定的 REQ 全集——章节→REQ
       归属的正式查询口（无需从矩阵反推）；与 covered/uncovered/ghost 的并集
       一致（三者在响应面内互斥分桶）。
状态: 不推状态（只读）
章节发现面（#80 第三批外部 Agent 实测摩擦 #3，从末尾括号注提升到正文）：
     - 本工具 sections[] **只列叶节点**（可起草面，按 order 排序）——外部 Agent
       获取章节清单的正式查询口；get_bid_project_state 不回章节清单；
     - 章节→条款归属（draft 的 source_requirements / depends_on）：矩阵行
       requirement × draft_section_id 反推（get_compliance_matrix 的行内字段）；
     - 起草前 covered_reqs 为空属正常（覆盖度是起草后诊断面）。
```

## 附录 A：KBAdapter（知识库适配器接口）

> 对用户问题的落地：**知识库不是 MCP 工具，而是 import_tender_files / search_evidence 的可替换实现层**。接口先行，实现可切换。

```python
class KBAdapter(Protocol):
    def ingest(self, project_id, files: list[IngestFile]) -> IngestReport:  # chunks/失败清单
        ...
    def search(self, project_id, query, mode, filters, top_k) -> list[Hit]:  # 双通道命中
        ...
```

### AdapterA：自建薄服务（默认实现，推荐终态）
- 存储：PGVector 或 Qdrant（向量）+ FTS5/全文（精确）+ 对象存储（原文件）；SQLite FTS5 方案见 rag-tender（MIT 可复用）。
- 分块：**确定性结构化切块器**——按条款编号/标题层级切分，chunk_id 稳定并登记 `chunk → file → 位置`（证据链可回溯，ark/rag-tender 同路）。
- 检索：语义 + 精确双通道，元数据过滤（类型/行业/有效期），交替合并 + 确定性排序。

### AdapterB：Dify 知识库 API（可选对比实现，不作核心依赖）
官方文档核验过的端点（https://docs.dify.ai，Knowledge API key 鉴权）：

| 能力 | 端点 | 备注 |
|---|---|---|
| 建库 | `POST /datasets` | indexing_technique: high_quality/economy |
| 文本/文件入文档 | `POST /datasets/{id}/document/create-by-file`（旧版 `create_by_file`） | multipart：file + data JSON（process_rule 可配 separator/max_tokens/chunk_overlap） |
| 索引进度 | `GET /datasets/{id}/documents/{batch}/indexing-status` | batch 由创建返回 |
| 分段管理 | `POST/GET/DELETE /datasets/{id}/documents/{doc_id}/segments[/{seg_id}]` | 可注入证据锚点 |
| 检索 | `POST /datasets/{id}/retrieve`（retrieval_model: semantic/full_text/hybrid + 权重 + rerank + top_k/threshold） | hybrid 可配 keyword/vector 权重 |

**局限（引用自官方文档+实测社区证据）**：
1. 自托管版依附整套容器栈（api/worker/postgres/redis/向量库/sandbox/nginx）——"太重"未解决；Cloud 版零部署但**数据出域，投标业务不可接受（R-09）**。
2. 文件默认 15MB 上限（`UPLOAD_FILE_SIZE_LIMIT`），大招标文件需拆分上传。
3. 分块归 Dify 所有：chunk 边界与 ID 不受控，证据链须靠"逐段 create-by-text + 注入 `[EV-锚点]`"硬拼，弱化可回溯性（R-08）。
4. 版本漂移：端点命名与 retrieval_model 结构在 0.x→1.x 多次变更（旧文档 `create_by_file` vs 现行 `create-by-file`）；llm_bidwriting 的 DSL 绑死 0.5.0 是同型实证。
5. 许可证：Dify 开源许可证为修改版 Apache2.0（品牌与多租户 SaaS 条款），内部自用无碍，对外多租户 SaaS 需商业授权。

**结论**：AdapterB 仅用于检索质量 A/B 对比（TC-10）与过渡期兜底，锁版本、包一层契约测试。终态以 AdapterA 为默认。

### AdapterC：Cherry Studio 知识库（否决记录）
CherryHQ/cherry-studio（AGPL-3.0，52.9k★）为 Electron **桌面客户端**，知识库为本地功能（客户端解析+本地索引），无 headless 服务端 API；第三方 api-for-cherrystudio（141★）需另建 API 服务反证其客户端定位。**不能作为封闭 Agent 的后端 RAG**。其 MCP 客户端能力可用于人工调试前端，不作为运行时。


## SV. save_version（AI 主动存版本，#42 阶段 10 新增）

**用途**：skill 编排层「每完成一个章节，调用 save_version 存版本」纪律的落点。与桌面壳
（mang-app「改动」面板 / 30s 自动快照）写**同一份** `<workspace>/.mang/history/`
（mang-snap crate 单一真源），kind=ai 在面板显示 AI 徽章，支持预览 / md 层 diff / 安全回滚。

**入参**：

```json
{ "file_path": "<绝对路径，.md/.txt/.markdown>", "message": "<版本说明——必填，写明完成了什么>" }
```

**出参**：`{ ok, file, ts, snapshot, bytes, message, kind: "ai", workspace, hint }`

**规则**：
- `file_path` 必须绝对路径；`message` 必填非空（版本列表的核心信息）
- 只支持文本文件（源头是 md，二进制不做版本——issue #42 §八）
- 工作区根发现：从 file_path 向上找最近的 `.mang` 目录（桌面壳建过就在）；找不到用文件所在目录
- 不走项目状态机（版本快照与项目状态正交，任何阶段都允许存）
- 错误面：相对路径 / 空 message / 非文本 → `E_UNPROCESSABLE`；文件不存在 → `E_NOTFOUND`

---

## 10. build_outline（#80 第三批 MCP 补齐 · G1——外部 Agent 纯 MCP 驱动的前置）

> 背景：此前建纲/建矩阵只有服务面 `prepare_outline_and_matrix`（e2e 走进程内直调），
> 外部 Agent（Claude Desktop 等）在 extract 之后断链——评估见
> docs/subagent-llm-assessment.md §四缺口 G1。

```
in : { project_id, length_profile?("practical"|"standard"|"conservative"|"maximum",
       #287 篇幅档位四选一),
       target_total_chars?(1000..=5000000, 直接给字数覆盖档位),
       max_reqs_per_leaf?(1..=50, 缺省 8), project_version }
       三者优先（#287）：target_total_chars 显式给 → 用它；length_profile
       显式给 → 档位字数；都不给 → 默认档 Conservative 1,200,000
       （单一真源 bid_domain::outline::DEFAULT_TARGET_TOTAL_CHARS；用户裁定
       2026-11-06：120 万字 ≈ 1422 页，四档 400k/800k/1.2M/2.5M；拼错档位键
       E_UNPROCESSABLE 显式拒）
out: { counts:{ sections, leaves, requirements, total_target_chars, matrix_rows, matrix_blocking },
       length:{ profile("practical"…|"custom"), target_total_chars, approx_pages }
       （#287 生效档回显——审计面可核对），
       status: "MATRIX_READY", project_version }
语义: 推导大纲树（确定性——REQ 类目分组 + 每叶上限）+ 矩阵首建（推进
      MATRIX_READY——draft 的硬前置）。写后自动落盘（服务层 auto_checkpoint）。
配额（#253 第二段分层计价——按「分值/内容」分配，取代按 REQ 数均摊）:
      · 评分轨（叶绑定评分项 REQ；#252 评分驱动大纲落地后出现）：底价
        10,000/叶，剩余预算按分值权重（params.score）最大余数法分摊；
      · 行政轨（is_admin_group 单一真源分类）：300 字/条；
      · 实质轨（技术参数/服务/质量类）：2,000 字/条；
      · 剩余让渡序：评分 → 实质 → 行政；
      · 配额总和 == target（构造性恒等；底价不可负担的小目标退化面
        回退按 REQ 数均摊）。
状态: REQUIREMENTS_EXTRACTED 态可调（FilesImported/Draft → E_STATE；
      MATRIX_READY+ 重复调用 → E_STATE——重建面属「补遗后增量重抽」后续需求，
      不隐式放行）
前置: 条款面非空（空纲无意义——与 build_matrix 同口径）
幂等: 推导确定性（同输入同树）；状态机面不重入（见上）
```

## 11-A. register_evidence（#80 第三批 · G2；#185 缺口 1 来源验证；#217 四态 + 正文摘录；#245 占位态）

```
in : { project_id, kind(qualification|performance|personnel|certificate|product_doc|
                        solution_module|contract 或中文展示名),
       title, source_file_id(TF-*——必填，须为 import_tender_files 回执
                        per_file[].file_id 指向的已导入文件),
       subject_name, scope?, valid_from?, valid_until?(ISO 日期),
       placeholder?(bool——#245：true = 占位/待核验登记，默认 false),
       locator?({ kind: docx_paragraph|pdf_page_text, ...指纹字段 }——
               传入则与该文件解析产物对账：段落序/页号在范围内且
               text_fingerprint 与实际内容一致；缺省 = 文件级锚),
       client?, amount_wan?, completed_on?, fields?(#147 结构化字段),
       project_version }
out: { ev_id: "EV-*", deduplicated, kind(snake_case), status,
       verification_tier(PLACEHOLDER|FILE_EXISTS|CONTENT_MATCHES——
                        #217+#245 五态面),
       placeholder(bool——占位/待核验标记),
       excerpt(正文摘录——CONTENT_MATCHES 时非空；FILE_EXISTS 为 null；
               PLACEHOLDER 可携带（占位指向回溯面）),
       excerpt_locator(人话定位：《文件》第 N 页 / 《文件》正文段 N),
       hint(占位登记带警示文案——不清合规阻塞/不得声称已具备/升级路径),
       project_version }
语义: **来源身份由服务端验证（#185）**：source_file_id 查 TenderFile
      （不存在 → E_NOTFOUND；解析失败件 → E_UNPROCESSABLE 禁引）→ 解析
      产物按库内 sha 对账 → locator 验证 → sha256 与指纹**由系统生成**
      （带 locator → fnv64:<定位文本哈希>；文件级锚 → sha256:<库内 sha>），
      不接受调用方自报。语义位查重在域层（同 kind+title+subject+scope
      最新非 SUPERSEDED 记录：同指纹**且同验证态** → 幂等重放回原 id；
      异指纹或异验证态 → 旧记录单向降级 SUPERSEDED + 签发新 EV，
      **旧记录既有绑定迁移到新记录**（语义位响应面跟随当前版本，
      #245：占位→真实的升级路径由此承载））。locator 缺省 =
      MarkdownBlock{ block_id: "mcp-registered:<文件名>" }——诚实标记
      「经登记面、无引擎定位器」（审计可辨识，不冒充引擎抽取）。

      **#217 验证四态（缺口 2——「找到证据」≠「读过证据」的区分面）**：
      · FILE_EXISTS：无 locator 的文件级锚——来源在库且非失败件，但正文
        未定位（excerpt = null，生成器不得编造该证据的实料细节）；
      · CONTENT_MATCHES：带 locator 且对账通过——定位文本与解析产物逐字
        一致，服务端截取入账（excerpt，上取 500 字，「……」标记截断；
        excerpt_locator 人话定位），draft/search/导出面直接消费；
      · SEMANTIC_SUPPORT / HUMAN_CONFIRMED：**预留态**（枚举先立面）——
        达成路径分别为 M2-b 语义比对服务与 #182 TrustedPrincipal receipt
        人工确认链路，当前不可经登记面设置（服务端只裁决前三态）。

      **#245 占位/待核验态（PLACEHOLDER——占位资质不得冒充已核验证据）**：
      · `placeholder: true` → tier 强制 PLACEHOLDER（即便 locator 对账
        通过也不升格——「文件里有这段文本」≠「资质真实具备」；摘录照记，
        作为占位指向的回溯面）；
      · 调用方**可以自报占位**（从低不从高的诚实方向），但**不得自报任何
        已验证态**（verification_tier 参数不存在，传入即拒）；
      · 占位证据**可绑定**（矩阵行呈现 PENDING_EVIDENCE 待补 +
        pending_evidence_ids 追踪面），但**不清除合规阻塞**（必选行仍
        blocking）、不计为有效证据（资格章节就绪不满足——无证硬写拦）、
        导出门 1d 独立复算拒（即使审核已直接批准）；
      · 起草面：mock/LLM 生成器对占位证据输出【待补：...】占位文案——
        不引用 [EV-xxxx] 编号、不声称已具备/已满足；
      · 升级路径（受控、可审计）：导入真实材料后**重新登记**（同语义位
        异指纹/异验证态 → 换版：旧占位记录 SUPERSEDED 留审计痕 + 绑定
        迁移到新记录）——无原地迁移通道即无非法迁移通道；合法人工豁免
        = NOT_APPLICABLE 覆写 + 非空偏离说明 + 可信 Human receipt
        （#247 审核收紧：Responded 等普通状态覆写不豁免占位门）。
状态: MATRIX_READY..READY_TO_EXPORT（#241 裁定扩窗同 §11-B——DRAFTING 起亦可
      登记/换版（占位升级、缺口补件的登记面）；EXPORTED 终态仍拒）
错误: E_NOTFOUND（source_file_id 不存在）；E_UNPROCESSABLE（kind/日期/fields
      非法；**旧参数 source_file / source_sha256 / fingerprint 已废除——传入
      即拒**，文案指路 source_file_id；**#245：verification_tier 参数不存在
      ——传入即拒**（验证态由服务端裁决，自报拒）；placeholder 非布尔；
      解析失败件/无解析产物；locator 形态
      不支持（xlsx/docx_table_cell/markdown_block/canvas 无对账面）或越界
      /指纹不匹配）
```

> #185 破坏面：旧调用方传 `source_file`（文件名）+ `source_sha256` 的登记
> 方式**不再被接受**——那正是「登记不存在的合同.pdf + 任意 hex sha 骗过
> 格式校验成为可信证据」的来源身份漏洞。适配：先 import_tender_files
> （回执 `per_file[].file_id`），再以 `source_file_id` 登记。
>
> #217 域层不变量：`CONTENT_MATCHES 必须携带非空 excerpt`（空摘录的内容
> 一致声明是无凭据强声明）；`excerpt 与 verification_tier 不可拆`。
>
> #245 背景：2026-11-05 真实客户端运行中，Agent 注册占位资质并绑定到
> 大量矩阵行，阻塞数因此归零，被误读为真实资质已满足。占位态落地后：
> 占位绑定 → PENDING_EVIDENCE（待补可见）+ blocking 保留（绝不静默清零），
> validate 报 E_EVIDENCE_PLACEHOLDER / E_QUAL_UNMET（BLOCKER），导出被
> 1a 矩阵门或 1d 占位证据门拒绝（即使三门全批）。

## 11-B. bind_evidence（#80 第三批 · G2）

```
in : { project_id, ev_id, requirement_ids:[REQ-*](非空), bid_open_date, project_version }
out: { ev_id, bound_new, replayed, requested, placeholder(bool——#245),
       matrix:{ rows, blocking }, status, project_version,
       hint?(占位绑定时携带警示——不清除合规阻塞) }
bid_open_date 语义（#80 第三批外部 Agent 实测摩擦 #5）：证据有效性的裁决基准日
       ——工程口径取【开标日】（评标时刻的有效性，不是 create 的 bid_deadline
       递交截止日；两者通常相近但语义不同。无独立开标日真源时，用招标文件
       载明的开标日期）。
语义: 逐条绑定（REQ 不存在 → 逐件 E_NOTFOUND；EXPIRED/SUPERSEDED 禁绑 → 逐件
      E_UNPROCESSABLE——R-03，开标日视角即时裁决；重复绑定 → 幂等重放）+
      绑后矩阵重算（行内证据即时可见——不重算会让 get_compliance_matrix 看到
      陈旧行）。**#245：占位证据可绑（待补可见的载体），但绑定不清除合规
      阻塞**——必选行 PENDING_EVIDENCE 仍 blocking，回执 blocking 面与 hint
      直接可见。部分失败 → E_PARTIAL（item_errors 逐件定位 + data 携带成功面
      ——与 import_tender_files 同款「错误码不吞成功面」）。
状态: MATRIX_READY..READY_TO_EXPORT（#241 裁定（2026-11-05）扩窗：DRAFTING/
      VALIDATING/REVIEWING/READY_TO_EXPORT 亦可用——validate/check_bid_quality
      在这些状态输出 evidence_missing 清单，「见缺口→补件→就绪」的解决出口
      不能断在最后一步（与 #226 set_matrix_override 同款窗口理由；#245 占位
      换版升级同样依赖）。EXPORTED 终态仍拒——终态不改数据不变量。）
```

## 12. record_review_gate（#80 第三批 · G3；#131 增第四门；#182 receipt 验签；#277 agent 声明面）

```
in : { project_id, gate(matrix_approval|outline_approval|pre_export_confirm|addendum_waiver),
       decision(APPROVED|REJECTED),
       receipt(#182 人类签域：v1.<actor>.human.review_gate:{gate}.<ts>.<hmac>；
               #277 agent 签域：v1.<actor>.agent.review_gate:{gate}:agent.<ts>.<hmac>
               ——agent 持钥自签声明，须工作区 require_human_receipt=false；
               旧 reviewer 参数显式拒——Agent 不得自述身份),
       reason?(#277——agent 签域路径必填非空：为什么可以过这个门；human 路径可空),
       findings?:[{ severity(BLOCKER|WARNING|INFO), code, message, section_id? }],
       project_version }
out: { receipt:{ review_id, gate, decision, reviewer(legacy 回显——human/agent:{actor_id} 随身份),
       finding_ids, spawned_rt_ids,
       reviewed_content_digest(#180——本次记审绑定的章节树内容摘要；幂等重放时为既有记录的摘要),
       method(#277——human_receipt | agent_declared),
       human_confirmed(#277——agent 声明面 false) },
       actor:{ actor_id, actor_type, authentication_method }
       (#182 可信身份回显), gates_approved:[...],
       agent_waivers:[{ gate, actor, actor_type, method, human_confirmed, at, reason }](#277),
       human_review_complete(bool)(#277), status, project_version }
语义: #182 复审人门 receipt 验签（服务层 HMAC-SHA256 是 MCP 面唯一入口
      ——宿主持钥签发、24h 时效窗、scope 门级绑定、常量时间比对）；
      #277 方案 A（用户裁定：「我同意A，ai可以操控这个过去但是留痕」）：
      双路验签——先验人类签域（现状不变），不过再验 agent 签域
      （:agent 后缀 + actor_type=agent）：通过且工作区
      require_human_receipt=false → agent 声明过门（GatePassRecord 审计账
      GP-xxxx：谁/什么身份/什么方法/何时/为什么——append-only 追认不覆盖；
      导出带警告）；严格模式（默认 true）→ 拒并指路配置。混用签域双向拒
      （Human 用 agent 签域 → NOT_AGENT_ACTOR；Agent 用人类签域 →
      NOT_HUMAN_ACTOR——scope 进 MAC，交叉必 BadSignature）。幂等区分族：
      human 只对 human 记录幂等（agent 过的门真人补签 = 追认落新记录）；
      agent 对同 actor 同摘要幂等（重试不刷账）、换 agent 落新记录（可统计）。
      状态门/门序全在域层 record_review 裁决——壳层零重实现。
三门: 导出门序链（matrix → outline → pre_export——export_allowed 逐门校验）。
第四门 addendum_waiver（#131）：L3「无补遗阻塞」的人工出口——单件招标
      文件（无补遗件）起草会被 MissingAddendum 拒；Approved = 人工确认本项目
      无补遗 → missing_addendum 放行。与三门正交：无状态门、无门序前置
      （导入后任意时刻可记），不入 export_allowed 门序；补遗件在场时走正常
      判定（不因 waiver 洗白 ParseFailed——补遗解析失败仍阻塞；#184 起
      OCR_PENDING 零文本件同样不被洗白，其逐件人工出口见 §16）。
状态: MATRIX_READY+（域层按门细分窗口；addendum_waiver 无窗口限制）
绑定: #180 起每条记录落账快照章节树内容摘要（sections_digest：按 (order,id)
      排序逐节点喂 id/parent_id/title/level/order/content 六字段，\0 分隔，
      sha256-hex；排除 status/dirty/content_attempt 等不进产物的账面字段）。
      pre_export_confirm 的 Approved 摘要 ≠ 当前 → export_allowed 拒（见 §8）；
      同门重复 Approved 的幂等分支先比摘要——内容已变则落新 REV（重过门
      重新绑定，否则改稿后重过门永远返回旧记录 = 死锁）。
错误: E_UNPROCESSABLE（gate/decision/findings 形态非法；旧 reviewer 参数在场；
      receipt 验签失败——Malformed/NotHuman/Expired/FutureDated/BadSignature
      分类码随错误信息，scope 不符与伪造同归 BadSignature）；
      E_STATE（门序/窗口外）
```

> **外部 Agent 纯 MCP 驱动闭环**（G1/G2/G3 补齐后）：create → import → extract →
> build_outline → register/bind →（无补遗件时 addendum_waiver 门；补遗不就绪时
> record_addendum_waiver 逐件确认——§12/§16）→ 三门 →
> check_bid_quality（**章节发现面**——sections 只列叶，get_bid_project_state 不回
> 章节清单）→ draft → validate → export。
> 验收：`tests/mcp_scaffold_tools.rs`（零服务面直调全链 + 错误路径）。

> **真实语料 e2e 红线**（#131）：`tests/real_corpus_e2e.rs`——3 件真实水科 PDF
> （real-corpus/water/files，sha256 对应 GT A-01/A-04/A-05）每件独立项目跑全链，
> 测试内嵌 veto-measure 三通道口径断言：**L1 召回 ≥ 309/318 = 0.9717、否决类
> 精确率 ≥ 291/302 = 0.9636（#123 基线，逐位不退化）**。产物落盘
> artifacts/real-corpus-e2e/latest/。合成面（synth-tender 115 REQ + e2e 十项
> 深断言）与真实面双轨把关：「合成通过 ≠ 真实通过」。

## 13. score_price（#100 任务 3 · B1 价格分接线——纯计算器）

```
in : { method?:"benchmark"|"lowest"|"average"（缺省 average）| 完整对象
       （{"BENCHMARK_PRICE":{"coefficient":..}} / {"COMPOSITE":{..}} /
        {"DISCOUNT_RATE":{"base":{"FIXED":0.04}}}——复合法/下浮率法全参数面）,
       coefficient?（benchmark 缺省 1.0 / average 缺省 0.95）,
       max_score?（缺省 30）, budget?（控制价——benchmark 必需）,
       deviation?:"ratio"（缺省，87号令第五十五条口径）|"price"（需 deduct_high/deduct_low）
       |"rate"（需 deduct）| 完整对象,
       bid_price?（缺省 = 纯推荐模式）, others?:[$]（竞争报价） }
out: { spec:{method,max_score,budget,deviation_rule}, bid_price, others,
       score?, detail?{benchmark,deviation,deduction,score},
       optimal_price?, expected_score_at_optimal?, analysis（人读——方法口径+最优+试算）,
       warnings[], request_id }
语义: B1 PriceScoreSpec 只调用不改（五方法/比率与价格扣分/闭式最优全在引擎）。
      bid_price 与 optimal 可同场（「这么报得几分 + 最优该报多少」一次回答）；
      others 为空时基准价按己方单家口径（analysis 提示）；纯推荐模式推荐不出
      （竞争报价不足/方法无界）→ E_UNPROCESSABLE 响亮透传，试算在场则降级告警。
状态: 无（纯计算器——不路由项目不走状态机；无项目也能算）
错误: E_UNPROCESSABLE（method/deviation 形态非法；缺 deduct 参数；B1 validate 拒——
      如 benchmark 缺控制价、报价超控制价）
```

## 14. score_section（#100 任务 3 · B2 章节自评）

> **#186 语义降级（规则诊断非评标模拟）**：本工具是关键词覆盖/长度阈值/证据词
> 存在性的启发式诊断（`diagnostic_only=true, result_kind=rule_heuristic`），
> **不是评标模拟**——tier 是覆盖度档位，不代表评委会会给的档。item_scores
> 透传 `score_rules` 评分细则原文 + `rule_checkpoints` 关键条件比对结果
> （#224 第 3 步规则层可见性——量化/资质条件提取成检查点与正文做**提及性**
> 比对，明显未提 → 可能失分标记；**不改判档/估分**，完整语义判档依赖模型）。

```
in : { project_id, section_id, project_version? }
out: { project_id, diagnostic_only(true), result_kind("rule_heuristic"),
       section_id, section_title, chars, estimated, max,
       covered_items:[REQ-*], missing_items:[REQ-*], rationale（逐项可追溯）,
       item_scores:[{item_id,estimated,max,tier,coverage,is_objective,rationale,
                     weight_share（#224 摊分比——单映射 1.0）,
                     score_rules?（细则原文——检查点比对基准，无则缺省）,
                     rule_checkpoints:[{text,hits:[命中组数,总组数],mentioned}]?
                       （#224 第 3 步关键条件检查点——空/缺省 = 无细则或
                       无可识别条件；mentioned=false = 正文明显未提）,
                     rule_gaps（#224 明显未提的条件数——>0 = 可能失分）}],
       unmapped_scoring_items:[{req_id,criteria,weight}…]（#224 文档级告警——
       非空 = 有评分项未映射章节，总分可能偏低；价格项不入——价格分不依赖
       章节映射）, request_id }
语义: B2 SectionScorer 只调用不改（四维规则判定——覆盖度/字数/证据/具体性）。
       #224 多对多：同一评分项可映射多章节（ScoringMapping 账本——域层
       upsert_scoring_mapping 登记，MCP 写面后续 issue）：本章该项满分 =
       原满分 × weight_share（跨 2 章 0.6/0.4 → 两章该项满分 12/8）——
       同一项跨章各算一次导致总分虚高的主防；单映射 share=1.0 与历史行为
       逐位一致；rationale 带「摊分N%」说明（share≠1.0 时）。
       评分项 section 映射：REQ params.section 优先，缺省按大纲绑定关系联结
       （REQ 所在叶——确定性兜底，无 LLM）；价格类叶跳过内容自评（价格分 =
       f(报价)，由 score_price 计算）；结构节点（有子）与未起草叶 → E_UNPROCESSABLE。

       ⚠️【#170 方案 A 已落地——评审表章引用联结（2026-11-02）】
       评分项**不进正文**（outline.rs:324），leaf_of 反查对评分项不命中
       （该路径保留，对正文类目 REQ 仍有效）。评分项的 section 映射新增
       **章引用联结**（bid-parse ↔ bid-mcp 两段接线）：
         · 抽取侧：评审表行文本的「第X章」引用（详见/按/响应 第X章…——
           bid-parse chapter_ref_of）登记为 params.section_title；
         · 联结侧（build_scorer）：章号归一（中文/阿拉伯统一）定位
           「被引用章的正文叶」——① 锚在该章的**正文类目** REQ 所住的叶
           （语义最准：被评内容就是那一章的条款；否决/资格的响应叶不算）；
           ② 大纲节点 title 自带章号（「其他」类目按章分组的形态）→ 子树
           首叶。两路首现序（确定性可重放）。
       由此 `score_section` 恢复「这一节能得几分」——含挂载到该叶的评分项。
       边界（如实）：评审表行**无**「第X章」引用时 params.section_title 缺省
       → 该评分项维持 section=None → score_section 对该条恒空（数据缺引用
       时不虚构映射——与 #126 后的诚实行为一致）。
状态: MATRIX_READY+（只读诊断——与 check_bid_quality 同窗口）
错误: E_NOTFOUND（section 不存在）；E_UNPROCESSABLE（未起草/结构节点）；E_STATE（无章节树）
```

## 15. score_document（#100 任务 3 · B2 整本自评——「这份标书能得几分」）

> **#186 语义降级（规则诊断非评标模拟）**：同 §14——total/tier 是规则启发式
> 诊断结果（`diagnostic_only=true, result_kind=rule_heuristic`），**不代表
> 评委会评分**；external Agent 不得以此为对外承诺依据。`score_rules` 已被
> #224 第 3 步**规则层消费**：细则原文透出（by_item）+ 关键条件检查点与
> 正文比对（提及性——不改判档/估分）；notes 以 SCORE_RULES_CHECKPOINTS
> 显式登记（含可能失分项计数），完整语义判档依赖模型（另开 issue）。

```
in : { project_id, project_version?,
       bid_price?（缺省 = 不计价——「没报价就不算」，price 出 null 而非 0）,
       others?:[$], method?/coefficient?/max_score?/budget?/deviation?（价格口径，
       仅 bid_price 在场时消费——参数面同 §13 score_price） }
out: { project_id, diagnostic_only(true), result_kind("rule_heuristic"),
       technical, price（null | 数值）, commercial（null | 数值）, total,
       target（诚实满分 = 非硬门可达叶 max 合计——含价格项与未映射项，不因没写缩分母）,
       gap = target − total, by_item:[{item_id,criteria,max,estimated,tier,coverage,
               section_id（首映射——旧键保留）,
               mapping_ids:[SEC-…]（#224 全部映射章节——多对多面）,
               weight_share（#224 首映射摊分比）,
               contributions:[{section_id,estimated,tier,coverage,weight_share,
               rule_gaps}…]（#224 多对多聚合：estimated = 逐章贡献之和——与
               total 计入口径同源，多章不取首章；tier 多章同档→该档、异档→
               多章混合列举；coverage 份额加权均值；rule_gaps 逐章缺口数；
               价格项 contributions 恒空）,
               score_rules（细则原文 | null——检查点比对基准）,
               rule_checkpoints:[{text,hits,mentioned}…]（#224 第 3 步检查点
               聚合面——跨章任一章提及即提及；价格项恒空数组）,
               rule_gaps（#224 全部映射章都未提的检查点数——>0 = 可能失分）}],
       mappings:[{item_id,criteria,weight,sections:[{section_id,section_title,
               weight_share,source(RULE_PARSED|AGENT_SUBMITTED|MANUAL),
               source_anchor:{file,kind,locator,excerpt},evidence_refs:[EV-…]}]}]
               （#224 映射报告——每个评分项→哪些章节→权重多少→依据原文哪里；
               RuleParsed 派生映射锚回 REQ 首锚，账本映射带登记原文锚+来源+证据；
               只认**生效**映射的来源——同节 inert 账本行（派生赢）不归因）,
       breakdown:[SectionScore…（逐章节；#224 摊分后的章内满分）],
       unmapped_scoring_items:[{req_id,criteria,weight}…]（#224 结构化 UNMAPPED——
       非空 = 响亮告警「有 N 个评分项未映射章节，总分可能偏低」）,
       notes[]（UNMAPPED/SECTION_MISSING/NO_PRICE_INPUT
       /SCORE_RULES_CHECKPOINTS 诚实登记——含「N 项可能失分」计数）,
       price_input（价格口径 echo | null）, sections_scored, request_id }
语义: B2 score_document 三段聚合（技术 = 主观叶估分和；商务 = 客观叶；价格 = B1
      实算）+ by_item 全量逐项（未覆盖 estimated=0 如实呈现）。e2e 十一件套的
      11_self-score.json 即本工具产物（scripts/e2e_bid/run_e2e_artifacts.sh）。
状态: MATRIX_READY+（只读诊断）
错误: E_UNPROCESSABLE（价格口径非法）；E_STATE（无章节树）
```

## 16. record_addendum_waiver（#184 补遗分级阻塞的逐件人工确认；#277 agent 声明面）

> 背景：#184 前旧判定「3 份补遗坏 1 份不拦工作流」（只有**全部**失败才阻塞），
> 且 OcrRequired 一律放行——零文本扫描件被当「没有内容」处理。翻案理由：
> **不知道 ≠ 没有**——坏的那份可能正是改控制价/截止时间的，按旧条款做标是
> 不可逆的废标；工作流被拦是可修复的摩擦（修复重传 / 逐件确认），且「放行」
> 本身也要变成留底的可审计行为，而非静默赌概率。
> #269 六场实证：单件招标（无补遗件）常态踩 MissingAddendum 门，纯 Agent 无
> 宿主签发面 → 设计内死锁（P5 全断的成因之一）；#277 方案 A 补 agent 声明面。

```
in : { project_id, file_id(TF-*——须真实存在，import 响应/拒稿理由中的
       addendum_readiness.not_ready[].file_id),
       reason(非空白——不可审计的确认不成确认；#277 agent 路径同构必填),
       receipt(#182 人类签域：scope=addendum_waiver:{file_id} 件级绑定
       ——TF-0003 的 receipt 免不了 TF-0004；#277 agent 签域：
       addendum_waiver:{file_id}:agent + actor_type=agent——须工作区
       require_human_receipt=false；旧 principal 参数显式拒),
       project_version }
out: { recorded:{ file_id, reason, at(unix epoch 秒),
       principal(legacy 回显——human/agent:{actor_id} 随身份),
       actor:{ actor_id, actor_type, authentication_method },
       method(#277——human_receipt | agent_declared),
       human_confirmed(#277——agent 声明面 false) },
       addendum_waivers:[{file_id, reason, at, principal}…全部确认记录——
       追加式审计日志：同件重复确认追加不覆盖，解除不走本面],
       addendum_readiness:{ total, ready, not_ready:[{file_id, name, readiness,
       waived, reason}]（已确认件仍在清单但 waived=true——审计面不因放行
       而消失）}, status, project_version }
语义: #184 分级就绪的伴生闸（地图 P0-1 起扩面到全部招标包输入件）：任一
      输入件不处 READY（FAILED / OCR_PENDING）时 draft_response_section 被
      MissingAddendum（补遗面）/ UnreadyInput（主件/附件/图纸面）拒（§6，
      逐件点名）；本工具对具体 file_id 记一条人工确认 → 该件不再构成阻塞。
      非全局布尔——「3 份坏 1 份」要确认到具体哪一份、为什么故意带病放行。
      写账即 content_version +1；导入可解析版本后阻塞自然消失，确认记录
      无害残留（与 §12 addendum_waiver 门同款语义）。工具名带 addendum 是
      #184 历史口径——确认面按 file_id 落账，覆盖所有招标包输入件（证据
      源文件不在此面，可用性归证据系统）。确认放行后起草为**受限起草**
      （§6 out.input_integrity.status = RESTRICTED——放行 ≠ 没问题）。
分级: Ok → READY；OcrRequired 且 ocr_blocks>0 → READY（OCR 已出内容，与
      ocr_tier "ocr" 档同口径）；OcrRequired 零块 → OCR_PENDING（域层暂无
      「OCR 跑过但零产出」与「还没跑」的区分信号，合一态保守阻塞）；
      Failed → FAILED。混存分类取重：有 FAILED 件报 parse_failed，否则
      ocr_pending（逐件真相在 not_ready 清单）。
状态: FILES_IMPORTED+（含 EXPORTED——与 §12 addendum_waiver 门同窗口：导入
      后任意时刻可确认，阻塞可能出现在任何后续阶段；DRAFT 拒 E_STATE）
错误: E_UNPROCESSABLE（principal 缺 human: 前缀 / reason 空白）；E_NOTFOUND
      （file_id 不存在于 files——对不存在的件做确认 = 凭空免责）；E_STATE
      （DRAFT）；拒绝路径零副作用
```

## 17. get_document_chunks（#219 全块覆盖 · 只读）

> 背景：条款抽取依赖正则匹配字符形态（`1.1`/`一、`/`第X章`），真实招标文件里
> 技术要求用「1、」编号、大量流程条款被误判必选（#226）、无编号段落/表格内容
> 完全丢失。架构转变：**引擎切块 + Agent 标区间**——引擎保证块 ID 稳定与区间
> 校验，语义判断归 Agent（coverage.rs 职责边界）。

```
in : { project_id, file_id, from?(缺省 0), size?(缺省 50，上限 200) }
out: { file_id, total_blocks, from, returned, next_from(取完为 null),
       chunks:[{ block_idx, kind(Para|TableRow|PdfLine|OcrLine|XlsxRow),
       locator, text, cells, verdict(已标记块的结论回显——未标记 null) }],
       coverage:{ covered_blocks, complete, uncovered_ranges, by_verdict } }
语义: 取单文件的块文本窗口（join 键 = sha256——与 #217 locate_content 同口径）。
      size 上限 200 防上下文吃爆（160 页文件 3600+ 块）；from 越界 → E_UNPROCESSABLE。
错误: E_NOTFOUND（file_id 不存在）；E_UNPROCESSABLE（size 0/>200、from 越界、
      无解析产物在服务态——须先 import 成功）；E_STATE（状态机窗口外）
状态: FILES_IMPORTED / REQUIREMENTS_EXTRACTED
```

## 18. submit_block_marks（#219 全块覆盖 · 写）

```
in : { project_id, file_id,
       marks:[{ from, to, verdict(requirement|noise|rule|review_table), reason? }]（非空，
       **整集替换该文件的标记集**——多窗口翻页标注时必须带全量，不是增量追加）,
       requirements?:[{ block_idx, statement(非空), category?(七类合法值——
       资格条件|否决项|评分项|技术参数|商务要求|格式要求|其他), mandatory? }]，
       project_version }
out: { marks_accepted, requirements_accepted, total_blocks,
       coverage:{ covered_blocks, complete, uncovered_ranges, by_verdict },
       next(人话指引：全覆盖 → 可继续 build_outline；否则 → 补标) }
语义: 提交区间标记 + Agent 提取的关键点。#219 二步起两者**原子落库并持久化**
      （PR #227 只存 marks，reqs 校验后回显即丢——二步修上这一半；经服务层
      auto_checkpoint，重启不丢）。requirements 是 extract_requirements 合并面
      （§3）的输入——block_idx 必须落在 verdict=requirement 的区间内。
      verdict 语义：requirement=需响应；noise=无关；rule=招标人规则说明
      （#226 防死锁核心——常被误判否决项）；review_table=审查表。
错误: E_UNPROCESSABLE（marks 空/区间非法/越界/重叠/requirement 越区间/
      statement 空/category 非法）；E_NOTFOUND（file_id）；E_STATE（窗口外）；
      拒绝路径零副作用
状态: FILES_IMPORTED / REQUIREMENTS_EXTRACTED
```

## 19. get_coverage_report（#219 全块覆盖 · 只读）

```
in : { project_id }
out: { complete, total_blocks, covered_blocks, uncovered_range_count,
       files:[{ file_id, total_blocks, covered_blocks, complete,
       uncovered_ranges(前 50 段), uncovered_range_count, by_verdict }],
       next(人话指引) }
语义: 全部导入文件的覆盖聚合（含 B3 证据件——与 §17 单文件窗口互补的全局面）。
      uncovered_ranges 为连续未覆盖块的合并区间 [(from, to)…]。
错误: E_STATE（窗口外——Validating 起全程放行）
状态: FILES_IMPORTED+（Validating 起含全部状态）
```

## 20. recover_validating_state（#243 校验状态安全与恢复 · 历史异常态出口）

```
in : { project_id, reason(必填非空白——审计面：不可审计的恢复不成恢复),
       project_version? }
out: { status("DRAFTING"), recovery:{ at, from:"VALIDATING", to:"DRAFTING",
       reason, missing_sections:[SEC-*…] }, project_version, request_id }
语义: 历史异常态的受支持恢复。旧版缺陷（#243 之前的 main）：
      validate_bid_document 在草稿未完成/校验失败时仍无条件推进 VALIDATING，
      而 VALIDATING 态禁 draft_response_section → 起草死锁，用户被迫终止
      进程并手工改 db.json 的状态字段。本工具取代那条不受支持的路径。
      三重约束（不是任意状态写入）：①仅 VALIDATING 态（Reviewing+ 属审核/
      导出阶段，语义不同，不在本通道）；②仅缺章（存在未 DRAFTED 的叶——
      正是历史缺陷制造的异常态；草稿完整的 VALIDATING 是健康态（校验已过），
      拒之）；③理由必填非空白。
      效果：status → DRAFTING（起草面解锁）+ content_version +1 +
      StateRecoveryRecord 审计账（何时/为何/从哪到哪/当时缺哪些章——随快照
      持久化，跨重启可见）+ checkpoint。章节树/矩阵/审核记录不动（恢复是
      状态面动作，内容面原样——恢复前后的不变量由测试锁定）。
重复/重试: 恢复成功后项目在 DRAFTING，重复请求被 guard 拒（E_STATE——
      非死锁态，起草面照常可用），审计账不重复；并发：MCP 服务串行处理
      （&mut 服务态），无交错写。
错误: E_STATE（非 VALIDATING 态——含重复恢复/项目本在 DRAFTING 可起草）；
      E_UNPROCESSABLE（草稿完整的 VALIDATING / reason 空白）。
状态: 仅 VALIDATING
```

## 21. 章节分段写入三工具（#254 章节级增量写入 · Skill 分批写作的引擎对偶面）

### 21-A. append_section_segment（追加段）

```
in : { project_id, section_id(叶章节), content(本批段文——非章节全文),
       idem_key?(缺省=内容指纹 sha256 前 16 hex——同段同内容重提自动 no-op),
       project_version? }
out: { section_id, segment:{ id("SEG-xxxx"), seq, version, chars }, changed,
       deduplicated(!changed), assembled_chars, target_chars, remaining_chars,
       over_target, segment_count, citations, unanchored_claims, warnings,
       status, project_version, request_id }
语义: 真增量追加——段按 seq 升序入账，合稿 = 全部段 "\n\n" 拼接写回
      OutlineNode.content（单一真源；引用回查/无锚检查/字符统计/
      E_CHARS_BELOW_MIN/checkpoint/导出全部按合稿，下游零改动）。
      种子语义：整稿写过的章节首段追加 → 既有正文自动入册为第 1 段
      （绝不静默丢正文）。幂等：同章节同 idem_key 重放 → no-op
      （changed=false，content_version 不动）。状态推进/工作区落盘
      （ChapterRecorder 合稿）/checkpoint 与整稿路径同链。
引用: pre-flight 整段拒——段内 [REQ-*]/[EV-*] 不在本章绑定集 →
      E_UNPROCESSABLE 点名（分段不自动剔除，保「合稿==段拼接」不变量；
      与整稿路径的「剔除+告警」语义差异是刻意的）。
错误: E_NOTFOUND（章节不存在）；E_UNPROCESSABLE（空白段/结构节点/
      引用回查失败/readiness 阻塞——与整稿同闸）；E_STATE（窗口外）。
状态: MATRIX_READY+（与 draft_response_section 同窗口）
```

### 21-B. rewrite_section_segment（局部修订）

```
in : { project_id, segment_id("SEG-xxxx"——list 回显定位), content(该段修订后全文),
       expected_version?(CAS 对照：与段当前 version 不符 → E_CONFLICT 无部分写入),
       project_version? }
out: 同 21-A（changed/deduplicated/version+1/合稿统计）
语义: 只改目标段（version+1），兄弟段与顺序不动；合稿随之同步。
      同内容重写 = no-op（version 不推进）。
错误: E_NOTFOUND（段不存在）；E_CONFLICT（expected_version 过期——并发
      写作面，先 list 对账再改）；E_UNPROCESSABLE（空白/引用回查失败）。
状态: MATRIX_READY+（与 21-A 同窗口）
```

### 21-C. list_section_segments（只读对账）

```
in : { project_id, section_id, project_version? }
out: { section_id, segments:[{ id, seq, version, chars, idem_key,
       updated_at, preview(段首 40 字) }], assembled_chars, target_chars,
       remaining_chars, over_target, status, project_version, request_id }
语义: 段账本发现面——续写前/重启后的「已写到哪」对账（段随 StoreSnapshot
      持久化，重启保真；SEG-xxxx 入 id 水位防重签碰撞）。
错误: E_NOTFOUND（章节不存在）。
状态: 全窗口只读（MATRIX_READY+，含导出后追溯）
```

### 21-D. 整稿 ↔ 分段互斥（与 §6 draft_response_section 的联动）

`draft_response_section` 提交**不同内容**时，该章节段账本重置为单段
（内容 = 整稿全文——账本与正文永不对不上）；同内容重提（幂等 no-op）
不动账本。Skill 面（bid-response-pipeline §篇幅纪律）已改写：分批写作
必须走 21-A，`draft_response_section` 仅用于 ≤4000 字小节或整章重写。

## 22. get_star_coverage（#285 ① ★漏标检测 · 只读）
```
in : { project_id, file_id }
out: { file_id, file_name,
       star_total,                    ★字面总数（含样板句——与 fixtures manifest
                                       star_count 同口径，±2 对账面）
       declarations,                  样板句★数（「打★号条款为实质性条款」是声明
                                       不是条款——规则 c 排除；declaration_samples ≤5 条）
       star_truth,                    真值★数 = star_total - declarations
       star_captured,                 已捕获（位置对应 REQ 在场）
       star_missing,                  漏标 = star_truth - star_captured
       captured:[{req_id, star, page, line, block_idx, stars, locator, text}],
                                       req_id 优先取 star-marked REQ——「命中但未标★」
                                       一眼可见（star=false 即漏 params.star 的标记面缺口）
       missing:[{page, line, block_idx, stars, locator, text, reason}],
                                       补标定位面；表格行（cells 非空）reason 追加
                                       装配面提示
       captured_truncated, missing_truncated,   明细 ≤100 块（计数全量——nm-lib 实测
                                       638 处漏标，全量回显会吃爆上下文）
       warnings[], next }
口径: 三级恒等式不漏不重——star_total = declarations + star_truth
      = declarations + captured + missing；truth = captured + missing。
匹配: 双腿——① REQ.anchors[].locator == 块 locator；② anchor.excerpt 包含
      该行★内容前缀（★后第一个内容 token；裸「N ★」子串跨表撞车禁用此腿）
      且同页（页守卫）。★信号（params.star / params.substantive / excerpt 含★）
      决定 captured 明细的优先展示，不改变命中判定。
样板句: 行内含「打★号」「标★」「★号条款」「非★号」（NFKC 归一 + 剥引号后
      匹配）→ 声明句不算真值——含 pdftotext 跨行碎片（「数中非★号的条款…」）。
窗口: FILES_IMPORTED+ 含 Exported（与 get_coverage_report 同窗口——抽取前查 =
      全部真值未捕获的诚实口径，warnings 提示先 extract）。
明确不做: 不自动补标（只报告——补标由 Agent/人决策：submit_block_marks
      substantive=true 后重跑 extract_requirements）；不改抽取逻辑；无 UI。

## LT. list_tools（#279 行协议发现面 · 只读）
```
in : {}（无参数、无状态前置——建项目前即可调）
out: { count, tools: [{ name, summary }...],
       line_protocol: { request, response, notes[] } }
语义: 行协议握手入口——#269 六场实测两个独立代理都试探了 list_tools
      （Agent 先握手再干活的合理预期），行协议无此工具只能 E_NOTFOUND、
      信封结构全靠猜（danqing / hc-cluster 各 5 连败）。返回工具清单
      （单一真源 TOOL_NAMES + 摘要）+ 行协议契约摘要（信封形态/错误码面/
      CAS 纪律/断点恢复入口）。
备注: 标准 MCP JSON-RPC 客户端用 tools/list 方法等价获取（含 inputSchema
      全量）；本工具是 stdio 裸 JSON 行协议的发现面补口。
```

## 22. recommend_length_profile（#287 篇幅档位推荐 · 只读）
```
in : { project_id }
out: { recommended_profile("practical"|"standard"|"conservative"|"maximum"),
       profile_label, target_chars, approx_pages, confidence(high|medium|low),
       reasons:[…逐信号一行——命中与未命中都写，用户可核对],
       from_tender_spec: u32|null（招标文件明写篇幅——最高优先级信号）,
       alternatives:[{profile,label,chars,pages,when}×4 全档位],
       signals:{ scoring_detail_count, star_count, budget_yuan, tender_type,
                 explicit_limit_chars },
       next }
窗口: REQUIREMENTS_EXTRACTED+ 含 Exported（只读——信号来自 REQ 面，建纲
      前后均可查；条款集空 → E_STATE 先 extract）。
语义: P3 建纲**前**调——把建议 + 理由 + alternatives 呈现给用户确认后
      build_outline(length_profile=…)。**决策权在用户，Agent 只推荐**
      （推荐可被覆盖——length_profile 四档任选或 target_total_chars 直给）。
信号: ① 明写篇幅（锚文限值模式扫描——「不超过/最多/限制在…N 页/字」、
      「N 页/字 以内」；页 × 844 换算）→ 覆盖一切推断；
      ② 评分细则数（collect_scoring_rows 的 sub_items 总和——#252 预派生，
      无需先建纲）：< 10 实用型 / 10-19 标准型 / 20-39 保险型 / ≥ 40 极限型；
      ③ 修正项任一命中上调一档（饱和极限型）：★条款 > 30（params.star
      标记面——#285 star_scan 字面真值口径在其 PR 合并后接入，两口径
      档位结论等价）/ 预算 > 1000 万（amount_wan 最大值）/ 工程类
      （项目名 + 锚文关键词推断）；多标段无引擎面信号——Agent 显式选档。
置信度: 明写 → high；细则 ≥ 10 → high；1-9 → medium；0 → low
      （未识别评分表——按实用型兜底，用户务必确认）。
四档: Practical 400,000≈474页 / Standard 800,000≈948页 /
      Conservative 1,200,000≈1422页（默认——用户裁定 2026-11-06）/
      Maximum 2,500,000≈2963页（接近装订上限：一本胶装 ≈ 500 页，
      2000 页 4 册；3-7 天评 20-50 家——起草成本极高，是用户显式选择）。
门禁联动: E_TOTAL_CHARS_BELOW_MIN 按 outline_total_chars()（配额总和
      = 选中档位目标）动态判——档位化后自动按选中档生效（非固定 844,000）。
```
