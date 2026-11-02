# 标书业务 MCP 工具契约（Bid MCP Contract）

> 定位：MCP Server 是**薄壳**——工具只做参数校验、状态机裁决与转发，业务逻辑在后端服务层（参照 BidMaster services/mcp/server.py 的形态，但规避其 AGPL 实现）。每个工具均要求幂等；响应统一携带 `project_version`（乐观并发）。
> 通用响应包装：`{ ok, tool(回显), data, request_id, project_version?, warnings? }`；
> 错误：`{ ok:false, tool, error:{ code, what, hint?, item_errors? } }`（`what` 是人读错误
> 正文——#80 第三批外部 Agent 实测摩擦 #12：历史契约写 `message`，实现面是 `what`，
> 以实现为准修订本文；`tool` 是调用回显字段）。

## 0. 通用约定

- **错误码**：`E_AUTH` / `E_NOTFOUND` / `E_STATE`（状态机拒绝越级或非法转换）/ `E_CONFLICT`（project_version 过期）/ `E_PARTIAL`（部分条目失败，附 item_errors）/ `E_UNPROCESSABLE`（解析不可恢复）/ `E_RATE` / `E_INTERNAL`。
- **幂等策略**：写操作接受 `idempotency_key`；同一 key 重复调用返回首次结果（不重复执行）。文件按 `sha256` 天然幂等；条款抽取按 `dedupe_hash` 幂等合并。
- **大结果分页（S-4，V3-5）**：列表类工具（get_compliance_matrix / list_evidence / list_tasks，及 M2-b 的 search_evidence）不得一次返回全量（12,000 行矩阵 / 10,000 条证据量级）——统一游标信封：
  `in : { project_id, filters:{...}, cursor?, limit?, page?(仅矩阵兼容) }`；
  `out: { rows|items, total(过滤后总数), limit(生效页宽), cursor(本次生效游标), next_cursor: string|null }`，`next_cursor=null` 即取完，原样回传取下一页。
  游标规则（fail-closed）：opaque base64（内嵌版本字节/偏移/过滤器指纹）；跨过滤器或跨工具复用游标 → `E_UNPROCESSABLE`；limit 合法域 1..=1000、缺省 100，越界/非整数**显式拒**（不静默 clamp）；游标损坏/版本不认识/偏移越界（底层列表已变化）→ `E_UNPROCESSABLE`。行序确定性（矩阵=条款位置序 CMP-0001..N；证据=id 字典序；任务=target_id 字典序）是分页安全前提，不得破坏。
- **补充建议（第 9 个只读工具）**：`get_bid_project_state`——断点恢复的入口（读项目状态、各阶段完成度、DIRTY 章节、BLOCKER 计数），仿 LingYan `check_pipeline_state`。原 8 工具契约不受影响。V3-5（S-4）另增两个只读列表工具 `list_evidence` / `list_tasks`（§4-A/§4-B，工具面共 11）。issue #42 阶段 10 增第 12 个工具 `save_version`（AI 主动存版本快照，见文末 §SV）。

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
out: { file_ids:[TF-*], per_file:[{file_id, sha256, pages, tables, chunks, ocr_tier, parse_status, errors?}],
       totals:{files, chunks, parse_failed, partial},
       addendum_readiness:{ total, ready, not_ready:[{file_id, name,
         readiness(READY|FAILED|OCR_PENDING), waived, reason}] } }   ← #184 分级就绪报告
幂等: sha256 已存在 → no-op 返回原 file_id（deduplicated:true）
语义: 后端确定性管线——解析（表格单元格级、扫描件 OCR 分档）→ 结构化切块（chunk_id 稳定、登记 文件→位置 映射）→ 证据注册（切块即入 EV 台账——后续 register_evidence 签发的 EV-id 从已有计数后继，不是从 EV-0001 起——#80 第三批外部 Agent 实测摩擦 #7）→ 向量+精确双索引。**Agent 不做解析与切块，只核对返回统计并重试失败件（R-01）。**
警告: 补遗件非 READY 且未逐件确认 → warnings[] 携带
      「ADDENDUM_NOT_READY: 补遗 <名> 未就绪（<原因>）——补遗可能改控制价/
      截止时间/评分办法，请修复重传或人工确认后放行（record_addendum_waiver）」
      （#184——import 是 Agent 看到坏补遗的第一现场；已确认件不重复告警）
