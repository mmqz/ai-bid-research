//! party.rs —— 投标方主体信息 BidderProfile（issue #103 子任务 A）。
//!
//! 定位：**填表用的真源**。[`crate::forms`] 里 7 个格式件的每一个
//! 填空位都从这里取值，绝不各处散落字符串。
//!
//! 三条纪律：
//! 1. **诚实状态**：缺字段就明说缺哪个（[`BidderProfile::validate`] 返回**逐字段**
//!    的人话缺陷清单），绝不用占位符蒙混过关；
//! 2. **硬格式字段格式不对 = 缺**：统一社会信用代码/身份证/银行账号
//!    都是会被评标委员会当场退回的硬格式，格式不合法就当缺处理；
//! 3. **零 IO 纯函数**：不依赖环境变量/时钟；磁盘读写不住本 crate
//!    （bidder-profile.json 的落盘面在 `bid_tools::profile_io`——#455
//!    自本文件迁出，tmp→fsync→rename 原子纪律）。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 格式件层的错误（加载/保存/校验未过）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormError {
    /// 读写文件失败（带路径与原因）。
    Io { path: PathBuf, message: String },
    /// JSON 解析/序列化失败。
    Json { path: PathBuf, message: String },
    /// 主体信息未通过校验（逐条列出缺什么）。
    Incomplete { defects: Vec<String> },
}

impl std::fmt::Display for FormError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormError::Io { path, message } => {
                write!(f, "文件读写失败（{}）：{}", path.display(), message)
            }
            FormError::Json { path, message } => {
                write!(f, "主体信息解析失败（{}）：{}", path.display(), message)
            }
            FormError::Incomplete { defects } => write!(
                f,
                "投标方主体信息不完整，共 {} 项缺陷：{}",
                defects.len(),
                defects.join("；")
            ),
        }
    }
}

impl std::error::Error for FormError {}

/// 拟派人员的一条事实（#117 任务 1：人员配置表的数据面）。
///
/// 与 bid-tools `tables::PersonnelRow` 的分工：本结构是**主体信息里的
/// 长期事实**（跟着投标人走，跨项目复用）；`PersonnelRow` 是**某次导出的
/// 表格行**（本项响应口径）。`years = None` = 未知——渲染层落【待填】，
/// 绝不落「0 年」（假值比缺值危险）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersonnelFact {
    /// 岗位（项目经理/技术负责人/安全员…）。
    #[serde(default)]
    pub position: String,
    /// 姓名（暗标场景由导出层脱敏）。
    #[serde(default)]
    pub name: String,
    /// 职称/执业资格（如「高级工程师 / 一级建造师」）。
    #[serde(default)]
    pub title: String,
    /// 专业。
    #[serde(default)]
    pub major: String,
    /// 本专业年限（年；None = 未知）。
    #[serde(default)]
    pub years: Option<u32>,
    /// 备注（在岗状态、社保等）。
    #[serde(default)]
    pub note: String,
}

/// 拟投入设备的一条事实（#117 任务 1：设备清单的数据面；口径同
/// [`PersonnelFact`]——长期事实，`quantity = None` = 未知 → 【待填】）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EquipmentFact {
    /// 设备名称。
    #[serde(default)]
    pub name: String,
    /// 型号规格。
    #[serde(default)]
    pub model: String,
    /// 数量（None = 未知）。
    #[serde(default)]
    pub quantity: Option<u32>,
    /// 单位（台/套/辆…）。
    #[serde(default)]
    pub unit: String,
    /// 产地/品牌。
    #[serde(default)]
    pub origin: String,
    /// 主要性能指标。
    #[serde(default)]
    pub spec: String,
    /// 进场时间（如「合同签订后 7 日内」）。
    #[serde(default)]
    pub arrival: String,
}

