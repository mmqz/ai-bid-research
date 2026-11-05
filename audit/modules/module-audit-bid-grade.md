# 模块审计 A2-3：bid-grade（corpus-A 判分器 + 自测门禁）

- 审计代理：4-a2（ai-bid-research 全仓审计 V2，Task ID: 4-a2）
- 基线：main@6105beb（分支 audit/v2-full-coverage）
- 纪律：只读审计；本文件与 inventory-a2.csv 为本轮 A2 新增文件之一；未修改任何生产/测试/配置/数据，未运行 cargo test/build。
- 审计方式：声明范围 8 文件逐行全读（grade.rs 537 / selftest.rs 349 / align.rs 148 / fixtures.rs 118 / main.rs 74 / lib.rs 7 / tests e2e_m1.rs 53 / selftest_gate.rs 17）；调查链读取 bid-tools::schema（RunDb 类型面）与 fixtures/corpus-A/expected-transitions（断言语料形态）。

---

## 0. 覆盖声明

| # | 文件 | 行数 | 覆盖 |
|---|---|---|---|
| 1 | bid-grade/src/grade.rs | 537 | 全读 |
| 2 | bid-grade/src/selftest.rs | 349 | 全读 |
| 3 | bid-grade/src/align.rs | 148 | 全读 |
| 4 | bid-grade/src/fixtures.rs | 118 | 全读 |
| 5 | bid-grade/src/main.rs | 74 | 全读 |
| 6 | bid-grade/src/lib.rs | 7 | 全读 |
| 7 | bid-grade/tests/e2e_m1.rs | 53 | 全读 |
| 8 | bid-grade/tests/selftest_gate.rs | 17 | 全读 |

---

## 1. 架构与判分纪律（正面）

- **七判分项 + 里程碑范围分级**：G1 召回+精确率守门 / G2 精确匹配 / G3 矩阵 policy / G4 引用存在率 / G6 规则命中 / G7 幂等重放 / G5 断言解释器。M1 场景（TC-01/02/07/11）只判 G1/G2/G5/G7，G3/G4/G6=OUT_OF_SCOPE（防越级判红）；verdict 只由被判定维度决定（OUT_OF_SCOPE 显式留痕不计成败）。
- **未执行 ≠ 通过**：G7 幂等块缺失判 FAIL（非 None==None 假绿）；G2 exact 块缺失在 TC-02/full 判 FAIL；G5 核心断言路径不可解析即失败（「留空字段不能中性化断言」）——三条 fail-closed 纪律均有破坏注入测试验证。
- **反标记回声对齐器**（align.rs）：召回判定只认对齐器三通道——条款号双因子（clause_no 匹配 + 佐证因子）、锚点互含（N2 归一空间 + 双向长度约束防全文锚/碎片锚）、唯一参数值（跨 GT 共享的模糊值剔除）。自报 gt_id 无法对齐的入 unverified 不计召回。selftest 三类伪对齐（自报无锚点/全文锚点/垃圾 REQ 撞条款号）全部被 G1 捕获。
- **类型化 schema 运行期防线**：RunDb 反序列化失败=契约违反（main.rs grade_one）；自测含「布尔字段伪装」经 serde 类型拒绝的 Rust 原生防线。

---

## 2. 调查问题逐项

### 2.1 评分估算与真实评标的边界

- bid-grade 判的是**抽取/检索/矩阵/管线正确性**（对 GT 的召回、查询命中、policy 翻转、幂等），不判评标分数——与 bid-domain scoring 的 rule_heuristic/diagnostic_only 边界无重叠。判分器自身「不调模型、不做语义判断」（lib.rs 头），对齐是确定性规则。**边界清晰，无越权伪装。**
- 唯一的「评分」语义在 GT 侧 expected_policy（eligible/not_eligible）与 RULES_8 规则码——均为 fixture 冻结数据，判分器只比对不推演。

### 2.2 selftest 的 Oracle 是否独立（是否用生产同源算法造真值）

**分两面：**

1. **判分器对外（真运行）——独立成立**：
   - GT/queries/scenarios 三源全部来自 fixtures/（requirements.jsonl / exact-25.jsonl / TC-*.json），fixtures.rs 只读不改（「语言中立的冻结数据」——fixture_spec.py 单一真源）。
   - align.rs 对齐器与生产抽取代码（bid-parse extract）**不同源**：对齐用 clause_no/锚点/参数三通道比对冻结 GT，不复用生产抽取的任何判定——反标记回声机制（selftest 三类伪对齐全捕获）正是为此设计。
   - e2e_m1.rs 跑**真实生产管线**（run_scenario：真实解析→抽取→kb→状态机）再判分——被测物与判分器通过冻结夹具隔离。
