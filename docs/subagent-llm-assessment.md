# 子代理 LLM 实测评估——skill 偏移面 + MCP 能力面（#80 第三批 · 任务 A'）

> 结论先行：**skill 面两轮通过（3 章小型轮 + 115 章/83.6 万字规模轮均零偏移），
> MCP 面够用（工具面 17 个 + 全链 stdio 驱动 + 规模轮 115 章全链验证）**。
> 本文是实测记录，不是推断——小型轮数字来自
> `artifacts/e2e-subagent-path/20261030-163329/`，规模轮数字来自
> `artifacts/e2e-subagent-path/latest/`（run 20261031-050432，band-report.json）。
>
> **Lead 反馈落地（规模轮由来的记录）**：小型轮完成后 Lead 指出「3 章 ~4.6k
> 字不构成规模检验——即使不用工作流也能写得很快没有偏差，至少应该
> 50w-100w 字的测试」。规模轮（§七）按此口径重跑：115 章 / 835,852 字。

---

## 一、实验设计：为什么「子代理写」才算数

第二批（PR #82）的「人肉 LLM」由开发者本人撰写正文——开发者构建了整个系统、
知道引擎每个闸门的实现，写作时天然避开所有坑。**这不构成对 skill 的检验。**

本批把作者换成**并行派遣的无上下文子代理**：

```
主 Agent（总工）                          子代理作者 ×3（互不知晓）
  ├─ 跑 capture 相位抓实发 prompt           ├─ 只拿到自己那份 prompt 原文
  ├─ 逐份派遣（零写作指导）                 ├─ 无仓库/引擎/彼此上下文
  ├─ 验收（引擎闸门 + 人工检视）            └─ 交付 = 补全输出本体
  └─ 跑 serve 相位回灌全链
```

隔离纪律（模拟真实 LLM API 服务）：

1. 子代理只收到 RealGenerator **实发**的 prompt（system + user 照录，含请求参数）；
2. 传输性框架只声明交付方式（写入哪个文件）与隔离约束（禁读仓库/禁探索环境），
   **零内容指导**——字数/格式/引用规则全部来自 prompt 本身；
3. 主代理不预审、不改写、不补写——手稿原样回灌（`content-map` 按标题路由）。

**这就是对「skill 是否成功可用不偏移」的操作化定义**：RealGenerator 构造的
prompt 是章节作者唯一的信息面。作者偏移（编造编号/漏条款/字数跑飞/夹带元信息）
= prompt 有缺口 = skill 要改。

## 二、结果数据（引擎闸门 + 人工检视双面）

目标章选择：确定性规则（REQ 数降序、id 升序取前三）——恰好都是「其他」
分块（该类目 24 条 REQ 被切成 3 个 8 条块，是全树最大的章）。

| 章 | 标题 | 手稿字数 | 目标 | 偏离 | 引用 | 覆盖 | 首行格式 | 引用卫生 |
|---|---|---|---|---|---|---|---|---|
| SEC-0008 | 其他（第1块） | 1547 | 1392 | +11.1% | 8 | 8/8 | `# 标题` ✅ | 零编造 ✅ |
| SEC-0009 | 其他（第2块） | 1610 | 1392 | +15.7% | 8 | 8/8 | `# 标题` ✅ | 零编造 ✅ |
| SEC-0010 | 其他（第3块） | 1493 | 1392 | +7.3% | 8 | 8/8 | `# 标题` ✅ | 零编造 ✅ |

引擎闸门全绿：generator=real ×3 → 引用回查通过 → 覆盖度 100%（uncovered=∅）
→ 字数全部带内 → `validate_bid_document` blocker=0 → 导出 docx 103,781 字节
→ llm-usage 台账 3 次调用（prompt 3,159 / completion 4,650 tokens）→ 重启恢复
EXPORTED + 16 叶全量在盘。每章完成即 `save_version`（skill 纪律 8——mang-snap
AI 版本，快照见 `save-versions/`）。

人工检视（引擎闸门覆盖不到的面）：

- **零元信息**：三份手稿均为正文本体，无解释/前言/后记；子代理的会话回复
  只有一行交付确认（与文件内容干净分离）；
- **零越界引用**：无一例 EV 编号（prompt 声明「证据材料：无」）；
- **结构自组织**：三份手稿都自发采用「二级小节分组 + 逐条 REQ 针对性回应 +
  量化承诺收尾」结构，彼此风格一致但非模板复制（内容全部针对各自条款）；
