# R5 独立复审报告 —— commit 0e50961（R3 剩余项修复轮）+ R4 自查轮的独立性补偿

- 审计轮次：R5（继 R1 夹具规格 / R2 代码 / R3 安全 / R4 主代理自查之后的第五轮）
- 审计日期：2026-10-26
- 审计对象：commit `0e50961`（S3-1 zip64 守恒 + S3-3 xlsx 双轴门 + S3-4 严格引用判据 + P2-4 stderr drain + fp_sampling + 计数回填），基线 `f59b3f1`，报告轮 `e5a86fd`（HEAD）
- **审计性质与独立性声明**：本轮为**独立子代理审计**——审计人与 0e50961 修复的作者（主代理）相分离；R4 的攻击样本已丢失（`/home/z/my-project/scripts/audit_r4` 不存在），本轮全部攻击样本为**独立重新构造**（zip64/EOCD 字节布局按 PKWARE APPNOTE 自行拼装，未参考仓库测试辅助函数与 R4 记录），probe crate 置于仓库外 `/home/z/my-project/scripts/audit_r5`（path 依赖 bid-parse 调 pub API 真跑，仓库零污染——git status 全程干净）。审计立场：**挑战 R4 结论**，非背书。
- 红证纪律替代方案（依任务约束）：对"测试杀伤力"的判定 = 静态断言方向审查（应 REJECT 样本是否真会 REJECT / 失败路径是否可达 / 是否被上游 crate 先拒导致测不到本仓逻辑 / 边界常量 > 与 >=）+ probe crate 独立攻击面实测交叉证明；未修改仓库业务代码、未 git commit、未 push。

---

## 0. 结论速览

| 级别 | 计数 | 摘要 |
|---|---|---|
| S1 严重 | **0** | — |
| S2 应修 | **0** | — |
| S3 登记建议 | **1 项新增（含 3 个变体）+ 3 项维持 R4 登记** | **新增**：EOCD 守恒门存在三类 R4 未测出的绕过形态——尾部垃圾 ≥65,536B 使 EOCD 移出 guard 65,557B 扫描窗（门整体跳过，R3 原始 zip32 同名条目攻击借 64KB 追加复活，V13a/V13b 一字节差分实证）；影子 EOCD（guard 取最后签名、crate 取最后**有效** EOCD 并回退，两者选择策略分歧）；zip32 计数字段分歧（guard 读 EOCD 总盘字段 +10、crate 读本盘字段 +8）。三者端态均与 A6 同族（物理同名条目被接受、单消费者视图内部一致），威胁评估维持低，但 **R4 "7 类攻击变体下无绕过"的结论被证伪**，threat-model "吸收面封死/已封" 措辞需修正，登记面需扩充 |
| P 轻微 | **5** | ① xlsx 单元格轴（200 万）无仓库负向测试（仅行轴有；本审计 X3 探针实证单元格轴真实生效并给出样本设计）② 压缩比门整数除法截断：实效拒点=商 ≥101 而非文档的"比值>100"（100.999 可过门，差 <1 个比值单位）③ run_limited 杀组假设边界：setsid 脱组孙进程逃过 kill(-pgid)，返回仍有界（实测 1.52s）但 detached 读线程存活至孙进程退出——当前三个外部命令不可能从文件内容派生 setsid 进程，生产不可达，注释假设需注记 ④ R4/任务口径笔误：P2-4 的 run_limited 位于 bid-parse/guard.rs 而非 bid-tools/runner.rs（后者是 R3-S2-4 的 per-file catch）⑤ worklog 缺 Task 19 条目（R3 修复轮记录），R3 报告引用"worklog Task 19"在文件中不存在 |

**核心结论（独立复核意见）**：0e50961 的五项修复**行为属实**——zip64 声明数篡改负向、xlsx 双轴门（含边界 > 语义与"拒绝不留部分结果"）、严格引用判据 18/18 与页偏差 {0:17,-1:1}、stderr drain 不中杀不阻塞、fp_sampling 确定性双跑零 diff，全部经我方独立样本/复跑证实；cargo 53/53、run_tests 46/46、xref 0 错误计数复核一致。但 R4 的总判断"zip64 守恒门在 7 类攻击变体下无绕过"**不成立**：R4 的变体清单恰好未覆盖守恒门的两个结构弱点（EOCD 发现策略与字段选择），本审计用 3 类新变体（尾部垃圾/影子 EOCD/字段分歧）实测绕过。因端态威胁与 A6 同族（低危、单消费者一致），不升 S2，但登记与措辞必须修正。负向测试杀伤力经逐项静态审查**无一假绿**（详见 §4）。

