# CLEANUP.md —— audit/ 工作区卫生政策与记录（issue #195）

> 交付物：`cleanup-audit.ps1`（清理脚本）+ 本文档 + `test-cleanup-195.sh`（模拟工作区测试）。
> **audit/ 本身不是 git 仓库**（issue #195 原文），清理动作发生在 Windows 机器的
> `E:\Code\ai\md\audit`，不入库；此处入库的是脚本、政策与验证记录。

---

## 一、背景（现状量化，来自 #195）

`E:\Code\ai\md\audit` 顶层散落 **5240 个文件 / 110.6 MB**，另有 **52 个子目录**：

| 扩展名 | 数量 | 体积 | 说明 |
| --- | --- | --- | --- |
| .log | 4263 | 49.4 MB | 构建日志占大头（`_w168b.log`、`_m93_bb.log`、`_ts_mdcocmain.log`…） |
| .txt | 388 | 3.8 MB | 多为日志转存 |
| .md | 381 | 1.3 MB | 混有调试转储与正式文档 |
| .ps1 / .bat | 61 / 60 | — | 一次性 CI 日志抓取脚本为主 |
| .png/.json/.py/.rs/.pdf/.docx 等 | 零散 | — | 产物与样本 |

## 二、四层安全模型（安全优先于彻底）

这是**不可逆操作**，脚本按四层防护设计：

1. **默认 dry-run**：不加 `-Execute` 时只读——产出逐项清单 CSV + 摘要，不移动、不删除任何文件（唯一副作用是创建 `_work/logs/` 放日志）；
2. **删除 = 7 天缓冲**：所有"删除"都是**移入** `_work/.trash/<时间戳>/`（同卷改名，秒级）。7 天后由 `-Weekly` 周任务按批次日期永久清除，或复核后手动 `-PurgeTrash` 立即清空。期间任何误判都可原样拖回；
3. **不确定一律归档**：`_archive/` 只进不删，人工判断后再处置；
4. **每批护栏**：默认每 500 项一批，每批完成后校验 —— 两个仓库 `git status` 与开工基线**逐字一致**、`.git` 在、`ai-bid-research/`、`mdcore/`、`_pack/`（基线存在时）都在。任一异常**立即中止**（已移动的都在回收站暂存，可恢复）。

## 三、保护名单（绝不触碰）

```
ai-bid-research/   标书主仓（含 .git 校验）
mdcore/            引擎主仓（含 .git 校验）
_pack/             当前打包源（基线存在则纳入护栏）
_archive/          归档区（本脚本的输出目标之一）
_work/             唯一临时工作区（本脚本的输出目标之一）
docs/              保留文档去向
```

另有两道路径保险：audit 根**本身是 git 仓库** → 拒绝运行；audit 根**位于任何上层仓库工作区内** → 拒绝运行（防止散落文件其实被跟踪）。

## 四、分类规则

### 顶层文件

| 类别 | 判定 | 去向 |
| --- | --- | --- |
| KEEP | 命中 #195 保留名单（见下） | `.md` → `docs/`；截图 → `docs/screenshots/`；样本原地保留 |
| A 纯日志 `.log` | 直接回收；文件名含 evidence/证据/keep/final/验收 → 归档 | `_work/.trash/` 或 `_archive/txt-review/` |
| `.txt` | 调试命名（`_` 开头、tmp/temp/debug/scratch/test- 前缀等）→ 回收；内容日志形（≥10 行且时间戳行占比 ≥70%）→ 回收；其余归档 | 同上 / `_archive/txt-review/` |
| B 脚本 `.ps1/.bat/.cmd/.py/.vbs/.js/.sh` | **扫内容**：调试命名或一次性 CI 抓取（含 run 号 / `gh run` 且无可复用结构且 ≤150 行）→ 回收；有 param()/函数/文档头 → 归档待审 | `_work/.trash/` / `_archive/scripts-review/` |
| C 文档 `.md` | 调试命名 → 回收；其余归档（有价值者人工挪 docs/） | `_work/.trash/` / `_archive/docs-review/` |
| D 产物 `.png/.pdf/.docx/.zip/.json/.rs/...` | 调试命名 → 回收；其余归档待审 | `_work/.trash/` / `_archive/artifacts-review/` |
| 隐藏文件（`.env`、`.gitignore` 等） | **只归档不删**（.env 可能含密钥） | `_archive/unknown-review/` |
| `CLEANUP.md` | 脚本自身输出，原地保留（幂等） | 原地 |

### 子目录（保护名单之外）

| 判定 | 去向 |
| --- | --- |
| 含 `.git`（疑似实验仓库） | **整体归档** `_archive/subdirs/`（.git 完好，绝不删） |
| 空目录 | `_work/.trash/.../dirs/` |
| 垃圾占比 ≥80%（.log/.txt/.rlib/.pdb、deps/、incremental/、.fingerprint/、调试命名文件） | `_work/.trash/.../dirs/` |
| 内容混杂（<80%） | 归档 `_archive/subdirs/`；若含 input/expected/sample/golden 命名的 .docx/.xlsx/.json → 标注"疑似测试夹具，建议人工确认后挪入对应仓库 fixtures/" |

> **为什么夹具不自动挪进仓库**：#195 验收要求两仓库 `git status` 干净。往 `fixtures/` 塞未提交文件会让仓库变脏。因此夹具类内容先整体归档并在清单里标注建议去向，由人工走正常提交流程。

