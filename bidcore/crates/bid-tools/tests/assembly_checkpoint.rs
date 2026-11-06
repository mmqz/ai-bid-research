//! assembly_checkpoint.rs —— S-2 断点续跑验收（issue #11 批次 v3 / V3-3）。
//!
//! 硬验收（Lead 原文）：「生成到第 300 章（共 500 章）时强杀进程 → 重启 →
//! resume → 前 299 章内容逐字节不变 → 从第 300 章继续，不重跑前 299 章 →
//! 总 token 消耗不含重复生成（即每章恰好生成一次）」——由
//! `acceptance_kill_at_300_resume_byte_identical` 以**真实进程级**双 spawn +
//! SIGABRT 强杀覆盖；其余测试覆盖工作区的 fail-closed / tmp 残留 / attempt
//! 单调 / 幂等重放 / 断点切分 / 跳过面 / 失败面 / 字节稳定面。
//!
//! 借鉴出处（mdcore `crates/workbench/tests/persistence_recovery.rs`——设计
//! 阶段借鉴仓的异常恢复测试模式）：
//! - fail-closed 三损拒载 ← `corrupt_truncated_and_missing_sidecar_fail_closed_on_open`；
//! - tmp 残留不破恢复 ← `interrupted_publish_leftover_does_not_break_recovery`；
//! - 重开后版本续增不回卷 ← `source_version_monotonic_across_reopen`；
//! - 跨会话幂等重放零副作用 ← `cross_session_idempotent_replay`。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::assembly::{assemble_all, AssemblerError, ChapterAssembler, CHAPTERS_SUBDIR};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 夹具

/// 500 章基座（照 bid-domain outline.rs 尾部 store_with_reqs 夹具模式）：
/// create_project → upsert_file → set_requirements（500 条 REQ，category 全
/// 「技术参数」，anchors 逐条唯一 excerpt——dedupe_hash 不合并）→ 手工
/// set_outline（500 个 level-1 平铺叶节点，order 1..=500，每节点绑 1 条 REQ，
/// target_chars=2000——不用 derive，结构可控：id 确定为 SEC-0001..SEC-0500，
/// 跨进程重建全等，这是断点续跑对账的前提）。
fn store_with_500_chapters() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("断点续跑测试项目", "ZFCG-2026-S2-001", "明标", "s2-key").unwrap();
    s.upsert_file("A-招标文件.docx", "招标文件", "aa", 31, 12, 120, 0,
                  ParseStatus::Ok, "engine-docx @ test").unwrap();
    let reqs: Vec<Requirement> = (0..500)
        .map(|i| Requirement {
            id: format!("REQ-{:04}", i + 1),
            category: "技术参数".into(),
            clause_no: Some(format!("3.{}", i + 1)),
            anchors: vec![Anchor {
                file: "A".into(),
                kind: "para".into(),
                locator: "第三章".into(),
                // 逐条唯一 excerpt——同 excerpt 会被 dedupe_hash 合并，500 章基座就塌了
                excerpt: format!("S-2 条款-{i} 的唯一原文摘录"),
                    block_idx: None,
                    page: None,
                    line: None
            }],
            params: std::collections::HashMap::new(),
            source_file: "A".into(),
        })
        .collect();
    s.set_requirements(reqs).unwrap();
    let nodes: Vec<OutlineNode> = (0..500)
        .map(|i| OutlineNode {
            id: format!("SEC-{:04}", i + 1),
            parent_id: None, // 全 level-1 平铺——无子即叶
            title: format!("技术参数响应（第{}章）", i + 1),
            level: 1,
            order: (i + 1) as u32,
            requirement_ids: vec![format!("REQ-{:04}", i + 1)],
            target_chars: 2000,
            status: SectionStatus::Pending,
            dirty: false,
            content: None, content_attempt: 0,
        })
        .collect();
    s.set_outline(nodes).unwrap();
    s
}

/// 确定性 generator：纯函数——同节点同输出（断点续跑「逐字节不变」的生成侧
/// 前提：内容是节点的确定函数，不含时钟/随机）。
fn deterministic_gen(node: &OutlineNode) -> Result<String, String> {
    Ok(format!(
        "{}：本章响应 {}，正文内容……（2000 字口径示意）",
        node.title,
        node.requirement_ids.join("、")
    ))
}

