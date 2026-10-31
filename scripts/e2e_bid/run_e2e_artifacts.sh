#!/bin/bash
# #80 第二批 · 任务 B：e2e 全链 + 10 产物落盘（run_e2e.sh 级别的产物清单与耗时记录）。
#
# 用法：./run_e2e_artifacts.sh [cargo-test-args...]
#       ./run_e2e_artifacts.sh -- --nocapture    # 查看全链逐步输出
#
# Rust 内核：bidcore/crates/bid-mcp/tests/e2e_artifacts.rs
# 载体：fixtures/synth-tender（仓库自带——CI 可跑；真 LLM 面见 run_real_llm_path.sh）
# 产物：artifacts/e2e/<run-id>/ 十一件套 + latest 刷新（#100 任务 3 增 11_self-score.json 整本自评）
#       01 项目 / 02 导入 / 03 条款 / 04 目录 / 05 drafts/ / 06 format-spec /
#       07 docx / 08 pdf（soffice 软门）/ 09 校验 / 11 自评（B2 接线）/ 10 报告（耗时/字数/警告）
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/../.."

echo "=============================================================="
echo " ai-bid-research · e2e 全链产物（#80 第二批 · 任务 B）"
echo " 输入: fixtures/synth-tender（A/B/C/D/E 五件）"
echo " 输出: artifacts/e2e/<run-id>/ 十件套 + latest"
echo "=============================================================="

cargo test --manifest-path bidcore/Cargo.toml -p bid-mcp --test e2e_artifacts "$@"

echo "=============================================================="
echo " e2e 全链产物通过 ✅"
echo "  - 最新产物：artifacts/e2e/latest/10_report.md（整本自评见 11_self-score.json）"
echo "  - 真实 LLM 通路（人肉）：scripts/e2e_bid/run_real_llm_path.sh"
echo "=============================================================="
