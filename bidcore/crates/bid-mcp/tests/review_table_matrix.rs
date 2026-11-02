//! #145 任务 2：审查表驱动的合规矩阵验证——**先行骨架**（待 #144 交付后收官）。
//!
//! 背景（issue #145 / #144）：真实招标文件第 6 章「评审」有三张核心审查表——
//! 表7 资格性审查表（9 行 × 2 列，不满足 = 废标）、表8 符合性审查表
//! （6 行 × 3 列，**两列同名「合格条件」**，不满足 = 废标）、表9 详细评审表
//! （8 行 × 4 列，标准分 30/25/14/14/3/10/4，合计 100）。代理 A 的 #144
//! 交付结构化抽取器（`bid-parse/src/review_table.rs`）后，这三张表的每一条
//! 都应成为 REQ 并驱动合规矩阵的 blocking 判定。
//!
//! 本文件在 #144 交付**前**先行落地的验证面（issue 原文：「先写验证测试的
//! 骨架（用假数据），等 A 交付后填真实数据」）：
//!
//! 1. [`RowExpect`] fixture——三张表 **20 条数据行**（#144/#145 原文逐条编码，
//!    keys 取行内特异片段——验证器按「去空白包含」匹配，不依赖 #144 的
//!    statement 组装方式）；
//! 2. [`verify_review_rows`]——纯验证器（三条语义：每行成 REQ + 表7/8 未响应
//!    blocking / 表9 评分不 blocking + params.score 承载标准分 + 总分守恒 100）；
//! 3. `verifier_selfcheck_with_fake_data`——**假数据自检（现在就跑）**：
//!    按 #145 预期语义构造的假矩阵 → 验证器零违例；四种突变（漏行 /
//!    评分项误 blocking / 分值错 / 类目错 / 总分失衡）→ 逐一抓获；
//! 4. `synthetic_review_docx_parses_today`——**基线（现在就跑）**：按真实表
//!    结构合成 docx（表8 两列同名复刻），走 import → extract → matrix 全链，
//!    断言管道健康 + 留档「当前抽取器对三张表的覆盖现状」（如实数字）；
//! 5. `review_matrix_semantics_full`——**#[ignore] 门控**：#144 交付后去掉
//!    ignore，同一合成件上跑完整三语义验收。
//!
//! 夹具纪律：合成 docx 在测试内生成（docx-rs，workspace 锁内与 bid-tools
//! 同款）——三张表的结构性怪癖（两列同名 / 序号跳 6 / 标准分合计 100）
//! 全部按真实件复刻，#144 的抽取器必须吃下这些怪癖才算交付。

use bid_mcp::{dispatch_with, BidService, ToolCall, ToolReply};
use docx_rs::{Docx, Paragraph, Run, Table, TableCell, TableRow};
use serde_json::{json, Value};

// ---------------------------------------------------------------- 调用助手

fn call(svc: &mut BidService, tool: &str, args: Value) -> ToolReply {
    dispatch_with(svc, ToolCall { tool: tool.to_string(), args })
}

fn ok_result(reply: ToolReply) -> Value {
    match reply {
        ToolReply::Ok(out) => out.result,
        ToolReply::Err { error, .. } => panic!("预期成功，实际错误: {error}"),
    }
}

// ---------------------------------------------------------------- 行期望 fixture

/// 三张表来源（真实呼中招标文件第 6 章，#144 原文）。
#[derive(Clone, Copy, PartialEq)]
enum TableKind {
    /// 表7 资格性审查表：不满足 = 废标 → REQ 类目须为必选集（资格条件/否决项/审查项）
    Qualification,
    /// 表8 符合性审查表：不满足 = 废标 → 同上
    Conformance,
    /// 表9 详细评审表：评分项 + params.score；不 blocking（评分减分而已）
    Detailed,
}

