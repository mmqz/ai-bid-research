# AI 标书系统 · 开源调研与 Skill+MCP 架构设计（研究阶段产出）

> 阶段定位：**只研究、审计、设计**——未实现完整业务，未提前锁定框架。v2 起进入**研究验证阶段**（见 research/verification-plan.md）。
> 审计基线：18 个仓库（5 指定 + 8 搜索补充 + 5 框架参考），全部经 GitHub API 实时取证 + 源码抽检，原始证据见 `research/raw/`；**文档引擎审计基线**：mmqz/mdcore（main=6daeace，只读）+ mmqz/cleanroom-analysis（main=149b32e），2026-10-23。

## 交付物索引

| 文档 | 内容 | 核心结论 |
|---|---|---|
| [research/repository-matrix.md](research/repository-matrix.md) | 18 仓库对比矩阵：许可证/活跃度/实现状态/复用判定 | MIT·Apache 可复用 7 家；AGPL 3 家禁抄；**BidMaster-Pro LICENSE=AGPL 但 README 写 MIT（冲突警报）** |
| [research/workflow-audit.md](research/workflow-audit.md) | 各仓库**真实执行链路**（✅/⚠️/❌ 节点级标注）+ 十项重点审计问题逐项结论 | 真实 Dify DSL / FastMCP Server / 业务 SKILL.md 均存在实证；无一家完整解决条款防丢 |
| [research/risk-register.md](research/risk-register.md) | 17 项风险 + **三层安全保证模型（L1 输入完整性/L2 规则正确性/L3 语义正确性）** + 全量追溯矩阵（风险→设计→模块→用例→残余风险） | v2 修正 3 处测试映射错误；"确定性兜底"收紧为"确定性校验≠确定性正确"，L3 残余归人工审校门禁；R-16/R-17（M1.7 轮新增：文件级输入 WAF 阻断/ GT 未冻结仅探索性） |
| [research/s2-scenario-plan.md](research/s2-scenario-plan.md) | **S2 场景裁决登记（用户已定稿）**：政府采购→工程招投标→企业采购 + M1.7-A/B/C 三批次拆解与完成标准 | 公告级闭环达成但**不宣称文件级验收**；GT 24 条初标草案待人工冻结 |
| [research/test-plan.md](research/test-plan.md) | 合成招标文件 25 埋点 + 12 用例矩阵 + **泛化与压力测试（真实结构分布测试集 D1~D6/组件版本矩阵/引用支持率人工协议/重跑稳定性/导出复验）** | 100% 阈值是**验收目标不是已证性能**；主夹具=第一套回归基准，泛化能力另验 |
| [research/verification-plan.md](research/verification-plan.md) | **验证阶段执行计划**：可执行评测夹具三件套、四条关键链路验证（V1 解析→导出/V2 补遗影响面/V3 内核集成/M1.7 政府采购真实链）、最小架构对照实验 EXP-1（Skill+MCP+状态机 vs Dify 五场景）、六方向落地任务、验证状态词典与回填纪律 | 下一步不堆仓库——验证假设、补风险、小型原型证明可行 |
| [research/cpa-issues-scenario-absorption.md](research/cpa-issues-scenario-absorption.md) | **真实生产场景吸收**（cpa-multi-plugins 63 条 issues 全量抓取）：流截断虚假成功/空内容不失败/超时治理/预算护栏/MCP 工具超限 → 四业务契约佐证 + REQ-OPS-01~03 新需求 + MCP-C9 工具描述预算 | 61→9 组有效映射；原始证据 research/raw/cpa-issues/ |
| [docs/bid-domain-model.md](docs/bid-domain-model.md) | 领域对象（PRJ/TF/REQ/EV/CMP/SEC/VF/RT）+ 状态机 + ID 命名空间隔离 | 条款/证据/文档块 ID 三套命名空间永不混用 |
| [docs/bid-mcp-contract.md](docs/bid-mcp-contract.md) | 8+1 个 MCP 工具契约（参数/返回/错误/幂等/状态转换）+ **KBAdapter 知识库适配器**（含 Dify API 端点核验与局限、Cherry 否决记录） | 知识库=import/search 的可替换实现层；自建为默认，Dify API 仅作 A/B 对比 |
| [docs/bid-skill-design.md](docs/bid-skill-design.md) | 主 Skill 九阶段路径（顺序锁死）+ 停止条件 + 恢复协议 + 反模式清单 | 封闭 Agent 只做编排核对，解析切块归后端确定性代码 |
| [docs/architecture-decision.md](docs/architecture-decision.md) | AD-01~AD-07 决策记录，**全部标注验证状态（TESTED/VERIFIED_SOURCE/DESIGN_ASSUMPTION/PENDING）** | 不上完整 Dify（论据已收紧，待 EXP-1 实测补证）；路径 A；AD-07=文档引擎对齐 mdcore 既有契约 |
| [docs/threat-model.md](docs/threat-model.md) | **安全威胁模型（S1 v1.0）**：信任边界五级 + 15 威胁四组×（面/引擎继承/业务补充/测试锚点/状态）+ 人工审批强制点矩阵（双层门禁） | 建模发现 2 个真实 GAP（T-03/T-04）→ **已于 M1.5 实装关闭（2026-10-25，IMPLEMENTED：guard.rs 五门 + run_limited 治理，20 负向测试）**；注入收口=人工审校门禁（确定性层结构性免疫） |
| [docs/doc-engine-audit.md](docs/doc-engine-audit.md) | **文档引擎只读审计**（mdcore + cleanroom-analysis，源码锚点级）：块锚点/证据三态/八步闭环/ReviewPolicy/Office/PDF 能力面；差距矩阵 G1~G11；集成边界映射（两系统共同维护版本与证据关系）；引擎能力需求清单 P0/P1/P2 | 引擎成熟度高于预期，集成从"自建薄适配"改为"对齐既有契约"；P0 缺口=目录域/文档级格式/暗标元数据清零（需引擎团队排期）；**2026-10-25 复核：引擎仓同步至 c33bcab，三锚点全部仍有效** |
| [docs/engine-capability-requests-g1g3.md](docs/engine-capability-requests-g1g3.md) | **G1/G2/G3 引擎能力需求提交材料**（2026-10-25）：三缺口的最新源码直引（非转述）+ 验收条件 + 依赖 + 业务侧现状（G1 域路径先例/G3 zip 补丁过渡）+ 责任边界声明 | 经用户转达引擎团队；引擎仓只读红线不破 |
| [docs/mvp-implementation-plan.md](docs/mvp-implementation-plan.md) | MVP 实施顺序（M0~M5） | M3 起接真实内核（接口按 AD-07 修订对齐） |
| [research/real-scenario-library.md](research/real-scenario-library.md) | **真实场景库 v1.0（2026-10-26）**：cpa-multi-plugins 45 条生产 issues 全量吸收——12 类故障模式分类学 × 四契约/T-03/T-04/G1~G3 同构映射 × SC-01~12 用例（来源 issue# + oracle + 覆盖状态 + 排期） | issue 库=场景设计源，公开文件=数据基准，不可互替；45 条全覆盖可溯源（原始证据 research/raw/cpa-issues/） |
| [research/audit-r5.md](research/audit-r5.md) | **R5 独立复审报告**（R4 自查独立性补偿轮）：23 探针全自建 + 34 项负向测试假绿猎杀零假绿 + 0e50961 五项修复声明逐项独立复证 + 主代理处置决议 + **M1.6 处置闭环（§9）** | 推翻 R4"守恒门 7 类变体无绕过"（exotica 绕过族三形态）→ **M1.6 已全封（镜像化实现，方案 1+2+3 全采纳）**；cargo 66/66 + run_tests 51/51 |

