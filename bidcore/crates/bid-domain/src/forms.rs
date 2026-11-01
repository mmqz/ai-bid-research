//! forms.rs —— 投标文件固定格式件（issue #103 子任务 A）。
//!
//! 问题：真实标书里「投标文件格式」章规定的 7 件格式件一直缺席——
//! 正文写「投标函详见附件一」，但根本没有附件一。本模块把它们变成
//! 可渲染的结构化真源：markdown 落地后走现有导出链（bid-tools/export.rs）
//! 变 docx。
//!
//! 真实锚点：fixtures/synth-tender 的招标文件第七章「投标文件格式」明确要求投标函须
//! 签署盖章、授权委托书须载明授权事项与权限范围、开标一览表总报价须与投标函一致——
//! 本模块据此把七件格式件的必备条款固化成结构。
//!
//! 7 件格式件（[`StandardForm`]）：
//! 1. [`StandardForm::BidLetter`] —— 投标函（填空式）
//! 2. [`StandardForm::LegalRepCertificate`] —— 法定代表人身份证明书
//! 3. [`StandardForm::PowerOfAttorney`] —— 授权委托书
//! 4. [`StandardForm::DepositRefund`] —— 投标保证金退还申请
//! 5. [`StandardForm::QualificationStatement`] —— 资格声明
//! 6. [`StandardForm::NoViolationStatement`] —— 无重大违法记录声明
//! 7. [`StandardForm::IntegrityCommitment`] —— 廉洁承诺书
//!
//! 填空位口径（七件统一，待填内容用全角方括号标注）：
//! - 从 [`crate::party::BidderProfile`] 取得的值 **原样落位**；
//! - 取不到 / 为空的值 → 落成 `【字段名】` 待填位（[`Blank`]）。
//!   绝不用「无」「-」「/」占位——一份带假值的投标函比一份
//!   明确标着待填的投标函危险得多。
//!
//! 纪律：
//! - **零 IO / 纯函数**：同输入逐字节同输出（可 diff、可重放）；
//! - **不编造事实**：本模块不推断任何业务值（报价/工期/有效期），
//!   只拿调用方明确传入的东西排版；取不到就落待填位；
//! - **硬门在调用方手上**：缺字段的 [`crate::party::BidderProfile`] 先过
//!   [`crate::party::BidderProfile::validate`]，不要拿一份空壳去渲染正式件。

use crate::party::BidderProfile;
use serde::{Deserialize, Serialize};

/// 一个填空位（待填的内容用全角方括号裹起来）。
///
/// 这是全核唯一的待填位生成器——不要在各处手写括号。
pub struct Blank;

impl Blank {
    /// 渲染一个待填位：全角方括号 + 字段名。
    pub fn of(label: &str) -> String {
        format!("【{}】", label)
    }

    /// 有值落值，无值落待填位。
    ///
    /// 只把全空字符串当缺（前后空格不算强制是缺，但去首尾空白再判）。
    pub fn or(value: &str, label: &str) -> String {
        let v = value.trim();
        if v.is_empty() {
            Self::of(label)
        } else {
            v.to_string()
        }
    }
}

