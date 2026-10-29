#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S5 残余 FP 清扫：增补轮 1 第二遍——处理三类残余

  1. 双生文本错位：同文同条款出现两处（须知 1.4.3 与 评标办法 3.1.2），GT true 只覆盖
     第一处锚定块，引擎 REQ 锚定第二处 → 块不重叠判 FP。修法：FP 文本若与同文件既有
     GT true 文本对齐，在 FP 锚定块补 true 注解（supplement 标记）。
  2. 编号组头盲区：「16. 投标报价有下列情形之一的：」式编号头被 S1 当子项跳过，
     其子项漏标。修法：残余 FP 逐条语义裁决。
  3. 原判例误标：原定向审计把「同为一个法定代表人」等利害关系项误标 false。
     修法：新增 supplement true 覆盖注解（原条目不动，历史留痕）。

用法：python3 s5_residual.py  →  产出 s5-verdicts.jsonl（人工复核后并入）
"""
import json, re, collections, os

MEAS = 'artifacts/veto-measure/after-gt-supplement.json'
GT = 'artifacts/veto-gt/ground-truth.json'
DUMP_DIR = '/tmp/vgt-dump/'
OUT = '/home/z/my-project/tmp-conv/vgt-supplement/s5-verdicts.jsonl'

def norm(s):
    return re.sub(r'\s+', '', s or '')

# ---- 明确误标家族（主裁裁决：真否决条件，原 false 系误标） ----
MISJUDGED_PATTERNS = [
    (r'同为一个法定代表人', '资格性否决', '利害关系负面清单子项（实施条例34条：单位负责人为同一人的不同单位不得参加同一标段投标）——原定向审计误标 false，增补轮改判'),
    (r'为本标段代建人|为本招标项目的?代建人', '资格性否决', '负面清单子项：代建人不得投标'),
    (r'与.{0,12}(代建人|监理人|招标代理).{0,10}相互控股或参股', '资格性否决', '负面清单子项：与代建/监理/代理机构控股参股'),
    (r'被责令停业的|被责令停业、暂扣', '资格性否决', '负面清单子项：责令停业'),
    (r'投标人之间约定.{0,8}分别以高、中、低价位报价', '串通造假否决', '串通投标情形列举子项（实施条例39条）'),
    (r'由其他单位及法定代表人在自己编制的投标文件上加盖印章', '串通造假否决', '以他人名义投标/弄虚作假情形子项'),
    (r'投标报价超出最高投标报价（不含等于）', '实质性响应否决', '投标报价超最高限价否决情形子项'),
    (r'投标文件未按照招标文件规定的格式、内容和要求编制', '形式性否决', '否决情形列举：格式不符'),
    (r'与本标段的监理人或代建人或招标代理机构相互任职或工作', '资格性否决', '负面清单子项：与代建/监理/代理机构相互任职'),
    (r'投标函及其附录未按规定的格式填写', '形式性否决', '否决情形列举：投标函格式不符'),
    (r'投标文件未对招标文件.{0,15}作出响应|对招标文件的偏差超出', '实质性响应否决', '否决情形列举：实质性不响应'),
]

def main():
    meas = json.load(open(MEAS))
    gt = json.load(open(GT))
    fps = meas['fp']

    # 同文件 GT true 文本索引
    gt_true_by_file = collections.defaultdict(list)
    for a in gt['annotations']:
        if a.get('is_veto'):
            gt_true_by_file[a['file']].append((norm(a['text']), a))

    # 已裁决块索引（避免重复注解）
    adjudicated = collections.defaultdict(set)
    for a in gt['annotations']:
        for bi in (a.get('clause_blocks') or []):
            try: adjudicated[a['file']].add(int(bi))
            except (TypeError, ValueError): pass
        if a.get('main_block_idx') is not None:
            try: adjudicated[a['file']].add(int(a['main_block_idx']))
            except (TypeError, ValueError): pass

    dumps = {}
    for fn in os.listdir(DUMP_DIR):
        if fn.endswith('.blocks.json') and not fn.startswith('_'):
            d = json.load(open(DUMP_DIR + fn))
            dumps[d['file']] = d['blocks']

    verdicts = []
    n_dual = n_misjudge = n_open = 0
    open_items = []
    for f in fps:
        nj = norm(f['joined'])
        if len(nj) < 6:
            continue
        blocks = dumps.get(f['file'], [])
        # FP 锚定块：文本包含匹配的第一个【未裁决】块
        hit = None
        for b in blocks:
            nb = norm(b['text'])
            if nb and (nj[:30] in nb or nb[:30] in nj) and len(nb) >= 6:
                if b['idx'] not in adjudicated[f['file']]:
                    hit = b
                    break
        if hit is None:
            continue
        # 判 1：双生文本（同文件已有等价 GT true）
        dual = False
        for t, a in gt_true_by_file[f['file']]:
            if (nj[:25] in t or t[:25] in nj) and abs(len(t) - len(nj)) < max(12, 0.3 * len(nj)):
                dual = True
                break
        # 判 2：明确误标家族
        mis = None
        for pat, cat, reason in MISJUDGED_PATTERNS:
            if re.search(pat, f['joined']):
                mis = (cat, reason)
                break
        if dual:
            verdicts.append({'file': f['file'], 'idx': hit['idx'], 'is_veto': True,
                             'category': gt_true_by_file[f['file']][0][1]['category'] if False else None,
                             'note': f'双生文本补锚：同条款在本文件出现两处（须知面+评标办法面），GT true 此前只覆盖第一处',
                             's5_kind': 'dual_anchor'})
            # category 从对齐的 true 取
            verdicts[-1]['category'] = next((a['category'] for t, a in gt_true_by_file[f['file']]
                                             if (nj[:25] in t or t[:25] in nj)), '资格性否决')
            n_dual += 1
        elif mis:
            verdicts.append({'file': f['file'], 'idx': hit['idx'], 'is_veto': True,
                             'category': mis[0], 'note': f'残余清扫改判：{mis[1]}',
                             's5_kind': 'misjudged_override'})
            n_misjudge += 1
        else:
            open_items.append({'file': f['file'], 'idx': hit['idx'], 'text': f['joined']})
            n_open += 1

    with open(OUT, 'w') as fo:
        for v in verdicts:
            fo.write(json.dumps(v, ensure_ascii=False) + '\n')
    with open('/home/z/my-project/tmp-conv/vgt-supplement/s5-open-items.json', 'w') as fo:
        json.dump(open_items, fo, ensure_ascii=False, indent=1)

    print(f'双生补锚 {n_dual} | 误标改判 {n_misjudge} | 待人工 {n_open}')
    print('→ ', OUT)

if __name__ == '__main__':
    main()
