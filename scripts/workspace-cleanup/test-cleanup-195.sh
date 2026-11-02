#!/usr/bin/env bash
# issue #195 —— cleanup-audit.ps1 模拟工作区测试（在 Linux pwsh 7.4.6 实测全绿 68/68）
# 用法: bash test-cleanup-195.sh   （需 PATH 里有 pwsh，或 PWSH=/path/to/pwsh bash test-cleanup-195.sh）
set -u
PWSH=${PWSH:-$(command -v pwsh 2>/dev/null || true)}
if [ -z "$PWSH" ] && [ -x /tmp/pwsh/pwsh ]; then PWSH=/tmp/pwsh/pwsh; fi
if [ -z "$PWSH" ]; then echo "需要 pwsh：请安装或 PWSH=/path/to/pwsh 指定"; exit 1; fi
SCRIPT=$(cd "$(dirname "$0")" && pwd)/cleanup-audit.ps1
MOCK=${MOCK:-/tmp/mock-audit}
PASS=0; FAIL=0

ok()   { PASS=$((PASS+1)); echo "  ✓ $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  ✗ $1"; }
check(){ if eval "$2" >/dev/null 2>&1; then ok "$1"; else bad "$1"; fi }
section(){ echo; echo "==== $1 ===="; }

setup_mock() {
  rm -rf "$MOCK"; mkdir -p "$MOCK"; cd "$MOCK" || exit 1
  # 两个 mock git 仓库
  for r in ai-bid-research mdcore; do
    mkdir "$r" && cd "$r" && git init -q . && git config user.email t@t && git config user.name t \
      && echo "fn main(){}" > main.rs && git add -A && git commit -qm init && cd ..
  done
  # _pack
  mkdir _pack && echo pack1 > _pack/a.txt && echo pack2 > _pack/b.zip
  # 保留名单（精确）
  echo "audit doc"   > AUDIT-engine-vs-bid.md
  echo "test pipe"   > TEST-REAL-DOC-PIPELINE.md
  echo "word deep"   > TEST-WORD-DEEP.md
  echo "ci burst"    > CI-BURST-README.md
  echo "sample"      > engine-export.docx
  echo "sample"      > engine-export.pdf
  echo "zipdata"     > mang-bid-3d76c7ee-20261102.zip
  # 保留名单（模式）
  echo ui     > ui-main.png
  echo pdfshot> pdf-export-view.png
  echo canvas > canvas-draw.png
  # A 类：纯日志（92 个可删 + 1 个证据名）
  for i in $(seq 1 60); do printf '2026-11-01 10:%02d:00 INFO line %d\n' "$i" "$i" > "_w168b_$i.log"; done
  for i in $(seq 1 30); do echo "debug $i" > "_m93_bb_$i.log"; done
  echo "run log" > _ts_mdcocmain.log
  echo "ci log"  > ci-run-41388819066.log
  echo "important evidence" > evidence-run42.log
  # .txt：调试名 15 + 日志形内容 8 + 正常 1
  for i in $(seq 1 15); do printf '2026-10-%02d 08:00:00 probe ok\n' "$i" > "_probe_$i.txt"; done
  # 日志形内容 txt：12 个文件 × 每个 12 行时间戳日志
  for i in $(seq 1 12); do
    : > "buildlog$i.txt"
    for j in $(seq 1 12); do printf '2026-10-%02d 09:00:00 build step %d\n' "$i" "$j" >> "buildlog$i.txt"; done
  done
  echo "meeting notes tomorrow" > notes.txt
  # .md：调试名 1 + 正常 1
  echo "# debug dump"    > _dump1.md
  echo "# some analysis" > analysis-notes.md
  # B 类：脚本 5 个
  printf 'Invoke-WebRequest "https://api.github.com/repos/x/y/actions/runs/123/logs" -OutFile _run123.log\n' > _fetch_run123.ps1
  printf 'param([string]$RunId)\nInvoke-WebRequest "https://api.github.com/repos/x/y/actions/runs/$RunId/logs" -OutFile run.log\n' > fetch-ci-logs.ps1
  printf 'gh run view 41388819066 --log > _gh.log\n' > gh-view.bat
  printf 'import re\nfor line in open("_w168b_1.log"):\n    print(re.findall(r"[0-9]+", line))\n' > scratch-parse.py
  printf 'print("hello world")\n' > hello.py
  # D 类：产物 5 个
  echo '{"run":1}' > _dump.json
  echo img > screenshot-temp.png
  echo z   > temp-export.zip
  echo "fn f(){}" > mod_dump.rs
  echo "cfg" > KEEP-config.json
  # 隐藏 2 个
  echo "A=1" > .env
  echo "*.log" > .gitignore
  # 子目录
  mkdir -p _target_debug/deps _target_debug/.fingerprint
  echo lib > _target_debug/deps/libfoo.rlib; echo f > _target_debug/.fingerprint/x; echo meta > _target_debug/meta.log
  mkdir -p _syntax; for i in 1 2 3; do echo "syntax err $i" > "_syntax/_s$i.log"; done
  mkdir -p _docx_e2e; echo docx > _docx_e2e/input.docx; echo '{"expected":1}' > _docx_e2e/expected.json
  mkdir -p _git2test && cd _git2test && git init -q . && git config user.email t@t && git config user.name t \
    && echo x > x.txt && git add -A && git commit -qm i && cd ..
  mkdir -p _dx; echo a > _dx/a.log; echo b > _dx/notes.md
  mkdir -p _mdcore_tmp; echo t > _mdcore_tmp/tmp_probe.txt
  mkdir -p _tables_stage; echo t1 > _tables_stage/table-stage.txt; echo d > _tables_stage/data.json
  mkdir _empty_dir
  mkdir -p random_notes; echo n > random_notes/note.md
  cd /tmp
}