错误: E_PARTIAL（部分文件 parse FAILED；失败件内容全程禁引；data 携带成功面含
      addendum_readiness 块）；E_STATE（project 已过 IMPORTED 阶段追加新文件 → 允许，但标 dirty 波及）
状态: DRAFT → FILES_IMPORTED
```

## 3. extract_requirements
```
in : { project_id, scope:"all"|file_ids[], options:{granularity, include_scoring_table:true}, project_version }
     options.granularity：**保留位，当前未实现**（传入被忽略——#86 摩擦 #6 清偿：
     契约曾提到但无实现，如实标注；未来实现时枚举合法值并在此补记）
out: { counts:{total, by_category{qualification,veto,scoring,tech_param,commercial,format,other}},
       conflicts:[{a:REQ-*,b:REQ-*,reason}], missing_sequence:[{file_id,expected_clause_nos,found}], warnings[] }
执行: 确定性先行（条款编号序列解析、表格结构化槽位抽取）→ 模型补全语义归类与陈述规范化；按 dedupe_hash 幂等合并重跑结果
硬规则: 条款编号缺号 → missing_sequence 必须报告（完整性校验）；补遗与正文冲突 → 双向登记 conflicts
错误: E_PARTIAL（某文件抽取失败，其余成功）；E_STATE（未导入文件）
状态: FILES_IMPORTED → REQUIREMENTS_EXTRACTED
```

## 4. get_compliance_matrix（只读）
```
in : { project_id, filters:{category?, status?}, cursor?, limit?, page? }
     category: 契约键（qualification|veto|scoring|tech_param|commercial|format|other）或中文类别名；
     status  : NOT_RESPONDED|RESPONDED|PARTIAL|NOT_APPLICABLE（T-2 四态派生）；
     cursor  : opaque 游标（与 filters+project_id 绑定，优先于 page）；limit: 1..=1000（缺省 100）；
     page    : 兼容旧调用（映射 offset=(page-1)*limit；page≥1，0 → E_UNPROCESSABLE）
out: { rows:[{cmp_id, requirement:{id, clause_no, category, statement, mandatory, source_anchor, params},
              evidence:[{ev_id, title, status, valid_until}], gap, response_policy, draft_section_id}],
       total(过滤后总数), limit, cursor(本次生效), next_cursor: string|null,
       counts:{total, returned, page(信息性回显), page_size(=limit)} }
无状态副作用；任何阶段可调。行序 = 条款位置序（cmp_id CMP-0001..N，确定性稳定）；
先过滤后分页（total 恒为过滤后总数）。
错误: E_UNPROCESSABLE（游标与过滤器不匹配/游标损坏/limit 或 page 越界——显式拒不 clamp）
```

## 4-A. list_evidence（只读，S-4 新增）
```
in : { project_id, filters:{kind?, status?}, cursor?, limit? }
     kind    : qualification|performance|personnel|certificate|product_doc|solution_module|contract
               （或对应中文展示名——与矩阵 category 双形态口径一致）；
     status  : ACTIVE|EXPIRING|EXPIRED|SUPERSEDED（T-1 台账状态）
