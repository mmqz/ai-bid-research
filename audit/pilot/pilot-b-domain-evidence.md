# 试点审计 B：bid-domain 领域模型 + bid-tools 证据/占位/质量链路

- 审计代理：3-b-pilot（ai-bid-research 全仓审计 V2）
- 基线：main@6105beb（分支 audit/v2-full-coverage，工作区另有未跟踪 audit/evidence/，无生产文件改动）
- 纪律：只读审计。本文件与 inventory-pilot-b.csv 是本轮唯一新增文件。
- 审计方式：逐文件通读（长文件分段读全）+ Grep 交叉定位消费方/调用方；未运行 cargo test/build（冻结纪律）。

---

## 1. 范围与覆盖声明

### 1.1 逐文件全覆盖（13 个声明范围内生产源码，全部读全）

| # | 文件 | 行数 | 覆盖 |
|---|---|---|---|
| 1 | bidcore/crates/bid-domain/src/lib.rs | 1722 | 全读 |
| 2 | bidcore/crates/bid-domain/src/evidence.rs | 1269 | 全读 |
| 3 | bidcore/crates/bid-domain/src/review.rs | 1319 | 全读 |
| 4 | bidcore/crates/bid-domain/src/principal.rs | 448 | 全读 |
| 5 | bidcore/crates/bid-domain/src/coverage.rs | 405 | 全读 |
| 6 | bidcore/crates/bid-domain/src/export_gate.rs | 169 | 全读 |
| 7 | bidcore/crates/bid-domain/src/linkage.rs | 182 | 全读 |
| 8 | bidcore/crates/bid-domain/src/locator.rs | 373 | 全读 |
| 9 | bidcore/crates/bid-domain/src/dates.rs | 115 | 全读 |
| 10 | bidcore/crates/bid-tools/src/placeholder.rs | 262 | 全读 |
| 11 | bidcore/crates/bid-tools/src/quality.rs | 922 | 主逻辑全读（480-922 为 B3 词表/打分纯函数与内联测试，结构性通读） |
| 12 | bidcore/crates/bid-tools/src/readiness.rs | 1114 | 全读 |
| 13 | bidcore/crates/bid-tools/src/draft.rs | 1265 | 全读 |

### 1.2 调查链路扩展读取（#245/#224/#244 推演必需的消费方，非声明范围但全部逐段读）

- bid-domain/src/matrix.rs（640 行全读）——EvidenceBinding→矩阵→导出门的派生真源
- bid-domain/src/scoring.rs（2388 行，定向读全关键面：ScoringItem/SectionMapping/ScoringMapping/derive_scoring_items/validate_sums/score_section/score_document/unmapped_items/upsert_scoring_mapping/scoring_items/scoring_tree）
- bid-tools/src/service.rs（1834 行，定向读：register_evidence/register_evidence_verified/bind_evidence_batch/record_review_gate/issue-verify receipt/export 三前置门/prepare_outline_and_matrix 面注释）
- bid-tools/src/validate.rs（537 行，规则 1-8 全读）
- bid-tools/src/check.rs（616 行，头部+check_local+结构读全）
- bid-tools/src/export.rs（2103 行，模块头+fail-closed 门清单+公开面读全，正文渲染细节未逐行）
- bid-mcp/src/tools.rs（4311 行，定向读：register_evidence/bind_evidence/score_section/score_document/build_scorer/build_outline/export_bid_document/search_evidence 有效期降权面）
- bid-domain/src/impact.rs、outline.rs、param.rs、assignment.rs：仅按引用点抽读（不属本轮范围）

### 1.3 对照测试（评估真实性，非逐行）

bid-domain/tests：state_machine_matrix.rs、status_enum_matrix.rs、four_contracts.rs、scoring_mapping.rs（#224 专项）；
bid-tools/tests：evidence_content.rs、evidence_source.rs、quality_check.rs、readiness_check.rs、draft_section.rs；另抽验 validate_tool.rs（E_EVIDENCE_EXPIRED 面）。

**覆盖声明**：13/13 声明范围内文件 100% 读全；#245/#224/#244 三条调查链路的全部消费方（matrix/scoring/service/validate/check/export/MCP tools）均已定位并读取关键面。未运行任何构建/测试（纪律要求）。

---

## 2. 逐文件审计记录

### 2.1 bid-domain/src/lib.rs（1722 行）
- **职责**：项目状态机（ProjectStatus 10 态线性链 + allowed_tools 裁决表 + guard_tool E_STATE 门）、领域对象（BidProject/TenderFile/FileRole/AddendumWaiverRecord/Requirement/Anchor）、DomainStore 权威存储、StoreSnapshot 持久化 DTO + restore 语义完整性把关、#219 覆盖标记落库、#224 映射账本字段与引用完整性守卫。
- **公开符号**：ProjectStatus::{next_allowed,allowed_tools,as_str}、DomainStore（create_project/guard_tool/check_version/advance_project_status/upsert_file/register_file_role/set_requirements/replace_requirements/set_block_marks/set_block_coverage/record_addendum_waiver/to_snapshot/restore）、ToolError、requirement_hash、StoreSnapshot。
- **不变量（正面）**：①状态转换集中单点裁决（next_allowed 唯一真源，跳级/回退 E_STATE 零副作用）；②restore 把关 id 唯一/id_seq 水位/孤儿引用（RT→FND、waiver→file、role→file、SM→REQ/SEC/EV/复合键/份额聚合 ≤1.0）；③快照全字段 serde(default) 加性迁移 + HashMap→BTreeMap 字节序确定；④#224 守卫：set/replace_requirements 不得孤儿化映射账本（fail-loud）。
- **发现引用**：PB-008（set_requirements 缺 id 唯一性检查，replace 有——不对称）；PB-010（tool_name 陈旧）。
- **身份/ID 观察**：next_id 为全命名空间共享单序列（id_seq），会话内无碰撞；restore 水位覆盖 store 签发前缀（PRJ/TF/EV/SEC/PR/FND/REV/RT/SM），REQ/CMP 豁免（各有来源）。同一业务事实多对象的可能面见 PB-013。

