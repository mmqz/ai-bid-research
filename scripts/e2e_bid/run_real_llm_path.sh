#!/bin/bash
# #80 第二批 · 任务 A：真实生成通路（人肉 LLM——无 API key）驱动入口。
#
# 两相位（draft 失败零副作用——生成器报错在 CAS 之前返回，可安全重试）：
#   ./run_real_llm_path.sh phase1 <ws-dir>
#     ① 写 <ws>/.mang/llm.json（指向本地中继——OpenAI 兼容端点）
#     ② 起中继（capture 模式）→ 跑 real_llm_path 测试（capture 相位）
#        → draft 报「LLM HTTP 503」（真实传输面 + 错误语义），prompt 落盘
#     ③ 把 prompt 渲染成人读 markdown：<ws>/.mang/real-path-prompt.md
#     【开发者此刻撰写正文 → <ws>/.mang/human-chapter.md】
#
#   ./run_real_llm_path.sh phase2 <ws-dir>
#     ④ 起中继（serve 模式，正文 = human-chapter.md）
#     ⑤ 跑 real_llm_path 测试（serve 相位）→ generator=real 全链：
#        draft（引用/覆盖度/字数 ±20%）→ mock 补齐余章 → 校验 blocker=0
#        → 导出 docx → llm-usage.json 台账 → 重启恢复断言
#     ⑥ 产物归档 artifacts/e2e-real-path/<run-id>/（prompt/正文/成品/台账/目标标记）
#
# 纪律：中继是工具面基础设施；serve 模式的正文必须开发者撰写（不是引擎
# 输出回灌——那是循环论证）。CI 不跑本脚本（real_llm_path 测试默认 SKIP）。
set -euo pipefail
cd "$(dirname "$0")/../.."

PHASE="${1:?用法: run_real_llm_path.sh phase1|phase2 <ws-dir>}"
WS="${2:?用法: run_real_llm_path.sh phase1|phase2 <ws-dir>}"
WS="$(mkdir -p "$WS" && cd "$WS" && pwd)"
PORT="${BID_RELAY_PORT:-8765}"
RELAY_DIR="$WS/relay"
RUN_ID="$(date +%Y%m%d-%H%M%S)"
ART="artifacts/e2e-real-path/$RUN_ID"

export PATH="$HOME/.cargo/bin:$PATH"

# stdio 二进制：先构建再显式传给测试（cargo test --test 过滤面不构建 bin）
build_stdio() {
    cargo build --manifest-path bidcore/Cargo.toml -p bid-mcp --bin bid-mcp-stdio
    # target 目录定位：CARGO_TARGET_DIR > bidcore/target（cargo metadata 权威解）
    local bin
    bin="$(cargo metadata --manifest-path bidcore/Cargo.toml --format-version 1 \
        | python3 -c 'import json,sys,os; print(os.path.join(json.load(sys.stdin)["target_directory"], "debug", "bid-mcp-stdio"))')"
    [ -x "$bin" ] || { echo "FATAL: $bin 不存在" >&2; exit 1; }
    export BID_REAL_PATH_STDIO_BIN="$bin"
    echo "[driver] stdio 二进制 → $BID_REAL_PATH_STDIO_BIN"
}

write_llm_json() {
    mkdir -p "$WS/.mang"
    cat > "$WS/.mang/llm.json" <<EOF
{
  "api_base": "http://127.0.0.1:$PORT/v1",
  "api_key": "human-relay-local",
  "model": "human-agent-author-v1",
  "timeout_secs": 60,
  "temperature": 0.3,
  "max_tokens_cap": 8192
}
EOF
    echo "[driver] llm.json → http://127.0.0.1:$PORT/v1（model=human-agent-author-v1）"
}

