# AGENTS.md

本文件为 AI 编程助手提供本项目的协作指引，帮助助手快速理解项目结构、约定与工作流。

> 本项目是 `pptx_selector_next`（C++/Win32）的 **Rust 重构版**。
> **`../pptx_selector_next` 是行为基准**：它是只读的参照物，禁止修改、重构、
> 格式化或"顺便优化"。任何行为对齐的疑问都以它的源码为准。

## 项目概述

**PPTX 智能打开器**（`pptx_selector.exe`）：一个 Windows 原生小工具，根据 PPTX 文件的创建软件（Microsoft PowerPoint 或 WPS）自动选择对应软件打开，避免排版错乱。

- **目标用户**：教室电脑等多人共享环境
- **核心机制**：PPTX 是 ZIP 包，读取 `docProps/app.xml` 中的 `<Application>` 标签判断创建者
- **运行方式**：作为 `.pptx` 的默认打开程序，双击触发
- **与 C++ 版的关键区别**：**流式读取**——绝不把整个 PPTX 读入内存，只解析 ZIP 中央目录定位并读取 `docProps/app.xml`

## 技术栈

- 语言：Rust（edition 2021，`rust-version = "1.85"`；实测工具链 1.98.1）
- 构建：Cargo；交叉编译目标 `x86_64-pc-windows-gnu`（MinGW-w64）+ `winresource` 嵌入图标
- 依赖白名单（`Cargo.toml`）：

| Crate | 用途 | 说明 |
|---|---|---|
| `rawzip` | ZIP 流式读取 | 只解析中央目录、定位 entry、提供原始字节流；**本身不含任何解压能力** |
| `miniz_oxide` | DEFLATE 解压 | 原项目 miniz 的官方 Rust 移植；唯一传递依赖 `adler2`。**说明书白名单的增补项**，原因见 `docs/behavior-mapping.md` D2 |
| `quick-xml` | 流式 XML 事件解析 | `default-features = false`；不走 serde |
| `windows-sys` | Win32 API（仅 Windows target） | feature 按实际调用配置 |
| `winresource` | build-dependency，嵌入图标 | |

- **禁止引入**：`tokio`、`async-std`、`reqwest`、`anyhow`、`thiserror`、`serde`、`log`、`env_logger`、`clap`、`eframe`、`egui`，以及任何需要 Tokio / async runtime 的 ZIP 方案。

## 目录结构

```
pptx_selector_rust/
├── README.md                 面向用户与开发者的项目文档
├── AGENTS.md                 本文件
├── Cargo.toml / Cargo.lock   依赖锁定
├── build.rs                  嵌入 Windows 图标（用 CARGO_CFG_TARGET_OS 判断 target）
├── config.ini                运行配置模板（与原项目逐字节一致）
├── .cargo/config.toml        windows-gnu 的 linker / ar
├── .github/workflows/release.yml  打 v* 标签自动构建 Windows Release
├── .gitignore                忽略 /target、*.log、超大 fixture
├── src/                      业务源码
│   ├── main.rs               入口、windows_subsystem、参数解析、主流程编排 + 测试用 TempDir
│   ├── error.rs              ErrorCode（退出码）与 DetectError
│   ├── config.rs             Config + load_config + get_exe_dir
│   ├── logger.rs             Logger（文件日志，时间戳 + BOM + CRLF）
│   ├── detector.rs           detect_pptx_creator_from_file + AppCreator + 流式 ZIP/XML
│   └── launcher.rs           file_attributes + app_exists + quote_command_line_arg + launch_app
├── resources/icon.ico        应用图标（复制自原项目，未联网 clone）
├── docs/
│   ├── behavior-mapping.md   第一阶段产出：逐函数行为映射 + 差异/存疑清单
│   └── refactor-report.md    第七阶段产出：最终报告（含内存原始数据）
├── examples/
│   └── mock_launcher.rs      Wine 验证用替身 EXE（记录 argv，不进正式二进制）
├── scripts/
│   ├── make_fixtures.py      生成测试 fixture（含 >500MB）
│   ├── verify.sh             一键跑完验收清单
│   ├── check_pe.py           校验交叉编译产物为 x86-64 GUI PE 且含图标资源
│   ├── measure_rss.sh        端到端内存测量
│   └── wine_checks.sh        Wine 集成验证
└── tests/fixtures/           小型 fixture 入库；big_*.pptx 按需生成、已 gitignore
```

