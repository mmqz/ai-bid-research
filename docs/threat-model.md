# 安全威胁模型与人工审批边界（S1）

> 定位：verification-plan §5.4 交付物。范围 = 标书系统 MVP 的**后端强制**安全面——
> 前提立场：Agent/Skill/模型属**不可信执行层**，一切防线以后端（bidcore + 文档内核
> mdcore）强制为准；提示词约束只作纵深一环，永不作唯一防线。
>
> 口径：每威胁一行——面 / 引擎继承（mdcore 已有机制，源码锚点见 doc-engine-audit §7）/
> 业务补充（bidcore/MCP 层职责）/ 测试锚点。状态词典同 verification-plan §6。
>
> 威胁建模基线：STRIDE 变体按四组展开（文件敌意面 / 越权与外发 / 提示注入 / 数据泄露），
> 外加供应链组；**T-03/T-04 为本次建模发现的真实缺口（GAP 登记，非既有设计）**。

## 1. 信任边界与信任级别

| 层 | 信任级别 | 依据 |
|---|---|---|
| 招标文件 / 证据文件（用户上传） | **不可信输入** | 任意来源文件——内容一律数据化，永不执行 |
| Agent（Skill 驱动的 LLM 操作者） | **不可信执行** | 模型可被注入操纵——工具参数 schema 化、无 shell/无自由 SQL、审校态不可写 |
| MCP 工具层（bid-tools） | 半可信 | serde 编译期契约（schema 拒绝在类型层）+ 状态机门禁（E_STATE） |
| 后端核心（bidcore domain/parse/kb） | 可信 | 确定性代码、无网络面、Result 纪律无 panic 路径 |
| 文档内核（mdcore） | 可信 | 容器安全门/消毒链/限额/PathPolicy/ReviewPolicy（审计已核） |
| 人工操作者（human: 前缀） | 最高 | 审批动作唯一合法来源 |

## 2. 威胁清单

### 2.1 文件敌意面（上传件 = 不可信输入）

