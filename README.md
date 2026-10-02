# PPTX 智能打开器（Rust 版）

根据 PPTX 文件的创建软件，自动选择 PowerPoint 或 WPS 打开，避免排版错乱。

> 本项目是 [`pptx_selector_next`](https://github.com/yangxuqin/pptx_selector_next)（C++/Win32）的 **Rust 重构版**：
> 在完全保持功能与使用行为的前提下，改为**流式读取 ZIP**，实现快速启动、低内存占用、小体积。
> C++ 版源码仍作为行为基准保留在 `pptx_selector_next`，未被改动。

## 功能特点

- 自动检测 PPTX 文件是由 Microsoft PowerPoint 还是 WPS 创建
- 智能选择对应软件打开，确保排版正确
- 通过 `config.ini` 配置路径，无需重新编译
- 零控制台窗口，静默运行（Windows GUI 子系统）
- 检测软件不可用时自动降级到另一软件，最终走兜底策略
- **流式读取**：只解析 ZIP 中央目录并读取 `docProps/app.xml`，**从不把整个 PPTX 读入内存**
- 完整的日志记录，便于问题排查

## 使用场景

适用于教室电脑等多人共享环境：不同老师使用不同软件制作 PPT，用对应软件打开才能避免排版错乱。将 `.pptx` 关联到本程序，双击即可自动选择正确的软件打开。

## 工作原理

1. 打开 PPTX（**不读取内容**，只取文件句柄与大小）
2. 用 `rawzip` 解析 ZIP 中央目录，按顺序线性定位 `docProps/app.xml`
3. 只读取该条目；STORE 直接使用，DEFLATE 用 `miniz_oxide` 流式解压，并校验 CRC32 与未压缩大小
4. 用 `quick-xml` 事件流解析，提取 `<Application>` 标签的**原始文本**（不做实体解码）
5. 根据应用名称判断创建软件：
   - 包含 `WPS` 或 `Kingsoft` → 使用 WPS 打开
   - 包含 `Microsoft` 或 `PowerPoint` → 使用 PowerPoint 打开
   - 其他情况 → 按 `Fallback` 配置兜底
6. 若检测到的软件未安装，自动降级到另一软件；都不可用则按兜底配置启动

> 与原 C++ 版的唯一架构性差异就是第 2–4 步的流式化。决策、日志、退出码、配置格式均逐字符/
  逐数值对齐，详见 [`docs/behavior-mapping.md`](docs/behavior-mapping.md)。

## 目录结构

```
pptx_selector_rust/
├── README.md              本文档
├── AGENTS.md              AI 助手协作指引
├── Cargo.toml/.lock       依赖与构建配置
├── LICENSE                MIT 许可证
├── build.rs               嵌入 Windows 图标
├── config.ini             运行配置（路径、日志、兜底策略）
├── .cargo/config.toml     交叉编译 linker / ar
├── .github/workflows/     GitHub Actions：自动构建 Windows Release
├── src/                   项目源码
│   ├── main.rs            程序入口与主流程编排
│   ├── error.rs           退出码与错误类型
│   ├── config.rs          配置加载（读 config.ini）
│   ├── logger.rs          日志系统
│   ├── detector.rs        PPTX 创建者检测（流式 ZIP + XML）
│   └── launcher.rs        外部程序启动（CreateProcessW）
├── docs/
│   ├── behavior-mapping.md  逐函数行为映射（对照 C++ 基准）
│   └── refactor-report.md   重构最终报告（含内存实测数据）
├── examples/mock_launcher.rs  Wine 验证用替身
├── scripts/               测试 fixture 生成、验收、内存测量、Wine 验证、PE 产物校验
├── resources/icon.ico     应用图标
└── tests/fixtures/        测试样本
```

## 编译

### 环境要求

- Rust 1.85+（实测 1.98.1）
- 交叉编译 Windows EXE 需要：MinGW-w64（`x86_64-w64-mingw32-gcc` / `windres`）与
  `rustup target add x86_64-pc-windows-gnu`

### 构建命令

```bash
# Linux 原生构建验证
cargo build --release

# Windows x64 EXE（交叉编译，保留图标与无控制台窗口）
cargo build --release --target x86_64-pc-windows-gnu
```

产物：

| 目标 | 路径 |
|---|---|
| Linux 原生 | `target/release/pptx_selector` |
| Windows x64 | `target/x86_64-pc-windows-gnu/release/pptx_selector.exe` |

### 构建参数

```toml
[profile.release]
opt-level = "z"      # 体积优先
lto = true           # 链接时优化
codegen-units = 1
strip = true         # 剥离符号
panic = "abort"      # 无 unwind 运行时
```

`build.rs` 在目标平台为 Windows 时用 `winresource` 嵌入 `resources/icon.ico`。

## 使用方法

### 下载预编译版本

不想自行编译时，直接从 [Releases](https://github.com/yangxuqin/pptx_selector_rust/releases)
下载 `pptx_selector-<版本>-windows-x64.zip`，解压后把 `pptx_selector.exe` 与 `config.ini`
放在同一目录即可使用（也可单独下载裸 `pptx_selector.exe`）。

该压缩包由 GitHub Actions 自动构建：推送 `v*` 标签（例如 `v1.0.0`）会触发 Linux + mingw-w64
交叉编译、校验 PE 产物并自动创建 Release；也可在 Actions 页面手动触发（默认只产出 Artifact，
填写 `tag` 输入框则同时创建 Release）。详见
[`.github/workflows/release.yml`](.github/workflows/release.yml)。

### 命令行调用

```bat
pptx_selector.exe "path\to\your\presentation.pptx"
```

### 文件关联（推荐）

将 `.pptx` 文件默认打开方式设置为本程序，即可双击自动选择软件打开。

## 配置

将 `config.ini` 放在 `pptx_selector.exe` 同目录下，程序启动时自动读取。修改后无需重新编译，重启程序生效。

```ini
[Paths]
PowerPoint=C:\Program Files\Microsoft Office\root\Office16\POWERPNT.EXE
WPS=C:\Program Files\Kingsoft\WPS Office\12.1.0.23542\office6\wpp.exe
LogFile=D:\logs\pptx_selector.log

[Options]
EnableLog=1
Fallback=0
```

| 配置项 | 说明 |
|--------|------|
| `PowerPoint` | PowerPoint 可执行文件路径 |
| `WPS` | WPS 演示可执行文件路径 |
| `LogFile` | 日志文件路径（自动创建目录） |
| `EnableLog` | 是否启用日志：`1`=启用，`0`=禁用 |
| `Fallback` | 兜底程序：`0`=WPS，`1`=PowerPoint |

> 无 `config.ini` 时程序使用内置默认值运行，但路径可能不匹配实际安装位置，建议始终配置。
> 键名/节名大小写不敏感，缺失项保留默认值，取值两侧空白自动裁剪——与 C++ 版一致。

## 退出码

| 值 | 含义 |
|---|---|
| 0 | 成功启动 |
| 1 | 命令行参数不足 |
| 2 | 目标 PPTX 文件不存在 |
| 3 | 文件读取失败（打开失败 / 0 字节 / 是目录） |
| 4 | ZIP 解析失败（**从不返回**：解析失败会降级为 Unknown 并走兜底） |
| 5 | 首选与兜底都启动失败 |
| 6 | 未找到可用启动器（**从不返回**） |

## 日志

- 位置与开关由 `config.ini` 的 `LogFile` / `EnableLog` 决定
- 编码 UTF-8（首次创建时写 BOM），行尾 CRLF
- 每行格式：`[YYYY-MM-DD HH:MM:SS] 正文`
- 每条日志独立追加写入；写入失败静默丢弃，绝不弹窗、绝不写控制台

## 测试与验证

```bash
# 1. 生成测试 fixture（必需）
python3 scripts/make_fixtures.py

# 2. 单元测试
cargo test

# 3. 一键验收
bash scripts/verify.sh

# 4. 含 >500MB 内存测试的完整验收
python3 scripts/make_fixtures.py --big     # 生成 545 MB fixture（约 545MB 磁盘）
bash scripts/verify.sh --with-big
```

单独运行各项：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
bash scripts/measure_rss.sh     # 端到端峰值 RSS 测量
bash scripts/wine_checks.sh     # Wine 集成验证（CreateProcessW / 图标 / Unicode 路径 / 降级兜底）
python3 scripts/check_pe.py target/x86_64-pc-windows-gnu/release/pptx_selector.exe  # PE 结构校验
```

> CI：推送 `v*` 标签时 [GitHub Actions](.github/workflows/release.yml) 会执行
> `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test`，再交叉编译 Windows x64
> release，并用 `scripts/check_pe.py` 校验产物后发布 Release。

## 技术栈

- **Rust**：核心逻辑实现
- **rawzip**：ZIP 中央目录解析与流式 entry 读取（纯容器库，**不含解压**）
- **miniz_oxide**：DEFLATE 解压（原项目 miniz 的官方 Rust 移植）
- **quick-xml**：流式 XML 事件解析，仅提取 `<Application>`
- **windows-sys**：`CreateFileW` / `CreateProcessW` / `GetPrivateProfileStringW` 等 Win32 API
- **winresource**：构建期嵌入图标

无 Tokio / async runtime，无 GUI framework，无后台进程。

## 性能与内存

实测（Arch Linux，545 MB PPTX，见 [`docs/refactor-report.md`](docs/refactor-report.md)）：

| 指标 | 数值 |
|---|---|
| 进程内探测期间 RSS 峰值增量（release） | **140 kB** |
| 进程内探测期间 RSS 峰值增量（dev） | **368 kB** |
| 端到端进程峰值 RSS（小文件基线） | 12 096 kB |
| 端到端进程峰值 RSS（545 MB 文件） | 12 080 kB |
| 验收门槛 | 增量 < 20 MB ✅ |

> 测试文件为 545 260 867 字节（1 个 STORE 大条目 + 正常 DEFLATE 的 `docProps/app.xml`，
> ZIP 结构完全合法）。探测期间 ZIP 层缓冲区固定 64 KiB，与文件大小无关。

## 故障排查

### 程序无法打开 PPTX

1. 查看日志文件（默认 `D:\logs\pptx_selector.log`）
2. 确认 `config.ini` 中 PowerPoint 和 WPS 的路径是否正确
3. 检查 PPTX 文件是否损坏（非有效 ZIP）

### 日志无法创建

1. 检查日志路径所在目录是否有写入权限
2. 修改 `config.ini` 中的 `LogFile` 到有权限的位置
3. 设置 `EnableLog=0` 可禁用日志

### 编译失败

1. 确认 `cargo --version` ≥ 1.85
2. 交叉编译：`rustup target add x86_64-pc-windows-gnu`，并确认
   `x86_64-w64-mingw32-gcc --version` 可用
3. 从项目根目录执行命令（脚本使用相对路径）

## 注意事项

1. 确保 PowerPoint 和 WPS 的安装路径与 `config.ini` 配置一致
2. 确保日志路径所在目录有写入权限
3. 程序仅在 Windows 系统运行（Linux 可构建、可跑单元测试，但无法启动外部程序）
4. 文件读取、配置、日志和启动均使用 Unicode Win32 API，避免中文路径经过系统 ANSI 代码页后损坏
5. 日志以 UTF-8（含 BOM）写入，便于记事本等工具正确识别中文
6. **Windows 专属行为仅在 Wine 下做过集成验证，尚未在真实 Windows 上验证**
   （详见 [`docs/refactor-report.md`](docs/refactor-report.md)）

## 许可证

[MIT](LICENSE) © 2026 杨绪钦 (yangxuqin)

Release 压缩包内随附 `LICENSE`，二次分发时请保留版权与许可声明。
