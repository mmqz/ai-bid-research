# AGENTS.md — 代理协作规则（两仓通用）

> 本文件是**唯一权威**的代理协作规则。与 issue 正文冲突时以本文件为准。
> 最后更新：2026-11-06

---


---

# 零、最高优先级铁律（违反即违规）

## 0. 代码完成 → 提交 PR，**永不直接合并 main**

```
❌ 绝对禁止：
  git checkout main && git merge <branch> && git push origin main
  git push origin HEAD:main
  任何形式的「直接推 main」

✅ 唯一正确流程：
  1. 在功能分支上提交
  2. push 功能分支
  3. 开 PR（gh pr create 或 gh api pulls）
  4. 等 Lead 审核合并
```

**为什么**：

```
· main 是唯一可信基线，任何未经审核的改动都可能破坏它
· 直接推 main 绕过审核 = 其他代理的 stale base 事故源头
· PR 是审核痕迹，没有 PR 就无法追溯「谁在什么时候改了什么」
· 回滚单个 PR 容易，回滚被污染的 main 极难
```

**如果分支与 main 冲突**：

```
✅ 正确：git fetch origin main && git rebase origin/main && push 功能分支（--force）
❌ 错误：把 main 合进分支后直接推 main
```

**例外：无。** 即使是「一行文档改动」「紧急修复」也必须走 PR。


---

# 零之二、认领与身份（必须先做）

## 0.1 开工前认领 issue

**开始任何工作之前，必须在 issue 上留言认领。**

```bash
gh issue comment <number> --repo mmqz/<repo> --body "认领：<你的身份> 承接本任务，预计 <时间>"
```

**为什么**：

```
· 多个代理同时看到同一个 issue → 不认领就会重复劳动
· Lead 需要知道每个 issue 归属谁，才能派单和催进度
· 认领记录是「谁做的」的第一手证据
```

## 0.2 身份命名规则

### 情况 A：用户已命名

```
用户说「你叫 C」→ 你的身份就是 C
所有提交、PR、issue 评论都用这个名字
```

### 情况 B：用户没命名

**自己生成一个不重复的随机后缀，格式 Agent-<4位小写字母数字>**

```
示例：Agent-hd2k4 / Agent-x7m2 / Agent-p9qr

生成方法（任选）：
  Linux:   echo "Agent-$(head -c4 /dev/urandom | xxd -p | head -c4)"
  Python:  python3 -c "import secrets,string;print(chr(34)+chr(65)+chr(103)+chr(101)+chr(110)+chr(116)+chr(45)+chr(34) + ...)"
  手动:    随机敲 4 个字符
```

**避免重复**：

```
生成后先查一下有没有被占用：
  在任务面板 #229 看已有的名字列表
  或 gh api repos/mmqz/<repo>/issues?state=all --jq 筛选

重复了 → 重新生成一个
```

### 身份一旦确定，全程不变

```
❌ 今天叫 Agent-hd2k4，明天叫 Agent-temp1
✅ 一个会话内所有产出都用同一个名字
   跨会话如果记得上次的名字，继续用；不记得就新生成
```

---

## 0.3 身份必须出现在四个地方

### ① issue 认领评论

```markdown
认领：Agent-hd2k4

承接本任务，预计 2 小时。
计划：
  1. xxx
  2. xxx
```

### ② commit message 末尾

```bash
git commit -m "fix(#285): 标题" -m "Co-Authored-By: Agent-hd2k4 <agent-hd2k4@mang.local>"
```

**格式固定**：`Co-Authored-By: <身份> <agent-<小写>@mang.local>`

### ③ PR 描述开头

```markdown
**提交者**：Agent-hd2k4

## 交付概要
...
```

### ④ 完成时在 issue 的关闭评论

```markdown
Agent-hd2k4 交付完成

PR #xxx 已合并，commit abc1234

验收对照：
| 要求 | 证据 |
|---|---|
...
```

---

# 0.4 为什么身份要可追溯

```
· 出问题时知道找谁（「这个 PR 是 Agent-x7m2 交付的，问他」）
· 质量统计（谁的 PR 一次过、谁的总要返工）
· 避免「无人认领」的悬空工作
· 避免 A 的代码记到 B 头上
```

## 0.5 认领冲突处理

```
如果发现 issue 已被人认领：
  · 不要重复做
  · 在评论里说明「看到 Agent-xxx 已认领，我不重复」
  · 去做别的 issue

如果发现认领者很久没动静（>1 天）：
  · 先问一句「Agent-xxx 还在做吗？」
  · 对方确认放弃后才接手，不要直接抢
```

---

# 一、工作流铁律

## 1. 开分支前必须同步 main

```bash
git fetch origin main && git checkout -f origin/main
git checkout -b <type>/<issue-number>-<slug>
```

**为什么**：2026-11-06 单日三次 stale base 事故，每次都携带数千行无关回滚：

