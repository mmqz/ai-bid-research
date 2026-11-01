//! impact.rs —— 影响面引擎（bid-domain-model §2 DIRTY/content_version 语义的 Rust 实装）。
//!
//! V2 SEMUL（scripts/v2_impact.py）的设计语义在产品层的落点：
//!   R-mod  条款修改 → 该 REQ 矩阵行 DIRTY + 绑定重检 + related 章节全 DIRTY
//!   R-del  条款删除 → 绑定解除 + 矩阵行移除 + related 章节全 DIRTY
//!   R-add  条款新增 → 矩阵行新增（无章节归属则零章节影响）
//!   R-date 评估基准日变更 → 全项目有效期重检（valid_until < 新基准日 → EXPIRED；
//!          资格条件→not_eligible 翻转；评分项→禁绑 细项 0 分）
//!   R-conf 双版本冲突 → 双向登记 + 关联章节 ACCEPTED 门禁（终裁前）
//!   R-idem 同内容重复导入 → no-op（content_version/DIRTY 均不变）
//!
//! 级联纪律（R2）：重检范围**恒为全项目**（防增量重检漏网），但翻转必须精确——
//! 重检不等于全推翻（已 EXPIRED 维持、未到期零误翻）。
//!
//! 验证状态：IMPACT_ENGINE_SEMANTICS = IMPLEMENTED（本模块）
//! ——Python SEMUL（v2_impact.py 七断言组全绿）与本实装为双实现交叉验证；
//! 事件面已泛化（AddendumEvent）：真实补遗链数据（D3，公告级）经 c1_real_addendum_
//! chain_cuc028 消费；文件级真实补遗解析器属 M2+（real-samples 登记边界）。

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingStatus {
    Valid,
    Expired,
}

#[derive(Debug, Clone)]
pub struct ReqV2 {
    pub clause_no: String,
    pub category: String,
    pub params: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub ev_files: Vec<String>,
    pub valid_until: Option<String>, // ISO 日期字典序 == 时间序（与 Python 语义一致）
    pub status: BindingStatus,
}

#[derive(Debug, Clone)]
pub struct MatrixRow {
    pub policy: String,
    pub score_subitem: Option<i64>,
    pub control_price_caliber: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub status: String, // DRAFTED | ACCEPTED
    pub dirty: bool,
    pub related: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub versions: Vec<(String, i64)>, // (source, value)
    pub gt_ids: Vec<String>,          // 关联 GT（R3-P2-2：门禁从冲突记录自身取，不耦合最近 spec）
    pub resolved: bool,
    pub final_value: Option<i64>,
    pub rule: String,
}

/// 一次导入的影响面报告（Python ImpactEngine.import_addendum 返回结构的对应物）。
#[derive(Debug, Default, Clone)]
pub struct ImportImpact {
    pub idempotent_noop: bool,
    pub updated_reqs: Vec<String>,
    pub recheck_bindings: Vec<String>,
    pub dirty_matrix: Vec<String>,
    pub dirty_sections: Vec<String>,
    pub conflicts: Vec<String>,
    pub flips: Vec<String>, // 证据状态翻转（valid→expired）
}

/// ACCEPTED 门禁拒绝原因（R4 语义）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptBlock {
    Dirty,
    UnresolvedConflict(Vec<String>),
}

pub struct ImpactEngine {
    pub content_version: u64,
    reqs: HashMap<String, ReqV2>,
    bindings: HashMap<String, Binding>,
    matrix: HashMap<String, MatrixRow>,
    sections: HashMap<String, Section>,
    conflicts: HashMap<String, Conflict>,
    eval_date: String,
    imported: HashMap<String, String>,
    pub recheck_log: Vec<String>,
}

/// 补遗事件（泛化事件面）：corpus-A 三变更联合与真实补遗链（D3 公告级）
/// 共用同一引擎语义——事件由调用方供给（corpus-A 契约测试 / 真实公告字段），
/// M2+ 的补遗解析器将产出同结构事件。
#[derive(Debug, Clone)]
pub enum AddendumEvent {
    /// R-mod：条款内容修改（参数合并，latest_version_wins）→ 行 DIRTY + 重检
    ClauseModified {
        gt_id: String,
        params: HashMap<String, serde_json::Value>,
    },
    /// R-date：评估基准日变更 → 全项目有效期重检（翻转仅限失配者）
    EvalDateChanged { new_date: String },
    /// R-conf：双版本冲突登记（控制价等）——真实补遗无价格变更时**不产生**本事件
    VersionConflict {
        conflict_id: String,
        versions: Vec<(String, i64)>,
        gt_ids: Vec<String>,
        rule: String,
    },
}