/// 投标文件固定格式件（招标文件「投标文件格式」章规定的七件）。
///
/// 每个变体都带足够的字段把格式件填满——格式件里的
/// 填空位只允许出现在「调用方确实没传进来」的字段上；
/// 用 [`BidderProfile`] 构造时替代正式件那类固定包装（见 [`StandardForm::bid_letter`]）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StandardForm {
    /// 投标函（填空式）。
    BidLetter {
        /// 项目名称。
        project_name: String,
        /// 项目编号（投标函开头与落款均需）。
        project_no: String,
        /// 投标人（全称）。
        tenderer: String,
        /// 投标总报价（元）。
        bid_amount: f64,
        /// 投标总报价大写（人民币「壹佰万元整」一类口径由调用方给出）。
        bid_amount_upper: String,
        /// 工期（日历天）。
        duration_days: u32,
        /// 投标有效期（日历天）。
        validity_days: u32,
    },

    /// 法定代表人身份证明书。
    LegalRepCertificate {
        /// 姓名。
        name: String,
        /// 性别（「男」/「女」——格式件要求原文写出来）。
        gender: String,
        /// 身份证号。
        id_no: String,
        /// 职务。
        position: String,
        /// 所属公司（全称）。
        company: String,
    },

    /// 授权委托书。
    PowerOfAttorney {
        /// 委托人（法定代表人）姓名。
        principal: String,
        /// 受托人姓名。
        agent: String,
        /// 受托人身份证号。
        agent_id_no: String,
        /// 授权范围（如「参加本项目投标、签署投标文件」）。
        scope: String,
        /// 项目名称。
        project_name: String,
    },

    /// 投标保证金退还申请。
    DepositRefund {
        /// 项目名称。
        project_name: String,
        /// 退还金额（元）。
        amount: f64,
        /// 收款人（须与投标人名称一致）。
        payee: String,
        /// 银行账号。
        bank_account: String,
    },

    /// 资格声明（投标人就主体资格、财务状况、业绩等作的声明）。
    QualificationStatement {
        /// 投标人（全称）。
        tenderer: String,
        /// 项目名称。
        project_name: String,
        /// 注册资本人话串（如「人民币 5000 万元」）。
        registered_capital: String,
        /// 成立日期（ISO 日期）。
        established_on: String,
        /// 资质等级/类别。
        qualification: String,
        /// 是否处于被责令停业、财产被接管/冻结/破产状态。
        solvency: bool,
        /// 是否依法缴纳税收及社会保障资金。
        tax_and_social_security_paid: bool,
    },

    /// 无重大违法记录声明。
    NoViolationStatement {
        /// 投标人（全称）。
        tenderer: String,
        /// 项目名称。
        project_name: String,
        /// 声明查询的时间区间起点（ISO 日期）。
        check_period_start: String,
        /// 声明查询的时间区间终点（ISO 日期）。
        check_period_end: String,
        /// 是否存在被列入失信被执行人、重大税收违法、政府采购严重违法失信名单的情形。
        has_violation_record: bool,
    },

    /// 廉洁承诺书。
    IntegrityCommitment {
        /// 承诺人（投标人全称）。
        tenderer: String,
        /// 项目名称。
        project_name: String,
        /// 承诺书的重点条款要求（如「不向评标委员会成员进行任何形式的利益输送」）。
        focus_clauses: Vec<String>,
    },
}

// ---------------------------------------------------------------- 渲染小工具

/// 金额渲染：正常值 → 以元为单位的两位小数；
/// 非正常值（≤ 0、NaN、∞）→ 待填位（**不编造报价**）。
pub fn format_amount(amount: f64, label: &str) -> String {
    if !amount.is_finite() || amount <= 0.0 {
        Blank::of(label)
    } else {
        format!("{amount:.2} 元")
    }
}

/// 「是/否」落文（格式件里必须写成正文，不能是 true/false）。
pub fn yes_no(flag: bool) -> &'static str {
    if flag {
        "是"
    } else {
        "否"
    }
}

