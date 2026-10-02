//! ============================================================================
//!  launcher.rs - 外部程序启动
//!  PPTX 智能打开器
//! ============================================================================
//!  封装 `CreateProcessW`，负责以指定 exe 打开 PPTX 文件。
//!  对应原项目 `src/app_launcher.h` / `src/app_launcher.cpp`。
//!
//!  为什么不用 `std::process::Command`：`CreateProcessW` 可以精确控制
//!  `STARTUPINFOW`、`CREATE_NEW_PROCESS_GROUP`、命令行拼接方式与窗口参数，
//!  与原 C++ 行为完全一致；`Command` 的抽象层隐藏了部分 Win32 细节。
//! ============================================================================

use std::path::Path;

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;

/// 非 Windows 下的常量镜像（仅用于测试 shim）。
#[cfg(not(windows))]
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// 非 Windows 下的常量镜像（仅用于测试 shim）。
#[cfg(not(windows))]
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

/// 检查指定路径的可执行文件是否存在。目录不算存在。
pub fn app_exists(exe_path: &Path) -> bool {
    if exe_path.as_os_str().is_empty() {
        return false;
    }
    match file_attributes(exe_path) {
        Some(attributes) => attributes & FILE_ATTRIBUTE_DIRECTORY == 0,
        None => false,
    }
}

/// 读取文件属性；不存在时返回 `None`。
#[cfg(windows)]
pub fn file_attributes(path: &Path) -> Option<u32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: wide 是 NUL 结尾的宽字符串，调用只读取它。
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        None
    } else {
        Some(attributes)
    }
}

/// 非 Windows 下的等价实现（测试 shim）。
#[cfg(not(windows))]
pub fn file_attributes(path: &Path) -> Option<u32> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(if metadata.is_dir() {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    })
}

/// 按 Windows 命令行规则为一个参数加引号，避免空格、中文或特殊字符被目标
/// 程序错误拆分。算法与原项目 `quoteCommandLineArg` **逐行等价**。
///
/// 入参/出参均为 UTF-16 码元序列（Windows 原生形态）。算法只把 `\` 与 `"`
/// 视作特殊字符，二者都是 ASCII，因此在任意码元序列上行为一致，这也让本函数
/// 可以在非 Windows 平台直接用 `str::encode_utf16()` 做单元测试。
///
/// 非 Windows 下只有单元测试会调用它，故显式放行 dead_code。
#[cfg_attr(not(windows), allow(dead_code))]
pub fn quote_command_line_arg(arg: &[u16]) -> Vec<u16> {
    const BACKSLASH: u16 = b'\\' as u16;
    const QUOTE: u16 = b'"' as u16;

    let mut quoted: Vec<u16> = Vec::with_capacity(arg.len() + 8);
    quoted.push(QUOTE);

    let mut backslashes: usize = 0;
    for &unit in arg {
        if unit == BACKSLASH {
            backslashes += 1;
            continue;
        }
        if unit == QUOTE {
            quoted.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2 + 1));
            quoted.push(QUOTE);
            backslashes = 0;
            continue;
        }
        quoted.extend(std::iter::repeat_n(BACKSLASH, backslashes));
        backslashes = 0;
        quoted.push(unit);
    }
    // 引号结尾前的反斜杠必须翻倍。
    quoted.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
    quoted.push(QUOTE);
    quoted
}

/// 以 `exe_path` 打开 `file_path`。成功表示**进程创建成功**（不等待、不检查退出码）。
///
/// 失败时返回 `GetLastError()` 的错误码，供调用方写日志。
#[cfg(windows)]
pub fn launch_app(exe_path: &Path, file_path: &Path) -> Result<(), u32> {
    use std::mem::zeroed;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER};
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, CREATE_NEW_PROCESS_GROUP, PROCESS_INFORMATION, STARTUPINFOW,
    };

    if exe_path.as_os_str().is_empty() || file_path.as_os_str().is_empty() {
        return Err(ERROR_INVALID_PARAMETER);
    }

    let exe_units: Vec<u16> = exe_path.as_os_str().encode_wide().collect();
    let file_units: Vec<u16> = file_path.as_os_str().encode_wide().collect();

    // 与原项目一致：引号 + 空格 + 引号，并为 CreateProcessW 补 NUL 终止符。
    let mut command_line = quote_command_line_arg(&exe_units);
    command_line.push(b' ' as u16);
    command_line.extend(quote_command_line_arg(&file_units));
    command_line.push(0);

    let mut exe_wide: Vec<u16> = exe_units;
    exe_wide.push(0);

    // SAFETY: STARTUPINFOW / PROCESS_INFORMATION 是全 POD 结构，全零初始化合法；
    // 随后只设置 cb 字段，与 C++ 的 `STARTUPINFOW startup = {}; startup.cb = sizeof(startup);` 一致。
    let (startup, mut process) = unsafe {
        let mut startup: STARTUPINFOW = zeroed();
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        (startup, zeroed::<PROCESS_INFORMATION>())
    };

    // SAFETY: 两个字符串缓冲区都以 NUL 结尾且在本调用期间保持有效；
    // command_line 是可写缓冲区（CreateProcessW 允许就地修改）；
    // 其余指针按原项目传 NULL；进程句柄在成功后立即释放。
    let created = unsafe {
        CreateProcessW(
            exe_wide.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NEW_PROCESS_GROUP,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut process,
        )
    };

    if created == 0 {
        // SAFETY: 无参数，只读取当前线程的 last-error。
        return Err(unsafe { GetLastError() });
    }

    // SAFETY: 两个句柄由 CreateProcessW 成功创建，归本进程所有。
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

