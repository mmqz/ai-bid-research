# R7 跨仓接口一致性审计（integration-contract-audit.md）

基线 6105beb（ai-bid-audit 只读）；mdcore 引擎仓克隆 /home/z/my-project/mdcore-audit @ f4328eb（= 生产实钉 rev，只读）。
范围：bid-engine 接触面纪律 / rev 钉扎与溯源 / 对账脚本 / 结构镜像语义 / 引擎能力漂移。

---

## 1. 三包同 rev 钉住确认 ✅

`bidcore/crates/bid-engine/Cargo.toml`（:33/:36/:39）三个 git 依赖**同 rev 钉住**，单源纪律成立：

```toml
mdc-office-io = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", ... }   # :33
mdcore         = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", ... }   # :36
mdc-pdf        = { git = "https://github.com/mmqz/mdcore", rev = "f4328ebf4f936e1321a2254382d59f9ab9f94b7b", features = ["engine-pdfium"] }  # :39
```

- rev 演进链：3204342d →（#110 任务 2，87 提交）→ aa0c859d →（后续 bump）→ **f4328ebf**（Cargo.toml 注释 :19-20 自述「#110 任务 2 指令 bump，旧 rev 3204342d → 本 rev 共 87 提交」——注：注释把 bump 目标写作本 rev，与 lib.rs 常量的 aa0c859d 漂移，见 §3）。
- mdcore-audit 克隆 HEAD = f4328eb（`git log --oneline -1` = `f4328eb fix(#157)`），与实钉一致——本审计引擎侧全部结论建立在**生产真实 rev** 上。
- calamine = "0.36"（Cargo.toml:43）与引擎 office-io 同版（office-io/Cargo.toml:32 calamine = "0.36"）——类型面零语义仅字段搬运的设计成立。
- 私有仓凭据：CI 只读 deploy key（windows-check.yml:39-67），PAT 不入仓不入日志（工作区账本核实）。

**裁定：IMPLEMENTED（三包同 rev + calamine 同版）**

---

## 2. 唯一接触面纪律验证 ✅（m1 例外登记）

全仓 grep（rg，bidcore/ + m1/ + scripts/）：

| 三方库 | bidcore 生产消费点 | 结论 |
|---|---|---|
| `mdc_office_io` / `mdcore::` / `mdc_pdf` | **仅 bid-engine src/tests**（reader.rs/mapping.rs/lib.rs/pages.rs/styles.rs）+ bid-kb 测试探针（dev 面） | 唯一接触面纪律成立——B 组结论复核确认 |
| `calamine` | bid-engine reader.rs（cell_to_string，:79-97——与引擎同版 0.36，类型镜像+字符串化）+ lib.rs 注释 | 单点消费成立 |
| `zip` | **生产面零消费**：bid-tools/bid-mcp 仅 dev-dep（bid-mcp Cargo.toml:28-31 zip="4"；bid-tools Cargo.toml:29-33 zip="4"+quick-xml="0.36"——测试解包导出件校验/夹具造件）；bid-parse Cargo.toml 无 zip（lib.rs:4 头注自认「OOXML 读面单一真源在引擎侧，本 crate 不再持有 zip/quick-xml」） | B4 收敛后纪律成立 |
| `quick-xml` | 同上——bid-tools dev-dep（export_stream.rs 测试良构校验）；bid-parse 无 | 成立 |
| docx-rs | bid-tools export.rs 写面（:82 use docx_rs） | 写面独立（非读面接触）——不违反「读面唯一接触面」纪律 |

**例外登记**：
1. **m1（研究遗留线）**：m1/src/docx.rs、xlsx.rs 直用 zip+quick-xml 自解析、无任何安全门（D-19，P3）——生产路径零引用（grep：仅 run_tests/scripts/docs 引用），风险限评测语义漂移；
2. **scripts/engine_probe**：mdc-office-io/mdc-pdf git 依赖（rev 3204342d，见 §4）——对账工具非生产面，但其 rev 漂移构成对账失效（B-17）；
3. **scripts/bid_health_check**：path 依赖 bid-parse/bid-engine/bid-domain（Cargo.toml:9-13）——跟随本地 checkout，无 rev 钉扎，跑在任意本地基线上（B-17 ②）。

