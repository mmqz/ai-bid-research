# 生产调用链审计（callgraph-audit.md）

> 审计员：整合员（链条与已知问题复核）。基线 6105beb（只读，未修改任何仓库文件）。
> 方法：六条业务链从 MCP 工具入口到持久化**逐跳**核对源码（函数名+文件:行 均经本轮 Read 复核，非转抄 agent 报告）；每跳标注守卫与已知断点（引用 A-x/B-x/C-x/D-x 编号，见 module-audit-A/B/C/D.md）。
> 通用入口：`bid-mcp/src/tools.rs:30 invoke()`（25 工具分发表 :31-76）→ 工具函数（参数校验 E_UNPROCESSABLE）→ `route_guard`（tools.rs:3587-3607 = 项目路由 E_NOTFOUND + `guard_tool` E_STATE + `check_version` E_CONFLICT）→ `BidService` 服务层（业务+`auto_checkpoint` service.rs:700-704）→ `DomainStore` 域层 → `store_io`（service.rs:693-698 → store_io.rs:109-121 tmp+rename）。
> 裁定口径：**PASS** = 有真实数据流动且无已知断点；**BROKEN(断点编号)** = 链上存在使业务语义失真/不可恢复的断点；**DEGRADED** = 主链通但存在系统性降级面。

---

## 链 1：项目生命周期链

`create_bid_project → import_tender_files → extract_requirements → build_outline → draft_response_section → validate_bid_document → record_review_gate → export_bid_document`

### 逐跳调用链

