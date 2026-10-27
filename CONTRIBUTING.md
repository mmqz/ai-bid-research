# 贡献指南（含双环境开发准备）

> issue #9 要求：Windows 开发环境准备必须写清，否则新人一上来就卡在 `linker link.exe not found`。

## Linux / WSL（推荐，CI 同口径）

```bash
# Rust（minimal 即可）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable

# 外部解析引擎（poppler 三件 + tesseract）
sudo apt install poppler-utils tesseract-ocr

# OCR 中文语言数据（tessdata_fast，放用户目录——pdf.rs 按 TESSDATA_PREFIX 或 ~/.tessdata 解析）
mkdir -p ~/.tessdata
curl -sL -o ~/.tessdata/chi_sim.traineddata \
  https://github.com/tesseract-ocr/tessdata_fast/raw/main/chi_sim.traineddata

# LibreOffice headless（V1 导出保真验证用）
sudo apt install libreoffice-writer
```

## Windows（原生）

### 1. 先跑 vcvars，再动 cargo（否则 `linker link.exe not found`）

```bat
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
cd /d <repo>\bidcore
cargo check --workspace --all-targets
```

需要 **Visual Studio 2022 Build Tools**（含 C++ 桌面开发工作负载）。

### 2. 外部工具

- poppler：`choco install poppler` 或 [ Releases ] 手动解压并加 PATH；
- tesseract：`choco install tesseract`，`chi_sim.traineddata` 放 `%TESSDATA_PREFIX%` 或 `%USERPROFILE%\.tessdata\`。

**缺件不是错误**：测试框架会打 `SKIP: <test>` + `reason: ENV_MISSING: <工具清单>` 横幅并跳过（issue #9 SKIP 机制），
不会伪装成业务失败。装齐后同一测试自动恢复执行。

### 3. 双环境验证口径

每轮交付附一句：「本轮在 Linux 验证 / Windows 未验证」或反之（issue #9 建议的诚实口径）。
CI 有 Windows 编译检查 job（`windows-check.yml`），但运行时行为仍以本机实测为准。

## 测试入口

```bash
# Rust 侧（bidcore workspace）
cd bidcore && cargo test --workspace

# Python 侧（全链路门禁：T0 环境预检 → T17）
python3 scripts/run_tests.py
```

两侧对「环境缺件」的行为一致：**SKIP（显式登记）而非 FAIL**。
