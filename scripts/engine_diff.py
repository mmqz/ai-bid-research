#!/usr/bin/env python3
"""engine_diff —— issue #2 · T1/B3-3：B3 双跑差分工具（v2）

输入两份 JSON（同 schema：{file, status, blocks: [{idx, kind, locator, text, cells}]}）：
  --old  baseline/parse/<file>.json        旧实现（bid_parse 现行）
  --new  <engine 侧适配 JSON>              B3 适配器输出（T1 阶段 = engine-probe-dump
                                          harness 产出：scripts/engine_probe/src/bin/dump.rs）

对齐：① canon 哈希全局配对（容忍块序位移 + 域码/CJK 边界空白差异）→ ② 前缀滑窗近配 → ③ 未配对单列。
canon = norm + 域指令码剥离：旧实现把 w:instrText（`PAGEREF _TocN \\hN` /
  `HYPERLINK "url"` / `TOC \\o ".." \\h \\u`）拼进正文文本；引擎 A2 族（338acdc）已剥离。
  剥离后同键 → 带码旧块与洁净新块直接配对（消解 TOC 条目/真实标题交叉错配），
  内容等价而旧侧带码 → OLD_BUG/FIELD-CODE-LEAK。

归一：全角空白→半角、行首尾空白剥离、连续空白压一（列对齐差异吸收——P1 定谳）；
  v2：CJK 边界空白吸收（任一侧为 CJK 字符即删中间空格——引擎 run 连接在 CJK
  邻接处插空格 vs 旧实现直接拼接；纯 ASCII 词间空格不受影响，「ISO 9001」保持）。

判定词典（docs/engine-bid-mapping.md §0，四分类 + 兜底）：
  EQUIVALENT   归一文本一致且 kind 兼容
  ENGINE_MISSING 引擎缺口（旧有新无）——规则：D6-INS（期望读回契约 inserted_runs 段）、
               D4-TEXTBOX（old components 文本框 present>0）、D5（批注面，同法）
  OLD_BUG      旧实现缺陷（旧多出/新多出）——规则：D6-DEL-LEAK（契约 deleted_runs 段）、
               HOST-TRUNC（宿主段截断方向）、
               FIELD-CODE-LEAK（v2：旧侧域指令码泄漏——PAGEREF/HYPERLINK/TOC，含孤块与残留面）、
               OLD-TABLE-MISS（v2：旧侧表格区拆散为散落 para，引擎整行 table_row；
                 v5 孤块对偶面：旧孤 para 空白不敏感包含于引擎整行）、
               DUP-EMISSION（v5：旧孤 table_row 同 locator 多次发射余量——
                 嵌套表区域的旧侧累积重复发射，基线变体已配对）
  EXPECTED_DIFF 已知预期差异——规则：A2（编号叠加 regex）；
               HF-NEWCAP（PR #15 review：新侧页眉/页脚 source 标注块——引擎新能力，
                 旧实现未读页眉页脚，非缺陷）；
               DUP-RESIDUE-WS（v5：旧孤 para 空白不敏感在册——同内容空白形态差异，
                 引擎侧已有同文本，旧侧多重发射余量）
  UNCLASSIFIED 兜底——报告显式列出，B3 验收要求清零（人工裁决）。
               含 HF-MIXED-BODY（PR #15 review：正文流（source=body）出现页眉/页脚
               文本——引擎或适配层把页面装置混进内容流，bug 面要报，验收前清零）

D6 系文件自动加载同目录 expected-readback.json 契约（插入/删除段规则锚）。

用法：
  python3 scripts/engine_diff.py --old A.json --new B.json [--out R.json]
  python3 scripts/engine_diff.py --selftest          # 合成注入自测（run_tests T17 消费）
  python3 scripts/engine_diff.py --pair-dir D        # 批量：D 内 <stem>.json(旧) × <stem>.engine.json(新)
"""

from __future__ import annotations

import collections
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# v5：空白不敏感键（孤块证据面专用——判定主链仍用 norm/canon 压一空格口径，
# 本键只做「同内容不同空白形态」的在册反查，不参与 EQUIVALENT 判定）
def _ws(t: str) -> str:
    return re.sub(r"\s+", "", t or "")