| # | 跳 | 调用链（函数@文件:行） | 守卫 | 已知断点 |
|---|---|---|---|---|
| 1 | MCP 入口 | `invoke`@tools.rs:30 → `create_bid_project`@tools.rs:271 | 参数校验（name/code/mode/external_key/deadline）；幂等重放（external_key 命中 → deduplicated:true） | — |
| 2 | 建项落库 | → `BidService::create_project`@service.rs:768-778 → `DomainStore::create_project`（域层） → `auto_checkpoint`@service.rs:700 | — | — |
| 3 | 导入 | `import_tender_files`@tools.rs:370-596：围栏 `resolve_read`@tools.rs:413（fs_fence 双层）→ `guard_tool`@tools.rs:439 → `check_version`@tools.rs:443 → `BidService::import`@service.rs:1083-1137（roots/件数 64/单件 500MB/累计 2GB 门 :1091-1125）→ `import_files`@service.rs:276-530：`sha256_file` 幂等（:288-357，#188 角色对账）→ 引擎路由 :366-379（docx/xlsx→bid-engine；pdf→引擎优先失败回退 `parse_file`）→ `upsert_file`@lib.rs:621-666（**失败件不推进状态 #174** :660-662）→ `auto_checkpoint`@service.rs:1135 | 围栏/限额/幂等/E_PARTIAL 逐件失败 | B-09/B-10（OCR 截断/1 页回退）；B-12（pdfium 通道无乱码仲裁）；B-13（Ok-0-块无 per-file 告警）；B-28（混合 PDF 丢扫描页） |
| 4 | 抽取 | `extract_requirements`@tools.rs:650-885：`guard_tool`@tools.rs:676-678 → `extract_requirements_with_kind`@extract.rs:75-146（#196 kind 分发 :100-116；#226 层 2 否决排除统一后置 :127-144）→ **合并面**：faces 组装@tools.rs:724-741（filter `has_extraction_face`@merge.rs:71-76）→ `merge_extraction`@merge.rs:100-220 → 缺号检测@tools.rs:746-757 → 补遗冲突检测@tools.rs:759-798 → `replace_requirements`@service.rs:810-814（整集替换，状态 FILES_IMPORTED→REQUIREMENTS_EXTRACTED）→ `auto_checkpoint` | 状态门；parsed 非空门 :684-692；ZERO_REQUIREMENTS 响亮告警 :857-864 | **B-02**（merge 面按文件名前缀过滤——真实文件名 Agent 标注整体失效）；**B-01**（requirement 标记未提交 → 正则丢弃、REQ-A 不生成 → 条款黑洞）；**B-08**（XlsxRow 黑洞）；**#244**（UNCOVERED_BLOCKS 仅 warning :836-842）；CONFLICTS 仅 warning :821-826（见链 4） |
| 5 | 建纲 | `build_outline`@tools.rs:3618-3688：`route_guard`@tools.rs:3639（guard 仅 REQUIREMENTS_EXTRACTED 态，lib.rs:179）→ 前置**唯一**检查：条款非空@tools.rs:3642-3653 → `prepare_outline_and_matrix`@service.rs:822-830：`derive_and_set_outline`（outline.rs:724）→ `build_matrix`@matrix.rs:218-252（首建推进 MATRIX_READY :246-250）→ `auto_checkpoint` | 状态门；空条款拒 | **#244**（不消费 coverage_complete/merge_stats——见链 2）；**A-09 相邻**（建纲后 REQ 变更无重导通道：set_outline 状态门 outline.rs:666-677 只许 REQUIREMENTS_EXTRACTED/MATRIX_READY/EVIDENCE_BOUND） |
| 6 | 起草 | `draft_response_section`@tools.rs:1845-2091：`guard_tool`@tools.rs:1895（MatrixReady/EvidenceBound/Drafting，lib.rs:192/211）→ 幂等键 :1914-1919 → `BidService::draft_section`@draft.rs:589-708：`build_section_input`@draft.rs:112 → readiness 闸 `check_draft_readiness`@draft.rs:614（readiness.rs：NoRequirements/MissingQualification/MissingPerformance/MissingTechEvidence/UncoveredVetoItem/**UncoveredMandatoryItem**/MissingAddendum/UnreadyInput 八阻塞）→ `input_integrity`@draft.rs:637（受限起草标注）→ 生成器 :652 → 引用回查 :656-670 → CAS `set_section_content`@outline.rs:742-769 → `advance_section_to_drafted`@draft.rs:762 → `advance_project_to_drafting`@draft.rs:786（MatrixReady→EvidenceBound→Drafting）→ 章节工作区 commit :697-707 → `checkpoint` | 状态门；readiness 六+二阻塞源；幂等 CAS | **A-14**（`check_evidence_bindable`@draft.rs:199-209 只看 status，tier 不进起草门）；P0-1 相邻（误调 validate 后本跳永久 E_STATE） |
| 7 | 校验 | `validate_bid_document`@tools.rs:3469-3586：`route_guard` → `validate_document_with_reviewer`@service.rs:1372-1389：`validate::validate_document`@validate.rs:156-421（规则 1-8 四桶）→ **无条件推进 Drafting→Validating**@service.rs:1379-1386 → `auto_checkpoint` | 状态门（Drafting+ lib.rs:212/229） | **P0-1（#243）**：:1377 产出 report 后 :1379-1386 不查结果即推进；Validating 态 allowed_tools 无 draft（lib.rs:224-239）→ 状态只进不退（lib.rs:601-617）→ 起草期误调即永久卡死；A-11（validate 报告与状态推进双口径分裂） |
| 8 | 审核 | `record_review_gate`@tools.rs:4011-4071：`route_guard`@tools.rs:4028；reviewer 参数显式拒 :4017-4023 → `BidService::record_review_gate`@service.rs:996-1009：`verify_approval_receipt`@service.rs:1049-1067 → `principal.rs:223-289`（六段解析/Human-only/24h 时效/scope 进 MAC/HMAC 常量时间比对）→ `DomainStore::record_review`@review.rs:274-469：状态门+门序 :300-353（矩阵门 VALIDATING+ 拒；大纲门需矩阵门先过；导出门需前两门）→ 拒审依据门 :364-371（Rejected 必带 Blocker）→ digest 快照 :376-377 → 幂等分支 :384-414 → BLOCKER 派生 RT :417-444 → `auto_checkpoint` | 验签；门序；身份纵深（域层再拦非 Human review.rs:287-295） | **A-03**（receipt 消息 principal.rs:276 无内容摘要——24h 窗内可对未审内容过门）；**A-23**（RT 生命周期 assign/advance/complete@review.rs:494-583 域层完备，MCP 面零工具——带 Blocker 即永久锁导出）；**A-10**（digest 只覆盖六字段 outline.rs:621-640） |
| 9 | 导出 | `export_bid_document`@tools.rs:3240-3418：format/路径词法校验 :3284-3311 → `guard_tool`@tools.rs:3323（Drafting+ 与 Exported）→ 幂等键 :3346-3357 → `BidService::export_document`@service.rs:1161-1348：**门 1a** `check_export_ready`@service.rs:1179 → matrix.rs:334-343 → `export_blockers`@matrix.rs:320-331（读**存量** `self.matrix`）→ **门 1b** `export_allowed`@service.rs:1181 → review.rs:601-643（三门遍历 :604-611 + digest 比对 :612-630 + RT 全 DONE :631-641）→ **门 1c** `writing_acceptance_findings`@service.rs:1188-1207 → validate.rs:502-530（E_SECTION_EMPTY/E_CHARS_BELOW_MIN，#242）→ format-spec@service.rs:1393-1422 → bidder-profile :1227-1247 → `resolve_write` 围栏 :1270 → `DocumentExporter::export_with`@export.rs:1045+（manifest 幂等 :1126-1128 + tmp+rename 原子）→ `advance_project_to_exported`@service.rs:1438-1458 → `auto_checkpoint` | 三道门；围栏；manifest 幂等 | **A-01**（门 1a 消费存量矩阵：build_matrix@matrix.rs:232-233 用**全量**绑定 vs matrix_view@matrix.rs:347-356 过滤 Active/Expiring——导出门比工具面更宽松）；**A-02**（apply_validity@evidence.rs:575 生产零调用——时间过期证据恒 Active）；**A-06/A-07**（金额冲突/负偏离零消费）；**C-07**（幂等键漏 bidder_name）；A-11（跳级推进） |

