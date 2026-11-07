//! linkage.rs —— 证据链状态账本（引擎 evidence.rs 有状态语义在业务侧的复刻实装）。
//!
//! V3 Q1/Q2 SEMUL（scripts/v3_integration.py LinkageLedger）的产品层落点：
//! - **有状态**迁移：Valid --失配--> Stale --显式重验--> Valid；
//! - **只降不升**：内容恢复（undo/改回原文）不自发回升——重验必须显式发起；
//! - **失配恰一链**：正文变化只降级指纹失配的链，其余链不受牵连（锚不跨链误伤）；
//! - 指纹口径：段落归一化文本 → fnv64（ooxml_util 同构；换 sha256 需过契约审查）。
//!
//! 验证状态（A-08 收口，双行口径）：
//! - 模块语义：LINKAGE_SEMANTICS = IMPLEMENTED（本模块，four_contracts 测试锁定）；
//! - 产品接线：**NOT_WIRED**——断链检测（refresh）/显式重验（reverify）只在
//!   tests/ 里活着，生产链路零引用。易混淆面澄清：证据状态
//!   Active→Expiring→Expired 的「只降不升」是 evidence.rs 的产品实现，
//!   与本模块的**引用链**账本是两回事。
//! DOCX 文件级书签寻址（bookmark_paragraph_map）属引擎面（mdcore），
//! V1-b/V3 SEMUL 保留文件级证据，本模块做语义层实装。

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkageStatus {
    Valid,
    Stale,
}

/// 引用链定位器：段落位置 + 文本指纹（引擎 Locator 七变体的业务最小子集——
/// 段落型；表格/书签型变体随 M3 引擎接入扩展）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkLocator {
    pub paragraph_index: usize,
    pub text_fingerprint: String,
}

pub struct LinkageLedger {
    status: HashMap<String, LinkageStatus>,
    pub audit: Vec<AuditEntry>,
}

#[derive(Debug, Clone)]
pub enum AuditEntry {
    BodyRefresh { degraded: Vec<String> },
    ClaimReverified { cid: String },
    ExternalChangeDetected { cid: String },
}

impl LinkageLedger {
    pub fn new(chain_ids: &[String]) -> Self {
        Self {
            status: chain_ids.iter().cloned().map(|c| (c, LinkageStatus::Valid)).collect(),
            audit: vec![],
        }
    }

    /// 引擎 refresh_document 语义：仅正文变化时跑锚断；失配 → Stale（只降不升）。
    /// 返回本轮降级的链（恰失配者——其余链不牵连）。
    pub fn refresh(
        &mut self,
        body_changed: bool,
        locators: &HashMap<String, LinkLocator>,
        fingerprints: &[String],
    ) -> Vec<String> {
        if !body_changed {
            return vec![];
        }
        let mut degraded = vec![];
        for (cid, loc) in locators {
            // 已 Stale：不自动恢复（只降不升）
            if self.status.get(cid) != Some(&LinkageStatus::Valid) {
                continue;
            }
            let mismatch = loc.paragraph_index >= fingerprints.len()
                || fingerprints[loc.paragraph_index] != loc.text_fingerprint;
            if mismatch {
                self.status.insert(cid.clone(), LinkageStatus::Stale);
                degraded.push(cid.clone());
            }
        }
        if !degraded.is_empty() {
            // 排序保证断言确定性（HashMap 迭代序不稳定）
            degraded.sort();
            self.audit.push(AuditEntry::BodyRefresh { degraded: degraded.clone() });
        }
        degraded
    }

    /// 显式重验（reverify_claim 语义）：新位置 + 指纹 → Valid。
    /// 这是**唯一**的回升通道（undo/内容恢复均不得触发）。
    pub fn reverify(&mut self, cid: &str, locator: &LinkLocator, fingerprints: &[String]) -> bool {
        let ok = locator.paragraph_index < fingerprints.len()
            && fingerprints[locator.paragraph_index] == locator.text_fingerprint;
        if ok {
            self.status.insert(cid.to_string(), LinkageStatus::Valid);
            self.audit.push(AuditEntry::ClaimReverified { cid: cid.into() });
        }
        ok
    }