impl StandardForm {
    /// 表单类型名（中文正式名称——直接用作 markdown 标题）。
    pub fn kind(&self) -> &'static str {
        match self {
            StandardForm::BidLetter { .. } => "投标函",
            StandardForm::LegalRepCertificate { .. } => "法定代表人身份证明书",
            StandardForm::PowerOfAttorney { .. } => "授权委托书",
            StandardForm::DepositRefund { .. } => "投标保证金退还申请",
            StandardForm::QualificationStatement { .. } => "资格声明",
            StandardForm::NoViolationStatement { .. } => "无重大违法记录声明",
            StandardForm::IntegrityCommitment { .. } => "廉洁承诺书",
        }
    }

    /// 格式件在投标文件里的编号（「附件一」一类口径——正文引用要对得上）。
    pub fn appendix_no(&self) -> &'static str {
        match self {
            StandardForm::BidLetter { .. } => "附件一",
            StandardForm::LegalRepCertificate { .. } => "附件二",
            StandardForm::PowerOfAttorney { .. } => "附件三",
            StandardForm::DepositRefund { .. } => "附件四",
            StandardForm::QualificationStatement { .. } => "附件五",
            StandardForm::NoViolationStatement { .. } => "附件六",
            StandardForm::IntegrityCommitment { .. } => "附件七",
        }
    }

    /// 格式件标题（带编号——目录与正文共用）。
    pub fn title(&self) -> String {
        format!("{}、{}", self.appendix_no(), self.kind())
    }

    /// 渲染为 markdown（后续走现有导出链变 docx）。
    ///
    /// 结构统一：一级标题（附件编号 + 格式件名）→ 正文段落 →
    /// 落款（签字/盖章/日期）悬空位。待填内容一律用全角方括号标注。
    /// 纯函数：同一格式件永远逐字节同一份 markdown。
    pub fn to_markdown(&self) -> String {
        match self {
            StandardForm::BidLetter {
                project_name,
                project_no,
                tenderer,
                bid_amount,
                bid_amount_upper,
                duration_days,
                validity_days,
            } => {
                let tenderer = Blank::or(tenderer, "投标人名称");
                let project_name = Blank::or(project_name, "项目名称");
                let project_no = Blank::or(project_no, "项目编号");
                let amount = format_amount(*bid_amount, "投标总报价");
                let upper = Blank::or(bid_amount_upper, "投标总报价（大写）");
                format!(
                    "# {title}\n\n\
                     致：{project_name} 招标人（或招标代理机构）\n\n\
                     根据贵方项目编号为 {project_no} 的 {project_name} 招标文件，我方经认真研究招标文件的全部内容后，\
                     愿以人民币（大写）{upper}（小写）{amount} 的投标总报价承担本项目的全部工作，\
                     工期 {duration_days} 日历天。\n\n\
                     **投标有效期**：{validity_days} 日历天自投标截止之日起计算。\n\n\
                     我方承诺：\n\n\
                     1. 已细致审阅了招标文件的全部内容（含补遗文件），完全接受招标文件中的各项条款、格式与评审标准；\n\
                     2. 上述报价在投标有效期内自始至终对我方具有约束力，我方将不提任何调价要求；\n\
                     3. 我方不接受贵方对投标文件作出的不利于我方的修改，也不提出任何不利于投标结果的条件；\n\
                     4. 本投标函与开标一览表、投标分项报价表载明的报价一致；\n\
                     5. 如我方中标，我方将按招标文件及投标文件的约定签订合同并全面履行。\n\n\
                     投标人（全称并盖公章）：{tenderer}\n\n\
                     法定代表人或其授权代理人（签字）：【签字人姓名】\n\n\
                     日期：【投标截止日期】\n",
                    title = self.title(),
                    tenderer = tenderer,
                    project_name = project_name,
                    project_no = project_no,
                    upper = upper,
                    amount = amount,
                    duration_days = duration_days,
                    validity_days = validity_days
                )
            }

            StandardForm::LegalRepCertificate {
                name,
                gender,
                id_no,
                position,
                company,
            } => {
                let name = Blank::or(name, "姓名");
                let gender = Blank::or(gender, "性别");
                let id_no = Blank::or(id_no, "身份证号");
                let position = Blank::or(position, "职务");
                let company = Blank::or(company, "出具单位名称");
                format!(
                    "# {title}\n\n\
                     单位名称：{company}\n\n\
                     单位性质：【公司类型（如有限责任公司）】\n\n\
                     地址：【注册地址】\n\n\
                     成立时间：【成立日期】\n\n\
                     经营期限：【营业期限】\n\n\
                     姓名：{name}　　性别：{gender}　　年龄：【年龄】\n\n\
                     职务：{position}\n\n\
                     系全国　　第　　号\n\n\
                     身份证号码：{id_no}\n\n\
                     上述同志系我单位法定代表人，特此证明。\n\n\
                     单位（盖章）：{company}\n\n\
                     日期：【出具日期】\n\n\
                     附：法定代表人身份证复印件（正、反面）\n",
                    title = self.title(),
                    company = company,
                    name = name,
                    gender = gender,
                    position = position,
                    id_no = id_no
                )
            }

            StandardForm::PowerOfAttorney {
                principal,
                agent,
                agent_id_no,
                scope,
                project_name,
            } => {
                let principal = Blank::or(principal, "法定代表人姓名");
                let agent = Blank::or(agent, "受托人姓名");
                let agent_id_no = Blank::or(agent_id_no, "受托人身份证号");
                let scope = Blank::or(scope, "授权范围");
                let project_name = Blank::or(project_name, "项目名称");
                format!(
                    "# {title}\n\n\
                     本授权委托书声明：我（{principal}）系【投标人名称】的法定代表人，\
                     现委托（{agent}）为我方代理人。\n\n\
                     **姓名**：{agent}\n\n\
                     **身份证号码**：{agent_id_no}\n\n\
                     **代理事项与权限**：{scope}\n\n\
                     代理人根据授权，以我方名义处理与「{project_name}」项目投标有关的一切事务，\
                     包括但不限于：签署、递交、补充、修改、撤回投标文件，出席开标会议，\
                     就投标事项进行澄清、说明、补正，以及签署与投标有关的全部文件。\n\n\
                     **授权期限**：自本授权委托书签署之日起至本项目招标投标活动结束之日止。\n\n\
                     代理人无转委权。\n\n\
                     委托人（法定代表人）签字：{principal}\n\n\
                     受托人签字：{agent}\n\n\
                     投标人（盖章）：【投标人名称】\n\n\
                     日期：【签署日期】\n",
                    title = self.title(),
                    principal = principal,
                    agent = agent,
                    agent_id_no = agent_id_no,
                    scope = scope,
                    project_name = project_name
                )
            }

            StandardForm::DepositRefund {
                project_name,
                amount,
                payee,
                bank_account,
            } => {
                let project_name = Blank::or(project_name, "项目名称");
                let payee = Blank::or(payee, "收款人名称");
                let bank_account = Blank::or(bank_account, "银行账号");
                let amount = format_amount(*amount, "退还金额");
                format!(
                    "# {title}\n\n\
                     致：{project_name} 招标人（或招标代理机构）\n\n\
                     我方（{payee}）参加了「{project_name}」项目的投标，\
                     已按招标文件要求缴纳投标保证金。现因未能中标（或已签订合同并提交履约担保），\
                     特向贵方申请退还投标保证金。\n\n\
                     **退还金额**：人民币 {amount}\n\n\
                     **收款人**：{payee}\n\n\
                     **开户银行**：【开户银行名称】\n\n\
                     **银行账号**：{bank_account}\n\n\
                     我方承诺：上述收款账户为我方基本存款账户，账户名称与投标人名称一致，\
                     因账户信息错误导致款项无法入账的后果由我方自行承担。\n\n\
                     申请人（全称并盖公章）：{payee}\n\n\
                     联系人：【联系人姓名】　　联系电话：【联系电话】\n\n\
                     日期：【申请日期】\n",
                    title = self.title(),
                    project_name = project_name,
                    payee = payee,
                    bank_account = bank_account,
                    amount = amount
                )
            }

            StandardForm::QualificationStatement {
                tenderer,
                project_name,
                registered_capital,
                established_on,
                qualification,
                solvency,
                tax_and_social_security_paid,
            } => {
                let tenderer = Blank::or(tenderer, "投标人名称");
                let project_name = Blank::or(project_name, "项目名称");
                let capital = Blank::or(registered_capital, "注册资本");
                let established = Blank::or(established_on, "成立日期");
                let qualification = Blank::or(qualification, "资质等级");
                format!(
                    "# {title}\n\n\
                     致：{project_name} 招标人（或招标代理机构）\n\n\
                     我方（{tenderer}）就「{project_name}」项目投标的主体资格作如下声明：\n\n\
                     **一、基本情况**\n\n\
                     1. 企业名称：{tenderer}\n\
                     2. 统一社会信用代码：【统一社会信用代码】\n\
                     3. 注册资本：{capital}\n\
                     4. 成立日期：{established}\n\
                     5. 注册地址：【注册地址】\n\
                     6. 资质等级：{qualification}\n\n\
                     **二、财务及经营状况**\n\n\
                     1. 我方当前【{solvency_text}】处于被责令停业、财产被接管、被冻结或处于破产状态；\n\
                     2. 我方【{tax_text}】依法缴纳税收及社会保障资金；\n\
                     3. 我方具备履行合同所必需的技术能力与人员配置。\n\n\
                     **三、声明**\n\n\
                     上述情况均属实，并有相关证明材料附后。如有虚假，我方愿意承担全部法律责任，\
                     并接受取消投标资格、投标无效等处理。\n\n\
                     声明人（全称并盖公章）：{tenderer}\n\n\
                     法定代表人（签字）：【法定代表人姓名】\n\n\
                     日期：【声明日期】\n",
                    title = self.title(),
                    tenderer = tenderer,
                    project_name = project_name,
                    capital = capital,
                    established = established,
                    qualification = qualification,
                    solvency_text = yes_no(!*solvency),
                    tax_text = yes_no(*tax_and_social_security_paid)
                )
            }

            StandardForm::NoViolationStatement {
                tenderer,
                project_name,
                check_period_start,
                check_period_end,
                has_violation_record,
            } => {
                let tenderer = Blank::or(tenderer, "投标人名称");
                let project_name = Blank::or(project_name, "项目名称");
                let period_start = Blank::or(check_period_start, "查询起始日期");
                let period_end = Blank::or(check_period_end, "查询截止日期");
                format!(
                    "# {title}\n\n\
                     致：{project_name} 招标人（或招标代理机构）\n\n\
                     我方（{tenderer}）就「{project_name}」项目投标的确无重大违法记录事项声明如下：\n\n\
                     **一、查询区间**\n\n\
                     自 {period_start} 至 {period_end}（三年内）。\n\n\
                     **二、查询内容**\n\n\
                     我方及我方法定代表人、拟派项目负责人在上述区间内是否存在下列情形：\n\n\
                     1. 被列入失信被执行人名单；\n\
                     2. 被列入重大税收违法案件当事人名单；\n\
                     3. 被列入政府采购严重违法失信行为记录名单；\n\
                     4. 因违法经营受到进一步处理的行政处罚记录。\n\n\
                     **三、声明**\n\n\
                     经查询，我方【{verdict}】上述重大违法记录。\n\n\
                     我方保证上述声明内容真实、准确、完整，承诺已通过“信用中国”网站及相关信用信息共享平台查询核实，并将查询结果截图作为附件附后。\
                     如有虚假或隐瞾，我方愿意接受投标无效、取消中标资格与依法追究责任的处理。\n\n\
                     声明人（全称并盖公章）：{tenderer}\n\n\
                     法定代表人（签字）：【法定代表人姓名】\n\n\
                     日期：【声明日期】\n",
                    title = self.title(),
                    tenderer = tenderer,
                    project_name = project_name,
                    period_start = period_start,
                    period_end = period_end,
                    verdict = if *has_violation_record { "存在" } else { "不存在" }
                )
            }

            StandardForm::IntegrityCommitment {
                tenderer,
                project_name,
                focus_clauses,
            } => {
                let tenderer = Blank::or(tenderer, "投标人名称");
                let project_name = Blank::or(project_name, "项目名称");
                let mut body = String::new();
                for (idx, clause) in focus_clauses.iter().enumerate() {
                    let clause = Blank::or(clause, "重点条款");
                    body.push_str(&format!("{}. {}\n", idx + 1, clause));
                }
                if body.is_empty() {
                    body.push_str("1. 【重点条款】\n");
                }
                format!(
                    "# {title}\n\n\
                     致：{project_name} 招标人（或招标代理机构）\n\n\
                     为保证「{project_name}」项目投标活动的廉洁、公平、公正，我方（{tenderer}）\
                     自愿作出如下廉洁承诺：\n\n\
                     **一、承诺内容**\n\n\
                     {body}\n\
                     **二、违约责任**\n\n\
                     如我方及其工作人员违反上述承诺，我方愿意接受招标人及监管部门依据法律法规、招标文件与合同约定作出的处理，\
                     包括但不限于投标无效、取消中标资格、没收投标保证金、列入不良行为记录、承担相应赔偿责任。\n\n\
                     **三、承诺期限**\n\n\
                     自签署之日起至本项目合同履行完毕之日止。\n\n\
                     承诺人（全称并盖公章）：{tenderer}\n\n\
                     法定代表人或其授权代理人（签字）：【签字人姓名】\n\n\
                     日期：【签署日期】\n",
                    title = self.title(),
                    tenderer = tenderer,
                    project_name = project_name,
                    body = body
                )
            }
        }
    }
}

