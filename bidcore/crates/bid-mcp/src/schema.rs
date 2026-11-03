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
        // ---- #219 全块覆盖三工具 ----
        "get_document_chunks" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "file_id": {"type": "string", "description": "文件 ID（import_tender_files 回执 per_file[].file_id）"},
                "from": {"type": "integer", "description": "起始块 idx（缺省 0）"},
                "size": {"type": "integer", "description": "窗口大小 1..=200（缺省 50）——一次取全量会吃爆上下文"}
            },
            "required": ["project_id", "file_id"]
        }),
        "submit_block_marks" => json!({
            "type": "object",
            "properties": {
                "project_id": pid,
                "file_id": {"type": "string"},
                "marks": {
                    "type": "array",
                    "description": "区间标记：每块必须恰好被一个区间覆盖；区间不得重叠",
                    "items": {
                        "type": "object",
                        "properties": {
                            "from": {"type": "integer", "description": "起始块 idx（含）"},
                            "to": {"type": "integer", "description": "结束块 idx（含）"},
                            "verdict": {"type": "string", "enum": ["requirement", "noise", "rule", "review_table"],
                                        "description": "requirement=需响应；noise=无关；rule=招标人规则说明（我方遵守但不写响应）；review_table=审查表"},
                            "reason": {"type": "string", "description": "判断理由（可审计）"}
                        },
                        "required": ["from", "to", "verdict"]
                    }
                },
                "requirements": {
                    "type": "array",
                    "description": "从 requirement 区间提取的关键点（block_idx 必须落在 requirement 区间内）",
                    "items": {
                        "type": "object",
                        "properties": {
                            "block_idx": {"type": "integer"},
                            "statement": {"type": "string", "description": "关键点陈述（不得为空）"},
                            "category": {"type": "string", "enum": ["资格条件", "否决项", "评分项", "技术参数", "商务要求", "格式要求", "其他"],
                                         "description": "分类由 Agent 判断（引擎只校验枚举）"},
                            "mandatory": {"type": "boolean", "description": "漏响应会废标 → true"}
                        },
                        "required": ["block_idx", "statement"]
                    }
                },
                "project_version": pv
            },
            "required": ["project_id", "file_id", "marks"]
        }),
        "get_coverage_report" => json!({
            "type": "object",
            "properties": {
                "project_id": pid
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
                "source_file_id": {"type": "string", "description": "TF-*——已导入文件（import_tender_files 回执 per_file[].file_id）；不存在/解析失败 → 拒（#185 来源验证）"},
                "subject_name": {"type": "string"},
                "scope": {"type": "string"},
                "valid_from": {"type": "string", "description": "ISO 日期"},
                "valid_until": {"type": "string", "description": "ISO 日期"},
                "locator": {
                    "type": "object",
                    "description": "可选——缺省为文件级锚（verification_tier=FILE_EXISTS，无正文摘录）；传入且对账通过则 CONTENT_MATCHES：正文摘录 excerpt 由服务端从解析产物逐字截取随回执/台账返回（#217）",
                    "properties": {
                        "kind": {"type": "string", "enum": ["docx_paragraph", "pdf_page_text"]},
                        "paragraph_index": {"type": "integer"},
                        "page": {"type": "integer"},
                        "text_fingerprint": {"type": "string"}
                    },
                    "required": ["kind"]
                },
                "client": {"type": "string", "description": "业绩类：发包人（#117）"},
                "amount_wan": {"type": "number", "description": "业绩类：合同金额（万元）"},
                "completed_on": {"type": "string", "description": "业绩类：完成日期"},
                "fields": {
                    "type": "object",
                    "description": "结构化字段（#147——search_evidence exact 通道的精确查询面：cert 查 cert_name/cert_no、amount 查 amount_wan、date 查 signed_at/finished_at）。全部可选；非法日期/负金额/年限 > 100 → E_UNPROCESSABLE",
                    "properties": {
                        "project_name": {"type": "string", "description": "合同/业绩：项目名称"},
                        "client": {"type": "string", "description": "合同/业绩：发包人/建设单位"},
                        "amount_wan": {"type": "number", "minimum": 0, "description": "合同/业绩：合同金额（万元，>= 0）"},
                        "signed_at": {"type": "string", "description": "合同/业绩：签订日期（YYYY-MM-DD）"},
                        "finished_at": {"type": "string", "description": "合同/业绩：完成日期（YYYY-MM-DD）"},
                        "cert_name": {"type": "string", "description": "证书/资质：证书名称"},
                        "cert_no": {"type": "string", "description": "证书/资质：证书编号"},
                        "issuer": {"type": "string", "description": "证书/资质：发证机关"},
                        "person_name": {"type": "string", "description": "人员：姓名"},
                        "person_title": {"type": "string", "description": "人员：职称"},
                        "years": {"type": "integer", "minimum": 0, "maximum": 100, "description": "人员：从业年限（<= 100）"}
                    }
                },
                "project_version": pv
            },
            "required": ["project_id", "kind", "title", "source_file_id", "subject_name"]
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
                "receipt": {"type": "string", "description": "#182 宿主签发的审批回执（v1.<actor_id>.human.<scope>.<issued_at>.<hmac>）——人工确认由宿主签发 receipt 提供，Agent 不得伪造；reviewer 参数已废除"},
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
            "required": ["project_id", "gate", "decision", "receipt"]
        }),
        "record_addendum_waiver" => json!({
            "type": "object",
            "description": "招标包输入件不就绪的逐件人工确认（#184 补遗分级阻塞 + 地图 P0-1 扩面到主件/附件）：任一输入件不处 READY（FAILED/OCR_PENDING）时 draft_response_section 被拒；人工继续必须逐件确认到具体 file_id（非全局布尔）。确认即审计面（file_id/reason/at/principal + actor）；放行后起草标 RESTRICTED（受限起草，不静默当完整输入）",
            "properties": {
                "project_id": pid,
                "file_id": {"type": "string", "description": "TF-* 输入件 id（须真实存在——import 响应 addendum_readiness.not_ready[].file_id 或任何不就绪件；receipt 签域绑定本件）"},
                "reason": {"type": "string", "description": "人工理由（非空白——不可审计的确认不成确认）"},
                "receipt": {"type": "string", "description": "#182 宿主签发的审批回执（签域 addendum_waiver:<file_id>，件级绑定）——principal 参数已废除，Agent 不得伪造"},
                "project_version": pv
            },
            "required": ["project_id", "file_id", "reason", "receipt"]
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
        "list_evidence" => "只读：证据台账列表（kind/status 过滤 + 游标分页；行含 verification_tier——#217 四态可见）",
        "list_tasks" => "只读：任务分工列表（owner/status 过滤 + 游标分页）",
        "search_evidence" => "双通道证据检索（#145 朴素实现 + #217 正文摘录面：semantic 通道检索 title/scope/subject/client/excerpt——正文含词但标题不含也能命中；exact 五字段整token/数值/日期；valid_at 过期降权×0.5；M2-b 落地时换真向量库 + cursor 游标信封）",
        "draft_response_section" => "单章起草（引用回查后写内核；输入完整性门禁——主件/补遗/附件不 READY 拒，人工确认放行走受限起草 RESTRICTED）",
        "validate_bid_document" => "双层校验（确定性规则 + 模型复核）",
        "export_bid_document" => "导出（含暗标模式；#117 表格接线——profile.bid_amount/price_breakdown 报价决策，六表自动注入/附表）",
        "get_bid_project_state" => "只读：项目状态与 allowed_next_tools",
        "save_version" => "AI 主动存版本快照（mang-snap 同源）",
        "check_bid_quality" => "只读：生成质量覆盖度检查 + 报告落盘（#68 阶段 2）",
        "build_outline" => "建纲 + 矩阵首建（G1——#80 第三批补齐；REQUIREMENTS_EXTRACTED 态）",
        "register_evidence" => "证据登记（G2——语义位查重/换版；MATRIX_READY+；#185 来源验证：source_file_id 须指向已导入文件，sha256/指纹由服务端生成；带 locator 且对账通过 → CONTENT_MATCHES + 正文摘录入账，缺省文件级锚 → FILE_EXISTS（#217 四态）；业绩类可带 client/amount_wan/completed_on 业务面；结构化字段走可选 fields 对象）",
        "bind_evidence" => "证据批量绑定 + 矩阵重算（G2——EXPIRED 禁绑；MATRIX_READY+）",
        "record_review_gate" => "三门记审（G3——matrix/outline/pre_export；#182 receipt 验签——人工确认由宿主签发 receipt 提供）",
        "record_addendum_waiver" => "补遗逐件人工确认（#184——分级阻塞的免责面；#182 receipt 验签（签域绑定 file_id）；FILES_IMPORTED+）",
        "score_price" => "价格分试算 + 最优报价反推（#100 任务 3——纯计算器，B1 五方法只调用不改）",
        "score_section" => "章节自评（#100 任务 3——四维规则判定；大纲联结补 section 映射）",
        "score_document" => "整本自评（#100 任务 3——技术/商务/价格三段 + by_item 逐项；没报价 → price null）",
        "get_document_chunks" => "只读：按文件取块窗口（#219 全块覆盖——block_idx + locator + text + verdict 回显；size ≤ 200 分页取）",
        "submit_block_marks" => "提交区间标记 + requirements（#219——verdict: requirement|noise|rule|review_table；校验区间合法/不越界/不重叠/requirement 落在 requirement 区间内）",
        "get_coverage_report" => "只读：覆盖报告（#219——哪些块没归属 + 未覆盖区间 + 各类 verdict 块数；全覆盖才允许建纲）",
        _ => "unknown",
    }
}

fn is_implemented(name: &str) -> bool {
    matches!(
        name,
        "get_bid_project_state"
            // save_version 早已实装（#42 阶段 10 分发臂）——此前 is_implemented
            // 漏登致 tools/list 误报 false（#145 测试面抓出的真 bug，如实登记）
            | "save_version"
            | "create_bid_project"
            | "import_tender_files"
            | "extract_requirements"
            | "get_compliance_matrix"
            | "list_evidence"
            | "list_tasks"
            | "search_evidence"
            | "draft_response_section"
            | "export_bid_document"
            | "validate_bid_document"
            | "check_bid_quality"
            | "build_outline"
            | "register_evidence"
            | "bind_evidence"
            | "record_review_gate"
            | "record_addendum_waiver"
            | "score_price"
            | "score_section"
            | "score_document"
            // #219 全块覆盖三工具（coverage_tools.rs）
            | "get_document_chunks"
            | "submit_block_marks"
            | "get_coverage_report"
    )
}
