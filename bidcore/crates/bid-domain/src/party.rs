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
//! 3. **零 IO 纯函数**：不依赖环境变量/时钟。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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

/// 主体信息在工作区里的缺省落盘名。
pub const PROFILE_FILE: &str = "bidder-profile.json";

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
}

/// 统一社会信用代码格式校验（18 位大写字母/数字，GB 32100-2015
/// 字符集排除 I/O/Z；加权模 31 校验码留给数据源侧）。
pub fn is_valid_credit_code(s: &str) -> bool {
    s.len() == 18
        && s
            .bytes()
            .all(|b| b.is_ascii_digit() || ((b'A'..=b'Z').contains(&b) && b != b'I' && b != b'O' && b != b'Z'))
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
    /// 从工作区读取 bidder-profile.json。
    pub fn load(workspace: &Path) -> Result<Self, FormError> {
        let path = workspace.join(PROFILE_FILE);
        let raw = fs::read_to_string(&path)
            .map_err(|e| FormError::Io { path: path.clone(), message: e.to_string() })?;
        serde_json::from_str(&raw).map_err(|e| FormError::Json { path, message: e.to_string() })
    }

    /// 写回 bidder-profile.json（用于人工填表后回存）。
    ///
    /// **不验证完整性**：半填的中间态要能存盘（否则用户填一半就保存不了）。
    /// 完整性的硬门在渲染侧（[`crate::forms`] 先调 [`BidderProfile::validate`]）。
    pub fn save(&self, workspace: &Path) -> Result<(), FormError> {
        let path = workspace.join(PROFILE_FILE);
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| FormError::Json { path: path.clone(), message: e.to_string() })?;
        fs::create_dir_all(workspace)
            .map_err(|e| FormError::Io { path: workspace.to_path_buf(), message: e.to_string() })?;
        fs::write(&path, body).map_err(|e| FormError::Io { path, message: e.to_string() })
    }

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
                out.push("缺「授权委托人身份证号」（agent_id：已填 agent，身份证号不能空）".to_string());
            } else if !is_valid_id_card(&self.agent_id) {
                out.push(format!(
                    "「授权委托人身份证号」格式不合法（agent_id={}：须为 18 位，末位可为 X）",
                    self.agent_id
                ));
            }
        }
        if !self.established_on.trim().is_empty() && !crate::dates::is_valid_ymd(&self.established_on) {
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
        }
    }

    #[test]
    fn complete_profile_has_no_defects() {
        let p = complete();
        assert!(p.validate().is_empty(), "完整资料应零缺陷: {:?}", p.validate());
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
        assert!(defects.iter().any(|d| d.contains("tax_no=91340100MA2RTXKJ")));
        assert!(defects.iter().any(|d| d.contains("legal_rep_id=")));
        assert!(defects.iter().any(|d| d.contains("bank_account=12345")));
        assert!(defects.iter().any(|d| d.contains("agent_id")));
        assert!(defects.iter().any(|d| d.contains("established_on")));

        assert!(is_valid_credit_code("91340100MA2RTXKJ0E"));
        assert!(!is_valid_credit_code("91340100MA2RTXKJ0I"), "信用代码不含 I");
        assert!(is_valid_id_card("34010419920202000X"));
        assert!(!is_valid_id_card("3401041992020200"));
        assert!(is_valid_bank_account("34001468608059999999"));
        assert!(!is_valid_bank_account("34001"), "不足 6 位拒");
    }

    #[test]
    fn save_then_load_roundtrips_even_when_incomplete() {
        let dir = std::env::temp_dir().join(format!("bid-forms-party-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        // A half-filled profile must still save: otherwise the user loses work.
        let mut partial = complete();
        partial.bank_account = String::new();
        assert!(!partial.is_valid(), "半填确实不完整");
        partial.save(&dir).expect("完整性不是保存的前置");

        let loaded = BidderProfile::load(&dir).expect("回读应成功");
        assert_eq!(loaded, partial, "往返全等");

        // A missing file is an IO error, never a silently defaulted profile.
        let missing = dir.join("nope");
        match BidderProfile::load(&missing) {
            Err(FormError::Io { path, .. }) => assert!(path.ends_with(PROFILE_FILE)),
            other => panic!("应为 Io 错，实际：{other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
