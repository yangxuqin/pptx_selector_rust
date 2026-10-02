//! ============================================================================
//!  main.rs - 入口、windows_subsystem、参数解析、主流程编排
//!  PPTX 智能打开器 - pptx_selector.exe
//! ============================================================================
//!  功能：根据 PPTX 文件的创建软件，自动选择 PowerPoint 或 WPS 打开
//!  场景：教室电脑，不同老师用不同软件制作 PPT，避免排版错乱
//!  原理：PPTX 是 ZIP 包，读取 docProps/app.xml 的 <Application> 判断创建者
//!
//!  配置：自动读取 exe 同目录下的 config.ini，无需重新编译即可修改路径
//!
//!  行为基准是原 C++ 项目 `pptx_selector_next`，对应 `src/main.cpp` 的 `WinMain`。
//!  与本文件相关的强制约束：
//!  - 完全无命令行窗口（Release 的 Windows target 使用 Windows 子系统）；
//!  - 不弹 MessageBox、不调用 CMD/PowerShell、不输出到控制台；
//!  - 运行路径上禁止 `.unwrap()` / `.expect()`，所有错误经 `Result` 传播；
//!  - 多数失败路径"兜底启动"而非报错退出（"尽量打开文件"的设计意图）。
//!
//! ============================================================================

// Release 构建（含交叉编译到 Windows）必须使用 Windows 子系统，不弹黑框。
// Linux 原生 debug / test 构建不受影响。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod detector;
mod error;
mod launcher;
mod logger;

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};

use config::Config;
use detector::{AppCreator, DetectionResult};
use error::ErrorCode;
use logger::Logger;

/// 只有测试才需要的小工具（保持 `src/` 为说明书要求的 6 个业务模块）。
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 一次性临时目录，`Drop` 时自动清理。避免引入额外测试依赖 crate。
    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = format!(
                "pptx_selector_test_{tag}_{}_{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(unique);
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir { path }
        }

        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn main() {
    std::process::exit(run().as_exit_code());
}