---

## 1. 方法

1. **独立 probe crate** `/home/z/my-project/scripts/audit_r5`（Cargo.toml path 依赖
   `bidcore/crates/bid-parse`），23 个探针全部调 pub API 真跑：
   `open_guarded` / `read_entry_limited` / `run_limited` / `xlsx::parse` /
   常量（LIMIT_* 系列）。样本 19 个（zip/zip64 15 + xlsx 3 + 子进程 5 场景），
   字节级构造（PKWARE APPNOTE 布局：EOCD 22B、CDFH 46B+nlen、zip64 EOCD 56B、
   locator 20B 字段偏移独立推导），与 R4 样本零重合。
2. **红证替代**：静态审查 34 项测试（guard 21 + xlsx 2 + four_contracts 9 +
   corpus_probe 抽查）断言方向与失败路径；probe 实测交叉证明（如 V6b/V13a/V13b
   差分对照证明 V6 的绕过由尾部垃圾单独引起）。
3. **全量复跑**：`cargo test --workspace`（53/53）+ `run_tests.py`（46/46）+
   `xref_check.py`（0 错误/8 存量告警）+ `fp_sampling.py` 双跑（stdout/JSON/与仓内
   提交件三方零 diff）+ `real_eval_score.py` 复跑（与提交版 score-report.json
   字节一致）。
4. **环境契约**：首轮全量跑 e2e_m1 **真实 FAIL**（GT-07 双锚点 2→1、B2 OCR 要素
   缺失）——根因为本审计沙箱缺 `~/.tessdata/chi_sim.traineddata`（环境契约件，
   scripts/fixtures-requirements.txt 已钉）。按契约补齐后 53/53。此 FAIL 本身是
   正面证据：OCR 面测试在环境缺件时**响亮变红而非静默过绿**（与 worklog Task 16
   "T4c-OCR 门禁 FAIL 不静默跳过——设计如此"一致），假绿猎杀的先决条件成立。

---

## 2. 样本清单与实测（23 探针，PASS=19 / BYPASS=4 / INFO=4 / FAIL=0）

### A. zip/zip64 守恒门（S3-1 声明面）——16 探针

| 探针 | 构造（全部独立字节级） | 预测 | 实测 | 裁决 |
|---|---|---|---|---|
| V1 基线 | 合法双条目 docx | ACCEPT len=2 | ACCEPT len=2 | PASS |
| V2 zip64 声明数篡改 | CD 同名条目 + zip64 记录 count=2 | REJECT 不守恒 | REJECT: 声明 2 ≠ 实际 1 | **PASS（S3-1 修复行为属实）** |
| V3 A6 形态复现 | 同上但 zip64 count=1（declared==absorbed） | ACCEPT | ACCEPT len=1 | INFO（A6 登记面维持） |
| V4 越界 locator | locator 偏移 0xFFFF_FFFF_0000 | REJECT | REJECT: crate "Invalid EOCD64 Locator CD offset" | PASS（fail-closed） |
| V5 前置垃圾 A7 复现 | 256B 'J' 前缀 + 合法 zip64 | ACCEPT | ACCEPT len=1 | INFO（A7 登记面维持） |
| **V6 尾部垃圾【新】** | zip32 同名条目归档 + 65,600B 尾部垃圾（>22+65535 窗） | （对抗预测）ACCEPT | **ACCEPT len=1** | **BYPASS** |
| V6b 差分对照 | 与 V6 同一归档、无垃圾 | REJECT | REJECT: 声明 2 ≠ 实际 1 | PASS（绕过由垃圾单独引起） |
| **V7 影子 EOCD【新】** | 真实 EOCD（声明2+物理同名对）后接 comment_len=60000（越 EOF）伪 EOCD（声明 1） | （对抗预测）ACCEPT | **ACCEPT len=1** | **BYPASS** |
| **V8 字段分歧【新】** | zip32 EOCD 本盘=2/总盘=1 + 物理同名对 | （对抗预测）ACCEPT | **ACCEPT len=1** | **BYPASS** |
| V8b 偏严方向对照 | 本盘=1/总盘=2 | REJECT | REJECT: 声明 2 ≠ 实际 1 | PASS（fail-closed 方向一致） |
| V9 天文计数 | zip64 count=0xFFFF_FFFF_FFFF | REJECT | REJECT: crate "inconsistent number of files" | PASS（A5 独立复证） |
| V10a 压缩比=100 边界 | Stored 1000B + CD 声明篡改 100,000 | ACCEPT | ACCEPT len=1 | PASS（> 非 >=） |
| V10b 压缩比=101 | Stored 1000B + 声明 101,000 | REJECT | REJECT: 压缩比异常 101000/1000 | PASS |
| V10c 截断边界 | 声明 100,999（真实比 100.999） | ACCEPT | ACCEPT len=1 | INFO（整数除法：实效拒点=商≥101，见 P-2） |
| V13a 窗口内边界 | 同名对 + 垃圾恰 65,535B | REJECT | REJECT: 声明 2 ≠ 实际 1 | PASS（EOCD 恰在窗口首） |
| **V13b 一字节差【新】** | 同名对 + 垃圾恰 65,536B | （对抗预测）ACCEPT | **ACCEPT len=1** | **BYPASS（阈值一字节证明）** |
| V11 声明撒谎截停 | Stored 4000B、CD 声明改 100、读上限 1000 | REJECT ActualReadOver | REJECT: 已读 1001 > 上限 1000 | PASS（真防线回归） |

