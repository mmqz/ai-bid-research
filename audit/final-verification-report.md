# ai-bid-research 全仓代码覆盖审计——最终验证报告

- 审计基线：`main @ 6105beb652e88760bdf38f08080af1ed7b1f048a`（冻结，PR #242 合并点，2026-11-04 21:25 +0800）
- 审计分支：`audit/phase-1`（只新增审计产物，生产代码零修改；全程 `git status` 闭环验证）
- 审计性质：全仓第一方代码逐文件审计 + 全模块调用链审计 + 测试有效性审计 + 业务链路验证 + 实际运行全部测试套件 + 真实语料重测 + 定向变异测试
- 报告日期：2026-11-06

---

## 一、覆盖率声明（§十二 完成标准核对）

| 标准 | 结果 |
|---|---|
| 100% Git 跟踪文件已分类 | ✅ **1,737/1,737**（full-source-inventory.csv，13 类分类，无未分类文件） |
| 100% 第一方代码文件已阅读审查 | ✅ **366 个代码类文件全部 REVIEWED**（bidcore 47 src + 测试源 + web-ui/mang/m1/scripts 84 + sdk 21 + CI/构建配置；四 Agent 逐文件覆盖表在 module-audit-A/B/C/D.md） |
| 所有被排除文件有明确原因 | ✅（THIRD_PARTY=56 research/raw 竞品抓取、EVAL_OUTPUT=807、EVAL_BASELINE=139、REAL_CORPUS=70、FIXTURE=69 等均登记用途，关键面抽样核验） |
| 所有独立 workspace 已纳入 | ✅ 17 个 Cargo.toml 全盘点（bidcore 8 成员 / mang-app 独立 / m1 / scripts 六工程）+ web-ui + Python 面 |
| 所有测试源码已纳入 | ✅（~1,123 Rust 测试 + Python T0-T19 + SDK 10 用例 + mcp_sdk_e2e） |
| 100% 对外公开 API 有清单 | ✅ public-api-inventory.csv（25 MCP 工具逐个契约对账 + 8 crate 924 pub items 统计） |
| 100% MCP 工具契约对账 | ✅ module-audit-C 工具对账大表（发现：契约漏收 set_matrix_override、工具数 21/24/25 三版本漂移） |
| 100% SDK 公开方法真实接口检查 | ✅（minimal_flow.py 已废除参数 + 四门时序错误 + project_version 取位错） |
| 关键业务链生产代码端到端追踪 | ✅ callgraph-audit.md 六链逐跳（**4 BROKEN / 2 DEGRADED**） |
| P0/P1 不变量有效测试守卫 | ✅ mutation-report.md（6 变异、5 直接击杀、1 迟杀；M1 揭示 #243 无条件推进无测试=变异学盲区） |
| 真实格式独立样本 | ✅ corpus-verification.md（16 件真实水利语料冻结基线重跑；9/16 字节级复原、7/16 内容级） |
| PASS/FAIL/SKIP/IGNORED/NOT_EXECUTED/BLOCKED 分开统计 | ✅ test-quality-audit.md（bidcore 1,118/0；run_tests 51/3/3；SDK 9/10；mang-app NOT_EXECUTED+BLOCKED；scripts 六工程 0 测试） |

**明确声明：代码覆盖是审计完整性要求，不等于正确性结论。正确性由调用链、独立测试、变异与真实语料证据裁决——以下为裁决结果。**

---

## 二、执行摘要