/// 投标方主体信息（填表真源）。
///
/// 字段取名对齐 issue #103 子任务 A 原文的 `BidderProfile`；
/// 另补了几个真实格式件硬需的面（均带 `#[serde(default)]`，
/// 旧快照可直接读入——加性演进）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BidderProfile {
    /// 投标人名称（营业执照全称——所有格式件的抬头）。
    #[serde(default)]
    pub company_name: String,
    /// 法定代表人姓名。
    #[serde(default)]
    pub legal_rep: String,
    /// 法定代表人身份证号（18 位）。
    #[serde(default)]
    pub legal_rep_id: String,
    /// 注册资本（万元为单位的数值；0 = 未填）。
    #[serde(default)]
    pub registered_capital: f64,
    /// 注册地址（营业执照住所）。
    #[serde(default)]
    pub address: String,
    /// 开户银行名称（全称）。
    #[serde(default)]
    pub bank_name: String,
    /// 银行账号。
    #[serde(default)]
    pub bank_account: String,
    /// 统一社会信用代码（18 位）。
    #[serde(default)]
    pub tax_no: String,
    /// 联系人。
    #[serde(default)]
    pub contact_person: String,
    /// 联系电话。
    #[serde(default)]
    pub contact_phone: String,

    // ---- 以下为格式件实需的补充面（可空，缺省回落或落待填位）
    /// 法定代表人职务（身份证明书要写；可空）。
    #[serde(default)]
    pub legal_rep_position: String,
    /// 授权委托人姓名（无委托时空——签署人回落法定代表人）。
    #[serde(default)]
    pub agent: String,
    /// 授权委托人身份证号（可空）。
    #[serde(default)]
    pub agent_id: String,
    /// 邮政编码（可空）。
    #[serde(default)]
    pub postcode: String,
    /// 成立日期 ISO YYYY-MM-DD（可空）。
    #[serde(default)]
    pub established_on: String,
    /// 资质等级/类别（可空）。
    #[serde(default)]
    pub qualification: String,

    // ---- #117 任务 1：表格数据面（人员配置表 / 设备清单表的真源——
    //      主体信息里的长期事实，导出层据此生成表格；缺省 = 空表 + 告警，
    //      不编造）
    /// 拟派人员（人员配置表数据面；空 = 主体信息未录——导出时告警）。
    #[serde(default)]
    pub personnel: Vec<PersonnelFact>,
    /// 拟投入设备（设备清单表数据面；空 = 主体信息未录——导出时告警）。
    #[serde(default)]
    pub equipment: Vec<EquipmentFact>,
}

/// 统一社会信用代码格式校验（18 位大写字母/数字，GB 32100-2015
/// 字符集排除 I/O/Z；加权模 31 校验码留给数据源侧）。
pub fn is_valid_credit_code(s: &str) -> bool {
    s.len() == 18
        && s.bytes().all(|b| {
            b.is_ascii_digit()
                || ((b'A'..=b'Z').contains(&b) && b != b'I' && b != b'O' && b != b'Z')
        })
}

/// 居民身份证号格式校验（18 位：17 位数字 + 1 位数字或 X）。
pub fn is_valid_id_card(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 18 {
        return false;
    }
    let body_ok = b[..17].iter().all(u8::is_ascii_digit);
    let tail_ok = b[17].is_ascii_digit() || b[17] == b'X' || b[17] == b'x';
    body_ok && tail_ok
}

