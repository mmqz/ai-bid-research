# ai-bid-research 架构审计报告（2026-11-05~06，已归档）

> **这是时间点快照，不是当前状态。**
> 基线：main `6105beb`（2026-11-04）。
> 归档原因：审计分支携带大量过程产物，只把 `audit/` 目录归档到 main。

## 规模

```
两轮审计合并：
  phase-1（R0~R7）：基线 → 模块审计 A/B/C/D → callgraph → 业务链 →
                    集成契约 → 安全 → 语料验证 → 变异测试 → 假绿登记 → 终验
  v2-full-coverage（R1~R8）：全量模块审计（16 模块）+ 库存清单 + 复核

77 个文件：
  19 份主报告 + 16 份模块审计 + 12 份库存清单 + 2 份 pilot + 证据目录
```

## 最有价值的产出：假绿登记册

**`audit/false-green-register.md`（FG-01 ~ FG-18）**

**定义**：「假绿」= 测试/门禁显示绿色，但**不构成对当前生产行为的可信验证**。

### 三条最严重的（都是 H 级）

#### FG-02：真件链在 CI 上恒 SKIP 恒绿

```
windows-latest 无 poppler / tesseract / chi_sim / pdfium；
→ e2e_m1、real_corpus_e2e、corpus_probe/kb_probe、e2e_pipeline、
  pdf_export、guard.rs 全模块（cfg(unix)）全部 SKIP 计 ok；
→ CI 绿，但主解析通道的全部回归防线为零。
```

修复建议：加 Linux job（装齐 poppler/tesseract/chi_sim）——一次改动解多个缺口。

**已处理**：#295（B 在做，装 poppler 到 CI）。

#### FG-03：「Verify SKIP banner」步骤本身是假绿门

```
步骤名义验证「SKIP 横幅真的出现」——
实际 pwsh 管道 $out = cargo test --workspace 2>&1 | Out-String；
→ 步骤退出码取自 Out-String（恒 0）；
→ 该步 cargo test 失败也不红；
→ 无法区分「SKIP 机制坏掉静默全跑全绿」与「工具在位真跑」。
```

审计原文：

> 作为「机制自证」的步骤自身不构成门禁；假红也被吞——**比不做更糟**（浪费+误导）

修复建议：

```powershell
$out = cargo test ...; $code = $LASTEXITCODE; ...; exit $code
横幅断言失败时显式 fail；删除重复全量跑
```

#### FG-01：mang-app 39 个测试零执行

```
CI 对 mang-app 只 cargo check --all-targets（编译不执行）；
39 测试含 fs 围栏 write_bytes_guard、paths 穿越、llm 掩码安全测试——
全部形同虚设；
且 diag.rs:303-304 断言 tools==13（审计时 TOOL_NAMES=25）；
→ 该套件当前必红从未被发现，说明门禁失效已被实证。
```

**注意：审计后 TOOL_NAMES 已增至 32**——这个断言现在是 13 vs 32，差距更大。

修复建议：

```
① 撤 mang-app 的 [workspace] 空表并入 bidcore，或 CI 加 cargo test step
② 13 → 动态断言（= TOOL_NAMES.len()）
③ 加「断言数=TOOL_NAMES.len()」防再腐
```

## 其他重要发现

### FG-04：T18a 引擎路径评测断裂（自 B4 起红无人知）

```
b5_engine_gt_eval.py 调 bid_parse::parse_file(path, "A-招标文件.docx")；
B4 后该函数仅支持 PDF（显式拒 docx）→ panic；
→ T18a 红（但 Python 主链无 CI，红与绿同样无人看见）。
```

### FG-05：SDK 02-full-pipeline 红（#242 起无人知）

```
SDK e2e 10 用例 9 绿 1 红：02-full-pipeline 因 #242 E_CHARS_BELOW_MIN 门真实生效；
夹具写死于 #242 前 → 全量 BLOCKER；
门本身按设计工作，但旗舰全链验收在基线即红且不在 CI。
```

### FG-06：SEMUL 仿真链无交叉验证

```
run_tests T7/T8/T9 全绿被历史文档引为「V1/V2/V3 验证全绿」；
实际跑的是 Python 语义仿真（头注自认 SEMUL ≠ ENGINE_RUN）；
Rust 实现与仿真分叉无探测器。
```

### FG-07：断言入库历史产物

```
T17b/c/d 断言的是 artifacts/ 里的历史 JSON（含过期 SHA f8c860a/f7ddd39）；
→ 能防产物被误删，不能防当前代码行为漂移；换机器/换 HEAD 即断。
```

### FG-08：engine_probe 钉旧 rev 对账

```
engine_probe 自称「与 bid-engine 同 rev」，实际 rev=3204342d ≠ 生产 f4328ebf；
→ 引擎三门对账失效。
```

## 审计的其他结论

### 语料验证（R4 re-run）

```
veto recall 0.386 → 0.700（修复后）。
```

### 变异测试（v2 R5）

```
6 项白名单：5 项 CAUGHT（篇幅门/导出复算/必选阻断/路径围栏/评分份额）；
receipt 验签整体 CAUGHT 但两消费面缺伪造负例（局部 TEST-GAP）。
```

### R8 独立交叉复核

```
#243/#245 修复变异级验证（M1/M2 KILLED）；
13 项未修复确认；
REV-01 main 真实红灯二分归因（2e98319 基线失修，CI 缺口兑现）；
main 1277 测试实跑统计。
```

## 索引

```
false-green-register.md         ⭐ 假绿登记（FG-01~18）——最有价值
final-verification-report.md    终验报告
risk-register.md                风险登记册
known-issue-verification.md     已知问题核验
callgraph-audit.md              调用图（4 BROKEN / 2 DEGRADED）
business-chain-audit.md         业务链审计
integration-contract-audit.md   集成契约
security-audit.md               安全审计
test-quality-audit.md           测试质量
corpus-verification.md          语料验证
mutation-report.md              变异测试
ci-audit.md                     CI 审计
dependency-graph.md             依赖图
workspace-inventory.md          工作区清单
baseline.md                     基线

module-audit-A.md ~ D.md        模块审计（phase-1 视角）
modules/module-audit-*.md       模块审计（v2 视角，16 模块）
inventory/*.csv                 库存清单（12 份）
pilot/*.md, *.csv               试点审计
review/review-report.md         复核报告
evidence/                       证据（变异/测试运行/GitHub 状态）
```

## 使用方式

```
✅ 当「问题清单」用——FG-01~18 都是真实的假绿，不是理论推测
✅ 当「证据集」用——每条都带代码位置和实证

✗ 不要当「当前状态」用——审计后已合并多个修复 PR
✗ 不要当「全部问题」用——审计基线是 6105beb，此后新增的代码未审
```

## 与主仓 issue 的对应

| 审计发现 | 主仓 issue | 状态 |
|---|---|---|
| FG-02 真件链 CI 恒 SKIP | #295 | B 在做 |
| FG-03 Verify banner 吞退出码 | 待开 | 未处理 |
| FG-01 mang-app 测试零执行 | 待开 | 未处理 |
| FG-04 T18a 引擎评测断裂 | 待开 | 未处理 |
| FG-05 SDK 02 红 | 待开 | 未处理 |
| REV-01 main 真实红灯 | #292（C 在做） | 处理中 |