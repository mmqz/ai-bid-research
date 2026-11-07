#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""S5 并入 GT：s5-final.jsonl → ground-truth.json（幂等，S5 标记）"""
import json, re, collections, os

# FG-16（#305）：仓外绝对路径参数化（一次性转换脚本——原值经环境变量可复现）
import os as _os, pathlib as _pl
_REPO = _pl.Path(__file__).resolve().parent.parent.parent
S5 = _os.environ.get('VGT_S5_JSONL') or str(_REPO / 'artifacts' / 'veto-gt-supplement' / 's5-final.jsonl')
GT = _os.environ.get('VETO_GT') or str(_REPO / 'artifacts' / 'veto-gt' / 'ground-truth.json')
DUMP_DIR = _os.environ.get('VGT_DUMP_DIR') or '/tmp/vgt-dump'

def norm(s):
    return re.sub(r'\s+', '', s or '')

def fkey(fname):
    m = re.match(r'A-(\d+)_', fname)
    if m: return f'A{int(m.group(1)):02d}'
    m = re.match(r'A-BJ2025-附件(\d)-', fname)
    if m: return f'BJ{m.group(1)}'
    raise ValueError(fname)

def main():
    gt = json.load(open(GT))
    verdicts = [json.loads(l) for l in open(S5)]

    dumps = {}
    for fn in os.listdir(DUMP_DIR):
        if fn.endswith('.blocks.json') and not fn.startswith('_'):
            d = json.load(open(DUMP_DIR + fn))
            dumps[d['file']] = d['blocks']

    # 幂等：去掉旧 S5
    gt['annotations'] = [a for a in gt['annotations'] if '-S5-' not in a['id']]

    # 已存在块（S1 轮已覆盖的）——跳过防重
    covered = collections.defaultdict(set)
    for a in gt['annotations']:
        for bi in (a.get('clause_blocks') or []):
            try: covered[a['file']].add(int(bi))
            except (TypeError, ValueError): pass

    seq = collections.Counter()
    added = []
    for v in verdicts:
        fname = v['file']; idx = v['idx']
        blocks = dumps.get(fname, [])
        bidx = {b['idx']: (b, i) for i, b in enumerate(blocks)}
        if idx is not None and idx in bidx:
            b, i = bidx[idx]
            page, locator, text = b.get('page'), b.get('locator'), b['text']
            ctx_prev = blocks[i-1]['text'] if i > 0 else ''
            ctx_next = blocks[i+1]['text'] if i+1 < len(blocks) else ''
            clause_blocks = v.get('clause_blocks') or [idx]
            # 若主块已被 S1 覆盖但 clause_blocks 有新块，仅当有新块才加
            if all(c in covered[fname] for c in clause_blocks):
                continue
        else:
            page = locator = None; text = ''; ctx_prev = ctx_next = ''
            clause_blocks = []
        seq[fname] += 1
        ann = {
            'id': f'{fkey(fname)}-S5-{seq[fname]:03d}',
            'file': fname, 'page': page, 'locator': locator,
            'main_block_idx': idx, 'clause_blocks': clause_blocks,
            'text': text or '(见 FP 文本)',
            'context_prev': ctx_prev, 'context_next': ctx_next,
            'is_veto': bool(v['is_veto']),
            'category': v.get('category') or '',
            'note': f"[增补轮1-S5残余清扫/{v.get('s5_kind','residual')}] {v['note']}",
        }
        added.append(ann)

    gt['annotations'].extend(added)
    trues = [a for a in gt['annotations'] if a.get('is_veto')]
    gt['supplement']['added_count'] = gt['supplement']['added_count'] + len(added)
    gt['supplement']['added_true'] = sum(1 for a in added if a['is_veto']) + gt['supplement']['added_true']
    gt['supplement']['added_false'] = sum(1 for a in added if not a['is_veto']) + gt['supplement']['added_false']
    gt['supplement']['s5_residual'] = 'S5 残余清扫：双生锚定/编号组头盲区/原判例误标三类，134 条（含真实块位修正 5 组）'
    gt['stats']['candidates_adjudicated'] = len(gt['annotations'])
    gt['stats']['veto_true'] = len(trues)
    gt['stats']['veto_false'] = len(gt['annotations']) - len(trues)
    gt['stats']['by_category'] = dict(collections.Counter(a['category'] for a in trues if a.get('category')))
    by_file = {}
    for a in gt['annotations']:
        rec = by_file.setdefault(a['file'], {'true': 0, 'adjudicated': 0})
        rec['adjudicated'] += 1
        if a.get('is_veto'): rec['true'] += 1
    gt['stats']['by_file'] = by_file

    with open(GT, 'w') as f:
        json.dump(gt, f, ensure_ascii=False, indent=1)
    n_true = sum(1 for a in added if a['is_veto'])
    print(f'S5 并入 {len(added)} 条（true {n_true} / false {len(added)-n_true}，跳过已覆盖 {len(verdicts)-len(added)}）')
    print(f'GT 总量 {len(gt["annotations"])}：true {len(trues)}')

if __name__ == '__main__':
    main()