### 链路完整性裁定：**BROKEN（P0-1/#243、A-23、A-01+A-02）**

数据真实流动（无孤岛：每一跳都写库+checkpoint），但：
1. **第 7 跳是单向门**（P0-1）：validate 无条件推进 + 无回退 + 无恢复工具 = 起草期误调即整单报废（唯一恢复=手改 db.json）；
2. **第 8 跳无出口**（A-23）：正常使用「拒审」语义（Rejected 必带 Blocker，review.rs:365-371）即永久锁死导出；
3. **第 9 跳门 1a 读旧账**（A-01/A-02）：失效/过期证据的响应可过导出闸门（P0-2 在导出面的残留）。

---

## 链 2：全块覆盖链

`导入 → get_document_chunks → submit_block_marks → merge → extract_requirements → build_outline`

### 逐跳调用链

| # | 跳 | 调用链 | 守卫 | 已知断点 |
|---|---|---|---|---|
| 1 | 导入 | 同链 1 跳 3（blocks 落 `svc.parsed`） | 同链 1 | 同链 1 |
| 2 | 取块 | `invoke`@tools.rs:66 → `get_document_chunks`@coverage_tools.rs:55-114：`route_project`@coverage_tools.rs:67（**仅项目路由，无 guard**）→ `find_parsed`@coverage_tools.rs:28-49（sha256 join）→ 分页 size≤200 :61-65 → 块+verdict 回显 :84-96 | size 上限；from 越界拒 :72-76 | **C-16**（契约 §17 承诺 E_STATE 窗口，实现无门——任何状态可调） |
| 3 | 标记 | `submit_block_marks`@coverage_tools.rs:139-230：`route_guard`@:194（FilesImported/RequirementsExtracted）→ marks 非空 :151-155 → `parse_verdict`@:116-128 → `validate_marks`@coverage.rs:158-181（区间/越界/重叠）→ `validate_submitted`@coverage.rs:192-218 → `set_block_coverage`@service.rs:795-805（marks+submitted 原子写）→ `auto_checkpoint` | 区间校验四件套 | **B-01**（`validate_submitted` 只查「submitted 落在 requirement 区间」（coverage.rs:208-215），**不查反向**——requirement 区间无 submitted 也放行） |
| 4 | 合并 | `extract_requirements`@tools.rs:724-742：faces = parsed.filter(`has_extraction_face`@merge.rs:71-76 = **A-/C-/B1/B2 文件名前缀**) → `merge_extraction`@merge.rs:100-220：② 正则 REQ 丢弃 :133-148（**与 verdict 无关**）③ REQ-A 生成 :151-189（仅来自 submitted）④ 覆盖统计 :192-204（`uncovered_blocks`/`coverage_complete`——**纯统计**） | 无标记严格 no-op（GT 红线 merge.rs:16-18） | **B-02**（真实文件名「光纤租赁费招标文件.pdf」的 marks/submitted 永不进合并——#196 修了抽取分发却没修合并面）；**B-01**（Agent 标 requirement 未随附 submitted → 该块正则 REQ 被丢 + 无 REQ-A → **条款无告警地从矩阵消失**；端态被 merge.rs:304-349 `covered_block_regex_dropped_any_verdict` 钉为预期）；B-19（REQ-A 序号按运行序重排，id 漂移） |
| 5 | 落库 | `replace_requirements`@tools.rs:805（整集替换）→ UNCOVERED_BLOCKS **warning**@tools.rs:836-842（"正则已兜底补漏——…补标，重跑 extract 更准"） | ZERO_REQUIREMENTS 全局兜底 :857-864 | **#244**（覆盖不足不阻断、不进任何门） |
| 6 | 建纲 | `build_outline`@tools.rs:3618-3688：前置**唯一**检查 `n_reqs == 0`@:3642-3653 → `prepare_outline_and_matrix`@service.rs:822-830 | 空条款拒 | **#244**（不消费 `coverage_complete`/`uncovered_blocks`；lib.rs:182 注释承诺「覆盖不足时 build_outline 应告警」——**实现无此告警，文档-实现背离**）；`get_coverage_report`@coverage_tools.rs:233-285 的 next 文案（:277-283）纯建议 |

