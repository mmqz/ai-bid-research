# R6 安全审计（security-audit.md）

基线 6105beb（ai-bid-audit 只读）；引擎仓 mdcore-audit @ f4328ebf（实钉 rev，只读）。
方法：威胁面组织逐条对账——「威胁 | 声称防护（docs/threat-model.md，S1 v1.0）| 实际实现位置 | 验证状态 | 残余风险」。
裁定词典：IMPLEMENTED（带证据）/ PARTIAL（缺口编号）/ NEEDS_ENV（需环境/运行验证）/ ABSENT。
交叉引用：P0-1/P0-2（Lead 已确认）、A-*/B-*/C-*/D-*（四份模块审计）、FG-*（假绿登记册）、M1-M6（变异报告）。

---

## 0. 总裁定一览

| # | 威胁面（threat-model ID） | 裁定 | 一句话 |
|---|---|---|---|
| 1 | 任意路径读写 / symlink / TOCTOU（T-07） | **PARTIAL** | import/export 双层围栏在位；save_version 无围栏（C-06）+ mang-app mkdir 不限 + TOCTOU v1 残余自认 |
| 2 | ZIP 炸弹 / XML 实体 / 嵌套深度（T-03） | **PARTIAL + NEEDS_ENV** | 引擎侧门 0/1/2/3+限额族在位；EOCD 守恒/影子 EOCD/尾部垃圾/压缩比四门无等价实现，zip 4.6.1→8.6 语义差未验证（B-03） |
| 3 | 恶意 PDF（T-02 + 字节门） | **PARTIAL** | 字节门+五分类+pdfium 回退语义在位；引擎路无乱码仲裁（B-12）、加密/损坏件对 poppler/pdfium 行为零测试 |
| 4 | OCR 资源耗尽（T-04 子面） | **IMPLEMENTED（抽查）** | 200 页门活代码在位（历史 S1 死门已修复）；但负向测试 CI 恒 SKIP |
| 5 | 子进程挂起/僵尸（T-04） | **IMPLEMENTED（面）+ PARTIAL（语义）** | run_limited 五分类在位；OCR 逐页 break 截断不可见（B-09）、pdfinfo 失败只 OCR 1 页（B-10） |
| 6 | 并发/版本冲突/并发导出 | **PARTIAL** | 乐观并发/4 席门在位；SDK 侧从不传版本号（C-08/C-09）、游标不防增长位移（C-14） |
| 7 | API Key 泄漏（T-13 近邻） | **IMPLEMENTED（单机口径）** | llm.rs/mang-app 掩码链路实证；明文经 WebView IPC + llm.json 无 0600（A-21/D-17） |
| 8 | Prompt Injection / Agent 越权（T-10/T-11/T-08） | **PARTIAL** | 确定性层结构性免疫成立；receipt 验签 M5 击杀 ✓，但信任根=approval.key 文件可读性（C-10）+ 证据语义全自报（C-23） |
| 9 | 证据伪造（T-12） | **ABSENT（语义面）** | P0-2 完整链成立：FILE_EXISTS 占位证据可绑资格条件清零 blocking，CONTENT_MATCHES 不防语义伪造 |
| 10 | 日志泄漏 / 导出元数据残留（T-13/T-14） | **PARTIAL** | 日志面干净（单机）；暗标 docProps 清零+禁字扫描只在 SEMUL 线（T7d），产品路径无等价物——**本审计新发现** |

---

## 1. 任意路径读写 / symlink / TOCTOU（threat-model T-07）

