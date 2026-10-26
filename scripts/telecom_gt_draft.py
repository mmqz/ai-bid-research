#!/usr/bin/env python3
"""
M1.7-TEL C批次：通信行业真实样本 GT 草案（24→N 条，四要素纪律）

四要素：原文定位核验（excerpt 对回源文本三级匹配：verbatim/normalized/keyword）
       + 标准答案（expected）+ 判定理由（rationale）+ 复核状态（draft_pending_human）

纪律：
  - 不凑数：每条必须有可验证的真实原文锚
  - excerpt 逐字取自源材料（源=txzbqy 官方 JSON 正文 / 字节存档镜像文本 / 已解析真实文件）
  - 定位失败即 FAIL 修正，不放过关
  - 机器只做预核，冻结权在人工（review_status=draft_pending_human）
"""
import json
import os
import re
import sys
import unicodedata

ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "real-corpus", "telecom")
GT_DIR = os.path.join(ROOT, "gt")
SRC = os.path.join(ROOT, "sources")
PARSE_INPUT = os.path.join(ROOT, "parse-input")


def norm(s):
    """N1 归一化（对齐 bid-parse norm_n1 语义）：NFKC + 去空白。"""
    return re.sub(r"\s+", "", unicodedata.normalize("NFKC", s))


def source_text(rel):
    """按 source_file 读全文（txt/json 自动处理）。"""
    if rel.startswith("sources/"):
        fn = os.path.join(ROOT, rel)
        if fn.endswith(".json"):
            d = json.load(open(fn))
            return d.get("_plain_text") or json.dumps(d, ensure_ascii=False)
        return open(fn, encoding="utf-8").read()
    if rel.startswith("parse-input/"):
        p = os.path.join(ROOT, rel)
        return open(p, "rb").read().decode("utf-8", errors="ignore")  # 二进制占位（GT 不用此通道）
    return open(os.path.join(ROOT, rel), encoding="utf-8").read()


def file_text_for_locate(rel):
    """定位通道：docx/xlsx 用压缩 XML 文本；pdf 用 pdftotext；txt/json 直读。"""
    p = os.path.join(ROOT, rel)
    ext = os.path.splitext(rel)[1].lower()
    if rel.startswith("sources/") and rel.endswith((".txt", ".json")):
        return source_text(rel)
    if ext == ".pdf":
        import subprocess
        r = subprocess.run(["pdftotext", "-layout", p, "-"], capture_output=True, text=True, timeout=120)
        return r.stdout
    if ext in (".docx", ".xlsx"):
        import zipfile
        with zipfile.ZipFile(p) as z:
            txt = ""
            for n in z.namelist():
                if n.endswith(".xml") and ("document" in n or "sharedStrings" in n or re.match(r"xl/worksheets/sheet\d+\.xml", n)):
                    txt += re.sub(r"<[^>]+>", "", z.read(n).decode("utf-8"))
            return txt
    return source_text(rel)


