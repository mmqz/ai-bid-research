#!/usr/bin/env python3
"""m17_inventory.py — M1.7-A 政府采购真实语料盘点（S2 批次一：政府采购优先）

对本地已获取的公开政府采购公告（中国政府采购网 www.ccgp.gov.cn，源自外部参考仓库
tlyyxjz__BidAgent 的 _w3/w4/w5_raw 抓取存档——公告本体为依法公开的政府采购信息，
逐份保留 来源URL / 抓取时间 / sha256 / 发布日期 / 项目编号 / 项目名称 溯源字段）做：

  1. 逐文件解析（头部 4 行 + 正文关键域），生成 inventory.json（576 条溯源记录）
  2. sha256 全量计算（可追溯性 + 幂等导入键）
  3. 链发现：按规范化项目编号聚合 tender/correction/award → chains.json
  4. 输出 stats（类型分布 / 平台 / 日期范围 / 链数 / 可提取率）

口径（与用户 M1.7-A 要求对齐）：
  - 只登记公开、可溯源的公告原文存档；不用公告摘要替代原始文件解析测试
  - 本盘点为公告级（announcement-level）；文件级（招标文件本体+补遗文件配对）
    在 m17_real_chain.py 中另行验证，两层不混记
  - 本脚本只读外部仓库，不向其写入任何内容
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "real-corpus" / "gov-procurement"
# 外部参考仓库原始存档（只读）
# FG-16（#305）：仓外绝对路径参数化——外部对照仓路径经 EXTERNAL_RAW 环境变量提供
# （审计 D-20 同族：硬编码仓外路径换环境即断）。缺省 None → 调用方自检并明确报缺。
import os as _os
EXTERNAL_RAW = Path(_os.environ["EXTERNAL_RAW"]) if _os.environ.get("EXTERNAL_RAW") else None
BATCHES = ["_w3_raw", "_w4_raw", "_w5_raw"]

_HDR = re.compile(r"^#\s*(.+)$")
_URL = re.compile(r"^#\s*URL:\s*(\S+)$")
_TYPE = re.compile(r"^#\s*Type:\s*(\S+)$")
_FETCH = re.compile(r"^#\s*Fetched:\s*([\d\-: ]+)$")
_PUB_BODY = re.compile(r"(\d{4})年(\d{2})月(\d{2})日\s*\d{2}:\d{2}\s*来源")
_PUB_PLAIN = re.compile(r"(\d{4})年(\d{1,2})月(\d{1,2})日")
# 项目编号：更正公告（原公告的采购项目编号：X）/ 招标·中标公告（采购项目编号：X）
_PID = re.compile(r"(?:原公告的)?采购项目编号[:：]\s*([^\s，,。；;）)]+)")
_PID2 = re.compile(r"项目编号[:：]\s*([^\s，,。；;）)]+)")
_PNAME = re.compile(r"(?:原公告的)?采购项目名称[:：]\s*([^\s，,。；;）)]+)")


_FW_MAP = {"（": "(", "）": ")", "【": "[", "】": "]", "，": ",", "　": "", " ": "",
           "\u00a0": "", "\t": ""}


def norm_pid(raw: str) -> str:
    """项目编号规范化：全角括号→半角、去空白、统一大写。"""
    s = raw.translate(str.maketrans(_FW_MAP))
    s = re.sub(r"\s+", "", s).upper()
    return s


def parse_one(path: Path, batch: str) -> dict:
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = text.splitlines()
    rec: dict = {
        "file": f"{batch}/{path.name}", "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "bytes": path.stat().st_size,
        "title": None, "url": None, "notice_type": None, "fetched": None,
        "publish_date": None, "publish_date_source": None,
        "project_numbers": [], "project_name": None,
        "body_chars": 0, "parse_flags": [],
    }
    for ln in lines[:6]:
        m = _HDR.match(ln)
        if m and rec["title"] is None and not ln.startswith("# URL") \
                and not ln.startswith("# Type") and not ln.startswith("# Fetched"):
            rec["title"] = m.group(1).strip()
        m = _URL.match(ln)
        if m:
            rec["url"] = m.group(1)
        m = _TYPE.match(ln)
        if m:
            rec["notice_type"] = m.group(1)
        m = _FETCH.match(ln)
        if m:
            rec["fetched"] = m.group(1).strip()
    if rec["notice_type"] is None:  # 头部缺失时回退文件名前缀
        m = re.match(r"w\d_([a-z]+)_", path.name)
        if m:
            rec["notice_type"] = m.group(1)
            rec["parse_flags"].append("type_from_filename")
    body = "\n".join(lines[4:])
    rec["body_chars"] = len(re.sub(r"\s", "", body))
    m = _PUB_BODY.search(text)
    if m:
        rec["publish_date"] = f"{m.group(1)}-{m.group(2)}-{m.group(3)}"
        rec["publish_date_source"] = "body_timestamp"
    else:
        m2 = re.search(r"/(\d{6})/t(\d{8})_", rec.get("url") or "")
        if m2 and m2.group(1) == m2.group(2)[:6]:
            d = m2.group(2)
            rec["publish_date"] = f"{d[:4]}-{d[4:6]}-{d[6:]}"
            rec["publish_date_source"] = "url_path"
        else:
            m3 = _PUB_PLAIN.search(body[:400])
            if m3:
                rec["publish_date"] = f"{m3.group(1)}-{int(m3.group(2)):02d}-{int(m3.group(3)):02d}"
                rec["publish_date_source"] = "body_first_date"
    for pat in (_PID, _PID2):
        for mm in pat.finditer(text):
            v = norm_pid(mm.group(1))
            # 括注噪声截断：值内括注若含“编号/文件/包/标段”等字样则为注释性噪声
            if "(" in v:
                head, paren = v.split("(", 1)
                if re.search(r"编号|文件|包|标段|招标", paren[:24]):
                    v = head
            if v and v not in rec["project_numbers"]:
                rec["project_numbers"].append(v)
    m = _PNAME.search(text)
    if m:
        rec["project_name"] = m.group(1).strip()
    if rec["title"] is None:
        rec["parse_flags"].append("no_title")
    if not rec["project_numbers"]:
        rec["parse_flags"].append("no_pid")
    return rec


def main() -> int:
    if EXTERNAL_RAW is None:
        print("!! 外部对照仓未提供：export EXTERNAL_RAW=<tlyyxjz__BidAgent 检出路径>"
              "（FG-16：不再硬编码仓外绝对路径）", file=sys.stderr)
        return 2
    records = []
    for b in BATCHES:
        d = EXTERNAL_RAW / b
        if not d.is_dir():
            print(f"!! 缺目录：{d}", file=sys.stderr)
            return 1
        for p in sorted(d.glob("*.txt")):
            records.append(parse_one(p, b))
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "inventory.json").write_text(
        json.dumps({"source_repo": "tlyyxjz__BidAgent（外部参考仓库，只读）",
                    "origin_platform": "中国政府采购网 www.ccgp.gov.cn",
                    "count": len(records), "files": records},
                   ensure_ascii=False, indent=1), encoding="utf-8")

    # ---- 链发现（规范化项目编号聚合）
    chains: dict[str, dict] = {}
    for r in records:
        for pid in r["project_numbers"]:
            chains.setdefault(pid, {"project_number": pid, "notices": []})["notices"].append(r)
    multi = []
    for pid, c in chains.items():
        seen = set()
        uniq = []
        for r in c["notices"]:
            k = r["file"]
            if k not in seen:
                seen.add(k)
                uniq.append({"file": r["file"], "notice_type": r["notice_type"],
                             "publish_date": r["publish_date"], "url": r["url"],
                             "sha256": r["sha256"],
                             "project_name": r["project_name"]})
        c["notices"] = uniq
        c["type_set"] = sorted({n["notice_type"] for n in uniq})
        if len(uniq) >= 2:
            multi.append(c)
    multi.sort(key=lambda c: (-len(c["notices"]), c["project_number"]))
    (OUT / "chains.json").write_text(
        json.dumps({"multi_notice_chains": multi,
                    "single_notice_pids": len(chains) - len(multi)},
                   ensure_ascii=False, indent=1), encoding="utf-8")

    # ---- stats
    by_type: dict[str, int] = {}
    for r in records:
        by_type[r["notice_type"]] = by_type.get(r["notice_type"], 0) + 1
    dates = sorted(r["publish_date"] for r in records if r["publish_date"])
    full = [c for c in multi if set(c["type_set"]) >= {"tender", "correction", "award"}]
    stats = {
        "total_files": len(records),
        "by_type": by_type,
        "with_url": sum(1 for r in records if r["url"]),
        "with_publish_date": len(dates),
        "date_range": [dates[0], dates[-1]] if dates else None,
        "with_pid": sum(1 for r in records if r["project_numbers"]),
        "distinct_pids": len(chains),
        "multi_notice_chains": len(multi),
        "full_chains_tender_correction_award": len(full),
        "avg_body_chars": sum(r["body_chars"] for r in records) // max(1, len(records)),
    }
    (OUT / "stats.json").write_text(json.dumps(stats, ensure_ascii=False, indent=1),
                                    encoding="utf-8")
    print(json.dumps(stats, ensure_ascii=False, indent=1))
    print("\n完整链（tender+correction+award 同项目编号）样例：")
    for c in full[:5]:
        print(f"  {c['project_number']}  {c['type_set']}  {len(c['notices'])} 份")
    print("\n双份链（≥2 公告）样例：")
    for c in multi[:8]:
        print(f"  {c['project_number']}  {c['type_set']}  {[n['file'] for n in c['notices']]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
