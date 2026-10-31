#!/bin/bash
# #80 第三批 · 任务 A'：子代理 LLM 多章实测——驱动入口。
#
# 与 run_real_llm_path.sh（第二批·人肉 LLM）的差异：
#   - 作者不是开发者本人，而是【并行派遣的无上下文子代理】——主代理只做
#     总工（分派 + 验收），正文全部由子代理按 RealGenerator 实发 prompt 撰写；
#   - 多章（3 章）并行：中继按「章节标题」路由（--content-map）；
#   - 每章完成后 save_version 存版本（skill 纪律 8——测试内已落地）。
#
# 两相位（draft 失败零副作用——生成器报错在 CAS 之前返回，可安全重试）：
#   ./run_subagent_path.sh phase1 <ws-dir>
#     ① 写 <ws>/.mang/llm.json（指向本地中继——OpenAI 兼容端点）
#     ② 起中继（capture 模式）→ 跑 subagent_llm_path 测试（capture 相位）
#        → 3 次 draft 各报「LLM HTTP 503」（真实传输面 + 错误语义）
#     ③ prompt 逐个渲染成人读 markdown → <ws>/prompts/<SEC>.md
#        （按 subagent-targets.json 顺序对账标题——错序即 FATAL）
#     【主代理此刻并行派遣 3 个子代理作者 → 各写 <ws>/manuscripts/<SEC>.md】
#
#   ./run_subagent_path.sh phase2 <ws-dir>
#     ④ 校验 3 份手稿（在场 + 非空；格式偏差记录不预拒——偏移是评估数据）
#        → 生成 content-map.json（标题 → 手稿路径）
#     ⑤ 起中继（serve --content-map）→ 跑 subagent_llm_path 测试（serve 相位）
#        → 每章真实链路（引用/覆盖/字数 ±20%/save_version）→ mock 补齐
#        → 校验 blocker=0 → 导出 docx → 台账 → 重启恢复断言
#     ⑥ 产物归档 artifacts/e2e-subagent-path/<run-id>/（prompts/手稿/成品/
#        台账/测试日志）+ latest 刷新
#
# 纪律：中继是工具面基础设施；serve 模式的正文必须由子代理作者撰写（不是
# 引擎输出回灌——循环论证）。CI 不跑本脚本（subagent_llm_path 测试默认 SKIP）。
set -euo pipefail
cd "$(dirname "$0")/../.."

PHASE="${1:?用法: run_subagent_path.sh phase1|phase2 <ws-dir>}"
WS="${2:?用法: run_subagent_path.sh phase1|phase2 <ws-dir>}"
WS="$(mkdir -p "$WS" && cd "$WS" && pwd)"
PORT="${BID_RELAY_PORT:-8766}"
RELAY_DIR="$WS/relay"
PROMPTS_DIR="$WS/prompts"
MS_DIR="$WS/manuscripts"
MARKER="$WS/.mang/subagent-targets.json"
RUN_ID="$(date +%Y%m%d-%H%M%S)"
ART="artifacts/e2e-subagent-path/$RUN_ID"

export PATH="$HOME/.cargo/bin:$PATH"

build_stdio() {
    cargo build --manifest-path bidcore/Cargo.toml -p bid-mcp --bin bid-mcp-stdio
    local bin
    bin="$(cargo metadata --manifest-path bidcore/Cargo.toml --format-version 1 \
        | python3 -c 'import json,sys,os; print(os.path.join(json.load(sys.stdin)["target_directory"], "debug", "bid-mcp-stdio"))')"
    [ -x "$bin" ] || { echo "FATAL: $bin 不存在" >&2; exit 1; }
    export BID_SUBAGENT_STDIO_BIN="$bin"
    echo "[driver] stdio 二进制 → $BID_SUBAGENT_STDIO_BIN"
}

write_llm_json() {
    mkdir -p "$WS/.mang"
    cat > "$WS/.mang/llm.json" <<EOF
{
  "api_base": "http://127.0.0.1:$PORT/v1",
  "api_key": "subagent-relay-local",
  "model": "subagent-author-v1",
  "timeout_secs": 60,
  "temperature": 0.3,
  "max_tokens_cap": 8192
}
EOF
    echo "[driver] llm.json → http://127.0.0.1:$PORT/v1（model=subagent-author-v1）"
}