2. **selftest 的完美库（perfect_db）——部分同源自洽（可接受但要登记）**：
   - perfect_db 的 requirements 直接从 fx.gts 克隆、matrix policy 从 gt.expected_policy 映射、exact per_query 直接抄 q["expect"]、**findings 用 grade::RULES_8 自产**（selftest.rs:91-94）。即「完美库」由 GT+判分器常量构造——判分器自测验证判分器（tautological 面：G6 对 perfect_db 必绿是构造保证）。
   - 这对「判分器能否捕获破坏」的自测目的是**正确方法论**（完美输入必须全绿+每处破坏必须被对应维度捕获，10 处破坏注入 + 2 处场景破坏 + 类型防线 + dedupe 合并运行全验证）；但**不构成生产规则引擎（bid-tools 规则 8 码）的验证**——那由 e2e_m1 的真实跑承载（runner 真产 findings，判分器比对）。GRADE-002 登记该边界。
   - **结论：selftest 不是用「生产同源算法」造真值**（造真值用的是 GT/夹具与 schema 类型，非生产算法）；同源风险仅在「RULES_8 常量与 bid-tools 规则码清单若单侧漂移」——判分器会 FAIL（fail-closed 方向正确），不会假绿。

### 2.3 错误路径可达性 / 断言语料形态

- expected-transitions 42 条断言的 kind 分布：set_cover/rules_hit/repeat/dirty/not_dirty/not_rerun/conflict/ocr/metric/state/error/policy/count/engine_gap——解释器逐 kind 分支（grade.rs:395-467）。`engine_gap` 显式 skipped 留痕；`metric` 无条件 pass(true)（GRADE-003）。
- resolve_path 20+ 路径形态全实现，未支持路径响亮 Err→断言 FAIL。

---

## 3. 问题记录

```
ID: GRADE-001
严重程度: P2
模块: bid-grade
文件路径: bidcore/crates/bid-grade/src/grade.rs
代码行号: 294-299（matrix[GT-XX].policy@open_bid= 路径解析）、330-334（fixture_open_bid_after 硬编码）
函数/结构体/接口: resolve_path / fixture_open_bid_after
问题描述: 补遗后开标日以硬编码常量 "2026-11-27" 内嵌于判分器代码（注释自认「从冲突登记数据读取口径；M2 前以已知口径为准」）——夹具数据口径被复制进判分逻辑：GT-02 的 policy_pre_addendum/policy_post_addendum 字段选择完全依赖该常量与断言路径 @open_bid= 值的字符串相等。夹具若调整开标日（或新增其他 @open_bid 场景），判分器静默把 post 当 pre 解析——pre/post 翻转断言（补遗 EXPIRED 翻转的核心验收）错位且无报错。
调用链: TC-03 断言 "matrix[GT-02].policy@open_bid=2026-11-27" → resolve_path → fixture_open_bid_after()="2026-11-27"（字符串相等）→ 取 policy_post_addendum
触发条件: 夹具开标日口径变更或新增不同 @open_bid 值的场景
实际影响: 判分器与夹具的隐性耦合——维护期静默错判（当前值与 fixtures/corpus-A/expected-transitions/TC-03.json:47 一致，今日无实害）
当前测试: selftest 的 GT-02 行手工注入 policy_pre/post 字段（selftest.rs:42-45）——不覆盖「夹具日期≠常量」形态
缺失测试: 常量与夹具口径一致性测试（或直接消灭常量）
建议方案: 从 fx（conflicts/manifest）读取补遗后开标日；过渡期加一致性断言（fixtures 日期变更即测试红）
证据: grade.rs:330-334
  fn fixture_open_bid_after(fx: &Fixtures) -> String {
      // 开标日（补遗后）——从冲突登记数据读取口径；M2 前以已知口径为准
      let _ = fx;
      "2026-11-27".to_string()
  }
审计置信度: CONFIRMED
状态分类: 当前仍存在（夹具口径复制进代码——文档与实现不一致的维护性变体）
```

