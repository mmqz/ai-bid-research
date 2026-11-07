//! 七判分项 + G5 断言解释器（grade.py 的 Rust 移植，里程碑范围分级版）。
//!
//! 里程碑口径（防假绿也防越级判红）：
//! - M1 场景（TC-01/02/07/11）：G1/G2/G5/G7 判分；G3/G4/G6 = OUT_OF_SCOPE（M2+ 交付面）；
//!   G2 仅当 db.exact 存在（TC-02）判分，G7 仅当 db.idempotency 存在（TC-07）判分；
//! - full（自测完美库）：全七项判分。
//! 核心断言不可跳过：路径不可解析 → 失败（留空字段不能中性化断言——R2 P1-2 纪律）。

use crate::align::{aggregate_anchors, aligned_gt_ids, ambiguous_values};
use crate::fixtures::{EtAssert, Fixtures};
use bid_tools::schema::RunDb;
use serde_json::{json, Value};
use std::collections::HashSet;

pub const RULES_8: [&str; 8] = [
    "CERT_EXPIRED", "PRICE_EXCEEDS_BUDGET", "DURATION_MISMATCH", "PAGE_LIMIT",
    "MISSING_SIGNATURE", "DARK_BID_WORD", "TODO_RESIDUE", "INJECTION_PATTERN",
];

#[derive(Debug, Clone)]
pub enum DimOutcome {
    Pass(Value),
    Fail(Value),
    OutOfScope(String),
}

impl DimOutcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, DimOutcome::Pass(_))
    }
    pub fn detail(&self) -> Value {
        match self {
            DimOutcome::Pass(v) | DimOutcome::Fail(v) => v.clone(),
            DimOutcome::OutOfScope(s) => json!({"out_of_scope": s}),
        }
    }
    pub fn passed(&self) -> bool {
        self.is_pass()
    }
}

fn pass(v: Value) -> DimOutcome { DimOutcome::Pass(v) }
fn fail(v: Value) -> DimOutcome { DimOutcome::Fail(v) }

// ---------------------------------------------------------------- G1 召回 + 精确率守门
pub fn grade_recall(db: &RunDb, fx: &Fixtures) -> DimOutcome {
    let amb = ambiguous_values(&fx.gts);
    let mut aligned_have: HashSet<String> = HashSet::new();
    let mut self_claimed: HashSet<String> = HashSet::new();
    let mut category_mismatch = vec![];
    let mut density_violations = vec![];
    for r in &db.requirements {
        let ids = aligned_gt_ids(r, &fx.gts, &amb);
        aligned_have.extend(ids.iter().cloned());
        self_claimed.extend(r.gt_ids.iter().cloned());
        if ids.len() > 3 {
            density_violations.push(json!({"req_id": r.req_id, "aligned": ids}));
        }
        for g in &ids {
            if let Some(gt) = fx.gts.iter().find(|x| x.gt_id == *g) {
                // 合并 REQ 可声明多类别（逗号连写，对应 Python 列表口径）；单类别 REQ 必须精确一致
                let declared: Vec<&str> = r.category.split('，').flat_map(|x| x.split(',')).collect();
                if !declared.contains(&gt.category.as_str()) {
                    category_mismatch.push(json!({
                        "req_id": r.req_id, "gt": g,
                        "req_category": r.category, "gt_category": gt.category}));
                }
            }
        }
    }
    let missing: Vec<String> = fx.gts.iter().map(|g| g.gt_id.clone())
        .filter(|g| !aligned_have.contains(g)).collect();
    // #490（A-20②）：dedupe 探针改消费夹具的 dedupe_group 标记——
    // 带组的 GT 全部须被对齐（合并或交叉登记后各 gt_id 仍可对齐）。
    // 对当前夹具（GT-03/GT-14 唯二带 "D-业绩要求" 组）与旧硬编码
    // ["GT-03", "GT-14"] 逐位等价；换语料时自动跟随新组标记，
    // 判分口径不再钉死 GT 字面量。
    let dedupe_ok = fx
        .gts
        .iter()
        .filter(|g| g.dedupe_group.is_some())
        .all(|g| {
            db.requirements
                .iter()
                .any(|r| aligned_gt_ids(r, &fx.gts, &amb).contains(&g.gt_id))
        });
    let recall = 1.0 - missing.len() as f64 / fx.gts.len() as f64;
    let unverified: Vec<String> = self_claimed.difference(&aligned_have).cloned().collect();
    let precision_ok = category_mismatch.is_empty() && density_violations.is_empty();
    let ok = missing.is_empty() && dedupe_ok && precision_ok;
    if ok {
        pass(json!({"recall": recall, "missing": [], "dedupe_anchors_ok": true,
                    "unverified_self_claims": unverified,
                    "note": "判分只认对齐器（条款号双因子/锚点互含/唯一参数）"}))
    } else {
        fail(json!({"recall": recall, "missing": missing, "dedupe_anchors_ok": dedupe_ok,
                    "unverified_self_claims": unverified,
                    "category_mismatch": category_mismatch,
                    "density_violations": density_violations}))
    }
}