**裁定：IMPLEMENTED（生产读面唯一接触面；例外 3 处全部登记）**

---

## 3. MDCORE_REV 常量错报 ❌（CONFIRMED，双源坐实）

- **静态证据**：`bid-engine/src/lib.rs:78` `pub const MDCORE_REV: &str = "aa0c859d956597992c2d93e9055e2f0beca017c3"` vs Cargo.toml:33/36/39 实钉 `f4328ebf4f936e1321a2254382d59f9ab9f94b7b`。常量头注（:74-77）自称「与 Cargo.toml git 依赖一致——parse_summary 溯源口径」——**声明与实现直接矛盾**（第二次 bump f4328ebf 时未同步常量；Cargo.toml 注释 :20 同样残留「3204342d → 本 rev」旧话术，三处口径不一致）。
- **运行时坐实**：corpus-verification.md §3.1——E-RUN 重跑 BASELINE env.mdcore_rev = `aa0c859d…`（经 parse_summary 流入产物），实钉 f4328ebf——差一个 bump 周期。
- **传播面**：mapping.rs:280-285/:369-373/:432-436 把常量写进每个引擎路径 ParsedFile.parse_summary——所有引擎解析产物 rev 溯源错报。
- **测试自引用盲区**：mapping_docx.rs:88、mapping_xlsx.rs:52/:75 断言 `contains(MDCORE_REV)`——同一常量自证，rev 漂移永不红（B-05）。
- **影响**：对账/差分回放（「升级走独立 PR 并重跑基线差分」纪律）失去可信锚；消费该常量的对账脚本对错版本。
- 文档面双重不符（B 组 E-3）：engine-bid-mapping.md §1 末行声称 rev「从 baseline/MANIFEST.json 的 BASELINE_MDCORE_REV 口径取」——代码硬编码常量且错报。

**裁定：ABSENT（溯源链不可信）——修复：常量改由 build.rs 从 Cargo.toml 注入或删除常量改读 cargo metadata；测试改断言「常量 == Cargo.toml 实钉」**

---

## 4. 对账脚本钉旧 rev 问题 ❌（行号取证）

| 脚本 | 位置 | 实钉 | 问题 |
|---|---|---|---|
| engine_probe | `scripts/engine_probe/Cargo.toml:16` `rev = "3204342d9a5ee33400aba2b17dad29696e2f0bb8"`（mdc-office-io）；`:21` 同 rev（mdc-pdf，features engine-pdfium） | **3204342d**（旧） | ①落后生产 f4328ebf **117 提交**（mdcore-audit `git log --oneline f4328ebf..origin/main | wc -l` = 117；3204342d→f4328ebf 又叠 aa0c859d 一跳）；②README/Cargo.toml:9-10 自称「与 bid-engine 同 rev——双跑差分两侧必须同引擎版本」，被自身违反；③engine_probe 是唯一曾实测 S7（解压比）/S8（zip 敌意面）/D4-D6/X3 的对账工具——其结论只对 3204342d 负责，**当前生产 rev 的引擎门行为零对账证据**（B-03/B-17） |
| baseline_harness | `scripts/baseline_harness/Cargo.toml:9` `bid-parse = { path = "../../bidcore/crates/bid-parse" }`（无引擎依赖） | path（随本地） | B1 基线冻结只走旧路径（bid_parse::parse_file）——本身不涉引擎 rev；但 baseline/MANIFEST.json 的 env.mdcore_rev 记录来自 bid-engine MDCORE_REV 常量（aa0c859d 错报，§3）——**基线账本的引擎指纹本身是错的**；且 path 依赖无 rev 钉扎，跑在任意本地 checkout 上（B-17 ②） |
| bid_health_check | Cargo.toml:9-13 path 依赖（bid-parse/bid-engine/bid-domain） | path（随本地） | 同上：无 rev 钉扎；corpus-verification 实测其 BASELINE.json 无条件写回仓内路径（覆写仓内基线） |
| engine_diff.py | scripts/engine_diff.py（双跑差分本体） | 调 engine_probe/dump 产物 | 差分两侧引擎版本一致性依赖 engine_probe 的 rev——上游已错（§4 行 1）；T17 断言只读入库旧 JSON（D-23） |

