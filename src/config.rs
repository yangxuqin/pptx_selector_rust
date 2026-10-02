//! ============================================================================
//!  config.rs - 配置管理
//!  PPTX 智能打开器
//! ============================================================================
//!  从 exe 同目录下的 config.ini 读取路径与选项，缺失项保留默认值。
//!  对应原项目 `src/config.h` / `src/config.cpp`。
//!
//!  平台策略：
//!  - Windows：调用真实 Win32 profile API（`GetPrivateProfileStringW` /
//!    `GetPrivateProfileIntW`），行为与原项目逐行一致。
//!  - 非 Windows：使用下方的 INI shim，**仅用于 Linux 原生单元测试与构建**，
//!    不参与 Windows 行为对齐。两者语义已按原项目注释与 Win32 文档对齐。
//!
//! ============================================================================

use std::path::{Path, PathBuf};

/// 程序运行配置。字段与内置默认值与原项目 `pptx_selector::Config` 一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// PowerPoint 可执行文件路径。
    pub path_powerpoint: PathBuf,
    /// WPS 演示可执行文件路径。
    pub path_wps: PathBuf,
    /// 日志文件路径。
    pub log_file: PathBuf,
    /// 是否启用日志。
    pub enable_log: bool,
    /// 兜底程序：0 = WPS，1 = PowerPoint。INI 中的取值原样保存。
    pub fallback: i32,
}

impl Default for Config {
    /// 与历史行为保持一致，便于无 config.ini 时直接运行。
    fn default() -> Self {
        Config {
            path_powerpoint: PathBuf::from(
                r"C:\Program Files\Microsoft Office\root\Office16\POWERPNT.EXE",
            ),
            path_wps: PathBuf::from(
                r"C:\Program Files\Kingsoft\WPS Office\12.1.0.23542\office6\wpp.exe",
            ),
            log_file: PathBuf::from(r"D:\logs\pptx_selector.log"),
            enable_log: true,
            fallback: 0,
        }
    }
}

/// 从 INI 文件加载配置，覆盖现有字段。缺失项保留传入 `cfg` 中的值。
///
/// 与原项目一致：任何读取失败都**不报错**，只是保留默认值。
pub fn load_config(cfg: &mut Config, ini_path: &Path) {
    let powerpoint = imp::read_string(
        ini_path,
        "Paths",
        "PowerPoint",
        &path_to_default(&cfg.path_powerpoint),
    );
    let wps = imp::read_string(ini_path, "Paths", "WPS", &path_to_default(&cfg.path_wps));
    let log_file = imp::read_string(
        ini_path,
        "Paths",
        "LogFile",
        &path_to_default(&cfg.log_file),
    );

    cfg.path_powerpoint = PathBuf::from(powerpoint);
    cfg.path_wps = PathBuf::from(wps);
    cfg.log_file = PathBuf::from(log_file);
    cfg.enable_log = imp::read_int(ini_path, "Options", "EnableLog", 1) != 0;
    cfg.fallback = imp::read_int(ini_path, "Options", "Fallback", 0);
}

