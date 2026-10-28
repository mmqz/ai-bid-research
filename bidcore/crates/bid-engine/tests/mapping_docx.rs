//! B5 映射保真测试：`docx_to_parsed_file` 黄金锚点（B4 固化）。
//!
//! B4（issue #11 任务 3）：自研 docx 解析删除后 legacy 对照面不复存在——
//! 原「引擎 vs 旧路径」差分断言改造为**固化常量锚点**。取值时点：mdcore
//! @3204342d（PR #31 r3：全量差分 EQ+ED 91.57% 验收，A 件等抽样 100% EQ）。
//! 锚点语义：引擎 bump 或映射回归的**防回归锚**——任何数值漂移必须来自
//! 显式的 mdcore rev 升级 PR（重跑差分后同步更新锚点），禁止静默漂移。
//!
//! 双轨测试设计（issue #11 测试纪律）：
//! - **自研**：块数/块型分布/表数/章节 locator 口径/表行口径/账本登记面
//!   （数值锚点 + 逐块抽查）；
//! - **借鉴 mdcore**（fixtures/engine-borrowed/TR2，PROVENANCE.md 溯源）：
//!   word-complex.docx 统计面（tables/comments/fields）经引擎读回后账本一致。

use bid_engine::{parse_docx_via_engine, EngineReader, MDCORE_REV};
use std::path::Path;

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// 真实样本在位预检（issue #9 SKIP 机制文件面——PR #24 修复批次）：
/// 该测试文件依赖的 fixture 目录缺失 → 响亮横幅 + 调用方立即 return
/// （缺件不是业务失败，也绝不静默假装通过）。
fn fixtures_ready(test_id: &str) -> bool {
    let dirs: Vec<std::path::PathBuf> = ["../../../fixtures/corpus-A/files",
        "../../../fixtures/corpus-R/D2-nonbody",
        "../../../fixtures/corpus-R/D1-tables",
        "../../../fixtures/engine-borrowed/TR2"]
        .iter()
        .map(|d| fixture(d))
        .collect();
    let refs: Vec<&std::path::Path> = dirs.iter().map(|p| p.as_path()).collect();
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &refs)
}

/// 真实样本（real-samples/）在位预检——SC-02 真实锚点测试专用（issue #9 口径）。
fn real_samples_ready(test_id: &str) -> bool {
    let d = fixture("../../../real-samples");
    !bid_parse::env_preflight::skip_if_fixture_missing(test_id, &[&d])
}

/// A-招标文件.docx（corpus-A 主文件）黄金锚点（B4 前差分锚定固化值——
/// PR #31 r3：EQ+ED 91.57% 全量验收 + 该件 100% EQ）。
/// 破坏推演：引擎 bump 或映射回归改动块流（空段对齐/表行发射/章节上下文）
/// → 任一数值断言红；锚点更新必须伴随 mdcore rev 升级 PR + 差分重跑。
#[test]
fn mapping_golden_anchor_on_corpus_a_main() {
    if !fixtures_ready("mapping_golden_anchor_on_corpus_a_main") {{
        return;
    }}
    let path = fixture("../../../fixtures/corpus-A/files/A-招标文件.docx");
    let engine = parse_docx_via_engine(&path).expect("引擎路径解析成功（A 件无嵌套表）");

    // 块流总量与块型分布（B4 固化：182 块 = 154 段落 + 28 表行）
    assert_eq!(engine.blocks.len(), 182, "块数（B3-3 差分 100% EQ 件——空段对齐后锚点）");
    let paras = engine.blocks.iter()
        .filter(|b| matches!(b.kind, bid_parse::BlockKind::Para)).count();
    let rows = engine.blocks.iter()
        .filter(|b| matches!(b.kind, bid_parse::BlockKind::TableRow)).count();
    assert_eq!((paras, rows), (154, 28), "块型分布（段落/表行）");

    // 表计数（ParsedFile.tables——下游 chunk 索引消费面）
    assert_eq!(engine.tables, 6, "表计数一致");

    // 逐块抽查（首/中/末——kind + locator + text 口径）
    let b0 = &engine.blocks[0];
    assert!(matches!(b0.kind, bid_parse::BlockKind::Para));
    assert_eq!(b0.locator, "第一章 招标公告");
    assert_eq!(b0.text, "第一章 招标公告");
    let b91 = &engine.blocks[91];
    assert_eq!(b91.locator, "第八章 合同主要条款（摘要）", "中位块章节上下文");
    let b_last = engine.blocks.last().unwrap();
    assert_eq!(
        b_last.locator,
        "第九章 技术规范与验收要求 9.22 运维与网络安全技术要求",
        "末块 locator（章 > 节 口径——extract.rs split_locator 消费）"
    );
    assert!(b_last.text.starts_with("9.22.3"), "末块条款文本锚：{}", b_last.text);

    // 账本登记面（SC-02）：正文 present=1 / extracted=独立段落块数
    let body = engine.components.iter().find(|c| c.component == "正文").unwrap();
    assert_eq!((body.present, body.extracted), (1, 154), "正文账本（extracted=段落块数）");
    assert_eq!(engine.components.len(), 9, "账本 9 面（B4 锚点：A 件无非正文部件）");

    // parse_summary 溯源：引擎路径必须携带 rev
    assert!(
        engine.parse_summary.contains(MDCORE_REV),
        "parse_summary 溯源 rev：{}",
        engine.parse_summary
    );
}

