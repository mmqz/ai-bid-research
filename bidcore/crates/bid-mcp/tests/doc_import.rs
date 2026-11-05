//! #270 .doc 导入两层设计——自动转换 + 无转换器引导 + OLE 改名识别。
//!
//! 三面验证（issue #270 验收 1-5）：
//! 1. **有转换器面**（本机 soffice 在位才真跑，否则 SKIP——CI 无转换器不伪装）：
//!    .doc 导入 → 自动转 docx → 解析成功 → 响应面标注「经转换」→ sha256 记原件
//!    （batch2 manifest 为锚）→ 转换产物落 `<ws>/.mang/convert/`
//! 2. **无转换器面**（`BID_NO_DOC_CONVERTER=1` 子进程注入——不在测试体 set_var，
//!    避并行竞态）：明确报错 + 两项检测结果 + 三种解决方式
//! 3. **改名面**：.doc 改名 .docx → OLE 魔数识别 → 报「旧格式」引导而非
//!    「容器损坏」；docx/pdf 正常路径零扰动

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    // #183 围栏：受信夹具根 canonical 绝对路径
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn doc_fixture() -> Option<PathBuf> {
    // fixtures/real-tender-batch2/doc-medical/（真实 .doc，552,309 字节 OLE2 容器；
    // manifest sha256 = ac8f48b77c017e647e8330a2c48dd49ee11a1d6c3c8d70b545dfbc99162b7302）
    let d = repo_root().join("fixtures/real-tender-batch2/doc-medical");
    let f = d.join("国家区域医疗中心建设项目-深部微波治疗仪等医疗设备采购项目公开招标文件-第2包.doc");
    f.is_file().then_some(f)
}

/// manifest 登记的原件 sha256（「sha256 记原件」断言锚）。
const DOC_MANIFEST_SHA: &str = "ac8f48b77c017e647e8330a2c48dd49ee11a1d6c3c8d70b545dfbc99162b7302";

fn temp_ws(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "doc-import-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok(svc: &mut BidService, tool: &str, args: Value) -> Value {
    match call(svc, tool, args) {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("{tool} 应成功但失败: {error}"),
    }
}