# ---------------------------------------------------------------- 归一
_WS = re.compile(r"\s+")
FULLWIDTH_SPACE = "\u3000"
# v2：CJK 字符类内容（汉字 + CJK 标点 + 全角形式 + 通用标点引号/破折/省略）——边界空白吸收的判定集
_CJK_CLS = r"\u4e00-\u9fff\u3000-\u303f\uff00-\uffef\u2010-\u2026"
_C2C = re.compile(f"([{_CJK_CLS}]) ([{_CJK_CLS}])")              # CJK—CJK：必吸收
_C2A = re.compile(f"([{_CJK_CLS}]) ([^\\s{_CJK_CLS}])")          # CJK—ASCII：吸收（「2026 年」→「2026年」）
_A2C = re.compile(f"([^\\s{_CJK_CLS}]) ([{_CJK_CLS}])")          # ASCII—CJK：吸收（两侧一致应用）
_BR_OPEN = re.compile(r"\[ +")                                   # 括号开侧内边空白
_BR_CLOSE = re.compile(r" +\]")                                  # 括号闭侧内边空白
_SP_PRE_BR = re.compile(r" +\[")                                 # 括号前空白（「【XX [投标人」族）
_SP_POST_BR = re.compile(r"\] +")                                 # 括号后空白（两侧一致应用）
_PAREN_DIG = re.compile(r"\) +(?=\d)")                              # 闭括号后数字前空白（「)1、予以」枚举族——run 边界，两侧一致）


def norm(text: str) -> str:
    """列对齐空白吸收：全角空白折叠、首尾剥离、连续空白压一 + CJK 边界空白吸收（v2）。

    v2 增量：任一侧为 CJK 字符时删除中间半角空格（引擎 run 连接在 CJK 邻接处
    插空格 vs 旧实现直接拼接——实测 203+ 对因该族差异被误报 UNCLASSIFIED）。
    纯 ASCII 词间空格不受影响（「ISO 9001」「SQLite 3.38」保持原样）。
    v2.1：括号内边空白吸收（`[ / ]`→`[/]`、`[ 投标人`→`[投标人`——表单勾选框/占位
    括号的内边空格不承载语义，实测 shanghai 件 33+ 例）。"""
    t = _WS.sub(" ", text.replace(FULLWIDTH_SPACE, " ")).strip()
    prev = None
    while prev != t:
        prev = t
        t = _C2C.sub(r"\1\2", t)
        t = _C2A.sub(r"\1\2", t)
        t = _A2C.sub(r"\1\2", t)
        t = _BR_OPEN.sub("[", t)
        t = _BR_CLOSE.sub("]", t)
        t = _SP_PRE_BR.sub("[", t)
        t = _SP_POST_BR.sub("]", t)
        t = _PAREN_DIG.sub(")", t)
        t = t.replace('""', '')
    return t


# ---------------------------------------------------------------- v2：域指令码剥离
# 旧实现把 w:instrText 拼进正文文本的实测形态（B3-3 全量差分 392+ 对锚定）：
#   ` PAGEREF _Toc4853 \\h1`（TOC 条目页码引用指令尾巴）
#   `HYPERLINK "https://..."`（超链接指令——引擎取结果文本，指令不残留）
#   `目录TOC \\o "1-2" \\h \\u`（TOC 域主指令——旧侧整段泄漏）
_PAGEREF_TAIL = re.compile(r"\s*PAGEREF _Toc\d+ \\h\d+\s*$")
_HYPERLINK_INSTR = re.compile(r'HYPERLINK "([^"\s]*)"')
_TOC_INSTR = re.compile(r"\s*(目录)?TOC \\o \"[^\"]*\"( \\h)?( \\u)?\s*")


def has_field_codes(text: str) -> bool:
    """文本是否携带域指令码（旧实现泄漏面）。"""
    return bool(_PAGEREF_TAIL.search(text) or _HYPERLINK_INSTR.search(text)
                or re.search(r"TOC \\o ", text))