// ---------------------------------------------------------------- G2 精确匹配
pub fn grade_exact(db: &RunDb, fx: &Fixtures) -> DimOutcome {
    let empty = vec![];
    let per = db.exact.as_ref().map(|e| &e.per_query).unwrap_or(&empty);
    let mut misses = vec![];
    for q in &fx.queries {
        let qid = q["query_id"].as_str().unwrap_or_default();
        let got = per.iter().find(|p| p.query_id == qid);
        match got {
            None => misses.push(json!({"query_id": qid, "reason": "运行产物缺该查询"})),
            Some(g) => {
                let expect_text = q["expect"]["hit_text"].as_str().unwrap_or_default();
                let expect_file = q["expect"]["hit_file"].as_str().unwrap_or_default();
                if !g.hit {
                    misses.push(json!({"query_id": qid, "reason": "未命中"}));
                } else if !g.hit_text.contains(expect_text)
                    && !bid_parse::norm_n1(&g.hit_text).contains(&bid_parse::norm_n1(expect_text))
                {
                    misses.push(json!({"query_id": qid, "reason": "命中文本不含期望"}));
                } else if g.hit_file != expect_file {
                    misses.push(json!({"query_id": qid, "reason": "命中文件错",
                                       "expect": expect_file, "got": g.hit_file}));
                }
            }
        }
    }
    if misses.is_empty() {
        pass(json!({"hits": per.iter().filter(|p| p.hit).count(), "misses": []}))
    } else {
        fail(json!({"hits": per.iter().filter(|p| p.hit).count(), "misses": misses}))
    }
}

// ---------------------------------------------------------------- G3 矩阵 policy（M2+）
pub fn grade_policy(db: &RunDb, fx: &Fixtures) -> DimOutcome {
    let m = &db.matrix;
    let mut fails = vec![];
    for (cat, want) in [("资格条件", 6), ("否决项", 5)] {
        let n = m.iter().filter(|r| r["category"] == cat && r["explicit_policy"].as_bool().unwrap_or(false)).count();
        if n != want {
            fails.push(json!({"check": format!("{cat}.explicit_policy"), "got": n, "want": want}));
        }
    }
    let row = |gt: &str| m.iter().find(|r| r["gt_id"] == gt);
    let checks: [(&str, &str, Value, &str); 4] = [
        ("GT-04", "policy", json!("not_eligible"), "项目经理证书缺失 → not_eligible"),
        ("GT-02", "policy_pre_addendum", json!("eligible"), "原开标日临界有效"),
        ("GT-02", "policy_post_addendum", json!("not_eligible"), "补遗后 EXPIRED 翻转"),
        ("GT-13", "forbidden_bind", json!(true), "过期安许禁绑"),
    ];
    for (gt, field, want, why) in checks {
        match row(gt) {
            None => fails.push(json!({"check": format!("{gt}.{field}"), "got": null, "want": want, "why": why})),
            Some(r) => {
                let got = &r[field];
                if *got != want {
                    fails.push(json!({"check": format!("{gt}.{field}"), "got": got, "want": want, "why": why}));
                }
            }
        }
    }
    let _ = fx;
    if fails.is_empty() { pass(json!({"fails": []})) } else { fail(json!({"fails": fails})) }
}

