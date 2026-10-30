//! llm_generator.rs —— #68 阶段 1：真实 LLM 通路单元面（RealGenerator）。
//!
//! 覆盖（issue #68 §1.1-§1.3 验收线的可注入面）：
//! - 配置解析：`.mang/llm.json` / env 覆盖 / 无 key → None（降级面）/
//!   坏 JSON → 响亮错误 / 有 key 无 model → 响亮错误 / 字段合法性；
//! - prompt 数据驱动：请求 messages 必含章节标题、逐 REQ 编号+摘录、
//!   逐 EV 编号+摘要、目标字数、起草指令（纪律 4——无预写内容句）；
//!   请求体绝不含 api_key（凭据只进 HTTP 头）；
//! - max_tokens 推导：target×3+400，512 下限，cap 上限；
//! - 容错解析：envelope JSON → content 层 JSON 解包 → 纯文本兜底；
//!   choices 空数组 / 空 body → 响亮错误；
//! - 超时重试 1 次（第 2 次成功 → Ok，2 次请求）；连续超时 → 明确错误；
//!   非超时传输错误不重试（1 次请求）；
//! - HTTP 非 200 → 响亮错误（含状态码与 body 摘录）；
//! - 失控检测：completion_tokens 超硬上限 2 倍 → 整章废弃（不落台账）；
//! - 用量台账：`.mang/llm-usage.json` 累计（calls/tokens/by_model/last_at），
//!   台账绝不含 api_key；
//! - 真实 API 冒烟：SKIP 门（无 BID_LLM_API_KEY 不跑——CI 零网络依赖）。
//!
//! 传输注入：FakeTransport 共享请求日志（Rc<RefCell>）+ 脚本化响应——
//! 生成器装箱消费后测试仍可对账请求面。单测零网络零端口。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;

use bid_tools::draft::{GeneratedDraft, SectionDraftInput, SectionGenerator};
use bid_tools::llm::{LlmConfig, LlmHttpError, LlmHttpResponse, LlmTransport, RealGenerator};
use serde_json::{json, Value};

// ---------------------------------------------------------------- 夹具

fn cfg() -> LlmConfig {
    LlmConfig {
        api_base: "https://api.example.com/v1".into(),
        api_key: "sk-test-never-real".into(),
        model: "test-model".into(),
        timeout_secs: 60,
        temperature: 0.3,
        max_tokens_cap: 8192,
    }
}

fn input() -> SectionDraftInput {
    SectionDraftInput {
        section_id: "SEC-0001".into(),
        title: "施工组织设计".into(),
        target_chars: 1000,
        requirements: vec![
            bid_tools::draft::ReqBrief {
                id: "REQ-0001".into(),
                category: "技术参数".into(),
                excerpt: "投标人须具备水利水电工程施工总承包贰级及以上资质".into(),
            },
            bid_tools::draft::ReqBrief {
                id: "REQ-0002".into(),
                category: "否决项".into(),
                excerpt: "投标文件未经法定代表人签字的作否决处理".into(),
            },
        ],
        evidences: vec![bid_tools::draft::EvBrief {
            id: "EV-0001".into(),
            kind: "资质证书".into(),
            title: "施工总承包贰级资质证书".into(),
            scope: "本标段全部施工内容".into(),
        }],
        instructions: Some("突出同类水库工程的施工业绩".into()),
    }
}

/// 标准成功 envelope。
fn envelope(content: &str, pt: u64, ct: u64) -> LlmHttpResponse {
    LlmHttpResponse {
        status: 200,
        body: json!({
            "choices": [{ "message": { "role": "assistant", "content": content } }],
            "usage": { "prompt_tokens": pt, "completion_tokens": ct, "total_tokens": pt + ct },
        })
        .to_string(),
    }
}

/// 共享请求日志（生成器装箱消费 FakeTransport 后，测试仍持有读面）。
type Log = Rc<RefCell<Vec<Value>>>;