| 项 | 内容 |
|---|---|
| 威胁 | Agent 经工具面越出工作区读/写任意路径；symlink 逃逸；check-then-use 竞态 |
| 声称防护 | threat-model.md T-07（:41）：「路径面已实施（#183/#189）——import upload_ref 受文件围栏（受信根 BID_FS_ROOT ∪ BID_WS、组件级包含判定防前缀陷阱、词法 `..` 预拒、逐段 symlink 检查）；导出侧 resolve_write 同款纵深；壳层逐件 resolve_read + 服务层持根复验双保险」；状态 IMPLEMENTED（TOCTOU check-then-use v1 残余登记 §4-6，:82） |
| 实际实现 | bid-tools/src/fs_fence.rs（294 行生产 + 355 行测试：逃逸三态/前缀陷阱 work vs work-evil/多根/限额 10 项）；export 服务层组件级 + symlink 复验（C 组对账：输出路径逃逸 ✅）；fs 限额门单件 500MB/批 64 件/累计 2GB（#183，T-05 同源） |
| 验证状态 | **IMPLEMENTED（import/export 主面，证据：bid-tools/tests/fs_fence.rs 全绿 + mutation 白名单未跑但 mang-app 39 测试含围栏面——NEEDS_ENV（CI 零执行，D-11））** |
| 残余风险与缺口 | ① **C-06（P2）**：save_version（bid-mcp/src/tools.rs:1781-1825）是工具面唯一「绝对路径 + 无根校验」写盘口——任意可读 .md/.txt 可探测存在性并在围栏外目录家族落 `.mang/history`；save_version_tool.rs 仅 3 例无围栏负例。② **D-13（P3）**：mang-app fs.mkdir 允许工作区任意路径建目录（fsapi.rs resolve 后无 .mang 检查）——与 writeBytes 限 .mang/（fsapi.rs:251-262）纪律不一致。③ **TOCTOU v1（threat-model §4-6 自认）**：resolve_read/resolve_write 三步分离，校验与 open 之间 symlink 可置换；单机单用户面不构成新增威胁，根治需 openat+O_NOFOLLOW。④ m1 无任何围栏（研究线，D-19）。 |

**裁定：PARTIAL（缺口 C-06/D-13；TOCTOU 登记在案）**

---

## 2. ZIP 炸弹 / XML 实体 / 嵌套深度（threat-model T-03）——重点面

