//! B-1 · import_tender_files 工具契约测试（issue #11 任务池 B-1）。
//!
//! 双轨（issue #11 测试要求）：
//! - **借鉴轨**：真实夹具 fixtures/engine-borrowed/TR2/word-complex.docx
//!   （mdcore @2d01444 借鉴件）——统计面对照该目录 gen-log.txt
//!   （`word-complex bytes=25767 paras=13 tables=1`：tables=1 为两侧共同
//!   稳定锚点；paras=13 是引擎侧含空段口径，bid-parse 出 7 非空段 + 2 表行
//!   = 9 块——确定性解析纪律下同输入同输出，精确断言成立）。
//! - **自研轨**：参数校验 / 状态机（合法追加 + 越级拒绝）/ sha256 幂等 /
//!   E_PARTIAL（截断 docx）/ 混合批次 / 乐观并发 / stdio 端到端。
//!
//! 并行安全：全部经 `dispatch_with` 注入独立 `BidService`（不共享全局态）；
//! 唯二例外——参数校验面（先于状态触达，无副作用）与 stdio 子进程（独立进程）。

use bid_mcp::{dispatch_json, dispatch_with, BidService, ToolCall, ToolReply};
use bid_domain::ProjectStatus;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixtures() -> PathBuf {
    std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // crates/bid-mcp → 仓库根
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
        })
}

fn word_complex() -> PathBuf {
    fixtures().join("engine-borrowed/TR2/word-complex.docx")
}

fn d1a_tables() -> PathBuf {
    fixtures().join("corpus-R/D1-tables/D1a-合并表头跨页表.docx")
}

fn r6a_insertion() -> PathBuf {
    fixtures().join("corpus-R/D6-revisions/R6a-纯插入澄清.docx")
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

fn err_code(reply: &ToolReply) -> String {
    match reply {
        ToolReply::Err { error, .. } => error.code().to_string(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

fn err_body(reply: &ToolReply) -> Value {
    match reply {
        ToolReply::Err { error, .. } => serde_json::to_value(error).unwrap(),
        ToolReply::Ok(out) => panic!("预期错误，实际成功: {}", out.result),
    }
}

/// 建项目（成功断言内联——create 是 import 的前置，失败即测试无效）
fn create(svc: &mut BidService, external_key: &str, deadline: &str) -> Value {
    let r = ok_result(call(svc, "create_bid_project", json!({
        "name": "B-1 契约测试项目", "bid_deadline": deadline, "mode": "明标", "external_key": external_key,
    })));
    assert_eq!(r["status"], "DRAFT", "新项目必须从 DRAFT 起步：{r}");
    r
}

/// 截断 docx 夹具（真实 word-complex.docx 前 1200 字节——EOCD 缺失 →
/// zip 防护拒绝，稳定的确定性失败面）
fn truncated(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("b1_mcp_truncated_{tag}_{}.docx", std::process::id()));
    let bytes = std::fs::read(word_complex()).expect("word-complex.docx 夹具缺失");
    std::fs::write(&p, &bytes[..1200]).unwrap();
    p
}

// ---------------------------------------------------------------- 1. 参数校验

/// 任务测试 1：缺 project_id / files 空 / filename 缺失 → E_UNPROCESSABLE
/// （+ 契约 §2 in 形状的其余校验面）。
#[test]
fn param_validation_rejects_malformed_import_args() {
    let mut svc = BidService::new();
    let wc = word_complex().display().to_string();
    let cases: Vec<(Value, &str)> = vec![
        (json!({}), "缺 project_id"),
        (json!({"files": []}), "缺 project_id 且 files 空"),
        (json!({"project_id": "PRJ-0001"}), "缺 files"),
        (json!({"project_id": "PRJ-0001", "files": []}), "files 空批次"),
        (json!({"project_id": "PRJ-0001", "files": "not-array"}), "files 非数组"),
        (
            json!({"project_id": "PRJ-0001", "files": [{"filename": "a.docx"}]}),
            "缺 upload_ref",
        ),
        (
            json!({"project_id": "PRJ-0001", "files": [{"upload_ref": wc}]}),
            "缺 filename",
        ),
        (
            json!({"project_id": "PRJ-0001", "files": [{"upload_ref": wc, "filename": "a.docx", "kind": "banana"}]}),
            "kind 越出契约七类枚举",
        ),
        (
            json!({"project_id": "PRJ-0001", "project_version": "one", "files": [{"upload_ref": wc, "filename": "a.docx"}]}),
            "project_version 非整数",
        ),
    ];
    for (args, why) in cases {
        let reply = call(&mut svc, "import_tender_files", args);
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "{why} 必须被拒");
    }
    // 参数校验先于状态触达：服务态未被污染（仍无项目）
    assert!(svc.store().project().is_err());
}

/// create_bid_project 参数校验（契约 §1）：mode 枚举 / deadline 形状 / 必填缺失。
#[test]
fn param_validation_rejects_malformed_create_args() {
    let mut svc = BidService::new();
    let cases: Vec<Value> = vec![
        json!({"bid_deadline": "2099-01-01", "mode": "明标", "external_key": "k"}),
        json!({"name": "n", "mode": "明标", "external_key": "k"}),
        json!({"name": "n", "bid_deadline": "2099-01-01", "external_key": "k"}),
        json!({"name": "n", "bid_deadline": "2099-01-01", "mode": "暗标?", "external_key": "k"}),
        json!({"name": "n", "bid_deadline": "不是日期", "mode": "暗标", "external_key": "k"}),
    ];
    for args in cases {
        let reply = call(&mut svc, "create_bid_project", args.clone());
        assert_eq!(err_code(&reply), "E_UNPROCESSABLE", "args={args}");
    }
    assert!(svc.store().project().is_err(), "参数拒绝必须零副作用");
}

// ---------------------------------------------------------------- 2. 项目路由

/// 无项目 / project_id 不符 → E_NOTFOUND（不伪造已存储的项目）。
#[test]
fn import_without_or_mismatched_project_is_not_found() {
    let mut svc = BidService::new();
    let wc = word_complex().display().to_string();
    // 无项目
    let reply = call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": wc, "filename": "a.docx"}],
    }));
    assert_eq!(err_code(&reply), "E_NOTFOUND");
    // 有项目但 id 不符
    create(&mut svc, "ext-route", "2099-01-01");
    let reply = call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-9999",
        "files": [{"upload_ref": wc, "filename": "a.docx"}],
    }));
    assert_eq!(err_code(&reply), "E_NOTFOUND");
    assert!(svc.store().files().is_empty(), "路由失败必须零副作用");
}

