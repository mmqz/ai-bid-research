#!/usr/bin/env python3
"""Search GitHub for supplemental bid-related repos with real code."""
import json, os
import urllib.request, urllib.parse, urllib.error

TOKEN = os.environ.get("GH_TOKEN", "").strip()
BASE = "https://api.github.com"
# FG-16（#305）：仓外绝对路径参数化——默认仓内 research/raw，GH_RAW_OUT 可覆盖
import pathlib as _pl
OUT = os.environ.get("GH_RAW_OUT") or str(_pl.Path(__file__).resolve().parent.parent / "research" / "raw")


def api(path):
    req = urllib.request.Request(BASE + path, headers={
        "Authorization": f"Bearer {TOKEN}", "Accept": "application/vnd.github+json",
        "User-Agent": "bid-research"})
    try:
        with urllib.request.urlopen(req, timeout=40) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, None
    except Exception as e:
        return -1, str(e)


QUERIES = [
    ("标书", "stars"), ("投标", "stars"), ("招标文件", "stars"), ("标书 生成", None),
    ("投标 助手", "stars"), ("tender bid LLM", "stars"), ("bid writing", "stars"),
    ("招投标 agent", "stars"), ("废标", None), ("响应矩阵", None),
]

seen = {}
for q, sort in QUERIES:
    qs = urllib.parse.quote(q)
    url = f"/search/repositories?q={qs}&per_page=8" + (f"&sort={sort}" if sort else "")
    st, res = api(url)
    total = res.get("total_count") if isinstance(res, dict) else "?"
    print(f"\n### q='{q}' sort={sort} -> {st} total={total}")
    if st != 200:
        continue
    for it in res.get("items", []):
        fn = it["full_name"]
        if fn in seen:
            continue
        seen[fn] = {"full_name": fn, "stars": it["stargazers_count"], "language": it.get("language"),
                    "size": it["size"], "pushed": it["pushed_at"][:10], "created": it["created_at"][:10],
                    "license": (it.get("license") or {}).get("spdx_id"), "desc": it.get("description"),
                    "archived": it.get("archived"), "fork": it.get("fork")}
        d = (it.get("description") or "")[:88]
        print(f"  {fn} | *{it['stargazers_count']} | {it.get('language')} | {it['size']}KB | push {it['pushed_at'][:10]} | {d}")

json.dump(list(seen.values()), open(os.path.join(OUT, "search_results.json"), "w", encoding="utf-8"),
          ensure_ascii=False, indent=1)
print(f"\nsaved {len(seen)} unique")
