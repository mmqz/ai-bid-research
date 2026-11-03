//! llm.rs —— #68 阶段 1：真实 LLM 通路（OpenAI 兼容 chat/completions）。
//!
//! 分层（与 draft.rs 同族纪律——生成面可注入可测试）：
//! - **配置**（[`LlmConfig`] / [`LlmConfig::resolve`]）：`.mang/llm.json`
//!   （项目面）+ 环境变量 `BID_LLM_API_BASE` / `BID_LLM_API_KEY` /
//!   `BID_LLM_MODEL`（env 覆盖文件——CI/本地免写盘切模型）。api_key 缺失
//!   → `Ok(None)`（调用方降级 mock——issue #68 §1.2）；文件在场但坏
//!   JSON → `Err`（配置错误要响亮，不静默吞）；
//! - **用量台账**（[`LlmUsage`]）：`.mang/llm-usage.json` 累计
//!   calls / prompt_tokens / completion_tokens / total_tokens / by_model /
//!   last_at（issue #68 §1.3 成本记录面）；
//! - **传输**（[`LlmTransport`]）：HTTP 面的注入位——[`HttpTransport`]
//!   （ureq，60s 超时）是真实通路；测试注入 FakeTransport（不起端口，
//!   单测零网络）。超时是**传输层错误**（重试语义在生成器）；
//! - **生成器**（[`RealGenerator`]）：实现 [`SectionGenerator`]——prompt
//!   由素材面逐字段展开（标题/目标字数/起草指令/逐 REQ 摘录/逐 EV 摘要，
//!   **无硬编码内容模板**——issue #68 纪律 4：章节差异由素材承载）；请求
//!   `max_tokens` 按 target_chars 推导（防失控）；超时重试 1 次；返回容错
//!   解析（envelope JSON → content 层 JSON → 纯文本兜底——issue #68 §1.1
//!   「JSON 失败时当作纯文本」）；completion_tokens 超硬上限 → 失控错误；
//!   用量落盘。
//!
//! 凭据纪律：api_key 只在配置文件/env 与 HTTP 头存在，**绝不写日志、绝不
//! 进生成内容、绝不落 llm-usage.json**。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::draft::{GeneratedDraft, SectionDraftInput, SectionGenerator};

/// LLM 配置（OpenAI 兼容面）。
#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    /// API 基址（含版本路径——如 `https://api.openai.com/v1`、
    /// `https://api.deepseek.com/v1`）。
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    /// 单请求超时（秒）——超时重试 1 次（issue #68 §1.3）。
    pub timeout_secs: u64,
    /// 采样温度（标书文风偏稳——默认 0.3）。
    pub temperature: f32,
    /// 单章 completion token 硬上限（防生成失控——默认 8192）。
    pub max_tokens_cap: u64,
}

/// 配置文件面（`.mang/llm.json`——全字段可选，api_key 逻辑见 resolve）。
#[derive(Debug, Default, Deserialize)]
struct LlmConfigFile {
    api_base: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    timeout_secs: Option<u64>,
    temperature: Option<f32>,
    max_tokens_cap: Option<u64>,
}

impl LlmConfig {
    /// 环境变量名（env 覆盖文件——本地/CI 免写盘切换）。
    pub const ENV_API_BASE: &'static str = "BID_LLM_API_BASE";
    pub const ENV_API_KEY: &'static str = "BID_LLM_API_KEY";
    pub const ENV_MODEL: &'static str = "BID_LLM_MODEL";

    /// 真实环境解析（env → 文件 的并集，env 优先）。
    pub fn resolve(ws: Option<&Path>) -> Result<Option<LlmConfig>, String> {
        Self::resolve_with(ws, &|k| {
            std::env::var(k).ok().filter(|v| !v.trim().is_empty())
        })
    }

