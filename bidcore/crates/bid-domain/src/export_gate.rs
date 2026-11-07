//! export_gate.rs —— 两阶段导出门（V3 Q3 ExportGate 语义的 Rust 实装）。
//!
//! 原子发布纪律：tmp+rename；**EXPORTED 以产物 sha256 落库为界**——
//! 渲染中断/确认前崩溃/取消 → 状态停在 EXPORTING，绝不出现
//! 「业务态 EXPORTED 而产物缺失/无指纹」的虚假成功（T-04 治理面在
//! pdf.rs 的 RunOutcome 分类之上，本模块承载状态面）。
//!
//! 验证状态（A-08 收口，双行口径）：
//! - 模块语义：EXPORT_GATE_SEMANTICS = IMPLEMENTED（本模块，four_contracts
//!   测试锁定两阶段/中断不变式/幂等恢复）；
//! - 产品接线：**NOT_WIRED**——本模块不在生产导出路径上（grep 全仓仅
//!   tests/four_contracts.rs 引用）。产品导出原子性的真源 =
//!   bid-tools/src/export.rs 的 manifest+tmp+rename + service.rs 的
//!   advance_project_to_exported；EXPORTING 中间态与 query_view
//!   （SC-01「进度面必须显式 EXPORTING」）在产品路径不存在。
//!   与 ProjectStatus::ReadyToExport→Exported 状态机的接线属后续单
//!   （需 DomainStore 持久化设计——等 lib.rs/service.rs 在飞波次合入后）。

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPhase {
    ReadyToExport,
    Exporting,
    Exported,
}

/// 查询面视图（SC-01，issue #35/#12 吸收）：任意查询消费者（进度查询 /
/// 产物列表 / 状态接口）的**唯一读取口径**——防各消费面自行解读状态出现
/// 「缺省成功」语义。三态纪律：
/// - 中间态必须**显式 EXPORTING**（禁止把未确认读成完成）；
/// - 进度面（preview_sha——渲染指纹）与产物面（artifact_sha——落库指纹）
/// **分离**：产物不存在 ≠ 导出失败，但未确认 ≠ EXPORTED；
/// - EXPORTED 仅当产物 sha256 已落库（与 consistent() 不变式同源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportQueryView {
    NotStarted,
    /// 渲染完成、确认未落库：进度面可见 preview_sha；产物面为空
    Exporting { preview_sha: String },
    /// 确认落库：产物面可见 artifact_sha
    Exported { artifact_sha: String },
}

#[derive(Debug)]
pub struct ExportGate {
    state: ExportPhase,
    rendered_sha: Option<String>,
    artifact_sha: Option<String>,
}

impl Default for ExportGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportGate {
    pub fn new() -> Self {
        Self { state: ExportPhase::ReadyToExport, rendered_sha: None, artifact_sha: None }
    }

    pub fn state(&self) -> ExportPhase {
        self.state
    }

    /// 渲染：进入 EXPORTING，计算产物指纹（渲染后确认前——中断注入点）。
    pub fn render(&mut self, artifact_bytes: &[u8]) -> String {
        self.state = ExportPhase::Exporting;
        let mut h = Sha256::new();
        h.update(artifact_bytes);
        let sha = hex::encode(h.finalize());
        self.rendered_sha = Some(sha.clone());
        sha
    }

    /// 确认：**sha256 落库为界**——无 rendered_sha 就没有 EXPORTED（幂等：
    /// 同产物确认两次状态不变；恢复路径：中断后重新 confirm 同 sha）。
    pub fn confirm(&mut self) -> bool {
        match &self.rendered_sha {
            None => false,
            Some(sha) => {
                self.artifact_sha = Some(sha.clone());
                self.state = ExportPhase::Exported;
                true
            }
        }
    }

    pub fn artifact_sha(&self) -> Option<&str> {
        self.artifact_sha.as_deref()
    }

    pub fn is_exported(&self) -> bool {
        self.state == ExportPhase::Exported
    }

    /// 一致性不变式：EXPORTED ⟺ 产物 sha256 已落库（虚假成功的不可能条件）。
    pub fn consistent(&self) -> bool {
        (self.state == ExportPhase::Exported) == self.artifact_sha.is_some()
    }

    /// SC-01 查询面唯一口径：中断后任何查询（进度/产物列表/状态）在中间态
    /// 必须读到显式 EXPORTING——绝无缺省成功语义。`is_exported`/`artifact_sha`
    /// 等单点读取器由消费方内部使用；**对外查询面一律经本视图**（单一实现，
    /// 防多面各自解读）。preview（进度面）与 artifact（产物面）分离：
    /// 未确认 ≠ EXPORTED，产物未落库 ≠ 导出失败。
    pub fn query_view(&self) -> ExportQueryView {
        debug_assert!(self.consistent(), "查询面前置：门不变式必须成立");
        match &self.state {
            ExportPhase::ReadyToExport => ExportQueryView::NotStarted,
            ExportPhase::Exporting => ExportQueryView::Exporting {
                preview_sha: self.rendered_sha.clone().unwrap_or_default(),
            },
            ExportPhase::Exported => ExportQueryView::Exported {
                artifact_sha: self.artifact_sha.clone().unwrap_or_default(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_phase_and_interrupt_invariant() {
        let mut gate = ExportGate::new();
        assert_eq!(gate.state(), ExportPhase::ReadyToExport);
        // 渲染后（确认前）中断 → 停在 EXPORTING，无落库
        let sha = gate.render(b"docx-bytes");
        assert_eq!(gate.state(), ExportPhase::Exporting);
        assert!(gate.artifact_sha().is_none());
        assert!(!gate.is_exported());
        assert!(gate.consistent());
        // 确认 → EXPORTED，sha 落库
        assert!(gate.confirm());
        assert_eq!(gate.artifact_sha(), Some(sha.as_str()));
        assert!(gate.is_exported());
        assert!(gate.consistent());
    }

    #[test]
    fn no_confirm_means_never_exported() {
        // 中断注入：渲染后不 confirm（等效确认前 kill）——永不 EXPORTED
        let mut gate = ExportGate::new();
        let _sha = gate.render(b"partial");
        assert_eq!(gate.state(), ExportPhase::Exporting);
        assert!(!gate.is_exported());
        assert!(gate.artifact_sha().is_none(), "无落库就不该有 sha——虚假成功条件");
    }

    #[test]
    fn confirm_idempotent_same_sha() {
        let mut gate = ExportGate::new();
        let _s1 = gate.render(b"same");
        gate.confirm();
        let before = gate.artifact_sha().unwrap().to_string();
        // 重复确认：幂等（同 sha、状态不变）
        assert!(gate.confirm());
        assert_eq!(gate.artifact_sha(), Some(before.as_str()));
        assert_eq!(gate.state(), ExportPhase::Exported);
    }

    #[test]
    fn resume_after_interrupt_same_artifact() {
        // 恢复路径：中断后重新导出同产物 → 同 sha
        let mut g1 = ExportGate::new();
        let sha1 = g1.render(b"artifact-v1");
        g1.confirm();
        let mut g2 = ExportGate::new(); // 恢复会话
        let sha2 = g2.render(b"artifact-v1");
        assert_eq!(sha1, sha2, "同产物 sha 必须一致（幂等恢复）");
        g2.confirm();
        assert_eq!(g2.artifact_sha(), Some(sha1.as_str()));
    }
}
