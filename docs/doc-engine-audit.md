# 文档引擎审计与集成边界（Doc Engine Audit：mdcore + cleanroom-analysis）

> 审计对象：`mmqz/mdcore`（私有产品仓，审计基线 main = `6daeace`，另有 compliance-baseline / mother/r23-snapshot / office-g-phase1-anchor 分支）与 `mmqz/cleanroom-analysis`（公开研究底稿仓，审计基线 main = `149b32e`）。审计时间：2026-10-23。方式：**只读**（依用户要求未向两仓提交任何内容；能力需求经用户口头转达引擎团队）。
> **2026-10-25 复核**：引擎仓已同步至最新（mdcore = `c33bcab`：新增提交集中于壳/web/打包面，`git diff --stat 6daeace..c33bcab` 无 office-io 触及）；G1/G2/G3 三锚点在 c33bcab 逐条复核**全部仍有效**（G1 docx.rs:2162-2163 todo!() 面 / G2 spacing-docDefaults 零命中 / G3 docProps 零命中）——正式提交材料见 `docs/engine-capability-requests-g1g3.md`。
> 结论先行：**引擎的成熟度显著高于本业务侧此前"最小接口契约"（AD-05：create_block/update_block/get_block/render/export）的假设**——块锚点、证据三态、审校门禁、原子发布、乐观并发、容器安全门均已产品化并有语义锁测试背书。集成方案应从"自建薄适配"调整为"对齐引擎既有契约面"。差距集中在**交付格式面**（目录域/文档级样式/暗标元数据），见 §4 差距矩阵与 §6 能力需求清单。
> 状态标注：全部条目为 `VERIFIED_SOURCE`（源码直读，锚点可复核）。

## 1. 审计范围与方法

- mdcore main：14 crates workspace（mdcore/mdcore-wasm/office/office-io/pdf/pdf-wasm/workbench/evidence/canvas/collab/relay/settings/shell/shell-native）+ web/ + CONTRACT.md v1.3（冻结契约）+ gates.md（102KB 门禁记录）+ COMPLIANCE.md（闭源最严姿态、SBOM 全白名单、四路审计 CLEAN）。
- cleanroom-analysis：office-line（xlsx-r1/r2、docx-r1/r2、office-f 渲染级读侧评估、office-g 锚点写面方案）、HANDOFF.md（阶段 5 四表面收口 + 阶段 6 骨架启动）、DECISIONS.md。
- 复核方式：源码逐模块读（anchor/evidence/refs/locator/status/workbench/security/office-io 五模块/CONTRACT 全文），能力宣称与"已验证可宣称"边界交叉核对（CAPABILITIES.md / FIDELITY.md 单一真源纪律）。

## 2. 引擎能力面清单（与本业务相关的子集）

### 2.1 块模型与锚点（crates/mdcore）

- `BlockTree` = 唯一语义真理；块物化 `BlockId`（ADR-B1 混合式：path 变化由 ID 消化）。`apply_ops` 唯一变异入口，`Source` 四分（**View/Undo/Redo/Ai/System**——人与 AI 同一命令路径、同受 undo/事件流约束）。
- **锚点寻址**（`src/anchor.rs`）：`Anchor{block, offset}`；`TextTarget::ByAnchor{expect}` 供 AI/延迟命令——**结构变化后仍可解析**；目标块被删 → `AnchorError::Missing` 显式 reject（"绝不静默猜测"）；`expect` 内容不匹配 → `ExpectMismatch`（乐观并发）。注意：偏移自动跟移（周围文本变化自动平移）为已登记的增强项，v0 以 expect 保护——**标书侧使用锚点必须始终携带 expect 串**。
- 版本与外部变更：`revision` 单调推进；`FileStamp/plan_reload/adopt_external`（外部重载三分：Noop/SilentAdopt/OfferChoice）；`match_external_markdown/fingerprint_manifest`（外部修改的结构匹配）；`is_dirty/mark_saved` 脏标记。
- 会话面：`SessionManager` 一窗一档（同 document_id 不得双开）、资源上限 16、焦点序恢复（CONTRACT §5）。

