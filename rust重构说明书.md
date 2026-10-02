# pptx_selector_next —— C++ → Rust 重构任务

> 本文件包含两部分：
> - **第一部分**：开发者（你）在 Arch Linux 上自行执行的环境准备
> - **第二部分**：交给实施 Agent 的重构 prompt
>
> 使用方式：先完成第一部分，验证工具链就绪后，把第二部分整段交给实施方。

---

# 第一部分：环境准备（开发者自行执行，不进 prompt）

在 Arch Linux 上由开发者本人执行。这些**不属于**重构 prompt，不要交给实施方。

## 1. 安装系统级依赖

需要自行用包管理器安装（按个人习惯选择命令）：

- **`mingw-w64-gcc`**（或发行版等价的 MinGW-w64 交叉编译工具链）
- **`wine`**（可选，但推荐，用于 Windows 行为验证）

## 2. 添加 Rust 交叉编译目标

```bash
rustup target add x86_64-pc-windows-gnu
```

## 3. 配置链接器

在项目 `.cargo/config.toml`（或 `~/.cargo/config.toml`）中写入：

```toml
[target.x86_64-pc-windows-gnu]
linker = "x86_64-w64-mingw32-gcc"
ar = "x86_64-w64-mingw32-ar"
```

## 4. 验证环境

```bash
x86_64-w64-mingw32-gcc --version
wine --version
rustup target list --installed | grep windows
```

三条都能正常输出，环境即就绪。之后把第二部分整段丢给实施方。

---

# 第二部分：重构 Prompt（交给实施方）

请重构以下仓库：

```
https://github.com/yangxuqin/pptx_selector_next
```

**目标：** 在完全保持原项目功能与使用行为的前提下，将 C++ 实现重构为轻量、高性能、低内存占用的 Rust 实现。这是**重构，不是重写**。

**运行环境：** Arch Linux 开发、构建、测试。**无 Windows 环境运行 C++ 原版**，所有行为对齐基于源码审读。交叉编译工具链与 Wine **已由开发者预先安装配置**，你无需执行任何 `sudo` 或系统级安装命令。

---

## 0. 工作顺序（强制）

**按以下阶段严格顺序执行，不得跳步：**

```
第一阶段：源码审计（只读）
    ↓ 产出行为映射文档，禁止写 Rust 代码
第二阶段：建立测试 fixture
    ↓ 基于行为映射，构造最小 PPTX 测试样本
第三阶段：Rust 最小实现
    ↓ 只实现审计中确认的行为，不多做
第四阶段：行为对齐验证
    ↓ 逐函数对照 C++ 源码
第五阶段：流式 ZIP 性能验证
    ↓ >500MB 文件，测量 RSS 增量
第六阶段：Linux 测试 + Windows 交叉编译 + Wine 验证
    ↓
第七阶段：最终报告
```

**第一阶段硬性约束：** 只允许读取、分析和记录原项目行为，**不允许修改原仓库任何文件**。完成行为映射后，方可开始 Rust 实现。

**原 C++ 项目是行为基准。** 除非为了添加测试 fixture、行为映射文档或 Rust 实现确有必要，**不得修改、重构、格式化或"顺便优化"原 C++ 业务代码**。Rust 实现完成后，原 C++ 源码仍应保持可审阅状态，以便逐函数进行行为对照。

---

## 1. 源码审计（第一阶段）

在写任何 Rust 代码前，完整审阅 C++ 源码：

- `main.cpp`、`config`、`logger`、`pptx_detector`、`app_launcher`、`error_codes`、`config.ini`

**产出物：** 一份行为映射文档（Markdown 即可），逐函数记录：

| C++ 函数 | 行为描述 | 错误码 | 日志文本 | 边界条件 |
|---|---|---|---|---|

Rust 版本必须以**原项目的源码逻辑**为准。

原项目已有的下列特性，Rust 版必须保持兼容：

- `config.ini` 格式与键名
- PowerPoint / WPS 判断逻辑
- Fallback 机制
- 日志机制（启用开关、位置、格式）
- 错误处理与错误码
- 启动参数语义
- Windows 无控制台窗口行为
- Unicode（中文、带空格）路径处理

**遇到无法确定的行为时：** 优先通过原项目其他源码、调用关系、配置模板、错误码、日志文本和已有测试继续推断；只有仍无法确定时，才保留原行为中最保守的实现，并在最终报告列出疑点。**禁止自行设计新行为。**

---

## 2. PPTX 流式读取（第三阶段核心）

原项目：`整个 PPTX → 读入内存 → ZIP 解析`

Rust 版本必须改成流式：

```
File::open()
   ↓
ZIP 中央目录（读取/解析以定位 entry）
   ↓
读取 docProps/app.xml
   ↓
流式 XML 解析
   ↓
<Application> 标签值
```

**硬性约束：**

- **禁止**将整个 PPTX 文件加载进内存
- **禁止**一次性解压全部 ZIP 条目
- **只读取** `docProps/app.xml`
- XML 采用流式事件解析，不构建完整 DOM