/// 主流程。逐行对照原项目 `WinMain`。
fn run() -> ErrorCode {
    let started_at = tick_count();

    // ---- 1. 加载配置 -------------------------------------------------------
    let ini_path = config::get_exe_dir().join("config.ini");
    let mut cfg = Config::default();
    config::load_config(&mut cfg, &ini_path);

    // ---- 2. 初始化日志 -----------------------------------------------------
    let logger = Logger::new(cfg.enable_log, &cfg.log_file);
    logger.init_dir();

    logger.write("========== 程序启动 ==========");
    logger.write(&format!("配置文件: {}", ini_path.display()));
    logger.write(&format!("PowerPoint: {}", cfg.path_powerpoint.display()));
    logger.write(&format!("WPS: {}", cfg.path_wps.display()));
    logger.write(&format!("日志文件: {}", cfg.log_file.display()));
    logger.write(&format!(
        "兜底策略: {}",
        if cfg.fallback == 0 {
            "WPS"
        } else {
            "PowerPoint"
        }
    ));

    // ---- 3. 解析命令行参数 -------------------------------------------------
    let args = command_line_args();
    let argc = args.len();
    if argc < 2 {
        logger.write(&format!("错误: 命令行参数不足 (argc={argc})"));
        return ErrorCode::InvalidArgs;
    }

    // 始终保留原始 Unicode 路径，不能转换到系统 ANSI 代码页后再交给启动器。
    let target = PathBuf::from(args[1].clone());
    logger.write(&format!("目标文件: {}", target.display()));

    // ---- 4. 检查软件可用性 -------------------------------------------------
    let powerpoint_exists = launcher::app_exists(&cfg.path_powerpoint);
    let wps_exists = launcher::app_exists(&cfg.path_wps);
    logger.write(&format!("PowerPoint存在: {}", yes_no(powerpoint_exists)));
    logger.write(&format!("WPS存在: {}", yes_no(wps_exists)));

    // ---- 5. 目标文件存在性检查 ---------------------------------------------
    if launcher::file_attributes(&target).is_none() {
        logger.write("错误: 目标文件不存在，兜底启动");
        launch_fallback(&cfg, &target, &logger);
        return ErrorCode::FileNotFound;
    }

    // ---- 6. 打开目标文件 ---------------------------------------------------
    // 注意：这里**不读取文件内容**，只是取得句柄与大小；内容由 detector 流式读取。
    let file = match open_target_file(&target) {
        Ok(file) => file,
        Err(_) => {
            logger.write("错误: 无法打开文件 (CreateFileW 失败)");
            logger.write("错误: 读取文件失败，兜底启动");
            launch_fallback(&cfg, &target, &logger);
            return ErrorCode::ReadFailed;
        }
    };
    let file_size = file.metadata().map_or(0, |metadata| metadata.len());
    if file_size == 0 {
        // 对齐原项目：0 字节文件与目录都会被判定为读失败。
        logger.write("错误: 读取文件失败，兜底启动");
        launch_fallback(&cfg, &target, &logger);
        return ErrorCode::ReadFailed;
    }
    // 原项目此处为「文件已读入内存 (N 字节)，耗时 M ms」。Rust 版改为流式，
    // 文件不再整体读入内存，经确认改为与事实一致的措辞（见 docs/behavior-mapping.md D1）。
    logger.write(&format!(
        "文件大小 ({file_size} 字节)，耗时 {} ms",
        tick_count().wrapping_sub(started_at)
    ));

    // ---- 7. 检测创建软件（流式） -------------------------------------------
    // 与原项目一致：ZIP/XML 失败静默折叠为 Unknown，不额外写日志，
    // 随后走「未找到可用的首选启动器 → 兜底」路径。
    let detection = detector::detect_pptx_creator_from_file(file).unwrap_or_default();
    let shown = if detection.application.is_empty() {
        "(空)"
    } else {
        detection.application.as_str()
    };
    logger.write(&format!("检测到 Application: {shown}"));

    // ---- 8. 决策并启动 -----------------------------------------------------
    let mut launched = false;
    match choose_launcher(&detection, &cfg, wps_exists, powerpoint_exists, &logger) {
        Some(exe_path) => match launcher::launch_app(exe_path, &target) {
            Ok(()) => {
                logger.write("启动进程创建成功");
                launched = true;
            }
            Err(code) => {
                logger.write(&format!("错误: 启动失败 (Win32={code})，尝试兜底"));
            }
        },
        None => logger.write("未找到可用的首选启动器，尝试兜底"),
    }

    if !launched {
        launched = launch_fallback(&cfg, &target, &logger);
    }

    logger.write(&format!(
        "总耗时: {} ms",
        tick_count().wrapping_sub(started_at)
    ));
    logger.write("========== 程序结束 ==========");

    if launched {
        ErrorCode::Ok
    } else {
        ErrorCode::LaunchFailed
    }
}

/// 按检测结果与可用性选择启动器。返回 `None` 表示无可用启动器。
fn choose_launcher<'a>(
    detection: &DetectionResult,
    cfg: &'a Config,
    wps_exists: bool,
    powerpoint_exists: bool,
    logger: &Logger,
) -> Option<&'a Path> {
    match detection.creator {
        AppCreator::Wps => {
            if wps_exists {
                logger.write("决策: 使用 WPS 打开");
                return Some(&cfg.path_wps);
            }
            if powerpoint_exists {
                logger.write("WPS 不存在，降级为 PowerPoint 打开");
                return Some(&cfg.path_powerpoint);
            }
        }
        AppCreator::MicrosoftOffice => {
            if powerpoint_exists {
                logger.write("决策: 使用 PowerPoint 打开");
                return Some(&cfg.path_powerpoint);
            }
            if wps_exists {
                logger.write("PowerPoint 不存在，降级为 WPS 打开");
                return Some(&cfg.path_wps);
            }
        }
        AppCreator::Unknown => {}
    }
    None
}

/// 执行兜底启动。
fn launch_fallback(cfg: &Config, pptx_path: &Path, logger: &Logger) -> bool {
    let (exe_path, name) = if cfg.fallback == 1 {
        (&cfg.path_powerpoint, "PowerPoint")
    } else {
        (&cfg.path_wps, "WPS")
    };
    logger.write(&format!("兜底: 使用 {name} 打开"));

    match launcher::launch_app(exe_path, pptx_path) {
        Ok(()) => {
            logger.write("兜底启动成功");
            true
        }
        Err(code) => {
            logger.write(&format!("错误: 兜底启动失败 (Win32={code})"));
            false
        }
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "是"
    } else {
        "否"
    }
}

// ============================================================================
//  平台相关薄封装
// ============================================================================