这是一个**域层设计质量高、测试纪律诚实、但存在系统性「接线缺口」**的系统：不变量在 bid-domain 集中且被变异测试证实有守卫（6 变异 5 杀）；SKIP 语义不伪装失败（A-02 扫描件缺 chi_sim 时响亮报错）；locator 回查两轮基线 100%（316/316）；崩溃率 0。但**六条业务链四条 BROKEN**——根因高度同构：「域规则已实现、生产链未接线」孤岛共 6 处（RevisionTask 生命周期、ExportGate 时点、check.rs 金额冲突、param 负偏离、LinkageLedger、ImpactEngine），加上两个已实证的 P0（校验死锁 #243、占位证据 #245）与暗标产品面缺失（R-22），构成当前主要风险面。测试体系的最大问题不是假绿机制，而是**「哪些测试从未被谁跑过」无账本**：mang-app 39 测试零执行且必红、Python 主链与 SDK 旗舰 E2E 无 CI 监视已腐化（T18a 自 B4 红、SDK 02 自 #242 红）、CI 的 SKIP 自证步骤自身吞退出码。真实语料重测显示否决召回 0.386→**0.700**（同口径同 GT，改善确凿）但距硬门 100% 仍远。**总体裁定：具备继续迭代的质量基础，但下一阶段必须先做「接线批」修复与 CI 重建，再扩展新能力。**

---

## 三、问题统计与最严重问题

登记册（risk-register.md）：**P0×7 条目（含 2 个实证 P0 + 5 项归并入口）、P1×21、P2×14、P3×2**；假绿登记 FG-01…FG-18 三族。

### 最严重的三个实现问题（回答最终问题 2）

**① P0-1（#243）：校验死锁族——validate 无条件推进 + 无回退路径（R-01）**
`validate_bid_document` 无条件推进 `Drafting→Validating`（service.rs:1372-1386，不查校验结果）；`Validating` 态 allowed_tools 不含 `draft_response_section`（lib.rs:224）；状态机只进不退（lib.rs:601-617）。起草期误调一次全局校验=项目永久卡死，唯一恢复=手改 db.json。同族放大面：RevisionTask 无任何工具出口（A-23，拒审即派生 RT 且 export_allowed 永久拒绝）、Skill 恢复协议在 VALIDATING 态引导跳过未完成章节（C-29）。变异学证明：无任何测试约束「失败校验不得困死」（M1 语义缺口=FG-10）。

**② P0-2（#245）：占位证据清零 blocking——矩阵不消费验证层级（R-02）**
矩阵行状态派生只看绑定存在（matrix.rs:176-192），`verification_tier` 全仓下游消费为零（A-14）；`bind_evidence` 只查 can_bind 不查内容匹配（evidence.rs:608-659）。Agent 用 register_evidence 自报语义属性（title/subject/kind 全由调用方给）指向**任意已导入文件（含招标文件本身）**的 FILE_EXISTS 级证据，批量绑定资格条件即可让 blocking 归零（C-23 完整攻击链）；SKILL P4 判据「blocking=0」即被骗达（FG-17）。导出门 1a 消费构建时点存量矩阵，过期证据绑定照样放行（A-01/A-02——apply_validity 无 MCP 入口）。

**③ R-22：暗标产品面缺失（废标级）**
threat-model T-14 声称「docProps 清零+禁字扫描 TESTED（业务侧）」——实际只在 Python SEMUL 仿真线（v1_darkbid.py:120-126）；产品导出路径（docx-rs 写面）**无 docProps 清零、无全部件禁字扫描**，validate 的 dark_bid 档显式降级 standard（tools.rs:3484-3488），回读只查 word/document.xml（export_readback.rs:418/766-771）。公司名残留于文档属性/页眉页脚部件=暗标识别点——业务后果是废标。

### 其余 P1 要点（完整清单见 risk-register.md）