// ---------------------------------------------------------------- 3. 真实 docx 主线（借鉴轨）

/// 任务测试 4 + 验收硬标准：真实 docx 导入成功，DRAFT → FILES_IMPORTED，
/// per_file 统计非零。借鉴锚点：gen-log.txt `word-complex ... tables=1`。
#[test]
fn real_docx_import_transitions_draft_to_files_imported() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-real-docx", "2099-01-01");
    let wc = word_complex().display().to_string();

    let data = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": wc, "filename": "word-complex.docx", "kind": "招标文件"}],
    })));

    // 状态机：DRAFT → FILES_IMPORTED（服务态真源 + 响应回执双口径）
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::FilesImported);
    assert_eq!(data["status"], "FILES_IMPORTED");
    // §2 out：file_ids / per_file / totals
    // （DomainStore id 序列全前缀共享：create 占 PRJ-0001 → 首件文件档为 TF-0002，
    //  与 runner 场景产物的既有口径一致）
    assert_eq!(data["file_ids"], json!(["TF-0002"]));
    let pf = &data["per_file"][0];
    assert_eq!(pf["file_id"], "TF-0002");
    assert_eq!(pf["filename"], "word-complex.docx");
    assert_eq!(pf["parse_status"], "ok");
    assert_eq!(pf["ocr_tier"], "none", "docx 文本层在位——无需 OCR");
    assert_eq!(pf["deduplicated"], false);
    let sha = pf["sha256"].as_str().unwrap();
    assert_eq!(sha.len(), 64, "sha256 十六进制 64 字符");
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()), "sha256 须为十六进制：{sha}");
    // 借鉴锚点（gen-log.txt）：tables=1 为两侧共同稳定统计；
    // chunks=9 = 7 非空段 + 2 表行（引擎侧 paras=13 含空段口径，见文件头注）
    assert_eq!(pf["tables"], 1, "统计面对照 gen-log.txt：word-complex tables=1");
    assert_eq!(pf["chunks"], 9);
    // B5 起 docx 走引擎路径：pages=0（engine-bid-mapping §1 既定口径——
    // 旧「结构页 1+章标题数」为旧实现启发式，引擎不做页数推断）
    assert_eq!(pf["pages"], 0, "engine-docx 页数口径：0（映射表 §1）");
    assert!(pf.get("errors").is_none(), "成功件不带 errors 字段（条件序列化）");
    // totals 对账
    assert_eq!(data["totals"]["files"], 1);
    assert_eq!(data["totals"]["chunks"], 9);
    assert_eq!(data["totals"]["parse_failed"], 0);
    assert_eq!(data["totals"]["partial"], 0);
    // 包装字段（契约 §0：project_version / request_id / warnings）
    assert_eq!(data["project_version"], 2, "create=1，首件导入=2");
    assert!(!data["request_id"].as_str().unwrap_or("").is_empty());
    assert_eq!(data["warnings"], json!([]), "DRAFT 首导无 dirty 面");
    // 落库真源（DomainStore）
    let f = &svc.store().files()[0];
    assert_eq!((f.kind.as_str(), f.name.as_str()), ("招标文件", "word-complex.docx"));
    assert_eq!(svc.parsed().len(), 1, "成功件入解析累积（M1-b 抽取输入面）");
}

