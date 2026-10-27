#!/usr/bin/env python3
"""
M1.7-TEL / B1.5：按 fetch-report.json 登记的 fileId 重取电信原始二进制（复现通道）。

与 telecom_fetch.py 的区别：fetch 走**当时门端列表**（随时间滚动，不可复现）；
refetch 走**已入库报告登记的 fileId**（uuid/fileId 稳定），用于语料复现与哈希复核。

纪律：
  - 只写 files/（gitignored 暂存）；不触碰 sources/、fetch-report.json 等已入库文件；
  - 每文件双重校验：md5(本地) vs 平台登记 keyNum.md5；sha256(本地) vs 首轮实测 sha256；
  - 首轮 MD5_MISMATCH 的 2 件（平台登记指纹与分发内容不一致——真实数据质量发现 GT-T17）
    按 sha256+size 双记录比对；若本次重取 sha256 与首轮一致，则平台内容未替换；
  - 任何新漂移 → HASH_DRIFT 状态显式登记，不静默沿用旧值；
  - 报告 refetch-report.json 确定性（无时间戳；重跑逐字节一致）。

用法：
  python3 scripts/telecom_refetch.py
"""
import hashlib
import json
import os
import re
import urllib.parse
import urllib.request

BASE = "https://txzbqy.miit.gov.cn"
ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "real-corpus", "telecom")
FILES = os.path.join(ROOT, "files")
UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"


def _req(path, payload=None, timeout=60):
    url = f"{BASE}{path}"
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(url, data=data, method="POST" if data else "GET")
    req.add_header("Content-Type", "application/json")
    req.add_header("User-Agent", UA)
    req.add_header("Referer", f"{BASE}/")
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read()


def refetch_one(rec):
    """按登记 fileId 重取单文件并校验。返回结果 dict（确定性字段）。"""
    fid = rec["fileId"]
    out = {
        "fileId": fid,
        "fileName": rec.get("fileName"),
        "size_first_fetch": rec.get("size_actual"),
        "md5_platform": rec.get("md5_platform"),
        "sha256_first_fetch": rec.get("sha256"),
        "first_fetch_status": rec.get("status"),
    }
    try:
        meta = json.loads(_req(f"/zbtb/zwzt/file/getFileByKey/{fid}")[1]).get("file") or {}
        keynum = meta.get("keyNum")
        if not keynum or "@" not in keynum:
            out["status"] = "NO_KEYNUM"
            return out
        md5_exp, size_exp = keynum.split("@", 1)
        out["keynum_now"] = keynum
        tok = json.loads(_req("/zbtb/zwzt/file/getAccessToken", {"userId": "-1", "md5Paths": keynum})[1])
        token = (tok.get("accessToken") or [None])[0]
        if not token:
            out["status"] = "NO_TOKEN"
            return out
        qt = urllib.parse.quote(token, safe="")
        data = _req(f"/fileServer/file/download?&t={qt}", timeout=180)[1]
        md5_real = hashlib.md5(data).hexdigest()
        sha_real = hashlib.sha256(data).hexdigest()
        out["size_actual"] = len(data)
        out["md5_actual"] = md5_real
        out["sha256_actual"] = sha_real
        # 校验 1：与首轮实测 sha256（内容是否被平台替换）
        out["sha256_match_first_fetch"] = sha_real == rec.get("sha256")
        # 校验 2：与平台登记 keyNum
        out["md5_match_platform"] = md5_real == md5_exp
        out["size_match_platform"] = len(data) == int(size_exp)
        if not out["sha256_match_first_fetch"]:
            out["status"] = "HASH_DRIFT"
        elif out["md5_match_platform"] and out["size_match_platform"]:
            out["status"] = "REFETCH_OK"
        else:
            # 与首轮同状态（平台登记指纹与内容不一致——GT-T17 真实数据质量发现）
            out["status"] = "REFETCH_OK_PLATFORM_MD5_MISMATCH" if rec.get("status") == "MD5_MISMATCH" else "MD5_MISMATCH_NEW"
        name = meta.get("fileName") or rec.get("fileName") or f"{fid}.bin"
        safe = re.sub(r"[\\/:*?\"<>|\s]", "_", name)[:120]
        out["saved"] = os.path.relpath(os.path.join(FILES, safe), ROOT)
        with open(os.path.join(FILES, safe), "wb") as f:
            f.write(data)
        return out
    except Exception as e:  # noqa: BLE001
        out["status"] = f"ERROR: {type(e).__name__}: {e}"[:200]
        return out


def main():
    os.makedirs(FILES, exist_ok=True)
    report_path = os.path.join(ROOT, "fetch-report.json")
    first = json.load(open(report_path, encoding="utf-8"))
    files = first.get("files") or []
    if not files:
        raise SystemExit("fetch-report.json 无 files 登记——先跑 telecom_fetch.py 建立首轮登记")
    results = []
    for rec in sorted(files, key=lambda x: x["fileId"]):
        r = refetch_one(rec)
        results.append(r)
        print(f"  [{r['status']}] {r.get('fileName','?')[:52]}")
    summary = {
        "total": len(results),
        "by_status": {},
        "all_match_first_fetch": all(r.get("sha256_match_first_fetch") for r in results),
    }
    for r in results:
        summary["by_status"][r["status"]] = summary["by_status"].get(r["status"], 0) + 1
    out = {"purpose": "B1.5 语料复现：按 fetch-report 登记 fileId 重取并双哈希校验（确定性报告，无时间戳）",
           "summary": summary, "files": results}
    dst = os.path.join(ROOT, "refetch-report.json")
    with open(dst, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, indent=1, sort_keys=True)
    print(f"报告: real-corpus/telecom/refetch-report.json  all_match={summary['all_match_first_fetch']}")
    if not summary["all_match_first_fetch"]:
        raise SystemExit("存在 HASH_DRIFT——平台已替换文件内容，需按新哈希重登记（不静默沿用旧值）")


if __name__ == "__main__":
    main()
