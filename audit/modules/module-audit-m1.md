# 模块审计 E1-a：m1（独立 Rust 研究线，16 个 tracked 文件）

- 审计代理：4-e1（ai-bid-research 全仓审计 V2，Task ID: 4-e1）
- 基线：main@6105beb（分支 audit/v2-full-coverage，HEAD=1e0bc9d 仅为 audit 文件叠加，生产树与 6105beb 一致）
- 纪律：只读审计；未修改任何生产/测试/配置/数据；未运行 cargo build/test（唯一"运行"是只读哈希计算，用于验证 M1-001）。
- 审计方式：m1/ 全部 16 个 tracked 文件逐行全读（src 11 文件 1795 行 + Cargo.toml + Cargo.lock 包名扫描 + README + .gitignore + tessdata 二进制 sha 校验）；调查链读取 run_tests.py（T10~T13 消费面）、bidcore/crates/bid-parse/src/lib.rs、bid-tools/src/runner.rs、bid-grade/tests/e2e_m1.rs、docs/capability-map.md、research/risk-register.md。

---

## 0. 覆盖声明

| # | 文件 | 行数 | 覆盖 |
|---|---|---|---|
| 1 | m1/src/run.rs | 319 | 全读 |
| 2 | m1/src/extract.rs | 304 | 全读 |
| 3 | m1/src/docx.rs | 210 | 全读 |
| 4 | m1/src/import.rs | 203 | 全读 |
| 5 | m1/src/search.rs | 164 | 全读 |
| 6 | m1/src/xlsx.rs | 152 | 全读 |
| 7 | m1/src/state.rs | 144 | 全读 |
| 8 | m1/src/contract.rs | 93 | 全读 |
| 9 | m1/src/canon.rs | 89 | 全读 |
| 10 | m1/src/main.rs | 62 | 全读 |
| 11 | m1/src/chunk.rs | 55 | 全读 |
| 12 | m1/Cargo.toml | 17 | 全读 |
| 13 | m1/Cargo.lock | — | 包名/zip 版本扫描（zip 2.6.1，满足 ^2.2） |
| 14 | m1/README.md | 75 | 全读 |
| 15 | m1/.gitignore | 3 | 全读 |
| 16 | m1/tessdata/chi_sim.traineddata | 二进制 | sha256 只读校验（=README 钉版值 a5fcb6f0…） |

注：任务描述提到"15 文件/含 m1/tests/"——实际 `git ls-files m1/` 无 tests/ 目录；测试以 `#[cfg(test)]` 内联于 state.rs（2 项）与 canon.rs（3 项），合计 5 项，与 README「5 项单测」口径一致。m1/target 不存在（本克隆未构建）。

---

## 1. m1 定位裁决（核心问题）

**结论：m1 = 遗留研究线（被演进版取代的"并行会话首轮实现"），非失效入口但已无生产价值；唯一活跃调用方是本地测试入口 scripts/run_tests.py T10~T13；属 capability-map.md 已登记的 B4 处置对象（重复实现）。**

### 1.1 证据链（逐条）

1. **自我定位声明**：m1/README.md:3「研究验证阶段的**被测系统**」；run_tests.py:19「T10~T13 被测系统 = m1/ 单 crate 首轮实现，并行会话产出；bidcore/ 为演进版，回归在 cargo 侧」；research/risk-register.md:82「m1/ 为并行会话首轮实现……本矩阵 TESTED 状态以 bidcore 证据为准，m1 证据作并行佐证随仓保留」。
2. **git 历史**：m1 全部内容只有一个内容提交 ecd7d97（2026-10-24，"M1 最小流水线（Rust）落地 + V1-a 首轮验证"），经 3ac962e 合并；此后 497 个 commit 中**零改动**（`git log ecd7d97..HEAD -- m1/` 为空）。同期 bidcore 从 ~51 测试演进到 175 测试并完成 B4 收敛。
3. **无 CI 门禁**：`.github/workflows/windows-check.yml` 只跑 `bidcore` 的 cargo check/test（m1 是 Linux 工具链链路，从未进任何 workflow）。audit/baseline.md:67 已登记「m1 与 scripts 下 6 个 Rust 工程无 CI 门禁」。
4. **e2e_m1 与 m1 无关（名字遗留）**：`bidcore/crates/bid-grade/tests/e2e_m1.rs:4-5,34-37`——`use bid_tools::runner::{run_scenario, write_run}` + `grade::grade_run(&out.db, &fx)`，跑的是 **bidcore 生产管线**（bid-domain 状态机 + bid-parse/bid-engine 解析 + bid-kb 检索），不 import 不调用 m1 crate。其名字来自"M1 里程碑"。
5. **bidcore 侧零依赖**：`rg '\bm1\b' bidcore/` 命中全部是局部变量名（`let m1 = …`）或 `db.milestone == "M1"` 里程碑字符串，无任何路径/依赖引用 m1/。
6. **重复实现清单**（m1 ↔ 生产演进版逐面）：