/// 章节上下文 locator：引擎映射必须携带章节（extract.rs 类别映射依赖——
/// 「章」「章 > 节」「{章} 表N 行M」三口径黄金锚）。
#[test]
fn mapping_carries_chapter_context() {
    if !fixtures_ready("mapping_carries_chapter_context") {{
        return;
    }}
    let path = fixture("../../../fixtures/corpus-A/files/A-招标文件.docx");
    let engine = parse_docx_via_engine(&path).expect("引擎路径解析成功");

    // A 件全部 182 块携带 locator（首章起全部处于章节上下文中——B4 锚点）
    let with_locator = engine.blocks.iter().filter(|b| !b.locator.is_empty()).count();
    assert_eq!(with_locator, 182, "全块携带章节 locator");

    // 三口径锚点：章 / 章 > 节 / 章内表行
    let has = |needle: &str| engine.blocks.iter().any(|b| b.locator == needle);
    assert!(has("第三章 投标人须知"), "「章」口径");
    assert!(has("第三章 投标人须知 > 七、开标、评标与否决条款"), "「章 > 节」口径（中文序号节）");
    assert!(has("第二章 投标人须知前附表 表1 行1"), "「{{章}} 表{{N}} 行{{M}}」口径（表行）");

    // 章节推进序锚（政采范本九章结构——类别映射的语义基座）：各章首次
    // 出现位置严格递增（第九章的 9.x 节挂「章 节号」空格口径——`starts_with`
    // 覆盖两种节挂接形态）。
    let chapters = ["第一章 招标公告", "第二章 投标人须知前附表", "第三章 投标人须知",
        "第四章 评标办法（综合评分法）", "第五章 技术需求（5 组参数表）", "第六章 商务要求",
        "第七章 投标文件格式", "第八章 合同主要条款（摘要）", "第九章 技术规范与验收要求"];
    let mut last_pos = None;
    for ch in chapters {
        let pos = engine.blocks.iter().position(|b| b.locator.starts_with(ch))
            .unwrap_or_else(|| panic!("章 {ch} 未在 locator 中出现——章节上下文断裂"));
        if let Some(lp) = last_pos {
            assert!(pos > lp, "章节推进序：{ch}（#{pos}）须晚于前章（#{lp}）");
        }
        last_pos = Some(pos);
    }
    assert_eq!(last_pos, Some(94), "第九章首现位（B4 锚点——extract.rs 类别映射依赖）");
}