// ---------------------------------------------------------------- 4. sha256 幂等

/// 任务测试 3：同一文件导两次 → 同 file_id + deduplicated:true，no-op 不推版本。
#[test]
fn same_sha_reimport_is_noop_deduplicated() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-dedup", "2099-01-01");
    let wc = word_complex().display().to_string();
    let args = json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": wc, "filename": "word-complex.docx", "kind": "招标文件"}],
    });
    let first = ok_result(call(&mut svc, "import_tender_files", args.clone()));
    let v_after_first = svc.store().content_version();

    let second = ok_result(call(&mut svc, "import_tender_files", args));
    assert_eq!(second["file_ids"], first["file_ids"], "幂等重放必须返回原 file_id");
    assert_eq!(second["per_file"][0]["deduplicated"], true);
    assert_eq!(second["per_file"][0]["file_id"], "TF-0002");
    assert_eq!(svc.store().files().len(), 1, "无重复文件记录");
    assert_eq!(svc.store().content_version(), v_after_first, "no-op 不 bump content_version");
    // 纯 no-op 重放（全部条目 deduplicated）不标 dirty——无物可脏
    assert_eq!(second["warnings"], json!([]));
    assert_eq!(second["totals"]["files"], 1);
}

/// 单批次内同 sha 双条目：第二条按已落库档 dedup。
#[test]
fn duplicate_upload_within_single_batch_dedups() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-batch-dup", "2099-01-01");
    let wc = word_complex().display().to_string();
    let data = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [
            {"upload_ref": wc, "filename": "word-complex.docx", "kind": "招标文件"},
            {"upload_ref": wc, "filename": "重名再传.docx", "kind": "补遗"},
        ],
    })));
    assert_eq!(data["per_file"][0]["deduplicated"], false);
    assert_eq!(data["per_file"][1]["deduplicated"], true, "批内后条命中先条落库的 sha");
    assert_eq!(data["per_file"][1]["file_id"], data["per_file"][0]["file_id"]);
    assert_eq!(svc.store().files().len(), 1, "同 sha 只落一档");
    assert_eq!(data["totals"]["files"], 2, "响应面对账两条目（请求侧口径）");
}

// ---------------------------------------------------------------- 5. 状态机

