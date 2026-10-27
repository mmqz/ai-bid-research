#!/usr/bin/env python3
"""engine_diff —— issue #2 · T1：B3 双跑差分工具（v1）

输入两份 JSON（同 schema：{file, status, blocks: [{idx, kind, locator, text, cells}]}）：
  --old  baseline/parse/<file>.json        旧实现（bid_parse 现行）
  --new  <engine 侧适配 JSON>              B3 适配器输出（T1 阶段 = engine-probe-dump
                                          harness 产出：scripts/engine_probe/src/bin/dump.rs）

对齐：① 归一文本哈希全局配对（容忍块序位移）→ ② 前缀滑窗近配 → ③ 未配对单列。
归一：全角空白→半角、行首尾空白剥离、连续空白压一（列对齐差异吸收——P1 定谳）。

判定词典（docs/engine-bid-mapping.md §0，四分类 + 兜底）：
  EQUIVALENT   归一文本一致且 kind 兼容
  ENGINE_MISSING 引擎缺口（旧有新无）——规则：D6-INS（期望读回契约 inserted_runs 段）、
               D4-TEXTBOX（old components 文本框 present>0）、D5（批注面，同法）
  OLD_BUG      旧实现缺陷（旧多出/新多出）——规则：D6-DEL-LEAK（契约 deleted_runs 段）、
               HOST-TRUNC（宿主段截断方向）
  EXPECTED_DIFF 已知预期差异——规则：A2（编号叠加 regex）
  UNCLASSIFIED 兜底——报告显式列出，B3 验收要求清零（人工裁决）

D6 系文件自动加载同目录 expected-readback.json 契约（插入/删除段规则锚）。

用法：
  python3 scripts/engine_diff.py --old A.json --new B.json [--out R.json]
  python3 scripts/engine_diff.py --selftest          # 合成注入自测（run_tests T17 消费）
  python3 scripts/engine_diff.py --pair-dir D        # 批量：D 内 <stem>.json(旧) × <stem>.engine.json(新)
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# ---------------------------------------------------------------- 归一
_WS = re.compile(r"\s+")
FULLWIDTH_SPACE = "\u3000"


def norm(text: str) -> str:
    """列对齐空白吸收：全角空白折叠、首尾剥离、连续空白压一。"""
    return _WS.sub(" ", text.replace(FULLWIDTH_SPACE, " ")).strip()


# ---------------------------------------------------------------- A2 编号叠加探测
# 引擎读面 TR-2D 实测签名：「1.1」→「11.1」（自动编号与可见文本叠加）。
_A2 = re.compile(r"^(?P<left>\d+)\.(?P<right>\d+)")


def _a2_candidate(old_seg: str, new_seg: str) -> bool:
    """new 前缀 = old 前缀首段重复叠加（1.1→11.1 型）。"""
    mo, mn = _A2.match(old_seg.strip()), _A2.match(new_seg.strip())
    if not (mo and mn):
        return False
    lo, ro = mo.group("left"), mo.group("right")
    ln, rn = mn.group("left"), mn.group("right")
    return rn == ro and ln == lo + lo


# ---------------------------------------------------------------- 差分段分析
def _segments(old: str, new: str) -> tuple[str, str]:
    """公共前后缀剥离 → (old-only 段, new-only 段)。"""
    if old == new:
        return "", ""
    a, b = norm(old), norm(new)
    pre = 0
    while pre < len(a) and pre < len(b) and a[pre] == b[pre]:
        pre += 1
    suf = 0
    while (suf < len(a) - pre and suf < len(b) - pre
           and a[len(a) - 1 - suf] == b[len(b) - 1 - suf]):
        suf += 1
    return a[pre:len(a) - suf], b[pre:len(b) - suf]


# ---------------------------------------------------------------- 规则锚：D6 期望读回契约
def _load_contract(old: dict) -> dict:
    """R6*/D2a 同目录 expected-readback.json → {ins_texts, del_texts}。"""
    contract: dict[str, list[str]] = {"ins": [], "del": []}
    f = old.get("file", "")
    fname = Path(f).name if f else ""
    if "R6" not in fname:
        return contract
    er = REPO / "fixtures/corpus-R/D6-revisions/expected-readback.json"
    if not er.exists():
        return contract
    data = json.loads(er.read_text(encoding="utf-8"))
    var = data.get("variants", {}).get(fname)
    if not var:
        return contract
    contract["ins"] = [r["text"] for r in var.get("inserted_runs", [])]
    contract["del"] = [r["text"] for r in var.get("deleted_runs", [])]
    return contract


def _components_map(old: dict) -> dict[str, dict]:
    return {c.get("component", ""): c for c in old.get("components", [])}


# ---------------------------------------------------------------- 分类器
def classify_pair(old_blk: dict, new_blk: dict | None, contract: dict,
                  comps: dict[str, dict], new_texts: list[str] | None = None) -> dict:
    """对齐块对（或单边）判定。返回 verdict item（new_texts = 引擎侧全文本集，
    供旧侧孤块反查引擎是否已提取——修复前后差异定向用）。"""
    o_t = norm(old_blk.get("text", ""))
    n_t = norm(new_blk.get("text", "")) if new_blk else ""
    kind_o = old_blk.get("kind")
    kind_n = new_blk.get("kind") if new_blk else None
    item = {
        "old_locator": old_blk.get("locator", ""),
        "new_locator": (new_blk or {}).get("locator", ""),
        "kind_old": kind_o,
        "kind_new": kind_n,
        "text_old": o_t,
        "text_new": n_t,
    }
    if o_t == n_t and kind_o == kind_n:
        item.update(verdict="EQUIVALENT", rule="")
        return item
    o_only, n_only = _segments(o_t, n_t)
    # —— D6-INS：插入段在旧在、在新缺（孤块时 n_t="" 天然满足；修复后两侧均在则不触发）
    if (o_only and any(t in o_t and t not in n_t for t in contract["ins"])
            and not n_only):
        item.update(verdict="ENGINE_MISSING", rule="D6-INS")
        return item
    # —— D6-DEL-LEAK（仅配对块）：删除历史段在旧在、在新缺（bid 粘性泄漏）。
    #    锚一：契约 del 文本；锚二（无契约）：旧件部件账本「修订删除」present>0。
    #    孤块不进此（防文本框重复块等误吸收）。
    if new_blk is not None:
        del_hit = any(norm(t) in o_t and norm(t) not in n_t for t in contract["del"])
        if (not del_hit and o_only and not n_only
                and comps.get("修订删除", {}).get("present", 0) > 0):
            del_hit = True
        if o_only and del_hit and not n_only:
            item.update(verdict="OLD_BUG", rule="D6-DEL-LEAK")
            return item
        # —— A2 编号叠加（new 多出纯数字前缀段，剥离后与 old 全等——「1.1→11.1」签名）
        if n_only and not o_only and n_only.strip().isdigit() and o_t == n_t.replace(n_only.strip(), "", 1):
            item.update(verdict="EXPECTED_DIFF", rule="A2")
            return item
    # —— D4 文本框面（旧孤块）：引擎侧已提取该文本 → 旧侧重复块（bid 双重提取缺陷，
    #    mdcore#18 勘误发现）；未提取 → 引擎缺口；无部件关联 → 待裁
    if not new_blk:
        present_in_new = new_texts is not None and o_t in new_texts
        if present_in_new:
            item.update(verdict="OLD_BUG", rule="D4-DOUBLE-EXTRACT（同文本重复入流——mdcore#18 勘误）")
        elif comps.get("文本框", {}).get("present", 0) > 0:
            item.update(verdict="ENGINE_MISSING", rule="D4-TEXTBOX")
        else:
            item.update(verdict="UNCLASSIFIED", rule="old-only（待人工裁决）")
        return item
    # —— 通用方向兜底（无规则命中）：
    if o_only and not n_only:
        item.update(verdict="UNCLASSIFIED", rule="")
        return item
    if n_only and not o_only:
        item.update(verdict="UNCLASSIFIED", rule="new-extra（候选 OLD_BUG 面，待人工裁决）")
        return item
    item.update(verdict="UNCLASSIFIED", rule="bidirectional")
    return item


# ---------------------------------------------------------------- 对齐
def _near_score(a: str, b: str) -> int:
    """公共前后缀总长（近配对分——A2 型「1.1→11.1」哈希不中时的第二道配对）。"""
    pre = 0
    while pre < len(a) and pre < len(b) and a[pre] == b[pre]:
        pre += 1
    suf = 0
    while (suf < len(a) - pre and suf < len(b) - pre
           and a[len(a) - 1 - suf] == b[len(b) - 1 - suf]):
        suf += 1
    return pre + suf


def align(old_blocks: list[dict], new_blocks: list[dict]):
    """① 归一哈希全局配对 ② 近配对（公共前后缀≥4 且≥半长） ③ 余项按序单列。"""
    n_by_text: dict[str, list[int]] = {}
    for i, b in enumerate(new_blocks):
        n_by_text.setdefault(norm(b.get("text", "")), []).append(i)
    used: set[int] = set()
    pairs: list[tuple[dict, dict | None]] = []
    unmatched_old: list[dict] = []
    for ob in old_blocks:
        key = norm(ob.get("text", ""))
        idxs = [i for i in n_by_text.get(key, []) if i not in used]
        if idxs:
            used.add(idxs[0])
            pairs.append((ob, new_blocks[idxs[0]]))
        else:
            unmatched_old.append(ob)
    # ② 近配对：分数 = 公共前后缀总长，阈值 max(4, 短文本长//2)（容纳包含型差异——
    #    如「澄清四：…」是旧块「澄清四：…+ins+del」的前缀：以长侧算阈值会拒配）
    new_free = [i for i in range(len(new_blocks)) if i not in used]
    for ob in unmatched_old:
        o_t = norm(ob.get("text", ""))
        best, best_score = None, 0
        for i in new_free:
            if i in used:
                continue
            n_t = norm(new_blocks[i].get("text", ""))
            score = _near_score(o_t, n_t)
            if score > best_score and score >= max(4, min(len(o_t), len(n_t)) // 2):
                best, best_score = i, score
        if best is not None:
            used.add(best)
            pairs.append((ob, new_blocks[best]))
        else:
            pairs.append((ob, None))
    rest = [(None, nb) for i, nb in enumerate(new_blocks) if i not in used]
    return pairs, rest


# ---------------------------------------------------------------- 单文件差分
def diff_file(old: dict, new: dict) -> dict:
    old_blocks = old.get("blocks", [])
    new_blocks = new.get("blocks", [])
    comps = _components_map(old)
    contract = _load_contract(old)
    new_texts = [norm(b.get("text", "")) for b in new_blocks]
    pairs, rest = align(old_blocks, new_blocks)
    items = []
    for ob, nb in pairs:
        if nb is None:
            # 旧块无配对 → 走单边分类（文本框重复/缺口/未配对面）
            items.append(classify_pair(ob, None, contract, comps, new_texts))
        else:
            items.append(classify_pair(ob, nb, contract, comps, new_texts))
    for _ob, nb in rest:
        n_t = norm(nb.get("text", ""))
        if not n_t:
            # 引擎保留空段（del-only/占位）而旧实现丢弃——结构性差异：适配层按映射表
            # §1 规则丢空段（块流稳定性），故归已知预期差异
            items.append({
                "old_locator": "", "new_locator": nb.get("locator", ""),
                "kind_old": None, "kind_new": nb.get("kind"),
                "text_old": "", "text_new": "",
                "verdict": "EXPECTED_DIFF", "rule": "EMPTY-PARA（引擎空段；适配层丢空段对齐旧口径）",
            })
            continue
        items.append({
            "old_locator": "", "new_locator": nb.get("locator", ""),
            "kind_old": None, "kind_new": nb.get("kind"),
            "text_old": "", "text_new": n_t,
            "verdict": "UNCLASSIFIED", "rule": "new-only（引擎多块，候选 OLD_BUG 面，待人工裁决）",
        })
    counts: dict[str, int] = {}
    for it in items:
        counts[it["verdict"]] = counts.get(it["verdict"], 0) + 1
    return {
        "file": old.get("file", ""),
        "old_status": old.get("status"),
        "new_status": new.get("status"),
        "old_blocks": len(old_blocks),
        "new_blocks": len(new_blocks),
        "aligned_pairs": len(pairs),
        "verdict_counts": counts,
        "items": items,
    }


# ---------------------------------------------------------------- 自测（合成注入）
def selftest() -> list[str]:
    """四分类 + 兜底各至少 1 例；返回失败清单（空 = 全过）。"""
    failures = []

    def blk(i, kind, text, loc="L"):
        return {"idx": i, "kind": kind, "locator": loc, "text": text, "cells": []}

    base_old = {"file": "R6X-自测.docx", "status": "ok", "blocks": [], "components": []}

    # 1) EQUIVALENT（全角空白/多空格吸收）
    old = {**base_old, "blocks": [blk(0, "para", "澄清一：正常 条款。")]}
    new = {"file": "R6X-自测.docx", "status": "ok",
           "blocks": [blk(0, "para", "澄清一：正常\u3000 条款。")]}
    r = diff_file(old, new)
    if r["verdict_counts"].get("EQUIVALENT") != 1:
        failures.append(f"EQUIVALENT: {r['verdict_counts']}")

    # 2) ENGINE_MISSING / D6-INS
    old = {**base_old, "blocks": [blk(0, "para", "开标地点：示例大厦三层第二开标室（插入后生效）")]}
    new = {"file": "R6X-自测.docx", "status": "ok", "blocks": [blk(0, "para", "开标地点：")]}
    er = json.loads((REPO / "fixtures/corpus-R/D6-revisions/expected-readback.json").read_text("utf-8"))
    var = er["variants"]["R6a-纯插入澄清.docx"]
    contract = {"ins": [r_["text"] for r_ in var["inserted_runs"]], "del": []}
    # 借 R6a 契约：file 名对齐
    old["file"] = new["file"] = "R6a-纯插入澄清.docx"
    r = diff_file(old, new)
    if r["verdict_counts"].get("ENGINE_MISSING") != 1:
        failures.append(f"ENGINE_MISSING(D6-INS): {r['verdict_counts']}")

    # 3) OLD_BUG / D6-DEL-LEAK
    old = {**base_old, "file": "R6b-纯删除澄清.docx",
           "blocks": [blk(0, "para", "原资格条款：具备建筑工程施工总承包三级及以上资质")]}
    new = {"file": "R6b-纯删除澄清.docx", "status": "ok",
           "blocks": [blk(0, "para", "原资格条款：")]}
    r = diff_file(old, new)
    if r["verdict_counts"].get("OLD_BUG") != 1:
        failures.append(f"OLD_BUG(D6-DEL-LEAK): {r['verdict_counts']}")

    # 4) EXPECTED_DIFF / A2
    old = {**base_old, "file": "word-complex.docx",
           "blocks": [blk(0, "para", "1.1 表格说明段落")]}
    new = {"file": "word-complex.docx", "status": "ok",
           "blocks": [blk(0, "para", "11.1 表格说明段落")]}
    r = diff_file(old, new)
    if r["verdict_counts"].get("EXPECTED_DIFF") != 1:
        failures.append(f"EXPECTED_DIFF(A2): {r['verdict_counts']}")

    # 5) UNCLASSIFIED 兜底（无规则命中的真实差异：旧孤 + 新孤各一）
    old = {**base_old, "file": "自测无规则.docx", "blocks": [blk(0, "para", "甲乙丙丁")],
           "components": []}
    new = {"file": "自测无规则.docx", "status": "ok", "blocks": [blk(0, "para", "子丑寅卯")]}
    r = diff_file(old, new)
    if r["verdict_counts"].get("UNCLASSIFIED") != 2:
        failures.append(f"UNCLASSIFIED: {r['verdict_counts']}（期望旧孤+新孤各一）")

    return failures


# ---------------------------------------------------------------- CLI
def main() -> int:
    args = sys.argv[1:]
    if "--selftest" in args:
        fails = selftest()
        if fails:
            print("自测失败:")
            for f in fails:
                print(f"  ✗ {f}")
            return 1
        print("engine_diff 自测通过：EQUIVALENT/ENGINE_MISSING(D6-INS)/OLD_BUG(D6-DEL-LEAK)/"
              "EXPECTED_DIFF(A2)/UNCLASSIFIED 五例全命中")
        return 0

    if "--pair-dir" in args:
        d = Path(args[args.index("--pair-dir") + 1])
        out_dir = Path(args[args.index("--out") + 1]) if "--out" in args else REPO / "artifacts/engine-diff"
        out_dir.mkdir(parents=True, exist_ok=True)
        summary = []
        for old_path in sorted(d.glob("*.json")):
            stem = old_path.stem
            new_path = d / f"{stem}.engine.json"
            if not new_path.exists():
                continue
            rep = diff_file(json.loads(old_path.read_text("utf-8")),
                            json.loads(new_path.read_text("utf-8")))
            out = out_dir / f"{stem}.diff.json"
            out.write_text(json.dumps(rep, ensure_ascii=False, indent=2, sort_keys=True) + "\n", "utf-8")
            summary.append({"file": rep["file"], "old_blocks": rep["old_blocks"],
                            "new_blocks": rep["new_blocks"], "verdict_counts": rep["verdict_counts"]})
        (out_dir / "summary.json").write_text(
            json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n", "utf-8")
        print(f"{len(summary)} 对差分完成 → {out_dir}")
        return 0

    if "--old" not in args or "--new" not in args:
        print(__doc__)
        return 2
    old = json.loads(Path(args[args.index("--old") + 1]).read_text("utf-8"))
    new = json.loads(Path(args[args.index("--new") + 1]).read_text("utf-8"))
    rep = diff_file(old, new)
    print(json.dumps({k: v for k, v in rep.items() if k != "items"}, ensure_ascii=False, indent=2))
    if "--out" in args:
        Path(args[args.index("--out") + 1]).write_text(
            json.dumps(rep, ensure_ascii=False, indent=2, sort_keys=True) + "\n", "utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