    /// 可注入解析（测试面——env 查询函数化，避免测试改全局环境）。
    pub fn resolve_with(
        ws: Option<&Path>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<LlmConfig>, String> {
        let mut f = LlmConfigFile::default();
        if let Some(dir) = ws {
            let path = dir.join(".mang").join("llm.json");
            if path.is_file() {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
                f = serde_json::from_str(&text).map_err(|e| {
                    format!("{} 不是合法 JSON（LLM 配置错误要响亮）: {e}", path.display())
                })?;
            }
        }
        let pick = |file: Option<String>, key: &str| env(key).or(file).unwrap_or_default();
        let api_key = pick(f.api_key, Self::ENV_API_KEY);
        if api_key.trim().is_empty() {
            // 无 key → 降级面（调用方走 mock + 告警——issue #68 §1.2）
            return Ok(None);
        }
        let model = pick(f.model, Self::ENV_MODEL);
        if model.trim().is_empty() {
            return Err("LLM 配置缺 model（.mang/llm.json 或 BID_LLM_MODEL）——api_key 在场时 model 必填".into());
        }
        let timeout_secs = f.timeout_secs.unwrap_or(60);
        if timeout_secs == 0 {
            return Err("LLM 配置 timeout_secs 须 > 0".into());
        }
        let temperature = f.temperature.unwrap_or(0.3);
        if !(0.0..=2.0).contains(&temperature) {
            return Err(format!("LLM 配置 temperature 超界（0.0-2.0）: {temperature}"));
        }
        let max_tokens_cap = f.max_tokens_cap.unwrap_or(8192);
        if max_tokens_cap < 256 {
            return Err(format!("LLM 配置 max_tokens_cap 过小（>=256）: {max_tokens_cap}"));
        }
        Ok(Some(LlmConfig {
            api_base: pick(f.api_base, Self::ENV_API_BASE)
                .trim_end_matches('/')
                .to_string(),
            api_key,
            model,
            timeout_secs,
            temperature,
            max_tokens_cap,
        }))
    }

    /// chat/completions 端点（api_base 含版本路径的 OpenAI SDK 约定）。
    fn endpoint(&self) -> String {
        let base = if self.api_base.is_empty() {
            "https://api.openai.com/v1"
        } else {
            &self.api_base
        };
        format!("{base}/chat/completions")
    }
}

// ---------------------------------------------------------------- 用量台账

/// 累计用量（`.mang/llm-usage.json`——issue #68 §1.3 成本面）。
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LlmUsage {
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub by_model: BTreeMap<String, ModelUsage>,
    /// 最近一次调用（unix 秒——跨时区口径稳定）。
    pub last_at: u64,
}

/// 单模型累计。
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelUsage {
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl LlmUsage {
    /// 台账路径（`.mang/llm-usage.json`——与 llm.json 同目录）。
    fn path_for(ws: &Path) -> PathBuf {
        ws.join(".mang").join("llm-usage.json")
    }

    /// 读台账（缺失/损坏 → 空台账——台账是审计面不是阻断面，损坏重新累计
    /// 并在下一次写入时覆盖；决不因台账问题阻塞生成）。
    fn load(ws: &Path) -> LlmUsage {
        match std::fs::read_to_string(Self::path_for(ws)) {
            Ok(t) => serde_json::from_str(&t).unwrap_or_default(),
            Err(_) => LlmUsage::default(),
        }
    }

    /// 记一笔并落盘（atomic-write：tmp + rename——与 store_io 同纪律）。
    fn record_and_save(ws: &Path, model: &str, pt: u64, ct: u64) {
        let mut u = Self::load(ws);
        u.calls += 1;
        u.prompt_tokens += pt;
        u.completion_tokens += ct;
        u.total_tokens += pt + ct;
        let m = u.by_model.entry(model.to_string()).or_default();
        m.calls += 1;
        m.prompt_tokens += pt;
        m.completion_tokens += ct;
        m.total_tokens += pt + ct;
        u.last_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = Self::path_for(ws);
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let tmp = path.with_extension("json.tmp");
        if serde_json::to_string_pretty(&u)
            .map_err(|e| e.to_string())
            .and_then(|t| std::fs::write(&tmp, t).map_err(|e| e.to_string()))
            .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()))
            .is_err()
        {
            // 台账失败不阻断生成（审计面尽力而为——错误已在返回值语义外）
        }
    }
}

// ---------------------------------------------------------------- 传输层

/// HTTP 响应（传输成功的面——状态码非 200 由生成器裁决错误语义）。
pub struct LlmHttpResponse {
    pub status: u16,
    pub body: String,
}

/// 传输层错误（网络面——非 HTTP 状态面）。
#[derive(Debug, Clone, PartialEq)]
pub enum LlmHttpError {
    /// 超时（生成器层重试 1 次）。
    Timeout(String),
    /// 其他传输失败（DNS/连接中断/协议……——不重试，响亮报错）。
    Transport(String),
}

/// LLM 传输注入位（测试 FakeTransport / 真实 HttpTransport 同接口）。
pub trait LlmTransport {
    /// `POST {api_base}/chat/completions`（Bearer 鉴权 + JSON body）。
    fn post_chat(
        &self,
        cfg: &LlmConfig,
        body: &serde_json::Value,
    ) -> Result<LlmHttpResponse, LlmHttpError>;
}