/// 单行期望：`keys` = 行内特异片段（匹配口径 = 去全部空白后的包含——与
/// GT 对齐判分同款规范化）；表9 行带 `score`（标准分）。
struct RowExpect {
    table: TableKind,
    /// 行序（表内 1 起——报告定位面）。
    row: u32,
    /// 评审内容（原文——报告/诊断用）。
    item: &'static str,
    /// 特异匹配片段（2 条以上交叉验证，避免「投标报价」在表8/表9 间互撞）。
    keys: &'static [&'static str],
    /// 表9 标准分（None = 表7/表8 行）。
    score: Option<i64>,
}

/// 表7 资格性审查表——9 行 × 2 列（含表头「评审内容｜合格条件」）→ 8 条数据行。
const T7_ROWS: &[RowExpect] = &[
    row(TableKind::Qualification, 1, "具有独立承担民事责任的能力",
        &["独立承担民事责任", "提交有效的营业执照"], None),
    row(TableKind::Qualification, 2, "法定代表人授权书",
        &["法定代表人授权书", "加盖公章"], None),
    row(TableKind::Qualification, 3, "有依法缴纳税收和社会保障资金的良好记录",
        &["缴纳税收", "社会保障资金"], None),
    row(TableKind::Qualification, 4, "具有良好的商业信誉和健全的财务会计制度",
        &["商业信誉", "财务会计制度"], None),
    row(TableKind::Qualification, 5, "履行合同所必须的设备和专业技术能力",
        &["履行合同所必须的设备", "专业技术能力"], None),
    row(TableKind::Qualification, 6, "可查证不存在违反「单位负责人为同一人…」",
        &["单位负责人为同一人"], None),
    row(TableKind::Qualification, 7, "通过「全国企业信用信息公示系统」可查证",
        &["全国企业信用信息公示系统"], None),
    row(TableKind::Qualification, 8, "投标人承诺不存在相互串通投标的行为",
        &["相互串通投标"], None),
];

/// 表8 符合性审查表——6 行 × 3 列（表头「评审内容｜合格条件｜合格条件」
/// **两列同名**——真实件怪癖，抽取器必须容忍）→ 5 条数据行。
const T8_ROWS: &[RowExpect] = &[
    row(TableKind::Conformance, 1, "投标承诺书",
        &["标准格式的", "投标承诺书"], None),
    row(TableKind::Conformance, 2, "投标报价",
        &["一个有效报价", "不超过采购预算"], None),
    row(TableKind::Conformance, 3, "主要商务条款",
        &["主要商务要求承诺书"], None),
    row(TableKind::Conformance, 4, "技术要求实质性内容",
        &["技术要求实质性", "响应程度"], None),
    row(TableKind::Conformance, 5, "其他要求",
        &["无效投标情形", "围标"], None),
];

/// 表9 详细评审表——8 行 × 4 列（表头「序号｜评审项目｜标准分｜评分细则」）
/// → 7 条数据行（**真实件序号跳 6**：1/2/3/4/5/7/8——怪癖按原样复刻）。
/// 标准分 30+25+14+14+3+10+4 = **100（总分守恒红线）**。
const T9_ROWS: &[RowExpect] = &[
    row(TableKind::Detailed, 1, "投标报价", &["低价优先法"], Some(30)),
    row(TableKind::Detailed, 2, "技术条款", &["技术偏离表"], Some(25)),
    row(TableKind::Detailed, 3, "整体供货方案", &["整体供货方案"], Some(14)),
    row(TableKind::Detailed, 4, "质量保证措施", &["质量保证措施"], Some(14)),
    row(TableKind::Detailed, 5, "培训方案", &["培训计划", "专职培训人员"], Some(3)),
    row(TableKind::Detailed, 7, "售后服务方案", &["售后服务方案"], Some(10)),
    row(TableKind::Detailed, 8, "项目应急预案", &["项目应急预案"], Some(4)),
];

const fn row(
    table: TableKind,
    r: u32,
    item: &'static str,
    keys: &'static [&'static str],
    score: Option<i64>,
) -> RowExpect {
    RowExpect { table, row: r, item, keys, score }
}

fn all_rows() -> Vec<&'static RowExpect> {
    T7_ROWS.iter().chain(T8_ROWS.iter()).chain(T9_ROWS.iter()).collect()
}