- **量化承诺面**（起草指令要求）：175 天工期、驻场人员编制、设备台套、
  响应时限等具体数字全部在场且可核验。

## 三、skill 面评估

### 3.1 首轮判定：prompt 够用，未见偏移

对「无上下文作者 + 单章独立作业」这个形态，RealGenerator 的 system prompt
五条规则（格式/引用卫生/覆盖/字数/纯正文）**被完整遵守**。规则表述没有
歧义导致的跑偏，约束密度适当（足以约束，未多到压制内容质量）。

### 3.2 行为模式观察（非违规，供画像）

- **字数策略一致偏保守上浮**：三份都往目标之上写（+7.3% ~ +15.7%，无人
  贴下限）——「宁多勿少」是常见的安全边际行为，对 ±20% 带宽无威胁；
- **指令的指令性传导良好**：起草指令里「不得空话」的量化要求被三份手稿
  一致执行（进度/人员/设备三类数字齐全）——说明 instructions 字段的措辞
  会直接塑形输出质量，总工写指令值得继续花功夫。

### 3.3 诚实局限（本轮没测到的，不算通过）

| # | 未覆盖面 | 为什么重要 |
|---|---|---|
| 1 | 样本 n=3、同模型（glm-5.3） | 换弱模型/换厂商是否稳，未证 |
| 2 | 章型同质（全是「其他」行政条款块） | 技术参数密集章/长目标章（3000+ 字）的长度控制未证 |
| 3 | 无证据章 | prompt 的「证据材料：无」分支测了；「有 EV 清单」分支（EV 引用纪律）未测 |
| 4 | 暗标模式 | rules_profile=dark_bid 的正文禁字面未进入本形态 |
| 5 | 传输框架非完全等同真实 API | 子代理仍有工具面（Write），真实模型只有文本进出——已用禁令模拟，但不是同构 |

**下一轮建议**：选章规则加「类目多样性」（每类目取最大章），把技术参数章、
带证据章、暗标项目各测一轮——这比增加同型章数量更有信息量。

### 3.4 产品反馈（prompt 改进候选，按价值排序）

1. **字数下界提示**：三份全偏上浮，带宽占用不均——可在 user 消息里给
   「建议围绕目标字数组织，勿系统性贴上限」的软提示（观察 3.2）；
2. **REQ 回应顺序自由度**：当前规则未要求按清单顺序回应（手稿自发保序了），
   若引擎对顺序无依赖，宜显式声明「顺序不限」消除作者的自缚；
3. **「不得出现投标人身份信息」只在暗标相关条款出现**（REQ-0104 类）——
   明标下作者正确地没自我审查，暗标 profile 下需要在 system 层统一加禁令。

## 四、MCP 能力面评估（缺口清单——#80 纪律 8 产品反馈）

**判定：不够。** 本轮全链 16 步中 13 步走 MCP 工具，3 步走服务面直调——
一个只有 MCP 接口的外部 Agent（Claude Desktop / 任意 MCP 客户端）会在
P3（矩阵）之前卡死，根本到不了起草。

| # | 缺口 | 现状（服务面直调） | 卡点 | 建议优先级 |
|---|---|---|---|---|
| G1 | 建纲 + 矩阵初始化 | `BidService::prepare_outline_and_matrix` | 无 MCP 工具 → extract 之后无路可走 | **P0**（无它全链断） |
| G2 | 证据登记 + 绑定 | `register_evidence` / `bind_evidence` | 矩阵 blocking 行无法清零 → draft 被 E_STATE 拒 | **P0**（与 G1 同批） |
| G3 | 三门审批 | `record_review`（Matrix/Outline/PreExport） | export 内部预检要求门已批 | P1（可先做只读查询面） |
| G4 | 编排纪律 | `check_bid_quality` 在场但本轮未调用 | 非工具缺口，是总工编排面纪律缺口（本评估自省） | 记录在案，下轮补 |

佐证：`tests/subagent_llm_path.rs` 的 `ensure_prepared` 内三段标注
【缺口 G1/G2/G3】的服务面调用即卡点实录；`docs/bid-skill-design.md` §4
的 P0-P8 映射里，P3/P4 依赖的 MCP 面与实现存在同样的落差。

