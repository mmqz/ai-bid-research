# CI 门禁审计（ci-audit.md）

基线 6105beb。审计对象：`.github/workflows/windows-check.yml`（140 行，全仓唯一工作流）——逐 step 取证；附 run_tests.py 主链不进 CI 的影响链与缺口清单。

---

## 1. 工作流结构

- 触发：push/pull_request → main + workflow_dispatch（:15-20）。
- 单 job `check`（windows-latest，:29-36）；job 级 env `MD_CORE_DEPLOY_KEY`（:35——GitHub 不允许 if 直接用 secrets 的规避写法）。
- 全局 env：CARGO_TERM_COLOR/CARGO_NET_RETRY=3/CARGO_NET_GIT_FETCH_WITH_CLI=true（:22-26——mdcore 私仓 git CLI 通道）。
- **无 Linux job、无 clippy/fmt、无 timeout 设定、无并发取消（concurrency:）**。

## 2. 逐 step 取证

| # | Step（行号） | 内容 | 评价 |
|---|---|---|---|
| 1 | `Configure mdcore read access`（:39-67） | 无 secret 时跳过（if :44）；有则写 `~/.ssh/id_ed25519`（icacls 权限收紧 :54）+ ssh-keyscan known_hosts（:55）+ https→ssh 改道仅 mdcore 仓（:57）+ `ssh -T` 连通性探测**显式不判死**（:58-67——认证成功也返回 1，归一 exit 0，注释详述 PR #15 红根因）；真门禁=cargo check 的 git fetch | ✅ 凭据纪律良好（PAT 不入仓不入日志；deploy key 只读）；「探测不判死+真门禁在后」设计正确 |
| 2 | `Install Rust toolchain`（:69-70） | dtolnay/rust-toolchain@stable——**浮动 stable**（无 rust-toolchain.toml 钉版，workspace-inventory §5） | ⚠️ 工具链漂移面：rustc 版本随上游 stable 漂移（本仓 1,118 测试系 1.99.1 验证） |
| 3 | `Cache cargo`（:72-82） | path 含 bidcore/target；**key 只 hash `bidcore/Cargo.lock`**（:81） | ⚠️ E-9：mang-app 独立 lock 变化不刷新缓存；scripts 六工程 lock 不在 key（不在 CI 无影响） |
| 4 | `cargo check`（:85-87） | working-directory: bidcore；`cargo check --workspace --all-targets` | ✅ Windows 编译门（E0596 教训的对策面）；含 all-targets（测试代码编译——guard.rs unix 分支在 Windows **不编译**，见缺口 G1） |
| 5 | `cargo check (mang-app)`（:93-95） | mang-app 独立 workspace `cargo check --all-targets`（:89-92 注释：#76 桌面壳编译门） | ⚠️ **仅 check 无 test**——39 测试零执行（FG-01/D-11 的直接 CI 根源）；含必红断言 tools==13 |
| 6 | `cargo test (SKIP 机制护航)`（:97-108） | `cargo test --workspace --no-fail-fast`（:108）；:99-107 注释详述 issue #9 SKIP 机制 + 2026-11-02 教训（默认 fail-fast 曾使 bid-parse 9 个测试二进制从来没在 CI 跑过——#168/#169） | ✅ --no-fail-fast 教训吸取；⚠️ 但 windows-latest 无 poppler/tesseract/chi_sim/pdfium → 真件链全 SKIP 计 ok（FG-02）；guard.rs 测试 unix-only 不编译（FG-14） |
| 7 | `Verify SKIP banner emitted`（:110-122） | pwsh：`$out = cargo test --workspace 2>&1 \| Out-String`（:117）→ 匹配 `SKIP: probe_corpus_a_full_chain` 两分支 echo（:118-122） | ❌ **G7/D-26：pwsh 管道吞退出码**——`$LASTEXITCODE` 取自 Out-String 恒 0，**该步 cargo test 失败也不红**；两分支恒过无法区分「SKIP 机制坏掉静默全跑」与「真跑」；附带 ~20min 重复全量跑且未 --no-fail-fast |
| 8 | `Detect external toolchain`（:124-139） | `if: always()`；探测 tesseract/pdftoppm/pdftotext/pdfinfo 四件→ENV_MISSING 写 GITHUB_STEP_SUMMARY（:132-138） | ✅ 诚实披露面（把「CI 从未真正执行过任何真实文件全链测试」写进 summary）；⚠️ 仅信息性——**门禁不因此黄/红**（E-1：披露≠门禁） |

## 3. run_tests.py 主链不进 CI 的影响链（D-27，P1）

全仓唯一 workflow 不调用 scripts/run_tests.py——T0~T19 全靠本地手跑。影响链：