### 2.2 bid-domain/src/evidence.rs（1269 行）
- **职责**：证据台账实体 + 状态机（Active/Expiring/Expired/Superseded 单向只降不升）+ 语义位查重/换版（Superseded 旧版保留审计）+ R-03 过期禁绑 + 主体核验告警 + #217 内容验证四态（VerificationTier）+ 结构化字段（EvidenceFields）。
- **公开符号**：EvidenceStatus::{can_bind,degrade_to}、VerificationTier::{as_str,content_read}、Evidence/EvidenceSpec/EvidenceBinding、validity_flip、subject_warning、DomainStore::{register_evidence,apply_validity,bind_evidence,subject_warnings,bindable_evidence_ids}。
- **不变量（正面）**：①degrade_to 白名单迁移，任何→Active 拒（静默复活禁令），同态 no-op；②换版经单向降级（含 Expired→Superseded）+ version+1 + 新 id；③CONTENT_MATCHES 必须携带非空 excerpt、excerpt 必须带 tier（域层不可拆对）；④validate_spec 校验指纹算法前缀/ISO 日期/坐标域/区域端点序。
- **发现引用**：PB-001（bind_evidence 只查 status.can_bind()，不查 verification_tier——文件级锚/未读内容证据可绑）；PB-002（EvidenceKind/EvidenceStatus 无占位/待核验语义；SemanticSupport/HumanConfirmed 预留态生产不可达，登记面无人工升档通道）；PB-013（语义键 (kind,title,subject,scope) 不含 source_file/sha——同证书异文件登记触发换版淘汰）。
- **序列化对称性**：verification_tier/excerpt/excerpt_locator/fields/client/amount_wan/completed_on 全部 default+skip，旧快照兼容有测试（evidence_fields_absent_in_legacy_snapshot / tier_and_excerpt_passthrough_with_serde_roundtrip）。

### 2.3 bid-domain/src/review.rs（1319 行）
- **职责**：审核闭环三件套（Finding/ReviewRecord/RevisionTask）+ 三道门门序强制 + BLOCKER→RT 结构性派生 + #180 内容摘要绑定失效 + #182 TrustedPrincipal 身份纵深。
- **公开符号**：Gate/Decision/Severity/RTStatus、ReviewRecord/ReviewReceipt、HUMAN_PREFIX、DomainStore::{record_review,gate_approved,findings,reviews,revision_tasks,open_blockers,assign_revision_task,advance_rt_status,complete_revision_task,export_allowed}。
- **不变量（正面）**：①门序（矩阵→大纲→导出确认）跳门 E_STATE typed reject 零副作用；②Blocker 数 ≡ RT 数由结构保证；③RT 终态封闭（Done 不可重开）+ 关单必须带 resolution；④#180：PreExportConfirm 绑 sections_digest，过门后正文变更 → export_allowed 拒（无关导入不误伤——绑摘要不绑全局版本，测试 B/C 钉死）；⑤幂等重放先比摘要（防改稿后重过门死锁）。
- **发现引用**：无新 P0/P1。备注（设计权衡非缺陷）：审核失效传播只覆盖「章节树内容」维度——评分项/证据/矩阵在过门后变更不会使 MatrixApproval/PreExportConfirm 失效（#224 第 4 步、#180 扩面均属未做，与 issue 状态一致）。
- **Receipt 评估**：见 2.4 与第 6 节——四重绑定（版本/actor_type/scope/HMAC ct_eq）在生产面成立。

### 2.4 bid-domain/src/principal.rs（448 行）
- **职责**：#182 TrustedPrincipal：HMAC-SHA256 receipt 签发/验签（v1 六段线格式、scope 绑定、24h TTL + 60s 未来容差、常量时间比对）。
- **正面结论**：eac447b 声称的「签域四重绑定」**属实且在当前 main 验证通过**：①线格式六段含 actor_type（域层再拦非 Human）；②scope 进 MAC 输入（A 门 receipt 过不了 B 门、TF-0003 免不了 TF-0004）；③时效窗防陈旧重放；④ct_eq 防时序泄漏。密钥面（service.rs:517-573）：`.mang/approval.key` 0600 或纯内存，**不进 MCP 工具表**（issue_approval_receipt 无 MCP 暴露，grep 验证）。Agent 经 MCP 伪造 receipt 需要宿主密钥文件——不成立。
- **残余面**：TrustedPrincipal::human() 便捷构造 pub（无身份保证，文档已声明「域层单测与已验签路径用」）；域 API（record_review 等）收 TrustedPrincipal 而非 receipt——进程内调用方绕过验签的可能仍在（纵深依赖「service 是 MCP 唯一入口」的部署约定）。PB-009 同族。

### 2.5 bid-domain/src/coverage.rs（405 行）
- **职责**：#219 全块覆盖：BlockVerdict（Requirement/Noise/Rule/ReviewTable）、区间标记 BlockMark（不重叠/不越界/全覆盖校验）、SubmittedRequirement（挂点必须在 requirement 区间 + category 七值域 fail-loud）、CoverageReport（covered/uncovered/by_verdict）。
- **正面**：纯函数零 IO；validate_marks/validate_submitted/compute_coverage 均有正反测试；REQUIREMENT_CATEGORIES 与矩阵 blocking 面对齐（单一真源声明）。
- **发现引用**：PB-012（数据模型能表达「已排除（verdict+reason）/未判定（uncovered）」，但 build_outline 无消费门——#244 的闭环缺失在接线层不在模型层）。

### 2.6 bid-domain/src/export_gate.rs（169 行）
- **职责**：两阶段导出门（EXPORTING → sha256 落库为界 → EXPORTED）+ SC-01 查询面唯一口径（preview_sha/artifact_sha 分离）。
- **正面**：consistent() 不变式（EXPORTED ⟺ artifact_sha 在场）；中断注入/幂等恢复/同 sha 复跑有测试。**注意**：本模块是内存门，与 DomainStore 状态机的接线在 service/export 面消费。
- 无新发现。

### 2.7 bid-domain/src/linkage.rs（182 行）
- **职责**：证据链状态账本（Valid/Stale 只降不升 + 显式 reverify 唯一回升 + 失配恰一链）+ FNV-1a 64 指纹（公开向量对齐）。
- **观察**：这是独立于 DomainStore 的独立账本结构（链 id 外部注入）；生产消费面在 impact.rs/four_contracts（SEMUL 交叉验证）。与 evidence.rs 台账是两套状态语义（LinkageStatus vs EvidenceStatus），无互操作——设计上属「引擎侧语义复刻」，未见混淆误用。
- 无新发现。

### 2.8 bid-domain/src/locator.rs（373 行）
- **职责**：引擎定位器七变体镜像（serde 与 mdcore 逐字节一致，字面 JSON 钉死）+ A1 坐标域/区域端点序校验 + 指纹算法前缀纪律。
- **正面**：字面 JSON 钉死测试防形态漂移；roundtrip 全变体恒等；fingerprint_algorithm 拒裸值/空算法。
- 无新发现。

