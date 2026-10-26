# real-corpus/telecom — 通信行业真实语料（M1.7-TEL）

- 来源平台：**工信部通信工程建设项目招标投标管理信息平台**（txzbqy.miit.gov.cn，通信行业法定
  公开媒介）+ 公开镜像（zhiliaobiaoxun 字节存档；bidcenter 快照证据）。
- 覆盖主体：中国移动 / 中国电信 / 中国联通 / 中国铁塔。
- 生成时间：2026-10-26（第 10 轮）；脚本幂等可重跑。

## 目录与入库政策

| 路径 | 内容 | 入库 |
|---|---|---|
| `sources/` | 20 节点官方 JSON（含正文纯文本）、镜像 HTML/TXT、快照证据、mirror-manifest | ✅（约 1.5MB 文本） |
| `gt/` | GT 草案 18 条（ground-truth-draft.jsonl，四要素） | ✅ |
| `fetch-report.json` | 16 文件下载报告：来源 URL、平台登记 md5（keyNum）、实测 md5/sha256、尺寸、状态（14 OK / 2 MD5_MISMATCH 双记录） | ✅ |
| `chain-report.json` | 四节点链 46 断言结果 | ✅ |
| `file-level-report.json` | 文件级解析报告（Rust 亲跑 17/17 + 2040 条款 + 14/14 跨向锚点） | ✅（1MB） |
| `converted-manifest.json` | LO 旧格式转换清单（源/产物 sha256 双留痕） | ✅ |
| `files/`（86MB） | 原始下载二进制（招标文件 zip/doc/docx/xlsx/pdf/公告附件） | ❌ gitignore——按 fetch-report 哈希可重取复验 |
| `extracted/`（83MB） | zip 解压产物 | ❌ gitignore——`telecom_prepare_files.py` 可重生成 |
| `parse-input/`（4.6MB） | A-/B-/C- 命名解析输入物化 | ❌ gitignore——`telecom_file_level.py` 可重生成 |
| `converted/` | LO 转换产物（.doc→.docx 等） | ❌ gitignore——`telecom_prepare_files.py` 可重生成 |

## 二进制重取协议（复现文件级测试）

```bash
python3 scripts/telecom_fetch.py          # txzbqy 免鉴权五件套 → files/（md5 与平台登记值逐字节校验）
python3 scripts/telecom_prepare_files.py  # 解压 + LO 转换（converted-manifest 留痕）
python3 scripts/telecom_file_level.py     # 物化 parse-input + cargo 真跑 + 跨向锚点核验
python3 scripts/telecom_chain.py          # 四节点链 46 断言
python3 scripts/telecom_gt_draft.py       # GT 草案 18 条定位预核
```

- 完整性判据：重取后逐文件 `md5 == fetch-report.files[].md5_platform`（2 个 MISMATCH 文件按
  `sha256` 与 `size` 双记录比对——平台登记指纹与分发内容不一致是本语料的真实数据质量发现，
  见 GT-T17）；若平台后续替换文件导致哈希漂移，以新哈希重登记并在报告注记，不静默沿用旧值。
- 门端列表（bidBulletinListDoor/bidPublicityListDoor）随时间滚动为最新 10 条：重跑会取到
  不同节点集——链节点以 `sources/` 已入库 JSON 为准（uuid 稳定），报告不因门端滚动而失效。

## 链构成（HX-BFZB-2026035 四节点）

招标公告 08-12（字节存档）→ 招标失败公告 09-08（**SNAPSHOT_ONLY**——bidcenter JS 站无法字节
抓取，关键句来自搜索快照并诚实登记；字节级原件须人工从联通采购与招标网/txzbqy/cebpubservice
法定媒介获取）→ 第二次招标公告 09-18（字节存档）→ 中标候选人公示 10-26（txzbqy 官方 JSON）。

副链：浙江移动 ZJYD20260500552 变更公告（frequency=1，官方 JSON + PDF 双落）。