/// ureq 真实传输（阻塞 + rustls——无 tokio，与 workspace 无异步栈一致）。
pub struct HttpTransport;

impl LlmTransport for HttpTransport {
    fn post_chat(
        &self,
        cfg: &LlmConfig,
        body: &serde_json::Value,
    ) -> Result<LlmHttpResponse, LlmHttpError> {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(cfg.timeout_secs))
            .build();
        let resp = agent
            .post(&cfg.endpoint())
            .set("Authorization", &format!("Bearer {}", cfg.api_key))
            .set("Content-Type", "application/json")
            .send_string(&body.to_string());
        match resp {
            Ok(r) => {
                let status = r.status();
                let body = r.into_string().map_err(|e| LlmHttpError::Transport(e.to_string()))?;
                Ok(LlmHttpResponse { status, body })
            }
            Err(ureq::Error::Status(code, r)) => {
                // HTTP 错误状态也是「有响应」——交给生成器给明确错误语义
                let body = r.into_string().unwrap_or_default();
                Ok(LlmHttpResponse { status: code, body })
            }
            Err(e) => {
                let msg = transport_message(&e);
                if is_timeout(&e) {
                    Err(LlmHttpError::Timeout(msg))
                } else {
                    Err(LlmHttpError::Transport(msg))
                }
            }
        }
    }
}

/// 传输错误消息（ureq 错误的统一文案面）。
fn transport_message(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => t.to_string(),
    }
}

/// 超时判定（io TimedOut/WouldBlock 或消息含 timed out——跨 TLS 栈兜底）。
fn is_timeout(e: &ureq::Error) -> bool {
    use std::error::Error as _;
    match e {
        ureq::Error::Status(..) => false,
        ureq::Error::Transport(t) => {
            if let Some(io) = t.source().and_then(|s| s.downcast_ref::<std::io::Error>()) {
                if matches!(io.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) {
                    return true;
                }
            }
            let m = t.to_string().to_lowercase();
            m.contains("timed out") || m.contains("timeout")
        }
    }
}

// ---------------------------------------------------------------- 生成器

/// 真实 LLM 生成器（OpenAI 兼容 chat/completions——一次性调用，非流式）。
///
/// 生成语义（issue #68 §1.1-§1.3 全量落地）：
/// - prompt 组装：系统面只给规则（引用纪律/字数/输出形态），**素材面逐字段
///   展开**（标题/目标字数/指令/REQ 摘录/EV 摘要）——无任何预写内容句
///   （纪律 4：标书章节差异大，内容只来自素材）；
/// - `max_tokens` = clamp(target_chars × 3 + 400, 512, cap)（中文 1 字 ≈
///   1-2 token 的裕量面；target=0 退化 1024）；
/// - 超时重试 1 次（共 2 次尝试）；非超时传输错误不重试；
/// - 容错解析：envelope JSON（choices[0].message.content）→ content 层
///   JSON（`{"content": ...}` 解包）→ 纯文本兜底；
/// - completion_tokens > cap × 2 → 失控错误（防 max_tokens 被服务端忽略）；
/// - 用量台账落盘（workspace 在场时）。
pub struct RealGenerator {
    cfg: LlmConfig,
    transport: Box<dyn LlmTransport>,
    /// 用量台账目录（None = 纯内存态——不记台账）。
    usage_ws: Option<PathBuf>,
    /// 超时重试次数（issue 定 1）。
    retries: u32,
}

impl RealGenerator {
    /// 真实通路构造（HttpTransport + workspace 台账）。
    pub fn new(cfg: LlmConfig, usage_ws: Option<PathBuf>) -> Self {
        Self {
            cfg,
            transport: Box::new(HttpTransport),
            usage_ws,
            retries: 1,
        }
    }

    /// 测试构造（注入 FakeTransport + 显式台账目录）。
    pub fn new_with_transport(
        cfg: LlmConfig,
        transport: Box<dyn LlmTransport>,
        usage_ws: Option<PathBuf>,
    ) -> Self {
        Self { cfg, transport, usage_ws, retries: 1 }
    }

    /// 请求体（素材面数据驱动组装——测试对账面）。
    fn build_request_body(&self, input: &SectionDraftInput) -> serde_json::Value {
        let max_tokens = request_max_tokens(input.target_chars, self.cfg.max_tokens_cap);
        serde_json::json!({
            "model": self.cfg.model,
            "temperature": self.cfg.temperature,
            "max_tokens": max_tokens,
            "messages": [
                { "role": "system", "content": system_framing(input) },
                { "role": "user",   "content": user_material(input) },
            ],
        })
    }