**结论**：引擎对账证据链整体过期——生产 rev 的三门行为、S7/S8 敌意面、双跑差分均无当前证据。**裁定：PARTIAL（工具在位）/对账失效（ABSENT）——修复：engine_probe rev 同步 f4328ebf + 重跑 S7/S8/双跑差分 + T17 加「从当前 HEAD 重跑」活体档。**

---

## 5. engine_pdf 回退语义与 pdfium 缺失

- **路由**：`bid-tools/src/service.rs:366-379`——pdf 引擎优先（parse_pdf_via_engine）；`EngineFailed`（无 pdfium 动态库）或 `MappingGap`（纯扫描件 any_text_layer=false）→ 回退旧路径 parse_file（poppler 链）。
- **SKIP 语义**：mdc-pdf 走 engine-pdfium 特性，pdfium 动态库 env `MDC_PDFIUM_LIB`（或编译期候选）；缺库环境测试面 SKIP（issue #9 口径，不伪装失败）。
- **运行实证**：corpus-verification.md §3.3——审计沙箱无 pdfium → **全部 PDF 走 poppler 旧路径；pdfium 通道（B5 引擎路径）在无动态库环境零覆盖**；test-quality-audit §1.1 同证（pdfium 缺失未触发红/绿差异，可能全走 poppler）。
- **回退语义的口径分裂**（B-12）：两通道行切分/页数口径/清洗（sanitize_line 仅旧路径）/乱码仲裁（仅旧路径）互不相同——同一 sha 文件在有无 pdfium 环境间 blocks/idx/locator/pages 不同；`submit_block_marks` 的 block_idx 对齐仅在同通道内稳定（环境变更=漂移面，跨会话持旧索引的 Agent 错位风险）。
- **引擎漂移相关**：mdcore f4328ebf 之后 main 新增 `--check-pdfium` 部署自检（#68，caddc3e）与 pdfium 定位序 exe 同目录候选（#210，715ceef）+ windows CI 接入 pdfium 真实验证（cfedd3b「229→231，此前零覆盖」）——**这些部署可靠性修复标书仓未消费（钉在 f4328ebf）**。

**裁定：IMPLEMENTED（回退机制本身）+ PARTIAL（B-12 口径分裂；引擎路径证据链断裂=T18a 红+B5 零覆盖）**

---

## 6. Locator / BlockID / Revision 序列化语义两仓消费点对照（抽查 3 结构）

镜像设计（bid-engine/src/types.rs 头注 :1-11）：镜像而非 re-export——引擎类型演进封闭在适配层。形状锚点声称 @ 2d014445 核对（**注意：锚点 rev 2d014445 既非 3204342d 也非 f4328ebf——又一个 rev 口径碎片**）。

