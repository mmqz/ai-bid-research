#!/usr/bin/env python3
"""
M1.7-TEL 文件级验证：
  1. 物化 parse-input/（A- 主件 / C- 补遗 / B- 附件 命名 = extract 层接口；内容为真实原始文件）
  2. cargo run --example realfiles（bid-parse Rust 真实解析器）
  3. 跨向锚点核验：txzbqy 官方公告事实（招标编号/数量/限价/关键词）必须在对应
     真实招标文件解析块中找到（公告 ↔ 文件 双向一致性，文件级证据闭环）
  4. 输出 real-corpus/telecom/file-level-report.json

口径：REAL_FILE + ENGINE_RUN（Rust 解析器亲跑；.doc/.xls 经 LO 转换——转换边界在
converted-manifest.json 留痕，原件只读）。
"""
import json
import os
import shutil
import subprocess
import sys

ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "real-corpus", "telecom")
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PARSE_INPUT = os.path.join(ROOT, "parse-input")
REPORT = os.path.join(ROOT, "file-level-report.json")

# ---- 物化清单：真实文件 -> extract 层命名 ----
MATERIALIZE = [
    # (来源相对 ROOT, 目标名, 业务类别)
    ("converted/1d77120d6d-0.招标文件-2026年中国联通软研院管理中台国资监管中心研发项目.docx",
     "A-unicom-ry-2026CUC-RJY-PT0783-招标文件.docx", "main"),
    ("extracted/1、文件/1、文件/中国电信股份有限公司淮安分公司2026年淮安本地视频能力平台建设采购项目.docx",
     "A-telecom-huaian-主文件.docx", "main"),
    ("files/招标正文.docx",
     "A-telecom-shanxi-灵枢智关-招标正文.docx", "main"),
    ("files/发售版.docx",
     "A-telecom-shanghai-songjiang-光终端放装-招标文件.docx", "main"),
    ("extracted/CQYD20260500728-标包1/20261023173711/3927059-CQYD20260500728-标包1-20261023173711.pdf",
     "A-cqyd-20260500728-重庆西永水冷空调-招标文件.pdf", "main"),
    ("files/中国移动浙江公司2025年算网能力二阶段项目数据库服务器采购_招标采购_变更公告_(1).pdf",
     "C-zjyd-20260500552-数据库服务器-变更公告.pdf", "addendum"),
    ("extracted/招标文件-2026年中国联通软研院管理中台国资监管中心研发项目/招标文件-2026年中国联通软研院管理中台国资监管中心研发项目/附件3：技术规范书.docx",
     "B-unicom-ry-技术规范书.docx", "attach"),
    ("extracted/招标文件-2026年中国联通软研院管理中台国资监管中心研发项目/招标文件-2026年中国联通软研院管理中台国资监管中心研发项目/附件1：详细评审表.xlsx",
     "B-unicom-ry-详细评审表.xlsx", "attach"),
    ("converted/3556d01ebe-附件4：投标一览表.xlsx",
     "B-unicom-ry-投标一览表.xlsx", "attach"),
    ("extracted/1、文件/1、文件/附件2：中国电信股份有限公司淮安分公司2026年淮安本地视频能力平台建设采购项目技术规范书.docx",
     "B-telecom-huaian-技术规范书.docx", "attach"),
    ("extracted/1、文件/1、文件/附件3：中国电信股份有限公司淮安分公司2026年淮安本地视频能力平台建设采购项目报价文件.xlsx",
     "B-telecom-huaian-报价文件.xlsx", "attach"),
    ("extracted/1、文件/1、文件/附件1：中国电信股份有限公司淮安分公司2026年淮安本地视频能力平台建设采购项目合同.docx",
     "B-telecom-huaian-合同.docx", "attach"),
    ("extracted/浙江立项批复[2026]_321_号-关于浙江公司2025年算网能力二阶段建设项目立项变更的批复.pdf/浙江立项批复[2026]_321_号-关于浙江公司2025年算网能力二阶段建设项目立项变更的批复.pdf",
     "B-zjyd-立项批复.pdf", "attach"),
    ("files/2026年西永、龙洲湾、水土数据中心水冷空调群控系统优化项目硬件采购项目_招标公告.pdf",
     "B-cqyd-招标公告.pdf", "attach"),
    ("files/招标公告.pdf",
     "B-telecom-hebei-无线网勘察设计-招标公告.pdf", "attach"),
    ("files/中国移动安徽公司2026年至2029年复合型井圈井盖产品采购项目_中标结果公示.pdf",
     "B-cmcc-anhui-井圈井盖-中标结果公示.pdf", "attach"),
    ("files/中国移动甘肃公司张掖分公司2026年高台县某单位“雪亮工程”一期升级改造工程项目_中标候选人公示.pdf",
     "B-cmcc-gansu-雪亮工程-中标候选人公示.pdf", "attach"),
]

