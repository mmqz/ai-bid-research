#Requires -Version 5.1
<#
.SYNOPSIS
    audit/ 工作区卫生清理脚本（issue #195）。

.DESCRIPTION
    针对 E:\Code\ai\md\audit 顶层的散落文件与临时子目录做分类清理。
    安全模型（四层防护）：
      1. 默认 dry-run：不动任何文件，只产出清单 CSV + 摘要；
      2. 所有"删除"都是移入 _work/.trash/<时间戳>/，有 7 天缓冲期，
         到期由 -Weekly 或手动 -PurgeTrash 才真正永久删除；
      3. 不确定的一律挪 _archive/，绝不直接删；
      4. 分批操作（默认每批 500 项），每批之后校验两个仓库 git status
         与保护目录仍然完好，一旦异常立即中止。

    保护名单（绝不触碰）：ai-bid-research/  mdcore/  _pack/  _archive/  _work/  docs/

.PARAMETER AuditRoot
    audit 工作区根目录，默认 E:\Code\ai\md\audit。

.PARAMETER Execute
    实际执行（默认为 dry-run 干跑）。

.PARAMETER PurgeTrash
    永久清空 _work/.trash/（不可恢复，需确认或 -Yes）。

.PARAMETER Weekly
    周清理：删除 _work/ 下 7 天前的文件与过期回收站暂存。

.EXAMPLE
    .\cleanup-audit.ps1 -AuditRoot E:\Code\ai\md\audit
    # 干跑：只读 + 生成清单，不移动不删除

.EXAMPLE
    .\cleanup-audit.ps1 -AuditRoot E:\Code\ai\md\audit -Execute
    # 复核清单后实际执行

.EXAMPLE
    .\cleanup-audit.ps1 -AuditRoot E:\Code\ai\md\audit -PurgeTrash -Yes
    # 确认无误后永久清空回收站暂存

.NOTES
    兼容 Windows PowerShell 5.1 与 pwsh 7+；Linux pwsh 亦可跑（已实测）。
    控制台中文若乱码：chcp 65001 或改用 pwsh 7；日志/清单文件均为 UTF-8。
