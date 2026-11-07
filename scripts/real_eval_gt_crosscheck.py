#!/usr/bin/env python3
"""real_eval_gt_crosscheck.py — FG-18（#307）：GT 页码锚定的异源抽检。

背景（audit/false-green-register.md FG-18，L 级——半自证评测）：
  real_eval_gt_enrich.py 的 GT 锚定核验（anchor_verification）对的是
  **自家解析产物**的文本层（parse/*.pdf.json 的 blocks）——真值流向
  = 解析器输出 → GT，属半自证：anchor PASS 不构成对原文的独立验证。

本脚本做「原文抽检」的工具面半边：
  用**异源提取器** pypdf（非 poppler/pdftotext 链——与解析管线无共享代码）
  直接读原始 PDF，逐条核验 GT excerpt 的页码锚定：
    命中判据 = excerpt 归一核心串出现在 GT 自报页 ±1 的 pypdf 文本层内
    （±1 容忍两提取器的分页/合页差异；页码语义 = pypdf 1-based 物理页）。
  逐条落盘 hit/miss + 命中页 + 偏差；miss 不是失败，是**抽检发现**——
  连同原因假设（双列穿插/跨行拆断/转写漂移）如实登记，供人工验收复核。

诚实纪律：
  - 本抽检仍是**工具面交叉验证**（pypdf vs poppler），不是人工读原文——
    GT 冻结前的人工逐条复核义务不变（登记在报告头部，见 REPORT.md FG-18 节）；
  - 输出不改写 GT 任何字段；exit code 恒 0（诊断脚本，miss 是发现不是错误）。

用法：python3 scripts/real_eval_gt_crosscheck.py
输出：artifacts/real-eval/gt-crosscheck.json + 控制台摘要
"""
from __future__ import annotations

import json
import re
import unicodedata
from datetime import datetime, timezone
from pathlib import Path

from pypdf import PdfReader

REPO = Path(__file__).resolve().parent.parent
EVAL = REPO / "artifacts" / "real-eval"
GT_SRC = EVAL / "ground-truth-draft.json"
SAMPLES = REPO / "real-corpus" / "water" / "files"
# GT source_file = 04_某水利工程施工标段_施工招标文件.pdf（原研究机文件名）；
# 仓内同 sha 对应件 A-某水利施工标段招标文件.pdf（FG-16 映射，score 脚本同款回退）
PDF_CANDIDATES = [
    SAMPLES / "04_某水利工程施工标段_施工招标文件.pdf",
    SAMPLES / "A-某水利施工标段招标文件.pdf",
]

# 已知转写漂移条目的原因注记（与 gt_enrich 的 VERBATIM_FIXES/KNOWN_ISSUES 同源——
# miss 时给人工复核者的定位线索，不是为 miss 开脱）
MISS_NOTES = {
    "GT-R01": "前附表双列穿插（『水利水电/誉 工程施工总承包』）——pypdf 无 -layout 模式，双列阅读序与 poppler 不同属预期；人工核版式原文（双写法：叁级/三级）",
    "GT-T01": "文本层阅读序损坏（『10.8 其它』小节头穿插句中）——gt_enrich 已改取无损连续段；异源阅读序差异属预期，人工核 PDF 版式原文",
    "GT-R05": "前附表行被表格两列拆行——表格结构面已知差距；人工核版式原文",
    "GT-S03": "评分算法行被 layout 跨行拆断——跨行族；人工核断行拼接口径",
    "GT-S04": "同 S03 跨行拆断族——同上口径",
    "GT-S05": "同 S03 跨行拆断族——同上口径",
    "GT-V04": "跨行否决条款——关键词窗定位条目；人工核完整句",
}

# 2026-11-07 首轮抽检实证发现（GT-S05）——页锚错位 + 跨句转写双缺陷：
# 自报 p26 是章节标题页（『10.7 招标控制价下浮率 K→见投标人须知前附表』），
# 内容实际在 p39（poppler 解析产物 block#1100 locator=第39页 行6 自己就是证据）；
# 且摘录前缀为跨句拼接（原文『招标控制价的下浮率。K 值为…』，GT 拼『招标控制价
# 下浮率 K 值为…』）——24 字归一探针在两提取器里都不可能逐字命中，这解释了
# anchor_verification 为何只记 keyword(3/3) 而非 verbatim：半自证 PASS 掩盖了
# 页锚错位。人工验收改判候选：页锚 26→39 + 摘录取 p39 无损连续段。
CONCRETE_FINDINGS = {
    "GT-S05": (
        "抽检实证（2026-11-07）：自报 p26 为章节标题页，内容实在 p39"
        "（poppler 产物 block#1100 locator=第39页 行6）；且摘录前缀系跨句转写"
        "（原文『招标控制价的下浮率。K 值为…』）——keyword(3/3) 的 anchor PASS "
        "掩盖页锚错位+转写双缺陷。人工验收改判候选：页锚 26→39，摘录取 p39 无损连续段"
    ),
}


def norm_n2(s: str) -> str:
    nfkc = unicodedata.normalize("NFKC", s)
    return "".join(c for c in nfkc if c.isalnum())