### 2.9 bid-domain/src/dates.rs（115 行）
- **职责**：ISO 日期严格解析（10 字符定长、月日域）+ Hinnant 公历换算 + add_days + today_ymd。
- **发现引用**：PB-011（parse_ymd 接受 2026-02-30——只校月日域不校真实日历；测试明示为既有口径。证据有效期面可落不存在日期，字典序比对不受影响，属低危口径债）。

### 2.10 bid-tools/src/placeholder.rs（262 行）
- **职责**：草稿正文显式占位语法（【待补充：…】/【待确认：…】）的构造/扫描/计数/报告（纯字符串面，零 store 依赖）。
- **正面**：make/scan 往返一致、跨行/未闭合不误吞、what 清洗（不产空壳）。
- **发现引用**：PB-004——模块头承诺「两者都不能留到导出（导出前清零或显式豁免）」，但 scan_placeholders/count_placeholders/placeholder_report **全仓零生产消费者**（grep 验证：仅本文件测试 + readiness 的 suggest_placeholder 建议文本）。导出闸门对占位残留完全无感知。

### 2.11 bid-tools/src/quality.rs（922 行）
- **职责**：单章质量检查（REQ 覆盖/EV 引用核对/字数带 ±20%/无锚承诺）+ 项目级报告 + artifacts/quality-report.json 落盘（tmp+rename）+ B3 针对性/实质性/重复度。
- **正面**：只读诊断不落库不推状态；needs_human 判定含 FAILED/未覆盖 REQ/幻觉引用/带外/严重重复；报告 serde roundtrip 有测试。
- **发现引用**：PB-001 佐证——cited_evs/uncited_evs 只做引用核对，不区分证据 verification_tier（FILE_EXISTS 证据被正文引用与 CONTENT_MATCHES 同权计入「已佐证」面）。

### 2.12 bid-tools/src/readiness.rs（1114 行）
- **职责**：生成前置检查（九类 Blocker + 警告）+ L3 补遗分级阻塞（#184 逐件判定+waiver 出口）+ 输入完整性（P0-1 RESTRICTED 受限起草标注）。
- **正面**：纯只读派生；`is_usable_evidence = ev.status.can_bind()` 与 evidence/draft 三处同口径声明；matrix_responded 消费人工覆写优先的真实矩阵态（「正文写了承诺不算已响应」有 Lead 裁决注释+测试钉）；Agent 必选接线（UncoveredMandatoryItem）。
- **发现引用**：PB-001 核心（is_usable_evidence 不查 verification_tier——FILE_EXISTS 占位资质即「可用」，MissingQualification/MissingTechEvidence/req_has_usable_evidence 全部放行）。

### 2.13 bid-tools/src/draft.rs（1265 行）
- **职责**：单章起草输入装配（build_section_input：幽灵 REQ 剔除/EXPIRED 拒绑）+ mock/external 生成器 + 引用回查/未知引用剔除 + 无锚承诺计数 + draft_section 提交路径（CAS/状态推进/S-2 落盘/draft_note 账面）+ B4 迭代起草器。
- **正面**：失效证据 E_UNPROCESSABLE 硬拒（R-03 时序第二道）；#217 EvBrief 带 excerpt/excerpt_locator（生成器只准引实料）；覆盖度/无锚承诺即时告警。
- **发现引用**：PB-003（EV_SUPPORT_TEMPLATE 对任何过检证据——含 FILE_EXISTS——生成「具备可核验的证据支撑，评标委员会可按证据台账索引回查原件」的正文断言）；PB-007（draft_section_iterative 的 scorer 用 derive_scoring_tree 直建，不并入 #224 账本映射）。

---

## 3. #245 专项推演（占位资质能否冒充已核验的合规证据？）

### 3.1 证据对象的实际状态维度表

| 维度 | 载体 | 登记时裁决 | **被谁执行（拦截/阻断）** |
|---|---|---|---|
| 存在性（来源文件在库） | source_file_id → TenderFile | 服务端（register_evidence_verified：不存在/Failed/无解析产物拒） | ✅ 执行（#185 已闭环） |
| 来源身份（sha256/指纹） | source_sha256/fingerprint | 服务端从库内生成（调用方不可自报；MCP 旧参数显式拒） | ✅ 执行（#185 已闭环） |
| 内容已读 | VerificationTier（FILE_EXISTS/CONTENT_MATCHES；带 locator 对账→CONTENT_MATCHES） | 服务端裁决（MCP 面无法自报 tier） | ❌ **零执行面**——全仓（matrix/readiness/draft/validate/export/quality）无一消费 |
| 核验通过（资格属实） | **无此维度** | — | ❌ 不存在（#245 要求的「占位/待核验」持久化状态或类型未建模） |
| 有效期 | EvidenceStatus（Active/Expiring/Expired/Superseded） | 登记一律 Active；翻转靠 apply_validity/bind 内联 | ⚠️ 部分执行（bind 时 R-03 硬门 ✔；后续翻转的矩阵陈旧面见 PB-005） |

### 3.2 压缩点（#245 正文所述「EvidenceStatus 没有 占位/未验真 语义」在当前 main 的形态）

`bind_evidence`（evidence.rs:608-659）的可绑判定**只有一行**：`ev.status.can_bind()`（Active|Expiring）。
矩阵派生 `derive_matrix_rows`（matrix.rs:167-197）：`has_ev = !evs.is_empty()`（绑定存在即算）→ (有证据,有章节)→RESPONDED / (有证据,无章节)→PARTIAL / blocking 仅在 NOT_RESPONDED。
就绪门 `is_usable_evidence`（readiness.rs:499-501）：`ev.status.can_bind()`。
即：**五个真实维度被压缩成「有效期单维度」；「内容已读」维度已采集（#217 数据面）但从未进入任何裁决路径。**

### 3.3 冒充路径推演（全链路逐步，每步有代码锚点）