/// 银行账号格式校验（6..=32 位数字——各行口径不一，只挡明显错误）。
pub fn is_valid_bank_account(s: &str) -> bool {
    (6..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

impl BidderProfile {
    /// 检查完整性（缺字段 → 列出缺什么）。
    ///
    /// 返回空集 = 可填表。缺陷顺序固定（主体标识 → 法定代表人 →
    /// 联系人 → 银行），保证同输入同输出（可直接做 diff）。
    /// 每一条都是**人话**并点名字段键，可直接抄进缺失回报。
    pub fn validate(&self) -> Vec<String> {
        let mut out = Vec::new();

        // ---- 主体标识
        if self.company_name.trim().is_empty() {
            out.push("缺「投标人名称」（company_name：须与营业执照全称逐字一致）".to_string());
        }
        if self.tax_no.trim().is_empty() {
            out.push("缺「统一社会信用代码」（tax_no）".to_string());
        } else if !is_valid_credit_code(&self.tax_no) {
            out.push(format!(
                "「统一社会信用代码」格式不合法（tax_no={}：须为 18 位大写字母/数字，不含 I/O/Z）",
                self.tax_no
            ));
        }
        if self.address.trim().is_empty() {
            out.push("缺「注册地址」（address：须与营业执照住所一致）".to_string());
        }
        if !(self.registered_capital.is_finite() && self.registered_capital > 0.0) {
            out.push("缺「注册资本」（registered_capital：单位万元，必须大于 0）".to_string());
        }

        // ---- 法定代表人
        if self.legal_rep.trim().is_empty() {
            out.push("缺「法定代表人姓名」（legal_rep）".to_string());
        }
        if self.legal_rep_id.trim().is_empty() {
            out.push("缺「法定代表人身份证号」（legal_rep_id）".to_string());
        } else if !is_valid_id_card(&self.legal_rep_id) {
            out.push(format!(
                "「法定代表人身份证号」格式不合法（legal_rep_id={}：须为 18 位，末位可为 X）",
                self.legal_rep_id
            ));
        }

        // ---- 联系人
        if self.contact_person.trim().is_empty() {
            out.push("缺「联系人」（contact_person）".to_string());
        }
        if self.contact_phone.trim().is_empty() {
            out.push("缺「联系电话」（contact_phone）".to_string());
        }

        // ---- 银行（保证金退还申请硬需）
        if self.bank_name.trim().is_empty() {
            out.push("缺「开户银行名称」（bank_name）".to_string());
        }
        if self.bank_account.trim().is_empty() {
            out.push("缺「银行账号」（bank_account）".to_string());
        } else if !is_valid_bank_account(&self.bank_account) {
            out.push(format!(
                "「银行账号」格式不合法（bank_account={}：须为 6-32 位数字）",
                self.bank_account
            ));
        }

        // ---- 可选面：给了就必须合法
        if !self.agent.trim().is_empty() {
            if self.agent_id.trim().is_empty() {
                out.push(
                    "缺「授权委托人身份证号」（agent_id：已填 agent，身份证号不能空）".to_string(),
                );
            } else if !is_valid_id_card(&self.agent_id) {
                out.push(format!(
                    "「授权委托人身份证号」格式不合法（agent_id={}：须为 18 位，末位可为 X）",
                    self.agent_id
                ));
            }
        }
        if !self.established_on.trim().is_empty()
            && !crate::dates::is_valid_ymd(&self.established_on)
        {
            out.push(format!(
                "「成立日期」格式不合法（established_on={}：须为 YYYY-MM-DD）",
                self.established_on
            ));
        }

        out
    }

    /// 是否通过校验。
    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }

    /// 硬门：不通过 → [`FormError::Incomplete`]（把**全部**缺陷带出去）。
    pub fn require_valid(&self) -> Result<(), FormError> {
        let defects = self.validate();
        if defects.is_empty() {
            Ok(())
        } else {
            Err(FormError::Incomplete { defects })
        }
    }

    /// 签署主体：有授权委托人时以委托人为签署人，否则法定代表人。
    ///
    /// 投标函 / 授权委托书共用这一口径——避免「投标函签了委托人、
    /// 授权书却写着法定代表人」这类自相矛盾。
    pub fn signer_name(&self) -> &str {
        if self.agent.trim().is_empty() {
            &self.legal_rep
        } else {
            &self.agent
        }
    }

    /// 是否有授权委托人（决定签字人一栏写谁）。
    pub fn has_agent(&self) -> bool {
        !self.agent.trim().is_empty()
    }
}

impl std::fmt::Display for BidderProfile {
    /// 一眼看完的摘要（日志/人工核对用）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}（{}）", self.company_name, self.tax_no)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            legal_rep_position: "董事长".into(),
            agent: String::new(),
            agent_id: String::new(),
            postcode: "230000".into(),
            established_on: "2005-06-18".into(),
            qualification: "建筑工程施工总承包二级".into(),
            personnel: vec![],
            equipment: vec![],
        }
    }

    /// #117：人员/设备事实的 serde 语义（往返/旧档兼容/未知不假造）——
    /// 磁盘往返面在 bid-tools profile_io（#455 迁出）。
    #[test]
    fn personnel_equipment_facts_serde_roundtrip_and_default_absent() {
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
        let raw = serde_json::to_string(&p).unwrap();
        let back: BidderProfile = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.personnel.len(), 1);
        assert_eq!(back.personnel[0].years, Some(12));
        assert_eq!(back.equipment[0].quantity, Some(48));
        // 旧版 JSON（无 personnel/equipment 键）→ 空表加载，不报错
        let legacy = r#"{ "company_name": "某公司" }"#;
        let old: BidderProfile = serde_json::from_str(legacy).unwrap();
        assert!(old.personnel.is_empty() && old.equipment.is_empty());
        // 数值未知 = None（不是 0——导出层落【待填】）
        let unknown = PersonnelFact {
            years: None,
            ..Default::default()
        };
        assert_eq!(unknown.years, None);
    }

    #[test]
    fn complete_profile_has_no_defects() {
        let p = complete();
        assert!(
            p.validate().is_empty(),
            "完整资料应零缺陷: {:?}",
            p.validate()
        );
        assert!(p.is_valid());
        assert!(p.require_valid().is_ok());
        assert_eq!(p.signer_name(), "张三", "无委托人时由法定代表人签署");
        assert!(!p.has_agent());
    }

    #[test]
    fn validate_points_at_each_missing_field() {
        let p = BidderProfile::default();
        let defects = p.validate();
        assert_eq!(defects.len(), 10, "空档恰好 10 项必填缺陷: {:?}", defects);
        let needles = [
            "投标人名称",
            "统一社会信用代码",
            "注册地址",
            "注册资本",
            "法定代表人姓名",
            "法定代表人身份证号",
            "联系人",
            "联系电话",
            "开户银行名称",
            "银行账号",
        ];
        for (idx, needle) in needles.iter().enumerate() {
            assert!(
                defects[idx].contains(needle),
                "第 {idx} 条应点名「{needle}」，实际: {}",
                defects[idx]
            );
        }
        // Each defect must carry the JSON field key so the fix is unambiguous.
        for d in &defects {
            assert!(d.contains('（'), "缺陷必须点名字段键: {d}");
        }

        let err = p.require_valid().unwrap_err();
        let text = err.to_string();
        assert!(text.contains("投标人名称"), "错误里必须有人话说明: {text}");
        assert!(text.contains("company_name"), "错误里必须有字段名: {text}");
        assert!(text.contains("共 10 项缺陷"), "错误里要给出缺陷数: {text}");
    }

    #[test]
    fn malformed_identifiers_are_reported_as_defects() {
        let mut p = complete();
        p.tax_no = "91340100MA2RTXKJ".into();
        p.legal_rep_id = "34010419900101123Y".into();
        p.bank_account = "12345".into();
        p.agent = "王五".into();
        p.established_on = "2005/06/18".into();
        let defects = p.validate();
        assert_eq!(defects.len(), 5, "五项格式错误要全报出: {:?}", defects);
        assert!(defects
            .iter()
            .any(|d| d.contains("tax_no=91340100MA2RTXKJ")));
        assert!(defects.iter().any(|d| d.contains("legal_rep_id=")));
        assert!(defects.iter().any(|d| d.contains("bank_account=12345")));
        assert!(defects.iter().any(|d| d.contains("agent_id")));
        assert!(defects.iter().any(|d| d.contains("established_on")));

        assert!(is_valid_credit_code("91340100MA2RTXKJ0E"));
        assert!(
            !is_valid_credit_code("91340100MA2RTXKJ0I"),
            "信用代码不含 I"
        );
        assert!(is_valid_id_card("34010419920202000X"));
        assert!(!is_valid_id_card("3401041992020200"));
        assert!(is_valid_bank_account("34001468608059999999"));
        assert!(!is_valid_bank_account("34001"), "不足 6 位拒");
    }
}
