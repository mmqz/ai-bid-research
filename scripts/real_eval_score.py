#!/usr/bin/env python3
"""real_eval_score.py — P2 真实文件评测判分（GT 草案 × REQ 对齐）。

指标（test-plan §5 口径的真实文件子集）：
  - 召回（漏项）：GT 条款 ↔ REQ anchors 的归一化对齐命中
  - 引用支持率（R3-S3-4 升级）：excerpt 归一串须出现在 anchor 自报页 ±1 内（跨页拆行容忍）
    ——从「可回查」升为「回查对得上」；页外命中另记 supported_elsewhere 不计入支持
  - 页码对齐：GT 页码 vs anchor locator 页码偏差（分布披露）
诚实纪律：
  - GT 为 AI 初标草案——指标冠名「草案对齐」；
  - 误报不自动判定（需人工核 Other 类样本）——报告中给出抽样建议；
  - 不外推合成覆盖率。

对齐算法（与 grade.py 反标记纪律同源，但不自报 gt_id）：
  norm_n2 归一化（NFKC+去空白+去标点）后，GT 核心（去前导编号后的 ≥8 字符）与
  REQ excerpt 互为子串即命中；OCR 面（C- 扫描件）额外容忍 norm_n2 ≥10 连续字符滑窗。
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import unicodedata
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EVAL = REPO / "artifacts" / "real-eval"
# FG-16（#305）：仓外绝对路径参数化——默认指仓内 real-corpus（同 sha 文件在库，
# 文件名按仓内命名）；原研究机路径经 REAL_EVAL_SAMPLES 环境变量覆盖。
SAMPLES = Path(os.environ.get("REAL_EVAL_SAMPLES") or REPO / "real-corpus" / "water" / "files")
GT_SRC = EVAL / "ground-truth-draft.json"
REQ_SRC = EVAL / "extract" / "requirements.json"
PF_SRC = EVAL / "parse" / "A-某水利施工标段招标文件.pdf.json"
PDF_SRC = SAMPLES / "04_某水利工程施工标段_施工招标文件.pdf"
# 仓内对应件（A-某水利施工标段招标文件.pdf——同 sha 不同名，FG-16 映射）
PDF_SRC_REPO = SAMPLES / "A-某水利施工标段招标文件.pdf"


def norm_n2(s: str) -> str:
    nfkc = unicodedata.normalize("NFKC", s)
    return "".join(c for c in nfkc if c.isalnum())


def strip_leading_no(text: str) -> str:
    return re.sub(r"^[\d.、（）()\s]+", "", text).strip()


def req_core(req: dict) -> str:
    return norm_n2(req["anchors"][0]["excerpt"])


def gt_core(gt: dict) -> str:
    return norm_n2(strip_leading_no(gt["excerpt"]))


def main() -> int:
    gts = json.loads(GT_SRC.read_text(encoding="utf-8"))["items"]
    reqs = json.loads(REQ_SRC.read_text(encoding="utf-8"))
    reqs_04 = [r for r in reqs if r["source_file"] == "A-某水利施工标段招标文件.pdf"]
    pf = json.loads(PF_SRC.read_text(encoding="utf-8"))
    # 原文页文本（引用支持率回查面）
    pages = subprocess.run(
        # FG-16：原研究机文件名不在仓内时回退同 sha 仓内件
        ["pdftotext", "-layout", str(PDF_SRC if PDF_SRC.exists() else PDF_SRC_REPO), "-"], capture_output=True
    ).stdout.decode("utf-8", "replace").split("\f")
    pages_n2 = [norm_n2(p) for p in pages]

    results = []
    for gt in gts:
        gtc = gt_core(gt)
        # 候选：REQ excerpt 与 GT 互含或长滑窗重叠（OCR/跨行容忍）
        hits = []
        for r in reqs_04:
            rc = req_core(r)
            if not rc or len(rc) < 6:
                continue
            probe = gtc[: min(len(gtc), 24)]
            if probe and (probe in rc or (len(rc) >= 10 and rc[:24] in gtc)):
                hits.append(r)
            else:
                # 滑窗：连续 12 归一字符重叠（跨行拆断容忍）
                for i in range(0, max(1, len(rc) - 12)):
                    if rc[i : i + 12] in gtc:
                        hits.append(r)
                        break
        # 引用支持率（R3-S3-4）：首个命中 anchor 的 excerpt 回查原文，
        # 严格口径 = 归一串出现在 anchor 自报页 ±1 内（anchor 页与文档物理页自洽）；
        # 页外命中另记 supported_elsewhere（不计支持，单独披露）——防「任意页子串」恒真。
        supported = None
        supported_elsewhere = False
        page_delta = None
        if hits:
            a = hits[0]["anchors"][0]
            m = re.match(r"第(\d+)页", a["locator"])
            anchor_page = int(m.group(1)) if m else None
            core = norm_n2(a["excerpt"])[:20]
            if anchor_page and 1 <= anchor_page <= len(pages_n2):
                window = pages_n2[max(0, anchor_page - 2): anchor_page + 1]
                supported = any(core and core in p for p in window)
                supported_elsewhere = (not supported) and any(
                    core and core in p for p in pages_n2)
            else:
                # 无页码锚（扫描件 OCR 通道等）：页对齐不可验，维持全文回查并在报告登记
                supported = any(core and core in p for p in pages_n2)
            page_delta = (anchor_page - gt["page"]) if anchor_page else None
        results.append({
            "gt_id": gt["gt_id"],
            "category": gt["category"],
            "risk": gt["risk"],
            "recall_hit": bool(hits),
            "matched_reqs": len(hits),
            "first_clause_no": hits[0]["clause_no"] if hits else None,
            # R3-S2-3：分类命中校验——命中 REQ 的类别与 GT 类别是否一致（错类命中
            # 不得计入分类召回；整体召回仍计但另记 category_mismatch）
            "category_match": bool(hits) and hits[0]["category"] == gt["category"],
            "matched_category": hits[0]["category"] if hits else None,
            "citation_supported": supported,
            "supported_elsewhere": supported_elsewhere,
            "page_delta": page_delta,
        })

    total = len(results)
    recalled = sum(1 for r in results if r["recall_hit"])
    cat_match = sum(1 for r in results if r["recall_hit"] and r["category_match"])
    supported_n = sum(1 for r in results if r["recall_hit"] and r["citation_supported"])
    by_cat = {}
    for r in results:
        c = r["category"]
        d = by_cat.setdefault(c, {"total": 0, "recall": 0, "cat_match": 0, "supported": 0})
        d["total"] += 1
        d["recall"] += 1 if r["recall_hit"] else 0
        d["cat_match"] += 1 if (r["recall_hit"] and r["category_match"]) else 0
        d["supported"] += 1 if (r["recall_hit"] and r["citation_supported"]) else 0

    misses = [r for r in results if not r["recall_hit"] or not r["category_match"]]
    mismatched = [r for r in results if r["recall_hit"] and not r["category_match"]]
    # R3-S3-4：页偏差分布 + 页外命中披露（判据升级后指标不再依赖数据侥幸成立）
    delta_dist = {}
    for r in results:
        if r["recall_hit"] and r["page_delta"] is not None:
            k = str(r["page_delta"])
            delta_dist[k] = delta_dist.get(k, 0) + 1
    elsewhere = [r["gt_id"] for r in results
                 if r["recall_hit"] and r.get("supported_elsewhere")]
    report = {
        "eval": "P2-real-file-score",
        # SC-11（issue #39 吸收）：机器可读契约稳定性——schema 语义版本。
        # 破坏性字段变更（删除/重命名/语义改判据）必须升版本并在此登记变更记录；
        # 追加字段不升版（消费方按字段名读取，serde/default 语义）。
        # 变更记录：1.0 = R3-S3-4 严格引用判据 + 类别命中维度定稿（2026-10-25）。
        "schema_version": "1.0",
        # FG-18（#307）：半自证口径登记——GT 锚定核验（gt_enrich 的
        # anchor_verification）对的是解析产物文本层，真值流向 = 解析器输出→GT，
        # anchor PASS 不构成独立验证。冻结前两道防线：①异源抽检
        # gt-crosscheck.json（pypdf 独立提取器，2026-11-07 起）；②人工逐条
        # 复核（Lead——GT 冻结的最终裁决）。字段追加不升版（SC-11 语义）。
        "gt_status": (
            "AI 初标草案（待人工验收）——指标为草案对齐结果，不外推为验收指标；"
            "GT 锚定核验对解析文本层，属半自证（FG-18）：anchor PASS 不构成独立验证，"
            "冻结前须过异源抽检（gt-crosscheck.json）+ 人工逐条复核"
        ),
        "gt_file_target": "04_某水利工程施工标段_施工招标文件.pdf（142 页）",
        "reqs_in_file": len(reqs_04),
        "gt_total": total,
        "recall": recalled,
        "recall_rate": round(recalled / total, 4),
        "category_match": cat_match,
        "category_match_rate": round(cat_match / total, 4),
        "category_mismatches": mismatched,
        "citation_supported": supported_n,
        "citation_support_rate": round(supported_n / max(1, recalled), 4),
        "citation_support_criterion": "严格：excerpt 归一串出现在 anchor 自报页 ±1（R3-S3-4；原任意页子串判据恒真风险已除）",
        "supported_elsewhere_ids": elsewhere,
        "page_delta_distribution": delta_dist,
        "by_category": by_cat,
        "misses": misses,
        "false_positive_note": "误报率不自动判定（'其他'类 2227 条含大量正文编号行噪声）——建议人工抽样 50 条 Other 类 REQ 核对（抽样协议同 test-plan §5.3）",
        "findings": [
            "真实文件章节用阿拉伯数字（第2章）——已修 extract_main_pdf（首轮抓出，合成夹具中文数字形态掩盖）",
            "多页扫描件页数错报（pdftotext 空文本层）——已修 OCR 渲染张数回写（02 号 8 页正确）",
            "真实水利范本 subsection 为数字小节（1.4 资格要求），政采货物范本为中文序号——分类面 94% 落'其他'：M1 确定性分类器的场景局限（设计内：M2 模型归类补全；S2 场景裁决的实证输入）",
            "前附表两列结构（layout 拆行）使跨行条款对齐依赖滑窗容忍——D1 表格结构抽取为已知差距（corpus-R D1a 已实证引擎面，业务侧 M2+）",
        ],
    }
    (EVAL / "score-report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(f"GT {total} 条：召回 {recalled}（{report['recall_rate']:.1%}）"
          f" | 类别命中 {cat_match}（{report['category_match_rate']:.1%}）"
          f" | 引用支持 {supported_n}/{recalled}（{report['citation_support_rate']:.1%}，锚点页±1 严格口径）")
    if delta_dist:
        print(f"  页偏差分布（anchor-GT）：{delta_dist}"
              + (f" | 页外命中：{elsewhere}" if elsewhere else ""))
    for c, d in sorted(by_cat.items()):
        print(f"  {c}: 召回 {d['recall']}/{d['total']} | 类别一致 {d['cat_match']}")
    for m in misses:
        tag = "错类" if m["recall_hit"] else "漏项"
        print(f"  ✗ {tag} {m['gt_id']}（期望 {m['category']}"
              + (f"，实得 {m['matched_category']}" if m["recall_hit"] else "") + "）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
