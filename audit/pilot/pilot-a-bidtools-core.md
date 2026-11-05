# 试点审计 A：bid-tools 服务层状态/持久化/恢复链路（pilot-a-bidtools-core）

- 审计代理：3-a-pilot（ai-bid-research 全仓审计 V2）
- 冻结基线：main@6105beb（分支 audit/v2-full-coverage 已检出，HEAD=6ae52f8 仅为 R0 审计清单提交，未触及生产代码）
- 范围：`bidcore/crates/bid-tools/`（9 个 src 文件 + 3 个指定测试文件，共 5410 行，全部逐行读毕）
- 纪律：只读审计，未修改任何生产文件/测试/配置；仅产出本报告与 inventory CSV
- 头号调查目标：Issue #243（P0 起草中校验无副作用并支持状态恢复）

---

## 1. 范围与覆盖声明

| # | 文件 | 行数 | 审计状态 | 一行结论 |
|---|------|------|----------|----------|
| 1 | src/service.rs | 1833 | REVIEWED | 服务层单一聚合（store+parsed+persist+密钥+围栏）；**`validate_document_with_reviewer` 存在无条件状态副作用（Drafting→Validating）＝ #243 根因（PA-001）**；auto_checkpoint 失败无回滚（PA-007） |
| 2 | src/store_io.rs | 282 | REVIEWED | tmp+rename 原子写正确、损坏 fail-closed 正确；**无 fsync（断电窗口，PA-006）**；无跨进程文件锁（PA-009） |
| 3 | src/runner.rs | 362 | REVIEWED | 场景运行器（内存态，不经持久化）；write_run 产物非原子写（PA-011，低危） |
| 4 | src/check.rs | 615 | REVIEWED | 纯只读检查（局部/全局冲突），确定性纪律完备，**零状态副作用，无发现** |
| 5 | src/validate.rs | 536 | REVIEWED | 校验本体确为**纯只读诊断**（不落库不改状态）；副作用全部在 service 层推进段；`writing_acceptance_findings` 导出复算面与 validate 同源 |
| 6 | src/schema.rs | 158 | READ | 运行产物 serde 契约（RunDb/TraceEntry），纯数据面，无状态语义 |
| 7 | src/fs_fence.rs | 294 | REVIEWED | 词法拒 `..` + 组件级包含 + 逐段 symlink 检查，设计良好；TOCTOU 残留已在模块头登记（PA-010） |
| 8 | src/lib.rs | 26 | READ | 模块声明与 re-export，无逻辑 |
| 9 | src/context.rs | 494 | REVIEWED | 单章 prompt 组装（只读+确定性渲染+8k 实测门），零状态副作用，无发现 |
| 10 | tests/store_roundtrip.rs | 308 | REVIEWED | 高质量重启续跑集成测试（双生命周期+磁盘唯一介质）；**未覆盖 #243、断电、并发** |
| 11 | tests/service_import.rs | 123 | REVIEWED | 导入管线真件集成测试，Oracle 独立（硬口径 tables≥5、version==2）；错误路径真实可达 |
| 12 | tests/input_integrity_gate.rs | 379 | REVIEWED | readiness/完整性判定单元测试（内存态）；断言部分耦合实现文案；与 #243 无关 |

覆盖：12/12（应覆盖 12，无遗漏；核心链路文件 service/store_io/validate/check/fs_fence/context/runner 均 REVIEWED）。

关键交叉证据（范围外引用，仅作调用链佐证）：
`bid-domain/src/lib.rs`（状态机 allowed_tools/next_allowed/advance_project_status）、`bid-domain/src/matrix.rs`（blocking 派生）、`bid-domain/src/review.rs`（record_review）、`bid-mcp/src/tools.rs`（壳层 guard_tool/check_version 转发）、`bid-mcp/src/state.rs`（全局 Mutex）、`bid-mcp/tests/validate_tool.rs`、`bid-mcp/tests/writing_acceptance_e2e.rs`（见 PA-001/PA-003）。

---

## 2. 逐文件审计记录

### 2.1 src/service.rs（1833 行）——头号目标

**职责**：MCP 后端服务态聚合（`BidService{store, parsed, persist, approval_key, fs_root}`）；所有写工具的「域调用 + 写后自动落盘（auto_checkpoint）」半边；证据来源验证（#185）；审批 receipt（#182）；导入围栏复验+限额（#183）；导出三前置门+PDF 通道。

**公开符号清单**：
- 类型：`ImportSpec`、`PerFileOutcome`、`ImportReport`、`EvidenceRegistration`、`BidService`
- 自由函数：`import_files`（受信内部管线）、`review_gate_scope`、`addendum_waiver_scope`、`matrix_override_scope`、`soffice_available`、`pdf_converter_available`、`pdf_converter_name`
- `BidService` 方法：`new` / `open` / `with_fs_root` / `fs_roots` / `checkpoint` / `store` / `store_mut` / `set_matrix_override` / `workspace_dir` / `parsed` / `create_project` / `set_requirements` / `set_block_coverage` / `replace_requirements` / `prepare_outline_and_matrix` / `register_evidence` / `register_evidence_verified` / `bind_evidence_batch` / `record_review_gate` / `record_addendum_waiver` / `issue_approval_receipt` / `verify_approval_receipt` / `approval_key` / `import` / `export_document` / `validate_document` / `validate_document_with_reviewer`；（draft.rs 内 `impl BidService` 扩展：`draft_section`、`draft_section_iterative`——不在本次范围文件清单但已交叉读取）