**根因（源码级）**：
- **V6/V13b**：guard 的 `eocd_declared_entry_count` 只回扫 `len-22-65_535` 起的尾部
  （guard.rs:285），EOCD 在窗外 → `Ok(None)` → 守恒门整体跳过；而 zip 4.6.1 的
  `MagicFinder<Backwards>` 以 bounds=(0, file_len) **全文件**回扫（spec.rs:612），
  且接受"EOCD 之后有垃圾"（relaxed comment 检查，注释明言兼容 Python zipfile）。
  尾部垃圾 ≥65,536B 即出窗——**R3 原始 zip32 同名条目攻击（S3-1 前身）借 64KB 追加复活**。
- **V7**：crate 的 EOCD 候选循环在 parse 失败（comment 越 EOF）时 `continue` 回退更早
  EOCD（spec.rs:625-660 + get_metadata 的 end_exclusive 回退循环 read.rs:638-656）；
  guard 取**最后一个签名**且不验证 comment 一致性——影子 EOCD 让 guard 读假计数。
- **V8**：zip32 路径 crate 用 EOCD **本盘**字段（+8..10，read.rs:509
  `number_of_files_on_this_disk`）；guard 读**总盘**字段（+10..12，guard.rs:301）。
  字段分歧时两者比的不是同一个声明值。
- 三者端态均为"物理同名条目 + 声明==吸收后计数"被接受：与 A6 同族。危害评估沿用
  R3 S3-1 原判：zip 4.6.1 按声明计数读 CD、IndexMap 吸收后 by_index/by_name 同一
  视图，单消费者无"查A读B"；calamine 同依赖同版本。**该门的意义是对未来 crate 语义
  漂移的纵深防御——绕过不产生新的当期攻击面，但使纵深防御的覆盖远小于其文档自述。**

### B. xlsx 读后双轴门（S3-3 声明面）——3 探针

| 探针 | 构造 | 实测 | 裁决 |
|---|---|---|---|
| X1 恰 200,000 行（=限值） | 独立最小合法 xlsx（inlineStr 单表） | ACCEPT blocks=200000，summary"200000 有效行" | PASS（`>` 非 `>=`） |
| X2 200,001 行 | LIMIT+1 | REJECT: 有效行 200001 超上限（拒绝不留部分结果） | PASS |
| X3 单元格轴独立负向 | 100,001 行 × 21 列 = 2,100,021 非空单元格（行 100,001 < 20 万不先触发行门） | REJECT: 非空单元格 2000019 超上限 2000000 | PASS（单元格轴真实生效；仓库无此轴负向测试→P-1） |

### C. 子进程治理 run_limited（P2-4 / T-04）——6 场景