### 2.2 证据系统（crates/evidence）——与本业务证据模型**直接同构**

- `EvidenceRecord`：claim（主张文本）+ `claimed_value`（数值锚）+ `claim_locator/claim_doc` + `source_locator/source_doc` + `excerpt` + `relation` + 三态 + `generation_batch`（AI 生成轮次可整体追溯）。
- **关系四分**（`RelationKind`）：DirectQuote（直接引用）/ Calculated（经计算记录推导）/ **Synthesized（综合推断——"程序不可完全验证，审核默认必经"）** / HumanAttested（人工背书）。
- **三态正交**（`src/status.rs`，原文："三个正确性维度互不蕴含，不存在单一 verified 汇总"）：
  - `LinkageStatus`：Valid / Stale（可达但指纹不匹配）/ Missing / Ambiguous——对应**来源可达与一致性**；
  - `CalcStatus`：NotApplicable / Passed / Failed(InputUnavailable|InputMismatch|EvaluationError|PrecisionMismatch|ResultMismatch)——对应**可复算面**；且仅 Linkage=Valid 才许复算（**级联诚实性**：来源不实不许复算通过）；
  - `ReviewStatus`：Pending / Approved / Rejected——**"AI 提出的关联/推导一律落 Pending；程序只能验证可程序验证的面，模型置信度不是验证"**。
- **只降不升铁律**（`degrade_linkage`）：Valid 可降 Stale/Missing/Ambiguous；失效态不经**显式重验通道**不得互转或回 Valid——静默重绑禁令。
- `DocumentRef{doc_id, engine, version, fingerprint}`：**失效判定用指纹（内容派生），审计用版本（保存代数）**——两者分离，另存不改 ID、回滚指纹复原而版本前进。`EngineKind`：Docx/Xlsx/Pdf/Markdown/Canvas。
- `Locator` 七变体（`src/locator.rs`）：XlsxCell/XlsxRegion/DocxParagraph/DocxTableCell/**MarkdownBlock**/PdfPageText/CanvasObject——每变体只携带"该引擎内可稳定重寻址的最小信息 + 定位时刻内容指纹"（"位置还在但内容已换"的二级失效判定）。刻意不做跨引擎统一坐标。
- `CalculationRecord`：输入（各带来源定位器+登记快照）+ 白名单表达式 + 精度舍入规则 + 中间值 + 复算状态（`expr.rs` 求值器 1077 行）。sidecar 本地存储纪律（敏感摘录不出本机）。

### 2.3 工作台八步闭环（crates/workbench）——审校门禁已产品化

- 九命令：IngestSource / ProposeBinding / ProposeCalculation / Recalc / **RefreshBody**（正文刷新→claim 锚失效检测）/ **ReverifySource / ReverifyClaim**（双侧显式重验）/ **Review**（人工审核）/ ExportClean（干净导出）。
- 命令层纪律（`src/command.rs`）：**幂等键**重放返回首次结果；**ExpectedVersion::Exactly/MustBeNew/Any** 乐观并发（过期拒绝而非覆盖）；Succeeded/Failed 入幂等表，Rejected/VersionConflict 不入（重试不被旧失败毒化）；全部命令入审计。
- **ReviewPolicy**（`src/security.rs`，会话级注入不入 sidecar）：`reviewer_prefixes` 默认 `["human:"]`——**AI 操作者不可改审核态**（"AI 只提关联与推导"的门面化）；`allow_approval_of_broken_linkage` 默认 false——**关联失效不得直接过审，必须先显式重验恢复 Valid**；`calc_required_at_or_above` 主张值门槛（高值主张须计算面 Passed）。
- 写入安全：`PathPolicy` 路径允许列表（canonicalize 符号链接逃逸防护）；资源限额 fail-closed（证据 ≤1000 条 / 计算 ≤500 条 / sidecar ≤16MB，超限拒绝发布而磁盘旧文件不动）；`publish` 同目录 tmp+rename 原子发布（失败不留半成品）。

### 2.4 wasm 出口面（crates/mdcore-wasm）——全部能力可经桥消费