**状态/IO 语义**：
- 15+ 个写方法统一形态「域调用 → auto_checkpoint → 返回」。checkpoint 失败 → `E_UNPROCESSABLE`，**内存已变更不回滚**（PA-007）。
- 状态推进集中在三处合法链补齐：`advance_project_to_exported`（service.rs:1438-1458，导出后半边）、`advance_project_to_drafting`（draft.rs:786-801，对 **Validating+ 一律 no-op**——与域层 allowed_tools 拒绝 Validating 起草的口径不一致，构成「测试特权」根，PA-001/PA-003）、`validate_document_with_reviewer` 的推进段（service.rs:1379-1387，**唯一一个不做任何前置完成度检查的推进点**，PA-001）。
- 乐观并发：本层不做，壳层 `check_version(project_version: Option<u64>)`（None=跳过）；域层 `advance_project_status` 实际推进时 `content_version += 1`（lib.rs:612-615）。
- 幂等：导入 sha256 去重（完全相同重放纯 no-op 不 bump 版本）；#188 异身份登记 FileRole；draft 幂等键在壳层（见 PA-008）；validate 推进幂等（已 Validating 则 steps 为空）。

**发现引用**：PA-001、PA-002、PA-004（service 侧无覆盖门）、PA-005（tier 裁决写入）、PA-007、PA-008（配合壳层）、PA-009。

### 2.2 src/store_io.rs（282 行）

**职责**：`DomainStore` 快照 ↔ 磁盘单桥；`db.json` 单文件承载 domain 快照 + parsed。

**公开符号**：`DB_FILE`、`StoreIoError{Io,Serialize,SchemaUnsupported,Corrupt}`、`StoreFile{new,dir,path,load,save}`。

**状态/IO 语义**：
- `save`（109-121）：serialize → create_dir_all → **write(tmp) → rename**，无 fsync（PA-006）；rename 失败时 tmp 残骸不清理（P3 微瑕）。
- `load`（96-106）：exists→read→parse；损坏 JSON → `Corrupt` 拒载且原文件保留（fail-closed 正确）；schema_version 门（未来版本响亮拒绝，缺省=1 加性迁移）；`BidService::open` 再经 `DomainStore::restore` 做域语义完整性校验。
- 内嵌单测 5 个（roundtrip/原子残骸/版本门/损坏拒载/legacy 兼容/双写字节一致），质量高。

**发现引用**：PA-006、PA-009、（PA-012 关联 open 失败行为）。

### 2.3 src/runner.rs（362 行）

**职责**：corpus-A 场景运行器（TC-01/02/07/11），复用 `service::import_files` 单一真源；产出 runs/<scenario>/{db.json,trace.jsonl}。

**公开符号**：`RunOutput`、`run_scenario`、`write_run`（lib.rs re-export）。

**状态/IO 语义**：纯内存 `DomainStore`（不落服务态 db.json）；trace 记录 `project_version`；TC-11 用 `guard_tool` 断言越级 E_STATE。`write_run` 直写非原子（PA-011）。runner 侧 import 无围栏（受信内部目录，模块头已声明边界）。

### 2.4 src/check.rs（615 行）

**职责**：S-5 审核分片 + 全局一致性检查（占位符/悬空引用/格式偏差局部；金额/工期/术语/承诺冲突全局）。

**公开符号**：`CheckFinding`、`CheckKind`、`CheckSeverity`、`FeatureTuple`、`FeatureType`、`ChapterInput`、`ReviewOutcome`、`check_local`、`extract_features`、`check_global`、`run_review`、`PLACEHOLDER_MARKERS`。

**状态/IO 语义**：纯函数、特征抽取不留原文、分片并行确定性（片号归位+统一排序）、线程上限 8。**零状态/IO 副作用，无发现。**

### 2.5 src/validate.rs（536 行）——#243 核心

**职责**：`validate_bid_document` 业务半边（确定性第一层 + 模型复核注入第二层）；1.0 写作验收导出复算面。

**公开符号**：`ValidationFinding`、`CheckResult{Passed,Failed,NotPerformed}`、`ValidationReport`、`ValidationCounts`、`ModelReviewer`、`NoModel`、`validate_document`、`writing_acceptance_findings`。

**状态/IO 语义**：**本文件确为纯只读诊断**——`validate_document(store: &DomainStore, ...)` 只读 store 构建报告，永不写库、永不推进状态、永不 touch 章节/审核记录（模块头 35-37 行明示「报告是导数不是源数据」）。#243 的副作用**不在校验本体，而在 service.rs 的推进段**（见 §3）。

规则面：E_VETO_UNRESPONDED/E_QUAL_UNMET（全局）、E_SECTION_EMPTY/E_CHARS_BELOW_MIN/W_CHARS_OFF_TARGET/W_PLACEHOLDER/E_CITATION_GHOST（章节级，scope 过滤）、E_EVIDENCE_EXPIRED（全局）、W_FORMAT_BASELINE（全局）；四桶分类 + NotPerformed 三态纪律完整。`is_drafted` 把 SectionStatus::Revising 视为已定稿（423-431）——与章节状态机一致。