```
ID: GRADE-002
严重程度: P3
模块: bid-grade
文件路径: bidcore/crates/bid-grade/src/selftest.rs
代码行号: 11-121（perfect_db 构造——reqs 克隆自 fx.gts、policy 映射自 expected_policy、per_query 抄 q["expect"]）、91-94（findings 自产自 grade::RULES_8）
函数/结构体/接口: perfect_db / selftest::run
问题描述: 判分器自测的「完美库」由 GT + 判分器自身常量构造：G6_rules 对 perfect_db 必绿是构造保证（findings 用 grade::RULES_8 生成）；G1 的 reqs 即 GT 克隆（对齐器三通道必然全中——自测无法暴露对齐器对「真实抽取形态」的假阴性）；exact per_query 直接抄 expect 文本（G2 的 contains 断言自洽）。自测验证的是判分器的破坏捕获力（其设计目的），不验证生产抽取/规则引擎——后者仅由 e2e_m1 真跑承载。
触发条件: 无需触发——方法论边界登记
实际影响: 低（e2e_m1 为真门禁；selftest 是判分器自校）。若 e2e_m1 因 ENV 缺件 SKIP（env_preflight 机制）而 selftest 通过，规则面（G6）当轮零验证——SKIP 横幅可见但 verdict 层不区分「自测绿+e2e SKIP」与「全绿」。
当前测试: selftest 本身
缺失测试: 不适用（边界登记）；可选：CI 层对「e2e_m1 SKIP 时禁止以 selftest 绿作为发布门禁」的显式口径
建议方案: 保持现状（自测目的明确）；在 selftest 输出加一行「完美库由 GT 构造——生产面验证见 e2e_m1」的显式声明
证据: selftest.rs:91-94
  findings: grade::RULES_8.iter().enumerate().map(|(i, c)| FindingInfo {
      rule_code: c.to_string(), kind: "DETERMINISTIC".into(),
      severity: if i < 3 { "BLOCKER" } else { "MAJOR" }.into(),
  }).collect(),
审计置信度: CONFIRMED
状态分类: 当前仍存在（可接受的方法论边界，非缺陷）
```

```
ID: GRADE-003
严重程度: P3
模块: bid-grade
文件路径: bidcore/crates/bid-grade/src/grade.rs
代码行号: 454（"metric" => pass(true)）
函数/结构体/接口: interpret_assert
问题描述: 断言解释器对 kind="metric" 无条件判过——TC-10 的两条 metric 断言（adapterA.ndcg@10 >= adapterB.ndcg@10 等）永远绿。夹具 note 自认「归档型用例：metric 断言不作发布门禁（test-plan §3）」——与实现口径一致，但任何未来新增的 metric 断言都会静默自动通过（假绿通道）。
触发条件: 未来往 expected-transitions 加 metric 断言并期望其生效
实际影响: 当前零（两条既有断言均为归档不门禁）；未来误用即假绿
当前测试: selftest perfect_db full 场景包含 TC-10 断言（未 excluded）——恒过
缺失测试: 不适用
建议方案: metric 分支改为「pass + skipped 留痕」（同 engine_gap 形态）或在解释器注释/输出显式标注「stub-恒过」
证据: grade.rs:454
  "metric" => pass(true), // A/B 指标比较由 TC-10 专项报告承载
审计置信度: CONFIRMED
状态分类: 文档与实现一致（口径债——test-plan §3 声明的既有决策）
```

```
ID: GRADE-004
严重程度: P3
模块: bid-grade
文件路径: bidcore/crates/bid-grade/src/grade.rs
代码行号: 15-18（RULES_8 常量）
函数/结构体/接口: RULES_8
问题描述: 八规则码清单硬编码于判分器（与 bid-tools 规则引擎的码表是两份拷贝）：若生产侧新增/改名规则码，判分器 G6「missed」即 FAIL（fail-closed 方向正确——不会假绿，但会阻塞）；两处清单漂移无编译期/测试期一致性检查。
触发条件: bid-tools 规则码表演进
实际影响: 维护耦合（失败方向安全）；当前两侧一致
当前测试: selftest G6 用 RULES_8 自产（同源——测不出漂移）；e2e_m1 真跑（runner 真产 findings）——生产侧码表与判分器码表在真跑中隐式对账（缺码即 G6 FAIL）
缺失测试: 判分器 RULES_8 与 bid-tools 规则码清单的一致性断言（若规则码有单一真源常量可引用）
建议方案: 规则码单一真源化（bid-tools 导出 const，bid-grade 引用）
证据: grade.rs:15-18
  pub const RULES_8: [&str; 8] = [
      "CERT_EXPIRED", "PRICE_EXCEEDS_BUDGET", "DURATION_MISMATCH", "PAGE_LIMIT",
      "MISSING_SIGNATURE", "DARK_BID_WORD", "TODO_RESIDUE", "INJECTION_PATTERN",
  ];
审计置信度: CONFIRMED
状态分类: 当前仍存在（两份清单拷贝——漂移时 fail-closed）
```

