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
# 探针（main = 10 探针；p1 = PDF 双侧对跑）
cd scripts/engine_probe && cargo build
./target/debug/engine-probe <repo_root>          # → stdout JSON（入库版见 artifacts/engine-probe/report.json）
MDC_PDFIUM_LIB=~/.pdfium/libpdfium.so ./target/debug/engine-probe-p1 <repo_root>
```

> 注：Cargo.toml 的 mdc-office-io/mdc-pdf 为**绝对 path 依赖**（指向只读 worktree，
> 环境锚定）——换环境时按上式重建 worktree 或改 path。探针输出为确定性 JSON
>（BTreeMap 序、无时间戳）；`artifacts/engine-probe/*.json` 为已入库证据基线。
