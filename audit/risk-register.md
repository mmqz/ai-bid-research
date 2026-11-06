# 风险登记册（risk-register.md）——V2 全仓审计合并版

基线 6105beb（冻结）。本册 = 四份模块审计（A/B/C/D）+ Lead 两个 P0 + 安全/契约/假绿/CI 四份整合交付的合并登记。
编号：**R-01…R-34 重新编号**（仓库 research/risk-register.md 的 R-01~R-17 为设计期登记册，语义不同——交叉引用列保留映射；新发现以「新增」标注）。
状态词典：存在 / 已修复验证 / 已修复缺证据 / 文档不一致 / 无法复现 / 需环境（NEEDS_ENV）。

## P0 / P1（阻断与重要正确性）

| 编号 | 级别 | 状态 | 模块 | 一句话 | 证据引用 |
|---|---|---|---|---|---|
| **R-01** | P0 | 存在（修复在基线后 main，未验证） | bid-tools/bid-mcp | validate_bid_document 无条件推进 Drafting→Validating，Validating 态禁起草+状态只进不退 → 起草期误调校验即永久卡死（唯一恢复=手改 db.json） | P0-1/#243：service.rs:1372-1386、lib.rs:224、lib.rs:601-617；变异 M1（反向无测试）；编排放大 C-04/C-29 |
| **R-02** | P0 | 存在（修复在基线后 main，未验证） | bid-domain | 矩阵行派生只看绑定存在不消费 verification_tier/内容匹配——FILE_EXISTS 占位证据（可指招标文件本身）绑资格条件即清零 blocking；证据语义属性全自报、CONTENT_MATCHES 不防语义伪造 | P0-2/#245：matrix.rs:176-192、evidence.rs:608-659；C-23 攻击链（tools.rs:3784-3880）；A-14（tier 下游消费为零）；A-01/A-02 导出面残留 |
| **R-03** | P1 | 存在 | bid-tools/bid-mcp | RevisionTask 生命周期 MCP 面零接线——record_review 带 Blocker（拒审语义甚至要求必带）即派生 Open RT → export_allowed 恒拒且无工具可关单 → 永久导出死锁 | A-23：review.rs:494-583 域方法完备、tools.rs 零命中；同族 set_param_responses/batch_task/reopen_section 均无 MCP 面 |
| **R-04** | P1 | 存在 | bid-domain/bid-tools | 导出门消费存量矩阵不过滤失效证据+换版/时间过期无重算入口——失效证据响应可过导出闸门（用户矩阵视图与导出门双口径分裂） | A-01：matrix.rs:218-252 vs :347-356、service.rs:1178-1181；无「换版后 export_blockers」测试 |
| **R-05** | P1 | 存在 | bid-domain | 证据有效期时间翻转 apply_validity 全仓仅测试调用——时间性过期证据长期被当 Active 消费（readiness 放行起草/quality 收录/context 进 prompt） | A-02：evidence.rs:575-602；grep 生产零调用、tests/ 手工充当调用方 |
| **R-06** | P1 | 存在 | bid-domain | 审批 receipt 只绑门名不绑内容摘要——24h 窗口内改章节内容后持旧 receipt 过门（人批的≠过门的）；对比矩阵覆写四重签域明显未达同一标准 | A-03：principal.rs:223-289（消息无 digest）、review.rs:373-415；无「同一 receipt 审不同内容」攻击样本测试 |
| **R-07** | P1 | 存在 | bid-domain/bid-tools | 逐件 waiver 与「本项目无补遗」全局门共用同一 Gate——对无关件做 waiver 静默解除「未发现补遗」阻塞 | A-04：lib.rs:734-762、readiness.rs:1062-1076 |
| **R-08** | P1 | 存在 | bid-domain | restore 校验漏 evidence_bindings/matrix/matrix_overrides/block_marks——孤儿绑定制造幽灵「已响应」 | A-05：lib.rs:1100-1401（对照 #224 给 mapping 补的三查） |
| **R-09** | P1 | 存在 | bid-parse | Agent 标 requirement 未随附 submitted 时正则 REQ 被丢、无 REQ-A 生成——条款无告警地从矩阵消失（端态反被测试钉为预期） | B-01：merge.rs:133-189、coverage.rs:192-218；`covered_block_regex_dropped_any_verdict` |
| **R-10** | P1 | 存在 | bid-mcp/bid-parse | merge 面按文件名前缀（A-/C-/B1/B2）过滤——真实文件名文件的 Agent 标注整体失效、UNCOVERED_BLOCKS 告警失真（#196 修了抽取分发没修合并面） | B-02：tools.rs:724-741、merge.rs:71-76 |
| **R-11** | P1 | 存在（引擎侧 NEEDS_ENV） | bid-parse/bid-engine/mdcore | 历史 ZIP 炸弹门链（EOCD 守恒/影子 EOCD/尾部垃圾/压缩比/条目名五类/嵌套载具）随 B4 删除，引擎三门+限额族在位但 EOCD 守恒族无等价实现、zip 4.6.1→8.6 语义差未验证；threat-model T-03 整段指向已删除代码 | B-03 + security-audit §2：guard.rs 现况、io.rs 门 0-3、engine-bid-mapping §4「ratio 门引擎无」；对账工具钉旧 rev（B-17）；NEEDS_ENV 变异项 NE-1..NE-3 |
| **R-12** | P1 | 存在 | bid-parse/bid-engine | PDF `tables` 计数恒 0（两处实现 collapse_spaces 与「保留双空格」注释直接相反）——死指标，消费方恒假阴性（健康检查 tables=0 根因） | B-04：poppler.rs:105-108/:124-139、mapping.rs:427-430/:452-467 |
| **R-13** | P1 | 存在 | bid-mcp/sdk | 错误码线格式漂移 E_NOTFOUND vs E_NOT_FOUND——契约承诺稳定字面量被 serde 打破，SDK 测试把漂移固化为断言（缺陷被测试洗白） | C-01/C-28：error.rs:12-14、protocol.rs:538-546、test_mcp_e2e.py:1099/1105 |
| **R-14** | P1 | 存在 | sdk/examples | 官方示例 minimal_flow.py 用已废除 reviewer 参数+起草前过门时序错误——示例链路必死且方向性错误教学（文档声称「实测全链通过」失真） | C-02：minimal_flow.py:222-228、tools.rs:4016-4023、agent-integration.md:369-371 |
| **R-15** | P1 | 存在 | sdk/skill | Skill 对 set_matrix_override 指引自相矛盾且与实现不符（p3-matrix 判不存在/p2 承认存在/契约 0 次）——否决项死锁唯一出口在编排面分裂 | C-03/C-30：p3-matrix.md:122、p2-extract.md:43、contract.md rg=0 |
| **R-16** | P1 | 存在 | bid-domain/sdk | register/bind/import/extract 的 guard 窗口仅 MatrixReady/EvidenceBound vs 契约「MATRIX_READY+」——起草后补证据与 Skill 补遗恢复链路全断 | C-05：lib.rs:187-205、contract.md:451/482；C-12 scope 参数叠加 |
| **R-17** | P1 | 存在 | sdk/skill | p7-review 教「MAJOR 回 P5 重提」在基线不可行（Validating 态 E_STATE）+恢复协议引导跳过未完成章节——P0-1 的编排放大面 | C-29/C-04：p7-review.md:98-101、SKILL.md:82-91 |
| **R-18** | P1 | 存在 | web-ui | 浏览器 E2E 基础设施缺口——无测试框架，注释引用的 W1-W13/web_smoke.py 不在仓库，性能/等价性护栏纯靠注释声称 | D-06：package.json 仅 build/serve/typecheck；全仓 grep 无 web_smoke |
| **R-19** | P1 | 存在 | mang-app/CI | mang-app 39 测试无 CI 执行且已腐烂（tools==13 断言必红）——含 fs 围栏/llm 掩码安全测试形同虚设 | D-11/FG-01：mang-app/Cargo.toml:3、diag.rs:303-304、windows-check.yml:93-95 |
| **R-20** | P1 | 存在 | scripts/CI | run_tests.py 主链不进任何 CI——T0-T19 全靠本地手跑（T18a/SDK 红、m1 漂移、SEMUL 分叉均无门禁信号） | D-27/FG-04/FG-05；ci-audit §3 影响链 |
| **R-21** | P1 | 存在 | CI | cargo 真件链在 windows-only CI 全 SKIP 恒绿——主解析通道回归防线在 CI 为零（含 guard 治理测试 unix-only 不编译） | E-1/FG-02/FG-14；windows-check.yml:97-108、guard.rs:339-341 |
| **R-22** | P1 | 文档不一致（新发现） | bid-tools/bid-mcp | 暗标 docProps 清零+全部件禁字扫描只在 SEMUL 导出线（v1_darkbid.py/T7d）——产品路径（docx-rs 写面）无等价物、validate dark_bid 档显式降级、回读只查 document.xml；threat-model T-14「业务侧已验」验证的是仿真件 | security-audit §10/FG-13：export.rs:909/:1043-1044、tools.rs:3484-3488、export_readback.rs:418/:766-771、v1_darkbid.py:120-126 |