### 链路完整性裁定：**BROKEN（#244、B-01、B-02）**

覆盖闭环的「度量面」真实存在（块 idx、区间标记、覆盖统计、覆盖报告工具），但**闭环的「裁决面」为零**：
1. **#244（基线成立）**：merge_stats 是纯返回值——`uncovered_blocks>0` 只产 warning（tools.rs:836-842），build_outline 的唯一前置是条款非空（tools.rs:3642-3653），遗漏待审块对建纲/矩阵/导出零阻断；
2. **B-01** 把黑洞钉为预期：Agent 标 requirement 的块即使零提交，其正则 REQ 也被丢弃（merge.rs:133-148），validate_submitted 的单向校验（coverage.rs:192-218）使这一形态静默通过；
3. **B-02** 使真实文件名世界的 Agent 标注**整体**不进合并面（tools.rs:727 filter）。

---

## 链 3：证据链

`import → register_evidence_verified → bind_evidence → derive_matrix_rows → draft → validate → export`

### 逐跳调用链

| # | 跳 | 调用链 | 守卫 | 已知断点 |
|---|---|---|---|---|
| 1 | 导入 | 同链 1 跳 3 | 同链 1 | 同链 1 |
| 2 | 登记 | `register_evidence`@tools.rs:3784-3887：`parse_locator_arg`@tools.rs:3699 → `route_guard`（guard 仅 MatrixReady/EvidenceBound，lib.rs:195-196）→ `register_evidence_verified`@service.rs:866-954：TenderFile 在场 :870-882 → Failed 禁引（R-01）:883-891 → parsed 对账 :892-902 → `locate_content`@service.rs:157 :907-910 → **tier 裁决** :918-931（带 locator 对账通过 → CONTENT_MATCHES+excerpt 服务端截取；无 locator → FILE_EXISTS，fingerprint=`sha256:<库内文件sha>` :925-930）→ `store.register_evidence`@evidence.rs:485-568（语义位查重 :494-504；same_payload=指纹+双日期 :500-502；换版旧件 Superseded :506-538）→ `auto_checkpoint` | #185 来源验证五步；#217 四态裁决；excerpt/tier 不可拆校验 evidence.rs:402-414 | **C-23**（语义属性 kind/title/validity/fields 全调用方自报——CONTENT_MATCHES 只证「文本在文件里」不证「是所称证据」）；**A-12**（same_payload 不含 tier/locator/excerpt——FILE_EXISTS 无法升级 CONTENT_MATCHES，重登记被 no-op 吞）；**C-05**（guard 窗口窄于契约「MATRIX_READY+」——起草后补证死） |
| 3 | 绑定 | `bind_evidence`@tools.rs:3888-3936 → `bind_evidence_batch`@service.rs:961-987：循环 `store.bind_evidence`@evidence.rs:608-659：开标日即时翻转 :625-634（绑定时刻唯一翻转点）→ **`can_bind` 只查 status** :635-643（Active/Expiring 放行）→ 幂等重放 :644-650 → 落绑定 :651-657 → 每轮 `build_matrix` 重算 → `auto_checkpoint` | R-03 禁绑（M2 变异 KILLED：evidence.rs:845 `gt13_expired_evidence_forbidden_to_bind`） | **P0-2（#245）**：:635-643 不查 verification_tier、不查内容匹配——**FILE_EXISTS 占位证据（甚至指向招标文件本身）可绑资格条件**；**A-24**（batch 循环内先落绑定，build_matrix/checkpoint 失败即整批错返而绑定已生效） |
| 4 | 矩阵派生 | `bind_evidence_batch` → `build_matrix`@matrix.rs:218-252 → `derive_matrix_rows`@matrix.rs:134-198：`ev_by_req` 只按绑定建索引 :141-144 → 行状态 `has_ev/has_sec` :168-179 → blocking=(is_mandatory∨agent_mandatory)∧NotResponded :186-187（#229 接线 matrix.rs:115-118） | 派生幂等（changed→version+1 :237-243）；override 优先 :171-183 | **P0-2**：:168 `evs` 只看**绑定存在**——不消费 verification_tier 与证据内容匹配；**A-14**：全下游（matrix/draft/readiness/validate/export）零 tier 消费者 |
| 5 | 起草 | 同链 1 跳 6：`check_evidence_bindable`@draft.rs:199-209（只看 status）→ `req_has_usable_evidence`@readiness.rs:712-717（绑定+status）→ EvBrief 进 prompt | readiness 阻塞源在位 | **A-14**（FILE_EXISTS 占位证据放行 MissingTechEvidence 起草门） |
| 6 | 校验 | 同链 1 跳 7：规则 8 `E_EVIDENCE_EXPIRED`@validate.rs:327-343（拦 status 已是 Expired/Superseded 的绑定） | 四桶 BLOCKER | **A-02**（时间上已过期而 status 仍 Active → 规则 8 拦不住——`apply_validity`@evidence.rs:575-602 全仓生产零调用，唯一翻转点是 bind 时刻 :625-634） |
| 7 | 导出 | 同链 1 跳 9：门 1a 读存量矩阵 | 三道门 | **A-01**（build_matrix :232-233 用全量绑定——换版后死绑定仍计 has_ev；三道门均不复查证据状态与规则 8；matrix_view :347-356 的活证据过滤只有 get_compliance_matrix 工具面消费） |