```
ID: GRADE-005
严重程度: P3
模块: bid-grade
文件路径: bidcore/crates/bid-grade/src/grade.rs；bidcore/crates/bid-tools/src/schema.rs（引用）
代码行号: grade.rs:9-11（use bid_tools::schema::RunDb）
函数/结构体/接口: grade_run / RunDb
问题描述: 判分器与被测产物共享同一份类型化 schema（bid_tools::schema::RunDb）——schema 是「被测方与判分方的共同真源」而非独立冻结契约。类型化带来布尔伪装防线（自测验证），但 schema 演进（加字段/放宽 Option）时判分器随被测方同步变化，无独立版本钉死。独立性的真正锚点是 fixtures/ 冻结数据（GT/queries/scenarios），schema 层为「同仓共享」而非独立。
触发条件: RunDb schema 演进（如某断言路径字段改 Option 且判分器 unwrap_or 默认值中性化）
实际影响: 低（核心断言 fail-closed 纪律+破坏注入覆盖主要中性化面；fixtures 冻结面独立）；理论漂移面存在
当前测试: 类型化反序列化拒绝测试（布尔伪装）；「留空字段中性化」破坏注入（TC-11 两处）
缺失测试: 不适用（架构边界登记）
建议方案: 若需强独立：判分器侧对 db.json 先过一层冻结 JSON schema 校验再进类型化解析（成本高，当前纪律下非必需）
审计置信度: CONFIRMED
状态分类: 当前仍存在（架构权衡，登记备查）
```

---

## 4. 测试有效性评估

| 测试文件 | 类型 | 独立 Oracle | 错误路径可达 | 评注 |
|---|---|---|---|---|
| tests/e2e_m1.rs（53 行） | E2E / REAL_CORPUS（corpus-A 真文件真管线四场景 TC-01/02/07/11） | ✅ 判分对齐冻结 GT/queries/scenarios（判分器与被测管线不同源） | ✅ 运行失败/判分未过均 panic 带 report 全文 | 「每轮测试」主门禁；ENV 缺件 SKIP（ENV_MISSING 横幅）+ 夹具缺件 SKIP——SKIP 与失败显式区分 |
| tests/selftest_gate.rs（17 行） | UNIT（判分器自测挂 cargo test 主入口） | ⚠️ 完美库由 GT+判分器常量构造（GRADE-002——自测目的下可接受） | ✅ 10 处破坏注入各被对应维度捕获 + 全文锚/标记回声/垃圾 REQ 三类伪对齐 + 布尔伪装类型拒绝 + TC-11 越级/留空两处场景破坏 + dedupe 合并运行 | 判分器自校密度高；破坏→维度映射逐条断言 |

selftest 内部用例矩阵：4 场景全绿 + 10 破坏 + 1 类型防线 + 1 dedupe + 2 场景破坏 = 18 检查点，failed 计数聚合断言（selftest_gate.rs:16）。

---

## 5. 结论速览

- **Oracle 独立性**：对外（真运行）成立——GT/查询/场景断言全部来自 fixtures 冻结真源，对齐器与生产抽取不同源（反标记回声三通道+三类伪对齐破坏全捕获）；selftest 完美库由 GT 构造（非生产算法造真值），属判分器自校的正确方法论，其边界已登记（GRADE-002）；同源风险仅 RULES_8 双份清单（GRADE-004，漂移方向 fail-closed）。
- **评分估算 vs 真实评标边界**：判分器不判分不调模型（政策翻转/规则码/召回均为冻结数据比对）——边界干净，无 rule_heuristic 伪装问题（那是 bid-domain scoring 的面，其 diagnostic_only 标记完备）。
- **缺陷**：唯一实质耦合是补遗后开标日硬编码（GRADE-001，P2——夹具口径复制进判分器，变更即静默错判）；metric 断言恒过（GRADE-003，归档口径一致）、schema 同仓共享（GRADE-005）为口径债。
- **门禁质量**：「未执行≠通过」「OUT_OF_SCOPE 显式留痕」「路径不可解析即失败」「破坏注入全捕获」四纪律全部有测试钉死——判分器本体可信度高。