run_ps(){ $PWSH -NoProfile -File "$SCRIPT" -AuditRoot "$1" "${@:2}"; }

top_files(){ find "$MOCK" -maxdepth 1 -type f | wc -l; }

if [ ! -f "$SCRIPT" ]; then echo "找不到 $SCRIPT"; exit 1; fi

# ============================================================
section "T1 干跑：不移动不删除任何文件"
setup_mock
BEFORE=$(top_files)
run_ps "$MOCK" > /tmp/t1.out 2>&1; RC=$?
check "退出码 0" "[ $RC -eq 0 ]"
check "干跑后顶层文件数不变 ($BEFORE)" "[ \$(top_files) -eq $BEFORE ]"
check "清单 CSV 已生成" "[ -n \"\$(ls $MOCK/_work/logs/*cleanup-manifest-*.csv 2>/dev/null)\" ]"
check "日志已生成" "[ -n \"\$(ls $MOCK/_work/logs/*cleanup-DryRun.log 2>/dev/null)\" ]"
check "未建 _archive（干跑零副作用）" "[ ! -d $MOCK/_archive ]"
check "未建 docs" "[ ! -d $MOCK/docs ]"
check "未建回收站 .trash" "[ ! -d $MOCK/_work/.trash ]"
grep -q "未移动、未删除任何文件" /tmp/t1.out && ok "输出含干跑确认语" || bad "输出缺干跑确认语"
grep -q "干跑预估" /tmp/t1.out && ok "输出含预估行" || bad "输出缺预估行"