/// 任务测试 2a：非 DRAFT 合法追加 → 允许 + dirty 告警（不回退状态）。
/// 构造：DRAFT → FILES_IMPORTED →（set_requirements）→ REQUIREMENTS_EXTRACTED，
/// 追加导入在两个非 DRAFT 态都合法——以最强态（REQUIREMENTS_EXTRACTED）验证。
#[test]
fn post_draft_append_allowed_with_dirty_warning() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-dirty", "2099-01-01");
    let d1a = d1a_tables().display().to_string();
    let r6a = r6a_insertion().display().to_string();
    ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": d1a, "filename": "D1a-合并表头跨页表.docx", "kind": "招标文件"}],
    })));
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::FilesImported);
    let v_before_append = svc.store().content_version();

    // 推进到 REQUIREMENTS_EXTRACTED（M1-b 工具未接线——走领域公开面，公有 API 可达的最深态）
    svc.store_mut().set_requirements(vec![]).unwrap();
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::RequirementsExtracted);

    // 非 DRAFT 追加：允许 + DIRTY_APPEND 告警 + 版本推进 + 状态不回退
    let data = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": r6a, "filename": "R6a-纯插入澄清.docx", "kind": "补遗"}],
    })));
    assert_eq!(data["status"], "REQUIREMENTS_EXTRACTED", "追加不得回退状态机");
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::RequirementsExtracted);
    let warns = data["warnings"].as_array().unwrap();
    assert!(
        warns.iter().any(|w| w.as_str().unwrap().contains("DIRTY_APPEND")),
        "非 DRAFT 追加必须标 dirty：{warns:?}"
    );
    assert!(svc.store().content_version() > v_before_append, "真追加必须推版本");
    assert_eq!(svc.store().files().len(), 2);
    assert_eq!(data["per_file"][0]["parse_status"], "ok");
}

/// 任务测试 2b：真正非法转换 → E_STATE（guard_tool 裁决，零副作用）。
///
/// import 自身的越级面（MATRIX_READY+ 追加）在 M2+ 工具接线前经公有 API
/// 不可达（set_requirements 最深只到 REQUIREMENTS_EXTRACTED——import 在该态
/// 仍合法）。本测以同一 `guard_tool` 通道构造可达的非法转换：FILES_IMPORTED
/// 态再建**不同** external_key 的项目（create 不在该态工具面）→ E_STATE；
/// 裁决表本身由 bid-domain tc11_state_guard_rejects_skip_ahead 覆盖。
#[test]
fn genuinely_illegal_transition_rejected_estate_zero_side_effect() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-estate", "2099-01-01");
    let wc = word_complex().display().to_string();
    ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": wc, "filename": "word-complex.docx", "kind": "招标文件"}],
    })));
    let (v_before, files_before) = (svc.store().content_version(), svc.store().files().len());

    let reply = call(&mut svc, "create_bid_project", json!({
        "name": "越级项目", "bid_deadline": "2099-01-01", "mode": "明标", "external_key": "ext-OTHER",
    }));
    assert_eq!(err_code(&reply), "E_STATE");
    // 零副作用：状态 / 版本 / 文件面均不动
    assert_eq!(svc.store().project().unwrap().status, ProjectStatus::FilesImported);
    assert_eq!(svc.store().content_version(), v_before);
    assert_eq!(svc.store().files().len(), files_before);

    // 对照：同 external_key 重放不是状态转换——静默幂等（契约 §1），不触发 E_STATE
    let replay = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "B-1 契约测试项目", "bid_deadline": "2099-01-01", "mode": "明标", "external_key": "ext-estate",
    })));
    assert_eq!(replay["deduplicated"], true);
    assert_eq!(replay["status"], "FILES_IMPORTED");
    assert_eq!(svc.store().content_version(), v_before, "幂等重放零副作用");
}

// ---------------------------------------------------------------- 6. E_PARTIAL 面

/// 任务测试 5：解析失败件（截断 docx）→ E_PARTIAL；失败件显式登记禁引，
/// 成功面统计随 error.data 携带（R-01：错误码不吞成功面）。
#[test]
fn truncated_docx_single_failure_is_epartial() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-trunc", "2099-01-01");
    let bad = truncated("single").display().to_string();

    let reply = call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": bad, "filename": "truncated.docx", "kind": "招标文件"}],
    }));
    assert_eq!(err_code(&reply), "E_PARTIAL");
    let e = err_body(&reply);
    let item_errors = e["item_errors"].as_array().unwrap();
    assert_eq!(item_errors.len(), 1);
    assert!(item_errors[0].as_str().unwrap().contains("truncated.docx"), "{item_errors:?}");
    assert!(e["hint"].is_string(), "契约错误形状 hint? 落位");
    // data：失败件也有 file_id（Failed 档显式登记）+ totals 对账
    let data = &e["data"];
    assert_eq!(data["file_ids"], json!(["TF-0002"]));
    assert_eq!(data["per_file"][0]["parse_status"], "failed");
    assert_eq!(data["per_file"][0]["ocr_tier"], "n/a");
    assert!(!data["per_file"][0]["errors"].as_array().unwrap().is_empty());
    assert_eq!(data["totals"]["files"], 1);
    assert_eq!(data["totals"]["parse_failed"], 1);
    assert_eq!(data["totals"]["partial"], 1);
    // 落库真源：Failed 档在位（禁引登记），状态推进语义沿用 domain 现行
    assert_eq!(svc.store().files().len(), 1);
    assert!(matches!(svc.store().files()[0].parse_status, bid_domain::ParseStatus::Failed));
    assert!(svc.parsed().is_empty(), "失败件不进解析累积（禁引）");
}