**补齐建议**（下一步开发项）：G1+G2 合并成一个 `prepare_response_scaffold`
或拆成 `build_outline` + `register_and_bind_evidence`（后者含 EXPIRED 拒绑
语义），G3 做 `record_review_gate`（含 decision/actor/意见留痕）。三者都
是薄壳转发，业务已在服务层——契约文档同步补 §10/§11/§12。

### 四-B、缺口闭环（同批次落地——评估后 24 小时内）

G1/G2/G3 已在同批补齐（拆法采纳上文建议的细分版）：

| 工具 | 契约 | 验收 |
|---|---|---|
| `build_outline` | §10 | 建纲+矩阵首建 → MATRIX_READY；值域校验；空条款拒绝 |
| `register_evidence` | §11-A | 语义位查重/换版；locator 可选（缺省诚实标记）；fingerprint 缺省规则 |
| `bind_evidence` | §11-B | 批量绑定 + 绑后矩阵重算；逐件 E_PARTIAL；R-03 禁绑 |
| `record_review_gate` | §12 | human: 复审人门；门序/窗口裁决全在域层；gates_approved 同源 |

**闭环证据**：`tests/mcp_scaffold_tools.rs` ——外部 Agent 视角**零服务面直调**
驱动全链（15 叶 / 绑 56 条 / 三门 / EXPORTED）+ 错误路径面（E_STATE×4 /
E_UNPROCESSABLE×5 / E_PARTIAL×2）。工具面 13 → 17。G4（总工调用
check_bid_quality 的编排纪律）在评估报告的下一轮选章规则中一并落地。

### 四-C、终极检验：无上下文外部 Agent 纯 stdio MCP 驱动（黑盒）

工具补齐后，又派遣一个**只拿契约文档**（docs/bid-mcp-contract.md）+ 部署配置
（二进制路径 / stdio 行协议 / BID_WS / 素材路径）的子代理——严格模拟
Claude Desktop 里对引擎一无所知的操作者（禁读源码/测试/其他文档，禁碰
工作区内部文件，报错只能靠 error.what 自救）：

**结果：45 次 MCP 调用（41 成功 / 4 报错全部自救）全链推到 EXPORTED**——
108 条款抽取、21 章 15 叶、64 条证据绑定（blocking 56→0）、三门齐过、
15 章 mock 起草 21,146 字、validate blocker=0、导出 docx 103,889 字节
（file 核验合法 Word）。**MCP 能力判定从「不够」翻转为「够用」**——
可驱动性成立，余下的是摩擦面。

**摩擦清单（14 条，按痛感排序）与本批处置**：

| # | 摩擦 | 处置 |
|---|---|---|
| 1 | export 的 output_path 契约漏写（两轮试错靠报错自救） | ✅ 契约 §8 补参数说明（含相对路径规则） |
| 2 | validate 响应不回 project_version 却推版本 → 后续 E_CONFLICT | ✅ 实现补回传 + 契约注明 |
| 3 | 章节 ID 发现路径藏在末尾括号注 | ✅ 契约 §9 提升为正文「章节发现面」 |
| 5 | bid_open_date 与 bid_deadline 关系无出处 | ✅ 契约 §11-B 补语义说明 |
| 7 | EV-id 不从 0001 起（import 自动入账没写） | ✅ 契约 §2 补注 |
| 8 | register_evidence hint 罐头文案误导 | ✅ 实现改条件式措辞 |
| 12 | 错误字段名 what vs 契约 message | ✅ 契约以实现为准修订 |
| 4 | 章节→REQ 归属无正式查询口（靠矩阵反推） | ⬜ 登记后续项（outline 明细只读面候选） |
| 6 | options.granularity 合法值未枚举 | ⬜ 登记后续项（契约补枚举） |
| 9 | project_version 步进不透明（+1~+6 不等） | ⬜ 登记后续项（文档说明步进源） |
| 10 | REVIEWING 状态从未出现（契约与状态机不符） | ⬜ 登记后续项（产品语义裁决） |
| 11 | unanchored_claims>0 无 warning（契约承诺落空） | ⬜ 登记后续项（K5 相关联动） |
| 13 | W_FORMAT_BASELINE 文案与事实打架 | ⬜ 登记后续项（文案修订） |
| 14 | 小噪音（watermark 告警 / BID_DEADLINE_PAST 位置） | ⬜ 登记后续项 |