# ============================================================
section "T2 执行：完整清理流程（分类/保留/归档/护栏）"
PACK_MD5_BEFORE=$(cd "$MOCK/_pack" && md5sum * | md5sum)
run_ps "$MOCK" -Execute > /tmp/t2.out 2>&1; RC=$?
check "退出码 0" "[ $RC -eq 0 ]"
check "顶层文件仅剩 3 样本 + CLEANUP.md = 4 个（实测 $(top_files)）" "[ \$(top_files) -eq 4 ]"
check "mang-bid zip 保留在顶层" "[ -f $MOCK/mang-bid-3d76c7ee-20261102.zip ]"
check "engine-export.docx 保留" "[ -f $MOCK/engine-export.docx ]"
check "engine-export.pdf 保留" "[ -f $MOCK/engine-export.pdf ]"
check "4 个保留 .md 已挪 docs/" "[ -f $MOCK/docs/AUDIT-engine-vs-bid.md ] && [ -f $MOCK/docs/TEST-REAL-DOC-PIPELINE.md ] && [ -f $MOCK/docs/TEST-WORD-DEEP.md ] && [ -f $MOCK/docs/CI-BURST-README.md ]"
check "3 张截图挪 docs/screenshots/" "[ -f $MOCK/docs/screenshots/ui-main.png ] && [ -f $MOCK/docs/screenshots/pdf-export-view.png ] && [ -f $MOCK/docs/screenshots/canvas-draw.png ]"
check "证据名日志被归档而非删除" "[ -f $MOCK/_archive/txt-review/evidence-run42.log ]"
check "可复用脚本归档 scripts-review" "[ -f $MOCK/_archive/scripts-review/fetch-ci-logs.ps1 ] && [ -f $MOCK/_archive/scripts-review/hello.py ]"
check "正常 md 归档 docs-review" "[ -f $MOCK/_archive/docs-review/analysis-notes.md ]"
check "正常 txt 归档 txt-review" "[ -f $MOCK/_archive/txt-review/notes.txt ]"
check "产物归档 artifacts-review（KEEP-config.json/mod_dump.rs）" "[ -f $MOCK/_archive/artifacts-review/KEEP-config.json ] && [ -f $MOCK/_archive/artifacts-review/mod_dump.rs ]"
check "隐藏文件归档 unknown-review（.env/.gitignore）" "[ -f $MOCK/_archive/unknown-review/.env ] && [ -f $MOCK/_archive/unknown-review/.gitignore ]"
check "夹具目录整体归档 _docx_e2e" "[ -f $MOCK/_archive/subdirs/_docx_e2e/input.docx ]"
check "git 实验仓库整体归档 _git2test（.git 完好）" "[ -d $MOCK/_archive/subdirs/_git2test/.git ]"
check "混杂目录归档 _dx/_tables_stage/random_notes" "[ -d $MOCK/_archive/subdirs/_dx ] && [ -d $MOCK/_archive/subdirs/_tables_stage ] && [ -d $MOCK/_archive/subdirs/random_notes ]"
check "构建目录进回收站 _target_debug" "[ -d $MOCK/_work/.trash/*/dirs/_target_debug ]"
check "纯日志目录进回收站 _syntax" "[ -d $MOCK/_work/.trash/*/dirs/_syntax ]"
check "临时目录进回收站 _mdcore_tmp/_empty_dir" "[ -d $MOCK/_work/.trash/*/dirs/_mdcore_tmp ] && [ -d $MOCK/_work/.trash/*/dirs/_empty_dir ]"
check "调试日志进回收站（抽样 _w168b_1.log）" "[ -f $MOCK/_work/.trash/*/files/_w168b_1.log ]"
check "日志形 txt 进回收站（buildlog1.txt）" "[ -f $MOCK/_work/.trash/*/files/buildlog1.txt ]"
check "一次性脚本进回收站（_fetch_run123.ps1/gh-view.bat）" "[ -f $MOCK/_work/.trash/*/files/_fetch_run123.ps1 ] && [ -f $MOCK/_work/.trash/*/files/gh-view.bat ]"
TRASHN=$(find "$MOCK/_work/.trash" -mindepth 3 -maxdepth 3 -type f 2>/dev/null | wc -l)
check "回收站文件数 = 126（实测 $TRASHN）" "[ $TRASHN -eq 126 ]"
ARCHN=$(find "$MOCK/_archive" -type f -not -path '*/.git/*' | wc -l)
check "归档文件数 = 17（实测 $ARCHN）" "[ $ARCHN -eq 17 ]"
check "ai-bid-research git status 干净" "[ -z \"\$(git -C $MOCK/ai-bid-research status --porcelain)\" ]"
check "mdcore git status 干净" "[ -z \"\$(git -C $MOCK/mdcore status --porcelain)\" ]"
check "_pack 完好（md5 一致）" "[ \"\$(cd $MOCK/_pack && md5sum * | md5sum)\" = '$PACK_MD5_BEFORE' ]"
check "_work/logs 结构存在" "[ -d $MOCK/_work/logs ]"
check "_work/scripts 结构存在" "[ -d $MOCK/_work/scripts ]"
check "脚本已自拷贝到 _work/scripts" "[ -f $MOCK/_work/scripts/cleanup-audit.ps1 ]"
check "_work/README.md 已写" "[ -f $MOCK/_work/README.md ]"
check "audit 根 CLEANUP.md 已生成" "[ -f $MOCK/CLEANUP.md ]"
grep -q "验收①达标" /tmp/t2.out && ok "输出含验收达标行" || bad "输出缺验收达标行"
grep -qi "cargo" "$MOCK/CLEANUP.md" && ok "CLEANUP.md 含 cargo 检查结果（SKIP 也显式登记）" || bad "CLEANUP.md 缺 cargo 检查结果"

# ============================================================
section "T3 幂等复跑"
run_ps "$MOCK" -Execute > /tmp/t3.out 2>&1; RC=$?
check "复跑退出码 0" "[ $RC -eq 0 ]"
check "复跑后顶层文件仍 4 个（实测 $(top_files)）" "[ \$(top_files) -eq 4 ]"
check "docs 内容未受影响" "[ -f $MOCK/docs/AUDIT-engine-vs-bid.md ]"
ARCHN2=$(find "$MOCK/_archive" -type f -not -path '*/.git/*' | wc -l)
check "归档文件数仍 17（实测 $ARCHN2）" "[ $ARCHN2 -eq 17 ]"

