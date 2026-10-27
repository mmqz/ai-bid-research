# engine_probe —— issue #2 · B2 引擎侧探针（只读审计 harness）

> **对象**：mdcore @ `f8c860aa3a377fa96b15982132b69a657bbba728`（2026-10-27 main）。
> **纪律**：path 依赖指向**只读 git worktree**
> `/home/z/my-project/repos/mdcore-audit-f8c860a`（`git worktree add <path> f8c860a`
> 生成）——不修改引擎任何源码、不向 mdcore 提交任何内容（B1 红线）。
> 与 `scripts/baseline_harness` 同模式：scripts/ 下独立验证 crate，非产品代码。

## 探针清单（10 项，`cargo run` 一次全跑）

| 探针 | 核实条目 | 输入 | 结论（`artifacts/engine-probe/report.json`） |
|---|---|---|---|
| s7a zip 炸弹 | S7 | `baseline/security/hostile/01-*` | 引擎 rejected（lie header 诚实判据） |
| s7b 合法高压缩比 | S7 | 运行时构造（D2a 底 + 1.08MB 重复正文，比率 149:1） | 引擎 **accepted**（6000 段全解析）——ratio 门误报面实证 |
| s8a 嵌套 zip 未声明 | S8 | D2a 底 + word/media/attached.zip | rejected（content-type 部件校验，非嵌套检测） |
| s8a2 嵌套 zip 已声明 | S8 | 同上 + Default zip 声明 | **accepted**（落 images 枚举，零递归解压） |
| s8b 内容级伪装 | S8 | document.xml 字节=zip | rejected（no root element，XML 良构面） |
| d456 TR2 word-complex | D4/D5/D6 | 引擎仓 `test-artifacts/TR2/fixtures/`（真 Word 件） | 批注 0 提取；读面复现 A2「11.1 表格」 |
| d456 corpus-R D2a | D4/D5/D6 | `fixtures/corpus-R/D2-nonbody/D2a-*` | 文本框条款 0/2（vs bid 2）；修订插入内容缺失；页眉/页脚条款全量读出（D3 引擎有） |
| x3 TR2 word-merged | X3 | 真 Word 纵横合并表 | docx TableData 扁平（4×3→4×2），无合并结构（设计登记） |
| x3 TR2 excel-complex | X3 | 真 Excel 件 | merges=0——openpyxl 独立核证该件本无合并（TR-2D 特性表勘误）；公式 2/超链接 1 读面对 |
| x3 corpus-A B3 图纸目录 | X3 | 真实合并 A1:F1 | **merges API 生效**（r1-1c1-6 读出） |

### D6 修订/批注专项（T2 批，issue #2 评论 2026-10-27 03:28 派发；2026-10-27 追加）

| 探针 | 输入（`fixtures/corpus-R/D6-revisions/`） | 引擎现状结论（修复前「前」证据） |
|---|---|---|
| d6rev_r6a_pure_insert | R6a-纯插入澄清.docx（2 处 w:ins） | ins 内容**缺失**（「开标地点：」后空——w:ins 容器整体跳过，B2 P0 发现在新夹具复现） |
| d6rev_r6b_pure_delete | R6b-纯删除澄清.docx（2 处 w:del） | delText 不入正文（final-view 合规）但**零删除登记面**；del-only 段落保留为空段 |
| d6rev_r6c_mixed | R6c-混合修订澄清.docx（同段替换+表格内修订） | del+ins 同段内容**全缺**（段落空壳）；表格读出但格内修订内容缺 |
| d6rev_r6d_comments | R6d-带批注澄清.docx（双批注） | 批注 **0 提取**（#19）；正文文本不受批注污染 |
| d6rev_r6e_combo | R6e-修订批注组合澄清.docx | del/ins 内容全缺 + 批注 0 提取——澄清实务形态双缺口叠加 |

期望读回契约（引擎 #17/#19 修复验收的对照标准）：`fixtures/corpus-R/D6-revisions/expected-readback.json`；
标书侧当前行为基线：`baseline/parse/fixtures/corpus-R/D6-revisions/*.json`（bid_parse ins 合规/ delText
条件性泄漏——粘性 capture_text，见 docs/engine-gap-requests-m0.5.md §D6 与 T2 报告）。

## P1 双侧对跑（`engine-probe-p1` 二进制）

pdfium（mdc-pdf `Command::ExtractText` 引擎面）vs `pdftotext -layout`，三件真实招标
PDF：结论 `artifacts/engine-probe/p1-layout-parity.json` + `docs/engine-gap-requests-m0.5.md` §6
（文本层件非空白字符比 1.000、结构探针双侧同命中；扫描件双侧零文本）。

pdfium 库取件：`https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz`
→ 解出 `libpdfium.so` 置 `~/.pdfium/`（或设 `MDC_PDFIUM_LIB`）。tessdata chi_sim 见
`scripts/fixtures-requirements.txt` 契约（`~/.tessdata/chi_sim.traineddata`）。

## 复现

```bash
# 引擎只读 worktree（一次性）
git -C <mdcore-clone> fetch origin main
git -C <mdcore-clone> worktree add /home/z/my-project/repos/mdcore-audit-f8c860a f8c860a
# 探针（main = 10 探针 + d6rev 5 探针；p1 = PDF 双侧对跑；dump = 全量块 JSON）
cd scripts/engine_probe && cargo build
./target/debug/engine-probe <repo_root>          # → stdout JSON（入库版见 artifacts/engine-probe/report.json）
MDC_PDFIUM_LIB=~/.pdfium/libpdfium.so ./target/debug/engine-probe-p1 <repo_root>
# T1 双跑差分供数（引擎侧全量块 JSON——harness 最小机械映射，产品映射=B3 bid-engine）：
./target/debug/engine-probe-dump <repo_root> <out_dir> <file...>
# 差分（旧=baseline/parse vs 新=引擎 dump）：见 scripts/engine_diff.py
```

> 注：Cargo.toml 的 mdc-office-io/mdc-pdf 为**绝对 path 依赖**（指向只读 worktree，
> 环境锚定）——换环境时按上式重建 worktree 或改 path。探针输出为确定性 JSON
>（BTreeMap 序、无时间戳）；`artifacts/engine-probe/*.json` 为已入库证据基线。

> **block_texts 截断策略**（issue #2 评论 2026-10-27 03:28 PR 精简要求）：`fmt_docx_result`
> 只保留前 20 条文本样例（超限置 `block_texts_truncated: true`），全量计数由
> `paragraphs`/`tables` 字段承载——结论保留、过程数据（如 s7b 6000 段合法高压缩比件
> 的 312K 字符全文 dump）不入库。截断前后差分实证：除 s7b `block_texts` 6000→20 条
>（前 3 条样例逐字节一致）外，10 探针全部字段零变化。