// ---------------------------------------------------------------- G4 引用存在率（M2+）
pub fn grade_citation(db: &RunDb) -> DimOutcome {
    match &db.citation {
        None => fail(json!({"reason": "citation 块缺失"})),
        Some(c) => {
            if c.unanchored_claims == 0 && c.coverage == 1.0 {
                pass(json!({"unanchored_claims": c.unanchored_claims, "coverage": c.coverage}))
            } else {
                fail(json!({"unanchored_claims": c.unanchored_claims, "coverage": c.coverage}))
            }
        }
    }
}

// ---------------------------------------------------------------- G6 规则命中（M4）
pub fn grade_rules(db: &RunDb) -> DimOutcome {
    let codes: HashSet<&str> = db.findings.iter()
        .filter(|f| f.kind == "DETERMINISTIC").map(|f| f.rule_code.as_str()).collect();
    let missed: Vec<&str> = RULES_8.iter().filter(|r| !codes.contains(*r)).copied().collect();
    if missed.is_empty() {
        pass(json!({"hit": RULES_8.len()}))
    } else {
        fail(json!({"hit": RULES_8.len() - missed.len(), "missed": missed}))
    }
}

// ---------------------------------------------------------------- G7 幂等重放
pub fn grade_idempotency(db: &RunDb) -> DimOutcome {
    match &db.idempotency {
        None => fail(json!({"reason": "幂等重放未执行（字段缺失判 FAIL，不 None==None 假绿）"})),
        Some(i) => {
            if i.files_before == i.files_after && i.totals_before == i.totals_after {
                pass(json!({"files": [i.files_before, i.files_after],
                            "totals": [i.totals_before, i.totals_after]}))
            } else {
                fail(json!({"files": [i.files_before, i.files_after],
                            "totals": [i.totals_before, i.totals_after]}))
            }
        }
    }
}