```
① MCP register_evidence（tools.rs:3784-3844）
   入参：source_file_id=任意已导入非失败文件（甚至一份无关扫描件），
        kind="qualification"（调用方自由选择）、title="…一级资质证书"（自由文本）、
        无 locator → 服务端裁决 FILE_EXISTS + status=Active（service.rs:918-931）
   ✔ 不被拦截（来源真实≠内容属实——tier 如实标 FILE_EXISTS，但无人消费）
② bind_evidence（evidence.rs:608）→ can_bind()==true（Active、无 valid_until 永久件）
   ✔ 绑定成功；bind_evidence_batch 顺手 build_matrix（service.rs:984）
③ 矩阵：has_ev=true + 章节在位 → RESPONDED；资格条件/否决项 blocking 清零
   （matrix.rs:176-187——「阻塞数因此归零」与 issue #245 实测现象一致）
④ readiness：usable_evidences(Qualification) 命中该 Active 记录 →
   MissingQualification 不触发；req_has_usable_evidence/MissingTechEvidence 放行
   （readiness.rs:571-606, 712-717）
⑤ draft：EvBrief 进生成素材；mock 模板产出
   「依据 [EV-xxxx]…本章节相关承诺具备可核验的证据支撑，评标委员会可按证据台账索引回查原件」
   （draft.rs:287）——正文宣称资质已具备，且该句是结构必产出
⑥ validate：E_VETO/E_QUAL 查矩阵（RESPONDED 不触发）；E_EVIDENCE_EXPIRED 只查
   Expired/Superseded（Active 不触发）；W_PLACEHOLDER 只查 TODO/待补充/【占位】
   （validate.rs:200-343）——全部通过
⑦ export：check_export_ready（矩阵 blocking=0）+ export_allowed（三门/RT/digest）
   + writing_acceptance（空章/篇幅）（service.rs:1178-1193）
   ✔ 导出成功——verification_tier 全程未被查询；导出产物（docx/pdf）无任何
     待补/未核验标记（export.rs 零 placeholder/verification 引用，grep 验证）
```

### 3.4 结论

**占位（FILE_EXISTS/未验真）资质在当前 main 可以完整流经 EvidenceBinding→矩阵→起草→导出而不被任何一道门拦截，且导出正文会主动生成「具备可核验的证据支撑」的断言。** issue #245 的复现路径成立；#217/#185 只完成了「来源真实」与「内容已读」的**数据面**，「内容已读」未接入任何**裁决面**，「资格属实/占位不得清阻塞」维度完全缺失。#245 应保持 OPEN（与当前状态一致）。

补充：Agent 自行声明 verified 的路径——MCP 面已封死（register_evidence 走 register_evidence_verified，EvidenceRegistration 无 tier 字段；旧来源参数显式拒）。但进程内 `BidService::register_evidence`（域直通、tier 透传）仍 pub（PB-009）。

---

## 4. #224 ScoringMapping 现状核查结论（PR #236 已合并、issue 仍开放——重核）

逐项对 #224 原始 7 项：

| # | 需求 | 当前 main 状态 | 证据 |
|---|---|---|---|
| 1 | ScoringMapping 业务对象 | ✅ 已落地（域层） | scoring.rs:134-150（id/scoring_req_id/section_id/weight_share/evidence_refs/source_anchor/created_from）+ lib.rs:417/1046-1050 持久化 |
| 2 | 真正消费 score_rules | ❌ 未做（透传 + SCORE_RULES_RESERVED note 显式登记） | scoring.rs:60-63 注释自认「判档不消费」；score_document notes（scoring.rs:1172-1183） |
| 3 | 评分项↔章节多对多 | ✅ 域层已落地；⚠️ **生产不可达**——upsert_scoring_mapping 无任何生产调用方（grep 全仓：仅域测试），MCP 无写面工具（PR #236 自述「不新增 MCP 工具」），MappingSource::AgentSubmitted/Manual 是死变体 | scoring.rs:1347-1457；grep upsert_scoring_mapping 仅 tests + restore 校验 |
| 4 | 跨章节重复计分检测 | ✅ 聚合不变量（Σ 有效份额 ≤ 1.0）在登记面与 restore 双侧执行（含派生 params.section 计入/同节去重/更新路径排除被替换行/1e-9 容差） | scoring.rs:1387-1417；lib.rs:1319-1362 |
| 5 | UNMAPPED 结构化 | ✅ unmapped_scoring_items[]（req_id/criteria 60 字摘要/weight）进 score_section/score_document 出参 + notes 告警；价格项不入 | scoring.rs:1232-1242, 1145-1158；tools.rs:2455, 2730 |
| 6 | 映射证据+双向追溯 | ✅（第 1 步口径）：score_document 出参 mappings 报告（section/weight_share/source/source_anchor/excerpt/evidence_refs）；by_item 带 mapping_ids + 旧键 section_id=首映射保留 | tools.rs:2565-2733 |
| 7 | 评分项变更→审核失效 | ❌ 未做（与 #224 分步计划、issue 状态一致） | review.rs #180 绑定面只覆盖章节树摘要 |

**weight_share 消费**：真实——score_section 按 share 摊分（满分×share、estimated 同比、rationale 带「摊分N%」；单映射 share=1.0 与历史逐位一致）（scoring.rs:1046-1102, 1216-1227）。

**缺陷**：
- PB-006（P1）：账本登记面零生产调用方——多对多、AgentSubmitted/Manual、聚合不变量在生产链路全部休眠；唯一活映射仍是抽取层 params.section 单值（RuleParsed）。#224 保持 OPEN 正确；且「多对多已落地」的说法只能限定在域层+restore 层。
- PB-007（P2）：**评分视图分裂**——`DomainStore::scoring_tree()`（scoring.rs:1322-1324）与 `BidService::draft_section_iterative`（draft.rs:1237-1240）都不并入持久化账本映射，而 MCP `build_scorer`（tools.rs:2935-2976）会并入。一旦账本有数据，B4 迭代自评循环与 score_* 工具对同一章节给出不同 max/ratio——优化目标与诊断口径不一致。
- 正面：restore 五道校验（id 唯一/水位/孤儿/复合键重复/份额聚合）+ set/replace_requirements 的映射引用完整性守卫（lib.rs:893-908, 951-968）真实存在且有测试（scoring_mapping.rs 891 行、upsert 幂等+五类拒+聚合双侧拒+快照往返）。

**一句话结论：#224 第 1 步（实体+多对多+UNMAPPED+摊分+双侧完整性）在域层真实、充分、可测；但登记面无生产入口（MCP 无写工具），多对多在生产不可达；第 2/3/4 步未做；另发现评分树视图与 MCP 评分器视图不一致的分裂缺陷。**

---

## 5. 问题记录

