//! #316 多标段/多包投标——分包注册表与「按包投」的域面。
//!
//! 真实招标常见分包形态（batch2 实证：`…-第2包.doc`）：
//! 一个项目分 N 个包/标段，每包技术要求、预算不同；投标人可投一包或全部。
//!
//! 域面职责（识别与归属在 bid-parse `packages.rs`，本模块只承载结果）：
//! - [`PackageInfo`]：分包注册表项（id/序号/原文标签/识别证据）；
//! - REQ 归属：`params["package_id"]`（无该键 = 共用条款或无分包结构——
//!   语义由 `packages()` 非空与否区分，与 #271 form_file_id 同一 params 模式，
//!   不动 `Requirement` 结构体字面量面）；
//! - 「按包投」：`selected_package` 记录本项目投哪个包——大纲/矩阵的推导
//!   输入收敛到 [`DomainStore::requirements_in_scope`]（选中包 + 共用条款），
//!   下游（矩阵重建/自评/导出门）全部经派生面自动继承同一口径。
//!
//! 纪律：
//! - PKG id 由识别面按归一化序号确定性签发（PKG-01 = 最小序号）——重跑
//!   幂等；不走 `next_id` 序列（分包是派生元数据，同 FileRole 不占业务
//!   id 命名空间）；
//! - 共用条款（资格要求等）不标 package_id——选任何包都进纲；
//! - 无分包结构的既有项目零位移：packages 为空 → in_scope = 全集，
//!   行为与 #316 之前逐位一致（GT 红线保护）。

use serde::{Deserialize, Serialize};

use crate::Requirement;

/// params 键名（REQ 归属的唯一真源——单一真源纪律，勿另写键名字面量）。
pub const REQ_PARAM_PACKAGE_ID: &str = "package_id";

/// 分包/标段注册表项（识别产物——三形态证据见 `sources`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInfo {
    /// 确定性 id：PKG-01..（按 number 升序；重跑幂等）
    pub id: String,
    /// 归一化序号（「第2包」/「第二标段」→ 2——跨形态去重键）
    pub number: u32,
    /// 原文形态标签（回显用——「第2包」「第1标段」）
    pub label: String,
    /// 维度（包 | 标段——同一文件内两种叫法归同一编号空间）
    pub kind: String,
    /// 识别证据（审计面：`filename:…` / `chapter:…` / `table:…`）
    pub sources: Vec<String>,
}

impl PackageInfo {
    pub fn new(number: u32, label: String, kind: String, sources: Vec<String>) -> Self {
        Self {
            id: format!("PKG-{number:02}"),
            number,
            label,
            kind,
            sources,
        }
    }
}