`WasmCore`（编辑核）、`WasmWorkbench`（八步闭环全链：restore(sidecar,body)→ingest→propose→recalc→reverify→review→export_clean，含 pending_reviews/stale_evidence/audit/report 查询面）、`export_docx`（BlockTree→DOCX 字节）、`import_docx`（DOCX→markdown→编辑器可编辑可撤销，window.__importDocx）、xlsx 三面（import/export/grid 切片）。阶段 5 另有表格表面（WasmBook 切片/虚拟滚动/冻结/复制粘贴/证据高亮）、PDF 表面（pdf.js 渲染 + lopdf 标注增量写回）、Canvas 表面、证据侧栏（集成旅程 e2e 7/7）。

### 2.5 Office 域（crates/office + office-io）——能力宣称边界纪律完备

**DOCX（docx.rs，4378 行，docx-rs 0.4.22 引擎，模型级处理与渲染保真分离）**：
- 读面：块序（段落/表）+ 样式 ID 透传 + run 级文本/加粗/斜体/删除线/下画线/等宽 + **超链接目标四变体**（外部 URL/包内/文档锚/兼容未解析）+ 表格单元格拼接 + **页眉页脚六槽位**（default/first/even × 页眉/页脚）+ 图片容器级媒体清单（word/media 枚举 + rels 联接 + 引用计数）+ numPr/ind 结构承载 + 列表导入映射（Bullet/Ordered 计数器语义镜像）。
- 写面：Heading1-9（outline 级）+ 富文本段落（bold/italic/mono/strike/underline/link）+ 文本表格 + 超链接（含 RunSpec.link 外部 URL）+ **页眉页脚三态**（default/first/even，first 自动 titlePg、even 自动 evenAndOddHeaders）+ 脚注引用 + **PNG 内联图** + **PAGE/NUMPAGES 域**（fldChar begin/instrText/end 三连）+ 列表（ordered/bullet 各 9 级 + 多行续行缩进）+ 等宽 code + 水平线 + 引用块（缩进×深度）+ **锚点写面**（office-g Phase 1+2 已实施：RunSpec.anchor / anchor_link / bookmark_paragraph + V1-V7 校验）。
- 明示不支持（登记在案）：TOC/PAGEREF/HYPERLINK 域（引擎 todo!() 崩溃面，**绝不暴露**）；code 语法高亮；表格单元格内超链接目标；读面脚注内容（引擎从不读 footnotes.xml）。

**XLSX（io.rs/write.rs/rebuild.rs）**：读 = 三道容器门（声明四检查预扫 + 尺寸诚实[谎报头闭环] + 部件良构[标签配对/深度限/截断检出，默认严格拒]）+ calamine 五面 + CellData 九变体三分（原始值/推断类型/显式转换，公式缓存值与公式文本两通道）+ **稀疏包围盒炸弹实证闭环**（1588B 输入曾实测触发 ~550GB 分配 abort → 现门 3 前置 typed 拒）；写 = rust_xlsxwriter 包装（引擎类型零泄漏、流式回退写 typed 拦截绝不静默丢数据、全簿 100 万格预算）；**C3 修改不宣称支持**——替代路径"读-改-重建"（保真五面：表序/值/公式/合并/超链接；**结构性丢样式**，FIDELITY v2 五档丢失矩阵定谳，skipped 显式登记非静默降级）。

### 2.6 PDF 域（crates/pdf，mdc-pdf 第二 Document Model）

产品边界 = **阅读 + 标注，不做内容编辑**。六命令（ExtractText 页粒度 / Search（max_hits 1000 钳制）/ Outline / AnnotateAdd / AnnotateEdit / AnnotateList），Human 与 AI 走同一 Command 路径（/T author 字段溯源）。安全策略 S1-S6：pdfium **non-V8 构建**（JS/XFA 不接，选型即安全决策）/ 危险对象族八族只读告警 / 限额三行为学（硬拒绝-钳制-弃置：输入 1GiB、打开预算 3s、渲染看门狗 20s）/ 渲染工人 panic 隔离 + 看门狗换代 / **加密态拒绝一切写命令** / 表单探针被动只读。lopdf 增量写保持原字节前缀（签名文档友好）。