/// 第 i 章（1 起）的章节文件路径（夹具 id 确定性的消费面）。
fn chapter_file(dir: &Path, i: usize) -> PathBuf {
    dir.join(CHAPTERS_SUBDIR).join(format!("SEC-{i:04}.json"))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bid-s2-asm-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

// ============================ 借鉴测试（borrow：mdcore persistence_recovery.rs） ============================

/// borrow：mdcore `persistence_recovery.rs::
/// corrupt_truncated_and_missing_sidecar_fail_closed_on_open`——截断/坏 JSON/
/// 缺字段三种损坏 → open 全部 Corrupt 拒载且**原文件保留**（不静默空态、
/// 不覆写现场——数据失踪不得伪装成「未提交」）。
#[test]
fn corrupt_truncated_missing_chapter_fail_closed() {
    let dir = temp_dir("corrupt");
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        asm.commit_section("SEC-0001", "完好章节内容（用于制造可损坏的样本）",
                           vec!["REQ-0001".into()]).unwrap();
    }
    let path = dir.join(CHAPTERS_SUBDIR).join("SEC-0001.json");
    let good = std::fs::read_to_string(&path).unwrap();

    // 损坏拒载断言器：Corrupt + 定位到具体文件 + 原文件保留
    let assert_corrupt = |case: &str| {
        match ChapterAssembler::open(&dir) {
            Err(AssemblerError::Corrupt { path: p, .. }) => {
                assert_eq!(&p, &path, "{case}: 损坏必须定位到具体章节文件");
            }
            Err(other) => panic!("{case}: 期望 Corrupt 拒载，实际错误: {other}"),
            Ok(_) => panic!("{case}: 期望 Corrupt 拒载，实际静默加载（数据伪装）"),
        }
        assert!(path.exists(), "{case}: 原文件必须保留（fail-closed 不覆写现场）");
    };

    // —— 截断 JSON（字节对半切，退到 UTF-8 字符边界——截断本身可以劈开多字节字）——
    let mut cut = good.len() / 2;
    while !good.is_char_boundary(cut) {
        cut -= 1;
    }
    std::fs::write(&path, &good[..cut]).unwrap();
    assert_corrupt("截断");

    // —— 坏 JSON ——
    std::fs::write(&path, b"{ not valid json !!!").unwrap();
    assert_corrupt("坏 JSON");

    // —— 缺字段（手改 JSON 删 content——结构缺失面）——
    let mut v: serde_json::Value = serde_json::from_str(&good).unwrap();
    v.as_object_mut().unwrap().remove("content");
    std::fs::write(&path, serde_json::to_string(&v).unwrap()).unwrap();
    assert_corrupt("缺 content 字段");

    // 恢复完好文件后一切正常（异常可诊断可恢复，不是永久损坏）
    std::fs::write(&path, &good).unwrap();
    let asm = ChapterAssembler::open(&dir).unwrap();
    assert_eq!(asm.committed_count(), 1, "完好文件恢复加载");
    assert_eq!(asm.record("SEC-0001").unwrap().content, "完好章节内容（用于制造可损坏的样本）");
    let _ = std::fs::remove_dir_all(&dir);
}

/// borrow：mdcore `persistence_recovery.rs::
/// interrupted_publish_leftover_does_not_break_recovery`——中断提交的 tmp
/// 残留不破恢复（主文件完好 = 上次提交的原子性保证）；下次提交原子顶替，
/// 垃圾被 rename 覆盖，零残留。
#[test]
fn leftover_tmp_does_not_break_recovery() {
    let dir = temp_dir("tmp");
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        asm.commit_section("SEC-0001", "v1 内容", vec!["REQ-0001".into()]).unwrap();
    }
    // 模拟中断提交：chapters 目录残留垃圾 tmp（rename 未发生）
    let tmp = dir.join(CHAPTERS_SUBDIR).join("SEC-0001.json.tmp");
    std::fs::write(&tmp, b"garbage from interrupted commit").unwrap();
    // 残留不破恢复：open 正常、既有记录在位
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    assert_eq!(asm.committed_count(), 1, "tmp 残留不破恢复");
    assert_eq!(asm.record("SEC-0001").unwrap().content, "v1 内容");
    // 下次提交原子顶替：垃圾被 rename 覆盖，零残留
    asm.commit_section("SEC-0001", "v2 修订内容", vec!["REQ-0001".into()]).unwrap();
    assert!(!tmp.exists(), "提交后 tmp 被 rename 顶替（零残留）");
    // 重读验证落盘为最新（且 attempt 续增）
    let asm2 = ChapterAssembler::open(&dir).unwrap();
    assert_eq!(asm2.record("SEC-0001").unwrap().content, "v2 修订内容");
    assert_eq!(asm2.record("SEC-0001").unwrap().attempt, 2, "修订 attempt=2");
    let _ = std::fs::remove_dir_all(&dir);
}

