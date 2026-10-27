# PROVENANCE — 真实语料来源与许可证登记（B1.5 建立）

> 依据 issue #2 B1.5 指令：新增语料文件必须记录**来源与许可证**。本文件覆盖
> `real-corpus/` 下全部真实二进制文件；公告级文本镜像（`sources/`、`files/*.txt`）的
> 逐件溯源已在各子目录 README 与 `fetch-report.json` 登记，不在本文件重复。

## 1. water/ — 水利行业招标文件（5 件，2026-10-27 入库）

| 文件 | 发布机关 | 来源 URL（公开附件） |
|---|---|---|
| `files/A-塔里木水库除险加固招标文件.pdf` | 新疆维吾尔自治区水利厅 | `https://slt.xinjiang.gov.cn/slt/uploadfiles/2019/4/19/1555666040089.pdf` |
| `files/A-某水利施工标段招标文件.pdf` | 新疆维吾尔自治区水利厅 | `https://slt.xinjiang.gov.cn/xjslt/c114434/202203/f47b0aad6e534a61a1c4f1d6a8f547ee/files/%E6%96%BD%E5%B7%A5%E6%A0%87%E6%AE%B5.pdf` |
| `files/A-伊犁水库质量检测招标文件.pdf` | 新疆维吾尔自治区水利厅 | `https://slt.xinjiang.gov.cn/slt/uploadfiles/2019/12/20/1576849285302.pdf` |
| `files/C-福建修订通知-扫描件.pdf` | 福建省水利厅 | `https://slt.fujian.gov.cn/xxgk/fggw/gfxwj/202404/P020240520623194182834.pdf` |
| `files/B3-北京示范文本2025.zip` | 北京市水务局 | `https://swj.beijing.gov.cn/zwgk/2024zcwj/202510/W020251010376832517349.zip` |

- 传入路径：第三方公开 GitHub 仓库 `sunShineLoveMe/ai_bidding_power_grid` 的
  `test_samples/water_tender_docs/`（其 README 登记上述政府来源 URL）；本项目按
  `artifacts/real-eval/parse/*.json` 登记的 sha256 逐一核验后入库（4 件 PDF 与评测证据
  哈希完全一致，B3 zip 为容器分发负向测试对象）。
- 直接来源校验状态：政府原 URL 未在本环境重访（原链 2019–2025 年发布，存档以第三方仓库
  转载 + sha256 锚定为准）；如需升级为「政府原站直取」，逐 URL 重访后把实测哈希补登于此。

## 2. telecom/parse-input/ — 通信行业招标文件解析输入（17 件，B1.5 起入库）

- **来源平台**：工信部通信工程建设项目招标投标管理信息平台 `txzbqy.miit.gov.cn`
  （通信行业法定公开媒介，免鉴权公开接口；下载通道与逐文件 `fileId`/平台登记 md5(keyNum)/
  实测 sha256/尺寸见 `telecom/fetch-report.json` 16 件登记）。
- **复现协议**（B1.5 新增 `scripts/telecom_refetch.py`，2026-10-27 亲跑）：
  按登记 `fileId` 经 `getFileByKey → getAccessToken → fileServer/file/download` 通道重取，
  **16/16 件 sha256 与首轮实测逐字节一致**（含 2 件「平台登记 md5 与分发内容不一致」的
  真实数据质量发现复认，见 GT-T17）；重取报告 `telecom/refetch-report.json`（确定性，
  无时间戳）。
- **parse-input 与原始件的关系**：`telecom_file_level.py` 从已哈希校验的下载件/解压件
  **原样复制**物化（15 件=原始字节）；**2 件**为 LO 旧格式转换产物
  （`A-unicom-ry-…招标文件.docx` ← `.doc`、`B-unicom-ry-投标一览表.xlsx` ← `.xls`），
  转换链留痕 `telecom/converted-manifest.json`（源/产物 sha256 双记录 + LO 版本）。
  - **转换确定性实测结论（2026-10-27）**：LO 25.2.3.2（与首轮同版本）重转换后
    **产物字节哈希漂移**（LO 输出 zip 内嵌时间戳，跨次转换必然不一致），但
    **全部内容派生字段（blocks/tables/xlsx_rows/2040 条 requirements/anchor_checks 14 项）
    与首轮逐项一致**——语义稳定、字节不稳定。file-level-report 已按本次产物哈希更新，
    首轮哈希保留于 git 历史（`file-level-report.json`@e57cac2 之前版本）。
- **许可**：平台发布的招标公告/招标文件为依法必须公开的采购文件（法定公开媒介），
  评测用途引用。

## 3. gov-procurement/ — 政府采购（cuc-028 链）

- 公告级文本（3 节点 txt + 官方镜像 HTML）溯源登记于 `gov-procurement/README.md`
  与 `chains.json`（ccgp.gov.cn 中国政府采购网 + cuc.edu.cn 校方镜像双源交叉）。
- **原招标文件本体（3 件：更正后招标文件.pdf / 采购需求.pdf / 获取采购文件登记表.docx）
  仍不可得**——ccgp.gov.cn 对本环境出口 IP 实施 WAF 阻断。B1.5 重试证据（2026-10-27
  亲测，四路全阻）：
  1. `curl` HTTP → `403 Forbidden`（`6ae0056f_PS` WAF 页）；
  2. `curl` HTTPS + 浏览器头 → `403`；
  3. 无头浏览器（真实 Chromium 指纹，agent-browser）→ `403 Forbidden`（证明为 IP 级而非
     TLS/JS 挑战级阻断）；
  4. Wayback Machine 无存档（`archived_snapshots: {}`）；cuc.edu.cn 校方镜像仅正文纯文本、
     无附件链接。
  → 维持 GT-M17-24「文件级验证待人工获取项」登记不变（国内网络环境人工下载后按
  PROVENANCE 格式补登）。

## 4. 许可声明（全部真实语料）

1. `real-corpus/` 全部输入为**依法公开的政府采购/招标投标文件及政府机关公开附件**
   （《政府采购法》《招标投标法》法定公开媒介与政府官网），本项目以**评测引用**为目的收录；
2. 不重新分发任何付费数据库内容；公告正文版权归原发布机关，本项目不做商业性再分发；
3. 每件二进制保留 sha256 锚定与来源 URL，可按登记协议重取复验；
4. 第三方转载来源（`sunShineLoveMe/ai_bidding_power_grid` test_samples）在本项目仅作
   传递路径记录，其政府来源 URL 已逐件登记。