/// 补遗事件描述（corpus-A C-补遗文件01 的三变更联合——数据由调用方供给，
/// 与 Python SEMUL 的硬编码事件对齐；M2+ 的补遗解析器将产出该结构）。
#[derive(Debug, Clone)]
pub struct AddendumSpec {
    pub new_model: String,            // GT-18 v2 型号
    pub new_eval_date: String,        // 开标日变更
    pub control_price: i64,           // 控制价新口径
    pub conflict_gt_ids: Vec<String>, // 冲突关联 GT（GT-21/GT-08）
}

impl ImpactEngine {
    /// corpus-A TC-09 初始态（fixture_spec.py REQUIREMENTS/EVIDENCE_LINKS/SECTIONS_SPEC
    /// 的 TC-09 相关子集——契约测试 four_contracts.rs 消费；值漂移由 xref_check 防）。
    pub fn corpus_a() -> Self {
        let mut reqs = HashMap::new();
        let mut matrix = HashMap::new();
        let mut put = |gt: &str, clause: &str, cat: &str, policy: &str| {
            reqs.insert(
                gt.to_string(),
                ReqV2 {
                    clause_no: clause.into(),
                    category: cat.into(),
                    params: HashMap::new(),
                },
            );
            matrix.insert(
                gt.to_string(),
                MatrixRow {
                    policy: policy.into(),
                    score_subitem: None,
                    control_price_caliber: None,
                },
            );
        };
        put("GT-01", "3.2.1", "资格条件", "eligible");
        put("GT-02", "3.2.2", "资格条件", "eligible");
        put("GT-05", "3.3.1", "资格条件", "eligible");
        put("GT-06", "3.3.2", "资格条件", "eligible");
        put("GT-13", "4.2", "评分项", "score_item");
        put("GT-18", "5.3", "技术参数", "model_response");
        put("GT-21", "6.1", "商务要求", "amount_response");
        put("GT-08", "3.7.2.2", "否决项", "price_reject");

        let mut bindings = HashMap::new();
        bindings.insert(
            "GT-01".into(),
            Binding {
                ev_files: vec!["EV-01-营业执照.pdf".into()],
                valid_until: None,
                status: BindingStatus::Valid,
            },
        );
        bindings.insert(
            "GT-02".into(),
            Binding {
                ev_files: vec!["EV-02-电子与智能化工程专业承包二级资质证书.pdf".into()],
                valid_until: Some("2026-11-25".into()),
                status: BindingStatus::Valid,
            },
        );
        bindings.insert(
            "GT-05".into(),
            Binding {
                ev_files: vec!["EV-06-财务审计报告-2025年度.pdf".into()],
                valid_until: None,
                status: BindingStatus::Valid,
            },
        );
        bindings.insert(
            "GT-06".into(),
            Binding {
                ev_files: vec!["EV-07-无失信记录承诺函.pdf".into()],
                valid_until: None,
                status: BindingStatus::Valid,
            },
        );
        bindings.insert(
            "GT-13".into(),
            Binding {
                ev_files: vec!["EV-13-安全生产许可证.pdf".into()],
                valid_until: Some("2025-08-01".into()),
                status: BindingStatus::Valid,
            },
        );

        let mut sections = HashMap::new();
        let mut put_sec = |id: &str, related: &[&str]| {
            sections.insert(
                id.to_string(),
                Section {
                    status: "DRAFTED".into(),
                    dirty: false,
                    related: related.iter().map(|s| s.to_string()).collect(),
                },
            );
        };
        put_sec("资格响应", &["GT-01", "GT-02", "GT-05", "GT-06"]);
        put_sec("技术方案-网络与安全", &["GT-18"]);
        put_sec("商务部分-报价", &["GT-08", "GT-21"]);
        put_sec("售后服务方案", &["GT-13"]);
        put_sec("项目团队", &["GT-15"]); // fixture_spec SECTIONS_SPEC 全六章节（related 为 TC-09 子集）
        put_sec("格式与签署", &["GT-07", "GT-24", "GT-25"]);

        Self {
            content_version: 1,
            reqs,
            bindings,
            matrix,
            sections,
            conflicts: HashMap::new(),
            eval_date: "2026-11-15".into(),
            imported: HashMap::new(),
            recheck_log: vec![],
        }
    }