```
ID: PB-001
严重程度: P0
模块: bid-domain + bid-tools
文件路径: bidcore/crates/bid-domain/src/matrix.rs（派生）；bid-domain/src/evidence.rs（绑定）；bid-tools/src/readiness.rs（就绪）
代码行号: matrix.rs:167-197（has_ev 派生）、evidence.rs:635-643（can_bind 单维判定）、readiness.rs:499-501
函数/结构体/接口: derive_matrix_rows / DomainStore::bind_evidence / is_usable_evidence
问题描述: #245 核心缺陷在基线 main@6105beb 仍完整存在：VerificationTier（内容已读四态）只有数据面（#217），零裁决面消费。证据可绑/可清阻塞/可导出的全部判定只看 EvidenceStatus（有效期单维），FILE_EXISTS（正文从未定位读取）证据与 CONTENT_MATCHES 同权重进入响应面。
调用链: register_evidence(MCP→verified, tier=FILE_EXISTS) → bind_evidence(can_bind✓) → build_matrix(RESPONDED/blocking清零) → readiness(is_usable_evidence✓) → draft(引用为佐证) → export(三前置门全过)
触发条件: Agent 对任意已导入非失败文件登记无 locator 的资质类证据（kind 自由选择、title 自由文本）并绑定到资格条件/否决项 REQ
实际影响: 占位/未验真资质清零合规阻塞；导出正文含「具备可核验的证据支撑」断言——资格性审查虚假响应，废标级
复现步骤: 见本报告 §3.3 七步推演（每步有代码锚点）
当前测试: 无。evidence_content.rs/evidence_content_tool.rs 只测 tier 数据正确性；readiness_check.rs 只测 status 维度（过期件）
缺失测试: 「FILE_EXISTS 证据绑定资格条件 REQ 后矩阵仍 PARTIAL/待补 + 导出门拒或受限」的正反测试
建议方案: ①derive_matrix_rows/readiness 引入 tier 投影：FILE_EXISTS → 状态降为「待核验」新态（如 PENDING_VERIFICATION），不清 mandatory blocking 或导出门标 RESTRICTED；②export 产物（封面/合规性声明/报告）显式列出未达 CONTENT_MATCHES 的绑定证据清单；③接线 #245 的服务端受控升档通道（接 #182 receipt）
证据: matrix.rs:176-179
  let st = match (has_ev, has_sec) {
      (true, true) => ResponseStatus::Responded,
      (true, false) => ResponseStatus::Partial,
      _ => ResponseStatus::NotResponded,
  };
readiness.rs:499-501
  pub fn is_usable_evidence(ev: &Evidence) -> bool {
      ev.status.can_bind()
  }
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PB-002
严重程度: P0
模块: bid-domain + bid-tools
文件路径: bidcore/crates/bid-domain/src/evidence.rs；bid-tools/src/export.rs
代码行号: evidence.rs:35-43（EvidenceKind 七值）、65-72（EvidenceStatus 四值）、212-223（VerificationTier 预留态）；export.rs 全文（零 placeholder/verification 引用，grep 验证）
函数/结构体/接口: EvidenceKind / EvidenceStatus / VerificationTier
问题描述: #245 要求的「占位/待核验材料的明确、可持久化状态或类型」未建模：EvidenceKind 无 Placeholder 类、EvidenceStatus 无占位/待核验态、VerificationTier 的 SemanticSupport/HumanConfirmed 是不可达预留态（登记面无升档通道）。导出产物（docx/pdf）对「待补」零标记。
触发条件: 同 PB-001；以及任何含【待补充】的草稿导出
实际影响: 占位材料与真实证据在台账/矩阵/导出三面不可区分；#245 验收条件（导出报告可追踪并显式标记）全部不满足
当前测试: 无（不存在该面）
缺失测试: 占位类型登记→绑定→导出全链负测试；旧数据迁移兼容测试
建议方案: 按 #245 范围 2/3：新状态走受控服务路径（receipt 升档），MCP 拒自报 verified（已具备——EvidenceRegistration 无 tier 位）；矩阵/导出门三分类（真实有效/待核验/缺失）
证据: evidence.rs:207-208
  /// | [`SemanticSupport`] | ... | **预留**（M2-b 语义服务落地前不可达——枚举先立面，防第三态悄悄冒充第四态） |
  /// | [`HumanConfirmed`] | 人工确认摘录与原件一致 | **预留**（接 #182 TrustedPrincipal receipt 链路；当前不可经登记面设置） |
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PB-003
严重程度: P1
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/draft.rs
代码行号: 287（EV_SUPPORT_TEMPLATE）、321-342（逐 EV 佐证段）
函数/结构体/接口: generate_mock / EV_SUPPORT_TEMPLATE
问题描述: 起草正文对任何过检证据（含 FILE_EXISTS 未读内容）结构性地生成「本章节相关承诺具备可核验的证据支撑，评标委员会可按证据台账索引回查原件」——正文直接宣称资质/佐证已具备。#245 范围 4 明确「占位不能生成『已具备/已满足』的结论」。
调用链: build_section_input（EvBrief，不筛 tier）→ generator.generate → EV_SUPPORT_TEMPLATE 固定句式
触发条件: 绑定的证据含任何 FILE_EXISTS/无 tier 记录（旧快照 tier=None 同样命中）
实际影响: 导出标书正文含虚假佐证声明；与 PB-001 叠加即 issue #245 描述的「输出宣称资质已具备」
复现步骤: 绑定 FILE_EXISTS 证据 → draft_response_section → 读正文 EV 佐证段
当前测试: draft_section.rs mock_output_omits_excerpt_when_unlocated 只验证摘录段缺省，未验证佐证声明句的措辞收敛
缺失测试: FILE_EXISTS 证据时佐证段应降级为「已登记待核验」措辞（或省略「可核验」断言）的断言
建议方案: EV_SUPPORT_TEMPLATE 按 tier 分档：CONTENT_MATCHES+ 才允许「可核验/回查原件」措辞；FILE_EXISTS 改为「已登记（内容未核验，待补证）」
证据: draft.rs:287
  const EV_SUPPORT_TEMPLATE: &str = "依据 [EV_ID]（KIND：TITLE，适用范围：SCOPE），本章节相关承诺具备可核验的证据支撑，评标委员会可按证据台账索引回查原件。";
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PB-004
严重程度: P1
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/placeholder.rs；bid-tools/src/validate.rs；bid-tools/src/check.rs
代码行号: placeholder.rs:25-27（「导出前清零或显式豁免」承诺）；validate.rs:281-295（W_PLACEHOLDER=MAJOR）；check.rs:133（PLACEHOLDER_MARKERS）
函数/结构体/接口: scan_placeholders / count_placeholders / placeholder_report / W_PLACEHOLDER / PLACEHOLDER_MARKERS
问题描述: ①placeholder.rs 的扫描/计数/报告函数全仓零生产消费者（grep 验证：仅自身测试 + suggest_placeholder 建议文本）——「导出前清零或显式豁免」是文档承诺不是实现；②W_PLACEHOLDER 严重度 MAJOR 非 BLOCKER，且导出三前置门（矩阵/审核/写作验收）不消费 validate findings——占位残留不阻断导出；③词表只有 TODO/待补充/【占位】，placeholder.rs 自产的 Confirm 类占位【待确认：…】不被任何校验捕获。
触发条件: 草稿正文含【待补充】或【待确认】占位后直接 export_bid_document
实际影响: #245 范围 4 的「占位可以进入草稿但导出报告可追踪并显式标记」不成立；【待确认】类占位静默穿透 validate 与导出
复现步骤: draft 时 readiness warning 注入 → 生成含【待确认：工期口径】正文 → validate（无命中）→ export（成功，产物无标记）
当前测试: readiness_check.rs:619-644 只测扫描函数本身；无 validate/export 层的穿透测试
缺失测试: W_PLACEHOLDER 升 BLOCKER 或导出门消费的正反测试；【待确认】marker 命中测试
建议方案: 导出门新增第 4 前置：全章 scan_placeholders>0 → 拒或强制 RESTRICTED+产物标记；PLACEHOLDER_MARKERS 并入「待确认」（与 placeholder.rs 词表面同源化，消除两套口径）
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PB-005
严重程度: P1
模块: bid-domain + bid-tools
文件路径: bidcore/crates/bid-domain/src/matrix.rs；bid-tools/src/service.rs；bid-tools/src/validate.rs
代码行号: matrix.rs:232-235（build_matrix 全量绑定）、347-356（matrix_view 过滤失效）、320-343（export_blockers 用持久化矩阵）；service.rs:1178-1193（导出三前置门）；validate.rs:327-343（E_EVIDENCE_EXPIRED）
函数/结构体/接口: build_matrix / matrix_view / check_export_ready / export_document 前置门
问题描述: 导出门与视图门不同源：持久化矩阵（export_blockers/check_export_ready/validate 规则 1-2/ readiness matrix_responded 消费）用**全量绑定**（不滤失效证据——测试注释自认「build_matrix 用全量绑定（含过期证据…视图层过滤）」）；matrix_view 才过滤 EXPIRED/SUPERSEDED。证据在绑定后经 apply_validity 翻转为 Expired 时矩阵不重建（build_matrix 只在 bind/override 时跑），行保持 RESPONDED；validate 的 E_EVIDENCE_EXPIRED（BLOCKER）不被导出门消费。
触发条件: 绑定 Active 证据 → apply_validity(或后续 bind 触发的内联翻转) 翻转该证据为 Expired（且未重建矩阵/未过 validate）→ export
实际影响: 已失效证据支撑的响应面可导出；get_compliance_matrix（matrix_view）与导出裁决矛盾（同库两口径）
复现步骤: 域层：register(valid_until=过去日) → bind(2026 开标日，翻转 Expired 拒绑——此路径被拦)；换路径：valid_until=未来 → bind 成功 → apply_validity(更晚开标日) 翻转 Expired → 不再调 build_matrix → check_export_ready 仍按旧 RESPONDED 行放行
当前测试: matrix.rs:607+ matrix_view_filters_dead_evidence_bindings 只测视图；validate_tool.rs:445-470 测 validate 报 BLOCKER——均无「导出门消费」测试
缺失测试: apply_validity 后直接 export 应拒（或自动重建矩阵）的测试
建议方案: apply_validity 翻转非空时触发 build_matrix 重算；或 check_export_ready 改用 matrix_view 同源过滤；或导出门新增 E_EVIDENCE_EXPIRED 消费
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PB-006
严重程度: P1
模块: bid-domain + bid-mcp
文件路径: bidcore/crates/bid-domain/src/scoring.rs；bid-mcp/src/tools.rs
代码行号: scoring.rs:1347-1457（upsert_scoring_mapping）；tools.rs 工具表（无评分映射写工具）
函数/结构体/接口: DomainStore::upsert_scoring_mapping / MappingSource::AgentSubmitted|Manual
问题描述: #224 第 1 步的账本登记面无生产调用方：grep 全仓 upsert_scoring_mapping 仅命中域测试与 restore 校验路径；MCP 工具表未暴露任何评分映射写面（PR #236 明示「不新增 MCP 工具…账本写面留后续 issue」）。多对多、聚合不变量、AgentSubmitted/Manual 在生产链路全部休眠；运行期唯一活映射是抽取层 params.section 单值（RuleParsed, share=1.0）。
触发条件: 无需触发——能力不可达本身即问题
实际影响: #224 的核心验收（真实评分表跑完整多对多映射）在生产不可复现；#170 根因（单映射表达不了跨章评分）在生产面实际未解
当前测试: scoring_mapping.rs（域层 8+ 测试）与 score_tools.rs（MCP 面多对多/UNMAPPED 各 2）均直接调域 API 构造账本——测试绿≠生产可达
缺失测试: 无法补（缺的是接线不是测试）
建议方案: 按遗留计划接 MCP 写面（AgentSubmitted 需 receipt 纪律与 #219 联动）——本审计仅记录：issue #224 开放状态正确，「第 1 步已进 main」应表述为「域层已进 main、生产入口未接」
审计置信度: CONFIRMED
状态分类: 当前仍存在（属 issue 既知遗留，非回归）
```

