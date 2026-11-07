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
#
# #248 发行段增强（-Release 或 MANG_RELEASE=1 时全部生效；开发打包缺省不变）：
#   5. UI 发行模式（minify + 无 sourcemap，mang-build.mjs #248 面）
#   6. 版本一致性：mang-app Cargo.toml ↔ web-ui package.json（不一致 fail-closed）
#   7. 开发产物零容忍：zip 内出现 .map/.pdb/.ts 源/node_modules/.git → 拒发
#   8. SHA-256 清单：SHA256SUMS.txt 入包 + 独立 artifact；逐文件可独立复核
#   9. 构建元数据：build-manifest.json（版本/commit/工具链/逐文件哈希）

param(
    [switch]$SkipBuild,
    [switch]$SkipUiBuild,
    # #248 发行模式：UI minify + 全部发行校验门。环境变量 MANG_RELEASE=1 同效
    # （CI 里用环境变量传档位，本地手动用开关——两者任一即发行档）。
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$appDir = Join-Path $repo "bidcore\crates\mang-app"
$webUi = Join-Path $repo "web-ui"
$dist = Join-Path $webUi "dist"
$version = (Select-String -Path (Join-Path $appDir "Cargo.toml") -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches.Groups[1].Value

# #248 发行档判定：-Release 开关或 MANG_RELEASE=1（CI 传环境变量）
$isRelease = ($Release -or $env:MANG_RELEASE -eq "1")
$mode = if ($isRelease) { "发行（minify + 发行校验门）" } else { "开发（可调试，无发行校验门）" }
Write-Host "== mang-app 绿色版打包 v$version  [$mode] =="

# ---- 1. 构建 UI（可跳过；发行档传 MANG_RELEASE=1 → minify） ----
if (-not $SkipUiBuild) {
    if (-not (Test-Path (Join-Path $webUi "node_modules"))) {
        Write-Host "[1/4] npm install（首次约 1-2 分钟）"
        Push-Location $webUi; npm install; Pop-Location
    } else {
        Write-Host "[1/4] node_modules 已存在，跳过 install"
    }
    if ($isRelease) { $env:MANG_RELEASE = "1" }
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

# ======================================================================
# #248 发行段（仅 -Release / MANG_RELEASE=1 时执行——开发打包到此为止）
# ======================================================================
if (-not $isRelease) {
    Write-Host "`n开发档打包完成（发行校验门未启用——加 -Release 或 MANG_RELEASE=1 启用）"
    exit 0
}

# ---- 5. 版本一致性（fail-closed）：mang-app Cargo.toml ↔ web-ui package.json ----
Write-Host "`n[发行门 5] 版本一致性"
$pkgJson = Get-Content (Join-Path $webUi "package.json") -Raw | ConvertFrom-Json
$uiVersion = $pkgJson.version
if ($version -ne $uiVersion) {
    Write-Host "  FAIL mang-app Cargo.toml v$version ≠ web-ui package.json v$uiVersion" -ForegroundColor Red
    Write-Host "       两面版本号必须一致（发行包一个版本号一个包——先对齐再打包）" -ForegroundColor Red
    exit 1
}
Write-Host "  OK   v$version 两面一致（Cargo.toml ↔ package.json）"

# ---- 6. 开发产物零容忍（fail-closed）：zip 逐条目扫描 ----
Write-Host "[发行门 6] 开发产物零容忍（.map/.pdb/.ts 源/node_modules/.git）"
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($zip)
$entryNames = @($archive.Entries | ForEach-Object { $_.FullName })
$archive.Dispose()
$forbidden = @()
foreach ($n in $entryNames) {
    $n = $n -replace '/', '\'
    if ($n -match '\.(map|pdb|ts)$' -or $n -match '\\node_modules\\' -or $n -match '\\\.git(\\|$)') {
        $forbidden += $n
    }
}
if ($forbidden.Count -gt 0) {
    Write-Host "  FAIL 发行包含 $($forbidden.Count) 个开发产物：" -ForegroundColor Red
    $forbidden | Select-Object -First 10 | ForEach-Object { Write-Host "       $_" -ForegroundColor Red }
    exit 1
}
Write-Host "  OK   零违禁项（全量 $($entryNames.Count) 条目扫描）"

# ---- 7. SHA-256 清单（入包 + 独立 artifact 双面） ----
Write-Host "[发行门 7] SHA-256 清单"
# 解包到临时目录逐文件哈希（exe 在 zip 里，从 zip 流读不可靠地对齐 stage 布局）
$verifyDir = Join-Path $repo "dist-pkg\verify-$version"
if (Test-Path $verifyDir) { Remove-Item $verifyDir -Recurse -Force }
Expand-Archive -Path $zip -DestinationPath $verifyDir
$sums = @()
Get-ChildItem $verifyDir -Recurse -File | Sort-Object FullName | ForEach-Object {
    $h = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    $rel = $_.FullName.Substring($verifyDir.Length + 1)
    $sums += "$h  $rel"
}
$sumsFile = Join-Path $verifyDir "SHA256SUMS.txt"
$sums | Set-Content $sumsFile -Encoding ascii
# 清单自身入包（重打 zip：stage 目录已删，用 verify 目录重组——布局相同）
$zipFinal = Join-Path $outDir "mang-app-portable-$version.zip"
if (Test-Path $zipFinal) { Remove-Item $zipFinal -Force }
Compress-Archive -Path (Join-Path $verifyDir "*") -DestinationPath $zipFinal
# 独立 artifact 面（包外同目录，workflow 直接上传）
$sumsArtifact = Join-Path $outDir "SHA256SUMS-$version.txt"
$sums | Set-Content $sumsArtifact -Encoding ascii
Remove-Item $verifyDir -Recurse -Force
Write-Host "  OK   SHA256SUMS.txt 入包（$($sums.Count) 文件）+ 独立清单 $sumsArtifact"

# ---- 8. 构建元数据（build-manifest.json——审计面） ----
Write-Host "[发行门 8] 构建元数据"
$commit = git -C $repo rev-parse HEAD 2>$null
if (-not $commit -or $LASTEXITCODE -ne 0) { $commit = "unknown" }
$toolchain = (rustc --version 2>$null); if (-not $toolchain) { $toolchain = "unknown" }
$nodeVer = (node --version 2>$null); if (-not $nodeVer) { $nodeVer = "unknown" }
$manifest = [ordered]@{
    product    = "mang-app"
    version    = $version
    mode       = "release"
    commit     = $commit
    rust       = $toolchain
    node       = $nodeVer
    zip        = "mang-app-portable-$version.zip"
    zip_sha256 = (Get-FileHash $zipFinal -Algorithm SHA256).Hash.ToLower()
    built_at   = ([DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ"))  # PS 5.1 兼容（无 -AsUTC）
    files      = $sums.Count
    signed     = $false   # 签名状态显式可见（workflow 的签名步骤负责翻转）
}
$manifestPath = Join-Path $outDir "build-manifest-$version.json"
$manifest | ConvertTo-Json | Set-Content $manifestPath -Encoding utf8
Write-Host "  OK   $manifestPath"
Write-Host "       签名状态：未签名（signed=false——Authenticode 由发行工作流可选步骤负责）"

Write-Host "`n发行打包完成："
$mbFinal = "{0:N1}" -f ((Get-Item $zipFinal).Length / 1MB)
Write-Host "  $zipFinal  ($mbFinal MB)"
Write-Host "  $sumsArtifact"
Write-Host "  $manifestPath"