### 2.7 安全与合规基线（可直接继承的防线）

容器门（zip/OPC 声明预扫-尺寸诚实-良构三道，两模式安全上限恒等）+ mdc-office 消毒六面（CSV 公式/XML 字符/XML 实体/HTML/域白名单/URL scheme）+ 各域资源限额四件套（常量+执行点+typed 错误+测试锚点）+ PathPolicy + 原子发布。合规：SBOM 367 项全白名单、GPL/AGPL 零容忍、denylist 门禁每轮收口。**标书系统上传面与导出面可直接继承这套纪律**——业务侧不再自建文件敌意面防线，只做调用接线。

### 2.8 协作域（预留，MVP 不依赖）

collab（yrs 0.28 适配层，D1-a 边界文档已确认：GC/UAF 隔离、skip_gc 不变量、内存上限、撤销栈归属、稳定锚点跨模型映射）+ relay（服务端中继，权威端形态）。多人协同编辑标书是后续扩展位。

## 3. 三层保证模型 × 引擎支撑映射（评审三层表的落地）

| 保证层 | 本业务要求（risk-register §0） | 引擎既有支撑（VERIFIED_SOURCE） | 集成动作 |
|---|---|---|---|
| L1 输入完整性 | 源文件/页/表/附件处理情况可核对，失败位置明确 | 容器门 typed reject + 良构性警告通道（Book.warnings）+ OCR/解析失败隔离是**业务解析层职责**（引擎只管 Office/PDF 容器内） | import_tender_files 的 per_file 报告对接引擎错误面；失败件禁引沿用 |
| L2 规则正确性 | 日期/金额/型号/资格/格式等可机械验证规则执行正确 | **CalcStatus 复算面**（输入快照+白名单表达式+精度舍入+五类失败码+级联诚实性）+ 只降不升失效判定 + 资源限额 | 报价合计/工期/有效期等数值断言走 CalculationRecord+Recalc；日期/禁字类规则留在业务规则引擎（引擎不解释招标语义） |
| L3 语义正确性 | 条款解释、证据-承诺支持关系必须人工确认 | **ReviewStatus 三态 + ReviewPolicy**（human: 前缀门禁 / 失效不过审 / 高值须复算通过）+ Synthesized 默认必经审核 + generation_batch 批次追溯 | 矩阵 policy 确认、导出审批直接映射 Review 命令；**审批动作的操作者前缀在业务侧强制 human:** |

**结论**：评审提出的"三层分立"在引擎侧已有同构实现且更细（三态正交、静默重绑禁令、级联诚实性）。业务侧不需要重造这些机制，需要的是**把领域对象（REQ/EV/CMP/SEC）正确映射到引擎对象**（§5）。

## 4. 与标书业务的差距矩阵

> 判定口径：`ENGINE_GAP`（引擎侧缺口，需引擎团队开发）/ `BIZ_SIDE`（业务侧职责，非引擎缺口）/ `ROADMAPED`（已在引擎路线图或已 spike）。