## 架构一句话总结

**Skill 定义路径、MCP 暴露能力、后端持有状态**：封闭 Agent 按阶段推进并核对数字；8+1 个 MCP 工具薄壳化，业务逻辑与证据链在自有领域库；RAG 知识库做成可替换 Adapter（自建默认 / Dify API 可选 / Cherry 否决）；文档内核（mdcore）经其既有契约面（op/anchor/workbench 命令/wasm 出口）集成，块 ID 与条款/证据 ID 隔离，证据失效判定用指纹、审计用版本。

## 当前状态与下一步

- 已完成：18 仓库实证审计 + 全套设计文档（v1）→ 评审意见落实（v2）→ **V0 评测夹具（v0.4，corpus-A 39 件 + corpus-R D1/D2/D5 五件确定性合成，判分器 13 类破坏注入全捕获）**。
- **已执行：V1 文件解析→最终 DOCX 导出全链路四段验证（v1.0 四段全绿，run_tests 26 项）**——①解析覆盖（corpus-A 文本层锚点 26/26 + corpus-R 结构探针 11/11，非正文盘点归因 G7）；②引用回填（10 链两级定位回查 + 结构漂移/书签重验语义，SEMUL 口径）；③导出保真（LO headless 8 页硬断言 + 分页纪律/页眉页脚域/图片/合并表）；④暗标口径（禁字双面零命中 + docProps 清零 + 金丝雀防空洞绿）。证据：`artifacts/v1/`；脚本：`scripts/v1_*.py`。
- **已执行：V2 补遗/答疑/版本影响面计算验证（v1.0 七断言组全绿，run_tests T8）**——TC-09 六断言契约消费 + DIRTY 恰三章节（无关零误伤）+ 开标日级联（GT-02 翻转 EXPIRED/EV-13 维持禁绑/零误翻）+ 矛盾终裁门禁（终裁前不得 ACCEPTED）+ 幂等 no-op + 删除/新增规则（合成层）。证据：`artifacts/v2/`；脚本：`scripts/v2_impact.py`。
- **已执行：V3 内核集成三问断言（SEMUL v1.0 全绿，run_tests T9）**——五轮重写引用保持（每轮恰一链降级+重验恢复+零悬空）；撤销只降不升（undo 不自发恢复）+ 外部修改不静默 adopt；两阶段导出中断一致性（EXPORTED ⟺ sha256 落库）。证据：`artifacts/v3/`；脚本：`scripts/v3_integration.py`。
- **已执行（M1.5，2026-10-25）：Rust 工具链恢复 + T-03/T-04 安全加固 IMPLEMENTED**——cargo 1.99.1 与锁文件零漂移，全 workspace 真跑全绿（51 测试，含 e2e_m1 OCR 30s 真跑）：① bid-parse/guard.rs 压缩包防御（输入字节门/prescan 三门/压缩比/**实际读取累计计量——声明撒谎截停**/EOCD 守恒门/条目名五类拒绝/XML 深度/流式 sha256；负向 11 测试——测试驱动抓出 2 真漏洞：目录条目绕名门、zip crate HashMap 吸收重复条目）；② run_limited 子进程治理（超时杀**进程组**/溢出即时杀/结果五分类不误标成功/并发门 4 席/TmpGuard 全路径回收/OCR 页数门 200，挂死与无限输出样本回归）。
- **已执行（P1-b/c，2026-10-25）：四关键案例 Rust 契约测试（Python SEMUL → IMPLEMENTED 双实现交叉）**——bid-domain 新增 impact.rs（影响面引擎）/linkage.rs（引用链只降不升+显式重验）/export_gate.rs（两阶段导出）+ four_contracts.rs 9 测试：①补遗级联失效（DIRTY 恰三章节/级联精确翻转/GT-13 维持禁绑/终裁门禁/幂等）②证据只降不升（改写恰一链/undo 不回升/外部不 adopt）③五轮重写锚点正确（每轮恰一链/零悬空/逐轮重验恢复）④导出中断无虚假 EXPORTED（停 EXPORTING/不变式/恢复幂等同 sha/与 ProjectStatus 终态对齐）。**三级状态词典（SIMULATED/IMPLEMENTED/INTEGRATION_TESTED）入 verification-plan §6——仿真通过不计入实装通过，分记不折抵**。
- **已执行（P2 第一轮 v0.1，2026-10-25）：真实文件评测**——五件真实公开水利招标文件（3 文本层 PDF + 1 纯扫描补遗 OCR 真跑 + zip 拒绝验证）× Rust 全链路；**召回 33%→75%（三轮修复）、引用支持 100%**；GT 草案 24 条待人工验收。评测驱动修 5 缺口（数字章节/扫描件页数/目录行/无编号否决-资格-评分三通道/续行窗）；实证"37/37 全绿 ≠ 系统可靠"（合成契约 vs 真实结构多样性不可互替）；已知差距登记 D1 表格拆行/OCR 噪声/技术语义归 M2。证据：`artifacts/real-eval/`（REPORT.md + score-report.json + GT 草案）。
- **已执行（R5 独立复审，2026-10-26）：R4 自查轮独立性补偿**——独立子代理 + 23 探针全自建（R4 样本已丢失，零参考）复证 0e50961 五项修复声明全部属实；34 项负向测试假绿猎杀零假绿；**推翻 R4"守恒门 7 类变体无绕过"**——新登记 exotica 绕过族三形态（尾部垃圾出窗 V6/影子 EOCD V7/zip32 字段分歧 V8，A6 同族端态、当期无可利用面），修复排 M1.6；独立基线：cargo 53/53 + run_tests 46/46（chi_sim 环境缺件曾响亮红，按 fixtures-requirements.txt 契约补装后全绿——缺件不静默过绿再获实证）。证据：research/audit-r5.md。
- **已执行（真实场景吸收 v1.0，2026-10-26）**：cpa-multi-plugins 45 条生产 issues 全量抓取吸收——12 类故障模式分类学 → 四契约/T-03/T-04/G1~G3 同构映射（已有覆盖如实标注测试锚点）→ SC-01~12 真实场景用例；定位：issue 库=场景设计源，公开文件=数据基准，不可互替；原始证据 research/raw/cpa-issues/。另见 63 条全量视角的场景→需求映射（research/cpa-issues-scenario-absorption.md：9 组有效映射 + REQ-OPS-01~03/MCP-C9）。
- **已执行（M1.6 安全加固 + P2 第二轮 + D3 真实链，2026-10-26）**：① EOCD 守恒门重写为 **zip crate 4.6.1 选择语义全文件镜像**——R5 exotica 绕过族三形态全封（V6/V7/V8 六组负向入仓）+ A7 前置垃圾门真跑 + 无可验候选 fail-closed（EocdUnverifiable）；压缩比门乘法判据；xlsx 格轴负向（SC-04）入仓；② SC-05 重写对象隔离 / SC-06 暗标标识符面 D5（T7d 升五断言）/ SC-12 环境预检 T0 / SC-02 docx 部件对账账本 / SC-08 生成器差异对 / SC-11 评测报告 schema 语义版本 / SC-01 查询面唯一口径（query_view 三态视图+中断态断言）；③ **D3 真实补遗链（公告级）**：中传 028 完整采购链驱动影响面引擎——真实「其他内容均无修改」零扩散、真实无价格变更零冲突、SC-07 重导入零触碰全字段快照、中标 105 万≤预算 108 万交叉核；真实样本入仓 `real-samples/`。双线亲测：cargo 66/66、run_tests 51/51、xref 0 错误。
- **S2 已裁决（2026-10-26 用户定稿）：政府采购 → 工程招投标 → 企业采购**；M1.7-A/B/C 三批次拆解见 research/s2-scenario-plan.md；第 10 轮用户指令追加通信行业专项（见下条）。
- **已执行（M1.7 批次一，2026-10-26）：政府采购真实链验证（公告级闭环，run_tests T14 全绿 34 断言）**——cuc-028 真实采购链（招标 07-13/更正 07-28/中标 08-14，ccgp 官方文本 + cuc.edu.cn 镜像字节存档交叉校验）：真实文件解析 6 断言 + 变更提取（恰 3 真实 MODIFY + 2 不变陷阱 + 同值陷阱不串位）6 断言 + 影响面六规则（DIRTY 恰两章=更正公告自带声明/级联/门禁终裁/幂等）9 断言 + 输出复验 9 断言 + GT 草案核对 4 断言。同批：576 份官方公告溯源清单（M1.7-A）+ cuc-028 链 GT 24 条初标草案（M1.7-C，全 draft_pending_human）。**未达成（如实）：文件级验证（招标文件本体受 ccgp WAF 阻断转人工 GT-M17-24）；真实删除/新增条款样本仍缺。证据：`real-corpus/gov-procurement/`；脚本：`scripts/m17_inventory.py`、`scripts/m17_real_chain.py`。**
- **已执行（M1.7-TEL 通信行业专项，2026-10-26，run_tests T16 全绿）：txzbqy 工信部法定平台真实链（四节点 46/46 + 文件级 17/17 + GT 18 条）**——免鉴权五件套 API（列表/详情/文件元数据/令牌/下载，平台 md5 逐字节校验）；四节点链：招标公告（08-12）→**招标失败公告（09-08，不足3家+通信招标办法第三十四条，快照 SNAPSHOT_ONLY 诚实登记）**→第二次招标公告（09-18）→中标候选人公示（10-26 官方JSON），真实 diff（MODIFY 恰 3 + 不变陷阱 4）+ 公示时间线跨文件逐字吻合 + 报价双档<限价双档；文件级：17 真实文件（移动/电信/联通，含 .doc/.xls LO 桥接）Rust 亲跑 17/17 + 条款抽取 2040 条 + 公告↔文件跨向锚点 14/14；真实数据质量双发现（平台 md5 与分发内容不一致同尺寸不同内容→指纹双记录；tenderFileId 字段语义漂移实为立项批复→字段名不可单信）。GT 18 条四要素草案（17 verbatim/1 normalized，draft_pending_human）。证据：`real-corpus/telecom/`；脚本：`scripts/telecom_fetch.py`、`scripts/telecom_chain.py`、`scripts/telecom_file_level.py`、`scripts/telecom_gt_draft.py`；bid-parse `examples/realfiles.rs`。**
- 进行中：研究验证阶段（verification-plan §7 顺序）——V0 ✅ → V1 ✅ → V2 ✅（双实现）→ V3 ✅（双实现）→ S1 ✅（T-03/T-04 已实施关闭）→ M1.5/P1/P2 v0.1 ✅ → R3/R4/R5 审计轮 ✅ → M1.6 ✅（exotica 绕过族全封）→ S2 ✅（用户裁决）→ M1.7 批次一 ✅（政采公告级闭环）→ M1.7-TEL ✅（通信行业四节点+文件级）→ EXP-1 对照实验（Dify 侧待环境）/ S3 首轮指标（待模型环境）→ 决策是否进入 MVP。
- 引擎侧协同：P0 能力需求（目录域/文档级格式/暗标元数据清零）已列 doc-engine-audit §6，经用户转达引擎团队排期；**V1 回填：G3 业务侧缓解已验（过渡）、G6 渲染复验价值坐实、G7 扩展为非正文定位器变体缺口（doc-engine-audit §4.1；M1.6 起解析层部件账本逐文件可见化——SC-02）**。

