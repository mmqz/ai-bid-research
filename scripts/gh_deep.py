#!/usr/bin/env python3
"""Deep scan: fetch selected files from a repo and outline them (cached tree)."""
import json, os, re, sys
import urllib.request, urllib.parse

# FG-16（#305）：仓外绝对路径参数化——默认仓内 research/raw，GH_RAW_OUT 可覆盖
import pathlib as _pl
OUT = os.environ.get("GH_RAW_OUT") or str(_pl.Path(__file__).resolve().parent.parent / "research" / "raw")


def slug(s):
    return s.replace("/", "__").replace(".", "_")


def raw_fetch(repo, branch, path):
    url = f"https://raw.githubusercontent.com/{repo}/{urllib.parse.quote(branch)}/{urllib.parse.quote(path)}"
    req = urllib.request.Request(url, headers={"User-Agent": "bid-research"})
    try:
        with urllib.request.urlopen(req, timeout=40) as r:
            return r.read().decode("utf-8", "replace")
    except Exception as e:
        return f"__FETCH_FAILED__ {e}"


def pick(files, maxn=10):
    scored = []
    pat = re.compile(r"(main|app|run|cli|server|pipeline|workflow|agent|prompt|parse|extract|requirement|matrix|compliance|evidence|search|index|embed|chunk|review|validate|export|skill|mcp|state|checkpoint|resume)", re.I)
    for p in files:
        name = p.split("/")[-1].lower()
        ext = name.rsplit(".", 1)[-1] if "." in name else ""
        if ext not in ("py", "js", "ts", "md", "yml", "yaml", "json"):
            continue
        s = 0
        if name == "skill.md" or p.endswith("SKILL.md"):
            s += 100
        if name in ("main.py", "app.py", "server.py", "cli.py", "run.py"):
            s += 70
        if pat.search(p):
            s += 40
        if "node_modules" in p or "dist/" in p or ".min." in p or "test" in name or "__pycache__" in p:
            s -= 60
        if p.startswith((".agents/", ".codex/", ".trellis/", ".claude/", ".github/")):
            s -= 500
        if p.count("/") > 5:
            s -= 15
        if ext in ("yml", "yaml") and ("workflow" in p or "dify" in p):
            s += 120
        scored.append((s, p))
    scored.sort(reverse=True)
    return [p for _, p in scored[:maxn]]


def show(path, content, maxlines=32):
    ext = path.rsplit(".", 1)[-1].lower()
    lines = content.splitlines()
    print(f"\n--- {path} ({len(content)} chars, {len(lines)} lines)")
    if content.startswith("__FETCH_FAILED__"):
        print("    " + content)
        return
    if ext in ("md",):
        out = [l for l in lines[:70] if l.strip()][:42]
        print("\n".join("    " + l[:120] for l in out))
    elif ext in ("yml", "yaml"):
        keep = [l for l in lines if re.match(r"^\s*(mode|app|kind|version|name|id|type|title|provider|model|desc|graph|edges|nodes):", l)]
        print("\n".join("    " + l[:120] for l in lines[:18]))
        print("    [KEY LINES]")
        print("\n".join("    " + l[:120] for l in keep[:18]))
    elif ext == "json":
        print("\n".join("    " + l[:120] for l in lines[:25]))
    else:
        out = []
        for i, l in enumerate(lines, 1):
            ls = l.strip()
            if re.match(r"^(def |class |async def |function |const |export (default )?(function|const|class)|import |from .+ import|@app\.|@router\.)", ls):
                out.append(f"{i:4}: {ls[:110]}")
            if len(out) >= maxlines:
                break
        print("\n".join("    " + x for x in out) if out else "    (no defs; head:)\n    " + content[:250].replace("\n", "\n    "))


repo = sys.argv[1]
branch = sys.argv[2]
paths = sys.argv[3:]
ap = os.path.join(OUT, f"{slug(repo.replace('__','/'))}__audit.json")
# audit.json uses repo full name slugged with __ ; reconstruct properly
import glob
cands = glob.glob(os.path.join(OUT, "*__audit.json"))
tree_files = []
for c in cands:
    base = os.path.basename(c)
    # filename slug replaces / with __ ; match
    if repo.replace("/", "__") in base:
        d = json.load(open(c, encoding="utf-8"))
        tree_files = d.get("files", [])
        break
if not paths:
    paths = pick(tree_files)
print(f"=== deep {repo}@{branch}: {len(paths)} files")
for p in paths:
    c = raw_fetch(repo, branch, p)
    if not c.startswith("__FETCH_FAILED__"):
        open(os.path.join(OUT, f"{repo.replace('/', '__')}__{slug(p)}"), "w", encoding="utf-8").write(c)
    show(p, c)