def strip_leading_no(text: str) -> str:
    return re.sub(r"^[\d.、（）()\s]+", "", text).strip()


def resolve_pdf() -> Path:
    for p in PDF_CANDIDATES:
        if p.exists():
            return p
    raise SystemExit(f"FG-16：原始 PDF 不在仓内（尝试过 {[str(p) for p in PDF_CANDIDATES]}）")


def main() -> int:
    gt_doc = json.loads(GT_SRC.read_text(encoding="utf-8"))
    gts = gt_doc["items"]
    pdf = resolve_pdf()

    # 异源文本层：pypdf 逐页抽取（与 poppler/pdftotext 无共享代码）
    reader = PdfReader(str(pdf))
    pages_n2 = [norm_n2(pg.extract_text() or "") for pg in reader.pages]
    n_pages = len(pages_n2)

    results = []
    for gt in gts:
        core = norm_n2(strip_leading_no(gt["excerpt"]))
        # 归一核心的探针（与 score 脚本同长口径：前 24 归一字符）
        probe = core[: min(len(core), 24)] if core else ""
        claimed = int(gt["page"])
        window = pages_n2[max(0, claimed - 2): claimed + 1]  # 自报页 ±1（1-based → 切片右开）
        hit_page = None
        if probe:
            for off, ptext in enumerate(window):
                if probe in ptext:
                    hit_page = claimed - 1 + off  # 切片起点 = claimed-2 → 第 off 页实际页 = claimed-1+off
                    break
        # 次级探针：12 连续字符滑窗（跨行拆断容忍——与 score 同口径）
        slide_hit = None
        if hit_page is None and len(core) >= 12:
            for off, ptext in enumerate(window):
                for i in range(0, max(1, len(core) - 12)):
                    if core[i: i + 12] in ptext:
                        slide_hit = claimed - 1 + off
                        break
                if slide_hit is not None:
                    break
        # 全文兜底定位（页窗未命中时：条目在文档别处？——页锚漂移的独立证据）
        elsewhere = None
        if probe:
            for i, ptext in enumerate(pages_n2):
                if probe in ptext:
                    elsewhere = i + 1
                    break

        results.append({
            "gt_id": gt["gt_id"],
            "category": gt["category"],
            "claimed_page": claimed,
            "verbatim_hit": hit_page is not None,
            "hit_page": hit_page,
            "slide_hit_page": slide_hit,
            "found_elsewhere_page": elsewhere,
            "note": CONCRETE_FINDINGS.get(gt["gt_id"]) or MISS_NOTES.get(gt["gt_id"], ""),
        })

    verbatim_n = sum(1 for r in results if r["verbatim_hit"])
    slide_n = sum(1 for r in results if not r["verbatim_hit"] and r["slide_hit_page"])
    elsewhere_n = sum(1 for r in results if r["found_elsewhere_page"] and not r["verbatim_hit"])
    miss_all = [r["gt_id"] for r in results
                if not r["verbatim_hit"] and not r["slide_hit_page"]]

    out = {
        "eval": "FG-18-gt-crosscheck",
        "purpose": "GT 页码锚定异源抽检（pypdf vs 解析链 poppler）——半自证口径的工具面交叉验证",
        "generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "pdf": pdf.name,
        "pdf_pages_pypdf": n_pages,
        "extractor": f"pypdf（独立于解析管线——无共享代码）",
        "criterion": "excerpt 归一核心前 24 字符出现在自报页 ±1 的 pypdf 文本层；次级 12 字符滑窗（跨行容忍）；页窗未中另记全文定位",
        "limitations": (
            "工具面交叉验证 ≠ 人工读原文——pypdf 无 -layout 模式，双列前附表/"
            "跨行拆断条目的阅读序差异属预期（见逐条 note）；GT 冻结前的人工逐条"
            "复核义务不变（REPORT.md FG-18 节登记）"
        ),
        "gt_total": len(results),
        "verbatim_window_hit": verbatim_n,
        "slide_window_hit": slide_n,
        "window_miss_but_elsewhere": elsewhere_n,
        "window_miss_total": len(results) - verbatim_n - slide_n,
        "window_miss_ids": miss_all,
        "items": results,
    }
    (EVAL / "gt-crosscheck.json").write_text(
        json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    print(f"FG-18 GT 异源抽检（pypdf {n_pages} 页 × GT {len(results)} 条）")
    print(f"  页窗±1 逐字命中：{verbatim_n}/{len(results)}")
    print(f"  页窗±1 滑窗命中（跨行容忍）：+{slide_n} → 合计 {verbatim_n + slide_n}/{len(results)}")
    if miss_all:
        print(f"  页窗全 miss（含别处命中 {elsewhere_n}）：{miss_all}")
    for r in results:
        if not r["verbatim_hit"]:
            where = (f"滑窗命中 p{r['slide_hit_page']}" if r["slide_hit_page"]
                     else (f"页窗未中但全文见 p{r['found_elsewhere_page']}"
                           if r["found_elsewhere_page"] else "页窗与全文探针均未中"))
            print(f"  △ {r['gt_id']}（自报 p{r['claimed_page']}）：{where}")
            if r["note"]:
                print(f"      注：{r['note']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
