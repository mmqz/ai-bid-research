#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S6 微清扫：最终 58 条未裁决 FP 锚块（双生 false + 3 真漏标），并入 GT。"""
import json, re, collections, os

MEAS = 'artifacts/veto-measure/after-gt-supplement.json'
GT = 'artifacts/veto-gt/ground-truth.json'
DUMP_DIR = '/tmp/vgt-dump/'

# 先复用 s5_final 的规则机
import importlib.util
spec = importlib.util.spec_from_file_location('s5', 'scripts/veto_gt/s5_final.py')

TRUE_EXTRA = [
    (r'由其他单位及法定代表人在自己编制的投标文件上加盖印章和签字', '串通造假否决',
     '以他人名义投标情形子项（双生第二处）——s5_residual 同型判 true'),
    (r'投标文件未按照招标文件规定的格式、内容和要求编制', '形式性否决',
     '否决情形列举 (5)：格式不符（与 (4) 字迹模糊同类）'),
    (r'投标报价超出最高投标报价（不含等于）', '实质性响应否决',
     '16. 投标报价否决情形子项：超最高限价（双生第二处）'),
    (r'签名盖章符合规定', '形式性否决', '2.1.1 形式评审标准 ⑵ 签章行（双生变体块）'),
]

def norm(s):
    return re.sub(r'\s+', '', s or '')

def fkey(fname):
    m = re.match(r'A-(\d+)_', fname)
    if m: return f'A{int(m.group(1)):02d}'
    m = re.match(r'A-BJ2025-附件(\d)-', fname)
    if m: return f'BJ{m.group(1)}'
    raise ValueError(fname)

def main():
    # 载入 s5_final 的规则
    src = open('scripts/veto_gt/s5_final.py').read()
    ns = {}
    exec(src.split('def norm')[0], ns)
    TRUE_RULES = ns['TRUE_RULES'] + list(TRUE_EXTRA)
    FALSE_RULES = ns['FALSE_RULES']

    meas = json.load(open(MEAS))
    gt = json.load(open(GT))
    covered = collections.defaultdict(set)
    for a in gt['annotations']:
        for bi in (a.get('clause_blocks') or []):
            try: covered[a['file']].add(int(bi))
            except (TypeError, ValueError): pass
        if a.get('main_block_idx') is not None:
            try: covered[a['file']].add(int(a['main_block_idx']))
            except (TypeError, ValueError): pass

    dumps = {}
    for fn in os.listdir(DUMP_DIR):
        if fn.endswith('.blocks.json') and not fn.startswith('_'):
            d = json.load(open(DUMP_DIR + fn))
            dumps[d['file']] = {b['idx']: b for b in d['blocks']}

    verdicts = []
    unmatched = []
    for f in meas['fp']:
        nj = norm(f['joined'])
        if len(nj) < 6:
            continue
        hit_block = None
        for b in dumps.get(f['file'], {}).values():
            nb = norm(b['text'])
            if nb and (nj[:28] in nb or nb[:28] in nj) and len(nb) >= 6:
                if b['idx'] not in covered[f['file']]:
                    hit_block = b
                    break
        if hit_block is None:
            continue
        t = f['joined']
        hit = None
        for pat, cat, reason in TRUE_RULES:
            if re.search(pat, t):
                hit = (True, cat, reason); break
        if not hit:
            for pat, bucket, reason in FALSE_RULES:
                if re.search(pat, t):
                    hit = (False, None, f'{bucket}：{reason}'); break
        if hit is None:
            unmatched.append((f, hit_block))
        else:
            verdicts.append({'file': f['file'], 'idx': hit_block['idx'], 'is_veto': hit[0],
                             'category': hit[1], 'note': hit[2]})

    # 去重
    byk = {}
    for v in verdicts:
        k = (v['file'], v['idx'])
        if k not in byk or (v['is_veto'] and not byk[k]['is_veto']):
            byk[k] = v
    verdicts = list(byk.values())

    # 幂等并入 GT
    gt['annotations'] = [a for a in gt['annotations'] if '-S6-' not in a['id']]
    seq = collections.Counter()
    n_true = 0
    for v in verdicts:
        fname = v['file']; idx = v['idx']
        b = dumps[fname][idx]
        seq[fname] += 1
        gt['annotations'].append({
            'id': f'{fkey(fname)}-S6-{seq[fname]:03d}',
            'file': fname, 'page': b.get('page'), 'locator': b.get('locator'),
            'main_block_idx': idx, 'clause_blocks': [idx],
            'text': b['text'],
            'context_prev': '', 'context_next': '',
            'is_veto': bool(v['is_veto']),
            'category': v.get('category') or '',
            'note': f"[增补轮1-S6微清扫] {v['note']}",
        })
        if v['is_veto']: n_true += 1

    trues = [a for a in gt['annotations'] if a.get('is_veto')]
    gt['supplement']['added_count'] += len(verdicts)
    gt['supplement']['added_true'] += n_true
    gt['supplement']['added_false'] += len(verdicts) - n_true
    gt['supplement']['s6_micro'] = f'S6 微清扫：{len(verdicts)} 条双生锚定块（true {n_true}）'
    gt['stats']['candidates_adjudicated'] = len(gt['annotations'])
    gt['stats']['veto_true'] = len(trues)
    gt['stats']['veto_false'] = len(gt['annotations']) - len(trues)
    gt['stats']['by_category'] = dict(collections.Counter(a['category'] for a in trues if a.get('category')))

    with open(GT, 'w') as fo:
        json.dump(gt, fo, ensure_ascii=False, indent=1)
    print(f'S6 微清扫 {len(verdicts)} 条：true {n_true} / false {len(verdicts)-n_true}')
    if unmatched:
        print('未命中', len(unmatched), '条:')
        for f, b in unmatched:
            print(f"  [{f['file'][:11]} #{b['idx']}] {f['joined'][:65]}")

if __name__ == '__main__':
    main()