| 探针 | 场景 | 实测 | 裁决 |
|---|---|---|---|
| S1 stdout 溢出 + stderr 洪泛并发 | `yes|head -c 20M &` + `dd 512KB >&2`，上限 64KB | **OutputOverflow，20ms 返回**（不等超时） | PASS（溢出分类不被 drain 破坏） |
| S2 挂死超时杀组 | `sleep 300`，1s 超时 | Timeout，1.004s；pgrep -x sleep 无残留 | PASS |
| S3 孙进程持管道（R3-S2-1 回归） | `sleep 30 & echo done`，1s 超时 | **Timeout，1.520s**（非 30s 晚归 Success） | PASS（限时收流生效） |
| S4 stderr 洪泛 + 非零退出 | `dd 256KB >&2; exit 3` | NonZeroExit code=3，stderr_head 恰 160 字符，20ms | PASS（drain 不中杀 + head 保留） |
| S5 stderr 洪泛 + 正常载荷 | `dd 256KB >&2 && echo done` | Success stdout=done，20ms | PASS（P2-4 声明面核心） |
| S6 脱组孙进程（setsid） | `python3 -c 'os.setsid(); time.sleep(12)' & echo hi` | **Timeout 1.520s 有界返回；孙进程 pid 实测逃过杀组存活** | INFO（P-3 登记面） |

---

## 3. 0e50961 五项声明逐项独立裁决

| # | 声明 | 独立复核证据 | 裁决 |
|---|---|---|---|
| 1 | S3-1 zip64 守恒门（eocd_declared_entry_count + open_guarded） | V2 独立样本 REJECT；V4/V9 crate 合围复证；V3/V5 A6/A7 形态复现维持登记。**但 V6/V7/V8/V13b 三类新绕过**（见 §2A） | **行为属实，覆盖面声明过强——R4 结论部分推翻** |
| 2 | S3-3 xlsx 双轴门（20 万行/200 万格、拒不留部分、`>` 边界） | X1（恰 200,000 放行）/X2（200,001 拒）/X3（单元格轴独立拒，仓库未测的轴我方补证） | **属实**（单元格轴负向测试缺口 → P-1） |
| 3 | S3-4 严格引用判据（18/18 严格口径 + {0:17,-1:1}） | real_eval_score.py 复跑：召回 18（75.0%）/类别命中 12（50.0%）/**引用支持 18/18（100.0%，锚点页±1）**/页偏差 {'0':17,'-1':1}；score-report.json 与仓内提交版**字节一致**；窗口切片 `pages_n2[anchor-2:anchor+1]` 语义复核正确（±1 含跨页拆行容忍） | **属实** |
| 4 | P2-4 stderr drain（溢出优先、超时杀组） | S1（20ms 溢出分类）/S4（drain 不中杀+head=160ch）/S5（Success）/S2/S3（杀组+限时收流）。注意位置：run_limited 在 **bid-parse/guard.rs**（bid-tools/runner.rs 是 S2-4 per-file catch——任务描述口径笔误，记 P-4） | **属实**（setsid 边界注记 → P-3） |
| 5 | fp_sampling 确定性（seed=20261025） | 双跑 stdout/JSON 零 diff，且与仓内提交版 fp-sampling.json 零 diff；749 条分类（43.0%/31.0%/15.0%/8.3%/2.8%）与 REPORT §6 一致 | **属实** |
| 6 | 计数核实 | `cargo test --workspace -- --list` = **53**；实跑 **53/53**（14 domain + 9 four_contracts + 1 e2e + 1 selftest + 1 kb_probe + 26 bid-parse + 1 corpus_probe）；threat-model T-03 12 项（guard zip 面 12）/T-04 9 项（guard 子进程面 9）/xlsx 2 项清点一致；REPORT D1=6 漏项逐条在位（GT-R05/S03/S04/S05/T01/T02）、D3=02 号配对诚实排除；run_tests 46/46；xref 0 错误；elapsed_secs=20.602 | **全部属实** |

---

## 4. 假绿猎杀记录（负向测试杀伤力静态审查 + 实测交叉）

审查方法：每项记录"测什么面 / 攻击样本 / 若实现被故意破坏该测试如何失败（红证推演） /
是否被上游 crate 先拒导致测不到本仓逻辑"。**34 项无一假绿。**

### 4.1 guard.rs 21 项（zip 面 12 + 子进程面 9）

