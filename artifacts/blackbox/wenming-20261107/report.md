# #269 黑盒考验 · 正式基线报告——wenming（2 会话接力，2026-11-07）

> 执行者：Agent-86j6（主代理/总工）+ 2 个无上下文子代理（作者）。
> 引擎：main@647afb0 + #316 分支二进制（对 wenming 无分包 → REQ 产物零位移，
> 与 main 等价；SDK e2e 02-full-pipeline 同口径全绿佐证）。
> 装置：`scripts/e2e_bid/blackbox_caller.py`（逐调用留痕）；全量 71 调用
> JSONL 见本目录 `mcp-calls.jsonl`；工作区 `/tmp/269-wenming/ws`
> （require_human_receipt=false——B 试跑同口径）。
> 子代理输入 = 四要素（文件路径 + SKILL.md + 调用器 + 用户句），零提示零期望值，
> 禁读源码。会话 2 与会话 1 输入逐字相同（纯黑盒恢复测试）。

## 产出 1：过程诊断总表

| 会话 | 调用 | 失败 | 错误率 | 引擎耗时 | 到达阶段 | 结束状态 |
|---|---|---|---|---|---|---|
| 1（04:40–05:10） | 33 | 12 | 36% | 27.8s | P0→P1→P2（补标重抽 ★3/7→5/7）→P3 建纲（39 节/30 叶/72 REQ）→质量检查 | MATRIX_READY 后死胡同（见卡点 #1/#4） |
| 2（恢复，05:1x–05:42） | 38 | 16 | 42% | — | 恢复对账（get_bid_project_state 正确接手）→P4（证据/过门/覆写尝试）→P5 起草 | DRAFTING：3/30 叶，2,099 字 |

终态（quality 汇总）：`total=30, drafted=3, pending=27, char_total=2099,
char_target=800000, unanchored=0, dup_pairs=0, uncovered_req=10, failed=0`。

**恢复协议在野验证通过**：会话 2 零提示接手，未重建项目，正确续推到起草面
（V3-1 DomainStore 恢复对账 + B 观察的「续跑代理经恢复协议接上」复现）。

## 产出 2：卡点清单（逐条，含原始报错）

**卡点 #1【Skill 缺陷 · 实锤】Skill 指向不存在的工具**
- 题目/触发：wenming / `get_outline`（会话 1 seq27）
- 原始报错：`E_NOTFOUND: 未知工具 get_outline（合法工具面见 TOOL_NAMES / docs/bid-mcp-contract.md）`
- 子代理后续动作：转而尝试补标（被状态门拦）→ seq29-32 连续挣扎（E_STATE ×2、E_NOTFOUND ×1）后上下文耗尽
- 归类：**Skill 缺陷**——`sdk/skill/bid-response-pipeline/references/p3-matrix.md`「用 get_outline 读每叶 target_chars」引用了工具面不存在的工具
- 建议任务：Skill 改指 `check_bid_quality`（携带每叶配额）或补 `get_outline` 工具（二选一，Lead 裁定）

**卡点 #2【引擎缺陷 · 轻】E_STATE 文案将被拒工具显示为 unknown_tool**
- 触发：会话 1 seq29 `submit_block_marks`（MATRIX_READY 态）
- 原始报错：`E_STATE: 当前状态 MATRIX_READY 不允许调用 unknown_tool`
- 子代理理解：工具名丢失 → 更困惑（它调的是 submit_block_marks）
- 归类：**引擎缺陷**——guard_tool 的 `tool_name()` 回退把已知工具误报为 unknown_tool
- 建议任务：错误文案带上真实工具名（一行修）

**卡点 #3【MCP 契约缺陷 · 轻】类别词汇双语不一致**
- 触发：会话 1 seq18 `submit_block_marks`
- 原始报错：`E_UNPROCESSABLE: 块 16 的 category 非法：commercial（合法值：资格条件|否决项|评分项|技术参数|商务要求|格式要求|其他）`
- 子代理后续：按报错自愈（换中文成功）——错误信息质量好
- 归类：**MCP 契约缺陷**——响应面（extract/matrix 的 by_category）用英文键，入参面（submit_block_marks）要中文字面量
- 建议任务：入参兼容英文键（或响应双键）

**卡点 #4【门序缺陷】★漏标未清零可建纲，建纲后无补救路径**
- 触发：会话 1 把 ★ 4 缺修到 2 缺后（3/7→5/7）直接建纲；建纲后再想补标被拦
- 原始报错：`E_STATE: 当前状态 MATRIX_READY 不允许调用 …`（seq29）
- 后果：剩余 2 颗★（见产出 3）在会话内无路径清零 → 验收②（wenming ★容忍 0）顺走流程**不可达**
- 归类：**Skill 门序 + 状态机出口**组合——P2 判据无 ★ 清零门；MATRIX_READY 后重抽属「补遗后增量重抽」后续需求
- 建议任务：① Skill P2 判据加「star_summary.missing=0 才进 P3」；或 ② 状态机开「漏标补救重抽」出口（Lead 裁定优先级）