/// SC-02 部件账本：引擎路径登记面（正文/页眉/页脚/文本框/批注/脚注尾注/修订/图片）。
#[test]
fn mapping_emits_component_ledger() {
    if !fixtures_ready("mapping_emits_component_ledger") {{
        return;
    }}
    let path = fixture("../../../fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx");
    let engine = parse_docx_via_engine(&path).expect("引擎路径解析成功（D2a：文本框+批注+修订混合件）");

    let names: Vec<&str> = engine.components.iter().map(|c| c.component.as_str()).collect();
    for expect in ["正文", "页眉", "页脚", "文本框", "批注", "脚注尾注", "修订插入", "修订删除", "图片"] {
        assert!(
            names.contains(&expect),
            "账本缺 {expect} 面（实际：{names:?}）——SC-02 对账不变式"
        );
    }
    // D2a 已知部件（B3-2 验收口径）：文本框 1 + 批注 1 + 修订删除 1
    let tb = engine.components.iter().find(|c| c.component == "文本框").unwrap();
    assert_eq!(tb.present, 1, "D2a 文本框 1（#18 口径）");
    let cm = engine.components.iter().find(|c| c.component == "批注").unwrap();
    assert_eq!(cm.present, 1, "D2a 批注 1（#19 口径）");
    let rd = engine.components.iter().find(|c| c.component == "修订删除").unwrap();
    assert!(rd.present >= 1, "D2a 修订删除登记（delText 计数）");
    // 零提取面必带 note（SC-02 不变式）
    for c in &engine.components {
        if c.extracted == 0 && c.present > 0 {
            assert!(
                !c.note.is_empty(),
                "部件 {} present>0 而 extracted=0 必须带 note（登记不伪装）",
                c.component
            );
        }
    }
}

/// 敌意面：截断容器必须被引擎三门拒绝（A3 口径——映射层不自研守卫）。
#[test]
fn mapping_rejects_truncated_container() {
    if !fixtures_ready("mapping_rejects_truncated_container") {{
        return;
    }}
    let path = fixture("../../../fixtures/corpus-R/D2-nonbody/D2a-非正文条款.docx");
    let bytes = std::fs::read(&path).unwrap();
    let truncated = &bytes[..bytes.len() / 2];
    let reader = bid_engine::MdcoreReader::new();
    let result = reader.read_docx(truncated);
    assert!(result.is_err(), "截断容器必须被引擎拒绝（三门语义）");
}

/// 借鉴 mdcore 夹具（TR2/word-complex.docx，PROVENANCE.md 溯源）：
/// 引擎读回统计面（tables=1/comments=1）经映射后账本一致。
#[test]
fn mapping_borrowed_word_complex_stats() {
    if !fixtures_ready("mapping_borrowed_word_complex_stats") {{
        return;
    }}
    let path = fixture("../../../fixtures/engine-borrowed/TR2/word-complex.docx");
    let engine = parse_docx_via_engine(&path).expect("TR2 word-complex 引擎路径解析成功");
    // gen-log.txt 口径：tables=1 / comments=1 / revisions=1 / fields=1
    assert_eq!(engine.tables, 1, "TR2 word-complex tables=1（gen-log 口径）");
    let cm = engine.components.iter().find(|c| c.component == "批注").unwrap();
    assert_eq!(cm.present, 1, "TR2 word-complex comments=1（gen-log 口径）");
    // 修订面（revisions=1：ins 或 del 至少一面登记）
    let has_rev = engine.components.iter().any(|c| {
        (c.component == "修订插入" || c.component == "修订删除") && c.present > 0
    });
    assert!(has_rev, "TR2 word-complex revisions=1（gen-log 口径）——修订面必须登记");
}

