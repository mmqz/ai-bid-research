# 标书领域数据模型（Bid Domain Model）

> 原则：领域状态独立持久化于业务后端数据库（与文档内核存储完全分离）；**条款 ID、证据 ID 与文档块 ID 属于三个互不混用的命名空间**，通过外键关联而非嵌套重编号。所有状态转换由后端状态机裁决，Agent 只能通过 MCP 工具申请转换。
> ID 命名空间：`PRJ-`(项目) `TF-`(招标文件) `REQ-`(条款) `EV-`(证据) `CMP-`(矩阵行) `SEC-`(章节) `VF-`(审校发现) `RT-`(修订任务) `DOC-`(文档内核块，独立系统签发)。

## 1. 领域对象

### BidProject（投标项目）
| 字段 | 类型 | 说明 |
|---|---|---|
| id | PRJ-xxxx | 项目主键 |
| name / tender_notice_ref | str | 项目名 / 公告编号 |
| status | ProjectStatus | 状态机（见 2） |
| bid_deadline | datetime | **投标截止/开标时间**（资质有效期校验基准） |
| mode | enum | 明标 / 暗标（影响导出与校验规则集） |
| content_version | int | 项目级内容版本（乐观并发控制） |
| external_key | str | 客户端幂等键（同 key 重复创建返回已有项目） |

### TenderFile（招标侧文件）
id=TF-xxxx；project_id；kind（招标文件/附件/图纸/补遗/答疑/评分办法/格式模板）；sha256（幂等去重）；pages；ocr_tier（native_text / pdf_plain / paddleocr_vl —— 借 LingYan 分档）；parse_status（PENDING / PARSING / OK / PARTIAL / FAILED）；parse_summary（页数、表格数、失败页清单）。**PARTIAL/FAILED 文件的产物禁止被引用**（R-01）。

### Requirement（招标条款）
| 字段 | 说明 |
|---|---|
| id / project_id | REQ-xxxx |
| category | qualification（资格条件）/ veto（否决项）/ scoring（评分项）/ tech_param（技术参数）/ commercial（商务要求）/ format（格式要求）/ other |
| source_anchor | 溯源锚：file_id + 定位（行号区间或页码+条号）+ **原文引用片段**（不可篡改） |
| statement | 规范化陈述 |
| params | 结构化参数抽取：model（型号）/ qty / amount（含单位与含税口径归一化）/ date / deadline / cert_type 等结构化槽位 |
| mandatory | required / optional / bonus（必选/可选/加分——借 ark"不得混谈"纪律） |
| clause_no | 条款编号（完整性校验与精确匹配主键，R-01/R-02） |
| response_policy | satisfy / partial_satisfy / deviation / not_respond / pending |
| status | NEW → CONFIRMED → MAPPED → DRAFTED → VALIDATED |
| conflicts_with | REQ-[]（补遗与正文冲突时双向登记，入澄清清单） |
| dedupe_hash | source_anchor+statement 归一化哈希（重跑抽取幂等合并） |

### Evidence（企业证据）
id=EV-xxxx；project_id + org_ref（跨项目复用企业资料库）；source_type（资质证书/业绩合同/验收报告/人员证书/检测报告/财务/信用/产品参数/历史标书）；file_ref（对象存储）；title；issuer；issue_date；**valid_until（有效期，EXPIRED 即禁止绑定，R-03）**；coverage_tags[]（可证明能力标签：资质等级/行业/产品线/人员规模）；chunks[]（chunk_id → 文件+位置映射，证据链可回溯）；status（OK / EXPIRING / EXPIRED / MISSING / CONFLICT）。

> **V3-1 实装注（2026-10-28）**：记录级已落地 `bid-domain/src/evidence.rs`——状态机取 Active/Expiring/Expired/Superseded 四态（MISSING/CONFLICT 属矩阵行视角，T-2 落地）；valid_until 为 ISO 字符串（字典序即时间序）；锚点/指纹经 `locator.rs` 引擎七变体镜像；sidecar 分离存储待 G-8 工具面（V3-6）接引擎。

### ComplianceItem（响应矩阵行）
id=CMP-xxxx；requirement_id ⇄ evidence_ids[]；gap（NONE / PARTIAL / FULL / UNMET）；draft_section_id；owner_note。**资格条件与否决项行必须显式 policy，未填禁止进入起草阶段**（R-03/R-04 闸门；原引审计问题 Q5，编号映射见 risk-register 追溯矩阵）。

> **V3-2 实装注（2026-10-28）**：矩阵行已落地 `bid-domain/src/matrix.rs`——四态（NOT_RESPONDED/PARTIAL/RESPONDED/NOT_APPLICABLE）从绑定+章节+覆写派生（单一真源 `derive_matrix_rows`）；**blocking = 必选类（资格/否决）未响应 → `check_export_ready` 阻断导出**；id = `CMP-{REQ 后缀}` 派生稳定（重建幂等）。评分面三维结构（分值/标准原文/硬门）同批落地 `scoring.rs`（分值来自 extract.rs T-3 扩展的 `params.score` 确定性正则槽位）。

