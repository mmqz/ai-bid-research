# -*- coding: utf-8 -*-
"""issue #52 第 1 步：493 条 FP 全量归因 v2（UNATTR 细化 + GT 交叉验证）。
桶语义分三类：
  [ENGINE] 引擎过度触发——真该修（排除规则候选）
  [GT]     GT 侧问题——盲区（同款跨文件未标）/ 标注争议 / 测量形态差异，如实报告不改引擎
"""
import json, re
from collections import defaultdict

# FG-16（#305）：仓外绝对路径参数化——默认仓内 fp-diag-baseline.json，FP_DIAG 可覆盖
import os as _os, pathlib as _pl
_REPO = _pl.Path(__file__).resolve().parent.parent.parent
FP = json.load(open(_os.environ.get("FP_DIAG") or _REPO / "scripts" / "fp_attribution" / "fp-diag-baseline.json"))

def hit(t, *pats):
    return any(re.search(p, t) for p in pats)

def bucket(x):
    t = x['joined']
    # ---------- GT 侧（不动引擎） ----------
    # GT 盲区：串通清单子项（GT 在部分文件已标 true——同款跨文件未覆盖）
    if hit(t, '高、中、低价位报价', '内部竞价', '泄露标底', '泄露投标人名称', '同一单位（或个人）编制', '委托同一人办理'):
        return 'GT-BLIND-COLLUSION', '[GT] 串通情形列举子项——GT 部分文件已标 true，同款跨文件盲区'
    # GT 盲区：负面清单同款（A-15 已标 true 的"受过行政处罚"，A-16/A-18 同款盲区）
    if hit(t, '受过行政处罚或刑事处罚', '重大监理质量问题', '招标人不能接受条款'):
        return 'GT-BLIND-NEG', '[GT] 资格负面清单同款——GT 部分文件已标 true，跨文件盲区'
    if hit(t, '被责令停业|吊销资质|吊销营业执照') and x['file'].endswith('.pdf'):
        return 'GT-BLIND-NEG', '[GT] PDF 侧限制投标情形列举项不在 GT 候选池'
    # 测量形态差异：GT text 非连续/省略导致文本互含失效（4.2.7 类——GT 已有 true）
    if hit(t, '完成上传的，视为'):
        return 'GT-MEASURE', '[GT] GT 标注文本省略中段，文本互含失效——引擎分类正确'
    # GT 标注争议（定向审计判 false，语义存疑——如实报告，不改引擎）
    if hit(t, '未提交法定代表人身份证明', '增值税税金', '资格评审标准营业执照', '承包人建议符合', '承包人实施方案符合'):
        return 'GT-DISPUTED', '[GT] 定向审计判 false（通用表述/非审查表）——标注口径，不改引擎'
    if hit(t, '实质性要求和条件的响应（投标人', '注：本表供招标人参考使用'):
        return 'GT-DISPUTED', '[GT] 审查表导语/表注——GT 判 false 的口径面'
    # ---------- 引擎侧（排除规则候选） ----------
    # 评分公式族：分值形态/算式/算法词/评分计算
    if hit(t, r'\d+(\.\d+)?\s*分', r'[A-Z]=\(?[A-Z]', r'[A-Z]×\(', '式中', '评标基准价|下浮值|算术平均值|权重计算|计算出?.{0,12}得分|评分值|最佳报价|报价合理性'):
        return 'SCORE', '[ENGINE] 评分公式/算式形态——算法描述非否决条件'
    # 流标族
    if hit(t, '否决所有投标|否决全部投标', '投标人少于3', '有效投标不足3', '重新招标|重新决标'):
        return 'FAIL-BID', '[ENGINE] 流标/重新招标——招标人程序行为，非对投标人的否决'
    # 拟制修正族
    if hit(t, '大写金额|小写金额|单价金额', '以大写金额为准|以单价金额为准|以开标报价为准|以各分项合价', '修正总价|修正投标报价|算术错误.{0,8}(修正|检查)|缺漏项'):
        return 'ARITH', '[ENGINE] 算术错误修正口径——更正规则，拒不接受才否决'
    # 钱面处置族
    if hit(t, '不予退还|不予退并', '没收其?投标保证金|没收其?履约保证金', '取消其中标资格|取消中标资格', '拒签合同|不与招标人订立合同|未按.{0,10}提交履约(担保|保证金)', '履约保证金的金额|履约保证金收款人'):
        return 'BOND', '[ENGINE] 保证金/中标资格处置（钱面）——中标后处置非投标否决'
    # 评委回避族
    if hit(t, '近亲属', '项目主管部门或者行政监督部门的人员', '与投标人有(经济利益关系|其他利害关系)', '可能影响对投标公正评审'):
        return 'RECUSE', '[ENGINE] 评标委员会成员回避情形——处置对象是评委'
    # 合同解除/履约族（扩展：监理/勘察/设计文件不符合、乙方义务、发包人付款）
    if hit(t, '无法履行或停止履行合同|不履行合同约定的其他义务|无法继续履行', '转包、违法分包|擅自分包|转让监理工作', '未按合同(约定|计划)', '合同一方当事人出现破产', '未能提供令对方满意',
           '文件不符合规范标准以及合同约定|文件不符合法律以及合同约定', '发包人未能按合同约定支付|拖延、拒绝批准付款', '发包人原因造成|委托人原因造成', '乙方不得以任何理由向甲方'):
        return 'CONTRACT', '[ENGINE] 合同解除/违约/廉洁条款——合同期处置非投标否决'
    # 空模板/表头/前附表配置行族
    if hit(t, r'其他情形：。|其他：。', '否决投标情况描述|否决投标的依据', '序号.{0,12}实质性要求和条件.{0,12}(投标)?响应内容', '其他符合招标文件中规定的其他.{0,6}(评审|审查)要求',
           '无行贿犯罪记录查询开始时间', '解释权构成', '条款号条款名称编列内容', '实质性要求和条件（1）招标范围'):
        return 'TEMPLATE', '[ENGINE] 空模板占位/表头/前附表配置行——无可执行否决语义'
    # 程序/格式族（扩展：评标程序/预备会/符合性审查/询标/评标报告/上传程序/提疑/修改撤回）
    if hit(t, '来往函件', '拒收的?，?视为(委托人|发包人|招标人)已经接收', '澄清通知和.{0,12}澄清函是.{0,10}组成部分', '评标程序|评标工作一般按以下程序|召开预备会|评标委员会成员熟悉',
           '初步评审，包括|详细评审。?包括|技术标评审；|商务资信标评审|对投标文件中的问题进行询标|技术组评审内容中需要商务组配合',
           '文件报送程序|视为无效文件', '备选方案外', '完成评标报告|评标报告由评标委员会成员起草', '符合性审查$|投标文件的符合性审查',
           '逾期提出的，招标人及代理机构不予受理', '投标文件的修改与撤回|投标人可以修改或撤回', 'CA数字证书登录|将加密的电子投标文件上传',
           '招标人将审查投标人提交的资格文件|资格文件将被保密', '不得通过补充、修改或撤消投标文件', '不提供书面答复的，由评标委员会'):
        return 'PROGRAM', '[ENGINE] 评标/递交/澄清程序——流程性规定非否决条件'
    # 投标函/声明/格式族
    if hit(t, r'^[0-9①-⑳]{1,3}[、.]我(将|们|方|单位)', '我方在此声明|我承诺|我方承诺', '所递交的投标文件及有关资料内容完整', '投标函附录属于合同文件的组成部分',
           '装订顺序必须与', '基本账户开户许可证复印件', '资格文件的签署应保证', '生产厂家并分别填写上表', '选型原则', '联合体协议书'):
        return 'DECLARE', '[ENGINE] 投标函声明/格式装订要求——格式范本非条款'
    # 现场服务/培训/人员要求族
    if hit(t, '现场服务人员|服务人员的|技术服务人员|培训(所需|计划|内容|方式)', '技术交底', '人天数', '有较强的责任感和事业心', '身体健康，适应现场工作', '了解合同设备的设计', '遵守现场的各项规章'):
        return 'SVC', '[ENGINE] 现场服务/培训/人员要求——合同技术条款'
    # 利害关系列举族（跨文档标注不一致——单独决策）
    if hit(t, '与招标人存在利害关系', '同一个单位负责人', '同为一个法定代表人', '与投标人有其他利害关系', '相互控股或参股', '为本标段代建人'):
        return 'RELATION', '[GT?] 利害关系情形列举——GT 跨文档不一致（PDF true / BJ false），单列决策'
    if hit(t, '两份或多份内容不同的投标文件', '报有两个或多个报价'):
        return 'MULTI-BID', '[GT] 多份投标/多报价——GT 定向审计判 false（未声明有效才否决）'
    if hit(t, '保函的实质性条款不符合'):
        return 'GUARANTEE', '[GT] 保函条款符合性——GT 定向审计判 false'
    if hit(t, '撤销或修改其?投标文件|撤销投标文件'):
        return 'WITHDRAW', '[ENGINE] 有效期内撤销/修改投标——GT 判 false（BOND 面）'
    # 跨章节词面误触发（水政监察类——第五章技术内容的行政处罚描述）
    if hit(t, '水政监察', '对管辖范围内公民、法人'):
        return 'NEG-CONTEXT', '[ENGINE] 负面词跨章节误触发（技术章行政职能描述）'
    # 声明/不良行为说明程序（专题说明）
    if hit(t, '投标人应对其情况专题说明'):
        return 'PROGRAM', '[ENGINE] 不良行为专题说明程序——非否决'
    return 'UNATTR', '未归类——人工审查'