| 项 | 内容 |
|---|---|
| 威胁 | 高压缩比 XLSX/DOCX 撑爆内存；声明值撒谎；EOCD 结构绕过（V6 尾垃圾/V7 影子 EOCD/V8 字段分歧）；XML 深度炸弹/十亿笑声；稀疏包围盒炸弹 |
| 声称防护 | threat-model.md T-03（:32）+ §4-1（:76-77）：「bid-parse/guard.rs 挂门——输入字节门/prescan 三门/压缩比（乘法判据）/实际读取累计计量/**EOCD 守恒门 crate 4.6.1 选择语义全文件镜像**（V6/V7/V8 三绕过已封 + A7 前置垃圾门真跑 + 无可验候选 fail-closed）+ 条目名五类拒绝 + XML 深度 + xlsx 读后行/单元格双轴门；负向测试 18 项全绿」——状态 **IMPLEMENTED+** |
| 实际实现 | **声称的位置已不存在**：bid-parse/src/guard.rs 现内容仅剩 PDF 输入字节门（:64-70 LIMIT_INPUT_BYTES_PDF=500MB）、流式 sha256（:73-86）、子进程治理（:88-336）；头注 :1-21 自认「B4 收敛（issue #11 任务 3）：zip 面无消费方——OOXML 容器安全全链收敛到引擎侧」。引擎侧实况（mdcore-audit/crates/office-io/src/io.rs，rev f4328ebf）：**门 0** 输入字节上限（io.rs:182-190 check_input_size；xlsx 族 LIMIT_INPUT_BYTES=300MB+2MB 边距 :109、docx 族 50MB+2MB :111，四入口统一 :1256-1262/:1294）；**门 1** prescan_family 四检查（io.rs:222-266——条目数 ≤10,000（:113）/单条目声明尺寸/路径逃逸 enclosed_name（:246-251）/总量声明 ≤300MB(xlsx)·50MB(docx)）；**门 2** 谎言头（CountingReader io.rs:322-367 实际>声明即拒，verify_pass :687-691/:727-731；依据注记 :768-771——zip 8.6 读界无声明值强制，由本层补）；**门 3** 部件良构性（validate_xml_structure io.rs:369-428：标签配对/根唯一/截断/深度 ≤1000（:117），GeneralRef 透传不展开——十亿笑声面免疫自注 :371-372）+ **GridScan 稠密包围盒面积门**（io.rs:693-702，LIMIT_TOTAL_CELLS=4M :131；稀疏炸弹 1588B→~550GB 分配 abort 实证闭环 :67-77）；**提取期限额族**：表数 ≤1000（:123，:1325-1330）+ 全簿累计稠密面积（:1340-1349）。docx 读管线复用同门（docx.rs:1729-1742 prescan_docx + verify_gates）。依赖：zip 8.6 / calamine 0.36 / quick-xml 0.41（office-io/Cargo.toml:32-44）。 |
| 验证状态 | **引擎门 0/1/2/3 + 限额族：IMPLEMENTED（静态证据 + 引擎侧内联单测）**；**EOCD 守恒门/影子 EOCD/尾部垃圾/压缩比 100:1 门/条目名五类拒绝/嵌套归载拒绝/zip64 前置垃圾门：未在引擎侧发现等价实现（全 io.rs/docx.rs 无 EOCD 结构校验代码——仅 ContainerCorrupt 透传 zip crate 报错，io.rs:145-146/:1522-1526）→ ABSENT**；zip crate 4.6.1→8.6 EOCD 搜索语义差异未验证 → **NEEDS_ENV** |
| 残余风险与缺口 | ① **B-03（P1）**：历史 V6/V7/V8 三绕过封堵全部挂在已删除的 guard.rs 镜像门上；引擎侧 ratio 门自认不存在（docs/engine-bid-mapping.md §4「S7 定谳 ratio 门引擎**无**（合法 149:1 引擎放行）」）——当前链上压缩比炸弹防线=声明值预扫+读后计量，无比例判据。② zip 8.6 自身 EOCD 搜索语义对 V6（尾部垃圾出窗）/V7（影子 EOCD）/V8（双计数字段分歧）三形态的行为未做敌意件回归——旧镜像门按 zip 4.6.1 语义写就，语义代差无对账证据。③ 唯一曾实测 S7/S8 的对账工具 engine_probe 钉旧 rev 3204342d（scripts/engine_probe/Cargo.toml:16/:21，落后生产 117 提交）——当前 rev 门行为零对账（B-17）。④ 威胁模型文档严重过时（E-1：T-03 整段指向已删除代码；capability-map.md S6/S9 锚点失效）。 |

**裁定：PARTIAL（引擎三门+限额族 IMPLEMENTED；EOCD 守恒族 ABSENT + 语义差 NEEDS_ENV）**
**登记 NEEDS_ENV 变异测试项**（见 false-green-register 修复建议）：以 baseline/security/hostile/ 六敌意件 + V6/V7/V8 再生样本对 mdcore @ f4328ebf 的 read_book/read_docx_from_bytes 做敌意件注入回归；同时验证 zip 8.6 EOCD 搜索语义（尾部垃圾窗口/影子 EOCD 回退/字段分歧）是否 fail-closed。

---

## 3. 恶意 PDF（threat-model T-02 + 输入字节门）

| 项 | 内容 |
|---|---|
| 威胁 | PDF JavaScript/嵌入对象执行；畸形 PDF 卡解析器；超大 PDF OOM；加密件 |
| 声称防护 | T-02（:31）：mdc-pdf ExtractText 仅提取文本（JS 不执行）；poppler 提取路径无执行面；VERIFIED_SOURCE。T-05 超大文件：fs_fence 限额门（#183）。 |
| 实际实现 | ① 字节门：guard.rs check_input_size 读前 metadata 检查（500MB，guard.rs:64-70）；② 旧路径 poppler 全套乱码仲裁链（probe_cjk_garble_pages/互证/OCR 救援/fail-loud，engines/mod.rs:65-207——B 组评「本仓最完整失败语义链」）；③ 引擎路径 pdfium（bid-engine lib.rs:96-100 parse_pdf_via_engine；无 pdfium → EngineFailed → service.rs:376-377 回退旧路径；纯扫描件 any_text_layer=false → MappingGap 回退）；④ pdf_text.rs 3283 行纯 Rust 兜底（NotPdf/Encrypted/NoTextLayer 显式 Err）。JS 不执行：三通道（pdftotext/pdfium/纯 Rust）均为文本提取面，无脚本执行点——静态成立。 |
| 验证状态 | **字节门+回退语义：IMPLEMENTED（service.rs import 路由静态可证 + corpus-verification 运行坐实「无 pdfium → 整体回退 poppler」）**；**引擎路防护质量：PARTIAL** |
| 残余风险与缺口 | ① **B-12（P2）**：pdfium 引擎通道绕过 sanitize_line/乱码仲裁/OCR 救援——控制字符剔除、页数口径、行切分与旧通道不同；环境差异=身份/定位漂移面。② **B-28（P2）**：混合 PDF（文本页+扫描页）两通道都按「任一页 ≥16 非空白→整件文本层」判——扫描页内容静默丢失，无逐页路由。③ 加密/损坏/EOCD-xref 损坏件对 poppler/pdfium 的行为差零测试（B 组缺口 7/8）；500MB 门无端到端负向测试（B-23 附带：sha256 前置双读+全量入内存——门序与声明不符）。④ pdfium ENV_MISSING 的 SKIP 语义使 B5 引擎路径在无动态库环境零覆盖（corpus-verification §3.3——与 T18a 共同构成「引擎路径未被验证」证据链）。 |

**裁定：PARTIAL（B-12/B-28；引擎路 NEEDS_ENV）**

---

## 4. OCR 资源耗尽（T-04 子面：200 页门）

| 项 | 内容 |
|---|---|
| 威胁 | 超长扫描件逐页渲染 OCR 撑爆 CPU/磁盘/时间 |
| 声称防护 | threat-model §4-2（:78）：「⑥ OCR 页数门 200（超出整体显式拒绝——不留部分结果）」；历史审计 S1 曾发现死门问题（R3 轮修复） |
| 实际实现 | guard.rs:114 `MAX_OCR_PAGES: usize = 200`；消费点 tesseract.rs:88-92——`real_pages > MAX_OCR_PAGES` 渲染**前**整体拒绝（Err 含页数与上限，「不留部分结果」）；页数门负向测试 tesseract.rs:281-320（需 python3+reportlab 环境） |
| 验证状态 | **IMPLEMENTED（现状抽查）**：历史 S1「死门」已修复——门在渲染循环之前真实短路，`pages` 字段不再先渲染后判；测试在位但 CI（windows-only、无 reportlab）恒 SKIP。 |
| 残余风险 | ① 页数探测依赖 pdfinfo（B-10：探测失败 `.unwrap_or(1)` 回退 1 页——门本身不会漏放，但产生另一形态静默截断）；② 门只护 OCR 主链，B2 扫描件关键词通道（3 词）不受门影响（无渲染面）。 |

**裁定：IMPLEMENTED（证据：guard.rs:114 + tesseract.rs:88-92；负向测试 NEEDS_ENV）**

---

## 5. 子进程挂起 / 僵尸（threat-model T-04）

| 项 | 内容 |
|---|---|
| 威胁 | 畸形输入卡死 pdftoppm/tesseract；孙进程残留；stderr 管道死锁；临时文件堆积 |
| 声称防护 | threat-model §4-2（:78）：run_limited 七件套——超时杀进程组/溢出即时杀/五分类/并发 4 席/TmpGuard 全路径回收/OCR 200 页门/stderr 持续 drain（P2-4 修正版） |
| 实际实现 | guard.rs:88-336 全套在位且与声称逐项对上：process_group(0)+kill(-pgid)（:167-182；非 unix 降级只杀直接子——Windows 部署需 taskkill /T 等价物，自注 :176-181）；SubprocSlot RAII 4 席（:149-165，MAX_CONCURRENT_SUBPROCS=4）；双读线程+溢出原子标志即时杀（:214-235）；stderr drain 保头 64KB（:236-257）；限时收流补杀组（:284-293）；五分类 RunOutcome（:118-147，timeout_never_lies_as_success 断言 :519-527）；TmpGuard 目录名 pid+seq+nanos 唯一（:314-336，并发交叉覆盖事故修复在案）。11 项负向测试（timeout/overflow/nonzero/success/spawn/busy/stderr 洪泛/tmpguard…）——但整体 `#[cfg(unix)]`（guard.rs:339-341） |
| 验证状态 | **IMPLEMENTED（治理框架）**；**PARTIAL（失败语义面）** |
| 残余风险与缺口 | ① **B-09（P2）**：OCR 逐页循环 NonZeroExit 即 break（tesseract.rs:137-141）——其后所有页不再 OCR，`pages`=全部渲染张数、`ocr_blocks>0` 返回 Ok——部分成功无状态、截断范围不可见。② **B-10（P2/SUSPECTED）**：pdfinfo 失败 `unwrap_or(1)`（tesseract.rs:87/:99）——整件只 OCR 第 1 页，大面积静默内容丢失且 summary 不记探测失败。③ **B-14（P2）**：guard.rs 测试模块 `#[cfg(unix)]`——11 项治理负向测试在仅 Windows 的 CI 不编译不执行，零 CI 守卫。④ B-27：TESSDATA_PREFIX 强制覆盖 ~/.tessdata——系统安装 chi_sim 不被识别（部署面易踩）。 |

**裁定：IMPLEMENTED(治理)/PARTIAL(语义 B-09/B-10)；CI 守卫缺口 B-14**

---

## 6. 并发 / 版本冲突 / 并发导出

| 项 | 内容 |
|---|---|
| 威胁 | 并发写丢失更新；导出竞态；子进程并发洪泛 |
| 声称防护 | （threat-model 未单列——属工具面通用纪律）契约 §0 乐观并发（project_version → E_CONFLICT）；T-04 并发 4 席 |
| 实际实现 | ① 乐观并发：tools.rs 全部写工具 `check_version(project_version)`（import :442-444、extract :680 等——缺省=内部步骤跳过检查，lib.rs `fn n()`）；响应统一回带 content_version（tools.rs:297/:317）。② MCP 层全局 Mutex 串行分发（state.rs——单客户端天然无并发写）。③ 子进程 4 席门（§5）。④ 导出原子性：manifest+tmp+rename（store_io.rs:109-121；A-13：无 fsync）。⑤ check.rs std::thread::scope 有界 8 线程 mpsc（A 组确认为确定性归位）。 |
| 验证状态 | **IMPLEMENTED（机制面；mutation M6 证明状态机跳级拒绝被杀）**；**PARTIAL（实践面）** |
| 残余风险与缺口 | ① **C-08/C-09（P2）**：SDK/checklist.py/minimal_flow.py 版本号顶层取位错——`self.project_version` 恒 None，**乐观并发在官方示例与 Skill 工具链上从未生效**（实践空转）。② **C-14（P2）**：游标 fail-closed 只防列表缩水（offset>total 拒），不防增长位移——连续写间翻页静默给错页。③ **A-13（P2）**：auto_checkpoint 全量快照写放大 + rename 无 fsync——掉电可失最近若干写。④ 并发导出本身经全局 Mutex 串行化——无竞态面（单进程口径）。 |

**裁定：PARTIAL（C-08/C-09/C-14/A-13）**

---

## 7. API Key 泄漏（threat-model T-13 近邻面）

| 项 | 内容 |
|---|---|
| 威胁 | LLM api_key 经日志/错误面/生成内容外泄 |
| 声称防护 | threat-model T-13（:57）日志脱敏清单（M1 trace 合规、全链路 M5）；A 组核实「不写日志/不进生成内容/不落 usage」grep 属实 |
| 实际实现 | ① bid-tools/src/llm.rs：.mang/llm.json 读取（:81-88）；配置错误只提示路径不含 key 值（tools.rs:1936-1941——C-27 核实）；② mang-app diag.rs mask_key（:43-44 `sk-***cdef`，过短全星号）+ llm_status 只出掩码（:135 api_key_hint）+ 测试断言明文不外泄（:327-330「环境变量可能带 key——只断言掩码形态」）；③ MCP 层无日志面、stdio bin 仅 IO 错误 eprintln（C 组日志面核查 ✅）；④ LLM 调用走 HttpTransport（mang-app run_llm_test 后台线程——不冻 UI）。 |
| 验证状态 | **IMPLEMENTED（单机/单租户口径）** |
| 残余风险与缺口 | ① **A-21（P3）**：llm.json 落盘无 chmod 0600（对比 approval.key 有 0600，service.rs:562-568）——同机跨用户可读。② **D-17（P3）**：key 经 WebView IPC 明文传输（设计使然，本机单页面）——若 D-01 注入链成立则 key 可被读取面触及（llm.save 可写）。③ 多租户前瞻：错误信息带项目 ID（C-27，P3）。 |

**裁定：IMPLEMENTED（单机）；卫生项 A-21/D-17**

---

## 8. Prompt Injection / Agent 越权（threat-model T-10/T-11/T-08）

| 项 | 内容 |
|---|---|
| 威胁 | 招标文件内嵌指令操纵 Agent；Agent 伪造人工身份自审；工具参数注入 |
| 声称防护 | T-10 确定性层结构性免疫（:49）；T-11 全参数 schema 化无 shell/自由 SQL（:50）；T-08 双层门禁 human: 前缀（:42） |
| 实际实现 | ① 确定性免疫：解析/抽取全确定性代码，注入文本只是数据——成立（TC-05 规则 8 注入 8 类命中）；起草层 L3 残余靠人工审校（结构性）。② 工具面：25 工具 serde schema 编译期拒绝 + E_STATE 状态机门（mutation M6 杀伤 ✓）。③ 人工授权：receipt HMAC-SHA256 + scope 绑定 + 24h 窗 + 常量时间比对（principal.rs:223-289，A 组评「实现质量高」）；**M5 变异击杀证明**：验签旁路变异被 set_matrix_override_tool.rs:491 forged_receipt_is_rejected 杀伤 ✓；reviewer/principal/operator 参数在场即拒（三工具）；human: 前缀只出现在回显非判据。 |
| 验证状态 | **确定性层+参数面+验签算法：IMPLEMENTED（含变异学证据）**；**信任根与内容绑定：PARTIAL** |
| 残余风险与缺口 | ① **C-10（P2）**：信任根 = approval.key 文件可读性——同 uid（或 root）进程可读 32B key 并伪造任意 actor 的人工授权（sdk/tests/test_mcp_e2e.py:137-150 mint_receipt 直接 read_bytes 即合法签发——e2e 全绿本身即该路径有效性的证明）；stdio 部署形态（agent-integration.md §2 同机同用户起子进程）下 Agent 框架进程具备直接 open 能力；威胁模型未把「宿主与 Agent 进程隔离」列为前置条件。② **A-03（P1）**：receipt 只绑门名不绑内容摘要（消息构成 `{RECEIPT_VERSION}|{actor_id}|{actor_type}|{scope}|{now}`）——24h 窗口内可对未审内容过门（对比矩阵覆写的四重签域绑定）。③ **A-17（P3）**：store_mut() 公开全量可变域 + TrustedPrincipal::human 公开构造不验签——嵌入方可绕过 MCP 面唯一入口保证。④ C-02：官方示例教 Agent 用已废除的 reviewer 参数（方向性错误教学，幸 fail-closed 拦截）。 |

**裁定：PARTIAL（C-10 信任边界 + A-03 内容绑定）**

---

## 9. 证据伪造（threat-model T-12）——P0-2 完整链

| 项 | 内容 |
|---|---|
| 威胁 | 伪造证据文件/证据属性使资格条件、否决项被「已响应」 |
| 声称防护 | T-12（:51）：证据三态+登记时刻指纹+来源登记（sha256 入库）+R-04 引用键纪律——状态 PARTIAL（机制 TESTED；支持率人工协议待 M3） |
| 实际实现 | #185 来源身份（source_file_id 必须指向已导入文件，sha/指纹服务端生成）✓；#217 内容定位（locator 对账 → CONTENT_MATCHES + excerpt 服务端截取）✓——**但证据语义属性（kind/title/subject/scope/validity/client/amount/fields）全部调用方自报**（tools.rs:3784-3880 → register_evidence_verified）。矩阵派生：matrix.rs:176-192 has_ev/has_sec 只看绑定存在+证据状态；bind_evidence 只查 can_bind（evidence.rs:608-659）——**不消费 verification_tier 与内容匹配**（P0-2）。 |
| 验证状态 | **ABSENT（语义防线）**：纯 MCP 工具面、无需任何 receipt 的完整伪造链静态成立（C-23 攻击链：招标文件任意无关段落 locator 锚定 → CONTENT_MATCHES 四态全绿 → kind="qualification"+title="ISO9001…" → bind 资格条件 → blocking 清零）；A-14：tier 下游消费方为零（draft/readiness/validate/export 全不消费）；A-01/A-02：导出门消费存量矩阵+时间过期不翻转——失效证据响应可过导出闸门（P0-2 在导出面残留）。 |
| 残余风险 | mutation 视角：M2/M3 证明 can_bind 门与 blocking 派生有红测试守卫——但这些测试钉的是「过期/Superseded 禁绑」「必选类未响应阻断」，**没有任何测试断言「FILE_EXISTS 占位证据不得清零资格条件 blocking」**（P0-1 的 M1 同型不对称：反向缺陷无测试约束）。 |

**裁定：ABSENT（P0-2 + C-23 + A-14；伪造成本=两次工具调用且全程无审计告警）**

---

## 10. 日志泄漏 / 导出元数据残留（threat-model T-13/T-14）

| 项 | 内容 |
|---|---|
| 威胁 | 证据摘录/密钥经日志泄露；暗标导出件元数据暴露身份 → 废标 |
| 声称防护 | T-13（:57）M1 trace 合规、全链路脱敏 M5；T-14（:58）「**业务侧缓解已验（V1-d）**：docProps 清零 + app.xml Company 移除 + 禁字双面扫描 + 金丝雀防空洞绿——run_tests T7d（每轮回归）TESTED（业务侧）；引擎侧 G3 P0 待排期」 |
| 实际实现 | ① 日志面：MCP 层无日志；trace 仅工具调用摘要（M1）；api_key 全链未见输出（§7）——单机口径干净。② 暗标清理**产品路径实况（本审计复核）**：生产导出（bid-tools/src/export.rs，docx-rs 0.4.22 写面）只有 **anonymize 文本面替换**（公司名→「投标人」，export.rs:909/:1043-1044/:1176——正文与章标题；**图片水印面自认不处理** :1044）；**docProps 清零/禁字扫描在产品路径无等价实现**（export.rs 无 docProps 处理——docx-rs 写 docProps/core.xml 默认 creator/lastModifiedBy="unknown"、docProps/app.xml）；validate 的 rules_profile=dark_bid **显式降级**：「暗标禁字清单扫描在阶段 7 词表面——本轮按 standard 规则集执行」（bid-mcp/src/tools.rs:3484-3488，仅 pre_warning）；生产回读测试只查 word/document.xml 正文公司名零残留（export_readback.rs:766-771/:418——不解 docProps/页眉页脚部件）。**docProps 清零+app.xml Company 移除+全部件禁字扫描只存在于 scripts/v1_darkbid.py（:9-16/:120-126）——SEMUL 仿真导出线（ooxml_util.py 自认仿真），即 threat-model 所称「业务侧缓解已验」验证的是仿真件不是产品件。** |
| 验证状态 | **日志面：IMPLEMENTED（单机）**；**暗标元数据面：PARTIAL（文本 anonymize IMPLEMENTED；docProps/全部件扫描 ABSENT-on-product-path——T-14「业务侧已验」声明与产品实况不符，属文档-代码不一致发现）** |
| 残余风险 | ① 暗标导出件的 docProps（core.xml/app.xml）与页眉页脚部件无禁字扫描防线——公司名若经 format_spec/页眉注入即成暗标识别点（废标级业务风险，R-11 同族）；② 引擎侧 G3 缺口仍开（threat-model 自认）；③ T7d 每轮回归守护的是 SEMUL 线——对产品路径漂移零防护（假绿面，登记 false-green-register）。 |

**裁定：PARTIAL（T-14 产品面 ABSENT——新发现，升 D 组正面登记为「仅 SEMUL 线在位」）**

---

## 附：NEEDS_ENV 变异测试项汇总（安全面）

| ID | 待验证项 | 方法 |
|---|---|---|
| NE-1 | mdcore @ f4328ebf 对 V6（尾部垃圾出窗）/V7（影子 EOCD）/V8（zip32 双计数字段分歧）再生样本的行为 | 敌意件注入 read_book_from_bytes / read_docx_from_bytes，断言 fail-closed |
| NE-2 | zip 8.6 EOCD 搜索语义 vs zip 4.6.1（旧镜像门基线）——注释窗口/候选回退/字段取值差 | 构造差分样本双版本对照（引擎仓只读：以外挂 harness 投喂字节） |
| NE-3 | 压缩比炸弹（>100:1 高熵件）在引擎门 0/1/2 下的行为（S7 定谳：ratio 门引擎无——确认仅声明值+读后计量拦截边界） | 敌意件注入 |
| NE-4 | guard.rs fs_fence/mang-app 围栏面变异（mutation 白名单未执行项——store_io/agent_mandatory 同批） | 原地变异+跑测试（Linux 环境） |
| NE-5 | pdfium 引擎通道对加密/损坏/xref 损坏 PDF 的行为（B 组缺口 7/8） | 配 MDC_PDFIUM_LIB 环境注入坏件 |
| NE-6 | D-01 href 注入链运行验证（属性逃逸→unsafe-inline CSP→IPC 面） | Playwright/WebView 注入样本 |

（本文件未修改两个仓库任何文件；全部结论基于静态阅读 + 既有运行证据引用。）
