# 行为映射文档（第一阶段产出）

> 对象：`pptx_selector_next`（C++/Win32，行为基准）
> 方式：**纯源码审读**（本机无 Windows 运行环境，无法运行 C++ 原版）
> 基准提交：`8866e16`（Merge pull request #2 from agui4541/master）
> 用途：Rust 重构的行为对齐依据。Rust 侧一律以本文件记录的 C++ 逻辑为准。
>
> 本阶段为只读审计，未修改原仓库任何文件。

---

## 0. 审计范围

| 文件 | 行数 | 内容 |
|---|---|---|
| `src/main.cpp` | 194 | 入口 `WinMain` + 匿名命名空间辅助函数 |
| `src/config.h` / `config.cpp` | 34 / 52 | `Config`、`LoadConfig`、`GetExeDir` |
| `src/logger.h` / `logger.cpp` | 42 / 91 | `Logger` 类 |
| `src/pptx_detector.h` / `pptx_detector.cpp` | 36 / 90 | `AppCreator`、`DetectionResult`、`detectPptxCreator` |
| `src/app_launcher.h` / `app_launcher.cpp` | 26 / 80 | `appExists`、`launchApp` |
| `src/error_codes.h` | 24 | `enum class Err` |
| `config.ini` | 24 | 运行配置模板 |
| `third_party/miniz.{c,h}` | — | ZIP 解压（第三方，仅审读其调用语义） |

---

## 1. 错误码（`error_codes.h`）

```cpp
enum class Err {
    Ok              = 0,   // 成功
    InvalidArgs     = 1,   // 命令行参数不足
    FileNotFound    = 2,   // 目标 PPTX 文件不存在
    ReadFailed      = 3,   // 文件读取失败
    InvalidArchive  = 4,   // ZIP 解析失败
    LaunchFailed    = 5,   // 启动外部程序失败
    NoLauncher      = 6,   // 未找到任何可用启动器
};
```

| C++ 名称 | 数值 | 是否会被 `WinMain` 返回 | Rust 名称 |
|---|---|---|---|
| `Err::Ok` | 0 | ✅ | `ErrorCode::Ok` |
| `Err::InvalidArgs` | 1 | ✅ | `ErrorCode::InvalidArgs` |
| `Err::FileNotFound` | 2 | ✅ | `ErrorCode::FileNotFound` |
| `Err::ReadFailed` | 3 | ✅ | `ErrorCode::ReadFailed` |
| `Err::InvalidArchive` | 4 | ❌ **从不返回** | `ErrorCode::InvalidArchive`（保留枚举完整性） |
| `Err::LaunchFailed` | 5 | ✅ | `ErrorCode::LaunchFailed` |
| `Err::NoLauncher` | 6 | ❌ **从不返回** | `ErrorCode::NoLauncher`（保留枚举完整性） |

> 关键约束：ZIP 解析失败**不是** `InvalidArchive`。`detectPptxCreator` 内部吞掉失败并返回 `creator = Unknown`，随后走"未找到可用首选启动器 → 兜底"路径，最终退出码是 0 或 5。

---

## 2. `config` 模块

### 2.1 `Config::Config()`

| 字段 | 默认值 | 类型 |
|---|---|---|
| `pathPowerPoint` | `C:\Program Files\Microsoft Office\root\Office16\POWERPNT.EXE` | `std::wstring` |
| `pathWPS` | `C:\Program Files\Kingsoft\WPS Office\12.1.0.23542\office6\wpp.exe` | `std::wstring` |
| `logFile` | `D:\logs\pptx_selector.log` | `std::wstring` |
| `enableLog` | `true` | `bool` |
| `fallback` | `0`（0=WPS，1=PowerPoint） | `int` |

边界条件：无 `config.ini` 时这些默认值直接生效。

### 2.2 `LoadConfig(Config& cfg, const wchar_t* iniPath)`

- 每字段用 `GetPrivateProfileStringW(section, key, **当前字段值**, buffer, 32768, iniPath)`：
  第三个参数传"当前字段值"，因此 **INI 缺失该键时保留原值（即默认值或已加载值）**。
  - `[Paths] PowerPoint` → `pathPowerPoint`
  - `[Paths] WPS` → `pathWPS`
  - `[Paths] LogFile` → `logFile`
