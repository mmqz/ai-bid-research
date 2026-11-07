//! profile_io.rs —— bidder-profile.json 的持久化面（#455 / 审计 A-15）。
//!
//! 迁移动机：bid-domain 头注自declare「本 crate 零 IO（纯逻辑 + 内存态），
//! 持久化由 bid-tools 层承接」，但 party.rs 的 `BidderProfile::load/save`
//! 直用 `std::fs` 且 save 是裸 `fs::write`——进程崩溃可留半写档（A-15）。
//! 本模块把读写接到 bid-tools（纪律归位），写路径按
//! [`store_io`] 同款原子纪律实现：tmp 写盘 → `sync_all` → rename →
//! unix 父目录 fsync（#411 先例）。
//!
//! 语义逐字保持（调用方零行为变化）：
//! - 缺档/不可读 → [`FormError::Io`]（service.rs 依赖它判「未建档」告警面）；
//! - JSON 坏档 → [`FormError::Json`]；
//! - 半填可存（完整性不是保存的前置——人工填一半必须能存）。

use bid_domain::party::{BidderProfile, FormError};
use std::fs;
use std::io::Write;
use std::path::Path;

/// 主体信息在工作区里的缺省落盘名（自 party.rs 迁入——#455）。
pub const PROFILE_FILE: &str = "bidder-profile.json";

/// 从工作区读取 bidder-profile.json。
///
/// 缺档是不可见环境态（工作区无该文件 = 用户未建档），沿用既有口径
/// 以 [`FormError::Io`] 返回，由调用方区分「未建档」与「坏档」。
pub fn load(workspace: &Path) -> Result<BidderProfile, FormError> {
    let path = workspace.join(PROFILE_FILE);
    let raw = fs::read_to_string(&path).map_err(|e| FormError::Io {
        path: path.clone(),
        message: e.to_string(),
    })?;
    serde_json::from_str(&raw).map_err(|e| FormError::Json {
        path,
        message: e.to_string(),
    })
}

/// 写回 bidder-profile.json（tmp → sync_all → rename 原子提交）。
///
/// **不验证完整性**：半填的中间态要能存盘（否则用户填一半就保存不了）。
/// 完整性的硬门在渲染侧（forms 先调 [`BidderProfile::validate`]）。
pub fn save(profile: &BidderProfile, workspace: &Path) -> Result<(), FormError> {
    let path = workspace.join(PROFILE_FILE);
    let body = serde_json::to_string_pretty(profile).map_err(|e| FormError::Json {
        path: path.clone(),
        message: e.to_string(),
    })?;
    fs::create_dir_all(workspace).map_err(|e| FormError::Io {
        path: workspace.to_path_buf(),
        message: e.to_string(),
    })?;
    write_fsync_atomic(&path, body.as_bytes())
}

