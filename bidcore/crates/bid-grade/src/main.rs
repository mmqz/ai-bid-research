//! bidgrade CLI：判分运行产物 / 自测。
//! 用法：
//!   bidgrade --self-test                       # 判分器自测（完美库 + 破坏注入全捕获）
//!   bidgrade --run <dir>                       # 判分 runs/<scenario>/db.json
//!   bidgrade --all-runs <runs_dir>             # 判分目录下全部场景

use bid_grade::{fixtures, grade, selftest};

use std::path::{Path, PathBuf};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let fixtures_dir = std::env::var("BID_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root.join("fixtures"));
    if args.iter().any(|a| a == "--self-test") {
        // #481（A-20④）：夹具装载失败不 panic——评测链终点被 run_tests.py 消费
        // 退出码，panic 栈轨迹对自动化不友好（错误码纪律）。
        let fx = match fixtures::Fixtures::load(&fixtures_dir) {
            Ok(fx) => fx,
            Err(e) => {
                eprintln!("[grade] 夹具装载失败（{}）：{}", fixtures_dir.display(), e);
                return std::process::ExitCode::FAILURE;
            }
        };
        let failed = selftest::run(&fx);
        if failed == 0 {
            println!("\n判分器自测：全部通过");
            std::process::ExitCode::SUCCESS
        } else {
            println!("\n判分器自测：{failed} 项失败");
            std::process::ExitCode::FAILURE
        }
    } else if let Some(i) = args.iter().position(|a| a == "--run") {
        let run_dir = PathBuf::from(&args[i + 1]);
        grade_one(&run_dir, &fixtures_dir)
    } else if let Some(i) = args.iter().position(|a| a == "--all-runs") {
        let runs_dir = PathBuf::from(&args[i + 1]);
        let mut any_fail = false;
        for entry in std::fs::read_dir(&runs_dir).expect("runs 目录不存在").flatten() {
            if entry.path().join("db.json").exists() {
                let code = grade_one(&entry.path(), &fixtures_dir);
                if code == std::process::ExitCode::FAILURE {
                    any_fail = true;
                }
            }
        }
        if any_fail { std::process::ExitCode::FAILURE } else { std::process::ExitCode::SUCCESS }
    } else {
        eprintln!("用法：bidgrade --self-test | --run <dir> | --all-runs <runs_dir>");
        std::process::ExitCode::from(2)
    }
}

fn grade_one(run_dir: &Path, fixtures_dir: &Path) -> std::process::ExitCode {
    let db: bid_tools::schema::RunDb = match std::fs::read_to_string(run_dir.join("db.json")) {
        Ok(t) => match serde_json::from_str(&t) {
            Ok(db) => db,
            Err(e) => {
                // 反序列化失败 = schema 契约违反（编译期类型的运行期防线）
                eprintln!("[grade] db.json 违反 schema 契约：{e}");
                return std::process::ExitCode::FAILURE;
            }
        },
        Err(e) => {
            eprintln!("[grade] 读取失败：{e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // #481（A-20④）：同上——不 panic，原因 + 非零退出码。
    let fx = match fixtures::Fixtures::load(fixtures_dir) {
        Ok(fx) => fx,
        Err(e) => {
            eprintln!("[grade] 夹具装载失败（{}）：{}", fixtures_dir.display(), e);
            return std::process::ExitCode::FAILURE;
        }
    };
    let report = grade::grade_run(&db, &fx);
    let out = run_dir.join("report.json");
    std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap()).ok();
    for (name, d) in report["dimensions"].as_object().unwrap() {
        println!("  {} {:>13} {}", if d["pass"].as_bool() == Some(true) { "✓" } else { "✗" },
                 d["status"].as_str().unwrap_or("?"), name);
    }
    println!("  => {}（{}）", report["verdict"], out.display());
    if report["verdict"] == "PASS" { std::process::ExitCode::SUCCESS }
    else { std::process::ExitCode::FAILURE }
}