fn table_tag(t: TableKind) -> &'static str {
    match t {
        TableKind::Qualification => "表7 资格性审查",
        TableKind::Conformance => "表8 符合性审查",
        TableKind::Detailed => "表9 详细评审",
    }
}

/// 匹配规范化：去全部空白（与 scripts/veto_measure / GT 对齐判分同款口径）。
fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 行 ↔ 矩阵行匹配：任一特异片段（去空白）包含于 REQ statement。
fn row_matches(stmt: &str, expect: &RowExpect) -> bool {
    let n = norm(stmt);
    expect.keys.iter().any(|k| n.contains(&norm(k)))
}

/// 表7/表8 行的合法类目集（#145 原文：「资格条件（或新增审查项）」——
/// 否决项同属必选集：三者在 is_mandatory 下都会 blocking）。
fn mandatory_cats() -> [&'static str; 3] {
    ["资格条件", "否决项", "审查项"]
}

// ---------------------------------------------------------------- 验证器（纯函数）

/// 审查表三语义验证器（#144 交付后的验收真源——输入是 get_compliance_matrix
/// 的 rows 数组原样 JSON）。返回违例清单（空 = 全过）。
///
/// 语义（issue #145 任务 2 原文）：
/// 1. 三张表的**每一条**都成了 REQ（keys 匹配面）；
/// 2. 表7/表8 的条目未响应 → blocking=true；表9 的条目 **不 blocking**
///    （评分不减分而已——减分不阻断导出）；
/// 3. 表9 → category="评分项" + params.score 承载标准分；总分守恒 = 100。
fn verify_review_rows(rows: &[Value]) -> Vec<String> {
    let mut violations: Vec<String> = Vec::new();
    let expects = all_rows();

    for e in &expects {
        let matched: Vec<&Value> = rows
            .iter()
            .filter(|r| {
                let stmt = r["requirement"]["statement"].as_str().unwrap_or("");
                row_matches(stmt, e)
            })
            .collect();
        if matched.is_empty() {
            violations.push(format!(
                "[{}] 第 {} 行「{}」未成为 REQ——keys {:?} 无命中",
                table_tag(e.table), e.row, e.item, e.keys
            ));
            continue;
        }
        match e.table {
            TableKind::Qualification | TableKind::Conformance => {
                // 语义 2a：必选类行——须存在一条 必选类目 && 未响应 blocking
                let ok = matched.iter().any(|r| {
                    let cat = r["requirement"]["category"].as_str().unwrap_or("");
                    let blocking = r["blocking"].as_bool().unwrap_or(false);
                    let status = r["status"].as_str().unwrap_or("");
                    mandatory_cats().contains(&cat)
                        && (blocking || status != "NOT_RESPONDED")
                });
                if !ok {
                    let cats: Vec<&str> = matched
                        .iter()
                        .map(|r| r["requirement"]["category"].as_str().unwrap_or("?"))
                        .collect();
                    violations.push(format!(
                        "[{}] 第 {} 行「{}」无必选类命中（实得类目 {:?}——须为 {:?} 且未响应时 blocking）",
                        table_tag(e.table), e.row, e.item, cats, mandatory_cats()
                    ));
                }
            }
            TableKind::Detailed => {
                // 语义 2b + 3：评分项——须存在一条 评分项 && score 相符 && 不 blocking
                let want = e.score.unwrap_or(0);
                let ok = matched.iter().any(|r| {
                    let cat = r["requirement"]["category"].as_str().unwrap_or("");
                    let got = r["requirement"]["params"]["score"].as_i64();
                    let blocking = r["blocking"].as_bool().unwrap_or(false);
                    cat == "评分项" && got == Some(want) && !blocking
                });
                if !ok {
                    let detail: Vec<String> = matched
                        .iter()
                        .map(|r| {
                            format!(
                                "cat={:?} score={:?} blocking={:?}",
                                r["requirement"]["category"].as_str().unwrap_or("?"),
                                r["requirement"]["params"]["score"].as_i64(),
                                r["blocking"].as_bool().unwrap_or(false)
                            )
                        })
                        .collect();
                    violations.push(format!(
                        "[{}] 第 {} 行「{}」评分项语义不符（期望 评分项/score={want}/不 blocking，实得 [{}]）",
                        table_tag(e.table), e.row, e.item, detail.join("; ")
                    ));
                }
            }
        }
    }

    // 语义 3b：表9 总分守恒（30+25+14+14+3+10+4 = 100）
    let score_sum: i64 = rows
        .iter()
        .filter(|r| r["requirement"]["category"].as_str() == Some("评分项"))
        .filter_map(|r| {
            // 只计 T9 行（keys 命中面）——其他来源的评分项不进守恒账
            let stmt = r["requirement"]["statement"].as_str().unwrap_or("");
            let hit = T9_ROWS.iter().any(|e| row_matches(stmt, e));
            hit.then(|| r["requirement"]["params"]["score"].as_i64().unwrap_or(0))
        })
        .sum();
    if score_sum != 100 {
        violations.push(format!(
            "[表9 详细评审] 总分守恒被破坏：评分项 params.score 合计 {score_sum} ≠ 100（30+25+14+14+3+10+4）"
        ));
    }

    violations
}