### BidSection（章节任务）
| 字段 | 说明 |
|---|---|
| id / project_id / parent_id / order / title | SEC-xxxx，目录树 |
| toc_ref | 对应招标评分项/必选章节（评分镜像——借 tender-master） |
| status | SectionStatus（PENDING → OUTLINING → DRAFTING → DRAFTED → VALIDATING → REVISING → ACCEPTED；DIRTY 为脏标记可叠加） |
| depends_on | 依赖的 REQ-[] 与 EV-[]（影响面分析依据，R-05） |
| inputs_version | 起草时消耗的 (矩阵版本, 证据集合哈希)；任一上游变更 → DIRTY |
| content_ref | 文档内核块句柄 **DOC-xxxx**（只存引用，不复制内容——与文档内核解耦） |
| word_target / page_budget | 字数/页数预算（暗标页数约束输入） |
| last_model / updated_at | 换模型恢复的依据（TC-08） |

> **V3-1 实装注（2026-10-28）**：实体 + 树校验（九不变量）+ 从 REQ 推导（配额最大余数法）+ 状态机（7 态 + ACCEPTED 显式 reopen）已落地 `bid-domain/src/outline.rs`；toc_ref/inputs_version/content_ref/last_model 属 M3 起草面（S-1/S-2 批次）。

### ValidationFinding（审校发现）
id=VF-xxxx；kind（DETERMINISTIC / MODEL）；rule_code（如 PRICE_EXCEEDS_BUDGET / CERT_EXPIRED / PAGE_LIMIT / DARK_BID_WORD / MISSING_SIGNATURE / UNANCHORED_CLAIM）；severity（BLOCKER / MAJOR / MINOR）；section_id / requirement_id；message + 证据定位；status（OPEN / RESOLVED / WAIVED + 理由）。**BLOCKER 未清零禁止 export**（状态机硬闸门）。

### RevisionTask（修订任务）
id=RT-xxxx；finding_ids[]；section_id；instruction；status（OPEN / CLAIMED / DONE / SUPERSEDED）。审校→修订→复检闭环，防"发现即丢失"（R-04/R-07；原引 Q3）。

## 2. 状态机

```
ProjectStatus:
DRAFT → FILES_IMPORTED → REQUIREMENTS_EXTRACTED → MATRIX_READY
      → EVIDENCE_BOUND → DRAFTING → VALIDATING → REVIEWING
      → READY_TO_EXPORT → EXPORTED
回退规则：任何阶段检测到上游 content_version 变化 → 受影响 SEC 标记 DIRTY，
项目不整体回退，仅重跑 DIRTY 章节（影响面分析，R-05；原引 Q8）。
越级保护：状态机拒绝与当前 status 不符的工具调用（E_STATE）——低能力模型跳步防护（R-14）。

SectionStatus: 见上；ACCEPTED 后需显式 reopen 才可修改（幂等保护，R-06）。
```

## 3. 关联关系（ER 摘要）

```
BidProject 1─N TenderFile / Requirement / Evidence / ComplianceItem / BidSection / ValidationFinding
Requirement 1─1 ComplianceItem 1─N Evidence（多对多经矩阵行）
BidSection N─N Requirement / Evidence（经 depends_on + 矩阵行派生）
ValidationFinding N─1 BidSection / Requirement；RevisionTask N─N ValidationFinding
BidSection ──(content_ref 单向引用)──> DOC-block（文档内核命名空间，本系统不解析其内部结构）
```

## 4. 关键设计决策

1. **ID 隔离**：REQ/EV/SEC 由业务库签发；DOC-* 由文档内核签发。两者只通过 `content_ref` 引用连接，条款 ID 不参与文档内部编号（用户要求：不混用）。
2. **溯源锚不可变**：source_anchor 的原文引用片段在 REQ 创建时固化，后续任何编辑不得覆盖（审校阶段比对用）。
3. **参数结构化前置**：型号/金额/日期在抽取阶段即落入 params 槽位，供确定性规则与精确检索直接消费，不再从自然语言二次猜测（R-01/R-02 与 R-07 规则分界；原引 Q2/Q7）。
4. **企业资料库跨项目**：EV 支持 org 级库 + 项目级绑定两段结构（借 rag-tender 的 Organization→Document→Evidence 分层），资质一次入库多项目复用。
5. **诚实状态**：所有"未确认/缺证据"以显式枚举表达（pending/MISSING/待澄清），禁止用空值或 0 默认替代——防"看起来完成了"。
6. **审核门四道**（T-7 + #131 增补）：`matrix_approval` / `outline_approval` / `pre_export_confirm` 三道导出门序链（export_allowed 逐门校验）+ `addendum_waiver`（#131：L3「无补遗阻塞」的人工确认出口——单件招标文件起草前置闸；与三道主门正交，无门序前置，不入导出门序）。Approved 是里程碑事实（同门重放幂等），Rejected 是过程记录。
7. **起草 readiness 六阻塞**（#110 + L3）：无关联 REQ / 缺资质 / 业绩不足 / 技术参数无佐证 / 否决项未响应 / 无补遗（MissingAddendum）→ 拒生成 E_UNPROCESSABLE（人话理由）。指令与缺失回报进 `draft_note` 账面（#128），正文零标记。
8. **评标规则不进正文**（#121/#126）：否决项/资格条件不抄规则原文，合并进「资格与响应」章节组写响应话术（矩阵有章节→不报 BLOCKER）；评分项完全不进正文（自评表面）。
