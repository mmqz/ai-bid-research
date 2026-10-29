#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""supplement_scan —— issue #48 GT 增补轮：候选池盲区扫描器

三类盲区（issue #48）：
  S1 成组列举：父条款带触发词、编号子项无关键词 → 组传播入池
  S2 评审/审查表行：表格级扫描（不依赖单行文本命中词表）
  S3 FP 清单：引擎已抽但 GT 无对齐 true 的条目（带上下文重裁决）
  S4 抽样审计：前三类之外的池外块，固定种子抽样（量残余盲区）

输入：veto-gt-dump 的 *.blocks.json + ground-truth.json + veto-measure 输出
输出：worklist-supplement.json（供标注员裁决，每条带上下文）

纪律（issue #48）：
  - 只按领域语义标注，不看实现结果
  - 原有 1212 true 不动（只加不改）
"""
import json
import os
import random
import re
import sys
import unicodedata

DUMP_DIR = sys.argv[1] if len(sys.argv) > 1 else '/tmp/vgt-dump'
GT_PATH = sys.argv[2] if len(sys.argv) > 2 else 'artifacts/veto-gt/ground-truth.json'
MEASURE_PATH = sys.argv[3] if len(sys.argv) > 3 else '/tmp/main-verify.json'
OUT_PATH = sys.argv[4] if len(sys.argv) > 4 else '/tmp/worklist-supplement.json'
AUDIT_PER_FILE = int(sys.argv[5]) if len(sys.argv) > 5 else 40

# ---------- 规范化（与 veto-measure 同口径：去全部空白） ----------
def norm(s):
    if not s:
        return ''
    return re.sub(r'\s+', '', s)

# ---------- 编号子项识别 ----------
ENUM_RE = re.compile(
    r'^(?:[（(][一二三四五六七八九十百0-9]{1,3}[)）]|'
    r'[①②③④⑤⑥⑦⑧⑨⑩⑪⑫⑬⑭⑮]|'
    r'\d{1,2}[、.．]\s*)'
)
def is_enum_item(text):
    t = (text or '').strip()
    return bool(ENUM_RE.match(t)) and len(norm(t)) >= 4

def enum_marker(text):
    t = (text or '').strip()
    m = ENUM_RE.match(t)
    return m.group(0) if m else None

# ---------- 组头语义（父条款触发） ----------
GROUP_HEAD_RE = re.compile(
    r'(下列情形|以下情形|下列情形|下列情形之一|属于串通|视为串通|视为投标人相互串通|'
    r'不得参加投标|不得单独参加投标|无权参加|取消其投标资格|否决其投标|'
    r'其投标(将|应|作|被)?(被)?(否决|拒绝|无效|废标)|按无效标处理|作废标处理|'
    r'投标保证金(将|应)?不予退还|不予接受|有权否决|有权拒绝|废标情形|无效投标情形)'
)

def is_group_head(text):
    t = (text or '').strip()
    if not t or len(norm(t)) < 6:
        return False
    return bool(GROUP_HEAD_RE.search(t))

def load_dump(fname):
    with open(os.path.join(DUMP_DIR, fname + '.blocks.json')) as f:
        return json.load(f)

def context(blocks, i, span=2):
    out = []
    for j in range(max(0, i - span), min(len(blocks), i + span + 1)):
        b = blocks[j]
        tag = '>>' if j == i else ('..' if j < i else '..')
        out.append(f"[{tag}{b['idx']}] {b['text'][:90]}")
    return out

def main():
    gt = json.load(open(GT_PATH))
    meas = json.load(open(MEASURE_PATH))
    # GT 已覆盖块（按文件）——原池 true/false 都算已裁决
    adjudicated = {}   # file -> set(block idx)
    for a in gt['annotations']:
        for bi in (a.get('clause_blocks') or []):
            adjudicated.setdefault(a['file'], set()).add(bi)
        if a.get('main_block_idx') is not None:
            adjudicated.setdefault(a['file'], set()).add(a['main_block_idx'])

    fp_by_file = {}
    for f in meas.get('fp', []):
        fp_by_file.setdefault(f['file'], []).append(f['joined'])

    # 原 GT false 判例文本索引（同文同文本已判 false 的，不重复裁决——一致性沿用原判）
    false_texts = set()
    for a in gt['annotations']:
        if not a.get('is_veto'):
            false_texts.add((a['file'], norm(a.get('text', ''))[:40]))

    worklist = []
    stats = {'S1_group_children': 0, 'S1_group_heads': 0, 'S2_table_rows': 0,
             'S3_fp': 0, 'S3_fp_already_false': 0, 'S4_audit': 0,
             'skip_adjudicated': 0, 'files': 0}

    dump_files = sorted(f[:-len('.blocks.json')] for f in os.listdir(DUMP_DIR)
                        if f.endswith('.blocks.json') and not f.startswith('_'))
    rng = random.Random(20261029)

    for fname in dump_files:
        if fname not in adjudicated and fname not in fp_by_file:
            # A-02 扫描件 / A-19 双生 等：GT 未覆盖文件跳过（与原轮一致）
            if fname not in [fl['file'] for fl in gt['files']]:
                continue
        d = load_dump(fname)
        blocks = d['blocks']
        stats['files'] += 1
        seen_idx = set()   # 本文件已入 worklist 的块（去重）

        def add_item(idx, family, extra=None):
            if idx is not None:
                if idx in adjudicated.get(fname, set()):
                    stats['skip_adjudicated'] += 1
                    return
                b = blocks[idx]
                if (fname, norm(b['text'])[:40]) in false_texts:
                    stats['skip_adjudicated'] += 1
                    return
            if idx in seen_idx:
                return
            if idx is None:
                return
            seen_idx.add(idx)
            b = blocks[idx]
            item = {
                'file': fname, 'idx': b['idx'], 'kind': b['kind'],
                'locator': b.get('locator'), 'text': b['text'],
                'in_orig_pool': bool(b.get('candidate')),
                'family': family,
                'ctx': context(blocks, idx),
            }
            if b.get('cells'):
                item['cells'] = b['cells']
            if extra:
                item.update(extra)
            worklist.append(item)

        # ---------- S1 成组列举扫描 ----------
        i = 0
        n = len(blocks)
        while i < n:
            if not is_enum_item(blocks[i]['text']):
                i += 1
                continue
            # 找连续编号串
            run = [i]
            j = i + 1
            while j < n and (is_enum_item(blocks[j]['text']) or
                             (not blocks[j]['text'].strip())):  # 空块容忍
                if blocks[j]['text'].strip():
                    run.append(j)
                j += 1
                if len(run) > 30:
                    break
            # 回看 6 块找组头
            head = None
            for k in range(i - 1, max(-1, i - 7), -1):
                t = blocks[k]['text']
                if not t.strip():
                    continue
                if is_enum_item(t):
                    continue
                if is_group_head(t):
                    head = k
                break   # 最近一个非编号非空块即视为潜在组头位置
            if head is not None:
                if not blocks[head].get('candidate'):
                    add_item(head, 'S1_group_head', {'group_run': run})
                    stats['S1_group_heads'] += 1
                for r in run:
                    if not blocks[r].get('candidate'):
                        add_item(r, 'S1_group_child', {'group_head_idx': blocks[head]['idx'],
                                                       'head_text': blocks[head]['text'][:120]})
                        stats['S1_group_children'] += 1
            i = j if j > i else i + 1

        # ---------- S2 评审/审查表行（表格级） ----------
        for bi, b in enumerate(blocks):
            if b['kind'] != 'TableRow':
                continue
            loc = b.get('locator') or ''
            if not re.search(r'评审|审查|评标办法|符合性|初审', loc):
                continue
            t = norm(b['text'])
            if len(t) < 8:
                continue
            # 排除表头/占位/纯省略
            if re.match(r'^(条款号|评审因素|评审标准|序号|审查内容|审查要素|审查标准|评审内容)', t):
                continue
            if re.match(r'^[.…·、，,；;]+$', t):
                continue
            if not b.get('candidate'):
                add_item(bi, 'S2_eval_table_row')
                stats['S2_table_rows'] += 1

        # ---------- S3 FP 清单（带块定位+上下文） ----------
        for joined in fp_by_file.get(fname, []):
            nj = norm(joined)
            hit = None
            for bi, b in enumerate(blocks):
                nb = norm(b['text'])
                if nb and (nj in nb or nb in nj) and len(nb) >= 4:
                    hit = bi
                    break
            if hit is not None and hit in adjudicated.get(fname, set()):
                # 已裁决块：若为 false 判例则沿用（不重裁）；true 不会出现在 FP
                stats['S3_fp_already_false'] += 1
                continue
            if hit is None:
                # 双通道失败：作为无块 FP 记录（文本面裁决）
                worklist.append({'file': fname, 'idx': None, 'kind': 'FP-unmatched',
                                 'locator': None, 'text': joined,
                                 'in_orig_pool': False, 'family': 'S3_fp',
                                 'ctx': []})
                stats['S3_fp'] += 1
            else:
                if hit not in seen_idx:
                    add_item(hit, 'S3_fp')
                    stats['S3_fp'] += 1

        # ---------- S4 抽样审计（前三类之外的池外块） ----------
        outside = [bi for bi, b in enumerate(blocks)
                   if not b.get('candidate') and bi not in seen_idx
                   and bi not in adjudicated.get(fname, set())
                   and len(norm(b['text'])) >= 10]
        sample = rng.sample(outside, min(AUDIT_PER_FILE, len(outside)))
        for bi in sorted(sample):
            add_item(bi, 'S4_audit')
            stats['S4_audit'] += 1

    with open(OUT_PATH, 'w') as f:
        json.dump({'stats': stats, 'items': worklist}, f, ensure_ascii=False, indent=1)
    print('worklist →', OUT_PATH)
    print(json.dumps(stats, ensure_ascii=False))

if __name__ == '__main__':
    main()