// ---------------------------------------------------------------- 假数据构造（自检用）

/// 按 get_compliance_matrix 的真实行形状构造一条假矩阵行。
fn fake_row(req_no: u32, category: &str, stmt: &str, score: Option<i64>, blocking: bool) -> Value {
    let mut params = json!({});
    if let Some(s) = score {
        params["score"] = json!(s);
    }
    json!({
        "cmp_id": format!("CMP-{:04}", req_no),
        "requirement": {
            "id": format!("REQ-{:04}", req_no),
            "clause_no": Value::Null,
            "category": category,
            "category_key": "fake",
            "statement": stmt,
            "mandatory": category == "资格条件" || category == "否决项",
            "source_anchor": Value::Null,
            "params": params,
        },
        "status": "NOT_RESPONDED",
        "evidence": [],
        "gap": Value::Null,
        "response_policy": "default",
        "draft_section_id": Value::Null,
        "blocking": blocking,
    })
}

/// 完整假矩阵：20 行期望 → 20 条假 REQ（表7/8 → 资格条件+blocking；
/// 表9 → 评分项+score+不 blocking）。**这是 #145 预期语义的假数据化身**。
fn fake_matrix_full() -> Vec<Value> {
    let mut rows = Vec::new();
    let mut n = 0;
    for e in all_rows() {
        n += 1;
        let stmt = fake_stmt_for(e);
        match e.table {
            TableKind::Qualification | TableKind::Conformance => {
                rows.push(fake_row(n, "资格条件", &stmt, None, true));
            }
            TableKind::Detailed => {
                rows.push(fake_row(n, "评分项", &stmt, e.score, false));
            }
        }
    }
    rows
}

/// 假 statement：把 keys 拼进一句（保证 row_matches 命中）。
fn fake_stmt_for(e: &RowExpect) -> String {
    format!("{}：{}", e.item, e.keys.join("；"))
}

// ---------------------------------------------------------------- 合成 docx（真实表结构）