def locate(excerpt, text):
    """三级匹配：verbatim（原文含）→ normalized（NFKC 归一化含）→ keyword（关键词集）。"""
    if excerpt in text:
        return "verbatim", None
    if norm(excerpt) in norm(text):
        return "normalized", None
    kws = [w for w in re.split(r"[^\w\u4e00-\u9fff]+", excerpt) if len(w) >= 2]
    hit = sum(1 for w in kws if w in text)
    if kws and hit >= max(2, len(kws) // 2):
        return "keyword", f"{hit}/{len(kws)}"
    return "FAIL", f"{hit}/{len(kws)}" if kws else "no-kw"


GT_ITEMS = [
    # ---------------- 链A：联通北京业务流程 HX-BFZB-2026035（第二次招标公告 + 公示） ----------------
    dict(gt_id="GT-T01", chain="unicom-bj-bpm", category="项目标识",
         source_file="sources/unicom-bj-bpm-2nd-tender-20260918.txt",
         source_url="https://bj.zhiliaobiaoxun.com/article/113463125（镜像；法定媒介：联通采购与招标网/txzbqy/cebpubservice）",
         excerpt="2026-2028年中国联通北京业务流程运营平台研发集中采购项目(第二次)(招标编号:HX-BFZB-2026035)",
         expected="HX-BFZB-2026035",
         rationale="四节点链跨轮次主键（招标编号），第一次公告、失败公告、第二次公告、中标候选人公示四节点逐字一致；与平台内部 bidId 按轮次隔离形成对照——电信行业轮次间关联键是招标编号而非平台主键",
         verify="engine"),
    dict(gt_id="GT-T02", chain="unicom-bj-bpm", category="商务要求",
         source_file="sources/unicom-bj-bpm-2nd-tender-20260918.txt",
         source_url="https://bj.zhiliaobiaoxun.com/article/113463125",
         excerpt="中级服务不含税单价最高限价681.00元/人天，高级服务不含税单价最高限价1136.00元/人天",
         expected="中级 681.00 元/人天；高级 1136.00 元/人天（不含税单价双档限价）",
         rationale="单价双档限价是报价否决线（高于即否决）；两轮招标该条款逐字未变（不变陷阱），且是 P3 输出复验中中标候选人报价合规判定（670<681、1130<1136）的基准",
         verify="engine"),
    dict(gt_id="GT-T03", chain="unicom-bj-bpm", category="时点",
         source_file="sources/unicom-bj-bpm-2nd-tender-20260918.txt",
         source_url="https://bj.zhiliaobiaoxun.com/article/113463125",
         excerpt="投标文件递交截止时间（即投标截止时间）：2026年10月14日09时30分（北京时间）。",
         expected="2026-10-14 09:30",
         rationale="第二次招标的投标截止/开标时点；与中标候选人公示正文自证时间线「2026年10月14日进行了开标」跨文件逐字吻合（输出复验锚点）；相对第一次公告的 09-03 09:30 为变更项",
         verify="engine"),
    dict(gt_id="GT-T04", chain="unicom-bj-bpm", category="资格条件",
         source_file="sources/unicom-bj-bpm-2nd-tender-20260918.txt",
         source_url="https://bj.zhiliaobiaoxun.com/article/113463125",
         excerpt="单项业绩金额在15万元人民币（含）以上的数据分析或管理分析类软件开发或研发业绩",
         expected="≥1 个 2023-01-01 以来签订的、单项金额 ≥15 万元（含税）的数据分析/管理分析类软件开发或研发业绩（附合同+订单+发票）",
         rationale="业绩门槛条款（2023-01-01 起算、含税口径、证明材料清单）；两轮一致为不变陷阱；全角括号（含）为真实文档形态，考察抽取器对全角标点的容忍",
         verify="engine"),
    dict(gt_id="GT-T05", chain="unicom-bj-bpm", category="结果事实",
         source_file="sources/publicity-64b8406b.json",
         source_url="https://txzbqy.miit.gov.cn（工信部通信工程建设项目招标投标管理信息平台，uuid=64b8406b95984e06806d7235cc6b6e49）",
         excerpt="第一名：北京凡得科技有限公司；",
         expected="第一名=北京凡得科技有限公司（中级 670.00 元/人天、高级 1130.00 元/人天）",
         rationale="txzbqy 官方公示正文事实；结构化实体 bidAwardInfoTEntitys 为空数组——公示数据全部承载于正文文本，正文解析不可替代的实证；报价双档均低于限价（P3 合规）",
         verify="engine"),
    dict(gt_id="GT-T06", chain="unicom-bj-bpm", category="承诺响应",
         source_file="sources/publicity-64b8406b.json",
         source_url="https://txzbqy.miit.gov.cn（uuid=64b8406b95984e06806d7235cc6b6e49）",
         excerpt="保修期：自招标人签发《终验合格证书》之日起36个月。",
         expected="36 个月",
         rationale="中标候选人承诺保修 36 个月 ≥ 公告要求「不少于24个月」——承诺优于要求而非冲突；此类条款在合规矩阵中应为「响应达标+超额承诺」而非矛盾项，考察影响面引擎的语义判定",
         verify="engine"),
    dict(gt_id="GT-T07", chain="unicom-bj-bpm", category="流程事件",
         source_file="sources/unicom-bj-bpm-fail-20260908-snapshot.json",
         source_url="https://bj.bidcenter.com.cn/zhaobiao/zbkeyw-22652-0-6-0.html（快照证据 SNAPSHOT_ONLY）",
         excerpt="招标失败说明:本项目开标后经评标委员会评审,通过初步评审的有效投标人不足3家,缺乏竞争性。依据《通信工程建设项目招标投标管理办法》第三十四条",
         expected="第一次招标失败；原因=有效投标人不足3家；依据=通信招标办法第三十四条",
         rationale="链中的失败节点（政采链没有的节点类型）：触发重新招标轮次；快照证据为 SNAPSHOT_ONLY 诚实登记，字节级原件须从法定媒介人工获取（人工事项）",
         verify="manual_snapshot"),
    # ---------------- 链B：浙江移动 ZJYD20260500552 变更链 ----------------
    dict(gt_id="GT-T08", chain="zjyd-20260500552", category="项目标识",
         source_file="sources/bulletin-a6f1b033.json",
         source_url="https://txzbqy.miit.gov.cn（uuid=a6f1b033370b…，frequency=1 变更公告）",
         excerpt="招标编号：ZJYD20260500552",
         expected="ZJYD20260500552",
         rationale="变更链主键；txzbqy 变更公告 frequency=1（第 1 次变更），正文自包含完整招标条款（变更后口径）",
         verify="engine"),
    dict(gt_id="GT-T09", chain="zjyd-20260500552", category="技术参数",
         source_file="sources/bulletin-a6f1b033.json",
         source_url="https://txzbqy.miit.gov.cn（uuid=a6f1b033370b…）",
         excerpt="采购77台通算服务器用于保障自主可控数据库按时上线",
         expected="77 台通算服务器（ARM 架构高性能服务器，非标产品）",
         rationale="采购规模事实；与变更公告 PDF 文件级锚点一致（77 台在 parse-input C- 文件 pdftotext 文本层可检——公告级与文件级双通道一致）",
         verify="engine"),
    dict(gt_id="GT-T10", chain="zjyd-20260500552", category="商务要求",
         source_file="sources/bulletin-a6f1b033.json",
         source_url="https://txzbqy.miit.gov.cn（uuid=a6f1b033370b…）",
         excerpt="项目不含税总价最高限价为：1674.336万元，报价超过最高限价将否决投标",
         expected="1674.336 万元（不含税总价最高限价）",
         rationale="总价限价否决线（★ 条款）；文件级 PDF 同值锚点核验 PASS（file-level-report anchor check）",
         verify="engine"),
    dict(gt_id="GT-T11", chain="zjyd-20260500552", category="商务要求",
         source_file="sources/bulletin-a6f1b033.json",
         source_url="https://txzbqy.miit.gov.cn（uuid=a6f1b033370b…）",
         excerpt="★卖方承诺收到招标人通知后20工作日内到货。",
         expected="20 工作日（收到招标人通知后）",
         rationale="★ 实质性到货承诺条款；公告级与文件级 PDF 逐字一致（含「20工作日」无「个」的真实措辞——全角/口语化措辞的抽取容忍考察）",
         verify="engine"),
    dict(gt_id="GT-T12", chain="zjyd-20260500552", category="商务要求",
         source_file="sources/bulletin-a6f1b033.json",
         source_url="https://txzbqy.miit.gov.cn（uuid=a6f1b033370b…）",
         excerpt="★从到货签收合格之后的36个月为产品的质量保证期。",
         expected="36 个月质量保证期（到货签收合格后起算）",
         rationale="★ 保修期条款；与到货承诺同为 ★ 实质性条款族——影响合同履约与投标成本测算",
         verify="engine"),
    dict(gt_id="GT-T13", chain="zjyd-20260500552", category="资格条件",
         source_file="sources/bulletin-a6f1b033.json",
         source_url="https://txzbqy.miit.gov.cn（uuid=a6f1b033370b…）",
         excerpt="所投服务器产品需取得有效期内的中国国家强制性产品认证证书（CCC认证证书）",
         expected="CCC 认证证书（有效期内，含查询截屏）",
         rationale="产品级强制认证资格条款（3C）；引用 www.cnca.gov.cn 在线核查——资格审查面的可核查性设计",
         verify="engine"),
    dict(gt_id="GT-T14", chain="zjyd-20260500552", category="资格条件",
         source_file="sources/bulletin-a6f1b033.json",
         source_url="https://txzbqy.miit.gov.cn（uuid=a6f1b033370b…）",
         excerpt="不接受代理商投标（投标人须是投标产品的制造商，投标人须提供制造商声明、投标产品为投标人制造的承诺函）",
         expected="不接受联合体投标；不接受代理商投标（仅制造商）",
         rationale="双重排他资格条款（联合体+代理商），影响资格审查判定与竞争充分性——与链A的失败节点（不足3家）构成行业竞争面背景",
         verify="engine"),
    # ---------------- 文件级：联通软研院主招标文件（LO 转换后真实文本） ----------------
    dict(gt_id="GT-T15", chain="unicom-ry-2026CUC-RJY-PT0783", category="项目标识",
         source_file="parse-input/A-unicom-ry-2026CUC-RJY-PT0783-招标文件.docx",
         source_url="https://txzbqy.miit.gov.cn fileServer 下载（keyNum=9bb0eee7…@720313，md5 逐字节校验 OK）",
         excerpt="招标编号：2026CUC-RJY-PT0783",
         expected="2026CUC-RJY-PT0783",
         rationale="文件级真实招标编号——公告级标识（txzbqy uuid）与文件级标识（招标编号）分层实证；公告正文与招标文件本体双通道一致（anchor check PASS）",
         verify="engine"),
    dict(gt_id="GT-T16", chain="unicom-ry-2026CUC-RJY-PT0783", category="结构",
         source_file="parse-input/A-unicom-ry-2026CUC-RJY-PT0783-招标文件.docx",
         source_url="同 GT-T15",
         excerpt="第二章  投标人须知",
         expected="章节结构含：第一章 招标公告 / 第二章 投标人须知（前附表）/ 评标办法 / 合同条款等",
         rationale="真实电信招标文件的章节骨架（运营商范本）；.doc 旧格式经 LO 转换后结构保真（45,713 字符、章节锚齐全）——旧格式转换边界有效性的结构面证据",
         verify="engine"),
    # ---------------- 数据质量发现（真实性锚） ----------------
    dict(gt_id="GT-T17", chain="telecom-data-quality", category="数据质量",
         source_file="fetch-report.json",
         source_url="https://txzbqy.miit.gov.cn fileServer（GZYD20260500294 / 电信河北无线网勘察设计）",
         excerpt="GZYD20260500294-标包1-4-261024.zip",
         expected="2 个文件（GZYD20260500294 / 电信河北招标文件.zip）平台登记 md5 与实测 md5 不一致，但尺寸逐字节相等",
         rationale="真实世界数据质量发现：平台元数据指纹不可单信——证据链必须双记录（平台登记 md5 + 本地实测 md5/sha256），与引擎指纹-版本分离设计互证；MISMATCH 不阻断解析（zip 完整可开）但溯源注记必须保留",
         verify="manual"),
    dict(gt_id="GT-T18", chain="telecom-data-quality", category="数据质量",
         source_file="fetch-report.json",
         source_url="https://txzbqy.miit.gov.cn（ZJYD20260500552 变更公告 tenderFileId=364c0dd2…）",
         excerpt="浙江立项批复[2026]_321_号-关于浙江公司2025年算网能力二阶段建设项目立项变更的批复",
         expected="txzbqy 字段 tenderFileId 语义存在漂移（ZJYD 变更公告的 tenderFileId 实际指向立项批复 PDF，非招标文件本体）",
         rationale="字段名不副实的真实案例：抽取器不能按字段名假设文件语义，必须以文件内容与元数据（fileName）双重判定——影响文件分类与主件识别规则设计",
         verify="manual"),
]


def main():
    os.makedirs(GT_DIR, exist_ok=True)
    out = []
    n_fail = 0
    for it in GT_ITEMS:
        # 四要素之一：原文定位核验
        if it["source_file"] == "fetch-report.json":
            fr = json.load(open(os.path.join(ROOT, "fetch-report.json")))
            text = json.dumps(fr, ensure_ascii=False)
        else:
            text = file_text_for_locate(it["source_file"])
        mode, detail = locate(it["excerpt"], text)
        status = "PASS" if mode != "FAIL" else "FAIL"
        if mode == "FAIL":
            n_fail += 1
        rec = dict(it)
        rec["locate_check"] = {"mode": mode, "status": status, "detail": detail,
                               "checked_against": it["source_file"]}
        rec["review_status"] = "draft_pending_human"
        out.append(rec)
        print(f"{it['gt_id']} [{mode}{'/'+detail if detail else ''}] {it['category']} | {it['excerpt'][:40]}")

    with open(os.path.join(GT_DIR, "ground-truth-draft.jsonl"), "w") as f:
        for r in out:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"\nGT 草案 {len(out)} 条落盘；定位 FAIL {n_fail} 条")
    print(f"文件: real-corpus/telecom/gt/ground-truth-draft.jsonl")
    sys.exit(1 if n_fail else 0)


if __name__ == "__main__":
    main()