| # | 结构 | bid-engine 镜像（types.rs） | mdcore 引擎侧（office-io/docx.rs、pdf/command.rs） | 对齐判定 |
|---|---|---|---|---|
| 1 | 段落 | `EngineParagraph`（types.rs:86-108）：style: Option\<String\>、heading_level: Option\<usize\>（1-9 域）、text（run 直连含 w:ins 当前生效文本）、has_numbering、fields_dropped、footnote_refs: Vec\<i64\>、revision_inserted: Vec\<String\>、revision_deleted（**恒空占位**——#17 口径 delText 不入正文计数走 warnings）、source | `ParagraphData`（docx.rs:1489-1503+）：style: Option\<String\>（:1491）、heading_level: Option\<usize\>（:1493-1502——四链解析 outlineLvl→pStyle→样式名 heading 形/中文形→样式级 outlineLvl，钳 1-9） | **字段对齐 ✓**（reader.rs:102-117 逐字段搬运；revision_inserted 从 runs 过滤、revision_deleted 显式置空并注释口径一致）；heading_level 解析链语义引擎侧单一真源，bid 侧只消费 |
| 2 | 表格 | `EngineTable`（types.rs:112-117）：rows: Vec\<Vec\<String\>>、source——**无 merges 字段**（注释「X3 定谳：扁平无合并结构，设计登记」） | `TableData`（docx.rs:1608-1614）：rows: Vec\<Vec\<String\>（**栅格展开形**——gridSpan 覆盖列/vMerge 续格置空，内容只居原点格；reader.rs:118-120 注释引引擎 cdd0a8d @ 3204342d 语义适配）、merges: Vec\<MergeRange\>（#22 P2-2 矩形对角记录）、source | **rows 对齐 ✓（栅格展开语义搬运）；merges 被镜像丢弃**——xlsx 面 EngineSheet 有 merges（types.rs:152 r1c1 1 基）而 docx 面丢弃：两格式合并区语义不对称，属设计登记（X3）而非错位；差分工具若按引擎 merges 对账会失配 |
| 3 | PDF 文本 | `EnginePdf`（types.rs:157-163）：pages: Vec\<String\>（页号 1 基升序）、any_text_layer: bool（全部页零文本→false；partial 语义由映射层消费） | `Outcome::ExtractedText { pages: Vec<(PageIndex, String)> }`（pdf/command.rs:128） | **对齐 ✓（PageIndex 被丢弃、按序转 Vec\<String\>——升序假设由引擎保证）；any_text_layer 是 bid 侧派生口径**（reader/mapping 按「任一页 ≥16 非空白」派生——B-28 判定粒度缺陷在 bid 侧派生而非引擎数据缺失） |

**结构对齐总评**：3 结构抽查字段级对齐成立、映射纪律（「只调用与字段搬运、零自研语义」issue #2 约束）被 reader.rs 实现遵守；两处已知语义不对称（docx merges 丢弃=X3 设计登记；revision_deleted 恒空=#17 口径）均有注释在案，非静默漂移。** Locator 序列化语义**：bid 侧 locator 生成在 mapping.rs（xlsx `{sheet}!行{N}`——B 组 E-6 指出文档写 `<sheet名> 行<N>` 与实现多 `!` 的小偏差；pdf 行号=块序、docx=块序），引擎侧不产 locator——定位语义单一真源在 bid-engine mapping，无跨仓双写面 ✓。**Revision 溯源语义**：parse_summary 引擎 rev 错报（§3）——结构对齐良好但溯源元数据不可信。

**裁定：IMPLEMENTED（3/3 结构字段对齐；2 处设计登记不对称）**

---

## 7. mdcore 主干 117 提交后的能力漂移面（f4328ebf..origin/main，只列能力面）

mdcore-audit `git log --oneline f4328ebf..origin/main` = **117 提交**（workspace-inventory 记 79+ 为早期快照，现值 117）；diff 触达：mang-shell/src(23) + crates/pdf(15) + mang-shell/tests(11) + office-io(8) + mdcore(7) + canvas(5) + mdcore-wasm(2) + web(4)。标书仓钉 f4328ebf 未消费的**引擎侧能力/修复**（按 issue 聚类，不深审）：

