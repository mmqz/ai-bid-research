#!/bin/bash
# #59 阶段 5 · 端到端 e2e 一键入口（Lead 验收：一条命令从招标文件跑到成品 docx）
#
# 用法：./run_e2e.sh [cargo-test-args...]
#       ./run_e2e.sh -- --nocapture    # 查看全链逐步输出
#
# Rust 内核：bidcore/crates/bid-mcp/tests/e2e_pipeline.rs
# 链路：导入(真件) → 抽取 → 建纲建矩阵 → 绑证据+三门 → 逐章起草(mock) → 校验 → 导出 docx
# SKIP 纪律（issue #9）：corpus-A 真件缺失 → 响亮横幅跳过（缺件不是业务失败）
set -euo pipefail
# PATH：容器/rustup 环境的 cargo 位置（本机 $HOME/.cargo/bin——CI 的 Linux
# runner 自带；本地 Windows 走 rustup 默认安装位）
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/../../bidcore"

echo "=============================================================="
echo " ai-bid-research · e2e 全链（#59 阶段 5）"
echo " 输入: fixtures/corpus-A/files（A 主文件 + C 补遗 + EV 证据）"
echo " 输出: 成品 docx（临时工作区 out/）+ 每步断言（失败定位到步）"
echo "=============================================================="

# 预检：真件在位（缺件响亮退出——不静默假装通过）
MAIN="../fixtures/corpus-A/files/A-招标文件.docx"
ADDENDUM="../fixtures/corpus-A/files/C-补遗文件01.docx"
for f in "$MAIN" "$ADDENDUM"; do
    if [ ! -f "$f" ]; then
        echo "SKIP: e2e 全链——真件缺失: $f"
        echo "      （corpus-A 是仓库自带 fixtures；缺件=仓库不完整，不是业务失败）"
        exit 0
    fi
done

# 一条命令全链（每步失败 → cargo test 的 panic 信息带步骤号 ①-⑧）
cargo test -p bid-mcp --test e2e_pipeline "$@"

echo "=============================================================="
echo " e2e 全链通过：招标文件进 → 成品标书出 ✅"
echo "  - e2e_tender_in_docx_out      主链（8 步 + 产物解包深检 + 重启恢复）"
echo "  - e2e_idempotent_rerun_zero_waste  幂等重放（导入/抽取去重）"
echo "=============================================================="
