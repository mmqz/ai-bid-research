#!/usr/bin/env python3
"""b5_engine_gt_eval —— B5 引擎路径抽取质量评测（GT-25 锚点口径，双路径对比）。

方法：
  同一主文件（corpus-A A-招标文件.docx）分别走
  ① 引擎路径（bid_engine::parse_docx_via_engine + extract.rs）
  ② 旧路径（bid_parse::parse_file + extract.rs）
  后与 GT-25 对齐：条款号命中（clause_no 相等）为主锚，excerpt 包含为内容锚。

输出：双路径召回率/类别命中对比——量化 B5 引擎交换的抽取面影响
（对应 issue #2 §B5 验收「解析结果可复现证据」扩展面）。
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


def extract_reqs() -> dict:
    """经 A8 管线（引擎路径）抽取；同文件再跑旧路径对照。"""
    # 引擎路径：直接用 bid-engine 门面 + extract（编译一次，跑两路径）
    code = r'''
fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/corpus-A/files/A-招标文件.docx");
    let engine = bid_engine::parse_docx_via_engine(&path).unwrap();
    let legacy = bid_parse::parse_file(&path, "A-招标文件.docx").unwrap();
    let r1 = bid_parse::extract::extract_requirements(&[engine]);
    let r2 = bid_parse::extract::extract_requirements(&[legacy]);
    let dump = |tag: &str, reqs: &[bid_domain::Requirement]| {
        let out: Vec<serde_json::Value> = reqs.iter().map(|r| serde_json::json!({
            "clause_no": r.clause_no, "category": r.category,
            "excerpt": r.anchors.first().map(|a| a.excerpt.clone()).unwrap_or_default(),
        })).collect();
        println!("=== {} ===", tag);
        println!("{}", serde_json::to_string(&out).unwrap());
    };
    dump("ENGINE", &r1);
    dump("LEGACY", &r2);
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
        result = {"ENGINE": [], "LEGACY": []}
        tag = None
        for line in out.stdout.splitlines():
            if line.startswith("=== "):
                tag = line.strip("= ").strip()
            elif tag and line.startswith("["):
                result[tag] = json.loads(line)
        return result
    finally:
        example.unlink(missing_ok=True)


def norm(t: str) -> str:
    return "".join(t.split()).replace("\u3000", "")


def main() -> int:
    gt = [json.loads(l) for l in GT.open(encoding="utf-8")]
    paths = extract_reqs()

    print(f"GT-25 对齐评测（corpus-A 主文件）")
    print(f"{'':12s} {'ENGINE':>10s} {'LEGACY':>10s}")
    stats = {}
    for tag, reqs in paths.items():
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
        stats[tag] = (hit_c, hit_x, hit_cat, len(reqs))

    for i, metric in enumerate(["条款号召回", "内容锚召回", "类别命中"]):
        e, l = stats["ENGINE"][i], stats["LEGACY"][i]
        print(f"{metric:12s} {e:6d}/25  {l:6d}/25")
    print(f"{'抽取总数':12s} {stats['ENGINE'][3]:6d}    {stats['LEGACY'][3]:6d}")

    # 判定：引擎路径召回不低于旧路径（内容锚口径）——B5 交换不丢内容
    ok = stats["ENGINE"][1] >= stats["LEGACY"][1]
    print(f"\n结论：引擎路径内容锚召回 {stats['ENGINE'][1]}/25 vs 旧路径 {stats['LEGACY'][1]}/25"
          f" —— {'通过（不低于旧路径）' if ok else '回归（引擎路径低于旧路径）'}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
