//! 状态机全路径矩阵——10 状态 × 21 工具 × 合法/非法迁移的全覆盖。
//!
//! 为什么单开这个文件：`ProjectStatus::next_allowed()` 与 `allowed_tools()`
//! 是**状态机的唯一权威真源**（lib.rs），但此前只被 e2e 顺带覆盖——
//! 散在 7 个端到端文件里的 8 个用例，走的是「一条快乐路径」，
//! 没有任何测试**穷举**过「哪些工具在哪些状态被拒」。
//!
//! 状态机漏一格 = 越级调用穿透 = 数据面损坏（例如 DRAFT 态就 export）。
//! 本文件把两张表钉死：改表必须同时改测试，否则编译/断言红。

use bid_domain::ProjectStatus;

/// 全部 10 个状态（顺序 = 状态机前向顺序）。
const ALL_STATES: [ProjectStatus; 10] = [
    ProjectStatus::Draft,
    ProjectStatus::FilesImported,
    ProjectStatus::RequirementsExtracted,
    ProjectStatus::MatrixReady,
    ProjectStatus::EvidenceBound,
    ProjectStatus::Drafting,
    ProjectStatus::Validating,
    ProjectStatus::Reviewing,
    ProjectStatus::ReadyToExport,
    ProjectStatus::Exported,
];

/// 全部 21 个工具（TOOL_NAMES 的真源在 bid-mcp——这里钉死字面量，
/// 目的是「工具面变更必须显式改测试」，而不是静默跟随）。
/// #184 增 `record_addendum_waiver`（尾插——放行窗口与 record_review_gate
/// 同族但更宽：FILES_IMPORTED+，含 Exported——与 Gate::AddendumWaiver
/// 「导入后任意时刻」同窗口语义，审计面写入与 record_review_gate 同款）。
const ALL_TOOLS: [&str; 21] = [
    "create_bid_project",
    "import_tender_files",
    "extract_requirements",
    "build_outline",
    "get_compliance_matrix",
    "register_evidence",
    "bind_evidence",
    "list_evidence",
    "search_evidence",
    "draft_response_section",
    "save_version",
    "validate_bid_document",
    "check_bid_quality",
    "export_bid_document",
    "get_bid_project_state",
    "record_review_gate",
    "score_price",
    "score_section",
    "score_document",
    "score_tender",
    "record_addendum_waiver",
];

// ---------------------------------------------------------------- 1. 前向迁移表

/// 前向迁移链必须是一条**链**（每状态至多一个后继），且 9 步走完不回头。
#[test]
fn forward_chain_is_linear_and_terminates() {
    // 线性：每状态的后继数 ≤ 1
    for s in ALL_STATES {
        let next = s.next_allowed();
        assert!(
            next.len() <= 1,
            "{s:?} 的后继 {} 个（前向链必须是线性的，实际 {next:?}）",
            next.len()
        );
    }
    // 从 Draft 出发恰好走 9 步到 Exported
    let mut cur = ProjectStatus::Draft;
    let mut steps = 0;
    while let Some(&n) = cur.next_allowed().first() {
        cur = n;
        steps += 1;
        assert!(steps <= 10, "前向链成环（走了 {steps} 步还没到终点）");
    }
    assert_eq!(cur, ProjectStatus::Exported, "前向链终点必须是 Exported");
    assert_eq!(steps, 9, "Draft → Exported 恰好 9 步，实际 {steps}");
}

/// 每个状态都必须在前向链上（没有孤儿状态）。
#[test]
fn no_orphan_states() {
    let mut reachable = vec![ProjectStatus::Draft];
    let mut cur = ProjectStatus::Draft;
    while let Some(&n) = cur.next_allowed().first() {
        reachable.push(n);
        cur = n;
    }
    for s in ALL_STATES {
        assert!(reachable.contains(&s), "{s:?} 从前向链不可达（孤儿状态）");
    }
    assert_eq!(reachable.len(), ALL_STATES.len());
}

// ---------------------------------------------------------------- 2. 工具面矩阵

/// 每个状态声明的工具名都必须是**已知工具**（防拼写错/幽灵工具）。
#[test]
fn allowed_tools_are_all_known_tools() {
    for s in ALL_STATES {
        for t in s.allowed_tools() {
            assert!(
                ALL_TOOLS.contains(t),
                "{s:?} 声明了未知工具 {t:?}（拼写错？还是新增工具没登记？）"
            );
        }
    }
}

/// 每个状态必须至少放行一个工具（否则卡死不可操作）。
#[test]
fn every_state_allows_at_least_one_tool() {
    for s in ALL_STATES {
        let tools = s.allowed_tools();
        assert!(!tools.is_empty(), "{s:?} 不放行任何工具——该状态不可操作（死锁）");
    }
}

/// 只读工具（状态查询 / 评分推演）在任何状态都必须可用。
#[test]
fn read_only_tools_are_always_available() {
    // get_bid_project_state 是【断点恢复入口】——必须全状态可用，
    // 否则 Agent 断线后无从知道当前在哪一步。
    for s in ALL_STATES {
        assert!(
            s.allowed_tools().contains(&"get_bid_project_state"),
            "{s:?} 不放行 get_bid_project_state——断点恢复被断"
        );
    }
    // 注：score_price / list_evidence / score_tender / save_version 是
    // 【无状态门】工具（纯计算器或只读盘点，不写项目状态），因此在
    // allowed_tools 表里全状态都不出现——它们的门在工具内部（路由/参数校验），
    // 不在状态机面。这不是遗漏：allowed_tools 只列【受状态门约束】的工具。
}

