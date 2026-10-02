# pptx_selector_next → Rust 重构最终报告

> 第七阶段产出。对应说明书第 11 节「最终要求」的必含项。
> 行为基准：`../pptx_selector_next`（提交 `8866e16`），**全程未改动**。
> 重构产物：`pptx_selector_rust/`。
> 开发/构建/测试环境：Arch Linux，rustc 1.98.1，MinGW-w64 GCC 16.2.0，Wine 11.18。

---

## 1. 交付物

| 交付物 | 路径 |
|---|---|
| 行为映射文档（第一阶段） | [`docs/behavior-mapping.md`](behavior-mapping.md) |
| 重构源码 | [`src/`](../src)（6 个模块，与说明书第 6 节一致） |
| 构建脚本 | [`build.rs`](../build.rs) |
| 交叉编译配置 | [`.cargo/config.toml`](../.cargo/config.toml) |
| 运行配置模板 | [`config.ini`](../config.ini)（与原项目逐字节一致，`diff` 无差异） |
| 图标 | [`resources/icon.ico`](../resources/icon.ico)（与原项目逐字节一致，`cmp` 无差异） |
| 测试 fixture 生成器 | [`scripts/make_fixtures.py`](../scripts/make_fixtures.py) |
| 一键验收 | [`scripts/verify.sh`](../scripts/verify.sh) |
| 内存测量 | [`scripts/measure_rss.sh`](../scripts/measure_rss.sh) |
| Wine 验证 | [`scripts/wine_checks.sh`](../scripts/wine_checks.sh) + [`examples/mock_launcher.rs`](../examples/mock_launcher.rs) |
| AI 协作指引 | [`AGENTS.md`](../AGENTS.md) |
| 用户文档 | [`README.md`](../README.md) |

---

## 2. 阶段执行记录

严格按说明书第 0 节的强制顺序执行：

| 阶段 | 内容 | 结果 |
|---|---|---|
| 一 | 源码审计（只读） | ✅ 产出 `docs/behavior-mapping.md`；**未写任何 Rust 代码**；原仓库零改动 |
| 二 | 建立测试 fixture | ✅ 14 个小 fixture + 1 个 545 MB fixture |
| 三 | Rust 最小实现 | ✅ 只实现审计确认的行为 |
| 四 | 行为对齐验证 | ✅ 逐函数对照，差异记入 D1–D14 |
| 五 | 流式 ZIP 性能验证 | ✅ 545 MB 文件，增量 140 kB（release） |
| 六 | Linux 测试 + 交叉编译 + Wine | ✅ 72 个用例全绿；双目标构建成功；Wine 36/36 |
| 七 | 最终报告 | ✅ 本文件 |

---

## 3. 修改内容

### 3.1 新增（全部位于 `pptx_selector_rust/`）

| 文件 | 说明 |
|---|---|
| `src/main.rs` | 入口、`windows_subsystem`、`CommandLineToArgvW` 参数解析、主流程编排 |
| `src/error.rs` | `ErrorCode`（与 C++ `Err` 数值 1:1）、`DetectError` |
| `src/config.rs` | `Config`、`load_config`、`get_exe_dir`（Windows 走真实 profile API） |
| `src/logger.rs` | `Logger`（BOM + `[时间戳]` + CRLF，每条独立 append） |
| `src/detector.rs` | **流式** ZIP 定位 + DEFLATE 解压 + 流式 XML 解析 + 分类 |
| `src/launcher.rs` | `GetFileAttributesW` 检查、命令行引号转义、`CreateProcessW` 启动 |
| `build.rs` | 用 `CARGO_CFG_TARGET_OS` 判断目标平台并嵌入图标 |
| `Cargo.toml` / `Cargo.lock` | 依赖与 release 优化配置 |
| `.cargo/config.toml` | `x86_64-pc-windows-gnu` 的 linker / ar |
| `config.ini` / `resources/icon.ico` | 从原项目复制（原项目只读） |
| `docs/`、`scripts/`、`examples/`、`tests/fixtures/` | 文档、脚本、替身、测试样本 |

### 3.2 相对原项目的**架构性**变更（唯一一处）

原：`CreateFileW → 整文件读入内存 → mz_zip_reader_init_mem → 解压 app.xml → strstr 截取`

