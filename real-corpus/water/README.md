# real-corpus/water — 水利行业真实招标文件（B1.5 入库）

- 入库时间：2026-10-27（issue #2 B1.5 语料扩充）
- 用途：**文件级真实解析基线**（B1 基线只有 fixtures 合成件 + 公告级镜像，真实 docx/pdf/xlsx 仅 24 件
  合成层——本目录补齐水利行业真实文件，使 B3 双跑差分具备真实文件基线）
- 命名约定：与 `artifacts/real-eval/`（P2 真实文件评测，GT 24 条草案的锚定语料）一致——
  `A-` 主招标文件 / `C-` 补遗（更正/修订通知）/ `B3-` 示范文本容器；
  **文件名不改**，保证 GT 草案（`ground-truth-draft.json`）与
  `artifacts/real-eval/parse/*.json`（sha256 登记）可直接对回本目录文件。

## 文件清单（5 件，sha256 与 real-eval parse 登记逐一致）

| 文件 | sha256（前16） | 页数/形态 | 来源（政府公开附件） |
|---|---|---|---|
| `files/A-塔里木水库除险加固招标文件.pdf` | `b2e91214675d7423` | 283 页·文本层（8107 行块） | 新疆维吾尔自治区水利厅 `https://slt.xinjiang.gov.cn/slt/uploadfiles/2019/4/19/1555666040089.pdf` |
| `files/A-某水利施工标段招标文件.pdf` | `6a9814b28e1a4f34` | 142 页·文本层（3525 行块，GT 主体） | 新疆维吾尔自治区水利厅 `https://slt.xinjiang.gov.cn/xjslt/c114434/202203/f47b0aad6e534a61a1c4f1d6a8f547ee/files/%E6%96%BD%E5%B7%A5%E6%A0%87%E6%AE%B5.pdf` |
| `files/A-伊犁水库质量检测招标文件.pdf` | `815dfcf7ef5316c8` | 76 页·文本层（2272 行块） | 新疆维吾尔自治区水利厅 `https://slt.xinjiang.gov.cn/slt/uploadfiles/2019/12/20/1576849285302.pdf` |
| `files/C-福建修订通知-扫描件.pdf` | `77c95e9778eabb34` | 8 页·纯扫描（OCR 100 行块） | 福建省水利厅 `https://slt.fujian.gov.cn/xxgk/fggw/gfxwj/202404/P020240520623194182834.pdf` |
| `files/B3-北京示范文本2025.zip` | `c06df5cb08d94796` | zip 容器（7 个 .doc 示范文本） | 北京市水务局 `https://swj.beijing.gov.cn/zwgk/2024zcwj/202510/W020251010376832517349.zip` |

## 与 real-eval 评测证据的关系

`artifacts/real-eval/`（P2 第一轮真实文件评测）的解析产物与指标（召回 75%、类别命中 50%、
引用支持率 100%）即基于上述 4 件 PDF 产生——本入库使该评测从「本地样本、语料不在仓」升级为
「语料在仓、哈希可复核」。`B3-北京示范文本2025.zip` 维持 zip 容器形态（解析显式拒绝=预期行为，
作为容器分发的负向测试对象；其内 `.doc` 已由 `real-samples/北京水利施工示范文本2025-LO.docx`
承载 LO 转换后的解析样本）。

## 许可与合规

全部为政府机关依法公开的招标采购文件/示范文本（《政府采购法》/《招标投标法》法定公开媒介及
政府部门官网公开附件），评测用途引用；再分发许可见根目录 `PROVENANCE.md` §许可。