**关于中央目录的澄清：** 允许 ZIP 库读取和解析中央目录以定位 `docProps/app.xml`，但**不得将整个 PPTX 文件读入内存**，也**不得将中央目录转换成与文件规模成比例的大型对象集合**。最终以内存实测结果为准。

**ZIP 层 crate 选择：**

- **首选 `rawzip`**。实施方**必须先确认当前 crate 版本的实际 API**（`ZipArchive::from_file` 是否仍为主入口、是否需要额外 buffer 参数等），不得照搬过时文档。
- **禁止引入需要 Tokio / async runtime 的 ZIP 方案。**
- **不使用默认配置的 `zip` crate**：默认将整个文件读入内存后解析中央目录，不满足流式要求。若坚持使用，必须启用其 `stream` feature 并实测峰值内存。

**XML 层：** 使用 `quick-xml` 的 `Reader::from_reader` 流式模式，只提取 `<Application>` 文本。**不要**走 `serde` 反序列化路径。`app.xml` 本身通常仅几 KB，**真正的内存瓶颈在 ZIP 解压环节**。

---

## 3. 保持 config.ini 兼容

**不要修改配置格式。** 继续兼容原项目：

```ini
[Paths]
PowerPoint=...
WPS=...
LogFile=...

[Options]
EnableLog=1
Fallback=0
```

- 路径依旧由 `config.ini` 提供
- **不新增** Windows Registry 检测
- **不新增** 自动搜索 Office / WPS 安装目录的逻辑
- 键名、节名、默认值、大小写敏感性均以原项目源码为准

---

## 4. 保持原日志行为

严格参考原 `logger` 模块的实际实现。

错误发生时的行为：

```
写日志
   ↓
静默处理（进程继续或按原逻辑退出）
```

**禁止：** 弹 MessageBox、调用 CMD / PowerShell、输出到控制台、弹任何 GUI 提示。程序必须**始终保持完全无命令行窗口**。

日志是否启用、日志文件位置、日志格式（时间戳、级别、内容）**以原项目源码为准**，逐字符对照。

---

## 5. 保持 Fallback

以下行为**必须保留**：

- WPS → PowerPoint 回退
- PowerPoint → WPS 回退
- 首选程序启动失败 → 触发回退
- 回退开关受 `[Options] Fallback` 控制

---

## 6. Rust 目录结构

```text
src/
├── main.rs        // 入口、windows_subsystem、参数解析
├── config.rs      // config.ini 读取
├── logger.rs      // 日志写入
├── detector.rs    // PPTX → Application 判断
├── launcher.rs    // CreateProcessW 启动外部程序
└── error.rs       // 错误码与错误类型
build.rs           // 嵌入 Windows 图标
resources/
└── icon.ico
```

Windows 专属代码用 `#[cfg(windows)]` 隔离。不引入不必要的 trait 抽象层。

---

## 7. 轻量化目标与依赖约束

优先级：快速启动 > 低内存 > 小体积 > 少依赖 > 无后台进程 > 无 GUI Framework > 无 Tokio / async runtime。

Release 配置：

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

**依赖白名单：**

| Crate | 用途 |
|---|---|
| `rawzip` | ZIP 流式读取（首选；实施方自行确认版本 API） |
| `quick-xml` | 流式 XML 解析 |
| `windows-sys` | Win32 API（feature 根据实际调用配置，见第 8 节） |
| `winresource`（build-dep） | 嵌入图标（见第 8 节） |

**禁止引入：** `tokio`、`async-std`、`reqwest`、`anyhow`、`thiserror`、`serde`、`log`、`env_logger`、`clap`、`eframe`、`egui`、以及任何需要 Tokio / async runtime 的 ZIP 方案。

**错误处理与 `panic = "abort"`：**

- 所有 I/O、ZIP、XML 错误通过 `Result` 传播
- **禁止**在最终运行程序 `src/` 的探测、配置、日志、启动路径上使用 `.unwrap()` / `.expect()`
- 损坏 PPTX、缺少 `app.xml`、`config.ini` 缺失等场景，返回 `Err` 并由上层走 Fallback 或按原逻辑处理
- `build.rs` 属于构建脚本，**不受此约束**：资源编译失败时可直接 `expect()` 终止构建
- 若原项目在特定场景确实需要 panic，需在代码注释中说明理由

---

## 8. Windows 平台细节

**无控制台窗口。** 在 `main.rs` 顶部添加：

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
```

**明确要求：** Release 构建的 Windows target **必须最终使用 Windows subsystem**（不弹黑框）；Linux 原生 debug / test 不受影响。

**启动外部程序的 API。** **必须使用 `CreateProcessW`**，不要用 `std::process::Command`。理由：`CreateProcessW` 可精确控制 `STARTUPINFOW`、`CREATE_NO_WINDOW`、命令行拼接方式和隐藏窗口参数，与原 C++ 行为一致。`std::process::Command` 虽然底层也调用 `CreateProcessW`，但其抽象层隐藏了部分 Win32 细节，无法保证与原项目完全等价。

**windows-sys feature 配置。** 不要锁死 feature 列表。实施方应根据实际调用的 API，查阅当前 windows-sys 版本文档，配置所需 feature；不同版本中模块归属可能不同（例如部分 Win32 类型在较新版本中被拆分到独立模块），缺少 feature 会导致编译错误，实施方需根据编译报错逐项补齐。

**图标嵌入（build.rs）。** winresource 在交叉编译时有**已知陷阱**：`build.rs` 在 host 上执行，`#[cfg(target_os = "windows")]` 不会按预期工作。必须改用 `CARGO_CFG_TARGET_OS` 环境变量判断：