### 链路完整性裁定：**BROKEN（P0-2/#245、A-01、A-02、A-14）**

「证据」数据流真实落库并驱动矩阵，但**证据的「有效性」语义在链上三次脱钩**：
1. 绑定门只看状态不看层级/内容（P0-2，evidence.rs:635-643 + matrix.rs:168/174-179）；
2. 时间翻转无生产入口（A-02）；
3. 导出门消费构建时点的存量矩阵而非同源活算视图（A-01）。
四态+tier 的防线全部停在「入账时防伪」半边（service.rs:918-931），下游零消费（A-14）。

---

## 链 4：补遗影响链

`补遗登记 → 冲突 → 脏章节 → 审核失效 → 重起草`

### 逐跳调用链

| # | 跳 | 调用链 | 守卫 | 已知断点 |
|---|---|---|---|---|
| 1 | 登记 | `import_tender_files`（C- 件）@tools.rs:370（guard：Draft/FilesImported/RequirementsExtracted，lib.rs:155-186）→ DIRTY_APPEND **warning**@tools.rs:502-509 → 补遗就绪度阻塞：`missing_addendum`@readiness.rs:1050-1104（NotProvided/ParseFailed/OcrPending 三形态，#184 逐件分级）拦 `draft_response_section`（draft.rs:614-619） | L3 阻塞在位（readiness 消费 `gate_approved(AddendumWaiver)` :1068-1071） | **C-05**（MATRIX_READY+ 不可再 import——Skill 恢复协议「补遗从 P1 补」在起草后不可走，SKILL.md:89）；**A-04**（`record_addendum_waiver`@lib.rs:695-767 对**任意 file_id** 无条件追加 `ReviewRecord{gate:AddendumWaiver, decision:Approved}` :751-762 → readiness.rs:1069 的全局门被无关件 waiver 顺手打开） |
| 2 | 冲突 | 重跑 `extract_requirements` → 冲突检测@tools.rs:759-798：主文件（A-）×补遗（C-）同条款号异文 → `conflicts` 数组 + **CONFLICTS warning**@tools.rs:821-826 | — | **断点：冲突产物零消费**——`conflicts` 只进响应 JSON，全仓无任何门/状态/dirty 消费（B 组 D 组均未发现消费者；本轮 grep 证实）。补遗改了控制价，主文件旧条款照常响应 |
| 3 | 脏章节 | `replace_requirements`（lib.rs 整集替换）→ **不碰 `sections[].dirty`** | — | **A-09**：`dirty=true` 全仓唯一置位点 = `reopen_section`@outline.rs:796-814（:809）；`set_requirements`/`replace_requirements`/`register_evidence`（换版）均不传播——影响面引擎 impact.rs 全模块孤岛（grep 无生产消费者）；且大纲无法重建：`set_outline` 状态门@outline.rs:666-677 只许 REQUIREMENTS_EXTRACTED/MATRIX_READY/EVIDENCE_BOUND，`build_outline` guard 仅 REQUIREMENTS_EXTRACTED（lib.rs:179）——补遗后走到的状态建不了新纲 |
| 4 | 审核失效 | （应当发生：既有 PreExportConfirm 审核因条款面变更而失效）→ 实际：`export_allowed`@review.rs:601-643 只比对 `sections_digest`@outline.rs:621-640 | digest 比对在位（#180） | **A-10**：digest 字段清单只有 id/parent/title/level/order/content 六字段（outline.rs:604-610 显式排除 requirement_ids 等）——`replace_requirements` 后 digest 不变 → **审核不失效**，export_allowed 照旧放行；矩阵覆写/评分映射变更同理 |
| 5 | 重起草 | （应当发生：脏章节重新起草）→ 实际：`draft_response_section` guard | lib.rs:206-223 Drafting 态可 draft | **P0-1**：若已误调 validate（Validating+）→ draft 永久 E_STATE（lib.rs:224-239）；即使仍在 Drafting：REQ 已整集替换（B-19 id 漂移），章节 `requirement_ids` 引用旧 id → 幽灵条款仅 warning（readiness.rs:653-655），无重建通道；`reopen_section`/`advance_section_status` 无 MCP 工具（A-23 同族） |

