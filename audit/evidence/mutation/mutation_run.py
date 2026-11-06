#!/usr/bin/env python3
"""R5 Mutation Testing 白名单运行器 v2（P0/P1 关键业务不变量）

修正：直连真实 toolchain cargo（绕过 rustup shim），异常时还原变异，
记录 test-result 行与错误摘录。生产仓库只接收 audit/ 结果文件。
"""
import subprocess
import os
import sys
import time

ISO = "/home/z/my-project/test-run"
RESULTS = "/home/z/my-project/ai-bid-research/audit/evidence/mutation/mutation-results.md"
CARGO = "/home/z/my-project/.cargo/bin/cargo"
ENV = dict(os.environ)
ENV["RUSTUP_HOME"] = "/home/z/my-project/.rustup"
ENV["CARGO_HOME"] = "/home/z/my-project/.cargo"
ENV["CARGO_NET_GIT_FETCH_WITH_CLI"] = "true"
ENV["PATH"] = ("/home/z/my-project/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:"
                "/home/z/my-project/.cargo/bin:" + os.environ.get("PATH", ""))
os.makedirs(os.path.dirname(RESULTS), exist_ok=True)

MUTATIONS = [
    {
        "id": "MUT-01",
        "desc": "删除起草侧最低篇幅 BLOCKER 判定（validate.rs:265 恒假）",
        "file": "bidcore/crates/bid-tools/src/validate.rs",
        "old": "if (chars as f64) < target * 0.80 {",
        "new": "if false && (chars as f64) < target * 0.80 {",
        "guard": ["-p", "bid-mcp", "--test", "writing_acceptance_e2e"],
        "guard_name": "bid-mcp::writing_acceptance_e2e",
    },
    {
        "id": "MUT-02",
        "desc": "删除导出门独立复算篇幅判定（validate.rs:524 恒假——#242 门 1c）",
        "file": "bidcore/crates/bid-tools/src/validate.rs",
        "old": "if (chars as f64) < f64::from(node.target_chars) * 0.80 {",
        "new": "if false && (chars as f64) < f64::from(node.target_chars) * 0.80 {",
        "guard": ["-p", "bid-mcp", "--test", "writing_acceptance_e2e"],
        "guard_name": "bid-mcp::writing_acceptance_e2e",
    },
    {
        "id": "MUT-03",
        "desc": "删除 Agent 必选阻断接线（readiness.rs:638 过滤器恒空——#241 门）",
        "file": "bidcore/crates/bid-tools/src/readiness.rs",
        "old": ".filter(|r| bid_domain::matrix::agent_mandatory(r))",
        "new": ".filter(|r| false && bid_domain::matrix::agent_mandatory(r))",
        "guard": ["-p", "bid-mcp", "--test", "agent_mandatory_gate"],
        "guard_name": "bid-mcp::agent_mandatory_gate",
    },
    {
        "id": "MUT-04",
        "desc": "删除 receipt HMAC 验签（principal.rs ct_eq 恒过——人工授权伪造面）",
        "file": "bidcore/crates/bid-domain/src/principal.rs",
        "old": "if !ct_eq(&presented, &expected) {",
        "new": "if !false && false {",
        "guard": ["-p", "bid-mcp", "--test", "set_matrix_override_tool"],
        "guard_name": "bid-mcp::set_matrix_override_tool",
    },
    {
        "id": "MUT-05",
        "desc": "删除路径围栏包含判定（fs_fence.rs is_inside 恒真——任意路径读写面）",
        "file": "bidcore/crates/bid-tools/src/fs_fence.rs",
        "old": "pub fn is_inside(a: &Path, b: &Path) -> bool {\n    strip_root(a, b).is_some()\n}",
        "new": "pub fn is_inside(a: &Path, b: &Path) -> bool {\n    let _ = (a, b);\n    true\n}",
        "guard": ["-p", "bid-tools", "--test", "fs_fence"],
        "guard_name": "bid-tools::fs_fence",
    },
    {
        "id": "MUT-06",
        "desc": "删除评分份额聚合校验（scoring.rs:1357 越界拒绝对恒假——#224 不变量）",
        "file": "bidcore/crates/bid-domain/src/scoring.rs",
        "old": "if !(0.0 < weight_share && weight_share <= 1.0) {",
        "new": "if false {",
        "guard": ["-p", "bid-domain", "--test", "scoring_mapping"],
        "guard_name": "bid-domain::scoring_mapping",
    },
]