/// REQ 的分包归属（None = 共用条款或无分包结构）。
///
/// 读 `params[REQ_PARAM_PACKAGE_ID]`——单一真源；类型不符（非字符串）按
/// None 处理（识别面只写字符串，防御旧数据）。
pub fn requirement_package_id(r: &Requirement) -> Option<&str> {
    r.params.get(REQ_PARAM_PACKAGE_ID).and_then(|v| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::DomainStore;

    fn req_with_pkg(id: &str, pkg: Option<&str>) -> Requirement {
        let mut params = HashMap::new();
        if let Some(p) = pkg {
            params.insert(REQ_PARAM_PACKAGE_ID.to_string(), serde_json::json!(p));
        }
        Requirement {
            id: id.into(),
            category: "技术参数".into(),
            clause_no: None,
            anchors: vec![crate::Anchor {
                file: "A-多包招标文件.docx".into(),
                kind: "para".into(),
                locator: format!("第二章/{id}"),
                excerpt: format!("{id} 条款原文——哈希去重需互异"),
                block_idx: None,
                page: None,
                line: None,
            }],
            params,
            source_file: "A-多包招标文件.docx".into(),
        }
    }

    fn store_with_packages() -> DomainStore {
        let mut s = DomainStore::new();
        s.create_project("多包测试", "PKG-316", "明标", "pkg-316")
            .unwrap();
        // 状态机前置：FilesImported 才允许 set_requirements（TC-11 越级防护）
        s.upsert_file(
            "A-多包招标文件.docx",
            "招标文件",
            "sha-316",
            1,
            0,
            10,
            0,
            crate::ParseStatus::Ok,
            "",
        )
        .unwrap();
        s.set_requirements(vec![
            req_with_pkg("REQ-0001", Some("PKG-01")),
            req_with_pkg("REQ-0002", Some("PKG-02")),
            req_with_pkg("REQ-0003", None),
        ])
        .unwrap();
        s.set_packages(vec![
            PackageInfo::new(1, "第1包".into(), "包".into(), vec!["filename:x".into()]),
            PackageInfo::new(2, "第2包".into(), "包".into(), vec!["filename:y".into()]),
        ]);
        s
    }

    #[test]
    fn package_info_id_is_deterministic_by_number() {
        let p = PackageInfo::new(2, "第2包".into(), "包".into(), vec![]);
        assert_eq!(p.id, "PKG-02");
        let p2 = PackageInfo::new(2, "第二包".into(), "包".into(), vec![]);
        assert_eq!(p.id, p2.id, "同 number 跨形态同 id（去重键语义）");
    }

    #[test]
    fn requirement_package_id_reads_params() {
        assert_eq!(
            requirement_package_id(&req_with_pkg("REQ-0001", Some("PKG-01"))),
            Some("PKG-01")
        );
        assert_eq!(
            requirement_package_id(&req_with_pkg("REQ-0001", None)),
            None
        );
        // 非字符串值 → None（防御旧数据）
        let mut r = req_with_pkg("REQ-0001", None);
        r.params
            .insert(REQ_PARAM_PACKAGE_ID.into(), serde_json::json!(3));
        assert_eq!(requirement_package_id(&r), None);
    }

    /// in_scope 语义：未选 = 全集；选中包 = 该包 + 共用（别的包出局）。
    #[test]
    fn requirements_in_scope_selects_package_plus_shared() {
        let mut s = store_with_packages();
        assert_eq!(s.requirements_in_scope().len(), 3, "未选包 = 全集");
        s.select_package("PKG-02").unwrap();
        let scoped = s.requirements_in_scope();
        assert_eq!(scoped.len(), 2, "第2包 + 共用");
        assert!(scoped
            .iter()
            .all(|r| requirement_package_id(r) != Some("PKG-01")));
        // 统计面
        let stats = s.package_stats();
        assert_eq!(stats[0].1, 1, "PKG-01 一条");
        assert_eq!(stats[1].1, 1, "PKG-02 一条");
        assert_eq!(s.shared_requirement_count(), 1);
    }

    /// 选不存在的包 → E_NOTFOUND；重抽后选中包消失 → 复位未选。
    #[test]
    fn select_package_notfound_and_reextract_reset() {
        let mut s = store_with_packages();
        let err = s.select_package("PKG-09").unwrap_err();
        assert_eq!(err.code(), "E_NOTFOUND");
        s.select_package("PKG-02").unwrap();
        assert!(s.selected_package().is_some());
        // 重抽：注册表只剩第1包 → 选中面必须复位（诚实：不可追溯即不选）
        s.set_packages(vec![PackageInfo::new(
            1,
            "第1包".into(),
            "包".into(),
            vec![],
        )]);
        assert!(s.selected_package().is_none(), "选中包消失 → 复位");
        assert_eq!(s.requirements_in_scope().len(), 3, "复位后回全集口径");
    }

    /// 快照往返：分包注册表 + 选中面持久化（重启续跑「投哪个包」不丢）。
    #[test]
    fn snapshot_roundtrip_preserves_packages() {
        let mut s = store_with_packages();
        s.select_package("PKG-02").unwrap();
        let snap = serde_json::to_value(s.to_snapshot()).unwrap();
        // 旧快照兼容：去掉 #316 字段 → 空注册表加载（加性迁移）
        let mut legacy = snap.clone();
        legacy.as_object_mut().unwrap().remove("packages");
        legacy.as_object_mut().unwrap().remove("selected_package");
        let restored_legacy =
            DomainStore::restore(serde_json::from_value(legacy).unwrap()).unwrap();
        assert!(restored_legacy.packages().is_empty());
        assert_eq!(restored_legacy.requirements_in_scope().len(), 3);

        let restored = DomainStore::restore(serde_json::from_value(snap).unwrap()).unwrap();
        assert_eq!(restored.packages().len(), 2);
        assert_eq!(
            restored.selected_package().map(|p| p.id.as_str()),
            Some("PKG-02")
        );
        assert_eq!(
            restored.requirements_in_scope().len(),
            2,
            "选包口径跨重启保持"
        );
    }
}