    /// 响应裁决面（状态码 / 容错解析 / 失控检查 / 台账）。
    fn handle_response(&self, resp: LlmHttpResponse) -> Result<GeneratedDraft, String> {
        if resp.status != 200 {
            return Err(format!(
                "LLM HTTP {}（model={}）: {}",
                resp.status,
                self.cfg.model,
                truncate(&resp.body, 300),
            ));
        }
        // ---- envelope 容错解析（JSON 失败 → 纯文本兜底——issue §1.1）
        let (content, usage) = match parse_envelope(&resp.body) {
            EnvelopeOutcome::Parsed { content, usage } => (content, usage),
            EnvelopeOutcome::NotJson => {
                // 容错底线：200 且有 body 但不是 JSON envelope——当纯文本
                // （部分兼容端点/网关的直接文本返回）。空 body 仍要响亮拒绝。
                if resp.body.trim().is_empty() {
                    return Err("LLM 返回 200 但 body 为空".into());
                }
                (resp.body.clone(), None)
            }
            EnvelopeOutcome::Malformed(why) => {
                return Err(format!("LLM 返回 200 但 envelope 异常: {why}"));
            }
        };
        // ---- content 层 JSON 解包（模型自包 JSON 的常见形态）
        let content = unwrap_json_content(&content);
        if content.trim().is_empty() {
            return Err("LLM 生成内容为空（choices[0].message.content 空）".into());
        }
        // ---- 失控检查 + 台账
        if let Some(u) = &usage {
            if u.completion_tokens > self.cfg.max_tokens_cap.saturating_mul(2) {
                return Err(format!(
                    "LLM 生成失控：completion_tokens={} 超硬上限 {} 的 2 倍——整章废弃",
                    u.completion_tokens, self.cfg.max_tokens_cap,
                ));
            }
            if let Some(ws) = &self.usage_ws {
                LlmUsage::record_and_save(ws, &self.cfg.model, u.prompt_tokens, u.completion_tokens);
            }
        }
        Ok(GeneratedDraft { content })
    }
}

impl SectionGenerator for RealGenerator {
    fn generate(&mut self, input: &SectionDraftInput) -> Result<GeneratedDraft, String> {
        let body = self.build_request_body(input);
        let mut last_timeout = String::new();
        for _ in 0..=self.retries {
            match self.transport.post_chat(&self.cfg, &body) {
                Ok(resp) => return self.handle_response(resp),
                Err(LlmHttpError::Timeout(m)) => last_timeout = m,
                Err(LlmHttpError::Transport(m)) => {
                    return Err(format!("LLM 传输失败（不重试）: {m}"));
                }
            }
        }
        Err(format!(
            "LLM 调用超时（{} 次尝试 × {}s 上限）: {last_timeout}",
            self.retries + 1,
            self.cfg.timeout_secs,
        ))
    }
}

// ---------------------------------------------------------------- prompt 组装

/// 系统面（规则——不承载章节内容，全部数据来自 input）。
fn system_framing(input: &SectionDraftInput) -> String {
    format!(
        "你是标书撰写引擎，为章节《{title}》撰写正文。规则：\
1) 输出 Markdown，首行是一级标题「# {title}」，其后是正文段落；\
2) 引用要求或证据时使用 [REQ-xxxx] / [EV-xxxx] 编号形态，且只准使用用户消息中列出的编号，禁止编造不存在的编号；\
3) 每一条要求条款都必须有对应的响应内容；\
4) 篇幅以用户消息给出的目标字数为准，允许上下浮动 20%；\
5) 只输出章节正文，不输出解释、前言、草稿标记或对话。",
        title = input.title,
    )
}

