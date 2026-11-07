//! export_stream.rs —— S-6 组装与流式导出验收（issue #11 批次 v3 / V3-5）。
//!
//! 硬验收（issue #11 规格原文）：「500 章 / 100 万字导出，峰值内存 < 1GB，
//! Word 打开不弹修复」。落地断言面：
//! - 500 章 × 恰 2000 字（层级树：25 个一级部件 × 20 个二级叶章）→ 导出
//!   docx：zip 结构完整（读入全量校验 CRC）、document.xml 良构、恰 525 个
//!   Heading 段（25 结构 + 500 叶）、**TOC 域指令存在**（`TOC \o` 字样）；
//! - 峰值内存：VmHWM < 1GB（1073741824B = 1048576kB），实测值 --nocapture
//!   打印（预期远低于阈值——Lead spike：docx-rs 模型 ~31MB）；
//! - 幂等 no-op：同输入二次导出 → 报告全等、输出 mtime/字节不变（一个
//!   字节不重写——「已写入的章不重写」的导出层落点）；
//! - 输入变化（重提交 attempt+1）→ 全量重建：输出 sha 变化、manifest 更新；
//! - fail-closed：缺记录 / 未 DRAFTED / 幽灵记录 / 记录挂结构节点 / 空树 /
//!   孤儿节点 → 拒绝导出且错误信息含章 id（半个文档不猜测）；
//! - tmp+rename 原子性（borrow：mdcore `persistence_recovery.rs::
//!   interrupted_publish_leftover_does_not_break_recovery`）：残骸 .docx.tmp
//!   存在 → 静默丢弃、导出正常完成；
//! - 确定性：删 manifest 强制两次全新构建 → 解包后的 document.xml 逐字节
//!   相等（docx zip 的部件时间戳在 zip 8.6 恒为 DateTime::DEFAULT
//!   1980-01-01，实际整包也是字节确定——但按规格口径断言 document.xml）；
//! - 「Word 打开不弹修复」本地代理口径：[Content_Types].xml / _rels/.rels /
//!   word/document.xml 三件齐全 + 每个 xml/rels 部件 quick-xml 全量解析
//!   良构（无真 Word 的本地最严等价——修复弹窗的主因是部件缺失/坏 XML）。
//!
//! SKIP 面（issue #9 纪律，borrow review_sharding.rs）：非 Linux 平台无
//! /proc/self/status 的 VmHWM 口径 → 响亮横幅 + 提前 return（环境缺件不是
//! 业务失败，也绝不静默假装通过）。