start_relay() {  # $1 = mode, $2 = content(可空)
    mkdir -p "$RELAY_DIR"
    local args=(scripts/e2e_bid/human_llm_relay.py "$1" --dir "$RELAY_DIR" --port "$PORT")
    [ -n "${2:-}" ] && args+=(--content "$2")
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

render_prompt() {  # prompt-0.json → 人读 markdown
    python3 - "$RELAY_DIR" "$WS/.mang/real-path-prompt.md" <<'PYEOF'
import json, sys
from pathlib import Path
relay, out = sys.argv[1], sys.argv[2]
prompts = sorted(Path(relay).glob("prompt-*.json"))
if not prompts:
    sys.exit("FATAL: 中继未捕获到 prompt")
body = json.loads(prompts[-1].read_text(encoding="utf-8"))
lines = ["# real-path prompt（RealGenerator 实发——人肉 LLM 的写作面）", ""]
for m in body.get("messages", []):
    role = m.get("role", "?")
    lines.append(f"## role: {role}")
    lines.append("")
    lines.append(m.get("content", ""))
    lines.append("")
lines.append("## 其他请求参数")
lines.append("")
lines.append(f"- model: `{body.get('model')}`")
lines.append(f"- temperature: {body.get('temperature')}")
lines.append(f"- max_tokens: {body.get('max_tokens')}")
Path(out).write_text("\n".join(lines), encoding="utf-8")
print(f"[driver] prompt 渲染 → {out}")
PYEOF
}

case "$PHASE" in
phase1)
    build_stdio
    write_llm_json
    rm -rf "$RELAY_DIR"; mkdir -p "$RELAY_DIR"
    start_relay capture ""
    trap stop_relay EXIT
    BID_REAL_PATH_WS="$WS" BID_REAL_PATH_PHASE=capture \
        cargo test --manifest-path bidcore/Cargo.toml -p bid-mcp --test real_llm_path -- --nocapture --test-threads=1
    stop_relay; trap - EXIT
    render_prompt
    echo
    echo "=================================================================="
    echo " capture 相位完成——现在【你】是 LLM："
    echo "   1. 阅读 $WS/.mang/real-path-prompt.md（章节标题/目标字数/REQ 清单）"
    echo "   2. 撰写正文到 $WS/.mang/human-chapter.md"
    echo "      规则：首行 '# 章节标题'；引用只用列出的 [REQ-xxxx]；"
    echo "            每条 REQ 都要回应；字数 = 目标 ±20%"
    echo "   3. 跑 phase2：$0 phase2 $WS"
    echo "=================================================================="
    ;;
phase2)
    CHAPTER="$WS/.mang/human-chapter.md"
    if [ ! -s "$CHAPTER" ]; then
        echo "FATAL: $CHAPTER 不存在或为空——先按 phase1 的指引撰写正文" >&2
        exit 1
    fi
    build_stdio
    write_llm_json
    start_relay serve "$CHAPTER"
    trap stop_relay EXIT
    BID_REAL_PATH_WS="$WS" BID_REAL_PATH_PHASE=serve \
        cargo test --manifest-path bidcore/Cargo.toml -p bid-mcp --test real_llm_path -- --nocapture --test-threads=1
    stop_relay; trap - EXIT
    # ---- 产物归档（run-id 快照 + latest 刷新）
    mkdir -p "$ART" "$ART/drafts"
    cp "$CHAPTER" "$ART/human-chapter.md"
    cp "$WS/.mang/real-path-prompt.md" "$ART/" 2>/dev/null || true
    cp "$WS/.mang/real-path-target.json" "$ART/" 2>/dev/null || true
    cp "$WS/.mang/llm-usage.json" "$ART/" 2>/dev/null || true
    cp "$WS"/out/*.docx "$ART/" 2>/dev/null || true
    cp -r "$RELAY_DIR" "$ART/relay-prompts" 2>/dev/null || true
    if [ -d "$WS/chapters" ]; then cp -r "$WS/chapters" "$ART/drafts/"; fi
    rm -rf artifacts/e2e-real-path/latest
    mkdir -p artifacts/e2e-real-path/latest
    cp -r "$ART"/. artifacts/e2e-real-path/latest/
    echo
    echo "=================================================================="
    echo " serve 相位完成——真实生成通路全链 ✅"
    echo " 产物：$ART（prompt/正文/成品 docx/台账/中继记录）+ latest 刷新"
    echo "=================================================================="
    ;;
*)
    echo "用法: $0 phase1|phase2 <ws-dir>" >&2; exit 1 ;;
esac