- `enableLog = (GetPrivateProfileIntW(L"Options", L"EnableLog", 1, iniPath) != 0)` → **默认 1（启用）**，任何非 0 值都为 true。
- `fallback = GetPrivateProfileIntW(L"Options", L"Fallback", 0, iniPath)` → **默认 0**，取值原样保存（不裁剪为 {0,1}）。
- 返回值经 `std::wstring(buffer.data(), length)` 构造，`length` 是 `GetPrivateProfileStringW` 的返回长度。
- 缓冲区大小固定 32768 wchar；`GetPrivateProfileStringW` 超长时截断。

边界条件：
- 文件不存在 / 节不存在 / 键不存在 → 全部保留默认值，**不报错、不返回错误码**。
- 节名、键名大小写不敏感（Win32 profile API 语义）。
- `GetPrivateProfileIntW` 默认值分别是 1（EnableLog）与 0（Fallback）。

### 2.3 `GetExeDir()`

1. `GetModuleFileNameW(nullptr, path, 32768)`。
2. `length == 0 || length >= path.size()` → 返回 `L"."`。
3. 取 `result(path, length)`，`find_last_of(L"\\/")`：
   - `npos` → 返回 `L"."`；
   - 否则 `resize(last)` → **不带末尾反斜杠**。

边界条件：根目录（如 `C:\a.exe`）会返回 `C:`。

---

## 3. `logger` 模块

### 3.1 构造

- `Logger()`：`enabled_ = false`，`file_` 为空。
- `Logger(bool enableLog, const wchar_t* logFile)`：`enabled_ = enableLog`；`logFile` 非空则 `file_ = logFile`。

### 3.2 `initDir() const`

```
if (!enabled_ || file_.empty()) return;
last = file_.find_last_of(L"\\/");
if (last != npos && last > 0) SHCreateDirectoryExW(nullptr, file_.substr(0, last), nullptr);
```

边界条件：`last == 0`（如 `\x.log`）或 `npos`（无目录部分）→ 不建目录。`SHCreateDirectoryExW` 返回值被忽略（目录已存在时返回 `ERROR_ALREADY_EXISTS` 也不影响）。

### 3.3 `write(const char* format, ...) const`

- 关闭或路径为空或 `format` 为空 → 空操作。
- 用 `_vsnprintf_s(message, 8192, _TRUNCATE, format, args)` 格式化，再 `writeUtf8(message, strlen(message))`。

### 3.4 `writeW(const wchar_t* format, ...) const`

- 同样前置检查；`_vsnwprintf_s(message, 8192, _TRUNCATE, ...)`。
- `WideCharToMultiByte(CP_UTF8, 0, message, -1, ...)`：先取长度，若 `bytes <= 1`（即空串）→ **直接返回，不写任何内容**。
- 否则写 `bytes - 1` 字节（去掉 NUL）。

### 3.5 `writeUtf8(const char* text, std::size_t length) const`（私有）

逐字符对照的输出格式：

```
打开: CreateFileW(file_, FILE_APPEND_DATA, FILE_SHARE_READ|FILE_SHARE_WRITE,
                  nullptr, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr)
失败 → 直接返回（静默）
GetFileSizeEx(file) 且 size == 0 → 先写 3 字节 BOM: EF BB BF
GetLocalTime(&st) → 前缀: "[%04d-%02d-%02d %02d:%02d:%02d] "
                      年4位 月2位 日2位 时2位 分2位 秒2位，均为补零十进制
prefixLength > 0 才写前缀
length > 0 才写正文
固定写 2 字节 CRLF: "\r\n"
CloseHandle(file)
```

**每条日志 = 独立 open/append/write/close**，不做缓冲，不做文件级锁。

边界条件：
- 目录不可写 → `CreateFileW` 失败 → 静默丢弃该行，**不报错、不弹窗**。
- 文件已存在且非空 → 不写 BOM。
- `writeW` 的空串不会产生任何输出（连时间戳与换行都不写）。
- 编码：UTF-8（带 BOM），行尾 CRLF。

---

## 4. `pptx_detector` 模块

### 4.1 类型

```cpp
enum class AppCreator { Unknown, WPS, MicrosoftOffice };
struct DetectionResult { AppCreator creator = Unknown; std::string application; };  // application 是原始字节
```

### 4.2 `extractTag(const char* src, size_t len, const char* tag)`（匿名命名空间）