**发现引用**：PA-001（副作用定位）、PA-005（evidence 桶只查 Expired/Superseded，不看 verification_tier）。

### 2.6 src/schema.rs（158 行）

RunDb/TraceEntry 等 serde 契约（与 bid-grade 共享编译期类型）。纯数据，无状态语义。READ。

### 2.7 src/fs_fence.rs（294 行）

**职责**：#183/#189 导入导出共用文件围栏。

**公开符号**：`MAX_FILE_BYTES`(500MB)、`MAX_BATCH_FILES`(64)、`MAX_TOTAL_BYTES`(2GB)、`WorkspaceFileRef{absolute,root}`、`is_inside`、`resolve_read`、`resolve_write`。

**语义**：词法预拒 `..`（先于磁盘操作）→ 锚定（相对=roots[0]）→ lenient canonicalize → 组件级包含（修 #189 前缀陷阱，Windows 折叠大小写/Unix 精确）→ **用户拼写段逐段 symlink_metadata 检查**（围栏内 symlink 一律拒，含 Windows reparse/junction）。符号链接绕过面：**词法与已解析双重检查下无直接绕过**；登记在案的残留是 check-then-use TOCTOU 窗口（PA-010，模块头 38-46 行自认，升级路径已注明 openat+O_NOFOLLOW）。相对路径仅锚 roots[0]（多根场景第二根不可相对引用，防御性行为，非漏洞）。

### 2.8 src/lib.rs（26 行）

模块声明 + re-export（run_scenario/write_run/RunOutput、import_files/BidService/ImportReport/ImportSpec、StoreFile/StoreIoError）。READ。

### 2.9 src/context.rs（494 行）

**职责**：S-7 单章输入组装（约束前缀+章节规格+证据块 → prompt 文本）。

**公开符号**：`ConstraintSource`/`BuiltinConstraintSource`、`EvidenceTextSource`/`LedgerTextSource`/`ChunkTextSource`、`RelevanceOracle`/`BindingOrderRelevance`/`KbRelevance`、`locator_brief`、`SectionInput`、`ContextAssembleError`、`assemble_section_input`(+`_with`)。

**语义**：只读 store + 确定性渲染 + 渲染后 token 实测红线（fail-closed）。孤儿绑定拒绝组装。**零状态副作用，无发现。**

---

## 3. #243 专项推演结论（分场景表）

推演基线：当前 main@6105beb 代码路径，MCP 工具面（`bid_mcp::dispatch` → tools.rs → BidService）。

| 场景 | 代码路径 | 副作用 | 恢复 |
|------|----------|--------|------|
| S1 部分章节时调用全局校验 | tools.rs:3470-3528（参数→route→guard_tool[Drafting 允许 validate]→check_version→仅查「章节树非空」3519-3523）→ service.rs:1372-1388 → validate.rs:156-421（纯读，返回 ok=false + E_SECTION_EMPTY BLOCKER 清单——**缺失章节清单其实在 findings 里**）→ service.rs:1379-1387：`Drafting => &[Validating]` 无条件 `advance_project_status`（lib.rs:601-617：status 变更 + content_version+1）→ auto_checkpoint 落盘 | **有**：项目状态 DRAFTING→VALIDATING、project_version +1、db.json 重写。审核记录不变（record_review 未触及）、导出状态不变。工具返回 `ok:false`（数据面）而非错误码——**无 DRAFT_INCOMPLETE 等价错误码，调用方极易当作「一次普通失败查询」继续编排** | 见 S3 |
| S2 校验失败（质量 BLOCKER，如 E_CHARS_BELOW_MIN）后修订 | Agent 调 `draft_response_section` → tools.rs:1895 `guard_tool` → lib.rs:224-239：**Validating 的 allowed_tools 不含 draft_response_section** → E_STATE（零副作用拒绝）。set_matrix_override/record_review_gate/validate/export 可用，但**没有任何工具能把状态搬回 Drafting** | **卡死（MCP 面）**：修订通道被状态门关闭。既有测试自证：writing_acceptance_e2e.rs:439-442 注释「VALIDATING 态 MCP draft 被状态机拒——服务面直调是**测试特权**」 | 无受支持路径 |
| S3 卡在 VALIDATING 后的出口枚举 | 前进：export（被三前置门拦：矩阵 blocking/审核门/写作验收门——空章节必拒 E_UNPROCESSABLE）→ 不通；record_review 只记账不推项目状态（review.rs:274-353 无 advance_project_status）；后退：`next_allowed(Validating)=[Reviewing]`（lib.rs:131）**无回退边**；仓内无 force_status/恢复工具生产代码（grep 全仓仅测试使用） | — | **唯一现实出路＝手工编辑 db.json 的 status 字段（#243 报告的恢复方式）——仓库未提供受支持恢复面。#243 在当前 main 仍然成立（CONFIRMED）** |
| S4 重试幂等 | 第二次 validate：guard 允许（Validating 列表含 validate）、steps=[]（service.rs:1381-1383 match 空）→ 无再推进、无版本 bump；同输入报告全等（validate_tool.rs:218-220 有测试） | 无（重复校验幂等 ✔） | — |
| S5 崩溃后重启一致性 | service.rs:1387 auto_checkpoint → store_io.rs:109-121 tmp+rename（进程崩溃原子）→ 重启 `BidService::open` 恢复 VALIDATING 态 | store 一致 ✔（但恢复出的是**卡死态**，S3）；断电窗口见 PA-006 | db.json 损坏 → open 拒载/全局态启动 panic（fail-closed，但无修复工具） |
| S6 并发 | 进程内：state.rs:16 `Mutex<BidService>` 全序列化（无并发交错）；进程间：无文件锁（PA-009）；乐观并发可选（project_version 缺省跳过） | 丢更新仅在跨进程/多宿主场景 | — |