| 测试 | 面/样本 | 杀伤模式（破坏推演） | 上游先拒? |
|---|---|---|---|
| zip_bomb_compress_ratio | 40MB 全零 → 比≈1000:1 | 删门→守恒门过（1==1）→open Ok→unwrap_err 红 | 否 |
| entry_count_flood | 10,001 条 | 删门→prescan 其余过→EOCD 声明==实际→open Ok→红 | 否 |
| path_traversal_and_bad_names | 五形态（../、绝对、控制符、盘符、深 70） | 删名门→crate 开归档不查名→open Ok→红 | 否（crate 无名策略） |
| duplicate_entries_rejected | CD 同名 patch（declared 2≠吸收 1） | 删守恒门→prescan 视野只有吸收后 1 条→open Ok→红 | 否 |
| zip64_duplicate_entries | 同上 + zip64 链 count=2 | 还原"0xFFFF 即跳过"旧逻辑→门跳→open Ok→红（**0e50961 回归门，V2 独立复证**） | 否 |
| legit_zip64_passes | 合法 zip64（1==1） | 链误读/过拟合→Err→unwrap 红（防"修门即全拒"） | — |
| nested_archive | attach/instructions.zip | 删载具门→open Ok→红 | 否 |
| input_size_gate_rejects_before_read | 4MB 不可压缩 > 2MB 限 | 删门→zip 本体小→open Ok→红；门在读前（metadata） | 否 |
| lying_declaration_stopped | CD 声明 patch 1KB（真 24MB）+ 读限 1KB | 先断言 prescan 放行（撒谎面在内）→删实际读取计量→返回 24MB→unwrap_err 红（**V11 独立复证**） | 否 |
| actual_read_limit_enforced | 诚实 2MB + 读限 1MB | 删计量→红；再读 4MB 限验证 take 不破坏正常路径 | 否 |
| normal_zip_passes / sha256_streaming | 正向对照 | 任何门误拒/哈希口径漂移→红 | — |
| subprocess_timeout_kills | sleep 30 / 1.2s | 删 kill 或 process_group(0)→wait() 阻塞→elapsed 断言红 | 否 |
| subprocess_output_overflow | yes 100M / 64KB | 删溢出标志→写端阻塞拖到 60s Timeout→matches!(OutputOverflow) 红 | 否 |
| nonzero / success / spawn / busy | exit 3 / printf / 不存在路径 / 占满 4 席 | 分类逻辑破坏或并发门删除→各自断言红；busy 释放后恢复（防永久占用假绿） | 否 |
| subprocess_stderr_flood_drained | dd 256KB && echo done | 还原旧 take 行为→dd 收 SIGPIPE(141)→&& 短路→NonZeroExit(141)→panic 红（R4 红证记录 + 我方 S4/S5 实测机制吻合：drain 后 20ms Success） | 否 |
| tmpguard_recovers_on_all_paths | Drop 回收（含带文件目录） | Drop 空转→exists 断言红 | 否 |
| timeout_never_lies_as_success | 三分类 into_result 全 Err | 任何映射为 Ok→is_err 红 | 否 |

### 4.2 xlsx.rs 2 项

- `xlsx_row_flood_rejected_by_post_gate`：200,001 行**真 xlsx**（字节构造+calamine 真解析，非 mock）；删门→Ok(blocks=200001)→unwrap_err 红；我的 X2 独立复证，且确认门在迭代内触发（200,001 行处）而非物化前。
- `xlsx_normal_file_passes_post_gate`：百行正向对照，防"修门即全拒"；断言 blocks.len()==100 + summary 含"100 有效行"——门常量或计数逻辑破坏即红。
- **缺口（P-1）**：单元格轴（LIMIT_XLSX_TOTAL_CELLS=2,000,000）无仓库负向测试——本审计 X3 证明该轴真实生效（100,001×21 → "非空单元格 2000019 超上限"），建议随批补测试（样本设计可直接采用 X3：行数 < 20 万而格数 > 200 万，确保触发的是格轴）。

### 4.3 bid-domain/tests/four_contracts.rs 9 项

