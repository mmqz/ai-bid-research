//! 坏输入矩阵——**程序生成的坏件**导入，逐类验证「坏件绝不静默产出成功的空标书」。
//!
//! 为什么单开：`input_format_matrix.rs` 已覆盖「格式路由 + 损坏形态 + 零副作用」，
//! 但它的损坏形态断言是**软口径**（`ok_n == 0 || n > 0` 之类——坏件被判成功也
//! 未必红）。本文件把这些断言收紧成**唯一红线**：
//!
//! > 坏件导入的结果只有两种合法形态——
//! > ① 信封层返回 E_* 错误码（现实路径是 `E_PARTIAL`，契约 §2）；
//! > ② 返回 Ok 但 per_file 里**逐件登记**了该件的失败（parse_status=failed /
//! >    errors 非空 / totals.partial 或 parse_failed > 0）。
//! >
//! > 二者皆无 = **静默成功的空标书**，直接红。
//!
//! 「静默成功的空标书」为什么是废标级事故：坏件一旦被当成成功件落库，下游
//! `extract_requirements` / `get_compliance_matrix` 会基于**零条款**得出
//! blocker=0，校验全绿——「不知道招标文件没读进来」与「招标文件没问题」
//! 在界面上长得一模一样（issue #9 同款「失败伪装成成功」）。
//!
//! 夹具纪律：**坏件全部程序生成**（tempdir 写字节）——CI 不依赖任何外部文件，
//! 跑得起来也不需要样本分发。唯一的外部依赖是「超大页数真件」（用户机器上的
//! 1550/665/437 页 PDF），文件不在时 SKIP（issue #9 口径：缺件不是业务失败）——
//! 仓库里只登记 manifest，不放实体文件。
//!
//! helper 复用 `input_format_matrix.rs`（temp_ws / new_project / import /
//! call / ok 同款写法）——不另造轮子。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolError, ToolReply};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("fixtures/synth-tender/招标文件")
}

/// 临时工作区（每测试独立——并行安全）。
fn temp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bad-input-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(
        svc,
        ToolCall {
            tool: tool.to_string(),
            args,
        },
    )
}

fn ok(svc: &mut BidService, tool: &str, args: Value) -> Value {
    match call(svc, tool, args) {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("{tool} 应成功但失败: {error}"),
    }
}

fn new_project(svc: &mut BidService, tag: &str) -> String {
    ok(
        svc,
        "create_bid_project",
        json!({
            "name": format!("坏输入矩阵-{tag}"),
            "bid_deadline": "2026-12-31",
            "mode": "明标",
            "external_key": format!("bad-input-{tag}"),
        }),
    )["project_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn import(svc: &mut BidService, pid: &str, path: &Path, name: &str, kind: &str) -> ToolReply {
    call(
        svc,
        "import_tender_files",
        json!({
            "project_id": pid,
            "files": [{
                "upload_ref": path.display().to_string(),
                "filename": name,
                "kind": kind,
            }],
        }),
    )
}

// ---------------------------------------------------------------- 坏件生成器

/// 确定性伪随机字节（LCG——不引 rand：同输入同输出，测试可复现）。
///
/// 不含 `%PDF-`（连续 5 字节撞上的概率可忽略）：保证「非 PDF 头」这条走的是
/// **内容不是 PDF** 的失败分支，而不是碰巧混进 PDF 魔数的分支。
fn junk_bytes(n: usize, seed: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    let mut x = seed;
    for _ in 0..n {
        x = x.wrapping_mul(131).wrapping_add(17);
        out.push(x);
    }
    out
}

/// 合法 ZIP 容器，但内部**不是 OOXML**（无 [Content_Types].xml /
/// word/document.xml）——钉的是「容器合法 ≠ 内容合法」这条分支。
///
/// 用 `zip` crate 现造（bid-mcp dev-dep 已钉 `zip = "4"`，与 bid-engine 同款；
/// Cargo.lock 既有条目，零新增网络取数）。
fn build_valid_zip_non_ooxml() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        // 一个普通文本文件——ZIP 结构完全合法，与 OOXML 毫无关系
        zw.start_file("readme.txt", stored).expect("zip start_file");
        zw.write_all(b"this is a perfectly valid zip, and definitely not a docx")
            .expect("zip write");
        // 再放一个假 OOXML 部件名（诱骗「按部件名嗅探」的实现——内部不是 OOXML）
        zw.start_file("word/document.xml", stored)
            .expect("zip start_file 2");
        zw.write_all(b"<not-ooxml at all>").expect("zip write 2");
        zw.finish().expect("zip finish");
    }
    buf.into_inner()
}