    /// 真实采购链初始态（D3 公告级）：中国传媒大学教学资源中心建设，
    /// 项目编号 中传招标【2026】第028号/CFTC-BJ01-2606001。
    /// **全部取值可溯源**（公开公告，URL 见 real-samples/中传028真实采购链.json）：
    /// 招标公告 2026-07-13（t20260713_26925801.htm）：预算 108 万、递交/开标
    /// 2026-08-03 14:00、实施节点 07-31/08-30/09-23；更正公告 2026-07-28
    /// （t20260728_27025731.htm）：开标 08-13、节点一 8-30、节点二 9-10、
    /// 「其他内容均无修改」、无预算变更；中标公告 2026-08-14：105 万、89.36 分。
    /// 章节为**响应文件面**模型（技术方案-实施计划/商务报价/资格响应）。
    pub fn cuc_2026_028() -> Self {
        let mut reqs = HashMap::new();
        let mut matrix = HashMap::new();
        let mut bindings = HashMap::new();
        let mut put = |gt: &str,
                       clause: &str,
                       cat: &str,
                       policy: &str,
                       param: Option<(&str, serde_json::Value)>| {
            let mut params = HashMap::new();
            if let Some((k, v)) = param {
                params.insert(k.to_string(), v);
            }
            reqs.insert(
                gt.to_string(),
                ReqV2 {
                    clause_no: clause.into(),
                    category: cat.into(),
                    params,
                },
            );
            matrix.insert(
                gt.to_string(),
                MatrixRow {
                    policy: policy.into(),
                    score_subitem: None,
                    control_price_caliber: None,
                },
            );
        };
        put(
            "GT-B01",
            "一、项目基本情况",
            "商务要求",
            "amount_response",
            Some(("budget", serde_json::json!(1_080_000))),
        );
        put(
            "GT-Q01",
            "二、申请人资格要求-2",
            "资格条件",
            "eligible",
            None,
        );
        put(
            "GT-Q02",
            "一、项目基本情况-联合体",
            "否决项",
            "consortium_reject",
            None,
        );
        put(
            "GT-S01",
            "四、实施要求-节点1",
            "技术需求",
            "milestone",
            Some(("milestone_date", serde_json::json!("2026-07-31"))),
        );
        put(
            "GT-S02",
            "四、实施要求-节点2",
            "技术需求",
            "milestone",
            Some(("milestone_date", serde_json::json!("2026-08-30"))),
        );
        put(
            "GT-S03",
            "四、实施要求-节点3",
            "技术需求",
            "milestone",
            Some(("milestone_date", serde_json::json!("2026-09-23"))),
        );
        // 公告可证的绑定：中小企业声明函（资格面）；本项目公告无带有效期的
        // 资质证书条款——R-date 重检真实运行但零翻转（不虚构到期证据）
        bindings.insert(
            "GT-Q01".into(),
            Binding {
                ev_files: vec!["中小企业声明函.pdf".into()],
                valid_until: None,
                status: BindingStatus::Valid,
            },
        );

        let mut sections = HashMap::new();
        let mut put_sec = |id: &str, related: &[&str]| {
            sections.insert(
                id.to_string(),
                Section {
                    status: "DRAFTED".into(),
                    dirty: false,
                    related: related.iter().map(|s| s.to_string()).collect(),
                },
            );
        };
        put_sec("技术方案-实施计划", &["GT-S01", "GT-S02", "GT-S03"]);
        put_sec("商务报价", &["GT-B01"]);
        put_sec("资格响应", &["GT-Q01", "GT-Q02"]);

        Self {
            content_version: 1,
            reqs,
            bindings,
            matrix,
            sections,
            conflicts: HashMap::new(),
            eval_date: "2026-08-03".into(), // 递交截止=开标=基准日（招标公告原文）
            imported: HashMap::new(),
            recheck_log: vec![],
        }
    }

    /// 按当前基准日重检全部证据绑定（级联核心——重检范围恒为全项目）。
    fn apply_validity(&mut self) {
        self.recheck_log.push(self.eval_date.clone());
        let eval = self.eval_date.clone();
        for (gt, b) in &mut self.bindings {
            if let Some(vu) = &b.valid_until {
                if vu.as_str() < eval.as_str() && b.status != BindingStatus::Expired {
                    b.status = BindingStatus::Expired;
                    let cat = self
                        .reqs
                        .get(gt)
                        .map(|r| r.category.clone())
                        .unwrap_or_default();
                    if let Some(row) = self.matrix.get_mut(gt) {
                        if cat == "资格条件" {
                            row.policy = "not_eligible".into();
                        } else if cat == "评分项" {
                            row.policy = "EXPIRED_FORBIDDEN_BIND".into();
                            row.score_subitem = Some(0);
                        }
                    }
                }
            }
        }
    }