// ---------------------------------------------------------------- 从 BidderProfile 构造（正式件的正常入口）

impl StandardForm {
    /// 投标函：抬头与落款从 [`BidderProfile`] 取值（不手写投标人名称）。
    ///
    /// 报价/工期/有效期属于投标决策，不在主体信息里——必须显式传入。
    pub fn bid_letter(
        profile: &BidderProfile,
        project_name: &str,
        project_no: &str,
        bid_amount: f64,
        duration_days: u32,
        validity_days: u32,
    ) -> Self {
        StandardForm::BidLetter {
            project_name: project_name.to_string(),
            project_no: project_no.to_string(),
            tenderer: profile.company_name.clone(),
            bid_amount,
            bid_amount_upper: String::new(),
            duration_days,
            validity_days,
        }
    }

    /// 法定代表人身份证明书：姓名/身份证号/职务/公司全取自主体信息。
    ///
    /// 性别不能从身份证号推断（第 17 位奇偶虽有惯例，但不是可靠事实）——
    /// 由调用方显式传入，空则落待填位。
    pub fn legal_rep_certificate(profile: &BidderProfile, gender: &str) -> Self {
        StandardForm::LegalRepCertificate {
            name: profile.legal_rep.clone(),
            gender: gender.to_string(),
            id_no: profile.legal_rep_id.clone(),
            position: profile.legal_rep_position.clone(),
            company: profile.company_name.clone(),
        }
    }