| ID | 威胁 | 面 | 引擎继承（mdcore） | 业务补充（bidcore/MCP） | 测试锚点 | 状态 |
|---|---|---|---|---|---|---|
| T-01 | 恶意 Office 宏/OLE 载体（docm/xlsm 伪装） | 上传面 | office-io 读面仅走数据结构解析（宏不执行）；容器门+消毒链（§2.7） | 上传类型白名单：仅 docx/xlsx/pdf（宏载体格式直接拒收）；失败件隔离禁引（R-01 缓解） | M0 上传检查单；解析器拒收测试（恶意扩展名） | DESIGN_ASSUMPTION（M0 落地） |
| T-02 | PDF JavaScript 动作/嵌入对象 | 上传面 | mdc-pdf ExtractText 仅提取文本（JS 不执行） | 同 T-01 白名单；poppler 提取路径无执行面 | 解析器单测 + 组件版本矩阵（test-plan §5.2） | VERIFIED_SOURCE（poppler 行为） |
| T-03 | **压缩包炸弹**（高压缩比 XLSX/DOCX 撑爆内存） | 解析面 | office-io XLSX 三门 + 稀疏炸弹闭环（fd 校准实测，§2.7） | **已实施（M1.5，2026-10-25；R3 修复轮 + S3-1/S3-3 加固；M1.6 EOCD 镜像重写 2026-10-26）**：bid-parse/guard.rs 挂门——输入字节/prescan 三门/压缩比（**乘法判据**）/实际读取累计计量（声明撒谎截停）/**EOCD 守恒门 crate 4.6.1 选择语义全文件镜像**（V6 尾垃圾出窗/V7 影子 EOCD/V8 字段分歧三绕过已封 + A7 前置垃圾门真跑 + 无可验候选 fail-closed 拒绝）+ 条目名拒绝 + XML 深度 + xlsx 读后行/单元格双轴门（**双轴负向测试均已入仓**：行轴 20 万+1 + 格轴 95,239×21）；负向测试 18 项全绿（含撒谎样本 + zip64 重复负向 + 尾垃圾三档差分 + 影子 EOCD + 字段分歧双向 + 终端 fail-closed + 压缩比边界对） | 已实施：guard.rs 18 测试 + xlsx 3 测试 + corpus-A 回归 | **IMPLEMENTED+（残余：calamine 黑盒内部解压内存尖峰——入口三门 + 读后双轴门已挡常规面与下游无界，根治需流式自读 M2+；镜像严格方向残留三形态均 fail-closed 已登记 §4；A6 同族端态维持登记——常规面与结构化路径均已封）** |
| T-04 | **外部解析器挂死**（畸形 PDF 卡住 pdftoppm/tesseract） | 解析面 | —（引擎自有渲染线不依赖外部进程） | **已实施（M1.5，2026-10-25；R3 修复轮 + P2-4 加固）**：run_limited——超时杀进程组/溢出即时杀/五分类/并发门 4 席/TmpGuard 全路径回收/OCR 页数门 200/stderr 持续 drain（P2-4：原 take 读满即停会 SIGPIPE 中杀 verbose 合法工具——实测修正审计原描述，非阻塞 Timeout；drain 后保留头 64KB 不阻塞不中杀）；挂死样本（sleep 30 超时 1.2s 杀）、无限输出样本（yes 溢出即时杀）、stderr 洪泛样本（256KB→Success）回归入测 | 已实施：guard.rs 9 测试 + corpus-A OCR 链路回归 | **IMPLEMENTED** |
| T-05 | 超大文件资源耗尽 | 上传面 | 写面限额（MAX_PARAGRAPHS/RUNS；G11 容量匹配） | **已实施（#183）**：fs_fence 限额门——单件 500MB / 单批 64 件 / 累计 2GB，超限 E_UNPROCESSABLE 逐件报因（先于读取，不吞资源） | bid-tools tests/fs_fence.rs 限额面 + bid-mcp 30 目标回归 | **IMPLEMENTED（#183）** |

### 2.2 越权与外发

| ID | 威胁 | 面 | 引擎继承 | 业务补充 | 测试锚点 | 状态 |
|---|---|---|---|---|---|---|
| T-06 | 跨项目/跨企业资料越权 | 领域库 | — | ID 命名空间隔离（PRJ/REQ/EV/CMP/SEC 永不混用，domain-model §2）+ 资料库按项目隔离的访问范围 | M2 权限用例（越权访问他项目 EV → 拒绝） | DESIGN_ASSUMPTION |
| T-07 | 任意路径读写 | 工具面 | PathPolicy 允许列表模式（§2.7） | 库内 ID 面：serde schema 编译期拒绝自由路径串（M1 起）。**路径面已实施（#183/#189）**：import_tender_files 的 upload_ref 受文件围栏约束——受信根（BID_FS_ROOT ∪ BID_WS）内组件级包含判定（防 `work` vs `work-evil` 前缀陷阱）、词法 `..` 预拒、逐段 symlink 检查；导出侧 resolve_write 同款纵深（工具层工作区相对路径形状校验之外的第二层）；壳层逐件 resolve_read + 服务层持根复验双保险 | bid-tools tests/fs_fence.rs（10 项：逃逸三态/前缀陷阱/多根/限额）+ bid-mcp 30 测试目标回归 + sdk e2e 双传输 | **IMPLEMENTED（#183/#189；TOCTOU check-then-use v1 残余登记 §4-6）** |
| T-08 | 审批绕过（AI 写审校态/自我 WAIVE） | 审批面 | ReviewPolicy `human:` 前缀强制——AI 操作者在引擎层不可写审核态；失效不过审（§2.7） | 双层门禁：MCP §8 precheck（BLOCKER 清零/显式 WAIVE 前置）+ 引擎 allow_approval_of_broken_linkage=false（§5.5）；终裁门禁 V2 R4 已验（终裁前不得 ACCEPTED） | run_tests T8 R4；引擎 security.rs 语义锁测试 | TESTED（V2 SEMUL + 引擎单测） |
| T-09 | 数据出域外发 | 部署面 | —（本地纪律：sidecar 本地） | 部署红线：自托管、无公网出口依赖；外发域名白名单（若有通知场景）；R-09 数据边界 | 部署检查单 | DESIGN_ASSUMPTION |

### 2.3 提示注入纵深

| ID | 威胁 | 面 | 引擎继承 | 业务补充 | 测试锚点 | 状态 |
|---|---|---|---|---|---|---|
| T-10 | 文档内容注入（招标文件内藏指令文本） | 解析→起草 | — | **结构性免疫（确定性层）**：bidcore 解析/抽取/影响面全部为确定性代码，无模型消费文档原文——注入文本在解析层只是数据；L3 残余：起草模型消费抽取结果时注入面存在 → 文件内容数据标记（domain-model）+ Skill 纪律（bid-skill-design 反模式） | TC-05 规则 8（注入指令 8 类命中）；已知模式扫描仅告警不作防线 | PARTIAL（确定性层免疫=TESTED；起草层 L3 残余→人工审校门禁） |
| T-11 | 工具参数注入（诱导 Agent 构造恶意参数） | 工具面 | — | 路径 A 封闭 Agent 只做编排核对；MCP 工具全部参数 schema 化；无 shell/无自由 SQL/无任意路径（与 T-07 同面） | bid-tools 类型化 schema 拒绝（cargo test）；TC-11 E_STATE 越级防护 | TESTED（M1） |
| T-12 | 证据库投毒（伪造证据文件） | 证据面 | 证据三态 + 登记时刻指纹（内容换即失效） | 证据来源登记（文件 sha256 入库）+ 引用键纪律（R-04：引用存在≠支持——人工复核 L3）+ 上传者身份审计 | TC-04 引用支持率人工协议（test-plan §5.3） | PARTIAL（机制 TESTED；支持率人工协议待 M3） |

### 2.4 日志与数据泄露

| ID | 威胁 | 面 | 引擎继承 | 业务补充 | 测试锚点 | 状态 |
|---|---|---|---|---|---|---|
| T-13 | 证据摘录/sidecar 经日志泄露 | 日志面 | sidecar 本地纪律（md+meta 不出机器） | 日志脱敏清单：证据摘录截断或哈希化；trace.jsonl 不含证据全文（M1 已按此实现——trace 仅工具调用摘要） | 部署检查单 + trace 抽查 | PARTIAL（M1 trace 已合规；全链路脱敏清单 M5） |
| T-14 | 导出件元数据泄露（暗标身份暴露→废标） | 导出面 | —（G3 引擎侧缺口仍开） | **业务侧缓解已验（V1-d）**：docProps 清零 + app.xml Company 移除 + 禁字双面扫描 + 金丝雀防空洞绿 | run_tests T7d（每轮回归） | TESTED（业务侧）；引擎侧 G3 P0 待排期 |
| T-15 | 依赖/镜像/模型出口投毒 | 供应链 | — | R-10 白名单 + NOTICES 式传递依赖闭包审查；镜像固定摘要；模型出口=本地部署默认 | M0 合规审查 | PENDING |

## 3. 人工审批边界（强制点矩阵）

> 原则：**审批动作的下沉与双层**——引擎层 ReviewPolicy `human:` 前缀（AI 结构性不可写）
> + 业务层 MCP §8 precheck（状态机门禁）。任一层独立成立，不依赖单点。

| 动作 | 引擎层门禁 | 业务层门禁 | 状态 |
|---|---|---|---|
| 矩阵 policy 确认（eligible/not_eligible 终判） | Review `human:` | MCP §6 强制绑定 + 复核位 | DESIGN_ASSUMPTION（M2） |
| BLOCKER WAIVE | 失效不过审（allow_approval_of_broken_linkage=false） | MCP §8 WAIVE 记录（显式、留痕、可审计） | DESIGN_ASSUMPTION（M4） |
| 冲突终裁（CONFLICT 双版本裁决） | —（业务侧状态机） | V2 R4 已验：终裁前相关章节不得 ACCEPTED | TESTED（V2 SEMUL） |
| 最终导出放行 | Review `human:` | MCP §8：BLOCKER 清零/显式 WAIVE 前置 + sha256 落库为界 | TESTED（V3 Q3 两阶段一致性） |
| 提交（递交投标文件） | — | 人工动作——系统只准备不代交（bid-skill-design 停止条件） | DESIGN_ASSUMPTION |

## 4. 残余风险与待办

1. **T-03 压缩包炸弹限额——✅ 已实施关闭（M1.5，2026-10-25；cargo test 12 项 + R3 审计核验 + R3 修复轮 S3-1/S3-3 加固）**：bid-parse/guard.rs——① 输入字节门（docx 52MB / xlsx 302MB，对齐引擎 LIMIT_INPUT_BYTES 族）② zip prescan 三门（条目数 10,000 / 单声明 300MB / 总声明 300MB）+ 压缩比门（>100:1 拒绝）③ **实际读取累计计量**（take 流式截停——声明值撒谎在此拦截，负向测试 lying_declaration 实证）④ EOCD 与 zip64-EOCD 条目守恒门（zip crate HashMap 吸收同名条目常规面已封；S3-1：0xFFFF 标记经 zip64 locator 链取真实计数，zip64 重复条目常规面已封——负向+合法不误拒正向双测试；exotica 绕过族 R5 登记，见下）⑤ 条目名五类拒绝（穿越/绝对路径/盘符/控制字符/深度 64）+ 嵌套归档载具拒绝 ⑥ XML 深度门 1000 ⑦ 流式 sha256 ⑧ xlsx 读后双轴门（S3-3：有效行 20 万/非空单元格 200 万，超量显式拒绝不留部分结果——负向 20 万+1 行真样本 + 正向百行对照）。**测试驱动抓出 2 个真漏洞并修复**：目录条目（尾部 /）绕过名字门、HashMap 吸收重复条目。
   **残余面（诚实登记）**：xlsx 的实际解压流在 calamine 黑盒内不可注入计量——入口三门 + 读后双轴门已挡常规炸弹与下游无界；声明撒谎骗过 prescan 后 calamine **内部**解压内存尖峰为残余（威胁极低：需构造声明值异常的畸形 zip；根治 = xlsx 读面改 zip crate 流式自读，M2+）。**zip64/zip32 exotica 处置状态（R4 自查 + R5 独立复审 → M1.6 封堵，2026-10-26）**：① "声明1+目录2同名"（declared==absorbed）一致性畸形归档被接受——**维持登记（A6 同族，端态固有）**：zip crate 按声明计数读 CD、IndexMap 吸收后单消费者视图一致，无"查A读B"分歧面；M1.6 镜像后该端态可经结构化路径达成的三绕过已封，孤立畸形巧合路径维持登记（威胁低）；② 前置垃圾 zip64——**M1.6 升级：门真跑（fail-closed 放行非跳过）**：守卫镜像 crate 的 [声明偏移, locator) 窗内正向扫，archive-offset 校正语义对齐（前置垃圾 zip64 正向测试 prefix_junk_zip64_gate_active_and_passes）；③ **R5 绕过族三形态——✅ M1.6 已封（测试逐项入仓）**：(a) 尾部垃圾出窗——EOCD 回扫全文件化（V6/V13a/V13b 三档差分负向）；(b) 影子 EOCD——comment 适应检查 + 候选回退循环镜像 crate 语义（V7 负向）；(c) zip32 双计数字段分歧——本盘/总盘不一致即拒 + 比对取本盘字段（V8/V8b 双向负向，新增 EocdCountFieldsDiverge）。另终端语义升级：**候选全部不可验而归档已打开 → EocdUnverifiable fail-closed 拒绝**（旧 Ok(None) 门跳过即三类绕过公共根因，负向在位）。**镜像严格方向残留（均 fail-closed、登记在案）**：crate Detect 猜测语义窗口微差、zip64 内层多候选回退未逐步复刻、prefix-junk 下 CDFH 空间检查用相对偏移近似——三者只会**过度拒绝**恶意构造件，不会 fail-open；④ xlsx 单元格轴负向测试——✅ 已入仓（M1.6：xlsx_cell_flood_rejected_by_post_gate_cell_axis，95,239 行×21 列恰 2,000,019 格，行门静默格门拒绝，R5 X3 直采）；⑤ 压缩比门——✅ 乘法判据已实装（declared > 100×compressed 精确兑现 ">100"，边界对负向在位：100,999 拒/100,000 放行）。
2. **T-04 子进程超时——✅ 已实施关闭（M1.5，2026-10-25；cargo test 9 项 + R3 审计修正加固 + P2-4 drain）**：bid-parse/guard.rs run_limited——① 超时杀**进程组**（process_group(0)+kill(-pgid)——孙进程不残留（setsid 脱组孙进程除外：R5 S6 实测返回仍有界 1.52s、当前三个外部命令不可能从文件内容派生 setsid 进程故生产不可达，注记随 M1.6）；非 unix 降级为只杀直接子，部署面为 Linux）② stdout 上限 + 溢出即时杀（yes 类无限输出不等超时即收割）③ 结果五分类（Success/Timeout/OutputOverflow/NonZeroExit/Busy/Spawn）——超时/溢出/非零**永不误标成功**（timeout_never_lies_as_success 断言）④ 并发全局门（4 席满显式 Busy）⑤ TmpGuard RAII 临时目录全路径回收（超时/失败/正常）⑥ OCR 页数门 200（超出整体显式拒绝——不留部分结果）⑦ stderr 持续 drain（P2-4：原 take 读满即停后关 FD，verbose 合法工具被 SIGPIPE 中杀误判 NonZeroExit——实测修正审计原“阻塞→Timeout”描述；drain 保留头 64KB，256KB 洪泛样本→Success 回归）。内存上界全档化（P2-5）：Success 路径 lossy 3×（192MB）×4 席 ≈ 0.75GB，见 run_limited 文档注释。
3. **T-10 起草层注入（L3 残余，不可穷举）**：已知模式扫描仅告警；唯一收口 = 人工审校门禁（审校态 AI 不可写，T-08 双层门禁）。
4. **T-14 引擎侧 G3**：暗标元数据清零的业务侧缓解（zip 补丁）为过渡——引擎导出面产品化前，每次导出必须跑 T7d 断言（已入每轮回归）。
5. T-15 供应链闭包审查待 M0 执行（NOTICES 机制）。
6. **T-07 文件围栏 TOCTOU（check-then-use v1，#183 实装时登记）**：fs_fence 的 resolve_read/resolve_write 为「解析-校验-使用」三步分离——校验与实际 open 之间路径组件（symlink 改指）理论上可被置换。当前单机单用户部署面（Agent 与用户同 OS 权限）下不构成新增威胁面；根治 = 打开目录句柄后相对路径操作（Unix openat+O_NOFOLLOW 语义），Windows 面用 File::open 后校验 handle 真实路径——按需排期。

> 以上 GAP 与待办已回填 risk-register（R-09/R-10/R-13 相关行注记）；T-03/T-04 已于
> M1.5 完成（真实施装 + 负向测试 + corpus-A 回归全绿，见 verification-plan §6 三级词典）——
> 真实用户文件接入前完成的安全前置已就位。
