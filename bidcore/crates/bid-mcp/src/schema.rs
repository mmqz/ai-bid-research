//! 传输面数据形状（MCP 附近的 serde 契约）。
//!
//! 形状刻意贴 MCP 的 `tools/call` 语义（`{tool, args}` 入，`{ok, data|error}` 出——
//! 契约 §0 通用响应包装），但**不与任何传输实现绑定**——stdio 与未来
//! HTTP / wasm 共用同一组类型。

use serde_json::{json, Value};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::ToolError;

/// 一次工具调用请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    #[serde(default)]
    pub args: Value,
}

/// 工具返回体（成功）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub tool: String,
    pub result: Value,
}

/// 工具返回体（成功或失败二选一）。
///
/// 线格式（契约 §0 通用响应包装，B-1 起对齐）：
/// `{"ok":true, "tool":.., "data":..}` / `{"ok":false, "tool":.., "error":{code,..}}`。
/// 手写 serde：`ok` 必须是**布尔**（serde 内部标签只能出字符串——M0 期的
/// `#[serde(tag="ok")]` 会发出 `"ok":"true"`，与契约及 dispatch_json 兜底
/// 路径的布尔口径不一致，B-1 修正）；成功载荷键名对齐契约的 `data`。
#[derive(Debug, Clone)]
pub enum ToolReply {
    Ok(ToolOutcome),
    Err { tool: String, error: ToolError },
}

impl ToolReply {
    #[must_use]
    pub fn ok(tool: &str, result: Value) -> Self {
        Self::Ok(ToolOutcome {
            tool: tool.to_string(),
            result,
        })
    }

    #[must_use]
    pub fn error(tool: &str, error: ToolError) -> Self {
        Self::Err {
            tool: tool.to_string(),
            error,
        }
    }
}

impl Serialize for ToolReply {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(3))?;
        match self {
            Self::Ok(out) => {
                map.serialize_entry("ok", &true)?;
                map.serialize_entry("tool", &out.tool)?;
                map.serialize_entry("data", &out.result)?;
            }
            Self::Err { tool, error } => {
                map.serialize_entry("ok", &false)?;
                map.serialize_entry("tool", tool)?;
                map.serialize_entry("error", error)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ToolReply {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            ok: bool,
            tool: String,
            data: Option<Value>,
            error: Option<ToolError>,
        }
        let raw = Raw::deserialize(deserializer)?;
        match (raw.ok, raw.data, raw.error) {
            (true, Some(result), _) => Ok(Self::Ok(ToolOutcome {
                tool: raw.tool,
                result,
            })),
            (true, None, _) => Err(serde::de::Error::custom("ok=true 但缺 data")),
            (false, _, Some(error)) => Ok(Self::Err {
                tool: raw.tool,
                error,
            }),
            (false, _, None) => Err(serde::de::Error::custom("ok=false 但缺 error")),
        }
    }
}

/// `tools/list` 应答体的静态描述。
///
/// #137（MCP 协议层）起每条补 `inputSchema`（合法 JSON Schema——官方
/// SDK 的 Tool 模型必填位），参数面**以 tools.rs 实现为准**逐位核对
/// （required = `req_str` 强制位；optional = `opt_*` 位；enum/值域
/// 照抄实现里的显式拒绝清单——schema 与实现的漂移由
/// tests/mcp_protocol.rs 的 `required ⊆ properties` 一致性断言兜底）。
#[must_use]
pub fn tools_schema() -> Value {
    let entries: Vec<Value> = crate::TOOL_NAMES
        .iter()
        .map(|name| {
            serde_json::json!({
                "name": name,
                "description": describe(name),
                "inputSchema": input_schema(name),
                "implemented": is_implemented(name),
            })
        })
        .collect();
    serde_json::json!({ "tools": entries })
}