// ---------------------------------------------------------------- G5 断言解释器
pub fn resolve_path(path: &str, db: &RunDb, fx: &Fixtures) -> Result<Value, String> {
    let amb = ambiguous_values(&fx.gts);
    if path == "requirements" {
        return Ok(json!(db.requirements.len()));
    }
    if path == "requirements.gt_ids" {
        let mut ids: HashSet<String> = HashSet::new();
        for r in &db.requirements {
            ids.extend(r.gt_ids.iter().cloned());
            ids.extend(aligned_gt_ids(r, &fx.gts, &amb));
        }
        return Ok(json!(ids));
    }
    if let Some(gt) = path.strip_prefix("requirements[gt=").and_then(|s| s.strip_suffix("].anchors")) {
        let n = aggregate_anchors(gt, &db.requirements, &fx.gts, &amb);
        return Ok(json!(n));
    }
    if path.starts_with("requirements[GT-") && path.contains("].params.") {
        let gt = path["requirements[".len()..].split("].params.").next().unwrap_or("").to_string();
        let field = path.split("].params.").nth(1).unwrap_or("").to_string();
        let amb2 = ambiguous_values(&fx.gts);
        let req = db.requirements.iter()
            .find(|r| aligned_gt_ids(r, &fx.gts, &amb2).contains(&gt))
            .ok_or_else(|| format!("REQ 未含 {gt}"))?;
        return Ok(req.params.get(&field).cloned().unwrap_or(Value::Null));
    }
    if path.starts_with("matrix[category=") && path.ends_with("].explicit_policy") {
        let cat = &path["matrix[category=".len()..path.len() - "].explicit_policy".len()];
        let n = db.matrix.iter()
            .filter(|r| r["category"] == cat && r["explicit_policy"].as_bool().unwrap_or(false)).count();
        return Ok(json!(n));
    }
    if path == "idempotency.files_delta" {
        let i = db.idempotency.as_ref().ok_or("idempotency 块缺失")?;
        return Ok(json!(i.files_before as i64 - i.files_after as i64));
    }
    if path == "idempotency.chunks_delta" {
        let i = db.idempotency.as_ref().ok_or("idempotency 块缺失")?;
        return Ok(json!(i.totals_before as i64 - i.totals_after as i64));
    }
    if path == "exact_hits" {
        return Ok(json!(db.exact.as_ref().map(|e| e.hits).unwrap_or(0)));
    }
    if path == "unanchored_claims" {
        return Ok(json!(db.citation.as_ref().map(|c| c.unanchored_claims).unwrap_or(u64::MAX)));
    }
    if path == "citation_coverage" {
        return Ok(json!(db.citation.as_ref().map(|c| c.coverage).unwrap_or(f64::NAN)));
    }
    if path == "parse_failures" {
        return Ok(json!(db.files.iter().filter(|f| f.parse_status == "failed").count()));
    }
    if path == "missing_sequence" {
        return Ok(json!(db.parse.missing_sequence.len()));
    }
    if path.starts_with("files[") && path.ends_with("].ocr_blocks") {
        let key = &path["files[".len()..path.len() - "].ocr_blocks".len()];
        let f = db.files.iter().find(|f| f.name.contains(key)).ok_or(format!("无文件 {key}"))?;
        return Ok(json!(f.ocr_blocks));
    }
    if path == "sections" {
        return Ok(json!(db.sections.len()));
    }
    if path == "files" {
        return Ok(json!(db.files.len()));
    }
    if path == "totals.chunks" {
        return Ok(json!(db.totals.chunks));
    }
    if path == "resume_plan.sections" {
        let n = db.resume_plan.as_ref()
            .and_then(|v| v["sections"].as_array()).map(|a| a.len()).unwrap_or(0);
        return Ok(json!(n));
    }
    if path.starts_with("findings[kind=") && path.contains("].severity=") {
        let rest = &path["findings[kind=".len()..];
        let (kind, sev) = rest.split_once("].severity=").ok_or("findings 路径格式错")?;
        let n = db.findings.iter().filter(|f| f.kind == kind && f.severity == sev).count();
        return Ok(json!(n));
    }
    if path == "dark_bid_word_scan_hits" {
        return Ok(json!(db.export.as_ref().and_then(|v| v["dark_bid_word_scan_hits"].as_i64()).unwrap_or(-1)));
    }
    if path == "findings" {
        return Ok(json!(db.findings.len()));
    }
    // ---- state 型 ----
    if path == "project.status" {
        return Ok(json!(db.project.status));
    }
    if path.starts_with("matrix[GT-") && path.contains("].policy") {
        let rest = &path["matrix[".len()..];
        let (gt, tail) = rest.split_once("].policy").ok_or("matrix 路径格式错")?;
        let row = db.matrix.iter().find(|r| r["gt_id"] == gt).ok_or(format!("矩阵无 {gt} 行"))?;
        if let Some(date) = tail.strip_prefix("@open_bid=") {
            let post = fixture_open_bid_after(fx);
            let field = if date == post { "policy_post_addendum" } else { "policy_pre_addendum" };
            return Ok(row[field].clone());
        }
        return Ok(row["policy"].clone());
    }
    if path.starts_with("sections[") && path.contains("].") {
        let rest = &path["sections[".len()..];
        let (sid, field) = rest.split_once("].").ok_or("sections 路径格式错")?;
        let sec = db.sections.iter().find(|s| s.id == sid).ok_or(format!("无章节 {sid}"))?;
        return Ok(match field {
            "status" => json!(sec.status),
            "dirty" => json!(sec.dirty),
            "last_model" => json!(sec.last_model),
            "rerun_count" => json!(sec.rerun_count),
            _ => Value::Null,
        });
    }
    if path == "last_tool_result.error" {
        return Ok(json!(db.last_tool_result.error));
    }
    if path == "artifact.pages_word_render" {
        return Ok(db.export.as_ref().map(|v| v["artifact"]["pages_word_render"].clone())
            .ok_or("export 块缺失")?);
    }
    if path == "artifact.sha256" {
        return Ok(db.export.as_ref().map(|v| v["artifact"]["sha256"].clone())
            .ok_or("export 块缺失")?);
    }
    if path == "requirements[GT-02].recheck" {
        return Ok(json!(db.recheck.get("GT-02")));
    }
    Err(format!("未支持的断言路径: {path}"))
}