1. `sTag = "<" + tag + ">"`，`eTag = "</" + tag + ">"`（`_snprintf_s`，缓冲 128）。
2. `p = strstr(src, sTag)`；未找到 → 返回空串。
3. `p += strlen(sTag)`。
4. `e = strstr(p, eTag)`；未找到 → 返回空串。
5. 返回 `std::string(p, e - p)` —— **原始字节，不做实体转义解码、不做空白裁剪**。

边界条件：
- `src == NULL || len == 0 || tag == NULL` → 空串。
- `len` 参数实际上**未被使用**：`strstr` 依赖 NUL 终止符。缓冲区由 `mz_zip_reader_extract_to_heap` 分配，**不保证有 NUL 终止符** → 存在越界读取的潜在 UB（Rust 侧按有界解析处理，见差异清单 D3）。
- 大小写敏感、字面匹配：`<Application >`、`<x:Application>`、`<Application/>` 都**不会**匹配。

### 4.3 `classifyApp(const std::string& appName)`（匿名命名空间）

```cpp
if (appName.find("WPS") != npos || appName.find("Kingsoft") != npos) return AppCreator::WPS;
if (appName.find("Microsoft") != npos || appName.find("PowerPoint") != npos) return AppCreator::MicrosoftOffice;
return AppCreator::Unknown;
```

**顺序不可颠倒**：同时含两类的字符串（如 `"WPS Microsoft"`）归类为 WPS。`std::string::find` 为**大小写敏感的裸字节子串匹配**。

### 4.4 `detectPptxCreator(const void* data, std::size_t size)`

```
if (!data || size == 0) return result;                 // Unknown
mz_zip_archive zip = {};  ZipGuard guard(zip);         // RAII: 退出时 mz_zip_reader_end
if (!mz_zip_reader_init_mem(&zip, data, size,
        MZ_ZIP_FLAG_DO_NOT_SORT_CENTRAL_DIRECTORY))    // 失败 → Unknown
    return result;
guard.valid = true;
int idx = mz_zip_reader_locate_file(&zip, "docProps/app.xml", nullptr, 0);
if (idx < 0) return result;                            // 无该 entry → Unknown
char* pXml = mz_zip_reader_extract_to_heap(&zip, idx, &sz, 0);
if (!pXml) return result;                              // 解压失败 → Unknown
result.application = extractTag(pXml, sz, "Application");
result.creator     = classifyApp(result.application);
mz_free(pXml);
return result;
```

**miniz 调用语义（关键，逐条已核实源码）：**

| 调用 | 语义 |
|---|---|
| `mz_zip_reader_init_mem(..., MZ_ZIP_FLAG_DO_NOT_SORT_CENTRAL_DIRECTORY)` | 中央目录**不排序**（不建查找表） |
| `mz_zip_reader_locate_file(&zip, "docProps/app.xml", nullptr, 0)` | 因初始化带了 `DO_NOT_SORT`，`miniz.c:4505` 的二分查找分支被跳过 → 走**中央目录顺序线性扫描**（`miniz.c:4521`）。`mz_zip_string_equal` 在 `flags=0` 时是 **ASCII 大小写不敏感**；要求 `filename_len == name_len`；返回**第一个**匹配项 |
| `mz_zip_reader_extract_to_heap(&zip, idx, &sz, 0)` | 解压到堆；**仅支持 STORE(0) 与 DEFLATE(8)**；其他方法、加密、CRC 不匹配、尺寸不符 → 返回 NULL |

边界条件：
- `data`/`size` 空 → Unknown。
- 非 ZIP、截断 ZIP、中央目录损坏 → Unknown。
- 缺 `docProps/app.xml` → Unknown。
- entry 有但解压失败（CRC 错、方法不支持、加密）→ Unknown。
- app.xml 中**没有** `<Application>` → `application` 为空串，`creator = Unknown`（**这是成功路径，不是错误**）。
- entry 名大小写：`docprops/app.xml`、`DOCPROPS/APP.XML` 均可被定位。

---

## 5. `app_launcher` 模块

### 5.1 `quoteCommandLineArg(const wchar_t* arg)`（匿名命名空间）

逐行等价实现（Windows 命令行引号规则）：

```
结果以 " 开头
backslashes = 0
遍历每个字符 c:
  if c == '\\': ++backslashes; continue
  if c == '"':  追加 (backslashes*2 + 1) 个 '\\'；追加 '"'；backslashes = 0；continue
  追加 backslashes 个 '\\'；backslashes = 0；追加 c
结束时追加 (backslashes*2) 个 '\\'
追加 "
```