struct FakeTransport {
    log: Log,
    script: RefCell<VecDeque<Result<LlmHttpResponse, LlmHttpError>>>,
}

impl FakeTransport {
    fn scripted(results: Vec<Result<LlmHttpResponse, LlmHttpError>>) -> (Self, Log) {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        (
            Self { log: log.clone(), script: RefCell::new(results.into()) },
            log,
        )
    }
}

impl LlmTransport for FakeTransport {
    fn post_chat(&self, _cfg: &LlmConfig, body: &Value) -> Result<LlmHttpResponse, LlmHttpError> {
        self.log.borrow_mut().push(body.clone());
        self.script
            .borrow_mut()
            .pop_front()
            .expect("测试脚本耗尽——脚本与重试次数不匹配")
    }
}

fn temp_ws(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bid-llm-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".mang")).unwrap();
    dir
}

fn no_env(_: &str) -> Option<String> {
    None
}

/// 生成（trait 对象形态——MCP 壳的真实使用面），返回 (结果, 请求日志)。
fn generate_logged(
    ft: FakeTransport,
    log: Log,
    ws: Option<PathBuf>,
    input: &SectionDraftInput,
) -> (Result<GeneratedDraft, String>, Vec<Value>) {
    let gen = RealGenerator::new_with_transport(cfg(), Box::new(ft), ws);
    let mut boxed: Box<dyn SectionGenerator> = Box::new(gen);
    let r = boxed.generate(input);
    (r, log.borrow().clone())
}

fn generate_with(
    ft: FakeTransport,
    ws: Option<PathBuf>,
    input: &SectionDraftInput,
) -> Result<GeneratedDraft, String> {
    let gen = RealGenerator::new_with_transport(cfg(), Box::new(ft), ws);
    let mut boxed: Box<dyn SectionGenerator> = Box::new(gen);
    boxed.generate(input)
}

// ---------------------------------------------------------------- 配置解析

#[test]
fn config_no_key_resolves_none_for_degradation() {
    // 无文件无 env → None（auto 模式降级 mock 的判定面）
    let ws = temp_ws("cfg-none");
    assert_eq!(LlmConfig::resolve_with(Some(&ws), &no_env).unwrap(), None);
    // 空串 api_key 同 None（.trim().is_empty 面）
    let ws = temp_ws("cfg-empty");
    std::fs::write(ws.join(".mang/llm.json"), r#"{"api_key": "  "}"#).unwrap();
    assert_eq!(LlmConfig::resolve_with(Some(&ws), &no_env).unwrap(), None);
}

#[test]
fn config_file_and_env_priority() {
    let ws = temp_ws("cfg-file");
    std::fs::write(
        ws.join(".mang/llm.json"),
        r#"{"api_base": "https://file.example.com/v1", "api_key": "sk-file",
            "model": "file-model", "timeout_secs": 30, "temperature": 0.1}"#,
    )
    .unwrap();
    // 纯文件 → 全量生效
    let c = LlmConfig::resolve_with(Some(&ws), &no_env).unwrap().unwrap();
    assert_eq!(c.api_base, "https://file.example.com/v1");
    assert_eq!(c.api_key, "sk-file");
    assert_eq!(c.model, "file-model");
    assert_eq!(c.timeout_secs, 30);
    assert_eq!(c.temperature, 0.1);
    assert_eq!(c.max_tokens_cap, 8192); // 缺省
    // env 覆盖文件（key/model/base 三面；尾斜杠收敛）
    let c = LlmConfig::resolve_with(Some(&ws), &|k| match k {
        "BID_LLM_API_KEY" => Some("sk-env".into()),
        "BID_LLM_MODEL" => Some("env-model".into()),
        "BID_LLM_API_BASE" => Some("https://env.example.com/v1/".into()),
        _ => None,
    })
    .unwrap()
    .unwrap();
    assert_eq!(c.api_key, "sk-env");
    assert_eq!(c.model, "env-model");
    assert_eq!(c.api_base, "https://env.example.com/v1");
}

