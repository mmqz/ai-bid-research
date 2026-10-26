#!/usr/bin/env python3
"""
M1.7-TEL A批次：工信部通信工程建设项目招标投标管理信息平台（txzbqy.miit.gov.cn）
真实通信行业招标公告/公示/招标文件 抓取与溯源落盘。

通道（全部免鉴权公开接口，2026-10-26 亲测）：
  1. POST /zbtb/gateway/gatewayPublicity/bidBulletinListDoor   -> 招标/变更公告（首页10条）
  2. POST /zbtb/gateway/gatewayPublicity/bidPublicityListDoor  -> 中标候选人/结果公示（首页10条）
  3. GET  /zbtb/gateway/gatewayPublicity/bidBulletinInfo?uuid=  -> 公告详情
  4. GET  /zbtb/gateway/gatewayPublicity/bidPublicityInfo?uuid= -> 公示详情
  5. GET  /zbtb/zwzt/file/getFileByKey/<fileId>                 -> 文件元数据(keyNum=md5@size)
  6. POST /zbtb/zwzt/file/getAccessToken {userId:-1,md5Paths}   -> 下载令牌
  7. GET  fileServer/file/download?&t=<token>                   -> 文件本体（md5 与 keyNum 前缀逐字节一致）

纪律：
  - 每个文件保留 来源URL/发布日期/项目编号/uuid/bidId/sha256/平台自报md5（keyNum）
  - 下载后校验 md5(本地) == keyNum.md5 且 size == keyNum.size，不一致判 FAIL 不入库
  - 幂等：已存在且哈希一致则跳过
"""
import json
import hashlib
import os
import re
import sys
import time
import urllib.request
import urllib.parse

BASE = "https://txzbqy.miit.gov.cn"
ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "real-corpus", "telecom")
SRC = os.path.join(ROOT, "sources")
FILES = os.path.join(ROOT, "files")
UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"


def _req(path, payload=None, method=None, timeout=30):
    url = f"{BASE}{path}"
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(url, data=data, method=method or ("POST" if data else "GET"))
    req.add_header("Content-Type", "application/json")
    req.add_header("User-Agent", UA)
    req.add_header("Referer", f"{BASE}/")
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read()


def api_json(path, payload=None):
    code, body = _req(path, payload)
    return json.loads(body)


def api_bytes(path, payload=None, timeout=120):
    code, body = _req(path, payload, timeout=timeout)
    return body


def html_to_text(html):
    if not html:
        return ""
    txt = re.sub(r"<(script|style)[^>]*>.*?</\1>", " ", html, flags=re.S | re.I)
    txt = re.sub(r"<br\s*/?>", "\n", txt, flags=re.I)
    txt = re.sub(r"</(p|div|tr|table|h\d)>", "\n", txt, flags=re.I)
    txt = re.sub(r"<[^>]+>", "", txt)
    txt = txt.replace("&nbsp;", " ").replace("&ldquo;", "\u201c").replace("&rdquo;", "\u201d")
    txt = txt.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
    txt = re.sub(r"[ \t\u3000]+", " ", txt)
    txt = re.sub(r"\n\s*\n+", "\n", txt)
    return txt.strip()


def sha256_of(data):
    return hashlib.sha256(data).hexdigest()


def fetch_file(file_id, allow_large=True):
    """fileId -> 元数据 + 下载本体（md5 校验）。返回 dict 或 None。"""
    meta = api_json(f"/zbtb/zwzt/file/getFileByKey/{file_id}").get("file") or {}
    keynum = meta.get("keyNum")
    if not keynum or "@" not in keynum:
        return {"fileId": file_id, "status": "NO_KEYNUM", "meta": meta}
    md5_exp, size_exp = keynum.split("@", 1)
    size_exp = int(size_exp)
    if not allow_large and size_exp > 30 * 1024 * 1024:
        return {"fileId": file_id, "status": "SKIP_LARGE", "meta": meta}
    tok = api_json("/zbtb/zwzt/file/getAccessToken", {"userId": "-1", "md5Paths": keynum})
    token = (tok.get("accessToken") or [None])[0]
    if not token:
        return {"fileId": file_id, "status": "NO_TOKEN", "meta": meta}
    qt = urllib.parse.quote(token, safe="")
    data = api_bytes(f"/fileServer/file/download?&t={qt}")
    md5_real = hashlib.md5(data).hexdigest()
    ok = (md5_real == md5_exp) and (len(data) == size_exp)
    name = meta.get("fileName") or f"{file_id}.bin"
    safe = re.sub(r"[\\/:*?\"<>|\s]", "_", name)[:120]
    path = os.path.join(FILES, safe)
    with open(path, "wb") as f:
        f.write(data)
    return {
        "fileId": file_id,
        "status": "OK" if ok else "MD5_MISMATCH",
        "fileName": name,
        "keyNum": keynum,
        "size_expected": size_exp,
        "size_actual": len(data),
        "md5_platform": md5_exp,
        "md5_actual": md5_real,
        "sha256": sha256_of(data),
        "saved": os.path.relpath(path, ROOT),
        "download_url": f"{BASE}/fileServer/file/download (token flow, keyNum={keynum})",
        "meta": meta,
    }