/// 表行细节口径：全空行跳过但行号消耗；text=非空列单空格连接；合并表头
/// 栅格压回 w:tc 形（D1a 黄金锚——B4 前差分锚定固化值，PR #31 r3 100% EQ）。
#[test]
fn mapping_table_row_conventions() {
    if !fixtures_ready("mapping_table_row_conventions") {{
        return;
    }}
    let path = fixture("../../../fixtures/corpus-R/D1-tables/D1a-合并表头跨页表.docx");
    let engine = parse_docx_via_engine(&path).expect("D1a 引擎路径解析成功");

    let e_rows: Vec<&bid_parse::Block> = engine
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, bid_parse::BlockKind::TableRow))
        .collect();
    // D1a 锚点：1 表 20 行（跨页表行号连续消耗）+ 首尾段落（23 块总量）
    assert_eq!(e_rows.len(), 20, "表行数（D1a 黄金锚）");
    assert_eq!(engine.tables, 1, "表计数");
    assert_eq!(engine.blocks.len(), 23, "块总量（20 表行 + 3 段落）");

    // 合并表头行（行1）：两级合并表头栅格压回 w:tc 形——5 格
    let r1 = e_rows[0];
    assert_eq!(r1.locator, " 表1 行1", "表行 locator（{{章}} 表N 行M——章上下文为空）");
    assert_eq!(r1.cells, ["序号", "设备信息", "商务信息", "类别", "备注"], "合并表头收敛格（P2-2 栅格→w:tc）");
    assert_eq!(r1.text, "序号 设备信息 商务信息 类别 备注", "text=非空列单空格连接");

    // 数据行（行2 起）：9 格（跨列合并压回后列数稳定）
    let r2 = e_rows[1];
    assert_eq!(r2.locator, " 表1 行2");
    assert_eq!(r2.cells.len(), 9, "数据行列数（w:tc 口径）");
    assert_eq!(r2.text, "名称 型号规格 单位 数量 单价上限（元） 质保期", "次级表头 text");

    // 行号消耗锚：行10 数据内容（跨页/空行不压缩行号——旧实现对齐）
    let r10 = e_rows[9];
    assert_eq!(r10.locator, " 表1 行10", "行号 = 矩阵行位（不因空行压缩）");
    assert_eq!(r10.text, "8 分布式存储 示例型号 ST-5000（示例） 套 1 320,000 3 年", "行10 text 锚");
}

/// SC-02 部件对账（真实锚点·引擎版）：北京水利施工示范文本 2025-LO.docx
/// （B4 自 bid-parse/src/docx.rs `real_sample_component_ledger_beijing_2025`
/// 迁移——引擎口径固化值：blocks=2741 / tables=52 / 正文 extracted=2302 /
/// 页眉·页脚 present=doc.headers/footers.len()=2（引擎按**挂接槽位**计数，
/// 旧实现按 zip 部件文件数 58——口径差异登记不伪装，语义面同为「零提取
/// 部件显式登记」））。
/// 破坏推演：删账本（或 extracted 误计 >0）→ 正文/页眉断言红；
/// 零提取部件无 note → 不变式断言红。
#[test]
fn real_sample_component_ledger_beijing_2025() {
    if !real_samples_ready("real_sample_component_ledger_beijing_2025") {{
        return;
    }}
    let path = fixture("../../../real-samples/北京水利施工示范文本2025-LO.docx");
    let pf = parse_docx_via_engine(&path).expect("真实示范文本必须可解析（引擎路径）");
    assert_eq!(pf.tables, 52, "真实表格数（引擎口径与旧口径一致）");
    assert_eq!(pf.blocks.len(), 2741, "真实块量（2302 段落 + 439 表行——引擎锚点）");
    let row = |name: &str| pf.components.iter().find(|c| c.component == name);
    let body = row("正文").expect("正文行在位");
    assert_eq!((body.present, body.extracted), (1, 2302), "正文 extracted=独立段落块数");
    let h = row("页眉").expect("页眉行在位");
    assert_eq!((h.present, h.extracted), (2, 0), "页眉=引擎挂接槽位数（零提取）");
    assert!(!h.note.is_empty(), "零提取部件必须带登记注记");
    let f = row("页脚").expect("页脚行在位");
    assert_eq!((f.present, f.extracted), (2, 0), "页脚=引擎挂接槽位数（零提取）");
    // SC-02 不变式：零提取部件必须显式登记（note 非空）
    assert!(pf.components.iter()
        .filter(|c| c.present > 0 && c.extracted == 0)
        .all(|c| !c.note.is_empty()));
    assert!(pf.parse_summary.contains("部件账本"), "summary 须携带账本口径：{}", pf.parse_summary);
}