/// 各工具入参的 JSON Schema（`type/properties/required/enum/minimum/maximum/
/// items/minItems` 键面——draft-07 广谱子集，不依赖 2020-12 专有键）。
#[must_use]
fn input_schema(name: &str) -> Value {
    // 高频参数的公共形状（match 各臂互斥——按值 move 各占一条执行路径，合法）
    let pid = json!({"type": "string", "description": "项目 ID（建项目时的 project_id）"});
    let pv = json!({"type": "integer", "description": "乐观并发版本号（上次响应的 project_version 原样回传；缺省跳过检查）"});
    match name {
        "create_bid_project" => json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "项目名"},
                "bid_deadline": {"type": "string", "description": "投标截止日（YYYY-MM-DD；已过 → 警告不阻断）"},
                "mode": {"type": "string", "enum": ["明标", "暗标"]},
                "external_key": {"type": "string", "description": "外部幂等键（同 key 重放返回已有 project_id）"},
                "tender_notice_ref": {"type": "string", "description": "招标公告编号（可选）"}
            },
            "required": ["name", "bid_deadline", "mode", "external_key"]
        }),
        "import_tender_files" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "files": {
                    "type": "array", "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "upload_ref": {"type": "string", "description": "文件路径（引擎可读位）"},
                            "filename": {"type": "string"},
                            "kind": {"type": "string", "enum": ["招标文件", "附件", "图纸", "补遗", "答疑", "评分办法", "格式模板"], "description": "缺省归「附件」"},
                            "expected_pages": {"type": "integer", "description": "页数对账期望值（可选）"}
                        },
                        "required": ["upload_ref", "filename"]
                    }
                },
                "project_version": pv
            },
            "required": ["project_id", "files"]
        }),
        "extract_requirements" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "scope": {"type": "string", "enum": ["all"], "description": "文件子集过滤属 M2 扩展（当前仅 all）"},
                "options": {"type": "object", "description": "granularity 保留位（未实现，传入被忽略）"},
                "project_version": pv
            },
            "required": ["project_id"]
        }),
        "get_compliance_matrix" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "filters": {
                    "type": "object",
                    "properties": {
                        "category": {"type": "string", "description": "契约键（qualification|veto|scoring|tech_param|commercial|format|other）或中文类别名"},
                        "status": {"type": "string", "enum": ["NOT_RESPONDED", "RESPONDED", "PARTIAL", "NOT_APPLICABLE"]}
                    }
                },
                "cursor": {"type": "string", "description": "opaque 游标（上次响应的 next_cursor；与 filters 绑定）"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "description": "缺省 100；越界显式拒"},
                "page": {"type": "integer", "minimum": 1, "description": "兼容旧调用（映射 offset=(page-1)*limit；cursor 优先）"}
            },
            "required": ["project_id"]
        }),
        "list_evidence" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "filters": {
                    "type": "object",
                    "properties": {
                        "kind": {"type": "string", "enum": ["qualification", "performance", "personnel", "certificate", "product_doc", "solution_module", "contract"], "description": "或对应中文展示名"},
                        "status": {"type": "string", "enum": ["ACTIVE", "EXPIRING", "EXPIRED", "SUPERSEDED"]}
                    }
                },
                "cursor": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
            },
            "required": ["project_id"]
        }),
        "list_tasks" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "filters": {
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string", "description": "执行人精确匹配"},
                        "status": {"type": "string", "enum": ["ASSIGNED", "IN_PROGRESS", "DONE"]}
                    }
                },
                "cursor": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
            },
            "required": ["project_id"]
        }),
        "search_evidence" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "query": {"type": "string"},
                "mode": {"type": "string", "enum": ["hybrid", "semantic", "exact"]},
                "exact": {"type": "object", "properties": {"field": {"type": "string", "enum": ["model", "amount", "date", "clause_no", "cert"]}, "value": {}}},
                "filters": {"type": "object"},
                "top_k": {"type": "integer", "minimum": 1},
                "threshold": {"type": "number"}
            },
            "required": ["project_id", "query"]
        }),
        "draft_response_section" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "section_id": {"type": "string", "description": "目标章节（SEC-*——建纲后起草）"},
                "instructions": {"type": "string", "description": "起草指令（进幂等键）"},
                "source_requirements": {"type": "array", "items": {"type": "string"}, "description": "REQ-* 清单（缺省 = 章节绑定集）"},
                "evidence_ids": {"type": "array", "items": {"type": "string"}, "description": "EV-* 清单（缺省 = 矩阵绑定集）"},
                "generator": {"type": "string", "enum": ["auto", "mock", "real", "external"], "description": "auto=有 key 走真实 LLM 无 key 降级 mock；external 需配 content"},
                "content": {"type": "string", "description": "外部手稿全文（在场即 external 语义，与 generator=mock/real 互斥）"},
                "project_version": pv
            },
            "required": ["project_id", "section_id"]
        }),
        "validate_bid_document" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "scope": {"type": "string", "enum": ["all", "sections"]},
                "section_ids": {"type": "array", "items": {"type": "string"}, "description": "scope=sections 时必填"},
                "rules_profile": {"type": "string", "enum": ["standard", "dark_bid"]},
                "project_version": pv
            },
            "required": ["project_id"]
        }),
        "export_bid_document" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "format": {"type": "string", "enum": ["docx", "pdf"]},
                "output_path": {"type": "string", "description": "工作区相对路径（不得绝对路径/含 ..）"},
                "profile": {
                    "type": "object",
                    "properties": {
                        "dark_bid": {"type": "boolean"},
                        "bidder_name": {"type": "string"},
                        "project_name": {"type": "string", "description": "招标官方项目名（#117）"},
                        "bid_amount": {"type": "number", "description": "总报价（元）——报价明细表（#117）"},
                        "price_breakdown": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "item": {"type": "string"},
                                    "unit": {"type": "string"},
                                    "quantity": {"type": "number"},
                                    "unit_price": {"type": "number"},
                                    "note": {"type": "string"}
                                },
                                "required": ["item", "unit", "quantity", "unit_price"]
                            }
                        },
                        "duration_days": {"type": "integer"},
                        "validity_days": {"type": "integer"},
                        "deposit_amount": {"type": "number"},
                        "legal_rep_gender": {"type": "string"}
                    }
                },
                "options": {
                    "type": "object",
                    "properties": {
                        "tracked_changes": {"type": "boolean", "description": "暂不支持（告警）"},
                        "watermark": {"type": "boolean", "description": "暂不支持（告警）"}
                    }
                },
                "project_version": pv
            },
            "required": ["project_id", "format", "output_path"]
        }),
        "get_bid_project_state" => json!({
            "type": "object",
            "properties": {
                "project_status": {"type": "string", "description": "调试/模拟面：显式带入状态（合法值见 bid-domain ProjectStatus）；缺省 = 服务态真实状态"}
            }
        }),
        "save_version" => json!({
            "type": "object",
            "properties": {
                "file_path": {"type": "string", "description": "绝对路径（.md/.txt/.markdown）"},
                "message": {"type": "string", "description": "版本说明（必填非空——写明完成了什么）"}
            },
            "required": ["file_path", "message"]
        }),
        "check_bid_quality" => json!({
            "type": "object",
            "properties": {
                "project_id": pid
            },
            "required": ["project_id"]
        }),
        "build_outline" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "target_total_chars": {"type": "integer", "minimum": 1000, "maximum": 5000000, "description": "缺省 20000"},
                "max_reqs_per_leaf": {"type": "integer", "minimum": 1, "maximum": 50, "description": "缺省 8"},
                "project_version": pv
            },
            "required": ["project_id"]
        }),
        "register_evidence" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "kind": {"type": "string", "enum": ["qualification", "performance", "personnel", "certificate", "product_doc", "solution_module", "contract"], "description": "或对应中文展示名"},
                "title": {"type": "string"},
                "source_file": {"type": "string"},
                "source_sha256": {"type": "string", "description": "必填——无文件身份不入账"},
                "subject_name": {"type": "string"},
                "scope": {"type": "string"},
                "valid_from": {"type": "string", "description": "ISO 日期"},
                "valid_until": {"type": "string", "description": "ISO 日期"},
                "fingerprint": {"type": "string", "description": "算法前缀:值——缺省 sha256:<source_sha256>"},
                "locator": {
                    "type": "object",
                    "properties": {
                        "kind": {"type": "string", "enum": ["docx_paragraph", "pdf_page_text", "markdown_block"]},
                        "paragraph_index": {"type": "integer"},
                        "page": {"type": "integer"},
                        "block_id": {"type": "string"},
                        "text_fingerprint": {"type": "string"}
                    },
                    "required": ["kind"]
                },
                "client": {"type": "string", "description": "业绩类：发包人（#117）"},
                "amount_wan": {"type": "number", "description": "业绩类：合同金额（万元）"},
                "completed_on": {"type": "string", "description": "业绩类：完成日期"},
                "project_version": pv
            },
            "required": ["project_id", "kind", "title", "source_file", "source_sha256", "subject_name"]
        }),
        "bind_evidence" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "ev_id": {"type": "string"},
                "requirement_ids": {"type": "array", "minItems": 1, "items": {"type": "string"}, "description": "REQ-* 批量绑定清单"},
                "bid_open_date": {"type": "string", "description": "开标日（YYYY-MM-DD）——证据有效性裁决基准日"},
                "project_version": pv
            },
            "required": ["project_id", "ev_id", "requirement_ids", "bid_open_date"]
        }),
        "record_review_gate" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "gate": {"type": "string", "enum": ["matrix_approval", "outline_approval", "pre_export_confirm", "addendum_waiver"]},
                "decision": {"type": "string", "enum": ["APPROVED", "REJECTED"]},
                "reviewer": {"type": "string", "description": "须 human: 前缀（AI 操作者不得自审自过）"},
                "findings": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "severity": {"type": "string", "enum": ["BLOCKER", "WARNING", "INFO"]},
                            "code": {"type": "string"},
                            "message": {"type": "string"},
                            "section_id": {"type": "string"}
                        },
                        "required": ["severity", "code", "message"]
                    }
                },
                "project_version": pv
            },
            "required": ["project_id", "gate", "decision", "reviewer"]
        }),
        "score_price" => json!({
            "type": "object",
            "description": "纯计算器（无项目路由/状态机）——bid_price 缺省 = 纯推荐模式",
            "properties": {
                "method": {"description": "benchmark|lowest|average 或完整对象形态（如 {\"BENCHMARK_PRICE\":{\"coefficient\":0.97}}）", "type": ["string", "object"]},
                "coefficient": {"type": "number", "description": "benchmark 缺省 1.0 / average 缺省 0.95"},
                "max_score": {"type": "number", "description": "缺省 30"},
                "budget": {"type": "number", "description": "控制价（benchmark 必需）"},
                "deviation": {"description": "ratio|price(需 deduct_high/deduct_low)|rate(需 deduct) 或完整对象", "type": ["string", "object"]},
                "deduct_high": {"type": "number"},
                "deduct_low": {"type": "number"},
                "deduct": {"type": "number"},
                "bid_price": {"type": "number", "description": "己方报价（在场即试算）"},
                "others": {"type": "array", "items": {"type": "number"}, "description": "竞争报价清单"}
            }
        }),
        "score_section" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "section_id": {"type": "string", "description": "须为已起草的叶章节（结构节点/未起草 → 拒）"},
                "project_version": pv
            },
            "required": ["project_id", "section_id"]
        }),
        "score_document" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "project_version": pv,
                "bid_price": {"type": "number", "description": "缺省 = 不计价（price 出 null 而非 0）"},
                "others": {"type": "array", "items": {"type": "number"}},
                "method": {"type": ["string", "object"], "description": "价格口径（仅 bid_price 在场时消费）"},
                "coefficient": {"type": "number"},
                "max_score": {"type": "number"},
                "budget": {"type": "number"},
                "deviation": {"type": ["string", "object"]},
                "deduct_high": {"type": "number"},
                "deduct_low": {"type": "number"},
                "deduct": {"type": "number"}
            },
            "required": ["project_id"]
        }),
        _ => json!({"type": "object"}),
    }
}

