//! ============================================================================
//!  logger.rs - 日志系统
//!  PPTX 智能打开器
//! ============================================================================
//!  轻量级文件日志，按行写入，自动补时间戳。
//!  对应原项目 `src/logger.h` / `src/logger.cpp`。
//!
//!  输出格式（逐字符对齐原项目 `writeUtf8`）：
//!      [YYYY-MM-DD HH:MM:SS] <正文>\r\n
//!  文件长度首次为 0（含新建）时先写入 UTF-8 BOM。每条日志独立 open/append/close。
//!  任何 I/O 失败都静默丢弃该行：禁止弹窗、禁止写控制台、禁止调用外部命令。
//!
//!  平台差异（仅实现细节，产出字节一致）：文件打开使用 `std::fs::OpenOptions`
//!  的 append+create 模式（Windows 上即 `FILE_APPEND_DATA` + `OPEN_ALWAYS`）；
//!  共享模式比原 C++ 多一个 `FILE_SHARE_DELETE`，不影响可观察行为。
//! ============================================================================

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// UTF-8 BOM。
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// 换行符（与原项目一致，固定 CRLF）。
const CRLF: &str = "\r\n";

/// 日志时间戳的组成部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimeParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

/// 按原项目 `_snprintf_s(prefix, 64, "[%04d-%02d-%02d %02d:%02d:%02d] ")` 格式化。
fn format_prefix(parts: &TimeParts) -> String {
    format!(
        "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] ",
        parts.year, parts.month, parts.day, parts.hour, parts.minute, parts.second
    )
}

/// Windows：使用 `GetLocalTime`，与原项目一致。
#[cfg(windows)]
fn now_parts() -> TimeParts {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    let mut st = SYSTEMTIME::default();
    // SAFETY: st 是一块合法的可写内存，GetLocalTime 会完整填充它。
    unsafe { GetLocalTime(&mut st) };
    TimeParts {
        year: i32::from(st.wYear),
        month: u32::from(st.wMonth),
        day: u32::from(st.wDay),
        hour: u32::from(st.wHour),
        minute: u32::from(st.wMinute),
        second: u32::from(st.wSecond),
    }
}

/// 非 Windows：使用 UTC（测试 shim）。
///
/// 本地时区渲染只能在 Windows 上验证，Linux 侧仅验证格式与字节。
#[cfg(not(windows))]
fn now_parts() -> TimeParts {
    use std::time::{SystemTime, UNIX_EPOCH};

    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    TimeParts {
        year: year as i32,
        month,
        day,
        hour: (rem / 3600) as u32,
        minute: ((rem % 3600) / 60) as u32,
        second: (rem % 60) as u32,
    }
}

/// 把"1970-01-01 起的天数"转换为公历年月日（Howard Hinnant 的 civil_from_days）。
#[cfg(not(windows))]
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// 轻量级文件日志器。禁用时所有写操作为空操作。
#[derive(Debug, Clone)]
pub struct Logger {
    enabled: bool,
    file: PathBuf,
}

impl Logger {
    /// 按配置初始化日志器。`enable_log == false` 时所有写操作静默。
    pub fn new(enable_log: bool, log_file: &Path) -> Self {
        Logger {
            enabled: enable_log,
            file: log_file.to_path_buf(),
        }
    }

    /// 写入一行日志，自动添加时间戳与换行。禁用或路径为空时为空操作。
    ///
    /// 对应原项目的 `write` / `writeW` / `writeUtf8` 三者：
    /// Rust 内部字符串本就是 UTF-8，无需再做宽字符转换，落盘字节完全一致。
    pub fn write(&self, message: &str) {
        if !self.enabled || self.file.as_os_str().is_empty() {
            return;
        }

        let line = format!("{}{}{}", format_prefix(&now_parts()), message, CRLF);

        let Ok(mut file) = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.file)
        else {
            return; // 打开失败：静默丢弃该行
        };

