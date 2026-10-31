# 外部 Agent 纯 stdio MCP 驱动——黑盒实测报告（#80 第三批 · 四-C 原始证据）

> 测试形态：派遣无上下文子代理，只给 `docs/bid-mcp-contract.md` + 部署配置
> （二进制路径 / stdio 行协议 / BID_WS / 素材路径），严格模拟 Claude Desktop
> 里对引擎一无所知的操作者（禁读源码/测试/其他文档，禁碰工作区内部文件，
> 报错只能靠 error.what 自救）。以下为该子代理的交付报告原文（未改写）。

---

**环境**：`bid-mcp-stdio`（stdio JSON 行协议）+ `BID_WS=/tmp/external-agent-ws`；
唯一知识源 = `bid-mcp-contract.md`。共发起 **MCP 调用 45 次**（成功 41 /
报错 4，全部报错自行恢复）。

## 一、步骤日志

| # | 工具 | 关键返回 |
|---|------|---------|
| 1 | `create_bid_project`（明标，deadline 2026-12-15，external_key=ext-agent-e2e-001） | PRJ-0001，DRAFT，pv=1 |
| 2 | `import_tender_files`（A/B 两件，upload_ref=绝对路径，kind=招标文件） | TF-0002/TF-0003；307 chunks（232+75）、12 tables（7+5）、parse_failed=0；FILES_IMPORTED，pv 1→3 |
| 3 | `extract_requirements`（scope=all，include_scoring_table=true，未传 granularity） | **108 条款**：qualification 8 / veto 48 / scoring 6 / tech_param 15 / format 4 / other 27 / commercial 0；conflicts=0，missing_sequence=0；pv→4 |
| 4 | `build_outline`（全缺省：20000 字 / 每叶 8 条） | **21 sections / 15 leaves**，矩阵 108 行，**blocking=56**；MATRIX_READY，pv→6 |
| 5 | `register_evidence` ×19（kind 按语义分布 7 类，valid_until=2030） | EV-0025…EV-0044 全部 ACTIVE，pv→28 |
| 6 | `bind_evidence` ×20 次调用（共 **64 条绑定**，bid_open_date=2026-12-15） | blocking 56→55→…→**0/108**，全程 requested=bound_new，零 replay；pv→111 |
| 7 | `record_review_gate` ×3（matrix→outline→pre_export，reviewer=human:bid-manager，APPROVED） | gates_approved=[三门齐]，REV-0045/46/47，pv→114 |
| 8 | `draft_response_section` ×15（全部叶子节，generator=mock） | **15/15 DRAFTED**，合计 21,146 字；每章 citations=该章 REQ 数，uncovered_requirements 全部=0；unanchored_claims 每章 4~11（共 105）；DRAFTING，pv→176 |
| 9 | `validate_bid_document`（scope=all，rules_profile=standard） | **blocker=0**、major=0、minor=3（2×W_CHARS_OFF_TARGET + 1×W_FORMAT_BASELINE）；status=VALIDATING |
| 10 | `export_bid_document`（docx） | **EXPORTED（terminal）**；产物 103,889 bytes，15 章，precheck.blockers=0，`file` 核验合法 Word 2007+ |
| — | `check_bid_quality`（终态快照） | drafted=15/15，uncovered_req_total=0，out_of_band=2（needs_human） |

章节→REQ 映射（draft 的 source_requirements 来源）：从 `get_compliance_matrix`
108 行的 `requirement.id × draft_section_id` 反推（每叶 3~8 条，合计 108）。

## 二、摩擦清单（按痛感排序）

1. **`export` 的 `output_path` 参数契约完全没写**（最大坑）。第一次调用报
   `E_UNPROCESSABLE: output_path 缺失或非字符串`；补绝对路径又报「须为工作区
   相对路径」；第三次才成功——两轮试错全靠报错文案自救。
2. **`validate` 成功响应不返回 `project_version`，却把版本推进**。export 带
   旧值直接 `E_CONFLICT`（报错信息救了一命，但违反契约承诺）。
3. **章节 ID 从哪来——契约正文没讲清**。唯一途径是 `check_bid_quality` 的
   sections 数组（只写在末尾括号注里）；靠矩阵行 `draft_section_id` 交叉拼出
   15 个叶子。
4. **每章的 REQ 集无正式查询口**——只能从矩阵 `draft_section_id` 反推。
5. **`bid_open_date` 语义无出处**（与 create 的 bid_deadline 什么关系）。
6. **`options.granularity` 合法值未枚举**——索性不传（成功）。
7. **ID 计数器不从 1 开始且无解释**：文件从 TF-0002 起、证据从 EV-0025 起。
   `list_evidence` 一查 total=1，白困惑一轮。
8. **`register_evidence` 的 hint 文案误导**：全新登记却返回「换版场景旧记录
   已降级」——实际台账里并没有旧记录被降级。
9. **project_version 步进不透明**：单次调用版本涨 1~6 不等。
10. **状态机与契约不符**：VALIDATING 后 REVIEWING 从未出现（export 仍直接成功）。
11. **unanchored_claims 行为与契约矛盾**：mock 稿 unanchored 总计 105，draft
    的 warnings=[]、validate findings 一条不提。
12. **报错字段名与契约不一致**：契约写 `message`，实际是 `what`，无 hint。
13. **W_FORMAT_BASELINE 文案与事实打架**：提示「未识别到格式条款」，但抽取
    明明有 format 类 4 条。
14. 小噪音：export 传 `watermark:false` 仍告警「暂不支持」；BID_DEADLINE_PAST
    警告写在 E_CONFLICT 条目下。

**正面体验**：所有 4 次报错信息本身都可操作、能 self-serve 恢复；分页
limit=1000 单页取全量正常；矩阵过滤、三门门序、E_PARTIAL 语义无障碍。

## 三、终态

✅ **成功导出（EXPORTED，terminal=true）**：15 章、108 条款全覆盖、
blocking 56→0、三门齐过、blocker=0。

**建议修复优先级**：① 契约补 `output_path`；② validate 响应补
project_version；③ 契约正文明确章节/REQ 归属发现路径；④ 统一错误字段名。

---

> 本批处置：#1/#2/#3/#5/#7/#8/#12 已修（见 docs/subagent-llm-assessment.md
> §四-C 处置表）；#4/#6/#9/#10/#11/#13/#14 登记为后续项。
> 伴随产物：投标文件-示例建设集团.docx（103,889 字节）+ quality-report.json。