fn new_project(svc: &mut BidService, tag: &str) -> String {
    ok(svc, "create_bid_project", json!({
        "name": format!("doc导入-{tag}"),
        "bid_deadline": "2026-12-31",
        "mode": "明标",
        "external_key": format!("doc-import-{tag}"),
    }))["project_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn import_doc(svc: &mut BidService, pid: &str, path: &Path, filename: &str) -> ToolReply {
    call(
        svc,
        "import_tender_files",
        json!({
            "project_id": pid,
            "files": [{ "upload_ref": path.display().to_string(), "filename": filename, "kind": "招标文件" }],
        }),
    )
}

/// 提取 E_PARTIAL item_errors[0]（无转换器面的报错全文）。
fn first_item_error(reply: &ToolReply) -> String {
    use bid_mcp::ToolError;
    match reply {
        ToolReply::Ok(out) => panic!("应 E_PARTIAL 但成功: {:?}", out.result),
        ToolReply::Err { error, .. } => match error {
            ToolError::EPartial { item_errors, .. } => {
                item_errors.first().cloned().unwrap_or_default()
            }
            other => panic!("单件解析失败应 E_PARTIAL，实际: {other}"),
        },
    }
}

// ---------------------------------------------------------------- 面 1：有转换器自动转换

/// .doc 导入 → 自动转换 → 解析成功 + 标注 + sha 记原件 + 产物落 <ws>/.mang/convert/。
///
/// SKIP 纪律：soffice 不在位（CI）→ 横幅跳过，不伪装通过（issue #270 测试节）。
#[test]
fn doc_autoconvert_roundtrip() {
    let Some(doc) = doc_fixture() else {
        eprintln!("SKIP: fixtures/real-tender-batch2/doc-medical 不在仓（real-tender-batch2 未入库的克隆）");
        return;
    };
    if !bid_tools::service::doc_converter_available() {
        eprintln!("SKIP: 本机无 .doc 转换器（Word COM / LibreOffice 均不在场）——有转换器面按 #9 纪律跳过");
        return;
    }
    let ws = temp_ws("autoconvert");
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(doc.parent().unwrap());
    let pid = new_project(&mut svc, "autoconvert");

    let reply = import_doc(&mut svc, &pid, &doc, "doc-medical-第2包.doc");
    let out = match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!(".doc 自动转换导入应成功: {error}"),
    };
    let pf = &out["per_file"][0];
    assert_eq!(
        pf["sha256"].as_str().unwrap(),
        DOC_MANIFEST_SHA,
        "sha256 必须记原件（manifest 为锚）——不是转换产物的 sha"
    );
    assert_ne!(pf["chunks"].as_u64().unwrap_or(0), 0, "转换后解析应有内容块");
    // 响应面标注：per_file.warnings（结构化面）+ parse_summary（落库面）双通道
    let warnings = pf["warnings"].as_array().cloned().unwrap_or_default();
    assert!(
        warnings.iter().any(|w| w.as_str().unwrap_or("").contains("CONVERTED_FROM_DOC")),
        "per_file.warnings 必须标注「经 .doc→.docx 转换后解析」: {warnings:?}"
    );
    let summary = pf["parse_summary"].as_str().unwrap_or("");
    assert!(summary.contains(".doc→.docx"), "parse_summary 应带转换标注: {summary}");
    // 转换产物落工作区内部位（不写用户目录；sha 前 16 位命名——同件重导不重转）
    let convert_dir = ws.join(".mang").join("convert");
    let artifact = convert_dir.join(format!("{}.docx", &DOC_MANIFEST_SHA[..16]));
    assert!(artifact.is_file(), "转换产物应落 <ws>/.mang/convert/: {}", artifact.display());
    // 幂等面：重导同件 → deduplicated（且不重转——产物 mtime 不动由复用逻辑保证）
    let before = std::fs::metadata(&artifact).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let reply2 = import_doc(&mut svc, &pid, &doc, "doc-medical-第2包.doc");
    if let ToolReply::Ok(o) = &reply2 {
        assert_eq!(o.result["per_file"][0]["deduplicated"], json!(true), "同 sha 重导应幂等");
    }
    let after = std::fs::metadata(&artifact).unwrap().modified().unwrap();
    assert_eq!(before, after, "有效产物在位时不得重转（幂等复用）");
    let _ = std::fs::remove_dir_all(&ws);
}

// ---------------------------------------------------------------- 面 2：无转换器引导报错

/// 子进程 helper：`BID_NO_DOC_CONVERTER=1` 下 .doc 导入应报引导文案。
/// （env 经子进程注入——测试体内 set_var 有并行竞态，#156 env_preflight 教训）
#[test]
fn doc_without_converter_child() {
    // 断言 env 已由父测试注入（直接跑本测试时给出明确提示，而不是误判产品行为）
    if std::env::var_os("BID_NO_DOC_CONVERTER").is_none() {
        eprintln!("SKIP: 本测试为 doc_without_converter_reports_guidance 的子进程 helper（需 BID_NO_DOC_CONVERTER=1 注入）");
        return;
    }
    let Some(doc) = doc_fixture() else {
        eprintln!("SKIP: fixture 不在仓");
        return;
    };
    let ws = temp_ws("noconv-child");
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(doc.parent().unwrap());
    let pid = new_project(&mut svc, "noconv-child");
    let reply = import_doc(&mut svc, &pid, &doc, "doc-medical-第2包.doc");
    let msg = first_item_error(&reply);
    for needle in [
        "不支持直接解析 .doc",
        "Word COM",
        "LibreOffice",
        "安装 Microsoft Office 或 LibreOffice",
        "soffice --headless --convert-to docx",
        "请用户提供 .docx 版本",
    ] {
        assert!(msg.contains(needle), "引导报错缺「{needle}」: {msg}");
    }
    let _ = std::fs::remove_dir_all(&ws);
}