/// 任务测试 6：多文件混合（好 + 坏）→ totals.partial 统计正确，
/// 好件正常落库可用。
#[test]
fn mixed_good_and_bad_batch_totals_partial() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-mixed", "2099-01-01");
    let good = word_complex().display().to_string();
    let bad = truncated("mixed").display().to_string();

    let reply = call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [
            {"upload_ref": good, "filename": "word-complex.docx", "kind": "招标文件"},
            {"upload_ref": bad, "filename": "truncated.docx", "kind": "附件"},
        ],
    }));
    assert_eq!(err_code(&reply), "E_PARTIAL");
    let e = err_body(&reply);
    let data = &e["data"];
    // 好件先落 TF-0002（create 占 PRJ-0001），坏件 Failed 档 TF-0003——批次不中止（R-01）
    assert_eq!(data["file_ids"], json!(["TF-0002", "TF-0003"]));
    assert_eq!(data["per_file"][0]["parse_status"], "ok");
    assert_eq!(data["per_file"][1]["parse_status"], "failed");
    assert_eq!(data["totals"]["files"], 2);
    assert_eq!(data["totals"]["chunks"], 9, "chunks 只计好件（失败件 0）");
    assert_eq!(data["totals"]["parse_failed"], 1);
    assert_eq!(data["totals"]["partial"], 1);
    assert_eq!(e["item_errors"].as_array().unwrap().len(), 1);
    assert_eq!(svc.store().files().len(), 2);
    assert_eq!(svc.parsed().len(), 1, "只有好件进解析累积");
}

/// 已登记失败件的重放：仍 E_PARTIAL（同内容确定性重解析必同果）；
/// 修复后的新内容 = 新 sha = 新档（hint 指引的通路）。
#[test]
fn failed_file_reimport_stays_epartial_same_id() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-retry", "2099-01-01");
    let bad = truncated("retry").display().to_string();
    let args = json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": bad, "filename": "truncated.docx", "kind": "招标文件"}],
    });
    assert_eq!(err_code(&call(&mut svc, "import_tender_files", args.clone())), "E_PARTIAL");
    let v_after_first = svc.store().content_version();

    let reply = call(&mut svc, "import_tender_files", args);
    assert_eq!(err_code(&reply), "E_PARTIAL", "失败面重放不得翻绿");
    let data = err_body(&reply)["data"].clone();
    assert_eq!(data["per_file"][0]["deduplicated"], true);
    assert_eq!(data["per_file"][0]["file_id"], "TF-0002");
    assert_eq!(data["totals"]["partial"], 1);
    assert_eq!(svc.store().files().len(), 1, "失败档不重复落");
    assert_eq!(svc.store().content_version(), v_after_first);
}

/// 不可读 upload_ref：无文件身份（无 sha）——不落库、逐件失败、批次不中止。
#[test]
fn unreadable_upload_is_item_failure_without_record() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-unreadable", "2099-01-01");
    let good = word_complex().display().to_string();
    let ghost = fixtures().join("no-such-dir/ghost.docx").display().to_string();

    let reply = call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [
            {"upload_ref": good, "filename": "word-complex.docx", "kind": "招标文件"},
            {"upload_ref": ghost, "filename": "ghost.docx"},
        ],
    }));
    assert_eq!(err_code(&reply), "E_PARTIAL");
    let e = err_body(&reply);
    assert_eq!(e["item_errors"].as_array().unwrap().len(), 1);
    assert!(e["item_errors"][0].as_str().unwrap().contains("不可读"));
    let data = &e["data"];
    assert_eq!(data["file_ids"], json!(["TF-0002"]), "无身份件不占 file_id");
    assert_eq!(data["totals"]["files"], 1, "无身份件不入 files 统计（无档可报）");
    assert_eq!(data["totals"]["partial"], 1);
    assert_eq!(svc.store().files().len(), 1, "好件正常落库——批次未中止");
}