```
ID: PB-007
严重程度: P2
模块: bid-domain + bid-tools
文件路径: bidcore/crates/bid-domain/src/scoring.rs；bid-tools/src/draft.rs
代码行号: scoring.rs:1303-1319（scoring_items 并入账本）vs 1322-1324（scoring_tree 不并入）；draft.rs:1237-1240（迭代起草直建 derive_scoring_tree）
函数/结构体/接口: DomainStore::scoring_tree / DomainStore::scoring_items / BidService::draft_section_iterative
问题描述: 评分映射联结视图分裂成两套：scoring_items()（+MCP build_scorer 手工并入 tree.index）并入持久化账本；DomainStore::scoring_tree() 与 draft_section_iterative 的 scorer 不并入。B4 迭代循环的优化目标（ratio）与 score_section/score_document 工具的诊断口径对同一章节会不一致。
触发条件: 账本存在 AgentSubmitted/Manual 映射（当前生产不可达——见 PB-006；一旦接通立即显性化）
实际影响: 迭代改写循环朝错误目标优化（低权重视图），多对多项的摊分不进 B4 判定
当前测试: 无（两视图一致性无断言）
缺失测试: scoring_tree() 与 build_scorer 对同一 store 产出同映射集的等价测试
建议方案: scoring_tree() 内部改走「scoring_items → 重建树」同源派生；draft_section_iterative 复用 MCP 同款联结（或下沉到 service 单一构造点）
审计置信度: CONFIRMED
状态分类: 当前仍存在（条件性——依赖 PB-006 接通后才有实际影响）
```