## P2（边界/一致性/架构——精选）

| 编号 | 级别 | 状态 | 模块 | 一句话 | 证据引用 |
|---|---|---|---|---|---|
| **R-23** | P2 | 存在 | bid-engine | MDCORE_REV 常量错报（aa0c859d vs 实钉 f4328ebf）——全部引擎产物 rev 溯源错报，测试自引用永不红 | B-05 + corpus-verification §3.1 运行坐实；lib.rs:78 vs Cargo.toml:33/36/39 |
| **R-24** | P2 | 存在（B-10 需环境） | bid-parse | OCR 逐页 NonZeroExit 即 break（其后页全丢无状态）+ pdfinfo 失败回退 1 页（整件只 OCR 第 1 页）——大面积静默内容丢失 | B-09/B-10：tesseract.rs:137-141/:87/:99 |
| **R-25** | P2 | 存在 | bid-engine/bid-parse | pdf 引擎通道绕过乱码仲裁/行清洗/页数口径与旧通道不同——环境差异=身份/定位漂移（block_idx 跨环境错位） | B-12：service.rs:376-377、mapping.rs:400-448 vs engines/mod.rs:65-207 |
| **R-26** | P2 | 存在（占比需环境） | bid-parse | 混合 PDF（文本页+扫描页）无逐页路由——两通道都按「任一页有文本」判整件，扫描页内容静默丢失 | B-28：engines/mod.rs:77-119、reader.rs:338-340 |
| **R-27** | P2 | 存在 | bid-mcp | save_version 无围栏——工具面唯一「绝对路径+无根校验」写盘口（存在性探测+围栏外目录家族写入） | C-06：tools.rs:1781-1825；save_version_tool.rs 3 例无负例 |
| **R-28** | P2 | 存在（需进程级越权） | bid-tools | receipt 信任根=approval.key 文件可读性——同 uid 进程可伪造任意 actor 人工授权（stdio 部署同机同用户即达） | C-10：service.rs:517-573、principal.rs:223-289、test_mcp_e2e.py:137-150 |
| **R-29** | P2 | 存在 | bid-mcp | 契约文档缺 set_matrix_override 条目+三处虚承诺（E_AUTH/E_RATE/idempotency_key）+接入文档滞后两版本——对账基准不完整 | C-30/C-13/C-25 |
| **R-30** | P2 | 存在（利用链需运行验证） | web-ui/mang | md 预览 href 属性注入+unsafe-inline CSP——渲染即触发，注入后可触全 IPC 面（含 llm.save） | D-01：app.js:2908、index.html:6；NE-6 |
| **R-31** | P2 | 存在 | scripts | SEMUL 仿真链（V1/V2/V3）与 Rust 实现无交叉验证——历史「全绿」实为仿真绿；T17 断言入库历史产物+六处脚本仓外路径必炸——评测不可从当前 HEAD 重现 | D-22/D-23/D-20/FG-06/07/16 |
| **R-32** | P2 | 存在 | CI | 「Verify SKIP banner」步骤假门：pwsh 管道吞退出码+双分支恒过——自身可被假绿骗过且重复全量跑 20min | D-26/FG-03：windows-check.yml:110-122 |
| **R-33** | P2 | 存在 | bid-tools/bid-engine | S-5 全局一致性（金额/工期/术语冲突）已实现未接线+负偏离查询零消费——投标函 vs 报价表金额冲突无自动防线 | A-06/A-07：check.rs:509-553、param.rs:264-280 |
| **R-34** | P2 | 文档不一致（能力漂移，新增） | bid-engine/mdcore | mdcore main 已 117 提交：#226 docx-rs panic 防御、#210/#68 pdfium 部署自检等读面修复标书未消费；对账脚本钉 3204342d 落后 117 提交——引擎门行为零对账证据 | integration-contract-audit §7/§4；engine_probe/Cargo.toml:16/:21 |