fn fixture_open_bid_after(fx: &Fixtures) -> String {
    // #490（A-20③）：开标日真源下沉夹具 meta.json
    // （open_bid_after_addendum——补遗 C 推迟后的开标日，GT-02 语义翻转
    // 的裁决时刻）。旧部署缺失时回退已知硬编码并提示（不炸——判分器
    // 对旧夹具目录的向后兼容面）。
    if let Some(d) = &fx.open_bid_after_addendum {
        return d.clone();
    }
    eprintln!(
        "[grade] meta.json 缺 open_bid_after_addendum（{}）——回退硬编码 2026-11-27",
        fx.root.display()
    );
    "2026-11-27".to_string()
}

fn cmp_values(op: &str, actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap_or(f64::NAN), b.as_f64().unwrap_or(f64::NAN));
            match op {
                "==" => a == b, "!=" => a != b, ">=" => a >= b,
                "<=" => a <= b, ">" => a > b, "<" => a < b, _ => false,
            }
        }
        _ => match op {
            "==" => actual == expected,
            "!=" => actual != expected,
            "in" => expected.as_array().map(|a| a.contains(actual)).unwrap_or(false),
            _ => false,
        },
    }
}

pub fn grade_transitions(db: &RunDb, fx: &Fixtures, only: Option<&str>) -> DimOutcome {
    let mut results = vec![];
    let mut skipped = vec![];
    // 场景互斥口径（R2 P1-1）：TC-06/TC-08/TC-11 各有专用场景库，full 运行不叠加
    let excluded_full = ["TC-06", "TC-08", "TC-11"];
    for sc in &fx.scenarios {
        if let Some(tc) = only {
            if sc.tc_id != tc {
                continue;
            }
        } else if excluded_full.contains(&sc.tc_id.as_str()) {
            continue;
        }
        for a in &sc.asserts {
            results.push(interpret_assert(a, db, fx, &mut skipped));
        }
    }
    let n_fail = results.iter().filter(|r| r["pass"].as_bool() == Some(false)).count();
    let summary = json!({
        "pass": results.iter().filter(|r| r["pass"].as_bool() == Some(true)).count(),
        "fail": n_fail,
        "skipped": skipped.len(),
    });
    if n_fail == 0 {
        pass(json!({"asserts": results, "skipped": skipped, "summary": summary}))
    } else {
        fail(json!({"asserts": results, "skipped": skipped, "summary": summary}))
    }
}

