# 模块审计报告 E2 — 评测/报告/爬取类脚本（scripts 根级 27 文件 + 3 依赖件）

> 全仓审计 V2 · Task 4-e2（模块审计代理 E2）。
> 基线：main@`6105beb652e88760bdf38f08080af1ed7b1f048a`；工作分支 `audit/v2-full-coverage`（HEAD 含 R0~R1 审计文档；`git diff 6105beb..HEAD -- . ':!audit' ':!*.md'` 为空——生产代码与基线一致，本报告所有结论对 6105beb 有效）。
> 方法：只读逐文件全读（Read/Grep/git ls-files），**未运行任何脚本**（纪律遵守）。产物：本文件 + audit/inventory/inventory-e2.csv。
> 编号：SCR-100 起（与 E1 的 SCR-0xx 不冲突）。

---

## 0. 覆盖清单与边界

**已审计（27/27，全量逐行读）**：

| 组 | 文件 | 行数 |
|---|---|---|
| 评测链 | real_eval_score.py / real_eval_gt_enrich.py / baseline_run.py / engine_diff.py / b5_engine_gt_eval.py / fp_sampling.py / grade.py | 190/165/622/754/125/143/751 |
| 报告分析 | title_impact_score.py / title_impact_summary.py / m17_inventory.py / m17_real_chain.py | 121/118/197/569 |
| 历史管线 | v1_citation.py / v1_corpus_r.py / v1_darkbid.py / v1_export.py / v1_pipeline.py / v2_impact.py / v3_integration.py | 164/814/213/467/426/328/253 |
| 爬取 | telecom_fetch.py / telecom_refetch.py / telecom_prepare_files.py / telecom_chain.py / telecom_file_level.py / telecom_gt_draft.py | 215/127/130/255/233/244 |
| GitHub 工具 | gh_audit.py / gh_deep.py / gh_search.py | 101/101/52 |

**依赖件（附带覆盖，非 E2 清单）**：scripts/baseline_harness/src/main.rs（84 行，全读）、scripts/title_impact/src/main.rs（头部+装载逻辑）、bidcore/crates/bid-tools/src/bin/real_eval.rs（头部+路由，real-eval 产物的真实生成器）。

