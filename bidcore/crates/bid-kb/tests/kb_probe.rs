//! 25 条精确查询 × 真实夹具：检索排序规则的全量验证（判分器介入前的独立探针）。
//! 断言口径 = queries/exact-25.jsonl 的 expect（hit_text 包含 + hit_file 相符）。

use bid_kb::{ExactQuery, Kb};
use bid_parse::{extract::extract_requirements, parse_file};
use serde_json::Value;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures"))
}

#[test]
fn probe_exact_25_queries() {
    // issue #9：外部工具缺件 → SKIP（ENV_MISSING），不伪装成业务 FAIL
    if bid_parse::env_preflight::skip_if_env_missing(
        "probe_exact_25_queries",
        bid_parse::env_preflight::FULL_PDF_CHAIN,
    ) {
        return;
    }
    let fx = fixtures();
    let files_dir = fx.join("corpus-A/files");
    let mut names: Vec<(String, String)> = vec![
        ("A-招标文件.docx".into(), "A-招标文件.docx".into()),
        ("B1-设备清单.pdf".into(), "B1-设备清单.pdf".into()),
        ("B2-投标函格式页-扫描件.pdf".into(), "B2-投标函格式页-扫描件.pdf".into()),
        ("B3-图纸目录.xlsx".into(), "B3-图纸目录.xlsx".into()),
        ("C-补遗文件01.docx".into(), "C-补遗文件01.docx".into()),
    ];
    for e in std::fs::read_dir(files_dir.join("evidence")).unwrap() {
        let e = e.unwrap().path();
        if e.extension().and_then(|x| x.to_str()) == Some("pdf") {
            let fname = e.file_name().unwrap().to_string_lossy().to_string();
            names.push((format!("evidence/{fname}"), format!("evidence/{fname}")));
        }
    }
    let mut parsed = vec![];
    for (label, rel) in &names {
        let p = parse_file(&files_dir.join(rel), label).expect("解析失败");
        parsed.push(p);
    }
    let kb = Kb::ingest(&parsed, |p| p.name.clone());
    println!("chunks: {}", kb.chunks.len());
    let _ = extract_requirements(&parsed); // 仅验证检索面

    let queries: Vec<Value> = std::fs::read_to_string(fx.join("queries/exact-25.jsonl"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(queries.len(), 25);

    let mut misses = vec![];
    for qv in &queries {
        let q = ExactQuery {
            query_id: qv["query_id"].as_str().unwrap().to_string(),
            query_type: qv["query_type"].as_str().unwrap().to_string(),
            query: qv["query"].as_str().unwrap().to_string(),
        };
        let hits = kb.search_exact(&q);
        let top = hits.first();
        let expect_text = qv["expect"]["hit_text"].as_str().unwrap();
        let expect_file = qv["expect"]["hit_file"].as_str().unwrap();
        let ok = match top {
            Some(h) => {
                let contains = h.hit_text.contains(expect_text)
                    || bid_parse::norm_n1(&h.hit_text).contains(&bid_parse::norm_n1(expect_text));
                contains && h.file == expect_file
            }
            None => false,
        };
        if !ok {
            misses.push(format!(
                "{} {} {:?} -> top={:?} (want {} @ {})",
                q.query_id, q.query_type, q.query,
                top.map(|h| format!("{} | {}…", h.file, h.hit_text.chars().take(24).collect::<String>())),
                expect_text, expect_file
            ));
        }
    }
    for m in &misses {
        println!("MISS {m}");
    }
    assert!(misses.is_empty(), "{} 条未命中: {misses:?}", misses.len());
}
