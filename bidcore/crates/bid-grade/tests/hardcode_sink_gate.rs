//! #490（A-20②③）：判分器硬编码下沉的解析与等价门禁。
//!
//! - ② dedupe_ok 原钉死 `["GT-03", "GT-14"]` 字面量——夹具早有
//!   `dedupe_group` 标记（fixture_spec.py 生成）但 GtRequirement 不解析。
//!   #490：判分面改消费组标记（带组 GT 全部须被对齐）——对当前夹具
//!   与旧硬编码逐位等价，换语料自动跟随；
//! - ③ fixture_open_bid_after 原硬编码 "2026-11-27"（`let _ = fx;`
//!   占位 TODO）——真源下沉 `ground-truth/meta.json`，缺失回退。

use bid_grade::fixtures::Fixtures;

fn fixtures_loaded() -> Option<Fixtures> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let fixtures_dir = std::env::var("BID_FIXTURES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| root.join("fixtures"));
    let corpus_a = fixtures_dir.join("corpus-A/files");
    if bid_parse::env_preflight::skip_if_fixture_missing("grade_hardcode_sink", &[&corpus_a]) {
        return None;
    }
    Some(Fixtures::load(&fixtures_dir).expect("夹具装载失败"))
}

/// ② dedupe_group 解析面：GT-03/GT-14 带组（当前夹具唯一二组员），
/// 其余 GT 无组——判分面「filter(dedupe_group.is_some())」与旧硬编码
/// `["GT-03", "GT-14"]` 的成员集逐位等价。
#[test]
fn dedupe_group_field_parses_exactly_the_legacy_pair() {
    let Some(fx) = fixtures_loaded() else { return };
    let grouped: Vec<&str> = fx
        .gts
        .iter()
        .filter(|g| g.dedupe_group.is_some())
        .map(|g| g.gt_id.as_str())
        .collect();
    assert_eq!(
        grouped,
        vec!["GT-03", "GT-14"],
        "带组 GT 集合须与旧硬编码清单逐位一致（语义等价锁）——实际: {grouped:?}"
    );
    // 组语义自证：两条同组（重复表述探针）
    let g03 = fx.gts.iter().find(|g| g.gt_id == "GT-03").unwrap();
    let g14 = fx.gts.iter().find(|g| g.gt_id == "GT-14").unwrap();
    assert_eq!(g03.dedupe_group, g14.dedupe_group, "GT-03/GT-14 须同组");
    assert_eq!(g03.dedupe_group.as_deref(), Some("D-业绩要求"));
}

/// ③ meta.json 真源面：开标日解析进 Fixtures（补遗后口径），
/// 且与 GT-02 expected_policy 的 @open_bid 后缀交叉印证一致。
#[test]
fn open_bid_after_addendum_parses_from_meta_and_crosschecks_gt02() {
    let Some(fx) = fixtures_loaded() else { return };
    let d = fx
        .open_bid_after_addendum
        .as_deref()
        .expect("meta.json 在场时须解析出 open_bid_after_addendum");
    assert_eq!(d, "2026-11-27", "补遗后开标日（PROJECT.open_bid_date_after_addendum 派生）");
    // 交叉印证：GT-02 的翻转时点 @open_bid=2026-11-27 与 meta 一致
    let gt02 = fx.gts.iter().find(|g| g.gt_id == "GT-02").unwrap();
    let policy = gt02.expected_policy.as_deref().unwrap_or_default();
    assert!(
        policy.contains("@open_bid=2026-11-27"),
        "GT-02 翻转时点须与 meta.json 同源——实际 policy: {policy}"
    );
}

/// ③ 回退面：meta.json 缺失的旧夹具目录 → None（消费方回退硬编码 +
/// eprintln 提示，不炸——向后兼容）。用临时目录构造最小夹具树。
#[test]
fn missing_meta_falls_back_to_none_not_error() {
    let tmp = std::env::temp_dir().join(format!(
        "gt-meta-missing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let gt = tmp.join("corpus-A/ground-truth");
    std::fs::create_dir_all(&gt).unwrap();
    // 最小 requirements.jsonl（无 meta.json——旧部署形态）
    std::fs::write(
        gt.join("requirements.jsonl"),
        "{\"gt_id\":\"GT-01\",\"category\":\"资格条件\",\"anchors\":[]}\n",
    )
    .unwrap();
    std::fs::create_dir_all(tmp.join("corpus-A/expected-transitions")).unwrap();
    std::fs::create_dir_all(tmp.join("queries")).unwrap();
    std::fs::write(tmp.join("queries/exact-25.jsonl"), "").unwrap();

    let fx = Fixtures::load(&tmp).expect("meta 缺失不应装载失败（向后兼容）");
    assert_eq!(
        fx.open_bid_after_addendum, None,
        "无 meta.json → None（消费方回退硬编码并提示）"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