### 链路完整性裁定：**BROKEN（A-09、A-10、A-04、P0-1/C-05）**

五跳中只有第 1 跳的「登记+阻塞」真实生效；**冲突→脏→失效→重起草四跳全部断线**：冲突零消费、dirty 零传播、审核失效面（digest）不覆盖条款、重起草被状态机+无工具双重锁死。domain-model §2 的 R-date/DIRTY/R-05 语义在生产为零（A-09/A-02 同族）。

---

## 链 5：评分链

`评分规则抽取 → ScoringItem → ScoringMapping → 章节贡献 → UNMAPPED`

### 逐跳调用链

| # | 跳 | 调用链 | 守卫 | 已知断点 |
|---|---|---|---|---|
| 1 | 规则抽取 | import（kind=评分办法 件）→ `extract_requirements_with_kind`@extract.rs:75-146（kind 分发 :100-116，#196/#199）→ 评分细则行 `re_score_line`@extract.rs:246-247 / 评分算法行 :248-249 → 审查表抽取（review_table.rs：Detailed→评分项 :791；`score_rules` 入 params :1086-1092；`section_title` 入 params :801-812）→ #226 层 2 否决排除统一后置 :127-144（`_veto_excluded` 可审计） | kind 值域；词表判据 | B-06（DOCX/PDF 否决词表两套不同源——L2 判据面跨格式不对称）；B-07（B1/B2 分支 kind 判据在生产不可达）；B-04（tables 恒 0） |
| 2 | ScoringItem | `derive_scoring_items`@scoring.rs:372-490：weight/max_score ← params.score :376-380；score_rules :386-388；派生 mappings（params.section → share=1.0 :449） | 派生幂等 | —（分值缺失入 gaps，诚实告警） |
| 3 | ScoringMapping | `upsert_scoring_mapping`@scoring.rs:1347-1457：单条 ∈(0,1] :1357-1361 → **聚合不变量**（联结后有效份额 ≤1.0+ε，派生同节去重、更新排除被替换行）:1387-1417 → 幂等键 (req,section) :1422-1441 → **content_version 真变更才 bump** :1435-1439/:1452-1455；restore 侧行约束+复合键+幽灵引用+聚合 @lib.rs:1263-1361 | #224 step1 三 blocker 闭合；scoring_mapping.rs 9 测试（:437/:486/:519/:571/:630/:657/:686/:716/:794） | **MCP 写面不存在**：`upsert_scoring_mapping` 全仓调用方仅 tests（本轮 grep 证实）——账本映射只能靠派生 params.section 或测试/嵌入方直连 store_mut；**A-22**（无删除/撤销面——错建映射只能降 share）；**A-18**（is_price_item 词面误伤 scoring.rs:971-974） |
| 4 | 章节贡献 | `score_section`@tools.rs:2403-2472 / `score_document`@tools.rs:2473-2753（route_guard，MatrixReady+）→ `build_scorer`@tools.rs:2853-2982：leaf_of 反查 :2864-2876 → #170 章引用联结（params.section_title → chapter_leaf）:2877-2931 → 账本映射联结（同节去重、生效归因 ledger_effective）:2942-2976 → `SectionScorer::score_section`@scoring.rs:1029-1112（#224 摊分：max=max_full×share :1056-1059）→ `score_document`@scoring.rs:1122-1230（技术/商务/价格三段聚合 :1127-1143） | 只读诊断（不落库） | 自评是关键词覆盖启发（rule_heuristic）——有 is_objective/notes 诚实标注，未冒充评标（可接受设计） |
| 5 | UNMAPPED | `unmapped_items`@scoring.rs:1232-1242（可达叶、非硬门、非价格、mappings 空）→ `score_document` :1145-1156：结构化 `unmapped_scoring_items` + notes 响亮告警（"UNMAPPED: N 个评分项未映射章节…总分可能偏低"） | #224 结构化面 | —（测试 scoring_mapping.rs:235 `unmapped_items_structured_and_loud`） |