> **不设 `lib.rs`**：决定采用"只有二进制 target"的结构，因此所有测试都是模块内
> `#[cfg(test)] mod tests`（单元测试可访问私有项）。新增模块请沿用该约定。

## 架构与模块职责

| 模块 | 职责 | 关键符号 |
|------|------|----------|
| `error` | 退出码与错误类型 | `ErrorCode`（`Ok=0` … `NoLauncher=6`）、`DetectError` |
| `config` | 读取 `config.ini`、定位 exe 目录 | `Config`, `load_config()`, `get_exe_dir()` |
| `logger` | 文件日志，自动时间戳 | `Logger::new/init_dir/write`, `format_prefix()` |
| `detector` | 流式定位 ZIP entry、解压、解析 `<Application>` 并分类 | `AppCreator`, `DetectionResult`, `detect_pptx_creator_from_file()` |
| `launcher` | 检查 exe 存在性、`CreateProcessW` 启动 | `file_attributes()`, `app_exists()`, `quote_command_line_arg()`, `launch_app()` |
| `main` | 流程编排与降级/兜底决策 | `main()`, `run()`, `choose_launcher()`, `launch_fallback()` |

### 主流程（`main.rs` 的 `run()`，对齐 C++ `WinMain`）

1. `get_exe_dir()` + `load_config()` → 得到 `Config`
2. 按 `Config` 构造 `Logger`，`init_dir()` 建目录
3. `CommandLineToArgvW` 取 PPTX 路径，全程保留 Unicode
4. `app_exists()` 检查 PowerPoint / WPS 是否可用
5. `file_attributes()` 检查目标文件是否存在
6. `open_target_file()`（`CreateFileW`）打开文件并取大小（**不读取内容**）
7. `detect_pptx_creator_from_file()` **流式**识别创建软件
8. `choose_launcher()` 决策；失败走 `launch_fallback()`

### 决策与降级逻辑

- 检测为 WPS：优先 WPS，WPS 不存在则降级 PowerPoint
- 检测为 Microsoft：优先 PowerPoint，不存在则降级 WPS
- 未知 / 无可用启动器 / 启动失败：按 `Config.fallback` 兜底（0=WPS, 1=PowerPoint）

> **重要**：本程序设计为"尽量打开文件"，多数失败路径会兜底启动而非直接报错退出。修改时请保留此行为。

### 退出码（与 C++ `enum class Err` 数值一一对应）

| 值 | 名称 | 何时返回 |
|---|---|---|
| 0 | `Ok` | 进程创建成功 |
| 1 | `InvalidArgs` | 命令行参数不足 |
| 2 | `FileNotFound` | 目标文件不存在 |
| 3 | `ReadFailed` | 打开失败 / 0 字节 / 目录 |
| 4 | `InvalidArchive` | **从不返回**（保留枚举完整性） |
| 5 | `LaunchFailed` | 首选与兜底都启动失败 |
| 6 | `NoLauncher` | **从不返回**（保留枚举完整性） |

## 编码约定

- **平台隔离**：Windows 专属代码用 `#[cfg(windows)]`；非 Windows 分支是**测试 shim**
  （INI 解析、UTC 时间戳、`fs::metadata`、`launch_app` 返回 `Err`），必须在注释里写明
  "仅用于 Linux 测试与构建"。**Windows 行为以真实 Win32 API 为准。**