```rust
// build.rs
fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("resources/icon.ico");
        res.compile().expect("failed to compile resources");
    }
}
```

**回退策略：** 若 winresource 在当前 MinGW 版本下无法完成交叉编译，实施方应报告具体错误，并根据实际工具链调整（如显式指定 `windres` 路径或换用 `embed-resource`），**不得擅自放弃图标嵌入或更改整个方案**。图标文件从原仓库 `git clone` 提取。

---

## 9. 交叉编译

环境已由开发者预先配置好（`mingw-w64-gcc` 已安装，`x86_64-pc-windows-gnu` 目标已添加，`.cargo/config.toml` 中 linker 已指定）。确保下列命令可成功执行：

```bash
cargo build --release --target x86_64-pc-windows-gnu
```

目标：从 Arch Linux 直接产出 Windows x64 EXE，保留 Windows 图标与无控制台窗口特性。

---

## 10. 测试清单

**A. Arch 原生测试（`cargo test`）：**

平台无关逻辑（ZIP 解析、XML 解析、config 读取、logger 输出）全部覆盖单元测试。**优先使用仓库已有测试样本**；若不存在，则构造最小合法 PPTX fixture，仅修改 `docProps/app.xml` 的 `<Application>` 值模拟 PowerPoint/WPS。

| 场景 | 期望 |
|---|---|
| PowerPoint fixture PPTX | 识别为 PowerPoint |
| WPS fixture PPTX | 识别为 WPS |
| 未知 Application | 按原 C++ 源码逻辑处理 |
| 损坏 PPTX | 返回 `Err`，不 panic |
| 缺少 `docProps/app.xml` | 返回 `Err`，不 panic |
| 中文路径 | 路径处理正常 |
| 带空格路径 | 路径处理正常 |
| config.ini 缺失/异常 | 按原 C++ 源码逻辑处理 |
| 日志开启/关闭 | 按原 C++ 源码逻辑处理 |
| >500MB PPTX | 见下方内存验收标准 |

**内存验收标准：**

- 测量**PPTX 探测操作期间**的进程 RSS 峰值，**相对于程序启动后的基线 RSS**，增量 < 20MB
- 测试**不包含 Wine**
- **优先使用真实 >500MB PPTX**；若无法获得，则构造结构合法、可被 ZIP/PPTX 正常解析、且 `docProps/app.xml` 有效的 >500MB 测试文件
- **不得通过修改探测逻辑或绕过 ZIP 解析来满足内存指标**
- 测量方式建议：在 Rust 测试中读取 `/proc/self/status` 的 `VmRSS`，或外部用 `/usr/bin/time -v` 监控

**B. Windows / Wine 验证（Wine 已由开发者安装）：**

Windows 专属逻辑（`CreateProcessW`、无控制台窗口、图标）无法在 Linux 原生 `cargo test` 中测试。使用 `#[cfg(windows)]` 隔离，通过 Wine 集成验证：

| 场景 | 验证方式 |
|---|---|
| 无控制台窗口 | Wine 运行 EXE，观察是否弹黑框 |
| 图标显示 | Wine 检查任务栏/资源管理器图标 |
| `CreateProcessW` 启动外部程序 | Wine 下 mock 替身 exe，验证命令行拼接与参数传递 |
| Fallback 触发 | Wine 下 mock 首选程序失败，验证回退 |
| Unicode / 带空格路径传给外部程序 | Wine 下 mock 验证 |

Wine 无法 100% 等价 Windows。**若 Wine 验证不通过，须在报告中明确标注"未在真实 Windows 上验证"。**

---

## 11. 最终要求

完成后运行：

```bash
cargo fmt
cargo clippy
cargo test
cargo build --release
cargo build --release --target x86_64-pc-windows-gnu
```

**构建产物区分：**

- `cargo build --release` → Linux 原生构建验证
- `cargo build --release --target x86_64-pc-windows-gnu` → Windows EXE 交叉编译验证

**最终报告必须包含：**

- 行为映射文档（第一阶段产出）
- 修改内容
- Arch 原生测试结果
- Wine 验证结果（若执行）
- **Linux 构建产物**路径与体积
- **Windows EXE** 路径与体积
- 内存测试原始数据（基线 RSS、峰值 RSS、增量）
- 任何**未能验证**或**存疑**的行为点，明确列出

**不要为了"架构漂亮"增加不必要的模块或功能。**