- 导出门 1a 与证据存活性脱钩（A-01/A-02，换版/过期后仍可过闸）
- 逐件 waiver 与「本项目无补遗」全局门共用同一 Gate 记录（A-04，对图纸做 waiver 会静默确认无补遗）
- merge 面 requirement 黑洞（B-01：Agent 标 requirement 未 submit 无告警）+ 文件名前缀过滤真实文件不进合并（B-02，#196 只修了抽取分发）
- ScoringMapping 变更的审核失效面=**空集**（business-chain-audit 特别复核项：upsert 只 bump content_version，sections_digest 显式排除评分面，export_allowed 唯一判据是 digest——评分规则变了旧审核照旧有效）
- Skill 示例/恢复协议缺陷族（C-02/C-29：minimal_flow 用已废除参数+四门时序错、文档却称实测通过）
- 工具面 guard 窗口与契约不符（C 组：register/bind 仅 MATRIX_READY|EVIDENCE_BOUND，契约承诺 MATRIX_READY+）
- web-ui XSS 可达 IPC 面（D-01，md 预览 href 未转义+CSP unsafe-inline）+ typecheck 开箱 33 错（@codemirror 未声明依赖）
- mang-app diag 断言过期+39 测试零执行（D-11/FG-01）
- 评测不可重现族（D-20/D-21：6 处仓外绝对路径必炸；GT 锚定自家解析产物半自证）

---

## 四、七个最终问题的回答（§十二）

**1. 当前代码是否具备继续迭代的质量基础？**
**是，有条件。** 正面证据：域层不变量集中且被 6 变异中 5 个击杀证实有守卫；1,118 Rust 测试全绿且真实执行生产代码；SKIP/诚实失败纪律上乘（扫描件零产出响亮报错）；locator 回查 100%；崩溃 0；否决召回一轮迭代 +81%。条件：先偿还「接线债」（六孤岛+两 P0+暗标面）并重建 CI——否则新能力会继续堆在断链上。

**2. 最严重的三个实现问题？** 见上节：校验死锁族（#243+RT）、占位证据清零 blocking（#245）、暗标产品面缺失。

**3. 哪些模块值得继续扩展？**
- **bid-domain**：不变量与守卫最强（M2/M3/M5/M6 全杀），扩展首选——但补「自层守卫」（M6 迟杀教训）。
- **bid-parse + bid-engine 接线面**：fail-loud 纪律好、真实召回改善确凿（0.70）、引擎边界纪律（唯一接触面）真实成立；扩展前先补 B-01/B-02 合并面与跨格式对称三件套。
- **validate/quality/export 门族**：#242 门经 M4+SDK 红双向证实真实生效——门族骨架可信，扩展时把 1a 改导出时点复算。

**4. 哪些模块需要先重构？**
- **service 接线层**（bid-tools/service.rs）：六孤岛全是「域层有了、服务层没接」——一次「接线批」PR 可整体解决（known-issue-verification 建议与 P0-1/#244/tier 合并）。
- **mang-app**：独立 workspace+CI 只 check+断言过期——并入 workspace 或加 test step，修 13→25。
- **scripts 评测链**：SEMUL 无交叉验证（FG-06）、T18a 断裂（FG-04）、不可重现（FG-16）、历史产物断言（FG-07）——需要一次「评测链重建」而不是修补。
- **web-ui**：无测试框架+typecheck 红——先达到「typecheck 可绿」再谈扩展。

**5. 哪些功能重复实现或选型不合理？**
m1（zip/quick-xml 自解析，遗留研究线，生产链已不用但 run_tests T10-T13 仍测它）；Python SEMUL 链 vs Rust 生产（无差分验证）；veto_lexicon 在 harness 的镜像复制（bid_health_check STRONG_VETO_PHRASES——两处漂移以 extract.rs 为准的口头纪律无探测器）；两套否决 GT 口径（1006 词表 vs 1787 增补）并存未回填；工具数 21/24/25 与 E_NOTFOUND/E_NOT_FOUND 漂移四处并存；engine_probe 钉旧 rev 对账（3204342d 落后 117 提交）。

**6. 当前测试体系最大的盲区是什么？**
**「哪些测试从未被谁跑过」没有账本**（FG 三族的公共根因）：mang-app 39 测试任何 CI 从未执行且当前必红；Windows CI 上真件链全 SKIP 恒绿（主解析通道回归防线为零）；「Verify SKIP banner」自证步骤 pwsh 管道吞退出码（cargo test 失败也不红）；Python 主链/SDK E2E 完全无 CI——T18a 与 SDK 02 两个旗舰验收各自腐化数周无人知。SKIP 机制本身诚实，缺的是执行账本与 Linux 真件 job。