```
ID: PB-008
严重程度: P2
模块: bid-domain
文件路径: bidcore/crates/bid-domain/src/lib.rs
代码行号: 846-916（set_requirements）vs 943-950（replace_requirements 的 id 唯一性检查）
函数/结构体/接口: DomainStore::set_requirements
问题描述: set_requirements 只按 dedupe_hash 合并，不校验 id 唯一性：同 id 不同内容的两条 REQ 可同时入库（replace_requirements 有显式 fail-loud 检查——同库两入口宽严不一致）。重复 REQ id → derived_cmp_id 镜像出两个同 id CMP 行 + 全部按 id 查找的下游（矩阵/大纲/质量/绑定）取首个命中。
调用链: runner.rs:255 / service.rs:781（进程内）→ set_requirements；MCP extract 走 replace_requirements（不受影响）
触发条件: 合并层编号缺陷或手造件把同 id 双 REQ 送进 set_requirements
实际影响: 矩阵行 id 冲突、章节引用解析歧义（静默取首）
当前测试: 无
缺失测试: set_requirements 拒重复 id 的对称测试
建议方案: set_requirements 补与 replace 同款 seen-set 检查（或抽出共用校验）
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PB-009
严重程度: P2
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/service.rs
代码行号: 838-845（BidService::register_evidence 域直通面）
函数/结构体/接口: BidService::register_evidence
问题描述: 与 register_evidence_verified 并存的域直通 pub 方法：EvidenceSpec 全字段（含 verification_tier/excerpt）由调用方构造，服务端零裁决。生产无调用方（grep 验证），但作为 pub API 保留即保留了一条「进程内自报 CONTENT_MATCHES」的旁路——与 #185/#217「不信调用方」纪律和 #245 验收「MCP 层不能把占位改成 verified」的纵深语义相悖。
触发条件: 任何进程内集成方/未来工具误用该入口登记证据
实际影响: 条件性（依赖新调用方出现）；一旦出现即 PB-001 的自报变体
当前测试: 域层 evidence.rs 的 tier 透传测试实际走的等价路径
缺失测试: 不适用（建议删除或收窄可见性）
建议方案: 删除 pub 直通面或改 pub(crate)/doc 警示；register_evidence_verified 作为唯一登记入口
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PB-010
严重程度: P3
模块: bid-domain
文件路径: bidcore/crates/bid-domain/src/lib.rs
代码行号: 1411-1428（tool_name）
函数/结构体/接口: tool_name
问题描述: E_STATE 的 attempted 字符串映射表陈旧：仅覆盖 12 个早期工具；score_*、set_matrix_override、覆盖三工具、save_version 等全部落 "unknown_tool"。错误码/排查面退化（不影响裁决本身——guard_tool 用 allowed_tools 表）。
当前测试: state_machine_matrix 未断言 attempted 字符串
建议方案: 与 ALL_TOOLS 同源生成或删除映射统一用原名
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PB-011
严重程度: P3
模块: bid-domain
文件路径: bidcore/crates/bid-domain/src/dates.rs
代码行号: 12-24（parse_ymd）
函数/结构体/接口: parse_ymd / is_valid_ymd
问题描述: 只校验月 1..=12、日 1..=31，不校验真实日历（2026-02-30 合法——测试注释自认「本层只校月日域」）。证据 valid_from/valid_until/fields 日期可落不存在日期；字典序比对与 add_days 归一不受影响，属口径债。
当前测试: parse_ymd_strict_domain 明示该行为（有意识的口径）
建议方案: 补每 月真实天数校验（闰年规则已有 Hinnant 换算可复用）
审计置信度: CONFIRMED
状态分类: 文档与实现一致（口径债，非不一致缺陷）
```

```
ID: PB-012
严重程度: P1
模块: bid-mcp + bid-tools（#244 接线面）
文件路径: bidcore/crates/bid-mcp/src/tools.rs（build_outline）；bid-tools/src/service.rs（prepare_outline_and_matrix）
代码行号: tools.rs:3618-3688（build_outline 无覆盖判定）；837-840（extract 时 UNCOVERED_BLOCKS 仅 warning 字符串）
函数/结构体/接口: build_outline / CoverageReport
问题描述: #244（建纲前闭环处置）未实现：coverage.rs 数据模型已能表达「已排除（verdict=Noise/Rule/ReviewTable + reason）vs 未判定（uncovered）」，block_marks 持久化与 restore 加性迁移都在，但 build_outline 对覆盖状态**零消费**——既不阻断未判定块，也不输出处置清单；唯一信号是 extract 合并时的一条 warning 字符串。区分「尚未判定/已判定为要求/已判定排除」的三态处置报告不存在。
触发条件: 留一块未标记的疑似要求块 → build_outline 照常建纲 → 静默进入起草
实际影响: #244 背景所述「仅一条 warning 不足以证明已审阅」成立；issue 开放状态正确
当前测试: readiness_l3_addendum/readiness_wire 等不涉此面；无 build_outline 覆盖门测试
缺失测试: 未判定块非空 → build_outline E_UNPROCESSABLE 点名块的负测试（#244 验收原文）
建议方案: build_outline 前置 get_coverage_report 聚合：全文件 uncovered 非空 → 阻断并逐块点名；已排除块带 reason 可追溯放行
审计置信度: CONFIRMED
状态分类: 当前仍存在（issue 既知未做，非回归）
```