/// 原子写 + fsync 纪律（与 [`store_io`] 的 `write_fsync_atomic` 同款——
/// #411 fsync 纪律先例；差异仅一处：rename 失败时顺手清掉自己的 tmp，
/// 不留残骸）。
///
/// 崩溃语义：tmp 写盘 + `sync_all` 之后、rename 之前崩溃 → 原文件不动
/// （旧档完整）；rename 是唯一提交点（原子）；rename 后 unix 侧 best-effort
/// fsync 父目录使目录项持久化。
///
/// Windows：目录 fsync 无对应语义（std 无目录句柄 sync）——文档化容忍，
/// 进程崩溃安全（rename 原子），掉电面依赖 NTFS 日志。
fn write_fsync_atomic(path: &Path, bytes: &[u8]) -> Result<(), FormError> {
    let tmp = path.with_extension("json.tmp");
    let io_err = |p: &Path, e: std::io::Error| FormError::Io {
        path: p.to_path_buf(),
        message: e.to_string(),
    };
    {
        let mut f = fs::File::create(&tmp).map_err(|e| io_err(&tmp, e))?;
        f.write_all(bytes).map_err(|e| io_err(&tmp, e))?;
        f.sync_all().map_err(|e| io_err(&tmp, e))?; // 提交点前刷盘（#411）
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp); // rename 失败不留 tmp 残骸（对 store_io 的唯一增量）
        io_err(path, e)
    })?; // 原子提交点
    if cfg!(unix) {
        if let Some(parent) = path.parent() {
            if let Ok(d) = fs::File::open(parent) {
                let _ = d.sync_all(); // 目录项持久化（best-effort）
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bid_domain::party::{EquipmentFact, PersonnelFact};
    use std::path::PathBuf;

    fn tmp_ws(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "profile-io-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn complete() -> BidderProfile {
        BidderProfile {
            company_name: "安徽示例建设集团有限公司".into(),
            legal_rep: "张三".into(),
            legal_rep_id: "340104199001011234".into(),
            registered_capital: 5000.0,
            address: "安徽省合肥市蜀山区示例路 88 号".into(),
            bank_name: "中国建设银行股份有限公司合肥分行".into(),
            bank_account: "34001468608059999999".into(),
            tax_no: "91340100MA2RTXKJ0E".into(),
            contact_person: "李四".into(),
            contact_phone: "13800000000".into(),
            ..Default::default()
        }
    }

    /// 半填 profile 必须能存（完整性不是保存的前置），往返全等；
    /// 缺档读 → Io 错且路径点名 bidder-profile.json（自 party.rs 随迁语义）。
    #[test]
    fn save_then_load_roundtrips_even_when_incomplete() {
        let dir = tmp_ws("roundtrip");

        let mut partial = complete();
        partial.bank_account = String::new();
        assert!(!partial.is_valid(), "半填确实不完整");
        save(&partial, &dir).expect("完整性不是保存的前置");

        let loaded = load(&dir).expect("回读应成功");
        assert_eq!(loaded, partial, "往返全等");

        // A missing file is an IO error, never a silently defaulted profile.
        let missing = dir.join("nope");
        match load(&missing) {
            Err(FormError::Io { path, .. }) => assert!(path.ends_with(PROFILE_FILE)),
            other => panic!("应为 Io 错，实际：{other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// #117：人员/设备长期事实随 profile 落盘往返（磁盘面——自 party.rs 随迁）。
    #[test]
    fn personnel_equipment_facts_disk_roundtrip() {
        let dir = tmp_ws("facts");
        let mut p = complete();
        p.personnel.push(PersonnelFact {
            position: "项目经理".into(),
            name: "王五".into(),
            title: "高级工程师/一级建造师".into(),
            major: "机电工程".into(),
            years: Some(12),
            note: "在岗".into(),
        });
        p.equipment.push(EquipmentFact {
            name: "物联网关".into(),
            model: "IoT-GW-3000".into(),
            quantity: Some(48),
            unit: "台".into(),
            origin: "国产".into(),
            spec: "LoRaWAN/RS485 双模".into(),
            arrival: "合同签订后 7 日内".into(),
        });
        save(&p, &dir).unwrap();
        let back = load(&dir).unwrap();
        assert_eq!(back.personnel.len(), 1);
        assert_eq!(back.personnel[0].years, Some(12));
        assert_eq!(back.equipment[0].quantity, Some(48));
        let _ = fs::remove_dir_all(&dir);
    }

    /// 原子性验收①：正常 save 后 `.json.tmp` 必须被 rename 清除（无残骸）。
    #[test]
    fn save_leaves_no_tmp_debris() {
        let dir = tmp_ws("debris");
        save(&complete(), &dir).unwrap();
        assert!(!dir.join(format!("{PROFILE_FILE}.tmp")).exists(), "临时文件必须被 rename 清除");
        // 覆盖写（第二遍 save）同样无残骸
        save(&complete(), &dir).unwrap();
        assert!(!dir.join(format!("{PROFILE_FILE}.tmp")).exists(), "覆盖写同样无残骸");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 原子性验收②：写失败路径原文件不动 + 错误如实报 Io。
    ///
    /// 注入方式（跨平台、零权限戏法）：把 tmp 位置占成一个目录——
    /// `File::create` 必失败 → save 返回 Io，落盘目标保持旧内容原字节。
    #[test]
    fn failed_write_preserves_original_file() {
        let dir = tmp_ws("fail-inject");
        let old = complete();
        save(&old, &dir).unwrap();
        let old_bytes = fs::read(dir.join(PROFILE_FILE)).unwrap();

        // tmp 位置被目录占位 → 本次写失败
        fs::create_dir_all(dir.join(format!("{PROFILE_FILE}.tmp"))).unwrap();
        let mut newer = complete();
        newer.company_name = "新公司名——不应落盘".into();
        match save(&newer, &dir) {
            Err(FormError::Io { .. }) => {}
            other => panic!("注入失败应为 Io 错，实际：{other:?}"),
        }

        assert_eq!(
            fs::read(dir.join(PROFILE_FILE)).unwrap(),
            old_bytes,
            "写失败路径原文件必须原字节不动（rename 才是提交点）"
        );
        let back = load(&dir).unwrap();
        assert_eq!(back, old, "读回仍是旧档（半写不可能发生）");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 旧版 JSON（无 personnel/equipment 键）从磁盘载入 → 空表不报错
    /// （加性兼容面，磁盘侧口径与 party.rs 的 serde 语义一致）。
    #[test]
    fn legacy_json_on_disk_loads_with_empty_tables() {
        let dir = tmp_ws("legacy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(PROFILE_FILE),
            r#"{ "company_name": "某公司" }"#,
        )
        .unwrap();
        let p = load(&dir).unwrap();
        assert_eq!(p.company_name, "某公司");
        assert!(p.personnel.is_empty() && p.equipment.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