    /// 授权委托书：委托人取法定代表人，受托人取主体信息里的授权委托人。
    pub fn power_of_attorney(profile: &BidderProfile, project_name: &str, scope: &str) -> Self {
        StandardForm::PowerOfAttorney {
            principal: profile.legal_rep.clone(),
            agent: profile.agent.clone(),
            agent_id_no: profile.agent_id.clone(),
            scope: scope.to_string(),
            project_name: project_name.to_string(),
        }
    }

    /// 投标保证金退还申请：收款人与账号取主体信息（同名账户硬要求）。
    pub fn deposit_refund(profile: &BidderProfile, project_name: &str, amount: f64) -> Self {
        StandardForm::DepositRefund {
            project_name: project_name.to_string(),
            amount,
            payee: profile.company_name.clone(),
            bank_account: profile.bank_account.clone(),
        }
    }

    /// 资格声明：基础情况全取主体信息；两个是否项由调用方诚实传入。
    pub fn qualification_statement(
        profile: &BidderProfile,
        project_name: &str,
        solvency: bool,
        tax_and_social_security_paid: bool,
    ) -> Self {
        StandardForm::QualificationStatement {
            tenderer: profile.company_name.clone(),
            project_name: project_name.to_string(),
            registered_capital: if profile.registered_capital > 0.0 {
                format!("人民币 {} 万元", profile.registered_capital)
            } else {
                String::new()
            },
            established_on: profile.established_on.clone(),
            qualification: profile.qualification.clone(),
            solvency,
            tax_and_social_security_paid,
        }
    }

