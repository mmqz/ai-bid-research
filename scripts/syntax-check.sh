#!/usr/bin/env bash
# 轻量语法检查——不编译、不占 CPU，秒级完成。
#
# 为什么需要这个：Lead 在用户开发机上工作，不能跑 cargo build/test
# （用户明确要求「本地不要编译尽量云端跑」）。但纯语法错误（括号/属性/
# 字面量越界之外的结构问题）可以靠 rustfmt 的 parser 在本地秒级查出，
# 避免每轮 CI 花 15 分钟才发现一个多余的花括号。
#
# 用法：scripts/syntax-check.sh [文件...]
#   无参数时检查全部 tests/ 下的 .rs
#
# 能查：括号不平衡、属性语法、match 臂结构、字符串未闭合
# 查不出：类型错误、借用检查、未定义符号、字面量越界（那些必须 cargo check）
set -uo pipefail

RUSTFMT="${RUSTFMT:-rustfmt}"
if ! command -v "$RUSTFMT" >/dev/null 2>&1; then
    echo "SKIP: rustfmt 不在 PATH（装：rustup component add rustfmt）"
    exit 0
fi

fail=0
count=0
if [ $# -eq 0 ]; then
    mapfile -t files < <(find bidcore/crates -path '*/tests/*.rs' -o -name '*.rs' -path '*/src/*' | sort)
else
    files=("$@")
fi

for f in "${files[@]}"; do
    [ -f "$f" ] || continue
    count=$((count + 1))
    out=$("$RUSTFMT" --emit stdout --edition 2021 "$f" 2>&1 >/dev/null)
    # rustfmt 的语法错误形如 `error: ...` 或 `error[E....]`
    if printf '%s' "$out" | grep -qE '^error(\[|:)'; then
        echo "FAIL: $f"
        printf '%s\n' "$out" | grep -E '^error' | head -3 | sed 's/^/      /'
        fail=$((fail + 1))
    fi
done

echo ""
echo "检查 $count 个文件，失败 $fail 个"
[ "$fail" -eq 0 ]