fn describe(name: &str) -> &'static str {
    match name {
        "create_bid_project" => "建项目（external_key 幂等）",
        "import_tender_files" => "导入招标文件 + 完整性对账",
        "extract_requirements" => "六类条款抽取 + 覆盖率对账",
        "get_compliance_matrix" => "只读：响应矩阵与资格三态（S-4 游标分页）",
        "list_evidence" => "只读：证据台账列表（kind/status 过滤 + 游标分页）",
        "list_tasks" => "只读：任务分工列表（owner/status 过滤 + 游标分页）",
        "search_evidence" => "双通道证据检索（M2-b 落地时必须走 cursor.rs 游标模块）——未接线：调用返回 UNIMPLEMENTED_INTERNAL",
        "draft_response_section" => "单章起草（引用回查后写内核）",
        "validate_bid_document" => "双层校验（确定性规则 + 模型复核）",
        "export_bid_document" => "导出（含暗标模式；#117 表格接线——profile.bid_amount/price_breakdown 报价决策，六表自动注入/附表）",
        "get_bid_project_state" => "只读：项目状态与 allowed_next_tools",
        "save_version" => "AI 主动存版本快照（mang-snap 同源）",
        "check_bid_quality" => "只读：生成质量覆盖度检查 + 报告落盘（#68 阶段 2）",
        "build_outline" => "建纲 + 矩阵首建（G1——#80 第三批补齐；REQUIREMENTS_EXTRACTED 态）",
        "register_evidence" => "证据登记（G2——语义位查重/换版；MATRIX_READY+；业绩类可带 client/amount_wan/completed_on 业务面）",
        "bind_evidence" => "证据批量绑定 + 矩阵重算（G2——EXPIRED 禁绑；MATRIX_READY+）",
        "record_review_gate" => "三门记审（G3——matrix/outline/pre_export；reviewer 须 human: 前缀）",
        "score_price" => "价格分试算 + 最优报价反推（#100 任务 3——纯计算器，B1 五方法只调用不改）",
        "score_section" => "章节自评（#100 任务 3——四维规则判定；大纲联结补 section 映射）",
        "score_document" => "整本自评（#100 任务 3——技术/商务/价格三段 + by_item 逐项；没报价 → price null）",
        _ => "unknown",
    }
}

fn is_implemented(name: &str) -> bool {
    matches!(
        name,
        "get_bid_project_state"
            | "create_bid_project"
            | "import_tender_files"
            | "extract_requirements"
            | "get_compliance_matrix"
            | "list_evidence"
            | "list_tasks"
            | "draft_response_section"
            | "export_bid_document"
            | "validate_bid_document"
            | "check_bid_quality"
            | "build_outline"
            | "register_evidence"
            | "bind_evidence"
            | "record_review_gate"
            | "score_price"
            | "score_section"
            | "score_document"
    )
}