/// 按真实件结构合成审查表 docx：第 6 章「评审」+ 三张表 + 若干编号条款
/// （编号条款保证**当前**抽取器就有产出——基线测试的管道健康面）。
fn build_review_docx(path: &std::path::Path) {
    let mut docx = Docx::new();
    // 章标题 + 导语
    docx = docx.add_paragraph(p_text("第六章 评审"));
    docx = docx.add_paragraph(p_text(
        "本章规定资格性审查、符合性审查与详细评审的评审因素与标准。",
    ));
    // 编号条款（当前抽取器的可靠产出面——基线断言 REQ > 0 用）
    docx = docx.add_paragraph(p_text("6.1 投标人须具备电子与智能化工程专业承包二级及以上资质，并提供证书副本。"));
    docx = docx.add_paragraph(p_text("6.2 投标文件须对招标文件的技术要求逐项作出实质性响应。"));
    docx = docx.add_paragraph(p_text("6.3 评标委员会对通过资格性与符合性审查的投标人进行详细评审。"));

    // ---- 表7 资格性审查表（表头 2 列 + 8 数据行）
    docx = docx.add_paragraph(p_text("表7 资格性审查表"));
    docx = docx.add_table(table_2col(
        &["评审内容", "合格条件"],
        &[
            &["具有独立承担民事责任的能力", "在中华人民共和国境内注册的法人，提交有效的营业执照"],
            &["法定代表人授权书", "提供「法定代表人授权书」并按要求签字、加盖公章"],
            &["有依法缴纳税收和社会保障资金的良好记录", "提供《投标单位资格承诺函》"],
            &["具有良好的商业信誉和健全的财务会计制度", "提供承诺函并加盖公章"],
            &["履行合同所必须的设备和专业技术能力", "提供说明材料"],
            &["通过合法渠道，可查证不存在违反「单位负责人为同一人或者存在直接控股、管理关系」的情形", "提供承诺函并加盖公章"],
            &["通过「全国企业信用信息公示系统」可查证无重大违法记录", "提供承诺函"],
            &["投标人承诺不存在相互串通投标的行为", "提供承诺函"],
        ],
    ));

    // ---- 表8 符合性审查表（表头 3 列，后两列**同名**「合格条件」——真实件怪癖）
    docx = docx.add_paragraph(p_text("表8 符合性审查表"));
    docx = docx.add_table(table_t8_three_col());

    // ---- 表9 详细评审表（表头 4 列 + 7 数据行，序号跳 6，标准分合计 100）
    docx = docx.add_paragraph(p_text("表9 详细评审表"));
    let t9_header = vec![
        cell("序号"), cell("评审项目"), cell("标准分"), cell("评分细则"),
    ];
    let t9_body: Vec<TableRow> = [
        ("1", "投标报价", "30", "采用低价优先法计算价格分"),
        ("2", "技术条款", "25", "不满足★号条款的为无效投标；技术偏离表逐项对照"),
        ("3", "整体供货方案", "14", "应至少包括供货组织、进度安排与保障措施"),
        ("4", "质量保证措施", "14", "应至少包括质量管理体系与检验检测安排"),
        ("5", "培训方案", "3", "①培训计划 ②专职培训人员 ③培训课时安排"),
        ("7", "售后服务方案", "10", "应至少包括响应时限、驻场与备件安排"),
        ("8", "项目应急预案", "4", "应至少包括：产品供应异常与突发情况处置"),
    ]
    .iter()
    .map(|(a, b, c, d)| TableRow::new(vec![cell(a), cell(b), cell(c), cell(d)]))
    .collect();
    let mut t9_rows = vec![TableRow::new(t9_header)];
    t9_rows.extend(t9_body);
    docx = docx.add_table(Table::new(t9_rows));

    let mut f = std::fs::File::create(path).expect("建合成 docx");
    docx.pack(&mut f).expect("写合成 docx");
}

fn p_text(s: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(s))
}

fn cell(s: &str) -> TableCell {
    TableCell::new().add_paragraph(p_text(s))
}

fn table_2col(header: &[&str; 2], body: &[&[&str; 2]]) -> Table {
    let mut rows = vec![TableRow::new(vec![cell(header[0]), cell(header[1])])];
    for r in body {
        rows.push(TableRow::new(vec![cell(r[0]), cell(r[1])]));
    }
    Table::new(rows)
}