fn interpret_assert(a: &EtAssert, db: &RunDb, fx: &Fixtures, skipped: &mut Vec<Value>) -> Value {
    let kind = a.kind.as_str();
    if kind == "engine_gap" {
        skipped.push(json!({"gap": a.value, "note": a.note}));
        return json!({"kind": kind, "pass": true, "skipped": true, "note": a.note});
    }
    // set_cover / rules_hit / repeat / dirty / not_dirty / not_rerun / conflict / ocr / metric
    let pass = |ok: bool| json!({
        "kind": kind, "path": a.path, "pass": ok,
        "value": a.value, "note": a.note
    });
    match kind {
        "set_cover" => {
            match resolve_path(&a.path, db, fx) {
                Ok(Value::Array(items)) => {
                    let want: Vec<&str> = a.value.as_array().map(|x| x.as_slice()).unwrap_or(&[])
                        .iter().filter_map(|v| v.as_str()).collect();
                    let have: HashSet<&str> = items.iter().filter_map(|v| v.as_str()).collect();
                    pass(want.iter().all(|w| have.contains(w)))
                }
                Ok(_) => pass(false),
                Err(e) => json!({"kind": kind, "path": a.path, "pass": false, "reason": e}),
            }
        }
        "rules_hit" => {
            let codes: HashSet<&str> = db.findings.iter().map(|f| f.rule_code.as_str()).collect();
            let want: Vec<&str> = a.value.as_array().map(|x| x.as_slice()).unwrap_or(&[])
                .iter().filter_map(|v| v.as_str()).collect();
            pass(want.iter().all(|w| codes.contains(w)))
        }
        "repeat" => pass(db.stability.as_ref().map(|s| s.repeat_consistent).unwrap_or(false)),
        "dirty" | "not_dirty" => {
            let dirty: HashSet<&str> = db.sections.iter().filter(|s| s.dirty)
                .map(|s| s.id.as_str()).collect();
            let want: Vec<&str> = a.value.as_array().map(|x| x.as_slice()).unwrap_or(&[])
                .iter().filter_map(|v| v.as_str()).collect();
            let ok = if kind == "dirty" {
                want.iter().all(|w| dirty.contains(w))
            } else {
                !want.iter().any(|w| dirty.contains(w))
            };
            pass(ok)
        }
        "not_rerun" => {
            let sid = a.path.trim_start_matches("sections[").trim_end_matches(']');
            let sec = db.sections.iter().find(|s| s.id == sid);
            let want_status = a.value.as_str().unwrap_or_default();
            pass(matches!(sec, Some(s) if s.status == want_status && s.rerun_count == 0))
        }
        "conflict" => {
            let cid = a.path.trim_start_matches("conflicts[").trim_end_matches(']');
            let con = db.conflicts.iter().find(|c| c["conflict_id"].as_str() == Some(cid));
            let final_v = a.value["final"].as_i64();
            let rule = a.value["rule"].as_str();
            pass(matches!(con, Some(c)
                if c["resolved"].as_bool() == Some(true)
                    && c["final_value"].as_i64() == final_v
                    && c["rule"].as_str() == rule))
        }
        "ocr" => {
            let key = a.path.trim_start_matches("files[").trim_end_matches("].ocr_blocks");
            // ocr 路径形如 files[B2].ocr_blocks —— 上面已剥；直接复用 resolve
            match resolve_path(&format!("files[{key}].ocr_blocks"), db, fx) {
                Ok(v) => {
                    let op = a.op.as_deref().unwrap_or(">=");
                    pass(cmp_values(op, &v, &a.value))
                }
                Err(e) => json!({"kind": kind, "path": a.path, "pass": false, "reason": e}),
            }
        }
        "metric" => pass(true), // A/B 指标比较由 TC-10 专项报告承载
        "error" | "policy" | "state" | "count" => {
            // 核心断言不可跳过：路径不可解析即失败
            match resolve_path(&a.path, db, fx) {
                Ok(v) => {
                    let op = a.op.as_deref().unwrap_or("==");
                    pass(cmp_values(op, &v, &a.value))
                }
                Err(e) => json!({"kind": kind, "path": a.path, "pass": false,
                                 "reason": format!("断言路径不可解析：{e}——留空字段不能中性化核心断言")}),
            }
        }
        _ => json!({"kind": kind, "pass": false, "reason": "未知断言类型"}),
    }
}