```
#258  基于旧 main → 携带 1,231 行回滚
#259  基于旧 main → 携带 6,567 行回滚
#235  基于旧 main → 携带 165 行 web 删除
```

**开 PR 前自检**：

```bash
git diff --stat origin/main | tail -3
# 如果 diff 里出现你没改过的文件 → 分支基于旧 main，重新来
```

## 2. 每个 commit 立即推送并**确认成功**

```bash
git push origin HEAD:<branch>
# 必须看到 "xxxx..yyyy HEAD -> branch" 才算成功
# 只看到 "Everything up-to-date" 但本地有 commit → 推错了分支
```

**为什么**：沙箱会被回收，本地未推送的 commit 全部丢失。

**回收后的恢复检查清单**：

```
[ ] git log origin/<branch> --oneline | head -3    ← 确认云端有你的 commit
[ ] git status                                     ← 确认工作区干净
[ ] 工具链是否还在（cargo --version）
[ ] 如果工具链丢了，重装到持久化路径
```

## 3. 不要等 CI，开完 PR 立即做下一项

```
❌ 错误做法：
  开 PR → 盯 CI → 等 10 分钟 → 绿了 → 才开下一项
  结果：一个工作周期只推进一个任务

✅ 正确做法：
  开 PR → 立刻开下一个分支做下一项
  CI 红了由 Lead 反馈，你回来修
  结果：一个工作周期推进 2-3 个任务
```

**唯一例外**：改共享聚合文件（gates.md / manifest.json）时建议等 CI，
因为冲突在这类文件上。但 gates.md 正在拆分（#248），拆完这个例外消失。

## 4. 提交 PR，**不得自行合并**（见第 0 条）

```
Lead 负责合并。你只负责交付。
自行合并 = 绕过审核 = 违规。
这条是最高优先级，没有例外。
```

## 5. 一个 PR 对应一个 issue

```
❌ 一个 PR 塞 3 个 issue → 审核困难、回滚困难
✅ 一个 PR 一个 issue → 除明确的依赖链（stacked PR 要说明）
```

## 6. issue 做完就关，不留悬空

**PR 合并后立即：**

```
① 在 issue 留「已完成」评论，含：
   · 合并 commit SHA
   · 验收对照表（每条要求 → 证据）
   · 本地实测命令与输出
② 关闭 issue
```

**为什么**：issue 不关 → 积压数字虚高 → Lead 无法判断真实进度。

**2026-11-06 实例**：ai-bid 14 个 open issue 里，至少 7 个是「已完成但没关」。

---

# 二、代码质量铁律

## 7. 真实测试，不许伪报

```
✅ 「本机 Linux 实测：cargo test -p xxx 全绿，输出见下」
✅ 「Windows 未验证——CI 只做编译检查」
✅ 「环境缺 poppler，本测试 SKIP，未执行」

❌ 「测试应该没问题」（没跑）
❌ 「CI 全绿所以没问题」（CI 可能没覆盖到）
❌ 把 SKIP 当成 PASS 报告
```

**关键：CI 全绿 ≠ 稳定。** 实例：

```
songling 的 9 条验收门依赖 poppler，Windows CI 走 SKIP
→ 这些门在 CI 里从未运行
→ #266 造成的配额回归（0.824→0.820）在 main 上静默存在
→ 直到某代理在本机 Linux 跑才暴露
```

## 8. 不许伪造完成

```
❌ 代码写了一半就报「已完成」
❌ 测试只跑一部分就报「全绿」
❌ 断言跑了但没验证就报「通过」
```

**如果没做完，明确说「未完成 + 卡在哪 + 需要什么」。**

## 9. 不放宽断言迁就实现

```
❌ 测试红了 → 把断言改松 → 绿了 → 报告完成
✅ 测试红了 → 查清是「代码错了」还是「基线该更新」
   · 代码错了 → 修代码
   · 基线该更新 → 改基线 + 写清根因 + 说明为什么不是放宽标准
```

**判据**：`target` / 阈值如果动了，就是放宽。`known` 基线随上游修复同步，不是放宽。

## 10. 用子代理加速

```
可以并行的工作 → 开子代理并行做
例：3 个独立 crate 的改动 / 3 份语料的验证 / 多方案对比

但：
  · 子代理改的文件范围必须互不重叠（Lead 会检查）
  · 子代理的产出要自己复核（不能直接信）
  · 不确定子代理会怎么做的事，不要委派
```

---

# 三、环境与资源

## 11. 硬盘管理

```
沙箱磁盘有限（曾经被 cargo target 撑爆）。

必须做的：
  · 用独立 CARGO_TARGET_DIR，不要污染仓内
    export CARGO_TARGET_DIR=/tmp/cargo-target-<yourname>
  · 定期清理：rm -rf /tmp/cargo-target-*  （只清自己的）
  · 构建产物不要提交进仓（.gitignore 要覆盖）
  · 大文件（fixture PDF / 词料）用 git-lfs 或引用外部路径

检查磁盘：
  df -h .        # Linux
  Get-PSDrive C  # Windows
```