新：`CreateFileW（只取句柄与大小）→ rawzip 解析中央目录定位 entry → 只读取该 entry → miniz_oxide 解压 → quick-xml 事件流解析`

硬性约束逐条满足：
- ✅ 禁止将整个 PPTX 加载进内存 —— ZIP 层缓冲区固定 64 KiB，与文件大小无关
- ✅ 禁止一次性解压全部 ZIP 条目 —— 只 `get_entry` 目标条目
- ✅ 只读取 `docProps/app.xml`
- ✅ XML 采用流式事件解析，不构建 DOM

### 3.3 依赖

运行时依赖树（`cargo tree`）：

```
Linux:    pptx_selector
          ├── miniz_oxide 0.9.1 └── adler2 2.0.1
          ├── quick-xml 0.42.0  └── memchr 2.8.3
          └── rawzip 0.5.1              （零依赖）
Windows:  上述 + windows-sys 0.61.2 └── windows-link 0.2.1
build:    winresource 0.1.31
```

- **白名单增补项 `miniz_oxide`**：实测确认 `rawzip` 0.5.1 **完全不含解压能力**（纯容器库，只解析中央目录、定位 entry、提供原始字节流；其官方示例必须外挂 `flate2`），而真实 PPTX 的 `docProps/app.xml` 是 DEFLATE 压缩的。`miniz_oxide` 是原项目所用 **miniz 的官方 Rust 移植**，纯 Rust、无 async、无 std 依赖、体积极小，仅多一个传递依赖 `adler2`。**此项已获用户批准。**
- ✅ 未引入 `tokio` / `async-std` / `reqwest` / `anyhow` / `thiserror` / `serde` / `log` / `env_logger` / `clap` / `eframe` / `egui`
- ✅ 未使用 `std::process::Command`（启动一律走 `CreateProcessW`）
- ✅ 无 Tokio / async runtime，无 GUI framework，无后台进程

### 3.4 与原 C++ 的有意差异

完整清单见 [`docs/behavior-mapping.md`](behavior-mapping.md) §10（D1–D14）。关键几条：

| 编号 | 差异 | 影响 |
|---|---|---|
| D1 | 日志措辞 `文件已读入内存 (N 字节)` → `文件大小 (N 字节)` | 仅该行文本（**已获用户批准**） |
| D2 | 白名单增补 `miniz_oxide` | 无（**已获用户批准**） |
| D3 | 不再复现 C++ `extractTag` 的越界读取 UB | 无 |
| D4 | 新增 4 MiB app.xml 上限（防解压炸弹） | 仅病态文件 |
| D5a | `<x:Application>` / 带属性写法可被识别（C++ 字面匹配不会） | 真实文件一致 |
| D5b | **实体引用与 CDATA 已按文档原文逐字节还原**，与原项目裸字节截取一致；残余差异仅"`<Application>` 内含子元素标记" | 真实文件一致 |
| D7 | ZIP/XML 失败经 `Result` 传播后折叠为 `Unknown`，不额外写日志 | 无（退出码与日志完全一致） |
| D8 | 保留原 `CREATE_NEW_PROCESS_GROUP`，**不引入** `CREATE_NO_WINDOW`（原源码无此项） | 无 |
| D14 | 日志 L13（部分读入）在流式实现中不可达 | 仅极端 I/O 错误 |

---

## 4. Arch Linux 原生测试结果

```
cargo test
test result: ok. 71 passed; 0 failed; 1 ignored        （默认运行；ignored 为 >500MB 内存测试）
cargo test --release --config 'profile.release.panic="unwind"' -- --ignored
test result: ok. 1 passed; 0 failed; 1 ignored → 实际该用例通过
```

合计 **72 个用例全部通过**（71 + 1）。覆盖说明书第 10 节 A 表逐项：