start_relay() {  # $1 = mode, $2 = 额外参数(--content-map 文件，可空)
    mkdir -p "$RELAY_DIR"
    local args=(scripts/e2e_bid/human_llm_relay.py "$1" --dir "$RELAY_DIR" --port "$PORT")
    [ -n "${2:-}" ] && args+=("$2")
    python3 "${args[@]}" &
    RELAY_PID=$!
    sleep 1
    if ! kill -0 "$RELAY_PID" 2>/dev/null; then
        echo "FATAL: 中继启动失败（端口 $PORT 被占？换 BID_RELAY_PORT=xxxx）" >&2
        exit 1
    fi
    echo "[driver] 中继 $1 模式 pid=$RELAY_PID"
}

stop_relay() {
    [ -n "${RELAY_PID:-}" ] && kill "$RELAY_PID" 2>/dev/null || true
    wait "${RELAY_PID:-}" 2>/dev/null || true
    RELAY_PID=""
}

run_test_phase() {  # $1 = capture|serve
    BID_SUBAGENT_PATH_WS="$WS" BID_SUBAGENT_PATH_PHASE="$1" \
        cargo test --manifest-path bidcore/Cargo.toml -p bid-mcp \
        --test subagent_llm_path -- --nocapture --test-threads=1 \
        2>&1 | tee "$WS/phase$([ "$1" = capture ] && echo 1 || echo 2)-test.log" \
        | grep -E "^\[(subagent-path|relay)|test result|running|^error|panicked" || true
}

# prompt-<n>.json × N → prompts/<SEC>.md（顺序对账：prompt-n 的标题必须等于
# 标记 targets[n].title——错序/漏抓都 FATAL，不许静默对齐）
render_prompts() {
    python3 - "$RELAY_DIR" "$PROMPTS_DIR" "$MARKER" <<'PYEOF'
import json, sys
from pathlib import Path
relay, out_dir, marker_p = map(Path, sys.argv[1:4])
marker = json.loads(marker_p.read_text(encoding="utf-8"))
targets = marker["targets"]
prompts = sorted(relay.glob("prompt-*.json"), key=lambda p: p.stat().st_mtime)
if len(prompts) != len(targets):
    sys.exit(f"FATAL: 中继捕获 {len(prompts)} 个 prompt，标记目标 {len(targets)} 章——数量不符")
out_dir.mkdir(parents=True, exist_ok=True)
for i, (pf, t) in enumerate(zip(prompts, targets)):
    body = json.loads(pf.read_text(encoding="utf-8"))
    title = None
    for m in reversed(body.get("messages", [])):
        if m.get("role") == "user":
            for line in m.get("content", "").splitlines():
                if line.startswith("章节标题："):
                    title = line[len("章节标题："):].strip()
                    break
            if title:
                break
    if title != t["title"]:
        sys.exit(f"FATAL: prompt-{i} 标题「{title}」≠ 标记 targets[{i}]「{t['title']}」——顺序错位")
    lines = [f"# 子代理作者请求（RealGenerator 实发——{t['section_id']}）", ""]
    for m in body.get("messages", []):
        lines.append(f"## role: {m.get('role', '?')}")
        lines.append("")
        lines.append(m.get("content", ""))
        lines.append("")
    lines.append("## 其他请求参数")
    lines.append("")
    lines.append(f"- model: `{body.get('model')}`")
    lines.append(f"- temperature: {body.get('temperature')}")
    lines.append(f"- max_tokens: {body.get('max_tokens')}")
    out = out_dir / f"{t['section_id']}.md"
    out.write_text("\n".join(lines), encoding="utf-8")
    print(f"[driver] prompt-{i} → {out}（{t['section_id']}《{t['title']}》）")
print(f"[driver] {len(prompts)} 份 prompt 渲染完成 → {out_dir}")
PYEOF
}