| 测试 | 面 | 杀伤模式 |
|---|---|---|
| c1_addendum_cascade_dirty_exact | 三变更联合级联 | DIRTY 恰三章节（多则污染少则漏网两向断言）；content_version 恰 +1；**model_v2 参数断言在位（R3-P2-2 修复验证）** |
| c1_cascade_precise_flips | 级联精确性 | GT-02 恰翻转、GT-13 维持 EXPIRED+细项 0 分、GT-01/05/06 零误翻、recheck_log 恰两轮、NOT DIRTY 恰三章——过度重检任一维度即红 |
| c1_adjudication_gate_and_idempotency | 终裁门禁+幂等 | 脏阻断→redraft→未裁决冲突仍阻断→终裁 465+双 GT 口径切换→放行；同 sha 重导 idempotent_noop+version 不动——门禁或幂等破坏任一断言红 |
| c1_delete_add_rules_synthetic | 删除/新增规则（诚实标注 SYNTHETIC） | 删除→绑定/矩阵/REQ 三处齐消+恰一章节；新增→new_row+零误伤 |
| c2_rewrite_degrades_exactly_one | 只降不升 | refresh 真比对指纹（linkage.rs:60-77 实逻辑非 setter 回环）→恰一链降级；undo 后 re.is_empty + Stale 断言（不回升）；reverify 唯一回升通道 |
| c2_external_change_no_silent_adopt | 外部修改不静默 adopt | **走 refresh 检测路径（R3-M-2 修复）**：指纹变化→degraded==[GT-01]+audit BodyRefresh 有痕；reverify 新 locator 恢复 |
| c3_five_round_rewrite_anchors | 五轮重写锚点 | 每轮恰一降级+重验恢复；**零悬空断言真化（R3-M-1 修复）**：索引在界内+指纹与当前段一致（非恒真） |
| c4_export_interrupt_never_fakes | 导出中断 | render 后停 EXPORTING+无 sha+consistent；confirm→EXPORTED+sha；同产物幂等同 sha——任何"未确认即 EXPORTED"即红 |
| c4_exported_state_alignment | 状态机一致性 | Exported 终态（next_allowed 空）+ReadyToExport 唯一入边+三非法边断言 |

另：corpus_probe.rs（1 项）为夹具全链冒烟（B2 必走 OCR ≥4 块、前附表-10、OCR 否决词、B1 中文日期、断行还原）——真实夹具硬口径，e2e_m1（1 项）在缺 chi_sim 环境**响亮失败**（本轮实测），selftest_gate/kb_probe 各 1 项实跑通过。

---

## 5. 对 R4 三项 S3 登记的独立裁决表

| R4 登记 | R4 论点 | R5 独立裁决 | 依据 |
|---|---|---|---|
| **A6**（declared==absorbed 一致性畸形被接受） | 单消费者内部一致、低威胁、门 green 属语义正确 | **维持 S3 登记**（V3 独立复现）。但 R4 将其框定为孤立"形态"低估了族谱——本审计 V6/V7/V8 显示同一端态可经**结构化绕过**（非畸形巧合）达成，登记措辞应从"一致性畸形归档"扩为"守恒门可达声明==吸收端态的多路径族"（见新增 S3） | V3/V6/V7/V8 实测 + zip 4.6.1 read.rs:691-695（按 number_of_files 读 CD、IndexMap 吸收） |
| **A7**（前置垃圾 zip64 守恒门跳过，未实现 archive-offset 校正） | crate 自身 zip64 计数一致性校验合围、无实际绕过路径 | **维持 S3 登记**（V5 独立复现；根因源码确认：guard 用绝对偏移 guard.rs:318、crate 用 found-declared 差值校正 spec.rs:754-762）。但"合围无绕过"论证不完备：crate 的 zip64 校验只覆盖 zip64 记录自身，**zip32 路径无对等校验**（V8 实证字段分歧即穿） | V5/V8 实测 + spec.rs 源码 |
| **_anchor_hit 相邻块拼接理论假命中窗口**（run_tests T12） | 有界：仅相邻对+归一化逐字、概率极低 | **维持 S3 登记**。T12 实跑 27/27（含 GT-07×2 扫描件 OCR 跨行拼接——回退通道有真实必要性）；假命中需 GT 锚归一串恰好跨两个语义无关相邻块的接缝逐字重组，corpus-A 确定性夹具下为理论窗口。附注：回退对**所有**锚点生效（非仅 OCR 锚），文本层锚点也可走拼接通道——风险仍受"逐字+相邻"双约束封顶，维持登记即可 | run_tests T12 实跑 + run_tests.py:302-312 源码 |

