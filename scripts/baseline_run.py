#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""baseline_run.py —— issue #2 · B1 盘点与基线冻结（一键复现）。

产物（全部落盘、重跑可复现）：
  baseline/env.json              环境指纹（无时间戳）
  baseline/MANIFEST.json         语料清单 + 每文件 sha256 + 环境指纹 + 二跑一致性
  baseline/parse/<rel>.json       逐文件解析结果（失败件也落盘，不许缺文件）
  baseline/parse-full/<rel>.json 大文件（>5MB）全量另存，parse/ 内只存摘要
  baseline/security/hostile/*    6 类敌意文件（确定性构造）
  baseline/security/<name>.json  敌意文件当前真实行为基线
  baseline/ENV_BLOCKED.md        环境阻塞项清单（没有则写「无」）

确定性纪律：
  - JSON sort_keys / ensure_ascii=False / indent=2，无时间戳、无随机；
  - elapsed_ms / peak_mem_mb 为性能观测字段，不参与一致性 digest；
  - 双跑一致性 = 单次调用内对全部语料做两遍解析 pass，对「去掉 elapsed_ms
    的规范化 JSON」逐文件比对 sha256；敌意文件两遍比对 (result, error_text)
    分类。MANIFEST 因此跨调用字节一致（除 mdcore 远端 rev 漂移外，
    可用环境变量 BASELINE_MDCORE_REV 钉住）。

用法：
  python3 scripts/baseline_run.py                 # 全量
  python3 scripts/baseline_run.py --skip-security # 跳过敌意件（快检）
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import platform
import shutil
import subprocess
import sys
import time
import warnings
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SCRIPTS = REPO / "scripts"
BASE = REPO / "baseline"
HARNESS_DIR = SCRIPTS / "baseline_harness"
HARNESS = HARNESS_DIR / "target" / "release" / "baseline-harness"
CORPUS_ROOTS = ["real-corpus", "fixtures"]
PARSE_TIMEOUT_S = 600      # 语料解析超时（敌意件另有 10s 硬门）
HOSTILE_TIMEOUT_S = 10     # 敌意件超时 → 记 hang
LARGE_BYTES = 5 * 1024 * 1024

ADVISORY_FIELDS = ("elapsed_ms", "peak_mem_mb")  # 性能观测，不进 digest


# ---------------------------------------------------------------- 基础工具

def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def canonical(obj) -> str:
    return json.dumps(obj, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def normalized_digest(payload: dict) -> str:
    core = {k: v for k, v in payload.items() if k not in ADVISORY_FIELDS}
    return hashlib.sha256(canonical(core).encode("utf-8")).hexdigest()


def write_json(path: Path, obj) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(obj, sort_keys=True, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def sh(args, cwd=None, timeout=60):
    return subprocess.run(
        args, cwd=cwd, capture_output=True, text=True, timeout=timeout,
        env={**os.environ, "PATH": f"{Path.home() / '.cargo' / 'bin'}:{os.environ.get('PATH', '')}"},
    )


def categorize(ext: str) -> str:
    e = ext.lower()
    if e in ("docx",):
        return "docx"
    if e in ("xlsx", "xlsm"):
        return "xlsx"
    if e == "pdf":
        return "pdf"
    if e == "zip":
        return "zip"
    return "other"


# ---------------------------------------------------------------- 环境指纹

def probe_tool(name: str, ver_args: list) -> str:
    try:
        p = subprocess.run([name, *ver_args], capture_output=True, text=True, timeout=15)
        out = (p.stderr.strip() or p.stdout.strip()).splitlines()
        return out[0][:120] if out else "present(version-unknown)"
    except FileNotFoundError:
        return "MISSING"
    except Exception as e:  # noqa: BLE001
        return f"PROBE_FAILED:{type(e).__name__}"


def env_fingerprint() -> dict:
    ai_rev = sh(["git", "-C", str(REPO), "rev-parse", "HEAD"]).stdout.strip() or "UNKNOWN"
    md_rev = os.environ.get("BASELINE_MDCORE_REV", "")
    if not md_rev:
        r = sh(["git", "ls-remote", "https://github.com/mmqz/mdcore.git", "main"], timeout=30)
        md_rev = r.stdout.split()[0] if r.returncode == 0 and r.stdout.split() else "UNREACHABLE"
    cargo_v = sh(["cargo", "--version"]).stdout.strip() or "MISSING"
    rustc_v = sh(["rustc", "--version"]).stdout.strip() or "MISSING"
    return {
        "ai_bid_research_rev": ai_rev,
        "mdcore_main_rev_record_only": md_rev,
        "cargo": cargo_v,
        "rustc": rustc_v,
        "os": f"{platform.system()} {platform.release()} {platform.machine()}",
        "cpu_cores": os.cpu_count(),
        "python": platform.python_version(),
        "tools": {
            "pdftotext": probe_tool("pdftotext", ["-v"]),
            "pdftoppm": probe_tool("pdftoppm", ["-v"]),
            "pdfinfo": probe_tool("pdfinfo", ["-v"]),
            "tesseract": probe_tool("tesseract", ["--version"]),
        },
        "tessdata_chi_sim": (
            "present" if (Path.home() / ".tessdata" / "chi_sim.traineddata").exists() else "MISSING"
        ),
        "usr_bin_time": "present" if Path("/usr/bin/time").exists() else "MISSING(以 rusage 替代)",
    }


# ---------------------------------------------------------------- 语料清单

def corpus_files() -> list[dict]:
    out = []
    for root in CORPUS_ROOTS:
        base = REPO / root
        for p in base.rglob("*"):
            if p.is_file():
                rel = p.relative_to(REPO).as_posix()
                out.append({
                    "path": rel,
                    "bytes": p.stat().st_size,
                    "sha256": sha256_file(p),
                    "category": categorize(p.suffix.lstrip(".")),
                })
    out.sort(key=lambda x: x["path"])
    return out


# ---------------------------------------------------------------- 解析（逐文件单进程）

def ensure_harness() -> None:
    if HARNESS.exists():
        return
    print("  [harness] 未找到 release 二进制，执行 cargo build --release ...")
    r = sh(["cargo", "build", "--release"], cwd=HARNESS_DIR, timeout=600)
    if r.returncode != 0 or not HARNESS.exists():
        print(r.stdout[-2000:])
        print(r.stderr[-2000:], file=sys.stderr)
        raise SystemExit("baseline-harness 构建失败（cargo 不可用或依赖拉取失败）→ ENV_BLOCKED")


def run_parse_one(abspath: Path, rel: str) -> dict:
    """spawn harness 单文件解析。返回规定 schema payload（含 elapsed_ms）。"""
    try:
        p = subprocess.run(
            [str(HARNESS), "--file", str(abspath), "--rel", rel],
            capture_output=True, text=True, timeout=PARSE_TIMEOUT_S,
        )
    except subprocess.TimeoutExpired:
        return {
            "file": rel, "sha256": None, "pages": 0, "has_text_layer": False,
            "parse_summary": "", "components": [], "blocks": [],
            "elapsed_ms": PARSE_TIMEOUT_S * 1000, "status": "failed",
            "error": f"timeout-{PARSE_TIMEOUT_S}s",
        }
    out = (p.stdout or "").strip()
    if p.returncode in (0, 3) and out:
        try:
            return json.loads(out.splitlines()[-1])
        except json.JSONDecodeError:
            pass
    # panic（101）/信号/输出损坏 → 失败件也落盘，不许缺
    tail = (p.stderr or "")[-1500:].replace("\n", " ⏎ ")
    return {
        "file": rel, "sha256": None, "pages": 0, "has_text_layer": False,
        "parse_summary": "", "components": [], "blocks": [],
        "elapsed_ms": -1, "status": "failed",
        "error": f"harness-exit-{p.returncode}: {tail}",
    }


_MEASURE_CHILD = r"""
import json, os, resource, signal, subprocess, sys, time
cmd = sys.argv[1:]
t0 = time.monotonic()
p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                     text=True, start_new_session=True)
timed_out = False
try:
    out, err = p.communicate(timeout=%(timeout)s)
except subprocess.TimeoutExpired:
    timed_out = True
    try:
        os.killpg(p.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    out, err = p.communicate()
ru = resource.getrusage(resource.RUSAGE_CHILDREN)
print(json.dumps({
    "returncode": p.returncode,
    "timed_out": timed_out,
    "stdout": out or "",
    "stderr": (err or "")[-2000:],
    "elapsed_ms": int((time.monotonic() - t0) * 1000),
    "peak_rss_kb": ru.ru_maxrss,
}))
"""


def run_isolated(abspath: Path, rel: str, timeout_s: int) -> dict:
    """隔离进程运行（fresh measuring child → 独立 rusage 峰值内存 + 超时 killpg）。"""
    code = _MEASURE_CHILD % {"timeout": timeout_s}
    r = subprocess.run(
        [sys.executable, "-c", code, str(HARNESS), "--file", str(abspath), "--rel", rel],
        capture_output=True, text=True, timeout=timeout_s + 30,
    )
    return json.loads(r.stdout.strip().splitlines()[-1])


def classify_hostile(m: dict) -> tuple[str, str]:
    """(result, error_text)：rejected|panic|hang|succeeded 的当前真实行为。"""
    if m["timed_out"]:
        return "hang", f"timeout-{HOSTILE_TIMEOUT_S}s"
    rc = m["returncode"]
    out = (m.get("stdout") or "").strip()
    if rc == 0 and out:
        try:
            payload = json.loads(out.splitlines()[-1])
            n = len(payload.get("blocks", []))
            return "succeeded", f"parse-ok blocks={n}"
        except json.JSONDecodeError:
            return "succeeded", "parse-ok(stdout-unparsable)"
    if rc == 3 and out:
        try:
            payload = json.loads(out.splitlines()[-1])
            return "rejected", payload.get("error", "")[:500]
        except json.JSONDecodeError:
            pass
        return "rejected", out[:500]
    if rc == 101 or (isinstance(rc, int) and rc < 0):
        return "panic", (m.get("stderr") or "")[-500:].replace("\n", " ⏎ ")
    return "panic", f"harness-exit-{rc}: {(m.get('stderr') or '')[-400:]}".replace("\n", " ⏎ ")


# ---------------------------------------------------------------- 敌意文件（确定性构造）

CT_XML = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Default Extension="xml" ContentType="application/xml"/>'
    '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
    "</Types>"
)
RELS = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>'
    "</Relationships>"
)
WDOC_HDR = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>'
)
WDOC_FTR = "</w:body></w:document>"

LOL_ENTITIES = "".join(
    f'<!ENTITY lol{i} "&lol{i - 1};&lol{i - 1};&lol{i - 1};&lol{i - 1};&lol{i - 1};&lol{i - 1};&lol{i - 1};&lol{i - 1};&lol{i - 1};&lol{i - 1};">'
    for i in range(1, 10)
)


def _zipinfo(name: str) -> zipfile.ZipInfo:
    zi = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
    zi.external_attr = 0o600 << 16
    zi.compress_type = zipfile.ZIP_DEFLATED
    return zi


def build_hostile_dir(hdir: Path) -> list[dict]:
    """确定性构造 6 类敌意文件。返回 [{name, file, construction}]。"""
    hdir.mkdir(parents=True, exist_ok=True)

    # 1. zip 炸弹：声明 10KB / 实解 1GB（流式写 1MB×1024 零块，事后补丁声明面）
    bomb = hdir / "01-zip-bomb-declared-10kb-actual-1gb.docx"
    with zipfile.ZipFile(bomb, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
        zf.writestr(_zipinfo("[Content_Types].xml"), CT_XML)
        zi = _zipinfo("word/document.xml")
        with zf.open(zi, "w") as f:
            chunk = bytes(1024 * 1024)
            for _ in range(1024):
                f.write(chunk)
    data = bomb.read_bytes()
    buf = bytearray(data)
    # 补丁所有 uncompressed 声明面：local header +22 / central dir +24 / data descriptor +12
    patched = 0
    i = 0
    while i < len(buf) - 4:
        if buf[i : i + 4] == b"PK\x03\x04":
            buf[i + 22 : i + 26] = (10240).to_bytes(4, "little"); patched += 1
        elif buf[i : i + 4] == b"PK\x01\x02":
            buf[i + 24 : i + 28] = (10240).to_bytes(4, "little"); patched += 1
        elif buf[i : i + 4] == b"PK\x07\x08":
            buf[i + 12 : i + 16] = (10240).to_bytes(4, "little"); patched += 1
        i += 1
    bomb.write_bytes(bytes(buf))
    print(f"    [hostile] zip-bomb 构造完成：磁盘 {bomb.stat().st_size} B，声明面补丁 {patched} 处")

    # 2. 路径穿越条目
    pt = hdir / "02-path-traversal.docx"
    with zipfile.ZipFile(pt, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr(_zipinfo("[Content_Types].xml"), CT_XML)
        zf.writestr(_zipinfo("word/../../evil.xml"), b"<evil>traversal</evil>")

    # 3. 重复条目（同名两次不同内容）
    dup = hdir / "03-duplicate-entries.docx"
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        with zipfile.ZipFile(dup, "w", zipfile.ZIP_DEFLATED) as zf:
            zf.writestr(_zipinfo("[Content_Types].xml"), CT_XML)
            zf.writestr(_zipinfo("word/document.xml"), WDOC_HDR + "<w:p><w:r><w:t>first</w:t></w:r></w:p>" + WDOC_FTR)
            zf.writestr(_zipinfo("word/document.xml"), WDOC_HDR + "<w:p><w:r><w:t>second-different</w:t></w:r></w:p>" + WDOC_FTR)

    # 4. 截断容器（合法 docx 砍尾 30%）
    src = REPO / "fixtures/corpus-A/files/A-招标文件.docx"
    raw = src.read_bytes()
    trunc = hdir / "04-truncated-container.docx"
    trunc.write_bytes(raw[: int(len(raw) * 0.7)])

    # 5. XML 实体炸弹（billion laughs）
    laugh = hdir / "05-billion-laughs.docx"
    doc = (
        '<?xml version="1.0" encoding="UTF-8"?>'
        "<!DOCTYPE w:document [<!ENTITY lol \"lol\">"
        + LOL_ENTITIES
        + "]>"
        + WDOC_HDR
        + "<w:p><w:r><w:t>&lol9;</w:t></w:r></w:p>"
        + WDOC_FTR
    )
    with zipfile.ZipFile(laugh, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr(_zipinfo("[Content_Types].xml"), CT_XML)
        zf.writestr(_zipinfo("_rels/.rels"), RELS)
        zf.writestr(_zipinfo("word/document.xml"), doc)

    # 6. 稀疏 xlsx（dimension A1:XFD1048576，实际 1 格）
    sparse = hdir / "06-sparse-xlsx.xlsx"
    xct = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
        '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        "</Types>"
    )
    xrels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
        "</Relationships>"
    )
    xl_rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>'
        "</Relationships>"
    )
    wb_xml = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        '<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>'
    )
    sh_xml = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        '<dimension ref="A1:XFD1048576"/>'
        '<sheetData><row r="1"><c r="A1" t="n"><v>1</v></c></row></sheetData>'
        "</worksheet>"
    )
    with zipfile.ZipFile(sparse, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr(_zipinfo("[Content_Types].xml"), xct)
        zf.writestr(_zipinfo("_rels/.rels"), xrels)
        zf.writestr(_zipinfo("xl/workbook.xml"), wb_xml)
        zf.writestr(_zipinfo("xl/_rels/workbook.xml.rels"), xl_rels)
        zf.writestr(_zipinfo("xl/worksheets/sheet1.xml"), sh_xml)

    return [
        {"name": "01-zip-bomb-declared-10kb-actual-1gb", "file": "01-zip-bomb-declared-10kb-actual-1gb.docx",
         "construction": "docx 形 zip；word/document.xml 实际 1GB 零字节流（deflate 流式写入），local/central/data-descriptor 声明面补丁为 10240B"},
        {"name": "02-path-traversal", "file": "02-path-traversal.docx",
         "construction": "docx 形 zip；条目名 word/../../evil.xml（路径穿越）"},
        {"name": "03-duplicate-entries", "file": "03-duplicate-entries.docx",
         "construction": "word/document.xml 同名条目两次（first / second-different 两版内容）"},
        {"name": "04-truncated-container", "file": "04-truncated-container.docx",
         "construction": f"fixtures/corpus-A/files/A-招标文件.docx（sha256:{sha256_file(src)[:16]}…）截尾 30%：保留前 70% 字节"},
        {"name": "05-billion-laughs", "file": "05-billion-laughs.docx",
         "construction": "word/document.xml 携带 10 层嵌套实体 DTD（&lol9; → 10^9 次展开型实体炸弹）"},
        {"name": "06-sparse-xlsx", "file": "06-sparse-xlsx.xlsx",
         "construction": "sheet1 dimension 声明 A1:XFD1048576（16,384 列 × 1,048,576 行），实际仅 A1 一格"},
    ]


# ---------------------------------------------------------------- 主流程

def main() -> None:
    skip_security = "--skip-security" in sys.argv
    t_start = time.monotonic()

    ensure_harness()

    # 清理再生：baseline 为纯生成物目录（防语料减员后的陈旧残留）
    for sub in ("parse", "parse-full", "security"):
        p = BASE / sub
        if p.exists():
            shutil.rmtree(p)
    BASE.mkdir(parents=True, exist_ok=True)

    print("== [1/5] 环境指纹")
    env = env_fingerprint()
    write_json(BASE / "env.json", env)

    print("== [2/5] 语料清单（real-corpus + fixtures）")
    files = corpus_files()
    by_cat: dict[str, int] = {}
    for f in files:
        by_cat[f["category"]] = by_cat.get(f["category"], 0) + 1
    total_bytes = sum(f["bytes"] for f in files)
    print(f"    {len(files)} 件 / {total_bytes} B / 分类 {by_cat}")

    print("== [3/5] 逐文件解析（双跑一致性地毯）")
    pass1_payloads = []
    for i, f in enumerate(files):
        rel = f["path"]
        p = run_parse_one(REPO / rel, rel)
        f["digest_p1"] = normalized_digest(p)
        pass1_payloads.append(p)
        print(f"    [{i + 1}/{len(files)}] {p['status']:7} {p['elapsed_ms']:>6}ms  {rel}")
    payloads_by_rel = {p["file"]: p for p in pass1_payloads}
    diffs = []
    for f in files:
        p = run_parse_one(REPO / f["path"], f["path"])
        f["digest_p2"] = normalized_digest(p)
        if f["digest_p2"] != f["digest_p1"]:
            diffs.append(f["path"])
            print(f"    !! 双跑不一致: {f['path']}")
    consistency_parse = {
        "method": "单次调用内双跑：全部语料两遍解析，对去掉 elapsed_ms 的规范化 JSON 逐文件 sha256 比对",
        "identical": not diffs,
        "diff_files": diffs,
    }
    print(f"    双跑一致性：{'identical' if not diffs else f'MISMATCH {diffs}'}")

    # 落盘 parse（用 pass1 的 payload；字节面含 elapsed_ms 为性能观测字段）
    ok_n = failed_n = 0
    fail_reasons: dict[str, int] = {}
    large_files = []
    kind_seen: dict[str, int] = {}
    for p in pass1_payloads:
        rel = p["file"]
        content = json.dumps(p, sort_keys=True, ensure_ascii=False, indent=2) + "\n"
        if len(content.encode("utf-8")) > LARGE_BYTES:
            summary = {k: v for k, v in p.items() if k != "blocks"}
            summary["block_count"] = len(p["blocks"])
            summary["block_kind_counts"] = {
                k: sum(1 for b in p["blocks"] if b["kind"] == k) for k in sorted({b["kind"] for b in p["blocks"]})
            }
            summary["large"] = True
            summary["full"] = f"parse-full/{rel}.json"
            write_json(BASE / "parse-full" / f"{rel}.json", p)
            write_json(BASE / "parse" / f"{rel}.json", summary)
            large_files.append(rel)
        else:
            write_json(BASE / "parse" / f"{rel}.json", p)
        if p["status"] == "ok":
            ok_n += 1
            for b in p["blocks"]:
                kind_seen[b["kind"]] = kind_seen.get(b["kind"], 0) + 1
        else:
            failed_n += 1
            err = p.get("error", "")
            key = "不支持的文件类型" if err.startswith("不支持的文件类型") else (
                "timeout" if err.startswith("timeout") else (
                    "panic" if err.startswith("harness-exit") else "解析拒绝/其他"
                )
            )
            fail_reasons[key] = fail_reasons.get(key, 0) + 1
    n_parse_files = sum(1 for _ in (BASE / "parse").rglob("*.json"))
    assert n_parse_files == len(files), f"parse 文件数 {n_parse_files} != 语料 {len(files)}（不许有缺口）"

    print("== [4/5] 敌意文件安全基线" + ("（--skip-security 跳过）" if skip_security else ""))
    security_summary = []
    if not skip_security:
        hostile_specs = build_hostile_dir(BASE / "security" / "hostile")
        sec_results = []
        for passno in (1, 2):
            results = []
            for spec in hostile_specs:
                fpath = BASE / "security" / "hostile" / spec["file"]
                m = run_isolated(fpath, spec["file"], HOSTILE_TIMEOUT_S)
                result, error_text = classify_hostile(m)
                rec = {
                    "file": spec["file"],
                    "bytes": fpath.stat().st_size,
                    "sha256": sha256_file(fpath),
                    "result": result,
                    "error_text": error_text,
                    "elapsed_ms": m["elapsed_ms"],
                    "peak_mem_mb": round(m["peak_rss_kb"] / 1024, 1),
                    "construction": spec["construction"],
                }
                results.append(rec)
                if passno == 1:
                    print(f"    [{rec['result']:9}] {rec['peak_mem_mb']:>8}MB {rec['elapsed_ms']:>6}ms  {spec['file']}")
            sec_results.append(results)
        sec_diffs = [
            a["file"] for a, b in zip(sec_results[0], sec_results[1])
            if (a["result"], a["error_text"]) != (b["result"], b["error_text"])
        ]
        security_summary = sec_results[0]
        for rec in security_summary:
            write_json(BASE / "security" / f"{rec['file']}.json", rec)
        consistency_sec = {
            "method": "敌意件双跑：比对 (result, error_text) 分类稳定性；elapsed/peak 为观测字段不进比对",
            "classification_identical": not sec_diffs,
            "diff_files": sec_diffs,
        }
        print(f"    敌意件分类双跑：{'identical' if not sec_diffs else f'MISMATCH {sec_diffs}'}")
    else:
        consistency_sec = {"method": "skipped", "classification_identical": None, "diff_files": []}

    print("== [5/5] MANIFEST / ENV_BLOCKED")
    manifest = {
        "schema": "ai-bid-research/b1-baseline/1",
        "generated_by": "scripts/baseline_run.py（issue #2 · B1）",
        "corpus_roots": CORPUS_ROOTS,
        "env": env,
        "counts": {
            "total": len(files),
            "by_category": {k: by_cat[k] for k in sorted(by_cat)},
            "parse_ok": ok_n,
            "parse_failed": failed_n,
            "fail_reasons": {k: fail_reasons[k] for k in sorted(fail_reasons)},
            "block_kinds_observed": {k: kind_seen[k] for k in sorted(kind_seen)},
        },
        "files": [
            {
                "path": f["path"], "bytes": f["bytes"], "sha256": f["sha256"],
                "category": f["category"], "parse_status": ("ok" if payloads_by_rel[f["path"]]["status"] == "ok" else "failed"),
                "parse_digest": f["digest_p1"], "large": f["path"] in large_files,
                "parse_json": f"baseline/parse/{f['path']}.json",
            }
            for f in files
        ],
        "security": [
            {
                "file": rec["file"], "bytes": rec["bytes"], "sha256": rec["sha256"],
                "result": rec["result"], "error_digest": hashlib.sha256(rec["error_text"].encode("utf-8")).hexdigest(),
                "detail": f"baseline/security/{rec['file']}.json",
            }
            for rec in security_summary
        ],
        "rerun_consistency": {
            "parse": consistency_parse,
            "security": consistency_sec,
        },
        "_determinism_note": (
            "elapsed_ms/peak_mem_mb 为性能观测字段，不参与一致性 digest；"
            "MANIFEST/env.json 不含时间戳与随机量；跨调用字节一致除 mdcore_main_rev_record_only "
            "（可用 BASELINE_MDCORE_REV 钉住）外。"
        ),
    }
    write_json(BASE / "MANIFEST.json", manifest)

    blocked = []
    for tool, ver in env["tools"].items():
        if ver == "MISSING":
            blocked.append(f"外部工具 {tool} 缺失（解析面相关，须安装后重跑基线）")
    if env["tessdata_chi_sim"] == "MISSING":
        blocked.append("tessdata chi_sim 缺失（OCR 通道：B2/D5a/D5b 扫描件将显式失败）")
    if env["cargo"] == "MISSING":
        blocked.append("cargo 缺失（harness 无法构建）")
    (BASE / "ENV_BLOCKED.md").write_text(
        "# ENV_BLOCKED（B1 基线环境阻塞项）\n\n"
        + ("无（本轮无环境阻塞项）\n" if not blocked else "\n".join(f"- [ ] {b}" for b in blocked) + "\n")
        + f"\n> 探测时间轴外记录：usr_bin_time = {env['usr_bin_time']}（峰值内存以 rusage children 替代，非阻塞）。\n"
        + f"> ai-bid-research rev = {env['ai_bid_research_rev']}；mdcore main rev（只记录）= {env['mdcore_main_rev_record_only']}。\n",
        encoding="utf-8",
    )

    dt = time.monotonic() - t_start
    print(f"\n== 完成（{dt:.0f}s）：{ok_n} ok / {failed_n} failed / 敌意 {len(security_summary)} 件")
    print(f"   parse 双跑：{'一致' if not diffs else '不一致'}；敌意分类双跑：{consistency_sec['classification_identical']}")


if __name__ == "__main__":
    main()