**#243 一句话结论：校验本体（validate.rs）确无副作用，但服务层包装（service.rs:1379-1387）在校验后无条件推进 DRAFTING→VALIDATING 且状态机无回退边、allowed_tools 在 VALIDATING 拒绝起草——误触发即死锁，失败后无法在 MCP 面修订，唯一恢复是手改 db.json；issue 全部核心验收条件在当前 main 均不满足。**

---

## 4. 问题记录

```
ID: PA-001
严重程度: P0
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/service.rs
代码行号: 1372-1389（推进段 1379-1387）；佐证 bid-mcp/src/tools.rs:3513-3528、bid-domain/src/lib.rs:130/224-239/601-617
函数/结构体/接口: BidService::validate_document_with_reviewer / BidService::validate_document
问题描述: 全局校验在章节未完成时仍无条件推进项目状态 DRAFTING→VALIDATING（不校验起草完成度、不看报告 BLOCKER 结果、无 DRAFT_INCOMPLETE 等价门禁）。状态变更 + content_version+1 + checkpoint 落盘，违反 #243 验收「部分章节状态下调用全局校验：项目状态、版本均不发生变化」。
调用链: MCP validate_bid_document（tools.rs:3470 参数/route/guard/check_version → 3519 仅查章节树非空）→ BidService::validate_document_with_reviewer → validate::validate_document（纯读）→ store.advance_project_status(Validating) → auto_checkpoint → db.json
触发条件: 项目处于 DRAFTING（至少起草过一章）且仍有未起草/空章节，Agent 误调全局校验（#243 真实客户端日志场景）
实际影响: 项目被持久化锁进 VALIDATING；后续 draft_response_section 被 guard_tool E_STATE 拒绝（allowed_tools 无此项）；无回退边（next_allowed(Validating)=[Reviewing]）→ 起草死锁
复现步骤: ① create_bid_project→import→extract→build_outline→bind→draft 仅 1/N 章（项目 DRAFTING）② MCP 调 validate_bid_document{scope:"all"} → 响应 ok:false（含 E_SECTION_EMPTY findings）但 project_status 已变 VALIDATING、project_version+1 ③ 调 draft_response_section → E_STATE ④ 重启 → 状态仍 VALIDATING，步骤 ③ 同样被拒
当前测试: bid-mcp/tests/validate_tool.rs:199-221 把该推进断言为预期行为（「DRAFTING → VALIDATING 推进」）；无任何「部分章节校验不推进」的测试
缺失测试: 部分章节校验后断言 status/version/审核记录不变；DRAFT_INCOMPLETE 错误码+缺失章节清单；重启后状态不回退断言
建议方案: 在 service 推进段前增加完成度门禁（存在未 DRAFTED/空正文叶 → 返回 DRAFT_INCOMPLETE 类错误并携带缺失章节清单，零状态变更）；或把「推进 VALIDATING」改为独立显式动作/仅当报告 ok=true 且全叶 DRAFTED 时推进；同步修 MCP 契约文档
证据: service.rs:1352-1357「成功跑完推进项目到至少 VALIDATING（Drafting → Validating 一步——复审语义；已在 Validating+ 者 no-op）」；1379-1386:
    let cur = self.store().project()?.status;
    let steps: &[bid_domain::ProjectStatus] = match cur {
        bid_domain::ProjectStatus::Drafting => &[bid_domain::ProjectStatus::Validating],
        _ => &[],
    };
    for to in steps { self.store_mut().advance_project_status(*to)?; }
    self.auto_checkpoint()?;
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PA-002
严重程度: P0
模块: bid-tools（交叉 bid-domain/bid-mcp）
文件路径: bidcore/crates/bid-tools/src/service.rs + bidcore/crates/bid-domain/src/lib.rs + bidcore/crates/bid-mcp/src/tools.rs
代码行号: lib.rs:131（Validating=>&[Reviewing]，无回退）、lib.rs:224-239（allowed_tools）、tools.rs:1895（draft guard）、draft.rs:786-801（advance_project_to_drafting 对 Validating+ no-op——服务层与域层口径不一致）
函数/结构体/接口: ProjectStatus::next_allowed / allowed_tools / guard_tool
问题描述: VALIDATING 态无回退边、无受支持恢复路径：全稿校验失败后（#243 验收「仍可修订并再次校验」）MCP 面 revision 通道被 E_STATE 关闭；仓内无任何生产恢复工具（force_status 仅测试），唯一出路是手改 db.json——issue 明确禁止的方式
调用链: MCP draft_response_section → guard_tool → allowed_tools(Validating) 不含 → E_STATE（零副作用拒绝，死锁）
触发条件: 任何一次 validate 之后（无论误触发还是终校验失败）需要改稿/补写
实际影响: 修订-重校验-导出闭环在 MCP 面断裂；测试用「服务面直调」掩盖（见 PA-003）
复现步骤: ① 全稿起草完成 → validate（ok:false，E_CHARS_BELOW_MIN）→ 项目 VALIDATING ② MCP 调 draft_response_section 补写 → E_STATE ③ 枚举 Validating 态全部 allowed_tools：无任何可写正文的工具 ④ export 被 1c 写作验收门拒 → 双向死锁
当前测试: bid-mcp/tests/writing_acceptance_e2e.rs:439-443 修复闭环用 svc.draft_section 直调（注释自认「服务面直调是测试特权」）——即生产不可达路径
缺失测试: VALIDATING 态经 MCP 修订的通道测试；受支持恢复流程（恢复前后的状态不变量+审计记录）
建议方案: ① 语义分治：起草期校验（scope/局部）不推进状态；终校验推进 ② 给 VALIDATING 增加受控回退（如带审计记录的 explicit reopen，或把 drafting 窗口扩为 Drafting|Validating）③ 恢复工具需走域层显式迁移+审计，不做任意状态写入
证据: writing_acceptance_e2e.rs:439-442:
    // ---- 修复闭环：补写回带内（mock 重起草）→ validate 复绿
    //      （VALIDATING 态 MCP draft 被状态机拒——服务面直调是测试特权，
    //      合法窗口：advance_project_to_drafting 对 Drafting+ no-op）
    svc.draft_section(&victim, ...)
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PA-003
严重程度: P1
模块: bid-tools（测试有效性）
文件路径: bidcore/crates/bid-mcp/tests/validate_tool.rs + bidcore/crates/bid-mcp/tests/writing_acceptance_e2e.rs（范围外佐证，用于评估本试点 3 个测试文件缺口的补位情况）
代码行号: validate_tool.rs:217/242/554；writing_acceptance_e2e.rs:282/439-443
函数/结构体/接口: 测试断言面
问题描述: 现有测试把 #243 的问题行为锁死为预期（「DRAFTING → VALIDATING 推进」断言），且修复闭环场景走服务层直调绕过 MCP 状态门——测试绿≠生产可达。属「历史已修复但缺乏当前证据」的反面：行为从未修复，测试在固化它
触发条件: 回归测试运行
实际影响: 修 #243 时这些测试会反向阻挡（需同步改测试口径）；e2e「修复闭环」给出误导性的绿色信号
建议方案: 测试改为断言新契约（部分章节不推进+可修订重试），e2e 闭环改走 MCP 工具调用
证据: validate_tool.rs:217 assert_eq!(r["project_status"], "VALIDATING", "DRAFTING → VALIDATING 推进");
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PA-004
严重程度: P1
模块: bid-tools（#244）
文件路径: bidcore/crates/bid-tools/src/service.rs（+ bid-mcp/src/tools.rs:3618-3688 build_outline）
代码行号: service.rs:822-830（prepare_outline_and_matrix）；tools.rs:3641-3653（唯一前置=条款集非空）
函数/结构体/接口: BidService::prepare_outline_and_matrix / MCP build_outline
问题描述: 建纲入口无覆盖处置闭环门禁：不消费 coverage 状态（get_coverage_report 是独立只读工具），未判定块/疑似要求块不会阻断或警告建纲——#244 前提成立（「仅生成 coverage_report 或一条 warning 不足够」，当前连 warning 都没接进建纲）。domain allowed_tools 注释（lib.rs:182「覆盖不足时 build_outline 应告警」）为未实现的意向声明
调用链: MCP build_outline → route_guard → 前置（requirements.len()>0）→ prepare_outline_and_matrix（derive_and_set_outline→build_matrix→checkpoint）→ 直接进入 MATRIX_READY+
触发条件: 导入+抽取后存在未判定/未覆盖块即建纲
实际影响: 「仍可能含要求但未覆盖的块静默进入起草」——正是 #244 要堵的盲区
当前测试: 无建纲前覆盖门测试（coverage 测试均在 get_coverage_report/submit_block_marks 面）
缺失测试: 保留未判定块 → build_outline 被阻断并点名；补标重跑幂等
建议方案: prepare_outline_and_matrix 前置读 coverage 处置状态，区分「已排除（可过）/未判定（阻断+清单）」；排除项带原因可追溯
证据: tools.rs:3642-3652:
    let n_reqs = svc.store().requirements().len();
    if n_reqs == 0 { return Err(...E_STATE...) }   // ← 唯一门禁，无 coverage 检查
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PA-005
严重程度: P1
模块: bid-tools（#245）
文件路径: bidcore/crates/bid-tools/src/service.rs（+ bid-domain/src/matrix.rs:167-197、bid-domain/src/evidence.rs:205-245）
代码行号: service.rs:918-931（tier 裁决写入）；matrix.rs:174-187（has_ev=!evs.is_empty()，不看 tier）；validate.rs:327-343（evidence 桶只查 Expired/Superseded）
函数/结构体/接口: BidService::register_evidence_verified / VerificationTier / derive_matrix_rows
问题描述: 证据「内容核验状态」（VerificationTier: FILE_EXISTS/CONTENT_MATCHES/…）由服务端裁决写入台账，但**没有任何门禁消费它**——矩阵 blocking 只看绑定是否非空，validate 证据桶只看效期状态。FILE_EXISTS 级（正文从未定位读取）甚至占位内容的证据绑定后即把资格/否决行翻出 NotResponded → blocking 清零。#245 前提（占位资质绑定→阻塞归零）在当前 main 成立
调用链: MCP register_evidence → register_evidence_verified（tier 裁决+落库）→ bind_evidence → build_matrix（derive_matrix_rows 不读 tier）→ export 门 check_export_ready 只看 blocking
触发条件: 登记无 locator 的文件级锚证据（或占位内容文件）并绑定到必选行
实际影响: 「来源存在」被等价为「资格已满足」；导出门/矩阵把占位当真实证据放行
当前测试: evidence_content.rs 断言 tier 值本身（185/207/221 行），无「tier 不满足 → 矩阵仍阻塞」的测试
缺失测试: FILE_EXISTS 级证据绑定后矩阵仍显示未满足/待补；占位不静默清零 blocking；tier 不可经 MCP 参数自报
建议方案: derive_matrix_rows / 导出门消费 verification_tier（FILE_EXISTS → 行状态呈现「待补/受限」而非 Responded）；为占位材料增加显式持久化状态；保持 #182/#212 边界
证据: matrix.rs:174-179:
    let has_ev = !evs.is_empty();
    let has_sec = sec.is_some();
    let st = match (has_ev, has_sec) {
        (true, true) => ResponseStatus::Responded, ...
（全文件无 verification_tier 引用；grep 全仓 content_read() 仅测试调用）
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PA-006
严重程度: P2
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/store_io.rs
代码行号: 109-121（save）
函数/结构体/接口: StoreFile::save
问题描述: 原子写缺 fsync：write(tmp) → rename 前/后均无 File::sync_all 与目录 sync。进程崩溃安全（rename 原子替换），但断电/内核 panic 窗口内 rename 可能先于数据块落盘 → db.json 变空/半文件；随后 load 正确拒载（Corrupt），但服务不可用且无修复面
触发条件: 断电或 OS 崩溃恰落在 checkpoint 窗口
实际影响: 重启后 fail-closed 拒载（好于静默半状态），但项目中断需人工介入
当前测试: store_roundtrip.rs 无断电模拟（也无法在用户态直接测）
缺失测试: 模拟 kill -9 于 write/rename 之间（用户态可测进程崩溃面——预期已覆盖，因 rename 原子）；断电面只能标注
建议方案: tmp 写后 sync_all(tmp) + rename + 可选 sync 父目录（POSIX durability 惯例）；Windows 对应 FlushFileBuffers
证据: store_io.rs:117-119:
    let tmp = self.dir.join(format!("{DB_FILE}.tmp"));
    std::fs::write(&tmp, json.as_bytes()).map_err(StoreIoError::Io)?;
    std::fs::rename(&tmp, self.path()).map_err(StoreIoError::Io)?; // 原子提交点
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PA-007
严重程度: P2
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/service.rs
代码行号: 700-704（auto_checkpoint）；509-512（模块头自认语义）
函数/结构体/接口: BidService::auto_checkpoint
问题描述: checkpoint 落盘失败 → E_UNPROCESSABLE 返回调用方，但域状态已在内存变更且**无回滚/无重试**：后续任意成功 checkpoint 会把该状态落盘；若进程在此之前崩溃则磁盘停在旧态——调用方收到过 error 但状态「已消失」。半提交语义仅靠错误码提示「不得当作成功」
触发条件: 磁盘满/权限/AV 锁文件等 IO 失败落在任一写工具的 checkpoint 步
实际影响: 内存-磁盘分叉窗口；对 #243 类状态推进（advance 后落盘失败）会造成「报错但状态已变」的不一致体验
当前测试: 无 checkpoint 失败注入测试（只读）
缺失测试: 注入落盘失败 → 断言错误码/内存态/磁盘态三角关系与后续行为
建议方案: 失败时快照回滚（to_snapshot 在域调用前留底、失败恢复）或至少提供显式 reconcile；当前至少应在错误信息里给出状态分叉事实与后续动作指引
证据: service.rs:700-704:
    fn auto_checkpoint(&self) -> Result<(), ToolError> {
        self.checkpoint().map_err(|e| ToolError::EUnprocessable {
            reason: format!("checkpoint 落盘失败（内存已变更、磁盘未跟上——不得当作成功）: {e}"),
        })
    }
审计置信度: CONFIRMED
状态分类: 当前仍存在（代码注释自认，属已知权衡——但无测试无缓解）
```

