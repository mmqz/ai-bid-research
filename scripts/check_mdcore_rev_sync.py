#!/usr/bin/env python3
"""check_mdcore_rev_sync.py —— FG-08（#304）：mdcore 引擎 rev 一致性门禁。

背景（audit/false-green-register.md FG-08）：
  engine_probe 自称「与 bid-engine 同 rev——双跑差分两侧必须同引擎版本」，
  但其 rev 曾漂移到旧审计基线（3204342d）而生产在 f4328ebf——
  仓库内无红信号，引擎三门/S7/S8/D4-D6/X3 的对账结论只对旧 rev 负责。

本脚本把「两侧同 rev」从口头承诺变成 CI 门禁（文本比对，零构建依赖）：

  生产侧   bidcore/Cargo.lock                    （实际构建解析的 rev）
           bidcore/crates/bid-engine/Cargo.toml   （manifest 钉的 rev）
  探针侧   scripts/engine_probe/Cargo.toml        （探针钉的 rev）
           scripts/engine_probe/Cargo.lock        （探针实际解析的 rev）

规则：
  ① 生产 lock 的 mdcore rev 必须唯一（生产自身不分裂）
  ② bid-engine manifest 的 rev == 生产 lock rev（manifest 与 lock 不漂移）
  ③ engine_probe lock 的 rev == 生产 rev（FG-08 主断言：差分两侧同引擎）
  ④ engine_probe manifest 的 rev == engine_probe lock rev（探针自身不漂移）

不一致 → exit 1（附修复指引）。用法：
  python3 scripts/check_mdcore_rev_sync.py          # 仓库根执行
  python3 scripts/check_mdcore_rev_sync.py --root <repo_root>
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

MDCORE_GIT = "github.com/mmqz/mdcore"

# Cargo.lock 的 git 源行：source = "git+https://github.com/mmqz/mdcore?rev=<REV>#<REV>"
LOCK_REV_RE = re.compile(
    r'source\s*=\s*"git\+https://' + re.escape(MDCORE_GIT) + r"\?rev=([0-9a-f]+)#\1"
)
# Cargo.toml 的 rev 钉：git = "...mdcore" 相邻块内的 rev = "<REV>"
TOML_REV_RE = re.compile(r'rev\s*=\s*"([0-9a-f]+)"')


def lock_mdcore_revs(lock_path: Path) -> set[str]:
    """Cargo.lock 里全部 mdcore git 源的 rev 集合。"""
    if not lock_path.is_file():
        return set()
    return set(LOCK_REV_RE.findall(lock_path.read_text(encoding="utf-8")))


def toml_mdcore_revs(toml_path: Path) -> set[str]:
    """Cargo.toml 里钉到 mmqz/mdcore 的 rev 集合。

    解析策略：先按 [dependencies.<name>] 表切块，块内须含
    git = ".../mmqz/mdcore"，再收块内 rev = "<REV>"——避免误收其他
    git 依赖（如 zip/serde 等注册表源不含 rev，或未来其他 git 依赖）。
    """
    if not toml_path.is_file():
        return set()
    text = toml_path.read_text(encoding="utf-8")
    revs: set[str] = set()
    # [dependencies.x] / [dependencies.x.y.z] 表头（含 target 门限表也一并覆盖）
    for m in re.finditer(
        r"^\[dependencies[^\]]*\]\s*$", text, re.MULTILINE
    ):
        block_start = m.end()
        nxt = re.search(r"^\[", text[block_start:], re.MULTILINE)
        block = text[block_start : block_start + (nxt.start() if nxt else len(text) - block_start)]
        if f"git = \"https://{MDCORE_GIT}\"" in block or (
            f"git = \"https://{MDCORE_GIT}/\"" in block
        ):
            revs.update(TOML_REV_RE.findall(block))
    return revs


def fmt(revs: set[str]) -> str:
    return ", ".join(sorted(r[:12] for r in revs)) or "(空)"


def main() -> int:
    root = Path(sys.argv[sys.argv.index("--root") + 1]) if "--root" in sys.argv else Path.cwd()

    prod_lock = lock_mdcore_revs(root / "bidcore" / "Cargo.lock")
    prod_manifest = toml_mdcore_revs(root / "bidcore" / "crates" / "bid-engine" / "Cargo.toml")
    probe_manifest = toml_mdcore_revs(root / "scripts" / "engine_probe" / "Cargo.toml")
    probe_lock = lock_mdcore_revs(root / "scripts" / "engine_probe" / "Cargo.lock")

    print("== mdcore rev 一致性检查（FG-08 / #304）==")
    print(f"  生产 lock      (bidcore/Cargo.lock):                {fmt(prod_lock)}")
    print(f"  生产 manifest  (bid-engine/Cargo.toml):            {fmt(prod_manifest)}")
    print(f"  探针 manifest  (scripts/engine_probe/Cargo.toml):  {fmt(probe_manifest)}")
    print(f"  探针 lock      (scripts/engine_probe/Cargo.lock):  {fmt(probe_lock)}")

    errors: list[str] = []

    if not prod_lock:
        errors.append("生产 Cargo.lock 未找到 mdcore git 源——检查解析或依赖结构变更")
    elif len(prod_lock) > 1:
        errors.append(
            f"生产 Cargo.lock 的 mdcore rev 不唯一（{fmt(prod_lock)}）——"
            "生产自身分裂，先统一生产侧再谈探针对账"
        )

    if prod_manifest != prod_lock:
        errors.append(
            f"bid-engine/Cargo.toml 钉的 rev（{fmt(prod_manifest)}）≠ 生产 Cargo.lock"
            f"（{fmt(prod_lock)}）——manifest 与 lock 漂移：改 manifest 后跑 "
            "`cargo update -p <mdcore 包>` 刷新 bidcore/Cargo.lock"
        )

    if not probe_lock:
        errors.append(
            "engine_probe/Cargo.lock 未找到 mdcore git 源——探针 lock 缺失或结构变更"
        )
    elif probe_lock != prod_lock:
        errors.append(
            f"FG-08 主断言：engine_probe Cargo.lock 的 mdcore rev（{fmt(probe_lock)}）"
            f"≠ 生产（{fmt(prod_lock)}）——双跑差分两侧引擎版本不同，差分结论无效。"
            "修复：同步 scripts/engine_probe/Cargo.toml 的 rev 到生产 rev，"
            "然后 `cd scripts/engine_probe && cargo update -p mdc-office-io -p mdc-pdf`"
        )

    if probe_manifest != probe_lock:
        errors.append(
            f"engine_probe/Cargo.toml 钉的 rev（{fmt(probe_manifest)}）≠ 其 Cargo.lock"
            f"（{fmt(probe_lock)}）——探针 manifest 与 lock 漂移：在 "
            "scripts/engine_probe/ 下跑 `cargo update -p mdc-office-io -p mdc-pdf`"
        )

    if errors:
        print()
        for e in errors:
            print(f"::error::{e}")
        print()
        print("结果: FAIL（rev 漂移——门禁拦截，见上）")
        return 1

    print()
    print(f"结果: PASS——生产与探针同 rev（{fmt(prod_lock)}），双跑差分对账口径成立")
    return 0


if __name__ == "__main__":
    sys.exit(main())