**7. 哪些已有功能只是「接口存在」，实际尚未形成完整业务能力？**
（business-chain-audit 不变量矩阵 + A 组孤岛清单）RevisionTask 生命周期（派生了但无 assign/advance/complete 出口）；ExportGate（域对象在、门时点错）；LinkageLedger；ImpactEngine（补遗影响面）；check.rs 金额冲突检测；param 负偏离；ScoringMapping 的审核失效联动（映射本身 step1 在位且份额守恒，但变更不触发任何审核失效）；rules_profile=dark_bid（占位告警）；OCR 后处理链（chi_sim 缺失时 e2e 不红不 SKIP 的覆盖空洞 FG-11）；pdfium 引擎通道（ENV_MISSING 全程回退+T18a 断裂=引擎路径整体未验证）。

**8. 下一阶段优先级如何排列？**

| 批次 | 内容 | 依据 |
|---|---|---|
| **P0 接线批**（先于一切新功能） | ① validate 条件推进+#243 恢复路径（main 已有修复——**先验证 main 修复再决定**）；② verification_tier 进 bind+矩阵派生+#245 占位语义；③ 建纲前覆盖闭环（#244：report/coverage_complete 进 build_outline 前置）；④ RT 出口或延迟派生；⑤ 导出门 1a 改导出时点复算；⑥ 产品面 docProps 清零+禁字扫描（移植 v1_darkbid 逻辑）；⑦ waiver 全局门与逐件门分离 | R-01/R-02/R-22/#244/A-23/A-04/A-01 |
| **P1 CI 重建批** | Linux job（poppler/tesseract/chi_sim/reportlab 装齐——一次解 FG-02/FG-14/B-14）；mang-app 加 test；「SKIP banner」步骤修退出码；run_tests.py 精简版+SDK e2e 进 CI；clippy/fmt 门 | FG-01/02/03/05、ci-audit G1-G11 |
| **P1 评测链修复批** | T18a 改走 bid-engine 门面；MDCORE_REV 改 build.rs 注入+对账断言；engine_probe rev 同步；E_NOTFOUND 线格式统一+一致性测试；SEMUL↔Rust 差分测试；GT 口径统一回填 | FG-04/08/09/12、D-20/21 |
| **P2 合并面与跨格式对称** | B-01 requirement 黑洞告警；B-02 文件名分发与合并面对齐；PDF/DOCX 否决词表同源；as_chapter 认阿拉伯章号；PDF tables 装配 | module-audit-B |
| **P2 前端与桌面** | web-ui typecheck 修绿+测试框架；D-01 XSS；mang-app mcp.status 语义澄清 | module-audit-D |
| **P3 卫生** | T3a 游离夹具入清单；测试非封闭性（产物覆写/lock 重写/游离文件）；SDK TaskGroup 可诊断性；env_preflight 测试环境假设 | test-quality-audit §3 |

**注**：审计期间 main 已推进 79 提交（含 #243/#245 修复与 #252/#253/#269 等）。按冻结纪律本报告全部结论针对 6105beb；main 上两 P0 的修复**未经验证**——建议作为接线批第 0 步做针对性复核（修复 commit 范围内重跑本审计的 M1/M2 变异与 validate/SDK 用例即可）。

---

## 五、模块裁决表（A 保持扩展 / B 局部重构 / C 暂停+结构重构 / D 换成熟实现）