def strip_field_codes(text: str) -> str:
    """剥离域指令码（PAGEREF 尾巴删除；HYPERLINK 指令取目标 URL 作文本；TOC 主指令删除）。"""
    t = _PAGEREF_TAIL.sub("", text)
    t = _HYPERLINK_INSTR.sub(r"\1", t)
    t = _TOC_INSTR.sub("", t)
    return norm(t)


def canon(text: str) -> str:
    """对齐/比较钥：norm + 域码剥离（旧侧带码与断侧洁净版直接同键——消解配对位移）。"""
    return strip_field_codes(text)


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
def _toc_prefix_found(o_t: str, old_canon_texts: set[str] | None,
                      new_texts: list[str] | None) -> bool:
    """TOC 条目签名：去尾部页码后的前缀在两侧其他块中可寻（双发/聚合形态）。"""
    stripped = re.sub(r"\d{1,3}\s*$", "", o_t).strip()
    if len(stripped) < 4:
        return False
    if old_canon_texts and any(stripped != t and stripped in t for t in old_canon_texts):
        return True
    return bool(new_texts) and any(stripped in t for t in new_texts)


def classify_pair(old_blk: dict, new_blk: dict | None, contract: dict,
                  comps: dict[str, dict], new_texts: list[str] | None = None,
                  old_canon_texts: set[str] | None = None,
                  ctx: dict | None = None) -> dict:
    """对齐块对（或单边）判定。返回 verdict item（new_texts = 引擎侧全文本集，
    供旧侧孤块反查引擎是否已提取——修复前后差异定向用；ctx = 文件级上下文：
    old_tbl_gap（旧表数-引擎表数）/ old_has_codes（旧侧任一块携带域码））。"""
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
        ctx = ctx or {}
        if present_in_new:
            item.update(verdict="OLD_BUG", rule="D4-DOUBLE-EXTRACT（同文本重复入流——mdcore#18 勘误）")
        elif has_field_codes(o_t):
            # v2：旧孤块携带域指令码（TOC 主指令整段泄漏等）——引擎已剥离，旧侧缺陷
            item.update(verdict="OLD_BUG",
                        rule="FIELD-CODE-LEAK（旧孤块域指令码——引擎 A2 族已剥离）")
        elif (kind_o == "table_row"
              and ctx.get("old_row_loc_counts", {}).get(old_blk.get("locator", ""), 0) > 1):
            # v5：旧侧同 locator 重复发射余量——r3 实证（引擎 #39 修复嵌套表后
            # 暴露）：huaian 表2行11 ×23 / shanghai 表2行3 ×30 / shanxi 表5行11
            # ×24，旧侧嵌套表区域累积重复发射（单元格数递增 3→4→7→8→11…），
            # 基线变体（与引擎一致的最短形态）已被配对消费，余量为旧侧多发。
            # 非引擎缺口——引擎侧该行恰一次、内容完整。
            item.update(verdict="OLD_BUG",
                        rule="DUP-EMISSION（旧侧同 locator 重复发射余量——基线变体已配对）")
        elif (kind_o == "para" and _ws(o_t) in ctx.get("new_ws_set", set())):
            # v5：空白不敏感在册——同内容空白形态差异（r3 实证：日期签署行族
            # 「___月_ __日」vs「___月___日」，表单下划线空格位漂移），引擎侧
            # 已有同文本 → 旧侧多重发射余量，非缺口。
            item.update(verdict="EXPECTED_DIFF",
                        rule="DUP-RESIDUE-WS（同内容空白形态差异在册——旧侧多发余量）")
        elif (kind_o == "para"
              and any(_ws(o_t) in t for t in ctx.get("new_tbl_ws_texts", []) if t)):
            # v5：OLD-TABLE-MISS 孤块对偶面——旧侧散落 para 空白不敏感包含于
            # 引擎某整行 table_row（r3 实证：huaian 表1行52 整行聚合了旧侧
            # 代理服务费条款段+算例段+合计段——引擎形态优于旧侧拆散）。
            item.update(verdict="OLD_BUG",
                        rule="OLD-TABLE-MISS（孤块对偶面：旧散落 para ⊂ 引擎整行 table_row）")
        elif kind_o == "table_row" or ctx.get("old_tbl_gap", 0) > 0:
            # v2.3：嵌套表格缺口——旧侧表数 > 引擎表数（实测 huaian 21>20 /
            #   shanghai 30>28 / shanxi 31>30，费率表 w:tbl@w:tc 内嵌、depth=2）；
            #   旧孤 table_row（缺表本性行）或旧孤 para（缺表区域散落段——旧侧
            #   以 para 形态发射嵌套表内容）→ 引擎缺口，非旧侧数据
            item.update(verdict="ENGINE_MISSING",
                        rule="NESTED-TABLE（嵌套表格未提取——旧表数>引擎表数，w:tbl@w:tc 内嵌族）")
        elif (re.search(r"\d{1,3}\s*$", o_t) and len(o_t) <= 60
              and _toc_prefix_found(o_t, old_canon_texts, new_texts)):
            # v2.3：TOC 域整域处理差异——旧发射缓存条目（尾部页码，短行；去页码前缀
            #   在两侧其他块中可寻——双发/聚合形态实证），引擎按域剥离计数；导航
            #   机器非内容，交换可接受（unicom 实测 95 例：纯文本 TOC，无 PAGEREF 码）
            item.update(verdict="EXPECTED_DIFF",
                        rule="TOC-FIELD-REGION（TOC 缓存条目发射差异——导航机器非内容）")
        elif comps.get("文本框", {}).get("present", 0) > 0:
            item.update(verdict="ENGINE_MISSING", rule="D4-TEXTBOX")
        else:
            item.update(verdict="UNCLASSIFIED", rule="old-only（待人工裁决）")
        return item
    # —— v2：域码泄漏（配对块）——旧侧携带 PAGEREF/HYPERLINK/TOC 指令码而引擎洁净，
    #    剥离后内容等价 → 旧实现把格式机器码泄进正文流（条款抽取噪声面）
    if has_field_codes(o_t) and not has_field_codes(n_t):
        stripped = strip_field_codes(o_t)
        if stripped == n_t or (stripped and n_t.startswith(stripped)):
            item.update(verdict="OLD_BUG",
                        rule="FIELD-CODE-LEAK（旧侧域指令码泄漏，剥离后内容等价/前缀）")
            return item
    # —— v2：旧侧表格结构缺失——旧把表格区拆成散落 para（逐格一段），引擎整行 table_row；
    #    判定：kind para→table_row 且 canon 文本互含（等价/子串聚合两形态）
    if kind_o == "para" and kind_n == "table_row":
        o_c, n_c = canon(o_t), canon(n_t)
        if o_c == n_c or (o_c and o_c in n_c) or (n_c and n_c in o_c):
            item.update(verdict="OLD_BUG",
                        rule="OLD-TABLE-MISS（旧侧表格区拆散为 para，引擎整行 table_row）")
            return item
    # —— v2：交叉错配残留——重复前缀（TOC 条目 vs 真标题、多条（2）小节）使近配对
    #    错位。内容双侧存在性校验：canon 文本在对方全文集中出现 → 错配伪差异，
    #    非内容缺口（真缺口不可能双侧都在）
    if new_texts is not None and old_canon_texts is not None:
        o_c2, n_c2 = canon(o_t), canon(n_t)
        o_in_new = bool(o_c2) and any(o_c2 in t for t in new_texts)
        n_in_old = bool(n_c2) and any(n_c2 in t for t in old_canon_texts)
        # 放宽（v3.2）：任一侧 canon 文本在对方全文集可寻即错配伪差异——
        # 内容双侧均在（真缺口不可能双侧都在），只是重复前缀使近配对错位
        if o_in_new or n_in_old:
            item.update(verdict="EXPECTED_DIFF",
                        rule="PAIRING-RESIDUE（重复前缀交叉错配——内容双侧均在，非缺口）")
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
    """① canon 哈希全局配对（v2：域码/CJK 边界空白差异不阻断配对）② 近配对（公共前后缀≥4 且≥半长）③ 余项按序单列。"""
    n_by_text: dict[str, list[int]] = {}
    for i, b in enumerate(new_blocks):
        n_by_text.setdefault(canon(b.get("text", "")), []).append(i)
    used: set[int] = set()
    pairs: list[tuple[dict, dict | None]] = []
    unmatched_old: list[dict] = []
    for ob in old_blocks:
        key = canon(ob.get("text", ""))
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
        o_t = canon(ob.get("text", ""))
        best, best_score = None, 0
        for i in new_free:
            if i in used:
                continue
            n_t = canon(new_blocks[i].get("text", ""))
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
    # PR #15 review：页眉/页脚文本集（source 标注块——HF 规则锚）
    hf_texts = {norm(b.get("text", "")) for b in new_blocks
                if b.get("source") in ("header", "footer")}
    pairs, rest = align(old_blocks, new_blocks)
    # v2：旧侧 para 文本集与 canon 键集——OLD-TABLE-MISS 对偶面与 DUP-RESIDUE 判定锚
    old_para_texts = [norm(b.get("text", "")) for b in old_blocks
                      if b.get("kind") == "para"]
    old_canon_texts = {canon(b.get("text", "")) for b in old_blocks}
    old_canon_list = list(old_canon_texts)
    # v2.3：文件级上下文——旧/引擎表数差（嵌套表缺口信号）与旧侧域码存在性（TOC 族信号）
    _old_tbl_ids = {m.group(0) for b in old_blocks
                    for m in [re.search(r"表\d+", b.get("locator", ""))] if m}
    _new_tbl_ids = {b.get("locator", "").split()[0] for b in new_blocks
                    if b.get("locator", "").startswith("表")}
    # v5：孤块三级证据面——①旧侧 table_row 同 locator 发射计数（DUP-EMISSION 锚：
    #   嵌套表区域旧侧累积重复发射，基线变体已被配对消费）；②引擎侧空白不敏感
    #   文本集（DUP-RESIDUE-WS 锚：同内容空白形态差异在册）；③引擎 table_row 的
    #   空白不敏感文本列表（OLD-TABLE-MISS 孤块对偶面锚：旧散落 para ⊂ 引擎整行）。
    #   （r3 重跑实证：引擎 #39 修复嵌套表后 old_tbl_gap ≤ 0，旧侧嵌套表区域的
    #   重复发射/散落形态暴露为孤块——需真实归类而非笼统记引擎缺口）
    _ws = lambda t: re.sub(r"\s+", "", t or "")
    ctx = {"old_tbl_gap": len(_old_tbl_ids) - len(_new_tbl_ids),
           "old_has_codes": any(has_field_codes(b.get("text", "")) for b in old_blocks),
           "old_row_loc_counts": collections.Counter(
               b.get("locator", "") for b in old_blocks if b.get("kind") == "table_row"),
           "new_ws_set": {_ws(b.get("text", "")) for b in new_blocks},
           "new_tbl_ws_texts": [_ws(b.get("text", "")) for b in new_blocks
                                if b.get("kind") == "table_row"]}
    items = []
    for ob, nb in pairs:
        if nb is None:
            # 旧块无配对 → 走单边分类（文本框重复/缺口/未配对面）
            items.append(classify_pair(ob, None, contract, comps, new_texts, old_canon_list, ctx))
        else:
            items.append(classify_pair(ob, nb, contract, comps, new_texts, old_canon_list, ctx))
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
        # —— PR #15 review：页眉/页脚 source 标注块 → 新能力，非缺陷
        src = nb.get("source", "body")
        if src in ("header", "footer"):
            items.append({
                "old_locator": "", "new_locator": nb.get("locator", ""),
                "kind_old": None, "kind_new": nb.get("kind"),
                "text_old": "", "text_new": n_t,
                "verdict": "EXPECTED_DIFF",
                "rule": f"HF-NEWCAP（{src} 块：引擎读页眉/页脚，旧实现未读——新能力，非缺陷）",
            })
            continue
        # —— PR #15 review：正文流混入页眉/页脚文本 → bug 面，要报（验收清零）
        if n_t in hf_texts:
            items.append({
                "old_locator": "", "new_locator": nb.get("locator", ""),
                "kind_old": None, "kind_new": nb.get("kind"),
                "text_old": "", "text_new": n_t,
                "verdict": "UNCLASSIFIED",
                "rule": "HF-MIXED-BODY（正文流混入页眉/页脚文本——非预期，bug 面要报）",
            })
            continue
        # —— v2：新侧多出的 table_row 吸收了旧侧散落 para——旧表格结构缺失的对偶面
        #    （旧侧散格被近配对消费到别处时，整行块以 new-only 面出现）
        if nb.get("kind") == "table_row":
            n_c = canon(n_t)
            if any(t and t in n_c for t in old_para_texts):
                items.append({
                    "old_locator": "", "new_locator": nb.get("locator", ""),
                    "kind_old": None, "kind_new": nb.get("kind"),
                    "text_old": "", "text_new": n_t,
                    "verdict": "OLD_BUG",
                    "rule": "OLD-TABLE-MISS（新侧整行吸收旧侧散落 para——旧表格结构缺失对偶面）",
                })
                continue
        # —— v2：新侧洁净文本与旧侧某块 canon 键相等但未被哈希消费（重复面/近配位移）——
        #    内容在旧侧确实存在（去码后等价），非引擎新增语义
        if canon(n_t) in old_canon_texts:
            items.append({
                "old_locator": "", "new_locator": nb.get("locator", ""),
                "kind_old": None, "kind_new": nb.get("kind"),
                "text_old": "", "text_new": n_t,
                "verdict": "EXPECTED_DIFF",
                "rule": "DUP-RESIDUE（同文本重复面残留——旧侧同文多块，配对消费后余量）",
            })
            continue
        # —— v3.2：签署行下划线形态差异——引擎 run 分割保留 '_ __' 序列，旧侧合并/丢弃；
        #    形态机器非内容（日期占位行实测 4 例）
        if re.fullmatch(r"[日期：:_\s年月日时]+", n_t):
            items.append({
                "old_locator": "", "new_locator": nb.get("locator", ""),
                "kind_old": None, "kind_new": nb.get("kind"),
                "text_old": "", "text_new": n_t,
                "verdict": "EXPECTED_DIFF",
                "rule": "UNDERSCORE-FORM（签署行下划线形态差异——形态机器非内容）",
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

    # 6) EXPECTED_DIFF / HF-NEWCAP（PR #15 review：引擎读页眉，旧未读——新能力）
    old = {**base_old, "file": "HF-自测.docx", "blocks": [blk(0, "para", "正文条款一")]}
    new = {"file": "HF-自测.docx", "status": "ok",
           "blocks": [blk(0, "para", "正文条款一"),
                       {**blk(1, "para", "HEADER-TENDER-2026", "页眉[default]"),
                        "source": "header"}]}
    r = diff_file(old, new)
    if r["verdict_counts"].get("EXPECTED_DIFF") != 1:
        failures.append(f"EXPECTED_DIFF(HF-NEWCAP): {r['verdict_counts']}")
    if not any("HF-NEWCAP" in it["rule"] for it in r["items"]):
        failures.append("EXPECTED_DIFF(HF-NEWCAP): rule 未命中")

    # 7) UNCLASSIFIED / HF-MIXED-BODY（PR #15 review：正文流混入页眉文本——要报）
    old = {**base_old, "file": "HF-混入.docx", "blocks": []}
    new = {"file": "HF-混入.docx", "status": "ok",
           "blocks": [blk(0, "para", "HEADER-TENDER-2026"),
                       {**blk(1, "para", "HEADER-TENDER-2026", "页眉[default]"),
                        "source": "header"}]}
    r = diff_file(old, new)
    hf_mixed = [it for it in r["items"] if "HF-MIXED-BODY" in it["rule"]]
    if len(hf_mixed) != 1:
        failures.append(f"UNCLASSIFIED(HF-MIXED-BODY): {r['verdict_counts']}（期望恰 1 例混入）")
    if not any("HF-NEWCAP" in it["rule"] for it in r["items"]):
        failures.append("HF-MIXED-BODY 例中页眉块自身应归 HF-NEWCAP")

    # 8) v2 EQUIVALENT：CJK 边界空白吸收（引擎 run 连接插空格 vs 旧直接拼接）
    old = {**base_old, "file": "CJK-自测.docx", "blocks": [blk(0, "para", "第一章中国电信招标公告")]}
    new = {"file": "CJK-自测.docx", "status": "ok",
           "blocks": [blk(0, "para", "第一章 中国电信 招标公告")]}
    r = diff_file(old, new)
    if r["verdict_counts"].get("EQUIVALENT") != 1:
        failures.append(f"EQUIVALENT(CJK-SPACE): {r['verdict_counts']}")
    # 纯 ASCII 词间空格不受影响（仍应报差异，不假等价）
    old = {**base_old, "file": "ASCII-自测.docx", "blocks": [blk(0, "para", "ISO 9001 认证")]}
    new = {"file": "ASCII-自测.docx", "status": "ok", "blocks": [blk(0, "para", "ISO9001 认证")]}
    r = diff_file(old, new)
    if r["verdict_counts"].get("EQUIVALENT") == 1:
        failures.append("ASCII 词间空格不应被吸收（假等价面）")

    # 9) v2 OLD_BUG / FIELD-CODE-LEAK（配对块：PAGEREF 尾巴，去码后等价）
    old = {**base_old, "file": "PAGEREF-自测.docx",
           "blocks": [blk(0, "para", "第二章 投标人须知 PAGEREF _Toc26466 \\h6")]}
    new = {"file": "PAGEREF-自测.docx", "status": "ok",
           "blocks": [blk(0, "para", "第二章 投标人须知")]}
    r = diff_file(old, new)
    hit = [it for it in r["items"] if "FIELD-CODE-LEAK" in it.get("rule", "")]
    if len(hit) != 1 or r["verdict_counts"].get("OLD_BUG") != 1:
        failures.append(f"OLD_BUG(FIELD-CODE-LEAK): {r['verdict_counts']}")

    # 10) v2 OLD_BUG / FIELD-CODE-LEAK（旧孤块：TOC 主指令整段泄漏）
    old = {**base_old, "file": "TOC-自测.docx",
           "blocks": [blk(0, "para", '目录TOC \\o "1-2" \\h \\u')]}
    new = {"file": "TOC-自测.docx", "status": "ok", "blocks": []}
    r = diff_file(old, new)
    hit = [it for it in r["items"] if "FIELD-CODE-LEAK" in it.get("rule", "")]
    if len(hit) != 1:
        failures.append(f"OLD_BUG(FIELD-CODE-LEAK 孤块): {r['verdict_counts']}")

    # 11) v2 OLD_BUG / OLD-TABLE-MISS（kind 冲突：旧散落 para vs 引擎整行 table_row）
    old = {**base_old, "file": "TABLE-自测.docx",
           "blocks": [blk(0, "para", "条款号 条款名称 编写内容")]}
    new = {"file": "TABLE-自测.docx", "status": "ok",
           "blocks": [blk(0, "table_row", "条款号 条款名称 编写内容")]}
    r = diff_file(old, new)
    hit = [it for it in r["items"] if "OLD-TABLE-MISS" in it.get("rule", "")]
    if len(hit) != 1:
        failures.append(f"OLD_BUG(OLD-TABLE-MISS kind冲突): {r['verdict_counts']}")

    # 12) v2 OLD_BUG / OLD-TABLE-MISS（子串聚合：旧散格 ⊂ 新整行）
    old = {**base_old, "file": "TABLE2-自测.docx",
           "blocks": [blk(0, "para", "编写内容")]}
    new = {"file": "TABLE2-自测.docx", "status": "ok",
           "blocks": [blk(0, "table_row", "条款号 条款名称 编写内容")]}
    r = diff_file(old, new)
    hit = [it for it in r["items"] if "OLD-TABLE-MISS" in it.get("rule", "")]
    if len(hit) != 1:
        failures.append(f"OLD_BUG(OLD-TABLE-MISS 聚合): {r['verdict_counts']}")

    # 13) v2 OLD_BUG / OLD-TABLE-MISS（对偶面：旧散格被配对走，新整行以 new-only 面出现）
    old = {**base_old, "file": "TABLE3-自测.docx",
           "blocks": [blk(0, "para", "第一列内容"), blk(1, "para", "unique-para-不进表")]}
    new = {"file": "TABLE3-自测.docx", "status": "ok",
           "blocks": [blk(0, "para", "第一列内容"), blk(1, "para", "unique-para-不进表"),
                       blk(2, "table_row", "第一列内容 第二列内容 第三列内容")]}
    r = diff_file(old, new)
    hit = [it for it in r["items"] if "OLD-TABLE-MISS" in it.get("rule", "")
           and it.get("text_new", "").startswith("第一列内容")]
    if len(hit) != 1:
        failures.append(f"OLD_BUG(OLD-TABLE-MISS 对偶面): {r['verdict_counts']}")

    # 14) v5 OLD_BUG / DUP-EMISSION（旧侧同 locator 重复发射余量：基线变体配对 +
    #     同 locator 长文本孤儿；引擎侧一行恰一次——r3 嵌套表实证形态）
    old = {**base_old, "file": "DUP-自测.docx",
           "blocks": [blk(0, "table_row", "100亿以上 0.004% 0.004%", loc="第二章 表2 行11"),
                       blk(1, "table_row", "100亿以上 0.004% 0.004% 0.004%", loc="第二章 表2 行11")]}
    new = {"file": "DUP-自测.docx", "status": "ok",
           "blocks": [blk(0, "table_row", "100亿以上 0.004% 0.004%", loc="表2 行11")]}
    r = diff_file(old, new)
    hit = [it for it in r["items"] if "DUP-EMISSION" in it.get("rule", "")]
    if len(hit) != 1 or r["verdict_counts"].get("OLD_BUG") != 1:
        failures.append(f"OLD_BUG(DUP-EMISSION): {r['verdict_counts']}")

    # 15) v5 EXPECTED_DIFF / DUP-RESIDUE-WS（空白形态差异在册：旧「月_ __日」
    #     vs 新「月___日」，norm 不等且近配对失败 → 孤儿路径；引擎在册非缺口。
    #     结构同真实数据：旧侧多发一份（精确变体已配对 EQUIVALENT，空白变体成孤儿）
    old = {**base_old, "file": "WS-自测.docx",
           "blocks": [blk(0, "para", "日期：___年___月_ __日"),
                       blk(1, "para", "日期：___年___月___日")]}
    new = {"file": "WS-自测.docx", "status": "ok",
           "blocks": [blk(0, "para", "日期：___年___月___日")]}
    r = diff_file(old, new)
    hit = [it for it in r["items"] if "DUP-RESIDUE-WS" in it.get("rule", "")]
    if len(hit) != 1 or r["verdict_counts"].get("EXPECTED_DIFF") != 1 \
            or r["verdict_counts"].get("EQUIVALENT") != 1:
        failures.append(f"EXPECTED_DIFF(DUP-RESIDUE-WS): {r['verdict_counts']}")

    # 16) v5 OLD_BUG / OLD-TABLE-MISS 孤块对偶面（旧孤 para 空白不敏感包含于引擎
    #     整行中位：r3 实证 huaian 表1行52 聚合面。中位包含 → 近配对不成立 →
    #     孤儿路径；新整行以 new-only 面另行计入 OLD-TABLE-MISS 族（两面同源，
    #     与真实数据双重计数口径一致）
    old = {**base_old, "file": "TDUAL-自测.docx",
           "blocks": [blk(0, "para", "10 0万元×1.5%=1.5万元")]}
    new = {"file": "TDUAL-自测.docx", "status": "ok",
           "blocks": [blk(0, "table_row", "服务费说明：10 0万元×1.5%=1.5万元，按差额定率累进")]}
    r = diff_file(old, new)
    hit = [it for it in r["items"] if "孤块对偶面" in it.get("rule", "")]
    if len(hit) != 1 or r["verdict_counts"].get("OLD_BUG") != 2:
        failures.append(f"OLD_BUG(OLD-TABLE-MISS 孤块对偶面): {r['verdict_counts']}")

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
              "EXPECTED_DIFF(A2+HF-NEWCAP)/UNCLASSIFIED(含 HF-MIXED-BODY) "
              "+ v2：CJK-SPACE/FIELD-CODE-LEAK(配对+孤块)/OLD-TABLE-MISS(kind冲突+聚合+对偶面)/"
              "ASCII 假等价防御 "
              "+ v5：DUP-EMISSION/DUP-RESIDUE-WS/OLD-TABLE-MISS孤块对偶 十六例全命中")
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