| m1 文件 | 生产对应 | 证据 |
|---|---|---|
| docx.rs / xlsx.rs（zip+quick-xml 自研，**无守卫**） | bid-engine→mdc-office-io（OOXML 单一真源）；bid-parse 自研解析已按 B4 删除 | bid-parse/src/lib.rs:3-6「B4 收敛：docx/xlsx 自研解析已删——引擎是 OOXML 读面的单一真源」；docs/capability-map.md L6「m1 第二套 docx/xlsx 解析……**无守卫**——直接 zip 读取，无 S1-S10 全部门；重复实现（B4 处置对象）」 |
| import.rs（扩展名分派 + sha256 + OCR） | bid-parse::parse_file + engines/（poppler/tesseract/仲裁器）+ guard.rs（529 行守卫） | m1 import.rs:33-59 无字节门/无 prescan/无进程治理；bid-parse guard.rs 有 S1-S10 全套 |
| state.rs（INIT→…状态机 + 幂等键） | bid-domain::DomainStore + bid-tools store_io.rs（**单 db.json tmp+rename 原子提交**） | README.md:47 V3-1 批次记录 |
| extract.rs（规则①~⑨，304 行） | bid-parse/src/extract.rs（1305 行）+ clause/numbering/nesting/veto_lexicon/veto_rule | m1 规则绑定 corpus-A 结构（见 M1-003） |
| search.rs（exact 通道） | bid-kb::Kb | — |
| run.rs（TC-01/02/07/11 编排） | bid-tools::runner::run_scenario（362 行，含 service 化导入） | runner.rs 头注释同款四场景 |

7. **删除候选证据链**（若裁决废弃）：删 `m1/`（16 文件，含 86MB 级 tessdata 二进制需核对体积）需同步：
   - scripts/run_tests.py：删 T10~T13（`M1_DIR/M1_BIN/M1_SCOPE/M1_PENDING/SCENARIOS` 常量 + `t_m1()` + `_m1_run/_grade_dims/_anchor_hit` + docstring T10~T13 行）；
   - README.md「每轮 59 项」口径与 m1 并行线描述、research/verification-plan.md:95-102 V1-a 口径注记、research/risk-register.md:80-82、docs/architecture-decision.md:43（v3 回填引用 m1/src/state.rs 的 TC-11 证据——需改为指向 bid-domain 证据）、docs/capability-map.md L6/§105、research/real-scenario-library.md:114；
   - 历史结果（锚点 27/27、四场景判分全绿、双跑字节一致）已抄录进上述文档，无"必须保活代码才能保住证据"的依赖。
8. **保留现状的正当性**（反向）：run_tests T10~T13 仍是 m1 的回归门禁（防其腐烂），且其双跑字节一致测试是仓库内唯一"OCR 通道工件级确定性"证据；但这只证明"旧原型还能跑"，不产生生产价值。

### 1.2 与 bidcore 的关系定性

原型/前身关系（同源两支）：m1 是 2026-10-24 并行会话对 M1 里程碑的最小实现，bidcore workspace 是同一起点的演进版（B4 删除自研 OOXML 解析改调引擎、安全面补齐 guard、服务化导入、175 测试）。两者**并行存在**而非调用关系；m1 未被反哺进 bidcore（bidcore 无 m1 依赖）。

---

## 2. 正面记录（m1 设计纪律中值得保留的部分）