## 五、保留名单（#195 原文 → 去向）

| 文件 | 去向 |
| --- | --- |
| `mang-bid-3d76c7ee-20261102.zip` | 顶层原地 |
| `AUDIT-engine-vs-bid.md` | `docs/` |
| `TEST-REAL-DOC-PIPELINE.md` | `docs/` |
| `TEST-WORD-DEEP.md` | `docs/` |
| `CI-BURST-README.md` | `docs/` |
| `ui-*.png` / `pdf-*.png` / `canvas-*.png` | `docs/screenshots/`（是否仍有引用价值由人工判断后自行处置） |
| `engine-export.docx` / `engine-export.pdf` | 顶层原地 |

## 六、_archive/ 布局

```
_archive/
  scripts-review/     临时脚本（清单含逐个判定理由）
  docs-review/        文档
  txt-review/         文本（含证据名日志）
  artifacts-review/   产物/样本
  unknown-review/     未知类型 + 隐藏文件
  subdirs/            整目录归档（含疑似仓库、疑似夹具）
```

每个文件为何这样判，见当次 `_work/logs/yyyymmdd-cleanup-manifest-*.csv`（Name / Kind / Size / Category / Verdict / Reason / Destination 七列）。

## 七、使用方法（Windows）

```powershell
# 0) 从本仓库 scripts/workspace-cleanup/ 取脚本（或任意位置），在 audit 所在机器：

# 1) 干跑（只读，产出清单）
.\cleanup-audit.ps1 -AuditRoot E:\Code\ai\md\audit

# 2) 人工复核清单（Excel 打开 CSV，按 Verdict/Reason 列筛选）
notepad E:\Code\ai\md\audit\_work\logs\*-cleanup-manifest-*.csv

# 3) 确认后执行（删除=移入 7 天缓冲回收站）
.\cleanup-audit.ps1 -AuditRoot E:\Code\ai\md\audit -Execute

# 4) 终检通过后，复核 _work/.trash/ 无误，永久清空回收站
.\cleanup-audit.ps1 -AuditRoot E:\Code\ai\md\audit -PurgeTrash -Yes

# 每周自动清理 _work/（可选，装一次计划任务，命令见 _work/README.md）
```

执行后 audit 根会自动生成/刷新 `CLEANUP.md`（统计 + 保留去向 + 归档清单 + 完好性结论）。

> 控制台中文乱码时先 `chcp 65001` 或改用 pwsh 7；文件输出均为 UTF-8（CSV 带 BOM，Excel 直开）。
> 兼容 Windows PowerShell 5.1 与 pwsh 7+（避免 5.1 之后的语法）。

## 八、验收对照（#195 五个勾）

| #195 验收项 | 由什么证明 |
| --- | --- |
| 顶层散落文件 5240 → <100 | Execute 输出「清理后：顶层文件 N 个」+「验收①达标」横幅；CLEANUP.md 统计表 |
| 顶层只剩两仓库 + _pack + _archive + _work + 少量保留文档 | `Get-ChildItem -Force` 目视 + CLEANUP.md 统计 |
| 两仓库完好（git status 干净，cargo metadata 能跑） | 每批护栏（git 基线逐字比对）+ 终检；cargo 结果写进 CLEANUP.md 第五节（SKIP 也显式登记原因） |
| CLEANUP.md 已写 | 脚本自动生成 audit/CLEANUP.md（本文件是政策全文，工作区那份是执行记录） |
| _work/ 结构与规矩已建立 | Execute 自动建 `_work/logs`、`_work/scripts`（含脚本自拷贝）、`_work/README.md`（含计划任务一键安装命令） |

## 九、今后规矩

1. 一切临时产物只写 `_work/`，不再散落 audit 顶层；
2. 日志统一 `_work/logs/yyyymmdd-xxx.log`；临时脚本 `_work/scripts/`；
3. 每周清理一次 7 天前的文件（计划任务命令见 `_work/README.md`；回收站按批次日期同规则过期）；
4. 不确定的一律 `_archive/`，人工判断后再处置；
5. 需要入库的产物（夹具、文档）走正常 PR，不直接往仓库目录里扔。

## 十、验证记录（诚实口径）

- **本测试在 Linux pwsh 7.4.6 上跑通**（`test-cleanup-195.sh`，68/68 全绿）：干跑零副作用、完整执行（145 文件 + 12 目录的模拟工作区：分类/保留/归档/护栏全对）、幂等复跑、周清理 7 天规则、回收站确认清空、脏仓库基线保护、错误路径/audit 是仓库/audit 在仓库内三种拒绝、清单 CSV 内容抽检；
- **Windows 原生未实测**（#9 双环境口径）。脚本刻意用 5.1 兼容语法 + `Join-Path` 拼路径，Linux pwsh 与 Windows 同引擎；建议 Windows 首跑务必先 dry-run 复核清单。

## 十一、已知边界

- 「删除」在本脚本语义里是**移入 7 天缓冲回收站**，永久删除只发生在 `-PurgeTrash` 或周任务过期后——这是刻意保守，代价是 `_work/.trash/` 会暂占 ~50 MB 磁盘，最多 7 天；
- 被占用（正在写入）的文件移动失败会**保留原地并计数**，下轮再收，不阻塞整体；
- 上限保护：清单/目录抽样上限 3000 项（防意外巨型目录拖死脚本），超过部分按已抽样比例判定。