#>
[CmdletBinding(DefaultParameterSetName = 'DryRun')]
param(
    [Parameter(ParameterSetName = 'DryRun')]
    [Parameter(ParameterSetName = 'Execute')]
    [Parameter(ParameterSetName = 'PurgeTrash')]
    [Parameter(ParameterSetName = 'Weekly')]
    [string]$AuditRoot = 'E:\Code\ai\md\audit',

    [Parameter(ParameterSetName = 'Execute')]
    [switch]$Execute,

    [Parameter(ParameterSetName = 'PurgeTrash')]
    [switch]$PurgeTrash,

    [Parameter(ParameterSetName = 'Weekly')]
    [switch]$Weekly,

    [Parameter(ParameterSetName = 'PurgeTrash')]
    [Parameter(ParameterSetName = 'Weekly')]
    [switch]$Yes,

    [Parameter(ParameterSetName = 'Execute')]
    [ValidateRange(10, 5000)]
    [int]$BatchSize = 500,

    [Parameter(ParameterSetName = 'Execute')]
    [switch]$SkipCargoCheck
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# ============================================================
# 常量：保护名单 / 保留名单 / 分类规则
# ============================================================
$script:Version      = 'v1.0 (issue #195)'
$script:Protected    = @('ai-bid-research', 'mdcore', '_pack', '_archive', '_work', 'docs')

# issue #195 明确要保留的文件（精确名）
$script:KeepExact = @(
    'mang-bid-3d76c7ee-20261102.zip',   # 测试包
    'AUDIT-engine-vs-bid.md',           # 审计文档
    'TEST-REAL-DOC-PIPELINE.md',        # 测试说明
    'TEST-WORD-DEEP.md',                # 测试说明
    'CI-BURST-README.md',
    'engine-export.docx',               # 导出样本
    'engine-export.pdf'                 # 导出样本
)
# issue #195 明确要保留的文件名模式（当前 UI 截图）
$script:KeepPatterns = @('^ui-.+\.png$', '^pdf-.+\.png$', '^canvas-.+\.png$')

# 调试残留命名特征：下划线开头 / tmp、temp、debug、scratch、oneoff、throwaway、test- 前缀等
$script:DebugNameRegex = '(?i)^(?:_|tmp[-_.]|temp[-_.]|debug[-_.]|scratch[-_.]|oneoff[-_.]|throwaway[-_.]|test[-_])|(?:^|[-_.])(?:tmp|temp|scratch)(?:[-_.]|$)'
# 文件名暗示重要证据的特征：绝不进回收站，改归档人工判断
$script:EvidenceNameRegex = '(?i)evidence|zhengju|证据|保留|keep|重要|final|验收'

$script:ScriptExts     = @('.ps1', '.bat', '.cmd', '.py', '.vbs', '.js', '.sh')
$script:ArtifactExts   = @('.png', '.jpg', '.jpeg', '.gif', '.bmp', '.pdf', '.docx', '.doc', '.xlsx', '.xls', '.pptx', '.zip', '.7z', '.tar', '.gz', '.json', '.rs', '.csv', '.html', '.htm', '.xml')
$script:BuildJunkExts  = @('.log', '.txt', '.rlib', '.rmeta', '.pdb', '.d', '.o', '.obj', '.tmp', '.cache')
$script:BuildJunkDirNames = @('deps', 'build', 'incremental', '.fingerprint', 'out', 'dist', 'target', 'node_modules')

# 运行期全局
$script:Root  = $null
$script:RepoA = $null
$script:RepoM = $null
$script:BaseA = $null
$script:BaseM = $null
$script:LogFile = $null
$script:TrashDir = $null
$script:RunStamp = $null
$script:PackExisted = $false

# ============================================================
# 工具函数
# ============================================================
function Write-Log {
    param([string]$Level = 'INFO', [string]$Message = '')
    $line = '[{0}] [{1}] {2}' -f (Get-Date -Format 'HH:mm:ss'), $Level, $Message
    Write-Host $line
    if ($script:LogFile) {
        Add-Content -LiteralPath $script:LogFile -Value $line -Encoding UTF8
    }
}

function Test-DebugName {
    param([string]$Name)
    return ($Name -match $script:DebugNameRegex)
}

function Test-EvidenceGuard {
    param([string]$Name)
    return ($Name -match $script:EvidenceNameRegex)
}

function Get-TopLevelFiles {
    param([string]$Path)
    return @(Get-ChildItem -LiteralPath $Path -Force -ErrorAction Stop | Where-Object { -not $_.PSIsContainer })
}

function Get-TopLevelDirs {
    param([string]$Path)
    return @(Get-ChildItem -LiteralPath $Path -Force -ErrorAction Stop | Where-Object { $_.PSIsContainer })
}

function Get-GitStatusText {
    param([string]$RepoPath)
    try {
        $out = & git -C $RepoPath status --porcelain 2>&1
        if ($null -eq $out) { return '' }
        return ((@($out) | Where-Object { $_ }) -join "`n")
    } catch {
        return $null
    }
}

function Test-Integrity {
    # 护栏：仓库 .git 在、保护目录在（以基线存在与否为准）、git status 与基线一致。任一异常 → throw 中止。
    param([string]$Phase = '')
    foreach ($d in @('ai-bid-research', 'mdcore')) {
        if (-not (Test-Path -LiteralPath (Join-Path $script:Root $d))) {
            throw ("[护栏] 保护目录丢失: {0}（阶段: {1}）—— 立即中止" -f $d, $Phase)
        }
    }
    if ($script:PackExisted -and -not (Test-Path -LiteralPath (Join-Path $script:Root '_pack'))) {
        throw ("[护栏] 保护目录 _pack 丢失（阶段: {0}）—— 立即中止" -f $Phase)
    }
    foreach ($p in @($script:RepoA, $script:RepoM)) {
        if (-not (Test-Path -LiteralPath (Join-Path $p '.git'))) {
            throw ("[护栏] 保护仓库 .git 丢失: {0}（阶段: {1}）—— 立即中止" -f $p, $Phase)
        }
    }
    $a = Get-GitStatusText -RepoPath $script:RepoA
    $m = Get-GitStatusText -RepoPath $script:RepoM
    if ($null -eq $a -or $null -eq $m) {
        throw ("[护栏] git status 不可用（阶段: {0}）—— 立即中止" -f $Phase)
    }
    if ($a -ne $script:BaseA) {
        throw ("[护栏] ai-bid-research 的 git status 相对基线发生变化（阶段: {0}）—— 立即中止" -f $Phase)
    }
    if ($m -ne $script:BaseM) {
        throw ("[护栏] mdcore 的 git status 相对基线发生变化（阶段: {0}）—— 立即中止" -f $Phase)
    }
    return $true
}

function Move-ItemSafe {
    # 同卷移动到目标目录；重名自动加 -1/-2 后缀；失败（如被占用）不中断，保留原地并计数。
    param([string]$Path, [string]$DestDir, [ref]$FailCount)
    $leaf = Split-Path -Leaf $Path
    $dest = Join-Path $DestDir $leaf
    $i = 1
    while (Test-Path -LiteralPath $dest) {
        $dest = Join-Path $DestDir ('{0}-{1}' -f $leaf, $i)
        $i++
    }
    try {
        Move-Item -LiteralPath $Path -Destination $dest -ErrorAction Stop
        return $true
    } catch {
        Write-Log 'WARN' ('移动失败（保留原地，稍后可重跑）: {0} —— {1}' -f $leaf, $_.Exception.Message)
        if ($FailCount) { $FailCount.Value++ }
        return $false
    }
}

function Get-UniqueDest {
    param([string]$DestDir, [string]$Name)
    $dest = Join-Path $DestDir $Name
    $i = 1
    while (Test-Path -LiteralPath $dest) {
        $dest = Join-Path $DestDir ('{0}-{1}' -f $Name, $i)
        $i++
    }
    return $dest
}

# ============================================================
# 分类判定
# ============================================================
function Get-ScriptVerdict {
    # B 类：临时脚本 —— 扫内容判断"一次性"还是"可复用"
    param([System.IO.FileInfo]$File)
    if (Test-DebugName -Name $File.Name) { return @{ Verdict = 'TRASH'; Reason = '调试命名的一次性脚本' } }
    $lines = @()
    try { $lines = @(Get-Content -LiteralPath $File.FullName -TotalCount 300 -ErrorAction Stop) } catch { $lines = @() }
    if ($lines.Count -eq 0) { return @{ Verdict = 'TRASH'; Reason = '空脚本' } }
    $content = $lines -join "`n"
    $oneOff   = $false
    $reusable = $false
    if ($content -match 'actions/runs/\d+') { $oneOff = $true }
    if ($content -match '(?i)gh\s+run\s+(view|download|watch)') { $oneOff = $true }
    if ($content -match '(?i)curl\s+.*actions') { $oneOff = $true }
    if ($content -match '(?im)^\s*param\s*\(') { $reusable = $true }
    if ($content -match '(?im)^(function|def|class)\s+\w+') { $reusable = $true }
    if ($content -match '(?s)^<#') { $reusable = $true }
    if ($oneOff -and -not $reusable -and $lines.Count -le 150) {
        return @{ Verdict = 'TRASH'; Reason = '一次性 CI 日志抓取脚本（内容含 run 号/gh run，无可复用结构）' }
    }
    return @{ Verdict = 'ARCHIVE'; Reason = '脚本待人工复审（含可复用结构）' }
}

function Get-FileVerdict {
    param([System.IO.FileInfo]$File)
    $name = $File.Name
    $ext  = $File.Extension.ToLower()

    # -1) 脚本自身输出：保留原地（否则幂等复跑会把上一轮 CLEANUP.md 归档掉）
    if ($name -eq 'CLEANUP.md') {
        return @{ Verdict = 'LEAVE'; Category = 'DOC'; Reason = '脚本自身输出（audit/CLEANUP.md），保留原地' }
    }

    # 0) issue #195 保留名单（最高优先级）
    if ($script:KeepExact -contains $name) {
        return @{ Verdict = 'KEEP'; Category = 'KEEP'; Reason = 'issue #195 保留名单' }
    }
    foreach ($p in $script:KeepPatterns) {
        if ($name -match $p) { return @{ Verdict = 'KEEP'; Category = 'KEEP'; Reason = 'issue #195 保留名单（UI 截图）' } }
    }

    # 1) 隐藏文件：绝不删（.env 可能含密钥）
    if ($name.StartsWith('.')) {
        return @{ Verdict = 'ARCHIVE'; Category = 'HIDDEN'; Reason = '隐藏文件待人工（.env 等可能含敏感信息，只归档不删）' }
    }

    $guard = Test-EvidenceGuard -Name $name

    # 2) A 类：纯日志
    if ($ext -eq '.log') {
        if ($guard) { return @{ Verdict = 'ARCHIVE'; Category = 'LOG'; Reason = '文件名提示证据，归档待人工' } }
        return @{ Verdict = 'TRASH'; Category = 'LOG'; Reason = '纯调试日志' }
    }

    # 3) .txt：调试命名或内容呈日志形 → 回收；否则归档
    if ($ext -eq '.txt') {
        if (Test-DebugName -Name $name) { return @{ Verdict = 'TRASH'; Category = 'TXT'; Reason = '调试命名的临时文本' } }
        $sample = @()
        try { $sample = @(Get-Content -LiteralPath $File.FullName -TotalCount 60 -ErrorAction Stop) } catch { $sample = @() }
        $nonEmpty = @($sample | Where-Object { $_ -and $_.Trim() })
        if ($nonEmpty.Count -ge 10) {
            $logLike = @($nonEmpty | Where-Object { $_ -match '^\s*(\d{4}[-/]\d{2}[-/]\d{2}|\d{2}:\d{2}:\d{2}|\d{4}-\d{2}-\d{2}T)' })
            if ($logLike.Count / $nonEmpty.Count -ge 0.7) {
                return @{ Verdict = 'TRASH'; Category = 'TXT'; Reason = '内容呈日志形（时间戳行占比≥70%）' }
            }
        }
        return @{ Verdict = 'ARCHIVE'; Category = 'TXT'; Reason = '文本待人工复审' }
    }

    # 4) C 类：文档
    if ($ext -eq '.md') {
        if (Test-DebugName -Name $name) { return @{ Verdict = 'TRASH'; Category = 'DOC'; Reason = '调试命名的临时文档' } }
        return @{ Verdict = 'ARCHIVE'; Category = 'DOC'; Reason = '文档待人工复审（有价值者人工挪 docs/）' }
    }

    # 5) B 类：临时脚本（扫内容）
    if ($script:ScriptExts -contains $ext) {
        $v = Get-ScriptVerdict -File $File
        return @{ Verdict = $v.Verdict; Category = 'SCRIPT'; Reason = $v.Reason }
    }

    # 6) D 类：产物/样本
    if ($script:ArtifactExts -contains $ext) {
        if (Test-DebugName -Name $name) { return @{ Verdict = 'TRASH'; Category = 'ARTIFACT'; Reason = '调试命名的临时产物' } }
        return @{ Verdict = 'ARCHIVE'; Category = 'ARTIFACT'; Reason = '产物/样本待人工复审' }
    }

    # 7) 未知类型：归档
    return @{ Verdict = 'ARCHIVE'; Category = 'OTHER'; Reason = '未知类型待人工复审' }
}

function Get-DirVerdict {
    param([System.IO.DirectoryInfo]$Dir)
    $lower = $Dir.Name.ToLower()
    if ($script:Protected -contains $lower) {
        return @{ Verdict = 'LEAVE'; Reason = '保护目录，绝不触碰' }
    }
    if (Test-Path -LiteralPath (Join-Path $Dir.FullName '.git')) {
        return @{ Verdict = 'ARCHIVE'; Reason = '疑似 git 仓库（含 .git），整体归档待人工判断' }
    }
    $entries = @()
    try {
        $entries = @(Get-ChildItem -LiteralPath $Dir.FullName -Recurse -Force -ErrorAction SilentlyContinue | Select-Object -First 3000)
    } catch { $entries = @() }
    if ($entries.Count -eq 0) { return @{ Verdict = 'TRASH'; Reason = '空目录' } }

    $junk = 0
    $fixtureHint = $false
    foreach ($e in $entries) {
        $isJunk = $false
        if ($e.PSIsContainer) {
            if ($script:BuildJunkDirNames -contains $e.Name.ToLower()) { $isJunk = $true }
        } else {
            $x = $e.Extension.ToLower()
            if ($script:BuildJunkExts -contains $x) { $isJunk = $true }
            elseif (Test-DebugName -Name $e.Name) { $isJunk = $true }
            if (@('.docx', '.xlsx', '.json') -contains $x -and $e.Name -match '(?i)(input|expected|sample|golden|fixture)') { $fixtureHint = $true }
        }
        if ($isJunk) { $junk++ }
    }
    $pct = [int](100.0 * $junk / $entries.Count)
    if ($pct -ge 80) {
        return @{ Verdict = 'TRASH'; Reason = ('构建产物/日志堆积（垃圾占比 {0}%）' -f $pct) }
    }
    $r = ('内容混杂（垃圾占比 {0}%），归档待人工' -f $pct)
    if ($fixtureHint) { $r += '；疑似测试夹具，建议人工确认后挪入对应仓库 fixtures/' }
    return @{ Verdict = 'ARCHIVE'; Reason = $r }
}

# ============================================================
# 预检
# ============================================================
function Invoke-Preflight {
    param([bool]$RequireCleanRepos)

    if (-not (Test-Path -LiteralPath $script:Root)) {
        throw ("audit 根目录不存在: {0}" -f $script:Root)
    }
    $resolved = (Resolve-Path -LiteralPath $script:Root).ProviderPath
    $rootOfRoots = [System.IO.Path]::GetPathRoot($resolved)
    if ($resolved.TrimEnd('\', '/') -eq $rootOfRoots.TrimEnd('\', '/')) {
        throw ('拒绝在文件系统根目录上运行')
    }

    if (Test-Path -LiteralPath (Join-Path $script:Root '.git')) {
        throw ('audit 根目录本身是一个 git 仓库（issue #195 明确 audit/ 不是仓库）—— 拒绝运行')
    }
    $inside = $null
    try { $inside = & git -C $script:Root rev-parse --is-inside-work-tree 2>&1 } catch { $inside = $null }
    if ("$inside" -match 'true') {
        throw ('audit 根目录位于某个上层 git 仓库工作区内 —— 拒绝运行（散落文件可能被跟踪）')
    }

    foreach ($d in @('ai-bid-research', 'mdcore')) {
        $repo = Join-Path $script:Root $d
        if (-not (Test-Path -LiteralPath (Join-Path $repo '.git'))) {
            throw ('保护仓库缺少 .git: {0} —— 路径不对，拒绝运行' -f $repo)
        }
    }
    $script:PackExisted = Test-Path -LiteralPath (Join-Path $script:Root '_pack')
    if (-not $script:PackExisted) {
        Write-Log 'WARN' '未发现 _pack/ 目录（issue #195 说它存在）—— 将不作为护栏条件，请人工确认路径无误'
    }

    $script:RepoA = Join-Path $script:Root 'ai-bid-research'
    $script:RepoM = Join-Path $script:Root 'mdcore'
    $script:BaseA = Get-GitStatusText -RepoPath $script:RepoA
    $script:BaseM = Get-GitStatusText -RepoPath $script:RepoM
    if ($null -eq $script:BaseA -or $null -eq $script:BaseM) {
        throw 'git 不可用（PATH 里没有 git？）—— 拒绝运行'
    }
    if ($script:BaseA -ne '') { Write-Log 'WARN' ('ai-bid-research 当前 git status 非空（{0} 行），以它为基线，任何额外变化都会触发中止' -f @($script:BaseA -split "`n").Count) }
    if ($script:BaseM -ne '') { Write-Log 'WARN' ('mdcore 当前 git status 非空（{0} 行），以它为基线，任何额外变化都会触发中止' -f @($script:BaseM -split "`n").Count) }
}

# ============================================================
# 统计 / 报告
# ============================================================
function Get-TopStats {
    param([string]$Path)
    $files = Get-TopLevelFiles -Path $Path
    $dirs  = Get-TopLevelDirs -Path $Path
    $bytes = 0
    foreach ($f in $files) { $bytes += $f.Length }
    return @{ FileCount = $files.Count; DirCount = $dirs.Count; Bytes = $bytes }
}

function Invoke-CargoCheck {
    param([string]$RepoPath, [string]$Label)
    if ($SkipCargoCheck) { return ('{0}: SKIP（-SkipCargoCheck）' -f $Label) }
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) { return ('{0}: SKIP（PATH 无 cargo，未验证）' -f $Label) }
    $manifest = @(Get-ChildItem -LiteralPath $RepoPath -Recurse -Depth 1 -Filter 'Cargo.toml' -ErrorAction SilentlyContinue | Select-Object -First 1)
    if ($manifest.Count -eq 0) { return ('{0}: SKIP（未找到 Cargo.toml）' -f $Label) }
    try { & cargo metadata --no-deps --format-version 1 --manifest-path $manifest[0].FullName *> $null } catch { }
    if ($LASTEXITCODE -eq 0) { return ('{0}: OK（cargo metadata 通过）' -f $Label) }
    return ('{0}: FAIL（cargo metadata 退出码 {1}）' -f $Label, $LASTEXITCODE)
}

function New-WorkBaseline {
    # _work/ 结构 + 自拷贝 + README + 周清理说明
    $work    = Join-Path $script:Root '_work'
    $logs    = Join-Path $work 'logs'
    $scriptsD = Join-Path $work 'scripts'
    foreach ($d in @($logs, $scriptsD)) {
        if (-not (Test-Path -LiteralPath $d)) { New-Item -ItemType Directory -Path $d -Force | Out-Null }
    }
    # 脚本自拷贝（工作区自足，仓库挪走也能跑周清理）
    if ($PSCommandPath) {
        $selfDest = Join-Path $scriptsD 'cleanup-audit.ps1'
        $srcFull = [System.IO.Path]::GetFullPath($PSCommandPath)
        $dstFull = [System.IO.Path]::GetFullPath($selfDest)
        if (-not [string]::Equals($srcFull, $dstFull, [System.StringComparison]::OrdinalIgnoreCase)) {
            Copy-Item -LiteralPath $PSCommandPath -Destination $selfDest -Force
        }
    }
    $readme = Join-Path $work 'README.md'
    $readmeText = @'
# _work/ —— 唯一临时工作区

**规矩（issue #195）**：

1. 一切临时产物只写 `_work/`，不再散落 audit 顶层；
2. 日志统一写 `_work/logs/`，文件名带日期前缀：`yyyymmdd-xxx.log`；
3. 临时脚本写 `_work/scripts/`；
4. 每周清理一次 7 天前的文件（推荐计划任务，见下）；
5. `_work/.trash/` 是清理脚本的回收站暂存（7 天缓冲），到期由周任务永久删除；
   想立即清空：`powershell -File _work\scripts\cleanup-audit.ps1 -PurgeTrash -Yes`。

## 每周清理计划任务（一次性安装，管理员 PowerShell）

```bat
schtasks /Create /TN "audit-work-weekly-purge" /SC WEEKLY /D MON /ST 09:00 ^
  /TR "powershell.exe -NoProfile -ExecutionPolicy Bypass -File E:\Code\ai\md\audit\_work\scripts\cleanup-audit.ps1 -Weekly -AuditRoot E:\Code\ai\md\audit"
```

## 绝对红线

- `ai-bid-research/`、`mdcore/`、`_pack/` 与本目录平级的 `_archive/` 不是临时区，别往里塞调试产物；
- 清理脚本本体见 `scripts/workspace-cleanup/`（ai-bid-research 仓库）与本目录 `scripts/cleanup-audit.ps1`（自拷贝）。
'@
    Set-Content -LiteralPath $readme -Value $readmeText -Encoding UTF8
}

function New-CleanupMd {
    # 在 audit 根生成/刷新 CLEANUP.md（政策 + 本次实际统计）
    param(
        [hashtable]$Before, [hashtable]$After,
        [System.Collections.Generic.List[object]]$Decisions,
        [int]$MovedTrash, [int]$MovedArchive, [int]$MovedKeep, [int]$FailCount,
        [string]$TrashRel, [string]$IntegrityLine
    )
    $mb = { param($b) if ($b -ge 1MB) { '{0:N1} MB' -f ($b / 1MB) } else { '{0:N1} KB' -f ($b / 1KB) } }

    $grp = @{}
    foreach ($v in @('TRASH', 'ARCHIVE', 'KEEP', 'LEAVE')) {
        $items = @($Decisions | Where-Object { $_.Verdict -eq $v })
        $bytes = 0
        foreach ($it in $items) { $bytes += [long]$it.SizeBytes }
        $grp[$v] = @{ N = $items.Count; MB = $bytes }
    }

    $archivedDirs = @($Decisions | Where-Object { $_.Verdict -eq 'ARCHIVE' -and $_.Kind -eq 'Dir' } | ForEach-Object { $_.Name })
    $keptFiles = @($Decisions | Where-Object { $_.Verdict -eq 'KEEP' } | ForEach-Object { $_.Name })

    $sb = New-Object System.Text.StringBuilder
    [void]$sb.AppendLine('# CLEANUP.md —— audit/ 工作区卫生记录')
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine(('> 生成：cleanup-audit.ps1 {0}；时间：{1}' -f $script:Version, (Get-Date -Format 'yyyy-MM-dd HH:mm:ss')))
    [void]$sb.AppendLine('> 政策与规则全文见 ai-bid-research 仓库 `scripts/workspace-cleanup/CLEANUP.md`。')
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('## 一、本次清理统计')
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('| 指标 | 清理前 | 清理后 |')
    [void]$sb.AppendLine('| --- | --- | --- |')
    [void]$sb.AppendLine(('| 顶层文件数 | {0} | {1} |' -f $Before.FileCount, $After.FileCount))
    [void]$sb.AppendLine(('| 顶层目录数 | {0} | {1} |' -f $Before.DirCount, $After.DirCount))
    [void]$sb.AppendLine(('| 顶层文件体积 | {0} | {1} |' -f (& $mb $Before.Bytes), (& $mb $After.Bytes)))
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('| 判定 | 数量 | 体积 |')
    [void]$sb.AppendLine('| --- | --- | --- |')
    [void]$sb.AppendLine(('| 回收站暂存（{0}，7 天缓冲） | {1} | {2} |' -f $TrashRel, $grp['TRASH'].N, (& $mb $grp['TRASH'].MB)))
    [void]$sb.AppendLine(('| 归档 _archive/（待人工） | {0} | {1} |' -f $grp['ARCHIVE'].N, (& $mb $grp['ARCHIVE'].MB)))
    [void]$sb.AppendLine(('| 保留（名单/截图/样本） | {0} | {1} |' -f $grp['KEEP'].N, (& $mb $grp['KEEP'].MB)))
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine(('移动结果：回收 {0} 项 / 归档 {1} 项 / 保留移动 {2} 项 / 失败保留原地 {3} 项。' -f $MovedTrash, $MovedArchive, $MovedKeep, $FailCount))
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('## 二、保留了什么（名单 + 去向）')
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('- `.md` 保留文档 → `docs/`；UI 截图（ui-\*.png / pdf-\*.png / canvas-\*.png）→ `docs/screenshots/`；')
    [void]$sb.AppendLine('- `engine-export.docx` / `engine-export.pdf` / `mang-bid-*.zip` → 留在顶层；')
    if ($keptFiles.Count -gt 0) {
        foreach ($k in $keptFiles) { [void]$sb.AppendLine(('  - {0}' -f $k)) }
    }
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('## 三、归档了什么（_archive/）')
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('- `scripts-review/` 临时脚本、`docs-review/` 文档、`txt-review/` 文本、`artifacts-review/` 产物、`unknown-review/` 未知、`subdirs/` 整目录；')
    [void]$sb.AppendLine('- 每个文件为何归档见当次清单 `_work/logs/yyyymmdd-cleanup-manifest-*.csv`；')
    if ($archivedDirs.Count -gt 0) {
        [void]$sb.AppendLine('- 本次整目录归档：')
        foreach ($d in $archivedDirs) { [void]$sb.AppendLine(('  - `{0}`' -f $d)) }
    }
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('## 四、今后规矩')
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('1. 临时产物只写 `_work/`；日志 `_work/logs/yyyymmdd-xxx.log`；脚本 `_work/scripts/`；')
    [void]$sb.AppendLine('2. 每周自动清理 7 天前文件（`_work/README.md` 内有一键安装计划任务命令）；')
    [void]$sb.AppendLine('3. `_work/.trash/` 满了想立即清：`cleanup-audit.ps1 -PurgeTrash -Yes`；')
    [void]$sb.AppendLine('4. 不确定的东西一律进 `_archive/`，人工判断后再处置。')
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine('## 五、仓库完好性')
    [void]$sb.AppendLine('')
    [void]$sb.AppendLine($IntegrityLine)
    [void]$sb.AppendLine('')
    $dest = Join-Path $script:Root 'CLEANUP.md'
    Set-Content -LiteralPath $dest -Value $sb.ToString() -Encoding UTF8
    return $dest
}

# ============================================================
# 主流程
# ============================================================
try {
    $script:Root = $AuditRoot
    $mode = $PSCmdlet.ParameterSetName
    $script:RunStamp = '{0}-{1}' -f (Get-Date -Format 'yyyyMMdd'), (Get-Date -Format 'HHmmss')

    Write-Host ('==== cleanup-audit.ps1 {0} | 模式: {1} ====' -f $script:Version, $mode)

    # ---------- 模式：PurgeTrash ----------
    if ($mode -eq 'PurgeTrash') {
        Invoke-Preflight -RequireCleanRepos $false
        $trashRoot = Join-Path (Join-Path $script:Root '_work') '.trash'
        if (-not (Test-Path -LiteralPath $trashRoot)) { Write-Host '回收站暂存为空，无需清理。'; exit 0 }
        $items = @(Get-ChildItem -LiteralPath $trashRoot -Force)
        $bytes = 0
        foreach ($it in $items) {
            if ($it.PSIsContainer) { $bytes += (Get-ChildItem -LiteralPath $it.FullName -Recurse -Force -File -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum }
            else { $bytes += $it.Length }
        }
        Write-Host ('将永久删除 {0} 个暂存批次 / {1:N1} MB —— 不可恢复！' -f $items.Count, ($bytes / 1MB))
        if (-not $Yes) {
            $ans = Read-Host '确认请输入 YES'
            if ($ans -ne 'YES') { Write-Host '已取消。'; exit 0 }
        }
        foreach ($it in $items) { Remove-Item -LiteralPath $it.FullName -Recurse -Force }
        Write-Host ('已永久删除 {0} 个批次。' -f $items.Count)
        exit 0
    }

    # ---------- 模式：Weekly ----------
    if ($mode -eq 'Weekly') {
        if (-not (Test-Path -LiteralPath $script:Root)) { throw ('audit 根目录不存在: {0}' -f $script:Root) }
        $work = Join-Path $script:Root '_work'
        if (-not (Test-Path -LiteralPath $work)) { Write-Host '_work/ 不存在，无事可做。'; exit 0 }
        $cutoff = (Get-Date).AddDays(-7)
        $scriptsDir = Join-Path $work 'scripts'
        $trashRoot = Join-Path $work '.trash'
        # 注意：.trash/ 不按文件 mtime 清（旧日志 mtime 很老，但刚进缓冲期）；
        # 回收站统一按批次目录名的日期戳计算 7 天缓冲，见下方。
        $old = @(Get-ChildItem -LiteralPath $work -Recurse -Force -File -ErrorAction SilentlyContinue | Where-Object {
            $_.LastWriteTime -lt $cutoff -and
            (-not $_.FullName.StartsWith($scriptsDir, [System.StringComparison]::OrdinalIgnoreCase)) -and
            (-not $_.FullName.StartsWith($trashRoot, [System.StringComparison]::OrdinalIgnoreCase)) -and
            ($_.Name -ne 'README.md')
        })
        $n = 0
        foreach ($f in $old) { try { Remove-Item -LiteralPath $f.FullName -Force; $n++ } catch { } }
        # 过期回收站批次（按目录名日期戳）
        if (Test-Path -LiteralPath $trashRoot) {
            foreach ($batch in @(Get-ChildItem -LiteralPath $trashRoot -Force)) {
                if ($batch.Name -match '^(\d{8})' -and $batch.PSIsContainer) {
                    $d = [datetime]::ParseExact($Matches[1], 'yyyyMMdd', $null)
                    if ($d -lt $cutoff.Date) { Remove-Item -LiteralPath $batch.FullName -Recurse -Force }
                }
            }
        }
        # 清理空目录（保留 logs / scripts / .trash 骨架）
        foreach ($sub in @('logs', '.trash')) {
            $p = Join-Path $work $sub
            if (Test-Path -LiteralPath $p) {
                Get-ChildItem -LiteralPath $p -Recurse -Force -Directory -ErrorAction SilentlyContinue |
                    Where-Object { @(Get-ChildItem -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue).Count -eq 0 } |
                    ForEach-Object { try { Remove-Item -LiteralPath $_.FullName -Force } catch { } }
            }
        }
        Write-Host ('周清理完成：删除 {0} 个 7 天前文件；过期回收站批次已处理。' -f $n)
        exit 0
    }

    # ---------- 模式：DryRun / Execute 共用预检与分类 ----------
    Invoke-Preflight -RequireCleanRepos $false

    $before = Get-TopStats -Path $script:Root
    Write-Log 'INFO' ('清理前：顶层文件 {0} 个（{1:N1} MB），目录 {2} 个' -f $before.FileCount, ($before.Bytes / 1MB), $before.DirCount)

    # 日志目录（dry-run 也只建这一个目录，属唯一副作用）
    $logDir = Join-Path (Join-Path $script:Root '_work') 'logs'
    if (-not (Test-Path -LiteralPath $logDir)) { New-Item -ItemType Directory -Path $logDir -Force | Out-Null }
    $script:LogFile = Join-Path $logDir ('{0}-cleanup-{1}.log' -f (Get-Date -Format 'yyyyMMdd'), $mode)
    Write-Log 'INFO' ('日志: {0}' -f $script:LogFile)

    # ---- 分类 ----
    $Decisions = New-Object System.Collections.Generic.List[object]
    foreach ($f in (Get-TopLevelFiles -Path $script:Root)) {
        $v = Get-FileVerdict -File $f
        $dest = ''
        if ($v.Verdict -eq 'KEEP') {
            if ($f.Extension.ToLower() -eq '.md') { $dest = 'docs/' }
            elseif ($f.Extension.ToLower() -eq '.png') { $dest = 'docs/screenshots/' }
            else { $dest = '(原地保留)' }
        } elseif ($v.Verdict -eq 'TRASH') { $dest = '_work/.trash/<stamp>/'
        } elseif ($v.Verdict -eq 'ARCHIVE') {
            switch ($v.Category) {
                'SCRIPT'   { $dest = '_archive/scripts-review/' }
                'DOC'      { $dest = '_archive/docs-review/' }
                'TXT'      { $dest = '_archive/txt-review/' }
                'HIDDEN'   { $dest = '_archive/unknown-review/' }
                'LOG'      { $dest = '_archive/txt-review/' }
                'ARTIFACT' { $dest = '_archive/artifacts-review/' }
                default    { $dest = '_archive/unknown-review/' }
            }
        } else { $dest = '(原地保留)' }
        $Decisions.Add([pscustomobject]@{
            Name = $f.Name; Kind = 'File'; Extension = $f.Extension.ToLower()
            SizeBytes = $f.Length; Category = $v.Category; Verdict = $v.Verdict
            Reason = $v.Reason; Destination = $dest
        })
    }
    foreach ($d in (Get-TopLevelDirs -Path $script:Root)) {
        $v = Get-DirVerdict -Dir $d
        $size = (Get-ChildItem -LiteralPath $d.FullName -Recurse -Force -File -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
        if ($null -eq $size) { $size = 0 }
        $dest = ''
        if ($v.Verdict -eq 'TRASH') { $dest = '_work/.trash/<stamp>/dirs/' }
        elseif ($v.Verdict -eq 'ARCHIVE') { $dest = '_archive/subdirs/' }
        else { $dest = '(保护，不动)' }
        $Decisions.Add([pscustomobject]@{
            Name = $d.Name; Kind = 'Dir'; Extension = ''
            SizeBytes = $size; Category = 'DIR'; Verdict = $v.Verdict
            Reason = $v.Reason; Destination = $dest
        })
    }

    # ---- 清单 CSV ----
    $manifestPath = Join-Path $logDir ('{0}-cleanup-manifest-{1}.csv' -f (Get-Date -Format 'yyyyMMdd'), (Get-Date -Format 'HHmmss'))
    $Decisions | Export-Csv -LiteralPath $manifestPath -NoTypeInformation -Encoding UTF8

    # ---- 摘要 ----
    $sum = @{}
    foreach ($v in @('TRASH', 'ARCHIVE', 'KEEP', 'LEAVE')) {
        $items = @($Decisions | Where-Object { $_.Verdict -eq $v })
        $sum[$v] = $items.Count
    }
    $projFiles = $before.FileCount - $sum['TRASH'] - $sum['ARCHIVE'] - @($Decisions | Where-Object { $_.Verdict -eq 'KEEP' -and $_.Destination -ne '(原地保留)' }).Count
    Write-Log 'INFO' ('判定汇总：回收 {0} / 归档 {1} / 保留 {2} / 保护不动 {3}' -f $sum['TRASH'], $sum['ARCHIVE'], $sum['KEEP'], $sum['LEAVE'])
    Write-Log 'INFO' ('清单已写入: {0}' -f $manifestPath)

    if ($mode -eq 'DryRun') {
        Write-Log 'INFO' ('干跑预估：执行后顶层文件约 {0} 个（验收目标 < 100）' -f $projFiles)
        Write-Host ''
        Write-Host '==== 干跑完成：未移动、未删除任何文件。 ====' -ForegroundColor Green
        Write-Host ('下一步：人工复核清单 {0}，确认后加 -Execute 执行。' -f $manifestPath) -ForegroundColor Yellow
        exit 0
    }

    # ---------- 模式：Execute ----------
    if ($mode -ne 'Execute') { throw ('未知模式: {0}' -f $mode) }

    Write-Log 'INFO' '---- 开始执行（删除=移入回收站暂存，7 天缓冲）----'

    # 目录骨架
    $stamp = '{0}-{1}' -f (Get-Date -Format 'yyyyMMdd'), (Get-Date -Format 'HHmmss')
    $script:TrashDir = Join-Path (Join-Path (Join-Path $script:Root '_work') '.trash') $stamp
    $trashFiles = Join-Path $script:TrashDir 'files'
    $trashDirs  = Join-Path $script:TrashDir 'dirs'
    $buckets = @{}
    foreach ($b in @('scripts-review', 'docs-review', 'txt-review', 'artifacts-review', 'unknown-review', 'subdirs')) {
        $buckets[$b] = Join-Path (Join-Path $script:Root '_archive') $b
    }
    $docsDir = Join-Path $script:Root 'docs'
    $shotsDir = Join-Path $docsDir 'screenshots'
    foreach ($d in @($trashFiles, $trashDirs) + @($buckets.Values) + @($docsDir, $shotsDir)) {
        if (-not (Test-Path -LiteralPath $d)) { New-Item -ItemType Directory -Path $d -Force | Out-Null }
    }

    New-WorkBaseline
    Write-Log 'INFO' '_work/ 基线结构已建立（logs / scripts / README / 脚本自拷贝）'

    Test-Integrity -Phase '开工'

    # ---- 批量移动：TRASH 文件 ----
    $failCount = 0
    $trashFilesList = @($Decisions | Where-Object { $_.Verdict -eq 'TRASH' -and $_.Kind -eq 'File' })
    $movedTrash = 0
    $done = 0
    for ($i = 0; $i -lt $trashFilesList.Count; $i += $BatchSize) {
        $end = [math]::Min($i + $BatchSize, $trashFilesList.Count)
        for ($j = $i; $j -lt $end; $j++) {
            $src = Join-Path $script:Root $trashFilesList[$j].Name
            if (Move-ItemSafe -Path $src -DestDir $trashFiles -FailCount ([ref]$failCount)) { $movedTrash++ }
            $done++
        }
        Test-Integrity -Phase ('回收文件批次 {0}/{1}' -f $end, $trashFilesList.Count)
        Write-Log 'INFO' ('回收进度: {0}/{1}' -f $done, $trashFilesList.Count)
    }

    # ---- 批量移动：ARCHIVE 文件 ----
    $archFilesList = @($Decisions | Where-Object { $_.Verdict -eq 'ARCHIVE' -and $_.Kind -eq 'File' })
    $movedArchive = 0
    for ($i = 0; $i -lt $archFilesList.Count; $i += $BatchSize) {
        $end = [math]::Min($i + $BatchSize, $archFilesList.Count)
        for ($j = $i; $j -lt $end; $j++) {
            $bucketName = ($archFilesList[$j].Destination -replace '^_archive/', '' -replace '/$', '')
            $bucketDir = $buckets[$bucketName]
            if (-not $bucketDir) { $bucketDir = $buckets['unknown-review'] }
            $src = Join-Path $script:Root $archFilesList[$j].Name
            if (Move-ItemSafe -Path $src -DestDir $bucketDir -FailCount ([ref]$failCount)) { $movedArchive++ }
        }
        Test-Integrity -Phase ('归档文件批次 {0}/{1}' -f $end, $archFilesList.Count)
    }

    # ---- KEEP 文件移动（.md → docs/，.png → docs/screenshots/）----
    $movedKeep = 0
    foreach ($k in @($Decisions | Where-Object { $_.Verdict -eq 'KEEP' })) {
        if ($k.Destination -eq 'docs/') {
            if (Move-ItemSafe -Path (Join-Path $script:Root $k.Name) -DestDir $docsDir -FailCount ([ref]$failCount)) { $movedKeep++ }
        } elseif ($k.Destination -eq 'docs/screenshots/') {
            if (Move-ItemSafe -Path (Join-Path $script:Root $k.Name) -DestDir $shotsDir -FailCount ([ref]$failCount)) { $movedKeep++ }
        }
    }
    Test-Integrity -Phase '保留文件移动'

    # ---- 目录移动（逐个，每个之后校验）----
    foreach ($dd in @($Decisions | Where-Object { $_.Kind -eq 'Dir' -and $_.Verdict -ne 'LEAVE' })) {
        $src = Join-Path $script:Root $dd.Name
        if (-not (Test-Path -LiteralPath $src)) { continue }
        if ($dd.Verdict -eq 'TRASH') {
            if (Move-ItemSafe -Path $src -DestDir $trashDirs -FailCount ([ref]$failCount)) { $movedTrash++ }
        } else {
            if (Move-ItemSafe -Path $src -DestDir $buckets['subdirs'] -FailCount ([ref]$failCount)) { $movedArchive++ }
        }
        Test-Integrity -Phase ('目录 {0}' -f $dd.Name)
    }

    # ---- 终检 ----
    Test-Integrity -Phase '终检'
    $after = Get-TopStats -Path $script:Root
    $cargoA = Invoke-CargoCheck -RepoPath $script:RepoA -Label 'ai-bid-research'
    $cargoM = Invoke-CargoCheck -RepoPath $script:RepoM -Label 'mdcore'
    $gitA = Get-GitStatusText -RepoPath $script:RepoA
    $gitM = Get-GitStatusText -RepoPath $script:RepoM
    $integrityLine = ('- git status：ai-bid-research = {0}；mdcore = {1}' -f $(if ($gitA -eq '') { '干净 ✓' } else { '非空（与基线一致）' }), $(if ($gitM -eq '') { '干净 ✓' } else { '非空（与基线一致）' }))
    $integrityLine += ('；{0}；{1}' -f $cargoA, $cargoM)

    # ---- 报告 + CLEANUP.md ----
    $trashRel = ('_work/.trash/{0}' -f $stamp)
    $cleanupPath = New-CleanupMd -Before $before -After $after -Decisions $Decisions `
        -MovedTrash $movedTrash -MovedArchive $movedArchive -MovedKeep $movedKeep -FailCount $failCount `
        -TrashRel $trashRel -IntegrityLine $integrityLine

    Write-Log 'INFO' ('清理后：顶层文件 {0} 个（{1:N1} MB），目录 {2} 个' -f $after.FileCount, ($after.Bytes / 1MB), $after.DirCount)
    Write-Log 'INFO' ('回收 {0} / 归档 {1} / 保留移动 {2} / 失败保留原地 {3}' -f $movedTrash, $movedArchive, $movedKeep, $failCount)
    Write-Log 'INFO' ('回收站暂存: {0}（7 天缓冲；立即清空: -PurgeTrash -Yes）' -f $trashRel)
    Write-Log 'INFO' ('CLEANUP.md: {0}' -f $cleanupPath)

    if ($after.FileCount -lt 100) {
        Write-Host ('==== 验收①达标：顶层文件 {0} < 100 ====' -f $after.FileCount) -ForegroundColor Green
    } else {
        Write-Host ('==== 注意：顶层文件仍为 {0}（≥100），多为移动失败或保护项，见日志 ====' -f $after.FileCount) -ForegroundColor Yellow
    }
    Write-Host '==== 执行完成。复核 _work/.trash/ 后可 -PurgeTrash -Yes 永久清理。 ====' -ForegroundColor Green
    exit 0
}
catch {
    $msg = 'FATAL: {0}' -f $_.Exception.Message
    Write-Host $msg -ForegroundColor Red
    if ($script:LogFile) { Add-Content -LiteralPath $script:LogFile -Value $msg -Encoding UTF8 }
    if ($script:LogFile) { Add-Content -LiteralPath $script:LogFile -Value ('位于: {0}' -f $_.InvocationInfo.PositionMessage) -Encoding UTF8 }
    exit 1
}