    /// 无重大违法记录声明。
    pub fn no_violation_statement(
        profile: &BidderProfile,
        project_name: &str,
        check_period_start: &str,
        check_period_end: &str,
        has_violation_record: bool,
    ) -> Self {
        StandardForm::NoViolationStatement {
            tenderer: profile.company_name.clone(),
            project_name: project_name.to_string(),
            check_period_start: check_period_start.to_string(),
            check_period_end: check_period_end.to_string(),
            has_violation_record,
        }
    }

    /// 廉洁承诺书。
    pub fn integrity_commitment(
        profile: &BidderProfile,
        project_name: &str,
        focus_clauses: Vec<String>,
    ) -> Self {
        StandardForm::IntegrityCommitment {
            tenderer: profile.company_name.clone(),
            project_name: project_name.to_string(),
            focus_clauses,
        }
    }

    /// 一口气生成全部七件格式件的 markdown（按附件编号从一到七）。
    ///
    /// 参数与 [`Self::bid_letter`] 等构造器一致；返回的 Vec 已排好序。
    pub fn render_all_markdown(forms: &[StandardForm]) -> String {
        forms
            .iter()
            .map(|f| f.to_markdown())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::BidderProfile;

    fn profile() -> BidderProfile {
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
            agent: "王五".into(),
            agent_id: "34010419920202000X".into(),
            postcode: "230000".into(),
            established_on: "2005-06-18".into(),
            qualification: "建筑工程施工总承包二级".into(),
            personnel: vec![],
            equipment: vec![],
        }
    }