use bid_domain::outline::{OutlineNode, SectionStatus};
use bid_domain::{Anchor, DomainStore, ParseStatus, Requirement};
use bid_tools::assembly::{ChapterAssembler, CHAPTERS_SUBDIR};
use bid_tools::export::{
    DocumentExporter, ExportError, ExportManifest, ExportReport, MANIFEST_FILE,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 夹具

/// 章配额（规格口径：500 章 × 2000 字 = 恰 100 万字）。
const TARGET: usize = 2000;
/// 一级部件数（层级树：25 × 20 = 500 叶）。
const PARTS: usize = 25;
/// 每部件叶章数。
const LEAVES_PER_PART: usize = 20;

/// 500 章基座（照 assembly_checkpoint.rs 的 store_with_500_chapters 模式，
/// 升级为**层级树**：25 个 level-1 结构部件（SEC-P00..SEC-P24）+ 500 个
/// level-2 叶章（SEC-0001..SEC-0500）。order 全局唯一且父先于子（p*21+1
/// 是部件，p*21+2+k 是其叶）——DFS 全序 == order 全序，跨进程重建全等。
fn store_with_500_chapter_tree() -> DomainStore {
    let mut s = DomainStore::new();
    s.create_project("S-6 导出验收项目", "ZFCG-2026-S6-001", "明标", "s6-key").unwrap();
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
                excerpt: format!("S-6 条款-{i} 的唯一原文摘录"),
                    block_idx: None,
                    page: None,
                    line: None
            }],
            params: std::collections::HashMap::new(),
            source_file: "A".into(),
        })
        .collect();
    s.set_requirements(reqs).unwrap();
    let mut nodes: Vec<OutlineNode> = Vec::with_capacity(PARTS + 500);
    for p in 0..PARTS {
        // 部件（结构节点）：requirement_ids = 子并集、target_chars = 子和
        // （set_outline 不变量 9——反范式聚合一致性）
        let mut union: Vec<String> = Vec::new();
        for k in 0..LEAVES_PER_PART {
            let i = p * LEAVES_PER_PART + k; // 0 基叶号
            union.push(format!("REQ-{:04}", i + 1));
        }
        nodes.push(OutlineNode {
            id: format!("SEC-P{p:02}"),
            parent_id: None,
            title: format!("第{}部分 实施方案（{p:02}）", p + 1),
            level: 1,
            order: (p * (LEAVES_PER_PART + 1) + 1) as u32,
            requirement_ids: union,
            target_chars: (LEAVES_PER_PART * TARGET) as u32,
            status: SectionStatus::Pending, // 结构节点不推进（S-2 语义）
            dirty: false,
            content: None, content_attempt: 0,
        });
        for k in 0..LEAVES_PER_PART {
            let i = p * LEAVES_PER_PART + k;
            nodes.push(OutlineNode {
                id: format!("SEC-{:04}", i + 1),
                parent_id: Some(format!("SEC-P{p:02}")),
                title: format!("技术参数响应（第{}章）", i + 1),
                level: 2,
                order: (p * (LEAVES_PER_PART + 1) + 2 + k) as u32,
                requirement_ids: vec![format!("REQ-{:04}", i + 1)],
                target_chars: TARGET as u32,
                status: SectionStatus::Pending,
                dirty: false,
                content: None, content_attempt: 0,
            });
        }
    }
    s.set_outline(nodes).unwrap();
    s
}

/// 第 i 章（0 基）正文：恰 2000 字（含一个换行——切段面覆盖），确定性
/// 纯函数（filler 词表 borrow review_sharding.rs 的 mk_chapter）。
fn mk_content(i: usize) -> String {
    let filler = "本章围绕实施方案与现场条件展开论述，逐项明确人员配置、质量保障、安全文明施工等要求，确保各项措施责任到人、落地可行。";
    let mut base = String::with_capacity(TARGET * 3);
    base.push_str(&format!("第{}章响应正文。", i + 1));
    while base.chars().count() < TARGET - 1 {
        let need = TARGET - 1 - base.chars().count();
        base.extend(filler.chars().take(need));
    }
    // 中点插一个换行（总字数 = 1999 + 1 换行 = 恰 2000——char_count 含换行）
    let chars: Vec<char> = base.chars().collect();
    let mid = chars.len() / 2;
    let mut out = String::with_capacity(TARGET * 3 + 1);
    out.extend(&chars[..mid]);
    out.push('\n');
    out.extend(&chars[mid..]);
    debug_assert_eq!(out.chars().count(), TARGET);
    out
}

/// 500 章工作区 + 全部叶 DRAFTED 的完整态（验收主夹具——store 供章节树）。
/// 返回 (store, dir)：store.sections() 是导出器的树输入。
fn committed_500_workspace(tag: &str) -> (DomainStore, PathBuf) {
    let dir = temp_dir(tag);
    let mut store = store_with_500_chapter_tree();
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        for i in 0..500 {
            let id = format!("SEC-{:04}", i + 1);
            // 叶状态推进到 DRAFTED（S-2 assemble_all 的等价终态）
            store.advance_section_status(&id, SectionStatus::Outlining).unwrap();
            store.advance_section_status(&id, SectionStatus::Drafting).unwrap();
            store.advance_section_status(&id, SectionStatus::Drafted).unwrap();
            asm.commit_section(&id, &mk_content(i), vec![format!("REQ-{:04}", i + 1)])
                .unwrap();
        }
        assert_eq!(asm.committed_count(), 500);
    }
    (store, dir)
}

