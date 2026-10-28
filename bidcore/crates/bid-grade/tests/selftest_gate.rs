//! 判分器自测门禁：完美库 + 破坏注入全捕获（挂在 cargo test 主入口）。

#[test]
fn grader_selftest_all_green() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let fixtures_dir = std::env::var("BID_FIXTURES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| root.join("fixtures"));
    // PR #24 修复批次：真实样本缺件 → SKIP 横幅（issue #9 机制文件面）
    let corpus_a = fixtures_dir.join("corpus-A/files");
    if bid_parse::env_preflight::skip_if_fixture_missing("grader_selftest_all_green", &[&corpus_a]) {
        return;
    }
    let fx = bid_grade::fixtures::Fixtures::load(&fixtures_dir).expect("夹具装载失败");
    let failed = bid_grade::selftest::run(&fx);
    assert_eq!(failed, 0, "判分器自测存在 {failed} 项失败");
}