| ID | 能力项 | 标书需求场景 | 引擎现状（锚点） | 判定 | 优先级 |
|---|---|---|---|---|---|
| G1 | **目录（TOC）域** | 标书必须有自动目录（多级+页码） | docx.rs 登记不暴露：docx-rs TOC/PAGEREF/HYPERLINK 域为 todo!() 崩溃面；PAGE/NUMPAGES 已用 fldChar 三连实现（存在同构先例） | ENGINE_GAP | **P0** |
| G2 | **文档级格式面** | 招标格式硬要求：页边距、正文字体字号（宋体/仿宋族）、行距、封面页 | 写面仅 Heading 内建样式 + 直接格式（缩进/边框/等宽字体）；无文档级默认样式/页边距/行距 API | ENGINE_GAP | **P0** |
| G3 | **暗标元数据清零** | 暗标废标红线：导出件 docProps 不得含作者/公司/最后修改者 | 无 docProps 清零面（ExportClean 是证据侧干净导出，非暗标格式） | ENGINE_GAP | **P0**（R-11 BLOCKER） |
| G4 | 分节符/页码重启 | 目录罗马页码、正文阿拉伯页码从 1 重启、奇偶页眉 | first/even 页眉页脚三态写面已有；**节（section）级页码重启/分节符未见** | ENGINE_GAP | P1 |
| G5 | 模板套用 | "按招标方提供的格式模板填"（封面/投标函格式页） | 导出路径 = 从 BlockTree 新建；无"模板 DOCX 反向填充/样式继承"路径 | ENGINE_GAP（形态需讨论：模板样式面 or 块插入） | P1 |
| G6 | 渲染级页数/格式复验 | TC-12 页数上限（Word 口径）、V1-c 格式断言自动化 | office-f C 路线 spike 全过（LO headless→PDF：5 分页符恰 6 页硬断言；冷 1024ms/热 304ms） | ROADMAPED（待产品化） | P1 |
| G7 | **非正文结构读面与定位器**（文本框/批注/修订/页眉页脚） | 招标 DOCX 模板的签章位/格式说明常置于文本框；页眉页脚可载限制性条款；批注/修订载澄清口径 | 文本框（w:txbxContent）读面未承载（引擎登记不支持面清单）；批注/修订读面 P2；**Locator 七变体均无 header/footer/textbox/comment/revision 寻址**（V1-a 实测确证：页眉页脚文本可读但无法挂证据） | ENGINE_GAP（读面+定位器变体；V1-a 实证升证） | P1 |
| G8 | MarkSpan URL 载荷 | 正文超链接（少见但存在） | 引擎已登记遗留（"富文本无消费方"裁决待定）；导出降级纯文本 | ENGINE_GAP（已登记） | P2 |
| G9 | PDF 页内定位粒度 | 证据定位从"页"到"页内行/区域" | PdfPageText = 页号+页文本指纹；页内区域未支持 | 引擎边界外（excerpt 摘录可补偿） | P2 |
| G10 | PDF/OCR 表格结构提取 | 招标文件 PDF 表格→条款清单（解析主路径） | mdc-pdf 产品边界 = 阅读+标注；ExtractText 页粒度文本 | **BIZ_SIDE**（解析层用 MinerU/OCR 管线，本就是业务侧 import_tender_files 职责，非引擎缺口） | — |
| G11 | 容量匹配 | 500 页级标书、量价清单 Excel | 写面限额：DOCX MAX_PARAGRAPHS/MAX_RUNS；XLSX 全簿 100 万格/表数 256（fd 校准实测） | 已有（容量规划输入：超限需分册导出策略） | — |

**已确认可直接依赖（无需引擎变更）**：块锚点/expect 乐观并发（V3 第一问）、证据三态+显式重验（V3 第二问）、原子发布+幂等+版本门（V3 第三问）、ReviewPolicy 人工门禁（审批边界）、容器安全门+消毒链（上传面）、ExportClean、锚点写面 bookmark_paragraph（引用回填 V1-b）。

### 4.1 V1 验证回填（verification-plan §3.1 四段执行后，2026-10-24）

> 业务侧验证套件（scripts/v1_*.py，run_tests T7，证据 artifacts/v1/）对差距矩阵的回填——引擎侧判定不变，验证状态列更新；定位器/指纹为 CONTRACT 语义 Python 仿真（SEMUL，引擎本体接入后升 ENGINE_RUN）。

