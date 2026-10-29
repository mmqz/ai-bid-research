# mang-dev 快捷脚本：构建 / 启动 / 自检
# 用法：
#   .\scripts\mang-dev\mang_dev.ps1 build            # 构建 release exe
#   .\scripts\mang-dev\mang_dev.ps1 selftest         # 跑 --selftest
#   .\scripts\mang-dev\mang_dev.ps1 ui               # 打印解析出的 UI 目录
#   .\scripts\mang-dev\mang_dev.ps1 run [工作区]      # 启动（默认当前目录）
#   .\scripts\mang-dev\mang_dev.ps1 smoke            # 窗口内 IPC 冒烟（需 devtools，走 CDP）
param(
    [Parameter(Position = 0)][string]$Cmd = 'build',
    [Parameter(Position = 1)][string]$Workspace = ''
)
$ErrorActionPreference = 'Stop'
$Repo   = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$BidCore = Join-Path $Repo 'bidcore'
$Exe    = Join-Path $BidCore 'target\release\mang-app.exe'

function Stop-MangApp {
    Get-Process mang-app -ErrorAction SilentlyContinue | ForEach-Object {
        Write-Host "[dev] 结束占用中的 mang-app (pid=$($_.Id))"
        $_.Kill()
        $_.WaitForExit(3000) | Out-Null
    }
}

switch ($Cmd) {
    'build' {
        Stop-MangApp
        Push-Location $BidCore
        try { cargo build --release -p mang-app } finally { Pop-Location }
    }
    'selftest' {
        & $Exe --selftest
        exit $LASTEXITCODE
    }
    'ui' {
        & $Exe --ls-ui
    }
    'run' {
        Stop-MangApp
        if ($Workspace -ne '') { & $Exe --workspace $Workspace } else { & $Exe }
    }
    'smoke' {
        node (Join-Path $PSScriptRoot 'mang_ipc_smoke.mjs') @args[1..($args.Count)]
    }
    default {
        Write-Host "用法: mang_dev.ps1 [build|selftest|ui|run|smoke] [工作区]"
        exit 1
    }
}