## 审计与验证脚本

- 审计：`scripts/gh_audit.py`（元数据+文件树+README 取证）、`scripts/gh_search.py`（补充仓库搜索）、`scripts/gh_deep.py`（关键源码抽检）。Token 经环境变量注入，不入库。
- 验证：`scripts/run_tests.py`（每轮 59 项：T0 环境契约预检（SC-12：钉版包/引擎四件/tessdata/字体/cargo——开跑即知，缺件响亮红不静默跳过）+ xref 一致性/夹具完整性/埋点读回/判分器自测（13 类破坏注入+攻击样本回归）/确定性/V1 四段（T7d 暗标五断言含 D5 标识符面）/V2 影响面/V3 三问 + m1 并行线 12 项（T10~T13，cargo 真跑）+ M1.7 真实链 T14 + GT 草案四要素 T15 + M1.7-TEL 通信行业 T16）；`scripts/v1_pipeline.py`（V1-a 解析覆盖）、`scripts/v1_export.py`（V1-c 导出保真）、`scripts/v1_citation.py`（V1-b 引用回填）、`scripts/v1_darkbid.py`（V1-d 暗标口径 D1~D5）、`scripts/v1_corpus_r.py`（corpus-R 结构层生成）、`scripts/v2_impact.py`（V2 影响面验证）、`scripts/v3_integration.py`（V3 内核集成三问）、`scripts/m17_inventory.py`（M1.7-A 真实语料盘点）、`scripts/m17_real_chain.py`（M1.7-B 真实链验证）、`scripts/telecom_fetch.py`/`telecom_chain.py`/`telecom_file_level.py`/`telecom_gt_draft.py`（M1.7-TEL 四脚本）、`scripts/real_eval_score.py`（P2 判分，schema_version SC-11）、`scripts/ooxml_util.py`（确定性 zip + 引擎指纹/定位器语义仿真）。真实语料库：`real-corpus/gov-procurement/`（576 份溯源清单 + cuc-028 三节点链 + GT 草案）+ `real-corpus/telecom/`（txzbqy 20 节点 + 16 文件 + 四节点链 + 文件级解析报告 + GT 18 条）+ `real-samples/`（中传 028 D3 公告级真实链）；真实文件评测证据：`artifacts/real-eval/`。