```
ID: PA-008
严重程度: P2
模块: bid-tools（交叉 bid-mcp）
文件路径: bidcore/crates/bid-mcp/src/tools.rs（关联 service.rs checkpoint 时序）
代码行号: tools.rs:1911-1928（幂等键构造）、2074-2076（登记在最后一次 checkpoint 之后）、3416（export 同款）
函数/结构体/接口: draft_response_section / export_bid_document 的幂等登记
问题描述: ① 幂等记录写在业务 checkpoint 之后、且 record_idempotent 经 store_mut 不触发 checkpoint——崩溃后幂等表丢失，客户端超时重试会**重新执行**（真实 LLM 面=重新计费+内容重生成覆盖，非幂等重放）；② 注释称幂等键含 content_version（「(section_id, instructions_hash, content_version)」）但实现不含 version（`let _ = version;` 死变量自证）——同章节同指令的「合法重起草」请求会被静默 deduplicated:true 挡掉
触发条件: ① draft/export 成功后进程崩溃→客户端重试；② Agent 对同章节用完全相同 instructions 再次请求重新生成
实际影响: ① 重试非幂等（费用/内容漂移）；② 重新生成通道被误锁（需改 instructions 文本才解锁）
当前测试: 无崩溃后重试测试；无同指令重生成语义测试
缺失测试: checkpoint 时序断言（幂等记录落盘）；同指令二次调用行为契约测试
建议方案: record_idempotent 后补一次轻量 checkpoint（或并入业务事务）；幂等键纳入 content_version 或删除误导注释并在 out 中给「去重命中」的显式出口
证据: tools.rs:2074-2076:
    // 幂等登记（重放面——content_version 已含本次写入的 bump，重放不重执行）
    let _ = version;
    svc.store_mut().record_idempotent(ikey, out.clone());
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PA-009
严重程度: P2
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/store_io.rs + bidcore/crates/bid-mcp/src/state.rs + bid-domain/src/lib.rs:577-591
代码行号: store_io.rs:76-122（无锁）；state.rs:16（进程内 Mutex）；lib.rs:579-590（check_version Option）
函数/结构体/接口: StoreFile / GLOBAL / DomainStore::check_version
问题描述: 持久化无跨进程文件锁：同一 BID_WS 目录两个进程 open（或双 MCP 宿主）并发写 → last-writer-wins 丢更新；乐观并发为可选（project_version 缺省 None=跳过，MCP 客户端可不传）。进程内因 Mutex 全序列化无此问题
触发条件: 双进程/双宿主同工作区
实际影响: 交叉 checkpoint 互相覆盖（含 approval.key 场景下 receipt 验签键漂移）
建议方案: 目录级 lockfile（创建互斥）或打开时检测 PID 锁；契约上把 project_version 设为必传（破坏性变更需评估）
证据: state.rs:16: static GLOBAL: LazyLock<Mutex<BidService>> = ...（仅进程内）；store_io.rs 无任何锁原语
审计置信度: CONFIRMED（进程内安全、跨进程风险为设计缺口）
状态分类: 当前仍存在
```