        // 原项目在写入前用 GetFileSizeEx 判断"文件长度为 0"，是则先写 BOM。
        let is_empty = file.metadata().is_ok_and(|m| m.len() == 0);
        if is_empty {
            let _ = file.write_all(BOM);
        }
        let _ = file.write_all(line.as_bytes());
    }

    /// 创建日志文件所在目录（若启用且目录不存在）。
    ///
    /// 目录部分的判定与原项目一致：取最后一个 `\` 或 `/` 之前的部分，
    /// 若不存在分隔符、或分隔符位于下标 0，则不建目录。
    #[cfg(windows)]
    pub fn init_dir(&self) {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows_sys::Win32::UI::Shell::SHCreateDirectoryExW;

        if !self.enabled || self.file.as_os_str().is_empty() {
            return;
        }
        let units: Vec<u16> = self.file.as_os_str().encode_wide().collect();
        let Some(index) = units
            .iter()
            .rposition(|&c| c == b'\\' as u16 || c == b'/' as u16)
        else {
            return;
        };
        if index == 0 {
            return;
        }
        let dir: Vec<u16> = OsString::from_wide(&units[..index])
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: dir 是 NUL 结尾的宽字符串；hwnd 与 SECURITY_ATTRIBUTES 传 NULL。
        // 返回值被忽略：目录已存在时 SHCreateDirectoryExW 返回 ERROR_ALREADY_EXISTS，
        // 这与原项目的行为一致。
        unsafe {
            SHCreateDirectoryExW(std::ptr::null_mut(), dir.as_ptr(), std::ptr::null());
        }
    }

    /// 非 Windows 下的等价实现（测试 shim）。
    #[cfg(not(windows))]
    pub fn init_dir(&self) {
        if !self.enabled || self.file.as_os_str().is_empty() {
            return;
        }
        let text = self.file.to_string_lossy();
        let Some(index) = text.rfind(['\\', '/']) else {
            return;
        };
        if index == 0 {
            return;
        }
        let _ = std::fs::create_dir_all(&text[..index]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;

    fn read(path: &Path) -> Vec<u8> {
        fs::read(path).expect("read log file")
    }

    #[test]
    fn format_prefix_is_zero_padded() {
        let parts = TimeParts {
            year: 2026,
            month: 10,
            day: 2,
            hour: 5,
            minute: 7,
            second: 9,
        };
        assert_eq!(format_prefix(&parts), "[2026-10-02 05:07:09] ");
    }

    #[test]
    fn disabled_logger_writes_nothing() {
        let dir = TempDir::new("log-disabled");
        let path = dir.path().join("nested").join("a.log");
        let logger = Logger::new(false, &path);
        logger.init_dir();
        logger.write("hello");
        assert!(!path.exists(), "禁用日志时不应创建文件或目录");
    }

    #[test]
    fn empty_path_is_a_no_op() {
        let logger = Logger::new(true, Path::new(""));
        logger.init_dir();
        logger.write("hello");
    }

    #[test]
    fn first_line_starts_with_bom_and_has_timestamp() {
        let dir = TempDir::new("log-bom");
        let path = dir.path().join("a.log");
        let logger = Logger::new(true, &path);
        logger.write("========== 程序启动 ==========");

        let bytes = read(&path);
        assert!(bytes.starts_with(BOM), "空文件首行前必须写 UTF-8 BOM");
        let text = String::from_utf8(bytes).expect("log is utf-8");
        assert!(
            text.contains("========== 程序启动 ==========\r\n"),
            "日志正文与 CRLF 必须逐字符一致: {text:?}"
        );

        let prefix = &text[3..25]; // BOM(3) + "[YYYY-MM-DD HH:MM:SS] " 共 22 字节
        let chars: Vec<char> = prefix.chars().collect();
        assert_eq!(chars.len(), 22);
        for (index, expected) in [
            (0, '['),
            (5, '-'),
            (8, '-'),
            (11, ' '),
            (14, ':'),
            (17, ':'),
            (20, ']'),
            (21, ' '),
        ] {
            assert_eq!(
                chars[index], expected,
                "前缀位置 {index} 不匹配: {prefix:?}"
            );
        }
        let expected_digits = [1, 2, 3, 4, 6, 7, 9, 10, 12, 13, 15, 16, 18, 19];
        for index in expected_digits {
            assert!(
                chars[index].is_ascii_digit(),
                "前缀位置 {index} 应为数字, 实际 {:?}: {prefix:?}",
                chars[index]
            );
        }
    }

    #[test]
    fn subsequent_writes_append_and_do_not_repeat_bom() {
        let dir = TempDir::new("log-append");
        let path = dir.path().join("a.log");
        let logger = Logger::new(true, &path);
        logger.write("第一行");
        logger.write("第二行");

        let bytes = read(&path);
        assert_eq!(&bytes[..3], BOM);
        let text = String::from_utf8(bytes[3..].to_vec()).expect("log is utf-8");
        assert_eq!(text.matches("第一行").count(), 1);
        assert_eq!(text.matches("第二行").count(), 1);
        // 两个 \r\n 结尾，且 BOM 只出现一次
        assert_eq!(text.matches("\r\n").count(), 2);
    }

    #[test]
    fn init_dir_creates_parent_chain() {
        let dir = TempDir::new("log-mkdir");
        let path = dir.path().join("a").join("b").join("c").join("x.log");
        let logger = Logger::new(true, &path);
        logger.init_dir();
        assert!(path.parent().expect("has parent").is_dir());
    }

    #[test]
    fn init_dir_skips_when_there_is_no_directory_part() {
        let logger = Logger::new(true, Path::new("plain.log"));
        logger.init_dir(); // 不应 panic，也不应创建任何东西
    }

    #[test]
    fn unicode_message_is_written_verbatim() {
        let dir = TempDir::new("log-unicode");
        let path = dir.path().join("a.log");
        let logger = Logger::new(true, &path);
        logger.write("目标文件: D:\\演示 文稿\\测试 文件.pptx");
        let text = String::from_utf8(read(&path)).expect("log is utf-8");
        assert!(text.contains("D:\\演示 文稿\\测试 文件.pptx"));
    }

    #[cfg(not(windows))]
    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_089), (2025, 1, 1));
    }
}
