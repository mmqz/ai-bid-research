#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""GT 增补轮合并：verdicts-merged → ground-truth.json（supplement_round 元数据 + 全量 false 记录）

纪律（issue #48）：
- 原 1212 true / 2067 false 一字不动（只 append）
- 新增 true/false 都带 supplement 标记与理由
- 增补后可复现（veto-measure 同口径重跑）
"""
import json, re, collections

# FG-16（#305）：仓外绝对路径参数化（一次性转换脚本——原值经环境变量可复现）
import os as _os, pathlib as _pl
_REPO = _pl.Path(__file__).resolve().parent.parent.parent
BASE = _os.environ.get('VGT_SUPPLEMENT_DIR') or str(_REPO / 'artifacts' / 'veto-gt-supplement')
GT_PATH = _os.environ.get('VETO_GT') or str(_REPO / 'artifacts' / 'veto-gt' / 'ground-truth.json')
DUMP_DIR = _os.environ.get('VGT_DUMP_DIR') or '/tmp/vgt-dump'
OUT = 'artifacts/veto-gt/ground-truth.json'

# 文件名 → 短代号（沿用原 GT 的 A01/BJ1 风格）
def fkey(fname):
    m = re.match(r'A-(\d+)_', fname)
    if m:
        return f'A{int(m.group(1)):02d}'
    m = re.match(r'A-BJ2025-附件(\d)-', fname)
    if m:
        return f'BJ{m.group(1)}'
    raise ValueError(fname)

# 跨行条款 clause_blocks 扩展（主裁指定）
CLAUSE_EXTEND = {
    ('A-18_塔里木河希尼尔水库监理I标二次_招标文件.pdf', 1394): [1394, 1395, 1396],
}

def norm(s):
    return re.sub(r'\s+', '', s or '')

def main():
    gt = json.load(open(GT_PATH))
    verdicts = [json.loads(l) for l in open(BASE + 'verdicts-merged.jsonl')]
    work = {}
    for tag in ['A', 'B', 'C']:
        for it in json.load(open(BASE + f'batch-{tag}.json')):
            work[(it['file'], it['idx'], it['family'])] = it

    # ---- 块信息（page/locator/text/上下文）----
    dumps = {}
    import os
    for fn in os.listdir(DUMP_DIR):
        if fn.endswith('.blocks.json') and not fn.startswith('_'):
            d = json.load(open(DUMP_DIR + fn))
            dumps[d['file']] = d['blocks']

    # ---- 生成增补注解 ----
    supp = []
    seq_by_file = collections.Counter()
    n_true = n_false = 0
    for v in verdicts:
        fname = v['file']
        blocks = dumps.get(fname, [])
        bidx = {b['idx']: b for b in blocks}
        it = work[(v['file'], v['idx'], v['family'])]
        idx = v['idx']
        if idx is not None and idx in bidx:
            b = bidx[idx]
            page = b.get('page')
            locator = b.get('locator')
            text = b['text']
            i = blocks.index(b)
            ctx_prev = blocks[i-1]['text'] if i > 0 else ''
            ctx_next = blocks[i+1]['text'] if i+1 < len(blocks) else ''
            clause_blocks = CLAUSE_EXTEND.get((fname, idx), [idx])
        else:
            # 无块 FP（idx=None）：只有文本面
            page = None
            locator = None
            text = it['text']
            ctx_prev = ctx_next = ''
            clause_blocks = []

        seq_by_file[fname] += 1
        fid = fkey(fname)
        ann = {
            'id': f'{fid}-S1-{seq_by_file[fname]:03d}',
            'file': fname,
            'page': page,
            'locator': locator,
            'main_block_idx': idx,
            'clause_blocks': clause_blocks,
            'text': text,
            'context_prev': ctx_prev,
            'context_next': ctx_next,
            'is_veto': bool(v['is_veto']),
            'category': v.get('category') or '',
            'note': f"[增补轮1/{v['family']}] {v['note']}",
        }
        if not clause_blocks:
            ann['clause_blocks'] = []
        supp.append(ann)
        if v['is_veto']:
            n_true += 1
        else:
            n_false += 1

    # ---- 完整性自检 ----
    assert len(supp) == len(verdicts) == 1624
    # 原 1212 true 不动校验
    orig_trues = [a for a in gt['annotations'] if a.get('is_veto')]
    assert len(orig_trues) == 1212, f'原 true 数变了: {len(orig_trues)}'

    # ---- 幂等：剔除旧增补轮条目 ----
    gt['annotations'] = [a for a in gt['annotations'] if '-S1-' not in a['id']]

    # ---- 合并 ----
    gt['annotations'].extend(supp)
    new_trues = [a for a in gt['annotations'] if a.get('is_veto')]

    # ---- 顶层元数据 ----
    gt['supplement'] = {
        'supplement_round': 1,
        'supplement_at': '2026-10-29T15:30:00+08:00',
        'supplement_reason': '候选池系统性盲区：成组列举子项无关键词（串通情形/负面清单/否决情形列举）+ 评审标准表行不依赖单行文本 + FP 面重裁决',
        'added_count': len(supp),
        'added_true': n_true,
        'added_false': n_false,
        'method': '池外组传播扫描（S1）+ 评审/审查表级扫描（S2）+ FP 清单带上下文重裁决（S3）+ 池外随机抽样审计 40/件（S4，种子 20261029）；'
                   '三子代理并行裁决 + 主裁全量 true 复核改判 5 条；原 1212 true / 2067 false 逐字不动',
        'discipline': '只按领域语义标注，不看实现结果；每条带理由；判例口径与原 GT 一致（重新招标=false/钱面处置=false/评审表行=true）',
        'worklist': 'scripts/veto_gt/supplement_scan.py 产出（复现命令见该文件头注）',
    }
    gt['stats'] = {
        'files': 14,
        'candidates_adjudicated': len(gt['annotations']),
        'veto_true': len(new_trues),
        'veto_false': len(gt['annotations']) - len(new_trues),
        'by_category': dict(collections.Counter(a['category'] for a in new_trues if a.get('category'))),
        'supplement_round': 1,
    }
    # by_file 更新
    by_file = {}
    for a in gt['annotations']:
        rec = by_file.setdefault(a['file'], {'true': 0, 'adjudicated': 0})
        rec['adjudicated'] += 1
        if a.get('is_veto'):
            rec['true'] += 1
    gt['stats']['by_file'] = by_file
    gt['schema'] = 'veto-gt/2'

    with open(OUT, 'w') as f:
        json.dump(gt, f, ensure_ascii=False, indent=1)

    print(f'增补注解 {len(supp)} 条：true {n_true} / false {n_false}')
    print(f'GT 总注解 {len(gt["annotations"])}：true {len(new_trues)} / false {len(gt["annotations"]) - len(new_trues)}')
    print('分类分布:', json.dumps(gt['stats']['by_category'], ensure_ascii=False))

if __name__ == '__main__':
    main()