## 12. 工具链持久化

```
沙箱重置会丢工具链。装到持久化路径：
  · Rust: ~/.cargo （默认就在持久位置）
  · Python venv: ~/.venvs/xxx
  · 不要在 /tmp 里装工具链
```

## 13. 环境缺件如实报告

```
本仓 CI 无 poppler / tesseract / LibreOffice。
本机环境也常常缺。

遇到缺件：
  ① 测试框架会打 SKIP 横幅 → 如实报告「SKIP，未执行」
  ② 不要为了「让测试通过」去伪造环境检查
  ③ 如果任务必须依赖缺失工具，明确说「需要 Lead 提供 X 环境」
```

---

# 四、任务执行

## 14. 先读 issue 全文，不要只看标题

```
issue 正文的「【任务指令】」章节 = 完整派单
包含：背景 / 目标 / 验收 / 纪律 / 裁定点

不要：
  ❌ 只看标题就开始写
  ❌ 跳过「明确不做」章节（会做多余的事）
  ❌ 忽略「需要 Lead 裁定」的点（要主动提）
```

## 15. 有歧义就问，不要猜

```
✅ 「issue 里说的 X 我理解成 Y，对吗？」
✅ 「这里有两个方案，我倾向 A，理由如下，请确认」
❌ 猜一个方向埋头做 2 小时
```

## 16. 主动报告偏离

```
如果实现与原计划不同：
  · 在 PR 描述里明确写「原计划 X，实际做了 Y，因为 Z」
  · 不要悄悄改方案
```

## 17. 发现新问题开 issue，不要顺手修

```
✅ 发现缺陷 → 开 issue → 继续做手上的
❌ 发现缺陷 → 顺手改了 → PR 变大 → 审核困难

例外：修复是当前任务的前置条件时，可以顺带修，但要在 PR 里单独 commit 说明。
```

---

# 五、数据与语料

## 18. 不要为了样本过拟合

```
当前 10 份真实语料，但只有 3 个平台来源（黑龙江/内蒙古/未知）。
章节骨架 Jaccard：组内 0.95-0.96，跨平台 0.028。

要求：
  · 识别规则用「形态」不用「字面」
    ❌ if chapter == "采购需求"
    ✅ if 章节含「参数表/技术要求」表格
  · 每条结论标注适用范围：「基于 3 个平台来源，未验证其它省份」
  · 发现硬编码固定字符串 → 单独开「过拟合缺陷」issue
```

## 19. 语料文件名不改

```
❌ tender.pdf
✅ 超融合集群招标文件（[230001]GCZX[GK]2026000720261023001）.pdf

理由：文件名带项目名称和编号，无上下文 Agent 会从文件名读这两项。
      改名会剥夺这条路径，改变测试条件。
```

---

# 六、沟通

## 20. 汇报格式

```markdown
## 交付概要
PR #xxx / 分支 / commits

## 做了什么
（按验收条目对应）

## 实测证据
（命令 + 输出，不要转述）

## 未做的 / 有偏离的
（如实列）

## 需要 Lead 裁定的
（如果没裁定点就写「无」）
```

## 21. 不要美化

```
❌ 「基本完成」「应该没问题」「大致符合预期」
✅ 「完成 8/10，第 9 项卡在 X，第 10 项未开始」
```

---

# 七、跨仓协作

## 22. 两仓隔离

```
mdcore          引擎（文档处理：PDF/DOCX/XLSX/MD/Canvas）
ai-bid-research 标书应用（消费引擎）

规则：
  · 引擎能力改动 → mdcore
  · 标书业务改动 → ai-bid-research
  · 不要在 ai-bid 里实现引擎能力
  · 不要在 mdcore 里写标书逻辑
```

## 23. 依赖关系

```
ai-bid 依赖 mdcore（通过 crates.io 或 git rev）
改动 mdcore 后，ai-bid 要同步升级依赖版本。
```

---

# 八、快速检查清单

**开工前**：
```
[ ] 在 issue 上留言认领（含身份）
[ ] 确认 issue 没被别人认领
```

**开分支前**：
```
[ ] git fetch origin main && git checkout -f origin/main
[ ] git diff --stat origin/main 无意外文件
```

**每个 commit 后**：
```
[ ] git push 成功（看到 xxxx..yyyy）
[ ] git log origin/<branch> 确认云端有
```

**开 PR 后**：
```
[ ] PR 描述开头有「**提交者**：<身份>」
[ ] 立即开下一项，不等 CI
[ ] PR 描述含：验收对照 + 实测证据 + 未做的
```

**PR 合并后**：
```
[ ] issue 留「已完成」评论（含 SHA + 证据）
[ ] 关闭 issue
[ ] 删除已合并分支
```