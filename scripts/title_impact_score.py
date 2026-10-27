#!/usr/bin/env python3
"""title_impact_score.py —— B2.5 标题影响面量化：GT 24 条召回 × V0/V1/V2 变体。

对齐逻辑与 scripts/real_eval_score.py 同源（同仓判分器复制，非语义复刻）：
norm_n2 归一 + GT 核心前 24 字符互含 + 12 字符滑窗（OCR/跨行容忍）。
本脚本只回答一个问题：标题识别缺失（V1）/编号污染（V2）使 GT 召回掉多少、
类别命中掉多少——引用支持率/页对齐面不变（锚文本不变），不在本脚本重复。

输入：artifacts/title-impact/experiment.json（V0/V1/V2 REQ dumps）
输出：artifacts/title-impact/gt-recall-by-variant.json（确定性）
"""
from __future__ import annotations

import json
import re
import unicodedata
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXP = REPO / "artifacts" / "title-impact" / "experiment.json"
GT_SRC = REPO / "artifacts" / "real-eval" / "ground-truth-draft.json"
OUT = REPO / "artifacts" / "title-impact" / "gt-recall-by-variant.json"
GT_MAIN = "A-某水利施工标段招标文件.pdf"


def norm_n2(s: str) -> str:
    nfkc = unicodedata.normalize("NFKC", s)
    return "".join(c for c in nfkc if c.isalnum())


def strip_leading_no(text: str) -> str:
    return re.sub(r"^[\d.、（）()\s]+", "", text).strip()


def gt_core(gt: dict) -> str:
    return norm_n2(strip_leading_no(gt["excerpt"]))


def hit(gt: dict, reqs: list[dict]) -> list[dict]:
    gtc = gt_core(gt)
    hits = []
    for r in reqs:
        if not r["anchors"]:
            continue
        rc = norm_n2(r["anchors"][0]["excerpt"])
        if not rc or len(rc) < 6:
            continue
        probe = gtc[: min(len(gtc), 24)]
        if probe and (probe in rc or (len(rc) >= 10 and rc[:24] in gtc)):
            hits.append(r)
        else:
            for i in range(0, max(1, len(rc) - 12)):
                if rc[i : i + 12] in gtc:
                    hits.append(r)
                    break
    return hits


def main() -> int:
    gts = json.loads(GT_SRC.read_text(encoding="utf-8"))["items"]
    exp = json.loads(EXP.read_text(encoding="utf-8"))
    variants = {}
    for tag in ("v0", "v1", "v2"):
        reqs = exp[f"{tag}_dump"]
        variants[tag] = [r for r in reqs if r["source_file"] == GT_MAIN]

    rows = []
    summary = {}
    for tag, reqs in variants.items():
        per_gt = []
        recalled = 0
        cat_match = 0
        for gt in gts:
            hs = hit(gt, reqs)
            r_hit = bool(hs)
            c_match = r_hit and hs[0]["category"] == gt["category"]
            recalled += r_hit
            cat_match += c_match
            per_gt.append({
                "gt_id": gt["gt_id"],
                "category": gt["category"],
                "recall_hit": r_hit,
                "matched_category": hs[0]["category"] if hs else None,
                "category_match": c_match,
                "clause_no": hs[0]["clause_no"] if hs else None,
            })
        rows.append({"variant": tag, "per_gt": per_gt})
        summary[tag] = {
            "reqs_04": len(reqs),
            "recall": f"{recalled}/{len(gts)}",
            "recall_pct": round(recalled / len(gts) * 100, 1),
            "category_match": f"{cat_match}/{len(gts)}",
            "category_match_pct": round(cat_match / len(gts) * 100, 1),
        }

    # 差分表：逐 GT 变体间翻转
    diff = []
    for i, gt in enumerate(gts):
        v0r, v1r, v2r = rows[0]["per_gt"][i], rows[1]["per_gt"][i], rows[2]["per_gt"][i]
        if v0r["recall_hit"] != v1r["recall_hit"] or v0r["category_match"] != v1r["category_match"] or v0r["clause_no"] != v2r["clause_no"]:
            diff.append({
                "gt_id": gt["gt_id"],
                "category": gt["category"],
                "v0": {"hit": v0r["recall_hit"], "cat": v0r["matched_category"], "clause": v0r["clause_no"]},
                "v1": {"hit": v1r["recall_hit"], "cat": v1r["matched_category"], "clause": v1r["clause_no"]},
                "v2": {"hit": v2r["recall_hit"], "cat": v2r["matched_category"], "clause": v2r["clause_no"]},
            })

    out = {
        "method": "对齐逻辑与 real_eval_score.py 同源（norm_n2 + 前24互含 + 12滑窗）；GT=A-某水利施工标段招标文件.pdf 24 条草案",
        "summary": summary,
        "changed_gts": diff,
    }
    OUT.write_text(json.dumps(out, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, indent=1))
    print(f"变化的 GT 条目: {len(diff)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