/// 主测试：子进程注入 `BID_NO_DOC_CONVERTER=1`（模拟无转换器环境——issue
/// 测试节建议的环境变量强制模拟），断言完整引导文案透出到 E_PARTIAL 面。
#[test]
fn doc_without_converter_reports_guidance() {
    let Some(doc) = doc_fixture() else {
        eprintln!("SKIP: fixtures/real-tender-batch2/doc-medical 不在仓");
        return;
    };
    let _ = doc; // fixture 在位即可（子进程内部再验）
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(&exe)
        .args(["doc_without_converter_child", "--exact", "--nocapture"])
        .env("BID_NO_DOC_CONVERTER", "1")
        .output()
        .expect("子进程测试启动失败");
    let tail = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "无转换器面子进程测试失败:\nstdout: {tail}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------- 面 3：OLE 改名识别

/// .doc 改名 .docx → OLE 魔数识别 → 报「旧格式」引导，不再「容器损坏」。
#[test]
fn doc_renamed_docx_reports_ole_not_corrupt() {
    let Some(doc) = doc_fixture() else {
        eprintln!("SKIP: fixtures/real-tender-batch2/doc-medical 不在仓");
        return;
    };
    let ws = temp_ws("ole-lie");
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(doc.parent().unwrap());
    let pid = new_project(&mut svc, "ole-lie");
    // 同内容、.docx 扩展名（upload_ref 原名已是 .doc——filename 撒谎不参与路由，
    // 需要物理改名拷贝）
    let liar = ws.join("doc-medical-改名件.docx");
    std::fs::copy(&doc, &liar).unwrap();
    let reply = import_doc(&mut svc, &pid, &liar, "doc-medical-改名件.docx");
    let msg = first_item_error(&reply);
    assert!(msg.contains("OLE2"), "应识别 OLE2 容器: {msg}");
    assert!(msg.contains("旧格式"), "应报「旧格式」: {msg}");
    assert!(
        !msg.contains("EOCD") && !msg.to_lowercase().contains("container corrupt"),
        "不得再报误导性的「容器损坏/EOCD」: {msg}"
    );
    // 状态机零推进（坏件不推进——#174/#175 面）
    let st = ok(
        &mut svc,
        "get_bid_project_state",
        json!({ "project_id": pid }),
    );
    assert_ne!(st["status"].as_str().unwrap(), "FILES_IMPORTED", "改名件失败不得推进状态机");
    let _ = std::fs::remove_dir_all(&ws);
}

/// 正常 docx/pdf 导入零扰动（回归面）：.doc 通道不碰既有路径。
#[test]
fn docx_pdf_import_unchanged() {
    let dir = repo_root().join("fixtures/synth-tender/招标文件");
    if !dir.is_dir() {
        eprintln!("SKIP: synth-tender fixtures 不在仓");
        return;
    }
    let docx = dir.join("A-测试招标文件.docx");
    let pdf = dir.join("A-测试招标文件.pdf");
    if !docx.is_file() || !pdf.is_file() {
        eprintln!("SKIP: synth-tender 主件缺失（docx={} pdf={}）", docx.display(), pdf.display());
        return;
    }
    let ws = temp_ws("regress");
    let mut svc = BidService::open(&ws).unwrap().with_fs_root(&dir);
    let pid = new_project(&mut svc, "regress");
    let reply = call(
        &mut svc,
        "import_tender_files",
        json!({
            "project_id": pid,
            "files": [
                { "upload_ref": docx.display().to_string(), "filename": "A-测试招标文件.docx", "kind": "招标文件" },
                { "upload_ref": pdf.display().to_string(), "filename": "A-测试招标文件.pdf", "kind": "附件" },
            ],
        }),
    );
    match reply {
        ToolReply::Ok(out) => {
            assert_eq!(out.result["totals"]["files"], json!(2), "docx+pdf 双件应全成功");
            for pf in out.result["per_file"].as_array().unwrap() {
                assert!(
                    !pf["warnings"].to_string().contains("CONVERTED_FROM_DOC"),
                    "非 .doc 件不得出现转换标注"
                );
            }
        }
        // 单件失败也可能因环境（pdftotext 缺失等）——关键断言是不出现转换语义
        ToolReply::Err { error, .. } => {
            assert!(
                !format!("{error}").contains("CONVERTED_FROM_DOC"),
                "docx/pdf 路径不得出现 .doc 转换语义: {error}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&ws);
}