/// 在工作区里落一个坏件（返回路径）。
fn write_bad(ws: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = ws.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

// ---------------------------------------------------------------- 判定核心

/// 一次坏件导入的判定结果。
struct Verdict {
    /// 坏件类别（人读——失败信息里直接点名哪一类漏网）。
    label: String,
    /// 信封层错误码（`Err` 分支；`Ok` 分支为 `None`）。
    code: Option<String>,
    /// 逐件失败清单（E_PARTIAL 的 `item_errors`；`Ok` 分支为空）。
    item_errors: Vec<String>,
    /// 成功面（`Ok` 的 result，或 E_PARTIAL 随错误携带的 `data`）。
    data: Value,
    /// 红线：坏件被判成成功且**任何地方都没有失败登记**。
    silent_success: bool,
}

/// 把一次导入的 `ToolReply` 归成 `Verdict`（唯一判定点——各类坏件共用，
/// 免得每类各写一份松紧不一的断言）。
///
/// 契约 §2（tools.rs:491）：`item_failures > 0` 一律 `Err(E_PARTIAL)` 且 `data`
/// 携带成功面——故 `Ok` 分支**不可能**携带失败登记；反过来，坏件走 `Ok` 就是
/// 静默成功。
fn classify(label: &str, reply: ToolReply) -> Verdict {
    match reply {
        ToolReply::Ok(out) => {
            let registered = failure_registered_in(&out.result);
            Verdict {
                label: label.to_string(),
                code: None,
                item_errors: Vec::new(),
                data: out.result.clone(),
                silent_success: !registered,
            }
        }
        ToolReply::Err { error, .. } => {
            // 错误码一律取自 `ToolError::code()`（契约字面量——不解析 message）
            let code = error.code().to_string();
            let (item_errors, data) = match &error {
                ToolError::EPartial { item_errors, data, .. } => {
                    (item_errors.clone(), data.clone().unwrap_or(Value::Null))
                }
                _ => (Vec::new(), Value::Null),
            };
            Verdict {
                label: label.to_string(),
                code: Some(code),
                item_errors,
                data,
                silent_success: false, // 信封层已响亮拒绝——不是静默
            }
        }
    }
}

/// data/result 面是否**显式登记了失败**（逐件核对面：任一命中即算登记）。
fn failure_registered_in(data: &Value) -> bool {
    if data["totals"]["partial"].as_u64().unwrap_or(0) > 0 {
        return true;
    }
    if data["totals"]["parse_failed"].as_u64().unwrap_or(0) > 0 {
        return true;
    }
    let Some(per_file) = data["per_file"].as_array() else {
        return false;
    };
    per_file.iter().any(|f| {
        f["parse_status"].as_str() == Some("failed")
            || f["errors"].as_array().is_some_and(|e| !e.is_empty())
    })
}

/// 红线断言：坏件**不得**静默成功。
fn assert_bad_input_rejected(v: &Verdict) {
    assert!(
        !v.silent_success,
        "坏件【{}】被当成成功导入——红线（坏件 → 静默产出成功的空标书）。
         判定：信封层没返回 E_* 错误码，且返回面任何地方都没有逐件失败登记         （totals.partial=0、totals.parse_failed=0、per_file 无 failed/errors 条目）。
         这一件随后会被下游当成已读进来的招标文件，抽条款得 blocker=0、校验全绿。
         返回面 = {}",
        v.label,
        serde_json::to_string_pretty(&v.data).unwrap_or_else(|_| format!("{:?}", v.data))
    );
    match &v.code {
        Some(c) => eprintln!("  坏件【{}】→ 信封层拒绝：{c}", v.label),
        None => eprintln!("  坏件【{}】→ Ok + per_file 逐件登记失败", v.label),
    }
}

// ---------------------------------------------------------------- 1. 0 字节文件

/// 0 字节文件（.docx / .pdf 两臂）→ 明确失败，**绝不**产出 chunks=0 的「成功」件。
///
/// 这条是四类里最要命的：0 字节最容易被「读成空文档」而当成功入库。
#[test]
fn zero_byte_files_are_never_silent_success() {
    let ws = temp_ws("zero");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "zero");

    for (name, label) in [("空文件.docx", "0 字节 docx"), ("空文件.pdf", "0 字节 pdf")] {
        let empty = write_bad(&ws, name, b"");
        let v = classify(label, import(&mut svc, &pid, &empty, name, "招标文件"));
        assert_bad_input_rejected(&v);
    }
    // 【#174/#175】坏件导入后项目状态**不得**推进到 FILES_IMPORTED。
    // 失败件仍会落库（Failed + 禁引，便于重放审计），但「落库」≠「导入成功」。
    // 若坏件也推进状态，后续会认为「文件已导入」而允许 extract_requirements，
    // 可失败件是禁引的 → 基于空内容一路走到导出 → **产出空标书**（最危险形态）。
    // #175 已修（upsert_file 加条件 parse_status != Failed），此处钉死。
    let st = ok(&mut svc, "get_bid_project_state", json!({}));
    assert_eq!(
        st["status"].as_str(),
        Some("DRAFT"),
        "坏件导入后状态应停在 DRAFT（不得推进到 FILES_IMPORTED）——#174/#175"
    );
}