# 手稿校验 + content-map 生成（格式偏差【记录不预拒】——偏移本身是评估数据）
verify_manuscripts() {
    python3 - "$MS_DIR" "$MARKER" "$WS/content-map.json" "$WS/manuscript-check.json" <<'PYEOF'
import json, sys
from pathlib import Path
ms_dir, marker_p, map_p, check_p = map(Path, sys.argv[1:5])
marker = json.loads(marker_p.read_text(encoding="utf-8"))
targets = marker["targets"]
cmap, notes = {}, []
for t in targets:
    sec, title = t["section_id"], t["title"]
    f = ms_dir / f"{sec}.md"
    if not f.is_file() or not f.read_text(encoding="utf-8").strip():
        sys.exit(f"FATAL: 手稿缺失或为空：{f}（子代理作者未交付 {sec}）")
    text = f.read_text(encoding="utf-8")
    first = text.strip().splitlines()[0]
    cmap[title] = str(f)
    fmt = "ok" if first == f"# {title}" else "deviation"
    notes.append({"section_id": sec, "title": title, "chars": len(text),
                  "first_line": first, "first_line_format": fmt})
Path(map_p).write_text(json.dumps(cmap, ensure_ascii=False, indent=1), encoding="utf-8")
Path(check_p).write_text(json.dumps(notes, ensure_ascii=False, indent=1), encoding="utf-8")
for n in notes:
    flag = "✅" if n["first_line_format"] == "ok" else "⚠️ 首行偏差"
    print(f"[driver] 手稿 {n['section_id']} 《{n['title']}》 {n['chars']} 字——{flag}")
print(f"[driver] content-map → {map_p}（{len(cmap)} 章）")
PYEOF
}

case "$PHASE" in
phase1)
    build_stdio
    write_llm_json
    rm -rf "$RELAY_DIR" "$PROMPTS_DIR"; mkdir -p "$RELAY_DIR"
    start_relay capture ""
    trap stop_relay EXIT
    run_test_phase capture
    stop_relay; trap - EXIT
    render_prompts
    echo
    echo "=================================================================="
    echo " capture 相位完成——现在【派遣子代理作者】（主代理 = 总工）："
    echo "   1. 逐份阅读 $PROMPTS_DIR/SEC-XXXX.md（实发 prompt 原文）"
    echo "   2. 并行派遣 3 个子代理，每个只拿到自己那份 prompt——"
    echo "      零仓库上下文、零额外写作指导（偏移检验的核心纪律）"
    echo "   3. 子代理把补全正文写入 $MS_DIR/SEC-XXXX.md"
    echo "      （文件内容 = 补全输出本体，不含解释/元信息）"
    echo "   4. 跑 phase2：$0 phase2 $WS"
    echo "=================================================================="
    ;;
phase2)
    [ -f "$MARKER" ] || { echo "FATAL: 缺 $MARKER（先跑 phase1）" >&2; exit 1; }
    verify_manuscripts
    build_stdio
    write_llm_json
    start_relay serve "--content-map=$WS/content-map.json"
    trap stop_relay EXIT
    run_test_phase serve
    stop_relay; trap - EXIT
    # ---- 产物归档（run-id 快照 + latest 刷新）
    mkdir -p "$ART"
    cp -r "$PROMPTS_DIR" "$ART/prompts"
    cp -r "$MS_DIR" "$ART/manuscripts"
    cp "$MARKER" "$ART/"
    cp "$WS/content-map.json" "$WS/manuscript-check.json" "$ART/" 2>/dev/null || true
    cp "$WS/.mang/llm-usage.json" "$ART/" 2>/dev/null || true
    cp "$WS"/phase*-test.log "$ART/" 2>/dev/null || true
    cp "$WS"/out/*.docx "$ART/" 2>/dev/null || true
    cp -r "$RELAY_DIR" "$ART/relay-prompts" 2>/dev/null || true
    if [ -d "$WS/chapters" ]; then mkdir -p "$ART/drafts"; cp -r "$WS/chapters" "$ART/drafts/"; fi
    if [ -d "$WS/.mang/history" ]; then cp -r "$WS/.mang/history" "$ART/save-versions" 2>/dev/null || true; fi
    rm -rf artifacts/e2e-subagent-path/latest
    mkdir -p artifacts/e2e-subagent-path/latest
    cp -r "$ART"/. artifacts/e2e-subagent-path/latest/
    echo
    echo "=================================================================="
    echo " serve 相位完成——子代理 LLM 多章全链 ✅"
    echo " 产物：$ART（prompts/手稿/成品 docx/台账/测试日志/save-versions）+ latest 刷新"
    echo "=================================================================="
    ;;
*)
    echo "用法: $0 phase1|phase2 <ws-dir>" >&2; exit 1 ;;
esac