out: { items:[{id:EV-*, kind(snake_case 机器形态), title, status, subject_name, valid_until}],
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

## 5. search_evidence（知识库检索——双通道）
```
in : { project_id, query, mode:"hybrid"|"semantic"|"exact", exact?:{field:"model|amount|date|clause_no|cert", value},
       filters:{source_type?, tags?, valid_at?:开标日, min_valid_until?}, top_k, threshold? }
out: { hits:[{ev_id, chunk_id, content, source:{file, position}, score, match_type:"semantic|exact|both"}],
       trace:{semantic_k, exact_k, merged_by} }
硬规则: 资格类查询默认 exact 优先；valid_until < 开标日 的证据自动降权并附 warning=EVIDENCE_EXPIRING/EXPIRED；
      排序带确定性 tie-breaker（score, evidence_id）保证可重放（R-15）；
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
阻塞: draft 前置 readiness 闸（#110 任务 1 + L3；#184 升级为分级）：无关联 REQ/缺
      资质/业绩不足/技术参数无佐证/否决项未响应/补遗不就绪（MissingAddendum
      ——任一件非 READY 且无逐件确认即阻塞，**部分失败/零文本扫描件也算**，
      拒稿理由逐件点名文件名与原因）六种 Blocker → E_UNPROCESSABLE（人话
      理由，不泄漏枚举名）。人工出口分两支：无补遗件 = addendum_waiver 门
      （§12）；补遗在场但不就绪 = record_addendum_waiver 逐件确认（§16）。
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
         compliance: {status:"passed"} | {status:"failed", findings:[…]} } }
规则分层（R-07 规则分界；原引审计问题 Q7）:
  确定性（代码断言，先行、必跑）: 报价 vs 控制价/预算、工期 vs 招标要求、资质有效期 vs 开标日、
      页数/字数上限（Word 渲染口径）、必填格式项、签章位存在性、暗标禁字清单扫描、条款引用完整性
  模型（仅产 finding 不改稿）: 表述质量、逻辑连贯、偏离合理化、疑似过度承诺
categories 四桶（#187——「没做 ≠ 做对了」，issue #9 口径）:
  structural（已实现）: E_SECTION_EMPTY / E_CITATION_GHOST / W_PLACEHOLDER /
      W_CHARS_OFF_TARGET / E_SCOPE_UNKNOWN
  semantic（未实现）: M_ 前缀模型发现——现网未配置模型，恒
      not_performed{reason:"no model configured"}，不得因「没发现错误」伪装
      passed；接入真实 LLM 后经服务层注入缝（validate_document_with_reviewer）
      变为 passed/failed
  evidence（部分实现）: E_EVIDENCE_EXPIRED（效期面；深度复核后续 PR）
  compliance（部分实现）: E_VETO_UNRESPONDED / E_QUAL_UNMET /
      W_FORMAT_BASELINE（暗标禁字等后续阶段）
  ok 语义（#187 厘清）: ok=true 只意味着「已执行桶中 BLOCKER == 0」，不是
      「整本标书已通过审核」；not_performed 不把 ok 翻 false（能力缺席不是
      缺陷）。桶内 findings 与平面 findings 串接一致（桶序：structural →
      semantic → evidence → compliance，桶内规则触发序）。
状态: → VALIDATING → REVIEWING；BLOCKER 清零（或登记 WAIVED+理由）才可 EXPORT
      （#86 摩擦 #10 清偿：REVIEWING 是导出时自动补齐的瞬态——单次 export 内部
      经 Drafting→Validating→Reviewing→ReadyToExport→Exported 逐态补链，
      不会作为可观察的停泊状态出现在两次调用之间；人工审面是三门
      record_review_gate（§12），不是项目状态 REVIEWING。若未来需要人工终审
      停泊位，再启用为可观察状态。）
出参补充（#80 第三批外部 Agent 实测摩擦 #2）：validate 会推进项目版本（状态转换面），
响应携带 project_version = 推进后的新值——后续工具调用必须用新值，否则 E_CONFLICT。
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
       tables_written, table_kinds:[...], forms_written, form_titles:[...], warnings[] }
     ← #117/#122 导出面对账：六表/七件全部进 docx，逐张点名（tables_written
       == 6 且 table_kinds 逐类在座；forms_written == 7）。e2e 不信任工具
       返回值——解包 docx 数节验：`<w:tbl>`==6、七件关键词、表行数下限、
       红run 数、合计一致、零 markdown 星号（#129 深断言，12_export-check.json）。
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
状态: → READY_TO_EXPORT → EXPORTED
```

---

## 9. check_bid_quality（#68 阶段 2 新增——只读诊断）
```
in : { project_id }
out: { project_id, summary:{total, drafted, failed, pending, uncovered_req_total, out_of_band,
       unanchored_total}, needs_human:[SEC-*], sections:[{section_id, title, status, chars,
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
in : { project_id, target_total_chars?(1000..=5000000, 缺省 20000),
       max_reqs_per_leaf?(1..=50, 缺省 8), project_version }
out: { counts:{ sections, leaves, requirements, total_target_chars, matrix_rows, matrix_blocking },
       status: "MATRIX_READY", project_version }
语义: 推导大纲树（确定性——REQ 类目分组 + 每叶上限）+ 矩阵首建（推进
      MATRIX_READY——draft 的硬前置）。写后自动落盘（服务层 auto_checkpoint）。
状态: REQUIREMENTS_EXTRACTED 态可调（FilesImported/Draft → E_STATE；
      MATRIX_READY+ 重复调用 → E_STATE——重建面属「补遗后增量重抽」后续需求，
      不隐式放行）
前置: 条款面非空（空纲无意义——与 build_matrix 同口径）
幂等: 推导确定性（同输入同树）；状态机面不重入（见上）
```

## 11-A. register_evidence（#80 第三批 · G2）

```
in : { project_id, kind(qualification|performance|personnel|certificate|product_doc|
                        solution_module|contract 或中文展示名),
       title, source_file, source_sha256(必填——无文件身份不入账),
       subject_name, scope?, valid_from?, valid_until?(ISO 日期),
       fingerprint?(算法前缀:值——缺省 sha256:<source_sha256>),
       locator?({ kind: docx_paragraph|pdf_page_text|markdown_block, ...指纹字段 }),
       project_version }
out: { ev_id: "EV-*", deduplicated, kind(snake_case), status, hint, project_version }
语义: 语义位查重在域层（同 kind+title+subject+scope 最新非 SUPERSEDED 记录：
      同指纹 → 幂等重放回原 id；异指纹 → 旧记录单向降级 SUPERSEDED + 签发新 EV）。
      locator 缺省 = MarkdownBlock{ block_id: "mcp-registered:<source_file>" }——
      诚实标记「经 MCP 登记面、无引擎定位器」（审计可辨识，不冒充引擎抽取）。
状态: MATRIX_READY+（P4 面——矩阵就绪后登记）
错误: E_UNPROCESSABLE（kind/日期/fingerprint 格式/locator 形态非法；source_sha256 缺失）
```

## 11-B. bind_evidence（#80 第三批 · G2）

```
in : { project_id, ev_id, requirement_ids:[REQ-*](非空), bid_open_date, project_version }
out: { ev_id, bound_new, replayed, requested, matrix:{ rows, blocking },
       status, project_version }
bid_open_date 语义（#80 第三批外部 Agent 实测摩擦 #5）：证据有效性的裁决基准日
       ——工程口径取【开标日】（评标时刻的有效性，不是 create 的 bid_deadline
       递交截止日；两者通常相近但语义不同。无独立开标日真源时，用招标文件
       载明的开标日期）。
语义: 逐条绑定（REQ 不存在 → 逐件 E_NOTFOUND；EXPIRED/SUPERSEDED 禁绑 → 逐件
      E_UNPROCESSABLE——R-03，开标日视角即时裁决；重复绑定 → 幂等重放）+
      绑后矩阵重算（行内证据即时可见——不重算会让 get_compliance_matrix 看到
      陈旧行）。部分失败 → E_PARTIAL（item_errors 逐件定位 + data 携带成功面
      ——与 import_tender_files 同款「错误码不吞成功面」）。
状态: MATRIX_READY+
```

## 12. record_review_gate（#80 第三批 · G3；#131 增第四门；#182 receipt 验签）

```
in : { project_id, gate(matrix_approval|outline_approval|pre_export_confirm|addendum_waiver),
       decision(APPROVED|REJECTED),
       receipt(#182——宿主签发的审批回执：v1.<actor>.human.<scope>.<issued_at>.<hmac>，
               scope=review_gate:{gate} 门级绑定；旧 reviewer 参数显式拒——Agent
               不得自述身份，防「以为过了人工门」的假成功),
       findings?:[{ severity(BLOCKER|WARNING|INFO), code, message, section_id? }],
       project_version }
out: { receipt:{ review_id, gate, decision, reviewer(legacy 回显——human:{actor_id} 派生),
       finding_ids, spawned_rt_ids,
       reviewed_content_digest(#180——本次记审绑定的章节树内容摘要；幂等重放时为既有记录的摘要) },
       actor:{ actor_id, actor_type:"human", authentication_method:"hmac_receipt_v1" }
       (#182 可信身份回显), gates_approved:[...], status, project_version }
语义: #182 复审人门升级为 receipt 验签（服务层 HMAC-SHA256 是 MCP 面唯一入口
      ——宿主持钥签发、24h 时效窗、scope 门级绑定、常量时间比对；域层再拦
      actor_type≠Human 的纵深——borrow mdcore ReviewPolicy：AI 操作者不得
      自审自过）、状态门
      （窗口外拒收——矩阵/大纲门 VALIDATING+ 不再受理）、门序（矩阵先于大纲；
      pre_export 要求前两门 Approved）全在域层 record_review 裁决——壳层零重实现。
      gates_approved 与域层 gate_approved 同一真源。
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

```
in : { project_id, section_id, project_version? }
out: { project_id, section_id, section_title, chars, estimated, max,
       covered_items:[REQ-*], missing_items:[REQ-*], rationale（逐项可追溯）,
       item_scores:[{item_id,estimated,max,tier,coverage,is_objective,rationale}], request_id }
语义: B2 SectionScorer 只调用不改（四维规则判定——覆盖度/字数/证据/具体性）。
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

```
in : { project_id, project_version?,
       bid_price?（缺省 = 不计价——「没报价就不算」，price 出 null 而非 0）,
       others?:[$], method?/coefficient?/max_score?/budget?/deviation?（价格口径，
       仅 bid_price 在场时消费——参数面同 §13 score_price） }
out: { project_id, technical, price（null | 数值）, commercial（null | 数值）, total,
       target（诚实满分 = 非硬门可达叶 max 合计——含价格项与未映射项，不因没写缩分母）,
       gap = target − total, by_item:[{item_id,criteria,max,estimated,tier,coverage,section_id}],
       breakdown:[SectionScore…（逐章节）], notes[]（UNMAPPED/SECTION_MISSING/NO_PRICE_INPUT
       诚实登记）, price_input（价格口径 echo | null）, sections_scored, request_id }
语义: B2 score_document 三段聚合（技术 = 主观叶估分和；商务 = 客观叶；价格 = B1
      实算）+ by_item 全量逐项（未覆盖 estimated=0 如实呈现）。e2e 十一件套的
      11_self-score.json 即本工具产物（scripts/e2e_bid/run_e2e_artifacts.sh）。
状态: MATRIX_READY+（只读诊断）
错误: E_UNPROCESSABLE（价格口径非法）；E_STATE（无章节树）
```

## 16. record_addendum_waiver（#184 补遗分级阻塞的逐件人工确认）

> 背景：#184 前旧判定「3 份补遗坏 1 份不拦工作流」（只有**全部**失败才阻塞），
> 且 OcrRequired 一律放行——零文本扫描件被当「没有内容」处理。翻案理由：
> **不知道 ≠ 没有**——坏的那份可能正是改控制价/截止时间的，按旧条款做标是
> 不可逆的废标；工作流被拦是可修复的摩擦（修复重传 / 逐件确认），且「放行」
> 本身也要变成留底的可审计行为，而非静默赌概率。

```
in : { project_id, file_id(TF-*——须真实存在，import 响应/拒稿理由中的
       addendum_readiness.not_ready[].file_id),
       reason(非空白——不可审计的确认不成确认),
       receipt(#182——宿主签发的审批回执，scope=addendum_waiver:{file_id} 件级
       绑定——TF-0003 的 receipt 免不了 TF-0004；旧 principal 参数显式拒),
       project_version }
out: { recorded:{ file_id, reason, at(unix epoch 秒),
       principal(legacy 回显——human:{actor_id} 派生),
       actor:{ actor_id, actor_type:"human", authentication_method:"hmac_receipt_v1" } },
       addendum_waivers:[{file_id, reason, at, principal}…全部确认记录——
       追加式审计日志：同件重复确认追加不覆盖，解除不走本面],
       addendum_readiness:{ total, ready, not_ready:[{file_id, name, readiness,
       waived, reason}]（已确认件仍在清单但 waived=true——审计面不因放行
       而消失）}, status, project_version }
语义: #184 分级就绪的伴生闸：任一补遗不处 READY（FAILED / OCR_PENDING）时
      draft_response_section 被 MissingAddendum 拒（§6，逐件点名）；本工具
      对具体 file_id 记一条人工确认 → 该件不再构成阻塞。非全局布尔——
      「3 份坏 1 份」要确认到具体哪一份、为什么故意带病放行。写账即
      content_version +1；导入可解析版本后阻塞自然消失，确认记录无害残留
      （与 §12 addendum_waiver 门同款语义）。
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