    /// 初始判定期（A 口径——构造完成后必须调用一次以建立基线；corpus_a() 已内联）。
    pub fn prime_validity(&mut self) {
        self.apply_validity();
    }

    pub fn sections(&self) -> &HashMap<String, Section> {
        &self.sections
    }

    pub fn bindings(&self) -> &HashMap<String, Binding> {
        &self.bindings
    }

    pub fn matrix(&self) -> &HashMap<String, MatrixRow> {
        &self.matrix
    }

    pub fn reqs(&self) -> &HashMap<String, ReqV2> {
        &self.reqs
    }

    pub fn conflicts(&self) -> &HashMap<String, Conflict> {
        &self.conflicts
    }

    pub fn eval_date(&self) -> &str {
        &self.eval_date
    }

    pub fn dirty_sections(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .sections
            .iter()
            .filter(|(_, s)| s.dirty)
            .map(|(k, _)| k.clone())
            .collect();
        v.sort();
        v
    }

    /// 导入补遗（corpus-A 三变更联合的便捷包装——行为与历史版本逐位对齐，
    /// 由 c1_* 系列契约测试锁定）。
    pub fn import_addendum(
        &mut self,
        fname: &str,
        sha256: &str,
        spec: &AddendumSpec,
    ) -> ImportImpact {
        let mut params: HashMap<String, serde_json::Value> = HashMap::new();
        params.insert("model_v2".into(), serde_json::json!(spec.new_model));
        params.insert("current_model".into(), serde_json::json!(spec.new_model));
        let events = vec![
            AddendumEvent::ClauseModified {
                gt_id: "GT-18".into(),
                params,
            },
            AddendumEvent::VersionConflict {
                conflict_id: "CONFLICT-01".into(),
                versions: vec![
                    ("A-招标文件.docx".into(), 4_800_000),
                    (fname.into(), spec.control_price),
                ],
                gt_ids: spec.conflict_gt_ids.clone(),
                rule: "补遗优先（后到有效版本覆盖）".into(),
            },
            AddendumEvent::EvalDateChanged {
                new_date: spec.new_eval_date.clone(),
            },
        ];
        self.import_events(fname, sha256, &events)
    }

    /// 泛化事件导入（真实补遗链与 corpus-A 共用）：幂等 → 版本戳 → 事件逐个
    /// 应用（R-mod/R-conf/R-date）→ 章节影响面传播（related ∩ 受影响 REQ）。
    pub fn import_events(
        &mut self,
        fname: &str,
        sha256: &str,
        events: &[AddendumEvent],
    ) -> ImportImpact {
        if self.imported.get(fname).map(|s| s.as_str()) == Some(sha256) {
            return ImportImpact {
                idempotent_noop: true,
                ..Default::default()
            };
        }
        self.imported.insert(fname.into(), sha256.into());
        self.content_version += 1;
        let mut impact = ImportImpact::default();

        for ev in events {
            match ev {
                // (a) R-mod：参数合并（latest_version_wins）
                AddendumEvent::ClauseModified { gt_id, params } => {
                    if let Some(r) = self.reqs.get_mut(gt_id) {
                        for (k, v) in params {
                            r.params.insert(k.clone(), v.clone());
                        }
                    }
                    impact.updated_reqs.push(gt_id.clone());
                    impact.dirty_matrix.push(gt_id.clone());
                    impact.recheck_bindings.push(gt_id.clone());
                }
                // (b) R-conf：双版本登记
                AddendumEvent::VersionConflict {
                    conflict_id,
                    versions,
                    gt_ids,
                    rule,
                } => {
                    self.conflicts.insert(
                        conflict_id.clone(),
                        Conflict {
                            versions: versions.clone(),
                            gt_ids: gt_ids.clone(),
                            resolved: false,
                            final_value: None,
                            rule: rule.clone(),
                        },
                    );
                    impact.conflicts.push(conflict_id.clone());
                    for gt in gt_ids {
                        impact.dirty_matrix.push(gt.clone());
                    }
                }
                // (c) R-date：开标日变更 → 全项目有效期重检
                AddendumEvent::EvalDateChanged { new_date } => {
                    self.eval_date = new_date.clone();
                    let before: HashMap<String, BindingStatus> = self
                        .bindings
                        .iter()
                        .map(|(k, v)| (k.clone(), v.status.clone()))
                        .collect();
                    self.apply_validity();
                    let mut flipped: Vec<String> = vec![];
                    for (gt, b) in &self.bindings {
                        if let Some(prev) = before.get(gt) {
                            if &b.status != prev {
                                impact.recheck_bindings.push(gt.clone());
                                impact.dirty_matrix.push(gt.clone());
                                if b.status == BindingStatus::Expired {
                                    flipped.push(gt.clone());
                                }
                            }
                        }
                    }
                    impact.flips = flipped;
                }
            }
        }

        // 章节影响面：related ∩ 受影响 REQ
        let affected: std::collections::HashSet<String> = impact
            .updated_reqs
            .iter()
            .chain(impact.dirty_matrix.iter())
            .cloned()
            .collect();
        for (sec, v) in &mut self.sections {
            if v.related.iter().any(|g| affected.contains(g)) {
                v.dirty = true;
                impact.dirty_sections.push(sec.clone());
            }
        }
        impact
    }

