# 模块审计 E1-b：scripts 根级测试基础设施（10 文件）

- 审计代理：4-e1（ai-bid-research 全仓审计 V2，Task ID: 4-e1）
- 基线：main@6105beb（分支 audit/v2-full-coverage）
- 纪律：只读审计；未修改任何生产/测试/配置/数据；**未运行 run_tests.py 及任何改文件的脚本**（patch_docs_r10.py 只做静态行为分析）；未运行测试。
- 审计方式：声明范围 10 文件逐行全读（run_tests.py 845 / fixture_spec.py 722 / synth_tender_spec.py 752 / fixture_gen.py 486 / synth_tender_gen.py 466 / xref_check.py 310 / ooxml_util.py 192 / patch_docs_r10.py 92 / syntax-check.sh 43 / fixtures-requirements.txt 15）；调查链读取 grade.py 头部与函数面、v1_pipeline.py 头部、b5_engine_gt_eval.py 全文、.github/workflows/windows-check.yml、.gitignore、baseline/ENV_BLOCKED.md 尾部、bidcore 消费面。

---

## 0. 覆盖声明

| # | 文件 | 行数 | 覆盖 |
|---|---|---|---|
| 1 | scripts/run_tests.py | 845 | 全读 |
| 2 | scripts/fixture_spec.py | 722 | 全读 |
| 3 | scripts/fixture_gen.py | 486 | 全读 |
| 4 | scripts/ooxml_util.py | 192 | 全读 |
| 5 | scripts/syntax-check.sh | 43 | 全读 |
| 6 | scripts/synth_tender_spec.py | 752 | 全读 |
| 7 | scripts/synth_tender_gen.py | 466 | 全读 |
| 8 | scripts/xref_check.py | 310 | 全读 |
| 9 | scripts/patch_docs_r10.py | 92 | 全读 |
| 10 | scripts/fixtures-requirements.txt | 15 | 全读 |

---

## 1. run_tests.py（本地测试总入口）——判定语义与覆盖面

### 1.1 跑什么（T0~T19，README 口径 59 项检查点）

- T0 环境契约预检（Python 钉版六包 / poppler×3+tesseract+soffice / chi_sim / SarasaMonoSC 字体 / cargo）；
- T1/T2 xref_check 常规+注入；T3 夹具完整性（manifest 双向 sha256）；T4 埋点读回+反标记回声+T4c-OCR 回读；
- T5 grade.py 判分器自测；T6 夹具重生成哈希一致（临时目录）；
- T7 V1 四段（v1_*.py）；T8 V2 影响面（v2_impact.py）；T9 V3 内核三问（v3_integration.py）；
- T10~T13 **m1 并行线**（cargo build/test + 四场景判分 + 锚点覆盖 + 双跑字节一致）；
- T14/T15/T16 M1.7 真实链（m17_real_chain / telecom_chain / GT 草案断言，多为**盘内 artifacts 静态断言**+Python 脚本回归）；
- T17 engine_diff 自测 + **冻结工件复断言**（artifacts/engine-diff*/summary.json 计数）；
- T18 b5_engine_gt_eval（真调 bidcore cargo）；T19 bidcore cargo test --workspace（真调）。

### 1.2 判定语义（问题 3 的裁决）

**SKIP 计为通过**：`skip()` 只入 `_skipped`，最终 `return 0 if not _failed else 1`（run_tests.py:834-841）——退出码 0 在有 SKIP 时照常返回。这是 issue #9 的**显式设计**（「依赖缺失 ≠ 业务失败」），汇总行单列「N 通过 / M 跳过 / K 失败」+ 跳过清单打印 + ENV_BLOCKED.md 留痕，可审计性尚可；但**门禁语义上 SKIP=绿**：缺 tesseract/chi_sim 的环境 T4c-OCR 跳过、缺 soffice/pdftotext 整个 T7 跳过、缺 OCR 链整组 T10~T13 跳过、缺 cargo T19 跳过——仍 exit 0。无「SKIP>0 禁发版」强模式。且仓库**无 Linux CI**（唯一 workflow windows-check.yml 只跑 bidcore cargo），该 exit code 目前无任何 CI 消费——「CI 门禁用」仅为文档宣称。见 SCR-001。

### 1.3 覆盖缺口（问题 3）

