#!/usr/bin/env python3
"""
M1.7-TEL 文件级准备：
  1. 解压招标文件 zip（联通软研院 / 电信河北 4 标包 / 贵州移动 4 标包 / 重庆西永 / 电信淮安）
  2. LibreOffice 无头转换 .doc/.xls/.ppt 等旧格式 -> .docx/.xlsx（转换边界登记，转换前后 sha256 双留痕）
  3. 输出 converted-manifest.json（原文件 sha256 + 转换产物 sha256 + LO 版本）

纪律：转换不改变原件（原件在 files/ 只读）；转换产物只用于解析测试；
      每个产物记录 origin（哪个真实文件转换而来）。
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys
import zipfile

ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "real-corpus", "telecom")
FILES = os.path.join(ROOT, "files")
EXTRACTED = os.path.join(ROOT, "extracted")
CONVERTED = os.path.join(ROOT, "converted")


def sha256_file(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def unzip_all():
    """解压 files/ 下所有 zip 到 extracted/<stem>/。rar 不解（无 unrar 工具则登记）。"""
    out = []
    for fn in sorted(os.listdir(FILES)):
        p = os.path.join(FILES, fn)
        if fn.lower().endswith(".zip"):
            stem = os.path.splitext(fn)[0][:60]
            dest = os.path.join(EXTRACTED, stem)
            os.makedirs(dest, exist_ok=True)
            try:
                with zipfile.ZipFile(p) as z:
                    if z.testzip() is not None:
                        out.append({"zip": fn, "status": "CORRUPT"})
                        continue
                    z.extractall(dest)
                out.append({"zip": fn, "status": "OK", "dest": os.path.relpath(dest, ROOT), "entries": len(os.listdir(dest))})
            except Exception as e:
                out.append({"zip": fn, "status": f"ERROR: {e}"})
        elif fn.lower().endswith(".rar"):
            out.append({"zip": fn, "status": "RAR_NOT_EXTRACTED", "note": "rar 保留原始件，本环境无 unrar；LO 可直接读 rar 内单文件吗——不可，登记"})
    return out


def lo_version():
    try:
        r = subprocess.run(["libreoffice", "--version"], capture_output=True, text=True, timeout=60)
        return r.stdout.strip().split("\n")[0]
    except Exception as e:
        return f"ERR {e}"


def convert_legacy():
    """LO 转换 .doc/.xls -> .docx/.xlsx（逐个文件，收集产物）。"""
    manifest = []
    srcs = []
    # files/ 顶层 + extracted/ 递归
    for fn in sorted(os.listdir(FILES)):
        p = os.path.join(FILES, fn)
        if os.path.isfile(p) and os.path.splitext(fn)[1].lower() in (".doc", ".xls"):
            srcs.append(p)
    for root, _dirs, fs in os.walk(EXTRACTED):
        for f in fs:
            if os.path.splitext(f)[1].lower() in (".doc", ".xls", ".ppt"):
                srcs.append(os.path.join(root, f))

    lo_out = os.path.join(ROOT, "lo-tmp")
    os.makedirs(lo_out, exist_ok=True)
    os.makedirs(CONVERTED, exist_ok=True)

    for src in srcs:
        ext = os.path.splitext(src)[1].lower()
        target_ext = {".doc": ".docx", ".xls": ".xlsx", ".ppt": ".pptx"}[ext]
        base = os.path.splitext(os.path.basename(src))[0]
        # 产物名：短哈希防撞名防超长
        rel = os.path.relpath(src, ROOT)
        short = hashlib.sha256(rel.encode()).hexdigest()[:10]
        prod = os.path.join(CONVERTED, f"{short}-{base[:40]}{target_ext}")
        if os.path.exists(prod):
            manifest.append({"src": os.path.relpath(src, ROOT), "product": os.path.relpath(prod, ROOT),
                             "status": "SKIP_EXISTS", "src_sha256": sha256_file(src), "product_sha256": sha256_file(prod)})
            continue
        r = subprocess.run(
            ["libreoffice", "--headless", "--norestore", "--convert-to", target_ext[1:], "--outdir", lo_out, src],
            capture_output=True, text=True, timeout=180)
        produced = os.path.join(lo_out, base + target_ext)
        if os.path.exists(produced):
            shutil.move(produced, prod)
            manifest.append({"src": os.path.relpath(src, ROOT), "product": os.path.relpath(prod, ROOT),
                             "status": "OK", "src_sha256": sha256_file(src), "product_sha256": sha256_file(prod),
                             "lo_rc": r.returncode})
        else:
            manifest.append({"src": os.path.relpath(src, ROOT), "status": "CONVERT_FAIL",
                             "lo_rc": r.returncode, "lo_stderr": (r.stderr or "")[-300:]})
    return manifest


def main():
    os.makedirs(EXTRACTED, exist_ok=True)
    os.makedirs(CONVERTED, exist_ok=True)
    rep = {
        "lo_version": lo_version(),
        "unzip": unzip_all(),
        "convert": convert_legacy(),
    }
    ok = sum(1 for c in rep["convert"] if c.get("status") in ("OK", "SKIP_EXISTS"))
    print(f"解压 zip: {sum(1 for u in rep['unzip'] if u['status']=='OK')} OK")
    for u in rep["unzip"]:
        print(' ', u.get("status"), u.get("zip", "")[:60])
    print(f"LO 转换: {ok}/{len(rep['convert'])}")
    for c in rep["convert"]:
        print(' ', c.get("status"), '|', c.get("src", "")[:70])
    with open(os.path.join(ROOT, "converted-manifest.json"), "w") as f:
        json.dump(rep, f, ensure_ascii=False, indent=1)
    print("报告: real-corpus/telecom/converted-manifest.json")


if __name__ == "__main__":
    main()
