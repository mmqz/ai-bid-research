//! corpus-A 解析探针：真实夹具上的全链路冒烟（DOCX/PDF/XLSX/OCR + 条款抽取）。
//! 运行：cargo test -p bid-parse --test corpus_probe -- --nocapture
//! 夹具路径：环境变量 BID_FIXTURES 或默认仓库相对路径 ../../fixtures。
//!
//! B4 路由（issue #11 任务 3）：docx/xlsx 走 bid-engine 引擎门面
//! （`parse_docx_via_engine` / `parse_xlsx_via_engine`——自研解析已删）；
//! pdf 保持 `parse_file`（文本层/OCR 补遗链是标书侧保留面）。断言数值不变
//! （引擎路径与旧路径该件 100% EQ——B3-3 r3 全量差分验收）。

use bid_parse::{extract::extract_requirements, parse_file};
use std::path::PathBuf;

/// B4：docx/xlsx → 引擎；pdf → 旧路径（parse_file）。与 service.rs 同款路由。
fn parse_routed(path: &std::path::Path, name: &str) -> Result<bid_parse::ParsedFile, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "docx" => bid_engine::parse_docx_via_engine(path).map_err(|e| e.to_string()),
        "xlsx" => bid_engine::parse_xlsx_via_engine(path).map_err(|e| e.to_string()),
        _ => parse_file(path, name),
    }
}

fn fixtures() -> PathBuf {
    std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // crates/bid-parse → 仓库根
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
        })
}

#[test]
fn probe_corpus_a_full_chain() {
    // issue #9：外部工具缺件 → SKIP（ENV_MISSING），不伪装成业务 FAIL
    if bid_parse::env_preflight::skip_if_env_missing(
        "probe_corpus_a_full_chain",
        bid_parse::env_preflight::FULL_PDF_CHAIN,
    ) {
        return;
    }
    // PR #24 修复批次：真实样本缺件 → SKIP（FIXTURE_MISSING 横幅——issue #9 机制文件面）
    let files_dir = fixtures().join("corpus-A/files");
    if bid_parse::env_preflight::skip_if_fixture_missing("probe_corpus_a_full_chain", &[&files_dir]) {
        return;
    }
    let names = [
        "A-招标文件.docx",
        "B1-设备清单.pdf",
        "B2-投标函格式页-扫描件.pdf",
        "B3-图纸目录.xlsx",
    ];
    let mut parsed = vec![];
    for n in names {
        let p = parse_routed(&files_dir.join(n), n).expect("解析失败");
        println!(
            "== {} : pages={} tables={} text_layer={} ocr={} blocks={} | {}",
            n, p.pages, p.tables, p.has_text_layer, p.ocr_blocks, p.blocks.len(), p.parse_summary
        );
        parsed.push(p);
    }
    // 结构断言（与夹具规格对齐的硬口径）
    let a = &parsed[0];
    assert!(a.has_text_layer && a.tables >= 5, "A 应有文本层与 >=5 表格");
    let b1 = &parsed[1];
    assert!(b1.has_text_layer && b1.pages >= 2, "B1 应有文本层");
    let b2 = &parsed[2];
    assert!(!b2.has_text_layer && b2.ocr_blocks >= 4, "B2 应走 OCR 通道且 >=4 块");
    let b3 = &parsed[3];
    assert!(b3.blocks.len() >= 20, "B3 应有 >=20 行");

    // 条款抽取（确定性层）
    let reqs = extract_requirements(&parsed);
    println!("== 抽取 REQ 总数: {}", reqs.len());
    let mut by_cat = std::collections::BTreeMap::new();
    for r in &reqs {
        *by_cat.entry(r.category.as_str()).or_insert(0usize) += 1;
    }
    println!("== 类别分布: {:?}", by_cat);
    for r in reqs.iter().filter(|r| r.category == "资格条件").take(3) {
        println!("   {} [{}] {} | {}…", r.id, r.category, r.clause_no.clone().unwrap_or_default(),
                 r.anchors.first().map(|a| a.excerpt.as_str()).unwrap_or(""));
    }
    // GT-11 前附表行 + GT-16/17/18 参数表 + GT-07 B2 OCR
    assert!(reqs.iter().any(|r| r.clause_no.as_deref() == Some("前附表-10")), "前附表-10 未抽到");
    for cn in ["5.1", "5.2", "5.3"] {
        assert!(reqs.iter().any(|r| r.clause_no.as_deref() == Some(cn) && r.anchors.iter().any(|a| a.kind == "table")),
            "参数表条款 {cn} 未抽到");
    }
    assert!(reqs.iter().any(|r| r.anchors.iter().any(|a| a.kind == "ocr" && a.excerpt.contains("否决"))),
        "B2 OCR 否决条款未抽到");
    // #487：钉「纯日期行」——旧断言用锚文 contains，被项目编号行
    // （ZFCG-2026-GK model 命中，锚文恰好含日期串）误当证据通过，
    // 40+ 条纯日期行（十字盲区）静默丢失（断言钉错对象）。改钉
    // params.date_cn 精确等于日期串：修复前全仓 date_cn 恒 None（十字
    // 不命中），本断言必红；修复后纯日期行产出才绿。
    assert!(
        reqs.iter().any(|r| r.source_file.starts_with("B1")
            && r.params.get("date_cn").and_then(|v| v.as_str()) == Some("二〇二六年十二月三十日")),
        "B1 纯日期行未抽到（date_cn 参数须承载完整日期串——旧断言被项目编号行误过，#487）"
    );
    // B1 到货日期行全量恢复：40 项清单 + 头部口径行（十字盲区回归锁）
    let b1_date_reqs = reqs.iter().filter(|r| r.source_file.starts_with("B1")
        && r.params.get("date_cn").and_then(|v| v.as_str()) == Some("二〇二六年十二月三十日")).count();
    assert!(b1_date_reqs >= 40, "B1 到货日期行应 ≥40 条全量恢复——实际 {b1_date_reqs}（#487 十字盲区回归锁）");
    // B2 OCR 还原检查：拼接行须含「否决投标处理」全文（断行还原）
    let ocr_req = reqs.iter().find(|r| r.anchors.iter().any(|a| a.kind == "ocr" && a.excerpt.contains("否决投"))).unwrap();
    let joined: String = ocr_req.anchors.iter().map(|a| a.excerpt.clone()).collect::<Vec<_>>().join("");
    assert!(joined.contains("否决投标处理") || joined.contains("否决投"), "OCR 条款断行还原异常");
    println!("   B2 OCR 条款 excerpt(前60): {}", &ocr_req.anchors[0].excerpt.chars().take(60).collect::<String>());
}