**正面信号**（外部 Agent 原话归纳）：全部 4 次报错信息可操作、能 self-serve
恢复；分页/过滤/幂等面无障碍——错误文案质量是本轮最结实的能力面。

## 五、顺带修复：环境 C 的 OCR 静默空导入（bug，非缺口）

**现象**：本沙箱 tesseract 在位而 chi_sim 缺失（环境 C）时，
`synth_tender_eval` 失败——引擎把「OCR 页 NonZeroExit」按数据面降级处理，
E 扫描件以 `ocr_required / 0 chunks` **静默导入成功**，内容整档丢失却无
任何失败信号（issue #9「环境缺失伪装成业务行为」的变体）。

**修复**（`bid-parse/src/pdf.rs`）：OCR 页失败且**零行块产出** → 解析失败
（E_PARTIAL 让调用方看见）；部分页成功仍降级（数据面残缺语义不变）；全页
成功但空文本（白扫）不在此列（诚实空）。修复后全仓 `cargo test -p bid-mcp
-p bid-parse` 绿。

## 六、小型轮复现方式（3 章形态——缺省旋钮不变）

```bash
# 两相位驱动（中继 + 测试 + 归档一体）：
scripts/e2e_bid/run_subagent_path.sh phase1 /tmp/subagent-ws   # 抓 prompt
#   → 主代理并行派遣 3 个子代理作者（只给 prompt，零额外指导）
#   → 手稿落 /tmp/subagent-ws/manuscripts/SEC-XXXX.md
scripts/e2e_bid/run_subagent_path.sh phase2 /tmp/subagent-ws   # 回灌全链
# 产物：artifacts/e2e-subagent-path/<run-id>/ + latest 刷新
```

测试本体：`bidcore/crates/bid-mcp/tests/subagent_llm_path.rs`（CI 默认
SKIP——需要中继与作者面，与 real_llm_path 同款纪律）。

---

## 七、规模轮：115 章 / 83.6 万字全子代理撰写（Lead 反馈落地）

> **背景**：小型轮（n=3，~4.6k 字）被 Lead 判定样本量不足——「即使不用
> 工作流也能写得很快没有偏差」。规模轮把体量拉到 **50w-100w 字区间**，
> 全部章节由无上下文子代理作者撰写、全链 MCP 回灌。

### 7.1 实验配置（规模旋钮——测试新增，缺省值保持 3 章形态不变）

| 旋钮 | 值 | 语义 |
|---|---|---|
| `BID_SUBAGENT_TOTAL_CHARS` | 800,000 | 建纲目标总字数（80 万字，落在 50w-100w 区间） |
| `BID_SUBAGENT_MAX_REQS_PER_LEAF` | 1 | 每叶单条款——115 叶，配额 ~6,957 字/叶（单次补全能力带内） |
| `BID_SUBAGENT_N_TARGETS` | 0 | 全部叶章都是目标章（无 mock 补齐） |
| `BID_SUBAGENT_BAND_TOLERANCE` | 0.15 | 字数带外率容差（偏移是评估数据，记录不中断；超容差才 fail） |

章型分布（synth-tender 引擎抽取 115 条）：否决项 51 章 / 其他 31 章 /
技术参数 15 章 / 资格条件 8 章 / 评分项 6 章 / 格式要求 4 章——
**六类章型全覆盖**（小型轮「章型同质」局限被消除）。

### 7.2 派遣面（115 个无上下文子代理作者）

- 主代理（总工）分 10 批并行派遣 115 个子代理（每批 ~12 个），传输性
  框架与小型轮完全一致：只给 prompt 文件路径 + 手稿输出路径 + 隔离纪律，
  **零内容指导**；
- 115/115 交付（同模型 glm-5.3，与小型轮同源——模型均匀性受控）；
- **传输摩擦实录（新发现）**：9/115（7.8%）子代理因沙箱 Write 工具对
  /tmp 的写入限制，自行改写至 `/home/z/tmp/...` 等镜像路径后报告成功——
  总工验收时发现并归位。**教训：传输框架的交付路径须在派遣前探测可写性，
  或工作区应置于无限制路径**（已登记为框架改进项，非 skill 缺陷——9 份
  手稿内容本身全部合规）。

### 7.3 结果：skill 面 115/115 零偏移（引擎闸门 + 人工检视双面）