| 模块 | 裁决 | 理由（证据） |
|---|---|---|
| bid-domain | **A**（补自层守卫） | 30 项不变量矩阵过半强守卫；4 变异击杀；ScoringMapping 份额守恒在位 |
| bid-parse | **A-**（先修合并面） | fail-loud 纪律+召回 0.70 改善；但 B-01/B-02/跨格式三件套需先修 |
| bid-kb | **A** | 小而正确（9 测试+双通道），无 P1 发现 |
| bid-tools | **B**（接线批重构） | 六孤岛+两 P0 全在服务层接线面；域调用本身质量好 |
| bid-grade | **A** | 规则诊断/评标边界诚实（notes 显式登记不伪装） |
| bid-engine | **A-** | 唯一接触面纪律成立、三包同 rev；MDCORE_REV 常量+probe 钉旧 rev 需修 |
| bid-mcp | **B**（契约对账） | 25 工具一致但契约漂移三处+guard 窗口与契约不符+E_NOT_FOUND 漂移 |
| mang-app | **C**（先修门禁再动） | 39 测试零执行+必红断言+XSS 面；结构无大错但无防线 |
| m1 | **C→退役** | 生产链不消费；保留为研究线（run_tests T10-T13 明确标注并行线语义） |
| web-ui | **C**（先修 typecheck+XSS） | 无测试框架、33 typecheck 错、关键状态同步缺陷族 |
| scripts 评测链 | **C**（重建） | FG-04/06/07/16/17 + D-20/21 不可重现族 |
| SDK/Skill | **B** | 真进程验收强度高（非 Mock）但示例断链+恢复协议教错 |
| mdcore（引擎仓，R7 边界） | **A**（外部依赖） | 三包同钉 ✅；EOCD 守恒族等价性 NEEDS_ENV（zip 4.6.1→8.6 语义差异）；117 提交未消费含读面 panic 防御 |

---

## 六、测试可信度最终裁定（回答审计问 4/5）

- **Rust 1,118 测试：真实执行生产代码，可信**（SEMUL 无混入 Rust 侧；NEGATIVE 族质量高）。
- **run_tests.py 51/3/3：FAIL 诚实**（T18a/T3a 真红、T19a 环境红如实计败）；SKIP 语义上乘；但 T7-T9 的「全绿」是 SEMUL 绿（FG-06），T17 是历史产物断言（FG-07）。
- **评测可信度：部分可信**。否决召回的 Oracle（LingYan 20 词）独立于生产算法 ✅；但 real-eval GT 半自证（D-21）、两套 GT 口径并存、评测不可重现（FG-16）——**「评测结果可信」的结论仅对 health-check 口径成立**。
- **变异：P0/P1 不变量 5/6 击杀**——门族真实有牙。
- **假绿面：FG-01…18**（详见 false-green-register.md），三族分类：从未执行 / 断言对象错 / 语义误读。

---

## 七、交付物清单（audit/ 目录，分支 audit/phase-1）

baseline.md、full-source-inventory.csv（1,737 行×18 列）、workspace-inventory.md、dependency-graph.md、public-api-inventory.csv、module-audit-A/B/C/D.md（四模块逐文件审计+124+条发现）、callgraph-audit.md（六链）、business-chain-audit.md（30 不变量矩阵）、known-issue-verification.md（12 issue 复核）、test-quality-audit.md、test-to-production-map.csv（150 行）、false-green-register.md（FG-01…18）、corpus-verification.md（真实语料重测 0.386→0.700）、security-audit.md（十威胁面）、integration-contract-audit.md（R7）、mutation-report.md（6 变异）、ci-audit.md、risk-register.md（R-01…R-34）、final-verification-report.md（本文件）。

**复现要点**：Rust 工具链 stable 1.99.1；`cd bidcore && cargo test --workspace --no-fail-fast`；`python3 scripts/run_tests.py`；`BID_MCP_TARGET_DIR=<bidcore/target> python3 sdk/tests/test_mcp_e2e.py`；健康检查：重建语料（provenance.json + power_grid@da58e64，7 件 .doc 需 LibreOffice 重转换）后 `cargo run --bin bid-health-check -- --corpus … --out …`。变异测试方法见 mutation-report.md（每轮 git checkout 回滚+纯净验证）。