- **诚实契约**：contract.rs/README 的「M2/M3/M4 范围字段一律不出现——判分器按缺字段 FAIL（不造假绿）」；REQ `gt_ids` 恒空（反标记回声纪律，召回只认判分器对齐器）。
- **确定性纪律**：全 ID 内容寻址（`REQ-<sha256[..10]>`、chunk_id、F-/P- 前缀）；输出零时间戳零随机；OCR 钉死 OMP_THREAD_LIMIT=1 + LC_ALL + 固定 psm 6 + 仓内 tessdata 内容寻址；`Cargo.lock` 提交。T13 双跑字节一致即由此支撑。
- **canon.rs**：OCR 规整为确定性回正（全角标点、CJK 伪空格），3 项单测锁行为；「两侧均汉字才把 '.' 回正为间隔号」的保守分支正确。
- **state.rs 越级防护**：E_STATE 零副作用（`reject()` 不改状态不入幂等表），有单测。
- **import.rs safe_name**：不可信内容截断 40 字符进错误信息（注入防御意识）。

---

## 3. 问题记录

---

ID: M1-001
严重程度: P2
模块/文件路径/代码行号/函数或脚本: m1/src/import.rs:20-26 `sha256_file`
问题描述: 名为 sha256 的函数不是 sha256——对**二进制文件**先做 `String::from_utf8_lossy` 有损解码再哈希（且追加 0x1f 分隔符）。README/夹具 manifest 里的真 sha256（对原始字节）与 m1 内部"sha256"**永不相等**；lossy 解码非单射，不同字节串可得到同一 lossy 字符串。
调用链: run.rs `do_import` → `import_file` → `sha256_file` → `Machine::import_file(sha,…)` 幂等键 / `F-<hash>` file_id / db.json `m1_notes.toolchain.chi_sim_sha256`（run.rs:115-121 同款算法）
触发条件: 任何 m1 run（每次导入）；跨系统对账或构造 lossy 碰撞文件时显性化
实际影响: ① db.json 的 sha256/chi_sim_sha256 字段语义失真（实测：chi_sim 真 sha256=a5fcb6f0…，m1 算法=6e5a40e5…，README 钉版值只与真 sha256 匹配）；② TC-07 幂等键理论碰撞（两个不同二进制文件 lossy 同串 → 误判 no-op、复用 file_id、丢块）；③ 与 fixture manifest 哈希永远对不上，无法互证。
复现步骤:（已做，只读）`python3 -c` 对 m1/tessdata/chi_sim.traineddata 分别算真 sha256 与 m1 算法（lossy 解码+0x1f），两者不同；真值=README 声明值。
当前测试: T13 双跑一致可过（算法确定性没问题）；无任何测试断言 m1 哈希 == manifest 哈希
缺失测试: 幂等键碰撞负向测试；m1 sha 与 manifest sha 一致性断言
建议方案: 改为流式真 sha256（对齐 bid-parse guard::sha256_streaming）；若保留 lossy 语义须改名（`content_key`）并在 README 撤回 sha256 表述
证据:
```rust
pub fn sha256_file(p: &Path) -> String {
    let mut f = std::fs::File::open(p).expect("open file");
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).expect("read file");
    crate::contract::content_hash(&[&String::from_utf8_lossy(&buf)])   // ← 有损解码
        .chars().take(64).collect::<String>()
}
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: M1-002
严重程度: P2
模块/文件路径/代码行号/函数或脚本: m1/src/docx.rs:15-22、xlsx.rs:31-39、import.rs:84-129（整体）
问题描述: m1 持有仓库内第二套 OOXML 解析（zip+quick-xml），**零安全守卫**：无输入字节门、无 zip prescan（条目数/单声明/总声明）、无 EOCD 守恒/截断容器检出、无条目名路径逃逸检查、无压缩比门、无 XML 深度门、`read_to_end` 全量内存、pdftotext/tesseract 输出无上限（对比 bid-parse guard.rs S1-S10 全套 + run_limited 子进程治理 + OCR 页数门 200）。docs/capability-map.md L6 已将其登记为「重复实现（B4 处置对象）」。
调用链: run.rs do_import → import_file → docx::parse / xlsx::parse / parse_pdf（→ ocr_pdf）
触发条件: 对恶意/超大 docx/xlsx/pdf（zip 炸弹、billion-laughs、超多页扫描件）运行 m1
实际影响: 研究线当前只跑可信夹具（corpus-A），无真实攻击面；若有人把 m1 当工具对不可信输入使用 → OOM/挂死/解压炸弹。重复实现本身造成维护面翻倍（见 §1.1 表）。
复现步骤: 构造 01-zip-bomb-declared-10kb-actual-1gb.docx（baseline/security 已有现成样本）喂给 `m1 run --corpus` 指向目录——docx.rs 会直接 read_to_end
当前测试: 无（m1 无任何 hostile 样本测试；baseline/security 样本只被 bidcore 消费）
缺失测试: hostile 输入负向测试（若保留 m1 则必须补，若废弃则随删）
建议方案: 按 capability-map B4 裁决执行：废弃 m1（推荐，证据链见 §1.1-7）；或改调 bid-engine 门面复刻 bid-parse B4 后形态
证据:
```rust
// docx.rs:15-22 —— 无任何守卫直读
let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
let mut zf = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
let mut xml = Vec::new();
zf.by_name("word/document.xml")…read_to_end(&mut xml)…
// import.rs:150-199 ocr_pdf：pdfimages 全部输出页循环 tesseract，无页数门/超时
```
审计置信度: CONFIRMED
状态分类: 当前仍存在（已在 docs/capability-map.md L6 登记为处置对象，但处置未执行）

---

ID: M1-003
严重程度: P3（定位佐证）
模块/文件路径/代码行号/函数或脚本: m1/src/extract.rs:193-199（规则①~⑥）、run.rs:125-129
问题描述: 抽取规则表硬绑定 corpus-A 单一语料结构：条款前缀 3.2.x/3.3.x/3.7.2/4./5./6. + locator 中文串（"资格要求"/"第四章"/"第五章"/"第六章"）+ 文件名 A-招标文件.docx/B2-投标函格式页-扫描件.pdf 作为函数参数。对 synth-tender 或任何真实招标文件，产出近零 REQ。
调用链: run.rs:195 `extract(&st.blocks, a, b2)`（文件名字面量来自 run.rs:125-127）
触发条件: 换语料（--corpus 指向其他目录）
实际影响: m1 不可复用于 corpus-A 之外——支持「研究验证夹具专属 harness」定性；非 bug（README 已声明研究定位），登记为定位证据。
复现步骤: `m1 run --scenario TC-01 --corpus fixtures/synth-tender/招标文件` → requirements≈0、step expects fail
当前测试: 仅 corpus-A 判分（T11）
缺失测试: 不适用（研究边界）
建议方案: 随 §1.1 裁决处置；文档显式声明"corpus-A 专属"
证据:
```rust
let category: Option<&str> =
    if clause.starts_with("3.2.") && locator.contains("资格要求") { Some("资格条件") }
    else if clause.starts_with("3.3.") && text.contains("不得") { Some("格式要求") }
    else if clause.starts_with("3.7.2") { Some("否决项") }
    else if clause.starts_with("4.") && locator.contains("第四章") { Some("评分项") }
    …