/// 表8 怪癖复刻：3 列表——后两列**同名同值**「合格条件」（真实件即如此，
/// 抽取器必须容忍同名列头不数错列）。
fn table_t8_three_col() -> Table {
    // 直接重建（Table::new 一次成型；不消费原表）
    let header = vec![cell("评审内容"), cell("合格条件"), cell("合格条件")];
    let body: Vec<TableRow> = [
        ("投标承诺书", "提供标准格式的「投标承诺书」并签字盖章"),
        ("投标报价", "只能有一个有效报价且不超过采购预算"),
        ("主要商务条款", "提供「主要商务要求承诺书」并加盖公章"),
        ("技术要求实质性内容", "依据采购文件规定的要求作出响应"),
        ("其他要求", "招标文件要求的其他无效投标情形；围标串标情形的按无效投标处理"),
    ]
    .iter()
    .map(|(a, b)| TableRow::new(vec![cell(a), cell(b), cell(b)]))
    .collect();
    let mut rows = vec![TableRow::new(header)];
    rows.extend(body);
    Table::new(rows)
}

// ---------------------------------------------------------------- 测试

/// 验证器自检（假数据）——#144 交付前验证【验证器本身】正确：
/// 全量假矩阵零违例；四类突变逐一抓获。
#[test]
fn verifier_selfcheck_with_fake_data() {
    // ---- 基线：按 #145 预期语义构造的完整假矩阵 → 零违例
    let rows = fake_matrix_full();
    assert_eq!(rows.len(), 20, "假矩阵应恰 20 行（8+5+7）");
    let v = verify_review_rows(&rows);
    assert!(v.is_empty(), "假数据基线应零违例：{v:#?}");

    // ---- 突变 1：漏一行（删表7 第 1 行——营业执照）
    let mut m1 = fake_matrix_full();
    m1.remove(0);
    let v = verify_review_rows(&m1);
    assert!(
        v.iter().any(|x| x.contains("独立承担民事责任") && x.contains("未成为 REQ")),
        "突变 1（漏行）应被抓：{v:#?}"
    );

    // ---- 突变 2：表9 评分项误 blocking
    let mut m2 = fake_matrix_full();
    let idx = m2.iter().position(|r| {
        row_matches(r["requirement"]["statement"].as_str().unwrap_or(""), &T9_ROWS[0])
            && r["requirement"]["category"].as_str() == Some("评分项")
    }).unwrap();
    m2[idx]["blocking"] = json!(true);
    let v = verify_review_rows(&m2);
    assert!(
        v.iter().any(|x| x.contains("投标报价") && x.contains("评分项语义不符")),
        "突变 2（评分项误 blocking）应被抓：{v:#?}"
    );

    // ---- 突变 3：表9 分值错（30 → 35 → 总分 105 破守恒 + 分值不符双违例）
    let mut m3 = fake_matrix_full();
    let idx = m3.iter().position(|r| {
        row_matches(r["requirement"]["statement"].as_str().unwrap_or(""), &T9_ROWS[0])
            && r["requirement"]["category"].as_str() == Some("评分项")
    }).unwrap();
    m3[idx]["requirement"]["params"]["score"] = json!(35);
    let v = verify_review_rows(&m3);
    assert!(v.iter().any(|x| x.contains("score=30") && x.contains("评分项语义不符")), "突变 3a（分值错）应被抓：{v:#?}");
    assert!(v.iter().any(|x| x.contains("总分守恒被破坏")), "突变 3b（守恒破）应被抓：{v:#?}");

    // ---- 突变 4：表7 行类目错（资格条件 → 商务要求：未响应却不 blocking）
    let mut m4 = fake_matrix_full();
    m4[0]["requirement"]["category"] = json!("商务要求");
    m4[0]["requirement"]["mandatory"] = json!(false);
    let v = verify_review_rows(&m4);
    assert!(
        v.iter().any(|x| x.contains("无必选类命中")),
        "突变 4（类目错）应被抓：{v:#?}"
    );

    // ---- 突变 5：表8 行整表缺失（5 行全删——5 条违例齐报）
    let mut m5 = fake_matrix_full();
    m5.retain(|r| !T8_ROWS.iter().any(|e| row_matches(r["requirement"]["statement"].as_str().unwrap_or(""), e)));
    let v = verify_review_rows(&m5);
    assert_eq!(
        v.iter().filter(|x| x.contains("表8")).count(),
        5,
        "突变 5（表8 整表缺失）应报 5 条：{v:#?}"
    );
}