- **禁止 `.unwrap()` / `.expect()`**：`src/` 的探测、配置、日志、启动路径上所有 I/O、
  ZIP、XML 错误都必须经 `Result` 传播；用 `unwrap_or_default()` / `map_or()` /
  `let ... else` 处理。`build.rs` 与 `#[cfg(test)]` 模块不受此限。
  `scripts/verify.sh` 会做机检门禁。
- **错误处理**：错误码用 `ErrorCode`；ZIP/XML 失败折叠为 `Unknown` 后继续走原逻辑，
  **不额外写日志**（对齐 C++ 的静默降级）。
- **字符串**：路径统一用 `PathBuf` / `OsString`，禁止先转成系统 ANSI 代码页；
  日志正文一律 UTF-8。Win32 调用前用 `encode_wide()` 转 UTF-16 并补 NUL。
- **`CreateProcessW` 而非 `std::process::Command`**：需要精确控制
  `STARTUPINFOW`、`CREATE_NEW_PROCESS_GROUP`、命令行拼接与窗口参数。
- **注释**：文件头用块注释说明模块用途与对应的 C++ 文件；复杂逻辑用行注释；
  公开 API 用 `///`。**每一处与 C++ 的有意差异都要在代码注释里指出**（并同步
  `docs/behavior-mapping.md` 的 D 编号）。
- **内存纪律**：ZIP 层缓冲区固定 64 KiB；禁止 `read()` 整个 PPTX；只读取
  `docProps/app.xml`；app.xml 读取/解压上限 4 MiB（`MAX_APP_XML_BYTES`）。
- **依赖纪律**：新增任何 crate 前先确认白名单；说明书禁止的 crate 一律不得引入。

## 构建

从项目根目录执行：

```bash
# Linux 原生（构建验证 / 单元测试）
cargo build --release

# Windows x64 EXE（交叉编译，保留图标与无控制台窗口）
cargo build --release --target x86_64-pc-windows-gnu
```

关键点：
- `build.rs` 在 **host** 上执行，必须用 `CARGO_CFG_TARGET_OS == "windows"` 判断目标平台，
  不能用 `#[cfg(target_os = "windows")]`
- `.cargo/config.toml` 指定 `x86_64-w64-mingw32-gcc` / `-ar`
- `[profile.release]`：`opt-level="z"`、`lto=true`、`codegen-units=1`、`strip=true`、`panic="abort"`
- `windows-sys` 不要锁死 feature 列表；缺 feature 会编译报错，按报错逐项补齐
  （注意 `CreateProcessW` / `SHCreateDirectoryExW` 的签名引用 `SECURITY_ATTRIBUTES`，
  需要同时启用 `Win32_Security`）
- **CI**：`.github/workflows/release.yml` 在推送 `v*` 标签时于 ubuntu-latest 上用
  mingw-w64 交叉编译 Windows x64 release，先过 `fmt` / `clippy -D warnings` / `test`
  门禁，再用 `scripts/check_pe.py` 校验产物，最后自动创建 GitHub Release；
  也可在 Actions 页面手动触发（`workflow_dispatch`）

## 测试与验证

```bash
# 1. 生成 fixture（必需，小文件）
python3 scripts/make_fixtures.py

# 2. 单元测试
cargo test

# 3. 一键验收（格式化、clippy、测试、双目标构建、内存、Wine、原项目零改动）
bash scripts/verify.sh                 # 默认跳过大文件内存测试
python3 scripts/make_fixtures.py --big  # 先生成 545MB fixture
bash scripts/verify.sh --with-big
```