**不在 E2 范围**（git ls-files scripts/ 核对后移交）：run_tests.py/fixture_spec.py/fixture_gen.py/ooxml_util.py/syntax-check.sh/synth_tender_*/xref_check.py/patch_docs_r10.py（E1 已认领）；scripts/veto_gt/*、scripts/veto_measure/*、scripts/bid_health_check/*（veto/健康检查链，E3）；scripts/engine_probe/*（B3 引擎侧 dump，属 R7 引擎线）；scripts/e2e_bid/*、scripts/mang-dev/*、scripts/fp_attribution/*（其他模块线）。E1/E3 覆盖核对建议见 §6。

---

## 1. 头号问题：Oracle 同源性总判定（§3.11）

**结论：本审计范围内的评测链不存在"用生产 veto_lexicon/veto_rule 同源算法生成真值再自证"的结构性问题；三套真实 GT 均为独立于生产抽取算法的草案源。但存在三层级弱化，R4/R5 引用前必须逐条消化。**

逐链判定：

1. **real_eval_gt_enrich.py（GT 富化）**：不调用 veto_lexicon/veto_rule，不用生产抽取算法生成真值。它只做四要素补全：excerpt 对回 parse 文本层三级定位（verbatim→norm→关键词窗，L121-136）、rationale 硬编码人工理由表（L56-81）、review_status=draft_pending_human（L145）、争议注记（L36-44）。GT 本体是 `artifacts/real-eval/ground-truth-draft.json`（P2 第一轮 **AI 初标草案，标注者=Z.ai**，REPORT.md 头部声明）。判分口径（real_eval_score.py 的互含+12 字符滑窗对齐，L64-79）是独立 Python 实现（norm_n2 复刻生产语义但用途是对齐器而非真值生成）。**同源性：无。弱化点：GT 未人工冻结（SCR-108）。**
2. **real_eval_score.py（判分）**：被测对象 = `artifacts/real-eval/extract/requirements.json`（由 bid-tools `real_eval.rs` 真跑生产 `bid_parse::extract_requirements` 生成——真实生产实现）；真值 = AI 草案 24 条；判分器独立。**自证程度：判分逻辑独立，GT 独立性为"AI 标注+待人工"，引用支持率判据（锚点页±1，L81-99）独立于生产。**
3. **baseline_run.py → baseline_harness（Rust）**：Cargo.toml path 依赖 `../../bidcore/crates/bid-parse`，main.rs L15 `use bid_parse::parse_file` —— **直接调用当前生产实现，无仿真复制**。双跑一致性 = 同一实现的确定性检验（非正确性 Oracle，但诚实标注为"复现基线"）。
4. **b5_engine_gt_eval.py**：经临时 cargo example 同时跑 `bid_engine::parse_docx_via_engine` 与 `bid_parse::parse_file` 两条**真实生产路径**（L41-58），GT=fixtures/corpus-A/ground-truth/requirements.jsonl（GT-25，fixture_spec 人工设计真值）。判分=条款号+互含锚。
5. **grade.py**：对齐器 `aligned_gt_ids`（L213-260）是独立实现且带反作弊（反标记回声 L196-198/自报 gt_ids 不计入；全文锚点 3× 长度上限 L217-218；垃圾合并密度门 L267-279）；GT=fixture_spec REQUIREMENTS（人工设计）。破坏注入自测 11 例（L570-593+echo 注入 L664-672）。
6. **telecom_gt_draft.py**：GT 18 条硬编码于脚本（L79-210），excerpt 逐字取自真实公告/文件，locate 预核（L66-76）；不调用生产算法。**同源性：无。**
7. **m17_real_chain.py**：GT 24 条为 committed jsonl（AI/人工直录，无生成脚本）；P4 逐字读回核对。**同源性：无。**
8. **对照（历史教训）**：artifacts/bid-health-check/REPORT.md 的否决召回 0.386 用的是 **LingYan 20 词反查表近似真值**（REPORT §局限 1 自认"词表命中块≠人工标注否决条款"）——这正是"关键词同源近似真值"的反面案例，属 E3 范畴；本范围脚本未重蹈。

**三层级弱化（R4/R5 必读）**：
- **L-1（最重）GT 全部未冻结**：real-eval 24 条（7 条争议在案）+ m17 24 条 + telecom 18 条均 `draft_pending_human`，无人工验收记录 → 所有真实文件准确率（75%/50%/70.8% 等）只能作探索性指标（各报告均已诚实冠名，但 R4/R5 不得当验收基线引用）→ SCR-108。
- **L-2 影响面/内核语义验证是设计复刻而非生产实现**：m17_real_chain 的 RealChainEngine（L206-290）、v2_impact 的 ImpactEngine（L41-184）、v3_integration 的 LinkageLedger/ExportGate（L45-111）均为 bid-domain 设计语义的 **Python 复刻（SEMUL/DESIGN_ENGINE，文件头诚实声明）**，不调用 bid-tools 的 Rust 生产实现；"Rust 交叉验证待 cargo 环境"的主张已过期（run_tests T19 已跑 cargo test --workspace）→ SCR-103。
- **L-3 合成 GT 与抽取器同团队设计**（corpus-A GT-25/grade.py）：双实现交叉纪律已在 verification-plan 声明，属已知边界。

---

## 2. 可重放性矩阵（能否从 HEAD 6105beb 重新生成报告）

| 脚本/报告 | 离线可重放 | 说明 |
|---|---|---|
| baseline/（139 文件，含 MANIFEST/parse×124/security×6） | ✅ 需 cargo | baseline_run.py 删目录再生（L437-441），harness path 依赖当前 bid-parse。**但已提交快照锚定 rev `8ec7f9a…`（≠HEAD）+ mdcore 远端 rev 探测**（SCR-105）；env 含 ls-remote 网络触点（L119，可 BASELINE_MDCORE_REV 钉住） |
| m17_real_chain → real-chain-report.json | ✅ 全离线 | 输入（real-corpus/gov-procurement files+sources+gt）全部入库；run_tests T14b 门禁 |
| telecom_chain → chain-report.json（46 断言） | ✅ 全离线（需 pdftotext） | sources/ 20 节点 JSON+镜像入库；T16b 有 pdftotext 环境门（缺→SKIP） |
| telecom_gt_draft → gt/ground-truth-draft.jsonl | ✅ 离线 | parse-input/ 17 件已入库；locate 通道 pdftotext/zip 直读 |
| grade.py --self-test | ✅ 纯离线 | T 门禁在 run_tests L514 |
| engine_diff --selftest | ✅ 纯离线（16 合成例） | T17 消费；--pair-dir 全量差分需 engine-probe dump（引擎线） |
| v1/v2/v3 管线 | ✅ 需 python-docx/PyMuPDF/openpyxl/LibreOffice | run_tests T 面内（L545-640）；产物落 artifacts/v1 |
| title_impact 链（summary.json + gt-recall-by-variant.json） | ✅ 需 cargo | reproduce 字段已写明（summary L74-78）：title_impact harness 真跑 extract_requirements × baseline/parse 块流；experiment.json（7MB）gitignored 但可再生 |
| b5_engine_gt_eval | ✅ 需 cargo | T18（400s 超时）；有生产树临时写入副作用（SCR-102） |
| **real_eval_score → score-report.json** | ⚠️ **半** | extract/parse 产物可由 `cargo run -p bid-tools --bin real_eval -- <samples> <out>` 再生，**但样本目录 `/home/z/my-project/repos/sunShineLoveMe__ai_bidding_power_grid/test_samples/water_tender_docs` 为仓外绝对路径，本环境不存在**；且 pdftotext 回查失败会静默降级（SCR-101）；产物无 rev 登记（SCR-107） |
| m17_inventory → inventory/chains/stats.json | ❌ | 依赖外部仓 tlyyxjz__BidAgent（`/home/z/my-project/repos/`，本环境缺失，脚本会显式 return 1——失败响亮）；已入库 inventory 为历史快照（T14a 冻结断言 576 份） |
| telecom_fetch / telecom_refetch / telecom_prepare_files | ❌（网络） | 重取协议在 real-corpus/telecom/README.md 落档；refetch 双哈希校验 + HASH_DRIFT 非零退出（纪律好） |
| **telecom_file_level → file-level-report.json** | ⚠️ **陷阱** | parse-input/ 已入库，**但 materialize() 先 rmtree 提交目录再从 gitignored 的 extracted/converted/files/ 复制**——净克隆上重跑会摧毁 parse-input/ 并用残缺报告覆盖 file-level-report.json（SCR-100，P1） |
| gh_* → research/raw/* | ❌（网络+GH_TOKEN） | 绝对出仓路径（SCR-106） |

---

## 3. 逐文件审计结论

### 3.1 评测链

**baseline_run.py**（622 行，SCR-105）— B1 基线冻结编排。✅ 调用真生产（baseline_harness→bid_parse::parse_file）；✅ 失败件也落盘不许缺（L513-514 assert）；✅ 双跑 digest 比对排除 elapsed_ms（L52/69-71）；✅ 敌意件确定性构造 6 类（zip 炸弹声明面补丁 L309-331/路径穿越/重复条目/截断/billion laughs/稀疏 xlsx）。副作用：删并重建 baseline/{parse,parse-full,security}（纯生成物目录，合理）。机器假设：~/.cargo/bin、~/.tessdata/chi_sim、/usr/bin/time、外部 pdftotext/pdfinfo/tesseract。网络触点：`git ls-remote https://github.com/mmqz/mdcore.git`（L119，仅记录）。SKIP 语义：无（工具缺失→ENV_BLOCKED.md 清单，非假绿）。

**real_eval_score.py**（190 行，SCR-101/107）— 判分器独立、GT 草案口径诚实（L147 gt_status；L163 误报不自动判定）。引用支持率严格判据（锚点页±1，L92-96）+ supported_elsewhere 单列（防任意页恒真）。**缺陷**：L28 仓外绝对样本路径；L58-60 `subprocess.run(["pdftotext",…])` 不查 returncode——PDF 缺失时 pages=[""]→supported 全 False 的静默假降级。写 artifacts/real-eval/score-report.json。

**real_eval_gt_enrich.py**（165 行，SCR-110）— 幂等（excerpt_original 首轮留存 L116-118）；"定位存疑"条目如实登记不伪装（L142）；退出码 0 允许存疑条目存在（设计声明 L19）。副作用：**原位覆写已入库的 ground-truth-draft.json**（L158）。norm_n2 标注错误（L85 声称复刻 bid-parse norm_n2，实现是 NFKC+去空白=生产 norm_n1 语义；bid-parse/src/lib.rs L143-153 可证）。无 veto 同源。

**engine_diff.py**（754 行）— 纯差分/判定器（不跑生产，消费两份 JSON）；自测 16 例覆盖四分类+兜底+v2/v5 规则族（L515-699）；ASCII 假等价防御（L603-607）。注释含历史引擎 rev `338acdc`（L11，SCR-116 级）。写 artifacts/engine-diff（默认 --pair-dir）。

**b5_engine_gt_eval.py**（125 行，SCR-102）— 真生产双路径对比；判定=引擎路径内容锚召回≥旧路径（L118-121，回归即 exit 1）。cargo 三通道定位（L25-35，CI 兼容）。**副作用**：向生产 crate 树写临时 example `bidcore/crates/bid-engine/examples/gt_eval_dump.rs`（L60-62）再删（L80 finally）——中断残留风险 + 冻结审计树上属生产文件写入。

**fp_sampling.py**（143 行）— 确定性噪声正则分类 + seed=20261025 分层抽样（可复现）；与 real_eval_score 同源对齐口径（L67-72，自洽声明）；AI 复核样本明确"待人工验收"。无风险点。

**grade.py**（751 行，SCR-109）— 七判分项全确定性；反作弊三件套（对齐器唯一可信源/类别一致门/密度门）；类型 lint 防布尔伪装（L351-353）；幂等字段缺失判 FAIL（L366-374，R2 P1-3 修复在位）；场景口径 TC-06/08/11 互斥处理（L468-483）。SKIP 语义：G5 engine_gap→skipped 列表（L391-393，不计失败但披露）；metric 断言 pass=None 中性（L441-444）；核心断言路径不可解析即 FAIL（L452-455，R2 P1-2 修复在位）。**runs/ 目录 gitignored → `--run` 无已入库判分对象，门禁内只有 --self-test**（自测用 _perfect_db 合成产物，是判分器自证而非运行验证——设计如此，双实现链在 v1/v2/v3）。

### 3.2 报告/分析

**title_impact_score.py / title_impact_summary.py**（121/118 行）— 判分对齐复制自 real_eval_score（L4 明示"同仓判分器复制，非语义复刻"）；summary 与 harness 计数一致性断言（L48-51，防口径漂移——好纪律）；reproduce 字段落档。GT 同为 AI 草案（探索性）。依赖 gitignored 的 experiment.json（可再生）。

**m17_inventory.py**（197 行，SCR-115）— 只读外部仓（声明）；输出入库 inventory/chains/stats。失败响亮（缺目录 return 1）。576 份快照被 T14a 冻结断言（历史数字绑定，可接受——有 sha256 溯源字段）。

**m17_real_chain.py**（569 行，SCR-103）— 真实数据（三节点 ccgp 官方文本 + cuc.edu.cn 官方镜像 HTML 字节存档交叉校验 L375-382——双官方源，好纪律）× **设计语义引擎**（RealChainEngine L206-290，Python 复刻六规则）；34 断言全硬判定无 SKIP；双跑确定性（L496-498）；诚实三层口径（L13-22：REAL_DATA+DESIGN_ENGINE / 公告级≠文件级 / GT 未冻结）。**验证对象是设计而非当前 Rust 实现**——文件头自认"Rust 层交叉验证仍待 cargo 环境"已过期（T19 已具备 cargo）。

### 3.3 历史管线 v1/v2/v3（仍为活入口，非失效）

run_tests.py L545-640 直接 import 并调用：v1_pipeline.build_parse_report（T 面内）、v1_export.build_bid_docx/run_fidelity_checks、v1_citation.run_citation_checks、v1_darkbid.run_darkbid_checks、v2_impact.run_v2、v3_integration.run_v3。**调用路径完好，依赖 python-docx/PyMuPDF/openpyxl/reportlab/PIL/LibreOffice 环境面**。

- **v1_pipeline.py**：独立 Python 解析实现（python-docx/PyMuPDF/openpyxl）——**双实现交叉验证的设计本意**（与生产 Rust 解析器互为独立实现），不是生产调用的缺失而是刻意独立。G7 归因（页眉页脚/文本框/批注无定位器变体）诚实落档。
- **v1_export.py**：技术标构建器+两级保真（python-docx 结构读回 + LO 渲染）；LO 产物 PDF 字节非确定→断言取属性不取哈希（诚实边界声明）。
- **v1_citation.py**：R1/R2/R3/R3b 引用链断言（SEMUL 口径明示）；书签再寻址恢复语义。
- **v1_darkbid.py**：暗标五断言含双金丝雀（内容面 D4 + 标识符面 D5 注入式——防空洞全绿，好纪律）；IDENT_HOST_TOKENS 宿主指纹扫描（SC-06）。
- **v1_corpus_r.py**（E1 边界件，头部已读）：确定性合成 corpus-R；`NOTO_PATH=/usr/share/fonts/truetype/chinese/SarasaMonoSC-Regular.ttf` 机器本地字体路径（SCR-111）；像素级自证断言防可变字体空白回归。
- **v2_impact.py**：SEMUL ImpactEngine + TC-09 契约消费；R6 删除/新增规则显式标 SYNTHETIC。
- **v3_integration.py**：SEMUL LinkageLedger/ExportGate 三问断言；"引擎本体接入后升 ENGINE_RUN"未发生（仍是 SEMUL）。

### 3.4 爬取链 telecom_*（重点安全）

**链路完整性**：获取（fetch：门端列表→详情→文件下载 md5 校验）→ 复现（refetch：按 fileId 重取双哈希校验）→ 准备（prepare：解压+LO 转换留痕）→ 物化+真跑（file_level：cargo realfiles 真生产路由 + 独立 Python 文本通道锚点核验）→ 链验证（chain：46 断言）→ GT 草案（gt_draft：18 条四要素+定位预核）。**README.md 落档重取协议与入库政策（sources ✅/files·extracted·converted ❌ gitignore/parse-input ✅）——链路设计完整、溯源字段齐全（URL/sha256/md5 双记录）**。

- **telecom_fetch.py**（215 行，SCR-104/113）：免鉴权公开接口；md5+size 双校验；幂等声明（但实际每轮重写 fetch-report.json——门端列表随时间滚动，README 已声明"链节点以已入库 sources 为准"）。**问题**：①重跑会覆写已入库 sources/bulletin-door.json 并新增滚动节点（证据污染面）；②无 robots.txt/请求节流（仅文件下载间 0.5s sleep，L203；API 调用无间隔）；③UA 伪装 Mozilla（L33）；④无重试（异常→ERROR 状态记录）；⑤L96-97 MD5_MISMATCH 文件**仍写盘**（docstring L17"不一致判 FAIL 不入库"与实现不符——files/ gitignored 故 git 意义上"不入库"成立，但本地盘面不符）；⑥allow_large=True 无大小上限。
- **telecom_refetch.py**（127 行）：只写 gitignored files/（不触碰已入库 sources/fetch-report ✅）；sha256 对首轮实测 + md5 对平台 keyNum 双校验；HASH_DRIFT 显式登记且非零退出（L122-123，不静默沿用旧值——好纪律）。无请求间隔（P3 级礼貌性）。
- **telecom_prepare_files.py**（130 行，SCR-113）：`z.extractall(dest)`（L47）解压互联网 zip——CPython stdlib 自带 `..` 组件清洗（_extract_member 过滤 curdir/pardir）故路径穿越被 stdlib 兜底，**但无解压比/总大小守门**（对比：生产 bid-parse 有 zip 炸弹守卫，此准备脚本无）；rar 显式登记不解。LO 转换源/产物 sha256 双留痕 ✅。
- **telecom_chain.py**（255 行，SCR-114）：全离线消费已入库 sources；P0-9 诚实登记节点 2 为 SNAPSHOT_ONLY；**节点 1/3 证据源为第三方镜像 zhiliaobiaoxun 字节存档**（GT-T01 source_url 自注"镜像；法定媒介：联通采购与招标网/txzbqy/cebpubservice"）——镜像≠法定媒介原件，仅节点 4 为 txzbqy 官方 JSON；退出码纪律严格（fail 即 1）。P4 读 file-level-report.json（不重跑 Rust——见 T16c 说明）。
- **telecom_file_level.py**（233 行，SCR-100，**P1**）：`materialize()` L106-107 `shutil.rmtree(PARSE_INPUT)` 后从 **gitignored** 的 converted/extracted/files 复制 17 件 → 净克隆重跑 = 摧毁已入库 parse-input/（17 件真实文件）+ cargo 跑空目录 + 用残缺结果**覆写已入库 file-level-report.json**（L226-227）。正向：cargo realfiles 是真生产路由（docx/xlsx→bid-engine 引擎门面，pdf→旧路径，与 service.rs 同款——realfiles.rs 头部可证）；锚点核验用独立 Python 通道（docx zip XML/pdftotext，L142-167）——**公告↔文件双向独立核验，Oracle 独立性好**。
- **telecom_gt_draft.py**（244 行，SCR-112）：GT 18 条硬编码于脚本（L79-210），excerpt 逐字锚 + 三级定位预核 + draft_pending_human；定位 FAIL 即 exit 1（不放过关 ✅）。docstring"24→N 条"与实际 18 不符。**人工标注状态声明：全部 18 条 draft_pending_human，未冻结**（T16d 只锁结构与定位状态）。

### 3.5 GitHub 工具 gh_*

- **Token 处理**：三个脚本均 `os.environ.get("GH_TOKEN","")`（gh_audit L7 / gh_search L6；gh_deep 不用 token 走 raw.githubusercontent）——**无硬编码凭据、无 token 落盘/回显，代码面无泄漏点** ✅。
- **gh_audit.py**（101 行，SCR-106）：L14-15 无条件发送 `Authorization: Bearer {TOKEN}`——TOKEN 为空串时发送空 Bearer → GitHub 返回 401，脚本把 401 当 NOT FOUND/ERROR 处理（无匿名回退——功能缺陷）；0.4s 间隔（L101）；rate_limit 打印。OUT=`/home/z/my-project/download/bid-agent-research/research/raw`（L9）——**仓外绝对路径**（机器本地假设；该目录本环境不存在）。
- **gh_deep.py**（101 行）：raw 内容抓取+outline；写入同 OUT；glob 匹配 audit.json 树缓存（L82-93 脆弱但有容错）；无请求间隔。
- **gh_search.py**（52 行）：10 个搜索查询无间隔（未认证时 search API 限 10/min——易触发限流）；写 search_results.json 至仓外。
- **"六条修订"来源判定**：**gh_audit.py 不是六条修订的来源工具**。六条修订来自 issue #203（全仓审计旧计划 issue，证据 audit/evidence/github/issue-203.json，historical-context §1#14a/§3 摘要）。gh_*.py 是竞品调研期的仓库取证采集器（README/文件树/提交抓取落 research/raw 供引用）——属研究资料工具，与审计计划修订无关。

---

## 4. 硬编码过期 SHA / 历史运行数字专项清点

| 位置 | 内容 | 性质 |
|---|---|---|
| baseline/MANIFEST.json env.ai_bid_research_rev | `8ec7f9a6710c056412e9773f3dbad64dfccd42ca`（git 验证为真实祖先提交，issue#2·PR#3 时代） | **快照锚定旧 rev**（非脚本硬编码）；title-impact/engine-diff 的对照基线即此快照（SCR-105） |
| baseline/MANIFEST.json env.mdcore_main_rev_record_only | `f8c860aa…`（远端探测记录） | record-only，可用 BASELINE_MDCORE_REV 钉住 |
| scripts/engine_diff.py L11 | 注释引用引擎 A2 族 rev `338acdc` | 历史注释（描述性，非判定依赖） |
| artifacts/title-impact/summary.json method | "块流取自 baseline/parse 118 件基线" | 历史快照口径（当前 baseline 124 件）——结论绑定生成时点，reproduce 可再生 |
| artifacts/real-eval/score-report.json | 17/24 召回等（2026-10-25 schema 1.0 定稿口径） | 历史运行数字；**产物无生成 rev/指纹登记**（SCR-107）；gt_status 诚实标注 |
| artifacts/real-eval/REPORT.md | 三轮修复数字并列（首轮/三轮/R3 修正后），均标注口径 | 诚实的历史对照表 |
| real-corpus/gov-procurement/stats.json + T14a | 576 份（224/174/178）冻结断言 | 历史快照断言（防漂移用途，可接受） |
| artifacts/bid-health-check/REPORT.md | 否决召回 0.386（竞品仓 da58e64 + LingYan 词表 de909f1） | **旧 SHA 数据**（E3 范畴；本范围无脚本引用它） |
| scripts 内 | **无任何脚本硬编码 6105beb 或要求特定 rev** | — |

---

## 5. 问题记录（SCR-100 ~ SCR-116）

---
ID: SCR-100
严重程度: P1
模块: scripts/telecom_file_level.py
文件路径: scripts/telecom_file_level.py
代码行号: L105-117（materialize：L106-107 rmtree；L25-61 MATERIALIZE 源表）、L226-227（覆写 REPORT）
函数或脚本: materialize() / main()
问题描述: materialize() 先 `shutil.rmtree(PARSE_INPUT)` 删除**已入库**的 real-corpus/telecom/parse-input/（17 件真实招标文件，4.6MB，B1.5 起入库），再从 **gitignored** 的 converted/、extracted/、files/ 复制重建。MATERIALIZE 的 17 个源路径全部位于 gitignore 目录。
调用链: README 重取协议（telecom_refetch→prepare→file_level）→ materialize → run_rust → 覆写 file-level-report.json
触发条件: 在净克隆（或未先跑 refetch+prepare 的环境）直接重跑 `python3 scripts/telecom_file_level.py`
实际影响: 已入库 parse-input/ 17 件被删（git 可恢复但工作树受损）；cargo 对空/残缺目录跑 realfiles；残缺结果覆写已入库 file-level-report.json（T16c 消费该文件——静态断言将随之失败或误绿）；退出码 1。
复现步骤: 净克隆仓 → 运行 telecom_file_level.py（审计纪律下未执行，静态推演：materialize 17/17 MISSING → run_rust 空目录 → verify 全 file_ok=False）
当前测试: run_tests T16c 仅读已入库 file-level-report.json（不重跑本脚本）
缺失测试: 无"净克隆重放不摧毁入库语料"的守门测试
建议方案: materialize 改为先校验源存在性（缺失即硬退出、不动 parse-input）；或源全缺失时直接用现存 parse-input 跑 run_rust；报告先写临时文件校验通过后再落盘
证据: `if os.path.exists(PARSE_INPUT): shutil.rmtree(PARSE_INPUT)` + MATERIALIZE 首项 `"converted/1d77120d6d-0.招标文件-…docx"`（converted/ 在 .gitignore）
审计置信度: CONFIRMED
状态分类: 当前仍存在
---
ID: SCR-101
严重程度: P2
模块: scripts/real_eval_score.py
文件路径: scripts/real_eval_score.py
代码行号: L28（SAMPLES 绝对路径）、L58-60（pdftotext 无 returncode 检查）、L91-99（依赖 pages_n2 的判据）
函数或脚本: main()
问题描述: 引用支持率回查通道依赖仓外绝对路径 PDF + pdftotext 子进程，且不检查 returncode/空输出——PDF 缺失或 pdftotext 失败时 pages=[""]，supported 静默全 False，报告无声降级而非报错。
调用链: main → subprocess pdftotext → pages_n2 → citation_supported/supported_elsewhere/page_delta
触发条件: 外部样本仓 /home/z/my-project/repos/sunShineLoveMe__ai_bidding_power_grid 缺失（本审计环境实测缺失）或 pdftotext 失败
实际影响: 再生成 score-report.json 会得到 citation_support 0% 的错误指标（召回不受影响）；无异常抛出，消费者难察觉
复现步骤: 在无该外部目录环境运行脚本，观察无报错但支持率全 False
当前测试: 无（T15 只锁 GT 草案结构，不跑本脚本）
缺失测试: pdftotext 通道有效性断言（returncode/pages 非空校验）
建议方案: 检查 p.returncode 与 pages 非空，失败即 SystemExit 并在报告标注 CHANNEL_MISSING；样本路径改为参数化
证据: `pages = subprocess.run(["pdftotext","-layout",str(PDF_SRC),"-"], capture_output=True).stdout.decode(...).split("\f")`（无 returncode 检查）
审计置信度: CONFIRMED
状态分类: 当前仍存在
---
ID: SCR-102
严重程度: P2
模块: scripts/b5_engine_gt_eval.py
文件路径: scripts/b5_engine_gt_eval.py
代码行号: L60-62（example.write_text）、L80（finally unlink）
函数或脚本: extract_reqs()
问题描述: 向生产 crate 源树写入临时 Rust example（bidcore/crates/bid-engine/examples/gt_eval_dump.rs）后删除——对冻结/审计树构成生产文件写入副作用；进程被杀时残留文件会改变 bid-engine crate 编译面。
调用链: main → extract_reqs → 写 example → cargo run -p bid-engine --example gt_eval_dump → finally unlink
触发条件: 运行脚本（含 run_tests T18）
实际影响: 运行期间生产树出现新增源文件；中断即残留（下次 cargo build 把它编进 examples）；对只读审计/CI 缓存不友好
复现步骤: 运行中途 kill 脚本，检查 examples/ 目录残留
当前测试: T18（400s 超时内正常路径会清理）
缺失测试: 残留文件守门（cargo clippy/tree diff）
建议方案: 改用 bid-tools 常驻 bin（如 real_eval.rs 同款模式）或 CARGO_TARGET_DIR 隔离的独立小 crate；至少启动时清理陈旧残留
证据: `example = REPO / "bidcore/crates/bid-engine/examples/gt_eval_dump.rs"; example.write_text(code)`
审计置信度: CONFIRMED
状态分类: 当前仍存在
---
ID: SCR-103
严重程度: P2
模块: scripts/m17_real_chain.py / scripts/v2_impact.py / scripts/v3_integration.py
文件路径: scripts/m17_real_chain.py L206-290（RealChainEngine）、scripts/v2_impact.py L41-184（ImpactEngine）、scripts/v3_integration.py L45-111（LinkageLedger/ExportGate）
代码行号: 见上行；v2 L305-306 semul_note；m17 L16（"Rust 层交叉验证仍待 cargo 环境"）、L545 caliber
函数或脚本: RealChainEngine / ImpactEngine / LinkageLedger / ExportGate
问题描述: 影响面六规则与文档内核三问的验证对象是**设计语义的 Python 复刻（SEMUL/DESIGN_ENGINE）**，非当前生产 Rust 实现（bid-tools 已有 readiness/addendum 语义实现与测试）。"待 cargo 环境"的免责声明已过期（run_tests T19 现已跑 cargo test --workspace）。
调用链: run_tests（T-m17/T-v2/T-v3）→ run()/run_v2()/run_v3() → Python 引擎复刻
触发条件: 任何引用 M1.7/V2/V3 结论作为"生产引擎已验证"的场景
实际影响: 这些全绿证明的是设计语义正确性（SEMUL 级），不能升格为生产实现 ENGINE_RUN 级证据；R4/R5 若引用需按三级分记口径降级表述
复现步骤: 读文件头 semul/caliber 声明 + 对照 bidcore/crates/bid-tools/src/readiness.rs（Rust 实现在位）
当前测试: run_tests T14b/T-v2/T-v3（Python 复刻路径）；T19（Rust 面独立覆盖，但与六规则断言集非同一契约）
缺失测试: SEMUL 断言集与 Rust 实现的逐规则交叉验证（同输入对拍）
建议方案: 把六规则/三问断言改写为对 bid-tools Rust API 的直调（或契约 JSON 双跑对拍），SEMUL 降为设计文档；更新过期免责声明
证据: v2 semul_note："影响面规则为 bid-domain-model §2 设计语义的 Python 复刻；bidcore/bid-domain Rust 同语义交叉验证待 cargo 环境"
审计置信度: CONFIRMED
状态分类: 文档与实现不一致（声明过期于当前环境事实）
---
ID: SCR-104
严重程度: P2
模块: scripts/telecom_fetch.py
文件路径: scripts/telecom_fetch.py
代码行号: L121-151（门端列表+详情覆写 sources/）、L17（docstring"不一致判 FAIL 不入库"）vs L96-97（MISMATCH 仍写盘）、L203（仅下载间 0.5s）、L33（UA 伪装）、L196（allow_large=True）
函数或脚本: main() / fetch_file()
问题描述: ①重跑覆写已入库 sources/bulletin-door.json、publicity-door.json 并新增随时间滚动的 bulletin-*.json（证据面污染）；②爬取合规面：无 robots.txt 检查、API 调用无间隔（仅文件下载 0.5s）、UA 伪装、无重试；③docstring 与实现不符（MD5_MISMATCH 文件仍落盘 files/）
调用链: main → api_json 门端 → 覆写 sources/*.json → fetch_file → 写 files/ + fetch-report.json（覆写已入库报告）
触发条件: 重跑 telecom_fetch.py
实际影响: 已入库证据被滚动数据稀释/覆盖（README 声明"链节点以已入库 sources 为准"可缓解但文件面仍被改）；对平台的请求礼貌性依赖 0.5s 单点
复现步骤: 重跑脚本前后 git status real-corpus/telecom/sources/
当前测试: 无
缺失测试: 入库证据不可变守门（fetch 不应触碰已入库 sources；应写新目录或先备份）
建议方案: fetch 输出到带时间戳的新目录或检测已入库文件即只读；robots.txt 检查 + 全局请求间隔；docstring 修正；MISMATCH 文件加 .mismatch 后缀隔离
证据: `with open(os.path.join(SRC, fn), "w") as f: json.dump(det, f, …)`（fn=bulletin-*.json，SRC=sources 已入库目录）
审计置信度: CONFIRMED
状态分类: 当前仍存在
---
ID: SCR-105
严重程度: P2
模块: baseline/（由 scripts/baseline_run.py 生成）
文件路径: baseline/MANIFEST.json（env 块）；scripts/baseline_run.py L115-141（env_fingerprint）
代码行号: MANIFEST env.ai_bid_research_rev=8ec7f9a6710c056412e9773f3dbad64dfccd42ca
函数或脚本: env_fingerprint() / 生成快照的提交
问题描述: 已提交的 baseline 快照（139 文件）生成于旧 rev 8ec7f9a（≠ 冻结基线 6105beb），语料计数（124 件）与当前 HEAD 语料（real-corpus 已增长）不一致；title-impact/engine-diff 等下游以该快照为对照基线。
调用链: baseline_run.py → baseline/ → title_impact harness（"118 件基线"口径）→ summary.json；engine_diff --old baseline/parse/*
触发条件: 引用 baseline 数字或以其为差分基线的任何报告
实际影响: 对照结论绑定历史快照；从 HEAD 重生成 baseline 会得到不同 digests/counts（合法的基线演进，但未在任何报告中标注"快照 rev≠HEAD"）
复现步骤: python3 读 MANIFEST env（审计已完成：8ec7f9a）；git log 验证为祖先提交
当前测试: 无快照 rev 与 HEAD 一致性提示
缺失测试: 报告侧引用 baseline 时强制披露 env.ai_bid_research_rev
建议方案: baseline_run 输出时在 stdout/ENV_BLOCKED.md 提示"当前 HEAD≠快照 rev"；下游报告（title-impact/engine-diff）方法字段自动带 rev
证据: MANIFEST：`"ai_bid_research_rev": "8ec7f9a…"`；counts total=124/ok=50/failed=74
审计置信度: CONFIRMED
状态分类: 当前仍存在
---
ID: SCR-106
严重程度: P2
模块: scripts/gh_audit.py / gh_deep.py / gh_search.py
文件路径: scripts/gh_audit.py L7-9/L13-30/L101；scripts/gh_deep.py L6；scripts/gh_search.py L6-8
代码行号: gh_audit L9 `OUT="/home/z/my-project/download/bid-agent-research/research/raw"`
函数或脚本: api()/audit()/main()
问题描述: ①产物落盘于**仓外绝对路径**（download/bid-agent-research——另一项目的目录，本机不存在需 makedirs 兜底，跨机器不可移植且证据不入本仓）；②gh_audit/gh_search 在 GH_TOKEN 为空时仍发送 `Authorization: Bearer `（空凭据头）→ GitHub 一律 401，无匿名回退（功能缺陷：未设 token 时全列表误报 NOT FOUND/ERROR）；③gh_search 10 连查询无间隔（未认证 search 限 10/min）；gh_deep 无间隔。
调用链: main → api()（Bearer 头恒发）→ 写 OUT（仓外）
触发条件: 未设置 GH_TOKEN 或在非原机器运行
实际影响: 无 token 环境下工具静默失效为全 404/401（可误判为仓库不存在）；证据文件散落仓外不受版本控制
复现步骤: `GH_TOKEN= python3 scripts/gh_audit.py some/repo`（审计未运行，静态推演：空 Bearer 头 → 401）
当前测试: 无
缺失测试: 空 token 匿名回退与 OUT 可配置测试
建议方案: TOKEN 为空时不加 Authorization 头；OUT 改为 repo 相对路径或参数化；查询间加 sleep
证据: `req.add_header("Authorization", f"Bearer {TOKEN}")`（TOKEN 可为 ""）
审计置信度: CONFIRMED（401 行为为 GitHub API 既定语义）
状态分类: 当前仍存在
---
ID: SCR-107
严重程度: P2
模块: artifacts/real-eval/*（scripts/real_eval_score.py 的输入）
文件路径: artifacts/real-eval/extract/requirements.json、artifacts/real-eval/parse/*.json
代码行号: （产物文件；生成器 bidcore/crates/bid-tools/src/bin/real_eval.rs）
函数或脚本: bid-tools real_eval（生成）/ real_eval_score.py（消费）
问题描述: P2 真实文件评测的中间产物（extract/parse）**不含生成 rev/参数指纹**——requirements.json 无元数据头；无法从产物本身判定它由哪个 HEAD 的生产抽取器生成（REPORT.md 文字记录 2026-10-26，但产物自证缺位）。
调用链: cargo run -p bid-tools --bin real_eval → extract/parse → real_eval_score.py → score-report.json
触发条件: 任何"当前 HEAD 重放一致性"核对
实际影响: score-report 与 REPORT.md 是历史运行数字，无法机器验证其对 HEAD 的代表性；重放依赖外部样本目录（SCR-101）
复现步骤: 检查 requirements.json 首元素（纯 REQ 数组，无 provenance 块——审计已完成）
当前测试: T15 只锁 GT 结构
缺失测试: 产物 provenance 断言（rev/样本 sha256 清单）
建议方案: real_eval.rs 输出包一层 {generated_at_rev, samples:[{path,sha256}], reqs:[…]}；score 脚本校验样本指纹后再判分
证据: requirements.json 直接以 `[{"id":"REQ-0001",…}]` 开头，无元数据
审计置信度: CONFIRMED
状态分类: 当前仍存在
---
ID: SCR-108
严重程度: P2
模块: 评测链 GT（real-eval / m17 / telecom 三套）
文件路径: artifacts/real-eval/ground-truth-draft.json；real-corpus/gov-procurement/gt/ground-truth-draft.jsonl；real-corpus/telecom/gt/ground-truth-draft.jsonl
代码行号: 各文件 enrichment/条目 review_status 字段（real_eval_gt_enrich.py L145 统一置 draft_pending_human）
函数或脚本: real_eval_gt_enrich.py / telecom_gt_draft.py / m17 GT（直录）
问题描述: 三套真实文件 GT（24+24+18 条）**全部处于 draft_pending_human、无人工冻结记录**；real-eval 侧 7 条争议（GT-T02 改判候选/GT-B04 类别裁决等）悬置。冻结协议写在 enrichment.protocol 字段但未见执行证据。
调用链: R4/R5 评测若以这些 GT 为真值基线
触发条件: 将 75%/50%/70.8% 等草案对齐指标用于验收结论
实际影响: 评测链的最大可信度缺口：指标口径已诚实冠名"探索性"，但下游（文档/审计引用）易丢失该限定；Oracle 独立性本身无同源问题，缺的是人工冻结这一环
复现步骤: 检索三文件 review_status 值（审计已完成：全部 draft_pending_human）
当前测试: T15a/T16d/T14 P4a 锁定"草案+四要素"状态（正确地未当验收）
缺失测试: 人工冻结流程产物（冻结清单/裁决记录）——人工侧动作
建议方案: 执行冻结协议（逐条复核→接受/退回→gold 标记+冻结 commit）；冻结前所有报告头部机器可读字段 EXPLORATORY=true
证据: `"review_status": "draft_pending_human"`（三套 GT 逐条）
审计置信度: CONFIRMED
状态分类: 当前仍存在（人工流程缺位）
---
ID: SCR-109
严重程度: P3
模块: scripts/grade.py
文件路径: scripts/grade.py
代码行号: L391-393（engine_gap→skipped）、L441-444（metric→pass=None）、L462-465（pass=n_fail==0）、L473-477（runs/ 不存在即 SystemExit）
函数或脚本: grade_transitions() / grade_run()
问题描述: SKIP 语义三处：①engine_gap 断言进 skipped 列表不计失败（有披露）；②metric 断言 pass=None 中性（注明由 TC-10 专项承载）；③`--run` 依赖 runs/<id>（gitignored）→ 门禁内只有 --self-test 可执行，七判分项对真实运行产物的判分不在 CI 门禁内。
调用链: run_tests L514（--self-test）/L419（--run，产物由测试内构造）
触发条件: 引用"七判分项全绿"作为运行验证时
实际影响: 自测绿=判分器正确性，≠真实流水线产物判分通过（runs 不入库，历史运行不可复查）
复现步骤: grade.py --run runs/demo（净克隆下 SystemExit"运行产物不完整"）
当前测试: T-门禁覆盖 --self-test 与测试内构造 run
缺失测试: 入库一份最小合规 run 产物（或 db.json 夹具）供 --run 门禁
建议方案: 提供 fixtures 下的最小 runs 样本入库，T 面加 --run 判分用例；skipped/metric 在 verdict 中单独披露计数
证据: `if kind == "engine_gap": skipped.append(…); continue`；`results.append({… "pass": None, "note": "A/B 指标比较由 TC-10 专项报告承载"})`
审计置信度: CONFIRMED
状态分类: 当前仍存在（设计内披露，但门禁面缺口真实）
---
ID: SCR-110
严重程度: P3
模块: scripts/real_eval_gt_enrich.py
文件路径: scripts/real_eval_gt_enrich.py
代码行号: L84-87（norm_n2 实现）vs L85 docstring；对照 bidcore/crates/bid-parse/src/lib.rs L143-153
函数或脚本: norm_n2()
问题描述: docstring 声称"与 P2 评测同源归一化（bid-parse norm_n2 语义复刻）：NFKC + 去空白"——实现（NFKC+去空白、保留标点）实为生产 **norm_n1** 语义；生产 norm_n2 是 norm_n1+去标点；real_eval_score.py 的 norm_n2（NFKC+仅字母数字）才是真 norm_n2。同一定义在两个脚本里语义不一致。
调用链: main → 三级定位 L125-127（normalized / normalized_hard 两级实际跨用了 n1/n2 语义）
触发条件: 富化定位统计（normalized vs normalized_hard 的归类口径）
实际影响: 定位方式统计标签与真实归一化级别错位（不影响 PASS/FAIL 结论——三级匹配是包含关系）；文档与实现不一致的维护隐患
复现步骤: 对照三处 norm_n2 定义（gt_enrich/score/bid-parse lib.rs）
当前测试: T15a 锁 anchor_stats 总量
缺失测试: 归一化函数与生产语义的一致性单测
建议方案: gt_enrich 的函数改名 norm_n1 并修 docstring；或统一 import 单一实现
证据: gt_enrich：`return re.sub(r"[\s\u00a0]+","",unicodedata.normalize("NFKC",s))` vs score：`"".join(c for c in nfkc if c.isalnum())`
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
---
ID: SCR-111
严重程度: P3
模块: scripts/v1_corpus_r.py（及 v1 系列环境面）
文件路径: scripts/v1_corpus_r.py L31
代码行号: `NOTO_PATH = "/usr/share/fonts/truetype/chinese/SarasaMonoSC-Regular.ttf"`
函数或脚本: 模块级常量
问题描述: 机器本地字体绝对路径（含 SAR 环境特定字体族）；v1/v2/v3 系列另依赖 LibreOffice、PyMuPDF(fitz)、python-docx、openpyxl、reportlab、PIL、numpy——无环境探测与降级声明（对比 baseline_run 的 ENV_BLOCKED 纪律）。
调用链: v1_corpus_r 生成 D5 扫描层 → v1_pipeline probe → run_tests T 面
触发条件: 异机器/CI 无该字体
实际影响: corpus-R 重生成失败或字体回退导致像素断言漂移（有 _assert_pixels_text 自证兜底）
复现步骤: 无该字体环境运行 v1_corpus_r
当前测试: T 面内执行（依赖原机器环境）
缺失测试: 字体存在性预检
建议方案: 字体路径探测+候选列表+缺失即显式退出；LO/fitz 缺失时 SKIP 声明
证据: 见行号引用
审计置信度: CONFIRMED
状态分类: 当前仍存在
---
ID: SCR-112
严重程度: P3
模块: scripts/telecom_gt_draft.py
文件路径: scripts/telecom_gt_draft.py
代码行号: L3（docstring"24→N 条"）vs L79-210（GT_ITEMS 实际 18 条）
函数或脚本: GT_ITEMS / main()
问题描述: 文件头声称"GT 草案（24→N 条）"与实际 18 条不符（24 是政采链 GT 的条数，串写）；GT 数据硬编码于脚本体内而非独立数据文件——数据与代码混合，修订 GT 需改代码。
调用链: main → 遍历 GT_ITEMS → locate 预核 → 写 gt/ground-truth-draft.jsonl
触发条件: 阅读/修订 GT
实际影响: 轻微（条数口径混淆 + 维护形态欠佳）；T16d 断言 18 与实现一致
复现步骤: 数 GT_ITEMS（7+7+2+2=18）
当前测试: T16d（len==18）
缺失测试: —
建议方案: docstring 修正为 18 条；GT 迁移到 jsonl 数据文件由脚本读取（与 m17 形态对齐）
证据: `"""M1.7-TEL C批次：通信行业真实样本 GT 草案（24→N 条，四要素纪律）"""`
审计置信度: CONFIRMED
状态分类: 文档与实现不一致
---
ID: SCR-113
严重程度: P3
模块: scripts/telecom_prepare_files.py / scripts/telecom_fetch.py
文件路径: scripts/telecom_prepare_files.py L43-47；scripts/telecom_fetch.py L75-97
代码行号: `z.extractall(dest)`；`fetch_file(fid, allow_large=True)`
函数或脚本: unzip_all() / fetch_file()
问题描述: 对互联网下载 zip 执行无守门 extractall——路径穿越由 CPython stdlib 的 _extract_member 过滤 `..` 兜底（无显式防护注释/断言），但无解压总大小/压缩比上限；fetch 默认 allow_large=True 无文件大小上限（SKIP_LARGE 分支仅在显式传 False 时生效且阈值 30MB）。
调用链: telecom_fetch 下载 → telecom_prepare_files.unzip_all → extracted/
触发条件: 平台分发超大 zip / 炸弹 zip
实际影响: 磁盘耗尽型风险（非生产解析面——生产 bid-parse 有守卫，此为语料准备面）；86MB+83MB 实际规模可控
复现步骤: 构造高压缩比 zip 放入 files/ 后运行 prepare（审计未运行）
当前测试: 无
缺失测试: 解压比/总大小守门
建议方案: extractall 前累计 uncompressed_size 阈值校验；fetch 默认大小上限参数化
证据: `with zipfile.ZipFile(p) as z: … z.extractall(dest)`
审计置信度: CONFIRMED
状态分类: 当前仍存在
---
ID: SCR-114
严重程度: P3
模块: scripts/telecom_chain.py
文件路径: scripts/telecom_chain.py L5-9（节点来源）、L80-86（P0-8 镜像存档断言）
代码行号: 节点1/3 `zhiliaobiaoxun` 镜像；节点4 txzbqy 官方 JSON
函数或脚本: p0()
问题描述: 四节点链的节点 1/3 证据为第三方聚合站镜像（zhiliaobiaoxun）字节存档，非法定媒介原件（法定媒介=联通采购与招标网/cebpubservice——GT-T01 source_url 自注）；仅节点 2 诚实登记 SNAPSHOT_ONLY、节点 4 为官方平台。镜像经 sha256 字节存档（mirror-manifest.json）但镜像本身可被上游篡改。
调用链: p0 → txt_of(sources/unicom-bj-bpm-*.txt)（镜像转文本）
触发条件: 以该链作"法定媒介溯源"表述时
实际影响: 溯源强度降级为"镜像存档级"（README/JSONL 有标注，但 chain-report 的 nodes 字段未逐节点标注 evidence_tier）
复现步骤: 读 sources/ 文件头 URL 字段（bj.zhiliaobiaoxun.com）
当前测试: T16b（46 断言含 P0-8 存档在案，不校验媒介法定性）
缺失测试: 节点级 evidence_tier 字段（official/mirror/snapshot）
建议方案: chain-report nodes 增列 evidence_tier；法定媒介原件获取列入人工事项（同 GT-M17-24 形态）
证据: `"source_url": "https://bj.zhiliaobiaoxun.com/article/113463125（镜像；法定媒介：联通采购与招标网/txzbqy/cebpubservice）"`
审计置信度: CONFIRMED
状态分类: 当前仍存在（已部分披露）
---
ID: SCR-115
严重程度: P3
模块: scripts/m17_inventory.py
文件路径: scripts/m17_inventory.py L30-31
代码行号: `EXTERNAL_RAW = Path("/home/z/my-project/repos/tlyyxjz__BidAgent")`
函数或脚本: main()
问题描述: 依赖仓外外部仓库绝对路径（本审计环境实测缺失）；inventory/chains/stats.json 为历史快照（576 份）被 run_tests T14a 冻结断言绑定。
调用链: main → 遍历 _w3/_w4/_w5_raw → 写 real-corpus/gov-procurement/*.json（覆写已入库文件）
触发条件: 重跑（外部仓缺失时 return 1——失败响亮 ✅；存在时覆写已入库 inventory）
实际影响: 可重放性受限（一次性盘点）；重跑在源存在时会覆写入库快照
复现步骤: 无外部仓环境运行（实测：`ls /home/z/my-project/repos/` 为空）
当前测试: T14a 读 stats.json（静态断言）
缺失测试: —
建议方案: 快照型脚本加 --dry-run/版本化输出目录；路径参数化
证据: 见行号引用
审计置信度: CONFIRMED
状态分类: 当前仍存在
---
ID: SCR-116
严重程度: P3
模块: scripts/engine_diff.py（及 artifacts/engine-diff 快照）
文件路径: scripts/engine_diff.py L11；artifacts/engine-diff/*.diff.json
代码行号: `引擎 A2 族（338acdc）已剥离`
函数或脚本: 模块 docstring
问题描述: 注释引用历史引擎 rev 338acdc（描述性）；committed diff 产物以 baseline/parse 旧快照+当时引擎 dump 为输入——结论绑定历史对照对。
调用链: --pair-dir → baseline/parse（8ec7f9a 快照）× engine dump
触发条件: 引用 engine-diff 产物数字时
实际影响: 轻微（结论需带"快照对照对"口径）；自测 16 例保证判定器本身可重放
复现步骤: 读 diff.json 头部 file 字段与 baseline MANIFEST rev
当前测试: T17（selftest）
缺失测试: —
建议方案: pair-dir 报告头部记入两侧产物 rev/时间
证据: 见行号引用
审计置信度: CONFIRMED
状态分类: 当前仍存在
---

**正向确认（无问题项）**：fp_sampling.py（确定性+seed 落档）、telecom_refetch.py（双哈希纪律+非零退出）、grade.py 反作弊注入自测（11 例破坏全捕获）、v1_darkbid.py 双金丝雀防空洞绿、telecom_file_level.py 锚点核验独立双通道、m17_real_chain.py 官方镜像交叉校验、baseline_run.py 失败件落盘+双跑 digest、gh_* 无凭据硬编码。

---

## 6. 移交与后续动作

1. **R4/R5（测试真实性）**：T14b/T16b 是真实重跑（m17_real_chain/telecom_chain 全离线可重放）；**T15a/T15b/T16a/T16c/T16d 是对已入库产物的静态断言**（GT 草案/fetch-report/file-level-report 不在门禁内重生成）——引用时须区分"重跑绿"与"快照绿"；T16b 受 pdftotext 环境门（缺→SKIP）。
2. **E3**：veto_gt/veto_measure/bid_health_check（0.386 健康检查，LingYan 词表近似真值）为 Oracle 同源性问题的另一主战场——本报告仅确认评测链（real-eval/m17/telecom/grade）未使用同源算法。
3. **E1**：run_tests.py 的 SKIP 统计口径（T16b env_ok 分支、T19 cargo 缺件 SKIP）与 grade.py skipped 语义的联动判读归 E1；本报告提供 SCR-109 事实。
4. **人工侧（最高优先建议）**：执行三套 GT 冻结协议（SCR-108）——这是评测链从"探索性"升"验收性"的唯一通道，纯代码侧无法替代。
5. 建议排期修复顺序：SCR-100（P1，重放摧毁语料）→ SCR-101/102/104（重放与副作用面）→ SCR-103/107/105（证据分级与 provenance）→ 其余 P3。

> 审计置信度总注：本报告全部结论为静态阅读推演（未运行任何脚本，纪律遵守）；标注 NEEDS_RUNTIME_VERIFY 的项无（所有引用的行为均有代码行号与入库产物双证）；SCR-106 的 401 行为依据 GitHub API 公开语义推定。