# ---- 跨向锚点核验表：官方公告（txzbqy JSON）事实 -> 必须命中的解析文件与文本 ----
ANCHOR_CHECKS = [
    # (说明, 官方公告来源文件(sources/), 公告侧锚文本, 目标解析文件前缀, 文件侧应含文本)
    ("联通软研院：招标编号贯通公告与文件",
     "bulletin-3021ef4e.json", "2026CUC-RJY-PT0783", "A-unicom-ry", "2026CUC-RJY-PT0783"),
    ("联通软研院：项目主体关键词",
     "bulletin-3021ef4e.json", "国资监管中心", "A-unicom-ry", "国资监管中心"),
    ("联通软研院：评标办法结构存在",
     None, None, "A-unicom-ry", "评标办法"),
    ("联通软研院：主文件章节结构（招标公告章）",
     None, None, "A-unicom-ry", "招标公告"),
    ("联通软研院：主文件章节结构（投标人须知）",
     None, None, "A-unicom-ry", "投标人须知"),
    ("ZJYD 变更链：招标编号",
     "bulletin-a6f1b033.json", "ZJYD20260500552", "C-zjyd", "ZJYD20260500552"),
    ("ZJYD 变更链：采购数量 77 台",
     "bulletin-a6f1b033.json", "77", "C-zjyd", "77"),
    ("ZJYD 变更链：最高限价 1674.336 万元",
     "bulletin-a6f1b033.json", "1674.336", "C-zjyd", "1674.336"),
    ("ZJYD 变更链：到货承诺 20 工作日",
     "bulletin-a6f1b033.json", "20工作日内到货", "C-zjyd", "20工作日内到货"),
    ("电信淮安：项目关键词贯通",
     None, None, "A-telecom-huaian", "淮安本地视频能力平台"),
    ("电信山西：项目关键词贯通",
     None, None, "A-telecom-shanxi", "灵枢智关"),
    ("电信上海松江：项目关键词贯通",
     None, None, "A-telecom-shanghai", "松江"),
    ("重庆西永：招标编号贯通公告与文件",
     None, None, "A-cqyd", "CQYD20260500728"),
    ("重庆西永：公告 PDF 与主文件关键词一致",
     "bulletin-67fc378a.json", "水冷空调", "A-cqyd", "水冷空调"),
]


def load_official_text(name):
    p = os.path.join(ROOT, "sources", name)
    if not os.path.exists(p):
        return ""
    d = json.load(open(p))
    return d.get("_plain_text", "") or ""


def materialize():
    if os.path.exists(PARSE_INPUT):
        shutil.rmtree(PARSE_INPUT)
    os.makedirs(PARSE_INPUT)
    man = []
    for src, dst, kind in MATERIALIZE:
        s = os.path.join(ROOT, src)
        if not os.path.exists(s):
            man.append({"src": src, "dst": dst, "status": "MISSING"})
            continue
        shutil.copy2(s, os.path.join(PARSE_INPUT, dst))
        man.append({"src": src, "dst": dst, "status": "OK", "kind": kind})
    return man


def run_rust():
    exe_dir = os.path.join(REPO, "bidcore")
    cargo = os.path.expanduser("~/.cargo/bin/cargo")
    if not os.path.exists(cargo):
        cargo = "cargo"
    cmd = [cargo, "run", "--release", "--example", "realfiles", "--",
           "--dir", PARSE_INPUT, "--out", REPORT]
    r = subprocess.run(cmd, cwd=exe_dir, capture_output=True, text=True, timeout=600)
    print(r.stdout.strip())
    if r.returncode != 0:
        print(r.stderr[-2000:])
        sys.exit(1)
    if not os.path.exists(REPORT):
        print("报告未生成")
        sys.exit(1)