| 场景 | 用例 | 结果 |
|---|---|---|
| PowerPoint fixture PPTX | `detects_powerpoint` | ✅ 识别为 MicrosoftOffice |
| WPS fixture PPTX | `detects_wps` | ✅ 识别为 Wps |
| Kingsoft | `detects_kingsoft_as_wps` | ✅ 归入 Wps |
| 未知 Application | `unknown_application_is_not_an_error` | ✅ `Ok` + `Unknown`（非错误） |
| 分类优先级 | `wps_wins_when_both_markers_are_present` | ✅ `"WPS Microsoft"` → Wps |
| 损坏 PPTX | `corrupt_pptx_returns_err_without_panicking` | ✅ `Err`，不 panic |
| 缺少 `docProps/app.xml` | `missing_app_xml_returns_entry_not_found` | ✅ `Err` |
| CRC 不匹配 | `crc_mismatch_returns_verification_failed` | ✅ `Err` |
| 0 字节文件 | `zero_byte_file_returns_err` | ✅ `Err` |
| 目录当文件 | `directory_instead_of_file_returns_err` | ✅ `Err` |
| 超大 app.xml（5 MiB） | `oversized_app_xml_is_rejected` | ✅ `Err`（上限防护） |
| entry 名大小写不敏感 | `entry_lookup_is_case_insensitive` | ✅ `DocProps/App.xml` 可定位 |
| STORE 压缩 | `stored_app_xml_is_supported` | ✅ |
| 实体引用原样保留 | `extract_application_restores_entity_references_verbatim` | ✅ `&#174;` 未被解码 |
| CDATA 原样保留 | `extract_application_restores_cdata_markers_verbatim` | ✅ |
| 中文路径 | `chinese_path_is_handled` | ✅ |
| 带空格路径 | `path_with_spaces_is_handled` | ✅ |
| config.ini 缺失/异常 | `missing_ini_keeps_defaults` / `malformed_integer_values_fall_back_to_zero` | ✅ 保留默认 |
| 日志开启/关闭 | `disabled_logger_writes_nothing` / `first_line_starts_with_bom_and_has_timestamp` | ✅ |
| 决策表 | `choose_launcher_matches_the_cpp_decision_table` | ✅ 7 种组合全覆盖 |
| 兜底分支 | `fallback_prefers_wps_unless_fallback_is_exactly_one` | ✅ 仅 `==1` 走 PowerPoint |
| 退出码 | `exit_codes_match_the_cpp_err_enum` | ✅ 0–6 数值一致 |
| 命令行引号规则 | `launcher::tests::*`（7 个用例） | ✅ 含结尾反斜杠翻倍、引号转义、Unicode |
| >500MB 内存 | `memory_peak_increment_on_large_pptx` | ✅ 见第 6 节 |

静态检查：

```
cargo fmt --check                                     ✅ 无差异
cargo clippy --all-targets -- -D warnings             ✅ 0 warning
cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings  ✅ 0 warning
生产代码 .unwrap()/.expect() 机检                       ✅ 0 处
```

端到端行为抽查（Linux release 二进制，含中文与空格路径）：

| 场景 | 退出码 | 日志 |
|---|---|---|
| 无参数 | 1 | `错误: 命令行参数不足 (argc=1)` |
| 目标文件不存在 | 2 | `错误: 目标文件不存在，兜底启动` + 兜底序列 |
| 0 字节文件 | 3 | `错误: 读取文件失败，兜底启动` |
| 正常探测（Linux 无法启动进程） | 5 | 完整 16 行链路，BOM + CRLF + `[时间戳]` |
| stdout/stderr 字节数 | — | **0 字节**（完全静默） |

---

## 5. Wine 验证结果

`bash scripts/wine_checks.sh` → **36 项断言全部通过**（Wine 11.18，独立 `WINEPREFIX`）。