- `arg == NULL` → 结果就是 `""`（两个引号）。
- 空串 → `""`。

### 5.2 `appExists(const wchar_t* exePath)`

```
if (!exePath || !*exePath) return false;
attrs = GetFileAttributesW(exePath);
return attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_DIRECTORY) == 0;
```

边界条件：空指针、空串、不存在、是目录 → 全部 `false`。

### 5.3 `launchApp(const wchar_t* exePath, const wchar_t* filePath, DWORD* errorCode)`

1. `*errorCode = ERROR_INVALID_PARAMETER`（若指针非空）。
2. `!exePath || !*exePath || !filePath || !*filePath` → 返回 false（`errorCode` 保持 `ERROR_INVALID_PARAMETER`）。
3. `commandLine = quote(exePath) + L" " + quote(filePath)`。
4. `STARTUPINFOW startup = {}; startup.cb = sizeof(startup);` `PROCESS_INFORMATION process = {};`
5. `CreateProcessW(exePath, &commandLine[0], nullptr, nullptr, FALSE,
                  CREATE_NEW_PROCESS_GROUP, nullptr, nullptr, &startup, &process)`
   - `lpApplicationName` = exePath（原样，不加引号）；
   - `lpCommandLine` = 已拼好的带引号命令行（可写缓冲区，NUL 结尾）；
   - `bInheritHandles = FALSE`；
   - `dwCreationFlags = CREATE_NEW_PROCESS_GROUP (0x00000200)` —— **没有** `CREATE_NO_WINDOW`；
   - `lpCurrentDirectory = nullptr`（继承本进程当前目录）；
   - `STARTUPINFOW` 其余字段为 0（`dwFlags = 0`，故 `wShowWindow` 不生效）。
6. 失败：`*errorCode = GetLastError()`，返回 false。
7. 成功：`CloseHandle(process.hThread)`、`CloseHandle(process.hProcess)`，`*errorCode = ERROR_SUCCESS`，返回 **true**。

> **重要**：返回值只表示"进程创建成功"，**不等待、不检查子进程退出码**。

---

## 6. `main.cpp`

### 6.1 `readFileToMemory(const wchar_t* path, const Logger& log)`（匿名命名空间）

```
hFile = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, nullptr,
                    OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr)
失败 → log.write("错误: 无法打开文件 (CreateFileW 失败)"); return {}
GetFileSizeEx(hFile, &fsize)
data.resize(fsize.QuadPart)          // 整文件读入内存
ok = ReadFile(hFile, data.data(), data.size(), &bytesRead, nullptr)
CloseHandle(hFile)
if (!ok || bytesRead != data.size()) → log.write("错误: 读取文件失败 (读取 %lu / 期望 %zu 字节)", ...); return {}
return data
```

边界条件：
- 目录：`CreateFileW` 需要 `FILE_FLAG_BACKUP_SEMANTICS` 才能打开目录，故此处**失败** → 走"无法打开文件"日志。
- **0 字节文件**：`data.size() == 0`，`ReadFile` 成功、`bytesRead == 0`，条件通过，返回**空 vector** → 调用方 `fileData.empty()` 判定为读失败。
- 注意两条不同的日志文本：函数内部的"无法打开文件"与"读取文件失败"；而 `main` 里另有第三句"错误: 读取文件失败，兜底启动"。

### 6.2 `chooseLauncher(detection, cfg, wpsExists, pptExists, log)`

| 检测结果 | 条件 | 日志 | 返回 |
|---|---|---|---|
| `WPS` | `wpsExists` | `决策: 使用 WPS 打开` | `cfg.pathWPS` |
| `WPS` | `!wpsExists && pptExists` | `WPS 不存在，降级为 PowerPoint 打开` | `cfg.pathPowerPoint` |
| `WPS` | 都不可用 | （无日志） | `nullptr` |
| `MicrosoftOffice` | `pptExists` | `决策: 使用 PowerPoint 打开` | `cfg.pathPowerPoint` |
| `MicrosoftOffice` | `!pptExists && wpsExists` | `PowerPoint 不存在，降级为 WPS 打开` | `cfg.pathWPS` |
| `MicrosoftOffice` | 都不可用 | （无日志） | `nullptr` |
| `Unknown` | 任意 | （无日志） | `nullptr` |

