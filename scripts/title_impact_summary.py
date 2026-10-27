#!/usr/bin/env python3
"""title_impact_summary —— issue #2 · PR #3 精简：experiment.json → summary.json

issue #2 评论 2026-10-27 03:28 要求：artifacts/ 下实验产物只保留**结论**，
不存过程数据（全量逐条 dump）。本脚本从本地再生产的 experiment.json
（约 7MB，三变体 REQ dump）提取关键指标摘要：

  - 语料分组与标题行统计（files_by_group / heading_lines_by_group）
  - 三变体 REQ 总数与类别分布（v0/v1/v2_by_category）
  - V0→V1 类别翻转计数 + 翻转样例（前 5 条，配对键与 harness 同源：
    (source_file, anchors[0].excerpt)——scripts/title_impact/src/main.rs pair()）
  - V2 条款号污染计数（v0_v2_* 字段原样承载）

确定性：sort_keys、无时间戳、无随机——同输入两跑字节一致。
用法：python3 scripts/title_impact_summary.py [experiment.json] [summary.json]
  默认: artifacts/title-impact/experiment.json → artifacts/title-impact/summary.json
"""

import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXP = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "artifacts/title-impact/experiment.json"
OUT = Path(sys.argv[2]) if len(sys.argv) > 2 else REPO / "artifacts/title-impact/summary.json"


def pair_key(req: dict) -> tuple:
    """与 harness pair() 同源：(source_file, anchors[0].excerpt)。"""
    anchors = req.get("anchors") or []
    excerpt = anchors[0].get("excerpt", "") if anchors else ""
    return (req.get("source_file", ""), excerpt)


def main() -> None:
    exp = json.loads(EXP.read_text(encoding="utf-8"))

    # ---- 翻转样例（V0→V1 类别变化，前 5 条；计数直接采信 harness 字段）
    # 口径与 harness pair() 逐条一致：v0 侧**不去重**（重复键各配一次），
    # v1 侧建 map 后同键以末条为准（HashMap 后写覆盖语义）。
    v1 = {pair_key(r): r for r in exp["v1_dump"]}
    flipped = [
        (a, v1[k])
        for a in exp["v0_dump"]
        if (k := pair_key(a)) in v1 and a.get("category") != v1[k].get("category")
    ]
    # 样例结构：harness flipped01=169 计数与这里推导集合一致性断言（防口径漂移）
    assert len(flipped) == exp["v0_v1_category_flipped"], (
        f"flip 计数漂移: 本地推导 {len(flipped)} != experiment.json "
        f"{exp['v0_v1_category_flipped']}——摘要脚本与 harness 配对口径不一致，停止"
    )
    flipped_sample = [
        {
            "id": a["id"],
            "source_file": a["source_file"],
            "category_v0": a["category"],
            "category_v1": b["category"],
            "excerpt": (a.get("anchors") or [{}])[0].get("excerpt", "")[:80],
        }
        for a, b in flipped[:5]
    ]

    summary = {
        "_note": (
            "experiment.json（三变体 REQ dump 全量，约 7MB）为本地再生产物，"
            "已按 issue #2 评论 2026-10-27 03:28 PR 精简要求移出入库（.gitignore）；"
            "本文件为其确定性摘要——结论保留、过程数据不存。"
        ),
        "method": (
            "scripts/title_impact/ Rust harness 真跑 bid_parse::extract_requirements，"
            "44 件 × V0/V1/V2 三变体，块流取自 baseline/parse 118 件基线；"
            "GT 判分 scripts/title_impact_score.py（对齐逻辑与 real_eval_score.py 同源）。"
        ),
        "reproduce": (
            "cd scripts/title_impact && cargo build --release && "
            "./target/release/title-impact <repo_root> artifacts/title-impact/experiment.json && "
            "python3 scripts/title_impact_summary.py"
        ),
        "corpus": {
            "loaded_files": exp["loaded_files"],
            "skipped_non_ok": exp["skipped_non_ok"],
            "files_by_group": exp["files_by_group"],
            "heading_lines_by_group": exp["heading_lines_by_group"],
        },
        "variants": {
            "v0_baseline": {"reqs_n": exp["v0_reqs_n"], "by_category": exp["v0_by_category"]},
            "v1_headings_stripped": {"reqs_n": exp["v1_reqs_n"], "by_category": exp["v1_by_category"]},
            "v2_a2_ordinal_pollution": {"reqs_n": exp["v2_reqs_n"], "by_category": exp["v2_by_category"]},
        },
        "impact": {
            "v0_v1_matched_pairs": exp["v0_v1_matched_pairs"],
            "v0_v1_category_flipped": exp["v0_v1_category_flipped"],
            "v1_reqs_lost_vs_v0": exp["v0_reqs_n"] - exp["v1_reqs_n"],
            "v0_v2_ordinal_matched": exp["v0_v2_ordinal_matched"],
            "v0_v2_clause_no_corrupted": exp["v0_v2_clause_no_corrupted"],
            "v0_v2_docx_matched": exp["v0_v2_docx_matched"],
            "v0_v2_docx_clause_corrupted": exp["v0_v2_docx_clause_corrupted"],
            "v0_v2_docx_corruption_pct": round(
                100.0 * exp["v0_v2_docx_clause_corrupted"] / exp["v0_v2_docx_matched"], 1
            ),
        },
        "flipped_category_sample": flipped_sample,
        "gt_recall_ref": "artifacts/title-impact/gt-recall-by-variant.json",
    }

    OUT.write_text(
        json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        f"summary → {OUT} ({OUT.stat().st_size} B); "
        f"flips={len(flipped)} sample={len(flipped_sample)} "
        f"v0={exp['v0_reqs_n']} v1={exp['v1_reqs_n']} v2={exp['v2_reqs_n']}"
    )


if __name__ == "__main__":
    main()