// ---------------------------------------------------------------- 7. 乐观并发

/// project_version 过期 → E_CONFLICT（零副作用）；携带正确版本 → 过。
#[test]
fn stale_project_version_conflicts_zero_side_effect() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-version", "2099-01-01");
    let wc = word_complex().display().to_string();
    let files = json!([{"upload_ref": wc, "filename": "word-complex.docx", "kind": "招标文件"}]);

    let reply = call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001", "project_version": 999, "files": files,
    }));
    assert_eq!(err_code(&reply), "E_CONFLICT");
    assert!(svc.store().files().is_empty(), "并发拒绝必须零副作用");
    assert_eq!(svc.store().content_version(), 1);

    // 缺省 = 内部步骤语义（跳过检查——M1 域口径）；显式正确版本 → 过
    let data = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001", "project_version": 1,
        "files": [{"upload_ref": wc, "filename": "word-complex.docx", "kind": "招标文件"}],
    })));
    assert_eq!(data["project_version"], 2);
}

// ---------------------------------------------------------------- 8. 对账告警

/// expected_pages 不符 → 逐件 EXPECTED_PAGES_MISMATCH 告警（不阻断）。
#[test]
fn expected_pages_mismatch_is_warning_not_error() {
    let mut svc = BidService::new();
    create(&mut svc, "ext-pages", "2099-01-01");
    let wc = word_complex().display().to_string();
    let data = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": wc, "filename": "word-complex.docx", "kind": "招标文件", "expected_pages": 999}],
    })));
    let warns = data["per_file"][0]["warnings"].as_array().unwrap();
    assert!(
        warns.iter().any(|w| w.as_str().unwrap().contains("EXPECTED_PAGES_MISMATCH")),
        "页数对账告警缺失：{warns:?}"
    );
    assert_eq!(data["totals"]["partial"], 0, "对账告警不是条目失败");
}

/// 契约 §1：deadline 已过 → 创建即警告 BID_DEADLINE_PAST（不阻断）。
#[test]
fn past_deadline_creates_with_warning() {
    let mut svc = BidService::new();
    let data = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "过期项目", "bid_deadline": "2020-01-01", "mode": "暗标", "external_key": "ext-past",
    })));
    assert_eq!(data["status"], "DRAFT");
    let warns = data["warnings"].as_array().unwrap();
    assert!(warns.iter().any(|w| w.as_str().unwrap().contains("BID_DEADLINE_PAST")), "{warns:?}");
}

// ---------------------------------------------------------------- 9. 只读面实况

/// get_bid_project_state 缺省读实况（B-1 起不再以 DRAFT 伪造）；显式
/// project_status 仍是 M0 纯函数面（假想态探测）。
#[test]
fn get_state_reads_live_service_by_default() {
    let mut svc = BidService::new();
    // 无项目 → E_NOTFOUND（不伪造）
    assert_eq!(err_code(&call(&mut svc, "get_bid_project_state", json!({}))), "E_NOTFOUND");
    // 建项 + 导入 → 实况
    create(&mut svc, "ext-state", "2099-01-01");
    let wc = word_complex().display().to_string();
    ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": "PRJ-0001",
        "files": [{"upload_ref": wc, "filename": "word-complex.docx", "kind": "招标文件"}],
    })));
    let data = ok_result(call(&mut svc, "get_bid_project_state", json!({})));
    assert_eq!(data["status"], "FILES_IMPORTED");
    assert!(data["allowed_next_tools"].as_array().unwrap().contains(&json!("extract_requirements")));
    // 显式带入 → 纯函数面（假想态探测，不触服务态）
    let probe = ok_result(call(&mut svc, "get_bid_project_state", json!({"project_status": "EXPORTED"})));
    assert_eq!(probe["terminal"], true);
}

// ---------------------------------------------------------------- 10. 传输面