/// 素材面（逐字段展开——章节差异的承载面，无预写内容句）。
fn user_material(input: &SectionDraftInput) -> String {
    let mut v = Vec::new();
    v.push("章节素材如下。".to_string());
    v.push(format!("章节标题：{}", input.title));
    v.push(format!("目标字数：{}（±20%）", input.target_chars));
    if let Some(inst) = input.instructions.as_deref() {
        if !inst.trim().is_empty() {
            v.push(format!("起草指令：{inst}"));
        }
    }
    v.push("要求条款（逐条回应，引用其编号）：".to_string());
    for r in &input.requirements {
        let exc = if r.excerpt.trim().is_empty() { "（原文摘录缺失）" } else { &r.excerpt };
        v.push(format!("- [{}]（{}）「{exc}」", r.id, r.category));
    }
    if input.evidences.is_empty() {
        v.push("证据材料：无（不得编造证据编号）。".to_string());
    } else {
        v.push("证据材料（可援引佐证，引用其编号）：".to_string());
        for e in &input.evidences {
            let scope = if e.scope.trim().is_empty() { "见证据原件" } else { &e.scope };
            let mut line = format!("- [{}]（{}：{}，适用范围：{scope}）", e.id, e.kind, e.title);
            // #217（#185 缺口 2）：正文摘录随行——引用金额/日期/范围等实料
            // 只准来自这段逐字截取的原文；无摘录（文件级锚）则显式告知不得编造细节。
            if !e.excerpt.trim().is_empty() {
                let loc = if e.excerpt_locator.trim().is_empty() {
                    "原文".to_string()
                } else {
                    e.excerpt_locator.clone()
                };
                line.push_str(&format!(
                    "。{loc}摘录（实料引用以此为准，不得超出摘录编造）：「{}」",
                    e.excerpt
                ));
            } else {
                line.push_str("。（正文未定位摘录——不得编造该证据的金额/日期/范围等细节）");
            }
            v.push(line);
        }
    }
    v.join("\n")
}

/// 请求 max_tokens（中文 1 字 ≈ 1-2 token，×3 裕量；512 下限防退化过小）。
fn request_max_tokens(target_chars: u32, cap: u64) -> u64 {
    let want = if target_chars > 0 {
        u64::from(target_chars) * 3 + 400
    } else {
        1024
    };
    want.clamp(512, cap)
}

// ---------------------------------------------------------------- 响应解析

/// envelope usage 面（容错：字段缺失按 0 记）。
#[derive(Debug, Deserialize)]
struct UsagePart {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

/// envelope choices 面（message.content 为 chat 形态；text 为旧 completions 形态）。
#[derive(Debug, Deserialize)]
struct ChoicePart {
    message: Option<MessagePart>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessagePart {
    content: Option<serde_json::Value>,
}

struct ParsedUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

enum EnvelopeOutcome {
    Parsed { content: String, usage: Option<ParsedUsage> },
    NotJson,
    Malformed(String),
}

/// envelope 解析（OpenAI 兼容 chat/completions）。
fn parse_envelope(body: &str) -> EnvelopeOutcome {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return EnvelopeOutcome::NotJson,
    };
    let choices = match v.get("choices").and_then(|c| c.as_array()) {
        Some(c) if !c.is_empty() => c,
        _ => {
            return EnvelopeOutcome::Malformed(
                "body 是 JSON 但无 choices（兼容端点异常返回）".into(),
            )
        }
    };
    let mut content: Option<String> = None;
    for c in choices {
        let cp: Option<ChoicePart> = serde_json::from_value(c.clone()).ok();
        if let Some(cp) = cp {
            if let Some(m) = cp.message {
                if let Some(cv) = m.content {
                    content = Some(value_to_text(&cv));
                    break;
                }
            }
            if let Some(t) = cp.text {
                content = Some(t);
                break;
            }
        }
    }
    let content = match content {
        Some(c) => c,
        None => {
            return EnvelopeOutcome::Malformed(
                "choices 在场但无 message.content/text（content 可能被 content_filter 截断）".into(),
            )
        }
    };
    let usage = v.get("usage").and_then(|u| serde_json::from_value::<UsagePart>(u.clone()).ok())
        .map(|u| ParsedUsage { prompt_tokens: u.prompt_tokens, completion_tokens: u.completion_tokens });
    EnvelopeOutcome::Parsed { content, usage }
}

/// message.content 的形态收敛（字符串 / null / 少数端点的分段数组）。
fn value_to_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// content 层 JSON 解包（模型自包 `{"content": "..."}` 的形态——解不出就原样返回）。
fn unwrap_json_content(content: &str) -> String {
    let t = content.trim();
    if !(t.starts_with('{') && t.ends_with('}')) {
        return content.to_string();
    }
    let v: serde_json::Value = match serde_json::from_str(t) {
        Ok(v) => v,
        Err(_) => return content.to_string(),
    };
    for key in ["content", "text", "body"] {
        if let Some(s) = v.get(key).and_then(|k| k.as_str()) {
            return s.to_string();
        }
    }
    content.to_string()
}

/// 尾部截断（错误消息里的 body 摘录面）。
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}
