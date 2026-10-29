#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S5 终裁：150 条残余 FP 的规则族裁决 + 特殊块修正，并入 GT 后重测

TRUE 规则优先于 FALSE 规则；无规则命中则报出来人工看。
特殊修正：标题页错配 FP 的真实块位补 true（A-18 #184 逾期拒收 / A-01 #771-773 招标人串通 /
A-01 #1103-1104+#1419-1420 签章评审行 / A-18 #272-277 1.12 实质性+不允许偏差）。
"""
import json, re, collections

OPEN = '/home/z/my-project/tmp-conv/vgt-supplement/s5-open-items.json'
GT = 'artifacts/veto-gt/ground-truth.json'
DUMP_DIR = '/tmp/vgt-dump/'

TRUE_RULES = [
    (r'不允许偏离招标文件的实质性要求|不允许偏离$', '实质性响应否决', '1.12 偏差不允许——实质性要求不可偏离（与 A-05 #305 判例一致）'),
    (r'^1\.4\.2.*前附表规定本工程不接受联合体投标', '资格性否决', '前附表 1.4.2 已填列：本工程不接受联合体投标（实际条件已载明，非空模板）'),
    (r'签名盖章符合规定', '形式性否决', '2.1.1 形式评审标准行：签章符合规定+造价专业人员逐页签名（跨行）'),
    (r'未响应招标文件的实质性要求和条件', '实质性响应否决', '否决情形列举子项：实质性要求未响应'),
    (r'按废标处理', '串通造假否决', '10.8 其他行为一经查出按废标处理（弄虚作假/违法节）'),
    (r'不在.{0,25}不良行为处罚期内', '资格性否决', '资格条件：不在禁止投标的处罚期内'),
    (r'涵盖的内容有对招标文件.{0,40}实质性偏差', '实质性响应否决', '16. 投标报价否决情形子项：报价涵盖内容实质性偏差'),
    (r'投标保证金出具人与被保证的投标人名称不一致', '实质性响应否决', '21. 保证金瑕疵子项：出具人与投标人名称不一致'),
    (r'中有不符合招标文件第[五六七]章', '实质性响应否决', '否决情形 23/24：技术部分/施工组织设计/承包人建议书不符实质性要求'),
    (r'法律法规或投标人须知前附表规定的其他情形', '资格性否决', '负面清单兜底子项（14）——与增补轮判例一致'),
]
FALSE_RULES = [
    (r'投标人少于3|否决所有投标|否决全部投标|重新招标|废标项目重新招标', '程序参数', '重新招标/流标情形组'),
    (r'不以此否决其投标|不参与评标基准价', '评分规则', '有效报价定义（明确不以此否决）'),
    (r'以大写金额为准|以单价金额为准|以开标报价为准|视为缺漏项价格|以各分项合价累计数为准', '拟制/推定', '算术修正/拟制规则'),
    (r'拒签合同|履约担保|来往函件|现场服务|服务人员|承包人|乙方|委托人应当及时接收|监理人转让|监理人未按|监理人无法', '履约期条款', '合同履行期条款'),
    (r'取消中标资格|不予退还|没收其投标保证金|列入不良行为记录|撤销或修改其投标文件|撤销投标文件', '钱面处置', '保证金没收/取消中标资格类钱面处置'),
    (r'装订顺序', '无否决语义', '业绩材料装订顺序要求'),
    (r'我方在此声明|我单位保证|保证上述承诺', '无否决语义', '投标函承诺/声明面（否决条件在所引条款）'),
    (r'项目主管部门或者行政监督部门的人员|近亲属|应当回避', '评委会权限', '评标委员会成员回避名单'),
    (r'其他情形：。', '表格/格式占位', '空白填空项'),
    (r'澄清通知|澄清函是|组成部分', '定义/口径', '澄清文件构成定义'),
    (r'水政监察|选型原则|设备组成', '无否决语义', '正文无关内容/标题页碎片'),
    (r'^1\.12\.1实质性要求和条件（1）', '定义/口径', '1.12.1 实质性要求清单式定义（触发在 1.12.3）'),
    (r'CA数字证书|应充分考虑', '程序参数', '递交通道程序说明'),
    (r'询标记录|显失公正|重新决标|评标程序评标工作|初步评审。包括|详细评审。包括', '程序参数', '评标程序/询标程序说明'),
    (r'得\d|分值|评分|扣分|评标基准|算术平均值|计算出得分|得分[ABC]|ISO|BBB级|不良行为记录和处罚|下浮值|偏差率|权重计算|最终得分|中标候选人排序|技术得分\+资信', '评分规则', '评分/基准价计算面'),
    (r'投标辅助资料如不能完全表述', '无否决语义', '补充说明条款'),
    (r'3\.1\.2投标人须知前附表规定.{0,8}不接受联合体投标的', '条件引用', '转引前附表句式（判例一致）'),
    (r'履约保证金的金额|收款人（全称）', '履约期条款', '履约保证金账户信息'),
    (r'提疑|匿名在网上提出', '程序参数', '提疑/质疑程序安排'),
    (r'评标报告|少数服从多数', '程序参数', '评标报告程序'),
    (r'不良行为记录（仅限于一般不良行为记录）', '评分规则', '不良行为记录扣分项'),
    (r'部分\d+\.\d+分|：\d+\.\d+分', '评分规则', '分值构成说明'),
    (r'了解合同设备的设计|现场工作经验', '履约期条款', '服务人员资历要求（履约面）'),
    (r'不符合文件报送程序', '履约期条款', '合同文件管理程序（履约面）'),
    (r'以此为准，投标人需对应', '定义/口径', '格式章节响应说明（否决条件在评标办法 23）'),
    (r'塔里木河流域希尼尔水库除险加固工程', '无否决语义', '标题页碎片——FP 锚错位，真实条款在正文（已另行补 true）'),
    (r'其他可以不予', '钱面处置', '保证金不予退还组子项（上下文 3.4.4 已核）'),
    (r'逾期送达', '程序参数', '递交流程序述（拒收处置条款在 5.2 真实块位）'),
]

# 特殊块修正（真实块位补 true——PDF 标题页错配 FP 的本体）
SPECIAL_TRUES = [
    ('A-18_塔里木河希尼尔水库监理I标二次_招标文件.pdf', 184, [184, 185], '程序性否决',
     '5.2 逾期送达/未送达指定地点/不按要求密封的投标文件拒收（真实块位修正：FP 锚到标题页碎片）'),
    ('A-01_塔里木河希尼尔水库坝基防渗施工标_招标文件.pdf', 771, [771, 772, 773], '串通造假否决',
     '「下列行为均属招标人与投标人串通投标」组头+⑴子项（招标人侧串通，实施条例40条）'),
    ('A-01_塔里木河希尼尔水库坝基防渗施工标_招标文件.pdf', 1103, [1103, 1104], '形式性否决',
     '2.1.1 形式评审标准 ⑵ 签章符合规定行（跨行）'),
    ('A-01_塔里木河希尼尔水库坝基防渗施工标_招标文件.pdf', 1419, [1419, 1420], '形式性否决',
     '同签章评审行第二处出现（同文双生）'),
    ('A-18_塔里木河希尼尔水库监理I标二次_招标文件.pdf', 272, [272, 273, 274, 275, 276, 277], '实质性响应否决',
     '前附表 1.12 实质性要求和条件（已填列条款清单）+ 1.12.3 偏差不允许——跨行整条款'),
]

def norm(s):
    return re.sub(r'\s+', '', s or '')

def main():
    open_items = json.load(open(OPEN))
    gt = json.load(open(GT))
    adjudicated = collections.defaultdict(set)
    for a in gt['annotations']:
        for bi in (a.get('clause_blocks') or []):
            try: adjudicated[a['file']].add(int(bi))
            except (TypeError, ValueError): pass
        if a.get('main_block_idx') is not None:
            try: adjudicated[a['file']].add(int(a['main_block_idx']))
            except (TypeError, ValueError): pass

    dumps = {}
    import os
    for fn in os.listdir(DUMP_DIR):
        if fn.endswith('.blocks.json') and not fn.startswith('_'):
            d = json.load(open(DUMP_DIR + fn))
            dumps[d['file']] = {b['idx']: (b, i) for i, b in enumerate(d['blocks'])}

    verdicts = []
    unmatched = []
    SKIP = {('A-18_塔里木河希尼尔水库监理I标二次_招标文件.pdf', 276)}
    for o in open_items:
        if (o['file'], o['idx']) in SKIP:
            continue
        t = o['text']
        hit = None
        for pat, cat, reason in TRUE_RULES:
            if re.search(pat, t):
                hit = (True, cat, reason); break
        if not hit:
            for pat, bucket, reason in FALSE_RULES:
                if re.search(pat, t):
                    hit = (False, None, f'{bucket}：{reason}'); break
        if hit is None:
            unmatched.append(o)
        else:
            verdicts.append({'file': o['file'], 'idx': o['idx'], 'is_veto': hit[0],
                             'category': hit[1], 'note': hit[2], 's5_kind': 'residual_rule'})

    # 特殊块修正（跳过已裁决块）
    for fname, idx, cbs, cat, note in SPECIAL_TRUES:
        if idx in adjudicated[fname]:
            continue
        verdicts.append({'file': fname, 'idx': idx, 'is_veto': True, 'category': cat,
                         'note': note, 's5_kind': 'special_block', 'clause_blocks': cbs})

    # 去重（同文件同块只留一条；true 优先）
    byk = {}
    for v in verdicts:
        k = (v['file'], v['idx'])
        if k not in byk or (v['is_veto'] and not byk[k]['is_veto']):
            byk[k] = v
    verdicts = list(byk.values())

    with open('/home/z/my-project/tmp-conv/vgt-supplement/s5-final.jsonl', 'w') as f:
        for v in verdicts:
            f.write(json.dumps(v, ensure_ascii=False) + '\n')

    n_true = sum(1 for v in verdicts if v['is_veto'])
    print(f'S5 终裁 {len(verdicts)} 条：true {n_true} / false {len(verdicts) - n_true}')
    if unmatched:
        print(f'!! 未命中规则 {len(unmatched)} 条（需人工）：')
        for u in unmatched:
            print(f"   [{u['file'][:12]} #{u['idx']}] {u['text'][:70]}")

if __name__ == '__main__':
    main()