/// dispatch_json 参数错误面（先于状态触达——全局态不受污染，并行安全）。
#[test]
fn dispatch_json_param_error_is_typed() {
    let out = dispatch_json(r#"{"tool":"import_tender_files","args":{}}"#);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "E_UNPROCESSABLE");
}

/// **验收硬标准（issue #11 B-1）**：stdio 子进程端到端——create_bid_project →
/// import_tender_files，真实 docx 导入成功，状态 DRAFT → FILES_IMPORTED。
/// 子进程独立全局态（并行安全；project_id 首件确定性地为 PRJ-0001）。
#[test]
fn stdio_end_to_end_real_docx_import() {
    let exe = env!("CARGO_BIN_EXE_bid-mcp-stdio");
    let fixture = word_complex();
    assert!(fixture.exists(), "真实夹具缺失：{fixture:?}");

    let create_line = serde_json::to_string(&ToolCall {
        tool: "create_bid_project".into(),
        args: json!({
            "name": "stdio 验收项目", "bid_deadline": "2099-01-01",
            "mode": "明标", "external_key": "ext-stdio-b1",
        }),
    })
    .unwrap();
    let import_line = serde_json::to_string(&ToolCall {
        tool: "import_tender_files".into(),
        args: json!({
            "project_id": "PRJ-0001",
            "files": [{"upload_ref": fixture.display().to_string(), "filename": "word-complex.docx", "kind": "招标文件"}],
        }),
    })
    .unwrap();

    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("stdio 子进程启动失败");
    {
        let mut stdin = child.stdin.take().unwrap();
        use std::io::Write;
        writeln!(stdin, "{create_line}").unwrap();
        writeln!(stdin, "{import_line}").unwrap();
    } // drop(stdin) → EOF → 子进程排空退出
    let out = child.wait_with_output().expect("子进程回收失败");
    assert!(out.status.success(), "stdio 进程异常退出：{out:?}");

    let replies: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("应答非 JSON（{e}）：{l}")))
        .collect();
    assert_eq!(replies.len(), 2, "两行请求必须有两行应答");
    assert_eq!(replies[0]["ok"], true, "create 应答：{replies:#?}");
    assert_eq!(replies[0]["data"]["status"], "DRAFT");
    assert_eq!(replies[0]["data"]["project_id"], "PRJ-0001");
    // import：真实 docx 导入成功 + 状态推进回执
    assert_eq!(replies[1]["ok"], true, "import 应答：{replies:#?}");
    let r = &replies[1]["data"];
    assert_eq!(r["status"], "FILES_IMPORTED", "DRAFT → FILES_IMPORTED");
    assert_eq!(r["file_ids"], json!(["TF-0002"]));
    assert_eq!(r["per_file"][0]["parse_status"], "ok");
    assert_eq!(r["per_file"][0]["tables"], 1, "gen-log.txt 借鉴锚点：tables=1");
    assert!(r["totals"]["chunks"].as_u64().unwrap() > 0, "真实解析统计非零");
}

// ---------------------------------------------------------------- B3-5：pdf 引擎路由

/// pdf 导入：引擎通道（pdfium 文本层）优先，缺 pdfium/扫描件回退旧 pdftotext
/// 路径——两条路都必须产出可抽取的文本层块（B1 设备清单件）。
#[test]
fn pdf_import_routes_engine_with_legacy_fallback() {
    let mut svc = BidService::new();
    let r = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "B3-5 pdf 路由测试", "bid_deadline": "2026-12-31", "mode": "明标",
        "external_key": "b35-pdf-route",
    })));
    let pid = r["project_id"].as_str().unwrap().to_string();
    let b1 = fixtures().join("corpus-A/files/B1-设备清单.pdf");
    let r = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": pid,
        "files": [{ "upload_ref": b1.display().to_string(), "filename": "B1-设备清单.pdf", "kind": "图纸" }],
    })));
    // 无论引擎通道还是回退：文本层件必须成功导入且块流非空（40 行设备表）
    let pf = &r["per_file"][0];
    assert_eq!(pf["parse_status"], "ok", "B1 文本层件导入成功（引擎或回退任一路径）：{r}");
    assert!(pf["chunks"].as_u64().unwrap() >= 30, "B1 设备清单块流非空（40 行量级）");
    assert!(svc.parsed().len() == 1, "成功件入解析累积");
    // 抽取链路（无论哪条解析路径，条款抽取面一致——B1 型号/中文日期行）
    let reqs = bid_parse::extract::extract_requirements(svc.parsed());
    assert!(!reqs.is_empty(), "B1 抽取面非空（型号/日期行条款）");
}