/// 基线（现在就跑）：合成审查表 docx 走 import → extract → matrix 全链——
/// 管道健康 + **如实留档当前抽取器对三张表的覆盖现状**（#144 交付前的缺口
/// 数字进测试输出，供 #144 验收对照）。
#[test]
fn synthetic_review_docx_parses_today() {
    let dir = std::env::temp_dir().join(format!(
        "review-table-matrix-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let docx_path = dir.join("A-评审章-合成件.docx");
    build_review_docx(&docx_path);
    assert!(std::fs::metadata(&docx_path).unwrap().len() > 1000, "合成 docx 应非空");

    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "#145 审查表骨架", "bid_deadline": "2026-12-31",
        "mode": "明标", "external_key": "review-table-skeleton",
    })))["project_id"]
        .as_str()
        .unwrap()
        .to_string();

    // ---- import：合成件必须健康解析（表结构不炸解析器）
    let r = ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": pid,
        "files": [{"upload_ref": docx_path.to_str().unwrap(),
                   "filename": "A-评审章-合成件.docx", "kind": "招标文件"}],
    })));
    let totals = r["totals"].clone();
    assert_eq!(totals["parse_failed"], 0, "合成件解析不得失败：{totals}");
    assert_eq!(totals["files"], 1);

    // ---- extract：当前抽取器在编号条款上应有产出
    let r = ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));
    let n_req = r["counts"]["total"].as_u64().unwrap_or(0);
    assert!(n_req > 0, "编号条款（6.1-6.3）应至少产出若干 REQ：{r}");

    // ---- matrix：只读派生可用 + 如实留档三张表覆盖现状
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 1000,
    })));
    let rows = m["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "矩阵行不应为空");
    let matched = all_rows()
        .iter()
        .filter(|e| {
            rows.iter().any(|r| {
                row_matches(r["requirement"]["statement"].as_str().unwrap_or(""), e)
            })
        })
        .count();
    let violations = verify_review_rows(rows);
    eprintln!(
        "#145 骨架基线（#144 前）：REQ={n_req}，20 条审查表期望命中 {matched}/20，\
         验证器违例 {} 条（如实留档——#144 交付后此数字应归零）",
        violations.len()
    );
    // 基线**不断言**三表全覆盖（#144 未交付——当前缺口是已知现状，不是回归）
    std::fs::remove_dir_all(&dir).ok();
}

/// 【#144 已交付——门控启用】三语义完整验收：
/// 三张表每条成 REQ + 表7/8 未响应 blocking + 表9 评分项不 blocking +
/// params.score 承载标准分 + 总分守恒 100。
#[test]
fn review_matrix_semantics_full() {
    let dir = std::env::temp_dir().join(format!(
        "review-table-full-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let docx_path = dir.join("A-评审章-合成件.docx");
    build_review_docx(&docx_path);

    let mut svc = BidService::open(&dir).unwrap();
    let pid = ok_result(call(&mut svc, "create_bid_project", json!({
        "name": "#145 审查表验收", "bid_deadline": "2026-12-31",
        "mode": "明标", "external_key": "review-table-accept",
    })))["project_id"]
        .as_str()
        .unwrap()
        .to_string();

    ok_result(call(&mut svc, "import_tender_files", json!({
        "project_id": pid,
        "files": [{"upload_ref": docx_path.to_str().unwrap(),
                   "filename": "A-评审章-合成件.docx", "kind": "招标文件"}],
    })));
    ok_result(call(&mut svc, "extract_requirements", json!({ "project_id": pid })));
    let m = ok_result(call(&mut svc, "get_compliance_matrix", json!({
        "project_id": pid, "limit": 1000,
    })));
    let rows = m["rows"].as_array().unwrap().clone();

    let violations = verify_review_rows(&rows);
    assert!(
        violations.is_empty(),
        "#144 三语义验收未过（{} 条违例）：\n{}",
        violations.len(),
        violations.join("\n")
    );
    std::fs::remove_dir_all(&dir).ok();
}