/// 小树节点构造（fail-closed 面夹具——手工 OutlineNode，不进 DomainStore）。
fn node(id: &str, parent: Option<&str>, level: u8, order: u32, status: SectionStatus) -> OutlineNode {
    OutlineNode {
        id: id.into(),
        parent_id: parent.map(str::to_string),
        title: format!("{id} 标题"),
        level,
        order,
        requirement_ids: vec!["REQ-0001".into()],
        target_chars: 100,
        status,
        dirty: false,
        content: None, content_attempt: 0,
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bid-s6-exp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Linux /proc/self/status 的 VmHWM（进程峰值 RSS，单位 kB）。
/// 非 Linux / 无 procfs → None（SKIP 面——issue #9 纪律，不伪装通过；
/// borrow：review_sharding.rs 同款口径）。
fn vmhwm_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|l| {
        l.strip_prefix("VmHWM:")
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse().ok())
    })
}

// ---------------------------------------------------------------- 解包校验辅助

/// 读 zip 条目全量字节（zip 4 的 Crc32Reader 在读尽时校验 CRC——「zip 结构
/// 完整」的本地口径）。
fn unzip_read(path: &Path, name: &str) -> Vec<u8> {
    let f = std::fs::File::open(path).unwrap();
    let mut zf = zip::ZipArchive::new(f).unwrap();
    let mut entry = zf.by_name(name).unwrap_or_else(|e| panic!("zip 条目缺失 {name}: {e}"));
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut buf).unwrap(); // CRC 在读尽时校验
    buf
}

/// 「Word 打开不弹修复」的本地代理断言器：
/// 1. [Content_Types].xml / _rels/.rels / word/document.xml 三件齐全；
/// 2. 包内全部 .xml/.rels 部件 quick-xml 全量解析良构（修复弹窗的主因是
///    部件缺失 / 坏 XML / zip CRC 坏——三者都被本断言器与 unzip_read 覆盖）。
fn assert_word_repair_proxy(path: &Path) {
    let f = std::fs::File::open(path).unwrap();
    let mut zf = zip::ZipArchive::new(f).unwrap();
    for must in ["[Content_Types].xml", "_rels/.rels", "word/document.xml"] {
        assert!(zf.by_name(must).is_ok(), "OPC 三件套缺失: {must}");
    }
    let names: Vec<String> = zf.file_names().map(str::to_string).collect();
    for name in names {
        if !(name.ends_with(".xml") || name.ends_with(".rels")) {
            continue; // 目录条目/媒体——非 XML 部件
        }
        let mut entry = zf.by_name(&name).unwrap();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf).unwrap();
        assert_well_formed(&name, &buf);
    }
}

/// XML 良构断言（quick-xml 事件流全量走一遍——borrow：bid-parse docx.rs 的
/// Reader 用法同源）。
fn assert_well_formed(name: &str, bytes: &[u8]) {
    use quick_xml::events::Event;
    let text = std::str::from_utf8(bytes).unwrap_or_else(|e| panic!("{name} 非 UTF-8: {e}"));
    let mut reader = quick_xml::Reader::from_str(text);
    reader.config_mut().trim_text(false);
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => panic!("XML 部件 {name} 非良构（Word 修复弹窗代理口径）: {e}"),
        }
    }
}

/// 读取 manifest（测试观察面——读盘后反序列化，与生产面同源）。
fn read_manifest(dir: &Path) -> Option<ExportManifest> {
    let raw = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    Some(serde_json::from_str(&raw).unwrap())
}

// ---------------------------------------------------------------- 1+8. 规格硬验收：500 章 / 100 万字