### 6.3 `launchFallback(cfg, pptxPath, log)`

```
fb = (cfg.fallback == 1) ? cfg.pathPowerPoint : cfg.pathWPS      // 非 1 一律走 WPS
log.write("兜底: 使用 %s 打开", fallback == 1 ? "PowerPoint" : "WPS")
launched = launchApp(fb, pptxPath, &errorCode)
if (!launched) log.write("错误: 兜底启动失败 (Win32=%lu)", errorCode)
else           log.write("兜底启动成功")
return launched
```

### 6.4 `WinMain` 主流程（逐行）

| 步 | 代码 | 日志 | 退出码 |
|---|---|---|---|
| 1 | `t0 = GetTickCount()` | — | — |
| 2 | `iniPath = GetExeDir() + L"\\config.ini"` | — | — |
| 3 | `Config cfg; LoadConfig(cfg, iniPath)` | — | — |
| 4 | `Logger logger(cfg.enableLog, cfg.logFile); logger.initDir()` | — | — |
| 5 | — | `========== 程序启动 ==========` | — |
| 6 | — | `配置文件: %ls` | — |
| 7 | — | `PowerPoint: %ls` | — |
| 8 | — | `WPS: %ls` | — |
| 9 | — | `日志文件: %ls` | — |
| 10 | — | `兜底策略: %s`（`fallback==0` → `WPS`，否则 `PowerPoint`） | — |
| 11 | `CommandLineToArgvW(GetCommandLineW(), &argc)`；`!argvW \|\| argc < 2` | `错误: 命令行参数不足 (argc=%d)` | **1** |
| 12 | — | `目标文件: %ls`（`argvW[1]`，全程保留 Unicode） | — |
| 13 | `pptExists = appExists(cfg.pathPowerPoint)` | `PowerPoint存在: 是\|否` | — |
| 14 | `wpsExists = appExists(cfg.pathWPS)` | `WPS存在: 是\|否` | — |
| 15 | `GetFileAttributesW(argvW[1]) == INVALID_FILE_ATTRIBUTES` → `launchFallback` | `错误: 目标文件不存在，兜底启动` | **2** |
| 16 | `readFileToMemory`；返回空 → `launchFallback` | 内部错误日志 + `错误: 读取文件失败，兜底启动` | **3** |
| 17 | — | `文件已读入内存 (%zu 字节)，耗时 %lu ms` → **Rust 改为 `文件大小 ({n} 字节)，耗时 {ms} ms`**（见 D1） | — |
| 18 | `detectPptxCreator(fileData.data(), fileData.size())` | — | — |
| 19 | — | `检测到 Application: %s`（空则打印 `(空)`） | — |
| 20 | `chooseLauncher(...)`；有 → `launchApp` | 第 6.2 节日志；成功后 `启动进程创建成功`；失败后 `错误: 启动失败 (Win32=%lu)，尝试兜底` | — |
| 21 | 无启动器 | `未找到可用的首选启动器，尝试兜底` | — |
| 22 | `!launched` → `launchFallback` | 第 6.3 节日志 | — |
| 23 | `LocalFree(argvW)` | — | — |
| 24 | — | `总耗时: %lu ms` | — |
| 25 | — | `========== 程序结束 ==========` | — |
| 26 | `return launched ? 0 : 5` | — | **0 / 5** |

**设计意图（AGENTS.md 明确要求保留）**：程序"尽量打开文件"，多数失败路径兜底启动而非报错退出。

**子系统**：`-mwindows` → 无控制台窗口。程序**从不**弹 MessageBox、不调用 CMD/PowerShell、不写 stdout/stderr。

---

## 7. 日志文本总表（逐字符）