```
ID: PA-010
严重程度: P3
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/fs_fence.rs
代码行号: 38-46（TOCTOU 自认）、200-252（resolve）、216-220（相对路径仅锚 roots[0]）
函数/结构体/接口: resolve/resolve_read/resolve_write
问题描述: ① check-then-use TOCTOU：围栏校验与实际 open/create 之间本地攻击者换 symlink 可绕过（模块头已登记，升级路径 openat+O_NOFOLLOW 已注明——零依赖约束下的已知取舍）；② 相对路径仅锚定 roots[0]，多受信根场景第二根不可相对引用（防御性，非漏洞）
触发条件: 本地对抗性环境（围栏内目录段在窗口内被替换为 symlink）
实际影响: MCP 面导入/导出路径逃逸的理论窗口（需本地写权限配合）
建议方案: 按模块头登记的升级路径实施 openat 逐段打开
审计置信度: CONFIRMED（登记在案的设计残留）
状态分类: 文档与实现不一致（文档诚实登记，实现为 v1 取舍）
```

```
ID: PA-011
严重程度: P3
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/runner.rs
代码行号: 348-362（write_run）
函数/结构体/接口: write_run
问题描述: 运行产物 db.json/trace.jsonl 直写（非 tmp+rename）——中断可留半产物；属审计/判分产物非服务态，与服务态 store_io 的原子纪律不对齐
建议方案: 复用 tmp+rename 惯例（低优先）
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

```
ID: PA-012
严重程度: P3
模块: bid-tools（交叉 bid-mcp）
文件路径: bidcore/crates/bid-mcp/src/state.rs
代码行号: 35-39（open 失败 panic）
函数/结构体/接口: open_global_service
问题描述: BID_WS 的 db.json 损坏 → 全局服务态构造 panic → stdio 宿主进程死亡。fail-fast 优于静默内存降级（注释明确），但对宿主表现为连接崩溃而非可操作的 MCP 错误；无诊断/修复指引面
建议方案: 宿主层捕获并提供显式恢复流程（备份损坏件+重建），或 MCP 增加管理面错误通道
审计置信度: CONFIRMED
状态分类: 当前仍存在（有意设计，登记风险面）
```

```
ID: PA-013
严重程度: P3
模块: bid-tools
文件路径: bidcore/crates/bid-tools/src/service.rs（关联 bid-domain/src/lib.rs:1052）
代码行号: 无界 HashMap（lib.rs:419/519-524）随快照序列化（1052/1086）
函数/结构体/接口: DomainStore::idempotency
问题描述: 幂等表无界增长且整体入 db.json（draft/export 每次调用都追加 JSON out 全文）——长项目 db.json 单调膨胀，checkpoint 成本随之上升
建议方案: 幂等键 TTL/容量上限，或值改为摘要+必要回显字段
审计置信度: CONFIRMED
状态分类: 当前仍存在
```

正面确认（非问题）：`check.rs`/`context.rs`/`validate.rs` 本体零副作用；`fs_fence` 前缀陷阱与 symlink 双查设计正确；`store_io` 损坏 fail-closed + 加性迁移 + 字节级确定性；导入 R-01 逐件失败不整批中止；#188 异身份 FileRole；#182 receipt 四重签域；重复导入/校验/导出幂等（导入 sha no-op 不 bump 版本、校验复跑零推进、导出 EXPORTED 重入+manifest/PDF marker 幂等）。

---

## 5. 测试有效性评估（3 个指定测试文件）

### tests/store_roundtrip.rs（308 行）
- **类型**：INTEGRATION（真实 fixtures + 真实磁盘 + 双 BidService 生命周期模拟进程重启；磁盘为唯一传递介质，内存态零参与——设计正确）。
- **Oracle 来源**：独立于实现的往返不变量（快照全等、字节一致、id 水位、跨重启幂等）+ 显式期望值（status==REQUIREMENTS_EXTRACTED、files.len 等）。未发现假绿。
- **错误路径**：损坏 db.json 拒载路径真实可达（手写坏 JSON → open → Corrupt）。
- **缺口**：未覆盖 #243（无 VALIDATING 卡死/恢复场景）；无断电/半写模拟（用户态不可测断电，kill -9 面也未测）；无并发（跨进程锁缺测试）；`store_mut()` 直调域 API（绕过服务包装——文档自认「工具面 V3-6 接线」，测试真实性轻微打折但被测面仍是持久化本身）；fixture 缺失时 SKIP 横幅（无 fixture 环境测试为空转——诚实但意味着覆盖依赖语料在位）。
- **假绿嫌疑**：低。

### tests/service_import.rs（123 行）
- **类型**：INTEGRATION（真实 corpus-A 文件走 `import_files` 管线；含 BidService::import 围栏面）。
- **Oracle 来源**：硬口径独立期望（tables≥5 来自 corpus_probe 基线、content_version==2 明确计数、chunks>0、sha256 累积面==落库面）——独立于实现内部，非自证。
- **错误路径**：不可读 upload_ref 真实触发（ghost 路径），断言无空 sha 落库、批次不中止——真实可达，非 mock。
- **缺口**：不覆盖 #243/#244/#245；无围栏越界路径（围栏测试在别处）；SKIP 机制同上。
- **假绿嫌疑**：低。

### tests/input_integrity_gate.rs（379 行）
- **类型**：UNIT（内存态手工构造 DomainStore，测 readiness/input_integrity/check_draft_readiness 判定面）。
- **Oracle 来源**：枚举变体匹配 + 部分断言耦合实现文案（`msg.contains("不等于输入完整")`、`contains("record_addendum_waiver")`）——文案即产品契约面，可接受但改文案会红。
- **错误路径**：真实验证阻断/放行/受限三态与不相交性；`TrustedPrincipal::human` 直调域层（绕过 receipt 验签——单元测试合理分层）。
- **缺口**：与 #243 无关（本文件管导入件完整性，不管校验/状态）；未覆盖 MCP 面的 RESTRICTED 回显链。
- **假绿嫌疑**：低（但断言面偏实现文案，重构脆）。

**三文件与 #243 的关系**：均不覆盖。#243 相关的测试在 bid-mcp/tests（validate_tool.rs / writing_acceptance_e2e.rs / e2e_pipeline.rs），其中 validate_tool.rs **锁定问题行为**（PA-003）、writing_acceptance_e2e.rs 的修复闭环走**生产不可达的服务面直调**——这是当前测试体系对 #243 最大的真实性缺口：绿灯测试演示的恢复流程，Agent 在 MCP 面根本走不通。

---

## 6. 结论摘要

1. **#243 CONFIRMED（当前 main 仍存在，P0×2）**：校验本体纯只读，但 service 包装无条件推进 VALIDATING + 状态机无回退 + allowed_tools 关闭修订通道 + 无恢复工具 → 误触发即持久化死锁，唯一出路手改 db.json。
2. **#244 前提成立（P1）**：build_outline 无覆盖处置门禁（仅查条款非空）。
3. **#245 前提成立（P1）**：VerificationTier 只写不读，占位/未验真证据绑定即清 blocking。
4. 持久化链路：tmp+rename 原子性正确、损坏 fail-closed 正确；缺 fsync（P2）、缺跨进程锁（P2）、checkpoint 失败半提交（P2）、幂等登记不落盘（P2）。
5. 测试真实性：3 个指定文件均无假绿嫌疑，但 #243 场景零覆盖，且 bid-mcp 侧现有断言在固化问题行为。