/// 写面工具必须在**正确的状态窗口**才放行（越级防护的正面断言）。
#[test]
fn write_tools_gated_by_state() {
    // import_tender_files：DRAFT 起（导入前后都可补件）
    assert!(ProjectStatus::Draft.allowed_tools().contains(&"import_tender_files"));
    assert!(ProjectStatus::FilesImported.allowed_tools().contains(&"import_tender_files"));
    // extract_requirements：DRAFT 不许抽（还没导入），FILES_IMPORTED 才许
    assert!(!ProjectStatus::Draft.allowed_tools().contains(&"extract_requirements"));
    assert!(ProjectStatus::FilesImported.allowed_tools().contains(&"extract_requirements"));
    // build_outline：REQUIREMENTS_EXTRACTED 才许建纲
    assert!(!ProjectStatus::FilesImported.allowed_tools().contains(&"build_outline"));
    assert!(ProjectStatus::RequirementsExtracted.allowed_tools().contains(&"build_outline"));
    // draft_response_section：MATRIX_READY 才许起草
    assert!(!ProjectStatus::RequirementsExtracted.allowed_tools().contains(&"draft_response_section"));
    assert!(ProjectStatus::MatrixReady.allowed_tools().contains(&"draft_response_section"));
    // validate_bid_document：DRAFTING 才许校验
    assert!(!ProjectStatus::MatrixReady.allowed_tools().contains(&"validate_bid_document"));
    assert!(ProjectStatus::Drafting.allowed_tools().contains(&"validate_bid_document"));
}

/// 终态（Exported）**不得**再放行写面工具——只允许只读/幂等面。
#[test]
fn terminal_state_blocks_write_tools() {
    let write_tools = [
        "create_bid_project",
        "import_tender_files",
        "extract_requirements",
        "build_outline",
        "register_evidence",
        "bind_evidence",
        "draft_response_section",
        "save_version",
        "validate_bid_document",
    ];
    let exported = ProjectStatus::Exported.allowed_tools();
    for t in write_tools {
        assert!(
            !exported.contains(&t),
            "Exported 仍放行写面工具 {t:?}——终态不该再改数据"
        );
    }
    // 但重导出必须放行（幂等/换路径）
    assert!(
        exported.contains(&"export_bid_document"),
        "Exported 必须放行 export_bid_document（重导出幂等面）"
    );
}

// ---------------------------------------------------------------- 3. 全矩阵钉死

/// 全矩阵快照：10 状态 × 21 工具的放行位。
///
/// 这是**回归护栏**——表一变测试就红，逼人显式确认「这是有意的变更」。
/// 用 grid 字面量（每个状态一行，工具按 ALL_TOOLS 顺序）便于肉眼 diff。
#[test]
fn full_matrix_snapshot() {
    let grid: Vec<(ProjectStatus, &str)> = vec![
        // Draft（record_addendum_waiver 拒——导入前无件可确认）
        (ProjectStatus::Draft, "110000000000001000000"),
        // FilesImported
        (ProjectStatus::FilesImported, "011010001000001000001"),
        // RequirementsExtracted
        (ProjectStatus::RequirementsExtracted, "011110001000001000001"),
        // MatrixReady
        (ProjectStatus::MatrixReady, "000011101100101101101"),
        // EvidenceBound
        (ProjectStatus::EvidenceBound, "000011101100101101101"),
        // Drafting
        (ProjectStatus::Drafting, "000010001101111101101"),
        // Validating
        (ProjectStatus::Validating, "000010001001111101101"),
        // Reviewing
        (ProjectStatus::Reviewing, "000010001001111101101"),
        // ReadyToExport
        (ProjectStatus::ReadyToExport, "000010001001111101101"),
        // Exported
        (ProjectStatus::Exported, "000010001000111101101"),
    ];
    assert_eq!(grid.len(), ALL_STATES.len());

    for (state, bits) in grid {
        assert_eq!(
            bits.len(),
            ALL_TOOLS.len(),
            "{state:?} 的位串长度 {}=ALL_TOOLS {}（矩阵与工具面不同步）",
            bits.len(),
            ALL_TOOLS.len()
        );
        let allowed = state.allowed_tools();
        for (i, t) in ALL_TOOLS.iter().enumerate() {
            let expected = bits.as_bytes()[i] == b'1';
            let actual = allowed.contains(t);
            assert_eq!(
                actual, expected,
                "{state:?} × {t}: 期望{}，实际{}（状态机表有变更——如属有意请同步本快照）",
                if expected { "放行" } else { "拒绝" },
                if actual { "放行" } else { "拒绝" }
            );
        }
    }
}

// ---------------------------------------------------------------- 4. 展示面

/// 状态串必须是稳定字面量（MCP 契约靠它分支，改名即破坏兼容）。
#[test]
fn state_strings_are_stable() {
    let expect = [
        (ProjectStatus::Draft, "DRAFT"),
        (ProjectStatus::FilesImported, "FILES_IMPORTED"),
        (ProjectStatus::RequirementsExtracted, "REQUIREMENTS_EXTRACTED"),
        (ProjectStatus::MatrixReady, "MATRIX_READY"),
        (ProjectStatus::EvidenceBound, "EVIDENCE_BOUND"),
        (ProjectStatus::Drafting, "DRAFTING"),
        (ProjectStatus::Validating, "VALIDATING"),
        (ProjectStatus::Reviewing, "REVIEWING"),
        (ProjectStatus::ReadyToExport, "READY_TO_EXPORT"),
        (ProjectStatus::Exported, "EXPORTED"),
    ];
    for (s, lit) in expect {
        assert_eq!(s.as_str(), lit, "{s:?} 的展示串变了——MCP 契约兼容性破坏");
    }
}