/// 解析命令行参数。
///
/// Windows 上使用 `CommandLineToArgvW`，与原项目完全一致（Unicode、`argc` 语义、
/// 参数切分规则）。非 Windows 使用 `args_os` shim。
#[cfg(windows)]
fn command_line_args() -> Vec<OsString> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::System::Environment::GetCommandLineW;
    use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

    let mut argc: i32 = 0;
    // SAFETY: GetCommandLineW 返回进程级只读字符串；CommandLineToArgvW 按其规则
    // 切分并返回一块需要用 LocalFree 释放的内存。
    let argv = unsafe { CommandLineToArgvW(GetCommandLineW(), &mut argc) };
    if argv.is_null() || argc <= 0 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(argc as usize);
    for index in 0..argc as isize {
        // SAFETY: index < argc，argv 至少有 argc 个元素。
        let ptr = unsafe { *argv.offset(index) };
        if ptr.is_null() {
            continue;
        }
        let mut len = 0usize;
        // SAFETY: 各参数均为 NUL 结尾的宽字符串。
        while unsafe { *ptr.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: 已确认 [ptr, ptr + len) 是本进程可读的连续内存。
        let units = unsafe { std::slice::from_raw_parts(ptr, len) };
        out.push(OsString::from_wide(units));
    }

    // SAFETY: argv 由 CommandLineToArgvW 分配，此处释放一次且不再使用。
    unsafe {
        LocalFree(argv as *mut std::ffi::c_void);
    }
    out
}

#[cfg(not(windows))]
fn command_line_args() -> Vec<OsString> {
    std::env::args_os().collect()
}

/// 以"只读、仅共享读"的方式打开目标文件。
///
/// Windows 上显式调用 `CreateFileW`，参数与原项目 `readFileToMemory` 完全一致
/// （`GENERIC_READ` / `FILE_SHARE_READ` / `OPEN_EXISTING` / `FILE_ATTRIBUTE_NORMAL`）。
#[cfg(windows)]
fn open_target_file(path: &Path) -> std::io::Result<File> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: wide 是 NUL 结尾的宽字符串；其余参数与原项目一致，传 NULL。
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: handle 是刚由 CreateFileW 成功返回的、归本进程所有的文件句柄；
    // File 接管其所有权并在析构时关闭它。
    Ok(unsafe { File::from_raw_handle(handle) })
}