/// 非 Windows 下的占位实现：不启动任何进程，直接返回错误。
///
/// Linux 侧只用于保证可编译、可测试，不参与 Windows 行为对齐。
#[cfg(not(windows))]
pub fn launch_app(_exe_path: &Path, _file_path: &Path) -> Result<(), u32> {
    /// `ERROR_CALL_NOT_IMPLEMENTED`
    const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;
    Err(ERROR_CALL_NOT_IMPLEMENTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote_str(text: &str) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        String::from_utf16(&quote_command_line_arg(&units)).expect("valid utf-16 output")
    }

    #[test]
    fn quotes_a_plain_path() {
        assert_eq!(quote_str(r"C:\a\b.exe"), r#""C:\a\b.exe""#);
    }

    #[test]
    fn quotes_a_path_with_spaces() {
        assert_eq!(
            quote_str(r"C:\Program Files\x.exe"),
            r#""C:\Program Files\x.exe""#
        );
    }

    #[test]
    fn doubles_backslashes_before_the_closing_quote() {
        assert_eq!(quote_str(r"C:\dir\"), "\"C:\\dir\\\\\"");
    }

    #[test]
    fn escapes_embedded_quotes() {
        assert_eq!(quote_str(r#"a"b"#), "\"a\\\"b\"");
    }

    #[test]
    fn escapes_quote_after_backslashes() {
        // `a\"b` -> 反斜杠翻倍后再加一个转义用的反斜杠
        assert_eq!(quote_str("a\\\"b"), "\"a\\\\\\\"b\"");
    }

    #[test]
    fn empty_argument_becomes_two_quotes() {
        assert_eq!(quote_str(""), "\"\"");
        assert_eq!(quote_command_line_arg(&[]), vec![b'"' as u16, b'"' as u16]);
    }

    #[test]
    fn unicode_arguments_are_preserved() {
        assert_eq!(quote_str("D:\\演示 文稿.pptx"), "\"D:\\演示 文稿.pptx\"");
    }

    #[test]
    fn non_ascii_units_are_not_special() {
        // 中文码元既不是反斜杠也不是引号，应原样保留
        let units: Vec<u16> = "文件".encode_utf16().collect();
        let mut expected = vec![b'"' as u16];
        expected.extend(units.iter().copied());
        expected.push(b'"' as u16);
        assert_eq!(quote_command_line_arg(&units), expected);
    }

    #[test]
    fn app_exists_rejects_empty_and_missing() {
        assert!(!app_exists(Path::new("")));
        assert!(!app_exists(Path::new("/definitely/not/here.exe")));
    }

    #[test]
    fn app_exists_rejects_directories_and_accepts_files() {
        let dir = crate::test_support::TempDir::new("launcher-exists");
        assert!(!app_exists(dir.path()));

        let file = dir.path().join("wpp.exe");
        std::fs::write(&file, b"stub").expect("write stub");
        assert!(app_exists(&file));
    }

    #[test]
    fn file_attributes_reports_directory_bit() {
        let dir = crate::test_support::TempDir::new("launcher-attrs");
        let attributes = file_attributes(dir.path()).expect("directory has attributes");
        assert_ne!(attributes & FILE_ATTRIBUTE_DIRECTORY, 0);
    }

    #[test]
    fn launch_app_reports_empty_paths() {
        // 非 Windows 下无论如何都返回 Err；这里只保证签名与错误通道可用。
        let result = launch_app(Path::new(""), Path::new(""));
        assert!(result.is_err());
    }
}