| 指标 | 结果 | 备注 |
|---|---|---|
| 总字数 | **835,852**（目标 800,000 的 104.5%） | 全部为子代理真实撰写 |
| 字数带内（±20%） | **115/115，带外率 0.0%**（容差 15% 未触及） | band-report.json |
| 偏离分布 | 均值 +5.8%，最大 +15.4%；0-5% 59 章 / 5-10% 37 章 / 10-15% 16 章 / 15-20% 3 章 | 全部偏上浮（与小型轮行为模式一致） |
| 首行格式 `# 标题` | 115/115 全对 | |
| 引用卫生 | 115/115 引用自身 REQ；否决项/资格条件章合法引用 [EV-0128]（prepare 链登记的资质证书）；**零编造编号** | |
| 覆盖度 | uncovered=∅ 全章 | 引擎闸门 |
| 元信息泄漏 | 零（唯一关键词命中为「外网用户无法访问内网」业务语境误报） | 人工检视 |
| G4 check_bid_quality | 章节发现 115｜out_of_band 0｜needs_human 0 | 总工编排纪律落地 |
| validate | blocker=0 | 引擎闸门 |
| 导出 | docx 4,503,331 字节（78.5 万字正文） | file 核验合法 Word |
| 用量台账 | 115 次调用 | |
| 重启恢复 | EXPORTED + 115 叶全量在盘 | |
| serve 相位耗时 | 23.67s（~100ms/章：中继回灌 + 引用回查 + 覆盖度 + save_version） | |

### 7.4 MCP 面：规模下的能力判定

prepare 链在本轮**全部改走 MCP 工具面**（build_outline §10 /
register_evidence + bind_evidence §11 / record_review_gate ×3 §12——
矩阵分页发现待绑 REQ 同外部 Agent 视角），起草/校验/导出/save_version
走真实 stdio 子进程——**115 章规模全链零服务面直调**。加上 G4
（check_bid_quality）纳入测试编排，小型轮登记的 G4 编排缺口同步闭环。

规模下新增摩擦：无（115 次 draft + 115 次 save_version + 分页矩阵 +
三门 + 校验 + 导出全链无报错；唯一故障是 §7.2 的沙箱路径问题，属传输
框架面而非 MCP 面）。

### 7.5 顺带修复：band-report 百分比缩放 bug（测试面，非引擎面）

首轮 serve 后发现 band-report.json 的 dev_pct 字段把百分比值缩小了
10 倍（`(dev*100).round()/10` 的舍入写法错误——12.9% 被记成 1.3）。
带内判定与带外清单不受影响（用原始 dev），但报告数字失真。已修复为
`(dev*1000).round()/10` 并全链重跑验证（本轮数字均为修复后口径）。

### 7.6 规模轮仍未覆盖的（诚实局限更新）

| # | 未覆盖面 | 状态 |
|---|---|---|
| 1 | 多条款章节形态（本轮每叶 1 条款；8 条款/叶的 7k+ 字长章未测） | 待下轮（max_reqs_per_leaf 旋钮已就绪） |
| 2 | 暗标模式（rules_profile=dark_bid 正文禁字面） | 未变 |
| 3 | 弱模型/异厂商作者 | 未变（两轮均 glm-5.3） |
| 4 | 修订循环（带外章打回重写→再回灌） | 本轮零带外未触发；机制已具备（fresh ws 重跑） |

### 7.7 规模轮复现方式

```bash
BID_SUBAGENT_TOTAL_CHARS=800000 BID_SUBAGENT_MAX_REQS_PER_LEAF=1 \
BID_SUBAGENT_N_TARGETS=0 BID_SUBAGENT_BAND_TOLERANCE=0.15 \
scripts/e2e_bid/run_subagent_path.sh phase1 /tmp/subagent-ws-scale
#   → 派遣 115 个子代理作者（手稿落 manuscripts/SEC-XXXX.md）
BID_SUBAGENT_TOTAL_CHARS=800000 BID_SUBAGENT_MAX_REQS_PER_LEAF=1 \
BID_SUBAGENT_N_TARGETS=0 BID_SUBAGENT_BAND_TOLERANCE=0.15 \
scripts/e2e_bid/run_subagent_path.sh phase2 /tmp/subagent-ws-scale
# 产物：artifacts/e2e-subagent-path/<run-id>/（含 band-report.json）+ latest
```

测试：`cargo test --workspace` 438 用例全绿（缺省旋钮 = 3 章形态，
向后兼容；规模旋钮只影响显式设置时的行为）。