    /// 外部修改检测（plan_reload 三分之 OfferChoice 前置）：检测到变化但
    /// **不自动 adopt**——登记审计、状态置 Stale，裁决权交业务（默认转人工）。
    /// （R3-M-2 后测试改走 refresh 检测路径；本 API 留供 M3 引擎接入时
    /// plan_reload 语义的显式登记面消费——保留不删。）
    #[allow(dead_code)]
    pub fn detect_external_change(
        &mut self,
        cid: &str,
    ) {
        self.status.insert(cid.to_string(), LinkageStatus::Stale);
        self.audit.push(AuditEntry::ExternalChangeDetected { cid: cid.into() });
    }

    pub fn status_of(&self, cid: &str) -> Option<LinkageStatus> {
        self.status.get(cid).copied()
    }

    pub fn all_valid(&self) -> bool {
        self.status.values().all(|s| *s == LinkageStatus::Valid)
    }
}

/// 段落文本 → 指纹（归一化 = 去全部空白；fnv64 与 Python ooxml_util 同构）。
pub fn text_fingerprint(text: &str) -> String {
    let norm: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    fnv64(norm.as_bytes())
}

/// FNV-1a 64 位（引擎 fingerprint.rs 逐位复刻——公开测试向量对齐，
/// scripts/ooxml_util.py 同构；变更指纹算法属契约变更，需双向审查）。
pub fn fnv64(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(i: usize, fp: &str) -> LinkLocator {
        LinkLocator { paragraph_index: i, text_fingerprint: fp.to_string() }
    }

    #[test]
    fn fnv64_matches_public_vector() {
        // FNV-1a 64 标准测试向量：空串 → cbf29ce484222325（offset basis）
        assert_eq!(fnv64(b""), "cbf29ce484222325");
        // "a" → af63dc4c8601ec8c（Rust/Python ooxml_util 双实现交叉一致值；
        // 首版曾误记 af57dc42d2f3e988——教训：向量必须双实现计算而非记忆引用）
        assert_eq!(fnv64(b"a"), "af63dc4c8601ec8c");
        // 归一化后指纹与文本等价（空白剔除）
        assert_eq!(text_fingerprint("投标 人须知"), text_fingerprint("投标人须知"));
    }

    #[test]
    fn refresh_only_degrades_mismatched_chain() {
        let chains = vec!["C1".to_string(), "C2".to_string()];
        let mut ledger = LinkageLedger::new(&chains);
        let mut locators = HashMap::new();
        locators.insert("C1".into(), loc(0, "fp0"));
        locators.insert("C2".into(), loc(1, "fp1"));
        let fps = vec!["fp0".to_string(), "CHANGED".to_string()];
        let degraded = ledger.refresh(true, &locators, &fps);
        assert_eq!(degraded, vec!["C2"]);
        assert_eq!(ledger.status_of("C1"), Some(LinkageStatus::Valid));
        assert_eq!(ledger.status_of("C2"), Some(LinkageStatus::Stale));
        // 无正文变化：零降级
        assert!(ledger.refresh(false, &locators, &fps).is_empty());
    }

    #[test]
    fn no_auto_recovery_and_explicit_reverify() {
        let chains = vec!["C1".to_string()];
        let mut ledger = LinkageLedger::new(&chains);
        let mut locators = HashMap::new();
        locators.insert("C1".into(), loc(0, "fp0"));
        let _ = ledger.refresh(true, &locators, &["CHANGED".to_string()]);
        // 内容恢复（指纹回到原值）→ 仍 Stale（只降不升）
        let _ = ledger.refresh(true, &locators, &["fp0".to_string()]);
        assert_eq!(ledger.status_of("C1"), Some(LinkageStatus::Stale));
        // 显式重验 → Valid
        assert!(ledger.reverify("C1", &loc(0, "fp0"), &["fp0".to_string()]));
        assert!(ledger.all_valid());
    }
}