单独运行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
bash scripts/measure_rss.sh     # 端到端峰值 RSS
bash scripts/wine_checks.sh     # Wine 集成验证
```

**已知构建约束**：`panic = "abort"` 只作用于 release profile，`cargo test`（dev）不受影响。
若需要在 release 模式下跑进程内内存测试，用：

```bash
cargo test --release --config 'profile.release.panic="unwind"' -- --ignored --nocapture memory_peak
```

## 常见陷阱

1. **`rawzip` 不含解压能力**：它是纯容器库，必须自己接 `miniz_oxide` 才能读 DEFLATE
   条目。官方示例用的是 `flate2`，本项目用 `miniz_oxide`（白名单增补项）。
2. **`rawzip::CompressionMethod` 不是 enum**：它是 `CompressionMethod(u16)` newtype +
   关联常量，必须用 `==` 比较，不能 `match` 它的"变体"。
3. **`claim_verifier()` 在 `ZipReader` 上，不在 `ZipEntry` 上**：先 `entry.reader()`，
   再取校验信息，最后 `take(..).read_to_end(..)`。
4. **quick-xml 0.42 是 str 基的**：`local_name().as_ref()` 得到 `&str`（不是 `&[u8]`）。
5. **quick-xml 会把实体/字符引用拆成 `Event::GeneralRef`**：它保存的是不含 `&`/`;` 的
   内部文本。要还原文档原文（对齐 C++ 裸字节截取）必须自己补回定界符；
   CDATA 同理要补回 `<![CDATA[` / `]]>`。
6. **`build.rs` 的 `#[cfg(target_os = ...)]` 陷阱**：见上文"构建"。
7. **日志每条独立 open/append/close**：这是刻意保持的 C++ 行为，不要改成持有句柄或缓冲。
8. **日志首行 BOM**：仅当文件长度为 0 时写入；测试断言 BOM 时注意偏移量 3。
9. **时间戳**：Windows 用 `GetLocalTime`；非 Windows shim 用 **UTC**，因此 Linux 上跑出来的
   小时数与本地时间不同，属预期行为（格式才是被测对象）。
10. **`GetTickCount` 回绕**：用 `wrapping_sub`，不要用 `saturating_sub`。
11. **非 Windows 上 `launch_app` 恒返回 `Err(120)`**：这是有意的，Linux 下主流程最终
    退出码为 5（LaunchFailed），端到端测试需按此预期断言。
12. **Wine 替身是异步进程**：`CreateProcessW` 不等待子进程，脚本必须轮询替身写出的
    文件（`scripts/wine_checks.sh` 里已有 `wineserver -w` + 轮询）。
13. **原项目只读**：`../pptx_selector_next` 的 `git status --porcelain` 必须始终为空。

## 修改指引

### 新增配置项

1. 在 `Config`（`src/config.rs`）加字段并在 `Default` 设默认值
2. 在 `load_config()` 中读取（Windows 走 `GetPrivateProfileStringW` / `GetPrivateProfileIntW`；
   同步更新非 Windows shim）
3. 更新 `config.ini` 注释、`README.md` 配置表、以及 `docs/behavior-mapping.md` §2.2

### 新增检测规则

- 修改 `src/detector.rs` 的 `classify_app()`，扩展 `AppCreator` 与匹配顺序
  （注意：WPS/Kingsoft 必须优先于 Microsoft/PowerPoint，且匹配大小写敏感）
- 若需新增启动器类型，同步更新 `choose_launcher()`（`src/main.rs`）

### 调整降级/兜底策略

- 集中在 `src/main.rs` 的 `choose_launcher()` 与 `launch_fallback()`
- 修改时务必保持"尽量打开文件"的设计意图

## 当前未解决问题

- **未在真实 Windows 上验证**：本机只有 Arch Linux + Wine，Windows 专属行为
  （`CreateProcessW`、无控制台窗口、图标、Unicode 参数传递）仅通过 Wine 集成验证与
  PE 结构检查确认。真实 Windows 上的差异未知。
- **无法运行 C++ 原版做二进制级对比**：所有行为对齐基于源码审读
  （见 `docs/behavior-mapping.md` 的存疑清单 U1–U6）。
- 原 C++ 项目本身**没有自动化测试**，因此不存在"仓库已有测试样本"，
  全部 fixture 都是按源码语义新构造的。