// ---------------------------------------------------------------- 2. 截断的 docx

/// 截断的 docx（合法 PK 头 + 垃圾）——ZIP 中央目录不成立。
#[test]
fn truncated_docx_is_rejected() {
    let ws = temp_ws("trunc");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "trunc");

    // PK\x03\x04 + 垃圾——让 zip 解析（中央目录/EOCD）失败
    let mut bytes = vec![0x50, 0x4b, 0x03, 0x04];
    bytes.extend(junk_bytes(512, 0x21));
    let bad = write_bad(&ws, "截断.docx", &bytes);

    let v = classify("截断 docx", import(&mut svc, &pid, &bad, "截断.docx", "招标文件"));
    assert_bad_input_rejected(&v);
}

// ---------------------------------------------------------------- 3. 非 PDF 头

/// 随机字节命名成 .pdf（内容不是 PDF）——三条降级路（pdfium → pdftotext →
/// 纯 Rust 兜底）全断时必须**明说失败**，不许产出「0 页 0 块的空标书」。
#[test]
fn non_pdf_bytes_named_pdf_is_rejected() {
    let ws = temp_ws("notpdf");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "notpdf");

    let junk = junk_bytes(2048, 0x37);
    assert!(
        !junk.windows(5).any(|w| w == b"%PDF-"),
        "造件自检：伪随机字节不得含 %PDF- 魔数（否则测的不是「非 PDF 头」分支）"
    );
    let bad = write_bad(&ws, "其实是垃圾.pdf", &junk);

    let v = classify(
        "非 PDF 头（随机字节 .pdf）",
        import(&mut svc, &pid, &bad, "其实是垃圾.pdf", "招标文件"),
    );
    assert_bad_input_rejected(&v);
}

// ---------------------------------------------------------------- 4. 合法 ZIP 非 OOXML

/// 合法 ZIP 容器但内容不是 OOXML——「容器合法 ≠ 文档合法」。
///
/// 与「截断 docx」区分开：那条是容器层就断了，这条容器**完全合法**，
/// 断在 OPC 部件面（无 [Content_Types].xml / 非 OOXML 的 word/document.xml）。
#[test]
fn valid_zip_but_not_ooxml_is_rejected() {
    let ws = temp_ws("zipnotooxml");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "zipnotooxml");

    let bytes = build_valid_zip_non_ooxml();
    // 造件自检：确实是个合法 ZIP（否则这条测的还是「截断」面）
    assert!(
        zip::ZipArchive::new(std::io::Cursor::new(&bytes)).is_ok(),
        "造件自检：坏件应是合法 ZIP（本条测的是「容器合法但内容不是 OOXML」）"
    );
    let bad = write_bad(&ws, "合法zip.docx", &bytes);

    let v = classify(
        "合法 ZIP 但内容不是 OOXML",
        import(&mut svc, &pid, &bad, "合法zip.docx", "招标文件"),
    );
    assert_bad_input_rejected(&v);
}

// ---------------------------------------------------------------- 5. 混合批次