| 序号 | 文本 | 触发点 |
|---|---|---|
| L1 | `========== 程序启动 ==========` | 启动 |
| L2 | `配置文件: {iniPath}` | 启动 |
| L3 | `PowerPoint: {path}` | 启动 |
| L4 | `WPS: {path}` | 启动 |
| L5 | `日志文件: {path}` | 启动 |
| L6 | `兜底策略: WPS` / `兜底策略: PowerPoint` | 启动 |
| L7 | `错误: 命令行参数不足 (argc={n})` | 参数不足 |
| L8 | `目标文件: {path}` | 取到参数后 |
| L9 | `PowerPoint存在: 是` / `否` | 可用性检查 |
| L10 | `WPS存在: 是` / `否` | 可用性检查 |
| L11 | `错误: 目标文件不存在，兜底启动` | 文件不存在 |
| L12 | `错误: 无法打开文件 (CreateFileW 失败)` | 打开失败 |
| L13 | `错误: 读取文件失败 (读取 {n} / 期望 {m} 字节)` | 读失败 |
| L14 | `错误: 读取文件失败，兜底启动` | 读失败后 |
| L15 | `文件已读入内存 ({n} 字节)，耗时 {ms} ms` | 读取完成 — **Rust 侧已批准改为 `文件大小 ({n} 字节)，耗时 {ms} ms`** |
| L16 | `检测到 Application: {app}` / `(空)` | 检测后 |
| L17 | `决策: 使用 WPS 打开` | 决策 |
| L18 | `WPS 不存在，降级为 PowerPoint 打开` | 决策 |
| L19 | `决策: 使用 PowerPoint 打开` | 决策 |
| L20 | `PowerPoint 不存在，降级为 WPS 打开` | 决策 |
| L21 | `启动进程创建成功` | 启动成功 |
| L22 | `错误: 启动失败 (Win32={code})，尝试兜底` | 启动失败 |
| L23 | `未找到可用的首选启动器，尝试兜底` | 无启动器 |
| L24 | `兜底: 使用 WPS 打开` / `兜底: 使用 PowerPoint 打开` | 兜底 |
| L25 | `兜底启动成功` | 兜底成功 |
| L26 | `错误: 兜底启动失败 (Win32={code})` | 兜底失败 |
| L27 | `总耗时: {ms} ms` | 收尾 |
| L28 | `========== 程序结束 ==========` | 收尾 |

行格式（`writeUtf8`）：`[YYYY-MM-DD HH:MM:SS] ` + 正文 + `\r\n`；文件首次创建时前置 UTF-8 BOM。

**读取失败分支的日志组合（Rust 侧逐条对齐）：**

| 场景 | C++ 日志序列 | Rust 日志序列 | 退出码 |
|---|---|---|---|
| `CreateFileW` 打开失败 | L12（`无法打开文件`）→ L14（`读取文件失败，兜底启动`） | 同 | 3 |
| 部分读入（`bytesRead != size`） | L13（`读取文件失败 (读取 N / 期望 M 字节)`）→ L14 | **不可达**（见 D14） | 3 |
| 0 字节文件 / 目录 | L14（C++ 下 `ReadFile` 成功但返回空 vector） | 同 | 3 |
| ZIP 解析失败 / 缺 app.xml / CRC 错 | 无额外日志，`Application` 打印 `(空)` | 同 | 0 或 5 |

---

## 8. `config.ini` 格式（不得修改）

```ini
[Paths]
PowerPoint=...      ; 键名区分大小写不敏感；值不做引号/转义处理（除 Win32 的引号剥离语义）
WPS=...
LogFile=...

[Options]
EnableLog=1         ; 默认 1
Fallback=0          ; 默认 0，0=WPS 1=PowerPoint
```

`;` 起始的行为注释。节名 `Paths` / `Options`，键名 `PowerPoint` / `WPS` / `LogFile` / `EnableLog` / `Fallback`。

---

## 9. 必须保持兼容的特性清单

| # | 特性 | 依据 |
|---|---|---|
| 1 | `config.ini` 格式与键名、节名、默认值 | §2.2、§8 |
| 2 | PowerPoint / WPS 判断逻辑与优先级 | §4.3 |
| 3 | Fallback 机制（双向降级 + 兜底开关） | §6.2、§6.3 |
| 4 | 日志机制（开关、位置、格式、UTF-8+BOM、CRLF） | §3 |
| 5 | 错误处理与退出码 | §1、§6.4 |
| 6 | 启动参数语义（`argv[1]` 为目标文件） | §6.4 |
| 7 | Windows 无控制台窗口 | `-mwindows` |
| 8 | Unicode（中文、空格）路径处理 | `GetPrivateProfileStringW`/`CreateFileW`/`CreateProcessW` 全宽字符链 |

---

## 10. Rust 侧差异清单（有意为之，逐条给出理由）

> 下表为**实现完成后的最终状态**，各项均已逐条复核。