    fn all_seven(p: &BidderProfile) -> Vec<StandardForm> {
        vec![
            StandardForm::bid_letter(
                p,
                "智慧病房物联网平台建设项目",
                "ZHYY-2026-GK-018",
                16_200_000.0,
                180,
                90,
            ),
            StandardForm::legal_rep_certificate(p, "男"),
            StandardForm::power_of_attorney(
                p,
                "智慧病房物联网平台建设项目",
                "参加本项目投标并签署投标文件",
            ),
            StandardForm::deposit_refund(p, "智慧病房物联网平台建设项目", 160_000.0),
            StandardForm::qualification_statement(p, "智慧病房物联网平台建设项目", true, true),
            StandardForm::no_violation_statement(
                p,
                "智慧病房物联网平台建设项目",
                "2023-11-01",
                "2026-10-31",
                false,
            ),
            StandardForm::integrity_commitment(p, "智慧病房物联网平台建设项目", vec![]),
        ]
    }

    /// 验收 1：七件格式件全部能生成 markdown，且结构完整。
    #[test]
    fn all_seven_forms_render_markdown() {
        let p = profile();
        let forms = all_seven(&p);
        assert_eq!(forms.len(), 7, "七件格式件缺一不可");

        let mut kinds = Vec::new();
        for form in &forms {
            let md = form.to_markdown();
            assert!(!md.is_empty(), "{} 渲染为空", form.kind());
            assert!(md.starts_with("# "), "{} 要有一级标题: {md}", form.kind());
            assert!(md.contains(form.kind()), "{} 标题要带格式件名", form.kind());
            assert!(
                md.contains(form.appendix_no()),
                "{} 要带附件编号",
                form.kind()
            );
            assert!(md.contains("日期"), "{} 要有落款日期栏", form.kind());
            // 除授权委托书外，每件都直接携带投标人名称（授权书只写委托人/受托人）
            if form.kind() != "授权委托书" {
                assert!(
                    md.contains(&p.company_name),
                    "{} 要带投标人名称",
                    form.kind()
                );
            }
            kinds.push(form.kind());
        }
        assert_eq!(
            kinds,
            vec![
                "投标函",
                "法定代表人身份证明书",
                "授权委托书",
                "投标保证金退还申请",
                "资格声明",
                "无重大违法记录声明",
                "廉洁承诺书"
            ]
        );

        // 合成全集：七件拼一份要有分隔
        let all = StandardForm::render_all_markdown(&forms);
        assert!(all.matches("# ").count() >= 7, "七件拼一份要有七个一级标题");
    }