## 附：P2 其余与 P3（不逐条展开，索引备查）

A-08（ExportGate/LinkageLedger 孤岛）、A-09（DIRTY 不接线）、A-10（digest 不覆盖 override/mapping）、A-11（export 跳级，R-01 同族）、A-12（幂等键吞 tier 升级）、A-13（快照写放大无 fsync）、A-19（偏离方向误判）、A-22（评分映射无撤销）；B-06（否决词表两套不同源）、B-07/B-08（B1/B2 与 XlsxRow 黑洞）、B-11（章头中文数字限定）、B-13（OCR 白扫无告警）、B-16（召回两口径）、B-18（跨页拼接吞页眉）、B-19（REQ-A id 漂移）、B-25（参数注入收窄面）；C-07（export 幂等键漏 bidder_name）、C-08/C-09（SDK 版本号取位+stderr 死锁）、C-11（WAIVE 通道无实现）、C-14（游标增长位移）、C-15/C-16/C-17（契约漂移族）、C-18（行无上限+panic 传播）；D-02/D-03/D-05（UI 语义）、D-16（git UI 线程挂起）、D-18/D-19（m1 遗留）；P3 全族见各模块审计——处理优先级建议见各审计 F 节。

## 处置优先级建议（合并后）

1. **立即（死锁/错误放行/伪造族）**：R-01/R-02（main 修复验证回灌）、R-03、R-04+R-05、R-06、R-09/R-10（合并语义洞）、R-22（暗标产品面——本审计新发现，废标级业务风险）。
2. **短期（安全与对账止血）**：R-11（NEEDS_ENV 敌意件回归+engine_probe rev 同步）、R-13+R-29（契约止血三件套）、R-27、R-30、R-32（行级修复）。
3. **基础设施（一次改动多缺口）**：R-19/R-20/R-21（Linux job + mang-app test + run_tests 入 CI——解 6 个 FG）。
4. **接线族随主修复联动**：R-33 随 #244/#245 后续批次。

（与旧册映射：旧 R-01→R-09/R-12/R-24/R-26 面；旧 R-03→R-02/R-05；旧 R-04→R-02+FG-17；旧 R-05→R-01/R-03/R-16；旧 R-11→R-22；旧 R-13→security-audit §8；旧 R-14→R-01 编排面；旧 R-16/R-17 语料方法学→corpus-verification。）