| # | 验证项 | 结果 |
|---|---|---|
| 1 | PE `Subsystem = 00000002`（Windows GUI，**无控制台窗口**） | ✅ `PE32+ executable for MS Windows 5.02 (GUI)` |
| 1 | `.rsrc` 含 `RT_ICON` + `RT_GROUP_ICON`（**图标已嵌入**） | ✅ RT_ICON 大小 0x10828 = 67624 B（= icon.ico） |
| 2 | WPS 文件 → 启动 WPS 替身，未启动 PowerPoint | ✅ |
| 2 | **Unicode / 带空格路径传给外部程序** | ✅ argv[1] 的 UTF-16 码元逐个匹配 `C:\pptxcheck\测试 文稿.pptx` |
| 2 | 日志链路（决策 / 检测结果 / 启动成功） | ✅ |
| 3 | PowerPoint 文件 → 启动 PowerPoint 替身 | ✅ |
| 4 | **Fallback：WPS 不存在 → 降级 PowerPoint** | ✅ `WPS 不存在，降级为 PowerPoint 打开` |
| 5 | 未知创建者 + `Fallback=1` → 兜底 PowerPoint | ✅ `未找到可用的首选启动器，尝试兜底` + `兜底: 使用 PowerPoint 打开` |
| 6 | 未知创建者 + `Fallback=0` → 兜底 WPS | ✅ |
| 7 | 目标文件不存在 → 退出码 2 且仍兜底启动 | ✅ argv[1] 仍是原目标路径 |
| 8 | 日志字节格式：UTF-8 BOM、CRLF、`[YYYY-MM-DD HH:MM:SS] ` | ✅ |

> ⚠️ **Wine 无法 100% 等价 Windows**。上述结果**不代表已在真实 Windows 上验证**。

---

## 6. 内存测试原始数据

### 6.1 进程内测量（说明书推荐方式，`/proc/self/status`）

测试：`detector::tests::memory_peak_increment_on_large_pptx`（`--ignored`）
方法：探测前后读取 `VmRSS` / `VmHWM`，`增量 = 探测后 VmHWM − 基线 VmRSS`（保守上界）。

**fixture**：`tests/fixtures/big_512mb.pptx`，**545,260,867 字节**（1 个 STORE 大条目 + 正常 DEFLATE 的 `docProps/app.xml`，ZIP 结构完全合法）

| Profile | 基线 RSS | 基线 VmHWM | 探测后 VmHWM | 探测后 VmRSS | **增量** |
|---|---|---|---|---|---|
| `--release` | 2 980 kB | 2 984 kB | 3 120 kB | 3 120 kB | **140 kB** |
| dev（`cargo test`） | 3 444 kB | 3 576 kB | 3 812 kB | 3 812 kB | **368 kB** |

（`--release` 需 `--config 'profile.release.panic="unwind"'`，因为 `panic="abort"` 与测试
harness 冲突；这不影响二进制本身。）

### 6.2 端到端测量（真实 release 二进制，`getrusage(RUSAGE_CHILDREN)`）

`bash scripts/measure_rss.sh`。本机**未安装 GNU `/usr/bin/time`**，改用 Python 的
`resource.getrusage(RUSAGE_CHILDREN).ru_maxrss` —— 这正是 `/usr/bin/time -v` 内部使用的
同一数据源（Linux 上单位为 kB）；每次测量在独立 Python 进程中执行。

| 运行 | 峰值 RSS |
|---|---|
| 基线（小 PPTX 1 213 B） | 12 096 kB |
| 大文件（545 260 867 B） | 12 080 kB |
| **增量** | **0 kB**（大文件未超过基线） |

### 6.3 验收判定

| 项 | 门槛 | 实测 | 结论 |
|---|---|---|---|
| 探测期间 RSS 峰值增量 | < 20 MB (20 480 kB) | 140 kB（release 进程内） / 0 kB（端到端） | ✅ **通过** |
| 测试是否包含 Wine | 不含 | 不含 | ✅ |
| 是否修改探测逻辑或绕过 ZIP 解析 | 不得 | 未修改；`app.xml` 走完整中央目录定位 + DEFLATE 解压 + CRC32 校验 | ✅ |

---

## 7. 构建产物

| 产物 | 路径 | 体积 |
|---|---|---|
| Linux 原生 | `target/release/pptx_selector` | **390 040 字节**（约 384 KiB） |
| Windows x64 EXE | `target/x86_64-pc-windows-gnu/release/pptx_selector.exe` | **404 992 字节**（约 396 KiB） |

Windows EXE 特性：
- `file`：`PE32+ executable for MS Windows 5.02 (GUI), x86-64 (stripped to external PDB)`
- 子系统：`2 (Windows GUI)` → **无控制台窗口**
- 资源：`RT_ICON` / `RT_GROUP_ICON` / `RT_VERSION` 均已嵌入
- 依赖：全静态（仅 `kernel32` / `shell32` / `advapi32` 等系统 DLL）

命令（说明书第 11 节要求，均执行成功）：