def main():
    os.makedirs(SRC, exist_ok=True)
    os.makedirs(FILES, exist_ok=True)
    fetched_at = time.strftime("%Y-%m-%dT%H:%M:%S+08:00")
    report = {"fetched_at": fetched_at, "platform": BASE, "nodes": [], "files": []}

    # ---- 1. 门端列表（招标/变更公告 + 中标公示） ----
    bull = api_json("/zbtb/gateway/gatewayPublicity/bidBulletinListDoor", {"page": 1, "limit": 10})
    publ = api_json("/zbtb/gateway/gatewayPublicity/bidPublicityListDoor", {"page": 1, "limit": 10})
    with open(os.path.join(SRC, "bulletin-door.json"), "w") as f:
        json.dump(bull, f, ensure_ascii=False, indent=1)
    with open(os.path.join(SRC, "publicity-door.json"), "w") as f:
        json.dump(publ, f, ensure_ascii=False, indent=1)

    nodes = []
    # ---- 2. 公告详情 ----
    for x in bull["page"]["list"]:
        det = api_json(f"/zbtb/gateway/gatewayPublicity/bidBulletinInfo?uuid={x['uuid']}").get("data") or {}
        node = {
            "node_type": "bulletin",
            "uuid": x["uuid"],
            "bidId": x.get("bidId"),
            "bidPackageId": x.get("bidPackageId"),
            "title": det.get("bulletinTitle") or x.get("bulletinTitle"),
            "bulletinType": det.get("bulletinType"),
            "issueDate": det.get("issueDate"),
            "tenderFileId": det.get("tenderFileId"),
            "fileId": det.get("fileId"),
            "source_url": f"{BASE}/#/bid/releaseInfoDetails?uuid={x['uuid']}",
            "api_url": f"{BASE}/zbtb/gateway/gatewayPublicity/bidBulletinInfo?uuid={x['uuid']}",
            "body_text_len": len(html_to_text(det.get("bulletinComment", ""))),
        }
        nodes.append(node)
        fn = f"bulletin-{x['uuid'][:8]}.json"
        det["_plain_text"] = html_to_text(det.get("bulletinComment", ""))
        with open(os.path.join(SRC, fn), "w") as f:
            json.dump(det, f, ensure_ascii=False, indent=1)
        node["source_file"] = f"sources/{fn}"

    # ---- 3. 公示详情 ----
    for x in publ["page"]["list"]:
        det = api_json(f"/zbtb/gateway/gatewayPublicity/bidPublicityInfo?uuid={x['uuid']}").get("data") or {}
        node = {
            "node_type": "publicity",
            "uuid": x["uuid"],
            "bidId": det.get("bidId") or x.get("bidId"),
            "bidPackageId": det.get("bidPackageId") or x.get("bidPackageId"),
            "title": det.get("publicityTitle") or x.get("publicityTitle"),
            "publicityType": det.get("publicityType"),
            "publishTime": det.get("publishTime"),
            "fileId": det.get("fileId"),
            "source_url": f"{BASE}/#/bid/publicityInfoDetails?uuid={x['uuid']}",
            "api_url": f"{BASE}/zbtb/gateway/gatewayPublicity/bidPublicityInfo?uuid={x['uuid']}",
        }
        aw = det.get("bidAwardInfoTEntitys") or []
        node["award_count"] = len(aw)
        if aw:
            node["award_first"] = {k: aw[0].get(k) for k in ("bidderName", "bidPrice", "bidderSort", "publicityResult") if k in aw[0]}
        nodes.append(node)
        fn = f"publicity-{x['uuid'][:8]}.json"
        det["_plain_text"] = html_to_text(det.get("publicityComment", ""))
        with open(os.path.join(SRC, fn), "w") as f:
            json.dump(det, f, ensure_ascii=False, indent=1)
        node["source_file"] = f"sources/{fn}"

    report["nodes"] = nodes

    # ---- 4. 文件下载（招标文件 + 变更公告附件；其余仅元数据） ----
    want_download = set()
    for n in nodes:
        if n.get("tenderFileId"):
            want_download.add(("tender", n["tenderFileId"], n["title"]))
        for fid in (n.get("fileId") or "").split(","):
            if fid.strip():
                want_download.add(("attach", fid.strip(), n["title"]))

    seen = set()
    for kind, fid, title in sorted(want_download):
        if fid in seen:
            continue
        seen.add(fid)
        try:
            r = fetch_file(fid, allow_large=True)
        except Exception as e:
            r = {"fileId": fid, "status": f"ERROR: {e}"}
        r["kind"] = kind
        r["linked_title"] = title[:60]
        report["files"].append(r)
        print(f"[{r['status']}] {kind} {fid[:8]} {r.get('fileName', '')[:50]}")
        time.sleep(0.5)

    with open(os.path.join(ROOT, "fetch-report.json"), "w") as f:
        json.dump(report, f, ensure_ascii=False, indent=1)

    ok = sum(1 for r in report["files"] if r.get("status") == "OK")
    print(f"\n节点: {len(nodes)}（bulletin {sum(1 for n in nodes if n['node_type']=='bulletin')} / publicity {sum(1 for n in nodes if n['node_type']=='publicity')}）")
    print(f"文件: {ok} OK / {len(report['files'])} 总计")
    print(f"报告: real-corpus/telecom/fetch-report.json")


if __name__ == "__main__":
    main()