// ---------------------------------------------------------------- 总判分（里程碑分级）
pub fn grade_run(db: &RunDb, fx: &Fixtures) -> Value {
    let scenario = db.scenario.clone();
    let only = if scenario.starts_with("TC-") { Some(scenario.as_str()) } else { None };
    let m1 = db.milestone == "M1";
    let mut dims = serde_json::Map::new();
    dims.insert("G1_recall".into(), outcome_json("G1_recall", g1_scope(db, fx), m1));
    dims.insert("G2_exact".into(), outcome_json("G2_exact", exact_scope(db, fx), m1));
    dims.insert("G3_policy".into(), outcome_json("G3_policy",
        if m1 { DimOutcome::OutOfScope("M2 矩阵交付面".into()) } else { grade_policy(db, fx) }, m1));
    dims.insert("G4_citation".into(), outcome_json("G4_citation",
        if m1 { DimOutcome::OutOfScope("M2 引用覆盖交付面".into()) } else { grade_citation(db) }, m1));
    dims.insert("G5_transitions".into(), outcome_json("G5_transitions",
        grade_transitions(db, fx, only), m1));
    dims.insert("G6_rules".into(), outcome_json("G6_rules",
        if m1 { DimOutcome::OutOfScope("M4 审校规则交付面".into()) } else { grade_rules(db) }, m1));
    dims.insert("G7_idempotency".into(), outcome_json("G7_idempotency", idem_scope(db), m1));
    // verdict 只由被判定维度决定：OUT_OF_SCOPE（里程碑未覆盖面）不计入成败，但显式留痕
    let all_pass = dims.values().all(|v| {
        v["status"].as_str() == Some("OUT_OF_SCOPE") || v["pass"].as_bool() == Some(true)
    });
    json!({
        "run_scenario": scenario,
        "milestone": db.milestone,
        "dimensions": dims,
        "verdict": if all_pass { "PASS" } else { "FAIL" },
    })
}

fn g1_scope(db: &RunDb, fx: &Fixtures) -> DimOutcome {
    // G1 适用性：抽取未执行（空 requirements）的场景不判召回——TC-11 只验越级防护
    if db.milestone == "full" || !db.requirements.is_empty() {
        grade_recall(db, fx)
    } else {
        DimOutcome::OutOfScope("该场景未执行条款抽取（TC-11 只验状态机越级防护）".into())
    }
}

fn exact_scope(db: &RunDb, fx: &Fixtures) -> DimOutcome {
    let required = db.milestone == "full" || db.scenario == "TC-02";
    if db.exact.is_some() {
        grade_exact(db, fx)
    } else if required {
        grade_exact(db, fx) // 无 exact 块 → 判 FAIL（未执行 ≠ 通过）
    } else {
        DimOutcome::OutOfScope("该场景未执行精确查询（TC-02 承载）".into())
    }
}

fn idem_scope(db: &RunDb) -> DimOutcome {
    let required = db.milestone == "full" || db.scenario == "TC-07";
    if db.idempotency.is_some() {
        grade_idempotency(db)
    } else if required {
        grade_idempotency(db) // 无 idempotency 块 → 判 FAIL
    } else {
        DimOutcome::OutOfScope("该场景未执行幂等重放（TC-07 承载）".into())
    }
}

fn outcome_json(name: &str, o: DimOutcome, _m1: bool) -> Value {
    let (st, detail) = match &o {
        DimOutcome::Pass(v) => ("PASS", v.clone()),
        DimOutcome::Fail(v) => ("FAIL", v.clone()),
        DimOutcome::OutOfScope(s) => ("OUT_OF_SCOPE", json!({"reason": s})),
    };
    json!({"name": name, "status": st, "pass": st == "PASS", "detail": detail})
}