| 缺口 | V1 前 | V1 后（业务侧证据） | 引擎侧判定 |
|---|---|---|---|
| G1 目录域 | ENGINE_GAP（docx-rs todo!() 面） | 同构先例增强：业务侧样本以 fldChar 三连落位 TOC/PAGE/NUMPAGES 域，LO 渲染通过（V1-c 结构断言 + 页脚页码域逐页渲染）——域代码路径可行性已验，引擎侧实现仍待排期 | ENGINE_GAP（P0 不变） |
| G3 暗标元数据清零 | ENGINE_GAP（无 docProps 清零面） | **业务侧缓解已验（V1-d，SEMUL 仿真线——FG-06②/FG-13 口径标注，#305）**：docProps 清零 + app.xml Company 移除 + 禁字双面扫描 + 金丝雀防空洞绿**仅在 scripts/v1_darkbid.py 仿真导出线验证**——产品路径（docx-rs 写面）仅文本 anonymize，无 docProps 清零/全部件禁字扫描；产品面端到端由 #317 承接，落地后补真机验证条目 | ENGINE_GAP（P0 不变） |
| G6 渲染级复验 | ROADMAPED（spike 全过） | **业务侧复验管道 TESTED**：LO headless → PDF 渲染断言固化于测试套件（8 页硬断言/分页纪律/页眉页脚域/图片绘制位置/引用文本可见）——引擎侧 C 路线产品化后可替换为引擎断言 API | ROADMAPED（待产品化，验证价值已坐实） |
| G7 非正文读面/定位器 | 登记不支持（文本框小项） | **V1-a 实证扩展**：corpus-R D2a（页眉页脚条款/文本框/批注/修订五探针）——①页眉页脚文本可读可抽取；②文本框/批注/修订对 naive 正文遍历不可见（raw XML 盘点才能捕获）；③**非正文条款无法定位挂证据**（如页眉载明的限制条款）——定位器变体缺口从"小项"升为实证差距 | ENGINE_GAP（P1，范围从文本框读面扩至非正文定位器变体） |
| G9 PDF 页内粒度 | 页级+excerpt 补偿 | V1-a 确证边界：B1 设备清单 40 行含全同日期列 → 页级定位多页命中（多义性已记录）；excerpt 摘录补偿在引用链场景够用（V1-b 10 链全定位） | 边界外（P2 不变） |

**V1-b 对 §5 集成边界设计的验证**：双载体定位链路（条款 Locator → 证据 Locator → 正文引用 Locator + CIT- 书签）全 10 链回查 valid；索引型定位器在结构变化下 10/10 漂移失效而书签再寻址存活并经显式重验恢复——§5.1/§5.2 的"锚点跨重写稳定 + 失效只降不升 + 显式重验通道"设计必要性获得运行时实证（SEMUL 口径）。

## 5. 集成边界：两系统如何共同维护版本与证据关系

> 评审要点：不只研究 MCP 如何调用文档引擎——研究**两个系统共同维护版本与证据关系**。以下为映射契约草案——**三问设计语义已经 SEMUL 断言全绿（scripts/v3_integration.py，run_tests T9：五轮重写引用保持/只降不升/两阶段导出中断一致性），引擎本体接入后升 ENGINE_RUN**。

### 5.1 ID 与版本映射（ID 隔离不变，映射单向）

- **ID 隔离维持 AD-05**：REQ/EV/CMP/SEC（业务命名空间）与 DOC-*（引擎 BlockId）永不混用；业务侧只存 `content_ref = BlockId` + `DocumentRef{doc_id, engine, version, fingerprint}` 快照。
- **失效判定用指纹、审计用版本**（引擎定谳原则直接采纳）：补遗/重写后，业务侧用内容指纹判"这段引用是否仍指同一内容"，用版本戳做审计序——**不再自造第二套失效判定**。
- 双载体定位：编辑态引用走 `Locator::MarkdownBlock{block_id, text_fingerprint}`（引擎侧已有）；导出态（DOCX 交付件）走 `Locator::DocxParagraph{paragraph_index, text_fingerprint}`。同一 EV 在两载体各登记一条，校验"从正文块到导出段落"的链路连续性（V1-b 断言）。

### 5.2 多轮重写下的引用有效性（V3 第一问）

- 章节重写 = 引擎侧 Ai source 的 op 序列（可 undo、有 revision、进事件流）；业务 `content_ref`（BlockId）**跨重写稳定**——锚点定位不随文本变化失效，除非块被删除（显式 Missing reject，正好是要暴露给业务的事件）。
- 业务侧每次重写携带 `expect` 串（引擎 v0 无自动跟移）——Skill 的 rewrite 指令必须显式传被替换内容摘要，失败即停并转人工。
- 重写后跑 `RefreshBody` 等价检查：受影响 EV 的 claim_locator 指纹失配 → degrade 至 Stale（只降不升）→ **必须经 ReverifyClaim/ReverifySource 显式重验或人工 Review**，绝无静默重绑。