# ============================================================
section "T4 周清理：7 天前的清掉，新的保留，scripts/README 不动"
touch -d "2026-01-01" "$MOCK/_work/logs/20260101-old.log"
mkdir -p "$MOCK/_work/.trash/20200101-000000/files" && echo old > "$MOCK/_work/.trash/20200101-000000/files/x.log"
run_ps "$MOCK" -Weekly > /tmp/t4.out 2>&1; RC=$?
check "周清理退出码 0" "[ $RC -eq 0 ]"
check "7 天前旧日志已删" "[ ! -f $MOCK/_work/logs/20260101-old.log ]"
check "过期回收站批次已整批删除" "[ ! -d $MOCK/_work/.trash/20200101-000000 ]"
check "今天的日志保留" "[ -n \"\$(ls $MOCK/_work/logs/*cleanup-*.log 2>/dev/null)\" ]"
check "_work/scripts 自拷贝保留" "[ -f $MOCK/_work/scripts/cleanup-audit.ps1 ]"
check "_work/README.md 保留" "[ -f $MOCK/_work/README.md ]"

# ============================================================
section "T5 清空回收站（-Yes 直清；无 -Yes 时非交互默认取消）"
echo n | run_ps "$MOCK" -PurgeTrash > /tmp/t5a.out 2>&1
grep -q "已取消" /tmp/t5a.out && ok "非交互且答 n → 取消" || bad "非交互取消逻辑异常"
check "回收站仍存在" "[ -d $MOCK/_work/.trash ]"
run_ps "$MOCK" -PurgeTrash -Yes > /tmp/t5b.out 2>&1; RC=$?
check "PurgeTrash -Yes 退出码 0" "[ $RC -eq 0 ]"
LEFT=$(find "$MOCK/_work/.trash" -mindepth 1 2>/dev/null | wc -l)
check "回收站已清空（实测剩 $LEFT）" "[ $LEFT -eq 0 ]"

# ============================================================
section "T6 脏仓库：可以执行，但仓库状态必须与基线一致（不能误伤也不能静默改库）"
setup_mock
echo dirty > "$MOCK/mdcore/uncommitted.txt"
run_ps "$MOCK" -Execute > /tmp/t6.out 2>&1; RC=$?
check "退出码 0" "[ $RC -eq 0 ]"
check "脏文件仍在（未被脚本动过）" "[ -f $MOCK/mdcore/uncommitted.txt ]"
check "git status 仍只有那一个脏文件" "[ \$(git -C $MOCK/mdcore status --porcelain | wc -l) -eq 1 ]"
grep -q "git status 非空" /tmp/t6.out && ok "开跑前有基线警告" || bad "缺基线警告"

# ============================================================
section "T7 错误路径 / audit 本身是仓库 / audit 在上层仓库内 —— 一律拒绝"
setup_mock
run_ps /nonexistent-path > /tmp/t7a.out 2>&1
check "不存在的路径 → 拒绝（退出码非 0）" "[ $? -ne 0 ]"
grep -q "不存在" /tmp/t7a.out && ok "报错信息明确" || bad "报错信息不明确"
cd "$MOCK" && git init -q . && cd /tmp
run_ps "$MOCK" > /tmp/t7b.out 2>&1
grep -q "本身是一个 git 仓库" /tmp/t7b.out && ok "audit 自身是仓库 → 拒绝" || bad "audit 自身是仓库未被拒绝"
rm -rf "$MOCK/.git"
rm -rf /tmp/mock-outer; mkdir -p /tmp/mock-outer/repo && cd /tmp/mock-outer/repo && git init -q . && cd /tmp
mv "$MOCK" /tmp/mock-outer/repo/mock-audit
run_ps /tmp/mock-outer/repo/mock-audit > /tmp/t7c.out 2>&1
grep -q "上层 git 仓库" /tmp/t7c.out && ok "audit 在上层仓库内 → 拒绝" || bad "上层仓库检测失效"
mv /tmp/mock-outer/repo/mock-audit /tmp/mock-audit; rm -rf /tmp/mock-outer

# ============================================================
section "T8 清单 CSV 内容抽检"
run_ps "$MOCK" > /tmp/t8.out 2>&1
CSV=$(ls "$MOCK"/_work/logs/*cleanup-manifest-*.csv | head -1)
check "CSV 存在" "[ -f '$CSV' ]"
ROWC=$($PWSH -NoProfile -Command "(Import-Csv '$CSV' | Measure-Object).Count")
check "清单行数 = 145 文件 + 12 目录 + _work = 158（实测 $ROWC）" "[ $ROWC -eq 158 ]"
V=$($PWSH -NoProfile -Command "(Import-Csv '$CSV' | Where-Object { \$_.Verdict -eq 'KEEP' } | Measure-Object).Count")
check "KEEP 判定 10 项（实测 $V）" "[ $V -eq 10 ]"

echo
echo "=========================================="
echo "结果: PASS=$PASS FAIL=$FAIL"
[ $FAIL -eq 0 ] && echo "ALL GREEN ✅" || echo "HAS FAILURES ❌"
exit $([ $FAIL -eq 0 ] && echo 0 || echo 1)