BUDGET = int(sys.argv[1]) if len(sys.argv) > 1 else 420
START = time.time()


def sh(cmd, cwd, timeout=480):
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout, env=ENV)
        return r.returncode, (r.stdout + r.stderr)
    except subprocess.TimeoutExpired:
        return 124, "TIMEOUT"


def revert(f):
    subprocess.run(["git", "-C", ISO, "checkout", "--", f], capture_output=True, env=ENV)


done_ids = set()
if os.path.exists(RESULTS):
    for line in open(RESULTS, encoding="utf-8"):
        if line.startswith("### MUT-") and "结论:" in line and "SKIP" not in line:
            done_ids.add(line.split()[1])

wrote_header = os.path.exists(RESULTS)
with open(RESULTS, "a", encoding="utf-8") as out:
    if not wrote_header:
        out.write("# R5 Mutation 测试结果（P0/P1 不变量白名单）\n\n")
        out.write("环境：Linux 隔离副本（冻结 6105beb）；变异=守卫条件恒假/恒真；\n")
        out.write("判定：守卫测试变红=被捕获（CAUGHT）；仍绿=未捕获（SURVIVED，测试缺口）。\n")
        out.write("（v2 重跑：v1 结果因 rustup shim 环境缺失全部无效，已废弃）\n\n")

    for m in MUTATIONS:
        if m["id"] in done_ids:
            continue
        if time.time() - START > BUDGET:
            out.write("（预算到达，待续跑）\n")
            break
        # 磁盘守卫：清理 incremental，不足 400MB 则跳过本轮
        inc = os.path.join(ISO, "bidcore/target/debug/incremental")
        subprocess.run(["rm", "-rf", inc], capture_output=True, env=ENV)
        try:
            st = os.statvfs(ISO)
            free = st.f_bavail * st.f_frsize
        except Exception:
            free = 1 << 40
        if free < 400 * 1024 * 1024:
            out.write(f"### {m['id']} 结论: BLOCKED_DISK\n\n- 可用磁盘 {free // 1048576}MB 不足，跳过\n\n")
            out.flush()
            continue
        path = os.path.join(ISO, m["file"])
        try:
            src = open(path, encoding="utf-8").read()
            cnt = src.count(m["old"])
            if cnt != 1:
                out.write(f"### {m['id']} 结论: SKIP\n\n- 变异点出现次数 {cnt} ≠ 1：{m['desc']}\n\n")
                out.flush()
                continue
            open(path, "w", encoding="utf-8").write(src.replace(m["old"], m["new"], 1))
            code, log = sh([CARGO, "test"] + m["guard"], os.path.join(ISO, "bidcore"))
            result_lines = [l.strip() for l in (log or "").splitlines() if "test result:" in l]
            err_lines = [l.strip() for l in (log or "").splitlines() if l.startswith("error")][:3]
            if code == 0:
                verdict = "SURVIVED(未捕获——测试缺口)"
            elif code == 124:
                verdict = "TIMEOUT"
            elif result_lines and any("0 failed" not in l for l in result_lines):
                verdict = "CAUGHT(守卫变红)"
            elif err_lines and not result_lines:
                verdict = "COMPILE_FAIL(变异破坏编译——不计为捕获)"
            else:
                verdict = "CAUGHT(守卫变红)" if code != 0 else "SURVIVED(未捕获——测试缺口)"
        finally:
            revert(m["file"])
            tname = m["guard"][-1]
            subprocess.run(["bash", "-c", f"rm -f {ISO}/bidcore/target/debug/deps/{tname}-*"],
                           capture_output=True, env=ENV)
            subprocess.run(["rm", "-rf", os.path.join(ISO, "bidcore/target/debug/incremental")],
                           capture_output=True, env=ENV)
        out.write(f"### {m['id']} 结论: {verdict}\n\n")
        out.write(f"- 变异：{m['desc']}\n")
        out.write(f"- 位置：{m['file']}\n")
        out.write(f"- 守卫测试：{m['guard_name']} → exit {code}\n")
        for l in result_lines[:3]:
            out.write(f"- {l}\n")
        for l in err_lines[:2]:
            out.write(f"- [编译] {l[:150]}\n")
        out.write("\n")
        out.flush()
        print(f"{m['id']}: {verdict} (exit {code})")
print("MUTATION-BATCH-DONE")