### 5.3 撤销/恢复/另存/重开下的关联维持（V3 第二问）

- undo/redo 是引擎事务（System 事务不入栈的例外规则已知）；业务侧订阅事件流（source=Undo/Redo）触发自己的影响面检查：undo 导致正文回退 → 关联 EV 的 claim 锚指纹可能失配 → degrade + 待重验。
- 另存/重开：`open_document(md, meta)` 身份复原 + document_id 不变；外部修改走 plan_reload 三分（Noop/SilentAdopt/OfferChoice）——**业务侧对 OfferChoice 场景（外部改了导出件再回读）默认转人工**，不自动 adopt。
- sidecar 与正文同生命周期（md+meta 双文件），业务库的 EV/绑定记录与 sidecar 的 EvidenceRecord **一对一同步**（业务 EV 是领域视图，引擎 EvidenceRecord 是文档内证据事实——同步点 = ProposeBinding/Review/Reverify 四命令，全部有幂等键与审计）。

### 5.4 导出与保存中断的一致性（V3 第三问）

- 两阶段确认：业务 `EXPORTING`（调用引擎 export_docx / ExportClean）→ 引擎原子发布返回字节 → 业务侧 sha256 落库 + 断言（页数/禁字/元数据——依赖 G1~G3 补齐后为完整断言）→ 置 `EXPORTED`。**没有 sha256 落库就没有 EXPORTED**——中途 kill 时业务态停在 EXPORTING，恢复协议重新导出（幂等键同参返回缓存或重跑，引擎侧 Succeeded/Failed 才入幂等表语义兼容）。
- 并发写保护：业务 project_version（乐观锁）与引擎 ExpectedVersion::Exactly 双门——两侧任一版本冲突即拒绝，不存在"业务接受了旧版本引擎产物"。
- 保存中断：引擎 publish tmp+rename 保证无半成品；业务态与文件态的最终一致由"EXPORTED 以落库 sha256 为界"规则收口。

### 5.5 人工审批边界（S1 威胁模型的审批面）

- 审批动作（矩阵 policy 确认、BLOCKER WAIVE、导出放行）映射引擎 Review 命令，操作者前缀强制 `human:`（ReviewPolicy 默认档即业务要求的默认）；AI 操作者（Skill 驱动的 Agent）在引擎层就不可写审核态——**审批强制点下沉到引擎**，业务层不再单点防守。
- 导出前 BLOCKER 清零/显式 WAIVE 的前置（MCP §8）叠加引擎 `allow_approval_of_broken_linkage=false`——双层门禁，任一层独立成立。

## 6. 引擎能力需求清单（P0/P1/P2，供转达引擎团队）

> 按用户要求：**不向引擎仓库提交任何内容；以下清单直接口头/对话交付，由用户转达**。每项含需求场景与建议验收形态；优先级按本业务 MVP 阻塞关系排序，供引擎侧排期参考。

**P0（标书 MVP 硬阻塞——对应 BLOCKER 风险 R-11 与格式合规）**

1. **DOCX 目录（TOC）写面**：自动目录能力——TOC 域（fldChar 三连同构于已有 PAGE/NUMPAGES 实现，需解决"打开时域更新前目录为空"的路径：或渲染级预计算页码后写静态目录+域可刷新，或暴露域更新策略）。验收形态：导出件在 Word/WPS 打开即有正确多级目录与页码。
2. **文档级格式写面（最小集）**：页边距、正文默认字体字号（中文字体族）、行距设置。验收形态：能满足典型招标格式条款（如"正文宋体小四、1.5 倍行距、页边距上下 2.5cm"）的确定性表达。
3. **暗标导出模式**：docProps 元数据清零（作者/公司/最后修改者/创建工具等标识字段）+ 可选无页眉页脚变体。验收形态：导出件属性面板零标识信息 + 文件级断言（docProps XML 检查）。**（V1-d 更新：业务侧 zip 层缓解已验可作过渡；引擎面仍为终态需求——见 §4.1 G3 行）**