/// 规格硬验收：500 章 × 恰 2000 字（层级树）→ 导出 docx 后校验：
/// 输出存在；zip 三件套 + 全 XML 部件良构（Word 修复弹窗代理）；document.xml
/// 良构且恰 525 个 Heading 段（25 结构 + 500 叶）；**TOC 域指令存在**
/// （`TOC \o` 字样，XML 引号转义形态 `&quot;`）；报告账面 500 章 / 恰
/// 100 万字 / toc_field_present。
#[test]
fn acceptance_500_chapters_million_chars_docx() {
    let (store, dir) = committed_500_workspace("acc");
    let out = dir.join("技术标书-导出样本.docx");
    let report = DocumentExporter::new(store.sections()).export(&dir, &out).unwrap();

    // 报告账面：500 章 / 恰 1,000,000 字（夹具每章恰 2000——含换行口径）
    assert_eq!(report.chapters_written, 500, "恰 500 章写入");
    assert_eq!(report.total_chars, 1_000_000, "恰 100 万字（500 × 2000）");
    assert!(report.toc_field_present, "TOC 域构造位");
    assert!(!report.output_sha256.is_empty(), "产物指纹非空");
    eprintln!(
        "[S-6 验收] 500 章 / {} 字 / {}ms / sha={}",
        report.total_chars,
        report.duration_ms.unwrap_or(0),
        &report.output_sha256[..16]
    );

    // 输出存在 + 「Word 打开不弹修复」本地代理（测试 8 同体覆盖：三件套 +
    // 全 XML 部件良构 + zip CRC——unzip_read 读尽校验）
    assert!(out.is_file(), "输出文件存在");
    assert_word_repair_proxy(&out);

    // document.xml：良构 + 恰 525 个 Heading 段 + TOC 域指令 + 章标题在位
    let doc = unzip_read(&out, "word/document.xml");
    let text = std::str::from_utf8(&doc).unwrap();
    assert_well_formed("word/document.xml", &doc);
    let heading_count = text.matches("<w:pStyle w:val=\"Heading").count();
    assert_eq!(heading_count, 525, "25 结构节点 + 500 叶 = 恰 525 个 Heading 段");
    assert!(
        text.contains(r#"TOC \o &quot;1-3&quot; \h \u"#),
        "TOC 域指令存在（规格原文 TOC \\o \"1-3\" \\h \\u 的 XML 转义形态）"
    );
    assert!(text.contains("技术参数响应（第1章）"), "首叶标题在位");
    assert!(text.contains("技术参数响应（第500章）"), "末叶标题在位");
    assert!(text.contains("第25部分 实施方案（24）"), "结构节点标题在位");
    // 首章正文锚（切段面：换行切成两段——首段以锚句开头）
    assert!(text.contains("第1章响应正文。"), "章正文进文档（只读已提交记录）");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 2. 峰值内存硬标准

/// 规格硬标准：500 章 / 100 万字导出，峰值内存 < 1GB（1073741824B =
/// 1048576kB）。VmHWM 是进程历史峰值（含 harness 与同进程先跑测试的驻留）
/// ——绝对值口径，基线先行记录、增量面一并打印供归因。非 Linux：SKIP
/// 横幅（issue #9 纪律——不伪装通过）。
#[test]
fn acceptance_peak_memory_under_1gb() {
    let Some(baseline) = vmhwm_kb() else {
        eprintln!("[SKIP] ENV_MISSING: /proc/self/status——非 Linux 平台无 VmHWM 口径");
        return;
    };
    let (store, dir) = committed_500_workspace("mem");
    let out = dir.join("out.docx");
    let report = DocumentExporter::new(store.sections()).export(&dir, &out).unwrap();
    assert_eq!(report.chapters_written, 500, "内存验收夹具与验收测试同构——500 章全量导出");

    let peak = vmhwm_kb().unwrap_or(baseline);
    eprintln!(
        "[S-6 内存验收] 基线 VmHWM={baseline}kB → 导出后 VmHWM={peak}kB（阈值 1048576kB / 1GB，增量 {}kB）",
        peak.saturating_sub(baseline)
    );
    assert!(
        peak < 1_048_576,
        "内存峰值 {peak}kB ≥ 1GB（1073741824B）——违反 S-6 规格硬标准"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 3. 幂等 no-op

/// 同输入二次导出：报告全等（含 duration_ms——no-op 直返上次报告）、输出
/// 字节与 mtime 不变（一个字节不重写）。manifest 在位且指纹数 = 500。
#[test]
fn idempotent_rerun_is_byte_level_noop() {
    let (store, dir) = committed_500_workspace("noop");
    let out = dir.join("out.docx");
    let exporter = DocumentExporter::new(store.sections());
    let first = exporter.export(&dir, &out).unwrap();
    let bytes1 = std::fs::read(&out).unwrap();
    let mtime1 = std::fs::metadata(&out).unwrap().modified().unwrap();

    // 二次导出：输入未变 → no-op（输出零重写）
    let second = exporter.export(&dir, &out).unwrap();
    assert_eq!(second, first, "no-op 返回上次报告全等（含 duration_ms）");
    assert_eq!(std::fs::read(&out).unwrap(), bytes1, "输出文件一个字节都不动");
    assert_eq!(
        std::fs::metadata(&out).unwrap().modified().unwrap(),
        mtime1,
        "未走写路径——mtime 不变"
    );

    // manifest 在位：指纹按文档序 500 条 + 上次报告
    let manifest = read_manifest(&dir).expect("幂等清单在位");
    assert_eq!(manifest.fingerprints.len(), 500);
    assert_eq!(manifest.fingerprints[0].section_id, "SEC-0001");
    assert_eq!(manifest.fingerprints[499].section_id, "SEC-0500");
    assert_eq!(manifest.report, first, "manifest 记录上次报告");

    // 跨会话重放（fresh exporter 同目录 = 重启后重开导出器）仍 no-op
    let third = DocumentExporter::new(store.sections()).export(&dir, &out).unwrap();
    assert_eq!(third, first, "跨会话重放仍直返上次报告");
    assert_eq!(std::fs::read(&out).unwrap(), bytes1);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 4. 输入变化触发重建

/// 重提交一章（attempt+1、内容变化）→ 指纹变 → 全量重建：输出 sha 变化、
/// manifest 指纹更新（attempt=2）、chapters_written 仍 500（全量重写不丢章）。
#[test]
fn input_change_triggers_full_rebuild() {
    let (store, dir) = committed_500_workspace("rebuild");
    let out = dir.join("out.docx");
    let exporter = DocumentExporter::new(store.sections());
    let first = exporter.export(&dir, &out).unwrap();
    let bytes1 = std::fs::read(&out).unwrap();

    // 第 250 章重提交（不同内容 → attempt+1 修订；**等长修订**——替换前缀
    // 与替换文同为 10 个字符，字数账 1,000,000 不变，隔离「内容变」单一变量。
    // 注：String::replace_range 吃的是**字节**区间（中文 3B/字）——初版
    // `0..6` 只换了 2 个字却塞进 6 个字（+2 字污染字数账），已按字符边界修正。
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        let mut revised = mk_content(249);
        const PREFIX: &str = "第250章响应正文。"; // mk_content(249) 的 10 字符前缀
        const REVISED: &str = "这是修订后的章正文。"; // 同为 10 字符（30 字节）
        assert_eq!(PREFIX.chars().count(), REVISED.chars().count());
        assert!(revised.starts_with(PREFIX));
        revised.replace_range(0..PREFIX.len(), REVISED);
        let rec = asm.commit_section("SEC-0250", &revised, vec!["REQ-0250".into()]).unwrap();
        assert_eq!(rec.attempt, 2, "修订 attempt=2");
    }

    let second = exporter.export(&dir, &out).unwrap();
    assert_ne!(second.output_sha256, first.output_sha256, "输入变 → 输出 sha 变");
    assert_ne!(std::fs::read(&out).unwrap(), bytes1, "输出字节已重建");
    assert_eq!(second.chapters_written, 500, "全量重建不丢章");
    assert_eq!(second.total_chars, 1_000_000, "字数账不变（等长修订）");
    let manifest = read_manifest(&dir).unwrap();
    assert_eq!(manifest.fingerprints[249].attempt, 2, "manifest 指纹已更新（attempt=2）");
    assert_eq!(manifest.fingerprints[249].section_id, "SEC-0250");

    // 三次导出（新输入）→ 又是 no-op（指纹稳定后收敛）
    let third = DocumentExporter::new(store.sections()).export(&dir, &out).unwrap();
    assert_eq!(third, second, "重建后输入稳定 → 再次 no-op");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 5. fail-closed 导出门

/// 半个文档不猜测：缺记录 / 未 DRAFTED / 幽灵记录 / 记录挂结构节点 / 空树 /
/// 孤儿节点 → 全部显式拒且错误信息含章 id；输出不产出。
#[test]
fn fail_closed_rejects_incomplete_workspace() {
    let leaf_ready = || node("SEC-0001", None, 1, 1, SectionStatus::Drafted);

    // —— 缺 ChapterRecord：1 叶树、零提交 → MissingRecord 含章 id
    let dir = temp_dir("fc-missing");
    let out = dir.join("out.docx");
    let nodes = vec![leaf_ready()];
    match DocumentExporter::new(&nodes).export(&dir, &out) {
        Err(ExportError::MissingRecord { section_id }) => {
            assert_eq!(section_id, "SEC-0001");
        }
        other => panic!("缺记录必须 MissingRecord 拒，实际: {:?}", other.err()),
    }
    assert!(!out.exists(), "拒绝路径绝不产出半个文档");

    // —— 未 DRAFTED：提交了但状态停在 DRAFTING → UndraftedSection 含章 id
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        asm.commit_section("SEC-0001", "已生成但未定稿的内容", vec!["REQ-0001".into()])
            .unwrap();
    }
    let drafting = vec![node("SEC-0001", None, 1, 1, SectionStatus::Drafting)];
    match DocumentExporter::new(&drafting).export(&dir, &out) {
        Err(ExportError::UndraftedSection { section_id, status }) => {
            assert_eq!(section_id, "SEC-0001");
            assert_eq!(status, "DRAFTING", "错误透传当前状态词（xref C7 口径）");
        }
        other => panic!("未 DRAFTED 必须 UndraftedSection 拒，实际: {:?}", other.err()),
    }
    assert!(!out.exists(), "拒绝路径绝不产出半个文档");

    // —— DRAFTED 后放行（同工作区补状态 → 成功——门的可达恢复面）
    let ok = DocumentExporter::new(&nodes).export(&dir, &out).unwrap();
    assert_eq!(ok.chapters_written, 1, "补齐状态后导出成功");
    assert!(out.is_file());

    // —— 幽灵记录：提交一个不在树中的章 → GhostSection 含章 id
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        asm.commit_section("SEC-9999", "幽灵章内容", vec![]).unwrap();
    }
    match DocumentExporter::new(&nodes).export(&dir, &out) {
        Err(ExportError::GhostSection { section_id }) => {
            assert_eq!(section_id, "SEC-9999");
        }
        other => panic!("幽灵记录必须 GhostSection 拒，实际: {:?}", other.err()),
    }

    // —— 记录挂结构节点：父节点有记录 → RecordOnStructuralNode
    let dir2 = temp_dir("fc-struct");
    {
        let mut asm = ChapterAssembler::open(&dir2).unwrap();
        asm.commit_section("SEC-P00", "挂在结构节点上的怪态记录", vec!["REQ-0001".into()])
            .unwrap();
    }
    let tree = vec![
        node("SEC-P00", None, 1, 1, SectionStatus::Pending), // 结构节点（Pending 合法——不查）
        node("SEC-0001", Some("SEC-P00"), 2, 2, SectionStatus::Drafted),
    ];
    match DocumentExporter::new(&tree).export(&dir2, &dir2.join("out.docx")) {
        Err(ExportError::RecordOnStructuralNode { section_id }) => {
            assert_eq!(section_id, "SEC-P00");
        }
        other => panic!("结构节点记录必须拒，实际: {:?}", other.err()),
    }

    // —— 空树 → EmptyTree
    match DocumentExporter::new(&[]).export(&dir2, &dir2.join("out.docx")) {
        Err(ExportError::EmptyTree) => {}
        other => panic!("空树必须 EmptyTree 拒，实际: {:?}", other.err()),
    }

    // —— 孤儿节点（父引用不存在）→ UnreachableSection 含章 id
    let orphan = vec![
        node("SEC-0001", None, 1, 1, SectionStatus::Drafted),
        node("SEC-0002", Some("NOPE"), 2, 2, SectionStatus::Drafted),
    ];
    match DocumentExporter::new(&orphan).export(&dir2, &dir2.join("out.docx")) {
        Err(ExportError::UnreachableSection { section_id }) => {
            assert_eq!(section_id, "SEC-0002", "报错定位到孤儿");
        }
        other => panic!("孤儿必须 UnreachableSection 拒，实际: {:?}", other.err()),
    }

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

// ---------------------------------------------------------------- 6. tmp+rename 原子性（borrow：mdcore interrupted_publish_leftover_does_not_break_recovery）

/// 导出中断模拟：写一半的 `{out}.docx.tmp` 残骸存在 → 再次 export 正常完成
/// （残骸静默丢弃、rename 原子顶替、零 tmp 残留）。分两个面：
/// (a) 全新导出遇残骸；(b) 已有产物 + 输入变化触发重建时遇残骸。
#[test]
fn leftover_tmp_does_not_break_export() {
    // (a) 全新导出遇残骸（borrow：mdcore interrupted_publish_leftover… 模式）
    let (store, dir) = committed_500_workspace("tmp");
    let out = dir.join("out.docx");
    let tmp = dir.join("out.docx.tmp");
    std::fs::write(&tmp, b"garbage from interrupted export (half-written zip)").unwrap();
    let report = DocumentExporter::new(store.sections()).export(&dir, &out).unwrap();
    assert_eq!(report.chapters_written, 500, "残骸不破导出");
    assert!(out.is_file());
    assert!(!tmp.exists(), "残骸被静默丢弃（导出后零 tmp 残留）");
    assert_word_repair_proxy(&out); // 产物仍是完整良构 docx

    // (b) 产物在位 + 输入变化 + 残骸 → 重建路径同样吞掉残骸
    {
        let mut asm = ChapterAssembler::open(&dir).unwrap();
        asm.commit_section("SEC-0001", "触发重建的修订内容（换掉整章）", vec!["REQ-0001".into()])
            .unwrap();
    }
    std::fs::write(&tmp, b"second garbage from interrupted rebuild").unwrap();
    let rebuilt = DocumentExporter::new(store.sections()).export(&dir, &out).unwrap();
    assert_eq!(rebuilt.chapters_written, 500);
    assert_ne!(rebuilt.output_sha256, report.output_sha256, "重建确实发生");
    assert!(!tmp.exists(), "重建后零 tmp 残留");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 7. 确定性（同输入两次全新构建）

/// 删 manifest 强制重建：同输入两次全新导出 → 解包后的 document.xml 逐字节
/// 相等（docx zip 部件时间戳在 zip 8.6 恒为 DateTime::DEFAULT 1980-01-01、
/// docx-rs paraId 逐次 build 前复位——实际整包也字节确定；按规格口径断言
/// document.xml，整包相等作附带观察打印不断言）。
#[test]
fn deterministic_rebuild_document_xml_identical() {
    let (store, dir) = committed_500_workspace("det");
    let out_a = dir.join("a.docx");
    let out_b = dir.join("b.docx");
    let exporter = DocumentExporter::new(store.sections());

    // 第一次全新构建
    let _r1 = exporter.export(&dir, &out_a).unwrap();
    std::fs::remove_file(dir.join(MANIFEST_FILE)).unwrap(); // 删 manifest 强制重建
    let _r2 = exporter.export(&dir, &out_b).unwrap();

    let doc_a = unzip_read(&out_a, "word/document.xml");
    let doc_b = unzip_read(&out_b, "word/document.xml");
    assert_eq!(doc_a, doc_b, "同输入两次全新构建的 document.xml 逐字节相等");

    // 附带观察：整包是否也字节确定（zip 8.6 部件 mtime 恒 1980-01-01）
    let whole_a = std::fs::read(&out_a).unwrap();
    let whole_b = std::fs::read(&out_b).unwrap();
    eprintln!(
        "[S-6 确定性] document.xml {}B 全等；整包 docx {}B vs {}B {}",
        doc_a.len(),
        whole_a.len(),
        whole_b.len(),
        if whole_a == whole_b { "全等" } else { "不等（zip 元数据面差异，不断言）" }
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 补充面：ExportReport 序列化往返

/// 报告/清单的 serde 往返（manifest 落盘格式的稳定性——no-op 直返的上游）。
#[test]
fn report_manifest_serde_roundtrip() {
    let report = ExportReport {
        chapters_written: 3,
        total_chars: 6000,
        duration_ms: Some(42),
        output_sha256: "deadbeef".into(),
        toc_field_present: true,
        // #117 表格/格式件对账面（旧清单无这些字段 → serde default 兼容读入）
        tables_written: 6,
        // #317 暗标清理报告（明标导出不在场——serde default 兼容读入同款）
        dark_bid: None,
        table_kinds: vec!["技术参数偏离表".into()],
        forms_written: 0,
        form_titles: vec![],
    };
    let json = serde_json::to_string(&report).unwrap();
    let back: ExportReport = serde_json::from_str(&json).unwrap();
    assert_eq!(back, report);
    // 旧版清单（无 #117 字段）→ default 兼容读入（加性演进面）
    let legacy = r#"{"chapters_written":1,"total_chars":10,"duration_ms":null,
        "output_sha256":"ab","toc_field_present":true}"#;
    let old: ExportReport = serde_json::from_str(legacy).unwrap();
    assert_eq!((old.tables_written, old.table_kinds.len()), (0, 0), "旧清单零表兼容");
    // manifest 字段名稳定（磁盘格式的兼容面）
    assert!(json.contains("\"chapters_written\":3"));
    assert!(json.contains("\"output_sha256\":\"deadbeef\""));
}

// ---------------------------------------------------------------- 补充面：chapters 工作区与导出物隔离

/// 章节文件残留 tmp（S-2 面的中断残骸）不破导出——ChapterAssembler::open
/// 静默跳过 `*.json.tmp`，导出照常（两个原子层互不干扰）。
#[test]
fn chapter_tmp_leftover_does_not_break_export() {
    let (store, dir) = committed_500_workspace("chtmp");
    let junk = dir.join(CHAPTERS_SUBDIR).join("SEC-0001.json.tmp");
    std::fs::write(&junk, b"interrupted chapter commit").unwrap();
    let out = dir.join("out.docx");
    let report = DocumentExporter::new(store.sections()).export(&dir, &out).unwrap();
    assert_eq!(report.chapters_written, 500, "章级 tmp 残骸不破导出（S-2 语义延伸）");
    assert!(out.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}