/// 把当前字段值转成传给 profile API 的默认值字符串。
///
/// 原项目用 `GetPrivateProfileStringW` 的第三个参数传"当前字段值"，
/// 从而实现"INI 缺失该键则保留默认"。
fn path_to_default(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// 返回 exe 所在目录（不带末尾反斜杠）。取不到时返回 `"."`。
#[cfg(windows)]
pub fn get_exe_dir() -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;

    let mut buffer = vec![0u16; 32768];
    // SAFETY: buffer 是我们持有的一块可写内存，长度为 buffer.len()，调用不会越界。
    let length = unsafe {
        GetModuleFileNameW(
            std::ptr::null_mut(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    };
    if length == 0 || length as usize >= buffer.len() {
        return PathBuf::from(".");
    }

    let units = &buffer[..length as usize];
    let split = units
        .iter()
        .rposition(|&c| c == b'\\' as u16 || c == b'/' as u16);
    match split {
        None => PathBuf::from("."),
        Some(index) => PathBuf::from(OsString::from_wide(&units[..index])),
    }
}

/// 非 Windows 下的等价实现（测试 shim）。
#[cfg(not(windows))]
pub fn get_exe_dir() -> PathBuf {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return PathBuf::from("."),
    };
    let text = exe.to_string_lossy().into_owned();
    match text.rfind(['\\', '/']) {
        None => PathBuf::from("."),
        Some(index) => PathBuf::from(&text[..index]),
    }
}

// ============================================================================
//  Windows：真实 Win32 profile API
// ============================================================================

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;
    use windows_sys::Win32::System::WindowsProgramming::{
        GetPrivateProfileIntW, GetPrivateProfileStringW,
    };

    /// 与 C++ 一致：固定 32768 个 wchar 的缓冲区。
    const BUFFER_LEN: usize = 32768;

    fn to_wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn read_string(ini_path: &Path, section: &str, key: &str, default: &str) -> String {
        let section_w = to_wide(section);
        let key_w = to_wide(key);
        let default_w = to_wide(default);
        let ini_w: Vec<u16> = ini_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut buffer = vec![0u16; BUFFER_LEN];
        // SAFETY: 所有指针都指向 NUL 结尾的宽字符串或长度为 BUFFER_LEN 的可写缓冲区；
        // 返回值 length 是写入 buffer 的字符数（不含 NUL），必然 <= BUFFER_LEN。
        let length = unsafe {
            GetPrivateProfileStringW(
                section_w.as_ptr(),
                key_w.as_ptr(),
                default_w.as_ptr(),
                buffer.as_mut_ptr(),
                BUFFER_LEN as u32,
                ini_w.as_ptr(),
            )
        };
        let length = (length as usize).min(buffer.len());
        OsString::from_wide(&buffer[..length])
            .to_string_lossy()
            .into_owned()
    }

    pub fn read_int(ini_path: &Path, section: &str, key: &str, default: i32) -> i32 {
        let section_w = to_wide(section);
        let key_w = to_wide(key);
        let ini_w: Vec<u16> = ini_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: 指针均指向 NUL 结尾的宽字符串。
        unsafe {
            GetPrivateProfileIntW(section_w.as_ptr(), key_w.as_ptr(), default, ini_w.as_ptr())
        }
    }
}