**P1（重要，影响交付质量与自动化复验）**

4. **分节与页码重启**：分节符 + 节级页码重启（罗马/阿拉伯切换）——目录页与正文页码分离的通用形态。
5. **模板套用路径**（形态需讨论）：基于模板 DOCX 的两种可能形态——a) 模板样式继承（读模板样式面导出时套用）；b) 模板占位块插入（BlockTree 插入模板固定段）。建议先出方案再实施。
6. **渲染级复验产品化**：office-f C 路线（LO headless→PDF）从 spike 转产品化（页数/分页断言 API）——本业务 TC-12 与 V1-c 的自动化依赖它。**（V1-c/d 更新：业务侧已固化同构复验管道于测试套件可作参照；引擎产品化后替换为引擎断言 API——见 §4.1 G6 行）**
7. **DOCX 文本框读面**（w:txbxContent）：招标模板的签章位/格式说明常在文本框，当前读面丢弃。**（V1-a 更新：范围扩证——建议连同非正文定位器变体（header/footer/textbox/comment/revision）一并评估，非正文条款当前无法挂证据——见 §4.1 G7 行）**

**P2（锦上添花）**

8. MarkSpan URL 载荷（引擎已登记遗留，本业务低频）。
9. PDF 页内区域定位器（行级/区域级；当前页级+excerpt 够用）。
10. DOCX 批注/修订读面（若未来做"修订协商"工作流）。

**明确不需要引擎做的**（业务侧自理，避免误会边界）：PDF/OCR 表格结构提取（解析层 MinerU 管线）；招标语义规则（有效期基准、评分办法解释）；知识库检索（KBAdapter 自建）。

## 7. 审计证据锚点（复核入口）

| 结论 | 锚点（mdcore main=6daeace） |
|---|---|
| 锚点/expect/reject 语义 | crates/mdcore/src/anchor.rs（全文件 67 行） |
| 四分关系/只降不升/claimed_value | crates/evidence/src/evidence.rs（L17-26, L151-165） |
| 三态正交/级联诚实性 | crates/evidence/src/status.rs（L1-13, L74-80） |
| DocumentRef 指纹/版本分离 | crates/evidence/src/refs.rs（L1-7, L109-138） |
| Locator 七变体+二级失效 | crates/evidence/src/locator.rs（L81-160） |
| 八步命令/幂等/版本门 | crates/workbench/src/command.rs（L1-14, L55-121） |
| ReviewPolicy human: 门禁/失效不过审 | crates/workbench/src/security.rs（L168-213） |
| PathPolicy/限额/原子发布 | crates/workbench/src/security.rs（L1-166） |
| DOCX 读写面与登记不支持项 | crates/office-io/src/docx.rs 模块文档（L1-90 增量史） |
| XLSX 三门/类型三分/炸弹闭环 | crates/office-io/src/io.rs 模块文档（r4-t1/t2 增量节） |
| 读-改-重建保真边界 | crates/office-io/src/rebuild.rs 模块文档 |
| 导出映射表与降级登记 | crates/mdcore-wasm/src/docx_export.rs 模块文档 |
| 八步闭环 wasm 面 | crates/mdcore-wasm/src/workbench.rs（L339-519） |
| PDF 六命令/S1-S6 | mdcore/CONTRACT.md §6；crates/pdf |
| 能力宣称边界 C1/C2/C3 | cleanroom-analysis/office-line/xlsx-r1/CAPABILITIES.md；docx-r1/FIDELITY.md（24 项五态矩阵） |
| 锚点写面 Phase 1+2 已实施 | cleanroom-analysis/office-line/office-g_锚点写面最小扩展方案.md；mdcore c4856f78 |
| 渲染级 C 路线 spike | cleanroom-analysis/office-line/office-f_docx渲染级读侧评估pass.md（B 样本 6 页硬断言/冷热耗时） |
| 当前进度（阶段 5 收口/阶段 6 骨架） | cleanroom-analysis/HANDOFF.md §6 速览（2026-10-23 p6-t1） |
