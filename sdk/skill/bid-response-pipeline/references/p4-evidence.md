# P4 证据细则（register_evidence / bind_evidence / list_evidence）

> 契约依据：`docs/bid-mcp-contract.md` §11-A / §11-B / §4-A。外部 Agent 纯
> MCP 驱动的证据三板斧：**登记（register）→ 绑定（bind）→ 盘点（list）**。
> `search_evidence`（知识库双通道检索，契约 §5）当前未接线（M2-b）——企业
> 资料库就绪后接入；检索模式选择（资格类 exact 优先等）届时生效，本文件
> 末节预留。

## 登记面（register_evidence）

```json
{
  "project_id": "…",
  "kind": "qualification",
  "title": "电子与智能化工程专业承包二级资质证书",
  "source_file": "files/资质证书.pdf",
  "source_sha256": "<必填——无文件身份不入账>",
  "subject_name": "投标人",
  "scope": "电子与智能化工程",
  "valid_until": "2028-06-30",
  "fingerprint": "sha256:<缺省=source_sha256>",
  "locator": { "kind": "docx_paragraph", "…": "指纹字段" },
  "project_version": 12
}
```

- `kind` 七种：`qualification | performance | personnel | certificate |
  product_doc | solution_module | contract`（或对应中文展示名）；
- 语义位查重：同 `kind+title+subject+scope` 的最新非 SUPERSEDED 记录——
  同指纹 → 幂等重放回原 EV-id；异指纹 → 旧记录单向降级 SUPERSEDED + 签发
  新 EV（证书换版就这么处理）；
- 切块即入 EV 台账：`import_tender_files` 的切块已自动注册 EV——**后续
  register 签发的 EV-id 从已有计数后继，不是从 EV-0001 起**；
- 返回 `{ ev_id, deduplicated, status, hint }`——`deduplicated: true` 即重放。

## 绑定面（bind_evidence）

```json
{
  "project_id": "…",
  "ev_id": "EV-0007",
  "requirement_ids": ["REQ-0012", "REQ-0013"],
  "bid_open_date": "2026-11-30",
  "project_version": 14
}
```

- `bid_open_date` 语义（易错点）：证据有效性的**裁决基准日 = 开标日**
  （评标时刻的有效性），不是 create 时的 bid_deadline（递交截止日）——两者
  通常相近但语义不同；无独立开标日真源时用招标文件载明的开标日期；
- EXPIRED / SUPERSEDED 证据禁绑（开标日视角即时裁决 → 逐件
  E_UNPROCESSABLE）——**不要试图绑过期待换证的证据**，先走补料流程；
- 绑后矩阵即时重算：返回 `matrix.blocking` 应清零（资格/否决/技术参数行
  全部有证据时）；
- 重复绑定幂等重放；部分失败 → E_PARTIAL（item_errors 逐件定位 + data 携
  带成功面）。

## 盘点面（list_evidence，只读）

```json
{ "project_id": "…", "filters": { "kind": "performance", "status": "ACTIVE" },
  "cursor": "<opaque>", "limit": 100 }
```

状态四态：`ACTIVE | EXPIRING | EXPIRED | SUPERSEDED`。大结果统一游标信封
（`next_cursor: null` 即取完；行序 = id 字典序）。游标与过滤器绑定——换过滤
器必须换游标，跨工具复用 → E_UNPROCESSABLE。`limit` 合法域 1..=1000、缺省
100，越界显式拒不静默 clamp。

## 有效期核对法（Agent 逐条核对模板）

对每条资格/业绩证据：

1. 读 `valid_until`，与 `bid_open_date` 比较；
2. `valid_until < 开标日` → EXPIRED：**禁绑**，进补料清单；
3. 临界（开标日前 90 天内到期）→ EXPIRING：可绑，但向用户提示换证风险；
4. 无 `valid_until`（如业绩合同）→ 不参与有效期裁决，正常绑。

## 补料清单模板（缺失项交用户）

> **证据缺口清单（P4 产出）**
>
> | REQ | 条款摘要 | 需要的证据 | 现状 |
> |---|---|---|---|
> | REQ-0015 | 电子与智能化二级资质 | 资质证书扫描件（sha256） | 待用户上传 |
> | REQ-0031 | 近三年类似业绩≥2 项 | 中标通知书+合同关键页 | 缺 1 项 |
>
> 请补齐后告诉我文件路径，我会登记并只补跑受影响的绑定行。

用户补料 → register（新 EV）→ bind（只绑受影响 REQ）→ 矩阵复blocking=0。
不要整单重跑（恢复协议：缺料只补受影响行）。

## 预留：search_evidence 双通道（M2-b 接线后）

- 模式三选：`hybrid`（缺省，语义+精确交替合并）/ `semantic` / `exact`；
- **资格类查询默认 exact 优先**（证书号/金额/日期/型号——精确命中即够，
  语义召回反而引入噪声）；
- `valid_until < 开标日` 的证据自动降权并附 warning（EVIDENCE_EXPIRING/
  EXPIRED）；
- 排序带确定性 tie-breaker（score, evidence_id）——同查询同结果，可重放；
- hits 是大结果面：走统一游标信封（与 list_evidence 同口径），不自带第二套
  分页。