| # | 差异 | 理由 | 是否影响可观察行为 |
|---|---|---|---|
| D1 | 日志 L15 措辞由 `文件已读入内存 (N 字节)` 改为 `文件大小 (N 字节)` | 用户已批准：流式实现下文件不再整体入内存，原文与事实不符；N 的取值口径（文件字节数）不变 | 仅该行文本 |
| D2 | 依赖白名单增补 `miniz_oxide` | `rawzip` 不含解压能力（已实测确认），真实 PPTX 的 app.xml 为 DEFLATE | 无 |
| D3 | `extractTag` 的越界读取 UB 不再复现，改为有界解析 | C++ 对非 NUL 结尾缓冲区 `strstr` 属未定义行为 | 无（无匹配时两者同为 Unknown） |
| D4 | 新增 4 MiB app.xml 读取/解压上限 | C++ 无上限，存在解压炸弹风险；超限 → `Err` → Unknown → 兜底 | 仅病态文件 |
| D5 | XML 由字面字节扫描改为 `quick-xml` 事件流解析 | 说明书第 2 节硬性要求 | 见 D5a/D5b |
| D5a | `<x:Application>` / `<Application xml:space=…>` 在 Rust 下可匹配，C++ 字面匹配不会 | quick-xml 按 XML 语义解析元素名 | 真实 PPTX 的 app.xml 为标准 `<Application>…</Application>`，结果一致 |
| D5b | **实体引用与 CDATA 已按文档原文逐字节还原**：`Event::GeneralRef` 补回 `&…;`、`Event::CData` 补回 `<![CDATA[…]]>`，因此 `&#174;`、`&amp;` 等与原项目裸字节截取结果**完全一致**（已用单测覆盖）。**残余差异**：`<Application>` 内部的**子元素标记**（如 `<a>…</a>`）会被丢弃，C++ 会原样保留 | 取 `BytesText::into_inner()` / `BytesRef::into_inner()` 的原始文本并补回定界符 | 真实 PPTX 的 `<Application>` 只含纯文本，结果一致 |
| D6 | 畸形 XML（标签不匹配等）：Rust 报错 → Unknown；C++ 的裸字节扫描仍可能截取到内容。未闭合的 `<Application>`：Rust 同样只得到 Unknown | 说明书要求走 quick-xml | 仅畸形文件 |
| D7 | `detect_pptx_creator_from_file` 返回 `Result`（ZIP/XML/校验失败为 `Err`），`main` 用 `unwrap_or_default()` 把 `Err` 折叠为 `Unknown` 后继续走原逻辑，**不额外写日志** | 说明书第 7 节要求错误经 `Result` 传播；"或按原逻辑处理" | 无（退出码与日志完全一致） |
| D8 | 不使用 `CREATE_NO_WINDOW`，保留原 `CREATE_NEW_PROCESS_GROUP` | 原 C++ 源码中无 `CREATE_NO_WINDOW`；说明书要求"以原项目源码为准" | 无 |
| D9 | 非 Windows 平台的分隔符/时间戳（UTC）/进程启动为测试 shim | `cargo test` 需覆盖平台无关逻辑；Windows target 走真实 Win32 API | 无（Windows target 行为逐字符一致） |
| D10 | 0 字节文件在 Rust 侧显式判为"读取失败"（退出码 3） | 对齐 C++ `readFileToMemory` 返回空 vector 的判定 | 无 |
| D11 | 目录作为目标文件在 Rust 侧显式判为"读取失败"（退出码 3） | 对齐 C++ `CreateFileW` 打开目录失败 | 无（Windows 语义一致） |
| D12 | 日志文件读写用 `std::fs::OpenOptions`（append+create；Windows 上即 `FILE_APPEND_DATA` + `OPEN_ALWAYS`）替代裸 `CreateFileW`；共享模式多出 `FILE_SHARE_DELETE` | Rust 惯用写法，落盘字节完全一致；减少平台分支 | 无 |
| D13 | 检测模块只暴露 `detect_pptx_creator_from_file(File)`；**没有**按路径的入口 | 主流程必须自行打开文件（判定读失败 + 取文件大小），把句柄交给检测模块可避免重复打开与 TOCTOU | 无 |
| D14 | 日志 L13 `错误: 读取文件失败 (读取 N / 期望 M 字节)` 在流式实现中**不可达** | 该分支对应"部分读入"，流式实现不再整体读取，无法复现；真实的打开失败（L12+L14）与 0 字节/目录（L14）均已保留 | 仅极端 I/O 错误场景 |