#[test]
fn config_errors_are_loud() {
    // 坏 JSON → Err（配置错误不静默吞）
    let ws = temp_ws("cfg-bad");
    std::fs::write(ws.join(".mang/llm.json"), "{ not json").unwrap();
    let e = LlmConfig::resolve_with(Some(&ws), &no_env).unwrap_err();
    assert!(e.contains("不是合法 JSON"), "{e}");
    // 有 key 无 model → Err
    let ws = temp_ws("cfg-nomodel");
    std::fs::write(ws.join(".mang/llm.json"), r#"{"api_key": "sk-x"}"#).unwrap();
    let e = LlmConfig::resolve_with(Some(&ws), &no_env).unwrap_err();
    assert!(e.contains("model"), "{e}");
    // 越界字段 → Err（timeout=0 / temperature 超界）
    for (tag, body) in [
        ("cfg-t0", r#"{"api_key": "sk-x", "model": "m", "timeout_secs": 0}"#),
        ("cfg-tmp", r#"{"api_key": "sk-x", "model": "m", "temperature": 9.9}"#),
        ("cfg-cap", r#"{"api_key": "sk-x", "model": "m", "max_tokens_cap": 10}"#),
    ] {
        let ws = temp_ws(tag);
        std::fs::write(ws.join(".mang/llm.json"), body).unwrap();
        assert!(LlmConfig::resolve_with(Some(&ws), &no_env).is_err(), "{tag}");
    }
}

// ---------------------------------------------------------------- 请求面对账

#[test]
fn request_body_carries_all_material_fields() {
    let (ft, log) = FakeTransport::scripted(vec![Ok(envelope("# 施工组织设计\n正文", 100, 200))]);
    let (r, reqs) = generate_logged(ft, log, None, &input());
    r.unwrap();
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    // 模型与采样面
    assert_eq!(req["model"], "test-model");
    assert_eq!(req["max_tokens"], 3400); // 1000×3+400
    // messages：system（规则）+ user（素材）
    let msgs = req["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    let user = msgs[1]["content"].as_str().unwrap();
    // 数据驱动验收（纪律 4——素材逐字段进 prompt，缺一不可）
    assert!(user.contains("施工组织设计"), "标题");
    assert!(user.contains("目标字数：1000"), "目标字数");
    assert!(user.contains("突出同类水库工程的施工业绩"), "起草指令");
    assert!(user.contains("[REQ-0001]"), "REQ 编号");
    assert!(user.contains("[REQ-0002]"), "REQ 编号 2");
    assert!(user.contains("投标人须具备水利水电工程施工总承包贰级及以上资质"), "REQ 摘录");
    assert!(user.contains("[EV-0001]"), "EV 编号");
    assert!(user.contains("施工总承包贰级资质证书"), "EV 标题");
    // 凭据纪律：请求体绝不含 api_key（只进 HTTP 头）
    assert!(!req.to_string().contains("sk-test-never-real"));
}

#[test]
fn max_tokens_clamps_to_bounds() {
    // target=0 → 1024；小 target → 512 下限；大 target → cap 上限
    for (target, expect) in [(0u32, 1024u64), (10, 512), (1_000_000, 8192)] {
        let mut inp = input();
        inp.target_chars = target;
        let (ft, log) = FakeTransport::scripted(vec![Ok(envelope("# t", 1, 1))]);
        let (r, reqs) = generate_logged(ft, log, None, &inp);
        r.unwrap();
        assert_eq!(reqs[0]["max_tokens"], expect, "target={target}");
    }
}

// ---------------------------------------------------------------- 容错解析

#[test]
fn happy_path_parses_envelope_and_records_usage() {
    let ws = temp_ws("happy");
    let (ft, _) = FakeTransport::scripted(vec![Ok(envelope("# 施工组织设计\n正文内容", 120, 340))]);
    let out = generate_with(ft, Some(ws.clone()), &input()).unwrap_or_else(|e| panic!("{e}"));
    assert!(out.content.contains("正文内容"));
    // 用量台账（issue §1.3——calls/tokens/by_model 全量累计）
    let usage: Value =
        serde_json::from_str(&std::fs::read_to_string(ws.join(".mang/llm-usage.json")).unwrap())
            .unwrap();
    assert_eq!(usage["calls"].as_u64().unwrap(), 1);
    assert_eq!(usage["prompt_tokens"].as_u64().unwrap(), 120);
    assert_eq!(usage["completion_tokens"].as_u64().unwrap(), 340);
    assert_eq!(usage["total_tokens"].as_u64().unwrap(), 460);
    assert_eq!(usage["by_model"]["test-model"]["calls"].as_u64().unwrap(), 1);
    assert!(usage["last_at"].as_u64().unwrap() > 0);
    // 台账不含 api_key（凭据纪律——绝不落盘）
    let raw = std::fs::read_to_string(ws.join(".mang/llm-usage.json")).unwrap();
    assert!(!raw.contains("sk-test-never-real"));
}

#[test]
fn usage_accumulates_across_calls() {
    let ws = temp_ws("accum");
    for i in 0..2u64 {
        let (ft, _) = FakeTransport::scripted(vec![Ok(envelope(&format!("# t{i}"), 10 * (i + 1), 20))]);
        generate_with(ft, Some(ws.clone()), &input()).unwrap_or_else(|e| panic!("{e}"));
    }
    let usage: Value =
        serde_json::from_str(&std::fs::read_to_string(ws.join(".mang/llm-usage.json")).unwrap())
            .unwrap();
    assert_eq!(usage["calls"].as_u64().unwrap(), 2);
    assert_eq!(usage["prompt_tokens"].as_u64().unwrap(), 30);
    assert_eq!(usage["completion_tokens"].as_u64().unwrap(), 40);
    assert_eq!(usage["by_model"]["test-model"]["calls"].as_u64().unwrap(), 2);
}

#[test]
fn json_wrapped_content_is_unwrapped() {
    // 模型自包 {"content": "..."} 的形态 → 解包
    let inner = "# 施工组织设计\n被包裹的正文";
    let wrapped = json!({ "content": inner }).to_string();
    let (ft, _) = FakeTransport::scripted(vec![Ok(envelope(&wrapped, 5, 5))]);
    let out = generate_with(ft, None, &input()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.content, inner);
}

#[test]
fn non_json_body_falls_back_to_plain_text() {
    // issue §1.1「JSON 失败时当作纯文本」——200 + 非 JSON body → 正文兜底
    let (ft, _) = FakeTransport::scripted(vec![Ok(LlmHttpResponse {
        status: 200,
        body: "# 施工组织设计\n纯文本兜底的正文".into(),
    })]);
    let out = generate_with(ft, None, &input()).unwrap_or_else(|e| panic!("{e}"));
    assert!(out.content.contains("纯文本兜底的正文"));
}

#[test]
fn empty_choices_is_loud_error() {
    let (ft, _) = FakeTransport::scripted(vec![Ok(LlmHttpResponse {
        status: 200,
        body: json!({ "choices": [], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } })
            .to_string(),
    })]);
    let e = generate_with(ft, None, &input()).unwrap_err();
    assert!(e.contains("envelope"), "{e}");
}

#[test]
fn empty_body_is_loud_error() {
    let (ft, _) = FakeTransport::scripted(vec![Ok(LlmHttpResponse { status: 200, body: "   ".into() })]);
    let e = generate_with(ft, None, &input()).unwrap_err();
    assert!(e.contains("空"), "{e}");
}

// ---------------------------------------------------------------- 超时 / 重试 / 失控

#[test]
fn timeout_retries_once_then_succeeds() {
    let (ft, log) = FakeTransport::scripted(vec![
        Err(LlmHttpError::Timeout("first attempt timed out".into())),
        Ok(envelope("# t\n成功", 10, 10)),
    ]);
    let (r, reqs) = generate_logged(ft, log, None, &input());
    assert!(r.unwrap().content.contains("成功"));
    assert_eq!(reqs.len(), 2, "超时重试 1 次 → 共 2 次请求");
}

#[test]
fn double_timeout_is_clear_error() {
    let (ft, log) = FakeTransport::scripted(vec![
        Err(LlmHttpError::Timeout("t1".into())),
        Err(LlmHttpError::Timeout("t2".into())),
    ]);
    let (r, reqs) = generate_logged(ft, log, None, &input());
    let e = r.unwrap_err();
    assert!(e.contains("超时"), "{e}");
    assert!(e.contains("2 次尝试"), "{e}"); // 60s × 重试 1 次的语义面
    assert_eq!(reqs.len(), 2);
}

#[test]
fn transport_error_does_not_retry() {
    let (ft, log) = FakeTransport::scripted(vec![Err(LlmHttpError::Transport("connection reset".into()))]);
    let (r, reqs) = generate_logged(ft, log, None, &input());
    let e = r.unwrap_err();
    assert!(e.contains("传输失败"), "{e}");
    assert!(e.contains("connection reset"), "{e}");
    assert_eq!(reqs.len(), 1, "非超时传输错误不重试");
}

#[test]
fn http_error_is_loud_with_status_and_excerpt() {
    let (ft, _) = FakeTransport::scripted(vec![Ok(LlmHttpResponse {
        status: 401,
        body: r#"{"error": {"message": "Invalid API key"}}"#.into(),
    })]);
    let e = generate_with(ft, None, &input()).unwrap_err();
    assert!(e.contains("401"), "{e}");
    assert!(e.contains("Invalid API key"), "{e}");
}

#[test]
fn runaway_completion_is_rejected_without_ledger() {
    // completion 超 cap×2（8192×2）→ 整章废弃（防 max_tokens 被服务端忽略）
    let ws = temp_ws("runaway");
    let (ft, _) = FakeTransport::scripted(vec![Ok(envelope("# t", 100, 8192 * 2 + 1))]);
    let e = generate_with(ft, Some(ws.clone()), &input()).unwrap_err();
    assert!(e.contains("失控"), "{e}");
    // 废弃章不落台账（错误路径零副作用）
    assert!(!ws.join(".mang/llm-usage.json").exists());
}

// ---------------------------------------------------------------- 真实 API 冒烟（SKIP 门）

#[test]
fn real_api_smoke_if_key_present() {
    // issue #68 纪律 2：真实 LLM 调用必须有 SKIP 门（无 key 环境不红）
    let Ok(key) = std::env::var("BID_LLM_API_KEY") else {
        eprintln!("SKIP: ENV_MISSING: BID_LLM_API_KEY（真实通路冒烟不跑）");
        return;
    };
    if key.trim().is_empty() {
        eprintln!("SKIP: ENV_MISSING: BID_LLM_API_KEY（空值）");
        return;
    }
    let cfg = LlmConfig::resolve(None).expect("env 在场则配置有效").expect("key 在场");
    let ws = temp_ws("real-smoke");
    let mut gen: Box<dyn SectionGenerator> = Box::new(RealGenerator::new(cfg, Some(ws.clone())));
    let mut inp = input();
    inp.target_chars = 200; // 冒烟成本面——小目标
    let out = gen.generate(&inp).expect("真实通路冒烟应成功");
    assert!(!out.content.trim().is_empty(), "真实生成不应为空");
    // 台账落盘（真实 usage 计数 > 0）
    let usage: Value =
        serde_json::from_str(&std::fs::read_to_string(ws.join(".mang/llm-usage.json")).unwrap())
            .unwrap();
    assert_eq!(usage["calls"].as_u64().unwrap(), 1);
    assert!(usage["total_tokens"].as_u64().unwrap() > 0);
}
