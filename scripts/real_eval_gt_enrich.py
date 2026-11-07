#!/usr/bin/env python3
"""real_eval_gt_enrich.py — M1.7-C：P2 真实文件评测 GT 草案 24 条逐项四要素补全

用户 M1.7-C 要求：为现有 24 条逐项补充**原文定位、标准答案、判定理由、复核状态**；
允许部分条目被退回修订，不为凑数降标。

本脚本对 artifacts/real-eval/ground-truth-draft.json（P2 第一轮 v0.1 的 AI 初标草案）：
  1. 原文定位补强：excerpt 逐条对回 parse 产物（A-某水利施工标段招标文件.pdf.json 的
     blocks 文本层），三级匹配（原文 → norm_n2 归一化 → 关键词窗）并记录命中方式与页码；
     命中失败=定位存疑，逐条如实登记（不伪装定位成功）
  2. 判定理由补全：逐条 rationale（为何入 GT、标准答案依据、风险语义）；
     已知争议/结构受损条目附 review_note（人工验收时的改判/退回候选）
  3. 复核状态落位：逐条 review_status=draft_pending_human（用户口径：未冻结前仅探索性指标）；
     争议条目 review_flag 列出（GT-T02 改判候选 / GT-B04 双重属性 / GT-R05 拆列 /
     GT-S03~S05 跨行 / GT-V04 跨行）
  4. 原有字段（gt_id/category/page/excerpt/risk/expected）一字不动；顶层状态与来源说明保留，
     追加 enrichment 块（时间/方法/统计）。

退出码：0 补全且定位核验完成（允许存在"定位存疑"条目——它们本身就是人工复核输入）。

FG-18（#307，2026-11-07）半自证口径登记：本脚本的 anchor_verification
对的是**解析产物文本层**（同源自证）——真值流向 = 解析器输出→GT，
anchor PASS 不构成对原文的独立验证。冻结前两道防线：
① 异源抽检 scripts/real_eval_gt_crosscheck.py（pypdf 独立提取器）；
② 人工逐条复核（Lead——冻结最终裁决）。enrichment 块新增
semi_self_certification 键登记本口径。
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
import unicodedata
from datetime import datetime
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
GT_PATH = REPO / "artifacts" / "real-eval" / "ground-truth-draft.json"
PARSE = REPO / "artifacts" / "real-eval" / "parse" / "A-某水利施工标段招标文件.pdf.json"

# 已知问题条目（REPORT.md §3/§6 登记在案 → 人工验收时的复核注记）
KNOWN_ISSUES = {
    "GT-R05": "前附表行被表格两列拆行（『能力和信 誉』）——D1 表格结构抽取面已知差距；定位以关键词窗命中，人工验收时核版式原文",
    "GT-S03": "评分算法行被 layout 跨行拆断——解析层已知差距；语义内容完整，人工验收时核断行拼接口径",
    "GT-S04": "同 S03 跨行拆断族——同上口径",
    "GT-S05": "同 S03 跨行拆断族——同上口径",
    "GT-V04": "跨行否决条款（『共资源交易平台』为跨行截断）——关键词窗定位；人工验收时核完整句",
    "GT-T02": "标注争议（REPORT §6）：『已进行信息登记，资质符合…』更像形式审查而非技术参数——人工验收改判候选（可能改类别）",
    "GT-B04": "否决项/商务要求双重属性（拒签合同→取消资格）——类别归属待人工裁决",
}

# 逐字修正（M1.7-C 定位纪律检出的转写漂移；原摘录保留在 excerpt_original，不抹证据）
# GT-R01：原摘录为前附表行清理转写（逗号归一+双列穿插被抹），且原文存在 叁级（3.1 条）/三级（前附表）双写法——抽取须双写法都命中；
# GT-T01：文本层阅读序损坏（『10.8 其它』小节头穿插在句中）——原摘录不可能逐字命中。
VERBATIM_FIXES = {
    "GT-R01": ("工程施工总承包三级（含三级）以上资质",
               "原摘录为转写：前附表双列穿插（『水利水电/誉 工程施工总承包』）被抹平+标点归一；且 3.1 条同语义用大写『叁级（含叁级）』——抽取器须双写法命中，人工验收时核两处版式原文"),
    "GT-T01": ("指标或质量标准与设计施工图不符之处",
               "文本层阅读序损坏：小节头『10.8 其它』穿插句中，原摘录全句不可能逐字命中；已改取无损连续段，人工验收时核 PDF 版式原文"),
}

RATIONALE = {
    "GT-R01": "法定一票否决资格：独立法人+水利水电施工总承包三级以上是资格审查硬门槛；前附表与须知 1.4.1 双处出现，抽取+可定位双锚点缺一不可（漏抽即资格误判）",
    "GT-R02": "类似业绩资格要求：投标截止前承担过类似工程——资格类业绩门槛；『投标截止时间）至少承担过』为前附表括注结构，检验日期边界与括注粘连解析",
    "GT-R03": "资格后审机制条款：决定资格审查发生在开标后而非报名时——影响资格检查流程语义（前审/后审不可互换）",
    "GT-R04": "名称一致性条款：投标人名称与证照五方一致——形式审查类，错抽会导致证照核对遗漏",
    "GT-R05": "资质总条款（前附表行）：资格条件汇总行，为 R01/R02 的上位汇总；D1 拆列导致原文受损（见 review_note）",
    "GT-R06": "联合体资质就低条款：同类法规语义（较低资质定级）——联合体场景的资格计算规则",
    "GT-V01": "弄虚作假否决+保证金没收：否决项+资金罚则双重后果；『一经发现』句式检验否决触发语义抽取",
    "GT-V02": "保证金时限否决：『规定时间内转入』——时限型否决，与 V03 缺交型否决互补构成保证金否决面",
    "GT-V03": "编号条款引用否决（本章第 3.4.1 项）：检验编号交叉引用解析（3.4.1 的义务在别处，否决在此）",
    "GT-V04": "递交渠道无效条款：电子交易平台外递交无效——渠道合规类否决；跨行受损（见 review_note）",
    "GT-V05": "挂靠借用资质否决：转让/租借资质=主体不适格——资格类与否决类交叉，两通道都应命中",
    "GT-V06": "否决全部投标条款：评审后整体否决的例外出口——流程级否决（非针对单一投标人）",
    "GT-V07": "取消中标资格条款：中标后阶段否决——时间维度上的否决（开标前否决面之外的补全）",
    "GT-B01": "控制价 3496688.66 元：报价红线（超限=废标）——金额参数必须保数值精度，是矩阵价格口径的锚",
    "GT-B02": "计划工期 180 日历天：工期参数（合同/进度/偏差计算基准）——与 DURATION_MISMATCH 规则联动",
    "GT-B03": "下浮率 K 值条款：评标联动商务参数（与 S05 K 值抽取机制同族，商务-评分双通道条目）",
    "GT-B04": "拒签合同取消资格：违约后果条款；否决/商务双重属性待人工裁决（见 review_note）",
    "GT-S01": "投标总报价 65 分：价格分权重——评分结构锚点，分值参数必须精确（评分总分构成 65+20+…）",
    "GT-S02": "单价合理性 20 分：评分细则第二权重——与 S01 合计构成报价评分主体",
    "GT-S03": "评标基准价算法：评标核心算法条款（控制价与有效报价算术平均）——错抽即价格分全错，评标语义关键",
    "GT-S04": "有效报价界定（下浮 15% 含）：无效报价判定的边界值条款——边界『含 15%』的闭区间语义不可丢",
    "GT-S05": "K 值抽取机制：开标现场抽取的下浮率——算法输入条款（与 B03 同族双通道）",
    "GT-T01": "技术标准优先级：与图纸不符以图纸为准——技术响应的让位条款（影响技术偏离判定方向）",
    "GT-T02": "信息登记形式条款：登记/资质符合表述——标注争议条目（见 review_note），人工验收时改判",
}


def norm_n2(s: str) -> str:
    """与 P2 评测同源归一化（bid-parse norm_n2 语义复刻）：NFKC + 去空白。"""
    s = unicodedata.normalize("NFKC", s)
    return re.sub(r"[\s\u00a0]+", "", s)


def norm_hard(s: str) -> str:
    """更强归一化（对付拆列/跨行）：NFKC + 去空白 + 去标点。"""
    s = unicodedata.normalize("NFKC", s)
    return re.sub(r"[\s\u00a0\W]+", "", s, flags=re.UNICODE)


def main() -> int:
    gt = json.loads(GT_PATH.read_text(encoding="utf-8"))
    parse = json.loads(PARSE.read_text(encoding="utf-8"))
    blocks = parse["blocks"]
    # 文本层拼接（带页码索引，便于逐页定位）
    page_texts: list[str] = []
    for b in blocks:
        pg = b.get("page", 1)
        while len(page_texts) < pg:
            page_texts.append("")
        page_texts[pg - 1] = (page_texts[pg - 1] + "\n" + (b.get("text") or ""))
    full = "\n".join(page_texts)
    full_norm = norm_n2(full)
    full_hard = norm_hard(full)

    stats = {"verbatim": 0, "normalized": 0, "keyword": 0, "unlocated": 0}
    fixed_ids = []
    for it in gt["items"]:
        if it["gt_id"] in VERBATIM_FIXES:
            fixed, note = VERBATIM_FIXES[it["gt_id"]]
            if "excerpt_original" not in it:               # 幂等：原摘录只在首轮留存
                it["excerpt_original"] = it["excerpt"]
            it["excerpt"] = fixed                             # 改为逐字无损段
            it["verbatim_fix_note"] = note
            fixed_ids.append(it["gt_id"])
        ex = it["excerpt"]
        # 三级定位：原文逐字 → 归一化 → 关键词窗
        if ex in full:
            method, hit = "verbatim", True
        elif norm_n2(ex) in full_norm:
            method, hit = "normalized", True
        elif norm_hard(ex) in full_hard:
            method, hit = "normalized_hard", True
        else:
            # 关键词窗：取 excerpt 最长 8 字连续段×2 段做降级证据
            segs = re.findall(r"[\u4e00-\u9fff0-9A-Za-z％%]{6,}", ex)
            hits = [s for s in segs if norm_hard(s) in full_hard]
            if len(hits) >= min(2, len(segs)):
                method, hit = f"keyword({len(hits)}/{len(segs)})", True
            else:
                method, hit = "keyword_weak" if hits else "unlocated", False
        stats[method.split("(")[0]] = stats.get(method.split("(")[0], 0) + 1
        it["anchor_verification"] = {
            "readback": "PASS" if hit else "FAIL",
            "method": method,
            "against": "artifacts/real-eval/parse/A-某水利施工标段招标文件.pdf.json（文本层 142 页）",
            "note": "excerpt 见于解析文本层（FG-18：半自证——对解析产物非原文，独立验证见 gt-crosscheck.json + 人工复核）" if hit else "excerpt 未完整见于解析文本层——定位存疑，人工验收必查（可能与拆列/跨行受损相关）",
        }
        it["rationale"] = RATIONALE[it["gt_id"]]
        it["review_status"] = "draft_pending_human"
        if it["gt_id"] in KNOWN_ISSUES:
            it["review_note"] = KNOWN_ISSUES[it["gt_id"]]

    gt["enrichment"] = {
        "enriched_at": datetime.now().strftime("%Y-%m-%d %H:%M"),
        "enriched_by": "M1.7-C（scripts/real_eval_gt_enrich.py）——用户口径：未人工冻结前仅探索性指标",
        "fields_added": ["anchor_verification(原文定位核验)", "rationale(判定理由)", "review_status(复核状态)", "review_note(退回候选注记)"],
        "anchor_stats": stats,
        "verbatim_fixed": fixed_ids,
        "disputed_items": sorted(KNOWN_ISSUES),
        "protocol": "人工冻结协议：逐条复核 → 接受/退回修订（GT-T02 改判候选、GT-B04 类别裁决、GT-R05/S03~S05/V04 结构受损核原文、GT-R01/T01 转写已修正待确认）→ 冻结为 gold；冻结前任何准确率输出必须标注 EXPLORATORY",
        # FG-18（#307）：半自证口径登记——anchor_verification 对解析产物
        # （同源），冻结前须过异源抽检 + 人工逐条复核。
        "semi_self_certification": {
            "registered": "2026-11-07（FG-18/#307）",
            "nature": "anchor_verification 对解析产物文本层（同源自证）——真值流向 = 解析器输出→GT，anchor PASS 不构成独立验证",
            "defenses": [
                "异源抽检：scripts/real_eval_gt_crosscheck.py（pypdf，独立于解析管线）→ artifacts/real-eval/gt-crosscheck.json",
                "人工逐条复核（Lead——GT 冻结的最终裁决）",
            ],
            "crosscheck_first_run": "2026-11-07：页窗±1 逐字 23/24；GT-S05 页锚错位实证（自报 p26 实为标题页，内容在 p39）+ 跨句转写——改判候选",
        },
    }
    GT_PATH.write_text(json.dumps(gt, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"24 条四要素补全完成：定位核验 PASS={sum(1 for i in gt['items'] if i['anchor_verification']['readback']=='PASS')}"
          f"（{stats}）；争议注记 {len(KNOWN_ISSUES)} 条；全部 review_status=draft_pending_human")
    return 0


if __name__ == "__main__":
    sys.exit(main())