buckets = defaultdict(list)
for x in FP:
    b, why = bucket(x)
    buckets[b].append((x, why))

order = ['SCORE','FAIL-BID','ARITH','BOND','RECUSE','CONTRACT','TEMPLATE','PROGRAM','DECLARE','SVC','NEG-CONTEXT','RELATION','MULTI-BID','GUARANTEE','WITHDRAW','GT-BLIND-COLLUSION','GT-BLIND-NEG','GT-MEASURE','GT-DISPUTED','UNATTR']
eng = gt = un = 0
print(f"{'桶':<20s} {'条数':>4s}  类别/依据")
for b in order:
    if b not in buckets: continue
    n = len(buckets[b])
    tag = buckets[b][0][1].split(']')[0] + ']'
    print(f"{b:<20s} {n:>4d}  {buckets[b][0][1]}")
    if tag == '[ENGINE]': eng += n
    elif tag == '[GT]' or tag == '[GT?]': gt += n
    else: un += n
print('-'*70)
print(f"合计 {sum(len(v) for v in buckets.values())} / {len(FP)} | ENGINE 可修 {eng} | GT 侧 {gt} | 未归类 {un}")

json.dump({b: [x['joined'] for x, _ in v] for b, v in buckets.items()},
          open(_pl.Path(_os.environ.get("FP_DIAG") or _REPO / "scripts" / "fp_attribution" / "fp-diag-baseline.json").parent / 'fp_buckets.json','w'), ensure_ascii=False, indent=0)
if 'UNATTR' in buckets:
    print('\n--- UNATTR 残余 ---')
    for x, _ in buckets['UNATTR']:
        print(f"  [{x['file'][-14:]:14s}] c={str(x.get('clause_no'))[:9]:9s} {x['joined'][:88]}")