- **web-ui**：完全不在 run_tests.py（typecheck/build/bench 全无任何入口）；
- **mang-app**：T19 跑 `bidcore cargo test --workspace`——mang-app 是独立 workspace（windows-check.yml:89-95 注释自证「bidcore 的 --workspace 不含它」），故桌面壳零覆盖；
- **sdk/**：sdk/tests/test_mcp_e2e.py 不在入口；
- scripts/ 下 6 个 Rust 工程（baseline_harness/bid_health_check/engine_probe/title_impact 等）同样不在。
- T7/T8/T9 被测物是 **Python 语义仿真**（v1_pipeline/v1_citation/v1_export/v1_darkbid/v2_impact/v3_integration 用 python-docx/PyMuPDF/openpyxl + ooxml_util SEMUL 复刻设计语义；v2_impact.py:306 自注「Python 复刻 = SEMUL 口径，交叉验证待 cargo 环境」）——生产 Rust 面实际只由 T18/T19（及 bidcore 侧 cargo test / e2e_m1）承载。见 SCR-004。

### 1.4 副作用清单（问题 3）

| 副作用 | 位置 | 状态 |
|---|---|---|
| runs/TC-* 工件（m1 5 文件×4 场景 + report.json） | REPO/runs | gitignored（.gitignore:6）——受控 |
| **baseline/ENV_BLOCKED.md 追加写入** | run_tests.py:100-112 | **被 git 跟踪的文件被测试弄脏**（现存 2026-10-28 条目为证）——SCR-002 |
| m1/target/、bidcore/target/ 构建产物 | — | gitignored |
| /tmp/_b2_ocr.png、/tmp/_b2_ocr_input.png | run_tests.py:265-270 | 固定名、不清理、多用户碰撞——SCR-007 |
| bidcore/crates/bid-engine/examples/gt_eval_dump.rs 临时写入 | b5_engine_gt_eval.py:60-80 | 写生产源码树、finally 删除——SCR-011 |
| 失败后清理：无统一清理；T6/T7/T13 tempdir 自动清；t_m1 内 T11 场景失败提前 return → T12/T13 不跑、runs/ 留半成品 | run_tests.py:467-471 | ——SCR-008 |

### 1.5 机器本地/绝对路径依赖（问题 3）

`/usr/share/fonts/truetype/chinese/SarasaMonoSC-Regular.ttf`（T0d + fixture_gen.py:49 + synth_tender_gen.py:46）、`/usr/bin|/bin/tesseract`（_b2_ocr_roundtrip:247-251）、`~/.tessdata`（T0c）、`~/.cargo/bin/cargo`（T0e/_cargo）、`/tmp/_b2_ocr*.png` 固定名。Debian/Linux 强假定，Windows 不可跑。缺件时多数有响亮红或 SKIP（T0d 字体为响亮红 check，不 SKIP——字体缺失会让 T6/T7 生成失败）。

---

## 2. 夹具与 GT 独立性（问题 4）

### 2.1 corpus-A（fixture_spec.py / fixture_gen.py）

- **独立性成立（spec-first 单一真源）**：fixture_spec.py 是纯 authored 数据（722 行零文件 IO，rg 验证无 open/write/subprocess）；A_DOC/C_DOC/B1_ROWS/B3_ROWS/EVIDENCE_FILES/REQUIREMENTS/QUERIES/TRANSITIONS 全部手写常量；fixture_gen.py 消费它生成实体文件与 GT/查询/转换。**夹具不是生产输出的回放**——生产（m1/bidcore）不参与生成任何夹具或 GT。
- GT（requirements.jsonl）与判分器（grade.py）与生产抽取（m1/bid-parse extract）三方不同源；T4a 把 GT excerpt 对**源文件**逐字读回（防答案对不上材料）；T4h 反标记回声扫描防语料泄 GT。
- 保留注记（非缺陷）：m1/bid-parse 的抽取规则是**对着同一份夹具结构开发**的（m1 extract 规则硬编码 corpus-A 前缀，见 M1-003）——GT 数据独立 ≠ 算法未见语料；过拟合风险由 corpus-R/synth-tender/real-corpus 三层补充承接（synth-tender README:326 自declare「本夹具不替代 corpus-A……本件测业务全链+真实结构+已知缺口行为锁定」）。
- 确定性纪律：freeze_zip（固定时间戳+条目排序）+ reportlab invariant + 钉版六包（fixtures-requirements.txt）+ manifest sha256 双向校验（T3a 含反向游离文件检查）；B2/E 扫描件有像素级自证（防可变字体静默空白回归，v0.4 教训固化）。
- 风险：**无 --out 时默认原地重生成仓内 fixtures**（fixture_gen.py:451 `out = FIXTURES`）——见 SCR-010。

### 2.2 synth-tender（synth_tender_spec.py / synth_tender_gen.py）

- GT（expected-*.json）来源：**人工核对答案**（95+3 条 ideal 分类 + 跑前人工推演 engine_expect），在 spec 内与内容同源 authored，excerpt_key 全部经自检是源文本子串（synth_tender_gen.py:379-384）；评测消费方是 **bidcore cargo test `synth_tender_eval`**（README:302），非 m1。
- 防循环论证纪律在案：engine_expect「跑后不许回改」；v0.2（commit d2c8198）补登 3 条（SYN-A-013a/013b/015a）**如实标注**「engine_expect 按 2026-10-30 引擎实测登记（当时已判对）——非独立跑前推演，在此如实标注，不冒充预判」，且冻结预演总账（EXPECTED_ENGINE_TOTAL=81）保持 v0.1 原样不含补登条目（self_check:415-429 有推演账平校验）。
- 结论一行：**GT 独立性成立（人工核对、非引擎输出回放）**；唯一保留：3 条补登的 engine_expect 属跑后回填，已有显式诚实标注，预演账不回改——方法论可接受，登记备查。

### 2.3 判分器自测（T5）边界

grade.py --self-test 的完美库由 GT+判分器常量构造（同源自洽），验证的是判分器破坏捕获力；生产面验证由 e2e_m1（bidcore）与 T11（m1）真跑承载。该边界在 bid-grade 审计（GRADE-002）已登记，不重复立项。

---

## 3. ooxml_util.py（问题 5）

- **能力三块**：① freeze_zip/freeze_docx/XLSX_FIXED_CORE——OOXML 字节级确定性（固定 1980-01-01 时间戳 + 条目名排序 + core.xml 补丁，tmp.replace 原子替换）；② fnv64——mdcore evidence::Fingerprint 的逐位复刻，**带公开测试向量 import 期断言**（防实现漂移）；③ Locator 五变体 emit/resolve 语义仿真（valid/stale_content/stale_position 三态，与引擎 refresh_document 语义对齐声明）。
- **安全**：zip 操作只处理自产文件（freeze 输入是 python-docx/openpyxl 刚写的文件）；条目按名排序重写无路径逃逸放大面；`patches.get(info.filename) or zin.read(...)` 的 `or` 在补丁值为空字节时会回落原数据（理论缺陷，实际补丁恒非空）；无解压（只读+重写），无 zip 炸弹面。
- **定位**：capability-map S11/L1 已判为「SEMUL 仿真层 = B4 删除对象」，但**当前是 load-bearing**：被 fixture_gen / synth_tender_gen / v1_pipeline / v1_citation / v1_export / v1_darkbid / v1_corpus_r / v3_integration 消费（rg 确认 9 个消费方）——删除须先把 freeze_zip（夹具确定性根基）迁走、把 v1/v3 的 Locator 断言迁到引擎侧。登记 SCR-014。

---

## 4. xref_check / patch_docs_r10 / syntax-check（问题 6）

- **xref_check.py**：只读检查器（C1~C8 跨文档一致性：风险↔用例双向、宇宙断言防双表一致删除、AD/状态词典/expected-transitions 覆盖）；`--inject` 自测**只改内存字符串**（inject_historical_errors 返回替换后文本），不写任何文件——安全。C7 的未登记状态词仅告警不阻断（有 allowlist + "PV" 前缀豁免的宽松分支，属告警口径）。无风险立项。
- **patch_docs_r10.py**：**一次性文档改写脚本滞留仓内**——见 SCR-009（绝对路径指向不存在的历史树 /home/z/my-project/download/bid-agent-research；会写 research/verification-plan.md、test-plan.md、README.md；带 assert 锚点但旧树存在时会改错对象）。**本审计未运行它**。
- **syntax-check.sh**：rustfmt parser 级语法检查（秒级、不编译）——见 SCR-012（覆盖面与自称不符 + rustfmt 缺件 exit 0 的 SKIP=绿语义）。

---

## 5. scripts 根级脚本是否调用当前生产实现（问题 7）

| 脚本 | 被测物 | 调生产？ |
|---|---|---|
| run_tests.py T1~T9, T14~T17 | xref/夹具/判分器/Python 仿真/盘内 artifacts | 否（Python 面 + 静态断言） |
| run_tests.py T10~T13 | **m1/ 研究线**（cargo build/test + m1 二进制四场景） | 是（但对象是遗留原型，非生产 bidcore） |
| run_tests.py T18 | b5_engine_gt_eval.py → cargo run bid-engine example（真引擎+旧路径双跑） | **是（bidcore 生产）** |
| run_tests.py T19 | bidcore cargo test --workspace | **是（bidcore 生产）** |
| v1_*.py / v2_impact.py / v3_integration.py | Python 复刻（SEMUL，自注在案） | 否 |
| grade.py | 冻结夹具判分（独立 oracle） | 否（设计如此） |
| 无任何脚本调用 "bin real_eval" 形态的独立评测二进制 | — | — |

结论：本地 Python 全链路以**仿真为主、生产为辅（仅 T18/T19）**；synth-tender 的评测消费在 bidcore cargo test 侧（synth_tender_eval）而非 scripts。

---

## 6. 问题记录

---

ID: SCR-001
严重程度: P1
模块/文件路径/代码行号/函数或脚本: scripts/run_tests.py:66-97, 834-841（skip/require_tools_or_skip/main 退出码）
问题描述: SKIP 判定语义 = 计为通过：`return 0 if not _failed else 1`——环境缺件（tesseract/chi_sim/soffice/pdftotext/cargo 任一）时 T4c-OCR、T7 整组、T10~T13 整组、T16b、T19 整组被 SKIP，退出码仍 0。设计意图（issue #9「依赖缺失≠业务失败」）可审计（跳过清单打印 + ENV_BLOCKED 留痕），但无「SKIP>0 不得作为发版门禁」的强模式；且仓库无 Linux CI 消费该退出码——「CI 门禁用」的 docstring 宣称（:23）无 CI 事实支撑。
调用链: t_env_preflight 登记 _ENV_MISSING → require_tools_or_skip → skip() → main() 汇总
触发条件: 在缺外部工具/ Rust 工具链的机器跑 run_tests.py
实际影响: 覆盖率随环境静默收缩而全绿——极端环境（无 cargo 无 OCR 无 LO）下 Rust/OCR/导出面零验证仍 exit 0；若未来以此作发版门禁而无 SKIP 清单人工复核，会放行未验证轮次
复现步骤: PATH 去掉 cargo 与 tesseract 后 `python3 scripts/run_tests.py; echo $?`（预期 0 + 多项 SKIP）
当前测试: 无（run_tests 自身无元门禁）
缺失测试: `--strict` 模式（SKIP>0 → exit 2）或 CI 侧断言 skipped==0
建议方案: 增加 --strict / 发布口径分离（冒烟 vs 发布）；CI 落地时用 strict 模式
证据:
```python
def skip(name, reason):
    print(f"  ○ {name} —— SKIP: {reason}")
    _skipped.append(name)          # ← 不入 _failed
…
return 0 if not _failed else 1     # ← SKIP 不影响退出码
```
审计置信度: CONFIRMED
状态分类: 当前仍存在（设计自洽但门禁语义弱，属 documented behavior）

---

ID: SCR-002
严重程度: P2
模块/文件路径/代码行号/函数或脚本: scripts/run_tests.py:100-112 `write_env_blocked`
问题描述: 测试入口**追加写入被 git 跟踪的文件** baseline/ENV_BLOCKED.md（含运行时间戳）——环境缺件的机器每跑一轮就把工作树弄脏一份非确定内容（该文件现存 2026-10-28 历史条目为实证）。
触发条件: _ENV_MISSING 非空时任意一轮 run_tests
实际影响: 脏工作树/意外把运行日志带进 commit；时间戳内容不可复现
建议方案: 改写到 gitignored 位置（如 baseline/env-blocked.local.md 或 runs/），ENV_BLOCKED.md 保留首建说明
证据:
```python
p = REPO / "baseline" / "ENV_BLOCKED.md"
stamp = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
…
with open(p, "a", encoding="utf-8") as f:
    f.write("\n".join(lines))
```
审计置信度: CONFIRMED（git ls-files 确认 tracked；tail 见历史条目）
状态分类: 当前仍存在

---

ID: SCR-003
严重程度: P2
模块/文件路径/代码行号/函数或脚本: scripts/run_tests.py 全文（覆盖面）；对照 .github/workflows/windows-check.yml:89-95
问题描述: 本地总入口不覆盖 web-ui（typecheck/build/bench 零入口）、mang-app（T19 的 bidcore --workspace 不含独立 workspace 的 mang-app——windows-check.yml 注释自证）、sdk/tests；scripts 下 6 个 Rust 工程同样不在。README「每轮测试 59 项」口径易被读成全仓门禁。
实际影响: web-ui/mang-app/sdk 回归只靠人手/他侧 workflow（mang-app 仅有 Windows CI 的 cargo check 无 test）——本仓测试叙事与实际门禁面不匹配
建议方案: run_tests 增 T20（web-ui tsc/build）、T21（mang-app cargo test）或至少在 docstring/README 显式声明不覆盖清单
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: SCR-004
严重程度: P2
模块/文件路径/代码行号/函数或脚本: scripts/run_tests.py:536-640（t_v1/t_v2/t_v3）
问题描述: T7/T8/T9 的被测物是 v1_*.py / v2_impact.py / v3_integration.py 的 **Python 语义复刻**（SEMUL）——设计语义的仿真验证，非生产 Rust 码。v2_impact.py:306 自注「bidcore/bid-domain Rust 同语义交叉验证待 cargo 环境」；verification-plan 以 SEMUL ≠ ENGINE_RUN 标注。生产面仅 T18/T19 承载。
实际影响: 「每轮测试全绿」对 V1-b/V1-c/V2/V3 语义只是**仿真口径全绿**——引擎/域实现若与仿真语义漂移，run_tests 不红（漂移面靠 T18 内容锚、bidcore cargo test、e2e_m1 部分兜底）
建议方案: 文档/输出行显式标注 SEMUL 维度（如「T8 V2（SEMUL）」）；逐步把 V2/V3 断言迁 bidcore cargo 测试
审计置信度: CONFIRMED
状态分类: 当前仍存在（自注在案，属已知边界）

---

ID: SCR-005
严重程度: P3
模块/文件路径/代码行号/函数或脚本: scripts/run_tests.py:417-422 `_grade_dims`
问题描述: 忽略 grade.py 子进程退出码，直接读 report.json——grade.py 崩溃/未写 report 时 FileNotFoundError **未捕获**，run_tests 整体 traceback 中断（无汇总、无清理）。
触发条件: grade.py 异常或 runs/<sc>/ 目录异常
实际影响: 一处判分器故障放大为整轮中断，失去"N/M/K"汇总
建议方案: try/except 包裹 → check(…, False, traceback)
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: SCR-006
严重程度: P3
模块/文件路径/代码行号/函数或脚本: scripts/run_tests.py:457-461（T10c）、725-736（t_b5_gt_eval）
问题描述: ① T10c 断言 `returncode==0 and "0 failed" in stdout`——若 m1 单测被整体删除（0 个测试）该检查**空洞通过**（无测试数下限）；② T18 无 cargo SKIP 守卫：cargo 缺件时 b5_engine_gt_eval.py 内 find_cargo 抛 FileNotFoundError → 脚本非零退出 → T18a 记 **FAIL**（而非 SKIP）——与 T19/issue #9「环境缺失≠业务失败」口径自相矛盾。
实际影响: ①测试数回归不可见；②缺 cargo 环境得到假业务失败
建议方案: T10c 加 `n_ok >= 5` 下限；t_b5_gt_eval 入口加 `if not env_ok(["cargo"]): skip(...); return`
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: SCR-007
严重程度: P3
模块/文件路径/代码行号/函数或脚本: scripts/run_tests.py:170, 247-270；scripts/fixture_gen.py:49；scripts/synth_tender_gen.py:46
问题描述: 机器本地绝对路径依赖群：`/usr/share/fonts/truetype/chinese/SarasaMonoSC-Regular.ttf`（T0d 预检 + 两个生成器硬编码）、`/usr/bin|/bin/tesseract`、`~/.tessdata`、`~/.cargo/bin`、固定名 `/tmp/_b2_ocr.png`、`/tmp/_b2_ocr_input.png`（写后不清理、多用户碰撞）。Debian 强假定，Windows 不可用。
实际影响: 换环境（非 Debian/Nix/容器异构路径）即红或跳；固定 /tmp 名在共享机器有互踩面
建议方案: 字体路径走 env 可覆盖（FONT_PATH）；/tmp 用 tempfile；tesseract 走 shutil.which
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: SCR-008
严重程度: P3
模块/文件路径/代码行号/函数或脚本: scripts/run_tests.py:465-471（t_m1 提前 return）、265-270（/tmp 残留）
问题描述: 失败后清理行为不统一：T6/T7/T13 用 TemporaryDirectory 自动清；runs/ 故意保留（gitignored，by design）；但 t_m1 中任一场景运行失败即 `return`（T12/T13 不执行，runs/ 留半成品）；_b2_ocr_roundtrip 的 /tmp 两文件任何路径下都不删。
实际影响: 低（研究仓）；半成品 runs/ 会误导下一轮读旧工件（T12 只读 TC-01，不受累，但人工排查困惑）
建议方案: finally 统一登记残留清单
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: SCR-009
严重程度: P2
模块/文件路径/代码行号/函数或脚本: scripts/patch_docs_r10.py:6（BASE=…）、61/74/91（io.open(…,"w")）
问题描述: 一次性文档批量改写脚本滞留 scripts/ 根：BASE 硬编码 `/home/z/my-project/download/bid-agent-research`——**与当前仓路径 /home/z/my-project/ai-bid-research 不符**（历史机器路径）。若在存有旧副本的机器上误运行，assert 锚点命中后会改写**旧树**的 verification-plan/test-plan/README；在当前仓运行则 FileNotFoundError。属"改文件的脚本"留仓风险。
调用链: 人工执行（无任何调用方；run_tests 不触它）
触发条件: 误运行
实际影响: 改错文档树/直接异常；仓库卫生问题（一次性迁移脚本应归档删除）
复现步骤: 静态分析即可（本审计未运行）
建议方案: 移入 docs/history/ 或删除（其内容已落盘于 2026-10-26 提交）
证据:
```python
BASE = "/home/z/my-project/download/bid-agent-research"   # ← 历史绝对路径，非本仓
…
io.open(p, "w", encoding="utf-8").write(s)                 # ← 写 verification-plan/test-plan/README
```
审计置信度: CONFIRMED
状态分类: 当前仍存在（滞留一次性脚本）

---

ID: SCR-010
严重程度: P3
模块/文件路径/代码行号/函数或脚本: scripts/fixture_gen.py:450-455、scripts/synth_tender_gen.py:434-437（main 默认 out）
问题描述: 两个生成器无 `--out` 时默认**原地重生成仓内 fixtures**（out=FIXTURES / DEFAULT_OUT）。确定性下字节不变（有 T6 保障），但 spec 已漂移而夹具未重生成时，一次手滑运行会静默换件（manifest 同步重写，T3 自洽，仅 git diff 可见）。
实际影响: 低（确定性 + git 兜底）；操作脚枪
建议方案: 默认要求显式 --out 或加确认；或在 README 标注「原地模式仅 Lead 使用」
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: SCR-011
严重程度: P3
模块/文件路径/代码行号/函数或脚本: scripts/b5_engine_gt_eval.py:60-80 `extract_reqs`
问题描述: T18 在**生产源码树**写临时 example（bidcore/crates/bid-engine/examples/gt_eval_dump.rs，cargo run 后 finally unlink）。并发两份 run_tests/T18 会互相覆盖/删除对方文件；崩溃兜底虽有 finally，但 SIGKILL 场景留残留文件会被 cargo 当真 example 编译。
实际影响: 低（单人本地惯例）；并发/中断脚枪
建议方案: 改用 `cargo script` 形态或独立 examples 目录 + 生成文件名带 pid；或把 dump 例程固化为正式 example（去掉运行时写码）
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: SCR-012
严重程度: P3
模块/文件路径/代码行号/函数或脚本: scripts/syntax-check.sh:24-28、17-20
问题描述: ① find 只扫 `bidcore/crates`——不覆盖 m1/src、scripts/{baseline_harness,bid_health_check,engine_probe,title_impact}/src、web-ui；注释自称「检查全部 tests/ 下的 .rs」与实际不符；② rustfmt 缺件 `exit 0`（SKIP=绿，与 SCR-001 同款弱语义，但此处连登记输出都无持久化）。
实际影响: 语法门覆盖面虚标；缺 rustfmt 机器静默全绿
建议方案: find 范围补齐四组 Rust 工程 + m1；SKIP 时打印持久化登记
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: SCR-013
严重程度: P3（正面确认 + 保留项）
模块/文件路径/代码行号/函数或脚本: scripts/synth_tender_spec.py:525-539（v0.2 补登注记）、scripts/synth_tender_gen.py:210-229（design_note）
问题描述: synth-tender 参考答案的 v0.2 修订（commit d2c8198）在首轮评测（v0.1，2026-10-30）**之后**发生——3 条补登条目的 engine_expect 按当日引擎实测登记。防循环论证纪律要求「跑后不许回改」，此处通过显式诚实标注（「非独立跑前推演……不冒充预判」）+ 冻结预演总账不含补登条目（self_check 有账平校验）来兼容。
实际影响: 可接受（理想分类 ideal_category 仍为人工核对、非引擎输出回放；回填面只 3 条且标注在案）；登记为方法论保留项
建议方案: 无需动作；后续补登继续沿用该标注纪律
审计置信度: CONFIRMED
状态分类: 当前仍存在（已如实标注）

---

ID: SCR-014
严重程度: P3
模块/文件路径/代码行号/函数或脚本: scripts/ooxml_util.py:1-17（模块头）、40-54（freeze_zip）
问题描述: 引擎语义仿真层（SEMUL）为 capability-map S11/L1 的 B4 删除对象，但当前被 9 个脚本消费（fixture_gen/synth_tender_gen/v1_*/v3_integration）——freeze_zip 是全部夹具确定性的根基，Locator 仿真承载 T7b/T9 断言。删除该层需先迁移这两面，否则 T6/T7/T9 全塌。
实际影响: 无即时风险（fnv64 有向量断言防漂移；freeze_zip 输入自产文件无安全面）；处置次序约束登记
建议方案: B4 执行时先迁 freeze_zip（如独立 freeze.py）再把 Locator 断言迁引擎侧
审计置信度: CONFIRMED
状态分类: 当前仍存在（处置依赖登记）

