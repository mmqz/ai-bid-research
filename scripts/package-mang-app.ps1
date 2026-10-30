# package-mang-app.ps1 — mang-app 绿色版打包（issue #68 任务 B）
#
# 产出：dist-pkg/mang-app-portable-<version>.zip
#   mang-app.exe   release 构建（必须在打包前完成 cargo build --release）
#   ui/            web-ui/dist 的完整内容（含 mang/ 子目录——UI 候选链第一项
#                  <exe>\ui 要求 ui\ 就是 dist 的结构，不是把页面撒在 ui\ 根下）
#
# 用法（PowerShell，仓库根目录）：
#   powershell -ExecutionPolicy Bypass -File scripts\package-mang-app.ps1
# 可选参数：
#   -SkipBuild      跳过 cargo build（已有产物时）
#   -SkipUiBuild    跳过 npm 构建（web-ui\dist 已存在时）
#
# 校验门（缺件即失败，不产半包）：
#   1. exe 存在
#   2. ui\mang\index.html 存在（布局约定）
#   3. 渲染器三件套 docx/xlsx/pdf_view.js 存在
#   4. pdf.js 静态资源 cmaps/ standard_fonts/ wasm/ 存在（PDF 渲染缺一即白屏）

param(
    [switch]$SkipBuild,
    [switch]$SkipUiBuild
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$appDir = Join-Path $repo "bidcore\crates\mang-app"
$webUi = Join-Path $repo "web-ui"
$dist = Join-Path $webUi "dist"
$version = (Select-String -Path (Join-Path $appDir "Cargo.toml") -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches.Groups[1].Value

Write-Host "== mang-app 绿色版打包 v$version =="

# ---- 1. 构建 UI（可跳过） ----
if (-not $SkipUiBuild) {
    if (-not (Test-Path (Join-Path $webUi "node_modules"))) {
        Write-Host "[1/4] npm install（首次约 1-2 分钟）"
        Push-Location $webUi; npm install; Pop-Location
    } else {
        Write-Host "[1/4] node_modules 已存在，跳过 install"
    }
    Write-Host "[1/4] node mang-build.mjs"
    Push-Location $webUi; node mang-build.mjs; Pop-Location
} else {
    Write-Host "[1/4] 跳过 UI 构建（-SkipUiBuild）"
}

# ---- 2. 构建 exe（可跳过） ----
if (-not $SkipBuild) {
    Write-Host "[2/4] cargo build --release（首次 5-15 分钟）"
    cargo build --release --manifest-path (Join-Path $appDir "Cargo.toml")
} else {
    Write-Host "[2/4] 跳过构建（-SkipBuild）"
}

# ---- 3. 校验门 ----
Write-Host "[3/4] 校验产物完整性"
$exe = Join-Path $appDir "target\release\mang-app.exe"
$checks = @(
    @{ Path = $exe;                                  Desc = "mang-app.exe（release 构建）" },
    @{ Path = (Join-Path $dist "mang\index.html");   Desc = "UI 入口页 mang\index.html（布局约定）" },
    @{ Path = (Join-Path $dist "mang\app.js");       Desc = "UI 逻辑 app.js" },
    @{ Path = (Join-Path $dist "mang\styles.css");   Desc = "样式 styles.css" },
    @{ Path = (Join-Path $dist "mang\docx_view.js"); Desc = "docx 渲染器" },
    @{ Path = (Join-Path $dist "mang\xlsx_view.js"); Desc = "xlsx 渲染器" },
    @{ Path = (Join-Path $dist "mang\pdf_view.js");  Desc = "pdf 渲染器" },
    @{ Path = (Join-Path $dist "mang\pdf.worker.min.mjs"); Desc = "pdf.js worker" },
    @{ Path = (Join-Path $dist "mang\cmaps");        Desc = "pdf.js cmaps（中文包）" },
    @{ Path = (Join-Path $dist "mang\standard_fonts"); Desc = "pdf.js 标准字体" },
    @{ Path = (Join-Path $dist "mang\wasm");         Desc = "pdf.js wasm 解码器" }
)
$failed = $false
foreach ($c in $checks) {
    if (Test-Path $c.Path) {
        $size = if ((Get-Item $c.Path) -is [System.IO.FileInfo]) {
            "{0:N1} KB" -f ((Get-Item $c.Path).Length / 1KB)
        } else { "目录" }
        Write-Host ("  OK  {0}  ({1})" -f $c.Desc, $size)
    } else {
        Write-Host ("  FAIL {0}  -> {1}" -f $c.Desc, $c.Path) -ForegroundColor Red
        $failed = $true
    }
}
if ($failed) {
    Write-Host "`n校验失败：缺件不打包。UI 缺件请去掉 -SkipUiBuild 重跑；exe 缺件请去掉 -SkipBuild。" -ForegroundColor Red
    exit 1
}

# ---- 4. 组装 + 压缩 ----
Write-Host "[4/4] 组装 zip"
$stage = Join-Path $repo "dist-pkg\stage"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $stage "ui") | Out-Null
Copy-Item $exe (Join-Path $stage "mang-app.exe")
# dist 整体（含 mang\ 子目录与顶层 index.host.html/host.js）→ ui\
Copy-Item (Join-Path $dist "*") (Join-Path $stage "ui") -Recurse

$outDir = Join-Path $repo "dist-pkg"
$zip = Join-Path $outDir "mang-app-portable-$version.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip

$mb = "{0:N1}" -f ((Get-Item $zip).Length / 1MB)
Write-Host "`n产出：$zip  ($mb MB)"
Write-Host "布局："
Write-Host "  mang-app-portable-$version\"
Write-Host "  ├── mang-app.exe"
Write-Host "  └── ui\mang\index.html  （UI 候选链第一项 <exe>\ui 命中）"
Write-Host "`n验证（解压后双击 exe，或命令行）："
Write-Host "  .\mang-app.exe --ls-ui        # 应打印 ...\ui"
Write-Host "  .\mang-app.exe --selftest     # 退出码 0"
Write-Host "  .\mang-app.exe <目录>         # 打开工作区"

Remove-Item $stage -Recurse -Force