**对 R4 总判断"修复属实且负向测试有真实杀伤力"的独立复核意见**：**半维持**。
- "修复属实"——**维持**：五项声明逐项独立复证全部属实（§3）。
- "负向测试有真实杀伤力"——**维持**：34 项静态审查无一假绿，负向样本均为真构造真断言，失败路径可达且不被上游先拒。
- **但 R4 的"zip64 守恒门 7 类变体无绕过"——推翻**：7 类变体恰好全部落在"门可被触达"的输入域内；R4 未构造"EOCD 不可被 guard 发现/发现不同源"类输入。R5 用 3 类新变体（V6/V7/V8，其中 V6 一字节差分 V13a/V13b 钉死阈值）实测绕过。该发现的严重度维持 S3（端态与 A6 同族、当期无可利用面），但它是 R4 独立性弱化（自查 + 样本丢失）造成的**实际漏检**——正是本轮补偿审计存在价值的直接证明。

---

## 6. S 计数汇总与处置建议

- **S1=0 / S2=0**。
- **S3=1 项新增（三变体合并登记）**：守恒门绕过族（尾部垃圾出窗 / 影子 EOCD / zip32 字段分歧）。
  登记建议：threat-model T-03 残余面第 ② 条后追加三种形态；同时把"吸收面封死"（guard.rs:262、
  threat-model:76）与"zip64 重复条目绕过已封"（threat-model T-03 行、REPORT:29）措辞降格为
  "常规面已封，exotica 族登记在案（R5 扩充）"。
  修复建议（低成本三选一或组合，不阻塞）：
  1. guard 的 EOCD 回扫窗口扩为全文件（文件已被输入字节门限 ≤52/302MB，扫描成本可忽略）——封 V6/V13b；
  2. EOCD 候选校验 comment_len 一致性并模拟 crate 的回退循环——封 V7；
  3. zip32 双字段（+8..10 与 +10..12）不一致即拒 + 比对取本盘字段——封 V8。
- **S3=3 项维持 R4 登记**（A6/A7/_anchor_hit，措辞按 §5 修正）。
- **P=5**：① xlsx 格轴负向测试缺口（X3 已给样本设计）② 压缩比门整数除法截断（建议改
  `declared > LIMIT * compressed_eff` 乘法判据，精确兑现">100"文档语义）③ run_limited
  杀组假设注记（setsid 脱组——注释补"组外写者存活期 detached 线程存留"说明）④ 审计口径
  位置笔误（run_limited 在 guard.rs 非 runner.rs）⑤ worklog 缺 Task 19（R3 修复轮）条目，
  R3 报告引用悬空。
- **环境契约提示**：全新环境跑全量门禁前须按 scripts/fixtures-requirements.txt 补
  `~/.tessdata/chi_sim.traineddata`（tessdata_fast）；缺失时 e2e_m1/run_tests T4c 响亮
  红（设计如此），不应误判为代码回归。

## 7. 仓库零污染声明

- `git status` 全程干净（`## main...origin/main`，无改动未提交）；fp_sampling/real_eval_score
  复跑产物与仓内提交版字节一致（diff exit=0），未产生任何工件漂移。
- probe crate、样本、本报告全部位于 `/home/z/my-project/scripts/audit_r5/`；chi_sim 安装于
  `~/.tessdata`（环境契约件，非仓库内容）。
- 未 git commit、未 push。

*审计人：R5 独立复审子代理（Task 19-a）。探针输出全文留存 run1_output.txt；样本 19 个留存 samples/。*

---

## 8. 主代理处置决议（Task 19，2026-10-26）

R5 六项建议全部采纳，处置如下：

1. **S3 新增登记（守恒门绕过族）**：threat-model T-03 残余面已扩充登记（V6 尾部垃圾出窗 / V7 影子 EOCD / V8 zip32 字段分歧，A6 同族端态）；"吸收面封死""绕过已封"措辞降格为"常规面已封，exotica 族登记在案（R5 扩充）"——随本批文档提交（threat-model.md §2.1 表行 + §4.1 残余面）。
2. **修复排期（M1.6 安全加固轮，下轮代码批）**：采纳 R5 方案 1+3 组合——EOCD 回扫窗口全文件化（文件已被输入字节门限 ≤52/302MB，扫描成本可忽略）+ zip32 双字段（+8..10 与 +10..12）不一致即拒；方案 2（EOCD 候选 comment 校验+回退模拟）保留为修复后回归对比项评估。
3. **P-1 xlsx 格轴负向测试**：下轮随 M1.6 入仓，样本设计直采 X3（100,001 行 × 21 列，行不触发行门、格超 200 万）。
4. **P-2 压缩比门乘法判据**：M1.6 改 `declared > LIMIT * compressed_eff`，精确兑现">100"文档语义。
5. **P-3 setsid 注记**：随 M1.6 在 guard.rs run_limited 注释补"组外写者存活期 detached 线程存留"说明。
6. **P-4 口径笔误更正**：run_limited 位于 **bid-parse/guard.rs**（bid-tools/runner.rs 是 R3-S2-4 的 per-file catch）——本报告 §3 第 4 行已如实记录，后续文档以本口径为准。
7. **P-5 worklog Task 19**：主代理本轮补录，R3 报告引用不再悬空。