```
run_tests.py 不进 CI
 ├─ T18a 评测腐化：b5_engine_gt_eval.py 自 B4 起红（parse_file 拒 docx）无人知 → 「引擎路径≥旧路径」无证据（FG-04）
 ├─ T15a real-eval GT 断言（半自证面）无门禁 → GT 漂移无探测器（FG-18）
 ├─ T17 族历史产物断言不重跑 → 当前 HEAD 行为零验证（FG-07）
 ├─ T0-T13 m1 线/合成线全绿结论只对本地环境负责 → m1 与 bid-parse 语义分叉无 CI 信号（D-18）
 ├─ T7d 暗标断言只在 SEMUL 线跑 → 产品路径 docProps/禁字面零守卫（FG-13）
 └─ T19 内嵌 cargo test 重复编译（沙箱磁盘满=环境红 FG-15）——即使进 CI 也是浪费面
```

**Python 面整体零门禁**（E-8）：scripts ~44 顶层脚本 + sdk/ + skill/checklist.py 连 `py_compile` 都没有——6 处「必炸绝对路径」（D-20）若有一道语法+干跑冒烟即可早发现。

## 4. 缺口清单（汇总 D 组 E-1..E-9 + 新增）

| ID | 级别 | 缺口 | 证据 | 修复 |
|---|---|---|---|---|
| G1 | P1 | **cargo 测试在 Windows runner 系统性空转**：真件链（e2e_m1/real_corpus_e2e/corpus_probe/kb_probe/e2e_pipeline/pdf_export）+ OCR 主链/救援全 SKIP 计 ok——CI 从未真正执行过任何真实文件全链测试 | E-1；env_preflight SKIP 语义；:97-108 | 加 Linux job（ubuntu + poppler/tesseract/chi_sim/reportlab action）跑同一套件 |
| G2 | P1 | **mang-app 无 cargo test**（39 测试含安全围栏测试零执行 + 确定性红） | E-2/D-11；:93-95 | mang-app 进 CI test（一次改动同修 FG-01） |
| G3 | P1 | **run_tests.py 主链不进任何 CI** | E-3/D-27；§3 影响链 | 最小版：Linux job 跑 T0-T7+T17 活体档+T18 |
| G4 | P2 | 无 Linux job：`#[cfg(unix)]` 分支（guard 治理测试/kill 组路径）与 Linux-only 依赖面零 CI 覆盖——E0596 教训只修了 Windows 侧，反向盲区对称存在 | E-4；guard.rs:339-341 | 同 G1 |
| G5 | P2 | 无 clippy/fmt 门禁；syntax-check.sh（本地秒级语法门）未入 CI | E-5 | CI 加 `cargo clippy --workspace -- -D warnings`（可先 allow 清单） |
| G6 | P2 | web-ui 零 CI：typecheck（tsc --noEmit）/build/CSP/HTML 结构无自动化——唯一验证是入库截图（154 张） | E-6/D-06 | node job：typecheck+build+（Playwright 冒烟可后置） |
| G7 | P2 | **「Verify SKIP banner」步骤可假绿**：pwsh 管道吞退出码 + 双分支恒过 + 重复全量跑 ~20min | E-7/D-26；:117-122 | §2 行 7 修复方案 |
| G8 | P3 | scripts/m1/SDK/Skill 无 CI——Python 连 py_compile 无门禁（6 处必炸路径暴露面） | E-8 | py_compile 批量 step（秒级）+ e2e 冒烟后置 |
| G9 | P3 | 缓存 key 只含 bidcore/Cargo.lock——mang-app 独立 lock 变化不刷新 | E-9；:81 | key 加 mang-app lock hash |
| G10 | P3 | 工具链浮动 stable 无钉版；无 job timeout；无 concurrency 取消旧跑 | workspace-inventory §5；:70 | rust-toolchain.toml 或 action pin；timeout-minutes: 60；concurrency 组 |
| G11 | P3 | 外部工具链探测（:124-139）只写 summary 不构成门禁信号——「披露≠门禁」 | :124-139 | SKIP 数>0 时输出 annotation（warning）供 PR 页可见 |

## 5. 结论

1. **CI 真实防护面 = 「Windows 编译绿 + 纯 Rust 单元/契约层绿」**（1,118 测试中不依赖外部工具的部分）——这一层纪律尚可（--no-fail-fast 教训、SKIP 横幅、deploy key 凭据、诚实 summary）。
2. **系统性盲区三件套**：真件链 CI 空转（G1）+ mang-app 零执行（G2）+ Python 主链零 CI（G3）——任何 PR 携带解析链回归/m1 语义漂移/评测腐化/web-ui 回归，CI 照绿（test-quality-audit §5、D 组 F 节同结论：**双轨门禁实质单轨**）。
3. **唯一「假门」**：Verify SKIP banner 步骤（G7）——自身可假绿且浪费 20min/轮，应优先修（行级修复成本最低、收益即时）。
4. 修复杠杆排序（D 组建议背书）：① mang-app 入 CI test；② Linux job（装齐外部工具即得真件链门禁——一次解 G1/G3/G4/FG-02/FG-14）；③ real_eval SAMPLES 改指仓内；④ SKIP banner step 修退出码；⑤ T17 活体档。