---

## 11. 存疑点（无法从源码确定的项）

| # | 疑点 | 处理方式 |
|---|---|---|
| U1 | 本机无 Windows，**所有行为对齐均基于源码审读**，无法运行 C++ 原版做二进制级对比 | 在最终报告中明确声明；Wine 仅验证 Rust 侧 |
| U2 | `GetPrivateProfileStringW` / `GetPrivateProfileIntW` 在非数字取值时的确切返回 | Windows 上调用真实 API（无需猜测）；非 Windows shim 取"键存在但无数字 → 0" |
| U3 | `GetPrivateProfileIntW` 对重复键取第一次还是最后一次出现 | 未定义行为；测试 fixture 不构造重复键 |
| U4 | 说明书第 8 节提到 `CREATE_NO_WINDOW`，但原 C++ 源码未使用 | 以源码为准（见 D8） |
| U5 | `SHCreateDirectoryExW` 在部分路径（UNC、超长路径）下的行为 | 忽略返回值，与 C++ 一致 |
| U6 | Wine 与真实 Windows 的差异（尤其文件系统重定向、环境块） | Wine 验证仅作为集成证据；报告中明确标注"未在真实 Windows 上验证" |


---

## 12. C++ → Rust 对照表（供逐函数复核）

| C++ 符号 | 文件 | Rust 符号 | 文件 |
|---|---|---|---|
| `enum class Err` | `error_codes.h` | `ErrorCode`（数值 1:1） | `src/error.rs` |
| `Config::Config()` | `config.cpp` | `Config::default()` | `src/config.rs` |
| `LoadConfig()` | `config.cpp` | `config::load_config()` | `src/config.rs` |
| `GetExeDir()` | `config.cpp` | `config::get_exe_dir()` | `src/config.rs` |
| `Logger::Logger(bool, const wchar_t*)` | `logger.cpp` | `Logger::new()` | `src/logger.rs` |
| `Logger::initDir()` | `logger.cpp` | `Logger::init_dir()` | `src/logger.rs` |
| `Logger::write()` / `writeW()` / `writeUtf8()` | `logger.cpp` | `Logger::write(&str)` | `src/logger.rs` |
| `_snprintf_s("[%04d-...] ")` | `logger.cpp` | `format_prefix()` | `src/logger.rs` |
| `detectPptxCreator()` | `pptx_detector.cpp` | `detect_pptx_creator_from_file()` | `src/detector.rs` |
| `mz_zip_reader_locate_file(..., 0)` | miniz | `entry.file_path().eq_ignore_ascii_case(b"docProps/app.xml")` 线性扫描 | `src/detector.rs` |
| `mz_zip_reader_extract_to_heap(..., 0)` | miniz | `ZipEntry::reader()` + `miniz_oxide::inflate` | `src/detector.rs` |
| `extractTag()` | `pptx_detector.cpp` | `extract_application()` | `src/detector.rs` |
| `classifyApp()` | `pptx_detector.cpp` | `classify_app()` | `src/detector.rs` |
| `ZipGuard`（RAII） | `pptx_detector.cpp` | `ZipArchive` / `File` 的所有权语义 | — |
| `appExists()` | `app_launcher.cpp` | `launcher::app_exists()` | `src/launcher.rs` |
| `quoteCommandLineArg()` | `app_launcher.cpp` | `launcher::quote_command_line_arg()` | `src/launcher.rs` |
| `launchApp()` | `app_launcher.cpp` | `launcher::launch_app()` | `src/launcher.rs` |
| `readFileToMemory()` | `main.cpp` | `open_target_file()` + `metadata().len()`（**不读取内容**） | `src/main.rs` |
| `chooseLauncher()` | `main.cpp` | `choose_launcher()` | `src/main.rs` |
| `launchFallback()` | `main.cpp` | `launch_fallback()` | `src/main.rs` |
| `WinMain()` | `main.cpp` | `run()` | `src/main.rs` |
| `CommandLineToArgvW` 解析 | `main.cpp` | `command_line_args()` | `src/main.rs` |
| `GetTickCount()` | Win32 | `tick_count()`（含 `wrapping_sub` 回绕语义） | `src/main.rs` |
| `resource.rc` + `windres` | 构建脚本 | `build.rs` + `winresource` | `build.rs` |
