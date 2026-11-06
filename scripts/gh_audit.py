#!/usr/bin/env python3
"""GitHub repo audit: metadata / file tree / README / languages / recent commits.
Saves raw evidence under research/raw/ for citation."""
import json, os, sys, time
import urllib.request, urllib.parse, urllib.error

TOKEN = os.environ.get("GH_TOKEN", "").strip()
BASE = "https://api.github.com"
# FG-16（#305）：仓外绝对路径参数化——默认仓内 research/raw，GH_RAW_OUT 可覆盖
import pathlib as _pl
OUT = os.environ.get("GH_RAW_OUT") or str(_pl.Path(__file__).resolve().parent.parent / "research" / "raw")
os.makedirs(OUT, exist_ok=True)


def api(path, raw=False):
    req = urllib.request.Request(BASE + path, headers={
        "Authorization": f"Bearer {TOKEN}",
        "Accept": "application/vnd.github.raw" if raw else "application/vnd.github+json",
        "User-Agent": "bid-research-audit",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    try:
        with urllib.request.urlopen(req, timeout=40) as r:
            body = r.read().decode("utf-8", "replace")
            return r.status, (body if raw else json.loads(body))
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read())
        except Exception:
            return e.code, None
    except Exception as e:
        return -1, str(e)


def slug(s):
    return s.replace("/", "__")


KWS = ["skill", "dify", "mcp", "workflow", "langgraph", "prompt", "parse", "extract",
       "requirement", "matrix", "compliance", "evidence", "agent", "state", "checkpoint",
       "resume", "retry", "export", "review", "knowledge", "rag",
       "知识库", "标书", "招标", "投标", "检索", "审校"]


def audit(repo):
    st, meta = api(f"/repos/{repo}")
    print(f"\n{'=' * 72}\n=== {repo} -> HTTP {st}")
    if st != 200:
        print(f"  RESULT: {'NOT FOUND / INACCESSIBLE' if st == 404 else f'ERROR code={st}'}")
        q = repo.split("/")[-1]
        st2, res = api(f"/search/repositories?q={urllib.parse.quote(q)}&per_page=5")
        if st2 == 200 and isinstance(res, dict) and res.get("total_count", 0) > 0:
            for it in res["items"][:5]:
                print(f"  ~similar: {it['full_name']} stars={it['stargazers_count']} "
                      f"pushed={it['pushed_at'][:10]} desc={(it.get('description') or '')[:80]}")
        return
    lic = (meta.get("license") or {}).get("spdx_id") or "NO_LICENSE"
    print(f"  license={lic} stars={meta['stargazers_count']} forks={meta['forks_count']} size_kb={meta['size']}")
    print(f"  created={meta['created_at'][:10]} pushed={meta['pushed_at'][:10]} branch={meta['default_branch']} lang={meta.get('language')}")
    print(f"  desc={meta.get('description')}")
    print(f"  archived={meta.get('archived')} fork={meta.get('fork')} open_issues={meta.get('open_issues_count')}")
    br = meta["default_branch"]
    stl, langs = api(f"/repos/{repo}/languages")
    if stl == 200 and isinstance(langs, dict):
        top = dict(sorted(langs.items(), key=lambda x: -x[1])[:5])
        print(f"  langs(bytes): {top}")
    stt, tree = api(f"/repos/{repo}/git/trees/{urllib.parse.quote(br)}?recursive=1")
    files, truncated = [], False
    if stt == 200 and isinstance(tree, dict):
        truncated = tree.get("truncated", False)
        files = [t["path"] for t in tree.get("tree", []) if t["type"] == "blob"]
    print(f"  tree: {len(files)} blobs, truncated={truncated}")
    if files:
        tops = sorted(set(p.split("/")[0] for p in files))[:24]
        print(f"  top_dirs: {tops}")
        hits = [p for p in files if any(k in p.lower() for k in KWS)]
        print(f"  keyword_hits({len(hits)}): {hits[:22]}")
        special = [p for p in files if p.endswith("SKILL.md") or p.lower().endswith((".yml", ".yaml")) or "mcp" in p.lower()]
        print(f"  skill/dsl/mcp_files({len(special)}): {special[:14]}")
    strd, rd = api(f"/repos/{repo}/readme", raw=True)
    if strd == 200 and isinstance(rd, str):
        text = rd.replace("\r", "")
        open(os.path.join(OUT, f"{slug(repo)}__README.md"), "w", encoding="utf-8").write(text)
        print(f"  README: {len(text)} chars saved. head:")
        print("    " + text[:400].replace("\n", "\n    "))
    stc, cm = api(f"/repos/{repo}/commits?per_page=3")
    if stc == 200 and isinstance(cm, list):
        for c in cm[:3]:
            msg = (c["commit"]["message"] or "").split("\n")[0][:70]
            print(f"  commit {c['commit']['author']['date'][:10]}: {msg}")
    json.dump({"meta": meta, "files": files, "truncated": truncated},
              open(os.path.join(OUT, f"{slug(repo)}__audit.json"), "w", encoding="utf-8"),
              ensure_ascii=False, indent=1)


if __name__ == "__main__":
    st, rl = api("/rate_limit")
    if st == 200:
        c = rl["resources"]["core"]
        print(f"rate: remaining={c['remaining']}/{c['limit']}")
    for r in sys.argv[1:]:
        audit(r)
        time.sleep(0.4)
