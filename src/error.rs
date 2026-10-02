//! ============================================================================
//!  error.rs - 错误码与错误类型
//!  PPTX 智能打开器
//! ============================================================================
//!  对应原项目 `src/error_codes.h` 的 `enum class Err`，数值一一对应。
//!  注意：本程序在多数失败路径下会兜底启动，退出码主要用于日志与诊断。
//!
//!  命名说明：Rust 侧不沿用 `Err` 这个名字（会与 `Result::Err` 冲突），
//!  改名为 `ErrorCode`；数值与语义的对应关系见 `docs/behavior-mapping.md`。
//! ============================================================================

use std::fmt;
use std::io;

/// 程序退出码。与原项目 `pptx_selector::Err` 数值一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ErrorCode {
    /// 成功。
    Ok = 0,
    /// 命令行参数不足。
    InvalidArgs = 1,
    /// 目标 PPTX 文件不存在。
    FileNotFound = 2,
    /// 文件读取失败。
    ReadFailed = 3,
    /// ZIP 解析失败。
    ///
    /// 原项目定义了这个错误码，但 `WinMain` **从不返回它**：`detectPptxCreator`
    /// 内部把 ZIP 解析失败吞掉并返回 `Unknown`，最终走兜底路径退出 0 或 5。
    /// Rust 侧保留该变体仅为维持退出码表完整，同样不会返回。
    #[allow(dead_code)]
    InvalidArchive = 4,
    /// 启动外部程序失败。
    LaunchFailed = 5,
    /// 未找到任何可用启动器。
    ///
    /// 同 [`ErrorCode::InvalidArchive`]，原项目 `WinMain` 从不返回。
    #[allow(dead_code)]
    NoLauncher = 6,
}

impl ErrorCode {
    /// 转为 `std::process::exit` 需要的退出码。
    pub fn as_exit_code(self) -> i32 {
        self as i32
    }
}

/// PPTX 探测失败的原因。
///
/// 原项目 `detectPptxCreator` 把所有失败都折叠成 `AppCreator::Unknown`；
/// Rust 侧按说明书要求先用 `Result` 传播，再由上层映射回 `Unknown`
/// （因此对外可观察行为——日志与退出码——与原项目完全一致）。
#[derive(Debug)]
pub enum DetectError {
    /// ZIP 定位/中央目录解析失败（非 ZIP、截断、中央目录损坏等）。
    Zip(rawzip::Error),
    /// 中央目录中不存在 `docProps/app.xml`。
    EntryNotFound,
    /// 压缩方法不是 STORE 或 DEFLATE（对应 miniz 只支持这两种）。
    UnsupportedCompression,
    /// 读取 entry 原始字节失败。
    Read(io::Error),
    /// DEFLATE 解压失败。
    Inflate,
    /// 解出的 app.xml 超过大小上限（防解压炸弹）。
    TooLarge,
    /// 解压结果与中央目录记录的 CRC32 / 未压缩大小不符。
    VerificationFailed,
    /// XML 流式解析失败。
    Xml(quick_xml::Error),
}

impl fmt::Display for DetectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetectError::Zip(e) => write!(f, "ZIP 解析失败: {e}"),
            DetectError::EntryNotFound => write!(f, "缺少 docProps/app.xml"),
            DetectError::UnsupportedCompression => write!(f, "不支持的压缩方法"),
            DetectError::Read(e) => write!(f, "读取条目失败: {e}"),
            DetectError::Inflate => write!(f, "DEFLATE 解压失败"),
            DetectError::TooLarge => write!(f, "app.xml 超过大小上限"),
            DetectError::VerificationFailed => write!(f, "CRC32 或未压缩大小不匹配"),
            DetectError::Xml(e) => write!(f, "XML 解析失败: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_the_cpp_err_enum() {
        assert_eq!(ErrorCode::Ok.as_exit_code(), 0);
        assert_eq!(ErrorCode::InvalidArgs.as_exit_code(), 1);
        assert_eq!(ErrorCode::FileNotFound.as_exit_code(), 2);
        assert_eq!(ErrorCode::ReadFailed.as_exit_code(), 3);
        assert_eq!(ErrorCode::InvalidArchive.as_exit_code(), 4);
        assert_eq!(ErrorCode::LaunchFailed.as_exit_code(), 5);
        assert_eq!(ErrorCode::NoLauncher.as_exit_code(), 6);
    }
}