---

ID: SCR-015
严重程度: P3
模块/文件路径/代码行号/函数或脚本: scripts/run_tests.py:408-414（_m1_run）、452-458（T10 cargo build/test）
问题描述: 对 m1 二进制与 cargo 的 subprocess 调用**无 timeout**（对比 T18 timeout=400、T19 timeout=900）。m1 的 OCR 链无页数门（M1-012）——一个超大扫描件挂起会让 run_tests 永久停在该处（配合 SCR-005 无异常兜底）。
触发条件: m1 场景挂死/超大输入
实际影响: 本地整轮测试无限挂起
建议方案: subprocess.run(…, timeout=600) 并把 TimeoutExpired 记 FAIL
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

## 7. scripts 基础设施审计总结

| 维度 | 结论 |
|---|---|
| run_tests.py 判定语义 | SKIP 计通过（exit 0），issue #9 显式设计、留痕可审计，但无 strict 模式且无 CI 消费——门禁强度弱（SCR-001, P1） |
| 覆盖面 | 夹具/判分器/一致性/m1/真实链静态断言齐；web-ui、mang-app、sdk、scripts 内 Rust 工程不在（SCR-003）；V1/V2/V3 为 SEMUL 仿真口径（SCR-004）；生产面仅 T18/T19 |
| 副作用 | runs/ 受控；**ENV_BLOCKED.md（tracked）被追加写**（SCR-002）；/tmp 固定名残留（SCR-007/008）；T18 写生产源码树临时 example（SCR-011） |
| 夹具 GT 独立性 | corpus-A：spec-first authored 单一真源，独立成立；synth-tender：人工核对答案 + 3 条跑后回填已如实标注（SCR-013）；判分器与生产抽取不同源 |
| ooxml_util | SEMUL 仿真层，B4 删除对象但 load-bearing（9 消费方），fnv64 向量断言防漂移（SCR-014） |
| patch_docs_r10 | 一次性改文档脚本滞留 + 历史绝对路径，删除候选（SCR-009） |
| syntax-check | 覆盖面虚标 + SKIP=绿（SCR-012） |
| 是否调生产实现 | T18/T19 真调 bidcore；T10~T13 调 m1（遗留原型）；其余为 Python 仿真/静态断言；无 "bin real_eval" 形态入口 |
