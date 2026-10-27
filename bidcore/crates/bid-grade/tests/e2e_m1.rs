//! M1 端到端：corpus-A 真实夹具 → 四场景运行（TC-01/02/07/11）→ bidgrade 判分全 PASS。
//! 这是「每轮测试」的主门禁：cargo test --workspace 的核心集成项。

use bid_grade::{fixtures::Fixtures, grade};
use bid_tools::runner::{run_scenario, write_run};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn e2e_m1_all_scenarios_pass() {
    // issue #9：外部工具缺件 → SKIP（ENV_MISSING），不伪装成业务 FAIL
    if bid_parse::env_preflight::skip_if_env_missing(
        "e2e_m1_all_scenarios_pass",
        bid_parse::env_preflight::FULL_PDF_CHAIN,
    ) {
        return;
    }
    let root = repo_root();
    let fixtures_dir = std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("fixtures"));
    let fx = Fixtures::load(&fixtures_dir).expect("夹具装载失败");
    let runs_dir = root.join("runs");
    let mut summaries = vec![];
    for scenario in ["TC-01", "TC-02", "TC-07", "TC-11"] {
        let out = run_scenario(scenario, &fixtures_dir)
            .unwrap_or_else(|e| panic!("{scenario} 运行失败: {e}"));
        write_run(&runs_dir, &out).unwrap_or_else(|e| panic!("{scenario} 产物写盘失败: {e}"));
        let report = grade::grade_run(&out.db, &fx);
        let dims: Vec<String> = report["dimensions"].as_object().unwrap()
            .iter().map(|(k, v)| format!("{}={}", k, v["status"].as_str().unwrap_or("?")))
            .collect();
        summaries.push(format!(
            "{} => {} [{}]",
            scenario,
            report["verdict"].as_str().unwrap_or("?"),
            dims.join(" ")
        ));
        assert_eq!(report["verdict"], "PASS",
            "{scenario} 判分未过：\n{}\n{summaries:#?}", serde_json::to_string_pretty(&report).unwrap());
    }
    for s in &summaries {
        println!("{s}");
    }
}