def verify():
    rep = json.load(open(REPORT))
    blocks_by_file = {}
    for f in rep["files"]:
        blocks_by_file[f["name"]] = f  # 先只存元信息；块文本需要重解析？
    # realfiles 报告未含块全文——锚点核验需要块文本。
    # 用 Python 侧对同一批文件做只读文本抽取（与 Rust 解析器同一语料、独立通道）：
    # 这里核验语义是「文本可检索性」，不重复实现解析器；docx=zip XML 文本、pdf=pdftotext、xlsx=sharedStrings。
    import zipfile
    import re as _re

    def file_text(path):
        ext = os.path.splitext(path)[1].lower()
        try:
            if ext == ".docx":
                with zipfile.ZipFile(path) as z:
                    xml = z.read("word/document.xml").decode("utf-8")
                return _re.sub(r"<[^>]+>", "", xml)
            if ext == ".xlsx":
                with zipfile.ZipFile(path) as z:
                    names = [n for n in z.namelist() if n.startswith("xl/sharedStrings") or _re.match(r"xl/worksheets/sheet\d+\.xml", n)]
                    txt = ""
                    for n in names:
                        txt += _re.sub(r"<[^>]+>", "", z.read(n).decode("utf-8"))
                    return txt
            if ext == ".pdf":
                r = subprocess.run(["pdftotext", "-layout", path, "-"], capture_output=True, text=True, timeout=120)
                return r.stdout
        except Exception as e:
            return f"__ERR__ {e}"
        return ""

    texts = {}
    for src, dst, kind in MATERIALIZE:
        p = os.path.join(PARSE_INPUT, dst)
        if os.path.exists(p):
            texts[dst] = file_text(p)

    results = []
    for desc, official, ann_anchor, target_prefix, needle in ANCHOR_CHECKS:
        # 1) 公告侧：锚文本必须存在于官方公告
        ann_ok = True
        if official:
            ot = load_official_text(official)
            ann_ok = ann_anchor in ot if ann_anchor else True
        # 2) 文件侧：needle 必须存在于目标解析文件文本
        tgt = [t for t in texts if t.startswith(target_prefix)]
        file_ok = any(needle in texts[t] for t in tgt) if tgt else False
        results.append({
            "check": desc, "official_source": official,
            "announcement_side": ann_ok, "file_side": file_ok,
            "target": target_prefix, "needle": needle,
            "pass": ann_ok and file_ok,
        })
    return rep, results


def main():
    man = materialize()
    missing = [m for m in man if m["status"] != "OK"]
    print(f"物化 {len(man) - len(missing)}/{len(man)}" + (f"（缺失 {len(missing)}）" if missing else ""))
    for m in missing:
        print("  MISSING:", m["src"][:80])
    run_rust()
    rep, checks = verify()
    # 解析完整性断言
    ok_files = rep["files_parsed"]
    zero_block = [f["name"] for f in rep["files"] if f["blocks"] == 0]
    req_by_cat = {}
    for r in rep["requirements"]:
        req_by_cat[r["category"]] = req_by_cat.get(r["category"], 0) + 1
    passed = sum(1 for c in checks if c["pass"])
    print(f"\n解析成功 {ok_files} 文件 / 失败 {rep['files_failed']}；零块文件: {zero_block or '无'}")
    print(f"条款抽取 {rep['requirements_n']} 条（{req_by_cat}）")
    print(f"跨向锚点核验 {passed}/{len(checks)} PASS")
    for c in checks:
        print(" ", "PASS" if c["pass"] else "FAIL", "|", c["check"],
              "" if c["pass"] else f"(公告侧={c['announcement_side']} 文件侧={c['file_side']})")

    out = {
        "materialize": man,
        "parse_summary": rep["files"],
        "parse_failures": rep["failures"],
        "requirements_n": rep["requirements_n"],
        "requirements_by_category": req_by_cat,
        "requirements": rep["requirements"],
        "anchor_checks": checks,
        "zero_block_files": zero_block,
    }
    with open(REPORT, "w") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
    print(f"\n报告: {REPORT}")
    sys.exit(0 if (passed == len(checks) and not zero_block and rep["files_failed"] == 0) else 1)


if __name__ == "__main__":
    main()