**R5 在审计序列中的位置与纪律固化**：R1 夹具规格 → R2 代码 → R3 安全 → R4 主代理自查（独立性弱化 + 样本丢失）→ **R5 独立子代理复审（本轮）**。R5 对 R4"7 类变体无绕过"的推翻实证了审计独立性纪律的必要性：**自查 + 样本丢失 = 实际漏检**（V6/V7/V8 全部落在 R4 变体清单恰好未覆盖的输入域内）。后续审计轮优先恢复子代理审计；假绿猎杀方法（本报告 §4 模板）已随真实场景库 SC-09 固化为周期纪律（每 R 轮抽 5 项负向测试做红证推演记录）。

## 9. 处置闭环（M1.6 执行批，2026-10-26 同日）

上节六项处置全部闭环，超出原方案的部分如实记录：

1. **S3 守恒门绕过族——✅ 已封堵（超出原"方案 1+3"采纳范围：1+2+3 全做）**：`eocd_declared_entry_count` 重写为 **zip crate 4.6.1 EOCD 选择语义的全文件镜像**——①回扫窗全文件化（封 V6/V13b；三档差分负向：65,535/65,536/65,600 全拒）；②comment 适应 + 候选回退循环镜像（封 V7；crate parse 失败回退更早候选的语义逐步对齐，含盘号自洽/空归档早退/CD 偏移前置/CDFH 窗内存在性/顺序读最小空间必要条件）；③zip32 双字段分歧即拒（新增 `EocdCountFieldsDiverge`，封 V8/V8b 双向）。**A7 同步升级**：zip64 记录 [声明偏移, locator) 窗内正向扫 → 前置垃圾 zip64 从「门跳过」升「门真跑」（fail-closed 放行）。**终端语义升级**：候选全部不可验而归档已开 → `EocdUnverifiable` fail-closed 拒绝（旧 Ok(None) 门跳过即三类绕过公共根因，负向在位）。
2. **P-1 格轴负向——✅ 入仓**：`xlsx_cell_flood_rejected_by_post_gate_cell_axis`（X3 直采优化形：95,239 行×21 列恰 2,000,019 格末行触发；行门静默断言防误触轴面混淆）。
3. **P-2 乘法判据——✅ 入仓**：`declared > LIMIT.saturating_mul(compressed_eff)`；边界对负向（100,999 拒 / 100,000 放行——旧除法判据下 100.999 过门，破坏推演注记在测）。
4. **P-3 setsid 注记——✅ 已补**（threat-model §4.2 与 guard.rs 注释）。
5. **P-4/P-5 口径与 worklog——✅ 已正**（本报告口径维持；worklog Task 19/20 双条目在案）。
6. **镜像严格方向残留（新增登记，均 fail-closed）**：crate Detect 猜测语义窗口微差 / zip64 内层多候选回退未逐步复刻 / prefix-junk 下 CDFH 空间检查用相对偏移近似——三者只产生过度拒绝（恶意构造件面），无 fail-open 路径；threat-model §4.1 已登记。A6 同族端态维持登记（结构化路径已封，孤立畸形巧合路径固有）。
7. **验证**：M1.6 批 cargo workspace **66/66**（53 基线 + 13 新测试——含 SC-01 查询面 c4b 随同批落地）、run_tests **51/51**（46 基线 + T0×5；T7d 升五断言）、xref 0 错误——全量本会话亲跑。

**R5 → M1.6 的审计-修复闭环证明**：R5 报告 §6 的"低成本三选一"实际全部实施且超出（镜像化而非补丁化），三条建议的破坏推演注记全部随测试入仓——审计产出物到防线落地的完整链路本批走通。