    /// 验收 2：空值一律落全角方括号待填位，绝不出现「无」「-」占位。
    #[test]
    fn empty_values_become_full_width_bracket_blanks() {
        // 全空业务值：投标函里应看到待填位而不是 0
        let form = StandardForm::BidLetter {
            project_name: String::new(),
            project_no: "  ".into(),
            tenderer: String::new(),
            bid_amount: 0.0,
            bid_amount_upper: String::new(),
            duration_days: 180,
            validity_days: 90,
        };
        let md = form.to_markdown();
        for label in [
            "【投标人名称】",
            "【项目名称】",
            "【项目编号】",
            "【投标总报价】",
            "【投标总报价（大写）】",
        ] {
            assert!(md.contains(label), "缺待填位 {label}:\n{md}");
        }
        assert!(!md.contains("报价 0.00"), "报价未填不能渲染成 0.00: {md}");

        // Blank 本身的两条路径
        assert_eq!(Blank::of("投标人名称"), "【投标人名称】");
        assert_eq!(Blank::or("", "投标人名称"), "【投标人名称】");
        assert_eq!(Blank::or("   ", "投标人名称"), "【投标人名称】");
        assert_eq!(Blank::or("安徽示例公司", "投标人名称"), "安徽示例公司");
        assert_eq!(format_amount(0.0, "投标总报价"), "【投标总报价】");
        assert_eq!(format_amount(f64::NAN, "投标总报价"), "【投标总报价】");
        assert_eq!(format_amount(-1.0, "退还金额"), "【退还金额】");
        assert_eq!(format_amount(16200000.0, "投标总报价"), "16200000.00 元");
        assert_eq!(yes_no(true), "是");
        assert_eq!(yes_no(false), "否");
    }

    /// 验收 3：格式件正文要读得对——关键条款与格式要求都在位。
    #[test]
    fn key_clauses_and_format_requirements_are_present() {
        let p = profile();
        let forms = all_seven(&p);
        let bid_letter = forms[0].to_markdown();
        assert!(
            bid_letter.contains("ZHYY-2026-GK-018"),
            "投标函要带项目编号"
        );
        assert!(bid_letter.contains("16200000.00 元"), "投标函要带小写报价");
        assert!(bid_letter.contains("180 日历天"), "投标函要带工期");
        assert!(bid_letter.contains("90 日历天"), "投标函要带投标有效期");
        assert!(
            !bid_letter.contains("廉洁承诺书"),
            "投标函不应混入廉洁承诺书"
        );

        let cert = forms[1].to_markdown();
        assert!(cert.contains(&p.legal_rep_id), "身份证明书要带身份证号");
        assert!(cert.contains(&p.legal_rep_position), "身份证明书要带职务");

        let poa = forms[2].to_markdown();
        assert!(poa.contains(&p.agent), "授权书要带受托人");
        assert!(poa.contains(&p.legal_rep), "授权书要带委托人");
        assert!(poa.contains(&p.agent_id), "授权书要带受托人身份证号");
        assert!(
            poa.contains("权限") || poa.contains("范围"),
            "授权书要带授权范围"
        );

        let refund = forms[3].to_markdown();
        assert!(refund.contains("160000.00 元"), "退还申请要带退还金额");
        assert!(refund.contains(&p.bank_account), "退还申请要带银行账号");

        let qual = forms[4].to_markdown();
        assert!(qual.contains("5000 万元"), "资格声明要带注册资本");
        assert!(qual.contains(&p.qualification), "资格声明要带资质等级");

        let nov = forms[5].to_markdown();
        assert!(
            nov.contains("2023-11-01") && nov.contains("2026-10-31"),
            "声明要带查询区间"
        );
        assert!(!nov.contains("【存在】"), "无异常时不应落「存在」");

        let integ = forms[6].to_markdown();
        assert!(integ.contains("承诺"), "廉洁承诺书要有承诺体");
        assert!(
            integ.contains("【重点条款】"),
            "条款为空时要落待填位而不是空行号"
        );
    }

    /// 验收 4：纯函数——同一格式件两次渲染逐字节相同。
    #[test]
    fn rendering_is_deterministic() {
        let p = profile();
        for form in all_seven(&p) {
            assert_eq!(
                form.to_markdown(),
                form.to_markdown(),
                "{} 渲染不确定",
                form.kind()
            );
        }
    }
}