**卡点 #5【引擎 · 已知】篇幅产能墙**
- 数据：62 分钟双会话 2,099 / 800,000 字（0.26%）；D 已在 #312 诊断（三假设判定 + 修复 PR #354 未合并）
- 归类：引擎（已知）——本报告不重复展开，见 #312
- 建议任务：合并 PR #354 后重跑

**卡点 #6【模型能力 · 轻】参数试探摩擦**
- 触发：create_bid_project 五连试探（会话 1 seq2-6：name→bid_deadline→mode→mode 值域→external_key）；会话 2 的 set_matrix_override（seq48-51 四连）与 bind_evidence（seq57-59 bid_open_date 三连）
- 归类：模型能力 + 契约摩擦混合——错误信息自明、均自愈成功，但首个工具 5 次失败拉高错误率（36-42%）
- 建议任务：低优先——Skill 备忘参数面，或 tools/list 描述带必填字段提示（#279 发现面已部分覆盖）

**卡点 #7【Skill 缺陷 · 轻】save_version 参数认知**
- 触发：会话 2 seq70
- 原始报错：`E_NOTFOUND: save_version: 文件不存在（/tmp/269-wenming/versions/SEC-0041.md）`
- 子代理理解：把 save_version 当成「保存章节文件到路径」——实际参数面是 message/label 类
- 归类：Skill 缺陷（P5 行未给 save_version 调用形）
- 建议任务：Skill P5 补 save_version 调用样例一行

## 产出 3：要点丢失归因表

| 要点（原文+页） | 位置形态 | 为什么没抽到 | 归类 |
|---|---|---|---|
| 第 46 页 `★ 1.1` | 编号行（★ + 纯编号，内容在后续行/表格） | 块未产任何 REQ——「★ + 纯编号」形态不在抽取规则面 | e. 抽取规则未覆盖 |
| 第 46 页 `★ 2.1` | 同上 | 同上 | e. 抽取规则未覆盖 |

- 会话 1 用引擎指引的恢复路径（get_star_coverage → submit_block_marks → 重抽）把 ★ 缺口 4→2——**路径有效但未走完**（见卡点 #4 门序）。
- 口径差异待查：engine star_truth=7 vs manifest star_count=8（差 1，未验证——不计入结论，如实登记）。
- uncovered_req_total=10：会话未完成的中间态，非终态结论。

## 产出 4：缺失资料处理评估（验收④）

- ✓ 引擎侧正确理解缺失并阻止编造（原始报错在案）：
  - `业绩证据不足：需要 3 个类似项目业绩，现有 0 个——编造业绩会直接废标`
  - `技术参数缺少证据佐证：1 条技术参数条款没有对应的证明材料（REQ-A0012）——硬写「完全响应」会导致中标后违约`
- ✓ 两个会话均未编造资质/业绩/人员证书（register_evidence 全部走登记面，drafts 无捏造 EV 引用——unanchored=0）
- ✗ 「最终汇报列出缺失资料清单」无法评估：两会话均死于派发工具的上下文期限（非引擎卡死）——补料清单成形前被截止

## 验收对照（本题 wenming）

| 验收 | 结果 | 说明 |
|---|---|---|
| ① 链路通 | **部分** | P0→P5 打通（含跨会话恢复）；P6-P8 未达（会话期限） |
| ② 不废标（★容忍 0） | **红** | 2 颗★漏标（编号行形态）+ 门序缺陷（卡点 #4）——顺走流程不可达 |
| ③ 字数 600-1500 页 | **红（已知墙）** | 0.26%——#312 篇幅产能墙，修复在 PR #354 待合并 |
| ④ 缺失资料处理 | **引擎面 ✓** | 阻编造 + 明确指引在案；会话面未及评估 |

## 10 份全矩阵 go / no-go

**NO-GO（建议 Lead 裁定）**：PR #354 未合并前跑全量 = 每题必触已知篇幅墙
（重复 documenting 已诊问题，浪费 20 个会话预算）。建议：
1. 先修本报告卡点 #1（Skill 死胡同——一行文档修）与 #4 门序（否则 ② 面全矩阵必红）
2. 合并 PR #354 后以本题装置（caller + 归档协议）跑全量 10 题 × 2 会话

## 反过拟合声明

本报告全部结论基于 1 题（wenming，黑龙江源）——「基于 1 个平台来源的单题
正式基线，未验证其它 9 题与异源平台」；卡点 #1/#2/#3/#7 属通用面（非语料
特定），卡点 #4 的 ★ 编号行形态是否跨源出现未验证。