| 能力面 | mdcore 提交（issue） | 标书仓关联 |
|---|---|---|
| **DOCX 读路径健壮性** | #226：docx-rs panic 预检防御（a0aa339 裸 run 属性注入缺省 + w:w 度量归一化）+ mang-shell 打开入口 catch_unwind（6876d45）+ health-check 基线再生（73057bd「docx4j panic 4→0」） | **直接相关**：bid-engine 读链同走 docx-rs——畸形 docx 在 f4328ebf 上可能 panic（引擎侧已修，标书未消费） |
| **PDF 表单/签名保护** | #227：表单保存真增量修订 + 已签名文档 flatten 拒签 + 签名在场探针（4ac0788/def9e42/88d1238/0cdf185） | 低关联（标书无表单写面）；导出后处理链若接 pdf organize 时相关 |
| **pdfium 部署可靠性** | #210：pdfium 定位序 exe 同目录候选（715ceef）+ windows CI 接 pdfium 真实验证（cfedd3b「229→231 此前零覆盖」）+ INSTALL.md 四层定位序（4eb8891）；#68：--check-pdfium 部署自检（caddc3e）；#211 pdfium missing 可操作指引（297d39e） | **相关**：B5 引擎 PDF 通道部署面（MDC_PDFIUM_LIB）——标书侧 SKIP 语义未获这些修复 |
| **xlsx 原位编辑** | #177：单元格级读改写 + 部件透传保真（09551ad） | 无消费点（标书无改 xlsx 用例）；引擎能力账本「可用未接」族 |
| **导出层增强** | #190：页眉/页脚六槽位 + 页码域（e741dea）+ Markdown 图片行内嵌 PNG（014b53f）；#196 任务 2/3：TOC PAGEREF 域 + 标题书签（deddf1a）+ Heading 样式视觉面/rStyle（9db6887）；#207 段落样式 | **相关（读面）**：页眉页脚块流镜像已消费；TOC/书签/样式视觉为写面——标书导出链（docx-rs 自写）不消费 |
| **xlsx 合并格语义勘误** | #203：合并格注释勘误「calamine 复制左上值」→「左上单点显示」（ced06c4） | 低关联：bid xlsx 映射 cell_to_string 口径需复核（f4328ebf 上语义=左上单点） |
| **web 桥 P0** | #225：textChars 增量维护 + TSV 公式消毒 guard_csv_cell（c465a05/76501e0/0ebfc49） | 无消费点（标书不经 mdcore web 桥） |
| **编辑器/UI 线**（不相关） | #164 性能桩/逐键 O(doc) 修复、#165-#174/#182/#183/#188/#217/#219/#220/#221 wiring·keymap·undo·菜单、#170 渲染保真测试、#195 GPU 失败原生对话框 | mang-shell 面——标书仓零关联 |

**漂移评估**：117 提交中**引擎读面实质修复 2 项（#226 docx-rs panic 防御、#210/#68 pdfium 部署链）对 bid-engine 有直接价值**；其余为 mang-shell UI/写面/测试纵深，与标书消费面正交。**裁决建议**：下次 bump 优先消费 #226（读链 panic 防御）与 #210/#68（pdfium 自检——可解 B5 零覆盖的部署盲区）；bump 时必须同步修 MDCORE_REV 常量（§3）并重跑 engine_probe 对账（§4）。

**裁定：PARTIAL（钉扎纪律好=能力漂移可控；但 #226/#210 类读面修复滞留未消费 + 溯源/对账链失效使「可控」无证据）**

---

## 8. 结论汇总

| 面 | 裁定 | 缺口 |
|---|---|---|
| 三包同 rev 钉扎 | IMPLEMENTED | — |
| 唯一接触面纪律 | IMPLEMENTED | m1/engine_probe/bid_health_check 例外已登记 |
| MDCORE_REV 溯源 | **ABSENT** | B-05（aa0c859d vs f4328ebf，运行时坐实；测试自引用永不红） |
| 对账脚本族 | **ABSENT（对账失效）** | B-17（engine_probe 钉 3204342d 落后 117 提交；path 依赖无钉扎；T17 只读旧产物） |
| engine_pdf 回退 | IMPLEMENTED/PARTIAL | B-12 双通道口径分裂；pdfium 零覆盖（T18a+corpus-verification 双证） |
| 结构镜像对齐 | IMPLEMENTED | 3/3 抽查对齐；docx merges 丢弃=设计登记；types.rs 锚点 rev 2d014445 口径碎片 |
| 能力漂移 | PARTIAL | #226/#210/#68 读面修复未消费；bump 纪律被 §3/§4 破坏 |

（本文件未修改两个仓库任何文件。）