```
审计置信度: CONFIRMED
状态分类: 当前仍存在（设计使然）

---

ID: M1-004
严重程度: P3
模块/文件路径/代码行号/函数或脚本: m1/src/extract.rs:79-89 `find_iso_date`
问题描述: 字节索引切片 `text[i..i+10]`——当模式命中但 `i+10` 落在多字节字符中间时 Rust 直接 panic（str 切片非字符边界）。前置条件只保证 b[i]/b[i+1]/b[i+4]/b[i+7] 是 ASCII，不保证 i+10 是边界。形如 "2026-11-2中"（日期样前缀 + 第 10 字节起 CJK）即触发。
调用链: extract.rs mk_params → find_iso_date（每条 Para 文本都过）
触发条件: 语料中出现 `20??-??-X` 后紧跟汉字且恰好第 10 字节起多字节字符的文本
实际影响: 整场 run panic 中断（非数据损坏）。corpus-A 的日期均为完整 10 字符 ISO 串，未触发。
复现步骤: 单测 `find_iso_date("截至2026-11-2中文描述")` → panic（预期）
当前测试: 无
缺失测试: find_iso_date 边界单测（含非边界 i+10 样本）
建议方案: `text.get(i..i+10)` 返回 Option 安全切片；或先判 `is_char_boundary(i+10)`
证据:
```rust
for i in 0..=b.len() - 10 {
    if b[i] == b'2' && b[i + 1] == b'0' && b[i + 4] == b'-' && b[i + 7] == b'-'
        && text[i..i + 10].chars().all(|c| c.is_ascii_digit() || c == '-') {   // ← 切片先于谓词
        return Some(text[i..i + 10].to_string());
```
审计置信度: SUSPECTED（panic 路径代码推理成立；语料未触发）
状态分类: 尚无法复现（需构造样本）

---

ID: M1-005
严重程度: P3
模块/文件路径/代码行号/函数或脚本: m1/src/xlsx.rs:71-74、135
问题描述: 只读 `xl/worksheets/sheet1.xml`——多工作表静默丢弃（无错误、无登记、locator 恒 "sheet1 行N"）。对比 bid-parse 的部件对账账本纪律（present>0 而 extracted==0 必须 note 登记）。
调用链: import_file("xlsx") → xlsx::parse
触发条件: xlsx 含 >1 个 sheet（真实图纸目录/报价表常见多 sheet）
实际影响: 数据完整性静默丢失；研究线夹具 B3 单 sheet 未触发
复现步骤: 双 sheet xlsx 导入 → 只有 sheet1 行块
当前测试: 无
缺失测试: 多 sheet 负向/登记测试
建议方案: 读 workbook.xml 列全部 sheet 或至少在 ParseReport.failures 登记 "sheets>1 only sheet1 read"
证据:
```rust
zf.by_name("xl/worksheets/sheet1.xml")
    .map_err(|e| format!("no sheet1: {}", e))?…
locator: format!("sheet1 行{}", row_idx),
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: M1-006
严重程度: P3
模块/文件路径/代码行号/函数或脚本: m1/src/docx.rs:91-95（Start 分支）vs 115-119（Empty 分支）
问题描述: 分页计数分支不一致：Start 形式 `<w:br w:type="page"/>`（带闭合）在 Cell 状态内也 `page += 1`；Empty 形式（自闭合）才有 `state != S::Cell` 守卫。同类页 break 依书写形态产生不同页码。另 84-89 行 `pStyle` 在 Cell 状态读到值后丢弃（死代码）。
触发条件: 表格单元格内显式分页符（罕见）
实际影响: 页码本为估算口径（README 声明 docx 无固定分页），影响极小；一致性问题登记备查
建议方案: 两分支统一 Cell 守卫；删死代码
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: M1-007
严重程度: P3
模块/文件路径/代码行号/函数或脚本: m1/src/state.rs（全文）/ run.rs:262-306
问题描述: state.rs 权威状态机**无持久化**（纯内存，问题清单中"state.rs 持久化原子性"的答案：不存在该面）；run.rs 产工件用 5 次裸 `std::fs::write`（db/trace/blocks/chunks/run-notes），非 tmp+rename 原子提交。对比 bid-tools/src/store_io.rs「单 db.json tmp+rename 原子提交（borrow mdcore SidecarStore）」。
触发条件: 写盘途中崩溃/磁盘满
实际影响: 留半截工件；下次判分读到旧/坏 JSON。研究线低影响（重跑即恢复）
建议方案: 若保留 m1：对齐 tmp+rename；若废弃：随删
审计置信度: CONFIRMED
状态分类: 当前仍存在（与生产纪律不一致，属原型差距）

---

ID: M1-008
严重程度: P3
模块/文件路径/代码行号/函数或脚本: m1/src/import.rs:152-153、181-183、201
问题描述: OCR 临时目录 `/tmp/m1-ocr-<pid>` 名字可预测；`remove_dir_all` 仅在成功路径执行——tesseract 任一页失败即 `return Err`，已抽出的 PNG 全部泄漏；多用户机器上可预测路径存在 symlink 抢注面。对比 bid-parse TmpGuard「全路径回收」纪律。
触发条件: OCR 中途失败（坏页/tesseract 崩）
实际影响: /tmp 垃圾堆积；理论 symlink 攻击（研究环境信任，降 P3）
建议方案: `tempfile` crate 匿名目录 + RAII 清理；错误路径也回收
证据:
```rust
let tmp = std::env::temp_dir().join(format!("m1-ocr-{}", std::process::id()));
…
if !out.status.success() {
    return Err(std::io::Error::new(…));      // ← 提前返回，跳过 remove_dir_all
…
let _ = std::fs::remove_dir_all(&tmp);        // 仅成功路径
```
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: M1-009
严重程度: P3
模块/文件路径/代码行号/函数或脚本: m1/src/extract.rs:51-77 `parse_money` 与 m1/src/search.rs:32-58 `money_values`
问题描述: 两份逐行相同的金额解析实现（含注释同文）分居两文件——单侧修改即静默漂移（抽取 params 与检索归一口径分叉）。
实际影响: 低（冻结研究线）；维护性问题
建议方案: 抽公共 fn；或随 m1 处置
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: M1-010
严重程度: P3
模块/文件路径/代码行号/函数或脚本: m1/src/import.rs:21-23
问题描述: `sha256_file` 用 `.expect("open file")/.expect("read file")` panic——IO 错误（权限/竞态删除）直接整场崩，不进 per-file failures 登记面（对比 parse 失败走 failures 的纪律）。
触发条件: do_import 的 `path.exists()` 预检与打开之间的竞态、权限错误
实际影响: 单文件 IO 问题放大为整场景 FATAL
建议方案: 返回 Result 并入 failures
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: M1-011
严重程度: P3
模块/文件路径/代码行号/函数或脚本: m1/src/{extract,search,chunk,docx,xlsx,import,run}.rs（测试面）
问题描述: 重逻辑零单测：extract 规则/参数抽取（304 行）、search 排序与数值归一化（164 行）、docx/xlsx 状态机、chunk 边界均无 `#[test]`；仅 state(2)+canon(3)=5 项。覆盖全靠 T11~T13 端到端判分——规则级回归无定位能力（判分红后需人肉定位到规则）。
当前测试: T10c 只断言 `"0 failed"`（零测试也能过该断言）；n_ok 计数仅展示
缺失测试: clause_of/parse_money/money_values/find_* 纯函数单测（都是无依赖纯函数，成本极低）
建议方案: 若保留：补纯函数单测 + T10c 断言 ≥5；若废弃：随删
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

ID: M1-012
严重程度: P2（条件性）
模块/文件路径/代码行号/函数或脚本: m1/src/import.rs:150-203 `ocr_pdf`（无页数门/无超时/无输出上限）
问题描述: pdfimages 输出多少页就循环多少次 tesseract，无页数上限、无超时、无 stdout 体积上限。对比 bid-parse「OCR 页数门 200 + 超时杀进程组 + 溢出即时杀」。
触发条件: 超大扫描件/图片型 PDF（真实招标扫描件几百页常见）
实际影响: m1 run 无界耗时（小时级）——run_tests T10~T13 无超时会挂起整轮本地测试（subprocess 无 timeout 参数，见 SCR-015）
建议方案: 页数门 + subprocess timeout；或随 m1 处置
审计置信度: CONFIRMED
状态分类: 当前仍存在

---

## 4. m1 审计总结

| 维度 | 结论 |
|---|---|
| 定位 | 遗留研究线（并行首轮实现），被 bidcore 全面取代；非失效入口（T10~T13 仍真跑），但零生产调用方、零 CI、零演进（2026-10-24 后无改动） |
| e2e_m1 归属 | bid-grade/tests/e2e_m1.rs 调 **bidcore**（bid_tools::runner），与 m1 无关（名字遗留） |
| 与生产重复 | docx/xlsx/import/state/extract/search/run 七面全部有演进版对应（§1.1 表）；capability-map L6 已登记 B4 处置 |
| 内部质量 | 诚实契约与确定性纪律优良（正面记录 §2）；工程缺口：假 sha256（M1-001）、零守卫（M1-002）、corpus-A 专属规则（M1-003）、若干 P3 |
| 处置建议 | 推荐**废弃删除**（证据链 §1.1-7 完整）；若保留须：真 sha256 化 + hostile 负向测试 + OCR 页数门/超时 + 纯函数单测，并在 README 撤回"sha256/生产链路"相关表述 |