### 链路完整性裁定：**DEGRADED（#224 step1 在位；MCP 写面缺失 + A-18/A-22/A-10 降级面）**

主链数据真实流动（抽取→派生→联结→自评→UNMAPPED 全程可跑、#224 三 blocker 修复经源码+测试+restore 逐项核对闭合），但：① 人工映射写面无 MCP 入口（映射实际来源=派生 params.section，跨章分摊只能靠嵌入方）；② 评分条件变更不触发任何审核失效（A-10——upsert 只 bump content_version，sections_digest 不含评分面）；③ is_price_item 词面误伤（A-18）；④ 无撤销语义（A-22）。属「主链通、系统性降级」而非断链。

---

## 链 6：写作校验导出链

`draft → validate → 审核 → export 门 1a/1b/1c → 回读`

### 逐跳调用链

| # | 跳 | 调用链 | 守卫 | 已知断点 |
|---|---|---|---|---|
| 1 | 起草 | 同链 1 跳 6：CAS `set_section_content`@outline.rs:742-769 + 章节工作区 `commit_section_with_note`@draft.rs:702 | readiness 八阻塞；幂等 CAS | A-14（tier 不进门） |
| 2 | 校验 | `validate_bid_document`@tools.rs:3469-3586 → `validate_document`@validate.rs:156-421：规则 3 `E_SECTION_EMPTY`（构造器 :437-462）→ 规则 4 下界 `E_CHARS_BELOW_MIN`（target×80%，:465-480/:522-527）→ 规则 7 引用回查 :297-315 → 规则 8 证据失效 :327-343 → 四桶归位 :371-410（ok = blocker==0 :410） | 规则族完备（BLOCKER/MAJOR/MINOR） | **P0-1（#243）**：service.rs:1377-1386 无条件推进——**校验结果与状态推进脱钩**（报告 22 个 BLOCKER 也照样进 VALIDATING）；A-11 |
| 3 | 审核 | 同链 1 跳 8：三门 + PreExportConfirm 绑 digest（review.rs:376-377 落账快照；:612-630 消费比对） | 验签+门序+digest 绑定（#180） | A-03（receipt 不绑内容）；A-23（RT 无出口——BLOCKER findings→RT Open→export_allowed 恒拒且无工具关单）；A-10（digest 六字段窄面） |
| 4 | 门 1a | `export_document`@service.rs:1179 → `check_export_ready`@matrix.rs:334-343 → `export_blockers`@matrix.rs:320-331（**存量 `self.matrix`**；矩阵未构建也拒 :322-325） | blocking 清零门在位（M3 变异 KILLED） | **A-01**（build_matrix 全量绑定 vs matrix_view 活证据——双口径使门 1a 弱于工具面）；**A-02**（时间过期无翻转入口）→ **过期证据脱钩 = 导出门最薄弱面** |
| 5 | 门 1b | `export_allowed`@service.rs:1181 → review.rs:601-643：三门遍历 :604-611 → 最后一条 PreExportConfirm Approved 的 digest ≠ 当前 → E_UNPROCESSABLE :612-630 → RT 全 DONE :631-641 | 三门+内容绑定+RT 面 | A-03/A-10（如上）；A-23（唯一出口缺失） |
| 6 | 门 1c | `writing_acceptance_findings`@service.rs:1188-1207 → validate.rs:502-530：必写叶无正文/未 DRAFTED → E_SECTION_EMPTY；正文 < target×80% → E_CHARS_BELOW_MIN——**导出时点独立复算，不消费 reviews/RT/gate_approved**（validate.rs:482-501 门禁语义注记） | **#242 门真实生效**：M4 变异 KILLED（writing_acceptance_e2e.rs:486 `writing_acceptance_export_independent_of_review_approval`）；SDK e2e 02-full-pipeline 红（344/119/256/166 字 < 400 下限 → 22 BLOCKER）= 门生效的反向运行证实 | —（本门为六链中最强门） |
| 7 | 产物落盘 | `DocumentExporter::export_with`@export.rs:1045+：① tmp 残骸清理 :1055-1060 ③ 导出门 ④ 幂等 no-op（manifest 指纹全等+输出 sha 吻合 :1126-1128）⑥ 构建 ⑦ tmp+rename 原子落盘 ⑧ sha+manifest 原子落盘；PDF 通道 `convert_to_pdf`@service.rs:1302-1331（幂等跳过 + sha 对账） | 原子性（tmp+rename）；manifest 幂等 | A-08（ExportGate/LinkageLedger 模块孤岛——产品等价能力由 manifest 承载但 EXPORTING 中间态不存在）；C-07（幂等键漏 bidder_name——暗标换名重导吃旧缓存） |
| 8 | 回读 | 导出成功路径回读 = manifest no-op 判定读产物 sha（export.rs:1126-1128；service.rs:1307-1311 PDF 跳过标记）；**深回读只在测试面**：export_readback.rs（6 测试，产物 zip 解包逐章断言——见 module-audit-C E-1） | 测试守卫在位 | 生产路径无独立「导出后全文回读校验」步骤（依赖门 1-3 + 构建期断言） |