// ============================================================================
//  非 Windows：仅用于测试与构建的 INI shim
// ============================================================================

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    /// INI 中的 `[section]` / `key=value` 语义按 Win32 profile API 复刻：
    /// 节名与键名 ASCII 大小写不敏感，值两侧空白裁剪，`;` / `#` 起始为注释，
    /// 成对包裹的双引号被剥离，键缺失时返回默认值。
    ///
    /// 已知近似：Win32 API 对"键存在但取值非数字"返回 0，本 shim 同样返回 0；
    /// 重复键取**第一次**出现（Win32 未定义，测试 fixture 不构造重复键）。
    pub fn read_string(ini_path: &Path, section: &str, key: &str, default: &str) -> String {
        lookup(ini_path, section, key).unwrap_or_else(|| default.to_string())
    }

    pub fn read_int(ini_path: &Path, section: &str, key: &str, default: i32) -> i32 {
        match lookup(ini_path, section, key) {
            Some(value) => parse_leading_int(&value),
            None => default,
        }
    }

    fn lookup(ini_path: &Path, section: &str, key: &str) -> Option<String> {
        let text = std::fs::read_to_string(ini_path).ok()?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let want_section = section.to_ascii_lowercase();
        let want_key = key.to_ascii_lowercase();
        let mut current_section = String::new();

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                if let Some(name) = rest.split(']').next() {
                    current_section = name.trim().to_ascii_lowercase();
                }
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            if current_section != want_section {
                continue;
            }
            if name.trim().to_ascii_lowercase() != want_key {
                continue;
            }
            return Some(unquote(value.trim()));
        }
        None
    }

    fn unquote(value: &str) -> String {
        match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            Some(inner) => inner.to_string(),
            None => value.to_string(),
        }
    }

    /// 解析开头连续的数字（可带正负号），无数字时返回 0。
    fn parse_leading_int(value: &str) -> i32 {
        let text = value.trim();
        let mut chars = text.chars().peekable();
        let mut negative = false;
        match chars.peek() {
            Some('-') => {
                negative = true;
                chars.next();
            }
            Some('+') => {
                chars.next();
            }
            _ => {}
        }

        let mut digits = 0usize;
        let mut acc: i64 = 0;
        for c in chars {
            let Some(digit) = c.to_digit(10) else { break };
            digits += 1;
            acc = (acc * 10 + i64::from(digit)).min(i64::from(i32::MAX));
        }
        if digits == 0 {
            return 0;
        }
        if negative {
            -acc as i32
        } else {
            acc as i32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;

    fn write_ini(dir: &TempDir, body: &str) -> PathBuf {
        let path = dir.path().join("config.ini");
        fs::write(&path, body).expect("write ini");
        path
    }

    #[test]
    fn defaults_match_the_original_config_constructor() {
        let cfg = Config::default();
        assert_eq!(
            cfg.path_powerpoint.to_string_lossy(),
            r"C:\Program Files\Microsoft Office\root\Office16\POWERPNT.EXE"
        );
        assert_eq!(
            cfg.path_wps.to_string_lossy(),
            r"C:\Program Files\Kingsoft\WPS Office\12.1.0.23542\office6\wpp.exe"
        );
        assert_eq!(cfg.log_file.to_string_lossy(), r"D:\logs\pptx_selector.log");
        assert!(cfg.enable_log);
        assert_eq!(cfg.fallback, 0);
    }

    #[test]
    fn missing_ini_keeps_defaults() {
        let dir = TempDir::new("cfg-missing");
        let missing = dir.path().join("does-not-exist.ini");
        let mut cfg = Config::default();
        load_config(&mut cfg, &missing);
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn ini_overrides_all_keys() {
        let dir = TempDir::new("cfg-override");
        let ini = write_ini(
            &dir,
            "; 注释\n[Paths]\nPowerPoint=C:\\ppt\\POWERPNT.EXE\nWPS=C:\\wps\\wpp.exe\nLogFile=C:\\logs\\a.log\n\n[Options]\nEnableLog=0\nFallback=1\n",
        );
        let mut cfg = Config::default();
        load_config(&mut cfg, &ini);
        assert_eq!(
            cfg.path_powerpoint.to_string_lossy(),
            r"C:\ppt\POWERPNT.EXE"
        );
        assert_eq!(cfg.path_wps.to_string_lossy(), r"C:\wps\wpp.exe");
        assert_eq!(cfg.log_file.to_string_lossy(), r"C:\logs\a.log");
        assert!(!cfg.enable_log);
        assert_eq!(cfg.fallback, 1);
    }

    #[test]
    fn partial_ini_keeps_remaining_defaults() {
        let dir = TempDir::new("cfg-partial");
        let ini = write_ini(&dir, "[Paths]\nWPS=D:\\wpp.exe\n");
        let mut cfg = Config::default();
        load_config(&mut cfg, &ini);
        assert_eq!(cfg.path_wps.to_string_lossy(), r"D:\wpp.exe");
        assert_eq!(cfg.path_powerpoint, Config::default().path_powerpoint);
        assert!(cfg.enable_log);
        assert_eq!(cfg.fallback, 0);
    }

    #[test]
    fn section_and_key_names_are_case_insensitive() {
        let dir = TempDir::new("cfg-case");
        let ini = write_ini(
            &dir,
            "[paths]\npowerpoint=E:\\a.exe\n[OPTIONS]\nENABLELOG=0\n",
        );
        let mut cfg = Config::default();
        load_config(&mut cfg, &ini);
        assert_eq!(cfg.path_powerpoint.to_string_lossy(), "E:\\a.exe");
        assert!(!cfg.enable_log);
    }

    #[test]
    fn malformed_integer_values_fall_back_to_zero() {
        let dir = TempDir::new("cfg-badint");
        let ini = write_ini(&dir, "[Options]\nEnableLog=abc\nFallback=not-a-number\n");
        let mut cfg = Config::default();
        load_config(&mut cfg, &ini);
        // EnableLog 原始默认值是 1，但键存在且非数字 -> 0。
        assert!(!cfg.enable_log);
        assert_eq!(cfg.fallback, 0);
    }

    #[test]
    fn value_whitespace_and_quotes_are_trimmed() {
        let dir = TempDir::new("cfg-quotes");
        let ini = write_ini(&dir, "[Paths]\nLogFile=   \"C:\\my logs\\a.log\"   \n");
        let mut cfg = Config::default();
        load_config(&mut cfg, &ini);
        assert_eq!(cfg.log_file.to_string_lossy(), r"C:\my logs\a.log");
    }

    #[test]
    fn unknown_keys_and_sections_are_ignored() {
        let dir = TempDir::new("cfg-unknown");
        let ini = write_ini(&dir, "[Nope]\nPowerPoint=X\n[Paths]\nOther=Y\n");
        let mut cfg = Config::default();
        load_config(&mut cfg, &ini);
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn unicode_values_survive() {
        let dir = TempDir::new("cfg-unicode");
        let ini = write_ini(&dir, "[Paths]\nLogFile=D:\\日志\\演示 文稿.log\n");
        let mut cfg = Config::default();
        load_config(&mut cfg, &ini);
        assert_eq!(cfg.log_file.to_string_lossy(), "D:\\日志\\演示 文稿.log");
    }

    #[test]
    fn get_exe_dir_returns_existing_directory() {
        let dir = get_exe_dir();
        assert!(dir.is_dir(), "get_exe_dir() = {dir:?} 应当是存在的目录");
    }
}