    /// 矛盾终裁（补遗优先——后到版本覆盖）。
    /// 关联 GT 取**冲突记录自身**的 gt_ids（与 try_accept 同纪律，R3-P2-2）。
    pub fn adjudicate(&mut self, conflict_id: &str) -> Option<(i64, String)> {
        let c = self.conflicts.get_mut(conflict_id)?;
        let final_value = c.versions.last().map(|(_, v)| *v)?;
        c.resolved = true;
        c.final_value = Some(final_value);
        let rule = c.rule.clone();
        let gt_ids = c.gt_ids.clone();
        for gt in &gt_ids {
            if let Some(row) = self.matrix.get_mut(gt) {
                row.control_price_caliber = Some(final_value);
            }
        }
        Some((final_value, rule))
    }

    /// ACCEPTED 门禁：脏章节或未裁决冲突关联章节均不得 ACCEPTED（零副作用拒绝）。
    /// R3-P2-2：关联 GT 从各冲突记录自身取（不耦合最近一次 addendum_spec）。
    pub fn try_accept(&mut self, section_id: &str) -> Result<(), AcceptBlock> {
        {
            let v = self.sections.get(section_id).expect("章节存在");
            if v.dirty {
                return Err(AcceptBlock::Dirty);
            }
            let unresolved: Vec<String> = self
                .conflicts
                .iter()
                .filter(|(_, c)| !c.resolved)
                .filter(|(_, c)| v.related.iter().any(|g| c.gt_ids.contains(g)))
                .map(|(k, _)| k.clone())
                .collect();
            if !unresolved.is_empty() {
                return Err(AcceptBlock::UnresolvedConflict(unresolved));
            }
        }
        self.sections.get_mut(section_id).unwrap().status = "ACCEPTED".into();
        Ok(())
    }

    /// 重起草（清 dirty——终裁门禁测试用）。
    pub fn redraft(&mut self, section_id: &str) {
        let s = self.sections.get_mut(section_id).unwrap();
        s.dirty = false;
        s.status = "DRAFTED".into();
    }

    // ---- 合成规则事件（SYNTHETIC——D3 真实样本待建）----

    pub fn clause_delete(&mut self, gt: &str) -> Vec<String> {
        self.reqs.remove(gt);
        self.bindings.remove(gt);
        self.matrix.remove(gt);
        let mut dirty = vec![];
        for (sec, v) in &mut self.sections {
            if v.related.iter().any(|g| g == gt) {
                v.dirty = true;
                dirty.push(sec.clone());
            }
        }
        dirty.sort();
        dirty
    }

    pub fn clause_add(
        &mut self,
        gt: &str,
        category: &str,
        clause_no: &str,
        section_id: Option<&str>,
    ) -> Vec<String> {
        self.reqs.insert(
            gt.into(),
            ReqV2 {
                clause_no: clause_no.into(),
                category: category.into(),
                params: HashMap::new(),
            },
        );
        self.matrix.insert(
            gt.into(),
            MatrixRow {
                policy: "new_row".into(),
                score_subitem: None,
                control_price_caliber: None,
            },
        );
        let mut dirty = vec![];
        if let Some(sec) = section_id {
            if let Some(v) = self.sections.get_mut(sec) {
                v.related.push(gt.into());
                v.dirty = true;
                dirty.push(sec.to_string());
            }
        }
        dirty
    }
}