### 链路完整性裁定：**DEGRADED（#242 门+原子落盘真实；P0-1 单向门 + 1a 存量矩阵 + 1b 三缺口）**

- **强面**：门 1c（#242）真实且经变异+e2e 双重验证；产物原子性（tmp+rename+manifest）与回读测试面扎实；
- **弱面**：① validate 跳是单向门（P0-1——本链第 2 跳一旦误触，第 1 跳不可回）；② 门 1a 与工具面双口径（A-01/A-02——失效证据响应可出标）；③ 门 1b 的 digest 窄面（A-10）+ receipt 不绑内容（A-03）+ RT 无出口（A-23）。

---

## 六链总览

| 链 | 裁定 | 关键断点 |
|---|---|---|
| 1 项目生命周期 | **BROKEN** | P0-1/#243（validate 单向门）、A-23（RT 无出口）、A-01/A-02（导出门旧账） |
| 2 全块覆盖 | **BROKEN** | #244（覆盖闭环未接）、B-01（requirement 黑洞）、B-02（前缀过滤） |
| 3 证据 | **BROKEN** | P0-2/#245（bind/matrix 不查 tier 与内容）、A-01/A-02/A-14 |
| 4 补遗影响 | **BROKEN** | A-09（dirty 零传播）、A-10（审核不失效）、A-04（waiver 共门）、P0-1/C-05（重起草锁死） |
| 5 评分 | **DEGRADED** | #224 step1 在位；MCP 写面缺失、A-18/A-22/A-10 |
| 6 写作校验导出 | **DEGRADED** | #242 门真实；P0-1、A-01/A-02、A-03/A-10/A-23 |

**结构共性**：六链的域层守卫（状态机 guard 表、readiness 八阻塞、receipt 验签、#224 聚合不变量、#242 写作门）实现质量高且有变异级测试守卫；系统性风险集中在**「域规则已实现、生产链路未接线/接错线」**（A 组 6 孤岛 + 双口径分裂）与**「编排层的单向推进」**（P0-1/A-11/A-23 三同型死锁）。