/// 1 好 1 坏混合批次：好件照走（入库 + 成功面可消费）、坏件逐件报错、
/// **不整批失败**（契约 §2 R-01：错误码不吞成功面）。
#[test]
fn mixed_batch_keeps_good_and_reports_bad() {
    let good = fixtures().join("A-招标文件.docx");
    if bid_parse::env_preflight::skip_if_fixture_missing(
        "mixed_batch_keeps_good_and_reports_bad",
        &[&good],
    ) {
        return;
    }

    let ws = temp_ws("mixed");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "mixed");
    let bad = write_bad(&ws, "坏件.docx", b"not a zip at all");

    let reply = call(
        &mut svc,
        "import_tender_files",
        json!({
            "project_id": pid,
            "files": [
                { "upload_ref": good.display().to_string(), "filename": "好件.docx", "kind": "招标文件" },
                { "upload_ref": bad.display().to_string(), "filename": "坏件.docx", "kind": "招标文件" },
            ],
        }),
    );

    // ① 含坏件 → 必须是 E_PARTIAL（成功面随 data 携带，不整批失败）
    let ToolReply::Err { error, .. } = &reply else {
        panic!(
            "混合批次含坏件却返回 Ok（契约 §2：item_failures>0 一律 E_PARTIAL）: {:?}",
            reply
        );
    };
    let ToolError::EPartial { data, item_errors, .. } = error else {
        panic!("混合批次应 E_PARTIAL，实际 {}", error.code());
    };
    let data = data.clone().unwrap_or(Value::Null);

    // ② 坏件逐件登记（点名到件——Agent 才能定位该重传哪一件）
    assert!(
        item_errors.iter().any(|e| e.contains("坏件")),
        "坏件未逐件登记（Agent 无从知道哪件要重传）: {item_errors:?}"
    );

    // ③ 好件照走：入库 + 解析成功 + 有内容（不是空壳成功）
    let per_file = data["per_file"].as_array().cloned().unwrap_or_default();
    let good_entry = per_file
        .iter()
        .find(|f| f["filename"].as_str() == Some("好件.docx"))
        .unwrap_or_else(|| panic!("好件未进 per_file（整批失败了）: {data}"));
    assert_eq!(
        good_entry["parse_status"].as_str(),
        Some("ok"),
        "好件未解析成功（坏件连累了整批）: {good_entry}"
    );
    assert!(
        good_entry["chunks"].as_u64().unwrap_or(0) > 0,
        "好件 chunks=0（成功面是空壳）: {good_entry}"
    );

    // ④ 统计面：坏件计入 parse_failed / partial，不混进成功面
    assert_eq!(
        data["totals"]["parse_failed"].as_u64().unwrap_or(0),
        1,
        "parse_failed 应恰 1（坏件）: {data}"
    );
    assert!(
        data["totals"]["partial"].as_u64().unwrap_or(0) >= 1,
        "totals.partial 应 >=1（R-01：错误码不吞成功面）: {data}"
    );
}

// ---------------------------------------------------------------- 6. 超大页数（外部真件，缺件 SKIP）

/// 超大页数真件（1550 / 665 / 437 页 PDF——用户机器上的坏件样本）。
///
/// 外部件不在时 SKIP（issue #9 口径：真实样本缺件不是业务失败）。
/// 断言口径比前四类宽：**允许解析失败**（CI 无 poppler/pdfium/tesseract 时
/// 必然失败），但**不允许「解析成功却是 0 页 0 块的空标书」**——那正是超大件
/// 最容易踩的静默面（页数门/OCR 页上限拒绝时若回落成空成功面）。
///
/// 仓库里只登记 manifest（本文件的常量清单），不放实体文件。
#[test]
fn oversized_pdf_never_silently_empty() {
    const DEFAULT_DIR: &str = r"C:Usersyezi6Downloadswork标书";
    // 候选按页数降序——取到任一即可（大页数件的压力面是同一条）
    const CANDIDATES: [&str; 3] = [
        "1550 页 中国电信大兴安岭分公司.pdf",
        "665 页 标书.pdf",
        "437 页 购置天通智能调度平台1套投标文件.pdf",
    ];
    let dir = std::env::var("BID_BAD_INPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_DIR));

    let picked = CANDIDATES.iter().map(|c| dir.join(c)).find(|p| p.is_file());
    let Some(path) = picked else {
        eprintln!();
        eprintln!("====================================================================");
        eprintln!("SKIP: oversized_pdf_never_silently_empty");
        eprintln!("reason: FIXTURE_MISSING: {}", dir.display());
        eprintln!("（超大页数真件不在本环境；清单：{}）", CANDIDATES.join(" / "));
        eprintln!("（外部真实件未随仓库分发——设 BID_BAD_INPUT_DIR 指向该目录即可启用本用例）");
        eprintln!("====================================================================");
        return;
    };
    eprintln!("超大页数件：{}", path.display());

    let ws = temp_ws("oversize");
    let mut svc = BidService::open(&ws).unwrap();
    let pid = new_project(&mut svc, "oversize");

    let v = classify(
        "超大页数 PDF",
        import(&mut svc, &pid, &path, "超大件.pdf", "招标文件"),
    );
    // 口径：解析失败可接受（环境缺件），静默成功不可接受
    assert_bad_input_rejected(&v);

    // 解析成功面（信封层没报错）：页数/块数不得为 0——0 页 0 块的「成功」
    // 就是超大件被页数门/OCR 上限拒掉后回落成的空标书
    if v.code.is_none() {
        let empty: Vec<Value> = Vec::new();
        let per_file = v.data["per_file"].as_array().unwrap_or(&empty);
        for f in per_file {
            let pages = f["pages"].as_u64().unwrap_or(0);
            let chunks = f["chunks"].as_u64().unwrap_or(0);
            assert!(
                pages > 0 && chunks > 0,
                "超大件解析「成功」却是 pages={pages} chunks={chunks}（空标书面——                 超大件被页数门拒绝后必须响亮失败，不得回落成空成功面）: {f}"
            );
        }
    }
}