```bash
cargo fmt         ✅      cargo clippy      ✅      cargo test        ✅
cargo build --release                             ✅
cargo build --release --target x86_64-pc-windows-gnu  ✅
```

---

## 8. 未能验证 / 存疑的行为点

### 8.1 未能验证

| # | 项 | 原因 |
|---|---|---|
| 1 | **真实 Windows 上的运行行为** | 本机只有 Arch Linux + Wine。无控制台窗口、图标显示、`CreateProcessW` 等价性仅通过 **PE 结构检查 + Wine 集成验证**间接确认 |
| 2 | **与原 C++ 二进制的逐字节/逐行为对比** | 无法运行 C++ 原版，所有对齐均基于源码审读 |
| 3 | `SHCreateDirectoryExW` 的本地时间戳渲染 | 非 Windows 下时间戳 shim 使用 UTC，**本地时区渲染只能在 Windows 上验证** |
| 4 | 真实 Office / WPS 程序被本程序拉起后的完整交互 | Wine 下用替身 EXE 验证了命令行与启动本身，未安装真实 Office/WPS |
| 5 | `GetPrivateProfileIntW` 在真实 Windows 上对病态取值的返回 | Wine 的 `kernel32` 实现未必与 Windows 完全一致 |

### 8.2 存疑（源码层面无法确定，已在行为文档 U1–U6 列明）

| # | 疑点 | 本项目采取的立场 |
|---|---|---|
| U1 | 行为对齐全部基于源码审读 | 差异与假设已在 `docs/behavior-mapping.md` 全量记录 |
| U2 | `GetPrivateProfileStringW` / `GetPrivateProfileIntW` 非数字取值的返回 | Windows 上调用真实 API（无需猜测）；Linux shim 取"键存在但无数字 → 0" |
| U3 | INI 重复键取第一次还是最后一次 | Win32 未定义；未在 fixture 中构造该情况 |
| U4 | 说明书 §8 提到 `CREATE_NO_WINDOW`，但原 C++ 源码**未使用** | **以源码为准**，保留 `CREATE_NEW_PROCESS_GROUP`（D8） |
| U5 | `SHCreateDirectoryExW` 在 UNC / 超长路径下的行为 | 忽略返回值，与 C++ 一致 |
| U6 | Wine 与真实 Windows 的差异 | 已在第 5 节显式标注 |

### 8.3 已知的、有意的行为差异（需使用者知悉）

1. **日志第 11 行措辞**：`文件已读入内存 (N 字节)，耗时 M ms` → `文件大小 (N 字节)，耗时 M ms`
   （已获用户批准）。
2. **日志 L13**（`错误: 读取文件失败 (读取 N / 期望 M 字节)`）在流式实现中**不可达**。
3. `<Application>` 内部若含**子元素标记**，Rust 会丢弃标记（真实 app.xml 只含纯文本）。
4. 畸形 XML 下 `quick-xml` 会报错并降级为 `Unknown`，而 C++ 的裸字节扫描可能截取到内容。
5. `app.xml` 读取/解压设有 4 MiB 上限（C++ 无上限）。

---

## 9. 结论

- ✅ 功能与使用行为对齐：`config.ini` 格式、PowerPoint/WPS 判断、双向降级与兜底、
  日志机制（开关/位置/格式/UTF-8+BOM+CRLF）、错误码与退出码、启动参数语义、
  无控制台窗口、Unicode（中文/空格）路径——逐项落实并测试。
- ✅ 完成流式 ZIP 改造：545 MB 文件下 RSS 峰值增量 **140 kB**（门槛 20 MB），
  远低于验收标准；未通过任何取巧手段达成。
- ✅ 轻量化：Windows EXE **396 KiB**，运行时依赖树仅 5 个 crate（含传递依赖），
  无 Tokio / async runtime / GUI framework。
- ✅ 完整交叉编译链路：Arch Linux 直接产出带图标、无控制台窗口的 Windows x64 EXE。
- ✅ 原项目 `../pptx_selector_next` **零改动**（`git status --porcelain` 为空，已纳入验收脚本）。
- ⚠️ 主要限制：**Windows 专属行为仅在 Wine 下验证，未在真实 Windows 上验证**；
  且无法与 C++ 原版做二进制级对比。