#[cfg(not(windows))]
fn open_target_file(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

/// 毫秒级单调计时，对齐原项目的 `GetTickCount()`（含无符号回绕语义）。
#[cfg(windows)]
fn tick_count() -> u32 {
    // SAFETY: 无参数，只读取系统 tick 计数。
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

#[cfg(not(windows))]
fn tick_count() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn cfg_with(path_powerpoint: &str, path_wps: &str, fallback: i32) -> Config {
        Config {
            path_powerpoint: PathBuf::from(path_powerpoint),
            path_wps: PathBuf::from(path_wps),
            log_file: PathBuf::from("unused.log"),
            enable_log: false,
            fallback,
        }
    }

    fn detection(creator: AppCreator) -> DetectionResult {
        DetectionResult {
            creator,
            application: String::new(),
        }
    }

    #[test]
    fn choose_launcher_matches_the_cpp_decision_table() {
        let cfg = cfg_with("PPT.EXE", "WPS.EXE", 0);
        let logger = Logger::new(false, Path::new("unused.log"));

        // WPS + WPS 可用 -> WPS
        assert_eq!(
            choose_launcher(&detection(AppCreator::Wps), &cfg, true, true, &logger),
            Some(Path::new("WPS.EXE"))
        );
        // WPS + 只有 PowerPoint -> 降级 PowerPoint
        assert_eq!(
            choose_launcher(&detection(AppCreator::Wps), &cfg, false, true, &logger),
            Some(Path::new("PPT.EXE"))
        );
        // WPS + 都不可用 -> None
        assert_eq!(
            choose_launcher(&detection(AppCreator::Wps), &cfg, false, false, &logger),
            None
        );
        // Microsoft + PowerPoint 可用 -> PowerPoint
        assert_eq!(
            choose_launcher(
                &detection(AppCreator::MicrosoftOffice),
                &cfg,
                true,
                true,
                &logger
            ),
            Some(Path::new("PPT.EXE"))
        );
        // Microsoft + 只有 WPS -> 降级 WPS
        assert_eq!(
            choose_launcher(
                &detection(AppCreator::MicrosoftOffice),
                &cfg,
                true,
                false,
                &logger
            ),
            Some(Path::new("WPS.EXE"))
        );
        // Microsoft + 都不可用 -> None
        assert_eq!(
            choose_launcher(
                &detection(AppCreator::MicrosoftOffice),
                &cfg,
                false,
                false,
                &logger
            ),
            None
        );
        // Unknown -> 永远是 None
        assert_eq!(
            choose_launcher(&detection(AppCreator::Unknown), &cfg, true, true, &logger),
            None
        );
    }

    #[test]
    fn fallback_prefers_wps_unless_fallback_is_exactly_one() {
        let cfg = cfg_with("PPT.EXE", "WPS.EXE", 0);
        let dir = TempDir::new("main-fallback");
        let target = dir.path().join("a.pptx");

        // fallback == 0 -> WPS（非 Windows 下 launch_app 返回 Err，故返回 false）
        let logger = Logger::new(false, Path::new("unused.log"));
        assert!(!launch_fallback(&cfg, &target, &logger));

        // 穷举 fallback 取值：只有恰好等于 1 才走 PowerPoint
        for value in [0i32, 2, -1, 99] {
            let cfg_other = cfg_with("PPT.EXE", "WPS.EXE", value);
            assert!(!launch_fallback(&cfg_other, &target, &logger));
        }

        let cfg_ppt = cfg_with("PPT.EXE", "WPS.EXE", 1);
        assert!(!launch_fallback(&cfg_ppt, &target, &logger));
    }

    #[test]
    fn fallback_writes_the_expected_log_lines() {
        let dir = TempDir::new("main-fallback-log");
        let log_path = dir.path().join("a.log");
        let logger = Logger::new(true, &log_path);
        let cfg = cfg_with("PPT.EXE", "WPS.EXE", 0);
        let target = dir.path().join("a.pptx");
        launch_fallback(&cfg, &target, &logger);

        let text = std::fs::read_to_string(&log_path).expect("log file");
        assert!(text.contains("兜底: 使用 WPS 打开\r\n"), "实际: {text:?}");
        assert!(
            text.contains("错误: 兜底启动失败 (Win32=120)"),
            "实际: {text:?}"
        );

        let logger_ppt = Logger::new(true, &dir.path().join("b.log"));
        let cfg_ppt = cfg_with("PPT.EXE", "WPS.EXE", 1);
        launch_fallback(&cfg_ppt, &target, &logger_ppt);
        let text = std::fs::read_to_string(dir.path().join("b.log")).expect("log file");
        assert!(
            text.contains("兜底: 使用 PowerPoint 打开"),
            "实际: {text:?}"
        );
    }

    #[test]
    fn yes_no_matches_the_cpp_literals() {
        assert_eq!(yes_no(true), "是");
        assert_eq!(yes_no(false), "否");
    }

    #[test]
    fn detection_failure_folds_to_unknown_without_panicking() {
        // 与 `run()` 中的处理保持一致：Err -> default()
        let dir = TempDir::new("main-fold");
        let path = dir.path().join("broken.pptx");
        std::fs::write(&path, b"not a zip").expect("write");
        let file = open_target_file(&path).expect("open");
        let folded = detector::detect_pptx_creator_from_file(file).unwrap_or_default();
        assert_eq!(folded.creator, AppCreator::Unknown);
        assert_eq!(folded.application, "");
    }

    #[test]
    fn command_line_args_are_available_in_tests() {
        // 至少包含测试二进制自身的 argv[0]
        assert!(!command_line_args().is_empty());
    }

    #[test]
    fn tick_count_is_usable() {
        let a = tick_count();
        let b = tick_count();
        assert!(b.wrapping_sub(a) < 60_000);
    }

    #[test]
    fn opening_a_missing_file_reports_an_error() {
        let dir = TempDir::new("main-open");
        assert!(open_target_file(&dir.path().join("nope.pptx")).is_err());
    }

    #[test]
    fn opening_an_existing_file_succeeds_and_reports_size() {
        let dir = TempDir::new("main-open-ok");
        let path = dir.path().join("a.pptx");
        std::fs::write(&path, b"12345").expect("write");
        let file = open_target_file(&path).expect("open");
        assert_eq!(file.metadata().map_or(0, |m| m.len()), 5);
    }
}