/// borrow：mdcore `persistence_recovery.rs::source_version_monotonic_across_reopen`
/// 的「重开后续增不回卷」——章 v1 提交 → drop（进程退出）→ reopen → 不同
/// content 提交 → attempt=2（从盘上记录续增，不重启为 1）；同 content 再提交
/// → no-op（attempt 不动、文件字节不变）。
#[test]
fn attempt_monotonic_across_reopen() {
    let dir = temp_dir("attempt");
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        let r1 = asm.commit_section("SEC-0001", "第一版内容", vec!["REQ-0001".into()]).unwrap();
        assert_eq!(r1.attempt, 1, "新章 attempt=1 起");
    } // —— drop 模拟进程退出（磁盘是唯一传递介质）——

    let mut asm = ChapterAssembler::open(&dir).unwrap();
    let r2 = asm.commit_section("SEC-0001", "第二版修订内容", vec!["REQ-0001".into()]).unwrap();
    assert_eq!(r2.attempt, 2, "重开后不同内容 → attempt 续增不回卷");

    // 同 content 再提交 → no-op：attempt 不动、文件一个字节都不动
    let before = std::fs::read(chapter_file(&dir, 1)).unwrap();
    let r3 = asm.commit_section("SEC-0001", "第二版修订内容", vec!["REQ-0001".into()]).unwrap();
    assert_eq!(r3.attempt, 2, "幂等 no-op 不增 attempt");
    assert_eq!(r3, r2, "no-op 返回既有记录全等");
    assert_eq!(std::fs::read(chapter_file(&dir, 1)).unwrap(), before, "文件字节不变");

    // 再跨一次重开验证不回卷（重放面：attempt 仍 2，文件仍不动）
    drop(asm);
    let mut asm2 = ChapterAssembler::open(&dir).unwrap();
    let r4 = asm2.commit_section("SEC-0001", "第二版修订内容", vec!["REQ-0001".into()]).unwrap();
    assert_eq!(r4.attempt, 2, "二次重开仍不回卷");
    assert_eq!(std::fs::read(chapter_file(&dir, 1)).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

/// borrow：mdcore `persistence_recovery.rs::cross_session_idempotent_replay`
/// ——同 content+citations 双提交（含跨会话重放）→ 第二次返回的记录与首次
/// 全等、文件 mtime/字节不变（首果复用零副作用——不重复执行写路径）。
#[test]
fn commit_idempotent_same_content_noop() {
    let dir = temp_dir("idem");
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    let citations = vec!["REQ-0001".to_string(), "EV-0001".to_string()];
    let first = asm.commit_section("SEC-0001", "稳定内容", citations.clone()).unwrap();
    let path = chapter_file(&dir, 1);
    let bytes1 = std::fs::read(&path).unwrap();
    let mtime1 = std::fs::metadata(&path).unwrap().modified().unwrap();

    // 会话内双提交：同 content + 同 citations → 首果全等 + 零副作用
    let second = asm.commit_section("SEC-0001", "稳定内容", citations.clone()).unwrap();
    assert_eq!(second, first, "重放返回首果全等（含 updated_at——零副作用）");
    assert_eq!(std::fs::read(&path).unwrap(), bytes1, "文件字节一个都不动");
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime1,
               "未走写路径——mtime 不变");

    // 跨会话重放（drop → reopen → 同参提交）仍幂等
    drop(asm);
    let mut asm2 = ChapterAssembler::open(&dir).unwrap();
    let third = asm2.commit_section("SEC-0001", "稳定内容", citations).unwrap();
    assert_eq!(third, first, "跨会话重放仍返回首果");
    assert_eq!(std::fs::read(&path).unwrap(), bytes1);
    // citations 变化则不是重放（修订通道：attempt+1 重写）
    let r5 = asm2.commit_section("SEC-0001", "稳定内容",
                                 vec!["REQ-0001".to_string()]).unwrap();
    assert_eq!(r5.attempt, 2, "citations 变化 → 修订（attempt+1）");
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================ 自研测试 ============================

/// resume_plan 断点切分：500 章提交前 100 → done=100 / todo=400 且 todo 首个
/// 是第 101 章（文档序断点）；幽灵章（已提交但不在树中）→ 显式拒。
#[test]
fn resume_plan_split_and_ghost() {
    let dir = temp_dir("plan");
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    let store = store_with_500_chapters();
    // 文档序叶 id 清单（与 assemble_all 同口径：order 升序）
    let mut secs: Vec<&OutlineNode> = store.sections().iter().collect();
    secs.sort_by_key(|n| n.order);
    let leaf_ids: Vec<String> = secs.iter().map(|n| n.id.clone()).collect();
    assert_eq!(leaf_ids.len(), 500);

    // 提交前 100 章（文档序）
    for (i, id) in leaf_ids[..100].iter().enumerate() {
        asm.commit_section(id, &format!("{id} 的正文内容"),
                           vec![format!("REQ-{:04}", i + 1)])
            .unwrap();
    }
    let (done, todo) = asm.resume_plan(&leaf_ids).unwrap();
    assert_eq!(done.len(), 100, "已完成 100");
    assert_eq!(todo.len(), 400, "待生成 400");
    assert_eq!(todo.first().map(String::as_str), Some("SEC-0101"),
               "断点 = 文档序首个未完成章（第 101 章）");
    assert_eq!(done.last().map(String::as_str), Some("SEC-0100"));
    assert_eq!(todo.last().map(String::as_str), Some("SEC-0500"));

    // 幽灵章：工作区放一个不在树中的记录 → 拒（重开纲后旧工作区不匹配）
    asm.commit_section("SEC-9999", "幽灵章内容", vec![]).unwrap();
    match asm.resume_plan(&leaf_ids) {
        Err(AssemblerError::GhostSection { section_id }) => {
            assert_eq!(section_id, "SEC-9999", "报错定位到幽灵章");
        }
        other => panic!("幽灵章必须 GhostSection 拒，实际: {:?}", other.err()),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// assemble_all 跳过面与状态推进：首轮 500 章全量 → generated=500/skipped=0；
/// 二轮（fresh assembler 同目录 + 调用计数 generator）→ generated=0/skipped=500
/// 且 **generator 零调用**（每章恰好生成一次——总 token 不含重复生成的结构
/// 证明）；store 里全部 500 章 DRAFTED（二轮对已完成章无重复推进被拒）。
#[test]
fn assemble_all_skips_done_and_advances_status() {
    let dir = temp_dir("skip");
    // —— 首轮：全量 500 章 ——
    let report1 = {
        let mut store = store_with_500_chapters();
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        let r = assemble_all(&mut store, &mut asm, &mut deterministic_gen).unwrap();
        assert_eq!(store.sections().iter()
                   .filter(|n| n.status == SectionStatus::Drafted).count(), 500,
                   "首轮后全部 500 章 DRAFTED");
        // 字数账 = Σ 逐章字数（章标题「第1章」/「第500章」长度不同——不能按单章 × 500 折算）
        let expected_total: usize = store.sections().iter()
            .map(|n| deterministic_gen(n).unwrap().chars().count()).sum();
        assert_eq!(r.total_chars as usize, expected_total,
                   "字数账 = Σ 单章字数（确定性 generator）");
        r
    };
    assert_eq!(report1.chapters_total, 500);
    assert_eq!(report1.chapters_generated, 500, "首轮全量生成");
    assert_eq!(report1.chapters_skipped, 0);
    assert!(report1.total_chars > 0, "字数账非空");

    // —— 二轮：断点续跑（fresh assembler 同目录 = 重启后重开工作区）——
    let mut store2 = store_with_500_chapters();
    let mut asm2 = ChapterAssembler::open(&dir).unwrap();
    assert_eq!(asm2.committed_count(), 500, "重启后 500 章全部在位");
    let mut gen_calls = 0usize;
    let report2 = assemble_all(&mut store2, &mut asm2, &mut |node| {
        gen_calls += 1;
        deterministic_gen(node)
    }).unwrap();
    assert_eq!(report2.chapters_generated, 0, "已完成章不重生成");
    assert_eq!(report2.chapters_skipped, 500, "全部跳过");
    assert_eq!(gen_calls, 0, "generator 零调用——总 token 不含重复生成");
    assert_eq!(report2.total_chars, report1.total_chars, "字数账不漂移");
    let drafted = store2.sections().iter()
        .filter(|n| n.status == SectionStatus::Drafted).count();
    assert_eq!(drafted, 500, "二轮后全部 500 章 DRAFTED（状态补齐幂等——无重复推进被拒）");
    let _ = std::fs::remove_dir_all(&dir);
}

/// generator 失败面（#68 阶段 2 新语义——重试 3 次 + FAILED 标记 + 不阻塞）：
/// 第 37 章 generator 恒 Err → 该章 4 次尝试（1+3 重试）后标记 FAILED，
/// **其余章继续**（500 章里 499 提交、1 失败）；失败章无章节文件（可续跑）；
/// failed.json 边车记错误明细；修复 generator 后续跑 → 从 37 章补完 500
/// （前 36 章零重生成）且失败痕迹被清除（恢复即抹痕）。
#[test]
fn generator_failure_retries_marks_failed_and_continues() {
    let dir = temp_dir("fail");
    let mut store = store_with_500_chapters();
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    let sleeps: std::cell::RefCell<Vec<u64>> = std::cell::RefCell::new(Vec::new());
    let report = bid_tools::assembly::assemble_all_with_policy(
        &mut store, &mut asm,
        &mut |node| {
            if node.id == "SEC-0037" {
                Err(format!("第 37 章生成器故障（模拟 LLM 超时）：{}", node.id))
            } else {
                deterministic_gen(node)
            }
        },
        &bid_tools::assembly::DraftPolicy { retries: 3, backoff_base_ms: 500 },
        &|ms| sleeps.borrow_mut().push(ms),
    ).unwrap();
    // —— 不阻塞面：其余 499 章全部提交，仅 37 章 FAILED ——
    assert_eq!(report.chapters_total, 500);
    assert_eq!(report.chapters_generated, 499, "失败章不阻塞其他章");
    assert_eq!(report.chapters_skipped, 0);
    assert_eq!(report.failed.len(), 1, "最终报告：恰好 1 章需人工处理");
    let f = &report.failed[0];
    assert_eq!(f.section_id, "SEC-0037");
    assert_eq!(f.attempts, 4, "首试 + 3 次重试 = 4 次尝试");
    assert!(f.error.contains("SEC-0037"), "错误原文透传: {}", f.error);
    // —— 指数退避序列：500/1000/2000（第 k 次重试前 base × 2^(k-1)）——
    assert_eq!(*sleeps.borrow(), vec![500, 1000, 2000], "指数退避序列");
    // —— 域状态：失败章 FAILED（一等结果，非「停在 DRAFTING 靠猜」）——
    assert_eq!(store.sections()[36].status, SectionStatus::Failed);
    assert_eq!(
        store.sections().iter().filter(|n| n.status == SectionStatus::Failed).count(),
        1, "全树恰好 1 个 FAILED",
    );
    assert!(!asm.has_committed("SEC-0037"), "失败章未提交（续跑从此章重来）");
    // —— failed.json 边车：错误明细在位 ——
    let failed_map = asm.failed_records();
    assert_eq!(failed_map.len(), 1);
    assert_eq!(failed_map["SEC-0037"].attempts, 4);
    assert!(failed_map["SEC-0037"].error.contains("SEC-0037"));

    // 修复 generator 后续跑 → 只补第 37 章（首轮不阻塞已提交 499——新语义
    // 的断点面从「36 已提交 + 464 待跑」变为「499 已提交 + 1 待补」）
    let mut store2 = store_with_500_chapters();
    let mut asm2 = ChapterAssembler::open(&dir).unwrap();
    let mut gen_calls2 = 0usize;
    let report = assemble_all(&mut store2, &mut asm2, &mut |node| {
        gen_calls2 += 1;
        deterministic_gen(node)
    }).unwrap();
    assert_eq!(report.chapters_generated, 1, "只补第 37 章（其余 499 已提交）");
    assert_eq!(report.chapters_skipped, 499, "499 章零重生成（token 不含重复）");
    assert_eq!(gen_calls2, 1, "generator 恰好 1 次调用");
    assert_eq!(asm2.committed_count(), 500, "最终 500 章在位");
    assert!(report.failed.is_empty(), "修复后零失败");
    // 恢复即抹痕：failed.json 中 SEC-0037 被清除
    assert!(!asm2.failed_records().contains_key("SEC-0037"));
    // FAILED → DRAFTED 的域推进（重试成功链）
    assert_eq!(store2.sections()[36].status, SectionStatus::Drafted);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 瞬时故障面：第 37 章前 2 次失败第 3 次成功 → 退避 2 次后该章正常提交，
/// 零 FAILED 零报告失败（重试的价值面）。
#[test]
fn transient_failure_recovers_within_retries() {
    let dir = temp_dir("transient");
    let mut store = store_with_500_chapters();
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    let sleeps: std::cell::RefCell<Vec<u64>> = std::cell::RefCell::new(Vec::new());
    let report = bid_tools::assembly::assemble_all_with_policy(
        &mut store, &mut asm,
        &mut |node| {
            if node.id == "SEC-0037" {
                thread_local_attempts(&node.id, 2, || deterministic_gen(node))
            } else {
                deterministic_gen(node)
            }
        },
        &bid_tools::assembly::DraftPolicy { retries: 3, backoff_base_ms: 100 },
        &|ms| sleeps.borrow_mut().push(ms),
    ).unwrap();
    assert_eq!(report.chapters_generated, 500, "全部提交（瞬时故障被重试吸收）");
    assert!(report.failed.is_empty(), "零失败章");
    assert_eq!(store.sections()[36].status, SectionStatus::Drafted);
    assert_eq!(*sleeps.borrow(), vec![100, 200], "两次退避（100/200）后成功");
    assert!(asm.failed_records().is_empty(), "无失败边车");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 退避上限防御：retries=0 → 单次尝试无退避即 FAILED（策略可关重试）。
#[test]
fn zero_retry_policy_fails_fast() {
    let dir = temp_dir("noretry");
    let mut store = store_with_500_chapters();
    let mut asm = ChapterAssembler::open(&dir).unwrap();
    let sleeps: std::cell::RefCell<Vec<u64>> = std::cell::RefCell::new(Vec::new());
    let report = bid_tools::assembly::assemble_all_with_policy(
        &mut store, &mut asm,
        &mut |node| {
            if node.id == "SEC-0001" { Err("boom".into()) } else { deterministic_gen(node) }
        },
        &bid_tools::assembly::DraftPolicy { retries: 0, backoff_base_ms: 500 },
        &|ms| sleeps.borrow_mut().push(ms),
    ).unwrap();
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].attempts, 1, "retries=0 → 单次尝试");
    assert!(sleeps.borrow().is_empty(), "无退避");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 章内计数夹具：同章前 `fail_times` 次返回 Err，之后转成功。
fn thread_local_attempts(id: &str, fail_times: usize, ok: impl FnOnce() -> Result<String, String>) -> Result<String, String> {
    use std::cell::RefCell;
    thread_local! {
        static COUNTS: RefCell<std::collections::HashMap<String, u32>> = RefCell::new(Default::default());
    }
    let key = id.to_string();
    let cur = COUNTS.with(|c| {
        let mut m = c.borrow_mut();
        let n = m.get(&key).copied().unwrap_or(0) + 1;
        m.insert(key.clone(), n);
        n
    });
    if (cur as usize) <= fail_times { Err(format!("第 {cur} 次瞬时故障")) } else { ok() }
}

/// 字节稳定面（跳过路径不触碰已完成章）：首轮全量跑完后快照全部 500 个
/// 章节文件字节 → 二轮换一个会返回**不同内容**的 generator → 断言全部
/// 500 文件字节与快照全等（即使 generator 变了也不重写——由构造保证，
/// 不靠内容比对兜底）。
#[test]
fn byte_stability_of_completed_chapters_across_rerun() {
    let dir = temp_dir("bytes");
    {
        let mut store = store_with_500_chapters();
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        assemble_all(&mut store, &mut asm, &mut deterministic_gen).unwrap();
    }
    // 快照全部 500 个章节文件字节
    let snapshot: Vec<Vec<u8>> = (1..=500)
        .map(|i| std::fs::read(chapter_file(&dir, i)).unwrap())
        .collect();

    // 二轮：generator 换成返回不同内容的版本（若无跳过路径会全部重写）
    let mut store2 = store_with_500_chapters();
    let mut asm2 = ChapterAssembler::open(&dir).unwrap();
    let report = assemble_all(&mut store2, &mut asm2, &mut |node| {
        Ok(format!("【新版】{} 的完全不同内容——若被重写即违约", node.title))
    }).unwrap();
    assert_eq!(report.chapters_generated, 0, "跳过路径不生成");
    for (i, bytes) in snapshot.iter().enumerate() {
        let cur = std::fs::read(chapter_file(&dir, i + 1)).unwrap();
        assert_eq!(&cur, bytes, "第 {} 章字节漂移——跳过路径不得触碰已完成章", i + 1);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================ 硬验收：真实进程强杀（规格原文） ============================

/// worker 内部断言：失败 → eprintln + exit(1)（子进程 panic 也会非零退出，
/// 父进程检查 exit code——两条失败路都被父侧捕获）。
fn worker_check(cond: bool, msg: &str) {
    if !cond {
        eprintln!("S-2 worker 断言失败: {msg}");
        std::process::exit(1);
    }
}

/// worker 主体（在子进程内跑）：500 章 store/assembler + 记账 generator +
/// assemble_all。`BID_S2_KILL_AT=N`（N>0）→ 第 N 次生成调用在内容构建
/// 完成后、交付/落盘前 `abort()`（SIGABRT 无清理——模拟生成完成但落盘前
/// 被 kill -9 的持久化现场）。
///
/// 记账口径（「每章恰好生成一次」的对账面）：gen-log.jsonl 一行 = 一次
/// **交付**的生成（append 一行后返回内容）。强杀点在 append 之前——被杀的
/// 在途调用不记账：它的内容从未交付、从未落盘，重启后的重生成是该章的
/// 首次交付而非重复。（若把 append 放在 abort 之前，强杀章会在日志留下
/// 501 行/重复 id——与硬验收「恰好 500 行且无重复」矛盾；两序只能取一，
/// 取验收口径。）
fn acceptance_worker(dir: &Path) {
    let kill_at: usize = std::env::var("BID_S2_KILL_AT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0); // 0 = 不杀（正常跑完）
    let mut store = store_with_500_chapters();
    let mut asm = match ChapterAssembler::open(dir) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("S-2 worker: 工作区打开失败: {e}");
            std::process::exit(1);
        }
    };
    let pre_committed = asm.committed_count(); // 续跑前已提交数（自校验基准）
    let log_path = dir.join("gen-log.jsonl");
    let mut invocations = 0usize;
    let mut gen = |node: &OutlineNode| -> Result<String, String> {
        invocations += 1;
        let content = format!(
            "{}：本章响应 {}，正文内容……（2000 字口径示意）",
            node.title,
            node.requirement_ids.join("、")
        );
        if kill_at > 0 && invocations >= kill_at {
            // 强杀点：内容已构建完成（生成已到本章）、日志未记、落盘未发生
            std::process::abort(); // SIGABRT——无析构无清理的异常终止
        }
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| e.to_string())?;
        writeln!(f, "{{\"section_id\":\"{}\"}}", node.id).map_err(|e| e.to_string())?;
        Ok(content)
    };
    let report = match assemble_all(&mut store, &mut asm, &mut gen) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("S-2 worker: assemble_all 失败: {e}");
            std::process::exit(1);
        }
    };
    // —— worker 内部对账（run 2 到达这里；run 1 死在 abort 不可达）——
    worker_check(report.chapters_total == 500, "叶节点总数 500");
    worker_check(report.chapters_skipped == pre_committed,
                 "跳过数 = 续跑前已提交数（不重跑已完成章）");
    worker_check(report.chapters_generated == 500 - pre_committed,
                 "新生成数 = 500 - 已提交数（从断点续跑）");
    worker_check(asm.committed_count() == 500, "全部 500 章在位");
    let drafted = store
        .sections()
        .iter()
        .filter(|n| n.status == SectionStatus::Drafted)
        .count();
    worker_check(drafted == 500, "store 全部 500 章 DRAFTED");
}

/// **S-2 硬验收（规格原文，真实进程级）**：生成到第 300 章（共 500 章）时
/// 强杀进程 → 重启 resume → (a) 全部 500 章在位；(b) 前 299 章逐字节不变；
/// (c) gen-log 恰好 500 行且 section_id 无重复（每章恰好生成一次——总 token
/// 不含重复生成）；(d) 断点续跑从第 300 章开始；(e) 500 章 DRAFTED。
///
/// 进程模型：`current_exe` 自引用双 spawn（Linux 本机 + Windows CI 同模式）。
/// 父进程分支跑断言；worker 分支（env `BID_S2_WORKER`）跑组装，`BID_S2_KILL_AT`
/// 控制强杀点，SIGABRT 模拟 kill -9（无清理路径）。
#[test]
fn acceptance_kill_at_300_resume_byte_identical() {
    // —— worker 分支：由父进程 spawn 的子进程体 ——
    if std::env::var("BID_S2_WORKER").is_ok() {
        let dir = match std::env::var("BID_S2_DIR") {
            Ok(d) => PathBuf::from(d),
            Err(_) => {
                eprintln!("S-2 worker: 缺 BID_S2_DIR");
                std::process::exit(1);
            }
        };
        acceptance_worker(&dir);
        std::process::exit(0);
    }

    // —— 父进程分支：两次 spawn 子进程 ——
    let dir = temp_dir("kill");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let test_name = "acceptance_kill_at_300_resume_byte_identical";

    // 第一次：跑到第 300 章生成时强杀（SIGABRT）
    let out1 = std::process::Command::new(&exe)
        .args(["--exact", test_name])
        .env("BID_S2_WORKER", "1")
        .env("BID_S2_DIR", &dir)
        .env("BID_S2_KILL_AT", "300")
        .output()
        .expect("spawn worker-1");
    // 异常终止判定（跨平台）：Unix = 信号杀（code()=None，下方精确断言
    // SIGABRT=6）；Windows = abort 非零退出（fastfail/ucrt 码随工具链——
    // 以磁盘法医为准绳：committed=299 等现场断言排除「早退失败」的伪强杀）
    let abnormal = if cfg!(unix) {
        out1.status.code().is_none()
    } else {
        out1.status.code() != Some(0)
    };
    assert!(
        abnormal,
        "worker-1 必须异常终止（强杀），实际: {:?}；stderr: {}",
        out1.status,
        String::from_utf8_lossy(&out1.stderr)
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            out1.status.signal(),
            Some(6),
            "SIGABRT（abort——无清理路径）；stderr: {}",
            String::from_utf8_lossy(&out1.stderr)
        );
    }

    // —— 强杀现场勘验：恰好 299 章已提交、第 300 章无记录（可续跑面）——
    let asm = ChapterAssembler::open(&dir).unwrap();
    assert_eq!(asm.committed_count(), 299, "强杀时恰好 299 章已提交");
    assert!(!asm.has_committed("SEC-0300"), "第 300 章生成完未落盘——无记录");
    let snapshot: Vec<Vec<u8>> = (1..=299)
        .map(|i| std::fs::read(chapter_file(&dir, i)).unwrap())
        .collect();
    let log1 = std::fs::read_to_string(dir.join("gen-log.jsonl")).unwrap();
    assert_eq!(log1.lines().count(), 299, "强杀前恰好 299 次生成交付（在途章不记账）");

    // 第二次：无 KILL_AT → 正常跑完（重启 resume）
    let out2 = std::process::Command::new(&exe)
        .args(["--exact", test_name])
        .env("BID_S2_WORKER", "1")
        .env("BID_S2_DIR", &dir)
        .output()
        .expect("spawn worker-2");
    assert_eq!(
        out2.status.code(),
        Some(0),
        "worker-2 必须正常完成（含内部 500 章 DRAFTED 断言）；stderr: {}",
        String::from_utf8_lossy(&out2.stderr)
    );

    // (a) 全部 500 章在位
    let asm2 = ChapterAssembler::open(&dir).unwrap();
    assert_eq!(asm2.committed_count(), 500, "(a) 全部 500 章在位");

    // (b) 前 299 章逐字节不变（跳过路径永不触碰已完成章）
    for (i, bytes) in snapshot.iter().enumerate() {
        let cur = std::fs::read(chapter_file(&dir, i + 1)).unwrap();
        assert_eq!(&cur, bytes, "(b) 第 {} 章字节漂移", i + 1);
    }

    // (c) gen-log 恰好 500 行且 section_id 无重复（每章恰好生成一次——总
    // token 不含重复生成）
    let log2 = std::fs::read_to_string(dir.join("gen-log.jsonl")).unwrap();
    let ids: Vec<String> = log2
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l)
                .unwrap_or_else(|e| panic!("gen-log 行不是 JSON: {l}（{e}）"));
            v["section_id"].as_str().expect("section_id 字段").to_string()
        })
        .collect();
    assert_eq!(ids.len(), 500, "(c) 恰好 500 行");
    let mut seen = HashSet::new();
    for id in &ids {
        assert!(seen.insert(id.clone()), "(c) section_id 重复: {id}（重复生成！）");
    }

    // (d) 断点续跑从第 300 章开始：run 1 贡献前 299 行，run 2 贡献后 201 行
    // （500 - 299 = 201；规格原文「后 301 行」按此取正——500 总行数下 run 2
    // 段只能是 201 行），且全 500 行恰按文档序 SEC-0001..SEC-0500 一行一章
    // （「从第 300 章继续，不重跑前 299 章」的逐行证明）
    assert_eq!(ids.len() - 299, 201, "(d) 续跑段 = 201 章（第 300..=500 章）");
    for (i, id) in ids.iter().enumerate() {
        let expect = format!("SEC-{:04}", i + 1);
        assert_eq!(id, &expect, "(d) 第 {} 行应为 {}（文档序逐章）", i + 1, expect);
    }

    // (e) 500 章 DRAFTED：worker-2 内部已断言（exit 0 为证）；父侧以工作区
    // 全量复核（store 是 worker 进程内存态——其持久化等价物 = chapters/
    // 全量 500 文件 + 无 tmp 残留）
    assert_eq!(asm2.committed_count(), 500);
    let tmp_leftovers: Vec<_> = std::fs::read_dir(dir.join(CHAPTERS_SUBDIR)).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(tmp_leftovers.is_empty(), "(e) 零 tmp 残留（原子提交纪律）");

    let _ = std::fs::remove_dir_all(&dir);
}
