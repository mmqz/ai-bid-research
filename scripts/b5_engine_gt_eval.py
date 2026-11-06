#!/usr/bin/env python3
"""b5_engine_gt_eval —— B5 引擎路径抽取质量评测（GT-25 锚点口径）。

FG-04（#305）修复说明：
  原「双路径对比」（引擎路径 vs 旧路径 bid_parse::parse_file）在 B4 后断裂
  ——旧自研 docx 解析已删，parse_file 显式拒 .docx（实测 panic
  「不支持的文件类型 .docx」）。旧路径腿已死，对比前提失效。

  现口径：
  ① 活体评测 = 引擎路径（bid_engine::parse_docx_via_engine）vs GT-25，
     三指标（条款号召回/内容锚召回/类别命中）不得低于**冻结基线**
     （baseline 常量，锚定 HEAD 实测——回归即红，收紧须附根因，AGENTS.md 规则 9）；
  ② 「引擎 ≥ 旧路径」的交换不丢内容保证由冻结验收证据承载：
     artifacts/engine-diff-r3/（B3-3 r3 全量差分，A-招标文件.docx 100% EQ
     ——引擎路径与旧路径输出逐位等价）。旧路径已删，等价性无从活体重跑，
     以验收档案为凭。

  T18a（run_tests.py）同步改判「不低于冻结基线」。
"""
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
GT = REPO / "fixtures/corpus-A/ground-truth/requirements.jsonl"
DOCX = REPO / "fixtures/corpus-A/files/A-招标文件.docx"

# 冻结基线（FG-04 修复时 HEAD 4a9f7d3 实测——引擎路径 GT-25 三指标）。
# 回归（低于任一）即红；收紧须附根因说明（不是放宽，是防漂移地板）。
BASELINE = {"clause": 25, "excerpt": 25, "category": 22}


def find_cargo() -> str:
    """cargo 定位：PATH → CARGO_HOME → ~/.cargo/bin（CI/本地/精简环境三通道）。"""
    c = shutil.which("cargo")
    if c:
        return c
    for cand in (os.environ.get("CARGO_HOME"), str(Path.home() / ".cargo")):
        if cand:
            p = Path(cand) / "bin" / "cargo"
            if p.is_file():
                return str(p)
    raise FileNotFoundError("cargo 不可用（Rust 工具链缺失——T18 前置依赖）")


def extract_reqs() -> list:
    """经引擎路径（B4 后 docx 唯一活体路径）抽取。"""
    code = r'''
fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files/A-招标文件.docx");
    let engine = bid_engine::parse_docx_via_engine(&path).unwrap();
    let reqs = bid_parse::extract::extract_requirements(&[engine]);
    let out: Vec<serde_json::Value> = reqs.iter().map(|r| serde_json::json!({
        "clause_no": r.clause_no, "category": r.category,
        "excerpt": r.anchors.first().map(|a| a.excerpt.clone()).unwrap_or_default(),
    })).collect();
    println!("{}", serde_json::to_string(&out).unwrap());
}
'''
    example = REPO / "bidcore/crates/bid-engine/examples/gt_eval_dump.rs"
    example.parent.mkdir(parents=True, exist_ok=True)
    example.write_text(code)
    try:
        out = subprocess.run(
            [find_cargo(), "run", "-q", "-p", "bid-engine", "--example", "gt_eval_dump"],
            cwd=REPO / "bidcore", capture_output=True, text=True, timeout=300,
        )
        if out.returncode != 0:
            print(out.stderr[-1500:])
            sys.exit(1)
        reqs = []
        for line in out.stdout.splitlines():
            if line.startswith("["):
                reqs = json.loads(line)
        return reqs
    finally:
        example.unlink(missing_ok=True)


def norm(t: str) -> str:
    return "".join(t.split()).replace("\u3000", "")


def main() -> int:
    gt = [json.loads(l) for l in GT.open(encoding="utf-8")]
    reqs = extract_reqs()

    print("GT-25 对齐评测（corpus-A 主文件，引擎路径——B4 后唯一活体 docx 路径）")
    by_clause = {r["clause_no"] for r in reqs if r["clause_no"]}
    excerpts = [norm(r["excerpt"]) for r in reqs]
    hit_c = sum(1 for g in gt if g["clause_no"] in by_clause)
    # 内容锚：GT excerpt 片段被抽取文本包含（容错条款号缺失/形态差）
    hit_x = sum(
        1 for g in gt
        if any(norm(g["anchors"][0]["excerpt"]) in e or e in norm(g["anchors"][0]["excerpt"])
               for e in excerpts if e)
    )
    hit_cat = sum(
        1 for g in gt
        if g["clause_no"] in by_clause
        and any(r["clause_no"] == g["clause_no"] and r["category"] == g["category"]
                for r in reqs)
    )

    print(f"{'条款号召回':12s} {hit_c:6d}/25（基线 {BASELINE['clause']}/25）")
    print(f"{'内容锚召回':12s} {hit_x:6d}/25（基线 {BASELINE['excerpt']}/25）")
    print(f"{'类别命中':12s} {hit_cat:6d}/25（基线 {BASELINE['category']}/25）")
    print(f"{'抽取总数':12s} {len(reqs):6d}")

    # 判定：三指标不低于冻结基线——引擎抽取质量回归即红
    # （「引擎 ≥ 旧路径」的交换等价性由 engine-diff-r3 100% EQ 冻结验收承载）
    ok = (hit_c >= BASELINE["clause"] and hit_x >= BASELINE["excerpt"]
          and hit_cat >= BASELINE["category"])
    print(f"\n结论：引擎路径 GT-25 三指标 {hit_c}/{hit_x}/{hit_cat} vs 冻结基线 "
          f"{BASELINE['clause']}/{BASELINE['excerpt']}/{BASELINE['category']}"
          f" —— {'通过（不低于冻结基线）' if ok else '回归（低于冻结基线）'}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