```
ID: PB-013
严重程度: P3
模块: bid-domain
文件路径: bidcore/crates/bid-domain/src/evidence.rs
代码行号: 447-454（semantic_key）、495-511（语义位查重/换版）
函数/结构体/接口: semantic_key / register_evidence
问题描述: 幂等语义键 (kind,title,subject,scope) 不含来源：同一张证书从两个不同文件（sha 不同 → fingerprint 不同）登记会被判「换版」——旧记录 Superseded、新记录 version+1，即使两者是同一业务事实的同一代证书。反向：title/scope 有一个字符差异（「资质证书」vs「资质证」）即两个并存语义位——同一业务事实两个对象可并存，各自绑定同一 REQ（矩阵 evidence_ids 双列）。
触发条件: 同证书多源登记 / 登记命名不一致
实际影响: 台账语义位漂移：误换版（审计噪声）与双对象并存（重复计数面，如 readiness 的 usable 计数会把双对象算 2 条）
当前测试: register_replay_noop_and_renewal_supersedes 只测同 key 语义
缺失测试: 异 source 同 key 的登记语义测试（期望：提示而非静默 Supersede）
建议方案: 换版前比对 source_sha256 不同时给出告警面（或将 source 纳入语义位并列场景的合并提示）
审计置信度: CONFIRMED（行为确定；「是否属缺陷」为 DESIGN_PREFERENCE）
状态分类: 当前仍存在
```

**对象身份/序列化/状态机三个横向问题的总结论**（对应核心调查问题 2/3/6）：
- 身份：store 签发命名空间单序列 + restore 水位——会话内与重启后均无碰撞；REQ/CMP 豁免各有出处（extract 计数器/镜像派生），set_requirements 的 id 唯一性缺口见 PB-008；业务身份的重复/并存面见 PB-013。
- 状态机：项目状态机/证据降级链/RT 链/章节链全部集中单点裁决、非法态 typed reject 零副作用，与 state_machine_matrix/status_enum_matrix 的穷举矩阵一致（见 §6）。EvidenceBound 态无专属写入器（由 draft 链补齐步进带过）——设计如此，非缺陷。
- 序列化对称性：StoreSnapshot 全字段 default + BTreeMap 确定序 + restore 语义校验；Evidence/EvidenceFields/VerificationTier/ReviewRecord/MatrixOverride/AddendumWaiverRecord 的 legacy 兼容均有正反测试（旧键缺失→宽容载入、孤儿→拒载）。未发现 roundtrip 破坏面。

---

## 6. 测试有效性评估

| 测试文件 | 独立 Oracle？ | 错误路径可达？ | 只查字符串/非空？ | 评注 |
|---|---|---|---|---|
| bid-domain/tests/state_machine_matrix.rs | ✅ 独立（位串字面量钉死 10×25 全矩阵——表变测试红） | ✅（越级/乱序/终态写面/孤儿状态全断言） | 否 | 高质量回归护栏；唯 tool_name attempted 串未断言（PB-010 因此漏网） |
| bid-domain/tests/status_enum_matrix.rs | ✅（serde 往返 + as_str 唯一 + 谓词全覆盖——防「加变体忘改分支」） | ✅（can_bind/next_allowed 全变体有确定答案断言） | 否 | 高质量；四枚举含 EvidenceStatus/ResponseStatus |
| bid-domain/tests/four_contracts.rs | ✅（Python SEMUL 断言组移植——C1 级联/C2 只降不升/C3 重写锚点/C4 导出原子性，交叉验证双实现） | ✅ | 否 | impact/linkage/export_gate 三模块的强验收 |
| bid-domain/tests/scoring_mapping.rs（#224 专项） | ✅（摊分期望值 12/8、聚合 1.4 拒、restore 双侧拒——数值断言非形态断言） | ✅（越界/孤儿/幽灵 EV/复合键重复/更新路径排除被替行） | 否 | 域层充分；**但全部直调域 API**——生产不可达性（PB-006）不被测试暴露 |
| bid-tools/tests/evidence_content.rs | ✅（corpus-A 真夹具，摘录=解析产物逐字文本，tier 裁决期望独立写死） | ✅（缺夹具 panic 显式失败，skip 门） | 否 | #217/#185 验收 D/E 的服务半边扎实 |
| bid-tools/tests/evidence_source.rs | ✅（库内 sha 优先、指纹不匹配拒——断言写死期望错误码+零入库） | ✅ | 否 | 覆盖 #185 点名两断言 |
| bid-tools/tests/quality_check.rs | ✅（covered/uncovered/幻觉 EV/带外各自独立构造夹具） | ✅（FAILED 章错误明细并入） | 否（needs_human 判定断言具体字段值） | 形式合规面可靠；tier 维度无覆盖（PB-001 佐证） |
| bid-tools/tests/readiness_check.rs | ✅（九类 Blocker 各自正反构造；`drafted_body_without_evidence_is_not_response` 钉死「正文承诺≠已响应」） | ✅（过期件/Agent 必选/未知章节不 panic） | 否 | 起草闸测试密度高；**status 维度以外的可用性（tier）无任何用例** |
| bid-tools/tests/draft_section.rs | ✅（mock 确定性=同输入同输出；字数带/引用回查/CAS attempt 各有数值断言） | ✅（EXPIRED 拒绑/E_NOTFOUND/结构节点/空响应面） | 否 | 注意：mock 模板的「可核验证据支撑」句式未被负面断言（PB-003 漏网点） |

**总体判断**：域层与工具层测试是真实的行为断言（数值/状态/错误码/零副作用），非「只查字符串非空」式占位测试；Oracle 多为独立写死的期望值或移植自 SEMUL/GT 夹具。系统性盲区恰好在三处：①VerificationTier 的执行面（无人消费也无人测消费）；②占位→导出的闸门（承诺未实现也无人测）；③生产可达性（域 API 直调测试掩盖 MCP 无入口——PB-006）。

---

## 7. 结论速览

- **#245**：占位资质**可以**冒充已核验证据流到导出（P0 复现成立，PB-001/002/003/004）；#185/#217 已封「来源伪造」与「tier 自报」，但 tier 是死数据。
- **#224**：域层第 1 步真实充分（实体/摊分/UNMAPPED/双侧完整性校验），但登记面无生产入口（PB-006），第 2/3/4 步未做；另有评分树视图分裂（PB-007）。
- **#244**：模型与持久化在、build_outline 接线不在（PB-012）。
- Receipt（#182）：四重绑定验证属实，MCP 面无伪造通道（密钥宿主持有）。
- 其余：PB-005（导出门陈旧矩阵/失效证据可导出）为证据链上另一处 P1；PB-008/009/010/011/013 为 P2/P3 口径债与旁路面。
